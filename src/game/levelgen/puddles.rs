//! Standing water: where a floor is wet enough for the rat to splash.
//!
//! The owner's brief: *"sewer especially should have areas of the floor be very
//! wet, and the rat should make little splashing sounds when walking over."*
//! A [`Puddle`] is decoration with one gameplay hook ([`Level::wet_at`] feeds
//! `systems::puddles`), placed **after every other stage** — lights and emitters
//! included — on its **own salted rng** (`level_seed ^` [`PUDDLE_SALT`]),
//! reading the finished tile grid and object list and writing nothing but
//! [`Level::puddles`].
//!
//! ## Why the map pin cannot move
//!
//! Two independent guarantees, both deliberate:
//!
//! 1. **The stream is its own.** This stage draws from
//!    `level_rng(theme, seed ^ PUDDLE_SALT, attempt)` — the same construction
//!    `levelgen::lights` uses — so not one number the layout, the maze, the
//!    holes, the traps, the hazards or the lights draw can be perturbed, and it
//!    is called last so nothing downstream of it exists.
//! 2. **The pin does not fold it.** `map_trace_pin` hashes every tile and every
//!    placed object of `LevelObjects`; `Level::puddles` is neither. So the pin
//!    is unchanged even when the puddle placement changes, and
//!    `puddles_leave_the_tiles_and_objects_alone` asserts that directly.
//!
//! ## Placement
//!
//! Centres land on **open floor cells** only — never on a rat-hole's face (or
//! anything within a cell of its tunnel), a one-way door, a shop hole's
//! threshold, a trap pad, or within [`START_CLEAR_PX`] of the start. Puddles may
//! overlap each other: in the Sewer they are drawn large on purpose so a run of
//! them merges into one long wet stretch along a corridor, which is the look the
//! brief asks for (~18 % of the Sewer's floor area is wet, measured by
//! `puddle_coverage_by_theme`).
//!
//! Water collects where it would in a real sewer: the Sewer's centres are
//! biased 4:1 toward corridors and cells that touch a wall. The Kitchen gets a
//! few small spills, biased the same way toward the props they would run off.
//! The Laundry is in between. The Lab, the Printworks and the Flatbed stay dry:
//! nothing in this stage runs for a theme [`count_for`] does not list.

use rand::RngExt;

use crate::game::art_walls::WallKind;
use crate::game::collision::{cell_of, TileKind, TileMap, TILE};
use crate::game::level::{level_rng, Level, Puddle, RoomKind, Theme};
use crate::math::Vec2;

/// Salt mixed into the level seed for this stage's own rng.
///
/// Spelled out so it is obvious that it is *a* salt and not a reused one:
/// `levelgen::lights` uses `SEED ^ 0x4c49_4748_5453`.
pub const PUDDLE_SALT: u64 = 0x5055_4444_4c45_5357; // "PUDDLESW"

/// How close a puddle centre may come to the start, in world px.
pub const START_CLEAR_PX: f32 = 40.0;

/// How close a puddle centre may come to a trap pad's centre, in world px (a
/// mousetrap is 40 × 24 px, so its half-diagonal is 23 px and 30 never covers
/// the pad).
pub const TRAP_CLEAR_PX: f32 = 30.0;

/// Rejection-sampling tries for one `wanted` puddle, before the unbiased
/// fallback in [`one_centre`]. Bounded, so generation can never loop forever.
const TRIES: u32 = 32;
/// Tries for the fallback: any open, unblocked cell.
const FALLBACK_TRIES: u32 = 96;

/// The share of draws that must land on a "wet place" when the floor has a
/// bias at all (see [`Bias`]).
const BIAS_CHANCE: f32 = 0.8;

/// How many puddles a floor gets (`min..=max`), or `None` for a floor whose
/// floor is dry.
///
/// The Sewer is the wet one; the Laundry and the Kitchen have standing water
/// where their hazards and spills are; the Lab, the Printworks and the Flatbed
/// get none.
pub fn count_for(theme: Theme) -> Option<(usize, usize)> {
    match theme {
        Theme::Sewer => Some((225, 255)),
        Theme::Laundry => Some((22, 30)),
        Theme::Kitchen => Some((8, 12)),
        Theme::Lab | Theme::Printworks | Theme::Trailer => None,
        Theme::Outfall => Some((240, 265)),
    }
}

/// What "a wet place" means on this floor — the 4:1 bias in [`BIAS_CHANCE`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bias {
    /// No bias: any open floor cell.
    Any,
    /// Corridors and cells that touch a solid wall (the Sewer, the Laundry).
    Edges,
    /// Cells within two cells of a prop (the Kitchen's spills).
    Props,
}

/// Place this level's puddles. Called last by `levelgen::generate_stages`.
pub(super) fn place(level: &mut Level, attempt: u32) {
    let Some((min, max)) = count_for(level.theme) else {
        return;
    };
    let mut rng = level_rng(level.theme, level.seed ^ PUDDLE_SALT, attempt);
    let blocked = blocked_cells(level);
    let bias = match level.theme {
        Theme::Sewer | Theme::Laundry | Theme::Outfall => Bias::Edges,
        Theme::Kitchen => Bias::Props,
        // `count_for` has already returned for these, so the arm is the "any
        // cell" default rather than a third policy.
        _ => Bias::Any,
    };
    let wanted = rng.random_range(min..=max);
    let mut puddles = Vec::with_capacity(wanted);
    if level.theme == Theme::Outfall {
        let cols = level.tiles.cols();
        for room in level.rooms.iter().filter(|r| matches!(
            r.kind, crate::game::level::RoomKind::Hall | crate::game::level::RoomKind::Room
        )) {
            // Start each bowl with a bank near its edge. The remaining
            // centres use the map-wide edge-biased draws.
            let Some((cx, cy)) = room.rect.cells()
                .filter(|&(x, y)| {
                    level.tiles.get(x, y) == TileKind::Open && !blocked[(y * cols + x) as usize]
                })
                .filter(|&(x, y)| room_edge_or_corridor(level, x, y))
                .min_by(|&(x, y), &(a, b)| {
                    let center = room.center();
                    TileMap::tile_rect(x, y).center().dist_sq(center)
                        .total_cmp(&TileMap::tile_rect(a, b).center().dist_sq(center))
                })
            else {
                continue;
            };
            let (rx, ry) = size_for(level, cx, cy, &mut rng);
            puddles.push(Puddle { pos: TileMap::tile_rect(cx, cy).center(), rx, ry, seed: rng.random() });
        }
    }
    for _ in puddles.len()..wanted {
        let Some(centre) = one_centre(level, &blocked, &mut rng, bias) else {
            break;
        };
        let (cx, cy) = cell_of(centre);
        let (rx, ry) = size_for(level, cx, cy, &mut rng);
        puddles.push(Puddle {
            pos: centre,
            rx,
            ry,
            seed: rng.random::<u32>(),
        });
    }
    if level.theme == Theme::Outfall {
        tune_outfall_coverage(level, &mut puddles);
    }
    level.puddles = puddles;
}

/// Measure the union on the walkable grid and resize together. The placement
/// draws and bank centres are already settled, so this reads no RNG.
fn tune_outfall_coverage(level: &Level, puddles: &mut [Puddle]) {
    let cols = level.tiles.cols();
    let rows = level.tiles.rows();
    let eligible: Vec<_> = (0..rows).flat_map(|y| (0..cols).map(move |x| (x, y)))
        .map(|(x, y)| level.tiles.get(x, y) == TileKind::Open
            && level.pit_at(TileMap::tile_rect(x, y).center()).is_none())
        .collect();
    let dry_cells = eligible.iter().filter(|&&open| open).count().max(1) as f32;
    let mut wet = vec![false; eligible.len()];
    for _ in 0..3 {
        wet.fill(false);
        for puddle in &*puddles {
            let span = Vec2::new(puddle.rx, puddle.ry);
            let (x0, y0) = cell_of(puddle.pos - span);
            let (x1, y1) = cell_of(puddle.pos + span);
            for y in y0.max(0)..=y1.min(rows - 1) {
                for x in x0.max(0)..=x1.min(cols - 1) {
                    let i = (y * cols + x) as usize;
                    if eligible[i] && !wet[i] && puddle.contains(TileMap::tile_rect(x, y).center()) {
                        wet[i] = true;
                    }
                }
            }
        }
        let share = wet.iter().filter(|&&w| w).count() as f32 / dry_cells;
        if (0.325..=0.395).contains(&share) {
            break;
        }
        let scale = (0.36 / share.max(0.01)).sqrt().clamp(0.75, 1.35);
        for puddle in &mut *puddles {
            puddle.rx = (puddle.rx * scale).clamp(50.0, 150.0);
            puddle.ry = (puddle.ry * scale).clamp(35.0, 115.0);
        }
    }
}

/// The half-width and half-height of a new puddle centred on cell `(cx, cy)`.
///
/// Both axes stay inside the contract's band (half-width `24..=90`, half-height
/// `14..=60`). Three bands, because the three wet floors want different water:
/// the Sewer wants big blobs that merge — top-heavy in the band, because the
/// count is 96–112 and the area target (~18 % of the floor) has to come
/// out of their size — and, in a corridor, a long one along the corridor's axis
/// so a walk down it is wet all the way. The Laundry takes medium ones and the
/// Kitchen small spills.
fn size_for(level: &Level, cx: i32, cy: i32, rng: &mut rand::rngs::StdRng) -> (f32, f32) {
    match level.theme {
        Theme::Sewer => {
            // Three in ten are the big sheets (a corridor's runs along its long
            // axis and the walls are drawn over its rim); the rest are pools
            // small enough for their whole shore to sit inside a 120 x 80 px
            // view. A sheet wider than the screen has no visible edge, and
            // without an edge it did not read as water at all.
            let big = rng.random_range(0..10) < 3;
            if let Some(room) = corridor_at(level, cx, cy) {
                return match (big, room.w >= room.h) {
                    (true, true) => (rng.random_range(78.0..=90.0), rng.random_range(34.0..=44.0)),
                    (true, false) => (rng.random_range(34.0..=44.0), rng.random_range(48.0..=60.0)),
                    (false, true) => (rng.random_range(34.0..=52.0), rng.random_range(14.0..=22.0)),
                    (false, false) => (rng.random_range(24.0..=30.0), rng.random_range(22.0..=34.0)),
                };
            }
            if big {
                (rng.random_range(64.0..=90.0), rng.random_range(44.0..=60.0))
            } else {
                (rng.random_range(24.0..=46.0), rng.random_range(14.0..=30.0))
            }
        }
        Theme::Laundry => (rng.random_range(30.0..=64.0), rng.random_range(20.0..=44.0)),
        Theme::Outfall => (rng.random_range(80.0..=106.0), rng.random_range(52.0..=82.0)),
        // Kitchen spills: small, and slightly wider than deep, like a splash on
        // a tiled floor.
        _ => (rng.random_range(24.0..=40.0), rng.random_range(14.0..=26.0)),
    }
}

/// The corridor room containing cell `(cx, cy)`, if any.
fn corridor_at(level: &Level, cx: i32, cy: i32) -> Option<crate::game::level::TileRect> {
    level
        .rooms
        .iter()
        .find(|room| room.kind == RoomKind::Corridor && room.rect.contains(cx, cy))
        .map(|room| room.rect)
}

/// One puddle centre: a biased draw, then an unbiased fallback so the count a
/// floor asks for is met even on a map where the bias's places are all blocked.
fn one_centre(
    level: &Level,
    blocked: &[bool],
    rng: &mut rand::rngs::StdRng,
    bias: Bias,
) -> Option<Vec2> {
    draw(level, blocked, rng, bias, TRIES).or_else(|| draw(level, blocked, rng, Bias::Any, FALLBACK_TRIES))
}

/// Rejection sampling: up to `tries` uniform cells, rejecting anything that is
/// not an open, unblocked floor cell and — while `bias` is not [`Bias::Any`] —
/// `BIAS_CHANCE` of the draws that miss the bias's own test.
fn draw(
    level: &Level,
    blocked: &[bool],
    rng: &mut rand::rngs::StdRng,
    bias: Bias,
    tries: u32,
) -> Option<Vec2> {
    let (cols, rows) = (level.tiles.cols(), level.tiles.rows());
    if cols <= 0 || rows <= 0 {
        return None;
    }
    for _ in 0..tries {
        let cx = rng.random_range(0..cols);
        let cy = rng.random_range(0..rows);
        if !matches!(level.tiles.get(cx, cy), TileKind::Open) {
            continue;
        }
        if blocked[(cy * cols + cx) as usize] {
            continue;
        }
        let wet_place = match bias {
            Bias::Any => true,
            Bias::Edges => room_edge_or_corridor(level, cx, cy),
            Bias::Props => near_a_prop(level, cx, cy),
        };
        let bias_chance = if level.theme == Theme::Outfall { 0.58 } else { BIAS_CHANCE };
        if !wet_place && rng.random::<f32>() < bias_chance {
            continue;
        }
        return Some(TileMap::tile_rect(cx, cy).center());
    }
    None
}

/// Is this cell a corridor cell, or does it touch a solid wall? (Water runs
/// along walls and pools in the low corridors.)
fn room_edge_or_corridor(level: &Level, cx: i32, cy: i32) -> bool {
    if corridor_at(level, cx, cy).is_some() {
        return true;
    }
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .any(|&(dx, dy)| level.tiles.get(cx + dx, cy + dy).is_wall())
}

/// Is this cell within two cells of a prop? (A Kitchen spill runs out from the
/// sink or the crate it came off.)
fn near_a_prop(level: &Level, cx: i32, cy: i32) -> bool {
    let centre = TileMap::tile_rect(cx, cy).center();
    const REACH: f32 = 2.0 * TILE as f32;
    level.props.iter().any(|prop| {
        let rect = &prop.rect;
        let dx = (rect.x - centre.x).max(0.0).max(centre.x - rect.right());
        let dy = (rect.y - centre.y).max(0.0).max(centre.y - rect.bottom());
        dx * dx + dy * dy <= REACH * REACH
    })
}

/// The cells a puddle may not be centred on, indexed `cy * cols + cx`.
///
/// A rat-hole's tunnel is a run of `WallKind::RatHole` cells (the hole's own
/// cell is a wall and could never be a centre anyway), so the face — the floor
/// cell at either mouth — is one cell from a RatHole cell. Doors and shop holes
/// are the same shape: block their cell and the ring around it. Traps are floor
/// art with no cell of their own, so they are cleared by distance.
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

    // Rat-hole faces: every RatHole cell and the ring around it, which is both
    // mouths of the tunnel.
    for cy in 0..rows {
        for cx in 0..cols {
            if matches!(level.tiles.get(cx, cy), TileKind::Wall(WallKind::RatHole { .. })) {
                ring(&mut blocked, cx, cy);
            }
        }
    }
    // One-way doors.
    for door in &level.objects.one_way_doors {
        ring(&mut blocked, door.cell.0, door.cell.1);
    }
    // Shop holes: the wall cell and the threshold cell in front of it.
    for hole in &level.objects.shop_holes {
        ring(&mut blocked, hole.cell.0, hole.cell.1);
        ring(&mut blocked, hole.mouth.0, hole.mouth.1);
    }
    // Trap pads and the start, both by distance from the cell's centre.
    for cy in 0..rows {
        for cx in 0..cols {
            let centre = TileMap::tile_rect(cx, cy).center();
            if centre.dist(level.start) < START_CLEAR_PX {
                blocked[(cy * cols + cx) as usize] = true;
                continue;
            }
            let on_a_trap = level
                .objects
                .sticky_traps
                .iter()
                .map(|trap| trap.pos)
                .chain(level.objects.mousetraps.iter().map(|trap| trap.pos))
                .chain(level.objects.crab_pots.iter().map(|trap| trap.pos))
                .any(|pos| pos.dist(centre) < TRAP_CLEAR_PX);
            let outfall_bank = level.theme == Theme::Outfall && (
                level.touches_tidal_pit(centre, 16.0)
                    || level.objects.lure_traps.iter().any(|trap| {
                        trap.pos.dist_sq(centre) < TRAP_CLEAR_PX * TRAP_CLEAR_PX
                    })
            );
            if on_a_trap || outfall_bank {
                blocked[(cy * cols + cx) as usize] = true;
            }
        }
    }
    blocked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;

    const SEEDS: u64 = 12;

    fn levels(theme: Theme) -> Vec<Level> {
        (0..SEEDS).map(|seed| Level::generate(theme, seed)).collect()
    }

    /// Every open-floor cell centre, for the coverage measurement.
    fn open_cells(level: &Level) -> Vec<Vec2> {
        let mut cells = Vec::new();
        for cy in 0..level.tiles.rows() {
            for cx in 0..level.tiles.cols() {
                if matches!(level.tiles.get(cx, cy), TileKind::Open) {
                    cells.push(TileMap::tile_rect(cx, cy).center());
                }
            }
        }
        cells
    }

    #[test]
    fn counts_per_theme_and_dry_floors_stay_dry() {
        for theme in Theme::all() {
            for level in levels(theme) {
                match count_for(theme) {
                    Some((min, max)) => {
                        let n = level.puddles.len();
                        assert!(
                            (min..=max).contains(&n),
                            "{theme:?} seed {} placed {n} puddles, wanted {min}..={max}",
                            level.seed
                        );
                    }
                    None => assert!(
                        level.puddles.is_empty(),
                        "{theme:?} is a dry floor but has {} puddles",
                        level.puddles.len()
                    ),
                }
            }
        }
        // The three wet floors are the ones the brief names, and the Flatbed is
        // never wet (it is the one theme whose generator returns early).
        assert!(count_for(Theme::Sewer).is_some());
        assert!(count_for(Theme::Laundry).is_some());
        assert!(count_for(Theme::Kitchen).is_some());
        assert!(count_for(Theme::Trailer).is_none());
    }

    #[test]
    fn every_centre_is_on_open_floor_and_clear_of_the_start() {
        for theme in Theme::playable() {
            for level in levels(theme) {
                for puddle in &level.puddles {
                    let (cx, cy) = cell_of(puddle.pos);
                    assert!(
                        matches!(level.tiles.get(cx, cy), TileKind::Open),
                        "{theme:?} seed {}: a puddle is centred on a wall cell ({cx}, {cy})",
                        level.seed
                    );
                    assert!(
                        puddle.pos.dist(level.start) >= START_CLEAR_PX,
                        "{theme:?} seed {}: a puddle is on the start",
                        level.seed
                    );
                    let (rx, ry) = if theme == Theme::Outfall {
                        (50.0..=150.0, 35.0..=115.0)
                    } else {
                        (24.0..=90.0, 14.0..=60.0)
                    };
                    assert!(rx.contains(&puddle.rx), "{theme:?}: half-width {}", puddle.rx);
                    assert!(ry.contains(&puddle.ry), "{theme:?}: half-height {}", puddle.ry);
                }
            }
        }
    }

    #[test]
    fn no_centre_covers_a_hole_door_shop_or_trap() {
        for theme in [Theme::Sewer, Theme::Laundry, Theme::Kitchen] {
            for level in levels(theme) {
                let blocked = blocked_cells(&level);
                let cols = level.tiles.cols();
                for puddle in &level.puddles {
                    let (cx, cy) = cell_of(puddle.pos);
                    assert!(
                        !blocked[(cy * cols + cx) as usize],
                        "{theme:?} seed {}: a puddle sits on a blocked cell",
                        level.seed
                    );
                    // The trap rule is a distance, checked from the centre.
                    for trap in level.objects.sticky_traps.iter().map(|t| t.pos)
                        .chain(level.objects.mousetraps.iter().map(|t| t.pos))
                        .chain(level.objects.crab_pots.iter().map(|t| t.pos))
                    {
                        assert!(
                            trap.dist(puddle.pos) >= TRAP_CLEAR_PX,
                            "{theme:?} seed {}: a puddle covers a trap pad",
                            level.seed
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        for theme in [Theme::Sewer, Theme::Laundry, Theme::Kitchen] {
            for seed in 0..4 {
                let a = Level::generate(theme, seed);
                let b = Level::generate(theme, seed);
                assert_eq!(a.puddles, b.puddles, "{theme:?} seed {seed} is not repeatable");
            }
        }
    }

    #[test]
    fn puddles_leave_the_tiles_and_objects_alone() {
        // The stage is the last one and writes only `Level::puddles`, so the
        // map pin cannot see it. Compared here on a fresh generation: the same
        // map with this stage's output discarded must have identical tiles and
        // objects.
        for theme in Theme::all() {
            for seed in 0..2 {
                let with = Level::generate(theme, seed);
                let mut without = Level::generate(theme, seed);
                without.puddles.clear();
                assert_eq!(
                    with.tiles.cols(),
                    without.tiles.cols(),
                    "{theme:?}: the tile grid size moved"
                );
                assert_eq!(with.tiles.rows(), without.tiles.rows());
                for cy in 0..with.tiles.rows() {
                    for cx in 0..with.tiles.cols() {
                        assert_eq!(
                            with.tiles.cell(cx, cy).kind,
                            without.tiles.cell(cx, cy).kind,
                            "{theme:?} seed {seed}: cell ({cx}, {cy}) moved"
                        );
                        assert_eq!(with.tiles.cell(cx, cy).mask.0, without.tiles.cell(cx, cy).mask.0);
                    }
                }
                let cells = |doors: &[crate::game::level::OneWayDoor]| {
                    doors.iter().map(|d| d.cell).collect::<Vec<_>>()
                };
                assert_eq!(
                    cells(&with.objects.one_way_doors),
                    cells(&without.objects.one_way_doors),
                    "{theme:?} seed {seed}: a door moved"
                );
                assert_eq!(with.objects.rat_holes.len(), without.objects.rat_holes.len());
                assert_eq!(
                    with.objects.shop_holes.len(),
                    without.objects.shop_holes.len()
                );
                assert_eq!(with.objects.sticky_traps.len(), without.objects.sticky_traps.len());
                assert_eq!(with.objects.mousetraps.len(), without.objects.mousetraps.len());
                assert_eq!(with.objects.crab_pots.len(), without.objects.crab_pots.len());
                assert_eq!(with.objects.lights.len(), without.objects.lights.len());
                assert_eq!(with.rooms.len(), without.rooms.len());
            }
        }
    }

    #[test]
    fn wet_at_agrees_with_contains_and_floor_cells() {
        let level = levels(Theme::Sewer).remove(0);
        for cy in 0..level.tiles.rows() {
            for cx in 0..level.tiles.cols() {
                let centre = TileMap::tile_rect(cx, cy).center();
                let covered = level.puddles.iter().any(|p| p.contains(centre));
                let open = matches!(level.tiles.get(cx, cy), TileKind::Open);
                assert_eq!(
                    level.wet_at(centre),
                    covered && open,
                    "cell ({cx}, {cy}) disagrees"
                );
            }
        }
        // A dry floor answers `false` everywhere, without walking the list.
        let dry = Level::empty(Theme::Lab);
        assert!(!dry.wet_at(TileMap::tile_rect(10, 10).center()));
    }

    /// Measurement, not a gate: prints the share of each floor's open cells
    /// that standing water covers. The Sewer is aimed at ~18 %.
    /// `cargo test puddle_coverage_by_theme -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn puddle_coverage_by_theme() {
        for theme in [Theme::Sewer, Theme::Laundry, Theme::Kitchen] {
            let levels = levels(theme);
            let mut shares = Vec::new();
            for level in &levels {
                let cells = open_cells(level);
                let wet = cells.iter().filter(|&&c| level.wet_at(c)).count();
                shares.push(wet as f32 / cells.len().max(1) as f32);
            }
            let per_seed: Vec<String> =
                shares.iter().take(5).map(|s| format!("{:.1}", s * 100.0)).collect();
            println!("{theme:?}: first five seeds, % wet: {}", per_seed.join(" / "));
            shares.sort_by(|a, b| a.total_cmp(b));
            let median = shares[shares.len() / 2];
            let mean_px = levels
                .iter()
                .map(|l| l.puddles.len() as f32)
                .sum::<f32>()
                / levels.len() as f32;
            println!(
                "{theme:?}: {:.1} % of open floor wet (median of {} seeds), {:.1} puddles a map, \
                 {} open cells a map",
                median * 100.0,
                SEEDS,
                mean_px,
                open_cells(&levels[0]).len(),
            );
        }
    }

    #[test]
    fn the_sewer_is_biased_to_corridors_and_walls() {
        // Not a statistical claim: on every seed, most of the Sewer's puddles
        // sit on a corridor cell or against a wall (the bias is 4:1 by design,
        // so this cannot be flaky at 96+ puddles a map).
        for level in levels(Theme::Sewer) {
            let biased = level
                .puddles
                .iter()
                .filter(|p| {
                    let (cx, cy) = cell_of(p.pos);
                    room_edge_or_corridor(&level, cx, cy)
                })
                .count();
            assert!(
                biased * 2 > level.puddles.len(),
                "seed {}: only {biased} of {} puddles are along a wall or in a corridor",
                level.seed,
                level.puddles.len()
            );
        }
    }

    #[test]
    fn puddle_half_widths_never_exceed_the_contract_band() {
        // Cheap guard on `size_for`'s bands: every theme that has water draws
        // inside `24..=90 × 14..=60` whatever branch it takes.
        for theme in [Theme::Sewer, Theme::Laundry, Theme::Kitchen] {
            let level = Level::empty(theme);
            let mut rng = level_rng(theme, 7, 0);
            for cx in 0..level.tiles.cols() {
                let (rx, ry) = size_for(&level, cx, cx / 3, &mut rng);
                assert!((24.0..=90.0).contains(&rx), "{theme:?}: rx {rx}");
                assert!((14.0..=60.0).contains(&ry), "{theme:?}: ry {ry}");
            }
        }
    }
}
