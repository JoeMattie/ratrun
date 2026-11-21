//! Level data model: theme, palette, the tile grid, rooms and level objects.
//!
//! A level is a **200 × 160 cell** lattice of 16 px tiles
//! ([`collision::TileMap`]) — ten times today's arena area — plus the
//! generator's named regions ([`Room`]), the maze sections ([`MazeSection`]),
//! the interactive objects ([`LevelObjects`]: traps, one-way doors, rat-holes,
//! maze rewards) and the legacy `props` (themed furniture, drawn by
//! `art_props.rs`).
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
//! keeps a 200 × 160 map as cheap per tick as the old 1020 × 800 arena.

use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::math::Vec2;
use crate::render::palette::Rgb;

use super::art_traps::{
    MazeRewardView, MousetrapView, OneWayDoorState, OneWayDoorView, RatHoleView, StickyTrapView,
};
use super::art_walls::{Cardinal, WallCellView};
use super::collision::{SizeClass, TileMap, TILE, TILE_F};
use super::levelgen;
use super::nav::NavGrid;

/// Level width in 16 px cells (10× the old arena's area).
pub const MAP_COLS: i32 = 200;
/// Level height in 16 px cells.
pub const MAP_ROWS: i32 = 160;
/// Level size in world pixels (`MAP_COLS × TILE` by `MAP_ROWS × TILE`).
pub const MAP_SIZE: Vec2 = Vec2 { x: 3200.0, y: 2560.0 };

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
}

impl Theme {
    /// Every theme in facility order: the campaign's three stages first (the
    /// original route), then the two side-route floors.
    pub fn all() -> [Theme; 5] {
        [
            Theme::Sewer,
            Theme::Kitchen,
            Theme::Lab,
            Theme::Laundry,
            Theme::Printworks,
        ]
    }

    /// The campaign stage (0, 1 or 2) a theme belongs to: the Sewer is stage
    /// one, the Kitchen and the Laundry are both stage two, the Lab and the
    /// Printworks are both stage three.
    pub fn stage(self) -> usize {
        match self {
            Theme::Sewer => 0,
            Theme::Kitchen | Theme::Laundry => 1,
            Theme::Lab | Theme::Printworks => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Sewer => "Sewer",
            Theme::Kitchen => "Kitchen",
            Theme::Lab => "Lab",
            Theme::Laundry => "Laundry",
            Theme::Printworks => "Printworks",
        }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
/// consumes them (spawn placement, boss arena, HUD) only needs the rect.
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

/// Every non-wall object placed on the map. Mutated by the simulation (trap
/// and door systems, later phase) and read by the renderer.
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
    /// Steam grates (Laundry): walkable floor vents with a warn/plume cycle.
    /// See [`super::hazards`].
    pub steam_grates: Vec<super::hazards::SteamGrate>,
    /// Ink pads (Printworks): permanent walkable stains that slow ground
    /// movement. See [`super::hazards`].
    pub ink_pads: Vec<super::hazards::InkPad>,
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
            + self.one_way_doors.len()
            + self.rat_holes.len()
            + self.rewards.len()
            + self.steam_grates.len()
            + self.ink_pads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
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
    /// What the boss arena furnishes for its fight (the central clear floor,
    /// the Lab's bumpers and charging dock, the Sewer's escape rat-hole).
    pub boss: BossArena,
}

/// The fight furniture inside the boss arena, exported by the generator so the
/// boss framework, the art and the validator agree on one place instead of
/// re-deriving it (`DESIGN_REVIEW.md` §5).
///
/// Every theme gets a `floor`; the rest is theme-specific:
///
/// - **Lab** (mower): two `bumpers` flanking the floor, so a 135 px charge
///   always has a solid to flip onto within a screenful, and a `dock` pad
///   against a wall away from the mouth.
/// - **Sewer** (jaguar): an `escape_hole` in the wall ring, the rat-hole the
///   rat ducks through so the pounce overshoots.
/// - **Kitchen** (human with a broom): just the 96 × 80 px clear floor; the
///   barrels/counter ends stay at the perimeter.
#[derive(Clone, Debug)]
pub struct BossArena {
    /// The clear encounter floor at the arena's centre. For the Lab every cell
    /// of it has a solid within [`levelgen::layout::SOLID_REACH_PX`] in at
    /// least two cardinal directions — the mower's flip is always on offer
    /// from here.
    pub floor: TileRect,
    /// World position of the charging dock pad's centre (Lab only): the art's
    /// [`crate::game::art_boss_mower::draw_dock_pad`] footprint is 32 × 30 px
    /// around it, so the pad sits against the wall without overlapping one.
    pub dock: Option<Vec2>,
    /// The dock pad's cell (same place as [`Self::dock`]; for the minimap and
    /// the generator's own checks).
    pub dock_cell: Option<(i32, i32)>,
    /// Solid bumper cells (Lab only), each a 1 × 3 or 3 × 1 bar.
    pub bumpers: Vec<TileRect>,
    /// The rat-hole in the arena's wall ring that leads out through a duct
    /// (Sewer only).
    pub escape_hole: Option<(i32, i32)>,
    /// The arena's mouth cells: the doorway cells in its wall ring. Nothing
    /// the arena furnishes may block them.
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

impl BossArena {
    /// The charging dock's world position (Lab only).
    pub fn dock_pos(&self) -> Option<Vec2> {
        self.dock
    }

    /// Where the boss framework should put the mower's dock: the exported pad
    /// in the Lab, the arena's centre on the other maps (the dock is unused
    /// there).
    pub fn dock_or_centre(&self, centre: Vec2) -> Vec2 {
        self.dock.unwrap_or(centre)
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

    /// The boss arena, if the map has one.
    pub fn boss_room(&self) -> Option<&Room> {
        self.rooms.iter().find(|r| r.kind == RoomKind::Boss)
    }

    /// The boss arena's fight furniture, when the map has an arena.
    pub fn boss_arena(&self) -> Option<&BossArena> {
        self.boss_room().map(|_| &self.boss)
    }

    /// The Lab's charging dock pad centre (see [`BossArena::dock`]).
    pub fn boss_dock(&self) -> Option<Vec2> {
        self.boss_arena().and_then(|a| a.dock)
    }

    /// The clear encounter floor at the arena's centre.
    pub fn boss_clear_floor(&self) -> Option<TileRect> {
        self.boss_arena().map(|a| a.floor)
    }

    /// The Lab's solid bumpers (empty on the other maps).
    pub fn boss_bumpers(&self) -> &[TileRect] {
        self.boss_arena().map_or(&[], |a| a.bumpers.as_slice())
    }

    /// The Sewer's escape rat-hole in the arena's wall ring.
    pub fn boss_escape_hole(&self) -> Option<(i32, i32)> {
        self.boss_arena().and_then(|a| a.escape_hole)
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
                return Some(TileMap::tile_rect(cx, cy).center());
            }
        }
        None
    }

    /// Is every cell touching `rect` walkable floor for the player's class?
    pub fn is_open_floor(&self, rect: Rectf) -> bool {
        self.tiles.rect_clear(rect, SizeClass::Small)
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
    fn placeholder_level_has_a_border_start_and_boss_arena() {
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
            let boss = level.boss_room().expect("boss arena");
            assert!(boss.rect.area() >= 20 * 20, "boss arena is too small");
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
}
