//! Art and gameplay geometry specification for procedural grid walls.
//!
//! # Architecture & Coordinate System
//!
//! Procedural levels are built on a square tile grid aligned 1:1 with
//! [`super::nav::NavGrid`]:
//!
//! - **Cell size**: [`CELL_SIZE`] = **16 × 16 world pixels** (1 world pixel = 1
//!   half-block pixel in [`PixelBuffer`]). Cell `(cx, cy)` covers world pixels
//!   `[cx * 16, cx * 16 + 16) × [cy * 16, cy * 16 + 16)`.
//! - **3/4 perspective & height**: Consistent with [`super::art_props`] and
//!   [`super::art_rat`], light comes from the **top-left**. Every solid wall
//!   cell whose southern neighbour is open (`!mask.has(NeighborMask::S)`) shows:
//!   - a **10 px lit top face** (`ly in 0..10`) with a bright top/left bevel and
//!     a 1 px cornice catchlight along `ly == 9`,
//!   - a **6 px vertical south front face** (`ly in 10..16`, [`FRONT_FACE_H`])
//!     carrying the theme's masonry/cabinet/bulkhead detail and a 1 px dark base
//!     rim, and
//!   - a soft **floor contact shadow** cast [`SHADOW_SOUTH`] = 4 px south and
//!     [`SHADOW_EAST`] = 3 px east onto open floor tiles.
//!
//!   When a wall cell *does* have a wall to its south (`mask.has(NeighborMask::S)`),
//!   its full 16 px height is continuous top face joining smoothly into the cell
//!   below it.
//! - **Collision agreement**: [`wall_colliders`] returns the exact solid
//!   world-pixel [`Rectf`]s for any [`WallCellView`] with zero heap allocation.
//!   Every solid pixel blocks movement and projectiles; floor contact shadows
//!   and overhead lintels never block movement.
//!
//! # Render Layers (Call Order)
//!
//! Call the three pure draw functions in these layers inside `World::draw`:
//!
//! 1. **Ground layer** — [`draw_wall_ground`]: call right after
//!    [`super::art_floor::draw_floor`] and floor stains, **before** traps,
//!    pickups, enemies, player, and wall bodies. Draws contact shadows,
//!    doorway/one-way floor thresholds, directional chevrons, and rat-hole floor
//!    recess shadows.
//! 2. **Wall body layer** — [`draw_wall_cell`]: call after ground decals and
//!    shadows, **before** entities (or Y-sorted with props/entities). Draws wall
//!    caps, south front faces, jambs, rat-hole piers, and one-way gate leaves.
//! 3. **Overhang layer** (optional but recommended) — [`draw_wall_overhang`]:
//!    call **after** entities (player, enemies, bullets) and before particles /
//!    damage numbers. Draws the upper lintel beams of horizontal [`WallKind::RatHole`],
//!    [`WallKind::Doorway`], and north/south [`WallKind::OneWayDoor`] cells so
//!    the rat visibly ducks *under* arches and wall openings.
//!
//! # Generator Neighbor-Mask Contract
//!
//! To ensure thick room walls, thin maze partitions, doorways, and rat-holes
//! join cleanly without width-mismatch artefacts, build [`NeighborMask`] per
//! cell as follows:
//!
//! - [`WallKind::Solid`] and [`WallKind::RatHole`]: set a neighbor bit iff that
//!   neighbor is `Solid` or `RatHole` (do *not* connect to `Maze`, `Doorway`, or
//!   `OneWayDoor`, so room walls keep crisp edges where thin partitions or
//!   doorways meet them).
//! - [`WallKind::Maze`]: set cardinal bits iff the neighbor is `Maze`, `Solid`,
//!   or `RatHole` (thin partitions butt cleanly into structural room walls).
//! - [`WallKind::Doorway`] and [`WallKind::OneWayDoor`]: set cardinal bits for
//!   adjacent walls (`Solid`, `Maze`, `RatHole`) so jambs are drawn only on
//!   walled sides. Leave adjacent `Doorway` bits **unset** so two side-by-side
//!   doorways merge into a seamless 30 px double-wide doorway.

#![allow(dead_code)]

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

use super::level::{Rectf, Theme};

// ---- Public Geometry & Collision Constants ---------------------------------

/// Width and height of one grid cell in world pixels (matches `NavGrid`'s 16 px cell).
pub const CELL_SIZE: i32 = 16;
/// Floating-point alias of [`CELL_SIZE`].
pub const CELL_SIZE_F: f32 = 16.0;

/// Visible vertical south front-face height on south-exposed [`WallKind::Solid`]
/// and [`WallKind::RatHole`] cells (local rows `10..16`).
pub const FRONT_FACE_H: i32 = 6;
/// Lit top-face height on south-exposed solid cells (local rows `0..10`).
pub const TOP_FACE_H: i32 = CELL_SIZE - FRONT_FACE_H;

/// Width of a vertical [`WallKind::Maze`] partition in world pixels (`lx in 4..12`).
/// Leaves 4 px open margin on each side, making a 1-cell maze aisle 24 px wide
/// for comfortable bullet-hell dodging.
pub const MAZE_WALL_W: i32 = 8;
/// Horizontal inset from the cell edge to a [`WallKind::Maze`] vertical spine.
pub const MAZE_INSET_X: i32 = (CELL_SIZE - MAZE_WALL_W) / 2;
/// Top inset (`ly = 3`) and bottom inset (`ly = 13`) of a horizontal [`WallKind::Maze`]
/// partition (`5 px` lit top rail + `5 px` south front face = `10 px` footprint).
pub const MAZE_INSET_Y: i32 = 3;
/// South front-face height on a south-exposed [`WallKind::Maze`] partition (`ly in 8..13`).
pub const MAZE_FRONT_H: i32 = 5;

/// Clear opening width (in world pixels) of a [`WallKind::RatHole`] gap.
/// Centered inside the 16 px cell (`4..12`), leaving two 4 px solid wall piers.
/// The hole art itself is drawn by `art_traps.rs` inside [`rat_hole_gap_rect`].
pub const RAT_HOLE_GAP_W: i32 = 8;
/// Thickness of each flanking solid wall pier in a [`WallKind::RatHole`] cell.
pub const RAT_HOLE_PIER: i32 = (CELL_SIZE - RAT_HOLE_GAP_W) / 2;
/// Maximum entity collision radius permitted through a [`WallKind::RatHole`].
/// Player rat (`r = 3.0`) fits with 1 px margin on each side; Cats (`r = 4.0`),
/// Brutes (`r = 6.5`), and Bosses (`r = 14.0`) are blocked.
pub const RAT_HOLE_MAX_RADIUS: f32 = 3.5;

/// Collision thickness (in world pixels) of each walled [`WallKind::Doorway`] or
/// [`WallKind::OneWayDoor`] jamb. Visual trim extends 2 px inward (inner 1 px is
/// chamfered visual trim). Leaves **14 px** clear in a 1-cell doorway (fits
/// Brutes, diameter 13 px) and **30 px** clear in a 2-cell doorway (fits Bosses,
/// diameter 28 px).
pub const DOORWAY_JAMB_COLLISION: i32 = 1;
/// Visual width (in pixels) of a doorway jamb pillar on a walled edge.
pub const DOORWAY_JAMB_VISUAL: i32 = 2;

/// Floor contact shadow depth cast south of an exposed south wall face.
pub const SHADOW_SOUTH: i32 = 4;
/// Floor contact shadow width cast east of an exposed east wall edge.
pub const SHADOW_EAST: i32 = 3;

// ---- Plain Copy View Types -------------------------------------------------

/// 8-bit neighbor presence mask for a grid wall cell.
///
/// Cardinal bits control continuous runs, ends, corners, and south front faces;
/// diagonal bits control crisp concave inner corners and L-corner ambient
/// occlusion shadows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NeighborMask(pub u8);

impl NeighborMask {
    pub const N: u8 = 1 << 0;
    pub const E: u8 = 1 << 1;
    pub const S: u8 = 1 << 2;
    pub const W: u8 = 1 << 3;
    pub const NE: u8 = 1 << 4;
    pub const SE: u8 = 1 << 5;
    pub const SW: u8 = 1 << 6;
    pub const NW: u8 = 1 << 7;

    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self(0xFF);

    #[inline]
    pub const fn from_cardinals(n: bool, e: bool, s: bool, w: bool) -> Self {
        let mut bits = 0u8;
        if n {
            bits |= Self::N;
        }
        if e {
            bits |= Self::E;
        }
        if s {
            bits |= Self::S;
        }
        if w {
            bits |= Self::W;
        }
        Self(bits)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub const fn from_all(
        n: bool,
        e: bool,
        s: bool,
        w: bool,
        ne: bool,
        se: bool,
        sw: bool,
        nw: bool,
    ) -> Self {
        let mut bits = Self::from_cardinals(n, e, s, w).0;
        if ne {
            bits |= Self::NE;
        }
        if se {
            bits |= Self::SE;
        }
        if sw {
            bits |= Self::SW;
        }
        if nw {
            bits |= Self::NW;
        }
        Self(bits)
    }

    #[inline]
    pub const fn has(self, bit: u8) -> bool {
        (self.0 & bit) != 0
    }

    #[inline]
    pub const fn with(self, bit: u8) -> Self {
        Self(self.0 | bit)
    }
}

/// Orientation of a wall segment containing an opening ([`WallKind::RatHole`] or
/// [`WallKind::Doorway`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallAxis {
    /// Wall runs West–East; passage traverses North ↔ South.
    Horizontal,
    /// Wall runs North–South; passage traverses West ↔ East.
    Vertical,
}

/// Cardinal pass-through direction for [`WallKind::OneWayDoor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinal {
    North,
    East,
    South,
    West,
}

impl Cardinal {
    #[inline]
    pub const fn wall_axis(self) -> WallAxis {
        match self {
            Cardinal::North | Cardinal::South => WallAxis::Horizontal,
            Cardinal::East | Cardinal::West => WallAxis::Vertical,
        }
    }
}

/// Architectural tile type / gameplay specification for one 16 × 16 wall cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallKind {
    /// Full-thickness structural room/corridor wall.
    ///
    /// - **Collision**: Solid `16 × 16` px rect (`[wx, wx+16) × [wy, wy+16)`).
    ///   Blocks all entities, enemy pathfinding (`NavGrid`), and projectiles.
    /// - **Height & art**: `10 px` lit top face + `6 px` south vertical front
    ///   face when `!S`, or full `16 px` continuous top face when `S`. Variant
    ///   details (continuous pipe runs, moss drips, kitchen cabinet bays vs
    ///   tiled wainscoting + skirting boards, steel bulkheads + louvers +
    ///   warning stripes) are selected deterministically from
    ///   `(cell_x, cell_y, seed)`.
    Solid,

    /// Thin partition wall for procedural maze sections.
    ///
    /// - **Collision**: Exact match to drawn bars via [`wall_colliders`]:
    ///   vertical spine `[wx+4, wx+12)` (8 px thick) and horizontal bar
    ///   `[wy+3, wy+13)` (10 px tall = 5 px top rail + 5 px south front face),
    ///   extending to cell borders on connected cardinal sides. Blocks all
    ///   entities and projectiles (except lobbed/overhead attacks if desired).
    /// - **Gameplay purpose**: Visually distinguishes maze wings from structural
    ///   rooms while widening 1-cell maze aisles from `16 px` to `22–24 px` so
    ///   the player can still weave through bullets and kite enemies.
    Maze,

    /// Full-thickness wall pierced by an **8 px wide rat-sized opening**.
    ///
    /// - **Collision**: Two solid `4 × 16` px (Horizontal) or `16 × 4` px
    ///   (Vertical) wall piers returned by [`wall_colliders`], leaving an exact
    ///   `8 px` wide central gap given by [`rat_hole_gap_rect`].
    /// - **Size-gated passability**: The implementer must treat
    ///   [`rat_hole_gap_rect`] as **passable** for the player rat (`r = 3.0`)
    ///   and tiny pests (`radius <= RAT_HOLE_MAX_RADIUS = 3.5`), and **solid**
    ///   (blocking both physics and `NavGrid` for large agents) for Cats
    ///   (`r = 4.0`), Brutes (`r = 6.5`), and all Bosses. Projectiles pass
    ///   through the 8 px gap.
    /// - **Art handoff**: This module draws the framing wall piers, recessed
    ///   floor shadow, and upper lintel overhang (`ly in 0..5` on Horizontal),
    ///   leaving the `8 px` opening clean for `art_traps.rs` to render the
    ///   mousehole arch / gnawed trim.
    RatHole { axis: WallAxis },

    /// Open architectural doorway connecting rooms, corridors, and maze entries.
    ///
    /// - **Collision**: Only the `1 px` solid jamb strips on edges touching an
    ///   adjacent wall (`mask` cardinal bits), returned by [`wall_colliders`].
    ///   - A **1-cell doorway** leaves **14 px** clear (fits player, Skitterers,
    ///     Cats, Spitters, and Brutes `dia = 13 px`).
    ///   - A **2-cell doorway** (two adjacent `Doorway` cells with the shared
    ///     edge bit unset in `mask`) leaves **30 px** clear (fits Bosses
    ///     `dia = 28 px`).
    /// - **Nav & bullets**: Passable for all entities that fit the clear width
    ///   and all projectiles.
    Doorway { axis: WallAxis },

    /// One-way exit door placed at the reward end of a maze section.
    ///
    /// - **Allowed direction**: `dir` is the sole travel direction that opens
    ///   the door (e.g., exiting a maze treasure alcove back into a main
    ///   corridor). Bold floor chevrons point along `dir`.
    /// - **State & timing** (driven via `WallCellView::open_progress` in `0..=1`):
    ///   - `0.0`: Closed / latched. Blocks movement from the `+dir` (reverse)
    ///     side via [`one_way_gate_rect`], and blocks enemy `NavGrid` routing
    ///     into the maze reward pocket.
    ///   - Player approaches from the `-dir` side moving toward `dir`: swings
    ///     open `0.0 -> 1.0` over **0.15 s** (`open_progress >= 0.5` clears the
    ///     gate collider).
    ///   - Once the player exits the cell on the `+dir` side: springs shut
    ///     `1.0 -> 0.0` over **0.35 s**. Enemies cannot trigger it from either
    ///     side (they may only slip through during the brief open window).
    /// - **Collision**: Jamb strips from [`wall_colliders`] are always solid;
    ///   [`one_way_gate_rect`] blocks reverse entry and closed enemies.
    OneWayDoor { dir: Cardinal },
}

/// Plain `Copy` view describing everything needed to draw one grid wall cell.
/// Free of simulation references; construct directly from the level grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallCellView {
    /// Grid column (`world_x = cell_x * CELL_SIZE`). Drives seamless hashing.
    pub cell_x: i32,
    /// Grid row (`world_y = cell_y * CELL_SIZE`). Drives seamless hashing.
    pub cell_y: i32,
    /// Level generation seed mixed into per-cell variant selection.
    pub seed: u32,
    /// Visual theme (`Sewer`, `Kitchen`, or `Lab`).
    pub theme: Theme,
    /// Architectural tile kind (solid, maze, rat-hole, doorway, one-way door).
    pub kind: WallKind,
    /// 8-bit neighbor connectivity mask (see module-level contract).
    pub mask: NeighborMask,
    /// Normalized state progress `0.0..=1.0`.
    /// Used by [`WallKind::OneWayDoor`] (`0.0` = closed, `1.0` = swung open);
    /// ignored by static wall kinds.
    pub open_progress: f32,
    /// Run clock in seconds for subtle ambient motion (drip glint, LED pulse,
    /// one-way chevron pulse). Pass `0.0` for static frames.
    pub anim_clock: f32,
}

impl WallCellView {
    /// Convenience constructor for static cells (`open_progress = 0.0`).
    #[inline]
    pub const fn new(
        cell_x: i32,
        cell_y: i32,
        seed: u32,
        theme: Theme,
        kind: WallKind,
        mask: NeighborMask,
    ) -> Self {
        Self {
            cell_x,
            cell_y,
            seed,
            theme,
            kind,
            mask,
            open_progress: 0.0,
            anim_clock: 0.0,
        }
    }

    /// World-pixel top-left origin `(wx, wy)` of this cell.
    #[inline]
    pub const fn world_origin(&self) -> (i32, i32) {
        (self.cell_x * CELL_SIZE, self.cell_y * CELL_SIZE)
    }
}

// ---- Exact Collision & Gap Geometry Helpers --------------------------------

/// Populate `out` with the exact solid collision rectangles (in world pixels)
/// for `view`, returning the count written (`0..=2`). Zero heap allocation.
///
/// For [`WallKind::RatHole`], this returns the two flanking wall piers; enforce
/// the size-gated passage separately using [`rat_hole_gap_rect`] and
/// [`RAT_HOLE_MAX_RADIUS`].
/// For [`WallKind::OneWayDoor`], this returns the permanent jamb strips; enforce
/// directional blocking with [`one_way_gate_rect`] when `open_progress < 0.5`.
pub fn wall_colliders(view: &WallCellView, out: &mut [Rectf; 2]) -> usize {
    let wx = (view.cell_x * CELL_SIZE) as f32;
    let wy = (view.cell_y * CELL_SIZE) as f32;
    let cs = CELL_SIZE_F;
    let m = view.mask;

    match view.kind {
        WallKind::Solid => {
            out[0] = Rectf::new(wx, wy, cs, cs);
            1
        }
        WallKind::Maze => {
            let has_n = m.has(NeighborMask::N);
            let has_e = m.has(NeighborMask::E);
            let has_s = m.has(NeighborMask::S);
            let has_w = m.has(NeighborMask::W);
            let mut n = 0;

            let has_h = has_e || has_w || (!has_n && !has_s);
            let has_v = has_n || has_s;

            if has_h {
                let x0 = if has_w { 0.0 } else { MAZE_INSET_X as f32 };
                let x1 = if has_e {
                    cs
                } else {
                    (CELL_SIZE - MAZE_INSET_X) as f32
                };
                let y0 = MAZE_INSET_Y as f32;
                let y1 = (CELL_SIZE - MAZE_INSET_Y) as f32;
                out[n] = Rectf::new(wx + x0, wy + y0, x1 - x0, y1 - y0);
                n += 1;
            }
            if has_v {
                let x0 = MAZE_INSET_X as f32;
                let x1 = (CELL_SIZE - MAZE_INSET_X) as f32;
                let y0 = if has_n { 0.0 } else { MAZE_INSET_Y as f32 };
                let y1 = if has_s {
                    cs
                } else {
                    (CELL_SIZE - MAZE_INSET_Y) as f32
                };
                out[n] = Rectf::new(wx + x0, wy + y0, x1 - x0, y1 - y0);
                n += 1;
            }
            n
        }
        WallKind::RatHole { axis } => {
            let p = RAT_HOLE_PIER as f32;
            match axis {
                WallAxis::Horizontal => {
                    out[0] = Rectf::new(wx, wy, p, cs);
                    out[1] = Rectf::new(wx + cs - p, wy, p, cs);
                }
                WallAxis::Vertical => {
                    out[0] = Rectf::new(wx, wy, cs, p);
                    out[1] = Rectf::new(wx, wy + cs - p, cs, p);
                }
            }
            2
        }
        WallKind::Doorway { axis } => jamb_colliders(wx, wy, axis, m, out),
        WallKind::OneWayDoor { dir } => jamb_colliders(wx, wy, dir.wall_axis(), m, out),
    }
}

fn jamb_colliders(wx: f32, wy: f32, axis: WallAxis, m: NeighborMask, out: &mut [Rectf; 2]) -> usize {
    let j = DOORWAY_JAMB_COLLISION as f32;
    let cs = CELL_SIZE_F;
    let mut n = 0;
    match axis {
        WallAxis::Horizontal => {
            if m.has(NeighborMask::W) {
                out[n] = Rectf::new(wx, wy, j, cs);
                n += 1;
            }
            if m.has(NeighborMask::E) {
                out[n] = Rectf::new(wx + cs - j, wy, j, cs);
                n += 1;
            }
        }
        WallAxis::Vertical => {
            if m.has(NeighborMask::N) {
                out[n] = Rectf::new(wx, wy, cs, j);
                n += 1;
            }
            if m.has(NeighborMask::S) {
                out[n] = Rectf::new(wx, wy + cs - j, cs, j);
                n += 1;
            }
        }
    }
    n
}

/// World-pixel rectangle of the `8 px` opening left clear by [`WallKind::RatHole`].
///
/// - `Horizontal`: `[wx + 4, wx + 12) × [wy, wy + 16)` (8 × 16 passage; front
///   face opening below the overhang lintel is `ly in 5..16`).
/// - `Vertical`: `[wx, wx + 16) × [wy + 4, wy + 12)` (16 × 8 passage).
///
/// `art_traps.rs` places mousehole arch/trim art inside this region, and the
/// simulation blocks entities whose `radius > RAT_HOLE_MAX_RADIUS` against it.
pub const fn rat_hole_gap_rect(cell_x: i32, cell_y: i32, axis: WallAxis) -> Rectf {
    let wx = (cell_x * CELL_SIZE) as f32;
    let wy = (cell_y * CELL_SIZE) as f32;
    let p = RAT_HOLE_PIER as f32;
    let g = RAT_HOLE_GAP_W as f32;
    match axis {
        WallAxis::Horizontal => Rectf {
            x: wx + p,
            y: wy,
            w: g,
            h: CELL_SIZE_F,
        },
        WallAxis::Vertical => Rectf {
            x: wx,
            y: wy + p,
            w: CELL_SIZE_F,
            h: g,
        },
    }
}

/// World-pixel barrier rectangle for a closed [`WallKind::OneWayDoor`] gate leaf.
/// Active for enemies (always) and for the player when approaching against `dir`.
pub const fn one_way_gate_rect(cell_x: i32, cell_y: i32, dir: Cardinal) -> Rectf {
    let wx = (cell_x * CELL_SIZE) as f32;
    let wy = (cell_y * CELL_SIZE) as f32;
    match dir {
        Cardinal::North | Cardinal::South => Rectf {
            x: wx + 1.0,
            y: wy + 6.0,
            w: CELL_SIZE_F - 2.0,
            h: 4.0,
        },
        Cardinal::East | Cardinal::West => Rectf {
            x: wx + 6.0,
            y: wy + 1.0,
            w: 4.0,
            h: CELL_SIZE_F - 2.0,
        },
    }
}

// ---- Theme Material Palettes -----------------------------------------------

#[derive(Clone, Copy)]
struct WallPalette {
    outline: Rgb,
    top_base: Rgb,
    top_alt: Rgb,
    top_hi: Rgb,
    top_lo: Rgb,
    cornice: Rgb,
    front_base: Rgb,
    front_alt: Rgb,
    front_dark: Rgb,
    mortar: Rgb,
    trim_hi: Rgb,
    trim_lo: Rgb,
    detail_a: Rgb, // moss / brass / hazard ochre
    detail_b: Rgb, // pipe iron / cream tile / conduit slate
    detail_c: Rgb, // rust / dark mahogany / bright steel
    maze_top: Rgb,
    maze_hi: Rgb,
    maze_front: Rgb,
    thresh_base: Rgb,
    thresh_hi: Rgb,
}

fn theme_palette(theme: Theme) -> WallPalette {
    match theme {
        Theme::Sewer => WallPalette {
            outline: (12, 20, 18),
            top_base: (50, 74, 69),
            top_alt: (43, 65, 60),
            top_hi: (94, 134, 124),
            top_lo: (30, 48, 45),
            cornice: (106, 146, 136),
            front_base: (38, 60, 55),
            front_alt: (48, 72, 65),
            front_dark: (20, 34, 31),
            mortar: (18, 30, 28),
            trim_hi: (128, 156, 148),
            trim_lo: (25, 38, 36),
            detail_a: (60, 102, 50),  // damp moss (richer green pop)
            detail_b: (92, 118, 114), // bolted sewer pipe iron
            detail_c: (116, 72, 46),  // weathered iron rust patina
            maze_top: (76, 88, 84),   // cast-iron culvert slat divider
            maze_hi: (132, 148, 140),
            maze_front: (46, 58, 56),
            thresh_base: (34, 50, 48),
            thresh_hi: (84, 112, 106),
        },
        Theme::Kitchen => WallPalette {
            outline: (26, 17, 11),
            top_base: (104, 78, 54),
            top_alt: (94, 70, 48),
            top_hi: (166, 132, 94),
            top_lo: (62, 44, 30),
            cornice: (178, 142, 100),
            front_base: (86, 60, 38),
            front_alt: (134, 116, 92), // crisp cream-ochre subway tile dado
            front_dark: (44, 28, 18),  // rich dark mahogany skirting board
            mortar: (46, 32, 22),
            trim_hi: (206, 172, 98),   // brushed brass hardware
            trim_lo: (54, 36, 22),
            detail_a: (212, 178, 96),  // brass cabinet knobs
            detail_b: (158, 142, 116), // glazed ceramic highlight
            detail_c: (68, 74, 84),    // wrought-iron pantry strap
            maze_top: (128, 92, 56),   // slatted oak pantry divider
            maze_hi: (186, 144, 94),
            maze_front: (78, 54, 32),
            thresh_base: (56, 40, 26),
            thresh_hi: (178, 146, 86),
        },
        Theme::Lab => WallPalette {
            outline: (12, 15, 24),
            top_base: (52, 62, 88),
            top_alt: (44, 54, 78),
            top_hi: (114, 134, 170),
            top_lo: (28, 34, 52),
            cornice: (128, 148, 186),
            front_base: (40, 48, 72),
            front_alt: (52, 62, 90),
            front_dark: (20, 24, 38),
            mortar: (19, 24, 38),
            trim_hi: (150, 168, 200),
            trim_lo: (24, 30, 44),
            detail_a: (190, 154, 64),  // industrial hazard ochre
            detail_b: (76, 92, 122),   // conduit steel
            detail_c: (182, 212, 232), // glass/steel glint
            maze_top: (68, 110, 136),  // frosted glass & steel test maze
            maze_hi: (154, 210, 232),
            maze_front: (40, 54, 76),
            thresh_base: (28, 36, 54),
            thresh_hi: (98, 118, 150),
        },
    }
}

// ---- Deterministic Hashing & Pixel Helpers ---------------------------------

#[inline]
fn hash3(a: i32, b: i32, c: u32) -> u32 {
    let mut h = (a as u32).wrapping_mul(0x9E37_79B1)
        ^ (b as u32).wrapping_mul(0x85EB_CA77)
        ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h
}

#[inline]
fn cell_screen_origin(view: &WallCellView, cam: Vec2) -> (i32, i32) {
    let (wx, wy) = view.world_origin();
    (
        (wx as f32 - cam.x).round() as i32,
        (wy as f32 - cam.y).round() as i32,
    )
}

#[inline]
fn culled(pb: &PixelBuffer, sx: i32, sy: i32) -> bool {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    sx >= bw + 4 || sy >= bh + 4 || sx + CELL_SIZE + SHADOW_EAST < 0 || sy + CELL_SIZE + SHADOW_SOUTH < 0
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, factor: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, palette::scale(cur, factor));
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, palette::mix(cur, c, alpha));
}

// ============================================================================
// LAYER 1: GROUND (SHADOWS, THRESHOLDS, CHEVRONS)
// ============================================================================

/// **Ground layer**: draw contact shadows, flush doorway/one-way threshold
/// plates, directional chevrons, and rat-hole floor recesses.
///
/// Call right after `art_floor::draw_floor` and floor stains, **before**
/// entities, traps, pickups, and [`draw_wall_cell`]. Zero heap allocation.
pub fn draw_wall_ground(pb: &mut PixelBuffer, view: &WallCellView, cam: Vec2, accent: Rgb) {
    let (sx, sy) = cell_screen_origin(view, cam);
    if culled(pb, sx, sy) {
        return;
    }
    let pal = theme_palette(view.theme);

    match view.kind {
        WallKind::Solid => draw_solid_shadow(pb, sx, sy, view.mask),
        WallKind::Maze => draw_maze_shadow(pb, sx, sy, view.mask),
        WallKind::RatHole { axis } => draw_rathole_ground(pb, sx, sy, view.mask, axis),
        WallKind::Doorway { axis } => {
            draw_doorway_ground(pb, sx, sy, view.mask, axis, view.theme, &pal, accent);
        }
        WallKind::OneWayDoor { dir } => {
            draw_doorway_ground(
                pb,
                sx,
                sy,
                view.mask,
                dir.wall_axis(),
                view.theme,
                &pal,
                accent,
            );
            draw_oneway_chevron(pb, sx, sy, dir, view.open_progress, view.anim_clock, accent);
        }
    }
}

fn draw_solid_shadow(pb: &mut PixelBuffer, sx: i32, sy: i32, m: NeighborMask) {
    let has_n = m.has(NeighborMask::N);
    let has_e = m.has(NeighborMask::E);
    let has_s = m.has(NeighborMask::S);
    let has_w = m.has(NeighborMask::W);
    let has_ne = m.has(NeighborMask::NE);

    // 1. South contact shadow (spills 4 px below exposed south face).
    if !has_s {
        let factors = [0.38, 0.56, 0.74, 0.88];
        for (dy, &f) in factors.iter().enumerate() {
            let dy = dy as i32;
            let py = sy + CELL_SIZE + dy;
            let x0 = if has_w { sx } else { sx + 1 + (dy / 2) };
            let x1 = if has_e {
                sx + CELL_SIZE
            } else {
                sx + CELL_SIZE + (SHADOW_EAST - dy).max(0)
            };
            for px in x0..x1 {
                darken_px(pb, px, py, f);
            }
        }
    }

    // 2. East contact shadow (spills 3 px right of exposed east edge).
    if !has_e {
        let factors = [0.42, 0.64, 0.82];
        for (dx, &f) in factors.iter().enumerate() {
            let dx = dx as i32;
            let px = sx + CELL_SIZE + dx;
            let y0 = if has_n && !has_ne { sy } else { sy + 2 + dx };
            let y1 = sy + CELL_SIZE;
            for py in y0..y1 {
                darken_px(pb, px, py, f);
            }
        }
    }
}

fn draw_maze_shadow(pb: &mut PixelBuffer, sx: i32, sy: i32, m: NeighborMask) {
    let has_n = m.has(NeighborMask::N);
    let has_e = m.has(NeighborMask::E);
    let has_s = m.has(NeighborMask::S);
    let has_w = m.has(NeighborMask::W);

    let s_factors = [0.44, 0.66, 0.84];
    let x_start = if has_w { 0 } else { MAZE_INSET_X };
    let x_end = if has_e {
        CELL_SIZE
    } else {
        CELL_SIZE - MAZE_INSET_X
    };

    for (dy, &f) in s_factors.iter().enumerate() {
        let dy = dy as i32;
        let py = sy + (CELL_SIZE - MAZE_INSET_Y) + dy;
        for lx in x_start..x_end {
            if has_s && (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx) {
                continue;
            }
            if !has_w && lx == MAZE_INSET_X && dy >= 1 {
                continue;
            }
            darken_px(pb, sx + lx, py, f);
        }
        if !has_e && dy < 2 {
            darken_px(pb, sx + x_end + dy, py, f);
            if dy == 0 {
                darken_px(pb, sx + x_end + 1, py, 0.66);
            }
        }
    }

    if !has_e {
        let e_factors = [0.48, 0.72];
        let y_top = if has_n { 0 } else { MAZE_INSET_Y + 2 };
        let y_bot = if has_s {
            CELL_SIZE
        } else {
            CELL_SIZE - MAZE_INSET_Y
        };
        for (dx, &f) in e_factors.iter().enumerate() {
            let dx = dx as i32;
            let px = sx + (CELL_SIZE - MAZE_INSET_X) + dx;
            for ly in (y_top + dx).min(y_bot)..y_bot {
                darken_px(pb, px, sy + ly, f);
            }
        }
    } else if has_s {
        for (dx, &f) in [0.48, 0.72].iter().enumerate() {
            let px = sx + (CELL_SIZE - MAZE_INSET_X) + dx as i32;
            for ly in (CELL_SIZE - MAZE_INSET_Y)..CELL_SIZE {
                darken_px(pb, px, sy + ly, f);
            }
        }
    }
}

fn draw_rathole_ground(pb: &mut PixelBuffer, sx: i32, sy: i32, m: NeighborMask, axis: WallAxis) {
    draw_solid_shadow(pb, sx, sy, m);

    match axis {
        WallAxis::Horizontal => {
            for ly in 3..CELL_SIZE {
                for lx in RAT_HOLE_PIER..(CELL_SIZE - RAT_HOLE_PIER) {
                    let edge = lx == RAT_HOLE_PIER || lx == CELL_SIZE - RAT_HOLE_PIER - 1;
                    let f = if ly < 7 {
                        0.36
                    } else if edge {
                        0.54
                    } else {
                        0.68
                    };
                    darken_px(pb, sx + lx, sy + ly, f);
                }
            }
        }
        WallAxis::Vertical => {
            for ly in RAT_HOLE_PIER..(CELL_SIZE - RAT_HOLE_PIER) {
                for lx in 0..CELL_SIZE {
                    let edge = ly == RAT_HOLE_PIER || ly == CELL_SIZE - RAT_HOLE_PIER - 1;
                    let mid = (4..12).contains(&lx);
                    let f = if ly == RAT_HOLE_PIER {
                        0.40 // deeper shadow directly beneath the north pier cheek
                    } else if mid && edge {
                        0.46
                    } else if mid {
                        0.58
                    } else {
                        0.76
                    };
                    darken_px(pb, sx + lx, sy + ly, f);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_doorway_ground(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    m: NeighborMask,
    axis: WallAxis,
    theme: Theme,
    pal: &WallPalette,
    accent: Rgb,
) {
    match axis {
        WallAxis::Horizontal => {
            let x0 = if m.has(NeighborMask::W) { 2 } else { 0 };
            let x1 = if m.has(NeighborMask::E) {
                CELL_SIZE - 2
            } else {
                CELL_SIZE
            };
            // Subtle floor contact shadow south of the threshold sill
            for lx in x0..x1 {
                darken_px(pb, sx + lx, sy + 14, 0.66);
                darken_px(pb, sx + lx, sy + 15, 0.84);
            }
            // Flush themed threshold sill across `ly in 6..14`
            for ly in 6..14 {
                for lx in x0..x1 {
                    let px = sx + lx;
                    let py = sy + ly;
                    let c = if ly == 6 || ly == 13 {
                        pal.outline
                    } else if ly == 7 {
                        pal.thresh_hi
                    } else if ly == 12 {
                        palette::scale(pal.thresh_base, 0.72)
                    } else {
                        match theme {
                            Theme::Sewer => {
                                // Iron drainage grate bars across threshold
                                if (lx % 3) == 0 {
                                    pal.outline
                                } else if (lx % 3) == 1 {
                                    palette::mix(pal.thresh_base, pal.thresh_hi, 0.28)
                                } else {
                                    pal.thresh_base
                                }
                            }
                            Theme::Kitchen => {
                                // Brushed brass kickplate strip centered inside dark oak sill
                                if ly == 9 || ly == 10 {
                                    palette::scale(pal.thresh_hi, if ly == 9 { 0.92 } else { 0.72 })
                                } else {
                                    pal.thresh_base
                                }
                            }
                            Theme::Lab => {
                                // Recessed airlock deck plate with fine steel ribs & edge accent ticks
                                if ly == 9 || ly == 10 {
                                    if (lx & 1) == 0 {
                                        pal.thresh_base
                                    } else {
                                        palette::scale(pal.thresh_base, 0.72)
                                    }
                                } else if (lx == x0 + 1 || lx == x1 - 2) && ly == 8 {
                                    palette::mix(pal.thresh_hi, accent, 0.45)
                                } else {
                                    pal.thresh_base
                                }
                            }
                        }
                    };
                    blend_px(pb, px, py, c, 0.90);
                }
            }
            // Ground shadows cast by flanking doorway pilasters
            if m.has(NeighborMask::W) {
                for dy in 0..3 {
                    for lx in 0..3 {
                        darken_px(pb, sx + lx, sy + CELL_SIZE + dy, 0.44 + dy as f32 * 0.16);
                    }
                }
                for ly in 3..CELL_SIZE {
                    darken_px(pb, sx + 2, sy + ly, 0.54);
                    darken_px(pb, sx + 3, sy + ly, 0.76);
                }
            }
            if m.has(NeighborMask::E) {
                for dy in 0..3 {
                    for lx in (CELL_SIZE - 2)..(CELL_SIZE + 2) {
                        darken_px(pb, sx + lx, sy + CELL_SIZE + dy, 0.44 + dy as f32 * 0.16);
                    }
                }
            }
        }
        WallAxis::Vertical => {
            let y0 = if m.has(NeighborMask::N) { 2 } else { 0 };
            let y1 = if m.has(NeighborMask::S) {
                CELL_SIZE - 2
            } else {
                CELL_SIZE
            };
            for ly in y0..y1 {
                darken_px(pb, sx + 13, sy + ly, 0.68);
                darken_px(pb, sx + 14, sy + ly, 0.84);
            }
            for ly in y0..y1 {
                for lx in 5..13 {
                    let px = sx + lx;
                    let py = sy + ly;
                    let c = if lx == 5 || lx == 12 {
                        pal.outline
                    } else if lx == 6 {
                        pal.thresh_hi
                    } else if lx == 11 {
                        palette::scale(pal.thresh_base, 0.72)
                    } else {
                        match theme {
                            Theme::Sewer => {
                                if (ly % 3) == 0 {
                                    pal.outline
                                } else if (ly % 3) == 1 {
                                    palette::mix(pal.thresh_base, pal.thresh_hi, 0.28)
                                } else {
                                    pal.thresh_base
                                }
                            }
                            Theme::Kitchen => {
                                if lx == 8 || lx == 9 {
                                    palette::scale(pal.thresh_hi, if lx == 8 { 0.92 } else { 0.72 })
                                } else {
                                    pal.thresh_base
                                }
                            }
                            Theme::Lab => {
                                if lx == 8 || lx == 9 {
                                    if (ly & 1) == 0 {
                                        pal.thresh_base
                                    } else {
                                        palette::scale(pal.thresh_base, 0.72)
                                    }
                                } else {
                                    pal.thresh_base
                                }
                            }
                        }
                    };
                    blend_px(pb, px, py, c, 0.90);
                }
            }
            // Shadow cast south from the north doorway jamb onto the floor
            if m.has(NeighborMask::N) {
                for lx in 0..CELL_SIZE {
                    darken_px(pb, sx + lx, sy + 2, 0.46);
                    darken_px(pb, sx + lx, sy + 3, 0.68);
                    darken_px(pb, sx + lx, sy + 4, 0.84);
                }
            }
            if m.has(NeighborMask::S) {
                for dy in 0..3 {
                    for lx in 1..(CELL_SIZE + 1) {
                        darken_px(pb, sx + lx, sy + CELL_SIZE + dy, 0.46 + dy as f32 * 0.16);
                    }
                }
            }
        }
    }
}

fn draw_oneway_chevron(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    dir: Cardinal,
    open_progress: f32,
    anim_clock: f32,
    accent: Rgb,
) {
    // Draw a bold, unmistakable twin-chevron stencil on the approach side +
    // exit lip so the pass-through direction is readable even when closed.
    let pulse = 0.65 + 0.25 * (anim_clock * 3.2).sin() + 0.15 * open_progress.clamp(0.0, 1.0);
    let col_hi = palette::scale(accent, pulse.clamp(0.45, 1.0));
    let col_dim = palette::scale(accent, (pulse * 0.62).clamp(0.25, 0.75));
    let cx = sx + 8;
    let cy = sy + 8;

    let draw_v = |pb: &mut PixelBuffer, tip_x: i32, tip_y: i32, c: Rgb, alpha: f32| {
        for d in 0..=3i32 {
            match dir {
                Cardinal::North => {
                    blend_px(pb, tip_x - d, tip_y + d, c, alpha);
                    blend_px(pb, tip_x + d, tip_y + d, c, alpha);
                }
                Cardinal::South => {
                    blend_px(pb, tip_x - d, tip_y - d, c, alpha);
                    blend_px(pb, tip_x + d, tip_y - d, c, alpha);
                }
                Cardinal::West => {
                    blend_px(pb, tip_x + d, tip_y - d, c, alpha);
                    blend_px(pb, tip_x + d, tip_y + d, c, alpha);
                }
                Cardinal::East => {
                    blend_px(pb, tip_x - d, tip_y - d, c, alpha);
                    blend_px(pb, tip_x - d, tip_y + d, c, alpha);
                }
            }
        }
    };

    match dir {
        Cardinal::North => {
            draw_v(pb, cx, cy - 4, col_hi, 0.85);
            draw_v(pb, cx, cy + 2, col_dim, 0.78);
        }
        Cardinal::South => {
            draw_v(pb, cx, cy + 6, col_hi, 0.85);
            draw_v(pb, cx, cy + 1, col_dim, 0.78);
        }
        Cardinal::West => {
            draw_v(pb, cx - 5, cy, col_hi, 0.85);
            draw_v(pb, cx + 2, cy, col_dim, 0.78);
        }
        Cardinal::East => {
            draw_v(pb, cx + 5, cy, col_hi, 0.85);
            draw_v(pb, cx - 2, cy, col_dim, 0.78);
        }
    }
}

// ============================================================================
// LAYER 2: WALL BODIES (TOP FACE, SOUTH FRONT FACE, JAMBS, GATES)
// ============================================================================

/// **Wall body layer**: draw one grid wall cell (top face, south vertical front
/// face, material variants, jambs, rat-hole piers, or one-way gate).
///
/// Call after [`draw_wall_ground`] and floor decals. Zero heap allocation.
pub fn draw_wall_cell(pb: &mut PixelBuffer, view: &WallCellView, cam: Vec2, accent: Rgb) {
    let (sx, sy) = cell_screen_origin(view, cam);
    if culled(pb, sx, sy) {
        return;
    }
    let pal = theme_palette(view.theme);

    match view.kind {
        WallKind::Solid => {
            draw_solid_cell(pb, sx, sy, view, &pal, accent, 0..CELL_SIZE, 0..CELL_SIZE)
        }
        WallKind::Maze => draw_maze_cell(pb, sx, sy, view, &pal, accent),
        WallKind::RatHole { axis } => draw_rathole_cell(pb, sx, sy, view, &pal, accent, axis),
        WallKind::Doorway { axis } => draw_doorway_jambs(pb, sx, sy, view, &pal, accent, axis),
        WallKind::OneWayDoor { dir } => {
            draw_doorway_jambs(pb, sx, sy, view, &pal, accent, dir.wall_axis());
            draw_oneway_gate(pb, sx, sy, view, &pal, accent, dir);
        }
    }
}

/// Core solid wall renderer, parameterized by local `x_range` and `y_range` so
/// [`WallKind::RatHole`] can reuse the exact theme materials for its flanking piers.
#[allow(clippy::too_many_arguments)]
fn draw_solid_cell(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    pal: &WallPalette,
    accent: Rgb,
    x_range: std::ops::Range<i32>,
    y_range: std::ops::Range<i32>,
) {
    let m = view.mask;
    let has_n = m.has(NeighborMask::N);
    let has_e = m.has(NeighborMask::E);
    let has_s = m.has(NeighborMask::S);
    let has_w = m.has(NeighborMask::W);
    let has_ne = m.has(NeighborMask::NE);
    let has_se = m.has(NeighborMask::SE);
    let has_sw = m.has(NeighborMask::SW);
    let has_nw = m.has(NeighborMask::NW);

    let has_front = !has_s;
    let top_h = if has_front { TOP_FACE_H } else { CELL_SIZE };

    let cell_h = hash3(view.cell_x, view.cell_y, view.seed);
    let bay_h = hash3(view.cell_x.div_euclid(2), view.cell_y, view.seed ^ 0x5A3C);
    let (wx0, wy0) = view.world_origin();

    let bw = pb.w as i32;
    let bh = pb.h as i32;

    for ly in y_range {
        let py = sy + ly;
        if py < 0 || py >= bh {
            continue;
        }
        let wy = wy0 + ly;
        let is_front = has_front && ly >= top_h;

        for lx in x_range.clone() {
            let px = sx + lx;
            if px < 0 || px >= bw {
                continue;
            }
            let wx = wx0 + lx;

            // --- 1. Crisp 1px outer & concave inner outlines ---
            let outer_n = ly == 0 && !has_n;
            let outer_s = ly == CELL_SIZE - 1 && !has_s;
            let outer_w = lx == 0 && !has_w;
            let outer_e = lx == CELL_SIZE - 1 && !has_e;
            let inner_nw = lx == 0 && ly == 0 && has_n && has_w && !has_nw;
            let inner_ne = lx == CELL_SIZE - 1 && ly == 0 && has_n && has_e && !has_ne;
            let inner_sw_edge = lx == 0 && ly >= TOP_FACE_H && has_s && has_w && !has_sw;
            let inner_se_edge =
                lx == CELL_SIZE - 1 && ly >= TOP_FACE_H && has_s && has_e && !has_se;

            if outer_n
                || outer_s
                || outer_w
                || outer_e
                || inner_nw
                || inner_ne
                || inner_sw_edge
                || inner_se_edge
            {
                pb.plot(px, py, pal.outline);
                continue;
            }

            // --- 2. South vertical front face (ly in 10..15) ---
            if is_front {
                let fy = ly - TOP_FACE_H; // 0..5 (ly == 15 is outline above)
                let mut c = sample_front_material(
                    view.theme,
                    wx,
                    wy,
                    lx,
                    fy,
                    cell_h,
                    bay_h,
                    has_w,
                    has_e,
                    pal,
                    accent,
                    view.anim_clock,
                );
                // Crisp horizontal cast shadow right beneath the protruding top cornice
                if fy == 0 {
                    c = palette::scale(c, 0.58);
                }
                // Ambient occlusion cast onto south face from protruding west L-wall (top-left light!)
                if has_w && has_sw && lx < 3 {
                    let ao = match lx {
                        0 => 0.44,
                        1 => 0.66,
                        _ => 0.84,
                    };
                    c = palette::scale(c, ao);
                }
                if has_e && has_se && lx == CELL_SIZE - 1 {
                    c = palette::scale(c, 0.72);
                }
                if !has_w && lx == 1 {
                    c = palette::scale(c, 1.18);
                }
                if !has_e && lx == CELL_SIZE - 2 {
                    c = palette::scale(c, 0.72);
                }
                pb.plot(px, py, c);
                continue;
            }

            // --- 3. Lit top face (ly in 0..top_h) ---
            let mut c = sample_top_material(view.theme, wx, wy, cell_h, pal);

            let bevel_n = ly == 1 && !has_n;
            let bevel_w = lx == 1 && (!has_w || (has_s && !has_sw && ly >= TOP_FACE_H));
            let bevel_e =
                lx == CELL_SIZE - 2 && (!has_e || (has_s && !has_se && ly >= TOP_FACE_H));
            let cornice = has_front && ly == TOP_FACE_H - 1;

            if cornice {
                c = palette::mix(c, pal.cornice, 0.74);
            } else if bevel_n || bevel_w {
                c = palette::mix(c, pal.top_hi, 0.68);
            } else if bevel_e {
                c = palette::mix(c, pal.top_lo, 0.56);
            }

            // Inner concave corner catchlights so top bevels wrap cleanly around bends
            if has_n && has_w && !has_nw && ((lx == 1 && ly == 0) || (lx == 0 && ly == 1)) {
                c = palette::mix(c, pal.top_hi, 0.54);
            }
            if has_n && has_e && !has_ne && lx == CELL_SIZE - 2 && ly == 0 {
                c = palette::mix(c, pal.top_lo, 0.45);
            }

            pb.plot(px, py, c);
        }
    }

    // --- 4. Cohesive per-cell surface accents on exposed south front face ---
    if has_front && x_range.start == 0 && x_range.end == CELL_SIZE {
        draw_front_accents(pb, sx, sy, view, cell_h, bay_h, pal, accent);
    }
}

fn sample_top_material(
    theme: Theme,
    wx: i32,
    wy: i32,
    cell_h: u32,
    pal: &WallPalette,
) -> Rgb {
    match theme {
        Theme::Sewer => {
            // World-aligned coursed damp coping stones (8x6 staggered blocks)
            let row = wy.div_euclid(6);
            let stagger = (row & 1) * 4;
            let col = (wx + stagger).div_euclid(8);
            let bx = (wx + stagger).rem_euclid(8);
            let by = wy.rem_euclid(6);
            let sh = hash3(col, row, 0x1147);

            let mut c = if sh & 3 == 0 {
                pal.top_alt
            } else if sh & 7 == 1 {
                palette::scale(pal.top_base, 1.08)
            } else {
                pal.top_base
            };
            if bx == 0 || by == 0 {
                c = palette::mix(c, pal.mortar, 0.64);
            } else if by == 1 {
                c = palette::mix(c, pal.top_hi, 0.16);
            }
            // Moss patches creeping from mortar seams
            if (cell_h & 7) < 3 && (sh & 7) == 5 && by <= 2 {
                c = palette::mix(c, pal.detail_a, 0.52);
            }
            // Hairline crack
            if (cell_h & 15) == 4 && bx == 3 && (by == 2 || by == 3) {
                c = palette::scale(c, 0.66);
            }
            c
        }
        Theme::Kitchen => {
            // Warm butcher-plaster & oak crown molding with world-aligned wood planks
            let plank = wy.div_euclid(5);
            let py_in = wy.rem_euclid(5);
            let seam_x = (wx + (plank & 1) * 11).rem_euclid(22);
            let ph = hash3(plank, (wx + (plank & 1) * 11).div_euclid(22), 0x2291);

            let mut c = if (ph & 1) == 0 {
                pal.top_base
            } else {
                pal.top_alt
            };
            if py_in == 0 {
                c = palette::mix(c, pal.mortar, 0.32);
            } else if py_in == 1 {
                c = palette::mix(c, pal.top_hi, 0.14);
            }
            if seam_x == 0 {
                c = palette::mix(c, pal.mortar, 0.48);
            }
            if ((wx + wy * 3) & 7) == 0 {
                c = palette::scale(c, 0.93);
            }
            c
        }
        Theme::Lab => {
            // Staggered interlocking hull armor plates (20x8 shifted by 10 per row — zero 16px grid echo!)
            let row = wy.div_euclid(8);
            let stagger = (row & 1) * 10;
            let col = (wx + stagger).div_euclid(20);
            let bx = (wx + stagger).rem_euclid(20);
            let by = wy.rem_euclid(8);
            let ph = hash3(col, row, 0x33F5);

            let mut c = if (ph & 3) == 0 {
                pal.top_alt
            } else if (ph & 7) == 2 {
                palette::scale(pal.top_base, 1.07)
            } else {
                pal.top_base
            };
            // Recessed plate seams
            if bx == 0 || by == 0 {
                c = palette::mix(c, pal.mortar, 0.72);
            } else if by == 1 && bx > 1 && bx < 19 {
                c = palette::mix(c, pal.top_hi, 0.20);
            }
            // Corner hex-bolts on hull plates
            if (bx == 2 || bx == 17) && (by == 2 || by == 6) {
                c = palette::mix(c, pal.trim_hi, 0.50);
            }
            // Recessed cable conduit channel on hashed plates
            if (ph & 7) == 5 && by == 4 && (4..=15).contains(&bx) {
                c = palette::mix(c, pal.mortar, 0.62);
            }
            c
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sample_front_material(
    theme: Theme,
    wx: i32,
    _wy: i32,
    lx: i32,
    fy: i32, // 0..5 (0 = under cornice, 4 = above ground outline)
    cell_h: u32,
    bay_h: u32,
    has_w: bool,
    has_e: bool,
    pal: &WallPalette,
    _accent: Rgb,
    _anim_clock: f32,
) -> Rgb {
    match theme {
        Theme::Sewer => {
            // Two distinct courses of weathered sewer brick + damp waterline grime
            let course = if fy < 2 { 0 } else { 1 };
            let bx = (wx + course * 3).rem_euclid(6);
            let bh = hash3((wx + course * 3).div_euclid(6), course, 0x4129);
            let mut c = if (bh & 1) == 0 {
                pal.front_base
            } else {
                pal.front_alt
            };
            if fy == 2 || bx == 0 {
                c = pal.mortar;
            } else if fy == 1 && bx == 1 {
                // Subtle lit upper-left brick corner catchlight
                c = palette::mix(c, pal.top_hi, 0.16);
            } else if fy == 4 {
                c = palette::mix(c, pal.front_dark, 0.68);
            }
            // Moss tongues spilling down from the cornice over masonry
            if (cell_h & 7) < 3 {
                let mx = 3 + ((cell_h >> 3) % 9) as i32;
                let md = (lx - mx).abs();
                if md <= 2 && fy <= (3 - md).max(0) {
                    let tip = if fy == 1 { 1.15 } else { 0.88 };
                    c = palette::mix(c, palette::scale(pal.detail_a, tip), 0.78);
                }
            }
            c
        }
        Theme::Kitchen => {
            // Alternate multi-cell runs between warm oak kitchen cabinets and
            // crisp cream-ochre subway-tiled wainscoting over a dark oak skirting board!
            let is_cabinet = (bay_h & 1) == 0 && (has_w || has_e);
            if is_cabinet {
                if fy == 4 {
                    return pal.front_dark; // recessed dark toe-kick plinth
                }
                let is_stile = lx == 0 || lx == 7 || lx == 8 || lx == 15;
                let in_panel = !is_stile && (fy == 1 || fy == 2);
                if lx == 0 || lx == 15 {
                    palette::scale(pal.front_base, 0.56) // cabinet bay divider
                } else if lx == 7 {
                    palette::scale(pal.front_base, 0.66) // double-door center seam
                } else if lx == 8 {
                    palette::scale(pal.front_base, 1.12) // right-door lit bevel
                } else if in_panel {
                    if fy == 1 {
                        palette::scale(pal.front_base, 0.72) // recessed panel shadow
                    } else {
                        palette::scale(pal.front_base, 1.12) // recessed panel warm face
                    }
                } else {
                    pal.front_base
                }
            } else {
                // Cream-ochre subway-tiled kitchen dado (`fy 0..3`, two staggered tile courses)
                // over a warm dark mahogany skirting board (`fy 3..5`)!
                if fy >= 3 {
                    return if fy == 3 {
                        palette::scale(pal.front_base, 0.88) // skirting top molding lip
                    } else {
                        pal.front_dark // deep mahogany baseboard skirting
                    };
                }
                let course = if fy <= 1 { 0 } else { 1 };
                let tile_x = (wx + course * 2).rem_euclid(5);
                if tile_x == 0 {
                    palette::mix(pal.front_alt, pal.mortar, 0.56)
                } else {
                    let tile_id = (wx + course * 2).div_euclid(5);
                    let mut tone = if ((tile_id + course) & 1) == 0 {
                        pal.front_alt
                    } else {
                        palette::scale(pal.front_alt, 0.91)
                    };
                    if fy == 2 {
                        // Subtle horizontal grout separation between upper & lower courses
                        tone = palette::scale(tone, 0.86);
                    } else if fy == 1 && tile_x == 1 {
                        tone = palette::mix(tone, pal.detail_b, 0.48);
                    }
                    tone
                }
            }
        }
        Theme::Lab => {
            // Steel bulkhead fascia with structural gusset ribs & bold 2px-tall
            // diagonal ochre/charcoal warning stripes on hazard bays and corners!
            let hazard = (bay_h & 3) == 1 || (!has_w || !has_e);
            if hazard && (fy == 2 || fy == 3) {
                let stripe = (wx + fy).rem_euclid(6);
                let c = if stripe < 3 {
                    if fy == 2 {
                        pal.detail_a
                    } else {
                        palette::scale(pal.detail_a, 0.82)
                    }
                } else {
                    pal.front_dark
                };
                return c;
            }
            if fy == 4 {
                return pal.front_dark; // bolted floor flange
            }
            let mut c = pal.front_base;
            if lx == 3 || lx == 12 {
                c = palette::scale(pal.front_base, 0.64);
            } else if lx == 4 || lx == 13 {
                c = palette::mix(pal.front_alt, pal.trim_hi, 0.28);
            }
            if fy == 2 && lx > 4 && lx < 12 {
                c = palette::scale(c, 0.82);
            }
            c
        }
    }
}

/// Overlay multi-pixel architectural features on south-facing solid walls
/// (continuous sewer pipe-runs, brass cabinet pulls, lab louvers & status LEDs).
#[allow(clippy::too_many_arguments)]
fn draw_front_accents(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    cell_h: u32,
    bay_h: u32,
    pal: &WallPalette,
    accent: Rgb,
) {
    let front_y = sy + TOP_FACE_H;
    let has_w = view.mask.has(NeighborMask::W);
    let has_e = view.mask.has(NeighborMask::E);

    match view.theme {
        Theme::Sewer => {
            // Continuous bolted iron pipe-run spanning cohesive 2-cell bays
            if (bay_h & 3) == 2 && has_w && has_e {
                for lx in 0..CELL_SIZE {
                    let px = sx + lx;
                    let rust_hit = ((view.cell_x * 16 + lx) % 11) == 0;
                    let pipe_mid = if rust_hit {
                        palette::mix(pal.detail_b, pal.detail_c, 0.45)
                    } else {
                        pal.detail_b
                    };
                    pb.plot(px, front_y + 1, palette::mix(pipe_mid, pal.trim_hi, 0.52));
                    pb.plot(px, front_y + 2, pipe_mid);
                    pb.plot(px, front_y + 3, pal.trim_lo);
                }
                // Protruding pipe coupling collar + animated drip glint
                let fx = if (view.cell_x & 1) == 0 { 4 } else { 11 };
                pb.plot(sx + fx, front_y + 1, pal.trim_hi);
                pb.plot(sx + fx, front_y + 2, pal.trim_hi);
                pb.plot(sx + fx, front_y + 3, pal.outline);
                pb.plot(sx + fx + 1, front_y + 1, pal.detail_b);
                pb.plot(sx + fx + 1, front_y + 2, pal.trim_lo);
                pb.plot(sx + fx + 1, front_y + 3, pal.outline);
                let glint = 0.55 + 0.45 * (view.anim_clock * 2.4 + view.cell_x as f32).sin();
                pb.plot(sx + fx, front_y + 4, palette::scale(accent, 0.65 * glint));
            } else if (cell_h & 7) == 5 && has_w && has_e {
                // Recessed drainage weep-slit grille
                pb.rect_fill(sx + 5, front_y + 1, 6, 3, pal.outline);
                pb.plot(sx + 6, front_y + 2, pal.trim_hi);
                pb.plot(sx + 8, front_y + 2, pal.trim_hi);
                pb.plot(sx + 10, front_y + 2, pal.trim_hi);
            }
        }
        Theme::Kitchen => {
            // Twin gleaming brass cabinet knobs flanking the double-door center stile
            let is_cabinet = (bay_h & 1) == 0 && (has_w || has_e);
            if is_cabinet {
                pb.plot(sx + 5, front_y + 2, pal.detail_a);
                pb.plot(sx + 5, front_y + 3, pal.trim_lo);
                pb.plot(sx + 10, front_y + 2, pal.detail_a);
                pb.plot(sx + 10, front_y + 3, pal.trim_lo);
            }
        }
        Theme::Lab => {
            let hazard = (bay_h & 3) == 1 || (!has_w || !has_e);
            if !hazard && (cell_h & 7) == 4 && has_w && has_e {
                // Recessed cooling louver grille + subtle pulsing diagnostic LED
                for lx in (5..=10).step_by(2) {
                    pb.plot(sx + lx, front_y + 1, pal.outline);
                    pb.plot(sx + lx, front_y + 2, pal.outline);
                }
                let pulse = 0.65 + 0.35 * (view.anim_clock * 2.8 + view.cell_x as f32 * 1.7).sin();
                pb.plot(sx + 14, front_y + 1, palette::scale(accent, pulse));
            } else if !hazard && (cell_h & 15) == 1 && has_w && has_e {
                let pip = if (cell_h & 16) == 0 {
                    palette::scale(accent, 0.85)
                } else {
                    pal.detail_a
                };
                pb.plot(sx + 8, front_y + 1, pip);
            }
        }
    }
}

// ---- Thin Maze Partition Renderer ------------------------------------------

#[inline]
fn in_maze_top(lx: i32, ly: i32, m: NeighborMask) -> bool {
    if !(0..CELL_SIZE).contains(&lx) || !(0..CELL_SIZE).contains(&ly) {
        return false;
    }
    let has_n = m.has(NeighborMask::N);
    let has_e = m.has(NeighborMask::E);
    let has_s = m.has(NeighborMask::S);
    let has_w = m.has(NeighborMask::W);

    let y_top = if has_n { 0 } else { MAZE_INSET_Y };
    let y_bot = if has_s {
        CELL_SIZE
    } else {
        CELL_SIZE - MAZE_INSET_Y - MAZE_FRONT_H // 8
    };
    let in_v = (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx) && ly >= y_top && ly < y_bot;

    let x_left = if has_w { 0 } else { MAZE_INSET_X };
    let x_right = if has_e {
        CELL_SIZE
    } else {
        CELL_SIZE - MAZE_INSET_X
    };
    let in_h = (MAZE_INSET_Y..CELL_SIZE - MAZE_INSET_Y - MAZE_FRONT_H).contains(&ly)
        && lx >= x_left
        && lx < x_right;

    in_v || in_h
}

#[inline]
fn in_maze_front(lx: i32, ly: i32, m: NeighborMask) -> bool {
    if !(0..CELL_SIZE).contains(&lx) || !(0..CELL_SIZE).contains(&ly) {
        return false;
    }
    let y_front_start = CELL_SIZE - MAZE_INSET_Y - MAZE_FRONT_H; // 8
    let y_front_end = CELL_SIZE - MAZE_INSET_Y; // 13
    if ly < y_front_start || ly >= y_front_end {
        return false;
    }
    if in_maze_top(lx, ly, m) {
        return false;
    }
    let has_e = m.has(NeighborMask::E);
    let has_s = m.has(NeighborMask::S);
    let has_w = m.has(NeighborMask::W);

    let on_core = (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx) && !has_s;
    let on_w = has_w && lx < MAZE_INSET_X;
    let on_e = has_e && lx >= CELL_SIZE - MAZE_INSET_X;
    on_core || on_w || on_e
}

#[inline]
fn in_maze_any(lx: i32, ly: i32, m: NeighborMask) -> bool {
    if lx < 0 {
        return m.has(NeighborMask::W) && (MAZE_INSET_Y..CELL_SIZE - MAZE_INSET_Y).contains(&ly);
    }
    if lx >= CELL_SIZE {
        return m.has(NeighborMask::E) && (MAZE_INSET_Y..CELL_SIZE - MAZE_INSET_Y).contains(&ly);
    }
    if ly < 0 {
        return m.has(NeighborMask::N) && (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx);
    }
    if ly >= CELL_SIZE {
        return m.has(NeighborMask::S) && (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx);
    }
    in_maze_top(lx, ly, m) || in_maze_front(lx, ly, m)
}

#[inline]
fn in_maze_top_ext(lx: i32, ly: i32, m: NeighborMask) -> bool {
    if lx < 0 {
        return m.has(NeighborMask::W)
            && (MAZE_INSET_Y..CELL_SIZE - MAZE_INSET_Y - MAZE_FRONT_H).contains(&ly);
    }
    if lx >= CELL_SIZE {
        return m.has(NeighborMask::E)
            && (MAZE_INSET_Y..CELL_SIZE - MAZE_INSET_Y - MAZE_FRONT_H).contains(&ly);
    }
    if ly < 0 {
        return m.has(NeighborMask::N) && (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx);
    }
    if ly >= CELL_SIZE {
        return m.has(NeighborMask::S) && (MAZE_INSET_X..CELL_SIZE - MAZE_INSET_X).contains(&lx);
    }
    in_maze_top(lx, ly, m)
}

fn draw_maze_cell(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    pal: &WallPalette,
    accent: Rgb,
) {
    let m = view.mask;
    let (wx0, wy0) = view.world_origin();
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let y_front_start = CELL_SIZE - MAZE_INSET_Y - MAZE_FRONT_H; // 8

    for ly in 0..CELL_SIZE {
        let py = sy + ly;
        if py < 0 || py >= bh {
            continue;
        }
        let wy = wy0 + ly;
        for lx in 0..CELL_SIZE {
            let px = sx + lx;
            if px < 0 || px >= bw {
                continue;
            }
            let wx = wx0 + lx;
            let top = in_maze_top(lx, ly, m);
            let front = !top && in_maze_front(lx, ly, m);
            if !top && !front {
                continue;
            }

            // Perimeter outline on any cardinal or concave diagonal border with open air
            let edge_n = !in_maze_any(lx, ly - 1, m);
            let edge_s = !in_maze_any(lx, ly + 1, m);
            let edge_w = !in_maze_any(lx - 1, ly, m);
            let edge_e = !in_maze_any(lx + 1, ly, m);
            let concave_nw = !in_maze_any(lx - 1, ly - 1, m);
            let concave_ne = !in_maze_any(lx + 1, ly - 1, m);
            let top_drop_w = top && in_maze_front(lx - 1, ly, m);
            let top_drop_e = top && in_maze_front(lx + 1, ly, m);

            if edge_n
                || edge_s
                || edge_w
                || edge_e
                || concave_nw
                || concave_ne
                || top_drop_w
                || top_drop_e
            {
                pb.plot(px, py, pal.outline);
                continue;
            }

            if top {
                let mut c = match view.theme {
                    Theme::Sewer => {
                        // Riveted weathered iron-slat culvert divider with rust flecks
                        let mut b = pal.maze_top;
                        if (wx + wy) % 6 == 0 {
                            b = palette::scale(b, 0.84);
                        }
                        if ((wx * 3 + wy * 5) & 15) == 0 {
                            b = palette::mix(b, pal.detail_c, 0.34);
                        }
                        if (wx & 7) == 0 && ly == MAZE_INSET_Y + 2 {
                            b = pal.maze_hi;
                        }
                        b
                    }
                    Theme::Kitchen => {
                        // Slatted oak pantry rack top with wrought-iron corner straps
                        let at_joint = (lx == MAZE_INSET_X + 1 || lx == CELL_SIZE - MAZE_INSET_X - 2)
                            && (ly == MAZE_INSET_Y + 1 || ly == y_front_start - 2);
                        if at_joint {
                            pal.detail_c
                        } else {
                            let slat = if ((wx + wy) & 3) == 0 { 0.84 } else { 1.04 };
                            palette::scale(pal.maze_top, slat)
                        }
                    }
                    Theme::Lab => {
                        // Frosted cyan polycarbonate panel framed in steel stanchion posts
                        let at_post = (wx % 8 == 0) && (ly == MAZE_INSET_Y + 1 || ly == y_front_start - 1);
                        if at_post {
                            pal.trim_hi
                        } else {
                            let sheen = if (wx + wy).rem_euclid(7) == 0 { 0.34 } else { 0.0 };
                            let base = palette::mix(pal.maze_top, accent, 0.16);
                            palette::mix(base, pal.detail_c, sheen)
                        }
                    }
                };

                let lit_n = !in_maze_top_ext(lx, ly - 2, m) && !in_maze_any(lx, ly - 2, m);
                let lit_w = !in_maze_top_ext(lx - 2, ly, m) && !in_maze_any(lx - 2, ly, m);
                let shade_e = !in_maze_top_ext(lx + 2, ly, m) && !in_maze_any(lx + 2, ly, m);
                let cornice = in_maze_front(lx, ly + 1, m);

                if cornice {
                    c = palette::mix(c, pal.maze_hi, 0.74);
                } else if lit_n || lit_w {
                    c = palette::mix(c, pal.maze_hi, 0.56);
                } else if shade_e {
                    c = palette::scale(c, 0.70);
                }
                pb.plot(px, py, c);
            } else {
                // 5 px tall maze partition south front face (`fy in 0..4`, `fy == 4` is outline)
                let fy = ly - y_front_start;
                let mut c = match view.theme {
                    Theme::Sewer => {
                        let rib = if (wx & 3) == 0 { 0.76 } else { 1.0 };
                        let mut b = palette::scale(pal.maze_front, rib);
                        if fy == 3 {
                            b = palette::mix(b, pal.detail_a, 0.42);
                        }
                        b
                    }
                    Theme::Kitchen => {
                        if fy == 3 {
                            pal.front_dark
                        } else {
                            let slat = if (wx & 2) == 0 { 0.78 } else { 1.06 };
                            palette::scale(pal.maze_front, slat)
                        }
                    }
                    Theme::Lab => {
                        if fy == 3 {
                            let s = (wx + fy).rem_euclid(6);
                            if s < 3 {
                                palette::scale(pal.detail_a, 0.86)
                            } else {
                                pal.front_dark
                            }
                        } else if (wx & 7) == 0 {
                            pal.trim_hi
                        } else {
                            palette::mix(pal.maze_front, pal.maze_top, 0.36)
                        }
                    }
                };
                if fy == 0 {
                    c = palette::scale(c, 0.60);
                }
                if in_maze_top(lx - 1, ly, m) || in_maze_top(lx - 2, ly, m) {
                    c = palette::scale(c, 0.56);
                }
                pb.plot(px, py, c);
            }
        }
    }
}

// ---- Rat-Hole Wall Renderer ------------------------------------------------

fn draw_rathole_cell(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    pal: &WallPalette,
    accent: Rgb,
    axis: WallAxis,
) {
    match axis {
        WallAxis::Horizontal => {
            let mut left_view = *view;
            left_view.mask = NeighborMask(view.mask.0 & !NeighborMask::E);
            let mut right_view = *view;
            right_view.mask = NeighborMask(view.mask.0 & !NeighborMask::W);

            draw_solid_cell(
                pb,
                sx,
                sy,
                &left_view,
                pal,
                accent,
                0..RAT_HOLE_PIER,
                0..CELL_SIZE,
            );
            draw_solid_cell(
                pb,
                sx,
                sy,
                &right_view,
                pal,
                accent,
                (CELL_SIZE - RAT_HOLE_PIER)..CELL_SIZE,
                0..CELL_SIZE,
            );

            // Frame only the lower opening (`ly in 5..16`) so the overhang lintel
            // (`ly in 0..5`) connects across the top without a vertical seam!
            for ly in 5..CELL_SIZE {
                pb.plot(sx + RAT_HOLE_PIER - 1, sy + ly, pal.outline);
                pb.plot(sx + CELL_SIZE - RAT_HOLE_PIER, sy + ly, pal.outline);
            }
            for ly in 5..(CELL_SIZE - 1) {
                let c = if !view.mask.has(NeighborMask::S) && ly >= TOP_FACE_H {
                    palette::scale(pal.front_alt, 1.15)
                } else {
                    pal.top_hi
                };
                pb.plot(sx + CELL_SIZE - RAT_HOLE_PIER + 1, sy + ly, c);
            }
        }
        WallAxis::Vertical => {
            let mut top_view = *view;
            top_view.mask = NeighborMask(view.mask.0 | NeighborMask::S);
            let mut bot_view = *view;
            bot_view.mask = NeighborMask(view.mask.0 & !NeighborMask::N);

            draw_solid_cell(
                pb,
                sx,
                sy,
                &top_view,
                pal,
                accent,
                0..CELL_SIZE,
                0..RAT_HOLE_PIER,
            );
            draw_solid_cell(
                pb,
                sx,
                sy,
                &bot_view,
                pal,
                accent,
                0..CELL_SIZE,
                (CELL_SIZE - RAT_HOLE_PIER)..CELL_SIZE,
            );

            // Frame only the outer tunnel mouths (`lx in 0..5` and `lx in 11..16`) so the
            // central overhang roof bridge (`lx in 6..10`) flows continuously into both piers!
            for lx in 0..CELL_SIZE {
                if (6..10).contains(&lx) {
                    continue;
                }
                pb.plot(sx + lx, sy + RAT_HOLE_PIER - 1, pal.outline);
                pb.plot(sx + lx, sy + CELL_SIZE - RAT_HOLE_PIER, pal.outline);
            }
            for lx in 1..(CELL_SIZE - 1) {
                if (6..10).contains(&lx) {
                    continue;
                }
                pb.plot(sx + lx, sy + CELL_SIZE - RAT_HOLE_PIER + 1, pal.top_hi);
            }
        }
    }
}

// ---- Doorway & One-Way Door Renderers --------------------------------------

fn draw_doorway_jambs(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    pal: &WallPalette,
    accent: Rgb,
    axis: WallAxis,
) {
    let m = view.mask;
    match axis {
        WallAxis::Horizontal => {
            if m.has(NeighborMask::W) {
                draw_jamb_pillar_v(pb, sx, sy, pal, accent, view.theme, true);
            }
            if m.has(NeighborMask::E) {
                draw_jamb_pillar_v(
                    pb,
                    sx + CELL_SIZE - DOORWAY_JAMB_VISUAL,
                    sy,
                    pal,
                    accent,
                    view.theme,
                    false,
                );
            }
        }
        WallAxis::Vertical => {
            if m.has(NeighborMask::N) {
                draw_jamb_pillar_h(pb, sx, sy, pal, accent, view.theme, true);
            }
            if m.has(NeighborMask::S) {
                draw_jamb_pillar_h(
                    pb,
                    sx,
                    sy + CELL_SIZE - DOORWAY_JAMB_VISUAL,
                    pal,
                    accent,
                    view.theme,
                    false,
                );
            }
        }
    }
}

fn draw_jamb_pillar_v(
    pb: &mut PixelBuffer,
    x: i32,
    y: i32,
    pal: &WallPalette,
    accent: Rgb,
    theme: Theme,
    is_left: bool,
) {
    // 2 px wide sculpted pilaster matching the wall's exact TOP_FACE_H / FRONT_FACE_H
    for ly in 0..CELL_SIZE {
        for dx in 0..DOORWAY_JAMB_VISUAL {
            let px = x + dx;
            let py = y + ly;
            let inner = (is_left && dx == 1) || (!is_left && dx == 0);
            if ly == 0 || ly == CELL_SIZE - 1 {
                pb.plot(px, py, pal.outline);
                continue;
            }
            if ly >= TOP_FACE_H {
                let c = if inner {
                    pal.outline
                } else if ly == TOP_FACE_H || ly == CELL_SIZE - 2 {
                    pal.front_dark
                } else {
                    palette::mix(pal.front_alt, pal.trim_hi, 0.32)
                };
                pb.plot(px, py, c);
            } else {
                let c = if ly == TOP_FACE_H - 1 {
                    pal.cornice
                } else if ly == 1 {
                    pal.top_hi
                } else if inner {
                    if is_left {
                        pal.top_lo
                    } else {
                        pal.trim_hi
                    }
                } else {
                    pal.top_base
                };
                pb.plot(px, py, c);
            }
        }
    }
    let pip_x = if is_left { x } else { x + 1 };
    let pip_col = match theme {
        Theme::Lab => palette::scale(accent, 0.90),
        Theme::Kitchen => pal.detail_a,
        Theme::Sewer => pal.trim_hi,
    };
    pb.plot(pip_x, y + TOP_FACE_H - 2, pip_col);
}

fn draw_jamb_pillar_h(
    pb: &mut PixelBuffer,
    x: i32,
    y: i32,
    pal: &WallPalette,
    accent: Rgb,
    theme: Theme,
    is_top: bool,
) {
    for dy in 0..DOORWAY_JAMB_VISUAL {
        for lx in 0..CELL_SIZE {
            let px = x + lx;
            let py = y + dy;
            if lx == 0 || lx == CELL_SIZE - 1 {
                pb.plot(px, py, pal.outline);
                continue;
            }
            let c = if is_top {
                // Shadowed stone/steel lintel return tucked beneath the north wall's south face
                if dy == 0 {
                    pal.front_dark
                } else if lx == 1 || lx == CELL_SIZE - 2 {
                    pal.trim_hi
                } else {
                    palette::scale(pal.front_alt, 0.85)
                }
            } else {
                // Sculpted threshold header sitting flush against the south wall's north bevel
                if dy == 0 {
                    pal.outline
                } else if lx == 1 || lx == CELL_SIZE - 2 {
                    pal.trim_hi
                } else {
                    pal.top_hi
                }
            };
            pb.plot(px, py, c);
        }
    }
    let pip_y = y + 1;
    let pip_col = match theme {
        Theme::Lab => palette::scale(accent, 0.90),
        Theme::Kitchen => pal.detail_a,
        Theme::Sewer => pal.trim_hi,
    };
    pb.plot(x + 3, pip_y, pip_col);
    pb.plot(x + CELL_SIZE - 4, pip_y, pip_col);
}

fn draw_oneway_gate(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    pal: &WallPalette,
    accent: Rgb,
    dir: Cardinal,
) {
    let t = view.open_progress.clamp(0.0, 1.0);
    let swing = t * 1.28; // swings up to ~73° in `dir`
    let stop_pip: Rgb = (206, 74, 64);
    let leaf_col = match view.theme {
        Theme::Sewer => pal.detail_b,
        Theme::Kitchen => pal.maze_top,
        Theme::Lab => pal.maze_top,
    };
    let leaf_hi = pal.trim_hi;

    let (dx_dir, dy_dir): (f32, f32) = match dir {
        Cardinal::North => (0.0, -1.0),
        Cardinal::South => (0.0, 1.0),
        Cardinal::East => (1.0, 0.0),
        Cardinal::West => (-1.0, 0.0),
    };

    match dir.wall_axis() {
        WallAxis::Horizontal => {
            let cy = sy as f32 + 8.0;
            let cos_s = swing.cos();
            let sin_s = swing.sin();
            let lx0 = sx as f32 + 2.0;
            let lx1 = lx0 + 5.5 * cos_s;
            let ly1 = cy + 5.5 * sin_s * dy_dir;
            let rx0 = sx as f32 + 13.0;
            let rx1 = rx0 - 5.5 * cos_s;
            let ry1 = cy + 5.5 * sin_s * dy_dir;

            draw_gate_leaf(pb, lx0, cy, lx1, ly1, pal.outline, leaf_col, leaf_hi);
            draw_gate_leaf(pb, rx0, cy, rx1, ry1, pal.outline, leaf_col, leaf_hi);

            // Hinge pins on the jambs
            pb.plot(sx + 2, sy + 8, leaf_hi);
            pb.plot(sx + 13, sy + 8, leaf_hi);

            // Blocked-side crimson stop flange & approach-side push pip when closed
            if t < 0.35 {
                let stop_y = if matches!(dir, Cardinal::North) {
                    sy + 6
                } else {
                    sy + 10
                };
                pb.plot(sx + 5, stop_y, stop_pip);
                pb.plot(sx + 8, stop_y, stop_pip);
                pb.plot(sx + 10, stop_y, stop_pip);
                let go_y = if matches!(dir, Cardinal::North) {
                    sy + 10
                } else {
                    sy + 6
                };
                pb.plot(sx + 7, go_y, palette::scale(accent, 0.90));
                pb.plot(sx + 8, go_y, palette::scale(accent, 0.90));
            }
        }
        WallAxis::Vertical => {
            let cx = sx as f32 + 8.0;
            let cos_s = swing.cos();
            let sin_s = swing.sin();
            let ty0 = sy as f32 + 2.0;
            let tx1 = cx + 5.5 * sin_s * dx_dir;
            let ty1 = ty0 + 5.5 * cos_s;
            let by0 = sy as f32 + 13.0;
            let bx1 = cx + 5.5 * sin_s * dx_dir;
            let by1 = by0 - 5.5 * cos_s;

            draw_gate_leaf(pb, cx, ty0, tx1, ty1, pal.outline, leaf_col, leaf_hi);
            draw_gate_leaf(pb, cx, by0, bx1, by1, pal.outline, leaf_col, leaf_hi);

            pb.plot(sx + 8, sy + 2, leaf_hi);
            pb.plot(sx + 8, sy + 13, leaf_hi);

            if t < 0.35 {
                let stop_x = if matches!(dir, Cardinal::West) {
                    sx + 6
                } else {
                    sx + 10
                };
                pb.plot(stop_x, sy + 5, stop_pip);
                pb.plot(stop_x, sy + 8, stop_pip);
                pb.plot(stop_x, sy + 10, stop_pip);
                let go_x = if matches!(dir, Cardinal::West) {
                    sx + 10
                } else {
                    sx + 6
                };
                pb.plot(go_x, sy + 7, palette::scale(accent, 0.90));
                pb.plot(go_x, sy + 8, palette::scale(accent, 0.90));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gate_leaf(
    pb: &mut PixelBuffer,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    outline: Rgb,
    fill: Rgb,
    hi: Rgb,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(0.5);
    let nx = -dy / len;
    let ny = dx / len;
    let steps = 10;

    // Soft floor drop shadow under the swung gate leaf
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        let x = (x0 + dx * f).round() as i32 + 1;
        let y = (y0 + dy * f).round() as i32 + 2;
        darken_px(pb, x, y, 0.56);
    }
    // 1px dark rim around the swinging bar
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        let fx = x0 + dx * f;
        let fy = y0 + dy * f;
        pb.plot((fx + nx).round() as i32, (fy + ny).round() as i32, outline);
        pb.plot((fx - nx).round() as i32, (fy - ny).round() as i32, outline);
    }
    // Crisp lit metal/wood bar spine
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        let x = (x0 + dx * f).round() as i32;
        let y = (y0 + dy * f).round() as i32;
        let c = if i <= 1 || i == steps { hi } else { fill };
        pb.plot(x, y, c);
    }
}

// ============================================================================
// LAYER 3: OVERHEAD LINTELS (OPTIONAL, DRAWN AFTER ENTITIES)
// ============================================================================

/// **Overhang layer** (optional): draw upper lintel beams over horizontal
/// [`WallKind::RatHole`], [`WallKind::Doorway`], and north/south
/// [`WallKind::OneWayDoor`] cells so the rat and enemies visibly duck *under*
/// arches and wall openings.
///
/// Call **after** drawing entities and **before** HUD/particles. No-op for
/// `Solid`, `Maze`, and `Vertical` doorways. Zero heap allocation.
pub fn draw_wall_overhang(pb: &mut PixelBuffer, view: &WallCellView, cam: Vec2, _accent: Rgb) {
    let (sx, sy) = cell_screen_origin(view, cam);
    if culled(pb, sx, sy) {
        return;
    }
    let pal = theme_palette(view.theme);
    let cell_h = hash3(view.cell_x, view.cell_y, view.seed);
    let (wx0, wy0) = view.world_origin();

    match view.kind {
        WallKind::RatHole {
            axis: WallAxis::Horizontal,
        } => {
            // Seamless world-textured 5 px wall lintel bridging `lx in 3..13, ly in 0..5`
            // with 1 px arched corner chamfers at `(4, 5)` and `(11, 5)`!
            let has_n = view.mask.has(NeighborMask::N);
            for ly in 0..5 {
                let py = sy + ly;
                let wy = wy0 + ly;
                for lx in (RAT_HOLE_PIER - 1)..(CELL_SIZE - RAT_HOLE_PIER + 1) {
                    let px = sx + lx;
                    let wx = wx0 + lx;
                    let mut c = sample_top_material(view.theme, wx, wy, cell_h, &pal);
                    if (ly == 0 && !has_n) || ly == 4 {
                        c = pal.outline;
                    } else if ly == 1 && !has_n {
                        c = palette::mix(c, pal.top_hi, 0.68);
                    } else if ly == 3 {
                        c = palette::mix(c, pal.cornice, 0.72);
                    }
                    pb.plot(px, py, c);
                }
            }
            // Subtle 1px arch-corner haunches framing the top of the 8px opening
            pb.plot(sx + RAT_HOLE_PIER, sy + 5, pal.outline);
            pb.plot(sx + CELL_SIZE - RAT_HOLE_PIER - 1, sy + 5, pal.outline);
            // Underside cast shadow below the lintel onto passing rats / gap
            for lx in (RAT_HOLE_PIER + 1)..(CELL_SIZE - RAT_HOLE_PIER - 1) {
                darken_px(pb, sx + lx, sy + 5, 0.50);
                darken_px(pb, sx + lx, sy + 6, 0.74);
            }
        }
        WallKind::RatHole {
            axis: WallAxis::Vertical,
        } => {
            // World-textured 4 px continuous roof bridge (`lx in 6..10, ly in 3..13`)
            // seamlessly joining the north and south wall piers while both side mouths stay clear.
            for ly in (RAT_HOLE_PIER - 1)..(CELL_SIZE - RAT_HOLE_PIER + 1) {
                let wy = wy0 + ly;
                let in_gap = (RAT_HOLE_PIER..(CELL_SIZE - RAT_HOLE_PIER)).contains(&ly);
                if in_gap {
                    pb.plot(sx + 5, sy + ly, pal.outline);
                }
                for lx in 6..10 {
                    let wx = wx0 + lx;
                    let mut c = sample_top_material(view.theme, wx, wy, cell_h, &pal);
                    if in_gap {
                        if lx == 6 {
                            c = palette::mix(c, pal.top_hi, 0.66);
                        } else if lx == 9 {
                            c = palette::mix(c, pal.top_lo, 0.54);
                        }
                    }
                    pb.plot(sx + lx, sy + ly, c);
                }
                if in_gap {
                    pb.plot(sx + 10, sy + ly, pal.outline);
                    darken_px(pb, sx + 11, sy + ly, 0.56);
                    darken_px(pb, sx + 12, sy + ly, 0.78);
                }
            }
        }
        WallKind::Doorway {
            axis: WallAxis::Horizontal,
        }
        | WallKind::OneWayDoor {
            dir: Cardinal::North | Cardinal::South,
        } => {
            // Sleek 4 px architectural header beam (`ly in 0..4`) spanning across the doorway
            let x0 = if view.mask.has(NeighborMask::W) { 1 } else { 0 };
            let x1 = if view.mask.has(NeighborMask::E) {
                CELL_SIZE - 1
            } else {
                CELL_SIZE
            };
            for lx in x0..x1 {
                let wx = wx0 + lx;
                let base = sample_top_material(view.theme, wx, wy0 + 1, cell_h, &pal);
                pb.plot(sx + lx, sy, pal.outline);
                pb.plot(sx + lx, sy + 1, palette::mix(base, pal.top_hi, 0.52));
                pb.plot(sx + lx, sy + 2, palette::mix(base, pal.cornice, 0.45));
                pb.plot(sx + lx, sy + 3, pal.outline);
                darken_px(pb, sx + lx, sy + 4, 0.58);
                darken_px(pb, sx + lx, sy + 5, 0.80);
            }
        }
        _ => {}
    }
}

// ============================================================================
// TESTS & VISUAL SHEET DUMP
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor::draw_floor;
    use crate::game::art_rat::draw_rat;
    use crate::game::level::Level;
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;

    /// Hand-written 18 × 12 sample layout showing:
    /// - Left room with outer corners, long runs, 2-cell horizontal doorway, 1-cell vertical doorway
    /// - Horizontal rat-hole (north wall) & vertical rat-hole (lower alcove)
    /// - Central corridor + standalone 1×1 pillar
    /// - Right maze wing with clean navigable thin `Maze` partitions, L/T corners, dead ends &
    ///   N/E/S/W Maze-to-Solid junctions
    /// - Closed `OneWayDoor` (West) exiting maze reward room + swinging `OneWayDoor` (South)
    const SAMPLE_MAP: [&str; 12] = [
        "..................",
        ".###H############.",
        ".#....#...#..m..#.",
        ".#....d...d.mm..#.",
        ".#....#...#...mm#.",
        ".##DD##.#.#mm.m.#.",
        "..........#.....>.",
        ".######...#.mmm.#.",
        ".#....v...<...m.#.",
        ".#....#...#.m...#.",
        ".##o#############.",
        "..................",
    ];

    fn parse_cell(ch: u8) -> Option<(WallKind, f32)> {
        match ch {
            b'#' => Some((WallKind::Solid, 0.0)),
            b'm' => Some((WallKind::Maze, 0.0)),
            b'H' => Some((
                WallKind::RatHole {
                    axis: WallAxis::Horizontal,
                },
                0.0,
            )),
            b'v' => Some((
                WallKind::RatHole {
                    axis: WallAxis::Vertical,
                },
                0.0,
            )),
            b'D' => Some((
                WallKind::Doorway {
                    axis: WallAxis::Horizontal,
                },
                0.0,
            )),
            b'd' => Some((
                WallKind::Doorway {
                    axis: WallAxis::Vertical,
                },
                0.0,
            )),
            b'<' => Some((
                WallKind::OneWayDoor {
                    dir: Cardinal::West,
                },
                0.0,
            )),
            b'o' => Some((
                WallKind::OneWayDoor {
                    dir: Cardinal::South,
                },
                0.55,
            )),
            b'>' => Some((
                WallKind::OneWayDoor {
                    dir: Cardinal::East,
                },
                1.0,
            )),
            _ => None,
        }
    }

    fn grid_at(x: i32, y: i32) -> Option<(WallKind, f32)> {
        if y < 0 || y >= SAMPLE_MAP.len() as i32 {
            return None;
        }
        let row = SAMPLE_MAP[y as usize].as_bytes();
        if x < 0 || x >= row.len() as i32 {
            return None;
        }
        parse_cell(row[x as usize])
    }

    fn connects(self_kind: WallKind, other: Option<(WallKind, f32)>) -> bool {
        let Some((ok, _)) = other else {
            return false;
        };
        match self_kind {
            WallKind::Solid | WallKind::RatHole { .. } => {
                matches!(ok, WallKind::Solid | WallKind::RatHole { .. })
            }
            WallKind::Maze => {
                matches!(ok, WallKind::Maze | WallKind::Solid | WallKind::RatHole { .. })
            }
            WallKind::Doorway { .. } | WallKind::OneWayDoor { .. } => {
                matches!(ok, WallKind::Solid | WallKind::Maze | WallKind::RatHole { .. })
            }
        }
    }

    fn compute_mask(x: i32, y: i32, kind: WallKind) -> NeighborMask {
        NeighborMask::from_all(
            connects(kind, grid_at(x, y - 1)),
            connects(kind, grid_at(x + 1, y)),
            connects(kind, grid_at(x, y + 1)),
            connects(kind, grid_at(x - 1, y)),
            connects(kind, grid_at(x + 1, y - 1)),
            connects(kind, grid_at(x + 1, y + 1)),
            connects(kind, grid_at(x - 1, y + 1)),
            connects(kind, grid_at(x - 1, y - 1)),
        )
    }

    #[test]
    fn wall_colliders_match_spec() {
        let mut buf = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];

        let solid = WallCellView::new(2, 3, 7, Theme::Sewer, WallKind::Solid, NeighborMask::EMPTY);
        assert_eq!(wall_colliders(&solid, &mut buf), 1);
        assert_eq!(
            (buf[0].x, buf[0].y, buf[0].w, buf[0].h),
            (32.0, 48.0, 16.0, 16.0)
        );

        let rh = WallCellView::new(
            1,
            1,
            7,
            Theme::Kitchen,
            WallKind::RatHole {
                axis: WallAxis::Horizontal,
            },
            NeighborMask::from_cardinals(false, true, false, true),
        );
        assert_eq!(wall_colliders(&rh, &mut buf), 2);
        assert_eq!(buf[0].w, 4.0);
        assert_eq!(buf[1].x - buf[0].right(), RAT_HOLE_GAP_W as f32);
        let gap = rat_hole_gap_rect(1, 1, WallAxis::Horizontal);
        assert_eq!(gap.w, RAT_HOLE_GAP_W as f32);

        let door1 = WallCellView::new(
            0,
            0,
            1,
            Theme::Lab,
            WallKind::Doorway {
                axis: WallAxis::Horizontal,
            },
            NeighborMask::from_cardinals(false, true, false, true),
        );
        assert_eq!(wall_colliders(&door1, &mut buf), 2);
        let clear = buf[1].x - buf[0].right();
        assert_eq!(clear, 14.0);
        assert!(clear > 13.0, "1-cell doorway must fit Brutes (dia 13 px)");

        let maze = WallCellView::new(0, 0, 1, Theme::Lab, WallKind::Maze, NeighborMask::EMPTY);
        assert_eq!(wall_colliders(&maze, &mut buf), 1);
        assert_eq!(
            (buf[0].x, buf[0].y, buf[0].w, buf[0].h),
            (4.0, 3.0, 8.0, 10.0)
        );

        // 2-cell merged doorway leaves 30 px clear (> Boss diameter 28 px)
        let door_l = WallCellView::new(
            0,
            0,
            1,
            Theme::Sewer,
            WallKind::Doorway {
                axis: WallAxis::Horizontal,
            },
            NeighborMask::from_cardinals(false, false, false, true),
        );
        let door_r = WallCellView::new(
            1,
            0,
            1,
            Theme::Sewer,
            WallKind::Doorway {
                axis: WallAxis::Horizontal,
            },
            NeighborMask::from_cardinals(false, true, false, false),
        );
        assert_eq!(wall_colliders(&door_l, &mut buf), 1);
        let left_edge = buf[0].right();
        assert_eq!(wall_colliders(&door_r, &mut buf), 1);
        let right_edge = buf[0].x;
        assert_eq!(right_edge - left_edge, 30.0);
        assert!(right_edge - left_edge > 28.0, "2-cell doorway must fit Bosses");
    }

    #[test]
    fn maze_colliders_match_drawn_pixels_for_all_16_cardinal_masks() {
        let mut buf = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
        for bits in 0u8..16 {
            let mask = NeighborMask(bits);
            let view = WallCellView::new(0, 0, 99, Theme::Lab, WallKind::Maze, mask);
            let n = wall_colliders(&view, &mut buf);
            assert!(n == 1 || n == 2);
            for ly in 0..CELL_SIZE {
                for lx in 0..CELL_SIZE {
                    let drawn = in_maze_top(lx, ly, mask) || in_maze_front(lx, ly, mask);
                    let pt = Vec2::new(lx as f32 + 0.5, ly as f32 + 0.5);
                    let blocked = buf[..n].iter().any(|r| r.contains(pt));
                    assert_eq!(
                        drawn, blocked,
                        "maze mask {bits:#06b} mismatch at ({lx},{ly}): drawn={drawn} blocked={blocked}"
                    );
                }
            }
        }
    }

    #[test]
    fn long_wall_runs_vary_by_coordinate_hash() {
        for theme in Theme::all() {
            let accent = Level::new(theme).palette.accent;
            let mask = NeighborMask::from_cardinals(false, true, false, true);
            let mut snapshots = Vec::new();
            for cx in 0..8 {
                let mut pb = PixelBuffer::new(CELL_SIZE as usize, CELL_SIZE as usize);
                let view = WallCellView::new(cx, 3, 1337, theme, WallKind::Solid, mask);
                let cam = Vec2::new((cx * CELL_SIZE) as f32, (3 * CELL_SIZE) as f32);
                draw_wall_cell(&mut pb, &view, cam, accent);
                let mut cell_px = Vec::with_capacity(256);
                for y in 0..CELL_SIZE as usize {
                    for x in 0..CELL_SIZE as usize {
                        cell_px.push(pb.pixel_at(x, y));
                    }
                }
                snapshots.push(cell_px);
            }
            let distinct = snapshots
                .iter()
                .enumerate()
                .filter(|(i, a)| !snapshots[..*i].contains(a))
                .count();
            assert!(
                distinct >= 4,
                "{theme:?}: expected varied wall tiles across 8-cell run, got only {distinct} distinct"
            );
        }
    }

    #[test]
    fn wall_drawing_never_panics_for_any_variant_mask_or_position() {
        let mut pb = PixelBuffer::new(48, 36);
        let kinds = [
            WallKind::Solid,
            WallKind::Maze,
            WallKind::RatHole {
                axis: WallAxis::Horizontal,
            },
            WallKind::RatHole {
                axis: WallAxis::Vertical,
            },
            WallKind::Doorway {
                axis: WallAxis::Horizontal,
            },
            WallKind::Doorway {
                axis: WallAxis::Vertical,
            },
            WallKind::OneWayDoor {
                dir: Cardinal::North,
            },
            WallKind::OneWayDoor {
                dir: Cardinal::East,
            },
            WallKind::OneWayDoor {
                dir: Cardinal::South,
            },
            WallKind::OneWayDoor {
                dir: Cardinal::West,
            },
        ];
        let masks: [u8; 8] = [0x00, 0x0F, 0xFF, 0x05, 0x0A, 0x13, 0x55, 0xAA];
        let cams = [
            Vec2::new(0.0, 0.0),
            Vec2::new(-24.0, -24.0),
            Vec2::new(15.5, 15.5),
            Vec2::new(500.0, 500.0),
            Vec2::new(-500.0, -500.0),
        ];

        for theme in Theme::all() {
            let accent = Level::new(theme).palette.accent;
            for &kind in &kinds {
                for &m in &masks {
                    for &prog in &[0.0f32, 0.5, 1.0] {
                        for &cam in &cams {
                            let view = WallCellView {
                                cell_x: 1,
                                cell_y: 1,
                                seed: 42,
                                theme,
                                kind,
                                mask: NeighborMask(m),
                                open_progress: prog,
                                anim_clock: prog * 3.0,
                            };
                            draw_wall_ground(&mut pb, &view, cam, accent);
                            draw_wall_cell(&mut pb, &view, cam, accent);
                            draw_wall_overhang(&mut pb, &view, cam, accent);
                        }
                    }
                }
            }
        }
    }

    /// Visual sheet: `RATRUN_DUMP=walls cargo test dump_walls_sheet` writes
    /// `/tmp/ratrun_walls.ppm` showing the 18 × 12 sample layout over all three
    /// themes' floors, with rats placed for scale.
    #[test]
    fn dump_walls_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("walls") {
            return;
        }
        let cols = SAMPLE_MAP[0].len();
        let rows = SAMPLE_MAP.len();
        let w = cols * CELL_SIZE as usize;
        let h = rows * CELL_SIZE as usize;
        let sep = 2usize;
        let total_h = h * 3 + sep * 2;
        let mut out = format!("P6\n{} {}\n255\n", w, total_h).into_bytes();

        for (ti, theme) in Theme::all().into_iter().enumerate() {
            let level = Level::new(theme);
            let accent = level.palette.accent;
            let cam = Vec2::new(64.0, 64.0);
            let cx_off = 4;
            let cy_off = 4;
            let mut pb = PixelBuffer::new(w, h);
            draw_floor(&mut pb, &level, cam, 1.5);

            let mut views = Vec::new();
            for y in 0..rows as i32 {
                for x in 0..cols as i32 {
                    if let Some((kind, prog)) = grid_at(x, y) {
                        views.push(WallCellView {
                            cell_x: x + cx_off,
                            cell_y: y + cy_off,
                            seed: 1337,
                            theme,
                            kind,
                            mask: compute_mask(x, y, kind),
                            open_progress: prog,
                            anim_clock: 1.5,
                        });
                    }
                }
            }

            // Pass 1: Ground shadows, thresholds & directional chevrons
            for v in &views {
                draw_wall_ground(&mut pb, v, cam, accent);
            }
            // Pass 2: Wall bodies, jambs, piers & gates
            for v in &views {
                draw_wall_cell(&mut pb, v, cam, accent);
            }

            // Rats for scale:
            // 1. Ducking into horizontal rat-hole at (4, 1)
            // 2. Approaching vertical rat-hole at (6, 8)
            // 3. Running through 2-cell doorway at (3.9, 5.5)
            // 4. Weaving through thin-walled maze aisle at (13.2, 6.4)
            let rats: [(f32, f32, f32, f32); 4] = [
                (4.5, 1.85, 0.0, -1.0),
                (5.45, 8.5, 1.0, 0.0),
                (3.9, 5.5, 0.2, 1.0),
                (13.2, 6.4, 1.0, 0.0),
            ];
            for (rx, ry, fx, fy) in rats {
                let pos = cam + Vec2::new(rx * CELL_SIZE_F, ry * CELL_SIZE_F);
                let mut p = Player::new(pos, WeaponKind::Gnaw);
                p.face = Vec2::new(fx, fy).normalized();
                p.vel = p.face * 55.0;
                draw_rat(&mut pb, &p, 0.7, cam);
            }

            // Pass 3: Overhead lintels (drawn over entities!)
            for v in &views {
                draw_wall_overhang(&mut pb, v, cam, accent);
            }

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
            if ti < 2 {
                for _ in 0..(w * sep) {
                    out.extend_from_slice(&[8, 8, 10]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_walls.ppm", out).unwrap();
    }
}
