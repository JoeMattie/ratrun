//! **Stage A — the map's skeleton.** Rooms, corridors, open halls, the boss
//! arena and the start room, then the props that furnish the rooms.
//!
//! Owner: *generator worker A*. The signatures, the [`Layout`] it returns and
//! the invariants below are final.
//!
//! ## Invariants `build_layout` must keep
//!
//! - The start room is centred on the map: `start_room.center_world()` is the
//!   rat's spawn point and equals `MAP_SIZE * 0.5`. Nothing solid, no trap and
//!   no prop may touch its cells.
//! - Exactly one [`RoomKind::Boss`] room exists (a large, wide-mouthed open
//!   arena for the map's boss) and the large size class can reach its centre.
//! - [`Layout::maze_slots`] are open, wall-free cell rectangles reserved for
//!   stage B; they never overlap a room's interior and never touch the border.
//! - Every wall is painted into the [`TileMap`] as a
//!   [`super::super::art_walls::WallKind`]; nothing returns a `Vec<Rectf>`
//!   anymore. Room openings are `Doorway` cells, never holes in a rect list.
//! - Nothing is drawn from a hash map and the only randomness is `rng`, in
//!   call order, so the same seed always yields the same map.
//!
//! ## How the map is built
//!
//! The whole lattice starts as **bedrock** (`WallKind::Solid`) and every room,
//! corridor and plaza is carved out of it. That order — rock first, floor
//! second — means a layout can never leak through a wall it did not make
//! itself, and the border is solid by construction.
//!
//! 1. the start room, centred exactly on the map (see [`Level::start`]);
//! 2. the boss arena, one big room far from the start room;
//! 3. rooms and halls sampled over the remaining floor space, with the gap
//!    between neighbours usually 2–3 cells and sometimes just 1 (a
//!    one-cell wall is where stage B can pierce a rat-hole later, and it is
//!    the "wall with a hole in it" the owner asked for);
//! 4. a nearest-first spanning tree of corridors — 1 or 2 cells wide, L
//!    shaped, each end a [`super::super::art_walls::WallKind::Doorway`] — so
//!    every room is connected to the start room (the boss arena's two-cell
//!    mouth is carved first, so it is always a real double door);
//! 5. 0..=2 maze slots: a reserved rectangle wrapped in a one-cell ring of
//!    open floor and joined to the nearest room by a short corridor, so both
//!    the maze's entry and its one-way exit open onto reachable floor;
//! 6. a one-cell partition with a doorway inside some halls (more thin walls
//!    to duck through, and cover to break line of sight) — after every
//!    corridor, so a partition can never be painted in front of a door;
//! 7. a final flood fill that stitches any orphaned region back onto the rest
//!    of the map, so the connectivity promise is proven, not assumed.
//!
//! [`Level::start`]: crate::game::level::Level::start

use rand::rngs::StdRng;
use rand::RngExt;

use crate::game::art_walls::{WallAxis, WallKind};
use crate::game::collision::{TileKind, TileMap, TILE_F};
use crate::game::level::{theme_props, Prop, Room, RoomKind, Theme, TileRect};
use crate::math::Vec2;

/// Solid cells kept between the playable field and the map edge: the wall art
/// draws a south face and a contact shadow, so the border is never one cell.
const BORDER: i32 = 2;

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
    /// Free rectangles reserved for maze sections (0..=2 used by stage B).
    pub maze_slots: Vec<TileRect>,
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

/// A side of a room, i.e. the direction a doorway on it faces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    /// Axis of the **wall** the doorway pierces (`WallAxis` names the wall's
    /// axis, not the passage's — see `CONTENT_PLAN.md` §9.3).
    #[inline]
    fn wall_axis(self) -> WallAxis {
        match self {
            Side::North | Side::South => WallAxis::Horizontal,
            Side::East | Side::West => WallAxis::Vertical,
        }
    }

    /// One-cell step along the passage direction (outwards on this side).
    #[inline]
    fn step(self) -> (i32, i32) {
        match self {
            Side::North => (0, -1),
            Side::East => (1, 0),
            Side::South => (0, 1),
            Side::West => (-1, 0),
        }
    }

    /// One-cell step along the wall, used to widen a doorway to two cells.
    #[inline]
    fn lateral(self) -> (i32, i32) {
        match self {
            Side::North | Side::South => (1, 0),
            Side::East | Side::West => (0, 1),
        }
    }
}

/// One corridor request. Bundled so the carve call stays readable.
struct Corridor<'a> {
    from: TileRect,
    to: TileRect,
    /// 1 or 2 cells: 1 = a normal doorway (14 px clear), 2 = a double door
    /// (30 px clear, wide enough for a boss).
    width: i32,
    /// Punch a doorway into `to`'s wall ring. `false` for a maze slot, whose
    /// surround is already open floor.
    door_to: bool,
    /// Rectangles the corridor legs should route around when they can (the
    /// rooms it is not connecting: a corridor that cuts through a third room
    /// would open a doorless hole in that room's wall).
    avoid: &'a [TileRect],
}

/// **Stage A.** Carve the map's rooms, corridors and structural walls into
/// `tiles` and return the regions.
pub fn build_layout(tiles: &mut TileMap, rng: &mut StdRng, theme: Theme) -> Layout {
    let cols = tiles.cols();
    let rows = tiles.rows();

    // Bedrock first: every room, corridor and plaza is carved out of it.
    tiles.fill_cells(0, 0, cols, rows, SOLID);
    let field = TileRect::new(BORDER, BORDER, cols - 2 * BORDER, rows - 2 * BORDER);
    let room_area = field.inflate(-ROOM_MARGIN);

    // ---- 1. start room: centred on the map, exactly -----------------------
    // Even dimensions keep `center_world()` on a whole cell boundary, which is
    // what makes `start == MAP_SIZE * 0.5` exact rather than approximate.
    let half_w = rng.random_range(9..=12);
    let half_h = rng.random_range(8..=11);
    let start_room = TileRect::new(
        cols / 2 - half_w,
        rows / 2 - half_h,
        half_w * 2,
        half_h * 2,
    );
    carve(tiles, start_room);
    debug_assert_eq!(
        start_room.center_world(),
        Vec2::new(cols as f32 * 0.5 * TILE_F, rows as f32 * 0.5 * TILE_F),
        "the start room must be centred on the map"
    );
    let mut rooms = vec![Room::new(start_room, RoomKind::Start)];

    // ---- 2. boss arena ----------------------------------------------------
    // A big open room in the far half of the map: the boss-fight phase needs
    // space to move, and a wide (two-cell) mouth the largest enemies fit.
    let boss_w = rng.random_range(26..=32);
    let boss_h = rng.random_range(25..=30);
    let map_centre = start_room.center_world();
    let far = |r: &TileRect| r.center_world().dist(map_centre) >= 48.0 * TILE_F;
    let boss_arena = pick_rect(rng, field, ROOM_MARGIN, boss_w, boss_h, 2, |r| {
        far(&r) && !r.inflate(6).intersects(&start_room)
    })
    .or_else(|| {
        pick_rect(rng, field, ROOM_MARGIN, boss_w, boss_h, 2, |r| {
            !r.inflate(6).intersects(&start_room)
        })
    })
    .unwrap_or_else(|| {
        // Deterministic last resort: the south-east corner of the field. It
        // cannot overlap the centred start room at this map size.
        TileRect::new(
            field.right() - ROOM_MARGIN - boss_w,
            field.bottom() - ROOM_MARGIN - boss_h,
            boss_w,
            boss_h,
        )
    });
    carve(tiles, boss_arena);
    rooms.push(Room::new(boss_arena, RoomKind::Boss));

    // ---- 3. rooms and halls ----------------------------------------------
    let hall_bias = match theme {
        Theme::Sewer => 0.30,
        Theme::Kitchen => 0.45,
        Theme::Lab => 0.38,
    };
    let wanted = rng.random_range(12..=18);
    let mut placed = 0;
    let mut tries = 0;
    while placed < wanted && tries < wanted * 8 {
        tries += 1;
        // Every map gets at least one open hall and one plain room, whatever
        // the theme's bias says: the horde needs a hall to surround the rat
        // in, and stage B's trap scatter needs both kinds.
        let hall = if placed < 2 {
            placed == 0
        } else {
            rng.random_bool(hall_bias)
        };
        let (w, h) = if hall {
            (rng.random_range(18..=30), rng.random_range(14..=24))
        } else {
            (rng.random_range(10..=20), rng.random_range(9..=17))
        };
        // Mostly 2–3 cells of rock between neighbours; one in six closes to a
        // single cell, which stage B can pierce with rat-holes.
        let gap = match rng.random_range(0..6) {
            0 => 1,
            1..=3 => 2,
            _ => 3,
        };
        let Some(rect) = random_rect(rng, room_area, w, h) else {
            break;
        };
        // The boss arena always keeps a two-cell gap: its own wall ring cell
        // has to be bedrock when the two-cell mouth is punched into it.
        if rect.inflate(2).intersects(&boss_arena) {
            continue;
        }
        if rooms.iter().any(|o| rect.inflate(gap).intersects(&o.rect)) {
            continue;
        }
        carve(tiles, rect);
        rooms.push(Room::new(
            rect,
            if hall { RoomKind::Hall } else { RoomKind::Room },
        ));
        placed += 1;
    }

    // ---- 4. corridors: a nearest-first spanning tree ----------------------
    // Nearest-first (Prim's) keeps corridors short and always connects every
    // room to the start room, which is the connectivity the validator proves.
    let tree_len = rooms.len();
    let mut linked = vec![false; tree_len];
    let mut corridors: Vec<TileRect> = Vec::new();

    // The boss arena's mouth is carved **first**, before any other corridor can
    // merge into its wall ring: a doorway punched into a cell that is already
    // open floor would not be a doorway at all, and the boss would have no
    // double door to walk through. The boss is *not* marked linked here: the
    // spanning tree below still has to reach it from the start room, so this
    // early corridor can never strand the arena in its own little component.
    let mut linked_count = 1;
    linked[0] = true;
    if tree_len > 1 {
        let mut nearest: Option<(f32, usize)> = None;
        for (k, room) in rooms[2..tree_len].iter().enumerate() {
            let d = room.rect.center_world().dist_sq(rooms[1].rect.center_world());
            if nearest.is_none_or(|(bd, _)| d < bd) {
                nearest = Some((d, k + 2));
            }
        }
        let from = nearest.map_or(0, |(_, k)| k);
        let avoid: Vec<TileRect> = rooms[..tree_len]
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != from && *k != 1)
            .map(|(_, r)| r.rect)
            .collect();
        corridors.extend(carve_corridor(
            tiles,
            rng,
            field,
            Corridor {
                from: rooms[from].rect,
                to: rooms[1].rect,
                width: 2,
                door_to: true,
                avoid: &avoid,
            },
        ));
    }
    while linked_count < tree_len {
        let mut best: Option<(f32, usize, usize)> = None;
        for (i, a) in rooms[..tree_len].iter().enumerate() {
            if !linked[i] {
                continue;
            }
            for (j, b) in rooms[..tree_len].iter().enumerate() {
                if linked[j] {
                    continue;
                }
                let d = a.rect.center_world().dist_sq(b.rect.center_world());
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, i, j));
                }
            }
        }
        let Some((_, i, j)) = best else { break };
        linked[j] = true;
        linked_count += 1;

        let width = if rng.random_bool(0.35) { 2 } else { 1 };
        let avoid: Vec<TileRect> = rooms[..tree_len]
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i && *k != j)
            .map(|(_, r)| r.rect)
            .collect();
        let legs = carve_corridor(
            tiles,
            rng,
            field,
            Corridor {
                from: rooms[i].rect,
                to: rooms[j].rect,
                width,
                door_to: true,
                avoid: &avoid,
            },
        );
        corridors.extend(legs);
    }
    for leg in &corridors {
        rooms.push(Room::new(*leg, RoomKind::Corridor));
    }

    // ---- 5. maze slots ---------------------------------------------------
    // Zero to two. Each is reserved as open floor with a one-cell ring around
    // it and joined to the nearest room, so the maze's entry and its one-way
    // exit both open onto reachable floor.
    let want_slots = match rng.random_range(0..6) {
        0 => 0,
        1 | 2 => 1,
        _ => 2,
    };
    let mut maze_slots: Vec<TileRect> = Vec::new();
    for _ in 0..want_slots {
        let w = rng.random_range(20..=26);
        let h = rng.random_range(16..=20);
        let slot = pick_rect(rng, field, ROOM_MARGIN + 1, w, h, 2, |r| {
            rooms.iter().all(|o| !r.inflate(3).intersects(&o.rect))
                && maze_slots.iter().all(|s| !r.inflate(4).intersects(s))
        });
        let Some(slot) = slot else { continue };

        // A short access corridor from the nearest room into the ring. It is
        // *planned* first: a leg that would cross this slot (or an earlier
        // one) would cut a maze open, so such a slot is simply not used.
        let mut nearest: Option<(f32, TileRect)> = None;
        for room in rooms[..tree_len].iter() {
            let d = room.rect.center_world().dist_sq(slot.center_world());
            if nearest.is_none_or(|(bd, _)| d < bd) {
                nearest = Some((d, room.rect));
            }
        }
        let Some((_, from)) = nearest else { continue };
        let avoid: Vec<TileRect> = rooms[..tree_len]
            .iter()
            .filter(|r| r.rect != from)
            .map(|r| r.rect)
            .collect();
        let plan = plan_corridor(
            rng,
            field,
            &Corridor {
                from,
                to: slot,
                width: 1,
                door_to: false,
                avoid: &avoid,
            },
        );
        let cuts_a_maze = plan.touches(slot, 1)
            || maze_slots.iter().any(|s| plan.touches(*s, 1));
        if cuts_a_maze {
            continue;
        }

        carve(tiles, slot.inflate(1));
        maze_slots.push(slot);
        rooms.push(Room::new(slot, RoomKind::Maze));
        for leg in apply_corridor(tiles, field, &plan, [from, slot]) {
            rooms.push(Room::new(leg, RoomKind::Corridor));
            corridors.push(leg);
        }
    }

    // ---- 6. partitions in halls ------------------------------------------
    // One-cell walls that split a hall in two with a single doorway: cover, a
    // second route for the rat through a rat-hole, and a wall for stage B to
    // put one in. This runs *after* every corridor — including the maze access
    // corridors — so a partition can never be painted in front of a doorway.
    let doors: Vec<(i32, i32)> = tiles
        .iter_walls()
        .filter(|(_, _, k)| matches!(k, WallKind::Doorway { .. }))
        .map(|(cx, cy, _)| (cx, cy))
        .collect();
    for room in rooms.iter().filter(|r| r.kind == RoomKind::Hall).copied() {
        partition_hall(tiles, rng, room.rect, &doors, &corridors);
    }

    // ---- 7. connectivity repair ------------------------------------------
    // The corridors above are geometric: an L path can be blocked by a room it
    // was not told about, and a leg can end one cell short of the doorway it
    // aimed at. The map's promise — the rat reaches every room, a large enemy
    // reaches every hall — is what `super::validate` proves a moment later, so
    // make it true *here* instead of hoping: flood fill from the start and
    // stitch every orphaned region back with a straight corridor. On a
    // well-formed map this pass finds nothing to do.
    let mut targets: Vec<TileRect> = rooms[..tree_len].iter().map(|r| r.rect).collect();
    for slot in &maze_slots {
        // The ring of open floor around a maze is what its entry and its
        // one-way exit have to reach; repairing inside the slot would be
        // pointless, because stage B carves the maze after this.
        targets.push(slot.inflate(1));
    }
    let seen = connect_orphans(
        tiles,
        field,
        &targets,
        (
            (start_room.center_world().x / TILE_F) as i32,
            (start_room.center_world().y / TILE_F) as i32,
        ),
    );
    // A corridor leg that no route reaches is a vestigial stub — carved into
    // rock by an elbow that missed its doorway. It is not a place the player or
    // the horde can be sent to, so it is not a room: the validator checks every
    // room it is given, and a one-cell pocket behind solid rock is not a
    // promise the map can keep.
    rooms.retain(|r| r.kind != RoomKind::Corridor || reached(&seen, tiles, r.rect));

    Layout {
        rooms,
        start: start_room.center_world(),
        start_room,
        boss_arena,
        maze_slots,
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
    let legs = [
        TileRect::new(x0, from.1, x1 - x0 + 1, 1),
        TileRect::new(to.0, y0, 1, y1 - y0 + 1),
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
    for room in layout.rooms.iter().filter(|r| layout.is_furnishable(r)) {
        // Keep a one-cell margin so a prop never merges into a wall run.
        let inner = room.rect.inflate(-1);
        if inner.w < 1 || inner.h < 1 {
            continue;
        }
        // The room's anchor: spawns, traps and tests all use it, so it stays
        // walkable on every map.
        let core = TileRect::new(
            room.rect.x + room.rect.w / 2 - 1,
            room.rect.y + room.rect.h / 2 - 1,
            3,
            3,
        );
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
                if cell.intersects(&core) {
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
                if keeps_openings_clear(tiles, cell) {
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
    props
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

/// [`carve_if_solid`], except that the one-cell wall rings of `protect` are left
/// alone.
///
/// A two-cell corridor whose leg runs alongside a room wall would otherwise
/// carve the wall's whole length open (a nineteen-cell hole instead of a
/// double door). The door cells themselves are punched first and are already
/// floor of their own kind, so skipping the ring never disconnects the
/// corridor: its first cell outside the ring is carved normally.
fn carve_leg(
    tiles: &mut TileMap,
    rect: TileRect,
    field: TileRect,
    protect: [TileRect; 2],
) {
    let r = clip(rect, field);
    for (cx, cy) in r.cells() {
        if protect.iter().any(|p| p.inflate(1).contains(cx, cy)) {
            continue;
        }
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

/// Looks at a prop's neighbourhood: a prop must never narrow a doorway, a
/// rat-hole or a one-way door, and never sit right in front of one.
fn keeps_openings_clear(tiles: &TileMap, cell: TileRect) -> bool {
    for (cx, cy) in cell.inflate(2).cells() {
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

/// Punch a `width`-cell doorway into the wall on `side` of a room, starting at
/// the wall cell `first`, and make sure the floor outside each cell is open.
///
/// The outside cell is carved explicitly because a two-cell doorway whose
/// corridor leg turns immediately would otherwise leave its second cell facing
/// rock — a doorway that is half blocked still looks open.
fn punch_door(
    tiles: &mut TileMap,
    field: TileRect,
    side: Side,
    first: (i32, i32),
    width: i32,
) {
    let lateral = side.lateral();
    let outward = side.step();
    for k in 0..width.max(1) {
        let cell = (first.0 + lateral.0 * k, first.1 + lateral.1 * k);
        if tiles.get(cell.0, cell.1) == SOLID {
            tiles.set(
                cell.0,
                cell.1,
                TileKind::Wall(WallKind::Doorway {
                    axis: side.wall_axis(),
                }),
            );
        }
        carve_if_solid(
            tiles,
            TileRect::new(cell.0 + outward.0, cell.1 + outward.1, 1, 1),
            field,
        );
    }
}

/// A corridor that has been decided but not yet painted: the doors it punches
/// and the legs it carves.
///
/// Splitting plan from paint lets the maze stage check a corridor against the
/// maze slots already reserved — a leg that crosses a reserved slot would cut
/// the maze open — before a single cell of the slot is touched.
struct CorridorPlan {
    legs: Vec<TileRect>,
    side_a: Side,
    door_a: (i32, i32),
    side_b: Side,
    door_b: (i32, i32),
    width: i32,
    door_to: bool,
}

impl CorridorPlan {
    /// Does any leg touch `rect` (grown by `margin` cells)?
    fn touches(&self, rect: TileRect, margin: i32) -> bool {
        let r = rect.inflate(margin);
        self.legs.iter().any(|leg| leg.intersects(&r))
    }
}

/// Carve one L-shaped corridor between two rooms and punch its doorways.
/// Returns the corridor's leg rectangles (recorded as [`RoomKind::Corridor`]).
fn carve_corridor(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    field: TileRect,
    c: Corridor<'_>,
) -> Vec<TileRect> {
    let plan = plan_corridor(rng, field, &c);
    apply_corridor(tiles, field, &plan, [c.from, c.to])
}

/// Decide one L-shaped corridor: which sides its doors pierce, and where its
/// two legs run. Paints nothing.
fn plan_corridor(rng: &mut StdRng, field: TileRect, c: &Corridor<'_>) -> CorridorPlan {
    let (a, b) = (c.from, c.to);
    // The rects do not intersect, so at least one axis separates them. Route
    // through that gap: a leg then never cuts through either room.
    let gap_x = (b.x - a.right()).max(a.x - b.right());
    let gap_y = (b.y - a.bottom()).max(a.y - b.bottom());
    let horizontal = gap_x > 0 && (gap_y <= 0 || gap_x >= gap_y);
    let (side_a, side_b) = if horizontal {
        if b.x >= a.right() {
            (Side::East, Side::West)
        } else {
            (Side::West, Side::East)
        }
    } else if b.y >= a.bottom() {
        (Side::South, Side::North)
    } else {
        (Side::North, Side::South)
    };

    let door_a = door_cell(rng, a, side_a);
    let door_b = door_cell(rng, b, side_b);
    let p1 = (door_a.0 + side_a.step().0, door_a.1 + side_a.step().1);
    let p2 = (door_b.0 + side_b.step().0, door_b.1 + side_b.step().1);
    let width = c.width.max(1);

    // Two possible elbow orderings; prefer the one whose legs miss every other
    // room, so a corridor does not silently open a doorless hole in one.
    let horizontal_first = rng.random_bool(0.5);
    let legs = |horiz_first: bool| -> [TileRect; 2] {
        let (x0, y0) = p1;
        let (x1, y1) = p2;
        if horiz_first {
            [
                TileRect::new(x0.min(x1), y0, (x0 - x1).abs() + 1, width),
                TileRect::new(x1, y0.min(y1), width, (y0 - y1).abs() + 1),
            ]
        } else {
            [
                TileRect::new(x0, y0.min(y1), width, (y0 - y1).abs() + 1),
                TileRect::new(x0.min(x1), y1, (x0 - x1).abs() + 1, width),
            ]
        }
    };
    // How many other rooms each elbow ordering would cut through. A clean
    // ordering (score 0) is the common case; when neither is clean the less
    // destructive one wins, and the merge is left as an open archway rather
    // than pretending a corridor can tunnel a room it was told to avoid.
    let collisions = |legs: &[TileRect; 2]| -> usize {
        c.avoid
            .iter()
            .filter(|r| legs.iter().any(|leg| leg.intersects(r)))
            .count()
    };
    let preferred = legs(horizontal_first);
    let other = legs(!horizontal_first);
    let chosen = if collisions(&preferred) <= collisions(&other) {
        preferred
    } else {
        other
    };

    // Clip to the field before carving *and* before recording: a corridor that
    // ran into the two-cell border would break the map edge, and a recorded
    // rectangle that reached outside it would look unreachable to the
    // validator.
    let mut legs: Vec<TileRect> = chosen
        .iter()
        .map(|leg| clip(*leg, field))
        .filter(|leg| leg.area() > 0)
        .collect();
    legs.dedup();

    CorridorPlan {
        legs,
        side_a,
        door_a,
        side_b,
        door_b,
        width,
        door_to: c.door_to,
    }
}

/// Paint a planned corridor: doorways first (a leg carved first can swallow the
/// wall cell the doorway sits in, turning a double door into a plain hole), then
/// its legs. Returns the legs that really became floor.
fn apply_corridor(
    tiles: &mut TileMap,
    field: TileRect,
    plan: &CorridorPlan,
    protect: [TileRect; 2],
) -> Vec<TileRect> {
    let mut legs = plan.legs.clone();
    punch_door(tiles, field, plan.side_a, plan.door_a, plan.width);
    if plan.door_to {
        punch_door(tiles, field, plan.side_b, plan.door_b, plan.width);
    }
    for leg in &legs {
        carve_leg(tiles, *leg, field, protect);
    }
    // Only record legs that really became floor: a leg that was protected in
    // full would otherwise be a "room" with nothing reachable in it.
    legs.retain(|leg| {
        leg.cells()
            .any(|(cx, cy)| tiles.get(cx, cy).passable(crate::game::collision::SizeClass::Small))
    });
    legs
}

/// The wall cell just outside `room` on `side`, with a random lateral position
/// kept two cells away from the room's corners (see `Side::wall_axis`: the
/// doorway needs wall on both of its wall-parallel sides).
fn door_cell(rng: &mut StdRng, room: TileRect, side: Side) -> (i32, i32) {
    match side {
        Side::North | Side::South => {
            let lo = room.x + 2;
            let hi = (room.right() - 3).max(lo);
            let x = rng.random_range(lo..=hi);
            if side == Side::North {
                (x, room.y - 1)
            } else {
                (x, room.bottom())
            }
        }
        Side::East | Side::West => {
            let lo = room.y + 2;
            let hi = (room.bottom() - 3).max(lo);
            let y = rng.random_range(lo..=hi);
            if side == Side::West {
                (room.x - 1, y)
            } else {
                (room.right(), y)
            }
        }
    }
}

/// Split a hall with a one-cell wall and a single doorway.
///
/// The wall stays three cells clear of the hall's centre (spawns and traps use
/// it) and is skipped when it would run into a corridor that already crosses
/// the hall or into one of the hall's own doorways.
fn partition_hall(
    tiles: &mut TileMap,
    rng: &mut StdRng,
    hall: TileRect,
    doors: &[(i32, i32)],
    corridors: &[TileRect],
) {
    if hall.w < 8 || hall.h < 8 {
        return;
    }
    let horizontal = rng.random_bool(0.5);
    let middle = hall.y + hall.h / 2;
    let mid_col = hall.x + hall.w / 2;
    // Two rows/columns to try: one near each wall, both >= 3 from the centre.
    let (first, second) = if rng.random_bool(0.5) {
        (2, -3)
    } else {
        (-3, 2)
    };
    for offset in [first, second] {
        let rect = if horizontal {
            let cy = hall.y + (hall.h / 2 + offset).clamp(2, hall.h - 3);
            if (cy - middle).abs() < 3 {
                continue;
            }
            TileRect::new(hall.x, cy, hall.w, 1)
        } else {
            let cx = hall.x + (hall.w / 2 + offset).clamp(2, hall.w - 3);
            if (cx - mid_col).abs() < 3 {
                continue;
            }
            TileRect::new(cx, hall.y, 1, hall.h)
        };
        // Never cut a corridor that crosses this hall, and never sit in front
        // of one of the hall's own doorways (a door that opens into a wall is
        // a dead end).
        if corridors.iter().any(|c| c.inflate(1).intersects(&rect)) {
            continue;
        }
        let blocks_door = doors.iter().any(|&(dx, dy)| {
            if horizontal {
                dx >= rect.x - 1 && dx <= rect.right() && (dy - rect.y).abs() <= 1
            } else {
                dy >= rect.y - 1 && dy <= rect.bottom() && (dx - rect.x).abs() <= 1
            }
        });
        if blocks_door {
            continue;
        }
        // Paint the wall, then open one doorway near its middle.
        tiles.fill_cells(rect.x, rect.y, rect.w, rect.h, SOLID);
        if horizontal {
            let lo = rect.x + 2;
            let hi = (rect.right() - 3).max(lo);
            let x = rng.random_range(lo..=hi);
            tiles.set(
                x,
                rect.y,
                TileKind::Wall(WallKind::Doorway {
                    axis: WallAxis::Horizontal,
                }),
            );
        } else {
            let lo = rect.y + 2;
            let hi = (rect.bottom() - 3).max(lo);
            let y = rng.random_range(lo..=hi);
            tiles.set(
                rect.x,
                y,
                TileKind::Wall(WallKind::Doorway {
                    axis: WallAxis::Vertical,
                }),
            );
        }
        return;
    }
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
            for seed in 0..40u64 {
                let (tiles, layout) = layout_for(theme, seed);
                let ctx = format!("{theme:?} seed {seed}");

                // Start room: centred, clear, first in the list.
                assert_eq!(layout.start, MAP_SIZE * 0.5, "{ctx}");
                assert_eq!(layout.start_room.center_world(), layout.start, "{ctx}");
                assert_eq!(layout.rooms[0].kind, RoomKind::Start, "{ctx}");
                assert_eq!(layout.rooms[0].rect, layout.start_room, "{ctx}");
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

                // Exactly one boss arena, big and open.
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
                        .boss_arena
                        .cells()
                        .all(|(x, y)| tiles.get(x, y).is_open()),
                    "{ctx}: the boss arena is not open floor"
                );
                // A wide mouth: a boss is 28 px across and a one-cell doorway
                // leaves 14 px, so at least one doorway into the arena has to
                // be a double door.
                let ring = layout.boss_arena.inflate(1);
                let wide = ring
                    .cells()
                    .filter(|&(x, y)| x == ring.x || y == ring.y || x == ring.right() - 1 || y == ring.bottom() - 1)
                    .any(|(x, y)| {
                        let is_door = |cx: i32, cy: i32| {
                            matches!(
                                tiles.get(cx, cy),
                                TileKind::Wall(WallKind::Doorway { .. } | WallKind::OneWayDoor { .. })
                            )
                        };
                        is_door(x, y) && (is_door(x + 1, y) || is_door(x, y + 1))
                    });
                assert!(wide, "{ctx}: the boss arena has no two-cell mouth");

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

                // Rooms stay in the field and the non-corridor ones never
                // overlap. Corridors may cross a room (they merge into it).
                let mut solid_rooms: Vec<TileRect> = Vec::new();
                let mut halls = 0;
                let mut plain = 0;
                let mut corridor_legs = 0;
                for room in &layout.rooms {
                    assert!(
                        in_field(room.rect, MAP_COLS, MAP_ROWS),
                        "{ctx}: {:?} {:?} leaves the field",
                        room.kind,
                        room.rect
                    );
                    assert!(room.rect.area() > 0, "{ctx}: empty room");
                    match room.kind {
                        RoomKind::Corridor => corridor_legs += 1,
                        RoomKind::Hall => halls += 1,
                        RoomKind::Room => plain += 1,
                        _ => {}
                    }
                    if room.kind != RoomKind::Corridor {
                        for other in &solid_rooms {
                            assert!(
                                !room.rect.intersects(other),
                                "{ctx}: rooms {:?} and {other:?} overlap",
                                room.rect
                            );
                        }
                        solid_rooms.push(room.rect);
                    }
                }
                assert!(halls >= 1, "{ctx}: no open hall");
                assert!(plain >= 1, "{ctx}: no plain room");
                assert!(corridor_legs >= 2, "{ctx}: no corridors");
                assert!(solid_rooms.len() >= 6, "{ctx}: only {} rooms", solid_rooms.len());

                // Maze slots: open, in the field, one maze room each, ringed by
                // open floor so both the entry and the exit have somewhere to
                // lead.
                assert!(layout.maze_slots.len() <= 2, "{ctx}");
                for slot in &layout.maze_slots {
                    assert!(
                        slot.cells().all(|(x, y)| tiles.get(x, y).is_open()),
                        "{ctx}: maze slot {slot:?} is not open floor"
                    );
                    assert!(
                        within(slot.inflate(2), field_of(&tiles)),
                        "{ctx}: maze slot {slot:?} is too close to the border"
                    );
                    assert!(
                        layout
                            .rooms
                            .iter()
                            .any(|r| r.kind == RoomKind::Maze && r.rect == *slot),
                        "{ctx}: maze slot {slot:?} has no RoomKind::Maze record"
                    );
                    assert!(
                        slot.inflate(1)
                            .cells()
                            .all(|(x, y)| tiles.get(x, y).is_open()),
                        "{ctx}: the ring around maze slot {slot:?} is not floor"
                    );
                    for room in layout.rooms.iter().filter(|r| {
                        !matches!(r.kind, RoomKind::Maze | RoomKind::Corridor)
                    }) {
                        assert!(
                            !room.rect.intersects(slot),
                            "{ctx}: maze slot overlaps {:?}",
                            room.kind
                        );
                    }
                }

                // Every doorway opens onto floor on both of its passage sides
                // (a doorway that faces rock is a broken corridor).
                for (cx, cy, kind) in tiles.iter_walls() {
                    let WallKind::Doorway { axis } = kind else {
                        continue;
                    };
                    let sides: [(i32, i32); 2] = match axis {
                        WallAxis::Horizontal => [(cx, cy - 1), (cx, cy + 1)],
                        WallAxis::Vertical => [(cx - 1, cy), (cx + 1, cy)],
                    };
                    for (sx, sy) in sides {
                        // Passable, not `is_open`: a doorway may face another
                        // doorway across a two-cell-thick wall (both rings get
                        // their own door, which is a 32 px double tunnel).
                        assert!(
                            tiles.passable(sx, sy, SizeClass::Small),
                            "{ctx}: doorway ({cx},{cy}) faces a wall at ({sx},{sy})"
                        );
                    }
                }

                // Connectivity, exactly as the validator proves it.
                for c in [(49,124),(49,125),(49,126),(48,126),(18,126),(18,127),(26,126),(26,127),(27,127),(26,128),(14,128),(20,128)] {
            println!("DIAG cell {:?} = {:?}", c, tiles.get(c.0, c.1));
        }
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

                // Stage B needs thin walls to pierce: the "holes the rat ducks
                // into" come from here.
                let candidates = (0..MAP_ROWS)
                    .flat_map(|cy| (0..MAP_COLS).map(move |cx| (cx, cy)))
                    .filter(|&(cx, cy)| {
                        hole_candidate(&tiles, cx, cy)
                            && !TileRect::new(cx, cy, 1, 1)
                                .intersects(&layout.start_room.inflate(1))
                            && !TileRect::new(cx, cy, 1, 1).intersects(&layout.boss_arena)
                    })
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
                    // Painted solid, and inside a furnishable room.
                    assert!(cell.cells().all(|(x, y)| tiles.get(x, y).is_wall()));
                    assert!(layout.rooms.iter().any(|r| {
                        layout.is_furnishable(r) && r.rect.inflate(1).contains(cell.x, cell.y)
                    }));
                    // Never in a maze slot, never adjacent to another prop.
                    assert!(!layout.maze_slots.iter().any(|s| s.intersects(&cell)));
                    assert!(!cells.iter().any(|c| c.inflate(1).intersects(&cell)));
                    cells.push(cell);
                }

                // Every room centre (3x3) and every opening is still clear.
                for room in layout.rooms.iter().filter(|r| layout.is_furnishable(r)) {
                    let core = TileRect::new(
                        room.rect.x + room.rect.w / 2 - 1,
                        room.rect.y + room.rect.h / 2 - 1,
                        3,
                        3,
                    );
                    assert!(
                        core.cells().all(|(x, y)| tiles.get(x, y).is_open()),
                        "{theme:?} seed {seed}: the {:?} centre is blocked",
                        room.kind
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
                for c in [(49,124),(49,125),(49,126),(48,126),(18,126),(18,127),(26,126),(26,127),(27,127),(26,128),(14,128),(20,128)] {
            println!("DIAG cell {:?} = {:?}", c, tiles.get(c.0, c.1));
        }
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
                for maze in &level.mazes {
                    maze_dist.push(cell_len(maze.reward));
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
        print_stats("open floor (% of map)", &mut open_fraction);
        let total: u64 = mazes_per_map.iter().sum();
        println!("\nmazes per map");
        for (n, count) in mazes_per_map.iter().enumerate() {
            println!(
                "  {n} mazes: {count:>6}  ({:.1} %)",
                100.0 * *count as f64 / total as f64
            );
        }
        assert_eq!(failures, 0, "{failures} maps failed first-attempt validation");
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


















