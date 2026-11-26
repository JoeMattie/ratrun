//! **Playability gate.** Every generated map is proven before it is used.
//!
//! Owner: *generator worker B* (this is the baseline implementation: the checks
//! below plus the connectivity search they share).
//!
//! What "playable" means here:
//!
//! 1. The player spawns on open, wall-clear floor in a start room.
//! 2. The **small** size class (the rat) can reach every room of the map from
//!    the start — mazes included — using physical cell passability and the
//!    one-way-door direction rule.
//! 3. The **large** size class can reach every room the horde has to enter
//!    (a rat-hole is not a route for them).
//! 4. Each maze's reward is reachable for the player and its exit leads back
//!    into reachable floor, in the allowed direction. The section's
//!    [`WallKind::Maze`](crate::game::art_walls::WallKind::Maze) ring is closed
//!    apart from its entry gap and its one-way door, and there are at most two
//!    sections.
//! 5. There is no boss arena and nothing is furnished for one: the boss arrives
//!    where the rat is (batch 12) and its fight rectangle is built at that
//!    moment, so no map owes a boss a room.
//! 6. The placed objects agree with the grid: every rat-hole sits on a
//!    rat-hole cell facing along its passage, every one-way door animates a
//!    door cell in the same direction, every maze has a reward pedestal, and
//!    no trap sits in a wall or on the spawn point.
//!
//! A failure is not fatal: [`super::generate`] retries with a salted seed, and
//! in debug builds reports the list of problems.

use std::collections::VecDeque;

use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
use crate::game::collision::{cardinal_delta, cardinal_of, cardinal_vec, SizeClass, TileKind, TileMap};
use crate::game::level::{Level, RoomKind, Theme, TileRect};
use crate::math::Vec2;

use super::maze;

use crate::game::collision::TILE_F;

/// 8-connected neighbourhood, cardinals first.
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

/// Cell containing a world position.
#[inline]
pub fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / TILE_F).floor() as i32, (p.y / TILE_F).floor() as i32)
}

/// Cells the player must be able to enter.
#[deprecated(note = "no map has a boss arena any more (batch 12)")]
pub const BOSS_ARENA_MIN_CELLS: i32 = 400;

/// Run every check and collect every problem (empty = playable).
pub fn validate(level: &Level) -> Result<(), Vec<String>> {
    let tiles = &level.tiles;
    let mut errors = Vec::new();
    let start = cell_of(level.start);

    // 1. Spawn point.
    if !tiles.passable(start.0, start.1, SizeClass::Small) {
        errors.push(format!(
            "start point {start:?} is not walkable floor ({:?})",
            tiles.get(start.0, start.1)
        ));
    }
    if !tiles.circle_clear(level.start, 3.0, SizeClass::Small) {
        errors.push(format!("start point {:?} overlaps a wall", level.start));
    }
    if level
        .start_room()
        .is_none_or(|r| !r.rect.contains_world(level.start))
    {
        errors.push("no start room contains the spawn point".to_string());
    }

    // 2./3. Connectivity per size class.
    let small = reachable(tiles, start, SizeClass::Small);
    let large = reachable(tiles, start, SizeClass::Large);
    for room in &level.rooms {
        if !reaches_rect(&small, tiles, room.rect) {
            errors.push(format!(
                "player cannot reach the {:?} room {:?}",
                room.kind, room.rect
            ));
        }
        let must_enter = matches!(room.kind, RoomKind::Hall | RoomKind::Room);
        if must_enter && !reaches_rect(&large, tiles, room.rect) {
            errors.push(format!(
                "the large size class cannot reach the {:?} room {:?}",
                room.kind, room.rect
            ));
        }
    }


    // 4. Mazes: reward inside, exit usable and leading somewhere reachable.
    for (i, maze) in level.mazes.iter().enumerate() {
        let reward_cell = cell_of(maze.reward);
        if !tiles.passable(reward_cell.0, reward_cell.1, SizeClass::Small) {
            errors.push(format!("maze {i}: reward {reward_cell:?} is in a wall"));
        } else if !is_reached(&small, tiles, reward_cell) {
            errors.push(format!("maze {i}: reward {reward_cell:?} is unreachable"));
        }
        match tiles.one_way_dir(maze.exit.0, maze.exit.1) {
            Some(dir) if dir == maze.exit_dir => {}
            Some(dir) => errors.push(format!(
                "maze {i}: exit {:?} allows {dir:?}, section says {:?}",
                maze.exit, maze.exit_dir
            )),
            None => errors.push(format!(
                "maze {i}: exit {:?} is not a one-way door",
                maze.exit
            )),
        }
        // The door itself must be walkable *from the inside*: the flood only
        // enters a door cell along its allowed direction, so a reached exit is
        // one the rat can actually leave through after the cache.
        if !is_reached(&small, tiles, maze.exit) {
            errors.push(format!(
                "maze {i}: exit {:?} cannot be reached from inside the warren",
                maze.exit
            ));
        }
        let (dx, dy) = cardinal_delta(maze.exit_dir);
        let outside = (maze.exit.0 + dx, maze.exit.1 + dy);
        if !tiles.passable(outside.0, outside.1, SizeClass::Small) {
            errors.push(format!("maze {i}: outside cell {outside:?} is a wall"));
        } else if !is_reached(&small, tiles, outside) {
            errors.push(format!("maze {i}: outside cell {outside:?} is unreachable"));
        }
        if !is_reached(&small, tiles, maze.entry) {
            errors.push(format!("maze {i}: entry {:?} is unreachable", maze.entry));
        }
        // The breach is within about 32 cells of the rat's spawn, so the first
        // cache is an opening choice and not a hunt (the designer's problem 1).
        let gap = (maze.entry.0 - start.0)
            .abs()
            .max((maze.entry.1 - start.1).abs());
        if gap > maze::ENTRY_NEAR_CELLS {
            errors.push(format!(
                "maze {i}: entry {:?} is {gap} cells from the start (max {})",
                maze.entry,
                maze::ENTRY_NEAR_CELLS
            ));
        }
        // The one-way door is never on the breach's wall: the maze is a
        // shortcut through to another side, not a U-turn back into the mob.
        if let (Some(entry_side), Some(exit_side)) = (
            maze::side_of_ring(maze.bounds, maze.entry),
            maze::side_of_ring(maze.bounds, maze.exit),
        ) {
            if entry_side == exit_side {
                errors.push(format!(
                    "maze {i}: exit {:?} is on the entry's wall ({entry_side:?})",
                    maze.exit
                ));
            }
        } else {
            errors.push(format!(
                "maze {i}: entry {:?} or exit {:?} is not on the partition ring",
                maze.entry, maze.exit
            ));
        }
        // The ring the section reports is the ring its cells are in, and the
        // cache cell is the pedestal's cell.
        if maze.ring != maze::maze_ring(maze.bounds) {
            errors.push(format!(
                "maze {i}: ring {:?} is not the ring around {:?}",
                maze.ring, maze.bounds
            ));
        }
        if cell_of(maze.reward) != maze.reward_cell {
            errors.push(format!(
                "maze {i}: reward_cell {:?} does not match reward {:?}",
                maze.reward_cell, maze.reward
            ));
        }
        // At least one interior loop: a warren with none is a single
        // unavoidable corridor once the horde is behind you.
        if maze::maze_loops(tiles, maze) < 1 {
            errors.push(format!("maze {i}: the warren has no interior loop"));
        }
        // The fork is public walkway floor outside the ring, one trail's walk
        // from the breach.
        if maze.ring.contains(maze.fork.0, maze.fork.1) {
            errors.push(format!(
                "maze {i}: fork {:?} is inside the partition ring",
                maze.fork
            ));
        }
        if !tiles.passable(maze.fork.0, maze.fork.1, SizeClass::Small) {
            errors.push(format!("maze {i}: fork {:?} is not floor", maze.fork));
        } else if !is_reached(&small, tiles, maze.fork) {
            errors.push(format!(
                "maze {i}: fork {:?} is not public floor",
                maze.fork
            ));
        }
        // Three gnaw marks cover the final `TRAIL_CELLS` cells between the fork
        // and the breach: the walk between them must be that trail, not a
        // corner-cut through the warren.
        match maze::trail_walk_cells(tiles, maze) {
            Some(cells) if (2..=maze::TRAIL_CELLS).contains(&cells) => {}
            Some(cells) => errors.push(format!(
                "maze {i}: the trail is {cells} cells (2..={})",
                maze::TRAIL_CELLS
            )),
            None => errors.push(format!(
                "maze {i}: fork {:?} is not on the walkway from its breach",
                maze.fork
            )),
        }
        // The ring is closed apart from the entry gap and the door, so the
        // section really is a pocket with a single one-way way out.
        let ring = maze::maze_ring(maze.bounds);
        let mut gap: Vec<(i32, i32)> = Vec::new();
        for (cx, cy) in maze::ring_border(ring) {
            if tiles.one_way_dir(cx, cy).is_some() {
                if (cx, cy) != maze.exit {
                    errors.push(format!("maze {i}: extra one-way door at ({cx}, {cy})"));
                }
            } else if tiles.get(cx, cy).is_open() {
                gap.push((cx, cy));
            }
        }
        if gap.is_empty() || gap.len() > 2 {
            errors.push(format!(
                "maze {i}: {} open ring cells, expected a one- or two-cell entry",
                gap.len()
            ));
        }
        if !gap.iter().all(|cell| {
            (cell.0 - maze.entry.0).abs() <= 1 && (cell.1 - maze.entry.1).abs() <= 1
        }) {
            errors.push(format!("maze {i}: the ring is open away from its entry"));
        }
        if !maze.bounds.contains(reward_cell.0, reward_cell.1) {
            errors.push(format!(
                "maze {i}: reward {reward_cell:?} is outside the section"
            ));
        }
        // The door must open *out* of the section, not back into it.
        let (dx, dy) = cardinal_delta(maze.exit_dir);
        let exit_centre = TileMap::tile_rect(maze.exit.0, maze.exit.1).center();
        if (exit_centre - maze.bounds.center_world()).dot(Vec2::new(dx as f32, dy as f32)) <= 0.0 {
            errors.push(format!(
                "maze {i}: exit {:?} does not open out of the maze",
                maze.exit
            ));
        }
    }

    // 6. A map never carries more maze sections than the data model allows.
    if level.mazes.len() > maze::MAX_MAZES {
        errors.push(format!(
            "{} maze sections, at most {} fit on a map",
            level.mazes.len(),
            maze::MAX_MAZES
        ));
    }

    // 7. Placed objects must describe the grid they animate.
    check_objects(level, &mut errors);

    // 7b. Shop holes: a solid wall cell with reachable floor in front of it.
    check_shop_holes(level, &small, &mut errors);

    // 8. Floor hazards: on public, reachable floor, off
    // every arena/start/maze reservation, and never without a dry way past.
    check_hazards(level, &small, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The cells a prop's world rectangle covers.
fn prop_cells(rect: crate::game::level::Rectf) -> TileRect {
    let x = (rect.x / TILE_F).floor() as i32;
    let y = (rect.y / TILE_F).floor() as i32;
    let w = (rect.right() / TILE_F).ceil() as i32 - x;
    let h = (rect.bottom() / TILE_F).ceil() as i32 - y;
    TileRect::new(x, y, w, h)
}

/// The drip shaft (Sewer), steam grates (Laundry) and ink pads (Printworks).
///
/// Each is walkable. Every hazard needs a dry route at least two cells wide
/// past it, and none may
/// sit on the start room, a maze section or a threshold.
fn check_hazards(level: &Level, small: &[bool], errors: &mut Vec<String>) {
    let tiles = &level.tiles;
    let theme = level.theme;
    let start_room = level.start_room().map(|room| room.rect);
    let clear = |cell: (i32, i32), what: &str, errors: &mut Vec<String>| {
        if !tiles.get(cell.0, cell.1).is_open() {
            errors.push(format!("{what} at {cell:?} is not open floor"));
            return false;
        }
        if !is_reached(small, tiles, cell) {
            errors.push(format!("{what} at {cell:?} is unreachable"));
        }
        if start_room.is_some_and(|room| room.contains(cell.0, cell.1)) {
            errors.push(format!("{what} at {cell:?} is in the start room"));
        }
        for slot in &level.mazes {
            if slot.bounds.inflate(1).contains(cell.0, cell.1) {
                errors.push(format!("{what} at {cell:?} is inside a maze section"));
            }
        }
        if near_threshold(tiles, cell) {
            errors.push(format!("{what} at {cell:?} is on a threshold"));
        }
        if !super::hazards::dry_route(tiles, cell) {
            errors.push(format!(
                "{what} at {cell:?} has no two-cell dry route past it"
            ));
        }
        true
    };

    for grate in &level.objects.steam_grates {
        clear(grate.cell, "steam grate", errors);
    }
    for pad in &level.objects.ink_pads {
        // The generator anchors a pad at its first cell; the dry route is
        // measured there (a grown pad spreads into the corner it was found
        // against, so its far cell can sit in the corner itself).
        let cell = (pad.cells.x, pad.cells.y);
        clear(cell, "ink pad", errors);
        for (cx, cy) in pad.cells.cells() {
            if !tiles.get(cx, cy).is_open() {
                errors.push(format!("ink pad cell ({cx}, {cy}) is not floor"));
            }
        }
    }

    let expected = match theme {
        Theme::Laundry => Some(level.objects.steam_grates.len()),
        Theme::Printworks => Some(level.objects.ink_pads.len()),
        _ => None,
    };
    if let Some(count) = expected {
        let other = match theme {
            Theme::Laundry => level.objects.ink_pads.len(),
            _ => level.objects.steam_grates.len(),
        };
        if other != 0 {
            errors.push(format!("{theme:?} carries another theme's hazard"));
        }
        let (lo, hi) = super::hazards::HAZARDS;
        if !(lo..=hi).contains(&count) {
            errors.push(format!(
                "{theme:?} placed {count} hazards, the design says {lo}-{hi}"
            ));
        }
    } else if !level.objects.steam_grates.is_empty() || !level.objects.ink_pads.is_empty() {
        errors.push(format!("{theme:?} has hazards it should not have"));
    }

    if theme == Theme::Sewer {
        if let Some(opening) = &level.objects.sewer_opening {
            clear(opening.drip_shaft.cell, "drip shaft", errors);
        } else {
            errors.push("Sewer is missing its opening encounters".to_owned());
        }
    }
}

/// Is any wall cell within two cells of `cell` a threshold (doorway, one-way
/// door or rat-hole)? A hazard in front of one would charge a toll on the only
/// way through.
fn near_threshold(tiles: &TileMap, cell: (i32, i32)) -> bool {
    for dy in -2..=2 {
        for dx in -2..=2 {
            if matches!(
                tiles.get(cell.0 + dx, cell.1 + dy),
                TileKind::Wall(
                    WallKind::Doorway { .. }
                        | WallKind::OneWayDoor { .. }
                        | WallKind::RatHole { .. }
                )
            ) {
                return true;
            }
        }
    }
    false
}

/// The mouse-hole shops: each is cut into a wall cell that stays solid (so
/// nothing walks into it) and is entered by touching its face from the mouth
/// cell, which must be open floor the rat can reach — a shop behind a wall is a
/// shop nobody can open.
fn check_shop_holes(level: &Level, small: &[bool], errors: &mut Vec<String>) {
    let tiles = &level.tiles;
    for (i, hole) in level.objects.shop_holes.iter().enumerate() {
        if hole.index != i {
            errors.push(format!(
                "shop hole {i} at {:?} carries index {}",
                hole.cell, hole.index
            ));
        }
        if tiles.passable(hole.cell.0, hole.cell.1, SizeClass::Small) {
            errors.push(format!("shop hole {i} at {:?} is not in a wall", hole.cell));
        }
        let (dx, dy) = (hole.mouth.0 - hole.cell.0, hole.mouth.1 - hole.cell.1);
        if dx.abs() + dy.abs() != 1 {
            errors.push(format!(
                "shop hole {i}: mouth {:?} is not beside its wall cell {:?}",
                hole.mouth, hole.cell
            ));
        }
        if !tiles.get(hole.mouth.0, hole.mouth.1).is_open() {
            errors.push(format!(
                "shop hole {i}: mouth {:?} is not open floor",
                hole.mouth
            ));
        } else if !is_reached(small, tiles, hole.mouth) {
            errors.push(format!(
                "shop hole {i}: mouth {:?} is unreachable",
                hole.mouth
            ));
        }
    }
}

fn check_objects(level: &Level, errors: &mut Vec<String>) {
    let tiles = &level.tiles;
    for hole in &level.objects.rat_holes {
        match tiles.get(hole.cell.0, hole.cell.1) {
            TileKind::Wall(WallKind::RatHole { axis }) => {
                let passage_runs_north_south = hole.view.facing.y.abs() >= hole.view.facing.x.abs();
                let expected = if passage_runs_north_south {
                    WallAxis::Horizontal
                } else {
                    WallAxis::Vertical
                };
                if axis != expected {
                    errors.push(format!(
                        "rat-hole {:?} faces across its wall ({axis:?})",
                        hole.cell
                    ));
                }
                // Both landings must be clear 16 px floor a large pursuer can
                // stand on (the hole itself is small-class only). The facing
                // sign is the public approach side, so the mouths are one cell
                // either side along that axis.
                let (dx, dy) = if passage_runs_north_south {
                    (0, 1)
                } else {
                    (1, 0)
                };
                for sign in [1, -1] {
                    let mut cell = hole.cell;
                    for _ in 0..3 {
                        let next = (cell.0 + sign * dx, cell.1 + sign * dy);
                        if !matches!(
                            tiles.get(next.0, next.1),
                            TileKind::Wall(WallKind::RatHole { .. })
                        ) {
                            break;
                        }
                        cell = next;
                    }
                    let mouth = (cell.0 + sign * dx, cell.1 + sign * dy);
                    if !tiles.get(mouth.0, mouth.1).is_open() {
                        errors.push(format!(
                            "rat-hole {:?} mouth {mouth:?} is not open floor",
                            hole.cell
                        ));
                    } else if !tiles.passable(mouth.0, mouth.1, SizeClass::Large) {
                        errors.push(format!(
                            "rat-hole {:?} landing {mouth:?} is too tight for the large class",
                            hole.cell
                        ));
                    }
                }
            }
            other => errors.push(format!(
                "rat-hole {:?} is not a rat-hole cell ({other:?})",
                hole.cell
            )),
        }
    }
    for door in &level.objects.one_way_doors {
        match tiles.one_way_dir(door.cell.0, door.cell.1) {
            Some(dir) => {
                if (door.view.facing - cardinal_vec(dir)).len_sq() > 1e-3 {
                    errors.push(format!(
                        "one-way door {:?} faces away from {dir:?}",
                        door.cell
                    ));
                }
            }
            None => errors.push(format!(
                "one-way door {:?} is not a door cell",
                door.cell
            )),
        }
    }
    for (cx, cy, kind) in tiles.iter_walls() {
        if matches!(kind, WallKind::OneWayDoor { .. }) && level.objects.door_at(cx, cy).is_none() {
            errors.push(format!("one-way door cell ({cx}, {cy}) has no door object"));
        }
        // The mirror of the object check above: a hole cell with no object is
        // passable but never animates, never squeezes and never tells the fog
        // it was crossed (`gates::passage_system` walks the objects).
        if matches!(kind, WallKind::RatHole { .. })
            && !level.objects.rat_holes.iter().any(|h| h.cell == (cx, cy))
        {
            errors.push(format!("rat-hole cell ({cx}, {cy}) has no rat-hole object"));
        }
    }
    if level.objects.rewards.len() != level.mazes.len() {
        errors.push(format!(
            "{} maze rewards for {} maze sections",
            level.objects.rewards.len(),
            level.mazes.len()
        ));
    }
    for pos in level
        .objects
        .sticky_traps
        .iter()
        .map(|t| t.pos)
        .chain(level.objects.mousetraps.iter().map(|t| t.pos))
    {
        let cell = cell_of(pos);
        if !tiles.get(cell.0, cell.1).is_open() {
            errors.push(format!("trap at {pos:?} is not on open floor"));
        }
        if (pos - level.start).len() <= 6.0 * TILE_F {
            errors.push(format!("trap at {pos:?} sits on the spawn point"));
        }
        if level
            .start_room()
            .is_some_and(|room| room.rect.contains(cell.0, cell.1))
        {
            errors.push(format!("trap at {pos:?} is inside the start room"));
        }
        if level
            .mazes
            .iter()
            .any(|maze| maze.bounds.contains(cell.0, cell.1))
        {
            errors.push(format!("trap at {pos:?} is inside a maze"));
        }
    }
}

/// Flood fill from `from` for one size class, honouring the one-way-door
/// direction rule (a door cell may be entered, and left, only along its
/// allowed direction). Diagonal steps are not allowed through a door cell and
/// must not cut a corner. Returns a row-major bitmap of reachable cells.
pub fn reachable(tiles: &TileMap, from: (i32, i32), class: SizeClass) -> Vec<bool> {
    let cols = tiles.cols();
    let rows = tiles.rows();
    let mut seen = vec![false; (cols * rows) as usize];
    if !tiles.passable(from.0, from.1, class) {
        return seen;
    }
    seen[(from.1 * cols + from.0) as usize] = true;
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    queue.push_back(from);
    while let Some((cx, cy)) = queue.pop_front() {
        for (dx, dy) in OFFS {
            let (nx, ny) = (cx + dx, cy + dy);
            if !tiles.in_bounds(nx, ny) {
                continue;
            }
            let i = (ny * cols + nx) as usize;
            if seen[i] {
                continue;
            }
            let ok = if dx == 0 || dy == 0 {
                let dir = cardinal_of(dx, dy).expect("cardinal offset");
                let enters = tiles.travel_ok(nx, ny, class, dir);
                let leaves = tiles
                    .one_way_dir(cx, cy)
                    .is_none_or(|allowed| allowed == dir);
                enters && leaves
            } else {
                // No door cell may be crossed diagonally, and never squeeze
                // between two blocked cells.
                tiles.passable(nx, ny, class)
                    && tiles.one_way_dir(nx, ny).is_none()
                    && tiles.one_way_dir(cx, cy).is_none()
                    && (tiles.passable(cx + dx, cy, class) || tiles.passable(cx, cy + dy, class))
            };
            if ok {
                seen[i] = true;
                queue.push_back((nx, ny));
            }
        }
    }
    seen
}

/// Is one cell reached by a [`reachable`] bitmap?
pub fn is_reached(seen: &[bool], tiles: &TileMap, cell: (i32, i32)) -> bool {
    if !tiles.in_bounds(cell.0, cell.1) {
        return false;
    }
    seen[(cell.1 * tiles.cols() + cell.0) as usize]
}

/// Does any open cell of `rect` lie on a reached patch? Rooms may contain
/// props, so a single blocked cell is not a failure.
pub fn reaches_rect(seen: &[bool], tiles: &TileMap, rect: TileRect) -> bool {
    let inner = if rect.w > 4 && rect.h > 4 {
        rect.inflate(-2)
    } else {
        rect
    };
    let reached = inner
        .cells()
        .any(|(cx, cy)| is_reached(seen, tiles, (cx, cy)));
    reached
}

/// The reachable cell count for a class (tests, benchmark reporting).
pub fn reachable_cells(tiles: &TileMap, from: (i32, i32), class: SizeClass) -> usize {
    reachable(tiles, from, class).iter().filter(|r| **r).count()
}

/// Cardinal direction an entity travelling from `a` to `b` uses, `None` when
/// they are not orthogonally adjacent.
pub fn travel_dir(a: (i32, i32), b: (i32, i32)) -> Option<Cardinal> {
    cardinal_of(b.0 - a.0, b.1 - a.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_traps::{RatHoleView, StickyTrapView};
    use crate::game::art_walls::WallAxis;
    use crate::game::level::{MazeSection, RatHole, Theme};

    /// A cheap first-attempt map: stage A, its props and the rat-hole objects,
    /// without mazes, traps or navigation.
    fn layout_only_level(theme: Theme, seed: u64) -> Level {
        use crate::game::level::{level_rng, level_seed, MAP_COLS, MAP_ROWS};
        use crate::game::levelgen::layout::{build_layout, furnish};

        let mut rng = level_rng(theme, seed, 0);
        let art_seed = level_seed(theme, seed, 0) as u32;
        let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, art_seed);
        let layout = build_layout(&mut tiles, &mut rng, theme);
        let props = furnish(&mut tiles, &layout, &mut rng, theme);
        tiles.rebuild_masks();
        let mut level = Level::empty(theme);
        level.seed = seed;
        level.arena = tiles.world_size();
        level.start = layout.start;
        level.rooms = layout.rooms.clone();
        level.boss = layout.boss.clone();
        level.tiles = tiles;
        level.props = props;
        // The objects the placement stage would report for these tiles.
        for (cx, cy, kind) in level.tiles.iter_walls() {
            if let WallKind::RatHole { .. } = kind {
                level.objects.rat_holes.push(RatHole {
                    cell: (cx, cy),
                    view: RatHoleView::new(TileMap::tile_rect(cx, cy).center(), Vec2::new(1.0, 0.0)),
                });
            }
        }
        level
    }

    #[test]
    fn every_theme_and_seed_is_playable() {
        for theme in Theme::all() {
            for seed in [0u64, 1, 7, 99, 1234, 987_654] {
                let level = Level::generate(theme, seed);
                if let Err(errors) = validate(&level) {
                    panic!("{theme:?} seed {seed} is not playable: {errors:#?}");
                }
            }
        }
    }

    #[test]
    fn the_generator_validates_on_its_first_attempt() {
        // A retry is a safety net, not the normal path: if a seed needed one,
        // `Level::generate` would silently map a seed to a different (salted)
        // map than `generate_once(.., 0)`.
        for theme in Theme::all() {
            for seed in 0..12u64 {
                let level = super::super::generate_once(theme, seed, 0);
                if let Err(errors) = validate(&level) {
                    panic!("{theme:?} seed {seed} needed a retry: {errors:#?}");
                }
            }
        }
    }

    /// Batch 12 removed the boss arena. Nothing may reintroduce one quietly:
    /// no generated map (any theme, any seed) carries a [`RoomKind::Boss`] room,
    /// arena furniture or an arena light treatment, and the validator passes
    /// every one of those maps.
    #[test]
    fn no_generated_level_has_a_boss_arena() {
        use crate::game::level::{LightFixtureKind, LightZoneKind};
        for theme in Theme::playable() {
            for seed in 0..20u64 {
                let level = super::super::generate_once(theme, seed, 0);
                let ctx = format!("{theme:?} seed {seed}");
                assert_eq!(
                    level.rooms_of(RoomKind::Boss).count(),
                    0,
                    "{ctx}: a boss arena room"
                );
                assert!(level.boss_room().is_none(), "{ctx}: boss_room()");
                assert_eq!(
                    level.boss.floor,
                    TileRect::new(0, 0, 0, 0),
                    "{ctx}: an arena floor"
                );
                assert!(level.boss.dock.is_none(), "{ctx}: an arena dock");
                assert!(level.boss.bumpers.is_empty(), "{ctx}: arena bumpers");
                assert!(level.boss.escape_hole.is_none(), "{ctx}: an escape hole");
                assert!(level.boss.mouths.is_empty(), "{ctx}: arena mouths");
                assert!(
                    !level
                        .objects
                        .light_zones
                        .iter()
                        .any(|z| z.kind == LightZoneKind::Arena),
                    "{ctx}: an Arena light zone"
                );
                assert!(
                    !level
                        .objects
                        .lights
                        .iter()
                        .any(|l| l.kind == LightFixtureKind::Arena),
                    "{ctx}: an Arena light fixture"
                );
                if let Err(errors) = validate(&level) {
                    panic!("{ctx}: a map without an arena is not playable: {errors:#?}");
                }
            }
        }
    }

    #[test]
    fn a_walled_off_start_fails_validation() {
        let mut level = Level::generate(Theme::Sewer, 3);
        // Seal the start room inside a solid ring.
        let room = level.start_room().expect("start room").rect.inflate(1);
        for (cx, cy) in room.cells() {
            let on_edge = cx == room.x
                || cy == room.y
                || cx == room.right() - 1
                || cy == room.bottom() - 1;
            if on_edge {
                level.tiles.set(
                    cx,
                    cy,
                    crate::game::collision::TileKind::Wall(
                        crate::game::art_walls::WallKind::Solid,
                    ),
                );
            }
        }
        let errors = validate(&level).expect_err("sealed start must fail");
        assert!(
            errors.iter().any(|e| e.contains("cannot reach")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_rat_hole_connects_the_small_class_only() {
        // A walled corridor with one rat-hole: the small class gets through,
        // the large class does not.
        let mut tiles = TileMap::new(24, 8, Theme::Sewer, 1);
        tiles.fill_cells(10, 0, 2, 8, crate::game::collision::TileKind::Wall(
            crate::game::art_walls::WallKind::Solid,
        ));
        tiles.fill_cells(
            10,
            4,
            2,
            1,
            crate::game::collision::TileKind::Wall(
                crate::game::art_walls::WallKind::RatHole {
                    axis: WallAxis::Vertical,
                },
            ),
        );
        tiles.rebuild_masks();
        let small = reachable(&tiles, (2, 4), SizeClass::Small);
        let large = reachable(&tiles, (2, 4), SizeClass::Large);
        assert!(is_reached(&small, &tiles, (20, 4)), "the rat is blocked");
        assert!(
            !is_reached(&large, &tiles, (20, 4)),
            "a large agent squeezed through a rat-hole"
        );
    }

    #[test]
    fn travel_dir_matches_cardinals() {
        assert_eq!(travel_dir((1, 1), (2, 1)), Some(Cardinal::East));
        assert_eq!(travel_dir((1, 1), (1, 2)), Some(Cardinal::South));
        assert_eq!(travel_dir((1, 1), (2, 2)), None);
    }

    #[test]
    fn a_seed_sweep_always_generates_a_playable_map() {
        // `Level::generate` retries a salted attempt when a map fails, and
        // panics in debug when even that fails: this sweep proves the retry
        // path never has to give up (and that stage B never produces a map
        // with a sealed maze, a bad hole or a trap on the spawn).
        for theme in Theme::all() {
            for seed in 0..40u64 {
                let level = Level::generate(theme, seed);
                if let Err(errors) = validate(&level) {
                    panic!("{theme:?} seed {seed} generated an unplayable map: {errors:#?}");
                }
            }
        }
    }

    #[test]
    fn a_trap_on_the_spawn_point_fails_validation() {
        let mut level = Level::generate(Theme::Sewer, 3);
        let spawn = level.start;
        level.objects.sticky_traps.push(StickyTrapView::new(
            spawn,
            Vec2::new(1.0, 0.0),
        ));
        let errors = validate(&level).expect_err("a trap on the spawn must fail");
        assert!(
            errors.iter().any(|e| e.contains("spawn")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_trap_inside_a_maze_fails_validation() {
        let Some(level) = (0..40u64)
            .map(|seed| Level::generate(Theme::Kitchen, seed))
            .find(|level| !level.mazes.is_empty())
        else {
            return; // no maze in this sweep: nothing to check
        };
        let mut level = level;
        let entry = level.mazes[0].entry;
        level.objects.sticky_traps.push(StickyTrapView::new(
            TileMap::tile_rect(entry.0, entry.1).center(),
            Vec2::new(1.0, 0.0),
        ));
        let errors = validate(&level).expect_err("a trap inside a maze must fail");
        assert!(
            errors.iter().any(|e| e.contains("inside a maze")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_rat_hole_object_off_its_wall_fails_validation() {
        let mut level = Level::generate(Theme::Kitchen, 5);
        level.objects.rat_holes.push(RatHole {
            cell: (0, 0),
            view: RatHoleView::new(Vec2::new(0.0, 0.0), Vec2::new(0.0, 1.0)),
        });
        let errors = validate(&level).expect_err("a hole without its wall must fail");
        assert!(
            errors.iter().any(|e| e.contains("rat-hole")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_rat_hole_cell_without_its_object_fails_validation() {
        let mut level = Level::generate(Theme::Kitchen, 5);
        assert!(
            !level.objects.rat_holes.is_empty(),
            "the fixture map must carry a rat-hole"
        );
        level.objects.rat_holes.remove(0);
        let errors = validate(&level).expect_err("an unanimated hole must fail");
        assert!(
            errors.iter().any(|e| e.contains("has no rat-hole object")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_shop_hole_with_a_walled_or_sealed_mouth_fails_validation() {
        let level = Level::generate(Theme::Kitchen, 5);
        assert!(
            !level.objects.shop_holes.is_empty(),
            "the fixture map must carry a shop hole"
        );
        // The mouth turned into a wall: nothing to stand on.
        let mut walled = Level::generate(Theme::Kitchen, 5);
        let mouth = walled.objects.shop_holes[0].mouth;
        walled
            .tiles
            .set(mouth.0, mouth.1, TileKind::Wall(WallKind::Solid));
        walled.tiles.rebuild_masks();
        let errors = validate(&walled).expect_err("a walled shop mouth must fail");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("shop hole 0") && e.contains("is not open floor")),
            "unexpected report: {errors:#?}"
        );
        // The hole's wall cell knocked out: the shop is no longer in a wall.
        let mut opened = Level::generate(Theme::Kitchen, 5);
        let cell = opened.objects.shop_holes[0].cell;
        opened.tiles.set(cell.0, cell.1, TileKind::Open);
        opened.tiles.rebuild_masks();
        let errors = validate(&opened).expect_err("a shop hole in open floor must fail");
        assert!(
            errors.iter().any(|e| e.contains("is not in a wall")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_maze_exit_walled_in_from_the_inside_fails_validation() {
        // Find a map with a maze, then wall up the cell just inside its door.
        for seed in 0..64u64 {
            let mut level = Level::generate(Theme::Sewer, seed);
            let Some(maze) = level.mazes.first().copied() else {
                continue;
            };
            let (dx, dy) = cardinal_delta(maze.exit_dir);
            let inside = (maze.exit.0 - dx, maze.exit.1 - dy);
            level
                .tiles
                .set(inside.0, inside.1, TileKind::Wall(WallKind::Solid));
            level.tiles.rebuild_masks();
            let errors = validate(&level).expect_err("a walled-in exit must fail");
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("cannot be reached from inside the warren")),
                "seed {seed}: unexpected report: {errors:#?}"
            );
            return;
        }
        panic!("no Sewer seed in 0..64 carries a maze");
    }

    #[test]
    fn a_door_cell_without_its_object_fails_validation() {
        let mut level = Level::generate(Theme::Lab, 6);
        // A door cell on the map's own wall ring, with no object.
        let cell = (2, 2);
        level
            .tiles
            .set(cell.0, cell.1, TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::East,
            }));
        level.tiles.rebuild_masks();
        let errors = validate(&level).expect_err("an unanimated door must fail");
        assert!(
            errors.iter().any(|e| e.contains("has no door object")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn more_than_two_mazes_fails_validation() {
        let mut level = Level::generate(Theme::Sewer, 2);
        let bounds = TileRect::new(20, 20, 10, 10);
        let section = MazeSection {
            bounds,
            ring: maze::maze_ring(bounds),
            entry: (20, 20),
            fork: (20, 19),
            exit: (29, 20),
            exit_dir: Cardinal::East,
            reward: TileMap::tile_rect(24, 24).center(),
            reward_cell: (24, 24),
        };
        level.mazes = vec![section; 3];
        let errors = validate(&level).expect_err("three mazes must fail");
        assert!(
            errors.iter().any(|e| e.contains("at most 2")),
            "unexpected report: {errors:#?}"
        );
    }

    #[test]
    fn a_sealed_maze_fails_validation() {
        let mut checked = false;
        for theme in Theme::all() {
            for seed in 0..8u64 {
                let mut level = Level::generate(theme, seed);
                let Some(section) = level.mazes.first().copied() else {
                    continue;
                };
                // Wall its entry gap up: the only way out is the one-way door,
                // which the player cannot come back through.
                let ring = maze::maze_ring(section.bounds);
                for (cx, cy) in ring.cells() {
                    if level.tiles.get(cx, cy).is_open() {
                        level.tiles.set(cx, cy, TileKind::Wall(WallKind::Maze));
                    }
                }
                level.tiles.rebuild_masks();
                let errors = validate(&level).expect_err("a sealed maze must fail");
                assert!(
                    errors.iter().any(|e| e.contains("maze")),
                    "unexpected report: {errors:#?}"
                );
                checked = true;
                break;
            }
            if checked {
                break;
            }
        }
        // A layout that reserves no maze slot has nothing to seal.
        assert!(checked || Theme::all().iter().all(|t| Level::generate(*t, 0).mazes.is_empty()));
    }
}
