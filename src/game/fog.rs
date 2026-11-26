//! Fog of war: the set of map cells the player has explored.
//!
//! The minimap draws only what the rat has actually seen, so a maze stays a
//! blank region until the player walks it. The state is a [`Fog`] resource that
//! is round-tripped through the [`crate::game::world::World`] facade exactly
//! like `Player`/`Level`/`Particles` (see `game::ecs::resources`), and it is
//! revealed by `game::ecs::systems::fog::reveal_system`, which runs directly
//! after the player system.
//!
//! ## The reveal rule
//!
//! When the player's *cell* changes, the fog floods outward from that cell:
//!
//! * through [`TileKind::Open`] floor and open [`WallKind::Doorway`] cells,
//!   eight-connected, up to [`FOG_REVEAL_RADIUS`] steps and within that radius
//!   in Euclidean cell distance;
//! * **not** through solid walls, [`WallKind::Maze`] partitions or a closed
//!   [`WallKind::OneWayDoor`] (one whose leaf has not swung open);
//! * the flood marks every wall cell bordering a revealed floor cell, so rooms
//!   and corridors get an outline instead of being invisible;
//! * a [`WallKind::RatHole`] stops the flood: seeing its near face only
//!   outlines the hole. Its far mouth is revealed by **sight or crossing** —
//!   once the rat has stood in the hole, a tiny glance flood
//!   (`FOG_GLANCE_RADIUS` cells) through it keeps working, so a known shortcut
//!   keeps showing the foothold on the other side (the designer's rule: never
//!   paint ground the player has not seen or been on).
//!
//! Diagonal steps follow the nav grid's rule — both orthogonal neighbours must
//! be open — so the flood cannot leak around a one-cell wall corner.
//!
//! ## Determinism
//!
//! Nothing here touches the rng or any simulated value, and the flood uses
//! fixed neighbour order, a reused queue and stamp-based scratch arrays, so it
//! neither allocates per tick nor perturbs a run.
//! [`Fog::reveal_enabled`] exists only so the fog tests can play the same seed
//! with the reveal switched off and prove nothing else moved.

use bevy_ecs::resource::Resource;

use crate::game::art_walls::WallKind;
use crate::game::collision::{cell_of, TileKind, TileMap};
use crate::game::ecs::systems::powerups::{HeadlampBeam, HEADLAMP_RANGE};
use crate::game::level::{Level, TileRect};
use crate::math::Vec2;

/// Flood radius, in cells, of one reveal (≈ 9 cells ≈ 144 px of explored
/// floor in every direction).
pub const FOG_REVEAL_RADIUS: i32 = 9;

/// How far a [`WallKind::RatHole`] lets the player glimpse past itself.
pub const FOG_GLANCE_RADIUS: i32 = 2;

/// Fade time of the "seen recently" glow, in ticks (2 s at 60 FPS).
pub const FOG_FADE_TICKS: u32 = 120;

/// A reveal can only touch a handful of holes; the buffer is a hard bound so
/// the flood never allocates.
const MAX_HOLES: usize = 32;

/// Rat-holes the rat can cross in one run. A generated map has a few dozen at
/// most; the cap keeps the crossed-hole list pool-free.
const MAX_CROSSED_HOLES: usize = 64;

/// The eight neighbour offsets, in a fixed order (determinism).
const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// The explored-cell set of one run, plus the scratch the flood needs.
///
/// `seen` is a bitset over the level's `cols × rows` cells (the authoritative
/// "has the player been here"), `seen_at` is the tick-stamp each cell was last
/// revealed at (the minimap's optional "seen recently" fade), and the rest is
/// reused flood scratch so a reveal allocates nothing.
#[derive(Resource)]
pub struct Fog {
    cols: i32,
    rows: i32,
    /// One bit per map cell: explored (or a wall bordering explored floor).
    seen: Vec<u64>,
    /// Reveal clock stamp of each cell (`0` = never seen).
    seen_at: Vec<u32>,
    /// Number of set bits in `seen` (the facade/minimap read it in O(1)).
    seen_count: usize,
    /// Flood queue of cell indices (FIFO from `head`), reused every reveal.
    queue: Vec<u32>,
    /// BFS depth per cell, valid only where `mark[i] == stamp`.
    depth: Vec<u8>,
    /// Generation stamp per cell, so the scratch arrays never need clearing.
    mark: Vec<u32>,
    /// Rat-hole cells the current reveal found, glanced through afterwards.
    holes: [(i32, i32); MAX_HOLES],
    holes_len: usize,
    /// Rat-hole cells the rat has crossed (its centre has been inside them).
    /// Only these holes keep a glimpse of their far mouth.
    crossed: Vec<(i32, i32)>,
    /// Per-maze discovery flags, indexed like `Level::mazes`.
    maze_entrance_seen: Vec<bool>,
    maze_claimed: Vec<bool>,
    /// Reveal generation counter (stamps `mark`).
    stamp: u32,
    /// Fade clock, advanced once per tick by the reveal system.
    clock: u32,
    /// Clock stamp of the most recent flood: cells stamped with it are the
    /// rat's *current* sight bubble (the minimap's live-vs-memory shading).
    view_clock: u32,
    /// Player cell the last reveal ran for; `(-1, -1)` before the first one.
    cell: (i32, i32),
    /// Which one-way doors were see-through at the last reveal (bit `i` =
    /// `LevelObjects::one_way_doors[i]`, the first 64). A leaf that swings
    /// open or shut while the rat stands still changes what it can see, so a
    /// change here reruns the flood without a cell change.
    door_sig: u64,
    /// Determinism hook: `false` makes the reveal system a no-op, which
    /// `systems::fog::tests::fog_never_moves_the_simulation` uses to prove the
    /// fog cannot move the simulation.
    pub reveal_enabled: bool,
}

impl Fog {
    pub fn new(cols: i32, rows: i32) -> Fog {
        Fog::with_mazes(cols, rows, 0)
    }

    /// Blank fog for a map with `mazes` maze sections (the per-maze discovery
    /// flags are sized up front, so a reveal never allocates).
    pub fn with_mazes(cols: i32, rows: i32, mazes: usize) -> Fog {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let cells = (cols * rows) as usize;
        Fog {
            cols,
            rows,
            seen: vec![0u64; cells.div_ceil(64)],
            seen_at: vec![0u32; cells],
            seen_count: 0,
            queue: Vec::with_capacity(cells.min(1024)),
            depth: vec![0u8; cells],
            mark: vec![0u32; cells],
            holes: [(0, 0); MAX_HOLES],
            holes_len: 0,
            crossed: Vec::new(),
            maze_entrance_seen: vec![false; mazes],
            maze_claimed: vec![false; mazes],
            stamp: 0,
            // Starts at 1 so `seen_at` (written as `clock`) is never 0 while a
            // cell is seen: `recency` is then `0` only for an unexplored cell
            // or a genuinely faded one.
            clock: 1,
            view_clock: 0,
            cell: (-1, -1),
            door_sig: 0,
            reveal_enabled: true,
        }
    }

    /// Blank fog of the same shape and maze count (facade round-trip spare).
    pub fn empty_like(&self) -> Fog {
        Fog::with_mazes(self.cols, self.rows, self.maze_entrance_seen.len())
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
    pub fn in_bounds(&self, cx: i32, cy: i32) -> bool {
        cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows
    }

    #[inline]
    fn index(&self, cx: i32, cy: i32) -> Option<usize> {
        if self.in_bounds(cx, cy) {
            Some((cy * self.cols + cx) as usize)
        } else {
            None
        }
    }

    /// Has the player explored `(cx, cy)` (walls bordering explored floor
    /// included)? Out-of-bounds cells are never seen.
    #[inline]
    pub fn is_seen(&self, cx: i32, cy: i32) -> bool {
        match self.index(cx, cy) {
            Some(i) => self.seen[i >> 6] & (1u64 << (i & 63)) != 0,
            None => false,
        }
    }

    /// Number of explored cells (floor and wall outlines).
    #[inline]
    pub fn seen_count(&self) -> usize {
        self.seen_count
    }

    /// Cell the last reveal ran for (`(-1, -1)` before the first one).
    #[inline]
    pub fn last_cell(&self) -> (i32, i32) {
        self.cell
    }

    /// Fade clock (ticks since the run started).
    #[inline]
    pub fn clock(&self) -> u32 {
        self.clock
    }

    /// Has any cell of `rect` been explored? Used for the "you have seen the
    /// boss arena" marker; allocation-free and short-circuits.
    pub fn any_seen_in(&self, rect: TileRect) -> bool {
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = rect.right().min(self.cols);
        let y1 = rect.bottom().min(self.rows);
        let mut cy = y0;
        while cy < y1 {
            let mut cx = x0;
            while cx < x1 {
                if self.is_seen(cx, cy) {
                    return true;
                }
                cx += 1;
            }
            cy += 1;
        }
        false
    }

    /// Has the maze's entrance been seen? (`index` is the position in
    /// `Level::mazes`; out of range reads as `false`.)
    #[inline]
    pub fn maze_entrance_seen(&self, index: usize) -> bool {
        self.maze_entrance_seen.get(index).copied().unwrap_or(false)
    }

    /// Has the maze's cache been claimed? The reward runtime is a later phase;
    /// it drives this through [`Self::set_maze_claimed`]. Until then it is
    /// `false` (or whatever that phase has recorded).
    #[inline]
    pub fn maze_claimed(&self, index: usize) -> bool {
        self.maze_claimed.get(index).copied().unwrap_or(false)
    }

    /// Mark a maze cache claimed (or not). Intended for the maze-reward system
    /// (`LevelObjects::rewards`) once it exists.
    pub fn set_maze_claimed(&mut self, index: usize, claimed: bool) {
        if let Some(slot) = self.maze_claimed.get_mut(index) {
            *slot = claimed;
        }
    }

    /// Number of maze sections this fog knows about.
    #[inline]
    pub fn maze_count(&self) -> usize {
        self.maze_entrance_seen.len()
    }

    /// Is `(cx, cy)` inside the rat's current sight bubble (revealed by the
    /// most recent flood)? Unseen cells are never "in view".
    #[inline]
    pub fn in_view(&self, cx: i32, cy: i32) -> bool {
        if self.view_clock == 0 {
            return false;
        }
        match self.index(cx, cy) {
            Some(i) => {
                self.seen[i >> 6] & (1u64 << (i & 63)) != 0 && self.seen_at[i] == self.view_clock
            }
            None => false,
        }
    }

    /// Has the rat crossed this rat-hole cell (stood inside it)?
    #[inline]
    pub fn hole_crossed(&self, cell: (i32, i32)) -> bool {
        self.crossed.contains(&cell)
    }

    /// How many rat-holes the rat has crossed.
    #[inline]
    pub fn crossed_holes(&self) -> usize {
        self.crossed.len()
    }

    /// `1.0` while the cell is inside the rat's current sight bubble, fading to
    /// `0.0` after [`FOG_FADE_TICKS`] once the rat has walked away. `0.0` also
    /// for unexplored cells.
    pub fn recency(&self, cx: i32, cy: i32) -> f32 {
        if self.in_view(cx, cy) {
            return 1.0;
        }
        let Some(i) = self.index(cx, cy) else {
            return 0.0;
        };
        if self.seen[i >> 6] & (1u64 << (i & 63)) == 0 {
            return 0.0;
        }
        let age = self.clock.wrapping_sub(self.seen_at[i]);
        if age >= FOG_FADE_TICKS {
            0.0
        } else {
            1.0 - age as f32 / FOG_FADE_TICKS as f32
        }
    }

    /// Advance the fade clock. Called once per tick by the reveal system; it
    /// touches no simulated value.
    #[inline]
    pub fn tick(&mut self) {
        self.clock = self.clock.wrapping_add(1);
        if self.clock == 0 {
            // Wrap: rebase every stamp so `recency` stays meaningful.
            self.seen_at.fill(1);
            self.clock = 1;
        }
    }

    /// Reveal from a world position, but only when the player's cell changed
    /// or a one-way door swung past the see-through threshold (otherwise a rat
    /// waiting in front of an opening leaf kept a blank far side until it
    /// stepped into the doorway). Returns `true` when the flood ran.
    pub fn update_from(&mut self, level: &Level, pos: Vec2) -> bool {
        let cell = cell_of(pos);
        let door_sig = Self::door_signature(level);
        if cell == self.cell && door_sig == self.door_sig {
            return false;
        }
        self.cell = cell;
        self.door_sig = door_sig;
        self.reveal_cell(level, cell);
        true
    }

    /// Flood from `cell` and return how many *new* cells became explored.
    ///
    /// This is the public reveal entry point (tests and the system both use
    /// it); it ignores [`Fog::last_cell`] bookkeeping.
    pub fn reveal_cell(&mut self, level: &Level, cell: (i32, i32)) -> u32 {
        self.sync_mazes(level.mazes.len());
        // Standing inside a rat-hole crosses it: from then on its far mouth is
        // known ground. The view from *inside* the hole is only a peek — the
        // hole is a wall pierced by an 8 px gap, not a window — so the flood
        // gets the glance radius until the rat is through.
        let in_hole = matches!(
            level.tiles.get(cell.0, cell.1),
            TileKind::Wall(WallKind::RatHole { .. })
        );
        if in_hole {
            self.note_crossed(cell);
        }
        let radius = if in_hole {
            FOG_GLANCE_RADIUS
        } else {
            FOG_REVEAL_RADIUS
        };
        let mut fresh = self.flood(level, cell, radius);
        // Peek through the rat-holes the flood ran into — but only the ones the
        // rat has already crossed (sight or crossing reveals a far mouth, never
        // a guess). Copied out because the glance floods reuse the buffer.
        let mut holes = [(0i32, 0i32); MAX_HOLES];
        let n = self.holes_len;
        holes[..n].copy_from_slice(&self.holes[..n]);
        for &hole in holes.iter().take(n) {
            if self.hole_crossed(hole) {
                fresh += self.flood(level, hole, FOG_GLANCE_RADIUS);
            }
        }
        self.view_clock = self.clock;
        // A maze entrance is "found" the first time its breach is on screen;
        // the marker then persists (see `World::maze_markers`).
        self.note_seen_entrances(level);
        fresh
    }

    /// Sweep the lamp even when the rat turns without changing cells. Keep
    /// the ordinary flood's live stamps so the added light can fade separately.
    pub(crate) fn reveal_headlamp(&mut self, level: &Level, beam: HeadlampBeam) {
        let reach = Vec2::splat(HEADLAMP_RANGE);
        let (x0, y0) = cell_of(beam.origin - reach);
        let (x1, y1) = cell_of(beam.origin + reach);
        for y in y0.max(0)..=y1.min(self.rows - 1) {
            for x in x0.max(0)..=x1.min(self.cols - 1) {
                if !self.in_view(x, y) && beam.reaches(level, TileMap::tile_rect(x, y).center()) {
                    let i = (y * self.cols + x) as usize;
                    self.mark_seen(i);
                    // Do not add a beam-only cell to the standing flood on
                    // a tick when the two reveals share a clock stamp.
                    if self.clock == self.view_clock {
                        self.seen_at[i] = self.clock.saturating_sub(1);
                    }
                }
            }
        }
        self.note_seen_entrances(level);
    }

    fn note_seen_entrances(&mut self, level: &Level) {
        for (i, maze) in level.mazes.iter().enumerate() {
            if !self.maze_entrance_seen[i] && self.is_seen(maze.entry.0, maze.entry.1) {
                self.maze_entrance_seen[i] = true;
            }
        }
    }

    /// Console `reveal`: mark every cell explored so the whole map shows.
    /// One pass over the bitset, no flood fill. Returns the cells newly seen.
    pub fn reveal_all(&mut self, _level: &Level) -> u32 {
        let total = (self.cols * self.rows) as usize;
        let words = total.div_ceil(64);
        let mut fresh = 0usize;
        #[allow(clippy::needless_range_loop)]
        for w in 0..words {
            let remaining = total.saturating_sub(w * 64);
            let full = if remaining >= 64 {
                u64::MAX
            } else {
                (1u64 << remaining) - 1
            };
            let new = full & !self.seen[w];
            fresh += new.count_ones() as usize;
            self.seen[w] |= full;
        }
        self.seen_count += fresh;
        fresh as u32
    }

    /// Size the per-maze flags to the level (once per level: no per-tick
    /// allocation).
    fn sync_mazes(&mut self, mazes: usize) {
        if self.maze_entrance_seen.len() != mazes {
            self.maze_entrance_seen.resize(mazes, false);
            self.maze_claimed.resize(mazes, false);
        }
    }

    fn note_crossed(&mut self, cell: (i32, i32)) {
        if self.crossed.len() >= MAX_CROSSED_HOLES || self.hole_crossed(cell) {
            return;
        }
        self.crossed.push(cell);
    }

    /// Is `(cx, cy)` see-through for the sight flood?
    ///
    /// Open floor and doorways are; a one-way door only once its leaf has
    /// swung open (the door system, a later phase, drives
    /// [`OneWayDoor::open_progress`](crate::game::level::OneWayDoor)); solid
    /// walls, maze partitions and rat-holes are not — a rat-hole is handled
    /// separately as a glance source, so you glimpse a couple of cells past it
    /// instead of seeing straight through.
    fn see_through(level: &Level, cx: i32, cy: i32) -> bool {
        match level.tiles.get(cx, cy) {
            TileKind::Open => true,
            TileKind::Wall(WallKind::Doorway { .. }) => true,
            TileKind::Wall(WallKind::OneWayDoor { .. }) => level
                .objects
                .door_at(cx, cy)
                .is_some_and(|door| door.open_progress() >= 0.5),
            _ => false,
        }
    }

    /// One bit per one-way door (the first 64) that is currently see-through,
    /// by the same `open_progress() >= 0.5` rule as [`Self::see_through`].
    fn door_signature(level: &Level) -> u64 {
        let mut sig = 0u64;
        for (i, door) in level.objects.one_way_doors.iter().enumerate().take(64) {
            if door.open_progress() >= 0.5 {
                sig |= 1u64 << i;
            }
        }
        sig
    }

    fn next_stamp(&mut self) -> u32 {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.stamp = 1;
            self.mark.fill(0);
        }
        self.stamp
    }

    fn mark_seen(&mut self, i: usize) -> u32 {
        let word = i >> 6;
        let bit = 1u64 << (i & 63);
        self.seen_at[i] = self.clock;
        if self.seen[word] & bit != 0 {
            return 0;
        }
        self.seen[word] |= bit;
        self.seen_count += 1;
        1
    }

    /// Breadth-first sight flood from `origin`, at most `radius` steps.
    ///
    /// Marks the origin, every see-through cell it reaches and every opaque
    /// cell bordering one. Rat-holes found on the way are recorded in the hole
    /// buffer for [`Self::reveal_cell`] to glance through (only for the wide
    /// main flood: `reveal_cell` passes a radius that is not the glance one).
    fn flood(&mut self, level: &Level, origin: (i32, i32), radius: i32) -> u32 {
        self.holes_len = 0;
        let Some(origin_idx) = self.index(origin.0, origin.1) else {
            return 0;
        };
        let glance = radius <= FOG_GLANCE_RADIUS && radius != FOG_REVEAL_RADIUS;
        let stamp = self.next_stamp();
        let mut fresh = self.mark_seen(origin_idx);

        self.queue.clear();
        self.mark[origin_idx] = stamp;
        self.depth[origin_idx] = 0;
        self.queue.push(origin_idx as u32);

        let r2 = radius * radius;
        let mut head = 0usize;
        while head < self.queue.len() {
            let i = self.queue[head] as usize;
            head += 1;
            let cx = (i as i32) % self.cols;
            let cy = (i as i32) / self.cols;
            let d = self.depth[i] as i32;
            if d >= radius {
                continue;
            }
            for &(dx, dy) in NEIGHBOURS.iter() {
                let (nx, ny) = (cx + dx, cy + dy);
                let Some(ni) = self.index(nx, ny) else {
                    continue;
                };
                let nd = d + 1;
                let ox = nx - origin.0;
                let oy = ny - origin.1;
                let kind = level.tiles.get(nx, ny);

                // Opaque wall: outline it (so a room reads as a room) and stop.
                if !Self::see_through(level, nx, ny) {
                    self.mark_seen(ni);
                    if matches!(kind, TileKind::Wall(WallKind::RatHole { .. })) && !glance {
                        self.remember_hole(nx, ny);
                    }
                    continue;
                }
                // Diagonal steps must not squeeze past a corner: the same rule
                // the nav mask uses. Do this after the outline pass so corner
                // wall cells still get drawn.
                if dx != 0 && dy != 0 {
                    let open_x = Self::see_through(level, cx + dx, cy);
                    let open_y = Self::see_through(level, cx, cy + dy);
                    if !(open_x && open_y) {
                        continue;
                    }
                }
                if nd > radius || ox * ox + oy * oy > r2 {
                    continue;
                }
                if self.mark[ni] == stamp && self.depth[ni] as i32 <= nd {
                    continue;
                }
                self.mark[ni] = stamp;
                self.depth[ni] = nd as u8;
                fresh += self.mark_seen(ni);
                self.queue.push(ni as u32);
            }
        }
        fresh
    }

    fn remember_hole(&mut self, cx: i32, cy: i32) {
        if self.holes_len >= MAX_HOLES {
            return;
        }
        for &(hx, hy) in self.holes[..self.holes_len].iter() {
            if hx == cx && hy == cy {
                return;
            }
        }
        self.holes[self.holes_len] = (cx, cy);
        self.holes_len += 1;
    }
}

/// One maze section's discovery state, as the minimap sees it.
///
/// Built by [`crate::game::world::World::maze_markers`]; `Copy` and free of
/// allocations so the renderer can iterate it per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MazeMarker {
    /// Position in [`crate::game::level::Level::mazes`].
    pub index: usize,
    /// Cell the player enters the maze through (the "breach").
    pub entrance: (i32, i32),
    /// World position of the cache pedestal at the maze's far end.
    pub reward: Vec2,
    /// Has the breach been seen? Stays true once it is.
    pub entrance_seen: bool,
    /// Is the cache itself on explored ground (its star may be drawn)?
    pub reward_seen: bool,
    /// Has the cache been claimed? Driven by the reward runtime (later phase)
    /// through [`Fog::set_maze_claimed`]; `false` until then.
    pub claimed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_traps::OneWayDoorState;
    use crate::game::art_walls::{Cardinal, WallAxis};
    use crate::game::collision::{SizeClass, TileMap};
    use crate::game::level::{Level, OneWayDoor, Theme};

    /// A blank 40 × 30-cell arena (wall-free, no objects).
    fn arena() -> Level {
        let mut level = Level::empty(Theme::Sewer);
        level.tiles = TileMap::new(40, 30, Theme::Sewer, 0);
        level.arena = level.tiles.world_size();
        level.start = level.arena * 0.5;
        level
    }

    fn centre(cell: (i32, i32)) -> Vec2 {
        TileMap::tile_rect(cell.0, cell.1).center()
    }

    /// A wall spanning the whole map height in column `cx`: a real barrier,
    /// so "behind it" cannot be reached by walking around.
    fn barrier_column(level: &mut Level, cx: i32, kind: TileKind) {
        for cy in 0..level.tiles.rows() {
            level.tiles.set(cx, cy, kind);
        }
        level.tiles.rebuild_masks();
    }

    #[test]
    fn a_reveal_reaches_the_radius_and_no_further() {
        let level = arena();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        let origin = (20, 15);
        let fresh = fog.reveal_cell(&level, origin);
        assert!(fresh > 100, "a radius-9 disc is ~250 cells, got {fresh}");
        assert!(fog.is_seen(origin.0, origin.1));
        assert!(fog.is_seen(origin.0 + FOG_REVEAL_RADIUS, origin.1));
        assert!(fog.is_seen(origin.0, origin.1 - FOG_REVEAL_RADIUS));
        assert!(!fog.is_seen(origin.0 + FOG_REVEAL_RADIUS + 2, origin.1));
        assert!(!fog.is_seen(origin.0, origin.1 + FOG_REVEAL_RADIUS + 2));
        // A diagonal at the edge of the disc is outside it.
        assert!(!fog.is_seen(
            origin.0 + FOG_REVEAL_RADIUS,
            origin.1 + FOG_REVEAL_RADIUS
        ));
    }

    #[test]
    fn a_wall_blocks_the_flood_but_gets_outlined() {
        let mut level = arena();
        let origin = (10, 15);
        barrier_column(&mut level, 14, TileKind::Wall(WallKind::Solid));
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, origin);
        // The wall cell itself is drawn (the room needs an outline) ...
        assert!(fog.is_seen(14, 15), "the bordering wall must be revealed");
        // ... but nothing behind it is.
        assert!(!fog.is_seen(15, 15));
        assert!(!fog.is_seen(16, 15));
        assert!(!fog.is_seen(20, 15));
    }

    #[test]
    fn a_closed_one_way_door_blocks_sight_until_it_opens() {
        let mut level = arena();
        let origin = (10, 15);
        barrier_column(&mut level, 14, TileKind::Wall(WallKind::Solid));
        level
            .tiles
            .set(14, 15, TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::East,
            }));
        level.tiles.rebuild_masks();
        let mut door = OneWayDoor::new((14, 15), Vec2::new(1.0, 0.0));
        door.view.state = OneWayDoorState::Closed;
        level.objects.one_way_doors.push(door);

        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, origin);
        assert!(fog.is_seen(14, 15), "the door leaf itself is a wall cell");
        assert!(!fog.is_seen(15, 15), "a closed door blocks the flood");
        assert!(!fog.is_seen(16, 15));

        // With the leaf swung open the same cell is see-through (the door art
        // has no separate `Open` state: a fully `Opening` leaf is open).
        level.objects.one_way_doors[0].view.state = OneWayDoorState::Opening;
        level.objects.one_way_doors[0].view.progress = 1.0;
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, origin);
        assert!(fog.is_seen(15, 15));
        assert!(fog.is_seen(16, 15));
    }

    /// A leaf that swings open while the rat stands still must rerun the
    /// flood: the cell did not change, what it can see did.
    #[test]
    fn a_door_opening_in_front_of_a_standing_rat_reruns_the_flood() {
        let mut level = arena();
        barrier_column(&mut level, 14, TileKind::Wall(WallKind::Solid));
        level
            .tiles
            .set(14, 15, TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::East,
            }));
        level.tiles.rebuild_masks();
        level
            .objects
            .one_way_doors
            .push(OneWayDoor::new((14, 15), Vec2::new(1.0, 0.0)));

        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        let pos = centre((13, 15));
        assert!(fog.update_from(&level, pos));
        assert!(!fog.is_seen(15, 15));
        assert!(!fog.update_from(&level, pos), "nothing changed: no flood");

        level.objects.one_way_doors[0].view.state = OneWayDoorState::Opening;
        level.objects.one_way_doors[0].view.progress = 1.0;
        assert!(fog.update_from(&level, pos), "the open leaf reruns the flood");
        assert!(fog.is_seen(15, 15));
        assert!(!fog.update_from(&level, pos), "and only once");
    }

    #[test]
    fn a_rat_hole_far_mouth_opens_only_by_sight_or_crossing() {
        let mut level = arena();
        let origin = (10, 15);
        barrier_column(&mut level, 14, TileKind::Wall(WallKind::Solid));
        level.tiles.set(
            14,
            15,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        level.tiles.rebuild_masks();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());

        // From the near side: the hole is outlined, the far mouth stays dark.
        fog.reveal_cell(&level, origin);
        assert!(fog.is_seen(14, 15), "the hole cell is outlined");
        assert!(
            !fog.is_seen(15, 15),
            "an uncrossed rat-hole must not paint the far side"
        );
        assert!(!fog.hole_crossed((14, 15)));

        // Crossing it (the rat's centre enters the hole) opens the far mouth —
        // as a peek while the rat is still in the hole ...
        fog.reveal_cell(&level, (14, 15));
        assert!(fog.hole_crossed((14, 15)));
        assert!(fog.is_seen(15, 15), "the far mouth is glimpsed");
        assert!(
            fog.is_seen(16, 15),
            "the glimpse reaches FOG_GLANCE_RADIUS cells past the hole"
        );
        assert!(!fog.is_seen(19, 15), "the glimpse is a peek, not a doorway");

        // ... and the full view arrives when the rat is through.
        fog.reveal_cell(&level, (15, 15));
        assert!(fog.is_seen(19, 15));

        // A crossed hole keeps glimpsing its far mouth from the near side, so a
        // known shortcut always shows its foothold.
        let mut far = Fog::new(level.tiles.cols(), level.tiles.rows());
        far.reveal_cell(&level, (14, 15));
        far.reveal_cell(&level, origin);
        assert!(
            far.is_seen(15, 15),
            "a crossed hole keeps its foothold when seen from the near side"
        );
    }

    #[test]
    fn walking_into_new_ground_reveals_more() {
        let level = arena();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.update_from(&level, centre((5, 5)));
        let first = fog.seen_count();
        assert!(first > 0);
        // Standing still does nothing.
        assert!(!fog.update_from(&level, centre((5, 5)) + Vec2::new(1.0, 1.0)));
        assert_eq!(fog.seen_count(), first);
        // A cell change floods new ground.
        assert!(fog.update_from(&level, centre((30, 20))));
        assert!(fog.seen_count() > first);
        assert!(fog.is_seen(30, 20));
    }

    #[test]
    fn a_maze_interior_hides_until_the_rat_walks_it() {
        // Any generated map with a maze section: from the doorway the flood
        // reaches a little way in, and the far end stays dark.
        let mut found = None;
        for seed in 0..64u64 {
            let level = Level::generate(Theme::Sewer, seed);
            if let Some(&maze) = level.mazes.first() {
                found = Some((seed, level, maze));
                break;
            }
        }
        let Some((seed, level, maze)) = found else {
            panic!("no generated Sewer map in the first 64 seeds has a maze");
        };
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, maze.entry);
        assert!(
            fog.is_seen(maze.entry.0, maze.entry.1),
            "the doorway is revealed (seed {seed})"
        );
        // The reward sits at the far end of the maze: never visible from the
        // entry, since the flood is bounded by radius *and* line of sight.
        let reward = cell_of(maze.reward);
        assert!(
            !fog.is_seen(reward.0, reward.1),
            "maze interior at {:?} must stay hidden from the entry {:?} (seed {seed})",
            reward,
            maze.entry
        );
        // Walking the maze reveals it.
        fog.reveal_cell(&level, reward);
        assert!(fog.is_seen(reward.0, reward.1));
    }

    #[test]
    fn maze_discovery_and_claim_flags_start_empty_and_follow_sight() {
        // Mazes are rare now (~1 map in 8), so a fixed seed is not guaranteed to
        // have one: search the first seeds for a maze-bearing Sewer map.
        let level = (0..64u64)
            .map(|seed| Level::generate(Theme::Sewer, seed))
            .find(|level| !level.mazes.is_empty())
            .expect("no Sewer map in the first 64 seeds has a maze");
        let mut fog = Fog::with_mazes(level.tiles.cols(), level.tiles.rows(), level.mazes.len());
        assert_eq!(fog.maze_count(), level.mazes.len());
        for i in 0..fog.maze_count() {
            assert!(!fog.maze_entrance_seen(i));
            assert!(!fog.maze_claimed(i), "no cache is claimed before the runtime");
        }
        if let Some(maze) = level.mazes.first() {
            fog.reveal_cell(&level, maze.entry);
            assert!(
                fog.maze_entrance_seen(0),
                "standing in the breach must mark the maze found"
            );
        }
        // The claim flag is driven by the (later) reward runtime.
        fog.set_maze_claimed(0, true);
        assert!(fog.maze_claimed(0));
        fog.set_maze_claimed(0, false);
        assert!(!fog.maze_claimed(0));
        // Out-of-range indices are harmless.
        assert!(!fog.maze_entrance_seen(99));
        assert!(!fog.maze_claimed(99));
        fog.set_maze_claimed(99, true);
    }

    #[test]
    fn an_unvisited_maze_entrance_is_never_flagged() {
        let level = Level::generate(Theme::Lab, 5);
        let mut fog = Fog::with_mazes(level.tiles.cols(), level.tiles.rows(), level.mazes.len());
        // Reveal far away from every maze entrance.
        fog.reveal_cell(&level, crate::game::collision::cell_of(level.start));
        for (i, maze) in level.mazes.iter().enumerate() {
            if !fog.is_seen(maze.entry.0, maze.entry.1) {
                assert!(!fog.maze_entrance_seen(i));
            }
        }
    }

    #[test]
    fn the_reveal_never_leaks_through_a_maze_partition() {
        // A maze partition spanning the map: nothing beyond it is revealed
        // except its near face.
        let mut level = arena();
        let origin = (10, 15);
        barrier_column(&mut level, 14, TileKind::Wall(WallKind::Maze));
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, origin);
        assert!(fog.is_seen(14, 15));
        for cy in 0..level.tiles.rows() {
            assert!(!fog.is_seen(15, cy), "leaked past the maze wall at y={cy}");
            assert!(!fog.is_seen(16, cy), "leaked past the maze wall at y={cy}");
        }
    }

    #[test]
    fn a_diagonal_step_cannot_cut_a_wall_corner() {
        // An L of solid cells whose only opening is the diagonal from
        // (10, 15) to (11, 14): both orthogonals are wall, so the flood must
        // not pass.
        let mut level = arena();
        for cell in [(11, 15), (10, 14), (11, 16), (12, 15), (11, 13), (12, 14), (12, 13)] {
            level.tiles.set(cell.0, cell.1, TileKind::Wall(WallKind::Solid));
        }
        level.tiles.rebuild_masks();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, (10, 15));
        assert!(
            !fog.is_seen(11, 14),
            "the sight flood squeezed through a one-cell wall corner"
        );
        assert!(fog.is_seen(9, 15), "the open side is still revealed");
    }

    #[test]
    fn reveal_is_stable_and_allocation_free_in_shape() {
        let level = arena();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        let cap = fog.queue.capacity();
        for i in 0..20 {
            fog.reveal_cell(&level, (5 + i, 15));
        }
        assert_eq!(
            fog.queue.capacity(),
            cap,
            "the flood queue must not grow during a run"
        );
        assert!(fog.seen_count() > 100);
    }

    #[test]
    fn out_of_bounds_cells_are_never_seen() {
        let level = arena();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, (0, 0));
        assert!(!fog.is_seen(-1, 0));
        assert!(!fog.is_seen(0, -1));
        assert!(!fog.is_seen(fog.cols(), 0));
        assert!(!fog.is_seen(0, fog.rows()));
        assert_eq!(fog.recency(-5, -5), 0.0);
    }

    #[test]
    fn the_fade_keeps_the_sight_bubble_live_and_fades_memory() {
        let level = arena();
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, (20, 15));
        assert_eq!(fog.recency(20, 15), 1.0);
        assert!(fog.in_view(20, 15));
        // Standing still: no new flood runs, so the bubble stays live.
        for _ in 0..(FOG_FADE_TICKS * 2) {
            fog.tick();
        }
        assert_eq!(fog.recency(20, 15), 1.0, "standing still must not dim the view");
        // Walking away turns it into memory, which then ages out.
        fog.reveal_cell(&level, (2, 2));
        assert!(!fog.in_view(20, 15));
        assert_eq!(fog.recency(20, 15), 0.0);
        // Fading never un-explores a cell.
        assert!(fog.is_seen(20, 15));
    }

    #[test]
    fn size_classes_do_not_matter_to_sight() {
        // A rat-hole is see-through for the glance but not for the flood, and
        // a doorway is see-through for both classes (Level::empty's mask is
        // large-class; the fog never reads it).
        let level = arena();
        assert!(TileKind::Open.passable(SizeClass::Small));
        assert!(TileKind::Open.passable(SizeClass::Large));
        let mut fog = Fog::new(level.tiles.cols(), level.tiles.rows());
        fog.reveal_cell(&level, (20, 15));
        assert!(fog.seen_count() > 0);
    }
}
