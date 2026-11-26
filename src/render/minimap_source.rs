//! Adapter from the simulation to the minimap art module (`render/minimap.rs`).
//!
//! `render/minimap.rs` owns the minimap's whole look: the `MinimapSource`
//! trait it reads the world through, the `MinimapView` snapshot it renders,
//! the compact/expanded panel rectangles and the `draw` entry point. This file
//! is the other half:
//!
//! * [`MinimapSource`] is implemented for the [`World`] facade from the
//!   fog-of-war bitset and the authoritative tile grid, allocation-free;
//! * [`minimap_cell`] maps a tile cell (plus traps) to the art's
//!   [`MinimapCell`];
//! * [`MinimapFrame`] rebuilds the per-frame marker/label lists into reused
//!   buffers (no per-frame allocation) and produces the [`MinimapView`].
//!
//! Discovery rules (game designer's review) live here because they are data,
//! not style: an undiscovered maze contributes nothing, a found maze keeps a
//! hollow cache marker at its breach until the cache is claimed, a rat-hole's
//! far mouth is only "known" once it has been seen or crossed. The old boss
//! arena bearing is gone (batch 12 removed the arena): [`boss_bearing_dir`]
//! always answers `None`.
//!
//! **Ink lives in the art module.** Nothing here names a colour, so the corner
//! radar and the expanded Tab map cannot drift apart: both paint the unseen
//! veil from `minimap::unknown_veil_rgb_for` through one `DerivedPalette`.

use ratatui::layout::Rect;

use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
use crate::game::collision::{cell_of, TileKind, TILE_F};
use crate::game::gem::{Chest, Pickup, PickupKind};
use crate::game::level::{Level, RoomKind, Theme};
use crate::game::world::World;
use crate::render::minimap::{
    self, LabelKind, MapLabel, MapMarker, MarkerKind, MinimapAxis, MinimapCell, MinimapDir,
    MinimapMode, MinimapPalette, MinimapSize, MinimapSource, MinimapView, PickupTag,
};

/// **Retired (batch 12).** The old arena bearing appeared this many seconds
/// into a run. Kept only because `App`'s HUD tests still name it; delete it with
/// [`boss_bearing_dir`].
pub const ARENA_BEARING_AFTER_SECS: f32 = 180.0;

/// Up to this many pickup blips are marked per frame (the buffers are reused,
/// so this is a hard bound, not an allocation).
pub const MAX_PICKUP_MARKERS: usize = 24;

/// Markers + labels for one frame: owned buffers the App keeps between frames.
///
/// `refresh` clears and refills them, so after the first frames the whole
/// minimap costs no heap allocation (matching the art module's own promise).
pub struct MinimapFrame {
    markers: Vec<MapMarker>,
    labels: Vec<MapLabel<'static>>,
}

impl Default for MinimapFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl MinimapFrame {
    pub fn new() -> Self {
        MinimapFrame {
            markers: Vec::with_capacity(8 + MAX_PICKUP_MARKERS),
            labels: Vec::with_capacity(4),
        }
    }

    /// Number of markers built for the last frame (tests).
    pub fn marker_count(&self) -> usize {
        self.markers.len()
    }

    /// The markers built for the last frame (tests and tooling).
    pub fn markers(&self) -> &[MapMarker] {
        &self.markers
    }

    /// The labels built for the last frame (tests and tooling).
    pub fn labels(&self) -> &[MapLabel<'static>] {
        &self.labels
    }

    /// Rebuild the marker and label lists from the world's discovery state.
    pub fn refresh(&mut self, world: &World) {
        self.markers.clear();
        self.labels.clear();
        let fog = world.fog();

        // Start room origin (expanded map draws it).
        let start_cell = cell_of(world.level.start);
        self.markers.push(MapMarker {
            cx: start_cell.0 as f32,
            cy: start_cell.1 as f32,
            kind: MarkerKind::Start,
        });
        if fog.is_seen(start_cell.0, start_cell.1) {
            self.labels.push(MapLabel {
                cx: start_cell.0 as f32,
                cy: start_cell.1 as f32,
                text: "START",
                kind: LabelKind::Start,
                discovered: true,
            });
        }

        // The living boss: a threat blip in both modes.
        if let Some(pos) = world.boss_marker() {
            let (cx, cy) = cell_of(pos);
            self.markers.push(MapMarker {
                cx: cx as f32,
                cy: cy as f32,
                kind: MarkerKind::Boss,
            });
        }
        // The living named miniboss: its own amber blip (wave 3).
        if let Some(pos) = world.miniboss_marker() {
            let (cx, cy) = cell_of(pos);
            self.markers.push(MapMarker {
                cx: cx as f32,
                cy: cy as f32,
                kind: MarkerKind::Miniboss,
            });
        }

        // Mazes: nothing at all until the breach is seen; then a persistent
        // entrance marker plus a hollow (or filled) cache symbol; the reward
        // pedestal itself only while its own cell is explored.
        for maze in world.maze_markers() {
            // The prize pedestal is its own landmark: it shows as soon as its
            // cell is explored, whether or not the breach was found first.
            if maze.reward_seen {
                let (rx, ry) = cell_of(maze.reward);
                self.markers.push(MapMarker {
                    cx: rx as f32,
                    cy: ry as f32,
                    kind: MarkerKind::MazeReward,
                });
            }
            if !maze.entrance_seen {
                continue;
            }
            let (ex, ey) = maze.entrance;
            self.markers.push(MapMarker {
                cx: ex as f32,
                cy: ey as f32,
                kind: MarkerKind::MazeEntrance,
            });
            self.markers.push(MapMarker {
                cx: ex as f32,
                cy: ey as f32,
                kind: if maze.claimed {
                    MarkerKind::MazeCacheClaimed
                } else {
                    MarkerKind::MazeCacheHollow
                },
            });
            self.labels.push(MapLabel {
                cx: ex as f32,
                cy: ey as f32,
                text: "MAZE",
                kind: LabelKind::Maze,
                discovered: true,
            });
        }

        // Mouse-hole shops: the spoon is revealed on sight and then remembered
        // (the fog's `seen` bit is sticky, so a visited hole keeps its marker
        // for the rest of the run and a hole nobody has walked past stays
        // hidden).
        for hole in &world.level.objects.shop_holes {
            let (hx, hy) = cell_of(hole.pos);
            if fog.is_seen(hx, hy) {
                self.markers.push(MapMarker {
                    cx: hx as f32,
                    cy: hy as f32,
                    kind: shop_marker_kind(world, hole.index),
                });
            }
        }

        // Mark chests first so a crowded map always has room for them.
        let mut pickups = 0;
        for entity in world.ecs.iter_entities() {
            if let Some(chest) = entity.get::<Chest>() {
                let (cx, cy) = cell_of(chest.pos);
                if fog.is_seen(cx, cy) {
                    self.markers.push(MapMarker {
                        cx: cx as f32,
                        cy: cy as f32,
                        kind: MarkerKind::Pickup(PickupTag::Chest),
                    });
                    pickups += 1;
                    if pickups >= MAX_PICKUP_MARKERS {
                        return;
                    }
                }
            }
        }

        // A chest sitting on a set mousetrap's plate is a landmark in its own
        // right: the chest marker is the same blip a dropped chest gets, so a
        // player reading the map sees the reward waiting behind the bar.
        for trap in &world.level.objects.mousetraps {
            if trap.bait != crate::game::art_traps::TrapBait::Chest || trap.bait_taken {
                continue;
            }
            let (cx, cy) = cell_of(trap.pos);
            if fog.is_seen(cx, cy) {
                self.markers.push(MapMarker {
                    cx: cx as f32,
                    cy: cy as f32,
                    kind: MarkerKind::Pickup(PickupTag::Chest),
                });
                pickups += 1;
                if pickups >= MAX_PICKUP_MARKERS {
                    return;
                }
            }
        }

        // Floor pickups on explored ground, in `SpawnId`-independent order (the
        // art only needs the positions; ties do not matter for a blip).
        for entity in world.ecs.iter_entities() {
            let Some(pickup) = entity.get::<Pickup>() else {
                continue;
            };
            let (cx, cy) = cell_of(pickup.pos);
            if !fog.is_seen(cx, cy) {
                continue;
            }
            // Every find keeps its own icon: the map must never merge two
            // different pocket finds into one "good for you" blip.
            let tag = match pickup.kind {
                PickupKind::Heal => PickupTag::Heal,
                PickupKind::Magnet => PickupTag::Magnet,
                PickupKind::Nuke => PickupTag::Nuke,
                PickupKind::BottleCap => PickupTag::BottleCap,
                PickupKind::WindupMouse => PickupTag::WindupMouse,
                PickupKind::PocketWatch => PickupTag::PocketWatch,
                PickupKind::CleanSocks => PickupTag::CleanSocks,
                PickupKind::GrowthSerum => PickupTag::GrowthSerum,
                PickupKind::ShrinkDraught => PickupTag::ShrinkDraught,
                PickupKind::WhiteCoat => PickupTag::WhiteCoat,
                PickupKind::TheCollar => PickupTag::Collar,
            };
            self.markers.push(MapMarker {
                cx: cx as f32,
                cy: cy as f32,
                kind: MarkerKind::Pickup(tag),
            });
            pickups += 1;
            if pickups >= MAX_PICKUP_MARKERS {
                break;
            }
        }
    }

    /// Build the per-frame view for `world` using the default `Normal` compact
    /// size (borrows the refreshed buffers).
    pub fn view(&self, world: &World, expanded: bool, hovered: bool) -> MinimapView<'_> {
        self.view_sized(world, expanded, MinimapSize::Normal, hovered)
    }

    /// Build the per-frame view for `world` with an explicit saved
    /// [`MinimapSize`] preference (borrows the refreshed buffers).
    pub fn view_sized(
        &self,
        world: &World,
        expanded: bool,
        compact_size: MinimapSize,
        hovered: bool,
    ) -> MinimapView<'_> {
        let palette = match world.level.theme {
            Theme::Sewer => MinimapPalette::SEWER,
            Theme::Kitchen => MinimapPalette::KITCHEN,
            Theme::Lab => MinimapPalette::LAB,
            Theme::Laundry => MinimapPalette::LAUNDRY,
            Theme::Printworks => MinimapPalette::PRINTWORKS,
            Theme::Trailer => MinimapPalette::TRAILER,
            Theme::Outfall => MinimapPalette::OUTFALL,
        };
        let cols = world.level.tiles.cols().max(1);
        let rows = world.level.tiles.rows().max(1);
        // The brackets frame the *rendered* span (logical viewport × the
        // mouse-wheel zoom), so zooming out widens the rectangle on the radar
        // exactly as it widens what the player sees.
        let span = world.view_span();
        let seen = world.fog().seen_count();
        let pct = ((seen * 100) / (cols as usize * rows as usize)).min(100) as u8;
        let maze_discovered = world
            .maze_markers()
            .any(|m| m.entrance_seen || m.reward_seen);
        // Mirror `overlay::draw_boss_compass`: the HUD edge compass already
        // points at the living boss once it is off screen, so `Hidden` mode does
        // not duplicate that cue.
        let hud_compass_active = world.boss_offset().is_some()
            || (!world.director.boss_spawned
                && world.elapsed
                    >= world.tuning().director.boss_time - crate::render::overlay::BOSS_WARNING_SECS
                && world.elapsed < world.tuning().director.boss_time);
        MinimapView {
            player_cell: (
                world.player.pos.x / TILE_F,
                world.player.pos.y / TILE_F,
            ),
            player_facing: (world.player.face.x, world.player.face.y),
            viewport_cells: minimap::CellRect {
                x: world.cam.x / TILE_F,
                y: world.cam.y / TILE_F,
                w: span.x / TILE_F,
                h: span.y / TILE_F,
            },
            markers: &self.markers,
            labels: &self.labels,
            boss_bearing_dir: boss_bearing_dir(world),
            palette,
            sector_name: world.level.theme.name(),
            clock: world.elapsed,
            player_clock: 0.0,
            mode: if expanded {
                MinimapMode::Expanded
            } else {
                MinimapMode::Compact
            },
            compact_size,
            hud_compass_active,
            hovered,
            explored_pct: Some(pct),
            maze_discovered,
            map_key: minimap::DEFAULT_MAP_KEY,
            size_key: minimap::DEFAULT_SIZE_KEY,
            reveal_clock: world.fog().clock(),
        }
    }
}

/// **Retired (batch 12).** The old pre-boss bearing pointed at the map's boss
/// arena from 3:00 until it was discovered. There is no arena any more — the
/// boss arrives where the rat is — so there is nothing to point at and this
/// always returns `None`. It is kept only because `App`'s HUD tests still call
/// it and read [`ARENA_BEARING_AFTER_SECS`]; delete both once those tests go.
pub fn boss_bearing_dir(_world: &World) -> Option<(f32, f32)> {
    None
}

/// The compact panel rect for a play field (`render/minimap.rs` owns the
/// geometry; the App only needs it for the hit map and for tests).
pub fn compact_rect(game: Rect) -> Rect {
    minimap::compact_rect(game)
}

/// The compact panel rect for a play field and saved [`MinimapSize`] preference.
pub fn compact_rect_for_size(game: Rect, size: MinimapSize) -> Rect {
    minimap::compact_rect_for_size(game, size)
}

/// The expanded panel rect for a play field.
pub fn expanded_rect(game: Rect) -> Rect {
    minimap::expanded_rect(game)
}

/// The art's cell classification for one tile cell.
pub fn minimap_cell(level: &Level, cx: i32, cy: i32) -> MinimapCell {
    match level.tiles.get(cx, cy) {
        TileKind::Wall(WallKind::Solid) => MinimapCell::Wall,
        TileKind::Wall(WallKind::Maze) => MinimapCell::MazeWall,
        TileKind::Wall(WallKind::Doorway { .. }) => MinimapCell::Doorway,
        TileKind::Wall(WallKind::OneWayDoor { dir }) => {
            MinimapCell::OneWayDoor(minimap_dir(dir))
        }
        TileKind::Wall(WallKind::RatHole { axis }) => MinimapCell::RatHole(minimap_axis(axis)),
        TileKind::Open => {
            let p = crate::game::collision::TileMap::tile_rect(cx, cy).center();
            if level.theme == Theme::Outfall {
                if level.pit_at(p).is_some() {
                    return MinimapCell::TidalDrop;
                }
                if level.objects.lure_traps.iter().any(|trap| {
                    trap.is_lit() && crate::game::collision::cell_of(trap.pos) == (cx, cy)
                }) {
                    return MinimapCell::Lure;
                }
            }
            if let Some(trap) = trap_at(level, p) {
                trap
            } else if level
                .boss_room()
                .is_some_and(|room| room.rect.contains(cx, cy))
            {
                MinimapCell::BossFloor
            } else {
                MinimapCell::Floor
            }
        }
    }
}

#[inline]
fn minimap_dir(dir: Cardinal) -> MinimapDir {
    match dir {
        Cardinal::North => MinimapDir::North,
        Cardinal::East => MinimapDir::East,
        Cardinal::South => MinimapDir::South,
        Cardinal::West => MinimapDir::West,
    }
}

#[inline]
fn minimap_axis(axis: WallAxis) -> MinimapAxis {
    match axis {
        WallAxis::Horizontal => MinimapAxis::Horizontal,
        WallAxis::Vertical => MinimapAxis::Vertical,
    }
}

/// The floor-trap mark for a cell centre, if a trap covers it.
///
/// A **spent** glue board and a **sprung** snap trap carry no promise any more,
/// so they leave the plain floor; a hair-trigger snap trap gets its own redder
/// mark (cheap, and the one trap whose risk a player wants on the map).
fn trap_at(level: &Level, p: crate::math::Vec2) -> Option<MinimapCell> {
    use crate::game::art_traps::{StickyTrapState, MOUSETRAP_SIZE, STICKY_TRAP_SIZE};
    let sticky_hx = STICKY_TRAP_SIZE.x * 0.5;
    let sticky_hy = STICKY_TRAP_SIZE.y * 0.5;
    if level.objects.sticky_traps.iter().any(|t| {
        t.state != StickyTrapState::Used
            && crate::game::art_traps::local_box_contains(
                t.pos,
                t.facing,
                p,
                -sticky_hx,
                sticky_hx,
                sticky_hy,
            )
    }) {
        return Some(MinimapCell::Trap);
    }
    let half = MOUSETRAP_SIZE * 0.5;
    for t in &level.objects.mousetraps {
        if !t.live() && t.bait_taken {
            continue;
        }
        let d = p - t.pos;
        if d.x.abs() <= half.x && d.y.abs() <= half.y {
            return Some(if t.kind.hair() {
                MinimapCell::TrapHair
            } else {
                MinimapCell::Trap
            });
        }
    }
    if level.objects.crab_pots.iter().any(|pot| {
        crate::game::ecs::systems::traps::in_crab_pot(pot.pos, p)
    }) {
        return Some(MinimapCell::Trap);
    }
    None
}

/// Which spoon key a seen mouse-hole shop gets on the map.
///
/// A hole whose shelf has never been opened reads as a plain `Shop` (the rat
/// has walked past it, not shopped there). Once opened, the marker follows the
/// shelf: nothing left ⇒ `ShopSoldOut`, and a cheapest remaining splice above
/// the current purse ⇒ `ShopTooPricey`. Re-evaluated every frame — the icon is
/// render-only and never changes the shop.
pub fn shop_marker_kind(world: &World, hole: usize) -> MarkerKind {
    let Some(stock) = world.shop.get(hole) else {
        return MarkerKind::Shop;
    };
    let mut cheapest: Option<u32> = None;
    for slot in 0..crate::game::shop::STOCK_SIZE {
        if stock.sold.get(slot).copied().unwrap_or(false) {
            continue;
        }
        if let Some(kind) = stock.slot(slot) {
            let price = kind.price();
            cheapest = Some(cheapest.map_or(price, |c| c.min(price)));
        }
    }
    match cheapest {
        None => MarkerKind::ShopSoldOut,
        Some(price) if price > world.gold => MarkerKind::ShopTooPricey,
        Some(_) => MarkerKind::Shop,
    }
}

// ---- The art module's data source, implemented for the facade -------------

impl MinimapSource for World {
    #[inline]
    fn cols(&self) -> i32 {
        self.level.tiles.cols()
    }

    #[inline]
    fn rows(&self) -> i32 {
        self.level.tiles.rows()
    }

    fn cell_at(&self, cx: i32, cy: i32) -> MinimapCell {
        if !self.level.tiles.in_bounds(cx, cy) {
            return MinimapCell::Unknown;
        }
        minimap_cell(&self.level, cx, cy)
    }

    #[inline]
    fn seen(&self, cx: i32, cy: i32) -> bool {
        self.explored(cx, cy)
    }

    /// Smooth visibility recency for the art's memory fade: `1.0` for ground
    /// revealed in the latest flood, falling off over
    /// [`crate::game::fog::FOG_FADE_TICKS`] into cartographic memory.
    #[inline]
    fn recency(&self, cx: i32, cy: i32) -> f32 {
        self.explored_recency(cx, cy)
    }

    /// A rat-hole's far mouth counts as known once the rat has crossed it (the
    /// fog records crossings) or once both sides of the passage are explored.
    fn rat_hole_far_known(&self, cx: i32, cy: i32, axis: MinimapAxis) -> bool {
        if self.fog().hole_crossed((cx, cy)) {
            return true;
        }
        let (a, b) = match axis {
            MinimapAxis::Horizontal => ((cx, cy - 1), (cx, cy + 1)),
            MinimapAxis::Vertical => ((cx - 1, cy), (cx + 1, cy)),
        };
        self.explored(a.0, a.1) && self.explored(b.0, b.1)
    }
}

/// Room kinds that get a name label on the expanded map.
#[allow(dead_code)]
fn label_for_room(kind: RoomKind) -> Option<LabelKind> {
    match kind {
        // No generated map has one since batch 12; a staged arena room in a
        // capture is just a room.
        RoomKind::Boss => Some(LabelKind::Hall),
        RoomKind::Hall => Some(LabelKind::Hall),
        RoomKind::Maze => Some(LabelKind::Maze),
        RoomKind::Start => Some(LabelKind::Start),
        RoomKind::Room | RoomKind::Corridor => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::level::Theme;
    use crate::math::Vec2;

    fn blank(cols: i32, rows: i32) -> Level {
        let mut level = Level::empty(Theme::Sewer);
        level.tiles = TileMap::new(cols, rows, Theme::Sewer, 0);
        level.arena = level.tiles.world_size();
        level.start = level.arena * 0.5;
        level
    }

    #[test]
    fn every_tile_kind_maps_to_the_arts_cell() {
        let mut level = blank(40, 30);
        level.tiles.set(1, 1, TileKind::Wall(WallKind::Solid));
        level.tiles.set(2, 1, TileKind::Wall(WallKind::Maze));
        level.tiles.set(
            3,
            1,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        level.tiles.set(
            4,
            1,
            TileKind::Wall(WallKind::Doorway {
                axis: WallAxis::Horizontal,
            }),
        );
        level.tiles.set(
            5,
            1,
            TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::West,
            }),
        );
        assert_eq!(minimap_cell(&level, 0, 0), MinimapCell::Floor);
        assert_eq!(minimap_cell(&level, 1, 1), MinimapCell::Wall);
        assert_eq!(minimap_cell(&level, 2, 1), MinimapCell::MazeWall);
        assert_eq!(
            minimap_cell(&level, 3, 1),
            MinimapCell::RatHole(MinimapAxis::Vertical)
        );
        assert_eq!(minimap_cell(&level, 4, 1), MinimapCell::Doorway);
        assert_eq!(
            minimap_cell(&level, 5, 1),
            MinimapCell::OneWayDoor(MinimapDir::West)
        );
    }

    #[test]
    fn outfall_marks_tidal_water_and_working_lures() {
        use crate::game::hazards::{LurePhase, LureTrap};
        use crate::game::level::{Pit, PitKind, TileRect};

        let mut level = blank(12, 12);
        level.theme = Theme::Outfall;
        level.objects.tidal_pits.push(Pit::new(
            TileRect::new(4, 4, 2, 2), PitKind::TidalDrop,
        ));
        level.objects.lure_traps.push(LureTrap::new(TileMap::tile_rect(7, 4).center(), vec![]));
        level.objects.crab_pots.push(crate::game::ecs::systems::traps::set_crab_pot(
            TileMap::tile_rect(8, 7).center(), Vec2::new(0.0, 1.0),
        ));
        assert_eq!(minimap_cell(&level, 4, 4), MinimapCell::TidalDrop);
        assert_eq!(minimap_cell(&level, 7, 4), MinimapCell::Lure);
        level.objects.lure_traps[0].phase = LurePhase::Spent;
        assert_eq!(minimap_cell(&level, 7, 4), MinimapCell::Floor);
        assert_eq!(minimap_cell(&level, 8, 7), MinimapCell::Trap);
    }

    #[test]
    fn traps_have_their_own_cell_kind() {
        let mut level = blank(40, 30);
        let mut trap = crate::game::art_traps::StickyTrapView::new(
            TileMap::tile_rect(6, 6).center(),
            Vec2::new(1.0, 0.0),
        );
        trap.pos = TileMap::tile_rect(6, 6).center();
        level.objects.sticky_traps.push(trap);
        assert_eq!(minimap_cell(&level, 6, 6), MinimapCell::Trap);
        // Bigger 36×28 horizontal board spans its long-axis neighbor cells too,
        // while staying 1 cell thick along its short (28 px) axis.
        assert_eq!(minimap_cell(&level, 5, 6), MinimapCell::Trap);
        assert_eq!(minimap_cell(&level, 7, 6), MinimapCell::Trap);
        assert_eq!(minimap_cell(&level, 6, 7), MinimapCell::Floor);
        assert_eq!(minimap_cell(&level, 6, 7), MinimapCell::Floor);
    }

    #[test]
    fn the_source_reads_the_fog_and_never_calls_cell_at_out_of_bounds() {
        let mut w = World::new(Theme::Sewer, 5);
        assert!(MinimapSource::cols(&w) >= 40);
        assert_eq!(w.cell_at(-1, 0), MinimapCell::Unknown);
        assert_eq!(w.cell_at(0, -1), MinimapCell::Unknown);
        assert!(!w.seen(-1, 0));
        // A fresh run starts unexplored.
        assert!(!w.seen(10, 10));
        for _ in 0..30 {
            w.update(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        }
        assert!(w.fog().seen_count() > 0);
        // The live sight bubble is smaller than the remembered map.
        let (pcx, pcy) = cell_of(w.player.pos);
        assert!(
            MinimapSource::recency(&w, pcx, pcy) > 0.9,
            "the player's cell is freshly revealed"
        );
        assert_eq!(
            MinimapSource::recency(&w, -1, -1),
            0.0,
            "an unseen cell has no recency"
        );
    }

    #[test]
    fn a_rat_hole_far_mouth_is_known_only_after_crossing_or_sight() {
        let mut level = blank(40, 30);
        for cy in 0..30 {
            level.tiles.set(14, cy, TileKind::Wall(WallKind::Solid));
        }
        level.tiles.set(
            14,
            15,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        level.tiles.rebuild_masks();
        let mut w = World::new(Theme::Sewer, 5);
        // Looking at the hole from the near side alone does not open it.
        w.fog.reveal_cell(&level, (10, 15));
        assert!(!w.rat_hole_far_known(14, 15, MinimapAxis::Vertical));
        // Crossing it (the rat's cell becomes the hole) does.
        w.fog.reveal_cell(&level, (14, 15));
        assert!(w.rat_hole_far_known(14, 15, MinimapAxis::Vertical));
        w.update(1.0 / 60.0, Vec2::ZERO, false);
        assert!(w.rat_hole_far_known(14, 15, MinimapAxis::Vertical));
    }

    #[test]
    fn a_fresh_run_has_no_maze_markers_and_no_bearing() {
        let w = World::new(Theme::Sewer, 3);
        let mut frame = MinimapFrame::new();
        frame.refresh(&w);
        assert_eq!(
            frame
                .markers
                .iter()
                .filter(|m| matches!(m.kind, MarkerKind::MazeEntrance))
                .count(),
            0,
            "a maze is marked before it is found"
        );
        assert!(boss_bearing_dir(&w).is_none(), "no bearing at t=0");
    }

    /// Batch 12: there is no arena and no bearing, at any time.
    #[test]
    fn the_map_never_draws_an_arena_or_a_bearing() {
        let mut w = World::new(Theme::Sewer, 11);
        for _ in 0..20 {
            w.update(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        }
        for t in [0.0, ARENA_BEARING_AFTER_SECS - 1.0, ARENA_BEARING_AFTER_SECS + 1.0, 890.0] {
            w.elapsed = t;
            assert!(boss_bearing_dir(&w).is_none(), "a bearing at t={t}");
            let mut frame = MinimapFrame::new();
            frame.refresh(&w);
            assert!(
                !frame.markers.iter().any(|m| m.kind == MarkerKind::BossArena
                    || m.kind == MarkerKind::ArenaBearing),
                "an arena marker at t={t}"
            );
            assert!(
                !frame
                    .labels
                    .iter()
                    .any(|l| l.kind == LabelKind::BossArena
                        || l.text.to_uppercase().contains("ARENA")),
                "an arena label at t={t}"
            );
        }
    }

    #[test]
    fn a_seen_maze_contributes_its_breach_and_cache_marker() {
        let mut seed = None;
        for candidate in 0..64u64 {
            if !Level::generate(Theme::Sewer, candidate).mazes.is_empty() {
                seed = Some(candidate);
                break;
            }
        }
        let Some(seed) = seed else {
            panic!("no Sewer map in the first 64 seeds has a maze");
        };
        let mut w = World::new(Theme::Sewer, seed);
        let maze = *w.level.mazes.first().unwrap();
        let level = Level::generate(Theme::Sewer, seed);
        let mut frame = MinimapFrame::new();
        frame.refresh(&w);
        assert!(!frame
            .markers
            .iter()
            .any(|m| m.kind == MarkerKind::MazeEntrance));

        // Seeing the breach marks the maze: a fork and a hollow cache.
        w.fog.reveal_cell(&level, maze.entry);
        frame.refresh(&w);
        assert!(frame
            .markers
            .iter()
            .any(|m| m.kind == MarkerKind::MazeEntrance));
        assert!(frame
            .markers
            .iter()
            .any(|m| m.kind == MarkerKind::MazeCacheHollow));
        assert!(frame
            .labels
            .iter()
            .any(|l| l.kind == LabelKind::Maze && l.discovered));

        // Claiming the cache fills the marker in (per-maze state the reward
        // runtime will drive).
        w.fog.set_maze_claimed(0, true);
        frame.refresh(&w);
        assert!(frame
            .markers
            .iter()
            .any(|m| m.kind == MarkerKind::MazeCacheClaimed));
        assert!(!frame
            .markers
            .iter()
            .any(|m| m.kind == MarkerKind::MazeCacheHollow));
    }

    /// The first seed in `theme`'s first 64 that has a maze, or `None`.
    fn maze_seed(theme: Theme) -> Option<u64> {
        (0..64u64).find(|&seed| !Level::generate(theme, seed).mazes.is_empty())
    }

    /// Render the expanded map for `w` into a `120 × 38` terminal and return its
    /// text, so a test can read what the art's side panel actually says.
    fn expanded_panel_text(w: &World, frame: &MinimapFrame) -> String {
        let view = frame.view(w, true, false);
        let mut scratch = minimap::MinimapScratch::new();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 38)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                minimap::draw(f, area, &view, w, &mut scratch);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Seven maps in eight have no maze now, and the expanded map must not
    /// claim otherwise: with no discovered maze the art omits its
    /// `CACHES: n / m` line and all maze-specific legend rows.
    #[test]
    fn a_map_without_a_maze_draws_no_cache_telemetry() {
        let seed = (0..64u64)
            .find(|&seed| Level::generate(Theme::Sewer, seed).mazes.is_empty())
            .expect("no maze-less Sewer map in the first 64 seeds");
        let mut w = World::new(Theme::Sewer, seed);
        for _ in 0..20 {
            w.update(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        }
        let mut frame = MinimapFrame::new();
        frame.refresh(&w);
        assert!(
            !frame.markers().iter().any(|m| matches!(
                m.kind,
                MarkerKind::MazeEntrance
                    | MarkerKind::MazeCacheHollow
                    | MarkerKind::MazeCacheClaimed
                    | MarkerKind::MazeReward
            )),
            "a maze-less map contributed a maze marker"
        );
        assert!(!frame.view(&w, true, false).maze_discovered);
        let text = expanded_panel_text(&w, &frame);
        assert!(
            !text.contains("CACHE"),
            "the side panel invented cache telemetry: {text}"
        );
        for maze_row in [
            "Maze Partition",
            "One-Way Exit",
            "Maze Breach",
            "Unclaimed Cache",
            "Claimed Cache",
        ] {
            assert!(
                !text.contains(maze_row),
                "a maze-less map showed maze legend row '{maze_row}'"
            );
        }
    }

    /// Even on the 1-in-8 map that carries a maze, the expanded legend must
    /// not spoil its existence until the player discovers the breach.
    #[test]
    fn an_undiscovered_maze_does_not_spoil_the_legend_until_seen() {
        let seed = maze_seed(Theme::Sewer).expect("no Sewer map in the first 64 has a maze");
        let mut w = World::new(Theme::Sewer, seed);
        let maze = *w.level.mazes.first().unwrap();
        let mut frame = MinimapFrame::new();
        frame.refresh(&w);

        assert!(
            !frame.view(&w, true, false).maze_discovered,
            "undiscovered maze spoiled maze_discovered before entry was seen"
        );
        let before = expanded_panel_text(&w, &frame);
        assert!(!before.contains("Maze Partition"));
        assert!(!before.contains("Maze Breach"));

        // Seeing the maze entrance reveals the maze legend rows.
        w.fog.reveal_cell(&w.level, maze.entry);
        frame.refresh(&w);
        assert!(frame.view(&w, true, false).maze_discovered);
        let after = expanded_panel_text(&w, &frame);
        assert!(after.contains("Maze Partition"));
        assert!(after.contains("Maze Breach"));
        assert!(after.contains("Unclaimed Cache"));
    }

    /// On the one map in eight that has a maze, the panel's total is the number
    /// of *known* caches — one — and claiming it fills the line in.
    #[test]
    fn a_found_cache_is_counted_one_of_one_and_fills_on_claim() {
        let seed = maze_seed(Theme::Sewer).expect("no Sewer map in the first 64 has a maze");
        let mut w = World::new(Theme::Sewer, seed);
        let maze = *w.level.mazes.first().unwrap();
        w.fog.reveal_cell(&w.level, maze.entry);
        let mut frame = MinimapFrame::new();
        frame.refresh(&w);

        let text = expanded_panel_text(&w, &frame);
        assert!(
            text.contains("CACHES: 0 / 1 CLAIMED"),
            "the cache total is not the mazes on the map: {text}"
        );

        w.fog.set_maze_claimed(0, true);
        frame.refresh(&w);
        let text = expanded_panel_text(&w, &frame);
        assert!(
            text.contains("CACHES: 1 / 1 CLAIMED"),
            "a claimed cache did not fill the counter: {text}"
        );
    }

    #[test]
    fn frame_buffers_are_reused_across_frames() {
        let w = World::new(Theme::Lab, 7);
        let mut frame = MinimapFrame::new();
        frame.refresh(&w);
        let cap = frame.markers.capacity();
        for _ in 0..10 {
            frame.refresh(&w);
        }
        assert_eq!(frame.markers.capacity(), cap, "markers reallocated per frame");
        assert!(frame.marker_count() > 0);
    }

    #[test]
    fn the_art_layout_is_used_and_stays_inside_the_field() {
        for (w, h) in [(80u16, 22u16), (120, 38), (200, 58)] {
            let game = Rect::new(0, 1, w, h);
            let compact = compact_rect(game);
            assert!(!compact.is_empty(), "no compact minimap on {w}x{h}");
            assert!(game.union(compact) == game, "the radar escaped the field");
            let expanded = expanded_rect(game);
            assert!(!expanded.is_empty(), "no expanded map on {w}x{h}");
            assert!(game.union(expanded) == game, "the modal escaped the field");
            assert!(expanded.width > compact.width);
        }
        for (w, h) in [(0u16, 0u16), (0, 5), (5, 0), (1, 1), (20, 6), (40, 12)] {
            let game = Rect::new(0, 0, w, h);
            assert!(compact_rect(game).is_empty(), "minimap on a tiny {w}x{h} field");
            assert!(expanded_rect(game).is_empty());
        }
    }

    /// The shop's spoon marker appears only once the hole has been seen, and is
    /// remembered afterwards (the fog's `seen` bit is sticky).
    #[test]
    fn the_shop_spoon_is_revealed_on_sight_and_then_remembered() {
        use crate::game::world::World;
        let mut world = World::new(Theme::Sewer, 7);
        // A bed can carry fewer than two shops (`levelgen::trailer` drops the
        // faces a pit or a crate blocks); every other floor carries two. The
        // guard keeps the test honest on a flatbed seed with none.
        let Some(&hole) = world.level.objects.shop_holes.first() else {
            return;
        };
        let (hx, hy) = cell_of(hole.pos);
        let mut frame = MinimapFrame::new();

        // Unexplored: no spoon anywhere on the map.
        frame.refresh(&world);
        assert!(
            !frame.markers().iter().any(|m| m.kind == MarkerKind::Shop),
            "an unexplored hole is not on the map"
        );

        // Seen once (the rat walks up to it): the spoon is there.
        world.fog.reveal_cell(&world.level, (hx, hy));
        frame.refresh(&world);
        let spoon = frame
            .markers()
            .iter()
            .find(|m| m.kind == MarkerKind::Shop)
            .expect("a seen hole keeps its spoon");
        assert_eq!((spoon.cx, spoon.cy), (hx as f32, hy as f32));

        // Walk away: the fog's live view moves on, the marker is remembered.
        world.player.pos = Vec2::new(32.0, 32.0);
        world.fog.update_from(&world.level, world.player.pos);
        frame.refresh(&world);
        assert!(
            frame.markers().iter().any(|m| m.kind == MarkerKind::Shop),
            "a visited shop stays on the map"
        );
    }

}
