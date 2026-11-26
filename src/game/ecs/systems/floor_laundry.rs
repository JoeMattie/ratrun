//! The Laundry's own rules (`WAVE4_ROSTERS.md`, "Laundry"): the three bodies
//! on this floor whose behaviour is more than their archetype's.
//!
//! | kind | build | rule here |
//! |---|---|---|
//! | Wool Pill | V(Skitterer) | stalls 0.15 s in every 1.1 s to pull its strand free |
//! | Starch Bulb | V(Spitter) | its shot leaves at 45 px/s and picks up to 95 after 0.5 s |
//! | Peg Snapper | N | two warned jaws slide shut on the spot the rat stood |
//!
//! Clothes Moth (`R(Bat)`) and Lint Golem (`R(Brute)`) need nothing here.
//!
//! ## Where each rule lives
//!
//! * [`floor_laundry_system`] is the one system. It is chained right after
//!   `enemy_wave4::wave4_system` (the coordinator adds that line; this file
//!   does not touch the schedule). It advances the Wool Pill's strand clock,
//!   shapes the Starch Bulb's jet in flight and runs the Peg Snapper's jaws.
//! * [`gait_mult`] is the seam `enemy_move::enemy_steering_system` multiplies
//!   into the speed it hands `Enemy::steer`: 0 while a pill is stalled or a
//!   peg is holding its jaws, exactly 1.0 for every other body (so no older
//!   kind moves a bit).
//! * [`shape_shot`] is the seam `enemy_move::enemy_fire_system` calls on a
//!   freshly built pellet: it tags and slows the Starch Bulb's, and is a no-op
//!   for every other shooter.
//!
//! ## The Peg Snapper is layered, not graduated
//!
//! `EnemyKind::archetype()` still answers `BellFlea` for the Peg Snapper: far
//! too many sites key on the archetype (radius, nav class, traps, the bot, the
//! death cue) to move them all blind. Instead this system **pins the flea's
//! hop cooldown** (`w3.flea_cd`) so the inherited hop never starts, and runs
//! the jaws from its own [`PegJaws`] component. The renderer only gets an
//! `&Enemy`, so the live jaw geometry is mirrored into three `W3` fields the
//! Bell Flea archetype never reads while `flea_phase == 0`: `bar_pos` (jaw
//! centre), `bar_dir` (jaw axis) and `home` (`x` = phase, `y` = seconds into
//! it). [`jaw_view`] turns those back into the one geometry both the hit test
//! and the art use.
//!
//! ## Determinism
//!
//! Pegs are ticked in `SpawnId` order (two pegs closing on the rat in the same
//! tick resolve by spawn order, because the first hit opens the i-frames).
//! A peg the Wind-up Mouse is luring sets its jaws on the key, not the rat.
//! The only rng draw in this file is `vfx::emit_player_hurt` on a jaw hit that
//! actually lands, the same draw every other hit in the game makes. The pill's
//! clock and the jet's speed are pure functions of time.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::bullet::{Bullet, Faction};
use crate::game::ecs::components::SpawnId;
#[cfg(test)]
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::palette::Rgb;

// ---- Wool Pill -------------------------------------------------------------

/// One gather-and-stretch cycle, seconds.
pub const WOOL_PILL_PERIOD: f32 = 1.1;
/// The stall at the end of each cycle, seconds.
pub const WOOL_PILL_PAUSE: f32 = 0.15;

/// The Wool Pill's strand clock lives in `w3.flea_t` (a Skitterer-archetype
/// body is not wave 3, so nothing else reads or writes its `W3`).
#[inline]
pub fn wool_pill_clock(e: &Enemy) -> f32 {
    e.w3.flea_t
}

/// Is this pill in the stalled tail of its cycle?
#[inline]
pub fn wool_pill_paused(e: &Enemy) -> bool {
    e.kind == EnemyKind::WoolPill && wool_pill_clock(e) >= WOOL_PILL_PERIOD - WOOL_PILL_PAUSE
}

/// 0..1 through the stall (0 outside it): the art's "gathered" pose.
#[inline]
pub fn wool_pill_gather(e: &Enemy) -> f32 {
    if !wool_pill_paused(e) {
        return 0.0;
    }
    ((wool_pill_clock(e) - (WOOL_PILL_PERIOD - WOOL_PILL_PAUSE)) / WOOL_PILL_PAUSE).clamp(0.0, 1.0)
}

// ---- Starch Bulb -----------------------------------------------------------

/// The jet's speed for its first [`STARCH_JET_SLOW_SECS`], px/s.
pub const STARCH_JET_SLOW_SPEED: f32 = 45.0;
/// The jet's speed after it picks up, px/s.
pub const STARCH_JET_FAST_SPEED: f32 = 95.0;
/// How long the jet stays slow, seconds.
pub const STARCH_JET_SLOW_SECS: f32 = 0.5;
/// The jet's lifetime: the Spitter pellet's (`enemy_fire_system` sets 4.0).
pub const STARCH_JET_LIFE: f32 = 4.0;
/// The jet's own colour, which is also how the system recognises it in flight
/// (an enemy pellet carries no owner): a cold starch white no palette accent
/// uses.
pub const STARCH_JET_COLOR: Rgb = (233, 241, 252);

/// The jet's speed `age` seconds after it left the nozzle.
#[inline]
pub fn starch_jet_speed(age: f32) -> f32 {
    if age < STARCH_JET_SLOW_SECS {
        STARCH_JET_SLOW_SPEED
    } else {
        STARCH_JET_FAST_SPEED
    }
}

/// Is this bullet a Starch Bulb's jet?
#[inline]
pub fn is_starch_jet(b: &Bullet) -> bool {
    matches!(b.faction, Faction::Enemy)
        && b.color == STARCH_JET_COLOR
        && b.lob.is_none()
        && b.orbit.is_none()
}

/// The fire seam: called by `enemy_fire_system` on the pellet it just built
/// for `kind`. A Starch Bulb's leaves slow and tagged; every other shooter's
/// is untouched.
#[inline]
pub fn shape_shot(kind: EnemyKind, b: &mut Bullet) {
    if kind != EnemyKind::StarchBulb {
        return;
    }
    let dir = b.vel.normalized();
    b.vel = dir * STARCH_JET_SLOW_SPEED;
    b.color = STARCH_JET_COLOR;
    b.life = STARCH_JET_LIFE;
}

// ---- Peg Snapper -----------------------------------------------------------

/// The rat has to be this close (centre to centre) for the peg to set its jaws.
pub const PEG_TRIGGER_RANGE: f32 = 20.0;
/// Each jaw is this long (along the jaw axis) and this thick, px.
pub const PEG_JAW_LEN: f32 = 16.0;
pub const PEG_JAW_THICK: f32 = 3.0;
/// The two jaws start this far apart (centre line to centre line), px.
pub const PEG_JAW_GAP: f32 = 14.0;
/// How far apart they end up when shut (they meet, edge to edge).
pub const PEG_JAW_SHUT_GAP: f32 = PEG_JAW_THICK;
/// The warning: jaws shown open and harmless, seconds.
pub const PEG_TELL_SECS: f32 = 0.8;
/// The strike: jaws slide together, live, seconds.
pub const PEG_CLOSE_SECS: f32 = 0.3;
/// The rest: jaws stay shut and harmless, seconds.
pub const PEG_SHUT_SECS: f32 = 1.0;
/// Seconds after the jaws open again before the next set.
pub const PEG_COOLDOWN: f32 = 4.0;
/// What a closing jaw takes off the rat on touch.
pub const PEG_JAW_DAMAGE: f32 = 7.0;
/// A fresh peg waits this long before its first set, so it cannot spawn shut.
pub const PEG_SPAWN_DELAY: f32 = 1.0;
/// The inherited Bell Flea hop cooldown is held at this so the hop never starts.
const PEG_FLEA_PIN: f32 = 60.0;

pub const PEG_PHASE_WALK: u8 = 0;
pub const PEG_PHASE_TELL: u8 = 1;
pub const PEG_PHASE_CLOSE: u8 = 2;
pub const PEG_PHASE_SHUT: u8 = 3;

/// The Peg Snapper's state machine: walk → tell → close → shut → walk.
#[derive(Component, Clone, Copy, Debug)]
pub struct PegJaws {
    pub phase: u8,
    /// Seconds into the current phase.
    pub t: f32,
    /// Seconds until the next set (counts down while walking).
    pub cd: f32,
    /// Where the jaws were set: the midpoint between them (the rat at lock).
    pub centre: Vec2,
    /// The jaws' long axis (unit): from the peg toward the rat at lock.
    pub axis: Vec2,
    /// This strike has already taken its one bite.
    pub spent: bool,
}

impl Default for PegJaws {
    fn default() -> Self {
        PegJaws {
            phase: PEG_PHASE_WALK,
            t: 0.0,
            cd: PEG_SPAWN_DELAY,
            centre: Vec2::ZERO,
            axis: Vec2::new(1.0, 0.0),
            spent: false,
        }
    }
}

/// The jaws as everything outside the state machine sees them: the hit test,
/// the art and (later) the bot all read this one view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JawView {
    pub centre: Vec2,
    pub axis: Vec2,
    /// Distance from the centre line to each jaw's own centre line, px.
    pub offset: f32,
    pub phase: u8,
    /// Seconds into the phase.
    pub t: f32,
}

impl JawView {
    /// The two jaws' centres (left of the axis, right of the axis).
    #[inline]
    pub fn jaws(&self) -> [Vec2; 2] {
        let side = self.axis.perp();
        [self.centre + side * self.offset, self.centre - side * self.offset]
    }

    /// Can the jaws hurt right now? Only while they slide.
    #[inline]
    pub fn live(&self) -> bool {
        self.phase == PEG_PHASE_CLOSE
    }

    /// Does a circle touch either jaw's `PEG_JAW_LEN × PEG_JAW_THICK` bar?
    pub fn touches(&self, p: Vec2, r: f32) -> bool {
        let side = self.axis.perp();
        for c in self.jaws() {
            let d = p - c;
            let u = d.dot(self.axis).abs() - PEG_JAW_LEN * 0.5;
            let v = d.dot(side).abs() - PEG_JAW_THICK * 0.5;
            let (u, v) = (u.max(0.0), v.max(0.0));
            if u * u + v * v <= r * r {
                return true;
            }
        }
        false
    }
}

/// Half the gap between the jaws for a phase and the seconds into it.
#[inline]
pub fn jaw_offset(phase: u8, t: f32) -> f32 {
    let open = PEG_JAW_GAP * 0.5;
    let shut = PEG_JAW_SHUT_GAP * 0.5;
    match phase {
        PEG_PHASE_TELL => open,
        PEG_PHASE_CLOSE => {
            let k = (t / PEG_CLOSE_SECS).clamp(0.0, 1.0);
            open + (shut - open) * k
        }
        PEG_PHASE_SHUT => shut,
        _ => open,
    }
}

/// The jaws of this body, if it is a Peg Snapper with jaws on the floor.
/// Read from the `W3` mirror the system writes (see the module doc).
pub fn jaw_view(e: &Enemy) -> Option<JawView> {
    if e.kind != EnemyKind::PegSnapper || !e.alive() {
        return None;
    }
    let phase = e.w3.home.x.round() as i32;
    if !(PEG_PHASE_TELL as i32..=PEG_PHASE_SHUT as i32).contains(&phase) {
        return None;
    }
    let phase = phase as u8;
    let t = e.w3.home.y;
    Some(JawView {
        centre: e.w3.bar_pos,
        axis: e.w3.bar_dir,
        offset: jaw_offset(phase, t),
        phase,
        t,
    })
}

/// Is this peg planted (tell, strike or rest)?
#[inline]
pub fn peg_holding(e: &Enemy) -> bool {
    e.kind == EnemyKind::PegSnapper && e.w3.home.x >= 0.5
}

/// Write the component's state into the `W3` mirror the renderer reads.
#[inline]
fn mirror(e: &mut Enemy, jaws: &PegJaws) {
    e.w3.bar_pos = jaws.centre;
    e.w3.bar_dir = jaws.axis;
    e.w3.home = Vec2::new(jaws.phase as f32, jaws.t);
}

/// Where a peg's jaws are aimed: the Wind-up Mouse's key if the key is luring
/// this body (in its radius, with a clear walking line), the rat otherwise.
/// The same three questions `enemy_wave3`'s lure asks, so the walk and the
/// tell agree. No rng.
fn lure_target(e: &Enemy, rat: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    use crate::game::ecs::systems::{powerups, projectiles};
    let Some(key) = decoy else {
        return rat;
    };
    if !powerups::decoy_lures_body(e)
        || e.pos.dist_sq(key)
            > powerups::WINDUP_MOUSE_RADIUS * powerups::WINDUP_MOUSE_RADIUS
        || !projectiles::los_clear(level, e.pos, key, e.radius)
    {
        return rat;
    }
    key
}

/// One tick of one peg. `target` is where a new set of jaws is aimed (the rat,
/// or the decoy's key); the bite always tests the rat. Returns `true` when the
/// jaws bit the rat this tick.
#[allow(clippy::too_many_arguments)]
fn peg_tick(
    e: &mut Enemy,
    jaws: &mut PegJaws,
    target: Vec2,
    level: &Level,
    dt: f32,
    player: &mut Player,
    sfx: &mut SfxQueue,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) -> bool {
    let mut bit = false;
    match jaws.phase {
        PEG_PHASE_TELL => {
            jaws.t += dt;
            if jaws.t >= PEG_TELL_SECS {
                jaws.phase = PEG_PHASE_CLOSE;
                jaws.t = 0.0;
                jaws.spent = false;
            }
        }
        PEG_PHASE_CLOSE => {
            jaws.t += dt;
            let view = JawView {
                centre: jaws.centre,
                axis: jaws.axis,
                offset: jaw_offset(PEG_PHASE_CLOSE, jaws.t),
                phase: PEG_PHASE_CLOSE,
                t: jaws.t,
            };
            // One bite per strike, through the rat's i-frames like every
            // other hit (a blocked touch does not spend the strike).
            if !jaws.spent
                && view.touches(player.pos, player.buffs.hurt_radius())
                && player.take_damage(PEG_JAW_DAMAGE * dmg_scale)
            {
                jaws.spent = true;
                bit = true;
                sfx.0.push(Sfx::Hurt);
            }
            if jaws.t >= PEG_CLOSE_SECS {
                jaws.phase = PEG_PHASE_SHUT;
                jaws.t = 0.0;
            }
        }
        PEG_PHASE_SHUT => {
            jaws.t += dt;
            if jaws.t >= PEG_SHUT_SECS {
                jaws.phase = PEG_PHASE_WALK;
                jaws.t = 0.0;
                jaws.cd = PEG_COOLDOWN;
                jaws.spent = false;
            }
        }
        _ => {
            jaws.cd = (jaws.cd - dt).max(0.0);
            let to = target - e.pos;
            let dist = to.len();
            // A stunned peg cannot set its jaws (the Sonic Squeak ring).
            // A wall between the peg and its target stops the set: the jaws
            // are never laid on a rat the peg cannot reach (a 16 px partition
            // is thinner than the 20 px trigger). One test at the trigger is
            // enough, since the set is fixed and the peg holds still.
            if jaws.cd <= 0.0
                && e.stun <= 0.0
                && dist <= PEG_TRIGGER_RANGE
                && crate::game::ecs::systems::projectiles::los_clear(
                    level,
                    e.pos,
                    target,
                    PEG_JAW_THICK * 0.5,
                )
            {
                let axis = if dist > 1e-3 {
                    to * (1.0 / dist)
                } else if e.facing.len() > 1e-3 {
                    e.facing.normalized()
                } else {
                    Vec2::new(1.0, 0.0)
                };
                jaws.phase = PEG_PHASE_TELL;
                jaws.t = 0.0;
                jaws.centre = target;
                jaws.axis = axis;
                jaws.spent = false;
                e.facing = axis;
                sfx.0.push(Sfx::Warn);
            }
        }
    }
    mirror(e, jaws);
    bit
}

// ---- The seams -------------------------------------------------------------

/// The steering seam: the multiplier `enemy_steering_system` folds into the
/// speed it hands `Enemy::steer`. Exactly 1.0 for every body that is not a
/// stalled Wool Pill or a planted Peg Snapper.
#[inline]
pub fn gait_mult(e: &Enemy) -> f32 {
    match e.kind {
        EnemyKind::WoolPill if wool_pill_paused(e) => 0.0,
        EnemyKind::PegSnapper if peg_holding(e) => 0.0,
        _ => 1.0,
    }
}

/// The Laundry's floor system. Chain it right after
/// `enemy_wave4::wave4_system`.
///
/// It takes `Commands` only to give a Peg Snapper its [`PegJaws`] on the first
/// tick it is seen (the director spawns a bare `Enemy`), so the chain's
/// `ApplyDeferred` makes the component real before the next tick.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn floor_laundry_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut PegJaws>)>,
    mut bullets: Query<&mut Bullet>,
    // The floor's hit softening (`[enemies] sector_damage_mult`): the peg's jaw
    // carries it.
    sector: crate::game::ecs::resources::SectorDamage,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let dmg_scale = sector.mult();

    // ---- Starch Bulb: the jet picks up after half a second ---------------
    // Each jet depends only on itself, so storage order is harmless here.
    for mut b in bullets.iter_mut() {
        if !is_starch_jet(&b) {
            continue;
        }
        let age = STARCH_JET_LIFE - b.life;
        let want = starch_jet_speed(age);
        let have = b.vel.len();
        if have > 1e-3 && (have - want).abs() > 0.01 {
            b.vel = b.vel * (want / have);
        }
    }

    // ---- Wool Pill: the strand clock (each pill depends only on itself) --
    order.clear();
    for (entity, id, mut e, _) in enemies.iter_mut() {
        if !e.alive() {
            continue;
        }
        match e.kind {
            EnemyKind::WoolPill => {
                let mut t = e.w3.flea_t + dt;
                if t >= WOOL_PILL_PERIOD {
                    t -= WOOL_PILL_PERIOD;
                }
                e.w3.flea_t = t;
            }
            EnemyKind::PegSnapper => order.push((id.0, entity)),
            _ => {}
        }
    }

    // ---- Peg Snapper: the jaws, in `SpawnId` order -----------------------
    if order.is_empty() {
        return;
    }
    order.sort_unstable_by_key(|&(id, _)| id);
    // The live key (highest `SpawnId` wins), the rule steering and wave 3 use.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);
    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e, jaws)) = enemies.get_mut(entity) else {
            continue;
        };
        // The inherited hop never starts: the flea's cooldown is held up.
        if e.w3.flea_phase == 0 {
            e.w3.flea_cd = PEG_FLEA_PIN;
        }
        let Some(mut jaws) = jaws else {
            commands.entity(entity).insert(PegJaws::default());
            continue;
        };
        let target = lure_target(&e, player.pos, decoy, &level);
        if peg_tick(&mut e, &mut jaws, target, &level, dt, &mut player, &mut sfx, dmg_scale) {
            // The one rng draw in this file: the same hurt burst every other
            // hit makes, and only when a bite actually lands.
            let at = player.pos;
            vfx::emit_player_hurt(&mut particles, &mut rng.0, at);
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

    fn world() -> World {
        let mut w = World::new(Theme::Laundry, 7);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.player.pos = Vec2::new(800.0, 800.0);
        w
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_laundry_system).unwrap();
        w.end_tick();
    }

    fn secs(s: f32) -> usize {
        (s / DT).ceil() as usize
    }

    fn state(w: &World, entity: Entity) -> Enemy {
        w.ecs.get::<Enemy>(entity).unwrap().clone()
    }

    #[test]
    fn gait_mult_is_one_for_every_other_kind() {
        let mut w = world();
        for kind in EnemyKind::all() {
            if matches!(kind, EnemyKind::WoolPill | EnemyKind::PegSnapper) {
                continue;
            }
            let en = w.spawn_enemy(kind, Vec2::new(400.0, 400.0), 1.0);
            assert_eq!(gait_mult(&state(&w, en)), 1.0, "{kind:?}");
        }
    }

    #[test]
    fn wool_pill_stalls_for_the_tail_of_each_cycle() {
        let mut w = world();
        let pill = w.spawn_enemy(EnemyKind::WoolPill, Vec2::new(400.0, 400.0), 1.0);
        let mut paused = 0usize;
        let cycle = secs(WOOL_PILL_PERIOD);
        for _ in 0..cycle * 4 {
            tick(&mut w);
            if gait_mult(&state(&w, pill)) == 0.0 {
                paused += 1;
            }
        }
        // 0.15 s of every 1.1 s, give or take a tick per cycle.
        let want = (WOOL_PILL_PAUSE / DT) as usize * 4;
        assert!(paused + 5 >= want && paused <= want + 5, "paused {paused} ticks, want ~{want}");
    }

    #[test]
    fn wool_pill_walks_at_the_start_of_a_cycle() {
        let mut w = world();
        let pill = w.spawn_enemy(EnemyKind::WoolPill, Vec2::new(400.0, 400.0), 1.0);
        tick(&mut w);
        assert_eq!(gait_mult(&state(&w, pill)), 1.0);
        assert_eq!(wool_pill_gather(&state(&w, pill)), 0.0);
    }

    #[test]
    fn starch_jet_is_slow_then_fast() {
        assert_eq!(starch_jet_speed(0.0), STARCH_JET_SLOW_SPEED);
        assert_eq!(starch_jet_speed(0.49), STARCH_JET_SLOW_SPEED);
        assert_eq!(starch_jet_speed(0.5), STARCH_JET_FAST_SPEED);
    }

    #[test]
    fn shape_shot_only_touches_the_starch_bulb() {
        let v = Vec2::new(75.0, 0.0);
        let mut plain = Bullet::new(Vec2::ZERO, v, 3.0, Faction::Enemy, (255, 0, 0));
        plain.life = 4.0;
        let before = (plain.vel, plain.color, plain.life);
        shape_shot(EnemyKind::Spitter, &mut plain);
        shape_shot(EnemyKind::PepperMill, &mut plain);
        assert_eq!((plain.vel, plain.color, plain.life), before);
        assert!(!is_starch_jet(&plain));

        shape_shot(EnemyKind::StarchBulb, &mut plain);
        assert!(is_starch_jet(&plain));
        assert!((plain.vel.len() - STARCH_JET_SLOW_SPEED).abs() < 1e-3);
        assert!(plain.vel.x > 0.0 && plain.vel.y.abs() < 1e-3, "direction kept");
    }

    #[test]
    fn the_system_speeds_an_old_jet_and_leaves_other_bullets() {
        let mut w = world();
        let mut jet = Bullet::new(Vec2::new(100.0, 100.0), Vec2::new(75.0, 0.0), 3.0, Faction::Enemy, (1, 2, 3));
        shape_shot(EnemyKind::StarchBulb, &mut jet);
        let young = w.spawn_bullet(jet.clone());
        let mut aged = jet.clone();
        aged.life = STARCH_JET_LIFE - STARCH_JET_SLOW_SECS - 0.1;
        let old = w.spawn_bullet(aged);
        let other = w.spawn_bullet(Bullet::new(
            Vec2::new(100.0, 120.0),
            Vec2::new(75.0, 0.0),
            3.0,
            Faction::Enemy,
            (1, 2, 3),
        ));
        tick(&mut w);
        let speed = |w: &World, e: Entity| w.ecs.get::<Bullet>(e).unwrap().vel.len();
        assert!((speed(&w, young) - STARCH_JET_SLOW_SPEED).abs() < 0.01);
        assert!((speed(&w, old) - STARCH_JET_FAST_SPEED).abs() < 0.01);
        assert!((speed(&w, other) - 75.0).abs() < 1e-4);
    }

    /// A peg next to the rat, already given its component and off cooldown.
    fn ready_peg(w: &mut World, at: Vec2) -> Entity {
        let peg = w.spawn_enemy(EnemyKind::PegSnapper, at, 1.0);
        w.ecs.entity_mut(peg).insert(PegJaws { cd: 0.0, ..PegJaws::default() });
        peg
    }

    #[test]
    fn a_fresh_peg_gets_its_component_and_never_hops() {
        let mut w = world();
        let at = w.player.pos + Vec2::new(200.0, 0.0);
        let peg = w.spawn_enemy(EnemyKind::PegSnapper, at, 1.0);
        tick(&mut w);
        assert!(w.ecs.get::<PegJaws>(peg).is_some());
        let e = state(&w, peg);
        assert_eq!(e.w3.flea_phase, 0);
        assert!(e.w3.flea_cd > 10.0, "the inherited hop is pinned");
        assert!(jaw_view(&e).is_none());
    }

    #[test]
    fn peg_out_of_range_does_not_set_its_jaws() {
        let mut w = world();
        let at = w.player.pos + Vec2::new(PEG_TRIGGER_RANGE + 6.0, 0.0);
        let peg = ready_peg(&mut w, at);
        for _ in 0..30 {
            tick(&mut w);
        }
        assert_eq!(w.ecs.get::<PegJaws>(peg).unwrap().phase, PEG_PHASE_WALK);
        assert_eq!(gait_mult(&state(&w, peg)), 1.0);
    }

    #[test]
    fn peg_tell_is_harmless_and_shows_open_jaws_around_the_rat() {
        let mut w = world();
        let rat = w.player.pos;
        let peg = ready_peg(&mut w, rat + Vec2::new(-15.0, 0.0));
        let hp = w.player.hp;
        tick(&mut w);
        let view = jaw_view(&state(&w, peg)).expect("jaws on the floor");
        assert_eq!(view.phase, PEG_PHASE_TELL);
        assert!(!view.live());
        assert!((view.offset - PEG_JAW_GAP * 0.5).abs() < 1e-4);
        assert!(view.centre.dist(rat) < 1e-3);
        // Jaws 14 apart, either side of the rat, along the peg→rat axis.
        let [a, b] = view.jaws();
        assert!((a.dist(b) - PEG_JAW_GAP).abs() < 1e-3);
        assert!(view.axis.x > 0.99);
        // The rat stands in the middle through the whole tell: no damage.
        for _ in 0..secs(PEG_TELL_SECS) - 2 {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp);
        assert_eq!(gait_mult(&state(&w, peg)), 0.0, "a planted peg does not walk");
    }

    #[test]
    fn closing_jaws_bite_once_for_seven() {
        let mut w = world();
        let rat = w.player.pos;
        let peg = ready_peg(&mut w, rat + Vec2::new(-15.0, 0.0));
        let hp = w.player.hp;
        for _ in 0..secs(PEG_TELL_SECS + PEG_CLOSE_SECS + PEG_SHUT_SECS) + 4 {
            // Hold the i-frames off so a second bite would show.
            w.player.iframes = 0.0;
            tick(&mut w);
        }
        let lost = hp - w.player.hp;
        assert!(lost > 0.0, "the jaws closed on a rat that stood still");
        assert!(lost <= PEG_JAW_DAMAGE + 1e-3, "one bite per strike, lost {lost}");
        let jaws = *w.ecs.get::<PegJaws>(peg).unwrap();
        assert_eq!(jaws.phase, PEG_PHASE_WALK);
        assert!(jaws.cd > PEG_COOLDOWN - 0.5);
    }

    #[test]
    fn walking_out_through_an_open_end_is_safe() {
        let mut w = world();
        let rat = w.player.pos;
        let peg = ready_peg(&mut w, rat + Vec2::new(-15.0, 0.0));
        let hp = w.player.hp;
        tick(&mut w);
        let view = jaw_view(&state(&w, peg)).unwrap();
        // Out past the far end of the jaws, along the axis.
        w.player.pos = view.centre + view.axis * (PEG_JAW_LEN * 0.5 + PLAYER_RADIUS + 3.0);
        for _ in 0..secs(PEG_TELL_SECS + PEG_CLOSE_SECS + PEG_SHUT_SECS) + 4 {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp);
    }

    #[test]
    fn a_lured_peg_sets_its_jaws_on_the_key() {
        use crate::game::ecs::systems::powerups::Decoy;
        let mut w = world();
        let rat = w.player.pos;
        let key = rat + Vec2::new(-30.0, 0.0);
        let peg = ready_peg(&mut w, key + Vec2::new(-12.0, 0.0));
        let id = w.ecs.resource_mut::<crate::game::ecs::resources::NextSpawnId>().take();
        w.ecs.spawn((Decoy::new(key), id));
        let hp = w.player.hp;
        tick(&mut w);
        let view = jaw_view(&state(&w, peg)).expect("the key is in range");
        assert!(view.centre.dist(key) < 1e-3, "jaws at {:?}, key at {key:?}", view.centre);
        for _ in 0..secs(PEG_TELL_SECS + PEG_CLOSE_SECS) + 2 {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp, "the rat is nowhere near the key");
    }

    #[test]
    fn a_peg_behind_a_wall_does_not_set_its_jaws() {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::{cell_of, TileKind};
        let mut w = world();
        // The rat stands 1 px inside its cell's west edge; the cell to the
        // west is solid; the peg stands 2 px beyond that wall, 19 px away.
        let (cx, cy) = cell_of(w.player.pos);
        let rat = Vec2::new(cx as f32 * 16.0 + 1.0, cy as f32 * 16.0 + 8.0);
        w.player.pos = rat;
        let peg = ready_peg(&mut w, rat + Vec2::new(-19.0, 0.0));
        w.level.tiles.set(cx - 1, cy, TileKind::Wall(WallKind::Solid));
        let hp = w.player.hp;
        for _ in 0..secs(PEG_TELL_SECS + PEG_CLOSE_SECS) + 4 {
            tick(&mut w);
        }
        assert_eq!(w.ecs.get::<PegJaws>(peg).unwrap().phase, PEG_PHASE_WALK);
        assert!(jaw_view(&state(&w, peg)).is_none(), "jaws shown through a wall");
        assert_eq!(w.player.hp, hp);

        // The same spacing with the wall gone sets the jaws at once.
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        tick(&mut w);
        assert_eq!(w.ecs.get::<PegJaws>(peg).unwrap().phase, PEG_PHASE_TELL);
    }

    #[test]
    fn shut_jaws_are_harmless() {
        let view = JawView {
            centre: Vec2::new(50.0, 50.0),
            axis: Vec2::new(1.0, 0.0),
            offset: jaw_offset(PEG_PHASE_SHUT, 0.5),
            phase: PEG_PHASE_SHUT,
            t: 0.5,
        };
        assert!(!view.live());
        assert!(view.touches(Vec2::new(50.0, 50.0), 2.0), "the geometry is still there");
    }

    #[test]
    fn jaw_offset_slides_from_open_to_shut() {
        assert_eq!(jaw_offset(PEG_PHASE_TELL, 0.3), PEG_JAW_GAP * 0.5);
        assert_eq!(jaw_offset(PEG_PHASE_CLOSE, 0.0), PEG_JAW_GAP * 0.5);
        assert!((jaw_offset(PEG_PHASE_CLOSE, PEG_CLOSE_SECS) - PEG_JAW_SHUT_GAP * 0.5).abs() < 1e-5);
        let mid = jaw_offset(PEG_PHASE_CLOSE, PEG_CLOSE_SECS * 0.5);
        assert!(mid < PEG_JAW_GAP * 0.5 && mid > PEG_JAW_SHUT_GAP * 0.5);
    }

}
