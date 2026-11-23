//! Wave 3's self-driving weapons: **Laser Eyes**, **Super Dash**, and the
//! bookkeeping **Counterweight**'s melee nudge needs.
//!
//! `weapons::fire_weapons_system` skips both kinds
//! (`WeaponKind::fires_outside_fire_one`), because neither is a "spawn a
//! projectile now" weapon:
//!
//! - **Laser Eyes** is an ordinary auto-firing weapon whose shot is a **sweep**:
//!   it charges its own `Weapon::timer` here, and on the trigger tick it locks
//!   the aim direction and pivots one continuous beam from one edge of the arc
//!   to the other over `Weapon::laser_sweep_secs` (L1 60° / 0.55 s, L5 110° /
//!   0.90 s). Everything the beam passes over takes the whole old pulse burst's
//!   damage, once per sweep (a per-sweep hit set), and the next sweep runs the
//!   other way. From L4 a second, mirrored beam sweeps at the same time and
//!   shares that hit set. Each beam stops at the first wall on its own ray.
//! - **Super Dash** is a weapon whose *fire* is the dash. `player_system`
//!   records the dash's real travelled segments into [`DashTrack`]; this system
//!   damages every body those segments crossed, once per body per dash. A
//!   blocked or cooling dash records no segment, so it buys no hit, and the
//!   dash's own motion and immunity are untouched (nothing here writes
//!   `Player`).
//!
//! Schedule slot 11b (right after `weapons::active_weapons_system`): the cached
//! [`EnemyIndex`] is the live `SpawnId` order there (nothing has spawned or
//! been reaped since `grid_refresh`), the damage lands before
//! `player_bullet_hits`/`reap_enemies` in the same tick, and both systems see
//! this tick's player position.
//!
//! Determinism: every walk is `SpawnId` order (or the cached `EnemyIndex`, which
//! *is* that order), every rng draw happens inside a `vfx` emitter called only
//! when something actually happened, and a run that never owns either weapon
//! draws no rng and changes no value — the golden traces do not move.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::segment_circle_hit;
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::resources::{
    DamageLedger, DashTrack, EnemyIndex, LaserFx, MeleeNudge, RngRes, SfxQueue, Shake, TickInput,
};
use crate::game::ecs::systems::boss::Boss;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::{
    WeaponKind, LASER_HALF_WIDTH,
};
use crate::math::Vec2;

#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;

/// How often Counterweight may nudge the same body (design §5).
pub const MELEE_NUDGE_COOLDOWN: f32 = 0.4;

/// **Super Dash**'s native drag: px/s of velocity added to every body the
/// dash's legs cross (Counterweight's `Stats::shove` multiplies it like every
/// other weapon's native shove).
pub const SUPER_DASH_SHOVE: f32 = 40.0;

/// A **full charge** widens the dash's swept radius by this fraction (a held
/// dash is a taller corridor, not just a longer one).
pub const SUPER_DASH_RADIUS_PER_CHARGE: f32 = 0.5;

/// A **full charge** multiplies the per-body damage by `1 + this`: up to 2.5x
/// the weapon's own hit, which is what makes the hold worth the wait.
pub const SUPER_DASH_DAMAGE_PER_CHARGE: f32 = 1.5;

/// The tick's stable `SpawnId` enemy order.
///
/// Prefers the cached [`EnemyIndex`] (identical order, no sort) and falls back
/// to sorting a reused `Local` buffer once for the tick, which is what narrow
/// single-system tests need (they never run the grid). Same rule as
/// `fire_weapons_system`; the returned slice borrows only `index` or
/// `fallback`, never the query, so a caller can still take `&mut` on it.
fn stable_order<'a>(
    index: &'a EnemyIndex,
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    fallback: &'a mut Vec<Entity>,
) -> &'a [Entity] {
    if index.0.len() == enemies.iter().len() {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        fallback.clear();
        fallback.extend(enemies.iter().map(|(entity, _, _)| entity));
        fallback.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        fallback.as_slice()
    }
}

/// Does the bar `origin -> origin + dir * len`, `half_w` px either side, reach
/// the circle at `center` with radius `r`?
///
/// The beam is a bar, not a capsule: the body's near edge has to be in front of
/// the beam's end (which is where a wall stopped it), and its perpendicular
/// distance from the ray has to be inside `half_w + r`. Bodies behind the
/// muzzle fail the same projection, so a rat standing in a crowd never shoots
/// backwards.
#[inline]
pub fn ray_hits_circle(origin: Vec2, dir: Vec2, len: f32, half_w: f32, center: Vec2, r: f32) -> bool {
    let rel = center - origin;
    let along = rel.dot(dir);
    if along + r < 0.0 || along - r > len {
        return false;
    }
    let lateral = (rel - dir * along).len();
    lateral <= half_w + r
}

/// The angular step a sweep is cut into when two ticks are far apart: a beam
/// that jumped a whole tick's angle would skip a body whose radius is a few px.
/// 0.035 rad (2°) is about 2 px of gap at the L5 reach, well under the smallest
/// body's radius, so every body between two ticks is crossed by some sample.
const SWEEP_STEP_RAD: f32 = 0.035;

/// How often (seconds) the sweep emits its beam sparkle. The drawn beam is
/// `art_rat::draw_laser_eyes`'s job; this is the particle garnish on top, and
/// 60 a second of a 40-dot row would drown the frame budget.
const LASER_SPARK_EVERY: f32 = 0.05;

/// The numbers one Laser Eyes sweep was cast with, snapshotted at the trigger
/// tick so a level-up card opened mid-sweep cannot change a hit already paid
/// for (the same rule every projectile and lash follows).
#[derive(Clone, Copy, Debug)]
struct SweepCast {
    /// The old burst's pulse count: the damage multiplier of one sweep hit.
    pulses: u32,
    /// Nominal beam length before the wall stop (Lens Cap applied).
    reach: f32,
    /// Per-**hit** damage (the whole old burst's, Sharp Teeth applied).
    dmg: f32,
    /// Fired by the evolved (**Carbon Copy**) weapon.
    evolved: bool,
    /// Half the arc, in radians.
    half: f32,
    /// Seconds the sweep lasts.
    secs: f32,
    /// A second, mirrored beam sweeps too (L4+).
    twin: bool,
}

/// The angle a sweep holds at progress `t` (0..1): from one edge of the arc to
/// the other, in this sweep's direction.
#[inline]
fn sweep_angle(base: f32, dir: f32, half: f32, t: f32) -> f32 {
    base + dir * (-half + 2.0 * half * t.clamp(0.0, 1.0))
}

/// Resolve one beam's angular **step**: every body the segment crosses at any
/// sample angle between `from_angle` and `to_angle`, once per sweep.
///
/// The sweep keeps its own hit set, so a body the beam passes over is cut
/// exactly once however many ticks the crossing takes, and the second (L4+)
/// beam shares that set — both beams together deal one sweep's damage. Walls
/// stop each sample ray where they stop the drawn beam, so a body past the
/// first face is never touched.
///
/// Returns the wall-stopped beam length at `to_angle` (the art draws the live
/// beam there).
#[allow(clippy::too_many_arguments)]
fn sweep_step(
    origin: Vec2,
    from_angle: f32,
    to_angle: f32,
    nominal: f32,
    dmg: f32,
    ghost: bool,
    hits: &mut crate::game::ecs::resources::LaserHits,
    boss_hit: &mut bool,
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
) -> f32 {
    let kind = WeaponKind::LaserEyes;
    let delta = to_angle - from_angle;
    let steps = (delta.abs() / SWEEP_STEP_RAD).ceil().max(1.0) as u32;
    let mut last_len = nominal;
    for step in 0..=steps {
        let angle = from_angle + delta * (step as f32 / steps as f32);
        let dir = Vec2::from_angle(angle);
        // Walls stop the beam: everything past the first face is shadowed.
        let end = origin + dir * nominal;
        let stop = level
            .tiles
            .segment_hit(origin, end, LASER_HALF_WIDTH)
            .map(|t| t.clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let len = nominal * stop;
        last_len = len;

        for &entity in order {
            let Ok((_, id, mut e)) = enemies.get_mut(entity) else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            if !ray_hits_circle(origin, dir, len, LASER_HALF_WIDTH, e.pos, e.radius) {
                continue;
            }
            // One cut per sweep, and the set is the only record: a body already
            // in it (or a full set) is left alone, so nothing can be cut twice.
            if !hits.insert(id.0) {
                continue;
            }
            let hp_before = e.hp.max(0.0);
            // Directional armour: the beam arrives from the rat's eyes.
            let applied = e.hurt_from(dmg, origin);
            let killed = !e.alive();
            ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
            let pos = e.pos;
            let enemy_radius = e.radius;
            let enemy_id = e.id;
            if ghost {
                vfx::emit_carbon_copy_hit(particles, &mut rng.0, pos, pos - origin, killed);
            } else {
                vfx::emit_laser_hit(particles, &mut rng.0, pos, pos - origin, killed);
            }
            damage_numbers.record(
                enemy_id,
                pos,
                enemy_radius,
                applied,
                killed,
                DamageSource::Normal,
            );
        }

        // The boss is a body too: its grounded hit shapes are cut like
        // everything else (a lifted foot or a mid-pounce jaguar is not).
        for mut boss in bosses.iter_mut() {
            if boss.dead || *boss_hit {
                continue;
            }
            let shapes = boss.hit_shapes();
            if shapes.iter().next().is_none() {
                continue;
            }
            if !shapes.segment_hits_grounded(
                origin,
                origin + dir * len.max(0.5),
                LASER_HALF_WIDTH,
            ) {
                continue;
            }
            let amount = dmg * boss.damage_mult;
            if !boss.take_damage(amount) {
                continue;
            }
            *boss_hit = true;
            ledger.add_boss(kind, amount);
            let killed = boss.hp <= 0.0;
            let (boss_pos, boss_radius, boss_id) =
                (boss.pos, boss.kind.body_radius(), boss.id);
            if ghost {
                vfx::emit_carbon_copy_hit(
                    particles,
                    &mut rng.0,
                    boss_pos,
                    boss_pos - origin,
                    killed,
                );
            } else {
                vfx::emit_laser_hit(particles, &mut rng.0, boss_pos, boss_pos - origin, killed);
            }
            damage_numbers.record(
                boss_id,
                boss_pos,
                boss_radius,
                amount,
                killed,
                DamageSource::Normal,
            );
        }
    }
    last_len
}

/// Laser Eyes: charge the weapon and sweep one continuous beam across an arc.
///
/// A sweep is the weapon's shot, so it fires on its own cycle without needing a
/// target (like Squeak Nova, unlike Gnaw): a beam across a corridor is worth
/// firing before the horde is on screen. `player.face` (or the reticle) is read
/// **once**, at the trigger tick — turning afterwards does not drag the arc,
/// which is what makes a sweep read as one deliberate cut.
///
/// Batch 12 replaced the old burst of evenly spaced pulses with this: the beam
/// pivots from one edge of the arc to the other over `laser_sweep_secs`,
/// painting the whole arc, and the **next** sweep runs the other way
/// (`LaserFx::dir` alternates). Everything it crosses takes the whole old
/// burst's damage once (a per-sweep hit set), so a body the beam passes over
/// takes exactly what the fan dealt it and the weapon's dps is unchanged. From
/// L4 a second, mirrored beam sweeps the other way at the same time.
#[allow(clippy::too_many_arguments)]
pub fn laser_eyes_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut fx: ResMut<LaserFx>,
    mut fallback: Local<Vec<Entity>>,
) {
    let dt = input.dt;
    let (dmg_mult, fire_rate, reach_mult) = {
        let s = player.loadout.stats;
        (player.damage_mult(), s.fire_rate_mult.max(0.1), s.reach_mult)
    };

    // 1. Charge (and trigger). Only one sweep is in flight at a time, and a
    // sweep is always shorter than its cycle, so a retrigger cannot overlap.
    let mut trigger: Option<SweepCast> = None;
    for w in player.loadout.weapons.iter_mut() {
        if w.kind != WeaponKind::LaserEyes {
            continue;
        }
        w.timer -= dt;
        if w.timer > 0.0 || fx.active() {
            continue;
        }
        w.timer += (w.cooldown() / fire_rate).max(0.05);
        // The beam's length is a ranged reach: Lens Cap stretches it. The
        // per-hit damage is the whole old pulse burst's, so dps is unchanged.
        trigger = Some(SweepCast {
            pulses: w.laser_pulses().max(1),
            reach: reach_mult * w.laser_reach(),
            dmg: w.laser_hit_damage() * dmg_mult,
            evolved: w.evolved,
            half: w.laser_half_arc(),
            secs: w.laser_sweep_secs(),
            twin: w.laser_twin(),
        });
    }
    if let Some(cast) = trigger {
        // An aim point turns the sweep's centre to the reticle instead of the
        // facing (wave 3 controls); with no aim this is exactly the locked
        // `face`.
        let aimed = input
            .aim
            .map(|p| p - player.pos)
            .filter(|v| v.len_sq() > 4.0)
            .map(|v| v.normalized());
        let base = match aimed {
            Some(d) => d,
            None if player.face.len_sq() > 1e-4 => player.face.normalized(),
            None => Vec2::new(0.0, -1.0),
        };
        let origin = player.pos;
        // Alternate: the sweep that just finished started from one edge, so
        // this one starts from the other.
        let dir = if fx.dir >= 0.0 { -1.0 } else { 1.0 };
        let base_angle = base.angle();
        let angle = sweep_angle(base_angle, dir, cast.half, 0.0);
        // The second beam mirrors the first about the aim, sweeping the other
        // way through the same arc.
        let angle_b = 2.0 * base_angle - angle;
        // Carbon Copy: the sweep recorded at the end of the *previous* burst
        // replays now from its own origin and facing, the other (mirrored) way
        // through the arc. One copy is kept, and a copy is never itself
        // recorded (only a real sweep is).
        let recorded = if cast.evolved { fx.copy } else { None };
        let copy_secs = if recorded.is_some() { cast.secs } else { 0.0 };
        let copy_dir = -dir;
        let copy_prev = match recorded {
            Some(copy) => sweep_angle(copy.facing.angle(), copy_dir, cast.half, 0.0),
            None => 0.0,
        };
        *fx = LaserFx {
            left: cast.secs,
            age: 0.0,
            pulses: cast.pulses,
            facing: base,
            origin,
            reach: cast.reach,
            dmg: cast.dmg,
            half: cast.half,
            dir,
            twin: cast.twin,
            angle,
            beam: 0.0,
            angle_b,
            beam_b: 0.0,
            prev_angle: angle,
            prev_angle_b: angle_b,
            total: cast.secs,
            evolved: cast.evolved,
            copy: recorded,
            copy_left: copy_secs,
            copy_age: 0.0,
            copy_fired: 0,
            copy_half: cast.half,
            copy_secs,
            copy_dir,
            copy_prev_angle: copy_prev,
            ..LaserFx::default()
        };
        shake.amount = shake.amount.max(0.9);
        vfx::emit_laser_muzzle(&mut particles, &mut rng.0, origin, base);
        // One zap per sweep: the beam is continuous now, so its sound is too.
        sfx.0.push(Sfx::LaserEyes);
        if let Some(copy) = recorded {
            // The negative's camera: an amber eye pair left where the recorded
            // sweep fired from.
            vfx::emit_carbon_copy_afterimage(
                &mut particles,
                &mut rng.0,
                copy.origin,
                copy.facing,
                copy.pulses,
            );
        }
    }

    if !fx.active() {
        return;
    }

    // 2. Sweep the live beam (and its mirror) forward. The angles are a pure
    // function of `age`, so a replay with the same dt cuts the same arc.
    fx.age += dt;
    let t = (fx.age / fx.total.max(1e-4)).clamp(0.0, 1.0);
    let base_angle = fx.facing.angle();
    let (dir, half, reach, dmg) = (fx.dir, fx.half, fx.reach, fx.dmg);
    let angle = sweep_angle(base_angle, dir, half, t);
    let angle_b = 2.0 * base_angle - angle;
    let (prev, prev_b, twin) = (fx.prev_angle, fx.prev_angle_b, fx.twin);
    let spin = (fx.age / LASER_SPARK_EVERY).floor() != ((fx.age - dt) / LASER_SPARK_EVERY).floor();
    {
        let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
        let mut hits = fx.hits;
        let mut boss_hit = fx.boss_hit;
        let len = sweep_step(
            fx.origin,
            prev,
            angle,
            reach,
            dmg,
            false,
            &mut hits,
            &mut boss_hit,
            &level,
            &mut rng,
            &mut particles,
            order,
            &mut enemies,
            &mut bosses,
            &mut damage_numbers,
            &mut ledger,
        );
        let len_b = if twin {
            sweep_step(
                fx.origin,
                prev_b,
                angle_b,
                reach,
                dmg,
                false,
                &mut hits,
                &mut boss_hit,
                &level,
                &mut rng,
                &mut particles,
                order,
                &mut enemies,
                &mut bosses,
                &mut damage_numbers,
                &mut ledger,
            )
        } else {
            0.0
        };
        fx.hits = hits;
        fx.boss_hit = boss_hit;
        fx.prev_angle = angle;
        fx.prev_angle_b = angle_b;
        fx.angle = angle;
        fx.beam = len;
        fx.angle_b = angle_b;
        fx.beam_b = len_b;
        fx.fired = fx.hits.len();
        // The steady beam's garnish, at 20 Hz rather than once a tick.
        if spin {
            vfx::emit_laser_pulse(&mut particles, &mut rng.0, fx.origin, angle, len, fx.fired);
        }
    }

    // 3. The Carbon Copy afterimage, on the same clock and the other way round
    // the arc (it starts where the live beam will end).
    if fx.copy_left > 0.0 {
        if let Some(copy) = fx.copy {
            fx.copy_age += dt;
            let ct = (fx.copy_age / fx.copy_secs.max(1e-4)).clamp(0.0, 1.0);
            let cdir = fx.copy_dir;
            let c_base = copy.facing.angle();
            let c_angle = sweep_angle(c_base, cdir, fx.copy_half, ct);
            let c_prev = fx.copy_prev_angle;
            {
                let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
                let mut hits = fx.copy_hits;
                let mut boss_hit = fx.copy_boss_hit;
                sweep_step(
                    copy.origin,
                    c_prev,
                    c_angle,
                    copy.reach,
                    copy.dmg * crate::game::ecs::systems::evolutions::CARBON_COPY_DAMAGE,
                    true,
                    &mut hits,
                    &mut boss_hit,
                    &level,
                    &mut rng,
                    &mut particles,
                    order,
                    &mut enemies,
                    &mut bosses,
                    &mut damage_numbers,
                    &mut ledger,
                );
                fx.copy_hits = hits;
                fx.copy_boss_hit = boss_hit;
                fx.copy_prev_angle = c_angle;
                fx.copy_fired = fx.copy_hits.len();
            }
            fx.copy_left = (fx.copy_left - dt).max(0.0);
        } else {
            fx.copy_left = 0.0;
        }
    }

    fx.left = (fx.left - dt).max(0.0);
    if fx.left <= 0.0 {
        // Done. A completed sweep is what the next burst's afterimage replays
        // (Carbon Copy keeps exactly one); everything else starts clean, and
        // the sweep direction is carried so the next sweep runs the other way.
        let recorded = if fx.evolved { fx.copy_of_this_sweep() } else { None };
        let dir = fx.dir;
        *fx = LaserFx {
            copy: recorded,
            dir,
            ..LaserFx::default()
        };
    }
}

/// Super Dash: damage every body this dash's travelled segment crosses, once
/// per body per dash.
///
/// `player_system` writes the segment (`DashTrack`); this system only reads it,
/// so a press that never became a dash cannot reach this code at all.
#[allow(clippy::too_many_arguments)]
pub fn super_dash_system(
    player: Res<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut track: ResMut<DashTrack>,
    mut fallback: Local<Vec<Entity>>,
) {
    let Some((from, to)) = track.segment else {
        return;
    };
    // The weapon's own numbers: no weapon, no hit — and no rng, no sfx, so a
    // run without Super Dash is byte-identical to one before this feature.
    let Some(w) = player
        .loadout
        .weapons
        .iter()
        .find(|w| w.kind == WeaponKind::SuperDash)
    else {
        return;
    };
    let radius = w.dash_radius();
    let dmg = w.damage() * player.damage_mult();
    // A charged dash (the player held the dash input and released it) is wider
    // and hits harder; the arms above are the whole curve, so a tap dash — the
    // pre-charge behaviour, and every scripted `charge = 0` — is unchanged.
    let charge = track.charge.clamp(0.0, 1.0);
    let radius = radius * (1.0 + SUPER_DASH_RADIUS_PER_CHARGE * charge);
    let dmg = dmg * (1.0 + SUPER_DASH_DAMAGE_PER_CHARGE * charge);
    // Counterweight scales the dash's native drag too: it is a shove, so the
    // passive that raises every weapon's shove has to reach it (it used to be
    // a hard-coded 40 px/s the passive could not touch).
    let shove = player.loadout.stats.shove(SUPER_DASH_SHOVE);
    let kind = WeaponKind::SuperDash;
    // The legs the rat actually travelled this tick (collision-resolved
    // sub-steps). A chord across them could cut a corner a wall put in the
    // path; `record` guarantees at least one leg whenever `segment` is set, so
    // a dash that moved nothing (pressed flush into a wall) hits nothing.
    // Copied out of the resource (the array is `Copy`), so the hit walk below
    // can still mark hits and extend the Ripcord route.
    let legs = track.segs;
    let leg_n = track.seg_len;
    let on_legs = |pos: Vec2, other: f32| {
        (0..leg_n).any(|i| {
            let (a, b) = legs[i];
            segment_circle_hit(a, b, radius, pos, other)
        })
    };

    // One sound for the whole dash, on its first leg.
    if track.legs == 1 {
        sfx.0.push(Sfx::SuperDash);
    }
    // Ripcord: the real travelled legs are kept for 0.6 s after the dash, so
    // the evolution damages the route the rat actually took (a dash stopped by
    // a wall records its short leg, exactly like the dash hit does).
    if w.evolved {
        for &(a, b) in &legs[..leg_n] {
            track.extend_route(a, b);
        }
    }
    vfx::emit_super_dash_comb(&mut particles, &mut rng.0, from, to, radius);

    let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
    for &entity in order {
        let Ok((_, id, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || track.already_hit(id.0) {
            continue;
        }
        if !on_legs(e.pos, e.radius) {
            continue;
        }
        track.mark_hit(id.0);
        let hp_before = e.hp.max(0.0);
        // Directional armour: the hit arrives along the dash's travel, i.e.
        // from the point one leg behind the body (a zero-length leg has no
        // direction and takes the unmodified multiplier).
        let arrives_from = e.pos - (to - from);
        let applied = e.hurt_from(dmg, arrives_from);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        // The dash drags what it crosses along with it: the ride is the point.
        let out = (pos - from).normalized();
        e.vel += out * shove;
        vfx::emit_super_dash_hit(&mut particles, &mut rng.0, pos, pos - from, killed);
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::Normal,
        );
    }

    // Bosses take the damage only (the slide is Ripcord's, in the evolution
    // table).
    for mut boss in bosses.iter_mut() {
        if boss.dead || track.boss_hit {
            continue;
        }
        let shapes = boss.hit_shapes();
        if shapes.iter().next().is_none() {
            continue;
        }
        // The same travelled legs the horde is tested against, not the
        // `from → to` chord: a dash that slid round a corner must not reach a
        // boss through the masonry the chord cuts across.
        if !(0..leg_n).any(|i| shapes.segment_hits_grounded(legs[i].0, legs[i].1, radius)) {
            continue;
        }
        track.boss_hit = true;
        let amount = dmg * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        ledger.add_boss(kind, amount);
        let killed = boss.hp <= 0.0;
        vfx::emit_super_dash_hit(&mut particles, &mut rng.0, boss.pos, boss.pos - from, killed);
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            killed,
            DamageSource::Normal,
        );
    }
}

/// Decay Counterweight's per-body nudge cooldowns (nothing else).
///
/// `weapons::fire_one` asks the log before nudging a melee hit; this system
/// keeps the timers honest. With no Counterweight stacks the log is always
/// empty, so the whole pass is a no-op.
pub fn melee_nudge_system(input: Res<TickInput>, mut nudge: ResMut<MeleeNudge>) {
    if nudge.len() > 0 {
        nudge.tick(input.dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::art_walls::WallKind;
    use crate::game::weapon::Weapon;
    use crate::game::world::World;

    /// Wipe the generated layout's walls so a test measures the beam, not the
    /// map.
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

    /// A 3-cell solid column whose left face is the line `x = 16 * cx`.
    fn wall_column(w: &mut World, cx: i32, cy: i32) -> f32 {
        w.level
            .tiles
            .fill_cells(cx, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        (cx * 16) as f32
    }

    /// Replace the loadout with one ready weapon at `level`.
    fn arm(w: &mut World, kind: WeaponKind, level: u8) {
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level,
            timer: 0.0,
            evolved: false,
        }];
    }

    fn run_laser(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(laser_eyes_system).unwrap();
        w.end_tick();
    }

    fn run_super_dash(w: &mut World) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(super_dash_system).unwrap();
        w.end_tick();
    }

    fn enemy_hp(w: &World, e: Entity) -> f32 {
        w.ecs.get::<Enemy>(e).map(|e| e.hp).unwrap_or(f32::NAN)
    }

    /// One sweep crosses its whole arc, from one edge to the other, and
    /// consecutive sweeps run the other way.
    #[test]
    fn a_sweep_spans_the_whole_arc_and_alternates() {
        let mut w = World::new(Theme::Sewer, 10);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 3);
        w.player.face = Vec2::new(1.0, 0.0);
        let half = {
            let mut l3 = Weapon::new(WeaponKind::LaserEyes);
            l3.level = 3;
            l3.laser_half_arc()
        };
        // Level 3 is the third tier: 80° of arc.
        assert!((half - 40.0f32.to_radians()).abs() < 1e-4, "L3 half-arc {half}");
        run_laser(&mut w, 1.0 / 60.0);
        let first = *w.ecs.resource::<LaserFx>();
        let base = 0.0; // facing +x
        assert!(first.active());
        assert_eq!(first.dir, 1.0, "the first sweep of a run starts at -half");
        // (Read after the trigger tick, so the beam is one step in.)
        assert!(
            (first.angle - (base - half)).abs() < 0.05,
            "a sweep starts at one edge: {}",
            first.angle
        );
        // Run it out, then trigger the next one.
        let mut ticks = 0;
        while w.ecs.resource::<LaserFx>().active() && ticks < 200 {
            run_laser(&mut w, 1.0 / 60.0);
            ticks += 1;
        }
        let done = *w.ecs.resource::<LaserFx>();
        w.player.loadout.weapons[0].timer = 0.0;
        run_laser(&mut w, 1.0 / 60.0);
        let second = *w.ecs.resource::<LaserFx>();
        assert_eq!(second.dir, -done.dir, "the sweep direction alternates");
        assert!(
            (second.angle - (base + half)).abs() < 0.05,
            "the next sweep starts at the other edge: {}",
            second.angle
        );
    }

    /// Per level: the beam's length, the damage one body takes **per sweep**
    /// (one old pulse's), the arc, its duration, and whether a
    /// second mirrored beam joins in.
    #[test]
    fn laser_numbers_per_level() {
        for (level, (pulses, reach, dmg, arc_deg, secs, twin)) in [
            (1u8, (3u32, 34.0f32, 6.0f32, 60.0f32, 0.55f32, false)),
            (2, (3, 40.0, 7.0, 70.0, 0.63, false)),
            (3, (4, 40.0, 8.0, 80.0, 0.70, false)),
            (4, (4, 48.0, 9.0, 95.0, 0.80, true)),
            (5, (5, 54.0, 10.0, 110.0, 0.90, true)),
        ] {
            let mut w = World::new(Theme::Sewer, 11);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::LaserEyes, level);
            run_laser(&mut w, 1.0 / 60.0);
            let fx = *w.ecs.resource::<LaserFx>();
            assert!(fx.active(), "L{level} sweep did not start");
            assert_eq!(fx.pulses, pulses, "L{level} pulse multiplier");
            assert_eq!(fx.reach, reach, "L{level} beam length");
            assert_eq!(fx.dmg, dmg, "L{level} damage per sweep hit");
            assert!(
                (fx.half - (arc_deg * 0.5f32).to_radians()).abs() < 1e-4,
                "L{level} half-arc: {}",
                fx.half
            );
            assert!((fx.total - secs).abs() < 1e-4, "L{level} sweep length");
            assert_eq!(fx.twin, twin, "L{level} second beam");
            // …and the sweep runs for its own duration, no longer.
            let mut ticks = 1;
            while w.ecs.resource::<LaserFx>().active() && ticks < 200 {
                run_laser(&mut w, 1.0 / 60.0);
                ticks += 1;
            }
            assert!(!w.ecs.resource::<LaserFx>().active(), "the sweep never ended");
            let ran = ticks as f32 / 60.0;
            assert!(
                (ran - secs).abs() < 0.06,
                "L{level}: the sweep ran {ran:.3}s, expected {secs:.3}s"
            );
        }
    }

    /// A body the beam passes over is cut **once** however many ticks the
    /// crossing takes, the beam pierces a whole line of bodies, and a body
    /// outside the arc is never touched.
    #[test]
    fn a_sweep_cuts_each_body_once_and_misses_what_it_does_not_cross() {
        let mut w = World::new(Theme::Sewer, 13);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1); // 60° arc, 34 px, 6 damage
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // Three bodies on the dead-ahead line (the beam pierces all of them),
        // one just inside the arc's edge, and two it never reaches: one well
        // outside the 30° half-arc, one behind the rat.
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(8.0, 0.0), 1.0);
        let mid = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(18.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(26.0, 0.0), 1.0);
        let edge = w.spawn_enemy(
            EnemyKind::Cat,
            p + Vec2::from_angle(25.0f32.to_radians()) * 20.0,
            1.0,
        );
        let outside = w.spawn_enemy(
            EnemyKind::Cat,
            p + Vec2::from_angle(60.0f32.to_radians()) * 20.0,
            1.0,
        );
        let behind = w.spawn_enemy(EnemyKind::Cat, p - Vec2::new(18.0, 0.0), 1.0);
        let full = [near, mid, far, edge, outside, behind].map(|e| enemy_hp(&w, e));

        let mut ticks = 0;
        while ticks < 200 {
            run_laser(&mut w, 1.0 / 60.0);
            ticks += 1;
            if !w.ecs.resource::<LaserFx>().active() {
                break;
            }
        }
        // 6 damage each (one old pulse), once.
        for (entity, before) in [near, mid, far, edge].iter().zip(full.iter()) {
            assert!(
                (enemy_hp(&w, *entity) - (before - 6.0)).abs() < 1e-3,
                "a body the beam crossed must take the sweep once: {} -> {}",
                before,
                enemy_hp(&w, *entity)
            );
        }
        assert_eq!(enemy_hp(&w, outside), full[4], "a body outside the arc was cut");
        assert_eq!(enemy_hp(&w, behind), full[5], "a body behind the rat was cut");
    }

    /// From L4 the second beam mirrors the first, and the two together still
    /// cut a body once (they share the sweep's hit set).
    #[test]
    fn the_second_beam_shares_the_sweeps_hits() {
        let mut w = World::new(Theme::Sewer, 18);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 4); // 95° arc, 48 px, 9 damage
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        let ahead = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(14.0, 0.0), 1.0);
        let full = enemy_hp(&w, ahead);
        let mut ticks = 0;
        while ticks < 200 {
            run_laser(&mut w, 1.0 / 60.0);
            ticks += 1;
            let fx = *w.ecs.resource::<LaserFx>();
            assert!(fx.twin || !fx.active(), "L4 must sweep a second beam");
            if !fx.active() {
                break;
            }
        }
        assert!(
            (enemy_hp(&w, ahead) - (full - 9.0)).abs() < 1e-3,
            "two beams must not double-hit one body: {} -> {}",
            full,
            enemy_hp(&w, ahead)
        );
    }

    /// The facing is locked at the trigger tick: turning the rat mid-burst
    /// must not drag the sweep.
    #[test]
    fn laser_locks_the_facing_at_burst_start() {
        let mut w = World::new(Theme::Sewer, 12);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        run_laser(&mut w, 1.0 / 60.0);
        let first = *w.ecs.resource::<LaserFx>();
        assert!((first.facing.x - 1.0).abs() < 1e-5 && first.facing.y.abs() < 1e-5);
        w.player.face = Vec2::new(0.0, 1.0);
        run_laser(&mut w, 1.0 / 60.0);
        let after = *w.ecs.resource::<LaserFx>();
        assert!(
            (after.facing.x - 1.0).abs() < 1e-5,
            "the burst followed the rat's turn"
        );
    }

    /// Every exposed body on the ray takes the pulse once, and the beam pierces
    /// the whole line rather than stopping at the first body.
    #[test]
    fn laser_pierces_every_exposed_body_once() {
        let mut w = World::new(Theme::Sewer, 13);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // Three bodies dead ahead inside the 34 px beam, plus two well off the
        // sweep (one beside, one behind the rat).
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let mid = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(18.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(26.0, 0.0), 1.0);
        let beside = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(10.0, 30.0), 1.0);
        let behind = w.spawn_enemy(EnemyKind::Cat, p - Vec2::new(18.0, 0.0), 1.0);
        let full = [near, mid, far, beside, behind].map(|e| enemy_hp(&w, e));

        let hit = Weapon {
            kind: WeaponKind::LaserEyes,
            level: 1,
            timer: 0.0,
            evolved: false,
        }
        .laser_hit_damage();
        let mut ticks = 0;
        while ticks < 60 {
            run_laser(&mut w, 1.0 / 60.0);
            ticks += 1;
            if !w.ecs.resource::<LaserFx>().active() {
                break;
            }
        }
        // One sweep: every body the beam crosses takes the same single hit.
        assert_eq!(enemy_hp(&w, near), full[0] - hit, "the near body");
        assert_eq!(
            enemy_hp(&w, mid),
            full[1] - hit,
            "the beam stopped at the near body"
        );
        assert_eq!(enemy_hp(&w, far), full[2] - hit, "the far body was shadowed");
        assert_eq!(enemy_hp(&w, beside), full[3], "a body beside the sweep was hit");
        assert_eq!(enemy_hp(&w, behind), full[4], "a body behind the rat was hit");
    }

    /// A wall stops the beam: nothing past the first face is damaged.
    #[test]
    fn laser_stops_at_a_wall() {
        let mut w = World::new(Theme::Sewer, 14);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 5); // 54 px reach
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // A solid column 16 px ahead of the rat, and a body behind it.
        let cx = (p.x / 16.0) as i32 + 1;
        let cy = (p.y / 16.0) as i32;
        let face = wall_column(&mut w, cx, cy);
        let behind_wall = w.spawn_enemy(EnemyKind::Brute, Vec2::new(face + 14.0, p.y), 1.0);
        let in_front = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(8.0, 0.0), 1.0);
        let behind_hp = enemy_hp(&w, behind_wall);
        let front_hp = enemy_hp(&w, in_front);
        for _ in 0..60 {
            if !w.ecs.resource::<LaserFx>().active() && w.ecs.resource::<LaserFx>().fired > 0 {
                break;
            }
            run_laser(&mut w, 1.0 / 60.0);
        }
        assert!(enemy_hp(&w, in_front) < front_hp, "the near body was shielded");
        assert_eq!(
            enemy_hp(&w, behind_wall),
            behind_hp,
            "the beam reached through the wall"
        );
    }

    /// The weapon never fires through the auto-fire pass: it charges itself and
    /// spawns no bullets at all.
    #[test]
    fn laser_is_not_fired_by_the_auto_fire_pass() {
        let mut w = World::new(Theme::Sewer, 15);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        assert!(w.bullets().is_empty(), "the laser spawned a projectile");
        assert!(w.ecs.resource::<LaserFx>().total == 0.0, "fired outside its system");
    }

    /// Nothing happens when the weapon is not owned: no beam, no state.
    #[test]
    fn laser_does_nothing_when_not_owned() {
        let mut w = World::new(Theme::Sewer, 16);
        w.clear_enemies();
        arm(&mut w, WeaponKind::Gnaw, 1);
        let e = w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(10.0, 0.0), 1.0);
        let hp = enemy_hp(&w, e);
        for _ in 0..30 {
            run_laser(&mut w, 1.0 / 60.0);
        }
        assert_eq!(enemy_hp(&w, e), hp);
        assert!(!w.ecs.resource::<LaserFx>().active());
        assert!(w.laser_visual().is_none());
    }

    /// The bar is **steady** now: a sweep is one continuous beam, so the visual
    /// is lit the whole way through and only fades at the very end.
    #[test]
    fn the_laser_bar_is_steady_through_the_sweep() {
        let mut w = World::new(Theme::Sewer, 17);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        run_laser(&mut w, 1.0 / 60.0);
        let on = w.laser_visual().expect("a beam on the trigger tick");
        assert!(on.brightness > 0.9, "the bar should be lit: {on:?}");
        assert!(on.muzzle > 0.0);
        // 0.2 s into a 0.55 s sweep it is exactly as lit.
        for _ in 0..12 {
            run_laser(&mut w, 1.0 / 60.0);
        }
        let mid = w.laser_visual().expect("the sweep is still running");
        assert!(
            mid.brightness > 0.9,
            "a continuous beam does not blink: {mid:?}"
        );
        // …and the angle has moved a long way from where it started.
        assert!((mid.angle - on.angle).abs() > 0.15, "the beam did not sweep");
    }

    /// Super Dash: only the travelled segment damages, once per body per dash.
    #[test]
    fn super_dash_hits_only_what_the_dash_crossed() {
        let mut w = World::new(Theme::Sewer, 21);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SuperDash, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 0.0;
        let p = w.player.pos;
        let crossed = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let aside = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 40.0), 1.0);
        let hp_crossed = enemy_hp(&w, crossed);
        let hp_aside = enemy_hp(&w, aside);

        // First dash tick: the press and the movement happen together.
        w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), true);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(super_dash_system).unwrap();
        w.end_tick();
        assert!(w.player.dash_active > 0.0, "the dash did not start");
        let after_first = enemy_hp(&w, crossed);
        assert!(
            after_first < hp_crossed,
            "the crossed body took no dash damage"
        );
        assert_eq!(enemy_hp(&w, aside), hp_aside, "an un-crossed body was hit");

        // The rest of the dash: the crossed body never takes a second hit.
        for _ in 0..12 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
        }
        assert_eq!(
            enemy_hp(&w, crossed),
            after_first,
            "the same body was hit twice by one dash"
        );
        assert!(w.player.dash_active == 0.0, "the dash should be over");
    }

    /// A dash on cooldown (a blocked press) buys no hit, and neither does the
    /// ordinary movement that follows it.
    #[test]
    fn super_dash_buys_nothing_on_a_failed_dash() {
        let mut w = World::new(Theme::Sewer, 22);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SuperDash, 5);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 5.0; // cooling: the press must do nothing
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(4.0, 0.0), 1.0);
        let hp = enemy_hp(&w, body);
        for _ in 0..12 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), true);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
        }
        assert_eq!(w.player.dash_active, 0.0, "the dash should not have started");
        assert_eq!(enemy_hp(&w, body), hp, "a cooling dash dealt damage");
    }

    /// Clean Socks ready the dash; they are not a dash. Collecting them with
    /// Super Dash owned, then walking through a body without pressing anything,
    /// deals nothing and opens no dash segment.
    #[test]
    fn super_dash_does_not_fire_when_clean_socks_ready_the_dash() {
        use crate::game::ecs::resources::DashTrack;
        use crate::game::ecs::systems::loot;
        use crate::game::gem::{Pickup, PickupKind};
        let mut w = World::new(Theme::Sewer, 24);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SuperDash, 5);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 1.5;
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(4.0, 0.0), 1.0);
        let hp = enemy_hp(&w, body);
        w.spawn_pickup(Pickup {
            pos: p,
            kind: PickupKind::CleanSocks,
            bob: 0.0,
        });
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(loot::pickups_system).unwrap();
        w.end_tick();
        assert_eq!(w.player.dash_cd, 0.0, "setup: the socks must ready the dash");
        for _ in 0..12 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
            assert!(
                w.ecs.resource::<DashTrack>().segment.is_none(),
                "readying the dash opened a dash segment"
            );
        }
        assert_eq!(w.player.dash_active, 0.0, "the socks must not start a dash");
        assert_eq!(enemy_hp(&w, body), hp, "Clean Socks bought a Super Dash hit");
    }

    /// No Super Dash owned, no damage — even through a body.
    #[test]
    fn super_dash_does_nothing_when_not_owned() {
        let mut w = World::new(Theme::Sewer, 23);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::Gnaw, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(8.0, 0.0), 1.0);
        let hp = enemy_hp(&w, body);
        let mut ran = false;
        for _ in 0..10 {
            w.player.dash_cd = 0.0;
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), true);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
            ran |= w.player.dash_active > 0.0;
        }
        assert!(ran, "the dash itself must still work");
        assert_eq!(enemy_hp(&w, body), hp, "an unowned Super Dash dealt damage");
    }

    /// Two runs with the same seed and the same inputs land the same numbers:
    /// the burst and the dash draw rng only through their emitters, in order.
    #[test]
    fn twin_runs_of_the_new_weapons_match() {
        let play = |seed: u64| {
            let mut w = World::new(Theme::Sewer, seed);
            w.clear_enemies();
            w.clear_bullets();
            open_map(&mut w);
            w.player.loadout.weapons = vec![
                Weapon {
                    kind: WeaponKind::LaserEyes,
                    level: 3,
                    timer: 0.0,
                    evolved: false,
                },
                Weapon {
                    kind: WeaponKind::SuperDash,
                    level: 3,
                    timer: 0.0,
                    evolved: false,
                },
            ];
            let p = w.player.pos;
            let mut enemies = Vec::new();
            for i in 0..12 {
                let a = i as f32 * 0.5;
                let at = p + Vec2::from_angle(a) * (8.0 + i as f32 * 3.0);
                enemies.push(w.spawn_enemy(EnemyKind::Cat, at, 4.0));
            }
            for tick in 0..240 {
                let move_dir = if tick < 30 {
                    Vec2::new(1.0, 0.0)
                } else {
                    Vec2::new(0.0, 1.0)
                };
                let dash = tick % 40 == 0;
                w.player.dash_cd = w.player.dash_cd.min(0.0);
                w.begin_tick(1.0 / 60.0, move_dir, dash);
                w.ecs
                    .run_system_cached(crate::game::ecs::systems::player::player_system)
                    .unwrap();
                w.ecs.run_system_cached(laser_eyes_system).unwrap();
                w.ecs.run_system_cached(super_dash_system).unwrap();
                w.end_tick();
            }
            let hps: Vec<u32> = enemies
                .iter()
                .map(|&e| (enemy_hp(&w, e) * 100.0) as u32)
                .collect();
            (hps, w.player.pos.x.to_bits(), w.player.pos.y.to_bits())
        };
        assert_eq!(play(900), play(900));
    }

    /// Every wave-3 passive that claims a weapon actually reaches it: the
    /// audit the design asks for (all eight pre-existing weapons plus the two
    /// new ones).
    #[test]
    fn passive_hooks_reach_every_weapon_they_claim() {
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::game::ecs::systems::weapons::active_weapons_system;

        // Lens Cap x4 (+32% ranged / +16% melee): Gnaw's bite (melee reach), a
        // Tail Whip's swipe, a Spore Orbit ring's radius, and the laser's beam
        // all grow by the hook's multiplier; without the passive they are the
        // plain numbers. Gnaw: a body just past the plain jaw is only bitten
        // with the passive.
        for lens in [false, true] {
            let mut w = World::new(Theme::Sewer, 41);
            w.clear_enemies();
            w.clear_bullets();
            open_map(&mut w);
            arm(&mut w, WeaponKind::Gnaw, 1);
            if lens {
                for _ in 0..PassiveKind::LensCap.max_stacks() {
                    w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LensCap));
                }
            }
            let plain = crate::game::weapon::GNAW_BITE_REACH[0];
            let reach = w.player.loadout.stats.melee_reach(plain);
            assert_eq!(lens, reach > plain, "Lens Cap must change the reach");
            // A small body whose near edge is 1.5 px past the plain jaw.
            let cat = w.spawn_enemy(EnemyKind::Skitterer, w.player.pos, 9.0);
            let r = w.ecs.get::<Enemy>(cat).unwrap().radius;
            let at = w.player.pos + Vec2::new(plain + r + 1.5, 0.0);
            w.ecs.get_mut::<Enemy>(cat).unwrap().pos = at;
            let hp = w.ecs.get::<Enemy>(cat).unwrap().hp;
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            assert!(w.bullets().is_empty(), "a bite fires no projectile");
            let bitten = w.ecs.get::<Enemy>(cat).unwrap().hp < hp;
            assert_eq!(bitten, lens, "Gnaw reach with Lens Cap = {lens}");
        }

        // Tail Whip's radius (melee reach) and Counterweight's shove.
        for (counter, lens) in [(false, false), (true, true)] {
            let mut w = World::new(Theme::Sewer, 42);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::TailWhip, 1);
            if counter {
                for _ in 0..PassiveKind::Counterweight.max_stacks() {
                    w.player
                        .loadout
                        .apply(&Upgrade::Passive(PassiveKind::Counterweight));
                }
            }
            for _ in 0..PassiveKind::LensCap.max_stacks() {
                if lens {
                    w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LensCap));
                }
            }
            let p = w.player.pos;
            // The swipe's plain reach plus the body's own radius: the rat faces
            // +x, so both bodies stand in the front half-circle.
            w.player.face = Vec2::new(1.0, 0.0);
            let cat_r = {
                let probe = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(900.0, 0.0), 9.0);
                w.ecs.get::<Enemy>(probe).unwrap().radius
            };
            let plain_radius = crate::game::weapon::WHIP_REACH[0] + cat_r;
            // Just past the plain swipe, inside the Lens Cap one; and one well
            // inside both.
            let edge = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(plain_radius + 1.5, 0.0), 9.0);
            let inside = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(plain_radius - 4.0, 0.0), 9.0);
            let edge_hp = enemy_hp(&w, edge);
            let inside_hp = enemy_hp(&w, inside);
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            assert!(enemy_hp(&w, inside) < inside_hp, "the plain aura missed");
            if lens {
                assert!(
                    enemy_hp(&w, edge) < edge_hp,
                    "Lens Cap did not widen the whip"
                );
            } else {
                assert_eq!(enemy_hp(&w, edge), edge_hp, "the plain aura reached too far");
            }
        }

        // Long Fuse x4: the acid pool's splash radius and life.
        for fuse in [false, true] {
            let mut w = World::new(Theme::Sewer, 43);
            w.clear_enemies();
            w.clear_bullets();
            open_map(&mut w);
            arm(&mut w, WeaponKind::AcidSpit, 1);
            if fuse {
                for _ in 0..PassiveKind::LongFuse.max_stacks() {
                    w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LongFuse));
                }
            }
            let lob_radius = w.player.loadout.stats.splash_radius(12.0 + 1.5);
            let pool_life = w.player.loadout.stats.zone_life(2.6);
            w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(10.0, 0.0), 9.0);
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            let b = w.bullets();
            assert_eq!(b.len(), 1);
            let lob = b[0].lob.expect("acid lob");
            assert!((lob.radius - lob_radius).abs() < 1e-4, "pool radius");
            assert!((lob.life - pool_life).abs() < 1e-4, "pool life");
            if fuse {
                assert!((lob_radius - 13.5 * 1.16).abs() < 1e-4);
                assert!((pool_life - 2.6 * 1.48).abs() < 1e-4);
            }
        }

        // Long Fuse on an orbiter (Spore Orbit's ring lifetime), and
        // Counterweight on the Sonic Squeak's ring shove.
        let mut w = World::new(Theme::Sewer, 44);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SporeOrbit, 1);
        for _ in 0..PassiveKind::LongFuse.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LongFuse));
        }
        let expect_life = w.player.loadout.stats.zone_life(WeaponKind::SporeOrbit.base_cooldown());
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        for b in w.bullets() {
            assert!((b.life - expect_life).abs() < 1e-4, "orbiter life {}", b.life);
        }

        // Sonic Squeak: Long Fuse widens the ring, Counterweight pushes harder.
        let mut w = World::new(Theme::Sewer, 45);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SonicSqueak, 1);
        for _ in 0..PassiveKind::LongFuse.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LongFuse));
        }
        for _ in 0..PassiveKind::Counterweight.max_stacks() {
            w.player
                .loadout
                .apply(&Upgrade::Passive(PassiveKind::Counterweight));
        }
        w.player.loadout.weapons[0].timer = 0.0;
        let p = w.player.pos;
        // The squeak is a cone along the facing: the body stands in it.
        w.player.face = Vec2::new(1.0, 0.0);
        let body = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(20.0, 0.0), 40.0);
        let ring = w.player.loadout.stats.splash_radius(90.0);
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
        w.ecs.run_system_cached(active_weapons_system).unwrap();
        w.end_tick();
        assert!((w.active_fx().squeak_max_r - ring).abs() < 1e-4, "ring reach");
        // The cone lands on the press tick.
        let shoved = w.ecs.get::<Enemy>(body).unwrap().vel.len();
        for _ in 0..40 {
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(active_weapons_system).unwrap();
            w.end_tick();
        }
        let vel = w.ecs.get::<Enemy>(body).unwrap().vel.len().max(shoved);
        assert!(vel > 1.0, "the cone did not shove the body");
    }

    /// Second Scent: gem XP is scaled and the fraction is banked across
    /// collections, so four stacks are worth exactly +40% over a run (a `+10%`
    /// that truncated every 1-XP gem would be worth nothing at all).
    #[test]
    fn second_scent_scales_gem_xp_and_banks_the_fraction() {
        use crate::game::ecs::resources::XpBank;
        use crate::game::gem::Gem;
        use crate::game::loadout::{PassiveKind, Upgrade};

        // The bank itself: 1 XP at x1.4 pays 1, 1, 2, 1, 2 (the fraction
        // carries: 0.4, 0.8, then 1.2 pays the extra point and banks 0.2, …),
        // so five 1-XP gems pay 7 and nothing is lost to truncation.
        let mut bank = XpBank::default();
        let paid: Vec<u32> = (0..5).map(|_| bank.grant(1, 1.4)).collect();
        assert_eq!(paid, vec![1, 1, 2, 1, 2], "the bank must not lose XP");
        assert_eq!(paid.iter().sum::<u32>(), 7, "five 1-XP gems at +40%");
        // Identity: no stacks, no change, no bank.
        let mut bank = XpBank::default();
        assert_eq!(bank.grant(3, 1.0), 3);
        assert_eq!(bank.0, 0.0);

        // …and through the real gem pass.
        let collect = |stacks: u32| {
            let mut w = World::new(Theme::Sewer, 51);
            w.clear_enemies();
            open_map(&mut w);
            // No level-ups in the window: this measures XP collected, not the
            // XP curve.
            w.player.xp_to_next = 10_000;
            for _ in 0..stacks {
                w.player
                    .loadout
                    .apply(&Upgrade::Passive(PassiveKind::SecondScent));
            }
            // Ten 1-XP gems, one per tick, all on the rat.
            for _ in 0..10 {
                let p = w.player.pos;
                w.spawn_gem(Gem::new(p, 1));
                w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
                w.ecs
                    .run_system_cached(crate::game::ecs::systems::loot::gems_system)
                    .unwrap();
                w.end_tick();
            }
            w.player.xp
        };
        assert_eq!(collect(0), 10);
        assert_eq!(collect(4), 14, "+40% of ten 1-XP gems");
    }

    /// The melee-nudge log holds the 0.4 s rule: a body is ready again only
    /// after the cooldown has run.
    #[test]
    fn melee_nudge_cooldowns_expire() {
        let mut nudge = MeleeNudge::default();
        assert!(nudge.ready(7));
        nudge.mark(7, MELEE_NUDGE_COOLDOWN);
        assert!(!nudge.ready(7), "the body should be on cooldown");
        assert!(nudge.ready(8), "another body is unaffected");
        nudge.tick(0.39);
        assert!(!nudge.ready(7));
        nudge.tick(0.02);
        assert!(nudge.ready(7), "the cooldown did not expire");
        assert_eq!(nudge.len(), 0);
    }

    /// A body is nudgeable once per 0.4 s from the Tail Whip, whatever the
    /// cooldown maths says — and not at all without the passive.
    #[test]
    fn counterweight_nudges_a_melee_target_once_per_cooldown() {
        let mut w = World::new(Theme::Sewer, 31);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        arm(&mut w, WeaponKind::TailWhip, 1);
        w.player
            .loadout
            .apply(&crate::game::loadout::Upgrade::Passive(
                crate::game::loadout::PassiveKind::Counterweight,
            ));
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 20.0);
        let start = w.ecs.get::<Enemy>(body).unwrap().pos;
        // Fire the whip twice inside one 0.4 s window: only the first nudges.
        for _ in 0..2 {
            w.player.loadout.weapons[0].timer = 0.0;
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.ecs.run_system_cached(melee_nudge_system).unwrap();
            w.end_tick();
        }
        let nudged = w.ecs.get::<Enemy>(body).unwrap().pos;
        let pushed = (nudged - start).len();
        assert!(pushed > 1.5, "the body was not nudged: {pushed}");
        assert!(pushed < 4.0, "the nudge stacked twice: {pushed}");
        assert_eq!(w.ecs.resource::<MeleeNudge>().len(), 1);
    }

    /// Run a Super Dash started with `charge` (`0` = a tap) through the player
    /// and weapon systems only, so nothing but the dash moves.
    fn run_charged_dash(w: &mut World, charge: f32) {
        w.begin_tick_charged(
            1.0 / 60.0,
            Vec2::new(1.0, 0.0),
            true,
            false,
            None,
            charge,
        );
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(super_dash_system).unwrap();
        w.end_tick();
    }

    fn tick_dash(w: &mut World) {
        w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(super_dash_system).unwrap();
        w.end_tick();
    }

    /// A charged release hits harder than the same level's tap, and Ripcord
    /// keeps the longer route the charged dash actually travelled.
    #[test]
    fn a_charged_dash_hits_harder_and_ripcord_keeps_the_longer_route() {
        let run = |charge: f32| -> (f32, f32) {
            let mut w = World::new(Theme::Sewer, 33);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::SuperDash, 1);
            w.player.loadout.weapons[0].evolved = true;
            w.player.face = Vec2::new(1.0, 0.0);
            w.player.vel = Vec2::new(58.0, 0.0);
            w.player.dash_cd = 0.0;
            let p = w.player.pos;
            let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
            let hp0 = enemy_hp(&w, body);
            run_charged_dash(&mut w, charge);
            for _ in 0..40 {
                tick_dash(&mut w);
            }
            let taken = hp0 - enemy_hp(&w, body);
            let route: f32 = w
                .ecs
                .resource::<DashTrack>()
                .route()
                .iter()
                .map(|&(a, b)| a.dist(b))
                .sum();
            (taken, route)
        };
        let (tap_dmg, tap_route) = run(0.0);
        let (full_dmg, full_route) = run(1.0);
        assert!(tap_dmg > 0.0, "the tap dash must still wound");
        assert!(
            full_dmg > tap_dmg * (1.0 + SUPER_DASH_DAMAGE_PER_CHARGE * 0.75),
            "a full charge hit for {full_dmg}, a tap for {tap_dmg}"
        );
        assert!(
            full_route > tap_route * 1.3,
            "the cord must keep the longer route: {full_route} vs {tap_route}"
        );
    }

    /// A charged dash sweeps a wider corridor: a body beside the lane that a tap
    /// clears is caught by the full charge.
    #[test]
    fn a_charged_dash_sweeps_a_wider_corridor() {
        let hit_beside = |charge: f32| -> bool {
            let mut w = World::new(Theme::Sewer, 35);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::SuperDash, 1);
            w.player.face = Vec2::new(1.0, 0.0);
            w.player.vel = Vec2::new(58.0, 0.0);
            w.player.dash_cd = 0.0;
            let p = w.player.pos;
            // A Skitterer (radius 2.5) 7.5 px beside a lane 10 px ahead: inside
            // the tap's 4 px sweep + 2.5 px body (6.5) but inside a full
            // charge's 6 px sweep (8.5).
            let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(10.0, 7.5), 4.0);
            let hp0 = enemy_hp(&w, body);
            run_charged_dash(&mut w, charge);
            for _ in 0..40 {
                tick_dash(&mut w);
            }
            enemy_hp(&w, body) < hp0
        };
        assert!(!hit_beside(0.0), "a tap dash is the narrow sweep");
        assert!(hit_beside(1.0), "a full charge must sweep wider");
    }
}
