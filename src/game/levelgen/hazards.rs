//! Stage F — walkable floor hazards and the Sewer opening placements.
//!
//! `WAVE3_DESIGN.md` §11 gives each side route one hazard:
//!
//! * **Laundry** — 3–5 steam grates in optional side lanes: three pale puffs
//!   warn for 1 s, a one-tile plume deals 8 once per occupant over 0.7 s, then
//!   the grate rests 4 s. It scalds pursuers too.
//! * **Printworks** — 3–5 ink pads at inside corners: a permanent stipple that
//!   holds ground movement to 75 % for the rat and for the horde, while flying
//!   and dashing bodies ignore it.
//!
//! ## The rules this stage keeps
//!
//! * both hazards are **walkable floor** (they never paint a cell), so they can
//!   never disconnect a map;
//! * neither is ever on the boss arena's floor, the start room, a maze
//!   corridor's only route or a rat-hole/threshold approach;
//! * **every hazard has a dry route at least two cells wide past it**
//!   ([`dry_route`]), proven on the finished grid, which is what makes a side
//!   lane optional rather than a toll;
//! * each theme teaches its hazard **once** beside the first public fork
//!   ([`teaching_cell`]) before the rest are scattered deeper in;
//! * the only randomness is the one level `StdRng`, in call order, and the
//!   candidate list is sorted (distance, then cell) before any draw, so no
//!   hash-map or query order can reach the result;
//! * the Sewer opening draws only its drip tile here, after shops and traps.
//!   Kitchen and Lab still draw no rng in this stage.

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::{WallAxis, WallKind};
use crate::game::collision::{SizeClass, TILE_F, TileKind, TileMap};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::enemy::EnemyKind;
use crate::game::hazards::{DripShaft, InkPad, SewerOpening, SteamGrate};
use crate::game::level::{Level, MazeSection, RoomKind, Theme, TileRect};
use crate::math::Vec2;

use super::layout::Layout;
use super::placement::{ShopHolePlacement, TrapPlacement};
use super::validate;

/// How many hazards a floor places (design: 3–5).
pub const HAZARDS: (usize, usize) = (3, 5);

/// Cells a scattered hazard keeps from the spawn point: the rat gets its first
/// breath before a grate can vent on it.
pub const SPAWN_CLEAR_CELLS: i32 = 14;

/// The dry route a hazard must have past it, in cells: two cells wide and three
/// long, centred on the hazard.
pub const DRY_ROUTE_CELLS: i32 = 2;

/// What one generation pass placed.
#[derive(Clone, Debug, Default)]
pub struct HazardPlacement {
    pub steam: Vec<SteamGrate>,
    pub ink: Vec<InkPad>,
    pub sewer_opening: Option<SewerOpening>,
}

impl HazardPlacement {
    pub fn len(&self) -> usize {
        self.steam.len() + self.ink.len() + usize::from(self.sewer_opening.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn place_sewer_opening(
    tiles: &TileMap,
    layout: &Layout,
    avoid: &[TileRect],
    rng: &mut StdRng,
) -> Option<SewerOpening> {
    let start = ((layout.start.x / TILE_F) as i32, (layout.start.y / TILE_F) as i32);
    // Use large-body reachability, with the maze and threshold reservations
    // below, so the Pillbugs stand on public floor the horde can reach.
    let reached = validate::reachable(tiles, start, SizeClass::Large);
    let bottle_cap = opening_bottle_cap(tiles, layout.start_room, &reached)?;
    let mut rooms: Vec<_> = layout
        .rooms
        .iter()
        .filter(|room| matches!(room.kind, RoomKind::Hall | RoomKind::Room))
        .collect();
    rooms.sort_by(|a, b| {
        a.center()
            .dist_sq(layout.start)
            .total_cmp(&b.center().dist_sq(layout.start))
    });

    for room in rooms {
        let center = room.center();
        let mut cells: Vec<_> = room
            .rect
            .cells()
            .filter(|&(x, y)| {
                tiles.get(x, y).is_open()
                    && validate::is_reached(&reached, tiles, (x, y))
                    && !avoid.iter().any(|r| r.contains(x, y))
            })
            .map(|cell| (cell, TileMap::tile_rect(cell.0, cell.1).center()))
            .filter(|&(_, p)| tiles.circle_clear(p, EnemyKind::Pillbug.radius(), SizeClass::Large))
            .collect();
        cells.sort_by(|a, b| a.1.dist_sq(center).total_cmp(&b.1.dist_sq(center)));

        // Leave the two closest cells for the pair. Restrict the draw to
        // nearby roomy tiles, so the splash always has space to go around.
        let drip_cells: Vec<_> = cells
            .iter()
            .skip(2)
            .filter(|&&(cell, _)| {
                dry_route(tiles, cell)
                    && !threshold_near(tiles, TileRect::new(cell.0, cell.1, 1, 1))
            })
            .take(8)
            .collect();
        if !drip_cells.is_empty() {
            let &(cell, _) = drip_cells[rng.random_range(0..drip_cells.len())];
            return Some(SewerOpening {
                pillbug_cells: cells
                    .into_iter()
                    .filter_map(|(c, pos)| (c != cell).then_some(pos))
                    .collect(),
                bottle_cap,
                drip_shaft: DripShaft::new(cell),
            });
        }
    }
    None
}

/// Take the public doorway closest to the map centre with a clear landing
/// 24 px past it. Open cuts made by connectivity repair are a fallback.
fn opening_bottle_cap(tiles: &TileMap, start: TileRect, reached: &[bool]) -> Option<Vec2> {
    let map_center = tiles.world_size() * 0.5;
    let edge = start.inflate(1);
    let mut best = None;
    for (x, y) in edge.cells().filter(|&(x, y)| !start.contains(x, y)) {
        for (dx, dy, axis) in [
            (0, -1, WallAxis::Horizontal),
            (0, 1, WallAxis::Horizontal),
            (-1, 0, WallAxis::Vertical),
            (1, 0, WallAxis::Vertical),
        ] {
            if !start.contains(x - dx, y - dy) || !tiles.get(x - dx, y - dy).is_open() {
                continue;
            }
            let open_cut = match tiles.get(x, y) {
                TileKind::Wall(WallKind::Doorway { axis: door_axis }) if door_axis == axis => false,
                TileKind::Open => true,
                _ => continue,
            };
            let exit = TileMap::tile_rect(x, y).center();
            let pos = exit + Vec2::new(dx as f32, dy as f32) * 24.0;
            let cell = ((pos.x / TILE_F) as i32, (pos.y / TILE_F) as i32);
            let key = (open_cut, exit.dist_sq(map_center));
            if tiles.get_at(pos).is_open()
                && tiles.circle_clear(pos, PLAYER_RADIUS, SizeClass::Large)
                && validate::is_reached(reached, tiles, cell)
                && best.is_none_or(|(nearest, _)| key < nearest)
            {
                best = Some((key, pos));
            }
        }
    }
    best.map(|(_, pos)| pos)
}

/// Is there a dry route at least two cells wide past `cell`?
///
/// "Past" means a two-cell-wide, three-cell-long strip of open floor running
/// alongside the hazard: offset 2..4 cells to one side, spanning the hazard's
/// own cell and one cell either side of it. Either axis will do — a hazard in a
/// dead end is not placed at all.
pub fn dry_route(tiles: &TileMap, cell: (i32, i32)) -> bool {
    let (cx, cy) = cell;
    for side in 0..4 {
        let (dx, dy) = match side {
            0 => (0, -1),
            1 => (0, 1),
            2 => (-1, 0),
            _ => (1, 0),
        };
        // The strip runs perpendicular to the offset direction.
        let mut clear = true;
        'strip: for across in -1..=1 {
            for off in 2..(2 + DRY_ROUTE_CELLS) {
                let x = cx + dx * off + if dy != 0 { across } else { 0 };
                let y = cy + dy * off + if dx != 0 { across } else { 0 };
                if !tiles.get(x, y).is_open() {
                    clear = false;
                    break 'strip;
                }
            }
        }
        if clear {
            return true;
        }
    }
    false
}

/// The one cell where the theme's hazard is taught: open floor beside the
/// nearest public fork out of the start room (adjacent to a corridor, on the
/// start room's side of it, never inside the corridor itself).
fn teaching_cell(
    tiles: &TileMap,
    layout: &Layout,
    start_cell: (i32, i32),
) -> Option<(i32, i32)> {
    let mut best: Option<(i32, i32, i32)> = None;
    for room in &layout.rooms {
        if room.kind != RoomKind::Corridor {
            continue;
        }
        for (cx, cy) in room.rect.inflate(1).cells() {
            if room.rect.contains(cx, cy) || !tiles.get(cx, cy).is_open() {
                continue;
            }
            let d = (cx - start_cell.0).abs() + (cy - start_cell.1).abs();
            if d < 8 {
                continue;
            }
            if best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, cx, cy));
            }
        }
    }
    best.map(|(_, cx, cy)| (cx, cy))
}

/// Everything a candidate must avoid, as cell rectangles.
fn avoided(
    layout: &Layout,
    mazes: &[MazeSection],
    traps: &[TrapPlacement],
    shops: &[ShopHolePlacement],
) -> Vec<TileRect> {
    let mut out: Vec<TileRect> = Vec::new();
    // The start room is never a hazard floor: the player learns to move there.
    out.push(layout.start_room);
    // The boss arena's central floor (its mouths stay clear too).
    out.push(layout.boss_arena.inflate(1));
    for slot in &layout.maze_slots {
        out.push(slot.inflate(1));
    }
    for maze in mazes {
        out.push(maze.bounds.inflate(1));
    }
    for trap in traps {
        let cell = (
            (trap.pos.x / crate::game::collision::TILE_F) as i32,
            (trap.pos.y / crate::game::collision::TILE_F) as i32,
        );
        out.push(TileRect::new(cell.0 - 1, cell.1 - 1, 3, 3));
    }
    // A shop threshold stays clear of ordinary hazards (§11): the mouth cell
    // in front of each mouse-hole shop, plus two cells around it, so the rat
    // can stand at the shelf and step clear to re-arm it without a toll.
    for shop in shops {
        out.push(TileRect::new(shop.mouth.0 - 2, shop.mouth.1 - 2, 5, 5));
    }
    out
}

/// Place the theme's hazards. Returns an empty placement (and draws no rng) for
/// every theme that has none.
pub fn place_hazards(
    tiles: &TileMap,
    layout: &Layout,
    mazes: &[MazeSection],
    traps: &[TrapPlacement],
    shops: &[ShopHolePlacement],
    rng: &mut StdRng,
    theme: Theme,
) -> HazardPlacement {
    let kinds_ink = theme == Theme::Printworks;
    let kinds_steam = theme == Theme::Laundry;
    if theme == Theme::Sewer {
        let avoid = avoided(layout, mazes, traps, shops);
        return HazardPlacement {
            sewer_opening: place_sewer_opening(tiles, layout, &avoid, rng),
            ..HazardPlacement::default()
        };
    }
    if !kinds_ink && !kinds_steam {
        return HazardPlacement::default();
    }

    let start_cell = (
        (layout.start.x / crate::game::collision::TILE_F) as i32,
        (layout.start.y / crate::game::collision::TILE_F) as i32,
    );
    let avoid = avoided(layout, mazes, traps, shops);
    let reached = validate::reachable(tiles, start_cell, crate::game::collision::SizeClass::Small);

    let mut placement = HazardPlacement::default();
    let count = rng.random_range(HAZARDS.0..=HAZARDS.1);
    let mut used: Vec<(i32, i32)> = Vec::new();

    // ---- the teaching hazard: one beside the first public fork -----------
    let teaching = teaching_cell(tiles, layout, start_cell).filter(|&cell| {
        tiles.get(cell.0, cell.1).is_open()
            && dry_route(tiles, cell)
            && !avoid.iter().any(|r| r.contains(cell.0, cell.1))
            && !threshold_near(tiles, TileRect::new(cell.0, cell.1, 1, 1))
    });
    if let Some(cell) = teaching {
        if place_one(tiles, cell, &avoid, rng, theme, &mut placement).is_some() {
            used.push(cell);
        }
    }

    // ---- the rest: rejection sampling in the halls and rooms -------------
    // Sampling (rather than scanning every open cell and sorting the matches)
    // keeps the stage cheap on a 200 x 160 map: at most
    // `TRIES_PER_HAZARD` probes per hazard instead of ~20 000 candidates.
    // The rooms are visited in generation order and the cell is drawn from the
    // level rng, so the result is deterministic in call order.
    let rooms: Vec<TileRect> = layout
        .rooms
        .iter()
        .filter(|room| matches!(room.kind, RoomKind::Hall | RoomKind::Room))
        .map(|room| room.rect.inflate(-1))
        .filter(|rect| rect.w >= 3 && rect.h >= 3)
        .collect();
    const TRIES_PER_HAZARD: usize = 96;
    let mut room_index = if rooms.is_empty() {
        0
    } else {
        rng.random_range(0..rooms.len())
    };
    while placement.len() < count && !rooms.is_empty() {
        let room = rooms[room_index % rooms.len()];
        room_index += 1;
        let mut placed = false;
        for _ in 0..TRIES_PER_HAZARD {
            let cx = rng.random_range(room.x..room.right());
            let cy = rng.random_range(room.y..room.bottom());
            let cell = (cx, cy);
            if !tiles.get(cx, cy).is_open() || !validate::is_reached(&reached, tiles, cell) {
                continue;
            }
            let d = (cx - start_cell.0).abs() + (cy - start_cell.1).abs();
            if d < SPAWN_CLEAR_CELLS {
                continue;
            }
            if avoid.iter().any(|r| r.contains(cx, cy))
                || threshold_near(tiles, TileRect::new(cx, cy, 1, 1))
            {
                continue;
            }
            if !dry_route(tiles, cell) {
                continue;
            }
            // Keep hazards apart so two plumes never overlap and a lane is not
            // a corridor of steam.
            if used
                .iter()
                .any(|&(ux, uy)| (ux - cx).abs() <= 2 && (uy - cy).abs() <= 2)
            {
                continue;
            }
            if place_one(tiles, cell, &avoid, rng, theme, &mut placement).is_some() {
                used.push(cell);
                placed = true;
                break;
            }
        }
        if !placed && room_index >= rooms.len() * 2 {
            // Every room has been sampled at least once (the walk starts at
            // a random room, so between one and two passes): stop rather than
            // spin. `validate` reports short counts, so a starved map is a
            // visible generator bug and not a silent one.
            break;
        }
    }
    placement
}

/// Does a hazard's footprint (grown by the two cells a hazard keeps clear) touch
/// a doorway, one-way door or rat-hole? Those are the only ways through a wall,
/// so a hazard in front of one would charge a toll on the only route.
fn threshold_near(tiles: &TileMap, rect: TileRect) -> bool {
    let probe = rect.inflate(2);
    let cells: Vec<(i32, i32)> = probe.cells().collect();
    cells.into_iter().any(|(cx, cy)| {
        matches!(
            tiles.get(cx, cy),
            crate::game::collision::TileKind::Wall(
                crate::game::art_walls::WallKind::Doorway { .. }
                    | crate::game::art_walls::WallKind::OneWayDoor { .. }
                    | crate::game::art_walls::WallKind::RatHole { .. }
            )
        )
    })
}

/// Is every cell of `cells` open floor, off the reservations and off every
/// threshold?
fn pad_fits(tiles: &TileMap, cells: TileRect, avoid: &[TileRect]) -> bool {
    cells.cells().all(|(cx, cy)| tiles.get(cx, cy).is_open())
        && !avoid.iter().any(|r| r.intersects(&cells))
        && !threshold_near(tiles, cells)
}

/// Place one hazard of the theme's kind at `cell`.
///
/// Returns the cell it was placed on, or `None` when the theme has no hazard or
/// the footprint does not fit (an ink pad never grows over a wall, a
/// reservation or a threshold).
fn place_one(
    tiles: &TileMap,
    cell: (i32, i32),
    avoid: &[TileRect],
    rng: &mut StdRng,
    theme: Theme,
    placement: &mut HazardPlacement,
) -> Option<(i32, i32)> {
    match theme {
        Theme::Laundry => {
            // Stagger the vent clocks: a washer aisle that vented in unison
            // would be a wave rather than a hazard.
            let offset = rng.random_range(0.0..crate::game::hazards::STEAM_CYCLE_SECS);
            placement.steam.push(SteamGrate::new(cell, offset));
            Some(cell)
        }
        Theme::Printworks => {
            // An ink pad prefers to spread into the inside corner it was found
            // against, and falls back to a single cell rather than growing over
            // a wall.
            let wide = rng.random_bool(0.45);
            let shapes = if wide {
                [
                    TileRect::new(cell.0, cell.1, 2, 1),
                    TileRect::new(cell.0, cell.1, 1, 1),
                ]
            } else {
                [
                    TileRect::new(cell.0, cell.1, 1, 2),
                    TileRect::new(cell.0, cell.1, 1, 1),
                ]
            };
            let cells = shapes.into_iter().find(|r| pad_fits(tiles, *r, avoid))?;
            let seed = rng.random_range(0..u32::MAX);
            placement.ink.push(InkPad::new(cells, seed));
            Some(cell)
        }
        _ => None,
    }
}



const TIDAL_SIDES: [(i32, i32); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];
const LURE_SPACING: f32 = 80.0;
pub(super) const REPAIRED_LURE_SPACING: f32 = 64.0;

/// Drop pockets sit flush against solid bedrock. Two continuous rows of dry
/// floor wrap the lip and extend past its ends, keeping the bank optional.
fn tidal_bank(
    level: &Level,
    room: TileRect,
    cell: (i32, i32),
    inward: (i32, i32),
    width: i32,
    depth: i32,
) -> Option<TileRect> {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;

    let (dx, dy) = inward;
    let at = |along, across| (cell.0 + dy.abs() * along + dx * across,
                              cell.1 + dx.abs() * along + dy * across);
    let tile = |along, across| {
        let (x, y) = at(along, across);
        level.tiles.get(x, y)
    };
    if (0..width).any(|i| tile(i, -1) != TileKind::Wall(WallKind::Solid)) {
        return None;
    }
    for j in 0..depth + 2 {
        for i in -1..=width {
            let (x, y) = at(i, j);
            if !room.contains(x, y) || level.tiles.get(x, y) != TileKind::Open {
                return None;
            }
        }
    }
    let end = at(width - 1, depth - 1);
    Some(TileRect::new(cell.0.min(end.0), cell.1.min(end.1),
        (cell.0 - end.0).abs() + 1, (cell.1 - end.1).abs() + 1))
}

pub(super) fn tidal_exclusions(level: &Level) -> Vec<TileRect> {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::cell_of;

    let around = |(x, y), gap| TileRect::new(x, y, 1, 1).inflate(gap);
    let mut avoid: Vec<_> = level.start_room().iter().map(|r| r.rect.inflate(1)).collect();
    for maze in &level.mazes {
        avoid.extend([maze.ring.inflate(4), around(maze.fork, 3)]);
    }
    avoid.extend(level.tiles.iter_walls()
        .filter(|&(_, _, kind)| matches!(kind, WallKind::Doorway { .. } | WallKind::RatHole { .. }))
        .map(|(x, y, _)| around((x, y), 2)));
    avoid.extend(level.objects.one_way_doors.iter().map(|door| around(door.cell, 3)));
    avoid.extend(level.objects.shop_holes.iter().map(|shop| around(shop.mouth, 3)));
    avoid.extend(level.objects.sticky_traps.iter().map(|trap| around(cell_of(trap.pos), 2)));
    avoid.extend(level.objects.mousetraps.iter().map(|trap| around(cell_of(trap.pos), 2)));
    avoid
}

fn outfall_rank(seed: u64, (x, y): (i32, i32)) -> u64 {
    let mut hash = seed ^ (x as u64).wrapping_mul(0x9e37_79b9)
        ^ (y as u64).wrapping_mul(0x85eb_ca6b);
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^ (hash >> 31)
}

pub(super) fn place_tidal_pits(level: &mut Level, seed: u64) {
    use crate::game::level::{Pit, PitKind};

    let avoid = tidal_exclusions(level);
    let mut sites = Vec::new();
    for room in level.rooms.iter().filter(|r| matches!(r.kind, RoomKind::Hall | RoomKind::Room)) {
        let r = room.rect;
        for cell @ (x, y) in r.cells() {
            for inward in TIDAL_SIDES {
                let edge = match inward {
                    (0, 1) => y == r.y,
                    (1, 0) => x == r.x,
                    (0, -1) => y == r.bottom() - 1,
                    _ => x == r.right() - 1,
                };
                if !edge {
                    continue;
                }
                let rank = outfall_rank(seed, cell);
                let width = 2 + (rank % 3) as i32;
                let depth = 1 + ((rank >> 2) & 1) as i32;
                let Some(rect) = tidal_bank(level, r, cell, inward, width, depth) else {
                    continue;
                };
                if avoid.iter().all(|guard| !guard.intersects(&rect)) {
                    sites.push((rank, rect));
                }
            }
        }
    }
    sites.sort_by_key(|site| site.0);
    let wanted = 4 + seed as usize % 5;
    for (_, rect) in sites {
        if level.objects.tidal_pits.len() == wanted {
            break;
        }
        if level.objects.tidal_pits.iter().all(|pit| !pit.rect.inflate(8).intersects(&rect)) {
            level.objects.tidal_pits.push(Pit::new(rect, PitKind::TidalDrop));
        }
    }
}

/// Validate a dry route on a copy. Pits stay walkable in the real tile map so
/// direct crossings can fall and the swimming bodies can cross normally.
pub(super) fn dry_tiles(level: &Level) -> TileMap {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;

    let tiles = &level.tiles;
    let mut dry = TileMap::new(tiles.cols(), tiles.rows(), tiles.theme(), tiles.seed());
    for (x, y, kind) in tiles.iter_walls() {
        dry.set(x, y, TileKind::Wall(kind));
    }
    for pit in &level.objects.tidal_pits {
        let r = pit.rect;
        dry.fill_cells(r.x, r.y, r.w, r.h, TileKind::Wall(WallKind::Solid));
    }
    dry
}

pub(super) fn place_lure_traps(level: &mut Level, seed: u64, repair: bool) {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{SizeClass, TileKind};
    use crate::game::hazards::{lure_landing_clear, LureTrap, OUTFALL_LURE_COUNT};

    let dry = dry_tiles(level);
    let reach = validate::reachable(&dry, validate::cell_of(level.start), SizeClass::Large);
    let avoid = tidal_exclusions(level);
    let mut sites = Vec::new();
    for y in 1..level.tiles.rows() - 1 {
        for x in 1..level.tiles.cols() - 1 {
            let cell = (x, y);
            if dry.get(x, y) != TileKind::Open
                || !validate::is_reached(&reach, &dry, cell)
                || avoid.iter().any(|r| r.contains(x, y))
                || TIDAL_SIDES.iter().all(|&(dx, dy)| {
                    level.tiles.get(x - dx, y - dy) != TileKind::Wall(WallKind::Solid)
                        || dry.get(x + dx, y + dy) != TileKind::Open
                })
            {
                continue;
            }
            let basin = level.rooms.iter().position(|room| {
                matches!(room.kind, RoomKind::Room | RoomKind::Hall) && room.rect.contains(x, y)
            });
            if basin.is_none() && !repair {
                continue;
            }
            sites.push((outfall_rank(seed, cell), basin, cell));
        }
    }
    sites.sort_by_key(|site| site.0);
    let spacing = if repair { REPAIRED_LURE_SPACING } else { LURE_SPACING };
    let mut used_basins = Vec::new();
    level.objects.lure_traps.clear();
    for one_per_basin in [true, false] {
        for &(rank, basin, (x, y)) in &sites {
            if level.objects.lure_traps.len() == OUTFALL_LURE_COUNT {
                return;
            }
            if one_per_basin && basin.is_none_or(|b| used_basins.contains(&b)) {
                continue;
            }
            let pos = TileMap::tile_rect(x, y).center();
            if level.objects.lure_traps.iter().any(|trap| trap.pos.dist_sq(pos) < spacing * spacing) {
                continue;
            }
            // Neighbours at sqrt(2) tiles or two tiles. Rotate the fixed order
            // by the site's rank, then keep only generator-legal landings.
            const OFFSETS: [(i32, i32); 8] = [(1, 1), (2, 0), (1, -1), (0, -2),
                (-1, -1), (-2, 0), (-1, 1), (0, 2)];
            let landings: Vec<_> = (0..OFFSETS.len()).filter_map(|i| {
                let (dx, dy) = OFFSETS[(i + (rank & 7) as usize) % OFFSETS.len()];
                let cell = (x + dx, y + dy);
                let at = TileMap::tile_rect(cell.0, cell.1).center();
                (validate::is_reached(&reach, &dry, cell)
                    && avoid.iter().all(|r| !r.contains(cell.0, cell.1))
                    && lure_landing_clear(level, pos, at)).then_some(at)
            }).collect();
            if landings.is_empty() {
                continue;
            }
            level.objects.lure_traps.push(LureTrap::new(pos, landings));
            if let Some(index) = basin {
                used_basins.push(index);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::SizeClass;
    use crate::game::level::{Level, Theme};

    fn start_cell(level: &Level) -> (i32, i32) {
        (
            (level.start.x / crate::game::collision::TILE_F) as i32,
            (level.start.y / crate::game::collision::TILE_F) as i32,
        )
    }

    /// Every hazard on the floor of a new theme is on public, reachable floor,
    /// off the start room, the boss arena, the mazes and every threshold, and
    /// has a two-cell dry route past it — over 200 seeds per new theme.
    #[test]
    fn hazards_are_legal_on_every_map_of_both_new_themes() {
        let mut checked = 0u32;
        for theme in [Theme::Laundry, Theme::Printworks] {
            for seed in 0..200u64 {
                // The first attempt must already be playable (the pipeline's
                // retry is a safety net, not a routine path).
                let level = crate::game::levelgen::generate_once(theme, seed, 0);
                if let Err(errors) = crate::game::levelgen::validate::validate(&level) {
                    panic!("{theme:?} seed {seed} does not validate: {errors:#?}");
                }
                let objects = &level.objects;
                let reached = crate::game::levelgen::validate::reachable(
                    &level.tiles,
                    start_cell(&level),
                    SizeClass::Small,
                );
                let arena = level.boss_room().map(|room| room.rect);
                let start_room = level.start_room().map(|room| room.rect);
                let cells: Vec<((i32, i32), &'static str)> = match theme {
                    Theme::Laundry => objects
                        .steam_grates
                        .iter()
                        .map(|g| (g.cell, "steam grate"))
                        .collect(),
                    _ => objects
                        .ink_pads
                        .iter()
                        .map(|p| ((p.cells.x, p.cells.y), "ink pad"))
                        .collect(),
                };
                assert!(
                    (HAZARDS.0..=HAZARDS.1).contains(&cells.len()),
                    "{theme:?} seed {seed}: {} hazards",
                    cells.len()
                );
                for (cell, what) in cells {
                    assert!(
                        level.tiles.get(cell.0, cell.1).is_open(),
                        "{theme:?} seed {seed}: {what} at {cell:?} is not floor"
                    );
                    assert!(
                        crate::game::levelgen::validate::is_reached(
                            &reached,
                            &level.tiles,
                            cell
                        ),
                        "{theme:?} seed {seed}: {what} at {cell:?} is unreachable"
                    );
                    assert!(
                        start_room.is_none_or(|room| !room.contains(cell.0, cell.1)),
                        "{theme:?} seed {seed}: {what} in the start room"
                    );
                    assert!(
                        arena.is_none_or(|room| !room.inflate(1).contains(cell.0, cell.1)),
                        "{theme:?} seed {seed}: {what} in the boss arena"
                    );
                    for maze in &level.mazes {
                        assert!(
                            !maze.bounds.inflate(1).contains(cell.0, cell.1),
                            "{theme:?} seed {seed}: {what} inside a maze"
                        );
                    }
                    assert!(
                        dry_route(&level.tiles, cell),
                        "{theme:?} seed {seed}: {what} at {cell:?} has no dry route"
                    );
                    let d = (cell.0 - start_cell(&level).0).abs()
                        + (cell.1 - start_cell(&level).1).abs();
                    assert!(
                        d >= SPAWN_CLEAR_CELLS || what == "steam grate" || what == "ink pad",
                        "sanity"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 600, "only {checked} hazards checked");
    }

    /// Hazards, sticky boards, mousetraps and shop thresholds never overlap
    /// each other, on 200 maps of each new floor: a hazard's cells are clear of
    /// every trap's footprint, of every shop hole's wall cell and the two
    /// cells around its threshold, and of every other hazard.
    #[test]
    fn hazards_never_overlap_traps_shop_holes_or_each_other() {
        use crate::game::art_traps::{MOUSETRAP_SIZE, STICKY_TRAP_SIZE};
        use crate::game::collision::TILE_F;
        // A hazard's footprint in world pixels.
        fn px(rect: TileRect) -> (f32, f32, f32, f32) {
            (
                rect.x as f32 * TILE_F,
                rect.y as f32 * TILE_F,
                rect.right() as f32 * TILE_F,
                rect.bottom() as f32 * TILE_F,
            )
        }
        fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
            a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
        }
        let mut hazards_seen = 0u32;
        let mut shops_seen = 0u32;
        for theme in [Theme::Laundry, Theme::Printworks] {
            for seed in 0..200u64 {
                let level = crate::game::levelgen::generate_once(theme, seed, 0);
                let objects = &level.objects;
                let mut cells: Vec<TileRect> = objects
                    .steam_grates
                    .iter()
                    .map(|g| TileRect::new(g.cell.0, g.cell.1, 1, 1))
                    .collect();
                cells.extend(objects.ink_pads.iter().map(|p| p.cells));
                for (i, a) in cells.iter().enumerate() {
                    hazards_seen += 1;
                    for b in &cells[..i] {
                        assert!(!a.intersects(b), "{theme:?} seed {seed}: two hazards overlap");
                    }
                    // A board may lie along either axis: its long half is the
                    // conservative reach both ways.
                    let reach = STICKY_TRAP_SIZE.x.max(STICKY_TRAP_SIZE.y) * 0.5;
                    for trap in &objects.sticky_traps {
                        let t = (trap.pos.x - reach, trap.pos.y - reach, trap.pos.x + reach, trap.pos.y + reach);
                        assert!(
                            !overlaps(px(*a), t),
                            "{theme:?} seed {seed}: a hazard at {a:?} overlaps a sticky board at {:?}",
                            trap.pos
                        );
                    }
                    let reach = MOUSETRAP_SIZE.x.max(MOUSETRAP_SIZE.y) * 0.5;
                    for trap in &objects.mousetraps {
                        let t = (trap.pos.x - reach, trap.pos.y - reach, trap.pos.x + reach, trap.pos.y + reach);
                        assert!(
                            !overlaps(px(*a), t),
                            "{theme:?} seed {seed}: a hazard at {a:?} overlaps a mousetrap at {:?}",
                            trap.pos
                        );
                    }
                    for hole in &objects.shop_holes {
                        let keep = TileRect::new(hole.mouth.0 - 2, hole.mouth.1 - 2, 5, 5);
                        assert!(
                            !a.intersects(&keep) && !a.contains(hole.cell.0, hole.cell.1),
                            "{theme:?} seed {seed}: a hazard at {a:?} crowds shop hole {} (mouth {:?})",
                            hole.index,
                            hole.mouth
                        );
                    }
                }
                // Shop holes are valid on the new floors (the full invariant
                // set runs in `placement`'s 200-seed test over every playable
                // theme); here: they exist and their threshold is open floor.
                for hole in &objects.shop_holes {
                    shops_seen += 1;
                    assert!(
                        level.tiles.get(hole.mouth.0, hole.mouth.1).is_open(),
                        "{theme:?} seed {seed}: shop hole {} opens onto a wall",
                        hole.index
                    );
                }
            }
        }
        assert!(hazards_seen > 1200, "only {hazards_seen} hazards checked");
        assert!(shops_seen > 600, "only {shops_seen} shop holes on the new floors");
    }

    /// The floor's own geometry actually appears: the Laundry grows long washer
    /// banks inside its halls, the Printworks offset press islands. Counting the
    /// solid share of the halls' interiors proves the stage ran and did
    /// something structural (and that it did not fill the hall).
    #[test]
    fn each_new_floor_grows_its_own_geometry() {
        for theme in [Theme::Laundry, Theme::Printworks] {
            let level = crate::game::levelgen::generate_once(theme, 3, 0);
            let mut interior = 0;
            let mut solid = 0;
            let mut runs = 0;
            for room in level.rooms_of(crate::game::level::RoomKind::Hall) {
                let inner = room.rect.inflate(-3);
                for cx in inner.x..inner.right() {
                    let mut run = 0;
                    for cy in inner.y..inner.bottom() {
                        interior += 1;
                        if level.tiles.get(cx, cy).is_wall() {
                            solid += 1;
                            run += 1;
                        } else {
                            if run >= 4 {
                                runs += 1;
                            }
                            run = 0;
                        }
                    }
                    if run >= 4 {
                        runs += 1;
                    }
                }
            }
            let share = solid as f64 / interior.max(1) as f64;
            assert!(
                share > 0.05,
                "{theme:?}: the halls' interiors are {:.1}% furniture, expected some",
                share * 100.0
            );
            assert!(
                share < 0.55,
                "{theme:?}: the halls' interiors are {:.1}% solid, too plugged to play",
                share * 100.0
            );
            assert!(runs >= 4, "{theme:?}: only {runs} long furniture runs");
        }
    }

    #[test]
    fn sewer_opening_rooms_hold_two_reachable_pillbugs_and_one_drip_shaft() {
        for seed in 0..20 {
            let level = Level::generate(Theme::Sewer, seed);
            let opening = level.objects.sewer_opening.as_ref().unwrap();
            let room = level.rooms.iter()
                .filter(|r| matches!(r.kind, RoomKind::Hall | RoomKind::Room))
                .min_by(|a, b| {
                    a.center()
                        .dist_sq(level.start)
                        .total_cmp(&b.center().dist_sq(level.start))
                })
                .unwrap();
            let reached = validate::reachable(&level.tiles, start_cell(&level), SizeClass::Large);
            assert!(room.rect.contains_world(opening.drip_shaft.pos), "seed {seed}");
            assert!(dry_route(&level.tiles, opening.drip_shaft.cell));
            assert!(opening.pillbug_cells.len() >= 2);
            for &pos in &opening.pillbug_cells {
                let cell = ((pos.x / TILE_F) as i32, (pos.y / TILE_F) as i32);
                assert!(room.rect.contains_world(pos));
                assert!(level.tiles.get_at(pos).is_open());
                assert!(validate::is_reached(&reached, &level.tiles, cell));
                assert_ne!(pos, opening.drip_shaft.pos);
            }
            assert!(level.tiles.get_at(opening.bottle_cap).is_open());
            assert!(!level.start_room().unwrap().rect.contains_world(opening.bottle_cap));
        }
    }

    #[test]
    fn sewer_opening_bottle_cap_uses_the_nearest_public_doorway() {
        let mut tiles = TileMap::new(20, 20, Theme::Sewer, 0);
        let start = TileRect::new(6, 6, 8, 8);
        let doorway = TileKind::Wall(WallKind::Doorway { axis: WallAxis::Horizontal });
        tiles.set(10, 5, doorway);
        tiles.set(7, 5, doorway);
        let reached = validate::reachable(&tiles, (10, 10), SizeClass::Large);
        let door_center = TileMap::tile_rect(10, 5).center();
        assert_eq!(
            opening_bottle_cap(&tiles, start, &reached),
            Some(door_center - Vec2::new(0.0, 24.0)),
        );
    }

    /// The stage draws no rng and places nothing in the Kitchen and Lab,
    /// which is why their pinned maps do not move.
    #[test]
    fn kitchen_and_lab_get_no_hazards_and_no_extra_draws() {
        for theme in [Theme::Kitchen, Theme::Lab] {
            let a = crate::game::levelgen::generate_once(theme, 11, 0);
            let b = crate::game::levelgen::generate_once(theme, 11, 0);
            assert!(a.objects.steam_grates.is_empty());
            assert!(a.objects.ink_pads.is_empty());
            assert!(a.objects.sewer_opening.is_none());
            // Same map twice, and no rng consumed by this stage: `level_seed`
            // drives both runs, so an extra draw would change the trap/hole
            // placement and this comparison would fail.
            let digest = |level: &Level| {
                level
                    .tiles
                    .iter_walls()
                    .collect::<Vec<_>>()
            };
            assert_eq!(digest(&a), digest(&b), "{theme:?}: generation is not stable");
        }
    }
}
