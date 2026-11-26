//! Wave 3: the five ordinary enemies with tells (`WAVE3_DESIGN.md` §9).
//!
//! One system owns every wave-3 behaviour state machine, plus the floor hazard
//! that outlives its owner (a dead Wick Moth's fuse) and the "at most two
//! nearby warned attacks at once" limiter.
//!
//! ## The five kinds
//!
//! | Kind | Tell | Resolves into |
//! |---|---|---|
//! | Pillbug | at 24 px it curls for [`PILLBUG_CURL_SECS`], immobile, locking its facing | frontal 120° shell ×[`PILLBUG_FRONT_MULT`], exposed rear ×[`PILLBUG_REAR_MULT`] (`Enemy::damage_mult_from`, so every damage source obeys it) |
//! | Bell Flea | both landing crosses are dotted out for [`FLEA_TELL_SECS`] | hop to a locked point within [`FLEA_HOP_REACH`] (7 within 6 px), then spring back home [`FLEA_GAP_SECS`] later for a second 7 |
//! | Wick Moth | the corpse's wick flashes for [`WICK_FUSE_SECS`] | one 9 px burst: 9 to the rat, 20 to enemies; a moth killed by a burst lights its own full fuse |
//! | Porter | raises the cube for [`PORTER_RAISE_SECS`] | heals a wounded ordinary ally 8; any hit on the Porter interrupts the raise |
//! | Turnkey | a dotted [`CROSSBAR_LEN`] line for [`CROSSBAR_TELL_SECS`] | the bar is live for [`CROSSBAR_LIVE_SECS`]; the rat crossing it takes 8 once, enemies pass freely |
//!
//! ## Why one system, placed after contact damage
//!
//! The schedule puts this between `combat::contact_system` and
//! `loot::reap_enemies_system`:
//!
//! * Everything a weapon can do to a body (bullets, pools, melee, actives,
//!   traps, boss hazards and the evolved weapons' own systems, which share
//!   this slot and run first) has already run, so a moth any of them killed
//!   lights its fuse here, and a burst resolves into this tick's loot
//!   path (`reap_enemies` is next): a moth the burst kills leaves its own fuse
//!   in the same tick.
//! * Every tell this system starts stays visible for at least a full tick
//!   before its damage can land, so "warned attack" is literally true: the rat
//!   sees the shell, the crosses, the cube, the dotted bar and the wick before
//!   any of them bite.
//! * It spawns and despawns only non-enemy entities (a [`Wick`]), so the
//!   [`crate::game::ecs::resources::EnemyIndex`] window is untouched.
//! * After it come `trailer::trailer_system` (a flea that lands on a flatbed
//!   hole falls the same tick; moths fly and never fall, so no fall can orphan
//!   a wick) and `weapons_wave3a::hantavirus_death_system` (which therefore
//!   sees burst kills), then the reap.
//!
//! ## Determinism
//!
//! Every pass walks the enemies in `SpawnId` order (one reused scratch sort,
//! and none at all on a tick with no wave-3 body on the map), nothing allocates
//! after the scratch buffers have grown, and **no behaviour draws rng**: the
//! locked points, the heal targets and the bar placements are all pure
//! functions of the tick state. So a run with no wave-3 enemy draws exactly the
//! rng it drew before the wave landed, and a run with them is reproducible from
//! the seed.
//!
//! ## The two-at-once limiter
//!
//! `WAVE3_DESIGN.md` §9: "At most two nearby ordinary enemies may start a
//! warned attack together. A body waiting its turn can still move and take
//! damage." A body wants to start a tell; if two or more tells are already
//! running within [`WARN_NEARBY_RADIUS`], the start is **deferred** (its
//! cooldown stays spent, so it retries next tick) and the body keeps steering,
//! taking damage and dying like any other. Tells in flight from earlier ticks
//! are counted too, so a cluster cannot stack more than two even when they
//! start on different frames.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::{self, SizeClass};
use crate::game::ecs::components::{SpawnId, Wick};
use crate::game::ecs::resources::{NextSpawnId, RngRes, SfxQueue, Shake, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::game::particle::vfx;
use crate::math::Vec2;

// ---- Pillbug -------------------------------------------------------------

/// Distance at which a Pillbug decides to curl up.
pub const PILLBUG_TRIGGER_RANGE: f32 = 24.0;
/// How long one curl lasts (immobile, shell forward).
pub const PILLBUG_CURL_SECS: f32 = 1.4;
/// Cooldown after a curl, so it walks between them instead of curling forever.
pub const PILLBUG_CURL_COOLDOWN: f32 = 2.6;
/// Damage multiplier inside the frontal 120° shell arc.
pub const PILLBUG_FRONT_MULT: f32 = 0.25;
/// Damage multiplier everywhere else (the exposed rear).
pub const PILLBUG_REAR_MULT: f32 = 2.0;

// ---- Bell Flea -----------------------------------------------------------

/// Seconds between one two-jump cycle and the next.
pub const FLEA_JUMP_PERIOD: f32 = 4.0;
/// How far the first jump lands (the locked point is this far toward the rat).
pub const FLEA_HOP_REACH: f32 = 26.0;
/// How long both landing crosses are dotted out before the first impact.
///
/// The design asks for a cross that "shows for 0.65 s": [`FLEA_TELL_SECS`] of
/// dotted tell plus the [`FLEA_GAP_SECS`] the flea then spends landed on the
/// first cross is 0.65 s.
pub const FLEA_TELL_SECS: f32 = 0.35;
/// Time spent landed on the first cross before springing back.
pub const FLEA_GAP_SECS: f32 = 0.30;
/// Radius of one landing cross's impact.
pub const FLEA_CROSS_RADIUS: f32 = 6.0;
/// Damage of one landing cross, to the rat only (enemies pass).
pub const FLEA_CROSS_DAMAGE: f32 = 7.0;

// ---- Wick Moth -----------------------------------------------------------

/// How long the wick flashes before the burst.
pub const WICK_FUSE_SECS: f32 = 1.0;
/// Burst radius.
pub const WICK_BURST_RADIUS: f32 = 9.0;
/// Burst damage to the rat.
pub const WICK_RAT_DAMAGE: f32 = 9.0;
/// Burst damage to enemies (it hurts its own side: lure the pack onto it).
pub const WICK_ENEMY_DAMAGE: f32 = 20.0;

// ---- Porter --------------------------------------------------------------

/// Range in which a Porter finds a wounded ally to treat.
pub const PORTER_RANGE: f32 = 24.0;
/// The cube goes up for this long before the heal lands.
pub const PORTER_RAISE_SECS: f32 = 0.9;
/// Heal per treatment.
pub const PORTER_HEAL: f32 = 8.0;
/// One treatment per this many seconds.
pub const PORTER_PERIOD: f32 = 5.0;
/// Flinch window after a hit interrupts a raise (a stray shot costs it a retry,
/// not its whole cycle).
pub const PORTER_INTERRUPT_COOLDOWN: f32 = 2.0;

// ---- Turnkey -------------------------------------------------------------

/// One crossbar per this many seconds.
pub const TURNKEY_PERIOD: f32 = 6.0;
/// Crossbar length.
pub const CROSSBAR_LEN: f32 = 22.0;
/// Half-width of the bar's damaging line.
pub const CROSSBAR_HALF_WIDTH: f32 = 1.0;
/// The dotted tell before the bar goes live.
pub const CROSSBAR_TELL_SECS: f32 = 0.8;
/// How long the bar stays live.
pub const CROSSBAR_LIVE_SECS: f32 = 2.0;
/// Damage to the rat, once per bar.
pub const CROSSBAR_DAMAGE: f32 = 8.0;
/// The free gap the bar must leave across its own line: a bar that would fill a
/// corridor (less than this much walkable width left over) is rejected.
pub const CROSSBAR_MIN_GAP: f32 = 12.0;
/// How far the free-span probe walks out from the bar's centre, at most.
pub const CROSSBAR_PROBE_MAX: f32 = 40.0;

// ---- The limiter ---------------------------------------------------------

/// "Nearby" for the two-at-once warned-attack rule.
pub const WARN_NEARBY_RADIUS: f32 = 64.0;
/// At most this many warned attacks may be running near one another.
pub const WARN_MAX_NEARBY: usize = 2;

/// One ally as the Porter sees it (a `SpawnId`-ordered snapshot, so the target
/// choice can never depend on storage order).
#[derive(Clone, Copy)]
pub struct AllySnap {
    id: u64,
    pos: Vec2,
}

/// Is a warned tell running on this body right now?
#[inline]
fn warn_active(e: &Enemy) -> bool {
    if !e.kind.is_wave3() {
        return false;
    }
    match e.kind.archetype() {
        EnemyKind::Pillbug => e.w3.curl_t > 0.0,
        EnemyKind::BellFlea => e.w3.flea_phase != 0,
        EnemyKind::Porter => e.w3.raise_t > 0.0,
        EnemyKind::Turnkey => e.w3.bar_telling(),
        _ => false,
    }
}

/// How many warned tells are already running within [`WARN_NEARBY_RADIUS`].
#[inline]
fn warn_pressure(warns: &[(Vec2, u64)], at: Vec2) -> usize {
    let r2 = WARN_NEARBY_RADIUS * WARN_NEARBY_RADIUS;
    let mut n = 0;
    for &(pos, _) in warns {
        if pos.dist_sq(at) <= r2 {
            n += 1;
        }
    }
    n
}

/// One treatment the Porter finished this tick, applied after the walk so the
/// mutable enemy borrow stays single.
#[derive(Clone, Copy)]
pub struct PendingHeal {
    target: u64,
    amount: f32,
}

/// The wave-3 system's reused scratch (see `wave3_system`'s parameter note).
#[derive(Default)]
pub struct Wave3Scratch {
    /// Every live enemy, in `SpawnId` order.
    pub order: Vec<(u64, Entity)>,
    /// The warned tells already running this tick (position + `SpawnId`).
    pub warns: Vec<(Vec2, u64)>,
    /// Wounded bodies a Porter may treat, in `SpawnId` order.
    pub allies: Vec<AllySnap>,
    /// The burning fuses, in `SpawnId` order.
    pub wick_order: Vec<(u64, Entity)>,
    /// Treatments finished this tick, landed after the walk.
    pub heals: Vec<PendingHeal>,
}


/// The full wave-3 behaviour tick. See the module docs for placement and
/// determinism.
pub fn wave3_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut next_id: ResMut<NextSpawnId>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut wicks: Query<(Entity, &SpawnId, &mut Wick)>,
    // The Wind-up Mouse's key: a lured body aims its warned attack at the key,
    // not at the rat (the lure already moves it there — see `enemy_steering`).
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    // The floor's hit softening (`[enemies] sector_damage_mult`): the wick
    // burst, the flea's cross and the turnkey's bar all carry it.
    sector: crate::game::ecs::resources::SectorDamage,
    // One reused scratch bundle: the `SpawnId`-ordered enemy list, the
    // warned-attack ledger, the porter's ally snapshot, the wick order and the
    // finished treatments. (`bevy_ecs` accepts at most 16 system parameters, so
    // they travel together.)
    mut scratch: Local<Wave3Scratch>,
) {
    let dt = input.dt;
    let dmg_scale = sector.mult();
    let ppos = player.pos;
    let scratch = &mut *scratch;
    // The live key (highest `SpawnId` wins if two ever exist, the same rule the
    // steering lure uses, so the walk and the tell agree).
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    // Cheap gate: with no wave-3 body and no burning wick on the map this
    // system does nothing at all — not one sort, not one rng draw. That is what
    // keeps an early run (and every golden trace's opening) identical.
    let has_wave3 = enemies.iter().any(|(_, _, e)| e.kind.is_wave3());
    let has_wicks = wicks.iter().next().is_some();
    if !has_wave3 && !has_wicks {
        return;
    }

    // ---- The wick pass: fuses burn down and burst -------------------------
    //
    // First, so a moth the burst kills is armed before the corpse scan below
    // and gets its own full fuse this same tick.
    scratch.order.clear();
    scratch.order.extend(enemies.iter().map(|(entity, id, _)| (id.0, entity)));
    scratch.order.sort_unstable_by_key(|&(id, _)| id);

    scratch.wick_order.clear();
    scratch.wick_order.extend(wicks.iter().map(|(entity, id, _)| (id.0, entity)));
    scratch.wick_order.sort_unstable_by_key(|&(id, _)| id);
    for i in 0..scratch.wick_order.len() {
        let entity = scratch.wick_order[i].1;
        let Ok((_, _, mut wick)) = wicks.get_mut(entity) else {
            continue;
        };
        wick.fuse -= dt;
        if wick.fuse > 0.0 {
            continue;
        }
        let pos = wick.pos;
        // The burst: 9 to the rat, 20 to every enemy inside the radius (its own
        // side included — that is the whole point of the wick).
        if pos.dist(ppos) <= WICK_BURST_RADIUS + player.buffs.hurt_radius()
            && player.take_damage(WICK_RAT_DAMAGE * dmg_scale)
        {
            sfx.0.push(Sfx::Hurt);
            vfx::emit_player_hurt(&mut particles, &mut rng.0, ppos);
        }
        for &(_, friendly) in scratch.order.iter() {
            let Ok((_, _, mut e)) = enemies.get_mut(friendly) else {
                continue;
            };
            if !e.alive() || e.pos.dist(pos) > WICK_BURST_RADIUS + e.radius {
                continue;
            }
            let applied = e.hurt_from(WICK_ENEMY_DAMAGE, pos);
            let killed = !e.alive();
            numbers.record(e.id, e.pos, e.radius, applied, killed, DamageSource::Normal);
        }
        vfx::emit_wave3_burst(&mut particles, &mut rng.0, pos);
        shake.amount = shake.amount.max(4.0);
        sfx.0.push(Sfx::WickBurst);
        commands.entity(entity).despawn();
    }

    // ---- Pass 1: what is already running, and who can be healed -----------
    scratch.allies.clear();
    scratch.warns.clear();
    for j in 0..scratch.order.len() {
        let (self_id, entity) = scratch.order[j];
        let Ok((_, _, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() || e.kind == EnemyKind::Boss {
            continue;
        }
        // The porter's casebook: any wounded body on the horde's side (the boss
        // aside), not only the wave-3 kinds. A named miniboss is not an
        // "ordinary ally" (§9), so a Porter never tops up an appointment.
        if e.hp < e.max_hp && !e.named {
            scratch.allies.push(AllySnap {
                id: self_id,
                pos: e.pos,
            });
        }
        if !e.kind.is_wave3() {
            continue;
        }
        // A tell started on an earlier tick is part of this tick's budget; one
        // starting *this* tick (`warn_age == 0`) is decided in order below.
        if warn_active(e) && e.w3.warn_age > 0.0 {
            scratch.warns.push((e.pos, self_id));
        }
    }

    // ---- Pass 2: the state machines, in `SpawnId` order -------------------
    scratch.heals.clear();
    for i in 0..scratch.order.len() {
        let entity = scratch.order[i].1;
        let Ok((_, id, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.kind.is_wave3() {
            continue;
        }

        // Lab (`floor_lab`): the Walking Syringe runs its own needle, not the
        // Bell Flea's hop.
        if crate::game::ecs::systems::floor_lab::owns(e.kind) {
            continue;
        }
        // A dead Wick Moth becomes a burning wick, once (a registration that
        // inherits the moth's fuse does the same).
        if e.kind.archetype() == EnemyKind::WickMoth && e.hp <= 0.0 && e.w3.wick_t > 0.0 {
            let pos = e.pos;
            e.w3.wick_t = -1.0;
            commands.spawn((Wick::new(pos), next_id.take()));
            continue;
        }

        if e.w3.armed == 0 {
            arm(&mut e);
        }

        // Sewer: the Sump Leech never curls; its tongue is `floor_sewer`'s.
        if e.kind == EnemyKind::SumpLeech {
            continue;
        }

        let self_id = id.0;
        // Where this body's warned attack is aimed: the lure's key if the key
        // is luring it, the rat otherwise. A decoyed Pillbug curls at the key,
        // a decoyed Flea hops at it and a decoyed Turnkey lays its bar across
        // the line to it — the tell has to match the body's target, or the
        // key would be free crowd control with the worst attacks still
        // pointing at the rat.
        let target = lure_target(&e, ppos, decoy, &level);
        // The state machines are the archetype's: a registered kind runs its
        // base body's tell until its floor's own batch replaces it.
        match e.kind.archetype() {
            EnemyKind::Pillbug => pillbug_tick(&mut e, target, dt, &scratch.warns, &mut particles, &mut rng, &mut sfx),
            EnemyKind::BellFlea => flea_tick(
                &mut e,
                target,
                dt,
                &level,
                &scratch.warns,
                &mut player,
                &mut particles,
                &mut rng,
                &mut sfx,
                dmg_scale,
            ),
            EnemyKind::Porter => {
                porter_tick(&mut e, self_id, dt, &scratch.allies, &scratch.warns, &mut particles, &mut rng, &mut sfx, &mut scratch.heals)
            }
            EnemyKind::Turnkey => turnkey_tick(
                &mut e,
                target,
                dt,
                &level,
                &scratch.warns,
                &mut player,
                &mut particles,
                &mut rng,
                &mut sfx,
                dmg_scale,
            ),
            _ => {}
        }

        // A tell that started on this tick joins the ledger now, so the bodies
        // later in `SpawnId` order see the slot taken (`warn_age == 0` is the
        // "started this tick" marker). Then age it, so the next tick's pass 1
        // counts it as already running.
        if warn_active(&e) {
            if e.w3.warn_age <= 0.0 {
                scratch.warns.push((e.pos, id.0));
            }
            e.w3.warn_age += dt;
        } else {
            e.w3.warn_age = 0.0;
        }
    }

    // ---- Pass 3: land the treatments -------------------------------------
    if !scratch.heals.is_empty() {
        for i in 0..scratch.order.len() {
            let entity = scratch.order[i].1;
            let id = scratch.order[i].0;
            let Some(&heal) = scratch.heals.iter().find(|h| h.target == id) else {
                continue;
            };
            let Ok((_, _, mut ally)) = enemies.get_mut(entity) else {
                continue;
            };
            if !ally.alive() {
                continue;
            }
            ally.hp = (ally.hp + heal.amount).min(ally.max_hp);
            vfx::emit_porter_heal(&mut particles, &mut rng.0, ally.pos);
            sfx.0.push(Sfx::Heal);
        }
    }
}

/// The point a body's warned attack is aimed at: the Wind-up Mouse's key while
/// the key is luring it, the rat otherwise.
///
/// The lure rule is the steering lure's own (`powerups::decoy_lures_body`, the
/// key's radius and a clear line), so a body the key is *not* pulling keeps
/// aiming at the rat and the two never disagree about who is being chased.
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

/// Seed a body's timers the first time it is simulated (spawn paths other than
/// the director — the facade, tests — never touch the wave-3 fields).
fn arm(e: &mut Enemy) {
    e.w3.armed = 1;
    match e.kind.archetype() {
        EnemyKind::BellFlea => {
            e.w3.flea_cd = FLEA_JUMP_PERIOD;
            e.w3.flea_phase = 0;
        }
        EnemyKind::Porter => e.w3.heal_cd = PORTER_PERIOD * 0.5,
        EnemyKind::Turnkey => e.w3.bar_cd = TURNKEY_PERIOD * 0.5,
        EnemyKind::Pillbug => e.w3.curl_cd = 0.0,
        _ => {}
    }
}

fn pillbug_tick(
    e: &mut Enemy,
    ppos: Vec2,
    dt: f32,
    warns: &[(Vec2, u64)],
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
) {
    // Kitchen (`floor_kitchen`): an Egg Timer never curls; its dial is its own
    // state machine.
    if e.kind == EnemyKind::EggTimer {
        return;
    }
    e.w3.curl_cd = (e.w3.curl_cd - dt).max(0.0);
    if e.w3.opening_curl {
        e.vel = Vec2::ZERO;
        if e.pos.dist(ppos) > PILLBUG_TRIGGER_RANGE {
            return;
        }
        e.w3.opening_curl = false;
    }
    if e.w3.curl_t > 0.0 {
        e.w3.curl_t -= dt;
        e.vel = Vec2::ZERO;
        if e.w3.curl_t <= 0.0 {
            e.w3.curl_t = 0.0;
            e.w3.curl_cd = PILLBUG_CURL_COOLDOWN;
        }
        return;
    }
    if e.w3.curl_cd > 0.0 || e.pos.dist(ppos) > PILLBUG_TRIGGER_RANGE {
        return;
    }
    if warn_pressure(warns, e.pos) >= WARN_MAX_NEARBY {
        // Waiting its turn: it keeps walking at the rat.
        return;
    }
    // Lock the shell toward the rat: the shell is where it is facing.
    let to = ppos - e.pos;
    if to.len_sq() > 1e-6 {
        e.facing = to.normalized();
    }
    // Kitchen (`floor_kitchen`): a Sink Crab's lid faces 90° left of its
    // approach for the whole curl.
    if e.kind == EnemyKind::SinkCrab {
        e.facing = crate::game::ecs::systems::floor_kitchen::sink_crab_shield_facing(e.facing);
    }
    e.w3.curl_t = PILLBUG_CURL_SECS;
    e.w3.warn_age = 0.0;
    e.vel = Vec2::ZERO;
    vfx::emit_wave3_tell(particles, &mut rng.0, e.pos, e.facing, e.kind);
    sfx.0.push(Sfx::Warn);
}

fn flea_tick(
    e: &mut Enemy,
    // The point this jump is aimed at: the rat, or the Wind-up Mouse's key.
    ppos: Vec2,
    dt: f32,
    level: &Level,
    warns: &[(Vec2, u64)],
    player: &mut Player,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) {
    match e.w3.flea_phase {
        0 => {
            e.w3.flea_cd = (e.w3.flea_cd - dt).max(0.0);
            if e.w3.flea_cd > 0.0 {
                return;
            }
            if warn_pressure(warns, e.pos) >= WARN_MAX_NEARBY {
                return;
            }
            let to = ppos - e.pos;
            let dist = to.len();
            let dir = if dist > 1e-3 {
                to * (1.0 / dist)
            } else if e.facing.len_sq() > 1e-6 {
                e.facing
            } else {
                Vec2::new(1.0, 0.0)
            };
            let reach = FLEA_HOP_REACH.min(dist.max(0.0));
            let hop = e.pos + dir * reach;
            let class = SizeClass::of_enemy(e.kind);
            if !level.tiles.circle_clear(hop, e.radius, class)
                || !level.tiles.circle_clear(e.pos, e.radius, class)
                // A hole in the deck is no landing either: the hop would drop
                // the flea into the flatbed's pit for 35 % of its max HP.
                || level.trailer.pit_at(hop).is_some()
                || level.trailer.pit_at(e.pos).is_some()
            {
                // No safe landing (a wall, a jamb or a hole): try again shortly.
                // No rng.
                e.w3.flea_cd = 0.25;
                return;
            }
            e.w3.home = e.pos;
            e.w3.hop_to = hop;
            e.w3.flea_phase = 1;
            e.w3.flea_t = FLEA_TELL_SECS;
            e.w3.warn_age = 0.0;
            e.facing = dir;
            vfx::emit_wave3_tell(particles, &mut rng.0, hop, dir, e.kind);
            sfx.0.push(Sfx::Warn);
        }
        1 => {
            e.w3.flea_t -= dt;
            if e.w3.flea_t > 0.0 {
                return;
            }
            // First landing.
            e.pos = e.w3.hop_to;
            e.vel = Vec2::ZERO;
            e.w3.flea_phase = 2;
            e.w3.flea_t = FLEA_GAP_SECS;
            flea_impact(&mut rng.0, particles, player, e.pos, false, sfx, dmg_scale);
        }
        _ => {
            e.w3.flea_t -= dt;
            if e.w3.flea_t > 0.0 {
                return;
            }
            // Spring back to where it started: the second cross.
            e.pos = e.w3.home;
            e.vel = Vec2::ZERO;
            e.w3.flea_phase = 0;
            e.w3.flea_cd = FLEA_JUMP_PERIOD;
            flea_impact(&mut rng.0, particles, player, e.pos, true, sfx, dmg_scale);
        }
    }
}

/// One landing: the cross signal (a tell turning into its bite) and the 7 the
/// rat takes if it is still standing on the locked point.
fn flea_impact(
    rng: &mut rand::rngs::StdRng,
    particles: &mut Particles,
    player: &mut Player,
    at: Vec2,
    echo: bool,
    sfx: &mut SfxQueue,
    dmg_scale: f32,
) {
    vfx::emit_flea_impact(particles, rng, at, FLEA_CROSS_RADIUS, echo);
    sfx.0.push(Sfx::FleaHop);
    if at.dist(player.pos) <= FLEA_CROSS_RADIUS + player.buffs.hurt_radius()
        && player.take_damage(FLEA_CROSS_DAMAGE * dmg_scale)
    {
        sfx.0.push(Sfx::Hurt);
    }
}

fn porter_tick(
    e: &mut Enemy,
    self_id: u64,
    dt: f32,
    allies: &[AllySnap],
    warns: &[(Vec2, u64)],
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
    heals: &mut Vec<PendingHeal>,
) {
    e.w3.heal_cd = (e.w3.heal_cd - dt).max(0.0);
    if e.w3.raise_t > 0.0 {
        e.w3.raise_t -= dt;
        e.vel = Vec2::ZERO;
        if e.w3.raise_t > 0.0 {
            return;
        }
        e.w3.raise_t = 0.0;
        // Land the treatment on the ally it was raising for (it may have died
        // or walked off in the meantime; pass 3 skips a missing target).
        heals.push(PendingHeal {
            target: e.w3.heal_target,
            amount: PORTER_HEAL,
        });
        e.w3.heal_target = u64::MAX;
        e.w3.heal_cd = PORTER_PERIOD;
        return;
    }
    if e.w3.heal_cd > 0.0 {
        return;
    }
    if warn_pressure(warns, e.pos) >= WARN_MAX_NEARBY {
        return;
    }
    // Nearest wounded ordinary ally, ties broken by `SpawnId` order.
    let mut best: Option<(f32, u64)> = None;
    for a in allies.iter() {
        if a.id == self_id {
            continue;
        }
        let d = a.pos.dist_sq(e.pos);
        if d > PORTER_RANGE * PORTER_RANGE {
            continue;
        }
        match best {
            Some((bd, _)) if d >= bd => {}
            _ => best = Some((d, a.id)),
        }
    }
    let Some((_, target_id)) = best else {
        return;
    };
    e.w3.raise_t = PORTER_RAISE_SECS;
    e.w3.heal_target = target_id;
    e.w3.warn_age = 0.0;
    e.vel = Vec2::ZERO;
    vfx::emit_wave3_tell(particles, &mut rng.0, e.pos, e.facing, e.kind);
    sfx.0.push(Sfx::Warn);
}

fn turnkey_tick(
    e: &mut Enemy,
    // The point the bar is laid across (the rat, or the lure's key).
    target: Vec2,
    dt: f32,
    level: &Level,
    warns: &[(Vec2, u64)],
    player: &mut Player,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) {
    if e.w3.bar_t > 0.0 {
        e.w3.bar_t = (e.w3.bar_t - dt).max(0.0);
        let rat = player.pos;
        if !e.w3.bar_telling()
            && !e.w3.bar_spent
            && bar_hits(e.w3.bar_pos, e.w3.bar_dir, rat, player.buffs.hurt_radius())
            && player.take_damage(CROSSBAR_DAMAGE * dmg_scale)
        {
            e.w3.bar_spent = true;
            vfx::emit_player_hurt(particles, &mut rng.0, rat);
            sfx.0.push(Sfx::Crossbar);
            // Flatbed (`floor_flatbed`): the Loose Bungee's bar also drags the
            // rat toward its hook. A no-op for every other kind; no rng.
            crate::game::ecs::systems::floor_flatbed::bungee_pull(e.kind, e.pos, level, player);
        }
        if e.w3.bar_t <= 0.0 {
            e.w3.bar_spent = false;
        }
        return;
    }
    e.w3.bar_cd = (e.w3.bar_cd - dt).max(0.0);
    if e.w3.bar_cd > 0.0 {
        return;
    }
    if warn_pressure(warns, e.pos) >= WARN_MAX_NEARBY {
        return;
    }
    // The bar lies across the path it is walking: perpendicular to the line to
    // whatever it is chasing (the rat, or the key that lured it).
    let to = target - e.pos;
    let dir = if to.len_sq() > 1e-6 {
        to.normalized().perp()
    } else if e.facing.len_sq() > 1e-6 {
        e.facing.perp()
    } else {
        Vec2::new(0.0, 1.0)
    };
    if !crossbar_placement_ok(level, e.pos, dir) {
        // Would fill an exit: skip this attempt and look again shortly.
        e.w3.bar_cd = 1.0;
        return;
    }
    e.w3.bar_pos = e.pos;
    e.w3.bar_dir = dir;
    e.w3.bar_t = CROSSBAR_TELL_SECS + CROSSBAR_LIVE_SECS;
    e.w3.bar_spent = false;
    e.w3.bar_cd = TURNKEY_PERIOD;
    e.w3.warn_age = 0.0;
    vfx::emit_wave3_tell(particles, &mut rng.0, e.pos, dir, e.kind);
    sfx.0.push(Sfx::Warn);
}

/// Does the crossbar's line touch a body of radius `r` at `ppos`?
#[inline]
fn bar_hits(pos: Vec2, dir: Vec2, ppos: Vec2, radius: f32) -> bool {
    let half = dir * (CROSSBAR_LEN * 0.5);
    collision::segment_circle_hit(
        pos - half,
        pos + half,
        CROSSBAR_HALF_WIDTH,
        ppos,
        radius,
    )
}

/// Free walkable span (px) across the bar's line, measured out from its centre
/// until a wall cell on each side (capped at [`CROSSBAR_PROBE_MAX`]).
pub fn crossbar_span(level: &Level, pos: Vec2, dir: Vec2) -> f32 {
    let mut span = 0.0f32;
    for sign in [1.0f32, -1.0] {
        let mut d = 0.0f32;
        loop {
            d += 2.0;
            if d >= CROSSBAR_PROBE_MAX {
                d = CROSSBAR_PROBE_MAX;
                break;
            }
            let p = pos + dir * (sign * d);
            let (cx, cy) = collision::cell_of(p);
            if !level.tiles.passable(cx, cy, SizeClass::Small) {
                break;
            }
        }
        span += d;
    }
    span
}

/// A Turnkey rejects a bar that would fill an exit: the free walkable width
/// across its own line has to leave a rat-sized gap beside the 22 px bar.
pub fn crossbar_placement_ok(level: &Level, pos: Vec2, dir: Vec2) -> bool {
    crossbar_span(level, pos, dir) >= CROSSBAR_LEN + CROSSBAR_MIN_GAP
}

#[cfg(test)]
mod tests;
