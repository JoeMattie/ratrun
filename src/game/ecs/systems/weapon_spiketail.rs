//! The **Spiked Tail Whip** (wave 3, `w3-spiketail`).
//!
//! An ordinary auto-firing level-up weapon with five levels, and the third tail
//! weapon in the game — deliberately *not* the other two:
//!
//! - **Tail Whip** is a radial aura pulse around the rat (everything close).
//! - **Tail Thrash** is an active, charged burst of full sweeps (Space).
//! - **Spiked Tail** is a single-target lash: on its cooldown the tail
//!   bristles, then cracks straight out at one targeted body, lands a heavy
//!   hit with a small burst at the impact point, and recoils.
//!
//! ## The beat
//!
//! One [`Lash`] entity per shot, spawned by `weapons::fire_one`, carries the
//! whole shot (the numbers are snapshotted at fire time, so a level-up card
//! opened mid-lash cannot change a hit that was already paid for). The sim
//! runs on the same clock the art does — [`LASH_WINDUP`](crate::game::weapon::
//! LASH_WINDUP) → the crack → [`LASH_RECOIL_AT`](crate::game::weapon::
//! LASH_RECOIL_AT) → [`LASH_TOTAL`](crate::game::weapon::LASH_TOTAL):
//!
//! 1. **Wind-up.** The tail thickens and sprouts spikes (drawn on the real
//!    tail chain in `art_rat`), and the impact point follows its body.
//! 2. **Crack.** The primary body takes the full lash damage
//!    ([`Enemy::hurt_from`] with the direction *from the rat*, so the
//!    Pillbug's shell still applies), a short stagger and a shove through
//!    `Stats::shove` (Counterweight). Everything *else* within the burst
//!    radius takes [`LASH_BURST_FRAC`](crate::game::weapon::LASH_BURST_FRAC)
//!    of it, arriving from the impact point. From level 4 a couple of spike
//!    shards break off and fly on as ordinary friendly bullets.
//! 3. **Recoil.** An evolved **Barbed Wire** lash drags its catch home,
//!    bursting every body it crosses. Heavy bodies are not dragged; a catch
//!    the crack killed is reeled in empty (the line still bursts). So that
//!    there *is* something to cross, an auto-aimed Barbed Wire lash casts at
//!    the farthest hookable body in reach instead of the nearest
//!    ([`pick_lash_target`]).
//!
//! The boss is not an `Enemy`: it is picked through its grounded hit points
//! (`BossTargetable`), takes the full lash through its hit shapes, and can
//! take the burst once on top (the design's rule for the big bodies).
//!
//! ## The screen
//!
//! `LASH_REACH` is 80–120 px, which is past the edge of a 120-column view at
//! 1× (120 px half-width, 40 px half-height), so a target is only eligible
//! inside the **unzoomed simulation view** ([`LASH_VIEW_INSET`], 4 px inside),
//! as well as within the tail's reach. The crack's burst centre and the drawn
//! whip's tip are therefore always on screen. That rectangle is the tick's
//! `Viewport` centred on the rat — never the render zoom, because every zoom
//! must simulate identically.
//!
//! ## Determinism
//!
//! - The enemy walk uses the cached [`EnemyIndex`] order (valid at this slot:
//!   nothing has spawned or despawned since `grid_refresh`), with the usual
//!   length-checked `Local` fallback for the narrow single-system tests.
//! - Lash entities are walked in `SpawnId` order; the burst walks the
//!   `SpawnId`-ordered enemy list, so two identical runs land the same hits in
//!   the same order and draw their sparks in the same order.
//! - Firing draws **no** rng at all (the bristle is the tail art, not
//!   particles), so a run that never owns the weapon is byte-for-byte
//!   unchanged. A lash with no target never spawns and draws nothing.
//! - The Barbed Wire drag moves a body, so the spatial grid is re-filed from
//!   the still-valid `EnemyIndex` before the projectile and contact passes read
//!   it (the same one-fill rule as Counterweight's nudge in `weapons.rs`).

use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{SizeClass, SpatialGrid};
use crate::game::ecs::components::{Entity, Lash, LashTarget, SpawnId};
use crate::game::ecs::resources::{
    DamageLedger, EnemyIndex, NextSpawnId, RngRes, Shake, TickInput,
};
use crate::game::ecs::systems::approach::in_sim_view_inset;
use crate::game::ecs::systems::boss::{Boss, BossTargetable};
use crate::game::ecs::systems::projectiles::los_clear;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::ecs::constants::GRID_CELL;
use crate::game::weapon::{
    WeaponKind, LASH_BURST_FRAC, LASH_DRAG_GAP, LASH_HEAVY_SCALE, LASH_RECOIL_AT,
    LASH_SHARD_FRAC, LASH_SHAKE, LASH_STAGGER, LASH_TOTAL, LASH_WINDUP,
};
use crate::math::Vec2;

#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;

/// Radius the lash's line-of-sight tests use (the honest body radius of a
/// melee swing — `weapons.rs` uses the same number for its melee effects).
const LASH_LOS_RADIUS: f32 = 1.0;
/// Radius a splash/line test uses (a floor effect, not a projectile).
const FIELD_LOS_RADIUS: f32 = 1.0;
/// Candidate cap for one pick, matching `weapons::TARGET_LOS_CANDIDATES`: the
/// pick is nearest-first, so a screen of covered bodies costs a fixed number of
/// tile queries.
const CANDIDATES: usize = 12;
/// How far inside the **unzoomed screen's** edge a lash target has to be.
///
/// The tail reaches 80–120 px, which is past the edge of a 120-column view at
/// 1× (120 px half-width, 40 px half-height), so a body could be lashed while
/// the player cannot see it. A target is therefore only eligible inside the
/// simulation view rectangle ([`in_sim_view_inset`], the
/// [`Viewport`](crate::game::ecs::resources::Viewport) resource
/// at zoom 1.0 — never the render zoom) with this margin, so the crack's burst
/// centre and the drawn whip's tip land on screen. (The 0.12 s wind-up still
/// follows its body, so the point can drift a few px from where it was picked.)
const LASH_VIEW_INSET: f32 = 4.0;
/// The boss is huge; a locked impact point still counts as "on it" inside this
/// many px of a grounded hit shape (the shape moves under the point for the
/// 0.12 s of wind-up).
const BOSS_LOCK_SLOP: f32 = 10.0;
/// Speed (px/s) and life (s) of a broken-off spike shard: about 63 px of
/// travel, a shard's throw.
const SHARD_SPEED: f32 = 150.0;
const SHARD_LIFE: f32 = 0.42;
/// Half-angle between two shards of the fan (radians).
const SHARD_FAN: f32 = 0.22;

/// The per-tick reusable scratch: the fallback `SpawnId`-ordered enemy list and
/// the lash entities in `SpawnId` order. Both are `Local`s' worth of memory
/// reused for the life of the schedule, so the system allocates nothing.
#[derive(Default)]
pub struct LashScratch {
    order: Vec<Entity>,
    lashes: Vec<(u64, Entity)>,
}

// ---- Target picking ------------------------------------------------------

/// Bodies the Barbed Wire hook slips off (and the crack does not stagger): an
/// elite, a named miniboss and every `EnemyKind::heavy()` body.
#[inline]
fn unhookable(e: &Enemy) -> bool {
    e.kind.heavy() || e.elite || e.named
}

/// Where a lash is aimed: the impact point plus what the full hit lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LashPick {
    pub pos: Vec2,
    pub target: LashTarget,
}

#[derive(Clone, Copy)]
struct Candidate {
    /// The pick key, smallest first: squared distance from the choosing point
    /// (the rat, or the aim point when an aim exists) — negated for a hookable
    /// body under a Barbed Wire auto-aim, which casts at the farthest one.
    dist_sq: f32,
    /// Enemy bodies first, boss hit points second (the rule every aiming
    /// weapon shares).
    rank: u8,
    /// `SpawnId` for an enemy, the point index for a boss point.
    id: u64,
    pos: Vec2,
    target: LashTarget,
}

const EMPTY_CANDIDATE: Candidate = Candidate {
    dist_sq: f32::MAX,
    rank: u8::MAX,
    id: u64::MAX,
    pos: Vec2::ZERO,
    target: LashTarget::Boss,
};

/// Nearest-first ordering: distance, then enemy before boss point, then lower
/// `SpawnId` / point index.
#[inline]
fn precedes(a: &Candidate, b: &Candidate) -> bool {
    match a.dist_sq.total_cmp(&b.dist_sq) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => match a.rank.cmp(&b.rank) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => a.id < b.id,
        },
    }
}

/// Insert `c` into the fixed-size nearest-first scratch (bodies past the cap
/// are simply not candidates this shot).
#[inline]
fn push_candidate(out: &mut [Candidate; CANDIDATES], n: &mut usize, c: Candidate) {
    let at = out[..*n].partition_point(|o| precedes(o, &c));
    if at >= CANDIDATES {
        return;
    }
    let last = (*n).min(CANDIDATES - 1);
    let mut i = last;
    while i > at {
        out[i] = out[i - 1];
        i -= 1;
    }
    out[at] = c;
    *n = last + 1;
}

/// **The one place the lash's target is chosen.**
///
/// The nearest eligible body in reach, ties by `SpawnId`, enemies ahead of the
/// boss's hit points; the walk then spends one line-of-sight query per
/// candidate, nearest first, and returns the first body the tail can actually
/// reach (walls stop a lash: it is a tail, not a bullet).
///
/// **Eligible** means inside the unzoomed view rectangle as well as in reach:
/// `view` is the [`Viewport`](crate::game::ecs::resources::Viewport) resource
/// and the rectangle is centred on the
/// rat (`from`), inset by [`LASH_VIEW_INSET`]. A body the player cannot see is
/// never picked, so the burst lands on screen. This is the one pick for every
/// lash — the plain Spiked Tail and the evolved **Barbed Wire**'s long cast
/// included.
///
/// `aim` is **the hook the mouse/gamepad aim worker plugs into**. `None` (the
/// keyboard/auto-fire case) picks the body nearest the *rat*; `Some(point)`
/// picks the eligible body nearest that world point instead, so an aimed
/// lash goes where the player is pointing rather than behind them. Everything
/// else — the reach, the tie order, the wall stop — is shared, so the aim
/// branch is one argument at the call site in [`fire_spiked_tail`].
///
/// `hook` is the evolved **Barbed Wire** lash. Under auto-aim the plain rule
/// (nearest the rat) made the evolution a no-op: the nearest body has, by
/// definition, nothing between it and the rat, so the reel crossed nobody and
/// moved its catch a few pixels. A hooking auto-aimed lash therefore casts
/// *long*: the **farthest** hookable body in reach first (ties by `SpawnId`),
/// then, when nothing in reach can be hooked, the heavy bodies and the boss
/// nearest-first exactly as before. An aimed lash is unchanged — the player
/// chose the catch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick_lash_target(
    level: &Level,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    from: Vec2,
    reach: f32,
    aim: Option<Vec2>,
    hook: bool,
    view: Vec2,
) -> Option<LashPick> {
    let reach_sq = reach * reach;
    // Only auto-aim casts long; see the doc comment.
    let cast_long = hook && aim.is_none();
    // With an aim, "nearest" is measured from the aim point; without, from the
    // rat's rump.
    let key_from = aim.unwrap_or(from);
    let mut cand = [EMPTY_CANDIDATE; CANDIDATES];
    let mut n = 0;
    for &entity in order {
        let Ok((_, id, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        // The reach is always the tail's: an aim cannot stretch the weapon.
        // It is measured to the body's near **edge**, the rule the other two
        // tail weapons use (`dist <= reach + e.radius`): measured to the
        // centre, a Brute or a miniboss pressed against the reach was refused
        // while visibly inside it, and the weapon sat idle.
        let dist_sq = e.pos.dist_sq(from);
        let limit = reach + e.radius;
        if dist_sq.is_nan() || dist_sq > limit * limit {
            continue;
        }
        // The player has to be able to see the crack land.
        if !in_sim_view_inset(e.pos, from, view, LASH_VIEW_INSET) {
            continue;
        }
        // A negated key sorts every hookable body ahead of the rest, farthest
        // first; everything else keeps its plain nearest-first key.
        let key = if cast_long && !unhookable(e) {
            -dist_sq
        } else {
            e.pos.dist_sq(key_from)
        };
        push_candidate(
            &mut cand,
            &mut n,
            Candidate {
                dist_sq: key,
                rank: 0,
                id: id.0,
                pos: e.pos,
                target: LashTarget::Enemy(entity),
            },
        );
    }
    for (index, point) in boss_points.iter().enumerate() {
        let dist_sq = point.dist_sq(from);
        if dist_sq.is_nan() || dist_sq > reach_sq {
            continue;
        }
        // The boss is huge; only the hit shape the tail can reach *on screen*.
        if !in_sim_view_inset(point, from, view, LASH_VIEW_INSET) {
            continue;
        }
        push_candidate(
            &mut cand,
            &mut n,
            Candidate {
                dist_sq: point.dist_sq(key_from),
                rank: 1,
                id: index as u64,
                pos: point,
                target: LashTarget::Boss,
            },
        );
    }
    cand[..n]
        .iter()
        .find(|c| los_clear(level, from, c.pos, LASH_LOS_RADIUS))
        .map(|c| LashPick {
            pos: c.pos,
            target: c.target,
        })
}

// ---- The shot ------------------------------------------------------------

/// The Spiked Tail's shot: pick one body in reach with a clear line and spawn
/// the lash that will crack at it.
///
/// Returns `false` when nothing eligible is in reach behind cover, which makes
/// `fire_weapons_system` retry next tick instead of serving a full cooldown
/// for a whip that cracked at nothing. Draws no rng and spawns nothing else:
/// the wind-up's whole tell is the drawn tail (`art_rat::draw_tail` with a
/// bristle), so a lash that never lands costs the rng stream nothing.
///
/// `view` is the tick's [`Viewport`](crate::game::ecs::resources::Viewport):
/// a target past the edge of the unzoomed screen is not eligible (see
/// [`LASH_VIEW_INSET`]), so the tail never cracks at something out of view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_spiked_tail(
    commands: &mut Commands,
    level: &Level,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    ppos: Vec2,
    reach: f32,
    radius: f32,
    dmg: f32,
    shove: f32,
    shards: u8,
    evolved: bool,
    next_id: &mut NextSpawnId,
    aim: Option<Vec2>,
    view: Vec2,
) -> bool {
    // The aim hook: `None` is auto-fire (nearest the rat); an aimed scheme
    // hands its reticle point through and the lash takes the eligible body
    // nearest it.
    let Some(pick) =
        pick_lash_target(level, order, enemies, boss_points, ppos, reach, aim, evolved, view)
    else {
        return false;
    };
    commands.spawn((
        Lash {
            origin: ppos,
            target: pick.pos,
            primary: match pick.target {
                LashTarget::Enemy(entity) => Some(LashTarget::Enemy(entity)),
                LashTarget::Boss => None,
            },
            boss: pick.target == LashTarget::Boss,
            reach,
            radius,
            dmg,
            shove,
            shards,
            evolved,
            timer: 0.0,
            struck: false,
            dragged: false,
        },
        next_id.take(),
    ));
    true
}

// ---- The crack -----------------------------------------------------------

/// Land one lash's crack: the primary hit, the boss, the small burst, the
/// shards and the feedback.
///
/// It never moves a body (only the scattered spike shards, which are new
/// entities), so unlike [`drag`] it cannot invalidate the spatial grid.
#[allow(clippy::too_many_arguments)]
fn crack(
    lash: &Lash,
    commands: &mut Commands,
    level: &Level,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    next_id: &mut NextSpawnId,
    shake: &mut Shake,
) {
    let kind = WeaponKind::SpikedTail;
    let burst = lash.dmg * LASH_BURST_FRAC;
    let from = lash.origin;
    let point = lash.target;
    let mut killed_any = false;

    // 1. The primary body: the full lash, a stagger and a shove.
    if let Some(LashTarget::Enemy(primary)) = lash.primary {
        if let Ok((_, _, mut e)) = enemies.get_mut(primary) {
            // It has to still be where the tail is pointing (the point follows
            // the body, so this only fails for a body that died) and still
            // behind a clear line.
            let reachable = e.alive()
                && e.pos.dist(point) <= e.radius + 8.0
                && los_clear(level, from, e.pos, LASH_LOS_RADIUS);
            if reachable {
                let hp_before = e.hp.max(0.0);
                // Directional armour: the lash arrives from the rat.
                let applied = e.hurt_from(lash.dmg, from);
                let killed = !e.alive();
                killed_any |= killed;
                ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
                let pos = e.pos;
                let radius = e.radius;
                let id = e.id;
                // Heavy bodies (an elite, a named miniboss, a Brute, a curled
                // Pillbug) keep their footing: a shorter shove, no stagger.
                let heavy = unhookable(&e);
                let scale = if heavy { LASH_HEAVY_SCALE } else { 1.0 };
                if !heavy {
                    e.stun = e.stun.max(LASH_STAGGER);
                }
                let out = (pos - from).normalized();
                e.vel = out * lash.shove * scale;
                damage_numbers.record(id, pos, radius, applied, killed, DamageSource::Lash);
            }
        }
    }

    // 2. The boss, when the lash was aimed at one of its hit shapes: the full
    //    lash damage there. (It can take the burst once on top, below.)
    if lash.boss {
        for mut boss in bosses.iter_mut() {
            if boss.dead {
                continue;
            }
            let shapes = boss.hit_shapes();
            if shapes.iter().next().is_none()
                || !shapes.circle_hits_grounded(point, BOSS_LOCK_SLOP)
            {
                continue;
            }
            if level
                .tiles
                .segment_hit(from, boss.pos, LASH_LOS_RADIUS)
                .is_some()
            {
                continue;
            }
            let amount = lash.dmg * boss.damage_mult;
            if !boss.take_damage(amount) {
                continue;
            }
            killed_any |= boss.hp <= 0.0;
            ledger.add_boss(kind, amount);
            damage_numbers.record(
                boss.id,
                boss.pos,
                boss.kind.body_radius(),
                amount,
                boss.hp <= 0.0,
                DamageSource::Lash,
            );
        }
    }

    // 3. The burst at the impact point. It never double-hits the primary body
    //    (it took the whole lash instead), and it is floor geometry: masonry
    //    stops it.
    let primary_entity = match lash.primary {
        Some(LashTarget::Enemy(entity)) => Some(entity),
        _ => None,
    };
    for &entity in order {
        if Some(entity) == primary_entity {
            continue;
        }
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || e.pos.dist(point) > lash.radius + e.radius {
            continue;
        }
        if !los_clear(level, point, e.pos, FIELD_LOS_RADIUS) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        // The burst arrives from the impact point, not from the rat.
        let applied = e.hurt_from(burst, point);
        let killed = !e.alive();
        killed_any |= killed;
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let radius = e.radius;
        let id = e.id;
        damage_numbers.record(id, pos, radius, applied, killed, DamageSource::Lash);
    }
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        if !boss.hit_shapes().circle_hits_grounded(point, lash.radius) {
            continue;
        }
        if level
            .tiles
            .segment_hit(point, boss.pos, FIELD_LOS_RADIUS)
            .is_some()
        {
            continue;
        }
        let amount = burst * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        killed_any |= boss.hp <= 0.0;
        ledger.add_boss(kind, amount);
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            boss.hp <= 0.0,
            DamageSource::Lash,
        );
    }

    // 4. Spikes break off and fly on (level 4+): ordinary friendly bullets in a
    //    narrow fan, so every existing projectile rule (walls, shells, the
    //    damage ledger column) applies to them.
    let dir = (point - from).normalized();
    if lash.shards > 0 {
        let base = dir.angle();
        let n = lash.shards as i32;
        for k in 0..n {
            let spread = if n > 1 {
                (k as f32 - (n - 1) as f32 / 2.0) * SHARD_FAN
            } else {
                0.0
            };
            let mut b = Bullet::new(
                point,
                Vec2::from_angle(base + spread) * SHARD_SPEED,
                lash.dmg * LASH_SHARD_FRAC,
                Faction::Player,
                kind.color(),
            );
            b.size = 1;
            b.radius = 1.5;
            b.life = SHARD_LIFE;
            b.weapon = Some(kind);
            commands.spawn((b, next_id.take()));
        }
    }

    // 5. Feedback: the spiked impact star with its burst ring, the shards' own
    //    glints, a small screen shake. All from the impact point.
    vfx::emit_lash_impact(
        particles,
        &mut rng.0,
        point,
        dir,
        lash.radius,
        killed_any,
    );
    if lash.shards > 0 {
        vfx::emit_lash_shards(particles, &mut rng.0, point, dir, lash.shards as usize);
    }
    shake.amount = shake.amount.max(LASH_SHAKE);
}

/// **Barbed Wire** (design: the lash hooks, the recoil reels the catch in).
///
/// The caught body is dragged from the impact point to the rat's feet; each
/// body it is dragged *through* takes one burst, exactly as if the lash had
/// cracked on it. Heavy bodies (elites, named minibosses, `EnemyKind::heavy()`)
/// are not dragged at all — the hook slips off and only the lash hit stands.
///
/// A catch the crack **killed** (the common case: a level-5 lash one-shots most
/// of the horde, and the corpse is reaped before the recoil starts) is still
/// reeled in: the barbs come home along the same line from the impact point
/// and burst everything they cross; there is just no body left to move. Before
/// this the evolution silently did nothing on every lash that killed.
///
/// Returns whether a body moved (the caller re-files the spatial grid).
#[allow(clippy::too_many_arguments)]
fn drag(
    lash: &Lash,
    level: &Level,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
) -> bool {
    let Some(LashTarget::Enemy(primary)) = lash.primary else {
        return false;
    };
    // Everything read off the caught body before the walk below (which needs
    // the query mutably). `None` = the catch is dead or already reaped: the
    // barbs reel home empty from the impact point (which stopped following
    // the body when it died).
    let live = enemies
        .get(primary)
        .ok()
        .and_then(|(_, _, e)| e.alive().then(|| (e.pos, e.radius, unhookable(e))));
    let (from, radius, heavy) = live.unwrap_or((lash.target, 0.0, false));
    // The hook slips off anything heavy: only the lash hit stands.
    if heavy {
        return false;
    }
    // The crack refuses a body that stepped behind masonry during the wind-up;
    // the reel must refuse it too, or a body the lash never touched would be
    // hauled through the wall.
    if !los_clear(level, lash.origin, from, LASH_LOS_RADIUS) {
        return false;
    }
    let gap = LASH_DRAG_GAP + radius;
    let dir = (from - lash.origin).normalized();
    let to = if from.dist(lash.origin) <= gap {
        // Already at the rat's feet: nothing to reel in.
        from
    } else {
        lash.origin + dir * gap
    };

    // Every body the drag crosses takes the burst once, in `SpawnId` order.
    let burst = lash.dmg * LASH_BURST_FRAC;
    let mut crossed = 0u32;
    for &entity in order {
        if entity == primary {
            continue;
        }
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        // Distance from the dragged body's centre to the line it is dragged
        // along: a body whose disc the catch sweeps through is crossed.
        let closest = closest_on_segment(from, to, e.pos);
        if e.pos.dist(closest) > lash.radius + e.radius {
            continue;
        }
        if !los_clear(level, from, e.pos, FIELD_LOS_RADIUS) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        let applied = e.hurt_from(burst, from);
        let killed = !e.alive();
        ledger.add(WeaponKind::SpikedTail, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let enemy_radius = e.radius;
        let id = e.id;
        damage_numbers.record(
            id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::Lash,
        );
        crossed += 1;
    }

    let moved = if live.is_some() && to != from {
        if let Ok((_, _, mut e)) = enemies.get_mut(primary) {
            e.pos = level
                .tiles
                .resolve_circle(to, e.radius, SizeClass::Small, None);
            e.vel = Vec2::ZERO;
            e.stun = e.stun.max(LASH_STAGGER);
            true
        } else {
            false
        }
    } else {
        false
    };
    vfx::emit_lash_drag(particles, &mut rng.0, from, to, crossed);
    moved
}

/// Closest point to `p` on the segment `a..b` (`a` for a degenerate segment).
#[inline]
fn closest_on_segment(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.len_sq();
    if len_sq <= 1e-6 {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

// ---- The tick ------------------------------------------------------------

/// Advance every lash in flight: follow the body, crack, drag, expire.
///
/// Chained right after the auto-fire pass (slot 11b2, `schedule.rs`), so the
/// cached [`EnemyIndex`] is the live `SpawnId` order here and the crack's
/// damage lands before `projectiles::bullets_system`, `combat` and the reap —
/// a spike shard spawned by a crack flies on the same tick and a lash kill is
/// looted on it.
#[allow(clippy::too_many_arguments)]
pub fn spiked_tail_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    player: Res<Player>,
    mut grid: ResMut<SpatialGrid>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut next_id: ResMut<NextSpawnId>,
    mut shake: ResMut<Shake>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut lashes: Query<(Entity, &SpawnId, &mut Lash)>,
    // Reused buffers: the narrow-harness fallback enemy order (the schedule's
    // cached `EnemyIndex` is used whenever it is the live list) and this tick's
    // lash entities in `SpawnId` order.
    mut scratch: Local<LashScratch>,
) {
    // No lash in flight: nothing to advance, and (importantly) no rng draw.
    if lashes.is_empty() {
        return;
    }
    let dt = input.dt;

    // Both scratch buffers at once: one `DerefMut` through the `Local`, then
    // two disjoint fields, so the enemy order can stay borrowed while the lash
    // list is filled (a field split through `Deref` would not be disjoint).
    let LashScratch {
        order: order_buf,
        lashes: lash_buf,
    } = &mut *scratch;

    // The tick's stable enemy order, exactly as `weapons.rs` derives it.
    let cached = index.0.len() == enemies.iter().len();
    if !cached {
        order_buf.clear();
        order_buf.extend(enemies.iter().map(|(entity, _, _)| entity));
        order_buf.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
    }
    let order: &[Entity] = if cached {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        order_buf.as_slice()
    };

    // `sort_by_key` on a mutable query allocates an entity list per call, so
    // the lashes are gathered into the reused buffer and sorted as
    // `(SpawnId, Entity)` — the repo's idiom (`enemy_move`, `combat`).
    lash_buf.clear();
    for (entity, id, _) in lashes.iter() {
        lash_buf.push((id.0, entity));
    }
    lash_buf.sort_unstable_by_key(|&(id, _)| id);

    let mut moved = false;
    for &(_, entity) in lash_buf.iter() {
        let Ok((_, _, mut lash)) = lashes.get_mut(entity) else {
            continue;
        };
        lash.timer += dt;
        // The tail's root follows the rat: the whip stays attached to the rump
        // it is really on, whatever the rat did during the wind-up.
        lash.origin = player.pos;
        // The impact point follows its body while the body lives (a lash that
        // whiffs a walking rat-sized target reads as broken). A boss point is
        // followed through the boss's own body.
        if let Some(LashTarget::Enemy(primary)) = lash.primary {
            if let Ok((_, _, e)) = enemies.get(primary) {
                if e.alive() {
                    lash.target = e.pos;
                }
            }
        } else if lash.boss {
            for boss in bosses.iter() {
                if !boss.dead {
                    lash.target = boss.pos;
                    break;
                }
            }
        }

        if !lash.struck && lash.timer >= LASH_WINDUP {
            lash.struck = true;
            crack(
                &lash,
                &mut commands,
                &level,
                order,
                &mut enemies,
                &mut bosses,
                &mut rng,
                &mut particles,
                &mut damage_numbers,
                &mut ledger,
                &mut next_id,
                &mut shake,
            );
        }
        // Independent of the branch above so a huge `dt` still resolves both
        // beats in one tick (the schedule is fixed-step in practice, but a
        // narrow test drives whatever `dt` it likes).
        if lash.struck && lash.evolved && !lash.dragged && lash.timer >= LASH_RECOIL_AT {
            lash.dragged = true;
            moved |= drag(
                &lash,
                &level,
                order,
                &mut enemies,
                &mut rng,
                &mut particles,
                &mut damage_numbers,
                &mut ledger,
            );
        }
        if lash.timer >= LASH_TOTAL {
            commands.entity(entity).despawn();
        }
    }

    // A drag moved a body. `EnemyIndex` is unchanged (nothing spawned or
    // despawned), so re-filling the buckets from it is one pass and every
    // later reader this tick sees the body where it actually is.
    if moved {
        let arena = level.arena;
        grid.reset(arena, GRID_CELL);
        grid.fill(
            index
                .0
                .iter()
                .filter_map(|&entity| enemies.get(entity).ok().map(|(_, _, e)| e.pos)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::components::LashPhase;
    use crate::game::ecs::resources::{DamageLedger, EnemyIndex};
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS, TileRect};
    use crate::game::loadout::{PassiveKind, Upgrade};
    use crate::game::weapon::{Weapon, LASH_CYCLE, LASH_DAMAGE, LASH_RADIUS, LASH_REACH};
    use crate::game::world::World;
    use rand::SeedableRng;

    const DT: f32 = 1.0 / 60.0;
    /// The view these tests measure the tail in: bigger than any reach on the
    /// table, so the screen cap (`LASH_VIEW_INSET`) never decides a hit and a
    /// test of the *reach* really tests the reach. The cap has its own tests.
    const WIDE_VIEW: Vec2 = Vec2 { x: 1600.0, y: 1600.0 };

    /// Wipe the generated layout's walls so a test measures the weapon, not
    /// the map.
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.level.tiles.rebuild_masks();
    }

    /// A fresh, empty arena with one Spiked Tail armed and ready, the rat
    /// facing +x.
    fn arena(seed: u64, level: u8) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        w.viewport = WIDE_VIEW;
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons = vec![Weapon {
            kind: WeaponKind::SpikedTail,
            level,
            timer: 0.0,
            evolved: false,
        }];
        w
    }

    /// The fixture `weapons.rs` uses for its line-of-sight tests: an open map,
    /// the rat pressed up against (6 px from) a 3-cell solid column whose near
    /// face is `x = 176`. Deliberately close, so "inside the lash's reach but
    /// behind the wall" is a real configuration at level 1.
    fn wall_between(w: &mut World) -> f32 {
        open_map(w);
        w.viewport = WIDE_VIEW;
        w.player.pos = Vec2::new(170.0, 168.0);
        w.player.face = Vec2::new(1.0, 0.0);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        let (cx, cy) = (11, 10);
        w.level
            .tiles
            .fill_cells(cx, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        (cx * 16) as f32
    }

    /// Fire once on a zero-length tick, so the observed cooldown is exactly the
    /// design's cycle. Returns whether a lash was spawned.
    fn run_fire(w: &mut World) -> bool {
        let before = w.player.loadout.weapons[0].timer;
        w.begin_tick(0.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        w.player.loadout.weapons[0].timer > before
    }

    /// [`run_fire`] with a reticle: the lash takes the eligible body nearest
    /// `aim` instead of the one nearest the rat.
    fn run_fire_aimed(w: &mut World, aim: Vec2) -> bool {
        let before = w.player.loadout.weapons[0].timer;
        w.begin_tick_with_aim(0.0, Vec2::ZERO, false, false, Some(aim));
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        w.player.loadout.weapons[0].timer > before
    }

    fn run_lash(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(spiked_tail_system).unwrap();
        w.end_tick();
    }

    /// Every live lash, in `SpawnId` order.
    fn lashes(w: &mut World) -> Vec<Lash> {
        let mut q = w.ecs.query::<(&SpawnId, &Lash)>();
        let mut v: Vec<(u64, Lash)> = q.iter(&w.ecs).map(|(id, l)| (id.0, *l)).collect();
        v.sort_unstable_by_key(|&(id, _)| id);
        v.into_iter().map(|(_, l)| l).collect()
    }

    /// Fire, then run the wind-up out so the crack lands (but not the recoil's
    /// full span, so the lash entity is still there to read).
    fn fire_and_crack(w: &mut World) -> Lash {
        assert!(run_fire(w), "the lash must have a target");
        // One tick past the wind-up lands the crack.
        let ticks = (LASH_WINDUP / DT).floor() as usize + 1;
        for _ in 0..ticks {
            run_lash(w, DT);
        }
        lashes(w).first().copied().expect("the lash is still in flight")
    }

    fn hp(w: &World, entity: Entity) -> f32 {
        w.ecs.get::<Enemy>(entity).unwrap().hp
    }

    fn dealt(w: &World) -> f32 {
        w.ecs.resource::<DamageLedger>().dealt[WeaponKind::SpikedTail.index()]
    }

    fn boss_at(w: &mut World, kind: BossKind, offset: Vec2) -> Entity {
        let arena = w
            .level
            .boss_room()
            .map(|r| r.rect)
            .unwrap_or_else(|| TileRect::new(0, 0, 10, 10));
        let at = w.player.pos + offset;
        let boss = Boss::new(kind, at, 5000.0, arena, at, false);
        w.ecs.spawn((boss, SpawnId(999))).id()
    }

    fn stack(w: &mut World, passive: PassiveKind) {
        for _ in 0..passive.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(passive));
        }
    }

    // ---- Targeting ------------------------------------------------------

    /// One lash at a time, aimed at the nearest body in reach, and the whole
    /// table of level numbers is what the shot actually uses.
    #[test]
    fn the_lash_targets_the_nearest_body_in_reach() {
        let mut w = arena(1, 1);
        let p = w.player.pos;
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 400.0);
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 6.0), 400.0);
        assert!(run_fire(&mut w));
        let l = lashes(&mut w);
        assert_eq!(l.len(), 1, "one lash per shot");
        assert_eq!(l[0].target, p + Vec2::new(20.0, 6.0), "the nearest body");
        assert_eq!(l[0].primary, Some(LashTarget::Enemy(near)));
        assert_eq!(l[0].reach, LASH_REACH[0]);
        assert_eq!(l[0].radius, LASH_RADIUS[0]);
        assert_eq!(l[0].dmg, LASH_DAMAGE[0]);
        assert_eq!(l[0].shards, 0, "no shards below level 4");
        assert!(!l[0].struck, "the crack waits for the wind-up");
        assert_ne!(near, far);
    }

    /// Ties on distance break by `SpawnId` — the lowest id wins — whatever
    /// order the bodies were queried in.
    #[test]
    fn distance_ties_break_by_spawn_id() {
        for flip in [false, true] {
            let mut w = arena(2, 1);
            let p = w.player.pos;
            let a = if flip {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 18.0), 400.0)
            } else {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(18.0, 0.0), 400.0)
            };
            let _b = if flip {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(18.0, 0.0), 400.0)
            } else {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 18.0), 400.0)
            };
            assert!(run_fire(&mut w));
            let l = lashes(&mut w);
            assert_eq!(
                l[0].primary,
                Some(LashTarget::Enemy(a)),
                "the lowest SpawnId wins an exact tie (flip = {flip})"
            );
        }
    }

    /// A body past the reach is not a target at all: the weapon stays ready
    /// instead of serving a cooldown for a whip that would crack at nothing.
    #[test]
    fn out_of_reach_fires_nothing_and_keeps_the_weapon_ready() {
        for level in 1..=5u8 {
            let mut w = arena(3 + level as u64, level);
            let p = w.player.pos;
            let reach = LASH_REACH[(level - 1) as usize];
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(reach + 14.0, 0.0), 400.0);
            assert!(!run_fire(&mut w), "L{level}: out of reach must not fire");
            assert!(lashes(&mut w).is_empty());
            assert_eq!(w.player.loadout.weapons[0].timer, 0.0, "retry next tick");
        }
    }

    /// **The screen caps the tail.** A body 100 px away is inside L5's 120 px
    /// reach, but a 120 x 80 px view (a narrow terminal at 1x) is 60 x 40 px
    /// from the rat: a body past that edge is not a target, so the crack always
    /// lands where the player can see it. The body on screen is still lashed,
    /// and a wide view leaves the reach in charge exactly as before.
    #[test]
    fn a_body_past_the_edge_of_the_screen_is_not_lashed() {
        let small = Vec2::new(120.0, 80.0);
        let mut w = arena(210, 5);
        w.viewport = small;
        let p = w.player.pos;
        const { assert!(LASH_REACH[4] > 100.0, "the reach must run past this view") };
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(100.0, 0.0), 400.0);
        assert!(
            !run_fire(&mut w),
            "a body 100 px out is past a 120 px wide view at 1x"
        );
        assert!(lashes(&mut w).is_empty(), "nothing was thrown off screen");

        // A body the player can see is a target, and the impact point (the
        // burst centre and the drawn tip) is inside the view.
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(50.0, 0.0), 400.0);
        assert!(run_fire(&mut w), "a body on screen is lashed");
        let l = lashes(&mut w)[0];
        assert_eq!(l.primary, Some(LashTarget::Enemy(near)));
        assert_ne!(near, far);
        assert!(
            in_sim_view_inset(l.target, p, small, LASH_VIEW_INSET),
            "the crack landed off screen: {:?}",
            l.target - p
        );

        // The same 100 px body alone, once the view is wide enough to hold it
        // (a fresh arena: the weapon above is on its cooldown).
        let mut w = arena(210, 5);
        w.viewport = Vec2::new(1200.0, 1200.0);
        let p = w.player.pos;
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(100.0, 0.0), 400.0);
        assert!(run_fire(&mut w), "a wide view leaves the reach in charge");
        let l = lashes(&mut w)[0];
        assert_eq!(l.primary, Some(LashTarget::Enemy(far)));
    }

    /// The cap is on the **pick**, so an off-screen body is not even a
    /// candidate: an on-screen body behind it in reach is what the tail takes.
    #[test]
    fn an_off_screen_body_does_not_shadow_an_on_screen_one() {
        for evolved in [false, true] {
            let mut w = arena(211, 5);
            w.viewport = Vec2::new(120.0, 80.0);
            w.player.loadout.weapons[0].evolved = evolved;
            let p = w.player.pos;
            // The off-screen body is the nearest *and* (under Barbed Wire's
            // long cast) the farthest hookable one: it must lose either way.
            let off = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, -70.0), 400.0);
            let on = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(30.0, 0.0), 400.0);
            assert!(run_fire(&mut w), "the on-screen body is a target");
            assert_eq!(
                lashes(&mut w)[0].primary,
                Some(LashTarget::Enemy(on)),
                "evolved = {evolved}: an off-screen body was picked"
            );
            assert_ne!(on, off);
        }
    }

    /// With a huge viewport the tail behaves exactly as it did before the cap:
    /// the farthest body in reach is still eligible at every level.
    #[test]
    fn a_view_bigger_than_the_reach_changes_nothing() {
        for level in 1..=5u8 {
            let mut w = arena(212 + level as u64, level);
            assert_eq!(w.viewport, WIDE_VIEW);
            let p = w.player.pos;
            let reach = LASH_REACH[(level - 1) as usize];
            let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(reach - 1.0, 0.0), 400.0);
            assert!(run_fire(&mut w), "L{level}: a body inside the reach is hit");
            assert_eq!(lashes(&mut w)[0].primary, Some(LashTarget::Enemy(body)));
        }
    }

    /// Reach grows with level: the same body is out of reach at L1 and in
    /// reach at L5 strictly by the table.
    #[test]
    fn reach_grows_with_level() {
        for level in 1..=5u8 {
            let mut w = arena(10 + level as u64, level);
            let p = w.player.pos;
            let reach = LASH_REACH[(level - 1) as usize];
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(reach - 1.0, 0.0), 400.0);
            assert!(run_fire(&mut w), "L{level}: a body inside the reach is hit");
            assert_eq!(lashes(&mut w)[0].reach, reach);
        }
    }

    /// A wall between the rat and the body stops the lash: no target, no shot,
    /// no damage. With a clear second line (the body beside the column) the
    /// weapon fires at *that* body instead.
    #[test]
    fn a_wall_blocks_the_lash() {
        let mut w = arena(20, 1);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        // Straight through the column: 30 px away (well inside L1's 44 px
        // reach), but not visible.
        let hidden = w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 24.0, p.y), 400.0);
        let before = hp(&w, hidden);
        assert!(!run_fire(&mut w), "a walled-off body is not a target");
        assert!(lashes(&mut w).is_empty(), "no lash was thrown at a wall");
        assert_eq!(hp(&w, hidden), before);
        // The same body stepping out from behind the column is reachable.
        let open = Vec2::new(p.x, p.y + 24.0);
        w.ecs.get_mut::<Enemy>(hidden).unwrap().pos = open;
        assert!(run_fire(&mut w), "a clear line fires");
    }

    // ---- The crack ------------------------------------------------------

    /// The primary takes the full lash (per the table for its level), the rest
    /// of the burst takes 40 % of it, the primary is never double-hit, and the
    /// whole thing is booked to the Spiked Tail column.
    #[test]
    fn the_crack_hits_the_primary_once_and_the_burst_for_the_rest() {
        for level in 1..=5u8 {
            let mut w = arena(30 + level as u64, level);
            let p = w.player.pos;
            let dmg = LASH_DAMAGE[(level - 1) as usize];
            let radius = LASH_RADIUS[(level - 1) as usize];
            let primary = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
            // Inside the burst radius of the impact point.
            let splash = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 6.0), 400.0);
            // Well outside it (the gap clears the target's own body radius).
            let far = w.spawn_enemy(
                EnemyKind::Brute,
                p + Vec2::new(20.0, radius + 24.0),
                400.0,
            );
            let before = [hp(&w, primary), hp(&w, splash), hp(&w, far)];
            let _ = fire_and_crack(&mut w);
            let burst = dmg * LASH_BURST_FRAC;
            let lost = |w: &World, i: usize| before[i] - hp(w, [primary, splash, far][i]);
            assert!(
                (lost(&w, 0) - dmg).abs() < 1e-3,
                "L{level}: the primary takes the whole lash, once ({})",
                lost(&w, 0)
            );
            assert!(
                (lost(&w, 1) - burst).abs() < 1e-3,
                "L{level}: the burst lands once at 40 % ({})",
                lost(&w, 1)
            );
            assert_eq!(lost(&w, 2), 0.0, "L{level}: outside the burst");
            let want = (dmg + burst) as f64;
            assert!(
                (dealt(&w) as f64 - want).abs() < 0.05,
                "L{level}: ledger {:.2} != {want:.2}",
                dealt(&w)
            );
        }
    }

    /// Every lash has its own cycle from the table: the weapon fires again
    /// only after `LASH_CYCLE[level - 1]`.
    #[test]
    fn the_lash_fires_on_its_design_cycle() {
        for level in 1..=5u8 {
            let mut w = arena(40 + level as u64, level);
            w.spawn_enemy(
                EnemyKind::Brute,
                w.player.pos + Vec2::new(20.0, 0.0),
                4000.0,
            );
            assert!(run_fire(&mut w));
            let cycle = LASH_CYCLE[(level - 1) as usize];
            let timer = w.player.loadout.weapons[0].timer;
            assert!(
                (timer - cycle).abs() < 1e-4,
                "L{level}: a shot serves {timer:.3} s, the table says {cycle}"
            );
        }
    }

    /// The crack staggers and shoves the primary, through the same shove hook
    /// (Counterweight) the other weapons read; heavy bodies are nudged less
    /// and are not staggered.
    #[test]
    fn the_crack_shoves_and_staggers_the_primary() {
        let mut w = arena(50, 1);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        assert!(hp(&w, body) > 0.0, "the body survives the crack under test");
        let e = w.ecs.get::<Enemy>(body).unwrap();
        assert!(e.stun > 0.0, "a staggered body");
        assert!(e.vel.x > 0.0, "shoved away from the rat: {:?}", e.vel);
        assert!(e.vel.y.abs() < 1e-3);

        // Counterweight multiplies the native shove.
        let mut w = arena(51, 1);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        let bare = w.ecs.get::<Enemy>(body).unwrap().vel.len();
        let mut w = arena(51, 1);
        let p = w.player.pos;
        stack(&mut w, PassiveKind::Counterweight);
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        let heavy_handed = w.ecs.get::<Enemy>(body).unwrap().vel.len();
        assert!(
            heavy_handed > bare * 1.1,
            "Counterweight must shove harder: {bare} -> {heavy_handed}"
        );

        // A Brute is heavy: no stagger, a token shove.
        let mut w = arena(52, 1);
        let p = w.player.pos;
        let brute = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        let e = w.ecs.get::<Enemy>(brute).unwrap();
        assert_eq!(e.stun, 0.0, "a Brute keeps its feet");
        assert!(e.vel.len() < 40.0, "and barely moves: {:?}", e.vel);
    }

    /// Level 4 and 5 break spike shards off the impact: two and three ordinary
    /// friendly bullets in a narrow fan, tagged with the weapon so the ledger
    /// and the impact art credit it.
    #[test]
    fn spike_shards_break_off_from_level_four() {
        let mut shipped = [0usize; 6];
        for level in 1..=5u8 {
            let mut w = arena(60 + level as u64, level);
            w.spawn_enemy(
                EnemyKind::Brute,
                w.player.pos + Vec2::new(20.0, 0.0),
                4000.0,
            );
            let _ = fire_and_crack(&mut w);
            let mut q = w.ecs.query::<&Bullet>();
            let shards: Vec<Bullet> = q
                .iter(&w.ecs)
                .filter(|b| b.weapon == Some(WeaponKind::SpikedTail))
                .cloned()
                .collect();
            shipped[level as usize] = shards.len();
            assert_eq!(shards.len(), crate::game::weapon::LASH_SHARDS[level as usize - 1] as usize);
            for b in &shards {
                assert_eq!(b.faction as u8, Faction::Player as u8);
                assert!((b.damage - LASH_DAMAGE[level as usize - 1] * LASH_SHARD_FRAC).abs() < 1e-4);
            }
            if shards.len() > 1 {
                // A narrow fan: they are not all on one line.
                assert!(
                    shards.iter().any(|b| b.vel.y != shards[0].vel.y),
                    "the shards fan out"
                );
            }
        }
        assert_eq!(shipped[1..], [0, 0, 0, 2, 3], "shards from level 4 only");
    }

    // ---- Passives -------------------------------------------------------

    /// Lens Cap stretches the reach (a melee weapon), Long Fuse widens the
    /// burst (a splash), Sharp Teeth lifts the hit and Frenzy shortens the
    /// cycle — each through its own named hook.
    #[test]
    fn every_passive_hook_reaches_the_lash() {
        // Lens Cap: the recorded reach is the stretched one.
        let mut w = arena(70, 1);
        stack(&mut w, PassiveKind::LensCap);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let want = w.player.loadout.stats.melee_reach(LASH_REACH[0]);
        assert!(want > LASH_REACH[0]);
        assert_eq!(lashes(&mut w)[0].reach, want);

        // Long Fuse: the recorded burst radius is the stretched one.
        let mut w = arena(71, 1);
        stack(&mut w, PassiveKind::LongFuse);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let want = w.player.loadout.stats.splash_radius(LASH_RADIUS[0]);
        assert!(want > LASH_RADIUS[0]);
        assert_eq!(lashes(&mut w)[0].radius, want);

        // Sharp Teeth: more damage actually lands.
        let mut bare = arena(72, 1);
        let p = bare.player.pos;
        let target = bare.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let start = hp(&bare, target);
        let _ = fire_and_crack(&mut bare);
        let plain = start - hp(&bare, target);
        let mut sharp = arena(72, 1);
        stack(&mut sharp, PassiveKind::Damage);
        let p = sharp.player.pos;
        let target = sharp.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let start = hp(&sharp, target);
        let _ = fire_and_crack(&mut sharp);
        let filed = start - hp(&sharp, target);
        assert!(filed > plain * 1.2, "Sharp Teeth: {plain} -> {filed}");

        // Frenzy: the same cooldown serves a shorter cycle.
        let mut w = arena(73, 1);
        stack(&mut w, PassiveKind::FireRate);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let cycled = w.player.loadout.weapons[0].timer;
        assert!(cycled < LASH_CYCLE[0], "Frenzy must shorten the cycle");
    }

    // ---- Bosses ---------------------------------------------------------

    /// The lash targets a boss hit shape and lands the full hit there; the
    /// burst can hit the boss once more on top.
    #[test]
    fn the_boss_takes_the_lash_and_one_burst() {
        for kind in [BossKind::Broom, BossKind::Mower] {
            let mut w = arena(80, 5);
            let boss = boss_at(&mut w, kind, Vec2::new(22.0, 0.0));
            let lash = fire_and_crack(&mut w);
            assert!(lash.boss, "{kind:?}: the pick was a boss hit point");
            assert_eq!(
                lash.primary, None,
                "{kind:?}: a boss has no Enemy entity to follow"
            );
            let lost = 5000.0 - w.ecs.get::<Boss>(boss).unwrap().hp;
            let want = LASH_DAMAGE[4] + LASH_DAMAGE[4] * LASH_BURST_FRAC;
            assert!(
                (lost - want).abs() < 1e-2,
                "{kind:?}: the boss takes the lash and one burst: {lost} != {want}"
            );
            let booked = w.ecs.resource::<DamageLedger>().boss[WeaponKind::SpikedTail.index()];
            assert!((booked - lost).abs() < 1e-2, "{kind:?}: ledger {booked}");
        }
    }

    /// A miniboss is an ordinary `Enemy` body: it is lashed like one (and,
    /// being heavy, is not dragged by the evolved weapon).
    #[test]
    fn a_miniboss_body_takes_the_lash_but_is_never_dragged() {
        use crate::game::ecs::systems::miniboss::MinibossKind;
        let mut w = arena(81, 5);
        let p = w.player.pos;
        let mb = w.spawn_miniboss(MinibossKind::Corkscrew, 0, p + Vec2::new(20.0, 0.0));
        w.player.loadout.weapons[0].evolved = true;
        let before = w.ecs.get::<Enemy>(mb).unwrap().hp;
        let _ = fire_and_crack(&mut w);
        for _ in 0..60 {
            run_lash(&mut w, DT);
        }
        let e = w.ecs.get::<Enemy>(mb).unwrap();
        assert!(e.hp < before, "the miniboss takes the lash");
        assert_eq!(
            e.pos,
            p + Vec2::new(20.0, 0.0),
            "a named miniboss is never dragged"
        );
    }

    // ---- The evolution --------------------------------------------------

    /// **Barbed Wire**: the recoil drags the catch home, each body it crosses
    /// takes the burst once, and heavy bodies are not dragged at all.
    #[test]
    fn barbed_wire_drags_the_catch_through_everything_on_the_way() {
        let mut w = arena(90, 5);
        let p = w.player.pos;
        let caught = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 400.0);
        // Two bodies on the line between the rat and the catch, so the drag
        // sweeps both discs — far enough from the impact that the crack's own
        // burst cannot reach them (this measures the drag, not the splash).
        let crossed_a = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(26.0, 1.0), 400.0);
        let crossed_b = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(9.0, -1.0), 400.0);
        // Off the line: never crossed.
        let aside = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(30.0, 30.0), 400.0);
        let start = [
            hp(&w, caught),
            hp(&w, crossed_a),
            hp(&w, crossed_b),
            hp(&w, aside),
        ];
        w.player.loadout.weapons[0].evolved = true;
        // Auto-fire would take the nearest body (`crossed_b`); the far catch
        // with bodies in between is an aimed lash, pointed at the catch.
        assert!(run_fire_aimed(&mut w, p + Vec2::new(50.0, 0.0)));
        let l = lashes(&mut w);
        assert!(l[0].evolved);
        assert_eq!(l[0].primary, Some(LashTarget::Enemy(caught)));
        let burst = LASH_DAMAGE[4] * LASH_BURST_FRAC;

        // Run past the wind-up and the recoil's start.
        let ticks = (LASH_RECOIL_AT / DT).floor() as usize + 2;
        for _ in 0..ticks {
            run_lash(&mut w, DT);
        }
        let dragged = w.ecs.get::<Enemy>(caught).unwrap().pos;
        assert!(
            dragged.dist(p) < 16.0,
            "the catch is reeled in to the rat's feet: {dragged:?}"
        );
        for (i, crossed) in [crossed_a, crossed_b].into_iter().enumerate() {
            let lost = start[1 + i] - hp(&w, crossed);
            assert!(
                (lost - burst).abs() < 1e-3,
                "a crossed body takes the burst once ({lost} != {burst})"
            );
        }
        assert_eq!(hp(&w, aside), start[3], "a body off the line is untouched");
        // The catch itself keeps the lash hit and is not burst again by the drag.
        let caught_lost = start[0] - hp(&w, caught);
        assert!(
            (caught_lost - LASH_DAMAGE[4]).abs() < 1e-3,
            "the catch takes the lash, not the drag's bursts: {caught_lost}"
        );
    }

    /// Heavy bodies are bookkeeping-only: the hook slips off, the lash hit
    /// stands, and nothing is moved.
    #[test]
    fn barbed_wire_never_drags_a_heavy_body() {
        let mut w = arena(91, 5);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        let brute = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(50.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        for _ in 0..60 {
            run_lash(&mut w, DT);
        }
        assert_eq!(
            w.ecs.get::<Enemy>(brute).unwrap().pos,
            p + Vec2::new(50.0, 0.0),
            "a Brute is not a fish on a line"
        );
    }

    /// The known "Barbed Wire never does anything on auto-aim" bug: the plain
    /// nearest pick leaves nothing between the catch and the rat. An evolved
    /// auto-aimed lash casts at the **farthest hookable** body in reach; the
    /// plain weapon and heavy bodies keep the nearest-first rule.
    #[test]
    fn barbed_wire_auto_aim_casts_at_the_farthest_hookable_body() {
        let layout = |w: &mut World| {
            let p = w.player.pos;
            let near = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(9.0, -1.0), 400.0);
            let far = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 400.0);
            // Farther still, but heavy: the hook would slip, so it is not the
            // long cast's choice while anything hookable is in reach.
            let brute = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 58.0), 400.0);
            (near, far, brute)
        };
        let mut plain = arena(93, 5);
        let (near, _, _) = layout(&mut plain);
        assert!(run_fire(&mut plain));
        assert_eq!(
            lashes(&mut plain)[0].primary,
            Some(LashTarget::Enemy(near)),
            "the plain lash still takes the nearest body"
        );

        let mut barbed = arena(93, 5);
        barbed.player.loadout.weapons[0].evolved = true;
        let (_, far, _) = layout(&mut barbed);
        assert!(run_fire(&mut barbed));
        assert_eq!(
            lashes(&mut barbed)[0].primary,
            Some(LashTarget::Enemy(far)),
            "Barbed Wire casts long, at the farthest hookable body"
        );

        // Only heavy bodies in reach: nearest-first, as before.
        let mut heavy = arena(94, 5);
        heavy.player.loadout.weapons[0].evolved = true;
        let p = heavy.player.pos;
        let close = heavy.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        heavy.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 50.0), 400.0);
        assert!(run_fire(&mut heavy));
        assert_eq!(lashes(&mut heavy)[0].primary, Some(LashTarget::Enemy(close)));
    }

    /// A catch the crack kills is reaped before the recoil; the barbs still
    /// come home along the line and burst what they cross. (Before the fix a
    /// lash that killed its target — most of them at level 5 — never reeled.)
    #[test]
    fn barbed_wire_reels_in_even_when_the_crack_kills_the_catch() {
        let mut w = arena(95, 5);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        let caught = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 400.0);
        // On the line home, outside the crack's own burst.
        let crossed = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(24.0, 1.0), 400.0);
        let start = hp(&w, crossed);
        // One hit point: the crack kills it. The narrow harness has no reap,
        // so despawn it by hand the way `reap_enemies` would the same tick.
        w.ecs.get_mut::<Enemy>(caught).unwrap().hp = 1.0;
        assert!(run_fire_aimed(&mut w, p + Vec2::new(50.0, 0.0)));
        let ticks = (LASH_WINDUP / DT).floor() as usize + 1;
        for _ in 0..ticks {
            run_lash(&mut w, DT);
        }
        assert!(!w.ecs.get::<Enemy>(caught).unwrap().alive(), "the crack killed it");
        assert_eq!(hp(&w, crossed), start, "the crack's burst does not reach the line");
        let _ = w.ecs.despawn(caught);
        for _ in 0..6 {
            run_lash(&mut w, DT);
        }
        let burst = LASH_DAMAGE[4] * LASH_BURST_FRAC;
        let lost = start - hp(&w, crossed);
        assert!(
            (lost - burst).abs() < 1e-3,
            "the empty reel still bursts a crossed body once ({lost} != {burst})"
        );
    }

    /// The evolution changes behaviour, not numbers: same cycle, same damage
    /// per hit, same reach.
    #[test]
    fn barbed_wire_keeps_the_timing_family() {
        let mut w = arena(92, 3);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let l = lashes(&mut w)[0];
        assert_eq!(l.dmg, LASH_DAMAGE[2]);
        assert_eq!(l.reach, LASH_REACH[2]);
        assert_eq!(l.radius, LASH_RADIUS[2]);
        assert!((w.player.loadout.weapons[0].timer - LASH_CYCLE[2]).abs() < 1e-4);
    }

    // ---- Ownership / determinism ---------------------------------------

    /// Without the weapon nothing happens at all: no lash ever appears, no
    /// damage is booked, and a run without it is the same run.
    #[test]
    fn nothing_changes_when_the_weapon_is_not_owned() {
        let mut a = World::new(Theme::Sewer, 100);
        open_map(&mut a);
        a.clear_enemies();
        a.player.loadout.weapons.clear();
        let mut b = World::new(Theme::Sewer, 100);
        open_map(&mut b);
        b.clear_enemies();
        b.player.loadout.weapons.clear();
        let p = a.player.pos;
        a.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 400.0);
        b.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 400.0);
        for _ in 0..90 {
            a.update(DT, Vec2::ZERO, false);
            b.update(DT, Vec2::ZERO, false);
        }
        assert!(lashes(&mut a).is_empty(), "an unowned lash never spawns");
        assert_eq!(dealt(&a), 0.0, "and never deals a point of damage");
        // The same seed plays out identically with and without the weapon.
        assert_eq!(a.player.pos, b.player.pos);
        assert_eq!(a.kills, b.kills);
        assert_eq!(a.enemies().len(), b.enemies().len());
    }

    /// Twin runs: the same seed and the same inputs land the same hits, in the
    /// same order, on every level and both evolution states.
    #[test]
    fn twin_runs_are_deterministic() {
        fn trace(level: u8, evolved: bool) -> (u64, f32) {
            let mut w = arena(123, level);
            w.player.loadout.weapons[0].evolved = evolved;
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            let p = w.player.pos;
            for i in 0..8 {
                let a = i as f32 * 0.8;
                w.spawn_enemy_with_rng(
                    EnemyKind::Brute,
                    p + Vec2::from_angle(a) * (18.0 + i as f32 * 5.0),
                    400.0,
                    &mut rand::rngs::StdRng::seed_from_u64(i as u64),
                );
            }
            for tick in 0..240u32 {
                let dir = match (tick / 30) % 4 {
                    0 => Vec2::new(1.0, 0.0),
                    1 => Vec2::new(0.0, 1.0),
                    2 => Vec2::new(-1.0, 0.0),
                    _ => Vec2::new(0.0, -1.0),
                };
                w.update(DT, dir, false);
                if tick % 12 == 0 {
                    for v in [
                        w.player.pos.x.to_bits() as u64,
                        w.player.pos.y.to_bits() as u64,
                        w.kills as u64,
                    ] {
                        h ^= v;
                        h = h.wrapping_mul(0x100_0000_01b3);
                    }
                    for e in w.enemies() {
                        for v in [e.pos.x.to_bits() as u64, e.pos.y.to_bits() as u64, e.hp.to_bits() as u64] {
                            h ^= v;
                            h = h.wrapping_mul(0x100_0000_01b3);
                        }
                    }
                }
            }
            (h, dealt(&w))
        }
        for level in [1u8, 3, 5] {
            for evolved in [false, true] {
                let a = trace(level, evolved);
                let b = trace(level, evolved);
                assert_eq!(a, b, "L{level} evolved={evolved}: twin runs diverged");
                assert!(a.1 > 0.0, "L{level} evolved={evolved}: nothing was hit");
            }
        }
    }

    /// The lash never spawns two entities for one shot, and the wind-up's
    /// crack is exactly once however the ticks are cut.
    #[test]
    fn one_crack_per_lash() {
        let mut w = arena(130, 1);
        let p = w.player.pos;
        let target = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let before = hp(&w, target);
        assert!(run_fire(&mut w));
        for _ in 0..90 {
            run_lash(&mut w, DT);
        }
        assert!((before - hp(&w, target) - LASH_DAMAGE[0]).abs() < 1e-3);
        assert!(lashes(&mut w).is_empty(), "the lash expired");
    }

    /// The index the grid and the burst both use stays the live `SpawnId`
    /// order: a dragged body is re-filed so a bullet still finds it.
    #[test]
    fn a_dragged_body_is_re_filed_in_the_grid() {
        let mut w = arena(140, 5);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        let caught = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        // The single-system harness has no schedule in front of it: build the
        // index and the grid the way the tick does before the lash runs.
        w.begin_tick(0.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::enemy_move::grid_build_system)
            .unwrap();
        w.end_tick();
        let ticks = (LASH_RECOIL_AT / DT).floor() as usize + 2;
        for _ in 0..ticks {
            run_lash(&mut w, DT);
        }
        let pos = w.ecs.get::<Enemy>(caught).unwrap().pos;
        assert!(pos.dist(p) < 14.0);
        // The grid lists the body under its *new* cell.
        let grid = w.ecs.resource::<SpatialGrid>();
        let index = w.ecs.resource::<EnemyIndex>();
        let want = index.0.iter().position(|&e| e == caught).unwrap() as u32;
        let mut out: Vec<u32> = Vec::new();
        grid.neighbors_within(pos, 1.0, &mut out);
        assert!(
            out.contains(&want),
            "the dragged body is filed at its new position"
        );
    }

    // ---- Batch 12: the Spiked Tail's contract (mechanic FROZEN) ----------

    /// The Spiked Tail is an ordinary auto-firing weapon: it is in
    /// `WeaponKind::all()`, the level-up pool offers it as a new kind, and an
    /// owned copy gets rank-up cards — nothing about batch 12 changed that.
    #[test]
    fn spiked_tail_is_offered_like_every_other_weapon() {
        use crate::game::loadout::Loadout;
        assert!(WeaponKind::all().contains(&WeaponKind::SpikedTail));
        assert!(!WeaponKind::SpikedTail.is_active());
        assert!(!WeaponKind::SpikedTail.fires_outside_fire_one());

        // A fresh rack can be dealt the weapon (over enough draws that the
        // shuffle cannot hide it).
        let empty = Loadout::from_parts(Vec::new(), Vec::new());
        let mut saw_new = false;
        for seed in 0..64u64 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            for card in empty.generate_choices(&mut rng) {
                if matches!(card, Upgrade::NewWeapon(WeaponKind::SpikedTail)) {
                    saw_new = true;
                }
            }
        }
        assert!(saw_new, "the level-up pool must offer the Spiked Tail");

        // An owned, un-maxed copy is a rank-up card.
        let mut owned = Weapon::new(WeaponKind::SpikedTail);
        owned.level = 3;
        let rack = Loadout::from_parts(vec![owned], Vec::new());
        let mut saw_rank = false;
        for seed in 0..64u64 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            for card in rack.generate_choices(&mut rng) {
                if matches!(card, Upgrade::LevelWeapon(0)) {
                    saw_rank = true;
                }
            }
        }
        assert!(saw_rank, "an owned Spiked Tail must get rank-up cards");
    }

    /// Auto-aim is reliable: with a crowd all around it, the Spiked Tail takes a
    /// target on **every ready cycle** (never a dud retry) and so fires at its
    /// own cadence — 1.3 s at L1, 0.85 s at L5.
    #[test]
    fn spiked_tail_fires_every_ready_cycle_in_a_crowd() {
        for level in [1u8, 5] {
            let mut w = arena(150, level);
            let p = w.player.pos;
            // A ring of bodies all round the rat, well inside the reach
            // (L1 44 px), including behind it: auto-aim may take any of them.
            for k in 0..12 {
                let a = k as f32 / 12.0 * std::f32::consts::TAU;
                w.spawn_enemy(
                    EnemyKind::Skitterer,
                    p + Vec2::from_angle(a) * 24.0,
                    1000.0,
                );
            }
            let mut want = Weapon::new(WeaponKind::SpikedTail);
            want.level = level;
            let cooldown = want.cooldown();
            let seconds = 10.0f32;
            let ticks = (seconds / DT) as usize;
            let mut fires = 0usize;
            let mut duds = 0usize;
            for _ in 0..ticks {
                let ready = w.player.loadout.weapons[0].timer <= 0.0;
                let before = w.player.loadout.weapons[0].timer;
                w.begin_tick(DT, Vec2::ZERO, false);
                w.ecs
                    .run_system_cached(fire_weapons_system)
                    .expect("the weapon pass");
                w.ecs
                    .run_system_cached(spiked_tail_system)
                    .expect("the lash pass");
                w.end_tick();
                if w.player.loadout.weapons[0].timer > before {
                    fires += 1;
                } else if ready {
                    duds += 1;
                }
                // Keep every body alive and planted: this measures the fire
                // cadence, not the damage.
                let mut q = w.ecs.query::<&mut Enemy>();
                for mut e in q.iter_mut(&mut w.ecs) {
                    e.hp = e.max_hp;
                    e.vel = Vec2::ZERO;
                }
            }
            let expected = (seconds / cooldown) as usize;
            assert_eq!(duds, 0, "L{level}: a ready lash found no target in a crowd");
            assert!(
                fires + 1 >= expected,
                "L{level}: only {fires} lashes in {seconds} s, the {cooldown} s cadence allows {expected}"
            );
        }
    }

    /// The art contract: one lash walks through `WindUp → Crack → Recoil`
    /// exposing its base point, its followed target, a progress in `0..1` for
    /// each beat, the exact crack tick, and the Barbed Wire flags.
    #[test]
    fn a_lash_exposes_its_phases_base_and_tip_to_the_art() {
        let mut w = arena(151, 1);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 1000.0);
        assert!(run_fire(&mut w), "the lash must have a target");
        let crack_tick = lashes(&mut w)[0].crack_tick(DT);
        assert_eq!(
            crack_tick,
            (LASH_WINDUP / DT).ceil() as u32,
            "the crack tick is the wind-up measured in ticks"
        );

        // The wind-up: bristling, target followed, a countdown to the crack.
        run_lash(&mut w, DT);
        let lash = lashes(&mut w)[0];
        assert_eq!(lash.base(), w.player.pos, "the base is the rat's rump");
        assert_eq!(lash.phase(), LashPhase::WindUp);
        assert!(lash.progress() > 0.0 && lash.progress() < 1.0);
        assert!(lash.phase_progress() > 0.0 && lash.phase_progress() < 1.0);
        assert!(!lash.cracked());
        assert!(lash.ticks_to_crack(DT) > 0);
        assert!(!lash.barbed_wire());
        assert_eq!(lash.drag_tick(DT), None);
        assert!((lash.crack_time() - LASH_WINDUP).abs() < 1e-6);

        // The body walks during the wind-up: the tip follows it every tick.
        w.ecs.get_mut::<Enemy>(body).unwrap().pos = p + Vec2::new(24.0, 4.0);
        run_lash(&mut w, DT);
        assert!(
            lashes(&mut w)[0].tip().dist(p + Vec2::new(24.0, 4.0)) < 1e-3,
            "the tip must follow the target body"
        );

        // The crack tick: `cracked_now` is true on exactly that one frame.
        let mut crack_frame = None;
        for i in 0..(LASH_WINDUP / DT).ceil() as usize + 6 {
            run_lash(&mut w, DT);
            let lash = lashes(&mut w)[0];
            if lash.cracked_now(DT) {
                crack_frame = Some(i);
                assert_eq!(lash.phase(), LashPhase::Crack);
                assert_eq!(lash.ticks_to_crack(DT), 0);
            }
        }
        assert!(crack_frame.is_some(), "the crack never reported itself");
        // …and every beat after it is the crack or the recoil, through to the
        // despawn at `LASH_TOTAL`.
        while !lashes(&mut w).is_empty() {
            run_lash(&mut w, DT);
            if let Some(lash) = lashes(&mut w).first().copied() {
                assert_ne!(lash.phase(), LashPhase::WindUp);
                assert!(lash.progress() > 0.0 && lash.progress() <= 1.0);
            }
        }
    }

    /// Barbed Wire: the same exposure names the drag beat and reports it once.
    #[test]
    fn a_barbed_wire_lash_exposes_its_drag_tick() {
        let mut w = arena(152, 1);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        // Out of the swing's own arc reach, so it is hooked and reeled in.
        let _ = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(40.0, 0.0), 1000.0);
        assert!(run_fire(&mut w), "the hooking lash must have a target");
        let lash = lashes(&mut w)[0];
        assert!(lash.barbed_wire());
        assert!(lash.drag_tick(DT).is_some());
        let mut dragged_frames = 0;
        while !lashes(&mut w).is_empty() {
            run_lash(&mut w, DT);
            if let Some(lash) = lashes(&mut w).first().copied() {
                if lash.dragged_now(DT) {
                    dragged_frames += 1;
                    assert_eq!(lash.drag_tick(DT), Some((LASH_RECOIL_AT / DT).ceil() as u32));
                }
            }
        }
        assert_eq!(dragged_frames, 1, "the drag reports exactly one frame");
    }
}
