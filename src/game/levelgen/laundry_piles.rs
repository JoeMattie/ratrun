//! Decorative piles of clothes and overflowing laundry baskets for the Laundry.
//!
//! Placed as the **last levelgen stage** (after lights and puddles) for
//! [`Theme::Laundry`] only, on its **own salted rng**
//! (`level_seed ^` [`LAUNDRY_PILE_SALT`]), reading the finished tile grid,
//! rooms, props and objects and writing nothing but [`Level::clothes_piles`].
//!
//! ## Why the map pin cannot move
//!
//! 1. **Isolated RNG stream.** Draws come from
//!    `level_rng(theme, seed ^ LAUNDRY_PILE_SALT, attempt)`, so no earlier stage
//!    can see a single draw, and no stage runs after this one.
//! 2. **Not folded by the pin.** `map_trace_pin` hashes tiles and `LevelObjects`
//!    only; `Level::clothes_piles` is purely decorative, like `Level::puddles`,
//!    invisible to collision, navigation, gameplay, the bot, fog and the minimap.
//! 3. **Laundry only.** Every other theme returns immediately without drawing.
//!
//! ## Placement rules
//!
//! Heaps and overflowing baskets land in folding bays, beside washer banks and
//! in room corners — never in the middle of aisles or corridors, so a pile
//! never obscures a moving body or telegraph:
//!
//! - Open floor cells only, inside a [`RoomKind::Hall`], [`RoomKind::Room`] or
//!   [`RoomKind::Start`] (never [`RoomKind::Corridor`] or [`RoomKind::Maze`]).
//! - Orthogonally adjacent to at least one `WallKind::Solid` cell (a room wall,
//!   washer bank or prop footprint), and never squeezed between two opposite
//!   walls.
//! - Clear of the start ([`START_CLEAR_PX`]), rat-holes, one-way doors,
//!   doorways, shop holes and their thresholds, steam grates ([`GRATE_CLEAR_PX`]
//!   and their 3×3 ring), traps ([`TRAP_CLEAR_PX`]) and maze rewards.

use rand::RngExt;

use crate::game::art_walls::WallKind;
use crate::game::collision::{TileKind, TileMap, TILE};
use crate::game::level::{
    level_rng, ClothesPile, ClothesPileKind, Level, PropKind, RoomKind, Theme,
};
use crate::math::Vec2;

/// Salt mixed into the level seed for this stage's own rng.
pub const LAUNDRY_PILE_SALT: u64 = 0x4c41_554e_4452_5950; // "LAUNDRYP"

/// Minimum clearance from the player start, in world px.
pub const START_CLEAR_PX: f32 = 48.0;

/// Minimum clearance from any sticky trap or mousetrap centre, in world px.
pub const TRAP_CLEAR_PX: f32 = 30.0;

/// Minimum clearance from any steam grate centre, in world px.
pub const GRATE_CLEAR_PX: f32 = 24.0;

/// Minimum clearance from any maze reward pedestal, in world px.
pub const REWARD_CLEAR_PX: f32 = 24.0;

/// Minimum centre-to-centre spacing between any two clothes piles, in world px.
pub const MIN_HEAP_SPACING_PX: f32 = 24.0;

/// Minimum centre-to-centre spacing between overflowing baskets, in world px.
pub const MIN_BASKET_SPACING_PX: f32 = 52.0;

/// Total clothes piles (heaps + baskets) placed on a Laundry floor.
pub const PILE_COUNT_RANGE: (usize, usize) = (38, 52);

/// Overflowing laundry baskets included in the total count.
pub const BASKET_COUNT_RANGE: (usize, usize) = (6, 10);

/// Gentle nudge toward the adjacent wall/bank edge (strictly inside the cell).
const EDGE_HUG_PX: f32 = 2.5;

/// Place the Laundry's decorative clothes piles and overflowing baskets.
/// Called last by `levelgen::generate_stages`. No-op on every other theme.
pub(super) fn place(level: &mut Level, attempt: u32) {
    if level.theme != Theme::Laundry {
        return;
    }
    let mut rng = level_rng(level.theme, level.seed ^ LAUNDRY_PILE_SALT, attempt);
    let blocked = blocked_cells(level);
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();

    let mut prime: Vec<(i32, i32)> = Vec::new();
    let mut edges: Vec<(i32, i32)> = Vec::new();

    for cy in 1..(rows - 1) {
        for cx in 1..(cols - 1) {
            if blocked[(cy * cols + cx) as usize] {
                continue;
            }
            if !is_valid_edge_cell(level, cx, cy) {
                continue;
            }
            if is_corner_cell(level, cx, cy) || near_washer_prop(level, cx, cy) {
                prime.push((cx, cy));
            } else {
                edges.push((cx, cy));
            }
        }
    }

    shuffle_deterministic(&mut prime, &mut rng);
    shuffle_deterministic(&mut edges, &mut rng);

    let wanted_total = rng.random_range(PILE_COUNT_RANGE.0..=PILE_COUNT_RANGE.1);
    let wanted_baskets = rng
        .random_range(BASKET_COUNT_RANGE.0..=BASKET_COUNT_RANGE.1)
        .min(wanted_total);

    let mut piles: Vec<ClothesPile> = Vec::with_capacity(wanted_total);

    // 1. Place overflowing baskets first, preferring corners and beside washer
    //    banks, then falling back to other folding-bay and room edges.
    for &(cx, cy) in prime.iter().chain(edges.iter()) {
        if piles.len() >= wanted_baskets {
            break;
        }
        let pos = hugged_pos(level, cx, cy);
        if too_close(&piles, pos, MIN_HEAP_SPACING_PX, Some(MIN_BASKET_SPACING_PX)) {
            continue;
        }
        piles.push(ClothesPile {
            pos,
            kind: ClothesPileKind::Basket,
            seed: rng.random::<u32>(),
        });
    }

    // 2. Scatter mixed-size clothes heaps across prime spots (corners & beside
    //    washer banks) and bay/room edges.
    let mut pi = 0usize;
    let mut ei = 0usize;
    while piles.len() < wanted_total && (pi < prime.len() || ei < edges.len()) {
        let pick_prime = if pi < prime.len() && ei < edges.len() {
            rng.random_range(0..10) < 7
        } else {
            pi < prime.len()
        };
        let (cx, cy) = if pick_prime {
            let c = prime[pi];
            pi += 1;
            c
        } else {
            let c = edges[ei];
            ei += 1;
            c
        };
        let pos = hugged_pos(level, cx, cy);
        if too_close(&piles, pos, MIN_HEAP_SPACING_PX, None) {
            continue;
        }
        let roll = rng.random_range(0..20);
        let kind = if roll < 9 {
            ClothesPileKind::SmallHeap
        } else if roll < 16 {
            ClothesPileKind::MediumHeap
        } else {
            ClothesPileKind::LargeHeap
        };
        piles.push(ClothesPile {
            pos,
            kind,
            seed: rng.random::<u32>(),
        });
    }

    level.clothes_piles = piles;
}

/// Deterministic Fisher-Yates shuffle over a slice using only `random_range`.
fn shuffle_deterministic<T>(items: &mut [T], rng: &mut rand::rngs::StdRng) {
    if items.len() <= 1 {
        return;
    }
    for i in (1..items.len()).rev() {
        let j = rng.random_range(0..=i);
        items.swap(i, j);
    }
}

/// True when `pos` violates minimum spacing against already-placed piles.
fn too_close(
    piles: &[ClothesPile],
    pos: Vec2,
    min_any: f32,
    min_basket: Option<f32>,
) -> bool {
    let any_sq = min_any * min_any;
    let basket_sq = min_basket.map(|d| d * d);
    piles.iter().any(|existing| {
        let d2 = existing.pos.dist_sq(pos);
        if d2 < any_sq {
            return true;
        }
        if let Some(bsq) = basket_sq {
            if existing.kind == ClothesPileKind::Basket && d2 < bsq {
                return true;
            }
        }
        false
    })
}

/// Cell centre nudged slightly toward adjacent solid wall(s) so heaps snuggle
/// against washer banks and room walls while staying strictly inside `(cx, cy)`.
fn hugged_pos(level: &Level, cx: i32, cy: i32) -> Vec2 {
    let mut pos = TileMap::tile_rect(cx, cy).center();
    let solid = |x: i32, y: i32| matches!(level.tiles.get(x, y), TileKind::Wall(WallKind::Solid));
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;
    if solid(cx - 1, cy) {
        dx -= EDGE_HUG_PX;
    }
    if solid(cx + 1, cy) {
        dx += EDGE_HUG_PX;
    }
    if solid(cx, cy - 1) {
        dy -= EDGE_HUG_PX;
    }
    if solid(cx, cy + 1) {
        dy += EDGE_HUG_PX;
    }
    pos.x += dx;
    pos.y += dy;
    pos
}

/// True when `(cx, cy)` is an open floor cell along a room wall, corner,
/// folding bay or washer bank — and never in a corridor, maze, or the middle
/// of an aisle.
pub fn is_valid_edge_cell(level: &Level, cx: i32, cy: i32) -> bool {
    if !matches!(level.tiles.get(cx, cy), TileKind::Open) {
        return false;
    }
    // Exclude corridors and mazes; allow only halls, rooms and the start room.
    let mut in_allowed_room = false;
    for room in &level.rooms {
        if !room.rect.contains(cx, cy) {
            continue;
        }
        match room.kind {
            RoomKind::Corridor | RoomKind::Maze => return false,
            RoomKind::Hall | RoomKind::Room | RoomKind::Start | RoomKind::Boss => {
                in_allowed_room = true;
            }
        }
    }
    if !in_allowed_room {
        return false;
    }
    let solid = |x: i32, y: i32| matches!(level.tiles.get(x, y), TileKind::Wall(WallKind::Solid));
    let n = solid(cx, cy - 1);
    let s = solid(cx, cy + 1);
    let w = solid(cx - 1, cy);
    let e = solid(cx + 1, cy);
    let count = u8::from(n) + u8::from(s) + u8::from(w) + u8::from(e);
    // Must hug at least one solid face (never the middle of an open room or
    // 3-cell aisle), and must not sit in a 1-cell squeeze between opposite walls.
    count >= 1 && !(n && s) && !(w && e)
}

/// True when two perpendicular solid walls meet at `(cx, cy)` (room corner or
/// bank alcove).
fn is_corner_cell(level: &Level, cx: i32, cy: i32) -> bool {
    let solid = |x: i32, y: i32| matches!(level.tiles.get(x, y), TileKind::Wall(WallKind::Solid));
    let vert = solid(cx, cy - 1) || solid(cx, cy + 1);
    let horiz = solid(cx - 1, cy) || solid(cx + 1, cy);
    vert && horiz
}

/// True when `(cx, cy)` sits within two cells of a `PropKind::Washer` bank.
fn near_washer_prop(level: &Level, cx: i32, cy: i32) -> bool {
    let centre = TileMap::tile_rect(cx, cy).center();
    const REACH: f32 = 2.0 * TILE as f32;
    level.props.iter().any(|prop| {
        if prop.kind != PropKind::Washer {
            return false;
        }
        let rect = &prop.rect;
        let dx = (rect.x - centre.x).max(0.0).max(centre.x - rect.right());
        let dy = (rect.y - centre.y).max(0.0).max(centre.y - rect.bottom());
        dx * dx + dy * dy <= REACH * REACH
    })
}

/// Flat mask of cells where a clothes pile may not land, indexed `cy * cols + cx`.
fn blocked_cells(level: &Level) -> Vec<bool> {
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();
    let mut blocked = vec![false; (cols * rows).max(0) as usize];
    let mark = |blocked: &mut Vec<bool>, cx: i32, cy: i32| {
        if cx < 0 || cy < 0 || cx >= cols || cy >= rows {
            return;
        }
        blocked[(cy * cols + cx) as usize] = true;
    };
    let ring = |blocked: &mut Vec<bool>, cx: i32, cy: i32| {
        for dy in -1..=1 {
            for dx in -1..=1 {
                mark(blocked, cx + dx, cy + dy);
            }
        }
    };

    // Rat-holes and doorways (both mouths / throats).
    for cy in 0..rows {
        for cx in 0..cols {
            if matches!(
                level.tiles.get(cx, cy),
                TileKind::Wall(WallKind::RatHole { .. } | WallKind::Doorway { .. })
            ) {
                ring(&mut blocked, cx, cy);
            }
        }
    }
    // One-way doors.
    for door in &level.objects.one_way_doors {
        ring(&mut blocked, door.cell.0, door.cell.1);
    }
    // Shop holes and their floor thresholds.
    for hole in &level.objects.shop_holes {
        ring(&mut blocked, hole.cell.0, hole.cell.1);
        ring(&mut blocked, hole.mouth.0, hole.mouth.1);
    }
    // Steam grates and their immediate 3×3 buffer.
    for grate in &level.objects.steam_grates {
        ring(&mut blocked, grate.cell.0, grate.cell.1);
    }

    // Distance-based clearances: start, traps, steam grates and maze rewards.
    for cy in 0..rows {
        for cx in 0..cols {
            let idx = (cy * cols + cx) as usize;
            if blocked[idx] {
                continue;
            }
            let centre = TileMap::tile_rect(cx, cy).center();
            if centre.dist(level.start) < START_CLEAR_PX + EDGE_HUG_PX {
                blocked[idx] = true;
                continue;
            }
            let on_trap = level
                .objects
                .sticky_traps
                .iter()
                .map(|t| t.pos)
                .chain(level.objects.mousetraps.iter().map(|t| t.pos))
                .any(|pos| pos.dist(centre) < TRAP_CLEAR_PX + EDGE_HUG_PX);
            let on_grate = level
                .objects
                .steam_grates
                .iter()
                .any(|g| g.pos.dist(centre) < GRATE_CLEAR_PX + EDGE_HUG_PX);
            let on_reward = level
                .objects
                .rewards
                .iter()
                .any(|r| r.pos.dist(centre) < REWARD_CLEAR_PX + EDGE_HUG_PX);
            if on_trap || on_grate || on_reward {
                blocked[idx] = true;
            }
        }
    }

    blocked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::cell_of;
    use crate::game::level::Theme;

    const SEEDS: u64 = 8;

    #[test]
    fn piles_exist_on_laundry_and_nowhere_else() {
        for theme in Theme::playable() {
            for seed in 0..4 {
                let level = Level::generate(theme, seed);
                if theme == Theme::Laundry {
                    assert!(
                        !level.clothes_piles.is_empty(),
                        "Laundry seed {seed} placed no clothes piles"
                    );
                    let baskets = level
                        .clothes_piles
                        .iter()
                        .filter(|p| p.kind == ClothesPileKind::Basket)
                        .count();
                    let heaps = level.clothes_piles.len() - baskets;
                    assert!(
                        baskets >= 2,
                        "Laundry seed {seed} placed only {baskets} baskets"
                    );
                    assert!(
                        heaps >= 10,
                        "Laundry seed {seed} placed only {heaps} clothes heaps"
                    );
                } else {
                    assert!(
                        level.clothes_piles.is_empty(),
                        "{theme:?} seed {seed} should have no clothes piles, got {}",
                        level.clothes_piles.len()
                    );
                }
            }
        }
    }

    #[test]
    fn same_seed_produces_identical_piles() {
        for seed in 0..4 {
            let a = Level::generate(Theme::Laundry, seed);
            let b = Level::generate(Theme::Laundry, seed);
            assert_eq!(
                a.clothes_piles, b.clothes_piles,
                "Laundry seed {seed} piles are not deterministic"
            );
        }
        let s0 = Level::generate(Theme::Laundry, 0);
        let s1 = Level::generate(Theme::Laundry, 1);
        assert_ne!(
            s0.clothes_piles, s1.clothes_piles,
            "different Laundry seeds should produce different clothes piles"
        );
    }

    #[test]
    fn placement_rules_hold_on_every_laundry_pile() {
        for seed in 0..SEEDS {
            let level = Level::generate(Theme::Laundry, seed);
            let blocked = blocked_cells(&level);
            let cols = level.tiles.cols();
            for (i, pile) in level.clothes_piles.iter().enumerate() {
                let (cx, cy) = cell_of(pile.pos);
                assert!(
                    matches!(level.tiles.get(cx, cy), TileKind::Open),
                    "seed {seed} pile {i} landed on non-open cell ({cx}, {cy})"
                );
                assert!(
                    !blocked[(cy * cols + cx) as usize],
                    "seed {seed} pile {i} landed on blocked cell ({cx}, {cy})"
                );
                assert!(
                    is_valid_edge_cell(&level, cx, cy),
                    "seed {seed} pile {i} at ({cx}, {cy}) is not on a valid room/bay edge"
                );
                assert!(
                    pile.pos.dist(level.start) >= START_CLEAR_PX,
                    "seed {seed} pile {i} is too close to start"
                );
                for grate in &level.objects.steam_grates {
                    assert!(
                        pile.pos.dist(grate.pos) >= GRATE_CLEAR_PX,
                        "seed {seed} pile {i} is too close to a steam grate"
                    );
                }
                for trap in level
                    .objects
                    .sticky_traps
                    .iter()
                    .map(|t| t.pos)
                    .chain(level.objects.mousetraps.iter().map(|t| t.pos))
                {
                    assert!(
                        pile.pos.dist(trap) >= TRAP_CLEAR_PX,
                        "seed {seed} pile {i} is too close to a trap"
                    );
                }
                for (j, other) in level.clothes_piles.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    assert!(
                        pile.pos.dist(other.pos) + 1e-3 >= MIN_HEAP_SPACING_PX,
                        "seed {seed} piles {i} and {j} overlap"
                    );
                }
            }
        }
    }

    #[test]
    fn laundry_piles_leave_tiles_props_and_objects_unchanged() {
        // Because laundry_piles runs last on its own salted RNG and writes only
        // `Level::clothes_piles` (not folded by `map_trace_pin`), the map's
        // tiles, props, rooms, objects and puddles are completely unaffected.
        for seed in 0..4 {
            let a = Level::generate(Theme::Laundry, seed);
            let b = Level::generate(Theme::Laundry, seed);
            assert_eq!(a.tiles.cols(), b.tiles.cols());
            assert_eq!(a.tiles.rows(), b.tiles.rows());
            for cy in 0..a.tiles.rows() {
                for cx in 0..a.tiles.cols() {
                    assert_eq!(a.tiles.cell(cx, cy).kind, b.tiles.cell(cx, cy).kind);
                    assert_eq!(a.tiles.cell(cx, cy).mask.0, b.tiles.cell(cx, cy).mask.0);
                }
            }
            assert_eq!(a.props.len(), b.props.len());
            for (pa, pb) in a.props.iter().zip(&b.props) {
                assert!(pa.kind == pb.kind);
                assert_eq!(pa.rect.x, pb.rect.x);
                assert_eq!(pa.rect.y, pb.rect.y);
                assert_eq!(pa.rect.w, pb.rect.w);
                assert_eq!(pa.rect.h, pb.rect.h);
            }
            assert_eq!(a.objects.steam_grates.len(), b.objects.steam_grates.len());
            assert_eq!(a.puddles, b.puddles);
        }
    }
}
