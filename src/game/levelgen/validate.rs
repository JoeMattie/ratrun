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
//! 3. The **large** size class can reach the boss arena and every room the
//!    horde has to enter (a rat-hole is not a route for them).
//! 4. Each maze's reward is reachable for the player and its exit leads back
//!    into reachable floor, in the allowed direction. The section's
//!    [`WallKind::Maze`](crate::game::art_walls::WallKind::Maze) ring is closed
//!    apart from its entry gap and its one-way door, and there are at most two
//!    sections.
//! 5. There is exactly one big boss arena.
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

/// Cells the player must be able to enter, and the boss arena's minimum area
/// (in cells).
pub const BOSS_ARENA_MIN_CELLS: i32 = 400;

/// The arena's fight fit, on its own: the reachability proof is recomputed from
/// the level's spawn, so a test can run this against a **layout-only** map (no
/// mazes, traps or navigation) and still cover the whole arena contract, which
/// is the cheap path a 2000-seed-per-theme sweep uses.
pub fn validate_arena(level: &Level) -> Result<(), Vec<String>> {
    let start = cell_of(level.start);
    let small = reachable(&level.tiles, start, SizeClass::Small);
    let large = reachable(&level.tiles, start, SizeClass::Large);
    let mut errors = Vec::new();
    check_arena(level, &small, &large, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

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

    // 5. Exactly one boss arena, and a big one.
    let boss: Vec<&crate::game::level::Room> = level.rooms_of(RoomKind::Boss).collect();
    if boss.len() != 1 {
        errors.push(format!("expected exactly one boss arena, found {}", boss.len()));
    }
    if let Some(room) = boss.first() {
        if room.rect.area() < BOSS_ARENA_MIN_CELLS {
            errors.push(format!(
                "boss arena {:?} has {} cells, needs {BOSS_ARENA_MIN_CELLS}",
                room.rect,
                room.rect.area()
            ));
        }
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
        let must_enter = matches!(room.kind, RoomKind::Boss | RoomKind::Hall | RoomKind::Room);
        if must_enter && !reaches_rect(&large, tiles, room.rect) {
            errors.push(format!(
                "the large size class cannot reach the {:?} room {:?}",
                room.kind, room.rect
            ));
        }
    }

    // 2b./3b. The arena fits its fight and stays connected.
    check_arena(level, &small, &large, &mut errors);

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

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The four cardinal rays the Lab's "a solid in two directions" promise is
/// measured along.
const CARDINAL_RAYS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// Is a solid within [`super::layout::SOLID_REACH_PX`] of `from` along `dir`?
fn solid_along(tiles: &TileMap, from: Vec2, dir: (i32, i32)) -> Option<f32> {
    let reach = super::layout::SOLID_REACH_PX;
    let to = from + Vec2::new(dir.0 as f32 * reach, dir.1 as f32 * reach);
    tiles.segment_hit(from, to, 0.0).map(|t| t * reach)
}

/// The cells a prop's world rectangle covers.
fn prop_cells(rect: crate::game::level::Rectf) -> TileRect {
    let x = (rect.x / TILE_F).floor() as i32;
    let y = (rect.y / TILE_F).floor() as i32;
    let w = (rect.right() / TILE_F).ceil() as i32 - x;
    let h = (rect.bottom() / TILE_F).ceil() as i32 - y;
    TileRect::new(x, y, w, h)
}

/// Each boss arena must fit the fight its boss was measured against
/// (`DESIGN_REVIEW.md` §5), and must stay fully connected for both size classes
/// after the furniture is painted.
///
/// The three requirements:
///
/// - **Lab** (mower): two solid bumpers flanking a clear central floor, so on
///   every cell of that floor a solid is within
///   [`super::layout::SOLID_REACH_PX`] in at least two cardinal directions (a
///   flip lane is always on offer), a 32 px lane beside each bumper, and a
///   charging dock against a wall away from the mouth.
/// - **Kitchen** (broom): a clear 96 × 80 px encounter floor at the centre with
///   its approach ring kept clear, and furniture only on the perimeter.
/// - **Sewer** (jaguar): a rat-hole in the wall ring that leads out (both mouth
///   cells open and reachable), with the arena floor clear except at the walls.
fn check_arena(level: &Level, small: &[bool], large: &[bool], errors: &mut Vec<String>) {
    let Some(room) = level.boss_room() else {
        return;
    };
    let arena = room.rect;
    let spec = &level.boss;
    let tiles = &level.tiles;
    let floor = spec.floor;

    // The exported central floor: inside, centred, clear and big enough for the
    // theme's contract size.
    let (min_w, min_h) = super::layout::boss_floor_cells(level.theme);
    if floor.w < min_w || floor.h < min_h {
        errors.push(format!(
            "boss arena floor {floor:?} is smaller than the {min_w} x {min_h}-cell contract"
        ));
    }
    if floor.w > 0 && floor.h > 0 {
        let inside = floor.x >= arena.x
            && floor.y >= arena.y
            && floor.right() <= arena.right()
            && floor.bottom() <= arena.bottom();
        if !inside {
            errors.push(format!("boss arena floor {floor:?} is not inside {arena:?}"));
        }
        let centre = cell_of(arena.center_world());
        if !floor.contains(centre.0, centre.1) {
            errors.push(format!(
                "boss arena floor {floor:?} does not contain the arena's centre {centre:?}"
            ));
        }
        let blocked: Vec<(i32, i32)> = floor
            .cells()
            .filter(|&(cx, cy)| !tiles.get(cx, cy).is_open())
            .collect();
        if !blocked.is_empty() {
            errors.push(format!("the arena's central floor is blocked at {blocked:?}"));
        }
    }

    // Kitchen: the design's 96 x 80 px encounter floor plus the approach to the
    // pocket under the human's feet (the floor's first ring).
    if level.theme == Theme::Kitchen {
        let world = floor.to_world();
        if world.w < 96.0 || world.h < 80.0 {
            errors.push(format!(
                "the Kitchen encounter floor is {:.0} x {:.0} px, needs 96 x 80",
                world.w, world.h
            ));
        }
        let blocked: Vec<(i32, i32)> = floor
            .inflate(1)
            .cells()
            .filter(|&(cx, cy)| !tiles.get(cx, cy).is_open())
            .collect();
        if !blocked.is_empty() {
            errors.push(format!(
                "the Kitchen encounter floor's approach ring is blocked at {blocked:?}"
            ));
        }
    }

    if level.theme == Theme::Lab {
        check_lab_arena(level, arena, errors);
    }

    if level.theme == Theme::Sewer {
        match spec.escape_hole {
            Some(cell) => check_escape_hole(level, arena, cell, small, errors),
            None => errors.push(
                "the Sewer arena has no escape rat-hole in its wall ring".to_string(),
            ),
        }
    }

    // Nothing but the Lab's bumpers may be solid inside the encounter core, and
    // every prop that touches the arena belongs to its perimeter band.
    let core = arena.inflate(-super::layout::ARENA_PERIMETER_CELLS);
    if let Some((cx, cy)) = arena.cells().find(|&(cx, cy)| {
        !tiles.get(cx, cy).is_open()
            && !spec.bumpers.iter().any(|bar| bar.contains(cx, cy))
            && core.contains(cx, cy)
    }) {
        errors.push(format!(
            "the arena has an unclaimed solid at ({cx}, {cy}) inside its encounter core"
        ));
    }
    for prop in &level.props {
        let cells = prop_cells(prop.rect);
        if cells.intersects(&arena) && cells.intersects(&core) {
            errors.push(format!(
                "a prop at {:?} is not on the arena's perimeter",
                prop.rect
            ));
        }
    }

    // The arena's open floor is one connected room for both size classes: a
    // bumper or prop that isolates a cell has made the fight unplayable.
    let lost: Vec<(i32, i32)> = arena
        .cells()
        .filter(|&(cx, cy)| {
            tiles.get(cx, cy).is_open()
                && (!is_reached(small, tiles, (cx, cy)) || !is_reached(large, tiles, (cx, cy)))
        })
        .collect();
    if !lost.is_empty() {
        errors.push(format!(
            "{} arena floor cells are unreachable (e.g. {:?})",
            lost.len(),
            lost[0]
        ));
    }
}

/// The Lab's bumpers, their side lanes, the flip-lane promise over the whole
/// central floor, and the charging dock.
fn check_lab_arena(level: &Level, arena: TileRect, errors: &mut Vec<String>) {
    let spec = &level.boss;
    let tiles = &level.tiles;
    let floor = spec.floor;

    if spec.bumpers.len() != 2 {
        errors.push(format!(
            "the Lab arena has {} bumpers, needs 2",
            spec.bumpers.len()
        ));
    }
    for (i, bar) in spec.bumpers.iter().enumerate() {
        let bar_shape = (bar.w == 1 && bar.h == super::layout::BUMPER_LEN)
            || (bar.h == 1 && bar.w == super::layout::BUMPER_LEN);
        if !bar_shape {
            errors.push(format!("bumper {i} {bar:?} is not a short bar"));
        }
        let solid = bar
            .cells()
            .all(|(cx, cy)| matches!(tiles.get(cx, cy), TileKind::Wall(WallKind::Solid)));
        if !solid {
            errors.push(format!("bumper {i} {bar:?} is not solid"));
        }
        if !(bar.x >= arena.x
            && bar.y >= arena.y
            && bar.right() <= arena.right()
            && bar.bottom() <= arena.bottom())
        {
            errors.push(format!("bumper {i} {bar:?} is not inside the arena"));
        }
        if bar.intersects(&floor) {
            errors.push(format!("bumper {i} {bar:?} sits on the encounter floor"));
        }
        // The mower's framework probes a 32 px escape lane beside an impact:
        // keep the two cells along at least one long face open.
        let lane = 2;
        let has_lane = [-1, 1].iter().any(|side| {
            (0..lane).all(|k| {
                (0..bar.w.max(bar.h)).all(|step| {
                    let (cx, cy) = if bar.w == 1 {
                        (
                            if *side < 0 { bar.x - 1 - k } else { bar.right() + k },
                            bar.y + step,
                        )
                    } else {
                        (
                            bar.x + step,
                            if *side < 0 { bar.y - 1 - k } else { bar.bottom() + k },
                        )
                    };
                    tiles.get(cx, cy).is_open()
                })
            })
        });
        if !has_lane {
            errors.push(format!("bumper {i} {bar:?} has no open 32 px side lane"));
        }
    }

    // The promise the mower was measured against: from every cell of the
    // exported central floor a solid is within reach in at least two cardinal
    // directions, so the charge always has a wall to flip onto.
    let short: Vec<(i32, i32, usize)> = floor
        .cells()
        .filter_map(|(cx, cy)| {
            let from = TileMap::tile_rect(cx, cy).center();
            let near = CARDINAL_RAYS
                .iter()
                .filter(|dir| solid_along(tiles, from, **dir).is_some())
                .count();
            (near < 2).then_some((cx, cy, near))
        })
        .collect();
    if !short.is_empty() {
        errors.push(format!(
            "{} of {} Lab floor cells have a solid in fewer than two directions (e.g. {:?})",
            short.len(),
            floor.area(),
            short[0]
        ));
    }

    match (spec.dock, spec.dock_cell) {
        (Some(pos), Some(cell)) => {
            if !arena.contains(cell.0, cell.1) {
                errors.push(format!("the Lab dock cell {cell:?} is not in the arena"));
            }
            if !tiles.get(cell.0, cell.1).is_open() {
                errors.push(format!("the Lab dock cell {cell:?} is not floor"));
            }
            if floor.contains(cell.0, cell.1) {
                errors.push(format!("the Lab dock cell {cell:?} is on the encounter floor"));
            }
            let cell_centre = TileMap::tile_rect(cell.0, cell.1).center();
            if cell_centre.dist(pos) > 0.5 {
                errors.push(format!(
                    "the Lab dock {pos:?} is not at its cell {cell:?}'s centre"
                ));
            }
            // The 32 x 30 px pad has to sit on arena floor, not across the wall.
            let half = crate::game::art_boss_mower::DOCK_HALF_SIZE;
            let pad = crate::game::level::Rectf::new(
                pos.x - half.x,
                pos.y - half.y,
                2.0 * half.x,
                2.0 * half.y,
            );
            let room = arena.to_world();
            let inside = pad.x >= room.x
                && pad.y >= room.y
                && pad.right() <= room.right()
                && pad.bottom() <= room.bottom();
            if !inside {
                errors.push(format!("the Lab dock pad at {pos:?} overlaps the arena wall"));
            }
            if !tiles.rect_clear(pad, SizeClass::Large) {
                errors.push(format!("the Lab dock pad at {pos:?} is not clear floor"));
            }
            // Away from the mouth: the dock is a landmark, not a second door.
            if let Some(gap) = spec
                .mouths
                .iter()
                .map(|&(mx, my)| (mx - cell.0).abs().max((my - cell.1).abs()))
                .min()
            {
                if gap < 6 {
                    errors.push(format!(
                        "the Lab dock cell {cell:?} is {gap} cells from the arena mouth"
                    ));
                }
            }
        }
        _ => errors.push("the Lab arena has no charging dock".to_string()),
    }
}

/// The Sewer's escape rat-hole: on the wall ring, a real hole cell with an
/// object, and open reachable floor on both sides — one mouth in the arena,
/// one outside it.
fn check_escape_hole(
    level: &Level,
    arena: TileRect,
    cell: (i32, i32),
    small: &[bool],
    errors: &mut Vec<String>,
) {
    let tiles = &level.tiles;
    let ring = arena.inflate(1);
    let on_ring = ring.contains(cell.0, cell.1)
        && (cell.0 == ring.x
            || cell.1 == ring.y
            || cell.0 == ring.right() - 1
            || cell.1 == ring.bottom() - 1);
    if !on_ring {
        errors.push(format!("the Sewer escape hole {cell:?} is not on the wall ring"));
    }
    let axis = match tiles.get(cell.0, cell.1) {
        TileKind::Wall(WallKind::RatHole { axis }) => Some(axis),
        other => {
            errors.push(format!(
                "the Sewer escape hole {cell:?} is not a rat-hole cell ({other:?})"
            ));
            None
        }
    };
    if !level.objects.rat_holes.iter().any(|h| h.cell == cell) {
        errors.push(format!(
            "the Sewer escape hole {cell:?} has no rat-hole object"
        ));
    }
    let Some(axis) = axis else { return };
    // `WallAxis` names the wall's axis; the passage runs across it.
    let (dx, dy) = match axis {
        WallAxis::Vertical => (1, 0),
        WallAxis::Horizontal => (0, 1),
    };
    let (mut inner, mut outer) = (false, false);
    for sign in [1, -1] {
        let mouth = (cell.0 + sign * dx, cell.1 + sign * dy);
        if !tiles.get(mouth.0, mouth.1).is_open() {
            errors.push(format!(
                "the Sewer escape hole mouth {mouth:?} is not open floor"
            ));
            continue;
        }
        if !is_reached(small, tiles, mouth) {
            errors.push(format!(
                "the Sewer escape hole mouth {mouth:?} is unreachable"
            ));
        }
        if arena.contains(mouth.0, mouth.1) {
            inner = true;
        } else {
            outer = true;
        }
    }
    if !inner {
        errors.push("the Sewer escape hole does not open into the arena".to_string());
    }
    if !outer {
        errors.push("the Sewer escape hole does not open out of the arena".to_string());
    }
}

/// The placed objects must agree with the grid, and no hazard may sit on the
/// player's spawn.
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
        if level.boss_room().is_some_and(|room| {
            super::placement::in_boss_fight_floor(room.rect, cell)
        }) {
            errors.push(format!(
                "trap at {pos:?} is on the boss arena's fighting floor"
            ));
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
    use crate::game::art_traps::{MousetrapView, RatHoleView, StickyTrapView};
    use crate::game::art_walls::WallAxis;
    use crate::game::level::{MazeSection, RatHole, Theme};

    /// A cheap first-attempt map: stage A, its props and the rat-hole objects,
    /// without mazes, traps or navigation. `validate_arena` needs none of those,
    /// so a 2000-seed-per-theme sweep can run on this path in seconds.
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

    /// How many cardinal directions from a floor cell hold a solid within five
    /// cells (80 px, conservative against the 90 px contract)? `skip` cells are
    /// treated as open floor, which is how the Lab's "before the bumpers"
    /// number is measured.
    fn near_directions(tiles: &TileMap, skip: &[TileRect], cx: i32, cy: i32) -> usize {
        let mut count = 0;
        for (dx, dy) in CARDINAL_RAYS {
            for k in 1..=5 {
                let (nx, ny) = (cx + dx * k, cy + dy * k);
                if !tiles.in_bounds(nx, ny) {
                    break;
                }
                if skip.iter().any(|bar| bar.contains(nx, ny)) {
                    continue;
                }
                if !tiles.get(nx, ny).is_open() {
                    count += 1;
                    break;
                }
            }
        }
        count
    }

    #[test]
    fn every_arena_fits_its_fight() {
        // The default-size slice of the 2000-seed sweep (the gated test below
        // is the full acceptance run).
        for theme in Theme::all() {
            for seed in 0..250u64 {
                let level = layout_only_level(theme, seed);
                if let Err(errors) = validate_arena(&level) {
                    panic!("{theme:?} seed {seed}: the arena does not fit its fight: {errors:#?}");
                }
            }
        }
    }

    #[test]
    fn a_lab_floor_without_bumpers_has_no_flip_lane() {
        // The bug the arena furniture fixes: an unfurnished Lab arena is 416 to
        // 512 px wide, so from its centre no wall is inside the art's 135 px
        // charge reach (the boss authors measured a flip lane in 0 of 96
        // setups). With the two bumpers, every cell of the exported central
        // floor has a solid in at least two directions.
        for seed in 0..40u64 {
            let level = layout_only_level(Theme::Lab, seed);
            let floor = level.boss_clear_floor().expect("the Lab has a floor");
            let bumpers = level.boss_bumpers();
            assert_eq!(bumpers.len(), 2, "seed {seed}: the Lab needs two bumpers");
            let mut after = 0;
            let mut before = 0;
            let cells = floor.area() as usize;
            for (cx, cy) in floor.cells() {
                if near_directions(&level.tiles, &[], cx, cy) >= 2 {
                    after += 1;
                }
                if near_directions(&level.tiles, bumpers, cx, cy) >= 2 {
                    before += 1;
                }
            }
            assert_eq!(
                after, cells,
                "seed {seed}: {after}/{cells} Lab floor cells have a flip target"
            );
            assert!(
                before < cells,
                "seed {seed}: the floor was already furnished before the bumpers"
            );
        }
    }

    #[test]
    fn arena_layout_sweep() {
        // ```text
        // RATRUN_ARENA_SWEEP=2000 cargo test arena_layout_sweep -- --nocapture
        // ```
        // Gated like `level_statistics`: the owner's acceptance sweep, with the
        // measured numbers printed.
        let Ok(per_theme) = std::env::var("RATRUN_ARENA_SWEEP") else {
            return;
        };
        let per_theme: u64 = per_theme.parse().unwrap_or(2000).max(1);
        let mut failures = Vec::new();
        let mut sizes: Vec<Vec<i32>> = vec![Vec::new(); 3];
        let mut lab_after = 0u64;
        let mut lab_before = 0u64;
        let mut lab_cells = 0u64;
        let mut holes = 0u64;
        let mut docks = 0u64;
        let mut arena_props = 0u64;
        let mut floors: Vec<Vec<(i32, i32)>> = vec![Vec::new(); 3];
        for theme in Theme::all() {
            let t = Theme::all().iter().position(|x| *x == theme).unwrap();
            for seed in 0..per_theme {
                let level = layout_only_level(theme, seed);
                if let Err(errors) = validate_arena(&level) {
                    failures.push(format!("{theme:?} seed {seed}: {errors:?}"));
                }
                if let Some(room) = level.boss_room() {
                    sizes[t].push(room.rect.area());
                }
                let floor = level.boss_clear_floor().expect("every arena has a floor");
                floors[t].push((floor.w, floor.h));
                if level.boss_escape_hole().is_some() {
                    holes += 1;
                }
                if level.boss_dock().is_some() {
                    docks += 1;
                }
                arena_props += level
                    .props
                    .iter()
                    .filter(|p| {
                        level
                            .boss_room()
                            .is_some_and(|r| prop_cells(p.rect).intersects(&r.rect))
                    })
                    .count() as u64;
                if theme == Theme::Lab {
                    let bumpers = level.boss_bumpers();
                    for (cx, cy) in floor.cells() {
                        lab_cells += 1;
                        if near_directions(&level.tiles, &[], cx, cy) >= 2 {
                            lab_after += 1;
                        }
                        if near_directions(&level.tiles, bumpers, cx, cy) >= 2 {
                            lab_before += 1;
                        }
                    }
                }
            }
        }
        let maps = 3 * per_theme;
        println!("arena sweep: {per_theme} seeds x 3 themes = {maps} layout-only first attempts, {} validation failures", failures.len());
        for (t, theme) in Theme::all().iter().enumerate() {
            let mut v = sizes[t].clone();
            v.sort_unstable();
            let (lo, hi) = (v.first().copied().unwrap_or(0), v.last().copied().unwrap_or(0));
            let med = v.get(v.len() / 2).copied().unwrap_or(0);
            let mut f = floors[t].clone();
            f.sort_unstable();
            let med_floor = f.get(f.len() / 2).copied().unwrap_or((0, 0));
            println!(
                "  {theme:?}: arena cells min/median/max {lo}/{med}/{hi}; central floor {med_floor:?} (cells)"
            );
        }
        println!(
            "  Lab flip targets: {lab_after}/{lab_cells} floor cells with a solid in >= 2 directions after the bumpers, {lab_before}/{lab_cells} before"
        );
        println!(
            "  Sewer escape holes {holes}/{maps}; Lab docks {docks}/{maps}; arena props placed {arena_props}"
        );
        assert!(
            failures.is_empty(),
            "{} of {maps} arenas failed their theme's contract:\n{}",
            failures.len(),
            failures
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(lab_after, lab_cells, "the Lab floor lost its flip target");
        assert_eq!(holes, per_theme, "a Sewer arena lost its escape rat-hole");
        assert_eq!(docks, per_theme, "a Lab arena lost its charging dock");
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
    fn a_trap_on_the_boss_fighting_floor_fails_validation() {
        let mut level = Level::generate(Theme::Sewer, 3);
        let arena = level.boss_room().expect("boss arena").rect;
        let cell = (arena.x + arena.w / 2, arena.y + arena.h / 2);
        level.objects.mousetraps.push(MousetrapView::new(
            TileMap::tile_rect(cell.0, cell.1).center(),
            Vec2::new(1.0, 0.0),
        ));
        let errors = validate(&level).expect_err("a trap on the fight floor must fail");
        assert!(
            errors.iter().any(|e| e.contains("boss arena")),
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
    fn a_door_cell_without_its_object_fails_validation() {
        let mut level = Level::generate(Theme::Lab, 6);
        // A door cell in the middle of the boss arena wall, with no object.
        let arena = level.boss_room().expect("boss arena").rect;
        let cell = (arena.x, arena.y);
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
