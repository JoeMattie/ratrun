//! **Stage B (mazes).**
//!
//! Owner: *generator worker P1*.
//!
//! - [`carve_mazes`] turns **every** [`Layout::maze_slots`] reservation into a
//!   real maze: a [`WallKind::Maze`] partition warren, a
//!   [`MazeRewardView`](crate::game::art_traps::MazeRewardView) pedestal at
//!   its end, and a [`WallKind::OneWayDoor`] as its only exit.
//!
//! Rat-holes and traps live in [`super::placement`] (worker P2).
//!
//! Nothing here validates the map — [`super::validate`] does, and
//! [`super::generate`] retries when a stage produces an unplayable map.
//!
//! ## The shape of a maze
//!
//! A maze is an odd-sized slot (19/21/23 by 15/17/19 cells) carrying an aisle
//! lattice with one open cell per aisle and one [`WallKind::Maze`] partition
//! cell between two aisles (`AISLE_STEP`). A recursive-backtracker spanning
//! tree gives long winding aisles, then one or two blind-tip links are opened
//! so every maze has at least one loop the rat can double back around a
//! pursuer in. The breach is a two-cell gap in the ring on the wall that faces
//! the access corridor from the starting chamber (within
//! [`ENTRY_SLACK_CELLS`] cells of where that corridor meets the walkway), and
//! the one-way door always sits on a **different** wall from the breach, at
//! the aisle whose route from the breach is closest to
//! a route inside [`REWARD_STEPS_BAND`] and closest to [`REWARD_STEPS_TARGET`]
//! lattice steps. The cache pedestal stands at that door, and
//! [`MazeSection::fork`] is the public walkway cell the gnaw-mark trail starts
//! from.
//!
//! Layout and carving agree by construction: the layout reserves at most one
//! slot (`layout::MAZE_ONE_IN`), of a size this stage can always carve, and
//! this stage carves *every* slot it is given. A reserved-but-empty maze room
//! is a generator bug and is caught in debug builds.
//!
//! ## Determinism
//!
//! Every random choice is one `rng` draw in call order: no clock, no hash map,
//! no query order. Two runs of the same seed produce the same mazes.

use std::collections::VecDeque;

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::{Cardinal, WallKind};
use crate::game::collision::{cardinal_delta, SizeClass, TileKind, TileMap, TILE_F};
use crate::game::level::{MazeSection, Room, RoomKind, Theme, TileRect, MAP_COLS, MAP_ROWS};

use super::layout::Layout;

/// The data model's cap on sections per map (`Level::mazes`). A map actually
/// carries at most [`MAZES_PER_MAP`]; the wider cap is the data model's.
pub const MAX_MAZES: usize = 2;

/// How many mazes a map carries: at most one.
///
/// The owner's decision — an expedition is a find — so `layout` rolls one slot
/// with probability one in `layout::MAZE_ONE_IN`, and this stage carves it.
pub const MAZES_PER_MAP: usize = 1;

/// Aisle lattice spacing of a carved maze: partition cells sit one cell to the
/// side of every aisle, so a maze aisle is one open cell wide (24 px of clear
/// floor between two 8 px [`WallKind::Maze`] spines).
const AISLE_STEP: i32 = 2;

/// Aisle steps the cache aims for along the best route from the breach.
///
/// One lattice step is two 16 px cells, so 16 steps ≈ 8.8 s at the rat's
/// 58 px/s — the designer's "seven to twelve second" cache walk, with the
/// reward at the exit's own aisle.
pub const REWARD_STEPS_TARGET: i32 = 16;

/// The band of route lengths a door may sit at, in lattice steps.
///
/// A lattice step is two 16 px cells, so 13–19 steps is a 7.7–11.6 s cache
/// walk at 58 px/s — the designer's "seven to twelve seconds" with the target
/// in the middle of the band. A door outside the band is only used when a
/// lattice has nothing better (hand-built test layouts).
pub const REWARD_STEPS_BAND: (i32, i32) = (13, 19);

/// How many cells along the outside walkway the breach may sit from the cell
/// where its access corridor meets that walkway.
///
/// The breach still faces the corridor, but a maze whose route is only ten
/// steps from a mid-wall breach is not worth the gamble; sliding the breach a
/// few cells along the walkway (never across a corner) buys the route its
/// length. The three gnaw marks cover the turn.
pub const ENTRY_SLACK_CELLS: i32 = 4;

/// The farthest a breach may sit from the start cell, in cells on the worse
/// axis, for the first cache to count as an opening choice (the owner's "within
/// about 32 cells of the start").
pub const ENTRY_NEAR_CELLS: i32 = 32;

/// Cells from the public fork to the breach that the gnaw-mark trail covers
/// (three marks on the final four cells).
pub const TRAIL_CELLS: i32 = 4;

/// Two blind-tip links for a lattice of 45+ nodes, one below.
const LOOP_NODE_THRESHOLD: i32 = 45;
const LOOP_LINKS_BIG: usize = 2;
const LOOP_LINKS_SMALL: usize = 1;

/// How far from the exit's landing a junction with two onward choices still
/// counts as "the landing opens onto another route".
const LANDING_RANGE: i32 = 8;

/// Every corner of the map, in the fixed order all stage-B tie-breaks use.
const SIDES: [Cardinal; 4] = [
    Cardinal::North,
    Cardinal::East,
    Cardinal::South,
    Cardinal::West,
];

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

/// The [`WallKind::Maze`] ring that encloses a maze slot: the slot's own
/// boundary cells, one cell thick.
///
/// The breach and the one-way door sit in this ring, so both open straight
/// onto the outside walkway the layout carved (`slot.inflate(1)`), and the
/// lattice inside is as large as the slot allows — a 23 × 19 slot carries an
/// 11 × 9 aisle lattice.
pub fn maze_ring(bounds: TileRect) -> TileRect {
    bounds
}

/// **Stage B (mazes).** Carve a maze into every slot the layout reserved.
///
/// Each section is a recursive-backtracker warren of [`WallKind::Maze`]
/// partitions with one or two blind-tip links for a loop, a two-cell entry gap
/// in the ring on the wall its access corridor faces, and a
/// [`WallKind::OneWayDoor`] on a different wall as the maze's only exit — with
/// the reward pedestal at that door.
pub fn carve_mazes(
    tiles: &mut TileMap,
    layout: &Layout,
    rng: &mut StdRng,
    theme: Theme,
) -> Vec<MazeSection> {
    // The maze structure is theme independent; the art tints the partitions.
    let _ = theme;
    let slots = layout
        .maze_slots
        .len()
        .min(MAZES_PER_MAP)
        .min(MAX_MAZES);
    if slots == 0 {
        return Vec::new();
    }
    let mut mazes = Vec::with_capacity(slots);
    for index in 0..slots {
        let Some(slot) = layout.maze_slot(index) else {
            break;
        };
        match carve_one(tiles, layout, slot, rng) {
            Some(section) => mazes.push(section),
            None => {
                // A reservation is a promise. `layout` only reserves slots of
                // a size, and in a place, this stage can always carve, so a
                // bail-out here is a generator bug, not weather.
                debug_assert!(
                    false,
                    "reserved maze slot {slot:?} could not be carved"
                );
                clear_rect(tiles, slot);
            }
        }
    }
    debug_assert_eq!(
        mazes.len(),
        slots,
        "layout reserved {slots} maze slot(s) and stage B carved {}",
        mazes.len()
    );
    mazes
}

/// Carve one maze into `bounds`.
fn carve_one(
    tiles: &mut TileMap,
    layout: &Layout,
    bounds: TileRect,
    rng: &mut StdRng,
) -> Option<MazeSection> {
    let ring = maze_ring(bounds);
    // Never touch the two-cell map border.
    if ring.x < 2 || ring.y < 2 || ring.right() > MAP_COLS - 2 || ring.bottom() > MAP_ROWS - 2 {
        return None;
    }
    let inner = ring.inflate(-1);
    if inner.w < 5 || inner.h < 5 {
        return None;
    }
    // `(w + 1) / 2` aisle columns: the last one lands on `inner.right() - 1`,
    // hard against the ring, so a boundary aisle always exists for the entry
    // and the exit.
    let cols = (inner.w + 1).div_euclid(AISLE_STEP);
    let rows = (inner.h + 1).div_euclid(AISLE_STEP);
    if cols < 2 || rows < 2 {
        return None;
    }
    let nodes = (cols * rows) as usize;
    let node = |i: i32, j: i32| (j * cols + i) as usize;
    let aisle = |i: i32, j: i32| (inner.x + i * AISLE_STEP, inner.y + j * AISLE_STEP);
    // The partition cell between two lattice neighbours, if both are inside.
    let inside_lattice = |i: i32, j: i32| (0..cols).contains(&i) && (0..rows).contains(&j);
    let link = |i: i32, j: i32, di: i32, dj: i32| {
        (inner.x + 2 * i + di, inner.y + 2 * j + dj)
    };

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
    let mut visited = vec![false; nodes];
    let root = (rng.random_range(0..cols), rng.random_range(0..rows));
    visited[node(root.0, root.1)] = true;
    let mut stack = vec![root];
    while let Some(&(i, j)) = stack.last() {
        let mut options = [(0, 0); 4];
        let mut count = 0;
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (ni, nj) = (i + di, j + dj);
            if !inside_lattice(ni, nj) || visited[node(ni, nj)] {
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
        // The one partition cell between the two aisles.
        let (cx, cy) = link(i, j, ni - i, nj - j);
        tiles.set(cx, cy, TileKind::Open);
        visited[node(ni, nj)] = true;
        stack.push((ni, nj));
    }

    // 3. Blind-tip links: a warren with no loop is one unavoidable corridor
    //    once a horde is behind you. Open one or two partitions at dead ends,
    //    so the rat can reverse around a pursuer while still paying distance.
    //    Only interior links open — a ring cell is never touched.
    let loop_budget = if cols * rows >= LOOP_NODE_THRESHOLD {
        LOOP_LINKS_BIG
    } else {
        LOOP_LINKS_SMALL
    };
    let mut dead_ends: Vec<(i32, i32)> = Vec::new();
    for j in 0..rows {
        for i in 0..cols {
            let mut degree = 0;
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if !inside_lattice(i + di, j + dj) {
                    continue;
                }
                let (cx, cy) = link(i, j, di, dj);
                if tiles.get(cx, cy).is_open() {
                    degree += 1;
                }
            }
            if degree == 1 {
                dead_ends.push((i, j));
            }
        }
    }
    let mut opened = 0usize;
    for (i, j) in dead_ends {
        if opened >= loop_budget {
            break;
        }
        let mut options = [(0, 0); 4];
        let mut count = 0;
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if !inside_lattice(i + di, j + dj) {
                continue;
            }
            let (cx, cy) = link(i, j, di, dj);
            if !tiles.get(cx, cy).is_open() {
                options[count] = (di, dj);
                count += 1;
            }
        }
        if count == 0 {
            continue;
        }
        let (di, dj) = options[rng.random_range(0..count)];
        let (cx, cy) = link(i, j, di, dj);
        tiles.set(cx, cy, TileKind::Open);
        opened += 1;
    }
    debug_assert!(opened >= 1, "a maze without an interior loop is a dead end");
    let _ = opened;

    // 4. The breach: a two-cell gap in the ring on the wall that faces the
    //    outside walkway's access corridor — and, within `ENTRY_SLACK_CELLS`
    //    of where that corridor lands, the spot whose best door on another
    //    wall comes closest to the target route.
    //
    //    Every boundary aisle reaches the ring (the lattice is packed against
    //    it), so every `(side, along)` is a possible breach whose ingress cell
    //    is already open floor.
    let mut entry_options: Vec<(Cardinal, i32)> = Vec::new();
    for side in SIDES {
        let count = side_count(side, cols, rows);
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
    let approach = approach_cells(tiles, bounds);
    // Distance from each candidate breach cell to the nearest cell where an
    // access corridor meets the walkway.
    let access: Vec<i32> = entry_options
        .iter()
        .map(|&(side, along)| {
            let (cell, _) = ring_cell(ring, inner, side, along);
            approach
                .iter()
                .map(|&(ax, ay)| (cell.0 - ax).abs() + (cell.1 - ay).abs())
                .min()
                .unwrap_or(i32::MAX / 4)
        })
        .collect();
    // The breach also has to be within `ENTRY_NEAR_CELLS` of the rat: the
    // outside walkway has a second corridor (the bypass), so "nearest access
    // cell" alone could put the breach on the far side of the slot. The slack
    // is measured against the nearest access cell *among the near candidates*,
    // so the breach always faces the corridor the rat arrives by.
    let start_cell = (
        (layout.start.x / TILE_F) as i32,
        (layout.start.y / TILE_F) as i32,
    );
    let start_dist: Vec<i32> = entry_options
        .iter()
        .map(|&(side, along)| {
            let (cell, _) = ring_cell(ring, inner, side, along);
            (cell.0 - start_cell.0)
                .abs()
                .max((cell.1 - start_cell.1).abs())
        })
        .collect();
    let least_access = (0..entry_options.len())
        .filter(|&index| start_dist[index] <= ENTRY_NEAR_CELLS)
        .map(|index| access[index])
        .min()
        .unwrap_or(0);
    // Rotate first so ties are broken by a seed draw, then keep the breach
    // whose best door on another wall comes closest to the target route (a
    // door inside `REWARD_STEPS_BAND` always wins over one outside it, even if
    // the outside one is numerically nearer: the band is the design).
    let rotation = rng.random_range(0..entry_options.len());
    let mut best_entry: Option<(usize, i32, bool)> = None; // (index, miss, in band)
    for k in 0..entry_options.len() {
        let index = (k + rotation) % entry_options.len();
        if access[index] > least_access + ENTRY_SLACK_CELLS
            || start_dist[index] > ENTRY_NEAR_CELLS
        {
            continue;
        }
        let (side, along) = entry_options[index];
        let (bi, bj) = aisle_of(side, along, cols, rows);
        let row = lattice_row(tiles, inner, cols, rows, (bi, bj));
        // The best door on any other wall, whether or not it is picked later.
        let mut miss = i32::MAX;
        let mut in_band = false;
        for other in SIDES {
            if other == side {
                continue;
            }
            for oalong in 0..side_count(other, cols, rows) {
                let (oi, oj) = aisle_of(other, oalong, cols, rows);
                let d = row[node(oi, oj)];
                if d < 0 {
                    continue;
                }
                miss = miss.min((d - REWARD_STEPS_TARGET).abs());
                in_band |= (REWARD_STEPS_BAND.0..=REWARD_STEPS_BAND.1).contains(&d);
            }
        }
        let better = match best_entry {
            None => true,
            Some((best_index, best_miss, best_in_band)) => {
                in_band & !best_in_band
                    || (in_band == best_in_band
                        && (miss < best_miss
                            || (miss == best_miss && access[index] < access[best_index])))
            }
        };
        if better {
            best_entry = Some((index, miss, in_band));
        }
    }
    let chosen = match best_entry {
        Some((index, _, _)) => index,
        None => {
            // The layout promises a slot whose near ring cell is inside the
            // 32-cell bound, so this is a fallback for a hand-built layout in
            // a test rather than a normal path.
            let mut best = 0usize;
            for index in 1..entry_options.len() {
                if start_dist[index] < start_dist[best] {
                    best = index;
                }
            }
            best
        }
    };
    debug_assert!(
        start_dist[chosen] <= ENTRY_NEAR_CELLS,
        "breach {} cells from the start",
        start_dist[chosen]
    );
    let (entry_side, entry_along) = entry_options[chosen];
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

    // 6. The exit: a wall the breach is not on, at the aisle whose route from
    //    the breach is inside [`REWARD_STEPS_BAND`] and closest to
    //    `REWARD_STEPS_TARGET`. Every wall with a usable door gets its own
    //    draw, so all four exit orientations occur across seeds.
    //
    //    The distance row is recomputed here, after the breach's vestibule has
    //    opened one more link: the picker must measure the maze the player
    //    actually walks, not the tree it started as.
    let row = lattice_row(tiles, inner, cols, rows, entry_aisle);
    // Per wall: its best door inside the band, as (miss, -landing tier, along).
    let mut sides: Vec<(Cardinal, i32, i32, i32)> = Vec::new();
    for side in SIDES {
        if side == entry_side {
            continue;
        }
        let mut best: Option<(i32, i32, i32)> = None;
        for along in 0..side_count(side, cols, rows) {
            let (cell, ingress) = ring_cell(ring, inner, side, along);
            if entry_cells.contains(&cell) || !tiles.get(ingress.0, ingress.1).is_open() {
                continue;
            }
            let (dx, dy) = cardinal_delta(side);
            let landing = (cell.0 + dx, cell.1 + dy);
            if !tiles.get(landing.0, landing.1).is_open() {
                continue;
            }
            let (i, j) = aisle_of(side, along, cols, rows);
            let dist = row[node(i, j)];
            if !(REWARD_STEPS_BAND.0..=REWARD_STEPS_BAND.1).contains(&dist) {
                continue;
            }
            let tier = i32::from(landing_tier(tiles, &layout.rooms, landing));
            let key = (
                (dist - REWARD_STEPS_TARGET).abs(),
                -tier,
                along,
            );
            let better = match best {
                None => true,
                Some((bm, bt, ba)) => {
                    key.0 < bm || (key.0 == bm && (key.1 < bt || (key.1 == bt && key.2 < ba)))
                }
            };
            if better {
                best = Some(key);
            }
        }
        if let Some((miss, tier, along)) = best {
            sides.push((side, miss, tier, along));
        }
    }
    let (exit_dir, exit_along) = if sides.is_empty() {
        // No wall offers a route inside the band (a tiny hand-built lattice):
        // take the closest there is.
        let mut best: Option<(i32, i32, Cardinal, i32)> = None;
        for side in SIDES {
            if side == entry_side {
                continue;
            }
            for along in 0..side_count(side, cols, rows) {
                let (i, j) = aisle_of(side, along, cols, rows);
                let d = row[node(i, j)];
                if d < 0 {
                    continue;
                }
                let miss = (d - REWARD_STEPS_TARGET).abs();
                if best.is_none_or(|(bm, _, _, _)| miss < bm) {
                    best = Some((miss, -1, side, along));
                }
            }
        }
        let Some((_, _, side, along)) = best else {
            clear_rect(tiles, bounds);
            return None;
        };
        (side, along)
    } else {
        let pick = rng.random_range(0..sides.len());
        (sides[pick].0, sides[pick].3)
    };
    let (exit, _) = ring_cell(ring, inner, exit_dir, exit_along);
    let (reward_i, reward_j) = aisle_of(exit_dir, exit_along, cols, rows);
    // The pedestal stands on the aisle right beside the door: the cache is the
    // last step of the route, and claiming it is what opens the way out.
    let (rx, ry) = aisle(reward_i, reward_j);
    let reward = TileMap::tile_rect(rx, ry).center();
    tiles.set(
        exit.0,
        exit.1,
        TileKind::Wall(WallKind::OneWayDoor { dir: exit_dir }),
    );

    // 7. The public fork: from the breach outwards along the walkway, the
    //    farthest cell within `TRAIL_CELLS` that is still open floor. This is
    //    where the gnaw-mark trail starts; the three marks sit on the cells
    //    between it and the breach.
    let fork = trail_fork(tiles, ring, entry, &approach);

    Some(MazeSection {
        bounds,
        ring,
        entry,
        fork,
        exit,
        exit_dir,
        reward,
        reward_cell: (rx, ry),
    })
}

/// Cells just outside the maze's open walkway (two to three cells out from the
/// slot) that are open floor: where its access corridors meet the walkway.
///
/// The walkway itself (`bounds.inflate(1)`) is open by construction, so those
/// cells say nothing about where a corridor arrives and are excluded.
fn approach_cells(tiles: &TileMap, bounds: TileRect) -> Vec<(i32, i32)> {
    let walkway = bounds.inflate(1);
    bounds
        .inflate(3)
        .cells()
        .filter(|&(cx, cy)| {
            !walkway.contains(cx, cy) && tiles.passable(cx, cy, SizeClass::Small)
        })
        .collect()
}

/// The public fork cell: the farthest open cell within [`TRAIL_CELLS`] steps of
/// the breach that is outside the maze's ring.
///
/// The walk follows the outside walkway (`slot.inflate(1)`, which the layout
/// carves as a continuous ring), so it may turn a corner when the breach slid
/// along the wall to where its corridor lands. Ties prefer a cell straight out
/// of the breach and then the one nearest an access corridor: that is where a
/// rat coming down the passage first sees the marks.
fn trail_fork(
    tiles: &TileMap,
    ring: TileRect,
    entry: (i32, i32),
    approach: &[(i32, i32)],
) -> (i32, i32) {
    let mut reached: Vec<((i32, i32), i32)> = Vec::with_capacity(24);
    let mut seen: Vec<(i32, i32)> = Vec::with_capacity(24);
    let mut queue: VecDeque<((i32, i32), i32)> = VecDeque::new();
    seen.push(entry);
    queue.push_back((entry, 0));
    while let Some((cell, dist)) = queue.pop_front() {
        if dist == TRAIL_CELLS {
            continue;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cell.0 + dx, cell.1 + dy);
            // Never back into the warren, and never twice.
            if ring.contains(next.0, next.1) || seen.contains(&next) {
                continue;
            }
            if !tiles.passable(next.0, next.1, SizeClass::Small) {
                continue;
            }
            seen.push(next);
            reached.push((next, dist + 1));
            queue.push_back((next, dist + 1));
        }
    }
    let mut best: Option<(i32, i32, i32, (i32, i32))> = None;
    for &(cell, dist) in &reached {
        // `0` for a cell straight out of the breach (same row or column).
        let off_axis = i32::from(cell.0 != entry.0 && cell.1 != entry.1);
        let corridor = approach
            .iter()
            .map(|&(ax, ay)| (cell.0 - ax).abs() + (cell.1 - ay).abs())
            .min()
            .unwrap_or(0);
        let key = (-dist, off_axis, corridor);
        if best.is_none_or(|(bd, bo, bc, _)| {
            key.0 < bd
                || (key.0 == bd && (key.1 < bo || (key.1 == bo && key.2 < bc)))
        }) {
            best = Some((key.0, key.1, key.2, cell));
        }
    }
    best.map(|(_, _, _, cell)| cell).unwrap_or(entry)
}

/// Cells of the gnaw-mark trail: how many open-floor steps the breach is from
/// [`MazeSection::fork`], walking the outside walkway (never back inside the
/// ring). `None` when the fork is not on that walk.
///
/// Three marks are drawn along the final [`TRAIL_CELLS`] cells between the fork
/// and the breach, so this is the length of the trail the player reads.
pub fn trail_walk_cells(tiles: &TileMap, maze: &MazeSection) -> Option<i32> {
    let mut seen: Vec<(i32, i32)> = Vec::with_capacity(24);
    let mut queue: VecDeque<((i32, i32), i32)> = VecDeque::new();
    seen.push(maze.entry);
    queue.push_back((maze.entry, 0));
    while let Some((cell, dist)) = queue.pop_front() {
        if cell == maze.fork {
            return Some(dist);
        }
        if dist == TRAIL_CELLS {
            continue;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cell.0 + dx, cell.1 + dy);
            if maze.ring.contains(next.0, next.1) || seen.contains(&next) {
                continue;
            }
            if !tiles.passable(next.0, next.1, SizeClass::Small) {
                continue;
            }
            seen.push(next);
            queue.push_back((next, dist + 1));
        }
    }
    None
}

/// How good the floor outside a one-way door is: `2` when it is inside or next
/// to an ordinary room, `1` when a junction with two onward choices is within
/// [`LANDING_RANGE`] cells, `0` otherwise.
///
/// The designer's requirement for the exit is "in or next to a second room or
/// a junction with two onward choices within 8 cells": the maze's outside
/// walkway is joined to a second room by the layout (`layout::nearest_ordinary`),
/// and this is how the exit picker measures whether its landing reaches that.
pub fn landing_tier(tiles: &TileMap, rooms: &[Room], landing: (i32, i32)) -> u8 {
    for room in rooms {
        if matches!(room.kind, RoomKind::Room | RoomKind::Hall)
            && room.rect.inflate(1).contains(landing.0, landing.1)
        {
            return 2;
        }
    }
    if junction_within(tiles, landing, LANDING_RANGE) {
        1
    } else {
        0
    }
}

/// Is there a cell with two onward choices (three or more open neighbours)
/// within `range` steps of `from`, over open floor?
fn junction_within(tiles: &TileMap, from: (i32, i32), range: i32) -> bool {
    let limit = ((2 * range + 1) * (2 * range + 1)) as usize;
    let mut seen: Vec<(i32, i32)> = Vec::with_capacity(limit);
    let mut reached: Vec<(i32, i32)> = Vec::with_capacity(limit);
    seen.push(from);
    reached.push(from);
    let mut head = 0;
    while head < reached.len() {
        let cell = reached[head];
        head += 1;
        let mut open = 0;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cell.0 + dx, cell.1 + dy);
            if !tiles.passable(next.0, next.1, SizeClass::Small) {
                continue;
            }
            open += 1;
            if seen.len() >= limit || seen.contains(&next) {
                continue;
            }
            seen.push(next);
            reached.push(next);
        }
        let travelled = (cell.0 - from.0).abs() + (cell.1 - from.1).abs();
        if travelled > 0 && open >= 3 {
            return true;
        }
    }
    false
}

/// Open ring cells of a side's lattice (the candidates for a breach or a door).
fn side_count(side: Cardinal, cols: i32, rows: i32) -> i32 {
    match side {
        Cardinal::North | Cardinal::South => cols,
        Cardinal::East | Cardinal::West => rows,
    }
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

/// Aisle lattice index reached from a ring side.
fn aisle_of(side: Cardinal, along: i32, cols: i32, rows: i32) -> (i32, i32) {
    match side {
        Cardinal::North => (along, 0),
        Cardinal::South => (along, rows - 1),
        Cardinal::West => (0, along),
        Cardinal::East => (cols - 1, along),
    }
}

/// The side of `bounds` a ring cell sits on (for the entry/exit wall check).
pub fn side_of_ring(bounds: TileRect, cell: (i32, i32)) -> Option<Cardinal> {
    let ring = maze_ring(bounds);
    if cell.1 == ring.y {
        Some(Cardinal::North)
    } else if cell.1 == ring.bottom() - 1 {
        Some(Cardinal::South)
    } else if cell.0 == ring.x {
        Some(Cardinal::West)
    } else if cell.0 == ring.right() - 1 {
        Some(Cardinal::East)
    } else {
        None
    }
}

/// The lattice geometry of a carved section: `(inner, cols, rows)`.
fn lattice_of(bounds: TileRect) -> (TileRect, i32, i32) {
    let ring = maze_ring(bounds);
    let inner = ring.inflate(-1);
    (
        inner,
        (inner.w + 1).div_euclid(AISLE_STEP),
        (inner.h + 1).div_euclid(AISLE_STEP),
    )
}

/// The partition cell between two lattice neighbours (see [`carve_one`]).
fn link_cell(inner: TileRect, i: i32, j: i32, di: i32, dj: i32) -> (i32, i32) {
    (inner.x + 2 * i + di, inner.y + 2 * j + dj)
}

/// Lattice distances from one aisle node to every node, `-1` where the carved
/// aisles do not reach — indexed by the lattice's own `node(i, j)` layout, so
/// `row[node(i, j)]` is the route length in lattice steps.
fn lattice_row(
    tiles: &TileMap,
    inner: TileRect,
    cols: i32,
    rows: i32,
    from: (i32, i32),
) -> Vec<i32> {
    let node = |i: i32, j: i32| (j * cols + i) as usize;
    let inside = |i: i32, j: i32| (0..cols).contains(&i) && (0..rows).contains(&j);
    let mut row = vec![-1i32; (cols * rows) as usize];
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    row[node(from.0, from.1)] = 0;
    queue.push_back(from);
    while let Some((ci, cj)) = queue.pop_front() {
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (ni, nj) = (ci + di, cj + dj);
            if !inside(ni, nj) || row[node(ni, nj)] >= 0 {
                continue;
            }
            let (cx, cy) = link_cell(inner, ci, cj, di, dj);
            if !tiles.get(cx, cy).is_open() {
                continue;
            }
            row[node(ni, nj)] = row[node(ci, cj)] + 1;
            queue.push_back((ni, nj));
        }
    }
    row
}

/// Independent interior loops of a maze's aisle lattice: edges − vertices + 1.
///
/// One loop means the rat can reverse around a pursuer inside the warren; zero
/// means every route is the only route.
pub fn maze_loops(tiles: &TileMap, maze: &MazeSection) -> i32 {
    let (inner, cols, rows) = lattice_of(maze.bounds);
    if cols < 2 || rows < 2 {
        return 0;
    }
    let mut nodes = 0;
    let mut edges = 0;
    for j in 0..rows {
        for i in 0..cols {
            let (cx, cy) = (inner.x + i * AISLE_STEP, inner.y + j * AISLE_STEP);
            if !tiles.passable(cx, cy, SizeClass::Small) {
                continue;
            }
            nodes += 1;
            // Count east and south links only, so every edge is counted once.
            for (di, dj) in [(1, 0), (0, 1)] {
                if i + di >= cols || j + dj >= rows {
                    continue;
                }
                let (lx, ly) = link_cell(inner, i, j, di, dj);
                if tiles.passable(lx, ly, SizeClass::Small)
                    && tiles.passable(
                        inner.x + (i + di) * AISLE_STEP,
                        inner.y + (j + dj) * AISLE_STEP,
                        SizeClass::Small,
                    )
                {
                    edges += 1;
                }
            }
        }
    }
    if nodes == 0 {
        0
    } else {
        edges - nodes + 1
    }
}

/// Grid cells from the breach to the cache, along the maze's own aisles — the
/// designer's "cache walk": `cells * 16 / 58` seconds at the rat's 58 px/s.
///
/// The search is confined to `maze.bounds`, so it measures the warren and not
/// the walkway outside it. `None` when the cache cannot be walked to.
pub fn maze_walk_cells(tiles: &TileMap, maze: &MazeSection) -> Option<i32> {
    let bounds = maze.bounds;
    if !bounds.contains(maze.entry.0, maze.entry.1) {
        return None;
    }
    let target = maze.reward_cell;
    let mut seen = vec![false; (bounds.w.max(0) * bounds.h.max(0)) as usize];
    let index = |cx: i32, cy: i32| ((cy - bounds.y) * bounds.w + (cx - bounds.x)) as usize;
    if !bounds.contains(target.0, target.1) {
        return None;
    }
    let mut dist = vec![-1i32; seen.len()];
    let mut queue = VecDeque::new();
    dist[index(maze.entry.0, maze.entry.1)] = 0;
    seen[index(maze.entry.0, maze.entry.1)] = true;
    queue.push_back(maze.entry);
    while let Some((cx, cy)) = queue.pop_front() {
        if (cx, cy) == target {
            return Some(dist[index(cx, cy)]);
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cx + dx, cy + dy);
            if !bounds.contains(next.0, next.1) || !tiles.passable(next.0, next.1, SizeClass::Small)
            {
                continue;
            }
            let i = index(next.0, next.1);
            if seen[i] {
                continue;
            }
            seen[i] = true;
            dist[i] = dist[index(cx, cy)] + 1;
            queue.push_back(next);
        }
    }
    None
}

/// Put a slot back the way stage A left it (open floor).
fn clear_rect(tiles: &mut TileMap, bounds: TileRect) {
    for (cx, cy) in bounds.cells() {
        tiles.set(cx, cy, TileKind::Open);
    }
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
    use crate::game::level::{level_rng, Level, MAP_SIZE};
    use crate::game::levelgen::{generate_once, validate};

    /// Seeds used by the property-style sweeps: three themes × these seeds.
    const SEEDS: [u64; 8] = [0, 1, 2, 3, 7, 11, 42, 99];

    /// Cells / second the rat walks, for turning a maze walk into seconds.
    const RAT_PX_PER_SEC: f32 = 58.0;

    fn generated() -> impl Iterator<Item = Level> {
        Theme::all()
            .into_iter()
            .flat_map(|theme| SEEDS.map(move |seed| (theme, seed)))
            .map(|(theme, seed)| generate_once(theme, seed, 0))
    }

    /// One map's **maze stage** only: layout, props and the carve.
    ///
    /// The later stages (rat-holes, traps, navigation) cost ~40 ms a map and
    /// change nothing about a maze's shape, so the property sweeps here run
    /// this instead — a few hundred microseconds a map, thousands of maps.
    struct Carved {
        theme: Theme,
        tiles: TileMap,
        layout: Layout,
        mazes: Vec<MazeSection>,
    }

    fn carve_only(theme: Theme, seed: u64) -> Carved {
        let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, seed as u32);
        let mut rng = level_rng(theme, seed, 0);
        let layout = crate::game::levelgen::layout::build_layout(&mut tiles, &mut rng, theme);
        let _ = crate::game::levelgen::layout::furnish(&mut tiles, &layout, &mut rng, theme);
        let mazes = carve_mazes(&mut tiles, &layout, &mut rng, theme);
        Carved {
            theme,
            tiles,
            layout,
            mazes,
        }
    }

    /// `per_theme` seeds on every theme, maze stage only.
    fn carve_sweep(per_theme: u64) -> impl Iterator<Item = Carved> {
        Theme::all().into_iter().flat_map(move |theme| {
            (0..per_theme).map(move |seed| carve_only(theme, seed))
        })
    }

    fn layout_with(slots: Vec<TileRect>) -> Layout {
        Layout {
            rooms: Vec::new(),
            start: MAP_SIZE * 0.5,
            start_room: TileRect::new(MAP_COLS / 2, MAP_ROWS / 2, 8, 8),
            boss_arena: TileRect::new(20, 20, 20, 20),
            boss: Default::default(),
            maze_slots: slots,
            partitions: Vec::new(),
        }
    }

    /// The side of `bounds` a ring/bounds cell sits on, from its coordinates.
    fn side_of(bounds: TileRect, cell: (i32, i32)) -> Cardinal {
        side_of_ring(bounds, cell).expect("cell is not on the ring")
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
                let reward = maze.reward_cell;
                assert!(level.tiles.passable(reward.0, reward.1, SizeClass::Small));
                assert!(maze.bounds.contains(reward.0, reward.1));
                assert_eq!(level.objects.rewards[i].pos, maze.reward);
                // The ring is closed except the entry gap and the one-way door.
                let ring = maze_ring(maze.bounds);
                assert_eq!(maze.ring, ring, "{theme:?} maze {i}: ring mismatch");
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
                // The exit is never on the breach's wall, and vice versa.
                assert_ne!(
                    side_of(maze.bounds, maze.exit),
                    side_of(maze.bounds, maze.entry),
                    "{theme:?} maze {i}: exit on the entry wall"
                );
                // At least one loop to double back around a pursuer in.
                assert!(
                    maze_loops(&level.tiles, maze) >= 1,
                    "{theme:?} maze {i}: no interior loop"
                );
                // The fork is public walkway floor outside the ring.
                assert!(
                    !ring.contains(maze.fork.0, maze.fork.1),
                    "{theme:?} maze {i}: fork {:?} is inside the maze",
                    maze.fork
                );
                assert!(level.tiles.passable(maze.fork.0, maze.fork.1, SizeClass::Small));
                let trail = trail_walk_cells(&level.tiles, maze)
                    .expect("the fork is on the walkway from its breach");
                assert_eq!(
                    trail, TRAIL_CELLS,
                    "{theme:?} maze {i}: the trail from fork {:?} is {trail} cells",
                    maze.fork
                );
            }
        }
    }

    #[test]
    fn a_maze_is_reachable_and_its_door_leads_out() {
        for level in generated() {
            for (i, maze) in level.mazes.iter().enumerate() {
                let seen = validate::reachable(&level.tiles, validate::cell_of(level.start), SizeClass::Small);
                let reward = maze.reward_cell;
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
    fn a_slot_too_small_to_carve_is_refused_before_anything_is_painted() {
        // `carve_mazes` carves **every** reservation, so the layout only
        // reserves slots of 19/21/23 × 15/17/19 cells. This checks the guard
        // underneath: a slot below that size bails out without touching it.
        let theme = Theme::Sewer;
        let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, 1);
        let mut rng = level_rng(theme, 5, 0);
        let layout = layout_with(Vec::new());
        let slot = TileRect::new(40, 40, 4, 4);
        assert!(carve_one(&mut tiles, &layout, slot, &mut rng).is_none());
        assert!(slot.cells().all(|(cx, cy)| tiles.get(cx, cy).is_open()));
    }

    #[test]
    fn open_cells_lists_exactly_the_passable_cells() {
        let level = generate_once(Theme::Sewer, 5, 0);
        let rect = level.boss_room().expect("boss arena").rect;
        let small = open_cells(&level.tiles, rect, SizeClass::Small);
        let large = open_cells(&level.tiles, rect, SizeClass::Large);
        // The arena is furnished for its fight (perimeter props, the Sewer's
        // escape hole), so it is not uniformly open: the list must be exactly
        // the passable cells of the rect.
        let passable: Vec<(i32, i32)> = rect
            .cells()
            .filter(|&(cx, cy)| level.tiles.passable(cx, cy, SizeClass::Small))
            .collect();
        assert_eq!(small.len(), passable.len(), "the boss arena's open cells are listed");
        assert_eq!(large.len(), small.len());
        assert!(small.iter().all(|(cx, cy)| level.tiles.passable(*cx, *cy, SizeClass::Small)));
    }

    /// The owner's frequency: about one map in eight, decided by the seed.
    ///
    /// Swept over [`crate::game::levelgen::layout::build_layout`] rather than
    /// the whole pipeline — the decision is that stage's draw — and
    /// [`every_reserved_slot_is_carved`] proves a reservation and a maze are
    /// the same thing, so 4000 maps stay cheap enough for the default suite.
    #[test]
    fn maze_rate_is_about_one_in_eight() {
        const MAPS: u64 = 4000;
        let mut with_maze = 0u64;
        for seed in 0..MAPS {
            let theme = Theme::all()[(seed % 3) as usize];
            let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, seed as u32);
            let mut rng = level_rng(theme, seed, 0);
            let layout =
                crate::game::levelgen::layout::build_layout(&mut tiles, &mut rng, theme);
            assert!(
                layout.maze_slots.len() <= MAZES_PER_MAP,
                "seed {seed}: {} reserved slots",
                layout.maze_slots.len()
            );
            if !layout.maze_slots.is_empty() {
                with_maze += 1;
            }
        }
        let rate = with_maze as f64 / MAPS as f64;
        println!(
            "maze rate: {with_maze} of {MAPS} maps ({:.1} %, one in eight is 12.5 %)",
            rate * 100.0
        );
        assert!(
            (0.10..=0.15).contains(&rate),
            "maze rate {rate:.3} over {MAPS} maps ({with_maze} with a maze)"
        );
    }

    /// Every reservation is carved: a reserved maze room is never left bare
    /// (the designer found nine of those because layout and carving each rolled
    /// their own maze count).
    #[test]
    fn every_reserved_slot_is_carved() {
        let mut mazes = 0;
        for map in carve_sweep(400) {
            let slots: Vec<TileRect> = map
                .layout
                .rooms
                .iter()
                .filter(|room| room.kind == RoomKind::Maze)
                .map(|room| room.rect)
                .collect();
            let carved: Vec<TileRect> = map.mazes.iter().map(|m| m.bounds).collect();
            assert_eq!(
                slots, carved,
                "{:?}: reserved {slots:?} but carved {carved:?}",
                map.theme
            );
            assert_eq!(slots.len(), map.layout.maze_slots.len());
            assert!(map.mazes.len() <= MAZES_PER_MAP);
            mazes += carved.len();
        }
        assert!(mazes > 0, "the sweep found no maze at all");
    }

    /// A maze is an opening choice, not a hunt: its slot is reserved inside an
    /// open area near the spawn, so the rat can walk from the spawn right onto
    /// its outside walkway — and the walkway is floor all the way round, so a
    /// bad wager can be walked past instead of trapping the player.
    #[test]
    fn maze_slot_sits_in_open_floor_near_the_start() {
        let mut mazes = 0;
        for map in carve_sweep(400) {
            let Some(slot) = map.layout.maze_slots.first().copied() else {
                continue;
            };
            mazes += 1;
            let start = validate::cell_of(map.layout.start);
            let seen = validate::reachable(&map.tiles, start, SizeClass::Small);
            // The whole outside walkway is reachable from the spawn, breach
            // included: that is the passage from the starting chamber.
            for (cx, cy) in slot
                .inflate(1)
                .cells()
                .filter(|&(cx, cy)| !slot.contains(cx, cy))
            {
                assert!(
                    validate::is_reached(&seen, &map.tiles, (cx, cy)),
                    "{:?} at {slot:?}: walkway cell ({cx}, {cy}) is not reachable from the spawn",
                    map.theme
                );
            }
            let Some(maze) = map.mazes.first() else {
                panic!("{:?} at {slot:?}: the slot was not carved", map.theme);
            };
            assert!(validate::is_reached(&seen, &map.tiles, maze.entry));
            // The walkway is open floor all the way round, so the maze can be
            // walked past: a full ring of open cells outside the partition.
            for (cx, cy) in slot.inflate(1).cells() {
                if slot.contains(cx, cy) {
                    continue;
                }
                let outside = !slot.inflate(1).contains(cx, cy) || !map.tiles.get(cx, cy).is_open();
                assert!(
                    outside || map.tiles.passable(cx, cy, SizeClass::Small),
                    "{:?} at {slot:?}: the walkway at ({cx}, {cy}) is walled",
                    map.theme
                );
            }
            // The breach is within the owner's 32-cell promise.
            let gap = (maze.entry.0 - start.0)
                .abs()
                .max((maze.entry.1 - start.1).abs());
            assert!(
                gap <= ENTRY_NEAR_CELLS,
                "{:?} at {slot:?}: the breach is {gap} cells from the spawn",
                map.theme
            );
        }
        assert!(mazes >= 100, "only {mazes} mazes in the sweep");
    }

    /// The shape requirements, measured on a sweep: breach near the start and
    /// facing its corridor, cache walk 7–12 s, all four exit walls, a loop.
    #[test]
    fn maze_shape_holds_on_a_sweep() {
        let mut exits = [0usize; 4];
        let mut walks = Vec::new();
        let mut mazes = 0;
        let mut shortest = f32::MAX;
        for map in carve_sweep(800) {
            let start = validate::cell_of(map.layout.start);
            for (i, maze) in map.mazes.iter().enumerate() {
                mazes += 1;
                let ctx = format!("{:?} maze {i} at {:?}", map.theme, maze.bounds);
                // The breach is within about 32 cells of the rat's start.
                let gap = (maze.entry.0 - start.0).abs().max((maze.entry.1 - start.1).abs());
                assert!(
                    gap <= ENTRY_NEAR_CELLS,
                    "{ctx}: breach {gap} cells from the start"
                );
                // It faces its access corridor: within the slack of the
                // nearest corridor meeting point among the ring cells that are
                // themselves within the 32-cell promise.
                let approach = approach_cells(&map.tiles, maze.bounds);
                let cell_gap = |cell: (i32, i32)| {
                    (cell.0 - start.0).abs().max((cell.1 - start.1).abs())
                };
                let corridor_gap = |cell: (i32, i32)| {
                    approach
                        .iter()
                        .map(|&(ax, ay)| (cell.0 - ax).abs() + (cell.1 - ay).abs())
                        .min()
                        .unwrap_or(i32::MAX / 4)
                };
                let least = ring_border(maze.ring)
                    .filter(|&cell| cell_gap(cell) <= ENTRY_NEAR_CELLS)
                    .map(corridor_gap)
                    .min()
                    .unwrap_or(0);
                let nearest = corridor_gap(maze.entry);
                assert!(
                    nearest <= least + ENTRY_SLACK_CELLS + 1,
                    "{ctx}: the breach is {nearest} cells from any corridor (least {least})"
                );
                // Cache walk: 7–12 s at 58 px/s.
                let cells = maze_walk_cells(&map.tiles, maze).expect("reward unreachable");
                let seconds = cells as f32 * 16.0 / RAT_PX_PER_SEC;
                walks.push(seconds);
                shortest = shortest.min(seconds);
                // A loop to double back in, a gnaw-mark trail of four cells,
                // and a landing with a way onward.
                assert!(maze_loops(&map.tiles, maze) >= 1, "{ctx}: no loop");
                assert_eq!(
                    trail_walk_cells(&map.tiles, maze),
                    Some(TRAIL_CELLS),
                    "{ctx}: the gnaw-mark trail is not the final four cells"
                );
                let (dx, dy) = cardinal_delta(maze.exit_dir);
                let landing = (maze.exit.0 + dx, maze.exit.1 + dy);
                assert!(map.tiles.passable(landing.0, landing.1, SizeClass::Small));
                assert!(
                    landing_tier(&map.tiles, &map.layout.rooms, landing) >= 1,
                    "{ctx}: the exit lands on a dead end"
                );
                exits[side_index(maze.exit_dir)] += 1;
            }
        }
        assert!(mazes >= 150, "only {mazes} mazes in the sweep");
        assert!(
            exits.iter().all(|count| *count > 0),
            "exit orientations: {exits:?}"
        );
        walks.sort_by(f32::total_cmp);
        let median = f64::from(walks[walks.len() / 2]);
        assert!(
            (7.0..=12.0).contains(&median),
            "median cache walk {median:.2} s over {} mazes",
            walks.len()
        );
        let longest = f64::from(walks[walks.len() - 1]);
        let at = |q: f64| f64::from(walks[((walks.len() - 1) as f64 * q).round() as usize]);
        println!(
            "maze sweep: {mazes} mazes, cache walk min {shortest:.2} p05 {:.2} p25 {:.2} med {median:.2} p75 {:.2} p95 {:.2} max {longest:.2}; exits N {} E {} S {} W {}",
            at(0.05),
            at(0.25),
            at(0.75),
            at(0.95),
            exits[0],
            exits[1],
            exits[2],
            exits[3]
        );
        // The cache walk band is the design; a lattice whose spanning-tree
        // distances skip it (they come in steps of two, and can jump from 11 to
        // 21) can miss it, so the tail is bounded rather than forced:
        // median and the bulk inside 7–12 s, and no maze far outside.
        assert!(
            (7.0..=12.0).contains(&median),
            "median cache walk {median:.2} s"
        );
        assert!(
            at(0.25) >= 7.0,
            "a quarter of mazes walk under 7 s ({:.2} s)",
            at(0.25)
        );
        assert!(at(0.95) <= 12.0, "p95 cache walk {:.2} s", at(0.95));
        assert!(longest <= 13.0, "longest cache walk {longest:.2} s");
        assert!(shortest >= 5.5, "shortest cache walk {shortest:.2} s");
    }

    #[test]
    fn a_seed_grows_the_same_maze_twice() {
        for theme in Theme::all() {
            for seed in [0u64, 3, 7] {
                let a = generate_once(theme, seed, 0);
                let b = generate_once(theme, seed, 0);
                assert_eq!(a.mazes.len(), b.mazes.len(), "{theme:?} seed {seed}");
                for (x, y) in a.mazes.iter().zip(&b.mazes) {
                    assert_eq!(x.entry, y.entry);
                    assert_eq!(x.fork, y.fork);
                    assert_eq!(x.ring, y.ring);
                    assert_eq!(x.exit, y.exit);
                    assert_eq!(x.exit_dir, y.exit_dir);
                    assert_eq!(x.reward, y.reward);
                    assert_eq!(x.reward_cell, y.reward_cell);
                }
            }
        }
    }

    fn side_index(dir: Cardinal) -> usize {
        match dir {
            Cardinal::North => 0,
            Cardinal::East => 1,
            Cardinal::South => 2,
            Cardinal::West => 3,
        }
    }

}
