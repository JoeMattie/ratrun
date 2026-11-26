//! The interactive-object runtime: one-way doors, the rat-hole feel change and
//! the maze-cache heist.
//!
//! [`LevelObjects`](crate::game::level::LevelObjects) already carries the art
//! modules' own `Copy` view structs (`OneWayDoorView`, `RatHoleView`,
//! `MazeRewardView`); this file is the state machine that drives them, i.e. the
//! "object runtime" the content plan reserves. It is three systems that share
//! one [`GateRuntime`] scratch resource, chained directly after the fog reveal:
//!
//! | system | drives | slot |
//! |---|---|---|
//! | [`passage_system`] | `RatHoleView` (`Idle`/`Squeezing`/`Blocked`), the rat's lateral centring in the final 6 px, the fog's crossed-hole note | 3 |
//! | [`cache_system`] | `MazeRewardView` (`Idle`→`Opening`→`Opened`), one free level-up, the 0.8 s grace, the minimap claim | 4 |
//! | [`door_system`] | `OneWayDoorView` (`Closed`/`Opening`/`Closing`/`Refused`) and the exit-gate slam behind the cache run | 5 |
//!
//! Order is `player → fog → passage → cache → door → flow_refresh`: all three
//! read the rat's **post-move** position, the fog has already recorded a hole
//! crossing, and nothing downstream (steering, weapons, loot) sees a stale
//! player cell.
//!
//! ## What the art files bind (do not "fix" these)
//!
//! * A rat-hole is an 8 px gap in a 16 px wall cell: 4 px piers on each
//!   passage-lateral side ([`art_walls::wall_colliders`]). With `PLAYER_RADIUS`
//!   = 3.0 that leaves ~1 px of centre tolerance each side, which is why
//!   [`passage_system`] centres the rat laterally.
//! * Door timing comes from [`art_traps::OneWayDoorState`] (0.24 s open,
//!   0.35 s close, 0.40 s refused), **not** `art_walls`' 0.15 s note
//!   (CONTENT_PLAN §9.2).
//! * Enemies never pass a gate, from either side, ever: collision resolves them
//!   with `player_vel: None` (`TileMap::resolve_circle`), so this file only
//!   animates the leaf. The slam is a *visual/audio* event plus the cliff the
//!   horde already stands behind.
//!
//! ## Determinism
//!
//! Rng is drawn at exactly three places, each where an *event* happens and its
//! [`vfx`](crate::game::particle::vfx) burst fires: the rat-hole squeeze, the
//! cache opening and the exit-gate slam. The squeeze's travel direction comes
//! from the lowest-`SpawnId` small body in the tunnel (never from the query's
//! storage order), and holes are walked in `LevelObjects` order, so twin runs
//! draw the same numbers in the same order.
//!
//! Everything else is a pure function of the rat's position, its intent, the
//! live enemies and the previous state, so the only other way this file moves a
//! scripted trace is through the values it *writes* (`pending_levelups`,
//! `player.iframes`, entity positions during centring). Every loop runs over a
//! `Vec` in `LevelObjects` order (never a hash map), and no system allocates per
//! tick: `GateRuntime`'s buffers are sized once per level (`resize_with` on a
//! key change) and the two `Local<Vec>` scratch buffers are cleared and reused.
//!
//! ## Sound + effects (one variant per edge, never a stand-in)
//!
//! | edge | `Sfx` | vfx |
//! |---|---|---|
//! | the exit leaf starts opening | `GateOpen` | — |
//! | the rat pushes the backstop (once per approach) | `GateRefuse` | — |
//! | the exit slams behind the cache run | `GateSlam` | `emit_one_way_gate_slam` |
//! | the rat or a small enemy squeezes a hole | `HoleSqueeze` | `emit_rat_hole_squeeze` |
//! | the cache is claimed | `CacheOpen` | `emit_maze_cache_open` |
//!
//! Enemy squeezes are rate-limited to one sound per hole per
//! [`HOLE_SQUEEZE_SOUND_SECS`] so a stream of Skitterers is not a machine gun;
//! the rat's own squeeze always plays (it is the feedback that explains the
//! 0.25 s animation).

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::art_traps::{
    MazeRewardState, OneWayDoorState, RatHoleState, MazeRewardView, OneWayDoorView, RatHoleView,
    MAZE_REWARD_INTERACT_RADIUS, MAZE_REWARD_OPEN_SECS, ONE_WAY_DOOR_CLOSE_SECS,
    ONE_WAY_DOOR_HALF_U, ONE_WAY_DOOR_OPEN_SECS, ONE_WAY_DOOR_PASSAGE_HALF_V,
    ONE_WAY_DOOR_REFUSED_SECS, ONE_WAY_DOOR_TRIGGER_DIST, RAT_HOLE_BLOCKED_SECS,
    RAT_HOLE_GAP_HALF_V, RAT_HOLE_HALF_U, RAT_HOLE_SIZE, RAT_HOLE_SQUEEZE_SECS,
};
use crate::game::art_walls::CELL_SIZE_F;
use crate::game::collision::{SizeClass, TileMap};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, RunCounters, SfxQueue, TickInput};
use crate::game::enemy::Enemy;
use crate::game::fog::Fog;
use crate::game::level::Level;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::palette::Rgb;

// ============================================================================
// Tuning (owner-approved gameplay numbers; see the report)
// ============================================================================

/// Invulnerability granted when the rat loots a cache: the art spec's `0.8 s`
/// (`MazeRewardState::Opening`) and the designer's "0.8 s grace" for the heist.
pub const CACHE_GRACE_SECS: f32 = 0.8;

/// The bit of the rat's approach to a rat-hole that gets lateral help: its
/// centre within `|u| ∈ [CELL/2, CELL/2 + LEAD]`, i.e. the final 6 px in front
/// of the wall face (`|u|` is measured from the hole cell's centre).
pub const HOLE_CENTRE_LEAD: f32 = 6.0;
/// Lateral centring speed toward the gap's centre line (px/s). The rat walks at
/// 58 px/s, so 6 px of approach buys ~3.7 px of correction — enough to thread a
/// 3.0 px body through the 8 px gap from the edge of the tolerance, and gentle
/// enough that it never feels like a rail.
pub const HOLE_CENTRE_SPEED: f32 = 40.0;
/// Half-width of the rat's input cone: it must point at least this much along
/// the passage for the centring to engage (`cos 75°`).
const HOLE_CENTRE_INTENT_DOT: f32 = 0.25;

/// Hole state is only *detected* (scanned for bodies) within this distance of
/// the rat. Holes further away keep advancing their timers, but a crowd 200 px
/// off screen is not worth 600 distance tests per hole per tick ("no per-tick
/// work that cannot be seen"). The view state is unaffected: a hole off screen
/// is not drawn.
pub const SCENE_RADIUS: f32 = 200.0;

/// Tolerance (px) added to a body's reach when testing whether it is pressing
/// against a hole's cheeks.
const CHEEK_PRESS_SLACK: f32 = 1.0;

/// Minimum spacing between two *enemy* squeeze sounds on the same hole: a
/// stream of Skitterers through one tunnel reads as traffic, not as a machine
/// gun. The rat's own squeeze is never suppressed.
pub const HOLE_SQUEEZE_SOUND_SECS: f32 = 0.4;

/// Minimum spacing between two `GateOpen` sounds on the same leaf, so a rat
/// hovering on the trigger line cannot rattle it open/closed every frame.
const GATE_OPEN_SOUND_SECS: f32 = 0.5;

/// How far short of `ONE_WAY_DOOR_HALF_U` the open zone stops on the `+facing`
/// side (px). A rat pressed against the shut leaf from the wrong side rests at
/// exactly `u = ONE_WAY_DOOR_HALF_U`; the margin keeps that spot (and its float
/// noise) out of the zone that opens and arms the door.
const OPEN_ZONE_BACKSTOP_MARGIN: f32 = 0.25;

/// Longest single-tick move along a door's passage axis that still counts as
/// the rat walking through it (px): three cells, several times any dash on a
/// 50 ms frame, and well short of a placed/teleported rat.
const MAX_CROSSING_STEP: f32 = 48.0;

// ============================================================================
// Per-object runtime state
// ============================================================================

/// One node of the HUD's maze feed: what happened, in tick order.
///
/// This is the *distinct pickup record* the cache run pushes: the existing
/// `LastPickups` feed only knows the three floor pickups
/// (`Heal`/`Magnet`/`Nuke`), so a maze cache claim would read as one of those.
/// `GateRuntime::feed` is cleared at the start of every tick by
/// [`cache_system`] and appended to by [`cache_system`]/[`door_system`]; a HUD
/// callout ("+CACHE", "…SEALED BEHIND YOU") reads it through
/// [`GateRuntime::feed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MazeEvent {
    /// The rat looted maze `index`'s cache: one free upgrade + the grace window.
    CacheClaimed { index: usize },
    /// Maze `index`'s one-way exit slammed shut behind the rat.
    GateSlammed { index: usize },
}

/// Runtime state of one `OneWayDoor` (the view holds the animation; this holds
/// the approach latch and the one-shot slam).
#[derive(Clone, Copy, Debug, Default)]
pub struct DoorRuntime {
    /// The wrong-side `Refused` reaction already fired for this approach, so a
    /// rat leaning on the backstop does not restart the flash every tick.
    pub refused_latched: bool,
    /// The exit slammed behind the rat after the cache run: one latch sound,
    /// ever.
    pub slammed: bool,
    /// The rat has been on the maze side of this leaf (it opened at least
    /// once), so a later crossing from `+facing` is a real exit, not a walk
    /// past the outside of the maze.
    pub armed: bool,
    /// How many times the `Refused` reaction fired (tests/diagnostics).
    pub refusals: u32,
    /// Seconds until `GateOpen` may play again on this leaf.
    pub open_cd: f32,
    /// How many times the leaf started opening (tests/diagnostics).
    pub opens: u32,
    /// The rat's `u` on the previous tick, while it was laterally inside the
    /// doorway (`None` otherwise). Lets a crossing faster than the sampled
    /// windows — a dash on a long frame — still arm and slam the leaf.
    pub last_u: Option<f32>,
}

/// Runtime state of one `RatHole`.
#[derive(Clone, Copy, Debug, Default)]
pub struct HoleRuntime {
    /// A passable body's centre is inside the tunnel this tick.
    pub inside: bool,
    /// `0.0..=1.0` progress of the squeeze animation.
    pub squeeze: f32,
    /// Remaining `Blocked` animation time (seconds).
    pub blocked_left: f32,
    /// The `Blocked` reaction already fired for this approach, so a packed
    /// crowd does not retrigger it every tick.
    pub blocked_latched: bool,
    /// Squeezes / blocked reactions fired (tests/diagnostics).
    pub squeezes: u32,
    pub blocks: u32,
    /// Seconds until another *enemy* squeeze may play its sound here
    /// ([`HOLE_SQUEEZE_SOUND_SECS`]).
    pub squeeze_cd: f32,
    /// Squeeze sounds actually played on this hole (tests/diagnostics).
    pub squeeze_sounds: u32,
}

/// Runtime state of one `MazeReward`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RewardRuntime {
    /// The cache was claimed (exactly once; the view state also proves it).
    pub claimed: bool,
}

/// Identity of the level the buffers were sized for. A test that swaps a map in
/// place must change one of these (same convention as `spawn::SpawnScratch`).
#[derive(Clone, Copy, PartialEq, Eq)]
struct LevelKey {
    seed: u64,
    doors: usize,
    holes: usize,
    rewards: usize,
}

/// Per-tick scratch + per-object runtime for the object layer.
///
/// Inserted permanently in [`World::new`](crate::game::world::World::new), like
/// `EnemyIndex`/`GridScratch`: it is not a facade singleton, so it never
/// round-trips, and it allocates only when the level identity changes.
#[derive(Resource, Default)]
pub struct GateRuntime {
    key: Option<LevelKey>,
    pub doors: Vec<DoorRuntime>,
    pub holes: Vec<HoleRuntime>,
    pub rewards: Vec<RewardRuntime>,
    /// `MazeEvent`s of the most recent tick (the HUD's maze feed).
    pub feed: Vec<MazeEvent>,
}

impl GateRuntime {
    /// What the HUD should show for the most recent tick.
    #[inline]
    pub fn feed(&self) -> &[MazeEvent] {
        &self.feed
    }

    /// Size (or reset) the per-object buffers for `level`.
    fn sync(&mut self, level: &Level) {
        let key = LevelKey {
            seed: level.seed,
            doors: level.objects.one_way_doors.len(),
            holes: level.objects.rat_holes.len(),
            rewards: level.objects.rewards.len(),
        };
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.doors.clear();
        self.doors.resize_with(key.doors, DoorRuntime::default);
        self.holes.clear();
        self.holes.resize_with(key.holes, HoleRuntime::default);
        self.rewards.clear();
        self.rewards.resize_with(key.rewards, RewardRuntime::default);
        self.feed.clear();
    }
}

// ============================================================================
// 3. Rat-holes: squeeze, blocked, centring, fog
// ============================================================================

/// Drive the rat-holes the rat is near: `Squeezing` while the rat or a small
/// swarmer is in the tunnel, `Blocked` once per approach when a large body
/// (Cat / Brute / boss) presses against the cheeks, the rat's lateral centring,
/// and the fog's crossed-hole note.
///
/// The squeeze is **animation only**: it never touches `player.pos`, so the rat
/// never slows, stops or gets held in a hole.
#[allow(clippy::too_many_arguments)]
pub fn passage_system(
    input: Res<TickInput>,
    mut level: ResMut<Level>,
    mut player: ResMut<Player>,
    enemies: Query<(&SpawnId, &Enemy)>,
    mut fog: ResMut<Fog>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
    mut rt: ResMut<GateRuntime>,
    mut crossed: Local<Vec<(i32, i32)>>,
) {
    let dt = input.dt.max(0.0);
    rt.sync(&level);
    crossed.clear();
    let intent = if input.move_dir.len_sq() > 0.01 {
        input.move_dir.normalized()
    } else {
        Vec2::ZERO
    };
    let player_vel = player.vel;
    let (player_radius, player_class) = player.movement_body(&level.tiles);

    for i in 0..level.objects.rat_holes.len() {
        let (cell, view) = {
            let hole = &level.objects.rat_holes[i];
            (hole.cell, hole.view)
        };
        let f = safe_dir(view.facing);
        let near = player.pos.dist_sq(view.pos) <= SCENE_RADIUS * SCENE_RADIUS;

        let mut inside = false;
        let mut player_inside = false;
        let mut blocked = false;
        // The lowest-`SpawnId` small body in the tunnel: the squeeze's travel
        // direction must not depend on the query's storage order.
        let mut smallest: Option<(u64, Vec2)> = None;
        if near {
            if player_class == SizeClass::Large {
                blocked = presses_cheeks(player.pos, view.pos, f, player_radius);
            } else if in_tunnel(player.pos, view.pos, f) {
                inside = true;
                player_inside = true;
                crossed.push(cell);
            }
            for (id, e) in enemies.iter() {
                if !e.alive() {
                    continue;
                }
                match SizeClass::of_enemy(e.kind) {
                    SizeClass::Small => {
                        if in_tunnel(e.pos, view.pos, f) {
                            inside = true;
                            if smallest.is_none_or(|(best, _)| id.0 < best) {
                                smallest = Some((id.0, e.vel));
                            }
                        }
                    }
                    SizeClass::Large => {
                        if presses_cheeks(e.pos, view.pos, f, e.radius) {
                            blocked = true;
                        }
                    }
                }
            }
            if player_class == SizeClass::Small {
                centre_player(&mut player, &level.tiles, &view, intent, dt);
            }
        }

        let was_inside = rt.holes[i].inside;
        let mut v = view;
        update_hole(&mut v, &mut rt.holes[i], inside, blocked, dt);
        level.objects.rat_holes[i].view = v;

        // A body just entered the tunnel: one puff out of each mouth, one
        // sound — unless an enemy squeeze played on this hole recently.
        if inside && !was_inside {
            let travel = if player_inside {
                player_vel
            } else {
                smallest.map_or(Vec2::ZERO, |(_, vel)| vel)
            };
            let allowed = player_inside || rt.holes[i].squeeze_cd <= 0.0;
            if allowed {
                rt.holes[i].squeeze_cd = HOLE_SQUEEZE_SOUND_SECS;
                rt.holes[i].squeeze_sounds = rt.holes[i].squeeze_sounds.saturating_add(1);
                vfx::emit_rat_hole_squeeze(&mut particles, &mut rng.0, v.pos, f, travel);
                sfx.0.push(Sfx::HoleSqueeze);
            }
        }
    }

    // Tell the fog the rat crossed a hole (its `crossed` list is what lets a
    // known shortcut keep showing its far mouth). `reveal_cell` is the public
    // "the rat stood here" entry point and notes the crossing itself; it is
    // skipped when the reveal is switched off (the fog tests do that).
    for &cell in crossed.iter() {
        if fog.reveal_enabled && !fog.hole_crossed(cell) {
            fog.reveal_cell(&level, cell);
        }
    }
}

/// One rat-hole's state machine.
fn update_hole(
    view: &mut RatHoleView,
    rt: &mut HoleRuntime,
    inside: bool,
    blocked: bool,
    dt: f32,
) {
    // Squeeze: ramps over RAT_HOLE_SQUEEZE_SECS while a passable body is in the
    // tunnel, holds at 1.0 while it is still inside, and plays out its tail
    // after a fast (dashed) crossing.
    if inside {
        if !rt.inside {
            rt.squeeze = 0.0;
            rt.squeezes = rt.squeezes.saturating_add(1);
        }
        rt.squeeze = (rt.squeeze + dt / RAT_HOLE_SQUEEZE_SECS).min(1.0);
    } else if rt.squeeze > 0.0 && rt.squeeze < 1.0 {
        rt.squeeze = (rt.squeeze + dt / RAT_HOLE_SQUEEZE_SECS).min(1.0);
    }
    rt.inside = inside;
    let squeezing = inside || (rt.squeeze > 0.0 && rt.squeeze < 1.0);

    // Blocked: one reaction per approach. The latch clears only once nothing
    // large is pressing any more, so a crowd that keeps shoving stays silent.
    if !inside && blocked {
        if !rt.blocked_latched {
            rt.blocked_latched = true;
            rt.blocks = rt.blocks.saturating_add(1);
            rt.blocked_left = RAT_HOLE_BLOCKED_SECS;
        }
    } else if !blocked {
        rt.blocked_latched = false;
    }
    rt.blocked_left = (rt.blocked_left - dt).max(0.0);
    rt.squeeze_cd = (rt.squeeze_cd - dt).max(0.0);

    if squeezing {
        view.state = RatHoleState::Squeezing;
        view.progress = rt.squeeze;
    } else if rt.blocked_left > 0.0 {
        view.state = RatHoleState::Blocked;
        view.progress = 1.0 - rt.blocked_left / RAT_HOLE_BLOCKED_SECS;
    } else {
        view.state = RatHoleState::Idle;
        view.progress = 0.0;
        rt.squeeze = 0.0;
    }
}

/// Gentle lateral centring for the rat in the last 6 px before a hole.
///
/// The 8 px gap around a 6 px rat leaves ~1 px of centre tolerance each side,
/// so a rat that is *aiming* through the arch gets pulled toward its centre
/// line at [`HOLE_CENTRE_SPEED`]. It only ever moves laterally, only within
/// [`HOLE_CENTRE_LEAD`] of the wall face, and only when the resulting position
/// is clear for the rat's radius — the centring can never push the rat into a
/// pier (the check is the tile grid's own circle test).
fn centre_player(
    player: &mut Player,
    tiles: &TileMap,
    view: &RatHoleView,
    intent: Vec2,
    dt: f32,
) {
    if intent.len_sq() <= 0.01 {
        return;
    }
    let f = safe_dir(view.facing);
    let half = CELL_SIZE_F * 0.5;
    let (u, v) = local(player.pos, view.pos, f);
    if u.abs() < half || u.abs() > half + HOLE_CENTRE_LEAD {
        return;
    }
    if v.abs() > RAT_HOLE_GAP_HALF_V {
        return;
    }
    // The input must point *through* the hole (the rat can approach either way).
    let through = f * -u.signum();
    if intent.dot(through) < HOLE_CENTRE_INTENT_DOT {
        return;
    }
    let step = (-v).clamp(-HOLE_CENTRE_SPEED * dt, HOLE_CENTRE_SPEED * dt);
    if step.abs() < 1e-4 {
        return;
    }
    let candidate = player.pos + f.perp() * step;
    if tiles.circle_clear(candidate, PLAYER_RADIUS, SizeClass::Small) {
        player.pos = candidate;
    }
}

/// Is `p`'s centre inside the hole's tunnel (`|u| <= RAT_HOLE_HALF_U`,
/// `|v| <= RAT_HOLE_GAP_HALF_V`)?
#[inline]
fn in_tunnel(p: Vec2, origin: Vec2, f: Vec2) -> bool {
    let (u, v) = local(p, origin, f);
    u.abs() <= RAT_HOLE_HALF_U && v.abs() <= RAT_HOLE_GAP_HALF_V
}

/// Is a body of `radius` pressing against the hole's cheeks (the wide outer
/// shoulders a Cat/Brute cannot fit through)?
#[inline]
fn presses_cheeks(p: Vec2, origin: Vec2, f: Vec2, radius: f32) -> bool {
    let (u, v) = local(p, origin, f);
    let reach = CELL_SIZE_F * 0.5 + radius + CHEEK_PRESS_SLACK;
    u.abs() <= reach && v.abs() <= RAT_HOLE_SIZE.y * 0.5
}

// ============================================================================
// 4. The cache run as a heist
// ============================================================================

/// Drive the maze rewards: claim on approach (one free upgrade, the grace
/// window, the minimap mark, one feed record) and play `Opening` → `Opened`.
///
/// The claim raises `RunCounters::pending_levelups` — the same counter the XP
/// path (`loot::gems_system`) raises — so the App opens its normal 1-of-3
/// upgrade screen. No XP is granted.
#[allow(clippy::too_many_arguments)]
pub fn cache_system(
    input: Res<TickInput>,
    mut level: ResMut<Level>,
    mut player: ResMut<Player>,
    mut counters: ResMut<RunCounters>,
    mut fog: ResMut<Fog>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
    mut rt: ResMut<GateRuntime>,
) {
    let dt = input.dt.max(0.0);
    rt.sync(&level);
    // This system owns the head of the per-tick maze feed; `door_system`
    // appends to it afterwards.
    rt.feed.clear();
    // Hoisted so the reward borrow below stays disjoint from the palette read.
    let accent = level.palette.accent;

    for i in 0..level.objects.rewards.len() {
        let reward: &mut MazeRewardView = &mut level.objects.rewards[i];
        match reward.state {
            MazeRewardState::Idle => {
                if rt.rewards[i].claimed
                    || player.pos.dist(reward.pos) > MAZE_REWARD_INTERACT_RADIUS
                {
                    continue;
                }
                rt.rewards[i].claimed = true;
                reward.state = MazeRewardState::Opening;
                reward.progress = 0.0;
                // One normal three-choice upgrade, exactly once.
                counters.pending_levelups = counters.pending_levelups.saturating_add(1);
                // 0.8 s of grace so looting a tight maze end never feels unfair.
                player.iframes = player.iframes.max(CACHE_GRACE_SECS);
                // The minimap's claimed cache pip (`World::maze_markers`).
                fog.set_maze_claimed(i, true);
                rt.feed.push(MazeEvent::CacheClaimed { index: i });
                // Coin fountain + gold shockwave on the pedestal, and the
                // cache's own sting (never the level-up one: the upgrade card
                // this opens plays its own sound).
                let tint = reward.reward_tint.unwrap_or(accent);
                vfx::emit_maze_cache_open(&mut particles, &mut rng.0, reward.pos, tint);
                sfx.0.push(Sfx::CacheOpen);
            }
            MazeRewardState::Opening => {
                reward.progress = (reward.progress + dt / MAZE_REWARD_OPEN_SECS).min(1.0);
                if reward.progress >= 1.0 {
                    reward.state = MazeRewardState::Opened;
                }
            }
            MazeRewardState::Opened => {}
        }
    }
}

// ============================================================================
// 5. One-way doors
// ============================================================================

/// Drive every one-way door's leaf and the exit-gate slam.
///
/// * `Closed` → `Opening` when the rat approaches through the `-facing` side
///   (the maze interior), `0.24 s` to swing open.
/// * held at `progress = 1.0` while the rat's body overlaps the doorway
///   (`|u| <= 8`, `|v| <= 9`), then `Closing` (`0.35 s`) once it clears.
/// * `Refused` (`0.40 s`, one flash per approach) when the rat pushes into the
///   backstop from the `+facing` side.
/// * **Slam**: once that maze's cache is claimed and the rat has actually been
///   through the leaf, the tick its body just clears the exit (`8 < u <= 18`,
///   so a rat walking around the outside never trips it) the leaf shuts in one
///   step (`Closed`, `hit_flash = 1`) with a single latch sound — the pack that
///   followed has to go back around, and collision keeps it behind either way.
pub fn door_system(
    input: Res<TickInput>,
    mut level: ResMut<Level>,
    player: Res<Player>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
    mut rt: ResMut<GateRuntime>,
) {
    let dt = input.dt.max(0.0);
    rt.sync(&level);
    let intent = if input.move_dir.len_sq() > 0.01 {
        input.move_dir.normalized()
    } else {
        Vec2::ZERO
    };
    // One reborrow of the scratch so `doors[i]` and `feed` are disjoint fields.
    let rt: &mut GateRuntime = &mut rt;
    let mut fx = ObjectFx {
        sfx: &mut sfx.0,
        particles: &mut particles,
        rng: &mut rng.0,
        accent: level.palette.accent,
    };
    let (radius, class) = player.movement_body(&level.tiles);

    for i in 0..level.objects.one_way_doors.len() {
        // Doors and rewards are pushed in `mazes` order, so door `i` is the
        // exit of maze `i` (the cache whose claim arms the slam).
        let claimed = rt.rewards.get(i).is_some_and(|r| r.claimed);
        let mut view = level.objects.one_way_doors[i].view;
        update_door(
            &mut view,
            &mut rt.doors[i],
            player.pos,
            radius,
            class == SizeClass::Small,
            intent,
            claimed,
            dt,
            &mut fx,
            i,
            &mut rt.feed,
        );
        level.objects.one_way_doors[i].view = view;
    }
}

/// The feedback an object state edge owns: one sound, one burst, the theme's
/// accent colour. Bundled so the state machines keep a readable signature.
struct ObjectFx<'a> {
    sfx: &'a mut Vec<Sfx>,
    particles: &'a mut Particles,
    rng: &'a mut rand::rngs::StdRng,
    accent: Rgb,
}

impl ObjectFx<'_> {
    /// The leaf started opening (once the anti-rattle window is clear).
    fn gate_open(&mut self, rt: &mut DoorRuntime) {
        rt.open_cd = GATE_OPEN_SOUND_SECS;
        rt.opens = rt.opens.saturating_add(1);
        self.sfx.push(Sfx::GateOpen);
    }

    /// The rat pushed the backstop: one sharp refusal per approach.
    fn gate_refuse(&mut self) {
        self.sfx.push(Sfx::GateRefuse);
    }

    /// The heist's latch: a slam along the whole threshold.
    fn gate_slam(&mut self, view: &OneWayDoorView) {
        vfx::emit_one_way_gate_slam(
            self.particles,
            self.rng,
            view.pos,
            view.facing,
            self.accent,
        );
        self.sfx.push(Sfx::GateSlam);
    }
}

/// The heist's punchline: shut the leaf in one step behind the rat's tail,
/// flash the impact, play one latch sound and feed the HUD. Collision already
/// keeps the pack behind it (enemies never pass a gate).
fn slam_door(
    view: &mut OneWayDoorView,
    rt: &mut DoorRuntime,
    index: usize,
    fx: &mut ObjectFx<'_>,
    feed: &mut Vec<MazeEvent>,
) {
    view.state = OneWayDoorState::Closed;
    view.progress = 0.0;
    view.hit_flash = 1.0;
    rt.slammed = true;
    feed.push(MazeEvent::GateSlammed { index });
    let feedback_view = *view;
    fx.gate_slam(&feedback_view);
}

/// One door's state machine. Arguments are what it needs, not what the schedule
/// has: the view, its runtime, the rat, and the per-tick feed.
#[allow(clippy::too_many_arguments)]
fn update_door(
    view: &mut OneWayDoorView,
    rt: &mut DoorRuntime,
    player_pos: Vec2,
    radius: f32,
    can_open: bool,
    intent: Vec2,
    claimed_exit: bool,
    dt: f32,
    fx: &mut ObjectFx<'_>,
    index: usize,
    feed: &mut Vec<MazeEvent>,
) {
    let f = safe_dir(view.facing);
    let (u, v) = local(player_pos, view.pos, f);
    let lateral = v.abs() <= ONE_WAY_DOOR_PASSAGE_HALF_V;
    // The rat's body overlaps the doorway: `|u| <= 8` (leaf half-thickness plus
    // the rat) and `|v| <= 9` (`ONE_WAY_DOOR_PASSAGE_HALF_V`), per the art doc.
    let held = can_open && lateral && u.abs() <= ONE_WAY_DOOR_HALF_U + radius;
    // Approach through the maze side: the art's `u in [-10, 0]` trigger, opened
    // out to the leaf plane so a rat that is already sliding through counts.
    //
    // The `+facing` end stops `OPEN_ZONE_BACKSTOP_MARGIN` short of
    // `ONE_WAY_DOOR_HALF_U`: the shut leaf's collider is 2 px either side of
    // the cell centre, so a rat leaning on the backstop from the wrong side
    // rests at `u = 2 + PLAYER_RADIUS = 5.0` give or take a float ulp. With an
    // inclusive `..= 5.0` that resting spot was half inside the open zone: the
    // leaf swung open for a rat it was refusing and `armed` latched from the
    // outside.
    let open_zone = can_open && lateral
        && (-ONE_WAY_DOOR_TRIGGER_DIST..=ONE_WAY_DOOR_HALF_U - OPEN_ZONE_BACKSTOP_MARGIN)
            .contains(&u);
    // Pushing into the backstop from the `+facing` side.
    let pressing_wrong = if can_open {
        lateral && (-ONE_WAY_DOOR_HALF_U..=ONE_WAY_DOOR_TRIGGER_DIST).contains(&u)
            && intent.dot(f) < -0.3
    } else {
        lateral && u.abs() <= ONE_WAY_DOOR_HALF_U + radius + 0.01
            && intent.dot(f) * u.signum() < -0.3
    };
    if !pressing_wrong {
        rt.refused_latched = false;
    }
    view.hit_flash = (view.hit_flash - dt * 3.0).max(0.0);
    rt.open_cd = (rt.open_cd - dt).max(0.0);

    let open = match view.state {
        OneWayDoorState::Opening => view.progress.clamp(0.0, 1.0),
        OneWayDoorState::Closing => 1.0 - view.progress.clamp(0.0, 1.0),
        _ => 0.0,
    };
    // A whole crossing inside one tick: last tick the rat was on the maze side
    // of the backstop (or inside the leaf), this tick its body is clear on the
    // `+facing` side, and both samples are in the doorway's lane. The open
    // zone and the clearing window below are 10-15 px wide and sampled once a
    // tick, so a dash on a long frame could step over either: the leaf then
    // never armed, or the slam waited for the rat's next stroll past the
    // outside. The player's move is sub-stepped against the tile grid, so the
    // only way from one side to the other is through the leaf; the step bound
    // keeps a teleport (a test, the console) from counting as a walk.
    let last_u = rt.last_u;
    rt.last_u = (can_open && lateral).then_some(u);
    let crossed = can_open && lateral
        && u >= ONE_WAY_DOOR_HALF_U + radius
        && last_u.is_some_and(|prev| {
            prev < ONE_WAY_DOOR_HALF_U - OPEN_ZONE_BACKSTOP_MARGIN
                && u - prev <= MAX_CROSSING_STEP
        });
    rt.armed |= open_zone || crossed;
    // The rat's body has just cleared the leaf into the `+facing` side. The
    // window is bounded (`u <= 8 + ONE_WAY_DOOR_TRIGGER_DIST`) so a rat that
    // walked around the outside of the maze never trips the slam.
    let cleared = crossed
        || (can_open && lateral
            && (ONE_WAY_DOOR_HALF_U + radius
                ..=ONE_WAY_DOOR_HALF_U + radius + ONE_WAY_DOOR_TRIGGER_DIST)
                .contains(&u));

    if claimed_exit && rt.armed && !rt.slammed && cleared {
        slam_door(view, rt, index, fx, feed);
        return;
    }

    match view.state {
        OneWayDoorState::Closed => {
            if open_zone {
                view.state = OneWayDoorState::Opening;
                view.progress = 0.0;
                if rt.open_cd <= 0.0 {
                    fx.gate_open(rt);
                }
            } else if pressing_wrong && !rt.refused_latched {
                view.state = OneWayDoorState::Refused;
                view.progress = 0.0;
                view.hit_flash = 1.0;
                rt.refused_latched = true;
                rt.refusals = rt.refusals.saturating_add(1);
                fx.gate_refuse();
            }
        }
        OneWayDoorState::Opening => {
            if held {
                // Held wide open while the body is in the doorway.
                view.progress = 1.0;
            } else if !can_open || u > ONE_WAY_DOOR_HALF_U + radius
                || u < -ONE_WAY_DOOR_TRIGGER_DIST
            {
                view.state = OneWayDoorState::Closing;
                view.progress = 1.0 - open;
            } else {
                view.progress = (view.progress + dt / ONE_WAY_DOOR_OPEN_SECS).min(1.0);
            }
        }
        OneWayDoorState::Closing => {
            if open_zone || held {
                // Walked back in mid-swing: reopen from where the leaf is.
                view.state = OneWayDoorState::Opening;
                view.progress = open;
                if rt.open_cd <= 0.0 {
                    fx.gate_open(rt);
                }
            } else if view.progress + dt / ONE_WAY_DOOR_CLOSE_SECS >= 1.0 {
                view.state = OneWayDoorState::Closed;
                view.progress = 0.0;
            } else {
                view.progress += dt / ONE_WAY_DOOR_CLOSE_SECS;
            }
        }
        OneWayDoorState::Refused => {
            view.progress = (view.progress + dt / ONE_WAY_DOOR_REFUSED_SECS).min(1.0);
            if view.progress >= 1.0 {
                view.state = OneWayDoorState::Closed;
                view.progress = 0.0;
            }
        }
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

/// A unit facing vector, falling back to `+x` for a degenerate one (the art
/// modules' `safe_dir`, kept local so this file owns no art code).
#[inline]
fn safe_dir(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-4 {
        v.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// `(u, v)` of `p` in an object's local frame: `u` along `facing` (the passage
/// axis), `v` across it (the gap width).
#[inline]
fn local(p: Vec2, origin: Vec2, facing: Vec2) -> (f32, f32) {
    let d = p - origin;
    (d.dot(facing), d.dot(facing.perp()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_traps::{
        MazeRewardState, OneWayDoorState, RatHoleState, RAT_HOLE_SQUEEZE_SECS,
    };
    use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
    use crate::game::collision::{cardinal_vec, cell_of, TileKind};
    use rand::Rng;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{
        Level, LevelObjects, MazeSection, OneWayDoor, RatHole, Theme, TileRect,
    };
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    const COLS: i32 = 40;
    const ROWS: i32 = 30;

    /// A blank 40 × 30-cell arena with no walls and no objects.
    fn blank_world(theme: Theme) -> World {
        let mut w = World::new(theme, 0);
        let mut level = Level::empty(theme);
        level.tiles = TileMap::new(COLS, ROWS, theme, 0);
        level.arena = level.tiles.world_size();
        level.start = level.arena * 0.5;
        level.rooms.clear();
        level.mazes.clear();
        level.objects = LevelObjects::default();
        w.level = level;
        w.player.pos = w.level.start;
        w.player.vel = Vec2::ZERO;
        w.fog = Fog::with_mazes(COLS, ROWS, 0);
        w
    }

    /// One tick of the three object systems through the narrow harness
    /// (`begin_tick` → `run_system_cached` → `end_tick`).
    fn tick_dt(w: &mut World, dt: f32, move_dir: Vec2) {
        w.begin_tick(dt, move_dir, false);
        w.ecs
            .run_system_cached(passage_system)
            .expect("passage_system must run");
        w.ecs
            .run_system_cached(cache_system)
            .expect("cache_system must run");
        w.ecs
            .run_system_cached(door_system)
            .expect("door_system must run");
        w.end_tick();
    }

    fn tick(w: &mut World, move_dir: Vec2) {
        tick_dt(w, DT, move_dir);
    }

    /// A maze fixture: one one-way door cell at (20, 15) with `dir`, its reward
    /// three cells inside the maze, and the matching `MazeSection`.
    struct Fixture {
        world: World,
        gate: Vec2,
        facing: Vec2,
        reward: Vec2,
    }

    fn maze_fixture(dir: Cardinal) -> Fixture {
        let mut w = blank_world(Theme::Sewer);
        let cell = (20, 15);
        // The gate is the only opening in a solid wall column, exactly like a
        // maze exit: no enemy can walk around it.
        w.level
            .tiles
            .fill_cells(cell.0, 0, 1, ROWS, TileKind::Wall(WallKind::Solid));
        w.level
            .tiles
            .set(cell.0, cell.1, TileKind::Wall(WallKind::OneWayDoor { dir }));
        w.level.tiles.rebuild_masks();
        let facing = cardinal_vec(dir);
        let gate = TileMap::tile_rect(cell.0, cell.1).center();
        let reward = gate - facing * 48.0;
        w.level.objects.one_way_doors.push(OneWayDoor::new(cell, facing));
        w.level.objects.rewards.push(MazeRewardView::new(reward, facing));
        w.level.mazes.push(MazeSection {
            bounds: TileRect {
                x: cell.0 - 6,
                y: cell.1 - 6,
                w: 12,
                h: 12,
            },
            ring: TileRect {
                x: cell.0,
                y: 0,
                w: 1,
                h: ROWS,
            },
            entry: (cell.0 - 6, cell.1),
            fork: (cell.0 - 6, cell.1),
            exit: cell,
            exit_dir: dir,
            reward,
            reward_cell: cell_of(reward),
        });
        w.fog = Fog::with_mazes(COLS, ROWS, 1);
        Fixture {
            world: w,
            gate,
            facing,
            reward,
        }
    }

    fn door(fx: &Fixture) -> OneWayDoorView {
        fx.world.level.objects.one_way_doors[0].view
    }

    fn reward_state(fx: &Fixture) -> MazeRewardState {
        fx.world.level.objects.rewards[0].state
    }

    // ---- One-way doors ----------------------------------------------------

    #[test]
    fn the_door_opens_over_024s_and_is_held_open_while_the_rat_is_in_it() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        // Approaching through the maze side: the art's `u in [-10, 0]` trigger.
        fx.world.player.pos = gate - f * 9.0;
        tick(&mut fx.world, f);
        let d = door(&fx);
        assert_eq!(d.state, OneWayDoorState::Opening);
        assert_eq!(d.progress, 0.0, "the swing starts at once closed");
        // 0.24 s = 14.4 ticks at 60 Hz: 13 ticks are still mid-swing, 15 are open.
        for _ in 0..13 {
            tick(&mut fx.world, f);
        }
        let mid = fx.world.level.objects.one_way_doors[0].open_progress();
        assert!((0.85..1.0).contains(&mid), "13 ticks in, got {mid}");
        for _ in 0..2 {
            tick(&mut fx.world, f);
        }
        assert_eq!(door(&fx).state, OneWayDoorState::Opening);
        assert_eq!(
            fx.world.level.objects.one_way_doors[0].open_progress(),
            1.0
        );
        // The rat's body overlapping the doorway holds the leaf wide open.
        fx.world.player.pos = gate;
        for _ in 0..60 {
            tick(&mut fx.world, f);
        }
        assert_eq!(door(&fx).state, OneWayDoorState::Opening);
        assert_eq!(
            fx.world.level.objects.one_way_doors[0].open_progress(),
            1.0,
            "the leaf must stay held open while the rat is in the doorway"
        );
    }

    #[test]
    fn the_door_closes_over_035s_once_the_rat_clears_it() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        fx.world.player.pos = gate;
        for _ in 0..30 {
            tick(&mut fx.world, f);
        }
        assert_eq!(door(&fx).state, OneWayDoorState::Opening);
        // Cleared into `u > 8`: the leaf starts closing from full open.
        fx.world.player.pos = gate + f * 12.0;
        tick(&mut fx.world, f);
        let d = door(&fx);
        assert_eq!(d.state, OneWayDoorState::Closing);
        assert!(fx.world.level.objects.one_way_doors[0].open_progress() > 0.95);
        // 0.35 s = 21 ticks at 60 Hz.
        for _ in 0..20 {
            tick(&mut fx.world, f);
        }
        assert_eq!(door(&fx).state, OneWayDoorState::Closing, "20 ticks is 0.33 s");
        tick(&mut fx.world, f);
        assert_eq!(
            door(&fx).state,
            OneWayDoorState::Closed,
            "0.35 s must be enough to shut the leaf"
        );
        assert_eq!(
            fx.world.level.objects.one_way_doors[0].open_progress(),
            0.0
        );
    }

    /// How many times `s` was queued for this world (the tick's whole queue).
    fn sfx_count(w: &World, s: Sfx) -> usize {
        w.sfx.iter().filter(|x| **x == s).count()
    }

    /// Slide every live enemy by `d` between ticks (the crowd helper the
    /// squeeze and gate tests use).
    fn move_enemies(w: &mut World, d: Vec2) {
        let mut q = w.ecs.query::<&mut Enemy>();
        for mut e in q.iter_mut(&mut w.ecs) {
            e.pos += d;
        }
    }

    /// The leaf speaks once when it starts to swing, is silent while it swings,
    /// and refuses once per approach — never a per-tick rattle.
    #[test]
    fn the_leaf_speaks_once_per_edge() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        // Approach from the maze side: the leaf starts opening.
        fx.world.player.pos = gate - f * 9.0;
        fx.world.sfx.clear();
        tick(&mut fx.world, f);
        assert_eq!(door(&fx).state, OneWayDoorState::Opening);
        assert_eq!(sfx_count(&fx.world, Sfx::GateOpen), 1, "one open sound");
        assert_eq!(fx.world.ecs.resource::<GateRuntime>().doors[0].opens, 1);
        // The 0.24 s swing itself is silent.
        fx.world.sfx.clear();
        for _ in 0..12 {
            tick(&mut fx.world, f);
        }
        assert_eq!(sfx_count(&fx.world, Sfx::GateOpen), 0, "the swing is silent");
        assert_eq!(door(&fx).state, OneWayDoorState::Opening);

        // Push the backstop from the wrong side: one refusal per approach.
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        fx.world.player.pos = gate + f * 8.0;
        fx.world.sfx.clear();
        tick(&mut fx.world, -f);
        assert_eq!(door(&fx).state, OneWayDoorState::Refused);
        assert_eq!(sfx_count(&fx.world, Sfx::GateRefuse), 1);
        // Leaning on it for half a second never retriggers.
        fx.world.sfx.clear();
        for _ in 0..30 {
            tick(&mut fx.world, -f);
        }
        assert_eq!(sfx_count(&fx.world, Sfx::GateRefuse), 0, "no per-tick rattle");
        // Step away and come back: a new approach refuses again.
        fx.world.player.pos = gate + f * 24.0;
        tick(&mut fx.world, -f);
        fx.world.sfx.clear();
        fx.world.player.pos = gate + f * 8.0;
        tick(&mut fx.world, -f);
        assert_eq!(sfx_count(&fx.world, Sfx::GateRefuse), 1, "a fresh approach");
        assert_eq!(sfx_count(&fx.world, Sfx::UiSelect), 0, "no menu stand-in");
    }

    #[test]
    fn a_wrong_side_approach_is_refused_once_per_approach() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        // Pushing into the backstop from the `+facing` side.
        fx.world.player.pos = gate + f * 8.0;
        tick(&mut fx.world, -f);
        assert_eq!(door(&fx).state, OneWayDoorState::Refused);
        assert!(door(&fx).hit_flash > 0.9);
        // 0.40 s = 24 ticks, then back to Closed — while still pressing.
        for _ in 0..24 {
            tick(&mut fx.world, -f);
        }
        assert_eq!(door(&fx).state, OneWayDoorState::Closed);
        for _ in 0..30 {
            tick(&mut fx.world, -f);
        }
        let rt = fx.world.ecs.resource::<GateRuntime>();
        assert_eq!(
            rt.doors[0].refusals, 1,
            "leaning on the backstop must not retrigger the flash every tick"
        );
        // Step away and come back: a new approach refuses again.
        fx.world.player.pos = gate + f * 24.0;
        tick(&mut fx.world, -f);
        fx.world.player.pos = gate + f * 8.0;
        tick(&mut fx.world, -f);
        assert_eq!(door(&fx).state, OneWayDoorState::Refused);
        assert_eq!(fx.world.ecs.resource::<GateRuntime>().doors[0].refusals, 2);
    }

    /// The rat's real resting spot against the shut leaf from the wrong side
    /// is `u = 2 (half the leaf collider) + PLAYER_RADIUS = 5.0`, i.e. exactly
    /// `ONE_WAY_DOOR_HALF_U`, give or take float noise. That spot must refuse,
    /// never open the leaf or arm the slam.
    #[test]
    fn the_backstop_resting_spot_refuses_and_never_opens_or_arms() {
        for nudge in [-1e-4_f32, 0.0, 1e-4] {
            let mut fx = maze_fixture(Cardinal::East);
            let (gate, f) = (fx.gate, fx.facing);
            fx.world.player.pos = gate + f * (ONE_WAY_DOOR_HALF_U + nudge);
            fx.world.sfx.clear();
            tick(&mut fx.world, -f);
            assert_eq!(
                door(&fx).state,
                OneWayDoorState::Refused,
                "nudge {nudge}: leaning on the backstop must refuse"
            );
            for _ in 0..60 {
                tick(&mut fx.world, -f);
            }
            let rt = fx.world.ecs.resource::<GateRuntime>();
            assert_eq!(rt.doors[0].opens, 0, "nudge {nudge}: the leaf never opens");
            assert!(!rt.doors[0].armed, "nudge {nudge}: never armed from outside");
            assert_eq!(rt.doors[0].refusals, 1, "nudge {nudge}");
        }
    }

    /// The horde can never pass a gate: collision resolves enemies with
    /// `player_vel: None`, so the leaf is a wall from both sides forever.
    #[test]
    fn enemies_never_pass_a_one_way_door() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        // A brute inside the maze keeps shoving east, straight at the gate.
        fx.world
            .spawn_enemy(EnemyKind::Brute, gate - f * 14.0, 1.0);
        for step in 0..90 {
            {
                let mut q = fx.world.ecs.query::<&mut Enemy>();
                for mut e in q.iter_mut(&mut fx.world.ecs) {
                    e.pos += f * 1.5;
                }
            }
            fx.world.begin_tick(DT, Vec2::ZERO, false);
            fx.world
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::grid_build_system)
                .expect("grid_build_system must run");
            fx.world
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::wall_resolve_system)
                .expect("wall_resolve_system must run");
            fx.world.end_tick();
            let e = &fx.world.enemies()[0];
            let u = (e.pos - gate).dot(f);
            assert!(
                u < -1.0,
                "the horde must never cross a one-way gate (u = {u} at step {step})"
            );
        }
    }

    // ---- The cache run as a heist ----------------------------------------

    #[test]
    fn the_cache_awards_exactly_one_upgrade_once_and_never_again() {
        let mut fx = maze_fixture(Cardinal::East);
        let reward = fx.reward;
        fx.world.player.pos = reward;
        let p0 = fx.world.particles.len();
        tick(&mut fx.world, Vec2::ZERO);
        assert!(
            fx.world.particles.len() > p0,
            "the claim must burst on the pedestal"
        );
        assert_eq!(fx.world.pending_levelups, 1, "one free level-up");
        assert_eq!(reward_state(&fx), MazeRewardState::Opening);
        assert!(fx.world.fog.maze_claimed(0), "the minimap must mark it");
        assert!(
            fx.world
                .ecs
                .resource::<GateRuntime>()
                .feed()
                .contains(&MazeEvent::CacheClaimed { index: 0 })
        );
        // The cache's own sting — not the level-up one (the upgrade card the
        // claim opens plays that), and exactly once.
        assert_eq!(
            fx.world.sfx.iter().filter(|s| **s == Sfx::CacheOpen).count(),
            1,
            "one cache sting: {:?}",
            fx.world.sfx
        );
        assert!(fx.world.sfx.iter().all(|s| *s != Sfx::LevelUp));
        // Walking away and back must never award it again.
        for _ in 0..10 {
            fx.world.player.pos = reward + Vec2::new(60.0, 0.0);
            tick(&mut fx.world, Vec2::ZERO);
            fx.world.player.pos = reward;
            tick(&mut fx.world, Vec2::ZERO);
        }
        assert_eq!(fx.world.pending_levelups, 1);
        // `Opening` (0.65 s) then the permanent `Opened` landmark.
        for _ in 0..40 {
            tick(&mut fx.world, Vec2::ZERO);
        }
        assert_eq!(reward_state(&fx), MazeRewardState::Opened);
    }

    #[test]
    fn the_cache_grants_08s_of_grace() {
        let mut fx = maze_fixture(Cardinal::East);
        fx.world.player.pos = fx.reward;
        tick(&mut fx.world, Vec2::ZERO);
        assert!(
            fx.world.player.iframes >= CACHE_GRACE_SECS - 1e-6,
            "the claim must arm the grace window, got {}",
            fx.world.player.iframes
        );
        // 0.4 s later the rat is still immune.
        for _ in 0..24 {
            fx.world.begin_tick(DT, Vec2::ZERO, false);
            fx.world
                .ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .expect("player_system must run");
            fx.world.end_tick();
        }
        let hp = fx.world.player.hp;
        assert!(
            !fx.world.player.take_damage(10.0),
            "0.4 s into a 0.8 s grace the rat must still be immune"
        );
        assert_eq!(fx.world.player.hp, hp);
        // Past 0.8 s the window is spent and a hit lands.
        for _ in 0..26 {
            fx.world.begin_tick(DT, Vec2::ZERO, false);
            fx.world
                .ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .expect("player_system must run");
            fx.world.end_tick();
        }
        assert_eq!(fx.world.player.iframes, 0.0);
        assert!(fx.world.player.take_damage(10.0));
    }

    #[test]
    fn the_gate_slams_behind_the_rat_after_the_cache_run() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        // Loot the cache, then leave through the one-way exit.
        fx.world.player.pos = fx.reward;
        tick(&mut fx.world, Vec2::ZERO);
        // A rat that never entered the maze and is merely outside the exit
        // (beyond the clearing window) must not trip the slam.
        fx.world.player.pos = gate + f * 40.0;
        tick(&mut fx.world, f);
        assert!(!fx.world.ecs.resource::<GateRuntime>().doors[0].slammed);
        assert_eq!(door(&fx).state, OneWayDoorState::Closed);
        fx.world.player.pos = gate - f * 9.0;
        tick(&mut fx.world, f);
        fx.world.player.pos = gate;
        for _ in 0..20 {
            tick(&mut fx.world, f);
        }
        assert_eq!(door(&fx).state, OneWayDoorState::Opening);
        // The rat's body clears the leaf: the gate slams in one step.
        fx.world.player.pos = gate + f * 9.0;
        fx.world.sfx.clear();
        let p0 = fx.world.particles.len();
        tick(&mut fx.world, f);
        assert_eq!(door(&fx).state, OneWayDoorState::Closed);
        assert!(
            fx.world.particles.len() > p0,
            "the slam must kick dust along the threshold"
        );
        assert_eq!(fx.world.level.objects.one_way_doors[0].open_progress(), 0.0);
        assert!(door(&fx).hit_flash > 0.9);
        assert_eq!(
            fx.world.sfx.iter().filter(|s| **s == Sfx::GateSlam).count(),
            1,
            "exactly one latch sound"
        );
        assert!(
            fx.world.sfx.iter().all(|s| *s != Sfx::UiSelect),
            "the latch must not reuse a menu sound: {:?}",
            fx.world.sfx
        );
        assert!(fx
            .world
            .ecs
            .resource::<GateRuntime>()
            .feed()
            .contains(&MazeEvent::GateSlammed { index: 0 }));
        assert!(fx.world.ecs.resource::<GateRuntime>().doors[0].slammed);
        assert!(fx.world.ecs.resource::<GateRuntime>().doors[0].armed);
        // A second crossing never latches again.
        fx.world.player.pos = gate - f * 9.0;
        tick(&mut fx.world, f);
        fx.world.player.pos = gate + f * 9.0;
        fx.world.sfx.clear();
        tick(&mut fx.world, f);
        assert!(fx.world.sfx.iter().all(|s| *s != Sfx::GateSlam));
        // The pack that followed is stuck behind the shut leaf.
        fx.world.spawn_enemy(EnemyKind::Brute, gate - f * 14.0, 1.0);
        for step in 0..60 {
            {
                let mut q = fx.world.ecs.query::<&mut Enemy>();
                for mut e in q.iter_mut(&mut fx.world.ecs) {
                    e.pos += f * 1.5;
                }
            }
            fx.world.begin_tick(DT, Vec2::ZERO, false);
            fx.world
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::grid_build_system)
                .expect("grid_build_system must run");
            fx.world
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::wall_resolve_system)
                .expect("wall_resolve_system must run");
            fx.world.end_tick();
            let e = &fx.world.enemies()[0];
            let u = (e.pos - gate).dot(f);
            assert!(
                u < -1.0,
                "nothing may follow through the slam (u = {u}, step {step})"
            );
        }
    }

    /// A dash on a long frame can carry the rat from before the open zone to
    /// beyond the clearing window in one tick. The crossing must still arm the
    /// leaf and slam it, on that tick.
    #[test]
    fn a_crossing_faster_than_the_windows_still_arms_and_slams() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        fx.world.player.pos = fx.reward;
        tick(&mut fx.world, Vec2::ZERO);
        assert!(fx.world.ecs.resource::<GateRuntime>().rewards[0].claimed);
        // Short of the open zone (`u = -12`): seen in the lane, not armed.
        fx.world.player.pos = gate - f * 12.0;
        tick(&mut fx.world, f);
        assert!(!fx.world.ecs.resource::<GateRuntime>().doors[0].armed);
        // One tick later the body is past the clearing window (`u = 20`).
        fx.world.player.pos = gate + f * 20.0;
        fx.world.sfx.clear();
        tick(&mut fx.world, f);
        let rt = fx.world.ecs.resource::<GateRuntime>();
        assert!(rt.doors[0].armed, "the crossing itself arms the leaf");
        assert!(rt.doors[0].slammed, "and slams it on the same tick");
        assert_eq!(sfx_count(&fx.world, Sfx::GateSlam), 1);
        assert_eq!(door(&fx).state, OneWayDoorState::Closed);
    }

    /// The crossing rule must not turn the outside of the maze into a trigger:
    /// a rat walking away from the backstop, or along the outside lane, never
    /// crossed anything.
    #[test]
    fn walking_the_outside_of_a_claimed_maze_never_slams() {
        let mut fx = maze_fixture(Cardinal::East);
        let (gate, f) = (fx.gate, fx.facing);
        fx.world.player.pos = fx.reward;
        tick(&mut fx.world, Vec2::ZERO);
        // Leaning on the backstop, then walking straight out through the
        // clearing window and beyond.
        for u in [5.0_f32, 6.0, 9.0, 14.0, 19.0, 30.0] {
            fx.world.player.pos = gate + f * u;
            tick(&mut fx.world, f);
        }
        // And a placed rat (reward -> far outside) is not a walk either.
        fx.world.player.pos = fx.reward;
        tick(&mut fx.world, Vec2::ZERO);
        fx.world.player.pos = gate + f * 40.0;
        tick(&mut fx.world, f);
        let rt = fx.world.ecs.resource::<GateRuntime>();
        assert!(!rt.doors[0].armed);
        assert!(!rt.doors[0].slammed);
    }

    // ---- Rat-holes --------------------------------------------------------

    /// A vertical wall cell carrying a rat-hole whose passage runs east–west
    /// (`facing = +x`), matching `levelgen::objects_from`.
    fn hole_world() -> (World, (i32, i32), Vec2) {
        let mut w = blank_world(Theme::Sewer);
        let cell = (20, 15);
        w.level.tiles.set(
            cell.0,
            cell.1,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        w.level.tiles.rebuild_masks();
        let pos = TileMap::tile_rect(cell.0, cell.1).center();
        w.level.objects.rat_holes.push(RatHole {
            cell,
            view: RatHoleView::new(pos, Vec2::new(1.0, 0.0)),
        });
        (w, cell, pos)
    }

    #[test]
    fn the_squeeze_plays_for_025s_and_never_moves_the_rat() {
        let (mut w, cell, pos) = hole_world();
        w.player.pos = pos;
        tick(&mut w, Vec2::ZERO);
        assert_eq!(w.level.objects.rat_holes[0].view.state, RatHoleState::Squeezing);
        // Held: the rat stands in the tunnel while the crowd does not move it.
        for _ in 0..20 {
            tick(&mut w, Vec2::ZERO);
            assert_eq!(
                w.player.pos.x.to_bits(),
                pos.x.to_bits(),
                "the squeeze must never move the rat"
            );
            assert_eq!(w.player.pos.y.to_bits(), pos.y.to_bits());
        }
        assert!(
            w.level.objects.rat_holes[0].view.progress >= 1.0 - 1e-4,
            "the 0.25 s squeeze must have played out"
        );
        assert!(w.fog.hole_crossed(cell), "the fog must hear about it");
        // A small swarmer in the tunnel squeezes it too.
        w.player.pos = pos + Vec2::new(80.0, 0.0);
        tick(&mut w, Vec2::ZERO);
        assert_eq!(w.level.objects.rat_holes[0].view.state, RatHoleState::Idle);
        w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
        tick(&mut w, Vec2::ZERO);
        assert_eq!(
            w.level.objects.rat_holes[0].view.state,
            RatHoleState::Squeezing,
            "a Skitterer uses the hole too"
        );
    }

    /// The squeeze puffs both tunnel mouths (and speaks) when the rat or a
    /// small body *enters* the tunnel — and a stream of Skitterers through one
    /// hole is rate-limited to one sound per 0.4 s.
    #[test]
    fn the_squeeze_puffs_both_mouths_once_per_body_and_rate_limits_traffic() {
        let (mut w, _cell, pos) = hole_world();
        // The rat's own squeeze: one puff and one sound, always.
        w.player.pos = pos;
        w.sfx.clear();
        let p0 = w.particles.len();
        tick(&mut w, Vec2::ZERO);
        assert_eq!(w.level.objects.rat_holes[0].view.state, RatHoleState::Squeezing);
        assert_eq!(sfx_count(&w, Sfx::HoleSqueeze), 1, "the rat squeezes once");
        assert!(w.particles.len() > p0, "the squeeze must puff both mouths");
        // Standing in the tunnel is silent (the sound is on entry).
        w.sfx.clear();
        tick(&mut w, Vec2::ZERO);
        assert_eq!(sfx_count(&w, Sfx::HoleSqueeze), 0);

        // The rat leaves; let the hole's anti-machine-gun window drain.
        w.player.pos = pos + Vec2::new(80.0, 0.0);
        for _ in 0..26 {
            tick(&mut w, Vec2::ZERO);
        }
        assert_eq!(
            w.ecs.resource::<GateRuntime>().holes[0].squeeze_cd,
            0.0,
            "the rate limit must expire on its own"
        );

        // Three Skitterers arrive together: one puff for the lot.
        for _ in 0..3 {
            w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
        }
        w.sfx.clear();
        tick(&mut w, Vec2::ZERO);
        assert_eq!(
            sfx_count(&w, Sfx::HoleSqueeze),
            1,
            "three bodies at once is still one squeeze"
        );

        // They clear the tunnel and a new body slides in well inside the
        // 0.4 s window: silent, or the tunnel becomes a machine gun.
        move_enemies(&mut w, Vec2::new(0.0, 40.0));
        tick(&mut w, Vec2::ZERO);
        w.sfx.clear();
        let p1 = w.particles.len();
        move_enemies(&mut w, Vec2::new(0.0, -40.0));
        tick(&mut w, Vec2::ZERO);
        assert_eq!(
            sfx_count(&w, Sfx::HoleSqueeze),
            0,
            "a stream of Skitterers must not machine-gun one hole"
        );
        assert_eq!(
            w.particles.len(),
            p1,
            "a suppressed squeeze spends no particle budget either"
        );
        // The *state* still plays for the body that is in the tunnel.
        assert_eq!(w.level.objects.rat_holes[0].view.state, RatHoleState::Squeezing);

        // Once the window is clear, the next body speaks again.
        move_enemies(&mut w, Vec2::new(0.0, 40.0));
        for _ in 0..25 {
            tick(&mut w, Vec2::ZERO);
        }
        w.sfx.clear();
        move_enemies(&mut w, Vec2::new(0.0, -40.0));
        tick(&mut w, Vec2::ZERO);
        assert_eq!(
            sfx_count(&w, Sfx::HoleSqueeze),
            1,
            "past {HOLE_SQUEEZE_SOUND_SECS} s the hole speaks again"
        );
        assert_eq!(
            w.ecs.resource::<GateRuntime>().holes[0].squeeze_sounds,
            3,
            "the rat, the first crowd and the late body"
        );
    }

    #[test]
    fn a_large_enemy_triggers_blocked_once_per_approach() {
        let (mut w, _cell, pos) = hole_world();
        // The rat is just outside the hole; a brute presses the cheeks.
        w.player.pos = pos + Vec2::new(40.0, 0.0);
        w.spawn_enemy(EnemyKind::Brute, pos + Vec2::new(10.0, 5.0), 1.0);
        for _ in 0..10 {
            tick(&mut w, Vec2::ZERO);
        }
        assert_eq!(w.level.objects.rat_holes[0].view.state, RatHoleState::Blocked);
        let blocks = w.ecs.resource::<GateRuntime>().holes[0].blocks;
        assert_eq!(blocks, 1, "one reaction per approach");
        // A packed crowd keeps shoving: still exactly one.
        w.spawn_enemy(EnemyKind::Cat, pos + Vec2::new(-9.0, -5.0), 1.0);
        w.spawn_enemy(EnemyKind::Cat, pos + Vec2::new(9.0, 4.0), 1.0);
        for _ in 0..60 {
            tick(&mut w, Vec2::ZERO);
        }
        assert_eq!(w.ecs.resource::<GateRuntime>().holes[0].blocks, 1);
    }

    #[test]
    fn hole_centring_pulls_the_rat_into_the_gap_without_moving_it_along_the_wall() {
        let (mut w, _cell, pos) = hole_world();
        let f = Vec2::new(1.0, 0.0);
        w.player.pos = pos + f * 12.0 + f.perp() * 3.0;
        let before = w.player.pos;
        tick(&mut w, -f);
        let (u0, v0) = local(before, pos, f);
        let (u1, v1) = local(w.player.pos, pos, f);
        assert!(v1.abs() < v0.abs(), "the rat must be centred, {v0} -> {v1}");
        assert!((u1 - u0).abs() < 1e-4, "centring must only move laterally");
        assert!(w
            .level
            .tiles
            .circle_clear(w.player.pos, PLAYER_RADIUS, SizeClass::Small));
        // Six more ticks take a 3 px offset inside the 1 px tolerance.
        for _ in 0..10 {
            tick(&mut w, -f);
            assert!(
                w.level
                    .tiles
                    .circle_clear(w.player.pos, PLAYER_RADIUS, SizeClass::Small),
                "centring must never push the rat into a pier at {:?}",
                w.player.pos
            );
        }
        assert!(local(w.player.pos, pos, f).1.abs() <= 1.0);
        // Not aiming through the hole: no help at all.
        w.player.pos = before;
        tick(&mut w, Vec2::new(0.0, 1.0));
        assert_eq!(w.player.pos.x.to_bits(), before.x.to_bits());
    }

    /// Every generated hole, both approaches, offsets inside the gap: the
    /// centring may never leave the rat inside a wall and may never widen its
    /// lateral offset.
    #[test]
    fn hole_centring_never_pushes_the_rat_into_a_wall() {
        for theme in Theme::all() {
            for seed in 0..6u64 {
                let mut w = World::new(theme, seed);
                let holes: Vec<RatHoleView> = w
                    .level
                    .objects
                    .rat_holes
                    .iter()
                    .map(|h| h.view)
                    .collect();
                for view in holes {
                    let f = safe_dir(view.facing);
                    for side in [-1.0f32, 1.0] {
                        for lateral in [-3.0f32, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0] {
                            let start = view.pos + f * (side * 12.0) + f.perp() * lateral;
                            if !w
                                .level
                                .tiles
                                .circle_clear(start, PLAYER_RADIUS, SizeClass::Small)
                            {
                                continue;
                            }
                            w.player.pos = start;
                            let intent = f * -side;
                            let (u0, v0) = local(start, view.pos, f);
                            w.begin_tick(DT, intent, false);
                            w.ecs.run_system_cached(passage_system).unwrap();
                            w.end_tick();
                            let (u1, v1) = local(w.player.pos, view.pos, f);
                            assert!(
                                w.level
                                    .tiles
                                    .circle_clear(w.player.pos, PLAYER_RADIUS, SizeClass::Small),
                                "{theme:?} seed {seed}: centring put the rat in a wall at {:?}",
                                w.player.pos
                            );
                            assert!(
                                v1.abs() <= v0.abs() + 1e-4,
                                "{theme:?} seed {seed}: centring widened the offset {v0} -> {v1}"
                            );
                            assert!((u1 - u0).abs() < 0.5, "lateral only");
                        }
                    }
                }
            }
        }
    }

    // ---- Determinism ------------------------------------------------------

    /// Same seed, same script, same object state — down to the bits, including
    /// the gate runtime's latches and the HUD feed.
    #[test]
    fn the_object_runtime_is_deterministic() {
        fn script(w: &mut World) {
            for step in 0..900u64 {
                let phase = step as f32 * 0.021;
                let dir = Vec2::new(crate::detmath::sin(phase), crate::detmath::cos(phase * 0.7));
                w.update(DT, dir, step % 211 == 0);
            }
        }
        for theme in [Theme::Sewer, Theme::Lab] {
            // A map that actually has a maze (mazes are rare by design), so the
            // door latch, the slam and the cache claim are all exercised.
            let seed = (0..400u64)
                .find(|seed| !Level::generate(theme, *seed).mazes.is_empty())
                .unwrap_or_else(|| panic!("no {theme:?} map in 400 seeds has a maze"));
            let mut a = World::new(theme, seed);
            let mut b = World::new(theme, seed);
            a.viewport = Vec2::new(160.0, 168.0);
            b.viewport = a.viewport;
            script(&mut a);
            script(&mut b);
            assert_eq!(
                a.level.objects.one_way_doors.len(),
                b.level.objects.one_way_doors.len()
            );
            for (x, y) in a
                .level
                .objects
                .one_way_doors
                .iter()
                .zip(&b.level.objects.one_way_doors)
            {
                assert_eq!(x.view.state, y.view.state);
                assert_eq!(x.view.progress.to_bits(), y.view.progress.to_bits());
                assert_eq!(x.view.hit_flash.to_bits(), y.view.hit_flash.to_bits());
            }
            for (x, y) in a
                .level
                .objects
                .rat_holes
                .iter()
                .zip(&b.level.objects.rat_holes)
            {
                assert_eq!(x.view.state, y.view.state);
                assert_eq!(x.view.progress.to_bits(), y.view.progress.to_bits());
            }
            for (x, y) in a
                .level
                .objects
                .rewards
                .iter()
                .zip(&b.level.objects.rewards)
            {
                assert_eq!(x.state, y.state);
                assert_eq!(x.progress.to_bits(), y.progress.to_bits());
            }
            for i in 0..a.level.mazes.len() {
                assert_eq!(a.fog.maze_claimed(i), b.fog.maze_claimed(i));
            }
            assert_eq!(a.player.pos.x.to_bits(), b.player.pos.x.to_bits());
            assert_eq!(a.player.pos.y.to_bits(), b.player.pos.y.to_bits());
            assert_eq!(a.player.iframes.to_bits(), b.player.iframes.to_bits());
            assert_eq!(a.pending_levelups, b.pending_levelups);
            let (ra, rb) = (
                a.ecs.resource::<GateRuntime>(),
                b.ecs.resource::<GateRuntime>(),
            );
            // The three systems really are in the schedule: one full tick
            // sized every per-object buffer from the level.
            assert_eq!(ra.doors.len(), a.level.objects.one_way_doors.len());
            assert_eq!(ra.holes.len(), a.level.objects.rat_holes.len());
            assert_eq!(ra.rewards.len(), a.level.objects.rewards.len());
            assert!(
                !ra.doors.is_empty(),
                "{theme:?} seed {seed}: the runtime must have been sized by a tick"
            );
            assert_eq!(ra.feed(), rb.feed());
            for (x, y) in ra.doors.iter().zip(&rb.doors) {
                assert_eq!(x.refused_latched, y.refused_latched);
                assert_eq!(x.slammed, y.slammed);
                assert_eq!(x.armed, y.armed);
                assert_eq!(x.refusals, y.refusals);
                assert_eq!(x.opens, y.opens);
                assert_eq!(x.open_cd.to_bits(), y.open_cd.to_bits());
            }
            for (x, y) in ra.holes.iter().zip(&rb.holes) {
                assert_eq!(x.inside, y.inside);
                assert_eq!(x.squeeze.to_bits(), y.squeeze.to_bits());
                assert_eq!(x.blocks, y.blocks);
                assert_eq!(x.squeezes, y.squeezes);
                assert_eq!(x.squeeze_sounds, y.squeeze_sounds);
                assert_eq!(x.squeeze_cd.to_bits(), y.squeeze_cd.to_bits());
            }
            assert_eq!(a.rng.next_u64(), b.rng.next_u64());
        }
    }

    /// The squeeze/blocked windows are the art's constants, not magic numbers.
    #[test]
    fn rat_hole_timings_match_the_art_spec() {
        assert_eq!(RAT_HOLE_SQUEEZE_SECS, 0.25);
        assert_eq!(ONE_WAY_DOOR_OPEN_SECS, 0.24);
        assert_eq!(ONE_WAY_DOOR_CLOSE_SECS, 0.35);
        assert_eq!(ONE_WAY_DOOR_REFUSED_SECS, 0.40);
        assert_eq!(MAZE_REWARD_OPEN_SECS, 0.65);
    }
}

