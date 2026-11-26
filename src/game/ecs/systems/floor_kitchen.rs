//! The Kitchen's own enemy rules (`WAVE4_ROSTERS.md`, "Kitchen").
//!
//! | Kind | Build | Rule | Where it runs |
//! |---|---|---|---|
//! | Crumb Ant | V(Skitterer) | holds its heading [`CRUMB_ANT_HOLD_SECS`] between course corrections | [`crumb_ant_course`], called by `enemy_move::enemy_steering_system` right before `Enemy::steer` |
//! | Pepper Mill | V(Spitter) | the shot leaves as two rays at ±[`PEPPER_MILL_SPLIT_DEG`] | [`pepper_mill_shots`], called by `enemy_move::enemy_fire_system` |
//! | Sink Crab | V(Pillbug) | the curl's shield faces 90° left of its approach | [`sink_crab_shield_facing`], called by `enemy_wave3::pillbug_tick` when the curl locks |
//! | Egg Timer | N | winds, ticks one expanding ring, sleeps | [`floor_kitchen_system`] with an [`EggDial`] per body |
//! | Fruit Fly, Pie Weight | R | none: the base body's behaviour intact | — |
//!
//! ## The Egg Timer is layered, not graduated
//!
//! `EnemyKind::EggTimer.archetype()` still answers `Pillbug`: its radius, nav
//! size class, glue units, death cue and steering all key on that, and this
//! batch did not audit every such site. Instead `pillbug_tick` returns early
//! for it (so it never curls and never wears a shell) and this module's system
//! owns its state machine. While the dial is wound, ticking or asleep the body
//! is pinned to the spot where it stopped: the steering pass still nudges it a
//! fraction of a pixel, and this system puts it back and zeroes its velocity
//! every tick.
//!
//! ## Schedule
//!
//! [`floor_kitchen_system`] belongs right after `enemy_wave4::wave4_system`:
//! after every weapon pass (so a kill lands before the reap) and after the
//! steering pass (so the pin wins the tick).
//!
//! ## Determinism
//!
//! Egg Timers are walked in `SpawnId` order. The state machine draws **no**
//! rng. The only rng draw in this file is the visual `vfx::emit_player_hurt`
//! on the tick a ring actually damages the rat, the same draw every other
//! hostile hit in the game makes. The Crumb Ant's first hold is staggered by
//! its `Enemy::id` (a hash, not a draw) so a file of ants does not correct on
//! the same tick. A run with no Kitchen kind on the floor does nothing here.
//!
//! ## Not done here
//!
//! * The warned-attack limiter (at most two nearby tells): wave 3's ledger is
//!   private to its system, so an Egg Timer does not wait its turn.
//! * The Wind-up Mouse: a lured Egg Timer winds when it is within range of
//!   the key ([`lure_target`], the rule waves 3 and 4 share); its ring still
//!   only hurts the rat.
//! * The bot does not know the ring yet: [`egg_ring_of`] and [`egg_tell_of`]
//!   are the read-only geometry for it and for the hostile layer.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::bullet::{Bullet, Faction};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::palette::Rgb;
use crate::detmath::DetF32;

// ---- Crumb Ant -------------------------------------------------------------

/// Seconds a Crumb Ant marches on one heading before it looks again.
pub const CRUMB_ANT_HOLD_SECS: f32 = 0.25;

/// The heading a Crumb Ant steers by this tick.
///
/// The ant keeps the heading it last read for [`CRUMB_ANT_HOLD_SECS`], then
/// takes the route's current opinion (`flow_dir`, or the straight line at
/// `target` where the field has none) and holds that. The held heading lives
/// in `w4.dir` and the clock in `w4.t`, both unused by a Skitterer body.
///
/// Returns `flow_dir` untouched for every other kind.
pub fn crumb_ant_course(e: &mut Enemy, target: Vec2, flow_dir: Vec2, dt: f32) -> Vec2 {
    if e.kind != EnemyKind::CrumbAnt {
        return flow_dir;
    }
    let fresh = e.w4.dir.len_sq() <= 1e-6;
    e.w4.t -= dt;
    if fresh || e.w4.t <= 0.0 {
        let want = if flow_dir.len_sq() > 0.001 {
            flow_dir.normalized()
        } else {
            (target - e.pos).normalized()
        };
        if want.len_sq() <= 1e-6 {
            // Standing on the target: nothing to hold yet.
            return flow_dir;
        }
        e.w4.dir = want;
        e.w4.t = if fresh {
            // Stagger the first hold by the body's id so a file of ants does
            // not turn as one. A hash, not an rng draw.
            let frac = (e.id.wrapping_mul(2_654_435_761) >> 24) as f32 / 256.0;
            CRUMB_ANT_HOLD_SECS * (0.2 + 0.8 * frac)
        } else {
            CRUMB_ANT_HOLD_SECS
        };
    }
    e.w4.dir
}

// ---- Pepper Mill -----------------------------------------------------------

/// Half the angle between the Pepper Mill's two rays, in degrees.
pub const PEPPER_MILL_SPLIT_DEG: f32 = 10.0;
/// Speed of each ray (the Spitter's shot speed).
pub const PEPPER_MILL_SHOT_SPEED: f32 = 75.0;
/// Seconds each ray lives (the Spitter's shot life).
pub const PEPPER_MILL_SHOT_LIFE: f32 = 4.0;

/// The two velocities of a Pepper Mill shot aimed from `from` at `ppos`:
/// the aim line turned by −10° and by +10°, in that order.
pub fn pepper_mill_rays(from: Vec2, ppos: Vec2) -> [Vec2; 2] {
    let aim = (ppos - from).normalized();
    let (s, c) = PEPPER_MILL_SPLIT_DEG.to_radians().dsin_cos();
    let turn = |sin: f32| Vec2::new(aim.x * c - aim.y * sin, aim.x * sin + aim.y * c);
    [
        turn(-s) * PEPPER_MILL_SHOT_SPEED,
        turn(s) * PEPPER_MILL_SHOT_SPEED,
    ]
}

/// The two bullets a Pepper Mill releases: no centre shot, so the seam between
/// the rays is the safe line.
pub fn pepper_mill_shots(from: Vec2, ppos: Vec2, damage: f32, color: Rgb) -> [Bullet; 2] {
    let [a, b] = pepper_mill_rays(from, ppos);
    let mut left = Bullet::new(from, a, damage, Faction::Enemy, color);
    left.life = PEPPER_MILL_SHOT_LIFE;
    let mut right = Bullet::new(from, b, damage, Faction::Enemy, color);
    right.life = PEPPER_MILL_SHOT_LIFE;
    [left, right]
}

// ---- Sink Crab -------------------------------------------------------------

/// Where a Sink Crab's lid points for the whole curl: 90° to the left of its
/// approach heading (screen space, y down: approaching east, the lid faces
/// north). The Pillbug's 120° shell arc is measured off `Enemy::facing`, so
/// turning the locked facing turns the shield, the art and the bot's reading
/// of it together.
#[inline]
pub fn sink_crab_shield_facing(approach: Vec2) -> Vec2 {
    Vec2::new(approach.y, -approach.x)
}

// ---- Egg Timer -------------------------------------------------------------

/// Walking at the rat; the dial may start once [`EggDial::cd`] is spent.
pub const EGG_PHASE_WALK: u8 = 0;
/// Winding: the tell. The ring's whole footprint is drawn.
pub const EGG_PHASE_WIND: u8 = 1;
/// The ring is live and expanding.
pub const EGG_PHASE_TICK: u8 = 2;
/// Asleep on the spot: the punish window.
pub const EGG_PHASE_SLEEP: u8 = 3;

/// Distance at which an Egg Timer stops and winds.
pub const EGG_TRIGGER_RANGE: f32 = 26.0;
/// Seconds of wind-up before the ring leaves.
pub const EGG_WIND_SECS: f32 = 1.0;
/// Seconds the ring takes to grow from [`EGG_RING_START`] to [`EGG_RING_END`].
pub const EGG_RING_SECS: f32 = 0.8;
/// The ring's first radius.
pub const EGG_RING_START: f32 = 4.0;
/// The ring's last radius.
pub const EGG_RING_END: f32 = 22.0;
/// The ring's band width.
pub const EGG_RING_WIDTH: f32 = 5.0;
/// Damage of the ring, once per strike.
pub const EGG_RING_DAMAGE: f32 = 7.0;
/// Seconds asleep after the ring.
pub const EGG_SLEEP_SECS: f32 = 2.0;
/// Seconds of walking before the dial may wind again.
pub const EGG_COOLDOWN_SECS: f32 = 4.0;

/// One Egg Timer's dial. Inserted by [`floor_kitchen_system`] on the first tick
/// it sees the body.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct EggDial {
    /// One of the `EGG_PHASE_*` constants.
    pub phase: u8,
    /// Seconds left in the current phase.
    pub t: f32,
    /// Seconds of cooldown left (counts down while walking).
    pub cd: f32,
    /// Where the body stopped: the ring's centre and the pin.
    pub anchor: Vec2,
    /// The strike's hit latch.
    pub hit: bool,
}

/// Centre-line radius of the live ring with `t` seconds of the tick phase left.
#[inline]
pub fn egg_ring_radius(t: f32) -> f32 {
    let k = (1.0 - t / EGG_RING_SECS).clamp(0.0, 1.0);
    EGG_RING_START + (EGG_RING_END - EGG_RING_START) * k
}

/// Does the ring band at `radius` around `center` cover the point `p`?
///
/// This is the one geometry test: the hit, the drawn band and the bot's
/// hazard all use it.
#[inline]
pub fn egg_ring_hits(center: Vec2, radius: f32, p: Vec2) -> bool {
    egg_ring_hits_with_padding(center, radius, p, 0.0)
}

fn egg_ring_hits_with_padding(center: Vec2, radius: f32, p: Vec2, padding: f32) -> bool {
    (center.dist(p) - radius).abs() <= EGG_RING_WIDTH * 0.5 + padding
}

/// The live ring of an Egg Timer, read from the body alone: `(centre, radius)`.
///
/// The system mirrors the dial into `w4.phase`/`w4.t` every tick and the body
/// is pinned to the ring's centre, so the renderer and the bot need no query.
#[inline]
pub fn egg_ring_of(e: &Enemy) -> Option<(Vec2, f32)> {
    (e.kind == EnemyKind::EggTimer && e.w4.phase == EGG_PHASE_TICK)
        .then(|| (e.pos, egg_ring_radius(e.w4.t)))
}

/// The winding tell of an Egg Timer: `(centre, progress 0..1)`.
#[inline]
pub fn egg_tell_of(e: &Enemy) -> Option<(Vec2, f32)> {
    (e.kind == EnemyKind::EggTimer && e.w4.phase == EGG_PHASE_WIND)
        .then(|| (e.pos, (1.0 - e.w4.t / EGG_WIND_SECS).clamp(0.0, 1.0)))
}

/// The Kitchen's per-tick pass: every Egg Timer's dial.
///
/// Chain it right after `enemy_wave4::wave4_system`.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn floor_kitchen_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut order: Local<Vec<(u64, Entity)>>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut EggDial>)>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
) {
    let dt = input.dt;
    order.clear();
    order.extend(
        enemies
            .iter()
            // A body killed earlier this tick waits for the reap one row
            // later: it must not wind, ring or hit.
            .filter(|(_, _, e, _)| e.kind == EnemyKind::EggTimer && e.alive())
            .map(|(entity, id, _, _)| (id.0, entity)),
    );
    if order.is_empty() {
        return;
    }
    order.sort_unstable_by_key(|(id, _)| *id);
    // The live Wind-up Mouse key (highest `SpawnId` wins): the rule the
    // steering lure and waves 3 and 4 use, so the walk and the tell agree.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e, dial)) = enemies.get_mut(entity) else {
            continue;
        };
        let Some(mut dial) = dial else {
            // First sight: the dial exists from the next tick on.
            commands.entity(entity).insert(EggDial::default());
            continue;
        };
        // Belt and braces with `pillbug_tick`'s early return: a timer never
        // wears the Pillbug's shell.
        e.w3.curl_t = 0.0;
        // What the dial measures: the key while it lures this body, else the
        // rat. The ring itself only ever hurts the rat.
        let target = lure_target(&e, player.pos, decoy, &level);
        egg_tick(
            &mut e,
            &mut dial,
            &mut player,
            &level,
            target,
            dt,
            &mut rng,
            &mut particles,
            &mut sfx,
        );
        e.w4.phase = dial.phase;
        e.w4.t = dial.t;
    }
}

/// The point an Egg Timer winds at: the Wind-up Mouse's key while the key is
/// luring it, the rat otherwise (the rule waves 3 and 4 share, verbatim).
#[inline]
fn lure_target(e: &Enemy, ppos: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    let Some(key) = decoy else {
        return ppos;
    };
    if !crate::game::ecs::systems::powerups::decoy_lures_body(e) {
        return ppos;
    }
    let decoy_r = crate::game::ecs::systems::powerups::WINDUP_MOUSE_RADIUS;
    if e.pos.dist_sq(key) > decoy_r * decoy_r {
        return ppos;
    }
    if !crate::game::ecs::systems::projectiles::los_clear(level, e.pos, key, e.radius) {
        return ppos;
    }
    key
}

/// One dial tick. `target` is what the dial measures ([`lure_target`]); the
/// ring only hurts the rat. No rng except the hurt cue on a landed hit.
#[allow(clippy::too_many_arguments)]
fn egg_tick(
    e: &mut Enemy,
    dial: &mut EggDial,
    player: &mut Player,
    level: &Level,
    target: Vec2,
    dt: f32,
    rng: &mut RngRes,
    particles: &mut Particles,
    sfx: &mut SfxQueue,
) {
    match dial.phase {
        EGG_PHASE_WIND => {
            dial.t -= dt;
            if dial.t <= 0.0 {
                dial.phase = EGG_PHASE_TICK;
                dial.t = EGG_RING_SECS;
                dial.hit = false;
            }
        }
        EGG_PHASE_TICK => {
            // The ring is tested where it is drawn this tick, then advanced.
            let radius = egg_ring_radius(dial.t);
            // A wall shadows the hit (not the drawn circle): the ring cannot
            // reach a rat with no clear line to the dial.
            if !dial.hit
                && egg_ring_hits_with_padding(
                    dial.anchor, radius, player.pos, player.buffs.point_hit_radius(),
                )
                && crate::game::ecs::systems::projectiles::los_clear(
                    level,
                    dial.anchor,
                    player.pos,
                    0.0,
                )
                && player.take_damage(EGG_RING_DAMAGE)
            {
                dial.hit = true;
                vfx::emit_player_hurt(particles, &mut rng.0, player.pos);
                sfx.0.push(Sfx::Hurt);
            }
            dial.t -= dt;
            if dial.t <= 0.0 {
                dial.phase = EGG_PHASE_SLEEP;
                dial.t = EGG_SLEEP_SECS;
            }
        }
        EGG_PHASE_SLEEP => {
            dial.t -= dt;
            if dial.t <= 0.0 {
                dial.phase = EGG_PHASE_WALK;
                dial.t = 0.0;
                dial.cd = EGG_COOLDOWN_SECS;
            }
        }
        _ => {
            dial.cd = (dial.cd - dt).max(0.0);
            if dial.cd <= 0.0
                && e.stun <= 0.0
                && e.pos.dist(target) <= EGG_TRIGGER_RANGE
                && crate::game::ecs::systems::projectiles::los_clear(level, e.pos, target, 0.0)
            {
                dial.phase = EGG_PHASE_WIND;
                dial.t = EGG_WIND_SECS;
                dial.anchor = e.pos;
                dial.hit = false;
                sfx.0.push(Sfx::Warn);
            }
        }
    }
    if dial.phase != EGG_PHASE_WALK {
        // Stationary until the dial has stopped and slept.
        e.pos = dial.anchor;
        e.vel = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    fn kitchen() -> World {
        let mut w = World::new(Theme::Kitchen, 7);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_kitchen_system).unwrap();
        w.end_tick();
    }

    fn enemy(w: &World, entity: Entity) -> Enemy {
        w.ecs.get::<Enemy>(entity).expect("enemy alive").clone()
    }

    fn dial(w: &World, entity: Entity) -> EggDial {
        *w.ecs.get::<EggDial>(entity).expect("dial inserted")
    }

    #[test]
    fn pepper_mill_rays_sit_ten_degrees_either_side_of_the_aim() {
        let from = Vec2::new(100.0, 100.0);
        let to = Vec2::new(200.0, 100.0);
        let [a, b] = pepper_mill_rays(from, to);
        let aim = Vec2::new(1.0, 0.0);
        for v in [a, b] {
            assert!((v.len() - PEPPER_MILL_SHOT_SPEED).abs() < 1e-3);
            let cos = v.normalized().dot(aim);
            assert!((cos - PEPPER_MILL_SPLIT_DEG.to_radians().dcos()).abs() < 1e-4);
        }
        // One ray each side of the seam.
        assert!(a.y < 0.0 && b.y > 0.0);
        // The seam itself is clear: neither ray travels along the aim.
        assert!(a.y.abs() > 1.0 && b.y.abs() > 1.0);
    }

    #[test]
    fn pepper_mill_releases_two_enemy_shots_with_the_spitters_life() {
        let shots = pepper_mill_shots(Vec2::ZERO, Vec2::new(0.0, 50.0), 3.6, (1, 2, 3));
        assert_eq!(shots.len(), 2);
        for b in &shots {
            assert!((b.life - PEPPER_MILL_SHOT_LIFE).abs() < 1e-6);
            assert!((b.damage - 3.6).abs() < 1e-6);
        }
    }

    #[test]
    fn sink_crab_lid_faces_ninety_degrees_left_of_the_approach() {
        // Approaching east, the lid faces north (screen y grows downward).
        let lid = sink_crab_shield_facing(Vec2::new(1.0, 0.0));
        assert!((lid.x).abs() < 1e-6 && (lid.y + 1.0).abs() < 1e-6);
        // Approaching south, the lid faces east.
        let lid = sink_crab_shield_facing(Vec2::new(0.0, 1.0));
        assert!((lid.x - 1.0).abs() < 1e-6 && lid.y.abs() < 1e-6);
    }

    #[test]
    fn a_curled_sink_crab_blocks_from_its_left_and_is_open_from_its_right() {
        use crate::game::ecs::systems::enemy_wave3::{PILLBUG_FRONT_MULT, PILLBUG_REAR_MULT};
        let mut w = kitchen();
        let at = Vec2::new(400.0, 400.0);
        let entity = w.spawn_enemy(EnemyKind::SinkCrab, at, 1.0);
        let mut e = enemy(&w, entity);
        // Curl locked while approaching east.
        e.facing = sink_crab_shield_facing(Vec2::new(1.0, 0.0));
        e.w3.curl_t = 1.0;
        let north = at + Vec2::new(0.0, -20.0);
        let south = at + Vec2::new(0.0, 20.0);
        assert!((e.damage_mult_from(north) - PILLBUG_FRONT_MULT).abs() < 1e-6);
        assert!((e.damage_mult_from(south) - PILLBUG_REAR_MULT).abs() < 1e-6);
    }

    #[test]
    fn crumb_ant_holds_its_heading_for_a_quarter_second() {
        let mut w = kitchen();
        let entity = w.spawn_enemy(EnemyKind::CrumbAnt, Vec2::new(300.0, 300.0), 1.0);
        let mut e = enemy(&w, entity);
        let target = Vec2::new(500.0, 300.0);
        let east = Vec2::new(1.0, 0.0);
        let south = Vec2::new(0.0, 1.0);
        let first = crumb_ant_course(&mut e, target, east, DT);
        assert!((first.x - 1.0).abs() < 1e-6);
        // Force a full hold from here so the stagger does not matter.
        e.w4.t = CRUMB_ANT_HOLD_SECS;
        let mut held = 0;
        loop {
            let dir = crumb_ant_course(&mut e, target, south, DT);
            if (dir.y - 1.0).abs() < 1e-6 {
                break;
            }
            assert!((dir.x - 1.0).abs() < 1e-6, "the old heading is held");
            held += 1;
            assert!(held < 60, "the ant must correct eventually");
        }
        // 0.25 s at 60 ticks/s is 15 ticks; allow one for float rounding.
        assert!((14..=15).contains(&held), "held {held} ticks");
    }

    #[test]
    fn crumb_ant_rule_leaves_other_kinds_alone() {
        let mut w = kitchen();
        let entity = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(300.0, 300.0), 1.0);
        let mut e = enemy(&w, entity);
        let before = e.w4.t;
        let south = Vec2::new(0.0, 1.0);
        let dir = crumb_ant_course(&mut e, Vec2::new(500.0, 300.0), south, DT);
        assert!(dir.x.abs() < 1e-6 && (dir.y - 1.0).abs() < 1e-6);
        assert!((e.w4.t - before).abs() < 1e-9);
        assert!(e.w4.dir.len_sq() < 1e-9);
    }

    #[test]
    fn egg_ring_grows_from_four_to_twenty_two() {
        assert!((egg_ring_radius(EGG_RING_SECS) - EGG_RING_START).abs() < 1e-5);
        assert!((egg_ring_radius(0.0) - EGG_RING_END).abs() < 1e-5);
        let c = Vec2::new(10.0, 10.0);
        assert!(egg_ring_hits(c, 12.0, c + Vec2::new(12.0, 0.0)));
        assert!(egg_ring_hits(c, 12.0, c + Vec2::new(0.0, 14.4)));
        assert!(!egg_ring_hits(c, 12.0, c + Vec2::new(15.0, 0.0)));
        assert!(!egg_ring_hits(c, 12.0, c + Vec2::new(9.0, 0.0)));
    }

    #[test]
    fn egg_timer_winds_ticks_once_sleeps_and_cools_down() {
        let mut w = kitchen();
        let center = w.player.pos;
        let at = center + Vec2::new(14.0, 0.0);
        let entity = w.spawn_enemy(EnemyKind::EggTimer, at, 1.0);
        let hp0 = w.player.hp;

        tick(&mut w); // inserts the dial
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WALK);
        tick(&mut w); // in range: winds
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WIND);
        assert!(egg_tell_of(&enemy(&w, entity)).is_some());
        assert!((w.player.hp - hp0).abs() < 1e-6, "the tell never bites");

        // A full second of warning with no damage.
        let wind_ticks = (EGG_WIND_SECS / DT) as usize - 2;
        for _ in 0..wind_ticks {
            tick(&mut w);
            assert_eq!(dial(&w, entity).phase, EGG_PHASE_WIND);
            assert!((w.player.hp - hp0).abs() < 1e-6);
        }
        // Through the ring: the rat stands 14 px out, inside 4..22.
        let mut saw_ring = false;
        for _ in 0..((EGG_RING_SECS / DT) as usize + 8) {
            tick(&mut w);
            saw_ring |= egg_ring_of(&enemy(&w, entity)).is_some();
            // Pinned to its spot throughout.
            assert!(enemy(&w, entity).pos.dist(at) < 1e-3);
        }
        assert!(saw_ring);
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_SLEEP);
        let lost = hp0 - w.player.hp;
        assert!(lost > 0.0, "the ring crossed the rat");
        assert!(lost <= EGG_RING_DAMAGE + 1e-3, "one hit per strike, lost {lost}");

        // Sleep, then a cooldown during which it may walk but not wind.
        for _ in 0..((EGG_SLEEP_SECS / DT) as usize + 2) {
            tick(&mut w);
        }
        let d = dial(&w, entity);
        assert_eq!(d.phase, EGG_PHASE_WALK);
        assert!(d.cd > EGG_COOLDOWN_SECS - 0.2);
        for _ in 0..60 {
            tick(&mut w);
            assert_eq!(dial(&w, entity).phase, EGG_PHASE_WALK);
        }
    }

    #[test]
    fn a_dead_egg_timer_cannot_ring_the_rat() {
        let mut w = kitchen();
        let at = w.player.pos + Vec2::new(14.0, 0.0);
        let entity = w.spawn_enemy(EnemyKind::EggTimer, at, 1.0);
        let hp0 = w.player.hp;
        tick(&mut w);
        tick(&mut w);
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WIND);
        // Killed mid-wind, not yet reaped (this test never runs the reap).
        w.ecs.get_mut::<Enemy>(entity).expect("enemy").hp = 0.0;
        for _ in 0..((EGG_WIND_SECS + EGG_RING_SECS) / DT) as usize + 10 {
            tick(&mut w);
        }
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WIND, "the dial is frozen");
        assert!((w.player.hp - hp0).abs() < 1e-6, "a corpse deals no damage");
    }

    #[test]
    fn a_lured_egg_timer_winds_at_the_key_not_the_rat() {
        let mut w = kitchen();
        // Far from the rat, right beside a live key.
        let at = w.player.pos + Vec2::new(120.0, 0.0);
        let key = at + Vec2::new(10.0, 0.0);
        let entity = w.spawn_enemy(EnemyKind::EggTimer, at, 1.0);
        crate::game::ecs::systems::powerups::spawn_decoy(&mut w, key);
        let hp0 = w.player.hp;
        for _ in 0..4 {
            tick(&mut w);
        }
        let e = enemy(&w, entity);
        if crate::game::ecs::systems::powerups::decoy_lures_body(&e) {
            assert_eq!(dial(&w, entity).phase, EGG_PHASE_WIND);
        } else {
            // The shared lure rule excludes this body: it measures the rat.
            assert_eq!(dial(&w, entity).phase, EGG_PHASE_WALK);
        }
        assert!((w.player.hp - hp0).abs() < 1e-6);
    }

    /// Raise a three-cell wall column through the midpoint of `a`..`b` and
    /// say whether it blocks the line.
    fn wall_between(w: &mut World, a: Vec2, b: Vec2) -> bool {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::TileKind;
        use crate::game::ecs::systems::projectiles::los_clear;
        let mid = (a + b) * 0.5;
        let (cx, cy) = ((mid.x / 16.0) as i32, (mid.y / 16.0) as i32);
        for dy in -1..=1 {
            w.level.tiles.set(cx, cy + dy, TileKind::Wall(WallKind::Solid));
        }
        w.level.tiles.rebuild_masks();
        !los_clear(&w.level, a, b, 0.0)
    }

    /// The rat 1 px short of cell 21 and the timer 5 px past it, 22 px apart:
    /// in trigger range and inside the ring's reach, with cell 21 between.
    fn across_cell_21() -> (Vec2, Vec2) {
        let rat = Vec2::new(335.0, 16.0 * 20.0 + 8.0);
        (rat, rat + Vec2::new(22.0, 0.0))
    }

    #[test]
    fn egg_timer_does_not_wind_at_a_rat_behind_a_wall() {
        let mut w = kitchen();
        let (rat, at) = across_cell_21();
        w.player.pos = rat;
        let entity = w.spawn_enemy(EnemyKind::EggTimer, at, 1.0);
        assert!(wall_between(&mut w, at, rat), "the test wall blocks the line");
        for _ in 0..30 {
            tick(&mut w);
        }
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WALK);
    }

    #[test]
    fn egg_ring_does_not_hit_through_a_wall() {
        let mut w = kitchen();
        let (rat, at) = across_cell_21();
        w.player.pos = rat;
        let entity = w.spawn_enemy(EnemyKind::EggTimer, at, 1.0);
        let hp0 = w.player.hp;
        tick(&mut w);
        tick(&mut w);
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WIND);
        // The wall goes up after the wind started: the ring still runs.
        assert!(wall_between(&mut w, at, rat), "the test wall blocks the line");
        for _ in 0..((EGG_WIND_SECS + EGG_RING_SECS) / DT) as usize + 10 {
            tick(&mut w);
        }
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_SLEEP);
        assert!((w.player.hp - hp0).abs() < 1e-6, "the wall shadows the hit");
    }

    #[test]
    fn egg_timer_out_of_range_keeps_walking() {
        let mut w = kitchen();
        let at = w.player.pos + Vec2::new(EGG_TRIGGER_RANGE + 10.0, 0.0);
        let entity = w.spawn_enemy(EnemyKind::EggTimer, at, 1.0);
        for _ in 0..30 {
            tick(&mut w);
        }
        assert_eq!(dial(&w, entity).phase, EGG_PHASE_WALK);
        assert!(egg_ring_of(&enemy(&w, entity)).is_none());
    }

    #[test]
    fn other_floors_are_untouched() {
        let mut w = kitchen();
        let entity = w.spawn_enemy(EnemyKind::Pillbug, w.player.pos + Vec2::new(10.0, 0.0), 1.0);
        let before = enemy(&w, entity);
        for _ in 0..10 {
            tick(&mut w);
        }
        let after = enemy(&w, entity);
        assert!(w.ecs.get::<EggDial>(entity).is_none());
        assert!(after.pos.dist(before.pos) < 1e-6);
        assert_eq!(after.w4.phase, before.w4.phase);
    }
}
