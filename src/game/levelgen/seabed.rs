//! Decorative aquatic plants on the Outfall seabed: kelp fronds along walls
//! and room edges, seagrass tufts on open sand, and corner anemones / sea fans.
//!
//! Plants are **purely decorative**: never a tile, never blocking, invisible to
//! gameplay, the bot, fog and the minimap. This stage runs **last** in
//! `levelgen::generate_stages` (after `puddles::place`) on `Theme::Outfall`
//! only, using its own salted RNG (`level_seed ^` [`SEABED_SALT`]) over the
//! finished grid and writing nothing but [`Level::plants`].
//!
//! ## Why neither the map pin nor any golden can move
//!
//! 1. **Outfall only.** Every other theme returns immediately without drawing a
//!    single number and keeps `Level::plants` empty.
//! 2. **Salted stream, run last.** `level_rng(Theme::Outfall, level.seed ^ SEABED_SALT, attempt)`
//!    is independent of every earlier stage, and no later stage exists.
//! 3. **Render-only data.** Nothing in the simulation, collision, navigation,
//!    fog, minimap or bot reads `Level::plants`.

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::WallKind;
use crate::game::collision::{cell_of, TileKind, TileMap, TILE_F};
use crate::game::level::{level_rng, Level, PlantKind, RoomKind, SeabedPlant, Theme};
use crate::math::Vec2;

/// Salt mixed into the level seed for seabed plant placement.
///
/// Distinct from `lights` (`0x4c49_4748_5453`) and `puddles` (`0x5055_4444_4c45_5357`).
pub const SEABED_SALT: u64 = 0x5345_4142_4544_504c; // "SEABEDPL"

/// Minimum distance from `Level::start`, in world px.
pub const START_CLEAR_PX: f32 = 48.0;

/// Minimum distance from a sticky trap or mousetrap pad centre, in world px.
pub const TRAP_CLEAR_PX: f32 = 30.0;

/// Minimum distance from an Outfall lure trap centre, in world px.
pub const LURE_CLEAR_PX: f32 = 28.0;

/// Margin outside every tidal drop pit rectangle, in world px.
pub const PIT_CLEAR_PX: f32 = 16.0;

/// Minimum distance from a maze reward pedestal, in world px.
pub const REWARD_CLEAR_PX: f32 = 24.0;

/// Minimum distance from a mouse-hole shop face, in world px.
pub const SHOP_CLEAR_PX: f32 = 24.0;

/// Cluster/patch target ranges across one 200 × 160 Outfall map.
const KELP_CLUSTERS: (usize, usize) = (36, 52);
const SEAGRASS_PATCHES: (usize, usize) = (45, 65);
const ANEMONE_COUNT: (usize, usize) = (18, 32);

/// Minimum spacing between cluster/patch anchors so swaying foliage stays
/// distributed and never crowds one viewport (capping terminal byte churn).
const KELP_CLUSTER_SPACING: f32 = 72.0;
const SEAGRASS_PATCH_SPACING: f32 = 64.0;
const ANEMONE_SPACING: f32 = 64.0;

/// Place decorative aquatic plants on an Outfall level. No-op on every other
/// theme. Called last by `levelgen::generate_stages` (and after Outfall lure
/// repair so repaired lure banks stay clear).
pub(super) fn place(level: &mut Level, attempt: u32) {
    level.plants.clear();
    if level.theme != Theme::Outfall {
        return;
    }

    let mut rng = level_rng(level.theme, level.seed ^ SEABED_SALT, attempt);
    let blocked = blocked_cells(level);
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();

    // Classify every eligible open cell in deterministic (row, col) order.
    let mut wall_edges: Vec<(i32, i32)> = Vec::new();
    let mut corners: Vec<(i32, i32)> = Vec::new();
    let mut open_sand: Vec<(i32, i32)> = Vec::new();

    for cy in 1..(rows - 1) {
        for cx in 1..(cols - 1) {
            if !matches!(level.tiles.get(cx, cy), TileKind::Open) {
                continue;
            }
            if blocked[(cy * cols + cx) as usize] {
                continue;
            }
            if is_corridor_middle(level, cx, cy) {
                continue;
            }
            let (n, s, w, e) = wall_neighbors(&level.tiles, cx, cy);
            // Skip 1-cell pinches (walls on opposite sides) everywhere so no
            // plant sits in a narrow choke point.
            if (n && s) || (w && e) {
                continue;
            }
            let touches_wall = n || s || w || e;
            if touches_wall {
                wall_edges.push((cx, cy));
                if (n || s) && (w || e) {
                    corners.push((cx, cy));
                }
            } else if !in_corridor(level, cx, cy) && !in_maze(level, cx, cy) {
                open_sand.push((cx, cy));
            }
        }
    }

    let mut plants: Vec<SeabedPlant> = Vec::with_capacity(420);

    // 1. Kelp clusters along walls and room edges (3–5 fronds per cluster).
    shuffle(&mut wall_edges, &mut rng);
    let wanted_kelp = rng.random_range(KELP_CLUSTERS.0..=KELP_CLUSTERS.1);
    let mut kelp_anchors: Vec<Vec2> = Vec::with_capacity(wanted_kelp);
    let kelp_space_sq = KELP_CLUSTER_SPACING * KELP_CLUSTER_SPACING;

    for &(cx, cy) in &wall_edges {
        if kelp_anchors.len() >= wanted_kelp {
            break;
        }
        let anchor = wall_hug_pos(&level.tiles, cx, cy);
        if kelp_anchors
            .iter()
            .any(|&a| a.dist_sq(anchor) < kelp_space_sq)
        {
            continue;
        }
        if !is_point_clear(level, anchor, PlantKind::Kelp, &blocked) {
            continue;
        }

        let in_corr = in_corridor(level, cx, cy);
        let fronds = rng.random_range(3..=5usize);
        let mut placed_any = false;
        let (n, s, w, e) = wall_neighbors(&level.tiles, cx, cy);
        // Spread fronds along the wall's tangent axis so the cluster forms a
        // natural little kelp bed hugging the piling or room edge.
        let horizontal_wall = n || s;
        for f in 0..fronds {
            let offset_along = (f as f32 - (fronds as f32 - 1.0) * 0.5) * 5.2
                + rng.random_range(-1.2..=1.2);
            let offset_perp = rng.random_range(-1.5..=1.8);
            let pos = if horizontal_wall && !(w || e) {
                Vec2::new(anchor.x + offset_along, anchor.y + offset_perp)
            } else if (w || e) && !horizontal_wall {
                Vec2::new(anchor.x + offset_perp, anchor.y + offset_along)
            } else {
                Vec2::new(
                    anchor.x + rng.random_range(-4.0..=4.0),
                    anchor.y + rng.random_range(-3.0..=3.0),
                )
            };
            if !is_point_clear(level, pos, PlantKind::Kelp, &blocked) {
                continue;
            }
            if plants.iter().any(|p| p.pos.dist_sq(pos) < 9.0) {
                continue;
            }
            let max_size = if in_corr { 1 } else { 2 };
            plants.push(SeabedPlant {
                pos,
                kind: PlantKind::Kelp,
                phase: rng.random_range(0.0..std::f32::consts::TAU),
                size: rng.random_range(0..=max_size),
                seed: rng.random::<u32>(),
            });
            placed_any = true;
        }
        if placed_any {
            kelp_anchors.push(anchor);
        }
    }

    // 2. Seagrass patches on open sand (3–6 short tufts per patch, rooms/halls only).
    shuffle(&mut open_sand, &mut rng);
    let wanted_grass = rng.random_range(SEAGRASS_PATCHES.0..=SEAGRASS_PATCHES.1);
    let mut grass_anchors: Vec<Vec2> = Vec::with_capacity(wanted_grass);
    let grass_space_sq = SEAGRASS_PATCH_SPACING * SEAGRASS_PATCH_SPACING;

    for &(cx, cy) in &open_sand {
        if grass_anchors.len() >= wanted_grass {
            break;
        }
        let anchor = TileMap::tile_rect(cx, cy).center();
        if grass_anchors
            .iter()
            .any(|&a| a.dist_sq(anchor) < grass_space_sq)
        {
            continue;
        }
        if !is_point_clear(level, anchor, PlantKind::Seagrass, &blocked) {
            continue;
        }

        let tufts = rng.random_range(3..=6usize);
        let mut placed_any = false;
        for _ in 0..tufts {
            let pos = Vec2::new(
                anchor.x + rng.random_range(-18.0..=18.0),
                anchor.y + rng.random_range(-14.0..=14.0),
            );
            if !is_point_clear(level, pos, PlantKind::Seagrass, &blocked) {
                continue;
            }
            if plants.iter().any(|p| p.pos.dist_sq(pos) < 25.0) {
                continue;
            }
            plants.push(SeabedPlant {
                pos,
                kind: PlantKind::Seagrass,
                phase: rng.random_range(0.0..std::f32::consts::TAU),
                size: rng.random_range(0..=2),
                seed: rng.random::<u32>(),
            });
            placed_any = true;
        }
        if placed_any {
            grass_anchors.push(anchor);
        }
    }

    // 3. A few anemones / sea fans tucked into wall corners.
    shuffle(&mut corners, &mut rng);
    let wanted_anemones = rng.random_range(ANEMONE_COUNT.0..=ANEMONE_COUNT.1);
    let mut anemone_anchors: Vec<Vec2> = Vec::with_capacity(wanted_anemones);
    let anemone_space_sq = ANEMONE_SPACING * ANEMONE_SPACING;

    for &(cx, cy) in &corners {
        if anemone_anchors.len() >= wanted_anemones {
            break;
        }
        let pos = wall_hug_pos(&level.tiles, cx, cy);
        if anemone_anchors
            .iter()
            .any(|&a| a.dist_sq(pos) < anemone_space_sq)
        {
            continue;
        }
        if !is_point_clear(level, pos, PlantKind::Anemone, &blocked) {
            continue;
        }
        if plants.iter().any(|p| p.pos.dist_sq(pos) < 36.0) {
            continue;
        }
        plants.push(SeabedPlant {
            pos,
            kind: PlantKind::Anemone,
            phase: rng.random_range(0.0..std::f32::consts::TAU),
            size: rng.random_range(0..=2),
            seed: rng.random::<u32>(),
        });
        anemone_anchors.push(pos);
    }

    // Draw order: back-to-front by world Y, with X and seed breaking ties.
    plants.sort_by(|a, b| {
        a.pos
            .y
            .total_cmp(&b.pos.y)
            .then_with(|| a.pos.x.total_cmp(&b.pos.x))
            .then_with(|| a.seed.cmp(&b.seed))
    });
    level.plants = plants;
}

/// Deterministic Fisher-Yates shuffle driven solely by `rng`.
fn shuffle<T>(slice: &mut [T], rng: &mut StdRng) {
    let mut i = slice.len();
    while i > 1 {
        i -= 1;
        let j = rng.random_range(0..=i);
        slice.swap(i, j);
    }
}

#[inline]
fn wall_neighbors(tiles: &TileMap, cx: i32, cy: i32) -> (bool, bool, bool, bool) {
    (
        tiles.get(cx, cy - 1).is_wall(),
        tiles.get(cx, cy + 1).is_wall(),
        tiles.get(cx - 1, cy).is_wall(),
        tiles.get(cx + 1, cy).is_wall(),
    )
}

/// Anchor point inside open cell `(cx, cy)` tucked cleanly toward any adjacent
/// wall face (while staying strictly inside the open cell).
fn wall_hug_pos(tiles: &TileMap, cx: i32, cy: i32) -> Vec2 {
    let rect = TileMap::tile_rect(cx, cy);
    let (n, s, w, e) = wall_neighbors(tiles, cx, cy);
    let x = if w && !e {
        rect.x + 4.5
    } else if e && !w {
        rect.right() - 4.5
    } else {
        rect.x + TILE_F * 0.5
    };
    let y = if n && !s {
        rect.y + 5.5
    } else if s && !n {
        rect.bottom() - 3.5
    } else {
        rect.y + TILE_F * 0.5
    };
    Vec2::new(x, y)
}

fn in_corridor(level: &Level, cx: i32, cy: i32) -> bool {
    level
        .rooms
        .iter()
        .any(|r| r.kind == RoomKind::Corridor && r.rect.contains(cx, cy))
}

fn in_maze(level: &Level, cx: i32, cy: i32) -> bool {
    level
        .rooms
        .iter()
        .any(|r| r.kind == RoomKind::Maze && r.rect.contains(cx, cy))
}

/// True when `(cx, cy)` lies in the middle of a corridor (either an interior
/// corridor cell with no adjacent wall, or a 1-cell-wide pinch with walls on
/// opposite sides). Plants are only allowed on corridor edges.
pub(crate) fn is_corridor_middle(level: &Level, cx: i32, cy: i32) -> bool {
    if !in_corridor(level, cx, cy) {
        return false;
    }
    let (n, s, w, e) = wall_neighbors(&level.tiles, cx, cy);
    let touches_wall = n || s || w || e;
    if !touches_wall {
        return true;
    }
    (n && s) || (w && e)
}

/// Cells a plant may never sit on, indexed `cy * cols + cx`.
///
/// Clears rat-holes, one-way doors, doorways, shop holes and their thresholds,
/// trap pads, Outfall lure traps, tidal drops, maze rewards and the start.
fn blocked_cells(level: &Level) -> Vec<bool> {
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();
    let mut blocked = vec![false; (cols * rows).max(0) as usize];

    let mark = |blocked: &mut Vec<bool>, cx: i32, cy: i32| {
        if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
            blocked[(cy * cols + cx) as usize] = true;
        }
    };
    let ring = |blocked: &mut Vec<bool>, cx: i32, cy: i32| {
        for dy in -1..=1 {
            for dx in -1..=1 {
                mark(blocked, cx + dx, cy + dy);
            }
        }
    };

    // Structural openings on the tile grid: rat-holes, doorways and one-way doors.
    for cy in 0..rows {
        for cx in 0..cols {
            if matches!(
                level.tiles.get(cx, cy),
                TileKind::Wall(
                    WallKind::RatHole { .. }
                        | WallKind::OneWayDoor { .. }
                        | WallKind::Doorway { .. }
                )
            ) {
                ring(&mut blocked, cx, cy);
            }
        }
    }
    for hole in &level.objects.rat_holes {
        ring(&mut blocked, hole.cell.0, hole.cell.1);
    }
    for door in &level.objects.one_way_doors {
        ring(&mut blocked, door.cell.0, door.cell.1);
    }
    for hole in &level.objects.shop_holes {
        ring(&mut blocked, hole.cell.0, hole.cell.1);
        ring(&mut blocked, hole.mouth.0, hole.mouth.1);
    }

    let start_sq = START_CLEAR_PX * START_CLEAR_PX;
    let trap_sq = TRAP_CLEAR_PX * TRAP_CLEAR_PX;
    let lure_sq = LURE_CLEAR_PX * LURE_CLEAR_PX;
    let reward_sq = REWARD_CLEAR_PX * REWARD_CLEAR_PX;
    let shop_sq = SHOP_CLEAR_PX * SHOP_CLEAR_PX;

    for cy in 0..rows {
        for cx in 0..cols {
            let idx = (cy * cols + cx) as usize;
            if blocked[idx] {
                continue;
            }
            let centre = TileMap::tile_rect(cx, cy).center();
            if centre.dist_sq(level.start) < start_sq
                || level.touches_tidal_pit(centre, PIT_CLEAR_PX)
                || level
                    .objects
                    .sticky_traps
                    .iter()
                    .map(|t| t.pos)
                    .chain(level.objects.mousetraps.iter().map(|t| t.pos))
                    .chain(level.objects.crab_pots.iter().map(|t| t.pos))
                    .any(|p| p.dist_sq(centre) < trap_sq)
                || level
                    .objects
                    .lure_traps
                    .iter()
                    .any(|t| t.pos.dist_sq(centre) < lure_sq)
                || level
                    .objects
                    .rewards
                    .iter()
                    .any(|r| r.pos.dist_sq(centre) < reward_sq)
                || level
                    .objects
                    .shop_holes
                    .iter()
                    .any(|h| h.face().dist_sq(centre) < shop_sq)
            {
                blocked[idx] = true;
            }
        }
    }
    blocked
}

/// Exact per-plant point clearance guard so no cluster member can ever bleed
/// onto a forbidden object, pit rim, wall, or corridor centre.
fn is_point_clear(level: &Level, pos: Vec2, kind: PlantKind, blocked: &[bool]) -> bool {
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();
    let (cx, cy) = cell_of(pos);
    if cx <= 0 || cy <= 0 || cx >= cols - 1 || cy >= rows - 1 {
        return false;
    }
    if !matches!(level.tiles.get(cx, cy), TileKind::Open) {
        return false;
    }
    if blocked[(cy * cols + cx) as usize] {
        return false;
    }
    if is_corridor_middle(level, cx, cy) {
        return false;
    }
    let (n, s, w, e) = wall_neighbors(&level.tiles, cx, cy);
    if (n && s) || (w && e) {
        return false;
    }
    if kind == PlantKind::Seagrass && (in_corridor(level, cx, cy) || in_maze(level, cx, cy)) {
        return false;
    }
    // Keep every plant tucked away from the centreline of a corridor.
    if in_corridor(level, cx, cy) {
        let cell = TileMap::tile_rect(cx, cy);
        let near_wall_edge = (n && pos.y <= cell.y + 7.5)
            || (s && pos.y >= cell.bottom() - 6.0)
            || (w && pos.x <= cell.x + 6.5)
            || (e && pos.x >= cell.right() - 6.5);
        if !near_wall_edge {
            return false;
        }
    }
    if pos.dist_sq(level.start) < START_CLEAR_PX * START_CLEAR_PX {
        return false;
    }
    if level.touches_tidal_pit(pos, PIT_CLEAR_PX) {
        return false;
    }
    let trap_sq = TRAP_CLEAR_PX * TRAP_CLEAR_PX;
    if level
        .objects
        .sticky_traps
        .iter()
        .map(|t| t.pos)
        .chain(level.objects.mousetraps.iter().map(|t| t.pos))
        .chain(level.objects.crab_pots.iter().map(|t| t.pos))
        .any(|p| p.dist_sq(pos) < trap_sq)
    {
        return false;
    }
    if level
        .objects
        .lure_traps
        .iter()
        .any(|t| t.pos.dist_sq(pos) < LURE_CLEAR_PX * LURE_CLEAR_PX)
    {
        return false;
    }
    if level
        .objects
        .rewards
        .iter()
        .any(|r| r.pos.dist_sq(pos) < REWARD_CLEAR_PX * REWARD_CLEAR_PX)
    {
        return false;
    }
    if level
        .objects
        .shop_holes
        .iter()
        .any(|h| h.face().dist_sq(pos) < SHOP_CLEAR_PX * SHOP_CLEAR_PX)
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outfall_has_all_plant_kinds_and_other_themes_have_none() {
        for seed in 0..8 {
            let outfall = Level::generate(Theme::Outfall, seed);
            assert!(
                !outfall.plants.is_empty(),
                "Outfall seed {seed} should place aquatic plants"
            );
            let kelp = outfall
                .plants
                .iter()
                .filter(|p| p.kind == PlantKind::Kelp)
                .count();
            let grass = outfall
                .plants
                .iter()
                .filter(|p| p.kind == PlantKind::Seagrass)
                .count();
            let anemones = outfall
                .plants
                .iter()
                .filter(|p| p.kind == PlantKind::Anemone)
                .count();
            assert!(kelp >= 30, "seed {seed}: too few kelp fronds ({kelp})");
            assert!(grass >= 30, "seed {seed}: too few seagrass tufts ({grass})");
            assert!(anemones >= 5, "seed {seed}: too few anemones ({anemones})");
        }

        for theme in Theme::playable() {
            if theme == Theme::Outfall {
                continue;
            }
            for seed in 0..4 {
                let level = Level::generate(theme, seed);
                assert!(
                    level.plants.is_empty(),
                    "{theme:?} seed {seed} must not have seabed plants, got {}",
                    level.plants.len()
                );
            }
        }
    }

    #[test]
    fn placement_rules_keep_plants_off_objects_and_corridor_centres() {
        for seed in 0..10 {
            let level = Level::generate(Theme::Outfall, seed);
            let blocked = blocked_cells(&level);
            let cols = level.tiles.cols();
            for plant in &level.plants {
                let (cx, cy) = cell_of(plant.pos);
                assert!(
                    matches!(level.tiles.get(cx, cy), TileKind::Open),
                    "seed {seed}: plant {:?} on wall cell ({cx}, {cy})",
                    plant.kind
                );
                assert!(
                    !blocked[(cy * cols + cx) as usize],
                    "seed {seed}: plant {:?} on blocked cell ({cx}, {cy})",
                    plant.kind
                );
                assert!(
                    !is_corridor_middle(&level, cx, cy),
                    "seed {seed}: plant {:?} in corridor middle ({cx}, {cy})",
                    plant.kind
                );
                assert!(
                    plant.pos.dist(level.start) >= START_CLEAR_PX,
                    "seed {seed}: plant too close to start"
                );
                assert!(
                    !level.touches_tidal_pit(plant.pos, PIT_CLEAR_PX),
                    "seed {seed}: plant too close to tidal drop"
                );
                for lure in &level.objects.lure_traps {
                    assert!(
                        plant.pos.dist(lure.pos) >= LURE_CLEAR_PX,
                        "seed {seed}: plant too close to lure trap"
                    );
                }
                for trap in level
                    .objects
                    .sticky_traps
                    .iter()
                    .map(|t| t.pos)
                    .chain(level.objects.mousetraps.iter().map(|t| t.pos))
                    .chain(level.objects.crab_pots.iter().map(|t| t.pos))
                {
                    assert!(
                        plant.pos.dist(trap) >= TRAP_CLEAR_PX,
                        "seed {seed}: plant too close to trap pad"
                    );
                }
                for hole in &level.objects.shop_holes {
                    assert!(
                        plant.pos.dist(hole.face()) >= SHOP_CLEAR_PX,
                        "seed {seed}: plant too close to shop threshold"
                    );
                }
                for door in &level.objects.one_way_doors {
                    assert!(
                        (door.cell.0 - cx).abs() > 1 || (door.cell.1 - cy).abs() > 1,
                        "seed {seed}: plant crowds one-way door"
                    );
                }
                for hole in &level.objects.rat_holes {
                    assert!(
                        (hole.cell.0 - cx).abs() > 1 || (hole.cell.1 - cy).abs() > 1,
                        "seed {seed}: plant crowds rat-hole"
                    );
                }
                if in_corridor(&level, cx, cy) {
                    assert_ne!(
                        plant.kind,
                        PlantKind::Seagrass,
                        "seed {seed}: seagrass placed in corridor"
                    );
                }
            }
        }
    }

    #[test]
    fn same_seed_gives_same_plants_and_leaves_tiles_objects_and_puddles_unchanged() {
        for seed in 0..4 {
            let a = Level::generate(Theme::Outfall, seed);
            let b = Level::generate(Theme::Outfall, seed);
            assert_eq!(a.plants, b.plants, "Outfall seed {seed} plants not repeatable");

            let mut rerun = Level::generate(Theme::Outfall, seed);
            let puddles_before = rerun.puddles.clone();
            let lures_before = rerun.objects.lure_traps.clone();
            let lights_before = rerun.objects.lights.clone();
            rerun.plants.clear();
            place(&mut rerun, 0);
            assert_eq!(rerun.puddles, puddles_before);
            assert_eq!(rerun.objects.lure_traps, lures_before);
            assert_eq!(rerun.objects.lights, lights_before);
        }

        let s0 = Level::generate(Theme::Outfall, 0);
        let s1 = Level::generate(Theme::Outfall, 1);
        assert_ne!(s0.plants, s1.plants, "different seeds should grow different plant layouts");
    }
}
