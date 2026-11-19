//! (b) Spawn director, spawn ring, elite + boss spawns, enemy cap.
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
//! ## Port notes
//!
//! The only structural change is the cap bookkeeping above: legacy asked
//! `self.enemies.len()` after each push (the `Vec` grew immediately), the ECS
//! port asks the query once and adds `queued`. Rng draws, their order and the
//! spawn order are unchanged — `spawn_point` draws its ring angle before
//! `Enemy::spawn` draws its three fields for every single spawn, and the elite
//! and boss branches keep consuming the same draws. `shake = 14.0` is an
//! assignment (not a `max`) exactly like the legacy code.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;
use rand::RngExt;

use crate::audio::Sfx;
use crate::game::collision::{SizeClass, TileMap};
use crate::game::director::{self, Director};
use crate::game::ecs::constants::ENEMY_CAP;
use crate::game::ecs::resources::{
    NextEnemyId, NextSpawnId, RngRes, RunCounters, Shake, SfxQueue, TickInput, Viewport,
};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{Level, RoomKind};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

// ---- Spawn ring tuning -----------------------------------------------------

/// Ring samples taken before the fallbacks (plus one mirrored sweep).
const RING_ATTEMPTS: i32 = 12;
/// Floor of the ring radius: spawns must never appear on screen.
const MIN_RING_RADIUS: f32 = 120.0;
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
/// Pulse count that must have fired before room waves start (a wave from a
/// room 1 000 px away in the first second would just be a lost enemy).
const ROOM_WAVE_AFTER_PULSES: u32 = 8;
/// Only rooms this close to the player are used for a room wave: the horde has
/// to reach the player within roughly one pulse cycle.
const ROOM_WAVE_RANGE: f32 = 1400.0;
/// Random cells [`spawn_point_in_room`] tries before giving up.
const ROOM_SAMPLES: u32 = 8;
/// Cells the wall-clear fallback searches around a blocked ring point.
const FALLBACK_SEARCH_CELLS: i32 = 10;

/// Cells of a cell rectangle's inset that a room spawn stays inside.
const ROOM_INSET: i32 = 2;

/// Legacy `World::run_director` .
#[allow(clippy::too_many_arguments)]
pub fn director_system(
    mut commands: Commands,
    input: Res<TickInput>,
    counters: Res<RunCounters>,
    viewport: Res<Viewport>,
    level: Res<Level>,
    player: Res<Player>,
    mut director: ResMut<Director>,
    mut rng: ResMut<RngRes>,
    mut next_id: ResMut<NextSpawnId>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut particles: ResMut<Particles>,
    mut shake: ResMut<Shake>,
    mut sfx: ResMut<SfxQueue>,
    enemies: Query<&Enemy>,
    // Per-run scratch, reused across ticks: the per-level reachability cache
    // costs one flood fill per map and must not be recomputed per spawn.
    mut scratch: Local<SpawnScratch>,
) {
    let dt = input.dt;
    let elapsed = counters.elapsed;
    // Keep one slot reserved for the mandatory boss until it has spawned, so
    // the live count stays `<= ENEMY_CAP` for the whole run.
    let cap = if director.boss_spawned {
        ENEMY_CAP
    } else {
        ENEMY_CAP - 1
    };
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

    director.spawn_timer -= dt;
    if director.spawn_timer <= 0.0 {
        director.spawn_timer = director::spawn_interval(elapsed);
        let count = director::pulse_count(elapsed);
        let mult = director::hp_mult(elapsed);
        let live = enemies.iter().count();
        // One pulse in `ROOM_WAVE_EVERY` brings its first enemy in through a
        // doorway of a room the player is not in, so the horde arrives instead
        // of all of it materialising on the ring.
        let room_wave = director.pulses >= ROOM_WAVE_AFTER_PULSES
            && director.pulses.is_multiple_of(ROOM_WAVE_EVERY);
        director.pulses = director.pulses.wrapping_add(1);
        for i in 0..count {
            if live + queued >= cap {
                break;
            }
            let kind = director::pick_kind(elapsed, &mut rng.0);
            let pos = spawn_pulse_point(
                &mut rng.0,
                &level,
                &player,
                viewport.0,
                &mut scratch,
                room_wave && i == 0,
            );
            let mut enemy = Enemy::spawn(kind, pos, mult, &mut rng.0);
            // Neon merge: legacy `run_director` ids every director-spawned
            // enemy from the monotonic `next_enemy_id` counter (0 is reserved
            // for "anonymous enemy" in `DamageNumbers`). `SpawnId` and the
            // enemy id are two independent orderings; never share them.
            enemy.id = next_enemy_id.take();
            commands.spawn((enemy, next_id.take()));
            queued += 1;
        }
    }

    // Periodic elite brute. Skip it instead of taking the boss's slot.
    director.elite_timer -= dt;
    if director.elite_timer <= 0.0 && elapsed > director::ELITE_START {
        director.elite_timer = director::ELITE_INTERVAL;
        if enemies.iter().count() + queued < cap {
            let pos = spawn_point_reachable(
                &mut rng.0,
                &level,
                &player,
                viewport.0,
                &mut scratch,
            );
            let mult = director::hp_mult(elapsed) * 1.6;
            let mut enemy = Enemy::spawn(EnemyKind::Brute, pos, mult, &mut rng.0);
            enemy.id = next_enemy_id.take();
            // Director elites are flagged so the art and death VFX can tell
            // them apart from late-game Brutes with overlapping scaled HP.
            enemy.elite = true;
            commands.spawn((enemy, next_id.take()));
        }
    }

    // Boss.
    if !director.boss_spawned && elapsed >= director::BOSS_TIME {
        director.boss_spawned = true;
        // The boss stays a ring spawn at `BOSS_TIME` (the five-minute threat is
        // unchanged); only its placement gets the wall/reachability filter.
        let pos = spawn_point_reachable(
            &mut rng.0,
            &level,
            &player,
            viewport.0,
            &mut scratch,
        );
        let mut enemy = Enemy::spawn(EnemyKind::Boss, pos, 1.0, &mut rng.0);
        enemy.id = next_enemy_id.take();
        commands.spawn((enemy, next_id.take()));
        // Neon merge: arrival shockwave + cross burst, drawn before the shake
        // and SFX exactly like `run_director` in the merged reference.
        let accent = level.palette.accent;
        vfx::emit_boss_spawn(&mut particles, &mut rng.0, pos, accent);
        shake.amount = 14.0;
        sfx.0.push(Sfx::Boss);
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

/// Facts about one level that every spawn needs but that are far too expensive
/// to recompute per spawn: which cells the horde can actually path to, and which
/// rooms contain such a cell.
///
/// The map is static for a whole run, so this is built **once** (at the first
/// pulse) and then read as O(1) table lookups — the "cache a per-room
/// reachability flag" the content plan asks for instead of a flood fill per
/// spawn. It is keyed by the level's identity (`seed`, `theme`, room count), so
/// a test that swaps in a different map rebuilds it automatically; a test that
/// repaints a map in place must give it a different seed.
#[derive(Default)]
pub struct SpawnScratch {
    key: Option<(u64, u8, usize)>,
    /// One flag per cell, row-major: can a `Large` agent path here from the
    /// start room (one-way doors honoured)?
    reach: Vec<bool>,
    /// One flag per entry in `Level::rooms`: does the room hold a reachable
    /// cell (so the horde can enter it)?
    room_ok: Vec<bool>,
}

impl SpawnScratch {
    /// (Re)build the cached facts when this is a different map.
    pub fn sync(&mut self, level: &Level) {
        let key = (level.seed, level.theme as u8, level.rooms.len());
        if self.key == Some(key) && self.reach.len() == (level.tiles.cols() * level.tiles.rows()) as usize {
            return;
        }
        self.key = Some(key);
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
        self.reach
            .get((cy * level.tiles.cols() + cx) as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Can the horde enter `room` (any reachable cell inside it)?
    pub fn room_reachable(&self, index: usize) -> bool {
        self.room_ok.get(index).copied().unwrap_or(false)
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

/// Radius of the spawn ring: just beyond the viewport, floored so a small
/// terminal still spawns off screen.
#[inline]
pub fn ring_radius(viewport: Vec2) -> f32 {
    (viewport.len() * 0.5 + 16.0).max(MIN_RING_RADIUS)
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
    let base = ring_base_angle(rng, player);
    if let Some(p) = sample_ring(level, player, viewport, base, None) {
        return p;
    }
    fallback_point(level, player, viewport, base, None)
}

/// The director's spawn point: [`spawn_point`] plus the cached reachability
/// test, so an enemy never materialises in a pocket the horde cannot leave.
/// When the whole ring is walled in (the rat ducking through a rat-hole or into
/// a maze aisle) it spawns in the nearest room the horde can enter instead, and
/// only then falls back to the nearest wall-clear cell.
pub fn spawn_point_reachable(
    rng: &mut StdRng,
    level: &Level,
    player: &Player,
    viewport: Vec2,
    scratch: &mut SpawnScratch,
) -> Vec2 {
    scratch.sync(level);
    let base = ring_base_angle(rng, player);
    let reach = &scratch.reach;
    if let Some(p) = sample_ring(level, player, viewport, base, Some(reach)) {
        return p;
    }
    if let Some(p) = nearest_reachable_room_cell(level, scratch, player, SizeClass::Large) {
        return p;
    }
    fallback_point(level, player, viewport, base, Some(reach))
}

/// Where the `room_wave`-th enemy of a pulse spawns: a wall-clear, reachable
/// cell inside a room the player is **not** in (so the horde arrives through a
/// doorway), or the ordinary ring when no such room is close enough.
pub fn spawn_pulse_point(
    rng: &mut StdRng,
    level: &Level,
    player: &Player,
    viewport: Vec2,
    scratch: &mut SpawnScratch,
    room_wave: bool,
) -> Vec2 {
    if room_wave {
        if let Some(p) = room_wave_point(rng, level, player, scratch) {
            return p;
        }
    }
    spawn_point_reachable(rng, level, player, viewport, scratch)
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
    let mut count = 0u32;
    for (i, room) in level.rooms.iter().enumerate() {
        if room_wave_ok(scratch, i, room, player) {
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let pick = rng.random_range(0..count);
    let mut seen = 0u32;
    for (i, room) in level.rooms.iter().enumerate() {
        if !room_wave_ok(scratch, i, room, player) {
            continue;
        }
        if seen == pick {
            return spawn_point_in_room(rng, level, room, SizeClass::Large);
        }
        seen += 1;
    }
    None
}

/// Is this room a legal room-wave target?
#[inline]
fn room_wave_ok(
    scratch: &SpawnScratch,
    index: usize,
    room: &crate::game::level::Room,
    player: &Player,
) -> bool {
    room_staging_ok(scratch, index, room)
        && !room.rect.contains_world(player.pos)
        && room.center().dist(player.pos) <= ROOM_WAVE_RANGE
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
fn room_open_cell(level: &Level, room: &crate::game::level::Room, class: SizeClass) -> Option<Vec2> {
    let area = room.rect.inflate(-ROOM_INSET);
    if area.w <= 0 || area.h <= 0 {
        return None;
    }
    let clear = room_clearance(class);
    let mut found = None;
    for (cx, cy) in area.cells() {
        let p = TileMap::tile_rect(cx, cy).center();
        if level.tiles.passable(cx, cy, class) && level.tiles.circle_clear(p, clear, class) {
            found = Some(p);
            break;
        }
    }
    found
}

/// The nearest room the horde can enter, and a wall-clear cell inside it.
fn nearest_reachable_room_cell(
    level: &Level,
    scratch: &SpawnScratch,
    player: &Player,
    class: SizeClass,
) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2)> = None;
    for (i, room) in level.rooms.iter().enumerate() {
        if !room_staging_ok(scratch, i, room) {
            continue;
        }
        let d = room.center().dist_sq(player.pos);
        if best.is_some_and(|(bd, _)| bd <= d) {
            continue;
        }
        if let Some(p) = room_open_cell(level, room, class) {
            best = Some((d, p));
        }
    }
    best.map(|(_, p)| p)
}

/// A wall-clear, reachable cell inside `room` for `class`, sampled from `rng`.
///
/// `None` when the room has no cell that fits the class with clearance (a room
/// crowded with props, or a `Small`-only room). Always takes the same number of
/// rng draws for the same outcome, and never loops forever.
pub fn spawn_point_in_room(
    rng: &mut StdRng,
    level: &Level,
    room: &crate::game::level::Room,
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
        let p = TileMap::tile_rect(cx, cy).center();
        if level.tiles.passable(cx, cy, class) && level.tiles.circle_clear(p, clear, class) {
            return Some(p);
        }
    }
    None
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
fn sample_ring(
    level: &Level,
    player: &Player,
    viewport: Vec2,
    base: f32,
    reach: Option<&[bool]>,
) -> Option<Vec2> {
    let r = ring_radius(viewport);
    let lo = Vec2::splat(SPAWN_MARGIN);
    let hi = level.arena - Vec2::splat(SPAWN_MARGIN);
    let step = std::f32::consts::TAU / RING_ATTEMPTS as f32;
    // Two passes over the same fixed candidate ring: the first insists the
    // point is off screen (so nothing is seen to materialise), the second takes
    // any legal point. Candidate order never depends on the rng, so the result
    // is deterministic.
    for strict in [true, false] {
        for k in 0..=RING_ATTEMPTS {
            // The last candidate mirrors the first angle: a cheap second sweep.
            let a = if k == RING_ATTEMPTS {
                -base
            } else {
                base + k as f32 * step
            };
            let p = player.pos + Vec2::from_angle(a) * r;
            if strict && !off_screen(p, player.pos, viewport) {
                continue;
            }
            if !spawn_point_ok(p, lo, hi, SPAWN_MIN_DIST, SPAWN_WALL_CLEAR, level, player.pos) {
                continue;
            }
            if !cell_reachable(reach, level, p) {
                continue;
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
) -> Vec2 {
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
            return q;
        }
    }
    p.clamp_box(Vec2::splat(6.0), level.arena - Vec2::splat(6.0))
}

/// Is `p` outside the screen the player is looking at? The camera is centred on
/// the player, so this is a plain box test around him.
#[inline]
fn off_screen(p: Vec2, player: Vec2, viewport: Vec2) -> bool {
    let d = p - player;
    let hw = viewport.x * 0.5 + OFF_SCREEN_MARGIN;
    let hh = viewport.y * 0.5 + OFF_SCREEN_MARGIN;
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
    reach
        .get((cy * level.tiles.cols() + cx) as usize)
        .copied()
        .unwrap_or(false)
}

/// Nearest wall-clear cell centre to `p`, searching ring by ring. Prefers a
/// cell the horde can reach when a reachability table is supplied, and falls
/// back to plain clearance so the result is never inside a wall. `min_from` is
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
    for require_reach in [true, false] {
        if require_reach && reach.is_none() {
            continue;
        }
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
                    let c = TileMap::tile_rect(cx, cy).center();
                    if let Some(from) = min_from {
                        if c.dist_sq(from) < min_dist * min_dist {
                            continue;
                        }
                    }
                    if require_reach && !cell_reachable(reach, level, c) {
                        continue;
                    }
                    if level.tiles.circle_clear(c, SPAWN_WALL_CLEAR, class) {
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
    level.tiles.circle_clear(p, wall_clear, SizeClass::Large)
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::RngExt;
    use rand::SeedableRng;

    use super::{
        director_system, ring_radius, spawn_point, spawn_point_in_room, spawn_point_ok,
        spawn_point_reachable, SpawnScratch,
    };
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{circle_rect_overlap, SizeClass, TileKind, TileMap, TILE_F};
    use crate::game::director::{self, Director};
    use crate::game::ecs::constants::{ENEMY_CAP, PARTICLE_CAP};
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::{Level, Rectf, RoomKind, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::particle::{vfx, Particles};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Strip the generated layout so the ring has room (what
    /// `Level::walls.clear()` used to do before the tile grid owned
    /// collision).
    fn open_level(level: &mut Level) {
        level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, level.theme, 0);
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
        assert!(
            (r - (Vec2::new(600.0, 400.0).len() * 0.5 + 16.0)).abs() < 1e-3,
            "ring radius does not follow the viewport: {r}"
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
        // Force a pulse this tick; at 100 s it asks for 6 enemies but only 3
        // slots exist (one is reserved for the boss).
        w.director.spawn_timer = 0.0;
        director_tick(&mut w, 0.1, 100.0);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 1);
        assert!(!w.director.boss_spawned);
    }

    /// The reserved slot holds for the whole run and is only spent by the boss.
    #[test]
    fn director_reserves_one_slot_for_the_boss_until_it_spawns() {
        let mut w = World::new(Theme::Sewer, 17);
        let dt = 0.1;
        let mut elapsed = 0.0;
        loop {
            elapsed += dt;
            assert!(elapsed < director::BOSS_TIME);
            director_tick(&mut w, dt, elapsed);
            assert!(
                w.enemies().len() < ENEMY_CAP,
                "cap exceeded at {elapsed:.1}s: {}",
                w.enemies().len()
            );
            assert!(!w.director.boss_spawned);
            if w.enemies().len() == ENEMY_CAP - 1 {
                break;
            }
        }
        // The reserved slot is still free: the next tick past BOSS_TIME fills
        // it, and the cap still holds.
        director_tick(&mut w, dt, director::BOSS_TIME);
        assert!(w.director.boss_spawned);
        assert_eq!(w.enemies().len(), ENEMY_CAP);
        assert!(w.enemies().iter().any(|e| e.kind == EnemyKind::Boss));
    }

    /// Boss spawn *assigns* `shake = 14.0` (the legacy code, not a `max`) and
    /// pushes `Sfx::Boss`.
    #[test]
    fn boss_spawn_shakes_the_screen_and_plays_the_boss_sfx() {
        let mut w = World::new(Theme::Sewer, 19);
        w.shake = 40.0;
        w.sfx.clear();
        director_tick(&mut w, 0.1, director::BOSS_TIME);
        assert!(w.director.boss_spawned);
        assert_eq!(w.shake, 14.0);
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::Boss).count(), 1);
    }

    /// The boss spawns exactly once.
    #[test]
    fn boss_spawns_once() {
        let mut w = World::new(Theme::Sewer, 23);
        director_tick(&mut w, 0.1, director::BOSS_TIME + 3.0);
        let first = w
            .enemies()
            .iter()
            .filter(|e| e.kind == EnemyKind::Boss)
            .count();
        assert_eq!(first, 1);
        director_tick(&mut w, 0.1, director::BOSS_TIME + 300.0);
        let second = w
            .enemies()
            .iter()
            .filter(|e| e.kind == EnemyKind::Boss)
            .count();
        assert_eq!(second, 1, "the boss respawned");
    }

    /// Elites are gated on `elapsed > ELITE_START` (strictly greater), spawn a
    /// Brute at 1.6× the current hp mult and reset the timer to the interval.
    #[test]
    fn elites_start_after_the_gate_and_scale_their_hp() {
        let mut w = World::new(Theme::Sewer, 29);
        w.director.elite_timer = 0.0;
        // Exactly at the gate: no elite (legacy `elapsed > ELITE_START`).
        director_tick(&mut w, 0.0, director::ELITE_START);
        assert!(!w.enemies().iter().any(|e| e.kind == EnemyKind::Brute));

        director_tick(&mut w, 0.1, director::ELITE_START + 0.1);
        let enemies = w.enemies();
        let brute = enemies
            .iter()
            .find(|e| e.kind == EnemyKind::Brute)
            .expect("an elite brute must spawn once the gate opens");
        assert_eq!(w.director.elite_timer, director::ELITE_INTERVAL);
        let expected =
            EnemyKind::Brute.base_hp() * director::hp_mult(director::ELITE_START + 0.1) * 1.6;
        assert!((brute.max_hp - expected).abs() < 1e-3, "hp = {}", brute.hp);
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
        director_tick(&mut w, 0.1, director::ELITE_START + 1.0);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 1, "elite took the boss slot");
        assert!(!w.enemies().iter().any(|e| e.kind == EnemyKind::Brute));
    }

    /// A tick that both pulses *and* crosses the boss gate still respects the
    /// cap: the pulse is blocked, the boss still gets its reserved slot.
    #[test]
    fn a_late_tick_blocked_at_the_cap_still_spawns_the_boss() {
        let mut w = World::new(Theme::Sewer, 43);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 1) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        w.director.spawn_timer = 0.0;
        director_tick(&mut w, 500.0, director::BOSS_TIME);
        assert_eq!(w.enemies().len(), ENEMY_CAP);
        assert!(w.director.boss_spawned);
    }

    /// The neon-era legacy body of `run_director`, kept as an independent
    /// reference so the rng
    /// draw order can be compared tick by tick — including the enemy-id
    /// counter and the boss-arrival VFX, both of which consume rng or touch
    /// state in the same order the system must.
    #[allow(clippy::too_many_arguments)]
    fn legacy_director_tick(
        dt: f32,
        elapsed: f32,
        director: &mut Director,
        rng: &mut StdRng,
        level: &Level,
        player: &Player,
        viewport: Vec2,
        particles: &mut Particles,
        next_enemy_id: &mut u32,
        scratch: &mut SpawnScratch,
        out: &mut Vec<Enemy>,
    ) {
        let cap = if director.boss_spawned {
            ENEMY_CAP
        } else {
            ENEMY_CAP - 1
        };

        director.spawn_timer -= dt;
        if director.spawn_timer <= 0.0 {
            director.spawn_timer = director::spawn_interval(elapsed);
            let count = director::pulse_count(elapsed);
            let mult = director::hp_mult(elapsed);
            let room_wave = director.pulses >= super::ROOM_WAVE_AFTER_PULSES
                && director.pulses.is_multiple_of(super::ROOM_WAVE_EVERY);
            director.pulses = director.pulses.wrapping_add(1);
            for i in 0..count {
                if out.len() >= cap {
                    break;
                }
                let kind = director::pick_kind(elapsed, rng);
                let pos = super::spawn_pulse_point(
                    rng,
                    level,
                    player,
                    viewport,
                    scratch,
                    room_wave && i == 0,
                );
                let mut e = Enemy::spawn(kind, pos, mult, rng);
                e.id = *next_enemy_id;
                *next_enemy_id = next_enemy_id.wrapping_add(1).max(1);
                out.push(e);
            }
        }

        director.elite_timer -= dt;
        if director.elite_timer <= 0.0 && elapsed > director::ELITE_START {
            director.elite_timer = director::ELITE_INTERVAL;
            if out.len() < cap {
                let pos = super::spawn_point_reachable(rng, level, player, viewport, scratch);
                let mult = director::hp_mult(elapsed) * 1.6;
                let mut e = Enemy::spawn(EnemyKind::Brute, pos, mult, rng);
                e.id = *next_enemy_id;
                e.elite = true;
                *next_enemy_id = next_enemy_id.wrapping_add(1).max(1);
                out.push(e);
            }
        }

        if !director.boss_spawned && elapsed >= director::BOSS_TIME {
            director.boss_spawned = true;
            let pos = super::spawn_point_reachable(rng, level, player, viewport, scratch);
            let mut e = Enemy::spawn(EnemyKind::Boss, pos, 1.0, rng);
            e.id = *next_enemy_id;
            *next_enemy_id = next_enemy_id.wrapping_add(1).max(1);
            out.push(e);
            let accent = level.palette.accent;
            vfx::emit_boss_spawn(particles, rng, pos, accent);
        }
    }

    /// Everything `spawn_point` and `Enemy::spawn` derive from the rng, so any
    /// divergence in the draw order shows up.
    fn fingerprint(e: &Enemy) -> (EnemyKind, bool, Vec<u32>) {
        let fields = [
            e.pos.x, e.pos.y, e.hp, e.max_hp, e.radius, e.damage, e.fire_cd, e.wander, e.anim,
        ];
        (
            e.kind,
            e.elite,
            fields.iter().map(|f| f.to_bits()).collect(),
        )
    }

    /// The system is a line-by-line port: the same seed, clock, level and
    /// viewport must produce the same enemies in the same order as the legacy
    /// body, which only holds if every rng draw (count, range and order)
    /// matches.
    #[test]
    fn director_matches_the_legacy_draw_order() {
        let seed = 41;
        let mut w = World::new(Theme::Sewer, seed);

        // The spawn ring depends on the map (reachability, rooms), so the
        // reference must play the *same* map: `Level::generate` is the same
        // deterministic call `World::new` made, and no other system moves the
        // player during a director-only run.
        let level = Level::generate(Theme::Sewer, seed);
        let player = Player::new(level.start, WeaponKind::Gnaw);
        assert_eq!(w.player.pos, player.pos);
        assert_eq!(w.level.arena, level.arena);
        let viewport = w.viewport;

        let mut ref_rng = StdRng::seed_from_u64(seed);
        let mut ref_director = Director::default();
        let mut ref_particles = Particles::new(PARTICLE_CAP);
        let mut ref_scratch = SpawnScratch::default();
        let mut ref_next_enemy_id: u32 = 1;
        let mut reference: Vec<Enemy> = Vec::new();

        let dt = 0.5;
        let mut elapsed = 0.0;
        let mut compared = 0usize;
        for _ in 0..620 {
            elapsed += dt;
            legacy_director_tick(
                dt,
                elapsed,
                &mut ref_director,
                &mut ref_rng,
                &level,
                &player,
                viewport,
                &mut ref_particles,
                &mut ref_next_enemy_id,
                &mut ref_scratch,
                &mut reference,
            );
            director_tick(&mut w, dt, elapsed);

            let port = w.enemies();
            assert_eq!(port.len(), reference.len(), "count at {elapsed:.2}s");
            for (i, expected) in reference.iter().enumerate().skip(compared) {
                assert_eq!(
                    fingerprint(&port[i]),
                    fingerprint(expected),
                    "enemy {i} differs at {elapsed:.2}s"
                );
            }
            compared = reference.len();
            assert_eq!(w.director.spawn_timer, ref_director.spawn_timer);
            assert_eq!(w.director.elite_timer, ref_director.elite_timer);
            assert_eq!(w.director.boss_spawned, ref_director.boss_spawned);
        }
        assert_eq!(compared, ENEMY_CAP, "the run must exercise the cap");
        assert!(w.director.boss_spawned, "the run must reach the boss");
        // In a director-only run the port's SpawnId-derived enemy ids land on
        // exactly the legacy values (1, 2, 3, ...), so the ids above are a real
        // equivalence check, not just uniqueness.
        assert_eq!(
            w.enemies().first().map(|e| e.id),
            Some(1),
            "first director enemy must get id 1"
        );
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
        for _ in 0..240 {
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

        // Now the elite branch fires: exactly one flagged Brute.
        w.director.elite_timer = 0.0;
        director_tick(&mut w, 0.1, 200.1);
        let enemies = w.enemies();
        let elites: Vec<&Enemy> = enemies.iter().filter(|e| e.elite).collect();
        assert_eq!(elites.len(), 1, "exactly one elite per elite spawn");
        assert_eq!(elites[0].kind, EnemyKind::Brute);
        assert_ne!(elites[0].id, 0, "the elite still needs a runtime id");
    }

    /// Hook: `vfx::emit_boss_spawn(...)` right after the boss is pushed and
    /// before the shake/SFX, so the arrival shockwave and cross burst exist on
    /// the boss tick.
    #[test]
    fn boss_arrival_emits_the_spawn_vfx() {
        let mut w = World::new(Theme::Sewer, 59);
        assert_eq!(w.particles.shockwaves.len(), 0);
        let particles_before = w.particles.len();

        director_tick(&mut w, 0.1, director::BOSS_TIME);

        assert!(w.director.boss_spawned);
        assert_eq!(
            w.particles.shockwaves.len(),
            1,
            "boss arrival should push its shockwave"
        );
        assert!(
            w.particles.len() > particles_before,
            "boss arrival should emit particles"
        );
        // Still the legacy shake + SFX.
        assert_eq!(w.shake, 14.0);
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::Boss).count(), 1);
    }

    /// The boss VFX draws from the same rng the spawn used, in the same place:
    /// the tick after the boss spawn must continue the identical draw sequence.
    #[test]
    fn boss_vfx_draws_do_not_disturb_the_following_tick() {
        let seed = 61;
        let mut w = World::new(Theme::Sewer, seed);

        let level = Level::generate(Theme::Sewer, seed);
        let player = Player::new(level.start, WeaponKind::Gnaw);
        let viewport = w.viewport;
        let mut ref_rng = StdRng::seed_from_u64(seed);
        let mut ref_director = Director::default();
        let mut ref_particles = Particles::new(PARTICLE_CAP);
        let mut ref_scratch = SpawnScratch::default();
        let mut ref_next_enemy_id: u32 = 1;
        let mut reference: Vec<Enemy> = Vec::new();

        let dt = 0.5;
        let mut elapsed = 0.0;
        for _ in 0..560 {
            elapsed += dt;
            legacy_director_tick(
                dt,
                elapsed,
                &mut ref_director,
                &mut ref_rng,
                &level,
                &player,
                viewport,
                &mut ref_particles,
                &mut ref_next_enemy_id,
                &mut ref_scratch,
                &mut reference,
            );
            director_tick(&mut w, dt, elapsed);
        }
        // Past BOSS_TIME: the boss VFX consumed rng inside the spawn tick, so
        // the next pulse's enemies only match if the draw order is unchanged.
        assert!(w.director.boss_spawned);
        assert_eq!(w.enemies().len(), reference.len());
        assert_eq!(
            w.enemies()
                .iter()
                .map(|e| e.fire_cd.to_bits())
                .collect::<Vec<_>>(),
            reference
                .iter()
                .map(|e| e.fire_cd.to_bits())
                .collect::<Vec<_>>(),
            "the boss VFX rng draws disturbed the following spawns"
        );
        assert_eq!(w.particles.shockwaves.len(), 1);
        assert_eq!(ref_particles.shockwaves.len(), 1);
    }

    // ---- Content update: spawn pacing on the big procedural maps ----------

    /// Wall clearance every ring point must keep (mirrors `SPAWN_WALL_CLEAR`).
    const RING_CLEAR: f32 = 4.0;

    /// The ring sits just beyond the viewport (and never closer than 120 px),
    /// so a spawn is never on screen — that is what makes "just outside the
    /// camera" work on any terminal size.
    #[test]
    fn the_ring_radius_follows_the_viewport() {
        assert!((ring_radius(Vec2::splat(1.0)) - 120.0).abs() < 1e-4);
        let vp = Vec2::new(600.0, 400.0);
        assert!((ring_radius(vp) - (vp.len() * 0.5 + 16.0)).abs() < 1e-4);
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
                            // The start room is wide enough for the whole ring,
                            // so the minimum distance must hold there too.
                            assert!(
                                p.dist(spot) >= 89.5,
                                "{theme:?} seed {seed}: spawn {p:?} too close to the player"
                            );
                        }
                    }
                }
                maps += 1;
            }
        }
        assert_eq!(maps, 24, "the sweep must cover every theme");
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
                        );
                        assert!(
                            scratch.is_reachable(&w.level, p),
                            "{theme:?} seed {seed}: spawn {p:?} is unreachable for the horde"
                        );
                        assert!(
                            w.level.tiles.circle_clear(p, RING_CLEAR, SizeClass::Large),
                            "{theme:?} seed {seed}: reachable spawn {p:?} overlaps a wall"
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
    /// staging room, always reachable for the horde.
    #[test]
    fn room_waves_stage_enemies_outside_the_players_room() {
        let mut hits = 0;
        for theme in Theme::all() {
            for seed in [3u64, 17, 1234] {
                let mut w = World::new(theme, seed);
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                // Stand in an open hall so another room is inside wave range.
                if let Some(centre) = w
                    .level
                    .rooms
                    .iter()
                    .find(|r| matches!(r.kind, RoomKind::Hall | RoomKind::Room))
                    .map(|r| r.center())
                {
                    w.player.pos = centre;
                }
                let mut a = StdRng::seed_from_u64(seed);
                let mut b = StdRng::seed_from_u64(seed);
                for _ in 0..20 {
                    let pa = super::room_wave_point(&mut a, &w.level, &w.player, &mut scratch);
                    let pb = super::room_wave_point(&mut b, &w.level, &w.player, &mut scratch);
                    assert_eq!(pa, pb, "{theme:?} seed {seed}: room wave is not deterministic");
                    let Some(p) = pa else { continue };
                    hits += 1;
                    assert!(
                        scratch.is_reachable(&w.level, p),
                        "{theme:?} seed {seed}: wave spawn {p:?} is unreachable"
                    );
                    assert!(w.level.tiles.circle_clear(p, RING_CLEAR, SizeClass::Large));
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
                }
            }
        }
        assert!(hits > 0, "no room wave ever found a staging room");
    }

    /// The pacing curve fills a map ten times larger: at least 40 live enemies
    /// before 60 s, and the whole 600-enemy cap by the end of the run. Driven
    /// through the real director only (nobody moves or dies), so it is exact
    /// and cheap.
    #[test]
    fn the_pacing_curve_reaches_forty_early_and_the_cap_late() {
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
            early_peak >= 40,
            "only {early_peak} enemies before 60 s: the horde never builds"
        );
        assert_eq!(peak, ENEMY_CAP, "the curve never fills the 600-enemy cap");
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
}
