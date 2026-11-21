//! **The getaway ride — the flatbed trailer's own generator.**
//!
//! The owner's brief: *"a level that is on the bed of a long trailer, with the
//! road flowing past you can see through cracks between the wood planks."*
//!
//! Every other theme is a sprawl of rooms ([`super::layout`]); the trailer is
//! the opposite shape, so it owns its skeleton:
//!
//! * **The bed.** A long, narrow strip of deck: [`DECK_LEFT`]..[`DECK_RIGHT`]
//!   cells wide and [`DECK_ROWS`] cells tall, centred on the map's middle rows.
//!   Everything outside it is the road, the verge and the dark — solid rock to
//!   the simulation, painted as the rushing road by `art_trailer`.
//! * **The journey.** The rat drops onto the tailgate at the left (the start
//!   room) and walks right, past cargo, to the cab hold at the right end: a
//!   22 × 26-cell open arena behind a three-cell mouth.
//! * **Cargo makes the layout.** Strapped crates, pallet stacks, tarped
//!   machinery and pipe bundles are painted into the deck as solid cells, in
//!   alternating stacks that leave lanes, small rooms and choke points. A few
//!   one-cell-thick walls are recorded as [`Layout::partitions`], so the shared
//!   placement stage pierces them with rat-holes the big enemies cannot follow.
//! * **Hazards are part of the bed.** Broken planks and missing rail sections
//!   are [`Pit`]s, the slipstream pulls light things tailward in [`wind`]
//!   stretches, and crates walk along their straps on the trailer's bumps
//!   ([`SlidingCargo`]). The hazards live in [`Level::trailer`]; the systems in
//!   `ecs::systems::trailer` read them and `art_trailer` draws them.
//!
//! The rest of the pipeline is shared: [`super::maze::carve_mazes`] (about one
//! map in eight reserves a warren inside a crate stack near the tailgate),
//! [`super::placement::place_rat_holes`], [`super::placement::place_traps`] and
//! [`super::validate`]. Determinism is the same contract as every other theme:
//! one [`rand::rngs::StdRng`] in call order, no hash-map iteration, and the
//! same `(seed, attempt)` always yields the same bed.
//!
//! [`wind`]: crate::game::level::TrailerDeck::wind
//! [`Pit`]: crate::game::level::Pit
//! [`SlidingCargo`]: crate::game::level::SlidingCargo

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::{WallAxis, WallKind};
use crate::game::collision::{cardinal_of, SizeClass, TileKind, TileMap};
use crate::game::level::{
    BossArena, Level, Palette, Pit, PitKind, Prop, Room, RoomKind, SlidingCargo, Theme, TileRect,
    TrailerDeck,
};
use crate::game::nav::NavGrid;
use crate::math::Vec2;

use super::layout::{Layout, MAZE_ONE_IN};
use super::{maze, objects_from, placement};

// ---- The bed ---------------------------------------------------------------

/// First walkable column of the bed (cells). Columns to its west are off the
/// tail.
pub const DECK_LEFT: i32 = 6;
/// One past the last column the deck reaches (cells).
pub const DECK_RIGHT: i32 = 194;
/// First walkable row of the bed (cells).
pub const DECK_TOP: i32 = 67;
/// Walkable rows of the bed (cells): a long, narrow strip, not a hall.
pub const DECK_ROWS: i32 = 26;
/// The north side rail (the row immediately above the deck).
pub const RAIL_TOP: i32 = DECK_TOP - 1;
/// The south side rail (the row immediately below the deck).
pub const RAIL_BOTTOM: i32 = DECK_TOP + DECK_ROWS;

/// Width of the tailgate hold the rat drops onto.
const TAILGATE_W: i32 = 14;
/// First column cargo may occupy (the tailgate hold and its lane stay clear).
const FIRST_CARGO_X: i32 = DECK_LEFT + TAILGATE_W + 4;
/// The cab hold's width in cells (its area clears the 400-cell arena contract).
const CAB_W: i32 = 22;
/// The cab hold's west wall column; the mouth is punched through it.
const CAB_RING_X: i32 = DECK_RIGHT - 4 - CAB_W - 1;
/// The cab hold's first open column.
const CAB_X: i32 = CAB_RING_X + 1;
/// Rows the three-cell cab mouth spans, centred on the bed.
const CAB_MOUTH_ROWS: i32 = 3;

/// The tailgate: where the rat lands, `(column, row)` of the cell centre.
pub const TAILGATE_CELL: (i32, i32) = (DECK_LEFT + 5, DECK_TOP + DECK_ROWS / 2);

const SOLID: TileKind = TileKind::Wall(WallKind::Solid);

/// **The trailer pipeline.** Build the bed, furnish it, then run the shared
/// maze, rat-hole and trap stages over the result.
pub fn generate(mut tiles: TileMap, rng: &mut StdRng, seed: u64) -> Level {
    let theme = Theme::Trailer;
    let (design, mut deck) = build_layout(&mut tiles, rng);
    let props = furnish(&mut tiles, &design, &deck, rng);
    let mazes = maze::carve_mazes(&mut tiles, &design, rng, theme);
    let holes = placement::place_rat_holes(&mut tiles, &design, rng);
    let mut traps = placement::place_traps(&tiles, &design, &mazes, rng);

    // A pit under a rat-hole or a trap's pad would stack two hazards on one
    // cell; drop any that ended up there (the maze warren is carved after the
    // pits, so it can also have covered one).
    deck.pits.retain(|pit| {
        pit.rect
            .cells()
            .all(|(cx, cy)| matches!(tiles.get(cx, cy), TileKind::Open))
    });

    // The mouse-hole shops (no rng, tiles untouched). On the flatbed a hole
    // is cut into cargo standing on the bed — never the rock outside the
    // rails, which is open air — and its threshold must be deck the rat can
    // stand on and walk to: not a hole in the planks nor beside one, not in a
    // sliding crate's lane, and reachable from the tailgate without crossing
    // a hole. A bed with fewer than two such faces carries fewer shops.
    let dry = dry_deck(&tiles, &deck, validate_cell(design.start));
    let cols = tiles.cols();
    let shop_holes = placement::place_shop_holes_where(&tiles, &design, &mazes, &|cell, mouth| {
        deck.deck.contains(cell.0, cell.1)
            && dry[(mouth.1 * cols + mouth.0) as usize]
            && shop_threshold_clear(&deck, mouth)
    });

    // A 40 × 24 snap trap never crowds a shop threshold on the deck either.
    placement::drop_traps_crowding_shops(&mut traps, &shop_holes);

    tiles.rebuild_masks();
    let objects = objects_from(
        &tiles,
        &mazes,
        &holes,
        &traps,
        &shop_holes,
        // The flatbed has its own hazards (pits, cargo, wind): no grates, no ink.
        &super::hazards::HazardPlacement::default(),
        seed,
    );
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
        mazes,
        objects,
        boss: design.boss.clone(),
        trailer: deck,
    }
}

/// The cell containing world position `p`.
fn validate_cell(p: Vec2) -> (i32, i32) {
    (
        (p.x / crate::game::collision::TILE_F).floor() as i32,
        (p.y / crate::game::collision::TILE_F).floor() as i32,
    )
}

/// Open deck cells reachable from `start` on foot **without stepping on a
/// hole**: a 4-connected flood over open cells that are in no [`Pit`]. Row
/// major, `cols` wide.
fn dry_deck(tiles: &TileMap, deck: &TrailerDeck, start: (i32, i32)) -> Vec<bool> {
    let (cols, rows) = (tiles.cols(), tiles.rows());
    let in_pit = |cx: i32, cy: i32| deck.pits.iter().any(|pit| pit.rect.contains(cx, cy));
    let walkable = |cx: i32, cy: i32| {
        cx >= 0
            && cy >= 0
            && cx < cols
            && cy < rows
            && matches!(tiles.get(cx, cy), TileKind::Open)
            && !in_pit(cx, cy)
    };
    let mut seen = vec![false; (cols * rows) as usize];
    let mut stack = Vec::new();
    if walkable(start.0, start.1) {
        seen[(start.1 * cols + start.0) as usize] = true;
        stack.push(start);
    }
    while let Some((cx, cy)) = stack.pop() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx + dx, cy + dy);
            if walkable(nx, ny) && !seen[(ny * cols + nx) as usize] {
                seen[(ny * cols + nx) as usize] = true;
                stack.push((nx, ny));
            }
        }
    }
    seen
}

/// Is a shop threshold at `mouth` clear of the bed's hazards? No hole in the
/// planks on it or on any of its eight neighbours, and no sliding crate's
/// lane (its box swept from rest to the end of its straps) over the cell.
pub(super) fn shop_threshold_clear(deck: &TrailerDeck, mouth: (i32, i32)) -> bool {
    let near = TileRect::new(mouth.0 - 1, mouth.1 - 1, 3, 3);
    if deck.pits.iter().any(|pit| pit.rect.intersects(&near)) {
        return false;
    }
    let t = crate::game::collision::TILE_F;
    let (x0, y0) = (mouth.0 as f32 * t, mouth.1 as f32 * t);
    let (x1, y1) = (x0 + t, y0 + t);
    !deck.slides.iter().any(|slide| {
        let end = slide.rest + slide.dir * slide.travel;
        let (lo_x, hi_x) = (slide.rest.x.min(end.x) - slide.half.x, slide.rest.x.max(end.x) + slide.half.x);
        let (lo_y, hi_y) = (slide.rest.y.min(end.y) - slide.half.y, slide.rest.y.max(end.y) + slide.half.y);
        x0 < hi_x && x1 > lo_x && y0 < hi_y && y1 > lo_y
    })
}

/// Carve every cell of `rect` open (out-of-bounds writes are ignored).
fn carve(tiles: &mut TileMap, rect: TileRect) {
    tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, TileKind::Open);
}

/// **Stage A for the flatbed.** Paint the bed and its cargo into `tiles` and
/// export the regions, the boss furniture and the bed's hazards.
fn build_layout(tiles: &mut TileMap, rng: &mut StdRng) -> (Layout, TrailerDeck) {
    let cols = tiles.cols();
    let rows = tiles.rows();
    // Rock first, deck second: nothing can leak through a wall the layout did
    // not make itself.
    tiles.fill_cells(0, 0, cols, rows, SOLID);

    let bed = TileRect::new(DECK_LEFT, DECK_TOP, DECK_RIGHT - DECK_LEFT, DECK_ROWS);
    carve(tiles, bed);

    // ---- the tailgate and the cab ---------------------------------------
    let start = TileMap::tile_rect(TAILGATE_CELL.0, TAILGATE_CELL.1).center();
    let start_room = TileRect::new(DECK_LEFT, DECK_TOP, TAILGATE_W, DECK_ROWS);
    let cab = TileRect::new(CAB_X, DECK_TOP, CAB_W, DECK_ROWS);
    // The cab hold is the last room; its west wall keeps a three-cell mouth
    // (48 px: the boss's 28 px body walks through it).
    let mouth_y = DECK_TOP + (DECK_ROWS - CAB_MOUTH_ROWS) / 2;
    tiles.fill_cells(CAB_RING_X, DECK_TOP, 1, DECK_ROWS, SOLID);
    tiles.fill_cells(
        CAB_RING_X,
        mouth_y,
        1,
        CAB_MOUTH_ROWS,
        TileKind::Wall(WallKind::Doorway {
            axis: WallAxis::Vertical,
        }),
    );
    // The cab's steel bulkhead: everything east of the hold is the tractor.
    tiles.fill_cells(cab.right(), DECK_TOP, DECK_RIGHT - cab.right(), DECK_ROWS, SOLID);

    let centre = cab.center_world();
    let centre_cell = (
        (centre.x / crate::game::collision::TILE_F) as i32,
        (centre.y / crate::game::collision::TILE_F) as i32,
    );
    let (fw, fh) = super::layout::boss_floor_cells(Theme::Trailer);
    let floor = TileRect::new(centre_cell.0 - fw / 2, centre_cell.1 - fh / 2, fw, fh);
    let boss = BossArena {
        floor,
        mouths: (0..CAB_MOUTH_ROWS)
            .map(|k| (CAB_RING_X, mouth_y + k))
            .collect(),
        ..BossArena::default()
    };

    let mut rooms = vec![
        Room::new(start_room, RoomKind::Start),
        Room::new(cab, RoomKind::Boss),
    ];

    // ---- the cargo run ----------------------------------------------------
    let run_end = CAB_RING_X - 3;
    let mut cargo: Vec<TileRect> = Vec::new();
    let mut partitions: Vec<TileRect> = Vec::new();
    let mut maze_slots: Vec<TileRect> = Vec::new();

    // About one map in eight hides a warren inside the first crate stack
    // cluster, within the maze contract's near-start radius (32 cells).
    let slot = if rng.random_range(0..MAZE_ONE_IN) == 0 {
        let slot = TileRect::new(DECK_LEFT + TAILGATE_W + 6, DECK_TOP + 2, 16, 12);
        // The walkway around the warren is open deck, so the breach and the
        // one-way door both open onto public floor.
        carve(tiles, slot.inflate(2));
        maze_slots.push(slot);
        Some(slot)
    } else {
        None
    };
    let clear_of_slot = |rect: TileRect| {
        slot.is_none_or(|slot| !rect.inflate(1).intersects(&slot.inflate(2)))
    };

    // The two halls the horde gathers in: the run's two halves, split by the
    // natural tide of the cargo. They overlap stacks on purpose (a room is a
    // place, not a promise of empty floor).
    rooms.push(Room::new(
        TileRect::new(FIRST_CARGO_X, DECK_TOP, 58, DECK_ROWS),
        RoomKind::Hall,
    ));
    rooms.push(Room::new(
        TileRect::new(FIRST_CARGO_X + 66, DECK_TOP, run_end - FIRST_CARGO_X - 66, DECK_ROWS),
        RoomKind::Hall,
    ));

    // Alternating stacks: the bed weaves. A stack is between 4 and 9 cells
    // long and 7 and 15 tall, so the lane on its far side is never narrower
    // than 11 cells — the trailer is a corridor, but a wide one.
    let mut x = FIRST_CARGO_X;
    let mut north = rng.random_range(0..2) == 0;
    while x < run_end - 6 {
        let w = rng.random_range(4..=9);
        let h = rng.random_range(7..=15);
        let y = if north { DECK_TOP } else { DECK_TOP + DECK_ROWS - h };
        let rect = TileRect::new(x, y, w, h);
        if rect.right() < run_end && clear_of_slot(rect) {
            tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, SOLID);
            cargo.push(rect);
            north = !north;
        }
        x += w + rng.random_range(4..=8);
    }

    // Thin cargo walls (a single strapped row of crates): chokepoints, and the
    // one-cell walls the rat-holes are pierced through.
    for _ in 0..rng.random_range(2..=4) {
        let px = rng.random_range(FIRST_CARGO_X + 18..run_end - 14);
        let len = rng.random_range(9..=16);
        let from_north = rng.random_range(0..2) == 0;
        let y = if from_north {
            DECK_TOP
        } else {
            DECK_TOP + DECK_ROWS - len
        };
        let rect = TileRect::new(px, y, 1, len);
        // Keep a lane on the far side of every wall (and clear of the maze).
        if len <= DECK_ROWS - 5
            && clear_of_slot(rect)
            && !cargo.iter().any(|c| c.inflate(1).intersects(&rect))
        {
            tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, SOLID);
            partitions.push(rect);
        }
    }

    // A barrier is only fair if the horde can go round it: the shared
    // placement stage pierces the ones it finds with rat-holes, which the big
    // enemies cannot use, so the horde must already have a way past. Walk the
    // furniture back (newest first, no rng) until both classes reach the cab.
    let mut guard = 0;
    while !connected(tiles, &Layout {
        rooms: Vec::new(),
        start,
        start_room,
        boss_arena: cab,
        boss: BossArena::default(),
        maze_slots: Vec::new(),
        partitions: Vec::new(),
    }) && guard < 40
    {
        guard += 1;
        if let Some(rect) = partitions.pop() {
            carve(tiles, rect);
        } else if let Some(rect) = cargo.pop() {
            carve(tiles, rect);
        } else {
            break;
        }
    }

    // ---- hazards ----------------------------------------------------------
    let mut deck = TrailerDeck {
        deck: bed,
        tailgate: start,
        ..TrailerDeck::default()
    };

    // Broken planks: one-cell holes in the middle of a lane, never in the
    // tailgate's first corridor, the cab hold or on a maze walkway.
    let mut pit_cells: Vec<(i32, i32)> = Vec::new();
    for cy in DECK_TOP + 2..DECK_TOP + DECK_ROWS - 2 {
        for cx in FIRST_CARGO_X + 10..run_end {
            let near_maze = slot.is_some_and(|slot| slot.inflate(1).contains(cx, cy));
            if near_maze || !matches!(tiles.get(cx, cy), TileKind::Open) {
                continue;
            }
            // Away from every cargo face by two cells, so the hole is a hole
            // in the middle of the deck and not a chipped crate corner.
            let crowded = (-2..=2).any(|dy| {
                (-2..=2).any(|dx| tiles.is_full_solid(cx + dx, cy + dy))
            });
            if crowded {
                continue;
            }
            pit_cells.push((cx, cy));
        }
    }
    if !pit_cells.is_empty() {
        let holes = rng.random_range(2..=4).min(pit_cells.len());
        for _ in 0..holes {
            let pick = rng.random_range(0..pit_cells.len());
            let cell = pit_cells.remove(pick);
            // A two-cell hole reads better than a single punched plank.
            let wide = rng.random_range(0..2) == 0
                && pit_cells.iter().any(|&(x, y)| x == cell.0 + 1 && y == cell.1);
            let (w, h) = if wide { (2, 1) } else { (1, 1) };
            let rect = TileRect::new(cell.0, cell.1, w, h);
            deck.pits.push(Pit::new(rect, PitKind::BrokenPlank));
            pit_cells.retain(|&(x, y)| !rect.contains(x, y));
        }
    }
    // Missing rail sections: a two-cell gap on one side rail, at least a few
    // cells in from the tailgate and short of the cab.
    for _ in 0..rng.random_range(1..=2) {
        let gx = rng.random_range(FIRST_CARGO_X + 12..run_end - 12);
        let north_side = rng.random_range(0..2) == 0;
        let row = if north_side { RAIL_TOP } else { RAIL_BOTTOM };
        let rect = TileRect::new(gx, row, 2, 1);
        if rect.cells().any(|(cx, cy)| !tiles.is_full_solid(cx, cy)) {
            continue;
        }
        carve(tiles, rect);
        deck.pits.push(Pit::new(rect, PitKind::RailGap));
    }

    // Sliding cargo: 1–3 crates that walk along their straps on every bump.
    // The crate rests at a stack's long edge and shoves out into the lane; the
    // candidates are collected first (both directions, then a shorter travel),
    // so a map always gets at least one.
    let mut slide_slots: Vec<(Vec2, Vec2, f32)> = Vec::new(); // (rest, dir, travel)
    for stack in &cargo {
        let rest_y = (stack.y as f32 + stack.h as f32 * 0.5) * 16.0;
        let mut found = false;
        for (toward_cab, travel) in [
            (true, 34.0f32),
            (false, 34.0),
            (true, 22.0),
            (false, 22.0),
        ] {
            let dir = if toward_cab { 1.0 } else { -1.0 };
            let rest_x = if toward_cab {
                (stack.right() as f32 + 1.0) * 16.0
            } else {
                (stack.x as f32 - 1.0) * 16.0
            };
            // The lane the crate shoves into must be open deck for its travel.
            let cells_ahead = (travel / 16.0_f32).ceil() as i32 + 1;
            let lane_open = (0..cells_ahead).all(|k| {
                let cx = if toward_cab {
                    stack.right() + k
                } else {
                    stack.x - 1 - k
                };
                (0..2).all(|dy| {
                    let cy = stack.y + stack.h / 2 + dy - (stack.h % 2) / 2;
                    matches!(tiles.get(cx, cy), TileKind::Open)
                })
            });
            if lane_open {
                slide_slots.push((Vec2::new(rest_x, rest_y), Vec2::new(dir, 0.0), travel));
                found = true;
                break;
            }
        }
        let _ = found;
    }
    let slide_count = rng.random_range(1..=3).min(slide_slots.len());
    for _ in 0..slide_count {
        let pick = rng.random_range(0..slide_slots.len());
        let (rest, dir, travel) = slide_slots.remove(pick);
        deck.slides.push(SlidingCargo {
            rest,
            dir,
            travel,
            phase: rng.random_range(0.0..7.5),
            half: Vec2::new(10.0, 9.0),
            warn: 1.2,
        });
    }

    // Exposed stretches where the slipstream drags light things tailward: the
    // open lanes, away from the maze walkway. Candidate rects are collected
    // first (a mostly-open stretch counts), so the draw can never leave a map
    // without one.
    let mut wind_spots: Vec<TileRect> = Vec::new();
    for wy in [DECK_TOP + 2, DECK_TOP + DECK_ROWS - 9] {
        let mut wx = FIRST_CARGO_X + 12;
        while wx + 14 < run_end {
            let rect = TileRect::new(wx, wy, 14, 7);
            if clear_of_slot(rect) {
                let open = rect
                    .cells()
                    .filter(|&(cx, cy)| matches!(tiles.get(cx, cy), TileKind::Open))
                    .count();
                // Mostly open lane: the streaks are drawn over the deck, and
                // the drag only applies on open cells.
                if open * 10 >= rect.area() as usize * 6 {
                    wind_spots.push(rect);
                }
            }
            wx += 4;
        }
    }
    let wind_count = rng.random_range(1..=2).min(wind_spots.len());
    for _ in 0..wind_count {
        let pick = rng.random_range(0..wind_spots.len());
        deck.wind.push(wind_spots.remove(pick));
    }

    let design = Layout {
        rooms,
        start,
        start_room,
        boss_arena: cab,
        boss,
        maze_slots,
        partitions,
    };
    (design, deck)
}

/// **Stage A2 for the flatbed.** Dress the cargo with props: the stacks keep
/// their cells (already solid), and a couple of small loose items sit in the
/// lanes as clutter.
fn furnish(
    tiles: &mut TileMap,
    layout: &Layout,
    deck: &TrailerDeck,
    rng: &mut StdRng,
) -> Vec<Prop> {
    let mut props: Vec<Prop> = Vec::new();
    let kinds = crate::game::level::theme_props(Theme::Trailer);
    // A prop on every cargo stack's leading corner, where a stack's top face
    // reads best, cycling the theme's cargo kinds.
    for (i, stack) in cargo_rects(tiles, layout).iter().enumerate() {
        let kind = kinds[i % kinds.len()];
        let (w, h) = kind.cells();
        if w > stack.w || h > stack.h {
            continue;
        }
        let rect = TileRect::new(stack.x, stack.y, w, h);
        if !rect.cells().all(|(cx, cy)| tiles.is_full_solid(cx, cy)) {
            continue;
        }
        props.push(Prop {
            kind,
            rect: rect.to_world(),
        });
    }

    // Two loose items in a lane: a toolbox and a tyre, never within six cells
    // of the tailgate, never on a hole, a wind stretch or a maze walkway.
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for cy in DECK_TOP + 2..DECK_TOP + DECK_ROWS - 2 {
        for cx in FIRST_CARGO_X + 8..CAB_RING_X - 6 {
            if !matches!(tiles.get(cx, cy), TileKind::Open) {
                continue;
            }
            if deck.pits.iter().any(|p| p.rect.inflate(1).contains(cx, cy))
                || layout.maze_slots.iter().any(|s| s.inflate(2).contains(cx, cy))
            {
                continue;
            }
            // Keep the prop out of the middle of a lane: tucked against cargo
            // it reads as stowed, in the open it reads as dropped.
            let at_side = [(0, -1), (0, 1), (-1, 0), (1, 0)]
                .iter()
                .filter(|&&(dx, dy)| tiles.is_full_solid(cx + dx, cy + dy))
                .count();
            if at_side == 0 {
                continue;
            }
            cells.push((cx, cy));
        }
    }
    for (n, kind) in [crate::game::level::PropKind::Toolbox, crate::game::level::PropKind::Tyre]
        .into_iter()
        .enumerate()
    {
        if cells.is_empty() {
            break;
        }
        let (w, h) = kind.cells();
        let mut placed = false;
        for _ in 0..8 {
            let pick = rng.random_range(0..cells.len());
            let (cx, cy) = cells[pick];
            let rect = TileRect::new(cx, cy, w, h);
            if !rect.cells().all(|(qx, qy)| matches!(tiles.get(qx, qy), TileKind::Open)) {
                continue;
            }
            tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, SOLID);
            // The lane must still be walkable end to end.
            if !connected(tiles, layout) {
                carve(tiles, rect);
                continue;
            }
            props.push(Prop {
                kind,
                rect: rect.to_world(),
            });
            cells.retain(|&(x, y)| !rect.inflate(2).contains(x, y));
            placed = true;
            break;
        }
        let _ = (n, placed);
    }
    props
}

/// The cargo stacks: every solid block inside the run that the layout made.
///
/// Re-derived from the grid (a stack is a solid rectangle at least 4 × 7
/// cells) rather than kept as extra state, so the props cannot drift from the
/// bed that was actually painted. Cargo walls are always grown from the north
/// or south rail, so a stack's leading corner is its `(x, y)` cell.
fn cargo_rects(tiles: &TileMap, layout: &Layout) -> Vec<TileRect> {
    let mut rects = Vec::new();
    for room in &layout.rooms {
        if !matches!(room.kind, RoomKind::Hall) {
            continue;
        }
        let mut cy = room.rect.y;
        while cy < room.rect.bottom() {
            let mut cx = room.rect.x;
            while cx < room.rect.right() {
                if !tiles.is_full_solid(cx, cy) {
                    cx += 1;
                    continue;
                }
                // Measure the solid run east and south from this corner.
                let mut w = 1;
                while cx + w < room.rect.right() && tiles.is_full_solid(cx + w, cy) {
                    w += 1;
                }
                let mut h = 1;
                while cy + h < room.rect.bottom()
                    && (cx..cx + w).all(|x| tiles.is_full_solid(x, cy + h))
                {
                    h += 1;
                }
                if w >= 4 && h >= 4 {
                    rects.push(TileRect::new(cx, cy, w, h));
                }
                cx += w;
            }
            cy += 1;
        }
    }
    rects
}

/// Is the bed still one walkable piece for both size classes, start to cab?
///
/// A deck-local cell flood (the whole rest of the map is bedrock, so walking
/// it would only cost time): the trailer's own invariant test uses the same
/// walk, and stage A's furniture must never cut the horde's lane. The rat can
/// use a rat-hole, the horde cannot, so the answer is required for **both**
/// classes.
fn connected(tiles: &TileMap, layout: &Layout) -> bool {
    let deck = TileRect::new(DECK_LEFT, DECK_TOP, DECK_RIGHT - DECK_LEFT, DECK_ROWS);
    let start = (
        (layout.start.x / crate::game::collision::TILE_F) as i32,
        (layout.start.y / crate::game::collision::TILE_F) as i32,
    );
    let target = (
        (layout.boss_arena.center_world().x / crate::game::collision::TILE_F) as i32,
        (layout.boss_arena.center_world().y / crate::game::collision::TILE_F) as i32,
    );
    let mut seen = vec![false; (deck.w * deck.h) as usize];
    let at = |cx: i32, cy: i32| ((cy - deck.y) * deck.w + (cx - deck.x)) as usize;
    for class in SizeClass::ALL {
        seen.fill(false);
        let mut queue = std::collections::VecDeque::new();
        if !tiles.passable(start.0, start.1, class) {
            return false;
        }
        seen[at(start.0, start.1)] = true;
        queue.push_back(start);
        while let Some((cx, cy)) = queue.pop_front() {
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if !deck.contains(nx, ny) || !tiles.travel_ok(nx, ny, class, cardinal_of(dx, dy).unwrap()) {
                    continue;
                }
                let i = at(nx, ny);
                if !seen[i] {
                    seen[i] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
        if !seen[at(target.0, target.1)] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::{level_rng, MAP_COLS, MAP_ROWS};

    fn level_for(seed: u64) -> Level {
        let mut rng = level_rng(Theme::Trailer, seed, 0);
        let tiles = crate::game::collision::TileMap::new(
            MAP_COLS,
            MAP_ROWS,
            Theme::Trailer,
            (seed as u32).wrapping_mul(0x9e37_79b9),
        );
        generate(tiles, &mut rng, seed)
    }

    #[test]
    fn the_bed_is_a_long_narrow_strip() {
        let level = level_for(3);
        let deck = &level.trailer.deck;
        assert_eq!(*deck, TileRect::new(DECK_LEFT, DECK_TOP, 188, DECK_ROWS));
        // The strip is long and narrow: at least six times wider than tall.
        assert!(deck.w >= 6 * deck.h, "the bed is not a strip: {deck:?}");
        // The tailgate is on the left, the cab on the right.
        assert!(level.start.x < deck.center_world().x);
        let cab = level.boss_room().expect("cab hold");
        assert!(cab.rect.center_world().x > deck.center_world().x);
        assert_eq!(level.start, TileMap::tile_rect(TAILGATE_CELL.0, TAILGATE_CELL.1).center());
    }

    #[test]
    fn the_deck_is_walkable_and_the_road_is_not() {
        let level = level_for(9);
        // Every cell just beyond the rails is off the trailer: solid rock.
        for cx in DECK_LEFT..DECK_RIGHT {
            assert!(level.tiles.get(cx, RAIL_TOP - 1).is_wall(), "road is walkable at {cx}");
            assert!(level.tiles.get(cx, RAIL_BOTTOM + 1).is_wall());
        }
        // The start cell is clear floor with its 3 x 3.
        let (sx, sy) = TAILGATE_CELL;
        for dy in -1..=1 {
            for dx in -1..=1 {
                assert!(
                    matches!(level.tiles.get(sx + dx, sy + dy), TileKind::Open),
                    "the drop zone is blocked at ({}, {})",
                    sx + dx,
                    sy + dy
                );
            }
        }
        // The cab hold is open behind its mouth.
        let cab = level.boss_room().expect("cab");
        for (cx, cy) in cab.rect.cells() {
            assert!(
                matches!(level.tiles.get(cx, cy), TileKind::Open),
                "the cab hold is blocked at ({cx}, {cy})"
            );
        }
        // ... and the mouth is a real opening.
        let (mx, my) = level.boss.mouths[1];
        assert!(level.tiles.passable(mx, my, SizeClass::Large));
    }

    #[test]
    fn the_generator_validates_over_200_seeds() {
        for seed in 0..200u64 {
            let level = level_for(seed);
            if let Err(errors) = super::super::validate::validate(&level) {
                panic!("seed {seed}: {errors:#?}");
            }
        }
    }

    #[test]
    fn generation_is_deterministic_and_seed_sensitive() {
        let a = level_for(7);
        let b = level_for(7);
        assert_eq!(a.tiles.wall_count(), b.tiles.wall_count());
        assert_eq!(
            a.tiles.iter_walls().collect::<Vec<_>>(),
            b.tiles.iter_walls().collect::<Vec<_>>()
        );
        assert_eq!(a.trailer.pits.len(), b.trailer.pits.len());
        assert_eq!(a.trailer.slides.len(), b.trailer.slides.len());
        assert_eq!(a.start, b.start);
        let c = level_for(8);
        assert_ne!(
            a.tiles.iter_walls().collect::<Vec<_>>(),
            c.tiles.iter_walls().collect::<Vec<_>>(),
            "two seeds made the same bed"
        );
    }

    #[test]
    fn every_hazard_is_off_the_tailgate_and_off_the_cab() {
        let cab_x = CAB_X;
        for seed in 0..60u64 {
            let level = level_for(seed);
            let deck = &level.trailer;
            for pit in &deck.pits {
                assert!(
                    pit.rect.x >= FIRST_CARGO_X + 8,
                    "seed {seed}: a pit sits in the first corridor"
                );
                assert!(
                    pit.rect.right() <= cab_x,
                    "seed {seed}: a pit sits in the cab hold"
                );
                for (cx, cy) in pit.rect.cells() {
                    assert!(
                        matches!(level.tiles.get(cx, cy), TileKind::Open),
                        "seed {seed}: pit cell ({cx}, {cy}) is not open floor"
                    );
                }
            }
            for slide in &deck.slides {
                assert!(slide.phase >= 0.0 && slide.phase < crate::game::level::SLIDE_PERIOD);
                assert!(slide.warn > 0.0, "a slide without a tell");
            }
        }
    }

    /// Every map carries the floor's own furniture: holes to fall through,
    /// sliding cargo and at least one slipstream stretch. A level where the
    /// hazards never appear is a bare rectangle with planks painted on it.
    #[test]
    fn every_map_has_its_hazards() {
        let (mut min_pits, mut max_pits) = (i32::MAX, 0);
        let (mut min_slides, mut max_slides) = (i32::MAX, 0);
        let mut wind_maps = 0;
        for seed in 0..120u64 {
            let level = level_for(seed);
            let deck = &level.trailer;
            let pits = deck.pits.len() as i32;
            let slides = deck.slides.len() as i32;
            assert!(pits >= 1, "seed {seed}: no holes at all");
            assert!(slides >= 1, "seed {seed}: no sliding cargo");
            assert!(!deck.wind.is_empty(), "seed {seed}: no slipstream stretch");
            // Holes are never on top of each other (a pit cell is spent once).
            for (i, a) in deck.pits.iter().enumerate() {
                for b in &deck.pits[i + 1..] {
                    assert!(!a.rect.intersects(&b.rect), "seed {seed}: holes overlap");
                }
            }
            min_pits = min_pits.min(pits);
            max_pits = max_pits.max(pits);
            min_slides = min_slides.min(slides);
            max_slides = max_slides.max(slides);
            wind_maps += 1;
        }
        assert_eq!(wind_maps, 120);
        assert!(max_pits <= 6, "up to {max_pits} holes on one map is a sieve");
        println!("flatbed hazards over 120 seeds: pits {min_pits}..{max_pits}, slides {min_slides}..{max_slides}");
    }

    #[test]
    fn a_safe_route_round_every_pit_survives() {
        for seed in 0..40u64 {
            let level = level_for(seed);
            // Flood the deck over **non-pit** cells only: every open deck cell
            // that is not a hole must still reach the tailgate, so a hole can
            // never cut the bed in two.
            let deck = level.trailer.deck;
            let mut seen = vec![false; (MAP_COLS * MAP_ROWS) as usize];
            let mut queue = std::collections::VecDeque::new();
            let start = TAILGATE_CELL;
            seen[(start.1 * MAP_COLS + start.0) as usize] = true;
            queue.push_back(start);
            while let Some((cx, cy)) = queue.pop_front() {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if !deck.contains(nx, ny) {
                        continue;
                    }
                    if !level.tiles.passable(nx, ny, crate::game::collision::SizeClass::Small) {
                        continue;
                    }
                    if level.trailer.pit_at(TileMap::tile_rect(nx, ny).center()).is_some() {
                        continue;
                    }
                    let i = (ny * MAP_COLS + nx) as usize;
                    if !seen[i] {
                        seen[i] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
            for (cx, cy) in deck.cells() {
                if !level.tiles.passable(cx, cy, crate::game::collision::SizeClass::Small) {
                    continue;
                }
                if level.trailer.pit_at(TileMap::tile_rect(cx, cy).center()).is_some() {
                    continue;
                }
                assert!(
                    seen[(cy * MAP_COLS + cx) as usize],
                    "seed {seed}: hole cut the deck at ({cx}, {cy})"
                );
            }
        }
    }

    #[test]
    fn the_maze_lands_near_the_tailgate_when_it_appears() {
        let mut seen = 0;
        for seed in 0..200u64 {
            let level = level_for(seed);
            assert!(level.mazes.len() <= 1);
            for section in &level.mazes {
                seen += 1;
                let gap = (section.entry.0 - TAILGATE_CELL.0)
                    .abs()
                    .max((section.entry.1 - TAILGATE_CELL.1).abs());
                assert!(
                    gap <= maze::ENTRY_NEAR_CELLS,
                    "seed {seed}: the maze breach is {gap} cells from the tailgate"
                );
            }
        }
        assert!(seen > 0, "200 seeds never grew a maze");
        assert!(
            seen < 200,
            "every map grew a maze; the one-in-{} rate is gone",
            MAZE_ONE_IN
        );
    }
}
