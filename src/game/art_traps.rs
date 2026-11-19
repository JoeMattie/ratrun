//! Art and gameplay specifications for traps and interactive level objects:
//!
//! 1. **Sticky Trap** (`StickyTrapView`, `StickyTrapState`) — glossy glue board
//!    that roots/heavily slows the rat or ground enemies, with stretching glue
//!    strands as the victim pulls free.
//! 2. **Mousetrap** (`MousetrapView`, `MousetrapState`) — baited wooden snap
//!    trap with a brief quivering warning before the steel striker bar slams
//!    shut, damaging either the rat or lured enemies.
//! 3. **One-Way Door** (`OneWayDoorView`, `OneWayDoorState`) — directional
//!    maze-exit V-flap turnstile with a glowing threshold arrow of light; opens
//!    only from `-facing` to `+facing` and visibly refuses entry from `+facing`.
//! 4. **Rat-Hole** (`RatHoleView`, `RatHoleState`) — small gnawed arched gap
//!    carved through a wall that lets the rat and small swarmers (`radius <= 3.2`)
//!    duck through while blocking Cats, Brutes, and Bosses.
//! 5. **Maze Reward** (`MazeRewardView`, `MazeRewardState`) — gilded powerup
//!    reliquary pedestal marking the end of a maze, with an inviting idle
//!    shimmer, celebratory opening burst, and spent landmark state.
//!
//! # Render Pass Order (for implementers)
//!
//! All five objects follow a zero-allocation, two-layer draw convention so
//! shadows, pads, and telegraphs stay beneath creatures while snapping bars,
//! stretching glue strands, door flaps, wall arches, and raised pedestals layer
//! naturally over them:
//!
//! 1. `art_floor::draw_floor` + floor stains
//! 2. **Ground layer** (`draw_*_ground`): drop shadows, glue boards, wooden
//!    trap bases, danger telegraphs, door floor arrows, tunnel floors, reward
//!    floor halos.
//! 3. Props, gems, pickups, enemies, and player rat (`art_rat::draw_rat`).
//! 4. **Foreground / overlay layer** (`draw_*`): foreground stretching glue
//!    strands, raised mousetrap springs & arcing striker bars, door jambs &
//!    swinging flaps, rat-hole masonry cheeks & over-arch lintel (so the rat
//!    visibly ducks *under* the arch), and the maze reward pedestal/chest.
//!
//! # Footprint & Movement-Blocking Summary
//!
//! | Object         | Footprint (`u × v`, world px) | Blocks Movement? |
//! |----------------|-------------------------------|------------------|
//! | Sticky Trap    | `18.0 × 14.0` (pad `14 × 10`) | No (walkable floor hazard; Bats fly over) |
//! | Mousetrap      | `18.0 × 11.0`                 | No (walkable floor hazard; Bats fly over) |
//! | One-Way Door   | `10.0 × 24.0` (gap `18.0`)    | Jambs always solid; passage open to player from `-facing`, solid from `+facing` and solid to enemies |
//! | Rat-Hole       | `14.0 × 22.0` (gap `8.0`)     | Cheeks always solid; gap passable if `radius <= 3.2`, solid if `radius > 3.2` |
//! | Maze Reward    | `18.0 × 18.0` (`r = 8.5`)     | No (low walkable dais; collects within `r = 11.0`) |
//!
//! Pure drawing: every view struct is `Copy` and decoupled from simulation
//! types. All coordinates are world-pixel space (`1 world px = 1 buffer px`),
//! lit from up-left (`LIGHT = (-0.55, -0.83)`) with crisp dark rims and
//! down-right contact shadows matching `art_rat.rs`, `art_enemy.rs`, and
//! `art_props.rs`.
#![allow(dead_code)]

use std::f32::consts::{PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

/// Direction *toward* the light in screen space (up-left), matching
/// `art_rat.rs`, `art_enemy.rs`, and `art_props.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// ============================================================================
// Collision footprints & gameplay constants (single source of truth)
// ============================================================================

/// Full outer cardboard tray footprint of a sticky trap (`u` length × `v` width).
pub const STICKY_TRAP_SIZE: Vec2 = Vec2 { x: 18.0, y: 14.0 };
/// Active adhesive inner pad half-extents (`|u| <= 7.0`, `|v| <= 5.0`).
pub const STICKY_TRAP_PAD_HALF: Vec2 = Vec2 { x: 7.0, y: 5.0 };
/// Sticky traps never block movement; they apply a heavy slow/root on entry.
pub const STICKY_TRAP_BLOCKS_MOVEMENT: bool = false;
/// Movement speed multiplier applied while trapped in `StickyTrapState::Stuck`.
pub const STICKY_TRAP_SLOW_MULT: f32 = 0.20;
/// Hold durations (seconds) by victim class.
pub const STICKY_TRAP_HOLD_PLAYER_SECS: f32 = 1.6;
pub const STICKY_TRAP_HOLD_SMALL_ENEMY_SECS: f32 = 2.5;
pub const STICKY_TRAP_HOLD_BRUTE_SECS: f32 = 1.5;

/// Full wooden baseboard footprint of a mousetrap (`u` length × `v` width).
/// `+facing` (`+u`) is the baited kill end; `-facing` (`-u`) holds the cocked bar.
pub const MOUSETRAP_SIZE: Vec2 = Vec2 { x: 18.0, y: 11.0 };
/// Mousetraps do not block movement.
pub const MOUSETRAP_BLOCKS_MOVEMENT: bool = false;
/// Center offset along `+facing` of the copper bait trigger plate.
pub const MOUSETRAP_TRIGGER_OFFSET_U: f32 = 3.5;
/// Circular trigger radius around `pos + facing * MOUSETRAP_TRIGGER_OFFSET_U`.
pub const MOUSETRAP_TRIGGER_RADIUS: f32 = 4.5;
/// Dangerous snap-hit box along local `u` (`-1.0..=9.5`) and `|v| <= 6.0`.
pub const MOUSETRAP_KILL_U_MIN: f32 = -1.0;
pub const MOUSETRAP_KILL_U_MAX: f32 = 9.5;
pub const MOUSETRAP_KILL_HALF_V: f32 = 6.0;
/// Warning duration before the striker bar slams shut (seconds).
pub const MOUSETRAP_TRIGGER_SECS: f32 = 0.28;
/// Total snap swing + impact animation duration (seconds).
pub const MOUSETRAP_SNAP_SECS: f32 = 0.20;
/// Active damage window within `MousetrapState::Snapping` (`progress` range).
pub const MOUSETRAP_HIT_PROGRESS_MIN: f32 = 0.40;
pub const MOUSETRAP_HIT_PROGRESS_MAX: f32 = 0.65;
/// Damage dealt by a mousetrap snap (player has 100 HP; Skitterer 6, Brute 22).
pub const MOUSETRAP_DAMAGE_PLAYER: f32 = 20.0;
/// Heavy damage dealt to enemies so luring Cats/Brutes into traps is rewarding.
pub const MOUSETRAP_DAMAGE_ENEMY: f32 = 80.0;

/// Full wall-slot footprint of a one-way door (`u` wall thickness × `v` span).
/// `+facing` (`+u`) is the EXIT side; `-facing` (`-u`) is the maze-interior ENTRY side.
pub const ONE_WAY_DOOR_SIZE: Vec2 = Vec2 { x: 10.0, y: 24.0 };
/// Half-width of the clear passage between the two steel jamb posts (`18.0` px gap).
pub const ONE_WAY_DOOR_PASSAGE_HALF_V: f32 = 9.0;
/// Half-thickness along `u` of the door jambs / solid barrier plane.
pub const ONE_WAY_DOOR_HALF_U: f32 = 5.0;
/// Approach distance on the `-facing` side at which the flaps swing open for the player.
pub const ONE_WAY_DOOR_TRIGGER_DIST: f32 = 10.0;
/// One-way doors block movement from `+facing` (and block enemies from both sides).
pub const ONE_WAY_DOOR_BLOCKS_WRONG_SIDE: bool = true;
/// State timings (seconds).
pub const ONE_WAY_DOOR_OPEN_SECS: f32 = 0.24;
pub const ONE_WAY_DOOR_CLOSE_SECS: f32 = 0.35;
pub const ONE_WAY_DOOR_REFUSED_SECS: f32 = 0.40;

/// Full wall-segment footprint of a rat-hole (`u` wall thickness × `v` total span).
/// Passage runs along `±facing` (`u`); the wall runs along `v`.
pub const RAT_HOLE_SIZE: Vec2 = Vec2 { x: 14.0, y: 22.0 };
/// Half-width of the small arched opening (`8.0` px wide clear tunnel).
pub const RAT_HOLE_GAP_HALF_V: f32 = 4.0;
/// Half-thickness of the wall along `u`.
pub const RAT_HOLE_HALF_U: f32 = 7.0;
/// Maximum entity collision radius that fits through a rat-hole:
/// Player Rat (`3.0`), Skitterer (`2.5`), Bat (`2.0`), Spitter (`3.0`) FIT;
/// Cat (`4.0`), Brute (`6.5`), and Bosses (`14.0+`) are BLOCKED.
pub const RAT_HOLE_MAX_PASS_RADIUS: f32 = 3.2;
/// State timings (seconds).
pub const RAT_HOLE_SQUEEZE_SECS: f32 = 0.25;
pub const RAT_HOLE_BLOCKED_SECS: f32 = 0.30;

/// Full bounding footprint of a maze reward pedestal (`18.0 × 18.0`).
pub const MAZE_REWARD_SIZE: Vec2 = Vec2 { x: 18.0, y: 18.0 };
/// Visual radius of the octagonal stepped dais.
pub const MAZE_REWARD_PEDESTAL_RADIUS: f32 = 8.5;
/// Collection / activation trigger radius around `pos` for the player rat.
pub const MAZE_REWARD_INTERACT_RADIUS: f32 = 11.0;
/// Pedestal is a low walkable floor dais so narrow maze ends never snag movement.
pub const MAZE_REWARD_BLOCKS_MOVEMENT: bool = false;
/// Celebratory opening animation duration (seconds).
pub const MAZE_REWARD_OPEN_SECS: f32 = 0.65;

/// Oriented rectangle hit test in an object's local `(u, v)` frame.
/// Use this in simulation code so trap triggers, kill zones, door slots, and
/// rat-hole clearances agree pixel-for-pixel with the drawn art.
#[inline]
pub fn local_box_contains(
    origin: Vec2,
    facing: Vec2,
    point: Vec2,
    u_min: f32,
    u_max: f32,
    half_v: f32,
) -> bool {
    let f = safe_dir(facing);
    let p = f.perp();
    let d = point - origin;
    let u = d.dot(f);
    let v = d.dot(p);
    u >= u_min && u <= u_max && v.abs() <= half_v
}

/// Returns true if an entity with collision `radius` can duck through a rat-hole.
#[inline]
pub fn fits_through_rat_hole(entity_radius: f32) -> bool {
    entity_radius <= RAT_HOLE_MAX_PASS_RADIUS
}

// ============================================================================
// 1. Sticky Trap (glue board)
// ============================================================================

/// Gameplay state machine for a floor glue board (`StickyTrapView`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickyTrapState {
    /// Fresh, glossy glue board lying flat on the floor.
    ///
    /// - **Duration**: Indefinite until stepped on by the player rat or a ground
    ///   enemy (Skitterer, Cat, Spitter, Brute; flying Bats pass over unaffected).
    /// - **Warning**: High-contrast ivory cardboard tray rim with ochre corner
    ///   markings and a continuously drifting diagonal specular sheen (`anim`)
    ///   across the amber adhesive pool.
    /// - **Active area**: Oriented rectangle `|u| <= 7.0`, `|v| <= 5.0` relative
    ///   to `pos` and `facing` (`STICKY_TRAP_PAD_HALF`). Does NOT block movement.
    /// - **Damage**: `0`.
    /// - **Counter**: Walk around it, or **Dash** directly across it (an active
    ///   dash clears the pad without sticking).
    Idle,

    /// A victim (player rat or ground enemy) is bogged down in the adhesive,
    /// pulling stretching glue strands as they struggle free.
    ///
    /// - **Duration**: `1.6s` for the player rat (`STICKY_TRAP_HOLD_PLAYER_SECS`),
    ///   `2.5s` for Skitterers/Cats/Spitters (`STICKY_TRAP_HOLD_SMALL_ENEMY_SECS`),
    ///   `1.5s` for Brutes (`STICKY_TRAP_HOLD_BRUTE_SECS`), or `0.8s` at 40%
    ///   slow on bosses. `progress` advances `0.0 -> 1.0` over the hold duration,
    ///   then transitions to `Used` as the strands snap.
    /// - **Warning / Readability**: Disturbed rippled glue pool with thick
    ///   glistening amber-ivory strands anchoring the board to `victim_pos`;
    ///   strands elongate, thin, vibrate, and snap into beads past `progress > 0.78`.
    /// - **Active area**: Affects the currently caught entity (does not double-root
    ///   a second victim while occupied; implementers may apply a mild 40% slow
    ///   to bystanders crossing the pad).
    /// - **Effect & Damage**: Reduces victim movement speed to `20%`
    ///   (`STICKY_TRAP_SLOW_MULT`) and deals `0` direct damage.
    /// - **Counter**: Pressing **Dash** immediately breaks the player free
    ///   (jumping `progress` to `1.0` -> `Used` and spending dash cooldown).
    ///   Conversely, players can lure fast Cats or tough Brutes onto idle glue
    ///   boards to pin them under sustained weapon fire.
    Stuck,

    /// Spent, torn, dusty glue board after a victim has broken free.
    ///
    /// - **Duration**: Permanent floor decal (or optional fade after `30.0s`).
    /// - **Visuals**: Scuffed/bent cardboard rim, matte gray-tan torn adhesive
    ///   with lint flecks and short curled stub strands — no wet gloss.
    /// - **Active area / Damage**: None (completely harmless).
    Used,
}

/// View data required to draw a sticky trap. `Copy` and simulation-independent.
#[derive(Clone, Copy, Debug)]
pub struct StickyTrapView {
    /// World-space center of the glue board.
    pub pos: Vec2,
    /// Unit direction along the board's long (`18px`) axis.
    pub facing: Vec2,
    /// Current trap state.
    pub state: StickyTrapState,
    /// Normalized progress `0.0..=1.0` through the current state (used in `Stuck`
    /// to stretch and snap strands).
    pub progress: f32,
    /// Monotonic clock in seconds for specular gloss drift and strand quiver.
    pub anim: f32,
    /// World-space position of the currently stuck victim (used in `Stuck`).
    /// If equal to `pos`, strands radiate gently from the board center.
    pub victim_pos: Vec2,
    /// Collision radius of the stuck victim (e.g. `3.0` rat, `6.5` Brute) so
    /// strands anchor cleanly onto its flanks.
    pub victim_radius: f32,
}

impl StickyTrapView {
    pub fn new(pos: Vec2, facing: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: StickyTrapState::Idle,
            progress: 0.0,
            anim: 0.0,
            victim_pos: pos,
            victim_radius: 3.0,
        }
    }
}

// ============================================================================
// 2. Mousetrap (wooden snap trap)
// ============================================================================

/// Gameplay state machine for a baited wooden snap mousetrap (`MousetrapView`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MousetrapState {
    /// Cocked and baited with a golden wedge of cheese on the `+facing` end.
    ///
    /// - **Duration**: Indefinite until any ground entity (player rat, Skitterer,
    ///   Cat, Spitter, or Brute; Bats fly over) enters the trigger zone.
    /// - **Warning**: Warm pine board, gleaming taut steel U-bar cocked back on
    ///   `-facing`, coiled torsion spring, and bright yellow cheese bait on `+facing`.
    /// - **Trigger area**: Circle of radius `4.5` (`MOUSETRAP_TRIGGER_RADIUS`)
    ///   centered at `pos + facing * 3.5` (`MOUSETRAP_TRIGGER_OFFSET_U`).
    ///   Does NOT block movement.
    /// - **Damage**: `0` while armed.
    /// - **Counter**: Walk around, or dash across the bait to intentionally
    ///   spring the trap behind you against pursuing enemies.
    Armed,

    /// Hairline pre-snap telegraph: the copper bait pedal has dipped and the
    /// cocked steel striker bar shudders violently before releasing.
    ///
    /// - **Duration**: `0.28s` (`MOUSETRAP_TRIGGER_SECS`), advancing `progress`
    ///   `0.0 -> 1.0`, then immediately entering `Snapping`.
    /// - **Warning**: High-frequency jitter of the cocked steel bar, bright
    ///   spring tension sparks, and a pulsing crimson-amber ground danger bracket
    ///   marking the exact `+facing` kill rectangle (`u in [-1.0, 9.5], |v| <= 6.0`).
    /// - **Damage**: `0` during warning (fair reaction window: at 58 px/s the rat
    ///   can walk ~16 px or dash clear; slow Brutes at 16 px/s cannot escape).
    /// - **Counter**: Step clear of the `+facing` kill rectangle or Dash.
    Triggered,

    /// The spring-steel striker bar whips 180° over the hinge and slams down
    /// onto the bait plate.
    ///
    /// - **Duration**: `0.20s` (`MOUSETRAP_SNAP_SECS`), advancing `progress`
    ///   `0.0 -> 1.0`, then transitioning to `Sprung`.
    /// - **Active hit window**: `progress` in `0.40..=0.65` (impact moment; each
    ///   overlapping entity is damaged at most once).
    /// - **Dangerous area**: Oriented rectangle `u in [-1.0, 9.5]`, `|v| <= 6.0`
    ///   relative to `pos` and `facing`.
    /// - **Damage**:
    ///   - Player rat: **`20` damage** (comparable to a Brute's `22` on 100 HP)
    ///     plus `0.25s` brief stagger/knockback.
    ///   - Ground enemies: **`80` damage** (one-shots Skitterers/Cats/Spitters,
    ///     devastates Brutes) plus `0.6s` stun/knockback.
    /// - **Counter**: Dash i-frames or exit the kill zone during `Triggered`.
    Snapping,

    /// Already sprung and resting flat; completely harmless.
    ///
    /// - **Duration**: Permanent floor prop (does not re-arm).
    /// - **Visuals**: Steel U-bar closed flat over `+facing`, relaxed spring,
    ///   cheese smashed into scattered crumbs, subdued contrast.
    /// - **Active area / Damage**: None.
    Sprung,
}

/// View data required to draw a wooden snap mousetrap. `Copy` and decoupled.
#[derive(Clone, Copy, Debug)]
pub struct MousetrapView {
    /// World-space center of the wooden baseboard.
    pub pos: Vec2,
    /// Unit direction pointing from the cocked rear (`-u`) toward the cheese
    /// bait and kill zone (`+u`).
    pub facing: Vec2,
    /// Current trap state.
    pub state: MousetrapState,
    /// Normalized progress `0.0..=1.0` through `Triggered` or `Snapping`.
    pub progress: f32,
    /// Monotonic clock in seconds for subtle metallic glint and jitter.
    pub anim: f32,
    /// Optional `0.0..=1.0` flash intensity (e.g. when a victim is struck).
    pub hit_flash: f32,
}

impl MousetrapView {
    pub fn new(pos: Vec2, facing: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: MousetrapState::Armed,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
        }
    }
}

// ============================================================================
// 3. One-Way Door (maze exit)
// ============================================================================

/// Gameplay state machine for a directional maze-exit one-way door
/// (`OneWayDoorView`).
///
/// The door spans a `24.0 px` wall slot (`10.0 px` thick along `facing`) with
/// two `3.0 px` steel jambs flanking an `18.0 px` clear passage (`|v| < 9.0`).
/// Passage is strictly **from `-facing` (maze interior) toward `+facing` (exit)**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneWayDoorState {
    /// Latched shut in a forward-angled V-wedge pointing toward `+facing`.
    ///
    /// - **Duration**: Indefinite until approached.
    /// - **Visual readability**: Two angled steel-and-glass saloon flaps meet in
    ///   a clear `>` wedge pointing `+facing`, backed by red-tipped steel stop
    ///   lugs on the `-facing` edge; a glowing `accent`-tinted floor light-arrow
    ///   and directional chevron strip on the threshold point strictly `+facing`.
    /// - **Movement blocking**:
    ///   - Side jambs (`|u| <= 5.0, |v| in [9.0, 12.0]`): always solid to all.
    ///   - Passage (`|u| <= 5.0, |v| < 9.0`): opens into `Opening` when the
    ///     player enters within `10.0 px` on the `-facing` side (`u in [-10.0, 0.0]`).
    ///     Acts as a **solid wall** to the player approaching from `+facing`
    ///     (triggering `Refused` on contact) and acts as a **solid wall to all
    ///     enemies** from both sides so the horde cannot enter the maze backward
    ///     or prop the exit open.
    /// - **Damage**: `0`.
    Closed,

    /// Spring-loaded saloon flaps swing outward into the `+facing` exit side as
    /// the rat passes through.
    ///
    /// - **Duration**: `0.24s` (`ONE_WAY_DOOR_OPEN_SECS`) to swing from `0°` to
    ///   `82°` (`progress` `0.0 -> 1.0`), remaining held at `progress = 1.0`
    ///   while the player's body overlaps the doorway (`|u| <= 8.0, |v| <= 9.0`),
    ///   then transitioning to `Closing` once the player clears into `u > 8.0`.
    /// - **Visuals**: Both leaves pivot forward toward `+facing` flush with the
    ///   jambs, flooding the threshold chevron with bright `accent` light.
    /// - **Movement blocking**: Passage is completely open to the player; still
    ///   blocks enemies approaching from `+facing`.
    /// - **Damage**: `0`.
    Opening,

    /// Flaps spring-return from wide open back to the closed V-latch after the
    /// rat exits.
    ///
    /// - **Duration**: `0.35s` (`ONE_WAY_DOOR_CLOSE_SECS`, `progress` `0.0 -> 1.0`)
    ///   with a crisp mechanical latch bounce around `progress = 0.85`, then
    ///   returns to `Closed`.
    /// - **Movement blocking**: Immediately blocks re-entry from `+facing` once
    ///   `progress >= 0.35` (preventing back-dashing into the exit).
    /// - **Damage**: `0`.
    Closing,

    /// Visibly refusing entry when pushed from the wrong (`+facing`) side.
    ///
    /// - **Duration**: `0.40s` (`ONE_WAY_DOOR_REFUSED_SECS`, `progress` `0.0 -> 1.0`),
    ///   then returns to `Closed`.
    /// - **Visuals**: The V-wedge leaves jam tightly against the backstop lugs
    ///   and rattle sharply without parting; a bright crimson-red denial bar,
    ///   crossed `X` emblem, and reject ripple flash across the `+facing` face.
    /// - **Movement blocking**: Fully solid along `u = +3.0`; pushes colliding
    ///   actor gently back toward `+facing`.
    /// - **Damage**: `0`.
    Refused,
}

/// View data required to draw a directional one-way maze exit door.
#[derive(Clone, Copy, Debug)]
pub struct OneWayDoorView {
    /// World-space center of the doorway slot.
    pub pos: Vec2,
    /// Unit vector pointing in the **allowed** travel direction (`-facing` -> `+facing`).
    pub facing: Vec2,
    /// Current door state.
    pub state: OneWayDoorState,
    /// Normalized progress `0.0..=1.0` through `Opening`, `Closing`, or `Refused`.
    pub progress: f32,
    /// Monotonic clock in seconds for the gentle idle floor-arrow pulse.
    pub anim: f32,
    /// Optional `0.0..=1.0` impact flash when rattled.
    pub hit_flash: f32,
}

impl OneWayDoorView {
    pub fn new(pos: Vec2, facing: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: OneWayDoorState::Closed,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
        }
    }
}

// ============================================================================
// 4. Rat-Hole (small arched gap in a wall)
// ============================================================================

/// Gameplay state machine for a rat-sized arched gap carved through a wall
/// (`RatHoleView`).
///
/// The wall segment is `14.0 px` thick (`u in [-7.0, 7.0]`) by `22.0 px` wide
/// (`v in [-11.0, 11.0]`). Two `7.0 px` masonry cheeks flank an `8.0 px` wide
/// arched tunnel (`|v| <= 4.0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RatHoleState {
    /// Quiet, inviting dark arched hole with gnawed pale rim and heavy big-enemy
    /// claw gouges scarred into the wide outer wall cheeks.
    ///
    /// - **Duration**: Indefinite.
    /// - **Visual readability**: Tight `8 px` dark arch (visibly rat-width and
    ///   half a Brute's width), chipped gnawed inner lip, tiny paw-print dust
    ///   trail on the floor leading straight through, and pale diagonal scrape
    ///   scars on the outer wall cheeks at Brute/Cat shoulder width (`|v| in 5..10`).
    /// - **Movement blocking**:
    ///   - Outer wall cheeks (`|u| <= 7.0, |v| in [4.0, 11.0]`): solid to ALL.
    ///   - Center tunnel (`|u| <= 7.0, |v| < 4.0`): **PASSABLE** for any entity
    ///     with `radius <= 3.2` (`RAT_HOLE_MAX_PASS_RADIUS` — Player Rat `3.0`,
    ///     Skitterer `2.5`, Bat `2.0`, Spitter `3.0`), and **SOLID** for any
    ///     entity with `radius > 3.2` (Cat `4.0`, Brute `6.5`, Bosses `14.0+`).
    /// - **Damage**: `0`.
    /// - **Tactical role**: Lets the rat duck into safe pockets or kite Brutes
    ///   and Bosses around walls while forcing small swarmers to funnel single-file.
    Idle,

    /// A small creature (the rat or a small swarmer) is ducking through the arch.
    ///
    /// - **Duration**: `0.25s` (`RAT_HOLE_SQUEEZE_SECS`) or held active while
    ///   a passable entity's center is inside `|u| <= 7.0, |v| <= 4.0`.
    /// - **Visuals**: Plaster/dust motes puff outward from both tunnel mouths,
    ///   subtle highlight warms the gnawed arch rim, and `draw_rat_hole` paints
    ///   the arch crown *over* the entity so it visibly ducks underneath.
    /// - **Damage**: `0`.
    Squeezing,

    /// A large enemy (`radius > 3.2`: Cat, Brute, or Boss) just collided with
    /// the too-narrow shoulders trying to follow the rat.
    ///
    /// - **Duration**: `0.30s` (`RAT_HOLE_BLOCKED_SECS`, `progress` `0.0 -> 1.0`),
    ///   then returns to `Idle`.
    /// - **Visuals**: Outer wall cheeks vibrate briefly; pale masonry chips and
    ///   fresh scratch sparks fly off the wide shoulder zones (`|v| in 4.5..9.5`)
    ///   with a brief warm-red/amber impact flash on the blocked cheeks while
    ///   the center arch stays open and dark.
    /// - **Movement blocking**: Solid against the large enemy; implementers should
    ///   apply a small pushback (`3.0 px` along wall normal) so large enemies
    ///   re-path via the flow field instead of jittering in place.
    /// - **Damage**: `0`.
    Blocked,
}

/// View data required to draw a rat-hole wall gap. `Copy` and decoupled.
#[derive(Clone, Copy, Debug)]
pub struct RatHoleView {
    /// World-space center of the arched gap inside the wall.
    pub pos: Vec2,
    /// Unit vector perpendicular to the wall (direction of travel through the
    /// hole). Holes are passable in BOTH `+facing` and `-facing`.
    pub facing: Vec2,
    /// Current state.
    pub state: RatHoleState,
    /// Normalized progress `0.0..=1.0` through `Squeezing` or `Blocked`.
    pub progress: f32,
    /// Monotonic clock in seconds for subtle ambient draft dust.
    pub anim: f32,
    /// Optional base wall color (e.g. `Some(level.palette.wall)`). When `None`,
    /// derives a weathered masonry tone harmonized with `accent`.
    pub wall_tint: Option<Rgb>,
}

impl RatHoleView {
    pub fn new(pos: Vec2, facing: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: RatHoleState::Idle,
            progress: 0.0,
            anim: 0.0,
            wall_tint: None,
        }
    }
}

// ============================================================================
// 5. Maze Reward (powerup reliquary pedestal)
// ============================================================================

/// Gameplay state machine for the maze-end powerup pedestal (`MazeRewardView`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MazeRewardState {
    /// Unopened gilded powerup cache gleaming atop an octagonal stone-and-brass
    /// pedestal at the end of a maze.
    ///
    /// - **Duration**: Indefinite until the player rat walks within
    ///   `11.0 px` (`MAZE_REWARD_INTERACT_RADIUS`) of `pos`. Enemies cannot open it.
    /// - **Visual readability**: Warm golden-amber & `accent` floor bloom,
    ///   stepped iron-and-brass dais with glowing cardinal conduits, ornate
    ///   floating/cradled golden reliquary with pulsing crystal core and orbiting
    ///   shimmer sparkles (`anim`). Readable from half a screen away.
    /// - **Movement blocking**: Does NOT block movement (`MAZE_REWARD_BLOCKS_MOVEMENT = false`).
    /// - **Damage**: `0`.
    Idle,

    /// Celebratory opening sequence triggered when the player claims the cache.
    ///
    /// - **Duration**: `0.65s` (`MAZE_REWARD_OPEN_SECS`, `progress` `0.0 -> 1.0`),
    ///   then transitions to `Opened`.
    /// - **Gameplay reward**: Grants an immediate maze prize (e.g. free 1-of-3
    ///   Level-Up upgrade OR guaranteed high-tier powerup bundle: Heal 35% +
    ///   XP Magnet + bonus upgrade) AND grants `0.8s` of grace invulnerability
    ///   frames so looting in a tight maze never feels unfair.
    /// - **Visuals**: Brass lid halves hinge wide open, a bright golden-white
    ///   light pillar surges upward, and expanding additive halo rings sweep
    ///   across the floor.
    /// - **Damage**: `0`.
    Opening,

    /// Looted, open pedestal remaining as a conquered-maze landmark.
    ///
    /// - **Duration**: Permanent.
    /// - **Visuals**: Open brass flaps revealing an empty recessed velvet cradle,
    ///   conduits dimmed to low cool standby, no bright halo or orbiting sparks.
    /// - **Active area / Damage**: None.
    Opened,
}

/// View data required to draw a maze-end powerup reward pedestal.
#[derive(Clone, Copy, Debug)]
pub struct MazeRewardView {
    /// World-space center of the pedestal.
    pub pos: Vec2,
    /// Unit orientation vector (pedestal is mostly radial; facing orients the
    /// clasp/lid hinge so it can face the maze approach).
    pub facing: Vec2,
    /// Current reward state.
    pub state: MazeRewardState,
    /// Normalized progress `0.0..=1.0` through `Opening`.
    pub progress: f32,
    /// Monotonic clock in seconds for continuous idle shimmer and floating bob.
    pub anim: f32,
    /// Optional tint for the powerup gem core (defaults to radiant gold blended
    /// lightly with `accent`).
    pub reward_tint: Option<Rgb>,
}

impl MazeRewardView {
    pub fn new(pos: Vec2, facing: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: MazeRewardState::Idle,
            progress: 0.0,
            anim: 0.0,
            reward_tint: None,
        }
    }
}

// ============================================================================
// Shared local-frame & zero-allocation pixel helpers
// ============================================================================

#[inline]
fn safe_dir(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-4 {
        v.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

#[inline]
fn clamp01(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy)]
struct Frame {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
}

impl Frame {
    #[inline]
    fn new(world_pos: Vec2, facing: Vec2, cam: Vec2) -> Self {
        let f = safe_dir(facing);
        let p = f.perp();
        Self {
            pos: world_pos - cam,
            f,
            p,
            fl: f.dot(LIGHT),
            pl: p.dot(LIGHT),
        }
    }

    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.pos + self.f * u + self.p * v
    }

    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        let q = self.place(u, v);
        (q.x.round() as i32, q.y.round() as i32)
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn local_f(&self, sx: f32, sy: f32) -> (f32, f32) {
        let dx = sx - self.pos.x;
        let dy = sy - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    #[inline]
    fn bbox_pb(&self, pb: &PixelBuffer, reach: f32) -> Option<(i32, i32, i32, i32)> {
        if pb.w == 0 || pb.h == 0 || !self.pos.x.is_finite() || !self.pos.y.is_finite() {
            return None;
        }
        let rr = reach.ceil() as i32;
        let cx = self.pos.x.round() as i32;
        let cy = self.pos.y.round() as i32;
        let x0 = (cx - rr).max(0);
        let x1 = (cx + rr).min(pb.w as i32 - 1);
        let y0 = (cy - rr).max(0);
        let y1 = (cy + rr).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            None
        } else {
            Some((x0, x1, y0, y1))
        }
    }
}

#[inline]
fn hash2(a: i32, b: i32) -> u32 {
    let mut h = (a as u32).wrapping_mul(0x9E37_79B1) ^ (b as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h
}

#[inline]
fn hash_f32(a: i32, b: i32) -> f32 {
    (hash2(a, b) & 0xFFFF) as f32 / 65535.0
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

#[inline]
fn add_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, factor: f32) {
    if factor <= 0.0 {
        return;
    }
    pb.plot_add(x, y, scale(c, factor));
}

#[inline]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.65 {
        mix(shadow, mid, l / 0.65)
    } else {
        mix(mid, hi, (l - 0.65) / 0.50)
    }
}

/// Signed distance from `(x, y)` to a segment `(ax, ay)-(bx, by)`, plus parametric `t`.
#[inline]
fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> (f32, f32) {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 1e-6 {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = px - (ax + vx * t);
    let dy = py - (ay + vy * t);
    ((dx * dx + dy * dy).sqrt(), t)
}

/// Signed distance to an axis-aligned box centered at `(0, 0)` with half-extents `(hx, hy)`.
#[inline]
fn sd_box(x: f32, y: f32, hx: f32, hy: f32) -> f32 {
    let qx = x.abs() - hx;
    let qy = y.abs() - hy;
    let out = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    out + inside
}

/// Signed distance to a regular octagon of radius `r` (apothem `r`).
#[inline]
fn sd_octagon(x: f32, y: f32, r: f32) -> f32 {
    let ax = x.abs();
    let ay = y.abs();
    let d_axis = ax.max(ay) - r;
    let d_diag = (ax + ay) * 0.70710677 - r * 1.08;
    d_axis.max(d_diag)
}

/// Soft additive glow disc centered at screen point `(cx, cy)`.
fn glow_disc(pb: &mut PixelBuffer, cx: f32, cy: f32, radius: f32, color: Rgb, intensity: f32) {
    if intensity <= 0.005 || radius <= 0.5 || !cx.is_finite() || !cy.is_finite() {
        return;
    }
    let r_i = radius.ceil() as i32;
    let x0 = (cx.floor() as i32 - r_i).max(0);
    let x1 = (cx.ceil() as i32 + r_i).min(pb.w as i32 - 1);
    let y0 = (cy.floor() as i32 - r_i).max(0);
    let y1 = (cy.ceil() as i32 + r_i).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let inv_r = 1.0 / radius;
    for y in y0..=y1 {
        let dy = y as f32 - cy;
        for x in x0..=x1 {
            let dx = x as f32 - cx;
            let d = (dx * dx + dy * dy).sqrt() * inv_r;
            if d < 1.0 {
                let fall = (1.0 - d) * (1.0 - d);
                pb.plot_add(x, y, scale(color, fall * intensity));
            }
        }
    }
}

/// Crisp anti-aliased-width segment drawn with blend/highlight, zero allocation.
#[allow(clippy::too_many_arguments)]
fn draw_strand_curve(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    sag: f32,
    width: f32,
    core: Rgb,
    rim: Rgb,
    alpha: f32,
) {
    if alpha <= 0.01 || !a.x.is_finite() || !b.x.is_finite() {
        return;
    }
    let dist = a.dist(b);
    if dist < 0.5 {
        return;
    }
    let steps = (dist * 2.2).ceil().clamp(4.0, 48.0) as i32;
    let dir = (b - a) * (1.0 / dist);
    let perp = dir.perp();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let arch = 4.0 * t * (1.0 - t);
        let p = a + (b - a) * t + perp * (sag * arch);
        // Strands neck down thinner in the middle under tension.
        let thick = width * (1.0 - 0.52 * arch);
        let sx = p.x.round() as i32;
        let sy = p.y.round() as i32;
        if thick > 0.72 {
            blend_px(pb, sx + 1, sy + 1, rim, alpha * 0.45);
            blend_px(pb, sx, sy, core, alpha);
            if thick > 1.15 {
                blend_px(pb, sx - 1, sy, mix(core, (255, 252, 236), 0.35), alpha * 0.65);
            }
        } else {
            blend_px(pb, sx, sy, mix(core, rim, 0.25), alpha * 0.85);
        }
    }
}

/// Draw a thick capsule-like bar in screen space with metallic bevel + shadow.
#[allow(clippy::too_many_arguments)]
fn draw_metal_bar(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    radius: f32,
    shadow_ofs: Vec2,
    base: Rgb,
    hi: Rgb,
    outline: Rgb,
) {
    if !a.x.is_finite() || !b.x.is_finite() {
        return;
    }
    let pad = (radius + 2.5 + shadow_ofs.len()).ceil() as i32;
    let x0 = (a.x.min(b.x).floor() as i32 - pad).max(0);
    let x1 = (a.x.max(b.x).ceil() as i32 + pad).min(pb.w as i32 - 1);
    let y0 = (a.y.min(b.y).floor() as i32 - pad).max(0);
    let y1 = (a.y.max(b.y).ceil() as i32 + pad).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let fx = x as f32;
            let fy = y as f32;
            let (d, _) = dist_to_segment(fx, fy, a.x, a.y, b.x, b.y);
            if d <= radius {
                // Top-left specular along cylinder cross-section
                let (ds_tl, _) = dist_to_segment(fx - 0.45, fy - 0.45, a.x, a.y, b.x, b.y);
                let c = if ds_tl > d + 0.15 {
                    hi
                } else if ds_tl < d - 0.18 {
                    scale(base, 0.58)
                } else {
                    base
                };
                pb.plot(x, y, c);
            } else if d <= radius + 0.85 {
                blend_px(pb, x, y, outline, 0.85);
            } else {
                let (d_sh, _) = dist_to_segment(
                    fx - shadow_ofs.x,
                    fy - shadow_ofs.y,
                    a.x,
                    a.y,
                    b.x,
                    b.y,
                );
                if d_sh <= radius + 0.45 {
                    darken_px(pb, x, y, 0.42);
                }
            }
        }
    }
}

// ============================================================================
// 1. Sticky Trap drawing
// ============================================================================

/// **Ground layer** for a sticky trap (draw after floor/stains, BEFORE entities).
///
/// Renders the grounded drop shadow, folded cardboard tray with pest-warning
/// corner ticks, the amber adhesive pool (glossy drifting sheen in `Idle`,
/// disturbed ripples & base tracks in `Stuck`, matte dusty torn glue in `Used`).
pub fn draw_sticky_trap_ground(
    pb: &mut PixelBuffer,
    view: &StickyTrapView,
    cam: Vec2,
    _accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 16.0) else {
        return;
    };

    let hx = STICKY_TRAP_SIZE.x * 0.5; // 9.0
    let hy = STICKY_TRAP_SIZE.y * 0.5; // 7.0
    let px = STICKY_TRAP_PAD_HALF.x; // 7.0
    let py = STICKY_TRAP_PAD_HALF.y; // 5.0
    let prog = clamp01(view.progress);

    let (tray_base, tray_hi, tray_lo, outline) = match view.state {
        StickyTrapState::Used => ((124, 114, 98), (154, 142, 122), (68, 60, 50), (26, 22, 18)),
        _ => ((196, 184, 154), (236, 226, 198), (112, 100, 80), (30, 26, 20)),
    };

    let (glue_dark, glue_mid, glue_hi) = match view.state {
        StickyTrapState::Idle => ((124, 90, 38), (186, 146, 66), (254, 246, 206)),
        StickyTrapState::Stuck => ((92, 60, 22), (154, 114, 44), (242, 224, 168)),
        StickyTrapState::Used => ((64, 56, 46), (92, 82, 68), (124, 114, 98)),
    };

    // Victim pull direction in local coordinates (used to furrow the glue in Stuck).
    let pull_world = view.victim_pos - view.pos;
    let pull_u = pull_world.dot(fr.f);
    let pull_v = pull_world.dot(fr.p);

    // Slow drifting diagonal gloss band across the fresh adhesive.
    let sheen_phase = (view.anim * 1.35).sin() * 4.8;
    let seed = hash2(view.pos.x.round() as i32, view.pos.y.round() as i32);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);

            // Visibly peeled/bent corner flap when Used!
            let peeled_corner = view.state == StickyTrapState::Used && u + v > (hx + hy - 4.4);
            if peeled_corner {
                // Dark floor shadow under the curled-up cardboard dog-ear.
                if sd_box(u, v, hx, hy) <= 0.3 {
                    darken_px(pb, x, y, 0.45);
                }
                continue;
            }

            let d_outer = sd_box(u, v, hx, hy);
            if d_outer > 2.2 {
                continue;
            }

            // Soft contact shadow cast down-right.
            if d_outer > 0.75 {
                let (su, sv) = fr.local_f(x as f32 - 1.3, y as f32 - 1.3);
                let ds = sd_box(su, sv, hx, hy);
                if ds <= 0.35 {
                    darken_px(pb, x, y, 0.42);
                }
                continue;
            }

            // Crisp dark outer silhouette rim.
            if d_outer > -0.25 {
                blend_px(pb, x, y, outline, 0.90);
                continue;
            }

            // Curled-back underside triangle right next to the peeled corner in Used!
            if view.state == StickyTrapState::Used
                && u + v > (hx + hy - 6.6)
                && u > hx - 4.2
                && v > hy - 4.2
            {
                let crease = (u + v - (hx + hy - 6.6)).abs() < 0.75;
                let flap_col = if crease {
                    (58, 50, 42)
                } else {
                    (168, 156, 134)
                };
                pb.plot(x, y, flap_col);
                continue;
            }

            let d_pad = sd_box(u, v, px, py);
            if d_pad > 0.0 {
                // Folded cardboard tray lip.
                let edge_lit = if u < -px || v < -py {
                    fr.lit(if u < -px { -1.0 } else { 0.0 }, if v < -py { -1.0 } else { 0.0 })
                } else {
                    fr.lit(if u > px { 1.0 } else { 0.0 }, if v > py { 1.0 } else { 0.0 })
                };
                let mut col = shade3(tray_lo, tray_base, tray_hi, 0.68 + 0.35 * edge_lit);

                // Subtle pest-warning ochre/crimson corner tabs on the tray border.
                let corner = u.abs() > hx - 2.3 && v.abs() > hy - 2.2;
                if corner {
                    let warn = if view.state == StickyTrapState::Used {
                        (104, 78, 52)
                    } else {
                        (182, 92, 46)
                    };
                    col = mix(col, warn, 0.72);
                } else if (u.abs() - (px + 0.4)).abs() < 0.45 && v.abs() <= py + 0.4
                    || (v.abs() - (py + 0.4)).abs() < 0.45 && u.abs() <= px + 0.4
                {
                    // Inner tray crease shadow
                    col = scale(col, 0.62);
                }

                pb.plot(x, y, col);
                continue;
            }

            // Inside the adhesive glue pool!
            let iu = (u * 1.4).round() as i32;
            let iv = (v * 1.4).round() as i32;
            let grain = hash_f32(iu + (seed as i32 & 31), iv + ((seed >> 5) as i32 & 31));

            // Meniscus edge darkness where glue pools against the tray walls.
            let edge_depth = (-d_pad / 1.8).clamp(0.0, 1.0);
            let top_left_ao = if u < -px + 1.2 || v < -py + 1.2 {
                0.76
            } else {
                1.0
            };
            let lum_base = (0.48 + 0.30 * edge_depth + 0.14 * fr.lit(u / px, v / py)) * top_left_ao;

            let mut col = match view.state {
                StickyTrapState::Idle => {
                    let mut c = shade3(glue_dark, glue_mid, glue_hi, lum_base);
                    // Wet diagonal specular sheen drifting smoothly across the glossy glue.
                    let diag = (u * 0.72 + v * 0.85) - sheen_phase;
                    let band1 = (1.0 - (diag.abs() / 1.55)).clamp(0.0, 1.0);
                    let band2 = (1.0 - ((diag + 3.4).abs() / 0.85)).clamp(0.0, 1.0) * 0.45;
                    let spec = (band1 * band1 + band2).clamp(0.0, 1.0);
                    c = mix(c, glue_hi, spec * 0.78);
                    if band1 > 0.76 && edge_depth > 0.35 {
                        c = mix(c, (255, 255, 246), 0.82);
                    }
                    // Syrupy curved meniscus highlight tucked cleanly inside the top-left pool corner.
                    let near_tl = u < -px + 2.2 && v < -py + 2.2;
                    let on_curve = u > -px + 0.8 && v > -py + 0.8;
                    if near_tl && on_curve && (u + px < 1.7 || v + py < 1.7) {
                        c = mix(c, (255, 252, 232), 0.56);
                    }
                    // Tiny trapped air bubble highlights in the amber glue.
                    if grain > 0.94 && edge_depth > 0.4 {
                        c = mix(c, (255, 248, 220), 0.58);
                    }
                    c
                }
                StickyTrapState::Stuck => {
                    // Disturbed, churned caramel-amber ripples pulling toward victim_pos!
                    let du = u - pull_u * 0.25;
                    let dv = v - pull_v * 0.25;
                    let dist = (du * du + dv * dv).sqrt();
                    let ripple = (dist * 2.2 - prog * 6.0).sin() * 0.5 + 0.5;
                    let mut c = shade3(glue_dark, glue_mid, glue_hi, lum_base * 0.88 + 0.20 * ripple);
                    // Deep stressed paw/foot trough right where the victim churned the adhesive.
                    if dist < 2.8 {
                        c = scale(c, 0.64);
                    }
                    // Wet golden ridge around the churn crater.
                    if (dist - 3.3).abs() < 0.75 {
                        c = mix(c, glue_hi, 0.58);
                    }
                    c
                }
                StickyTrapState::Used => {
                    // Matte, torn, dusty glue residue — zero bright wet gloss.
                    let mut c = shade3(glue_dark, glue_mid, glue_hi, lum_base * 0.84);
                    // Diagonal ripped-up cardboard scar across the middle.
                    let rip = ((u * 0.55 - v * 0.75).abs() < 1.35) && grain > 0.38;
                    if rip {
                        c = mix(c, (158, 146, 126), 0.64);
                    } else if grain < 0.16 {
                        c = scale(c, 0.58);
                    }
                    c
                }
            };

            // Slight wet rim highlight on bottom-right inner wall (bounced light).
            if view.state != StickyTrapState::Used && (u > px - 1.1 || v > py - 1.1) {
                col = mix(col, glue_hi, 0.26);
            }

            pb.plot(x, y, col);
        }
    }

    // In Used state, add 2 tiny dried snapped glue stub tags on the rim.
    if view.state == StickyTrapState::Used {
        for &(su, sv) in &[(-2.2, -2.5), (1.8, 1.6)] {
            let a = fr.place(su, sv);
            let b = fr.place(su + 2.2, sv - 1.1);
            draw_strand_curve(pb, a, b, 0.6, 0.7, (148, 138, 118), (48, 42, 34), 0.78);
        }
    }

    // In Stuck state, draw rich shadowed base glue strings bridging off the board
    // over the dark floor toward the struggling victim.
    if view.state == StickyTrapState::Stuck {
        let vp = sticky_effective_victim(view, &fr);
        let fade = (1.0 - (prog - 0.85).max(0.0) / 0.15).clamp(0.0, 1.0);
        for &(au, av, sag) in &[(-4.8, -3.6, 1.6), (-4.2, 3.6, -1.5), (1.5, 0.0, 0.8)] {
            let anchor = fr.place(au, av);
            draw_strand_curve(
                pb,
                anchor,
                vp,
                sag * (1.0 - prog * 0.45),
                1.35,
                (238, 220, 152),
                (58, 38, 14),
                0.90 * fade,
            );
        }
    }
}

#[inline]
fn sticky_effective_victim(view: &StickyTrapView, fr: &Frame) -> Vec2 {
    let d = view.victim_pos - view.pos;
    let prog = clamp01(view.progress);
    if d.len_sq() > 1.0 {
        // Screen-space victim position.
        fr.pos + d
    } else {
        // Fallback struggling pull along +facing if caller didn't offset victim_pos.
        let stretch = 4.0 + prog * 9.0;
        let wobble = (view.anim * 16.0).sin() * 1.4;
        fr.place(stretch, wobble)
    }
}

/// **Foreground / overlay layer** for a sticky trap (draw AFTER entities).
///
/// In `StickyTrapState::Stuck`, draws high-contrast glistening amber-ivory
/// taffy strands stretching from board anchors across the dark floor onto the
/// struggling victim's flanks, complete with dark amber shadow rims and glossy
/// stretched droplets that thin and snap near `progress -> 1.0`.
/// No-op in `Idle` and `Used` (fast early-out).
pub fn draw_sticky_trap(
    pb: &mut PixelBuffer,
    view: &StickyTrapView,
    cam: Vec2,
    _accent: Rgb,
) {
    if view.state != StickyTrapState::Stuck {
        return;
    }
    let fr = Frame::new(view.pos, view.facing, cam);
    if fr.bbox_pb(pb, 32.0).is_none() {
        return;
    }

    let prog = clamp01(view.progress);
    let vp = sticky_effective_victim(view, &fr);
    let vr = view.victim_radius.clamp(2.0, 9.0);

    let to_v = vp - fr.pos;
    let dir = safe_dir(to_v);
    let side = dir.perp();

    let strand_hi: Rgb = (255, 252, 224);
    let strand_mid: Rgb = (242, 214, 128);
    let strand_rim: Rgb = (52, 34, 12);

    // 5 taut foreground strands anchored across the glue pool -> victim flanks/paws.
    let anchors: [(f32, f32, f32, f32); 5] = [
        (-5.2, -3.6, -0.82, 0.76),
        (-4.5, 3.8, 0.80, 0.84),
        (0.8, -3.5, -0.50, 0.90),
        (1.2, 3.4, 0.55, 0.96),
        (4.5, 0.0, 0.05, 1.00),
    ];

    for (idx, &(au, av, flank_k, snap_at)) in anchors.iter().enumerate() {
        let a = fr.place(au, av);
        let quiver = (view.anim * (19.0 + idx as f32 * 5.0) + idx as f32 * 1.9).sin()
            * (0.55 + 0.95 * prog);
        // Anchor onto the victim's trailing haunches/flanks so its face stays readable!
        let target = vp + side * (flank_k * vr * 0.72) - dir * (vr * 0.52);

        if prog < snap_at {
            let tension = (prog / snap_at).clamp(0.0, 1.0);
            let width = (1.55 - 0.72 * tension).max(0.65);
            let col = mix(strand_mid, strand_hi, 0.55 + 0.45 * tension);
            // Crisp shadowed taffy filament
            draw_strand_curve(pb, a, target, quiver, width, col, strand_rim, 0.96);
            // Bright glossy midpoint bead hanging on the stretched strand!
            let mid = a + (target - a) * 0.52 + safe_dir(target - a).perp() * (quiver * 0.85);
            let mx = mid.x.round() as i32;
            let my = mid.y.round() as i32;
            darken_px(pb, mx + 1, my + 1, 0.42);
            blend_px(pb, mx, my, (255, 255, 240), 0.96);
            // Anchor droplet grabbing the victim's body
            let tx = target.x.round() as i32;
            let ty = target.y.round() as i32;
            blend_px(pb, tx, ty, (255, 250, 224), 0.94);
        } else {
            // Snapped! Recoiling elastic droplets whipping back toward board & victim.
            let since = ((prog - snap_at) / (1.0 - snap_at).max(0.05)).clamp(0.0, 1.0);
            let p_board = a + (target - a) * (0.46 * (1.0 - since));
            let p_vic = target + (a - target) * (0.38 * (1.0 - since));
            let alpha = (1.0 - since) * 0.95;
            blend_px(pb, p_board.x.round() as i32, p_board.y.round() as i32, strand_hi, alpha);
            blend_px(pb, p_vic.x.round() as i32, p_vic.y.round() as i32, strand_hi, alpha);
        }
    }
}

// ============================================================================
// 2. Mousetrap drawing
// ============================================================================

#[inline]
fn mousetrap_jitter(view: &MousetrapView) -> Vec2 {
    let prog = clamp01(view.progress);
    match view.state {
        MousetrapState::Triggered => {
            let amp = 0.25 + 0.65 * prog;
            let s = (view.anim * 68.0 + prog * 42.0).sin();
            safe_dir(view.facing).perp() * (s * amp)
        }
        MousetrapState::Snapping if (0.42..=0.72).contains(&prog) => {
            let decay = 1.0 - (prog - 0.42) / 0.30;
            safe_dir(view.facing) * (-0.75 * decay)
        }
        _ => Vec2::ZERO,
    }
}

/// **Ground layer** for a mousetrap (draw after floor/stains, BEFORE entities).
///
/// Renders the grounded drop shadow, beveled warm pine baseboard with wood
/// grain, the pulsing red-amber kill-zone telegraph during `Triggered`, and
/// the snap shockwave ring during `Snapping`.
pub fn draw_mousetrap_ground(
    pb: &mut PixelBuffer,
    view: &MousetrapView,
    cam: Vec2,
    _accent: Rgb,
) {
    let prog = clamp01(view.progress);
    let fr = Frame::new(view.pos + mousetrap_jitter(view), view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 16.0) else {
        return;
    };

    let hx = MOUSETRAP_SIZE.x * 0.5; // 9.0
    let hy = MOUSETRAP_SIZE.y * 0.5; // 5.5

    // 1. Beveled wooden baseboard + drop shadow.
    let wood_shadow: Rgb = if view.state == MousetrapState::Sprung {
        (68, 48, 30)
    } else {
        (82, 54, 30)
    };
    let wood_mid: Rgb = if view.state == MousetrapState::Sprung {
        (124, 90, 56)
    } else {
        (158, 112, 66)
    };
    let wood_hi: Rgb = if view.state == MousetrapState::Sprung {
        (166, 128, 86)
    } else {
        (214, 168, 108)
    };
    let outline: Rgb = (24, 16, 10);

    let seed = hash2(view.pos.x.round() as i32, view.pos.y.round() as i32);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = sd_box(u, v, hx, hy);
            if d > 2.4 {
                continue;
            }
            if d > 0.75 {
                let (su, sv) = fr.local_f(x as f32 - 1.5, y as f32 - 1.6);
                if sd_box(su, sv, hx, hy) <= 0.45 {
                    darken_px(pb, x, y, 0.48);
                }
                continue;
            }
            if d > -0.25 {
                blend_px(pb, x, y, outline, 0.92);
                continue;
            }

            // Wood grain streaks running lengthwise along `u`.
            let grain_row = ((v + 6.0) * 1.65).floor() as i32;
            let grain = (hash_f32(grain_row + (seed as i32 & 15), 3) - 0.5) * 0.16;
            let knot = (u * 0.5 + (seed & 7) as f32).sin() * 0.04;

            // Bevel lighting along top-left edges.
            let near_u = (hx - u.abs()).min(1.4);
            let near_v = (hy - v.abs()).min(1.4);
            let bevel_lit = if near_u < 1.1 || near_v < 1.1 {
                let nu = if near_u < near_v { u.signum() } else { 0.0 };
                let nv = if near_v <= near_u { v.signum() } else { 0.0 };
                0.35 * fr.lit(nu, nv)
            } else {
                0.0
            };

            let mut col = shade3(
                wood_shadow,
                wood_mid,
                wood_hi,
                0.66 + grain + knot + bevel_lit + 0.10 * fr.lit(u / hx, v / hy),
            );

            // Subtle dark staple/impact dent on the front half when Sprung.
            if view.state == MousetrapState::Sprung && (u - 6.8).abs() < 0.7 && v.abs() < 4.0 {
                col = scale(col, 0.64);
            }

            // Subtle warm impact flash on the +u strike zone only (keeps wood rich!).
            if view.hit_flash > 0.01 && u > 0.0 {
                col = mix(col, (255, 224, 162), clamp01(view.hit_flash) * 0.30);
            }

            pb.plot(x, y, col);
        }
    }

    // 2. Triggered danger telegraph bracket drawn OVER the board & extending
    //    outside the +u kill zone so the warning reads immediately!
    if view.state == MousetrapState::Triggered {
        let pulse = 0.62 + 0.38 * (prog * 18.0 + view.anim * 24.0).sin().abs();
        let warn_col: Rgb = mix((245, 58, 42), (255, 198, 72), prog * 0.45);
        let ku0 = MOUSETRAP_KILL_U_MIN;
        let ku1 = MOUSETRAP_KILL_U_MAX + 1.2;
        let kv = MOUSETRAP_KILL_HALF_V + 1.2;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = fr.local(x, y);
                let in_outer = u >= ku0 && u <= ku1 && v.abs() <= kv;
                if !in_outer {
                    continue;
                }
                let on_u_edge = (u - ku0).abs() < 0.85 || (u - ku1).abs() < 0.85;
                let on_v_edge = (v.abs() - kv).abs() < 0.85;
                // Crisp L-shaped danger corner brackets + dashed kill-zone perimeter!
                let near_corner = (u - ku0).abs() < 3.0
                    || (u - ku1).abs() < 3.2
                    || (v.abs() - kv).abs() < 2.2;
                if (on_u_edge || on_v_edge) && near_corner {
                    blend_px(pb, x, y, warn_col, 0.92 * pulse);
                } else if u >= 0.0 && u <= hx && v.abs() <= hy {
                    // Pulsing crimson hazard tint across the +u strike half
                    blend_px(pb, x, y, (235, 56, 38), (0.20 + 0.24 * prog) * pulse);
                }
            }
        }
    }

    // 3. Impact shockwave ring on Snapping slam (drawn over floor + board rim)!
    if view.state == MousetrapState::Snapping && prog >= 0.40 {
        let imp = ((prog - 0.40) / 0.60).clamp(0.0, 1.0);
        let impact_c = fr.place(4.8, 0.0);
        let ring_r = 3.5 + imp * 10.5;
        let fade = (1.0 - imp) * (1.0 - imp);
        glow_disc(pb, impact_c.x, impact_c.y, 8.5, (255, 196, 98), 0.70 * fade);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - impact_c.x;
                let dy = y as f32 - impact_c.y;
                let d = (dx * dx + dy * dy).sqrt();
                if (d - ring_r).abs() < 1.0 {
                    add_px(pb, x, y, (255, 228, 152), 0.92 * fade);
                }
            }
        }
    }
}

/// **Foreground / overlay layer** for a mousetrap (draw AFTER entities).
///
/// Renders the central steel torsion spring coil, copper trigger pedal, golden
/// cheese wedge (or smashed cheese crumbs when sprung), holding lock rod, and
/// the gleaming spring-steel striker U-bar (cocked at `-u`, quivering in
/// `Triggered`, whipping 180° overhead with motion streaks in `Snapping`, or
/// slammed shut across `+u` in `Sprung`).
pub fn draw_mousetrap(
    pb: &mut PixelBuffer,
    view: &MousetrapView,
    cam: Vec2,
    _accent: Rgb,
) {
    let prog = clamp01(view.progress);
    let fr = Frame::new(view.pos + mousetrap_jitter(view), view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 16.0) else {
        return;
    };

    // 1. Copper bait pedal at +u (u = 2.6..7.2, |v| <= 2.4)
    let pedal_dip = matches!(
        view.state,
        MousetrapState::Triggered | MousetrapState::Snapping | MousetrapState::Sprung
    );
    let cop_shadow: Rgb = (92, 46, 22);
    let cop_mid: Rgb = (176, 102, 52);
    let cop_hi: Rgb = (238, 174, 112);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let dp = sd_box(u - 4.7, v, 2.3, 2.2);
            if dp <= 0.0 {
                let lum = if pedal_dip { 0.52 } else { 0.72 }
                    + 0.25 * fr.lit((u - 4.7) / 2.3, v / 2.2);
                let col = shade3(cop_shadow, cop_mid, cop_hi, lum);
                pb.plot(x, y, col);
            } else if dp <= 0.65 {
                darken_px(pb, x, y, 0.45);
            }
        }
    }

    // 2. Chunky golden Swiss cheese wedge on the pedal (intact in Armed/Triggered;
    //    splattering in Snapping; smashed crumbs in Sprung).
    match view.state {
        MousetrapState::Armed | MousetrapState::Triggered => {
            let cheese_base: Rgb = (248, 202, 56);
            let cheese_hi: Rgb = (255, 246, 152);
            let cheese_lo: Rgb = (186, 128, 22);
            let cheese_hole: Rgb = (132, 78, 14);
            let cheese_rim: Rgb = (72, 42, 12);

            // Slightly tilted classic Swiss cheese wedge perched on the copper pedal
            // (22° cant breaks symmetry so it reads instantly as cheese, not an arrow!).
            let (sin_c, cos_c) = 0.42_f32.sin_cos();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (u, v) = fr.local(x, y);
                    let du = u - 4.9;
                    let dv = v + 0.1;
                    let cu = du * cos_c - dv * sin_c;
                    let cv = du * sin_c + dv * cos_c;
                    let half_w = (2.3 - cu) * 0.56 + 0.55;
                    let in_wedge = (-2.0..=2.3).contains(&cu) && cv.abs() <= half_w;
                    let in_rim = (-2.7..=2.9).contains(&cu) && cv.abs() <= half_w + 0.75;
                    if in_wedge {
                        let lit = 0.74 + 0.32 * fr.lit(du * 0.4, dv * 0.4);
                        let mut c = shade3(cheese_lo, cheese_base, cheese_hi, lit);
                        // Two classic round Swiss cheese holes
                        if (cu + 0.5).hypot(cv - 0.65) < 0.72 || (cu - 0.65).hypot(cv + 0.45) < 0.58 {
                            c = cheese_hole;
                        }
                        pb.plot(x, y, c);
                    } else if in_rim {
                        blend_px(pb, x, y, cheese_rim, 0.78);
                    }
                }
            }
        }
        MousetrapState::Snapping => {
            // Flying golden cheese crumbs + wood splinters bursting outward on impact!
            let burst = ((prog - 0.35).max(0.0) / 0.65).clamp(0.0, 1.0);
            let crumbs: [(f32, f32); 6] = [
                (1.0, -0.8),
                (0.7, 1.1),
                (-0.4, -1.2),
                (1.2, 0.4),
                (0.2, 1.0),
                (1.1, -0.1),
            ];
            for (i, &(du, dv)) in crumbs.iter().enumerate() {
                let spread = 1.6 + burst * (6.2 + i as f32 * 0.95);
                let p = fr.px(5.0 + du * spread, dv * spread);
                let col = if i % 2 == 0 {
                    (255, 224, 82)
                } else {
                    (236, 182, 46)
                };
                blend_px(pb, p.0, p.1, col, 1.0 - burst * 0.45);
            }
        }
        MousetrapState::Sprung => {
            // Scattered flattened cheese crumbs stuck near the front edge.
            for &(cu, cv, bright) in &[
                (4.2, -1.6, true),
                (6.0, 1.4, true),
                (5.2, -0.4, false),
                (3.4, 1.9, false),
                (6.8, -1.2, true),
            ] {
                let p = fr.px(cu, cv);
                let c = if bright {
                    (214, 174, 62)
                } else {
                    (164, 128, 44)
                };
                blend_px(pb, p.0, p.1, c, 0.88);
            }
        }
    }

    // 3. Central steel torsion coil spring along u = -0.3, |v| <= 3.6
    let spring_a = fr.place(-0.3, -3.4);
    let spring_b = fr.place(-0.3, 3.4);
    draw_metal_bar(
        pb,
        spring_a,
        spring_b,
        0.95,
        Vec2::new(0.8, 0.9),
        (134, 146, 158),
        (216, 228, 240),
        (24, 28, 34),
    );
    // Dark coil wrap grooves across the spring
    for k in -2..=2 {
        let p = fr.px(-0.3, k as f32 * 1.25);
        darken_px(pb, p.0, p.1, 0.45);
    }

    // 4. Holding lock rod (visible when Armed / Triggered).
    if matches!(view.state, MousetrapState::Armed | MousetrapState::Triggered) {
        let rod_a = fr.place(-7.6, 0.0);
        let rod_b = fr.place(3.0, 0.0);
        draw_metal_bar(
            pb,
            rod_a,
            rod_b,
            0.45,
            Vec2::new(0.5, 0.6),
            (168, 178, 188),
            (236, 244, 250),
            (32, 36, 42),
        );
    }

    // 5. The heavy U-shaped spring-steel striker bail (kill bar)!
    let (theta, lift_height, bar_flash) = match view.state {
        MousetrapState::Armed => (0.04, 0.4, 0.0),
        MousetrapState::Triggered => {
            let vib = (view.anim * 90.0 + prog * 55.0).sin() * (0.10 + 0.22 * prog);
            (0.14 + vib.abs(), 1.1 + prog * 1.2, 0.30 + 0.45 * prog)
        }
        MousetrapState::Snapping => {
            if prog < 0.44 {
                let t = prog / 0.44;
                let ang = (t * t) * PI;
                let lift = ang.sin() * 4.8;
                (ang, lift, 0.45 + 0.55 * t)
            } else {
                let post = (prog - 0.44) / 0.56;
                let bounce = ((post * PI * 2.0).sin().abs() * (1.0 - post) * 0.18).max(0.0);
                (PI - bounce, bounce * 2.0, (1.0 - post) * 0.95)
            }
        }
        MousetrapState::Sprung => (PI, 0.2, 0.0),
    };

    let bar_reach = -7.4 * theta.cos();
    let bar_half_v = 4.3;
    let lift_ofs = Vec2::new(0.0, -lift_height);
    let sh_ofs = Vec2::new(0.9 + lift_height * 0.45, 1.0 + lift_height * 0.55);

    let hinge_l = fr.place(-0.2, -bar_half_v);
    let hinge_r = fr.place(-0.2, bar_half_v);
    let tip_l = fr.place(bar_reach, -bar_half_v) + lift_ofs;
    let tip_r = fr.place(bar_reach, bar_half_v) + lift_ofs;

    let steel_mid = mix((170, 186, 204), (255, 246, 224), bar_flash.max(clamp01(view.hit_flash)));
    let steel_hi = mix((236, 246, 255), (255, 255, 255), bar_flash);
    let steel_out: Rgb = (22, 26, 34);

    // Motion-blur smear arc behind the whipping striker bar during Snapping!
    if view.state == MousetrapState::Snapping && prog < 0.52 {
        for ghost in 1..=3 {
            let g_ang = (theta - ghost as f32 * 0.34).max(0.0);
            let g_u = -7.4 * g_ang.cos();
            let g_lift = Vec2::new(0.0, -g_ang.sin() * 4.6);
            let ga = fr.place(g_u, -bar_half_v) + g_lift;
            let gb = fr.place(g_u, bar_half_v) + g_lift;
            let alpha = (0.55 - ghost as f32 * 0.14).max(0.1);
            draw_strand_curve(pb, ga, gb, 0.0, 1.0, (240, 248, 255), (120, 140, 165), alpha);
        }
    }

    draw_metal_bar(pb, hinge_l, tip_l, 0.65, sh_ofs, steel_mid, steel_hi, steel_out);
    draw_metal_bar(pb, hinge_r, tip_r, 0.65, sh_ofs, steel_mid, steel_hi, steel_out);
    draw_metal_bar(pb, tip_l, tip_r, 0.85, sh_ofs, steel_mid, steel_hi, steel_out);

    // Bright tension sparks at the latch in Triggered; crisp radial impact lines in Snapping!
    if view.state == MousetrapState::Triggered {
        let spark_p = fr.place(-0.3, (view.anim * 40.0).sin() * 2.2);
        glow_disc(pb, spark_p.x, spark_p.y, 4.0, (255, 186, 68), 0.88 * (0.4 + 0.6 * prog));
        // Tiny amber tension sparks zipping off the cocked spring!
        for k in 0..3 {
            let s = fr.px(-0.8 - k as f32 * 1.4, (k as f32 - 1.0) * 3.8);
            add_px(pb, s.0, s.1, (255, 220, 112), 0.85 * prog);
        }
    } else if view.state == MousetrapState::Snapping && (0.42..=0.80).contains(&prog) {
        let imp = 1.0 - (prog - 0.42) / 0.38;
        let mid_bar = (tip_l + tip_r) * 0.5;
        glow_disc(pb, mid_bar.x, mid_bar.y, 5.5, (255, 236, 178), 0.90 * imp);
        // 4 crisp radial impact burst streaks!
        for &(du, dv) in &[(1.0, -0.7), (1.0, 0.7), (0.3, -1.0), (0.3, 1.0)] {
            let a = fr.place(6.5 + du * 1.6, dv * 2.6);
            let b = fr.place(6.5 + du * (4.2 + (1.0 - imp) * 3.5), dv * (5.2 + (1.0 - imp) * 2.5));
            draw_strand_curve(pb, a, b, 0.0, 0.8, (255, 248, 212), (255, 168, 64), 0.90 * imp);
        }
    }
}

// ============================================================================
// 3. One-Way Door drawing
// ============================================================================

/// **Ground layer** for a one-way door (draw after floor/stains, BEFORE entities).
///
/// Renders the recessed steel threshold sill, directional ratchet markings,
/// glowing `accent`-tinted arrow/chevrons of light pointing strictly in `+facing`
/// (`-u -> +u`), and the crimson denial floor wash during `Refused`.
pub fn draw_one_way_door_ground(
    pb: &mut PixelBuffer,
    view: &OneWayDoorView,
    cam: Vec2,
    accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 18.0) else {
        return;
    };

    let prog = clamp01(view.progress);
    let half_u = ONE_WAY_DOOR_HALF_U; // 5.0
    let gap_v = ONE_WAY_DOOR_PASSAGE_HALF_V; // 9.0

    // Threshold energy brightness depends on state.
    let (arrow_col, arrow_glow) = match view.state {
        OneWayDoorState::Closed => {
            let pulse = 0.65 + 0.35 * (view.anim * 3.4).sin();
            (mix(accent, (170, 255, 230), 0.30), 0.78 * pulse)
        }
        OneWayDoorState::Opening => (mix(accent, (235, 255, 245), 0.55), 0.90 + 0.35 * prog),
        OneWayDoorState::Closing => (mix(accent, (170, 245, 225), 0.35), 0.95 - 0.30 * prog),
        OneWayDoorState::Refused => {
            let flash = (1.0 - prog) * (0.6 + 0.4 * (prog * 22.0).sin().abs());
            ((255, 64, 68), 0.95 * flash)
        }
    };

    // Soft glowing light spill projected onto the floor pointing toward +u!
    let spill_center = fr.place(3.6, 0.0);
    glow_disc(pb, spill_center.x, spill_center.y, 10.0, arrow_col, 0.36 * arrow_glow);
    let entry_hint = fr.place(-6.5, 0.0);
    glow_disc(pb, entry_hint.x, entry_hint.y, 7.5, arrow_col, 0.28 * arrow_glow);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);

            // Recessed floor threshold plate between the jambs.
            let d_sill = sd_box(u, v, half_u - 0.5, gap_v);
            if d_sill <= 0.0 {
                let plate = (38, 44, 52);
                let plate_hi = (66, 76, 86);
                let mut col = shade3((22, 26, 32), plate, plate_hi, 0.56 + 0.20 * fr.lit(u * 0.15, v * 0.1));
                // Recessed track & seam grooves
                if (u.abs() - (half_u - 1.3)).abs() < 0.45 {
                    col = scale(col, 0.58);
                }
                pb.plot(x, y, col);
            }

            // Crisp glowing directional light-arrow / double chevron pointing strictly +u!
            if v.abs() <= 5.6 && (-10.5..=9.0).contains(&u) {
                let shift = if view.state == OneWayDoorState::Refused {
                    0.0
                } else {
                    ((view.anim * 3.8) % 1.0) * 1.4
                };
                // Outer approach chevron on -u floor, and bright exit chevron at +u!
                let tip1 = 4.0 + shift * 0.45;
                let tip2 = -3.4 + shift * 0.45;
                let d_chev1 = ((u + v.abs() * 0.90) - tip1).abs();
                let d_chev2 = ((u + v.abs() * 0.90) - tip2).abs();
                let on_chev1 = d_chev1 < 0.95 && u <= tip1 + 0.4 && v.abs() <= 4.8;
                let on_chev2 = d_chev2 < 0.90 && u <= tip2 + 0.4 && v.abs() <= 4.0;
                // Center arrow shaft leading into the door from the -u entry side
                let on_shaft = v.abs() < 0.78 && (-10.0..=-3.2).contains(&u);

                if on_chev1 || on_chev2 || on_shaft {
                    let core = if on_chev1 {
                        mix(arrow_col, (255, 255, 255), 0.50)
                    } else {
                        arrow_col
                    };
                    let strength = if on_chev1 {
                        arrow_glow.min(1.0)
                    } else {
                        (arrow_glow * 0.82).min(0.96)
                    };
                    blend_px(pb, x, y, core, strength);
                }
            }
        }
    }
}

/// **Foreground / overlay layer** for a one-way door (draw AFTER entities).
///
/// Renders the two heavy steel side jambs with orange-brass hinge pins and
/// red-tipped mechanical backstop lugs on `-facing`, the paired V-wedge saloon
/// flaps (angled forward toward `+facing` when closed, swinging wide into
/// `+facing` in `Opening`, spring-closing in `Closing`, and rattling shut with
/// a vivid crimson NO-ENTRY bar / `X` on the `+facing` face in `Refused`).
pub fn draw_one_way_door(
    pb: &mut PixelBuffer,
    view: &OneWayDoorView,
    cam: Vec2,
    accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 19.0) else {
        return;
    };

    let prog = clamp01(view.progress);
    let half_u = ONE_WAY_DOOR_HALF_U; // 5.0
    let gap_v = ONE_WAY_DOOR_PASSAGE_HALF_V; // 9.0
    let outer_v = ONE_WAY_DOOR_SIZE.y * 0.5; // 12.0

    let jamb_dark: Rgb = (34, 42, 50);
    let jamb_mid: Rgb = (88, 102, 116);
    let jamb_hi: Rgb = (162, 182, 198);
    let outline: Rgb = (16, 20, 26);

    // 1. Two chunky 3/4 steel doorjamb posts at v in [-12, -9] and [+9, +12].
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let av = v.abs();
            let d_jamb = sd_box(u, av - (gap_v + outer_v) * 0.5, half_u, (outer_v - gap_v) * 0.5);
            if d_jamb > 2.2 {
                continue;
            }
            if d_jamb > 0.75 {
                let (su, sv) = fr.local_f(x as f32 - 1.5, y as f32 - 1.6);
                let ds = sd_box(
                    su,
                    sv.abs() - (gap_v + outer_v) * 0.5,
                    half_u,
                    (outer_v - gap_v) * 0.5,
                );
                if ds <= 0.4 {
                    darken_px(pb, x, y, 0.48);
                }
                continue;
            }
            if d_jamb > -0.25 {
                pb.plot(x, y, outline);
                continue;
            }

            let lit = 0.65 + 0.35 * fr.lit(u / half_u, (av - 10.5) * 0.45);
            let mut col = shade3(jamb_dark, jamb_mid, jamb_hi, lit);
            // Status indicator strip embedded on top of each jamb post!
            if u.abs() < 1.2 && (av - 10.5).abs() < 0.9 {
                col = match view.state {
                    OneWayDoorState::Refused => (255, 72, 68),
                    OneWayDoorState::Opening => mix(accent, (220, 255, 240), 0.55),
                    _ => mix(accent, (140, 220, 200), 0.35),
                };
            }
            pb.plot(x, y, col);
        }
    }

    // 2. Physical mechanical backstop lugs on the -u edge of each jamb
    //    (visually proving the flaps cannot swing backward toward -u!).
    for side in [-1.0_f32, 1.0] {
        let lug = fr.px(-3.2, side * (gap_v - 0.8));
        pb.plot(lug.0, lug.1, (206, 72, 62));
        blend_px(pb, lug.0 - 1, lug.1, (244, 138, 118), 0.88);
        // Brass hinge pin at (0.0, side * gap_v)
        let pin = fr.px(0.0, side * (gap_v - 0.3));
        pb.plot(pin.0, pin.1, (232, 192, 102));
    }

    // 3. Paired V-wedge saloon flaps hinged at (-0.6, ±gap_v).
    //    Bold closed V-angle (0.36 rad) forms an unmistakable `>` funnel pointing +u!
    let (swing, rattle_u, refused_glow) = match view.state {
        OneWayDoorState::Closed => (0.36_f32, 0.0_f32, 0.0_f32),
        OneWayDoorState::Opening => {
            let e = 1.0 - (1.0 - prog) * (1.0 - prog);
            (0.36 + e * 1.10, 0.0, 0.0)
        }
        OneWayDoorState::Closing => {
            let e = prog * prog;
            let bounce = if prog > 0.80 {
                ((prog - 0.80) / 0.20 * PI).sin() * 0.10
            } else {
                0.0
            };
            (1.46 - e * 1.10 + bounce, 0.0, 0.0)
        }
        OneWayDoorState::Refused => {
            let decay = 1.0 - prog;
            let vib = (prog * 34.0).sin() * 0.55 * decay;
            (0.28 + vib.abs() * 0.05, vib, decay)
        }
    };

    let leaf_len = 9.2_f32;
    let flap_frame: Rgb = mix((102, 122, 136), (255, 110, 96), refused_glow * 0.55);
    let flap_hi: Rgb = mix((204, 228, 242), (255, 200, 190), refused_glow * 0.45);
    let flap_glass: Rgb = mix((116, 172, 188), accent, 0.28);

    for side in [-1.0_f32, 1.0] {
        let hinge = fr.place(-0.6 + rattle_u * 0.25, side * (gap_v - 0.4));
        let leaf_dir_u = swing.sin();
        let leaf_dir_v = -side * swing.cos();
        let tip = fr.place(
            -0.6 + leaf_dir_u * leaf_len + rattle_u * 0.6,
            side * (gap_v - 0.4) + leaf_dir_v * leaf_len,
        );

        draw_metal_bar(
            pb,
            hinge,
            tip,
            1.15,
            Vec2::new(1.1, 1.2),
            flap_frame,
            flap_hi,
            outline,
        );
        let mid_a = hinge + (tip - hinge) * 0.20;
        let mid_b = hinge + (tip - hinge) * 0.80;
        draw_strand_curve(pb, mid_a, mid_b, 0.0, 0.9, flap_glass, flap_hi, 0.78);

        // Distinct crimson "NO ENTRY" denial face locked to the leaf's true rotated
        // +u-side outward normal through the entire swing animation!
        let leaf_out_normal = fr.f * swing.cos() + fr.p * (side * swing.sin());
        let wrong_side_ofs = leaf_out_normal * 1.05;
        let red_a = mid_a + wrong_side_ofs;
        let red_b = mid_b + wrong_side_ofs;
        let red_col = if view.state == OneWayDoorState::Refused {
            (255, 76, 72)
        } else {
            (196, 54, 52)
        };
        draw_strand_curve(
            pb,
            red_a,
            red_b,
            0.0,
            0.85,
            red_col,
            (84, 16, 18),
            if view.state == OneWayDoorState::Refused {
                0.98
            } else {
                0.84
            },
        );
    }

    // Center brass latch clasp when Closed or Refused.
    if matches!(view.state, OneWayDoorState::Closed | OneWayDoorState::Refused) {
        let latch = fr.px(-0.6 + swing.sin() * leaf_len + rattle_u * 0.6, 0.0);
        let latch_col = if view.state == OneWayDoorState::Refused {
            (255, 92, 82)
        } else {
            (236, 198, 108)
        };
        pb.plot(latch.0, latch.1, latch_col);
    }

    // 4. Unmistakable crimson NO-ENTRY shield bar & bold X on the +u face in Refused!
    if view.state == OneWayDoorState::Refused {
        let center_x = fr.place(4.2, 0.0);
        let flash = (1.0 - prog) * (0.72 + 0.28 * (prog * 28.0).sin().abs());
        glow_disc(pb, center_x.x, center_x.y, 7.5, (255, 44, 48), 0.75 * flash);

        // Dark-backed crisp stop barrier bar on the +u face
        let bar_a = fr.place(3.6, -7.2);
        let bar_b = fr.place(3.6, 7.2);
        draw_strand_curve(pb, bar_a, bar_b, 0.0, 1.1, (255, 78, 72), (96, 12, 16), 0.96 * flash);

        // High-contrast bold crimson-white X centered on the barrier plane
        let r = 2.8;
        let d1a = fr.place(4.2 - r * 0.75, -r);
        let d1b = fr.place(4.2 + r * 0.75, r);
        let d2a = fr.place(4.2 - r * 0.75, r);
        let d2b = fr.place(4.2 + r * 0.75, -r);
        draw_strand_curve(pb, d1a, d1b, 0.0, 1.35, (255, 196, 178), (168, 18, 22), flash);
        draw_strand_curve(pb, d2a, d2b, 0.0, 1.35, (255, 196, 178), (168, 18, 22), flash);
    }
}

// ============================================================================
// 4. Rat-Hole drawing
// ============================================================================

/// **Ground layer** for a rat-hole (draw after floor/stains, BEFORE entities).
///
/// Renders the dark arched tunnel floor corridor through the wall (`8 px` wide),
/// dusty worn threshold aprons with tiny rat paw-print tracks leading through,
/// and outer wall contact shadows.
pub fn draw_rat_hole_ground(
    pb: &mut PixelBuffer,
    view: &RatHoleView,
    cam: Vec2,
    _accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 19.0) else {
        return;
    };

    let half_u = RAT_HOLE_HALF_U; // 7.0
    let gap_v = RAT_HOLE_GAP_HALF_V; // 4.0
    let outer_v = RAT_HOLE_SIZE.y * 0.5; // 11.0

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let av = v.abs();

            // Deep pitch-black arched tunnel cavity cutting cleanly through the wall!
            if u.abs() <= half_u + 0.5 && av <= gap_v + 0.3 {
                let depth = 1.0 - (u.abs() / (half_u + 1.2)).clamp(0.0, 1.0);
                let tunnel_dark: Rgb = (4, 6, 9);
                let alpha = (0.76 + 0.22 * depth).clamp(0.0, 0.98);
                blend_px(pb, x, y, tunnel_dark, alpha);
                continue;
            }

            // Worn dusty threshold apron spilling slightly outside both tunnel mouths!
            if u.abs() > half_u && u.abs() <= half_u + 4.8 && av <= gap_v + 1.4 {
                let d_mouth = (u.abs() - half_u) / 4.8;
                let d_side = av / (gap_v + 1.4);
                let fade = ((1.0 - d_mouth) * (1.0 - d_side * d_side)).clamp(0.0, 1.0);
                blend_px(pb, x, y, (118, 102, 82), fade * 0.34);
            }

            // Grounded drop shadow cast by the solid wall segment down-right.
            let (su, sv) = fr.local_f(x as f32 - 2.0, y as f32 - 2.2);
            if su.abs() <= half_u && sv.abs() <= outer_v && sv.abs() >= gap_v - 0.5 {
                darken_px(pb, x, y, 0.48);
            }
        }
    }

    // Tiny pale dusty rat paw-print trail leading straight into & out of the tunnel!
    for &(pu, pv) in &[
        (-10.0, -0.9),
        (-8.4, 0.9),
        (8.4, -0.9),
        (10.0, 0.9),
    ] {
        let p = fr.px(pu, pv);
        blend_px(pb, p.0, p.1, (176, 158, 130), 0.65);
    }
}

/// **Foreground / overlay layer** for a rat-hole (draw AFTER entities so the
/// rat visibly ducks *under* the arched crown lintel!).
///
/// Renders:
/// - A crisp straight rectangular 3/4 masonry wall (`14 × 22 px`) with solid
///   wall cheeks (`|v| in [4.0, 11.0]`), mortar seams, and **3 bold parallel
///   diagonal claw/scrape gouges** at Brute/Cat shoulder width (`|v| in [5.4, 9.8]`).
/// - Deep **pitch-black rounded arched tunnel mouths** carved into both sides
///   across `|v| <= 4.0`, traced by a crisp 1 px gnawed pale rim and bridged by
///   a thin solid masonry keystone lintel overhead so the rat visibly ducks
///   underneath while Brutes slam harmlessly into the scarred cheeks.
pub fn draw_rat_hole(
    pb: &mut PixelBuffer,
    view: &RatHoleView,
    cam: Vec2,
    accent: Rgb,
) {
    let prog = clamp01(view.progress);
    let shake = match view.state {
        RatHoleState::Blocked => {
            let decay = 1.0 - prog;
            let s = (prog * 44.0).sin() * 0.70 * decay;
            safe_dir(view.facing) * s
        }
        RatHoleState::Squeezing => {
            let s = (prog * 22.0).sin() * 0.20 * (1.0 - prog);
            safe_dir(view.facing).perp() * s
        }
        RatHoleState::Idle => Vec2::ZERO,
    };

    let fr = Frame::new(view.pos + shake, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 19.0) else {
        return;
    };

    let half_u = RAT_HOLE_HALF_U; // 7.0
    let gap_v = RAT_HOLE_GAP_HALF_V; // 4.0
    let outer_v = RAT_HOLE_SIZE.y * 0.5; // 11.0

    let base_wall = match view.wall_tint {
        Some(wt) => mix(wt, (88, 94, 102), 0.34),
        None => mix((86, 92, 100), accent, 0.14),
    };
    let wall_dark = scale(base_wall, 0.42);
    let wall_hi = mix(scale(base_wall, 1.44), (222, 218, 208), 0.24);
    let gnaw_rim: Rgb = (206, 184, 146);
    let gnaw_hi: Rgb = (242, 224, 186);
    let scratch_hi: Rgb = (224, 212, 188);
    let outline: Rgb = (14, 16, 20);

    let blocked_flash = if view.state == RatHoleState::Blocked {
        (1.0 - prog) * (0.55 + 0.45 * (prog * 24.0).sin().abs())
    } else {
        0.0
    };

    let seed = hash2(view.pos.x.round() as i32, view.pos.y.round() as i32);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let av = v.abs();

            // Outer straight rectangular wall bounds: |u| <= 7.0, |v| <= 11.0
            let d_wall = sd_box(u, v, half_u, outer_v);
            if d_wall > 0.75 {
                continue;
            }

            // True semicircular rounded mouse-hole arch cutout on BOTH ends across |v| <= 4.0!
            // At v=0 (center of hole), dome=1 -> arch_u = 3.0 (deep 4px rounded black cavity,
            // sturdy 6px masonry crown bridge overhead); at |v|=4.0, dome=0 -> arch_u = 7.0
            // meeting the straight rectangular wall edge smoothly without a V-notch!
            let t_v = (av / gap_v).clamp(0.0, 1.0);
            let dome = (1.0 - t_v * t_v).max(0.0).sqrt();
            let arch_u = half_u - 4.0 * dome;
            let in_dark_mouth = av <= gap_v && u.abs() > arch_u;
            let on_gnaw_edge = av <= gap_v + 0.55
                && u.abs() >= arch_u - 1.05
                && u.abs() <= arch_u + 0.15;

            // Leave the deep black tunnel mouth open so entities & floor shadows show!
            if in_dark_mouth {
                continue;
            }

            // Crisp dark exterior wall outline (skip across the open black tunnel mouths).
            if d_wall > -0.25 {
                pb.plot(x, y, outline);
                continue;
            }

            // Crisp 1 px gnawed / chipped pale arch rim tracing ONLY the edge of the hole!
            if on_gnaw_edge && av <= gap_v + 0.3 {
                let notch = hash_f32((v * 2.5).round() as i32 + (seed as i32 & 15), u.signum() as i32);
                let squeeze_boost = if view.state == RatHoleState::Squeezing {
                    0.35 * (1.0 - prog)
                } else {
                    0.0
                };
                let lit = 0.66 + 0.30 * fr.lit(u.signum(), -v / gap_v) + notch * 0.14;
                let mut col = shade3((94, 76, 54), gnaw_rim, gnaw_hi, lit);
                if squeeze_boost > 0.0 {
                    col = mix(col, (255, 244, 212), squeeze_boost);
                }
                pb.plot(x, y, col);
                continue;
            }

            // Solid masonry wall body (both wide cheeks AND continuous top lintel bridge!).
            let is_outer_bevel = u.abs() > half_u - 1.15 || av > outer_v - 1.15;
            let nu = if u.abs() > half_u - 1.5 { u.signum() } else { 0.0 };
            let nv = if av > outer_v - 1.5 { v.signum() } else { 0.0 };
            let lum = 0.64 + 0.38 * fr.lit(nu, nv);
            let mut col = shade3(wall_dark, base_wall, wall_hi, lum);

            // Subtle masonry brick/course seams & central keystone over the arch.
            if av <= gap_v {
                // Keystone wedge seams on the narrow masonry bridge over the arch
                if (av - 1.7).abs() < 0.35 {
                    col = scale(col, 0.64);
                } else if av < 1.7 && u.abs() < 1.6 {
                    col = mix(col, wall_hi, 0.22); // proud central keystone highlight
                }
            } else {
                // Brick mortar lines on the wide wall cheeks
                let mortar = ((u + 1.5).abs() < 0.35 && av > 6.8) || ((av - 7.6).abs() < 0.35);
                if mortar && !is_outer_bevel {
                    col = scale(col, 0.62);
                }

                // 3 BOLD parallel diagonal claw/scrape gouges at big-enemy shoulder width!
                // (|v| in 5.0..9.6, near both entrance faces |u| in 1.6..5.8)
                if (5.0..=9.6).contains(&av) && (1.6..=5.8).contains(&u.abs()) {
                    let slash = (u.abs() * 0.85 - (av - 5.0) * 0.78).rem_euclid(1.95);
                    if slash < 0.42 {
                        col = mix(col, scratch_hi, 0.80);
                    } else if slash < 0.82 {
                        col = scale(col, 0.48); // deep dark claw groove right beside pale chip
                    }
                }

                // Tight impact rim flash ONLY on the struck front shoulder face when Blocked!
                if blocked_flash > 0.01 && u > 3.2 && (4.2..=9.8).contains(&av) {
                    col = mix(col, (255, 198, 128), blocked_flash * 0.65);
                }
            }

            pb.plot(x, y, col);
        }
    }

    // State VFX particles (zero-allocation deterministic bursts).
    match view.state {
        RatHoleState::Squeezing => {
            let fade = 1.0 - prog;
            for i in 0..6 {
                let side_u = if i % 2 == 0 { 1.0 } else { -1.0 };
                let spread_u = side_u * (6.0 + prog * (3.6 + (i % 3) as f32 * 1.1));
                let spread_v = ((i as f32 * 1.9 + 0.5).sin()) * (1.8 + prog * 2.2);
                let p = fr.px(spread_u, spread_v);
                blend_px(pb, p.0, p.1, (206, 190, 162), 0.78 * fade);
            }
        }
        RatHoleState::Blocked => {
            // Crisp white-amber impact sparks & stone chips flying off the struck shoulders!
            let fade = 1.0 - prog;
            for side in [-1.0_f32, 1.0] {
                let impact = fr.place(6.2, side * 6.4);
                glow_disc(pb, impact.x, impact.y, 4.0, (255, 172, 84), 0.75 * fade);
                for k in 0..3 {
                    let ang = (k as f32 - 1.0) * 0.65;
                    let dist = 1.2 + prog * (4.8 + k as f32 * 1.3);
                    let sp = fr.px(
                        6.4 + ang.cos() * dist,
                        side * 6.4 + side * (ang.sin().abs() * dist),
                    );
                    let col = if k == 1 {
                        (255, 242, 178)
                    } else {
                        (222, 208, 182)
                    };
                    blend_px(pb, sp.0, sp.1, col, 0.94 * fade);
                }
            }
        }
        RatHoleState::Idle => {}
    }
}

// ============================================================================
// 5. Maze Reward drawing
// ============================================================================

/// **Ground layer** for a maze reward pedestal (draw after floor/stains, BEFORE entities).
///
/// Renders the welcoming golden-amber + `accent` floor bloom, soft octagonal
/// pedestal contact shadow, and expanding celebratory light rings during `Opening`.
pub fn draw_maze_reward_ground(
    pb: &mut PixelBuffer,
    view: &MazeRewardView,
    cam: Vec2,
    accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 22.0) else {
        return;
    };

    let prog = clamp01(view.progress);
    let gem_col = view
        .reward_tint
        .unwrap_or_else(|| mix((255, 210, 82), accent, 0.28));

    // 1. Soft floor bloom drawing the player's eye from half a screen away!
    match view.state {
        MazeRewardState::Idle => {
            let pulse = 0.72 + 0.28 * (view.anim * 2.8).sin();
            glow_disc(pb, fr.pos.x, fr.pos.y, 16.0, gem_col, 0.36 * pulse);
            glow_disc(pb, fr.pos.x, fr.pos.y, 10.5, accent, 0.20 * pulse);
        }
        MazeRewardState::Opening => {
            let burst = (1.0 - prog).powf(0.75);
            glow_disc(pb, fr.pos.x, fr.pos.y, 19.0, (255, 232, 138), 0.72 * burst);
            // Expanding twin celebratory shockwave rings!
            let r1 = 5.0 + prog * 15.0;
            let r2 = (2.0 + prog * 11.0).max(1.0);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let dx = x as f32 - fr.pos.x;
                    let dy = y as f32 - fr.pos.y;
                    let d = (dx * dx + dy * dy).sqrt();
                    if (d - r1).abs() < 0.95 {
                        add_px(pb, x, y, (255, 238, 164), 0.90 * burst);
                    } else if (d - r2).abs() < 0.85 {
                        add_px(pb, x, y, accent, 0.72 * burst);
                    }
                }
            }
        }
        MazeRewardState::Opened => {
            // Faint spent standby glow so the player remembers they looted this maze.
            glow_disc(pb, fr.pos.x, fr.pos.y, 10.0, accent, 0.09);
        }
    }

    // 2. Grounded octagonal pedestal contact shadow shifted down-right.
    let r = MAZE_REWARD_PEDESTAL_RADIUS;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - (fr.pos.x + 1.5);
            let dy = y as f32 - (fr.pos.y + 1.7);
            if sd_octagon(dx, dy, r + 0.3) <= 0.0 {
                darken_px(pb, x, y, 0.46);
            }
        }
    }
}

/// **Foreground / overlay layer** for a maze reward pedestal (draw AFTER entities).
///
/// Renders the stepped octagonal dark-iron and beveled-brass dais with glowing
/// cardinal conduits, the gilded powerup reliquary chest / floating golden core
/// with orbiting shimmer sparkles in `Idle`, the parting brass lid + soaring
/// golden pillar in `Opening`, and the open empty dark-velvet cradle in `Opened`.
pub fn draw_maze_reward(
    pb: &mut PixelBuffer,
    view: &MazeRewardView,
    cam: Vec2,
    accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 20.0) else {
        return;
    };

    let prog = clamp01(view.progress);
    let gem_col = view
        .reward_tint
        .unwrap_or_else(|| mix((255, 210, 82), accent, 0.28));
    let pulse = 0.65 + 0.35 * (view.anim * 3.6).sin();

    let iron_dark: Rgb = (32, 38, 46);
    let iron_mid: Rgb = (70, 78, 92);
    let iron_hi: Rgb = (128, 140, 158);
    let brass_dark: Rgb = (108, 78, 30);
    let brass_mid: Rgb = (196, 154, 68);
    let brass_hi: Rgb = (252, 224, 134);
    let outline: Rgb = (14, 16, 22);

    let r_outer = MAZE_REWARD_PEDESTAL_RADIUS; // 8.5
    let r_inner = 5.8;

    let conduit_power = match view.state {
        MazeRewardState::Idle => 0.55 + 0.35 * pulse,
        MazeRewardState::Opening => (1.0 - prog * 0.5).max(0.4),
        MazeRewardState::Opened => 0.16,
    };

    // 1. Stepped octagonal dais with beveled brass rim and glowing cardinal conduits.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - fr.pos.x;
            let dy = y as f32 - fr.pos.y;
            let d_out = sd_octagon(dx, dy, r_outer);
            if d_out > 0.75 {
                continue;
            }
            if d_out > -0.25 {
                pb.plot(x, y, outline);
                continue;
            }

            let d_in = sd_octagon(dx, dy, r_inner);
            let lum_dir = (dx * LIGHT.x + dy * LIGHT.y) / r_outer;

            if d_in > 0.0 {
                // Lower outer octagonal plinth step
                let mut col = shade3(iron_dark, iron_mid, iron_hi, 0.58 + 0.38 * lum_dir);
                // Beveled brass corner gussets on diagonals
                if (dx.abs() - dy.abs()).abs() < 1.35 && d_out > -1.9 {
                    col = shade3(brass_dark, brass_mid, brass_hi, 0.65 + 0.35 * lum_dir);
                }
                // Glowing cardinal energy conduits leading toward the center
                let on_conduit = (dx.abs() < 0.75 || dy.abs() < 0.75) && d_out < -1.0;
                if on_conduit {
                    let cond_c = mix(accent, gem_col, 0.45);
                    col = mix(col, cond_c, conduit_power);
                }
                pb.plot(x, y, col);
            } else if d_in > -0.85 {
                // Raised brass bezel lip around the inner reliquary cradle!
                let col = shade3(brass_dark, brass_mid, brass_hi, 0.68 + 0.42 * lum_dir);
                pb.plot(x, y, col);
            } else {
                // Dark recessed velvet/energy cradle interior
                let radial = (dx * dx + dy * dy).sqrt() / r_inner;
                let mut col = mix((20, 14, 28), (44, 26, 46), 1.0 - radial);
                if view.state != MazeRewardState::Opened {
                    col = mix(col, gem_col, (1.0 - radial) * 0.42 * conduit_power);
                }
                pb.plot(x, y, col);
            }
        }
    }

    // 2. Central Gilded Powerup Reliquary / Floating Core (state-dependent).
    match view.state {
        MazeRewardState::Idle => {
            // Subtle floating vertical bob in screen -y!
            let bob = (view.anim * 3.2).sin() * 0.9 - 1.0;
            let cx = fr.pos.x;
            let cy = fr.pos.y + bob;

            // Shadow of the floating reliquary onto the cradle floor
            darken_px(pb, fr.pos.x.round() as i32 + 1, fr.pos.y.round() as i32 + 1, 0.45);

            // Ornate gilded diamond-chest & radiant powerup core
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let manhattan = dx.abs() + dy.abs();
                    // Slightly tapered chest-crown silhouette (wider shoulder cap)
                    let box_d = sd_box(dx, dy, 3.8, 3.2);

                    if box_d <= 0.65 && box_d > -0.15 {
                        pb.plot(x, y, outline);
                    } else if box_d <= -0.15 {
                        let lit = 0.68 + 0.42 * ((dx * LIGHT.x + dy * LIGHT.y) / 3.8);
                        let mut col = if manhattan <= 2.45 {
                            // Radiant inner powerup gem crystal!
                            let core_t = (1.0 - manhattan / 2.45).clamp(0.0, 1.0);
                            mix(gem_col, (255, 255, 246), core_t * (0.58 + 0.42 * pulse))
                        } else if dx.abs() < 0.78 || dy.abs() < 0.78 || dy < -2.1 {
                            // Gilded crown lid bevel & filigree cross-straps
                            shade3(brass_dark, brass_mid, brass_hi, lit + 0.18)
                        } else {
                            // Rich enameled reliquary casing
                            shade3((74, 48, 22), (168, 122, 46), brass_hi, lit)
                        };
                        // Diagonal specular glint sweeping across the treasure cache
                        let glint = ((dx + dy) - (view.anim * 4.5).sin() * 4.2).abs();
                        if glint < 0.85 {
                            col = mix(col, (255, 255, 244), 0.75);
                        }
                        pb.plot(x, y, col);
                    }
                }
            }

            // Soft additive sparkle halo + crisp 4-point star twinkle flare!
            glow_disc(pb, cx, cy, 5.4, gem_col, 0.48 * pulse);
            let sx = (cx - 1.5).round() as i32;
            let sy = (cy - 1.5).round() as i32;
            let star_k = 0.55 + 0.45 * (view.anim * 5.2).sin().abs();
            add_px(pb, sx, sy, (255, 255, 244), 0.95 * star_k);
            add_px(pb, sx - 1, sy, gem_col, 0.65 * star_k);
            add_px(pb, sx + 1, sy, gem_col, 0.65 * star_k);
            add_px(pb, sx, sy - 1, gem_col, 0.65 * star_k);
            add_px(pb, sx, sy + 1, gem_col, 0.65 * star_k);

            // 4 orbiting pixel shimmer motes around the pedestal!
            for i in 0..4 {
                let ang = view.anim * 1.9 + i as f32 * (TAU / 4.0);
                let r_orb = 6.2 + (view.anim * 3.0 + i as f32).sin() * 0.9;
                let mx = (fr.pos.x + ang.cos() * r_orb).round() as i32;
                let my = (fr.pos.y + ang.sin() * r_orb * 0.82 - 1.2).round() as i32;
                let twinkle = 0.45 + 0.55 * (view.anim * 7.0 + i as f32 * 2.1).sin().abs();
                let col = if i % 2 == 0 { (255, 248, 196) } else { accent };
                add_px(pb, mx, my, col, twinkle);
            }
        }

        MazeRewardState::Opening => {
            // Hinged brass lid petals swinging wide open to left & right!
            let spread = 2.6 + prog * 3.4;
            for side in [-1.0_f32, 1.0] {
                let a = Vec2::new(fr.pos.x + side * spread, fr.pos.y - 2.6);
                let b = Vec2::new(fr.pos.x + side * (spread + 1.4), fr.pos.y + 2.2);
                draw_metal_bar(pb, a, b, 0.85, Vec2::new(0.8, 1.0), brass_mid, brass_hi, outline);
            }

            // Rising golden-white powerup orb & radiant vertical light beam!
            let rise = prog * 9.5;
            let fade = (1.0 - prog * prog).clamp(0.0, 1.0);
            let orb_y = fr.pos.y - 1.5 - rise;
            let beam_w = (3.4 * (1.0 - prog * 0.65)).max(1.0);

            for dy_i in -14..=2 {
                let py = (fr.pos.y as i32) + dy_i;
                let height_t = ((-dy_i as f32) / 14.0).clamp(0.0, 1.0);
                let beam_a = (1.0 - height_t * 0.75) * fade * 0.72;
                for dx_i in -4..=4 {
                    if (dx_i as f32).abs() <= beam_w {
                        let center_k = 1.0 - (dx_i as f32).abs() / (beam_w + 0.4);
                        let col = mix(gem_col, (255, 255, 246), center_k * 0.75);
                        add_px(pb, fr.pos.x.round() as i32 + dx_i, py, col, beam_a * center_k);
                    }
                }
            }

            // Rising golden orb dissolving into starlight!
            glow_disc(pb, fr.pos.x, orb_y, 5.5 * fade + 1.5, (255, 244, 178), 0.95 * fade);
            if fade > 0.15 {
                let ox = fr.pos.x.round() as i32;
                let oy = orb_y.round() as i32;
                blend_px(pb, ox, oy, (255, 255, 255), fade);
                blend_px(pb, ox - 1, oy, gem_col, fade * 0.85);
                blend_px(pb, ox + 1, oy, gem_col, fade * 0.85);
                blend_px(pb, ox, oy - 1, gem_col, fade * 0.85);
                blend_px(pb, ox, oy + 1, gem_col, fade * 0.85);
            }

            // Celebratory radial gold sparkles bursting outward!
            for i in 0..8 {
                let ang = i as f32 * (TAU / 8.0) + prog * 1.1;
                let dist = 2.0 + prog * (11.5 + (i % 3) as f32 * 2.0);
                let sx = (fr.pos.x + ang.cos() * dist).round() as i32;
                let sy = (fr.pos.y + ang.sin() * dist * 0.85 - prog * 3.0).round() as i32;
                let col = if i % 2 == 0 { (255, 246, 172) } else { accent };
                add_px(pb, sx, sy, col, fade);
            }
        }

        MazeRewardState::Opened => {
            // Open brass lid halves resting wide on the pedestal flanks, exposing
            // the empty dark velvet recess inside!
            let spent_brass = scale(brass_mid, 0.72);
            let spent_hi = scale(brass_hi, 0.74);
            for side in [-1.0_f32, 1.0] {
                let a = Vec2::new(fr.pos.x + side * 4.8, fr.pos.y - 2.6);
                let b = Vec2::new(fr.pos.x + side * 5.6, fr.pos.y + 2.4);
                draw_metal_bar(pb, a, b, 0.8, Vec2::new(0.8, 0.9), spent_brass, spent_hi, outline);
            }
            // Empty inner socket rim with crisp dark-velvet depth + 2 subtle cooling embers
            let cx = fr.pos.x.round() as i32;
            let cy = fr.pos.y.round() as i32;
            pb.plot(cx, cy, (18, 12, 24));
            blend_px(pb, cx - 2, cy - 2, (112, 92, 56), 0.55);
            blend_px(pb, cx + 2, cy + 2, (34, 24, 38), 0.65);
            let ember = 0.28 + 0.25 * (view.anim * 2.2).sin().abs();
            add_px(pb, cx - 1, cy - 1, mix(gem_col, accent, 0.5), ember);
        }
    }
}

// ============================================================================
// Tests & Visual Reference Sheet
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_enemy::draw_enemy;
    use crate::game::art_floor::draw_floor;
    use crate::game::art_rat::draw_rat;
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::{Level, Theme};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// Exhaustive panic / bounds safety test: every object, every state,
    /// arbitrary progress values (including out-of-range and NaN), arbitrary
    /// facings (including zero), and on-screen / edge / far-off-screen positions.
    #[test]
    fn all_traps_draw_safely_everywhere_without_panic() {
        let mut pb = PixelBuffer::new(48, 36);
        let positions = [
            Vec2::new(24.0, 18.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(47.5, 35.5),
            Vec2::new(-80.0, 15.0),
            Vec2::new(400.0, -200.0),
            Vec2::new(10000.0, 10000.0),
        ];
        let facings = [
            Vec2::ZERO,
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.6, -0.8),
        ];
        let progresses = [-0.5, 0.0, 0.25, 0.5, 0.85, 1.0, 1.5, f32::NAN];
        let accent = (90, 220, 200);

        for &pos in &positions {
            for &facing in &facings {
                for &prog in &progresses {
                    for state in [
                        StickyTrapState::Idle,
                        StickyTrapState::Stuck,
                        StickyTrapState::Used,
                    ] {
                        let v = StickyTrapView {
                            pos,
                            facing,
                            state,
                            progress: prog,
                            anim: 1.7,
                            victim_pos: pos + Vec2::new(5.0, -2.0),
                            victim_radius: 4.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, Vec2::ZERO, accent);
                        draw_sticky_trap(&mut pb, &v, Vec2::ZERO, accent);
                    }
                    for state in [
                        MousetrapState::Armed,
                        MousetrapState::Triggered,
                        MousetrapState::Snapping,
                        MousetrapState::Sprung,
                    ] {
                        let v = MousetrapView {
                            pos,
                            facing,
                            state,
                            progress: prog,
                            anim: 2.3,
                            hit_flash: prog,
                        };
                        draw_mousetrap_ground(&mut pb, &v, Vec2::ZERO, accent);
                        draw_mousetrap(&mut pb, &v, Vec2::ZERO, accent);
                    }
                    for state in [
                        OneWayDoorState::Closed,
                        OneWayDoorState::Opening,
                        OneWayDoorState::Closing,
                        OneWayDoorState::Refused,
                    ] {
                        let v = OneWayDoorView {
                            pos,
                            facing,
                            state,
                            progress: prog,
                            anim: 0.9,
                            hit_flash: prog,
                        };
                        draw_one_way_door_ground(&mut pb, &v, Vec2::ZERO, accent);
                        draw_one_way_door(&mut pb, &v, Vec2::ZERO, accent);
                    }
                    for state in [
                        RatHoleState::Idle,
                        RatHoleState::Squeezing,
                        RatHoleState::Blocked,
                    ] {
                        let v = RatHoleView {
                            pos,
                            facing,
                            state,
                            progress: prog,
                            anim: 1.1,
                            wall_tint: Some((60, 80, 76)),
                        };
                        draw_rat_hole_ground(&mut pb, &v, Vec2::ZERO, accent);
                        draw_rat_hole(&mut pb, &v, Vec2::ZERO, accent);
                    }
                    for state in [
                        MazeRewardState::Idle,
                        MazeRewardState::Opening,
                        MazeRewardState::Opened,
                    ] {
                        let v = MazeRewardView {
                            pos,
                            facing,
                            state,
                            progress: prog,
                            anim: 3.1,
                            reward_tint: None,
                        };
                        draw_maze_reward_ground(&mut pb, &v, Vec2::ZERO, accent);
                        draw_maze_reward(&mut pb, &v, Vec2::ZERO, accent);
                    }
                }
            }
        }
    }

    /// Zero-sized buffer must also be a clean no-op.
    #[test]
    fn zero_sized_buffer_is_safe() {
        let mut pb = PixelBuffer::new(0, 0);
        let accent = (120, 220, 255);
        let st = StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0));
        draw_sticky_trap_ground(&mut pb, &st, Vec2::ZERO, accent);
        draw_sticky_trap(&mut pb, &st, Vec2::ZERO, accent);
        let mt = MousetrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0));
        draw_mousetrap_ground(&mut pb, &mt, Vec2::ZERO, accent);
        draw_mousetrap(&mut pb, &mt, Vec2::ZERO, accent);
        let ow = OneWayDoorView::new(Vec2::ZERO, Vec2::new(1.0, 0.0));
        draw_one_way_door_ground(&mut pb, &ow, Vec2::ZERO, accent);
        draw_one_way_door(&mut pb, &ow, Vec2::ZERO, accent);
        let rh = RatHoleView::new(Vec2::ZERO, Vec2::new(0.0, 1.0));
        draw_rat_hole_ground(&mut pb, &rh, Vec2::ZERO, accent);
        draw_rat_hole(&mut pb, &rh, Vec2::ZERO, accent);
        let mr = MazeRewardView::new(Vec2::ZERO, Vec2::new(0.0, 1.0));
        draw_maze_reward_ground(&mut pb, &mr, Vec2::ZERO, accent);
        draw_maze_reward(&mut pb, &mr, Vec2::ZERO, accent);
    }

    /// Verify collision helper agreements: rat fits through rat-hole, Brute does not;
    /// local_box_contains agrees with facing rotation.
    #[test]
    fn collision_helpers_match_design_spec() {
        assert!(fits_through_rat_hole(crate::game::art_rat::BODY_RADIUS));
        assert!(fits_through_rat_hole(EnemyKind::Skitterer.radius()));
        assert!(fits_through_rat_hole(EnemyKind::Bat.radius()));
        assert!(fits_through_rat_hole(EnemyKind::Spitter.radius()));
        assert!(!fits_through_rat_hole(EnemyKind::Cat.radius()));
        assert!(!fits_through_rat_hole(EnemyKind::Brute.radius()));
        assert!(!fits_through_rat_hole(EnemyKind::Boss.radius()));

        let origin = Vec2::new(50.0, 50.0);
        let down = Vec2::new(0.0, 1.0);
        // Point 5px down (+u) is inside the mousetrap kill box; 5px up (-u) is not.
        assert!(local_box_contains(
            origin,
            down,
            Vec2::new(50.0, 55.0),
            MOUSETRAP_KILL_U_MIN,
            MOUSETRAP_KILL_U_MAX,
            MOUSETRAP_KILL_HALF_V,
        ));
        assert!(!local_box_contains(
            origin,
            down,
            Vec2::new(50.0, 45.0),
            MOUSETRAP_KILL_U_MIN,
            MOUSETRAP_KILL_U_MAX,
            MOUSETRAP_KILL_HALF_V,
        ));
    }

    /// Visual sheet test: `RATRUN_DUMP=traps cargo test dump_traps_sheet`
    /// writes `/tmp/ratrun_traps.ppm` showing all 5 interactive objects across
    /// every state, multiple progress values and facings, over all three themes'
    /// floors, alongside the player rat and an armored Brute for scale.
    #[test]
    fn dump_traps_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("traps") {
            return;
        }
        const CELL_W: usize = 50;
        const CELL_H: usize = 36;
        const COLS: usize = 7;
        const ROWS: usize = 5;
        let w = CELL_W * COLS;
        let h = CELL_H * ROWS;
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();

        let make_rat = |pos: Vec2, face: Vec2| {
            let mut p = Player::new(pos, WeaponKind::Gnaw);
            p.face = face;
            p.vel = face * 40.0;
            p
        };
        let make_brute = |pos: Vec2, face: Vec2| {
            let mut rng = StdRng::seed_from_u64(42);
            let mut e = Enemy::spawn(EnemyKind::Brute, pos, 1.0, &mut rng);
            e.facing = face;
            e.anim = 1.2;
            e
        };

        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let level = Level::new(theme);
            let accent = level.palette.accent;
            let wall_col = Some(level.palette.wall);
            let cam = Vec2::new(260.0, 260.0);
            let mut pb = PixelBuffer::new(w, h);
            draw_floor(&mut pb, &level, cam, 1.2);

            let cell_center = |row: usize, col: usize| -> Vec2 {
                cam + Vec2::new(
                    (col * CELL_W + CELL_W / 2) as f32,
                    (row * CELL_H + CELL_H / 2) as f32,
                )
            };

            // Row 0: Sticky Trap (Idle x2, Stuck w/ Rat x2, Stuck w/ Brute, Used x2)
            for col in 0..COLS {
                let c = cell_center(0, col);
                match col {
                    0 => {
                        let v = StickyTrapView {
                            pos: c,
                            facing: Vec2::new(1.0, 0.0),
                            state: StickyTrapState::Idle,
                            progress: 0.0,
                            anim: 0.2,
                            victim_pos: c,
                            victim_radius: 3.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                    1 => {
                        let v = StickyTrapView {
                            pos: c,
                            facing: Vec2::from_angle(0.38),
                            state: StickyTrapState::Idle,
                            progress: 0.0,
                            anim: 1.4,
                            victim_pos: c,
                            victim_radius: 3.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                    2 => {
                        let rat_pos = c + Vec2::new(4.5, -1.5);
                        let v = StickyTrapView {
                            pos: c,
                            facing: Vec2::new(1.0, 0.0),
                            state: StickyTrapState::Stuck,
                            progress: 0.28,
                            anim: 1.1,
                            victim_pos: rat_pos,
                            victim_radius: 3.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        let p = make_rat(rat_pos, Vec2::new(1.0, -0.3));
                        draw_rat(&mut pb, &p, 0.7, cam);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                    3 => {
                        let board_c = c - Vec2::new(4.0, 0.0);
                        let rat_pos = board_c + Vec2::new(13.5, -2.5);
                        let v = StickyTrapView {
                            pos: board_c,
                            facing: Vec2::new(1.0, 0.0),
                            state: StickyTrapState::Stuck,
                            progress: 0.68,
                            anim: 2.6,
                            victim_pos: rat_pos,
                            victim_radius: 3.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        let p = make_rat(rat_pos, Vec2::new(1.0, -0.2));
                        draw_rat(&mut pb, &p, 1.2, cam);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                    4 => {
                        let board_c = c - Vec2::new(5.0, 0.0);
                        let brute_pos = board_c + Vec2::new(12.5, -1.0);
                        let v = StickyTrapView {
                            pos: board_c,
                            facing: Vec2::new(1.0, 0.0),
                            state: StickyTrapState::Stuck,
                            progress: 0.55,
                            anim: 1.9,
                            victim_pos: brute_pos,
                            victim_radius: 6.5,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        let b = make_brute(brute_pos, Vec2::new(1.0, 0.0));
                        draw_enemy(&mut pb, &b, cam, accent);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                    5 => {
                        let v = StickyTrapView {
                            pos: c,
                            facing: Vec2::new(1.0, 0.0),
                            state: StickyTrapState::Used,
                            progress: 1.0,
                            anim: 0.0,
                            victim_pos: c,
                            victim_radius: 3.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                    _ => {
                        let v = StickyTrapView {
                            pos: c,
                            facing: Vec2::new(0.0, 1.0),
                            state: StickyTrapState::Used,
                            progress: 1.0,
                            anim: 0.0,
                            victim_pos: c,
                            victim_radius: 3.0,
                        };
                        draw_sticky_trap_ground(&mut pb, &v, cam, accent);
                        draw_sticky_trap(&mut pb, &v, cam, accent);
                    }
                }
            }

            // Row 1: Mousetrap (Armed R, Armed Up, Triggered 0.4, Triggered 0.85,
            //                   Snapping 0.22 mid-air, Snapping 0.52 SLAM, Sprung)
            for col in 0..COLS {
                let c = cell_center(1, col);
                let (facing, state, prog, flash) = match col {
                    0 => (Vec2::new(1.0, 0.0), MousetrapState::Armed, 0.0, 0.0),
                    1 => (Vec2::new(0.0, -1.0), MousetrapState::Armed, 0.0, 0.0),
                    2 => (Vec2::new(1.0, 0.0), MousetrapState::Triggered, 0.40, 0.0),
                    3 => (Vec2::new(1.0, 0.0), MousetrapState::Triggered, 0.88, 0.0),
                    4 => (Vec2::new(1.0, 0.0), MousetrapState::Snapping, 0.22, 0.2),
                    5 => (Vec2::new(1.0, 0.0), MousetrapState::Snapping, 0.50, 0.8),
                    _ => (Vec2::new(1.0, 0.0), MousetrapState::Sprung, 1.0, 0.0),
                };
                let v = MousetrapView {
                    pos: c,
                    facing,
                    state,
                    progress: prog,
                    anim: 1.3 + col as f32 * 0.4,
                    hit_flash: flash,
                };
                draw_mousetrap_ground(&mut pb, &v, cam, accent);
                draw_mousetrap(&mut pb, &v, cam, accent);
            }

            // Row 2: One-Way Door (Closed R, Closed Down, Opening 0.45, Opening 0.95 + Rat,
            //                      Closing 0.55, Refused 0.22 + Rat wrong side, Refused 0.62)
            for col in 0..COLS {
                let c = cell_center(2, col);
                let (facing, state, prog) = match col {
                    0 => (Vec2::new(1.0, 0.0), OneWayDoorState::Closed, 0.0),
                    1 => (Vec2::new(0.0, 1.0), OneWayDoorState::Closed, 0.0),
                    2 => (Vec2::new(1.0, 0.0), OneWayDoorState::Opening, 0.45),
                    3 => (Vec2::new(1.0, 0.0), OneWayDoorState::Opening, 0.95),
                    4 => (Vec2::new(1.0, 0.0), OneWayDoorState::Closing, 0.55),
                    5 => (Vec2::new(1.0, 0.0), OneWayDoorState::Refused, 0.22),
                    _ => (Vec2::new(1.0, 0.0), OneWayDoorState::Refused, 0.62),
                };
                let v = OneWayDoorView {
                    pos: c,
                    facing,
                    state,
                    progress: prog,
                    anim: 0.6 + col as f32 * 0.3,
                    hit_flash: 0.0,
                };
                draw_one_way_door_ground(&mut pb, &v, cam, accent);
                if col == 3 {
                    let p = make_rat(c + Vec2::new(1.5, 0.0), Vec2::new(1.0, 0.0));
                    draw_rat(&mut pb, &p, 0.5, cam);
                } else if col == 5 {
                    // Rat pushing against the wrong (+u) side!
                    let p = make_rat(c + Vec2::new(8.5, 0.0), Vec2::new(-1.0, 0.0));
                    draw_rat(&mut pb, &p, 0.9, cam);
                }
                draw_one_way_door(&mut pb, &v, cam, accent);
            }

            // Row 3: Rat-Hole (Idle H-wall, Idle V-wall, Squeezing + Rat H, Squeezing + Rat V,
            //                  Blocked 0.25 + Brute, Blocked 0.60 + Brute, Scale: Rat + Hole + Brute)
            for col in 0..COLS {
                let c = cell_center(3, col);
                match col {
                    0 => {
                        let v = RatHoleView {
                            pos: c,
                            facing: Vec2::new(0.0, 1.0),
                            state: RatHoleState::Idle,
                            progress: 0.0,
                            anim: 0.5,
                            wall_tint: wall_col,
                        };
                        draw_rat_hole_ground(&mut pb, &v, cam, accent);
                        draw_rat_hole(&mut pb, &v, cam, accent);
                    }
                    1 => {
                        let v = RatHoleView {
                            pos: c,
                            facing: Vec2::new(1.0, 0.0),
                            state: RatHoleState::Idle,
                            progress: 0.0,
                            anim: 0.5,
                            wall_tint: wall_col,
                        };
                        draw_rat_hole_ground(&mut pb, &v, cam, accent);
                        draw_rat_hole(&mut pb, &v, cam, accent);
                    }
                    2 => {
                        let v = RatHoleView {
                            pos: c,
                            facing: Vec2::new(0.0, 1.0),
                            state: RatHoleState::Squeezing,
                            progress: 0.45,
                            anim: 1.0,
                            wall_tint: wall_col,
                        };
                        draw_rat_hole_ground(&mut pb, &v, cam, accent);
                        let p = make_rat(c + Vec2::new(0.0, 2.0), Vec2::new(0.0, 1.0));
                        draw_rat(&mut pb, &p, 0.6, cam);
                        draw_rat_hole(&mut pb, &v, cam, accent);
                    }
                    3 => {
                        let v = RatHoleView {
                            pos: c,
                            facing: Vec2::new(1.0, 0.0),
                            state: RatHoleState::Squeezing,
                            progress: 0.50,
                            anim: 1.2,
                            wall_tint: wall_col,
                        };
                        draw_rat_hole_ground(&mut pb, &v, cam, accent);
                        let p = make_rat(c + Vec2::new(2.5, 0.0), Vec2::new(1.0, 0.0));
                        draw_rat(&mut pb, &p, 0.8, cam);
                        draw_rat_hole(&mut pb, &v, cam, accent);
                    }
                    4 => {
                        let hole_c = c - Vec2::new(4.0, 0.0);
                        let v = RatHoleView {
                            pos: hole_c,
                            facing: Vec2::new(1.0, 0.0),
                            state: RatHoleState::Blocked,
                            progress: 0.25,
                            anim: 1.5,
                            wall_tint: wall_col,
                        };
                        draw_rat_hole_ground(&mut pb, &v, cam, accent);
                        let b = make_brute(hole_c + Vec2::new(11.5, 0.0), Vec2::new(-1.0, 0.0));
                        draw_enemy(&mut pb, &b, cam, accent);
                        draw_rat_hole(&mut pb, &v, cam, accent);
                    }
                    5 => {
                        let hole_c = c - Vec2::new(4.0, 0.0);
                        let v = RatHoleView {
                            pos: hole_c,
                            facing: Vec2::new(1.0, 0.0),
                            state: RatHoleState::Blocked,
                            progress: 0.60,
                            anim: 1.8,
                            wall_tint: wall_col,
                        };
                        draw_rat_hole_ground(&mut pb, &v, cam, accent);
                        let b = make_brute(hole_c + Vec2::new(12.5, 0.0), Vec2::new(-1.0, 0.0));
                        draw_enemy(&mut pb, &b, cam, accent);
                        draw_rat_hole(&mut pb, &v, cam, accent);
                    }
                    _ => {
                        // Scale lineup: Rat (left), Brute (right) facing up
                        let p = make_rat(c + Vec2::new(-9.0, 2.0), Vec2::new(0.0, -1.0));
                        draw_rat(&mut pb, &p, 0.3, cam);
                        let b = make_brute(c + Vec2::new(8.0, 0.0), Vec2::new(0.0, -1.0));
                        draw_enemy(&mut pb, &b, cam, accent);
                    }
                }
            }

            // Row 4: Maze Reward (Idle 0, Idle 1.4, Opening 0.20, Opening 0.55,
            //                     Opening 0.85, Opened, Scale w/ Rat + Brute)
            for col in 0..COLS {
                let c = cell_center(4, col);
                let (state, prog, anim) = match col {
                    0 => (MazeRewardState::Idle, 0.0, 0.1),
                    1 => (MazeRewardState::Idle, 0.0, 1.5),
                    2 => (MazeRewardState::Opening, 0.20, 0.6),
                    3 => (MazeRewardState::Opening, 0.55, 1.0),
                    4 => (MazeRewardState::Opening, 0.85, 1.4),
                    5 => (MazeRewardState::Opened, 1.0, 2.0),
                    _ => (MazeRewardState::Idle, 0.0, 0.8),
                };
                let v = MazeRewardView {
                    pos: c,
                    facing: Vec2::new(0.0, 1.0),
                    state,
                    progress: prog,
                    anim,
                    reward_tint: None,
                };
                draw_maze_reward_ground(&mut pb, &v, cam, accent);
                if col == 6 {
                    let p = make_rat(c + Vec2::new(-16.0, 2.0), Vec2::new(1.0, 0.0));
                    draw_rat(&mut pb, &p, 0.4, cam);
                    let b = make_brute(c + Vec2::new(16.5, 0.0), Vec2::new(-1.0, 0.0));
                    draw_enemy(&mut pb, &b, cam, accent);
                }
                draw_maze_reward(&mut pb, &v, cam, accent);
            }

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_traps.ppm", out).unwrap();
    }
}
