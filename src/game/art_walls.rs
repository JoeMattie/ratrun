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
//! 4. **Boss arena threshold overlay** (optional, `WallKind`-independent) —
//!    [`draw_boss_threshold_ground`] (in the Ground layer, right after
//!    [`draw_wall_ground`]) and [`draw_boss_threshold_overhang`] (in the Overhang
//!    layer, right after [`draw_wall_overhang`]). Call once per mouth cell of the
//!    boss arena's double doorway using [`BossThresholdView`]. Purely decorative:
//!    **never changes collision**.
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

use super::level::{Rectf, ShopHole, Theme};

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

// ---- Theme Material Palettes & Precomputed LUTs ----------------------------

#[inline]
const fn mix_c(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = if t < 0.0 {
        0.0
    } else if t > 1.0 {
        1.0
    } else {
        t
    };
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline]
const fn scale_c(a: Rgb, f: f32) -> Rgb {
    let f = if f < 0.0 { 0.0 } else { f };
    let r = a.0 as f32 * f;
    let g = a.1 as f32 * f;
    let b = a.2 as f32 * f;
    (
        if r > 255.0 { 255 } else { r as u8 },
        if g > 255.0 { 255 } else { g as u8 },
        if b > 255.0 { 255 } else { b as u8 },
    )
}

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

const PAL_SEWER: WallPalette = WallPalette {
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
};

const PAL_KITCHEN: WallPalette = WallPalette {
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
};

const PAL_LAUNDRY: WallPalette = WallPalette {
    outline: (12, 15, 24),
    top_base: (58, 92, 108),   // faded denim #426779, lifted for the coping
    top_alt: (48, 80, 96),
    top_hi: (150, 186, 186),
    top_lo: (28, 44, 58),
    cornice: (179, 199, 195),  // porcelain #B3C7C3
    front_base: (52, 78, 94),
    front_alt: (74, 110, 126),
    front_dark: (24, 32, 46),
    mortar: (34, 38, 59),      // indigo tile #22263B
    trim_hi: (196, 214, 208),  // glazed porcelain trim
    trim_lo: (26, 34, 48),
    detail_a: (176, 142, 72),  // brass drain fittings
    detail_b: (150, 172, 174), // chrome drum bezel
    detail_c: (216, 126, 164), // the pink of a ruined white wash
    maze_top: (70, 104, 120),
    maze_hi: (186, 206, 202),
    maze_front: (38, 48, 66),
    thresh_base: (32, 38, 56),
    thresh_hi: (170, 190, 186),
};

const PAL_PRINTWORKS: WallPalette = WallPalette {
    outline: (12, 12, 16),
    top_base: (150, 148, 136),  // newsprint #B6B4A5 in shadow
    top_alt: (136, 134, 122),
    top_hi: (196, 194, 182),
    top_lo: (92, 92, 84),
    cornice: (182, 180, 165),
    front_base: (108, 106, 96),
    front_alt: (146, 144, 132),
    front_dark: (34, 35, 42),   // charcoal #22232A
    mortar: (30, 30, 38),
    trim_hi: (206, 204, 190),
    trim_lo: (40, 40, 48),
    detail_a: (85, 64, 100),    // dried ink #554064
    detail_b: (78, 148, 152),   // registration cyan #4E9498
    detail_c: (120, 90, 132),   // wet ink highlight
    maze_top: (94, 92, 100),
    maze_hi: (182, 180, 168),
    maze_front: (48, 48, 56),
    thresh_base: (36, 36, 44),
    thresh_hi: (150, 148, 136),
};

const PAL_LAB: WallPalette = WallPalette {
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
};

#[inline]
const fn theme_palette(theme: Theme) -> &'static WallPalette {
    match theme {
        Theme::Sewer => &PAL_SEWER,
        Theme::Kitchen => &PAL_KITCHEN,
        Theme::Lab => &PAL_LAB,
        // The flatbed never reaches the wall interior art (it dispatches to
        // `art_trailer` before this), but the palette must still exist for the
        // shared helpers.
        Theme::Trailer => &PAL_KITCHEN,
        Theme::Laundry => &PAL_LAUNDRY,
        Theme::Printworks => &PAL_PRINTWORKS,
    }
}

/// Legacy helper kept only for any theme that does not yet have bespoke wall
/// carving; both wave 3 side routes (`Laundry`, `Printworks`) now implement
/// their own full wall, doorway, maze, and boss-threshold art directly.
#[inline]
const fn wall_style(theme: Theme) -> Theme {
    theme
}

// ---- Wave 3 wall materials (compile-time LUTs, zero per-pixel mix) ---------
//
// Laundry: ribbed porcelain & faded-denim washboard coping over alternating
// corrugated dado and recessed washer-bank enamel cabinets.
// Printworks: cast-lead letterpress type-block chases embossed with genuinely
// readable reversed capital letters, ink-stained reglets and cyan crop ticks
// over riveted charcoal press-bed plinths and stacked newsprint reams.

/// Precomputed compile-time color table for Laundry ribbed washboard walls.
struct LaundryLut {
    rib_hi: [Rgb; 2],
    rib_mid: [Rgb; 2],
    rib_lo: [Rgb; 2],
    seam: Rgb,
    porcelain_hi: Rgb,
    porcelain_mid: Rgb,
    porcelain_lo: Rgb,
    front_flute_hi: Rgb,
    front_flute_mid: Rgb,
    front_flute_lo: Rgb,
    front_cab_hi: Rgb,
    front_cab_mid: Rgb,
    front_cab_lo: Rgb,
    brass_hi: Rgb,
    brass_lo: Rgb,
}

const LAUNDRY_LUT: LaundryLut = {
    let p = PAL_LAUNDRY;
    let b0 = p.top_base;
    let b1 = p.top_alt;
    LaundryLut {
        rib_hi: [mix_c(b0, p.cornice, 0.52), mix_c(b1, p.cornice, 0.44)],
        rib_mid: [b0, b1],
        rib_lo: [mix_c(b0, p.top_lo, 0.56), mix_c(b1, p.top_lo, 0.58)],
        seam: p.mortar,
        porcelain_hi: mix_c(p.cornice, (230, 238, 236), 0.35),
        porcelain_mid: mix_c(p.top_base, p.cornice, 0.48),
        porcelain_lo: mix_c(p.top_alt, p.top_lo, 0.42),
        front_flute_hi: mix_c(p.front_alt, p.cornice, 0.42),
        front_flute_mid: p.front_base,
        front_flute_lo: p.front_dark,
        front_cab_hi: mix_c(p.cornice, (218, 228, 226), 0.30),
        front_cab_mid: mix_c(p.front_alt, p.cornice, 0.52),
        front_cab_lo: mix_c(p.front_base, p.front_dark, 0.55),
        brass_hi: mix_c(p.detail_a, (236, 208, 128), 0.38),
        brass_lo: scale_c(p.detail_a, 0.68),
    }
};

/// 8 genuinely readable 5×5 **reversed** capital letterpress glyphs
/// (`Ǝ`, `Я`, `И`, `ꟼ`, `ꓶ`, `Ƨ`, `ꓞ`, `ꓕ`). Bit 4 is leftmost pixel.
const REVERSED_TYPE_GLYPHS: [[u8; 5]; 8] = [
    // 0: mirrored E (Ǝ)
    [0b11111, 0b00001, 0b01111, 0b00001, 0b11111],
    // 1: mirrored R (Я)
    [0b01111, 0b10001, 0b01111, 0b00101, 0b01001],
    // 2: mirrored N (И)
    [0b10001, 0b10011, 0b10101, 0b11001, 0b10001],
    // 3: mirrored P (ꟼ)
    [0b01111, 0b10001, 0b01111, 0b00001, 0b00001],
    // 4: mirrored L (ꓶ)
    [0b00001, 0b00001, 0b00001, 0b00001, 0b11111],
    // 5: mirrored S (Ƨ)
    [0b11110, 0b00001, 0b01110, 0b10000, 0b01111],
    // 6: mirrored F (ꓞ)
    [0b11111, 0b00001, 0b01111, 0b00001, 0b00001],
    // 7: mirrored G (ꓨ)
    [0b01110, 0b00001, 0b11001, 0b10001, 0b01110],
];

/// Precomputed compile-time color table for Printworks type-block walls.
struct PrintworksLut {
    block_body: [Rgb; 3],
    block_bevel: [Rgb; 3],
    block_shade: [Rgb; 3],
    letter_hi: Rgb,
    letter_ink: Rgb,
    reglet: Rgb,
    cyan_tick: Rgb,
    brass_quoin: Rgb,
    iron_hi: Rgb,
    iron_mid: Rgb,
    iron_lo: Rgb,
    ink_stripe: Rgb,
    ream_hi: Rgb,
    ream_mid: Rgb,
    ream_lo: Rgb,
}

const PRINTWORKS_LUT: PrintworksLut = {
    let p = PAL_PRINTWORKS;
    let b0 = mix_c(p.front_dark, p.top_alt, 0.42);  // cast lead-grey sort
    let b1 = mix_c(p.front_dark, p.top_base, 0.56); // newsprint-silver sort
    let b2 = mix_c(p.front_dark, p.detail_a, 0.42); // ink-stained lead sort
    PrintworksLut {
        block_body: [b0, b1, b2],
        block_bevel: [
            mix_c(b0, p.top_hi, 0.46),
            mix_c(b1, p.top_hi, 0.52),
            mix_c(b2, p.top_hi, 0.36),
        ],
        block_shade: [
            scale_c(b0, 0.66),
            scale_c(b1, 0.68),
            scale_c(b2, 0.64),
        ],
        letter_hi: p.top_hi,
        letter_ink: mix_c(p.front_dark, p.detail_a, 0.58),
        reglet: p.mortar,
        cyan_tick: p.detail_b,
        brass_quoin: (184, 152, 82),
        iron_hi: (86, 88, 98),
        iron_mid: (52, 54, 64),
        iron_lo: p.front_dark,
        ink_stripe: mix_c(p.front_dark, p.detail_a, 0.72),
        ream_hi: p.top_hi,
        ream_mid: p.front_alt,
        ream_lo: p.top_lo,
    }
};

/// The Laundry's top face: framed 16×8 washboard porcelain & faded-denim
/// corrugated panels with varied rib direction/tone, indigo mortar seams,
/// brass rivets, and occasional recessed enamel coping plates.
#[inline]
fn laundry_top(wx: i32, wy: i32, _cell_h: u32) -> Rgb {
    let bx = wx.div_euclid(16);
    let by = wy.div_euclid(8);
    let lx = wx.rem_euclid(16);
    let ly = wy.rem_euclid(8);
    let n = hash3(bx, by, 0x31a7);
    let ti = (n & 1) as usize;

    // Crisp recessed indigo panel joints.
    if ly == 0 || lx == 0 {
        return LAUNDRY_LUT.seam;
    }
    // Top porcelain highlight rail on each panel.
    if ly == 1 {
        return if lx == 1 || lx == 15 {
            LAUNDRY_LUT.porcelain_hi
        } else {
            LAUNDRY_LUT.porcelain_mid
        };
    }
    // Bottom shadowed channel lip.
    if ly == 7 {
        return LAUNDRY_LUT.rib_lo[ti];
    }
    // Occasional smooth porcelain coping hatch (breaks up long washboard runs).
    if (n & 7) == 5 && (3..=12).contains(&lx) && (2..=5).contains(&ly) {
        return if lx == 3 || ly == 2 {
            LAUNDRY_LUT.porcelain_hi
        } else if lx == 12 || ly == 5 {
            LAUNDRY_LUT.porcelain_lo
        } else if lx == 8 && ly == 3 {
            LAUNDRY_LUT.brass_hi
        } else {
            LAUNDRY_LUT.porcelain_mid
        };
    }
    // Brass corner fasteners on alternating washboard bays.
    if (lx == 2 || lx == 13) && ly == 2 && (n & 3) == 0 {
        return LAUNDRY_LUT.brass_hi;
    }
    // Corrugated washboard ribs (3 px repeat: bright crest, denim body, shadow trough).
    let horiz = (n & 4) != 0;
    let phase = if horiz {
        (ly - 2).rem_euclid(3)
    } else {
        (lx + ((n >> 4) & 3) as i32).rem_euclid(3)
    };
    match phase {
        0 => LAUNDRY_LUT.rib_hi[ti],
        2 => LAUNDRY_LUT.rib_lo[ti],
        _ => LAUNDRY_LUT.rib_mid[ti],
    }
}

/// The Laundry's south face: alternates corrugated washboard porcelain dado
/// with recessed enamel washer-bank cabinet panels (matching the long washer
/// banks in Laundry halls) above a brass kick strip and dark indigo skirting.
#[inline]
fn laundry_front(wx: i32, fy: i32, bay_h: u32, has_w: bool, has_e: bool) -> Rgb {
    let lx = wx.rem_euclid(16);
    let is_washer_bay = (bay_h & 1) == 0 && (has_w || has_e);
    if is_washer_bay {
        match fy {
            0 => {
                if lx == 0 || lx == 15 {
                    LAUNDRY_LUT.seam
                } else {
                    LAUNDRY_LUT.front_cab_hi
                }
            }
            4 => LAUNDRY_LUT.front_flute_lo,
            _ => {
                if lx == 0 || lx == 15 {
                    LAUNDRY_LUT.front_cab_lo
                } else if fy == 3 {
                    LAUNDRY_LUT.brass_lo
                } else {
                    LAUNDRY_LUT.front_cab_mid
                }
            }
        }
    } else {
        let flute = wx.rem_euclid(4);
        match fy {
            0 => LAUNDRY_LUT.front_flute_hi,
            3 => {
                if flute == 0 {
                    LAUNDRY_LUT.brass_lo
                } else {
                    LAUNDRY_LUT.brass_hi
                }
            }
            4 => LAUNDRY_LUT.front_flute_lo,
            _ => match flute {
                0 => LAUNDRY_LUT.front_flute_lo,
                1 => LAUNDRY_LUT.front_flute_hi,
                _ => LAUNDRY_LUT.front_flute_mid,
            },
        }
    }
}

/// The Printworks' top face: staggered 10 × 8 cast-lead letterpress type
/// blocks with genuinely readable reversed capital letters (`Ǝ Я И ꟼ ꓶ Ƨ ꓞ ꓨ`),
/// charcoal reglet seams, brass lockup quoins, and registration-cyan crop ticks.
#[inline]
fn printworks_top(wx: i32, wy: i32, _cell_h: u32) -> Rgb {
    let by = wy.div_euclid(8);
    let ly = wy.rem_euclid(8);
    let stagger = (by & 1) * 5;
    let sx = wx + stagger;
    let bx = sx.div_euclid(10);
    let lx = sx.rem_euclid(10);
    let n = hash3(bx, by, 0x5511);
    let bi = (n % 3) as usize;

    // Charcoal reglet / furniture seams separating each type sort.
    if lx == 0 || ly == 0 {
        return PRINTWORKS_LUT.reglet;
    }
    // Top bevel highlight & lower lip shadow.
    if ly == 1 {
        return if lx == 1 && (n & 7) == 3 {
            PRINTWORKS_LUT.brass_quoin
        } else {
            PRINTWORKS_LUT.block_bevel[bi]
        };
    }
    if ly == 7 || lx == 9 {
        // Registration cyan corner tick on every 4th sort.
        if ly == 7 && lx == 8 && (n & 3) == 1 {
            return PRINTWORKS_LUT.cyan_tick;
        }
        return PRINTWORKS_LUT.block_shade[bi];
    }

    // Embossed 5×5 reversed capital letter standing proud on ~60% of sorts!
    if (n & 7) < 5 && (2..=6).contains(&lx) && (2..=6).contains(&ly) {
        let glyph = REVERSED_TYPE_GLYPHS[((n >> 4) & 7) as usize];
        let gx = (lx - 2) as usize;
        let gy = (ly - 2) as usize;
        let on = ((glyph[gy] >> (4 - gx)) & 1) != 0;
        if on {
            // Top-lit raised lead face vs ink-stained lower edge.
            let above_on = gy > 0 && ((glyph[gy - 1] >> (4 - gx)) & 1) != 0;
            return if !above_on {
                PRINTWORKS_LUT.letter_hi
            } else {
                PRINTWORKS_LUT.letter_ink
            };
        }
        // Subtle 1 px cast shadow directly below a raised letter stroke.
        if gy > 0 && ((glyph[gy - 1] >> (4 - gx)) & 1) != 0 {
            return PRINTWORKS_LUT.block_shade[bi];
        }
    }

    PRINTWORKS_LUT.block_body[bi]
}

/// The Printworks' south face: alternates riveted charcoal press-bed iron
/// plinths (with a dried-ink purple stripe) and stacked newsprint paper reams.
#[inline]
fn printworks_front(wx: i32, fy: i32, bay_h: u32, has_w: bool, has_e: bool) -> Rgb {
    let lx = wx.rem_euclid(16);
    let is_ream = (bay_h & 1) == 1 && (has_w || has_e);
    if is_ream {
        // Compressed stack of newsprint sheets: crisp pale paper strata.
        if lx == 0 || lx == 15 {
            PRINTWORKS_LUT.iron_lo
        } else {
            match fy {
                0 => PRINTWORKS_LUT.ream_hi,
                1 => PRINTWORKS_LUT.ream_lo,
                2 => PRINTWORKS_LUT.ream_mid,
                3 => PRINTWORKS_LUT.ream_hi,
                _ => PRINTWORKS_LUT.iron_lo,
            }
        }
    } else {
        // Riveted charcoal cast-iron press plinth with dried-ink plum stripe.
        let seam = lx == 0 || lx == 15;
        match fy {
            0 => if seam { PRINTWORKS_LUT.iron_lo } else { PRINTWORKS_LUT.iron_hi },
            2 => if seam { PRINTWORKS_LUT.iron_lo } else { PRINTWORKS_LUT.ink_stripe },
            3 => {
                if lx == 4 || lx == 11 {
                    PRINTWORKS_LUT.brass_quoin
                } else {
                    PRINTWORKS_LUT.iron_mid
                }
            }
            4 => PRINTWORKS_LUT.iron_lo,
            _ => if seam { PRINTWORKS_LUT.iron_lo } else { PRINTWORKS_LUT.iron_mid },
        }
    }
}

/// Fill a solid cell's whole 16 x 16 area with the theme's top material (the
/// wave 3 themes' "fully surrounded wall" case).
fn draw_wave3_solid_interior(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    view: &WallCellView,
    laundry: bool,
) {
    let (wx0, wy0) = view.world_origin();
    let cell_h = hash3(view.cell_x, view.cell_y, view.seed);
    let bh = pb.h as i32;
    let bw = pb.w as i32;
    for ly in 0..CELL_SIZE {
        let py = sy + ly;
        if py < 0 || py >= bh {
            continue;
        }
        for lx in 0..CELL_SIZE {
            let px = sx + lx;
            if px < 0 || px >= bw {
                continue;
            }
            let c = if laundry {
                laundry_top(wx0 + lx, wy0 + ly, cell_h)
            } else {
                printworks_top(wx0 + lx, wy0 + ly, cell_h)
            };
            pb.plot(px, py, c);
        }
    }
}

/// Precomputed compile-time color table for Sewer top coping stones.
struct SewerTopLut {
    base: [Rgb; 3],
    mortar: [Rgb; 3],
    hi: [Rgb; 3],
    crack: [Rgb; 3],
    moss_base: Rgb,
    moss_mortar: Rgb,
    moss_hi: Rgb,
}

const SEWER_TOP_LUT: SewerTopLut = {
    let p = PAL_SEWER;
    let b0 = p.top_alt;
    let b1 = scale_c(p.top_base, 1.08);
    let b2 = p.top_base;
    let m0 = mix_c(b0, p.mortar, 0.64);
    let m1 = mix_c(b1, p.mortar, 0.64);
    let m2 = mix_c(b2, p.mortar, 0.64);
    let h0 = mix_c(b0, p.top_hi, 0.16);
    let h1 = mix_c(b1, p.top_hi, 0.16);
    let h2 = mix_c(b2, p.top_hi, 0.16);
    SewerTopLut {
        base: [b0, b1, b2],
        mortar: [m0, m1, m2],
        hi: [h0, h1, h2],
        crack: [scale_c(b0, 0.66), scale_c(b1, 0.66), scale_c(b2, 0.66)],
        moss_base: mix_c(b2, p.detail_a, 0.52),
        moss_mortar: mix_c(m2, p.detail_a, 0.52),
        moss_hi: mix_c(h2, p.detail_a, 0.52),
    }
};

/// Precomputed compile-time color table for Sewer south brick front face.
struct SewerFrontLut {
    brick: [Rgb; 2],
    brick_hi: [Rgb; 2],
    brick_bot: [Rgb; 2],
    moss_hi: Rgb,
    moss_lo: Rgb,
}

const SEWER_FRONT_LUT: SewerFrontLut = {
    let p = PAL_SEWER;
    SewerFrontLut {
        brick: [p.front_base, p.front_alt],
        brick_hi: [
            mix_c(p.front_base, p.top_hi, 0.16),
            mix_c(p.front_alt, p.top_hi, 0.16),
        ],
        brick_bot: [
            mix_c(p.front_base, p.front_dark, 0.68),
            mix_c(p.front_alt, p.front_dark, 0.68),
        ],
        moss_hi: scale_c(p.detail_a, 1.15),
        moss_lo: scale_c(p.detail_a, 0.88),
    }
};

/// Precomputed compile-time color table for Kitchen top oak planks.
/// Indexed by `row_kind` (0 = plain, 1 = py_in==0 mortar, 2 = py_in==1 hi) and `bi` (0..2).
struct KitchenTopLut {
    row: [[Rgb; 2]; 3],
    row_grain: [[Rgb; 2]; 3],
    seam: [[Rgb; 2]; 3],
    seam_grain: [[Rgb; 2]; 3],
}

const KITCHEN_TOP_LUT: KitchenTopLut = {
    let p = PAL_KITCHEN;
    let b = [p.top_base, p.top_alt];
    let r0 = [mix_c(b[0], p.mortar, 0.32), mix_c(b[1], p.mortar, 0.32)];
    let r1 = [mix_c(b[0], p.top_hi, 0.14), mix_c(b[1], p.top_hi, 0.14)];
    let s_plain = [mix_c(b[0], p.mortar, 0.48), mix_c(b[1], p.mortar, 0.48)];
    let s_r0 = [mix_c(r0[0], p.mortar, 0.48), mix_c(r0[1], p.mortar, 0.48)];
    let s_r1 = [mix_c(r1[0], p.mortar, 0.48), mix_c(r1[1], p.mortar, 0.48)];
    KitchenTopLut {
        row: [b, r0, r1],
        row_grain: [
            [scale_c(b[0], 0.93), scale_c(b[1], 0.93)],
            [scale_c(r0[0], 0.93), scale_c(r0[1], 0.93)],
            [scale_c(r1[0], 0.93), scale_c(r1[1], 0.93)],
        ],
        seam: [s_plain, s_r0, s_r1],
        seam_grain: [
            [scale_c(s_plain[0], 0.93), scale_c(s_plain[1], 0.93)],
            [scale_c(s_r0[0], 0.93), scale_c(s_r0[1], 0.93)],
            [scale_c(s_r1[0], 0.93), scale_c(s_r1[1], 0.93)],
        ],
    }
};

/// Precomputed compile-time color table for Kitchen cabinets and subway tile dado.
struct KitchenFrontLut {
    cab_div: Rgb,
    cab_seam: Rgb,
    cab_hi: Rgb,
    cab_panel_sh: Rgb,
    skirt_lip: Rgb,
    tile_grout: Rgb,
    tile: [Rgb; 2],
    tile_bot: [Rgb; 2],
    tile_hi: [Rgb; 2],
}

const KITCHEN_FRONT_LUT: KitchenFrontLut = {
    let p = PAL_KITCHEN;
    let t0 = p.front_alt;
    let t1 = scale_c(p.front_alt, 0.91);
    KitchenFrontLut {
        cab_div: scale_c(p.front_base, 0.56),
        cab_seam: scale_c(p.front_base, 0.66),
        cab_hi: scale_c(p.front_base, 1.12),
        cab_panel_sh: scale_c(p.front_base, 0.72),
        skirt_lip: scale_c(p.front_base, 0.88),
        tile_grout: mix_c(p.front_alt, p.mortar, 0.56),
        tile: [t0, t1],
        tile_bot: [scale_c(t0, 0.86), scale_c(t1, 0.86)],
        tile_hi: [mix_c(t0, p.detail_b, 0.48), mix_c(t1, p.detail_b, 0.48)],
    }
};

/// Precomputed compile-time color table for Lab interlocking hull armor plates.
struct LabTopLut {
    base: [Rgb; 3],
    seam: [Rgb; 3],
    hi: [Rgb; 3],
    bolt: [Rgb; 3],
    conduit: Rgb,
}

const LAB_TOP_LUT: LabTopLut = {
    let p = PAL_LAB;
    let b0 = p.top_alt;
    let b1 = scale_c(p.top_base, 1.07);
    let b2 = p.top_base;
    LabTopLut {
        base: [b0, b1, b2],
        seam: [
            mix_c(b0, p.mortar, 0.72),
            mix_c(b1, p.mortar, 0.72),
            mix_c(b2, p.mortar, 0.72),
        ],
        hi: [
            mix_c(b0, p.top_hi, 0.20),
            mix_c(b1, p.top_hi, 0.20),
            mix_c(b2, p.top_hi, 0.20),
        ],
        bolt: [
            mix_c(b0, p.trim_hi, 0.50),
            mix_c(b1, p.trim_hi, 0.50),
            mix_c(b2, p.trim_hi, 0.50),
        ],
        conduit: mix_c(b2, p.mortar, 0.62),
    }
};

/// Precomputed compile-time color table for Lab steel bulkhead front face.
struct LabFrontLut {
    hazard_hi: Rgb,
    hazard_lo: Rgb,
    rib_lo: Rgb,
    rib_hi: Rgb,
    mid_band: Rgb,
}

const LAB_FRONT_LUT: LabFrontLut = {
    let p = PAL_LAB;
    LabFrontLut {
        hazard_hi: p.detail_a,
        hazard_lo: scale_c(p.detail_a, 0.82),
        rib_lo: scale_c(p.front_base, 0.64),
        rib_hi: mix_c(p.front_alt, p.trim_hi, 0.28),
        mid_band: scale_c(p.front_base, 0.82),
    }
};

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
    pb.plot(x, y, scale_c(cur, factor));
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
    pb.plot(x, y, mix_c(cur, c, alpha));
}

// ============================================================================
// LAYER 1: GROUND (SHADOWS, THRESHOLDS, CHEVRONS)
// ============================================================================

/// **Ground layer**: draw contact shadows, flush doorway/one-way threshold
/// plates, directional chevrons, and rat-hole floor recesses.
///
/// Call right after `art_floor::draw_floor` and floor stains, **before**
/// entities, traps, pickups, and [`draw_wall_cell`]. Zero heap allocation.
/// **The mouse-hole shop's marker**: the opening, the bent spoon over it and
/// the three white whisker scratches on the floor in front.
///
/// **The mouse-hole shop's marker**: the arched den opening with warm
/// subterranean glow, the bent pewter spoon nailed over the keystone, and
/// three chalk-white whisker scratches scored into the floor at the fork.
///
/// Drawn on the foreground object layer over the wall body; integrates into
/// every theme's wall masonry via blended mortar/gnaw rims and grooved floor
/// shadows so it reads clearly on dark Sewer/Lab stone and sunlit Flatbed deck.
pub fn draw_shop_hole(pb: &mut PixelBuffer, hole: &ShopHole, cam: Vec2, t: f32) {
    let cx = hole.pos.x - cam.x;
    let cy = hole.pos.y - cam.y;
    if !cx.is_finite() || !cy.is_finite() || cx < -40.0 || cy < -40.0 {
        return;
    }
    if cx > pb.w as f32 + 40.0 || cy > pb.h as f32 + 40.0 {
        return;
    }
    let (x, y) = (cx.round() as i32, cy.round() as i32);
    let (fx, fy) = (hole.facing.x.round() as i32, hole.facing.y.round() as i32);
    // Anchor the arch on the wall face facing the walkable fork.
    let ox = x + fx * 5;
    let oy = y + fy * 5 - 1;

    const VOID: Rgb = (8, 7, 11);
    const DEEP_WARM: Rgb = (58, 34, 18);
    const CANDLE: Rgb = (196, 126, 56);
    const CANDLE_HI: Rgb = (246, 198, 122);
    const GNAW_DARK: Rgb = (26, 22, 26);
    const GNAW_CHIP: Rgb = (118, 108, 100);
    const SPOON_LO: Rgb = (88, 94, 108);
    const SPOON_MID: Rgb = (168, 174, 188);
    const SPOON_HI: Rgb = (244, 248, 255);
    const NAIL: Rgb = (52, 46, 44);
    const SCRATCH_HI: Rgb = (242, 246, 248);
    const SCRATCH_MID: Rgb = (188, 196, 204);

    let candle_pulse = 0.5 + 0.5 * (t * 2.9 + hole.pos.x * 0.07).sin();
    let glint = 0.5 + 0.5 * (t * 1.65 + hole.pos.y * 0.05).sin();

    // 1) Soft warm subterranean light spill onto the threshold floor in front.
    let spill_x = ox + fx * 3;
    let spill_y = oy + fy * 3 + 2;
    let spill_col = palette::mix(DEEP_WARM, CANDLE, 0.35 + 0.25 * candle_pulse);
    for dy in -2i32..=3 {
        for dx in -4i32..=4 {
            let d2 = dx * dx + dy * dy * 2;
            if d2 <= 18 {
                let a = (1.0 - d2 as f32 / 20.0) * (0.20 + 0.08 * candle_pulse);
                blend_px(pb, spill_x + dx, spill_y + dy, spill_col, a);
            }
        }
    }

    // 2) The arched mouse-hole opening (domed top, flat sill bottom, gnawed masonry collar).
    //    Half-widths per row from top (dy = -4) to sill (dy = +3).
    const ARCH: [(i32, i32); 8] = [
        (-4, 1),
        (-3, 3),
        (-2, 4),
        (-1, 4),
        (0, 4),
        (1, 4),
        (2, 4),
        (3, 4),
    ];
    // Outer ambient shadow / gnawed masonry ring.
    for &(dy, half) in &ARCH {
        darken_px(pb, ox - half - 2, oy + dy, 0.62);
        darken_px(pb, ox + half + 2, oy + dy, 0.62);
        blend_px(pb, ox - half - 1, oy + dy, GNAW_DARK, 0.88);
        blend_px(pb, ox + half + 1, oy + dy, GNAW_DARK, 0.88);
        // Scattered gnawed tooth-marks along the outer lip.
        if dy == -2 || dy == 0 || dy == 2 {
            blend_px(pb, ox - half - 1, oy + dy, GNAW_CHIP, 0.42);
            blend_px(pb, ox + half + 1, oy + dy, GNAW_CHIP, 0.32);
        }
    }
    for dx in -2i32..=2 {
        blend_px(pb, ox + dx, oy - 5, GNAW_DARK, 0.85);
        darken_px(pb, ox + dx, oy - 6, 0.65);
    }
    // Flat threshold sill at dy = +4.
    for dx in -4i32..=4 {
        blend_px(pb, ox + dx, oy + 4, GNAW_CHIP, 0.55);
        darken_px(pb, ox + dx, oy + 5, 0.55);
    }

    // Interior tunnel fill: pitch black at the crown, warm den candle glow at the lower recess.
    for &(dy, half) in &ARCH {
        for dx in -half..=half {
            let px = ox + dx;
            let py = oy + dy;
            let edge = dx.abs() == half || dy == -4;
            let col = if dy >= 2 && dx.abs() <= 2 {
                // Warm candlelit den floor peeking from behind the wall.
                let k = (1.0 - dx.abs() as f32 * 0.30) * (if dy == 3 { 1.0 } else { 0.55 });
                let glow = palette::mix(DEEP_WARM, CANDLE, 0.45 + 0.35 * candle_pulse);
                palette::mix(VOID, glow, k * 0.85)
            } else if dy == 1 && dx.abs() <= 1 {
                palette::mix(VOID, DEEP_WARM, 0.55 + 0.25 * candle_pulse)
            } else if edge {
                (16, 13, 18)
            } else {
                VOID
            };
            pb.plot(px, py, col);
        }
    }
    // Tiny warm lantern ember deep at the sill centre so the hole feels inhabited.
    let ember = palette::mix(CANDLE, CANDLE_HI, 0.35 + 0.50 * candle_pulse);
    blend_px(pb, ox, oy + 3, ember, 0.82);

    // 3) The bent pewter spoon hanging over the arch: flared handle, iron nail,
    //    kinked elbow bent over the wall lip, and a concave gleaming pewter bowl.
    //    Wrapped in a tight dark contour halo so it reads crisply on pale
    //    porcelain Laundry ribs and bright newsprint Printworks blocks as well
    //    as dark Sewer/Lab stone.
    //    When the hole faces North (fy < 0), keep the spoon handle tucked onto
    //    the wall masonry to the right so it never crosses the north floor scratches.
    let glint_col = palette::mix(SPOON_MID, SPOON_HI, 0.35 + 0.65 * glint);
    if fy < 0 {
        // North-facing wall: bowl hangs over the arch, kinked handle angles down-right onto masonry.
        let by = oy - 1;
        // Dark contour halo around bowl and angled handle so pale newsprint /
        // porcelain walls never swallow the silver spoon.
        for dy in -2i32..=3 {
            for dx in -3i32..=3 {
                if dx * dx + (dy - 1) * (dy - 1) <= 7 {
                    blend_px(pb, ox + dx, by + dy, GNAW_DARK, 0.72);
                }
            }
        }
        for &(hx, hy) in &[
            (2, 0), (3, 0), (4, 1), (5, 1), (6, 2), (7, 3), (7, 4),
            (6, 5), (5, 4), (4, 3), (3, 2),
        ] {
            blend_px(pb, ox + hx, by + hy, GNAW_DARK, 0.78);
        }
        // Handle pinned into the wall masonry to the right of the arch.
        pb.plot(ox + 2, by + 1, SPOON_HI);
        pb.plot(ox + 3, by + 1, SPOON_LO);
        pb.plot(ox + 4, by + 2, SPOON_MID);
        pb.plot(ox + 5, by + 2, NAIL);
        pb.plot(ox + 5, by + 3, SPOON_MID);
        pb.plot(ox + 6, by + 3, SPOON_HI);
        pb.plot(ox + 6, by + 4, SPOON_LO);
        for dy in -1i32..=2 {
            let hw = if dy == -1 || dy == 2 { 1 } else { 2 };
            for dx in -hw..=hw {
                let col = if dy == -1 || dx == -hw {
                    SPOON_HI
                } else if dy == 2 || dx == hw {
                    SPOON_LO
                } else if dx == 0 && dy == 0 {
                    glint_col
                } else {
                    SPOON_MID
                };
                pb.plot(ox + dx, by + dy, col);
            }
        }
        if glint > 0.65 {
            pb.plot(ox - 1, by, SPOON_HI);
        }
    } else {
        let top = oy - 9;
        // Full dark contour outline behind handle, elbow, and bowl so the
        // bright pewter silhouette separates cleanly from porcelain & newsprint walls.
        for &(hx, hy) in &[
            (2, -1), (3, -1), (4, -1), (5, 0), (5, 1), (5, 2), (4, 3), (4, 4),
            (2, 0), (2, 1), (2, 2), (2, 3), (1, 4), (0, 5), (3, 5), (2, 6),
        ] {
            blend_px(pb, ox + hx, top + hy, GNAW_DARK, 0.78);
        }
        for dy in -1i32..=4 {
            for dx in -3i32..=3 {
                if dx * dx + (dy - 1) * (dy - 1) <= 7 {
                    blend_px(pb, ox + dx, top + 6 + dy, GNAW_DARK, 0.68);
                }
            }
        }
        // Flared pewter handle + iron nail pin near top.
        pb.plot(ox + 3, top, SPOON_HI);
        pb.plot(ox + 4, top, SPOON_MID);
        pb.plot(ox + 3, top + 1, SPOON_MID);
        pb.plot(ox + 4, top + 1, SPOON_LO);
        pb.plot(ox + 3, top + 2, NAIL); // iron nail holding the spoon
        pb.plot(ox + 3, top + 3, SPOON_MID);
        pb.plot(ox + 3, top + 4, SPOON_HI);
        // Kinked elbow bending left over the keystone of the arch.
        pb.plot(ox + 2, top + 4, SPOON_MID);
        pb.plot(ox + 2, top + 5, SPOON_LO);
        pb.plot(ox + 1, top + 5, SPOON_MID);
        pb.plot(ox + 1, top + 6, SPOON_HI);
        // Concave oval spoon bowl (5 x 4 px) hanging just over the dark arch crown.
        let by = top + 7;
        for dy in -1i32..=2 {
            let hw = if dy == -1 || dy == 2 { 1 } else { 2 };
            for dx in -hw..=hw {
                let col = if dy == -1 || dx == -hw {
                    SPOON_HI // top-left metallic rim
                } else if dy == 2 || dx == hw {
                    SPOON_LO // shaded lower-right rim
                } else if dx == 0 && dy == 0 {
                    glint_col // travelling specular glint inside the bowl
                } else {
                    SPOON_MID
                };
                pb.plot(ox + dx, by + dy, col);
            }
        }
        // Extra specular sparkle pixel at peak glint.
        if glint > 0.65 {
            pb.plot(ox - 1, by, SPOON_HI);
        }
    }

    // 4) Three chalk-white whisker scratches scored into the floor at the fork,
    //    pointing cleanly toward the opening, each with a dark shadow groove so
    //    they read on both dark Sewer/Lab masonry and sunlit Flatbed planks.
    let (sx, sy) = (ox + fx * 5, oy + fy * 6 + if fy == 0 { 2 } else { 1 });
    let side = (fy, -fx);
    for (i, &spread) in [-3i32, 0, 3].iter().enumerate() {
        let len = if i == 1 { 6 } else { 5 };
        // Fan slightly outward away from the mouth like three rat whiskers.
        let fan = spread / 3;
        let x0 = sx + side.0 * spread;
        let y0 = sy + side.1 * spread;
        for step in 0..len {
            let px = x0 + fx * step + side.0 * ((step * fan) / 3);
            let py = y0 + fy * step + side.1 * ((step * fan) / 3);
            // Recessed dark groove right beside the chalk scratch.
            darken_px(pb, px + if fx == 0 { 1 } else { 0 }, py + 1, 0.52);
            let col = if step == 0 || step == len - 1 {
                SCRATCH_MID
            } else {
                SCRATCH_HI
            };
            let alpha = if step == len - 1 { 0.72 } else { 0.96 };
            blend_px(pb, px, py, col, alpha);
        }
    }
}

pub fn draw_wall_ground(pb: &mut PixelBuffer, view: &WallCellView, cam: Vec2, accent: Rgb) {
    // The getaway ride's own layers (cargo, rails, the road over the side).
    if view.theme == Theme::Trailer {
        return crate::game::art_trailer::draw_wall_ground(pb, view, cam, accent);
    }
    if matches!(view.kind, WallKind::Solid)
        && view.mask.has(NeighborMask::S)
        && view.mask.has(NeighborMask::E)
    {
        return;
    }
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
            draw_doorway_ground(pb, sx, sy, view.mask, axis, view.theme, pal, accent);
        }
        WallKind::OneWayDoor { dir } => {
            draw_doorway_ground(
                pb,
                sx,
                sy,
                view.mask,
                dir.wall_axis(),
                view.theme,
                pal,
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
                        match wall_style(theme) {
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
                            Theme::Laundry => {
                                // Brass junction drain grate set inside porcelain threshold
                                if ly == 9 || ly == 10 {
                                    if (lx % 3) == 0 {
                                        pal.outline
                                    } else {
                                        pal.detail_a
                                    }
                                } else {
                                    pal.thresh_base
                                }
                            }
                            Theme::Printworks => {
                                // Cast-iron press-track sill with cyan registration ticks at jambs
                                if (lx == x0 + 1 || lx == x1 - 2) && (ly == 9 || ly == 10) {
                                    pal.detail_b
                                } else if ly == 9 || ly == 10 {
                                    if (lx % 4) == 0 {
                                        pal.thresh_hi
                                    } else {
                                        pal.front_dark
                                    }
                                } else {
                                    pal.thresh_base
                                }
                            }
                            _ => pal.thresh_base,
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
                        match wall_style(theme) {
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
                            Theme::Laundry => {
                                // Vertical brass junction drain grate inside porcelain threshold
                                if lx == 8 || lx == 9 {
                                    if (ly % 3) == 0 {
                                        pal.outline
                                    } else {
                                        pal.detail_a
                                    }
                                } else {
                                    pal.thresh_base
                                }
                            }
                            Theme::Printworks => {
                                // Vertical press-track sill with cyan registration ticks at jambs
                                if (ly == y0 + 1 || ly == y1 - 2) && (lx == 8 || lx == 9) {
                                    pal.detail_b
                                } else if lx == 8 || lx == 9 {
                                    if (ly % 4) == 0 {
                                        pal.thresh_hi
                                    } else {
                                        pal.front_dark
                                    }
                                } else {
                                    pal.thresh_base
                                }
                            }
                            _ => pal.thresh_base,
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
    // The getaway ride's own layers (cargo, rails, the road over the side).
    if view.theme == Theme::Trailer {
        return crate::game::art_trailer::draw_wall_cell(pb, view, cam, accent);
    }
    let (sx, sy) = cell_screen_origin(view, cam);
    if culled(pb, sx, sy) {
        return;
    }
    let pal = theme_palette(view.theme);

    match view.kind {
        WallKind::Solid => {
            draw_solid_cell(pb, sx, sy, view, pal, accent, 0..CELL_SIZE, 0..CELL_SIZE)
        }
        WallKind::Maze => draw_maze_cell(pb, sx, sy, view, pal, accent),
        WallKind::RatHole { axis } => draw_rathole_cell(pb, sx, sy, view, pal, accent, axis),
        WallKind::Doorway { axis } => draw_doorway_jambs(pb, sx, sy, view, pal, accent, axis),
        WallKind::OneWayDoor { dir } => {
            draw_doorway_jambs(pb, sx, sy, view, pal, accent, dir.wall_axis());
            draw_oneway_gate(pb, sx, sy, view, pal, accent, dir);
        }
    }
}

/// Fast span-based Lab solid interior renderer (`NeighborMask::ALL`, full 16×16 cell).
/// Computes at most 2 divisions and 2–4 block hashes per cell instead of 256 per pixel,
/// using compile-time [`LAB_TOP_LUT`] colors and `PixelBuffer::rect_fill`.
#[inline]
fn draw_lab_solid_interior(pb: &mut PixelBuffer, sx: i32, sy: i32, view: &WallCellView) {
    let (wx0, _) = view.world_origin();
    let bh = pb.h as i32;
    for band in 0..2i32 {
        let y_base = sy + band * 8;
        if y_base + 8 <= 0 || y_base >= bh {
            continue;
        }
        let row = view.cell_y * 2 + band;
        let stagger = band * 10;
        let x_start = wx0 + stagger;
        let col0 = x_start.div_euclid(20);
        let bx0 = x_start.rem_euclid(20);
        let seg0_w = (20 - bx0).min(CELL_SIZE);

        draw_lab_plate_segment(pb, sx, y_base, seg0_w, bx0, hash3(col0, row, 0x33F5));
        if seg0_w < CELL_SIZE {
            draw_lab_plate_segment(
                pb,
                sx + seg0_w,
                y_base,
                CELL_SIZE - seg0_w,
                0,
                hash3(col0 + 1, row, 0x33F5),
            );
        }
    }
}

#[inline]
fn draw_lab_plate_segment(
    pb: &mut PixelBuffer,
    px0: i32,
    py0: i32,
    seg_w: i32,
    bx_start: i32,
    ph: u32,
) {
    let bi = if (ph & 3) == 0 {
        0
    } else if (ph & 7) == 2 {
        1
    } else {
        2
    };
    let base = LAB_TOP_LUT.base[bi];
    let seam = LAB_TOP_LUT.seam[bi];
    let hi = LAB_TOP_LUT.hi[bi];
    let bolt = LAB_TOP_LUT.bolt[bi];
    let bx_end = bx_start + seg_w;

    // by == 0: horizontal plate seam
    pb.rect_fill(px0, py0, seg_w, 1, seam);
    // by == 1: top bevel highlight (`bx in 2..19`), base at `bx == 1, 19`, seam at `bx == 0`
    pb.rect_fill(px0, py0 + 1, seg_w, 1, hi);
    if bx_start == 0 {
        pb.plot(px0, py0 + 1, seam);
    }
    if bx_start <= 1 && bx_end > 1 {
        pb.plot(px0 + (1 - bx_start), py0 + 1, base);
    }
    if bx_start <= 19 && bx_end > 19 {
        pb.plot(px0 + (19 - bx_start), py0 + 1, base);
    }
    // by in 2..8: plate body + left vertical seam + corner hex bolts + optional conduit
    pb.rect_fill(px0, py0 + 2, seg_w, 6, base);
    if bx_start == 0 {
        pb.rect_fill(px0, py0 + 2, 1, 6, seam);
    }
    if bx_start <= 2 && bx_end > 2 {
        let bx_x = px0 + (2 - bx_start);
        pb.plot(bx_x, py0 + 2, bolt);
        pb.plot(bx_x, py0 + 6, bolt);
    }
    if bx_start <= 17 && bx_end > 17 {
        let bx_x = px0 + (17 - bx_start);
        pb.plot(bx_x, py0 + 2, bolt);
        pb.plot(bx_x, py0 + 6, bolt);
    }
    if (ph & 7) == 5 {
        let c0 = 4.max(bx_start);
        let c1 = 16.min(bx_end);
        if c0 < c1 {
            pb.rect_fill(px0 + (c0 - bx_start), py0 + 4, c1 - c0, 1, LAB_TOP_LUT.conduit);
        }
    }
}

/// Fast span-based Sewer solid interior renderer (`NeighborMask::ALL`, full 16×16 cell).
#[inline]
fn draw_sewer_solid_interior(pb: &mut PixelBuffer, sx: i32, sy: i32, view: &WallCellView, cell_h: u32) {
    let (_, wy0) = view.world_origin();
    let allow_moss = (cell_h & 7) < 3;
    let has_crack = (cell_h & 15) == 4;
    let bh = pb.h as i32;

    let mut ly = 0;
    let mut row = wy0.div_euclid(6);
    let mut by0 = wy0.rem_euclid(6);
    while ly < CELL_SIZE {
        let band_h = (6 - by0).min(CELL_SIZE - ly);
        let py_band = sy + ly;
        if py_band + band_h > 0 && py_band < bh {
            let base_col = view.cell_x * 2;
            if (row & 1) == 0 {
                draw_sewer_stone_segment(
                    pb, sx, py_band, 8, band_h, 0, by0, base_col, row, allow_moss, has_crack,
                );
                draw_sewer_stone_segment(
                    pb, sx + 8, py_band, 8, band_h, 0, by0, base_col + 1, row, allow_moss, has_crack,
                );
            } else {
                draw_sewer_stone_segment(
                    pb, sx, py_band, 4, band_h, 4, by0, base_col, row, allow_moss, has_crack,
                );
                draw_sewer_stone_segment(
                    pb, sx + 4, py_band, 8, band_h, 0, by0, base_col + 1, row, allow_moss, has_crack,
                );
                draw_sewer_stone_segment(
                    pb, sx + 12, py_band, 4, band_h, 0, by0, base_col + 2, row, allow_moss, has_crack,
                );
            }
        }
        ly += band_h;
        row += 1;
        by0 = 0;
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn draw_sewer_stone_segment(
    pb: &mut PixelBuffer,
    px0: i32,
    py0: i32,
    seg_w: i32,
    band_h: i32,
    bx_start: i32,
    by0: i32,
    col: i32,
    row: i32,
    allow_moss: bool,
    has_crack: bool,
) {
    let sh = hash3(col, row, 0x1147);
    let bi = if (sh & 3) == 0 {
        0
    } else if (sh & 7) == 1 {
        1
    } else {
        2
    };
    let moss = allow_moss && (sh & 7) == 5;
    let by_end = by0 + band_h;

    pb.rect_fill(px0, py0, seg_w, band_h, SEWER_TOP_LUT.base[bi]);
    if bx_start == 0 {
        pb.rect_fill(px0, py0, 1, band_h, SEWER_TOP_LUT.mortar[bi]);
    }
    if by0 == 0 {
        let c = if moss {
            SEWER_TOP_LUT.moss_mortar
        } else {
            SEWER_TOP_LUT.mortar[bi]
        };
        pb.rect_fill(px0, py0, seg_w, 1, c);
    }
    if by0 <= 1 && by_end > 1 {
        let y1 = py0 + (1 - by0);
        let hi_c = if moss {
            SEWER_TOP_LUT.moss_hi
        } else {
            SEWER_TOP_LUT.hi[bi]
        };
        if bx_start == 0 {
            let m_c = if moss {
                SEWER_TOP_LUT.moss_mortar
            } else {
                SEWER_TOP_LUT.mortar[bi]
            };
            pb.plot(px0, y1, m_c);
            if seg_w > 1 {
                pb.rect_fill(px0 + 1, y1, seg_w - 1, 1, hi_c);
            }
        } else {
            pb.rect_fill(px0, y1, seg_w, 1, hi_c);
        }
    }
    if moss && by0 <= 2 && by_end > 2 {
        let y2 = py0 + (2 - by0);
        if bx_start == 0 {
            pb.plot(px0, y2, SEWER_TOP_LUT.moss_mortar);
            if seg_w > 1 {
                pb.rect_fill(px0 + 1, y2, seg_w - 1, 1, SEWER_TOP_LUT.moss_base);
            }
        } else {
            pb.rect_fill(px0, y2, seg_w, 1, SEWER_TOP_LUT.moss_base);
        }
    }
    if has_crack && bx_start <= 3 && bx_start + seg_w > 3 {
        let cx = px0 + (3 - bx_start);
        let cc = SEWER_TOP_LUT.crack[bi];
        if by0 <= 2 && by_end > 2 {
            pb.plot(cx, py0 + (2 - by0), cc);
        }
        if by0 <= 3 && by_end > 3 {
            pb.plot(cx, py0 + (3 - by0), cc);
        }
    }
}

/// Fast span-based Kitchen solid interior renderer (`NeighborMask::ALL`, full 16×16 cell).
#[inline]
fn draw_kitchen_solid_interior(pb: &mut PixelBuffer, sx: i32, sy: i32, view: &WallCellView) {
    let (wx0, wy0) = view.world_origin();
    let bh = pb.h as i32;

    let mut ly = 0;
    let mut plank = wy0.div_euclid(5);
    let mut py0 = wy0.rem_euclid(5);
    while ly < CELL_SIZE {
        let band_h = (5 - py0).min(CELL_SIZE - ly);
        let stagger = (plank & 1) * 11;
        let x_start = wx0 + stagger;
        let col0 = x_start.div_euclid(22);
        let sx0 = x_start.rem_euclid(22);
        let seg0_w = (22 - sx0).min(CELL_SIZE);
        let bi0 = (hash3(plank, col0, 0x2291) & 1) as usize;
        let bi1 = if seg0_w < CELL_SIZE {
            (hash3(plank, col0 + 1, 0x2291) & 1) as usize
        } else {
            0
        };

        for dy in 0..band_h {
            let py = sy + ly + dy;
            if py < 0 || py >= bh {
                continue;
            }
            let py_in = py0 + dy;
            let wy = wy0 + ly + dy;
            let rk = if py_in == 0 {
                1
            } else if py_in == 1 {
                2
            } else {
                0
            };

            pb.rect_fill(sx, py, seg0_w, 1, KITCHEN_TOP_LUT.row[rk][bi0]);
            if sx0 == 0 {
                pb.plot(sx, py, KITCHEN_TOP_LUT.seam[rk][bi0]);
            }
            if seg0_w < CELL_SIZE {
                pb.rect_fill(
                    sx + seg0_w,
                    py,
                    CELL_SIZE - seg0_w,
                    1,
                    KITCHEN_TOP_LUT.row[rk][bi1],
                );
                pb.plot(sx + seg0_w, py, KITCHEN_TOP_LUT.seam[rk][bi1]);
            }

            // Wood grain flecks (`(wx + wy * 3) & 7 == 0`): exactly 2 pixels per 16-px row
            let g0 = (-(wx0 + wy * 3)).rem_euclid(8);
            for lx in [g0, g0 + 8] {
                let c = if lx < seg0_w {
                    if sx0 == 0 && lx == 0 {
                        KITCHEN_TOP_LUT.seam_grain[rk][bi0]
                    } else {
                        KITCHEN_TOP_LUT.row_grain[rk][bi0]
                    }
                } else if lx == seg0_w {
                    KITCHEN_TOP_LUT.seam_grain[rk][bi1]
                } else {
                    KITCHEN_TOP_LUT.row_grain[rk][bi1]
                };
                pb.plot(sx + lx, py, c);
            }
        }
        ly += band_h;
        plank += 1;
        py0 = 0;
    }
}

/// Fill one row (`x_start..x_end`) of top-face material pixels without per-pixel
/// divisions, hashing, or floating-point palette math.
#[inline]
fn fill_top_row(
    theme: Theme,
    wx0: i32,
    wy: i32,
    cell_h: u32,
    x_start: i32,
    x_end: i32,
    row_px: &mut [Rgb; CELL_SIZE as usize],
) {
    match theme {
        Theme::Sewer => {
            let row = wy.div_euclid(6);
            let by = wy.rem_euclid(6);
            let stagger = (row & 1) * 4;
            let allow_moss = (cell_h & 7) < 3 && by <= 2;
            let has_crack = (cell_h & 15) == 4 && (by == 2 || by == 3);

            let x0 = wx0 + x_start + stagger;
            let mut col = x0.div_euclid(8);
            let mut bx = x0.rem_euclid(8);
            let mut sh = hash3(col, row, 0x1147);
            let mut bi = if sh & 3 == 0 {
                0
            } else if sh & 7 == 1 {
                1
            } else {
                2
            };
            let mut moss = allow_moss && (sh & 7) == 5;

            for lx in x_start..x_end {
                let c = if bx == 0 || by == 0 {
                    if moss {
                        SEWER_TOP_LUT.moss_mortar
                    } else {
                        SEWER_TOP_LUT.mortar[bi]
                    }
                } else if by == 1 {
                    if moss {
                        SEWER_TOP_LUT.moss_hi
                    } else {
                        SEWER_TOP_LUT.hi[bi]
                    }
                } else if moss {
                    SEWER_TOP_LUT.moss_base
                } else if has_crack && bx == 3 {
                    SEWER_TOP_LUT.crack[bi]
                } else {
                    SEWER_TOP_LUT.base[bi]
                };
                row_px[lx as usize] = c;

                bx += 1;
                if bx == 8 {
                    bx = 0;
                    col += 1;
                    sh = hash3(col, row, 0x1147);
                    bi = if sh & 3 == 0 {
                        0
                    } else if sh & 7 == 1 {
                        1
                    } else {
                        2
                    };
                    moss = allow_moss && (sh & 7) == 5;
                }
            }
        }
        Theme::Kitchen => {
            let plank = wy.div_euclid(5);
            let py_in = wy.rem_euclid(5);
            let rk = if py_in == 0 {
                1
            } else if py_in == 1 {
                2
            } else {
                0
            };
            let stagger = (plank & 1) * 11;
            let x0 = wx0 + x_start + stagger;
            let mut col = x0.div_euclid(22);
            let mut seam_x = x0.rem_euclid(22);
            let mut bi = (hash3(plank, col, 0x2291) & 1) as usize;
            let mut grain_phase = (wx0 + x_start + wy * 3) & 7;

            for lx in x_start..x_end {
                let c = if grain_phase == 0 {
                    if seam_x == 0 {
                        KITCHEN_TOP_LUT.seam_grain[rk][bi]
                    } else {
                        KITCHEN_TOP_LUT.row_grain[rk][bi]
                    }
                } else if seam_x == 0 {
                    KITCHEN_TOP_LUT.seam[rk][bi]
                } else {
                    KITCHEN_TOP_LUT.row[rk][bi]
                };
                row_px[lx as usize] = c;

                seam_x += 1;
                if seam_x == 22 {
                    seam_x = 0;
                    col += 1;
                    bi = (hash3(plank, col, 0x2291) & 1) as usize;
                }
                grain_phase = (grain_phase + 1) & 7;
            }
        }
        Theme::Lab => {
            let row = wy.div_euclid(8);
            let by = wy.rem_euclid(8);
            let stagger = (row & 1) * 10;
            let x0 = wx0 + x_start + stagger;
            let mut col = x0.div_euclid(20);
            let mut bx = x0.rem_euclid(20);
            let mut ph = hash3(col, row, 0x33F5);
            let mut bi = if (ph & 3) == 0 {
                0
            } else if (ph & 7) == 2 {
                1
            } else {
                2
            };
            let mut has_conduit = by == 4 && (ph & 7) == 5;
            let is_bolt_row = by == 2 || by == 6;

            for lx in x_start..x_end {
                let c = if bx == 0 || by == 0 {
                    LAB_TOP_LUT.seam[bi]
                } else if by == 1 && bx > 1 && bx < 19 {
                    LAB_TOP_LUT.hi[bi]
                } else if is_bolt_row && (bx == 2 || bx == 17) {
                    LAB_TOP_LUT.bolt[bi]
                } else if has_conduit && (4..=15).contains(&bx) {
                    LAB_TOP_LUT.conduit
                } else {
                    LAB_TOP_LUT.base[bi]
                };
                row_px[lx as usize] = c;

                bx += 1;
                if bx == 20 {
                    bx = 0;
                    col += 1;
                    ph = hash3(col, row, 0x33F5);
                    bi = if (ph & 3) == 0 {
                        0
                    } else if (ph & 7) == 2 {
                        1
                    } else {
                        2
                    };
                    has_conduit = by == 4 && (ph & 7) == 5;
                }
            }
        }
        Theme::Laundry => {
            for lx in x_start..x_end {
                row_px[lx as usize] = laundry_top(wx0 + lx, wy, cell_h);
            }
        }
        Theme::Printworks => {
            for lx in x_start..x_end {
                row_px[lx as usize] = printworks_top(wx0 + lx, wy, cell_h);
            }
        }
        _ => {
            for lx in x_start..x_end {
                row_px[lx as usize] = (96, 72, 44);
            }
        }
    }
}

/// Fill one row (`x_start..x_end`) of south front-face material (`fy in 0..5`)
/// without per-pixel divisions, hashing, or palette math.
#[allow(clippy::too_many_arguments)]
#[inline]
fn fill_front_row(
    theme: Theme,
    wx0: i32,
    fy: i32,
    cell_h: u32,
    bay_h: u32,
    has_w: bool,
    has_e: bool,
    pal: &WallPalette,
    x_start: i32,
    x_end: i32,
    row_px: &mut [Rgb; CELL_SIZE as usize],
) {
    match theme {
        Theme::Sewer => {
            let course = if fy < 2 { 0 } else { 1 };
            let x0 = wx0 + x_start + course * 3;
            let mut bcol = x0.div_euclid(6);
            let mut bx = x0.rem_euclid(6);
            let mut bi = (hash3(bcol, course, 0x4129) & 1) as usize;
            let has_moss_tongue = (cell_h & 7) < 3;
            let mx = 3 + ((cell_h >> 3) % 9) as i32;
            let moss_col = if fy == 1 {
                SEWER_FRONT_LUT.moss_hi
            } else {
                SEWER_FRONT_LUT.moss_lo
            };

            for lx in x_start..x_end {
                let mut c = if fy == 2 || bx == 0 {
                    pal.mortar
                } else if fy == 1 && bx == 1 {
                    SEWER_FRONT_LUT.brick_hi[bi]
                } else if fy == 4 {
                    SEWER_FRONT_LUT.brick_bot[bi]
                } else {
                    SEWER_FRONT_LUT.brick[bi]
                };
                if has_moss_tongue {
                    let md = (lx - mx).abs();
                    if md <= 2 && fy <= (3 - md).max(0) {
                        c = mix_c(c, moss_col, 0.78);
                    }
                }
                row_px[lx as usize] = c;

                bx += 1;
                if bx == 6 {
                    bx = 0;
                    bcol += 1;
                    bi = (hash3(bcol, course, 0x4129) & 1) as usize;
                }
            }
        }
        Theme::Kitchen => {
            let is_cabinet = (bay_h & 1) == 0 && (has_w || has_e);
            if is_cabinet {
                if fy == 4 {
                    for lx in x_start..x_end {
                        row_px[lx as usize] = pal.front_dark;
                    }
                } else {
                    let panel_row = fy == 1 || fy == 2;
                    let panel_c = if fy == 1 {
                        KITCHEN_FRONT_LUT.cab_panel_sh
                    } else {
                        KITCHEN_FRONT_LUT.cab_hi
                    };
                    for lx in x_start..x_end {
                        row_px[lx as usize] = if lx == 0 || lx == 15 {
                            KITCHEN_FRONT_LUT.cab_div
                        } else if lx == 7 {
                            KITCHEN_FRONT_LUT.cab_seam
                        } else if lx == 8 {
                            KITCHEN_FRONT_LUT.cab_hi
                        } else if panel_row {
                            panel_c
                        } else {
                            pal.front_base
                        };
                    }
                }
            } else if fy >= 3 {
                let c = if fy == 3 {
                    KITCHEN_FRONT_LUT.skirt_lip
                } else {
                    pal.front_dark
                };
                for lx in x_start..x_end {
                    row_px[lx as usize] = c;
                }
            } else {
                let course = if fy <= 1 { 0 } else { 1 };
                let x0 = wx0 + x_start + course * 2;
                let mut tile_id = x0.div_euclid(5);
                let mut tile_x = x0.rem_euclid(5);
                for lx in x_start..x_end {
                    row_px[lx as usize] = if tile_x == 0 {
                        KITCHEN_FRONT_LUT.tile_grout
                    } else {
                        let ti = ((tile_id + course) & 1) as usize;
                        if fy == 2 {
                            KITCHEN_FRONT_LUT.tile_bot[ti]
                        } else if fy == 1 && tile_x == 1 {
                            KITCHEN_FRONT_LUT.tile_hi[ti]
                        } else {
                            KITCHEN_FRONT_LUT.tile[ti]
                        }
                    };
                    tile_x += 1;
                    if tile_x == 5 {
                        tile_x = 0;
                        tile_id += 1;
                    }
                }
            }
        }
        Theme::Lab => {
            let hazard = (bay_h & 3) == 1 || (!has_w || !has_e);
            if hazard && (fy == 2 || fy == 3) {
                let hi_c = if fy == 2 {
                    LAB_FRONT_LUT.hazard_hi
                } else {
                    LAB_FRONT_LUT.hazard_lo
                };
                let mut stripe = (wx0 + x_start + fy).rem_euclid(6);
                for lx in x_start..x_end {
                    row_px[lx as usize] = if stripe < 3 { hi_c } else { pal.front_dark };
                    stripe += 1;
                    if stripe == 6 {
                        stripe = 0;
                    }
                }
            } else if fy == 4 {
                for lx in x_start..x_end {
                    row_px[lx as usize] = pal.front_dark;
                }
            } else {
                let mid = fy == 2;
                for lx in x_start..x_end {
                    row_px[lx as usize] = if lx == 3 || lx == 12 {
                        LAB_FRONT_LUT.rib_lo
                    } else if lx == 4 || lx == 13 {
                        LAB_FRONT_LUT.rib_hi
                    } else if mid && lx > 4 && lx < 12 {
                        LAB_FRONT_LUT.mid_band
                    } else {
                        pal.front_base
                    };
                }
            }
        }
        Theme::Laundry => {
            for lx in x_start..x_end {
                row_px[lx as usize] = laundry_front(wx0 + lx, fy, bay_h, has_w, has_e);
            }
        }
        Theme::Printworks => {
            for lx in x_start..x_end {
                row_px[lx as usize] = printworks_front(wx0 + lx, fy, bay_h, has_w, has_e);
            }
        }
        _ => {
            for lx in x_start..x_end {
                row_px[lx as usize] = pal.front_base;
            }
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

    // Fast path for interior bedrock cells (no outlines, no front face, no bevels).
    if m.0 == NeighborMask::ALL.0
        && x_range.start == 0
        && x_range.end == CELL_SIZE
        && y_range.start == 0
        && y_range.end == CELL_SIZE
    {
        match view.theme {
            Theme::Lab => draw_lab_solid_interior(pb, sx, sy, view),
            Theme::Sewer => {
                let cell_h = hash3(view.cell_x, view.cell_y, view.seed);
                draw_sewer_solid_interior(pb, sx, sy, view, cell_h);
            }
            Theme::Kitchen => draw_kitchen_solid_interior(pb, sx, sy, view),
            // The side routes fill a buried cell with their own material, so a
            // wall mass never inherits a donor's bricks.
            Theme::Laundry => draw_wave3_solid_interior(pb, sx, sy, view, true),
            Theme::Printworks => draw_wave3_solid_interior(pb, sx, sy, view, false),
            _ => {}
        }
        return;
    }

    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x_start = x_range.start.max(-sx);
    let x_end = x_range.end.min(bw - sx);
    let y_start = y_range.start.max(-sy);
    let y_end = y_range.end.min(bh - sy);
    if x_start >= x_end || y_start >= y_end {
        return;
    }

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

    let mut row_px = [(0u8, 0u8, 0u8); CELL_SIZE as usize];
    let top_end = y_end.min(top_h);

    // --- Lit top face (`ly in y_start..top_end`) ---
    for ly in y_start..top_end {
        let py = sy + ly;
        if ly == 0 && !has_n {
            pb.rect_fill(sx + x_start, py, x_end - x_start, 1, pal.outline);
            continue;
        }
        let wy = wy0 + ly;
        fill_top_row(view.theme, wx0, wy, cell_h, x_start, x_end, &mut row_px);

        let cornice = has_front && ly == TOP_FACE_H - 1;
        let bevel_n = ly == 1 && !has_n;
        let bevel_w = !has_w || (has_s && !has_sw && ly >= TOP_FACE_H);
        let bevel_e = !has_e || (has_s && !has_se && ly >= TOP_FACE_H);

        if cornice {
            for lx in x_start..x_end {
                row_px[lx as usize] = mix_c(row_px[lx as usize], pal.cornice, 0.74);
            }
        } else if bevel_n {
            for lx in x_start..x_end {
                row_px[lx as usize] = mix_c(row_px[lx as usize], pal.top_hi, 0.68);
            }
        } else {
            if bevel_w && x_start <= 1 && 1 < x_end {
                row_px[1] = mix_c(row_px[1], pal.top_hi, 0.68);
            }
            if bevel_e && x_start <= CELL_SIZE - 2 && CELL_SIZE - 2 < x_end {
                row_px[(CELL_SIZE - 2) as usize] =
                    mix_c(row_px[(CELL_SIZE - 2) as usize], pal.top_lo, 0.56);
            }
        }

        // Inner concave corner catchlights
        if has_n && has_w && !has_nw {
            if ly == 0 && x_start <= 1 && 1 < x_end {
                row_px[1] = mix_c(row_px[1], pal.top_hi, 0.54);
            } else if ly == 1 && x_start == 0 {
                row_px[0] = mix_c(row_px[0], pal.top_hi, 0.54);
            }
        }
        if has_n
            && has_e
            && !has_ne
            && ly == 0
            && x_start <= CELL_SIZE - 2
            && CELL_SIZE - 2 < x_end
        {
            row_px[(CELL_SIZE - 2) as usize] =
                mix_c(row_px[(CELL_SIZE - 2) as usize], pal.top_lo, 0.45);
        }

        // Crisp 1 px left/right outer & concave inner outlines
        if x_start == 0
            && (!has_w
                || (ly == 0 && has_n && has_w && !has_nw)
                || (ly >= TOP_FACE_H && has_s && has_w && !has_sw))
        {
            row_px[0] = pal.outline;
        }
        if x_end == CELL_SIZE
            && (!has_e
                || (ly == 0 && has_n && has_e && !has_ne)
                || (ly >= TOP_FACE_H && has_s && has_e && !has_se))
        {
            row_px[(CELL_SIZE - 1) as usize] = pal.outline;
        }

        for lx in x_start..x_end {
            pb.plot(sx + lx, py, row_px[lx as usize]);
        }
    }

    // --- South vertical front face (`ly in top_h..CELL_SIZE`) ---
    if has_front {
        let front_start = y_start.max(top_h);
        for ly in front_start..y_end {
            let py = sy + ly;
            if ly == CELL_SIZE - 1 {
                pb.rect_fill(sx + x_start, py, x_end - x_start, 1, pal.outline);
                continue;
            }
            let fy = ly - TOP_FACE_H;
            fill_front_row(
                view.theme,
                wx0,
                fy,
                cell_h,
                bay_h,
                has_w,
                has_e,
                pal,
                x_start,
                x_end,
                &mut row_px,
            );

            // Crisp horizontal cast shadow right beneath the protruding top cornice
            if fy == 0 {
                for lx in x_start..x_end {
                    row_px[lx as usize] = scale_c(row_px[lx as usize], 0.58);
                }
            }
            // Ambient occlusion from protruding west/east L-walls
            if has_w && has_sw {
                if x_start == 0 {
                    row_px[0] = scale_c(row_px[0], 0.44);
                }
                if x_start <= 1 && 1 < x_end {
                    row_px[1] = scale_c(row_px[1], 0.66);
                }
                if x_start <= 2 && 2 < x_end {
                    row_px[2] = scale_c(row_px[2], 0.84);
                }
            }
            if has_e && has_se && x_end == CELL_SIZE {
                row_px[(CELL_SIZE - 1) as usize] = scale_c(row_px[(CELL_SIZE - 1) as usize], 0.72);
            }
            if !has_w && x_start <= 1 && 1 < x_end {
                row_px[1] = scale_c(row_px[1], 1.18);
            }
            if !has_e && x_start <= CELL_SIZE - 2 && CELL_SIZE - 2 < x_end {
                row_px[(CELL_SIZE - 2) as usize] =
                    scale_c(row_px[(CELL_SIZE - 2) as usize], 0.72);
            }
            if !has_w && x_start == 0 {
                row_px[0] = pal.outline;
            }
            if !has_e && x_end == CELL_SIZE {
                row_px[(CELL_SIZE - 1) as usize] = pal.outline;
            }

            for lx in x_start..x_end {
                pb.plot(sx + lx, py, row_px[lx as usize]);
            }
        }

        // --- Cohesive per-cell surface accents on exposed south front face ---
        if x_range.start == 0 && x_range.end == CELL_SIZE {
            draw_front_accents(pb, sx, sy, view, cell_h, bay_h, pal, accent);
        }
    }
}

#[inline]
fn sample_top_material(
    theme: Theme,
    wx: i32,
    wy: i32,
    cell_h: u32,
    _pal: &WallPalette,
) -> Rgb {
    match theme {
        Theme::Sewer => {
            let row = wy.div_euclid(6);
            let stagger = (row & 1) * 4;
            let col = (wx + stagger).div_euclid(8);
            let bx = (wx + stagger).rem_euclid(8);
            let by = wy.rem_euclid(6);
            let sh = hash3(col, row, 0x1147);
            let bi = if sh & 3 == 0 {
                0
            } else if sh & 7 == 1 {
                1
            } else {
                2
            };
            let moss = (cell_h & 7) < 3 && (sh & 7) == 5 && by <= 2;
            if bx == 0 || by == 0 {
                if moss {
                    SEWER_TOP_LUT.moss_mortar
                } else {
                    SEWER_TOP_LUT.mortar[bi]
                }
            } else if by == 1 {
                if moss {
                    SEWER_TOP_LUT.moss_hi
                } else {
                    SEWER_TOP_LUT.hi[bi]
                }
            } else if moss {
                SEWER_TOP_LUT.moss_base
            } else if (cell_h & 15) == 4 && bx == 3 && (by == 2 || by == 3) {
                SEWER_TOP_LUT.crack[bi]
            } else {
                SEWER_TOP_LUT.base[bi]
            }
        }
        Theme::Kitchen => {
            let plank = wy.div_euclid(5);
            let py_in = wy.rem_euclid(5);
            let rk = if py_in == 0 {
                1
            } else if py_in == 1 {
                2
            } else {
                0
            };
            let x = wx + (plank & 1) * 11;
            let seam_x = x.rem_euclid(22);
            let bi = (hash3(plank, x.div_euclid(22), 0x2291) & 1) as usize;
            let grain = ((wx + wy * 3) & 7) == 0;
            if grain {
                if seam_x == 0 {
                    KITCHEN_TOP_LUT.seam_grain[rk][bi]
                } else {
                    KITCHEN_TOP_LUT.row_grain[rk][bi]
                }
            } else if seam_x == 0 {
                KITCHEN_TOP_LUT.seam[rk][bi]
            } else {
                KITCHEN_TOP_LUT.row[rk][bi]
            }
        }
        Theme::Lab => {
            let row = wy.div_euclid(8);
            let stagger = (row & 1) * 10;
            let x = wx + stagger;
            let col = x.div_euclid(20);
            let bx = x.rem_euclid(20);
            let by = wy.rem_euclid(8);
            let ph = hash3(col, row, 0x33F5);
            let bi = if (ph & 3) == 0 {
                0
            } else if (ph & 7) == 2 {
                1
            } else {
                2
            };
            if bx == 0 || by == 0 {
                LAB_TOP_LUT.seam[bi]
            } else if by == 1 && bx > 1 && bx < 19 {
                LAB_TOP_LUT.hi[bi]
            } else if (bx == 2 || bx == 17) && (by == 2 || by == 6) {
                LAB_TOP_LUT.bolt[bi]
            } else if (ph & 7) == 5 && by == 4 && (4..=15).contains(&bx) {
                LAB_TOP_LUT.conduit
            } else {
                LAB_TOP_LUT.base[bi]
            }
        }
        Theme::Laundry => laundry_top(wx, wy, cell_h),
        Theme::Printworks => printworks_top(wx, wy, cell_h),
        _ => (96, 72, 44),
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

    match wall_style(view.theme) {
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
        Theme::Laundry => {
            let is_washer_bay = (bay_h & 1) == 0 && (has_w || has_e);
            if is_washer_bay {
                // Recessed chrome washer porthole bezel with soft aqua drum glint
                // (makes long washer banks in Laundry halls read as real machines!)
                pb.rect_fill(sx + 5, front_y + 1, 6, 3, pal.outline);
                pb.plot(sx + 5, front_y + 1, pal.trim_hi);
                pb.plot(sx + 10, front_y + 1, pal.detail_b);
                let shimmer = 0.55 + 0.45 * (view.anim_clock * 2.2 + view.cell_x as f32).sin();
                let water = palette::mix((28, 56, 78), accent, 0.35 + 0.35 * shimmer);
                pb.plot(sx + 6, front_y + 2, water);
                pb.plot(sx + 7, front_y + 2, palette::mix(water, pal.trim_hi, 0.45));
                pb.plot(sx + 8, front_y + 2, water);
                pb.plot(sx + 9, front_y + 2, pal.detail_c); // hint of pink wash tumbling!
                // Little soap drawer pull on the left & cycle dial on the right.
                pb.plot(sx + 2, front_y + 1, pal.trim_hi);
                pb.plot(sx + 13, front_y + 1, pal.detail_a);
            } else if (cell_h & 3) == 1 && has_w && has_e {
                // Brass hot/cold laundry spigot & soap-pipe collar on washboard bay.
                pb.plot(sx + 7, front_y + 1, LAUNDRY_LUT.brass_hi);
                pb.plot(sx + 8, front_y + 1, LAUNDRY_LUT.brass_lo);
                pb.plot(sx + 7, front_y + 2, pal.detail_c);
                pb.plot(sx + 8, front_y + 2, accent);
            }
        }
        Theme::Printworks => {
            let is_ream = (bay_h & 1) == 1 && (has_w || has_e);
            if is_ream {
                // Bound twine straps holding stacked newsprint reams.
                for &tx in &[4, 11] {
                    pb.plot(sx + tx, front_y + 1, pal.detail_a);
                    pb.plot(sx + tx, front_y + 2, pal.detail_a);
                    pb.plot(sx + tx, front_y + 3, PRINTWORKS_LUT.brass_quoin);
                }
            } else if has_w || has_e {
                // Brass press nameplate + subtle pulsing cyan registration lamp.
                pb.rect_fill(sx + 5, front_y + 1, 5, 1, PRINTWORKS_LUT.brass_quoin);
                if (cell_h & 3) == 0 {
                    let pulse = 0.65 + 0.35 * (view.anim_clock * 2.4 + view.cell_x as f32 * 1.3).sin();
                    pb.plot(sx + 13, front_y + 1, palette::scale(pal.detail_b, 0.75 + 0.55 * pulse));
                    pb.plot(sx + 13, front_y + 2, pal.outline);
                }
            }
        }
        _ => {}
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

    // Precompute theme-specific maze colors once per cell instead of per pixel.
    let lab_top_base = mix_c(pal.maze_top, accent, 0.16);
    let lab_top_sheen = mix_c(lab_top_base, pal.detail_c, 0.34);
    let lab_front_mid = mix_c(pal.maze_front, pal.maze_top, 0.36);
    let lab_haz_hi = scale_c(pal.detail_a, 0.86);
    let sewer_top_rib = scale_c(pal.maze_top, 0.84);
    let sewer_front_rib = scale_c(pal.maze_front, 0.76);
    let kitchen_top_lo = scale_c(pal.maze_top, 0.84);
    let kitchen_top_hi = scale_c(pal.maze_top, 1.04);
    let kitchen_front_lo = scale_c(pal.maze_front, 0.78);
    let kitchen_front_hi = scale_c(pal.maze_front, 1.06);

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
                let mut c = match wall_style(view.theme) {
                    Theme::Sewer => {
                        // Riveted weathered iron-slat culvert divider with rust flecks
                        let mut b = if (wx + wy) % 6 == 0 {
                            sewer_top_rib
                        } else {
                            pal.maze_top
                        };
                        if ((wx * 3 + wy * 5) & 15) == 0 {
                            b = mix_c(b, pal.detail_c, 0.34);
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
                        } else if ((wx + wy) & 3) == 0 {
                            kitchen_top_lo
                        } else {
                            kitchen_top_hi
                        }
                    }
                    Theme::Lab => {
                        // Frosted cyan polycarbonate panel framed in steel stanchion posts
                        let at_post = (wx % 8 == 0) && (ly == MAZE_INSET_Y + 1 || ly == y_front_start - 1);
                        if at_post {
                            pal.trim_hi
                        } else if (wx + wy).rem_euclid(7) == 0 {
                            lab_top_sheen
                        } else {
                            lab_top_base
                        }
                    }
                    Theme::Laundry => {
                        // Porcelain-capped ribbed washboard & drying-rack divider
                        let at_post = (wx % 8 == 0) && (ly == MAZE_INSET_Y + 1 || ly == y_front_start - 1);
                        if at_post {
                            pal.detail_a
                        } else if (wx & 3) == 0 {
                            LAUNDRY_LUT.porcelain_hi
                        } else if (wx & 3) == 2 {
                            LAUNDRY_LUT.rib_lo[0]
                        } else {
                            pal.maze_top
                        }
                    }
                    Theme::Printworks => {
                        // Charcoal galley-rack divider with pale newsprint cap & cyan clip
                        let at_clip = (wx % 10 == 0) && ly == MAZE_INSET_Y + 1;
                        if at_clip {
                            pal.detail_b
                        } else if (wx + wy) % 5 == 0 {
                            PRINTWORKS_LUT.ream_mid
                        } else {
                            pal.maze_top
                        }
                    }
                    _ => kitchen_top_hi,
                };

                let lit_n = !in_maze_top_ext(lx, ly - 2, m) && !in_maze_any(lx, ly - 2, m);
                let lit_w = !in_maze_top_ext(lx - 2, ly, m) && !in_maze_any(lx - 2, ly, m);
                let shade_e = !in_maze_top_ext(lx + 2, ly, m) && !in_maze_any(lx + 2, ly, m);
                let cornice = in_maze_front(lx, ly + 1, m);

                if cornice {
                    c = mix_c(c, pal.maze_hi, 0.74);
                } else if lit_n || lit_w {
                    c = mix_c(c, pal.maze_hi, 0.56);
                } else if shade_e {
                    c = scale_c(c, 0.70);
                }
                pb.plot(px, py, c);
            } else {
                // 5 px tall maze partition south front face (`fy in 0..4`, `fy == 4` is outline)
                let fy = ly - y_front_start;
                let mut c = match wall_style(view.theme) {
                    Theme::Sewer => {
                        let mut b = if (wx & 3) == 0 {
                            sewer_front_rib
                        } else {
                            pal.maze_front
                        };
                        if fy == 3 {
                            b = mix_c(b, pal.detail_a, 0.42);
                        }
                        b
                    }
                    Theme::Kitchen => {
                        if fy == 3 {
                            pal.front_dark
                        } else if (wx & 2) == 0 {
                            kitchen_front_lo
                        } else {
                            kitchen_front_hi
                        }
                    }
                    Theme::Lab => {
                        if fy == 3 {
                            let s = (wx + fy).rem_euclid(6);
                            if s < 3 {
                                lab_haz_hi
                            } else {
                                pal.front_dark
                            }
                        } else if (wx & 7) == 0 {
                            pal.trim_hi
                        } else {
                            lab_front_mid
                        }
                    }
                    Theme::Laundry => {
                        if fy == 3 {
                            LAUNDRY_LUT.brass_lo
                        } else if (wx & 3) == 0 {
                            LAUNDRY_LUT.front_flute_hi
                        } else {
                            pal.maze_front
                        }
                    }
                    Theme::Printworks => {
                        if fy == 3 {
                            PRINTWORKS_LUT.ink_stripe
                        } else if (wx & 7) == 0 {
                            PRINTWORKS_LUT.ream_lo
                        } else {
                            pal.maze_front
                        }
                    }
                    _ => kitchen_front_hi,
                };
                if fy == 0 {
                    c = scale_c(c, 0.60);
                }
                if in_maze_top(lx - 1, ly, m) || in_maze_top(lx - 2, ly, m) {
                    c = scale_c(c, 0.56);
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
    let pip_col = match wall_style(theme) {
        Theme::Lab => palette::scale(accent, 0.90),
        Theme::Kitchen => pal.detail_a,
        Theme::Sewer => pal.trim_hi,
        Theme::Laundry => pal.detail_a,
        Theme::Printworks => pal.detail_b,
        _ => pal.detail_a,
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
    let pip_col = match wall_style(theme) {
        Theme::Lab => palette::scale(accent, 0.90),
        Theme::Kitchen => pal.detail_a,
        Theme::Sewer => pal.trim_hi,
        Theme::Laundry => pal.detail_a,
        Theme::Printworks => pal.detail_b,
        _ => pal.detail_a,
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
    let leaf_col = match wall_style(view.theme) {
        Theme::Sewer => pal.detail_b,
        Theme::Kitchen => pal.maze_top,
        Theme::Lab => pal.maze_top,
        Theme::Laundry => pal.cornice,
        Theme::Printworks => pal.maze_top,
        Theme::Trailer => pal.maze_top,
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
    // The getaway ride's own layers (cargo, rails, the road over the side).
    if view.theme == Theme::Trailer {
        return crate::game::art_trailer::draw_wall_overhang(pb, view, cam, _accent);
    }
    if matches!(
        view.kind,
        WallKind::Solid
            | WallKind::Maze
            | WallKind::Doorway {
                axis: WallAxis::Vertical
            }
            | WallKind::OneWayDoor {
                dir: Cardinal::East | Cardinal::West
            }
    ) {
        return;
    }
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
                    let mut c = sample_top_material(view.theme, wx, wy, cell_h, pal);
                    if (ly == 0 && !has_n) || ly == 4 {
                        c = pal.outline;
                    } else if ly == 1 && !has_n {
                        c = mix_c(c, pal.top_hi, 0.68);
                    } else if ly == 3 {
                        c = mix_c(c, pal.cornice, 0.72);
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
                    let mut c = sample_top_material(view.theme, wx, wy, cell_h, pal);
                    if in_gap {
                        if lx == 6 {
                            c = mix_c(c, pal.top_hi, 0.66);
                        } else if lx == 9 {
                            c = mix_c(c, pal.top_lo, 0.54);
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
                let base = sample_top_material(view.theme, wx, wy0 + 1, cell_h, pal);
                pb.plot(sx + lx, sy, pal.outline);
                pb.plot(sx + lx, sy + 1, mix_c(base, pal.top_hi, 0.52));
                pb.plot(sx + lx, sy + 2, mix_c(base, pal.cornice, 0.45));
                pb.plot(sx + lx, sy + 3, pal.outline);
                darken_px(pb, sx + lx, sy + 4, 0.58);
                darken_px(pb, sx + lx, sy + 5, 0.80);
            }
        }
        _ => {}
    }
}

// ============================================================================
// BOSS ARENA THRESHOLD OVERLAY (WALLKIND-INDEPENDENT, ZERO COLLISION IMPACT)
// ============================================================================

/// Extra floor-apron reach (in world pixels) extending from a boss arena mouth
/// cell onto the immediately adjacent approach and interior floor cells.
/// Purely decorative ground decals — never solid.
pub const BOSS_THRESHOLD_APRON: i32 = 9;

/// Plain `Copy` view describing one mouth cell of a boss arena's double doorway.
///
/// This is a **purely decorative, `WallKind`-independent overlay**: calling
/// [`draw_boss_threshold_ground`] or [`draw_boss_threshold_overhang`] **never
/// changes collision** (`wall_colliders` for the underlying doorway cells is
/// completely unaffected, preserving the full 30 px boss clearance).
///
/// # Cell Coverage
/// Call once for each doorway mouth cell of the boss arena:
/// - **North / South arena wall** (`inward` is [`Cardinal::South`] / [`Cardinal::North`]):
///   the **2 horizontally adjacent doorway cells** `(cx, cy)` and `(cx + 1, cy)`
///   piercing the boss room's 1-cell perimeter wall ring, with the shared inner
///   mask bit (`E` / `W`) unset so the two cells fuse into one 32 px landmark.
/// - **West / East arena wall** (`inward` is [`Cardinal::East`] / [`Cardinal::West`]):
///   the **2 vertically adjacent doorway cells** `(cx, cy)` and `(cx, cy + 1)`
///   piercing the boss room's 1-cell perimeter wall ring, with the shared inner
///   mask bit (`S` / `N`) unset.
/// - **Floor apron bleed**: ground decals extend up to [`BOSS_THRESHOLD_APRON`]
///   (`9 px`) onto the immediately adjacent open floor cells on both the corridor
///   approach side and the arena interior side. Single-cell doorways are also
///   supported gracefully (centered motif).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossThresholdView {
    /// Underlying mouth cell view (`cell_x`, `cell_y`, `seed`, `theme`, `mask`,
    /// `anim_clock`; `kind` is ignored so callers do not need a new `WallKind`).
    pub cell: WallCellView,
    /// Cardinal direction pointing **into** the boss arena from the approach corridor.
    pub inward: Cardinal,
}

impl BossThresholdView {
    /// Construct a boss-arena threshold overlay for one mouth cell.
    #[inline]
    pub const fn new(cell: WallCellView, inward: Cardinal) -> Self {
        Self { cell, inward }
    }
}

/// Map doorway-local `(u, v)` coordinates into screen pixels `(px, py)`:
/// - `u` (`0..16`) runs along the doorway opening width (parallel to the wall).
/// - `v` (`-BOSS_THRESHOLD_APRON..CELL_SIZE + BOSS_THRESHOLD_APRON`) runs
///   perpendicular to the wall, increasing **into** the boss arena (`inward`).
#[inline]
fn boss_uv_to_screen(sx: i32, sy: i32, inward: Cardinal, u: i32, v: i32) -> (i32, i32) {
    match inward {
        Cardinal::South => (sx + u, sy + v),
        Cardinal::North => (sx + u, sy + (CELL_SIZE - 1 - v)),
        Cardinal::East => (sx + v, sy + u),
        Cardinal::West => (sx + (CELL_SIZE - 1 - v), sy + u),
    }
}

/// Compute local opening span `(u0, u1)` and 32 px double-doorway offset `du_off`
/// so two adjacent mouth cells seamlessly form one centered 32 px threshold.
#[inline]
fn boss_opening_span(mask: NeighborMask, inward: Cardinal) -> (i32, i32, i32) {
    let (has_lo, has_hi) = match inward.wall_axis() {
        WallAxis::Horizontal => (mask.has(NeighborMask::W), mask.has(NeighborMask::E)),
        WallAxis::Vertical => (mask.has(NeighborMask::N), mask.has(NeighborMask::S)),
    };
    let u0 = if has_lo { DOORWAY_JAMB_VISUAL } else { 0 };
    let u1 = if has_hi {
        CELL_SIZE - DOORWAY_JAMB_VISUAL
    } else {
        CELL_SIZE
    };
    let du_off = match (has_lo, has_hi) {
        (true, false) => 0,  // left / top half of 2-cell mouth -> du in 2..16
        (false, true) => 16, // right / bottom half of 2-cell mouth -> du in 16..30
        _ => 8,              // single-cell or middle cell -> centered
    };
    (u0, u1, du_off)
}

#[inline]
fn boss_culled(pb: &PixelBuffer, sx: i32, sy: i32) -> bool {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let pad = BOSS_THRESHOLD_APRON + 2;
    sx >= bw + pad || sy >= bh + pad || sx + CELL_SIZE + pad < 0 || sy + CELL_SIZE + pad < 0
}

/// **Ground layer**: draw the boss arena's per-theme landmark threshold overlay
/// for one mouth cell (claw-scored stone for Sewer, worn doormat & scuffed lino
/// edge for Kitchen, hazard chevrons & scorched lawn-strip for Lab).
///
/// Call in the Ground layer right **after** [`draw_wall_ground`] and **before**
/// entities and [`draw_wall_cell`]. Zero heap allocation; **never changes collision**.
pub fn draw_boss_threshold_ground(
    pb: &mut PixelBuffer,
    view: &BossThresholdView,
    cam: Vec2,
    accent: Rgb,
) {
    let (sx, sy) = cell_screen_origin(&view.cell, cam);
    if boss_culled(pb, sx, sy) {
        return;
    }
    let pal = theme_palette(view.cell.theme);
    let (u0, u1, du_off) = boss_opening_span(view.cell.mask, view.inward);

    match wall_style(view.cell.theme) {
        Theme::Sewer => {
            draw_boss_threshold_sewer(pb, sx, sy, view.inward, u0, u1, du_off, view.cell.seed, pal);
        }
        Theme::Kitchen => {
            draw_boss_threshold_kitchen(pb, sx, sy, view.inward, u0, u1, du_off, pal);
        }
        Theme::Lab => {
            draw_boss_threshold_lab(
                pb,
                sx,
                sy,
                view.inward,
                u0,
                u1,
                du_off,
                view.cell.anim_clock,
                pal,
                accent,
            );
        }
        Theme::Laundry => {
            draw_boss_threshold_laundry(pb, sx, sy, view.inward, u0, u1, du_off, pal);
        }
        Theme::Printworks => {
            draw_boss_threshold_printworks(
                pb,
                sx,
                sy,
                view.inward,
                u0,
                u1,
                du_off,
                view.cell.anim_clock,
                pal,
            );
        }
        _ => {}
    }
}

/// **Overhang layer (optional)**: add subtle per-theme boss-doorway trim to the
/// 4 px horizontal doorway lintel (claw-notched stone for Sewer, brass-cornered
/// timber architrave for Kitchen, hazard-striped warning beam + beacons for Lab,
/// striped mangle-roller trim for Laundry, reversed type notches + cyan lamps for Printworks).
///
/// Call right **after** [`draw_wall_overhang`] for horizontal boss doorways.
/// No-op for vertical doorways. Zero heap allocation; **never changes collision**.
pub fn draw_boss_threshold_overhang(
    pb: &mut PixelBuffer,
    view: &BossThresholdView,
    cam: Vec2,
    accent: Rgb,
) {
    if view.inward.wall_axis() != WallAxis::Horizontal {
        return;
    }
    let (sx, sy) = cell_screen_origin(&view.cell, cam);
    if culled(pb, sx, sy) {
        return;
    }
    let pal = theme_palette(view.cell.theme);
    let (u0, u1, du_off) = boss_opening_span(view.cell.mask, view.inward);

    match wall_style(view.cell.theme) {
        Theme::Sewer => {
            // Three bold chipped jaguar claw-notches across the overhead stone lintel
            let chip_hi: Rgb = (162, 190, 174);
            let claw_dark: Rgb = (8, 12, 11);
            for u in u0..u1 {
                let du = u + du_off;
                let px = sx + u;
                if du == 9 || du == 15 || du == 21 {
                    pb.plot(px, sy + 1, claw_dark);
                    pb.plot(px + 1, sy + 2, claw_dark);
                    pb.plot(px, sy + 2, chip_hi);
                    pb.plot(px - 1, sy + 1, chip_hi);
                } else if du == 3 || du == 28 {
                    pb.plot(px, sy + 2, pal.detail_a);
                    pb.plot(px, sy + 3, scale_c(pal.detail_a, 0.72));
                }
            }
        }
        Theme::Kitchen => {
            // Tarnished brass corner brackets + oak architrave header highlight
            let brass_hi: Rgb = (232, 200, 122);
            let brass_mid: Rgb = (188, 152, 78);
            for u in u0..u1 {
                let du = u + du_off;
                let px = sx + u;
                if (2..=5).contains(&du) || (26..=29).contains(&du) {
                    pb.plot(px, sy + 1, brass_hi);
                    pb.plot(px, sy + 2, brass_mid);
                    if du == 3 || du == 28 {
                        pb.plot(px, sy + 2, pal.outline);
                    }
                }
            }
        }
        Theme::Lab => {
            // Bold amber-and-charcoal hazard header stripe + framed pulsing warning beacons
            let haz_hi: Rgb = (234, 182, 54);
            let haz_lo: Rgb = (180, 134, 38);
            let haz_dk: Rgb = (18, 22, 32);
            let pulse = 0.55 + 0.45 * (view.cell.anim_clock * 4.2).sin();
            let beacon = mix_c((232, 66, 44), accent, 0.16 * pulse);
            for u in u0..u1 {
                let du = u + du_off;
                let px = sx + u;
                if du == 3 || du == 28 {
                    pb.plot(px, sy + 1, scale_c(beacon, 0.72 + 0.28 * pulse));
                    pb.plot(px, sy + 2, scale_c(beacon, 0.48 + 0.28 * pulse));
                } else if du == 4 || du == 27 {
                    pb.plot(px, sy + 1, pal.outline);
                    pb.plot(px, sy + 2, pal.outline);
                } else if (5..=26).contains(&du) {
                    let s1 = (du + 1).rem_euclid(6);
                    let s2 = (du + 2).rem_euclid(6);
                    pb.plot(px, sy + 1, if s1 < 3 { haz_hi } else { haz_dk });
                    pb.plot(px, sy + 2, if s2 < 3 { haz_lo } else { haz_dk });
                }
            }
        }
        Theme::Laundry => {
            // Porcelain-and-pink candy-striped mangle roller trim + brass end caps
            let stripe_a: Rgb = (206, 220, 218);
            let stripe_b: Rgb = pal.detail_c;
            for u in u0..u1 {
                let du = u + du_off;
                let px = sx + u;
                if du == 3 || du == 4 || du == 27 || du == 28 {
                    pb.plot(px, sy + 1, LAUNDRY_LUT.brass_hi);
                    pb.plot(px, sy + 2, LAUNDRY_LUT.brass_lo);
                } else if (6..=25).contains(&du) {
                    let s1 = (du + 1).rem_euclid(4) < 2;
                    let s2 = (du + 2).rem_euclid(4) < 2;
                    pb.plot(px, sy + 1, if s1 { stripe_a } else { stripe_b });
                    pb.plot(px, sy + 2, if s2 { scale_c(stripe_a, 0.80) } else { scale_c(stripe_b, 0.78) });
                }
            }
        }
        Theme::Printworks => {
            // Cast-iron press lintel with twin pulsing cyan registration lamps & brass quoins
            let pulse = 0.60 + 0.40 * (view.cell.anim_clock * 3.4).sin();
            let lamp = scale_c((98, 206, 212), 0.72 + 0.28 * pulse);
            for u in u0..u1 {
                let du = u + du_off;
                let px = sx + u;
                if du == 3 || du == 28 {
                    pb.plot(px, sy + 1, lamp);
                    pb.plot(px, sy + 2, pal.detail_b);
                } else if du == 4 || du == 27 {
                    pb.plot(px, sy + 1, pal.outline);
                    pb.plot(px, sy + 2, pal.outline);
                } else if (6..=25).contains(&du) && (du % 4) == 0 {
                    pb.plot(px, sy + 1, PRINTWORKS_LUT.ream_hi);
                    pb.plot(px, sy + 2, PRINTWORKS_LUT.ink_stripe);
                }
            }
        }
        _ => {}
    }
}

/// **Laundry boss threshold**: wet indigo & porcelain tile apron with a central
/// slotted brass floor drain and pink ruined-wash drips leading into The Mangle.
#[allow(clippy::too_many_arguments)]
fn draw_boss_threshold_laundry(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    inward: Cardinal,
    u0: i32,
    u1: i32,
    du_off: i32,
    pal: &WallPalette,
) {
    let porc: Rgb = (146, 166, 166);
    let denim: Rgb = (54, 78, 96);
    let throat: Rgb = (14, 18, 30);
    let pink: Rgb = (168, 98, 128);
    let (v0, v1) = match inward {
        Cardinal::South => (6, 15),
        Cardinal::North => (1, 10),
        Cardinal::East | Cardinal::West => (4, 13),
    };
    for u in u0..u1 {
        let du = u + du_off;
        for v in v0..v1 {
            let lv = v - v0;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let in_drain = (9..=22).contains(&du) && (2..=6).contains(&lv);
            let c = if in_drain {
                if lv == 2 || lv == 6 || du == 9 || du == 22 {
                    LAUNDRY_LUT.brass_hi
                } else if (du % 3) == 0 {
                    LAUNDRY_LUT.brass_lo
                } else {
                    throat
                }
            } else if ((du / 4) + (lv / 3)) & 1 == 0 {
                porc
            } else {
                denim
            };
            let a = if lv == 0 || lv == v1 - v0 - 1 { 0.52 } else { 0.88 };
            blend_px(pb, px, py, c, a);
        }
        // Stray pink-wash puddle streak crossing the threshold edge.
        if (12..=19).contains(&du) && (du & 1) == 0 {
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v1 + 1);
            blend_px(pb, px, py, pink, 0.42);
        }
    }
    let _ = pal;
}

/// **Printworks boss threshold**: printed broadsheet proof-sheet apron with a
/// clean unprinted blank margin stripe, dark inked column rules and cyan
/// corner registration marks foreshadowing The Night Edition.
#[allow(clippy::too_many_arguments)]
fn draw_boss_threshold_printworks(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    inward: Cardinal,
    u0: i32,
    u1: i32,
    du_off: i32,
    anim_clock: f32,
    pal: &WallPalette,
) {
    let paper: Rgb = (138, 136, 124);
    let margin: Rgb = (164, 162, 148);
    let ink: Rgb = (58, 42, 72);
    let pulse = 0.65 + 0.35 * (anim_clock * 2.4).sin();
    let cyan = scale_c(pal.detail_b, 0.75 + 0.35 * pulse);
    let (v0, v1) = match inward {
        Cardinal::South => (6, 16),
        Cardinal::North => (1, 11),
        Cardinal::East | Cardinal::West => (3, 13),
    };
    for u in u0..u1 {
        let du = u + du_off;
        if !(3..=28).contains(&du) {
            continue;
        }
        for v in v0..v1 {
            let lv = v - v0;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let corner_crop = ((du == 3 || du == 4 || du == 27 || du == 28) && (lv == 0 || lv == v1 - v0 - 1))
                || ((du == 3 || du == 28) && (lv == 1 || lv == v1 - v0 - 2));
            let c = if corner_crop {
                cyan
            } else if (21..=26).contains(&du) {
                // Blank margin stripe (visual foreshadow of Night Edition's safe margin!)
                margin
            } else if lv == 2 && (6..=19).contains(&du) {
                ink
            } else if (lv == 4 || lv == 6) && (6..=18).contains(&du) && du != 12 {
                mix_c(paper, ink, 0.65)
            } else {
                paper
            };
            let a = if lv == 0 || lv == v1 - v0 - 1 { 0.60 } else { 0.86 };
            blend_px(pb, px, py, c, a);
        }
    }
}

/// **Sewer boss threshold**: weathered pale-limestone den threshold and cracked
/// approach flagstones deeply slashed by three bold, curved jaguar claw gouges
/// with high-contrast bone-pale chipped edges, mossy crevices, and den-spoor flecks.
#[allow(clippy::too_many_arguments)]
fn draw_boss_threshold_sewer(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    inward: Cardinal,
    u0: i32,
    u1: i32,
    du_off: i32,
    seed: u32,
    pal: &WallPalette,
) {
    // Pale weathered limestone-slate contrasts sharply against dark damp sewer
    // walls so the jaguar's den mouth reads cleanly from across the room.
    let stone_base: Rgb = (90, 108, 96);
    let stone_alt: Rgb = (74, 92, 82);
    let stone_hi: Rgb = (144, 168, 152);
    let stone_lo: Rgb = (44, 60, 54);
    let apron_stone: Rgb = (62, 78, 70);
    let chip_hi: Rgb = (182, 204, 186);
    let claw_core: Rgb = (7, 11, 10);
    let claw_wall: Rgb = (22, 34, 30);
    let claw_rust: Rgb = (156, 94, 52);
    let moss_hi: Rgb = (74, 128, 54);
    let moss_lo: Rgb = (42, 78, 36);

    // Clear the 4 px overhead lintel (`v in 0..4`) on North walls (`inward == South`)
    // and extend the approach flagstones slightly ahead of the mouth.
    let (apron_v0, apron_v1, slab_v0, slab_v1) = if inward == Cardinal::South {
        (-6, 0, 5, 16)
    } else {
        (-4, 3, 4, 15)
    };

    // 1. Approach fractured limestone apron + heavy carved den sill + interior step
    for u in u0..u1 {
        let du = u + du_off;

        // Fractured pale-stone approach apron (gives high-contrast backing to approach claw rakes!)
        if (4..=27).contains(&du) {
            for v in apron_v0..apron_v1 {
                let h = hash3(du, v, seed ^ 0xA910);
                // Organic broken/chipped outer stone edge
                let dist_edge = v - apron_v0;
                if dist_edge == 0 && (!(7..=24).contains(&du) || (h & 3) == 0) {
                    continue;
                }
                if (du == 4 || du == 27) && dist_edge <= 1 {
                    continue;
                }
                let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
                let c = if dist_edge == 0 {
                    stone_lo
                } else if du == 11 || du == 21 {
                    pal.mortar
                } else if (h & 3) == 0 {
                    stone_alt
                } else {
                    apron_stone
                };
                let alpha = if dist_edge == 0 { 0.56 } else { 0.84 };
                blend_px(pb, px, py, c, alpha);
            }
        }

        // Cast shadows framing the raised threshold slab
        for (v_sh, f) in [(slab_v0 - 1, 0.62), (slab_v1, 0.54), (slab_v1 + 1, 0.76)] {
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v_sh);
            darken_px(pb, px, py, f);
        }

        // Three thick ancient limestone flagstones (joints at du == 10 and du == 22)
        let slab_id = if du < 10 {
            0
        } else if du < 22 {
            1
        } else {
            2
        };
        let is_joint = du == 10 || du == 22;

        for v in slab_v0..slab_v1 {
            let lv = v - slab_v0; // 0..11
            // Chipped stone notches where the 3 claws bit into the threshold lip
            if lv == 0 && (du == 9 || du == 15 || du == 21) {
                continue;
            }
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let h = hash3(du, v, seed ^ 0xB055);
            let mut c = if lv == 0 || lv == 10 {
                pal.outline
            } else if lv == 1 {
                stone_hi
            } else if lv == 9 {
                stone_lo
            } else if is_joint && (2..=8).contains(&lv) {
                pal.mortar
            } else if (slab_id & 1) == 0 {
                if (h & 7) == 0 {
                    stone_alt
                } else {
                    stone_base
                }
            } else if (h & 5) == 0 {
                stone_base
            } else {
                stone_alt
            };

            if (du == 11 || du == 23) && (2..=8).contains(&lv) {
                c = mix_c(c, stone_hi, 0.28);
            }
            // Lush damp moss hugging the jamb edges of the stone den sill
            if (du <= 5 || du >= 26) && (1..=9).contains(&lv) && (h & 3) != 0 {
                let mc = if (h & 4) == 0 { moss_hi } else { moss_lo };
                c = mix_c(c, mc, if du <= 3 || du >= 28 { 0.72 } else { 0.42 });
            }
            blend_px(pb, px, py, c, 0.96);
        }

        // Weathered arena-side cracked stone step leading inward
        if (5..=26).contains(&du) {
            for dv in 1..=4 {
                let v = slab_v1 + dv;
                let h = hash3(du, v, seed ^ 0x5E0E);
                if (du == 5 || du == 26) && dv > 2 {
                    continue;
                }
                let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
                let c = if dv == 4 || (du == 16 && dv < 4) {
                    pal.mortar
                } else if (h & 3) == 0 {
                    stone_alt
                } else {
                    stone_lo
                };
                let alpha = if dv <= 2 { 0.72 } else { 0.42 };
                blend_px(pb, px, py, c, alpha);
            }
        }
    }

    // 2. Three bold, widely spaced diagonal Jaguar claw gouges slashing across
    //    the limestone sill and out onto the approach apron! Wide spacing (6 px)
    //    keeps every gouge unmistakably legible at 1× playing resolution.
    let v_shift = slab_v0 - 4;
    const PRIMARY_CLAWS: [(i32, i32, i32); 3] = [
        (9, -3, 16),  // left claw
        (15, -5, 19), // center main claw (longest, deepest)
        (21, -3, 17), // right claw
    ];
    let mid_v = (slab_v0 + slab_v1) / 2;
    for &(base_du, v_min_rel, v_max_rel) in &PRIMARY_CLAWS {
        let v_min = v_min_rel + v_shift;
        let v_max = v_max_rel + v_shift;
        for v in v_min..=v_max {
            let dv = v - mid_v;
            let curve = (dv * dv) / 52;
            let gouge_du = base_du + (dv * 6) / 11 + curve;
            let u = gouge_du - du_off;
            if u < u0 || u >= u1 {
                continue;
            }
            let in_slab = v > slab_v0 && v < slab_v1 - 1;
            let deep_core = v >= slab_v0 + 2 && v <= slab_v1 - 3;
            let in_apron = v >= apron_v0 && v < apron_v1;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);

            if in_slab {
                pb.plot(px, py, claw_core);
                // Bold bone-pale chipped limestone highlight on the upper-left gouge rim
                if u > u0 {
                    let (hx, hy) = boss_uv_to_screen(sx, sy, inward, u - 1, v);
                    blend_px(pb, hx, hy, chip_hi, if deep_core { 0.88 } else { 0.56 });
                }
                // Deepened dark shadow wall on the lower-right side of the gouge
                if deep_core && u + 1 < u1 {
                    let (sx2, sy2) = boss_uv_to_screen(sx, sy, inward, u + 1, v);
                    blend_px(pb, sx2, sy2, claw_wall, 0.84);
                }
            } else if in_apron {
                // High-contrast dark gouge raking across the fractured approach stone
                pb.plot(px, py, claw_core);
                if u > u0 {
                    let (hx, hy) = boss_uv_to_screen(sx, sy, inward, u - 1, v);
                    blend_px(pb, hx, hy, chip_hi, 0.62);
                }
            } else {
                let dist_end = (v - v_min).min(v_max - v);
                let alpha = if dist_end == 0 { 0.45 } else { 0.78 };
                blend_px(pb, px, py, claw_core, alpha);
            }
        }
    }

    // 3. Chipped pale limestone shards & warm amber-ochre den spoor flecks
    const FLECKS: [(i32, i32, bool); 8] = [
        (7, -2, false),
        (11, 0, true),
        (14, -4, false),
        (18, 1, true),
        (22, -2, false),
        (16, 17, true),
        (10, 16, false),
        (24, 14, true),
    ];
    for &(f_du, fv_rel, rust) in &FLECKS {
        let u = f_du - du_off;
        if u >= u0 && u < u1 {
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, fv_rel + v_shift);
            blend_px(pb, px, py, if rust { claw_rust } else { chip_hi }, 0.78);
        }
    }
}

/// **Kitchen boss threshold**: frayed woven coir & oxblood-bordered kitchen
/// doormat with a deep scuffed slipper-wear hollow and dog-eared corner, paired
/// with a scuffed vintage checker-lino transition strip, tarnished brass binder
/// bar, and stray golden corn-straw broom bristles.
#[allow(clippy::too_many_arguments)]
fn draw_boss_threshold_kitchen(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    inward: Cardinal,
    u0: i32,
    u1: i32,
    du_off: i32,
    pal: &WallPalette,
) {
    let coir_hi: Rgb = (166, 122, 74);
    let coir_mid: Rgb = (138, 96, 56);
    let coir_lo: Rgb = (106, 70, 38);
    let coir_hem: Rgb = (64, 40, 24);
    let coir_worn: Rgb = (66, 44, 28);
    let oxblood: Rgb = (118, 44, 38);
    let oxblood_hi: Rgb = (148, 60, 50);
    let lino_cream: Rgb = (172, 152, 122);
    let lino_tan: Rgb = (122, 98, 72);
    let lino_scuff: Rgb = (56, 38, 28);
    let brass_hi: Rgb = (232, 198, 120);
    let brass_mid: Rgb = (186, 150, 80);
    let straw: Rgb = (218, 180, 88);

    // Keep both the doormat AND the tarnished brass binder strip 100% clear of
    // the 4 px overhead lintel (`ly in 0..4`) on horizontal North & South walls!
    let (mat_v0, mat_v1, binder_v, lino_v0, lino_v1) = match inward {
        Cardinal::South => (-9, 0, 5, 7, 16),
        Cardinal::North => (-6, 7, 8, 10, 20),
        Cardinal::East | Cardinal::West => (-4, 10, 11, 13, 19),
    };
    let mat_cy = (mat_v0 + mat_v1) as f32 * 0.5;
    let mat_ry = ((mat_v1 - mat_v0) as f32 * 0.34).max(2.6);

    for u in u0..u1 {
        let du = u + du_off;

        // 1. Scuffed vintage checker-lino transition edge on the arena side
        if (2..=29).contains(&du) {
            for v in lino_v0..lino_v1 {
                let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
                let lv = v - lino_v0;
                let checker = ((du / 4) + (lv / 3)) & 1;
                let mut c = if checker == 0 { lino_cream } else { lino_tan };
                if v == lino_v1 - 1 || (du & 3) == 0 {
                    c = mix_c(c, pal.mortar, 0.48);
                }
                let scuff = (lv == 2 && (9..=15).contains(&du))
                    || (lv == 3 && (17..=23).contains(&du))
                    || (lv == 1 && (13..=17).contains(&du))
                    || (lv == 4 && (11..=14).contains(&du));
                if scuff {
                    c = mix_c(c, lino_scuff, 0.64);
                }
                let dist_far = lino_v1 - 1 - v;
                let alpha = if dist_far >= 2 {
                    0.90
                } else if dist_far == 1 {
                    0.70
                } else {
                    0.46
                };
                blend_px(pb, px, py, c, alpha);
            }
        }

        // 2. Tarnished brass carpet-to-lino binder strip with screw heads
        for dv in 0..=1 {
            let v = binder_v + dv;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let is_screw = dv == 0 && (du == 4 || du == 10 || du == 16 || du == 22 || du == 28);
            let c = if is_screw {
                pal.outline
            } else if dv == 0 {
                brass_hi
            } else {
                brass_mid
            };
            blend_px(pb, px, py, c, 0.96);
        }
        let (bx, by) = boss_uv_to_screen(sx, sy, inward, u, binder_v + 2);
        darken_px(pb, bx, by, 0.70);

        // 3. Worn woven coir & oxblood doormat (`du in 4..28`, `v in mat_v0..=mat_v1`)
        if (3..=28).contains(&du) {
            let (sx_top, sy_top) = boss_uv_to_screen(sx, sy, inward, u, mat_v0 - 1);
            darken_px(pb, sx_top, sy_top, 0.68);
        }
        if du == 3 || du == 28 {
            for v in mat_v0..=mat_v1 {
                let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
                darken_px(pb, px, py, 0.62);
                if (v - mat_v0) % 4 == 2 {
                    blend_px(pb, px, py, coir_lo, 0.78);
                }
            }
        }
        if (4..=27).contains(&du) {
            if du == 6 || du == 11 || du == 16 || du == 21 || du == 25 {
                let (fx, fy) = boss_uv_to_screen(sx, sy, inward, u, mat_v0 - 1);
                blend_px(pb, fx, fy, coir_mid, 0.80);
            }
            for v in mat_v0..=mat_v1 {
                // Dog-eared / curled approach corner casting a crisp shadow
                if du == 27 && v == mat_v0 {
                    let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
                    darken_px(pb, px, py, 0.54);
                    continue;
                }
                let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
                if (du == 26 && v == mat_v0) || (du == 27 && v == mat_v0 + 1) {
                    pb.plot(px, py, coir_hi);
                    continue;
                }

                let on_outer_rim = du == 4 || du == 27 || v == mat_v0 || v == mat_v1;
                let on_oxblood_stripe = !on_outer_rim
                    && (du == 6 || du == 25 || v == mat_v0 + 2 || v == mat_v1 - 2)
                    && (6..=25).contains(&du)
                    && (v >= mat_v0 + 2 && v <= mat_v1 - 2);

                let mut c = if on_outer_rim {
                    if v == mat_v0 && (du & 1) == 0 {
                        coir_lo
                    } else {
                        coir_hem
                    }
                } else if on_oxblood_stripe {
                    if ((du + v) & 1) == 0 {
                        oxblood_hi
                    } else {
                        oxblood
                    }
                } else {
                    let weave = ((du + (v & 1)) & 3) == 0;
                    let alt = ((du ^ v) & 1) == 0;
                    if weave {
                        coir_lo
                    } else if alt {
                        coir_hi
                    } else {
                        coir_mid
                    }
                };

                // Deep worn heel-pivot hollow in the center of the doormat
                let dx = (du - 16) as f32 / 7.2;
                let dy = (v as f32 - mat_cy) / mat_ry;
                let d2 = dx * dx + dy * dy;
                if !on_outer_rim && d2 < 1.0 {
                    let wear = (1.0 - d2) * 0.78;
                    c = mix_c(c, coir_worn, wear);
                    if ((du - v) & 3) == 0 && d2 < 0.48 {
                        c = mix_c(c, lino_cream, 0.28);
                    }
                }
                if v == mat_v0 + 1 && (5..=25).contains(&du) {
                    c = mix_c(c, coir_hi, 0.28);
                }
                pb.plot(px, py, c);
            }
        }
    }

    // 4. Stray golden corn-straw broom bristles tucked beside the jamb corners
    let straw_v_base = match inward {
        Cardinal::South => 8,
        Cardinal::North => 2,
        _ => 4,
    };
    const STRAWS: [(i32, i32); 6] = [(3, 0), (3, 1), (2, 2), (28, -1), (29, 0), (28, 3)];
    for &(s_du, sv) in &STRAWS {
        let u = s_du - du_off;
        if u >= u0 && u < u1 {
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, straw_v_base + sv);
            blend_px(pb, px, py, straw, 0.88);
        }
    }
}

/// **Lab boss threshold**: bold inward-pointing amber/charcoal hazard chevrons on
/// the approach apron, steel containment sill with mower tire skids & blade scratches,
/// and an asymmetric mown, scorched botanical test-lawn strip on the arena side.
#[allow(clippy::too_many_arguments)]
fn draw_boss_threshold_lab(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    inward: Cardinal,
    u0: i32,
    u1: i32,
    du_off: i32,
    anim_clock: f32,
    pal: &WallPalette,
    accent: Rgb,
) {
    let haz_hi: Rgb = (234, 182, 54);
    let haz_mid: Rgb = (198, 148, 42);
    let haz_dk: Rgb = (18, 22, 32);
    let steel_hi: Rgb = (106, 126, 156);
    let steel_mid: Rgb = (52, 64, 88);
    let steel_lo: Rgb = (32, 40, 58);
    let rubber: Rgb = (16, 18, 26);
    let scratch_hi: Rgb = (190, 218, 236);
    let grass_dk: Rgb = (36, 84, 40);
    let grass_mid: Rgb = (58, 130, 52);
    let grass_hi: Rgb = (96, 178, 70);
    let grass_fresh: Rgb = (122, 198, 76);
    let scorch_edge: Rgb = (102, 72, 36);
    let scorch_mid: Rgb = (44, 30, 22);
    let scorch_char: Rgb = (20, 16, 16);
    let ember: Rgb = (176, 82, 36);

    let pulse = 0.55 + 0.45 * (anim_clock * 4.2).sin();
    let beacon = mix_c((228, 76, 44), accent, 0.14 * pulse);

    // Keep approach hazard chevrons, steel sill, and scorched lawn-strip clear of
    // the 4 px overhead lintel (`ly in 0..4`) on both North and South walls.
    let (haz_v0, haz_v1, sill_v0, sill_v1, lawn_v0, lawn_v1_base) = match inward {
        Cardinal::South => (-8, 0, 4, 13, 13, 20),
        Cardinal::North => (-6, 3, 3, 12, 15, 22),
        Cardinal::East | Cardinal::West => (-5, 4, 4, 13, 13, 20),
    };

    for u in u0..u1 {
        let du = u + du_off;
        let dist_c = (du - 16).abs();
        let on_tire_l = (7..=10).contains(&du);
        let on_tire_r = (21..=24).contains(&du);

        let (bx_a, by_a) = boss_uv_to_screen(sx, sy, inward, u, haz_v0 - 1);
        darken_px(pb, bx_a, by_a, 0.72);

        // 1. Approach apron: bolted steel frame + inward-pointing hazard chevrons
        for v in haz_v0..haz_v1 {
            let lv = v - haz_v0;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let mut c = if v == haz_v0 || v == haz_v1 - 1 {
                pal.outline
            } else if du == 2 || du == 29 {
                steel_lo
            } else if du == 3 || du == 28 {
                if lv == 3 || lv == 4 {
                    scale_c(beacon, 0.65 + 0.35 * pulse)
                } else {
                    steel_mid
                }
            } else {
                let phase = (dist_c - lv).rem_euclid(6);
                if phase < 3 {
                    if lv == 1 || phase == 0 {
                        haz_hi
                    } else {
                        haz_mid
                    }
                } else {
                    haz_dk
                }
            };
            if (on_tire_l || on_tire_r) && v >= haz_v1 - 3 && v < haz_v1 - 1 {
                c = mix_c(c, rubber, 0.42);
            }
            blend_px(pb, px, py, c, 0.94);
        }

        // 2. Heavy steel containment sill with mower tire skids & blade scratches
        for v in sill_v0..sill_v1 {
            let lv = v - sill_v0;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let mut c = if v == sill_v0 || v == sill_v1 - 1 {
                steel_hi
            } else if lv == 4 {
                pal.outline
            } else if (v & 1) == 0 {
                steel_mid
            } else {
                steel_lo
            };

            if (on_tire_l || on_tire_r) && v > sill_v0 && v < sill_v1 - 1 {
                let tread = ((du + v) & 1) == 0;
                c = mix_c(c, rubber, if tread { 0.82 } else { 0.50 });
            }
            if (lv == 2 && (12..=15).contains(&du))
                || (lv == 3 && (15..=19).contains(&du))
                || (lv == 6 && (13..=17).contains(&du))
            {
                c = mix_c(c, scratch_hi, 0.76);
            }
            blend_px(pb, px, py, c, 0.95);
        }

        // 3. Arena-side scorched botanical test-lawn strip
        let h_col = hash3(du, 7, 0x1A00);
        let tip_v = lawn_v1_base + (h_col & 1) as i32;
        for v in lawn_v0..=tip_v {
            let lv = v - lawn_v0;
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, v);
            let h = hash3(du, v, 0x92A5);
            let scorch_center = 15 + lv / 3;
            let sd = (du - scorch_center).abs();
            let c = if lv == 0 {
                pal.outline
            } else if sd <= 4 {
                if sd <= 2 && (2..=5).contains(&lv) {
                    if (du == 14 && lv == 3) || (du == 17 && lv == 4) || (du == 15 && lv == 5) {
                        ember
                    } else {
                        scorch_char
                    }
                } else if (h & 3) == 0 {
                    scorch_edge
                } else {
                    scorch_mid
                }
            } else if sd == 5 || sd == 6 {
                if sd == 6 && (h & 1) == 0 && v < tip_v {
                    grass_fresh
                } else if (h & 2) == 0 {
                    scorch_edge
                } else {
                    grass_dk
                }
            } else {
                let mut g = {
                    let blade = (du + v * 3 + (h as i32)) & 3;
                    if v == tip_v || blade == 0 {
                        grass_dk
                    } else if blade == 1 {
                        grass_hi
                    } else {
                        grass_mid
                    }
                };
                if (on_tire_l || on_tire_r) && lv <= 3 {
                    g = mix_c(g, scorch_mid, 0.56);
                }
                g
            };
            let alpha = if v == tip_v { 0.78 } else { 0.94 };
            blend_px(pb, px, py, c, alpha);
        }
        let (sx_t, sy_t) = boss_uv_to_screen(sx, sy, inward, u, tip_v + 1);
        darken_px(pb, sx_t, sy_t, 0.76);
    }

    // 4. Scattered bright-green cut grass clippings thrown back onto the steel sill
    const CLIPPINGS: [(i32, i32); 7] = [
        (6, 6),
        (11, 7),
        (14, 5),
        (19, 7),
        (23, 6),
        (26, 5),
        (12, 8),
    ];
    for &(c_du, cv_rel) in &CLIPPINGS {
        let u = c_du - du_off;
        if u >= u0 && u < u1 {
            let (px, py) = boss_uv_to_screen(sx, sy, inward, u, sill_v0 + cv_rel);
            blend_px(pb, px, py, grass_fresh, 0.86);
        }
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

    fn fnv1a_pixels(pb: &PixelBuffer) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let (r, g, b) = pb.pixel_at(x, y);
                for byte in [r, g, b] {
                    h ^= byte as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        h
    }

    fn render_sample_map(theme: Theme) -> PixelBuffer {
        let cols = SAMPLE_MAP[0].len();
        let rows = SAMPLE_MAP.len();
        let w = cols * CELL_SIZE as usize;
        let h = rows * CELL_SIZE as usize;
        let accent = crate::game::level::Palette::for_theme(theme).accent;
        let cam = Vec2::new(64.0, 64.0);
        let cx_off = 4;
        let cy_off = 4;
        let mut pb = PixelBuffer::new(w, h);
        pb.clear(crate::game::level::Palette::for_theme(theme).bg);

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
        for v in &views {
            draw_wall_ground(&mut pb, v, cam, accent);
        }
        for v in &views {
            draw_wall_cell(&mut pb, v, cam, accent);
        }
        for v in &views {
            draw_wall_overhang(&mut pb, v, cam, accent);
        }
        pb
    }

    /// Pins the exact FNV-1a pixel hash of the rendered 18 × 12 sample layout
    /// (covering Solid, Maze, RatHole H/V, Doorway H/V, OneWayDoor W/S/E,
    /// shadows, and overhangs) for every theme so visual changes are deliberate.
    #[test]
    fn sample_render_hashes_are_pinned() {
        const EXPECTED: [(Theme, u64); 3] = [
            (Theme::Sewer, 0xa556_4534_949c_5d82),
            (Theme::Kitchen, 0xcc14_6036_0100_16a7),
            (Theme::Lab, 0x1a39_d95b_f07f_e784),
        ];
        for (theme, expected) in EXPECTED {
            let pb = render_sample_map(theme);
            let actual = fnv1a_pixels(&pb);
            assert_eq!(
                actual, expected,
                "{theme:?} wall render hash drifted: got 0x{actual:016x}, expected 0x{expected:016x}"
            );
        }
    }

    /// Draws a screenful (10 × 11 = 110 cells) of each theme across many
    /// interleaved rounds (balancing CPU turbo / contention) and asserts that
    /// Lab stays well under 2× Sewer's draw time.
    #[test]
    fn lab_wall_perf_within_ratio_of_sewer() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        let mut pb = PixelBuffer::new(160, 168);
        let cam = Vec2::new(64.0, 64.0);
        let rounds = 40;
        let frames_per_round = 10;

        let build_screenful = |theme: Theme, interior_only: bool| -> (Rgb, Vec<WallCellView>) {
            let accent = crate::game::level::Palette::for_theme(theme).accent;
            let mut views = Vec::with_capacity(110);
            for cy in 4..15 {
                for cx in 4..14 {
                    let mask = if interior_only {
                        NeighborMask::ALL
                    } else if cy == 14 {
                        NeighborMask::from_cardinals(true, true, false, true)
                    } else if cx == 4 || cx == 13 || cy == 4 {
                        NeighborMask::from_cardinals(cy > 4, cx < 13, cy < 14, cx > 4)
                    } else {
                        NeighborMask::ALL
                    };
                    views.push(WallCellView {
                        cell_x: cx,
                        cell_y: cy,
                        seed: 1337,
                        theme,
                        kind: WallKind::Solid,
                        mask,
                        open_progress: 0.0,
                        anim_clock: 1.5,
                    });
                }
            }
            (accent, views)
        };

        for (label, interior_only) in [("solid_bedrock", true), ("mixed_screenful", false)] {
            let cases = [
                build_screenful(Theme::Sewer, interior_only),
                build_screenful(Theme::Kitchen, interior_only),
                build_screenful(Theme::Lab, interior_only),
            ];
            // Warmup all three themes
            for (accent, views) in &cases {
                for v in views {
                    draw_wall_ground(&mut pb, v, cam, *accent);
                    draw_wall_cell(&mut pb, v, cam, *accent);
                    draw_wall_overhang(&mut pb, v, cam, *accent);
                }
            }

            let mut totals = [Duration::ZERO; 3];
            for r in 0..rounds {
                // Rotate theme order each round so transient OS scheduling noise cancels out
                for step in 0..3 {
                    let ti = (r + step) % 3;
                    let (accent, views) = &cases[ti];
                    let t0 = Instant::now();
                    for _ in 0..frames_per_round {
                        for v in views {
                            draw_wall_ground(&mut pb, v, cam, *accent);
                        }
                        for v in views {
                            draw_wall_cell(&mut pb, v, cam, *accent);
                        }
                        for v in views {
                            draw_wall_overhang(&mut pb, v, cam, *accent);
                        }
                    }
                    totals[ti] += t0.elapsed();
                    black_box(pb.pixel_at(80, 84));
                }
            }

            let total_frames = (rounds * frames_per_round) as f64;
            let sewer_us = totals[0].as_secs_f64() * 1e6 / total_frames;
            let kitchen_us = totals[1].as_secs_f64() * 1e6 / total_frames;
            let lab_us = totals[2].as_secs_f64() * 1e6 / total_frames;
            let ratio = lab_us / sewer_us.max(0.001);
            eprintln!(
                "wall perf ({label}, {total_frames:.0} frames × 110 cells): \
                 Sewer={sewer_us:.1}us Kitchen={kitchen_us:.1}us Lab={lab_us:.1}us \
                 Lab/Sewer={ratio:.2}x"
            );
            assert!(
                ratio < 2.0,
                "{label}: Lab ({lab_us:.1}us) / Sewer ({sewer_us:.1}us) = {ratio:.2}x >= 2.0"
            );
        }
    }

    #[test]
    fn boss_threshold_never_changes_collision_and_paints_each_theme() {
        let mut buf = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
        let mut pb = PixelBuffer::new(96, 96);
        let cam = Vec2::new(0.0, 0.0);

        // Collision on the underlying 2-cell doorway remains strictly 1 px outer jambs (30 px clear)
        let left_cell = WallCellView::new(
            2,
            2,
            42,
            Theme::Sewer,
            WallKind::Doorway {
                axis: WallAxis::Horizontal,
            },
            NeighborMask::from_cardinals(false, false, false, true),
        );
        let right_cell = WallCellView::new(
            3,
            2,
            42,
            Theme::Sewer,
            WallKind::Doorway {
                axis: WallAxis::Horizontal,
            },
            NeighborMask::from_cardinals(false, true, false, false),
        );
        assert_eq!(wall_colliders(&left_cell, &mut buf), 1);
        let left_edge = buf[0].right();
        assert_eq!(wall_colliders(&right_cell, &mut buf), 1);
        let right_edge = buf[0].x;
        assert_eq!(right_edge - left_edge, 30.0);

        // Render every theme × all 4 inward directions × edge cameras with zero panics
        let mut theme_hashes = [0u64; 5];
        for (ti, theme) in Theme::all().into_iter().enumerate() {
            let accent = crate::game::level::Palette::for_theme(theme).accent;
            let bg = crate::game::level::Palette::for_theme(theme).bg;

            for &inward in &[
                Cardinal::North,
                Cardinal::South,
                Cardinal::East,
                Cardinal::West,
            ] {
                let (m0, m1, c1_dx, c1_dy) = match inward.wall_axis() {
                    WallAxis::Horizontal => (
                        NeighborMask::from_cardinals(false, false, false, true),
                        NeighborMask::from_cardinals(false, true, false, false),
                        1,
                        0,
                    ),
                    WallAxis::Vertical => (
                        NeighborMask::from_cardinals(true, false, false, false),
                        NeighborMask::from_cardinals(false, false, true, false),
                        0,
                        1,
                    ),
                };
                for &test_cam in &[
                    cam,
                    Vec2::new(-40.0, -40.0),
                    Vec2::new(400.0, 400.0),
                    Vec2::new(28.0, 28.0),
                ] {
                    let v0 = BossThresholdView::new(
                        WallCellView {
                            cell_x: 2,
                            cell_y: 2,
                            seed: 1337,
                            theme,
                            kind: WallKind::Doorway {
                                axis: inward.wall_axis(),
                            },
                            mask: m0,
                            open_progress: 0.0,
                            anim_clock: 1.5,
                        },
                        inward,
                    );
                    let v1 = BossThresholdView::new(
                        WallCellView {
                            cell_x: 2 + c1_dx,
                            cell_y: 2 + c1_dy,
                            seed: 1337,
                            theme,
                            kind: WallKind::Doorway {
                                axis: inward.wall_axis(),
                            },
                            mask: m1,
                            open_progress: 0.0,
                            anim_clock: 1.5,
                        },
                        inward,
                    );
                    pb.clear(bg);
                    draw_wall_ground(&mut pb, &v0.cell, test_cam, accent);
                    draw_wall_ground(&mut pb, &v1.cell, test_cam, accent);
                    let before = fnv1a_pixels(&pb);
                    draw_boss_threshold_ground(&mut pb, &v0, test_cam, accent);
                    draw_boss_threshold_ground(&mut pb, &v1, test_cam, accent);
                    draw_wall_cell(&mut pb, &v0.cell, test_cam, accent);
                    draw_wall_cell(&mut pb, &v1.cell, test_cam, accent);
                    draw_wall_overhang(&mut pb, &v0.cell, test_cam, accent);
                    draw_wall_overhang(&mut pb, &v1.cell, test_cam, accent);
                    draw_boss_threshold_overhang(&mut pb, &v0, test_cam, accent);
                    draw_boss_threshold_overhang(&mut pb, &v1, test_cam, accent);

                    if test_cam == cam && inward == Cardinal::South {
                        let after = fnv1a_pixels(&pb);
                        assert_ne!(
                            before, after,
                            "{theme:?}: boss threshold must visibly change the doorway"
                        );
                        theme_hashes[ti] = after;
                    }
                }
            }
        }
        // Every floor's boss threshold is visibly its own (the wave 3 floors
        // added two more, so the pairwise check covers all five).
        for i in 0..theme_hashes.len() {
            for j in i + 1..theme_hashes.len() {
                assert_ne!(
                    theme_hashes[i], theme_hashes[j],
                    "boss thresholds {i} and {j} are identical"
                );
            }
        }
    }

    /// Visual sheet: `RATRUN_DUMP=boss_thresholds cargo test dump_boss_thresholds_sheet`
    /// writes `/tmp/ratrun_boss_thresholds.ppm` showing all 3 themes' boss arena
    /// double-doorway thresholds across all 4 wall orientations (South/North/East/West
    /// inward mouths), with rats placed for scale.
    #[test]
    fn dump_boss_thresholds_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("boss_thresholds") {
            return;
        }
        // 16 × 10 miniature boss-chamber showcase per theme:
        // - Top wall: 2-cell South-inward boss mouth at (7, 2) & (8, 2)
        // - Bottom wall: 2-cell North-inward boss mouth at (7, 7) & (8, 7)
        // - Left wall: 2-cell East-inward boss mouth at (4, 4) & (4, 5)
        // - Right wall: 2-cell West-inward boss mouth at (11, 4) & (11, 5)
        const ARENA_MAP: [&str; 10] = [
            "................",
            "................",
            "....###DD###....",
            "....#......#....",
            "....d......d....",
            "....d......d....",
            "....#......#....",
            "....###DD###....",
            "................",
            "................",
        ];

        let cols = ARENA_MAP[0].len();
        let rows = ARENA_MAP.len();
        let w = cols * CELL_SIZE as usize;
        let h = rows * CELL_SIZE as usize;
        let sep = 2usize;
        let total_h = h * 3 + sep * 2;
        let mut out = format!("P6\n{} {}\n255\n", w, total_h).into_bytes();

        let cell_at = |x: i32, y: i32| -> Option<WallKind> {
            if y < 0 || y >= rows as i32 || x < 0 || x >= cols as i32 {
                return None;
            }
            match ARENA_MAP[y as usize].as_bytes()[x as usize] {
                b'#' => Some(WallKind::Solid),
                b'D' => Some(WallKind::Doorway {
                    axis: WallAxis::Horizontal,
                }),
                b'd' => Some(WallKind::Doorway {
                    axis: WallAxis::Vertical,
                }),
                _ => None,
            }
        };
        let mask_at = |x: i32, y: i32, kind: WallKind| -> NeighborMask {
            let conn = |ox: i32, oy: i32| connects(kind, cell_at(ox, oy).map(|k| (k, 0.0)));
            NeighborMask::from_all(
                conn(x, y - 1),
                conn(x + 1, y),
                conn(x, y + 1),
                conn(x - 1, y),
                conn(x + 1, y - 1),
                conn(x + 1, y + 1),
                conn(x - 1, y + 1),
                conn(x - 1, y - 1),
            )
        };

        for (ti, theme) in Theme::all().into_iter().enumerate() {
            let level = Level::new(theme);
            let accent = level.palette.accent;
            let cam = Vec2::new(64.0, 64.0);
            let cx_off = 4;
            let cy_off = 4;
            let mut pb = PixelBuffer::new(w, h);
            draw_floor(&mut pb, &level, cam, 1.5);

            let mut views = Vec::new();
            let mut thresholds = Vec::new();
            for y in 0..rows as i32 {
                for x in 0..cols as i32 {
                    if let Some(kind) = cell_at(x, y) {
                        let v = WallCellView {
                            cell_x: x + cx_off,
                            cell_y: y + cy_off,
                            seed: 1337,
                            theme,
                            kind,
                            mask: mask_at(x, y, kind),
                            open_progress: 0.0,
                            anim_clock: 1.5,
                        };
                        if matches!(kind, WallKind::Doorway { .. }) {
                            let inward = if y == 2 {
                                Cardinal::South
                            } else if y == 7 {
                                Cardinal::North
                            } else if x == 4 {
                                Cardinal::East
                            } else {
                                Cardinal::West
                            };
                            thresholds.push(BossThresholdView::new(v, inward));
                        }
                        views.push(v);
                    }
                }
            }

            // Pass 1: Wall ground + Boss threshold ground overlays
            for v in &views {
                draw_wall_ground(&mut pb, v, cam, accent);
            }
            for t in &thresholds {
                draw_boss_threshold_ground(&mut pb, t, cam, accent);
            }
            // Pass 2: Wall bodies & doorway jambs
            for v in &views {
                draw_wall_cell(&mut pb, v, cam, accent);
            }
            // Rats approaching the North, East, and South boss thresholds for scale
            for (rx, ry, fx, fy) in [
                (8.0, 1.25, 0.0, 1.0),
                (3.0, 5.0, 1.0, 0.0),
                (8.0, 8.75, 0.0, -1.0),
            ] {
                let pos = cam + Vec2::new(rx * CELL_SIZE_F, ry * CELL_SIZE_F);
                let mut p = Player::new(pos, WeaponKind::Gnaw);
                p.face = Vec2::new(fx, fy).normalized();
                p.vel = p.face * 55.0;
                draw_rat(&mut pb, &p, 0.7, cam);
            }
            // Pass 3: Wall overhang + Boss threshold overhang trim
            for v in &views {
                draw_wall_overhang(&mut pb, v, cam, accent);
            }
            for t in &thresholds {
                draw_boss_threshold_overhang(&mut pb, t, cam, accent);
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
        std::fs::write("/tmp/ratrun_boss_thresholds.ppm", out).unwrap();
    }

    /// Shop hole rendered across all four themes (Sewer, Kitchen, Lab, Trailer).
    /// `RATRUN_DUMP=shop_holes cargo test dump_shop_holes_all_themes`.
    #[test]
    fn dump_shop_holes_all_themes() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("shop_holes") {
            return;
        }
        use crate::game::world::World;
        let themes = [Theme::Sewer, Theme::Kitchen, Theme::Lab, Theme::Trailer];
        let tile_w = 120usize;
        let tile_h = 84usize;
        let mut out = format!("P6\n{} {}\n255\n", tile_w * 2, tile_h * 2).into_bytes();
        let mut grid = vec![(0u8, 0u8, 0u8); tile_w * 2 * tile_h * 2];
        for (idx, theme) in themes.iter().enumerate() {
            let mut w = World::new(*theme, 1);
            w.elapsed = 3.2;
            if let Some(&hole) = w.level.objects.shop_holes.first() {
                w.player.pos = hole.face() + hole.facing * 20.0
                    + Vec2::new(-hole.facing.y, hole.facing.x) * 14.0;
            }
            let mut pb = PixelBuffer::new(tile_w, tile_h);
            w.draw(&mut pb);
            let ox = (idx % 2) * tile_w;
            let oy = (idx / 2) * tile_h;
            for y in 0..tile_h {
                for x in 0..tile_w {
                    grid[(oy + y) * (tile_w * 2) + (ox + x)] = pb.pixel_at(x, y);
                }
            }
        }
        for (r, g, b) in grid {
            out.extend_from_slice(&[r, g, b]);
        }
        let dir = std::env::var_os("RATRUN_DUMP_DIR")
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        std::fs::write(dir.join("ratrun_shop_holes_themes.ppm"), out).unwrap();
    }
}
