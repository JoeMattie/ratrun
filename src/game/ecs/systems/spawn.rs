//! (b) Spawn director, spawn ring, flanking groups, elite spawns, enemy cap.
//!
//! ## Where the horde may arrive (content wave)
//!
//! * **Never inside a warren.** A maze's ring and interior are excluded from
//!   every spawn path — ring pulses, room waves, flanking groups, the elite
//!   and the fallback point. The chase begins at the mouth: existing enemies
//!   pursue through the entrance, the director never introduces one there.
//! * **Skip, never shove.** When no safe off-screen point exists (the rat is
//!   deep in a maze, or boxed in a corner) the director skips that spawn
//!   instead of materialising a body in a warren or on top of the player.
//! * **Two off-screen flanking groups** when the rat has held one chamber for
//!   [`director::FLANK_DWELL_SECS`] and the chamber has two real approaches:
//!   the pulse's own enemies are reused, split between the two doorways, so a
//!   camper is pinched instead of fed from the ring one at a time.
//! * **A room wave is short-leashed**: the staging room is within
//!   [`ROOM_WAVE_RANGE`] and the approach from the player is at most
//!   [`APPROACH_CELLS`] path cells, so a wave arrives instead of queueing
//!   behind a wall on the far side of the map.
//!
//! ## Where "out of sight" is measured (batch ten)
//!
//! The zoom ladder shows up to **twice** the world in each direction, so the
//! simulation's "out of sight" is the **widest** view ([`widest_view`]): the
//! simulation viewport times [`MAX_ZOOM_OUT`], a plain number mirroring
//! `render::zoom::MAX` so the simulation depends on no render state. The ring
//! ([`ring_radius`]), the off-screen test behind it ([`off_screen`]), the flank
//! anchors and their outward walk, the room-wave acceptance, the room escape
//! and the miniboss arrival all read that one rectangle, and none of them reads
//! the current zoom: the same seed still plays out identically at every rung.
//! Bodies therefore walk further in than they used to, so a body still outside
//! the **unzoomed** (1.0) view carries an approach speed boost
//! ([`boost_approach`]) until it comes into it — no rng, no position change.
//! Measured against the old ring, first contact on a standing rat is 4.2 s
//! before and 4.1 s after (160 × 80).
//!
//! ## The boss
//!
//! The director no longer spawns one. At `director::BOSS_TIME` it raises the
//! `BossDue` resource ([`director::raise_boss_due`]) and the boss framework
//! spawns the level's boss **out of sight, using this file's own rule**
//! ([`spawn_point_reachable`], on the boss's private rng stream) so the boss
//! appears no more suddenly than a straggler; the shake, the arrival VFX and
//! `Sfx::Boss` belong to that arrival, not to the ring. Its fight is a
//! rectangle centred on the rat at the moment it arrives (batch 12 removed the
//! arena), never a room this stage has to keep clear.
//!
//! ## Port notes
//!
//! Reproduce **verbatim**:
//! - `World::run_director`.
//! - `World::spawn_point`.
//! - `World::spawn_point_ok`.
//! - `Enemy::spawn` (`src/game/enemy.rs`) is the only constructor; it draws
//!   `fire_cd`, `wander` and `anim` from the rng in that order.
//!
//! Resources: `Res<TickInput>` (dt), `Res<RunCounters>` (elapsed),
//! `Res<Viewport>` (spawn ring radius), `Res<Level>`, `Res<Player>`,
//! `ResMut<Director>`, `ResMut<RngRes>`, `ResMut<NextSpawnId>`,
//! `ResMut<Shake>` + `ResMut<SfxQueue>` (boss shake/sfx), `Query<&Enemy>` for
//! the live count, `Commands` to spawn.
//!
//! Ordering: after `player_system` + `flow_refresh_system`, before enemy
//! steering (the legacy `update()` order). `.chain()` inserts an
//! `ApplyDeferred` after this system, so enemies spawned here steer on the same
//! tick, exactly like the legacy `self.enemies.push` inside `run_director`.
//!
//! **Enemy cap:** `ENEMY_CAP` is 600 and one slot is reserved until
//! `director.boss_spawned`. Because `Commands` defers spawns until the end of
//! the system, count the query **and** add your own running count for each
//! queued spawn; simply re-querying `enemies.len()` inside the pulse loop would
//! over-spawn.
//!
//! Every spawned enemy is `commands.spawn((enemy, next_id.take()))` so it gets
//! a monotonic `SpawnId` (see `components::SpawnId`). Spawn order inside a
//! pulse is the legacy loop order: one `pick_kind` draw, one `spawn_point`
//! (which draws from the same rng), then `Enemy::spawn`.
//!
//! The only structural change from the port is the cap bookkeeping above: legacy asked
//! `self.enemies.len()` after each push (the `Vec` grew immediately), the ECS
//! port asks the query once and adds `queued`. Rng draws, their order and the
//! spawn order are unchanged — `spawn_point` draws its ring angle before
//! `Enemy::spawn` draws its three fields for every single spawn, and the elite
//! and boss branches keep consuming the same draws. `shake = 14.0` is an
//! assignment (not a `max`) exactly like the legacy code.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use rand::rngs::StdRng;
use rand::RngExt;

use crate::detmath::DetF32;
use crate::game::collision::{SizeClass, TileMap};
use crate::game::director::{self, Director};
use crate::game::ecs::constants::ENEMY_CAP;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{
    BossDue, Cvars, DebugFlags, DifficultyLevel, DrakeSpawner, LevelIndex, NextEnemyId, NextSpawnId,
    RngRes, RunCounters, SfxQueue, TickInput, Viewport,
};
use crate::game::ecs::systems::approach::in_sim_view;
use crate::game::ecs::systems::miniboss::MinibossSchedule;
use crate::game::ecs::systems::miniboss_behaviour::MinibossSummon;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{Level, MazeSection, Room, RoomKind, Theme, TileRect};
use crate::game::player::Player;
use crate::game::roster;
use crate::math::Vec2;

// ---- Spawn ring tuning -----------------------------------------------------

/// Ring samples taken before the fallbacks (plus one mirrored sweep).
const RING_ATTEMPTS: i32 = 12;
/// Floor of the ring radius: spawns must never appear on screen.
const MIN_RING_RADIUS: f32 = 120.0;

/// The widest rung of the render-only zoom ladder, as a **plain number**.
///
/// The simulation must never depend on render state ([`crate::render::zoom`]
/// owns the real ladder), so the ladder's top is mirrored here;
/// `the_spawn_view_is_the_widest_rung_of_the_zoom_ladder` pins the two
/// together. At this zoom the screen shows `viewport × 2` of world per axis.
pub const MAX_ZOOM_OUT: f32 = 2.0;

/// Slack between the widest view's half-diagonal and the spawn ring.
const RING_MARGIN: f32 = 16.0;

/// Approach speed multiplier for a **ground** body that is still outside the
/// unzoomed (1.0) view (see [`boost_approach`]).
///
/// The ring moved out to the widest view's half-diagonal, so a body has further
/// to walk before it can be fought: this gives the walk-in back, and it is
/// taken off again the moment the body comes into the 1.0 view, so nothing on
/// the unzoomed screen moves differently from before. (Zoomed out, the band
/// between the two views shows the body hurrying in.) Tuned on the integration
/// pass: with the boost held only outside the *widest* view first contact on a
/// standing rat went from 4.2 s to 5.4 s (the ring is barely outside that
/// view on the horizontal bearings, so there was nothing to boost across);
/// held to the 1.0 view, ×1.8 gives 4.4 s, ×2.0 4.1 s and ×2.5 3.5 s.
pub const APPROACH_BOOST_GROUND: f32 = 2.0;
/// The same for a flying body, which is already quick: ×1.3.
pub const APPROACH_BOOST_FLYER: f32 = 1.3;
/// A body whose own speed is at or above this (px/s) is never boosted: the
/// centipede and the hornet already cross the ring faster than the rat can
/// walk, and the point of the boost is the slow opening fodder.
pub const APPROACH_SPEED_CEIL: f32 = 40.0;

/// The world-pixel view the **widest zoom-out** reveals around the rat: the
/// simulation viewport scaled by [`MAX_ZOOM_OUT`].
///
/// Every "out of sight" distance in the simulation is measured against *this*
/// rectangle — the spawn ring ([`ring_radius`]), the off-screen preference
/// behind it ([`off_screen`]), the flank anchors, the room-wave and fallback
/// preferences, the miniboss arrival (which shares
/// [`spawn_point_reachable`]) and the straggler approach boost — and never
/// against the current zoom. That is what makes a spawn invisible on every
/// rung of the ladder (the owner saw bodies pop in at `2.0`) while the run
/// still simulates identically at all of them
/// (`world::zoom_tests::the_same_seed_plays_out_identically_at_every_zoom`).
#[inline]
pub fn widest_view(viewport: Vec2) -> Vec2 {
    viewport * MAX_ZOOM_OUT
}
/// Keep spawns this far inside the arena border.
const SPAWN_MARGIN: f32 = 8.0;
/// Never spawn closer than this to the player.
const SPAWN_MIN_DIST: f32 = 90.0;
/// The last-resort distance when a map is too tight for any ring point: better
/// a spawn at the edge of the rat's own body than one inside a wall.
const NEAR_SPAWN_DIST: f32 = 24.0;
/// Wall clearance every ring spawn keeps (the horde is the `Large` class).
const SPAWN_WALL_CLEAR: f32 = 4.0;
/// Angular spread of the "player is heading this way" bias, in radians.
const HEADING_SPREAD: f32 = 0.55;
/// Extra margin around the viewport for the "off screen" preference.
const OFF_SCREEN_MARGIN: f32 = 8.0;
/// Every `ROOM_WAVE_EVERY`-th pulse sends its first enemy in through a room
/// instead of materialising it on the ring.
const ROOM_WAVE_EVERY: u32 = 5;
/// Pulse count that must have fired before room waves start (a wave in the
/// first second would just be a lost enemy).
const ROOM_WAVE_AFTER_PULSES: u32 = 8;

/// While the level's boss is on the field the horde is a chorus, not the
/// headline: the curve is frozen at its late value for the whole fight (see
/// `director::curve_clock`), which used to keep the 600-body cap saturated and
/// the arena opaque, so the bot could not get near the boss for 20-35 minutes
/// while the horde never let go of it: the threat field around the pack
/// outranked the boss.
///
/// So during a boss fight the pulses are stretched by this factor and their
/// size divided by [`BOSS_HORDE_PULSE_DIV`]: the fight is about the boss, the
/// horde still trickles in, and the rat's weapons are not spent on bodies it
/// cannot walk past.
///
/// How thin is tuned by measurement, not taste. The trickle is also the rat's
/// sustain in the fight: its kills are XP, and a level-up during the boss is
/// the heal that keeps the run alive (see `loot`/`loadout`), so starving the
/// chorus turns the fight into a coin flip decided by who runs out of HP
/// first. Three levels on the same 60-seed sample x 2 seed bases:
///
/// | effective spawn | Skilled L1 | Casual L1 | Expert L1 |
/// |---|---|---|---|
/// | x1/24 (6.0 / 4) | 37 % | 27 % | 37 % |
/// | x1/9 (3.0 / 3, old) | 48 % | 23 % | 63 % |
/// | x1/4 (2.0 / 2, this) | 64 % | 29 % | 75 % |
///
/// (30-run batches for the x1/24 row; the other two are 120-run batches — see
/// the balance log.) 2/2 keeps the fight legible and skill-decided and leaves
/// the boss's own pool/telegraph as the difficulty lever.
const BOSS_HORDE_INTERVAL_MULT: f32 = 2.0;
const BOSS_HORDE_PULSE_DIV: usize = 2;
/// Only rooms whose centre is this close to the player are used for a room
/// wave (design change 6 cut this from 1 400 px: a wave that takes fifteen
/// seconds to arrive is a queue, not pressure).
pub const ROOM_WAVE_RANGE: f32 = 320.0;
/// Longest approach (path length, in cells, from the player's cell) a room
/// wave or a flanking group may be staged at — 20 cells ≈ 320 px.
///
/// Measured on the horde's own graph by [`SpawnScratch::build_approach`] and
/// deliberately *path* cells, not straight-line distance: a room 300 px away
/// on the other side of a wall is a 30-cell walk, and that is what the cap is
/// for.
pub const APPROACH_CELLS: i32 = 20;
/// Random cells [`spawn_point_in_room`] tries before giving up.
const ROOM_SAMPLES: u32 = 8;
/// Cells the wall-clear fallback searches around a blocked ring point.
const FALLBACK_SEARCH_CELLS: i32 = 10;
/// Minimum angle between the two flanking approaches (radians, ≈ 60°): two
/// doors on the same wall are one approach, not a pinch.
const FLANK_MIN_SEPARATION: f32 = std::f32::consts::FRAC_PI_3;
/// Cells [`FlankPlan::point`] walks outwards from a crowded doorway before it
/// gives up and stands the group on the anchor.
const FLANK_OUTWARD_CELLS: i32 = 16;

/// Member offsets (in cells) for a flanking group, ordered outwards from the
/// doorway. Members cycle through it, so a late pulse (25+ bodies) still
/// spreads instead of stacking on one pixel; a per-member clearance check keeps
/// a candidate out of a prop or a wall.
const FLANK_OFFSETS: [(i32, i32); 32] = [
    (0, 0),
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
    (2, 0),
    (-2, 0),
    (0, 2),
    (0, -2),
    (2, 1),
    (-2, 1),
    (2, -1),
    (-2, -1),
    (1, 2),
    (-1, 2),
    (1, -2),
    (-1, -2),
    (2, 2),
    (-2, 2),
    (2, -2),
    (-2, -2),
    (3, 0),
    (-3, 0),
    (0, 3),
    (0, -3),
    (3, 1),
    (-3, 1),
    (3, -1),
];

/// Cells of a cell rectangle's inset that a room spawn stays inside.
const ROOM_INSET: i32 = 2;

/// Read-only console tunables and switches, bundled so `director_system` stays
/// within Bevy's 16-parameter `IntoSystem` limit. Neutral defaults: a run that
/// never opens the console reads the same numbers as before.
#[derive(SystemParam)]
pub struct DirectorConsole<'w> {
    pub cvars: Res<'w, Cvars>,
    pub debug: Res<'w, DebugFlags>,
}

/// The two monotonic id counters, bundled for the same reason.
#[derive(SystemParam)]
pub struct DirectorIds<'w> {
    pub next_id: ResMut<'w, NextSpawnId>,
    pub next_enemy_id: ResMut<'w, NextEnemyId>,
    /// The guest's clock (the Drake): bundled here because the director is at
    /// bevy's 16-parameter limit.
    pub guest: ResMut<'w, DrakeSpawner>,
}

#[cfg(test)]
thread_local! {
    /// Test-only measurement hook: when armed (`Some`), every point the
    /// director places a body at is recorded as `(point, rat, tag)` — tag 0 an
    /// ordinary pulse body, 1 a Lilliputian squad's centre, 2 an elite, 3 a
    /// pulse body that was **skipped** (no point). Read by
    /// `spawns_inside_the_widest_view_in_bot_play`.
    pub(crate) static SPAWN_LOG: std::cell::RefCell<Option<Vec<(Vec2, Vec2, u8)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn log_spawn(pos: Vec2, rat: Vec2, tag: u8) {
    SPAWN_LOG.with(|l| {
        if let Some(log) = l.borrow_mut().as_mut() {
            log.push((pos, rat, tag));
        }
    });
}

/// Legacy `World::run_director`, now with the campaign curve, the maze-free
/// spawn rule, room waves and the two-group flank.
pub fn director_system(
    mut commands: Commands,
    input: Res<TickInput>,
    counters: Res<RunCounters>,
    viewport: Res<Viewport>,
    level: Res<Level>,
    player: Res<Player>,
    level_index: Res<LevelIndex>,
    difficulty: Res<DifficultyLevel>,
    mut director: ResMut<Director>,
    (schedule, opening, tuning, mut sfx): (
        Res<MinibossSchedule>, Res<super::floor_sewer::OpeningSpawns>,
        Res<crate::tuning::RunTuning>, ResMut<SfxQueue>,
    ),
    // Console tunables: `set enemy_cap/spawn_rate`, `peace`. Neutral defaults.
    console: DirectorConsole,
    mut boss_due: ResMut<BossDue>,
    mut rng: ResMut<RngRes>,
    mut ids: DirectorIds,
    // `&mut`: the approach boost below is the one write the director makes to a
    // living body (its speed scale, never its position or its rng).
    mut enemies: Query<(&SpawnId, &mut Enemy), Without<MinibossSummon>>,
    // Per-run scratch, reused across ticks: the per-level reachability cache
    // costs one flood fill per map and must not be recomputed per spawn.
    mut scratch: Local<SpawnScratch>,
) {
    let dt = input.dt;
    let elapsed = counters.elapsed;
    // The ring sits outside the widest view, so a body walks further in than it
    // used to: give the ones still outside the 1.0 view their approach speed back (no
    // rng, no spawn, no position) before anything else this tick.
    boost_approach(&mut enemies, &mut director.approach, &player, viewport.0);
    let boss_phase = director.boss_spawned;
    director.pressure.tick(&tuning, difficulty.0, level.theme, elapsed, boss_phase);
    director.pressure.warning &= !console.debug.peace;
    if director.pressure.warning {
        sfx.0.push(crate::audio::Sfx::Warn);
    }
    if counters.testbed && console.debug.peace {
        return;
    }
    // The campaign level shifts the whole curve (`tuning.director.curve_clock`), which
    // also freezes at the boss time, so the horde keeps its late-curve rate for
    // as long as the boss fight lasts.
    let curve = tuning.director.curve_clock(difficulty.0, elapsed);
    // The level's own horde modifiers: damage and speed ramp over the level
    // (`tuning.director.enemy_mods_for`); level 1 is exactly `EnemyMods::NONE`.
    let mods = tuning.director.enemy_mods_for(difficulty.0, elapsed);
    // Keep one slot reserved for the level's boss until it is due, so the live
    // count stays `<= ENEMY_CAP` for the whole level: the boss framework spends
    // the reservation on the boss itself. A living miniboss also has its own
    // reserved slot, separate from the horde's budget.
    // The console's `set enemy_cap N` overrides the built-in cap; 0 means the
    // compiled-in value. A zero cap would stall the horde, so clamp it to 1.
    let base_cap = if console.cvars.enemy_cap == 0 {
        ENEMY_CAP
    } else {
        console.cvars.enemy_cap.max(1)
    };
    let cap = (if director.boss_spawned {
        base_cap
    } else {
        base_cap.saturating_sub(1)
    })
    .saturating_sub(schedule.active as usize);
    // Legacy `self.enemies.len()` is the *live* length, which grows with every
    // push inside the pulse. `Commands` defers those pushes, so mirror the
    // growth locally: live count + spawns queued by this system.
    //
    // The count is only needed on ticks that actually spawn, so it is sampled
    // lazily inside each branch (no per-tick query work when the timer is
    // still running down). `enemies` cannot see this system's own deferred
    // spawns, so re-sampling in the elite branch yields the same value the
    // pulse branch saw.
    let mut queued = 0usize;

    // Flank bookkeeping runs every tick: the dwell is time spent *staying* in
    // one chamber, so leaving it has to reset the clock on the tick it happens.
    director.note_chamber(chamber_room_index(&level, player.pos), dt);

    // Spend the existing timer at the current cadence, so the last surge
    // pulse cannot leave a faster interval behind in the recovery.
    let cadence = director.pressure.cadence;
    director.spawn_timer -= dt * cadence;
    if director.spawn_timer <= 0.0 {
        // After the boss arrives the ordinary horde falls to the design's
        // cleanup pace (1 body/s, live budget 35); during the build-up it uses
        // the §1 curve. A miniboss arrival also halves the rate for 20 s, and
        // its death suppresses fresh spawns for 8 s.
        let grace = schedule.spawn_interval_mult(elapsed);
        // `set spawn_rate N` scales the interval (higher = faster spawns).
        // A zero/negative rate would divide by zero, so clamp to a positive
        // floor; the default 1.0 leaves the interval untouched.
        let rate = if console.cvars.spawn_rate > 0.0 {
            console.cvars.spawn_rate
        } else {
            0.01
        };
        director.spawn_timer = tuning.director.horde_interval(difficulty.0, elapsed, boss_phase) * grace / rate;
        let count = tuning.director.horde_pulse(difficulty.0, elapsed, boss_phase);
        // The fixed opening pair arrives after this pass and spends the same budget.
        let reserved = if elapsed < tuning.director.boss_time
            && level.objects.sewer_opening.is_some()
            && !opening.pillbugs
        {
            super::floor_sewer::OPENING_PILLBUGS_COUNT
        } else {
            0
        };
        let budget = tuning.director.horde_live_budget(difficulty.0, elapsed, boss_phase);
        let budget = director.pressure.live_budget(budget).saturating_sub(reserved);
        let live = enemies.iter_mut().count();
        let guests = if level.theme == Theme::Outfall {
            enemies.iter_mut().filter(|(_, e)| e.kind == EnemyKind::ShoreCrab).count()
        } else {
            0
        };
        if schedule.suppress(elapsed) || console.debug.peace {
            // Quiet after the death, or `peace on`: hold every fresh spawn,
            // but keep the boss hand-off alive.
            if !counters.testbed {
                director::raise_boss_due_at(&mut director, &mut boss_due, elapsed, level_index.0, tuning.director.boss_time);
            }
            return;
        }
        // Horde hp: the level's curve *and* its `LEVEL_HP_EXTRA` (the boss's
        // pool stays on `hp_mult_for`, so this cannot stretch a boss fight).
        let mult = tuning.horde_hp_mult_for(difficulty.0, elapsed);
        // One pulse in `ROOM_WAVE_EVERY` brings its first enemy in through a
        // doorway of a room the player is not in, so the horde arrives instead
        // of all of it materialising on the ring.
        let room_wave = director.pulses >= ROOM_WAVE_AFTER_PULSES
            && director.pulses.is_multiple_of(ROOM_WAVE_EVERY);
        director.pulses = director.pulses.wrapping_add(1);
        // Two off-screen flanking groups: only after the rat has held one
        // chamber for `FLANK_DWELL_SECS`, only when that chamber has two real
        // approaches, and only from the pulse's own enemies. The attempt spends
        // the dwell either way, so a bad chamber is retried once every 8 s, not
        // every pulse.
        let flank = if director.take_flank() {
            let room = director.chamber_room;
            scratch.sync(&level);
            if room >= 0 {
                plan_flank(&level, &player, viewport.0, room as usize, &mut scratch)
            } else {
                None
            }
        } else {
            None
        };
        // Live ordinary bodies per roster slot, so the picker can hand a capped
        // slot's share back to its floor's fodder inside the one draw. Counted
        // once per pulse and topped up as this pulse queues bodies (the design
        // counts queued bodies toward every cap).
        let mut slot_counts = live_slot_counts(level.theme, enemies.iter_mut().map(|(_, e)| e));
        for i in 0..count {
            if live + queued >= cap || live - guests + queued >= budget {
                break;
            }
            // One weighted draw over the floor's eight slots (see
            // `roster::pick_kind`): a locked or capped slot is at zero mass, so
            // the caps cost no retry and no extra rng. The snake's own live cap
            // and every slot cap arrive the same way.
            let caps = roster::SlotCaps::from_counts(level.theme, slot_counts);
            let roster_clock = if elapsed >= tuning.director.boss_time {
                curve.max(director::BOSS_TIME)
            } else {
                curve
            };
            let mut kind = roster::pick_kind(level.theme, roster_clock, &caps, &mut rng.0);
            // A lilliputian roll brings a whole squad instead of one body.
            // The cap keeps it to two squads on the floor; over the cap the
            // roll is substituted with fodder, and the squad itself is
            // truncated to the remaining budget, so the enemy cap still holds.
            if kind == EnemyKind::Lilliputian {
                let live_lill = enemies
                    .iter_mut()
                    .filter(|(_, e)| e.kind == EnemyKind::Lilliputian)
                    .count();
                let room = cap
                    .saturating_sub(live + queued)
                    .min(budget.saturating_sub(live - guests + queued))
                    .min(crate::game::ecs::systems::enemy_wave4::LILLI_SQUAD_SIZE)
                    .min(director::WAVE4_LILLI_LIVE_CAP.saturating_sub(live_lill));
                if room == 0 {
                    // No room for the squad: the floor's own fodder takes the
                    // roll instead, which draws nothing extra.
                    kind = roster::fodder_kind(level.theme);
                } else if let Some(centre) = spawn_pulse_point(
                    &mut rng.0,
                    &level,
                    &player,
                    viewport.0,
                    &mut scratch,
                    room_wave && i == 0,
                    flank.as_ref(),
                    i,
                ) {
                    // Two passes over the same fixed ring of offsets: count
                    // the members that fit, then spawn exactly those. Roles and
                    // slots follow the *placed* index, so the first body on the
                    // floor is always the leader (a blocked first offset used
                    // to leave the squad leaderless), the peak is the number
                    // that really arrived, and no `SpawnId` is spent on a body
                    // that was never spawned.
                    #[cfg(test)]
                    log_spawn(centre, player.pos, 1);
                    let placed = lilli_squad_points(&level, centre, room, |_, _| {});
                    let mut squad = 0u32;
                    let spawned = lilli_squad_points(&level, centre, room, |j, at| {
                        // The leader's `SpawnId` is the squad's id, so a
                        // straggler can name the squad it wants to rejoin.
                        let id = ids.next_id.take();
                        if j == 0 {
                            squad = id.0 as u32;
                        }
                        let mut enemy = Enemy::spawn_with_mods(
                            EnemyKind::Lilliputian,
                            at,
                            mult,
                            mods,
                            &mut rng.0,
                        );
                        enemy.id = ids.next_enemy_id.take();
                        enemy.w4.role = lilli_role(j);
                        enemy.w4.slot = j as u8;
                        enemy.w4.squad = squad;
                        enemy.w4.squad_peak = placed as u16;
                        commands.spawn((enemy, id));
                    });
                    if spawned == 0 {
                        // Nowhere to stand around the point: no squad, and
                        // nothing against the slot's cap.
                        continue;
                    }
                    queued += spawned;
                    // One roll, one body against the cap, however many of the
                    // squad made it onto the floor.
                    if let Some(slot) = roster::floor(level.theme).slot_of(EnemyKind::Lilliputian)
                    {
                        let at = slot.index();
                        slot_counts[at] = slot_counts[at].saturating_add(1);
                    }
                    continue;
                } else {
                    // No safe off-screen point for the squad: skip the pulse
                    // body entirely, exactly like a failed single spawn.
                    #[cfg(test)]
                    log_spawn(player.pos, player.pos, 3);
                    continue;
                }
            }
            let Some(pos) = spawn_pulse_point(
                &mut rng.0,
                &level,
                &player,
                viewport.0,
                &mut scratch,
                room_wave && i == 0,
                flank.as_ref(),
                i,
            ) else {
                // No safe, off-screen, wall-clear point (the rat is deep in a
                // maze, or boxed into a pocket): skip the spawn. A body in a
                // warren or on top of the player is worse than no body.
                #[cfg(test)]
                log_spawn(player.pos, player.pos, 3);
                continue;
            };
            if level.theme == Theme::Outfall
                && !spawn_circle_clear(&level, pos, kind.radius(), SizeClass::of_enemy(kind))
            {
                #[cfg(test)]
                log_spawn(player.pos, player.pos, 3);
                continue;
            }
            #[cfg(test)]
            log_spawn(pos, player.pos, 0);
            let mut enemy = Enemy::spawn_with_mods(kind, pos, mult, mods, &mut rng.0);
            // Neon merge: legacy `run_director` ids every director-spawned
            // enemy from the monotonic `next_enemy_id` counter (0 is reserved
            // for "anonymous enemy" in `DamageNumbers`). `SpawnId` and the
            // enemy id are two independent orderings; never share them.
            enemy.id = ids.next_enemy_id.take();
            commands.spawn((enemy, ids.next_id.take()));
            queued += 1;
            if let Some(slot) = roster::floor(level.theme).slot_of(kind) {
                let at = slot.index();
                slot_counts[at] = slot_counts[at].saturating_add(1);
            }
        }
    }

    // Periodic elite brute. Skip it instead of taking the boss's slot, and skip
    // outright when no safe point exists.
    director.elite_timer -= dt;
    //
    // An elite never arrives within `ELITE_APPOINTMENT_GAP` of a miniboss
    // appointment (one named threat at a time): a due elite waits, with its
    // timer left spent and no rng drawn, and fires when the window closes.
    if director.elite_timer <= 0.0
        && curve > tuning.director.elite_start_for(difficulty.0)
        && !tuning.minibosses.times.iter().any(|&at| (elapsed - at).abs() < tuning.director.elite_appointment_gap)
    {
        director.elite_timer = tuning.director.elite_interval_at(difficulty.0, curve);
        let elites = enemies.iter_mut().filter(|(_, e)| e.elite).count();
        if enemies.iter_mut().count() + queued < cap && elites < 3 {
            if let Some(pos) = spawn_point_reachable(
                &mut rng.0,
                &level,
                &player,
                viewport.0,
                &mut scratch,
            ).filter(|&pos| {
                let kind = roster::elite_kind(level.theme);
                level.theme != Theme::Outfall
                    || spawn_circle_clear(&level, pos, kind.radius(), SizeClass::of_enemy(kind))
            }) {
                #[cfg(test)]
                log_spawn(pos, player.pos, 2);
                let mult = tuning.horde_hp_mult_for(difficulty.0, elapsed) * 1.6;
                // The elite wears the floor's own heavy (the roster's H slot):
                // its elite is that body with the double stripe and the
                // attachment, so the silhouette the rat has been fighting since
                // 6:35 is the one that turns up armoured.
                let mut enemy = Enemy::spawn_with_mods(
                    roster::elite_kind(level.theme),
                    pos,
                    mult,
                    mods,
                    &mut rng.0,
                );
                enemy.id = ids.next_enemy_id.take();
                // Director elites are flagged so the art and death VFX can tell
                // them apart from an ordinary late-game heavy with overlapping
                // scaled HP.
                enemy.elite = true;
                commands.spawn((enemy, ids.next_id.take()));
            }
        }
    }

    // The guest: at most one Drake on the Sewer (two from 8:00), from 2:30 on
    // the curve clock, 45 s after the last one spawned or died, never during
    // the boss fight and never beside a live miniboss. It spends no roster
    // slot, no live budget and no cap: it is placed through the same
    // out-of-sight ladder a straggler uses, and it is the only body on the
    // floor that is not the roster's (`systems::drake`).
    //
    // The `peace`/suppress early return above has already held it, like every
    // other spawn. The curve clock freezes at the boss time, so "after
    // `BossDue`" is read from the director and the hand-off, not from it.
    //
    // The theme and clock gate is first: the other five floors and the first
    // two and a half minutes of the Sewer pay one compare a tick (nothing can
    // exist before 2:30, so the death clock starts empty and correct).
    // It spends no roster slot and no live *budget*, but it does respect the
    // hard entity cap (and with it the slot the cap reserves for the boss): a
    // guest must never be the body that crowds the boss out.
    if level.theme == Theme::Sewer
        && curve >= crate::game::ecs::systems::drake::FROM_CLOCK
        && enemies.iter_mut().count() < cap
    {
        let live_drakes = enemies
            .iter_mut()
            .filter(|(_, e)| e.kind == EnemyKind::Drake)
            .count();
        let mut tick = crate::game::ecs::systems::drake::GuestTick {
            elapsed,
            curve,
            theme: level.theme,
            player: &player,
            viewport: viewport.0,
            level: &level,
            boss_phase: director.boss_spawned || boss_due.due || boss_due.claimed,
            miniboss_alive: schedule.active > 0,
            live_drakes,
            hp_mult: tuning.horde_hp_mult_for(difficulty.0, elapsed),
            mods,
            spawner: &mut ids.guest,
            scratch: &mut scratch,
            rng: &mut rng.0,
            next_id: &mut ids.next_id,
            next_enemy_id: &mut ids.next_enemy_id,
        };
        crate::game::ecs::systems::drake::spawn_guest(&mut tick, &mut commands);
    }

    // Boss hand-off (no generic boss spawn any more): the boss framework reads
    // `BossDue` and spawns the level's boss in its arena.
    if !counters.testbed {
        director::raise_boss_due_at(&mut director, &mut boss_due, elapsed, level_index.0, tuning.director.boss_time);
    }
}

/// The role of the `j`-th body of a Lilliputian squad **that made it onto the
/// floor**: the first is always the leader.
#[inline]
fn lilli_role(j: usize) -> u8 {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    match j {
        0 => w4::LILLI_ROLE_LEADER,
        1..=3 => w4::LILLI_ROLE_SPEAR,
        4 | 5 => w4::LILLI_ROLE_ARCHER,
        _ => w4::LILLI_ROLE_ROPE,
    }
}

/// Walk the `room` ring offsets of a squad around `centre` and call `place`
/// with `(placed index, position)` for each one a Lilliputian fits at. Returns
/// how many fitted. No rng and no allocation, so the director runs it twice
/// (count, then spawn) and gets the same answer both times.
fn lilli_squad_points(
    level: &Level,
    centre: Vec2,
    room: usize,
    mut place: impl FnMut(usize, Vec2),
) -> usize {
    let mut placed = 0usize;
    for k in 0..room {
        let off = Vec2::from_angle(k as f32 / room as f32 * std::f32::consts::TAU) * 7.0;
        let at = centre + off;
        if !level
            .tiles
            .circle_clear(at, EnemyKind::Lilliputian.radius(), SizeClass::Small)
        {
            continue;
        }
        place(placed, at);
        placed += 1;
    }
    placed
}

/// Live ordinary bodies per roster slot on `theme`, counted the way the
/// design's caps count them.
///
/// * A **named miniboss** is a `Brute` body but not the floor's heavy, and an
///   **elite** is the heavy plus its armour: neither is an ordinary body, so
///   neither spends a slot's cap.
/// * A **squad** counts once ("count a squad, segment chain or swarm as one
///   body with one payout"): ten lilliputians are one II body, and a
///   half-destroyed squad still counts once, which is why the count rounds up.
fn live_slot_counts<E: std::ops::Deref<Target = Enemy>>(
    theme: Theme,
    enemies: impl Iterator<Item = E>,
) -> [u16; roster::SLOT_COUNT] {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    let floor = roster::floor(theme);
    let mut counts = [0u16; roster::SLOT_COUNT];
    let mut lillis = 0u16;
    for e in enemies {
        if e.named || e.elite {
            continue;
        }
        if e.kind == EnemyKind::Lilliputian {
            lillis = lillis.saturating_add(1);
            continue;
        }
        if let Some(slot) = floor.slot_of(e.kind) {
            let i = slot.index();
            counts[i] = counts[i].saturating_add(1);
        }
    }
    if lillis > 0 {
        if let Some(slot) = floor.slot_of(EnemyKind::Lilliputian) {
            let squads = (lillis as usize).div_ceil(w4::LILLI_SQUAD_SIZE);
            let i = slot.index();
            counts[i] = counts[i].saturating_add(squads as u16);
        }
    }
    counts
}

/// One tick of the approach speed boost.
///
/// A body outside the unzoomed (1.0) view walks in at
/// [`APPROACH_BOOST_GROUND`] (×1.3 for flyers) until it comes into that view,
/// at which point it is put back on exactly the speed scale it spawned with.
/// (A body that wanders back out of it is boosted again: the rule is where the
/// body is, not where it has been.)
/// `Director::approach` is the registry, keyed by `SpawnId` and sorted, so the
/// boost is applied and removed exactly once per body.
///
/// Why this mechanism rather than a pacing change: it draws no rng, changes no
/// spawn position, no spawn count and no pulse cadence, and it cannot fire
/// inside the 1.0 view — so the change that makes a spawn invisible at every
/// zoom leaves the horde's arrival schedule (and the number of rng draws per
/// run) alone.
///
/// Never a named miniboss (it has its own arrival walk), never an elite, never
/// a body whose own speed is already [`APPROACH_SPEED_CEIL`] or more.
fn boost_approach(
    enemies: &mut Query<(&SpawnId, &mut Enemy), Without<MinibossSummon>>,
    approach: &mut Vec<(u64, f32)>,
    player: &Player,
    viewport: Vec2,
) {
    let mut live = 0usize;
    for (id, mut e) in enemies.iter_mut() {
        live += 1;
        let at = approach.binary_search_by_key(&id.0, |&(id, _)| id);
        let out_of_sight = !e.named
            && !e.elite
            && e.speed_scale > 0.0
            && !in_sim_view(e.pos, player.pos, viewport);
        if !out_of_sight {
            // Back in view (or no longer an ordinary body): restore the exact
            // scale it spawned with — a multiply/divide pair would leave a
            // rounding error on every body for the rest of the run.
            if let Ok(at) = at {
                e.speed_scale = approach[at].1;
                approach.remove(at);
            }
            continue;
        }
        if at.is_ok() {
            continue;
        }
        let speed = e.kind.speed();
        if speed <= 0.0 || speed >= APPROACH_SPEED_CEIL {
            continue;
        }
        let boost = if e.kind.archetype() == EnemyKind::Bat {
            APPROACH_BOOST_FLYER
        } else {
            APPROACH_BOOST_GROUND
        };
        if let Err(at) = at {
            approach.insert(at, (id.0, e.speed_scale));
            e.speed_scale *= boost;
        }
    }
    // A body that dies while it is still boosted leaves its row behind (the
    // restore above only ever sees the living), and most fodder dies out there.
    // `SpawnId`s are never reused, so a dead row is only weight: sweep them when
    // they outnumber the living. No rng, no simulation state.
    if approach.len() > live * 2 + 64 {
        let mut alive = vec![false; approach.len()];
        for (id, _) in enemies.iter() {
            if let Ok(at) = approach.binary_search_by_key(&id.0, |&(id, _)| id) {
                alive[at] = true;
            }
        }
        let mut row = 0;
        approach.retain(|_| {
            row += 1;
            alive[row - 1]
        });
    }
}

/// 8-connected neighbourhood, cardinals first.
const HORDE_OFFS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// Flood fill over the cells the **horde** can actually occupy.
///
/// Deliberately stricter than `levelgen::validate::reachable`, in one way that
/// matters: a one-way door cell counts as solid. The gate leaf is a permanent
/// collider for everything that is not the player ([`TileMap::resolve_circle`]
/// with `player_vel: None`), so an enemy spawned in a maze whose only exit is a
/// gate could never leave — the exact "lost enemy in a sealed pocket" this
/// cache exists to prevent. Excluding the gate cells also makes the horde graph
/// symmetric, so "reachable from the start" and "can walk back to the start"
/// agree: a maze entered through its normal doorway is both, and its one-way
/// exit is just a wall.
fn horde_reachable(tiles: &TileMap, from: (i32, i32)) -> Vec<bool> {
    let cols = tiles.cols();
    let rows = tiles.rows();
    let mut seen = vec![false; (cols * rows) as usize];
    let walkable = |cx: i32, cy: i32| {
        tiles.passable(cx, cy, SizeClass::Large) && tiles.one_way_dir(cx, cy).is_none()
    };
    if !walkable(from.0, from.1) {
        return seen;
    }
    seen[(from.1 * cols + from.0) as usize] = true;
    let mut queue: Vec<(i32, i32)> = Vec::with_capacity((cols * rows) as usize / 8);
    queue.push(from);
    let mut head = 0usize;
    while head < queue.len() {
        let (cx, cy) = queue[head];
        head += 1;
        for (dx, dy) in HORDE_OFFS {
            let (nx, ny) = (cx + dx, cy + dy);
            if !tiles.in_bounds(nx, ny) {
                continue;
            }
            let i = (ny * cols + nx) as usize;
            if seen[i] || !walkable(nx, ny) {
                continue;
            }
            // No squeezing diagonally between two walls (the flow field's rule).
            if dx != 0 && dy != 0 && !(walkable(cx + dx, cy) && walkable(cx, cy + dy)) {
                continue;
            }
            seen[i] = true;
            queue.push((nx, ny));
        }
    }
    seen
}

/// The ring/interior rectangle of one maze section: the area no enemy may ever
/// be *introduced* in.
///
/// This is the generator's own ring (`maze_ring`), i.e. the `WallKind::Maze`
/// border plus everything inside it. The one-cell open moat between the ring and
/// the reserved slot's edge is a normal corridor and stays legal ground — the
/// ring is what the designer measured against. Worker P1 is exporting this
/// rectangle on `Level`; until that lands this derives it from `Level::mazes`
/// with the generator's own helper, so there is still exactly one definition of
/// where a warren is.
#[inline]
pub fn maze_region(maze: &MazeSection) -> TileRect {
    crate::game::levelgen::maze::maze_ring(maze.bounds)
}

/// Is `p` inside any maze's ring or interior?
#[inline]
pub fn in_maze(level: &Level, p: Vec2) -> bool {
    level.mazes.iter().any(|m| maze_region(m).contains_world(p))
}

/// Is `(cx, cy)` inside any maze's ring or interior?
#[inline]
pub fn cell_in_maze(level: &Level, cx: i32, cy: i32) -> bool {
    level
        .mazes
        .iter()
        .any(|m| maze_region(m).contains(cx, cy))
}

/// The chamber the player is standing in, as a `Level::rooms` index.
///
/// Only the kinds a player can *camp* in count (a hall, an ordinary room, the
/// start room, the boss arena); a corridor is a passage, not a chamber, so it
/// never charges the flank timer.
#[inline]
pub fn chamber_room_index(level: &Level, p: Vec2) -> Option<usize> {
    level
        .rooms
        .iter()
        .position(|room| room.rect.contains_world(p))
        .filter(|&i| {
            matches!(
                level.rooms[i].kind,
                RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room
            )
        })
        .or_else(|| {
            // `position` may have found a corridor drawn over the chamber
            // first; fall through to any chamber that also contains the point.
            level.rooms.iter().position(|room| {
                room.rect.contains_world(p)
                    && matches!(
                        room.kind,
                        RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room
                    )
            })
        })
}

/// How far the spawn search had to fall back before it found a point. Counted
/// per level on the [`SpawnScratch`] (a `Local`, so it costs no resource), so a
/// test — or the coordinator measuring the doubled ring — can read how often a
/// 2× ring is clamped against a wall, the level edge or a corridor.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnFallbacks {
    /// The widest-view ring: the normal case, invisible at every zoom.
    pub ring: u32,
    /// 1.5 ring radii out (the ring itself was walled, further out was not).
    pub ring_outward: u32,
    /// The 1.0-view ring: nearer, still outside the unzoomed view.
    pub ring_inward: u32,
    /// Any legal point on the ring, which can be inside the widest view.
    pub ring_any: u32,
    /// The nearest staging room whose cell is outside the unzoomed view.
    pub room_out_of_view: u32,
    /// The nearest staging room, whatever the view (the rat is boxed in).
    pub room: u32,
    /// The nearest clear cell around the ring point (the last legal resort).
    pub near: u32,
    /// No point at all: the spawn was skipped.
    pub skipped: u32,
    /// A room-wave point was refused because it was inside the widest view.
    pub room_wave_in_view: u32,
}

/// Facts about one level that every spawn needs but that are far too expensive
/// to recompute per spawn: which cells the horde can actually path to, which
/// rooms contain such a cell, and (rebuilt when the player changes cell) the
/// path distance from the player out to [`APPROACH_CELLS`] cells.
///
/// The map is static for a whole run, so the reachability side is built **once**
/// (at the first pulse) and then read as O(1) table lookups — the "cache a
/// per-room reachability flag" the content plan asks for instead of a flood
/// fill per spawn. It is keyed by the level's identity (`seed`, `theme`, room
/// count), so a test that swaps in a different map rebuilds it automatically; a
/// test that repaints a map in place must give it a different seed.
#[derive(Default)]
pub struct SpawnScratch {
    key: Option<(u64, u8, usize)>,
    /// One flag per cell, row-major: can a `Large` agent path here from the
    /// start room (one-way doors honoured)?
    reach: Vec<bool>,
    /// One flag per entry in `Level::rooms`: does the room hold a reachable
    /// cell (so the horde can enter it)?
    room_ok: Vec<bool>,
    /// Bounded path-length table around the player: `0` = not built, otherwise
    /// `steps` from the player's cell (1 = the player's own cell). Bounded to
    /// ≤ [`APPROACH_CELLS`], so a build touches a few thousand cells at most.
    approach: Vec<u16>,
    /// Cells written into `approach` by the last build, so resetting it costs
    /// the cells it touched instead of the whole map.
    approach_touched: Vec<(i32, i32)>,
    /// The cell `approach` was built from.
    approach_from: Option<(i32, i32)>,
    /// BFS queue, reused.
    queue: Vec<(i32, i32)>,
    /// One flag per entry in `Level::rooms`: does the room hold a cell inside
    /// the current approach table? Built with the table, read per room-wave
    /// candidate (the room scan itself is ~100x more expensive than this).
    approach_rooms: Vec<bool>,
    /// Chamber openings: raw candidates, then one representative per cluster.
    raw_openings: Vec<(i32, i32)>,
    openings: Vec<(i32, i32)>,
    /// How often each fallback rung of the search fired (see [`SpawnFallbacks`]).
    pub fallbacks: SpawnFallbacks,
}

impl SpawnScratch {
    /// (Re)build the cached facts when this is a different map.
    pub fn sync(&mut self, level: &Level) {
        let cells = (level.tiles.cols() * level.tiles.rows()) as usize;
        if self.approach.len() != cells {
            self.approach = vec![0u16; cells];
            self.approach_touched.clear();
            self.approach_from = None;
        }
        let key = (level.seed, level.theme as u8, level.rooms.len());
        if self.key == Some(key) && self.reach.len() == cells {
            return;
        }
        self.key = Some(key);
        // A different map: the approach table describes the old one.
        self.approach_from = None;
        // The horde's own map, not the validator's: see [`horde_reachable`].
        self.reach = horde_reachable(&level.tiles, cell_of(level.start));
        let cols = level.tiles.cols();
        let reach = &self.reach;
        self.room_ok.clear();
        self.room_ok.extend(level.rooms.iter().map(|room| {
            room.rect.cells().any(|(cx, cy)| {
                reach
                    .get((cy * cols + cx) as usize)
                    .copied()
                    .unwrap_or(false)
            })
        }));
    }

    /// Is the cell containing `p` on the horde's connected map?
    pub fn is_reachable(&self, level: &Level, p: Vec2) -> bool {
        let (cx, cy) = cell_of(p);
        // An off-map column must not wrap into the neighbouring row's index.
        if !level.tiles.in_bounds(cx, cy) {
            return false;
        }
        self.reach
            .get((cy * level.tiles.cols() + cx) as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Can the horde enter `room` (any reachable cell inside it)?
    pub fn room_reachable(&self, index: usize) -> bool {
        self.room_ok.get(index).copied().unwrap_or(false)
    }

    /// (Re)build the bounded approach table around `from` when the player has
    /// moved to a new cell. Costs one BFS over ≤ ~1 700 cells, so it is only
    /// called when a room wave or a flank is actually being considered.
    pub fn build_approach(&mut self, level: &Level, from: Vec2) {
        let start = cell_of(from);
        if self.approach_from == Some(start) {
            return;
        }
        let cols = level.tiles.cols();
        let rows = level.tiles.rows();
        if self.approach.len() != (cols * rows) as usize {
            self.approach = vec![0u16; (cols * rows) as usize];
        }
        for (cx, cy) in self.approach_touched.drain(..) {
            if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
                self.approach[(cy * cols + cx) as usize] = 0;
            }
        }
        self.approach_rooms.clear();
        self.approach_from = Some(start);
        let walkable = |tiles: &TileMap, cx: i32, cy: i32| {
            tiles.passable(cx, cy, SizeClass::Large) && tiles.one_way_dir(cx, cy).is_none()
        };
        if !walkable(&level.tiles, start.0, start.1) {
            return;
        }
        self.approach[(start.1 * cols + start.0) as usize] = 1;
        self.approach_touched.push(start);
        self.queue.clear();
        self.queue.push(start);
        let mut head = 0usize;
        while head < self.queue.len() {
            let (cx, cy) = self.queue[head];
            head += 1;
            let d = self.approach[(cy * cols + cx) as usize];
            if d as i32 >= APPROACH_CELLS {
                continue;
            }
            for (dx, dy) in HORDE_OFFS {
                let (nx, ny) = (cx + dx, cy + dy);
                if !level.tiles.in_bounds(nx, ny) {
                    continue;
                }
                let i = (ny * cols + nx) as usize;
                if self.approach[i] != 0 || !walkable(&level.tiles, nx, ny) {
                    continue;
                }
                if dx != 0
                    && dy != 0
                    && !(walkable(&level.tiles, cx + dx, cy) && walkable(&level.tiles, cx, cy + dy))
                {
                    continue;
                }
                self.approach[i] = d + 1;
                self.approach_touched.push((nx, ny));
                self.queue.push((nx, ny));
            }
        }
        // One pass over the rooms: which of them hold a cell in the table.
        let mut rooms = std::mem::take(&mut self.approach_rooms);
        rooms.clear();
        let approach = &self.approach;
        rooms.extend(level.rooms.iter().map(|room| {
            room.rect
                .cells()
                .any(|(cx, cy)| approach[(cy * cols + cx) as usize] != 0)
        }));
        self.approach_rooms = rooms;
    }

    /// Path length in cells from the player's cell to the cell containing `p`,
    /// if the approach table reaches it (≤ [`APPROACH_CELLS`]).
    #[inline]
    pub fn approach_cells(&self, level: &Level, p: Vec2) -> Option<u16> {
        let (cx, cy) = cell_of(p);
        if cx < 0 || cy < 0 || cx >= level.tiles.cols() || cy >= level.tiles.rows() {
            return None;
        }
        match self.approach[(cy * level.tiles.cols() + cx) as usize] {
            0 => None,
            d => Some(d - 1),
        }
    }

    /// Does `index` (a `Level::rooms` position) hold a cell within
    /// [`APPROACH_CELLS`] path cells of the player?
    #[inline]
    fn room_in_approach(&self, index: usize) -> bool {
        self.approach_rooms.get(index).copied().unwrap_or(false)
    }

    /// Collect one representative per approach (doorway) of `room`.
    ///
    /// An approach is a cell on the room's boundary that is open floor inside
    /// the room *and* touches open floor outside it. Adjacent boundary cells of
    /// the same opening are merged (a two-cell doorway is one approach), so the
    /// "two real approaches" test cannot be satisfied by the two halves of one
    /// door. Fills `self.openings`; allocation-free after warm-up.
    fn scan_openings(&mut self, level: &Level, room: &Room) {
        self.raw_openings.clear();
        let rect = room.rect;
        for (cx, cy) in rect.cells() {
            let edge = cx == rect.x
                || cy == rect.y
                || cx == rect.right() - 1
                || cy == rect.bottom() - 1;
            if !edge {
                continue;
            }
            if !level.tiles.passable(cx, cy, SizeClass::Large) {
                continue;
            }
            let mut opens = false;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if rect.contains(nx, ny) {
                    continue;
                }
                if level.tiles.passable(nx, ny, SizeClass::Large) {
                    opens = true;
                    break;
                }
            }
            if opens {
                self.raw_openings.push((cx, cy));
            }
        }
        self.openings.clear();
        for k in 0..self.raw_openings.len() {
            let (cx, cy) = self.raw_openings[k];
            let near = self
                .openings
                .iter()
                .any(|&(ox, oy)| (cx - ox).abs() <= 1 && (cy - oy).abs() <= 1);
            if !near {
                self.openings.push((cx, cy));
            }
        }
    }

    /// The representatives from the last [`SpawnScratch::scan_openings`].
    #[inline]
    pub fn openings(&self) -> &[(i32, i32)] {
        &self.openings
    }
}

/// The cell containing a world position (the level generator's rule).
#[inline]
fn cell_of(p: Vec2) -> (i32, i32) {
    (
        (p.x / crate::game::collision::TILE_F).floor() as i32,
        (p.y / crate::game::collision::TILE_F).floor() as i32,
    )
}

/// Radius of the spawn ring: just beyond the **widest** view the zoom ladder
/// can show, floored so a small terminal still spawns off screen.
///
/// The ring is a circle and the off-screen test is per axis, so the widest
/// view's half-diagonal (plus [`RING_MARGIN`]) puts every sample outside that
/// rectangle on every bearing: a point at that radius always has one axis
/// further out than the widest view's half-extent on that axis. This is what
/// keeps the pop-in the owner saw at `2.0` from happening.
#[inline]
pub fn ring_radius(viewport: Vec2) -> f32 {
    (widest_view(viewport).len() * 0.5 + RING_MARGIN).max(MIN_RING_RADIUS)
}

/// Radius of the ring at the **unzoomed** (1.0) view: the old radius, kept as
/// the first inward step of the spawn search and as the line the escape paths
/// hold ("at least outside the 1.0 view").
#[inline]
pub fn one_x_ring_radius(viewport: Vec2) -> f32 {
    (viewport.len() * 0.5 + RING_MARGIN).max(MIN_RING_RADIUS)
}

/// `World::spawn_point` . Also used by the facade's
/// `World::spawn_point` test hook.
///
/// A wall-clear point on a ring around the player, just beyond view, preferring
/// the direction the player is heading and cells that are off screen. Bounded
/// by the arena margin and a minimum distance, and never inside a wall.
///
/// This is the ring primitive: it carries no reachability cache (see
/// [`SpawnScratch`]) and therefore cannot prove the point is on the horde's
/// connected map. The director uses [`spawn_point_reachable`], which filters
/// the very same ring through the cached flood fill; both share the sampling
/// below, so there is one ring implementation, not two.
pub fn spawn_point(rng: &mut StdRng, level: &Level, player: &Player, viewport: Vec2) -> Vec2 {
    let mut fb = SpawnFallbacks::default();
    spawn_point_counted(rng, level, player, viewport, &mut fb)
}

/// [`spawn_point`], with the fallback rungs counted into `fb`.
fn spawn_point_counted(
    rng: &mut StdRng,
    level: &Level,
    player: &Player,
    viewport: Vec2,
    fb: &mut SpawnFallbacks,
) -> Vec2 {
    let base = ring_base_angle(rng, player);
    sample_ring(level, player, viewport, base, None, fb)
        .or_else(|| fallback_point(level, player, viewport, base, None))
        .unwrap_or_else(|| last_resort(level, player, viewport, base))
}

/// The ring given an angle that has *already* been drawn: the pulse loop draws
/// one angle per enemy (so the draw count never depends on which placement rule
/// an enemy ends up using) and hands it in here.
fn spawn_point_from_base(
    level: &Level,
    player: &Player,
    viewport: Vec2,
    base: f32,
    reach: Option<&[bool]>,
    fb: &mut SpawnFallbacks,
) -> Option<Vec2> {
    sample_ring(level, player, viewport, base, reach, fb)
}

/// The director's spawn point: [`spawn_point`] plus the cached reachability
/// test, so an enemy never materialises in a pocket the horde cannot leave.
///
/// `None` means "skip this spawn": the ring had no wall-clear, off-screen,
/// maze-free, reachable candidate and the only fallbacks left would drop a body
/// into a warren or onto the rat, which is worse than one fewer enemy.
///
/// When the whole ring is walled in (the rat ducking through a rat-hole or
/// into a maze aisle) the ladder is: the nearest staging room whose cell is
/// outside the **unzoomed** view (so the body still arrives from out of sight),
/// then the nearest staging room at all, then the nearest clear cell around the
/// ring point. Every rung is counted in [`SpawnScratch::fallbacks`].
pub fn spawn_point_reachable(
    rng: &mut StdRng,
    level: &Level,
    player: &Player,
    viewport: Vec2,
    scratch: &mut SpawnScratch,
) -> Option<Vec2> {
    let base = ring_base_angle(rng, player);
    scratch.sync(level);
    spawn_ladder(level, player, viewport, base, scratch)
}

/// The search ladder both spawn entry points share: the ring passes, then the
/// room escapes, then the nearest clear cell around the ring point.
fn spawn_ladder(
    level: &Level,
    player: &Player,
    viewport: Vec2,
    base: f32,
    scratch: &mut SpawnScratch,
) -> Option<Vec2> {
    let mut stats = scratch.fallbacks;
    let hit = spawn_point_from_base(level, player, viewport, base, Some(&scratch.reach), &mut stats);
    scratch.fallbacks = stats;
    if let Some(p) = hit {
        return Some(p);
    }
    let room = nearest_reachable_room_cell(level, scratch, player, SizeClass::Large, viewport);
    if let Some((p, out_of_view)) = room {
        if out_of_view {
            scratch.fallbacks.room_out_of_view += 1;
        } else {
            scratch.fallbacks.room += 1;
        }
        return Some(p);
    }
    let last = fallback_point(level, player, viewport, base, Some(&scratch.reach));
    match last {
        Some(p) => {
            scratch.fallbacks.near += 1;
            Some(p)
        }
        None => {
            scratch.fallbacks.skipped += 1;
            None
        }
    }
}

/// Where member `index` of a pulse spawns.
///
/// Order matters for determinism: the ring angle is drawn first, whatever
/// placement rule the enemy ends up using (a flank reuses the pulse's enemies,
/// so it must not change how many rng values a pulse consumes per member).
/// `None` means skip.
pub fn spawn_pulse_point(
    rng: &mut StdRng,
    level: &Level,
    player: &Player,
    viewport: Vec2,
    scratch: &mut SpawnScratch,
    room_wave: bool,
    flank: Option<&FlankPlan>,
    index: usize,
) -> Option<Vec2> {
    // One angle draw per pulse member, exactly like the legacy ring.
    let base = ring_base_angle(rng, player);
    if let Some(flank) = flank {
        // The table was synced when the flank was planned (same tick).
        if let Some(p) = flank.point(level, player, viewport, index, Some(&scratch.reach)) {
            return Some(p);
        }
        // No out-of-sight cell around this group's doorway: the ring stages
        // this body instead (still out of sight), so a pinch never pops in.
    }
    if room_wave {
        if let Some(p) = room_wave_point(rng, level, player, scratch) {
            if off_screen(p, player.pos, viewport) {
                return Some(p);
            }
            // The chosen room is inside the widest view (a room wave is staged
            // in a chamber the rat is not in, but that chamber can be on
            // screen). The ring stages this body instead, so nothing
            // materialises where the rat can see it. One counter, no rng.
            scratch.fallbacks.room_wave_in_view += 1;
        }
    }
    scratch.sync(level);
    spawn_ladder(level, player, viewport, base, scratch)
}

/// A room-wave spawn: pick (deterministically, from the caller's rng) one room
/// the horde can enter, that the player is not standing in and that is close
/// enough to arrive, then sample a cell inside it.
///
/// `None` when no room qualifies. Draws one rng value for the room choice, so
/// the number of draws depends only on the level and the player's room — never
/// on rejection loop counts inside a room, which are fixed by
/// [`spawn_point_in_room`].
pub fn room_wave_point(
    rng: &mut StdRng,
    level: &Level,
    player: &Player,
    scratch: &mut SpawnScratch,
) -> Option<Vec2> {
    scratch.sync(level);
    scratch.build_approach(level, player.pos);
    // The chamber the rat is standing in, if any: no wave may be staged in a
    // room whose rectangle touches it. Levels are generated as merged open
    // areas now, so two rooms can share floor and a room-wave rectangle can
    // *contain* the rat even though `room_at` names a different one — the old
    // "the rat is not inside this rect" test missed that (this is the fix for
    // the failing `room_waves_stage_enemies_outside_the_players_room`).
    let player_room = level.room_at(player.pos).map(|r| r.rect);
    let mut count = 0u32;
    for (i, room) in level.rooms.iter().enumerate() {
        if room_wave_ok(scratch, i, room, player, player_room) {
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let pick = rng.random_range(0..count);
    let mut seen = 0u32;
    for (i, room) in level.rooms.iter().enumerate() {
        if !room_wave_ok(scratch, i, room, player, player_room) {
            continue;
        }
        if seen == pick {
            return room_approach_point(rng, level, room, player, scratch);
        }
        seen += 1;
    }
    None
}

/// Is this room a legal room-wave target?
///
/// Three filters, in increasing cost: a staging room (the horde can enter it),
/// a room the player is not in, within [`ROOM_WAVE_RANGE`] of him, and — the
/// design change — an approach of at most [`APPROACH_CELLS`] path cells so the
/// wave arrives in seconds instead of queueing behind a wall.
#[inline]
fn room_wave_ok(
    scratch: &SpawnScratch,
    index: usize,
    room: &Room,
    player: &Player,
    player_room: Option<TileRect>,
) -> bool {
    room_staging_ok(scratch, index, room)
        && !room.rect.contains_world(player.pos)
        && player_room.is_none_or(|area| !room.rect.intersects(&area))
        && room.center().dist(player.pos) <= ROOM_WAVE_RANGE
        && scratch.room_in_approach(index)
}

/// Is this room a place the horde can gather in at all? Narrow corridors and
/// maze aisles are not staging rooms: a wave has to look like it came through a
/// door, and the horde has to be able to surround the player when it arrives.
#[inline]
fn room_staging_ok(scratch: &SpawnScratch, index: usize, room: &crate::game::level::Room) -> bool {
    scratch.room_reachable(index)
        && matches!(
            room.kind,
            RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room
        )
}

/// A wall-clear cell inside `room`, chosen without any rng (the first one in
/// cell order): the deterministic escape used when no ring point is legal.
///
/// `keep_away` is the rat: a cell closer than [`SPAWN_MIN_DIST`] to it is never
/// chosen, so "no safe off-screen point" degrades to *skipping* the spawn rather
/// than dropping a body on top of the player.
fn room_open_cell(
    level: &Level,
    room: &Room,
    class: SizeClass,
    keep_away: Option<Vec2>,
    reach: Option<&[bool]>,
) -> Option<Vec2> {
    let area = room.rect.inflate(-ROOM_INSET);
    if area.w <= 0 || area.h <= 0 {
        return None;
    }
    let clear = room_clearance(class);
    let mut found = None;
    for (cx, cy) in area.cells() {
        if cell_in_maze(level, cx, cy) {
            continue;
        }
        let p = TileMap::tile_rect(cx, cy).center();
        if let Some(from) = keep_away {
            if p.dist_sq(from) < SPAWN_MIN_DIST * SPAWN_MIN_DIST {
                continue;
            }
        }
        // The cell itself must be on the horde's map, not just somewhere in
        // a room that has such a cell (a walled-off corner of the room is a
        // pocket the body could never leave).
        if !cell_reachable(reach, level, p) {
            continue;
        }
        if level.tiles.passable(cx, cy, class) && spawn_circle_clear(level, p, clear, class) {
            found = Some(p);
            break;
        }
    }
    found
}

/// The nearest room the horde can enter, and a wall-clear cell inside it.
///
/// Returns the point and whether it is outside the **unzoomed** view. A room
/// whose cell is out of sight is preferred (only tie-broken by distance among
/// rooms that are): a body that has to arrive from the room escape should still
/// arrive from off screen, which is the whole point of the ring move. When no
/// staging room has an out-of-view cell (the rat is boxed into a pocket), the
/// nearest room at all is the answer — still never a maze cell.
fn nearest_reachable_room_cell(
    level: &Level,
    scratch: &SpawnScratch,
    player: &Player,
    class: SizeClass,
    viewport: Vec2,
) -> Option<(Vec2, bool)> {
    let mut out: Option<(f32, Vec2)> = None;
    let mut near: Option<(f32, Vec2)> = None;
    for (i, room) in level.rooms.iter().enumerate() {
        if !room_staging_ok(scratch, i, room) {
            continue;
        }
        let d = room.center().dist_sq(player.pos);
        let better_out = out.is_none_or(|(bd, _)| d < bd);
        let better_near = near.is_none_or(|(nd, _)| d < nd);
        if !better_out && !better_near {
            continue;
        }
        if let Some(p) = room_open_cell(level, room, class, Some(player.pos), Some(&scratch.reach)) {
            if better_near {
                near = Some((d, p));
            }
            if better_out && !in_sim_view(p, player.pos, viewport) {
                out = Some((d, p));
            }
        }
    }
    out.map(|(_, p)| (p, true))
        .or_else(|| near.map(|(_, p)| (p, false)))
}

/// A wall-clear, reachable cell inside `room` for `class`, sampled from `rng`.
///
/// `None` when the room has no cell that fits the class with clearance (a room
/// crowded with props, or a `Small`-only room). Always takes the same number of
/// rng draws for the same outcome, and never loops forever.
pub fn spawn_point_in_room(
    rng: &mut StdRng,
    level: &Level,
    room: &Room,
    class: SizeClass,
) -> Option<Vec2> {
    let area = room.rect.inflate(-ROOM_INSET);
    if area.w <= 0 || area.h <= 0 {
        return None;
    }
    let clear = room_clearance(class);
    for _ in 0..ROOM_SAMPLES {
        let cx = rng.random_range(area.x..area.right());
        let cy = rng.random_range(area.y..area.bottom());
        if cell_in_maze(level, cx, cy) {
            continue;
        }
        let p = TileMap::tile_rect(cx, cy).center();
        if level.tiles.passable(cx, cy, class) && spawn_circle_clear(level, p, clear, class) {
            return Some(p);
        }
    }
    None
}

/// A room-wave point: like [`spawn_point_in_room`], but it must also be a real
/// approach — at least [`SPAWN_MIN_DIST`] from the rat and within
/// [`APPROACH_CELLS`] path cells of him — so the bodies that appear there
/// actually arrive. Draws the same fixed [`ROOM_SAMPLES`] pairs as
/// `spawn_point_in_room`, so the number of rng values a room wave consumes does
/// not depend on the map.
fn room_approach_point(
    rng: &mut StdRng,
    level: &Level,
    room: &Room,
    player: &Player,
    scratch: &SpawnScratch,
) -> Option<Vec2> {
    let area = room.rect.inflate(-ROOM_INSET);
    if area.w <= 0 || area.h <= 0 {
        return None;
    }
    let clear = room_clearance(SizeClass::Large);
    for _ in 0..ROOM_SAMPLES {
        let cx = rng.random_range(area.x..area.right());
        let cy = rng.random_range(area.y..area.bottom());
        if cell_in_maze(level, cx, cy) {
            continue;
        }
        let p = TileMap::tile_rect(cx, cy).center();
        if !level.tiles.passable(cx, cy, SizeClass::Large) {
            continue;
        }
        if !spawn_circle_clear(level, p, clear, SizeClass::Large) {
            continue;
        }
        if p.dist(player.pos) < SPAWN_MIN_DIST {
            continue;
        }
        if !scratch.is_reachable(level, p) {
            continue;
        }
        if scratch.approach_cells(level, p).is_none() {
            continue;
        }
        // The point itself must satisfy the staging rule, not just the room it
        // was sampled in: generated maps are merged open areas, so a point can
        // land in a *different* room that overlaps the chosen one (possibly the
        // rat's own, or one whose centre is out of wave range). Rejecting those
        // keeps "a wave is staged in a room the rat is not in, within
        // `ROOM_WAVE_RANGE`" true of the actual spawn point.
        match level.room_at(p) {
            Some(at)
                if !at.rect.contains_world(player.pos)
                    && at.center().dist(player.pos) <= ROOM_WAVE_RANGE => {}
            _ => continue,
        }
        return Some(p);
    }
    None
}

/// Two off-screen flanking groups: one per real approach of the chamber the rat
/// has been holding.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlankPlan {
    /// The two chamber openings, as world points just inside the room.
    pub anchors: [Vec2; 2],
}

impl FlankPlan {
    /// Where member `index` of the pulse appears: group `index % 2`, member
    /// `index / 2` of that group.
    ///
    /// No rng: the offsets are a fixed spiral from the doorway, so the pulse's
    /// enemies stay one pulse's worth of bodies (the angle draw still happened
    /// in [`spawn_pulse_point`]).
    pub fn point(
        &self,
        level: &Level,
        player: &Player,
        viewport: Vec2,
        index: usize,
        reach: Option<&[bool]>,
    ) -> Option<Vec2> {
        let anchor = self.anchors[index % 2];
        let member = index / 2;
        let (acx, acy) = cell_of(anchor);
        for step in 0..FLANK_OFFSETS.len() {
            let (dx, dy) = FLANK_OFFSETS[(member + step) % FLANK_OFFSETS.len()];
            let (cx, cy) = (acx + dx, acy + dy);
            if cell_in_maze(level, cx, cy) {
                continue;
            }
            if !level.tiles.passable(cx, cy, SizeClass::Large) {
                continue;
            }
            let p = TileMap::tile_rect(cx, cy).center();
            if !spawn_circle_clear(level, p, SPAWN_WALL_CLEAR, SizeClass::Large) {
                continue;
            }
            if p.dist(player.pos) < SPAWN_MIN_DIST {
                continue;
            }
            if !off_screen(p, player.pos, viewport) {
                continue;
            }
            // A cell beside the doorway can be on the far side of a wall from
            // it: only cells on the horde's own map (the anchor already is).
            if !cell_reachable(reach, level, p) {
                continue;
            }
            return Some(p);
        }
        // A crowded doorway: walk *outwards* along this group's own bearing
        // until the point is off the widest view, so a pinch still arrives from
        // out of sight instead of materialising in it. Never a maze cell, never
        // an unreachable one.
        let outward = (anchor - player.pos).normalized();
        for k in 1..=FLANK_OUTWARD_CELLS {
            let p = anchor + outward * (k as f32 * crate::game::collision::TILE_F);
            let (cx, cy) = cell_of(p);
            if cell_in_maze(level, cx, cy)
                || !level.tiles.passable(cx, cy, SizeClass::Large)
                || !spawn_circle_clear(level, p, SPAWN_WALL_CLEAR, SizeClass::Large)
                || p.dist(player.pos) < SPAWN_MIN_DIST
                || !off_screen(p, player.pos, viewport)
                || !cell_reachable(reach, level, p)
            {
                continue;
            }
            return Some(p);
        }
        // A crowded doorway, or nothing off screen along its bearing: no point
        // for this member. Skipping is the file's rule ("skip, never shove") —
        // the caller stages the body on the ring instead, which is always out of
        // sight; a flank body is never placed where the rat can see it appear.
        None
    }
}

/// Plan the two flanking groups for chamber `room_index`.
///
/// `None` unless the chamber offers **two real approaches**: openings on its
/// boundary that are reachable, at least [`SPAWN_MIN_DIST`] away, within
/// [`APPROACH_CELLS`] path cells, and at least [`FLANK_MIN_SEPARATION`] apart in
/// bearing — **and** both groups can actually be placed out of sight (see
/// [`FlankPlan::point`]: a pinch that materialises on screen is the pop-in this
/// batch removes). The pair with the widest bearing wins (deterministically;
/// ties keep the first).
///
/// The *doorway* itself may be on screen — it is a route marker, not a spawn
/// point, and a chamber small enough to show both its doors can still earn a
/// pinch whose bodies appear out in the dark on either side.
pub fn plan_flank(
    level: &Level,
    player: &Player,
    viewport: Vec2,
    room_index: usize,
    scratch: &mut SpawnScratch,
) -> Option<FlankPlan> {
    let room = *level.rooms.get(room_index)?;
    scratch.build_approach(level, player.pos);
    scratch.scan_openings(level, &room);
    let n = scratch.openings.len();
    let mut best: Option<(f32, Vec2, Vec2)> = None;
    for i in 0..n {
        let (ax, ay) = scratch.openings[i];
        let a = TileMap::tile_rect(ax, ay).center();
        if !flank_anchor_ok(level, player, scratch, a) {
            continue;
        }
        for j in (i + 1)..n {
            let (bx, by) = scratch.openings[j];
            let b = TileMap::tile_rect(bx, by).center();
            if !flank_anchor_ok(level, player, scratch, b) {
                continue;
            }
            let sep = angle_between(a - player.pos, b - player.pos);
            if sep < FLANK_MIN_SEPARATION {
                continue;
            }
            if best.is_none_or(|(bs, _, _)| sep > bs) {
                best = Some((sep, a, b));
            }
        }
    }
    let plan = best.map(|(_, a, b)| FlankPlan { anchors: [a, b] })?;
    // Both groups must really land out of sight, or there is no flank.
    for member in [0usize, 1] {
        let p = plan.point(level, player, viewport, member, Some(&scratch.reach))?;
        if !off_screen(p, player.pos, viewport) {
            return None;
        }
    }
    Some(plan)
}

/// Is this chamber opening a legal flanking approach?
///
/// Note what is **not** here: the doorway does not have to be off screen. The
/// bodies are placed by [`FlankPlan::point`], which walks out of sight along the
/// doorway's own bearing; requiring the doorway itself to be beyond the widest
/// view would have refused every pinch inside a small chamber.
fn flank_anchor_ok(level: &Level, player: &Player, scratch: &SpawnScratch, p: Vec2) -> bool {
    let (cx, cy) = cell_of(p);
    if cell_in_maze(level, cx, cy) {
        return false;
    }
    if !level.tiles.passable(cx, cy, SizeClass::Large) {
        return false;
    }
    if !spawn_circle_clear(level, p, room_clearance(SizeClass::Large), SizeClass::Large) {
        return false;
    }
    if p.dist(player.pos) < SPAWN_MIN_DIST {
        return false;
    }
    if !scratch.is_reachable(level, p) {
        return false;
    }
    scratch.approach_cells(level, p).is_some()
}

/// Angle between two vectors, in radians (`0` for a degenerate one).
fn angle_between(a: Vec2, b: Vec2) -> f32 {
    let (la, lb) = (a.len(), b.len());
    if la < 1e-3 || lb < 1e-3 {
        return 0.0;
    }
    (a.dot(b) / (la * lb)).clamp(-1.0, 1.0).dacos()
}

/// Clearance a room spawn keeps from any wall, per size class (a boss-sized
/// circle needs more room than the rat).
#[inline]
fn room_clearance(class: SizeClass) -> f32 {
    match class {
        SizeClass::Small => SPAWN_WALL_CLEAR + 1.0,
        SizeClass::Large => SPAWN_WALL_CLEAR + 5.0,
    }
}

/// The ring sampler shared by [`spawn_point`] and [`spawn_point_reachable`].
fn ring_base_angle(rng: &mut StdRng, player: &Player) -> f32 {
    // One rng draw per spawn point, exactly like the legacy ring: the angle. A
    // moving player biases the sweep to the direction he is heading, so the
    // horde comes from the front and the sides instead of from his back.
    let heading = player.vel;
    if heading.len_sq() > 1e-6 {
        heading.angle() + rng.random_range(-HEADING_SPREAD..HEADING_SPREAD)
    } else {
        rng.random_range(0.0..std::f32::consts::TAU)
    }
}

/// Sample the ring around the player; `None` when no candidate is legal.
///
/// Four passes over **fixed** candidate lists — the order never depends on the
/// rng, so the result is deterministic — each counted in `fb`:
///
/// 0. the [`ring_radius`] ring, out of sight at the widest zoom (the normal
///    case): `SpawnFallbacks::ring`;
/// 1. 1.5 ring radii out, when the ring itself is walled (`ring_outward`);
/// 2. the [`one_x_ring_radius`] ring — inward, nearer, still outside the
///    *unzoomed* view (`ring_inward`);
/// 3. any legal point on the ring (`ring_any`), which can be inside the widest
///    view: a rat in a one-cell corridor leaves nothing better, and the room
///    and nearest-cell escapes follow.
fn sample_ring(
    level: &Level,
    player: &Player,
    viewport: Vec2,
    base: f32,
    reach: Option<&[bool]>,
    fb: &mut SpawnFallbacks,
) -> Option<Vec2> {
    let outer = ring_radius(viewport);
    let lo = Vec2::splat(SPAWN_MARGIN);
    let hi = level.arena - Vec2::splat(SPAWN_MARGIN);
    let step = std::f32::consts::TAU / RING_ATTEMPTS as f32;
    for pass in 0..4 {
        let r = match pass {
            1 => outer * 1.5,
            2 => one_x_ring_radius(viewport),
            _ => outer,
        };
        for k in 0..=RING_ATTEMPTS {
            // The last candidate mirrors the first angle: a cheap second sweep.
            let a = if k == RING_ATTEMPTS {
                -base
            } else {
                base + k as f32 * step
            };
            let p = player.pos + Vec2::from_angle(a) * r;
            let out_of_sight = match pass {
                0 | 1 => off_screen(p, player.pos, viewport),
                2 => !in_sim_view(p, player.pos, viewport),
                _ => true,
            };
            if !out_of_sight {
                continue;
            }
            if !spawn_point_ok(p, lo, hi, SPAWN_MIN_DIST, SPAWN_WALL_CLEAR, level, player.pos) {
                continue;
            }
            if !cell_reachable(reach, level, p) {
                continue;
            }
            match pass {
                0 => fb.ring += 1,
                1 => fb.ring_outward += 1,
                2 => fb.ring_inward += 1,
                _ => fb.ring_any += 1,
            }
            return Some(p);
        }
    }
    None
}

/// The outermost fallback: a ring too tight for any legal point leaves the
/// nearest wall-clear cell (reachable when a table is supplied), first at the
/// normal minimum distance and then as close as the rat's own body allows.
/// Clamping the ring point is the very last resort, exactly like the legacy
/// ring.
fn fallback_point(
    level: &Level,
    player: &Player,
    viewport: Vec2,
    base: f32,
    reach: Option<&[bool]>,
) -> Option<Vec2> {
    let p = player.pos + Vec2::from_angle(base) * ring_radius(viewport);
    for min_dist in [SPAWN_MIN_DIST, NEAR_SPAWN_DIST] {
        if let Some(q) = nearest_clear(
            level,
            p,
            SizeClass::Large,
            FALLBACK_SEARCH_CELLS,
            reach,
            Some(player.pos),
            min_dist,
        ) {
            return Some(q);
        }
    }
    None
}

/// The void primitive's very last resort ([`spawn_point`], the facade's test
/// hook): a ring point clamped inside the arena. The director never uses this —
/// it skips the spawn instead ([`spawn_point_reachable`]).
fn last_resort(level: &Level, player: &Player, viewport: Vec2, base: f32) -> Vec2 {
    let p = player.pos + Vec2::from_angle(base) * ring_radius(viewport);
    p.clamp_box(Vec2::splat(6.0), level.arena - Vec2::splat(6.0))
}

/// Is `p` outside the **widest** view the player can be looking at (the
/// [`widest_view`] rectangle, plus [`OFF_SCREEN_MARGIN`])? The camera is
/// centred on the player, so this is a plain box test around him — and it is
/// the *widest* box, so a spawn is out of sight at every rung of the zoom
/// ladder instead of only at the unzoomed one.
#[inline]
pub(super) fn off_screen(p: Vec2, player: Vec2, viewport: Vec2) -> bool {
    let view = widest_view(viewport);
    let d = p - player;
    let hw = view.x * 0.5 + OFF_SCREEN_MARGIN;
    let hh = view.y * 0.5 + OFF_SCREEN_MARGIN;
    d.x.abs() > hw || d.y.abs() > hh
}

/// Reachability table lookup, `true` when no table is supplied (the cache-free
/// [`spawn_point`] path).
#[inline]
fn cell_reachable(reach: Option<&[bool]>, level: &Level, p: Vec2) -> bool {
    let Some(reach) = reach else {
        return true;
    };
    let (cx, cy) = cell_of(p);
    // An off-map column must not wrap into the neighbouring row's index.
    if !level.tiles.in_bounds(cx, cy) {
        return false;
    }
    reach
        .get((cy * level.tiles.cols() + cx) as usize)
        .copied()
        .unwrap_or(false)
}

/// Nearest wall-clear cell centre to `p`, searching ring by ring. With a
/// reachability table (the director's path) only a cell the horde can reach is
/// ever returned — `None` means skip the spawn, never a body in a sealed
/// pocket. Without one (the cache-free [`spawn_point`] primitive) plain
/// clearance decides, so the result is never inside a wall. `min_from` is
/// the keep-away point (the player): a cell closer than `min_dist` to it is
/// never chosen, so the fallback cannot drop an enemy on top of the rat.
fn nearest_clear(
    level: &Level,
    p: Vec2,
    class: SizeClass,
    max_cells: i32,
    reach: Option<&[bool]>,
    min_from: Option<Vec2>,
    min_dist: f32,
) -> Option<Vec2> {
    let (hx, hy) = cell_of(p);
    // One pass: the table decides whenever there is one. (There used to be a
    // second, table-free pass that could hand the director an unreachable
    // cell.)
    let require_reach = reach.is_some();
    {
        for r in 0..=max_cells {
            for dy in -r..=r {
                for dx in -r..=r {
                    if r > 0 && dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    let (cx, cy) = (hx + dx, hy + dy);
                    if !level.tiles.in_bounds(cx, cy) {
                        continue;
                    }
                    if cell_in_maze(level, cx, cy) {
                        continue;
                    }
                    let c = TileMap::tile_rect(cx, cy).center();
                    if let Some(from) = min_from {
                        if c.dist_sq(from) < min_dist * min_dist {
                            continue;
                        }
                    }
                    if require_reach && !cell_reachable(reach, level, c) {
                        continue;
                    }
                    if spawn_circle_clear(level, c, SPAWN_WALL_CLEAR, class) {
                        return Some(c);
                    }
                }
            }
        }
    }
    None
}

/// `World::spawn_point_ok` .
pub fn spawn_point_ok(
    p: Vec2,
    lo: Vec2,
    hi: Vec2,
    min_dist: f32,
    wall_clear: f32,
    level: &Level,
    player_pos: Vec2,
) -> bool {
    if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y {
        return false;
    }
    if p.dist_sq(player_pos) < min_dist * min_dist {
        return false;
    }
    // A single tile-grid query instead of a walk over every wall: only the
    // cells within `wall_clear + 16` px are consulted, so the cost does not
    // grow with the map. The horde is the `Large` class, so a rat-hole cell
    // reads as a solid face here and no enemy materialises in a hole the
    // player could duck through.
    if !spawn_circle_clear(level, p, wall_clear, SizeClass::Large) {
        return false;
    }
    // Never inside a maze's ring or interior: the chase begins at the mouth.
    !in_maze(level, p)
}

fn spawn_circle_clear(level: &Level, at: Vec2, radius: f32, class: SizeClass) -> bool {
    if level.theme == Theme::Outfall {
        crate::game::hazards::outfall_sand_clear(level, at, radius, class)
    } else {
        level.tiles.circle_clear(at, radius, class)
    }
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::RngExt;
    use rand::SeedableRng;

    use super::{
        director_system, in_maze, plan_flank, ring_radius, spawn_point, spawn_point_in_room,
        spawn_point_ok, spawn_point_reachable, SpawnScratch, APPROACH_CELLS,
    };
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{circle_rect_overlap, SizeClass, TileKind, TileMap, TILE_F};
    use crate::game::director;
    use crate::game::ecs::constants::ENEMY_CAP;
    use crate::game::ecs::resources::BossDue;
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::{Level, Rectf, RoomKind, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::loadout::Loadout;
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;
    use crate::game::world::World;
    use crate::math::Vec2;

    /// The director's last fallback never leaves the horde's map: with a
    /// reachability table that says "nowhere", there is no point at all, while
    /// the table-free primitive still finds plain clear floor.
    #[test]
    fn the_fallback_never_returns_a_cell_off_the_horde_map() {
        let mut w = World::new(Theme::Sewer, 13);
        open_level(&mut w.level);
        w.level.tiles.rebuild_masks();
        let cells = (w.level.tiles.cols() * w.level.tiles.rows()) as usize;
        let nowhere = vec![false; cells];
        let p = w.level.arena * 0.5;
        assert!(
            super::nearest_clear(&w.level, p, SizeClass::Large, 10, Some(&nowhere), None, 0.0)
                .is_none(),
            "an unreachable cell was offered to the director"
        );
        assert!(
            super::nearest_clear(&w.level, p, SizeClass::Large, 10, None, None, 0.0).is_some()
        );
        // The room escape obeys the same table, cell by cell.
        let room = crate::game::level::Room::new(
            crate::game::level::TileRect::new(20, 20, 12, 12),
            RoomKind::Room,
        );
        assert!(super::room_open_cell(&w.level, &room, SizeClass::Large, None, Some(&nowhere))
            .is_none());
        assert!(super::room_open_cell(&w.level, &room, SizeClass::Large, None, None).is_some());
    }

    /// A squad's first placed body is its leader even when the first ring
    /// offset is walled off, and the count is what really fits.
    #[test]
    fn a_squad_with_a_blocked_first_offset_still_has_a_leader() {
        use crate::game::ecs::systems::enemy_wave4 as w4;
        let mut w = World::new(Theme::Sewer, 17);
        open_level(&mut w.level);
        // The centre sits just left of a cell boundary; the cell to its right
        // (where offset 0, +7 px along x, lands) is solid.
        let (ccx, ccy) = (60, 60);
        let centre = Vec2::new((ccx + 1) as f32 * TILE_F - 3.0, ccy as f32 * TILE_F + TILE_F * 0.5);
        w.level
            .tiles
            .fill_cells(ccx + 1, ccy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        let mut first: Option<(usize, Vec2)> = None;
        let mut indices = Vec::new();
        let n = super::lilli_squad_points(&w.level, centre, 10, |j, at| {
            if first.is_none() {
                first = Some((j, at));
            }
            indices.push(j);
        });
        assert!(n > 0 && n < 10, "the wall should block some of the ring, not all ({n})");
        let (j0, at0) = first.expect("someone fits");
        assert_eq!(j0, 0);
        assert!(at0.x < (ccx + 1) as f32 * TILE_F, "the leader is not in the wall");
        assert_eq!(indices, (0..n).collect::<Vec<_>>(), "placed indices are compact");
        assert_eq!(super::lilli_role(0), w4::LILLI_ROLE_LEADER);
        assert_ne!(super::lilli_role(1), w4::LILLI_ROLE_LEADER);
    }

    /// A flank member is never placed on a cell the horde cannot reach: with a
    /// table that says "nowhere" every offset is refused and the group falls
    /// back to its (validated) anchor.
    #[test]
    fn a_flank_member_stays_on_the_horde_map() {
        let mut w = World::new(Theme::Sewer, 19);
        open_level(&mut w.level);
        w.level.tiles.rebuild_masks();
        let anchor = TileMap::tile_rect(40, 40).center();
        let plan = super::FlankPlan { anchors: [anchor, anchor] };
        w.player.pos = anchor + Vec2::new(600.0, 0.0);
        let viewport = Vec2::new(120.0, 80.0);
        let cells = (w.level.tiles.cols() * w.level.tiles.rows()) as usize;
        let nowhere = vec![false; cells];
        let mut only = vec![false; cells];
        // One reachable cell, two to the right of the anchor.
        only[(40 * w.level.tiles.cols() + 42) as usize] = true;
        for member in 0..6 {
            // No reachable cell anywhere: no member at all — a body is never
            // dropped where the rat can see it appear (the ring stages it).
            assert_eq!(
                plan.point(&w.level, &w.player, viewport, member, Some(&nowhere)),
                None
            );
            assert_eq!(
                plan.point(&w.level, &w.player, viewport, member, Some(&only)),
                Some(TileMap::tile_rect(42, 40).center())
            );
        }
    }

    /// A point off the map is never "reachable": a negative column used to wrap
    /// into the previous row's last cell (`cy * cols + cx` with `cx = -1`).
    #[test]
    fn a_point_off_the_map_is_never_reachable() {
        let w = World::new(Theme::Sewer, 11);
        let mut scratch = SpawnScratch::default();
        scratch.sync(&w.level);
        // Pretend every cell is on the horde's map, so only the bounds test
        // can say no.
        scratch.reach.iter_mut().for_each(|c| *c = true);
        for row in 1..w.level.tiles.rows() {
            let y = row as f32 * TILE_F + TILE_F * 0.5;
            assert!(!scratch.is_reachable(&w.level, Vec2::new(-TILE_F * 0.5, y)));
            assert!(!super::cell_reachable(
                Some(&scratch.reach),
                &w.level,
                Vec2::new(-TILE_F * 0.5, y)
            ));
        }
        assert!(scratch.is_reachable(&w.level, Vec2::new(TILE_F * 0.5, TILE_F * 0.5)));
    }

    /// Strip the generated layout so the ring has room (what
    /// `Level::walls.clear()` used to do before the tile grid owned
    /// collision).
    fn open_level(level: &mut Level) {
        level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, level.theme, 0);
        // The maze exclusion reads `Level::mazes`, not the tiles, so a blank
        // map has to clear them as well.
        level.mazes.clear();
        level.rooms.clear();
    }

    /// Drive exactly one director tick at a chosen run clock, exactly the way
    /// the real schedule does (`World::update` sets `elapsed` before
    /// `begin_tick`).
    fn director_tick(w: &mut World, dt: f32, elapsed: f32) {
        w.elapsed = elapsed;
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(director_system)
            .expect("director system must run");
        w.end_tick();
    }

    #[test]
    fn pressure_keeps_spawn_totals_and_xp_supply_close_with_room_to_spawn() {
        let enabled = crate::tuning::RunTuning::default();
        let disabled = crate::tuning::RunTuning::new(
            crate::tuning::Tuning::parse("[director]\npressure_beats = false").unwrap(),
        );
        for (sector, theme) in [Theme::Sewer, Theme::Kitchen, Theme::Lab, Theme::Trailer]
            .into_iter().enumerate()
        {
            let measure = |tuning| {
                let mut world = World::new_level_tuned(
                    sector, theme, 61, Loadout::starting(), Default::default(), tuning,
                );
                let dt = 1.0 / 60.0;
                let mut bodies = 0usize;
                let mut xp = 0u32;
                for frame in 1..54_000 {
                    director_tick(&mut world, dt, frame as f32 * dt);
                    if world.enemy_count() != 0 {
                        let spawned = world.enemies();
                        bodies += spawned.len();
                        xp += spawned.iter().map(|e| world.tuning().enemies.kinds[&e.kind].xp).sum::<u32>();
                        // Clear each pulse so both runs have the same space to
                        // spawn. This isolates supply from fighting and pickups.
                        world.clear_enemies();
                    }
                }
                (bodies, xp as usize)
            };
            let (base_bodies, base_xp) = measure(disabled.clone());
            let (beat_bodies, beat_xp) = measure(enabled.clone());
            assert!(base_bodies.abs_diff(beat_bodies) <= base_bodies / 20,
                "{theme:?} bodies: {base_bodies} → {beat_bodies}");
            assert!(base_xp.abs_diff(beat_xp) <= base_xp / 20,
                "{theme:?} potential XP: {base_xp} → {beat_xp}");
        }
    }

    #[test]
    fn practice_director_uses_sector_one_overrides_on_a_later_floor() {
        let tuning = crate::tuning::Tuning::parse(r#"
            [director.sectors.1]
            live_budget = [[0, 0], [150, 0], [300, 0], [450, 0], [600, 0], [750, 0], [900, 0]]
            spawn_interval = [[0, 2.5], [150, 2.5], [300, 2.5], [450, 2.5], [600, 2.5], [750, 2.5], [900, 2.5]]
            elite_start = 300.0
        "#).unwrap();
        let run = crate::tuning::RunTuning::new(tuning);
        let make_world = |tuning| World::new_level_tuned(
            3, Theme::Trailer, 61, Loadout::starting(), Default::default(), tuning,
        );
        let mut campaign = make_world(run.clone());
        let mut practice = make_world(run);
        practice.difficulty_override = Some(0);

        let at = 255.0;
        director_tick(&mut campaign, 0.1, at);
        director_tick(&mut practice, 0.1, at);
        assert_eq!(campaign.director.spawn_timer, campaign.tuning().director.horde_interval(3, at, false));
        assert!(campaign.director.elite_timer > 0.0);
        assert_eq!(practice.director.spawn_timer, 2.5);
        assert!(practice.director.elite_timer <= 0.0);
        assert!(practice.enemies().is_empty());
    }

    /// Legacy `spawn_point_stays_in_bounds_clear_of_walls_and_player`: the ring
    /// stays inside the 8 px margin, at least 90 px from the player and 4 px
    /// clear of a wall.
    #[test]
    fn spawn_point_respects_margins_distance_and_walls() {
        let mut level = Level::new(Theme::Sewer);
        let centre = level.arena * 0.5;
        let player = Player::new(centre, WeaponKind::Gnaw);
        open_level(&mut level);
        // A 2 x 2-cell solid block out on the 120 px ring, east of the rat.
        let (ccx, ccy) = crate::game::collision::cell_of(centre);
        level
            .tiles
            .fill_cells(ccx + 7, ccy - 2, 2, 2, TileKind::Wall(WallKind::Solid));
        level.tiles.rebuild_masks();
        let block = {
            let r = TileMap::tile_rect(ccx + 7, ccy - 2);
            Rectf::new(r.x, r.y, 2.0 * TILE_F, 2.0 * TILE_F)
        };
        // The facade default viewport, so the ring radius is the legacy 120 px.
        let viewport = Vec2::new(160.0, 80.0);
        let mut rng = StdRng::seed_from_u64(13);
        for _ in 0..300 {
            let p = spawn_point(&mut rng, &level, &player, viewport);
            assert!(
                p.x >= 8.0 - 1e-3 && p.y >= 8.0 - 1e-3,
                "spawn {p:?} past min margin"
            );
            assert!(
                p.x <= level.arena.x - 8.0 + 1e-3 && p.y <= level.arena.y - 8.0 + 1e-3,
                "spawn {p:?} past max margin"
            );
            assert!(
                p.dist(centre) >= 90.0 - 1e-3,
                "spawn {p:?} too close to player"
            );
            assert!(
                !circle_rect_overlap(p, 4.0 - 1e-3, &block),
                "spawn {p:?} overlaps a wall"
            );
        }
        // The viewport argument drives the radius: `len() * 0.5 + 16`, floor
        // 120 px . A cleared level accepts the whole ring, so
        // the distance is exact.
        open_level(&mut level);
        let small = spawn_point(&mut rng, &level, &player, Vec2::splat(1.0));
        assert!(
            (small.dist(centre) - 120.0).abs() < 1e-3,
            "ring radius floored wrong: {small:?}"
        );
        let wide = spawn_point(&mut rng, &level, &player, Vec2::new(600.0, 400.0));
        let r = (wide - centre).len();
        let ring = super::widest_view(Vec2::new(600.0, 400.0)).len() * 0.5 + 16.0;
        assert!(
            (r - ring).abs() < 1e-3,
            "ring radius does not follow the widest view: {r} (wanted {ring})"
        );
    }

    /// Property over many generated seeds: `spawn_point_ok` agrees with the
    /// tile grid — it rejects every solid or maze cell's centre, and it accepts
    /// open floor far enough from the player.
    #[test]
    fn spawn_point_ok_matches_the_tile_grid_on_every_map() {
        let lo = Vec2::splat(8.0);
        for theme in Theme::all() {
            for seed in 0..8u64 {
                let level = Level::generate(theme, seed);
                let hi = level.arena - Vec2::splat(8.0);
                let mut rejected = 0;
                for (cx, cy, kind) in level.tiles.iter_walls() {
                    if !matches!(kind, WallKind::Solid | WallKind::Maze) {
                        continue;
                    }
                    let c = TileMap::tile_rect(cx, cy).center();
                    if c.x < 16.0 || c.y < 16.0 || c.x > hi.x - 8.0 || c.y > hi.y - 8.0 {
                        continue;
                    }
                    assert!(
                        !spawn_point_ok(c, lo, hi, 0.0, 4.0, &level, c + Vec2::new(200.0, 0.0)),
                        "{theme:?} seed {seed}: accepted a wall cell centre at {c:?}"
                    );
                    rejected += 1;
                }
                assert!(rejected > 100, "{theme:?} seed {seed}: map has almost no walls");

                // Twenty open cell centres far from the player are accepted.
                let player = level.arena * 0.5;
                let clear_floor = |cx: i32, cy: i32| {
                    let c = TileMap::tile_rect(cx, cy).center();
                    c.dist(player) >= 200.0
                        && level.tiles.passable(cx, cy, SizeClass::Large)
                        && level.tiles.circle_clear(c, 4.0, SizeClass::Large)
                };
                let mut accepted = 0;
                for cx in (4..level.tiles.cols() - 4).step_by(7) {
                    for cy in (4..level.tiles.rows() - 4).step_by(7) {
                        if clear_floor(cx, cy) {
                            accepted += 1;
                        }
                    }
                    if accepted >= 20 {
                        break;
                    }
                }
                assert!(accepted >= 20, "{theme:?} seed {seed}: no clear floor found");
            }
        }
    }

    /// A ring that cannot find a legal point (arena smaller than the ring) is
    /// clamped to the 6 px last resort, never left outside the arena.
    #[test]
    fn spawn_point_clamps_when_the_ring_cannot_fit() {
        let level = Level::new(Theme::Sewer);
        let player = Player::new(level.arena * 0.5, WeaponKind::Gnaw);
        let mut rng = StdRng::seed_from_u64(3);
        // A 2000 px viewport pushes the ring far outside the arena.
        for _ in 0..50 {
            let p = spawn_point(&mut rng, &level, &player, Vec2::splat(2000.0));
            assert!(
                p.x >= 6.0 && p.y >= 6.0 && p.x <= level.arena.x - 6.0 && p.y <= level.arena.y - 6.0,
                "clamped spawn {p:?} left the arena"
            );
        }
    }

    /// `World::spawn_point` (the facade hook used by the ported legacy test) is
    /// the same free function with the facade rng/level/player/viewport.
    #[test]
    fn facade_spawn_point_matches_the_system_helper() {
        let mut w = World::new(Theme::Sewer, 11);
        let mut rng = StdRng::seed_from_u64(11);
        for _ in 0..20 {
            let expected = spawn_point(&mut rng, &w.level, &w.player, w.viewport);
            let actual = w.spawn_point();
            assert_eq!(expected, actual);
        }
    }

    /// One pulse that would overflow the cap must stop at the cap *within* the
    /// tick: `Commands` are deferred, so the system has to count its own queued
    /// spawns instead of re-reading the enemy count.
    #[test]
    fn a_pulse_never_overfills_the_cap_between_deferred_spawns() {
        let mut w = World::new(Theme::Sewer, 5);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 4) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        assert_eq!(w.enemies().len(), ENEMY_CAP - 4);
        // Force a pulse this tick. At 100 s the live body budget is 42 and the
        // map already holds far more than that, so the director must add
        // nothing — and it must never overshoot the cap either.
        w.director.spawn_timer = 0.0;
        director_tick(&mut w, 0.1, 100.0);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 4, "an overspent pulse spawned");
        assert!(w.enemies().len() < ENEMY_CAP);
        assert!(!w.director.boss_spawned);
    }

    /// The reserved slot holds for the whole run and is only released when the
    /// boss becomes due (the boss framework then spends it on the boss itself).
    #[test]
    fn director_reserves_one_slot_until_the_boss_is_due() {
        let mut w = World::new(Theme::Sewer, 17);
        let dt = 0.1;
        let mut elapsed = 0.0;
        while elapsed + dt < director::BOSS_TIME {
            elapsed += dt;
            director_tick(&mut w, dt, elapsed);
            assert!(
                w.enemies().len() < ENEMY_CAP,
                "cap exceeded at {elapsed:.1}s: {}",
                w.enemies().len()
            );
            assert!(!w.director.boss_spawned, "the boss came early");
        }
        // At the boss time the hand-off releases the slot for the boss itself.
        director_tick(&mut w, dt, director::BOSS_TIME);
        assert!(w.director.boss_spawned);
        assert!(w.ecs.resource::<BossDue>().is_pending());
        assert!(w.enemies().len() <= ENEMY_CAP);
    }

    /// The boss is handed over, not spawned: at `BOSS_TIME` the director raises
    /// `BossDue`, sets `boss_spawned` (so the 25 s warning still closes) and
    /// spawns no boss of its own — and no longer plays the arrival either.
    #[test]
    fn the_boss_is_handed_over_instead_of_spawned() {
        let mut w = World::new(Theme::Sewer, 23);
        // A dying-screen shake from earlier must not be overwritten by a boss.
        w.shake = 40.0;
        w.sfx.clear();
        director_tick(&mut w, 0.1, director::BOSS_TIME - 0.1);
        assert!(!w.director.boss_spawned, "the boss came early");
        assert!(!w.ecs.resource::<BossDue>().due);

        director_tick(&mut w, 0.1, director::BOSS_TIME);
        assert!(w.director.boss_spawned);
        let due = *w.ecs.resource::<BossDue>();
        assert!(due.is_pending(), "the boss signal was not raised");
        assert_eq!(due.level, 0);
        assert!((due.at - director::BOSS_TIME).abs() < 1e-3);
        assert!(
            !w.enemies().iter().any(|e| e.kind == EnemyKind::Boss),
            "the director still spawns a generic boss"
        );
        // The boss framework owns the arrival feedback now.
        assert_eq!(w.shake, 40.0);
        assert!(!w.sfx.contains(&Sfx::Boss));
    }

    /// The signal is raised once and consumed once; the boss framework's
    /// `take` is the only thing that clears it.
    #[test]
    fn the_boss_signal_is_raised_and_claimed_once() {
        let mut w = World::new_level(1, 24, Loadout::starting());
        director_tick(&mut w, 0.1, director::BOSS_TIME);
        director_tick(&mut w, 0.1, director::BOSS_TIME + 3.0);
        {
            let mut due = w.ecs.resource_mut::<BossDue>();
            assert!(due.take(), "the first take must hand the boss over");
            assert!(!due.take(), "the signal was handed over twice");
            assert!(!due.is_pending());
        }
        // Never raised again, however long the fight lasts.
        director_tick(&mut w, 0.1, director::BOSS_TIME + 400.0);
        assert!(!w.ecs.resource::<BossDue>().is_pending());
        assert_eq!(w.ecs.resource::<BossDue>().level, 1);
    }

    /// After the hand-off the horde keeps coming at the level's late-curve rate
    /// — the curve froze at `BOSS_TIME` instead of running away — and the cap
    /// still holds.
    ///
    /// The boss phase *thins* the pulses (`BOSS_HORDE_INTERVAL_MULT` 2 ×
    /// `BOSS_HORDE_PULSE_DIV` 2, the balance pass that made the fight about the
    /// boss), so a 24 s window no longer fills the map: this test protects that
    /// the horde never *stops*, that the thinning is real, and that the 600 cap
    /// still holds once the phase runs long enough to fill it.
    #[test]
    fn the_horde_keeps_coming_after_the_boss_is_due() {
        let mut w = World::new(Theme::Sewer, 25);
        director_tick(&mut w, 0.1, director::BOSS_TIME);
        let before = w.enemies().len();
        assert!(before > 0, "no horde arrived on the boss tick");
        let mut peak = before;
        for k in 1..=120 {
            director_tick(&mut w, 0.2, director::BOSS_TIME + k as f32 * 0.2);
            peak = peak.max(w.enemies().len());
        }
        assert!(
            w.enemies().len() > before,
            "the horde stopped spawning after the boss was due"
        );
        assert!(
            w.enemies().len() <= director::CLEANUP_LIVE_BUDGET,
            "the boss-phase trickle exceeded the cleanup budget: {} bodies",
            w.enemies().len()
        );
        // Given four more minutes of boss phase it reaches the cleanup budget
        // and never past it, and the 600-body cap is never touched.
        for k in 1..=1200 {
            director_tick(&mut w, 0.2, director::BOSS_TIME + 24.0 + k as f32 * 0.2);
            peak = peak.max(w.enemies().len());
        }
        assert_eq!(
            peak, director::CLEANUP_LIVE_BUDGET,
            "the boss-phase horde never reached the cleanup budget"
        );
        assert!(w.enemies().len() <= ENEMY_CAP, "the cap slipped");
    }

    /// Elites are gated on `elapsed > ELITE_START` (strictly greater), spawn
    /// the floor's own heavy (its H slot) at 1.6× the current hp mult and reset
    /// the timer to the interval.
    #[test]
    fn elites_start_after_the_gate_and_scale_their_hp() {
        let mut w = World::new(Theme::Sewer, 29);
        w.director.elite_timer = 0.0;
        // Exactly at the gate: no elite (legacy `elapsed > ELITE_START`).
        director_tick(&mut w, 0.0, director::ELITE_START);
        assert!(!w.enemies().iter().any(|e| e.kind == EnemyKind::SluiceToad));

        // Past the gate but inside the first appointment's window: the due
        // elite is held (one named threat at a time), and its timer stays
        // spent so it fires the moment the window closes.
        director_tick(&mut w, 0.1, director::ELITE_START + 0.1);
        assert!(director::near_appointment(director::ELITE_START + 0.1));
        assert!(!w.enemies().iter().any(|e| e.elite), "an elite shared the miniboss's arrival");
        assert!(w.director.elite_timer <= 0.0, "a held elite must stay due");

        let open = director::MINIBOSS_TIMES[0] + director::ELITE_APPOINTMENT_GAP + 0.1;
        assert!(!director::near_appointment(open));
        director_tick(&mut w, 0.1, open);
        let enemies = w.enemies();
        let brute = enemies
            .iter()
            .find(|e| e.elite)
            .expect("an elite heavy must spawn once the appointment window closes");
        // The elite wears the floor's H body (`WAVE4_ROSTERS.md`: "Elite uses
        // the local H body"), which on the Sewer is the Sluice Toad.
        assert_eq!(
            brute.kind,
            crate::game::roster::kind(Theme::Sewer, crate::game::roster::Slot::H)
        );
        assert_eq!(brute.kind, EnemyKind::SluiceToad);
        assert_eq!(
            w.director.elite_timer,
            w.tuning().director.elite_interval_at(0, w.tuning().director.curve_clock(0, open))
        );
        let expected = crate::game::roster::kind(Theme::Sewer, crate::game::roster::Slot::H)
            .base_hp()
            // The horde's own curve: the shared ramp times level one's
            // `LEVEL_HP_EXTRA` (which starts at the same 3:00 gate).
            * w.tuning().horde_hp_mult_for(0, open)
            * 1.6;
        assert!((brute.max_hp - expected).abs() < 1e-3, "hp = {}", brute.hp);
    }

    /// No elite spawns within `ELITE_APPOINTMENT_GAP` of any appointment, on
    /// either side, however long its timer has been spent.
    #[test]
    fn no_elite_spawns_inside_an_appointment_window() {
        for &at in director::MINIBOSS_TIMES {
            for offset in [-19.9f32, -10.0, 0.0, 10.0, 19.9] {
                // Skip instants before the elite gate itself.
                if at + offset <= director::ELITE_START {
                    continue;
                }
                let mut w = World::new(Theme::Sewer, 29);
                w.director.elite_timer = 0.0;
                // Keep the ordinary pulse out of the picture.
                w.director.spawn_timer = 1.0e6;
                director_tick(&mut w, 0.1, at + offset);
                assert!(
                    !w.enemies().iter().any(|e| e.elite),
                    "an elite spawned {offset:+} s from the {at} s appointment"
                );
            }
            let mut w = World::new(Theme::Sewer, 29);
            w.director.elite_timer = 0.0;
            w.director.spawn_timer = 1.0e6;
            director_tick(&mut w, 0.1, at + director::ELITE_APPOINTMENT_GAP + 0.5);
            assert!(
                w.enemies().iter().any(|e| e.elite),
                "the held elite never arrived after the {at} s appointment"
            );
        }
    }

    /// The elite branch never takes the boss's reserved slot.
    #[test]
    fn elites_are_skipped_when_only_the_boss_slot_is_left() {
        let mut w = World::new(Theme::Sewer, 31);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 1) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        w.director.elite_timer = 0.0;
        let open = director::MINIBOSS_TIMES[0] + director::ELITE_APPOINTMENT_GAP + 1.0;
        director_tick(&mut w, 0.1, open);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 1, "elite took the boss slot");
        assert!(!w.enemies().iter().any(|e| e.kind == EnemyKind::SluiceToad));
    }

    /// A tick that both pulses *and* crosses the boss time still respects the
    /// cap and still hands the boss over.
    #[test]
    fn a_late_tick_blocked_at_the_cap_still_hands_the_boss_over() {
        let mut w = World::new(Theme::Sewer, 43);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 1) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        w.director.spawn_timer = 0.0;
        director_tick(&mut w, 500.0, director::BOSS_TIME);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 1, "the pulse spent the reserved slot");
        assert!(w.director.boss_spawned);
        assert!(w.ecs.resource::<BossDue>().is_pending());
    }

    /// **One run per floor: the roster decides the kinds.** A floor only ever
    /// spawns its own cast (the Drake is the Sewer's guest, on no slot by
    /// design). The same-seed-twice half of this test lived here as a twin run;
    /// that rule is proven once per floor by
    /// `ecs::tests::the_same_seed_plays_the_same_run` now, so this is one world
    /// and 120 s of director time instead of two worlds and 600 s.
    #[test]
    fn every_floor_only_spawns_its_own_roster() {
        let seed = 41;
        for theme in Theme::playable() {
            let mut a = World::new(theme, seed);
            let dt = 0.5;
            let mut elapsed = 0.0;
            let mut kinds: Vec<EnemyKind> = Vec::new();
            for _ in 0..240 {
                elapsed += dt;
                director_tick(&mut a, dt, elapsed);
                for x in a.enemies() {
                    if !kinds.contains(&x.kind) {
                        kinds.push(x.kind);
                    }
                }
            }
            assert!(a.enemies().len() > 10, "{theme:?}: the run spawned almost nothing");
            // And it only ever spawned its own cast.
            let floor = crate::game::roster::floor(theme);
            for kind in kinds {
                // The Drake is the Sewer's guest: on no roster slot by design.
                if kind == EnemyKind::Drake && theme == Theme::Sewer {
                    continue;
                }
                assert!(
                    floor.slot_of(kind).is_some(),
                    "{theme:?} ran the {:?} roster but spawned {kind:?}",
                    theme
                );
            }
        }
    }

    /// A practice run (`difficulty_override = Some(0)`) keeps the level-0
    /// scalars *and* the floor it is played on: the Flatbed's own cast, at the
    /// Sewer's hp, speed and elite numbers.
    #[test]
    fn practice_scalars_do_not_change_the_floors_cast() {
        let mut w = World::new(Theme::Trailer, 61);
        w.difficulty_override = Some(0);
        let floor = crate::game::roster::floor(Theme::Trailer);
        let mut elapsed = 0.0;
        let mut first_spawned_at: Option<f32> = None;
        for _ in 0..120 {
            elapsed += 0.5;
            let before = w.enemies().len();
            director_tick(&mut w, 0.5, elapsed);
            if first_spawned_at.is_none() && w.enemies().len() > before {
                first_spawned_at = Some(elapsed);
            }
        }
        assert!(!w.enemies().is_empty(), "the practice run spawned nothing");
        let spawned_at = first_spawned_at.expect("a body appeared");
        assert!(
            spawned_at < director::ELITE_START,
            "the first body must precede the elite gate: {spawned_at}"
        );
        // The scalars are level 0's: hp is the kind's own pool times the level-0
        // curve at spawn, with no per-stage extra.
        let expected_mult = w.tuning().horde_hp_mult_for(0, spawned_at);
        let spawned = w.enemies();
        for e in spawned.iter() {
            assert!(
                floor.slot_of(e.kind).is_some(),
                "practice spawned {:?}, which is not on the Flatbed",
                e.kind
            );
        }
        // Every body spawned on the first tick shares the multiplier.
        if let Some(e) = spawned.first() {
            let expected = e.kind.base_hp() * expected_mult;
            assert!(
                (e.max_hp - expected).abs() < 1e-2,
                "{:?} has {} hp, level 0 says {expected}",
                e.kind,
                e.max_hp
            );
        }
    }

    /// The elite wears the floor's own heavy (the roster's H slot), not a
    /// hard-coded Brute, and it keeps the 1.6× pool and the elite flag.
    #[test]
    fn the_elite_is_the_floors_heavy() {
        for theme in Theme::playable() {
            let heavy = crate::game::roster::elite_kind(theme);
            let mut w = World::new(theme, 67);
            w.director.elite_timer = 0.0;
            w.director.spawn_timer = 1.0e6; // keep the ordinary pulse out of it
            let open = director::MINIBOSS_TIMES[0] + director::ELITE_APPOINTMENT_GAP + 0.5;
            director_tick(&mut w, 0.1, open);
            let snapshot = w.enemies();
            let elites: Vec<&Enemy> = snapshot.iter().filter(|e| e.elite).collect();
            assert_eq!(elites.len(), 1, "{theme:?}: no elite arrived");
            assert_eq!(elites[0].kind, heavy, "{theme:?}: the wrong elite body");
            let expected = heavy.base_hp() * w.tuning().horde_hp_mult_for(0, open) * 1.6;
            assert!(
                (elites[0].max_hp - expected).abs() < 1e-3,
                "{theme:?}: elite hp {} is not the 1.6x pool {expected}",
                elites[0].max_hp
            );
        }
    }

    /// The caps are the roster's: four shooters, eight heavies, and the big
    /// snake at two. A pulse that keeps rolling never exceeds them, and the
    /// capped share goes to fodder instead (no extra draw, no retry).
    #[test]
    fn the_pulse_respects_the_roster_caps() {
        use crate::game::roster;
        let mut w = World::new(Theme::Sewer, 71);
        let dt = 0.2;
        let mut elapsed = 0.0;
        let mut peak_snake = 0usize;
        let mut peak_shooter = 0usize;
        let mut peak_heavy = 0usize;
        for _ in 0..4200 {
            elapsed += dt;
            director_tick(&mut w, dt, elapsed);
            let snapshot = w.enemies();
            let count = |k: EnemyKind| snapshot.iter().filter(|e| e.kind == k).count();
            peak_snake = peak_snake.max(count(roster::kind(Theme::Sewer, roster::Slot::III)));
            peak_shooter = peak_shooter.max(count(roster::kind(Theme::Sewer, roster::Slot::R)));
            peak_heavy = peak_heavy.max(
                snapshot
                    .iter()
                    .filter(|e| e.kind == roster::kind(Theme::Sewer, roster::Slot::H) && !e.elite)
                    .count(),
            );
            // Nothing in this harness kills anything, so the crowd-pool
            // bodies (A, B, C) are swept each tick: otherwise the opening
            // fodder fills the live budget for good and the later slots never
            // get a roll. The capped slots' own bodies stay and pile up.
            let sweep: Vec<bevy_ecs::entity::Entity> = w
                .ecs
                .query::<(bevy_ecs::entity::Entity, &Enemy)>()
                .iter(&w.ecs)
                .filter(|(_, e)| {
                    matches!(
                        roster::floor(Theme::Sewer).slot_of(e.kind),
                        Some(roster::Slot::A | roster::Slot::B | roster::Slot::C)
                    )
                })
                .map(|(id, _)| id)
                .collect();
            for id in sweep {
                w.despawn_entity(id);
            }
        }
        assert!(
            peak_snake <= roster::kind_cap(EnemyKind::BigSnake),
            "the snake cap slipped: {peak_snake} snakes"
        );
        assert!(
            peak_shooter <= roster::Slot::R.cap(),
            "the shooter cap slipped: {peak_shooter} shooters"
        );
        assert!(
            peak_heavy <= roster::Slot::H.cap(),
            "the heavy cap slipped: {peak_heavy} heavies"
        );
        // The caps are reached, not just never breached: later in the run the
        // roster really does hold a full shooter line and a snake.
        assert!(peak_snake > 0, "no snake ever spawned on the Sewer");
    }

    // ---- Neon-merge hooks (`run_director`) -------------------------------

    /// Hook: `e.id = next_enemy_id` on every director spawn. Ids must be
    /// non-zero (0 means "anonymous enemy" to `DamageNumbers`), unique, and
    /// increase in spawn order, so per-enemy damage numbers aggregate instead
    /// of merging strangers.
    #[test]
    fn director_assigns_unique_nonzero_enemy_ids_in_spawn_order() {
        let mut w = World::new(Theme::Sewer, 47);
        let dt = 0.25;
        let mut elapsed = 0.0;
        for _ in 0..400 {
            elapsed += dt;
            director_tick(&mut w, dt, elapsed);
        }
        assert!(w.enemies().len() > 20, "the run should have spawned a horde");

        let ids: Vec<u32> = w.enemies().iter().map(|e| e.id).collect();
        assert!(ids.iter().all(|id| *id != 0), "an enemy got id 0");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "enemy ids are not unique");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "ids must increase in spawn order: {:?}",
            &ids[..ids.len().min(8)]
        );
    }

    /// Hook: `e.elite = true` on the director's periodic elite Brute only —
    /// ordinary late-game Brutes stay unflagged so the art/VFX can tell them
    /// apart.
    #[test]
    fn only_director_elites_are_flagged_elite() {
        let mut w = World::new(Theme::Sewer, 53);
        // A late pulse with the elite gate closed: nothing may be elite.
        w.director.spawn_timer = 0.0;
        w.director.elite_timer = director::ELITE_INTERVAL;
        director_tick(&mut w, 0.1, 200.0);
        assert!(!w.enemies().is_empty(), "the pulse should have spawned");
        assert!(
            w.enemies().iter().all(|e| !e.elite),
            "a non-elite spawn was flagged elite"
        );

        // Now the elite branch fires: exactly one flagged heavy.
        w.director.elite_timer = 0.0;
        director_tick(&mut w, 0.1, 200.1);
        let enemies = w.enemies();
        let elites: Vec<&Enemy> = enemies.iter().filter(|e| e.elite).collect();
        assert_eq!(elites.len(), 1, "exactly one elite per elite spawn");
        assert_eq!(
            elites[0].kind,
            crate::game::roster::kind(Theme::Sewer, crate::game::roster::Slot::H)
        );
        assert_ne!(elites[0].id, 0, "the elite still needs a runtime id");
    }

    // ---- Content update: spawn pacing on the big procedural maps ----------

    /// Wall clearance every ring point must keep (mirrors `SPAWN_WALL_CLEAR`).
    const RING_CLEAR: f32 = 4.0;

    /// The ring sits just beyond the **widest** view the zoom ladder can show
    /// (and never closer than 120 px), so a spawn is never on screen at any
    /// zoom — that is what makes "just outside the camera" work on any terminal
    /// size and at `2.0`, where the old 1.0-view ring was visible.
    #[test]
    fn the_ring_radius_follows_the_widest_view() {
        assert!((ring_radius(Vec2::splat(1.0)) - 120.0).abs() < 1e-4);
        let vp = Vec2::new(600.0, 400.0);
        let widest = super::widest_view(vp);
        assert_eq!(widest, Vec2::new(1200.0, 800.0));
        assert!((ring_radius(vp) - (widest.len() * 0.5 + 16.0)).abs() < 1e-4);
        // The 1.0-view ring is kept as the inward step of the search.
        assert!(super::one_x_ring_radius(vp) < ring_radius(vp));
        assert!((super::one_x_ring_radius(vp) - (vp.len() * 0.5 + 16.0)).abs() < 1e-4);
    }

    /// The simulation's "out of sight" is the widest rung of the *render-only*
    /// zoom ladder, and the ladder itself is the render module's. This pins the
    /// plain-number mirror (`MAX_ZOOM_OUT`) to `render::zoom::MAX`, and proves
    /// that every point on the ring is outside that rectangle on every bearing.
    #[test]
    fn the_spawn_view_is_the_widest_rung_of_the_zoom_ladder() {
        assert_eq!(super::MAX_ZOOM_OUT, crate::render::zoom::MAX);
        for vp in [
            Vec2::new(120.0, 74.0),
            Vec2::new(160.0, 80.0),
            Vec2::new(200.0, 120.0),
            Vec2::new(60.0, 40.0),
        ] {
            let r = ring_radius(vp);
            let widest = super::widest_view(vp);
            let rat = Vec2::new(500.0, 500.0);
            for k in 0..360 {
                let p = rat + Vec2::from_angle(k as f32 * std::f32::consts::TAU / 360.0) * r;
                assert!(
                    super::off_screen(p, rat, vp),
                    "a ring point {p:?} is inside the widest view {widest:?}"
                );
                // …and it is beyond the unzoomed view too, so the inward ring
                // is still an in-view-free fallback.
                assert!(!super::in_sim_view(p, rat, vp));
            }
        }
    }

    /// A moving player biases the ring to the direction he is heading: the
    /// horde arrives from ahead and the sides instead of from behind his back.
    #[test]
    fn the_ring_prefers_the_direction_the_player_is_heading() {
        let mut w = World::new(Theme::Lab, 31);
        open_level(&mut w.level);
        w.player.vel = Vec2::new(1.0, 0.0);
        let r = ring_radius(w.viewport);
        let mut rng = StdRng::seed_from_u64(31);
        for _ in 0..200 {
            let p = spawn_point(&mut rng, &w.level, &w.player, w.viewport);
            let d = p - w.player.pos;
            assert!(d.x > r * 0.8, "spawn {d:?} is not ahead of the rat");
            assert!((d.len() - r).abs() < 1e-3, "spawn {d:?} left the ring");
        }
    }

    /// A handful of standpoints per map: the start, plus four random open cells
    /// (so rooms, corridors and open halls all exercise the ring).
    fn standpoints(w: &World, seed: u64) -> Vec<Vec2> {
        let mut rng = StdRng::seed_from_u64(seed ^ 0x5eed_cafe);
        let mut spots = vec![w.player.pos];
        for _ in 0..4 {
            for _ in 0..128 {
                let cx = rng.random_range(4..MAP_COLS - 4);
                let cy = rng.random_range(4..MAP_ROWS - 4);
                let p = TileMap::tile_rect(cx, cy).center();
                if w.level.tiles.circle_clear(p, 10.0, SizeClass::Large) {
                    spots.push(p);
                    break;
                }
            }
        }
        spots
    }

    /// **Property test over seeds**: on every generated map, from every
    /// standpoint, the spawn ring lands on in-bounds, wall-clear floor at least
    /// the minimum distance from the rat. The legacy ring only checked the wall
    /// list and could clamp into a wall cluster on a 10× map; this is the
    /// invariant that a spawn is never *in* a wall.
    #[test]
    fn the_spawn_ring_never_lands_in_a_wall_on_any_generated_map() {
        let mut maps = 0usize;
        for theme in Theme::all() {
            for seed in [1u64, 2, 3, 7, 11, 29, 101, 4096] {
                let mut w = World::new(theme, seed);
                let start = w.player.pos;
                for spot in standpoints(&w, seed) {
                    w.player.pos = spot;
                    for _ in 0..25 {
                        let p = spawn_point(&mut w.rng, &w.level, &w.player, w.viewport);
                        assert!(
                            p.x >= 7.5
                                && p.y >= 7.5
                                && p.x <= w.level.arena.x - 7.5
                                && p.y <= w.level.arena.y - 7.5,
                            "{theme:?} seed {seed}: spawn {p:?} left the arena margin"
                        );
                        assert!(
                            w.level.tiles.circle_clear(p, RING_CLEAR, SizeClass::Large),
                            "{theme:?} seed {seed}: spawn {p:?} overlaps a wall"
                        );
                        if spot == start {
                            // The ladder prefers the widest-view ring and steps
                            // inward to the 1.0-view ring before the room and
                            // nearest-cell escapes, so the minimum distance
                            // holds unless the rat is boxed into a pocket the
                            // escapes have to reach through
                            // (`SpawnFallbacks` counts which rung fired).
                            assert!(
                                p.dist(spot) >= super::NEAR_SPAWN_DIST - 1e-3,
                                "{theme:?} seed {seed}: spawn {p:?} on top of the player"
                            );
                        }
                    }
                }
                maps += 1;
            }
        }
        assert_eq!(
            maps,
            Theme::all().len() * 8,
            "the sweep must cover every theme"
        );
    }

    /// The director's ring additionally proves the point is on the horde's
    /// connected map: no enemy is ever spawned into a sealed pocket it cannot
    /// leave (the content plan's "a cell in a sealed pocket is not a spawn, it
    /// is a lost enemy").
    #[test]
    fn horde_spawns_are_always_reachable_from_the_start() {
        for theme in Theme::all() {
            for seed in [1u64, 5, 13, 77] {
                let mut w = World::new(theme, seed);
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                assert!(
                    scratch.is_reachable(&w.level, w.player.pos),
                    "{theme:?} seed {seed}: the start room is not on the horde's map"
                );
                for spot in standpoints(&w, seed) {
                    w.player.pos = spot;
                    for _ in 0..25 {
                        let p = spawn_point_reachable(
                            &mut w.rng,
                            &w.level,
                            &w.player,
                            w.viewport,
                            &mut scratch,
                        )
                        .expect("a generated map must offer a spawn point");
                        assert!(
                            scratch.is_reachable(&w.level, p),
                            "{theme:?} seed {seed}: spawn {p:?} is unreachable for the horde"
                        );
                        assert!(
                            w.level.tiles.circle_clear(p, RING_CLEAR, SizeClass::Large),
                            "{theme:?} seed {seed}: reachable spawn {p:?} overlaps a wall"
                        );
                        assert!(
                            !super::in_maze(&w.level, p),
                            "{theme:?} seed {seed}: spawn {p:?} is inside a maze"
                        );
                    }
                }
            }
        }
    }

    /// Room spawns (used by room waves, elites and the boss phase) land inside
    /// the room, on wall-clear floor, and in a room the horde can enter.
    #[test]
    fn room_spawns_stay_inside_a_reachable_room() {
        let mut w = World::new(Theme::Kitchen, 3);
        let mut scratch = SpawnScratch::default();
        scratch.sync(&w.level);
        let mut sampled = 0usize;
        for (i, room) in w.level.rooms.iter().enumerate() {
            let Some(p) = spawn_point_in_room(&mut w.rng, &w.level, room, SizeClass::Large) else {
                continue;
            };
            sampled += 1;
            assert!(
                room.rect.contains_world(p),
                "room spawn {p:?} is outside {:?}",
                room.rect
            );
            assert!(w.level.tiles.circle_clear(p, RING_CLEAR, SizeClass::Large));
            if matches!(room.kind, RoomKind::Hall | RoomKind::Room | RoomKind::Boss) {
                assert!(
                    scratch.room_reachable(i) && scratch.is_reachable(&w.level, p),
                    "a {:?} room is not on the horde's map",
                    room.kind
                );
            }
        }
        assert!(sampled > 0, "no room on a generated map accepted a spawn");
    }

    /// Room waves: a pulse can stage its first enemy inside a room the player
    /// is not in, so the horde arrives through a doorway instead of all of it
    /// materialising on the ring. Deterministic per rng, always inside a
    /// staging room, always reachable for the horde, and — design change 6 —
    /// never further than [`super::ROOM_WAVE_RANGE`] away or more than
    /// [`super::APPROACH_CELLS`] path cells from the rat.
    #[test]
    fn room_waves_stage_enemies_outside_the_players_room() {
        let mut hits = 0;
        for theme in Theme::all() {
            for seed in [3u64, 17, 1234] {
                let mut w = World::new(theme, seed);
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                // Try every chamber centre: with the wave range cut to 320 px
                // only a standpoint with a neighbour room inside it qualifies,
                // and which rooms those are depends on the map.
                let mut standpoints: Vec<Vec2> =
                    w.level.rooms.iter().map(|r| r.center()).collect();
                standpoints.insert(0, w.player.pos);
                for spot in standpoints {
                    w.player.pos = spot;
                    let mut a = StdRng::seed_from_u64(seed);
                    let mut b = StdRng::seed_from_u64(seed);
                    for _ in 0..8 {
                        let pa = super::room_wave_point(&mut a, &w.level, &w.player, &mut scratch);
                        let pb = super::room_wave_point(&mut b, &w.level, &w.player, &mut scratch);
                        assert_eq!(
                            pa, pb,
                            "{theme:?} seed {seed}: room wave is not deterministic"
                        );
                        let Some(p) = pa else { continue };
                        hits += 1;
                        assert!(
                            scratch.is_reachable(&w.level, p),
                            "{theme:?} seed {seed}: wave spawn {p:?} is unreachable"
                        );
                        assert!(w.level.tiles.circle_clear(p, RING_CLEAR, SizeClass::Large));
                        assert!(
                            !super::in_maze(&w.level, p),
                            "{theme:?} seed {seed}: wave spawn {p:?} is in a maze"
                        );
                        let room = w
                            .level
                            .room_at(p)
                            .expect("a room wave must land inside a room");
                        assert!(
                            !room.rect.contains_world(w.player.pos),
                            "{theme:?} seed {seed}: wave spawned in the player's own room"
                        );
                        assert!(matches!(
                            room.kind,
                            RoomKind::Start | RoomKind::Boss | RoomKind::Hall | RoomKind::Room
                        ));
                        assert!(
                            room.center().dist(w.player.pos) <= super::ROOM_WAVE_RANGE + 1e-3,
                            "{theme:?} seed {seed}: wave room is out of range"
                        );
                        assert!(
                            scratch.approach_cells(&w.level, p).is_some(),
                            "{theme:?} seed {seed}: wave approach is longer than {} cells",
                            super::APPROACH_CELLS
                        );
                        assert!(
                            p.dist(w.player.pos) >= super::SPAWN_MIN_DIST - 1e-3,
                            "{theme:?} seed {seed}: wave spawned on top of the rat"
                        );
                    }
                }
            }
        }
        assert!(hits > 0, "no room wave ever found a staging room");
    }

    /// The pacing curve follows §1's live budget: the first minute stays under
    /// its 18-body opening budget, the horde grows to the late 165-body budget
    /// by the boss time, and the 600-body cap is never touched. Driven through
    /// the real director only (nobody moves or dies), so it is exact and cheap.
    #[test]
    fn the_pacing_curve_stays_on_the_live_budget() {
        let mut w = World::new(Theme::Lab, 9);
        let dt = 0.1;
        let mut elapsed = 0.0;
        let mut early_peak = 0usize;
        let mut peak = 0usize;
        while elapsed < director::RUN_SECONDS {
            elapsed += dt;
            director_tick(&mut w, dt, elapsed);
            let live = w.enemy_count();
            if elapsed <= 60.0 {
                early_peak = early_peak.max(live);
            }
            peak = peak.max(live);
        }
        assert!(
            early_peak <= director::live_budget_at(59.9),
            "the opening exceeded its live budget: {early_peak}"
        );
        assert!(
            early_peak >= 12,
            "only {early_peak} enemies before 60 s: the horde never builds"
        );
        let top = director::live_budget_at(director::RUN_SECONDS - 1.0);
        assert!(
            (top * 3 / 4..=top).contains(&peak),
            "the late horde did not reach its budget: peak {peak} of {top}"
        );
        assert!(peak < ENEMY_CAP, "the 600-body cap was touched: {peak}");
        assert!(w.director.boss_spawned, "the boss slot never filled");
        assert!(w.director.pulses > 100, "too few pulses: {}", w.director.pulses);
    }

    /// A one-way gate is a **wall for the horde**: its leaf is a permanent
    /// collider for anything that is not the player, so a cell whose only exit
    /// is a gate must never be a spawn — an enemy placed there is lost forever.
    /// The same room entered through a normal doorway *is* a legal spawn.
    #[test]
    fn a_one_way_gate_is_a_wall_for_the_horde_but_a_doorway_is_not() {
        use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
        use crate::game::collision::TileKind;

        /// A 5 x 5 cell room at 20..25 with a one-cell shell and a single
        /// opening of the given kind at (22, 19).
        fn sealed_room(theme: Theme, exit: TileKind, seed: u64) -> Level {
            let mut level = Level::empty(theme);
            level.seed = seed;
            level
                .tiles
                .fill_cells(19, 19, 7, 7, TileKind::Wall(WallKind::Solid));
            level.tiles.fill_cells(20, 20, 5, 5, TileKind::Open);
            level.tiles.set(22, 19, exit);
            level.tiles.rebuild_masks();
            level
        }

        let inside = Vec2::new(22.5 * 16.0, 22.5 * 16.0);

        let gated = sealed_room(
            Theme::Sewer,
            TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::North,
            }),
            1,
        );
        let mut scratch = SpawnScratch::default();
        scratch.sync(&gated);
        assert!(
            !scratch.is_reachable(&gated, inside),
            "a room whose only exit is a one-way gate counts as a spawn region"
        );

        let doored = sealed_room(
            Theme::Sewer,
            TileKind::Wall(WallKind::Doorway {
                axis: WallAxis::Horizontal,
            }),
            2,
        );
        let mut scratch = SpawnScratch::default();
        scratch.sync(&doored);
        assert!(
            scratch.is_reachable(&doored, inside),
            "a room with a normal doorway is not a legal spawn region"
        );
    }

    /// Regression guard for "do not re-run a full BFS per spawn": the ring is a
    /// handful of tile queries, and the one flood fill behind it is paid once
    /// per level. Prints a table under `--nocapture`.
    #[test]
    fn spawn_placement_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let mut w = World::new(Theme::Sewer, 7);
        let mut scratch = SpawnScratch::default();
        let t = Instant::now();
        scratch.sync(&w.level);
        let sync_ms = t.elapsed().as_secs_f64() * 1e3;

        const N: usize = 20_000;
        let t = Instant::now();
        for _ in 0..N {
            black_box(spawn_point_reachable(
                &mut w.rng,
                &w.level,
                &w.player,
                w.viewport,
                &mut scratch,
            ));
        }
        let per_reachable_us = t.elapsed().as_secs_f64() * 1e6 / N as f64;

        let t = Instant::now();
        for _ in 0..N {
            black_box(spawn_point(&mut w.rng, &w.level, &w.player, w.viewport));
        }
        let per_ring_us = t.elapsed().as_secs_f64() * 1e6 / N as f64;

        println!("spawn placement benchmark (200 x 160-cell map, dev profile)");
        println!("  SpawnScratch::sync (one flood fill per level)  {sync_ms:>8.3} ms");
        println!("  spawn_point_reachable (cached reach)         {per_reachable_us:>8.3} us/spawn");
        println!("  spawn_point (ring only, no cache)            {per_ring_us:>8.3} us/spawn");
        // A pulse spawns at most ~25 enemies; at 20 us each that is 0.5 ms of a
        // 16 ms frame, and anything near a per-spawn flood fill (hundreds of
        // microseconds at best) fails loudly here.
        assert!(
            per_reachable_us < 200.0,
            "spawn placement costs {per_reachable_us:.1} us: is the flood fill running per spawn?"
        );
    }

    /// The horde still *arrives*: with the ring and the room waves both in
    /// play, the crowd comes to the rat instead of piling up behind a wall.
    ///
    /// Two things are asserted strictly: **every** enemy that survived a 20 s
    /// window is on the horde's own reachable map (so the spawn ring never
    /// places a body in a pocket — the one thing this file is responsible for)
    /// and none is left inside a wall. The pile-up check is a fraction (≤ 5 %
    /// stalled) rather than "every single body moved", because a handful of
    /// bodies can end up pinned flush against a wall face when the *steering*
    /// points into it — that is the nav/collision pair's business (and what
    /// `nav_benchmark` measures), not the spawn ring's, and it is reported
    /// rather than hidden here.
    #[test]
    fn the_horde_converges_on_a_stationary_player() {
        let mut w = World::new(Theme::Sewer, 23);
        let mut scratch = SpawnScratch::default();
        scratch.sync(&w.level);
        let dt = 1.0 / 30.0;
    let run = |w: &mut World, secs: f32| {
            for _ in 0..(secs / dt) as usize {
                w.update(dt, Vec2::ZERO, false);
                // Keep him alive so the run measures the horde, not the rat.
                w.player.hp = w.player.max_hp();
                w.pending_levelups = 0;
            }
        };

        run(&mut w, 70.0);
        let kills_before = w.kills;
        let before: Vec<(u32, Vec2)> = w.enemies().iter().map(|e| (e.id, e.pos)).collect();
        run(&mut w, 20.0);
        let after = w.enemies();
        assert!(
            !before.is_empty() || !after.is_empty(),
            "no enemy ever existed: the test would be vacuous"
        );

        let mut stalled = Vec::new();
        let mut survivors = 0usize;
        for e in &after {
            let Some((_, p0)) = before.iter().find(|(id, _)| *id == e.id) else {
                continue;
            };
            survivors += 1;
            // Soundness of the spawn ring: nothing that ever spawned through it
            // can be off the horde's map or inside a wall.
            assert!(
                scratch.is_reachable(&w.level, e.pos),
                "enemy {} is off the horde's reachable map at {:?}",
                e.id,
                e.pos
            );
            assert!(
                w.level
                    .tiles
                    .circle_clear(e.pos, e.radius, SizeClass::of_enemy(e.kind)),
                "enemy {} is inside a wall at {:?}",
                e.id,
                e.pos
            );
            // Still far away after 20 s *and* essentially in the same spot.
            if e.pos.dist(*p0) < 60.0 && e.pos.dist(w.player.pos) > 400.0 {
                stalled.push((e.id, e.pos, e.pos.dist(w.player.pos)));
            }
        }
        assert!(
            stalled.len() * 20 <= survivors.max(1),
            "{} of {survivors} enemies stalled for 20 s away from the rat: {stalled:?}",
            stalled.len()
        );
        assert!(
            w.kills > kills_before,
            "nothing reached the rat in 20 s (kills {} -> {})",
            kills_before,
            w.kills
        );
    }

    // ---- Content wave: where and how the horde arrives --------------------

    /// **Design change 1 — the chase begins at the mouth.**
    ///
    /// No spawn path may place a body inside a maze's ring or interior: not the
    /// ring, not a room wave, not the fallback, not an elite (all of which share
    /// the director's sampler). Sampled from the two worst vantage points: the
    /// maze's mouth (where the spawn ring falls inside the warren) and its cache
    /// (where the entire ring is inside it).
    ///
    /// The designer measured 2 609 / 5 376 inside on the baseline; the
    /// acceptance number is zero. Mazes are rare by owner decision (about one
    /// map in eight), so the sweep walks seeds until it has enough maps.
    #[test]
    fn zero_spawns_land_inside_a_maze() {
        let mut maps = 0usize;
        let mut sampled = 0usize;
        let mut mazes = 0usize;
        for theme in Theme::all() {
            for seed in 0..64u64 {
                if maps >= 6 {
                    break;
                }
                let mut w = World::new(theme, seed);
                if w.level.mazes.is_empty() {
                    continue;
                }
                maps += 1;
                let sections = w.level.mazes.clone();
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                for maze in &sections {
                    mazes += 1;
                    let entry = TileMap::tile_rect(maze.entry.0, maze.entry.1).center();
                    for spot in [entry, maze.reward] {
                        w.player.pos = spot;
                        for _ in 0..500 {
                            let p = spawn_point_reachable(
                                &mut w.rng,
                                &w.level,
                                &w.player,
                                w.viewport,
                                &mut scratch,
                            )
                            .expect("a maze side still has rooms to spawn in");
                            assert!(
                                !in_maze(&w.level, p),
                                "{theme:?} seed {seed}: ring/fallback spawn {p:?} is inside a maze"
                            );
                            sampled += 1;
                        }
                        for k in 0..200 {
                            let p = super::spawn_pulse_point(
                                &mut w.rng,
                                &w.level,
                                &w.player,
                                w.viewport,
                                &mut scratch,
                                true,
                                None,
                                k,
                            )
                            .expect("a room wave or the ring must find a point");
                            assert!(
                                !in_maze(&w.level, p),
                                "{theme:?} seed {seed}: pulse spawn {p:?} is inside a maze"
                            );
                            sampled += 1;
                        }
                    }
                }
            }
        }
        println!("zero-spawn-in-maze: {sampled} samples over {maps} maps / {mazes} mazes");
        assert!(maps >= 3, "only {maps} maze-bearing maps: the test is thin");
        assert!(sampled >= 4_000, "only {sampled} samples: not a property test");
    }

    /// **Design change 6, first half — two off-screen flanking groups.**
    ///
    /// A chamber only earns a flank when it has two *real* approaches: openings
    /// that are reachable, off screen from the rat, at least `SPAWN_MIN_DIST`
    /// away, inside the 20-cell approach cap and at least 60° apart in bearing.
    /// Every planned anchor must satisfy all of that, and the two must be on
    /// different sides.
    #[test]
    fn a_flank_needs_two_real_off_screen_approaches() {
        let mut planned = 0usize;
        let mut refused = 0usize;
        for theme in Theme::all() {
            for seed in [2u64, 9, 21, 44] {
                let mut w = World::new(theme, seed);
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                for (i, room) in w.level.rooms.iter().enumerate() {
                    if !matches!(
                        room.kind,
                        RoomKind::Start | RoomKind::Hall | RoomKind::Room | RoomKind::Boss
                    ) || !scratch.room_reachable(i)
                    {
                        continue;
                    }
                    w.player.pos = room.center();
                    scratch.build_approach(&w.level, w.player.pos);
                    let Some(plan) = plan_flank(&w.level, &w.player, w.viewport, i, &mut scratch)
                    else {
                        refused += 1;
                        continue;
                    };
                    planned += 1;
                    for anchor in plan.anchors {
                        assert!(
                            !in_maze(&w.level, anchor),
                            "{theme:?} seed {seed}: flank anchor {anchor:?} is in a maze"
                        );
                        // The doorway is a route marker: it may be on screen,
                        // because the bodies are placed out of sight (below).
                        assert!(
                            anchor.dist(w.player.pos) >= super::SPAWN_MIN_DIST - 1e-3,
                            "{theme:?} seed {seed}: flank anchor {anchor:?} is on the rat"
                        );
                        assert!(
                            scratch.is_reachable(&w.level, anchor),
                            "{theme:?} seed {seed}: flank anchor {anchor:?} is unreachable"
                        );
                        assert!(
                            scratch.approach_cells(&w.level, anchor).is_some(),
                            "{theme:?} seed {seed}: flank approach is longer than {APPROACH_CELLS} cells"
                        );
                    }
                    let sep = super::angle_between(
                        plan.anchors[0] - w.player.pos,
                        plan.anchors[1] - w.player.pos,
                    );
                    assert!(
                        sep >= super::FLANK_MIN_SEPARATION - 1e-4,
                        "{theme:?} seed {seed}: the two flank groups are {sep:.2} rad apart"
                    );
                    // Each group's bodies stay off screen and in the chamber's
                    // neighbourhood (the offsets are a fixed cell spiral).
                    for member in 0..8 {
                        if let Some(p) = plan.point(
                            &w.level,
                            &w.player,
                            w.viewport,
                            member,
                            Some(&scratch.reach),
                        ) {
                            assert!(!in_maze(&w.level, p));
                            assert!(
                                scratch.is_reachable(&w.level, p),
                                "{theme:?} seed {seed}: flank member {member} at {p:?} is off the horde's map"
                            );
                            assert!(w
                                .level
                                .tiles
                                .circle_clear(p, super::SPAWN_WALL_CLEAR, SizeClass::Large));
                            assert!(p.dist(w.player.pos) >= super::SPAWN_MIN_DIST - 1e-3);
                            assert!(super::off_screen(p, w.player.pos, w.viewport));
                        }
                    }
                }
            }
        }
        assert!(planned > 0, "no chamber ever offered two approaches");
        println!("flank planner: {planned} chambers flankable, {refused} refused");
    }

    /// The flank actually fires through the director: a rat that holds one
    /// flankable chamber for more than `FLANK_DWELL_SECS` spends the dwell (the
    /// director's own `take_flank` resets it), while a chamber with no second
    /// approach keeps the dwell climbing.
    #[test]
    fn a_camped_chamber_is_flanked_through_the_director() {
        use crate::game::director::FLANK_DWELL_SECS;
        let mut flanked = 0;
        let mut refused = 0;
        'maps: for theme in Theme::all() {
            for seed in [2u64, 9, 21, 44, 61] {
                let mut probe_scratch = SpawnScratch::default();
                let probe = World::new(theme, seed);
                probe_scratch.sync(&probe.level);
                for (i, room) in probe.level.rooms.iter().enumerate() {
                    if !matches!(
                        room.kind,
                        RoomKind::Start | RoomKind::Hall | RoomKind::Room | RoomKind::Boss
                    ) {
                        continue;
                    }
                    let centre = room.center();
                    probe_scratch.build_approach(&probe.level, centre);
                    let flankable = plan_flank(
                        &probe.level,
                        &probe.player,
                        probe.viewport,
                        i,
                        &mut probe_scratch,
                    )
                    .is_some();
                    let mut w = World::new(theme, seed);
                    w.player.pos = centre;
                    // 12 s in 0.25 s ticks: the dwell crosses 8 s and a pulse
                    // follows within one interval.
                    let mut elapsed = 0.0;
                    while elapsed < 12.0 {
                        elapsed += 0.25;
                        director_tick(&mut w, 0.25, elapsed);
                    }
                    if flankable {
                        assert!(
                            w.director.chamber_dwell < FLANK_DWELL_SECS,
                            "{theme:?} seed {seed}: a camped, flankable chamber never spent its dwell"
                        );
                        assert_eq!(w.director.chamber_room, i as i32);
                        flanked += 1;
                    } else {
                        refused += 1;
                    }
                }
                if flanked >= 4 {
                    break 'maps;
                }
            }
        }
        assert!(flanked >= 4, "only {flanked} flankable chambers ({refused} refused)");
    }

    /// **Design change 6, measurement.** Bodies arriving within 60 px of a
    /// stationary rat, per 4 s window, and from how many distinct sides (8
    /// compass sectors), standing in a hall, in a corridor and in a doorway.
    ///
    /// Run with `cargo test arrivals_within_sixty_pixels -- --nocapture` to read
    /// the table.
    #[test]
    fn arrivals_within_sixty_pixels_by_sides() {
        const WINDOW: f32 = 4.0;
        const NEAR: f32 = 60.0;
        const RUN: f32 = 24.0;
        const SIDES: usize = 8;
        let dt = 1.0 / 30.0;

        // One standpoint per shape, taken from the first maps that have one of
        // each (the shapes are on different maps, which is fine: the point is
        // the arrival pattern per shape, not one map).
        let mut spots: Vec<(&str, Theme, u64, Vec2)> = Vec::new();
        'outer: for theme in Theme::all() {
            for seed in 0..8u64 {
                let w = World::new(theme, seed);
                for (label, kind) in [("hall", RoomKind::Hall), ("corridor", RoomKind::Corridor)] {
                    if spots.iter().any(|(l, _, _, _)| *l == label) {
                        continue;
                    }
                    if let Some(room) = w.level.rooms.iter().find(|r| r.kind == kind) {
                        let p = room.center();
                        // The rat is `Small` (radius 3): a one-cell corridor
                        // and a doorway both clear 4 px but no more.
                        if w.level.tiles.circle_clear(p, 4.0, SizeClass::Small) {
                            spots.push((label, theme, seed, p));
                        }
                    }
                }
                if !spots.iter().any(|(l, _, _, _)| *l == "doorway") {
                    if let Some((cx, cy, _)) = w
                        .level
                        .tiles
                        .iter_walls()
                        .find(|(_, _, k)| matches!(k, WallKind::Doorway { .. }))
                    {
                        let p = TileMap::tile_rect(cx, cy).center();
                        if w.level.tiles.circle_clear(p, 4.0, SizeClass::Small) {
                            spots.push(("doorway", theme, seed, p));
                        }
                    }
                }
                if spots.len() >= 3 {
                    break 'outer;
                }
            }
        }
        assert_eq!(spots.len(), 3, "could not find a hall + corridor + doorway");

        println!("\narrivals within {NEAR} px, per {WINDOW} s window (8 sides):");
        let mut totals = Vec::new();
        for (label, theme, seed, spot) in spots {
            let mut w = World::new(theme, seed);
            w.player.pos = spot;
            // Per-enemy previous distance, keyed by the runtime enemy id.
            let mut was: Vec<(u32, f32)> = Vec::new();
            let mut windows: Vec<u32> = vec![0; (RUN / WINDOW) as usize];
            let mut sides: Vec<[bool; SIDES]> = vec![[false; SIDES]; windows.len()];
            let mut steps = 0u32;
            let mut t = 0.0f32;
            while t < RUN {
                w.update(dt, Vec2::ZERO, false);
                w.player.hp = w.player.max_hp();
                w.pending_levelups = 0;
                t += dt;
                steps += 1;
                let index = ((t - 1e-4) / WINDOW) as usize;
                if index >= windows.len() {
                    break;
                }
                let mut now: Vec<(u32, f32)> = Vec::with_capacity(w.enemies().len());
                for e in w.enemies() {
                    let d = e.pos.dist(w.player.pos);
                    let before = was
                        .iter()
                        .find(|(id, _)| *id == e.id)
                        .map(|(_, d)| *d)
                        .unwrap_or(f32::INFINITY);
                    if before > NEAR && d <= NEAR {
                        windows[index] += 1;
                        let a = (e.pos - w.player.pos).angle().rem_euclid(std::f32::consts::TAU);
                        let side = (a / std::f32::consts::TAU * SIDES as f32) as usize % SIDES;
                        sides[index][side] = true;
                    }
                    now.push((e.id, d));
                }
                was = now;
            }
            let mut total = 0u32;
            let mut all_sides = [false; SIDES];
            print!("  {label:<9}");
            for (i, arrivals) in windows.iter().enumerate() {
                let distinct = sides[i].iter().filter(|s| **s).count();
                for (k, s) in sides[i].iter().enumerate() {
                    all_sides[k] |= *s;
                }
                total += *arrivals;
                print!(
                    "  {}–{}s: {arrivals:>3} from {distinct} sides",
                    i as f32 * WINDOW,
                    (i + 1) as f32 * WINDOW
                );
            }
            println!(
                "  | total {total}, {} sides over {:.0} s ({} ticks)",
                all_sides.iter().filter(|s| **s).count(),
                RUN,
                steps
            );
            assert!(
                total > 0,
                "{label}: the horde never arrived within {NEAR} px in {RUN} s"
            );
            totals.push((label, total, all_sides.iter().filter(|s| **s).count()));
        }
        let hall_sides = totals
            .iter()
            .find(|(label, _, _)| *label == "hall")
            .map(|(_, _, s)| *s)
            .unwrap_or(0);
        assert!(
            hall_sides >= 2,
            "a stationary rat in a hall was only ever approached from {hall_sides} side(s)"
        );
    }

    /// The campaign level scales the horde: level 2 and level 3 open with the
    /// hp of a mid-run level 1, because the director reads `LevelIndex` and
    /// runs the whole curve through `director::curve_clock` — and their horde
    /// carries the level's `LEVEL_HP_EXTRA` on top (`horde_hp_mult_for`).
    #[test]
    fn the_campaign_level_scales_the_horde() {
        let mut multipliers = Vec::new();
        for level in 0..crate::game::campaign::LEVEL_COUNT {
            let mut w = World::new_level(level, 7, Loadout::starting());
            w.director.spawn_timer = 0.0;
            director_tick(&mut w, 0.1, 300.0);
            let enemies = w.enemies();
            assert!(!enemies.is_empty(), "level {level} spawned nothing");
            let expected = w.tuning().horde_hp_mult_for(level, 300.0);
            for e in enemies.iter().filter(|e| !e.elite) {
                // (The elite branch stacks its own 1.6x on top of the curve.)
                let base = e.max_hp / expected;
                assert!(
                    (base - e.kind.base_hp()).abs() < 1e-2,
                    "level {level}: {:?} hp {} is not its base × {expected}",
                    e.kind,
                    e.max_hp
                );
            }
            let plain = enemies.iter().find(|e| !e.elite).expect("a plain spawn");
            multipliers.push(plain.max_hp / plain.kind.base_hp());
        }
        assert!(
            multipliers[1] > multipliers[0] && multipliers[2] > multipliers[1],
            "hp did not scale with the campaign level: {multipliers:?}"
        );
    }

    /// The maze exclusion is a *spawn* rule only: a maze stays on the horde's
    /// connected map, so enemies that follow the rat in keep chasing it (the
    /// one-way door is the exception — it is a wall to them).
    #[test]
    fn a_maze_is_still_walkable_for_the_horde() {
        let mut checked = 0;
        for theme in Theme::all() {
            for seed in 0..64u64 {
                if checked >= 4 {
                    break;
                }
                let w = World::new(theme, seed);
                let Some(maze) = w.level.mazes.first().copied() else {
                    continue;
                };
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                let entry = TileMap::tile_rect(maze.entry.0, maze.entry.1).center();
                let reward = maze.reward;
                assert!(
                    scratch.is_reachable(&w.level, entry),
                    "{theme:?} seed {seed}: the maze mouth is off the horde's map"
                );
                assert!(
                    scratch.is_reachable(&w.level, reward),
                    "{theme:?} seed {seed}: the horde cannot follow the rat to the cache"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no maze-bearing maps in the sweep");
    }

    // ---- Batch ten: the spawn ring is the widest view ---------------------

    /// Standpoints that stress the ring: each map's start, four random open
    /// cells, cells near the level edge (where a doubled ring clamps) and the
    /// corridor centres (one cell wide, so the ring is inside a wall on every
    /// bearing and the escape ladder has to find floor).
    fn hard_standpoints(w: &World, seed: u64) -> Vec<Vec2> {
        let mut spots = standpoints(w, seed);
        let (cols, rows) = (w.level.tiles.cols(), w.level.tiles.rows());
        for (cx, cy) in [(3, 3), (cols - 4, rows - 4), (cols - 4, 3)] {
            if w.level.tiles.passable(cx, cy, SizeClass::Small) {
                spots.push(TileMap::tile_rect(cx, cy).center());
            }
        }
        for room in w
            .level
            .rooms
            .iter()
            .filter(|r| r.kind == RoomKind::Corridor)
            .take(2)
        {
            spots.push(room.center());
        }
        spots
    }

    /// **Batch ten, the promise.** No ordinary spawn lands inside the widest
    /// view rectangle when the widest-view ring could find a reachable cell
    /// outside it.
    ///
    /// Every path a pulse body (or an elite, or a miniboss, which share
    /// [`spawn_point_reachable`]) can take is sampled on all six floors, from
    /// standpoints that include the level edge and one-cell corridors. The
    /// claim is exact and checkable: a point inside the widest view is only
    /// legal when the *primary* ring pass (`SpawnFallbacks::ring`) found
    /// nothing on this sample.
    #[test]
    fn no_ordinary_spawn_lands_inside_the_widest_view() {
        let mut outside = 0usize;
        let mut inside = 0usize;
        let mut skipped = 0usize;
        for theme in Theme::playable() {
            for seed in [1u64, 44] {
                let mut w = World::new(theme, seed);
                let vp = w.viewport;
                let mut scratch = SpawnScratch::default();
                for spot in hard_standpoints(&w, seed) {
                    w.player.pos = spot;
                    let mut rng = StdRng::seed_from_u64(seed ^ 0x5eed);
                    for k in 0..24 {
                        let before = scratch.fallbacks;
                        let p = super::spawn_pulse_point(
                            &mut rng,
                            &w.level,
                            &w.player,
                            vp,
                            &mut scratch,
                            k % 5 == 0,
                            None,
                            k,
                        );
                        let after = scratch.fallbacks;
                        let Some(p) = p else {
                            skipped += 1;
                            assert!(
                                after.skipped > before.skipped,
                                "{theme:?} seed {seed}: a skipped spawn was not counted"
                            );
                            continue;
                        };
                        let ring_won = after.ring > before.ring;
                        if ring_won {
                            assert!(
                                super::off_screen(p, w.player.pos, vp),
                                "{theme:?} seed {seed}: the widest-view ring put {p:?} on screen"
                            );
                        }
                        if super::off_screen(p, w.player.pos, vp) {
                            outside += 1;
                        } else {
                            inside += 1;
                            assert!(
                                !ring_won,
                                "{theme:?} seed {seed}: {p:?} is on screen although the \
                                 widest-view ring had a reachable point"
                            );
                        }
                        // Whatever the rung, the point is legal ground for the
                        // horde (no wall, no maze, not on top of the rat).
                        assert!(
                            !in_maze(&w.level, p),
                            "{theme:?} seed {seed}: spawn {p:?} is inside a maze"
                        );
                        assert!(p.dist(w.player.pos) >= super::NEAR_SPAWN_DIST - 1e-3);
                    }
                }
            }
        }
        println!(
            "widest-view spawns: {outside} off screen, {inside} forced on screen, {skipped} skipped"
        );
        assert!(outside > 0, "the sweep never spawned at all");
        assert!(
            outside > inside * 20,
            "most spawns ({inside} of {}) had to fall back on screen: {outside} were \
             off screen",
            outside + inside
        );
    }

    /// **[`SpawnFallbacks`] measurement.** How often each rung of the search
    /// fires per floor: the coordinator reads the doubled ring's cost here
    /// (`cargo test spawn_fallbacks -- --nocapture`).
    #[test]
    fn spawn_fallbacks() {
        println!("\nspawn search rungs, 24 pulses per standpoint (viewport 160 x 80):");
        println!(
            "  {:<11} {:>6} {:>7} {:>6} {:>7} {:>6} {:>6} {:>6}",
            "floor", "ring", "outward", "inward", "any", "room", "near", "skipped"
        );
        let mut total = super::SpawnFallbacks::default();
        for theme in Theme::playable() {
            let mut scratch = SpawnScratch::default();
            let mut rng = StdRng::seed_from_u64(9);
            for seed in [1u64, 44] {
                let mut w = World::new(theme, seed);
                for spot in hard_standpoints(&w, seed) {
                    w.player.pos = spot;
                    for k in 0..24 {
                        super::spawn_pulse_point(
                            &mut rng,
                            &w.level,
                            &w.player,
                            w.viewport,
                            &mut scratch,
                            k % 5 == 0,
                            None,
                            k,
                        );
                    }
                }
            }
            let fb = scratch.fallbacks;
            println!(
                "  {:<11} {:>6} {:>7} {:>6} {:>7} {:>6} {:>6} {:>6}",
                format!("{theme:?}"),
                fb.ring,
                fb.ring_outward,
                fb.ring_inward,
                fb.ring_any,
                fb.room + fb.room_out_of_view,
                fb.near,
                fb.skipped
            );
            total.ring += fb.ring;
            total.ring_outward += fb.ring_outward;
            total.ring_inward += fb.ring_inward;
            total.ring_any += fb.ring_any;
            total.room += fb.room + fb.room_out_of_view;
            total.near += fb.near;
            total.skipped += fb.skipped;
            assert!(fb.ring > 0, "{theme:?}: the widest-view ring never fired");
        }
        println!(
            "  {:<11} {:>6} {:>7} {:>6} {:>7} {:>6} {:>6} {:>6}",
            "total",
            total.ring,
            total.ring_outward,
            total.ring_inward,
            total.ring_any,
            total.room,
            total.near,
            total.skipped
        );
        assert!(
            total.ring > 0,
            "the widest-view ring never carried a spawn: {total:?}"
        );
    }

    /// **Batch ten, determinism.** The ring is a pure function of the rng, the
    /// map, the rat and the *simulation* viewport, so the render-only zoom
    /// cannot move it: the same samples at `0.5`, `1` and `2` are identical
    /// (`world::zoom_tests` owns the whole-run version of this promise).
    #[test]
    fn the_spawn_ring_is_identical_at_every_zoom() {
        let mut runs = Vec::new();
        for z in [0.5f32, 1.0, 2.0] {
            let mut w = World::new(Theme::Sewer, 11);
            w.set_zoom_instant(z);
            assert_eq!(w.zoom_view, z, "the world renders at the requested zoom");
            let mut samples: Vec<(u32, u32)> = Vec::new();
            let mut rng = StdRng::seed_from_u64(7);
            for _ in 0..150 {
                let p = spawn_point(&mut rng, &w.level, &w.player, w.viewport);
                samples.push((p.x.to_bits(), p.y.to_bits()));
            }
            let mut scratch = SpawnScratch::default();
            let mut rng = StdRng::seed_from_u64(7);
            for _ in 0..60 {
                let p = spawn_point_reachable(
                    &mut rng,
                    &w.level,
                    &w.player,
                    w.viewport,
                    &mut scratch,
                );
                samples.push(match p {
                    Some(p) => (p.x.to_bits(), p.y.to_bits()),
                    None => (u32::MAX, u32::MAX),
                });
            }
            runs.push(samples);
        }
        assert_eq!(runs[0], runs[1], "z = 0.5 moved the spawn ring");
        assert_eq!(runs[1], runs[2], "z = 2 moved the spawn ring");
    }

    /// **Batch ten, pacing.** The ring moved out to the widest view, so measure
    /// what that costs: first contact and the live count over the first three
    /// minutes, on a fixed seed, with no weapons (nothing dies, so the count is
    /// the director's own) and an invulnerable rat.
    ///
    /// The old ring was `max(vp.len() * 0.5 + 16, 120)` (120 px at 120×74 and
    /// at 160×80, where the floor binds), the first pulse fires on tick 1
    /// (`Director::spawn_timer` starts spent) with one 30 px/s skitterer, and
    /// first contact in this exact loop measured 4.17 s on the commit before
    /// the ring moved. The ring is now 157 px at 120×74 and 195 px at 160×80,
    /// and the boost runs at 60 px/s outside the 1.0 view: 4.1 s.
    #[test]
    fn the_opening_still_reaches_the_rat_on_time() {
        let dt = 1.0 / 30.0;
        let mut w = World::new(Theme::Sewer, 41);
        w.player.loadout.weapons.clear();
        w.player.god = true;
        let rat = w.player.pos;
        let vp = w.viewport;
        let mut first_contact: Option<f32> = None;
        let mut named: Vec<u32> = Vec::new();
        let mut marks: Vec<(f32, usize, usize)> = Vec::new();
        let mut t = 0.0f32;
        while t < 190.0 {
            w.update(dt, Vec2::ZERO, false);
            w.player.hp = w.player.max_hp();
            w.pending_levelups = 0;
            t += dt;
            for e in w.enemies() {
                // A named arrival (the 3:00 miniboss) must come from out of
                // sight like everything else — it shares `spawn_point_reachable`.
                if e.named && !named.contains(&e.id) {
                    named.push(e.id);
                    assert!(
                        super::off_screen(e.pos, rat, vp),
                        "a miniboss arrived on screen at {:?}",
                        e.pos
                    );
                }
                // The approach boost ends at the unzoomed view's edge: nothing
                // on the 1.0 screen moves at anything but its own speed.
                // (Asserted a body's width inside that edge: a body that
                // crosses it during a tick keeps last tick's scale until the
                // next director pass.)
                let inset = Vec2::new(vp.x - 16.0, vp.y - 16.0);
                if !e.named && !e.elite && super::in_sim_view(e.pos, rat, inset) {
                    assert!(
                        (e.speed_scale - 1.0).abs() < 1e-6,
                        "{:?} moves at {}× inside the view",
                        e.kind,
                        e.speed_scale
                    );
                }
                if first_contact.is_none()
                    && e.pos.dist(rat)
                        <= crate::game::ecs::constants::PLAYER_RADIUS + e.radius + 2.0
                {
                    first_contact = Some(t);
                }
            }
            for at in [30.0f32, 60.0, 180.0] {
                if (t - at).abs() <= dt * 0.5 && !marks.iter().any(|(m, _, _)| *m == at) {
                    let near = w
                        .enemies()
                        .iter()
                        .filter(|e| e.pos.dist(rat) <= 300.0)
                        .count();
                    marks.push((at, w.enemy_count(), near));
                }
            }
        }
        let first = first_contact.expect("nothing ever reached the rat");
        println!(
            "opening (sewer, seed 41): first contact {first:.1} s; live/near at {}; named \
             arrivals {}",
            marks
                .iter()
                .map(|(at, live, near)| format!("{at:.0}s {live}/{near}"))
                .collect::<Vec<_>>()
                .join(", "),
            named.len()
        );
        // The window the old ring held (~4 s of walk) plus the extra ring, with
        // the out-of-sight boost giving most of it back.
        assert!(
            (2.5..=5.5).contains(&first),
            "first contact at {first:.1} s is outside the 2.5-5.5 s opening (it was 4.2 s on the old ring)"
        );
        for (at, live, near) in marks {
            // The budget is a step table, and it steps *at* these instants: the
            // count to hold is the one the mark has just left.
            let held = director::live_budget_at((at - 1.0).max(0.0));
            let budget = director::live_budget_at(at.min(director::BOSS_TIME));
            assert!(live <= budget, "{at}s: {live} live bodies over the {budget} budget");
            assert!(
                live * 2 >= held,
                "{at}s: only {live} live bodies of the {held} budget it has been holding"
            );
            assert!(
                near > 0,
                "{at}s: nothing within 300 px of the rat (the horde thinned out)"
            );
        }
        assert_eq!(named.len(), 1, "the 3:00 miniboss did not arrive");
    }

    /// Measurement, not a gate: bot play on every floor at two viewports, with
    /// every director placement logged. Prints how many bodies were placed
    /// inside the view the widest zoom shows (rat-centred, and through the
    /// arena-clamped camera the renderer really uses), inside the unzoomed
    /// view, and how many pulse bodies were skipped.
    ///
    /// `cargo test spawns_inside_the_widest_view_in_bot_play -- --ignored --nocapture`
    /// (`RATRUN_SPAWN_SECS` sets the seconds per run, default 240).
    #[test]
    #[ignore = "measurement"]
    fn spawns_inside_the_widest_view_in_bot_play() {
        use crate::autoplay::{AutoOpts, AutoRun};
        use crate::game::bot::Skill;
        let secs: f32 = std::env::var("RATRUN_SPAWN_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(240.0);
        println!(
            "floor       viewport  placed  in-2x(rat) in-2x(cam) in-1x  skipped  min-dist"
        );
        let mut totals = [0usize; 5];
        for theme in Theme::playable() {
            for vp in [Vec2::new(120.0, 74.0), Vec2::new(200.0, 116.0)] {
                let mut opts = if theme == Theme::Trailer {
                    AutoOpts::getaway(Skill::Skilled)
                } else {
                    AutoOpts::practice(0, Skill::Skilled)
                };
                if theme != Theme::Trailer {
                    opts.route = Some(theme);
                }
                let mut run = AutoRun::new(1000, opts);
                assert_eq!(run.world().level.theme, theme);
                run.world_mut().viewport = vp;
                run.world_mut().player.god = true;
                let arena = run.world().level.arena;
                super::SPAWN_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
                while run.world().elapsed < secs && !run.finished() {
                    run.step(1.0 / 60.0);
                }
                let log = super::SPAWN_LOG.with(|l| l.borrow_mut().take()).unwrap_or_default();
                let (mut placed, mut in_rat, mut in_cam, mut in_one, mut skipped) = (0, 0, 0, 0, 0);
                let mut min_dist = f32::MAX;
                for (p, rat, tag) in log {
                    if tag == 3 {
                        skipped += 1;
                        continue;
                    }
                    placed += 1;
                    min_dist = min_dist.min(p.dist(rat));
                    let d = p - rat;
                    if d.x.abs() < vp.x && d.y.abs() < vp.y {
                        in_rat += 1;
                    }
                    if d.x.abs() < vp.x * 0.5 && d.y.abs() < vp.y * 0.5 {
                        in_one += 1;
                    }
                    // The renderer's camera at the widest zoom: the rat-centred
                    // rectangle of `viewport × 2`, clamped to the arena.
                    let view = vp * 2.0;
                    let clamp = |c: f32, a: f32, v: f32| {
                        if a <= v {
                            (a - v) * 0.5
                        } else {
                            c.clamp(0.0, a - v)
                        }
                    };
                    let cx = clamp(rat.x - view.x * 0.5, arena.x, view.x);
                    let cy = clamp(rat.y - view.y * 0.5, arena.y, view.y);
                    if p.x > cx && p.x < cx + view.x && p.y > cy && p.y < cy + view.y {
                        in_cam += 1;
                    }
                }
                println!(
                    "{:<11} {:>3}x{:<4} {placed:>6} {in_rat:>10} {in_cam:>10} {in_one:>6} {skipped:>8} {min_dist:>9.0}",
                    format!("{theme:?}"),
                    vp.x,
                    vp.y,
                );
                for (t, v) in totals.iter_mut().zip([placed, in_rat, in_cam, in_one, skipped]) {
                    *t += v;
                }
            }
        }
        println!(
            "TOTAL                {:>6} {:>10} {:>10} {:>6} {:>8}",
            totals[0], totals[1], totals[2], totals[3], totals[4]
        );
    }

    /// Measurement, not a gate: the opening's pace in bot play on every floor
    /// (four seeds, viewport 120 × 74): first contact, first kill, first
    /// level-up, and kills / live bodies / rat level at 1:00, 3:00 and 6:00.
    ///
    /// `cargo test opening_pace_in_bot_play -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement"]
    fn opening_pace_in_bot_play() {
        use crate::autoplay::{AutoOpts, AutoRun};
        use crate::game::bot::Skill;
        println!(
            "floor       contact  kill  lvl-up | kills@1/3/6 min        | live@1/3/6 min     | level@3/6"
        );
        let mut all = [0.0f32; 11];
        let mut floors = 0.0f32;
        for theme in Theme::playable() {
            let mut sum = [0.0f32; 11];
            let seeds = [1000u64, 1001, 1002, 1003];
            for seed in seeds {
                let mut opts = if theme == Theme::Trailer {
                    AutoOpts::getaway(Skill::Skilled)
                } else {
                    AutoOpts::practice(0, Skill::Skilled)
                };
                if theme != Theme::Trailer {
                    opts.route = Some(theme);
                }
                let mut run = AutoRun::new(seed, opts);
                run.world_mut().viewport = Vec2::new(120.0, 74.0);
                run.world_mut().player.god = true;
                let (mut contact, mut kill, mut lvl) = (None, None, None);
                let marks = [60.0f32, 180.0, 360.0];
                let mut next = 0;
                while next < marks.len() && !run.finished() {
                    run.step(1.0 / 60.0);
                    let w = run.world_mut();
                    let t = w.elapsed;
                    if contact.is_none() {
                        let rat = w.player.pos;
                        let touch = w.enemies().iter().any(|e| {
                            e.pos.dist(rat)
                                <= crate::game::ecs::constants::PLAYER_RADIUS + e.radius + 2.0
                        });
                        if touch {
                            contact = Some(t);
                        }
                    }
                    if kill.is_none() && w.kills > 0 {
                        kill = Some(t);
                    }
                    if lvl.is_none() && w.player.level > 1 {
                        lvl = Some(t);
                    }
                    if t >= marks[next] {
                        sum[3 + next] += w.kills as f32;
                        sum[6 + next] += w.enemy_count() as f32;
                        if next > 0 {
                            sum[8 + next] += w.player.level as f32;
                        }
                        next += 1;
                    }
                }
                sum[0] += contact.unwrap_or(360.0);
                sum[1] += kill.unwrap_or(360.0);
                sum[2] += lvl.unwrap_or(360.0);
            }
            let n = seeds.len() as f32;
            let m: Vec<f32> = sum.iter().map(|v| v / n).collect();
            println!(
                "{:<11} {:>6.1}s {:>5.1}s {:>5.1}s | {:>6.0} {:>6.0} {:>6.0}   | {:>5.0} {:>5.0} {:>5.0}  | {:>4.1} {:>4.1}",
                format!("{theme:?}"),
                m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10]
            );
            for (a, v) in all.iter_mut().zip(&m) {
                *a += v;
            }
            floors += 1.0;
        }
        let m: Vec<f32> = all.iter().map(|v| v / floors).collect();
        println!(
            "{:<11} {:>6.1}s {:>5.1}s {:>5.1}s | {:>6.0} {:>6.0} {:>6.0}   | {:>5.0} {:>5.0} {:>5.0}  | {:>4.1} {:>4.1}",
            "MEAN", m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10]
        );
    }
}
