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
//! | Sticky Trap    | `36.0 × 28.0` (pad `30 × 22`) | No (walkable floor hazard; Bats fly over) |
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
pub const STICKY_TRAP_SIZE: Vec2 = Vec2 { x: 36.0, y: 28.0 };
/// Active adhesive inner pad half-extents (`|u| <= 15.0`, `|v| <= 11.0`).
pub const STICKY_TRAP_PAD_HALF: Vec2 = Vec2 { x: 15.0, y: 11.0 };
/// Sticky traps never block movement; they apply a heavy slow/root on entry.
pub const STICKY_TRAP_BLOCKS_MOVEMENT: bool = false;
/// Movement speed multiplier applied while trapped in `StickyTrapState::Stuck`.
pub const STICKY_TRAP_SLOW_MULT: f32 = 0.20;
/// Hold durations (seconds) by victim class.
pub const STICKY_TRAP_HOLD_PLAYER_SECS: f32 = 1.6;
pub const STICKY_TRAP_HOLD_SMALL_ENEMY_SECS: f32 = 2.5;
pub const STICKY_TRAP_HOLD_BRUTE_SECS: f32 = 1.5;

/// Glue **units** a board can hold at once (a crowd-control budget, not a body
/// count: a Skitterer is 1 unit, a Cat 2, a Brute 4).
///
/// `STICKY_TRAP_SLOTS` is the same number of *bodies*: the whole capacity spent
/// on the smallest bodies. Both are fixed, so the victim storage is a fixed-size
/// array and the simulation allocates nothing per tick.
pub const STICKY_TRAP_CAPACITY: u8 = 8;
/// Fixed victim slots on one board (`STICKY_TRAP_CAPACITY` one-unit bodies).
pub const STICKY_TRAP_SLOTS: usize = STICKY_TRAP_CAPACITY as usize;
/// Glue units a board spends over its whole life before it is `Used`.
///
/// A catch costs the victim's units from this budget; the units are gone even
/// after the victim tears free, so a fresh board pins a dozen to twenty small
/// bodies before it is spent (8 units in one go at most).
pub const STICKY_TRAP_BUDGET: u8 = 20;
/// Glue units the rat takes (it no longer spends a whole board by itself).
pub const STICKY_TRAP_RAT_UNITS: u8 = 2;
/// Seconds a board ignores a body that just tore free of it, so a victim that
/// is still standing on the pad cannot be caught again (a re-catch would burn
/// the whole lifetime budget on one enemy).
pub const STICKY_TRAP_RELEASE_GRACE_SECS: f32 = 1.2;
/// How many torn-patch scars the art keeps (where victims pulled free).
pub const STICKY_TRAP_TEARS: usize = 6;


/// Full wooden baseboard footprint of a mousetrap (`u` length × `v` width).
///
/// Wave 3 made the snap trap roughly twice the old `18 × 11` board: a real
/// spring trap next to a rat, with the striker bar's swing covering most of the
/// deck. Every trigger/kill/art extent below is measured from this one pair of
/// numbers, so the drawn board, the plate and the kill zone cannot drift.
/// `+facing` (`+u`) is the baited kill end; `-facing` (`-u`) holds the cocked bar.
pub const MOUSETRAP_SIZE: Vec2 = Vec2 { x: 40.0, y: 24.0 };
/// Mousetraps do not block movement.
pub const MOUSETRAP_BLOCKS_MOVEMENT: bool = false;
/// Center offset along `+facing` of the copper bait trigger plate.
pub const MOUSETRAP_TRIGGER_OFFSET_U: f32 = 8.0;
/// Circular trigger radius around `pos + facing * MOUSETRAP_TRIGGER_OFFSET_U`.
pub const MOUSETRAP_TRIGGER_RADIUS: f32 = 6.5;
/// Dangerous snap-hit box along local `u` (`-4.0..=18.0`) and `|v| <= 10.0`: the
/// area the striker bar covers when it lands, inset from the board's `20 × 12`
/// half-extents so a rat at the very rim survives.
pub const MOUSETRAP_KILL_U_MIN: f32 = -4.0;
pub const MOUSETRAP_KILL_U_MAX: f32 = 18.0;
pub const MOUSETRAP_KILL_HALF_V: f32 = 10.0;
/// Warning duration of an **ordinary** trap before the striker bar slams shut
/// (seconds).
///
/// The bar's damage lands at `MOUSETRAP_HIT_PROGRESS_MIN` of the `0.20 s`
/// swing, so the rat's real window from touching the plate to the bite is
/// `0.18 + 0.08 = 0.26 s`: a walking rat covers ~15 px and cannot clear the
/// `22 × 20` kill rect from the plate, while a dash (`3.2 ×` speed) is already
/// 48 px away. The pre-wave-3 trap warned for `0.28 s`, which a walking rat
/// *could* survive on the bigger board — this is the "slightly tighter" value
/// the wave-3 brief allows.
pub const MOUSETRAP_TRIGGER_SECS: f32 = 0.18;
/// Total snap swing + impact animation duration (seconds).
///
/// A hair-trigger snap resolves its damage on the touch tick and then plays this
/// swing for the eye: the bar is already on its way when the noise starts.
pub const MOUSETRAP_SNAP_SECS: f32 = 0.20;
/// Active damage window within `MousetrapState::Snapping` (`progress` range).
pub const MOUSETRAP_HIT_PROGRESS_MIN: f32 = 0.40;
pub const MOUSETRAP_HIT_PROGRESS_MAX: f32 = 0.65;
/// Damage dealt by an **ordinary** snap, as a share of the rat's max HP.
///
/// Before wave 3 this was a flat 20 (a knock, not a threat). Trap damage reads
/// as a share of the pool so it keeps biting late builds, and only half of the
/// rat's flat armour applies to it (a spring bar does not care about a thick
/// hide) — see `traps::snap_player_damage`.
pub const MOUSETRAP_DAMAGE_PLAYER_SHARE: f32 = 0.35;
/// Damage dealt by a **hair-trigger** snap (share of the rat's max HP).
///
/// Never 1.0: no trap may take a rat from full health in one hit, however thin
/// the build. `0.65` leaves 35 % and a decision to make.
pub const MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE: f32 = 0.65;
/// Share of the rat's flat armour a snap ignores (half).
pub const MOUSETRAP_ARMOUR_IGNORED: f32 = 0.5;
/// Heavy damage dealt to enemies so luring Cats/Brutes into traps is rewarding
/// (a Brute at 1× is 60 HP: one ordinary bar kills it outright).
pub const MOUSETRAP_DAMAGE_ENEMY: f32 = 80.0;
/// Hair-trigger bar damage to ground enemies: enough to kill a Brute three times
/// over and to chunk a named miniboss in one hit.
pub const MOUSETRAP_DAMAGE_ENEMY_HAIR: f32 = 150.0;
/// Plate radius the rat must touch to take the bait (its centre inside it).
pub const MOUSETRAP_BAIT_TAKE_RADIUS: f32 = 7.0;

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
    /// - **Active area**: Oriented rectangle `|u| <= 15.0`, `|v| <= 11.0` relative
    ///   to `pos` and `facing` (`STICKY_TRAP_PAD_HALF`). Does NOT block movement.
    /// - **Damage**: `0`.
    /// - **Counter**: Walk around it, or **Dash** directly across it (an active
    ///   dash clears the pad without sticking).
    Idle,

    /// **One or more** victims (the player rat and/or ground enemies) are bogged
    /// down in the adhesive, pulling stretching glue strands as they struggle
    /// free.
    ///
    /// - **Capacity**: a board holds [`STICKY_TRAP_CAPACITY`] (8) glue *units*
    ///   at once — a Skitterer 1, a Cat 2, a Brute 4 — and a lifetime budget of
    ///   [`STICKY_TRAP_BUDGET`] (20) units, so it pins a dozen to twenty small
    ///   bodies over its life. `victims` carries one entry per fixed pad slot
    ///   (the sim pins each ground victim to its slot).
    /// - **Duration**: `1.6s` for the player rat (`STICKY_TRAP_HOLD_PLAYER_SECS`),
    ///   `2.5s` for Skitterers/Cats/Spitters (`STICKY_TRAP_HOLD_SMALL_ENEMY_SECS`),
    ///   `1.5s` for Brutes (`STICKY_TRAP_HOLD_BRUTE_SECS`), each after its own
    ///   catch; then that victim tears free and its capacity comes back (its
    ///   spent glue does not). `progress` is the most-progressed victim's hold.
    ///   Bosses and named minibosses are only slowed while crossing (`0.8s` at
    ///   40 %) and never enter this state.
    /// - **Warning / Readability**: Disturbed rippled glue pool with thick
    ///   glistening amber-ivory strands anchoring the board to each victim, and
    ///   a tinted glue mound under every sunk body so the load reads at a
    ///   glance; strands elongate, thin, vibrate, and snap past `progress > 0.78`.
    /// - **Active area**: every body on the pad that fits the room left. A body
    ///   too big for the room (or arriving once the budget is gone) is only
    ///   slowed while it crosses.
    /// - **Effect & Damage**: Holds each victim at `20%` movement
    ///   (`STICKY_TRAP_SLOW_MULT`) and deals `0` direct damage.
    /// - **Counter**: Pressing **Dash** immediately breaks the player free
    ///   (spending dash cooldown; the board keeps its remaining glue), and Clean
    ///   Socks ignore the pad entirely. Conversely, players can lure a whole
    ///   pack onto a board and kill it there.
    Stuck,

    /// Spent, torn, dusty glue board: its lifetime glue budget is gone **and**
    /// its last victim has left.
    ///
    /// - **Duration**: Permanent floor decal (or optional fade after `30.0s`).
    /// - **Visuals**: Scuffed/bent cardboard rim, matte gray-tan torn adhesive
    ///   with lint flecks and short curled stub strands — no wet gloss.
    /// - **Active area / Damage**: None (completely harmless).
    Used,
}

/// How big a stuck body reads on the pad. The art uses it for the glue mound
/// the victim is sunk into (a Brute lump is visibly wider than a Skitterer's),
/// and the simulation uses it to size a victim's own pad slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickyVictimKind {
    /// The player rat (`3.0` body radius).
    Rat,
    /// Skitterer / Bell Flea / small bodies (`1` glue unit).
    Small,
    /// Cat / Spitter / Pillbug / Porter / Turnkey (`2` glue units).
    Medium,
    /// Brute and elites (`4` glue units).
    Big,
}

impl StickyVictimKind {
    /// Drawn body radius on the pad, in world px.
    pub fn body_radius(self) -> f32 {
        match self {
            StickyVictimKind::Rat => 3.2,
            StickyVictimKind::Small => 2.6,
            StickyVictimKind::Medium => 4.2,
            StickyVictimKind::Big => 6.5,
        }
    }

    /// Width of the glue mound the body is sunk into.
    pub fn mound_radius(self) -> f32 {
        self.body_radius() + 1.6
    }
}

/// One body currently stuck on a glue board, drawn at its own pad slot.
///
/// The sim writes these; the art reads them. `pos` is the slot position (the
/// sim pins ground victims there) so the load is stable and the drawing has no
/// ties to break.
#[derive(Clone, Copy, Debug)]
pub struct StickyVictimView {
    /// World-space slot position on the pad.
    pub pos: Vec2,
    /// Collision radius of the body (strands anchor onto its flanks).
    pub radius: f32,
    /// Body class, for the drawn silhouette/mound size.
    pub kind: StickyVictimKind,
    /// The victim's own colour (`EnemyKind::color`, or a rat tone), so a
    /// Skitterer and a Brute read apart by hue as well as by size.
    pub color: Rgb,
    /// `0.0..=1.0` through this victim's own hold (its strand stretch driver).
    pub progress: f32,
}

/// View data required to draw a sticky trap. `Copy` and simulation-independent.
#[derive(Clone, Copy, Debug)]
pub struct StickyTrapView {
    /// World-space center of the glue board.
    pub pos: Vec2,
    /// Unit direction along the board's long (`36px`) axis.
    pub facing: Vec2,
    /// Current trap state.
    pub state: StickyTrapState,
    /// Normalized progress `0.0..=1.0` through the current state (used in `Stuck`
    /// to stretch and snap strands: the *most-progressed* victim's hold).
    pub progress: f32,
    /// Monotonic clock in seconds for specular gloss drift and strand quiver.
    pub anim: f32,
    /// World-space position of the primary (most-progressed) stuck victim (used
    /// in `Stuck`). If equal to `pos`, strands radiate gently from the board
    /// center.
    pub victim_pos: Vec2,
    /// Collision radius of the primary stuck victim (e.g. `3.0` rat, `6.5`
    /// Brute) so strands anchor cleanly onto its flanks.
    pub victim_radius: f32,
    /// Glue units currently held (`0..=STICKY_TRAP_CAPACITY`).
    pub load: u8,
    /// Share of the lifetime glue budget already spent, `0.0` fresh to `1.0`
    /// spent: drives the sheen fading, the coverage going patchy and the debris.
    pub worn: f32,
    /// The bodies currently stuck, one per fixed slot (holes are `None`). Slot
    /// index is the body's pad slot, so a position never moves mid-hold.
    pub victims: [Option<StickyVictimView>; STICKY_TRAP_SLOTS],
    /// Where glue was torn away by victims pulling free (ring buffer, world
    /// space); the art draws a ripped scar at each.
    pub tears: [Vec2; STICKY_TRAP_TEARS],
    /// How many entries of `tears` are live (`0..=STICKY_TRAP_TEARS`).
    pub tear_n: u8,
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
            load: 0,
            worn: 0.0,
            victims: [None; STICKY_TRAP_SLOTS],
            tears: [pos; STICKY_TRAP_TEARS],
            tear_n: 0,
        }
    }

    /// Push a torn-glue scar (a victim pulled free of this spot).
    pub fn add_tear(&mut self, at: Vec2) {
        let n = STICKY_TRAP_TEARS;
        if (self.tear_n as usize) < n {
            self.tears[self.tear_n as usize] = at;
            self.tear_n += 1;
        } else {
            // Full: overwrite the oldest (the ring shifts down by one).
            let mut i = 1;
            while i < n {
                self.tears[i - 1] = self.tears[i];
                i += 1;
            }
            self.tears[n - 1] = at;
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
    ///   Cat, Spitter, or Brute; Bats fly over) touches the trigger plate.
    /// - **Warning**: Warm pine `40 × 24` board, gleaming taut steel U-bar cocked
    ///   back on `-facing`, coiled torsion spring, and the **bait** on `+facing`
    ///   (cheese, a good floor pickup, or a small chest — level data, see
    ///   [`TrapBait`]). A hair trigger ([`MousetrapKind::HairTrigger`]) shows a
    ///   thin quivering filed wire catch and a red-painted bar instead.
    /// - **Trigger area**: Circle of radius `6.5` (`MOUSETRAP_TRIGGER_RADIUS`)
    ///   centered at `pos + facing * 8.0` (`MOUSETRAP_TRIGGER_OFFSET_U`).
    ///   Does NOT block movement.
    /// - **Bait**: the rat's touch collects it through the normal pickup / chest
    ///   path and springs the trap in the same tick.
    /// - **Damage**: `0` while armed.
    /// - **Counter**: Walk around, or dash across the bait to take the reward
    ///   and leave the bar behind you against pursuing enemies.
    Armed,

    /// Hairline pre-snap telegraph: the copper bait pedal has dipped and the
    /// cocked steel striker bar shudders violently before releasing.
    ///
    /// - **Duration**: `0.18s` (`MOUSETRAP_TRIGGER_SECS`), advancing `progress`
    ///   `0.0 -> 1.0`, then immediately entering `Snapping`. A **hair trigger**
    ///   never enters this state: its bar resolves on the touch tick.
    /// - **Warning**: High-frequency jitter of the cocked steel bar, bright
    ///   spring tension sparks, and a pulsing crimson-amber ground danger bracket
    ///   marking the exact `+facing` kill rectangle
    ///   (`u in [-4.0, 18.0], |v| <= 10.0`, `MOUSETRAP_KILL_*`).
    /// - **Damage**: `0` during warning. The rat's real window from plate to bite
    ///   is `0.18 + 0.08 = 0.26 s`: a walk covers ~15 px (still inside the rect),
    ///   a dash (`3.2 ×` speed) is 48 px away.
    /// - **Counter**: Step clear of the `+facing` kill rectangle or Dash.
    Triggered,

    /// The spring-steel striker bar whips 180° over the hinge and slams down
    /// onto the bait plate.
    ///
    /// - **Duration**: `0.20s` (`MOUSETRAP_SNAP_SECS`), advancing `progress`
    ///   `0.0 -> 1.0`, then transitioning to `Sprung`.
    /// - **Active hit window**: `progress` in `0.40..=0.65` (impact moment; each
    ///   overlapping entity is damaged at most once). A hair trigger resolved its
    ///   damage already and never hits again.
    /// - **Dangerous area**: Oriented rectangle `u in [-4.0, 18.0]`, `|v| <= 10.0`
    ///   relative to `pos` and `facing` (`MOUSETRAP_KILL_*`).
    /// - **Damage**:
    ///   - Player rat: a share of its own pool — `35 %` ordinary
    ///     (`MOUSETRAP_DAMAGE_PLAYER_SHARE`), `65 %` hair trigger
    ///     (`MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE`) — with **half** its flat armour
    ///     ignored and never a one-shot from full health.
    ///   - Ground enemies: `80` damage ordinary (`MOUSETRAP_DAMAGE_ENEMY`), `150`
    ///     hair trigger (`MOUSETRAP_DAMAGE_ENEMY_HAIR`): a Brute dies to either
    ///     bar and a named miniboss takes a real chunk, once per snap.
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

/// Which bar a set mousetrap carries. **Readable before committing**: a
/// hair-trigger trap has a thin filed wire catch and a red-painted bar, and it
/// resolves its damage on the touch tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MousetrapKind {
    /// The ordinary trap: a `0.18 s` reaction window after the plate is touched,
    /// then a hard but survivable bar.
    Ordinary,
    /// No window at all: the bar lands on the tick the plate is touched, so
    /// only a dash that is already under way (its i-frames) gets through.
    HairTrigger,
}

impl MousetrapKind {
    /// Is this the instant kind?
    #[inline]
    pub fn hair(self) -> bool {
        matches!(self, MousetrapKind::HairTrigger)
    }

    /// Warning after the plate is touched, in seconds (`0.0` = instant).
    #[inline]
    pub fn reaction_secs(self) -> f32 {
        match self {
            MousetrapKind::Ordinary => MOUSETRAP_TRIGGER_SECS,
            MousetrapKind::HairTrigger => 0.0,
        }
    }
}

/// What sits on a set mousetrap's trigger plate.
///
/// Level data (chosen once at generation from the level's own seeded stream),
/// never rolled at runtime: the player can read the bait from a distance and
/// decide whether the trap is worth the risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapBait {
    /// Nothing but a crumb of cheese, exactly like the old trap. Sprung for the
    /// fun of it (enemies still set it off); taking it is free of reward.
    Cheese,
    /// A floor pickup, collected through the normal pickup path (minimap tag,
    /// HUD callout and effect all come from that one place).
    Pickup(crate::game::gem::PickupKind),
    /// A miniboss chest: collecting it raises `pending_chests`, so it opens the
    /// normal chest card (and can evolve a weapon).
    Chest,
}

impl TrapBait {
    /// Worth dashing through an ordinary trap for?
    #[inline]
    pub fn is_good(self) -> bool {
        !matches!(self, TrapBait::Cheese)
    }

    /// The label the HUD's telemetry / the minimap blip uses.
    pub fn label(self) -> &'static str {
        match self {
            TrapBait::Cheese => "CHEESE",
            TrapBait::Pickup(kind) => kind.label(),
            TrapBait::Chest => "CHEST",
        }
    }
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
    /// Ordinary bar or hair-trigger (level data, never rolled at runtime).
    pub kind: MousetrapKind,
    /// What sits on the plate while it is set (level data).
    pub bait: TrapBait,
    /// The rat has taken the bait (it stays lying on a sprung trap).
    pub bait_taken: bool,
}

impl MousetrapView {
    /// An ordinary trap baited with cheese, exactly like the pre-wave-3 trap
    /// (`MousetrapView::new` keeps every old call site and test honest).
    pub fn new(pos: Vec2, facing: Vec2) -> Self {
        Self::baited(pos, facing, MousetrapKind::Ordinary, TrapBait::Cheese)
    }

    /// A trap with its generated kind and bait.
    pub fn baited(pos: Vec2, facing: Vec2, kind: MousetrapKind, bait: TrapBait) -> Self {
        Self {
            pos,
            facing,
            state: MousetrapState::Armed,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
            kind,
            bait,
            bait_taken: false,
        }
    }

    /// Is this trap still set (or mid-snap) and therefore dangerous?
    #[inline]
    pub fn live(&self) -> bool {
        matches!(
            self.state,
            MousetrapState::Armed | MousetrapState::Triggered | MousetrapState::Snapping
        )
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
/// Renders the grounded drop shadow, folded `36 × 28` cardboard tray with
/// pest-warning corner brackets, fold score notches, and curled corner, the
/// `30 × 22` amber adhesive pool (glossy drifting sheen + trapped debris in
/// `Idle`, disturbed ripples, base glue bridges & a caught struggling bug in
/// `Stuck`, matte dusty torn glue & peeled dog-ear flap in `Used`).
///
/// Wave 3: the board is a **crowd** tool, so this layer also reads its load and
/// its wear from a glance:
///
/// - every currently stuck body gets a glue mound of its own class size and its
///   own colour at its fixed pad slot (a Brute lump is visibly wider than a
///   Skitterer's), under the body sprite the enemy layer draws on top;
/// - `worn` (the share of the lifetime glue budget already spent) fades the
///   wet sheen, breaks the coverage into drier patches, and adds grit;
/// - every place a victim pulled free keeps a ripped scar (`tears`).
pub fn draw_sticky_trap_ground(
    pb: &mut PixelBuffer,
    view: &StickyTrapView,
    cam: Vec2,
    _accent: Rgb,
) {
    let fr = Frame::new(view.pos, view.facing, cam);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 28.0) else {
        return;
    };

    let hx = STICKY_TRAP_SIZE.x * 0.5; // 18.0
    let hy = STICKY_TRAP_SIZE.y * 0.5; // 14.0
    let px = STICKY_TRAP_PAD_HALF.x; // 15.0
    let py = STICKY_TRAP_PAD_HALF.y; // 11.0
    let prog = clamp01(view.progress);
    // Share of the lifetime glue budget already spent (0 fresh, 1 spent).
    let worn = clamp01(view.worn);

    let (tray_base, tray_hi, tray_lo, outline) = match view.state {
        StickyTrapState::Used => ((124, 114, 98), (154, 142, 122), (68, 60, 50), (26, 22, 18)),
        _ => ((196, 184, 154), (236, 226, 198), (112, 100, 80), (30, 26, 20)),
    };

    let (glue_dark, glue_mid, glue_hi) = match view.state {
        StickyTrapState::Idle => ((120, 86, 34), (188, 148, 64), (254, 246, 206)),
        StickyTrapState::Stuck => ((92, 60, 22), (156, 116, 44), (244, 226, 168)),
        StickyTrapState::Used => ((64, 56, 46), (92, 82, 68), (124, 114, 98)),
    };

    // Victim pull direction in local coordinates (used to furrow the glue in Stuck).
    let pull_world = view.victim_pos - view.pos;
    let pull_u = pull_world.dot(fr.f).clamp(-px * 0.75, px * 0.75);
    let pull_v = pull_world.dot(fr.p).clamp(-py * 0.75, py * 0.75);

    // Slow drifting diagonal gloss band across the fresh adhesive.
    let sheen_phase = (view.anim * 1.35).sin() * 9.5;
    let seed = hash2(view.pos.x.round() as i32, view.pos.y.round() as i32);
    let swirl_phase = (seed & 15) as f32 * 0.4;

    // Corner curl geometry: crisp tactile dog-ear on Idle/Stuck (strictly on the
    // outer cardboard lip, never cutting into the active `px × py` glue pad), and
    // a dramatic torn/peeled flap when Used.
    let curl_cut = if view.state == StickyTrapState::Used {
        hx + hy - 8.2
    } else {
        hx + hy - 3.3
    };
    let flap_cut = if view.state == StickyTrapState::Used {
        hx + hy - 12.8
    } else {
        hx + hy - 5.4
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);

            // Peeled / curled-up corner at (+u, +v).
            let peeled_corner = u + v > curl_cut && u > 0.0 && v > 0.0;
            if peeled_corner {
                // Dark floor shadow cast beneath the lifted cardboard dog-ear.
                if sd_box(u, v, hx, hy) <= 0.4 {
                    darken_px(pb, x, y, 0.46);
                }
                continue;
            }

            let d_outer = sd_box(u, v, hx, hy);
            if d_outer > 2.6 {
                continue;
            }

            // Soft contact shadow cast down-right.
            if d_outer > 0.75 {
                let (su, sv) = fr.local_f(x as f32 - 1.6, y as f32 - 1.6);
                let ds = sd_box(su, sv, hx, hy);
                if ds <= 0.4 && !(su + sv > curl_cut && su > 0.0 && sv > 0.0) {
                    darken_px(pb, x, y, 0.42);
                }
                continue;
            }

            // Crisp dark outer silhouette rim.
            if d_outer > -0.45 {
                blend_px(pb, x, y, outline, 0.92);
                continue;
            }

            // Curled-back pale cardboard underside triangle next to the lifted corner.
            let in_flap = if view.state == StickyTrapState::Used {
                u + v > flap_cut && u > hx - 8.0 && v > hy - 8.0
            } else {
                u + v > flap_cut && u > hx - 3.6 && v > hy - 3.6 && sd_box(u, v, px, py) > 0.0
            };
            if in_flap {
                let crease = (u + v - flap_cut).abs() < 0.75;
                let outer_fold = (curl_cut - (u + v)) < 0.75;
                let flap_col = if crease {
                    (58, 50, 40)
                } else if outer_fold {
                    outline
                } else if view.state == StickyTrapState::Used {
                    (168, 156, 134)
                } else {
                    (228, 218, 192)
                };
                pb.plot(x, y, flap_col);
                continue;
            }

            let d_pad = sd_box(u, v, px, py);
            if d_pad > 0.0 {
                // Folded cardboard tray lip (3.0 px wide around the glue pool).
                let edge_lit = if u < -px || v < -py {
                    fr.lit(
                        if u < -px { -1.0 } else { 0.0 },
                        if v < -py { -1.0 } else { 0.0 },
                    )
                } else {
                    fr.lit(
                        if u > px { 1.0 } else { 0.0 },
                        if v > py { 1.0 } else { 0.0 },
                    )
                };
                let paper_grain =
                    hash_f32(u.round() as i32 + (seed as i32 & 15), v.round() as i32);
                let mut col = shade3(
                    tray_lo,
                    tray_base,
                    tray_hi,
                    0.66 + 0.34 * edge_lit + (paper_grain - 0.5) * 0.08,
                );

                // Crisp pest-warning ochre/terracotta corner L-brackets.
                let corner_l = (u.abs() > hx - 5.2 && v.abs() > hy - 2.3)
                    || (u.abs() > hx - 2.3 && v.abs() > hy - 4.6);
                // Industrial fold-score / printed label dashes centered on long edges.
                let fold_dash = u.abs() < 4.2
                    && v.abs() > py + 0.9
                    && v.abs() < hy - 1.1
                    && ((u + 5.0).floor() as i32) % 2 == 0;
                // Side grip-tab ticks on short edges.
                let side_tick = v.abs() < 2.2 && u.abs() > px + 1.0 && u.abs() < hx - 1.0;

                if d_pad < 0.85 {
                    // Crisp inner tray wall crease shadow framing the glue pool.
                    col = scale(col, 0.56);
                } else if corner_l {
                    let warn = if view.state == StickyTrapState::Used {
                        (104, 78, 52)
                    } else {
                        (186, 86, 42)
                    };
                    col = mix(col, warn, 0.78);
                } else if fold_dash || side_tick {
                    col = scale(col, 0.72);
                }

                pb.plot(x, y, col);
                continue;
            }

            // Inside the active `30 × 22` adhesive glue pool!
            let iu = (u * 1.25).round() as i32;
            let iv = (v * 1.25).round() as i32;
            let grain = hash_f32(iu + (seed as i32 & 31), iv + ((seed >> 5) as i32 & 31));
            // Spent glue leaves dry, crusted patches: the wet coverage breaks up
            // as the lifetime budget goes, so "nearly spent" reads at a glance.
            let dry = grain + 0.78 - worn * 1.45 < 0.0;

            // Meniscus edge darkness where thick glue pools against the tray walls.
            let edge_depth = (-d_pad / 2.6).clamp(0.0, 1.0);
            let top_left_ao = if u < -px + 1.7 || v < -py + 1.7 {
                0.76
            } else {
                1.0
            };
            // Subtle viscous amber honey swirl across the larger pool.
            let swirl = ((u * 0.24 + v * 0.17 + swirl_phase).sin()
                * (u * 0.13 - v * 0.22).cos())
                * 0.06;
            let lum_base =
                (0.46 + 0.30 * edge_depth + 0.14 * fr.lit(u / px, v / py) + swirl) * top_left_ao;

            let mut col = match view.state {
                StickyTrapState::Idle => {
                    let mut c = shade3(glue_dark, glue_mid, glue_hi, lum_base);
                    // Wet diagonal specular sheen drifting smoothly across the glossy glue.
                    // The sheen fades with the spent glue budget: a nearly dry board
                    // barely glints.
                    let sheen = 1.0 - 0.88 * worn;
                    let diag = (u * 0.68 + v * 0.88) - sheen_phase;
                    let band1 = (1.0 - (diag.abs() / 2.55)).clamp(0.0, 1.0);
                    let band2 = (1.0 - ((diag + 5.8).abs() / 1.35)).clamp(0.0, 1.0) * 0.48;
                    let spec = (band1 * band1 + band2).clamp(0.0, 1.0) * sheen;
                    c = mix(c, glue_hi, spec * 0.76);
                    if band1 > 0.74 && edge_depth > 0.30 && worn < 0.55 {
                        c = mix(c, (255, 255, 246), 0.84 - worn);
                    }
                    // Syrupy curved meniscus highlight tucked cleanly inside the top-left corner.
                    let near_tl = u < -px + 4.4 && v < -py + 3.8;
                    let on_curve = u > -px + 1.0 && v > -py + 1.0;
                    if near_tl && on_curve && (u + px < 2.1 || v + py < 2.0) && worn < 0.8 {
                        c = mix(c, (255, 252, 232), 0.58 * (1.0 - worn));
                    }
                    // Tiny trapped air bubble specular highlights in the amber glue.
                    if grain > 0.962 && edge_depth > 0.35 && worn < 0.7 {
                        c = mix(c, (255, 250, 224), 0.65);
                    }
                    c
                }
                StickyTrapState::Stuck => {
                    // Disturbed, churned caramel-amber ripples pulling toward victim_pos!
                    let du = u - pull_u * 0.32;
                    let dv = v - pull_v * 0.32;
                    let dist = (du * du + dv * dv).sqrt();
                    let ripple = (dist * 1.45 - prog * 6.5).sin() * 0.5 + 0.5;
                    let mut c =
                        shade3(glue_dark, glue_mid, glue_hi, lum_base * 0.88 + 0.22 * ripple);
                    // Deep stressed paw/foot trough right where the victim churned the adhesive.
                    if dist < 4.2 {
                        c = scale(c, 0.64);
                    }
                    // Wet golden ridge around the churn crater.
                    if (dist - 4.9).abs() < 1.05 {
                        c = mix(c, glue_hi, 0.60);
                    }
                    // Subtle secondary wet highlight streak near the far rim.
                    let diag = (u * 0.68 + v * 0.88) - sheen_phase * 0.5;
                    let gl = (1.0 - (diag.abs() / 1.8)).clamp(0.0, 1.0);
                    c = mix(c, glue_hi, gl * gl * 0.28);
                    c
                }
                StickyTrapState::Used => {
                    // Matte, torn, dusty glue residue — zero bright wet gloss.
                    let mut c = shade3(glue_dark, glue_mid, glue_hi, lum_base * 0.84);
                    // Wide diagonal ripped-up fibrous cardboard scar across the middle.
                    let rip = ((u * 0.46 - v * 0.72).abs() < 2.65) && grain > 0.34;
                    let scuff = ((u * 0.32 + v * 0.55 - 1.5).abs() < 1.4) && grain > 0.52;
                    if rip {
                        c = mix(c, (158, 146, 126), 0.68);
                    } else if scuff {
                        c = mix(c, (136, 124, 106), 0.45);
                    } else if grain < 0.14 {
                        c = scale(c, 0.58);
                    }
                    c
                }
            };

            // Slight wet rim highlight on bottom-right inner wall (bounced light).
            if view.state != StickyTrapState::Used && (u > px - 1.4 || v > py - 1.4) {
                col = mix(col, glue_hi, 0.28 * (1.0 - 0.7 * worn));
            }

            // A dry patch shows the tray underneath, ringed with a crust of old glue.
            if dry && view.state != StickyTrapState::Used {
                let crust = hash_f32(iu + 7, iv - 3) < 0.35;
                col = if crust {
                    mix(col, (134, 108, 74), 0.62)
                } else {
                    mix(tray_base, (176, 162, 138), 0.42)
                };
            }

            pb.plot(x, y, col);
        }
    }

    // Embedded stuck debris on the bigger 30×22 glue canvas (lint fiber, specks,
    // tiny dead gnat) — gives every board grounded floor texture, and it
    // accumulates as the board wears out.
    draw_sticky_debris(pb, &fr, seed, view.state, worn);

    // Torn glue scars: one per victim that pulled free, a ragged hole ringed
    // with dried strands (drawn after the pool so they read over it).
    let tears = (view.tear_n as usize).min(STICKY_TRAP_TEARS);
    for (k, tear) in view.tears.iter().take(tears).enumerate() {
        draw_sticky_tear(pb, &fr, *tear, seed, k as i32);
    }

    // The load: one glue mound per stuck body, sized and tinted by its kind, at
    // its fixed pad slot. The body's own sprite is drawn later by the enemy
    // layer, so this is the glue it is sunk into — the widest thing it must
    // read through is the body itself.
    for slot in 0..STICKY_TRAP_SLOTS {
        if let Some(v) = view.victims[slot] {
            draw_sticky_victim_mound(pb, &fr, &v, view.anim, slot as u32);
        }
    }

    // Dried remains of the board's long-dead first catch, in Used only (the
    // live victims are drawn as mounds above).
    if view.state == StickyTrapState::Used {
        let bug_u = if pull_u < -1.5 { 9.2 } else { -9.2 };
        let bug_v = if pull_v < -1.0 { 5.8 } else { -5.8 };
        draw_caught_bug(pb, &fr, bug_u, bug_v, view.anim, view.state);
    }

    // In Used state, add dried snapped glue stub tags along the ripped adhesive scar.
    if view.state == StickyTrapState::Used {
        for &(su, sv, du, dv) in &[
            (-6.5, -4.2, 3.2, -1.4),
            (1.5, 2.8, 3.0, -1.6),
            (-1.2, 5.4, 2.6, 1.2),
            (6.8, -3.5, -2.4, -1.5),
        ] {
            let a = fr.place(su, sv);
            let b = fr.place(su + du, sv + dv);
            draw_strand_curve(pb, a, b, 0.7, 0.75, (148, 138, 118), (48, 42, 34), 0.80);
        }
    }

    // In Stuck state, draw rich shadowed base glue strings bridging off the board
    // over the dark floor toward **each** struggling victim (routed clear of the
    // caught bug). One bundle per slot, in slot order, so the drawing has no
    // tie to break.
    if view.state == StickyTrapState::Stuck {
        for victim in view.victims.iter().take(STICKY_TRAP_SLOTS).flatten().copied() {
            let vp = sticky_victim_screen(&fr, &victim);
            let vprog = clamp01(victim.progress);
            let fade = (1.0 - (vprog - 0.85).max(0.0) / 0.15).clamp(0.0, 1.0);
            let su = if pull_u < -1.5 { -1.0 } else { 1.0 };
            let sv = if pull_v < -1.0 { -1.0 } else { 1.0 };
            for &(au, av, sag) in &[
                (-9.2 * su, 6.8 * sv, -1.9),
                (-3.2 * su, -6.8 * sv, 1.7),
                (3.2 * su, 6.4 * sv, -1.3),
                (6.0 * su, -4.8 * sv, 1.1),
            ] {
                let anchor = fr.place(au, av);
                draw_strand_curve(
                    pb,
                    anchor,
                    vp,
                    sag * (1.0 - vprog * 0.45),
                    1.45,
                    (238, 220, 152),
                    (58, 38, 14),
                    0.90 * fade,
                );
            }
        }
    }
}

/// One ragged, dried glue scar where a victim tore free of the pad.
///
/// `at` is world space; the scar is a small dark hole with pale torn fibres and
/// a crust ring, so a board that has caught a lot reads its history on the
/// floor even between waves.
fn draw_sticky_tear(pb: &mut PixelBuffer, fr: &Frame, at: Vec2, seed: u32, idx: i32) {
    let d = at - fr.pos;
    let (u, v) = (d.dot(fr.f), d.dot(fr.p));
    let px = STICKY_TRAP_PAD_HALF.x;
    let py = STICKY_TRAP_PAD_HALF.y;
    if u.abs() > px || v.abs() > py {
        return;
    }
    let r = 2.3 + (hash_f32(idx + (seed as i32 & 15), 5) - 0.5) * 0.8;
    let (cx, cy) = fr.px(u, v);
    let reach = (r + 2.0).ceil() as i32;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let fx = dx as f32;
            let fy = dy as f32;
            let dist = (fx * fx + fy * fy).sqrt();
            let ang = fy.atan2(fx);
            let ragged = 0.75 + 0.45 * ((ang * 3.0 + idx as f32).sin() * 0.5 + 0.5);
            let x = cx + dx;
            let y = cy + dy;
            if dist <= r * ragged {
                // The hole: dry cardboard floor underneath, very dark at the rim.
                if dist > r * ragged - 0.9 {
                    blend_px(pb, x, y, (58, 46, 30), 0.85);
                } else {
                    blend_px(pb, x, y, (34, 28, 20), 0.9);
                }
            } else if dist <= r * ragged + 1.2 {
                // Crust ring of dried glue and pale torn fibres.
                let fibre = hash_f32(dx + idx * 7, dy - idx * 3);
                let col = if fibre > 0.55 {
                    (238, 222, 176)
                } else {
                    (156, 126, 74)
                };
                blend_px(pb, x, y, col, 0.72);
            }
        }
    }
}

/// The glue mound one stuck body is sunk into: a class-sized, victim-tinted
/// crater ring with a meniscus and a ring of bubbles, so a Brute reads bigger
/// and a Spitter reads greener than the Skitterer next to it. Drawn before the
/// body sprite, so it reads as the glue around and behind the body.
fn draw_sticky_victim_mound(
    pb: &mut PixelBuffer,
    fr: &Frame,
    victim: &StickyVictimView,
    anim: f32,
    slot: u32,
) {
    let r = victim.kind.mound_radius();
    let d = victim.pos - fr.pos;
    let (u, v) = (d.dot(fr.f), d.dot(fr.p));
    let (cx, cy) = fr.px(u, v);
    let reach = (r + 1.5).ceil() as i32;
    // The victim's own colour, dragged into the amber so it still reads as
    // glue: a green Spitter and a red Brute stay apart at a glance.
    let tint = mix((196, 154, 74), victim.color, 0.55);
    let ring = mix((250, 224, 150), victim.color, 0.30);
    let pulse = 0.5 + 0.5 * (anim * 5.0 + slot as f32 * 1.7).sin();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let fx = dx as f32;
            let fy = dy as f32;
            let dist = (fx * fx + fy * fy).sqrt();
            let x = cx + dx;
            let y = cy + dy;
            if dist <= r * 0.62 {
                // Body-shaped shadow: the body itself is drawn on top of this.
                blend_px(pb, x, y, mix((44, 30, 12), victim.color, 0.22), 0.55);
            } else if dist <= r {
                blend_px(pb, x, y, tint, 0.42 + 0.10 * pulse);
            } else if dist <= r + 1.1 {
                // Wet meniscus lip catching the light around the sunken body.
                blend_px(pb, x, y, ring, 0.62);
            }
        }
    }
    // A couple of bubbles pushed up around the struggling body.
    for k in 0..3 {
        let a = anim * 2.2 + slot as f32 * 2.0 + k as f32 * 2.1;
        let bu = u + a.cos() * (r - 0.4);
        let bv = v + a.sin() * (r - 0.4);
        let (bx, by) = fr.px(bu, bv);
        blend_px(pb, bx, by, (255, 246, 206), 0.55);
    }
}

/// Draw subtle embedded floor debris (stray lint fiber, tiny crumbs, micro gnat
/// speck) trapped inside the `30 × 22` glue pool, strictly within `STICKY_TRAP_PAD_HALF`.
///
/// `worn` (`0` fresh, `1` spent) adds grit: a tired board is dirtier than a
/// fresh one, and the Used decal keeps the full litter.
fn draw_sticky_debris(
    pb: &mut PixelBuffer,
    fr: &Frame,
    seed: u32,
    state: StickyTrapState,
    worn: f32,
) {
    let used = state == StickyTrapState::Used;
    let crumb_dark: Rgb = if used { (46, 40, 34) } else { (68, 44, 18) };
    let crumb_mid: Rgb = if used { (76, 68, 56) } else { (104, 72, 30) };
    let lint_col: Rgb = if used { (148, 138, 120) } else { (236, 214, 158) };
    let glint_col: Rgb = (255, 244, 198);

    // Slight deterministic per-board mirror/offset so boards don't look stamped identically.
    let su = if seed & 1 == 0 { 1.0 } else { -1.0 };
    let sv = if seed & 2 == 0 { 1.0 } else { -1.0 };

    // 1. Curled pale lint hair embedded in the glue.
    let (lu, lv) = (su * 7.8, sv * 5.2);
    for &(du, dv) in &[(0.0, 0.0), (1.0, 0.6), (2.0, 0.3), (2.7, -0.5)] {
        let (x, y) = fr.px(lu + du * su * 0.85, lv + dv * sv * 0.85);
        blend_px(pb, x, y, lint_col, if used { 0.55 } else { 0.68 });
    }

    // 2. Submerged dark crumbs / grit specks with a tiny amber meniscus highlight.
    for &(cu, cv) in &[
        (-10.2 * su, 5.8 * sv),
        (10.6 * su, -4.8 * sv),
        (-3.4 * su, -7.6 * sv),
    ] {
        let (x, y) = fr.px(cu, cv);
        blend_px(pb, x, y, crumb_dark, 0.78);
        let (x2, y2) = fr.px(cu + 0.9, cv + 0.5);
        blend_px(pb, x2, y2, crumb_mid, 0.62);
        if !used {
            let (gx, gy) = fr.px(cu - 0.8, cv - 0.8);
            blend_px(pb, gx, gy, glint_col, 0.52);
        }
    }

    // 3. Tiny stray gnat speck tucked near the outer edge of the pad.
    let (gu, gv) = (9.6 * su, 6.8 * sv);
    let (gx, gy) = fr.px(gu, gv);
    blend_px(pb, gx, gy, crumb_dark, 0.82);
    let (wx, wy) = fr.px(gu + 0.9, gv - 0.7);
    blend_px(pb, wx, wy, lint_col, if used { 0.35 } else { 0.58 });

    // 4. Wear grit: spent glue collects dust, hairs and crumbs. One extra speck
    // per wear step, at fixed positions, so a tired board looks dirty and a
    // fresh one stays clean (no rng, no per-frame work when fresh).
    if !used && worn > 0.05 {
        let grit = ((worn * 7.0).ceil() as i32).min(7);
        for k in 0..grit {
            let a = k as f32 * 2.399 + (seed & 31) as f32 * 0.1;
            let rad = 3.0 + (k % 3) as f32 * 3.4;
            let (dx, dy) = fr.px(a.cos() * rad * su, a.sin() * rad * sv);
            let col = if k % 2 == 0 { crumb_dark } else { lint_col };
            blend_px(pb, dx, dy, col, 0.55);
        }
    }
}

/// Draw a small trapped beetle/roach stuck in the glue board (glazed & twitching
/// in `Stuck`, dusty & motionless in `Used`).
fn draw_caught_bug(
    pb: &mut PixelBuffer,
    fr: &Frame,
    bu: f32,
    bv: f32,
    anim: f32,
    state: StickyTrapState,
) {
    let used = state == StickyTrapState::Used;
    // Slight diagonal orientation relative to the board so it looks organically trapped.
    let angle = 0.42f32;
    let (ca, sa) = (angle.cos(), angle.sin());
    let to_board = |lx: f32, ly: f32| -> (f32, f32) { (bu + lx * ca - ly * sa, bv + lx * sa + ly * ca) };

    // Wet golden glue meniscus ringing the trapped bug in Stuck.
    if !used {
        for &(du, dv) in &[
            (-3.2, -1.6),
            (-3.2, 1.6),
            (3.2, -1.5),
            (3.2, 1.5),
            (0.0, -2.8),
            (0.0, 2.8),
        ] {
            let (u, v) = to_board(du, dv);
            let (x, y) = fr.px(u, v);
            blend_px(pb, x, y, (246, 214, 120), 0.52);
        }
    }

    // 6 tiny legs (3 per side), subtly twitching in Stuck.
    let leg_col: Rgb = if used { (42, 36, 30) } else { (44, 24, 14) };
    let twitch = if used { 0.0 } else { (anim * 18.0).sin() * 0.45 };
    for side in [-1.0f32, 1.0f32] {
        for (i, &lx_base) in [-1.6f32, 0.0, 1.5].iter().enumerate() {
            let phase = if i % 2 == 0 { twitch } else { -twitch };
            let (u1, v1) = to_board(lx_base, side * 1.9);
            let (u2, v2) = to_board(lx_base + phase * 1.1, side * 3.1);
            let (x1, y1) = fr.px(u1, v1);
            let (x2, y2) = fr.px(u2, v2);
            blend_px(pb, x1, y1, leg_col, 0.88);
            blend_px(pb, x2, y2, leg_col, 0.74);
        }
    }

    // Oval dark mahogany carapace + head.
    let shell_rim: Rgb = if used { (34, 30, 26) } else { (32, 18, 10) };
    let shell_mid: Rgb = if used { (66, 58, 48) } else { (84, 42, 22) };
    let shell_hi: Rgb = if used { (94, 86, 74) } else { (142, 82, 38) };

    for ly_i in -2..=2 {
        for lx_i in -3..=3 {
            let lx = lx_i as f32;
            let ly = ly_i as f32;
            let ell = (lx / 2.7) * (lx / 2.7) + (ly / 1.85) * (ly / 1.85);
            if ell > 1.0 {
                continue;
            }
            let (u, v) = to_board(lx, ly);
            let (x, y) = fr.px(u, v);
            let col = if ell > 0.62 {
                shell_rim
            } else if lx < 0.0 && ly < 0.2 {
                shell_hi
            } else {
                shell_mid
            };
            blend_px(pb, x, y, col, if used { 0.85 } else { 0.96 });
        }
    }

    // Tiny head + 2 feelers at +lx end.
    let (hu, hv) = to_board(2.9, 0.0);
    let (hx_p, hy_p) = fr.px(hu, hv);
    blend_px(pb, hx_p, hy_p, shell_rim, 0.94);
    for side in [-0.9f32, 0.9f32] {
        let (au, av) = to_board(3.9, side + twitch * 0.5);
        let (ax, ay) = fr.px(au, av);
        blend_px(pb, ax, ay, leg_col, 0.72);
    }

    // Glossy amber-white glue glaze highlight on the bug's back when Stuck.
    if !used {
        let (su, sv) = to_board(-0.6, -0.5);
        let (sx, sy) = fr.px(su, sv);
        blend_px(pb, sx, sy, (255, 248, 214), 0.90);
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
        let stretch = 7.0 + prog * 13.0;
        let wobble = (view.anim * 16.0).sin() * 1.8;
        fr.place(stretch, wobble)
    }
}

/// Screen position of one slot's victim (its pad slot, as the sim wrote it:
/// world pixels are buffer pixels).
#[inline]
fn sticky_victim_screen(_fr: &Frame, victim: &StickyVictimView) -> Vec2 {
    victim.pos
}

/// **Foreground / overlay layer** for a sticky trap (draw AFTER entities).
///
/// In `StickyTrapState::Stuck`, draws high-contrast glistening amber-ivory
/// taffy strands stretching from board anchors across the `30 × 22` glue pool
/// onto the flanks of **every** stuck victim, complete with dark amber shadow
/// rims and glossy stretched droplets that thin and snap as each victim's own
/// hold runs out (`progress` per slot).
///
/// A crowd therefore reads as a bundle of strands per body: the anchored bodies
/// visible on the pad, each with its own pull. No-op in `Idle` and `Used` (fast
/// early-out).
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
    if fr.bbox_pb(pb, 48.0).is_none() {
        return;
    }

    let strand_hi: Rgb = (255, 252, 224);
    let strand_mid: Rgb = (242, 214, 128);
    let strand_rim: Rgb = (52, 34, 12);

    for slot in 0..STICKY_TRAP_SLOTS {
        let Some(victim) = view.victims[slot] else {
            continue;
        };
        let prog = clamp01(victim.progress);
        let vp = sticky_victim_screen(&fr, &victim);
        let vr = victim.radius.clamp(2.0, 9.0);
        let to_v = vp - fr.pos;
        let dir = safe_dir(to_v);
        let side = dir.perp();

        // Three taut strands per body, from fixed anchors (deterministic from
        // the slot index) across the padding to the victim's trailing flanks so
        // its own sprite stays readable. The strand set rotates with the slot
        // so neighbouring bodies do not draw identical bundles.
        let base = slot as f32 * 2.3;
        let anchors: [(f32, f32, f32, f32); 3] = [
            (
                (base + 0.3).cos() * (STICKY_TRAP_PAD_HALF.x - 1.4),
                (base + 0.3).sin() * (STICKY_TRAP_PAD_HALF.y - 1.2),
                0.82,
                0.72,
            ),
            (
                (base + 2.4).cos() * (STICKY_TRAP_PAD_HALF.x - 1.8),
                (base + 2.4).sin() * (STICKY_TRAP_PAD_HALF.y - 1.5),
                -0.74,
                0.84,
            ),
            (
                (base + 4.4).cos() * (STICKY_TRAP_PAD_HALF.x - 2.4),
                (base + 4.4).sin() * (STICKY_TRAP_PAD_HALF.y - 1.9),
                0.18,
                0.94,
            ),
        ];

        for (idx, &(au, av, flank_k, snap_at)) in anchors.iter().enumerate() {
            let a = fr.place(au, av);
            let quiver = (view.anim * (19.0 + idx as f32 * 5.0) + slot as f32 * 1.1)
                .sin()
                * (0.65 + 1.05 * prog);
            // Anchor onto the victim's trailing haunches/flanks so its face stays readable!
            let target = vp + side * (flank_k * vr * 0.72) - dir * (vr * 0.52);

            if prog < snap_at {
                let tension = (prog / snap_at).clamp(0.0, 1.0);
                let width = (1.60 - 0.72 * tension).max(0.68);
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
}

// ============================================================================
// 2. Mousetrap drawing
// ============================================================================

/// Art unit scale of the mousetrap: the drawing below is authored in the
/// original `18 × 11` board's units and mapped through this pair, so the bigger
/// trap is the same trap redrawn at the new resolution (proportions kept) rather
/// than a stretched bitmap.
#[inline]
fn mousetrap_art_scale() -> Vec2 {
    Vec2::new(MOUSETRAP_SIZE.x / 18.0, MOUSETRAP_SIZE.y / 11.0)
}

#[inline]
fn mousetrap_jitter(view: &MousetrapView) -> Vec2 {
    let prog = clamp01(view.progress);
    let k = mousetrap_art_scale().x;
    match view.state {
        MousetrapState::Triggered => {
            let amp = (0.25 + 0.65 * prog) * k;
            let s = (view.anim * 68.0 + prog * 42.0).sin();
            safe_dir(view.facing).perp() * (s * amp)
        }
        MousetrapState::Snapping if (0.42..=0.72).contains(&prog) => {
            let decay = 1.0 - (prog - 0.42) / 0.30;
            safe_dir(view.facing) * (-0.75 * decay * k)
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
    let art = mousetrap_art_scale();
    let (kx, ky) = (art.x, art.y);
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 34.0) else {
        return;
    };

    let hx = 9.0; // art units: the old board's half extents
    let hy = 5.5;

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
            let (wu, wv) = fr.local(x, y);
            let u = wu / kx;
            let v = wv / ky;
            let d = sd_box(u, v, hx, hy);
            if d > 2.4 {
                continue;
            }
            if d > 0.75 {
                let (su, sv) = fr.local_f(x as f32 - 1.5, y as f32 - 1.6);
                if sd_box(su / kx, sv / ky, hx, hy) <= 0.45 {
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

    // A hair-trigger board is painted: a dull oxblood wash over the strike half
    // and a red rim stripe, so the instant kind reads from across the room.
    if view.kind.hair() {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (wu, wv) = fr.local(x, y);
                let u = wu / kx;
                let v = wv / ky;
                let d = sd_box(u, v, hx, hy);
                if d > -0.25 {
                    continue;
                }
                let rim = d > -1.0 || (u > hx - 1.1 || v.abs() > hy - 1.1);
                if rim {
                    blend_px(pb, x, y, (168, 44, 34), 0.72);
                } else if u > 0.0 {
                    blend_px(pb, x, y, (142, 38, 32), 0.20);
                }
            }
        }
    }

    // 2. Triggered danger telegraph bracket drawn OVER the board & extending
    //    outside the +u kill zone so the warning reads immediately!
    if view.state == MousetrapState::Triggered {
        let pulse = 0.62 + 0.38 * (prog * 18.0 + view.anim * 24.0).sin().abs();
        let warn_col: Rgb = mix((245, 58, 42), (255, 198, 72), prog * 0.45);
        let ku0 = MOUSETRAP_KILL_U_MIN / kx;
        let ku1 = MOUSETRAP_KILL_U_MAX / kx + 1.2;
        let kv = MOUSETRAP_KILL_HALF_V / ky + 1.2;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (wu, wv) = fr.local(x, y);
                let u = wu / kx;
                let v = wv / ky;
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
        let impact_c = fr.place(4.8 * kx, 0.0);
        let ring_r = (3.5 + imp * 10.5) * kx;
        let fade = (1.0 - imp) * (1.0 - imp);
        glow_disc(pb, impact_c.x, impact_c.y, 8.5 * kx, (255, 196, 98), 0.70 * fade);
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
/// Renders the central steel torsion spring coil, copper trigger pedal, the
/// bait on the plate (a golden cheese wedge, a floor pickup, or a small chest),
/// the holding catch, and the gleaming spring-steel striker U-bar (cocked at
/// `-u`, quivering in `Triggered`, whipping 180° overhead with motion streaks in
/// `Snapping`, or slammed shut across `+u` in `Sprung`).
///
/// A **hair-trigger** trap draws a thin, quivering filed wire catch and a
/// red-painted bar: the kind is legible from a distance before the rat commits.
pub fn draw_mousetrap(
    pb: &mut PixelBuffer,
    view: &MousetrapView,
    cam: Vec2,
    _accent: Rgb,
) {
    let prog = clamp01(view.progress);
    let fr = Frame::new(view.pos + mousetrap_jitter(view), view.facing, cam);
    let art = mousetrap_art_scale();
    let (kx, ky) = (art.x, art.y);
    let k = (kx + ky) * 0.5;
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 34.0) else {
        return;
    };
    let hair = view.kind.hair();

    // 1. Copper bait pedal at +u (u = 2.6..7.2, |v| <= 2.4 in art units)
    let pedal_dip = matches!(
        view.state,
        MousetrapState::Triggered | MousetrapState::Snapping | MousetrapState::Sprung
    );
    let cop_shadow: Rgb = (92, 46, 22);
    let cop_mid: Rgb = (176, 102, 52);
    let cop_hi: Rgb = (238, 174, 112);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (wu, wv) = fr.local(x, y);
            let u = wu / kx;
            let v = wv / ky;
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

    // 2. The bait on the plate. Cheese keeps the old wedge (and its smashed
    //    crumbs); a pickup or a chest is the reward the trap is guarding, and it
    //    sits there until the rat takes it — sprung trap or not.
    let bait_on_plate = !view.bait_taken;
    let show_cheese = matches!(view.bait, TrapBait::Cheese);
    if show_cheese {
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
                        let (wu, wv) = fr.local(x, y);
                        let du = wu / kx - 4.9;
                        let dv = wv / ky + 0.1;
                        let cu = du * cos_c - dv * sin_c;
                        let cv = du * sin_c + dv * cos_c;
                        let half_w = (2.3 - cu) * 0.56 + 0.55;
                        let in_wedge = (-2.0..=2.3).contains(&cu) && cv.abs() <= half_w;
                        let in_rim = (-2.7..=2.9).contains(&cu) && cv.abs() <= half_w + 0.75;
                        if in_wedge {
                            let lit = 0.74 + 0.32 * fr.lit(du * 0.4, dv * 0.4);
                            let mut c = shade3(cheese_lo, cheese_base, cheese_hi, lit);
                            // Two classic round Swiss cheese holes
                            if (cu + 0.5).hypot(cv - 0.65) < 0.72
                                || (cu - 0.65).hypot(cv + 0.45) < 0.58
                            {
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
                    let p = fr.px((5.0 + du * spread) * kx, dv * spread * ky);
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
                    let p = fr.px(cu * kx, cv * ky);
                    let c = if bright {
                        (214, 174, 62)
                    } else {
                        (164, 128, 44)
                    };
                    blend_px(pb, p.0, p.1, c, 0.88);
                }
            }
        }
    } else if bait_on_plate {
        // A pickup or a chest sitting on the plate: the same sprite the floor
        // would draw, so the reward is unmistakable from a distance.
        let plate = fr.px(4.9 * kx, 0.0);
        match view.bait {
            TrapBait::Chest => {
                // A chest is drawn small enough to fit the plate; the glint ring
                // makes it read as "there is a chest on that trap" at range.
                glow_disc(
                    pb,
                    plate.0 as f32,
                    plate.1 as f32,
                    9.0,
                    (255, 208, 96),
                    0.30 + 0.18 * (view.anim * 2.4).sin(),
                );
                crate::game::art_floor::draw_chest(pb, plate, view.anim);
            }
            TrapBait::Pickup(kind) => {
                crate::game::art_floor::draw_pickup(pb, kind, plate, view.anim);
            }
            TrapBait::Cheese => {}
        }
    }

    // 3. Central steel torsion coil spring along u = -0.3, |v| <= 3.6
    let spring_a = fr.place(-0.3 * kx, -3.4 * ky);
    let spring_b = fr.place(-0.3 * kx, 3.4 * ky);
    draw_metal_bar(
        pb,
        spring_a,
        spring_b,
        0.95 * k,
        Vec2::new(0.8, 0.9) * k,
        (134, 146, 158),
        (216, 228, 240),
        (24, 28, 34),
    );
    // Dark coil wrap grooves across the spring
    for c in -2..=2 {
        let p = fr.px(-0.3 * kx, c as f32 * 1.25 * ky);
        darken_px(pb, p.0, p.1, 0.45);
    }

    // 4. The catch that holds the bar back. The ordinary trap has a thick steel
    //    lock rod; the hair-trigger has a hair-thin filed wire that visibly
    //    quivers — and it is quiet until it is not.
    match view.state {
        MousetrapState::Armed | MousetrapState::Triggered => {
            if hair {
                let quiver = (view.anim * (52.0 + prog * 60.0)).sin() * 0.55;
                let a = fr.place(-7.0 * kx, -2.6 * ky);
                let b = fr.place(2.6 * kx, quiver * ky);
                draw_metal_bar(
                    pb,
                    a,
                    b,
                    0.32 * k,
                    Vec2::new(0.35, 0.4) * k,
                    (196, 168, 154),
                    (255, 240, 226),
                    (68, 24, 18),
                );
                // Filed notch: the wire sits in a bright nick on the catch post.
                let notch = fr.px(-7.2 * kx, 0.0);
                blend_px(pb, notch.0, notch.1, (255, 246, 210), 0.95);
                // A faint red tension bead travels along the wire as the rat nears.
                let bead = fr.place((-7.0 + 9.6 * ((view.anim * 0.7) % 1.0)) * kx, quiver * ky);
                blend_px(pb, bead.x.round() as i32, bead.y.round() as i32, (232, 64, 48), 0.55);
            } else {
                let rod_a = fr.place(-7.6 * kx, 0.0);
                let rod_b = fr.place(3.0 * kx, 0.0);
                draw_metal_bar(
                    pb,
                    rod_a,
                    rod_b,
                    0.45 * k,
                    Vec2::new(0.5, 0.6) * k,
                    (168, 178, 188),
                    (236, 244, 250),
                    (32, 36, 42),
                );
            }
        }
        _ => {}
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
    let lift_ofs = Vec2::new(0.0, -lift_height) * k;
    let sh_ofs = Vec2::new(0.9 + lift_height * 0.45, 1.0 + lift_height * 0.55) * k;

    let hinge_l = fr.place(-0.2 * kx, -bar_half_v * ky);
    let hinge_r = fr.place(-0.2 * kx, bar_half_v * ky);
    let tip_l = fr.place(bar_reach * kx, -bar_half_v * ky) + lift_ofs;
    let tip_r = fr.place(bar_reach * kx, bar_half_v * ky) + lift_ofs;

    // A hair-trigger bar is red-painted: the silhouette answer to "is this one
    // going to bite before I can move?".
    let steel_base: Rgb = if hair { (176, 52, 40) } else { (170, 186, 204) };
    let steel_glint: Rgb = if hair { (255, 148, 120) } else { (236, 246, 255) };
    let steel_mid = mix(
        steel_base,
        if hair { (255, 190, 150) } else { (255, 246, 224) },
        bar_flash.max(clamp01(view.hit_flash)),
    );
    let steel_hi = mix(steel_glint, (255, 255, 255), bar_flash);
    let steel_out: Rgb = (22, 26, 34);

    // Motion-blur smear arc behind the whipping striker bar during Snapping!
    if view.state == MousetrapState::Snapping && prog < 0.52 {
        for ghost in 1..=3 {
            let g_ang = (theta - ghost as f32 * 0.34).max(0.0);
            let g_u = -7.4 * g_ang.cos();
            let g_lift = Vec2::new(0.0, -g_ang.sin() * 4.6) * k;
            let ga = fr.place(g_u * kx, -bar_half_v * ky) + g_lift;
            let gb = fr.place(g_u * kx, bar_half_v * ky) + g_lift;
            let alpha = (0.55 - ghost as f32 * 0.14).max(0.1);
            draw_strand_curve(pb, ga, gb, 0.0, 1.0 * k, (240, 248, 255), (120, 140, 165), alpha);
        }
    }

    draw_metal_bar(pb, hinge_l, tip_l, 0.65 * k, sh_ofs, steel_mid, steel_hi, steel_out);
    draw_metal_bar(pb, hinge_r, tip_r, 0.65 * k, sh_ofs, steel_mid, steel_hi, steel_out);
    draw_metal_bar(pb, tip_l, tip_r, 0.85 * k, sh_ofs, steel_mid, steel_hi, steel_out);

    // Bright tension sparks at the latch in Triggered; crisp radial impact lines in Snapping!
    if view.state == MousetrapState::Triggered {
        let spark_p = fr.place(-0.3 * kx, (view.anim * 40.0).sin() * 2.2 * ky);
        glow_disc(
            pb,
            spark_p.x,
            spark_p.y,
            4.0 * k,
            if hair { (255, 96, 72) } else { (255, 186, 68) },
            0.88 * (0.4 + 0.6 * prog),
        );
        // Tiny amber tension sparks zipping off the cocked spring!
        for c in 0..3 {
            let s = fr.px(-0.8 * kx - c as f32 * 1.4 * kx, (c as f32 - 1.0) * 3.8 * ky);
            add_px(pb, s.0, s.1, (255, 220, 112), 0.85 * prog);
        }
    } else if view.state == MousetrapState::Snapping && (0.42..=0.80).contains(&prog) {
        let imp = 1.0 - (prog - 0.42) / 0.38;
        let mid_bar = (tip_l + tip_r) * 0.5;
        glow_disc(pb, mid_bar.x, mid_bar.y, 5.5 * k, (255, 236, 178), 0.90 * imp);
        // 4 crisp radial impact burst streaks!
        for &(du, dv) in &[(1.0, -0.7), (1.0, 0.7), (0.3, -1.0), (0.3, 1.0)] {
            let a = fr.place((6.5 + du * 1.6) * kx, dv * 2.6 * ky);
            let b = fr.place(
                (6.5 + du * (4.2 + (1.0 - imp) * 3.5)) * kx,
                dv * (5.2 + (1.0 - imp) * 2.5) * ky,
            );
            draw_strand_curve(
                pb,
                a,
                b,
                0.0,
                0.8 * k,
                (255, 248, 212),
                (255, 168, 64),
                0.90 * imp,
            );
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // Ordinary bar, cheese bait: the pre-wave-3 trap.
    ..MousetrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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

    /// Verify that the drawn sticky board's outer cardboard tray and active
    /// amber glue pool agree with `STICKY_TRAP_SIZE` and `STICKY_TRAP_PAD_HALF`
    /// in all four cardinal facings.
    #[test]
    fn sticky_pad_matches_drawn_glue() {
        let bg: Rgb = (10, 12, 18);
        let accent: Rgb = (120, 220, 255);
        let hx = STICKY_TRAP_SIZE.x * 0.5;
        let hy = STICKY_TRAP_SIZE.y * 0.5;
        let px = STICKY_TRAP_PAD_HALF.x;
        let py = STICKY_TRAP_PAD_HALF.y;
        assert!(px > 0.0 && px < hx);
        assert!(py > 0.0 && py < hy);

        let center = Vec2::new(60.0, 60.0);
        for facing in [
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, -1.0),
        ] {
            let mut pb = PixelBuffer::new(120, 120);
            pb.clear(bg);
            let view = StickyTrapView {
                pos: center,
                facing,
                state: StickyTrapState::Idle,
                progress: 0.0,
                anim: 0.0,
                victim_pos: center,
                victim_radius: 3.0,
                // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
            };
            draw_sticky_trap_ground(&mut pb, &view, Vec2::ZERO, accent);

            let f = facing.normalized();
            let p = f.perp();
            let sample = |u: f32, v: f32| -> Rgb {
                let q = center + f * u + p * v;
                pb.pixel_at(q.x.round() as usize, q.y.round() as usize)
            };

            // Inside the active glue pad near all 4 edges: warm amber glue (R > G > B, strong R-B).
            for &(u, v) in &[
                (px - 1.5, 0.0),
                (-(px - 1.5), 0.0),
                (0.0, py - 1.5),
                (0.0, -(py - 1.5)),
            ] {
                let c = sample(u, v);
                assert_ne!(c, bg, "facing {facing:?} ({u},{v}) should draw glue, got bg");
                assert!(
                    c.0 > c.2 + 40,
                    "facing {facing:?} ({u},{v}) should be warm amber glue, got {c:?}"
                );
            }

            // Cardboard tray rim just outside the glue pad: drawn, paler/less saturated than glue.
            for &(u, v) in &[
                (px + 1.5, 0.0),
                (-(px + 1.5), 0.0),
                (0.0, py + 1.5),
                (0.0, -(py + 1.5)),
            ] {
                let c = sample(u, v);
                assert_ne!(c, bg, "facing {facing:?} ({u},{v}) should draw tray rim, got bg");
                assert!(
                    c.2 >= 60,
                    "facing {facing:?} ({u},{v}) should be ivory cardboard rim, got {c:?}"
                );
            }

            // Strictly outside the board + shadow: untouched background.
            for &(u, v) in &[
                (hx + 4.0, 0.0),
                (-(hx + 4.0), 0.0),
                (0.0, hy + 4.0),
                (0.0, -(hy + 4.0)),
            ] {
                assert_eq!(
                    sample(u, v),
                    bg,
                    "facing {facing:?} ({u},{v}) should be clear background"
                );
            }
        }
    }

    /// Dedicated sticky-trap visual sheet: `RATRUN_DUMP=sticky cargo test dump_sticky_sheet`
    /// writes `/tmp/ratrun_sticky.ppm` showing all 4 cardinal facings across
    /// Idle, Stuck (with Rat, with Brute, and solo), and Used states on real
    /// themed floors, plus scale reference.
    #[test]
    fn dump_sticky_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("sticky") {
            return;
        }
        const CELL_W: usize = 72;
        const CELL_H: usize = 64;
        const COLS: usize = 5;
        const ROWS: usize = 4;
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

        let facings = [
            Vec2::new(1.0, 0.0),  // East
            Vec2::new(-1.0, 0.0), // West
            Vec2::new(0.0, 1.0),  // South
            Vec2::new(0.0, -1.0), // North
        ];

        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let level = Level::new(theme);
            let accent = level.palette.accent;
            let cam = Vec2::new(260.0, 260.0);
            let mut pb = PixelBuffer::new(w, h);
            draw_floor(&mut pb, &level, cam, 1.2);

            let cell_center = |row: usize, col: usize| -> Vec2 {
                cam + Vec2::new(
                    (col * CELL_W + CELL_W / 2) as f32,
                    (row * CELL_H + CELL_H / 2) as f32,
                )
            };

            for (row, &facing) in facings.iter().enumerate() {
                // Col 0: Idle (fresh)
                let c0 = cell_center(row, 0);
                let v0 = StickyTrapView {
                    pos: c0,
                    facing,
                    state: StickyTrapState::Idle,
                    progress: 0.0,
                    anim: 0.35 + row as f32 * 0.4,
                    victim_pos: c0,
                    victim_radius: 3.0,
                    // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
                };
                draw_sticky_trap_ground(&mut pb, &v0, cam, accent);
                draw_sticky_trap(&mut pb, &v0, cam, accent);

                // Col 1: Stuck (occupied, caught bug + rat pulling)
                let c1 = cell_center(row, 1);
                let rat_pos = c1 + facing * 8.5 + facing.perp() * 2.5;
                let v1 = StickyTrapView {
                    pos: c1,
                    facing,
                    state: StickyTrapState::Stuck,
                    progress: 0.42,
                    anim: 1.4,
                    victim_pos: rat_pos,
                    victim_radius: 3.0,
                    // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
                };
                draw_sticky_trap_ground(&mut pb, &v1, cam, accent);
                let p = make_rat(rat_pos, facing);
                draw_rat(&mut pb, &p, 0.7, cam);
                draw_sticky_trap(&mut pb, &v1, cam, accent);

                // Col 2: Stuck (rat stretching hard near edge)
                let c2 = cell_center(row, 2);
                let rat_pos2 = c2 + facing * 17.5 - facing.perp() * 3.0;
                let v2 = StickyTrapView {
                    pos: c2,
                    facing,
                    state: StickyTrapState::Stuck,
                    progress: 0.76,
                    anim: 2.5,
                    victim_pos: rat_pos2,
                    victim_radius: 3.0,
                    // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
                };
                draw_sticky_trap_ground(&mut pb, &v2, cam, accent);
                let p2 = make_rat(rat_pos2, facing);
                draw_rat(&mut pb, &p2, 1.1, cam);
                draw_sticky_trap(&mut pb, &v2, cam, accent);

                // Col 3: Stuck (Brute pinned)
                let c3 = cell_center(row, 3);
                let brute_pos = c3 + facing * 13.0 + facing.perp() * 2.0;
                let v3 = StickyTrapView {
                    pos: c3,
                    facing,
                    state: StickyTrapState::Stuck,
                    progress: 0.55,
                    anim: 1.9,
                    victim_pos: brute_pos,
                    victim_radius: 6.5,
                    // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
                };
                draw_sticky_trap_ground(&mut pb, &v3, cam, accent);
                let b = make_brute(brute_pos, facing);
                draw_enemy(&mut pb, &b, cam, accent);
                draw_sticky_trap(&mut pb, &v3, cam, accent);

                // Col 4: Used (spent)
                let c4 = cell_center(row, 4);
                let v4 = StickyTrapView {
                    pos: c4,
                    facing,
                    state: StickyTrapState::Used,
                    progress: 1.0,
                    anim: 0.0,
                    victim_pos: c4,
                    victim_radius: 3.0,
                    // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
                };
                draw_sticky_trap_ground(&mut pb, &v4, cam, accent);
                draw_sticky_trap(&mut pb, &v4, cam, accent);
            }

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_sticky.ppm", out).unwrap();
    }

    /// In-level visual capture: `RATRUN_DUMP=sticky_level cargo test dump_sticky_in_level`
    /// writes `/tmp/ratrun_sticky_level.ppm` showing real generated `36 × 28`
    /// sticky traps in a real `World::draw` frame (walls, props, rat, enemies),
    /// covering every state (`Idle`, `Stuck` with rat/Brute, `Used`) and all four
    /// cardinal facings across Sewer, Kitchen, and Lab.
    #[test]
    fn dump_sticky_in_level() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("sticky_level") {
            return;
        }
        use crate::game::world::World;

        const VW: usize = 160;
        const VH: usize = 120;
        const COLS: usize = 4;
        const ROWS: usize = 3;
        let total_w = VW * COLS;
        let total_h = VH * ROWS;
        let mut sheet = vec![0u8; total_w * total_h * 3];

        let blit = |sheet: &mut [u8], pb: &PixelBuffer, row: usize, col: usize| {
            for y in 0..VH {
                for x in 0..VW {
                    let c = pb.pixel_at(x, y);
                    let gx = col * VW + x;
                    let gy = row * VH + y;
                    let idx = (gy * total_w + gx) * 3;
                    sheet[idx] = c.0;
                    sheet[idx + 1] = c.1;
                    sheet[idx + 2] = c.2;
                }
            }
        };

        // Row 0..3 = Sewer, Kitchen, Lab.
        // Col 0 = real generated Idle board (East/natural facing) with rat beside it.
        // Col 1 = real generated board forced West facing, Stuck with rat pulling.
        // Col 2 = real generated board forced South facing, Stuck with Brute pinned.
        // Col 3 = real generated board forced North facing, Used (spent).
        let themes = [Theme::Sewer, Theme::Kitchen, Theme::Lab];
        for (row, &theme) in themes.iter().enumerate() {
            let mut w = World::new(theme, 7 + row as u64);
            let trap_idx = 0usize;
            assert!(
                !w.level.objects.sticky_traps.is_empty(),
                "{theme:?} must have sticky traps"
            );
            let trap_pos = w.level.objects.sticky_traps[trap_idx].pos;
            w.elapsed = 1.4;

            // Col 0: Idle (East)
            w.level.objects.sticky_traps[trap_idx] = StickyTrapView {
                pos: trap_pos,
                facing: Vec2::new(1.0, 0.0),
                state: StickyTrapState::Idle,
                progress: 0.0,
                anim: 0.4,
                victim_pos: trap_pos,
                victim_radius: 3.0,
                // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
            };
            w.player.pos = trap_pos + Vec2::new(-28.0, 4.0);
            w.player.face = Vec2::new(1.0, 0.0);
            let mut pb = PixelBuffer::new(VW, VH);
            w.draw(&mut pb);
            blit(&mut sheet, &pb, row, 0);

            // Col 1: Stuck with Rat (West)
            let rat_stuck_pos = trap_pos + Vec2::new(-11.0, 2.5);
            w.level.objects.sticky_traps[trap_idx] = StickyTrapView {
                pos: trap_pos,
                facing: Vec2::new(-1.0, 0.0),
                state: StickyTrapState::Stuck,
                progress: 0.55,
                anim: 1.8,
                victim_pos: rat_stuck_pos,
                victim_radius: 3.0,
                // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
            };
            w.player.pos = rat_stuck_pos;
            w.player.face = Vec2::new(-1.0, 0.0);
            let mut pb = PixelBuffer::new(VW, VH);
            w.draw(&mut pb);
            blit(&mut sheet, &pb, row, 1);

            // Col 2: Stuck with Brute (South)
            let brute_pos = trap_pos + Vec2::new(2.0, 12.0);
            let mut rng = StdRng::seed_from_u64(42);
            w.spawn_enemy_with_rng(EnemyKind::Brute, brute_pos, 1.0, &mut rng);
            w.level.objects.sticky_traps[trap_idx] = StickyTrapView {
                pos: trap_pos,
                facing: Vec2::new(0.0, 1.0),
                state: StickyTrapState::Stuck,
                progress: 0.48,
                anim: 2.1,
                victim_pos: brute_pos,
                victim_radius: 6.5,
                // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
            };
            w.player.pos = trap_pos + Vec2::new(-26.0, -6.0);
            w.player.face = Vec2::new(1.0, 0.2);
            let mut pb = PixelBuffer::new(VW, VH);
            w.draw(&mut pb);
            blit(&mut sheet, &pb, row, 2);

            // Col 3: Used / spent (North)
            let mut w_used = World::new(theme, 7 + row as u64);
            let trap_pos_u = w_used.level.objects.sticky_traps[0].pos;
            w_used.elapsed = 2.0;
            w_used.level.objects.sticky_traps[0] = StickyTrapView {
                pos: trap_pos_u,
                facing: Vec2::new(0.0, -1.0),
                state: StickyTrapState::Used,
                progress: 1.0,
                anim: 0.0,
                victim_pos: trap_pos_u,
                victim_radius: 3.0,
                // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
            };
            w_used.player.pos = trap_pos_u + Vec2::new(26.0, -4.0);
            w_used.player.face = Vec2::new(-1.0, 0.0);
            let mut pb = PixelBuffer::new(VW, VH);
            w_used.draw(&mut pb);
            blit(&mut sheet, &pb, row, 3);
        }

        let mut out = format!("P6\n{} {}\n255\n", total_w, total_h).into_bytes();
        out.extend_from_slice(&sheet);
        std::fs::write("/tmp/ratrun_sticky_level.ppm", out).unwrap();
    }

    /// Wave 3 sheet: `RATRUN_DUMP=sticky_wave3 cargo test dump_sticky_wave3_sheet`
    ///
    /// Writes `/tmp/ratrun_sticky_wave3.ppm` (or `RATRUN_DUMP_DIR`) — a real
    /// `World::draw` frame per case, so the *load* and the *wear* of a glue
    /// board can be eyeballed: a board at 0, 3 and 8 victims; fresh, half-spent,
    /// nearly spent and Used; the torn scars a released crowd leaves; and the
    /// two mousetrap kinds with each bait.
    #[test]
    fn dump_sticky_wave3_sheet() {
        let target = match std::env::var("RATRUN_DUMP") {
            Ok(v) => v,
            Err(_) => return,
        };
        if target != "sticky_wave3" {
            return;
        }
        use crate::game::enemy::EnemyKind;
        use crate::game::world::World;
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        const VW: usize = 150;
        const VH: usize = 104;
        const COLS: usize = 4;
        const ROWS: usize = 2;
        let total_w = VW * COLS;
        let total_h = VH * ROWS;
        let mut sheet = vec![0u8; total_w * total_h * 3];
        let mut pb = PixelBuffer::new(VW, VH);

        // One case: a board at the arena centre with `n` stuck Skitterers (their
        // sprites stand on the pads the art draws, exactly as the sim pins them)
        // and a given wear share.
        let mut case = |row: usize, col: usize, n: usize, worn: f32, used: bool, tears: usize| {
            let theme = if row == 0 { Theme::Sewer } else { Theme::Kitchen };
            let mut w = World::new(theme, 11 + (row * COLS + col) as u64);
            w.level.objects.mousetraps.clear();
            w.level.objects.sticky_traps.clear();
            let c = w.level.arena * 0.5;
            w.player.pos = c + Vec2::new(26.0, 30.0);
            w.player.face = Vec2::new(0.0, -1.0);
            w.elapsed = 1.2 + col as f32 * 0.3;

            let mut rng = StdRng::seed_from_u64(3);
            let mut v = StickyTrapView::new(c, Vec2::new(1.0, 0.0));
            v.state = if used {
                StickyTrapState::Used
            } else if n > 0 {
                StickyTrapState::Stuck
            } else {
                StickyTrapState::Idle
            };
            v.worn = worn;
            v.load = (n as u8).min(STICKY_TRAP_CAPACITY);
            v.progress = if n > 0 { 0.35 } else { 0.0 };
            for k in 0..tears.min(STICKY_TRAP_TEARS) {
                let a = k as f32 * 1.9 + 0.4;
                v.add_tear(c + Vec2::new(a.cos() * 7.0, a.sin() * 5.0));
            }
            for (k, slot) in v.victims.iter_mut().take(n.min(STICKY_TRAP_SLOTS)).enumerate() {
                // The sim's own slot spread, near enough for a sheet.
                let inset = Vec2::new(
                    STICKY_TRAP_PAD_HALF.x - 2.5,
                    STICKY_TRAP_PAD_HALF.y - 2.5,
                );
                let a = k as f32 * 2.399;
                let at = c + Vec2::new(a.cos() * inset.x * 0.7, a.sin() * inset.y * 0.7);
                *slot = Some(StickyVictimView {
                    pos: at,
                    radius: 2.5,
                    kind: StickyVictimKind::Small,
                    color: EnemyKind::Skitterer.color(),
                    progress: 0.2 + 0.08 * k as f32,
                });
                w.spawn_enemy_with_rng(EnemyKind::Skitterer, at, 1.0, &mut rng);
            }
            if n > 0 {
                v.victim_pos = v.victims[0].map(|x| x.pos).unwrap_or(c);
                v.victim_radius = 2.5;
            }
            w.level.objects.sticky_traps.push(v);

            pb.clear((0, 0, 0));
            w.draw(&mut pb);
            for y in 0..VH {
                for x in 0..VW {
                    let px = pb.pixel_at(x, y);
                    let gx = col * VW + x;
                    let gy = row * VH + y;
                    let idx = (gy * total_w + gx) * 3;
                    sheet[idx] = px.0;
                    sheet[idx + 1] = px.1;
                    sheet[idx + 2] = px.2;
                }
            }
        };

        // Row 0: the load. A fresh board, three victims, a full crowd (8), and a
        // half-spent board with a crowd on it.
        case(0, 0, 0, 0.0, false, 0);
        case(0, 1, 3, 0.08, false, 0);
        case(0, 2, 8, 0.14, false, 0);
        case(0, 3, 3, 0.50, false, 1);
        // Row 1: the wear. Nearly spent with a crowd, a spent Used decal, a
        // board whose crowd pulled free (torn scars), and the same at night.
        case(1, 0, 6, 0.92, false, 2);
        case(1, 1, 0, 1.0, true, 0);
        case(1, 2, 1, 0.72, false, 4);
        case(1, 3, 2, 0.30, false, 0);

        let mut out = format!("P6\n{} {}\n255\n", total_w, total_h).into_bytes();
        out.extend_from_slice(&sheet);
        let dir = std::env::var_os("RATRUN_DUMP_DIR")
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        std::fs::write(dir.join("ratrun_sticky_wave3.ppm"), out).unwrap();
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
        const CELL_W: usize = 64;
        const CELL_H: usize = 48;
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                            // The wave-3 fields (load, wear, victims, tears) default to an empty board.
    ..StickyTrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
                    // Ordinary bar, cheese bait: the pre-wave-3 trap.
    ..MousetrapView::new(Vec2::ZERO, Vec2::new(1.0, 0.0))
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
