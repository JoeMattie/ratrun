//! Collision helpers: the tile grid that owns every wall, circle/rect
//! resolution and a uniform spatial grid so the many-bullets-vs-many-enemies
//! test stays near O(n).
//!
//! # The tile grid ([`TileMap`])
//!
//! Procedural levels are a 16 × 16 px lattice shared by **walls, navigation
//! and collision** ([`art_walls`](super::art_walls) is the art + geometry
//! spec). A wall query is therefore a lookup in the handful of cells that
//! touch a circle, never an iteration over a `Vec<Rectf>` of every wall in the
//! level: with ten times the walls, per-tick cost stays constant.
//!
//! The exact solid rectangles of a cell come from
//! [`art_walls::wall_colliders`], so art and physics cannot disagree. Two rules
//! sit on top of that:
//!
//! - **Size classes.** A rat-hole cell is one solid cell for a large agent and
//!   a 8 px gap for a small one; see [`SizeClass`].
//! - **One-way doors.** The gate leaf blocks everyone except a player heading
//!   along the allowed direction, and navigation has no edge against it; see
//!   [`TileMap::travel_ok`].

use crate::math::Vec2;

use super::art_walls::{self, Cardinal, NeighborMask, WallCellView, WallKind};
use super::enemy::EnemyKind;
use super::level::{Rectf, Theme};

pub fn circles_overlap(a: Vec2, ar: f32, b: Vec2, br: f32) -> bool {
    let r = ar + br;
    a.dist_sq(b) <= r * r
}

/// Swept circle test: does the segment `a -> b` (a bullet of radius `ar`)
/// come within `ar + br` of `c`? Catches fast projectiles that jump over a
/// target between two ticks.
pub fn segment_circle_hit(a: Vec2, b: Vec2, ar: f32, c: Vec2, br: f32) -> bool {
    let ab = b - a;
    let len_sq = ab.len_sq();
    let t = if len_sq > 1e-9 {
        ((c - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = a + ab * t;
    let r = ar + br;
    closest.dist_sq(c) <= r * r
}

/// Push a circle out of an axis-aligned rectangle (if overlapping).
pub fn resolve_circle_rect(pos: Vec2, r: f32, rect: &Rectf) -> Vec2 {
    let cx = pos.x.clamp(rect.x, rect.right());
    let cy = pos.y.clamp(rect.y, rect.bottom());
    let closest = Vec2::new(cx, cy);
    let delta = pos - closest;
    let d2 = delta.len_sq();
    if d2 >= r * r {
        return pos;
    }
    if d2 > 1e-6 {
        let d = d2.sqrt();
        pos + delta * ((r - d) / d)
    } else {
        // Center is inside the rect: push out along the nearest edge.
        let left = (pos.x - rect.x).abs();
        let right = (rect.right() - pos.x).abs();
        let top = (pos.y - rect.y).abs();
        let bottom = (rect.bottom() - pos.y).abs();
        let m = left.min(right).min(top).min(bottom);
        if m == left {
            Vec2::new(rect.x - r, pos.y)
        } else if m == right {
            Vec2::new(rect.right() + r, pos.y)
        } else if m == top {
            Vec2::new(pos.x, rect.y - r)
        } else {
            Vec2::new(pos.x, rect.bottom() + r)
        }
    }
}

/// First intersection of the segment `a -> b` with `rect` expanded by `r`,
/// as a parameter in `[0, 1]` (0 = already touching/overlapping at `a`).
/// Slab/Liang-Barsky clipping so a fast projectile cannot skip a thin wall.
pub fn segment_rect_hit(a: Vec2, b: Vec2, r: f32, rect: &Rectf) -> Option<f32> {
    let min = Vec2::new(rect.x - r, rect.y - r);
    let max = Vec2::new(rect.right() + r, rect.bottom() + r);
    let d = b - a;
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    let planes = [
        (-d.x, a.x - min.x),
        (d.x, max.x - a.x),
        (-d.y, a.y - min.y),
        (d.y, max.y - a.y),
    ];
    for (p, q) in planes {
        if p.abs() < 1e-9 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return None;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    Some(t0)
}

/// Outward normal of a swept circle's entry face. The caller has already
/// found an intersection. A corner tie uses X before Y.
pub(crate) fn segment_rect_normal(from: Vec2, to: Vec2, radius: f32, rect: &Rectf) -> Vec2 {
    let delta = to - from;
    let entry_x = if delta.x > 0.0 { rect.x - radius } else { rect.right() + radius };
    let entry_y = if delta.y > 0.0 { rect.y - radius } else { rect.bottom() + radius };
    let tx = if delta.x.abs() < 1e-9 { f32::NEG_INFINITY } else { (entry_x - from.x) / delta.x };
    let ty = if delta.y.abs() < 1e-9 { f32::NEG_INFINITY } else { (entry_y - from.y) / delta.y };
    if tx >= ty {
        Vec2::new(-delta.x.signum(), 0.0)
    } else {
        Vec2::new(0.0, -delta.y.signum())
    }
}

// ---- Tile grid ------------------------------------------------------------

/// One grid cell in world pixels ([`art_walls::CELL_SIZE`]).
pub const TILE: i32 = art_walls::CELL_SIZE;
/// [`TILE`] as a float.
pub const TILE_F: f32 = art_walls::CELL_SIZE_F;

/// Radius (in cells) of the search an ejection makes when a circle's centre is
/// buried inside a wall block ([`TileMap::resolve_circle`]).
const BURIED_SEARCH_CELLS: i32 = 3;

/// The grid cell a world position is in (negative for points left of/above
/// the map, which every tile query already reads as solid).
#[inline]
pub fn cell_of(p: Vec2) -> (i32, i32) {
    (
        (p.x / TILE_F).floor() as i32,
        (p.y / TILE_F).floor() as i32,
    )
}

/// The two navigation/collision size classes.
///
/// An entity fits through a [`WallKind::RatHole`] iff its collision radius is
/// `<= art_walls::RAT_HOLE_MAX_RADIUS` (3.5 px): the rat (3.0) and the small
///swarmers (Bat 2.0, Skitterer 2.5, Spitter 3.0) are small; a Cat (4.0), a
/// Brute (6.5) and every boss (14.0) are large and must route around.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SizeClass {
    /// Fits a rat-hole. Also the player's class.
    Small,
    /// Does not fit a rat-hole.
    Large,
}

impl SizeClass {
    pub const ALL: [SizeClass; 2] = [SizeClass::Small, SizeClass::Large];

    /// Class of an entity with collision `radius`.
    #[inline]
    pub fn of_radius(radius: f32) -> SizeClass {
        if radius <= art_walls::RAT_HOLE_MAX_RADIUS {
            SizeClass::Small
        } else {
            SizeClass::Large
        }
    }

    /// Class of a spawned enemy kind.
    #[inline]
    pub fn of_enemy(kind: EnemyKind) -> SizeClass {
        SizeClass::of_radius(kind.radius())
    }

    /// Index into a per-class array (`[Small, Large]` order).
    #[inline]
    pub const fn index(self) -> usize {
        match self {
            SizeClass::Small => 0,
            SizeClass::Large => 1,
        }
    }
}

/// What one grid cell holds. Floor cells are [`TileKind::Open`]; everything
/// else keeps the art module's architectural kind so the renderer and the
/// colliders read the same value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    /// Walkable floor (or the start of a corridor, a maze aisle, a doorway).
    Open,
    /// A wall cell of the given architectural kind.
    Wall(WallKind),
}

impl TileKind {
    #[inline]
    pub const fn is_open(self) -> bool {
        matches!(self, TileKind::Open)
    }

    #[inline]
    pub const fn is_wall(self) -> bool {
        !self.is_open()
    }

    #[inline]
    pub const fn wall_kind(self) -> Option<WallKind> {
        match self {
            TileKind::Open => None,
            TileKind::Wall(kind) => Some(kind),
        }
    }

    /// May an agent of `class` occupy this cell at all? (No direction: see
    /// [`TileMap::travel_ok`] for the one-way-door rule.)
    #[inline]
    pub fn passable(self, class: SizeClass) -> bool {
        match self {
            TileKind::Open => true,
            TileKind::Wall(WallKind::Solid) | TileKind::Wall(WallKind::Maze) => false,
            TileKind::Wall(WallKind::RatHole { .. }) => class == SizeClass::Small,
            // A doorway is open in both directions for anything that fits
            // (1 cell = 14 px clear, 2 cells = 30 px clear).
            TileKind::Wall(WallKind::Doorway { .. }) => true,
            // One-way: the cell itself is reachable for the player's class
            // only; the direction gate is `travel_ok`.
            TileKind::Wall(WallKind::OneWayDoor { .. }) => class == SizeClass::Small,
        }
    }

    /// Navigation passability **for a move in `travel` direction** into this
    /// cell. Only [`WallKind::OneWayDoor`] differs from [`Self::passable`]: it
    /// is traversable by the small class along its allowed direction and
    /// never against it (and never by a large agent).
    #[inline]
    pub fn travel_ok(self, class: SizeClass, travel: Cardinal) -> bool {
        match self {
            TileKind::Wall(WallKind::OneWayDoor { dir }) => class == SizeClass::Small && dir == travel,
            _ => self.passable(class),
        }
    }
}

/// One grid cell: its kind plus the neighbour mask the wall art needs.
///
/// The mask is *derived* state, rebuilt by [`TileMap::rebuild_masks`] after a
/// generator paints cells — never maintained by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileCell {
    pub kind: TileKind,
    pub mask: NeighborMask,
}

impl Default for TileCell {
    fn default() -> Self {
        TileCell {
            kind: TileKind::Open,
            mask: NeighborMask::EMPTY,
        }
    }
}

impl TileCell {
    pub const OPEN: TileCell = TileCell {
        kind: TileKind::Open,
        mask: NeighborMask::EMPTY,
    };
}

/// Offset `(dx, dy)` of a cardinal direction (grid space, `+y` is south).
#[inline]
pub const fn cardinal_delta(dir: Cardinal) -> (i32, i32) {
    match dir {
        Cardinal::North => (0, -1),
        Cardinal::East => (1, 0),
        Cardinal::South => (0, 1),
        Cardinal::West => (-1, 0),
    }
}

/// Inverse of [`cardinal_delta`]; `None` for a zero or diagonal offset.
#[inline]
pub fn cardinal_of(dx: i32, dy: i32) -> Option<Cardinal> {
    match (dx.signum(), dy.signum()) {
        (0, -1) if dy != 0 => Some(Cardinal::North),
        (1, 0) if dx != 0 => Some(Cardinal::East),
        (0, 1) if dy != 0 => Some(Cardinal::South),
        (-1, 0) if dx != 0 => Some(Cardinal::West),
        _ => None,
    }
}

/// World-pixel unit vector along a cardinal direction.
#[inline]
pub fn cardinal_vec(dir: Cardinal) -> Vec2 {
    let (dx, dy) = cardinal_delta(dir);
    Vec2::new(dx as f32, dy as f32)
}

/// The level's wall lattice: floor/wall cells, their neighbour masks, and every
/// wall query the simulation makes.
///
/// Determinism: every query walks cells in `(row, column)` order and never
/// allocates, so results do not depend on hash iteration or heap layout.
#[derive(bevy_ecs::resource::Resource)]
pub struct TileMap {
    theme: Theme,
    /// Wall-art variant seed ([`WallCellView::seed`]).
    seed: u32,
    cols: i32,
    rows: i32,
    cells: Vec<TileCell>,
}

impl TileMap {
    /// A `cols × rows` map of open floor. Call [`Self::rebuild_masks`] after
    /// painting walls.
    pub fn new(cols: i32, rows: i32, theme: Theme, seed: u32) -> TileMap {
        assert!(cols > 0 && rows > 0, "tile map needs a positive size");
        TileMap {
            theme,
            seed,
            cols,
            rows,
            cells: vec![TileCell::OPEN; (cols * rows) as usize],
        }
    }

    #[inline]
    pub fn cols(&self) -> i32 {
        self.cols
    }

    #[inline]
    pub fn rows(&self) -> i32 {
        self.rows
    }

    #[inline]
    pub fn theme(&self) -> Theme {
        self.theme
    }

    #[inline]
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Level size in world pixels.
    #[inline]
    pub fn world_size(&self) -> Vec2 {
        Vec2::new(
            self.cols as f32 * TILE_F,
            self.rows as f32 * TILE_F,
        )
    }

    #[inline]
    pub fn in_bounds(&self, cx: i32, cy: i32) -> bool {
        cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows
    }

    #[inline]
    fn index(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    /// Kind of a cell. **Out of bounds reads as [`WallKind::Solid`]**, so no
    /// caller has to special-case the map border.
    #[inline]
    pub fn get(&self, cx: i32, cy: i32) -> TileKind {
        if !self.in_bounds(cx, cy) {
            return TileKind::Wall(WallKind::Solid);
        }
        self.cells[self.index(cx, cy)].kind
    }

    /// Kind at a world position (out of bounds = solid).
    #[inline]
    pub fn get_at(&self, p: Vec2) -> TileKind {
        self.get(
            (p.x / TILE_F).floor() as i32,
            (p.y / TILE_F).floor() as i32,
        )
    }

    /// Is the cell solid over its whole 16 × 16 footprint ([`WallKind::Solid`])?
    /// The only wall kind a rectangle list can merge into runs.
    #[inline]
    pub fn is_full_solid(&self, cx: i32, cy: i32) -> bool {
        matches!(self.get(cx, cy), TileKind::Wall(WallKind::Solid))
    }

    /// Full cell (kind + mask).
    #[inline]
    pub fn cell(&self, cx: i32, cy: i32) -> TileCell {
        if !self.in_bounds(cx, cy) {
            return TileCell {
                kind: TileKind::Wall(WallKind::Solid),
                mask: NeighborMask::ALL,
            };
        }
        self.cells[self.index(cx, cy)]
    }

    #[inline]
    pub fn mask(&self, cx: i32, cy: i32) -> NeighborMask {
        self.cell(cx, cy).mask
    }

    /// Paint one cell (out-of-bounds writes are ignored). Masks go stale: call
    /// [`Self::rebuild_masks`] once the generator is done.
    #[inline]
    pub fn set(&mut self, cx: i32, cy: i32, kind: TileKind) {
        if self.in_bounds(cx, cy) {
            let i = self.index(cx, cy);
            self.cells[i].kind = kind;
        }
    }

    /// Paint `w × h` cells from `(x0, y0)` (clipped).
    pub fn fill_cells(&mut self, x0: i32, y0: i32, w: i32, h: i32, kind: TileKind) {
        for cy in y0..y0 + h {
            for cx in x0..x0 + w {
                self.set(cx, cy, kind);
            }
        }
    }

    /// Recompute every cell's [`NeighborMask`] from its neighbours, following
    /// the generator contract in [`art_walls`]'s module docs.
    pub fn rebuild_masks(&mut self) {
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let mask = neighbour_mask(self, cx, cy);
                let i = self.index(cx, cy);
                self.cells[i].mask = mask;
            }
        }
    }

    /// Art view for one cell (static: `open_progress`/`anim_clock` at 0; the
    /// renderer fills them from the door state and the run clock).
    #[inline]
    pub fn wall_view(&self, cx: i32, cy: i32) -> WallCellView {
        let cell = self.cell(cx, cy);
        let kind = cell.kind.wall_kind().unwrap_or(WallKind::Solid);
        WallCellView::new(cx, cy, self.seed, self.theme, kind, cell.mask)
    }

    /// World-pixel rectangle of one cell.
    #[inline]
    pub fn tile_rect(cx: i32, cy: i32) -> Rectf {
        Rectf::new(
            (cx * TILE) as f32,
            (cy * TILE) as f32,
            TILE_F,
            TILE_F,
        )
    }

    /// Inclusive cell range `(x0, y0, x1, y1)` overlapping a world rectangle,
    /// clamped to the map. Returns an empty range (`x1 < x0`) when the rect is
    /// entirely outside.
    pub fn cell_range(&self, area: Rectf) -> (i32, i32, i32, i32) {
        let x0 = (area.x / TILE_F).floor() as i32;
        let y0 = (area.y / TILE_F).floor() as i32;
        let x1 = ((area.right() / TILE_F).ceil() as i32) - 1;
        let y1 = ((area.bottom() / TILE_F).ceil() as i32) - 1;
        (x0.max(0), y0.max(0), x1.min(self.cols - 1), y1.min(self.rows - 1))
    }

    /// Inclusive cell range covering a viewport at `cam`, grown by `margin`
    /// cells so tile art that overhangs its cell (shadows, overhangs) is still
    /// drawn. Used by the renderer instead of iterating the whole map.
    ///
    /// A viewport entirely outside the map yields an **empty** range
    /// (`x1 < x0` or `y1 < y0`), which every `for x in x0..=x1` loop skips.
    pub fn visible_range(&self, cam: Vec2, viewport: Vec2, margin: i32) -> (i32, i32, i32, i32) {
        let (x0, y0, x1, y1) = self.cell_range(Rectf::new(cam.x, cam.y, viewport.x, viewport.y));
        (
            (x0 - margin).max(0),
            (y0 - margin).max(0),
            (x1 + margin).min(self.cols - 1),
            (y1 + margin).min(self.rows - 1),
        )
    }

    /// Is the cell passable for `class` (undirected: the one-way-door rule is
    /// [`Self::travel_ok`])?
    #[inline]
    pub fn passable(&self, cx: i32, cy: i32, class: SizeClass) -> bool {
        self.get(cx, cy).passable(class)
    }

    /// Is a move **into** `(cx, cy)` travelling in `travel` allowed for
    /// `class`? This is the navigation edge test (one-way doors are directed).
    #[inline]
    pub fn travel_ok(&self, cx: i32, cy: i32, class: SizeClass, travel: Cardinal) -> bool {
        self.get(cx, cy).travel_ok(class, travel)
    }

    /// Allowed travel direction of a [`WallKind::OneWayDoor`] cell.
    #[inline]
    pub fn one_way_dir(&self, cx: i32, cy: i32) -> Option<Cardinal> {
        match self.get(cx, cy) {
            TileKind::Wall(WallKind::OneWayDoor { dir }) => Some(dir),
            _ => None,
        }
    }

    /// Visit the exact solid colliders (world pixels) of every cell touching
    /// `area`, in `(row, column)` order. Zero allocation.
    ///
    /// `class` selects the rat-hole rule, `player_vel` the one-way-gate rule
    /// (`Some(vel)` only for the player rat: the gate leaf swings for it when
    /// it heads along the allowed direction; `None` keeps every gate shut).
    pub fn for_each_collider_in(
        &self,
        area: Rectf,
        class: SizeClass,
        player_vel: Option<Vec2>,
        mut f: impl FnMut(Rectf),
    ) {
        let (x0, y0, x1, y1) = self.cell_range(area);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                self.emit_cell_colliders(cx, cy, class, player_vel, &mut f);
            }
        }
    }

    fn emit_cell_colliders(
        &self,
        cx: i32,
        cy: i32,
        class: SizeClass,
        player_vel: Option<Vec2>,
        f: &mut impl FnMut(Rectf),
    ) {
        let Some(kind) = self.get(cx, cy).wall_kind() else {
            return;
        };
        if matches!(kind, WallKind::RatHole { .. }) && class != SizeClass::Small {
            // Large agents see the whole cell as one wall face: bouncing off
            // the two piers would jitter instead of re-pathing.
            f(Self::tile_rect(cx, cy));
            return;
        }
        let mut out = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
        let n = art_walls::wall_colliders(&self.wall_view(cx, cy), &mut out);
        for r in out.iter().take(n) {
            f(*r);
        }
        if n == 0 && !matches!(kind, WallKind::Doorway { .. } | WallKind::OneWayDoor { .. }) {
            // A maze partition with no connected side, or any future kind: the
            // cell is a wall, so nothing is left walkable.
            f(Self::tile_rect(cx, cy));
        }
        if let WallKind::OneWayDoor { dir } = kind {
            let opens = player_vel.is_some_and(|vel| vel.dot(cardinal_vec(dir)) > 0.0);
            if !opens {
                f(art_walls::one_way_gate_rect(cx, cy, dir));
            }
        }
    }

    /// Push a circle out of every wall it overlaps, using only the cells near
    /// it. O(1): the query is a lookup in the ≤ 4 × 4 cells a circle of
    /// `radius` can reach, never a walk over the level's walls.
    ///
    /// The per-collider push is exact for a circle that grazes a wall (sliding
    /// along a face, squeezing through a doorway or a rat-hole). It cannot
    /// converge when the circle's **centre** is inside a wall block — with two
    /// flush cells the two pushes cancel and the circle stays buried — so a
    /// circle that is still overlapping after the sweep falls back to the
    /// nearest clear cell position ([`Self::resolve_buried`]). Both paths are
    /// bounded and allocation-free.
    pub fn resolve_circle(
        &self,
        pos: Vec2,
        radius: f32,
        class: SizeClass,
        player_vel: Option<Vec2>,
    ) -> Vec2 {
        let m = radius + TILE_F;
        let area = Rectf::new(pos.x - m, pos.y - m, 2.0 * m, 2.0 * m);
        let mut p = pos;
        let mut pushed = false;
        // Collect nothing: resolve as we walk, in `(row, column)` order, so the
        // result depends only on cell order (the legacy per-wall loop's
        // determinism, kept).
        let (x0, y0, x1, y1) = self.cell_range(area);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let before = p;
                self.emit_cell_colliders(cx, cy, class, player_vel, &mut |rect| {
                    p = resolve_circle_rect(p, radius, &rect);
                });
                pushed |= p != before;
            }
        }
        // No push means no collider in the window overlapped the circle (a
        // wall merely being nearby costs nothing), so only a circle that was
        // actually pushed needs the clearance re-check and the ejection.
        if !pushed || self.circle_clear_with(p, radius, class, player_vel) {
            return p;
        }
        // Centre the ejection search on the cell the circle *came from*: the
        // per-collider pushes above may have dragged it a cell deeper into the
        // block, and the nearest open floor is defined from where it was.
        self.resolve_buried(pos, radius, class, player_vel)
    }

    /// Nearest passable cell centre to `p` (bounded search); used by spawn and
    /// by knockback so an entity is never left inside a wall.
    ///
    /// Rings outwards from `p`'s cell and returns the closest centre of the
    /// first ring that has one, ties going to the lower row then the lower
    /// column. `None` when every cell within `max_cells` is a wall for `class`.
    pub fn nearest_open_center(&self, p: Vec2, class: SizeClass, max_cells: i32) -> Option<Vec2> {
        let (c0x, c0y) = cell_of(p);
        for r in 0..=max_cells.max(0) {
            let mut best: Option<(f32, i32, i32)> = None;
            for cy in (c0y - r)..=(c0y + r) {
                for cx in (c0x - r)..=(c0x + r) {
                    if r > 0 && (cx - c0x).abs() != r && (cy - c0y).abs() != r {
                        continue; // ring, not the interior (already searched)
                    }
                    if !self.passable(cx, cy, class) {
                        continue;
                    }
                    let c = Self::tile_rect(cx, cy).center();
                    let d = c.dist_sq(p);
                    if best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, cx, cy));
                    }
                }
            }
            if let Some((_, cx, cy)) = best {
                return Some(Self::tile_rect(cx, cy).center());
            }
        }
        None
    }

    /// Eject a circle whose centre is buried inside a wall block: the nearest
    /// position inside an open cell that the circle fits in.
    ///
    /// The candidate for an open cell is `p` clamped into that cell shrunk by
    /// the circle's radius, so the ejection lands flush against the face the
    /// circle came from instead of at the cell centre. The search window is
    /// [`BURIED_SEARCH_CELLS`] cells.
    ///
    /// A circle too wide for a cell (only a boss's 14 px radius) fits none of
    /// those candidates: it lands on the nearest open cell centre instead
    /// ([`Self::nearest_open_center`]), which is a best effort, not a proof —
    /// spawn checks never place a boss inside a wall in the first place. With
    /// no open cell in reach at all, the circle is returned unchanged.
    fn resolve_buried(
        &self,
        p: Vec2,
        radius: f32,
        class: SizeClass,
        player_vel: Option<Vec2>,
    ) -> Vec2 {
        let inset = radius.min(TILE_F * 0.5);
        let (c0x, c0y) = cell_of(p);
        let mut best: Option<(f32, Vec2)> = None;
        for cy in (c0y - BURIED_SEARCH_CELLS)..=(c0y + BURIED_SEARCH_CELLS) {
            for cx in (c0x - BURIED_SEARCH_CELLS)..=(c0x + BURIED_SEARCH_CELLS) {
                if !self.passable(cx, cy, class) {
                    continue;
                }
                let rect = Self::tile_rect(cx, cy);
                let cand = Vec2::new(
                    p.x.clamp(rect.x + inset, rect.right() - inset),
                    p.y.clamp(rect.y + inset, rect.bottom() - inset),
                );
                if !self.circle_clear_with(cand, radius, class, player_vel) {
                    continue;
                }
                let d = cand.dist_sq(p);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, cand));
                }
            }
        }
        if let Some((_, c)) = best {
            return c;
        }
        self.nearest_open_center(p, class, BURIED_SEARCH_CELLS)
            .unwrap_or(p)
    }

    /// Earliest hit of the segment `from -> to` (a projectile of `radius`)
    /// against a wall, as a `0..=1` parameter. One-way gate leaves do not stop
    /// projectiles; their permanent jambs do.
    pub fn segment_hit(&self, from: Vec2, to: Vec2, radius: f32) -> Option<f32> {
        self.segment_wall(from, to, radius).map(|(t, _, _)| t)
    }

    /// The first projectile contact, its outward face normal, and the struck
    /// tile. A corner tie uses the X face before the Y face.
    pub fn segment_hit_face(
        &self, from: Vec2, to: Vec2, radius: f32,
    ) -> Option<(f32, Vec2, (i32, i32))> {
        let (t, rect, tile) = self.segment_wall(from, to, radius)?;
        Some((t, segment_rect_normal(from, to, radius, &rect), tile))
    }

    #[inline]
    fn segment_wall(
        &self, from: Vec2, to: Vec2, radius: f32,
    ) -> Option<(f32, Rectf, (i32, i32))> {
        let area = Rectf::new(
            from.x.min(to.x) - radius - TILE_F,
            from.y.min(to.y) - radius - TILE_F,
            (from.x - to.x).abs() + 2.0 * (radius + TILE_F),
            (from.y - to.y).abs() + 2.0 * (radius + TILE_F),
        );
        let mut best: Option<(f32, Rectf, (i32, i32))> = None;
        let (x0, y0, x1, y1) = self.cell_range(area);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if self.get(cx, cy).is_open() {
                    continue;
                }
                let mut out = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
                let n = art_walls::wall_colliders(&self.wall_view(cx, cy), &mut out);
                for r in out.iter().take(n) {
                    if let Some(t) = segment_rect_hit(from, to, radius, r) {
                        if best.is_none_or(|(previous, _, _)| t < previous) {
                            best = Some((t, *r, (cx, cy)));
                        }
                    }
                }
            }
        }
        best
    }

    /// Is a circle of `radius` free of walls (a spawn-point check)?
    pub fn circle_clear(&self, pos: Vec2, radius: f32, class: SizeClass) -> bool {
        self.circle_clear_with(pos, radius, class, None)
    }

    /// [`Self::circle_clear`] with the one-way-door permission, so the player
    /// (and only the player) may stand inside a gate leaf it is opening.
    fn circle_clear_with(
        &self,
        pos: Vec2,
        radius: f32,
        class: SizeClass,
        player_vel: Option<Vec2>,
    ) -> bool {
        let m = radius + TILE_F;
        let area = Rectf::new(pos.x - m, pos.y - m, 2.0 * m, 2.0 * m);
        let mut clear = true;
        self.for_each_collider_in(area, class, player_vel, |rect| {
            if clear && circle_rect_overlap(pos, radius, &rect) {
                clear = false;
            }
        });
        clear
    }

    /// Is every cell touching `rect` passable for `class` and free of wall
    /// colliders (a room-spawn check)?
    pub fn rect_clear(&self, rect: Rectf, class: SizeClass) -> bool {
        let (x0, y0, x1, y1) = self.cell_range(rect);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if !self.passable(cx, cy, class) {
                    return false;
                }
            }
        }
        true
    }

    /// Iterate every non-open cell in `(row, column)` order (generator
    /// validation, tests).
    pub fn iter_walls(&self) -> impl Iterator<Item = (i32, i32, WallKind)> + '_ {
        self.cells.iter().enumerate().filter_map(|(i, cell)| {
            let kind = cell.kind.wall_kind()?;
            let i = i as i32;
            Some((i % self.cols, i / self.cols, kind))
        })
    }

    /// Number of non-open cells (tests, HUD/debug).
    pub fn wall_count(&self) -> usize {
        self.cells.iter().filter(|c| c.kind.is_wall()).count()
    }
}

/// Resolve a circle against one rectangle. Kept as a free function so callers
/// holding only a rect (a prop footprint, a test's own wall) behave exactly
/// like the tile grid.
#[inline]
pub fn circle_rect_overlap(pos: Vec2, r: f32, rect: &Rectf) -> bool {
    let cx = pos.x.clamp(rect.x, rect.right());
    let cy = pos.y.clamp(rect.y, rect.bottom());
    (pos.x - cx) * (pos.x - cx) + (pos.y - cy) * (pos.y - cy) < r * r
}

/// Art-driven neighbour mask of one cell ([`art_walls`] generator contract).
fn neighbour_mask(tiles: &TileMap, cx: i32, cy: i32) -> NeighborMask {
    let Some(kind) = tiles.get(cx, cy).wall_kind() else {
        return NeighborMask::EMPTY;
    };
    let connects: fn(TileKind) -> bool = match kind {
        WallKind::Solid | WallKind::RatHole { .. } => {
            |k| matches!(k, TileKind::Wall(WallKind::Solid | WallKind::RatHole { .. }))
        }
        WallKind::Maze => |k| {
            matches!(
                k,
                TileKind::Wall(WallKind::Solid | WallKind::RatHole { .. } | WallKind::Maze)
            )
        },
        WallKind::Doorway { .. } | WallKind::OneWayDoor { .. } => |k| {
            matches!(
                k,
                TileKind::Wall(WallKind::Solid | WallKind::RatHole { .. } | WallKind::Maze)
            )
        },
    };
    let n = connects(tiles.get(cx, cy - 1));
    let e = connects(tiles.get(cx + 1, cy));
    let s = connects(tiles.get(cx, cy + 1));
    let w = connects(tiles.get(cx - 1, cy));
    let doors = matches!(
        kind,
        WallKind::Doorway { .. } | WallKind::OneWayDoor { .. }
    );
    let (ne, se, sw, nw) = if doors {
        (false, false, false, false)
    } else {
        (
            connects(tiles.get(cx + 1, cy - 1)),
            connects(tiles.get(cx + 1, cy + 1)),
            connects(tiles.get(cx - 1, cy + 1)),
            connects(tiles.get(cx - 1, cy - 1)),
        )
    };
    NeighborMask::from_all(n, e, s, w, ne, se, sw, nw)
}

/// Uniform grid over enemy positions. The world keeps one and reuses it:
/// `reset` clears buckets in place (keeping their capacity) and `fill`
/// re-populates them, so a frame does no bucket allocation.
#[derive(bevy_ecs::resource::Resource)]
pub struct SpatialGrid {
    cell: f32,
    cols: i32,
    rows: i32,
    buckets: Vec<Vec<u32>>,
}

impl Default for SpatialGrid {
    fn default() -> Self {
        SpatialGrid {
            cell: 16.0,
            cols: 0,
            rows: 0,
            buckets: Vec::new(),
        }
    }
}

impl SpatialGrid {
    pub fn new(arena: Vec2, cell: f32) -> Self {
        let mut g = SpatialGrid {
            cell,
            cols: 0,
            rows: 0,
            buckets: Vec::new(),
        };
        g.reset(arena, cell);
        g
    }

    /// Convenience constructor: allocate a fresh grid and fill it.
    pub fn build(positions: impl Iterator<Item = Vec2>, arena: Vec2, cell: f32) -> Self {
        let mut g = SpatialGrid::new(arena, cell);
        g.fill(positions);
        g
    }

    /// Resize to an arena and clear every bucket, keeping bucket allocations
    /// when the arena/cell size is unchanged (the common case each frame).
    pub fn reset(&mut self, arena: Vec2, cell: f32) {
        self.cell = cell;
        self.cols = (arena.x / cell).ceil() as i32 + 1;
        self.rows = (arena.y / cell).ceil() as i32 + 1;
        let n = (self.cols * self.rows) as usize;
        if self.buckets.len() == n {
            for b in &mut self.buckets {
                b.clear();
            }
        } else {
            self.buckets.clear();
            self.buckets.resize(n, Vec::new());
        }
    }

    /// Insert entity indices by position. Call after `reset`.
    pub fn fill(&mut self, positions: impl Iterator<Item = Vec2>) {
        for (i, p) in positions.enumerate() {
            let cx = (p.x / self.cell) as i32;
            let cy = (p.y / self.cell) as i32;
            if cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows {
                self.buckets[(cy * self.cols + cx) as usize].push(i as u32);
            }
        }
    }

    /// Indices of entities in the 3×3 block of cells around `p`.
    pub fn neighbors(&self, p: Vec2, out: &mut Vec<u32>) {
        self.neighbors_within(p, self.cell, out);
    }

    /// Indices of entities in every cell within `reach` pixels of `p`.
    /// `reach` must cover the query circle plus the largest target radius,
    /// otherwise an overlap across a cell boundary is missed.
    pub fn neighbors_within(&self, p: Vec2, reach: f32, out: &mut Vec<u32>) {
        out.clear();
        let span = (reach / self.cell).ceil().max(1.0) as i32;
        let cx = (p.x / self.cell) as i32;
        let cy = (p.y / self.cell) as i32;
        for ny in (cy - span)..=(cy + span) {
            for nx in (cx - span)..=(cx + span) {
                if nx >= 0 && ny >= 0 && nx < self.cols && ny < self.rows {
                    out.extend_from_slice(&self.buckets[(ny * self.cols + nx) as usize]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_basic() {
        assert!(circles_overlap(Vec2::ZERO, 2.0, Vec2::new(3.0, 0.0), 2.0));
        assert!(!circles_overlap(Vec2::ZERO, 1.0, Vec2::new(5.0, 0.0), 1.0));
    }

    #[test]
    fn swept_circle_catches_a_jumped_target() {
        let a = Vec2::new(80.0, 100.0);
        let b = Vec2::new(92.0, 100.0);
        let target = Vec2::new(86.0, 100.0);
        // Both endpoints are outside the 4.5 px hit radius, but the path hits.
        assert!(!circles_overlap(a, 2.0, target, 2.5));
        assert!(!circles_overlap(b, 2.0, target, 2.5));
        assert!(segment_circle_hit(a, b, 2.0, target, 2.5));
        assert!(!segment_circle_hit(a, b, 2.0, Vec2::new(86.0, 120.0), 2.5));
    }

    #[test]
    fn segment_rect_hit_catches_a_thin_wall() {
        let wall = Rectf::new(20.0, -10.0, 1.0, 20.0);
        // A 12 px step that lands beyond the thin wall still reports a hit on
        // the near face (expanded by the bullet radius).
        let t = segment_rect_hit(Vec2::new(10.0, 0.0), Vec2::new(22.0, 0.0), 1.0, &wall)
            .expect("segment should hit the wall");
        assert!((t - 0.75).abs() < 1e-3, "hit t was {t}");
        assert!(segment_rect_hit(Vec2::new(10.0, 50.0), Vec2::new(22.0, 50.0), 1.0, &wall).is_none());
    }

    #[test]
    fn resolve_pushes_out() {
        let rect = Rectf::new(0.0, 0.0, 10.0, 10.0);
        let p = Vec2::new(-1.0, 5.0); // just left of the rect, radius overlaps
        let fixed = resolve_circle_rect(p, 3.0, &rect);
        assert!(fixed.x <= -3.0 + 0.01);
    }

    #[test]
    fn grid_finds_neighbors() {
        let pts = [
            Vec2::new(5.0, 5.0),
            Vec2::new(7.0, 6.0),
            Vec2::new(200.0, 200.0),
        ];
        let grid = SpatialGrid::build(pts.iter().copied(), Vec2::new(256.0, 256.0), 16.0);
        let mut out = Vec::new();
        grid.neighbors(Vec2::new(6.0, 6.0), &mut out);
        assert!(out.contains(&0) && out.contains(&1));
        assert!(!out.contains(&2));
    }

    #[test]
    fn reset_clears_but_keeps_bucket_capacity() {
        let arena = Vec2::new(256.0, 256.0);
        let mut grid = SpatialGrid::new(arena, 16.0);
        grid.fill(std::iter::once(Vec2::new(5.0, 5.0)));
        let cap = grid.buckets[0].capacity();
        assert!(cap > 0);
        grid.reset(arena, 16.0);
        assert_eq!(grid.buckets.len(), 17 * 17);
        assert!(grid.buckets.iter().all(|b| b.is_empty()));
        assert_eq!(grid.buckets[0].capacity(), cap, "bucket allocation was dropped");
    }

    // ---- tile grid -------------------------------------------------------

    use super::super::art_walls::WallAxis;

    /// A 10 x 10-cell map with a solid block and one cell of each wall kind.
    fn test_map() -> TileMap {
        let mut t = TileMap::new(10, 10, Theme::Sewer, 7);
        t.fill_cells(3, 3, 2, 2, TileKind::Wall(WallKind::Solid));
        t.set(
            5,
            5,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Horizontal,
            }),
        );
        t.set(
            7,
            5,
            TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::East,
            }),
        );
        t.fill_cells(0, 8, 10, 1, TileKind::Wall(WallKind::Solid));
        t.rebuild_masks();
        t
    }

    #[test]
    fn out_of_bounds_reads_as_solid() {
        let t = TileMap::new(4, 4, Theme::Sewer, 0);
        assert!(t.get(-1, 0).is_wall());
        assert!(t.get(0, 4).is_wall());
        assert!(!t.passable(-1, 0, SizeClass::Small));
        assert_eq!(t.world_size(), Vec2::new(64.0, 64.0));
    }

    #[test]
    fn a_circle_is_pushed_out_of_a_solid_cell() {
        let t = test_map();
        // Centre of cell (3,3): pushed out to a face, never left inside.
        let p = t.resolve_circle(Vec2::new(56.0, 56.0), 3.0, SizeClass::Small, None);
        assert!(
            !circle_rect_overlap(p, 3.0, &TileMap::tile_rect(3, 3)),
            "still overlapping the solid cell at {p:?}"
        );
    }

    #[test]
    fn a_rat_hole_blocks_the_large_class_only() {
        let t = test_map();
        let center = TileMap::tile_rect(5, 5).center();
        assert!(t.passable(5, 5, SizeClass::Small));
        assert!(!t.passable(5, 5, SizeClass::Large));

        // Walk a small circle across the hole, cell by cell: it gets through.
        let mut p = Vec2::new(center.x, center.y - 24.0);
        for _ in 0..200 {
            p.y += 0.5;
            p = t.resolve_circle(p, 3.0, SizeClass::Small, None);
        }
        assert!(
            p.y > center.y + 16.0,
            "the rat was stopped at {p:?} (hole centre {center:?})"
        );

        // A large one is stopped: pushing south from above leaves it north.
        let mut q = Vec2::new(center.x, center.y - 24.0);
        for _ in 0..200 {
            q.y += 0.5;
            q = t.resolve_circle(q, 6.5, SizeClass::Large, None);
        }
        assert!(
            q.y < center.y - 6.0,
            "a large agent passed a rat-hole at {q:?}"
        );
    }

    #[test]
    fn a_one_way_gate_needs_the_allowed_direction() {
        let t = test_map();
        let center = TileMap::tile_rect(7, 5).center();
        assert_eq!(t.one_way_dir(7, 5), Some(Cardinal::East));
        assert!(t.travel_ok(7, 5, SizeClass::Small, Cardinal::East));
        assert!(!t.travel_ok(7, 5, SizeClass::Small, Cardinal::West));
        assert!(!t.travel_ok(7, 5, SizeClass::Large, Cardinal::East));

        // Walk east through the cell. Only the player (a velocity) may pass.
        let east = cardinal_vec(Cardinal::East);
        let walk = |player_vel: Option<Vec2>| {
            let mut p = Vec2::new(center.x - 24.0, center.y);
            for _ in 0..200 {
                p.x += 0.5;
                p = t.resolve_circle(p, 3.0, SizeClass::Small, player_vel);
            }
            p
        };
        let gate = art_walls::one_way_gate_rect(7, 5, Cardinal::East);
        let through = walk(Some(east));
        assert!(
            through.x > center.x + 16.0,
            "the player was refused the allowed direction at {through:?}"
        );
        let blocked = walk(None);
        assert!(
            blocked.x < gate.x - 1.0,
            "a non-player slipped through a one-way gate: {blocked:?}"
        );
        // Pushing west (the wrong side) keeps it shut.
        let west = cardinal_vec(Cardinal::West);
        let refused = walk(Some(west));
        assert!(
            refused.x < gate.x - 1.0,
            "the gate opened against its direction: {refused:?}"
        );
    }

    #[test]
    fn segment_hit_finds_the_near_face() {
        let t = test_map();
        let hit = t
            .segment_hit(Vec2::new(56.0, 30.0), Vec2::new(56.0, 90.0), 1.0)
            .expect("the block should be hit");
        // The block's top face is at y = 48; a radius-1 shot stops at 47.
        assert!((48.0 - (30.0 + 60.0 * hit)).abs() < 2.0, "t = {hit}");
        assert!(
            t.segment_hit(Vec2::new(80.0, 20.0), Vec2::new(80.0, 40.0), 1.0)
                .is_none(),
            "empty floor reported a wall hit"
        );
    }

    #[test]
    fn neighbour_masks_follow_the_art_contract() {
        let mut t = TileMap::new(6, 6, Theme::Sewer, 0);
        t.fill_cells(1, 1, 3, 1, TileKind::Wall(WallKind::Solid));
        // A doorway between two wall stubs: jambs on the walled sides only.
        t.set(2, 1, TileKind::Wall(WallKind::Doorway {
            axis: WallAxis::Horizontal,
        }));
        t.rebuild_masks();
        let m = t.mask(2, 1);
        assert!(m.has(NeighborMask::W) && m.has(NeighborMask::E));
        assert!(!m.has(NeighborMask::N) && !m.has(NeighborMask::S));
        // Solid cells connect to solid neighbours (and not to doorways).
        let solid = t.mask(1, 1);
        assert!(!solid.has(NeighborMask::N), "solid->open must not link");
        assert!(!solid.has(NeighborMask::E), "solid->doorway must not link");
    }

    /// Is a circle of `r` at `p` overlapping any wall collider for `class`?
    fn overlaps_wall(t: &TileMap, p: Vec2, r: f32, class: SizeClass) -> bool {
        let mut hit = false;
        let m = r + TILE_F;
        t.for_each_collider_in(
            Rectf::new(p.x - m, p.y - m, 2.0 * m, 2.0 * m),
            class,
            None,
            |rect| {
                if circle_rect_overlap(p, r, &rect) {
                    hit = true;
                }
            },
        );
        hit
    }

    /// A circle buried in the middle of a two-cell-thick wall must come out on
    /// the *outside* of the slab. Resolving against each flush cell in turn
    /// pushes it into the neighbouring cell, so the tile grid's ejection is a
    /// bounded nearest-clear search, not a single pass.
    #[test]
    fn a_circle_inside_a_two_cell_wall_is_ejected_to_the_outside() {
        let mut t = TileMap::new(16, 16, Theme::Sewer, 3);
        // A 2 x 6-cell slab, the shape of every room wall on a generated map.
        t.fill_cells(6, 5, 2, 6, TileKind::Wall(WallKind::Solid));
        t.rebuild_masks();
        let slab = Rectf::new(6.0 * TILE_F, 5.0 * TILE_F, 2.0 * TILE_F, 6.0 * TILE_F);

        for (dx, dy) in [(0.5f32, 0.5f32), (0.3, 0.7), (0.5, 0.2), (0.2, 0.5)] {
            // The shared edge of the two columns, at various heights.
            let p = Vec2::new(
                slab.x + slab.w * dx,
                slab.y + slab.h * dy,
            );
            assert!(
                overlaps_wall(&t, p, 3.0, SizeClass::Small),
                "the probe {p:?} should start buried"
            );
            let out = t.resolve_circle(p, 3.0, SizeClass::Small, None);
            assert!(
                !overlaps_wall(&t, out, 3.0, SizeClass::Small),
                "a buried circle was left inside the slab at {out:?} (from {p:?})"
            );
            assert!(
                !slab.contains(out),
                "the ejection stayed inside the slab: {out:?} (from {p:?})"
            );
        }

        // The same for a brute-sized circle: it must clear the slab's faces.
        let p = Vec2::new(slab.x + slab.w * 0.5, slab.y + slab.h * 0.5);
        let out = t.resolve_circle(p, 6.5, SizeClass::Large, None);
        assert!(
            !overlaps_wall(&t, out, 6.5, SizeClass::Large),
            "a large circle was left inside the slab at {out:?}"
        );
    }

    /// Open-floor cell centres of a map, up to `limit` of them (deterministic:
    /// `(row, column)` scan order).
    fn open_floor_probes(t: &TileMap, limit: usize) -> Vec<Vec2> {
        let mut out = Vec::with_capacity(limit);
        for cy in 0..t.rows() {
            for cx in 0..t.cols() {
                if t.passable(cx, cy, SizeClass::Small) {
                    out.push(TileMap::tile_rect(cx, cy).center());
                    if out.len() == limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// A wall query costs the same on a 200 x 160 map (thousands of walls) as
    /// on a 16 x 16 one: it is a lookup in the cells near the circle, never a
    /// walk over the level's walls. This is the regression test for the
    /// per-wall loops that used to live in the player, enemy, bullet and spawn
    /// systems.
    #[test]
    fn wall_queries_do_not_scale_with_the_map() {
        use std::time::Instant;

        // A 200 x 160 map with a wall lattice: ~10 000 wall cells, the scale of
        // a generated level.
        let mut big = TileMap::new(200, 160, Theme::Sewer, 1);
        let mut cy = 2;
        while cy < 156 {
            big.fill_cells(2, cy, 196, 2, TileKind::Wall(WallKind::Solid));
            cy += 6;
        }
        big.rebuild_masks();
        let mut tiny = TileMap::new(16, 16, Theme::Sewer, 1);
        tiny.fill_cells(3, 3, 2, 6, TileKind::Wall(WallKind::Solid));
        tiny.rebuild_masks();

        let big_probes = open_floor_probes(&big, 512);
        let tiny_probes = open_floor_probes(&tiny, 512);
        assert!(big_probes.len() >= 256 && tiny_probes.len() >= 64);
        assert!(big.wall_count() > 1000, "the map should have many walls");

        let time = |t: &TileMap, probes: &[Vec2]| {
            let start = Instant::now();
            let mut acc = 0.0;
            for _ in 0..8 {
                for p in probes {
                    let q = t.resolve_circle(*p, 3.0, SizeClass::Small, None);
                    acc += q.x;
                }
            }
            std::hint::black_box(acc);
            start.elapsed().as_secs_f64()
        };
        // Warm up both maps, then take the best of three runs each.
        time(&tiny, &tiny_probes);
        time(&big, &big_probes);
        let mut a = f64::INFINITY;
        let mut b = f64::INFINITY;
        for _ in 0..3 {
            a = a.min(time(&tiny, &tiny_probes));
            b = b.min(time(&big, &big_probes));
        }
        eprintln!(
            "wall query scaling: 16x16 map {:.2} ms, 200x160 map {:.2} ms for {} probes",
            a * 1e3,
            b * 1e3,
            big_probes.len()
        );
        assert!(
            b < a * 4.0 + 20e-3,
            "resolving a few thousand circles took {:.2} ms on a 16x16 map and {:.2} ms \
             on the 200x160 map: the query is walking the level's walls",
            a * 1e3,
            b * 1e3
        );
    }

    #[test]
    fn nearest_open_center_finds_the_closest_floor() {
        let mut t = TileMap::new(12, 12, Theme::Sewer, 0);
        t.fill_cells(4, 4, 4, 4, TileKind::Wall(WallKind::Solid));
        t.rebuild_masks();
        // From the middle of the block the nearest centre is one cell out.
        let c = t.nearest_open_center(TileMap::tile_rect(6, 6).center(), SizeClass::Small, 4);
        let c = c.expect("an open cell within four cells");
        let (cx, cy) = cell_of(c);
        assert!(t.passable(cx, cy, SizeClass::Small));
        assert!(!(4..8).contains(&cx) || !(4..8).contains(&cy), "still in the block");
        // A hole the large class cannot use is not "open" for it.
        t.set(2, 2, TileKind::Wall(WallKind::RatHole {
            axis: WallAxis::Horizontal,
        }));
        t.rebuild_masks();
        assert!(t.nearest_open_center(TileMap::tile_rect(2, 2).center(), SizeClass::Small, 3).is_some());
        // Nothing within zero cells of a wall cell: `None`, never a panic.
        assert_eq!(
            t.nearest_open_center(TileMap::tile_rect(5, 5).center(), SizeClass::Large, 0),
            None
        );
    }

    #[test]
    fn visible_range_is_clamped_and_covers_the_view() {
        let t = TileMap::new(20, 10, Theme::Sewer, 0);
        assert_eq!(
            t.visible_range(Vec2::ZERO, Vec2::new(32.0, 16.0), 0),
            (0, 0, 1, 0)
        );
        assert_eq!(
            t.visible_range(Vec2::new(16.0, 16.0), Vec2::new(16.0, 16.0), 1),
            (0, 0, 2, 2)
        );
        // A viewport entirely outside the map yields an empty range.
        let (ox0, _, ox1, _) = t.visible_range(Vec2::new(-400.0, -400.0), Vec2::splat(32.0), 0);
        assert!(ox1 < ox0 || ox1 < 0, "offscreen range was {ox0}..{ox1}");
        let (x0, _, x1, _) = t.visible_range(Vec2::new(1000.0, 0.0), Vec2::new(32.0, 32.0), 0);
        assert!(x1 <= 19, "the range must clamp to the last column");
        assert!(x1 < x0, "a viewport beyond the map must be empty: {x0}..{x1}");
    }

    #[test]
    fn wide_query_covers_more_than_one_cell() {
        let pts = [Vec2::new(15.9, 20.0), Vec2::new(32.5, 20.0)];
        let grid = SpatialGrid::build(pts.iter().copied(), Vec2::new(256.0, 256.0), 16.0);
        let mut out = Vec::new();
        grid.neighbors(Vec2::new(15.9, 20.0), &mut out);
        assert!(!out.contains(&1), "3x3 query should not reach two cells away");
        grid.neighbors_within(Vec2::new(15.9, 20.0), 17.0, &mut out);
        assert!(out.contains(&1), "wide query should reach two cells away");
    }
}
