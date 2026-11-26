//! Wave 3 **pocket finds**: the timed effects of the four new pickups.
//!
//! `WAVE3_DESIGN.md` §10 adds Bottle Cap, Wind-up Mouse, Pocket Watch and Clean
//! Socks to the rare-pickup table. The drop roll and the one-shot collection
//! effects live with the old two (`loot::pickups_system`); the **timers** live
//! here, in one system that runs first in the tick:
//!
//! | slot | system | why it is first |
//! |---|--------|-----------------|
//! | 0 | `powerups_system` | ages every buff before anything reads it, so a 4 s Pocket Watch covers exactly 4 s of projectile travel, a 6 s Clean Socks covers exactly 6 s of floor traps, and an expiring Bottle Cap cannot absorb a hit in its last frame |
//!
//! The timers themselves are [`Player::buffs`](crate::game::player::Buffs) —
//! plain simulation-second countdowns on the player, which the facade already
//! round-trips every tick (`World::player`), so the HUD can read them from
//! `&World` without a new resource, a spare twin or an accessor.
//!
//! ## Who reads what (the minimal hooks)
//!
//! - **Bottle Cap** → [`Player::take_damage`](crate::game::player::Player::take_damage),
//!   the single funnel every damage source goes through (enemy bullets,
//!   contact, mousetraps, boss hazards, boss projectiles, boss contact). No
//!   system needs a hook.
//! - **Wind-up Mouse** → one `Decoy` entity and a three-line branch in
//!   `enemy_move::enemy_steering_system`. Ordinary enemies within
//!   [`WINDUP_MOUSE_RADIUS`] that can *walk* to the key (no wall across the
//!   line) steer at it instead of the rat; elites and bosses keep their target.
//!   There is exactly one key at a time, so this costs one `Query` read and no
//!   flow-field rebuild: the key is a lure, not a second navigation problem.
//! - **Pocket Watch** → a multiplier on the *travel step* in
//!   `projectiles::bullets_system`, applied to hostile [`Bullet`]s only. Boss
//!   telegraph timers, hazard shapes and boss projectiles keep their own
//!   clocks, so every named attack warns for exactly its design time.
//! - **Clean Socks** → `traps::traps_system` skips the rat when a glue board
//!   looks for a victim (nothing else in the game slows the rat), plus a dash
//!   refill at collection time (`loot::pickups_system`).
//!
//! ## Determinism
//!
//! A run in which none of these pickups is collected carries four zeros, draws
//! no rng here and spawns no entity: the system returns before touching
//! anything. While a key is on the floor it reuses
//! [`vfx::emit_pickup_ambient`] (the same probabilistic sparkle every waiting
//! pickup already gets), and the shield rim / sock prints / expiry fizzles are
//! **rng-free** emitters (fixed counts and angles), so a twin run with the same
//! seed and inputs makes the same draws in the same order.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{
    DamageLedger, NextSpawnId, RngRes, RunCounters, Shake, SfxQueue, TickInput,
};
use crate::game::ecs::systems::boss::Boss;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::PickupKind;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;

/// Bottle Cap: how long the tin dome keeps its charge.
pub const BOTTLE_CAP_SECS: f32 = 12.0;

/// Wind-up Mouse: how long the brass key spins on the floor.
pub const WINDUP_MOUSE_SECS: f32 = 5.0;

/// Wind-up Mouse: how far an ordinary enemy can smell the key, in world px
/// (three 16 px tiles — the design's "within 48 px").
pub const WINDUP_MOUSE_RADIUS: f32 = 48.0;

/// Pocket Watch: how long hostile pellets crawl.
pub const POCKET_WATCH_SECS: f32 = 4.0;

/// Pocket Watch: hostile bullet travel multiplier while it runs (design: 40 %).
pub const POCKET_WATCH_BULLET_SLOW: f32 = 0.4;

/// Clean Socks: how long the rat ignores sticky-board drag.
pub const CLEAN_SOCKS_SECS: f32 = 6.0;

/// Growth Serum (batch 13): how long the rat stays giant.
pub const GROWTH_SECS: f32 = 20.0;

/// Growth Serum: full draw scale and uncapped hurt-radius multiplier.
pub const GIANT_SCALE: f32 = 5.0;

/// Growth Serum: seconds the rat takes to grow from 1x to [`GIANT_SCALE`]
/// (the draw pass ramps the block size over this window), and the seconds it
/// takes to shrink back at the end of the find.
pub const GIANT_GROW_SECS: f32 = 0.4;
pub const GIANT_SHRINK_SECS: f32 = 0.6;

/// Growth Serum: the last seconds of the find, during which the picture blinks
/// between [`GIANT_SCALE`] and [`GIANT_BLINK_SCALE`] as the warning that it is
/// about to end.
pub const GIANT_BLINK_SECS: f32 = 3.0;
pub const GIANT_BLINK_SCALE: f32 = 4.5;

/// Growth Serum's shared draw and size multiplier, if active.
///
/// Three phases, all read straight off the buff clock so there is no render
/// state to keep: the rat **grows** from 1x to [`GIANT_SCALE`] over
/// [`GIANT_GROW_SECS`], holds, then **blinks** between 5x and 4.5x
/// for the last [`GIANT_BLINK_SECS`] while it shrinks back over
/// [`GIANT_SHRINK_SECS`]. Never below 1x, so the first and last frames
/// of the find are exactly the rat the player always has.
pub fn giant_draw_scale(buff_left: f32) -> Option<f32> {
    if buff_left <= 0.0 {
        return None;
    }
    let elapsed = (GROWTH_SECS - buff_left).max(0.0);
    let grow = (elapsed / GIANT_GROW_SECS).clamp(0.0, 1.0);
    let shrink = (buff_left / GIANT_SHRINK_SECS).clamp(0.0, 1.0);
    let mut scale = GIANT_SCALE * grow.min(shrink).max(0.0);
    // The warning blink: about three blinks a second over the last seconds, at
    // the smaller of the two sizes so the picture never grows past its hold.
    if buff_left <= GIANT_BLINK_SECS {
        let blink = (buff_left * 6.0) as i32;
        if blink % 2 == 0 {
            scale = scale.min(GIANT_BLINK_SCALE);
        }
    }
    Some(scale.max(1.0))
}


/// Growth Serum: the radius (world px) of the stomp disc under the giant rat.
/// Six tiles wide — the rat's ordinary body is three pixels across.
pub const GIANT_STOMP_RADIUS: f32 = 30.0;

/// Growth Serum: the gap between two stomp beats, in seconds. One beat is one
/// shake, one dust ring and one thud, however many bodies it crushes.
pub const GIANT_STOMP_INTERVAL: f32 = 0.25;

/// Growth Serum: damage one stomp beat deals to an **elite** and to a named
/// miniboss or a grounded boss. Ordinary bodies are crushed outright.
pub const GIANT_ELITE_DAMAGE: f32 = 60.0;
pub const GIANT_BOSS_DAMAGE: f32 = 40.0;

/// Growth Serum: how much of an incoming hit still lands on the giant (see
/// [`crate::game::player::Buffs::damage_taken_scale`]).
pub const GIANT_DAMAGE_TAKEN: f32 = 0.5;

/// Growth Serum: the giant's walking speed multiplier. The dash is untouched.
pub const GIANT_MOVE_MULT: f32 = 0.9;

/// Growth Serum: the screen shake one crushing stomp beat adds (the camera
/// scales every amplitude down by `world::SHAKE_SCALE` already).
pub const GIANT_STOMP_SHAKE: f32 = 3.5;

pub const SHRINK_SECS: f32 = 12.0;
pub const SHRINK_SCALE: f32 = 0.35;
pub const SHRINK_RAMP_SECS: f32 = 0.3;
pub const SHRINK_BLINK_SECS: f32 = 2.0;
pub const SHRINK_BLINK_SCALE: f32 = 0.5;
pub const SHRINK_HURT_RADIUS: f32 = 1.5;
pub const SHRINK_MOVE_MULT: f32 = 1.15;
pub const WHITE_COAT_SECS: f32 = 5.0;
pub const COLLAR_SECS: f32 = 16.0;
pub const COLLAR_XP_MULT: f32 = 2.0;
pub const COLLAR_CHASE_MULT: f32 = 1.3;

pub const HEADLAMP_SECS: f32 = 18.0;
pub const HEADLAMP_RANGE: f32 = 128.0;
pub const HEADLAMP_HALF_ANGLE: f32 = 0.38;
pub const HEADLAMP_SHY_STRENGTH: f32 = 0.9;
pub const HEADLAMP_FADE_SECS: f32 = 3.0;

/// The lamp's ground-plane beam, shared by movement, fog and lighting.
#[derive(Clone, Copy)]
pub(crate) struct HeadlampBeam {
    pub origin: Vec2,
    pub dir: Vec2,
    slope: f32,
}

impl HeadlampBeam {
    pub fn from_player(
        level: &crate::game::level::Level,
        player: &Player,
        facing: Vec2,
    ) -> Option<Self> {
        if player.buffs.headlamp <= 0.0 || facing.len_sq() <= 1e-6 {
            return None;
        }
        let dir = facing.normalized();
        let head = player.pos + dir * (3.2 * player.buffs.draw_scale());
        // A giant or a rat facing a nearby wall can put its drawn head past
        // the wall face. Keep the ground-plane light on the near side.
        let origin = match super::projectiles::wall_hit(level, player.pos, head, 0.0) {
            Some(t) => player.pos + (head - player.pos) * (t - 0.01).max(0.0),
            None => head,
        };
        let (sin, cos) = crate::detmath::sin_cos(HEADLAMP_HALF_ANGLE);
        Some(Self { origin, dir, slope: sin / cos })
    }

    /// Relative strength at this point, with a signed side for the push.
    fn profile(self, level: &crate::game::level::Level, at: Vec2) -> Option<(f32, f32)> {
        let delta = at - self.origin;
        let along = delta.dot(self.dir);
        let side = delta.dot(self.dir.perp());
        let distance_sq = delta.len_sq();
        if along <= 0.0
            || side.abs() >= along * self.slope
            || distance_sq >= HEADLAMP_RANGE * HEADLAMP_RANGE
            || !super::projectiles::los_clear(level, self.origin, at, 0.0)
        {
            return None;
        }
        let angular = 1.0 - side.abs() / (along * self.slope);
        let radial = 1.0 - distance_sq.sqrt() / HEADLAMP_RANGE;
        Some((angular * radial, side))
    }

    pub fn reaches(self, level: &crate::game::level::Level, at: Vec2) -> bool {
        self.profile(level, at).is_some()
    }

    /// A lateral fraction of the body's effective gait. Use the spawn slot
    /// to break an exact centre-line tie so a stacked crowd takes both sides.
    pub fn shy(self, level: &crate::game::level::Level, enemy: &Enemy, spawn: u64) -> Vec2 {
        let Some((strength, side)) = self.profile(level, enemy.pos) else {
            return Vec2::ZERO;
        };
        let response = headlamp_response(enemy);
        if response == 0.0 {
            return Vec2::ZERO;
        }
        let sign = if side.abs() > 1e-3 {
            side.signum()
        } else if spawn.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        self.dir.perp() * (sign * strength * response * HEADLAMP_SHY_STRENGTH)
    }
}

/// A lamp never interrupts an attack whose geometry has already committed.
fn headlamp_response(e: &Enemy) -> f32 {
    if e.named
        || e.kind == EnemyKind::Boss
        || !e.targetable()
        || e.crab_pot_held
        || e.position_locked
        || e.w3.holds()
        || e.w3.bar_t > 0.0
        || (e.w4.holds(e.kind) && e.w4.phase != 0)
        || super::floor_sewer::leech_tongue(e).is_some()
        || super::floor_kitchen::egg_tell_of(e).is_some()
        || super::floor_kitchen::egg_ring_of(e).is_some()
        || super::floor_lab::syringe_ray(e).is_some()
        || super::floor_laundry::peg_holding(e)
        || super::floor_printworks::staple_warned(e)
        || super::floor_outfall::outfall_enemy_tell(e).is_some()
        || (e.kind == EnemyKind::Drake && e.w4.phase != super::drake::PHASE_WADDLE)
    {
        return 0.0;
    }
    if e.elite { 0.5 } else { 1.0 }
}

/// World px the rat walks between two little heel prints.
const SOCK_PRINT_SPACING: f32 = 9.0;

/// The brass key a Wind-up Mouse leaves on the floor.
///
/// It is an entity (not a resource) because it is drawn, culled and despawned
/// like everything else in the world, and because a repeat find has to replace
/// it at the new spot. `pos` lives in the component: nothing in this sim has a
/// transform.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Decoy {
    pub pos: Vec2,
    /// Seconds left before the key runs down.
    pub ttl: f32,
}

impl Decoy {
    /// A fresh key at `pos` (a full [`WINDUP_MOUSE_SECS`] of winding).
    pub fn new(pos: Vec2) -> Self {
        Self {
            pos,
            ttl: WINDUP_MOUSE_SECS,
        }
    }

    /// Still spinning? (The expiry tick despawns it through `Commands`.)
    #[inline]
    pub fn live(&self) -> bool {
        self.ttl > 0.0
    }
}

/// Does a decoy lure this body? Ordinary enemies only: elites and the full boss
/// keep the rat as their target (the design's rule), which is what makes the
/// key a crowd tool instead of a boss exploit.
#[inline]
pub fn decoy_lures(elite: bool, kind: EnemyKind) -> bool {
    !elite && kind != EnemyKind::Boss
}

/// [`decoy_lures`] for a live body: a named miniboss (an ordinary-kind body
/// with `Enemy::named`) keeps the rat as its target exactly like an elite.
#[inline]
pub fn decoy_lures_body(e: &crate::game::enemy::Enemy) -> bool {
    !e.named && decoy_lures(e.elite, e.kind)
}

/// Spawn a live key at `pos` (the Wind-up Mouse's floor decoy).
///
/// Systems spawn this through `Commands` from `loot::pickups_system`; tests and
/// setup use this helper so they get a real `SpawnId` (the entity draw order
/// depends on it).
pub fn spawn_decoy(world: &mut crate::game::world::World, pos: Vec2) -> Entity {
    let id = world.ecs.resource_mut::<NextSpawnId>().take();
    world.ecs.spawn((Decoy::new(pos), id)).id()
}

/// Age every wave-3 pocket-find timer and keep its floor/rat cues alive.
///
/// Runs first in the tick (before `traps_system`), so every reader in the tick
/// sees the aged value: a buff that runs out this tick is already gone.
pub fn powerups_system(
    input: Res<TickInput>,
    counters: Res<RunCounters>,
    mut player: ResMut<Player>,
    mut commands: Commands,
    mut decoys: Query<(Entity, &mut Decoy)>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut next_id: ResMut<NextSpawnId>,
    mut walked: Local<f32>,
    mut last_pos: Local<Option<Vec2>>,
) {
    let dt = input.dt;
    let ppos = player.pos;

    // 1. Buff countdowns. `expired` is what the player *lost* this tick, for
    //    the quiet release cue.
    let expired = player.buffs.tick(dt);
    if expired[0] {
        vfx::emit_buff_expire(&mut particles, ppos, PickupKind::BottleCap);
    }
    if expired[1] {
        vfx::emit_buff_expire(&mut particles, ppos, PickupKind::PocketWatch);
    }
    if expired[2] {
        vfx::emit_buff_expire(&mut particles, ppos, PickupKind::CleanSocks);
        *walked = 0.0;
        *last_pos = None;
    }
    if expired[4] {
        vfx::emit_buff_expire(&mut particles, ppos, PickupKind::GrowthSerum);
    }
    for (did_expire, kind) in [
        (expired[5], PickupKind::ShrinkDraught),
        (expired[6], PickupKind::WhiteCoat),
        (expired[7], PickupKind::TheCollar),
        (expired[8], PickupKind::Headlamp),
    ] {
        if did_expire {
            vfx::emit_buff_expire(&mut particles, ppos, kind);
        }
    }

    // 2. Materialise a Wind-up Mouse collected since the last tick: move the
    //    one key if it is already on the floor (a repeat find replaces it at
    //    the new spot) or spawn it. Keeping this here rather than in the
    //    collection pass means one entity owns the key for its whole life:
    //    spawned, aged and despawned by the same system.
    //    A key spawned this tick is only *visible* through the deferred command
    //    queue on the next tick, so the freshly armed countdown is carried in
    //    `decoy_left` by hand instead of being read back from the component.
    let mut decoy_left = 0.0f32;
    if let Some(at) = player.decoy_at.take() {
        // At most one key exists, so its component is either replaced or the
        // key is spawned.
        if let Some((_entity, mut decoy)) = decoys.iter_mut().next() {
            *decoy = Decoy::new(at);
        } else {
            let id = next_id.take();
            commands.spawn((Decoy::new(at), id));
        }
        decoy_left = player.buffs.decoy;
    }

    // 3. The key's own clock owns the decoy timer; mirror it for the HUD.
    for (entity, mut decoy) in decoys.iter_mut() {
        decoy.ttl -= dt;
        if !decoy.live() {
            commands.entity(entity).despawn();
            vfx::emit_buff_expire(&mut particles, decoy.pos, PickupKind::WindupMouse);
            continue;
        }
        decoy_left = decoy_left.max(decoy.ttl);
        // Reuses the waiting-pickup sparkle, so the spinning key reads as a
        // findable object on the floor (the Neon pass replaces the sprite).
        vfx::emit_pickup_ambient(
            &mut particles,
            &mut rng.0,
            decoy.pos,
            PickupKind::WindupMouse,
            dt,
        );
    }
    player.buffs.decoy = decoy_left;

    // 4. The Bottle Cap's charge is visible: a white rim that turns once every
    //    two seconds around the rat.
    if player.buffs.shield > 0.0 {
        let clock = counters.elapsed;
        vfx::emit_shield_rim(&mut particles, ppos, clock);
    }

    // 5. Clean Socks leave little heel prints behind the walking rat.
    if player.buffs.socks > 0.0 {
        if let Some(last) = *last_pos {
            *walked += last.dist(ppos);
            if *walked >= SOCK_PRINT_SPACING {
                *walked = 0.0;
                let face = player.face;
                vfx::emit_sock_print(&mut particles, ppos, face);
            }
        }
        *last_pos = Some(ppos);
    } else {
        *walked = 0.0;
        *last_pos = None;
    }
}

/// Startup sanity: the lure radius and the slow factor are sane.
const _: () = {
    assert!(POCKET_WATCH_BULLET_SLOW > 0.0 && POCKET_WATCH_BULLET_SLOW < 1.0);
    assert!(WINDUP_MOUSE_RADIUS >= 3.0 * 16.0);
    assert!(GIANT_SCALE > 1.0 && GIANT_STOMP_RADIUS > 16.0);
    assert!(GIANT_DAMAGE_TAKEN > 0.0 && GIANT_DAMAGE_TAKEN < 1.0);
};

/// Does one stomp beat crush this body outright?
///
/// Only the **ordinary** horde: an elite (the floor's heavy wearing its gild),
/// a named miniboss and a boss each take a fixed chunk and keep fighting — an
/// appointment is a promised fight, not a pickup's footnote.
#[inline]
pub fn giant_crushes(e: &Enemy) -> bool {
    !e.elite && !e.named && e.kind != EnemyKind::Boss
}

/// **Growth Serum's stomp**: the giant body is a damage disc, and every
/// [`GIANT_STOMP_INTERVAL`] it crushes whatever is standing in it.
///
/// Runs right after `combat::contact_system` and before the reap
/// (`schedule.rs`), so a body the stomp kills this tick is despawned, drops its
/// gem, gold and rare pickup, and counts in `RunCounters` on the same tick —
/// exactly like a trap kill. Nothing here spawns an entity or draws rng:
/// * the beat is a `Local` countdown, so the stomp cannot shift the
///   simulation's stream;
/// * the dust ring ([`vfx::emit_giant_stomp`]) is a fixed-shape emitter.
///
/// Rules (batch 13):
/// * ordinary body inside the disc → crushed outright (`hp = -1`, the same
///   path a Nuke takes for fodder) and reaped by the schedule's normal reap;
/// * elite → [`GIANT_ELITE_DAMAGE`] through `hurt_from`, so the Pillbug's shell
///   and every other directional rule still applies;
/// * named miniboss → [`GIANT_BOSS_DAMAGE`] the same way;
/// * boss → [`GIANT_BOSS_DAMAGE`] but only while a **grounded** hit shape is in
///   the disc (a leaping jaguar and a hawk overhead are not stomped).
///
/// Every hit books the Gnaw column of `DamageLedger`: the ledger's columns are
/// weapon indices and it has no free-form source, so the stomp is credited to
/// the rat's own bite rather than inventing a sixteenth column (a batch-harness
/// reader would have to know it anyway). The floating number is
/// `DamageSource::Normal`, the same read as an untyped hit.
pub fn giant_stomp_system(
    input: Res<TickInput>,
    player: Res<Player>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut ledger: ResMut<DamageLedger>,
    mut beat: Local<f32>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    // Not giant: the beat is parked at zero, so a find always opens on a fresh
    // stomp (the first crush lands on the tick the serum's first beat is up).
    if !player.buffs.is_giant() || player.buffs.weapons_suspended() {
        *beat = 0.0;
        return;
    }
    *beat += input.dt;
    if *beat < GIANT_STOMP_INTERVAL {
        return;
    }
    *beat -= GIANT_STOMP_INTERVAL;
    if *beat < 0.0 {
        *beat = 0.0;
    }

    let ppos = player.pos;
    let reach = GIANT_STOMP_RADIUS;
    let mut crushed = false;

    // `SpawnId` order decides everything here (which body's kill the reap folds
    // first, which damage numbers are recorded in which order), never the
    // query's storage order.
    order.clear();
    order.extend(enemies.iter().map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|&(id, _)| id);
    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || !grounded_body(e.kind) {
            continue;
        }
        if !crate::game::collision::circles_overlap(ppos, reach, e.pos, e.radius) {
            continue;
        }
        let pos = e.pos;
        let radius = e.radius;
        let id = e.id;
        if giant_crushes(&e) {
            // Crushed outright: the same damage path a Nuke uses on fodder, so
            // the reap below pays the kill, its gem, its gold and its rare drop.
            let applied = e.hp.max(1.0);
            e.hp = -1.0;
            e.flash = 0.12;
            e.note_damage();
            ledger.add(WeaponKind::Gnaw, applied);
            damage_numbers.record(id, pos, radius, applied, true, DamageSource::Normal);
        } else {
            let dmg = if e.elite {
                GIANT_ELITE_DAMAGE
            } else {
                GIANT_BOSS_DAMAGE
            };
            let applied = e.hurt_from(dmg, ppos);
            let killed = !e.alive();
            ledger.add(WeaponKind::Gnaw, applied);
            damage_numbers.record(id, pos, radius, applied, killed, DamageSource::Normal);
        }
        crushed = true;
    }

    // The full boss is not an `Enemy`: it takes the same chunk through its own
    // art-defined **grounded** hit shapes, so a leaping jaguar or a hawk on the
    // wing cannot be stomped out of the air.
    for mut boss in bosses.iter_mut() {
        if boss.dead || boss.is_airborne() {
            continue;
        }
        if !boss.hit_shapes().circle_hits_grounded(ppos, reach) {
            continue;
        }
        // The same per-state and shell multipliers every other boss hit takes.
        let amount =
            GIANT_BOSS_DAMAGE * boss.damage_mult * boss.incoming_multiplier(Vec2::ZERO, ppos);
        if !boss.take_damage(amount) {
            continue;
        }
        ledger.add_boss(WeaponKind::Gnaw, amount);
        let killed = boss.hp <= 0.0;
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            killed,
            DamageSource::Normal,
        );
        crushed = true;
    }

    if crushed {
        // One beat, one read: a small shudder, a ring of dust and a soft thud
        // (`Sfx::Bump` is the game's existing floor-thud sample).
        shake.amount = shake.amount.max(GIANT_STOMP_SHAKE);
        vfx::emit_giant_stomp(&mut particles, ppos);
        sfx.0.push(Sfx::Bump);
    }
}

/// Which bodies a stomp can reach at all: only a creature standing on the
/// floor. The flyers pass over the giant's feet, exactly as they pass over a
/// glue board or a mousetrap's striker bar (`traps::is_ground`'s rule, read
/// through the archetype so a registered kind flies exactly as its base does).
#[inline]
fn grounded_body(kind: EnemyKind) -> bool {
    !matches!(
        kind.archetype(),
        EnemyKind::Bat | EnemyKind::WickMoth | EnemyKind::MurderHornet
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::bullet::{Bullet, Faction};
    use crate::game::ecs::components::SpawnId;
    use crate::game::ecs::resources::{RngRes, SfxQueue};
    use crate::game::ecs::systems::boss::{
        resolve_hazards, Boss, BossHazard, BossKind, HazardShape,
    };
    use crate::game::ecs::systems::{combat, enemy_move, loot, projectiles, traps};
    use crate::game::enemy::Enemy;
    use crate::game::gem::{Pickup, PickupKind};
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::particle::damage_number::DamageNumbers;
    use crate::game::particle::Particles;
    use crate::game::world::World;
    use crate::math::Vec2;

    const DT: f32 = 1.0 / 60.0;

    /// One `powerups_system` tick through the facade round-trip.
    fn buff_tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(powerups_system)
            .expect("powerups system must run");
        w.end_tick();
    }

    /// Collect one pickup of `kind` at the rat's feet through the real loot
    /// path, so every collection effect is exercised as the game does it.
    fn collect(w: &mut World, kind: PickupKind) {
        let ppos = w.player.pos;
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind,
            bob: 0.0,
        });
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(loot::pickups_system)
            .expect("pickups run");
        w.end_tick();
    }

    /// A world with no walls and no generated traps, so a failure is the
    /// mechanic's and not the map's.
    fn open_arena(seed: u64) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        w.level.tiles =
            crate::game::collision::TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.level.objects.sticky_traps.clear();
        w.level.objects.mousetraps.clear();
        w.player.pos = w.level.arena * 0.5;
        w
    }

    fn decoy_pos(w: &mut World) -> Option<Vec2> {
        let mut q = w.ecs.query::<&Decoy>();
        q.iter(&w.ecs).next().map(|d| d.pos)
    }

    fn decoy_count(w: &mut World) -> usize {
        let mut q = w.ecs.query::<&Decoy>();
        q.iter(&w.ecs).count()
    }

    fn enemy_pos(w: &mut World, e: Entity) -> Vec2 {
        let mut q = w.ecs.query::<&Enemy>();
        q.get(&w.ecs, e).expect("enemy alive").pos
    }

    #[test]
    fn new_pickup_timers_refresh_and_expire_on_simulation_time() {
        use crate::game::player::Buffs;

        let mut w = open_arena(719);
        type BuffTimer = fn(&Buffs) -> f32;
        let cases: [(PickupKind, f32, BuffTimer); 4] = [
            (PickupKind::ShrinkDraught, SHRINK_SECS, |b| b.tiny),
            (PickupKind::WhiteCoat, WHITE_COAT_SECS, |b| b.white_coat),
            (PickupKind::TheCollar, COLLAR_SECS, |b| b.collar),
            (PickupKind::Headlamp, HEADLAMP_SECS, |b| b.headlamp),
        ];
        for (kind, secs, left) in cases {
            collect(&mut w, kind);
            assert_eq!(left(&w.player.buffs), secs);
            buff_tick(&mut w);
            assert!((left(&w.player.buffs) - (secs - DT)).abs() < 1e-5);
            collect(&mut w, kind);
            assert_eq!(left(&w.player.buffs), secs);
            for dt in [secs - 0.125, 0.125] {
                assert!(left(&w.player.buffs) > 0.0);
                w.begin_tick(dt, Vec2::ZERO, false);
                w.ecs.run_system_cached(powerups_system).unwrap();
                w.end_tick();
            }
            assert_eq!(left(&w.player.buffs), 0.0);
        }
        assert!(!w.player.buffs.any());
    }

    #[test]
    fn shrink_draught_avoids_grazes_but_not_direct_hits() {
        for shrunk in [false, true] {
            let mut w = open_arena(720);
            if shrunk {
                collect(&mut w, PickupKind::ShrinkDraught);
            }
            let p = w.player.pos;
            let enemy = w.spawn_enemy(EnemyKind::Brute, p, 1.0);
            let mut body = w.ecs.get_mut::<Enemy>(enemy).unwrap();
            body.pos = p + Vec2::new(body.radius + 2.25, 0.0);
            let graze = Bullet::new(
                p + Vec2::new(0.0, 4.25),
                Vec2::ZERO,
                20.0,
                Faction::Enemy,
                (255, 0, 0),
            );
            w.spawn_bullet(graze);
            w.player.iframes = 0.0;
            let hp = w.player.hp;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(combat::contact_system).unwrap();
            w.end_tick();
            assert_eq!(w.player.hp == hp, shrunk, "contact graze");

            w.player.iframes = 0.0;
            let hp = w.player.hp;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(combat::enemy_bullet_hits_system).unwrap();
            w.end_tick();
            assert_eq!(w.player.hp == hp, shrunk, "pellet graze");

            w.clear_bullets();
            w.spawn_bullet(Bullet::new(p, Vec2::ZERO, 20.0, Faction::Enemy, (255, 0, 0)));
            w.player.iframes = 0.0;
            let hp = w.player.hp;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(combat::enemy_bullet_hits_system).unwrap();
            w.end_tick();
            assert!(w.player.hp < hp, "a direct pellet must still land");
        }
    }

    #[test]
    fn shrink_draught_speeds_movement_and_preserves_weapon_output() {
        use crate::game::ecs::systems::{player, weapons};
        use crate::game::weapon::{Weapon, WeaponKind};

        let trial = |shrunk| {
            let mut w = open_arena(721);
            if shrunk {
                collect(&mut w, PickupKind::ShrinkDraught);
            }
            let mut spray = Weapon::new(WeaponKind::CheeseSpray);
            spray.timer = 0.0;
            w.player.loadout.weapons = vec![spray];
            w.begin_tick(DT, Vec2::new(1.0, 0.0), false);
            w.ecs.run_system_cached(player::player_system).unwrap();
            w.ecs.run_system_cached(weapons::fire_weapons_system).unwrap();
            w.end_tick();
            let shots = w.bullets().iter().map(|b| (b.damage, b.vel, b.life)).collect::<Vec<_>>();
            (w.player.vel.x, shots)
        };
        let (plain_speed, plain_shots) = trial(false);
        let (tiny_speed, tiny_shots) = trial(true);
        assert!(plain_speed > 0.0);
        assert!((tiny_speed - plain_speed * SHRINK_MOVE_MULT).abs() < 1e-4);
        assert!(!plain_shots.is_empty());
        assert_eq!(tiny_shots, plain_shots);
    }

    #[test]
    fn white_coat_blocks_incoming_hits_and_suspends_all_weapon_triggers() {
        use crate::game::bullet::Orbit;
        use crate::game::ecs::resources::{ActiveFx, LaserFx};
        use crate::game::ecs::systems::{player, weapons, weapons_wave3b};
        use crate::game::weapon::{Weapon, WeaponKind};

        let mut w = open_arena(722);
        collect(&mut w, PickupKind::WhiteCoat);
        w.player.buffs.shield = BOTTLE_CAP_SECS;
        w.player.iframes = 0.0;
        let hp = w.player.hp;
        assert!(!w.player.take_damage(20.0));
        assert_eq!(w.player.hp, hp);
        assert_eq!(w.player.buffs.shield, BOTTLE_CAP_SECS);
        assert_eq!(w.player.iframes, 0.0);

        w.player.loadout.weapons = [
            WeaponKind::CheeseSpray,
            WeaponKind::TailThrash,
            WeaponKind::LaserEyes,
            WeaponKind::SuperDash,
        ]
        .into_iter()
        .map(|kind| Weapon { timer: 0.0, ..Weapon::new(kind) })
        .collect();
        let p = w.player.pos;
        let enemy = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 10.0);
        let target_hp = w.ecs.get::<Enemy>(enemy).unwrap().hp;
        let mut spore = Bullet::new(p, Vec2::ZERO, 10.0, Faction::Player, (255, 0, 0));
        spore.orbit = Some(Orbit { angle: 0.0, radius: 10.0, speed: 1.0 });
        w.spawn_bullet(spore);
        let trigger = |w: &mut World, dash| {
            w.begin_tick_with(DT, Vec2::new(1.0, 0.0), dash, true);
            w.ecs.run_system_cached(player::player_system).unwrap();
            w.ecs.run_system_cached(weapons::fire_weapons_system).unwrap();
            w.ecs.run_system_cached(weapons::active_weapons_system).unwrap();
            w.ecs.run_system_cached(weapons_wave3b::laser_eyes_system).unwrap();
            w.ecs.run_system_cached(weapons_wave3b::super_dash_system).unwrap();
            w.ecs.run_system_cached(projectiles::bullets_system).unwrap();
            w.end_tick();
        };
        trigger(&mut w, true);
        assert!(w.player.dash_active > 0.0, "the ordinary dash is available");
        assert!(w.bullets().is_empty());
        assert_eq!(w.ecs.resource::<ActiveFx>().thrash_left, 0.0);
        assert!(!w.ecs.resource::<LaserFx>().active());
        assert_eq!(w.ecs.get::<Enemy>(enemy).unwrap().hp, target_hp);

        w.player.buffs.white_coat = DT;
        buff_tick(&mut w);
        w.player.iframes = 0.0;
        assert!(w.player.take_damage(20.0), "the Coat has expired");
        assert_eq!(w.player.buffs.shield, 0.0, "the Cap takes the next hit");
        trigger(&mut w, false);
        assert!(!w.bullets().is_empty(), "auto-fire returns after expiry");
        assert!(w.ecs.resource::<ActiveFx>().thrash_left > 0.0);
        assert!(w.ecs.resource::<LaserFx>().active());
    }

    #[test]
    fn collar_doubles_gem_xp_and_hastens_only_elites_and_the_miniboss() {
        use crate::game::ecs::systems::miniboss::MinibossKind;
        use crate::game::gem::Gem;

        let trial = |collared| {
            let mut w = open_arena(723);
            if collared {
                collect(&mut w, PickupKind::TheCollar);
            }
            let p = w.player.pos;
            let at = p + Vec2::new(40.0, 0.0);
            let ordinary = w.spawn_enemy(EnemyKind::Brute, at, 1.0);
            let elite = w.spawn_enemy(EnemyKind::Brute, at, 1.0);
            w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
            let miniboss = w.spawn_miniboss(MinibossKind::Corkscrew, 0, p + Vec2::new(100.0, 0.0));
            w.spawn_gem(Gem::new(p, 2));
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(super::super::miniboss::miniboss_system).unwrap();
            w.ecs.run_system_cached(enemy_move::enemy_steering_system).unwrap();
            w.ecs.run_system_cached(loot::gems_system).unwrap();
            w.end_tick();
            let speeds = [ordinary, elite, miniboss]
                .map(|e| w.ecs.get::<Enemy>(e).unwrap().vel.len());
            (w.player.xp, speeds)
        };
        let (xp, [normal, elite, miniboss]) = trial(false);
        let (collared_xp, [collared_normal, collared_elite, collared_miniboss]) = trial(true);
        assert_eq!((xp, collared_xp), (2, 4));
        assert!(normal > 0.0 && elite > 0.0 && miniboss > 0.0);
        assert_eq!(collared_normal, normal);
        assert!((collared_elite - elite * COLLAR_CHASE_MULT).abs() < 1e-4);
        assert!((collared_miniboss - miniboss * COLLAR_CHASE_MULT).abs() < 1e-4);
    }

    // ---- Bottle Cap ------------------------------------------------------

    /// The cap grants a 12 s charge, and a repeat find refreshes it.
    #[test]
    fn bottle_cap_charges_for_twelve_seconds_and_refreshes() {
        let mut w = open_arena(201);
        collect(&mut w, PickupKind::BottleCap);
        assert!((w.player.buffs.shield - BOTTLE_CAP_SECS).abs() < 1e-6);

        // Ten seconds later: half spent, still charged. A second find restores
        // the full twelve (repeat finds refresh, never stack).
        for _ in 0..600 {
            buff_tick(&mut w);
        }
        let left = w.player.buffs.shield;
        assert!(
            (left - (BOTTLE_CAP_SECS - 600.0 * DT)).abs() < 0.05,
            "left {left}"
        );
        collect(&mut w, PickupKind::BottleCap);
        assert!(
            (w.player.buffs.shield - BOTTLE_CAP_SECS).abs() < 1e-6,
            "a second cap must refresh to 12 s, not stack"
        );

        // It runs out on its own; nothing is left behind.
        for _ in 0..800 {
            buff_tick(&mut w);
        }
        assert_eq!(w.player.buffs.shield, 0.0, "the cap should expire");
        assert!(w.sfx.contains(&crate::audio::Sfx::PocketFind));
    }

    /// A charged cap absorbs exactly one hit, whatever its size, and the same
    /// hit still opens the normal post-hit immunity window.
    #[test]
    fn a_charge_absorbs_exactly_one_hit_and_still_grants_iframes() {
        let mut w = open_arena(203);
        collect(&mut w, PickupKind::BottleCap);
        let hp = w.player.hp;
        assert!(w.player.take_damage(25.0), "a blocked hit still 'lands'");
        assert_eq!(w.player.hp, hp, "the cap must absorb the damage");
        assert_eq!(w.player.buffs.shield, 0.0, "the charge is spent");
        assert!(w.player.iframes > 0.0, "a blocked hit still grants i-frames");
        assert!(!w.player.take_damage(25.0), "i-frames block the follow-up");

        // Once the i-frame window is over the next hit bites normally.
        w.player.iframes = 0.0;
        assert!(w.player.take_damage(25.0));
        assert!(w.player.hp < hp, "the second hit must land");
    }

    /// An expired cap does not absorb anything.
    #[test]
    fn an_expired_cap_blocks_nothing() {
        let mut w = open_arena(205);
        collect(&mut w, PickupKind::BottleCap);
        for _ in 0..800 {
            buff_tick(&mut w);
        }
        assert_eq!(w.player.buffs.shield, 0.0);
        let hp = w.player.hp;
        assert!(w.player.take_damage(10.0));
        assert!(w.player.hp < hp, "an expired cap absorbed a hit");
    }

    /// A tiny system wrapper so the boss hazard resolver (a `pub fn` taking a
    /// live `Query`) can be driven from a narrow test.
    #[derive(Resource, Default)]
    struct HazardProbe {
        boss: Option<Boss>,
        hit: bool,
    }

    fn hazard_probe_system(
        mut probe: ResMut<HazardProbe>,
        level: Res<crate::game::level::Level>,
        mut player: ResMut<Player>,
        mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
        mut particles: ResMut<Particles>,
        mut numbers: ResMut<DamageNumbers>,
        mut rng: ResMut<RngRes>,
        mut sfx: ResMut<SfxQueue>,
        mut order: Local<Vec<(u64, Entity)>>,
    ) {
        let Some(mut boss) = probe.boss.take() else {
            return;
        };
        probe.hit |= resolve_hazards(
            &level,
            &mut boss,
            DT,
            &mut player,
            &mut enemies,
            &mut order,
            &mut particles,
            &mut numbers,
            &mut rng.0,
            &mut sfx,
        );
        probe.boss = Some(boss);
    }

    /// The shield stops **every** damage source, because every source goes
    /// through `Player::take_damage`:
    ///
    /// - an enemy bullet (`combat::enemy_bullet_hits_system`),
    /// - contact (`combat::contact_system`),
    /// - a mousetrap snap (`traps::traps_system`),
    /// - a boss hazard (`boss::resolve_hazards`).
    ///
    /// Boss *contact* and boss projectiles go through `player.take_damage(...)`
    /// in that same boss system, so the one-charge rule covers them too.
    #[test]
    fn the_shield_blocks_a_hit_from_every_source() {
        // 1. Enemy bullet.
        let mut w = open_arena(207);
        collect(&mut w, PickupKind::BottleCap);
        w.clear_bullets();
        w.player.iframes = 0.0;
        let ppos = w.player.pos;
        w.spawn_bullet(Bullet::new(
            ppos,
            Vec2::ZERO,
            6.0,
            Faction::Enemy,
            (255, 0, 0),
        ));
        let hp = w.player.hp;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(combat::enemy_bullet_hits_system)
            .expect("bullets run");
        w.end_tick();
        assert_eq!(w.player.hp, hp, "the cap must absorb an enemy bullet");
        assert_eq!(w.player.buffs.shield, 0.0);
        assert_eq!(
            w.bullets()[0].life,
            0.0,
            "a blocked bullet is still consumed"
        );

        // 2. Contact.
        let mut w = open_arena(209);
        collect(&mut w, PickupKind::BottleCap);
        w.player.iframes = 0.0;
        let ppos = w.player.pos;
        w.clear_enemies();
        w.spawn_enemy(EnemyKind::Brute, ppos, 1.0);
        let hp = w.player.hp;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(combat::contact_system)
            .expect("contact runs");
        w.end_tick();
        assert_eq!(w.player.hp, hp, "the cap must absorb contact damage");

        // 3. A mousetrap's striker bar.
        let mut w = open_arena(211);
        collect(&mut w, PickupKind::BottleCap);
        w.player.iframes = 0.0;
        let ppos = w.player.pos;
        w.level.objects.mousetraps.push(
            crate::game::art_traps::MousetrapView::new(
                ppos + Vec2::new(-8.0, 0.0),
                Vec2::new(1.0, 0.0),
            ),
        );
        let hp = w.player.hp;
        for _ in 0..60 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(traps::traps_system)
                .expect("traps run");
            w.end_tick();
            if w.player.buffs.shield == 0.0 {
                break;
            }
        }
        assert_eq!(w.player.buffs.shield, 0.0, "the trap never snapped");
        assert_eq!(w.player.hp, hp, "the cap must absorb the mousetrap");

        // 4. A boss hazard.
        let mut w = open_arena(213);
        collect(&mut w, PickupKind::BottleCap);
        w.player.iframes = 0.0;
        let ppos = w.player.pos;
        let mut boss = Boss::new(
            BossKind::Cat,
            ppos + Vec2::new(60.0, 0.0),
            5000.0,
            crate::game::level::TileRect::new(0, 0, 400, 400),
            ppos,
            false,
        );
        boss.hazards
            .push(BossHazard::new(
                HazardShape::Circle {
                    center: ppos,
                    radius: 20.0,
                },
                0.0,
                1.0,
                20.0,
            ));
        w.ecs.insert_resource(HazardProbe {
            boss: Some(boss),
            hit: false,
        });
        let hp = w.player.hp;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(hazard_probe_system)
            .expect("hazard probe runs");
        let hit = w.ecs.resource::<HazardProbe>().hit;
        w.end_tick();
        assert!(hit, "the hazard must register the contact");
        assert_eq!(w.player.hp, hp, "the cap must absorb a boss hazard");
        assert_eq!(w.player.buffs.shield, 0.0);
        assert!(w.sfx.contains(&crate::audio::Sfx::Hurt), "hurt feedback");
    }

    // ---- Wind-up Mouse ---------------------------------------------------

    /// The key lasts five seconds, mirrors into the HUD timer and is despawned
    /// when it runs down.
    #[test]
    fn the_windup_mouse_spins_for_five_seconds() {
        let mut w = open_arena(221);
        collect(&mut w, PickupKind::WindupMouse);
        // The collection pass only files the request; the tick's
        // `powerups_system` spawns the key.
        buff_tick(&mut w);
        assert!(decoy_pos(&mut w).is_some(), "no key on the floor");
        assert!((w.player.buffs.decoy - WINDUP_MOUSE_SECS).abs() < 0.1);

        // Count the ticks the key is actually on the floor: it must survive
        // close to five seconds and then be despawned (one entity, one clock).
        let mut lived = 0usize;
        for _ in 0..(WINDUP_MOUSE_SECS / DT) as usize + 10 {
            if decoy_pos(&mut w).is_none() {
                break;
            }
            buff_tick(&mut w);
            lived += 1;
        }
        assert!(decoy_pos(&mut w).is_none(), "the key never wound down");
        let secs = lived as f32 * DT;
        assert!(
            (secs - WINDUP_MOUSE_SECS).abs() <= 3.0 * DT,
            "the key spun for {secs} s, expected {WINDUP_MOUSE_SECS} s"
        );
        assert_eq!(w.player.buffs.decoy, 0.0);
    }

    /// A second mouse replaces the key at the new spot and refreshes the timer.
    #[test]
    fn a_second_mouse_moves_and_refreshes_the_key() {
        let mut w = open_arena(223);
        collect(&mut w, PickupKind::WindupMouse);
        for _ in 0..120 {
            buff_tick(&mut w);
        }
        let before = w.player.buffs.decoy;
        assert!(before < WINDUP_MOUSE_SECS - 1.5, "setup: {before}");
        w.player.pos += Vec2::new(40.0, 0.0);
        let spot = w.player.pos;
        collect(&mut w, PickupKind::WindupMouse);
        buff_tick(&mut w);
        assert!((w.player.buffs.decoy - WINDUP_MOUSE_SECS).abs() < 0.1);
        let at = decoy_pos(&mut w).expect("one key");
        assert!(at.dist(spot) < 1.0, "the key did not move: {at:?}");
        assert_eq!(decoy_count(&mut w), 1, "a second key must replace the first");
    }

    /// Ordinary enemies in range chase the key; elites and the boss keep the
    /// rat as their target.
    #[test]
    fn the_key_lures_ordinary_enemies_only() {
        let mut w = open_arena(227);
        let ppos = Vec2::new(120.0, 120.0);
        w.player.pos = ppos;
        w.clear_enemies();
        let key = ppos + Vec2::new(20.0, 0.0);
        spawn_decoy(&mut w, key);
        // Ordinary, elite and boss bodies all ~20 px west of the rat, all
        // inside the key's smell radius.
        let ordinary = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(-20.0, 0.0), 1.0);
        let elite = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(-20.0, 20.0), 1.0);
        w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
        let boss = w.spawn_enemy(EnemyKind::Boss, ppos + Vec2::new(-20.0, -20.0), 1.0);

        let (o0, e0, b0) = (
            enemy_pos(&mut w, ordinary),
            enemy_pos(&mut w, elite),
            enemy_pos(&mut w, boss),
        );
        for _ in 0..40 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(powerups_system)
                .expect("powerups run");
            w.ecs
                .run_system_cached(enemy_move::enemy_steering_system)
                .expect("steering runs");
            w.end_tick();
        }
        let (o1, e1, b1) = (
            enemy_pos(&mut w, ordinary),
            enemy_pos(&mut w, elite),
            enemy_pos(&mut w, boss),
        );
        assert!(
            o1.dist(key) < o0.dist(key) - 4.0,
            "the ordinary body ignored the key: {} -> {}",
            o0.dist(key),
            o1.dist(key)
        );
        assert!(
            e1.dist(ppos) < e0.dist(ppos) - 3.0,
            "an elite must keep chasing the rat"
        );
        assert!(
            b1.dist(ppos) < b0.dist(ppos) - 3.0,
            "the boss must keep chasing the rat"
        );
    }

    /// Out of range, the key is invisible: the body keeps walking the rat.
    #[test]
    fn a_key_out_of_range_does_not_lure() {
        let mut w = open_arena(229);
        let ppos = Vec2::new(120.0, 120.0);
        w.player.pos = ppos;
        w.clear_enemies();
        let key = ppos + Vec2::new(WINDUP_MOUSE_RADIUS + 40.0, 0.0);
        spawn_decoy(&mut w, key);
        // The body sits between rat and key, beyond the smell radius of both.
        let e = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(30.0, 0.0), 1.0);
        let before = enemy_pos(&mut w, e);
        for _ in 0..30 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(powerups_system)
                .expect("powerups run");
            // The flow field was built for the generated walls this fixture
            // replaced: refresh it for the open map.
            w.ecs
                .run_system_cached(enemy_move::flow_refresh_system)
                .expect("flow refresh runs");
            w.ecs
                .run_system_cached(enemy_move::enemy_steering_system)
                .expect("steering runs");
            w.end_tick();
        }
        let after = enemy_pos(&mut w, e);
        assert!(
            after.dist(ppos) < before.dist(ppos),
            "the body did not walk at the rat"
        );
        assert!(
            after.dist(key) > before.dist(key) - 1.0,
            "an out-of-range key lured the body"
        );
    }

    /// A wall between the body and the key stops the lure: the ordinary body
    /// keeps walking the rat instead of grinding into masonry.
    #[test]
    fn the_key_does_not_lure_through_a_wall() {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::TileKind;
        let mut w = open_arena(231);
        // A solid block between the body (west) and the key (east).
        w.level
            .tiles
            .fill_cells(7, 7, 1, 1, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        let ppos = Vec2::new(100.0, 120.0);
        w.player.pos = ppos;
        w.clear_enemies();
        let body = Vec2::new(108.0, 120.0);
        let key = Vec2::new(140.0, 120.0);
        spawn_decoy(&mut w, key);
        let e = w.spawn_enemy(EnemyKind::Skitterer, body, 1.0);
        for _ in 0..30 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(powerups_system)
                .expect("powerups run");
            w.ecs
                .run_system_cached(enemy_move::enemy_steering_system)
                .expect("steering runs");
            w.end_tick();
        }
        let pos = enemy_pos(&mut w, e);
        assert!(
            pos.dist(ppos) < body.dist(ppos),
            "a body behind a wall was lured through it at {pos:?}"
        );
    }

    // ---- Pocket Watch ----------------------------------------------------

    /// Hostile bullets travel at 40 % while the watch runs — and only hostile
    /// bullets.
    #[test]
    fn the_pocket_watch_slows_hostile_pellets_only() {
        let mut w = open_arena(233);
        w.clear_bullets();
        let ppos = w.player.pos;
        // Two identical shots, one hostile and one friendly, heading away from
        // the rat so nothing collides this tick.
        let mut hostile = Bullet::new(
            ppos,
            Vec2::new(120.0, 0.0),
            1.0,
            Faction::Enemy,
            (255, 0, 0),
        );
        hostile.life = 5.0;
        let mut friendly = Bullet::new(
            ppos,
            Vec2::new(120.0, 0.0),
            1.0,
            Faction::Player,
            (0, 0, 255),
        );
        friendly.life = 5.0;
        w.spawn_bullet(hostile);
        w.spawn_bullet(friendly);

        fn steps(w: &mut World) -> (f32, f32) {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(projectiles::bullets_system)
                .expect("bullets run");
            w.end_tick();
            let bullets = w.bullets();
            (
                (bullets[0].pos - bullets[0].prev_pos).len(),
                (bullets[1].pos - bullets[1].prev_pos).len(),
            )
        }

        w.player.buffs.slow_bullets = POCKET_WATCH_SECS;
        let (hostile_step, friendly_step) = steps(&mut w);
        assert!(
            (hostile_step - 120.0 * DT * POCKET_WATCH_BULLET_SLOW).abs() < 0.3,
            "hostile pellet stepped {hostile_step} px"
        );
        assert!(
            (friendly_step - 120.0 * DT).abs() < 0.3,
            "the watch must not slow friendly fire: {friendly_step}"
        );

        // With the watch spent, the same pellet is back to full speed.
        w.player.buffs.slow_bullets = 0.0;
        let (hostile_step, _) = steps(&mut w);
        assert!(
            (hostile_step - 120.0 * DT).abs() < 0.3,
            "the pellet stayed slow after the watch ran out: {hostile_step}"
        );
    }

    /// The watch is a four second buff and a repeat find refreshes it.
    #[test]
    fn the_pocket_watch_lasts_four_seconds_and_refreshes() {
        let mut w = open_arena(235);
        collect(&mut w, PickupKind::PocketWatch);
        assert!((w.player.buffs.slow_bullets - POCKET_WATCH_SECS).abs() < 1e-6);
        for _ in 0..90 {
            buff_tick(&mut w);
        }
        collect(&mut w, PickupKind::PocketWatch);
        assert!(
            (w.player.buffs.slow_bullets - POCKET_WATCH_SECS).abs() < 1e-6,
            "a second watch must refresh, not stack"
        );
        for _ in 0..(POCKET_WATCH_SECS / DT) as usize + 2 {
            buff_tick(&mut w);
        }
        assert_eq!(w.player.buffs.slow_bullets, 0.0);
    }

    // ---- Clean Socks -----------------------------------------------------

    /// The socks ready the dash at once and keep the rat off the glue for six
    /// seconds; the board stays usable and grabs the rat the moment they expire.
    #[test]
    fn clean_socks_ready_the_dash_and_ignore_glue_for_six_seconds() {
        let mut w = open_arena(241);
        w.player.dash_cd = 1.4;
        let c = w.player.pos;
        w.level.objects.sticky_traps.push(
            crate::game::art_traps::StickyTrapView::new(c, Vec2::new(1.0, 0.0)),
        );
        collect(&mut w, PickupKind::CleanSocks);
        assert_eq!(w.player.dash_cd, 0.0, "the socks must ready the dash");
        assert!((w.player.buffs.socks - CLEAN_SOCKS_SECS).abs() < 1e-6);

        // Stand on the pad while the socks hold: never glued.
        for _ in 0..(CLEAN_SOCKS_SECS / DT) as usize - 4 {
            w.player.pos = c;
            w.player.vel = Vec2::ZERO;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(powerups_system)
                .expect("powerups run");
            w.ecs
                .run_system_cached(traps::traps_system)
                .expect("traps run");
            w.end_tick();
            assert!(w.player.buffs.socks > 0.0, "setup: the socks expired early");
            assert_eq!(
                w.level.objects.sticky_traps[0].state,
                crate::game::art_traps::StickyTrapState::Idle,
                "a socked rat was glued"
            );
        }
        // The socks are spent: the same pad grabs the same rat.
        for _ in 0..12 {
            buff_tick(&mut w);
        }
        assert_eq!(w.player.buffs.socks, 0.0, "the socks should expire");
        w.player.pos = c;
        w.player.vel = Vec2::ZERO;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(traps::traps_system)
            .expect("traps run");
        w.end_tick();
        assert_ne!(
            w.level.objects.sticky_traps[0].state,
            crate::game::art_traps::StickyTrapState::Idle,
            "the spent socks must stop protecting the rat"
        );
    }

    // ---- Growth Serum (batch 13) ----------------------------------------

    /// One stomp-beat's worth of ticks of the stomp system through the facade
    /// round-trip. The beat lives in a `Local`, so the cached system keeps the
    /// countdown across these calls exactly as it would across a live run.
    fn stomp_ticks(w: &mut World, n: usize) {
        for _ in 0..n {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(giant_stomp_system)
                .expect("the stomp system must run");
            w.end_tick();
        }
    }

    /// The beats a full interval needs at this dt, with one to spare.
    fn one_beat() -> usize {
        (GIANT_STOMP_INTERVAL / DT).ceil() as usize + 1
    }

    fn enemy_hp(w: &mut World, e: Entity) -> Option<f32> {
        let mut q = w.ecs.query::<&Enemy>();
        q.get(&w.ecs, e).ok().map(|en| en.hp)
    }

    /// The serum arms a 20 s timer, a second find refreshes it, and it runs out
    /// on its own.
    #[test]
    fn the_growth_serum_lasts_twenty_seconds_and_refreshes() {
        let mut w = open_arena(401);
        collect(&mut w, PickupKind::GrowthSerum);
        assert!((w.player.buffs.giant - GROWTH_SECS).abs() < 1e-6);
        assert!(w.player.buffs.is_giant());

        for _ in 0..600 {
            buff_tick(&mut w);
        }
        let left = w.player.buffs.giant;
        assert!(
            (left - (GROWTH_SECS - 600.0 * DT)).abs() < 0.05,
            "ten seconds in the serum has {left} left"
        );
        collect(&mut w, PickupKind::GrowthSerum);
        assert!(
            (w.player.buffs.giant - GROWTH_SECS).abs() < 1e-6,
            "a second serum must refresh to 20 s, not stack"
        );

        for _ in 0..(GROWTH_SECS / DT) as usize + 4 {
            buff_tick(&mut w);
        }
        assert_eq!(w.player.buffs.giant, 0.0, "the serum should expire");
        assert!(!w.player.buffs.is_giant());
        assert!(!w.player.buffs.any(), "nothing else may be left running");
    }

    /// A run that never finds the serum carries a zero, so the stomp pass is a
    /// no-op and the twin runs still match.
    #[test]
    fn a_world_without_the_serum_never_stomps() {
        let mut w = open_arena(403);
        let p = w.player.pos;
        w.clear_enemies();
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(10.0, 0.0), 1.0);
        stomp_ticks(&mut w, one_beat() * 4);
        assert!(
            enemy_hp(&mut w, body).is_some_and(|hp| hp > 0.0),
            "a rat that is not giant must not crush anything"
        );
        assert!(!w.sfx.contains(&crate::audio::Sfx::Bump));
    }

    /// An ordinary body inside the 30 px disc is crushed outright and takes the
    /// normal loot path: the reap in the same tick's slot drops its gem and
    /// books the kill. A body outside the disc is untouched.
    #[test]
    fn the_stomp_crushes_the_horde_inside_the_disc_and_loots_it() {
        let mut w = open_arena(405);
        w.player.buffs.giant = GROWTH_SECS;
        let p = w.player.pos;
        w.clear_enemies();
        let near = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(12.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(90.0, 0.0), 1.0);
        let gems_before = w.gems().len();

        stomp_ticks(&mut w, one_beat());
        assert!(
            enemy_hp(&mut w, near).is_some_and(|hp| hp <= 0.0),
            "the body inside the disc must be crushed"
        );
        assert!(
            enemy_hp(&mut w, far).is_some_and(|hp| hp > 0.0),
            "a body outside the disc must be untouched"
        );
        assert!(
            w.sfx.contains(&crate::audio::Sfx::Bump),
            "the crush must thud"
        );

        // The kill is an ordinary corpse: the reap (the schedule's next slot)
        // despawns it and pays its gem, exactly as for a trap or a bullet.
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(loot::reap_enemies_system)
            .expect("the reap must run");
        w.end_tick();
        assert_eq!(
            w.gems().len(),
            gems_before + 1,
            "the crushed body must drop its gem on the same tick"
        );
        assert_eq!(w.kills, 1, "the crushed body must count as a kill");
    }

    /// An elite takes 60 a stomp and a named body 40, and neither is killed
    /// outright: the stomp is not a Nuke.
    #[test]
    fn an_elite_and_a_named_body_take_their_listed_damage_and_survive() {
        let mut w = open_arena(407);
        w.player.buffs.giant = GROWTH_SECS;
        let p = w.player.pos;
        w.clear_enemies();
        let elite = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(14.0, 0.0), 1.0);
        let named = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 14.0), 1.0);
        {
            let mut e = w.ecs.get_mut::<Enemy>(elite).expect("the elite is alive");
            e.elite = true;
            e.max_hp = 4000.0;
            e.hp = 4000.0;
        }
        {
            let mut e = w.ecs.get_mut::<Enemy>(named).expect("the named body is alive");
            e.named = true;
            e.max_hp = 4000.0;
            e.hp = 4000.0;
        }

        stomp_ticks(&mut w, one_beat());
        let elite_hp = enemy_hp(&mut w, elite).expect("the elite must survive a stomp");
        assert!(
            (elite_hp - (4000.0 - GIANT_ELITE_DAMAGE)).abs() < 0.01,
            "an elite took {} damage, expected {GIANT_ELITE_DAMAGE}",
            4000.0 - elite_hp
        );
        let named_hp = enemy_hp(&mut w, named).expect("the named body must survive a stomp");
        assert!(
            (named_hp - (4000.0 - GIANT_BOSS_DAMAGE)).abs() < 0.01,
            "a named body took {} damage, expected {GIANT_BOSS_DAMAGE}",
            4000.0 - named_hp
        );
    }

    /// An ordinary body cannot bite the giant; a hostile pellet still lands, at
    /// half; an elite's contact also lands, at half.
    #[test]
    fn the_giant_ignores_ordinary_contact_and_takes_half_from_the_rest() {
        // 1. Ordinary contact: nothing at all.
        let mut w = open_arena(409);
        w.player.buffs.giant = GROWTH_SECS;
        w.player.iframes = 0.0;
        let p = w.player.pos;
        w.clear_enemies();
        w.spawn_enemy(EnemyKind::Brute, p, 1.0);
        let hp = w.player.hp;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(combat::contact_system)
            .expect("contact runs");
        w.end_tick();
        assert_eq!(w.player.hp, hp, "an ordinary body must not bite the giant");

        // 2. An elite's contact lands, at the giant's half.
        let mut w = open_arena(411);
        w.player.buffs.giant = GROWTH_SECS;
        w.player.iframes = 0.0;
        let p = w.player.pos;
        w.clear_enemies();
        let elite = w.spawn_enemy(EnemyKind::Brute, p, 1.0);
        let raw = {
            let mut body = w.ecs.get_mut::<Enemy>(elite).expect("the elite is alive");
            body.elite = true;
            body.damage
        };
        let hp = w.player.hp;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(combat::contact_system)
            .expect("contact runs");
        w.end_tick();
        assert!(
            (hp - w.player.hp - raw * GIANT_DAMAGE_TAKEN).abs() < 1e-3,
            "an elite dealt {} to the giant, expected {}",
            hp - w.player.hp,
            raw * GIANT_DAMAGE_TAKEN
        );

        // 3. A hostile pellet lands, at exactly half of what it does to a rat
        //    that is not giant.
        fn pellet_damage(giant: bool) -> f32 {
            let mut w = open_arena(413);
            if giant {
                w.player.buffs.giant = GROWTH_SECS;
            }
            w.clear_bullets();
            w.player.iframes = 0.0;
            let p = w.player.pos;
            let mut pellet = Bullet::new(p, Vec2::ZERO, 20.0, Faction::Enemy, (255, 0, 0));
            pellet.life = 4.0;
            w.spawn_bullet(pellet);
            let hp = w.player.hp;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(combat::enemy_bullet_hits_system)
                .expect("the pellet pass runs");
            w.end_tick();
            hp - w.player.hp
        }
        let plain = pellet_damage(false);
        let giant = pellet_damage(true);
        assert!(plain > 1.0, "setup: the pellet must land: {plain}");
        assert!(
            (giant - plain * GIANT_DAMAGE_TAKEN).abs() < 1e-3,
            "the giant took {giant} from a pellet that deals {plain}"
        );
    }

    fn walk_with_buffs(w: &mut World, dir: Vec2, ticks: usize) {
        for _ in 0..ticks {
            w.begin_tick(DT, dir, false);
            w.ecs.run_system_cached(powerups_system).expect("buffs age");
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .expect("the rat moves");
            w.end_tick();
        }
    }

    fn narrow_passage(opening: crate::game::art_walls::WallKind) -> World {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::TileKind;

        let mut w = open_arena(415);
        w.level.tiles.fill_cells(6, 1, 1, 20, TileKind::Wall(WallKind::Solid));
        w.level.tiles.set(6, 6, TileKind::Wall(opening));
        w.level.tiles.rebuild_masks();
        w.player.pos = Vec2::new(72.0, 104.0);
        w
    }

    #[test]
    fn the_rat_hole_and_one_way_leaf_block_the_giant() {
        use crate::game::art_walls::{Cardinal, WallAxis, WallKind};

        for opening in [
            WallKind::RatHole { axis: WallAxis::Vertical },
            WallKind::OneWayDoor { dir: Cardinal::East },
        ] {
            let mut ordinary = narrow_passage(opening);
            let mut giant = narrow_passage(opening);
            let east = Vec2::new(1.0, 0.0);
            collect(&mut giant, PickupKind::GrowthSerum);
            walk_with_buffs(&mut giant, Vec2::ZERO, 30);
            walk_with_buffs(&mut ordinary, east, 100);
            walk_with_buffs(&mut giant, east, 100);
            assert!(ordinary.player.pos.x > 112.0, "ordinary: {:?}", ordinary.player.pos);
            assert!(giant.player.pos.x < 104.0, "giant: {:?}", giant.player.pos);
        }
    }

    #[test]
    fn the_giant_fits_a_one_cell_corridor_with_jambs() {
        use crate::game::art_walls::{WallAxis, WallKind};
        use crate::game::collision::{SizeClass, TileKind};
        use crate::game::player::GIANT_BODY_MAX_RADIUS;

        let mut w = open_arena(415);
        w.level.tiles.fill_cells(6, 3, 1, 10, TileKind::Wall(WallKind::Solid));
        w.level.tiles.fill_cells(8, 3, 1, 10, TileKind::Wall(WallKind::Solid));
        w.level.tiles.set(7, 7, TileKind::Wall(WallKind::Doorway { axis: WallAxis::Horizontal }));
        w.level.tiles.rebuild_masks();
        w.player.pos = Vec2::new(120.0, 56.0);
        collect(&mut w, PickupKind::GrowthSerum);
        walk_with_buffs(&mut w, Vec2::ZERO, 30);
        assert_eq!(w.player.movement_body(&w.level.tiles), (GIANT_BODY_MAX_RADIUS, SizeClass::Large));

        walk_with_buffs(&mut w, Vec2::new(0.0, 1.0), 115);
        assert_eq!(w.player.pos.x, 120.0);
        assert!(w.player.pos.y > 128.0, "walked through the jambs: {:?}", w.player.pos);
    }

    #[test]
    fn growth_in_a_rat_hole_waits_for_clearance_even_after_a_size_swap() {
        use crate::game::art_walls::{WallAxis, WallKind};
        use crate::game::collision::SizeClass;
        use crate::game::ecs::constants::PLAYER_RADIUS;
        use crate::game::player::GIANT_BODY_MAX_RADIUS;

        let mut w = narrow_passage(WallKind::RatHole { axis: WallAxis::Vertical });
        w.player.pos.x = 104.0;
        let origin = w.player.pos;
        collect(&mut w, PickupKind::ShrinkDraught);
        assert_eq!(w.player.movement_body(&w.level.tiles), (PLAYER_RADIUS, SizeClass::Small));
        assert_eq!(w.player.buffs.hurt_radius(), SHRINK_HURT_RADIUS);
        collect(&mut w, PickupKind::GrowthSerum);
        walk_with_buffs(&mut w, Vec2::ZERO, 30);
        assert_eq!(w.player.pos, origin);
        assert_eq!(w.player.buffs.tiny, 0.0);
        assert_eq!(w.player.movement_body(&w.level.tiles), (PLAYER_RADIUS, SizeClass::Small));

        walk_with_buffs(&mut w, Vec2::new(1.0, 0.0), 30);
        assert!(w.player.pos.x > 112.0 + GIANT_BODY_MAX_RADIUS);
        assert_eq!(w.player.movement_body(&w.level.tiles), (GIANT_BODY_MAX_RADIUS, SizeClass::Large));

        collect(&mut w, PickupKind::ShrinkDraught);
        assert_eq!(w.player.buffs.giant, 0.0);
        assert_eq!(w.player.buffs.hurt_radius(), SHRINK_HURT_RADIUS);
        assert_eq!(w.player.movement_body(&w.level.tiles), (PLAYER_RADIUS, SizeClass::Small));
    }

    #[test]
    fn giant_body_and_hurt_radii_follow_grow_in_and_shrink_out() {
        use crate::game::ecs::constants::PLAYER_RADIUS;
        use crate::game::player::GIANT_BODY_MAX_RADIUS;

        let mut w = open_arena(415);
        collect(&mut w, PickupKind::GrowthSerum);
        assert_eq!(w.player.buffs.hurt_radius(), PLAYER_RADIUS);
        assert_eq!(w.player.movement_body(&w.level.tiles).0, PLAYER_RADIUS);
        walk_with_buffs(&mut w, Vec2::ZERO, 9);
        let midway = w.player.movement_body(&w.level.tiles).0;
        assert!(midway > PLAYER_RADIUS && midway < GIANT_BODY_MAX_RADIUS);
        assert_eq!(midway, w.player.buffs.hurt_radius());
        walk_with_buffs(&mut w, Vec2::ZERO, 30);
        assert_eq!(w.player.movement_body(&w.level.tiles).0, GIANT_BODY_MAX_RADIUS);
        assert_eq!(w.player.buffs.hurt_radius(), PLAYER_RADIUS * GIANT_SCALE);
        w.player.buffs.giant = GIANT_SHRINK_SECS / 3.0;
        let shrinking = w.player.movement_body(&w.level.tiles).0;
        assert!(shrinking > PLAYER_RADIUS && shrinking < GIANT_BODY_MAX_RADIUS);
        assert_eq!(shrinking, w.player.buffs.hurt_radius());
        walk_with_buffs(&mut w, Vec2::ZERO, 15);
        assert_eq!(w.player.buffs.hurt_radius(), PLAYER_RADIUS);
        assert_eq!(w.player.movement_body(&w.level.tiles).0, PLAYER_RADIUS);
    }

    #[test]
    fn growing_flush_with_a_wall_waits_until_the_rat_steps_away() {
        use crate::game::art_walls::{WallAxis, WallKind};
        use crate::game::collision::SizeClass;
        use crate::game::ecs::constants::PLAYER_RADIUS;
        use crate::game::player::GIANT_BODY_MAX_RADIUS;

        let mut w = narrow_passage(WallKind::RatHole { axis: WallAxis::Vertical });
        w.player.pos = Vec2::new(96.0 - PLAYER_RADIUS, 136.0);
        let origin = w.player.pos;
        collect(&mut w, PickupKind::GrowthSerum);
        walk_with_buffs(&mut w, Vec2::ZERO, 30);
        assert_eq!(w.player.pos, origin);
        assert_eq!(w.player.movement_body(&w.level.tiles), (PLAYER_RADIUS, SizeClass::Large));
        walk_with_buffs(&mut w, Vec2::new(-1.0, 0.0), 30);
        assert!(w.player.pos.x < origin.x - 10.0);
        assert_eq!(w.player.movement_body(&w.level.tiles).0, GIANT_BODY_MAX_RADIUS);
    }

    #[test]
    fn pellets_and_elite_contact_can_graze_the_giant() {
        for giant in [false, true] {
            let mut w = open_arena(419);
            if giant {
                collect(&mut w, PickupKind::GrowthSerum);
                walk_with_buffs(&mut w, Vec2::ZERO, 30);
            }
            let offset = w.player.pos + Vec2::new(13.0, 0.0);
            let hp = w.player.hp;
            w.spawn_bullet(Bullet::new(offset, Vec2::ZERO, 20.0, Faction::Enemy, (255, 0, 0)));
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(combat::enemy_bullet_hits_system).unwrap();
            w.end_tick();
            assert_eq!(w.player.hp < hp, giant, "pellet graze");
            w.player.iframes = 0.0;
            let hp = w.player.hp;
            let elite = w.spawn_enemy(EnemyKind::Brute, offset, 1.0);
            w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(combat::contact_system).unwrap();
            w.end_tick();
            assert_eq!(w.player.hp < hp, giant, "elite contact graze");
        }
    }

    /// The drop table still spends exactly one word, and every bucket still
    /// pays a kind — now including the serum.
    #[test]
    fn the_drop_table_pays_the_serum_too() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let mut rng = StdRng::seed_from_u64(0xBEEF);
        let mut saw_serum = false;
        for _ in 0..4096 {
            if loot::roll_pickup_kind(&mut rng) == PickupKind::GrowthSerum {
                saw_serum = true;
            }
        }
        assert!(saw_serum, "the serum must be reachable on the rare table");
    }

    // ---- Drop weights ----------------------------------------------------

    /// The Headlamp takes the last default Medkit bucket.
    #[test]
    fn drop_weights_match_the_design() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let n = 200_000;
        let mut counts = std::collections::HashMap::new();
        for _ in 0..n {
            *counts.entry(loot::roll_pickup_kind(&mut rng)).or_insert(0u32) += 1;
        }
        let pct = |k: PickupKind| *counts.get(&k).unwrap_or(&0) as f32 * 100.0 / n as f32;
        for (kind, want) in [
            (PickupKind::Heal, 29.0),
            (PickupKind::Magnet, 18.0),
            (PickupKind::Nuke, 3.0),
            (PickupKind::GrowthSerum, 4.0),
            (PickupKind::BottleCap, 14.0),
            (PickupKind::WindupMouse, 10.0),
            (PickupKind::PocketWatch, 9.0),
            (PickupKind::CleanSocks, 9.0),
            (PickupKind::ShrinkDraught, 1.0),
            (PickupKind::WhiteCoat, 1.0),
            (PickupKind::TheCollar, 1.0),
            (PickupKind::Headlamp, 1.0),
        ] {
            let got = pct(kind);
            assert!(
                (got - want).abs() < 0.6,
                "{kind:?} came up {got:.2}%, the design says {want}%"
            );
        }
        assert_eq!(counts.len(), PickupKind::all().len(), "every kind must be reachable");
    }

    /// A successful drop spends the same number of rng draws as before: the
    /// one table roll is the only draw either side of the change, and the wider
    /// range consumes the stream identically.
    #[test]
    fn a_drop_draws_the_same_number_of_random_numbers() {
        use rand::RngExt;
        use rand::SeedableRng;
        type Rng = rand::rngs::StdRng;

        let mut a = Rng::seed_from_u64(5);
        let mut b = Rng::seed_from_u64(5);
        assert_eq!(
            loot::roll_pickup_kind(&mut a),
            loot::roll_pickup_kind(&mut b),
            "the table roll is not settled by the seed"
        );
        assert_eq!(
            a.random::<u64>(),
            b.random::<u64>(),
            "the table roll consumed a different number of draws"
        );

        // The legacy three-way table and the new weighted one leave the stream
        // in the same place **for every seed**: one 32-bit word each, never a
        // value-dependent second word (see the test below).
        for seed in 0..1024u64 {
            let mut old = Rng::seed_from_u64(seed);
            let mut new = Rng::seed_from_u64(seed);
            let _ = old.random_range(0..3u32);
            let _ = loot::roll_pickup_kind(&mut new);
            assert_eq!(
                old.random::<u64>(),
                new.random::<u64>(),
                "the wider pickup table changed how much rng a drop consumes (seed {seed})"
            );
        }
    }

    /// A counting generator: the sampler's draw **count** is a spec value, so
    /// the test counts the words instead of comparing one seed's tail.
    struct CountRng {
        inner: rand::rngs::StdRng,
        words: usize,
    }

    impl CountRng {
        fn seeded(seed: u64) -> Self {
            Self {
                inner: <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(seed),
                words: 0,
            }
        }
    }

    /// A scripted generator: yields the words the test hands it, so an
    /// adversarial first word can be posed exactly (the review's
    /// `128849018`, whose multiply-high low limb is `2^32 - 88`).
    struct ScriptRng {
        words: Vec<u32>,
        at: usize,
    }

    impl ScriptRng {
        fn new(words: &[u32]) -> Self {
            Self {
                words: words.to_vec(),
                at: 0,
            }
        }
    }

    impl rand::TryRng for ScriptRng {
        type Error = std::convert::Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            let w = self.words.get(self.at).copied().unwrap_or(0);
            self.at += 1;
            Ok(w)
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            let lo = u64::from(self.words.get(self.at).copied().unwrap_or(0));
            let hi = u64::from(self.words.get(self.at + 1).copied().unwrap_or(0));
            self.at += 2;
            Ok(lo | (hi << 32))
        }

        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
            for chunk in dst.chunks_mut(4) {
                let w = self.try_next_u32().unwrap_or(0);
                for (i, b) in chunk.iter_mut().enumerate() {
                    *b = (w >> (8 * i)) as u8;
                }
            }
            Ok(())
        }
    }

    impl rand::TryRng for CountRng {
        type Error = std::convert::Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            self.words += 1;
            Ok(rand::Rng::next_u32(&mut self.inner))
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            self.words += 2;
            Ok(rand::Rng::next_u64(&mut self.inner))
        }

        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
            self.words += dst.len().div_ceil(4);
            rand::Rng::fill_bytes(&mut self.inner, dst);
            Ok(())
        }
    }

    /// **The pickup roll spends exactly one word for every seed and every
    /// value.** `random_range(0..100)` does not: its multiply-high sampler
    /// takes a second word when the low product limb lands short, which is the
    /// review's "the new pickup range can consume an extra RNG sample even on
    /// a legacy result".
    #[test]
    fn a_pickup_roll_spends_exactly_one_word_whatever_the_value() {
        use rand::RngExt;

        for seed in 0..4096u64 {
            let mut r = CountRng::seeded(seed);
            let _ = loot::roll_pickup_kind(&mut r);
            assert_eq!(r.words, 1, "the pickup roll spent {} words (seed {seed})", r.words);

            // The legacy width: one word, as the table's doc says.
            let mut legacy = CountRng::seeded(seed);
            let _ = legacy.random_range(0..3u32);
            assert_eq!(legacy.words, 1, "the legacy table did not take one word");
        }

        // The ranged sample's extra word, posed exactly: `wmul(128849018, 100)`
        // has the low limb `2^32 - 88`, which is inside the rejection band of a
        // width-100 range, so `random_range` draws a **second** word and —
        // being insensitive to it — returns the same kind the first word would
        // have. That value-dependent draw is what the fixed sampler removes.
        let adversarial = 128_849_018u32;
        let mut wide = ScriptRng::new(&[adversarial, adversarial]);
        let kind = loot::roll_pickup_kind(&mut wide);
        assert_eq!(wide.at, 1, "the fixed sampler asked for a second word");

        let mut ranged = ScriptRng::new(&[adversarial, adversarial]);
        let _ = ranged.random_range(0..loot::PICKUP_WEIGHT_TOTAL);
        assert_eq!(
            ranged.at, 2,
            "the ranged sampler no longer shows the extra draw this fix avoids"
        );
        // Same answer either way, so the change is stream-only.
        assert_eq!(kind, loot::roll_pickup_kind_from_word(adversarial));
        assert_eq!(kind, PickupKind::Heal, "the adversarial word is still a Heal");
        let headlamp_word = (29u64 << 32).div_ceil(100) as u32;
        let mut lamp = ScriptRng::new(&[headlamp_word]);
        assert_eq!(loot::roll_pickup_kind(&mut lamp), PickupKind::Headlamp);
        assert_eq!(lamp.at, 1, "the Headlamp spends the same single loot word");
        // Every bucket is reachable from a word, so the table still pays every
        // kind (the distribution test above pins the weights).
        for word in [0u32, u32::MAX / 2, u32::MAX] {
            let _ = loot::roll_pickup_kind_from_word(word);
        }
    }

    #[test]
    fn headlamp_loot_weights_preserve_all_other_buckets() {
        let mut weights = [0; PickupKind::all().len()];
        for bucket in 0..100u64 {
            let word = ((bucket << 32).div_ceil(100)) as u32;
            weights[loot::roll_pickup_kind_from_word(word) as usize] += 1;
        }
        assert_eq!(weights, [29, 18, 3, 14, 10, 9, 9, 4, 1, 1, 1, 1]);
    }

    #[test]
    fn headlamp_collection_and_expiry_do_not_draw_rng() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        let mut w = open_arena(21);
        w.rng = StdRng::seed_from_u64(21);
        let mut before = StdRng::seed_from_u64(21);
        collect(&mut w, PickupKind::Headlamp);
        assert_eq!(w.player.buffs.headlamp, HEADLAMP_SECS);
        buff_tick(&mut w);
        assert!(w.player.buffs.headlamp < HEADLAMP_SECS);
        collect(&mut w, PickupKind::Headlamp);
        assert_eq!(w.player.buffs.headlamp, HEADLAMP_SECS);
        w.player.buffs.headlamp = DT * 0.5;
        buff_tick(&mut w);
        assert_eq!(w.player.buffs.headlamp, 0.0);
        assert_eq!(w.rng.next_u64(), before.next_u64());
    }

    // ---- Effect captures (manual, `RATRUN_FX_DUMP=1`) -------------------

    /// Render one frame per pocket find's effect straight out of the world
    /// (no terminal, no HUD) into `/tmp/ratrun_fx_<name>.ppm`. A no-op unless
    /// `RATRUN_FX_DUMP` is set, exactly like `app::dump_frame_ppm`; it is the
    /// harness the wave-3 captures were taken with.
    #[test]
    fn dump_effect_frames() {
        if std::env::var_os("RATRUN_FX_DUMP").is_none() {
            return;
        }
        use crate::render::framebuffer::PixelBuffer;
        let (vw, vh) = (120usize, 46usize);

        let save = |name: &str, pb: &PixelBuffer| {
            let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let (r, g, b) = pb.pixel_at(x, y);
                    out.extend_from_slice(&[r, g, b]);
                }
            }
            std::fs::write(format!("/tmp/ratrun_fx_{name}.ppm"), out).unwrap();
        };

        // 1. The four finds on the floor, plus the brass key on its own.
        let mut w = open_arena(301);
        let p = w.player.pos;
        for (i, kind) in [
            PickupKind::BottleCap,
            PickupKind::WindupMouse,
            PickupKind::PocketWatch,
            PickupKind::CleanSocks,
        ]
        .into_iter()
        .enumerate()
        {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 + 0.5;
            w.spawn_pickup(Pickup {
                pos: p + Vec2::from_angle(a) * 22.0,
                kind,
                bob: i as f32 * 1.9,
            });
        }
        spawn_decoy(&mut w, p + Vec2::new(0.0, 16.0));
        {
            let mut pb = PixelBuffer::new(vw, vh);
            w.draw(&mut pb);
            save("floor", &pb);
        }

        // 2. Bottle Cap: the white rim around the rat.
        let mut w = open_arena(303);
        w.player.buffs.shield = BOTTLE_CAP_SECS;
        for _ in 0..6 {
            w.update(DT, Vec2::ZERO, false);
        }
        {
            let mut pb = PixelBuffer::new(vw, vh);
            w.draw(&mut pb);
            save("shield", &pb);
        }

        // 3. Wind-up Mouse: the key and the crowd it pulled.
        let mut w = open_arena(305);
        let p = w.player.pos;
        w.player.buffs.decoy = WINDUP_MOUSE_SECS;
        let key = p + Vec2::new(30.0, 0.0);
        w.player.decoy_at = Some(key);
        // Inside the key's 48 px smell radius but out of reach of the rat, so
        // the capture shows the crowd walking at the key and not at the rat.
        for i in 0..5 {
            let a = -0.9 + i as f32 * 0.5;
            w.spawn_enemy(EnemyKind::Skitterer, key + Vec2::from_angle(a) * 30.0, 3.0);
        }
        for _ in 0..150 {
            w.update(DT, Vec2::ZERO, false);
        }
        {
            let mut pb = PixelBuffer::new(vw, vh);
            w.draw(&mut pb);
            save("decoy", &pb);
        }

        // 4. Pocket Watch: hostile pellets crawling with dark tails.
        let mut w = open_arena(307);
        let p = w.player.pos;
        w.player.buffs.slow_bullets = POCKET_WATCH_SECS;
        for i in 0..6 {
            let mut b = Bullet::new(
                p + Vec2::new(-40.0, -10.0 + i as f32 * 4.0),
                Vec2::new(70.0, 0.0),
                3.0,
                Faction::Enemy,
                (255, 120, 160),
            );
            b.life = 4.0;
            w.spawn_bullet(b);
        }
        for _ in 0..40 {
            w.update(DT, Vec2::new(-0.3, 0.0), false);
        }
        {
            let mut pb = PixelBuffer::new(vw, vh);
            w.draw(&mut pb);
            save("watch", &pb);
        }

        // 5. Clean Socks: the heel prints of a walking rat, over a glue board.
        let mut w = open_arena(309);
        w.player.buffs.socks = CLEAN_SOCKS_SECS;
        let p = w.player.pos;
        w.level.objects.sticky_traps.push(
            crate::game::art_traps::StickyTrapView::new(p, Vec2::new(1.0, 0.0)),
        );
        for _ in 0..50 {
            w.update(DT, Vec2::new(1.0, 0.0), false);
        }
        {
            let mut pb = PixelBuffer::new(vw, vh);
            w.draw(&mut pb);
            save("socks", &pb);
        }
    }

    // ---- Twin-run determinism -------------------------------------------

    /// Same seed, same inputs, same pocket finds: same run. The fingerprint
    /// covers the rat, the buffs, the key and the horde, so anything order- or
    /// rng-sensitive in the new systems shows up here.
    #[test]
    fn twin_runs_with_pocket_finds_match() {
        use rand::RngExt;

        fn run() -> (Vec2, f32, [f32; 4], u32, Option<Vec2>, usize, u32) {
            let mut w = World::new(Theme::Sewer, 2031);
            let ppos = w.player.pos;
            let kinds = [
                PickupKind::BottleCap,
                PickupKind::WindupMouse,
                PickupKind::PocketWatch,
                PickupKind::CleanSocks,
            ];
            for (i, kind) in kinds.iter().enumerate() {
                w.spawn_pickup(Pickup {
                    pos: ppos + Vec2::new(i as f32 * 3.0, 0.0),
                    kind: *kind,
                    bob: 0.0,
                });
            }
            for _ in 0..240 {
                w.update(DT, Vec2::new(1.0, 0.0), false);
            }
            let b = w.player.buffs;
            let next_draw = w.rng.random::<u32>();
            (
                w.player.pos,
                w.player.hp,
                [b.shield, b.decoy, b.slow_bullets, b.socks],
                w.kills,
                decoy_pos(&mut w),
                w.enemies().len(),
                next_draw,
            )
        }
        let a = run();
        let b = run();
        assert_eq!(a, b, "two identical runs diverged");
    }
}
