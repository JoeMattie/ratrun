//! The Sewer's own rules (`WAVE4_ROSTERS.md` "Sewer"): the two behaviours the
//! floor's new kinds add on top of their archetypes.
//!
//! | kind | build | rule here |
//! |---|---|---|
//! | Pipe Eel | V(Cat) | steers at **half turning rate** ([`pipe_eel_before`] / [`pipe_eel_after`], called around `Enemy::steer` in `enemy_move::enemy_steering_system`) |
//! | Sump Leech | N | the tongue: at ≤ [`LEECH_TRIGGER_RANGE`] it stops, warns for [`LEECH_WARN_SECS`], holds a straight [`LEECH_TONGUE_LEN`]×[`LEECH_TONGUE_WIDTH`] tongue for [`LEECH_HOLD_SECS`], then rests [`LEECH_REST_SECS`]. A touch slows the rat 30 % for 1 s, once per extension ([`floor_sewer_system`]) |
//! | Grate Archer | R(Spitter) | none (the Spitter's shot, its own numbers) |
//! | Sluice Toad | R(Brute) | none |
//!
//! ## The leech is layered, not graduated
//!
//! `EnemyKind::SumpLeech.archetype()` is still `Pillbug`: radius, nav size
//! class, glue units, steering and the bot's reading all key on it and are
//! untouched. The one Pillbug rule the leech must not run is the curl, which
//! `enemy_wave3::wave3_system` skips for it (one line there). Its own state
//! lives in the [`SumpLeech`] component; the three numbers the `&Enemy`
//! readers need (art, bot) are mirrored into the body's otherwise-unused `w4`
//! scratch (`phase`, `t`, `dir`, `warn_age`), which no wave-4 code reads for a
//! Pillbug-archetype body (`W4::holds` and `is_wave4` are both false).
//!
//! ## Determinism
//!
//! Bodies are walked in `SpawnId` order. **No rng is drawn** anywhere in this
//! file, and with no Sump Leech on the floor and no slow running the system
//! returns before its sort. The Pipe Eel hook is pure arithmetic on one body.
//!
//! ## Placement
//!
//! [`floor_sewer_system`] belongs right after `enemy_wave4::wave4_system`.
//! [`floor_sewer_slow_system`] belongs right after `hazards::hazards_system`
//! (before `player`), where the ink pad's slow is applied, so the tongue's
//! slow is exact; without it the late system's velocity clip still drags the
//! rat, only less precisely.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::{ENEMY_CAP, PLAYER_RADIUS};
use crate::game::ecs::resources::{
    Cvars, DebugFlags, LevelIndex, NextEnemyId, NextSpawnId, RunCounters, SfxQueue, TickInput,
    Viewport,
};
use crate::game::ecs::systems::{enemy_wave3::PILLBUG_CURL_SECS, spawn::off_screen};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Pickup, PickupKind};
use crate::game::level::Level;
use crate::game::player::Player;
use crate::math::Vec2;

const OPENING_PILLBUGS_AT: f32 = 22.0;
pub(super) const OPENING_PILLBUGS_COUNT: usize = 2;
pub(super) const OPENING_BOTTLE_CAP_AT: f32 = 90.0;

#[derive(Default, Resource)]
pub struct OpeningSpawns {
    pub(super) pillbugs: bool,
    bottle_cap: bool,
}

/// Fixed Sewer encounters, with no director roll and no run rng draws.
/// If the room is on-screen at 0:22, hold the pair until both can enter unseen.
#[allow(clippy::too_many_arguments)] // Independent ECS queries and resources.
pub fn sewer_opening_system(
    mut commands: Commands,
    level: Res<Level>,
    level_index: Res<LevelIndex>,
    counters: Res<RunCounters>,
    player: Res<Player>,
    viewport: Res<Viewport>,
    debug: Res<DebugFlags>,
    cvars: Res<Cvars>,
    enemies: Query<&Enemy>,
    mut next_id: ResMut<NextSpawnId>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut spawned: ResMut<OpeningSpawns>,
) {
    let Some(opening) = level.objects.sewer_opening.as_ref().filter(|_| level_index.0 == 0) else {
        return;
    };
    let elapsed = counters.elapsed;
    if !spawned.pillbugs
        && (OPENING_PILLBUGS_AT..crate::game::director::BOSS_TIME).contains(&elapsed)
        && !debug.peace
    {
        let cap = if cvars.enemy_cap == 0 {
            ENEMY_CAP
        } else {
            cvars.enemy_cap
        };
        if enemies.iter().count() + OPENING_PILLBUGS_COUNT < cap {
            let mut hidden = opening
                .pillbug_cells
                .iter()
                .copied()
                .filter(|&p| off_screen(p, player.pos, viewport.0));
            if let (Some(a), Some(b)) = (hidden.next(), hidden.next()) {
                for pos in [a, b] {
                    let mut enemy = Enemy::spawn_fixed(EnemyKind::Pillbug, pos, 1.0);
                    enemy.id = next_enemy_id.take();
                    enemy.facing = (player.pos - pos).normalized();
                    enemy.w3.curl_t = PILLBUG_CURL_SECS;
                    enemy.w3.opening_curl = true;
                    commands.spawn((enemy, next_id.take()));
                }
                spawned.pillbugs = true;
            }
        }
    }
    if !spawned.bottle_cap && elapsed >= OPENING_BOTTLE_CAP_AT {
        commands.spawn((
            Pickup {
                pos: opening.bottle_cap,
                kind: PickupKind::BottleCap,
                bob: 0.0,
            },
            next_id.take(),
        ));
        spawned.bottle_cap = true;
    }
}

// ---- Pipe Eel -------------------------------------------------------------

/// `Enemy::steer` eases velocity toward the desired one at this rate (1/s).
pub const STEER_RATE: f32 = 6.0;
/// The Pipe Eel's: half of it.
pub const PIPE_EEL_STEER_RATE: f32 = STEER_RATE * 0.5;

/// What [`pipe_eel_after`] needs from before the steer: the eel's velocity and
/// whether it was stunned (a stunned body does not steer). `None` for every
/// other kind, so the hook costs one compare.
#[inline]
pub fn pipe_eel_before(e: &Enemy) -> Option<(Vec2, bool)> {
    if e.kind == EnemyKind::PipeEel {
        Some((e.vel, e.stun > 0.0))
    } else {
        None
    }
}

/// Re-do this tick's velocity ease at half rate.
///
/// `Enemy::steer` computed `new = old + (desired - old) * min(6 dt, 1)` and
/// moved the body by `new * dt`. The eel's own ease is
/// `old + (desired - old) * min(3 dt, 1)`, which is `old + (new - old) * k`
/// with `k = min(3 dt, 1) / min(6 dt, 1)`; the position is corrected by the
/// difference, so the result is exactly what a half-rate `steer` would have
/// produced.
#[inline]
pub fn pipe_eel_after(e: &mut Enemy, before: Option<(Vec2, bool)>, dt: f32) {
    let Some((old, stunned)) = before else {
        return;
    };
    if stunned || dt <= 0.0 {
        return;
    }
    let full = (STEER_RATE * dt).min(1.0);
    if full <= 0.0 {
        return;
    }
    let k = (PIPE_EEL_STEER_RATE * dt).min(1.0) / full;
    let new = e.vel;
    let eased = old + (new - old) * k;
    e.pos += (eased - new) * dt;
    e.vel = eased;
}

// ---- Sump Leech -----------------------------------------------------------

/// Centre distance at which a leech commits to a tongue.
pub const LEECH_TRIGGER_RANGE: f32 = 24.0;
/// The warning: the sucker opens and the tongue's outline is on the floor.
pub const LEECH_WARN_SECS: f32 = 0.7;
/// The tongue is out and live.
pub const LEECH_HOLD_SECS: f32 = 0.4;
/// The rest after it retracts (the leech walks again).
pub const LEECH_REST_SECS: f32 = 4.0;
/// The tongue's length from the sucker's lip, world px.
pub const LEECH_TONGUE_LEN: f32 = 24.0;
/// Its full width, world px.
pub const LEECH_TONGUE_WIDTH: f32 = 4.0;
/// The rat's speed under the tongue's slow (30 % off).
pub const LEECH_SLOW_MULT: f32 = 0.7;
/// How long one touch slows the rat.
pub const LEECH_SLOW_SECS: f32 = 1.0;

/// Walking; `cd` counts the rest down.
pub const LEECH_PHASE_IDLE: u8 = 0;
/// Planted, sucker opening, footprint shown. Cannot hurt yet.
pub const LEECH_PHASE_WARN: u8 = 1;
/// Planted, tongue out and live.
pub const LEECH_PHASE_HOLD: u8 = 2;

/// One Sump Leech's tongue state. Authoritative; see the module docs for the
/// `w4` mirror the art reads.
#[derive(Component, Clone, Copy, Debug)]
pub struct SumpLeech {
    pub phase: u8,
    /// Seconds left in the current warn/hold phase.
    pub t: f32,
    /// Seconds of rest left before the next tongue.
    pub cd: f32,
    /// The locked tongue direction (unit).
    pub dir: Vec2,
    /// Where the body planted itself for this extension.
    pub anchor: Vec2,
    /// The once-per-extension latch.
    pub spent: bool,
}

impl Default for SumpLeech {
    fn default() -> Self {
        SumpLeech {
            phase: LEECH_PHASE_IDLE,
            t: 0.0,
            cd: 0.0,
            dir: Vec2::new(1.0, 0.0),
            anchor: Vec2::ZERO,
            spent: false,
        }
    }
}

/// Seconds of tongue slow left on the rat. Inserted by the first touch, so a
/// floor without a leech never has it.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct TongueSlow(pub f32);

/// The tongue's centre line for a body at `pos` with `radius`, aimed along
/// `dir`: from the sucker's lip to the tip. **The one geometry**: the hit test
/// ([`tongue_touches`]) and the drawn footprint (`art_roster_sewer`) both
/// read it.
#[inline]
pub fn tongue_segment(pos: Vec2, radius: f32, dir: Vec2) -> (Vec2, Vec2) {
    let lip = pos + dir * radius;
    (lip, lip + dir * LEECH_TONGUE_LEN)
}

/// Does a circle at `at` with radius `r` touch the 24×4 tongue?
#[inline]
pub fn tongue_touches(pos: Vec2, radius: f32, dir: Vec2, at: Vec2, r: f32) -> bool {
    let (a, b) = tongue_segment(pos, radius, dir);
    let ab = b - a;
    let len_sq = ab.len_sq();
    let t = if len_sq > 1e-6 {
        ((at - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let nearest = a + ab * t;
    let reach = LEECH_TONGUE_WIDTH * 0.5 + r;
    nearest.dist_sq(at) <= reach * reach
}

/// What an `&Enemy` reader sees of a leech's tongue: `(lip, tip, phase,
/// progress)` while it is warning or out, `None` otherwise. `progress` runs
/// 0 → 1 over the current phase. Read from the `w4` mirror, so the art and the
/// bot need no query.
pub fn leech_tongue(e: &Enemy) -> Option<(Vec2, Vec2, u8, f32)> {
    if e.kind != EnemyKind::SumpLeech || !e.alive() {
        return None;
    }
    let total = match e.w4.phase {
        LEECH_PHASE_WARN => LEECH_WARN_SECS,
        LEECH_PHASE_HOLD => LEECH_HOLD_SECS,
        _ => return None,
    };
    let (a, b) = tongue_segment(e.pos, e.radius, e.w4.dir);
    let progress = (1.0 - e.w4.t / total).clamp(0.0, 1.0);
    Some((a, b, e.w4.phase, progress))
}

/// Copy the component's state where `&Enemy` readers can see it.
#[inline]
fn mirror(e: &mut Enemy, s: &SumpLeech) {
    let was = e.w4.phase;
    e.w4.phase = s.phase;
    e.w4.t = s.t;
    e.w4.dir = s.dir;
    e.w4.cd = s.cd;
    if s.phase == LEECH_PHASE_IDLE || was != s.phase {
        e.w4.warn_age = 0.0;
    }
}

/// One leech, one tick. Returns true when its tongue touched the rat this
/// tick (the caller starts the slow).
#[allow(clippy::too_many_arguments)]
fn leech_tick(
    e: &mut Enemy,
    s: &mut SumpLeech,
    dt: f32,
    // Where the tongue is aimed: the rat, or the Wind-up Mouse's key.
    aim: Vec2,
    rat: Vec2,
    rat_alive: bool,
    hurt_radius: f32,
    level: &Level,
    sfx: &mut SfxQueue,
) -> bool {
    // A stunned leech drops whatever it was doing and rests.
    if e.stun > 0.0 && s.phase != LEECH_PHASE_IDLE {
        s.phase = LEECH_PHASE_IDLE;
        s.cd = LEECH_REST_SECS;
        s.spent = false;
        return false;
    }
    match s.phase {
        LEECH_PHASE_WARN => {
            e.pos = s.anchor;
            e.vel = Vec2::ZERO;
            e.facing = s.dir;
            e.w4.warn_age += dt;
            s.t -= dt;
            if s.t <= 0.0 {
                s.phase = LEECH_PHASE_HOLD;
                s.t = LEECH_HOLD_SECS;
                s.spent = false;
            }
            false
        }
        LEECH_PHASE_HOLD => {
            e.pos = s.anchor;
            e.vel = Vec2::ZERO;
            e.facing = s.dir;
            let mut touched = false;
            if !s.spent
                && rat_alive
                && tongue_touches(e.pos, e.radius, s.dir, rat, hurt_radius)
                && crate::game::ecs::systems::projectiles::los_clear(
                    level,
                    e.pos,
                    rat,
                    LEECH_TONGUE_WIDTH * 0.5,
                )
            {
                s.spent = true;
                touched = true;
            }
            s.t -= dt;
            if s.t <= 0.0 {
                s.phase = LEECH_PHASE_IDLE;
                s.cd = LEECH_REST_SECS;
                s.t = 0.0;
            }
            touched
        }
        _ => {
            s.cd = (s.cd - dt).max(0.0);
            if s.cd > 0.0 || e.stun > 0.0 || !rat_alive {
                return false;
            }
            let to = aim - e.pos;
            if to.len_sq() > LEECH_TRIGGER_RANGE * LEECH_TRIGGER_RANGE || to.len_sq() < 1e-6 {
                return false;
            }
            // A tongue is never aimed through a wall.
            if !crate::game::ecs::systems::projectiles::los_clear(
                level,
                e.pos,
                aim,
                LEECH_TONGUE_WIDTH * 0.5,
            ) {
                return false;
            }
            s.phase = LEECH_PHASE_WARN;
            s.t = LEECH_WARN_SECS;
            s.dir = to.normalized();
            s.anchor = e.pos;
            s.spent = false;
            e.vel = Vec2::ZERO;
            e.facing = s.dir;
            sfx.0.push(Sfx::Warn);
            false
        }
    }
}

/// The point a leech's tongue is aimed at: the Wind-up Mouse's key while the
/// key is luring the body, the rat otherwise (`enemy_wave4::lure_target`'s
/// rule, verbatim).
#[inline]
fn lure_target(e: &Enemy, rat: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    use crate::game::ecs::systems::powerups;
    let Some(key) = decoy else {
        return rat;
    };
    if !powerups::decoy_lures_body(e)
        || e.pos.dist_sq(key)
            > powerups::WINDUP_MOUSE_RADIUS * powerups::WINDUP_MOUSE_RADIUS
        || !crate::game::ecs::systems::projectiles::los_clear(level, e.pos, key, e.radius)
    {
        return rat;
    }
    key
}

/// Is the rat exempt from a ground slow this tick? The same rule the ink pad
/// uses: a dash in progress and Clean Socks both ignore it.
#[inline]
fn slow_exempt(player: &Player) -> bool {
    player.dash_active > 0.0 || player.buffs.socks > 0.0 || !player.alive()
}

/// The Sewer's behaviour tick: every Sump Leech's tongue, and the clock of the
/// slow it lays on the rat. Chain right after `enemy_wave4::wave4_system`.
#[allow(clippy::too_many_arguments)]
pub fn floor_sewer_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut sfx: ResMut<SfxQueue>,
    mut slow: Option<ResMut<TongueSlow>>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut SumpLeech>)>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;

    // ---- the slow already on the rat ---------------------------------------
    if let Some(slow) = slow.as_mut() {
        if slow.0 > 0.0 {
            slow.0 = (slow.0 - dt).max(0.0);
            if !slow_exempt(&player) {
                let cap = player.loadout.stats.move_speed * LEECH_SLOW_MULT;
                player.vel = player.vel.clamp_len(cap);
            }
        }
    }

    // Cheap gate: no leech, no sort.
    if !enemies.iter().any(|(_, _, e, _)| e.kind == EnemyKind::SumpLeech) {
        return;
    }

    order.clear();
    order.extend(
        enemies
            .iter()
            .filter(|(_, _, e, _)| e.kind == EnemyKind::SumpLeech)
            .map(|(entity, id, _, _)| (id.0, entity)),
    );
    order.sort_unstable_by_key(|&(id, _)| id);

    let rat = player.pos;
    let rat_alive = player.alive();
    // The live Wind-up Mouse key (highest `SpawnId` wins), the same rule the
    // steering lure, wave 3 and wave 4 use.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);
    let mut touched = false;
    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e, state)) = enemies.get_mut(entity) else {
            continue;
        };
        let Some(mut state) = state else {
            // First tick alive: give it its state; it starts next tick.
            commands.entity(entity).insert(SumpLeech::default());
            continue;
        };
        if !e.alive() {
            if state.phase != LEECH_PHASE_IDLE {
                state.phase = LEECH_PHASE_IDLE;
                mirror(&mut e, &state);
            }
            continue;
        }
        // The tongue is aimed at whatever the body is chasing: the key while
        // the key lures it. Only the rat can be touched (and slowed).
        let aim = lure_target(&e, rat, decoy, &level);
        let hurt_radius = PLAYER_RADIUS + player.buffs.giant_hit_padding();
        if leech_tick(&mut e, &mut state, dt, aim, rat, rat_alive, hurt_radius, &level, &mut sfx) {
            touched = true;
        }
        mirror(&mut e, &state);
    }

    if touched && !slow_exempt(&player) {
        match slow.as_mut() {
            Some(slow) => slow.0 = slow.0.max(LEECH_SLOW_SECS),
            None => commands.insert_resource(TongueSlow(LEECH_SLOW_SECS)),
        }
    }
}

/// The exact half of the tongue's slow: shorten this tick's throttle and clip
/// the velocity, exactly as `hazards::hazards_system` does for an ink pad.
/// Chain right after `hazards::hazards_system` (before `player`).
pub fn floor_sewer_slow_system(
    mut input: ResMut<TickInput>,
    mut player: ResMut<Player>,
    slow: Option<Res<TongueSlow>>,
) {
    let Some(slow) = slow else {
        return;
    };
    if slow.0 <= 0.0 || slow_exempt(&player) || (input.dash && player.dash_cd <= 0.0) {
        return;
    }
    input.move_dir = input.move_dir * LEECH_SLOW_MULT;
    let cap = player.loadout.stats.move_speed * LEECH_SLOW_MULT;
    player.vel = player.vel.clamp_len(cap);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;
    use rand::{RngExt, SeedableRng};

    const DT: f32 = 1.0 / 60.0;

    fn tick_opening(w: &mut World, elapsed: f32) {
        w.elapsed = elapsed;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(sewer_opening_system).unwrap();
        w.end_tick();
    }

    #[test]
    fn sewer_opening_pillbugs_spawn_curled_at_22_seconds_without_rng() {
        let mut w = World::new(Theme::Sewer, 7);
        w.viewport = Vec2::new(40.0, 40.0);
        let cells = &w.level.objects.sewer_opening.as_ref().unwrap().pillbug_cells;
        let expected = [cells[0], cells[1]];
        let next_rng = World::new(Theme::Sewer, 7).rng.random::<u64>();
        tick_opening(&mut w, 22.0 - DT);
        assert_eq!(w.enemy_count(), 0);
        tick_opening(&mut w, 22.0);
        let enemies = w.enemies();
        assert_eq!(enemies.len(), 2);
        for (e, pos) in enemies.iter().zip(expected) {
            assert_eq!(e.kind, EnemyKind::Pillbug);
            assert_eq!(e.pos, pos);
            assert!(off_screen(e.pos, w.player.pos, w.viewport));
            assert!(e.w3.opening_curl);
            assert_eq!(e.w3.curl_t, PILLBUG_CURL_SECS);
        }
        tick_opening(&mut w, 23.0);
        assert_eq!(w.enemy_count(), 2);
        assert_eq!(w.rng.random::<u64>(), next_rng);
    }

    #[test]
    fn sewer_opening_waits_until_both_pillbugs_are_off_screen() {
        let mut w = World::new(Theme::Sewer, 7);
        w.viewport = w.level.arena;
        tick_opening(&mut w, 22.0);
        assert_eq!(w.enemy_count(), 0);
        w.viewport = Vec2::new(40.0, 40.0);
        tick_opening(&mut w, 22.0 + DT);
        assert_eq!(w.enemy_count(), 2);
    }

    #[test]
    fn sewer_opening_places_one_bottle_cap_at_90_seconds() {
        let mut w = World::new(Theme::Sewer, 7);
        w.difficulty_override = Some(0); // Sewer practice uses this override.
        let pos = w.level.objects.sewer_opening.as_ref().unwrap().bottle_cap;
        tick_opening(&mut w, 90.0 - DT);
        assert!(w.pickups().is_empty());
        tick_opening(&mut w, 90.0);
        tick_opening(&mut w, 91.0);
        let caps = w.pickups();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].kind, PickupKind::BottleCap);
        assert_eq!(caps[0].pos, pos);

        w.player.pos = pos;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(super::super::loot::pickups_system).unwrap();
        w.end_tick();
        assert_eq!(w.last_pickups(), &[PickupKind::BottleCap]);
        assert!(w.player.buffs.shield > 0.0);
        tick_opening(&mut w, 92.0);
        assert!(w.pickups().is_empty());
    }

    #[test]
    fn sewer_opening_is_absent_on_later_levels_and_other_themes() {
        let mut later = World::new_level_with_theme(
            1,
            Theme::Sewer,
            7,
            crate::game::loadout::Loadout::starting(),
            Default::default(),
        );
        let mut other = World::new(Theme::Kitchen, 7);
        for w in [&mut later, &mut other] {
            assert!(w.level.objects.sewer_opening.is_none());
            tick_opening(w, 90.0);
            assert_eq!(w.enemy_count(), 0);
            assert!(w.pickups().is_empty());
        }
    }

    fn open_floor(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_sewer_system).unwrap();
        w.end_tick();
    }

    fn secs(s: f32) -> usize {
        (s / DT).ceil() as usize
    }

    fn leech(w: &World, entity: Entity) -> SumpLeech {
        *w.ecs.get::<SumpLeech>(entity).expect("the leech has its state")
    }

    fn world_with_leech(offset: Vec2) -> (World, Entity) {
        let mut w = World::new(Theme::Sewer, 7);
        open_floor(&mut w);
        w.clear_enemies();
        w.player.pos = Vec2::new(400.0, 400.0);
        let at = w.player.pos + offset;
        let entity = w.spawn_enemy(EnemyKind::SumpLeech, at, 1.0);
        // The first tick only attaches the component.
        tick(&mut w);
        (w, entity)
    }

    #[test]
    fn pipe_eel_turns_at_half_rate_and_only_the_eel() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut eel = Enemy::spawn(EnemyKind::PipeEel, Vec2::new(100.0, 100.0), 1.0, &mut rng);
        let mut cat = eel.clone();
        cat.kind = EnemyKind::Cat;
        eel.vel = Vec2::new(30.0, 0.0);
        cat.vel = eel.vel;
        let target = Vec2::new(100.0, 400.0);

        let before = pipe_eel_before(&eel);
        assert!(before.is_some());
        assert!(pipe_eel_before(&cat).is_none());

        let old = eel.vel;
        let start = eel.pos;
        eel.steer(target, Vec2::ZERO, DT, 1.0);
        let full = eel.vel;
        pipe_eel_after(&mut eel, before, DT);

        // Exactly half of the velocity change the full-rate steer made.
        let want = old + (full - old) * 0.5;
        assert!((eel.vel.x - want.x).abs() < 1e-4 && (eel.vel.y - want.y).abs() < 1e-4);
        // ...and the position moved by the eased velocity, not the full one.
        let moved = eel.pos - start;
        assert!((moved.x - want.x * DT).abs() < 1e-4 && (moved.y - want.y * DT).abs() < 1e-4);

        // A `None` (any other kind) leaves the body alone.
        let cat_before = cat.clone();
        pipe_eel_after(&mut cat, None, DT);
        assert_eq!(cat.vel.x, cat_before.vel.x);
        assert_eq!(cat.pos.x, cat_before.pos.x);
    }

    #[test]
    fn a_stunned_eel_is_not_corrected() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut eel = Enemy::spawn(EnemyKind::PipeEel, Vec2::new(100.0, 100.0), 1.0, &mut rng);
        eel.vel = Vec2::new(30.0, 0.0);
        eel.stun = 0.5;
        let before = pipe_eel_before(&eel);
        eel.steer(Vec2::new(100.0, 400.0), Vec2::ZERO, DT, 1.0);
        let (vel, pos) = (eel.vel, eel.pos);
        pipe_eel_after(&mut eel, before, DT);
        assert_eq!((eel.vel.x, eel.vel.y), (vel.x, vel.y));
        assert_eq!((eel.pos.x, eel.pos.y), (pos.x, pos.y));
    }

    #[test]
    fn the_tongue_geometry_is_24_by_4_from_the_lip() {
        let pos = Vec2::new(50.0, 50.0);
        let dir = Vec2::new(1.0, 0.0);
        let (a, b) = tongue_segment(pos, 5.0, dir);
        assert_eq!((a.x, b.x), (55.0, 79.0));
        // On the line, at the tip, and just off the side.
        assert!(tongue_touches(pos, 5.0, dir, Vec2::new(70.0, 50.0), 0.0));
        assert!(tongue_touches(pos, 5.0, dir, Vec2::new(79.0, 51.9), 0.0));
        assert!(!tongue_touches(pos, 5.0, dir, Vec2::new(70.0, 52.5), 0.0));
        // Behind the head is safe: "cross behind its head".
        assert!(!tongue_touches(pos, 5.0, dir, Vec2::new(40.0, 50.0), 2.0));
    }

    #[test]
    fn a_leech_out_of_range_keeps_walking() {
        let (mut w, e) = world_with_leech(Vec2::new(60.0, 0.0));
        for _ in 0..30 {
            tick(&mut w);
        }
        assert_eq!(leech(&w, e).phase, LEECH_PHASE_IDLE);
    }

    #[test]
    fn warn_then_hold_then_rest_and_one_slow_per_extension() {
        let (mut w, e) = world_with_leech(Vec2::new(20.0, 0.0));
        tick(&mut w);
        let s = leech(&w, e);
        assert_eq!(s.phase, LEECH_PHASE_WARN, "in range: it commits");
        assert!(s.dir.x < -0.99, "aimed at the rat");

        // The whole warning is harmless and the footprint is readable.
        for _ in 0..secs(LEECH_WARN_SECS) - 2 {
            tick(&mut w);
            assert_eq!(leech(&w, e).phase, LEECH_PHASE_WARN);
            assert!(w.ecs.get_resource::<TongueSlow>().is_none());
        }
        let body = w.ecs.get::<Enemy>(e).unwrap().clone();
        let seen = leech_tongue(&body).expect("the tell is visible to &Enemy readers");
        assert_eq!(seen.2, LEECH_PHASE_WARN);

        // It strikes: the rat is standing in the tongue.
        for _ in 0..4 {
            tick(&mut w);
        }
        assert_eq!(leech(&w, e).phase, LEECH_PHASE_HOLD);
        assert!(leech(&w, e).spent, "one touch latched");
        tick(&mut w);
        let slow = w.ecs.get_resource::<TongueSlow>().expect("the slow is on").0;
        assert!(slow > 0.9 && slow <= LEECH_SLOW_SECS);

        // The hold ends, the leech rests four seconds and cannot re-extend.
        for _ in 0..secs(LEECH_HOLD_SECS) {
            tick(&mut w);
        }
        let s = leech(&w, e);
        assert_eq!(s.phase, LEECH_PHASE_IDLE);
        assert!(s.cd > LEECH_REST_SECS - 0.2);
        for _ in 0..secs(1.5) {
            tick(&mut w);
            assert_eq!(leech(&w, e).phase, LEECH_PHASE_IDLE, "resting");
        }
        // The slow ran out after its second and was not refreshed.
        assert_eq!(w.ecs.get_resource::<TongueSlow>().unwrap().0, 0.0);
    }

    #[test]
    fn stepping_off_the_line_during_the_warning_avoids_the_slow() {
        let (mut w, e) = world_with_leech(Vec2::new(20.0, 0.0));
        tick(&mut w);
        assert_eq!(leech(&w, e).phase, LEECH_PHASE_WARN);
        // Cross behind its head.
        w.player.pos += Vec2::new(40.0, 0.0);
        for _ in 0..secs(LEECH_WARN_SECS + LEECH_HOLD_SECS) + 2 {
            tick(&mut w);
        }
        assert!(!leech(&w, e).spent);
        assert!(w.ecs.get_resource::<TongueSlow>().is_none());
    }

    #[test]
    fn a_planted_leech_does_not_move() {
        let (mut w, e) = world_with_leech(Vec2::new(20.0, 0.0));
        tick(&mut w);
        let anchor = leech(&w, e).anchor;
        w.ecs.get_mut::<Enemy>(e).unwrap().pos = anchor + Vec2::new(0.4, 0.0);
        tick(&mut w);
        let pos = w.ecs.get::<Enemy>(e).unwrap().pos;
        assert_eq!((pos.x, pos.y), (anchor.x, anchor.y));
    }

    #[test]
    fn the_slow_clips_the_rat_and_a_dash_ignores_it() {
        let (mut w, _) = world_with_leech(Vec2::new(200.0, 0.0));
        w.ecs.insert_resource(TongueSlow(1.0));
        let speed = w.player.loadout.stats.move_speed;
        w.player.vel = Vec2::new(speed, 0.0);
        tick(&mut w);
        assert!((w.player.vel.len() - speed * LEECH_SLOW_MULT).abs() < 1e-3);

        w.ecs.insert_resource(TongueSlow(1.0));
        w.player.vel = Vec2::new(speed, 0.0);
        w.player.dash_active = 0.1;
        tick(&mut w);
        assert!((w.player.vel.len() - speed).abs() < 1e-3);
    }

    #[test]
    fn a_lured_leech_spends_its_tongue_on_the_key() {
        // The rat is in range to the left; the key is in range to the right.
        let (mut w, e) = world_with_leech(Vec2::new(20.0, 0.0));
        let at = w.ecs.get::<Enemy>(e).unwrap().pos;
        crate::game::ecs::systems::powerups::spawn_decoy(&mut w, at + Vec2::new(18.0, 0.0));
        tick(&mut w);
        let s = leech(&w, e);
        assert_eq!(s.phase, LEECH_PHASE_WARN);
        assert!(s.dir.x > 0.99, "aimed at the key, away from the rat");
        for _ in 0..secs(LEECH_WARN_SECS + LEECH_HOLD_SECS) + 2 {
            tick(&mut w);
        }
        assert!(w.ecs.get_resource::<TongueSlow>().is_none(), "the rat was behind its head");
    }

    #[test]
    fn no_leech_means_no_state_and_no_resource() {
        let mut w = World::new(Theme::Sewer, 9);
        w.clear_enemies();
        let cat = w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(10.0, 0.0), 1.0);
        for _ in 0..10 {
            tick(&mut w);
        }
        assert!(w.ecs.get::<SumpLeech>(cat).is_none());
        assert!(w.ecs.get_resource::<TongueSlow>().is_none());
    }
}
