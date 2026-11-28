//! **Measurement only** — the open-world numbers, the map dump and the
//! survival probe.
//!
//! None of this runs in a normal `cargo test`: every entry point is gated on an
//! environment variable, so the default suite pays one `std::env::var` per
//! test and nothing else. The module exists so the numbers in the open-world
//! change can be reproduced:
//!
//! ```text
//! RATRUN_MAP_STATS=500 cargo test open_world_statistics -- --nocapture
//! RATRUN_MAP_DUMP=/tmp/ratrun-maps cargo test open_world_dump -- --nocapture
//! RATRUN_SURVIVAL=1 cargo test open_world_survival -- --nocapture
//! ```
//!
//! * [`measure`] is the acceptance table: walkable fraction, largest open
//!   area, area count, cycle count, narrowest required passage, corridor cell
//!   share, useful holes and room-mouth traps.
//! * [`dump_ppm`] writes one whole-map image per seed (`ppm2png.py` converts
//!   it) with the rooms, walls, openings, holes and traps coloured the way the
//!   design review drew them.
//! * [`survival`] drives the **real** tick schedule with two scripted players
//!   (circling the start area, and wandering) and reports how long each lives
//!   and the peak enemy count.
//!
//! Determinism: every number here is a pure function of the level (plus, for
//! the survival probe, the world), and no loop iterates a hash map.

#![cfg(test)]

use std::collections::BinaryHeap;
use std::fmt::Write as _;

use crate::game::art_walls::WallKind;
use crate::game::collision::{SizeClass, TileKind, TileMap};
use crate::game::level::{Level, RoomKind, Theme, MAP_COLS, MAP_ROWS};
use crate::math::Vec2;

use super::{placement, validate};

/// One map's open-world measurements.
#[derive(Clone, Debug, Default)]
pub struct MapMetrics {
    /// Passable cells for the rat, and their share of the whole 200 × 160 grid.
    pub open_cells: usize,
    pub open_fraction: f64,
    /// The largest 4-connected patch of passable cells.
    pub largest_component: usize,
    /// How many passable cells are *not* on that patch (should be 0).
    pub stray_cells: usize,
    /// The largest axis-aligned open rectangle anywhere on the map, in cells
    /// (`w`, `h`) — the "great hall" measure.
    pub largest_rect: (i32, i32),
    /// Named areas: halls, rooms, the start area and the boss arena.
    pub areas: usize,
    /// The largest named area, in cells.
    pub largest_area: i32,
    /// Independent cycles of the walkable **cell** graph (`E − V + C`): how
    /// many independent ways around the map's rock there are.
    pub cell_cycles: i64,
    /// Free-standing rock: solid components that touch neither the border nor
    /// another wall (every one of them can be circled).
    pub loop_makers: usize,
    /// The **area** graph: one node per named area that owns floor, an edge per
    /// pair of areas whose floor touches.
    pub area_nodes: usize,
    pub area_edges: usize,
    pub area_cycles: i64,
    /// Spaces: areas that overlap are one open space, not two areas joined by a
    /// wall with a hole.
    pub spaces: usize,
    /// Spaces with only one opening onto the rest of the map (a single choke
    /// point), and the largest such space in cells.
    pub single_opening_spaces: usize,
    pub largest_single_opening: i32,
    /// The narrowest passage any required route has to use, in cells (the
    /// widest-path bottleneck from the start to each area centre, minimum over
    /// areas).
    pub narrowest_passage: i32,
    /// Corridor cells, and their share of the walkable cells.
    pub corridor_cells: usize,
    pub corridor_share: f64,
    /// Rat-holes, and how many of them are the review's *useful* kind.
    pub holes: usize,
    pub useful_holes: usize,
    /// Traps, and how many of them are room-mouth encounters.
    pub traps: usize,
    pub encounter_traps: usize,
    /// Mazes on this map.
    pub mazes: usize,
}

/// Measure one map.
pub fn measure(level: &Level) -> MapMetrics {
    let tiles = &level.tiles;
    let cols = tiles.cols();
    let rows = tiles.rows();
    let cell_count = (cols * rows) as usize;

    let mut passable = vec![false; cell_count];
    let mut open = 0usize;
    for cy in 0..rows {
        for cx in 0..cols {
            if tiles.passable(cx, cy, SizeClass::Small) {
                passable[(cy * cols + cx) as usize] = true;
                open += 1;
            }
        }
    }

    // ---- components of the walkable graph --------------------------------
    let mut label = vec![usize::MAX; cell_count];
    let mut sizes: Vec<usize> = Vec::new();
    let mut queue: Vec<usize> = Vec::new();
    for start in 0..cell_count {
        if !passable[start] || label[start] != usize::MAX {
            continue;
        }
        let id = sizes.len();
        let mut size = 0usize;
        label[start] = id;
        queue.clear();
        queue.push(start);
        while let Some(i) = queue.pop() {
            size += 1;
            let (cx, cy) = ((i as i32) % cols, (i as i32) / cols);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                    continue;
                }
                let j = (ny * cols + nx) as usize;
                if passable[j] && label[j] == usize::MAX {
                    label[j] = id;
                    queue.push(j);
                }
            }
        }
        sizes.push(size);
    }
    let largest_component = sizes.iter().copied().max().unwrap_or(0);
    let stray_cells = open - largest_component;

    // ---- independent cycles of the cell graph ----------------------------
    let mut edges = 0usize;
    for cy in 0..rows {
        for cx in 0..cols {
            if !passable[(cy * cols + cx) as usize] {
                continue;
            }
            if cx + 1 < cols && passable[(cy * cols + cx + 1) as usize] {
                edges += 1;
            }
            if cy + 1 < rows && passable[((cy + 1) * cols + cx) as usize] {
                edges += 1;
            }
        }
    }
    let cell_cycles = edges as i64 - open as i64 + sizes.len() as i64;

    // ---- free-standing rock ---------------------------------------------
    let loop_makers = free_standing_solids(tiles);

    // ---- areas and the area graph ---------------------------------------
    // Nodes are the named regions that own walkable floor (start, boss, halls,
    // rooms and corridors — not mazes, whose interior is a warren, not a
    // thoroughfare). An edge means two rooms' floor cells touch, i.e. there is
    // a way straight from one into the other: an overlap (the areas merged) or
    // a punched opening. Independent cycles are `E − V + C`; that is the number
    // of ways round the map an area graph offers on top of a tree.
    let mut areas = Vec::new();
    // The open areas, on their own: the ones a player has to be able to stand
    // and fight in (corridors are connections, not rooms, so they are not
    // required to be wide).
    let mut named = Vec::new();
    for room in &level.rooms {
        if matches!(
            room.kind,
            RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room
        ) {
            named.push(room.rect);
        }
        if matches!(
            room.kind,
            RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room | RoomKind::Corridor
        ) {
            areas.push(room.rect);
        }
    }
    // `-1` = open floor that belongs to no named region.
    let mut owner = vec![-1i32; cell_count];
    for (index, rect) in areas.iter().enumerate() {
        for (cx, cy) in rect.cells() {
            if !tiles.in_bounds(cx, cy) {
                continue;
            }
            let i = (cy * cols + cx) as usize;
            if passable[i] && owner[i] < 0 {
                owner[i] = index as i32;
            }
        }
    }
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); areas.len()];
    for cy in 0..rows {
        for cx in 0..cols {
            let i = (cy * cols + cx) as usize;
            if owner[i] < 0 {
                continue;
            }
            if cx + 1 < cols {
                let j = i + 1;
                if owner[j] >= 0 && owner[j] != owner[i] {
                    link(&mut adjacency, owner[i] as usize, owner[j] as usize);
                }
            }
            if cy + 1 < rows {
                let j = i + cols as usize;
                if owner[j] >= 0 && owner[j] != owner[i] {
                    link(&mut adjacency, owner[i] as usize, owner[j] as usize);
                }
            }
        }
    }
    let area_nodes = (0..areas.len())
        .filter(|&i| {
            areas[i]
                .cells()
                .any(|(cx, cy)| tiles.in_bounds(cx, cy) && owner[(cy * cols + cx) as usize] == i as i32)
        })
        .count();
    let mut area_edges = 0usize;
    for neighbours in &adjacency {
        area_edges += neighbours.len();
    }
    area_edges /= 2;
    // Connected components of the area graph (isolated areas count).
    let mut comp = vec![usize::MAX; areas.len()];
    let mut area_components = 0usize;
    let mut queue: Vec<usize> = Vec::new();
    for root in 0..areas.len() {
        if comp[root] != usize::MAX {
            continue;
        }
        area_components += 1;
        comp[root] = area_components;
        queue.clear();
        queue.push(root);
        while let Some(node) = queue.pop() {
            for &next in &adjacency[node] {
                if comp[next] == usize::MAX {
                    comp[next] = area_components;
                    queue.push(next);
                }
            }
        }
    }
    let area_cycles =
        area_edges as i64 - area_nodes as i64 + area_components.max(1) as i64;

    // ---- spaces: merged areas are one room, and how many ways out each has
    // Areas that overlap are the same open space. A space is "hung off the rest
    // by a single choke point" when its floor meets the rest of the map at only
    // one opening — one contiguous run of floor on its boundary — however many
    // areas it is made of.
    let mut space_of: Vec<usize> = (0..named.len()).collect();
    fn root_of(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for i in 0..named.len() {
        for j in i + 1..named.len() {
            if named[i].intersects(&named[j]) {
                let (ri, rj) = (root_of(&mut space_of, i), root_of(&mut space_of, j));
                if ri != rj {
                    space_of[ri] = rj;
                }
            }
        }
    }
    // owner per cell: which named area (first match) and which space.
    let mut cell_owner = vec![usize::MAX; cell_count];
    for (index, rect) in named.iter().enumerate() {
        for (cx, cy) in rect.cells() {
            if tiles.in_bounds(cx, cy) && passable[(cy * cols + cx) as usize] {
                let i = (cy * cols + cx) as usize;
                if cell_owner[i] == usize::MAX {
                    cell_owner[i] = root_of(&mut space_of, index);
                }
            }
        }
    }
    let mut spaces: Vec<usize> = (0..named.len()).map(|i| root_of(&mut space_of, i)).collect();
    spaces.sort_unstable();
    spaces.dedup();
    let mut single_opening_spaces = 0usize;
    let mut largest_single_opening = 0i32;
    for space in &spaces {
        let mut size = 0i32;
        let mut opening_cells: Vec<(i32, i32)> = Vec::new();
        for (index, rect) in named.iter().enumerate() {
            if root_of(&mut space_of, index) != *space {
                continue;
            }
            size += rect.area();
            for (cx, cy) in rect.cells() {
                if !tiles.in_bounds(cx, cy) {
                    continue;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if !tiles.in_bounds(nx, ny) {
                        continue;
                    }
                    let j = (ny * cols + nx) as usize;
                    if !passable[j] || cell_owner[j] == *space {
                        continue;
                    }
                    if !opening_cells.contains(&(nx, ny)) {
                        opening_cells.push((nx, ny));
                    }
                }
            }
        }
        // Group the outside floor cells that touch this space into runs: one
        // run per opening.
        let mut seen: Vec<(i32, i32)> = Vec::new();
        let mut openings = 0usize;
        for &cell in &opening_cells {
            if seen.contains(&cell) {
                continue;
            }
            openings += 1;
            seen.push(cell);
            let mut queue = vec![cell];
            while let Some((cx, cy)) = queue.pop() {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let next = (cx + dx, cy + dy);
                    if !seen.contains(&next) && opening_cells.contains(&next) {
                        seen.push(next);
                        queue.push(next);
                    }
                }
            }
        }
        if openings <= 1 {
            single_opening_spaces += 1;
            largest_single_opening = largest_single_opening.max(size);
        }
    }

    // ---- the narrowest passage a required route must use -----------------
    let narrowest_passage = narrowest_required(&passable, cols, rows, level, &named);

    // ---- corridor share -------------------------------------------------
    let mut corridor_cells = 0usize;
    for room in level.rooms_of(RoomKind::Corridor) {
        for (cx, cy) in room.rect.cells() {
            if tiles.in_bounds(cx, cy) && passable[(cy * cols + cx) as usize] {
                corridor_cells += 1;
            }
        }
    }

    // ---- the review's placement numbers ---------------------------------
    let stats = placement::placement_stats(level);

    MapMetrics {
        open_cells: open,
        open_fraction: open as f64 / cell_count as f64,
        largest_component,
        stray_cells,
        largest_rect: largest_open_rect(&passable, cols, rows),
        areas: named.len(),
        largest_area: named.iter().map(|r| r.area()).max().unwrap_or(0),
        cell_cycles,
        loop_makers,
        area_nodes,
        area_edges,
        area_cycles,
        spaces: spaces.len(),
        single_opening_spaces,
        largest_single_opening,
        narrowest_passage,
        corridor_cells,
        corridor_share: if open == 0 {
            0.0
        } else {
            corridor_cells as f64 / open as f64
        },
        holes: stats.holes,
        useful_holes: stats.useful_holes,
        traps: stats.traps,
        encounter_traps: stats.encounter_traps,
        mazes: level.mazes.len(),
    }
}

fn link(adjacency: &mut [Vec<usize>], a: usize, b: usize) {
    if a == b || a >= adjacency.len() || b >= adjacency.len() {
        return;
    }
    if !adjacency[a].contains(&b) {
        adjacency[a].push(b);
    }
    if !adjacency[b].contains(&a) {
        adjacency[b].push(a);
    }
}

/// Solid cells that belong to a component which touches neither the map border
/// nor the two-cell bedrock frame: each is a mass the player can run around.
fn free_standing_solids(tiles: &TileMap) -> usize {
    let cols = tiles.cols();
    let rows = tiles.rows();
    let mut seen = vec![false; (cols * rows) as usize];
    let mut count = 0usize;
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for cy in 0..rows {
        for cx in 0..cols {
            let i = (cy * cols + cx) as usize;
            if seen[i] || tiles.get(cx, cy).is_open() {
                continue;
            }
            let mut size = 0usize;
            let mut touches_edge = false;
            seen[i] = true;
            stack.push((cx, cy));
            while let Some((x, y)) = stack.pop() {
                size += 1;
                if x <= 2 || y <= 2 || x >= cols - 3 || y >= rows - 3 {
                    touches_edge = true;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if !tiles.in_bounds(nx, ny) {
                        continue;
                    }
                    let j = (ny * cols + nx) as usize;
                    if seen[j] || tiles.get(nx, ny).is_open() {
                        continue;
                    }
                    seen[j] = true;
                    stack.push((nx, ny));
                }
            }
            if !touches_edge && size >= 6 {
                count += 1;
            }
        }
    }
    count
}

/// The largest all-open axis-aligned rectangle (maximal-rectangle over a
/// histogram), returned as `(w, h)`.
fn largest_open_rect(passable: &[bool], cols: i32, rows: i32) -> (i32, i32) {
    let mut best = (0, 0);
    let mut best_area = 0;
    let mut heights = vec![0i32; cols as usize];
    for cy in 0..rows {
        for cx in 0..cols {
            let i = (cy * cols + cx) as usize;
            heights[cx as usize] = if passable[i] {
                heights[cx as usize] + 1
            } else {
                0
            };
        }
        // Largest rectangle in the histogram.
        let mut stack: Vec<usize> = Vec::new();
        for x in 0..=cols as usize {
            let h = if x == cols as usize {
                0
            } else {
                heights[x]
            };
            while let Some(&top) = stack.last() {
                if heights[top] <= h {
                    break;
                }
                stack.pop();
                let left = stack.last().map_or(0, |t| t + 1);
                let w = (x - left) as i32;
                let hh = heights[top];
                if w * hh > best_area {
                    best_area = w * hh;
                    best = (w, hh);
                }
            }
            stack.push(x);
        }
    }
    best
}

/// The passenger-width of every cell: the shorter of the open run west–east and
/// the open run north–south through it. A one-cell corridor reads `1`, a
/// four-cell opening reads `4`, the middle of a plaza reads its short side.
fn clearance(passable: &[bool], cols: i32, rows: i32) -> Vec<i32> {
    let idx = |cx: i32, cy: i32| (cy * cols + cx) as usize;
    let mut east_west = vec![0i32; passable.len()];
    let mut north_south = vec![0i32; passable.len()];
    // A cell's run is the whole open stretch through it: forward + back − 1.
    for cy in 0..rows {
        let mut forward = vec![0i32; cols as usize];
        let mut run = 0;
        for cx in 0..cols {
            run = if passable[idx(cx, cy)] { run + 1 } else { 0 };
            forward[cx as usize] = run;
        }
        let mut back = 0;
        for cx in (0..cols).rev() {
            back = if passable[idx(cx, cy)] { back + 1 } else { 0 };
            east_west[idx(cx, cy)] = forward[cx as usize] + back - 1;
        }
    }
    for cx in 0..cols {
        let mut forward = vec![0i32; rows as usize];
        let mut run = 0;
        for cy in 0..rows {
            run = if passable[idx(cx, cy)] { run + 1 } else { 0 };
            forward[cy as usize] = run;
        }
        let mut back = 0;
        for cy in (0..rows).rev() {
            back = if passable[idx(cx, cy)] { back + 1 } else { 0 };
            north_south[idx(cx, cy)] = forward[cy as usize] + back - 1;
        }
    }
    // A run of `n` cells across a corridor is `n` cells wide: the per-cell
    // clearance of a cell inside it is the whole crossing width.
    let mut out = vec![0i32; passable.len()];
    for i in 0..out.len() {
        out[i] = east_west[i].min(north_south[i]);
    }
    out
}

/// The widest-path bottleneck from the start to every area centre, minimum over
/// the areas — the narrowest passage a required route has to use.
fn narrowest_required(
    passable: &[bool],
    cols: i32,
    rows: i32,
    level: &Level,
    areas: &[crate::game::level::TileRect],
) -> i32 {
    let width = clearance(passable, cols, rows);
    let idx = |cx: i32, cy: i32| (cy * cols + cx) as usize;
    let start = validate::cell_of(level.start);
    if !passable[idx(start.0, start.1)] {
        return 0;
    }
    let mut best = vec![0i32; passable.len()];
    let mut heap: BinaryHeap<(i32, i32, i32)> = BinaryHeap::new();
    best[idx(start.0, start.1)] = width[idx(start.0, start.1)];
    heap.push((best[idx(start.0, start.1)], start.0, start.1));
    while let Some((w, cx, cy)) = heap.pop() {
        if w < best[idx(cx, cy)] {
            continue;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx + dx, cy + dy);
            if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                continue;
            }
            let j = idx(nx, ny);
            if !passable[j] {
                continue;
            }
            let candidate = w.min(width[j]);
            if candidate > best[j] {
                best[j] = candidate;
                heap.push((candidate, nx, ny));
            }
        }
    }
    let mut narrowest = i32::MAX;
    for rect in areas {
        let centre = validate::cell_of(rect.center_world());
        if !rect.contains(centre.0, centre.1) || !passable[idx(centre.0, centre.1)] {
            continue;
        }
        // A room may have its centre blocked by a prop; take the best cell of
        // the room's middle square.
        let mut room_best = 0;
        for (cx, cy) in rect.inflate(-1).cells() {
            if !rect.contains(cx, cy) || !passable[idx(cx, cy)] {
                continue;
            }
            room_best = room_best.max(best[idx(cx, cy)]);
        }
        narrowest = narrowest.min(room_best);
    }
    if narrowest == i32::MAX {
        0
    } else {
        narrowest
    }
}

// ---------------------------------------------------------------------------
// The map dump
// ---------------------------------------------------------------------------

/// Paint one whole map into `path` as a binary PPM, one generator cell scaled
/// `scale` times. Colours follow the design review's atlas: green start, brick
/// red boss chamber, slate blue hall, grey-green room, beige corridor, lavender
/// maze, cyan rat-hole, orange door, amber glue, red snap trap, dark bedrock.
pub fn dump_ppm(level: &Level, path: &str, scale: i32) -> std::io::Result<()> {
    let scale = scale.max(1);
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();
    let mut rgb = vec![[24u8, 20, 28]; (cols * rows) as usize];
    // The flatbed's own hazards first (they sit under the rooms/walls pass):
    // holes you fall through, the slipstream stretches, sliding cargo.
    for pit in &level.trailer.pits {
        let colour = match pit.kind {
            crate::game::level::PitKind::BrokenPlank => [10, 10, 14],
            crate::game::level::PitKind::RailGap => [40, 40, 48],
            crate::game::level::PitKind::TidalDrop => [10, 24, 31],
        };
        for (cx, cy) in pit.cells() {
            if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
                rgb[(cy * cols + cx) as usize] = colour;
            }
        }
    }
    for zone in &level.trailer.wind {
        for (cx, cy) in zone.cells() {
            if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
                rgb[(cy * cols + cx) as usize] = [96, 168, 220];
            }
        }
    }
    for slide in &level.trailer.slides {
        let (cx, cy) = validate::cell_of(slide.rest);
        if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
            rgb[(cy * cols + cx) as usize] = [214, 128, 64];
        }
    }
    let inside = |rect: crate::game::level::TileRect| -> Vec<(i32, i32)> {
        rect.cells()
            .filter(|&(cx, cy)| cx >= 0 && cy >= 0 && cx < cols && cy < rows)
            .collect()
    };
    let mut paint = |cells: Vec<(i32, i32)>, colour: [u8; 3]| {
        for (cx, cy) in cells {
            rgb[(cy * cols + cx) as usize] = colour;
        }
    };
    for room in &level.rooms {
        let colour = match room.kind {
            RoomKind::Start => [46, 158, 84],
            RoomKind::Boss => [150, 54, 54],
            RoomKind::Hall => [82, 104, 150],
            RoomKind::Room => [96, 128, 108],
            RoomKind::Corridor => [176, 160, 120],
            RoomKind::Maze => [150, 120, 190],
        };
        paint(inside(room.rect), colour);
    }
    // Walls and floor last-resort colouring, then the openings.
    for cy in 0..rows {
        for cx in 0..cols {
            let i = (cy * cols + cx) as usize;
            match level.tiles.get(cx, cy) {
                TileKind::Open => {}
                TileKind::Wall(WallKind::Solid) => rgb[i] = [36, 32, 44],
                TileKind::Wall(WallKind::Maze) => rgb[i] = [96, 66, 130],
                TileKind::Wall(WallKind::Doorway { .. }) => rgb[i] = [236, 150, 60],
                TileKind::Wall(WallKind::RatHole { .. }) => rgb[i] = [70, 210, 220],
                TileKind::Wall(WallKind::OneWayDoor { .. }) => rgb[i] = [250, 210, 60],
            }
        }
    }
    for prop in &level.props {
        let cells = crate::game::levelgen::validate::cell_of(prop.rect.center());
        let (w, h) = prop.kind.cells();
        for dy in 0..h {
            for dx in 0..w {
                let (cx, cy) = (cells.0 - w / 2 + dx, cells.1 - h / 2 + dy);
                if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
                    rgb[(cy * cols + cx) as usize] = [92, 74, 60];
                }
            }
        }
    }
    for trap in &level.objects.sticky_traps {
        let (cx, cy) = validate::cell_of(trap.pos);
        if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
            rgb[(cy * cols + cx) as usize] = [226, 190, 60];
        }
    }
    for trap in &level.objects.mousetraps {
        let (cx, cy) = validate::cell_of(trap.pos);
        if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
            rgb[(cy * cols + cx) as usize] = [230, 70, 70];
        }
    }
    for pit in &level.objects.tidal_pits {
        for (cx, cy) in inside(pit.bounds()) {
            rgb[(cy * cols + cx) as usize] = [10, 24, 31];
        }
    }
    for trap in &level.objects.lure_traps {
        let (cx, cy) = validate::cell_of(trap.pos);
        if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
            rgb[(cy * cols + cx) as usize] = [102, 244, 238];
        }
    }
    for hole in &level.objects.rat_holes {
        let (cx, cy) = hole.cell;
        if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
            rgb[(cy * cols + cx) as usize] = [70, 210, 220];
        }
    }
    for maze in &level.mazes {
        let (ex, ey) = maze.entry;
        let (xx, xy) = maze.exit;
        if ex >= 0 && ey >= 0 && ex < cols && ey < rows {
            rgb[(ey * cols + ex) as usize] = [60, 240, 120];
        }
        if xx >= 0 && xy >= 0 && xx < cols && xy < rows {
            rgb[(xy * cols + xx) as usize] = [250, 210, 60];
        }
        let (rx, ry) = validate::cell_of(maze.reward);
        if rx >= 0 && ry >= 0 && rx < cols && ry < rows {
            rgb[(ry * cols + rx) as usize] = [255, 240, 140];
        }
    }
    let (sx, sy) = validate::cell_of(level.start);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (sx + dx, sy + dy);
            if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
                rgb[(cy * cols + cx) as usize] = [255, 255, 255];
            }
        }
    }

    let (w, h) = (cols * scale, rows * scale);
    let mut bytes: Vec<u8> = format!("P6\n{w} {h}\n255\n").into_bytes();
    bytes.reserve((w * h * 3) as usize);
    for cy in 0..rows {
        for _ in 0..scale {
            for cx in 0..cols {
                let p = rgb[(cy * cols + cx) as usize];
                for _ in 0..scale {
                    bytes.extend_from_slice(&p);
                }
            }
        }
    }
    std::fs::write(path, bytes)
}

/// A human-readable one-line summary of a map's numbers.
pub fn summary(theme: Theme, seed: u64, m: &MapMetrics) -> String {
    let mut line = String::new();
    let _ = write!(
        line,
        "{theme:?} seed {seed}: open {:.1} %, largest rect {}x{}, areas {}, \
         cell cycles {}, loop makers {}, area cycles {} ({} nodes / {} edges), \
         spaces {} ({} with one opening, largest {}), narrowest {} cells, \
         corridor {:.1} %, holes {} ({} useful), traps {} ({} at mouths), mazes {}",
        m.open_fraction * 100.0,
        m.largest_rect.0,
        m.largest_rect.1,
        m.areas,
        m.cell_cycles,
        m.loop_makers,
        m.area_cycles,
        m.area_nodes,
        m.area_edges,
        m.spaces,
        m.single_opening_spaces,
        m.largest_single_opening,
        m.narrowest_passage,
        m.corridor_share * 100.0,
        m.holes,
        m.useful_holes,
        m.traps,
        m.encounter_traps,
        m.mazes,
    );
    line
}

// ---------------------------------------------------------------------------
// The survival probe
// ---------------------------------------------------------------------------

/// One survival probe result.
#[derive(Clone, Copy, Debug)]
pub struct Survival {
    pub seconds: f32,
    pub peak_enemies: usize,
    pub kills: u32,
    pub level: u32,
}

/// Drive the real schedule for one scripted player.
///
/// `tactic` is `"circle"` (round the start at a 56 px radius, the design
/// review's exploit) or anything else (wander: pick a compass direction, walk
/// it until blocked, then turn).
pub fn survival(theme: Theme, seed: u64, tactic: &str, max_seconds: f32) -> Survival {
    use crate::game::world::World;

    let dt = 1.0 / 30.0;
    let mut world = World::new(theme, seed);
    world.viewport = Vec2::new(120.0, 90.0);
    let origin = world.player.pos;
    let mut peak = 0usize;
    let mut ticks = 0u32;
    let mut wander_heading = (seed as f32 * 1.7).rem_euclid(std::f32::consts::TAU);
    let mut wander_left = 0.0f32;
    let mut last = world.player.pos;
    let mut stuck = 0;
    while world.elapsed < max_seconds {
        let dir = if tactic == "circle" {
            let phase = world.elapsed;
            let next = origin + Vec2::from_angle(phase + 0.10) * 56.0;
            (next - world.player.pos).normalized()
        } else {
            if wander_left <= 0.0 {
                wander_heading += 2.399_963;
                wander_left = 1.5 + (ticks % 7) as f32 * 0.25;
            }
            wander_left -= dt;
            // If the rat has not moved for a while it is against a wall: turn.
            if world.player.pos.dist(last) < 0.4 {
                stuck += 1;
                if stuck > 8 {
                    wander_heading += 2.0;
                    stuck = 0;
                }
            } else {
                stuck = 0;
            }
            last = world.player.pos;
            Vec2::from_angle(wander_heading)
        };
        world.update(dt, dir, false);
        while world.pending_levelups > 0 {
            let choices = world.player.loadout.generate_choices(&mut world.rng);
            if let Some(upgrade) = choices.first() {
                let heal = world.player.loadout.apply(upgrade);
                world.player.heal(heal);
            }
            world.pending_levelups -= 1;
        }
        world.player.hp = world.player.hp.min(world.player.max_hp());
        peak = peak.max(world.enemy_count());
        ticks += 1;
        if world.finished() {
            break;
        }
    }
    Survival {
        seconds: world.elapsed,
        peak_enemies: peak,
        kills: world.kills,
        level: world.player.level,
    }
}

#[cfg(test)]
mod stats {
    use super::*;
    use crate::game::levelgen::generate_once;

    fn env_seeds(var: &str, default: u64) -> u64 {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    fn print_dist(label: &str, values: &mut [f64]) {
        if values.is_empty() {
            println!("  {label:<38} (none)");
            return;
        }
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        println!(
            "  {label:<38} min {:>6.1}  p10 {:>6.1}  med {:>6.1}  p90 {:>6.1}  max {:>6.1}  mean {:>6.1}",
            values[0],
            at(0.10),
            at(0.5),
            at(0.90),
            values[values.len() - 1],
            mean
        );
    }

    /// The acceptance table. `RATRUN_MAP_STATS=<seeds per theme>`.
    #[test]
    fn open_world_statistics() {
        let Ok(raw) = std::env::var("RATRUN_MAP_STATS") else {
            return;
        };
        let per_theme: u64 = raw.parse().unwrap_or(500).max(1);
        let mut open = Vec::new();
        let mut rect_w = Vec::new();
        let mut rect_h = Vec::new();
        let mut areas = Vec::new();
        let mut largest_area = Vec::new();
        let mut cell_cycles = Vec::new();
        let mut loop_makers = Vec::new();
        let mut area_cycles = Vec::new();
        let mut spaces = Vec::new();
        let mut single_opening = Vec::new();
        let mut largest_single_opening = Vec::new();
        let mut narrowest = Vec::new();
        let mut corridor_share = Vec::new();
        let mut holes = Vec::new();
        let mut useful_holes = Vec::new();
        let mut traps = Vec::new();
        let mut encounter_traps = Vec::new();
        let mut mazes = Vec::new();
        let mut failures = 0u64;
        let mut sample = String::new();
        for theme in Theme::all() {
            for seed in 0..per_theme {
                let level = generate_once(theme, seed, 0);
                if let Err(errors) = validate::validate(&level) {
                    failures += 1;
                    if failures <= 6 {
                        println!("VALIDATION FAILURE {theme:?} seed {seed}: {errors:#?}");
                    }
                }
                let m = measure(&level);
                if seed < 3 {
                    sample.push_str(&summary(theme, seed, &m));
                    sample.push('\n');
                }
                open.push(m.open_fraction * 100.0);
                rect_w.push(m.largest_rect.0 as f64);
                rect_h.push(m.largest_rect.1 as f64);
                areas.push(m.areas as f64);
                largest_area.push(m.largest_area as f64);
                cell_cycles.push(m.cell_cycles as f64);
                loop_makers.push(m.loop_makers as f64);
                area_cycles.push(m.area_cycles as f64);
                spaces.push(m.spaces as f64);
                single_opening.push(m.single_opening_spaces as f64);
                largest_single_opening.push(m.largest_single_opening as f64);
                narrowest.push(m.narrowest_passage as f64);
                corridor_share.push(m.corridor_share * 100.0);
                holes.push(m.holes as f64);
                useful_holes.push(m.useful_holes as f64);
                traps.push(m.traps as f64);
                encounter_traps.push(m.encounter_traps as f64);
                mazes.push(m.mazes as f64);
            }
        }
        let maps = 3.0 * per_theme as f64;
        println!("\nopen-world statistics: {per_theme} seeds x 3 themes = {maps:.0} maps, {failures} validation failures\n");
        println!("samples:\n{sample}");
        print_dist("walkable (% of grid)", &mut open);
        print_dist("largest open rect width (cells)", &mut rect_w);
        print_dist("largest open rect height (cells)", &mut rect_h);
        print_dist("named areas (start+boss+halls+rooms)", &mut areas);
        print_dist("largest named area (cells)", &mut largest_area);
        print_dist("cell-graph independent cycles", &mut cell_cycles);
        print_dist("free-standing rock masses", &mut loop_makers);
        print_dist("area-graph independent cycles", &mut area_cycles);
        print_dist("open spaces (merged areas are one)", &mut spaces);
        print_dist("spaces with a single opening", &mut single_opening);
        print_dist("largest single-opening space (cells)", &mut largest_single_opening);
        print_dist("narrowest required passage (cells)", &mut narrowest);
        print_dist("corridor share of walkable (%)", &mut corridor_share);
        print_dist("rat-holes per map", &mut holes);
        print_dist("useful rat-holes per map", &mut useful_holes);
        print_dist("traps per map", &mut traps);
        print_dist("room-mouth encounters per map", &mut encounter_traps);
        print_dist("mazes per map", &mut mazes);
        println!();
    }

    /// `RATRUN_MAP_DUMP=<dir>`: one whole-map PPM per seed.
    #[test]
    fn open_world_dump() {
        let Ok(dir) = std::env::var("RATRUN_MAP_DUMP") else {
            return;
        };
        let per_theme = env_seeds("RATRUN_MAP_SEEDS", 6);
        std::fs::create_dir_all(&dir).unwrap();
        for theme in Theme::playable() {
            for seed in 0..per_theme {
                let level = generate_once(theme, seed, 0);
                let path = format!("{dir}/{theme:?}-{seed:02}.ppm");
                dump_ppm(&level, &path, 3).unwrap();
                println!("{path}  {}", summary(theme, seed, &measure(&level)));
            }
        }
    }

    /// `RATRUN_SURVIVAL=<max seconds>`: the circling and wandering probes.
    #[test]
    fn open_world_survival() {
        let Ok(max) = std::env::var("RATRUN_SURVIVAL") else {
            return;
        };
        let max: f32 = max.parse().unwrap_or(300.0);
        let seeds = env_seeds("RATRUN_SURVIVAL_SEEDS", 3);
        println!("\nsurvival probe (max {max:.0} s):");
        for theme in Theme::all() {
            for tactic in ["circle", "wander"] {
                for seed in 0..seeds {
                    let r = survival(theme, seed, tactic, max);
                    println!(
                        "  {theme:?} seed {seed} {tactic:<6}: {:6.1} s  peak {:4}  kills {:4}  level {}",
                        r.seconds, r.peak_enemies, r.kills, r.level
                    );
                }
            }
        }
        println!();
    }

    /// Sanity: the measurements describe the map they are given (no simulation
    /// involved), so a metric regression is caught without a full sweep.
    #[test]
    fn measurements_describe_the_map() {
        let level = generate_once(Theme::Sewer, 0, 0);
        let m = measure(&level);
        assert!(m.open_cells > 0);
        assert!(m.open_fraction > 0.0 && m.open_fraction < 1.0);
        assert!(m.largest_component <= m.open_cells);
        assert_eq!(m.stray_cells, m.open_cells - m.largest_component);
        assert!(m.largest_rect.0 > 0 && m.largest_rect.1 > 0);
        assert!(m.areas >= 3, "start, boss and at least one room");
        assert!(m.narrowest_passage >= 1);
        assert!(m.holes > 0);
        assert!(m.traps > 0);
        let _ = yes_cell(&level.tiles);
    }

    fn yes_cell(tiles: &TileMap) -> bool {
        tiles.cols() == MAP_COLS && tiles.rows() == MAP_ROWS
    }
}


