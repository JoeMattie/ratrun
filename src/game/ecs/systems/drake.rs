//! The guest: **the Drake**, a big duck that pecks (owner: "new enemy: a big
//! duck that pecks at you").
//!
//! It is not a roster body. The Sewer's eight slots (`game/roster.rs`) are
//! untouched and it spends none of them, none of the live budget and none of
//! the per-slot caps: the director's guest rule ([`spawn_guest`]) keeps at most
//! one alive, two from 8:00, on the Sewer only, from 2:30 on the curve clock,
//! with a 45 s gap after the last one spawned or died, never during the boss
//! fight and never beside a live miniboss. Its placement is the ordinary
//! out-of-sight ladder (`spawn::spawn_point_reachable`), so it appears exactly
//! where a straggler would and draws exactly the rng a straggler's placement
//! draws.
//!
//! ## The state machine
//!
//! | state | how long | what it does |
//! |---|---|---|
//! | [`DrakeState::Waddle`] | until engaged | the ordinary large-class chase, 30 px/s on dry floor and 40 in a puddle ([`water_gait`]), on the same flow field every body uses |
//! | [`DrakeState::Tell`] | [`TELL_SECS`] 0.45 s | planted, neck back; the aim **tracks the rat for the first [`TELL_LOCK_SECS`] (0.25 s) then locks**, and the peck lane is outlined on the hostile layer |
//! | [`DrakeState::Peck`] | 3 × [`PECK_SECS`] (0.12 s out + 0.10 s back) | planted; each peck's bill goes out to [`art_duck::PECK_REACH`] and back, one hit test per peck at full extension; between pecks the aim turns toward the rat by at most [`REAIM_MAX_RAD`] |
//! | [`DrakeState::Recover`] | [`RECOVER_SECS`] 1.3 s | planted and exposed: x[`RECOVER_DAMAGE_MULT`] incoming damage |
//!
//! Then Waddle again. A hit test is "the rat's centre within
//! [`PECK_HIT_RADIUS`] of the bill tip at full extension", and the tip comes
//! from [`art_duck::bill_tip`] on the same [`drake_view`] the renderer draws,
//! so the picture and the hit test cannot disagree. The strike is resolved on
//! the tick the out-stroke ends, with the state clock snapped to the peak
//! ([`PECK_STRIKE_PROGRESS`]) for that one question.
//!
//! ## The one geometry
//!
//! [`peck_lane`] is the lane from the body's centre out to the bill's reach
//! along the current aim: the hostile-layer outline (`World::draw_drake_lane`),
//! the bot's read and the hit test's line are all that one function. The
//! **`art_duck` module is the spec**: `PECK_REACH` and `bill_tip` are its, and
//! [`PECK_STRIKE_PROGRESS`] is the one number the two files share.
//!
//! ## Layering, like the Sump Leech's tongue
//!
//! `EnemyKind::Drake.archetype()` is `EnemyKind::Brute` so the nav size class,
//! the glue weight, `W4::holds` and `Enemy::steer` agree with a body of its
//! mass; every rule of its own lives here. The component ([`Drake`]) is
//! authoritative and the three numbers an `&Enemy` reader needs (phase, phase
//! clock, aim) are mirrored into the body's otherwise-unused `w4` scratch —
//! the same trick `floor_sewer` uses for the leech — so the bot, the batch's
//! damage attribution and `Enemy::damage_mult_from` (the exposed recovery)
//! work from a bare `&Enemy`.
//!
//! ## Determinism
//!
//! Bodies are walked in `SpawnId` order and body behaviour draws **no** rng:
//! the aim, the re-aim, the strike and the water speed are arithmetic. The one
//! draw in this file is the player-hurt particle burst, which every other floor
//! pass also takes (`vfx::emit_player_hurt`). The guest spawner draws exactly
//! what a placement draws: one ring angle from `spawn_point_reachable`, then
//! `Enemy::spawn`'s three fields. Nothing here can fire before 2:30 on the
//! curve clock, and the determinism goldens run 60 s, so **the goldens cannot
//! see this file** (see `.review-report-duck-code.md`).

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::art_duck::{self, DuckPose, DuckView};
use crate::game::ecs::components::{Drake, DrakeState, SpawnId};
use crate::game::ecs::resources::{
    DrakeSpawner, NextEnemyId, NextSpawnId, RngRes, SfxQueue, TickInput,
};
use crate::game::ecs::systems::powerups;
use crate::game::ecs::systems::projectiles;
use crate::game::ecs::systems::spawn::{spawn_point_reachable, SpawnScratch};
use crate::game::enemy::{Enemy, EnemyKind, EnemyMods};
use crate::game::level::{Level, Theme};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

// ---- The timings and reach (the shared contract) ---------------------------

/// Centre distance at which a Drake commits to a peck volley.
pub const ENGAGE_RANGE: f32 = 30.0;
/// The telegraphed, planted warning.
pub const TELL_SECS: f32 = 0.45;
/// How far into the tell the aim tracks the rat: locked at tell start + 0.25 s.
pub const TELL_LOCK_SECS: f32 = 0.25;
/// One peck's out-stroke (the bill going out to full extension).
pub const PECK_OUT_SECS: f32 = 0.12;
/// One peck's return stroke.
pub const PECK_BACK_SECS: f32 = 0.10;
/// One whole peck: out and back.
pub const PECK_SECS: f32 = PECK_OUT_SECS + PECK_BACK_SECS;
/// Pecks in one volley.
pub const PECK_COUNT: u8 = 3;
/// Damage one peck that lands deals, before the level's horde damage scaling
/// (`Enemy::damage / EnemyKind::damage`, the multiplier the director baked in
/// at spawn — the same scaling every other enemy attack carries).
pub const PECK_DAMAGE: f32 = 9.0;
/// How close the rat's centre must be to the bill tip at full extension.
pub const PECK_HIT_RADIUS: f32 = 5.0;
/// The drawn/hit lane's full width, world px (the `on_screen` margin and the
/// line-of-sight half-width both read it).
pub const PECK_LANE_WIDTH: f32 = 6.0;
/// The most the aim may turn toward the rat between two pecks (35°).
///
/// `35.0f32.to_radians()` is not const, so the radian value is written out:
/// 35 × π / 180.
pub const REAIM_MAX_RAD: f32 = 0.610_865_3;
/// The exposed recovery after a volley.
pub const RECOVER_SECS: f32 = 1.3;
/// What the recovery multiplies every incoming hit by (the punish window).
pub const RECOVER_DAMAGE_MULT: f32 = 1.5;
/// The Drake's paddling speed in a puddle (`Level::wet_at`), against
/// `EnemyKind::Drake.speed()`'s 30 on the dry floor.
pub const WET_SPEED: f32 = 40.0;
/// `Enemy::apply_damage`'s hit-flash seconds, which the drawn `hurt` is a
/// fraction of.
pub const HIT_FLASH_SECS: f32 = 0.12;

/// The progress through one peck at which the bill is at full extension: the
/// end of the out-stroke. **The one number `art_duck::bill_tip` and this file
/// share** besides `PECK_REACH`.
pub const PECK_STRIKE_PROGRESS: f32 = PECK_OUT_SECS / PECK_SECS;

// ---- The guest spawner's clock --------------------------------------------

/// The curve clock a Drake may first spawn on (2:30 on the design clock).
pub const FROM_CLOCK: f32 = 150.0;
/// …and the clock from which two may be alive at once (8:00).
pub const SECOND_FROM_CLOCK: f32 = 480.0;
/// Seconds that must pass after the last Drake spawned or left the floor
/// before another may arrive.
pub const GAP_SECS: f32 = 45.0;

// ---- The scratch mirror's phase numbers -----------------------------------

/// `w4.phase` for each state. The discriminant **is** the mirrored number.
pub const PHASE_WADDLE: u8 = DrakeState::Waddle as u8;
pub const PHASE_TELL: u8 = DrakeState::Tell as u8;
pub const PHASE_PECK: u8 = DrakeState::Peck as u8;
pub const PHASE_RECOVER: u8 = DrakeState::Recover as u8;

// ---- The one geometry -----------------------------------------------------

/// The peck lane: from the body's centre out to the bill's reach along `aim`.
///
/// The hostile layer's outline, the bot's hazard read and the hit test's line
/// are all this. (`art_duck::bill_tip` is the authority on where the bill
/// actually is at a given pose; the lane is where it can reach.)
#[inline]
pub fn peck_lane(pos: Vec2, aim: Vec2) -> (Vec2, Vec2) {
    (pos, pos + aim * art_duck::PECK_REACH)
}

/// The steering seam: the multiplier `enemy_steering_system` folds into the
/// speed it hands `Enemy::steer`.
///
/// A Drake paddles at [`WET_SPEED`] in a puddle against its 30 on the dry
/// floor; exactly 1.0 for every other body and every other kind.
#[inline]
pub fn water_gait(e: &Enemy, level: &Level) -> f32 {
    if e.kind != EnemyKind::Drake {
        return 1.0;
    }
    if level.wet_at(e.pos) {
        WET_SPEED / EnemyKind::Drake.speed()
    } else {
        1.0
    }
}

/// The drawn view of one Drake, built from the body and its state only.
///
/// **One function**: `World::draw` draws through it and the peck's hit test
/// asks it (then `art_duck::bill_tip`) where the bill is, so the picture and
/// the damage cannot disagree.
pub fn drake_view(e: &Enemy, d: &Drake) -> DuckView {
    let pose = match d.state {
        DrakeState::Waddle => DuckPose::Waddle,
        DrakeState::Tell => DuckPose::Tell(progress(TELL_SECS - d.t, TELL_SECS)),
        DrakeState::Peck => DuckPose::Peck(progress(PECK_SECS - d.t, PECK_SECS)),
        DrakeState::Recover => DuckPose::Recover(progress(RECOVER_SECS - d.t, RECOVER_SECS)),
    };
    let facing = if d.aim.len_sq() > 1e-6 {
        d.aim.normalized()
    } else if e.facing.len_sq() > 1e-6 {
        e.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    DuckView {
        pos: e.pos,
        facing,
        pose,
        walk: d.walk,
        on_water: d.on_water,
        hurt: (e.flash / HIT_FLASH_SECS).clamp(0.0, 1.0),
        // `Enemy::id` is the run's monotonic body id (`NextEnemyId`), so the
        // art's per-body variation is stable and costs no rng.
        seed: e.id,
    }
}

/// `done / total` as 0..=1, guarding a zero-length state.
#[inline]
fn progress(done: f32, total: f32) -> f32 {
    if total <= 0.0 {
        return 0.0;
    }
    (done / total).clamp(0.0, 1.0)
}

/// The view `&Enemy` readers get when the component is not in hand: built from
/// the `w4` mirror [`drake_system`] keeps. `None` for every other kind.
///
/// Used by the autoplay batch's damage attribution and by the tests that check
/// the mirror; the renderer and the hit test use [`drake_view`] directly.
pub fn mirrored_view(e: &Enemy) -> Option<DuckView> {
    if e.kind != EnemyKind::Drake {
        return None;
    }
    let state = match e.w4.phase {
        PHASE_TELL => DrakeState::Tell,
        PHASE_PECK => DrakeState::Peck,
        PHASE_RECOVER => DrakeState::Recover,
        _ => DrakeState::Waddle,
    };
    let d = Drake {
        state,
        t: e.w4.t,
        pecks_left: 0,
        aim: e.w4.dir,
        walk: e.w4.cd,
        anchor: e.pos,
        on_water: false,
    };
    Some(drake_view(e, &d))
}

/// Copy the component's state where `&Enemy` readers can see it.
#[inline]
fn mirror(e: &mut Enemy, d: &Drake) {
    e.w4.phase = d.state as u8;
    e.w4.t = d.t;
    e.w4.dir = d.aim;
    e.w4.cd = d.walk;
    e.w4.warn_age = if d.state == DrakeState::Tell {
        TELL_SECS - d.t
    } else {
        0.0
    };
}

/// Turn `from` toward `to` (both directions) by at most `max` radians.
fn rotate_toward(from: Vec2, to: Vec2, max: f32) -> Vec2 {
    if from.len_sq() <= 1e-6 {
        return to;
    }
    if to.len_sq() <= 1e-6 {
        return from;
    }
    let (from, to) = (from.normalized(), to.normalized());
    let dot = from.dot(to).clamp(-1.0, 1.0);
    let angle = dot.acos();
    if angle <= max {
        return to;
    }
    // Rotating by exactly `max` along the shorter arc: rotate `from` by
    // `sign * max`, with the sign from the cross product (`perp().dot()`).
    let sign = if from.perp().dot(to) < 0.0 { -1.0 } else { 1.0 };
    from.rotate(sign * max)
}

// ---- The spawner ----------------------------------------------------------

/// Everything the guest rule reads from the director's own tick.
pub struct GuestTick<'a> {
    /// Run clock (`RunCounters::elapsed`).
    pub elapsed: f32,
    /// The design/curve clock (`director::curve_clock`): 2:30 and 8:00 are its
    /// times.
    pub curve: f32,
    pub theme: Theme,
    pub player: &'a Player,
    /// The simulation viewport (the spawn ladder's "out of sight").
    pub viewport: Vec2,
    pub level: &'a Level,
    /// The director has raised `BossDue` or the boss is on the floor.
    pub boss_phase: bool,
    /// A named miniboss is alive (the schedule's `active` slot count).
    pub miniboss_alive: bool,
    /// Live Drakes, counted from the query this tick.
    pub live_drakes: usize,
    /// The horde's hp multiplier for this tick (`director::horde_hp_mult_for`),
    /// the same one the pulse uses.
    pub hp_mult: f32,
    pub mods: EnemyMods,
    pub spawner: &'a mut DrakeSpawner,
    pub scratch: &'a mut SpawnScratch,
    pub rng: &'a mut rand::rngs::StdRng,
    pub next_id: &'a mut NextSpawnId,
    pub next_enemy_id: &'a mut NextEnemyId,
}

/// The gate the guest rule applies before it places a body: the floor, the
/// 2:30 clock, the one-or-two cap, the boss phase, the miniboss and the 45 s
/// gap.
///
/// Split out of [`spawn_guest`] so a test can ask the rule a question without
/// a `Commands` in hand; [`spawn_guest`] is the only production caller, and it
/// asks this first (before any rng is drawn).
#[inline]
pub fn guest_due(
    theme: Theme,
    curve: f32,
    elapsed: f32,
    last_change: f32,
    live: usize,
    boss_phase: bool,
    miniboss_alive: bool,
) -> bool {
    theme == Theme::Sewer
        && curve >= FROM_CLOCK
        && live < if curve >= SECOND_FROM_CLOCK { 2 } else { 1 }
        && !boss_phase
        && !miniboss_alive
        && elapsed - last_change >= GAP_SECS
}

/// The director's guest rule: keep the 45 s clock, then spawn one Drake if the
/// floor, the clock, the boss state, the miniboss state and the cap allow it.
///
/// Returns `true` when a body was queued. Draws **one** ring angle plus
/// `Enemy::spawn`'s three fields (exactly what `spawn_point_reachable` +
/// `Enemy::spawn` always draw) and nothing else; on every refused path it
/// returns before touching the rng.
pub fn spawn_guest(g: &mut GuestTick<'_>, commands: &mut Commands<'_, '_>) -> bool {
    // The death clock first: a Drake that left the floor restarts the gap, and
    // that has to hold on a tick the floor is otherwise refusing spawns (a
    // miniboss arriving, the boss phase).
    let live = g.live_drakes;
    if live < g.spawner.live as usize {
        g.spawner.last = g.elapsed;
    }
    g.spawner.live = live.min(u8::MAX as usize) as u8;

    if !guest_due(
        g.theme,
        g.curve,
        g.elapsed,
        g.spawner.last,
        live,
        g.boss_phase,
        g.miniboss_alive,
    ) {
        return false;
    }
    let Some(pos) = spawn_point_reachable(g.rng, g.level, g.player, g.viewport, g.scratch) else {
        // No safe off-screen point: skip, exactly like a straggler.
        return false;
    };
    let mut enemy = Enemy::spawn_with_mods(EnemyKind::Drake, pos, g.hp_mult, g.mods, g.rng);
    enemy.id = g.next_enemy_id.take();
    let id = g.next_id.take();
    commands.spawn((enemy, id, Drake::default()));
    g.spawner.last = g.elapsed;
    g.spawner.live = g.spawner.live.saturating_add(1);
    true
}

// ---- The behaviour --------------------------------------------------------

/// Where a Drake is aimed: the Wind-up Mouse's key while the key is luring it
/// (`powerups::decoy_lures_body`, the key's radius, a clear line), the rat
/// otherwise — the rule every floor pass and both steering lures use, verbatim.
#[inline]
fn lure_target(e: &Enemy, rat: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    let Some(key) = decoy else {
        return rat;
    };
    if !powerups::decoy_lures_body(e) {
        return rat;
    }
    if e.pos.dist_sq(key) > powerups::WINDUP_MOUSE_RADIUS.powi(2) {
        return rat;
    }
    if !projectiles::los_clear(level, e.pos, key, e.radius) {
        return rat;
    }
    key
}

/// One Drake, one tick. Returns `true` when a peck landed on the rat (the
/// caller draws the hurt burst).
///
/// `target` is what it aims at (the rat, or the lure); `rat` is always the rat,
/// the only thing a peck can hurt.
#[allow(clippy::too_many_arguments)]
fn drake_tick(
    e: &mut Enemy,
    d: &mut Drake,
    dt: f32,
    target: Vec2,
    rat: Vec2,
    rat_alive: bool,
    level: &Level,
    player: &mut Player,
    sfx: &mut SfxQueue,
) -> bool {
    d.on_water = level.wet_at(e.pos);
    // A stunned Drake drops whatever it was doing and walks again (the Sump
    // Leech's rule): the Sonic Squeak ring buys the rat out of a volley.
    if e.stun > 0.0 && d.state != DrakeState::Waddle {
        d.state = DrakeState::Waddle;
        d.t = 0.0;
        d.pecks_left = 0;
        return false;
    }
    let mut landed = false;
    match d.state {
        DrakeState::Waddle => {
            d.walk += dt;
            if e.stun > 0.0 || !rat_alive {
                return false;
            }
            let to = target - e.pos;
            if to.len_sq() > ENGAGE_RANGE * ENGAGE_RANGE || to.len_sq() < 1e-6 {
                return false;
            }
            // A peck is never aimed through a wall (the leech's rule).
            if !projectiles::los_clear(level, e.pos, target, PECK_LANE_WIDTH * 0.5) {
                return false;
            }
            d.state = DrakeState::Tell;
            d.t = TELL_SECS;
            d.pecks_left = PECK_COUNT;
            d.aim = to.normalized();
            d.anchor = e.pos;
            e.vel = Vec2::ZERO;
            e.facing = d.aim;
            sfx.0.push(Sfx::Quack);
        }
        DrakeState::Tell => {
            // Planted: the steering pass moved the body this tick and this puts
            // it back on its mark (the leech's rule, and the reason a shove
            // cannot drag a committed bird off its lane).
            e.pos = d.anchor;
            e.vel = Vec2::ZERO;
            // Tracks for the first 0.25 s, then locks: a tell that locked on
            // its first frame is outrun by a rat that keeps moving.
            let age = TELL_SECS - d.t;
            if age < TELL_LOCK_SECS && rat_alive {
                let to = target - e.pos;
                if to.len_sq() > 1e-6 {
                    d.aim = to.normalized();
                }
            }
            e.facing = d.aim;
            d.t -= dt;
            if d.t <= 0.0 {
                d.state = DrakeState::Peck;
                d.t = PECK_SECS;
                d.pecks_left = PECK_COUNT;
            }
        }
        DrakeState::Peck => {
            e.pos = d.anchor;
            e.vel = Vec2::ZERO;
            e.facing = d.aim;
            let t_next = d.t - dt;
            // Full extension: the tick the out-stroke ends. One hit test per
            // peck, against the bill tip `bill_tip` puts there for the drawn
            // pose at that progress.
            if d.t > PECK_BACK_SECS && t_next <= PECK_BACK_SECS {
                let mut probe = *d;
                probe.t = PECK_BACK_SECS;
                let tip = art_duck::bill_tip(&drake_view(e, &probe));
                sfx.0.push(Sfx::Peck);
                if rat_alive && tip.dist(rat) <= PECK_HIT_RADIUS {
                    // The level's horde damage scaling, as every other enemy
                    // attack carries it (`EnemyMods::damage`, baked in at
                    // spawn); then the ordinary player damage path (i-frames,
                    // armour, Growth Serum, splices).
                    let base = e.kind.damage();
                    let scale = if base > 0.0 { e.damage / base } else { 1.0 };
                    if player.take_damage(PECK_DAMAGE * scale) {
                        landed = true;
                        sfx.0.push(Sfx::Hurt);
                    }
                }
            }
            d.t = t_next;
            if d.t <= 0.0 {
                d.pecks_left = d.pecks_left.saturating_sub(1);
                if d.pecks_left == 0 {
                    d.state = DrakeState::Recover;
                    d.t = RECOVER_SECS;
                } else {
                    // Between pecks: the aim turns toward the rat by at most
                    // 35°. The first peck always flies the locked lane.
                    if rat_alive {
                        d.aim = rotate_toward(d.aim, target - e.pos, REAIM_MAX_RAD);
                    }
                    d.t = PECK_SECS;
                }
            }
        }
        DrakeState::Recover => {
            e.pos = d.anchor;
            e.vel = Vec2::ZERO;
            e.facing = d.aim;
            d.t -= dt;
            if d.t <= 0.0 {
                d.state = DrakeState::Waddle;
                d.t = 0.0;
                d.pecks_left = 0;
            }
        }
    }
    landed
}

/// Give the body its state on the first tick it is seen: the director's guest
/// spawner inserts one, but the console (`spawn drake`), the Library scene and
/// a test all hand the floor a bare `Enemy`.
///
/// Chain right after `floor_sewer::floor_sewer_system` (the same slot, after
/// `enemy_wave4::wave4_system`).
#[allow(clippy::too_many_arguments)]
pub fn drake_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut Drake>)>,
    decoys: Query<(&SpawnId, &powerups::Decoy)>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    // Cheap gate: no Drake on the floor, no sort and no rng.
    if !enemies.iter().any(|(_, _, e, _)| e.kind == EnemyKind::Drake) {
        return;
    }
    order.clear();
    order.extend(
        enemies
            .iter()
            .filter(|(_, _, e, _)| e.kind == EnemyKind::Drake)
            .map(|(entity, id, _, _)| (id.0, entity)),
    );
    order.sort_unstable_by_key(|&(id, _)| id);

    let rat = player.pos;
    let rat_alive = player.alive();
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e, state)) = enemies.get_mut(entity) else {
            continue;
        };
        let Some(mut state) = state else {
            // First tick alive: give it its state; it starts next tick.
            commands.entity(entity).insert(Drake::default());
            continue;
        };
        if !e.alive() {
            // A corpse is not exposed: the recovery multiplier must not follow
            // the body into the reap.
            if state.state != DrakeState::Waddle {
                state.state = DrakeState::Waddle;
                state.t = 0.0;
                state.pecks_left = 0;
            }
            mirror(&mut e, &state);
            continue;
        }
        let target = lure_target(&e, rat, decoy, &level);
        if drake_tick(
            &mut e,
            &mut state,
            dt,
            target,
            rat,
            rat_alive,
            &level,
            &mut player,
            &mut sfx,
        ) {
            vfx::emit_player_hurt(&mut particles, &mut rng.0, rat);
        }
        mirror(&mut e, &state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::level::{Puddle, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;
    use rand::SeedableRng;

    const DT: f32 = 1.0 / 60.0;

    fn open_floor(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(drake_system).unwrap();
        w.end_tick();
    }

    fn secs(s: f32) -> usize {
        (s / DT).ceil() as usize
    }

    fn state(w: &World, entity: Entity) -> Drake {
        *w.ecs.get::<Drake>(entity).expect("the drake has its state")
    }

    /// A world with one Drake `offset` from the rat, its state attached.
    fn world_with_drake(offset: Vec2) -> (World, Entity) {
        let mut w = World::new(Theme::Sewer, 7);
        open_floor(&mut w);
        w.clear_enemies();
        w.player.pos = Vec2::new(400.0, 400.0);
        let at = w.player.pos + offset;
        let entity = w.spawn_enemy(EnemyKind::Drake, at, 1.0);
        // The first tick only attaches the component.
        tick(&mut w);
        (w, entity)
    }

    #[test]
    fn the_kind_is_registered_counted_and_capped_as_a_guest() {
        use crate::game::roster;
        assert_eq!(EnemyKind::all().len(), 50);
        assert!(EnemyKind::all().contains(&EnemyKind::Drake));
        assert_eq!(EnemyKind::Drake.name(), "drake");
        assert_eq!(EnemyKind::Drake.base_hp(), 92.8);
        assert_eq!(EnemyKind::Drake.speed(), 30.0);
        assert_eq!(EnemyKind::Drake.damage(), 5.0);
        assert_eq!(EnemyKind::Drake.xp(), 30);
        assert_eq!(EnemyKind::Drake.radius(), 8.0);
        assert_eq!(
            crate::game::collision::SizeClass::of_enemy(EnemyKind::Drake),
            crate::game::collision::SizeClass::Large
        );
        assert!(EnemyKind::Drake.heavy());
        assert_eq!(EnemyKind::Drake.archetype(), EnemyKind::Brute);
        // Nobody's slot, on any floor.
        for theme in Theme::playable() {
            assert!(roster::floor(theme).slot_of(EnemyKind::Drake).is_none());
        }
    }

    #[test]
    fn no_drake_before_2_30_and_one_after() {
        // The gate on its own: the Sewer at 0:00, 1:40 and 2:29.9 refuses; at
        // 2:30 it answers (the floor is empty and the gap is long spent).
        for (curve, want) in [(0.0f32, false), (100.0, false), (149.9, false), (150.0, true)] {
            assert_eq!(
                guest_due(Theme::Sewer, curve, curve, 0.0, 0, false, false),
                want,
                "at {curve}"
            );
        }
    }

    #[test]
    fn at_most_one_drake_before_8_00_and_two_after() {
        // One alive: refused before 8:00, allowed from 8:00 until two.
        assert!(!guest_due(Theme::Sewer, 479.9, 600.0, 0.0, 1, false, false));
        assert!(guest_due(Theme::Sewer, 480.0, 600.0, 0.0, 1, false, false));
        assert!(!guest_due(Theme::Sewer, 600.0, 600.0, 0.0, 2, false, false));
        // …and never two before it.
        assert!(!guest_due(Theme::Sewer, 151.0, 600.0, 0.0, 1, false, false));
    }

    #[test]
    fn the_45s_gap_runs_from_the_last_spawn_or_death() {
        // Spawned at 200: nothing until 245, whatever the clock.
        assert!(!guest_due(Theme::Sewer, 300.0, 244.9, 200.0, 0, false, false));
        assert!(guest_due(Theme::Sewer, 300.0, 245.0, 200.0, 0, false, false));
        // A death at 400 restarts it (the caller writes `last` on the count
        // going down).
        assert!(!guest_due(Theme::Sewer, 600.0, 440.0, 400.0, 0, false, false));
        assert!(guest_due(Theme::Sewer, 600.0, 445.0, 400.0, 0, false, false));
    }

    #[test]
    fn never_during_the_boss_fight_or_beside_a_miniboss() {
        assert!(!guest_due(Theme::Sewer, 600.0, 600.0, 0.0, 0, true, false));
        assert!(!guest_due(Theme::Sewer, 600.0, 600.0, 0.0, 0, false, true));
    }

    #[test]
    fn a_drake_is_only_ever_the_sewers_guest() {
        for theme in Theme::playable() {
            assert_eq!(
                guest_due(theme, 600.0, 600.0, 0.0, 0, false, false),
                theme == Theme::Sewer,
                "{theme:?}"
            );
        }
    }

    #[test]
    fn the_volley_is_tell_then_three_pecks_then_recover() {
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        tick(&mut w);
        let d = state(&w, e);
        assert_eq!(d.state, DrakeState::Tell, "in range: it commits");
        assert_eq!(d.pecks_left, PECK_COUNT);
        assert!(d.aim.x < -0.99, "aimed at the rat: {:?}", d.aim);

        // The tell holds, planted, for 0.45 s.
        let mut pecks = 0usize;
        let mut recover = 0usize;
        let mut tell = 0usize;
        for _ in 0..secs(TELL_SECS + PECK_SECS * 3.0 + RECOVER_SECS) {
            tick(&mut w);
            match state(&w, e).state {
                DrakeState::Tell => tell += 1,
                DrakeState::Peck => pecks += 1,
                DrakeState::Recover => recover += 1,
                DrakeState::Waddle => {}
            }
        }
        assert!(
            tell + 2 >= secs(TELL_SECS) && tell <= secs(TELL_SECS) + 2,
            "tell {tell} ticks"
        );
        assert!(
            pecks + 4 >= secs(PECK_SECS * 3.0) && pecks <= secs(PECK_SECS * 3.0) + 4,
            "peck {pecks} ticks"
        );
        assert!(
            recover + 2 >= secs(RECOVER_SECS) && recover <= secs(RECOVER_SECS) + 2,
            "recover {recover} ticks"
        );
    }

    #[test]
    fn a_planted_drake_does_not_move() {
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        tick(&mut w);
        let anchor = state(&w, e).anchor;
        w.ecs.get_mut::<Enemy>(e).unwrap().pos = anchor + Vec2::new(0.5, 0.0);
        tick(&mut w);
        let pos = w.ecs.get::<Enemy>(e).unwrap().pos;
        assert_eq!((pos.x, pos.y), (anchor.x, anchor.y));
    }

    /// Run the state machine until the aim has locked (a tick past
    /// `TELL_LOCK_SECS` into the tell).
    fn tick_to_the_lock(w: &mut World, e: Entity) {
        for _ in 0..secs(TELL_SECS) + 4 {
            tick(w);
            let d = state(w, e);
            if d.state != DrakeState::Tell || TELL_SECS - d.t >= TELL_LOCK_SECS + 1e-3 {
                return;
            }
        }
        panic!("the tell never reached its lock");
    }

    #[test]
    fn a_peck_in_the_lane_hits_and_one_that_stepped_aside_misses() {
        // Standing in the lane: the first peck lands.
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        w.player.iframes = 0.0;
        let hp = w.player.hp;
        let mut hit = false;
        for _ in 0..secs(TELL_SECS + PECK_SECS * 3.0) {
            tick(&mut w);
            if w.player.hp < hp {
                hit = true;
                break;
            }
        }
        assert!(hit, "a rat standing in the locked lane was never pecked");
        assert_eq!(state(&w, e).state, DrakeState::Peck, "one hit, mid-volley");

        // Stepping out of the lane after the lock: every peck misses.
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        w.player.iframes = 0.0;
        tick_to_the_lock(&mut w, e);
        assert_eq!(state(&w, e).state, DrakeState::Tell, "still warning");
        // Cross behind its shoulder: the lane is 6 px wide and 22 px long, and
        // the lock means it does not turn to follow.
        w.player.pos += Vec2::new(0.0, 40.0);
        let hp = w.player.hp;
        for _ in 0..secs(TELL_SECS + PECK_SECS * 3.0 + RECOVER_SECS) {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp, "a peck found a rat that left the lane");
    }

    #[test]
    fn the_aim_tracks_for_025s_then_locks() {
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        tick(&mut w);
        // Inside the tracking window the aim follows the rat…
        w.player.pos = w.ecs.get::<Enemy>(e).unwrap().pos + Vec2::new(0.0, 20.0);
        tick(&mut w);
        assert!(
            state(&w, e).aim.y > 0.9,
            "the aim did not track inside the lock window: {:?}",
            state(&w, e).aim
        );
        // …and once locked it does not move again for the rest of the tell.
        tick_to_the_lock(&mut w, e);
        let locked = state(&w, e).aim;
        w.player.pos = w.ecs.get::<Enemy>(e).unwrap().pos + Vec2::new(0.0, -20.0);
        let mut guard = 0;
        while state(&w, e).state == DrakeState::Tell && guard < secs(TELL_SECS) + 8 {
            tick(&mut w);
            guard += 1;
            if state(&w, e).state == DrakeState::Tell {
                assert_eq!(state(&w, e).aim, locked, "the aim moved after the lock");
            }
        }
        assert!(guard > 0, "the tell never ran");
    }

    #[test]
    fn a_drake_paddles_faster_in_a_puddle() {
        let (mut w, e) = world_with_drake(Vec2::new(-200.0, 0.0));
        let en = w.ecs.get::<Enemy>(e).unwrap().clone();
        assert_eq!(water_gait(&en, &w.level), 1.0, "a dry floor is 30 px/s");
        let at = en.pos;
        w.level.puddles = vec![Puddle {
            pos: at,
            rx: 90.0,
            ry: 60.0,
            seed: 3,
        }];
        let en = w.ecs.get::<Enemy>(e).unwrap().clone();
        let wet = water_gait(&en, &w.level);
        assert!((wet - WET_SPEED / 30.0).abs() < 1e-6, "a puddle is 40 px/s");
        // Every other kind is untouched, wet or dry.
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        for kind in EnemyKind::all() {
            if kind == EnemyKind::Drake {
                continue;
            }
            let mut other = Enemy::spawn(kind, at, 1.0, &mut rng);
            other.pos = at;
            assert_eq!(water_gait(&other, &w.level), 1.0, "{kind:?}");
        }
    }

    #[test]
    fn the_drawn_bill_and_the_hit_test_agree_at_full_extension() {
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        tick(&mut w);
        let mut d = state(&w, e);
        d.state = DrakeState::Peck;
        d.t = PECK_BACK_SECS;
        let body = w.ecs.get::<Enemy>(e).unwrap().clone();
        let view = drake_view(&body, &d);
        assert_eq!(view.pose, DuckPose::Peck(PECK_STRIKE_PROGRESS));
        let tip = art_duck::bill_tip(&view);
        let reach = tip.dist(body.pos);
        assert!(
            (reach - art_duck::PECK_REACH).abs() < 0.05,
            "the bill reaches {reach} px at full extension, not {}",
            art_duck::PECK_REACH
        );
        // The lane the tell draws ends at the same reach.
        let (a, b) = peck_lane(body.pos, d.aim);
        assert_eq!(a, body.pos);
        assert!((b.dist(a) - art_duck::PECK_REACH).abs() < 1e-3);
    }

    #[test]
    fn the_recover_is_the_exposed_second() {
        let (w, e) = world_with_drake(Vec2::new(200.0, 0.0));
        let mut body = w.ecs.get::<Enemy>(e).unwrap().clone();
        let plain = body.hurt_from(10.0, body.pos + Vec2::new(4.0, 0.0));
        body.hp = body.max_hp;
        body.w4.phase = PHASE_RECOVER;
        let exposed = body.hurt_from(10.0, body.pos + Vec2::new(4.0, 0.0));
        assert!((exposed / plain - RECOVER_DAMAGE_MULT).abs() < 1e-6);
        // Any other state is a plain hit.
        body.hp = body.max_hp;
        body.w4.phase = PHASE_TELL;
        let planted = body.hurt_from(10.0, body.pos + Vec2::new(4.0, 0.0));
        assert_eq!(planted, plain);
    }

    #[test]
    fn the_mirror_carries_the_state_to_bare_enemy_readers() {
        let (mut w, e) = world_with_drake(Vec2::new(20.0, 0.0));
        tick(&mut w);
        let body = w.ecs.get::<Enemy>(e).unwrap().clone();
        assert_eq!(body.w4.phase, PHASE_TELL);
        let seen = mirrored_view(&body).expect("a Drake view");
        assert!(matches!(seen.pose, DuckPose::Tell(_)));
        // No other kind answers.
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let other = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        assert!(mirrored_view(&other).is_none());
    }

    #[test]
    fn the_re_aim_is_capped_at_35_degrees() {
        let from = Vec2::new(1.0, 0.0);
        // Within the cap: it simply points at the target.
        let near = Vec2::from_angle(0.3);
        assert_eq!(rotate_toward(from, near, REAIM_MAX_RAD), near);
        // Beyond it: exactly the cap, on the short side.
        let far = Vec2::from_angle(2.0);
        let out = rotate_toward(from, far, REAIM_MAX_RAD);
        let turn = out.dot(from).clamp(-1.0, 1.0).acos();
        assert!((turn - REAIM_MAX_RAD).abs() < 1e-4, "turned {turn}");
        assert!(out.y > 0.0, "it turned the wrong way round: {out:?}");
        // Degenerate inputs are not a panic and never move the aim.
        assert_eq!(rotate_toward(Vec2::ZERO, far, REAIM_MAX_RAD), far);
        assert_eq!(rotate_toward(from, Vec2::ZERO, REAIM_MAX_RAD), from);
    }

    /// The four seeded 60 s golden scripts never contain a Drake: the guest is
    /// blind before 2:30 on the curve clock.
    #[test]
    fn no_golden_script_meets_a_drake() {
        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let mut w = World::new(theme, 1000);
            for _ in 0..60 * 60 {
                w.update(DT, Vec2::ZERO, false);
                assert!(
                    !w.enemies().iter().any(|e| e.kind == EnemyKind::Drake),
                    "{theme:?}: a Drake inside the 60 s golden window"
                );
            }
            assert!(w.elapsed < FROM_CLOCK, "{theme:?}: the run reached 2:30");
        }
    }

    /// The guest's clock really is the curve clock: a Sewer world stepped past
    /// 2:30 spawns one (the run itself, not a hand-built context).
    #[test]
    fn a_full_run_spawns_its_first_drake_after_2_30() {
        let mut w = World::new(Theme::Sewer, 4242);
        let mut at = None;
        while w.elapsed < 200.0 {
            w.update(0.1, Vec2::ZERO, false);
            w.player.hp = w.player.max_hp();
            if w.enemies().iter().any(|e| e.kind == EnemyKind::Drake) {
                at = Some(w.elapsed);
                break;
            }
        }
        let at = at.expect("no Drake arrived by 3:20 on the Sewer");
        assert!(at >= FROM_CLOCK - 0.5, "a Drake arrived at {at}s, before 2:30");
    }
}
