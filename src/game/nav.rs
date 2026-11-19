//! Flow-field pathfinding: one field per size class, fine near the player,
//! coarse further out, staggered across frames.
//!
//! For a horde all chasing one target, per-agent A* is wasteful: one **flow
//! field** per size class ([`SizeClass`]) is built backwards from the player's
//! cell (a BFS integration field plus one routing direction per node) and every
//! enemy samples it in O(1).
//!
//! # The map is 10× bigger
//!
//! A full-map recompute over the 200 × 160 cell grid costs **~0.9 ms** on this
//! machine in the dev profile (`nav_benchmark`'s baseline; the pre-split field
//! also carried escape-direction searches, which measured 2.6 ms) and used to
//! run every time the player crossed a cell boundary, ~4×/s — a visible hitch.
//! This module splits the work three ways:
//!
//! * **Near-fine.** A field with one node per tile inside
//!   [`FINE_RADIUS_CELLS`] cells of the player (49 × 49 cells for the default
//!   radius). That is where the horde lives: the director's spawn ring is a few
//!   cells out, well inside the window. Measured: **0.10 ms** for one class.
//! * **Far-coarse.** One node per [`COARSE_BLOCK_CELLS`] × [`COARSE_BLOCK_CELLS`]
//!   cell block (25 × 20 blocks) everywhere else. A block's target is the
//!   **gate** it leaves through (the midpoint of the opening on the shared
//!   boundary, or the neighbouring block's middle cell when an edge has no
//!   open pair), so [`FlowField::dir_at`] aims at a point instead of a compass
//!   direction. Block edges require a real gate of open cells on the boundary,
//!   so a coarse route never crosses a wall between two half-open blocks, and
//!   one-way door cells are not coarse gates at all (a small enemy would jam
//!   against the leaf). Measured: fine + coarse, both classes sharing, **0.22
//!   ms**; unshared, **0.38 ms**.
//! * **Staggering.** [`FlowField::update`] spends at most [`NavBudget`] work per
//!   call and picks up where it left off next frame. The previous field stays
//!   readable the whole time, so crossing a cell does not cost a multi-
//!   millisecond frame: a shared window completes ~3 frames after the player
//!   enters a new cell, worst single frame measured **0.16 ms**, amortised
//!   **0.013 ms** per frame over a 30 s walk.
//!
//! Two details keep bodies off wall faces, where a pure cell-step field strands
//! them:
//!
//! * [`FlowField::centreline`] bends every step toward the centre of the cell it
//!   enters, so a 14 px doorway or an 8 px rat-hole is approached down the
//!   middle instead of along a grid line that clips a jamb.
//! * A **diagonal** step needs both of its orthogonal neighbours open. That is
//!   stricter than [`super::levelgen::validate::reachable`]'s "either side"
//!   rule, which only proves that a *point* can reach a cell: a rat-sized circle
//!   clips the corner of a wall and wall resolution parks it there for good.
//!   Both the BFS and the direction pass use the same rule, so they cannot
//!   disagree; the cost is that a cell only reachable through a diagonal corner
//!   squeeze is not routed (it is not walkable either).
//! * [`FlowField::dir_at`] returns **a legal step or `ZERO`** — never a
//!   direction into a cell the body cannot step to. `ZERO` means "steer
//!   straight at the player", which is the only thing the caller can fall back
//!   on. This is a hard contract (asserted over every cell of every generated
//!   map by `dir_at_only_ever_points_at_a_legal_step`), because the failure it
//!   prevents is a body pinned flush against a wall: `Enemy::steer` pushes it
//!   into the face, wall resolution pushes it straight back out, and the
//!   position never changes. The coarse field therefore picks its step from the
//!   block-cost gradient and only uses its geometric aim point (the gate
//!   midpoint it routes through) as a tie-break.
//!
//! # Size classes and directed doors
//!
//! A [`super::art_walls::WallKind::RatHole`] cell is open for [`SizeClass::Small`]
//! only (radius ≤ 3.5 px: the rat, Bats, Skitterers, Spitters), so a Cat, a
//! Brute or a boss must take the long way round. Each class gets its own
//! passability mask, built once at level load from [`TileMap::passable`].
//!
//! A [`super::art_walls::WallKind::OneWayDoor`] cell is a **directed edge**:
//! the BFS-from-the-goal expands *into* a cell only when the reverse of that
//! edge is legal (see [`super::levelgen::validate::reachable`], the reference
//! implementation of the same rule), so an enemy can leave a maze but never
//! re-enter one.
//!
//! # Sharing between classes
//!
//! Rat-holes and one-way doors are rare, so the two passability masks are
//! usually identical over the fine window. When they are, the window is
//! computed **once** and both classes read that field ([`FlowField::shared`]);
//! when they differ (the player is near a rat-hole) each class gets its own.
//!
//! # Determinism
//!
//! Nothing here draws rng, allocates per call (every buffer and queue is
//! allocated in [`FlowField::new`] and reused), or iterates a hash map. Neighbour
//! expansion walks a table in a fixed order, so the same tiles + the same goal
//! cell give the same field on every run.

use std::collections::VecDeque;

use super::art_walls::Cardinal;
use super::collision::{cardinal_of, SizeClass, TileKind, TileMap, TILE_F};
use super::level::Rectf;
use crate::math::Vec2;

/// 8-connected neighborhood (orthogonal + diagonal), fixed order.
const OFFS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// Weight of the pull toward the centre of the next cell (see
/// [`FlowField::centreline`]): 1.0 would aim straight at the cell centre, which
/// is far too eager in the open; 0.75 bends a step enough to clear a jamb.
const CENTRE_PULL: f32 = 0.75;

/// Radius, in cells, of the fine window around the player's cell: 24 cells =
/// 384 px ≈ 2.4 screens of the 160 × 84 px viewport.
pub const FINE_RADIUS_CELLS: i32 = 24;
/// Side length of the coarse block, in cells (8 cells = 128 px).
pub const COARSE_BLOCK_CELLS: i32 = 8;

/// Side length of the fine window grid (49 × 49 cells for the default radius).
const FINE_SIDE: i32 = FINE_RADIUS_CELLS * 2 + 1;
/// Inside this Chebyshev radius the fine field alone steers; from here out to
/// [`FINE_RADIUS_CELLS`] it is blended into the coarse field so an enemy does
/// not snap between the two at the window edge.
const BLEND_INNER_CELLS: i32 = FINE_RADIUS_CELLS - 6;
/// Minimum open cells for a coarse block to count as traversable (of 64). A
/// one-cell-wide corridor crossing a block contributes 8, so narrow routes
/// survive; a block that is all wall contributes 0 and is an obstacle.
const BLOCK_MIN_OPEN: i32 = 8;

/// Work [`FlowField::update`] may spend in one call, in BFS/direction cells.
///
/// The default is sized so the worst case (an unshared window: both classes,
/// fine + coarse) stays well under 0.3 ms per frame in the dev profile; see
/// the `nav_benchmark` test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavBudget {
    /// Fine-window cells (BFS expansion or direction derivation) per class.
    pub fine_cells: u32,
    /// Coarse blocks (BFS expansion or target derivation) per class.
    pub coarse_blocks: u32,
}

impl NavBudget {
    /// Everything in one call (level load, tests).
    pub const FULL: NavBudget = NavBudget {
        fine_cells: u32::MAX,
        coarse_blocks: u32::MAX,
    };
}

impl Default for NavBudget {
    fn default() -> Self {
        NavBudget {
            fine_cells: 1024,
            coarse_blocks: 512,
        }
    }
}

/// What one [`FlowField::update`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavUpdate {
    /// The call finished a recompute toward a new goal cell.
    pub changed: bool,
    /// Fine-window cells spent (summed over both classes when unshared).
    pub fine_cells: u32,
    /// Coarse blocks spent (summed over both classes).
    pub coarse_blocks: u32,
    /// Both size classes read one shared field (see the module docs).
    pub shared: bool,
}

/// A 16 px navigation mask, kept from the pre-procedural-levels build so that
/// `Level::nav` (and the systems and tests that still read it) keep working.
///
/// [`FlowField`] no longer routes from it — it builds its own per-class masks
/// from the [`TileMap`] — but it remains the single-class (large) occupancy
/// bitmap that `Level` exposes while the tile-collision migration finishes.
pub struct NavGrid {
    pub cols: i32,
    pub rows: i32,
    pub cell: f32,
    blocked: Vec<bool>,
}

impl NavGrid {
    /// Build the blocked mask straight from the level's tile grid (one cell
    /// per tile, no wall list to scan). This is what `Level` uses; `build` is
    /// kept for hand-built test maps.
    pub fn from_tiles(tiles: &TileMap, class: SizeClass) -> NavGrid {
        let cols = tiles.cols();
        let rows = tiles.rows();
        let mut blocked = vec![false; (cols * rows) as usize];
        for cy in 0..rows {
            for cx in 0..cols {
                blocked[(cy * cols + cx) as usize] = !tiles.passable(cx, cy, class);
            }
        }
        NavGrid {
            cols,
            rows,
            cell: TILE_F,
            blocked,
        }
    }

    pub fn build(arena: Vec2, walls: &[Rectf], cell: f32) -> NavGrid {
        let cols = (arena.x / cell).ceil() as i32;
        let rows = (arena.y / cell).ceil() as i32;
        let mut blocked = vec![false; (cols * rows) as usize];
        // Inflate walls slightly so flow lines keep a little clearance.
        let m = 5.0;
        for cy in 0..rows {
            for cx in 0..cols {
                let x = cx as f32 * cell;
                let y = cy as f32 * cell;
                let b = walls.iter().any(|w| {
                    x + cell > w.x - m
                        && x < w.right() + m
                        && y + cell > w.y - m
                        && y < w.bottom() + m
                });
                blocked[(cy * cols + cx) as usize] = b;
            }
        }
        NavGrid {
            cols,
            rows,
            cell,
            blocked,
        }
    }

    #[inline]
    fn idx(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    #[inline]
    pub fn is_blocked(&self, cx: i32, cy: i32) -> bool {
        cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows || self.blocked[self.idx(cx, cy)]
    }

    #[inline]
    pub fn cell_of(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell) as i32, (p.y / self.cell) as i32)
    }
}

// ---- Passability masks -----------------------------------------------------

/// One-way door directions are stored as a per-cell code so the navigation edge
/// test never touches the [`TileMap`] again: `0` = not a door, otherwise the
/// [`Cardinal`] direction the door may be travelled in.
#[inline]
const fn dir_code(dir: Cardinal) -> u8 {
    match dir {
        Cardinal::North => 1,
        Cardinal::East => 2,
        Cardinal::South => 3,
        Cardinal::West => 4,
    }
}

/// Per-class occupancy plus the (class-independent) one-way door directions.
///
/// Built once in [`FlowField::new`]. Rat-holes are the only kind whose
/// passability differs between the classes, and `open[Large]` is `false` for a
/// one-way door cell as well (only the rat may use it).
struct Masks {
    cols: i32,
    rows: i32,
    open: [Vec<bool>; 2],
    /// `open && door == 0`: a cell a *coarse* route may use. One-way door cells
    /// are excluded because a small enemy would jam against the leaf.
    gate: [Vec<bool>; 2],
    door: Vec<u8>,
}

impl Masks {
    fn from_tiles(tiles: &TileMap) -> Masks {
        let cols = tiles.cols();
        let rows = tiles.rows();
        let n = (cols * rows) as usize;
        let mut small = vec![false; n];
        let mut large = vec![false; n];
        let mut small_gate = vec![false; n];
        let mut large_gate = vec![false; n];
        let mut door = vec![0u8; n];
        for cy in 0..rows {
            for cx in 0..cols {
                let kind = tiles.get(cx, cy);
                let i = (cy * cols + cx) as usize;
                small[i] = kind.passable(SizeClass::Small);
                large[i] = kind.passable(SizeClass::Large);
                if let TileKind::Wall(super::art_walls::WallKind::OneWayDoor { dir }) = kind {
                    door[i] = dir_code(dir);
                }
                small_gate[i] = small[i] && door[i] == 0;
                large_gate[i] = large[i] && door[i] == 0;
            }
        }
        Masks {
            cols,
            rows,
            open: [small, large],
            gate: [small_gate, large_gate],
            door,
        }
    }

    #[inline]
    fn index(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    #[inline]
    fn in_bounds(&self, cx: i32, cy: i32) -> bool {
        cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows
    }

    /// May a `class` agent stand in this cell? Out of bounds is solid.
    #[inline]
    fn open(&self, class: usize, cx: i32, cy: i32) -> bool {
        if !self.in_bounds(cx, cy) {
            return false;
        }
        self.open[class][self.index(cx, cy)]
    }

    /// Allowed travel direction of a one-way door cell (`0` = not a door).
    #[inline]
    fn door(&self, cx: i32, cy: i32) -> u8 {
        if !self.in_bounds(cx, cy) {
            return 0;
        }
        self.door[self.index(cx, cy)]
    }

    /// Can a `class` agent move **into** `(nx, ny)` travelling in `dir`?
    #[inline]
    fn enters(&self, class: usize, nx: i32, ny: i32, dir: Cardinal) -> bool {
        let code = self.door(nx, ny);
        if code != 0 {
            code == dir_code(dir) && self.open(class, nx, ny)
        } else {
            self.open(class, nx, ny)
        }
    }

    /// May a `class` agent **leave** `(cx, cy)` along `dir`? (A one-way door cell
    /// may only be left in its allowed direction.)
    #[inline]
    fn leaves(&self, cx: i32, cy: i32, dir: Cardinal) -> bool {
        let code = self.door(cx, cy);
        code == 0 || code == dir_code(dir)
    }

    /// The whole navigation edge rule for one 8-connected step: may a `class`
    /// agent at `(cx, cy)` move to `(cx + dx, cy + dy)`?
    ///
    /// This is [`super::levelgen::validate::reachable`]'s rule, factored out so
    /// the BFS (which walks it backwards from the goal) and the direction
    /// derivation cannot disagree.
    #[inline]
    fn can_step(&self, class: usize, cx: i32, cy: i32, dx: i32, dy: i32) -> bool {
        let (nx, ny) = (cx + dx, cy + dy);
        match cardinal_of(dx, dy) {
            Some(dir) => self.enters(class, nx, ny, dir) && self.leaves(cx, cy, dir),
            None => {
                // No door cell may be crossed diagonally, and **both** vanilla
                // neighbours must be open: a diagonal squeeze past a wall
                // corner is fine for a point but not for a rat-sized circle (it
                // clips the corner and wall resolution parks it). This is
                // stricter than `levelgen::validate::reachable`, which may use
                // the "either side open" rule because it only proves
                // connectivity of the map, not that a body can walk the line.
                self.open(class, nx, ny)
                    && self.door(nx, ny) == 0
                    && self.door(cx, cy) == 0
                    && self.open(class, cx + dx, cy)
                    && self.open(class, cx, cy + dy)
            }
        }
    }

    /// Do both classes see the same passability over a fine window? When they
    /// do, one field serves both (see the module docs).
    fn window_shared(&self, win: (i32, i32)) -> bool {
        for cy in win.1..win.1 + FINE_SIDE {
            for cx in win.0..win.0 + FINE_SIDE {
                let same = self.in_bounds(cx, cy)
                    && self.open[0][self.index(cx, cy)] == self.open[1][self.index(cx, cy)];
                let both_out = !self.in_bounds(cx, cy);
                if !same && !both_out {
                    return false;
                }
            }
        }
        true
    }
}

// ---- Fine window helpers ---------------------------------------------------

/// Index of a cell inside a fine window, `None` when it lies outside.
#[inline]
fn win_index(win: (i32, i32), cx: i32, cy: i32) -> Option<usize> {
    let ix = cx - win.0;
    let iy = cy - win.1;
    if ix < 0 || iy < 0 || ix >= FINE_SIDE || iy >= FINE_SIDE {
        None
    } else {
        Some((iy * FINE_SIDE + ix) as usize)
    }
}

/// World-pixel centre of a cell.
#[inline]
fn cell_centre(cx: i32, cy: i32) -> Vec2 {
    Vec2::new((cx as f32 + 0.5) * TILE_F, (cy as f32 + 0.5) * TILE_F)
}

/// Cell containing a world position (floor, so negative coordinates stay on the
/// correct side of the origin).
#[inline]
fn cell_of_pos(p: Vec2) -> (i32, i32) {
    ((p.x / TILE_F).floor() as i32, (p.y / TILE_F).floor() as i32)
}

/// Which stage of a recompute the field is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// BFS integration field over the fine window.
    Fine,
    /// Per-cell routing directions over the fine window.
    FineDirs,
    /// Coarse block BFS + targets.
    Coarse,
    /// Fully committed; `job` is dropped on the next pump.
    Done,
}

/// An in-flight recompute. Small and `Copy`: the field keeps the buffers.
#[derive(Clone, Copy, Debug)]
struct Job {
    goal: (i32, i32),
    win: (i32, i32),
    shared: bool,
    /// Class whose fine field is being built.
    class: usize,
    stage: Stage,
    /// Direction-derivation cursor (fine cells, then coarse blocks).
    cursor: usize,
    /// Class whose coarse field is being built.
    coarse_class: usize,
}

// ---- Static coarse tables --------------------------------------------------

/// `(open, gate bits, gate midpoints)` per size class, one entry per block.
type BlockTables = ([Vec<bool>; 2], [Vec<u8>; 2], [Vec<[Vec2; 4]>; 2]);

/// Coarse block tables for both size classes, built once per [`FlowField`]:
/// which blocks are traversable, and which of their 8 edges have a real gate.
///
/// One pass over the cells per class: a gate cell (open, not a one-way door)
/// counts toward its block's open total and sets the edge bit on both sides of
/// every block boundary it straddles. Doing this here keeps the per-frame
/// coarse BFS to array lookups.
/// Middle row (for an east/west edge) or column (for a north/south edge) of the
/// block a cell belongs to: the gate closest to it wins, so a wide corridor
/// keeps its centre line instead of picking the first opening in the run.
#[inline]
fn by_mid(cx: i32, cy: i32, horizontal_edge: bool) -> i32 {
    if horizontal_edge {
        (cy / COARSE_BLOCK_CELLS) * COARSE_BLOCK_CELLS + COARSE_BLOCK_CELLS / 2
    } else {
        (cx / COARSE_BLOCK_CELLS) * COARSE_BLOCK_CELLS + COARSE_BLOCK_CELLS / 2
    }
}

fn build_blocks(masks: &Masks, blocks_x: i32, blocks_y: i32) -> BlockTables {
    const E: u8 = 1;
    const W: u8 = 2;
    const S: u8 = 4;
    const N: u8 = 8;
    let n = (blocks_x * blocks_y) as usize;
    let mut open = [vec![false; n], vec![false; n]];
    let mut gates = [vec![0u8; n], vec![0u8; n]];
    // Midpoint of one real gate per block edge, in `[E, W, S, N]` order: the
    // coarse route aims at the *opening* it will cross rather than at the
    // neighbouring block's centre, which can sit behind a wall the block graph
    // crossed elsewhere in the band.
    let mut gate_pos = [vec![[Vec2::ZERO; 4]; n], vec![[Vec2::ZERO; 4]; n]];
    // Distance of the chosen gate from the block's middle line, so a corridor
    // gets the opening in the middle of its run instead of the first cell.
    let mut gate_off = [vec![[i32::MAX; 4]; n], vec![[i32::MAX; 4]; n]];
    for class in 0..2 {
        let mut count = vec![0i32; n];
        for cy in 0..masks.rows {
            for cx in 0..masks.cols {
                if !masks.gate[class][masks.index(cx, cy)] {
                    continue;
                }
                let b = (cy / COARSE_BLOCK_CELLS) * blocks_x + (cx / COARSE_BLOCK_CELLS);
                count[b as usize] += 1;
                let last_col = cx % COARSE_BLOCK_CELLS == COARSE_BLOCK_CELLS - 1;
                let last_row = cy % COARSE_BLOCK_CELLS == COARSE_BLOCK_CELLS - 1;
                if last_col
                    && cx + 1 < masks.cols
                    && masks.gate[class][masks.index(cx + 1, cy)]
                {
                    gates[class][b as usize] |= E;
                    gates[class][(b + 1) as usize] |= W;
                    let mid = Vec2::new((cx + 1) as f32 * TILE_F, (cy as f32 + 0.5) * TILE_F);
                    let off = (cy - (by_mid(cx, cy, true))).abs();
                    let bi = b as usize;
                    if off < gate_off[class][bi][0] {
                        gate_off[class][bi][0] = off;
                        gate_pos[class][bi][0] = mid;
                    }
                    let wi = (b + 1) as usize;
                    if off < gate_off[class][wi][1] {
                        gate_off[class][wi][1] = off;
                        gate_pos[class][wi][1] = mid;
                    }
                }
                if last_row
                    && cy + 1 < masks.rows
                    && masks.gate[class][masks.index(cx, cy + 1)]
                {
                    gates[class][b as usize] |= S;
                    gates[class][(b + blocks_x) as usize] |= N;
                    let mid = Vec2::new((cx as f32 + 0.5) * TILE_F, (cy + 1) as f32 * TILE_F);
                    let off = (cx - (by_mid(cx, cy, false))).abs();
                    let si = b as usize;
                    if off < gate_off[class][si][2] {
                        gate_off[class][si][2] = off;
                        gate_pos[class][si][2] = mid;
                    }
                    let ni = (b + blocks_x) as usize;
                    if off < gate_off[class][ni][3] {
                        gate_off[class][ni][3] = off;
                        gate_pos[class][ni][3] = mid;
                    }
                }
            }
        }
        for (i, c) in count.iter().enumerate() {
            open[class][i] = *c >= BLOCK_MIN_OPEN;
        }
    }
    (open, gates, gate_pos)
}

// ---- The flow field --------------------------------------------------------

#[derive(bevy_ecs::resource::Resource)]
pub struct FlowField {
    cols: i32,
    rows: i32,
    masks: Masks,

    // Committed fine fields. `fine_shared` means class 1 reads slot 0.
    goal: Option<(i32, i32)>,
    fine_shared: bool,
    fine_valid: [bool; 2],
    fine_win: [(i32, i32); 2],
    fine_dir: [Vec<Vec2>; 2],
    fine_cost: [Vec<u32>; 2],

    // Committed coarse fields (one target point per block).
    blocks_x: i32,
    blocks_y: i32,
    coarse_target: [Vec<Vec2>; 2],
    coarse_cost: [Vec<u32>; 2],

    // In-flight recompute.
    job: Option<Job>,
    queue: VecDeque<(i32, i32)>,
    scratch_cost: Vec<u32>,
    scratch_dir: Vec<Vec2>,
    coarse_queue: VecDeque<(i32, i32)>,
    coarse_scratch_cost: Vec<u32>,
    coarse_scratch_target: Vec<Vec2>,
    /// Static coarse tables, built once in [`FlowField::new`]: is a block
    /// traversable, and which of its 8 edges have a real gate.
    block_open: [Vec<bool>; 2],
    block_gate: [Vec<u8>; 2],
    block_gate_pos: [Vec<[Vec2; 4]>; 2],
}

impl FlowField {
    /// Build the per-class masks from the tile grid and reserve every buffer.
    /// No field exists yet: call [`Self::update`] before reading
    /// [`Self::dir_at`] (it answers `ZERO` until then, i.e. "steer straight").
    pub fn new(tiles: &TileMap) -> FlowField {
        let cols = tiles.cols();
        let rows = tiles.rows();
        let blocks_x = (cols + COARSE_BLOCK_CELLS - 1) / COARSE_BLOCK_CELLS;
        let blocks_y = (rows + COARSE_BLOCK_CELLS - 1) / COARSE_BLOCK_CELLS;
        let fine_n = (FINE_SIDE * FINE_SIDE) as usize;
        let coarse_n = (blocks_x * blocks_y) as usize;
        let masks = Masks::from_tiles(tiles);
        let (block_open, block_gate, block_gate_pos) =
            build_blocks(&masks, blocks_x, blocks_y);
        FlowField {
            cols,
            rows,
            masks,
            goal: None,
            fine_shared: false,
            fine_valid: [false; 2],
            fine_win: [(0, 0); 2],
            fine_dir: std::array::from_fn(|_| vec![Vec2::ZERO; fine_n]),
            fine_cost: std::array::from_fn(|_| vec![u32::MAX; fine_n]),
            blocks_x,
            blocks_y,
            coarse_target: std::array::from_fn(|_| vec![Vec2::ZERO; coarse_n]),
            coarse_cost: std::array::from_fn(|_| vec![u32::MAX; coarse_n]),
            job: None,
            queue: VecDeque::with_capacity(fine_n),
            scratch_cost: vec![u32::MAX; fine_n],
            scratch_dir: vec![Vec2::ZERO; fine_n],
            coarse_queue: VecDeque::with_capacity(coarse_n),
            coarse_scratch_cost: vec![u32::MAX; coarse_n],
            coarse_scratch_target: vec![Vec2::ZERO; coarse_n],
            block_open,
            block_gate,
            block_gate_pos,
        }
    }

    /// Recompute toward `goal` if it moved to a new cell, spending at most
    /// `budget` work this call. Returns what changed.
    ///
    /// A recompute that does not fit in one budget is resumed by the next call
    /// (the previous field stays readable meanwhile). Asking for the same goal
    /// cell again does no work at all.
    pub fn update(&mut self, tiles: &TileMap, goal: Vec2, budget: NavBudget) -> NavUpdate {
        debug_assert_eq!(
            (self.cols, self.rows),
            (tiles.cols(), tiles.rows()),
            "a FlowField is built for one tile grid"
        );
        let mut out = NavUpdate::default();
        let Some(gc) = self.resolved_goal_cell(tiles, goal) else {
            // No open cell anywhere near the target: keep the previous field
            // (routing toward a stale goal beats clearing it to ZERO).
            out.shared = self.fine_shared;
            return out;
        };

        if self.job.map(|j| j.goal) != Some(gc) {
            if self.goal == Some(gc) {
                // The player came back to the committed goal cell: drop the
                // partial work, the field already points there.
                self.job = None;
                out.shared = self.fine_shared;
                return out;
            }
            self.start(gc);
        }
        out.shared = self.job.map_or(self.fine_shared, |j| j.shared);
        self.pump(budget, &mut out);
        out
    }

    /// Begin a recompute toward `goal`. Free (the expensive part is pumped).
    fn start(&mut self, goal: (i32, i32)) {
        let win = (
            goal.0 - FINE_RADIUS_CELLS,
            goal.1 - FINE_RADIUS_CELLS,
        );
        let shared = self.masks.window_shared(win);
        self.reset_fine_scratch(goal, win);
        self.job = Some(Job {
            goal,
            win,
            shared,
            class: 0,
            stage: Stage::Fine,
            cursor: 0,
            coarse_class: 0,
        });
    }

    fn reset_fine_scratch(&mut self, goal: (i32, i32), win: (i32, i32)) {
        self.scratch_cost.fill(u32::MAX);
        self.queue.clear();
        let i = win_index(win, goal.0, goal.1).expect("goal cell is inside its window");
        self.scratch_cost[i] = 0;
        self.queue.push_back(goal);
    }

    /// Spend the budget across the job's stages until it runs out or the job
    /// completes.
    fn pump(&mut self, budget: NavBudget, out: &mut NavUpdate) {
        let mut fine_left = budget.fine_cells;
        let mut coarse_left = budget.coarse_blocks;
        loop {
            let Some(mut job) = self.job else { return };
            match job.stage {
                Stage::Fine => {
                    if fine_left == 0 {
                        return;
                    }
                    let used = self.fine_bfs_step(job, fine_left);
                    fine_left -= used;
                    out.fine_cells += used;
                    if self.queue.is_empty() {
                        job.stage = Stage::FineDirs;
                        job.cursor = 0;
                    }
                    self.job = Some(job);
                }
                Stage::FineDirs => {
                    if fine_left == 0 {
                        return;
                    }
                    let used = self.fine_dir_step(job, fine_left);
                    fine_left -= used;
                    out.fine_cells += used;
                    // `fine_dir_step` advanced the cursor in `self.job`: re-read
                    // it, the local copy is stale.
                    let mut job = self.job.expect("the job survives a dir step");
                    if job.cursor >= (FINE_SIDE * FINE_SIDE) as usize {
                        self.commit_fine(job);
                        if !job.shared && job.class == 0 {
                            // Second class gets its own window.
                            job.class = 1;
                            job.stage = Stage::Fine;
                            job.cursor = 0;
                            self.job = Some(job);
                            self.reset_fine_scratch(job.goal, job.win);
                        } else {
                            job.stage = Stage::Coarse;
                            job.cursor = 0;
                            job.coarse_class = 0;
                            self.job = Some(job);
                            self.start_coarse(job);
                        }
                    } else {
                        self.job = Some(job);
                    }
                }
                Stage::Coarse => {
                    if coarse_left == 0 {
                        return;
                    }
                    let used = if !self.coarse_queue.is_empty() {
                        self.coarse_bfs_step(job, coarse_left)
                    } else {
                        self.coarse_dir_step(job, coarse_left)
                    };
                    coarse_left -= used;
                    out.coarse_blocks += used;
                    let mut job = self.job.expect("the job survives a coarse step");
                    if job.cursor >= (self.blocks_x * self.blocks_y) as usize {
                        self.commit_coarse(job.coarse_class);
                        if job.coarse_class == 0 {
                            job.coarse_class = 1;
                            job.cursor = 0;
                            self.job = Some(job);
                            self.start_coarse(job);
                        } else {
                            job.stage = Stage::Done;
                            self.job = Some(job);
                        }
                    } else {
                        self.job = Some(job);
                    }
                }
                Stage::Done => {
                    self.job = None;
                    self.goal = Some(job.goal);
                    out.changed = true;
                    out.shared = job.shared;
                    return;
                }
            }
        }
    }

    /// Nearest passable cell to `goal` (small class first — that is the rat's
    /// own rule — then large), clamped into the grid.
    fn resolved_goal_cell(&self, tiles: &TileMap, goal: Vec2) -> Option<(i32, i32)> {
        let (gx, gy) = cell_of_pos(goal);
        let cx = gx.clamp(0, self.cols - 1);
        let cy = gy.clamp(0, self.rows - 1);
        for class in SizeClass::ALL {
            if tiles.passable(cx, cy, class) {
                return Some((cx, cy));
            }
        }
        for r in 1..=8i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    let (nx, ny) = (cx + dx, cy + dy);
                    for class in SizeClass::ALL {
                        if tiles.passable(nx, ny, class) {
                            return Some((nx, ny));
                        }
                    }
                }
            }
        }
        None
    }

    // ---- Fine window ------------------------------------------------------

    /// Expand up to `budget` queued cells of the fine BFS. The BFS runs
    /// backwards from the goal, so the edge test is for the *reverse* of the
    /// step being taken (cell `nx` is reached from `cx` only if an agent can
    /// walk `nx -> cx`).
    fn fine_bfs_step(&mut self, job: Job, budget: u32) -> u32 {
        let class = job.class;
        let mut used = 0;
        while used < budget {
            let Some((cx, cy)) = self.queue.pop_front() else { break };
            used += 1;
            let i = win_index(job.win, cx, cy).expect("queued cell is inside the window");
            let c = self.scratch_cost[i];
            for &(dx, dy) in OFFS.iter() {
                let (nx, ny) = (cx + dx, cy + dy);
                let Some(ni) = win_index(job.win, nx, ny) else {
                    continue;
                };
                if self.scratch_cost[ni] != u32::MAX {
                    continue;
                }
                if !self.masks.can_step(class, nx, ny, -dx, -dy) {
                    continue;
                }
                self.scratch_cost[ni] = c + 1;
                self.queue.push_back((nx, ny));
            }
        }
        used
    }

    /// Derive the routing direction of up to `budget` window cells.
    fn fine_dir_step(&mut self, mut job: Job, budget: u32) -> u32 {
        let class = job.class;
        let total = (FINE_SIDE * FINE_SIDE) as usize;
        let mut used = 0;
        while job.cursor < total && used < budget {
            let i = job.cursor;
            job.cursor += 1;
            used += 1;
            let cx = job.win.0 + (i as i32 % FINE_SIDE);
            let cy = job.win.1 + (i as i32 / FINE_SIDE);
            self.scratch_dir[i] = self.fine_dir_for(job, class, cx, cy, i);
        }
        self.job = Some(job);
        used
    }

    /// Direction of one fine cell. Mirrors the pre-split field's rules: a blend
    /// toward every cheaper neighbour, but only when it actually steps downhill
    /// (otherwise the cheapest neighbour wins, so progress is monotone); a
    /// blocked cell gets an escape direction instead.
    fn fine_dir_for(&self, job: Job, class: usize, cx: i32, cy: i32, i: usize) -> Vec2 {
        if !self.masks.open(class, cx, cy) {
            return self.fine_escape(job, class, cx, cy);
        }
        let c = self.scratch_cost[i];
        if c == u32::MAX || c == 0 {
            return Vec2::ZERO;
        }
        let mut d = Vec2::ZERO;
        let mut best: Option<(u32, Vec2)> = None;
        for &(dx, dy) in OFFS.iter() {
            if !self.masks.can_step(class, cx, cy, dx, dy) {
                continue;
            }
            let Some(ni) = win_index(job.win, cx + dx, cy + dy) else {
                continue;
            };
            let nc = self.scratch_cost[ni];
            if nc == u32::MAX || nc >= c {
                continue;
            }
            let step = Vec2::new(dx as f32, dy as f32).normalized();
            d += step * (c - nc) as f32;
            if best.is_none_or(|(bc, _)| nc < bc) {
                best = Some((nc, step));
            }
        }
        let blended = d.normalized();
        if blended.len_sq() > 0.0 {
            let (bx, by) = (blended.x.round() as i32, blended.y.round() as i32);
            // The blended vector must land on a cell that is both downhill and
            // *walkable* from here: rounding two downhill steps can pick a
            // diagonal whose corner is walled (a point can squeeze past, a
            // rat-sized body cannot), and steering into that corner parks it.
            if self.masks.can_step(class, cx, cy, bx, by) {
                if let Some(ti) = win_index(job.win, cx + bx, cy + by) {
                    if self.scratch_cost[ti] < c {
                        return blended;
                    }
                }
            }
        }
        best.map_or(Vec2::ZERO, |(_, step)| step)
    }

    /// Direction out of a blocked (or unreached) fine cell: toward its
    /// cheapest free 8-neighbour, else the coarse field's opinion (which is
    /// defined everywhere and costs one block lookup).
    ///
    /// Blocked cells appear when wall resolution has not yet ejected an enemy,
    /// or when the player ducked into a rat-hole the class cannot fit. The old
    /// field searched a widening ring here; on a map full of props that was the
    /// single most expensive thing in the whole recompute, and the coarse
    /// target is a better answer anyway (it points along the route, not at the
    /// nearest hole in the wall).
    fn fine_escape(&self, job: Job, class: usize, cx: i32, cy: i32) -> Vec2 {
        let mut best: Option<(u32, Vec2)> = None;
        let mut any_free: Option<Vec2> = None;
        for &(dx, dy) in OFFS.iter() {
            let (nx, ny) = (cx + dx, cy + dy);
            if !self.masks.open(class, nx, ny) {
                continue;
            }
            let v = Vec2::new(dx as f32, dy as f32).normalized();
            let Some(ni) = win_index(job.win, nx, ny) else {
                continue;
            };
            let nc = self.scratch_cost[ni];
            if nc == u32::MAX {
                any_free.get_or_insert(v);
                continue;
            }
            if best.is_none_or(|(bc, _)| nc < bc) {
                best = Some((nc, v));
            }
        }
        if let Some((_, v)) = best {
            return v;
        }
        if let Some(v) = any_free {
            return v;
        }
        self.coarse_dir(class, cell_centre(cx, cy))
    }

    /// Publish the finished window for the job's class (and for both classes
    /// when the window is shared).
    fn commit_fine(&mut self, job: Job) {
        let slot = if job.shared { 0 } else { job.class };
        self.fine_dir[slot].copy_from_slice(&self.scratch_dir);
        self.fine_cost[slot].copy_from_slice(&self.scratch_cost);
        self.fine_win[slot] = job.win;
        self.fine_valid[slot] = true;
        if job.shared {
            self.fine_shared = true;
        } else if job.class == 0 {
            // Class 1 keeps its previous window until its own build lands.
            self.fine_shared = false;
        }
    }

    // ---- Coarse blocks ----------------------------------------------------

    /// Inclusive-exclusive cell range of a block, clipped to the map.
    #[inline]
    fn block_range(&self, bx: i32, by: i32) -> (i32, i32, i32, i32) {
        let x0 = (bx * COARSE_BLOCK_CELLS).clamp(0, self.cols);
        let y0 = (by * COARSE_BLOCK_CELLS).clamp(0, self.rows);
        let x1 = ((bx + 1) * COARSE_BLOCK_CELLS).clamp(0, self.cols);
        let y1 = ((by + 1) * COARSE_BLOCK_CELLS).clamp(0, self.rows);
        (x0, y0, x1, y1)
    }

    #[inline]
    fn block_index(&self, bx: i32, by: i32) -> usize {
        (by * self.blocks_x + bx) as usize
    }

    #[inline]
    fn block_in_bounds(&self, bx: i32, by: i32) -> bool {
        bx >= 0 && by >= 0 && bx < self.blocks_x && by < self.blocks_y
    }

    /// Is a block traversable for `class` (enough open cells to walk through)?
    /// Static: computed once in [`FlowField::new`].
    #[inline]
    fn block_open_at(&self, class: usize, bx: i32, by: i32) -> bool {
        if !self.block_in_bounds(bx, by) {
            return false;
        }
        self.block_open[class][self.block_index(bx, by)]
    }

    /// World position of the gate a step from `(bx, by)` crosses (the midpoint
    /// of the boundary opening), `None` when that edge has no gate. A diagonal
    /// step returns the midpoint between its two half-gates.
    fn gate_midpoint(&self, class: usize, bx: i32, by: i32, dx: i32, dy: i32) -> Option<Vec2> {
        if !self.block_in_bounds(bx, by) {
            return None;
        }
        let pos = |dx: i32, dy: i32| -> Option<Vec2> {
            let i = match (dx.signum(), dy.signum()) {
                (1, 0) => 0,
                (-1, 0) => 1,
                (0, 1) => 2,
                (0, -1) => 3,
                _ => return None,
            };
            let p = self.block_gate_pos[class][self.block_index(bx, by)][i];
            if p.len_sq() == 0.0 {
                None
            } else {
                Some(p)
            }
        };
        match (pos(dx, 0), pos(0, dy)) {
            (Some(a), Some(b)) => Some((a + b) * 0.5),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// World-pixel centre of a block's middle **cell** (where a coarse target
    /// points). Using the geometric midpoint of the block instead would put the
    /// target on a cell boundary — 8 px off the centre of a one-cell corridor,
    /// which is enough to walk a rat into the pier beside a rat-hole.
    fn block_centre(&self, bx: i32, by: i32) -> Vec2 {
        let (x0, y0, x1, y1) = self.block_range(bx, by);
        cell_centre(x0 + (x1 - x0) / 2, y0 + (y1 - y0) / 2)
    }

    /// Does the edge from block `(bx, by)` toward `(dx, dy)` have a real gate
    /// (a pair of open, non-door cells straddling the shared boundary)?
    /// Diagonal edges need both of their orthogonal halves. Static, like
    /// [`Self::block_open_at`].
    #[inline]
    fn block_gate_at(&self, class: usize, bx: i32, by: i32, dx: i32, dy: i32) -> bool {
        if !self.block_in_bounds(bx, by) {
            return false;
        }
        const E: u8 = 1;
        const W: u8 = 2;
        const S: u8 = 4;
        const N: u8 = 8;
        let bits = self.block_gate[class][self.block_index(bx, by)];
        let open = |bit: u8| bits & bit != 0;
        match (dx.signum(), dy.signum()) {
            (1, 0) => open(E),
            (-1, 0) => open(W),
            (0, 1) => open(S),
            (0, -1) => open(N),
            (1, 1) => open(E) && open(S),
            (1, -1) => open(E) && open(N),
            (-1, 1) => open(W) && open(S),
            (-1, -1) => open(W) && open(N),
            _ => false,
        }
    }

    /// Start the coarse BFS for `job.coarse_class` from the goal's block (or
    /// the nearest traversable block when the player stands in a thin one).
    fn start_coarse(&mut self, job: Job) {
        let class = job.coarse_class;
        let (gx, gy) = (
            (job.goal.0 / COARSE_BLOCK_CELLS).clamp(0, self.blocks_x - 1),
            (job.goal.1 / COARSE_BLOCK_CELLS).clamp(0, self.blocks_y - 1),
        );
        self.coarse_scratch_cost.fill(u32::MAX);
        self.coarse_queue.clear();
        let mut start = None;
        'outer: for r in 0..=4i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    let (bx, by) = (gx + dx, gy + dy);
                    if self.block_open_at(class, bx, by) {
                        start = Some((bx, by));
                        break 'outer;
                    }
                }
            }
        }
        if let Some((bx, by)) = start {
            let i = self.block_index(bx, by);
            self.coarse_scratch_cost[i] = 0;
            self.coarse_queue.push_back((bx, by));
        }
    }

    /// Expand up to `budget` queued blocks.
    fn coarse_bfs_step(&mut self, job: Job, budget: u32) -> u32 {
        let class = job.coarse_class;
        let mut used = 0;
        while used < budget {
            let Some((bx, by)) = self.coarse_queue.pop_front() else {
                break;
            };
            used += 1;
            let c = self.coarse_scratch_cost[self.block_index(bx, by)];
            for &(dx, dy) in OFFS.iter() {
                let (nx, ny) = (bx + dx, by + dy);
                if !self.block_open_at(class, nx, ny) || !self.block_gate_at(class, bx, by, dx, dy) {
                    continue;
                }
                let ni = self.block_index(nx, ny);
                if self.coarse_scratch_cost[ni] == u32::MAX {
                    self.coarse_scratch_cost[ni] = c + 1;
                    self.coarse_queue.push_back((nx, ny));
                }
            }
        }
        used
    }

    /// Derive up to `budget` coarse targets: the centre of the next block on
    /// the route (or of the nearest reachable block, for a block the coarse
    /// graph cannot connect to the goal).
    fn coarse_dir_step(&mut self, mut job: Job, budget: u32) -> u32 {
        let class = job.coarse_class;
        let total = (self.blocks_x * self.blocks_y) as usize;
        let mut used = 0;
        while job.cursor < total && used < budget {
            let i = job.cursor;
            job.cursor += 1;
            used += 1;
            let bx = i as i32 % self.blocks_x;
            let by = i as i32 / self.blocks_x;
            self.coarse_scratch_target[i] = self.coarse_target_for(class, bx, by, i);
        }
        self.job = Some(job);
        used
    }

    fn coarse_target_for(&self, class: usize, bx: i32, by: i32, i: usize) -> Vec2 {
        let open = self.block_open_at(class, bx, by);
        let c = self.coarse_scratch_cost[i];
        if open && c == 0 {
            // The goal block: the fine field (or the straight line) takes over.
            return Vec2::ZERO;
        }
        if open && c != u32::MAX {
            let mut best: Option<(u32, Vec2)> = None;
            for &(dx, dy) in OFFS.iter() {
                let (nx, ny) = (bx + dx, by + dy);
                if !self.block_open_at(class, nx, ny) || !self.block_gate_at(class, bx, by, dx, dy) {
                    continue;
                }
                let nc = self.coarse_scratch_cost[self.block_index(nx, ny)];
                if nc == u32::MAX || nc >= c {
                    continue;
                }
                if best.is_none_or(|(bc, _)| nc < bc) {
                    // Aim at the gate this edge crosses, not at the neighbour's
                    // centre: the centre can sit behind a wall inside that
                    // block (the block graph only knows a gate exists somewhere
                    // on the shared boundary).
                    let target = self
                        .gate_midpoint(class, bx, by, dx, dy)
                        .unwrap_or_else(|| self.block_centre(nx, ny));
                    best = Some((nc, target));
                }
            }
            if let Some((_, target)) = best {
                return target;
            }
        }
        // Blocked or unreachable: aim at the nearest reachable block so a
        // straggler walks back onto the coarse graph instead of into a wall.
        for r in 1..=5i32 {
            let mut best: Option<(u32, Vec2)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    let (nx, ny) = (bx + dx, by + dy);
                    if !self.block_open_at(class, nx, ny) {
                        continue;
                    }
                    let nc = self.coarse_scratch_cost[self.block_index(nx, ny)];
                    if nc == u32::MAX {
                        continue;
                    }
                    if best.is_none_or(|(bc, _)| nc < bc) {
                        best = Some((nc, self.block_centre(nx, ny)));
                    }
                }
            }
            if let Some((_, target)) = best {
                return target;
            }
        }
        Vec2::ZERO
    }

    fn commit_coarse(&mut self, class: usize) {
        self.coarse_target[class].copy_from_slice(&self.coarse_scratch_target);
        self.coarse_cost[class].copy_from_slice(&self.coarse_scratch_cost);
    }

    // ---- Queries ----------------------------------------------------------

    /// Routing direction at a world position for one size class.
    ///
    /// Inside [`BLEND_INNER_CELLS`] of the player this is the fine field; out
    /// to [`FINE_RADIUS_CELLS`] it is blended into the coarse field; beyond it
    /// the coarse field is used. `ZERO` means "no opinion" — the caller steers
    /// straight at the player (only inside the goal's own neighbourhood, or
    /// when a class cannot be routed at all).
    ///
    /// The result is always **a legal step for `class`** (a unit vector whose
    /// rounded cell this body can walk into) or `ZERO`: see the module docs. A
    /// caller may therefore always move along it without checking the map.
    pub fn dir_at(&self, class: SizeClass, p: Vec2) -> Vec2 {
        let ci = class.index();
        let (cx, cy) = cell_of_pos(p);
        if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
            // Off the map (a body clamped or shoved past the edge): no opinion,
            // the caller steers straight.
            return Vec2::ZERO;
        }
        let slot = if self.fine_shared { 0 } else { ci };
        if self.fine_valid[slot] {
            let win = self.fine_win[slot];
            if let Some(i) = win_index(win, cx, cy) {
                let fine = self.fine_dir[slot][i];
                if fine.len_sq() > 0.0 {
                    let gx = win.0 + FINE_RADIUS_CELLS;
                    let gy = win.1 + FINE_RADIUS_CELLS;
                    let r = (cx - gx).abs().max((cy - gy).abs());
                    if r <= BLEND_INNER_CELLS {
                        return self.centred_step(ci, p, cx, cy, fine);
                    }
                    let coarse = self.coarse_dir(ci, p);
                    if coarse.len_sq() == 0.0 {
                        return self.centred_step(ci, p, cx, cy, fine);
                    }
                    let t = ((r - BLEND_INNER_CELLS) as f32
                        / (FINE_RADIUS_CELLS - BLEND_INNER_CELLS) as f32)
                        .clamp(0.0, 1.0);
                    let blended = (fine * (1.0 - t) + coarse * t).normalized();
                    if blended.len_sq() > 1e-4 {
                        // The coarse field is block-granular: never let it bend
                        // the fine direction into a wall, or into a cell this
                        // body cannot step to (a diagonal past a wall corner).
                        // Rounding the *normalised* vector, because that is
                        // what the caller moves along.
                        let bx = blended.x.round() as i32;
                        let by = blended.y.round() as i32;
                        if self.masks.can_step(ci, cx, cy, bx, by) {
                            return blended;
                        }
                    }
                    return self.centred_step(ci, p, cx, cy, fine);
                }
            }
        }
        self.coarse_dir(ci, p)
    }

    /// [`Self::centreline`], but only when the centred vector still rounds to a
    /// cell this body can actually step into.
    ///
    /// The pull is lateral as well as forward, so a body hugging one side of a
    /// cell can see the blend round to a *different* neighbour than the step it
    /// started from — a diagonal across a walled corner, for instance. Steering
    /// there parks the body against the corner (the classic "pinned flush"
    /// body), so the un-centred step is used instead.
    fn centred_step(&self, class: usize, p: Vec2, cx: i32, cy: i32, step: Vec2) -> Vec2 {
        let out = self.centreline(p, cx, cy, step);
        let (bx, by) = (out.x.round() as i32, out.y.round() as i32);
        if self.masks.can_step(class, cx, cy, bx, by) {
            out
        } else {
            step
        }
    }

    /// Pull a cell-step direction toward the centre of the cell it steps into.
    ///
    /// A pure cell-step field sends an agent down a 16 px grid line, so it
    /// clips the 1 px jamb of a doorway or the pier beside a rat-hole and wall
    /// resolution parks it there for good. Aiming the same step at the *centre*
    /// of the target cell keeps the body in the middle of a narrow passage; the
    /// line from `p` to that centre lies inside the two open cells, so the pull
    /// can never steer into a wall.
    fn centreline(&self, p: Vec2, cx: i32, cy: i32, dir: Vec2) -> Vec2 {
        let tx = cx + dir.x.round() as i32;
        let ty = cy + dir.y.round() as i32;
        let to = (cell_centre(tx, ty) - p).normalized();
        let out = dir + to * CENTRE_PULL;
        if out.len_sq() > 1e-4 {
            out.normalized()
        } else {
            dir
        }
    }

    /// Direction toward the current coarse block's target.
    ///
    /// Blocks are 8 cells wide, so the target point is block-granular: it can
    /// sit behind a wall that the coarse graph happily crossed at a gate. When
    /// the cell ahead is a wall, slide instead — pick the open neighbour that
    /// best agrees with the target direction — so a straggler follows the wall
    /// to the opening instead of pushing into it forever.
    /// Coarse steering. **It never returns a direction that is not a legal
    /// step** (or `ZERO`); that guarantee is what keeps the horde from
    /// pressing into a wall face forever.
    ///
    /// The guide is the coarse BFS's own cost gradient: of the legal steps out
    /// of the body's cell, take the one that lands in the cheapest block, and
    /// break ties with the geometric aim point (the gate midpoint the block's
    /// route crosses), which is what keeps the walk inside a block centred on
    /// the opening instead of hugging its edge.
    ///
    /// Aiming straight at the aim point was the bug behind the pinned bodies:
    /// the aim is a *point*, so a body that reaches it (or that is inside a
    /// centimetre of it) gets a direction that flips sign every tick and never
    /// leaves — measured with a full-speed velocity and a bit-for-bit unchanged
    /// position. Three shapes of that, all fixed by the gradient:
    ///
    /// 1. the aim point is **under** the body — a gate midpoint it has just
    ///    reached, or the average of two gate midpoints for a diagonal edge,
    ///    which lands inside the block;
    /// 2. the aim point is on the far side of a wall *inside* the block (gate
    ///    midpoints sit on the block edge, the interiors are furnished);
    /// 3. the body's block is off the coarse graph, so the fallback aim is a
    ///    neighbouring block's centre, straight through a wall.
    ///
    /// When there is no usable aim at all (case 1, exactly), the aim becomes
    /// the gate of the cheapest legal crossing, so the tie-break still means
    /// something. `ZERO` — "steer straight at the rat" — remains for the goal
    /// block (no aim by construction) and for a cell with no legal step.
    fn coarse_dir(&self, class: usize, p: Vec2) -> Vec2 {
        let (cx, cy) = cell_of_pos(p);
        let bx = (cx / COARSE_BLOCK_CELLS).clamp(0, self.blocks_x - 1);
        let by = (cy / COARSE_BLOCK_CELLS).clamp(0, self.blocks_y - 1);
        let i = self.block_index(bx, by);
        let target = self.coarse_target[class][i];
        let my_cost = self.coarse_cost[class][i];
        let mut dir = if target.len_sq() > 0.0 && (target - p).len_sq() > 1e-6 {
            (target - p).normalized()
        } else if my_cost != 0 {
            // No coarse target at all: the goal block (the fine field handles
            // it, `my_cost == 0`), or a block the coarse graph cannot express
            // (fewer than `BLOCK_MIN_OPEN` gate cells in the 8 x 8 block, or a
            // region its gates miss) even though the cells are reachable. Aim
            // at the goal cell: wall-following toward the rat beats standing
            // still, and the step selection keeps every move legal.
            match self.goal {
                Some((gx, gy)) => {
                    let to = cell_centre(gx, gy) - p;
                    if to.len_sq() > 1e-6 {
                        to.normalized()
                    } else {
                        Vec2::ZERO
                    }
                }
                None => Vec2::ZERO,
            }
        } else {
            // The goal block: "no opinion" — the caller steers straight.
            return Vec2::ZERO;
        };
        if dir.len_sq() == 0.0 {
            // Standing on the aim point: aim at the gate of the cheapest legal
            // crossing instead (deterministic: `OFFS` order, first cheapest).
            let mut best: Option<(u32, Vec2)> = None;
            for &(dx, dy) in OFFS.iter() {
                if !self.masks.can_step(class, cx, cy, dx, dy) {
                    continue;
                }
                let (nbx, nby) = self.block_of_cell(cx + dx, cy + dy);
                if (nbx, nby) == (bx, by) {
                    continue;
                }
                let cost = self.coarse_cost[class][self.block_index(nbx, nby)];
                if best.is_none_or(|(bc, _)| cost < bc) {
                    let gate = self
                        .gate_midpoint(class, bx, by, nbx - bx, nby - by)
                        .unwrap_or_else(|| self.block_centre(nbx, nby));
                    best = Some((cost, gate));
                }
            }
            if let Some((_, gate)) = best {
                let g = gate - p;
                if g.len_sq() > 1e-6 {
                    dir = g.normalized();
                }
            }
        }
        // Cheapest block first, then the step closest to the aim; `OFFS` order
        // breaks the remaining ties, so the choice is deterministic.
        let mut best: Option<(u32, f32, Vec2)> = None;
        for &(dx, dy) in OFFS.iter() {
            if !self.masks.can_step(class, cx, cy, dx, dy) {
                continue;
            }
            let (nbx, nby) = self.block_of_cell(cx + dx, cy + dy);
            let cost = self.coarse_cost[class][self.block_index(nbx, nby)];
            let step = Vec2::new(dx as f32, dy as f32).normalized();
            let align = step.dot(dir); // `dir == ZERO` ⇒ every step ties at 0
            let better = match best {
                None => true,
                Some((bc, ba, _)) => cost < bc || (cost == bc && align > ba + 1e-6),
            };
            if better {
                best = Some((cost, align, step));
            }
        }
        let Some((_, _, step)) = best else {
            // Boxed in on all eight sides: no opinion, the caller steers
            // straight and wall resolution does the rest.
            return Vec2::ZERO;
        };
        // Same centring as the fine field: a one-cell doorway is only 14 px
        // clear, so an agent walking its grid line clips the jamb and stops.
        self.centred_step(class, p, cx, cy, step)
    }

    /// The coarse block a grid cell belongs to (`(bx, by)`).
    #[inline]
    fn block_of_cell(&self, cx: i32, cy: i32) -> (i32, i32) {
        (
            (cx / COARSE_BLOCK_CELLS).clamp(0, self.blocks_x - 1),
            (cy / COARSE_BLOCK_CELLS).clamp(0, self.blocks_y - 1),
        )
    }

    /// Do both size classes currently read one shared fine field?
    pub fn shared(&self) -> bool {
        self.fine_shared
    }

    /// The cell the committed field routes toward.
    pub fn goal_cell(&self) -> Option<(i32, i32)> {
        self.goal
    }

    /// Fine-window routing (tests and diagnostics): `(dir, cost)` of a cell in
    /// the committed window, `None` when it is outside it.
    #[cfg(test)]
    fn fine_sample(&self, class: SizeClass, cx: i32, cy: i32) -> Option<(Vec2, u32)> {
        let slot = if self.fine_shared { 0 } else { class.index() };
        if !self.fine_valid[slot] {
            return None;
        }
        let i = win_index(self.fine_win[slot], cx, cy)?;
        Some((self.fine_dir[slot][i], self.fine_cost[slot][i]))
    }

    #[cfg(test)]
    fn coarse_sample(&self, class: SizeClass, bx: i32, by: i32) -> (Vec2, u32) {
        let i = self.block_index(bx, by);
        (
            self.coarse_target[class.index()][i],
            self.coarse_cost[class.index()][i],
        )
    }
}

/// A `cols × rows` tile map with `walls` painted as solid cells: the test
/// replacement for the old `NavGrid::build(arena, &walls, 16.0)` helper.
///
/// Only compiled for tests; `enemy.rs`'s synthetic-map tests use it too.
#[cfg(test)]
pub fn test_tiles(cols: i32, rows: i32, walls: &[Rectf]) -> TileMap {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;
    use crate::game::level::Theme;
    let mut tiles = TileMap::new(cols, rows, Theme::Sewer, 0);
    for w in walls {
        let x0 = (w.x / TILE_F).floor() as i32;
        let y0 = (w.y / TILE_F).floor() as i32;
        let x1 = (w.right() / TILE_F).ceil() as i32;
        let y1 = (w.bottom() / TILE_F).ceil() as i32;
        tiles.fill_cells(x0, y0, x1 - x0, y1 - y0, TileKind::Wall(WallKind::Solid));
    }
    tiles.rebuild_masks();
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::{WallAxis, WallKind};
    use crate::game::level::{Level, Theme};
    use std::time::Instant;

    /// Full-budget solve: one call finishes the whole recompute.
    fn solve(tiles: &TileMap, goal: Vec2) -> FlowField {
        let mut f = FlowField::new(tiles);
        let u = f.update(tiles, goal, NavBudget::FULL);
        assert!(u.changed, "a full-budget update must finish");
        f
    }

    fn radius_of(class: SizeClass) -> f32 {
        match class {
            SizeClass::Small => 3.0,
            SizeClass::Large => 4.0,
        }
    }

    /// Drive a point agent along the field until it reaches `goal` or runs out
    /// of steps. Walls are resolved with the tile grid, exactly like a system.
    ///
    /// `player` grants the one-way-door permission (`Some(vel)` is the player
    /// rat only; every enemy passes `None`, so a gate leaf is solid to it).
    fn walk_as(
        tiles: &TileMap,
        f: &FlowField,
        class: SizeClass,
        from: Vec2,
        goal: Vec2,
        steps: usize,
        player: bool,
    ) -> (Vec2, bool) {
        let r = radius_of(class);
        let mut p = from;
        for _ in 0..steps {
            if p.dist(goal) < 6.0 {
                return (p, true);
            }
            let d = f.dir_at(class, p);
            let step = if d.len_sq() > 0.0 {
                d
            } else {
                (goal - p).normalized()
            };
            let before = p;
            p += step * 2.0;
            let vel = if player { Some(step) } else { None };
            p = tiles.resolve_circle(p, r, class, vel);
            if p.dist(before) < 0.5 {
                // Blocked head-on: slide along the wall, which is what the
                // velocity smoothing in `Enemy::steer` does for a real enemy.
                for side in [step.perp(), -step.perp()] {
                    let cand = tiles.resolve_circle(before + side * 2.0, r, class, vel);
                    if cand.dist(before) > 0.5 {
                        p = cand;
                        break;
                    }
                }
            }
        }
        let reached = p.dist(goal) < 6.0;
        (p, reached)
    }

    /// [`walk_as`] as an enemy (no gate permission).
    fn walk(
        tiles: &TileMap,
        f: &FlowField,
        class: SizeClass,
        from: Vec2,
        goal: Vec2,
        steps: usize,
    ) -> (Vec2, bool) {
        walk_as(tiles, f, class, from, goal, steps, false)
    }

    fn centre(cx: i32, cy: i32) -> Vec2 {
        cell_centre(cx, cy)
    }

    /// A vertical solid wall with one hole in it: the classic two-room map.
    fn two_rooms(hole: TileKind) -> TileMap {
        let mut tiles = TileMap::new(48, 24, Theme::Sewer, 0);
        tiles.fill_cells(
            24,
            0,
            1,
            24,
            TileKind::Wall(WallKind::Solid),
        );
        tiles.set(24, 12, hole);
        tiles.rebuild_masks();
        tiles
    }

    // ---- Basic routing ----------------------------------------------------

    #[test]
    fn flow_points_toward_goal_in_open_field() {
        let tiles = test_tiles(20, 20, &[]);
        let goal = Vec2::new(160.0, 160.0);
        let f = solve(&tiles, goal);
        let from = Vec2::new(24.0, 24.0);
        let d = f.dir_at(SizeClass::Small, from);
        let to_goal = (goal - from).normalized();
        assert!(d.dot(to_goal) > 0.5, "flow should aim at the goal: {d:?}");
    }

    #[test]
    fn flow_routes_around_a_wall() {
        // A vertical wall with a gap at the bottom; goal on the right.
        let tiles = test_tiles(20, 20, &[Rectf::new(96.0, 0.0, 16.0, 240.0)]);
        let goal = Vec2::new(260.0, 120.0);
        let f = solve(&tiles, goal);
        let from = Vec2::new(60.0, 120.0);
        let d = f.dir_at(SizeClass::Small, from);
        assert!(d.len() > 0.1, "left-side cell should have a route");
        assert!(d.x < 0.98, "flow should bend around, not push into the wall");
        let (end, reached) = walk(&tiles, &f, SizeClass::Small, from, goal, 2_000);
        assert!(reached, "agent got stuck at {end:?} going around the wall");
    }

    #[test]
    fn blocked_cells_escape_toward_the_cheapest_free_neighbour() {
        let tiles = test_tiles(20, 20, &[Rectf::new(100.0, 40.0, 16.0, 66.0)]);
        let goal = Vec2::new(200.0, 73.0);
        let f = solve(&tiles, goal);
        // A point inside the wall cell (an enemy wall resolution has not
        // ejected yet) must have an escape direction, not ZERO.
        let (cx, cy) = cell_of_pos(Vec2::new(105.0, 73.0));
        assert!(!tiles.passable(cx, cy, SizeClass::Small));
        let d = f.dir_at(SizeClass::Small, Vec2::new(105.0, 73.0));
        assert!(d.len() > 0.5, "blocked cell has no escape direction: {d:?}");
        let (ex, ey) = (d.x.round() as i32, d.y.round() as i32);
        assert!(tiles.passable(cx + ex, cy + ey, SizeClass::Small));
    }

    #[test]
    fn an_agent_parked_in_a_wall_walks_back_onto_the_field() {
        let tiles = test_tiles(20, 20, &[Rectf::new(100.0, 40.0, 16.0, 66.0)]);
        let goal = Vec2::new(200.0, 73.0);
        let f = solve(&tiles, goal);
        let start = Vec2::new(105.0, 73.0);
        let (end, _) = walk(&tiles, &f, SizeClass::Small, start, goal, 900);
        assert!(
            end.dist(goal) < start.dist(goal),
            "agent made no progress: {end:?}"
        );
        assert!(tiles.passable(
            cell_of_pos(end).0,
            cell_of_pos(end).1,
            SizeClass::Small
        ));
    }



    // ---- Robustness -------------------------------------------------------

    #[test]
    fn goal_outside_the_arena_is_clamped_not_indexed() {
        let tiles = test_tiles(20, 20, &[]);
        let mut f = FlowField::new(&tiles);
        // Used to index `cost` out of range and panic.
        f.update(&tiles, Vec2::new(-5000.0, -5000.0), NavBudget::FULL);
        f.update(&tiles, Vec2::new(9000.0, 9000.0), NavBudget::FULL);
        f.update(&tiles, Vec2::new(160.0, -9000.0), NavBudget::FULL);
        assert!(f.goal_cell().is_some());
        assert!(f.dir_at(SizeClass::Small, Vec2::new(60.0, 120.0)).len() > 0.1);
    }

    #[test]
    fn no_free_goal_cell_leaves_the_field_alone() {
        // Fully walled arena: the goal has no open cell anywhere near it, so
        // the field stays empty instead of routing toward an invalid cell.
        let mut tiles = TileMap::new(4, 4, Theme::Sewer, 0);
        tiles.fill_cells(0, 0, 4, 4, TileKind::Wall(WallKind::Solid));
        tiles.rebuild_masks();
        let mut f = FlowField::new(&tiles);
        let u = f.update(&tiles, Vec2::new(32.0, 32.0), NavBudget::FULL);
        assert!(!u.changed);
        assert_eq!(f.goal_cell(), None);
        assert_eq!(f.dir_at(SizeClass::Small, Vec2::new(32.0, 32.0)), Vec2::ZERO);
    }

    #[test]
    fn recompute_only_on_cell_change() {
        let tiles = test_tiles(40, 40, &[]);
        let mut f = FlowField::new(&tiles);
        assert!(f.update(&tiles, Vec2::new(100.0, 100.0), NavBudget::FULL).changed);
        let g0 = f.goal_cell();
        let u = f.update(&tiles, Vec2::new(104.0, 102.0), NavBudget::FULL);
        assert!(!u.changed, "same cell must not recompute");
        assert_eq!(u.fine_cells, 0);
        assert_eq!(u.coarse_blocks, 0);
        assert_eq!(f.goal_cell(), g0);
        assert!(f.update(&tiles, Vec2::new(300.0, 300.0), NavBudget::FULL).changed);
        assert_ne!(f.goal_cell(), g0);
    }

    #[test]
    fn a_partial_update_keeps_the_previous_field_readable() {
        let level = Level::generate(Theme::Sewer, 11);
        let tiles = &level.tiles;
        let mut f = FlowField::new(tiles);
        f.update(tiles, level.start, NavBudget::FULL);
        let probe = level.start + Vec2::new(-90.0, 40.0);
        let before = f.dir_at(SizeClass::Small, probe);
        assert!(before.len() > 0.1, "the first field must be usable");

        // A goal two cells away, fed one cell of budget at a time: the field
        // must keep answering from the previous window while it catches up.
        let goal = level.start + Vec2::new(32.0, 0.0);
        let mut frames = 0;
        loop {
            let u = f.update(
                tiles,
                goal,
                NavBudget {
                    fine_cells: 64,
                    coarse_blocks: 64,
                },
            );
            assert!(u.fine_cells <= 64 && u.coarse_blocks <= 64, "budget overrun");
            let mid = f.dir_at(SizeClass::Small, probe);
            assert!(mid.len() > 0.1, "field went dark during a staggered update");
            frames += 1;
            if u.changed {
                break;
            }
            assert!(frames < 200, "staggered update never finished");
        }
        assert!(frames > 5, "the tiny budget should stagger the work");
    }

    #[test]
    fn the_default_budget_finishes_in_a_few_frames_without_overrunning() {
        let level = Level::generate(Theme::Kitchen, 5);
        let tiles = &level.tiles;
        let mut f = FlowField::new(tiles);
        f.update(tiles, level.start, NavBudget::FULL);
        let budget = NavBudget::default();
        let goal = level.start + Vec2::new(3.0 * TILE_F, 0.0);
        let mut frames = 0;
        loop {
            let u = f.update(tiles, goal, budget);
            assert!(u.fine_cells <= budget.fine_cells, "fine budget overrun");
            assert!(u.coarse_blocks <= budget.coarse_blocks, "coarse budget overrun");
            frames += 1;
            if u.changed {
                break;
            }
            assert!(frames < 20, "default budget needs too many frames");
        }
        assert!(frames >= 2, "the default budget should be staggered");
    }

    // ---- Size classes and directed doors ----------------------------------

    #[test]
    fn a_small_agent_uses_a_rat_hole_and_a_large_one_cannot() {
        let hole = TileKind::Wall(WallKind::RatHole {
            axis: WallAxis::Vertical,
        });
        let tiles = two_rooms(hole);
        let goal = centre(8, 12);
        let start = centre(40, 12);

        let f = solve(&tiles, goal);
        // The hole is a wall node for the large class and a route for the rat.
        assert_ne!(
            f.fine_sample(SizeClass::Small, 24, 12).map(|s| s.1),
            Some(u32::MAX)
        );
        assert!(!tiles.passable(24, 12, SizeClass::Large));

        let (end, reached) = walk(&tiles, &f, SizeClass::Small, start, goal, 2_000);
        assert!(reached, "the rat did not use its own rat-hole: {end:?}");

        let mut g = FlowField::new(&tiles);
        g.update(&tiles, goal, NavBudget::FULL);
        let (end, reached) = walk(&tiles, &g, SizeClass::Large, start, goal, 2_000);
        assert!(!reached, "a Large agent squeezed through a rat-hole at {end:?}");
        assert!(
            end.x > 24.0 * TILE_F,
            "the Large agent walked past the wall: {end:?}"
        );
    }

    #[test]
    fn a_one_way_door_routes_out_of_the_maze_but_never_into_it() {
        let door = TileKind::Wall(WallKind::OneWayDoor {
            dir: Cardinal::East,
        });
        let tiles = two_rooms(door);

        // Player outside (east): the agent west of the door exits through it.
        let outside = centre(40, 12);
        let f = solve(&tiles, outside);
        let (end, reached) =
            walk_as(&tiles, &f, SizeClass::Small, centre(8, 12), outside, 2_000, true);
        assert!(reached, "the maze exit did not let the rat out: {end:?}");

        // Player inside (west): nothing may enter the maze through the door.
        let inside = centre(8, 12);
        let g = solve(&tiles, inside);
        let (end, reached) = walk(&tiles, &g, SizeClass::Small, centre(40, 12), inside, 2_000);
        assert!(!reached, "an agent entered a maze through its exit: {end:?}");
        assert!(end.x > 24.0 * TILE_F, "agent crossed the one-way door: {end:?}");
    }

    // ---- Property tests over generated maps -------------------------------

    /// Themes and seeds the property tests sweep.
    fn seed_sweep() -> impl Iterator<Item = (Theme, u64)> {
        Theme::all()
            .into_iter()
            .flat_map(|theme| (0..6u64).map(move |seed| (theme, seed)))
    }

    /// Property over generated maps: **`dir_at` never steers a body into a cell
    /// that body cannot step into.**
    ///
    /// A direction that is a legal step (or `ZERO`, "steer straight") is the
    /// whole contract between the field and `Enemy::steer` + wall resolution:
    /// a direction into a wall means the body walks into the face, the collider
    /// pushes it straight back out and the position never changes — a pinned
    /// body. Swept over every cell of every map, both size classes, at the cell
    /// centre and at four off-centre points (the centring pull is lateral, so a
    /// body hugging a cell edge is where a bad blend shows up).
    #[test]
    fn dir_at_only_ever_points_at_a_legal_step() {
        let mut checked = 0;
        for (theme, seed) in seed_sweep() {
            let level = Level::generate(theme, seed);
            let f = solve(&level.tiles, level.start);
            for class in SizeClass::ALL {
                let ci = class.index();
                for cy in 0..level.tiles.rows() {
                    for cx in 0..level.tiles.cols() {
                        if !f.masks.open(ci, cx, cy) {
                            continue; // wall: `dir_at` escapes, checked elsewhere
                        }
                        for (fx, fy) in [(0.5f32, 0.5f32), (0.15, 0.15), (0.85, 0.85), (0.85, 0.15)] {
                            let p = Vec2::new(
                                (cx as f32 + fx) * TILE_F,
                                (cy as f32 + fy) * TILE_F,
                            );
                            let d = f.dir_at(class, p);
                            if d.len_sq() == 0.0 {
                                continue;
                            }
                            assert!(
                                (d.len() - 1.0).abs() < 1e-3,
                                "{theme:?} seed {seed} {class:?} ({cx},{cy}): not unit {d:?}"
                            );
                            let (bx, by) = (d.x.round() as i32, d.y.round() as i32);
                            assert!(
                                (bx != 0 || by != 0),
                                "{theme:?} seed {seed} {class:?} at {p:?}: direction rounds to no step"
                            );
                            assert!(
                                f.masks.can_step(ci, cx, cy, bx, by),
                                "{theme:?} seed {seed} {class:?} ({cx},{cy}) offset ({fx},{fy}): \
                                 dir {d:?} steps into ({},{}) which is not a legal step",
                                cx + bx,
                                cy + by
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 50_000, "only {checked} directions were checked");
    }

    #[test]
    fn every_routable_window_cell_keeps_stepping_downhill() {
        // Properties on every generated map:
        //  * a cell the BFS reached (and that is not the goal) has a direction;
        //  * the direction stays inside its 8-neighbourhood;
        //  * it points at a strictly cheaper cell (monotone progress is what
        //    stops an enemy ping-ponging between two equal-cost cells);
        //  * walking the field cell by cell actually arrives at the goal, for
        //    every cell in the window — the field is a complete routing
        //    function, not just a local gradient.
        let mut checked = 0;
        for (theme, seed) in seed_sweep() {
            let level = Level::generate(theme, seed);
            let f = solve(&level.tiles, level.start);
            let (gx, gy) = f.goal_cell().expect("start cell");
            for cy in gy - FINE_RADIUS_CELLS..=gy + FINE_RADIUS_CELLS {
                for cx in gx - FINE_RADIUS_CELLS..=gx + FINE_RADIUS_CELLS {
                    let Some((d, c)) = f.fine_sample(SizeClass::Small, cx, cy) else {
                        continue;
                    };
                    if c == u32::MAX || c == 0 {
                        continue;
                    }
                    assert!(
                        d.len() > 0.1,
                        "{theme:?} seed {seed}: ({cx},{cy}) cost {c} has no direction"
                    );
                    let mut at = (cx, cy);
                    let mut cost = c;
                    for _ in 0..128 {
                        if at == (gx, gy) {
                            break;
                        }
                        let (d, c) = f
                            .fine_sample(SizeClass::Small, at.0, at.1)
                            .expect("routable cell is inside the window");
                        let (tx, ty) = (at.0 + d.x.round() as i32, at.1 + d.y.round() as i32);
                        assert!(
                            (tx - at.0).abs() <= 1
                                && (ty - at.1).abs() <= 1
                                && (tx, ty) != at,
                            "{theme:?} seed {seed}: {at:?} points outside its neighbourhood: {d:?}"
                        );
                        let (_, tc) = f
                            .fine_sample(SizeClass::Small, tx, ty)
                            .expect("a downhill step stays inside the window");
                        assert!(
                            tc < c,
                            "{theme:?} seed {seed}: {at:?} cost {c} does not step down into ({tx},{ty}) cost {tc}"
                        );
                        at = (tx, ty);
                        cost = tc;
                    }
                    assert_eq!(
                        at,
                        (gx, gy),
                        "{theme:?} seed {seed}: following the field from ({cx},{cy}) (cost {cost}) never reached the goal cell"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 10_000, "only {checked} cells were checked");
    }

    #[test]
    fn both_classes_share_the_field_in_open_areas() {
        let mut shared_maps = 0;
        let mut probes = 0;
        for (theme, seed) in seed_sweep() {
            let level = Level::generate(theme, seed);
            let f = solve(&level.tiles, level.start);
            let window_has_a_gate = {
                let (gx, gy) = f.goal_cell().expect("start cell");
                let mut found = false;
                for cy in gy - FINE_RADIUS_CELLS..=gy + FINE_RADIUS_CELLS {
                    for cx in gx - FINE_RADIUS_CELLS..=gx + FINE_RADIUS_CELLS {
                        if level.tiles.one_way_dir(cx, cy).is_some()
                            || matches!(
                                level.tiles.get(cx, cy),
                                TileKind::Wall(WallKind::RatHole { .. })
                            )
                        {
                            found = true;
                        }
                    }
                }
                found
            };
            if !window_has_a_gate {
                assert!(
                    f.shared(),
                    "{theme:?} seed {seed}: identical masks over the window must share one field"
                );
                shared_maps += 1;
            }
            if f.shared() {
                let (gx, gy) = f.goal_cell().expect("start cell");
                for k in 0..64 {
                    let a = k as f32 / 64.0 * std::f32::consts::TAU;
                    let r = 32.0 + (k % 7) as f32 * 30.0;
                    let p = level.start + Vec2::from_angle(a) * r;
                    let (cx, cy) = cell_of_pos(p);
                    if !level.tiles.passable(cx, cy, SizeClass::Small) {
                        continue;
                    }
                    let ds = f.dir_at(SizeClass::Small, p);
                    let dl = f.dir_at(SizeClass::Large, p);
                    assert_eq!(
                        (ds.x.to_bits(), ds.y.to_bits()),
                        (dl.x.to_bits(), dl.y.to_bits()),
                        "{theme:?} seed {seed}: shared field disagrees at {p:?} (goal {gx},{gy})"
                    );
                    probes += 1;
                }
            }
        }
        assert!(shared_maps > 0, "no map shared its field: property never ran");
        assert!(probes > 100, "only {probes} shared-field probes");
    }

    #[test]
    fn an_open_map_always_shares_and_a_gate_never_does() {
        let tiles = test_tiles(40, 40, &[]);
        let f = solve(&tiles, Vec2::new(320.0, 320.0));
        assert!(f.shared());

        let hole = TileKind::Wall(WallKind::RatHole {
            axis: WallAxis::Horizontal,
        });
        let mut gated = tiles;
        gated.set(21, 20, hole);
        gated.rebuild_masks();
        let g = solve(&gated, Vec2::new(320.0, 320.0));
        assert!(!g.shared(), "a rat-hole in the window must split the fields");
        // ... and the classes may only differ where the gate is.
        assert!(!gated.passable(21, 20, SizeClass::Large));
        assert!(gated.passable(21, 20, SizeClass::Small));
    }

    // ---- Benchmark --------------------------------------------------------

    /// The pre-split algorithm: one full-map BFS from the goal plus a routing
    /// direction for every cell. This is the 2.6 ms baseline the plan quotes,
    /// kept here so the benchmark table stays comparable.
    fn full_map_one_class(tiles: &TileMap, goal: Vec2, class: SizeClass) -> u64 {
        let cols = tiles.cols();
        let rows = tiles.rows();
        let mut cost = vec![u32::MAX; (cols * rows) as usize];
        let idx = |cx: i32, cy: i32| (cy * cols + cx) as usize;
        let open = |cx: i32, cy: i32| tiles.passable(cx, cy, class);
        let step_ok = |cx: i32, cy: i32, dx: i32, dy: i32| match cardinal_of(dx, dy) {
            Some(dir) => {
                let nx = cx + dx;
                let ny = cy + dy;
                let enters = tiles.one_way_dir(nx, ny).map_or(open(nx, ny), |d| {
                    d == dir && open(nx, ny) && class == SizeClass::Small
                });
                let leaves = tiles.one_way_dir(cx, cy).is_none_or(|d| d == dir);
                enters && leaves
            }
            None => {
                open(cx + dx, cy + dy)
                    && tiles.one_way_dir(cx + dx, cy + dy).is_none()
                    && tiles.one_way_dir(cx, cy).is_none()
                    && (open(cx + dx, cy) || open(cx, cy + dy))
            }
        };
        let (gx, gy) = cell_of_pos(goal);
        cost[idx(gx, gy)] = 0;
        let mut queue = VecDeque::new();
        queue.push_back((gx, gy));
        while let Some((cx, cy)) = queue.pop_front() {
            let c = cost[idx(cx, cy)];
            for &(dx, dy) in OFFS.iter() {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                    continue;
                }
                if cost[idx(nx, ny)] != u32::MAX {
                    continue;
                }
                if !step_ok(nx, ny, -dx, -dy) {
                    continue;
                }
                cost[idx(nx, ny)] = c + 1;
                queue.push_back((nx, ny));
            }
        }
        let mut dir = vec![Vec2::ZERO; (cols * rows) as usize];
        for cy in 0..rows {
            for cx in 0..cols {
                let i = idx(cx, cy);
                let c = cost[i];
                if c == u32::MAX || c == 0 {
                    continue;
                }
                let mut d = Vec2::ZERO;
                let mut best: Option<(u32, Vec2)> = None;
                for &(dx, dy) in OFFS.iter() {
                    if !step_ok(cx, cy, dx, dy) {
                        continue;
                    }
                    let nc = cost[idx(cx + dx, cy + dy)];
                    if nc == u32::MAX || nc >= c {
                        continue;
                    }
                    let step = Vec2::new(dx as f32, dy as f32).normalized();
                    d += step * (c - nc) as f32;
                    if best.is_none_or(|(bc, _)| nc < bc) {
                        best = Some((nc, step));
                    }
                }
                let blended = d.normalized();
                if blended.len_sq() > 0.0
                    && cost[idx(
                        cx + blended.x.round() as i32,
                        cy + blended.y.round() as i32,
                    )] < c
                {
                    dir[i] = blended;
                } else {
                    dir[i] = best.map_or(Vec2::ZERO, |(_, s)| s);
                }
            }
        }
        // Fold the result into a value the caller keeps: the compiler may not
        // elide the work, and the checksum is stable (determinism by test).
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for d in &dir {
            hash ^= u64::from(d.x.to_bits());
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            hash ^= u64::from(d.y.to_bits());
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    #[test]
    fn nav_benchmark() {
        // A complete field always exists before each timed update, and every
        // timed call retargets it at a goal cell it can reach, so the numbers
        // are the cost of a *recompute*, not of building a `FlowField`.
        //
        // The wall-clock columns are informational (this machine builds other
        // workers' crates at the same time, so absolute times drift with load);
        // the assertions are load-independent: work counts, and ratios against
        // the full-map baseline measured in the same run.
        const REPEATS: u32 = 8;
        let mut level = Level::generate(Theme::Sewer, 42);
        let goal = level.start;
        // A second goal cell whose window the two size classes *share* (no
        // rat-hole, no one-way door in it), so the shared columns measure the
        // real thing on this map. The window is fixed-size, so the distance
        // does not matter — only that the goal cell moved.
        let mut tiles = std::mem::replace(&mut level.tiles, TileMap::new(2, 2, Theme::Sewer, 0));
        let far = {
            let mut probe = FlowField::new(&tiles);
            let mut far = goal;
            for offset in [
                Vec2::new(400.0, 300.0),
                Vec2::new(160.0, 96.0),
                Vec2::new(-240.0, 160.0),
                Vec2::new(320.0, -160.0),
                Vec2::new(-96.0, -256.0),
            ] {
                let cand = goal + offset;
                probe.update(&tiles, goal, NavBudget::FULL);
                let u = probe.update(&tiles, cand, NavBudget::FULL);
                if u.changed && u.shared {
                    far = cand;
                    break;
                }
            }
            far
        };
        assert_ne!(cell_of_pos(far), cell_of_pos(goal), "the far goal must move");

        // Baseline: the pre-split full-map field, one class, one shot.
        let mut baseline_hash = 0u64;
        let mut baseline_ms = 0.0;
        for _ in 0..REPEATS {
            let t = Instant::now();
            baseline_hash =
                baseline_hash.wrapping_add(full_map_one_class(&tiles, goal, SizeClass::Small));
            baseline_ms += t.elapsed().as_secs_f64() * 1000.0;
        }
        baseline_ms /= f64::from(REPEATS);
        assert_ne!(baseline_hash, 0, "the baseline must compute a field");

        // Fine window only (one class), then fine + coarse with both classes
        // sharing — every measurement targets the same `far` goal from a field
        // that already points at `goal`, so only the recompute is timed.
        let mut f = FlowField::new(&tiles);
        let (mut fine_only_ms, mut shared_ms) = (0.0, 0.0);
        let mut shared_work = 0u32;
        for _ in 0..REPEATS {
            f.update(&tiles, goal, NavBudget::FULL);
            let t = Instant::now();
            let fine = f.update(
                &tiles,
                far,
                NavBudget {
                    fine_cells: u32::MAX,
                    coarse_blocks: 0,
                },
            );
            fine_only_ms += t.elapsed().as_secs_f64() * 1000.0;
            assert!(fine.fine_cells > 0, "the fine-only update did no work");
            f.update(&tiles, far, NavBudget::FULL);

            f.update(&tiles, goal, NavBudget::FULL);
            let t = Instant::now();
            let u = f.update(&tiles, far, NavBudget::FULL);
            shared_ms += t.elapsed().as_secs_f64() * 1000.0;
            assert!(u.changed, "the shared update did no work");
            assert!(u.shared, "the benchmark baseline map should share");
            shared_work = shared_work.max(u.fine_cells);
        }
        fine_only_ms /= f64::from(REPEATS);
        shared_ms /= f64::from(REPEATS);

        // Both classes unshared: a rat-hole three cells from the player, so the
        // two windows differ. Same goal pair, same repeat structure.
        let (sx, sy) = cell_of_pos(goal);
        tiles.set(
            sx + 3,
            sy,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        tiles.rebuild_masks();
        let mut g = FlowField::new(&tiles);
        let (mut unshared_ms, mut unshared_work) = (0.0, 0u32);
        for _ in 0..REPEATS {
            g.update(&tiles, goal, NavBudget::FULL);
            let t = Instant::now();
            let u = g.update(&tiles, far, NavBudget::FULL);
            unshared_ms += t.elapsed().as_secs_f64() * 1000.0;
            assert!(u.changed, "the unshared update did no work");
            assert!(!u.shared, "a rat-hole in the window must not share");
            unshared_work = unshared_work.max(u.fine_cells);
        }
        unshared_ms /= f64::from(REPEATS);

        // Amortised per-frame cost at the default budget: walk the rat across
        // the map for 30 s of frames and pay whatever `update` charges.
        std::mem::swap(&mut level.tiles, &mut tiles);
        let mut walker = FlowField::new(&level.tiles);
        let budget = NavBudget::default();
        walker.update(&level.tiles, level.start, budget);
        let frames = 1_800;
        let mut worst = 0.0f64;
        let mut updates = 0;
        let mut spent_fine = 0u64;
        let t = Instant::now();
        for k in 0..frames {
            // ~75 px/s: a realistic rat speed, so the field can keep up.
            let a = k as f32 / 720.0;
            let p = level.start
                + Vec2::new(a.cos() * 900.0, (a * 0.8).sin() * 700.0).clamp_len(1_100.0);
            let one = Instant::now();
            let upd = walker.update(&level.tiles, p, budget);
            worst = worst.max(one.elapsed().as_secs_f64() * 1000.0);
            spent_fine += u64::from(upd.fine_cells);
            if upd.changed {
                updates += 1;
            }
        }
        let frame_ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);

        println!();
        println!("nav_benchmark: 200 x 160 cells, dev profile, {frames} frames");
        println!("  {:<46} {:>9}", "measurement", "cost");
        println!(
            "  {:<46} {:>9}",
            "full-map recompute, one class (baseline)",
            format!("{baseline_ms:.3} ms")
        );
        println!(
            "  {:<46} {:>9}",
            "fine window only (one class)",
            format!("{fine_only_ms:.3} ms")
        );
        println!(
            "  {:<46} {:>9}",
            "fine + coarse, both classes shared",
            format!("{shared_ms:.3} ms")
        );
        println!(
            "  {:<46} {:>9}",
            "fine + coarse, both classes unshared",
            format!("{unshared_ms:.3} ms")
        );
        println!(
            "  {:<46} {:>9}",
            format!("amortised per frame, default budget ({updates} updates)"),
            format!("{frame_ms:.3} ms")
        );
        println!(
            "  {:<46} {:>9}",
            "worst single frame, default budget",
            format!("{worst:.3} ms")
        );
        println!(
            "  {:<46} {:>9}",
            "fine cells per recompute (shared / unshared)",
            format!("{shared_work} / {unshared_work}")
        );
        println!(
            "  {:<46} {:>9}",
            format!("fine window {FINE_SIDE} x {FINE_SIDE} cells"),
            format!("{spent_fine} cells over the walk")
        );
        println!();

        assert!(
            frame_ms < 0.3,
            "amortised per-frame nav cost {frame_ms:.3} ms exceeds the 0.3 ms budget"
        );
        assert!(
            frame_ms < baseline_ms * 0.25,
            "amortised {frame_ms:.3} ms is not well under the {baseline_ms:.3} ms full-map recompute"
        );
        assert!(
            updates > 50,
            "the benchmark walk should cross many cells, only {updates}"
        );
        assert!(
            fine_only_ms < baseline_ms,
            "the fine window ({fine_only_ms:.3} ms) must beat the full map ({baseline_ms:.3} ms)"
        );
        assert!(
            shared_work < unshared_work,
            "sharing one field ({shared_work} cells) must be cheaper than two ({unshared_work})"
        );
    }
}
