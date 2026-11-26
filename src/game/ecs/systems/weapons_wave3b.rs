//! Wave 3's self-driving weapons: **Laser Eyes**, **Super Dash**, and the
//! bookkeeping **Counterweight**'s melee nudge needs.
//!
//! `weapons::fire_weapons_system` skips both kinds
//! (`WeaponKind::fires_outside_fire_one`), because neither is a "spawn a
//! projectile now" weapon:
//!
//! - **Laser Eyes** is an ordinary auto-firing weapon whose shot is a **sweep**:
//!   it charges its own `Weapon::timer` here, and on the trigger tick it locks
//!   the aim direction and sweeps **two** continuous beams **inward**: one from
//!   each outer edge of the arc (`aim ± half`) closing on the aim line over
//!   `Weapon::laser_sweep_secs` (L1 60° / 0.26 s, L5 110° / 0.34 s). From L4
//!   (`Weapon::laser_twin`) they do not stop where they meet: they cross and
//!   carry on to the opposite edges over the same sweep (an X-shaped scissor),
//!   so the pair covers the whole arc at twice the angular speed. A body takes
//!   one distance-scaled pulse per sweep (both beams share one hit set). Each
//!   beam starts at the rat's current eyes and stops at the first wall or the
//!   edge of the widest simulation view. Carbon Copy keeps the cast origin.
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
//! when something actually happened. Only traces that fire Laser Eyes can see
//! the moving origin, which can change which bodies are hit and their damage.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::{segment_circle_hit, TileMap};
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::resources::{
    DamageLedger, DashTrack, EnemyIndex, LaserFx, MeleeNudge, RngRes, SfxQueue, Shake, TickInput,
    Viewport,
};
use crate::game::ecs::systems::approach::sim_view_ray_exit;
use crate::game::ecs::systems::boss::Boss;
use crate::game::ecs::systems::spawn::widest_view;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::{WeaponKind, LASER_EYE_OFFSET, LASER_HALF_WIDTH};
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

/// Keep adjacent sweep samples at most 3 px apart at the farthest end of the
/// view, and keep at least the old angular resolution in very small views.
const SWEEP_STEP_PX: f32 = 3.0;
const MAX_SWEEP_STEP_RAD: f32 = 0.035;

/// The numbers one Laser Eyes sweep was cast with, snapshotted at the trigger
/// tick so a level-up card opened mid-sweep cannot change a hit already paid
/// for (the same rule every projectile and lash follows).
#[derive(Clone, Copy, Debug)]
struct SweepCast {
    /// The old pulse count, used by the afterimage's sparks.
    pulses: u32,
    /// Radius before damage falls off.
    full_damage_radius: f32,
    /// Damage per hit before falloff.
    dmg: f32,
    /// Fired by the evolved (**Carbon Copy**) weapon.
    evolved: bool,
    /// Half the arc, in radians.
    half: f32,
    /// Seconds the sweep lasts.
    secs: f32,
    /// Do the two beams cross the aim line and run on to the opposite edges
    /// (L4+)? Otherwise they stop where they meet, on the aim line.
    twin: bool,
}

/// Both beams' angles at progress `t` (0..1).
///
/// The pair starts at the two outer edges of the arc — `base - half` and
/// `base + half` — and closes **inward**: at L1–L3 each beam travels `half` and
/// both land on the aim line as the sweep ends; at L4+ (`twin`) each travels the
/// whole arc (`2 * half`), so they cross on the aim line at the halfway mark and
/// finish on the opposite edges. There is no alternating direction any more:
/// every sweep is the same symmetrical pair.
#[inline]
fn sweep_pair(base: f32, half: f32, span: f32, t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    (base - half + span * t, base + half - span * t)
}

fn beam_length(tiles: &TileMap, origin: Vec2, dir: Vec2, center: Vec2, view: Vec2) -> f32 {
    let edge = sim_view_ray_exit(origin, dir, center, view, 0.0);
    let stop = tiles.segment_hit(origin, origin + dir * edge, LASER_HALF_WIDTH);
    edge * stop.unwrap_or(1.0).clamp(0.0, 1.0)
}

fn laser_damage(pulse: f32, full_damage_radius: f32, distance_sq: f32) -> f32 {
    let radius_sq = full_damage_radius * full_damage_radius;
    pulse * (radius_sq / distance_sq.max(radius_sq))
}

/// Resolve one beam's angular **step**: every body the segment crosses at any
/// sample angle between `from_angle` and `to_angle`, once per sweep.
///
/// The sweep keeps its own hit set, so a body the beam passes over is cut
/// exactly once however many ticks the crossing takes, and both beams (they are
/// always a pair) share that set — so the pair together deals one sweep's
/// damage. Walls stop each sample ray where they stop the drawn beam, so a body
/// past the first face is never touched.
///
/// Returns the wall-stopped beam length at `to_angle` (the art draws the live
/// beam there).
#[allow(clippy::too_many_arguments)]
fn sweep_step(
    origin: Vec2,
    from_angle: f32,
    to_angle: f32,
    full_damage_radius: f32,
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
    view_center: Vec2,
    view: Vec2,
) -> f32 {
    let kind = WeaponKind::LaserEyes;
    let delta = to_angle - from_angle;
    let farthest = view.len() * 0.5 + view_center.dist(origin);
    let step_rad = (SWEEP_STEP_PX / farthest).min(MAX_SWEEP_STEP_RAD);
    let steps = (delta.abs() / step_rad).ceil().max(1.0) as u32;
    let mut last_len = 0.0;
    for step in 0..=steps {
        // Resolve the exact angle the art will use for the final sample.
        let angle = if step == steps {
            to_angle
        } else {
            from_angle + delta * (step as f32 / steps as f32)
        };
        let dir = Vec2::from_angle(angle);
        let len = beam_length(&level.tiles, origin, dir, view_center, view);
        last_len = len;
        if len <= 0.0 {
            continue;
        }

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
            let amount = laser_damage(dmg, full_damage_radius, origin.dist_sq(e.pos));
            let applied = e.hurt_from(amount, origin);
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
            if applied >= 1.0 {
                damage_numbers.record(
                    enemy_id, pos, enemy_radius, applied, killed, DamageSource::Normal,
                );
            }
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
                origin + dir * len,
                LASER_HALF_WIDTH,
            ) {
                continue;
            }
            let amount = laser_damage(dmg, full_damage_radius, origin.dist_sq(boss.pos))
                * boss.damage_mult;
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
            if amount >= 1.0 {
                damage_numbers.record(
                    boss_id, boss_pos, boss_radius, amount, killed, DamageSource::Normal,
                );
            }
        }
    }
    last_len
}

/// Laser Eyes: charge the weapon and sweep two continuous beams across an arc.
///
/// A sweep is the weapon's shot, so it fires on its own cycle without needing a
/// target (like Squeak Nova, unlike Gnaw): a beam across a corridor is worth
/// firing before the horde is on screen. The aim locks to the facing or reticle
/// at the trigger tick. The eye position follows the rat's current facing and
/// position throughout the sweep.
///
/// Batch 13 replaced batch 12's one beam (which alternated direction) with a
/// **symmetrical pair**: one beam starts at each outer edge of the arc and both
/// close inward over `laser_sweep_secs`, meeting on the aim line as the sweep
/// ends. From L4 they do not stop there — they cross and carry on to the
/// opposite edges over the same sweep, so the pair moves twice as fast. Every
/// body the pair crosses takes one pulse once (one hit set shared by both
/// beams), scaled by inverse-square falloff beyond the rank's full-damage
/// radius.
///
/// Walls and the widest simulation view ([`Viewport`] × max zoom-out) cap
/// the reach. Live beams use the rat's current view; Carbon Copy uses the
/// recorded origin. The simulation supplies the same stopped lengths to hit
/// tests, beam art, lights and water.
#[allow(clippy::too_many_arguments)]
pub fn laser_eyes_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    viewport: Res<Viewport>,
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
    if player.buffs.weapons_suspended() {
        *fx = LaserFx::default();
        return;
    }
    let dt = input.dt;
    let (dmg_mult, fire_rate) = (player.damage_mult(), player.loadout.stats.fire_rate_mult.max(0.1));

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
        trigger = Some(SweepCast {
            pulses: w.laser_pulses().max(1),
            full_damage_radius: w.laser_full_damage_radius(),
            dmg: w.laser_hit_damage() * dmg_mult,
            evolved: w.evolved,
            half: w.laser_half_arc(),
            secs: w.laser_sweep_secs(),
            twin: w.laser_twin(),
        });
    }
    if trigger.is_none() && !fx.active() {
        return;
    }

    let face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let eye = player.pos + face * LASER_EYE_OFFSET;
    // Near a wall the decorative eyes can extend past the collision radius.
    // Keep the muzzle on the rat's side of that wall on every tick.
    let origin = match level.tiles.segment_hit(player.pos, eye, LASER_HALF_WIDTH) {
        Some(t) => player.pos + face * (LASER_EYE_OFFSET * t - 0.01).max(0.0),
        None => eye,
    };

    if let Some(cast) = trigger {
        // An aim point turns the sweep's centre to the reticle instead of the
        // facing (wave 3 controls); with no aim this is exactly the locked
        // `face`.
        let aimed = input
            .aim
            .map(|p| p - player.pos)
            .filter(|v| v.len_sq() > 4.0)
            .map(|v| v.normalized());
        let base = aimed.unwrap_or(face);
        let base_angle = base.angle();
        // Both beams start at the two outer edges of the arc at the same
        // instant and close inward, so there is no direction to alternate any
        // more: every sweep is the same symmetrical pair.
        let span = if cast.twin {
            2.0 * cast.half
        } else {
            cast.half
        };
        let (angle, angle_b) = sweep_pair(base_angle, cast.half, span, 0.0);
        // Carbon Copy: the sweep recorded at the end of the *previous* burst
        // replays now from its own origin and facing — the same inward-closing
        // pair, at half the recorded hit. One copy is kept, and a copy is never
        // itself recorded (only a real sweep is).
        let recorded = if cast.evolved { fx.copy } else { None };
        let (copy_left, copy_prev, copy_prev_b) = match recorded {
            Some(copy) => {
                let (a, b) = sweep_pair(copy.facing.angle(), cast.half, span, 0.0);
                (cast.secs, a, b)
            }
            None => (0.0, 0.0, 0.0),
        };
        *fx = LaserFx {
            left: cast.secs,
            age: 0.0,
            pulses: cast.pulses,
            facing: base,
            full_damage_radius: cast.full_damage_radius,
            dmg: cast.dmg,
            half: cast.half,
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
            copy_left,
            copy_age: 0.0,
            copy_fired: 0,
            copy_half: cast.half,
            copy_span: span,
            copy_secs: copy_left,
            copy_prev_angle: copy_prev,
            copy_prev_angle_b: copy_prev_b,
            cast_origin: origin,
            ..LaserFx::default()
        };
        shake.amount = shake.amount.max(0.9);
        vfx::emit_laser_muzzle(&mut particles, &mut rng.0, player.pos, face);
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

    // 2. Sweep the two live beams inward from this tick's eye position.
    fx.origin = origin;
    fx.age += dt;
    let t = (fx.age / fx.total.max(1e-4)).clamp(0.0, 1.0);
    let (half, span, full_damage_radius, dmg) = (fx.half, fx.span(), fx.full_damage_radius, fx.dmg);
    let view = widest_view(viewport.0);
    // Match the arena clamp of the widest ordinary view without reading the
    // render camera.
    let half_view = Vec2::new(view.x.min(level.arena.x), view.y.min(level.arena.y)) * 0.5;
    let center = Vec2::new(
        player.pos.x.clamp(half_view.x, level.arena.x - half_view.x),
        player.pos.y.clamp(half_view.y, level.arena.y - half_view.y),
    );
    let (angle, angle_b) = sweep_pair(fx.facing.angle(), half, span, t);
    let (prev, prev_b) = (fx.prev_angle, fx.prev_angle_b);
    {
        let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
        let mut hits = fx.hits;
        let mut boss_hit = fx.boss_hit;
        let len = sweep_step(
            origin,
            prev,
            angle,
            full_damage_radius,
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
            center,
            view,
        );
        // The mirrored beam shares the same hit set, so the pair still cuts a
        // body once per sweep.
        let len_b = sweep_step(
            origin,
            prev_b,
            angle_b,
            full_damage_radius,
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
            center,
            view,
        );
        fx.hits = hits;
        fx.boss_hit = boss_hit;
        fx.prev_angle = angle;
        fx.prev_angle_b = angle_b;
        fx.angle = angle;
        fx.beam = len;
        fx.angle_b = angle_b;
        fx.beam_b = len_b;
        fx.fired = fx.hits.len();
    }

    // 3. The Carbon Copy afterimage: the recorded sweep's own inward-closing
    // pair, from the recorded origin and facing, on the same clock.
    if fx.copy_left > 0.0 {
        if let Some(copy) = fx.copy {
            fx.copy_age += dt;
            let ct = (fx.copy_age / fx.copy_secs.max(1e-4)).clamp(0.0, 1.0);
            let (c_angle, c_angle_b) =
                sweep_pair(copy.facing.angle(), fx.copy_half, fx.copy_span, ct);
            let (c_prev, c_prev_b) = (fx.copy_prev_angle, fx.copy_prev_angle_b);
            {
                let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
                let mut hits = fx.copy_hits;
                let mut boss_hit = fx.copy_boss_hit;
                let ghost_dmg = copy.dmg
                    * crate::game::ecs::systems::evolutions::CARBON_COPY_DAMAGE;
                let len = sweep_step(
                    copy.origin,
                    c_prev,
                    c_angle,
                    copy.full_damage_radius,
                    ghost_dmg,
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
                    copy.origin,
                    view,
                );
                let len_b = sweep_step(
                    copy.origin,
                    c_prev_b,
                    c_angle_b,
                    copy.full_damage_radius,
                    ghost_dmg,
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
                    copy.origin,
                    view,
                );
                fx.copy_hits = hits;
                fx.copy_boss_hit = boss_hit;
                fx.copy_prev_angle = c_angle;
                fx.copy_prev_angle_b = c_angle_b;
                fx.copy_beams = [len, len_b];
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
        // (Carbon Copy keeps exactly one); everything else starts clean.
        let recorded = if fx.evolved { fx.copy_of_this_sweep() } else { None };
        *fx = LaserFx {
            copy: recorded,
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
    if player.buffs.weapons_suspended() {
        return;
    }
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
    // and hits harder. A tap dash (`charge = 0`) uses the base radius and
    // damage along its travelled path.
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

    /// One sweep is a **symmetrical pair**: a beam starts at each outer edge of
    /// the arc at the same instant and both close inward, landing on the aim
    /// line together when the sweep ends (L1–L3).
    #[test]
    fn both_beams_start_at_the_edges_and_meet_on_the_aim_line() {
        for level in [1u8, 2, 3] {
            let mut w = World::new(Theme::Sewer, 10);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::LaserEyes, level);
            w.player.face = Vec2::new(1.0, 0.0);
            let half = {
                let mut l = Weapon::new(WeaponKind::LaserEyes);
                l.level = level;
                l.laser_half_arc()
            };
            let base = 0.0; // facing +x
            run_laser(&mut w, 1.0 / 60.0);
            let first = *w.ecs.resource::<LaserFx>();
            assert!(first.active());
            assert!(!first.twin, "L{level} must not cross the aim line");
            assert!((first.span() - half).abs() < 1e-5, "L{level} each beam travels half");
            // (Read after the trigger tick, so each beam is one step in.)
            assert!(
                (first.angle - (base - half)).abs() < 0.08,
                "L{level}: the first beam starts at the minus edge: {}",
                first.angle
            );
            assert!(
                (first.angle_b - (base + half)).abs() < 0.08,
                "L{level}: the second beam starts at the plus edge: {}",
                first.angle_b
            );
            // Run it out: the last frame before the reset has both beams on the
            // aim line, having closed the whole way in.
            let mut last = first;
            let mut ticks = 0;
            while w.ecs.resource::<LaserFx>().active() && ticks < 200 {
                last = *w.ecs.resource::<LaserFx>();
                run_laser(&mut w, 1.0 / 60.0);
                ticks += 1;
            }
            assert!(!w.ecs.resource::<LaserFx>().active(), "the sweep never ended");
            assert!(
                (last.angle - base).abs() < 0.08,
                "L{level}: the beams must meet on the aim line: {}",
                last.angle
            );
            assert!(
                (last.angle_b - base).abs() < 0.08,
                "L{level}: the second beam must land on the aim line: {}",
                last.angle_b
            );
            // …and the next sweep is the same pair, not the reverse: there is no
            // direction to alternate any more.
            w.player.loadout.weapons[0].timer = 0.0;
            run_laser(&mut w, 1.0 / 60.0);
            let second = *w.ecs.resource::<LaserFx>();
            assert!(
                (second.angle - (base - half)).abs() < 0.08
                    && (second.angle_b - (base + half)).abs() < 0.08,
                "L{level}: every sweep is the same inward pair"
            );
        }
    }

    /// From L4 the pair does not stop on the aim line: it crosses there and
    /// carries on to the opposite edges over the same sweep — the X — so each
    /// beam covers the whole arc and moves twice as fast.
    #[test]
    fn from_level_four_the_beams_cross_and_end_on_the_opposite_edges() {
        for level in [4u8, 5] {
            let mut w = World::new(Theme::Sewer, 19);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::LaserEyes, level);
            w.player.face = Vec2::new(1.0, 0.0);
            let half = {
                let mut l = Weapon::new(WeaponKind::LaserEyes);
                l.level = level;
                l.laser_half_arc()
            };
            let base = 0.0;
            run_laser(&mut w, 1.0 / 60.0);
            let first = *w.ecs.resource::<LaserFx>();
            assert!(first.twin, "L{level} must cross the aim line");
            assert!(
                (first.span() - 2.0 * half).abs() < 1e-5,
                "L{level} each beam travels the whole arc"
            );
            // Track the closest the pair comes to the aim line (it has to pass
            // through it) and the last frame before the reset.
            let mut last = first;
            let mut closest = f32::MAX;
            let mut ticks = 0;
            while w.ecs.resource::<LaserFx>().active() && ticks < 200 {
                let fx = *w.ecs.resource::<LaserFx>();
                if fx.angle.abs() < closest {
                    closest = fx.angle.abs();
                }
                if fx.angle_b.abs() < closest {
                    closest = fx.angle_b.abs();
                }
                last = fx;
                run_laser(&mut w, 1.0 / 60.0);
                ticks += 1;
            }
            assert!(
                closest < 3.0f32.to_radians(),
                "L{level}: the beams must cross on the aim line ({closest})"
            );
            assert!(
                (last.angle - (base + half)).abs() < 0.15,
                "L{level}: the first beam ends on the opposite (plus) edge: {}",
                last.angle
            );
            assert!(
                (last.angle_b - (base - half)).abs() < 0.15,
                "L{level}: the second beam ends on the opposite (minus) edge: {}",
                last.angle_b
            );
        }
    }

    /// Per level: full-damage radius, pulse damage, sweep angle and duration,
    /// and which ranks cross the aim line.
    #[test]
    fn laser_numbers_per_level() {
        for (level, (pulses, radius, dmg, arc_deg, secs, twin)) in [
            (1u8, (3u32, 28.0f32, 6.0f32, 60.0f32, 0.26f32, false)),
            (2, (3, 34.0, 7.0, 70.0, 0.28, false)),
            (3, (4, 40.0, 8.0, 80.0, 0.30, false)),
            (4, (4, 48.0, 9.0, 95.0, 0.32, true)),
            (5, (5, 58.0, 10.0, 110.0, 0.34, true)),
        ] {
            let mut w = World::new(Theme::Sewer, 11);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::LaserEyes, level);
            run_laser(&mut w, 1.0 / 60.0);
            let fx = *w.ecs.resource::<LaserFx>();
            assert!(fx.active(), "L{level} sweep did not start");
            assert_eq!(fx.pulses, pulses, "L{level} pulse multiplier");
            assert_eq!(fx.full_damage_radius, radius, "L{level} full-damage radius");
            assert_eq!(fx.dmg, dmg, "L{level} damage per sweep hit");
            assert!(
                (fx.half - (arc_deg * 0.5f32).to_radians()).abs() < 1e-4,
                "L{level} half-arc: {}",
                fx.half
            );
            assert!((fx.total - secs).abs() < 1e-4, "L{level} sweep length");
            assert_eq!(fx.twin, twin, "L{level} crossing beams");
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

    /// A body anywhere in the arc is cut **once** per sweep however many ticks
    /// the crossing takes (the two beams share one hit set), the beams pierce a
    /// whole line of bodies, and a body outside the arc is never touched.
    #[test]
    fn a_sweep_cuts_each_body_once_and_misses_what_it_does_not_cross() {
        let mut w = World::new(Theme::Sewer, 13);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1); // 60° arc, up to 6 damage
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // A lattice of bodies across the whole arc — both beams' halves, the
        // aim line they close on, and its edges — plus the dead-ahead line the
        // beams pierce end to end, and two the sweep never reaches: one well
        // outside the 30° half-arc, one behind the rat.
        let mut inside: Vec<(Entity, f32)> = Vec::new();
        for deg in [-25.0f32, -12.0, 0.0, 12.0, 25.0] {
            let e = w.spawn_enemy(
                EnemyKind::Cat,
                p + Vec2::from_angle(deg.to_radians()) * 20.0,
                1.0,
            );
            inside.push((e, enemy_hp(&w, e)));
        }
        for (kind, offset) in [
            (EnemyKind::Brute, Vec2::new(8.0, 0.0)),
            (EnemyKind::Cat, Vec2::new(18.0, 0.0)),
            (EnemyKind::Cat, Vec2::new(26.0, 0.0)),
        ] {
            let e = w.spawn_enemy(kind, p + offset, 1.0);
            inside.push((e, enemy_hp(&w, e)));
        }
        let outside = w.spawn_enemy(
            EnemyKind::Cat,
            p + Vec2::from_angle(60.0f32.to_radians()) * 20.0,
            1.0,
        );
        let behind = w.spawn_enemy(EnemyKind::Cat, p - Vec2::new(18.0, 0.0), 1.0);
        let outside_hp = enemy_hp(&w, outside);
        let behind_hp = enemy_hp(&w, behind);

        let mut ticks = 0;
        while ticks < 200 {
            run_laser(&mut w, 1.0 / 60.0);
            ticks += 1;
            if !w.ecs.resource::<LaserFx>().active() {
                break;
            }
        }
        // 6 damage each (one old pulse), once — whichever beam crossed it and
        // however many ticks the two beams spent over it.
        for (entity, before) in &inside {
            assert!(
                (enemy_hp(&w, *entity) - (before - 6.0)).abs() < 1e-3,
                "a body the beams crossed must take the sweep once: {} -> {}",
                before,
                enemy_hp(&w, *entity)
            );
        }
        assert_eq!(enemy_hp(&w, outside), outside_hp, "a body outside the arc was cut");
        assert_eq!(enemy_hp(&w, behind), behind_hp, "a body behind the rat was cut");
    }

    /// Both beams always exist, and they share the sweep's hit set: a body on
    /// the aim line — which both beams cross — is still cut once per sweep, at
    /// every rank.
    #[test]
    fn both_beams_share_the_sweeps_hits() {
        for (level, dmg, crossing) in [(1u8, 6.0f32, false), (4, 9.0, true)] {
            let mut w = World::new(Theme::Sewer, 18);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::LaserEyes, level);
            w.player.face = Vec2::new(1.0, 0.0);
            let p = w.player.pos;
            let ahead = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(14.0, 0.0), 1.0);
            let full = enemy_hp(&w, ahead);
            let mut ticks = 0;
            while ticks < 200 {
                run_laser(&mut w, 1.0 / 60.0);
                ticks += 1;
                let fx = *w.ecs.resource::<LaserFx>();
                if fx.active() {
                    assert_eq!(fx.twin, crossing, "L{level}: which ranks cross");
                    assert!(fx.beam_b > 1.0, "L{level} must sweep both beams");
                } else {
                    break;
                }
            }
            assert!(
                (enemy_hp(&w, ahead) - (full - dmg)).abs() < 1e-3,
                "two beams must not double-hit one body: {} -> {}",
                full,
                enemy_hp(&w, ahead)
            );
        }
    }

    #[test]
    fn laser_origin_follows_the_eyes_while_aim_stays_locked() {
        let mut w = World::new(Theme::Sewer, 12);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        let start = w.player.pos;
        run_laser(&mut w, 1.0 / 60.0);
        let first = *w.ecs.resource::<LaserFx>();
        assert_eq!(first.facing, Vec2::new(1.0, 0.0));
        assert_eq!(first.origin, start + Vec2::new(LASER_EYE_OFFSET, 0.0));
        for dy in [10.0, 20.0, 30.0] {
            w.player.pos = start + Vec2::new(0.0, dy);
            w.player.face = Vec2::new(0.0, 1.0);
            run_laser(&mut w, 1.0 / 60.0);
            let fx = w.ecs.resource::<LaserFx>();
            assert!(fx.active());
            assert_eq!(fx.origin, w.player.pos + Vec2::new(0.0, LASER_EYE_OFFSET));
            assert_eq!(fx.facing, first.facing);
            let progress = fx.age / fx.total;
            assert_eq!(
                (fx.angle, fx.angle_b),
                sweep_pair(first.facing.angle(), first.half, first.span(), progress),
            );
        }
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
        // Three bodies dead ahead inside the full-damage radius, plus two well off
        // the sweep (one beside, one behind the rat).
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
        arm(&mut w, WeaponKind::LaserEyes, 5);
        w.player.pos = Vec2::new(996.0, 1000.0);
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
        run_laser(&mut w, 1.0 / 60.0);
        let fx = *w.ecs.resource::<LaserFx>();
        for (angle, len) in [(fx.angle, fx.beam), (fx.angle_b, fx.beam_b)] {
            let end = fx.origin + Vec2::from_angle(angle) * len;
            assert!(end.x > face - 3.0 && end.x <= face, "beam end {end:?}, wall {face}");
        }
        for _ in 0..25 {
            run_laser(&mut w, 1.0 / 60.0);
        }
        assert!(enemy_hp(&w, in_front) < front_hp, "the near body was shielded");
        assert_eq!(
            enemy_hp(&w, behind_wall),
            behind_hp,
            "the beam reached through the wall"
        );
    }

    #[test]
    fn both_laser_beams_reach_the_widest_view_edges() {
        for viewport in [Vec2::new(57.0, 36.0), Vec2::new(160.0, 80.0), Vec2::new(240.0, 124.0)] {
            for face in [
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, -1.0),
                Vec2::new(-1.0, 1.0).normalized(),
            ] {
                let mut w = World::new(Theme::Sewer, 31);
                w.clear_enemies();
                open_map(&mut w);
                w.viewport = viewport;
                w.player.pos = w.level.arena * 0.5;
                w.player.face = face;
                arm(&mut w, WeaponKind::LaserEyes, 1);

                run_laser(&mut w, 1.0 / 60.0);
                // The view and the muzzle follow the rat.
                w.player.pos += Vec2::new(10.0, -8.0);
                run_laser(&mut w, 1.0 / 60.0);
                let fx = w.ecs.resource::<LaserFx>();
                for (angle, len) in [(fx.angle, fx.beam), (fx.angle_b, fx.beam_b)] {
                    let end = fx.origin + Vec2::from_angle(angle) * len - w.player.pos;
                    // At 2x the original width and height are the half-extents.
                    let edge = (end.x.abs() / viewport.x).max(end.y.abs() / viewport.y);
                    assert!((edge - 1.0).abs() < 1e-4, "viewport {viewport:?}, end {end:?}");
                }
            }
        }
    }

    #[test]
    fn laser_damage_falls_off_with_the_square_of_distance() {
        for rank in [1, 5] {
            let mut weapon = Weapon::new(WeaponKind::LaserEyes);
            weapon.level = rank;
            let r = weapon.laser_full_damage_radius();
            let pulse = weapon.laser_hit_damage();
            for (distance, expected) in [(r * 0.5, 1.0), (r, 1.0), (r * 2.0, 0.25), (r * 4.0, 0.0625)] {
                assert_eq!(laser_damage(pulse, r, distance * distance), pulse * expected);
            }
        }
    }

    #[test]
    fn distant_laser_hits_under_one_damage_have_no_number() {
        let mut w = World::new(Theme::Sewer, 22);
        w.clear_enemies();
        open_map(&mut w);
        w.viewport = Vec2::new(160.0, 80.0);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        let eye = w.player.pos + w.player.face * LASER_EYE_OFFSET;
        let near = w.spawn_enemy(EnemyKind::Cat, eye + Vec2::new(14.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Cat, eye + Vec2::new(112.0, 0.0), 1.0);
        let outside = w.spawn_enemy(EnemyKind::Cat, eye + Vec2::new(180.0, 0.0), 1.0);
        let [near_hp, far_hp, outside_hp] = [near, far, outside].map(|e| enemy_hp(&w, e));
        for _ in 0..20 {
            run_laser(&mut w, 1.0 / 60.0);
        }
        assert_eq!(near_hp - enemy_hp(&w, near), 6.0);
        assert_eq!(far_hp - enemy_hp(&w, far), 0.375);
        assert_eq!(enemy_hp(&w, outside), outside_hp);
        assert_eq!(w.damage_numbers.len(), 1, "only the near hit should print");
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
        // 0.2 s into a 0.26 s sweep it is exactly as lit.
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
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(46.0, 0.0), 1.0);
        let aside = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 40.0), 1.0);
        let hp_crossed = enemy_hp(&w, crossed);
        let hp_far = enemy_hp(&w, far);
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
        assert_eq!(enemy_hp(&w, far), hp_far, "the first tick reached too far");

        // The rest of the dash: the crossed body never takes a second hit.
        for _ in 0..40 {
            if w.player.dash_active == 0.0 {
                break;
            }
            tick_dash(&mut w);
        }
        assert_eq!(
            enemy_hp(&w, crossed),
            after_first,
            "the same body was hit twice by one dash"
        );
        assert!(w.player.dash_active == 0.0, "the dash should be over");
        assert!(enemy_hp(&w, far) < hp_far, "the dash missed a body beyond its old reach");
        assert_eq!(enemy_hp(&w, aside), hp_aside, "an un-crossed body was hit");
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
            // Both bodies stand behind the rat, along the swipe's centre.
            w.player.face = Vec2::new(-1.0, 0.0);
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
            assert!(enemy_hp(&w, inside) < inside_hp, "the plain swipe missed");
            if lens {
                assert!(
                    enemy_hp(&w, edge) < edge_hp,
                    "Lens Cap did not widen the whip"
                );
            } else {
                assert_eq!(enemy_hp(&w, edge), edge_hp, "the plain swipe reached too far");
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
        w.player.face = Vec2::new(-1.0, 0.0);
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
            // A fast loadout produces more than the old route cap of 96 legs.
            w.player.loadout.stats.move_speed = 92.0;
            w.player.vel = Vec2::new(92.0 * 3.2, 0.0);
            w.player.dash_cd = 0.0;
            let p = w.level.arena * 0.5;
            w.player.pos = p;
            let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 4.0);
            let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(170.0, 0.0), 4.0);
            let hp0 = enemy_hp(&w, body);
            let hp_far = enemy_hp(&w, far);
            run_charged_dash(&mut w, charge);
            for _ in 0..40 {
                if w.player.dash_active == 0.0 {
                    break;
                }
                tick_dash(&mut w);
            }
            assert_eq!(w.player.dash_active, 0.0);
            assert_eq!(enemy_hp(&w, far) < hp_far, charge == 1.0);
            let taken = hp0 - enemy_hp(&w, body);
            let track = w.ecs.resource::<DashTrack>();
            let route = track.route();
            assert_eq!(route.first().unwrap().0, p);
            assert_eq!(route.last().unwrap().1, w.player.pos);
            if charge == 1.0 {
                assert!(route.len() > 96, "the old cap would collapse part of the route");
            }
            let length = route.iter().map(|&(a, b)| a.dist(b)).sum();
            (taken, length)
        };
        let (tap_dmg, tap_route) = run(0.0);
        let (full_dmg, full_route) = run(1.0);
        assert!(tap_dmg > 0.0, "the tap dash must still wound");
        assert!(
            full_dmg > tap_dmg * (1.0 + SUPER_DASH_DAMAGE_PER_CHARGE * 0.75),
            "a full charge hit for {full_dmg}, a tap for {tap_dmg}"
        );
        assert!(
            (full_route - tap_route * 2.0).abs() < 0.03,
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
