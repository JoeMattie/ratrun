//! The Lab's own rules (batch 6b, `WAVE4_ROSTERS.md` § Lab).
//!
//! | Kind | Build | What lives here |
//! |---|---|---|
//! | Swab Foot | R(Skitterer) | nothing (the Skitterer, unchanged) |
//! | Spinner Lid | V(Bat) | [`lid_weave`]: the Bat's weave flips handedness every [`LID_FLIP_SECS`] |
//! | Glass Runner | R(Cat) | nothing (the Cat, unchanged) |
//! | Walking Syringe | N | [`floor_lab_system`]: plant, dotted ray, thrust, retract, cooldown |
//! | Autoclave Cart | R(Brute) | nothing (the Brute, unchanged) |
//!
//! ## Spinner Lid
//!
//! The Bat's weave is `wander += 4 dt` inside `Enemy::steer`. The steering
//! system calls [`lid_weave`] right after `steer` with the pre-steer `wander`;
//! for a Spinner Lid in a flipped half-period the advance `steer` just made is
//! mirrored, so the weave circles the other way. The clock is `e.w4.t` (a
//! Bat-archetype body never touches its wave-4 state), it only runs while
//! `steer` advanced the weave (a stunned lid does not age), and it draws no
//! rng. Every other kind returns on the first line, bit-for-bit unchanged.
//! The art reads [`lid_face`] from the same clock: the disk is edge-on at the
//! flip and broad mid-way, so "cross while it shows its broad face" is true.
//!
//! ## Walking Syringe
//!
//! Layered, not graduated: `archetype()` still answers `BellFlea` (size class,
//! cull reach, heavy/knockback rules and the Library all keep that key), but
//! `enemy_wave3` skips the flea's hop for a kind this file [`owns`], so the
//! body is an ordinary flow-field walker (`Enemy::steer`, `w3.holds()` false)
//! until it plants. Its state machine is the [`Needle`] component:
//!
//! ```text
//! WALK --(cooldown spent, rat within SYRINGE_PLANT_RANGE)--> TELL 0.75 s
//!   TELL (planted, dotted 26×3 ray) --> THRUST 0.2 s (ray live, 8 damage, once)
//!   THRUST --> RETRACT 0.8 s (planted, harmless: the punish window)
//!   RETRACT --> WALK, cooldown 4 s
//! ```
//!
//! The aim is locked when the tip plants and never tracks, so the answer is
//! the spec's: step across the needle and approach the barrel. The ray is
//! [`ray_from`], one segment read by both the hit test and the art
//! (`art_roster_lab::draw_syringe_tell`). The phase, timer and aim are
//! mirrored into `e.w4` each tick so the art (which only sees `&Enemy`) can
//! draw the pose and the ray; the component stays authoritative.
//!
//! Walls: the syringe only plants with a clear line to its target and the
//! thrust only lands with a clear line from the barrel to the rat
//! (`projectiles::los_clear`), so the ray never hurts through a wall.
//!
//! The Wind-up Mouse: a lured syringe plants on and aims at the key
//! ([`lure_target`], the rule waves 3 and 4 share); the ray still only hurts
//! the rat.
//!
//! Determinism: bodies are walked in `SpawnId` order; the state machine draws
//! **no rng**. The only draw is `vfx::emit_player_hurt`'s on a landed hit, the
//! same draw every other contact hit makes. With no Walking Syringe on the
//! floor the system returns before sorting anything.
//!
//! Schedule: chain `floor_lab::floor_lab_system` right after
//! `enemy_wave4::wave4_system` (after every weapon pass, before the reap).

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

// ---- Spinner Lid ------------------------------------------------------------

/// Seconds between the Spinner Lid's handedness flips.
pub const LID_FLIP_SECS: f32 = 1.2;

/// Is the lid's weave mirrored at weave-clock `t`? (Odd half-periods.)
#[inline]
pub fn lid_flipped(t: f32) -> bool {
    ((t.max(0.0) / LID_FLIP_SECS) as u32) & 1 == 1
}

/// The Spinner Lid's one rule. Call right after `Enemy::steer` with the
/// `wander` the body had before it. A no-op for every other kind.
#[inline]
pub fn lid_weave(e: &mut Enemy, wander_before: f32, dt: f32) {
    if e.kind != EnemyKind::SpinnerLid {
        return;
    }
    let advance = e.wander - wander_before;
    if advance == 0.0 {
        // Stunned or held: the weave did not move, so its clock does not age.
        return;
    }
    if lid_flipped(e.w4.t) {
        e.wander = wander_before - advance;
    }
    e.w4.t += dt;
}

/// How much of its broad face the lid shows, 0 (edge-on, at a flip) to 1
/// (mid-way between flips). Render-side read of the weave clock.
#[inline]
pub fn lid_face(e: &Enemy) -> f32 {
    let x = (e.w4.t.max(0.0) / LID_FLIP_SECS).fract();
    (x * std::f32::consts::PI).sin().clamp(0.0, 1.0)
}

// ---- Walking Syringe --------------------------------------------------------

/// The rat must be this close before the syringe plants its tip.
pub const SYRINGE_PLANT_RANGE: f32 = 30.0;
/// The dotted warning.
pub const SYRINGE_TELL_SECS: f32 = 0.75;
/// The live thrust.
pub const SYRINGE_THRUST_SECS: f32 = 0.2;
/// The harmless retraction (the punish window).
pub const SYRINGE_RETRACT_SECS: f32 = 0.8;
/// Between one retraction and the next plant.
pub const SYRINGE_COOLDOWN: f32 = 4.0;
/// A fresh body cannot plant on its first second.
pub const SYRINGE_FIRST_COOLDOWN: f32 = 1.0;
/// One thrust's damage.
pub const SYRINGE_DAMAGE: f32 = 8.0;
/// The ray: 26 long, 3 wide, starting at the needle tip.
pub const SYRINGE_RAY_LEN: f32 = 26.0;
pub const SYRINGE_RAY_WIDTH: f32 = 3.0;
/// Needle tip, in px ahead of the body centre (the art draws the needle to
/// exactly here).
pub const SYRINGE_TIP: f32 = 7.0;

pub const SYRINGE_WALK: u8 = 0;
pub const SYRINGE_TELL: u8 = 1;
pub const SYRINGE_THRUST: u8 = 2;
pub const SYRINGE_RETRACT: u8 = 3;

/// Does this file own the kind's behaviour (so its archetype's own state
/// machine must leave it alone)?
#[inline]
pub fn owns(kind: EnemyKind) -> bool {
    kind == EnemyKind::WalkingSyringe
}

/// A Walking Syringe's state.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Needle {
    pub phase: u8,
    /// Seconds left in the current phase (TELL, THRUST, RETRACT).
    pub t: f32,
    /// Seconds until it may plant again (WALK).
    pub cd: f32,
    /// The locked aim (unit).
    pub dir: Vec2,
    /// Where the body planted; it is pinned here until the retraction ends.
    pub plant: Vec2,
    /// This thrust already landed (one hit per strike).
    pub hit: bool,
}

impl Default for Needle {
    fn default() -> Self {
        Needle {
            phase: SYRINGE_WALK,
            t: 0.0,
            cd: SYRINGE_FIRST_COOLDOWN,
            dir: Vec2::new(1.0, 0.0),
            plant: Vec2::ZERO,
            hit: false,
        }
    }
}

/// The ray segment for a body planted at `plant` aiming along `dir`.
#[inline]
pub fn ray_from(plant: Vec2, dir: Vec2) -> (Vec2, Vec2) {
    let from = plant + dir * SYRINGE_TIP;
    (from, from + dir * SYRINGE_RAY_LEN)
}

/// Does the ray from `plant` along `dir` touch a circle at `at`?
#[inline]
pub fn ray_hits(plant: Vec2, dir: Vec2, at: Vec2, radius: f32) -> bool {
    let (a, b) = ray_from(plant, dir);
    let ab = b - a;
    let len_sq = ab.len_sq();
    let t = if len_sq > 1e-6 {
        ((at - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let nearest = a + ab * t;
    let reach = SYRINGE_RAY_WIDTH * 0.5 + radius;
    nearest.dist_sq(at) <= reach * reach
}

/// What the art needs to draw the ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyringeRay {
    pub from: Vec2,
    pub to: Vec2,
    /// False: the dotted warning. True: the thrust, which hurts.
    pub live: bool,
    /// 0..1 through the warning (1 on the thrust).
    pub progress: f32,
}

/// The ray this body is showing, from the state mirrored into `e.w4`.
pub fn syringe_ray(e: &Enemy) -> Option<SyringeRay> {
    if e.kind != EnemyKind::WalkingSyringe {
        return None;
    }
    let (live, progress) = match e.w4.phase {
        SYRINGE_TELL => (false, (1.0 - e.w4.t / SYRINGE_TELL_SECS).clamp(0.0, 1.0)),
        SYRINGE_THRUST => (true, 1.0),
        _ => return None,
    };
    // A planted body is pinned, so `e.pos` is the plant point.
    let (from, to) = ray_from(e.pos, e.w4.dir);
    Some(SyringeRay { from, to, live, progress })
}

/// The body's pose for the art.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SyringePose {
    Walk,
    /// 0..1: the plunger drawing back.
    Tell(f32),
    Thrust,
    /// 0..1: the plunger easing back out to rest.
    Retract(f32),
}

pub fn syringe_pose(e: &Enemy) -> SyringePose {
    match e.w4.phase {
        SYRINGE_TELL => SyringePose::Tell((1.0 - e.w4.t / SYRINGE_TELL_SECS).clamp(0.0, 1.0)),
        SYRINGE_THRUST => SyringePose::Thrust,
        SYRINGE_RETRACT => {
            SyringePose::Retract((1.0 - e.w4.t / SYRINGE_RETRACT_SECS).clamp(0.0, 1.0))
        }
        _ => SyringePose::Walk,
    }
}

/// Put a body's mirrored state into a phase (art tests and captures).
pub fn pose_for_test(e: &mut Enemy, phase: u8, dir: Vec2) {
    e.w4.phase = phase;
    e.w4.dir = dir;
    e.w4.t = match phase {
        SYRINGE_TELL => SYRINGE_TELL_SECS * 0.5,
        SYRINGE_THRUST => SYRINGE_THRUST_SECS * 0.5,
        SYRINGE_RETRACT => SYRINGE_RETRACT_SECS * 0.5,
        _ => 0.0,
    };
}

/// Where the needle is aimed: the Wind-up Mouse's key while the key is luring
/// this body, the rat otherwise (the wave-3/wave-4 rule, verbatim: lurable
/// body, inside the key's radius, clear line to it).
#[inline]
fn lure_target(e: &Enemy, ppos: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    let Some(key) = decoy else {
        return ppos;
    };
    if !crate::game::ecs::systems::powerups::decoy_lures_body(e) {
        return ppos;
    }
    if e.pos.dist_sq(key) > crate::game::ecs::systems::powerups::WINDUP_MOUSE_RADIUS.powi(2) {
        return ppos;
    }
    if !crate::game::ecs::systems::projectiles::los_clear(level, e.pos, key, e.radius) {
        return ppos;
    }
    key
}

/// One syringe, one tick. Returns true when its thrust landed this tick.
///
/// `target` is where it plants on and aims at (the Wind-up Mouse's key while
/// the key lures it, the rat otherwise); `ppos` is the rat, the only thing the
/// ray can hurt.
fn needle_tick(
    n: &mut Needle,
    e: &mut Enemy,
    target: Vec2,
    ppos: Vec2,
    dt: f32,
    level: &Level,
    player: &mut Player,
    sfx: &mut SfxQueue,
) -> bool {
    let mut landed = false;
    match n.phase {
        SYRINGE_TELL => {
            n.t -= dt;
            if n.t <= 0.0 {
                n.phase = SYRINGE_THRUST;
                n.t = SYRINGE_THRUST_SECS;
                n.hit = false;
            }
        }
        SYRINGE_THRUST => {
            // One hit per strike, through the rat's i-frames like every other
            // hit in the game.
            if !n.hit
                && ray_hits(n.plant, n.dir, ppos, PLAYER_RADIUS)
                // A wall between the barrel and the rat stops the needle.
                && crate::game::ecs::systems::projectiles::los_clear(
                    level,
                    n.plant,
                    ppos,
                    SYRINGE_RAY_WIDTH * 0.5,
                )
                && player.take_damage(SYRINGE_DAMAGE)
            {
                n.hit = true;
                landed = true;
                sfx.0.push(Sfx::Hurt);
            }
            n.t -= dt;
            if n.t <= 0.0 {
                n.phase = SYRINGE_RETRACT;
                n.t = SYRINGE_RETRACT_SECS;
            }
        }
        SYRINGE_RETRACT => {
            n.t -= dt;
            if n.t <= 0.0 {
                n.phase = SYRINGE_WALK;
                n.t = 0.0;
                n.cd = SYRINGE_COOLDOWN;
            }
        }
        _ => {
            n.cd = (n.cd - dt).max(0.0);
            let to = target - e.pos;
            let dist = to.len();
            // A needle is never aimed through a wall (the leech's rule).
            if n.cd <= 0.0
                && e.stun <= 0.0
                && dist <= SYRINGE_PLANT_RANGE
                && crate::game::ecs::systems::projectiles::los_clear(
                    level,
                    e.pos,
                    target,
                    SYRINGE_RAY_WIDTH * 0.5,
                )
            {
                n.dir = if dist > 1e-3 {
                    to * (1.0 / dist)
                } else if e.facing.len_sq() > 1e-6 {
                    e.facing.normalized()
                } else {
                    Vec2::new(1.0, 0.0)
                };
                n.plant = e.pos;
                n.phase = SYRINGE_TELL;
                n.t = SYRINGE_TELL_SECS;
                n.hit = false;
                sfx.0.push(Sfx::Warn);
            }
        }
    }
    if n.phase != SYRINGE_WALK {
        // Planted: the body does not walk, slide or get shoved off its mark.
        e.pos = n.plant;
        e.vel = Vec2::ZERO;
        e.facing = n.dir;
    }
    // Mirror for the art (see the module docs).
    e.w4.phase = n.phase;
    e.w4.t = n.t;
    e.w4.dir = n.dir;
    landed
}

/// The Lab floor's behaviour pass: the Walking Syringe's needle.
///
/// Chain right after `enemy_wave4::wave4_system`.
#[allow(clippy::too_many_arguments)]
pub fn floor_lab_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut Needle>)>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    // Cheap gate: no syringe on the map, nothing to do (no sort, no rng).
    if !enemies.iter().any(|(_, _, e, _)| owns(e.kind)) {
        return;
    }
    let dt = input.dt;
    // The live key (highest `SpawnId` wins): the rule the steering lure and
    // waves 3 and 4 use, so the walk and the needle agree.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);
    order.clear();
    order.extend(
        enemies
            .iter()
            .filter(|(_, _, e, _)| owns(e.kind))
            .map(|(entity, id, _, _)| (id.0, entity)),
    );
    order.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e, needle)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        let Some(mut needle) = needle else {
            // First sight: give the body its state. It starts walking, so
            // nothing is lost by the one-tick delay.
            commands.entity(entity).insert(Needle::default());
            continue;
        };
        let ppos = player.pos;
        let target = lure_target(&e, ppos, decoy, &level);
        // Explicit reborrows: `Mut`/`ResMut` down to the plain `&mut T` the
        // helpers take.
        if needle_tick(&mut *needle, &mut *e, target, ppos, dt, &level, &mut *player, &mut *sfx) {
            vfx::emit_player_hurt(&mut *particles, &mut rng.0, ppos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    fn lab() -> World {
        let mut w = World::new(Theme::Lab, 11);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.clear_enemies();
        w
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_lab_system).unwrap();
        w.end_tick();
    }

    fn secs(s: f32) -> usize {
        (s / DT).ceil() as usize
    }

    fn needle(w: &World, entity: Entity) -> Needle {
        *w.ecs.get::<Needle>(entity).expect("the syringe has its state")
    }

    /// A syringe 20 px right of the rat, ready to plant.
    fn ready_syringe(w: &mut World) -> Entity {
        let p = w.player.pos;
        let s = w.spawn_enemy(EnemyKind::WalkingSyringe, p + Vec2::new(20.0, 0.0), 1.0);
        tick(w); // inserts the component
        w.ecs.get_mut::<Needle>(s).unwrap().cd = 0.0;
        s
    }

    // ---- Spinner Lid ----

    #[test]
    fn the_lid_flips_every_period() {
        assert!(!lid_flipped(0.0));
        assert!(!lid_flipped(1.19));
        assert!(lid_flipped(1.21));
        assert!(lid_flipped(2.39));
        assert!(!lid_flipped(2.41));
    }

    #[test]
    fn a_flipped_lid_weaves_the_other_way() {
        let mut w = lab();
        let lid = w.spawn_enemy(EnemyKind::SpinnerLid, Vec2::new(300.0, 300.0), 1.0);
        let mut e = w.ecs.get::<Enemy>(lid).unwrap().clone();
        // First half-period: the Bat's own advance survives.
        let before = e.wander;
        e.wander += 4.0 * DT;
        lid_weave(&mut e, before, DT);
        assert!((e.wander - (before + 4.0 * DT)).abs() < 1e-6);
        // Flipped half-period: the advance is mirrored.
        e.w4.t = LID_FLIP_SECS + 0.1;
        let before = e.wander;
        e.wander += 4.0 * DT;
        lid_weave(&mut e, before, DT);
        assert!((e.wander - (before - 4.0 * DT)).abs() < 1e-6);
    }

    #[test]
    fn a_stunned_lid_does_not_age_its_weave() {
        let mut w = lab();
        let lid = w.spawn_enemy(EnemyKind::SpinnerLid, Vec2::new(300.0, 300.0), 1.0);
        let mut e = w.ecs.get::<Enemy>(lid).unwrap().clone();
        let before = e.wander;
        lid_weave(&mut e, before, DT);
        assert_eq!(e.w4.t, 0.0);
        assert_eq!(e.wander, before);
    }

    #[test]
    fn the_weave_rule_leaves_a_bat_alone() {
        let mut w = lab();
        let bat = w.spawn_enemy(EnemyKind::Bat, Vec2::new(300.0, 300.0), 1.0);
        let mut e = w.ecs.get::<Enemy>(bat).unwrap().clone();
        e.w4.t = LID_FLIP_SECS + 0.1;
        let before = e.wander;
        e.wander += 4.0 * DT;
        let want = e.wander;
        lid_weave(&mut e, before, DT);
        assert_eq!(e.wander, want);
        assert_eq!(e.w4.t, LID_FLIP_SECS + 0.1);
    }

    #[test]
    fn the_lid_is_edge_on_at_the_flip_and_broad_between() {
        let mut w = lab();
        let lid = w.spawn_enemy(EnemyKind::SpinnerLid, Vec2::new(300.0, 300.0), 1.0);
        let mut e = w.ecs.get::<Enemy>(lid).unwrap().clone();
        e.w4.t = 0.0;
        assert!(lid_face(&e) < 0.05);
        e.w4.t = LID_FLIP_SECS * 0.5;
        assert!(lid_face(&e) > 0.95);
        e.w4.t = LID_FLIP_SECS * 1.5;
        assert!(lid_face(&e) > 0.95);
    }

    // ---- Walking Syringe ----

    #[test]
    fn the_ray_is_26_by_3_from_the_tip() {
        let plant = Vec2::new(100.0, 100.0);
        let dir = Vec2::new(1.0, 0.0);
        let (a, b) = ray_from(plant, dir);
        assert_eq!(a, Vec2::new(100.0 + SYRINGE_TIP, 100.0));
        assert!((a.dist(b) - 26.0).abs() < 1e-4);
        // A point body: inside the 3-wide lane, outside it, past its end,
        // and behind the barrel.
        assert!(ray_hits(plant, dir, Vec2::new(120.0, 101.4), 0.0));
        assert!(!ray_hits(plant, dir, Vec2::new(120.0, 101.6), 0.0));
        assert!(!ray_hits(plant, dir, Vec2::new(100.0 + SYRINGE_TIP + 26.0 + 1.6, 100.0), 0.0));
        assert!(!ray_hits(plant, dir, Vec2::new(90.0, 100.0), 0.0));
    }

    #[test]
    fn the_ray_reaches_the_rat_it_planted_on() {
        // Anything inside the plant range along the aim is inside the ray's
        // reach (tip offset + length), or the thrust could never land.
        assert!(SYRINGE_TIP + SYRINGE_RAY_LEN >= SYRINGE_PLANT_RANGE);
    }

    #[test]
    fn a_far_syringe_keeps_walking() {
        let mut w = lab();
        let p = w.player.pos;
        let s = w.spawn_enemy(EnemyKind::WalkingSyringe, p + Vec2::new(80.0, 0.0), 1.0);
        tick(&mut w);
        w.ecs.get_mut::<Needle>(s).unwrap().cd = 0.0;
        for _ in 0..secs(1.0) {
            tick(&mut w);
        }
        assert_eq!(needle(&w, s).phase, SYRINGE_WALK);
    }

    #[test]
    fn it_plants_warns_thrusts_once_retracts_and_cools_down() {
        let mut w = lab();
        let s = ready_syringe(&mut w);
        let hp0 = w.player.hp;
        tick(&mut w);
        let n = needle(&w, s);
        assert_eq!(n.phase, SYRINGE_TELL, "in range and off cooldown, it plants");
        assert!(n.dir.x < -0.99, "aimed at the rat");
        // The whole warning is harmless.
        for _ in 0..secs(SYRINGE_TELL_SECS) - 2 {
            tick(&mut w);
            assert_eq!(needle(&w, s).phase, SYRINGE_TELL);
        }
        assert_eq!(w.player.hp, hp0, "the dotted ray does not hurt");
        // The thrust lands exactly once.
        for _ in 0..secs(SYRINGE_THRUST_SECS) + 8 {
            w.player.iframes = 0.0; // even with no grace, one hit per strike
            tick(&mut w);
        }
        let took = hp0 - w.player.hp;
        assert!(took > 0.0, "the thrust hurts a rat standing in the ray");
        assert!(took <= SYRINGE_DAMAGE + 0.001, "one hit per strike, not one per tick ({took})");
        assert_eq!(needle(&w, s).phase, SYRINGE_RETRACT);
        // The retraction is harmless and ends in a 4 s cooldown.
        let hp1 = w.player.hp;
        for _ in 0..secs(SYRINGE_RETRACT_SECS) + 1 {
            w.player.iframes = 0.0;
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp1);
        let n = needle(&w, s);
        assert_eq!(n.phase, SYRINGE_WALK);
        assert!(n.cd > SYRINGE_COOLDOWN - 0.3);
        // And it does not plant again until the cooldown is spent.
        for _ in 0..secs(SYRINGE_COOLDOWN - 0.5) {
            tick(&mut w);
        }
        assert_eq!(needle(&w, s).phase, SYRINGE_WALK);
        for _ in 0..secs(0.8) {
            tick(&mut w);
        }
        assert_eq!(needle(&w, s).phase, SYRINGE_TELL);
    }

    #[test]
    fn stepping_across_the_needle_dodges_the_thrust() {
        let mut w = lab();
        let s = ready_syringe(&mut w);
        tick(&mut w);
        assert_eq!(needle(&w, s).phase, SYRINGE_TELL);
        // The aim is locked: sidestep and the thrust misses.
        w.player.pos = w.player.pos + Vec2::new(0.0, 14.0);
        let hp0 = w.player.hp;
        for _ in 0..secs(SYRINGE_TELL_SECS + SYRINGE_THRUST_SECS + SYRINGE_RETRACT_SECS) + 2 {
            w.player.iframes = 0.0;
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp0);
        assert!(needle(&w, s).dir.x < -0.99, "the aim never tracked");
    }

    #[test]
    fn a_planted_syringe_stays_on_its_mark_and_mirrors_its_pose() {
        let mut w = lab();
        let s = ready_syringe(&mut w);
        tick(&mut w);
        let plant = needle(&w, s).plant;
        w.ecs.get_mut::<Enemy>(s).unwrap().pos = plant + Vec2::new(5.0, 5.0);
        tick(&mut w);
        let e = w.ecs.get::<Enemy>(s).unwrap().clone();
        assert_eq!(e.pos, plant);
        assert_eq!(e.w4.phase, SYRINGE_TELL);
        let ray = syringe_ray(&e).expect("a telling syringe shows its ray");
        assert!(!ray.live);
        assert_eq!((ray.from, ray.to), ray_from(plant, needle(&w, s).dir));
        assert!(matches!(syringe_pose(&e), SyringePose::Tell(_)));
    }

    #[test]
    fn a_lured_needle_plants_on_the_key_not_the_rat() {
        let mut w = lab();
        let p = w.player.pos;
        // Syringe 20 px right of the rat; the key 20 px above the syringe.
        let s = w.spawn_enemy(EnemyKind::WalkingSyringe, p + Vec2::new(20.0, 0.0), 1.0);
        let key = p + Vec2::new(20.0, -20.0);
        crate::game::ecs::systems::powerups::spawn_decoy(&mut w, key);
        tick(&mut w);
        w.ecs.get_mut::<Needle>(s).unwrap().cd = 0.0;
        tick(&mut w);
        let n = needle(&w, s);
        let e = w.ecs.get::<Enemy>(s).unwrap().clone();
        if crate::game::ecs::systems::powerups::decoy_lures_body(&e) {
            assert_eq!(n.phase, SYRINGE_TELL);
            assert!(n.dir.y < -0.99, "aimed at the key, not the rat ({:?})", n.dir);
            // The rat, off the lane, is not hurt by the thrust at the key.
            let hp0 = w.player.hp;
            for _ in 0..secs(SYRINGE_TELL_SECS + SYRINGE_THRUST_SECS) + 4 {
                w.player.iframes = 0.0;
                tick(&mut w);
            }
            assert_eq!(w.player.hp, hp0);
        } else {
            // The shared lure rule excludes this body: it must aim at the rat.
            assert!(n.dir.x < -0.99);
        }
    }

    /// A solid wall column between the rat and a body 20 px to its right.
    /// With the rat at x = 652 (cell 40) and the body at 672 (cell 42), the
    /// midpoint 662 is cell 41: neither stands in the wall.
    fn wall_between(w: &mut World) {
        use crate::game::collision::{cell_of, TileKind, WallKind};
        let mid = w.player.pos + Vec2::new(10.0, 0.0);
        let cell = cell_of(mid);
        for dy in -2i32..=2 {
            w.level.tiles.set(cell.0, cell.1 + dy, TileKind::Wall(WallKind::Solid));
        }
    }

    #[test]
    fn a_needle_does_not_plant_through_a_wall() {
        let mut w = lab();
        // Keep the rat and the syringe in different cells on a known row.
        w.player.pos = Vec2::new(16.0 * 40.0 + 12.0, 16.0 * 40.0 + 8.0);
        let s = ready_syringe(&mut w);
        wall_between(&mut w);
        for _ in 0..secs(1.0) {
            tick(&mut w);
        }
        assert_eq!(needle(&w, s).phase, SYRINGE_WALK, "no line to the rat, no plant");
    }

    #[test]
    fn a_wall_raised_mid_tell_stops_the_thrust() {
        let mut w = lab();
        w.player.pos = Vec2::new(16.0 * 40.0 + 12.0, 16.0 * 40.0 + 8.0);
        let s = ready_syringe(&mut w);
        tick(&mut w);
        assert_eq!(needle(&w, s).phase, SYRINGE_TELL, "open floor: it plants");
        wall_between(&mut w);
        let hp0 = w.player.hp;
        for _ in 0..secs(SYRINGE_TELL_SECS + SYRINGE_THRUST_SECS) + 4 {
            w.player.iframes = 0.0;
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp0, "the ray does not hurt through a wall");
        assert_eq!(needle(&w, s).phase, SYRINGE_RETRACT);
    }

    #[test]
    fn the_system_ignores_a_floor_with_no_syringe() {
        let mut w = lab();
        let p = w.player.pos;
        let flea = w.spawn_enemy(EnemyKind::BellFlea, p + Vec2::new(20.0, 0.0), 1.0);
        let before = w.ecs.get::<Enemy>(flea).unwrap().clone();
        for _ in 0..30 {
            tick(&mut w);
        }
        assert!(w.ecs.get::<Needle>(flea).is_none());
        let after = w.ecs.get::<Enemy>(flea).unwrap().clone();
        assert_eq!(before.pos, after.pos);
        assert_eq!(before.w4.phase, after.w4.phase);
    }

    #[test]
    fn only_the_syringe_is_owned() {
        for kind in EnemyKind::all() {
            assert_eq!(owns(kind), kind == EnemyKind::WalkingSyringe, "{kind:?}");
        }
    }
}
