//! **Stage A — the map's skeleton.** One open world: a great hall around the
//! spawn, a sprawl of plazas and medium rooms that merge into each other
//! through wide openings, the boss arena on the far side, the maze slot, and
//! the partitions, stubs and pillars that give the floor its cover.
//!
//! Owner: *generator worker A*. The signatures, the [`Layout`] it returns and
//! the invariants below are final.
//!
//! ## Invariants `build_layout` must keep
//!
//! - The start area is centred on the map: `start_room.center_world()` is the
//!   rat's spawn point and equals `MAP_SIZE * 0.5`, and the 3 × 3 cells around
//!   it stay open.
//! - Exactly one [`RoomKind::Boss`] room exists (a large, wide-mouthed open
//!   arena for the map's boss) and the large size class can reach its centre.
//! - [`Layout::maze_slots`] is at most one open, wall-free cell rectangle
//!   reserved for stage B (about one map in eight), with a one-cell ring of
//!   open floor around it and no solid, prop or partition inside that ring.
//!   Stage B carves every slot it is given, so a reservation is never left as
//!   a bare rectangle.
//! - Every wall is painted into the [`TileMap`] as a
//!   [`super::super::art_walls::WallKind`]; nothing returns a `Vec<Rectf>`.
//!   Openings between areas are runs of `Doorway` cells, never holes in a rect
//!   list.
//! - Nothing is drawn from a hash map and the only randomness is `rng`, in
//!   call order, so the same seed always yields the same map.
//!
//! ## How the map is built — the open world
//!
//! The whole lattice starts as **bedrock** (`WallKind::Solid`) and every area
//! is carved out of it. That order — rock first, floor second — means a layout
//! can never leak through a wall it did not make itself, and the border is
//! solid by construction.
//!
//! The owner's playtest was blunt: *the rooms are too small, make a more open
//! world, more connected areas, not just areas joined by hallways.* So the
//! skeleton is no longer a tree of rooms hung off 1–2-cell corridors:
//!
//! 1. the **start area**, a great hall of 40–46 × 28–34 cells centred on the
//!    map (see [`Level::start`]), is the first area;
//! 2. the **maze slot** is reserved inside an area near the spawn (about one
//!    map in eight) while the floors around it are still empty;
//! 3. the **sprawl** grows by attaching one area at a time to an existing one:
//!    a plaza (34–50 × 24–38 cells) or a medium room (16–30 × 12–24), either
//!    **merged** into the parent (they overlap by 3–8 cells) or separated by
//!    1–3 cells of bedrock and joined by a **wide opening** of 4–10 cells. It
//!    keeps growing until about half the grid is walkable, so the map reads as
//!    one connected place instead of a corridor network;
//! 4. extra **loop links**: areas that ended up 1–3 cells apart are joined by
//!    more wide openings, so the area graph carries several independent cycles
//!    and the player can always circle away from a horde;
//! 5. the **boss arena** (34–40 × 28–34) is attached to the area farthest from
//!    the spawn through a 6–10-cell mouth;
//! 6. **cover and texture** ([`scatter_cover`]): notched corners, free-standing
//!    partitions of 10–26 cells (long ones are recorded in
//!    [`Layout::partitions`] so stage B can pierce them with the rat-holes that
//!    make ducking through one worth it), wall stubs and pillars;
//! 7. a final flood fill that stitches any orphaned region back onto the rest
//!    of the map, so the connectivity promise is proven, not assumed;
//! 8. the arena's fight furniture ([`furnish_arena`]) — the clear central
//!    floor, the Lab's two bumpers, its charging dock and the Sewer's escape
//!    rat-hole — exported as [`crate::game::level::BossArena`] so the boss
//!    framework, the art and the validator all read the same cells.
//!
//! [`Level::start`]: crate::game::level::Level::start

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::{WallAxis, WallKind};
use crate::game::collision::{TileKind, TileMap, TILE_F};
use crate::game::level::{theme_props, BossArena, Prop, Rectf, Room, RoomKind, Theme, TileRect};
use crate::math::Vec2;

/// Solid cells kept between the playable field and the map edge: the wall art
/// draws a south face and a contact shadow, so the border is never one cell.
const BORDER: i32 = 2;

/// Walkable share of the grid the sprawl grows to, before [`scatter_cover`]
/// removes a little back for partitions and pillars.
///
/// The owner's playtest target is "roughly 45–55 % of the grid walkable" (the
/// corridor map this replaced managed about 17 %). One draw per map picks a
/// value in the band, so maps differ without any fans running out of floor.
const OPEN_TARGET: (f64, f64) = (0.49, 0.53);

/// The start area's size in cells: a great hall, not a room.
const START_W: (i32, i32) = (40, 46);
const START_H: (i32, i32) = (28, 34);

/// A plaza (a [`RoomKind::Hall`]) is 34–50 × 24–38 cells: the "40 × 30 or
/// larger" great halls the owner asked for, with the small end still big enough
/// for the horde to surround the rat.
const PLAZA_W: (i32, i32) = (34, 50);
const PLAZA_H: (i32, i32) = (24, 38);

/// A medium room (a [`RoomKind::Room`]) is 16–30 × 12–24 cells: a side chamber,
/// a wing off a plaza, never a cupboard.
const ROOM_W: (i32, i32) = (16, 30);
const ROOM_H: (i32, i32) = (12, 24);

/// The boss arena: bigger than the old 26–32 × 25–30 room, so the jaguar's
/// pounce, the human's sweep and the mower's charge all have room.
const BOSS_W: (i32, i32) = (34, 40);
const BOSS_H: (i32, i32) = (28, 34);

/// Width of the opening two areas share, in cells (the owner's "4 to 10").
const OPENING_W: (i32, i32) = (4, 10);

/// The boss arena's mouth is never narrower than this: a 28 px boss has to walk
/// through it.
const BOSS_MOUTH_MIN: i32 = 6;

/// Cells of bedrock a link may leave between two areas. `0` means the areas
/// merge (they overlap); `1..=3` gets a wide opening punched through the rock.
const LINK_GAP_MAX: i32 = 3;

/// The smallest shared run an opening needs: the widest opening plus a jamb.
const LINK_SPAN_MIN: i32 = 8;

/// How many areas a map may grow to before the sprawl stops. The measured maps
/// reach about half the grid in 18–24 areas.
const MAX_AREAS: usize = 30;

/// How many candidate pairs the loop pass may try to give a starved area a
/// second way out. A crowded map runs out of candidates long before this.
const MAX_LINK_ATTEMPTS: usize = 40;

/// How many great halls the last-resort pass may connect with a carved wide
/// corridor, so "no large area hangs off one way in" holds by construction.
const MAX_RESCUE_LINKS: usize = 2;

/// A "large" area, in cells: the size at which a single way in would be a
/// choke point worth reporting.
const LARGE_AREA_CELLS: i32 = 620;

/// A single-mouth space this big is a dead end the horde can trap the player
/// in: the space rescue pass widens it whatever the floor cost.
const RESCUE_SPACE_CELLS: i32 = 900;

/// The boss arena's index in the `areas` list built by [`build_layout`]: the
/// start area is 0, the arena is placed second and is never a growth parent.
const BOSS_AREA: usize = 1;

/// A free-standing partition's length in cells. Long enough to be worth a
/// rat-hole (stage B wants an 8-cell saving), short enough to walk around.
const PARTITION_LEN: (i32, i32) = (10, 26);

/// Clearance every room keeps from the field edge. Two cells leaves room for
/// the room's own wall ring, the doorway cell outside it and the corridor cell
/// beyond that, so a corridor can always reach a room without touching the
/// border.
const ROOM_MARGIN: i32 = 2;

/// A free-standing room wall (`WallKind::Solid`).
const SOLID: TileKind = TileKind::Wall(WallKind::Solid);

/// The smallest boss arena the validator accepts
/// ([`super::validate::BOSS_ARENA_MIN_CELLS`]) — the generator asks for far
/// more than this, but tests compare against the contract constant.
pub const BOSS_ARENA_CELLS: i32 = 400;

/// A solid within this many pixels counts as **in reach** for a boss that has to
/// steer a charge into one: the Lab mower's flip lane only lands where the art's
/// charge (135 px) meets a wall, prop or bumper, and the framework only prefers
/// an impact within 88 px of the rat. From every cell of the Lab's exported
/// central floor a solid is this close in at least two cardinal directions
/// (`DESIGN_REVIEW.md` §5: "a useful target exists within a screenful").
pub const SOLID_REACH_PX: f32 = 90.0;

/// The outer band of the arena, in cells, that may hold furniture. Everything
/// further in is the encounter floor: the Kitchen needs 96 × 80 px clear at the
/// centre, the Sewer needs the pounce's landing lane, and the Lab's bumpers are
/// the only solids that live there (they are placed as an explicit exception).
pub const ARENA_PERIMETER_CELLS: i32 = 2;

/// Length of each Lab steel bumper in cells: the designer's "two short steel
/// bumpers, each about 3 cells long".
pub const BUMPER_LEN: i32 = 3;

/// The lane the mower's framework insists on beside an impact
/// (`mower::IMPACT_SIDE_LANE`), in pixels. Every bumper keeps it open on both
/// long sides.
const BUMPER_SIDE_LANE_PX: f32 = 32.0;

/// Clear cells the Lab's dock cell keeps from every mouth cell, so the pad is
/// "against a wall away from the mouth".
const DOCK_MOUTH_CLEAR: i32 = 6;

/// Clear cells the dock **cell** keeps from the arena ring: the art's pad
/// footprint is 32 × 30 px ([`crate::game::art_boss_mower::DOCK_HALF_SIZE`]),
/// so one clear cell between pad and wall keeps the pad off the wall art.
const DOCK_WALL_INSET: i32 = 2;

/// Clear cells the Sewer's escape rat-hole keeps from the arena mouth: the hole
/// is an escape route, not a second front door beside the first.
const ESCAPE_MOUTH_CLEAR: i32 = 5;

/// Longest duct the Sewer's escape hole may carve through bedrock to reach open
/// floor outside the arena's wall ring. The search always takes the nearest
/// floor, so most maps need a handful of cells; the arena is the map's
/// outermost room by design, so the cap keeps a pathological case bounded.
const ESCAPE_DUCT_MAX: i32 = 64;

/// One map in this many carries a maze: the owner's "about one in eight".
///
/// It is one draw from the level seed, so the rate is a property of the seed
/// (identical maps on every run) rather than of a clock or of how lucky the
/// slot search was.
pub const MAZE_ONE_IN: u64 = 8;

/// How far the maze slot's edge may sit from the start cell, on the worse
/// axis, so the breach is within about 32 cells of the rat.
///
/// A slot further out is not reserved: an expedition the player has to spend
/// forty seconds walking to is the problem this stage's maze slot exists to
/// fix.
pub const MAZE_NEAR_CELLS: i32 = 32;

/// Cells kept back from [`MAZE_NEAR_CELLS`] for the breach sliding along the
/// outside walkway (`super::maze::ENTRY_SLACK_CELLS`) and the ring's inset, so
/// the breach itself is inside the 32-cell promise.
const MAZE_ENTRY_SLACK: i32 = 6;

/// What stage A reserves for the rest of the pipeline.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Named regions, [`RoomKind::Start`] first.
    pub rooms: Vec<Room>,
    /// Rat spawn point (the start room's centre, `MAP_SIZE * 0.5`).
    pub start: Vec2,
    /// The start room's cells.
    pub start_room: TileRect,
    /// The boss arena's cells.
    pub boss_arena: TileRect,
    /// What the arena furnishes for its fight (the exported [`BossArena`]).
    pub boss: BossArena,
    /// The maze reservation, if this map has one: at most one slot, on about
    /// one map in eight (`MAZE_ONE_IN`). Stage B carves every slot it is given.
    pub maze_slots: Vec<TileRect>,
    /// Free-standing partitions painted inside the open areas: one-cell-thick
    /// walls with floor on both sides.
    ///
    /// They are cover, and — because an open world otherwise has almost no
    /// one-cell walls to pierce — the walls stage B's rat-holes go through. A
    /// hole in a 20-cell partition really does save a cat the walk around it.
    pub partitions: Vec<TileRect>,
}

impl Layout {
    /// The first maze slot that no room except the maze reservation occupies.
    pub fn maze_slot(&self, index: usize) -> Option<TileRect> {
        self.maze_slots.get(index).copied()
    }

    /// The rooms stage B furnishes or traps (everything but the start room,
    /// the boss arena and the corridors).
    pub fn is_furnishable(&self, room: &Room) -> bool {
        matches!(room.kind, RoomKind::Hall | RoomKind::Room)
    }
}

/// One area under construction: its cells and what it is for.
#[derive(Clone, Copy)]
struct Area {
    rect: TileRect,
    kind: RoomKind,
}

/// Round `v` down to an even number: the start area's size must stay even so
/// its centre lands exactly on `MAP_SIZE * 0.5`.
fn even(v: i32) -> i32 {
    v & !1
}

/// The 3 × 3 cells around a room's centre: spawns, traps and tests all use it,
/// so it stays walkable on every map.
fn core_3x3(rect: TileRect) -> TileRect {
    TileRect::new(rect.x + rect.w / 2 - 1, rect.y + rect.h / 2 - 1, 3, 3)
}

/// Carve `rect` and return how many cells became floor (the sprawl's budget).
fn carve_count(tiles: &mut TileMap, rect: TileRect) -> usize {
    let added = rect
        .cells()
        .filter(|&(cx, cy)| tiles.get(cx, cy) == SOLID)
        .count();
    carve(tiles, rect);
    added
}

/// **Stage A.** Carve the map's open world into `tiles` and return the regions.
pub fn build_layout(tiles: &mut TileMap, rng: &mut StdRng, theme: Theme) -> Layout {
    let cols = tiles.cols();
    let rows = tiles.rows();

    // Bedrock first: every area, opening and notch is carved out of it.
    tiles.fill_cells(0, 0, cols, rows, SOLID);
    let field = TileRect::new(BORDER, BORDER, cols - 2 * BORDER, rows - 2 * BORDER);
    let room_area = field.inflate(-ROOM_MARGIN);

    // ---- 1. the start area: a great hall centred on the map --------------
    let start_w = even(rng.random_range(START_W.0..=START_W.1));
    let start_h = even(rng.random_range(START_H.0..=START_H.1));
    let start_room = TileRect::new(
        cols / 2 - start_w / 2,
        rows / 2 - start_h / 2,
        start_w,
        start_h,
    );
    debug_assert_eq!(
        start_room.center_world(),
        Vec2::new(cols as f32 * 0.5 * TILE_F, rows as f32 * 0.5 * TILE_F),
        "the start area must be centred on the map"
    );
    let mut open = carve_count(tiles, start_room);
    let mut areas: Vec<Area> = vec![Area {
        rect: start_room,
        kind: RoomKind::Start,
    }];
    let mut corridors: Vec<TileRect> = Vec::new();
    // The area graph's edges, by construction: one per growth step plus every
    // loop opening and the boss arena's mouth(s).
    let mut linked: Vec<(usize, usize)> = Vec::new();

    // ---- 3. the boss arena: one big room far from the spawn -------------
    // Placed first, because the sprawl is not allowed to touch it: the arena
    // keeps its own wall ring (the Sewer's escape rat-hole, the Lab's dock and
    // the Kitchen's clear centre all need it) and never grows a prop.
    let boss_w = even(rng.random_range(BOSS_W.0..=BOSS_W.1));
    let boss_h = even(rng.random_range(BOSS_H.0..=BOSS_H.1));
    let centre = start_room.center_world();
    let far = |rect: &TileRect| rect.center_world().dist(centre) >= 48.0 * TILE_F;
    let boss_arena = pick_rect(rng, field, ROOM_MARGIN, boss_w, boss_h, 2, |rect| {
        far(&rect) && !rect.inflate(6).intersects(&start_room)
    })
    .or_else(|| {
        pick_rect(rng, field, ROOM_MARGIN, boss_w, boss_h, 2, |rect| {
            !rect.inflate(6).intersects(&start_room)
        })
    })
    .unwrap_or_else(|| {
        // Deterministic last resort: the field's far corner. It cannot overlap
        // the centred start area at this map size.
        TileRect::new(
            room_area.right() - boss_w,
            room_area.bottom() - boss_h,
            boss_w,
            boss_h,
        )
    });
    carve_count(tiles, boss_arena);
    let boss_index = areas.len();
    areas.push(Area {
        rect: boss_arena,
        kind: RoomKind::Boss,
    });
    debug_assert_eq!(boss_index, BOSS_AREA);
    // Links per area, kept while the sprawl grows so the next area can be hung
    // off whichever one still has only one way in: that is what turns the
    // growth tree into a looped network of areas rather than a chain.
    let mut degree: Vec<usize> = vec![0; areas.len()];

    // ---- 2. grow the sprawl ----------------------------------------------
    // One area at a time, attached to an existing one: merged (overlapping) or
    // `gap` cells of bedrock away and joined by a wide opening. Growth stops
    // when about half the grid is floor.
    let hall_bias = match theme {
        Theme::Sewer => 0.42,
        Theme::Kitchen => 0.56,
        Theme::Lab => 0.50,
        // The Laundry is halls and aisles, not warrens: big open bays to line
        // with washers. The Printworks is press-hall floor: slightly more than
        // half halls, so the press islands have room to be offset in.
        Theme::Laundry => 0.58,
        Theme::Printworks => 0.52,
        // The flatbed has its own generator; this is only reached by tests
        // that exercise `build_layout` directly on every theme.
        Theme::Trailer => 0.50,
    };
    let target = OPEN_TARGET.0 + rng.random_range(0..=4) as f64 * 0.01;
    let target_cells = (cols * rows) as f64 * target;
    let mut attempts = 0;
    while (open as f64) < target_cells && areas.len() < MAX_AREAS && attempts < 700 {
        attempts += 1;
        // The boss arena is area 1 and never grows a wing: its mouth is punched
        // once, wide, by the loop pass below. Everything else prefers a parent
        // that still has only one way in, so the sprawl keeps looping back on
        // itself instead of running away in a chain.
        let starved: Vec<usize> = (0..areas.len())
            .filter(|&index| index != BOSS_AREA && degree[index] < 2)
            .collect();
        let parent = if !starved.is_empty() && rng.random_bool(0.65) {
            starved[rng.random_range(0..starved.len())]
        } else {
            let pick = rng.random_range(0..areas.len() - 1);
            if pick == 0 {
                0
            } else {
                pick + 1
            }
        };
        // Every map grows at least two plazas, whatever the theme's bias says:
        // the horde needs great halls to surround the rat in.
        let plaza = rng.random_bool(hall_bias) || areas.len() < 3;
        let (w, h) = if plaza {
            (
                rng.random_range(PLAZA_W.0..=PLAZA_W.1),
                rng.random_range(PLAZA_H.0..=PLAZA_H.1),
            )
        } else {
            (
                rng.random_range(ROOM_W.0..=ROOM_W.1),
                rng.random_range(ROOM_H.0..=ROOM_H.1),
            )
        };
        // A few placements are sampled and the one that lands next to a
        // *second* existing area wins: an area with two ways in is a loop, and
        // loops are what let the player circle away from a horde.
        let mut best: Option<(usize, TileRect, Vec<usize>)> = None;
        for _ in 0..6 {
            let gap = match rng.random_range(0..5) {
                0 | 1 => 0,
                2 => 1,
                3 => 2,
                _ => 3,
            };
            // A merge never leaves a neck narrower than a wide opening.
            let overlap = rng.random_range(4..=8);
            let side = rng.random_range(0..4);
            let Some(rect) =
                attach_rect(rng, room_area, areas[parent].rect, side, w, h, gap, overlap)
            else {
                continue;
            };
            // The parent is the one area this may touch; everything else keeps
            // at least a cell of bedrock between it and the new floor.
            if areas
                .iter()
                .enumerate()
                .any(|(index, area)| index != parent && rect.inflate(1).intersects(&area.rect))
            {
                continue;
            }
            let partners: Vec<usize> = (0..areas.len())
                .filter(|&index| index != parent && shared_run(rect, areas[index].rect).is_some())
                .collect();
            let score = partners.len();
            if best.as_ref().is_none_or(|(bs, _, _)| score > *bs) {
                best = Some((score, rect, partners));
            }
            if score > 0 {
                break; // a second way in: good enough, stop sampling
            }
        }
        let Some((_, rect, partners)) = best else {
            continue;
        };
        open += carve_count(tiles, rect);
        areas.push(Area {
            rect,
            kind: if plaza {
                RoomKind::Hall
            } else {
                RoomKind::Room
            },
        });
        let child = areas.len() - 1;
        degree.push(0);
        if let Some(gallery) = punch_arch(tiles, rng, areas[parent].rect, rect, OPENING_W.0) {
            corridors.push(gallery);
        }
        degree[child] += 1;
        degree[parent] += 1;
        linked.push((parent.min(child), parent.max(child)));
        for partner in partners {
            if partner == parent || linked.contains(&(partner.min(child), partner.max(child))) {
                continue;
            }
            if let Some(gallery) = punch_arch(tiles, rng, rect, areas[partner].rect, OPENING_W.0) {
                corridors.push(gallery);
            }
            degree[child] += 1;
            degree[partner] += 1;
            linked.push((partner.min(child), partner.max(child)));
        }
    }

    // ---- 4. the arena's mouth, and loops: a second way round -------------
    // The growth tree is a tree; an open world needs cycles, or every area
    // hangs off the rest by one opening. Every pair of areas that ended up
    // 1-3 cells apart gets a wide opening of its own — starting with the pairs
    // that give an area its second way out — and the boss arena gets its own
    // wide mouth.
    degree.resize(areas.len(), 0);
    let mut arena_links = 0;
    for other in 0..boss_index {
        if arena_links >= 2 {
            break;
        }
        if link_areas(
            tiles,
            rng,
            &areas,
            other,
            boss_index,
            BOSS_MOUTH_MIN,
            &mut corridors,
            &mut degree,
            &mut linked,
        ) {
            arena_links += 1;
        }
    }
    // No area came within reach of the arena (it sits in the bedrock ring
    // beyond the sprawl): carve one wide link from the arena to the nearest
    // area, so "the boss arena is connected" is structural, not luck.
    if arena_links == 0 {
        let nearest = (0..boss_index)
            .min_by_key(|&index| {
                let rect = areas[index].rect;
                let dx = (rect.x - boss_arena.right())
                    .max(boss_arena.x - rect.right())
                    .max(0);
                let dy = (rect.y - boss_arena.bottom())
                    .max(boss_arena.y - rect.bottom())
                    .max(0);
                (dx.max(dy), index)
            })
            .unwrap_or(0);
        for leg in carve_free_link(
            tiles,
            field,
            areas[nearest].rect,
            boss_arena,
            BOSS_MOUTH_MIN,
            &areas,
        ) {
            corridors.push(leg);
        }
        degree[nearest] += 1;
        degree[boss_index] += 1;
        linked.push((nearest.min(boss_index), nearest.max(boss_index)));
    }

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..areas.len() {
        for j in i + 1..areas.len() {
            if !linked.contains(&(i, j)) && shared_run(areas[i].rect, areas[j].rect).is_some() {
                pairs.push((i, j));
            }
        }
    }
    if !pairs.is_empty() {
        let rotation = rng.random_range(0..pairs.len());
        pairs.rotate_left(rotation);
    }
    let mut pours = 0;
    for &(i, j) in &pairs {
        if pours >= MAX_LINK_ATTEMPTS {
            break;
        }
        // Only the pairs that buy an area its second way out.
        if degree[i] >= 2 && degree[j] >= 2 {
            continue;
        }
        pours += 1;
        link_areas(
            tiles,
            rng,
            &areas,
            i,
            j,
            OPENING_W.0,
            &mut corridors,
            &mut degree,
            &mut linked,
        );
    }

    // A great hall that still has one way in gets a wide corridor to its
    // nearest unlinked neighbour: the owner's "no large region may hang off the
    // rest by a single choke point" is about the big rooms, and geometry alone
    // does not always offer a shared wall to open.
    let mut rescued = 0;
    let mut hopeless: Vec<usize> = Vec::new();
    while rescued < MAX_RESCUE_LINKS {
        let starved = (0..areas.len())
            .filter(|&index| {
                index != BOSS_AREA
                    && areas[index].rect.area() >= LARGE_AREA_CELLS
                    && degree[index] < 2
                    && !hopeless.contains(&index)
            })
            .max_by_key(|&index| (areas[index].rect.area(), index));
        let Some(starved) = starved else { break };
        let partner = (0..areas.len())
            .filter(|&index| {
                index != starved && !linked.contains(&(index.min(starved), index.max(starved)))
            })
            .min_by_key(|&index| (rect_gap(areas[starved].rect, areas[index].rect), index));
        let Some(partner) = partner else {
            hopeless.push(starved);
            continue;
        };
        if rect_gap(areas[starved].rect, areas[partner].rect) > 20 {
            hopeless.push(starved);
            continue;
        }
        for leg in carve_free_link(
            tiles,
            field,
            areas[starved].rect,
            areas[partner].rect,
            OPENING_W.0,
            &areas,
        ) {
            corridors.push(leg);
        }
        degree[starved] += 1;
        degree[partner] += 1;
        linked.push((partner.min(starved), partner.max(starved)));
        rescued += 1;
    }

    // A space made of two merged areas can still open onto the rest of the map
    // only once, even though each of its areas has a second link — to the
    // other. That is exactly the "large region hung off one choke point" the
    // owner ruled out, so a big single-mouth space gets one more wide way in.
    let mut rescued_spaces = 0;
    let mut tries: Vec<(usize, usize)> = Vec::new();
    while rescued_spaces < MAX_RESCUE_LINKS {
        let Some((starved, partner)) = single_mouth_pair(tiles, &areas, &tries) else {
            break;
        };
        tries.push((starved, partner));
        if shared_run(areas[starved].rect, areas[partner].rect).is_some() {
            // A shared wall is cheaper than a corridor: open it wide.
            link_areas(
                tiles,
                rng,
                &areas,
                starved,
                partner,
                OPENING_W.0,
                &mut corridors,
                &mut degree,
                &mut linked,
            );
        } else {
            for leg in carve_free_link(
                tiles,
                field,
                areas[starved].rect,
                areas[partner].rect,
                OPENING_W.0,
                &areas,
            ) {
                corridors.push(leg);
            }
        }
        rescued_spaces += 1;
    }

    // ---- 5. the maze slot: about one map in eight ------------------------
    // Reserved once the halls exist (so it can sit in one of them) and before
    // any cover is painted (so its ring stays open floor). Stage B carves every
    // reservation it is given: a reserved-but-empty maze room cannot happen.
    let start_cell = (
        (start_room.center_world().x / TILE_F) as i32,
        (start_room.center_world().y / TILE_F) as i32,
    );
    let mut maze_slots: Vec<TileRect> = Vec::new();
    if rng.random_range(0..MAZE_ONE_IN) == 0 {
        if let Some(slot) = reserve_maze_slot(rng, tiles, &areas, start_cell) {
            maze_slots.push(slot);
        }
    }

    // ---- 6. cover and texture -------------------------------------------
    let partitions = scatter_cover(tiles, rng, &areas, &maze_slots, start_cell);

    // ---- 6b. the floor's own geometry (wave 3 side routes) ---------------
    // The Laundry's washer aisles and the Printworks' press islands are the
    // character of those floors (`WAVE3_DESIGN.md` §11). They are painted here,
    // after the generic cover and before the connectivity repair, so a bad
    // arrangement is stitched back rather than shipped. Every other theme
    // draws no rng here and is byte-for-byte unchanged.
    theme_layout(tiles, rng, &areas, &maze_slots, start_cell, theme);

    // ---- 7. connectivity repair ------------------------------------------
    // The sprawl is connected by construction. This pass proves it, and
    // stitches anything an area's own geometry left orphaned, so the promise is
    // checked here rather than assumed.
    let mut targets: Vec<TileRect> = areas.iter().map(|area| area.rect).collect();
    for slot in &maze_slots {
        // The ring of open floor around a maze is what its entry and its
        // one-way exit have to reach; repairing inside the slot would be
        // pointless, because stage B carves the maze after this.
        targets.push(slot.inflate(1));
    }
    let seen = connect_orphans(tiles, field, &targets, start_cell);

    // ---- 8. the arena's fight furniture ---------------------------------
    // After every opening (so the mouths are known and the Sewer's escape duct
    // can reach real floor) and after the repair pass (so the duct lands on
    // floor that is actually reachable).
    let boss = furnish_arena(tiles, theme, boss_arena, field);

    let start_cell = (
        (start_room.center_world().x / TILE_F) as i32,
        (start_room.center_world().y / TILE_F) as i32,
    );
    // The owner's promise, measured on the grid that was just carved: every
    // named area has a route at least [`WIDE_ENOUGH`] cells wide. A map that
    // fails it (a merge neck, a bumper, a partition too near a mouth) gets one
    // wide corridor, so the promise is structural and not a hope.
    let skip_boss = matches!(theme, Theme::Laundry | Theme::Printworks);
    for _ in 0..MAX_WIDENINGS {
        let Some(rect) = narrow_area(tiles, &areas, start_cell, &maze_slots, skip_boss) else {
            break;
        };
        let partner = nearest_other(tiles, &areas, rect);
        if let Some(partner) = partner {
            for leg in carve_free_link(tiles, field, rect, areas[partner].rect, WIDE_ENOUGH, &areas) {
                corridors.push(leg);
            }
        }
    }

    // ---- the named regions ----------------------------------------------
    // A doorway that ended up facing rock (a repair corridor that turned a
    // corner, a band clipped at the map edge) is a hole, not a doorway: leave it
    // open so the wall art never draws a jamb against a wall.
    {
        use crate::game::collision::SizeClass;
        let broken: Vec<(i32, i32)> = tiles
            .iter_walls()
            .filter_map(|(cx, cy, kind)| {
                let WallKind::Doorway { axis } = kind else {
                    return None;
                };
                let sides = match axis {
                    WallAxis::Horizontal => [(cx, cy - 1), (cx, cy + 1)],
                    WallAxis::Vertical => [(cx - 1, cy), (cx + 1, cy)],
                };
                sides
                    .iter()
                    .any(|&(sx, sy)| !tiles.passable(sx, sy, SizeClass::Small))
                    .then_some((cx, cy))
            })
            .collect();
        for (cx, cy) in broken {
            tiles.set(cx, cy, TileKind::Open);
        }
    }

    // A widening corridor may have cut a partition in half: only the parts that
    // are still a one-cell wall are offered to stage B as hole candidates.
    let partitions: Vec<TileRect> = partitions
        .into_iter()
        .filter(|rect| {
            rect.cells()
                .all(|(cx, cy)| matches!(tiles.get(cx, cy), TileKind::Wall(WallKind::Solid)))
        })
        .collect();

    let mut rooms: Vec<Room> = Vec::with_capacity(areas.len() + corridors.len() + 1);
    for area in &areas {
        rooms.push(Room::new(area.rect, area.kind));
    }
    // The start area is first in the list by contract; move it there.
    rooms.sort_by_key(|room| room.kind != RoomKind::Start);
    for leg in &corridors {
        if reached(&seen, tiles, *leg) {
            rooms.push(Room::new(*leg, RoomKind::Corridor));
        }
    }
    for slot in &maze_slots {
        rooms.push(Room::new(*slot, RoomKind::Maze));
    }

    Layout {
        rooms,
        start: start_room.center_world(),
        start_room,
        boss_arena,
        boss,
        maze_slots,
        partitions,
    }
}

/// Punch the wide opening between areas `i` and `j` and account for it.
#[allow(clippy::too_many_arguments)]
fn link_areas(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    areas: &[Area],
    i: usize,
    j: usize,
    min_open: i32,
    corridors: &mut Vec<TileRect>,
    degree: &mut [usize],
    linked: &mut Vec<(usize, usize)>,
) -> bool {
    if i == j || degree.len() <= i.max(j) {
        return false;
    }
    if shared_run(areas[i].rect, areas[j].rect).is_none() {
        return false;
    }
    if let Some(gallery) = punch_arch(tiles, rng, areas[i].rect, areas[j].rect, min_open) {
        corridors.push(gallery);
    }
    degree[i] += 1;
    degree[j] += 1;
    linked.push((i.min(j), i.max(j)));
    true
}

/// Place a `w × h` area against `side` of `parent`: `gap` cells of bedrock away,
/// or `gap == 0` for a merge that overlaps the parent by `overlap` cells.
///
/// The new area always keeps a shared run of at least [`LINK_SPAN_MIN`] cells
/// along the parent's wall, so the link can be a wide opening with a jamb at
/// each end. `side` is `0` north, `1` east, `2` south, `3` west. `None` when
/// the placement leaves `bounds`.
#[allow(clippy::too_many_arguments)]
fn attach_rect(
    rng: &mut StdRng,
    bounds: TileRect,
    parent: TileRect,
    side: usize,
    w: i32,
    h: i32,
    gap: i32,
    overlap: i32,
) -> Option<TileRect> {
    if w < 4 || h < 4 || gap < 0 {
        return None;
    }
    let span = LINK_SPAN_MIN;
    let merge = gap == 0;
    let rect = match side {
        0 | 2 => {
            let lo = parent.x - w + span;
            let hi = parent.right() - span;
            if lo > hi {
                return None;
            }
            let x = rng.random_range(lo..=hi);
            let y = if side == 0 {
                parent.y - h - gap + if merge { overlap } else { 0 }
            } else {
                parent.bottom() + gap - if merge { overlap } else { 0 }
            };
            TileRect::new(x, y, w, h)
        }
        _ => {
            let lo = parent.y - h + span;
            let hi = parent.bottom() - span;
            if lo > hi {
                return None;
            }
            let y = rng.random_range(lo..=hi);
            let x = if side == 3 {
                parent.x - w - gap + if merge { overlap } else { 0 }
            } else {
                parent.right() + gap - if merge { overlap } else { 0 }
            };
            TileRect::new(x, y, w, h)
        }
    };
    within(rect, bounds).then_some(rect)
}

/// Chebyshev gap between two rectangles, in cells (`0` when they overlap).
fn rect_gap(a: TileRect, b: TileRect) -> i32 {
    let dx = (b.x - a.right()).max(a.x - b.right()).max(0);
    let dy = (b.y - a.bottom()).max(a.y - b.bottom()).max(0);
    dx.max(dy)
}

/// The run two areas share along the wall between them, when they are `1..=3`
/// cells apart on exactly one axis and that run is wide enough for an opening.
///
/// `Some((vertical, gap))`: `vertical` means the wall between them runs
/// north-south (they are side by side).
fn shared_run(a: TileRect, b: TileRect) -> Option<(bool, i32)> {
    let gap_x = (b.x - a.right()).max(a.x - b.right());
    let gap_y = (b.y - a.bottom()).max(a.y - b.bottom());
    if gap_y > 0 && gap_x <= 0 {
        let span = a.right().min(b.right()) - a.x.max(b.x);
        if span >= LINK_SPAN_MIN && gap_y <= LINK_GAP_MAX {
            return Some((true, gap_y));
        }
    } else if gap_x > 0 && gap_y <= 0 {
        let span = a.bottom().min(b.bottom()) - a.y.max(b.y);
        if span >= LINK_SPAN_MIN && gap_x <= LINK_GAP_MAX {
            return Some((false, gap_x));
        }
    }
    None
}

/// Punch the wide opening between two areas and return the short gallery it
/// became when the rock between them is thicker than one cell.
///
/// A `1`-cell wall gets a run of [`OPENING_W`] cells of `Doorway`; a `2`- or
/// `3`-cell wall keeps its two arch rows as doorways and its middle as plain
/// floor — a short, wide gallery, the only kind of corridor an open map needs.
fn punch_arch(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    a: TileRect,
    b: TileRect,
    min_open: i32,
) -> Option<TileRect> {
    let (vertical, gap) = shared_run(a, b)?;
    let (lo, hi, band) = if vertical {
        (
            a.x.max(b.x),
            a.right().min(b.right()),
            if b.y > a.y { a.bottom() } else { b.bottom() },
        )
    } else {
        (
            a.y.max(b.y),
            a.bottom().min(b.bottom()),
            if b.x > a.x { a.right() } else { b.right() },
        )
    };
    let span = hi - lo;
    if span < LINK_SPAN_MIN {
        return None;
    }
    // A jamb at each end where the span allows one, so the opening reads as an
    // arch in a wall rather than a room that lost its side.
    let widest = (span - 2).clamp(1, OPENING_W.1);
    let mut narrowest = OPENING_W.0.min(widest).max(1);
    let widest = widest.max(narrowest);
    narrowest = narrowest.max(min_open.max(1).min(widest)).min(widest);
    let open = rng.random_range(narrowest..=widest).min(span);
    let first = lo + rng.random_range(0..=(span - open));
    let axis = if vertical {
        WallAxis::Horizontal
    } else {
        WallAxis::Vertical
    };
    for step in 0..gap {
        let line = band + step;
        let arch = step == 0 || step + 1 == gap;
        for k in 0..open {
            let (cx, cy) = if vertical {
                (first + k, line)
            } else {
                (line, first + k)
            };
            if tiles.get(cx, cy) != SOLID {
                continue;
            }
            tiles.set(
                cx,
                cy,
                if arch {
                    TileKind::Wall(WallKind::Doorway { axis })
                } else {
                    TileKind::Open
                },
            );
        }
    }
    (gap >= 2).then(|| {
        if vertical {
            TileRect::new(first, band, open, gap)
        } else {
            TileRect::new(band, first, gap, open)
        }
    })
}

/// Carve a wide L-shaped link between two areas that are too far apart for a
/// shared opening, and return the two bands (recorded as corridors).
///
/// This is the only corridor an open map grows besides the short galleries a
/// two- or three-cell wall leaves behind, and it is never narrower than
/// `width` (the owner's "at least 4 cells": the boss arena asks for 6).
fn carve_free_link(
    tiles: &mut TileMap,
    field: TileRect,
    a: TileRect,
    b: TileRect,
    width: i32,
    areas: &[Area],
) -> Vec<TileRect> {
    let width = width.clamp(4, 12);
    let half = width / 2;
    let ax = a.x + a.w / 2;
    let ay = a.y + a.h / 2;
    let bx = b.x + b.w / 2;
    let by = b.y + b.h / 2;
    let clamp_x = |x: i32| (x - half).clamp(field.x, field.right() - width);
    let clamp_y = |y: i32| (y - half).clamp(field.y, field.bottom() - width);
    let horizontal = TileRect::new(ax.min(bx), clamp_y(ay), (ax - bx).abs() + 1, width);
    let vertical = TileRect::new(clamp_x(bx), ay.min(by), width, (ay - by).abs() + 1);
    // The two ends of each band are arches, not plain holes: a doorway cell is
    // what stops a prop or a pillar being painted in the mouth later (and what
    // the wall art draws a jamb for).
    let mut arches: Vec<(i32, i32, WallAxis)> = Vec::new();
    for (band, axis) in [
        (horizontal, WallAxis::Vertical),
        (vertical, WallAxis::Horizontal),
    ] {
        let passage_x = axis == WallAxis::Vertical;
        let along = if passage_x { band.w } else { band.h };
        let across = if passage_x { band.h } else { band.w };
        for k in 0..across {
            let cell_at = |step: i32| {
                if passage_x {
                    (band.x + step, band.y + k)
                } else {
                    (band.x + k, band.y + step)
                }
            };
            let solid = |step: i32| {
                let (cx, cy) = cell_at(step);
                tiles.get(cx, cy) == SOLID
            };
            let first = (0..along).find(|&step| solid(step));
            let last = (0..along).rev().find(|&step| solid(step));
            if let (Some(first), Some(last)) = (first, last) {
                for step in [first, last] {
                    let (cx, cy) = cell_at(step);
                    // An arch belongs in a wall between two rooms, never in the
                    // middle of a room the band happens to cross.
                    // The arch needs floor on both sides of the passage: a band
                    // clipped by the map edge would otherwise leave a doorway
                    // facing solid rock.
                    let neighbours = match axis {
                        WallAxis::Vertical => [(cx - 1, cy), (cx + 1, cy)],
                        WallAxis::Horizontal => [(cx, cy - 1), (cx, cy + 1)],
                    };
                    if within(TileRect::new(cx, cy, 1, 1), field)
                        && !areas.iter().any(|area| area.rect.contains(cx, cy))
                        && neighbours
                            .iter()
                            .all(|&(nx, ny)| within(TileRect::new(nx, ny, 1, 1), field))
                    {
                        arches.push((cx, cy, axis));
                    }
                }
            }
        }
    }
    for (cx, cy, axis) in &arches {
        if tiles.get(*cx, *cy) == SOLID {
            tiles.set(
                *cx,
                *cy,
                TileKind::Wall(WallKind::Doorway { axis: *axis }),
            );
        }
    }
    for (band, _) in [(horizontal, 0), (vertical, 0)] {
        carve_if_solid(tiles, band, field);
    }
    // Record only the part of each band that was new floor: the arms inside the
    // two rooms are already open, so a "corridor" that spans the map is not.
    let trim = |band: TileRect| -> Option<TileRect> {
        let mut lo = (i32::MAX, i32::MAX);
        let mut hi = (i32::MIN, i32::MIN);
        for (cx, cy) in band.cells() {
            if !within(TileRect::new(cx, cy, 1, 1), field) {
                continue;
            }
            if a.contains(cx, cy) || b.contains(cx, cy) {
                continue;
            }
            lo = (lo.0.min(cx), lo.1.min(cy));
            hi = (hi.0.max(cx), hi.1.max(cy));
        }
        (lo.0 <= hi.0).then(|| TileRect::new(lo.0, lo.1, hi.0 - lo.0 + 1, hi.1 - lo.1 + 1))
    };
    [trim(horizontal), trim(vertical)]
        .into_iter()
        .flatten()
        .collect()
}

/// Find a big open space (areas that overlap are one space) with a single
/// opening, and an area outside it to hang a second one off.
///
/// Returns `None` once every space of [`LARGE_AREA_CELLS`] or more opens onto
/// the rest of the map at least twice, which is the promise this pass keeps.
fn single_mouth_pair(
    tiles: &TileMap,
    areas: &[Area],
    tried: &[(usize, usize)],
) -> Option<(usize, usize)> {
    let cols = tiles.cols();
    let rows = tiles.rows();
    let cells = (cols * rows) as usize;
    // Union the areas that overlap: they are one open space.
    let mut group: Vec<usize> = (0..areas.len()).collect();
    fn root(group: &mut [usize], mut x: usize) -> usize {
        while group[x] != x {
            group[x] = group[group[x]];
            x = group[x];
        }
        x
    }
    for i in 0..areas.len() {
        for j in i + 1..areas.len() {
            if areas[i].rect.intersects(&areas[j].rect) {
                let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                if ri != rj {
                    group[ri] = rj;
                }
            }
        }
    }
    // Which space every open cell belongs to (`-1` for none).
    let mut owner = vec![-1i32; cells];
    for (index, area) in areas.iter().enumerate() {
        let space = root(&mut group, index) as i32;
        for (cx, cy) in area.rect.cells() {
            if !tiles.in_bounds(cx, cy) {
                continue;
            }
            let i = (cy * cols + cx) as usize;
            if tiles.get(cx, cy).is_open() && owner[i] < 0 {
                owner[i] = space;
            }
        }
    }
    // Openings per space: contiguous runs of floor outside the space that touch
    // it. A space with one run has one way in.
    let mut openings: Vec<Vec<(i32, i32)>> = vec![Vec::new(); areas.len()];
    let mut sizes = vec![0i32; areas.len()];
    for (index, area) in areas.iter().enumerate() {
        sizes[root(&mut group, index)] += area.rect.area();
    }
    for cy in 0..rows {
        for cx in 0..cols {
            let i = (cy * cols + cx) as usize;
            if owner[i] < 0 {
                continue;
            }
            let space = owner[i] as usize;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if !tiles.in_bounds(nx, ny) || !tiles.get(nx, ny).is_open() {
                    continue;
                }
                let j = (ny * cols + nx) as usize;
                if owner[j] == owner[i] {
                    continue;
                }
                if !openings[space].contains(&(nx, ny)) {
                    openings[space].push((nx, ny));
                }
            }
        }
    }
    let count_openings = |cells_of_space: &[(i32, i32)]| -> usize {
        let mut seen: Vec<(i32, i32)> = Vec::new();
        let mut runs = 0usize;
        for &cell in cells_of_space {
            if seen.contains(&cell) {
                continue;
            }
            runs += 1;
            seen.push(cell);
            let mut queue = vec![cell];
            while let Some((cx, cy)) = queue.pop() {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let next = (cx + dx, cy + dy);
                    if !seen.contains(&next) && cells_of_space.contains(&next) {
                        seen.push(next);
                        queue.push(next);
                    }
                }
            }
        }
        runs
    };
    // The largest single-mouth space wins; the area to link from is its biggest.
    let mut worst: Option<(i32, usize, usize)> = None; // (size, space, area)
    for space in 0..areas.len() {
        if root(&mut group, space) != space || sizes[space] < RESCUE_SPACE_CELLS {
            continue;
        }
        if count_openings(&openings[space]) > 1 {
            continue;
        }
        let member = (0..areas.len())
            .filter(|&index| root(&mut group, index) == space)
            .max_by_key(|&index| (areas[index].rect.area(), index))?;
        if worst.is_none_or(|(size, _, _)| sizes[space] > size) {
            worst = Some((sizes[space], space, member));
        }
    }
    let (_, space, starved) = worst?;
    // The new way in has to be a *second* one: pick the area on the side
    // opposite the existing opening (a link beside the old mouth would merge
    // with it and leave the space one-mouthed), and never retry a pair.
    let old = openings[space].iter().fold((0.0f32, 0.0f32), |acc, &(cx, cy)| {
        (acc.0 + cx as f32, acc.1 + cy as f32)
    });
    let count = openings[space].len().max(1) as f32;
    let hub = areas[starved].rect.center_world();
    let away = Vec2::new(old.0 / count * TILE_F, old.1 / count * TILE_F) - hub;
    let away = if away.len_sq() > 1.0 { away.normalized() } else { Vec2::ZERO };
    let mut candidates: Vec<(f32, i32, usize)> = (0..areas.len())
        .filter(|&index| root(&mut group, index) != space)
        .filter(|&index| !tried.contains(&(starved, index)))
        .filter(|&index| rect_gap(areas[starved].rect, areas[index].rect) <= 24)
        .map(|index| {
            let dir = (areas[index].rect.center_world() - hub).normalized();
            // Lower is better: pointing against the existing opening first.
            (-(dir.x * away.x + dir.y * away.y), rect_gap(areas[starved].rect, areas[index].rect), index)
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    let partner = candidates.first().map(|&(_, _, index)| index)?;
    Some((starved, partner))
}

/// The narrowest passage a required route may use, in cells (the owner's
/// "no required route through a passage narrower than 3 cells").
const WIDE_ENOUGH: i32 = 3;

/// How many wide repairs the promise may cost one map.
const MAX_WIDENINGS: usize = 3;

/// The width of every passable cell: the shorter of its east–west and
/// north–south open runs. A one-cell corridor reads `1`, a four-cell opening
/// reads `4`.
fn cell_widths(tiles: &TileMap) -> Vec<i32> {
    use crate::game::collision::SizeClass;
    let (cols, rows) = (tiles.cols(), tiles.rows());
    let index = |cx: i32, cy: i32| (cy * cols + cx) as usize;
    let open = |cx: i32, cy: i32| {
        cx >= 0 && cy >= 0 && cx < cols && cy < rows && tiles.passable(cx, cy, SizeClass::Small)
    };
    let mut east_west = vec![0i32; (cols * rows) as usize];
    let mut north_south = vec![0i32; (cols * rows) as usize];
    // A cell's run is the whole open stretch through it: forward + back − 1.
    for cy in 0..rows {
        let mut run = 0;
        for cx in 0..cols {
            run = if open(cx, cy) { run + 1 } else { 0 };
            east_west[index(cx, cy)] = run;
        }
        run = 0;
        for cx in (0..cols).rev() {
            run = if open(cx, cy) { run + 1 } else { 0 };
            let i = index(cx, cy);
            if run > 0 {
                east_west[i] += run - 1;
            }
        }
    }
    for cx in 0..cols {
        let mut run = 0;
        for cy in 0..rows {
            run = if open(cx, cy) { run + 1 } else { 0 };
            north_south[index(cx, cy)] = run;
        }
        run = 0;
        for cy in (0..rows).rev() {
            run = if open(cx, cy) { run + 1 } else { 0 };
            let i = index(cx, cy);
            if run > 0 {
                north_south[i] += run - 1;
            }
        }
    }
    let mut widths = vec![0i32; (cols * rows) as usize];
    for i in 0..widths.len() {
        widths[i] = east_west[i].min(north_south[i]);
    }
    widths
}

/// The widest route from `start` to every cell (max-min over paths).
fn widest_paths(tiles: &TileMap, start: (i32, i32)) -> Vec<i32> {
    let cols = tiles.cols();
    let widths = cell_widths(tiles);
    let mut best = vec![0i32; widths.len()];
    let index = |cx: i32, cy: i32| (cy * cols + cx) as usize;
    if widths[index(start.0, start.1)] == 0 {
        return best;
    }
    // Bucket queue: the width is a small integer, so no heap is needed.
    let mut buckets: Vec<Vec<(i32, i32)>> = vec![Vec::new(); 64];
    let cap = |w: i32| w.clamp(0, 63) as usize;
    best[index(start.0, start.1)] = widths[index(start.0, start.1)];
    buckets[cap(best[index(start.0, start.1)])].push(start);
    for width in (1..64).rev() {
        while let Some((cx, cy)) = buckets[width].pop() {
            if best[index(cx, cy)] != width as i32 {
                continue;
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx < 0 || ny < 0 || nx >= cols || ny >= tiles.rows() {
                    continue;
                }
                let j = index(nx, ny);
                if widths[j] == 0 {
                    continue;
                }
                let candidate = (width as i32).min(widths[j]);
                if candidate > best[j] {
                    best[j] = candidate;
                    buckets[cap(candidate)].push((nx, ny));
                }
            }
        }
    }
    best
}

/// A named area whose best route is narrower than [`WIDE_ENOUGH`]: the largest
/// such area's cells, widest first (so the repair starts from the room and not
/// from the seam).
fn narrow_area(
    tiles: &TileMap,
    areas: &[Area],
    start: (i32, i32),
    slots: &[TileRect],
    skip_boss: bool,
) -> Option<TileRect> {
    let best = widest_paths(tiles, start);
    let cols = tiles.cols();
    let index = |cx: i32, cy: i32| (cy * cols + cx) as usize;
    let mut worst: Option<(i32, TileRect)> = None;
    for area in areas {
        // The boss arena is furnished for its fight (the Lab's bumpers, the
        // mower's dock); a wide repair corridor carved into it would cut the
        // very furniture the fight needs. The wave 3 floors opt out entirely
        // (`skip_boss`), because their halls hold more floor furniture than the
        // original themes' and the arena came out narrowest too often. The
        // original three themes keep the historical behaviour byte for byte.
        if area.kind == RoomKind::Boss && (skip_boss || area.rect.area() < BOSS_ARENA_CELLS) {
            continue;
        }
        let mut floor = 0;
        let mut reachable = 0;
        let mut enough = 0;
        for (cx, cy) in area.rect.cells() {
            if !tiles.in_bounds(cx, cy) || !tiles.get(cx, cy).is_open() {
                continue;
            }
            // A maze inside the area is a warren of one-cell aisles by design:
            // it is not the room's route, so it is not measured here.
            if slots.iter().any(|slot| slot.contains(cx, cy)) {
                continue;
            }
            floor += 1;
            if best[index(cx, cy)] > 0 {
                reachable += 1;
                if best[index(cx, cy)] >= WIDE_ENOUGH {
                    enough += 1;
                }
            }
        }
        // Only a room you can actually stand in needs the promise.
        if floor < 40 || enough * 4 >= floor * 3 {
            continue;
        }
        let _ = reachable;
        if worst.is_none_or(|(size, _)| floor > size) {
            worst = Some((floor, area.rect));
        }
    }
    worst.map(|(_, rect)| rect)
}

/// The index of the area nearest `rect`, so a widening corridor is short.
fn nearest_other(tiles: &TileMap, areas: &[Area], rect: TileRect) -> Option<usize> {
    let _ = tiles;
    (0..areas.len())
        .filter(|&index| areas[index].rect != rect)
        .min_by_key(|&index| (rect_gap(rect, areas[index].rect), index))
}

/// Reserve the map's maze slot inside an area near the spawn: its ring, entry,
/// exit and gnaw-mark trail all need open floor around it, and the owner's
/// "findable near the start" puts it inside the first minute's walk.
fn reserve_maze_slot(
    rng: &mut StdRng,
    tiles: &TileMap,
    areas: &[Area],
    start_cell: (i32, i32),
) -> Option<TileRect> {
    // An odd slot carrying an odd aisle lattice: 19/21/23 by 15/17/19 cells
    // (`maze::AISLE_STEP` doubles the lattice inside it).
    let w = 2 * rng.random_range(9..=11) + 1;
    let h = 2 * rng.random_range(7..=9) + 1;
    // Halls and rooms first (nearest the spawn), the start area last: a warren
    // in a plaza is an excursion, one in the starting hall is a hole in the
    // first minute's kiting room.
    let mut hosts: Vec<(i32, usize)> = areas
        .iter()
        .enumerate()
        .filter(|(_, area)| matches!(area.kind, RoomKind::Hall | RoomKind::Room))
        .map(|(index, area)| (cell_point_gap(start_cell, area.rect), index))
        .collect();
    hosts.sort_by_key(|&(gap, index)| (gap, index));
    hosts.push((0, 0));
    for (_, index) in hosts {
        // Two cells in from the host's wall: the one-cell ring around the slot
        // plus a cell of walkway, so walking past the warren is never the only
        // route between a wall and a maze partition.
        let inner = areas[index].rect.inflate(-2);
        if inner.w < w || inner.h < h {
            continue;
        }
        let mut candidates: Vec<TileRect> = Vec::new();
        for cy in inner.y..=inner.bottom() - h {
            for cx in inner.x..=inner.right() - w {
                let slot = TileRect::new(cx, cy, w, h);
                if cell_point_gap(start_cell, slot) > MAZE_NEAR_CELLS - MAZE_ENTRY_SLACK {
                    continue;
                }
                // The spawn's own 3 × 3 stays floor, and the whole slot plus its
                // ring is already open floor (stage B paints the ring and the
                // lattice into it).
                if slot.inflate(1).contains(start_cell.0, start_cell.1) {
                    continue;
                }
                if !slot.inflate(1).cells().all(|(x, y)| tiles.get(x, y).is_open()) {
                    continue;
                }
                candidates.push(slot);
            }
        }
        if !candidates.is_empty() {
            return Some(candidates[rng.random_range(0..candidates.len())]);
        }
    }
    None
}

/// May `candidate` be painted as cover? It stays clear of the spawn, the area's
/// own centre, every opening's approach, the maze slots and other cover.
fn cover_is_free(
    tiles: &TileMap,
    candidate: TileRect,
    slots: &[TileRect],
    painted: &[TileRect],
    spawn_box: TileRect,
    cores: &[TileRect],
) -> bool {
    // Every area's centre stays clear, not just this one's: areas merge, so a
    // pillar in one hall can land on its neighbour's anchor.
    if candidate.intersects(&spawn_box) || cores.iter().any(|core| candidate.intersects(core)) {
        return false;
    }
    if slots
        .iter()
        .any(|slot| slot.inflate(1).intersects(&candidate))
    {
        return false;
    }
    if painted
        .iter()
        .any(|other| other.inflate(2).intersects(&candidate))
    {
        return false;
    }
    !keeps_openings_clear_within(tiles, candidate, PROP_OPENING_CLEAR)
}

/// Notched corners, free-standing partitions, wall stubs and pillars.
///
/// This is what turns a field of open rectangles into a place: cover to break
/// line of sight, corners to tuck into, and — for stage B — the long one-cell
/// partitions that are the only walls a rat-hole can pierce on an open map. The
/// returned rectangles are the partitions, recorded in [`Layout::partitions`].
fn scatter_cover(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    areas: &[Area],
    slots: &[TileRect],
    start_cell: (i32, i32),
) -> Vec<TileRect> {
    let mut partitions: Vec<TileRect> = Vec::new();
    let mut painted: Vec<TileRect> = Vec::new();
    let spawn_box = TileRect::new(start_cell.0 - 1, start_cell.1 - 1, 3, 3);
    let cores: Vec<TileRect> = areas
        .iter()
        .map(|area| core_3x3(area.rect).inflate(2))
        .collect();
    for area in areas {
        let rect = area.rect;
        // The boss arena is furnished by `furnish_arena` and the validator
        // insists on its clear encounter floor; the sprawl does not touch it.
        if area.kind == RoomKind::Boss || rect.w < 16 || rect.h < 16 {
            continue;
        }
        // Interior work keeps four cells from the wall ring: an opening lives
        // on that ring, and cover in front of one turns it into a dead end.
        let inner = rect.inflate(-4);
        if inner.w < 6 || inner.h < 6 {
            continue;
        }

        // 1. Notched corners: a bite out of one or two corners, so an area is
        //    not a rectangle.
        for _ in 0..rng.random_range(0..=2) {
            let nw = rng.random_range(3..=6);
            let nh = rng.random_range(3..=5);
            let (x, y) = match rng.random_range(0..4) {
                0 => (rect.x, rect.y),
                1 => (rect.right() - nw, rect.y),
                2 => (rect.x, rect.bottom() - nh),
                _ => (rect.right() - nw, rect.bottom() - nh),
            };
            let notch = TileRect::new(x, y, nw, nh);
            if !cover_is_free(tiles, notch, slots, &painted, spawn_box, &cores) {
                continue;
            }
            tiles.fill_cells(notch.x, notch.y, notch.w, notch.h, SOLID);
            painted.push(notch);
        }

        // 2. Partitions: a free-standing one-cell wall across the area, four
        //    cells clear of the wall ring so both its ends have a way round.
        if rect.area() >= 620 && rng.random_bool(0.8) {
            let count = if rect.area() >= 1200 {
                rng.random_range(1..=2)
            } else {
                1
            };
            for _ in 0..count {
                let horizontal = rng.random_bool(0.5);
                let longest = if horizontal {
                    inner.w.min(PARTITION_LEN.1)
                } else {
                    inner.h.min(PARTITION_LEN.1)
                };
                if longest < PARTITION_LEN.0 {
                    continue;
                }
                let len = rng.random_range(PARTITION_LEN.0..=longest);
                let partition = if horizontal {
                    let y = rng.random_range(inner.y..inner.bottom());
                    let x = rng.random_range(inner.x..=inner.right() - len);
                    TileRect::new(x, y, len, 1)
                } else {
                    let x = rng.random_range(inner.x..inner.right());
                    let y = rng.random_range(inner.y..=inner.bottom() - len);
                    TileRect::new(x, y, 1, len)
                };
                if !cover_is_free(tiles, partition, slots, &painted, spawn_box, &cores) {
                    continue;
                }
                tiles.fill_cells(
                    partition.x,
                    partition.y,
                    partition.w,
                    partition.h,
                    SOLID,
                );
                painted.push(partition);
                partitions.push(partition);
            }
        }

        // 3. Wall stubs: short spurs off the wall ring, pointing in.
        for _ in 0..rng.random_range(0..=3) {
            let len = rng.random_range(3..=6);
            let stub = match rng.random_range(0..4) {
                0 => TileRect::new(
                    rng.random_range(rect.x + 2..=rect.right() - 2),
                    rect.y,
                    1,
                    len,
                ),
                1 => TileRect::new(
                    rect.right() - len,
                    rng.random_range(rect.y + 2..=rect.bottom() - 2),
                    len,
                    1,
                ),
                2 => TileRect::new(
                    rng.random_range(rect.x + 2..=rect.right() - 2),
                    rect.bottom() - len,
                    1,
                    len,
                ),
                _ => TileRect::new(
                    rect.x,
                    rng.random_range(rect.y + 2..=rect.bottom() - 2),
                    len,
                    1,
                ),
            };
            if !cover_is_free(tiles, stub, slots, &painted, spawn_box, &cores) {
                continue;
            }
            tiles.fill_cells(stub.x, stub.y, stub.w, stub.h, SOLID);
            painted.push(stub);
        }

        // 4. Pillars: free-standing blocks in the open floor.
        for _ in 0..rng.random_range(0..=4) {
            let (w, h) = [(2, 2), (2, 3), (3, 2), (3, 3)][rng.random_range(0..4)];
            if w > inner.w || h > inner.h {
                continue;
            }
            let Some(pillar) = random_rect(rng, inner, w, h) else {
                continue;
            };
            if !cover_is_free(tiles, pillar, slots, &painted, spawn_box, &cores) {
                continue;
            }
            tiles.fill_cells(pillar.x, pillar.y, pillar.w, pillar.h, SOLID);
            painted.push(pillar);
        }
    }
    partitions
}

// ============================================================================
// Theme floor geometry (wave 3 side routes)
// ============================================================================

/// Paint the theme's own floor geometry into the open areas.
///
/// * **Laundry** — paired washer banks around aisles, with a folding bay beside
///   every pair: a 10-cell pattern of `bank(2) aisle(3) bank(2) bay(3)`, the
///   banks pierced by a cross-aisle every 6–9 cells and both ends two cells
///   short of the hall so every aisle loops.
/// * **Printworks** — offset press islands on a 10 × 7-cell grid (a 7 × 4-cell
///   island in a 3-cell paper lane), every other row shifted half a period so
///   the lanes wind, and two stair-step diagonal shortcuts cut through each
///   island row.
///
/// Every other theme returns before drawing a single rng number, which is what
/// keeps the Sewer/Kitchen/Lab maps identical to the pinned trace.
fn theme_layout(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    areas: &[Area],
    slots: &[TileRect],
    start_cell: (i32, i32),
    theme: Theme,
) {
    match theme {
        Theme::Laundry => laundry_floor(tiles, rng, areas, slots, start_cell),
        Theme::Printworks => printworks_floor(tiles, rng, areas, slots, start_cell),
        _ => {}
    }
}

/// The cells a theme pass must never paint: everywhere an anchor, a route or a
/// reservation lives.
fn theme_guard(areas: &[Area], slots: &[TileRect], start_cell: (i32, i32)) -> Vec<TileRect> {
    let mut guard: Vec<TileRect> = areas
        .iter()
        .map(|area| core_3x3(area.rect).inflate(2))
        .collect();
    guard.push(TileRect::new(start_cell.0 - 2, start_cell.1 - 2, 5, 5));
    for slot in slots {
        guard.push(slot.inflate(2));
    }
    guard
}

/// May this run of cells be turned into theme furniture? It must be open floor,
/// off every guard, and wholly inside the area it belongs to.
fn theme_run_ok(tiles: &TileMap, rect: TileRect, guard: &[TileRect]) -> bool {
    if rect.w <= 0 || rect.h <= 0 {
        return false;
    }
    if guard.iter().any(|g| g.intersects(&rect)) {
        return false;
    }
    rect.cells().all(|(cx, cy)| tiles.get(cx, cy).is_open())
}

/// Laundry: paired washer banks around three-cell aisles, a folding bay beside
/// every pair, cross-aisles every 6–9 cells and both aisle ends open.
fn laundry_floor(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    areas: &[Area],
    slots: &[TileRect],
    start_cell: (i32, i32),
) {
    let guard = theme_guard(areas, slots, start_cell);
    for area in areas {
        if area.kind == RoomKind::Boss {
            continue;
        }
        let rect = area.rect;
        // Three cells clear of the ring: an opening lives on the ring, and the
        // cell inside it has to stay walkable.
        let inner = rect.inflate(-3);
        if inner.w < 14 || inner.h < 12 || inner.area() < 320 {
            continue;
        }
        // Sink the aisles a little: one or two cells of random margin so two
        // halls of the same size do not line up.
        let margin = rng.random_range(0..=2);
        let top = inner.y + margin;
        let bottom = inner.bottom() - margin;
        if bottom - top < 8 {
            continue;
        }
        // 10-cell pattern: bank(2) aisle(3) bank(2) bay(3).
        let mut x = inner.x;
        while x + 4 <= inner.right() {
            let bank = TileRect::new(x, top, 2, bottom - top);
            // Cross-aisle every 6–9 cells: the banks are what makes an aisle
            // long, and a cross-aisle is what keeps a long one looping.
            let mut openings: Vec<(i32, i32)> = Vec::new();
            let mut y = top;
            while y < bottom {
                let run = rng.random_range(6..=9);
                let gap = rng.random_range(1..=2);
                y += run;
                if y + gap < bottom {
                    openings.push((y, gap));
                }
                y += gap;
            }
            let cells: Vec<(i32, i32)> = bank
                .cells()
                .filter(|&(_, cy)| {
                    !openings
                        .iter()
                        .any(|&(oy, oh)| cy >= oy && cy < oy + oh)
                })
                .collect();
            // A bank is only painted when every cell it covers is free floor:
            // a bank is furniture, not a repair.
            let blocked = cells.iter().any(|&(cx, cy)| {
                !tiles.get(cx, cy).is_open()
                    || guard.iter().any(|g| g.contains(cx, cy))
            });
            if !blocked && cells.len() > 6 {
                for (cx, cy) in cells {
                    tiles.set(cx, cy, SOLID);
                }
            }
            // bank(2) + aisle(3) + bank(2) + bay(3) = 10.
            x += 10;
        }
    }
}

/// Printworks: offset press islands on a 10 × 7-cell grid with three-cell paper
/// lanes, staggered every other row, and stair-step diagonal shortcuts cut
/// through two islands per row.
fn printworks_floor(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    areas: &[Area],
    slots: &[TileRect],
    start_cell: (i32, i32),
) {
    const IW: i32 = 7;
    const IH: i32 = 4;
    const LANE: i32 = 3;
    let guard = theme_guard(areas, slots, start_cell);
    for area in areas {
        if area.kind == RoomKind::Boss {
            continue;
        }
        let rect = area.rect;
        let inner = rect.inflate(-3);
        if inner.w < 16 || inner.h < 14 || inner.area() < 340 {
            continue;
        }
        let phase = rng.random_range(0..=1) * ((IW + LANE) / 2);
        let mut islands: Vec<TileRect> = Vec::new();
        let mut row = 0;
        let mut y = inner.y;
        while y + IH <= inner.bottom() {
            // Offset press islands: every other row slides half a period, so
            // the lanes have to wind around them.
            let shift = if row % 2 == 0 { 0 } else { phase.max(2) };
            let mut x = inner.x - shift;
            while x + IW <= inner.right() {
                let island = TileRect::new(x.max(inner.x), y, IW, IH);
                if island.right() <= inner.right()
                    && theme_run_ok(tiles, island, &guard)
                {
                    for (cx, cy) in island.cells() {
                        tiles.set(cx, cy, SOLID);
                    }
                    islands.push(island);
                }
                x += IW + LANE;
            }
            row += 1;
            y += IH + LANE;
        }
        // Two stair-step shortcuts per island row (design: "at least two
        // diagonal stair-step shortcuts through each long pair of lanes"): a
        // one-cell diagonal cut from the north lane to the south lane.
        let mut per_row: Vec<usize> = Vec::new();
        for (index, island) in islands.iter().enumerate() {
            let r = (island.y - inner.y) / (IH + LANE);
            if let Some(slot) = per_row.get_mut(r as usize) {
                if *slot < 2 {
                    *slot += 1;
                    carve_stair(tiles, *island, (index % 2) as u8);
                }
            } else {
                per_row.resize(r as usize + 1, 0);
                per_row[r as usize] = 1;
                carve_stair(tiles, *island, (index % 2) as u8);
            }
        }
    }
}

/// Cut a one-cell diagonal staircase through an island, leaving a cell of press
/// either side of the cut so it reads as a shortcut rather than a hole.
fn carve_stair(tiles: &mut TileMap, island: TileRect, variant: u8) {
    let steps = island.h.min(island.w - 2).max(2);
    for k in 0..steps {
        let cx = if variant == 0 {
            island.x + 1 + k
        } else {
            island.right() - 2 - k
        };
        let cy = island.y + k;
        if island.contains(cx, cy) {
            tiles.set(cx, cy, TileKind::Open);
        }
    }
}

/// Is any cell of `rect` on the reached patch? (`validate::reaches_rect`
/// without the "big room" shrink — corridors and rings are thin rects.)
fn reached(seen: &[bool], tiles: &TileMap, rect: TileRect) -> bool {
    use crate::game::levelgen::validate;
    rect.cells().any(|(cx, cy)| validate::is_reached(seen, tiles, (cx, cy)))
}

/// Stitch every region that the flood fill from `start` cannot reach back onto
/// the rest of the map with a straight (L-shaped) one-cell corridor.
///
/// Returns the final reachability bitmap (the caller uses it to drop dead
/// corridor stubs without a second flood fill). Bounded, no rng, no hash map:
/// normally nothing is unreachable and the single flood fill is the whole cost.
/// A repair that does not reach its target is remembered and not retried, so a
/// pathological map cannot loop.
fn connect_orphans(
    tiles: &mut TileMap,
    field: TileRect,
    targets: &[TileRect],
    start: (i32, i32),
) -> Vec<bool> {
    use crate::game::collision::SizeClass;
    use crate::game::levelgen::validate;

    /// Rounds of (flood fill, carve, flood fill) a broken map may cost.
    const MAX_REPAIRS: usize = 6;

    let mut seen = validate::reachable(tiles, start, SizeClass::Small);
    let mut hopeless: Vec<TileRect> = Vec::new();
    for _ in 0..MAX_REPAIRS {
        let target = targets
            .iter()
            .find(|r| !hopeless.contains(r) && !validate::reaches_rect(&seen, tiles, **r))
            .copied();
        let Some(target) = target else { break };
        let Some((from, to)) = nearest_pair(tiles, &seen, target, field) else {
            hopeless.push(target);
            continue;
        };
        carve_repair_path(tiles, field, from, to, target);
        seen = validate::reachable(tiles, start, SizeClass::Small);
        if !validate::reaches_rect(&seen, tiles, target) {
            hopeless.push(target);
        }
    }
    seen
}

/// Carve one straight (L-shaped) one-cell repair corridor from `from` to `to`,
/// turning the wall cells it crosses into doorways rather than plain holes.
fn carve_repair_path(
    tiles: &mut TileMap,
    field: TileRect,
    from: (i32, i32),
    to: (i32, i32),
    target: TileRect,
) {
    use crate::game::collision::SizeClass;

    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
    let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
    // A repair is a wide corridor, never a one-cell seam: even the safety net
    // keeps the owner's "no required route narrower than 3 cells" (and a
    // one-cell seam is exactly the choke point an open map must not have).
    const REPAIR_WIDTH: i32 = OPENING_W.0;
    let legs = [
        TileRect::new(x0, from.1 - REPAIR_WIDTH / 2, x1 - x0 + 1, REPAIR_WIDTH),
        TileRect::new(to.0 - REPAIR_WIDTH / 2, y0, REPAIR_WIDTH, y1 - y0 + 1),
    ];
    // The path in travel order, starting inside the orphaned room: along the
    // first row to the elbow, then along the last column to `to`.
    let mut path: Vec<(i32, i32)> = vec![from];
    let step_x = (to.0 - from.0).signum();
    let mut cx = from.0;
    while cx != to.0 {
        cx += step_x;
        path.push((cx, from.1));
    }
    let step_y = (to.1 - from.1).signum();
    let mut cy = from.1;
    while cy != to.1 {
        cy += step_y;
        path.push((to.0, cy));
    }
    let was_rock = |cx: i32, cy: i32| tiles.get(cx, cy) == SOLID;
    // The first run of rock the path crosses, restricted to the orphaned
    // room's own one-cell wall ring: turn *that* into a doorway, so the repair
    // reads as a door instead of a hole. Rock further along is a tunnel through
    // solid mass and stays plain floor — turning all of it into doorways would
    // paint a nine-cell "door" across the map.
    let ring = target.inflate(1);
    let crossing: Vec<(i32, i32)> = path
        .iter()
        .skip_while(|(cx, cy)| !was_rock(*cx, *cy))
        .take_while(|(cx, cy)| was_rock(*cx, *cy))
        .filter(|(cx, cy)| ring.contains(*cx, *cy) && !target.contains(*cx, *cy))
        .copied()
        .collect();

    for leg in &legs {
        carve_if_solid(tiles, *leg, field);
    }
    for (cx, cy) in crossing {
        // The passage axis is decided by which pair of neighbours is open, so a
        // doorway can never be written facing rock. When neither pair is (a
        // corner of a thick wall) the cell stays plain floor: a doorway that
        // leads nowhere is worse than a hole.
        let ns = tiles.passable(cx, cy - 1, SizeClass::Small)
            && tiles.passable(cx, cy + 1, SizeClass::Small);
        let ew = tiles.passable(cx - 1, cy, SizeClass::Small)
            && tiles.passable(cx + 1, cy, SizeClass::Small);
        let axis = match (ns, ew) {
            (true, false) => Some(WallAxis::Horizontal),
            (false, true) => Some(WallAxis::Vertical),
            _ => None,
        };
        if let Some(axis) = axis {
            tiles.set(cx, cy, TileKind::Wall(WallKind::Doorway { axis }));
        }
    }
}

/// The nearest `(target cell, reached cell)` pair for [`connect_orphans`].
///
/// `from` is taken from **inside** the target rectangle. An open cell in the
/// ring around it may be a corridor that merely runs past the room: attaching
/// there would connect the corridor and leave the room itself sealed, and the
/// repair would then repeat the same useless carve forever.
fn nearest_pair(
    tiles: &TileMap,
    seen: &[bool],
    target: TileRect,
    field: TileRect,
) -> Option<((i32, i32), (i32, i32))> {
    use crate::game::collision::SizeClass;
    use crate::game::levelgen::validate;

    let mut from: Option<(i32, i32)> = None;
    for (cx, cy) in target.cells() {
        if !tiles.passable(cx, cy, SizeClass::Small) {
            continue;
        }
        from = Some((cx, cy));
        break;
    }
    let from_cell = from?;
    let mut best: Option<(i32, (i32, i32))> = None;
    for cy in field.y..field.bottom() {
        for cx in field.x..field.right() {
            if !validate::is_reached(seen, tiles, (cx, cy)) {
                continue;
            }
            let d = (cx - from_cell.0).abs() + (cy - from_cell.1).abs();
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, (cx, cy)));
            }
        }
    }
    best.map(|(_, to)| (from_cell, to))
}

/// **Stage A (props).** Furnish every [`RoomKind::Hall`]/[`RoomKind::Room`]
/// with the theme's props, painted into the grid as solid cells.
///
/// A prop's `rect` is cell-aligned and its cells become
/// [`WallKind::Solid`], so what is drawn is exactly what blocks. Props keep a
/// one-cell margin from the room's walls, a one-cell gap from each other and
/// clear of the room's 3 × 3 centre, its maze slots and every doorway's
/// approach.
pub fn furnish(tiles: &mut TileMap, layout: &Layout, rng: &mut StdRng, theme: Theme) -> Vec<Prop> {
    let kinds = theme_props(theme);
    let mut props: Vec<Prop> = Vec::new();
    let mut taken: Vec<TileRect> = Vec::new();
    // Every room's anchor (spawns, traps and tests all use it) stays walkable:
    // rooms merge, so a crate in one hall can land on its neighbour's centre.
    let cores: Vec<TileRect> = layout
        .rooms
        .iter()
        .filter(|other| layout.is_furnishable(other) || other.kind == RoomKind::Start)
        .map(|other| core_3x3(other.rect))
        .collect();
    for room in layout.rooms.iter().filter(|r| layout.is_furnishable(r)) {
        // Keep a one-cell margin so a prop never merges into a wall run.
        let inner = room.rect.inflate(-1);
        if inner.w < 1 || inner.h < 1 {
            continue;
        }
        let count = (room.rect.area() / 90).clamp(2, 9);
        for _ in 0..count {
            let kind = kinds[rng.random_range(0..kinds.len())];
            let (w, h) = kind.cells();
            if w > inner.w || h > inner.h {
                continue;
            }
            for _ in 0..14 {
                let Some(cell) = random_rect(rng, inner, w, h) else {
                    break;
                };
                if cores.iter().any(|core| cell.intersects(core)) {
                    continue;
                }
                if layout
                    .maze_slots
                    .iter()
                    .any(|slot| slot.inflate(1).intersects(&cell))
                {
                    continue;
                }
                if taken.iter().any(|t| t.inflate(1).intersects(&cell)) {
                    continue;
                }
                if !cell.cells().all(|(x, y)| tiles.get(x, y).is_open()) {
                    continue;
                }
                if keeps_openings_clear_within(tiles, cell, PROP_OPENING_CLEAR) {
                    continue;
                }
                tiles.fill_cells(cell.x, cell.y, cell.w, cell.h, SOLID);
                taken.push(cell);
                props.push(Prop {
                    kind,
                    rect: cell.to_world(),
                });
                break;
            }
        }
    }
    // ---- the arena's fight furniture ------------------------------------
    // The boss arena is not a hall: it is the room a specific fight happens in,
    // and each of the three fights needs something from it (`DESIGN_REVIEW.md`
    // §5). Rooms are furnished first so the arena's perimeter props can stay
    // out of every doorway, rat-hole and one-way door (`keeps_openings_clear`).
    furnish_arena_perimeter(tiles, layout, rng, theme, &mut props, &mut taken);
    // Furniture is cover, never a plug: if a prop has pinched an area's only
    // route to one or two cells, open a wide corridor so the owner's "no
    // required route through a passage narrower than 3 cells" still holds on
    // the finished map.
    widen_pinched_areas(
        tiles,
        layout,
        matches!(theme, Theme::Laundry | Theme::Printworks),
    );
    // A widening corridor may have cut through a prop's own cells: that prop is
    // no longer there, so it is not reported (the grid is the truth the art and
    // the collision both read).
    props.retain(|prop| {
        let (w, h) = prop.kind.cells();
        let cell = TileRect::new(
            (prop.rect.x / TILE_F) as i32,
            (prop.rect.y / TILE_F) as i32,
            w,
            h,
        );
        let solid = cell.cells().all(|(cx, cy)| tiles.get(cx, cy).is_wall());
        solid
    });
    props
}

/// Carve up to [`MAX_WIDENINGS`] wide links into any area that furnishing left
/// behind a narrow seam.
fn widen_pinched_areas(tiles: &mut TileMap, layout: &Layout, skip_boss: bool) {
    let field = TileRect::new(
        BORDER,
        BORDER,
        tiles.cols() - 2 * BORDER,
        tiles.rows() - 2 * BORDER,
    );
    let areas: Vec<Area> = layout
        .rooms
        .iter()
        .filter(|room| matches!(room.kind, RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room))
        .map(|room| Area {
            rect: room.rect,
            kind: room.kind,
        })
        .collect();
    let start = (
        (layout.start.x / TILE_F) as i32,
        (layout.start.y / TILE_F) as i32,
    );
    for _ in 0..MAX_WIDENINGS {
        let Some(rect) = narrow_area(tiles, &areas, start, &layout.maze_slots, skip_boss) else {
            break;
        };
        let Some(partner) = nearest_other(tiles, &areas, rect) else {
            break;
        };
        carve_free_link(tiles, field, rect, areas[partner].rect, OPENING_W.0, &areas);
    }
}

// ============================================================================
// Arena furniture
// ============================================================================

/// The clear encounter floor every arena keeps at its centre, in cells.
///
/// - **Kitchen**: the designer's `96 × 80 px` encounter floor
///   (`DESIGN_REVIEW.md` §5) — the human's sweeps are read from it and the
///   pocket under its boots has to stay approachable.
/// - **Sewer**: `96 × 80 px`, the jaguar's pounce landing with room for the
///   escape lane that leads to the rat-hole.
/// - **Lab**: `64 × 48 px` between the two bumpers. It is exactly the strip
///   where both steel bars are within [`SOLID_REACH_PX`] on opposite sides, so
///   every cell of it guarantees the mower a lane that ends on a real solid.
///
/// Every floor below fits inside [`super::placement::in_boss_fight_floor`]'s
/// 6 × 5-cell trap-free fighting floor, so a trap can never sit on it.
pub fn boss_floor_cells(theme: Theme) -> (i32, i32) {
    match theme {
        Theme::Sewer => (6, 5),
        Theme::Kitchen => (6, 5),
        Theme::Lab => (4, BUMPER_LEN),
        // Laundry borrows the broom's arena (a clear 96 x 80 px pocket).
        Theme::Laundry => (6, 5),
        // The Night Edition's press bed: the same clear 96 x 80 px pocket. It
        // is a stamp fight — the sheet is 36 x 30 px and the carriage rule is
        // 54 px long, both drawn at wherever the rat stands — so the fight is
        // won and lost on open floor, and the only furniture is the press bed
        // the art lays under the boss itself.
        Theme::Printworks => (6, 5),
        Theme::Trailer => (6, 5),
    }
}

/// Furnish the boss arena for its fight and return the exported
/// [`BossArena`]. No rng draws: the furniture is a *property* of the arena's
/// geometry, so every seed gets the most deterministic arrangement that fits.
///
/// This runs late in stage A, after every corridor, so the arena's mouth cells
/// are painted and the Sewer's escape duct can reach real floor.
fn furnish_arena(tiles: &mut TileMap, theme: Theme, arena: TileRect, field: TileRect) -> BossArena {
    let (fw, fh) = boss_floor_cells(theme);
    let centre = arena.center_world();
    let centre_cell = ((centre.x / TILE_F) as i32, (centre.y / TILE_F) as i32);
    // Even arena dimensions put the exact centre on a cell boundary: build the
    // floor around the cell that contains it so the centre is always on it.
    let floor = TileRect::new(centre_cell.0 - fw / 2, centre_cell.1 - fh / 2, fw, fh);
    let mouths = arena_mouths(tiles, arena);
    let mut boss = BossArena {
        floor,
        mouths: mouths.clone(),
        ..BossArena::default()
    };
    match theme {
        Theme::Lab => furnish_lab(tiles, arena, floor, &mouths, &mut boss, false),
        Theme::Sewer => {
            boss.escape_hole = carve_escape_hole(tiles, arena, &mouths, field);
        }
        // The Kitchen's requirement is a constraint (clear centre, furniture at
        // the perimeter), enforced by `furnish_arena_perimeter` and the
        // validator: nothing else has to be placed inside the arena.
        Theme::Kitchen => {}
        // Wave 3 routes, same mapping as `BossKind::for_theme`: the Laundry's
        // boss is the broom (a clear pocket, until the Mangle lands) and the
        // Printworks' is the Night Edition, whose arena is a clear press bed.
        // Neither wants the Lab's steel bumpers or its charging dock.
        Theme::Laundry => {}
        Theme::Printworks => {}
        // The flatbed's cab hold is empty plate.
        Theme::Trailer => {}
    }
    boss
}

/// The arena's mouth cells: the doorway cells in its one-cell wall ring.
fn arena_mouths(tiles: &TileMap, arena: TileRect) -> Vec<(i32, i32)> {
    crate::game::levelgen::maze::ring_border(arena.inflate(1))
        .filter(|&(cx, cy)| {
            matches!(
                tiles.get(cx, cy),
                TileKind::Wall(WallKind::Doorway { .. } | WallKind::OneWayDoor { .. })
            )
        })
        .collect()
}

/// The Lab's two steel bumpers and its charging dock.
///
/// The bumpers are 1 × [`BUMPER_LEN`] solid bars immediately west and east of
/// the central floor, so from any floor cell the −x and +x rays both end on a
/// solid (at most `2 × (floor width)` px away, always inside
/// [`SOLID_REACH_PX`]). They are free-standing: a three-cell bar in a 26-cell
/// room can neither block the mouth nor isolate floor, and the validator proves
/// the interior stays connected.
fn furnish_lab(
    tiles: &mut TileMap,
    arena: TileRect,
    floor: TileRect,
    mouths: &[(i32, i32)],
    boss: &mut BossArena,
    sweep: bool,
) {
    let bars = [
        TileRect::new(floor.x - 1, floor.y, 1, BUMPER_LEN),
        TileRect::new(floor.right(), floor.y, 1, BUMPER_LEN),
    ];
    for bar in bars {
        // Only a bar that sits wholly on arena floor is a bumper; anything else
        // would be a property the validator cannot promise.
        if !bars_fit(tiles, bar, arena, floor) {
            continue;
        }
        tiles.fill_cells(bar.x, bar.y, bar.w, bar.h, SOLID);
        boss.bumpers.push(bar);
    }
    if let Some((cell, pos)) = lab_dock(tiles, arena, floor, mouths, sweep) {
        boss.dock = Some(pos);
        boss.dock_cell = Some(cell);
    }
}

/// Is `bar` a legal bumper: inside the arena, clear of the floor, and with the
/// framework's 32 px side lane open along at least one long side?
fn bars_fit(tiles: &TileMap, bar: TileRect, arena: TileRect, floor: TileRect) -> bool {
    if !within(bar, arena) || bar.intersects(&floor) {
        return false;
    }
    // The impact lane runs along the bar's long face; the framework probes a
    // 32 px escape segment from a chassis 13 px off that face, so the two cells
    // beside the bar have to be floor for the whole bar length.
    let lane_cells = (BUMPER_SIDE_LANE_PX / TILE_F).ceil() as i32;
    [-1, 1].iter().any(|&side| {
        (0..lane_cells).all(|k| {
            (0..bar.h).all(|row| {
                let cx = if side < 0 {
                    bar.x - 1 - k
                } else {
                    bar.right() + k
                };
                tiles.get(cx, bar.y + row).is_open()
            })
        })
    })
}

/// The Lab's charging dock: a floor cell `DOCK_WALL_INSET` cells in from the
/// wall ring on the side farthest from the mouth, with the art's 32 × 30 px pad
/// footprint clear.
fn lab_dock(
    tiles: &TileMap,
    arena: TileRect,
    floor: TileRect,
    mouths: &[(i32, i32)],
    sweep: bool,
) -> Option<((i32, i32), Vec2)> {
    let mid_x = arena.x + arena.w / 2;
    let mid_y = arena.y + arena.h / 2;
    let candidates = [
        (mid_x, arena.y + DOCK_WALL_INSET - 1),          // north wall
        (arena.right() - DOCK_WALL_INSET, mid_y),        // east wall
        (mid_x, arena.bottom() - DOCK_WALL_INSET),       // south wall
        (arena.x + DOCK_WALL_INSET - 1, mid_y),          // west wall
    ];
    let mut best: Option<(i32, (i32, i32), Vec2)> = None;
    for (cx, cy) in candidates {
        if !arena.contains(cx, cy) || floor.contains(cx, cy) {
            continue;
        }
        if !tiles.get(cx, cy).is_open() {
            continue;
        }
        let pos = TileMap::tile_rect(cx, cy).center();
        let pad = Rectf::new(
            pos.x - crate::game::art_boss_mower::DOCK_HALF_SIZE.x,
            pos.y - crate::game::art_boss_mower::DOCK_HALF_SIZE.y,
            2.0 * crate::game::art_boss_mower::DOCK_HALF_SIZE.x,
            2.0 * crate::game::art_boss_mower::DOCK_HALF_SIZE.y,
        );
        if !tiles.rect_clear(pad, crate::game::collision::SizeClass::Large) {
            continue;
        }
        let gap = mouths
            .iter()
            .map(|&(mx, my)| (mx - cx).abs().max((my - cy).abs()))
            .min()
            .unwrap_or(i32::MAX);
        if gap < DOCK_MOUTH_CLEAR {
            continue;
        }
        if best.is_none_or(|(bg, _, _)| gap > bg) {
            best = Some((gap, (cx, cy), pos));
        }
    }
    if best.is_none() && sweep {
        // The wave 3 floors (`sweep`) never ship an arena without its dock: if a
        // corridor happens to open one of the four wall midpoints, the whole
        // perimeter band is searched in a deterministic order for any cell that
        // satisfies the same constraints. The original three themes keep the
        // four-candidate behaviour byte for byte.
        let band = ARENA_PERIMETER_CELLS;
        for cy in (arena.y + 1)..(arena.bottom() - 1) {
            for cx in (arena.x + 1)..(arena.right() - 1) {
                let on_band = cx < arena.x + band
                    || cx >= arena.right() - band
                    || cy < arena.y + band
                    || cy >= arena.bottom() - band;
                if !on_band || floor.contains(cx, cy) {
                    continue;
                }
                if !tiles.get(cx, cy).is_open() {
                    continue;
                }
                let pos = TileMap::tile_rect(cx, cy).center();
                let pad = Rectf::new(
                    pos.x - crate::game::art_boss_mower::DOCK_HALF_SIZE.x,
                    pos.y - crate::game::art_boss_mower::DOCK_HALF_SIZE.y,
                    2.0 * crate::game::art_boss_mower::DOCK_HALF_SIZE.x,
                    2.0 * crate::game::art_boss_mower::DOCK_HALF_SIZE.y,
                );
                if !tiles.rect_clear(pad, crate::game::collision::SizeClass::Large) {
                    continue;
                }
                let gap = mouths
                    .iter()
                    .map(|&(mx, my)| (mx - cx).abs().max((my - cy).abs()))
                    .min()
                    .unwrap_or(i32::MAX);
                if gap < DOCK_MOUTH_CLEAR {
                    continue;
                }
                if best.is_none_or(|(bg, _, _)| gap > bg) {
                    best = Some((gap, (cx, cy), pos));
                }
            }
        }
    }
    best.map(|(_, cell, pos)| (cell, pos))
}

/// The Sewer's escape rat-hole: a hole in the arena wall ring that leads, via a
/// short duct, onto real floor outside.
///
/// The jaguar's pounce is answered by ducking through a hole so the cat
/// overshoots (`DESIGN_REVIEW.md` 5). The hole is chosen deterministically:
///
/// 1. only a solid ring cell at least [`ESCAPE_MOUTH_CLEAR`] cells from the
///    mouth, with arena floor open behind it, is a candidate;
/// 2. a multi-source breadth-first search runs from every candidate outward
///    through bare bedrock (never along the ring, never through a threshold) to
///    the nearest open floor;
/// 3. the nearest candidate wins and the search path is carved as a one-cell
///    duct.
///
/// A straight outward ray is not enough: the arena is the map's outermost room
/// by design, so on some seeds every ray leaves the field or runs 30+ cells
/// before it hits floor, while a duct that turns once reaches the mouth corridor
/// in a handful of cells.
fn carve_escape_hole(
    tiles: &mut TileMap,
    arena: TileRect,
    mouths: &[(i32, i32)],
    field: TileRect,
) -> Option<(i32, i32)> {
    let ring = arena.inflate(1);
    let cols = tiles.cols();
    let index = |cx: i32, cy: i32| (cy * cols + cx) as usize;

    // 1. Candidate ring cells: a solid side cell, clear of the mouth, with the
    // arena's own floor open behind it.
    let mut sources: Vec<(i32, i32)> = Vec::new();
    for (cx, cy) in crate::game::levelgen::maze::ring_border(ring) {
        let vertical_wall = cx == ring.x || cx == ring.right() - 1;
        let horizontal_wall = cy == ring.y || cy == ring.bottom() - 1;
        if vertical_wall == horizontal_wall || !tiles.is_full_solid(cx, cy) {
            continue; // a corner, or a mouth/doorway/hole already lives here
        }
        let gap = mouths
            .iter()
            .map(|&(mx, my)| (mx - cx).abs().max((my - cy).abs()))
            .min()
            .unwrap_or(i32::MAX);
        if gap < ESCAPE_MOUTH_CLEAR {
            continue;
        }
        let (dx, dy) = if vertical_wall {
            (if cx == ring.x { -1 } else { 1 }, 0)
        } else {
            (0, if cy == ring.y { -1 } else { 1 })
        };
        let inner = (cx - dx, cy - dy);
        if !arena.contains(inner.0, inner.1) || !tiles.get(inner.0, inner.1).is_open() {
            continue;
        }
        sources.push((cx, cy));
    }
    if sources.is_empty() {
        return None;
    }

    // 2. BFS outward through bedrock. The ring itself is excluded so a duct can
    // never run along the arena wall, and only `Solid` cells are walked so no
    // doorway, one-way door, rat-hole or maze partition is ever eaten.
    let cells = (cols * tiles.rows()) as usize;
    let mut parent: Vec<i32> = vec![-1; cells];
    let mut seen = vec![false; cells];
    let mut queue: std::collections::VecDeque<(i32, i32, i32)> = std::collections::VecDeque::new();
    for &(cx, cy) in &sources {
        let i = index(cx, cy);
        seen[i] = true;
        parent[i] = -2; // a source: the path ends here
        queue.push_back((cx, cy, 0));
    }
    let mut landing: Option<(i32, i32)> = None;
    while let Some((cx, cy, depth)) = queue.pop_front() {
        if depth > ESCAPE_DUCT_MAX {
            break; // BFS pops by depth, so every remaining path is longer
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx + dx, cy + dy);
            if !field.contains(nx, ny) || ring.contains(nx, ny) || seen[index(nx, ny)] {
                continue;
            }
            match tiles.get(nx, ny) {
                TileKind::Open => {
                    landing = Some((cx, cy));
                    break;
                }
                SOLID => {
                    let i = index(nx, ny);
                    seen[i] = true;
                    parent[i] = index(cx, cy) as i32;
                    queue.push_back((nx, ny, depth + 1));
                }
                _ => {}
            }
        }
        if landing.is_some() {
            break;
        }
    }
    let mut cur = landing?;

    // 3. Walk the parents back to the chosen ring cell and carve the path.
    let mut duct = Vec::new();
    loop {
        let p = parent[index(cur.0, cur.1)];
        if p == -2 {
            break;
        }
        duct.push(cur);
        cur = ((p as usize % cols as usize) as i32, (p as usize / cols as usize) as i32);
    }
    duct.reverse();
    let cell = cur;
    for (nx, ny) in duct {
        tiles.set(nx, ny, TileKind::Open);
    }
    let vertical_wall = cell.0 == ring.x || cell.0 == ring.right() - 1;
    let axis = if vertical_wall {
        WallAxis::Vertical
    } else {
        WallAxis::Horizontal
    };
    tiles.set(cell.0, cell.1, TileKind::Wall(WallKind::RatHole { axis }));
    Some(cell)
}

/// Is every open cell of the arena interior reachable from its centre for the
/// small class? The promise "never isolating any floor" is exactly this, and it
/// is cheap at arena size (≈ 900 cells).
fn arena_interior_connected(tiles: &TileMap, arena: TileRect) -> bool {
    use crate::game::collision::SizeClass;
    let centre = arena.center_world();
    let start = ((centre.x / TILE_F) as i32, (centre.y / TILE_F) as i32);
    if !tiles.passable(start.0, start.1, SizeClass::Small) {
        return false;
    }
    let mut seen = vec![false; (arena.w * arena.h) as usize];
    let index = |cx: i32, cy: i32| ((cy - arena.y)
        * arena.w
        + (cx - arena.x)) as usize;
    seen[index(start.0, start.1)] = true;
    let mut queue = vec![start];
    while let Some((cx, cy)) = queue.pop() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx + dx, cy + dy);
            if !arena.contains(nx, ny) || !tiles.passable(nx, ny, SizeClass::Small) {
                continue;
            }
            let i = index(nx, ny);
            if !seen[i] {
                seen[i] = true;
                queue.push((nx, ny));
            }
        }
    }
    arena.cells().all(|(cx, cy)| {
        !tiles.passable(cx, cy, SizeClass::Small) || seen[index(cx, cy)]
    })
}

/// Furniture against the arena's walls: the Kitchen's barrels and counter ends,
/// the Sewer's pipes and barriers.
///
/// The owner's rule is "barrels or counter ends only at the perimeter" (§5), so
/// a prop has to sit in the outer [`ARENA_PERIMETER_CELLS`] band of the arena,
/// clear of the central floor's approach, the dock, the escape hole and every
/// opening. A placement that would isolate arena floor is reverted, so the
/// fight floor is connected by construction, not by luck.
fn furnish_arena_perimeter(
    tiles: &mut TileMap,
    layout: &Layout,
    rng: &mut StdRng,
    theme: Theme,
    props: &mut Vec<Prop>,
    taken: &mut Vec<TileRect>,
) {
    let kinds: &[crate::game::level::PropKind] = match theme {
        Theme::Kitchen => &[
            crate::game::level::PropKind::Barrel,
            crate::game::level::PropKind::Counter,
        ],
        Theme::Sewer => &[
            crate::game::level::PropKind::Barrier,
            crate::game::level::PropKind::PipeV,
            crate::game::level::PropKind::PipeH,
        ],
        // The Lab's furniture is its bumpers and dock; nothing else competes
        // with the charge lanes.
        Theme::Lab => return,
        // The Laundry dresses its arena with linen trolleys and white goods.
        Theme::Laundry => &[
            crate::game::level::PropKind::TowelRoll,
            crate::game::level::PropKind::Washer,
        ],
        // The Printworks borrows the mower's arena: nothing may compete with
        // its charge lanes.
        Theme::Printworks => return,
        // The flatbed furnishes its deck from its own generator.
        Theme::Trailer => return,
    };
    let arena = layout.boss_arena;
    if arena.w < 8 || arena.h < 8 {
        return;
    }
    let count = 2 + rng.random_range(0..=1);
    for _ in 0..count {
        let kind = kinds[rng.random_range(0..kinds.len())];
        // The room scatter runs first, so its props are already on the map:
        // an arena prop keeps the same one-cell gap from them (a pipe against
        // a pipe reads as one wall, not two props).
        let candidates: Vec<TileRect> = arena_prop_sites(tiles, layout, arena, kind)
            .into_iter()
            .filter(|rect| !taken.iter().any(|other| other.inflate(1).intersects(rect)))
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let rect = candidates[rng.random_range(0..candidates.len())];
        let before: Vec<TileKind> = rect.cells().map(|(x, y)| tiles.get(x, y)).collect();
        tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, SOLID);
        if !arena_interior_connected(tiles, arena) {
            for (i, (cx, cy)) in rect.cells().enumerate() {
                tiles.set(cx, cy, before[i]);
            }
            continue;
        }
        props.push(Prop {
            kind,
            rect: rect.to_world(),
        });
        taken.push(rect);
    }
}

/// Every cell-aligned site in the arena's perimeter band where `kind` may go.
///
/// A site is inside the arena, wholly within the outer
/// [`ARENA_PERIMETER_CELLS`] band, clear of the central floor's approach ring,
/// the dock and the escape hole, and never in front of an opening. Placed props
/// become `Solid`, so later candidates see them as occupied through the
/// "all cells open" test.
fn arena_prop_sites(
    tiles: &TileMap,
    layout: &Layout,
    arena: TileRect,
    kind: crate::game::level::PropKind,
) -> Vec<TileRect> {
    let (w, h) = kind.cells();
    let mut sites = Vec::new();
    if w > arena.w || h > arena.h {
        return sites;
    }
    // Only the outer band qualifies: anything that touches this core is out.
    let core = arena.inflate(-ARENA_PERIMETER_CELLS);
    let clear_of_floor = layout.boss.floor.inflate(1);
    for cy in arena.y..=arena.bottom() - h {
        for cx in arena.x..=arena.right() - w {
            let rect = TileRect::new(cx, cy, w, h);
            if rect.intersects(&core) || rect.inflate(1).intersects(&clear_of_floor) {
                continue;
            }
            if layout
                .boss
                .dock_cell
                .is_some_and(|d| rect.inflate(3).contains(d.0, d.1))
            {
                continue;
            }
            if layout
                .boss
                .escape_hole
                .is_some_and(|hole| rect.inflate(2).contains(hole.0, hole.1))
            {
                continue;
            }
            if layout
                .boss
                .bumpers
                .iter()
                .any(|bar| rect.inflate(1).intersects(bar))
            {
                continue;
            }
            if !rect.cells().all(|(x, y)| tiles.get(x, y).is_open()) {
                continue;
            }
            if keeps_openings_clear(tiles, rect) {
                continue;
            }
            sites.push(rect);
        }
    }
    sites
}

/// A random top-left cell for a `w × h` block inside `within`, or `None` when it
/// cannot fit.
pub fn random_cell(rng: &mut StdRng, within: TileRect, w: i32, h: i32) -> Option<(i32, i32)> {
    let max_x = within.right() - w;
    let max_y = within.bottom() - h;
    if max_x < within.x || max_y < within.y {
        return None;
    }
    Some((
        rng.random_range(within.x..=max_x),
        rng.random_range(within.y..=max_y),
    ))
}

/// [`random_cell`] as a rectangle.
fn random_rect(rng: &mut StdRng, within: TileRect, w: i32, h: i32) -> Option<TileRect> {
    let (x, y) = random_cell(rng, within, w, h)?;
    Some(TileRect::new(x, y, w, h))
}

/// Collect every `w × h` rectangle inside `bounds` whose `margin`-cell ring is
/// still inside `bounds` and that satisfies `ok`, then pick one uniformly.
///
/// Candidate-list picking (rather than unbounded rejection sampling) means the
/// number of rng draws does not depend on how lucky the first samples were, and
/// a stage can never loop forever on an unlucky seed.
fn pick_rect(
    rng: &mut StdRng,
    bounds: TileRect,
    margin: i32,
    w: i32,
    h: i32,
    step: i32,
    ok: impl Fn(TileRect) -> bool,
) -> Option<TileRect> {
    let x0 = bounds.x + margin;
    let y0 = bounds.y + margin;
    let x1 = bounds.right() - margin - w;
    let y1 = bounds.bottom() - margin - h;
    if x1 < x0 || y1 < y0 {
        return None;
    }
    let step = step.max(1);
    let mut candidates: Vec<TileRect> = Vec::new();
    let mut y = y0;
    while y <= y1 {
        let mut x = x0;
        while x <= x1 {
            let rect = TileRect::new(x, y, w, h);
            if ok(rect) {
                candidates.push(rect);
            }
            x += step;
        }
        y += step;
    }
    if candidates.is_empty() {
        return None;
    }
    Some(candidates[rng.random_range(0..candidates.len())])
}

/// Paint `rect` as open floor.
fn carve(tiles: &mut TileMap, rect: TileRect) {
    tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, TileKind::Open);
}

/// Open `rect` but **only** where the cell is still bare bedrock: a corridor
/// must never eat a doorway, a maze partition or a rat-hole another stage (or
/// an earlier corridor) already placed.
fn carve_if_solid(tiles: &mut TileMap, rect: TileRect, field: TileRect) {
    let r = clip(rect, field);
    for (cx, cy) in r.cells() {
        if tiles.get(cx, cy) == SOLID {
            tiles.set(cx, cy, TileKind::Open);
        }
    }
}

/// Clip a rectangle to `bounds` (empty when they do not overlap).
fn clip(r: TileRect, bounds: TileRect) -> TileRect {
    let x0 = r.x.max(bounds.x);
    let y0 = r.y.max(bounds.y);
    let x1 = r.right().min(bounds.right());
    let y1 = r.bottom().min(bounds.bottom());
    TileRect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
}

/// Is `inner` entirely inside `outer`?
fn within(inner: TileRect, outer: TileRect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

/// Cells a prop (or a piece of cover) keeps clear of an opening, so it can
/// never narrow the passage: the owner's "no required route through a passage
/// narrower than 3 cells".
const PROP_OPENING_CLEAR: i32 = 3;

/// Looks at a prop's neighbourhood: a prop must never narrow a doorway, a
/// rat-hole or a one-way door, and never sit right in front of one.
fn keeps_openings_clear(tiles: &TileMap, cell: TileRect) -> bool {
    keeps_openings_clear_within(tiles, cell, 2)
}

/// [`keeps_openings_clear`] with an explicit clearance in cells.
fn keeps_openings_clear_within(tiles: &TileMap, cell: TileRect, clear: i32) -> bool {
    for (cx, cy) in cell.inflate(clear).cells() {
        if matches!(
            tiles.get(cx, cy),
            TileKind::Wall(
                WallKind::Doorway { .. } | WallKind::RatHole { .. } | WallKind::OneWayDoor { .. }
            )
        ) {
            return true;
        }
    }
    false
}

/// Chebyshev gap in cells between a cell and the nearest cell of `rect` (`0`
/// when the cell is inside it).
fn cell_point_gap(cell: (i32, i32), rect: TileRect) -> i32 {
    let dx = (rect.x - cell.0).max(cell.0 - (rect.right() - 1)).max(0);
    let dy = (rect.y - cell.1).max(cell.1 - (rect.bottom() - 1)).max(0);
    dx.max(dy)
}

/// Is `rect` inside the map field (what `build_layout` guarantees)?
#[cfg(test)]
fn in_field(rect: TileRect, cols: i32, rows: i32) -> bool {
    within(rect, TileRect::new(BORDER, BORDER, cols - 2 * BORDER, rows - 2 * BORDER))
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::game::collision::SizeClass;
    use crate::game::level::{level_rng, MAP_COLS, MAP_ROWS, MAP_SIZE};
    use crate::game::levelgen::maze;
    use crate::game::levelgen::validate;
    use crate::game::levelgen::validate::{is_reached, reachable};

    fn layout_for(theme: Theme, seed: u64) -> (TileMap, Layout) {
        let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, 1);
        let mut rng = level_rng(theme, seed, 0);
        let layout = build_layout(&mut tiles, &mut rng, theme);
        (tiles, layout)
    }

    /// Every wall cell, in row-major order — two layouts are equal iff their
    /// wall lists are (open cells are the complement).
    fn walls(tiles: &TileMap) -> Vec<(i32, i32, WallKind)> {
        tiles.iter_walls().collect()
    }

    /// Is `(cx, cy)` a one-cell wall a rat-hole could pierce: open on both
    /// sides of one axis, wall on both sides of the other?
    fn hole_candidate(tiles: &TileMap, cx: i32, cy: i32) -> bool {
        if !matches!(tiles.get(cx, cy), TileKind::Wall(WallKind::Solid)) {
            return false;
        }
        let ns = tiles.get(cx, cy - 1).is_open() && tiles.get(cx, cy + 1).is_open();
        let ew = tiles.get(cx - 1, cy).is_open() && tiles.get(cx + 1, cy).is_open();
        (ns && !ew) || (ew && !ns)
    }

    #[test]
    fn layout_invariants_hold_on_every_map() {
        for theme in Theme::all() {
            for seed in 0..30u64 {
                let (tiles, layout) = layout_for(theme, seed);
                let ctx = format!("{theme:?} seed {seed}");
                let field = field_of(&tiles);

                // Start: centred, clear, first in the list, and big.
                assert_eq!(layout.start, MAP_SIZE * 0.5, "{ctx}");
                assert_eq!(layout.start_room.center_world(), layout.start, "{ctx}");
                assert_eq!(layout.rooms[0].kind, RoomKind::Start, "{ctx}");
                assert_eq!(layout.rooms[0].rect, layout.start_room, "{ctx}");
                assert!(
                    layout.start_room.area() >= 1000,
                    "{ctx}: the start area is only {} cells",
                    layout.start_room.area()
                );
                let spawn3 = TileRect::new(
                    layout.start.x as i32 / 16 - 1,
                    layout.start.y as i32 / 16 - 1,
                    3,
                    3,
                );
                assert!(
                    spawn3.cells().all(|(x, y)| tiles.get(x, y).is_open()),
                    "{ctx}: the 3x3 around the spawn is not clear"
                );

                // Exactly one boss arena, big, with a clear central floor.
                let bosses: Vec<&Room> = layout
                    .rooms
                    .iter()
                    .filter(|r| r.kind == RoomKind::Boss)
                    .collect();
                assert_eq!(bosses.len(), 1, "{ctx}");
                assert_eq!(bosses[0].rect, layout.boss_arena, "{ctx}");
                assert!(
                    layout.boss_arena.area() >= BOSS_ARENA_CELLS,
                    "{ctx}: the boss arena is {} cells",
                    layout.boss_arena.area()
                );
                assert!(
                    layout
                        .boss
                        .floor
                        .cells()
                        .all(|(x, y)| tiles.get(x, y).is_open()),
                    "{ctx}: the arena's central floor is not clear"
                );
                let core = layout.boss_arena.inflate(-ARENA_PERIMETER_CELLS);
                for (x, y) in layout.boss_arena.cells() {
                    if tiles.get(x, y).is_open() {
                        continue;
                    }
                    let bumper = layout.boss.bumpers.iter().any(|b| b.contains(x, y));
                    assert!(
                        bumper || !core.contains(x, y),
                        "{ctx}: an unfurnished solid at ({x}, {y}) inside the arena"
                    );
                }

                // The solid border is at least two cells thick.
                for cx in 0..MAP_COLS {
                    for ring in 0..BORDER {
                        assert!(tiles.get(cx, ring).is_wall(), "{ctx}: top border hole");
                        assert!(
                            tiles.get(cx, MAP_ROWS - 1 - ring).is_wall(),
                            "{ctx}: bottom border hole"
                        );
                    }
                }
                for cy in 0..MAP_ROWS {
                    for ring in 0..BORDER {
                        assert!(tiles.get(ring, cy).is_wall(), "{ctx}: left border hole");
                        assert!(
                            tiles.get(MAP_COLS - 1 - ring, cy).is_wall(),
                            "{ctx}: right border hole"
                        );
                    }
                }

                // The open world: half the grid is floor, and it is one place.
                let open = (0..MAP_ROWS)
                    .flat_map(|cy| (0..MAP_COLS).map(move |cx| (cx, cy)))
                    .filter(|&(cx, cy)| tiles.passable(cx, cy, SizeClass::Small))
                    .count();
                let fraction = open as f64 / (MAP_COLS * MAP_ROWS) as f64;
                assert!(
                    (0.38..=0.62).contains(&fraction),
                    "{ctx}: {:.0} % of the grid is walkable",
                    fraction * 100.0
                );

                // A few great halls and medium rooms, all inside the field.
                let mut named = 0;
                let mut biggest = 0;
                let mut halls = 0;
                let mut plain = 0;
                let solid_rooms: Vec<&Room> = layout
                    .rooms
                    .iter()
                    .filter(|r| !matches!(r.kind, RoomKind::Corridor | RoomKind::Maze))
                    .collect();
                for room in &solid_rooms {
                    assert!(
                        in_field(room.rect, MAP_COLS, MAP_ROWS),
                        "{ctx}: {:?} {:?} leaves the field",
                        room.kind,
                        room.rect
                    );
                    assert!(room.rect.area() > 0, "{ctx}: empty room");
                    named += 1;
                    biggest = biggest.max(room.rect.area());
                    match room.kind {
                        RoomKind::Hall => halls += 1,
                        RoomKind::Room => plain += 1,
                        _ => {}
                    }
                }
                assert!(named >= 8, "{ctx}: only {named} areas");
                assert!(halls >= 1 && plain >= 1, "{ctx}: no plaza or no medium room");
                assert!(
                    biggest >= 900,
                    "{ctx}: the largest area is only {biggest} cells"
                );

                // Corridors: the exception, and never a squeeze.
                for room in layout.rooms.iter().filter(|r| r.kind == RoomKind::Corridor) {
                    // A gallery is `opening x wall thickness`: its passage is
                    // the longer side.
                    let passage = room.rect.w.max(room.rect.h);
                    assert!(
                        passage >= OPENING_W.0,
                        "{ctx}: corridor {:?} is only {passage} cells wide",
                        room.rect
                    );
                    assert!(
                        room.rect.w.max(room.rect.h) <= 64,
                        "{ctx}: corridor {:?} is a long hallway",
                        room.rect
                    );
                }

                // The maze slot: open floor with an open ring around it, near
                // the spawn, recorded as a maze room.
                assert!(layout.maze_slots.len() <= 1, "{ctx}");
                for slot in &layout.maze_slots {
                    assert!(
                        within(slot.inflate(2), field),
                        "{ctx}: maze slot {slot:?} is too close to the border"
                    );
                    assert!(
                        slot.inflate(1).cells().all(|(x, y)| tiles.get(x, y).is_open()),
                        "{ctx}: the ring around maze slot {slot:?} is not floor"
                    );
                    assert!(
                        layout
                            .rooms
                            .iter()
                            .any(|r| r.kind == RoomKind::Maze && r.rect == *slot),
                        "{ctx}: maze slot {slot:?} has no RoomKind::Maze record"
                    );
                    let start_cell = validate::cell_of(layout.start);
                    assert!(
                        cell_point_gap(start_cell, *slot) <= MAZE_NEAR_CELLS,
                        "{ctx}: maze slot {slot:?} is not near the spawn"
                    );
                }

                // Every doorway opens onto floor on both of its passage sides.
                for (cx, cy, kind) in tiles.iter_walls() {
                    let WallKind::Doorway { axis } = kind else {
                        continue;
                    };
                    let sides: [(i32, i32); 2] = match axis {
                        WallAxis::Horizontal => [(cx, cy - 1), (cx, cy + 1)],
                        WallAxis::Vertical => [(cx - 1, cy), (cx + 1, cy)],
                    };
                    for (sx, sy) in sides {
                        assert!(
                            tiles.passable(sx, sy, SizeClass::Small),
                            "{ctx}: doorway ({cx},{cy}) faces a wall at ({sx},{sy})"
                        );
                    }
                }

                // Connectivity, exactly as the validator proves it.
                let start = validate::cell_of(layout.start);
                let small = reachable(&tiles, start, SizeClass::Small);
                let large = reachable(&tiles, start, SizeClass::Large);
                for room in &layout.rooms {
                    assert!(
                        validate::reaches_rect(&small, &tiles, room.rect),
                        "{ctx}: the rat cannot reach the {:?} {:?}",
                        room.kind,
                        room.rect
                    );
                    if matches!(room.kind, RoomKind::Boss | RoomKind::Hall | RoomKind::Room) {
                        assert!(
                            validate::reaches_rect(&large, &tiles, room.rect),
                            "{ctx}: a large enemy cannot reach the {:?} {:?}",
                        room.kind,
                            room.rect
                        );
                    }
                }

                // Stage B needs thin walls to pierce: the open world grows its
                // own partitions for exactly that.
                assert!(
                    !layout.partitions.is_empty(),
                    "{ctx}: the map has no free-standing partition for a rat-hole"
                );
                for partition in &layout.partitions {
                    assert_eq!(
                        partition.w.min(partition.h),
                        1,
                        "{ctx}: partition {partition:?} is thicker than a cell"
                    );
                    assert!(
                        partition
                            .cells()
                            .all(|(x, y)| matches!(tiles.get(x, y), TileKind::Wall(WallKind::Solid))),
                        "{ctx}: partition {partition:?} is not painted"
                    );
                }
                let candidates = (0..MAP_ROWS)
                    .flat_map(|cy| (0..MAP_COLS).map(move |cx| (cx, cy)))
                    .filter(|&(cx, cy)| hole_candidate(&tiles, cx, cy))
                    .count();
                assert!(
                    candidates >= 2,
                    "{ctx}: only {candidates} one-cell walls for a rat-hole"
                );
            }
        }
    }

    fn field_of(tiles: &TileMap) -> TileRect {
        TileRect::new(
            BORDER,
            BORDER,
            tiles.cols() - 2 * BORDER,
            tiles.rows() - 2 * BORDER,
        )
    }

    #[test]
    fn generation_is_deterministic_and_seed_sensitive() {
        for theme in Theme::all() {
            let (a_tiles, a) = layout_for(theme, 7);
            let (b_tiles, b) = layout_for(theme, 7);
            assert_eq!(walls(&a_tiles), walls(&b_tiles), "{theme:?}: same seed, different map");
            assert_eq!(a.start, b.start);
            assert_eq!(a.boss_arena, b.boss_arena);
            assert_eq!(a.maze_slots.len(), b.maze_slots.len());
            let (c_tiles, _) = layout_for(theme, 8);
            assert_ne!(
                walls(&a_tiles),
                walls(&c_tiles),
                "{theme:?}: two seeds made the same map"
            );
        }
    }

    #[test]
    fn furnish_paints_props_inside_rooms_and_never_in_maze_slots() {
        for theme in Theme::all() {
            for seed in 0..6u64 {
                let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, 1);
                let mut rng = level_rng(theme, seed, 0);
                let layout = build_layout(&mut tiles, &mut rng, theme);
                let props = furnish(&mut tiles, &layout, &mut rng, theme);
                assert!(!props.is_empty(), "{theme:?} seed {seed}: nothing furnished");

                let mut cells: Vec<TileRect> = Vec::new();
                for p in &props {
                    let (w, h) = p.kind.cells();
                    let cell = TileRect::new(
                        (p.rect.x / TILE_F) as i32,
                        (p.rect.y / TILE_F) as i32,
                        w,
                        h,
                    );
                    // Painted solid, and inside a furnishable room or the boss
                    // arena's perimeter band.
                    assert!(cell.cells().all(|(x, y)| tiles.get(x, y).is_wall()));
                    let in_room = layout.rooms.iter().any(|r| {
                        layout.is_furnishable(r) && r.rect.inflate(1).contains(cell.x, cell.y)
                    });
                    let in_arena_band = layout.boss_arena.contains(cell.x, cell.y)
                        && !layout
                            .boss_arena
                            .inflate(-ARENA_PERIMETER_CELLS)
                            .intersects(&cell);
                    assert!(
                        in_room || in_arena_band,
                        "{theme:?} seed {seed}: a prop at {cell:?} is not in a room or at the arena's perimeter"
                    );
                    // Never in a maze slot, never adjacent to another prop.
                    assert!(!layout.maze_slots.iter().any(|s| s.intersects(&cell)));
                    if let Some(neighbour) = cells.iter().find(|c| c.inflate(1).intersects(&cell)) {
                        panic!(
                            "{theme:?} seed {seed}: props {neighbour:?} and {cell:?} are adjacent"
                        );
                    }
                    cells.push(cell);
                }

                // Every room centre (3x3) and every opening is still clear —
                // except a room a maze was carved into, whose centre is inside
                // the warren by design.
                for room in layout.rooms.iter().filter(|r| {
                    layout.is_furnishable(r)
                        && !layout.maze_slots.iter().any(|slot| slot.intersects(&r.rect))
                }) {
                    let core = TileRect::new(
                        room.rect.x + room.rect.w / 2 - 1,
                        room.rect.y + room.rect.h / 2 - 1,
                        3,
                        3,
                    );
                    let blocked: Vec<(i32, i32, TileKind)> = core
                        .cells()
                        .filter(|&(x, y)| !tiles.get(x, y).is_open())
                        .map(|(x, y)| (x, y, tiles.get(x, y)))
                        .collect();
                    assert!(
                        blocked.is_empty(),
                        "{theme:?} seed {seed}: the {:?} {:?} centre is blocked at {blocked:?}",
                        room.kind,
                        room.rect
                    );
                }
                for (cx, cy, kind) in tiles.iter_walls() {
                    if !matches!(
                        kind,
                        WallKind::Doorway { .. } | WallKind::RatHole { .. } | WallKind::OneWayDoor { .. }
                    ) {
                        continue;
                    }
                    // No prop may sit within two cells of an opening: a prop
                    // in front of a door turns the door into a dead end.
                    let near = TileRect::new(cx - 2, cy - 2, 5, 5);
                    assert!(
                        !cells.iter().any(|c| c.intersects(&near)),
                        "{theme:?} seed {seed}: a prop blocks the opening ({cx},{cy})"
                    );
                }

                // Furnishing never seals a room off.
                let start = validate::cell_of(layout.start);
                let small = reachable(&tiles, start, SizeClass::Small);
                for room in &layout.rooms {
                    assert!(
                        validate::reaches_rect(&small, &tiles, room.rect),
                        "{theme:?} seed {seed}: props sealed the {:?} {:?}",
                        room.kind,
                        room.rect
                    );
                }
                assert!(is_reached(&small, &tiles, start));
            }
        }
    }

    /// The later stages may carve mazes, pierce rat-holes and scatter traps,
    /// but they must never make a place the layout promised unreachable. This
    /// is A's half of the playability gate (stage B's `validate` adds the
    /// maze-pocket and trap rules).
    #[test]
    fn later_stages_keep_the_layout_playable() {
        for theme in Theme::all() {
            for seed in 0..8u64 {
                let level = crate::game::levelgen::generate_once(theme, seed, 0);
                let ctx = format!("{theme:?} seed {seed}");
                assert_eq!(level.start, MAP_SIZE * 0.5, "{ctx}");
                assert_eq!(level.arena, MAP_SIZE, "{ctx}");
                assert_eq!(
                    level.rooms.iter().filter(|r| r.kind == RoomKind::Boss).count(),
                    1,
                    "{ctx}"
                );
                let Some(boss) = level.boss_room() else {
                    panic!("{ctx}: no boss arena");
                };
                assert!(boss.rect.area() >= BOSS_ARENA_CELLS, "{ctx}");
                let start_room_ok = level
                    .start_room()
                    .is_some_and(|r| r.rect.contains_world(level.start));
                assert!(start_room_ok, "{ctx}: no start room at the spawn point");
                assert!(
                    level.tiles.circle_clear(level.start, 3.0, SizeClass::Small),
                    "{ctx}: the spawn point is not wall clear"
                );

                let start = validate::cell_of(level.start);
                let small = reachable(&level.tiles, start, SizeClass::Small);
                let large = reachable(&level.tiles, start, SizeClass::Large);
                for room in &level.rooms {
                    assert!(
                        validate::reaches_rect(&small, &level.tiles, room.rect),
                        "{ctx}: the rat cannot reach the {:?} {:?}",
                        room.kind,
                        room.rect
                    );
                    if matches!(room.kind, RoomKind::Boss | RoomKind::Hall | RoomKind::Room) {
                        assert!(
                            validate::reaches_rect(&large, &level.tiles, room.rect),
                            "{ctx}: a large enemy cannot reach the {:?} {:?}",
                            room.kind,
                            room.rect
                        );
                    }
                }
                // Every maze stage B carved sits in a slot stage A reserved.
                for maze in &level.mazes {
                    assert!(
                        level
                            .rooms
                            .iter()
                            .any(|r| r.kind == RoomKind::Maze && r.rect == maze.bounds),
                        "{ctx}: maze at {:?} has no reserved slot",
                        maze.bounds
                    );
                }
            }
        }
    }

    /// Every map must pass the validator **on its first attempt**.
    ///
    /// `Level::generate` retries with a salted seed when a map fails, and that
    /// retry has to stay a safety net rather than a routine path: a map that
    /// needs one is a generator bug, it costs a second full generation, and in
    /// a debug build four failures panic. The sweep is deliberately small here
    /// so the default suite stays quick; `level_statistics` below runs the
    /// same check over thousands of seeds per theme.
    #[test]
    fn every_map_validates_on_its_first_attempt() {
        let mut failures: Vec<String> = Vec::new();
        for theme in Theme::all() {
            for seed in 0..100u64 {
                let level = crate::game::levelgen::generate_once(theme, seed, 0);
                if let Err(errors) = validate::validate(&level) {
                    failures.push(format!("{theme:?} seed {seed}: {errors:?}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} of 300 first attempts failed validation:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The designer-facing numbers, over a large seed sweep.
    ///
    /// Gated on `RATRUN_LEVEL_STATS=<seeds per theme>` (default 2500 when the
    /// variable is set to anything unparsable) so the default suite does not
    /// pay for it; it asserts the same first-attempt guarantee as the test
    /// above while it measures, so a regression in either shows up here too.
    ///
    /// ```text
    /// RATRUN_LEVEL_STATS=2500 cargo test level_statistics -- --nocapture
    /// ```
    #[test]
    fn level_statistics() {
        let Ok(per_theme) = std::env::var("RATRUN_LEVEL_STATS") else {
            return;
        };
        let per_theme: u64 = per_theme.parse().unwrap_or(2500).max(1);

        let mut failures = 0u64;
        let mut rooms = Vec::new();
        let mut halls = Vec::new();
        let mut plain = Vec::new();
        let mut corridor_legs = Vec::new();
        let mut corridor_len = Vec::new();
        let mut corridor_cells = Vec::new();
        let mut boss_area = Vec::new();
        let mut boss_dist = Vec::new();
        let mut maze_dist = Vec::new();
        let mut maze_entry = Vec::new();
        let mut maze_entry_secs = Vec::new();
        let mut maze_walk_cells = Vec::new();
        let mut maze_walk = Vec::new();
        let mut maze_loops = Vec::new();
        let mut maze_fork = Vec::new();
        let mut exit_walls = [0u64; 4];
        let mut landing_tiers = [0u64; 3];
        let mut bypass = [0u64; 3];
        let mut mazes_seen = 0u64;
        let mut open_fraction = Vec::new();
        let mut mazes_per_map = [0u64; 3];

        for theme in Theme::all() {
            for seed in 0..per_theme {
                let level = crate::game::levelgen::generate_once(theme, seed, 0);
                if validate::validate(&level).is_err() {
                    failures += 1;
                }
                let start = validate::cell_of(level.start);
                let cell_len = |p: Vec2| {
                    let c = validate::cell_of(p);
                    (((c.0 - start.0).pow(2) + (c.1 - start.1).pow(2)) as f64).sqrt()
                };
                let h = level.rooms_of(RoomKind::Hall).count() as f64;
                let p = level.rooms_of(RoomKind::Room).count() as f64;
                halls.push(h);
                plain.push(p);
                rooms.push(h + p);
                if let Some(boss) = level.boss_room() {
                    boss_area.push(boss.rect.area() as f64);
                    boss_dist.push(cell_len(boss.rect.center_world()));
                }
                mazes_per_map[level.mazes.len().min(2)] += 1;
                for section in &level.mazes {
                    mazes_seen += 1;
                    maze_dist.push(cell_len(section.reward));
                    // Entrance distance from the spawn, straight line and in
                    // walking seconds (58 px/s over 16 px cells).
                    let entry_gap = (section.entry.0 - start.0)
                        .abs()
                        .max((section.entry.1 - start.1).abs()) as f64;
                    maze_entry.push(entry_gap);
                    maze_entry_secs.push(entry_gap * 16.0 / 58.0);
                    // Cache walk: the maze's own grid route, in seconds.
                    let cells = maze::maze_walk_cells(&level.tiles, section)
                        .expect("the cache must be walkable") as f64;
                    maze_walk_cells.push(cells);
                    maze_walk.push(cells * 16.0 / 58.0);
                    maze_loops.push(maze::maze_loops(&level.tiles, section) as f64);
                    let trail = maze::trail_walk_cells(&level.tiles, section)
                        .expect("the fork must walk back to its breach");
                    maze_fork.push(trail as f64);
                    let wall = match section.exit_dir {
                        crate::game::art_walls::Cardinal::North => 0,
                        crate::game::art_walls::Cardinal::East => 1,
                        crate::game::art_walls::Cardinal::South => 2,
                        crate::game::art_walls::Cardinal::West => 3,
                    };
                    exit_walls[wall] += 1;
                    // The exit's landing must open onto another route: a room
                    // or a junction within eight cells.
                    let (dx, dy) = crate::game::collision::cardinal_delta(section.exit_dir);
                    let landing = (section.exit.0 + dx, section.exit.1 + dy);
                    let tier = maze::landing_tier(&level.tiles, &level.rooms, landing);
                    landing_tiers[tier as usize] += 1;
                    // The outside walkway is joined to a second ordinary room
                    // when two or more corridor legs reach it.
                    let walkway = section.bounds.inflate(2);
                    let reaches = level
                        .rooms_of(RoomKind::Corridor)
                        .filter(|room| room.rect.intersects(&walkway))
                        .count();
                    bypass[reaches.min(2)] += 1;
                }
                let mut legs = 0.0;
                let mut len = 0.0;
                for leg in level.rooms_of(RoomKind::Corridor) {
                    legs += 1.0;
                    let l = leg.rect.w.max(leg.rect.h) as f64;
                    len += l;
                    corridor_len.push(l);
                }
                corridor_legs.push(legs);
                corridor_cells.push(len);
                // Open floor for the player's size class, over the whole map.
                let open = (0..MAP_ROWS)
                    .flat_map(|cy| (0..MAP_COLS).map(move |cx| (cx, cy)))
                    .filter(|&(cx, cy)| level.tiles.passable(cx, cy, SizeClass::Small))
                    .count() as f64;
                open_fraction.push(100.0 * open / (MAP_COLS * MAP_ROWS) as f64);
            }
        }

        let maps = 3.0 * per_theme as f64;
        println!(
            "level statistics: {per_theme} seeds x 3 themes = {maps:.0} first attempts, {failures} validation failures"
        );
        println!("map: {MAP_COLS} x {MAP_ROWS} cells\n");
        print_stats("rooms (hall+room)", &mut rooms);
        print_stats("  of which halls", &mut halls);
        print_stats("  of which plain rooms", &mut plain);
        print_stats("corridor legs", &mut corridor_legs);
        print_stats("corridor leg length (cells)", &mut corridor_len);
        print_stats("corridor cells per map", &mut corridor_cells);
        print_stats("boss arena (cells)", &mut boss_area);
        print_stats("boss arena distance from start (cells)", &mut boss_dist);
        print_stats("maze reward distance from start (cells)", &mut maze_dist);
        print_stats("maze entrance distance (cells)", &mut maze_entry);
        print_stats("maze entrance distance (s, straight line)", &mut maze_entry_secs);
        print_stats("cache walk (cells)", &mut maze_walk_cells);
        print_stats("cache walk (s at 58 px/s)", &mut maze_walk);
        print_stats("maze interior loops", &mut maze_loops);
        print_stats("gnaw-mark trail (cells)", &mut maze_fork);
        print_stats("open floor (% of map)", &mut open_fraction);
        let total: u64 = mazes_per_map.iter().sum();
        println!("\nmazes per map");
        for (n, count) in mazes_per_map.iter().enumerate() {
            println!(
                "  {n} mazes: {count:>6}  ({:.1} %)",
                100.0 * *count as f64 / total as f64
            );
        }
        if mazes_seen == 0 {
            panic!("the sweep generated no maze at all");
        }
        let rate = 100.0 * mazes_per_map[1] as f64 / total as f64;
        println!(
            "\nmaze rate: {:.1} % of maps (owner's target: about one in eight)",
            rate
        );
        println!(
            "exit doors: N {}  E {}  S {}  W {}",
            exit_walls[0], exit_walls[1], exit_walls[2], exit_walls[3]
        );
        println!(
            "exit landing: in/next to a room {}  junction <= 8 cells {}  dead end {}",
            landing_tiers[2], landing_tiers[1], landing_tiers[0]
        );
        println!(
            "outside walkway reached by 0 / 1 / 2+ corridor legs: {} / {} / {}",
            bypass[0], bypass[1], bypass[2]
        );
        assert_eq!(failures, 0, "{failures} maps failed first-attempt validation");
        // The owner's rate: one map in eight, inside 10–15 %.
        assert!(
            (10.0..=15.0).contains(&rate),
            "maze rate {rate:.1} % is outside 10–15 %"
        );
        // The shape the designer asked for, on the whole sweep.
        assert!(
            exit_walls.iter().all(|count| *count > 0),
            "an exit orientation never occurs: {exit_walls:?}"
        );
        assert_eq!(mazes_per_map[2], 0, "a map carried two mazes");
        assert_eq!(landing_tiers[0], 0, "an exit landed on a dead end");
        assert!(
            maze_loops.iter().all(|loops| *loops >= 1.0),
            "a maze had no interior loop"
        );
        maze_walk.sort_by(f64::total_cmp);
        let median_walk = maze_walk[maze_walk.len() / 2];
        assert!(
            (7.0..=12.0).contains(&median_walk),
            "median cache walk {median_walk:.2} s is outside 7–12 s"
        );
        let longest = maze_walk[maze_walk.len() - 1];
        let shortest = maze_walk[0];
        println!("cache walk range: {shortest:.2}–{longest:.2} s");
    }


    /// One row of [`level_statistics`]: min / p25 / median / p75 / max / mean.
    fn print_stats(label: &str, values: &mut [f64]) {
        if values.is_empty() {
            println!("{label:<44} (none)");
            return;
        }
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        println!(
            "{label:<44} min {:>6.1}  p25 {:>6.1}  med {:>6.1}  p75 {:>6.1}  max {:>6.1}  mean {:>6.1}",
            values[0],
            at(0.25),
            at(0.5),
            at(0.75),
            values[values.len() - 1],
            mean
        );
    }
}
