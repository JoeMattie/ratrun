//! Seeded, non-colliding room lights. This stage only reads the finished map.
//!
//! Two passes: the **hanging** lights (a row of pendants over each room, a
//! passage lamp down each corridor, the blackout floors'
//! dying bulbs — batch 8 and earlier) and the **wall-mounted** sconces (batch
//! 9, [`place_wall_lamps`]). The sconce pass is appended after every existing
//! draw of the stage's salted rng, so it can only add fixtures: the zones, the
//! emitters and the hanging lamps are byte-identical to before. Fixtures are
//! render-only inputs (no system reads `objects.lights`), so this stage cannot
//! move a determinism golden.
use rand::RngExt;

use crate::game::art_walls::{Cardinal, WallKind};
use crate::game::collision::{cardinal_delta, cardinal_vec, cell_of, TileKind, TileMap, TILE_F};
use crate::game::level::{
    level_rng, AmbientEmitter, EmitterKind, Level, LightFixture, LightFixtureKind, LightZone,
    LightZoneKind, MazeSection, Room, RoomKind, Theme, TileRect,
};
use crate::math::Vec2;

pub(super) fn place(level: &mut Level, attempt: u32) {
    let mut rng = level_rng(level.theme, level.seed ^ 0x4c49_4748_5453, attempt);
    place_hanging_lights(level, &mut rng);
    // Batch 9: the wall-mounted lamps. Appended after every existing draw, so
    // this stage's earlier output cannot move and the fixture list can only
    // grow.
    place_wall_lamps(level, &mut rng);
}

/// The pre-batch-9 stage: one row of hanging lamps per room, the zone list and
/// the ambient emitters. Its rng draws are untouched, so
/// `sconces_do_not_move_the_hanging_lamps` can run it on its own.
fn place_hanging_lights(level: &mut Level, rng: &mut rand::rngs::StdRng) {
    if level.theme == Theme::Trailer {
        level.objects.lights.push(LightFixture {
            pos: level.start,
            kind: LightFixtureKind::Sun,
            radius: 0.0,
            seed: rng.random(),
        });
        // The warren below the planks is still dark. The sun pass skips it.
        level
            .objects
            .light_zones
            .extend(level.mazes.iter().map(|maze| LightZone {
                rect: maze.bounds,
                kind: LightZoneKind::Maze,
            }));
        place_emitters(level, rng);
        return;
    }

    for room in &level.rooms {
        let seed = rng.random::<u32>();
        let zone = match room.kind {
            RoomKind::Start => LightZoneKind::Start,
            RoomKind::Boss => LightZoneKind::Room,
            RoomKind::Corridor => LightZoneKind::Corridor,
            RoomKind::Maze => LightZoneKind::Maze,
            RoomKind::Room if seed.is_multiple_of(5) => LightZoneKind::Blackout,
            _ => LightZoneKind::Room,
        };
        level.objects.light_zones.push(LightZone {
            rect: room.rect,
            kind: zone,
        });

        let span = room.rect.w.max(room.rect.h) as f32 * 16.0;
        let horizontal = room.rect.w >= room.rect.h;
        let (kind, count, radius) = match zone {
            LightZoneKind::Start => (
                LightFixtureKind::Pendant,
                2,
                (span * 0.65).clamp(88.0, 160.0),
            ),
            // Unreachable since batch 12 removed the boss arena: no map has an
            // Arena zone. The arm stays because the enum variant is matched
            // exhaustively here (and in `render::lighting`).
            LightZoneKind::Arena => (
                LightFixtureKind::Arena,
                4,
                (span * 0.64).clamp(148.0, 272.0),
            ),
            LightZoneKind::Room => (
                LightFixtureKind::Pendant,
                if span > 208.0 { 2 } else { 1 },
                (span * 0.70).clamp(88.0, 172.0),
            ),
            LightZoneKind::Corridor => (
                LightFixtureKind::Passage,
                (span / 132.0).ceil().clamp(1.0, 6.0) as usize,
                58.0,
            ),
            LightZoneKind::Blackout => (LightFixtureKind::Broken, 1, 27.0),
            LightZoneKind::Maze => continue,
        };
        for i in 0..count {
            let along = (i as f32 + 0.5) / count as f32 - 0.5;
            // Hanging lamps hang over the room's long axis; the arena's
            // staggered pairing is gone with the arena.
            let across = 0.0;
            let shift = along * span * 0.76;
            let intended = room.center()
                + if horizontal {
                    Vec2::new(shift, across)
                } else {
                    Vec2::new(across, shift)
                };
            let (x, y) = crate::game::collision::cell_of(intended);
            let usable = |cx, cy| {
                room.rect.contains(cx, cy)
                    && matches!(level.tiles.get(cx, cy), TileKind::Open)
                    && !level.mazes.iter().any(|maze| maze.bounds.contains(cx, cy))
            };
            let cell = if usable(x, y) {
                Some((x, y))
            } else {
                room.rect
                    .cells()
                    .filter(|&(cx, cy)| usable(cx, cy))
                    .min_by_key(|&(cx, cy)| (cx - x).pow(2) + (cy - y).pow(2))
            };
            if let Some((cx, cy)) = cell {
                let pos = TileMap::tile_rect(cx, cy).center();
                level.objects.lights.push(LightFixture {
                    pos,
                    kind,
                    radius,
                    seed: seed.wrapping_add(i as u32 * 127),
                });
            }
        }
    }
    place_emitters(level, rng);
}

fn place_emitters(level: &mut Level, rng: &mut rand::rngs::StdRng) {
    use EmitterKind::*;
    // Sparse, ordered placements. A room need not have an emitter at all.
    // The first entry prefers a blackout for the theme's signature vignette.
    let plan = match level.theme {
        Theme::Sewer => [Fire, Drip, Steam, Fumes],
        Theme::Kitchen => [Fire, Drip, Pilot, Exhaust],
        Theme::Lab => [Cable, Steam, Fumes, Dust],
        Theme::Laundry => [Fog, Steam, Drip, Insects],
        Theme::Printworks => [Cable, Dust, Exhaust, Pilot],
        Theme::Trailer => [Exhaust, Fire, Dust, Cable],
    };
    let (cx, cy) = crate::game::collision::cell_of(level.start);
    for (slot, kind) in plan.into_iter().enumerate() {
        let seed = rng.random::<u32>();
        let at = level
            .rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| {
                matches!(
                    room.kind,
                    RoomKind::Room | RoomKind::Hall | RoomKind::Corridor
                )
            })
            .filter_map(|(index, room)| {
                let world = room.rect.to_world();
                let edge = Vec2::new(
                    world.x + world.w * if seed & 1 == 0 { 0.30 } else { 0.73 },
                    world.y + world.h * if seed & 2 == 0 { 0.25 } else { 0.76 },
                );
                let (x, y) = crate::game::collision::cell_of(edge);
                let valid = |x, y| {
                    let pos = TileMap::tile_rect(x, y).center();
                    matches!(level.tiles.get(x, y), TileKind::Open)
                        && (x - cx).abs().max((y - cy).abs()) >= 13
                        && !level.rooms.iter().any(|r| {
                            matches!(r.kind, RoomKind::Boss | RoomKind::Start)
                                && r.rect.contains(x, y)
                        })
                        && !level.mazes.iter().any(|m| m.bounds.contains(x, y))
                        && level.pit_at(pos).is_none()
                        && !level
                            .objects
                            .shop_holes
                            .iter()
                            .any(|shop| shop.mouth == (x, y))
                        && level
                            .objects
                            .emitters
                            .iter()
                            .all(|e| e.pos.dist_sq(pos) > 100.0 * 100.0)
                };
                // Try a bounded neighbourhood rather than walking each room.
                let (x, y) = [
                    (0, 0),
                    (1, 0),
                    (0, 1),
                    (-1, 0),
                    (0, -1),
                    (2, 0),
                    (0, 2),
                    (-2, 0),
                    (0, -2),
                ]
                .into_iter()
                .map(|(dx, dy)| (x + dx, y + dy))
                .find(|&(x, y)| room.rect.contains(x, y) && valid(x, y))?;
                let dark = level
                    .objects
                    .light_zones
                    .iter()
                    .any(|z| z.kind == LightZoneKind::Blackout && z.rect == room.rect);
                let priority = match (slot, dark, room.kind) {
                    (0, true, _) => 0,
                    (1, _, RoomKind::Corridor) => 0,
                    (_, _, RoomKind::Hall | RoomKind::Room) => 1,
                    _ => 2,
                };
                let order = (index as u32)
                    .wrapping_mul(0x9e37_79b1)
                    .rotate_left(seed & 31)
                    ^ seed;
                Some(((priority, order), TileMap::tile_rect(x, y).center()))
            })
            .min_by_key(|a| a.0)
            .map(|(_, pos)| pos);
        if let Some(pos) = at {
            let intensity = if level.theme == Theme::Trailer && kind == Fire {
                0.39
            } else {
                1.0
            };
            level.objects.emitters.push(AmbientEmitter {
                kind,
                pos,
                seed,
                rate: 0.9 + (seed % 31) as f32 / 100.0,
                intensity,
            });
        }
    }
    for maze in &level.mazes {
        level.objects.emitters.push(AmbientEmitter {
            kind: Pilot,
            pos: maze.reward + Vec2::new(2.0, 0.0),
            seed: rng.random(),
            rate: 0.70,
            intensity: 0.51,
        });
    }
}

// ---- Wall-mounted lamps (batch 9, re-lit in batch 12) ---------------------
//
// The owner's note: *"walls should have light sources in some places"*. Every
// fixture above hangs over open floor; a **sconce** hangs on a structural wall
// face instead and throws a fan into the room (`LightFixtureKind::Sconce`), so
// a corridor reads as a rhythm of pools with dark stretches between them
// instead of one even dimness.
//
// Batch 12 (the owner: *"the wall lights need to be WAAAAY brighter"*) made the
// lamp a room light — a 110 px fan three times as bright, drawn as a 5–7 px
// emissive fitting — raised the budget to 40 a map, and gave **corridors their
// own admission rule**: nearly every corridor was a short door connector whose
// faces all sat inside the room rule's three-cell doorway clearance, so they
// carried no lamp at all. Nothing here reads a simulation value and nothing
// moves a golden or the map pin.
//
// Everything here runs on the stage's own salted rng, after every existing
// draw, and only appends to `objects.lights`. A fixture is a render-only input:
// no system reads the list, so nothing here can move a determinism golden. The
// map pin (`levelgen::mod::map_hash`) folds tiles, rat-holes, traps, doors and
// rewards — never fixtures — so it cannot move either. The batch-12 filters add
// no new rng draw; they only change which faces are admitted, which moves the
// draws that follow (accepted: fixture positions move, nothing else).

/// The most wall lamps one map may carry. A fixture costs the lighting pass one
/// cached shadow bake (regenerated only when the map's occluders change) plus
/// one splat while it overlaps the view, so a map's lamp count is a *light*
/// budget rather than a per-frame cost — but it is still bounded: a long
/// corridor map would stack them otherwise. A corridor contributes one per
/// 8–12 cells (two or three on a typical passage), a room one or two and a maze
/// at most two, so a campaign map collects two dozen or so and the flatbed's
/// rails carry at most
/// [`RAIL_LAMP_CAP`]. Batch 12 raised it from 26: the roles ask for about fifty
/// and the budget always bound, so corridors were starved.
const SCONCE_CAP: usize = 40;
/// How far a wall lamp's fan reaches into the room (world px). Batch 9 was a
/// modest 46 px pool; batch 12 made a wall lamp a room light, so the fan runs
/// 110 px — nearly seven tiles — with the soft edge falling to nothing at the
/// radius.
const SCONCE_RADIUS: f32 = 110.0;
/// A wall lamp keeps this many cells (Chebyshev) clear of a hanging Pendant or
/// Passage lamp, so the two pools never pile up on each other.
const SCONCE_LAMP_GAP: i32 = 3;
/// ...and this many cells from another wall lamp.
const SCONCE_GAP: i32 = 4;
/// ...and this many cells from any doorway cell: a threshold keeps the light it
/// has, and a lamp beside a doorway reads as a second exit.
const SCONCE_DOOR_GAP: i32 = 3;
/// A *corridor* face keeps only this many cells from a doorway (batch 12): most
/// corridors are short door connectors, and a three-cell clearance covers a
/// whole five-cell link, so the old rule left them dark.
const SCONCE_CORRIDOR_DOOR_GAP: i32 = 2;
/// A corridor face may sit under its own hanging Passage lamp's shoulder: the
/// room rule's three cells refuses every face of a short connector, because a
/// Passage lamp hangs in the middle of the corridor by construction.
const SCONCE_CORRIDOR_LAMP_GAP: i32 = 1;
/// A corridor wall must run this many cells straight (this cell included) to
/// carry a lamp: a fitting on the end of a one-cell nub between two doorways
/// reads as a stray pixel, not as a wall lamp.
const SCONCE_CORRIDOR_RUN: i32 = 4;
/// The walking rhythm of a corridor's lamps, in cells: one every 8–12 tiles,
/// alternating sides.
const SCONCE_STEP_MIN: i32 = 8;
const SCONCE_STEP_MAX: i32 = 12;
/// A room whose long wall is at least this long (cells) gets two wall lamps.
const SCONCE_LONG_ROOM: i32 = 13;
/// The flatbed's side rails carry at most this many marker lamps (8 in batch 9).
const RAIL_LAMP_CAP: usize = 12;
/// ...at least this many columns apart (a longer bed spreads them further).
const RAIL_LAMP_STEP: i32 = 11;

/// A structural wall face that borders walkable floor: the wall cell a lamp
/// hangs on, the direction its pool runs, and the floor cell it lights.
#[derive(Clone, Copy, Debug)]
struct Face {
    wall: (i32, i32),
    dir: Cardinal,
    floor: (i32, i32),
}

impl Face {
    /// Where the lamp hangs: on the face, half a pixel inside the floor cell it
    /// lights, so the cell it lands in is that floor and never its own wall.
    /// [`LightFixture::wall_facing`] reads exactly this position back.
    fn pos(self) -> Vec2 {
        TileMap::tile_rect(self.wall.0, self.wall.1).center()
            + cardinal_vec(self.dir) * (TILE_F * 0.5 + 0.5)
    }
}

/// Chebyshev cell distance between two cells.
fn cell_gap(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// Chebyshev cell distance between two world points.
fn world_gap(a: Vec2, b: Vec2) -> i32 {
    cell_gap(cell_of(a), cell_of(b))
}

/// Is `cell` within `gap` cells (Chebyshev) of any of `cells`?
fn near(cells: &[(i32, i32)], cell: (i32, i32), gap: i32) -> bool {
    cells.iter().any(|&c| cell_gap(c, cell) <= gap)
}

/// Chebyshev cell distance from a cell to a rectangle (0 when inside it).
fn rect_gap(rect: TileRect, cell: (i32, i32)) -> i32 {
    let dx = (rect.x - cell.0).max(cell.0 - (rect.right() - 1)).max(0);
    let dy = (rect.y - cell.1).max(cell.1 - (rect.bottom() - 1)).max(0);
    dx.max(dy)
}

/// How many of a cell's four neighbours are walkable floor.
fn open_neighbours(tiles: &TileMap, cell: (i32, i32)) -> usize {
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .filter(|&(dx, dy)| tiles.get(cell.0 + dx, cell.1 + dy) == TileKind::Open)
        .count()
}

/// Cells a lamp may never hang on or over: a rat-hole, a one-way door, a
/// mouse-hole shop (the cell it is carved into and its threshold).
fn blocked_cells(level: &Level) -> Vec<(i32, i32)> {
    let mut cells: Vec<(i32, i32)> = level.objects.rat_holes.iter().map(|h| h.cell).collect();
    cells.extend(level.objects.one_way_doors.iter().map(|d| d.cell));
    for shop in &level.objects.shop_holes {
        cells.push(shop.cell);
        cells.push(shop.mouth);
    }
    cells.sort_unstable();
    cells
}

/// Every doorway cell on the map.
fn doorway_cells(level: &Level) -> Vec<(i32, i32)> {
    level
        .tiles
        .iter_walls()
        .filter(|(_, _, kind)| matches!(kind, WallKind::Doorway { .. }))
        .map(|(x, y, _)| (x, y))
        .collect()
}

/// Every structural wall face bordering walkable floor, in the grid's
/// `(row, column)` scan order and a fixed normal order per cell — the order the
/// placement walks, so a seed's lamps are fixed. A face on a rat-hole, a maze
/// partition, a doorway or a one-way door is not structural and is skipped.
fn sconce_faces(level: &Level, blocked: &[(i32, i32)]) -> Vec<Face> {
    const DIRS: [Cardinal; 4] = [
        Cardinal::North,
        Cardinal::East,
        Cardinal::South,
        Cardinal::West,
    ];
    let mut faces = Vec::new();
    for (x, y, kind) in level.tiles.iter_walls() {
        if kind != WallKind::Solid || blocked.contains(&(x, y)) {
            continue;
        }
        for dir in DIRS {
            let (dx, dy) = cardinal_delta(dir);
            let floor = (x + dx, y + dy);
            // The pool needs floor to run into: a face onto a one-cell-wide
            // slot (the far side is wall again) is not a lamp's wall.
            let beyond = (floor.0 + dx, floor.1 + dy);
            if level.tiles.get(floor.0, floor.1) == TileKind::Open
                && level.tiles.get(beyond.0, beyond.1) == TileKind::Open
                && !blocked.contains(&floor)
            {
                faces.push(Face {
                    wall: (x, y),
                    dir,
                    floor,
                });
            }
        }
    }
    faces
}

/// Whether `face` may carry a lamp given the hanging lamps on the map and the
/// wall lamps already chosen.
fn admits(face: Face, hanging: &[Vec2], chosen: &[Face]) -> bool {
    let centre = TileMap::tile_rect(face.floor.0, face.floor.1).center();
    if hanging
        .iter()
        .any(|&p| world_gap(centre, p) <= SCONCE_LAMP_GAP)
    {
        return false;
    }
    !chosen
        .iter()
        .any(|c| cell_gap(c.floor, face.floor) <= SCONCE_GAP)
}

/// How many structural wall cells run straight through `wall` along the face's
/// own wall, counting `wall` itself. A corridor lamp wants at least
/// [`SCONCE_CORRIDOR_RUN`] of them (batch 12).
fn wall_run(tiles: &TileMap, wall: (i32, i32), face: Cardinal) -> i32 {
    let (dx, dy) = cardinal_delta(face);
    // The wall runs across the face normal.
    let (ax, ay) = (dy, dx);
    // Out of bounds reads as solid (`TileMap::get`), so the walk must stop at
    // the map edge itself or it would never terminate on a border wall.
    let solid = |x, y| tiles.in_bounds(x, y) && tiles.get(x, y) == TileKind::Wall(WallKind::Solid);
    let mut run = 1;
    for sign in [-1, 1] {
        let mut step = 1;
        while solid(wall.0 + ax * sign * step, wall.1 + ay * sign * step) {
            run += 1;
            step += 1;
        }
    }
    run
}

/// Whether `face` may carry a **corridor** lamp (batch 12).
///
/// A corridor is one or two cells wide, its hanging Passage lamp sits in the
/// middle of it by construction, and most of them are short door connectors —
/// so the room rule's three-cell doorway clearance and three-cell lamp
/// clearance refused almost every corridor face on a real map. The corridor
/// rule instead asks for a straight wall long enough to mount a fitting on, two
/// cells of clearance from any threshold, and the ordinary spacing from the
/// wall lamps already chosen. A rat-hole face, a shop hole, a one-way door and
/// a doorway tile never reach here: [`sconce_faces`] already dropped them.
fn admits_corridor(
    face: Face,
    tiles: &TileMap,
    doorways: &[(i32, i32)],
    hanging: &[Vec2],
    chosen: &[Face],
) -> bool {
    if near(doorways, face.floor, SCONCE_CORRIDOR_DOOR_GAP) {
        return false;
    }
    if wall_run(tiles, face.wall, face.dir) < SCONCE_CORRIDOR_RUN {
        return false;
    }
    let centre = TileMap::tile_rect(face.floor.0, face.floor.1).center();
    if hanging
        .iter()
        .any(|&p| world_gap(centre, p) <= SCONCE_CORRIDOR_LAMP_GAP)
    {
        return false;
    }
    !chosen
        .iter()
        .any(|c| cell_gap(c.floor, face.floor) <= SCONCE_GAP)
}

/// Batch 9: hang a sconce on some of the map's structural wall faces. Batch 12
/// raised the budget and widened what a corridor may ask for.
fn place_wall_lamps(level: &mut Level, rng: &mut rand::rngs::StdRng) {
    if level.theme == Theme::Trailer {
        // The flatbed has rails rather than walls.
        place_rail_lamps(level, rng);
        return;
    }
    let blocked = blocked_cells(level);
    let doorways = doorway_cells(level);
    let faces = sconce_faces(level, &blocked);
    if faces.is_empty() {
        return;
    }
    let hanging: Vec<Vec2> = level
        .objects
        .lights
        .iter()
        .filter(|f| matches!(f.kind, LightFixtureKind::Pendant | LightFixtureKind::Passage))
        .map(|f| f.pos)
        .collect();
    let mut chosen: Vec<Face> = Vec::new();
    // The cap still binds on some campaign maps (the roles would ask for about
    // fifty), so the order decides who is lit. Spend it where a lamp matters
    // most and then thin evenly instead of truncating the room list: the start
    // room, the corridors, every other room or hall, and only then the rooms in
    // between. Each group keeps the level's own room order.
    let mut order: Vec<&Room> = Vec::with_capacity(level.rooms.len());
    order.extend(level.rooms.iter().filter(|r| r.kind == RoomKind::Start));
    order.extend(level.rooms.iter().filter(|r| r.kind == RoomKind::Corridor));
    let living = || {
        level
            .rooms
            .iter()
            .filter(|r| matches!(r.kind, RoomKind::Room | RoomKind::Hall))
    };
    order.extend(living().step_by(2));
    order.extend(living().skip(1).step_by(2));
    for room in order {
        if chosen.len() >= SCONCE_CAP {
            break;
        }
        match room.kind {
            RoomKind::Corridor => {
                corridor_lamps(room, &level.tiles, &faces, &doorways, &hanging, &mut chosen, rng)
            }
            RoomKind::Room | RoomKind::Hall => {
                room_lamps(room, &faces, &doorways, &hanging, &mut chosen, rng)
            }
            RoomKind::Start => start_room_lamp(room, &faces, &doorways, &hanging, &mut chosen),
            // No generated map has one since batch 12; a hand-built arena room
            // gets the ordinary room treatment.
            RoomKind::Boss => room_lamps(room, &faces, &doorways, &hanging, &mut chosen, rng),
            RoomKind::Maze => {}
        }
    }
    for maze in &level.mazes {
        if chosen.len() >= SCONCE_CAP {
            break;
        }
        maze_lamps(&level.tiles, maze, &faces, &hanging, &mut chosen, &doorways);
    }
    if chosen.is_empty() {
        // Every map gets at least one wall lamp: the first face that clears the
        // hanging lamps. A map lit only from above would read as a miss.
        let fallback = faces.iter().copied().find(|f| {
            !near(&doorways, f.floor, SCONCE_DOOR_GAP)
                && admits(*f, &hanging, chosen.as_slice())
        });
        if let Some(face) = fallback {
            chosen.push(face);
        }
    }
    for face in chosen {
        level.objects.lights.push(LightFixture {
            pos: face.pos(),
            kind: LightFixtureKind::Sconce,
            radius: SCONCE_RADIUS,
            seed: rng.random(),
        });
    }
}

/// A corridor gets one lamp every 8–12 cells, alternating sides: the rhythm the
/// player reads a passage by. The faces are walked in `along` order, so the rng
/// draws (the step) come in a fixed order for a given map. Admission is
/// [`admits_corridor`], not the room rule.
fn corridor_lamps(
    room: &Room,
    tiles: &TileMap,
    faces: &[Face],
    doorways: &[(i32, i32)],
    hanging: &[Vec2],
    chosen: &mut Vec<Face>,
    rng: &mut rand::rngs::StdRng,
) {
    let horizontal = room.rect.w >= room.rect.h;
    let mut list: Vec<(i32, i32, Face)> = faces
        .iter()
        .filter(|f| room.rect.contains(f.floor.0, f.floor.1))
        .filter_map(|f| {
            // Side: -1 for the north/west wall, +1 for the south/east one.
            let side = match (horizontal, f.dir) {
                (true, Cardinal::North) => -1,
                (true, Cardinal::South) => 1,
                (false, Cardinal::West) => -1,
                (false, Cardinal::East) => 1,
                _ => return None,
            };
            let along = if horizontal { f.floor.0 } else { f.floor.1 };
            Some((along, side, *f))
        })
        .collect();
    list.sort_unstable_by_key(|(along, side, f)| (*along, *side, f.wall));
    let mut next = i32::MIN;
    let mut last_side = 0;
    for (along, side, face) in list {
        if chosen.len() >= SCONCE_CAP {
            return;
        }
        if along < next
            || side == last_side
            || !admits_corridor(face, tiles, doorways, hanging, chosen.as_slice())
        {
            continue;
        }
        chosen.push(face);
        last_side = side;
        next = along + rng.random_range(SCONCE_STEP_MIN..=SCONCE_STEP_MAX);
    }
}

/// A room or hall gets one lamp (two when its long wall is long enough) on a
/// **longer** wall, a cell in from the corners and away from its doorways. The
/// wall is split into equal spans and one face is drawn from each.
fn room_lamps(
    room: &Room,
    faces: &[Face],
    doorways: &[(i32, i32)],
    hanging: &[Vec2],
    chosen: &mut Vec<Face>,
    rng: &mut rand::rngs::StdRng,
) {
    let horizontal = room.rect.w >= room.rect.h;
    let (lo, hi) = if horizontal {
        (room.rect.x, room.rect.right())
    } else {
        (room.rect.y, room.rect.bottom())
    };
    let mut list: Vec<(i32, Face)> = faces
        .iter()
        .filter(|f| room.rect.contains(f.floor.0, f.floor.1))
        .filter(|f| {
            matches!(
                (horizontal, f.dir),
                (true, Cardinal::North | Cardinal::South) | (false, Cardinal::East | Cardinal::West)
            )
        })
        .filter(|f| !near(doorways, f.floor, SCONCE_DOOR_GAP))
        .map(|f| {
            let along = if horizontal { f.floor.0 } else { f.floor.1 };
            (along, *f)
        })
        .filter(|(along, _)| *along > lo && *along < hi - 1)
        .collect();
    list.sort_unstable_by_key(|(along, f)| (*along, f.wall));
    let len = list.len();
    if len == 0 {
        return;
    }
    let span = if horizontal { room.rect.w } else { room.rect.h };
    let count = if span >= SCONCE_LONG_ROOM { 2 } else { 1 };
    for i in 0..count {
        if chosen.len() >= SCONCE_CAP {
            return;
        }
        let start = len * i / count;
        let end = (len * (i + 1) / count).max(start + 1).min(len);
        let face = list[start + rng.random_range(0..(end - start))].1;
        if admits(face, hanging, chosen.as_slice()) {
            chosen.push(face);
        }
    }
}

/// The start room gets one lamp a few cells beside its exit, so the way out is
/// readable on the first frame.
fn start_room_lamp(
    room: &Room,
    faces: &[Face],
    doorways: &[(i32, i32)],
    hanging: &[Vec2],
    chosen: &mut Vec<Face>,
) {
    // Its exit: the doorway nearest the room's own rectangle.
    let Some(exit) = doorways
        .iter()
        .copied()
        .filter(|&c| room.rect.inflate(2).contains(c.0, c.1))
        .min_by_key(|&c| (rect_gap(room.rect, c), c))
    else {
        return;
    };
    let lamp = faces
        .iter()
        .filter(|f| room.rect.contains(f.floor.0, f.floor.1))
        .filter(|f| ((SCONCE_DOOR_GAP + 1)..=6).contains(&cell_gap(f.floor, exit)))
        .filter(|f| !near(doorways, f.floor, SCONCE_DOOR_GAP))
        .filter(|f| admits(**f, hanging, chosen.as_slice()))
        .min_by_key(|f| (cell_gap(f.floor, exit), f.wall));
    if let Some(face) = lamp {
        chosen.push(*face);
    }
}

/// A maze's walls get a sparse pair facing its junctions only: the dark between
/// the crossings is the maze's whole point.
fn maze_lamps(
    tiles: &TileMap,
    maze: &MazeSection,
    faces: &[Face],
    hanging: &[Vec2],
    chosen: &mut Vec<Face>,
    doorways: &[(i32, i32)],
) {
    let mut junctions: Vec<Face> = faces
        .iter()
        .copied()
        .filter(|f| maze.bounds.contains(f.floor.0, f.floor.1))
        .filter(|f| open_neighbours(tiles, f.floor) >= 3)
        .filter(|f| !near(doorways, f.floor, SCONCE_DOOR_GAP))
        .filter(|f| admits(*f, hanging, chosen.as_slice()))
        .collect();
    junctions.sort_unstable_by_key(|f| (f.floor, f.wall));
    for face in junctions.into_iter().take(2) {
        if chosen.len() >= SCONCE_CAP {
            return;
        }
        chosen.push(face);
    }
}

/// The flatbed's marker lamps: an orange one along the side rails, alternating
/// sides, spread over the length of the bed and never over a missing rail
/// section or a broken plank.
fn place_rail_lamps(level: &mut Level, rng: &mut rand::rngs::StdRng) {
    let deck = level.trailer.deck;
    if deck.w <= 4 {
        return;
    }
    let step = ((deck.w - 4) / RAIL_LAMP_CAP as i32).max(RAIL_LAMP_STEP);
    let rows = [(deck.y - 1, Cardinal::South), (deck.bottom(), Cardinal::North)];
    let mut side = 0usize;
    let mut x = deck.x + 2;
    let mut placed = 0usize;
    while x < deck.right() - 2 && placed < RAIL_LAMP_CAP {
        let (row, dir) = rows[side % 2];
        let normal = cardinal_delta(dir);
        let face = Face {
            wall: (x, row),
            dir,
            floor: (x + normal.0, row + normal.1),
        };
        let wall_ok = level.tiles.get(face.wall.0, face.wall.1) == TileKind::Wall(WallKind::Solid);
        let floor_ok = level.tiles.get(face.floor.0, face.floor.1) == TileKind::Open;
        let clear = !level.trailer.pits.iter().any(|pit| {
            pit.rect.contains(face.wall.0, face.wall.1)
                || pit.rect.contains(face.floor.0, face.floor.1)
        });
        if wall_ok && floor_ok && clear {
            level.objects.lights.push(LightFixture {
                pos: face.pos(),
                kind: LightFixtureKind::Sconce,
                radius: SCONCE_RADIUS,
                seed: rng.random(),
            });
            placed += 1;
            side += 1;
        }
        x += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallAxis;
    use crate::game::levelgen::generate_once;
    use crate::game::levelgen::validate::validate;

    /// Twenty seeds on every playable floor: the batch's placement contract.
    fn maps() -> impl Iterator<Item = (Theme, Level)> {
        Theme::playable()
            .into_iter()
            .flat_map(|theme| (0..20u64).map(move |seed| (theme, generate_once(theme, seed, 0))))
    }

    fn sconces(level: &Level) -> Vec<&LightFixture> {
        level
            .objects
            .lights
            .iter()
            .filter(|f| f.kind == LightFixtureKind::Sconce)
            .collect()
    }

    /// The doorway clearance a lamp on this floor cell must keep: a corridor may
    /// come within two cells of a threshold, anything else keeps three.
    fn door_gap_for(level: &Level, floor: (i32, i32)) -> i32 {
        let corridor = level
            .rooms
            .iter()
            .any(|r| r.kind == RoomKind::Corridor && r.rect.contains(floor.0, floor.1));
        if corridor {
            SCONCE_CORRIDOR_DOOR_GAP
        } else {
            SCONCE_DOOR_GAP
        }
    }

    /// The pre-batch-9 stage alone, on the stage's own salt: the baseline the
    /// wall lamps must not move.
    fn hanging_stage_only(level: &mut Level, attempt: u32) {
        let mut rng = level_rng(level.theme, level.seed ^ 0x4c49_4748_5453, attempt);
        place_hanging_lights(level, &mut rng);
    }

    #[test]
    fn walls_carry_lamps_where_they_border_floor() {
        for (theme, level) in maps() {
            let ctx = (theme, level.seed);
            let lamps = sconces(&level);
            assert!(
                lamps.len() <= SCONCE_CAP,
                "{ctx:?}: {} wall lamps, cap {SCONCE_CAP}",
                lamps.len()
            );
            if theme == Theme::Trailer {
                assert!(lamps.len() <= RAIL_LAMP_CAP, "{ctx:?}");
            } else {
                assert!(!lamps.is_empty(), "{ctx:?}: a map with no wall lamp");
            }
            let hanging: Vec<Vec2> = level
                .objects
                .lights
                .iter()
                .filter(|f| matches!(f.kind, LightFixtureKind::Pendant | LightFixtureKind::Passage))
                .map(|f| f.pos)
                .collect();
            let blocked = blocked_cells(&level);
            let doorways = doorway_cells(&level);
            for (i, lamp) in lamps.iter().enumerate() {
                assert_eq!(
                    level.tiles.get_at(lamp.pos),
                    TileKind::Open,
                    "{ctx:?}: {lamp:?}"
                );
                assert!(level.pit_at(lamp.pos).is_none(), "{ctx:?}: {lamp:?}");
                assert!(lamp.radius > 0.0, "{ctx:?}: {lamp:?}");
                // The wall it hangs on, facing the floor its pool runs into.
                let dir = lamp.wall_facing(&level.tiles);
                assert_eq!(dir.len(), 1.0, "{ctx:?}: {dir:?}");
                let (fx, fy) = cell_of(lamp.pos);
                let (dx, dy) = (dir.x as i32, dir.y as i32);
                assert_eq!(
                    level.tiles.get(fx - dx, fy - dy),
                    TileKind::Wall(WallKind::Solid),
                    "{ctx:?}: {lamp:?} is not on a structural wall face"
                );
                assert_eq!(
                    level.tiles.get(fx + dx, fy + dy),
                    TileKind::Open,
                    "{ctx:?}: {lamp:?} does not face floor"
                );
                // Never on a hole, a door, a threshold or a doorway's doorstep.
                // A corridor face may come two cells from a threshold instead
                // of three (batch 12), so the clearance is read per lamp.
                assert!(!blocked.contains(&(fx, fy)), "{ctx:?}: {lamp:?}");
                assert!(
                    !near(&doorways, (fx, fy), door_gap_for(&level, (fx, fy))),
                    "{ctx:?}: {lamp:?}"
                );
                // Clear of the hanging lamps and of the other wall lamps. A
                // corridor face may sit under its own Passage lamp's shoulder
                // (batch 12), so the clearance is read per lamp too.
                let centre = TileMap::tile_rect(fx, fy).center();
                let lamp_gap = if door_gap_for(&level, (fx, fy)) == SCONCE_CORRIDOR_DOOR_GAP {
                    SCONCE_CORRIDOR_LAMP_GAP
                } else {
                    SCONCE_LAMP_GAP
                };
                for &h in &hanging {
                    assert!(
                        world_gap(centre, h) > lamp_gap,
                        "{ctx:?}: {lamp:?} crowds a hanging lamp"
                    );
                }
                for (other_i, other) in lamps.iter().enumerate() {
                    if other_i != i {
                        assert!(
                            world_gap(lamp.pos, other.pos) > SCONCE_GAP,
                            "{ctx:?}: {lamp:?} crowds {other:?}"
                        );
                    }
                }
            }
            assert!(
                validate(&level).is_ok(),
                "{ctx:?}: {:?}",
                validate(&level)
            );
        }
    }

    /// The counts the batch is allowed to spend, over 20 seeds of every
    /// playable floor: no map over its cap, and corridors do get lamps.
    #[test]
    fn wall_lamp_counts_stay_within_the_new_caps() {
        let mut corridor_lamps = 0usize;
        let mut most = 0usize;
        for (theme, level) in maps() {
            let lamps = sconces(&level);
            let cap = if theme == Theme::Trailer {
                RAIL_LAMP_CAP
            } else {
                SCONCE_CAP
            };
            assert!(
                lamps.len() <= cap,
                "{theme:?} {}: {} wall lamps over the cap {cap}",
                level.seed,
                lamps.len()
            );
            most = most.max(lamps.len());
            for lamp in &lamps {
                let cell = cell_of(lamp.pos);
                if door_gap_for(&level, cell) == SCONCE_CORRIDOR_DOOR_GAP {
                    corridor_lamps += 1;
                }
            }
        }
        // Batch 12 exists because corridors were dark; the sweep has to show
        // lamps in them at all.
        assert!(
            corridor_lamps > 0,
            "no corridor lamp in {} maps",
            Theme::playable().len() * 20
        );
        assert!(
            most > SCONCE_CAP / 2,
            "the cap {SCONCE_CAP} is not being spent (most {most} lamps)"
        );
    }

    /// A hand-built corridor: `len` cells long and two wide, walled on both
    /// sides with a two-cell doorway in each end cap. The rest of the map is
    /// open floor with no walls, so the corridor is the only place a lamp can
    /// go (and there are no hanging lamps to refuse a face).
    fn corridor_level(len: i32) -> Level {
        let mut level = Level::empty(Theme::Sewer);
        let (x0, y0) = (20, 40);
        let wall = TileKind::Wall(WallKind::Solid);
        level.tiles.fill_cells(x0 - 1, y0 - 1, len + 2, 1, wall);
        level.tiles.fill_cells(x0 - 1, y0 + 2, len + 2, 1, wall);
        for x in [x0 - 1, x0 + len] {
            level.tiles.fill_cells(x, y0 - 1, 1, 4, wall);
            for y in [y0, y0 + 1] {
                level.tiles.set(
                    x,
                    y,
                    TileKind::Wall(WallKind::Doorway {
                        axis: WallAxis::Vertical,
                    }),
                );
            }
        }
        level
            .rooms
            .push(Room::new(TileRect::new(x0, y0, len, 2), RoomKind::Corridor));
        let mut rng = level_rng(level.theme, level.seed ^ 0x4c49_4748_5453, 0);
        place_wall_lamps(&mut level, &mut rng);
        level
    }

    /// A door connector six cells long: every face of it is inside the room
    /// rule's three-cell doorway clearance, so batch 9 left these dark. The
    /// corridor rule (two cells of clearance, four cells of straight wall)
    /// lights it.
    #[test]
    fn a_corridor_with_a_four_tile_straight_wall_gets_a_lamp() {
        let level = corridor_level(6);
        let lamps = sconces(&level);
        assert_eq!(lamps.len(), 1, "corridor lamps: {lamps:?}");
        let cell = cell_of(lamps[0].pos);
        assert!(
            TileRect::new(20, 40, 6, 2).contains(cell.0, cell.1),
            "lamp {cell:?} is outside the corridor"
        );
        // Three cells from each doorway: the room rule would have refused it.
        let doorways = doorway_cells(&level);
        assert_eq!(doorways.len(), 4, "{doorways:?}");
        assert!(
            !near(&doorways, cell, SCONCE_DOOR_GAP),
            "the fixture sits inside the old doorway clearance"
        );
        assert!(near(&doorways, cell, SCONCE_CORRIDOR_DOOR_GAP + 1));
        // On a wall at least four cells long, which is the corridor rule.
        let dir = lamps[0].wall_facing(&level.tiles);
        let wall = (cell.0 - dir.x as i32, cell.1 - dir.y as i32);
        assert!(wall_run(&level.tiles, wall, Cardinal::South) >= SCONCE_CORRIDOR_RUN);
    }

    /// The straight-wall rule: a six-cell run counts six, a doorway in it
    /// breaks the count, and a short corridor carries no lamp at all.
    #[test]
    fn a_corridor_lamp_needs_four_cells_of_straight_wall() {
        let mut level = Level::empty(Theme::Sewer);
        let wall = TileKind::Wall(WallKind::Solid);
        level.tiles.fill_cells(10, 10, 6, 1, wall);
        assert_eq!(wall_run(&level.tiles, (13, 10), Cardinal::South), 6);
        assert_eq!(wall_run(&level.tiles, (10, 10), Cardinal::South), 6);
        assert_eq!(wall_run(&level.tiles, (13, 10), Cardinal::East), 1);
        level.tiles.set(
            14,
            10,
            TileKind::Wall(WallKind::Doorway {
                axis: WallAxis::Horizontal,
            }),
        );
        // The doorway breaks the run: four cells to its west, one to its east.
        assert_eq!(wall_run(&level.tiles, (13, 10), Cardinal::South), 4);
        assert_eq!(wall_run(&level.tiles, (15, 10), Cardinal::South), 1);
        for len in 1..=3 {
            assert!(sconces(&corridor_level(len)).is_empty(), "len {len}");
        }
    }

    #[test]
    fn wall_lamps_are_placed_deterministically() {
        for theme in Theme::playable() {
            for seed in 0..20u64 {
                let a = generate_once(theme, seed, 0);
                let b = generate_once(theme, seed, 0);
                assert_eq!(a.objects.lights, b.objects.lights, "{theme:?} {seed}");
            }
        }
    }

    /// The new stage may only *append*: the hanging lamps, the zones and the
    /// emitters are exactly what the pre-batch-9 stage produces on its own.
    #[test]
    fn sconces_do_not_move_the_hanging_lamps() {
        for theme in Theme::playable() {
            for seed in 0..20u64 {
                let full = generate_once(theme, seed, 0);
                let hanging: Vec<LightFixture> = full
                    .objects
                    .lights
                    .iter()
                    .copied()
                    .filter(|f| f.kind != LightFixtureKind::Sconce)
                    .collect();
                let mut rebuilt = generate_once(theme, seed, 0);
                rebuilt.objects.lights.clear();
                rebuilt.objects.light_zones.clear();
                rebuilt.objects.emitters.clear();
                hanging_stage_only(&mut rebuilt, 0);
                assert_eq!(rebuilt.objects.lights, hanging, "{theme:?} {seed}");
                assert_eq!(
                    rebuilt.objects.light_zones, full.objects.light_zones,
                    "{theme:?} {seed}"
                );
                assert_eq!(
                    rebuilt.objects.emitters, full.objects.emitters,
                    "{theme:?} {seed}"
                );
            }
        }
    }
}
