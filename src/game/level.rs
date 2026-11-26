//! Level data model: theme, palette, the tile grid, rooms and level objects.
//!
//! A level is a **200 × 160 cell** lattice of 16 px tiles
//! ([`collision::TileMap`]) — ten times the old hand-built arena's area — plus
//! the generator's named regions ([`Room`]), the maze sections
//! ([`MazeSection`]), the interactive objects ([`LevelObjects`]: traps,
//! one-way doors, rat-holes, maze rewards) and the legacy `props` (themed
//! furniture, drawn by `art_props.rs`).
//!
//! Everything here is generated from `(theme, seed)` by
//! [`Level::generate`] — see `CONTENT_PLAN.md` for the generator's stages and
//! [`super::levelgen`] for the code. Generation is deterministic: the same
//! seed and theme always produce the same map, because the generator runs off
//! one `StdRng` seeded from them and never iterates a hash map.
//!
//! ## Collision
//!
//! [`Level::tiles`] is the **only** source of wall geometry: the player,
//! enemies, bullets, lobs and spawn checks all query the tile grid and
//! `art_walls::wall_colliders`, so what is drawn and what blocks can never
//! disagree. There is no rectangle list of walls to iterate, which is what
//! keeps a 200 × 160 map as cheap per tick as the old 160 × 125 arena build.

use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::detmath::DetF32;
use crate::math::Vec2;
use crate::render::palette::Rgb;

use super::art_traps::{
    MazeRewardView, MousetrapView, OneWayDoorState, OneWayDoorView, RatHoleView, StickyTrapView,
};
use super::art_walls::{Cardinal, WallCellView};
use super::collision::{cell_of, SizeClass, TileMap, TILE, TILE_F};
use super::levelgen;
use super::nav::NavGrid;

/// Level width in 16 px cells.
pub const MAP_COLS: i32 = 200;
/// Level height in 16 px cells.
pub const MAP_ROWS: i32 = 160;
/// Level size in world pixels (`MAP_COLS × TILE` by `MAP_ROWS × TILE`).
pub const MAP_SIZE: Vec2 = Vec2 { x: 3200.0, y: 2560.0 };

/// The rectangle a boss fight happens in, in cells: the size the removed
/// arena's room had (34 × 28), clamped to the map and centred on the rat when
/// the boss arrives ([`Level::fight_rect_around`]).
pub const BOSS_FIGHT_TILES: (i32, i32) = (34, 28);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Sewer,
    Kitchen,
    Lab,
    /// Wave 3 side route, stage two: the laundry floor with its washer aisles,
    /// folding bays and steam grates (`WAVE3_DESIGN.md` §11).
    Laundry,
    /// Wave 3 side route, stage three: the printworks with its offset press
    /// islands, paper lanes and ink pads (`WAVE3_DESIGN.md` §11).
    Printworks,
    /// The getaway ride: the bed of a long flatbed trailer running down a
    /// road. Its own generator (`levelgen::trailer`) builds the bed, the cargo
    /// that shapes it and the hazards; `art_trailer` owns the planks, the road
    /// seen through their gaps, the rails and the cargo art.
    Trailer,
    /// The tidal basins beyond the Sewer's storm gate.
    Outfall,
}

impl Theme {
    /// Generated floors in facility order: the three stages of the
    /// original route first (Sewer, Kitchen, Lab), then the side routes.
    /// The first three entries are therefore
    /// still "the original route by level index".
    ///
    /// [`Theme::Trailer`] is deliberately **not** here: it is the getaway
    /// ride, a bonus practice floor and not a campaign step (see
    /// [`crate::game::campaign::getaway_theme`]). Lists that build or draw
    /// every theme the engine can render use [`Theme::playable`] instead.
    pub fn all() -> [Theme; 6] {
        [
            Theme::Sewer,
            Theme::Kitchen,
            Theme::Lab,
            Theme::Laundry,
            Theme::Printworks,
            Theme::Outfall,
        ]
    }

    /// The campaign stage (0, 1 or 2) a theme belongs to: the Sewer is stage
    /// one, the Kitchen, Laundry and Outfall are stage two, the Lab and the
    /// Printworks are both stage three. The Flatbed is a last-stage getaway,
    /// so it reads as stage three.
    pub fn stage(self) -> usize {
        match self {
            Theme::Sewer => 0,
            Theme::Kitchen | Theme::Laundry | Theme::Outfall => 1,
            Theme::Lab | Theme::Printworks | Theme::Trailer => 2,
        }
    }

    /// Every theme a `World` can be built on, in **route order**: stage one,
    /// the three stage-two alternatives, the two stage-three alternatives, then
    /// the getaway ride last. Tests, the title's sector row and the sector
    /// select screen all list this order, so the row's cursor, the select
    /// screen's grid and a run's `level_index` never disagree.
    pub fn playable() -> [Theme; 7] {
        [
            Theme::Sewer,
            Theme::Kitchen,
            Theme::Laundry,
            Theme::Outfall,
            Theme::Lab,
            Theme::Printworks,
            Theme::Trailer,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Sewer => "Sewer",
            Theme::Kitchen => "Kitchen",
            Theme::Lab => "Lab",
            Theme::Laundry => "Laundry",
            Theme::Printworks => "Printworks",
            Theme::Trailer => "Flatbed",
            Theme::Outfall => "Outfall",
        }
    }

    /// The campaign theme a level index plays, when it is not the getaway
    /// ride (see [`crate::game::campaign::sector_theme`]).
    pub fn is_trailer(self) -> bool {
        matches!(self, Theme::Trailer)
    }
}

/// Axis-aligned rectangle in world-pixel space.
#[derive(Clone, Copy, Debug)]
pub struct Rectf {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rectf {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
}

/// A rectangle of grid cells (`x, y` are cell coordinates; `w, h` are cell
/// counts). Rooms, maze sections and generator reservations all use it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl TileRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Exclusive right edge in cells.
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    /// Exclusive bottom edge in cells.
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, cx: i32, cy: i32) -> bool {
        cx >= self.x && cx < self.right() && cy >= self.y && cy < self.bottom()
    }

    pub fn contains_world(&self, p: Vec2) -> bool {
        self.contains((p.x / TILE_F).floor() as i32, (p.y / TILE_F).floor() as i32)
    }

    pub fn intersects(&self, o: &TileRect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// Grow by `by` cells on every side (negative shrinks).
    pub fn inflate(&self, by: i32) -> TileRect {
        TileRect::new(self.x - by, self.y - by, self.w + 2 * by, self.h + 2 * by)
    }

    pub fn to_world(self) -> Rectf {
        Rectf::new(
            (self.x * TILE) as f32,
            (self.y * TILE) as f32,
            (self.w * TILE) as f32,
            (self.h * TILE) as f32,
        )
    }

    pub fn center_world(&self) -> Vec2 {
        Vec2::new(
            (self.x as f32 + self.w as f32 * 0.5) * TILE_F,
            (self.y as f32 + self.h as f32 * 0.5) * TILE_F,
        )
    }

    /// Every cell of the rectangle, `(row, column)` order.
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        (self.y..self.bottom()).flat_map(move |cy| (self.x..self.right()).map(move |cx| (cx, cy)))
    }

    pub fn area(&self) -> i32 {
        self.w.max(0) * self.h.max(0)
    }
}

pub struct Palette {
    pub bg: Rgb,
    pub bg_alt: Rgb,
    pub wall: Rgb,
    pub wall_edge: Rgb,
    pub accent: Rgb,
    pub blood: Rgb,
}

impl Palette {
    pub fn for_theme(theme: Theme) -> Palette {
        match theme {
            Theme::Sewer => Palette {
                bg: (14, 22, 20),
                bg_alt: (18, 28, 26),
                wall: (40, 70, 64),
                wall_edge: (70, 120, 110),
                accent: (90, 220, 200),
                blood: (120, 200, 90),
            },
            Theme::Kitchen => Palette {
                bg: (26, 20, 16),
                bg_alt: (34, 26, 20),
                wall: (110, 80, 50),
                wall_edge: (170, 130, 80),
                accent: (255, 180, 90),
                blood: (200, 60, 50),
            },
            Theme::Lab => Palette {
                bg: (12, 14, 24),
                bg_alt: (16, 20, 34),
                wall: (40, 50, 90),
                wall_edge: (90, 120, 220),
                accent: (120, 220, 255),
                blood: (90, 200, 255),
            },
            // Indigo tile #22263B over faded denim #426779, porcelain #B3C7C3
            // highlights and the pink of a ruined white wash.
            Theme::Laundry => Palette {
                bg: (0x22, 0x26, 0x3B),
                bg_alt: (0x2A, 0x2F, 0x48),
                wall: (0x42, 0x67, 0x79),
                wall_edge: (0xB3, 0xC7, 0xC3),
                accent: (0x9F, 0xD8, 0xD4),
                blood: (0xD8, 0x7E, 0xA6),
            },
            // Newsprint grey #B6B4A5 against charcoal #22232A, dried ink
            // #554064 and registration cyan #4E9498.
            Theme::Printworks => Palette {
                bg: (0x22, 0x23, 0x2A),
                bg_alt: (0x2C, 0x2D, 0x36),
                wall: (0x96, 0x94, 0x88),
                wall_edge: (0xB6, 0xB4, 0xA5),
                accent: (0x4E, 0x94, 0x98),
                blood: (0x55, 0x40, 0x64),
            },
            // Weathered oak over hot asphalt at dusk: warm browns for the
            // planks and the tarps, a stale amber for the lane markings, and a
            // rust red for what the road does to the horde.
            Theme::Trailer => Palette {
                bg: (38, 29, 21),
                bg_alt: (52, 39, 27),
                wall: (92, 68, 42),
                wall_edge: (168, 132, 86),
                accent: (226, 178, 84),
                blood: (150, 66, 48),
            },
            Theme::Outfall => Palette {
                bg: (84, 77, 60),
                bg_alt: (96, 88, 69),
                wall: (34, 58, 63),
                wall_edge: (138, 170, 173),
                accent: (105, 230, 223),
                blood: (96, 158, 155),
            },
        }
    }
}

/// A solid obstacle drawn as a themed object. Its `rect` is the collision +
/// draw footprint (cell-aligned: the tiles it covers are `WallKind::Solid`);
/// the kind selects how it's rendered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    PipeV,
    PipeH,
    Valve,
    Grate,
    Table,
    Crate,
    Barrel,
    Counter,
    Console,
    Tank,
    Barrier,
    /// A round-porthole washer (Laundry): a long white box you hop over.
    Washer,
    /// A rolled towel on end (Laundry).
    TowelRoll,
    /// A giant wooden clothespin leaning on the wall (Laundry).
    Clothespin,
    /// An offset press island (Printworks): a squat inked machine.
    PressIsland,
    /// A standing paper reel (Printworks).
    PaperReel,
    /// A reversed type block (Printworks): a lead letter, mirrored.
    TypeBlock,
    // ---- Flatbed cargo (the trailer's load) ------------------------------
    /// A stack of slats on bearers (32 × 24 px): a low, walk-round obstacle.
    Pallet,
    /// A tarped machine shape (24 × 24 px).
    TarpLump,
    /// A stacked spare tyre (18 × 18 px).
    Tyre,
    /// A steel toolbox (20 × 12 px).
    Toolbox,
    /// A strap-bound stack of pipes (40 × 18 px).
    PipeStack,
    /// A coiled chain on a hook (16 × 16 px).
    ChainCoil,
    OutfallPiling,
    OutfallBallast,
    OutfallRib,
}

impl PropKind {
    pub fn size(self) -> (f32, f32) {
        match self {
            PropKind::PipeV => (14.0, 72.0),
            PropKind::PipeH => (72.0, 14.0),
            PropKind::Valve => (20.0, 20.0),
            PropKind::Grate => (30.0, 30.0),
            PropKind::Table => (40.0, 26.0),
            PropKind::Crate => (22.0, 22.0),
            PropKind::Barrel => (20.0, 20.0),
            PropKind::Counter => (66.0, 22.0),
            PropKind::Console => (46.0, 18.0),
            PropKind::Tank => (24.0, 24.0),
            PropKind::Barrier => (16.0, 66.0),
            PropKind::Washer => (58.0, 30.0),
            PropKind::TowelRoll => (26.0, 26.0),
            PropKind::Clothespin => (16.0, 34.0),
            PropKind::PressIsland => (74.0, 46.0),
            PropKind::PaperReel => (26.0, 34.0),
            PropKind::TypeBlock => (30.0, 30.0),
            PropKind::Pallet => (32.0, 24.0),
            PropKind::TarpLump => (24.0, 24.0),
            PropKind::Tyre => (18.0, 18.0),
            PropKind::Toolbox => (20.0, 12.0),
            PropKind::PipeStack => (40.0, 18.0),
            PropKind::ChainCoil => (16.0, 16.0),
            PropKind::OutfallPiling => (16.0, 16.0),
            PropKind::OutfallBallast => (32.0, 16.0),
            PropKind::OutfallRib => (16.0, 32.0),
        }
    }

    /// Footprint in tiles (`ceil` of [`Self::size`] over the 16 px cell).
    pub fn cells(self) -> (i32, i32) {
        let (w, h) = self.size();
        (
            (w / TILE_F).ceil().max(1.0) as i32,
            (h / TILE_F).ceil().max(1.0) as i32,
        )
    }
}

#[derive(Clone, Copy)]
pub struct Prop {
    pub kind: PropKind,
    pub rect: Rectf,
}

/// The prop kinds a theme furnishes rooms with (cycled by the generator).
pub fn theme_props(theme: Theme) -> &'static [PropKind] {
    match theme {
        Theme::Sewer => &[
            PropKind::PipeV,
            PropKind::PipeH,
            PropKind::Valve,
            PropKind::Grate,
            PropKind::Barrier,
            PropKind::PipeH,
            PropKind::PipeV,
            PropKind::Valve,
        ],
        Theme::Kitchen => &[
            PropKind::Table,
            PropKind::Crate,
            PropKind::Barrel,
            PropKind::Counter,
            PropKind::Crate,
            PropKind::Table,
            PropKind::Barrel,
            PropKind::Counter,
        ],
        Theme::Lab => &[
            PropKind::Console,
            PropKind::Tank,
            PropKind::Crate,
            PropKind::Barrier,
            PropKind::Console,
            PropKind::Tank,
            PropKind::Barrier,
            PropKind::Crate,
        ],
        // Washer banks and laundry trolleys; the clothespin is the tall one.
        Theme::Laundry => &[
            PropKind::Washer,
            PropKind::Washer,
            PropKind::TowelRoll,
            PropKind::Clothespin,
            PropKind::Washer,
            PropKind::TowelRoll,
        ],
        // Press beds, paper reels and loose type: the print floor's furniture.
        Theme::Printworks => &[
            PropKind::PressIsland,
            PropKind::PaperReel,
            PropKind::TypeBlock,
            PropKind::PressIsland,
            PropKind::PaperReel,
            PropKind::TypeBlock,
        ],
        Theme::Trailer => &[
            PropKind::Crate,
            PropKind::Crate,
            PropKind::Pallet,
            PropKind::TarpLump,
            PropKind::Tyre,
            PropKind::Toolbox,
            PropKind::PipeStack,
            PropKind::ChainCoil,
        ],
        Theme::Outfall => &[
            PropKind::OutfallPiling,
            PropKind::OutfallPiling,
            PropKind::OutfallBallast,
            PropKind::OutfallRib,
        ],
    }
}

// ---- Named regions ---------------------------------------------------------

/// What a [`Room`] is for. Spawning, the director and the boss fight all pick
/// rooms by kind instead of hard-coding coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    /// Where the run starts: the rat spawns at its centre. Always centred on
    /// the map, kept clear of walls and traps.
    Start,
    /// The big open room the boss fight happens in (one per map). Wide, no
    /// pillars near the centre, entered through a 1- or 2-cell doorway.
    Boss,
    /// A large open hall: the horde can surround the player here.
    Hall,
    /// A regular room (possibly with props).
    Room,
    /// A corridor segment (narrow: 1–2 cells wide).
    Corridor,
    /// The interior of a maze section.
    Maze,
}

/// A named region of the map. Rooms are axis-aligned cell rectangles; the
/// generator may carve them into irregular shapes later, but everything that
/// consumes them (spawn placement, HUD, minimap) only needs the rect.
#[derive(Clone, Copy, Debug)]
pub struct Room {
    pub rect: TileRect,
    pub kind: RoomKind,
}

impl Room {
    pub fn new(rect: TileRect, kind: RoomKind) -> Room {
        Room { rect, kind }
    }

    pub fn center(&self) -> Vec2 {
        self.rect.center_world()
    }
}

/// One maze section: a puzzle pocket with a reward at its end and a one-way
/// door as its exit.
///
/// Everything later systems need is exported here, so no consumer has to
/// re-derive the lattice: the breach ([`Self::entry`]), the public fork the
/// gnaw-mark trail starts from ([`Self::fork`]), the partition ring
/// ([`Self::ring`], for spawn exclusion and fog), and the cache
/// ([`Self::reward`], [`Self::reward_cell`]).
#[derive(Clone, Copy, Debug)]
pub struct MazeSection {
    /// Bounding cells of the section (its entry is on this boundary).
    pub bounds: TileRect,
    /// The section's [`WallKind::Maze`](super::art_walls::WallKind::Maze)
    /// partition ring: the one-cell wall the breach and the one-way door sit
    /// in. Spawn exclusion and fog use this, not `bounds`.
    pub ring: TileRect,
    /// Cell the player enters the maze through (the breach, on `ring`).
    pub entry: (i32, i32),
    /// Public walkway cell the gnaw-mark trail starts from: three marks along
    /// the final four cells between here and the breach.
    pub fork: (i32, i32),
    /// The [`super::art_walls::WallKind::OneWayDoor`] cell that exits the maze.
    pub exit: (i32, i32),
    /// The only allowed travel direction through `exit` (out of the maze).
    pub exit_dir: Cardinal,
    /// World position of the [`MazeRewardView`] pedestal at the maze's end.
    pub reward: Vec2,
    /// The pedestal's cell (precomputed: `cell_of(reward)`, without the
    /// per-frame divide).
    pub reward_cell: (i32, i32),
}

// ---- Interactive level objects --------------------------------------------
//
// These are the art modules' own `Copy` view structs: the doc comment on each
// state (`StickyTrapState::Stuck`, `MousetrapState::Snapping`, ...) *is* the
// gameplay spec, so the simulation mutates the very struct the renderer draws.
// There is deliberately no second "sim state" copy that could drift.

/// A placed one-way door, linked to the cell whose `WallKind` it animates.
#[derive(Clone, Copy, Debug)]
pub struct OneWayDoor {
    pub cell: (i32, i32),
    pub view: OneWayDoorView,
}

impl OneWayDoor {
    pub fn new(cell: (i32, i32), facing: Vec2) -> OneWayDoor {
        OneWayDoor {
            cell,
            view: OneWayDoorView::new(TileMap::tile_rect(cell.0, cell.1).center(), facing),
        }
    }

    /// `0.0` fully shut … `1.0` fully swung open, for the wall art's
    /// `open_progress` ([`super::art_walls::WallCellView::open_progress`]).
    pub fn open_progress(&self) -> f32 {
        match self.view.state {
            OneWayDoorState::Closed | OneWayDoorState::Refused => 0.0,
            OneWayDoorState::Opening => self.view.progress.clamp(0.0, 1.0),
            OneWayDoorState::Closing => 1.0 - self.view.progress.clamp(0.0, 1.0),
        }
    }
}

/// A placed rat-hole, linked to the wall cell that carries it.
#[derive(Clone, Copy, Debug)]
pub struct RatHole {
    pub cell: (i32, i32),
    pub view: RatHoleView,
}

/// What a rat-hole is **for** — the designer's three roles (`DESIGN_REVIEW.md`
/// §4). The generator sets it; gameplay reads it through
/// [`LevelObjects::rat_hole_record`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RatHoleRole {
    /// Near the first excursion out of the start room.
    Excursion,
    /// Outside a mid-run gathering spot (a hall).
    Gathering,
    /// Any other hole, grown by the random scatter pass.
    #[default]
    Incidental,
}

/// The generator's record for one rat-hole: what it is for and which side of
/// the wall the routes arrive from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RatHoleRecord {
    /// Why this hole exists.
    pub role: RatHoleRole,
    /// Direction from the hole toward the **public** side: the mouth nearer the
    /// start room, i.e. the side the rat is expected to arrive from. Holes are
    /// passable both ways; this is the orientation the art faces
    /// ([`RatHoleView::facing`]) and the side a "useful hole" cue should point
    /// back to.
    pub approach: Cardinal,
}

/// What a **mouse-hole shop** is for (the two placement roles the design
/// asks for).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShopHoleRole {
    /// The first hole: 16–40 walking cells from the spawn, on the face of a
    /// wall that looks onto a fork on the way to an ordinary room.
    #[default]
    Fork,
    /// The second hole: on a different approach toward the map's far hall (the
    /// footprint the boss arena used to take).
    Approach,
}

/// One mouse-hole shop in a structural wall face (`WAVE3_DESIGN.md` §6).
///
/// A shop hole is **carved into a wall cell**: the cell stays solid, so the rat
/// and the horde never walk into it (neither can get stuck on it) and the map's
/// passability is unchanged. The rat touches the face from the public floor and
/// the shop opens over the paused world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShopHole {
    /// Index in [`LevelObjects::shop_holes`] (the shelf's seed key).
    pub index: usize,
    /// The wall cell the hole is cut into.
    pub cell: (i32, i32),
    /// Centre of the hole in world pixels (the wall cell's centre).
    pub pos: Vec2,
    /// Unit direction from the wall cell into the public floor: the face the
    /// rat walks up to and the direction the art's spoon points.
    pub facing: Vec2,
    /// The same direction as a cardinal (the wall art's convention).
    pub approach: Cardinal,
    /// The floor cell in front of the face (the threshold's cell).
    pub mouth: (i32, i32),
    /// Why the generator placed it where it did.
    pub role: ShopHoleRole,
}

impl ShopHole {
    /// The threshold point the rat touches: the wall face, in world pixels.
    pub fn face(&self) -> Vec2 {
        self.pos + self.facing * (crate::game::collision::TILE_F * 0.5)
    }
}

/// The generator's record for one trap: what it was placed for and which way
/// the routes arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrapRecord {
    /// True when the trap is a room-mouth **encounter**: one or two cells
    /// inside a hall or room where a corridor fans out, with two parallel ways
    /// past it. Those are worth an audible/visual cue; the rest are incidental.
    pub encounter: bool,
    /// Direction from the trap toward the side the routes arrive from (the
    /// public side). For a mousetrap this is the baited kill end's side, so it
    /// greets the arriving lane; a sticky board only uses it as its long axis.
    pub approach: Cardinal,
}

/// Lighting-only room treatment. A zone never affects visibility or traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightZoneKind {
    Corridor,
    Room,
    Start,
    Arena,
    Blackout,
    Maze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightZone {
    pub rect: TileRect,
    pub kind: LightZoneKind,
}

/// Walkable, cosmetic fixtures. The renderer supplies theme art and response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightFixtureKind {
    Pendant,
    Passage,
    Arena,
    Broken,
    Sun,
    /// A lamp mounted on a structural wall face (batch 9). Unlike the hanging
    /// fixtures its source sits on the face itself and throws a fan of light
    /// into the room along [`LightFixture::wall_facing`], so a corridor reads
    /// as a rhythm of pools with dark stretches between them. The wall it
    /// hangs on occludes the half of the fan behind it. Placed by
    /// `levelgen::lights`, drawn and lit by `render::lighting::fixtures`.
    Sconce,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightFixture {
    pub pos: Vec2,
    pub kind: LightFixtureKind,
    pub radius: f32,
    pub seed: u32,
}

impl LightFixture {
    /// A sputtering or spent wall lamp is never a stable gameplay light.
    pub fn is_live_sconce(&self) -> bool {
        self.kind == LightFixtureKind::Sconce && !Self::sconce_spent(self.seed)
    }

    pub fn sconce_spent(seed: u32) -> bool {
        seed.is_multiple_of(8)
    }

    /// The direction a wall-mounted lamp faces: the unit vector pointing away
    /// from the wall face it hangs on, into the floor it lights.
    ///
    /// A [`LightFixtureKind::Sconce`] is placed **on its face**, half a pixel
    /// inside the floor cell in front of it, so the nearest wall cell is always
    /// the one it is mounted on (8.5 px from `pos`) and every other wall cell is
    /// at least 17.9 px out — the rule is unambiguous and needs no stored
    /// facing. The generator mounts by exactly this rule and the renderer reads
    /// it back with it, so the two can never disagree; a fixture with no wall
    /// neighbour (nothing places one) falls back to south, into the room.
    ///
    /// Allocation-free and total, so the renderer may call it per fixture per
    /// frame.
    pub fn wall_facing(&self, tiles: &TileMap) -> Vec2 {
        let (cx, cy) = cell_of(self.pos);
        let mut best: Option<(f32, Vec2)> = None;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if !tiles.get(cx + dx, cy + dy).is_wall() {
                continue;
            }
            let centre = TileMap::tile_rect(cx + dx, cy + dy).center();
            let d = self.pos.dist_sq(centre);
            if best.is_none_or(|(nearest, _)| d < nearest) {
                best = Some((d, Vec2::new(-(dx as f32), -(dy as f32))));
            }
        }
        best.map_or(Vec2::new(0.0, 1.0), |(_, dir)| dir)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmitterKind {
    Fire,
    Steam,
    Drip,
    Cable,
    Dust,
    Fog,
    Exhaust,
    Pilot,
    Fumes,
    Insects,
}

/// Ambient effects are evaluated from seed and elapsed time by the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientEmitter {
    pub kind: EmitterKind,
    pub pos: Vec2,
    pub rate: f32,
    pub intensity: f32,
    pub seed: u32,
}

/// Every non-wall object placed on the map.
#[derive(Clone, Debug, Default)]
pub struct LevelObjects {
    pub sticky_traps: Vec<StickyTrapView>,
    pub mousetraps: Vec<MousetrapView>,
    pub one_way_doors: Vec<OneWayDoor>,
    pub rat_holes: Vec<RatHole>,
    pub rewards: Vec<MazeRewardView>,
    /// Generator record for each entry of [`Self::sticky_traps`], **same
    /// index**. Filled once by the generator; nothing in the simulation writes
    /// it.
    pub sticky_trap_records: Vec<TrapRecord>,
    /// Generator record for each entry of [`Self::mousetraps`], same index.
    pub mousetrap_records: Vec<TrapRecord>,
    /// Generator record for each entry of [`Self::rat_holes`], same index.
    pub rat_hole_records: Vec<RatHoleRecord>,
    /// The two mouse-hole shops, in placement order (index 0 is the fork
    /// hole, index 1 the boss-approach one). The shelf's seed keys on the
    /// index, so a hole's stock is stable across a visit.
    pub shop_holes: Vec<ShopHole>,
    /// Steam grates (Laundry): walkable floor vents with a warn/plume cycle.
    /// See [`super::hazards`].
    pub steam_grates: Vec<super::hazards::SteamGrate>,
    /// Ink pads (Printworks): permanent walkable stains that slow ground
    /// movement. See [`super::hazards`].
    pub ink_pads: Vec<super::hazards::InkPad>,
    pub lights: Vec<LightFixture>,
    pub light_zones: Vec<LightZone>,
    pub emitters: Vec<AmbientEmitter>,
    /// Level one's scripted Sewer encounters, authored by levelgen.
    pub sewer_opening: Option<super::hazards::SewerOpening>,
    /// Wet bands left by completed Corkscrew strikes.
    pub trench_puddles: Vec<TrenchPuddle>,
    pub tidal_pits: Vec<Pit>,
    pub lure_traps: Vec<super::hazards::LureTrap>,
    /// Fixture indices. The Grouper and Lantern each own their own slot.
    pub outfall_elite_snuff: Option<usize>,
    pub outfall_boss_snuff: Option<usize>,
    pub crab_pots: Vec<super::art_crabpot::CrabPotView>,
    /// Generator record for each crab pot, in the same order.
    pub crab_pot_records: Vec<TrapRecord>,
}

impl LevelObjects {
    /// The one-way door animating `(cx, cy)`, if any.
    pub fn door_at(&self, cx: i32, cy: i32) -> Option<&OneWayDoor> {
        self.one_way_doors
            .iter()
            .find(|d| d.cell == (cx, cy))
    }

    /// The generator record of the sticky trap at `pos`, if there is one.
    ///
    /// Matched by position, so a caller that passes a live
    /// [`StickyTrapView::pos`] gets the right record even if an earlier entry
    /// was removed from the list.
    pub fn sticky_trap_record(&self, pos: Vec2) -> Option<TrapRecord> {
        self.sticky_traps
            .iter()
            .zip(&self.sticky_trap_records)
            .find(|(trap, _)| trap.pos == pos)
            .map(|(_, record)| *record)
    }

    /// The generator record of the mousetrap at `pos`, if there is one.
    pub fn mousetrap_record(&self, pos: Vec2) -> Option<TrapRecord> {
        self.mousetraps
            .iter()
            .zip(&self.mousetrap_records)
            .find(|(trap, _)| trap.pos == pos)
            .map(|(_, record)| *record)
    }

    /// The generator record of the trap at `pos`, whichever kind it is.
    pub fn trap_record(&self, pos: Vec2) -> Option<TrapRecord> {
        self.sticky_trap_record(pos)
            .or_else(|| self.mousetrap_record(pos))
            .or_else(|| self.crab_pot_record(pos))
    }

    pub fn crab_pot_record(&self, pos: Vec2) -> Option<TrapRecord> {
        self.crab_pots
            .iter()
            .zip(&self.crab_pot_records)
            .find(|(trap, _)| trap.pos == pos)
            .map(|(_, record)| *record)
    }

    pub fn rat_in_crab_pot(&self) -> bool {
        self.crab_pots.iter().any(|pot| pot.caught_rat)
    }

    /// Is the trap at `pos` a room-mouth encounter (rather than incidental)?
    pub fn is_encounter_trap(&self, pos: Vec2) -> bool {
        self.trap_record(pos).is_some_and(|record| record.encounter)
    }

    /// The generator record of the rat-hole in `cell`, if there is one.
    pub fn rat_hole_record(&self, cell: (i32, i32)) -> Option<RatHoleRecord> {
        self.rat_holes
            .iter()
            .zip(&self.rat_hole_records)
            .find(|(hole, _)| hole.cell == cell)
            .map(|(_, record)| *record)
    }

    /// Every rat-hole with the given role, in generation order.
    pub fn rat_holes_with_role(
        &self,
        role: RatHoleRole,
    ) -> impl Iterator<Item = &RatHole> + '_ {
        self.rat_holes
            .iter()
            .zip(&self.rat_hole_records)
            .filter(move |(_, record)| record.role == role)
            .map(|(hole, _)| hole)
    }

    /// Number of placed objects (generator tests / validation).
    pub fn len(&self) -> usize {
        self.sticky_traps.len()
            + self.mousetraps.len()
            + self.crab_pots.len()
            + self.one_way_doors.len()
            + self.rat_holes.len()
            + self.rewards.len()
            + self.steam_grates.len()
            + self.ink_pads.len()
            + self.lights.len()
            + self.emitters.len()
            + usize::from(self.sewer_opening.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---- The flatbed -----------------------------------------------------------
//
// The trailer level is the one theme whose *hazards are the layout*: a long,
// narrow bed of oak planks with the road running underneath it, cargo strapped
// down in lanes, holes where planks have gone, and gaps where a rail section is
// missing. Only `Theme::Trailer` fills this in; every other theme leaves it
// empty (`TrailerDeck::is_empty`).
//
// Like `LevelObjects`, the data is plain `Copy` records the renderer reads, so
// the drawn holes and the simulated falls cannot disagree.

/// Why a cell is a hole you can fall through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PitKind {
    /// A run of planks has gone: the road is visible through the deck.
    #[default]
    BrokenPlank,
    /// A missing rail section on the bed's open edge.
    RailGap,
    /// A tidal drop at the rim of an Outfall basin.
    TidalDrop,
}

/// One fall hazard: the cells a body can drop through.
#[derive(Clone, Copy, Debug)]
pub struct Pit {
    pub rect: TileRect,
    pub kind: PitKind,
}

impl Pit {
    pub fn new(rect: TileRect, kind: PitKind) -> Pit {
        Pit { rect, kind }
    }

    pub fn center_world(&self) -> Vec2 {
        self.rect.center_world()
    }

    pub fn contains_world(&self, p: Vec2) -> bool {
        self.rect.contains_world(p)
    }
}

/// A crate on ratchet straps that shifts along the deck whenever the trailer
/// hits an expansion joint. The position is a pure function of the run clock
/// ([`TrailerDeck::slide_offset`]), so the sim and the art read the same
/// motion with no per-tick state.
#[derive(Clone, Copy, Debug)]
pub struct SlidingCargo {
    /// Resting centre in world pixels (the crate is drawn here at rest).
    pub rest: Vec2,
    /// Slide axis (unit, along the bed) and travel in pixels.
    pub dir: Vec2,
    pub travel: f32,
    /// Cycle phase offset in seconds, so two crates do not slide together.
    pub phase: f32,
    /// Half-extent of the crate's box on the deck.
    pub half: Vec2,
    /// Seconds the cycle spends creaking before it shifts (the tell).
    pub warn: f32,
}

/// The flatbed's own layout data. Empty on every other theme.
#[derive(Clone, Debug, Default)]
pub struct TrailerDeck {
    /// The walkable bed, in cells.
    pub deck: TileRect,
    /// Where the rat drops onto the tailgate (== `Level::start`).
    pub tailgate: Vec2,
    /// Cells a body can fall through (broken planks and missing rail sections).
    pub pits: Vec<Pit>,
    /// Stretches where the slipstream drags light things toward the tailgate.
    pub wind: Vec<TileRect>,
    /// Cargo that walks along its straps on every bump.
    pub slides: Vec<SlidingCargo>,
}

impl TrailerDeck {
    pub fn is_empty(&self) -> bool {
        self.deck.area() == 0
    }

    /// The pit containing world position `p`, if any.
    pub fn pit_at(&self, p: Vec2) -> Option<&Pit> {
        self.pits.iter().find(|pit| pit.contains_world(p))
    }

    /// Steering push away from a hole the body is about to walk into.
    ///
    /// The flatbed's holes are the one hazard that can *kill* a pursuer (35 % of
    /// its max HP), so a ground body has to treat them like walls. The flow
    /// field is built from the tile grid and does not know about the deck's
    /// pits, and rebuilding a per-level mask for the trailer's few holes is not
    /// worth its own field, so this is the cheap local rule: a body whose own
    /// cell is a hole (or whose step is one tile from one) is pushed straight
    /// away from the nearest hole's centre. One linear scan over the deck's
    /// 3-5 pits, and only on the flatbed.
    ///
    /// Returns `None` on every other theme (the caller checks
    /// [`TrailerDeck::is_empty`] first) and for a body clear of every hole.
    pub fn pit_avoid(&self, p: Vec2, radius: f32) -> Option<Vec2> {
        let mut best: Option<(f32, Vec2)> = None;
        for pit in &self.pits {
            let c = pit.rect.center_world();
            // Distance to the hole's inflated cell rectangle, not to its centre:
            // a hole is a square of deck, so a body a few pixels off its lip is
            // already "about to walk in".
            let half = Vec2::new(
                pit.rect.w as f32 * crate::game::collision::TILE_F * 0.5,
                pit.rect.h as f32 * crate::game::collision::TILE_F * 0.5,
            );
            let d = Vec2::new(
                (p.x - c.x).abs() - half.x,
                (p.y - c.y).abs() - half.y,
            );
            let gap = d.x.max(d.y).max(0.0) - radius;
            if gap > crate::game::collision::TILE_F {
                continue;
            }
            if best.is_none_or(|(bg, _)| gap < bg) {
                best = Some((gap, c));
            }
        }
        best.map(|(_, c)| {
            let away = p - c;
            if away.len_sq() > 1e-4 {
                away.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            }
        })
    }

    /// Is `p` on an exposed stretch where the slipstream pulls toward the
    /// tailgate?
    pub fn windy_at(&self, p: Vec2) -> bool {
        self.wind.iter().any(|w| w.contains_world(p))
    }

    /// One crate's slide offset along `slide.dir`, in pixels, at run clock
    /// `elapsed`.
    ///
    /// The cycle is [`SLIDE_PERIOD`] long: a short creak (`slide.warn`), a
    /// [`SLIDE_TRAVEL_SECS`] shift out along the straps, a hold, then the
    /// straps pull it back. Render-only: no rng and no per-tick state.
    pub fn slide_offset(slide: &SlidingCargo, elapsed: f32) -> f32 {
        let t = (elapsed + slide.phase).rem_euclid(SLIDE_PERIOD);
        let shift = slide.warn + SLIDE_TRAVEL_SECS;
        if t < slide.warn {
            0.0
        } else if t < shift {
            // Out: a smooth shove over the travel window.
            let u = (t - slide.warn) / SLIDE_TRAVEL_SECS;
            slide.travel * (u * u * (3.0 - 2.0 * u))
        } else if t < shift + SLIDE_HOLD_SECS {
            slide.travel
        } else {
            // Back: the straps win.
            let u = (t - shift - SLIDE_HOLD_SECS) / (SLIDE_PERIOD - shift - SLIDE_HOLD_SECS);
            slide.travel * (1.0 - u * u * (3.0 - 2.0 * u))
        }
    }
}

/// One full slide cycle, in seconds (creak + shove + hold + return).
pub const SLIDE_PERIOD: f32 = 7.5;
/// How long the crate takes to walk out along its straps.
pub const SLIDE_TRAVEL_SECS: f32 = 0.9;
/// How long it sits at the end of its straps before returning.
pub const SLIDE_HOLD_SECS: f32 = 1.6;

// ---- Standing water --------------------------------------------------------

/// Lobes around a puddle's rim ([`puddle_wobble`]).
const PUDDLE_LOBES: usize = 8;

/// Softness of a puddle's rim, in world px: [`Puddle::depth`] ramps from 0 to 1
/// over the outer this many pixels, so the art can feather the edge and the
/// lighting has a gradient to reflect in.
pub const PUDDLE_EDGE_PX: f32 = 6.0;

/// The rim wobble of a blob at `angle`, in `0..=1`.
///
/// A fixed [`PUDDLE_LOBES`]-lobe hash-lerp: one lobe's value is an integer hash
/// of `(seed, lobe)`, and the angle lerps (smoothstepped, so the rim's slope is
/// continuous too) between the two lobes it falls between. Integer hash + lerp
/// only — no sin, no table, no rng — so [`Puddle::contains`], [`Puddle::depth`],
/// the art and the lighting all read exactly the same edge.
///
/// The owner's brief ("the floor should be very wet") needs a shape that reads
/// as water rather than as an ellipse; eight lobes is the fewest that gives a
/// blob two independent bays without looking like a gear.
pub fn puddle_wobble(seed: u32, angle: f32) -> f32 {
    let turns = angle / std::f32::consts::TAU;
    let turns = turns.rem_euclid(1.0) * PUDDLE_LOBES as f32;
    let lobe = turns.floor();
    let frac = turns - lobe;
    let k = (lobe as i32).rem_euclid(PUDDLE_LOBES as i32) as u32;
    let a = puddle_lobe(seed, k);
    let b = puddle_lobe(seed, (k + 1) % PUDDLE_LOBES as u32);
    let s = frac * frac * (3.0 - 2.0 * frac);
    a + (b - a) * s
}

/// One lobe's value, in `0..=1`: a small integer hash of `(seed, lobe)`.
fn puddle_lobe(seed: u32, lobe: u32) -> f32 {
    let mut h = seed ^ lobe.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    // The top 24 bits, so the value cannot round up to 1.0 from the full range.
    (h >> 8) as f32 * (1.0 / 16_777_215.0)
}

/// One blob of standing water on a floor, in world pixels.
///
/// Puddles are **floor decoration with a gameplay hook**, not geometry: they
/// never paint, block or slow a tile. The Outfall's Skater uses them for
/// steering and splash attacks. `systems::puddles` reads [`Level::wet_at`]
/// for footsteps, and the renderer draws the wet-floor material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puddle {
    /// Centre, world px. Always on an open floor cell (the generator's rule).
    pub pos: Vec2,
    /// Half-width, world px.
    pub rx: f32,
    /// Half-height, world px.
    pub ry: f32,
    /// Shape variation (the rim's lobes).
    pub seed: u32,
}

/// Seven overlapping puddles laid along a single trench.
#[derive(Clone, Debug)]
pub struct TrenchPuddle {
    pub patches: [Puddle; 7],
    pub expires_at: f32,
}

impl Puddle {
    /// The rim factor at `angle`: `0.78` (pinched) to `1.0` (full).
    pub fn edge_factor(&self, angle: f32) -> f32 {
        0.78 + 0.22 * puddle_wobble(self.seed, angle)
    }

    /// Is `p` inside the blob?
    ///
    /// Irregular by design: inside when `(dx/rx)² + (dy/ry)² <= edge(angle)²`,
    /// where `edge` is [`Self::edge_factor`] at the direction of `p`. The
    /// bounding ellipse (`edge = 1`) therefore always contains the blob.
    pub fn contains(&self, p: Vec2) -> bool {
        if self.rx <= 0.0 || self.ry <= 0.0 {
            return false;
        }
        let d = p - self.pos;
        let nx = d.x / self.rx;
        let ny = d.y / self.ry;
        let radial_sq = nx * nx + ny * ny;
        if radial_sq > 1.0 {
            return false;
        }
        let edge = self.edge_factor(d.y.datan2(d.x));
        radial_sq <= edge * edge
    }

    /// `0.0` outside, `1.0` deep inside, linear over the outer
    /// [`PUDDLE_EDGE_PX`] pixels — the soft edge the art and the lighting use.
    pub fn depth(&self, p: Vec2) -> f32 {
        if self.rx <= 0.0 || self.ry <= 0.0 {
            return 0.0;
        }
        let d = p - self.pos;
        let edge = self.edge_factor(d.y.datan2(d.x));
        let nx = d.x / (self.rx * edge);
        let ny = d.y / (self.ry * edge);
        let nr = (nx * nx + ny * ny).sqrt();
        if nr >= 1.0 {
            return 0.0;
        }
        if nr <= 1e-6 {
            return 1.0;
        }
        // The rim along this ray sits at `|d| / nr`, so the distance still to
        // travel is `|d| * (1 - nr) / nr` px.
        let to_edge = d.len() * (1.0 - nr) / nr;
        (to_edge / PUDDLE_EDGE_PX).clamp(0.0, 1.0)
    }
}

/// What one seabed patch is made of. The Outfall's floor is not water: it is
/// dark gravel, silt and shell rubble, and each material is drawn and lit as
/// matte seabed. Placed by [`super::levelgen::puddles::place_seabed_patches`]
/// where the puddles used to be; never a tile, never a gameplay volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeabedKind {
    /// Coarse dark gravel scattered at wall edges and around plants.
    Gravel,
    /// Pale broken shell grit, the brightest patch material.
    ShellGrit,
    /// Rounded pebbles in a thin, flat scatter.
    Pebbles,
    /// A broad, soft silt field: the largest and flattest of the patches.
    Silt,
    /// A broad field of small flat stones, the other big field material.
    Shingle,
}

/// One patch of mixed seabed material on the Outfall floor, in world pixels.
///
/// Floor decoration with no gameplay hook: it never paints, blocks or slows a
/// tile and is invisible to the bot, the fog and the minimap. The Outfall uses
/// these where the other floors use [`Puddle`]s (the owner: *"why are there
/// little lakes when we are underwater? ... shaded like different kinds of
/// gravel"*). Empty on every floor but [`Theme::Outfall`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeabedPatch {
    /// Centre, world px. Always on an open floor cell (the generator's rule).
    pub center: Vec2,
    /// Half-width, world px.
    pub rx: f32,
    /// Half-height, world px.
    pub ry: f32,
    /// What it is made of.
    pub kind: SeabedKind,
    /// Shape variation (the rim's lobes), the art's tone variation.
    pub seed: u32,
}

impl SeabedPatch {
    /// The rim factor at `angle`: `0.78` (pinched) to `1.0` (full). The same
    /// eight-lobe hash blob [`Puddle`]s use, so the two materials share one
    /// shoreline rule and one soft edge.
    pub fn edge_factor(&self, angle: f32) -> f32 {
        0.78 + 0.22 * puddle_wobble(self.seed, angle)
    }

    /// Is `p` inside the patch? (Same irregular test as [`Puddle::contains`].)
    pub fn contains(&self, p: Vec2) -> bool {
        if self.rx <= 0.0 || self.ry <= 0.0 {
            return false;
        }
        let d = p - self.center;
        let nx = d.x / self.rx;
        let ny = d.y / self.ry;
        let radial_sq = nx * nx + ny * ny;
        if radial_sq > 1.0 {
            return false;
        }
        let edge = self.edge_factor(d.y.datan2(d.x));
        radial_sq <= edge * edge
    }

    /// `0.0` outside, `1.0` deep inside, linear over the outer
    /// [`PUDDLE_EDGE_PX`] pixels — the soft edge the art feathers.
    pub fn depth(&self, p: Vec2) -> f32 {
        if self.rx <= 0.0 || self.ry <= 0.0 {
            return 0.0;
        }
        let d = p - self.center;
        let edge = self.edge_factor(d.y.datan2(d.x));
        let nx = d.x / (self.rx * edge);
        let ny = d.y / (self.ry * edge);
        let nr = (nx * nx + ny * ny).sqrt();
        if nr >= 1.0 {
            return 0.0;
        }
        if nr <= 1e-6 {
            return 1.0;
        }
        let to_edge = d.len() * (1.0 - nr) / nr;
        (to_edge / PUDDLE_EDGE_PX).clamp(0.0, 1.0)
    }
}

/// Kind of decorative aquatic plant rooted on the Outfall seabed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlantKind {
    /// Tall olive/brown kelp frond clustered along walls and room edges.
    Kelp,
    /// Short green seagrass tuft in patches on open sand.
    Seagrass,
    /// Compact coral-pink / sea-orange anemone or sea fan in corners.
    Anemone,
}

/// One decorative aquatic plant on the Outfall seabed, in world pixels.
///
/// Purely decorative: never a tile, never blocking, invisible to gameplay,
/// the bot, fog and the minimap. Placed by [`super::levelgen::seabed`]
/// on its own salted rng so no earlier stage or other theme can move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeabedPlant {
    /// Root anchor on the seabed, world px.
    pub pos: Vec2,
    /// Species (kelp frond, seagrass tuft, or corner anemone/sea fan).
    pub kind: PlantKind,
    /// Continuous sway phase offset in radians (`0..TAU`).
    pub phase: f32,
    /// Size variant (`0..=2`: small, medium, tall).
    pub size: u8,
    /// Deterministic visual variation seed (tone, lean, blade layout).
    pub seed: u32,
}

/// What kind of decorative laundry heap sits on the Laundry floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothesPileKind {
    /// Small loose scatter of a folded shirt and stray sock (~10 × 7 px).
    SmallHeap,
    /// Medium tumbled heap of mixed shirts, towels and socks (~15 × 10 px).
    MediumHeap,
    /// Large slumped mound with a draped bedsheet corner (~20 × 13 px).
    LargeHeap,
    /// Woven laundry basket overflowing with folded clothes (~14 × 13 px).
    Basket,
}

/// One decorative pile of clothes or overflowing basket on the Laundry floor.
///
/// Purely decorative: never paints a tile, never blocks, slows or routes, and
/// is invisible to gameplay, the bot, fog and the minimap. Placed by
/// [`super::levelgen::laundry_piles`] on its own salted rng so the map pin
/// cannot move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothesPile {
    /// Centre, world px. Always on an open floor cell.
    pub pos: Vec2,
    /// Heap size or overflowing basket.
    pub kind: ClothesPileKind,
    /// Deterministic colour and fold variation.
    pub seed: u32,
}

// ---- The level -------------------------------------------------------------

#[derive(bevy_ecs::resource::Resource)]
pub struct Level {
    pub theme: Theme,
    /// The run seed the level was generated from (`Level::new` uses 0).
    pub seed: u64,
    /// `tiles.world_size()`: the playfield in world pixels.
    pub arena: Vec2,
    /// The wall lattice: authoritative for collision and navigation.
    pub tiles: TileMap,
    /// Rendered obstacle objects (their cells are `WallKind::Solid`).
    pub props: Vec<Prop>,
    pub palette: Palette,
    /// Coarse navigation mask, built from the tile grid.
    ///
    /// Interim single field: it uses the **large** size class, so no enemy is
    /// ever routed through a rat-hole it does not fit. The per-class fields
    /// (with near-fine/far-coarse staggering) replace it — see
    /// `CONTENT_PLAN.md` §3.
    pub nav: NavGrid,
    /// Named regions, in generation order (the start room is always first).
    pub rooms: Vec<Room>,
    /// Where the rat spawns: the centre of the start room, always
    /// `arena * 0.5`.
    pub start: Vec2,
    /// At most one maze section: about one map in eight carries a maze
    /// ([`crate::game::levelgen::layout::MAZE_ONE_IN`]).
    pub mazes: Vec<MazeSection>,
    /// Traps, doors, rat-holes and maze rewards.
    pub objects: LevelObjects,
    /// Empty on every generated map: batch 12 removed the boss arena, so no
    /// theme furnishes one. The field survives because the Flatbed's own
    /// generator and the sandbox [`Level`] literals in `render::library` build
    /// it; `levelgen::generate` clears whatever a theme's generator left here.
    /// The boss's fight rectangle is built when the boss arrives, around the
    /// rat ([`Level::fight_rect_around`]).
    pub boss: BossArena,
    /// The flatbed's bed, pits, wind stretches and sliding cargo. Empty on
    /// every theme but [`Theme::Trailer`].
    pub trailer: TrailerDeck,
    /// Standing water on the floor (Sewer, Laundry, Kitchen). Render-only
    /// decoration plus the rat's footstep splashes; placed by
    /// [`super::levelgen::puddles`] on its own salted rng after every other
    /// stage, so it cannot move the map pin. Empty on the Lab, the Printworks,
    /// the Flatbed **and the Outfall** (whose floor is seabed, not lakes).
    pub puddles: Vec<Puddle>,
    /// Mixed gravel, silt and shell patches on the Outfall seabed. Render-only
    /// floor decoration placed by
    /// [`super::levelgen::puddles::place_seabed_patches`] on the same salted
    /// rng as the puddles, so it cannot move the map pin. Empty on every theme
    /// except [`Theme::Outfall`].
    pub seabed_patches: Vec<SeabedPatch>,
    /// Decorative aquatic plants on the Outfall seabed. Render-only; placed
    /// by [`super::levelgen::seabed`] on its own salted rng so no earlier
    /// stage or other theme can move. Empty on every theme except
    /// [`Theme::Outfall`].
    pub plants: Vec<SeabedPlant>,
    /// Decorative heaps of clothes and overflowing baskets (Laundry only).
    /// Placed by [`super::levelgen::laundry_piles`] on its own salted rng,
    /// so it cannot move the map pin. Empty on every other theme.
    pub clothes_piles: Vec<ClothesPile>,
}

/// The fight furniture a boss arena used to carry (`DESIGN_REVIEW.md` §5).
///
/// **Batch 12 removed the boss arena.** No generator fills this any more: a
/// generated level's `boss` field is [`BossArena::default`] and no room has
/// [`RoomKind::Boss`]. The type stays only because the Flatbed's cab-hold
/// generator and the Library's sandbox `Level` literals still build one; the
/// boss's dock is now derived from the tile map at arrival time, and the fight
/// rectangle is recentred on the rat.
#[derive(Clone, Debug)]
pub struct BossArena {
    /// The clear encounter floor at the arena's centre (always empty now).
    pub floor: TileRect,
    /// World position of the charging dock pad's centre (Lab only), as the art's
    /// [`crate::game::art_boss_mower::draw_dock_pad`] footprint wants it (32 ×
    /// 30 px around this point).
    pub dock: Option<Vec2>,
    /// The dock pad's cell (same place as [`Self::dock`]).
    pub dock_cell: Option<(i32, i32)>,
    /// Solid bumper cells (Lab only), each a 1 × 3 or 3 × 1 bar.
    pub bumpers: Vec<TileRect>,
    /// The rat-hole in the arena's wall ring that led out through a duct
    /// (Sewer only).
    pub escape_hole: Option<(i32, i32)>,
    /// The arena's mouth cells: the doorway cells in its wall ring.
    pub mouths: Vec<(i32, i32)>,
}

impl Default for BossArena {
    fn default() -> Self {
        BossArena {
            floor: TileRect::new(0, 0, 0, 0),
            dock: None,
            dock_cell: None,
            bumpers: Vec::new(),
            escape_hole: None,
            mouths: Vec::new(),
        }
    }
}

impl Level {
    /// Generate the level for `(theme, seed)`. Deterministic; retries with a
    /// salted seed if the generator produces an unplayable map (see
    /// [`levelgen::validate`]).
    pub fn generate(theme: Theme, seed: u64) -> Level {
        levelgen::generate(theme, seed)
    }

    /// `Level::generate(theme, 0)`: the convenience constructor for tests and
    /// callers that do not have a run seed. New code should pass the run seed.
    pub fn new(theme: Theme) -> Level {
        Level::generate(theme, 0)
    }

    /// A blank, wall-free level of the standard size.
    ///
    /// Used for the facade's round-trip spare (never simulated) and as the
    /// base for tests that build their own walls. It is *not* a playable map.
    pub fn empty(theme: Theme) -> Level {
        let tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, 0);
        let nav = NavGrid::from_tiles(&tiles, SizeClass::Large);
        Level {
            theme,
            seed: 0,
            arena: MAP_SIZE,
            tiles,
            props: Vec::new(),
            palette: Palette::for_theme(theme),
            nav,
            rooms: Vec::new(),
            start: MAP_SIZE * 0.5,
            mazes: Vec::new(),
            objects: LevelObjects::default(),
            boss: BossArena::default(),
            trailer: TrailerDeck::default(),
            puddles: Vec::new(),
            seabed_patches: Vec::new(),
            plants: Vec::new(),
            clothes_piles: Vec::new(),
        }
    }

    /// Art view for one wall cell, with the door state and run clock filled in.
    pub fn wall_view(&self, cx: i32, cy: i32, anim_clock: f32) -> WallCellView {
        let mut view = self.tiles.wall_view(cx, cy);
        view.anim_clock = anim_clock;
        if self.tiles.one_way_dir(cx, cy).is_some() {
            if let Some(door) = self.objects.door_at(cx, cy) {
                view.open_progress = door.open_progress();
            }
        }
        view
    }

    /// The room containing a world position, if it is inside one.
    pub fn room_at(&self, p: Vec2) -> Option<&Room> {
        self.rooms.iter().find(|r| r.rect.contains_world(p))
    }

    /// The start room (always present, always first in `rooms`).
    pub fn start_room(&self) -> Option<&Room> {
        self.rooms.iter().find(|r| r.kind == RoomKind::Start)
    }

    /// The boss arena's room, if the map has one.
    ///
    /// **Always `None` on a generated map since batch 12**: the boss arrives
    /// where the rat is, and the footprint the arena used to take is an ordinary
    /// [`RoomKind::Hall`]. Kept because the art and render captures stage a
    /// boss against `boss_room()` when a hand-built level provides one.
    pub fn boss_room(&self) -> Option<&Room> {
        self.rooms.iter().find(|r| r.kind == RoomKind::Boss)
    }

    /// The rectangle a boss fight happens in: the nominal fight size
    /// ([`BOSS_FIGHT_TILES`], the old arena's) centred on `pos` and clamped
    /// inside the map.
    ///
    /// This is the whole of what the arena used to provide, and the scheduled
    /// arrival (15:00) and the console's `boss <kind> here` both use it, so a
    /// boss fights where it caught the rat.
    pub fn fight_rect_around(&self, pos: Vec2) -> TileRect {
        let cols = self.tiles.cols().max(1);
        let rows = self.tiles.rows().max(1);
        let w = BOSS_FIGHT_TILES.0.clamp(1, cols);
        let h = BOSS_FIGHT_TILES.1.clamp(1, rows);
        let (cx, cy) = crate::game::collision::cell_of(pos);
        TileRect::new(
            (cx - w / 2).clamp(0, (cols - w).max(0)),
            (cy - h / 2).clamp(0, (rows - h).max(0)),
            w,
            h,
        )
    }

    /// The largest hall on the level (first in generation order on a tie): "a
    /// big open room" for tests and captures. The far hall the old boss arena
    /// became is usually it.
    pub fn largest_hall(&self) -> Option<&Room> {
        let mut best: Option<&Room> = None;
        for room in self.rooms.iter().filter(|r| r.kind == RoomKind::Hall) {
            if best.is_none_or(|b| room.rect.area() > b.rect.area()) {
                best = Some(room);
            }
        }
        best
    }

    /// Every room of one kind, in generation order.
    pub fn rooms_of(&self, kind: RoomKind) -> impl Iterator<Item = &Room> {
        self.rooms.iter().filter(move |r| r.kind == kind)
    }

    /// A random open, wall-clear cell centre inside `area` for `class`, using
    /// `rng` (deterministic for a given rng state).
    ///
    /// Returns `None` when `area` has no room for such a cell — callers must
    /// have a fallback and must not loop forever.
    pub fn random_open_cell(
        &self,
        rng: &mut StdRng,
        area: TileRect,
        class: SizeClass,
    ) -> Option<Vec2> {
        use rand::RngExt;
        const TRIES: u32 = 24;
        for _ in 0..TRIES {
            let cx = rng.random_range(area.x..area.right().max(area.x + 1));
            let cy = rng.random_range(area.y..area.bottom().max(area.y + 1));
            if self.tiles.passable(cx, cy, class) {
                let centre = TileMap::tile_rect(cx, cy).center();
                if self.theme != Theme::Outfall || self.pit_at(centre).is_none() {
                    return Some(centre);
                }
            }
        }
        None
    }

    /// Is every cell touching `rect` walkable floor for the player's class?
    pub fn is_open_floor(&self, rect: Rectf) -> bool {
        self.tiles.rect_clear(rect, SizeClass::Small)
    }

    /// The fall hazard containing `p` on the Flatbed or in the Outfall.
    pub fn pit_at(&self, p: Vec2) -> Option<&Pit> {
        if self.theme == Theme::Outfall {
            self.objects.tidal_pits.iter().find(|pit| pit.contains_world(p))
        } else {
            self.trailer.pit_at(p)
        }
    }

    /// Whether a body footprint meets tidal water. Flatbed callers retain
    /// their existing deck-specific fall and recovery rules.
    pub fn touches_tidal_pit(&self, pos: Vec2, radius: f32) -> bool {
        self.objects.tidal_pits.iter().any(|pit| {
            pit.contains_world(pos)
                || super::collision::circle_rect_overlap(pos, radius, &pit.rect.to_world())
        })
    }

    /// Whether an Outfall owner currently holds this fixture dark.
    pub fn sconce_snuffed(&self, index: usize) -> bool {
        self.objects.outfall_elite_snuff == Some(index)
            || self.objects.outfall_boss_snuff == Some(index)
    }

    /// Closest visible, working mount, with fixture order breaking ties.
    pub fn nearest_live_sconce(&self, from: Vec2, reach: f32) -> Option<(usize, Vec2)> {
        let mut best = None;
        let mut best_distance = reach * reach;
        for (index, fixture) in self.objects.lights.iter().enumerate() {
            let distance = from.dist_sq(fixture.pos);
            if (distance < best_distance || (best.is_none() && distance == best_distance))
                && fixture.is_live_sconce()
                && !self.sconce_snuffed(index)
                && self.tiles.segment_hit(from, fixture.pos, 0.0).is_none()
            {
                best = Some((index, fixture.pos));
                best_distance = distance;
            }
        }
        best
    }

    /// Is `p` on standing water? The puddle test only, and **floor cells only**:
    /// a blob's rim may spill over a wall face (the wall layer is drawn over it)
    /// and that is not water the rat can step in.
    pub fn wet_at(&self, p: Vec2) -> bool {
        if self.puddles.is_empty() && self.objects.trench_puddles.is_empty() {
            return false;
        }
        self.tiles.get_at(p).is_open() && self.standing_water().any(|puddle| puddle.contains(p))
    }

    /// The seabed material covering `p`, if any. The last patch in placement
    /// order wins, which is the one drawn on top. Floor decoration only: it is
    /// never water, never a tile and never read by the fog or the minimap.
    pub fn seabed_at(&self, p: Vec2) -> Option<SeabedKind> {
        self.seabed_patches
            .iter()
            .rev()
            .find(|patch| patch.contains(p))
            .map(|patch| patch.kind)
    }

    pub fn standing_water(&self) -> impl Iterator<Item = &Puddle> {
        self.puddles
            .iter()
            .chain(self.objects.trench_puddles.iter().flat_map(|scar| &scar.patches))
    }
}

/// Deterministic seed for a level: mixes the run seed with the theme so the
/// three maps of one run are all distinct but reproducible.
pub fn level_seed(theme: Theme, seed: u64, attempt: u32) -> u64 {
    let t = match theme {
        Theme::Sewer => 0x9e37_79b9_7f4a_7c15u64,
        Theme::Kitchen => 0xc2b2_ae3d_27d4_eb4fu64,
        Theme::Lab => 0x1656_67b1_9e37_79f9u64,
        Theme::Laundry => 0x41c6_4f0a_b4e8_d2c7u64,
        Theme::Printworks => 0x8f2d_9e55_1c73_a6b1u64,
        Theme::Trailer => 0x2f1d_4b8c_9a63_0d57u64,
        Theme::Outfall => 0x5b27_8cc2_a86f_34d1u64,
    };
    let mut h = seed ^ t ^ ((attempt as u64) << 32);
    // splitmix64 finaliser: plenty to decorrelate neighbouring seeds.
    h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The `StdRng` a level is generated from.
pub fn level_rng(theme: Theme, seed: u64, attempt: u32) -> StdRng {
    StdRng::seed_from_u64(level_seed(theme, seed, attempt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;

    #[test]
    fn map_size_matches_the_cell_grid() {
        assert_eq!(MAP_SIZE, Vec2::new((MAP_COLS * TILE) as f32, (MAP_ROWS * TILE) as f32));
        // "About ten times today's area": 3200 × 2560 vs the old 1020 × 800.
        let area = MAP_SIZE.x * MAP_SIZE.y;
        let old = 1020.0 * 800.0;
        assert!(area >= 10.0 * old, "{area} is less than ten old arenas");
    }

    #[test]
    fn placeholder_level_has_a_border_and_a_start_room() {
        for theme in Theme::all() {
            let level = Level::new(theme);
            assert_eq!(level.arena, MAP_SIZE);
            assert_eq!(level.start, MAP_SIZE * 0.5);
            assert!(level.tiles.wall_count() > 100, "{theme:?}: no walls");
            // The border is solid on every cell of the outermost ring.
            for cx in 0..MAP_COLS {
                assert!(level.tiles.get(cx, 0).is_wall(), "{theme:?}: border hole");
                assert!(level.tiles.get(cx, MAP_ROWS - 1).is_wall());
            }
            let start = level.start_room().expect("start room");
            assert_eq!(start.kind, RoomKind::Start);
            assert!(start.rect.contains_world(level.start));
            // No boss arena (batch 12): the map is still a map without one, and
            // the fight rectangle is built around the rat when a boss arrives.
            assert!(level.boss_room().is_none(), "{theme:?}: a boss arena");
            let fight = level.fight_rect_around(level.start);
            assert_eq!(
                (fight.w, fight.h),
                BOSS_FIGHT_TILES,
                "{theme:?}: the fight rectangle is not the nominal size"
            );
            assert!(
                fight.contains_world(level.start),
                "{theme:?}: the fight rectangle does not hold the rat"
            );
            // The start room is clear of walls around the spawn point.
            let spawn = Rectf::new(level.start.x - 12.0, level.start.y - 12.0, 24.0, 24.0);
            assert!(level.is_open_floor(spawn), "{theme:?}: spawn is blocked");
        }
    }

    #[test]
    fn generated_level_places_the_required_objects() {
        let level = Level::new(Theme::Sewer);
        let holes = level
            .tiles
            .iter_walls()
            .filter(|(_, _, k)| matches!(k, WallKind::RatHole { .. }))
            .count();
        // Stage B grows several rat-holes in the structural walls.
        assert!(holes > 0, "no rat-holes in the map");
        assert_eq!(level.objects.rat_holes.len(), holes);
        // One door and one reward per maze section (0..=2 of them).
        assert!(level.mazes.len() <= 2);
        assert_eq!(level.objects.one_way_doors.len(), level.mazes.len());
        assert_eq!(level.objects.rewards.len(), level.mazes.len());
        for (door, maze) in level.objects.one_way_doors.iter().zip(&level.mazes) {
            assert_eq!(door.cell, maze.exit);
            assert!(
                door.view
                    .facing
                    .dot(crate::game::collision::cardinal_vec(maze.exit_dir))
                    > 0.5
            );
        }
        // Traps sit on the floor, away from the spawn.
        let traps = level.objects.sticky_traps.len() + level.objects.mousetraps.len();
        assert!(traps > 0, "a furnished map grows traps");
        for pos in level
            .objects
            .sticky_traps
            .iter()
            .map(|t| t.pos)
            .chain(level.objects.mousetraps.iter().map(|t| t.pos))
        {
            assert!(level.tiles.get_at(pos).is_open(), "trap at {pos:?} in a wall");
            assert!(
                pos.dist(level.start) > 6.0 * crate::game::collision::TILE_F,
                "trap at {pos:?} on the spawn"
            );
        }
    }

    /// Every non-open cell as `(cx, cy, kind)`: the structural digest of a map
    /// (the tile grid is the only wall data there is).
    fn wall_cells(level: &Level) -> Vec<(i32, i32, WallKind)> {
        level.tiles.iter_walls().collect()
    }

    #[test]
    fn generation_is_deterministic_and_seed_sensitive() {
        for theme in Theme::all() {
            let a = Level::generate(theme, 7);
            let b = Level::generate(theme, 7);
            assert_eq!(a.tiles.wall_count(), b.tiles.wall_count());
            assert_eq!(wall_cells(&a), wall_cells(&b), "{theme:?}: same seed, different map");
            assert_eq!(a.start, b.start);
            let c = Level::generate(theme, 8);
            assert_ne!(
                wall_cells(&a),
                wall_cells(&c),
                "{theme:?}: different seeds produced the same map"
            );
        }
    }

    #[test]
    fn empty_level_is_wall_free_and_centred() {
        let level = Level::empty(Theme::Lab);
        assert_eq!(level.tiles.wall_count(), 0);
        assert_eq!(level.start, MAP_SIZE * 0.5);
        assert_eq!(level.arena, MAP_SIZE);
    }

    /// Does `tiles` have any collider overlapping a circle of `r` at `p`?
    fn overlaps_a_wall(
        tiles: &TileMap,
        p: Vec2,
        r: f32,
        class: crate::game::collision::SizeClass,
    ) -> bool {
        let mut hit = false;
        let m = r + crate::game::collision::TILE_F;
        tiles.for_each_collider_in(
            Rectf::new(p.x - m, p.y - m, 2.0 * m, 2.0 * m),
            class,
            None,
            |rect| {
                if crate::game::collision::circle_rect_overlap(p, r, &rect) {
                    hit = true;
                }
            },
        );
        hit
    }

    /// Is there plain floor (not a doorway or a hole) within `reach` cells?
    /// A circle buried in a wall block that thick has nowhere to be ejected
    /// to, which the tile grid documents as best-effort; the property below
    /// only speaks about walls that have open floor nearby.
    fn has_open_floor_near(tiles: &TileMap, cx: i32, cy: i32, reach: i32) -> bool {
        for y in (cy - reach)..=(cy + reach) {
            for x in (cx - reach)..=(cx + reach) {
                if matches!(tiles.get(x, y), TileKind::Open) {
                    return true;
                }
            }
        }
        false
    }

    /// Property over many seeds: on every generated map a circle pushed out of
    /// the walls is never left overlapping one — for the rat's class and for a
    /// brute's — whenever there is open floor within the tile grid's search
    /// window, and the bounded nearest-open search always finds a passable
    /// cell there.
    #[test]
    fn every_generated_map_ejects_circles_out_of_its_walls() {
        use crate::game::collision::SizeClass;

        let mut probes = 0;
        for theme in Theme::all() {
            for seed in 0..10u64 {
                let level = Level::generate(theme, seed);
                let mut seen = 0u32;
                for (cx, cy, _) in level.tiles.iter_walls() {
                    // One wall cell in 17 keeps this quick while still covering
                    // every part of the map.
                    seen += 1;
                    if !seen.is_multiple_of(17) {
                        continue;
                    }
                    if !has_open_floor_near(&level.tiles, cx, cy, 3) {
                        continue;
                    }
                    let c = TileMap::tile_rect(cx, cy).center();
                    for (class, r) in [(SizeClass::Small, 3.0), (SizeClass::Large, 6.5)] {
                        probes += 1;
                        let p = level.tiles.resolve_circle(c, r, class, None);
                        assert!(
                            !overlaps_a_wall(&level.tiles, p, r, class),
                            "{theme:?} seed {seed}: {class:?} circle buried at {c:?} \
                             was left overlapping a wall at {p:?}"
                        );
                        let n = level.tiles.nearest_open_center(c, class, 4).unwrap_or_else(|| {
                            panic!("{theme:?} seed {seed}: no open cell near {c:?}")
                        });
                        let (nx, ny) = (
                            (n.x / crate::game::collision::TILE_F).floor() as i32,
                            (n.y / crate::game::collision::TILE_F).floor() as i32,
                        );
                        assert!(
                            level.tiles.passable(nx, ny, class),
                            "{theme:?} seed {seed}: nearest_open_center returned a wall"
                        );
                    }
                }
            }
        }
        assert!(probes > 200, "the property only saw {probes} probes");
    }

    // ---- Standing water ---------------------------------------------------

    /// A blobby puddle for the shape tests: the generator's biggest bands.
    fn test_puddle(seed: u32) -> Puddle {
        Puddle {
            pos: Vec2::new(400.0, 300.0),
            rx: 64.0,
            ry: 40.0,
            seed,
        }
    }

    #[test]
    fn the_wobble_is_bounded_and_smooth() {
        for seed in [0u32, 1, 7, 0xdead_beef, u32::MAX] {
            // Bounded, and never flat: an eight-lobe hash must actually wobble.
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            let mut prev = puddle_wobble(seed, 0.0);
            for i in 0..720 {
                let angle = i as f32 * (std::f32::consts::TAU / 720.0);
                let v = puddle_wobble(seed, angle);
                assert!((0.0..=1.0).contains(&v), "seed {seed}: wobble {v} out of 0..=1");
                lo = lo.min(v);
                hi = hi.max(v);
                assert!(
                    (v - prev).abs() < 0.05,
                    "seed {seed}: the rim jumps {prev} -> {v} at {angle} rad"
                );
                prev = v;
            }
            assert!(hi - lo > 0.05, "seed {seed}: the rim is (nearly) a circle");
            // A full turn is a full turn: the function is periodic.
            assert!((puddle_wobble(seed, 0.0) - puddle_wobble(seed, std::f32::consts::TAU)).abs() < 1e-5);
            assert!((puddle_wobble(seed, 0.7) - puddle_wobble(seed, 0.7 + std::f32::consts::TAU * 3.0)).abs() < 1e-5);
        }
    }

    #[test]
    fn contains_is_inside_the_bounding_ellipse() {
        for seed in 0..24u32 {
            let p = test_puddle(seed);
            for i in 0..360 {
                let angle = i as f32 * (std::f32::consts::TAU / 360.0);
                // `contains` reads the rim at the point's *geometric* bearing,
                // which is not the ellipse parameter unless `rx == ry`.
                let bearing = (angle.dsin() * p.ry).datan2(angle.dcos() * p.rx);
                // Just outside the blob at that angle must be outside.
                let r = p.edge_factor(bearing) + 0.01;
                let (s, c) = angle.dsin_cos();
                let on = p.pos + Vec2::new(c * p.rx * r, s * p.ry * r);
                assert!(!p.contains(on), "seed {seed}: a point outside the edge is wet");
                // And just inside must be inside.
                let r = p.edge_factor(bearing) * 0.99;
                let inside = p.pos + Vec2::new(c * p.rx * r, s * p.ry * r);
                assert!(p.contains(inside), "seed {seed}: a point inside the edge is dry");
                // The bounding ellipse holds every blob.
                assert!(r <= 1.0);
            }
            // The blob never reaches the bounding ellipse's own rim: its edge
            // factor tops out at 1.0 and the wobble is <= 1.
            assert!(p.edge_factor(0.0) <= 1.0);
            assert!(p.contains(p.pos));
        }
    }

    #[test]
    fn depth_is_zero_outside_and_one_at_the_centre() {
        let p = test_puddle(11);
        assert_eq!(p.depth(p.pos), 1.0);
        assert_eq!(p.depth(p.pos + Vec2::new(0.0, -2.0)), 1.0, "a few px in is deep");
        // Far outside is exactly zero, and the soft band ramps between.
        assert_eq!(p.depth(p.pos + Vec2::new(p.rx, 0.0)), 0.0);
        assert_eq!(p.depth(p.pos + Vec2::new(0.0, p.ry * 3.0)), 0.0);
        let rim = p.pos + Vec2::new(p.rx * p.edge_factor(0.0) * 0.97, 0.0);
        let d = p.depth(rim);
        assert!(d > 0.0 && d < 1.0, "the outer band is a ramp, got {d}");
        // `contains` and `depth > 0` are the same set.
        for i in 0..200 {
            let angle = i as f32 * 0.37;
            let r = (i as f32 / 200.0) * 1.3;
            let (s, c) = angle.dsin_cos();
            let q = p.pos + Vec2::new(c * p.rx * r, s * p.ry * r);
            assert_eq!(p.contains(q), p.depth(q) > 0.0, "contains and depth disagree");
        }
        // A degenerate puddle is not water anywhere.
        let flat = Puddle {
            rx: 0.0,
            ..p
        };
        assert!(!flat.contains(flat.pos));
        assert_eq!(flat.depth(flat.pos), 0.0);
    }

    #[test]
    fn wet_at_is_floor_only() {
        // A blob over the start: the start cell is open floor, so it is wet,
        // while a border-wall cell the blob also covers is not.
        let mut level = Level::new(Theme::Sewer);
        level.puddles = vec![Puddle {
            pos: level.start,
            rx: 90.0,
            ry: 60.0,
            seed: 5,
        }];
        assert!(level.wet_at(level.start), "the open start cell is not water");
        assert!(
            !level.wet_at(Vec2::new(4.0, 4.0)),
            "a border wall cell is water"
        );
        // A dry level answers false everywhere without walking the list.
        let dry = Level::empty(Theme::Sewer);
        assert!(!dry.wet_at(dry.start));
    }

    #[test]
    fn a_seabed_patch_shares_the_puddle_blob_and_is_never_water() {
        let patch = SeabedPatch {
            center: Vec2::new(400.0, 300.0),
            rx: 64.0,
            ry: 40.0,
            kind: SeabedKind::Shingle,
            seed: 11,
        };
        assert!(patch.contains(patch.center));
        assert_eq!(patch.depth(patch.center), 1.0);
        assert_eq!(patch.depth(patch.center + Vec2::new(patch.rx, 0.0)), 0.0);
        // `contains` and `depth > 0` are the same set, on the same 8-lobe rim.
        for i in 0..200 {
            let angle = i as f32 * 0.37;
            let r = (i as f32 / 200.0) * 1.3;
            let (s, c) = angle.dsin_cos();
            let q = patch.center + Vec2::new(c * patch.rx * r, s * patch.ry * r);
            assert_eq!(patch.contains(q), patch.depth(q) > 0.0, "contains and depth disagree");
        }
        assert_eq!(
            patch.edge_factor(0.0),
            Puddle { pos: patch.center, rx: patch.rx, ry: patch.ry, seed: patch.seed }
                .edge_factor(0.0),
            "a patch must use the puddle rim"
        );
        // A degenerate patch covers nothing.
        let flat = SeabedPatch { rx: 0.0, ..patch };
        assert!(!flat.contains(flat.center));
        assert_eq!(flat.depth(flat.center), 0.0);
        // And a level reports its material without calling it water.
        let mut level = Level::empty(Theme::Outfall);
        level.seabed_patches.push(patch);
        assert_eq!(level.seabed_at(patch.center), Some(SeabedKind::Shingle));
        assert!(!level.wet_at(patch.center), "a seabed patch is not water");
        assert_eq!(level.seabed_at(Vec2::new(4.0, 4.0)), None);
    }
}
