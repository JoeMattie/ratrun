//! Procedural level generation.
//!
//! Five stages, run in this order, each owning one concern (the file-ownership
//! table with signatures is in `CONTENT_PLAN.md` §2):
//!
//! 1. [`layout::build_layout`] — rooms, corridors, open halls, the boss arena
//!    and the start room; carves them into the tile grid and returns the named
//!    regions. *(generator worker A)*
//! 2. [`layout::furnish`] — the theme's props inside the rooms, painted into
//!    the grid as solid cells. *(worker A)*
//! 3. [`maze::carve_mazes`] — zero to two maze sections in the reserved slots,
//!    each with a reward at its end and a one-way door as its exit.
//!    *(worker B)*
//! 4. [`maze::place_rat_holes`] — rat-holes in structural walls. *(worker B)*
//! 5. [`maze::place_traps`] — sticky traps and mousetraps in corridors and
//!    rooms. *(worker B)*
//!
//! Then [`validate`] proves the map is playable; [`generate`] retries with a
//! salted seed when it is not. The retry is a **safety net, not a routine
//! path**: the first attempt validates on every map in the sweep
//! (`layout::tests::every_map_validates_on_its_first_attempt`, and thousands of
//! seeds per theme under `RATRUN_LEVEL_STATS`), so a retry means a generator
//! bug rather than bad luck.
//!
//! ## Determinism
//!
//! One [`StdRng`] seeded from `(theme, run seed, attempt)` drives every stage
//! in order, and nothing iterates a hash map, so a seed always yields the same
//! map. The generator never reads the clock, the player or the ECS world.
//!
//! ## Measuring it
//!
//! ```text
//! RATRUN_LEVEL_STATS=2500 cargo test level_statistics -- --nocapture
//! ```
//!
//! prints the designer-facing distributions (rooms, halls, corridors, boss
//! arena size and distance, mazes per map and their distance, open-floor
//! share) over that many seeds per theme, and fails if any first attempt does
//! not validate.

pub mod layout;
pub mod maze;
pub mod validate;

use crate::math::Vec2;

use super::art_traps::{MazeRewardView, MousetrapView, RatHoleView, StickyTrapView};
use super::art_walls::WallAxis;
use super::collision::{cardinal_vec, SizeClass, TileMap};
use super::level::{
    level_rng, level_seed, Level, LevelObjects, MazeSection, OneWayDoor, Palette, RatHole, Theme,
    MAP_COLS, MAP_ROWS,
};
use super::nav::NavGrid;

/// How many differently-salted maps to try before giving up on validation.
///
/// Retries are part of the deterministic contract: the same seed always
/// produces the same map *and* the same retry outcome.
pub const MAX_ATTEMPTS: u32 = 4;

/// Generate the level for `(theme, seed)` and prove it is playable.
///
/// A map that fails [`validate`] is thrown away and regenerated from a salted
/// seed (up to [`MAX_ATTEMPTS`]). In debug builds a map that never validates
/// panics with the validator's report — that is a generator bug, and every
/// test that generates a map will show it. In a release build the last
/// attempt is used so the player still gets a runnable map.
pub fn generate(theme: Theme, seed: u64) -> Level {
    let mut last: Option<(Level, Vec<String>)> = None;
    for attempt in 0..MAX_ATTEMPTS {
        let level = generate_once(theme, seed, attempt);
        match validate::validate(&level) {
            Ok(()) => return level,
            Err(errors) => last = Some((level, errors)),
        }
    }
    let (level, errors) = last.expect("MAX_ATTEMPTS is at least one");
    if cfg!(debug_assertions) {
        panic!("level generation failed validation for {theme:?} seed {seed}: {errors:#?}");
    }
    level
}

/// One generation attempt: the five stages, then the derived data (masks, the
/// legacy wall mirror, the navigation mask).
pub fn generate_once(theme: Theme, seed: u64, attempt: u32) -> Level {
    let mut rng = level_rng(theme, seed, attempt);
    let art_seed = level_seed(theme, seed, attempt) as u32;
    let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, art_seed);

    let design = layout::build_layout(&mut tiles, &mut rng, theme);
    let props = layout::furnish(&mut tiles, &design, &mut rng, theme);
    let mazes = maze::carve_mazes(&mut tiles, &design, &mut rng, theme);
    let holes = maze::place_rat_holes(&mut tiles, &design, &mut rng);
    let traps = maze::place_traps(&tiles, &design, &mazes, &mut rng);

    tiles.rebuild_masks();

    let objects = objects_from(&tiles, &mazes, &holes, &traps);
    let nav = NavGrid::from_tiles(&tiles, SizeClass::Large);
    Level {
        theme,
        seed,
        arena: tiles.world_size(),
        tiles,
        props,
        palette: Palette::for_theme(theme),
        nav,
        rooms: design.rooms.clone(),
        start: design.start,
        mazes: mazes.clone(),
        objects,
    }
}

/// Build the runtime [`LevelObjects`] from the generator's placements: the
/// art modules' own view structs, initialised to their idle states.
fn objects_from(
    tiles: &TileMap,
    mazes: &[MazeSection],
    holes: &[((i32, i32), WallAxis)],
    traps: &[maze::TrapPlacement],
) -> LevelObjects {
    let mut objects = LevelObjects::default();

    // Rat-holes: one view per hole cell, facing along the passage.
    for &(cell, axis) in holes {
        let facing = match axis {
            WallAxis::Horizontal => Vec2::new(0.0, 1.0),
            WallAxis::Vertical => Vec2::new(1.0, 0.0),
        };
        let center = TileMap::tile_rect(cell.0, cell.1).center();
        objects.rat_holes.push(RatHole {
            cell,
            view: RatHoleView::new(center, facing),
        });
    }

    // One-way doors and the reward that ends each maze.
    for section in mazes {
        let dir = cardinal_vec(section.exit_dir);
        objects
            .one_way_doors
            .push(OneWayDoor::new(section.exit, dir));
        let exit_center = TileMap::tile_rect(section.exit.0, section.exit.1).center();
        let facing = (exit_center - section.reward).normalized();
        let facing = if facing.len_sq() > 0.5 {
            facing
        } else {
            Vec2::new(1.0, 0.0)
        };
        objects
            .rewards
            .push(MazeRewardView::new(section.reward, facing));
    }

    // Traps (floor hazards: they never paint a cell).
    for t in traps {
        match t.kind {
            maze::TrapKind::Sticky => objects
                .sticky_traps
                .push(StickyTrapView::new(t.pos, t.facing)),
            maze::TrapKind::Snap => objects
                .mousetraps
                .push(MousetrapView::new(t.pos, t.facing)),
        }
    }

    // Doors are authored by `carve_mazes`, so their cells must exist.
    debug_assert!(
        objects
            .one_way_doors
            .iter()
            .all(|d| tiles.one_way_dir(d.cell.0, d.cell.1).is_some()),
        "one-way door placed on a cell that is not a OneWayDoor wall"
    );

    objects
}
