# CONTENT_PLAN.md — procedural levels, mazes, traps, bosses and pathfinding

> **Status: delivered; kept as the record of the contract.** Everything below
> shipped — the tile-grid levels, mazes, rat-holes, traps, the nav rewrite and
> the first three bosses — and `Level::walls` is gone. The game has since grown
> past it (six floors, ten bosses, per-floor rosters), so read `CLAUDE.md` for
> how the tree works now and this file for *why* the level, collision and nav
> contracts look the way they do. Still binding: §7 (determinism) and §9 (art
> view structs are the spec where art and gameplay disagree). No longer
> current: the five-owner split in §8, "all three maps" in §11, the §2 note of
> up to two mazes (the generator carves at most one), and §10's `cargo test
> levelgeneration` (no test has that name; the generator's tests live under
> `game::levelgen`, e.g. `cargo test levelgen`). A failing golden now prints
> every moved row ready to paste into `CASES`.

The owner's brief: boss fights with *a huge jungle cat, a human with a broom and a
robot lawnmower*; *sticky traps and mousetraps*; *each level 10× larger with
procedural layouts, mazes, walls with holes the rat can duck into that larger
enemies can't follow*; *mazes that generate sometimes, with a powerup at the end
and a one-way door*; and *optimize the pathfinding*.

The art worker's Gemini-Neon files are the **design spec** for the gameplay
below — their view structs, state enums, per-state doc comments and collision
constants are binding:

| file | owns |
|---|---|
| `src/game/art_walls.rs` | 16 px wall tile system: `WallKind`, `NeighborMask`, `WallCellView`, `wall_colliders`, `rat_hole_gap_rect`, `one_way_gate_rect`, the three draw layers |
| `src/game/art_traps.rs` | `StickyTrapView`, `MousetrapView`, `OneWayDoorView`, `RatHoleView`, `MazeRewardView` + all their numbers |
| `src/game/art_boss_cat.rs` / `art_boss_broom.rs` / `art_boss_mower.rs` | the three bosses' art (their view structs are the boss-fight spec) |

**Never edit an `art_*.rs` file.** If art and gameplay disagree, say so and stop
(§9 lists the disagreements found so far).

This document is the contract for five parallel implementation workers (§8) plus
the skeleton that is already in the tree (§0). Read `CLAUDE.md` first: the
single-threaded chained schedule, the `SpawnId` iteration order, the
`EnemyIndex` validity window and the "no per-tick allocation in systems" rule
bind every line of new code.

---

## 0. What is already in the tree (this change)

The foundation is implemented, tested and clippy-clean. It is a *trivial
placeholder map*, deliberately, so that all five workers have real data:

* **`src/game/collision.rs` — the tile grid.** `TileMap` (200 × 160 cells of 16
  px), `TileKind`, `SizeClass`, neighbour masks, and every wall query
  (`get`, `passable`, `travel_ok`, `one_way_dir`, `for_each_collider_in`,
  `resolve_circle`, `segment_hit`, `circle_clear`, `rect_clear`,
  `cell_range`, `visible_range`, `iter_walls`). Colliders come from
  `art_walls::wall_colliders`, so art and physics cannot disagree.
* **`src/game/level.rs` — the data model** (§1): `Level`, `TileRect`, `Room`,
  `RoomKind`, `MazeSection`, `LevelObjects`, `Prop`, `Palette`.
* **`src/game/levelgen/` — the pipeline** (§2) with every function of the
  ownership table stubbed at its final signature and a placeholder body:
  one bordered arena (two-thick border, two halls, four blocks, a two-thick
  interior wall with a two-cell `Doorway` and a `RatHole`), one 20 × 16-cell
  maze with a reward and a `OneWayDoor` exit, four rat-hole cells, and four
  traps (two sticky, two mousetraps).
* **`collision::NavGrid::from_tiles`** and `Level::nav` (interim: one
  *large*-class mask, so no enemy is ever routed into a hole it cannot fit).
* **`world.rs`**: `Level::generate(theme, seed)` wired to `World::new(seed)`,
  and the four draw layers (wall ground → wall bodies → entities → overhang +
  object foreground) culled to the viewport.
* **`levelgen::validate`**: connectivity for both size classes + object checks;
  `Level::generate` retries with a salted seed (up to 4 attempts) and panics in
  debug if a map never validates.
* **`src/game/ecs/harness.rs` goldens were regenerated** — levels are
  procedural now, so the pre-port reference can no longer reproduce a trace.
  From this change the goldens freeze *this* build's behaviour
  (`same_seed_same_run` still proves determinism).

Measured on this machine (`cargo test`, dev profile `opt-level = 1`):

| measurement | value |
|---|---|
| `Level::generate` (placeholder) | 3.5 ms |
| `World::draw`, 160 × 84 px viewport | 62 µs |
| `World::draw`, same + 200 enemies | 178 µs |
| `World::update`, 200 enemies | 125 µs/tick |
| full 200 × 160 flow-field recompute (old `FlowField`, one class) | **2.6 ms** ← the baseline worker C must beat |
| whole test suite | 4.5 s, 340 tests |

Still on the old path (deliberately, each is another worker's task):
`level.walls` iteration in five systems (§4), the single-class `FlowField` (§3),
and the spawn ring that ignores walls (§5).

---

## 1. The data model

```rust
// src/game/level.rs  (all implemented)
pub const MAP_COLS: i32 = 200;
pub const MAP_ROWS: i32 = 160;
pub const MAP_SIZE: Vec2 = Vec2 { x: 3200.0, y: 2560.0 };   // = 10× the old 1020×800 area

pub struct Level {
    pub theme: Theme,
    pub seed: u64,              // the run seed it was generated from (0 for Level::new)
    pub arena: Vec2,            // == tiles.world_size() == MAP_SIZE
    pub tiles: TileMap,         // authoritative: walls, collision, navigation
    pub walls: Vec<Rectf>,      // TRANSITIONAL mirror of the solid cells (see §4)
    pub props: Vec<Prop>,       // themed furniture; its cells are WallKind::Solid
    pub palette: Palette,
    pub nav: NavGrid,           // interim single-class mask (§3)
    pub rooms: Vec<Room>,       // named regions, start room first
    pub start: Vec2,            // rat spawn; ALWAYS MAP_SIZE * 0.5
    pub mazes: Vec<MazeSection>,// 0..=2
    pub objects: LevelObjects,  // traps, doors, rat-holes, rewards (mutable runtime state)
}

pub struct TileRect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }  // cell space
pub enum RoomKind { Start, Boss, Hall, Room, Corridor, Maze }
pub struct Room { pub rect: TileRect, pub kind: RoomKind }

pub struct MazeSection {
    pub bounds: TileRect,       // the reserved slot
    pub entry: (i32, i32),      // cell the player enters through
    pub exit: (i32, i32),       // the OneWayDoor cell
    pub exit_dir: Cardinal,     // the only allowed travel direction (out)
    pub reward: Vec2,           // MazeRewardView position
}

pub struct LevelObjects {
    pub sticky_traps: Vec<StickyTrapView>,   // art_traps' own structs: one, not two
    pub mousetraps: Vec<MousetrapView>,
    pub one_way_doors: Vec<OneWayDoor>,      // { cell, view: OneWayDoorView }
    pub rat_holes: Vec<RatHole>,             // { cell, view: RatHoleView }
    pub rewards: Vec<MazeRewardView>,
}
```

```rust
// src/game/collision.rs  (all implemented)
pub const TILE: i32 = 16;   pub const TILE_F: f32 = 16.0;

pub enum SizeClass { Small, Large }          // Small ⇔ radius <= 3.5 px
impl SizeClass {
    pub fn of_radius(radius: f32) -> SizeClass;
    pub fn of_enemy(kind: EnemyKind) -> SizeClass;
    pub const ALL: [SizeClass; 2];  pub const fn index(self) -> usize;
}
// Small: rat 3.0, Bat 2.0, Skitterer 2.5, Spitter 3.0   (fit an 8 px rat-hole)
// Large: Cat 4.0, Brute 6.5, Boss 14.0                  (may not pass a rat-hole)

pub enum TileKind { Open, Wall(WallKind) }
pub struct TileCell { pub kind: TileKind, pub mask: NeighborMask }

pub struct TileMap { /* theme, seed, cols, rows, cells */ }
```

**Decisions the contract left open** (made here, all binding):

1. **The tile grid owns collision; `Level::walls` is a transitional mirror.**
   `sync_walls()` merges solid cells into runs; nothing may write it from
   simulation code, and worker D deletes it (§4).
2. **Props are cell-aligned and painted into the grid.** A `Prop.rect` is a
   whole number of cells and those cells are `WallKind::Solid`, so
   "what you see blocks" holds exactly. `art_props::draw_prop` sizes its art
   from `prop.rect`, so a 32×32 crate cell-block looks right.
3. **`LevelObjects` stores `art_traps`' view structs directly.** The doc comment
   on each art state *is* the gameplay spec, so there is one struct, not a
   parallel sim struct that can drift. The renderer only overwrites `anim`
   (the run clock).
4. **`SizeClass` has exactly two variants.** Bosses are `Large`; the boss arena
   is entered through a normal `Doorway` (14 px clear) and its interior is open
   floor, so the large class reaches it (validated). If a later boss phase needs
   a "boss-only" route, add a third class, do not special-case radii.
5. **The start room is centred on the map and `Level::start == MAP_SIZE * 0.5`.**
   `app.rs` and dozens of tests assume the arena centre is playable; the
   generator must keep that invariant.
6. **`Level::empty(theme)`** is the facade's round-trip spare (never simulated,
   never drawn) — it spares a second full generation per `World::new`, which
   matters because tests build hundreds of worlds.
7. **`Level::generate` retries** with `(seed, attempt)` up to 4 times when
   `validate` fails. Deterministic; `generate_once(theme, seed, 0)` must already
   validate for every seed (there is a test).

---

## 2. The generator (`src/game/levelgen/`)

```text
Level::generate(theme, seed)                     levelgen::generate
  └─ for attempt in 0..MAX_ATTEMPTS
       generate_once(theme, seed, attempt)       one StdRng, one TileMap
         A1 layout::build_layout(&mut tiles, &mut rng, theme) -> Layout
         A2 layout::furnish(&mut tiles, &layout, &mut rng, theme) -> Vec<Prop>
         B1 maze::carve_mazes(&mut tiles, &layout, &mut rng, theme) -> Vec<MazeSection>
         B2 maze::place_rat_holes(&mut tiles, &layout, &mut rng) -> Vec<((i32,i32), WallAxis)>
         B3 maze::place_traps(&tiles, &layout, &mazes, &mut rng) -> Vec<TrapPlacement>
         ── tiles.rebuild_masks(); objects_from(..); sync_walls(); nav = NavGrid::from_tiles
       validate::validate(&level)?   or try the next attempt
```

`level_seed(theme, seed, attempt)` mixes (run seed, theme, attempt) with
splitmix64; `level_rng` turns it into `StdRng`. **Every** random choice comes
from that one rng in call order and **no stage iterates a `HashMap`** — that is
the whole determinism contract of the generator.

### A. Generator part A — `levelgen/layout.rs`

```rust
pub struct Layout {
    pub rooms: Vec<Room>,          // RoomKind::Start first
    pub start: Vec2,               // == start_room.center_world() == MAP_SIZE*0.5
    pub start_room: TileRect,
    pub boss_arena: TileRect,
    pub maze_slots: Vec<TileRect>, // 0..=2 free rectangles reserved for part B
}
impl Layout { pub fn maze_slot(&self, index: usize) -> Option<TileRect>; }

pub fn build_layout(tiles: &mut TileMap, rng: &mut StdRng, theme: Theme) -> Layout;
pub fn furnish(tiles: &mut TileMap, layout: &Layout, rng: &mut StdRng, theme: Theme) -> Vec<Prop>;
pub fn random_cell(rng: &mut StdRng, within: TileRect, w: i32, h: i32) -> Option<(i32, i32)>;
```

Must provide: rooms, corridors, open halls, **exactly one** `RoomKind::Boss`
arena of ≥ 400 cells (the contract asks for "a large open boss arena room on
every map"), the start room, and 0..=2 reserved `maze_slots`. Rooms are
connected with `Doorway` cells (one or two cells wide: a two-cell doorway is
30 px clear and reads as a double door in the art). Furnishing paints props as
solid cells and must keep the 3 × 3 cells around every room centre clear.

May assume: `TileMap`'s write API (`set`, `fill_cells`), `TileRect` helpers,
`theme_props(theme)`, and that `furnish` runs after `build_layout` and before
anything else. Must leave `rows`/`cols` alone, never touch the outer two cells,
and never write a `RatHole`, `Doorway` or `OneWayDoor` client code by hand —
only the kinds art defines.

### B. Generator part B — `levelgen/maze.rs` + `levelgen/validate.rs`

```rust
pub enum TrapKind { Sticky, Snap }
pub struct TrapPlacement { pub kind: TrapKind, pub pos: Vec2, pub facing: Vec2 }

pub fn carve_mazes(tiles: &mut TileMap, layout: &Layout, rng: &mut StdRng, theme: Theme)
    -> Vec<MazeSection>;
pub fn place_rat_holes(tiles: &mut TileMap, layout: &Layout, rng: &mut StdRng)
    -> Vec<((i32, i32), WallAxis)>;      // EVERY hole cell in the map, sorted
pub fn place_traps(tiles: &TileMap, layout: &Layout, mazes: &[MazeSection], rng: &mut StdRng)
    -> Vec<TrapPlacement>;
pub fn open_cells(tiles: &TileMap, rect: TileRect, class: SizeClass) -> Vec<(i32, i32)>;

pub fn validate(level: &Level) -> Result<(), Vec<String>>;
pub fn reachable(tiles: &TileMap, from: (i32, i32), class: SizeClass) -> Vec<bool>;
pub fn is_reached(seen: &[bool], tiles: &TileMap, cell: (i32, i32)) -> bool;
pub fn reaches_rect(seen: &[bool], tiles: &TileMap, rect: TileRect) -> bool;
pub fn reachable_cells(tiles: &TileMap, from: (i32, i32), class: SizeClass) -> usize;
pub fn cell_of(p: Vec2) -> (i32, i32);
pub const BOSS_ARENA_MIN_CELLS: i32 = 400;
```

Must provide: 0..=2 mazes parsed out of `layout.maze_slots` (each a `Maze`
partition warren with a two-cell entry gap, a `MazeRewardView` at its end and a
`OneWayDoor` as its **only** exit, `exit_dir` pointing out of the maze);
rat-holes only in walls that are one cell thick on the passage axis (piercing a
two-cell wall means piercing both cells) and never in the border, the start room
or inside the boss arena; traps only on open floor away from the start room and
never inside a maze corridor's only route; and a `validate` that proves (a) the
player reaches every room and reward, (b) the large class reaches the boss arena
and every room the horde must enter, (c) each maze exit leads back to reachable
floor.

May assume: everything §0 lists, and that `carve_mazes` runs after `furnish`
(slots are still empty floor).

The **placeholder** in the tree already satisfies all of this for every
(theme, seed) tested; replace the bodies, keep the invariants and the
`validate` guarantees.

---

## 3. Navigation — owner C (`src/game/nav.rs`)

### The problem

The map is 10× bigger, so a full-map BFS costs **2.6 ms** (measured, §0) instead
of 0.26 ms — and it runs every time the player crosses a cell boundary (~4×/s).
Today `FlowField` is one field for every enemy, built from `level.nav`.

### The design (must end up like this)

* **One passability mask per size class** (`Small`, `Large`) built once from
  `TileMap::passable`; a rat-hole is open for `Small` only. Build both at level
  load (`Level`), never per tick.
* **Directed one-way doors**: the edge test is `TileMap::travel_ok(cell, class,
  travel_dir)`, and a *door cell may only be left along its allowed direction*
  (`TileMap::one_way_dir`). In BFS-from-goal terms: expand to a cell only if the
  reverse of that edge is legal. Enemies must be able to travel *out* of a maze
  and never *in*. `levelgen::validate::reachable` is a working reference
  implementation of exactly this rule — copy it, do not reinvent it.
* **Near-fine / far-coarse**:
  * a fine field (one node per tile) inside `FINE_RADIUS_CELLS = 24` cells
    (384 px ≈ 2.4 screens) of the player's cell;
  * a coarse field (one node per 8 × 8-cell block, 25 × 20 blocks) everywhere
    else, whose `dir` points at the next block's centre;
  * `dir_at` picks the fine field inside the radius, the coarse one outside, and
    blends across the boundary band so an enemy does not zig-zag there.
* **Staggering**: an update is only started when the player's *cell* changes (as
  today), but the work is spread over frames with a fixed budget (e.g.
  2 000 cells/frame), keeping the previous field usable meanwhile — never a
  multi-millisecond hitch in the frame that crosses a cell.
* **Sharing between classes**: compare the two passability masks over the fine
  window (a cheap flag per window, refreshed when the window moves). If they are
  identical there — the common case in open halls and corridors — compute and
  store **one** field and have both classes read it.

### Required API

```rust
// keeps the name `FlowField` so the facade round-trip (`World::flow`,
// `swap_in`/`swap_out`) and the schedule are untouched.
pub const FINE_RADIUS_CELLS: i32 = 24;
pub const COARSE_BLOCK_CELLS: i32 = 8;

pub struct NavBudget { pub fine_cells: u32, pub coarse_blocks: u32 }
impl Default for NavBudget { .. }          // the per-frame default

impl FlowField {
    pub fn new(tiles: &TileMap) -> FlowField;
    /// Recompute toward `goal` if it moved to a new cell, spending at most
    /// `budget` work this call. Returns what changed.
    pub fn update(&mut self, tiles: &TileMap, goal: Vec2, budget: NavBudget) -> NavUpdate;
    pub fn dir_at(&self, class: SizeClass, p: Vec2) -> Vec2;
}
pub struct NavUpdate { pub changed: bool, pub fine_cells: u32, pub coarse_blocks: u32, pub shared: bool }
```

Call sites to change (both in `systems/enemy_move.rs`, and only these two
functions):
`flow_refresh_system` → `flow.update(&level.tiles, player.pos, NavBudget::default());`
and `enemy_steering_system` → `flow.dir_at(SizeClass::of_enemy(e.kind), e.pos)`.
`World::new` builds the field from `&level.tiles` instead of `&level.nav`.
Keep `NavGrid` (or a renamed equivalent) working for `Level::nav` while workers
D and E still read it, or migrate `Level::nav` in the same change — say which in
your report.

### The benchmark (required)

`cargo test nav_benchmark -- --nocapture` must print a table for a full-size map
(700 × 160? no: 200 × 160 cells) with: full-map one-class recompute (the 2.6 ms
baseline), fine-window recompute, fine + coarse, both classes shared, both
classes unshared, and the amortised per-frame cost at the default budget. Keep
it a normal test (it asserts the amortised cost stays under a frame budget) so a
regression is caught. Report before/after numbers in your summary.

---

## 4. Tile collision — owner D (`collision.rs` consumers)

`TileMap` (§0) is done. What remains is deleting every wall iteration and the
`Level::walls` field.

### Migration table (exact call sites)

| file / function | today | becomes |
|---|---|---|
| `systems/player.rs::player_system` (per sub-step) | `for w in &level.walls { resolve_circle_rect }` | `player.pos = level.tiles.resolve_circle(player.pos, PLAYER_RADIUS, SizeClass::Small, Some(player.vel));` |
| `systems/enemy_move.rs::wall_resolve_system` | loop over `level.walls` | `e.pos = level.tiles.resolve_circle(e.pos, e.radius, SizeClass::of_enemy(e.kind), None);` |
| `systems/combat.rs` (contact knockback re-resolve) | loop over `level.walls` | same as `wall_resolve`, `None` for `player_vel` |
| `systems/projectiles.rs` (lob landing) | loop + `segment_rect_hit` | `level.tiles.segment_hit(b.prev_pos, b.pos, b.radius)` (keeps the earliest `t`) |
| `systems/spawn.rs::spawn_point_ok` | loop over `level.walls` | `level.tiles.circle_clear(p, wall_clear, SizeClass::Large)` (see §5 for the rest of spawn) |

`player_vel` is `Some(..)` **only** for the player: it is the one-one-way-door
permission. Every enemy passes `None`, so a one-way gate is solid to the horde
from both sides forever.

### API additions (add to `collision.rs`, do not change the existing ones)

```rust
impl TileMap {
    /// Nearest passable cell centre to `p` (bounded search); used by spawn and
    /// by knockback so an entity is never left inside a wall.
    pub fn nearest_open_center(&self, p: Vec2, class: SizeClass, max_cells: i32) -> Option<Vec2>;
}
```

### Deleting `Level::walls`

After the five call sites are migrated: delete the field, `sync_walls`, and the
`walls` setup in every test (`level.walls = vec![Rectf::new(..)]`) — paint tiles
instead:

```rust
level.tiles.fill_cells(cx, cy, w, h, TileKind::Wall(WallKind::Solid));
level.tiles.rebuild_masks();
```

Tests that deliberately test a thin wall ("`fast_player_cannot_tunnel_a_thin_wall`")
should use a one-cell `WallKind::Maze` partition (an 8 px spine — thinner than
the tile grid used to allow) or keep a `Solid` cell and dash at `PLAYER_SUBSTEP`.

Also update `enemy.rs`'s tests (they loop `&level.walls`) and `nav.rs`'s
"points just outside every wall" test.

---

## 5. Rendering + spawn pacing — owner E

### Rendering (`world.rs::draw`)

The layer order is already implemented (`draw_wall_layer`, `draw_object_layer`,
`WallLayer`, `WALL_DRAW_MARGIN`) and matches the art contract:

```text
floor → floor stains → wall GROUND (contact shadows, thresholds)
      → object ground (trap pads, door arrows, reward halo, rat-hole recesses)
      → wall BODIES (caps, south faces, jambs, piers)
      → props → pools → gems → pickups → enemies → bullets → player
      → wall OVERHANG (lintels over rat-holes/doorways/one-way doors)
      → object foreground (glue strands, striker bars, door leaves, arch, pedestal)
      → particles → damage numbers
```

Everything is culled: `TileMap::visible_range(cam, viewport, WALL_DRAW_MARGIN)`
for the three wall passes, `on_screen(..)` for the object lists. Do not iterate
the 32 000 cells per frame.

Owner E's remaining work: verify the three layers against the art doc's
"call order" section, add a frame test per theme (`RATRUN_DUMP=play cargo test
dump_frame_ppm` plus a `TestBackend` assertion that a wall cell and a rat-hole
actually draw pixels), and add the boss arena's visual treatment if the art
files define one.

### Spawn pacing (`systems/spawn.rs` + `director.rs`)

The ring is `player + dir * max(viewport*0.5 + 16, 120)` and today it ignores
walls: on a 10× map a ring point can be inside a wall cluster or in a sealed
pocket, and on the new maps the player can kite much further. Required:

```rust
/// A wall-clear, reachable spawn point for `class`, on the ring, preferring
/// out-of-sight cells and the direction the player is heading.
pub fn spawn_point(rng: &mut StdRng, level: &Level, player: &Player, viewport: Vec2) -> Vec2;
/// Keep the existing signature (tests call it).
pub fn spawn_point_ok(p: Vec2, lo: Vec2, hi: Vec2, min_dist: f32, wall_clear: f32,
                      level: &Level, player_pos: Vec2) -> bool;   // → tiles.circle_clear

/// A reachable cell inside a room (used for a share of pulses, for elites and
/// for the boss), falling back to the ring.
pub fn spawn_point_in_room(rng: &mut StdRng, level: &Level, room: &Room, class: SizeClass) -> Option<Vec2>;
```

Rejection sampling: ≥ 12 ring samples with `circle_clear(.., Large)` **and** the
reachability check (the enemy must be able to path to the player — a cell in a
sealed pocket is not a spawn, it is a lost enemy; you may use the nav field's
cost or `levelgen::validate::reachable`, but do not re-run a full BFS per spawn:
cache a per-room reachability flag or sample inside rooms the large class
reaches).

Re-tune the curve for the bigger map (keep `RUN_SECONDS = 300`, `BOSS_TIME =
255`, `ENEMY_CAP = 600`, one slot reserved for the boss):
`spawn_interval` floor 0.28 s → ~0.20 s and `pulse_count` `2 + t/22` → `3 + t/16`
are the suggested starting points; the acceptance test is
`cargo test scripted_crowd_reaches_the_cap_curve` (peak ≥ 40 before 60 s) plus
`enemy_cap_holds_through_a_full_run`, and the whole horde must still arrive at
the player rather than piling up behind a wall (add a test: 200 s of simulation,
every live enemy's distance to the player decreasing over a 10 s window, or
use the existing nav benchmark).

**Boss spawn**: keep it where it is (a ring point at `BOSS_TIME`) so the
five-minute threat is unchanged. The boss *arena* is a reserved set-piece room
for the boss-fight phase (§6); do not spawn the boss there until that phase
decides how a fight starts (the win is survival, so a boss 2 500 px away would
be irrelevant).

### Director extras worth having

* A 25 s boss warning (callout + compass) — `render/overlay.rs` already draws a
  compass from `World::boss_offset`.
* Spawn "waves" from the rooms the player is not in, so the horde arrives from
  doorways instead of materialising in the open. This is the natural use of
  `Level::rooms` + `spawn_point_in_room`.

---

## 6. Not in these five: the object runtime (phase 6)

Traps, one-way doors and maze rewards have **data** now (`LevelObjects`) and a
`Level::wall_view(cx, cy, anim_clock)` that feeds a door's `open_progress` into
the wall art, but nothing ticks them yet. That is deliberate: the five
assignments above are level infrastructure. The follow-up phase must add:

* `systems/traps.rs`: sticky trap slow/root (`STICKY_TRAP_SLOW_MULT`,
  `STICKY_TRAP_HOLD_*`), mousetrap trigger → snap → damage
  (`MOUSETRAP_TRIGGER_SECS`, `MOUSETRAP_SNAP_SECS`, `MOUSETRAP_DAMAGE_*`,
  `MOUSETRAP_KILL_U_*`), the dash escape, and bats flying over. Chaos order:
  run it right after `player` and before `enemy_steering` (a rooted victim must
  not move that tick) — pick the slot from `schedule.rs` and document it.
* `systems/doors.rs`: `OneWayDoorState` (Closed → Opening → Open → Closing /
  Refused), driving `OneWayDoor::view` and therefore `open_progress` for the
  wall art and the `player_vel: Some(..)` permission in `TileMap::resolve_circle`.
* `systems/rewards.rs`: `MazeRewardState` (Idle → Opening → Opened), granting
  either a free 1-of-3 level-up (`Level::pending_levelups`, the App already
  handles it) or a heal + XP bundle. It needs `App`-level wiring (like the
  level-up screen) — talk to the coordinator before choosing.
* Boss phases (cat / broom / mower `art_boss_*` views), replacing the generic
  boss per theme.

---

## 7. Determinism rules for all new code

1. **Order comes from `SpawnId`, never from query iteration order.** Any loop
   whose result depends on order (rng draws per enemy, drops, nearest ties,
   knockback chains) sorts by `SpawnId` exactly like the existing systems; the
   per-tick `Local<Vec<..>>` scratch pattern is there for that.
2. **Draw rng only where the old code drew.** Every `rng.0` call in a system is
   a spec change: the number *and* order of draws is part of the golden trace.
   Generation-time rng is free-form but must be one `StdRng` in call order.
3. **No hash-map iteration** in generation, validation or per-tick code. Use
   `Vec`, sort explicitly, or index by cell.
4. **No per-tick allocation** in systems: reuse `Local<Vec<_>>`, the facade's
   `swap_in`/`swap_out` twins, and the resource scratch pads. `Level` is
   round-tripped, so mutating `objects` in a system is free.
5. **`EnemyIndex` window**: the cached order is only valid until the next spawn
   or despawn (see `CLAUDE.md`). New systems that read it must call
   `resources::debug_assert_enemy_index`.
6. **Never mutate facade singletons during a tick** — use
   `world.ecs.resource_mut::<T>()`.
7. **Goldens**: `src/game/ecs/harness.rs` must be regenerated only deliberately,
   in the same change that alters level generation or the sim, and the report
   must say so. `same_seed_same_run` and `scripted_crowd_reaches_the_cap_curve`
   must keep passing at every step.

---

## 8. File ownership (five parallel assignments)

| # | Worker | Owns (write) | May read / must not touch |
|---|---|---|---|
| A | **Generator part A** | `src/game/levelgen/layout.rs` (bodies), `src/game/levelgen/mod.rs` only if the pipeline needs a new stage | reads `collision.rs`, `level.rs`; must not touch `maze.rs`, `validate.rs` |
| B | **Generator part B** | `src/game/levelgen/maze.rs`, `src/game/levelgen/validate.rs` | reads A's `Layout`, `collision.rs`, `level.rs`; must not touch `layout.rs` |
| C | **Navigation** | `src/game/nav.rs`; the two nav call sites in `systems/enemy_move.rs` (`flow_refresh_system`, the `dir_at` line in `enemy_steering_system`); `World::new`'s `FlowField` construction in `world.rs` (2 lines) | reads `collision.rs`; must not touch `wall_resolve_system` (`enemy_move.rs`), `Level::nav` semantics beyond building it, or `world.rs::draw` |
| D | **Tile collision** | `src/game/collision.rs` (additions only); `systems/player.rs`, `systems/combat.rs` (knockback line), `systems/projectiles.rs` (lob loop), `systems/spawn.rs::spawn_point_ok`; the deletion of `Level::walls` + `sync_walls` in `level.rs`; the tests that set `level.walls` (`level.rs`, `systems/player.rs`, `systems/spawn.rs`, `systems/projectiles.rs`, `enemy.rs`, `nav.rs`, `ecs/tests.rs`, `ecs/systems/enemy_move.rs`) | reads everything; must not change `TileMap`'s existing signatures (other workers depend on them) |
| E | **Rendering + pacing** | `world.rs::draw` + `draw_wall_layer` / `draw_object_layer`; `systems/spawn.rs` (spawn ring + pacing); `director.rs`; `render/*` if a HUD/overlay change is needed | reads `Level`, `TileMap`, `art_walls`, `art_traps`; must not touch `collision.rs` or `nav.rs` |

Shared files, in priority order — **whoever touches them first should announce
it in the report**:

* `src/game/levelgen/mod.rs` (A adds stages; B may add `objects_from` fields).
* `src/game/world.rs` (C: 2 lines in `World::new`; E: `draw` — different regions).
* `src/game/ecs/systems/enemy_move.rs` (C: `flow_refresh_system`, `dir_at`;
  D: `wall_resolve_system` and its test — different functions).
* `src/game/ecs/harness.rs` (anyone whose change moves a hash: regenerate
  deliberately and say so; never "fix" a golden by editing a simulated number).

Integration order if two workers must serialise: **C and D first** (D deletes
`Level::walls`, which touches many test files), then A and B (the generator
produces what E's pacing samples), E last (it needs rooms to exist). C, D and E
can all start immediately on the placeholder map.

---

## 9. Art ↔ contract conflicts (report, do not "fix" silently)

1. **Rat-hole pass radius: 3.2 vs 3.5.** `art_traps::RAT_HOLE_MAX_PASS_RADIUS`
   is 3.2; `art_walls::RAT_HOLE_MAX_RADIUS` is 3.5. The coordinator's decision
   is **3.5**, so `SizeClass::of_radius` uses
   `art_walls::RAT_HOLE_MAX_RADIUS` and `art_traps::fits_through_rat_hole` must
   **not** be used for gameplay (only the art module calls it today). With 3.5
   the "small" set is unchanged (Bat 2.0, Skitterer 2.5, Spitter 3.0, rat 3.0)
   and "large" starts at Cat 4.0, so nothing observable changes.
2. **One-way door timing.** `art_walls`'s `OneWayDoor` doc says the leaf swings
   open over **0.15 s** and shuts over **0.35 s**; `art_traps`'
   `OneWayDoorState` says **0.24 s** (`ONE_WAY_DOOR_OPEN_SECS`) and 0.35 s.
   `art_traps`' constant is the one to implement (0.24 s), because the *view*
   struct and its state machine live there.
3. **`art_walls::WallKind::RatHole` inversion.** The doc describes the piers as
   two `4 × 16` (Horizontal) / `16 × 4` (Vertical) blocks *plus* the gap, while
   `rat_hole_gap_rect` documents `Horizontal` as an 8 × 16 passage with the wall
   running east-west — i.e. the enum's `WallAxis` names the **wall's** axis, not
   the passage's. Fine, but easy to get backwards: `TileMap` uses
   `WallAxis::Vertical` for a north-south wall whose passage is east-west.
4. **Spitter radius** is 3.0 in `enemy.rs` (the art doc also says Spitter 3.0),
   so a Spitter fits a rat-hole and is `Small`. That is what the art says — keep
   it.
5. `art_walls`' `draw_wall_cell` is documented "before entities (or Y-sorted
   with props/entities)"; we draw it strictly before entities and use the
   overhang layer for the "under the arch" effect, per the same doc's layer
   list.

---

## 10. Commands

```sh
cargo build && cargo test && cargo clippy --all-targets      # must stay clean
cargo test nav_benchmark -- --nocapture                       # owner C's table
RATRUN_DUMP=play cargo test dump_frame_ppm                    # /tmp/ratrun_play.ppm
cargo test levelgeneration -- --nocapture                     # generator tests
```

Golden hashes: `cargo test scripted_runs_match_the_reference_trace` fails with
`left: <new>, right: <golden>` — take the new value from the message, replace
the entry in `harness.rs`'s `CASES`, and note it in your report. **Never**
change a golden to make a *logic* test pass.

Tests that must exist before a worker's change is done:

* **A**: rooms/corridors/halls/boss arena invariants; every room reachable in
  the generated map; `furnish` never blocks a room centre, a maze slot or a
  doorway; two runs of the same seed give the same map.
* **B**: 0..=2 mazes per map with a reward + one-way exit each; every hole is in
  a one-cell wall and separates two open areas that a *large* agent cannot
  cross; traps never on the spawn point; `validate` rejects a deliberately
  sealed map; the first attempt validates for a sweep of seeds.
* **C**: the benchmark table + an assertion; both classes' fields agree in open
  areas; a large enemy never takes a rat-hole route and a small one does; a
  goal across a one-way door is reached from the maze side and not from the
  corridor side; the per-frame budget never exceeds ~0.3 ms.
* **D**: a circle in the middle of a two-cell wall is ejected to the *outside*;
  a fast player cannot tunnel a one-cell `Maze` partition; a lob lands on the
  near face of a wall; spawn points are wall-clear for both classes; no system
  iterates `level.walls` any more (`rg "level.walls"` finds only tests that
  build their own map).
* **E**: each theme draws a frame without panicking (already covered); a wall
  cell and a rat-hole actually paint pixels; the spawn ring never lands in a
  wall on the generated maps; the crowd curve reaches ≥ 40 by 60 s and the cap
  at 300 s.

---

## 11. Definition of done (all five)

`cargo build`, `cargo test`, `cargo clippy --all-targets` clean; the game still
starts and plays on all three maps (`cargo run --release`, real TTY); no art
file edited; goldens regenerated deliberately if (and only if) the sim changed,
with the reason in the report; `Level::walls` gone once D lands; the report names
the data model decisions, the numbers measured and anything in the art files
that conflicted with the contract.
