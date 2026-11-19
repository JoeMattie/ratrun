//! **Stage B — mazes, rat-holes and traps.**
//!
//! Owner: *generator worker B*.
//!
//! - [`carve_mazes`] turns zero to two of the layout's reserved
//!   [`Layout::maze_slots`] into real mazes: a [`WallKind::Maze`] partition
//!   warren, a [`MazeRewardView`](crate::game::art_traps::MazeRewardView)
//!   pedestal at its end, and a [`WallKind::OneWayDoor`] as its only exit.
//! - [`place_rat_holes`] pierces structural walls with
//!   [`WallKind::RatHole`] cells: passable for the small size class only.
//! - [`place_traps`] scatters sticky traps and mousetraps in corridors and
//!   rooms (never on the spawn point, never inside a wall).
//!
//! Nothing here validates the map — [`super::validate`] does, and
//! [`super::generate`] retries when a stage produces an unplayable map.
//!
//! ## Determinism
//!
//! Every random choice is one `rng` draw in call order: no clock, no hash map,
//! no query order. Two runs of the same seed produce the same mazes, holes and
//! traps.
//!
//! ## Known constraints of the fixed pipeline signature
//!
//! The stages only receive the [`TileMap`] and the [`Layout`], so a rat-hole
//! cannot be told apart from a furnished prop by the grid alone (both are
//! [`WallKind::Solid`] cells). Stage B therefore only pierces walls **outside**
//! every room rectangle — the contract places props inside rooms and structural
//! walls between them — and only walls that are part of a solid component of at
//! least [`MIN_PIERCE_COMPONENT`] cells (a prop island is a dozen cells at
//! most). See the report for the deviation this implies.

use std::collections::VecDeque;

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
use crate::game::collision::{cardinal_delta, SizeClass, TileKind, TileMap};
use crate::game::level::{MazeSection, RoomKind, Theme, TileRect, MAP_COLS, MAP_ROWS};
use crate::math::Vec2;

use super::layout::Layout;

/// The most maze sections a map may have (`Layout::maze_slots` holds 0..=2).
pub const MAX_MAZES: usize = 2;

/// Aisle lattice spacing of a carved maze: partition cells sit one cell to the
/// side of every aisle, so a maze aisle is one open cell wide (24 px of clear
/// floor between two 8 px [`WallKind::Maze`] spines).
const AISLE_STEP: i32 = 2;

/// A solid component smaller than this is a prop/cover block, not a wall run,
/// and is never pierced. Props are at most ~10 cells; a wall run is far longer.
const MIN_PIERCE_COMPONENT: usize = 8;

/// How many rat-holes a map tries to grow (the layout may offer fewer walls).
const HOLE_COUNT_MIN: usize = 6;
const HOLE_COUNT_SPREAD: usize = 7;

/// Every corner of the map, in the fixed order all stage-B tie-breaks use.
const SIDES: [Cardinal; 4] = [
    Cardinal::North,
    Cardinal::East,
    Cardinal::South,
    Cardinal::West,
];

/// A trap the generator wants on the floor. The simulation turns these into
/// [`StickyTrapView`](crate::game::art_traps::StickyTrapView) /
/// [`MousetrapView`](crate::game::art_traps::MousetrapView) in its idle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    /// Glue board (`STICKY_TRAP_*`): slows, never damages.
    Sticky,
    /// Wooden snap trap (`MOUSETRAP_*`): warns, then damages.
    Snap,
}

/// One trap placement: world position, orientation and kind.
#[derive(Clone, Copy, Debug)]
pub struct TrapPlacement {
    pub kind: TrapKind,
    pub pos: Vec2,
    pub facing: Vec2,
}

// ---- Mazes -----------------------------------------------------------------

/// The border cells of a maze ring (the [`WallKind::Maze`] ring itself, not the
/// warren inside it).
pub fn ring_border(ring: TileRect) -> impl Iterator<Item = (i32, i32)> {
    let (x0, y0, x1, y1) = (ring.x, ring.y, ring.right(), ring.bottom());
    (y0..y1).flat_map(move |cy| {
        (x0..x1).filter_map(move |cx| {
            let edge = cx == x0 || cy == y0 || cx == x1 - 1 || cy == y1 - 1;
            edge.then_some((cx, cy))
        })
    })
}

/// The [`WallKind::Maze`] ring that encloses a maze slot.
///
/// A slot of at least 9 × 9 cells keeps a one-cell open moat between the ring
/// and the slot's edge, so the one-way door always swings onto floor that is
/// inside the reserved slot; a smaller slot is ringed on its own edge.
pub fn maze_ring(bounds: TileRect) -> TileRect {
    if bounds.w >= 9 && bounds.h >= 9 {
        bounds.inflate(-1)
    } else {
        bounds
    }
}

/// **Stage B (mazes).** Carve up to two maze sections into the layout's
/// reserved slots and return them.
///
/// Each section is a recursive-backtracker warren of [`WallKind::Maze`]
/// partitions: one open cell per aisle, one partition cell between two aisles,
/// a two-cell entry gap in the ring, the reward pedestal on the boundary aisle
/// farthest from that entry, and a [`WallKind::OneWayDoor`] beside the reward
/// as the maze's only door — pointing out of the maze.
pub fn carve_mazes(
    tiles: &mut TileMap,
    layout: &Layout,
    rng: &mut StdRng,
    theme: Theme,
) -> Vec<MazeSection> {
    // The maze structure is theme independent; the art tints the partitions.
    let _ = theme;
    let slots = layout.maze_slots.len().min(MAX_MAZES);
    if slots == 0 {
        return Vec::new();
    }
    // One maze always, a second one half the time: mazes show up in maps
    // "sometimes" without ever wasting a slot the layout reserved.
    let want = if slots == 1 {
        1
    } else {
        rng.random_range(1..=MAX_MAZES)
    };
    let mut mazes = Vec::with_capacity(want);
    for index in 0..want {
        let Some(slot) = layout.maze_slot(index) else {
            break;
        };
        if let Some(section) = carve_one(tiles, slot, rng) {
            mazes.push(section);
        }
    }
    mazes
}

/// Carve one maze into `bounds`, or leave the slot untouched and return `None`
/// when it is too small or has nowhere to put its door.
fn carve_one(tiles: &mut TileMap, bounds: TileRect, rng: &mut StdRng) -> Option<MazeSection> {
    let ring = maze_ring(bounds);
    // Never touch the two-cell map border.
    if ring.x < 2 || ring.y < 2 || ring.right() > MAP_COLS - 2 || ring.bottom() > MAP_ROWS - 2 {
        return None;
    }
    let inner = ring.inflate(-1);
    if inner.w < 3 || inner.h < 3 {
        return None;
    }
    let cols = inner.w.div_euclid(AISLE_STEP);
    let rows = inner.h.div_euclid(AISLE_STEP);
    if cols < 2 || rows < 2 {
        return None;
    }
    let node = |i: i32, j: i32| (j * cols + i) as usize;
    let aisle = |i: i32, j: i32| (inner.x + i * AISLE_STEP, inner.y + j * AISLE_STEP);

    // 1. Solid maze partitions everywhere inside the ring, then the aisle
    //    lattice opened up on the odd offsets.
    let partition = TileKind::Wall(WallKind::Maze);
    for (cx, cy) in ring_border(ring) {
        tiles.set(cx, cy, partition);
    }
    for (cx, cy) in inner.cells() {
        tiles.set(cx, cy, partition);
    }
    for j in 0..rows {
        for i in 0..cols {
            let (cx, cy) = aisle(i, j);
            tiles.set(cx, cy, TileKind::Open);
        }
    }

    // 2. Recursive-backtracker spanning tree: long winding aisles with exactly
    //    one route between any two aisle cells.
    let mut visited = vec![false; (cols * rows) as usize];
    let root = (rng.random_range(0..cols), rng.random_range(0..rows));
    visited[node(root.0, root.1)] = true;
    let mut stack = vec![root];
    while let Some(&(i, j)) = stack.last() {
        let mut options = [(0, 0); 4];
        let mut count = 0;
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= cols || nj >= rows || visited[node(ni, nj)] {
                continue;
            }
            options[count] = (ni, nj);
            count += 1;
        }
        if count == 0 {
            stack.pop();
            continue;
        }
        let (ni, nj) = options[rng.random_range(0..count)];
        // The one partition cell between the two aisles (`i + ni`, `j + nj`).
        tiles.set(
            inner.x + i + ni,
            inner.y + j + nj,
            TileKind::Open,
        );
        visited[node(ni, nj)] = true;
        stack.push((ni, nj));
    }

    // 3. A two-cell entry gap in the ring, aligned with a boundary aisle.
    let mut entry_options: Vec<(Cardinal, i32)> = Vec::new();
    for side in SIDES {
        let count = match side {
            Cardinal::North | Cardinal::South => cols,
            _ => rows,
        };
        for along in 0..count {
            let (_, ingress) = ring_cell(ring, inner, side, along);
            if tiles.get(ingress.0, ingress.1).is_open() {
                entry_options.push((side, along));
            }
        }
    }
    if entry_options.is_empty() {
        clear_rect(tiles, bounds);
        return None;
    }
    let (entry_side, entry_along) = entry_options[rng.random_range(0..entry_options.len())];
    let (entry, entry_ingress) = ring_cell(ring, inner, entry_side, entry_along);
    let entry_aisle = aisle_of(entry_side, entry_along, cols, rows);
    debug_assert_eq!(aisle(entry_aisle.0, entry_aisle.1), entry_ingress);

    let mut entry_cells = vec![entry];
    tiles.set(entry.0, entry.1, TileKind::Open);
    // The second gap cell: one ring cell to the side, whose inward neighbour
    // gives the vestibule real depth instead of a blind slot.
    let first_step = if rng.random_range(0..2) == 0 { 1 } else { -1 };
    for step in [first_step, -first_step] {
        let (gap, vestibule) = match entry_side {
            Cardinal::North => ((entry.0 + step, entry.1), (entry.0 + step, entry.1 + 1)),
            Cardinal::South => ((entry.0 + step, entry.1), (entry.0 + step, entry.1 - 1)),
            Cardinal::West => ((entry.0, entry.1 + step), (entry.0 + 1, entry.1 + step)),
            Cardinal::East => ((entry.0, entry.1 + step), (entry.0 - 1, entry.1 + step)),
        };
        if !inner.contains(vestibule.0, vestibule.1) {
            continue;
        }
        tiles.set(gap.0, gap.1, TileKind::Open);
        tiles.set(vestibule.0, vestibule.1, TileKind::Open);
        entry_cells.push(gap);
        break;
    }

    // 4. Distances over the carved lattice, so the reward can sit at the far
    //    end of the warren rather than next to the entry.
    let mut dist = vec![-1i32; (cols * rows) as usize];
    dist[node(entry_aisle.0, entry_aisle.1)] = 0;
    let mut queue = VecDeque::new();
    queue.push_back(entry_aisle);
    while let Some((i, j)) = queue.pop_front() {
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= cols || nj >= rows || dist[node(ni, nj)] >= 0 {
                continue;
            }
            if !tiles
                .get(inner.x + i + ni, inner.y + j + nj)
                .is_open()
            {
                continue;
            }
            dist[node(ni, nj)] = dist[node(i, j)] + 1;
            queue.push_back((ni, nj));
        }
    }

    // 5. The exit: the one-way door on the farthest boundary aisle that has
    //    open floor on both faces of its ring cell. Prefer a different side
    //    from the entry so the maze reads as a one-way shortcut.
    let mut best: Option<ExitSpec> = None;
    for side in SIDES {
        let count = match side {
            Cardinal::North | Cardinal::South => cols,
            _ => rows,
        };
        for along in 0..count {
            let (cell, ingress) = ring_cell(ring, inner, side, along);
            if !tiles.get(ingress.0, ingress.1).is_open() || entry_cells.contains(&cell) {
                continue;
            }
            let (dx, dy) = cardinal_delta(side);
            let outside = (cell.0 + dx, cell.1 + dy);
            if !tiles.get(outside.0, outside.1).is_open() {
                continue;
            }
            let (i, j) = aisle_of(side, along, cols, rows);
            let d = dist[node(i, j)];
            if d < 0 {
                continue;
            }
            let candidate = ExitSpec {
                rank: u8::from(side == entry_side),
                dist: d,
                dir: side,
                along,
                cell,
            };
            if best.is_none_or(|best| candidate.better_than(&best)) {
                best = Some(candidate);
            }
        }
    }
    let Some(exit_spec) = best else {
        clear_rect(tiles, bounds);
        return None;
    };
    let exit_dir = exit_spec.dir;
    let exit_along = exit_spec.along;
    let exit = exit_spec.cell;
    let (reward_i, reward_j) = aisle_of(exit_dir, exit_along, cols, rows);
    // The pedestal sits one aisle back from the door, at the true dead end.
    let (rx, ry) = aisle(reward_i, reward_j);
    let reward = TileMap::tile_rect(rx, ry).center();
    tiles.set(
        exit.0,
        exit.1,
        TileKind::Wall(WallKind::OneWayDoor { dir: exit_dir }),
    );

    Some(MazeSection {
        bounds,
        entry,
        exit,
        exit_dir,
        reward,
    })
}

/// The ring cell of `side` at aisle index `along`, plus the cell just inside
/// it (i.e. the aisle cell, when the lattice reaches the ring).
fn ring_cell(
    ring: TileRect,
    inner: TileRect,
    side: Cardinal,
    along: i32,
) -> ((i32, i32), (i32, i32)) {
    match side {
        Cardinal::North => {
            let x = inner.x + along * AISLE_STEP;
            ((x, ring.y), (x, ring.y + 1))
        }
        Cardinal::South => {
            let x = inner.x + along * AISLE_STEP;
            ((x, ring.bottom() - 1), (x, ring.bottom() - 2))
        }
        Cardinal::West => {
            let y = inner.y + along * AISLE_STEP;
            ((ring.x, y), (ring.x + 1, y))
        }
        Cardinal::East => {
            let y = inner.y + along * AISLE_STEP;
            ((ring.right() - 1, y), (ring.right() - 2, y))
        }
    }
}

/// The best one-way door found for a maze so far (see step 5 of [`carve_one`]).
#[derive(Clone, Copy, Debug)]
struct ExitSpec {
    /// `0` when the door is on a side the entry is not on (preferred).
    rank: u8,
    /// Aisle distance from the entry (the reward sits at the farthest one).
    dist: i32,
    /// Outward travel direction through the door.
    dir: Cardinal,
    /// Aisle index along that side.
    along: i32,
    /// The `OneWayDoor` ring cell.
    cell: (i32, i32),
}

impl ExitSpec {
    fn better_than(&self, other: &ExitSpec) -> bool {
        (self.rank, self.dist) > (other.rank, other.dist)
    }
}

/// Aisle lattice index reached from a ring side.
fn aisle_of(side: Cardinal, along: i32, cols: i32, rows: i32) -> (i32, i32) {
    match side {
        Cardinal::North => (along, 0),
        Cardinal::South => (along, rows - 1),
        Cardinal::West => (0, along),
        Cardinal::East => (cols - 1, along),
    }
}

/// Put a slot back the way stage A left it (open floor).
fn clear_rect(tiles: &mut TileMap, bounds: TileRect) {
    for (cx, cy) in bounds.cells() {
        tiles.set(cx, cy, TileKind::Open);
    }
}

// ---- Rat-holes -------------------------------------------------------------

/// One candidate piercing: one or two adjacent wall cells, depending on how
/// thick the wall on the passage axis is.
#[derive(Clone, Copy, Debug)]
struct Pierce {
    first: (i32, i32),
    second: Option<(i32, i32)>,
    axis: WallAxis,
}

impl Pierce {
    fn cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        [Some(self.first), self.second].into_iter().flatten()
    }
}

/// **Stage B (rat-holes).** Pierce structural walls with rat-hole cells and
/// return every rat-hole cell (with the wall axis), including any an earlier
/// stage already carved.
///
/// A hole goes into a wall that is one cell thick on its passage axis; a
/// two-cell-thick wall is pierced as a pair of aligned cells, so the 8 px
/// tunnel stays straight (`CONTENT_PLAN.md` §2). Holes never touch the border,
/// the start room, the boss arena, a room interior (props live there) or a
/// doorway, and two picks are kept apart so they never merge into a door.
pub fn place_rat_holes(
    tiles: &mut TileMap,
    layout: &Layout,
    rng: &mut StdRng,
) -> Vec<((i32, i32), WallAxis)> {
    let field = HoleField::new(tiles, layout);
    let mut candidates: Vec<Pierce> = Vec::new();
    for cy in 2..tiles.rows() - 2 {
        for cx in 2..tiles.cols() - 2 {
            if !tiles.is_full_solid(cx, cy) {
                continue;
            }
            if let Some(pierce) = pierce_at(tiles, layout, &field, cx, cy) {
                candidates.push(pierce);
            }
        }
    }

    let want = HOLE_COUNT_MIN + rng.random_range(0..=HOLE_COUNT_SPREAD);
    for _ in 0..want {
        if candidates.is_empty() {
            break;
        }
        let pierce = candidates.swap_remove(rng.random_range(0..candidates.len()));
        for (cx, cy) in pierce.cells() {
            tiles.set(
                cx,
                cy,
                TileKind::Wall(WallKind::RatHole { axis: pierce.axis }),
            );
        }
        // Keep holes apart so two picks never merge into a doorway-sized gap.
        candidates.retain(|other| {
            other.cells().all(|(ox, oy)| {
                pierce
                    .cells()
                    .all(|(px, py)| (ox - px).abs() > 2 || (oy - py).abs() > 2)
            })
        });
    }

    // Report **every** hole in the map, including the ones the layout carved
    // itself: this is the list the level turns into `RatHoleView`s.
    let mut holes: Vec<((i32, i32), WallAxis)> = tiles
        .iter_walls()
        .filter_map(|(cx, cy, kind)| match kind {
            WallKind::RatHole { axis } => Some(((cx, cy), axis)),
            _ => None,
        })
        .collect();
    holes.sort_by_key(|((cx, cy), _)| (*cy, *cx));
    holes
}

/// Can the solid cell at `(cx, cy)` carry a rat-hole? The wall must pierce a
/// one- or two-cell-thick run, with open floor at both mouths.
fn pierce_at(
    tiles: &TileMap,
    layout: &Layout,
    field: &HoleField,
    cx: i32,
    cy: i32,
) -> Option<Pierce> {
    if !hole_zone_ok(tiles, layout, field, cx, cy) {
        return None;
    }
    let north = tiles.get(cx, cy - 1);
    let south = tiles.get(cx, cy + 1);
    let west = tiles.get(cx - 1, cy);
    let east = tiles.get(cx + 1, cy);

    // A one-cell-thick east-west wall: passage north-south.
    if north.is_open() && south.is_open() && west.is_wall() && east.is_wall() {
        if mouth_ok(tiles, cx, cy - 1) && mouth_ok(tiles, cx, cy + 1) {
            return Some(Pierce {
                first: (cx, cy),
                second: None,
                axis: WallAxis::Horizontal,
            });
        }
        return None;
    }
    // A one-cell-thick north-south wall: passage east-west.
    if west.is_open() && east.is_open() && north.is_wall() && south.is_wall() {
        if mouth_ok(tiles, cx - 1, cy) && mouth_ok(tiles, cx + 1, cy) {
            return Some(Pierce {
                first: (cx, cy),
                second: None,
                axis: WallAxis::Vertical,
            });
        }
        return None;
    }
    // A two-cell-thick wall, pierced as a pair. Only the northern/western cell
    // of the pair starts a candidate, so each pair is offered once.
    if north.is_open()
        && tiles.is_full_solid(cx, cy + 1)
        && tiles.get(cx, cy + 2).is_open()
        && west.is_wall()
        && east.is_wall()
        && tiles.get(cx - 1, cy + 1).is_wall()
        && tiles.get(cx + 1, cy + 1).is_wall()
        && hole_zone_ok(tiles, layout, field, cx, cy + 1)
        && mouth_ok(tiles, cx, cy - 1)
        && mouth_ok(tiles, cx, cy + 2)
    {
        return Some(Pierce {
            first: (cx, cy),
            second: Some((cx, cy + 1)),
            axis: WallAxis::Horizontal,
        });
    }
    if west.is_open()
        && tiles.is_full_solid(cx + 1, cy)
        && tiles.get(cx + 2, cy).is_open()
        && north.is_wall()
        && south.is_wall()
        && tiles.get(cx + 1, cy - 1).is_wall()
        && tiles.get(cx + 1, cy + 1).is_wall()
        && hole_zone_ok(tiles, layout, field, cx + 1, cy)
        && mouth_ok(tiles, cx - 1, cy)
        && mouth_ok(tiles, cx + 2, cy)
    {
        return Some(Pierce {
            first: (cx, cy),
            second: Some((cx + 1, cy)),
            axis: WallAxis::Vertical,
        });
    }
    None
}

/// Places a rat-hole may never appear: the border, the start room, the boss
/// arena, any room interior (props are painted there as solid cells), and the
/// cells immediately around a doorway, door or another hole. Also requires the
/// cell to belong to a real wall run rather than a prop-sized island.
fn hole_zone_ok(tiles: &TileMap, layout: &Layout, field: &HoleField, cx: i32, cy: i32) -> bool {
    if cx < 2 || cy < 2 || cx > tiles.cols() - 3 || cy > tiles.rows() - 3 {
        return false;
    }
    let cell = TileRect::new(cx, cy, 1, 1);
    if cell.intersects(&layout.start_room.inflate(1)) || cell.intersects(&layout.boss_arena) {
        return false;
    }
    if field.in_room(cx, cy) {
        return false; // furnished room interiors are props, not walls
    }
    for dy in -1..=1 {
        for dx in -1..=1 {
            if matches!(
                tiles.get(cx + dx, cy + dy),
                TileKind::Wall(
                    WallKind::Doorway { .. } | WallKind::OneWayDoor { .. } | WallKind::RatHole { .. }
                )
            ) {
                return false;
            }
        }
    }
    field.component_at_least(tiles, cx, cy, MIN_PIERCE_COMPONENT)
}

/// Which cells may carry a rat-hole, precomputed once per map.
///
/// Stage B only sees the grid, so it tells a prop from a wall by two things it
/// can precompute in one pass: the room rectangles (props live inside rooms)
/// and the size of each 4-connected solid component (a prop island is a dozen
/// cells at most, a wall run far longer). Building both up front keeps the
/// candidate scan itself free of allocation and flood fills.
struct HoleField {
    cols: i32,
    rows: i32,
    in_room: Vec<bool>,
    /// Solid-component id per cell (`u32::MAX` for open floor).
    component: Vec<u32>,
    /// Cell count per component id.
    component_size: Vec<u32>,
}

impl HoleField {
    fn new(tiles: &TileMap, layout: &Layout) -> HoleField {
        let (cols, rows) = (tiles.cols(), tiles.rows());
        let mut in_room = vec![false; (cols * rows) as usize];
        for room in &layout.rooms {
            let rect = room.rect;
            for cy in rect.y.max(0)..rect.bottom().min(rows) {
                for cx in rect.x.max(0)..rect.right().min(cols) {
                    in_room[(cy * cols + cx) as usize] = true;
                }
            }
        }

        let mut component = vec![u32::MAX; (cols * rows) as usize];
        let mut component_size: Vec<u32> = Vec::new();
        let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
        for cy in 0..rows {
            for cx in 0..cols {
                if !tiles.is_full_solid(cx, cy) || component[(cy * cols + cx) as usize] != u32::MAX
                {
                    continue;
                }
                let id = component_size.len() as u32;
                let mut count = 0u32;
                component[(cy * cols + cx) as usize] = id;
                queue.push_back((cx, cy));
                while let Some((ax, ay)) = queue.pop_front() {
                    count += 1;
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, ny) = (ax + dx, ay + dy);
                        // Out of bounds reads as solid, so bounds first.
                        if !tiles.in_bounds(nx, ny) || !tiles.is_full_solid(nx, ny) {
                            continue;
                        }
                        let i = (ny * cols + nx) as usize;
                        if component[i] != u32::MAX {
                            continue;
                        }
                        component[i] = id;
                        queue.push_back((nx, ny));
                    }
                }
                component_size.push(count);
            }
        }

        HoleField {
            cols,
            rows,
            in_room,
            component,
            component_size,
        }
    }

    fn in_room(&self, cx: i32, cy: i32) -> bool {
        cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows && self.in_room[(cy * self.cols + cx) as usize]
    }

    fn component_at_least(&self, tiles: &TileMap, cx: i32, cy: i32, cap: usize) -> bool {
        if !tiles.in_bounds(cx, cy) {
            return false;
        }
        let id = self.component[(cy * self.cols + cx) as usize];
        id != u32::MAX && self.component_size[id as usize] as usize >= cap
    }
}

/// Is `(cx, cy)` open floor a rat can approach and turn around on?
fn mouth_ok(tiles: &TileMap, cx: i32, cy: i32) -> bool {
    if !tiles.get(cx, cy).is_open() {
        return false;
    }
    let mut open = 0;
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        if tiles.get(cx + dx, cy + dy).is_open() {
            open += 1;
        }
    }
    open >= 2
}

// ---- Traps -----------------------------------------------------------------

/// **Stage B (traps).** Scatter sticky traps and mousetraps across the map.
///
/// Traps go on open floor in halls, rooms and corridors — never in the start
/// room (or on the spawn point), never in the boss arena, and never within a
/// few cells of a maze mouth, door or reward. Maze interiors stay trap-free:
/// a maze aisle is the only route to its reward, so a trap there could not be
/// walked around. Placement reads only cell kinds (masks are still stale at
/// this point in the pipeline).
pub fn place_traps(
    tiles: &TileMap,
    layout: &Layout,
    mazes: &[MazeSection],
    rng: &mut StdRng,
) -> Vec<TrapPlacement> {
    /// Hard cap so a huge map cannot end up as a carpet of traps.
    const MAX_TRAPS: usize = 24;
    let keep_clear = layout.start_room.inflate(6);
    let mut protected: Vec<(i32, i32)> = Vec::new();
    for maze in mazes {
        protected.push(maze.entry);
        protected.push(maze.exit);
        protected.push(super::validate::cell_of(maze.reward));
    }

    let mut traps: Vec<TrapPlacement> = Vec::new();
    let mut taken: Vec<(i32, i32)> = Vec::new();
    for room in &layout.rooms {
        if !matches!(
            room.kind,
            RoomKind::Hall | RoomKind::Room | RoomKind::Corridor
        ) {
            continue;
        }
        let inner = room.rect.inflate(-1);
        if inner.w < 1 || inner.h < 1 {
            continue;
        }
        let cap = match room.kind {
            RoomKind::Corridor => 2,
            _ => 4,
        };
        // A corridor is only a cell or two wide, so its area is small; it still
        // gets one trap once it is long enough to be worth walking down.
        let floor = i32::from(inner.area() >= 10);
        let want = (inner.area() / 120).clamp(floor, cap) as usize;
        let mut placed = 0;
        for _ in 0..want * 8 {
            if placed >= want || traps.len() >= MAX_TRAPS {
                break;
            }
            let cx = rng.random_range(inner.x..inner.right());
            let cy = rng.random_range(inner.y..inner.bottom());
            let pos = TileMap::tile_rect(cx, cy).center();
            if !trap_cell_ok(tiles, (cx, cy), &keep_clear, &protected, &taken) {
                continue;
            }
            let facing = trap_facing(tiles, cx, cy, rng);
            let kind = if rng.random_range(0..100u32) < 55 {
                TrapKind::Sticky
            } else {
                TrapKind::Snap
            };
            traps.push(TrapPlacement { kind, pos, facing });
            taken.push((cx, cy));
            placed += 1;
        }
    }
    traps
}

/// Is this cell a fair trap spot?
fn trap_cell_ok(
    tiles: &TileMap,
    cell: (i32, i32),
    keep_clear: &TileRect,
    protected: &[(i32, i32)],
    taken: &[(i32, i32)],
) -> bool {
    let (cx, cy) = cell;
    if !tiles.get(cx, cy).is_open() || keep_clear.contains_world(TileMap::tile_rect(cx, cy).center())
    {
        return false;
    }
    if protected.iter().any(|&(px, py)| (px - cx).abs() <= 3 && (py - cy).abs() <= 3)
        || taken
            .iter()
            .any(|&(tx, ty)| (tx - cx).abs() <= 3 && (ty - cy).abs() <= 3)
    {
        return false;
    }
    let mut open = 0;
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let kind = tiles.get(cx + dx, cy + dy);
        if matches!(
            kind,
            TileKind::Wall(
                WallKind::Doorway { .. } | WallKind::OneWayDoor { .. } | WallKind::RatHole { .. }
            )
        ) {
            return false; // never block a threshold
        }
        if kind.is_open() {
            open += 1;
        }
    }
    // Room to walk around the hazard instead of over it.
    open >= 2
}

/// Which way the trap's long axis points: along the corridor when it is in one,
/// at a random open side when it is in the open.
fn trap_facing(tiles: &TileMap, cx: i32, cy: i32, rng: &mut StdRng) -> Vec2 {
    let north_south = tiles.get(cx, cy - 1).is_open() && tiles.get(cx, cy + 1).is_open();
    let east_west = tiles.get(cx - 1, cy).is_open() && tiles.get(cx + 1, cy).is_open();
    if north_south && !east_west {
        return Vec2::new(0.0, if rng.random_range(0..2) == 0 { 1.0 } else { -1.0 });
    }
    if east_west && !north_south {
        return Vec2::new(if rng.random_range(0..2) == 0 { 1.0 } else { -1.0 }, 0.0);
    }
    let mut options = [(0.0f32, 0.0f32); 4];
    let mut count = 0;
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        if tiles.get(cx + dx, cy + dy).is_open() {
            options[count] = (dx as f32, dy as f32);
            count += 1;
        }
    }
    if count == 0 {
        return Vec2::new(1.0, 0.0);
    }
    let (dx, dy) = options[rng.random_range(0..count)];
    Vec2::new(dx, dy)
}

// ---- Helpers ---------------------------------------------------------------

/// Cells of a rectangle that are open floor for `class` (stage-B helpers and
/// tests).
pub fn open_cells(tiles: &TileMap, rect: TileRect, class: SizeClass) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for cy in rect.y.max(0)..rect.bottom().min(MAP_ROWS) {
        for cx in rect.x.max(0)..rect.right().min(MAP_COLS) {
            if tiles.passable(cx, cy, class) {
                out.push((cx, cy));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TILE_F;
    use crate::game::level::{level_rng, Level, Room, MAP_SIZE};
    use crate::game::levelgen::{generate_once, validate};

    /// Seeds used by the property-style sweeps: three themes × these seeds.
    const SEEDS: [u64; 8] = [0, 1, 2, 3, 7, 11, 42, 99];

    fn generated() -> impl Iterator<Item = Level> {
        Theme::all()
            .into_iter()
            .flat_map(|theme| SEEDS.map(move |seed| (theme, seed)))
            .map(|(theme, seed)| generate_once(theme, seed, 0))
    }

    fn layout_with(slots: Vec<TileRect>) -> Layout {
        Layout {
            rooms: Vec::new(),
            start: MAP_SIZE * 0.5,
            start_room: TileRect::new(MAP_COLS / 2, MAP_ROWS / 2, 8, 8),
            boss_arena: TileRect::new(20, 20, 20, 20),
            maze_slots: slots,
        }
    }

    #[test]
    fn every_map_has_zero_to_two_playable_mazes() {
        for level in generated() {
            let theme = level.theme;
            assert!(
                level.mazes.len() <= MAX_MAZES,
                "{theme:?}: {} mazes",
                level.mazes.len()
            );
            // One reward and one door object per section, in generation order.
            assert_eq!(level.objects.rewards.len(), level.mazes.len());
            assert_eq!(level.objects.one_way_doors.len(), level.mazes.len());
            for (i, maze) in level.mazes.iter().enumerate() {
                assert!(maze.bounds.w >= 5 && maze.bounds.h >= 5, "{theme:?}: {maze:?}");
                assert!(
                    maze.bounds.contains(maze.entry.0, maze.entry.1),
                    "{theme:?} maze {i}: entry {:?} outside {:?}",
                    maze.entry,
                    maze.bounds
                );
                assert!(maze.bounds.contains(maze.exit.0, maze.exit.1));
                assert_eq!(
                    level.tiles.one_way_dir(maze.exit.0, maze.exit.1),
                    Some(maze.exit_dir),
                    "{theme:?} maze {i}: exit is not the door the section claims"
                );
                // The pedestal is real floor, reachable in the maze.
                let reward = validate::cell_of(maze.reward);
                assert!(level.tiles.passable(reward.0, reward.1, SizeClass::Small));
                assert!(maze.bounds.contains(reward.0, reward.1));
                assert_eq!(level.objects.rewards[i].pos, maze.reward);
                // The ring is closed except the entry gap and the one-way door.
                let ring = maze_ring(maze.bounds);
                let mut open = Vec::new();
                let mut doors = Vec::new();
                for (cx, cy) in ring_border(ring) {
                    if level.tiles.get(cx, cy).is_open() {
                        open.push((cx, cy));
                    }
                    if level.tiles.one_way_dir(cx, cy).is_some() {
                        doors.push((cx, cy));
                    }
                }
                assert_eq!(doors, vec![maze.exit], "{theme:?} maze {i}: extra door");
                assert!(
                    (1..=2).contains(&open.len()),
                    "{theme:?} maze {i}: {} open ring cells",
                    open.len()
                );
                for cell in open {
                    assert!(
                        (cell.0 - maze.entry.0).abs() <= 1 && (cell.1 - maze.entry.1).abs() <= 1,
                        "{theme:?} maze {i}: the ring leaks at {cell:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_maze_is_reachable_and_its_door_leads_out() {
        for level in generated() {
            for (i, maze) in level.mazes.iter().enumerate() {
                let seen = validate::reachable(&level.tiles, validate::cell_of(level.start), SizeClass::Small);
                let reward = validate::cell_of(maze.reward);
                assert!(
                    validate::is_reached(&seen, &level.tiles, reward),
                    "{:?} maze {i}: the player cannot reach the reward",
                    level.theme
                );
                let (dx, dy) = cardinal_delta(maze.exit_dir);
                let outside = (maze.exit.0 + dx, maze.exit.1 + dy);
                assert!(
                    validate::is_reached(&seen, &level.tiles, outside),
                    "{:?} maze {i}: the door opens onto nothing",
                    level.theme
                );
            }
        }
    }

    #[test]
    fn a_slot_too_small_to_hold_a_maze_is_left_alone() {
        let theme = Theme::Sewer;
        let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, 1);
        let mut rng = level_rng(theme, 5, 0);
        let slot = TileRect::new(40, 40, 4, 4);
        let layout = layout_with(vec![slot]);
        let mazes = carve_mazes(&mut tiles, &layout, &mut rng, theme);
        assert!(mazes.is_empty(), "a 4 × 4 slot cannot hold a maze");
        assert!(slot.cells().all(|(cx, cy)| tiles.get(cx, cy).is_open()));
    }

    #[test]
    fn every_rat_hole_pierces_a_wall_and_the_large_class_cannot_follow() {
        let mut total = 0;
        for level in generated() {
            let theme = level.theme;
            for hole in &level.objects.rat_holes {
                total += 1;
                let TileKind::Wall(WallKind::RatHole { axis }) = level.tiles.get(hole.cell.0, hole.cell.1)
                else {
                    panic!("{theme:?}: rat-hole {:?} is not a rat-hole cell", hole.cell);
                };
                // The view faces along the passage, the axis names the wall.
                let along_y = hole.view.facing.y.abs() >= hole.view.facing.x.abs();
                assert_eq!(
                    axis,
                    if along_y {
                        WallAxis::Horizontal
                    } else {
                        WallAxis::Vertical
                    },
                    "{theme:?}: rat-hole {:?} faces across its wall",
                    hole.cell
                );
                // Small gets through, large is stopped, and both ends of the
                // tunnel are open floor.
                assert!(level.tiles.passable(hole.cell.0, hole.cell.1, SizeClass::Small));
                assert!(!level.tiles.passable(hole.cell.0, hole.cell.1, SizeClass::Large));
                let step = if along_y {
                    (0, hole.view.facing.y.signum() as i32)
                } else {
                    (hole.view.facing.x.signum() as i32, 0)
                };
                for sign in [1, -1] {
                    let mut cell = hole.cell;
                    for _ in 0..3 {
                        cell = (cell.0 + sign * step.0, cell.1 + sign * step.1);
                        if !matches!(
                            level.tiles.get(cell.0, cell.1),
                            TileKind::Wall(WallKind::RatHole { .. })
                        ) {
                            break;
                        }
                    }
                    assert!(
                        level.tiles.get(cell.0, cell.1).is_open(),
                        "{theme:?}: rat-hole {:?} opens into a wall at {cell:?}",
                        hole.cell
                    );
                }
            }
        }
        assert!(total > 0, "no generated map had a single rat-hole");
    }

    #[test]
    fn a_two_cell_wall_is_pierced_as_an_aligned_pair() {
        let theme = Theme::Lab;
        let mut made = 0;
        for seed in 0..8u64 {
            let mut tiles = TileMap::new(60, 24, theme, seed as u32);
            let solid = TileKind::Wall(WallKind::Solid);
            tiles.fill_cells(0, 0, 60, 2, solid);
            tiles.fill_cells(0, 22, 60, 2, solid);
            tiles.fill_cells(0, 0, 2, 24, solid);
            tiles.fill_cells(58, 0, 2, 24, solid);
            // A two-cell-thick east-west wall right across the map.
            tiles.fill_cells(2, 10, 56, 2, solid);
            tiles.rebuild_masks();

            let mut rng = level_rng(theme, seed, 0);
            let layout = layout_with(Vec::new());
            let holes = place_rat_holes(&mut tiles, &layout, &mut rng);
            let small = validate::reachable(&tiles, (5, 5), SizeClass::Small);
            let large = validate::reachable(&tiles, (5, 5), SizeClass::Large);
            if holes.is_empty() {
                assert!(
                    !validate::is_reached(&small, &tiles, (5, 18)),
                    "seed {seed}: nobody crossed a wall without a hole"
                );
                continue;
            }
            made += 1;
            assert!(
                validate::is_reached(&small, &tiles, (5, 18)),
                "seed {seed}: the rat cannot use its hole"
            );
            assert!(
                !validate::is_reached(&large, &tiles, (5, 18)),
                "seed {seed}: a large agent squeezed through a rat-hole"
            );
            // Every hole in the two-cell wall is part of an aligned pair.
            for &((cx, cy), axis) in &holes {
                assert_eq!(axis, WallAxis::Horizontal);
                let partner = if matches!(
                    tiles.get(cx, cy + 1),
                    TileKind::Wall(WallKind::RatHole { axis: WallAxis::Horizontal })
                ) {
                    (cx, cy + 1)
                } else {
                    (cx, cy - 1)
                };
                assert!(
                    matches!(
                        tiles.get(partner.0, partner.1),
                        TileKind::Wall(WallKind::RatHole { axis: WallAxis::Horizontal })
                    ),
                    "seed {seed}: hole ({cx}, {cy}) is not aligned with a partner cell"
                );
            }
        }
        assert!(made > 0, "no seed grew a hole in a two-cell wall");
    }

    #[test]
    fn traps_stay_on_open_floor_away_from_the_spawn() {
        for level in generated() {
            let theme = level.theme;
            let mut traps = 0;
            for pos in level
                .objects
                .sticky_traps
                .iter()
                .map(|t| t.pos)
                .chain(level.objects.mousetraps.iter().map(|t| t.pos))
            {
                traps += 1;
                let cell = validate::cell_of(pos);
                assert!(
                    level.tiles.get(cell.0, cell.1).is_open(),
                    "{theme:?}: trap at {pos:?} is not on open floor"
                );
                assert!(
                    (pos - level.start).len() > 6.0 * TILE_F,
                    "{theme:?}: trap at {pos:?} sits on the spawn"
                );
                assert!(
                    level.start_room().is_none_or(|room| !room.rect.contains(cell.0, cell.1)),
                    "{theme:?}: trap at {pos:?} is inside the start room"
                );
                for maze in &level.mazes {
                    assert!(
                        !maze.bounds.contains(cell.0, cell.1),
                        "{theme:?}: trap at {pos:?} blocks a maze route"
                    );
                }
            }
            // Traps are only placed in halls, rooms and corridors, so a map
            // that has one of those must have grown at least one trap.
            let has_room = level.rooms.iter().any(|room| {
                matches!(
                    room.kind,
                    RoomKind::Hall | RoomKind::Room | RoomKind::Corridor
                ) && room.rect.area() >= 200
            });
            if has_room {
                assert!(traps > 0, "{theme:?}: no traps on a furnished map");
            }
        }
    }

    #[test]
    fn holes_never_sit_in_the_border_the_start_room_or_the_boss_arena() {
        for level in generated() {
            let theme = level.theme;
            for hole in &level.objects.rat_holes {
                let (cx, cy) = hole.cell;
                let cell = TileRect::new(cx, cy, 1, 1);
                assert!(
                    !cell.intersects(&level.start_room().expect("start room").rect.inflate(1)),
                    "{theme:?}: hole {cell:?} is in the start room"
                );
                assert!(
                    !cell.intersects(&level.boss_room().expect("boss room").rect),
                    "{theme:?}: hole {cell:?} is in the boss arena"
                );
                assert!(
                    cx >= 2 && cy >= 2 && cx <= MAP_COLS - 3 && cy <= MAP_ROWS - 3,
                    "{theme:?}: hole {cell:?} is in the border"
                );
            }
        }
    }

    #[test]
    fn a_hole_zone_excludes_borders_rooms_and_prop_islands() {
        let theme = Theme::Sewer;
        let mut tiles = TileMap::new(40, 30, theme, 1);
        let solid = TileKind::Wall(WallKind::Solid);
        // A long east-west run: pierceable.
        tiles.fill_cells(2, 10, 36, 1, solid);
        // A three-cell prop island inside the hall: solid, but not a wall.
        tiles.fill_cells(10, 20, 3, 1, solid);
        let mut layout = layout_with(Vec::new());
        layout.rooms.push(Room::new(TileRect::new(6, 16, 20, 12), RoomKind::Hall));
        let field = HoleField::new(&tiles, &layout);
        assert!(
            hole_zone_ok(&tiles, &layout, &field, 20, 10),
            "a long run outside every room must be pierceable"
        );
        assert!(!hole_zone_ok(&tiles, &layout, &field, 1, 10), "the border is out");
        assert!(
            !hole_zone_ok(&tiles, &layout, &field, 11, 20),
            "a cell inside a room rect is a prop, not a wall"
        );
        assert!(
            !field.component_at_least(&tiles, 11, 20, MIN_PIERCE_COMPONENT),
            "a three-cell prop island is not a wall run"
        );
        assert!(field.component_at_least(&tiles, 20, 10, MIN_PIERCE_COMPONENT));
    }

    #[test]
    fn a_seed_grows_the_same_map_twice() {
        for theme in Theme::all() {
            for seed in [0u64, 3, 77] {
                let a = generate_once(theme, seed, 0);
                let b = generate_once(theme, seed, 0);
                assert_eq!(a.mazes.len(), b.mazes.len(), "{theme:?} seed {seed}");
                for (x, y) in a.mazes.iter().zip(&b.mazes) {
                    assert_eq!(x.entry, y.entry);
                    assert_eq!(x.exit, y.exit);
                    assert_eq!(x.exit_dir, y.exit_dir);
                    assert_eq!(x.reward, y.reward);
                }
                assert_eq!(a.objects.rat_holes.len(), b.objects.rat_holes.len());
                for (x, y) in a.objects.rat_holes.iter().zip(&b.objects.rat_holes) {
                    assert_eq!(x.cell, y.cell);
                }
                let cell = |m: &Level, cx: i32, cy: i32| m.tiles.get(cx, cy);
                for cy in 0..MAP_ROWS {
                    for cx in 0..MAP_COLS {
                        assert_eq!(cell(&a, cx, cy), cell(&b, cx, cy), "{theme:?} {cx},{cy}");
                    }
                }
            }
        }
    }

    #[test]
    fn open_cells_lists_exactly_the_passable_cells() {
        let level = generate_once(Theme::Sewer, 5, 0);
        let rect = level.boss_room().expect("boss arena").rect;
        let small = open_cells(&level.tiles, rect, SizeClass::Small);
        let large = open_cells(&level.tiles, rect, SizeClass::Large);
        assert_eq!(small.len(), rect.area() as usize, "the boss arena is open");
        assert_eq!(large.len(), small.len());
        assert!(small.iter().all(|(cx, cy)| level.tiles.passable(*cx, *cy, SizeClass::Small)));
    }
}
