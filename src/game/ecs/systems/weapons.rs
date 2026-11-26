//! (d) Weapon firing: `fire_weapons`, `fire_one` for every weapon, and
//! line-of-sight targeting.
//!
//! Ported from the legacy `World` (then replayed against the merged neon
//! branch):
//! - `nearest_of` — plain nearest pick, still shared with the
//!   `World::nearest_enemy` facade. It is no longer what the aiming weapons
//!   use: see [`nearest_visible`] and the "shots respect walls" section below.
//! - `fire_weapons` (cooldown, `fire_rate_mult`, `damage_mult`, dud-shot retry
//!   rule).
//! - `fire_one` — the six original weapons (Gnaw, Cheese Spray, Squeak Nova,
//!   Spore Orbit, Tail Whip, Acid Spit) plus the dispatch arms for Acid Rain,
//!   Hantavirus and Spiked Tail; the ACTIVE weapons, Laser Eyes and Super Dash
//!   fire from their own systems. Includes Gnaw's bite and Tail Whip's crack
//!   (both direct `Enemy::hurt`, no projectile) and Spore Orbit's "carry the
//!   angle, drop the old ring" rule.
//! - `nova_ring` — **Squeak Nova**'s expanding ring (batch 12): stepped from
//!   `fire_weapons_system` every tick, so the shockwave is one system's
//!   business and needs no schedule slot of its own.
//!
//! ## Batch 12: the weapons that play differently
//!
//! Six weapons changed *shape* so the look-alike groups read apart in one
//! glance. Each one keeps the damage a body in its region takes per cycle:
//!
//! | weapon | before | now |
//! |---|---|---|
//! | Gnaw | a homing dart (a projectile) | a frontal **bite**: no projectile, a 20-24 px jaw, single target |
//! | Cheese Spray | a long fan of darts | a short fan of **slow, fat wedges** whose cone slows |
//! | Squeak Nova | a ring of radial darts | one **expanding ring** that damages what its front passes |
//! | Tail Whip | a melee aura all round | a **frontal half-circle swipe** (180°, 30-42 px) |
//! | Sonic Squeak | an expanding ring all round | a forward **cone** of 38° |
//! | Laser Eyes | a fan of timed pulses | one continuous **sweep** across an arc |
//!
//! Tail Thrash (a full disc with knockback), Spiked Tail (a lash with a burst at
//! the impact), Acid Spit (one lobbed glob into a pool) and Acid Rain
//! (staggered drops, no lingering pool) already played the way the design table
//! says, and were left alone.
//!
//! Neon-branch hooks re-applied here (`fire_one`):
//! - `b.weapon = Some(kind)` on every player projectile.
//! - `vfx::emit_weapon_fire(particles, rng, kind, ppos, aim, radius)` replacing
//!   the old inline muzzle/pulse particles (same call sites, same arguments).
//! - Tail Whip: `e.alive()` guard, `vfx::emit_tail_whip_hit(...)` and
//!   `damage_numbers.record(..., DamageSource::TailWhip)` per clipped enemy.
//! - Weapon-specific `Sfx` variants when a shot fires.
//!
//! ## Shots respect walls: per-weapon rules
//!
//! The player has no aim button, so the game picks the threat: an aiming
//! weapon takes the **nearest body it can actually reach** — within
//! [`target_range`] and with a clear tile line ([`projectiles::los_clear`]) at
//! the shot's own radius — keeping `SpawnId` order for distance ties. If
//! nothing qualifies the weapon falls back to exactly today's no-target
//! behaviour (Gnaw and Cheese Spray again next tick with no dud cooldown, the
//! spray and Acid Spit fired along the facing as before).
//!
//! **The boss is just another body in that queue.** Its grounded
//! [`BossTargetable`] points (the art's hit-shape centres) enter
//! [`nearest_visible`] alongside the enemies under the same reach, line test
//! and [`TARGET_LOS_CANDIDATES`] cap, so a horde standing nearer still screens
//! it and the boss never gets priority treatment. A boss with no grounded point
//! (the cat mid-`Pounce`, the broom's lifted slipper) is not targetable. On an
//! exact distance tie the enemy wins, then the lower boss point index.
//!
//! Walls mean different things per weapon, and each one stays useful in a
//! two-cell (32 px) corridor:
//!
//! | weapon | walls | why |
//! |---|---|---|
//! | Gnaw | aimed at a visible foe; the bite needs a clear line | the point of the LOS rule: no biting through cover |
//! | Cheese Spray | aimed at a visible foe; the wedges stop on walls; the slowing field needs a clear line too | point-blank it still delivers the whole wedge fan |
//! | Squeak Nova | the ring stops on walls (a body behind masonry is never passed) | a shockwave that ignores walls makes corridors meaningless |
//! | Spore Orbit | **passes through walls** (projectiles exempt from the sweep); damage still needs a line from the rat | a 24 px orbit cannot exist in a 16 px half-width corridor, so destroy-on-contact would silently empty the weapon there — and its damage is point-blank anyway |
//! | Tail Whip | the swipe needs a clear line per body it covers | a whip may not reach through a maze partition |
//! | Acid Spit | lob lands on the first wall (unchanged), now also on a closed one-way gate; targets only a visible foe | the pool is area denial in the open, not artillery through cover |
//!
//! Resources: `Res<TickInput>` (dt), `ResMut<Player>`, `Res<Level>`,
//! `ResMut<RngRes>`, `ResMut<Particles>`, `ResMut<SfxQueue>`,
//! `Commands` + `ResMut<NextSpawnId>`, `ResMut<DamageNumbers>`,
//! `Res<EnemyIndex>` (stable order), `Query<&Boss>` (read-only: the boss's
//! target points for the aim pick) and three `Local` scratch buffers.
//!
//! ## Performance
//!
//! No per-tick heap allocation and no repeated sorting:
//! - The enemy iteration order is taken from `EnemyIndex`, which
//!   `enemy_move::grid_refresh_system` rebuilds in `SpawnId` order immediately
//!   before this system; no enemy is spawned or despawned in between, so it is
//!   complete and current. Narrow single-system tests that never ran the grid
//!   are detected by a length check and fall back to sorting a reused `Local`
//!   scratch **once** for the tick (never once per target pick).
//! - [`nearest_visible`] walks that order exactly once per aiming weapon, keeps
//!   only the [`TARGET_LOS_CANDIDATES`] nearest in-range enemies in a reused
//!   `Local` (`O(n)` distance tests, no allocation) and then pays one tile
//!   query per candidate, nearest first, returning at the first clear line. So
//!   a shot costs one pass plus at most 12 segment queries, never one per
//!   enemy.
//! - Tail Whip reuses the same order instead of sorting the enemy query.
//! - Spore Orbit's orbit-bullet scan is a single unsorted pass that keeps the
//!   lowest-`SpawnId` angle and collects entities into a reused `Local` scratch
//!   instead of sorting the bullet query twice.
//!
//! **Stable order:** enemy iteration is `SpawnId` order (exactly legacy `Vec`
//! order before any `swap_remove` shuffle), so nearest-enemy ties and Tail
//! Whip's per-enemy rng draws are deterministic. Dead enemies are skipped by
//! the targeting pick; `total_cmp` keeps NaN distances last exactly as legacy.

// `fire_one` mirrors the legacy signature (weapon stats + the whole sim as
// arguments) and the loadout loop needs Vec indices, exactly like legacy.
#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]

use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::detmath::DetF32;
use crate::game::bullet::{Bullet, Faction, Lob, Orbit};
// Wave 3: Counterweight's melee nudge keeps its body on legal floor.
use crate::game::collision::SizeClass;
use crate::game::ecs::systems::weapons_wave3b::MELEE_NUDGE_COOLDOWN;
use crate::game::weapon::{
    BITE_SECS, CHEESE_SLOW_MULT, CHEESE_SLOW_SECS, CHEESE_WEDGE_LIFE, CHEESE_WEDGE_SPEED,
    GNAW_BITE_HALF, GNAW_BITE_REACH, NOVA_EXPAND, NOVA_REACH, SQUEAK_BOSS_SLOW,
    SQUEAK_BOSS_SLOW_SECS, SQUEAK_CONE_HALF, SQUEAK_ELITE_SLOW, SQUEAK_EXPAND, SQUEAK_SHOVE,
    SQUEAK_STUN, THRASH_BOSS_KNOCKBACK, THRASH_BURST, THRASH_KNOCKBACK, THRASH_SHAKE,
    THRASH_SWEEP_RATE, THRASH_SWEEPS, WHIP_ARC_HALF, WHIP_REACH, WHIP_SWIPE_SECS,
};
use crate::game::ecs::components::{
    BiteSnap, Entity, EvolvedShot, FairyRing, MilkTooth, SpawnId, WhipCrack,
};
use crate::game::ecs::systems::evolutions;
use crate::game::ecs::resources::{
    ActiveFx, Cvars, DamageLedger, EnemyIndex, MeleeNudge, NextSpawnId, RngRes, SfxQueue, Shake,
    TickInput, Viewport,
};
use crate::game::ecs::systems::boss::{Boss, BossTargetable};
use crate::game::ecs::systems::projectiles::los_clear;
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;

pub fn fire_weapons_system(
    mut commands: Commands,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut next_id: ResMut<NextSpawnId>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    bullets: Query<(Entity, &SpawnId, &Bullet)>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    // Wave 3: Counterweight's "once per target per 0.4 s" melee-nudge log.
    mut melee_nudge: ResMut<MeleeNudge>,
    mut scratch: FireScratch,
) {
    let dt = input.dt;
    // The reticle (wave 3 controls): `None` = auto-aim exactly as before.
    let aim = input.aim;
    // Batch 12: age the melee weapons' snapshots (a bite and a whip crack each
    // live a tenth of a second). Despawn is deferred, so the component exists
    // for the whole frame it was spawned in.
    for (entity, mut snap) in scratch.bites.iter_mut() {
        snap.t -= dt;
        if snap.t <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut crack) in scratch.cracks.iter_mut() {
        crack.t -= dt;
        if crack.t <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
    // Legacy: snapshot the player state and the two multipliers.
    let ppos = player.pos;
    let pface = player.face;
    let fire_rate = player.loadout.stats.fire_rate_mult;
    let dmg_mult = player.damage_mult() * scratch.cvars.damage_mult.max(0.0);
    // Wave 3: the passive hooks (Lens Cap, Long Fuse, Counterweight) are a copy
    // of the stat block, so `fire_one` can read them while the loop still
    // mutates weapon timers.
    let stats = player.loadout.stats;
    let n = player.loadout.weapons.len();

    // One stable enemy order for the whole tick. `EnemyIndex` is rebuilt by
    // `enemy_move::grid_refresh_system` right before this system and nothing
    // spawns or despawns an enemy in between, so it is the `SpawnId`-ordered
    // enemy list. Validate its length so the narrow single-system harness
    // (which never runs the grid) still targets correctly: sort a reused
    // scratch once, not once per lookup.
    // `ExactSizeIterator::len` is O(1) (no per-tick allocation or scan).
    let enemy_count = enemies.iter().len();
    let order: &[Entity] = if index.0.len() == enemy_count {
        // Debug-only guard: the cached order must still be the live
        // `SpawnId` order before it decides targeting (see `enemy_move`).
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        scratch.fallback_order.clear();
        scratch
            .fallback_order
            .extend(enemies.iter().map(|(entity, _, _)| entity));
        scratch.fallback_order.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        scratch.fallback_order.as_slice()
    };

    // The boss's grounded target points for this tick, as plain bodies in the
    // aiming queue (`Boss::target_points`; empty while it has no grounded hit
    // shape, e.g. the cat mid-pounce). One live boss is the invariant, and the
    // lowest `id` breaks a hypothetical tie so the pick never depends on query
    // order.
    let mut boss_points = BossTargetable::empty();
    {
        let mut best: Option<&Boss> = None;
        for boss in bosses.iter() {
            if boss.dead {
                continue;
            }
            if best.is_none_or(|b| boss.id < b.id) {
                best = Some(boss);
            }
        }
        if let Some(boss) = best {
            boss_points = boss.target_points();
        }
    }

    for i in 0..n {
        let (kind, wlevel, dmg, cd, projs, evolved) = {
            let w = &player.loadout.weapons[i];
            (
                w.kind,
                w.level,
                w.damage() * dmg_mult,
                (w.cooldown() / fire_rate).max(0.05),
                w.level_projectiles(),
                w.evolved,
            )
        };
        // Active weapons never auto-fire: their `timer` is a charge meter that
        // `active_weapons_system` counts down and the player spends. Skipping
        // them here also leaves their timer strictly to that system, so neither
        // can double-tick it. Wave 3's Laser Eyes (a burst) and Super Dash (the
        // dash itself) are owned by `weapons_wave3b` for the same reason.
        if kind.is_active() || kind.fires_outside_fire_one() {
            continue;
        }
        player.loadout.weapons[i].timer -= dt;
        if player.loadout.weapons[i].timer > 0.0 {
            continue;
        }
        let fired = fire_one(
            &mut commands,
            kind,
            wlevel,
            evolved,
            dmg,
            cd,
            projs,
            ppos,
            pface,
            &level,
            &mut rng,
            &mut particles,
            &mut damage_numbers,
            &mut ledger,
            &mut sfx,
            &mut next_id,
            order,
            &mut enemies,
            &bullets,
            &mut scratch.orbit_scratch,
            &mut scratch.target_scratch,
            &boss_points,
            &mut bosses,
            &stats,
            &mut melee_nudge,
            &mut scratch.evo,
            &mut scratch.moved,
            aim,
            // Batch 12: the Squeak Nova's ring is the weapon's live effect; the
            // arm below seats it and `nova_ring` (after this loop) steps it.
            &mut scratch.active,
            // The unzoomed view, for the one weapon whose reach runs past it.
            scratch.viewport.0,
            &scratch.droppings,
        );
        if fired {
            player.loadout.weapons[i].timer += cd;
        } else {
            // Nothing spawned (e.g. Gnaw with no live target): retry next
            // tick instead of serving a full cooldown for a dud shot.
            player.loadout.weapons[i].timer = 0.0;
        }
    }

    // Batch 12: **Squeak Nova**'s expanding ring. It is an ordinary
    // auto-firing weapon (no charge meter), so nothing else owns its clock; the
    // front is a pure function of its age, and `EnemyIndex` is live here with
    // nothing reaped yet, exactly like the rest of this pass.
    if scratch.active.nova_active() {
        nova_ring(
            dt,
            &level,
            &mut rng,
            &mut particles,
            order,
            &mut enemies,
            &mut bosses,
            &mut damage_numbers,
            &mut ledger,
            &mut scratch.active,
        );
    }

    // A weapon moved a body this tick (Counterweight's nudge or Ratchet Tail's
    // swing): re-file the buckets from the still-valid `EnemyIndex` so every
    // later reader (`bullets`, `pools`, `boss_damage`, `player_bullet_hits`,
    // `contact`) sees the body where it actually is. One grid fill, and only on
    // a tick that moved something; `SpawnId` order is unchanged, so the index
    // is never sorted again here.
    if *scratch.moved {
        *scratch.moved = false;
        let arena = level.arena;
        scratch.grid.reset(arena, crate::game::ecs::constants::GRID_CELL);
        let grid = &mut scratch.grid;
        grid.fill(
            index
                .0
                .iter()
                .filter_map(|&entity| enemies.get(entity).ok().map(|(_, _, e)| e.pos)),
        );
    }
}

/// Plain, allocation-free nearest-enemy pick.
///
/// Shared by [`fire_weapons_system`] and the `World::nearest_enemy` facade
/// (worker h calls it with `(Entity, SpawnId, pos, alive)` tuples pulled from
/// a `(Entity, &SpawnId, &Enemy)` query). Dead candidates are skipped; the
/// winner is the smallest `dist_sq` under `total_cmp` (NaN sorts last), and an
/// exact tie keeps the lowest [`SpawnId`] — the deterministic form of legacy's
/// "first minimum in `Vec` order". The result does not depend on the caller's
/// iteration order.
pub fn nearest_of<I>(candidates: I, from: Vec2) -> Option<(Entity, Vec2)>
where
    I: IntoIterator<Item = (Entity, SpawnId, Vec2, bool)>,
{
    let mut best: Option<(Entity, Vec2, f32, SpawnId)> = None;
    for (entity, id, pos, alive) in candidates {
        if !alive {
            continue;
        }
        let dist = pos.dist_sq(from);
        best = match best {
            None => Some((entity, pos, dist, id)),
            Some((best_entity, best_pos, best_dist, best_id)) => {
                match dist.total_cmp(&best_dist) {
                    Ordering::Less => Some((entity, pos, dist, id)),
                    Ordering::Equal if id.0 < best_id.0 => Some((entity, pos, dist, id)),
                    _ => Some((best_entity, best_pos, best_dist, best_id)),
                }
            }
        };
    }
    best.map(|(entity, pos, _, _)| (entity, pos))
}

// ---- Targeting: the nearest enemy the rat can actually reach ----------------

/// Most line-of-sight queries one aiming weapon may spend per shot. The pick
/// walks nearest-first and returns at the first clear line, so a map full of
/// enemies behind cover costs the same as a handful of nearby ones.
///
/// The cap is shared by the boss's target points (see [`BossTargetable`]): a
/// boss is only ever *one more body* in the horde's queue, so a screen of
/// twelve nearer enemies still keeps the weapons off it.
pub const TARGET_LOS_CANDIDATES: usize = 12;

/// Line-of-sight radius for a melee effect (a whip is not a projectile).
const MELEE_LOS_RADIUS: f32 = 1.0;

/// How far a weapon will even consider a target, in px: the shot's own reach
/// (`life x speed`) rounded down, so "in range" means the projectile can
/// actually arrive. Radial and aura weapons do not aim at a body (0.0).
fn target_range(kind: WeaponKind) -> f32 {
    match kind {
        // 2.5 s life x 120 px/s.
        WeaponKind::Gnaw => 300.0,
        // 1.4 s life x 110 px/s.
        WeaponKind::CheeseSpray => 154.0,
        // A lob slower than this is a lost volley: 200 px is 1.8 s of flight.
        WeaponKind::AcidSpit => 200.0,
        // An active weapon never aims: it hits its own reach (the thrash arc,
        // the squeak ring) and is fired by the player, not by a cooldown.
        WeaponKind::SqueakNova
        | WeaponKind::SporeOrbit
        | WeaponKind::TailWhip
        | WeaponKind::TailThrash
        | WeaponKind::SonicSqueak => 0.0,
        // Wave 3: Laser Eyes aims with the rat's locked facing (a sweep, not a
        // target pick) and Super Dash has no target at all — both are owned by
        // `weapons_wave3b`, which never asks this function.
        WeaponKind::LaserEyes | WeaponKind::SuperDash => 0.0,
        // Wave 3 weapons A mark/tag bodies with their own picks (they need the
        // *entities* and several marks per volley, which `nearest_visible`
        // deliberately does not return). `weapons_wave3a` uses the same reach
        // (`RAIN_RANGE`), the same cap and the same tie order.
        WeaponKind::AcidRain => crate::game::weapon::RAIN_RANGE,
        // Hantavirus tags nothing at range any more: the rat leaves a dropping
        // where it stands and `weapons_wave3a::droppings_system` infects the
        // first body that steps in one (`DROPPING_RADIUS`). It has no target to
        // pick, so nothing here may hold its fire.
        WeaponKind::Hantavirus => 0.0,
        // The lash picks through `weapon_spiketail::pick_lash_target` (it
        // needs the *entity*, not just a point, so the impact can follow the
        // body); this is the longest reach any level of it has.
        WeaponKind::SpikedTail => crate::game::weapon::LASH_REACH[4],
    }
}

/// The aim direction from an aim **point** (world pixels), or `None` when
/// there is no aim or the reticle sits on the rat.
fn aim_dir(ppos: Vec2, aim: Option<Vec2>) -> Option<Vec2> {
    aim.map(|p| p - ppos)
        .filter(|v| v.len_sq() > 4.0)
        .map(|v| v.normalized())
}

/// The unit direction a whole-cast weapon fires along (batch 12's cone
/// weapons): the reticle when the player aims, else the rat's facing, else up.
fn cast_dir(ppos: Vec2, pface: Vec2, aim: Option<Vec2>) -> Vec2 {
    aim_dir(ppos, aim).unwrap_or_else(|| {
        if pface.len_sq() > 1e-4 {
            pface.normalized()
        } else {
            Vec2::new(0.0, -1.0)
        }
    })
}

/// Does the circle at `center` (radius `r`) lie inside the cone that starts at
/// `origin`, points along `dir`, is `len` px long and is `half` radians wide?
///
/// The body's own radius is slack at both ends: a body whose edge pokes into the
/// wedge is inside it (that is what makes a cone feel like a swept volume
/// rather than a laser pointer). The angular slack is `asin(r / distance)`, the
/// angle the body's silhouette subtends — small at range, generous point-blank.
pub fn cone_hits_circle(
    origin: Vec2,
    dir: Vec2,
    len: f32,
    half: f32,
    center: Vec2,
    r: f32,
) -> bool {
    let rel = center - origin;
    let dist = rel.len();
    // Behind the muzzle (a body the rat is standing on is not "behind").
    // A half-angle of PI means "any direction" (Gnaw's auto-aim).
    if half < std::f32::consts::PI && rel.dot(dir) + r < 0.0 {
        return false;
    }
    if dist - r > len {
        return false;
    }
    if dist <= r + 0.5 {
        return true;
    }
    let angle = rel.normalized().dot(dir).clamp(-1.0, 1.0).dacos();
    let slack = (r / dist).clamp(0.0, 1.0).dasin();
    angle <= half + slack
}

/// The nearest live body inside a cone with a clear line to the rat.
///
/// `SpawnId` order decides ties (the lowest id wins), exactly like
/// [`nearest_visible`], and the walk stops at the first body because the pick is
/// "nearest", not "first". Used by the two batch-12 melee weapons: Gnaw's bite
/// and the Tail Whip's crack.
fn nearest_in_cone(
    level: &Level,
    reach: f32,
    half: f32,
    dir: Vec2,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    from: Vec2,
) -> Option<(Entity, Vec2)> {
    let mut best: Option<(f32, Entity, Vec2)> = None;
    for &entity in order {
        let Ok((_, _, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        let d = e.pos.dist_sq(from);
        if best.is_some_and(|(bd, _, _)| d >= bd) {
            continue;
        }
        if !cone_hits_circle(from, dir, reach, half, e.pos, e.radius) {
            continue;
        }
        if !los_clear(level, from, e.pos, MELEE_LOS_RADIUS) {
            continue;
        }
        best = Some((d, entity, e.pos));
    }
    best.map(|(_, entity, pos)| (entity, pos))
}

/// The same pick as [`nearest_visible`], with the reach supplied by the caller
/// so a passive (Lens Cap's ranged reach) can stretch it without every caller
/// knowing about stats.
#[allow(clippy::too_many_arguments)]
fn nearest_visible_within(
    level: &Level,
    kind: WeaponKind,
    range: f32,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    from: Vec2,
    scratch: &mut Vec<Candidate>,
) -> Option<Vec2> {
    let range_sq = range * range;
    let radius = target_los_radius(kind);
    scratch.clear();
    for &entity in order {
        let Ok((_, id, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        let dist_sq = e.pos.dist_sq(from);
        if dist_sq.is_nan() || dist_sq > range_sq {
            continue;
        }
        push_candidate(
            scratch,
            Candidate {
                dist_sq,
                rank: RANK_ENEMY,
                id: id.0,
                entity,
                pos: e.pos,
            },
        );
    }
    for (index, point) in boss_points.iter().enumerate() {
        let dist_sq = point.dist_sq(from);
        if dist_sq.is_nan() || dist_sq > range_sq {
            continue;
        }
        push_candidate(
            scratch,
            Candidate {
                dist_sq,
                rank: RANK_BOSS,
                id: index as u64,
                entity: Entity::PLACEHOLDER,
                pos: point,
            },
        );
    }
    for c in scratch.iter() {
        if los_clear(level, from, c.pos, radius) {
            return Some(c.pos);
        }
    }
    None
}

/// Radius a line-of-sight query uses for `kind`, i.e. the projectile's own
/// radius, so a cleared line means the shot genuinely fits.
fn target_los_radius(kind: WeaponKind) -> f32 {
    match kind {
        // `fire_one` sets 3.0 on the lobbed shot.
        WeaponKind::AcidSpit => 3.0,
        // `Bullet::new`'s default, kept by Gnaw and Cheese Spray.
        _ => 2.0,
    }
}

/// Which body a targeting candidate belongs to: regular enemies first, then
/// the boss's grounded target points. Part of the ordering key, so an exact
/// distance tie always resolves to the enemy (the higher-priority body).
const RANK_ENEMY: u8 = 0;
const RANK_BOSS: u8 = 1;

/// One bounded-targeting candidate (distance, then [`RANK_ENEMY`] before
/// [`RANK_BOSS`], then `SpawnId` / point index is the deterministic
/// tie-break).
///
/// Public only because the system's `Local` scratch parameter names it; no
/// other module is meant to build one.
#[derive(Clone, Copy)]
pub struct Candidate {
    dist_sq: f32,
    rank: u8,
    /// `SpawnId` for an enemy, the point's index for a boss point.
    id: u64,
    entity: Entity,
    pos: Vec2,
}

/// The per-tick reusable scratch the firing loop borrows.
///
/// Bundled into one [`bevy_ecs::system::SystemParam`] because bevy caps a
/// system at 16 parameters and `fire_weapons_system` is at the limit (the
/// damage ledger, added for the batch harness's per-weapon report, is the 17th
/// slot without this). Fields are plain `Local`s, so the per-tick behaviour is
/// unchanged: each buffer is reused across ticks.
#[derive(bevy_ecs::system::SystemParam)]
pub struct FireScratch<'w, 's> {
    fallback_order: Local<'s, Vec<Entity>>,
    orbit_scratch: Local<'s, Vec<Entity>>,
    target_scratch: Local<'s, Vec<Candidate>>,
    /// Hantavirus's live **droppings**: the weapon's shot is to leave one
    /// (`weapons_wave3a::fire_hantavirus`), and the level's cap evicts the
    /// oldest (`SpawnId`) first.
    droppings: Query<'w, 's, (Entity, &'static crate::game::ecs::components::SpawnId, &'static crate::game::ecs::components::Dropping)>,
    /// Per-run evolution bookkeeping (**Fairy Ring**'s "every second cast"
    /// counter). A resource, not a `Local`: the schedule's copy of this system
    /// and any `run_system_cached` copy are different instances with their own
    /// locals, so a `Local` counter made the cast sequence depend on which path
    /// fired it — and survived a world reset. `World::new_level*` inserts it, so
    /// it starts at zero with the run.
    evo: ResMut<'w, crate::game::ecs::resources::EvoRuntime>,
    /// The enemy spatial grid, re-filed here when a weapon *moved* a body.
    ///
    /// `enemy_move::grid_refresh_system` files every enemy under the cell it
    /// stood in at the end of the movement phase. Two weapons then write
    /// `Enemy::pos` themselves: Counterweight's melee nudge (the whip) and
    /// Ratchet Tail's quarter-arc swing. `EnemyIndex` stays valid (nothing
    /// spawned or despawned), so the debug guard passes, but the grid still
    /// lists the body in its *old* cell: a bullet that sweeps it later this
    /// tick asks the grid at the new position, gets the old bucket and misses
    /// a body it is touching. Rebuilding the buckets from the (still valid)
    /// index takes one grid fill, so it only happens on a tick that actually
    /// moved something (`moved`).
    grid: ResMut<'w, crate::game::collision::SpatialGrid>,
    moved: Local<'s, bool>,
    /// Console tunable: `set damage_mult N` (read-only here, default 1.0 leaves
    /// the numbers alone). Bundled into this scratch because the system is at
    /// bevy's 16-parameter cap and the console's extra slot would overflow it.
    cvars: Res<'w, Cvars>,
    /// The two melee snares' pictures (`BiteSnap`, `WhipCrack`): aged and
    /// despawned here because this pass runs every tick and the schedule has no
    /// spare slot for two 0.1 s decorations.
    bites: Query<'w, 's, (Entity, &'static mut BiteSnap)>,
    cracks: Query<'w, 's, (Entity, &'static mut WhipCrack)>,
    /// The concentrated weapons' live effects. **Squeak Nova**'s expanding
    /// ring is stepped here (it is a weapon effect, not an active weapon) and
    /// `fire_one` writes it on the cast; the Tail Thrash and the Sonic Squeak
    /// cone are `active_weapons_system`'s business and never touched here.
    active: ResMut<'w, ActiveFx>,
    /// The unzoomed simulation view: the Spiked Tail only hooks what is on it
    /// (see `weapon_spiketail::LASH_VIEW_INSET`). Bundled here for the same
    /// 16-parameter cap as `cvars`.
    viewport: Res<'w, Viewport>,
}

/// The active-weapons pass's two reused buffers.
///
/// Bundled into one [`bevy_ecs::system::SystemParam`] because bevy caps a
/// system at 16 parameters: the `SpawnId`-ordered enemy list (the narrow-harness
/// fallback order) and the `SpawnId`-ordered hostile-pellet list Flywheel's
/// catch walks in (an evolution that records state per pellet, so the order it
/// visits them in is part of the spec).
#[derive(bevy_ecs::system::SystemParam)]
pub struct ActiveScratch<'s> {
    fallback: Local<'s, Vec<Entity>>,
    pellets: Local<'s, Vec<(u64, Entity)>>,
}

/// Nearest-first ordering key: smaller distance, then the regular enemy before
/// any boss point, then smaller `SpawnId` / boss point index (the deterministic
/// form of legacy's "first minimum in `Vec` order").
#[inline]
fn candidate_precedes(
    a_dist: f32,
    a_rank: u8,
    a_id: u64,
    b_dist: f32,
    b_rank: u8,
    b_id: u64,
) -> bool {
    match a_dist.total_cmp(&b_dist) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => match a_rank.cmp(&b_rank) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => a_id < b_id,
        },
    }
}

/// Insert one candidate into the nearest-first scratch, honouring the
/// [`TARGET_LOS_CANDIDATES`] cap (a body beyond the cap is simply not a
/// candidate this shot).
#[inline]
fn push_candidate(scratch: &mut Vec<Candidate>, c: Candidate) {
    let at = scratch.partition_point(|o| {
        candidate_precedes(o.dist_sq, o.rank, o.id, c.dist_sq, c.rank, c.id)
    });
    if at >= TARGET_LOS_CANDIDATES {
        return;
    }
    scratch.insert(at, c);
    if scratch.len() > TARGET_LOS_CANDIDATES {
        scratch.pop();
    }
}

/// Nearest target of `kind`'s reach that has an unobstructed line from `from`.
///
/// This replaces the legacy plain nearest pick for the aiming weapons. It walks
/// the tick's stable `SpawnId` order **once**, keeps only the
/// [`TARGET_LOS_CANDIDATES`] nearest in-range bodies in a reused scratch (so the
/// number of tile queries per shot is bounded no matter how big the horde is),
/// then asks [`los_clear`] nearest-first and returns the first visible one.
///
/// The boss's grounded [`BossTargetable`] points are the same kind of candidate
/// as an enemy (same reach, same line test, same cap, nearest wins), so an
/// intervening horde screens the boss exactly as it screens the rat. A boss
/// with no grounded point (the cat mid-pounce, the broom's lifted slipper) is
/// not targetable at all. Ties go to the enemy first and then to the lowest
/// boss point index, which keeps the pick independent of query order.
/// `None` means "nothing the rat can shoot", which is what the caller's
/// no-target path handles.
fn nearest_visible(
    level: &Level,
    kind: WeaponKind,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    from: Vec2,
    scratch: &mut Vec<Candidate>,
) -> Option<Vec2> {
    // A NaN position is never a real target: `nearest_visible_within` rejects
    // it there instead of letting it sort last and win the range check.
    nearest_visible_within(
        level,
        kind,
        target_range(kind),
        order,
        enemies,
        boss_points,
        from,
        scratch,
    )
}

// ---- Active (player-triggered) weapons ------------------------------------

/// Player-triggered weapons: charge, the Space release, and the two live
/// effects (the thrash burst and the squeak ring).
///
/// Runs immediately after [`fire_weapons_system`] (schedule point 11b), where
/// the cached [`EnemyIndex`] is guaranteed to be the live `SpawnId` order and
/// nothing has been reaped yet, so the sweeps damage and the buffer rules are
/// the same as every other weapon's. It owns the whole behaviour of the two
/// active weapons: `fire_weapons_system` skips them entirely.
///
/// Two halves, in order:
///
/// 1. **Charge and release.** Each active weapon's `Weapon::timer` is its charge
///    meter; it counts down here (level and the Frenzy passive shorten it
///    through `Weapon::cooldown`, exactly like every other weapon).
///    `TickInput::active` is the Space/click press: every weapon whose meter has
///    reached zero is spent. A press with nothing charged does nothing, and an
///    active still fires when the dash is on cooldown (the dash is the
///    `player_system`'s business, not this system's).
/// 2. **Live effects.** A Tail Thrash burst sweeps three full discs over
///    `THRASH_BURST` seconds: each sweep hurts every enemy in the disc, deletes
///    enemy bullets inside it, knocks survivors outward and re-damages the boss.
///    A Sonic Squeak press resolves **once, on the press tick**, against the
///    forward cone (38° either side of the aim, out to the weapon's reach): every
///    body inside it takes the hit, is stunned (elites and heavies slowed) and is
///    shoved outward, and the drawn chevrons that follow only animate the push.
///
/// Determinism: the enemy walk uses the same stable order the other weapons use
/// and every per-enemy rng draw happens inside [`vfx`] calls that already draw
/// in `SpawnId` order, so a replay of the same inputs lands the same hits in the
/// same order. Nothing here draws rng unless a weapon is actually spent.
///
/// The **Squeak Nova**'s ring is stepped by [`nova_ring`] from
/// `fire_weapons_system` instead (it is an ordinary auto-firing weapon, so
/// nothing here owns its clock).
#[allow(clippy::too_many_arguments)]
pub fn active_weapons_system(
    mut commands: Commands,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    enemy_bullets: Query<(Entity, &SpawnId, &Bullet)>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut shake: ResMut<Shake>,
    mut fx: ResMut<ActiveFx>,
    mut scratch: ActiveScratch,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let pface = player.face;
    let fire_rate = player.loadout.stats.fire_rate_mult.max(0.1);
    let dmg_mult = player.damage_mult();
    // Wave 3: copy the passive hooks out before the loadout is borrowed mutably
    // (`Stats` is `Copy`).
    let stats = player.loadout.stats;

    // One stable enemy order for the tick, exactly like `fire_weapons_system`:
    // `EnemyIndex` is rebuilt immediately before that system and nothing has
    // spawned or despawned an enemy since, so it is the live `SpawnId` order.
    // A narrow single-system test that never ran the grid is detected by the
    // length check and gets a sorted reused scratch instead (never once per
    // sweep).
    let enemy_count = enemies.iter().len();
    // The two buffers are disjoint fields, so the enemy order can stay
    // borrowed while the pellet scratch is handed to the sweep.
    let fallback: &mut Vec<Entity> = &mut scratch.fallback;
    let pellets: &mut Vec<(u64, Entity)> = &mut scratch.pellets;
    let order: &[Entity] = if index.0.len() == enemy_count {
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
    };

    // 1a. Charge every active weapon.
    for w in player.loadout.weapons.iter_mut() {
        if w.kind.is_active() && w.timer > 0.0 {
            w.timer = (w.timer - dt).max(0.0);
        }
    }

    // 1b. The Space press spends every fully charged active weapon.
    // The Sonic Squeak's cone is aimed like every other whole-cast weapon: the
    // reticle when the player aims, else the rat's facing.
    let cone_dir = cast_dir(ppos, pface, input.aim);
    if input.active {
        for i in 0..player.loadout.weapons.len() {
            let (kind, charged) = {
                let w = &player.loadout.weapons[i];
                (w.kind, w.kind.is_active() && w.timer <= 0.0)
            };
            if !charged {
                continue;
            }
            let (charge, dmg, reach, evolved) = {
                let w = &player.loadout.weapons[i];
                (
                    (w.cooldown() / fire_rate).max(0.05),
                    w.damage() * dmg_mult,
                    // Wave 3 passive hooks: the thrash is a melee arc (Lens
                    // Cap's melee reach), the squeak is a radial ring (Long
                    // Fuse's splash radius **and** Lens Cap's ring reach: the
                    // squeak is a ring, and leaving that hook out made the
                    // passive silently do nothing for this weapon).
                    match w.kind {
                        WeaponKind::SonicSqueak => {
                            stats.melee_reach(stats.splash_radius(w.active_reach()))
                        }
                        _ => stats.melee_reach(w.active_reach()),
                    },
                    w.evolved,
                )
            };
            player.loadout.weapons[i].timer = charge;
            match kind {
                WeaponKind::TailThrash => {
                    *fx = ActiveFx {
                        thrash_left: THRASH_BURST,
                        thrash_total: THRASH_BURST,
                        thrash_age: 0.0,
                        // Start the arc behind the rat (the tail's home) and
                        // sweep forward from there.
                        thrash_angle: pface.angle() + std::f32::consts::PI,
                        thrash_reach: reach,
                        thrash_dmg: dmg,
                        thrash_done: 0,
                        ..*fx
                    };
                    fx.thrash_evolved = evolved;
                    // A fresh burst starts with an empty catch (Flywheel).
                    fx.clear_caught();
                    shake.amount = shake.amount.max(THRASH_SHAKE);
                    vfx::emit_thrash_start(&mut particles, &mut rng.0, ppos);
                    sfx.0.push(Sfx::ShootThrash);
                }
                WeaponKind::SonicSqueak => {
                    // Batch 12: a **cone**, not a ring. The damage, the stun,
                    // the shove and Roll Call's tags all land on the press
                    // tick, inside the wedge; the fields below only drive the
                    // drawn nested chevrons pushing outward.
                    *fx = ActiveFx {
                        squeak_left: SQUEAK_EXPAND,
                        squeak_prev_r: 0.0,
                        squeak_r: 0.0,
                        squeak_max_r: reach,
                        squeak_dmg: dmg,
                        squeak_pos: ppos,
                        squeak_half: SQUEAK_CONE_HALF,
                        squeak_facing: cone_dir,
                        ..*fx
                    };
                    fx.squeak_evolved = evolved;
                    // A fresh cone starts with no tags (Roll Call).
                    fx.clear_roll_call();
                    shake.amount = shake.amount.max(1.6);
                    vfx::emit_sonic_squeak_cone(
                        &mut particles,
                        &mut rng.0,
                        ppos,
                        cone_dir,
                        reach,
                        SQUEAK_CONE_HALF,
                        SQUEAK_EXPAND,
                    );
                    sfx.0.push(Sfx::SonicSqueak);
                    squeak_cone(
                        &level,
                        &mut rng,
                        &mut particles,
                        order,
                        &mut enemies,
                        &mut bosses,
                        &mut damage_numbers,
                        &mut ledger,
                        &mut fx,
                        stats.shove_mult,
                    );
                }
                _ => {}
            }
        }
    }

    // 2a. The thrash burst: sweep the arc and resolve one sweep per tick at
    // most. At 60 fps the sweep interval is 1/3 s, so a tick never owes two.
    if fx.thrash_left > 0.0 {
        let total = fx.thrash_total.max(1e-4);
        fx.thrash_age += dt;
        fx.thrash_angle = (fx.thrash_angle + THRASH_SWEEP_RATE * dt) % std::f32::consts::TAU;
        let interval = total / THRASH_SWEEPS as f32;
        let due = ((fx.thrash_age / interval) as u32 + 1).min(THRASH_SWEEPS);
        if due > fx.thrash_done {
            fx.thrash_done = due;
            thrash_sweep(
                &mut commands,
                ppos,
                pface,
                &level,
                &mut rng,
                &mut particles,
                order,
                &mut enemies,
                &mut bosses,
                &enemy_bullets,
                &mut damage_numbers,
                &mut ledger,
                &mut fx,
                stats.shove_mult,
                pellets,
            );
        }
        fx.thrash_left = (fx.thrash_left - dt).max(0.0);
        if fx.thrash_left <= 0.0 {
            fx.thrash_total = 0.0;
            fx.thrash_age = 0.0;
            fx.thrash_done = 0;
        }
    }

    // 2b. The squeak cone's push. The hit already landed on the press tick, so
    // this only advances the drawn chevron front (the art fades the wedge out
    // with `squeak_left`).
    if fx.squeak_left > 0.0 {
        fx.squeak_left = (fx.squeak_left - dt).max(0.0);
        let prev = fx.squeak_r;
        let step = fx.squeak_max_r / SQUEAK_EXPAND * dt.min(SQUEAK_EXPAND);
        let now = (prev + step).min(fx.squeak_max_r);
        fx.squeak_prev_r = prev;
        fx.squeak_r = now;
        if fx.squeak_left <= 0.0 {
            fx.squeak_r = 0.0;
            fx.squeak_prev_r = 0.0;
        }
    }
}

/// Resolve one thrash sweep: damage, knockback, bullet deletion, boss damage.
#[allow(clippy::too_many_arguments)]
fn thrash_sweep(
    commands: &mut Commands,
    ppos: Vec2,
    pface: Vec2,
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    enemy_bullets: &Query<(Entity, &SpawnId, &Bullet)>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    fx: &mut ActiveFx,
    // Counterweight: a weapon's native shove (see `Stats::shove`).
    shove_mult: f32,
    // `(SpawnId, Entity)` of this tick's hostile pellets, reused every sweep.
    pellet_order: &mut Vec<(u64, Entity)>,
) {
    let kind = WeaponKind::TailThrash;
    let reach = fx.thrash_reach;
    let dmg = fx.thrash_dmg;
    let evolved = fx.thrash_evolved;
    let final_sweep = fx.thrash_done >= THRASH_SWEEPS;
    let mut hits = 0u32;

    for &entity in order {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || e.pos.dist(ppos) > reach + e.radius {
            continue;
        }
        // The arc is a melee sweep: it does not reach through a maze wall.
        if !los_clear(level, ppos, e.pos, MELEE_LOS_RADIUS) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        // Directional armour (the Pillbug's shell): the sweep arrives from the
        // rat, so the recorded number is what actually landed.
        let applied = e.hurt_from(dmg, ppos);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let out = (pos - ppos).normalized();
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        // Brutes, the boss and the wave-3 Pillbug keep their footing (a
        // curled shell is not something a tail sweep shoves around).
        let heavy = e.kind.heavy();
        let knock = if heavy { THRASH_BOSS_KNOCKBACK } else { 1.0 };
        e.vel = out * THRASH_KNOCKBACK * knock * shove_mult;
        vfx::emit_thrash_hit(particles, &mut rng.0, pos, out, killed);
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::TailWhip,
        );
        hits += 1;
    }

    // The arc also thrashes aside every enemy shot inside its reach.
    //
    // The catch is the one part of the sweep with **rng and entity state**
    // (Flywheel records which pellets it caught), so the walk must not follow
    // query storage order: two layouts holding the same `SpawnId`s at the same
    // positions would otherwise catch different pellets and draw their sparks
    // in a different order. Sorted into the reused scratch, as everywhere else.
    pellet_order.clear();
    pellet_order.extend(
        enemy_bullets
            .iter()
            .filter(|(_, _, b)| b.faction == Faction::Enemy)
            .map(|(entity, id, _)| (id.0, entity)),
    );
    pellet_order.sort_unstable_by_key(|&(id, _)| id);
    let mut caught_this_sweep = 0u8;
    for &(_, entity) in pellet_order.iter() {
        let Ok((_, _, b)) = enemy_bullets.get(entity) else {
            continue;
        };
        if b.faction != Faction::Enemy {
            continue;
        }
        if b.pos.dist(ppos) <= reach + b.radius {
            if evolved && caught_this_sweep < evolutions::FLYWHEEL_PER_SWEEP
                && fx.catch_pellet(b.vel) {
                    caught_this_sweep += 1;
                    vfx::emit_flywheel_catch(particles, &mut rng.0, b.pos, caught_this_sweep as usize);
                }
            commands.entity(entity).despawn();
        }
    }
    // Flywheel: the final sweep flings what the burst caught back out.
    if evolved && final_sweep {
        // Queue the fling: the command spawns the pellets with a `SpawnId` when
        // it applies, so this system keeps its parameter budget.
        let count = fx.caught;
        if count > 0 {
            vfx::emit_flywheel_fling(particles, &mut rng.0, ppos, pface, count as usize);
            commands.queue(evolutions::FlingPellets {
                pos: ppos,
                forward: pface,
                damage: dmg,
                count,
            });
            fx.clear_caught();
        }
    }

    // A heavy body takes the damage with little knockback (it has no velocity
    // to speak of; `boss.slow_for` is the squeak's business, not the thrash's).
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        let shapes = boss.hit_shapes();
        if shapes.iter().next().is_none() {
            continue;
        }
        if !shapes.circle_hits_grounded(ppos, reach)
            || level.tiles.segment_hit(ppos, boss.pos, 1.0).is_some()
        {
            continue;
        }
        let amount = dmg * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        // A sweep shoves a boss the same few pixels a squeak does: capped by
        // `Boss::shove` (4 px a hit, 12 px a second) and never inside a
        // committed beat.
        let away = boss.pos - ppos;
        let _ = boss.shove(
            level,
            away,
            crate::game::ecs::systems::boss::SHOVE_PER_HIT_PX,
        );
        ledger.add_boss(kind, amount);
        let killed = boss.hp <= 0.0;
        vfx::emit_thrash_hit(particles, &mut rng.0, boss.pos, boss.pos - ppos, killed);
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            killed,
            DamageSource::TailWhip,
        );
        hits += 1;
    }

    // One arc flare + a spray of sparks along the drawn sweep, per sweep.
    vfx::emit_thrash_sweep(
        particles,
        &mut rng.0,
        ppos,
        fx.thrash_angle,
        reach,
        hits > 0,
    );
}

/// Resolve a **Sonic Squeak** press: everything inside the forward cone.
///
/// Batch 12 made the squeak directional. It used to be a ring: everything whose
/// near edge the expanding front crossed, all the way around the rat. It is now
/// one wedge, `SQUEAK_CONE_HALF` either side of the aim, out to the weapon's
/// reach — the shape the tail thrash is not (a disc) and the nova is not (a
/// ring). Everything inside it takes the hit once per press, is stunned (elites
/// and heavies slowed), shoved outward along the cone's axis, and tagged for
/// **Roll Call**.
///
/// Walls stop it: a body behind masonry takes nothing — no damage, no shove, no
/// stun, no tag — exactly as before.
#[allow(clippy::too_many_arguments)]
fn squeak_cone(
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    fx: &mut ActiveFx,
    // Counterweight: the cone's native outward shove.
    shove_mult: f32,
) {
    let kind = WeaponKind::SonicSqueak;
    let origin = fx.squeak_pos;
    let axis = fx.squeak_facing;
    let reach = fx.squeak_max_r;
    let half = fx.squeak_half;
    let dmg = fx.squeak_dmg;

    for &entity in order {
        let Ok((_, sid, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        if !cone_hits_circle(origin, axis, reach, half, e.pos, e.radius) {
            continue;
        }
        if !los_clear(level, origin, e.pos, MELEE_LOS_RADIUS) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        // The cone arrives from its own apex, so the shell faces it.
        let applied = e.hurt_from(dmg, origin);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let out = (pos - origin).normalized();
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        // Elites keep their footing (slowed, not stunned); everything else is
        // pinned for the full stun.
        if e.elite || e.kind.heavy() {
            e.slow_for(SQUEAK_ELITE_SLOW, SQUEAK_STUN);
        } else {
            e.stun_for(SQUEAK_STUN);
        }
        // Roll Call: the cone tags everything it stunned or slowed; the answers
        // fire 0.8 s later (`evolutions::roll_call_system`).
        if fx.squeak_evolved && fx.tag(entity, sid.0) {
            if fx.answer_left <= 0.0 {
                fx.answer_left = evolutions::ROLL_CALL_DELAY;
                fx.answer_damage = dmg * 0.5;
            }
            vfx::emit_roll_call_tag(particles, &mut rng.0, pos);
        }
        e.vel += out * SQUEAK_SHOVE * shove_mult;
        vfx::emit_squeak_hit(particles, &mut rng.0, pos, out, killed);
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::Normal,
        );
    }

    // The boss is too big to stun: it takes the hit and is slowed briefly.
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        let shapes = boss.hit_shapes();
        // An airborne boss (no grounded shape) is out of a floor-level
        // attack's reach, exactly as it is for bullets.
        if !shapes.iter().any(|s| s.grounded()) {
            continue;
        }
        if !cone_hits_circle(origin, axis, reach, half, boss.pos, boss.kind.body_radius()) {
            continue;
        }
        if level.tiles.segment_hit(origin, boss.pos, 1.0).is_some() {
            continue;
        }
        let amount = dmg * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        boss.slow_for(SQUEAK_BOSS_SLOW, SQUEAK_BOSS_SLOW_SECS);
        // The cone's native outward shove, on a boss: `Boss::shove` caps it at
        // 4 px a hit and 12 px a second and refuses it outright inside a
        // committed beat (batch 13 — a shove mid-wind-up would slide the pose
        // out from under its own tell).
        let away = boss.pos - origin;
        let _ = boss.shove(
            level,
            away,
            crate::game::ecs::systems::boss::SHOVE_PER_HIT_PX,
        );
        ledger.add_boss(kind, amount);
        let killed = boss.hp <= 0.0;
        let (boss_pos, boss_radius, boss_id) = (boss.pos, boss.kind.body_radius(), boss.id);
        vfx::emit_squeak_hit(particles, &mut rng.0, boss_pos, boss_pos - origin, killed);
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

/// Step **Squeak Nova**'s ring: expand the front and damage everything it
/// passes, once per pass.
///
/// Batch 12 replaced the nova's ring of radial darts with one shockwave. The
/// front's radius is a pure function of its age (`NOVA_EXPAND` out; with
/// **Echo Chamber** the same again back in), and the damage band is
/// `prev..now`, so a body can only be hit once on the way out and once on the
/// way back however the tick rate divides the pass. Walls stop it: a body
/// behind masonry takes nothing, exactly like the spoke could not get there.
/// Enemy walks are `SpawnId` ordered (`EnemyIndex`) and every rng draw happens
/// inside a `vfx` call, so a replay lands the same hits in the same order.
#[allow(clippy::too_many_arguments)]
fn nova_ring(
    dt: f32,
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    fx: &mut ActiveFx,
) {
    let kind = WeaponKind::SqueakNova;
    let origin = fx.nova_pos;
    // Last tick's front is this tick's inner (outbound) or outer (inbound)
    // band edge, so each body is met exactly once per pass.
    let prev = fx.nova_r;
    let dmg = fx.nova_dmg;
    let max_r = fx.nova_max_r;
    let prev_phase = fx.nova_age / NOVA_EXPAND;
    fx.nova_age += dt;
    // Out for `NOVA_EXPAND`, then (Echo Chamber only) back in for the same
    // again. The tick that crosses the turnaround finishes the outbound pass
    // and starts the return from the rim.
    let phase = fx.nova_age / NOVA_EXPAND;
    let outbound = prev_phase < 1.0;
    let inbound = fx.nova_echo && phase > 1.0;
    let out_to = max_r * phase.min(1.0);
    let in_from = if outbound { max_r } else { prev };
    let now = if inbound {
        max_r * (2.0 - phase).clamp(0.0, 1.0)
    } else {
        out_to
    };
    fx.nova_prev_r = prev;
    fx.nova_r = now;

    for &entity in order {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        let dist = e.pos.dist(origin);
        // The front meets the body's near edge on the way out and its far edge
        // on the way back, so a ring that sweeps a body never skips it. A body
        // on top of the rat is met by the first tick; one straddling the rim
        // by the last.
        let near = (dist - e.radius).max(1e-3);
        let far = (dist + e.radius).min(max_r - 1e-3);
        let out_hit = outbound && prev < near && near <= out_to;
        let in_hit = inbound && now <= far && far < in_from;
        if !(out_hit || in_hit) || !los_clear(level, origin, e.pos, MELEE_LOS_RADIUS) {
            continue;
        }
        for returning in [false, true] {
            if !(if returning { in_hit } else { out_hit }) || !e.alive() {
                continue;
            }
            let hp_before = e.hp.max(0.0);
            // The ring arrives from its own origin, so the shell faces it.
            let applied = e.hurt_from(dmg, origin);
            let killed = !e.alive();
            ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
            let pos = e.pos;
            let enemy_radius = e.radius;
            let enemy_id = e.id;
            let enemy_kind = e.kind;
            if returning {
                // Echo Chamber: the returning front, drawn as the rebound
                // (`vfx::emit_echo_return`'s "inward" direction).
                vfx::emit_echo_return(
                    particles,
                    &mut rng.0,
                    pos,
                    (origin - pos).normalized(),
                );
            } else {
                vfx::emit_weapon_hit(
                    particles,
                    &mut rng.0,
                    pos,
                    pos - origin,
                    Some(kind),
                    enemy_kind,
                    killed,
                );
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
    }

    // The boss takes the ring too, once per pass, exactly like the horde.
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        let shapes = boss.hit_shapes();
        // An airborne boss (no grounded shape) is out of a floor-level
        // attack's reach, exactly as it is for bullets.
        if !shapes.iter().any(|s| s.grounded()) {
            continue;
        }
        let dist = boss.pos.dist(origin);
        let near = (dist - boss.kind.body_radius()).max(1e-3);
        let far = (dist + boss.kind.body_radius()).min(max_r - 1e-3);
        let out_hit = outbound && prev < near && near <= out_to;
        let in_hit = inbound && now <= far && far < in_from;
        if !(out_hit || in_hit) || level.tiles.segment_hit(origin, boss.pos, 1.0).is_some() {
            continue;
        }
        for returning in [false, true] {
            if !(if returning { in_hit } else { out_hit }) || boss.dead {
                continue;
            }
            let amount = dmg * boss.damage_mult;
            if !boss.take_damage(amount) {
                continue;
            }
            ledger.add_boss(kind, amount);
            let killed = boss.hp <= 0.0;
            let (boss_pos, boss_radius, boss_id) = (boss.pos, boss.kind.body_radius(), boss.id);
            vfx::emit_weapon_hit(
                particles,
                &mut rng.0,
                boss_pos,
                boss_pos - origin,
                Some(kind),
                EnemyKind::Boss,
                killed,
            );
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

    // The ring's own clock. Once it is spent the whole cast is forgotten (the
    // next cast seats a fresh one).
    let total = fx.nova_total();
    fx.nova_left = (total - fx.nova_age).max(0.0);
    if fx.nova_left <= 0.0 {
        fx.nova_left = 0.0;
        fx.nova_age = 0.0;
        fx.nova_prev_r = 0.0;
        fx.nova_r = 0.0;
    }
}

/// Legacy `World::fire_one`, with the neon-branch VFX/damage-number hooks.
/// Returns whether a shot was produced (Gnaw with no live target fires
/// nothing).
#[allow(clippy::too_many_arguments)]
fn fire_one(
    commands: &mut Commands,
    kind: WeaponKind,
    wlevel: u8,
    evolved: bool,
    dmg: f32,
    cd: f32,
    projs: i32,
    ppos: Vec2,
    pface: Vec2,
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    sfx: &mut SfxQueue,
    next_id: &mut NextSpawnId,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bullets: &Query<(Entity, &SpawnId, &Bullet)>,
    orbit_scratch: &mut Vec<Entity>,
    target_scratch: &mut Vec<Candidate>,
    boss_points: &BossTargetable,
    // The boss's own body: a whip that reaches a boss body must damage it here,
    // with the reach and damage this cast actually used (see the Tail Whip arm).
    bosses: &mut Query<&mut Boss>,
    // Wave 3 passive hooks (Lens Cap's reach, Long Fuse's zone life and splash
    // radius, Counterweight's shove) and the per-body melee-nudge log it needs.
    stats: &crate::game::loadout::Stats,
    melee_nudge: &mut MeleeNudge,
    // Per-instance **Fairy Ring** parity (see `EvoRuntime::count_fairy_cast`).
    evo: &mut crate::game::ecs::resources::EvoRuntime,
    // Set when this shot moved a body: the grid has to be re-filed before the
    // projectile/contact passes read it (see `FireScratch::grid`).
    moved: &mut bool,
    // The reticle in world pixels (`None` = auto-aim). The per-weapon table is
    // in the module docs on `TickInput::aim`; each arm below applies it.
    aim: Option<Vec2>,
    // The concentrated weapons' live effects: this pass writes the **Squeak
    // Nova** ring (the ring is stepped after the loadout loop).
    fx: &mut crate::game::ecs::resources::ActiveFx,
    // The tick's [`Viewport`]: the unzoomed screen rectangle the Spiked Tail's
    // target pick is capped to (`weapon_spiketail::LASH_VIEW_INSET`).
    view: Vec2,
    // Hantavirus's live droppings (the cap evicts the oldest).
    droppings: &Query<(Entity, &SpawnId, &crate::game::ecs::components::Dropping)>,
) -> bool {
    let color = kind.color();
    let mut fired = false;
    match kind {
        WeaponKind::Gnaw => {
            // Batch 12: Gnaw is a **bite** — a snapping jaw in front of the
            // rat's nose, no projectile. It keeps the dart's single-target
            // identity (it takes the nearest visible body inside a narrow
            // forward cone) and the whole volley's output: from level 4 the old
            // two darts' worth lands as one bite, so a body takes per cycle
            // what the darts dealt it.
            let i = (wlevel.clamp(1, 5) as usize) - 1;
            let reach = stats.melee_reach(GNAW_BITE_REACH[i]);
            // Aimed (reticle or pad stick): the jaw snaps inside its cone of
            // that direction. Auto-aim (Classic, no reticle): the rat turns its
            // head, so the bite takes the nearest body in reach whatever way it
            // is standing — the same "nearest visible body" rule every other
            // auto-firing weapon uses.
            let aimed = aim_dir(ppos, aim);
            let aim_u = aimed.unwrap_or_else(|| cast_dir(ppos, pface, aim));
            let half = if aimed.is_some() {
                GNAW_BITE_HALF
            } else {
                std::f32::consts::PI
            };
            let shots = 1 + projs / 4;
            let bite = dmg * shots as f32;
            let pick = nearest_in_cone(level, reach, half, aim_u, order, enemies, ppos);
            // The jaw closes on the body it caught (so the art and the drawn
            // line point at it), else straight ahead.
            let dir = pick
                .map(|(_, pos)| (pos - ppos).normalized())
                .unwrap_or(aim_u);
            let mut bit = false;
            if let Some((entity, pos)) = pick {
                if let Ok((_, _, mut e)) = enemies.get_mut(entity) {
                    let hp_before = e.hp.max(0.0);
                    // Directional armour: a bite arrives from the rat.
                    let applied = e.hurt_from(bite, ppos);
                    let killed = !e.alive();
                    ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
                    let enemy_radius = e.radius;
                    let enemy_id = e.id;
                    let enemy_kind = e.kind;
                    vfx::emit_weapon_hit(
                        particles,
                        &mut rng.0,
                        pos,
                        pos - ppos,
                        Some(kind),
                        enemy_kind,
                        killed,
                    );
                    damage_numbers.record(
                        enemy_id,
                        pos,
                        enemy_radius,
                        applied,
                        killed,
                        DamageSource::Normal,
                    );
                    bit = true;
                }
            }
            // The boss is a body too: the jaw closes on it when the rat is
            // pressed against it, with the bite's own damage this cast.
            if !bit {
                for mut boss in bosses.iter_mut() {
                    if boss.dead {
                        continue;
                    }
                    let shapes = boss.hit_shapes();
                    // An airborne boss (no grounded shape) is out of a floor-level
                    // attack's reach, exactly as it is for bullets.
                    if !shapes.iter().any(|s| s.grounded()) {
                        continue;
                    }
                    if !cone_hits_circle(
                        ppos,
                        dir,
                        reach,
                        half,
                        boss.pos,
                        boss.kind.body_radius(),
                    ) || level.tiles.segment_hit(ppos, boss.pos, 1.0).is_some()
                    {
                        continue;
                    }
                    let amount = bite * boss.damage_mult;
                    if !boss.take_damage(amount) {
                        continue;
                    }
                    ledger.add_boss(kind, amount);
                    let killed = boss.hp <= 0.0;
                    let (boss_pos, boss_radius, boss_id) =
                        (boss.pos, boss.kind.body_radius(), boss.id);
                    vfx::emit_weapon_hit(
                        particles,
                        &mut rng.0,
                        boss_pos,
                        boss_pos - ppos,
                        Some(kind),
                        EnemyKind::Boss,
                        killed,
                    );
                    damage_numbers.record(
                        boss_id,
                        boss_pos,
                        boss_radius,
                        amount,
                        killed,
                        DamageSource::Normal,
                    );
                    bit = true;
                    break;
                }
            }
            if bit {
                // A decoration, not a body: it carries no `SpawnId`, so the
                // spawn-order sequence every other system orders by is not
                // perturbed by a jaw snap.
                commands.spawn(BiteSnap {
                    pos: ppos + dir * 4.0,
                    dir,
                    t: BITE_SECS,
                    total: BITE_SECS,
                });
                // Milk Teeth (`WAVE3_DESIGN` §3) evolves the old lead dart: the
                // evolved bite also spits the tooth, which lodges for 1 s and
                // comes home biting every body it crosses
                // (`evolutions::milk_teeth_system`). The base weapon stays a
                // bite; the evolution adds its own projectile on top, so an
                // earned evolution is never a downgrade.
                if evolved {
                    commands.spawn((
                        MilkTooth::new(
                            ppos,
                            dir * evolutions::MILK_SPEED,
                            dmg,
                            stats.ranged_reach(evolutions::MILK_REACH),
                        ),
                        next_id.take(),
                    ));
                }
                vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, dir, 0.0);
                fired = true;
            }
        }
        WeaponKind::CheeseSpray => {
            // Aimed: the fan is centred on the reticle. Auto-aim: the fan
            // centres on the nearest visible body, as before.
            let spray_dir = aim_dir(ppos, aim).unwrap_or_else(|| {
                nearest_visible(level, kind, order, enemies, boss_points, ppos, target_scratch)
                    .map(|p| (p - ppos).normalized())
                    .unwrap_or(pface)
            });
            let base = spray_dir.angle();
            // Fondue Fork: the spray's two **outer** darts are paired tine
            // projectiles (they fly, hit and plant like a dart — no duplicated
            // cast-time ray), and the string is created when both have settled.
            // The inner darts stay ordinary darts.
            let spread = 0.18;
            // Batch 12: the fan is a **shotgun of slow, short wedges**, so it
            // also slows what it covers (the wedge that lands on a body still
            // deals its damage on its own tick — this is the fan's slowing
            // field, applied as the fan leaves, on the same open floor the
            // wedges can cross).
            let fan_half = 0.5 * spread * (projs - 1) as f32 + crate::game::weapon::CHEESE_FAN_SLACK;
            // Lens Cap stretches the wedges' flight, so it stretches the fan.
            let fan_reach = CHEESE_WEDGE_SPEED * stats.ranged_reach(CHEESE_WEDGE_LIFE);
            for &entity in order {
                let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                    continue;
                };
                if !e.alive()
                    || !cone_hits_circle(ppos, spray_dir, fan_reach, fan_half, e.pos, e.radius)
                    || !los_clear(level, ppos, e.pos, MELEE_LOS_RADIUS)
                {
                    continue;
                }
                e.slow_for(CHEESE_SLOW_MULT, CHEESE_SLOW_SECS);
            }
            let half = (projs - 1) as f32 / 2.0;
            let tines = evolved && projs >= 2;
            if tines {
                let cast = next_id.take().0;
                commands.spawn((
                    crate::game::ecs::components::FondueCast {
                        ends: [None; 2],
                        damage: dmg,
                        wait: evolutions::TINE_FLIGHT_MAX,
                    },
                    SpawnId(cast),
                ));
                // Lens Cap stretches how far a tine can plant.
                let reach = stats.ranged_reach(evolutions::TINE_REACH);
                for (slot, k) in [(0u8, 0i32), (1, projs - 1)] {
                    let angle = base + (k as f32 - half) * spread;
                    let vel = Vec2::from_angle(angle) * evolutions::TINE_SPEED;
                    commands.spawn((
                        crate::game::ecs::components::FondueTine {
                            pos: ppos,
                            prev: ppos,
                            vel,
                            damage: dmg,
                            reach,
                            slot,
                            cast,
                        },
                        next_id.take(),
                    ));
                }
            }
            for k in 0..projs {
                if tines && (k == 0 || k == projs - 1) {
                    continue;
                }
                let spread = (k as f32 - (projs - 1) as f32 / 2.0) * spread;
                let v = Vec2::from_angle(base + spread) * CHEESE_WEDGE_SPEED;
                let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                // Chunky wedges: a wide body on a short flight (`size 2` is
                // the fat projectile art, the 3 px radius the hit reach).
                b.size = 2;
                b.radius = 3.0;
                // Lens Cap.
                b.life = stats.ranged_reach(CHEESE_WEDGE_LIFE);
                b.weapon = Some(kind);
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, spray_dir, 0.0);
            fired = true;
        }
        WeaponKind::SqueakNova => {
            // Batch 12: a true shockwave — **one expanding ring** that damages
            // every body its front passes, instead of a ring of radial darts.
            // The front is stepped by [`nova_ring`] at the end of
            // `fire_weapons_system` (the cached `EnemyIndex` is live there and
            // the damage lands in the same tick as every other weapon's).
            let reach = stats.ranged_reach(NOVA_REACH);
            // Echo Chamber adds the return leg to the ring's life.
            let passes: f32 = if evolved { 2.0 } else { 1.0 };
            let life = passes * NOVA_EXPAND;
            *fx = ActiveFx {
                nova_left: life,
                nova_age: 0.0,
                nova_prev_r: 0.0,
                nova_r: 0.0,
                nova_max_r: reach,
                nova_dmg: dmg,
                nova_pos: ppos,
                // Echo Chamber: the ring strikes the walls and comes back
                // through for a second hit.
                nova_echo: evolved,
                ..*fx
            };
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, reach);
            fired = true;
        }
        WeaponKind::SporeOrbit => {
            // Lens Cap widens the ring, Long Fuse keeps it alive longer.
            let radius = stats.melee_reach(evolutions::FAIRY_RADIUS);
            // Carry the ring's current angle forward, then drop the old ring
            // before spawning its replacement so two never stack. One pass:
            // remember the lowest-`SpawnId` orbit angle (legacy `find_map`
            // over stable `SpawnId` order) and collect the old ring for
            // deferred despawn without sorting the bullet query.
            let mut carry: Option<(u64, f32, f32)> = None;
            orbit_scratch.clear();
            for (entity, id, b) in bullets.iter() {
                if let Some(o) = b.orbit {
                    orbit_scratch.push(entity);
                    if carry.is_none_or(|(best, _, _)| id.0 < best) {
                        carry = Some((id.0, o.angle, b.life));
                    }
                }
            }
            let base = carry.map(|(_, angle, _)| angle).unwrap_or(0.0);
            for &entity in orbit_scratch.iter() {
                commands.entity(entity).despawn();
            }
            // Long Fuse: an orbiter is a lingering friendly zone, and a recast
            // may **never** cut that life short. The old ring is still dropped
            // (two rings would stack damage), but the replacement inherits any
            // remaining Long Fuse life instead of erasing it — without the
            // passive the leftover is ~0 and the legacy "drop and respawn" is
            // unchanged.
            let zone = stats.zone_life(cd);
            let ring_life = zone.max(carry.map(|(_, _, life)| life).unwrap_or(0.0));
            for k in 0..projs {
                let angle = base + k as f32 / projs as f32 * std::f32::consts::TAU;
                let mut b = Bullet::new(ppos, Vec2::ZERO, dmg, Faction::Player, color);
                b.life = ring_life;
                b.pierce = 999;
                b.size = 2;
                // A 4 px spore body against a 3-7 px enemy is a wider ring
                // band: the measured weapon was the weakest pick by a wide
                // margin even with the same nominal dps.
                b.radius = 4.0;
                b.weapon = Some(kind);
                b.orbit = Some(Orbit {
                    angle,
                    radius,
                    speed: 3.0,
                });
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, radius);
            if evolved {
                // Fairy Ring: every second cast also leaves its satellites
                // fixed around the casting point for 3 s. `fairy_ring_system`
                // burns whoever crosses the band (once per second each).
                // The parity is scoped to **this weapon instance** (kind +
                // level + evolved), never to the world: a Molt or a dropped
                // and re-acquired orbit starts its own count.
                let key = (kind as u32) ^ ((wlevel as u32) << 8) ^ ((evolved as u32) << 16);
                let casts = evo.count_fairy_cast(key);
                if casts.is_multiple_of(2) {
                    commands.spawn((
                        FairyRing::new(
                            ppos,
                            radius,
                            // A fixed ring is a lingering friendly zone.
                            stats.zone_life(evolutions::FAIRY_SECS),
                            projs.clamp(1, u8::MAX as i32) as u8,
                            dmg,
                        ),
                        next_id.take(),
                    ));
                }
            }
            fired = true;
        }
        WeaponKind::TailWhip => {
            // Batch 12 (corrected): a **quick low swipe in front of the rat** —
            // a 180° arc centred on the aim (mouse/pad) or the rat's facing
            // (keyboard), short reach, fast cadence. Every body in that front
            // half-circle takes the hit, so the whip is the rat's near guard;
            // Tail Thrash is the full-circle spin and the Spiked Tail is the
            // one that targets a single body. Damage per body per cycle is
            // unchanged: the same `dmg` on the same cooldown.
            let i = (wlevel.clamp(1, 5) as usize) - 1;
            let reach = stats.melee_reach(WHIP_REACH[i]);
            let dir = cast_dir(ppos, pface, aim);
            for &entity in order {
                let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                    continue;
                };
                // A swipe, not an aura: only the front half-circle.
                if !e.alive()
                    || !cone_hits_circle(ppos, dir, reach, WHIP_ARC_HALF, e.pos, e.radius)
                {
                    continue;
                }
                // It does not reach through a maze partition.
                if !los_clear(level, ppos, e.pos, MELEE_LOS_RADIUS) {
                    continue;
                }
                let hp_before = e.hp.max(0.0);
                // The whip arrives from the rat: the shell can eat it.
                let applied = e.hurt_from(dmg, ppos);
                let killed = !e.alive();
                // Balance instrumentation only (see `DamageLedger`).
                ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
                let pos = e.pos;
                let enemy_radius = e.radius;
                let enemy_id = e.id;
                // Counterweight: +2 px of outward shove per stack on a melee
                // hit that has no native knockback (the whip), once per body
                // per 0.4 s. `resolve_circle` keeps the nudge on legal floor
                // instead of pushing a body into a wall.
                if stats.shove_px > 0.0 && melee_nudge.ready(enemy_id as u64) {
                    let out = (pos - ppos).normalized();
                    melee_nudge.mark(enemy_id as u64, MELEE_NUDGE_COOLDOWN);
                    let shifted = pos + out * stats.shove_px;
                    e.pos = level.tiles.resolve_circle(
                        shifted,
                        e.radius,
                        SizeClass::Small,
                        None,
                    );
                    *moved = true;
                }
                vfx::emit_tail_whip_hit(particles, &mut rng.0, pos, pos - ppos, killed);
                damage_numbers.record(
                    enemy_id,
                    pos,
                    enemy_radius,
                    applied,
                    killed,
                    DamageSource::TailWhip,
                );
            }
            // The crescent itself: the arc the swipe just cut, drawn for
            // `WHIP_SWIPE_SECS` (`components::WhipCrack`). A decoration: no
            // `SpawnId`, so no other system's order moves. A swing happens even
            // when it hits nothing, so the player sees the arc every time.
            commands.spawn(WhipCrack {
                from: ppos,
                to: ppos + dir * reach,
                t: WHIP_SWIPE_SECS,
                total: WHIP_SWIPE_SECS,
            });
            // The full boss is not an `Enemy`: its hit shapes take the arc,
            // with **this cast's** Lens Cap-stretched reach and Short
            // Teeth-scaled damage. Resolving it here (rather than from a "the
            // whip fired" signal another system watches for) keeps a consumed
            // `Sfx` out of the simulation: whether the caller drains audio can
            // never change a boss's HP.
            for mut boss in bosses.iter_mut() {
                if boss.dead {
                    continue;
                }
                let shapes = boss.hit_shapes();
                // An airborne boss (no grounded shape) is out of a floor-level
                // attack's reach, exactly as it is for bullets.
                if !shapes.iter().any(|s| s.grounded()) {
                    continue;
                }
                // Inside the front half-circle: the boss's own body radius is
                // the slack, so standing against it always counts.
                if !cone_hits_circle(
                    ppos,
                    dir,
                    reach,
                    WHIP_ARC_HALF,
                    boss.pos,
                    boss.kind.body_radius(),
                ) || level.tiles.segment_hit(ppos, boss.pos, 1.0).is_some()
                {
                    continue;
                }
                let amount = dmg * boss.damage_mult;
                if !boss.take_damage(amount) {
                    continue;
                }
                ledger.add_boss(kind, amount);
                let killed = boss.hp <= 0.0;
                vfx::emit_tail_whip_hit(particles, &mut rng.0, boss.pos, boss.pos - ppos, killed);
                damage_numbers.record(
                    boss.id,
                    boss.pos,
                    boss.kind.body_radius(),
                    amount,
                    killed,
                    DamageSource::TailWhip,
                );
            }
            if evolved {
                // Ratchet Tail hooks one of the bodies the swipe actually cut
                // and swings it a quarter turn around the rat, so the grid has
                // to follow it (below).
                if ratchet_swing(
                    level,
                    ppos,
                    pface,
                    dir,
                    reach,
                    order,
                    enemies,
                    dmg,
                    rng,
                    particles,
                    damage_numbers,
                    ledger,
                ) {
                    *moved = true;
                }
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, dir, reach);
            fired = true;
        }
        WeaponKind::AcidSpit => {
            // Lob a projectile: it does no damage in flight and only becomes a
            // pool once it lands (target, wall, or timeout). Lens Cap stretches
            // the lob's reach the same way it stretches a dart's life.
            let reach = stats.ranged_reach(target_range(kind));
            let target = match aim_dir(ppos, aim) {
                // Aimed: lob to the reticle, clamped to the weapon's range and
                // to a clear line (a lobbed pool behind a wall would be wasted,
                // so a blocked aim falls back to the facing lob).
                Some(d) => {
                    let wanted = (aim.unwrap_or(ppos) - ppos).len().min(reach);
                    let at = ppos + d * wanted;
                    if los_clear(level, ppos, at, 2.0) {
                        at
                    } else {
                        ppos + pface * 50.0
                    }
                }
                None => nearest_visible_within(
                    level,
                    kind,
                    reach,
                    order,
                    enemies,
                    boss_points,
                    ppos,
                    target_scratch,
                )
                .unwrap_or(ppos + pface * 50.0),
            }
            .clamp_box(Vec2::splat(4.0), level.arena - Vec2::splat(4.0));
            let to = target - ppos;
            let speed = 110.0;
            let mut b = Bullet::new(ppos, to.normalized() * speed, dmg, Faction::Player, color);
            b.size = 2;
            b.radius = 3.0;
            b.life = to.len() / speed + 0.5;
            b.weapon = Some(kind);
            b.lob = Some(Lob {
                target,
                // Long Fuse: splash radius; its `life` below is the pool's life.
                radius: stats.splash_radius(12.0 + projs as f32 * 1.5),
                // 4.0 was the strongest weapon in both measured columns; the
                // per-weapon report now puts Acid Spit at ~29% of all damage
                // and of boss damage, above the 20-25% band. 3.3 takes ~6%
                // more off the pool without touching the lob/land feel.
                dps: dmg * 3.3,
                life: stats.zone_life(2.6),
                color: (140, 220, 70),
            });
            if evolved {
                // Drain Cleaner: the landing turns into a directed corrosive
                // channel instead of a round pool (`projectiles` builds it;
                // `evolutions::drain_cleaner_system` runs it).
                commands.spawn((
                    b,
                    EvolvedShot { kind, dir: to.normalized() },
                    next_id.take(),
                ));
            } else {
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, to, 0.0);
            fired = true;
        }
        // Active weapons are player-triggered: `fire_weapons_system` skips
        // them and `active_weapons_system` owns everything they do, so this arm
        // is unreachable by construction.
        WeaponKind::TailThrash | WeaponKind::SonicSqueak => {}
        // Wave 3's two self-driving weapons are skipped by
        // `fire_weapons_system` too: `weapons_wave3b` owns the laser burst and
        // the dash hit, so this arm is unreachable by construction.
        WeaponKind::LaserEyes | WeaponKind::SuperDash => {}
        // Wave 3 weapons A (see `weapons_wave3a`): the volley locks its marks
        // and spawns its drops here; the tag plants one `Infection`.
        WeaponKind::AcidRain => {
            let i = (wlevel.clamp(1, 5) as usize) - 1;
            fired = crate::game::ecs::systems::weapons_wave3a::fire_acid_rain(
                commands,
                level,
                order,
                enemies,
                boss_points,
                ppos,
                // Lens Cap stretches the marking reach (a ranged weapon);
                // Long Fuse widens the splash.
                stats.ranged_reach(crate::game::weapon::RAIN_RANGE),
                dmg,
                crate::game::weapon::RAIN_DROPS[i],
                stats.splash_radius(crate::game::weapon::RAIN_RADIUS[i]),
                // `Weapon::evolved`: Downspout's marching gutter.
                evolved,
                particles,
                rng,
                next_id,
                aim,
            );
        }
        // The Spiked Tail (wave 3, `weapon_spiketail`): pick one body in reach
        // with a clear line and spawn the lash that will crack at it. The
        // wind-up and the hit are the system's job, so this arm only snapshots
        // the level's numbers — **with the passive hooks the weapon claims**:
        // Lens Cap stretches a melee weapon's reach, Long Fuse widens a splash,
        // Counterweight its native shove.
        //
        // Aimed (mouse/gamepad): the aim point goes to the target pick, which
        // then takes the eligible body nearest the reticle. `None` is auto-aim.
        WeaponKind::SpikedTail => {
            let i = (wlevel.clamp(1, 5) as usize) - 1;
            fired = crate::game::ecs::systems::weapon_spiketail::fire_spiked_tail(
                commands,
                level,
                order,
                enemies,
                boss_points,
                ppos,
                stats.melee_reach(crate::game::weapon::LASH_REACH[i]),
                stats.splash_radius(crate::game::weapon::LASH_RADIUS[i]),
                dmg,
                stats.shove(crate::game::weapon::LASH_SHOVE),
                crate::game::weapon::LASH_SHARDS[i],
                evolved,
                next_id,
                aim,
                view,
            );
        }
        WeaponKind::Hantavirus => {
            let i = (wlevel.clamp(1, 5) as usize) - 1;
            fired = crate::game::ecs::systems::weapons_wave3a::fire_hantavirus(
                commands,
                level,
                ppos,
                pface,
                dmg,
                crate::game::weapon::VIRUS_JUMPS[i],
                // Lens Cap stretches the jump reach the chain carries with it,
                // and the dropping's trigger radius (both are ranged reaches).
                stats.ranged_reach(crate::game::weapon::VIRUS_REACH[i]),
                stats.ranged_reach(crate::game::weapon::DROPPING_RADIUS),
                crate::game::weapon::DROPPING_CAP[i],
                // The cap's victims: the oldest live dropping goes first.
                droppings,
                next_id,
            );
        }
    }
    // Only announce a shot when the weapon actually produced something
    // (Gnaw with no target in range fires nothing). A weapon whose cue belongs
    // to its effect rather than its cast pushes nothing here: **Hantavirus's
    // drop is silent**, and `Sfx::Infect` plays on the tick a body actually
    // steps in one (`weapons_wave3a::droppings_system`).
    if fired && !matches!(kind, WeaponKind::Hantavirus) {
        let sound = match kind {
            WeaponKind::Gnaw => Sfx::Shoot,
            WeaponKind::CheeseSpray => Sfx::ShootSpray,
            WeaponKind::SqueakNova => Sfx::ShootNova,
            WeaponKind::SporeOrbit => Sfx::ShootOrbit,
            WeaponKind::TailWhip => Sfx::ShootWhip,
            WeaponKind::AcidSpit => Sfx::ShootAcid,
            // Announced by `active_weapons_system` instead.
            WeaponKind::TailThrash => Sfx::ShootThrash,
            WeaponKind::SonicSqueak => Sfx::SonicSqueak,
            // Announced by `weapons_wave3b` instead.
            WeaponKind::LaserEyes => Sfx::LaserEyes,
            WeaponKind::SuperDash => Sfx::SuperDash,
            WeaponKind::AcidRain => Sfx::ShootRain,
            // Unreachable: filtered above (the drop is silent).
            WeaponKind::Hantavirus => Sfx::Infect,
            // The whip-crack. Announce it at the *bristle*, because the sample
            // itself spends `LASH_WINDUP` on a rising swish before its crack;
            // that puts the snap under the frame the tail actually lands on
            // (see `LASH_WINDUP` in `weapon.rs`), and keeps a consumed `Sfx`
            // out of the simulation — whether the caller drains audio can
            // never change a body's HP.
            WeaponKind::SpikedTail => Sfx::Lash,
        };
        sfx.0.push(sound);
    }
    fired
}


/// **Ratchet Tail**: hook the nearest ordinary body the whip hit and swing it a
/// quarter turn around the rat, one base hit into everything the arc crosses.
/// Heavy bodies (elites, Brutes, the boss) never get hooked — they took the
/// whip hit and nothing more.
#[allow(clippy::too_many_arguments)]
fn ratchet_swing(
    level: &Level,
    ppos: Vec2,
    pface: Vec2,
    // The swipe's arc: Ratchet Tail hooks the nearest foe the whip actually
    // cut, which since batch 12 means inside the front half-circle.
    swipe_dir: Vec2,
    reach: f32,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    dmg: f32,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
) -> bool {
    let kind = WeaponKind::TailWhip;
    // 1. The hook: nearest ordinary body inside the arc, ties by `SpawnId`.
    let mut hook: Option<(Entity, Vec2, f32, f32)> = None; // entity, pos, dist, radius
    for &entity in order {
        let Ok((_, _, e)) = enemies.get(entity) else {
            continue;
        };
        // Heavy targets take the whip hit only (§3): elites, named
        // minibosses and every `EnemyKind::heavy()` body, a Pillbug included
        // (curled or not, it is never hooked or swung into).
        if !e.alive() || e.elite || e.named || e.kind.heavy() {
            continue;
        }
        let d = e.pos.dist(ppos);
        if d > reach + e.radius || d <= 1.0 {
            continue;
        }
        if !cone_hits_circle(ppos, swipe_dir, reach, WHIP_ARC_HALF, e.pos, e.radius) {
            continue;
        }
        if !los_clear(level, ppos, e.pos, MELEE_LOS_RADIUS) {
            continue;
        }
        if hook.is_none_or(|(_, _, best, _)| d < best) {
            hook = Some((entity, e.pos, d, e.radius));
        }
    }
    let Some((hooked, from, radius_from_rat, hooked_radius)) = hook else {
        return false;
    };
    // 2. Which way round? Sweep into more bodies; an exact tie follows the
    //    facing side of the hook (and clockwise when that is zero too).
    let offset = from - ppos;
    let start_angle = offset.angle();
    // The swung body travels the arc at `radius_from_rat`; a body behind
    // masonry is not on the arc's path even when both endpoints of the quarter
    // turn are open floor (the review's "a 90° arc can cross a blocked
    // intermediate point"). Sampled at 8 samples per quarter turn — a 1 px
    // tile test per sample, and only for the handful of bodies that already
    // matched the radius band.
    let arc_clear = |delta: f32| -> bool {
        let steps = ((delta.abs() / (std::f32::consts::FRAC_PI_2 / 8.0)).ceil() as u32).max(1);
        (0..=steps).all(|i| {
            let a = start_angle + delta * (i as f32 / steps as f32);
            los_clear(
                level,
                ppos,
                ppos + Vec2::from_angle(a) * radius_from_rat,
                MELEE_LOS_RADIUS,
            )
        })
    };
    let count_in = |turn: f32| -> u32 {
        let mut n = 0;
        for &entity in order {
            let Ok((_, _, e)) = enemies.get(entity) else {
                continue;
            };
            if entity == hooked || !e.alive() {
                continue;
            }
            let rel = e.pos - ppos;
            let d = rel.len();
            if (d - radius_from_rat).abs() > hooked_radius + e.radius + 2.0 {
                continue;
            }
            let mut delta = (rel.angle() - start_angle) % std::f32::consts::TAU;
            if delta < 0.0 {
                delta += std::f32::consts::TAU;
            }
            let in_arc = if turn >= 0.0 {
                delta <= turn + 0.12
            } else {
                delta >= std::f32::consts::TAU + turn - 0.12
            };
            if !in_arc {
                continue;
            }
            // Only a body the hook can actually reach counts toward the
            // swing's direction (it samples the same cover rule as the hit).
            // The signed sweep to this body, held inside the quarter turn (the
            // `in_arc` test above admits a 0.12 rad overshoot): `[0, turn]`
            // counter-clockwise, `[turn, 0]` clockwise.
            let swing = if turn >= 0.0 {
                delta.min(turn)
            } else {
                (delta - std::f32::consts::TAU).max(turn)
            };
            if !arc_clear(swing) {
                continue;
            }
            n += 1;
        }
        n
    };
    let half_pi = std::f32::consts::FRAC_PI_2;
    let cw = count_in(-half_pi);
    let ccw = count_in(half_pi);
    let cross = offset.x * pface.y - offset.y * pface.x;
    let turn = if ccw > cw {
        half_pi
    } else if cw > ccw || cross < 0.0 {
        -half_pi
    } else {
        half_pi
    };
    // 3. The release point must be open floor; a hook that cannot swing just
    //    keeps the whip hit.
    let dest = ppos + Vec2::from_angle(start_angle + turn) * radius_from_rat;
    let class = crate::game::collision::SizeClass::of_enemy(EnemyKind::Skitterer);
    if !level.tiles.circle_clear(dest, hooked_radius, class) {
        return false;
    }
    // 4. One base hit each into everything the quarter arc crosses.
    for &entity in order {
        if entity == hooked {
            continue;
        }
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || e.elite || e.named || e.kind.heavy() {
            continue;
        }
        let rel = e.pos - ppos;
        let d = rel.len();
        if (d - radius_from_rat).abs() > hooked_radius + e.radius + 2.0 {
            continue;
        }
        let mut delta = (rel.angle() - start_angle) % std::f32::consts::TAU;
        if delta < 0.0 {
            delta += std::f32::consts::TAU;
        }
        let in_arc = if turn >= 0.0 {
            delta <= turn
        } else {
            delta >= std::f32::consts::TAU + turn
        };
        if !in_arc {
            continue;
        }
        // The hook has to reach this body: sample the sub-arc it travels to
        // get there against the tile map. Both endpoints being open is not
        // enough — a corner in the middle of the quarter turn is cover.
        let swing = if turn >= 0.0 {
            delta
        } else {
            delta - std::f32::consts::TAU
        };
        if !arc_clear(swing) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        // Directional armour: the swung body arrives from the rat's side,
        // like the whip itself.
        let applied = e.hurt_from(dmg, ppos);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let radius = e.radius;
        let id = e.id;
        vfx::emit_tail_whip_hit(particles, &mut rng.0, pos, pos - ppos, killed);
        damage_numbers.record(id, pos, radius, applied, killed, DamageSource::TailWhip);
    }
    // 5. Release: the body lands a quarter turn around the rat.
    let mut moved = false;
    if let Ok((_, _, mut e)) = enemies.get_mut(hooked) {
        e.pos = dest;
        e.vel = Vec2::ZERO;
        moved = true;
    }
    vfx::emit_ratchet_swing(particles, &mut rng.0, ppos, radius_from_rat, start_angle, turn);
    moved
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::{
        BiteSnap, Candidate, RANK_BOSS, RANK_ENEMY, TARGET_LOS_CANDIDATES, WhipCrack,
        fire_weapons_system, nearest_of,
    };
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::ecs::systems::boss::{Boss, BossKind};
    use crate::game::bullet::Bullet;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::components::{Entity, SpawnId};
    use crate::game::ecs::resources::EnemyIndex;
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::ecs::components::{EvolvedShot, MilkTooth};
    use crate::game::level::{MAP_COLS, MAP_ROWS, Theme, TileRect};
    use crate::game::particle::Particles;
    use crate::game::particle::damage_number::DamageNumbers;
    use crate::game::player::Player;
    use crate::game::weapon::{Weapon, WeaponKind};
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Wipe the generated layout's walls so the test measures targeting, not
    /// the map.
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

    /// Every kind that spawns a projectile (Tail Whip is an aura).
    /// The weapons that still spawn a projectile. Batch 12 took Gnaw (a bite)
    /// and Squeak Nova (a ring) off this list: neither fires anything any more.
    const PROJECTILE_WEAPONS: [WeaponKind; 3] = [
        WeaponKind::CheeseSpray,
        WeaponKind::SporeOrbit,
        WeaponKind::AcidSpit,
    ];

    /// Replace the loadout with a single level-1 weapon whose cooldown is
    /// ready, which is what the legacy tests get by calling `World::fire_one`
    /// directly with `cd = 1.0`.
    fn arm(w: &mut World, kind: WeaponKind) {
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level: 1,
            timer: 0.0,
            evolved: false,
        }];
    }

    fn run_weapons(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
    }

    /// Run the weapon pass with a reticle (wave 3 controls).
    fn run_weapons_aim(w: &mut World, dt: f32, aim: Option<Vec2>) {
        w.begin_tick_with_aim(dt, Vec2::ZERO, false, false, aim);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
    }

    /// An aimed lob is clamped to the weapon's reach and stays on the aim
    /// line. (Gnaw is a bite since batch 12: its aimed cone is pinned by
    /// `gnaw_bites_one_body_at_arm_s_length`.)
    #[test]
    fn an_aimed_lob_is_clamped_to_its_reach() {
        // Far past the reach: clamped to it, still along the aim.
        let mut w = World::new(Theme::Sewer, 26);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::AcidSpit);
        let p = w.player.pos;
        run_weapons_aim(&mut w, 1.0 / 60.0, Some(p + Vec2::new(4000.0, 0.0)));
        let target = w.bullets()[0].lob.expect("a lob").target;
        let d = (target - p).len();
        assert!(d < 4000.0 && d > 100.0, "aim range was not clamped: {d}");
        assert!((target.y - p.y).abs() < 1.0, "the clamped lob left the aim line");
    }

    /// **Milk Teeth, aimed**: the evolved lead tooth follows the reticle too.
    #[test]
    fn milk_teeth_follows_the_aim() {
        let mut w = World::new(Theme::Sewer, 27);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::Gnaw);
        w.player.loadout.weapons[0].level = 5;
        w.player.loadout.weapons[0].evolved = true;
        let p = w.player.pos;
        // The tooth is spat with the bite, so there is a body in the jaw.
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 14.0), 50.0);
        run_weapons_aim(&mut w, 1.0 / 60.0, Some(p + Vec2::new(0.0, 150.0)));
        let mut q = w.ecs.query::<(&MilkTooth, &SpawnId)>();
        let tooth = q
            .iter(&w.ecs)
            .min_by_key(|(_, id)| id.0)
            .map(|(t, _)| t.vel)
            .expect("the lead tooth");
        assert!(tooth.y > 0.0 && tooth.x.abs() < 1.0, "tooth flew {tooth:?}");
    }

    /// **Fondue Fork, aimed**: the two tines straddle the reticle, so the
    /// string plants across the aim line rather than across the old facing.
    /// (The tines are projectiles: the string itself appears once both have
    /// planted, so the cast is read from the tines' flight.)
    #[test]
    fn fondue_fork_tines_straddle_the_aim() {
        let mut w = World::new(Theme::Sewer, 28);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::CheeseSpray);
        w.player.loadout.weapons[0].level = 5;
        w.player.loadout.weapons[0].evolved = true;
        let p = w.player.pos;
        run_weapons_aim(&mut w, 1.0 / 60.0, Some(p + Vec2::new(0.0, 160.0)));
        let mut q = w.ecs.query::<&crate::game::ecs::components::FondueTine>();
        let tines: Vec<Vec2> = q.iter(&w.ecs).map(|t| t.vel).collect();
        assert_eq!(tines.len(), 2, "the fork throws two tines");
        // Both fly to the aim's forward side (+y) and straddle it.
        assert!(tines.iter().all(|v| v.y > 0.0), "tines behind the aim: {tines:?}");
        assert!(
            tines[0].x * tines[1].x < 0.0,
            "tines did not straddle the aim: {tines:?}"
        );
    }

    /// **Drain Cleaner, aimed**: the channel's direction is the lob's, and the
    /// lob is the reticle's.
    #[test]
    fn drain_cleaner_channel_follows_the_aim() {
        let mut w = World::new(Theme::Sewer, 29);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::AcidSpit);
        w.player.loadout.weapons[0].level = 5;
        w.player.loadout.weapons[0].evolved = true;
        let p = w.player.pos;
        run_weapons_aim(&mut w, 1.0 / 60.0, Some(p + Vec2::new(-120.0, 0.0)));
        let mut q = w.ecs.query::<&EvolvedShot>();
        let shot = q.iter(&w.ecs).next().expect("the evolved channel");
        assert!(
            shot.dir.x < -0.99,
            "the channel did not follow the reticle: {:?}",
            shot.dir
        );
    }

    /// **Milk Teeth** adds its tooth on top of the bite: the level-5 bite
    /// still lands both old darts' damage, so an earned evolution is never a
    /// downgrade in output, and no Gnaw projectile is fired.
    #[test]
    fn milk_teeth_spits_one_tooth_on_top_of_the_bite() {
        let mut w = World::new(Theme::Sewer, 31);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::Gnaw);
        w.player.loadout.weapons[0].level = WeaponKind::Gnaw.max_level();
        w.player.loadout.weapons[0].evolved = true;
        w.player.loadout.weapons[0].timer = 0.0;
        let per_dart = w.player.loadout.weapons[0].damage();
        let target = w.player.pos + Vec2::new(14.0, 0.0);
        let body = w.spawn_enemy(EnemyKind::Brute, target, 50.0);
        let hp = w.ecs.get::<Enemy>(body).unwrap().hp;
        run_weapons(&mut w, 1.0 / 60.0);

        let mut darts = w.ecs.query::<&crate::game::ecs::components::MilkTooth>();
        let teeth = darts.iter(&w.ecs).count();
        let gnaw = w
            .bullets()
            .iter()
            .filter(|b| b.weapon == Some(WeaponKind::Gnaw))
            .count();
        assert_eq!(teeth, 1, "the evolved Gnaw spits one Milk Tooth");
        assert_eq!(gnaw, 0, "the bite fires no dart");
        let lost = hp - w.ecs.get::<Enemy>(body).unwrap().hp;
        assert!(
            (lost - per_dart * 2.0).abs() < 1e-3,
            "the level-5 bite lands both darts' worth: {lost} vs {per_dart} x 2"
        );
    }

    /// The whip's boss damage is resolved with the cast (reach, damage and
    /// attribution), not from `Sfx::ShootWhip`: twin worlds that drain the
    /// sound queue differently must see the same boss HP.
    #[test]
    fn whip_boss_damage_does_not_depend_on_draining_audio() {
        use crate::game::ecs::systems::boss::{Boss, BossKind};
        let build = || {
            let mut w = World::new(Theme::Sewer, 41);
            w.clear_enemies();
            arm(&mut w, WeaponKind::TailWhip);
            let mid = w.level.arena * 0.5;
            w.player.pos = mid + Vec2::new(-12.0, 0.0);
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .unwrap_or_else(|| crate::game::level::TileRect::new(0, 0, 10, 10));
            let mut boss = Boss::new(BossKind::Cat, mid, 5000.0, arena, mid, false);
            boss.invuln = 0.0;
            w.ecs.spawn((boss, crate::game::ecs::components::SpawnId(999)));
            w
        };
        let mut drained = build();
        let mut undrained = build();
        let hp0 = {
            let mut q = drained.ecs.query::<&Boss>();
            q.iter(&drained.ecs).next().unwrap().hp
        };
        // Two ticks each; only one of them clears `sfx` between ticks.
        for _ in 0..2 {
            drained.update(1.0 / 60.0, Vec2::ZERO, false);
            drained.sfx.clear();
            undrained.update(1.0 / 60.0, Vec2::ZERO, false);
        }
        let hp = |w: &mut World| {
            let mut q = w.ecs.query::<&Boss>();
            q.iter(&w.ecs).next().unwrap().hp
        };
        assert!(hp(&mut drained) < hp0, "the whip must hurt a boss in reach");
        assert_eq!(
            hp(&mut drained),
            hp(&mut undrained),
            "boss HP must not depend on whether the caller drained the sfx queue"
        );
    }

    /// A weapon that **moved** a body (Counterweight's melee nudge, Ratchet
    /// Tail's swing) re-files the spatial grid, so this tick's bullet sweep
    /// still finds the body where it now stands.
    ///
    /// White-box on purpose: the bucket contents are what the sweep reads, and
    /// the failure this pins is precisely a body that is in `EnemyIndex` but in
    /// the grid's *old* cell.
    #[test]
    fn a_nudged_body_is_re_filed_in_the_grid() {
        use crate::game::collision::SpatialGrid;
        use crate::game::ecs::resources::{EnemyIndex, NeighborScratch};
        use crate::game::loadout::PassiveKind;
        let mut w = World::new(Theme::Sewer, 43);
        w.clear_enemies();
        // The whip swipes the half-circle in front of the rat, so face it.
        w.player.face = Vec2::new(1.0, 0.0);
        arm(&mut w, WeaponKind::TailWhip);
        // Counterweight: +2 px of outward shove per stack on the whip, once per
        // body per 0.4 s.
        w.player.loadout.passives.push((PassiveKind::Counterweight, 4));
        w.player.loadout.rebuild_stats();
        let ppos = w.player.pos;
        // Right at the whip's inner reach, so the nudge of 8 px moves it.
        let body = w.spawn_enemy(EnemyKind::Brute, ppos + Vec2::new(18.0, 0.0), 400.0);
        let before = w.ecs.get::<Enemy>(body).unwrap().pos;
        // The schedule's order: grid build, then the weapon pass.
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::enemy_move::grid_build_system)
            .unwrap();
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        let after = w.ecs.get::<Enemy>(body).unwrap().pos;
        let slot = {
            let index = w.ecs.resource::<EnemyIndex>();
            index
                .0
                .iter()
                .position(|&e| e == body)
                .expect("the body is in the index")
        };
        // Ask the grid at the body's **new** position, exactly like a bullet.
        let mut neigh = NeighborScratch::default();
        w.ecs
            .resource::<SpatialGrid>()
            .neighbors_within(after, 1.0, &mut neigh.0);
        w.end_tick();
        assert!(
            after.dist_sq(before) > 0.01,
            "the whip's Counterweight nudge did not move the body"
        );
        assert!(
            neigh.0.iter().any(|&j| j as usize == slot),
            "the grid still lists the body in its old cell ({before:?} -> {after:?})"
        );
    }

    /// Fire `kind` once at a fixed near target so targeting weapons always
    /// shoot. Returns the world after the tick.
    fn fire_kind(kind: WeaponKind) -> World {
        let mut w = World::new(Theme::Sewer, 1234);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        arm(&mut w, kind);
        let target = w.player.pos + Vec2::new(10.0, 0.0);
        w.spawn_enemy(EnemyKind::Cat, target, 1.0);
        run_weapons(&mut w, 1.0 / 60.0);
        w
    }

    /// Orbit angles in stable `SpawnId` order (legacy `Vec<Bullet>` order).
    fn orbit_angles(ecs: &mut bevy_ecs::world::World) -> Vec<f32> {
        let mut query = ecs.query::<(&SpawnId, &Bullet)>();
        query
            .iter(ecs)
            .sort_by_key::<&SpawnId, _>(|id| id.0)
            .filter_map(|(_, b)| b.orbit.map(|o| o.angle))
            .collect()
    }

    /// Legacy `spore_orbit_replaces_its_ring_and_keeps_its_angle`: a second
    /// volley drops the old ring instead of stacking, and the first new orbit
    /// continues from the old ring's first angle.
    #[test]
    fn spore_orbit_replaces_its_ring_and_keeps_its_angle() {
        let mut w = World::new(Theme::Sewer, 4);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::SporeOrbit);
        // Level 2 grants 2 + level = 4 orbit spores, the legacy test's
        // `projs = 4`, without bypassing the loadout's projectile count.
        w.player.loadout.weapons[0].level = 2;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        assert_eq!(orbit_angles(&mut w.ecs).len(), 4);

        // Legacy test: advance every orbit angle by 0.7 before re-firing.
        {
            let mut query = w.ecs.query::<&mut Bullet>();
            for mut b in query.iter_mut(&mut w.ecs) {
                if let Some(o) = b.orbit.as_mut() {
                    o.angle += 0.7;
                }
            }
        }
        let carried = orbit_angles(&mut w.ecs)[0];

        // Ready the weapon again, exactly like resetting `weapons[i].timer`.
        w.ecs
            .resource_mut::<Player>()
            .loadout
            .weapons[0]
            .timer = 0.0;
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        let angles = orbit_angles(&mut w.ecs);
        w.end_tick();

        assert_eq!(angles.len(), 4, "two spore rings stacked");
        assert!(
            (angles[0] - carried).abs() < 1e-4,
            "ring angle snapped instead of carrying forward"
        );
    }

    /// Legacy `untargeted_gnaw_does_not_emit_a_shoot_sfx`.
    #[test]
    fn untargeted_gnaw_does_not_emit_a_shoot_sfx() {
        let mut w = World::new(Theme::Sewer, 2);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(w.bullets().is_empty());
        assert!(!w.sfx.contains(&Sfx::Shoot));
    }

    /// Legacy `untargeted_gnaw_retries_next_tick`: a dud shot leaves the timer
    /// at exactly 0 so the weapon retries instead of serving a cooldown.
    #[test]
    fn untargeted_gnaw_retries_next_tick() {
        let mut w = World::new(Theme::Sewer, 61);
        w.clear_enemies();
        w.clear_bullets();
        w.player.loadout.weapons[0].timer = 0.0;

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(w.bullets().is_empty(), "Gnaw fired with no target");
        assert_eq!(
            w.player.loadout.weapons[0].timer, 0.0,
            "a dud shot should leave the timer ready for next tick"
        );
    }

    /// Legacy `nearest_enemy_skips_dead_and_nan_positions`, exercised through
    /// the firing system: the dart must aim at the live enemy, not the dead one
    /// sitting on the player or the NaN one.
    #[test]
    fn nearest_enemy_skips_dead_and_nan_positions() {
        let mut w = World::new(Theme::Sewer, 29);
        w.clear_enemies();
        w.clear_bullets();
        let mut rng = StdRng::seed_from_u64(3);
        let from = w.player.pos;

        let dead = w.spawn_enemy_with_rng(EnemyKind::Skitterer, from, 1.0, &mut rng);
        w.ecs.get_mut::<Enemy>(dead).unwrap().hp = 0.0;
        let nan = w.spawn_enemy_with_rng(EnemyKind::Skitterer, from, 1.0, &mut rng);
        w.ecs.get_mut::<Enemy>(nan).unwrap().pos = Vec2::new(f32::NAN, f32::NAN);
        let alive_pos = from + Vec2::new(10.0, 0.0);
        w.spawn_enemy_with_rng(EnemyKind::Skitterer, alive_pos, 1.0, &mut rng);
        arm(&mut w, WeaponKind::Gnaw);

        let alive = {
            let mut q = w.ecs.query::<(Entity, &Enemy)>();
            q.iter(&w.ecs)
                .find(|(_, e)| e.alive() && e.pos.x.is_finite())
                .map(|(entity, e)| (entity, e.hp))
                .expect("the live enemy")
        };

        run_weapons(&mut w, 1.0 / 60.0);

        // Gnaw is a bite: it closes on exactly the live enemy.
        assert!(
            w.ecs.get::<Enemy>(alive.0).unwrap().hp < alive.1,
            "the bite missed the live enemy"
        );
        assert!(w.bullets().is_empty(), "a bite fires no projectile");
    }

    /// The plain pick shared with `World::nearest_enemy`: ties keep the lowest
    /// `SpawnId` regardless of iteration order, dead entries are skipped and a
    /// NaN distance never wins over a finite one.
    #[test]
    fn nearest_of_is_order_independent_and_stable() {
        let mut w = World::new(Theme::Sewer, 7);
        w.clear_enemies();
        let a = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(10.0, 0.0), 1.0);
        let b = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(-10.0, 0.0), 1.0);
        // Equal distances: the lower SpawnId wins even when it comes second.
        let picked = nearest_of(
            vec![
                (b, SpawnId(9), Vec2::new(-10.0, 0.0), true),
                (a, SpawnId(4), Vec2::new(10.0, 0.0), true),
            ],
            Vec2::ZERO,
        )
        .expect("a live enemy");
        assert_eq!(picked.0, a);

        let dead = w.spawn_enemy(EnemyKind::Skitterer, Vec2::ZERO, 1.0);
        let nan = w.spawn_enemy(EnemyKind::Skitterer, Vec2::ZERO, 1.0);
        let picked = nearest_of(
            vec![
                (dead, SpawnId(1), Vec2::ZERO, false),
                (nan, SpawnId(2), Vec2::new(f32::NAN, f32::NAN), true),
                (a, SpawnId(3), Vec2::new(4.0, 0.0), true),
            ],
            Vec2::ZERO,
        )
        .expect("a live enemy");
        assert_eq!(picked.0, a);
    }

    /// Legacy `acid_spit_launches_a_projectile_not_an_instant_pool`.
    #[test]
    fn acid_spit_launches_a_projectile_not_an_instant_pool() {
        let mut w = World::new(Theme::Sewer, 73);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        arm(&mut w, WeaponKind::AcidSpit);

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(w.bullets().len(), 1);
        assert!(w.bullets()[0].lob.is_some(), "acid shot should be lobbed");
        assert!(w.pools().is_empty(), "acid pool appeared instantly");
    }

    /// Multi-projectile weapons still emit exactly one `Sfx::Shoot`.
    #[test]
    fn one_shoot_sfx_per_successful_shot() {
        let mut w = World::new(Theme::Sewer, 2);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        // Cheese Spray level 1: 2 + level projectiles.
        assert_eq!(w.bullets().len(), 3);
        assert_eq!(
            w.sfx.iter().filter(|s| **s == Sfx::ShootSpray).count(),
            1,
            "one specialized shot sfx per volley"
        );
    }

    /// The Tail Whip is a **frontal half-circle swipe**: every body in the
    /// 180° arc in front of the rat takes the hit in place (`Enemy::hurt`, no
    /// projectile) — including one off to the side at 70° — while a body behind
    /// the rat, or one past the short reach, is never touched.
    #[test]
    fn tail_whip_swipes_the_half_circle_in_front() {
        use crate::game::weapon::WHIP_ARC_HALF;
        let mut w = World::new(Theme::Sewer, 5);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        let ppos = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        // Dead ahead, inside the L1 30 px reach.
        let ahead = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(18.0, 0.0), 1.0);
        // 70° off the axis at 22 px: still inside the half-circle (90°).
        let off_axis = w.spawn_enemy(
            EnemyKind::Cat,
            ppos + Vec2::from_angle(70.0f32.to_radians()) * 22.0,
            1.0,
        );
        // Behind the rat (and inside the reach): the swipe is not an aura.
        let behind = w.spawn_enemy(EnemyKind::Cat, ppos - Vec2::new(18.0, 0.0), 1.0);
        // Past the reach.
        let far = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(80.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::TailWhip);

        run_weapons(&mut w, 1.0 / 60.0);

        let hp = |w: &World, e| w.ecs.get::<Enemy>(e).unwrap().hp;
        let base = EnemyKind::Cat.base_hp();
        assert!(hp(&w, ahead) < base, "the body ahead was not swiped");
        assert!(
            hp(&w, off_axis) < base,
            "a body inside the front half-circle was missed"
        );
        assert_eq!(hp(&w, behind), base, "the swipe reached behind the rat");
        assert_eq!(hp(&w, far), base, "a body past the reach was swiped");
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::ShootWhip).count(), 1);
        assert!(w.bullets().is_empty(), "Tail Whip must not spawn bullets");
        // The drawn swipe: one crescent, centred on the aim at the reach.
        let cracks: Vec<WhipCrack> = w
            .ecs
            .query::<&WhipCrack>()
            .iter(&w.ecs)
            .copied()
            .collect();
        assert_eq!(cracks.len(), 1, "the swipe must be drawn once");
        let arc = cracks[0];
        assert!(
            (arc.to - ppos).normalized().dot(Vec2::new(1.0, 0.0)) > 0.999,
            "the crescent must be centred on the facing"
        );
        assert!(
            (arc.to - ppos).len() > 20.0,
            "the crescent's centre line is the swipe's reach"
        );
        assert!(
            (WHIP_ARC_HALF - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "the swipe is a half-circle"
        );
    }

    /// Gnaw is a **bite**: a very short reach, single target, no projectile, and
    /// the jaw's own `BiteSnap` for the art.
    #[test]
    fn gnaw_bites_one_body_at_arm_s_length() {
        let mut w = World::new(Theme::Sewer, 4242);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        let in_jaws = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(12.0, 0.0), 1.0);
        let second = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(16.0, 6.0), 1.0);
        let out_of_reach = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(60.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::Gnaw);
        run_weapons(&mut w, 1.0 / 60.0);
        let hp = |w: &World, e| w.ecs.get::<Enemy>(e).unwrap().hp;
        let base = EnemyKind::Cat.base_hp();
        assert!(hp(&w, in_jaws) < base, "the nearest body was not bitten");
        assert_eq!(hp(&w, second), base, "the bite must be single-target");
        assert_eq!(hp(&w, out_of_reach), base, "the bite reached too far");
        assert!(w.bullets().is_empty(), "a bite must not spawn a projectile");
        assert_eq!(w.ecs.query::<&BiteSnap>().iter(&w.ecs).count(), 1);
        // The bite took the whole volley's damage (one dart at L1).
        assert!(
            (base - hp(&w, in_jaws) - Weapon::new(WeaponKind::Gnaw).damage()).abs() < 1e-3,
            "the bite must deal one dart's damage at L1"
        );
    }

    /// **Sonic Squeak only hits inside its cone.** A body 30° off the axis is
    /// inside the 38° wedge; one 80° off — at the same distance — is not.
    #[test]
    fn sonic_squeak_only_hits_inside_its_cone() {
        let mut w = World::new(Theme::Sewer, 26);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        let inside = w.spawn_enemy(
            EnemyKind::Skitterer,
            p + Vec2::from_angle(30.0f32.to_radians()) * 40.0,
            1.0,
        );
        let outside = w.spawn_enemy(
            EnemyKind::Skitterer,
            p + Vec2::from_angle(80.0f32.to_radians()) * 40.0,
            1.0,
        );
        let behind = w.spawn_enemy(EnemyKind::Skitterer, p - Vec2::new(30.0, 0.0), 1.0);
        let hp = |w: &World, e| w.ecs.get::<Enemy>(e).unwrap().hp;
        let (i0, o0, b0) = (hp(&w, inside), hp(&w, outside), hp(&w, behind));
        arm_active(&mut w, WeaponKind::SonicSqueak, 0.0);

        tick_active(&mut w, true);

        assert!(hp(&w, inside) < i0, "a body inside the cone was missed");
        assert_eq!(hp(&w, outside), o0, "a body outside the cone was hit");
        assert_eq!(hp(&w, behind), b0, "a body behind the rat was hit");
    }

    /// The three recognisable **groups** now have three different hit regions:
    /// the whip is a front crescent and the thrash a whole disc; the nova is a ring all
    /// the way round and the squeak a forward cone; Gnaw is a bite (no
    /// projectile) and Cheese Spray a fan of wedges that slows.
    #[test]
    fn the_look_alike_groups_have_different_hit_shapes() {
        let hp = |w: &World, e| w.ecs.get::<Enemy>(e).unwrap().hp;
        // --- the tails: whip = the front half-circle, thrash = the full disc
        let mut w = World::new(Theme::Sewer, 31);
        open_map(&mut w);
        w.clear_enemies();
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // 20 px directly behind the rat: inside both reaches, but only the
        // thrash's disc covers it.
        let behind = w.spawn_enemy(EnemyKind::Brute, p - Vec2::new(20.0, 0.0), 400.0);
        // …and one 40° off the front axis: the whip's half-circle takes it.
        let in_front = w.spawn_enemy(
            EnemyKind::Brute,
            p + Vec2::from_angle(40.0f32.to_radians()) * 20.0,
            400.0,
        );
        let (behind_hp, front_hp) = (hp(&w, behind), hp(&w, in_front));
        arm(&mut w, WeaponKind::TailWhip);
        run_weapons(&mut w, 1.0 / 60.0);
        assert!(hp(&w, in_front) < front_hp, "the whip must swipe in front");
        assert_eq!(
            hp(&w, behind),
            behind_hp,
            "the whip is a front half-circle, not a disc"
        );
        // The thrash's full disc takes the body behind the rat.
        w.player.loadout.weapons = vec![Weapon {
            kind: WeaponKind::TailThrash,
            level: 1,
            timer: 0.0,
            evolved: false,
        }];
        tick_active(&mut w, true);
        assert!(hp(&w, behind) < behind_hp, "the thrash sweeps the whole disc");

        // --- the squeaks: nova = a ring all round, squeak = a cone --------
        let mut w = World::new(Theme::Sewer, 32);
        open_map(&mut w);
        w.clear_enemies();
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        let behind = w.spawn_enemy(EnemyKind::Brute, p - Vec2::new(30.0, 0.0), 400.0);
        let before = hp(&w, behind);
        arm(&mut w, WeaponKind::SqueakNova);
        for _ in 0..80 {
            run_weapons(&mut w, 1.0 / 60.0);
            if !w.active_fx().nova_active() {
                break;
            }
        }
        assert!(hp(&w, behind) < before, "the nova ring goes all the way round");
        w.player.loadout.weapons = vec![Weapon {
            kind: WeaponKind::SonicSqueak,
            level: 1,
            timer: 0.0,
            evolved: false,
        }];
        let before = hp(&w, behind);
        tick_active(&mut w, true);
        assert_eq!(hp(&w, behind), before, "the squeak is a cone, not a ring");

        // --- the pellets: Gnaw = a bite with no projectile, Cheese Spray =
        // a fan of wedges that slows what it covers -----------------------
        let mut w = World::new(Theme::Sewer, 33);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(48.0, 0.0), 400.0);
        arm(&mut w, WeaponKind::Gnaw);
        run_weapons(&mut w, 1.0 / 60.0);
        assert!(w.bullets().is_empty(), "Gnaw must not fire a projectile");
        assert_eq!(
            hp(&w, far),
            w.ecs.get::<Enemy>(far).unwrap().max_hp,
            "a 34 px jaw must not reach a body 48 px away"
        );
        // Cheese Spray reaches it with wedges and slows it.
        w.player.loadout.weapons = vec![Weapon {
            kind: WeaponKind::CheeseSpray,
            level: 1,
            timer: 0.0,
            evolved: false,
        }];
        run_weapons(&mut w, 1.0 / 60.0);
        assert!(!w.bullets().is_empty(), "Cheese Spray must fire wedges");
        assert!(
            w.ecs.get::<Enemy>(far).unwrap().slow_t > 0.0,
            "the wedge fan must slow what it covers"
        );
    }

    /// Every auto-firing weapon in `fire_one` serves exactly its own level-1
    /// cooldown for a shot (measured on a zero-length tick), then stays quiet
    /// until that cooldown has run out.
    #[test]
    fn every_auto_weapon_fires_on_its_own_cooldown() {
        for kind in [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::SqueakNova,
            WeaponKind::SporeOrbit,
            WeaponKind::TailWhip,
            WeaponKind::AcidSpit,
        ] {
            let mut w = World::new(Theme::Sewer, 4321);
            open_map(&mut w);
            w.clear_enemies();
            w.clear_bullets();
            w.clear_pools();
            arm(&mut w, kind);
            let at = w.player.pos + Vec2::new(10.0, 0.0);
            w.spawn_enemy(EnemyKind::Brute, at, 400.0);
            run_weapons(&mut w, 0.0);
            let timer = w.player.loadout.weapons[0].timer;
            assert!(
                (timer - kind.base_cooldown()).abs() < 1e-5,
                "{kind:?}: a shot served {timer}, the table says {}",
                kind.base_cooldown()
            );
            // Half a cooldown later it is still cooling (no second shot: a
            // shot would have added a whole cooldown to the timer).
            let cd = kind.base_cooldown();
            run_weapons(&mut w, cd * 0.5);
            let timer = w.player.loadout.weapons[0].timer;
            assert!(
                (timer - cd * 0.5).abs() < 1e-4,
                "{kind:?} fired again inside its cooldown (timer {timer})"
            );
            // Once the rest has run out it fires again and serves a new one.
            run_weapons(&mut w, cd * 0.5 + 0.01);
            let timer = w.player.loadout.weapons[0].timer;
            assert!(
                (timer - (cd - 0.01)).abs() < 1e-3,
                "{kind:?} did not fire when its cooldown ran out (timer {timer})"
            );
        }
    }

    /// Squeak Nova: one **expanding ring** per cast (no spokes), and the ring's
    /// front damages a body as it passes — once on the way out.
    #[test]
    fn squeak_nova_is_one_expanding_ring() {
        use crate::game::ecs::resources::ActiveFx;
        use crate::game::weapon::NOVA_REACH;
        let mut w = World::new(Theme::Sewer, 77);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        // One body just inside the L1 ring, one well outside it.
        let ppos = w.player.pos;
        let inner = w.spawn_enemy(EnemyKind::Brute, ppos + Vec2::new(30.0, 0.0), 400.0);
        let outer = w.spawn_enemy(EnemyKind::Brute, ppos + Vec2::new(180.0, 0.0), 400.0);
        let (inner_hp, outer_hp) = (
            w.ecs.get::<Enemy>(inner).unwrap().hp,
            w.ecs.get::<Enemy>(outer).unwrap().hp,
        );
        arm(&mut w, WeaponKind::SqueakNova);
        run_weapons(&mut w, 0.0);
        let fx = *w.ecs.resource::<ActiveFx>();
        assert!(fx.nova_active(), "the cast must leave a ring playing");
        assert!(
            (fx.nova_max_r - NOVA_REACH).abs() < 1e-3,
            "the ring's radius is the old spokes' flight: {}",
            fx.nova_max_r
        );
        assert!(w.bullets().is_empty(), "the nova must not fire spokes");
        // The front starts at the rat and has not reached the body yet.
        assert_eq!(fx.nova_r, 0.0);
        assert_eq!(w.ecs.get::<Enemy>(inner).unwrap().hp, inner_hp);
        // Step the ring out.
        let mut ticks = 0;
        while ticks < 120 && w.ecs.resource::<ActiveFx>().nova_active() {
            run_weapons(&mut w, 1.0 / 60.0);
            ticks += 1;
        }
        let dmg = Weapon::new(WeaponKind::SqueakNova).damage();
        assert!(
            (inner_hp - w.ecs.get::<Enemy>(inner).unwrap().hp - dmg).abs() < 1e-3,
            "the ring's front must damage what it passes, once"
        );
        assert_eq!(
            w.ecs.get::<Enemy>(outer).unwrap().hp,
            outer_hp,
            "a body outside the ring was hit"
        );
    }

    /// Tail Whip books what it dealt to its own ledger column and to no other.
    #[test]
    fn tail_whip_credits_its_own_ledger_column() {
        let mut w = World::new(Theme::Sewer, 78);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        let at = w.player.pos + Vec2::new(10.0, 0.0);
        let body = w.spawn_enemy(EnemyKind::Brute, at, 400.0);
        let before = w.ecs.get::<Enemy>(body).unwrap().hp;
        w.player.face = Vec2::new(1.0, 0.0);
        arm(&mut w, WeaponKind::TailWhip);
        run_weapons(&mut w, 0.0);
        let lost = before - w.ecs.get::<Enemy>(body).unwrap().hp;
        assert!(lost > 0.0, "the whip missed");
        let ledger = w.ecs.resource::<crate::game::ecs::resources::DamageLedger>();
        for kind in WeaponKind::all() {
            let booked = ledger.dealt[kind.index()];
            if kind == WeaponKind::TailWhip {
                assert!((booked - lost).abs() < 1e-3, "whip column {booked} != {lost}");
            } else {
                assert_eq!(booked, 0.0, "{kind:?} was credited with a whip hit");
            }
        }
    }

    /// Neon hook: every player projectile is tagged with the weapon that fired
    /// it (used by `resolve_player_bullets` for spark/damage-number styling).
    #[test]
    fn every_projectile_is_tagged_with_its_weapon() {
        for kind in PROJECTILE_WEAPONS {
            let mut w = fire_kind(kind);
            let bullets = w.bullets();
            assert!(!bullets.is_empty(), "{kind:?} fired nothing");
            for b in &bullets {
                assert_eq!(b.weapon, Some(kind), "{kind:?} left an untagged bullet");
            }
        }
    }

    /// Neon hook: each weapon emits its own `Sfx` variant (legacy
    /// `Sfx::Shoot` only for Gnaw).
    #[test]
    fn weapons_emit_their_specialized_sfx() {
        let cases = [
            (WeaponKind::Gnaw, Sfx::Shoot),
            (WeaponKind::CheeseSpray, Sfx::ShootSpray),
            (WeaponKind::SqueakNova, Sfx::ShootNova),
            (WeaponKind::SporeOrbit, Sfx::ShootOrbit),
            (WeaponKind::TailWhip, Sfx::ShootWhip),
            (WeaponKind::AcidSpit, Sfx::ShootAcid),
        ];
        for (kind, expected) in cases {
            let w = fire_kind(kind);
            assert_eq!(
                w.sfx.iter().filter(|s| **s == expected).count(),
                1,
                "{kind:?} should emit exactly one {expected:?}, got {:?}",
                w.sfx
            );
        }
    }

    /// Neon hook: `vfx::emit_weapon_fire` runs for every weapon — particles for
    /// most, a shockwave ring for Squeak Nova / Tail Whip.
    #[test]
    fn weapon_fire_emits_muzzle_vfx() {
        let gnaw = fire_kind(WeaponKind::Gnaw);
        assert!(
            !gnaw.particles.is_empty(),
            "Gnaw muzzle particles missing"
        );

        let nova = fire_kind(WeaponKind::SqueakNova);
        assert!(
            !nova.particles.shockwaves.is_empty(),
            "Squeak Nova shockwave missing"
        );

        let whip = fire_kind(WeaponKind::TailWhip);
        assert!(
            !whip.particles.shockwaves.is_empty(),
            "Tail Whip pulse ring missing"
        );
    }

    /// Neon hook: Tail Whip records a floating damage number per clipped
    /// enemy and emits its hit VFX.
    #[test]
    fn tail_whip_records_damage_numbers_and_hit_vfx() {
        let mut w = World::new(Theme::Sewer, 4242);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        let enemy = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(10.0, 0.0), 1.0);
        w.player.face = Vec2::new(1.0, 0.0);
        arm(&mut w, WeaponKind::TailWhip);
        let hp_before = w.ecs.get::<Enemy>(enemy).unwrap().hp;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        let particles_before = w.ecs.resource::<Particles>().len();
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        let hp_after = w.ecs.get::<Enemy>(enemy).unwrap().hp;
        let (numbers_len, first_number) = {
            let numbers = w.ecs.resource::<DamageNumbers>();
            (
                numbers.len(),
                numbers.items.first().map(|n| (n.total_dmg, n.is_kill)),
            )
        };
        let particles_after = w.ecs.resource::<Particles>().len();
        let shockwaves = w.ecs.resource::<Particles>().shockwaves.len();
        w.end_tick();

        assert!(hp_after < hp_before, "Tail Whip did not hurt the enemy");
        assert_eq!(numbers_len, 1, "one damage number per clipped enemy");
        let (total_dmg, is_kill) = first_number.expect("damage number recorded");
        assert!(
            (total_dmg - (hp_before - hp_after)).abs() < 1e-4,
            "number {total_dmg} != dealt {}",
            hp_before - hp_after
        );
        assert!(!is_kill);
        assert!(
            particles_after > particles_before,
            "tail-whip hit VFX missing"
        );
        assert_eq!(shockwaves, 1, "tail-whip pulse ring missing");
    }

    // ---- Active (player-triggered) weapons --------------------------------

    use super::{active_weapons_system, fire_weapons_system as fire_auto};
    use crate::game::bullet::Faction;
    use crate::game::ecs::resources::TickInput;
    use crate::game::weapon::{SQUEAK_STUN, THRASH_BURST};

    /// Drive one tick of the active-weapon system (`press` = the Space press).
    fn tick_active(w: &mut World, press: bool) -> bool {
        tick_active_with(w, press, 1.0 / 60.0)
    }

    fn tick_active_with(w: &mut World, press: bool, dt: f32) -> bool {
        w.begin_tick(dt, Vec2::ZERO, press);
        w.ecs.run_system_cached(active_weapons_system).unwrap();
        w.end_tick();
        w.active_fx().thrash_active() || w.active_fx().squeak_active()
    }

    /// Own `kind` as the only weapon, with `charge` seconds left before it is
    /// ready (0 = fully charged).
    fn arm_active(w: &mut World, kind: WeaponKind, charge: f32) {
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level: 1,
            timer: charge,
            evolved: false,
        }];
    }

    /// A charge meter is spent by one press and refills on its own; a press with
    /// nothing charged does nothing.
    #[test]
    fn active_charge_fills_spends_and_does_nothing_until_full() {
        let mut w = World::new(Theme::Sewer, 21);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        arm_active(&mut w, WeaponKind::TailThrash, 0.0);

        // Pressing with a full meter starts the burst and spends the charge.
        assert!(tick_active(&mut w, true), "a charged thrash did not fire");
        assert!(w.active_fx().thrash_active());
        let spent = w.player.loadout.weapons[0].timer;
        assert!(
            (spent - WeaponKind::TailThrash.base_cooldown()).abs() < 1e-4,
            "the burst must cost a full {:.1}s charge, got {spent}",
            WeaponKind::TailThrash.base_cooldown()
        );
        assert_eq!(
            w.sfx.iter().filter(|s| **s == Sfx::ShootThrash).count(),
            1,
            "one thrash sfx per trigger"
        );

        // It runs its full burst and then goes idle by itself.
        let mut ticks = 0;
        while w.active_fx().thrash_active() {
            tick_active(&mut w, false);
            ticks += 1;
            assert!(ticks < 200, "the thrash burst never ended");
        }
        assert!(
            (ticks as f32 / 60.0 - THRASH_BURST).abs() < 0.05,
            "burst lasted {:.2}s of ticks",
            ticks as f32 / 60.0
        );

        // Not charged: a press is a no-op. The meter keeps filling by `dt` (and
        // only by `dt`): a press must never *reset* or spend it.
        let before = w.player.loadout.weapons[0].timer;
        assert!(before > 1.0, "the charge should still be filling");
        assert!(!tick_active(&mut w, true), "an uncharged weapon fired");
        let after = w.player.loadout.weapons[0].timer;
        assert!(
            (before - after - 1.0 / 60.0).abs() < 1e-4,
            "a press with nothing charged moved the charge {before} -> {after}"
        );

        // …and the meter does fill: 1 s of ticks takes exactly 1 s off it.
        let start = w.player.loadout.weapons[0].timer;
        for _ in 0..60 {
            tick_active(&mut w, false);
        }
        let left = w.player.loadout.weapons[0].timer;
        assert!(
            (start - 1.0 - left).abs() < 0.05,
            "charge went {start} -> {left} in one second"
        );
    }

    /// An active weapon never auto-fires, and `fire_weapons_system` leaves its
    /// charge meter alone (only the active system ticks it).
    #[test]
    fn active_weapons_do_not_auto_fire() {
        let mut w = World::new(Theme::Sewer, 22);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        let enemy = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(12.0, 0.0), 1.0);
        arm_active(&mut w, WeaponKind::TailThrash, 0.0);
        let hp_before = w.ecs.get::<Enemy>(enemy).unwrap().hp;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_auto).unwrap();
        w.end_tick();

        assert!(
            w.ecs.get::<Enemy>(enemy).unwrap().hp == hp_before,
            "the thrash fired itself"
        );
        assert!(w.bullets().is_empty(), "an active weapon spawned a bullet");
        assert_eq!(w.player.loadout.weapons[0].timer, 0.0);
        assert!(!w.active_fx().thrash_active());
    }

    /// One thrash sweep damages and knocks back everything in the reach, deletes
    /// enemy bullets inside it, and leaves bodies outside it alone.
    #[test]
    fn thrash_hurts_knocks_back_and_deletes_enemy_bullets() {
        let mut w = World::new(Theme::Sewer, 23);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        let ppos = w.player.pos;
        let near = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(20.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(200.0, 0.0), 1.0);
        let mut shot = Bullet::new(
            ppos + Vec2::new(0.0, 24.0),
            Vec2::new(0.0, 10.0),
            4.0,
            Faction::Enemy,
            (255, 120, 160),
        );
        shot.life = 2.0;
        w.spawn_bullet(shot);
        let mut outside = Bullet::new(
            ppos + Vec2::new(0.0, 130.0),
            Vec2::new(0.0, 10.0),
            4.0,
            Faction::Enemy,
            (255, 120, 160),
        );
        outside.life = 2.0;
        w.spawn_bullet(outside);
        arm_active(&mut w, WeaponKind::TailThrash, 0.0);
        let hp_before = w.ecs.get::<Enemy>(near).unwrap().hp;

        tick_active(&mut w, true);

        let near_enemy = w.ecs.get::<Enemy>(near).unwrap();
        let dealt = hp_before - near_enemy.hp;
        assert!(
            (dealt - WeaponKind::TailThrash.base_damage()).abs() < 1e-4,
            "first sweep dealt {dealt}"
        );
        assert!(
            near_enemy.vel.x > 100.0,
            "the sweep must knock the body outward, got {:?}",
            near_enemy.vel
        );
        assert_eq!(
            w.ecs.get::<Enemy>(far).unwrap().hp,
            EnemyKind::Skitterer.base_hp(),
            "the thrash reached a body outside its arc"
        );
        assert_eq!(
            w.bullets().len(),
            1,
            "only the shot inside the arc may be thrashed aside"
        );
        assert_eq!(
            w.sfx.iter().filter(|s| **s == Sfx::ShootThrash).count(),
            1
        );
        assert!(
            !w.particles.shockwaves.is_empty(),
            "a sweep must draw its arc shockwave"
        );
    }

    /// The burst sweeps three times, so a body that stays inside the arc takes
    /// three times the per-sweep damage over its full length.
    #[test]
    fn a_full_thrash_burst_sweeps_three_times() {
        let mut w = World::new(Theme::Sewer, 24);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        // A huge hp pool so the 78 damage of a L1 burst cannot kill it.
        let tank = w.spawn_enemy(EnemyKind::Brute, ppos + Vec2::new(24.0, 0.0), 40.0);
        arm_active(&mut w, WeaponKind::TailThrash, 0.0);
        let hp_before = w.ecs.get::<Enemy>(tank).unwrap().hp;

        tick_active(&mut w, true);
        while w.active_fx().thrash_active() {
            tick_active(&mut w, false);
        }

        let dealt = hp_before - w.ecs.get::<Enemy>(tank).unwrap().hp;
        // Knockback pushes the body outward, but a Brute is heavy and the reach
        // is 60 px, so it stays inside the arc for all three sweeps.
        let expect = WeaponKind::TailThrash.base_damage() * 3.0;
        assert!(
            (dealt - expect).abs() < 1e-3,
            "a full burst dealt {dealt}, expected {expect}"
        );
    }

    /// The squeak ring stuns regular enemies, slows elites, shoves outward and
    /// never hits a body twice.
    #[test]
    fn squeak_stuns_regulars_slows_elites_and_hits_each_body_once() {
        let mut w = World::new(Theme::Sewer, 25);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        // Batch 12: the squeak is a forward cone, so everything the test wants
        // hit stands in front of the rat.
        w.player.face = Vec2::new(1.0, 0.0);
        let bug = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(30.0, 0.0), 1.0);
        let elite = w.spawn_enemy(EnemyKind::Brute, ppos + Vec2::new(50.0, 24.0), 20.0);
        w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
        let far = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(0.0, 400.0), 1.0);
        arm_active(&mut w, WeaponKind::SonicSqueak, 0.0);
        let bug_hp = w.ecs.get::<Enemy>(bug).unwrap().hp;
        let elite_hp = w.ecs.get::<Enemy>(elite).unwrap().hp;

        tick_active(&mut w, true);
        // Mid-expansion the drawn ring is exposed to the renderer.
        tick_active(&mut w, false);
        let (origin, radius) = w
            .squeak_visual()
            .expect("the squeak cone must be drawn while its chevrons push out");
        assert_eq!(origin, ppos, "the cone starts at the rat");
        assert!(radius > 0.0 && radius < 90.0, "ring radius {radius}");
        // Run the ring out to its final radius.
        let mut ticks = 0;
        while w.active_fx().squeak_active() {
            tick_active(&mut w, false);
            ticks += 1;
            assert!(ticks < 100, "the squeak ring never finished");
        }

        let bug = w.ecs.get::<Enemy>(bug).unwrap();
        assert!(bug.hp < bug_hp, "the ring missed the regular enemy");
        assert!(
            (bug_hp - bug.hp - WeaponKind::SonicSqueak.base_damage()).abs() < 1e-4,
            "the ring must hit a body exactly once, dealt {}",
            bug_hp - bug.hp
        );
        assert!(
            (bug.stun - SQUEAK_STUN).abs() < 0.1,
            "a skitterer should be stunned for ~{SQUEAK_STUN}s, got {}",
            bug.stun
        );

        let elite = w.ecs.get::<Enemy>(elite).unwrap();
        assert!(elite.hp < elite_hp, "the ring missed the elite");
        assert_eq!(elite.stun, 0.0, "an elite must be slowed, not stunned");
        assert!(
            elite.slow_t > 0.0 && elite.slow_mult < 0.9,
            "an elite should be slowed, got slow_t {} mult {}",
            elite.slow_t,
            elite.slow_mult
        );

        assert_eq!(
            w.ecs.get::<Enemy>(far).unwrap().hp,
            EnemyKind::Skitterer.base_hp(),
            "the ring reached past its radius"
        );
        assert!(
            !w.particles.shockwaves.is_empty(),
            "the squeak must draw its pushing front"
        );
    }

    /// Boss rules: the thrash damages it with little knockback, the squeak
    /// damages and only briefly slows it (never stuns).
    #[test]
    fn active_weapons_damage_the_boss_but_barely_control_it() {
        let mut w = World::new(Theme::Sewer, 26);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        let at = w.player.pos + Vec2::new(24.0, 0.0);
        // The cone weapons aim with the rat's facing; point it at the boss.
        w.player.face = Vec2::new(1.0, 0.0);
        spawn_boss_at(&mut w, BossKind::Cat, at);
        let boss_hp = |w: &mut World| {
            let mut q = w.ecs.query::<&Boss>();
            q.iter(&w.ecs).next().map(|b| b.hp).expect("boss alive")
        };
        arm_active(&mut w, WeaponKind::TailThrash, 0.0);
        let hp_before = boss_hp(&mut w);

        tick_active(&mut w, true);
        let after_thrash = boss_hp(&mut w);
        assert!(
            after_thrash < hp_before,
            "the thrash sweep missed the boss"
        );

        // The squeak: damage plus a brief slow, no stun.
        arm_active(&mut w, WeaponKind::SonicSqueak, 0.0);
        w.player.loadout.weapons[0].level = 3;
        tick_active(&mut w, true);
        let mut ticks = 0;
        while w.active_fx().squeak_active() {
            tick_active(&mut w, false);
            ticks += 1;
            assert!(ticks < 100);
        }
        let mut q = w.ecs.query::<&Boss>();
        let boss = q.iter(&w.ecs).next().expect("boss alive");
        assert!(
            boss.hp < after_thrash,
            "the squeak ring missed the boss"
        );
        assert!(
            boss.slow_t > 0.0 && boss.slow_mult() < 1.0,
            "the boss should be briefly slowed (slow_t {})",
            boss.slow_t
        );
        assert!(
            boss.slow_t <= crate::game::weapon::SQUEAK_BOSS_SLOW_SECS,
            "the boss slow must be brief, got {}s",
            boss.slow_t
        );
    }

    /// The Space press is shared with the dash: a dash on cooldown still
    /// releases a charged active weapon, and the press does not spend the dash.
    #[test]
    fn space_with_the_dash_charging_still_fires_an_active_weapon() {
        let mut w = World::new(Theme::Sewer, 27);
        w.clear_enemies();
        w.clear_bullets();
        w.player.dash_cd = 1.0;
        w.player.dash_active = 0.0;
        arm_active(&mut w, WeaponKind::TailThrash, 0.0);

        // The press: `begin_tick(.., dash = true)` reaches both systems.
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(active_weapons_system).unwrap();
        w.end_tick();

        assert!(
            w.active_fx().thrash_active(),
            "a charged active must fire even with the dash on cooldown"
        );
        assert_eq!(w.player.dash_active, 0.0, "the dash was on cooldown");
        assert!(w.player.dash_cd > 0.5, "the dash cooldown was not spent");
    }

    /// Reach and charge scale with the weapon's level.
    #[test]
    fn active_reach_and_charge_scale_with_level() {
        let reach_at = |level: u8, kind: WeaponKind| -> (f32, f32) {
            let mut w = World::new(Theme::Sewer, 28);
            w.clear_enemies();
            arm_active(&mut w, kind, 0.0);
            w.player.loadout.weapons[0].level = level;
            tick_active(&mut w, true);
            let fx = w.active_fx();
            if kind == WeaponKind::TailThrash {
                (fx.thrash_reach, w.player.loadout.weapons[0].timer)
            } else {
                (fx.squeak_max_r, w.player.loadout.weapons[0].timer)
            }
        };
        for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
            let (r1, c1) = reach_at(1, kind);
            let (r5, c5) = reach_at(5, kind);
            assert!(r5 > r1, "{kind:?} reach did not grow with level");
            assert!(c5 < c1, "{kind:?} charge time did not shrink with level");
        }
        assert_eq!(reach_at(1, WeaponKind::TailThrash).0, 60.0);
        assert_eq!(reach_at(5, WeaponKind::TailThrash).0, 84.0);
        assert_eq!(reach_at(1, WeaponKind::SonicSqueak).0, 90.0);
        assert_eq!(reach_at(5, WeaponKind::SonicSqueak).0, 130.0);
    }

    /// **Sonic Squeak reads Lens Cap's ring reach as well as Long Fuse's
    /// radius.** It used to apply only the splash hook, so one of the two
    /// passives silently did nothing for the weapon it was listed against.
    #[test]
    fn sonic_squeak_ring_reads_lens_cap_and_long_fuse() {
        use crate::game::loadout::{PassiveKind, Upgrade};
        let ring_at = |passives: &[PassiveKind]| -> f32 {
            let mut w = World::new(Theme::Sewer, 29);
            w.clear_enemies();
            arm_active(&mut w, WeaponKind::SonicSqueak, 0.0);
            for &p in passives {
                w.player.loadout.apply(&Upgrade::Passive(p));
            }
            tick_active(&mut w, true);
            w.active_fx().squeak_max_r
        };
        let base = ring_at(&[]);
        let fuse = ring_at(&[PassiveKind::LongFuse]);
        let lens = ring_at(&[PassiveKind::LensCap]);
        let both = ring_at(&[PassiveKind::LongFuse, PassiveKind::LensCap]);
        assert!(fuse > base, "Long Fuse must widen the ring ({fuse} vs {base})");
        assert!(lens > base, "Lens Cap must widen the ring ({lens} vs {base})");
        assert!(
            both > fuse.max(lens),
            "both hooks multiply: {both} vs fuse {fuse} / lens {lens}"
        );
    }

    /// A recast **never erases the orbiters' Long Fuse life**: the old ring is
    /// still dropped (two rings would stack damage), but the replacement keeps
    /// any remaining life instead of resetting it to the shorter base.
    #[test]
    fn a_recast_does_not_erase_the_orbiters_long_fuse_life() {
        let mut w = World::new(Theme::Sewer, 30);
        w.clear_enemies();
        arm(&mut w, WeaponKind::SporeOrbit);
        w.player.loadout.weapons[0].timer = 0.0;
        run_weapons(&mut w, 1.0 / 60.0);
        // Pretend the live ring already carries a long residual (Long Fuse).
        {
            let mut q = w.ecs.query::<(bevy_ecs::entity::Entity, &mut Bullet)>();
            for (_, mut b) in q.iter_mut(&mut w.ecs) {
                if b.orbit.is_some() {
                    b.life = 2.5;
                }
            }
        }
        w.player.loadout.weapons[0].timer = 0.0;
        run_weapons(&mut w, 1.0 / 60.0);
        let mut q = w.ecs.query::<&Bullet>();
        let lives: Vec<f32> = q.iter(&w.ecs).filter(|b| b.orbit.is_some()).map(|b| b.life).collect();
        assert!(!lives.is_empty(), "the recast must still leave an orbit ring");
        for life in lives {
            assert!(
                life >= 2.5,
                "the recast cut the ring's remaining Long Fuse life to {life}"
            );
        }
    }

    /// Both actives charged and one press: both fire.
    #[test]
    fn one_press_spends_every_charged_active() {
        let mut w = World::new(Theme::Sewer, 29);
        w.clear_enemies();
        w.clear_bullets();
        w.player.loadout.weapons = WeaponKind::all()
            .iter()
            .filter(|k| k.is_active())
            .map(|&kind| Weapon {
                kind,
                level: 1,
                timer: 0.0,
                evolved: false,
            })
            .collect();
        assert_eq!(w.player.loadout.weapons.len(), 2);

        tick_active(&mut w, true);

        let fx = w.active_fx();
        assert!(fx.thrash_active(), "the thrash did not fire");
        assert!(fx.squeak_active(), "the squeak did not fire");
        assert!(
            w.player.loadout.weapons.iter().all(|w| w.timer > 1.0),
            "every fired weapon must pay its charge"
        );
    }

    /// A run that never owns an active weapon draws no extra rng and changes
    /// nothing, however often Space is pressed: the active pass is a pure
    /// lookup over the loadout until a weapon is actually charged.
    #[test]
    fn pressing_space_without_active_weapons_changes_nothing() {
        let run = |press: bool| -> Vec<(f32, f32, f32)> {
            let mut w = World::new(Theme::Sewer, 777);
            w.clear_enemies();
            w.clear_bullets();
            let p = w.player.pos;
            let mut rng = StdRng::seed_from_u64(5);
            for i in 0..40 {
                let a = i as f32 * 0.5;
                let pos = (p + Vec2::from_angle(a) * (20.0 + (i % 8) as f32 * 12.0))
                    .clamp_box(Vec2::splat(8.0), w.level.arena);
                w.spawn_enemy_with_rng(EnemyKind::Skitterer, pos, 3.0, &mut rng);
            }
            for _ in 0..180 {
                w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
                if press {
                    // The Space half only; the dash is not spent.
                    w.ecs.resource_mut::<TickInput>().active = true;
                }
                w.ecs.run_system_cached(active_weapons_system).unwrap();
                w.ecs.run_system_cached(fire_auto).unwrap();
                w.end_tick();
            }
            w.enemies()
                .into_iter()
                .map(|e| (e.pos.x, e.pos.y, e.hp))
                .collect()
        };
        assert_eq!(
            run(false),
            run(true),
            "pressing Space with no active weapon owned moved the sim"
        );
    }

    /// Same seed, same presses: the two active weapons land identical damage,
    /// crowd control and effect state.
    #[test]
    fn active_weapons_are_deterministic() {
        let run = || -> Vec<(f32, f32, f32)> {
            let mut w = World::new(Theme::Kitchen, 424242);
            w.clear_enemies();
            w.clear_bullets();
            let ppos = w.player.pos;
            let mut rng = StdRng::seed_from_u64(9);
            for i in 0..24 {
                let a = i as f32 * 0.7;
                let pos = (ppos + Vec2::from_angle(a) * (14.0 + i as f32 * 3.0))
                    .clamp_box(Vec2::splat(8.0), w.level.arena);
                w.spawn_enemy_with_rng(EnemyKind::Skitterer, pos, 6.0, &mut rng);
            }
            for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
                let idx = w.player.loadout.weapons.len();
                w.player.loadout.weapons.push(Weapon {
                    kind,
                    level: 1,
                    timer: 0.0,
                    evolved: false,
                });
                let _ = idx;
            }
            for tick in 0..240u32 {
                // One press at the start, then let the effects play out.
                tick_active(&mut w, tick == 0);
            }
            let mut out = Vec::new();
            for e in w.enemies() {
                out.push((e.pos.x, e.pos.y, e.hp));
            }
            out
        };
        let a = run();
        let b = run();
        assert_eq!(a.len(), b.len(), "same seed spawned different hordes");
        assert_eq!(a, b, "the active weapons desynchronised the run");
    }

    // ---- Per-tick cost (reported in the change that added line of sight) --

    /// Fill `EnemyIndex` in `SpawnId` order the way
    /// `enemy_move::grid_build_system` does, so the benchmark below measures
    /// the real cached-order path instead of the narrow-harness sort fallback.
    fn build_enemy_index(w: &mut World) {
        let mut query = w.ecs.query::<(bevy_ecs::entity::Entity, &SpawnId)>();
        let mut items: Vec<(u64, bevy_ecs::entity::Entity)> = query
            .iter(&w.ecs)
            .map(|(entity, id)| (id.0, entity))
            .collect();
        items.sort_unstable_by_key(|&(id, _)| id);
        w.ecs.resource_mut::<EnemyIndex>().0 = items.into_iter().map(|(_, e)| e).collect();
    }

    /// Worst-case framing: all six weapons ready on every tick, 600 enemies
    /// (the cap) spread in a disc around the player on a generated map, so
    /// most candidate rays cross walls. Prints µs/tick; the assertion is a
    /// loose ceiling so the number is reported rather than raced.
    #[test]
    fn fire_weapons_cost_per_tick_at_the_enemy_cap() {
        let mut w = World::new(Theme::Sewer, 5);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        let p = w.player.pos;
        let mut rng = StdRng::seed_from_u64(11);
        for i in 0..600 {
            let a = i as f32 * 0.37;
            let r = 24.0 + (i % 30) as f32 * 8.0;
            let pos = (p + Vec2::from_angle(a) * r).clamp_box(Vec2::splat(8.0), w.level.arena);
            // Huge hp so Tail Whip cannot thin the crowd during the run.
            w.spawn_enemy_with_rng(EnemyKind::Skitterer, pos, 500.0, &mut rng);
        }
        w.player.loadout.weapons = WeaponKind::all()
            .iter()
            .map(|&kind| Weapon {
                kind,
                level: 1,
                timer: 0.0,
                evolved: false,
            })
            .collect();
        build_enemy_index(&mut w);

        let ticks = 120u32;

        // In-run reference for the *pre-change* targeting cost: the plain
        // nearest pick that each of the three aiming weapons used to run once
        // per shot (no distance/line-of-sight work at all beyond the scan).
        // Measured in the same process, so a loaded machine hits both numbers.
        let t_ref = std::time::Instant::now();
        for _ in 0..ticks {
            for _ in 0..3 {
                let mut query = w.ecs.query::<(bevy_ecs::entity::Entity, &SpawnId, &Enemy)>();
                let candidates = query
                    .iter(&w.ecs)
                    .map(|(entity, id, enemy)| (entity, *id, enemy.pos, enemy.alive()));
                let _ = nearest_of(candidates, p);
            }
            // Same bookkeeping as the measured loop, so the delta below is
            // the targeting work and nothing else.
            w.clear_bullets();
            w.clear_pools();
        }
        let plain_pick = t_ref.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);

        let t0 = std::time::Instant::now();
        for _ in 0..ticks {
            for weapon in w.player.loadout.weapons.iter_mut() {
                weapon.timer = 0.0;
            }
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            w.clear_bullets();
            w.clear_pools();
        }
        let per_tick = t0.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);

        // Realistic cadence: the loadout's own cooldowns, so an aiming pick
        // costs what it costs in play (Gnaw 0.55 s, Spray 0.95 s, Acid 1.5 s).
        let realistic_ticks = 600u32;
        let t2 = std::time::Instant::now();
        for _ in 0..realistic_ticks {
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            w.clear_bullets();
            w.clear_pools();
        }
        let realistic = t2.elapsed().as_secs_f64() * 1e6 / f64::from(realistic_ticks);

        println!(
            "fire_weapons @600 enemies: plain nearest pick x3 {plain_pick:.1} us/tick, \
             worst case (all six ready) {per_tick:.1} us/tick (+{:.1}), \
             realistic cadence {realistic:.1} us/tick",
            per_tick - plain_pick
        );
        assert!(per_tick < 2000.0, "{per_tick:.1} us/tick is out of budget");
        assert!(realistic < 2000.0, "{realistic:.1} us/tick is out of budget");
    }

    /// An enemy outside the aura records no damage number.
    #[test]
    fn tail_whip_records_nothing_for_far_enemies() {
        let mut w = World::new(Theme::Sewer, 1717);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(80.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::TailWhip);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(
            w.damage_numbers.is_empty(),
            "far enemy should not pop a damage number"
        );
    }

    // ---- Targeting respects walls ------------------------------------------

    /// Setup shared by the line-of-sight tests: an open map, the rat pressed up
    /// against (6 px from) a 3-cell solid column whose near face is
    /// `x = 176`. The rat is deliberately close, so "inside the melee aura but
    /// behind the wall" is a real configuration.
    fn wall_between(w: &mut World) -> f32 {
        open_map(w);
        w.player.pos = Vec2::new(170.0, 168.0);
        w.player.face = Vec2::new(1.0, 0.0);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.sfx.clear();
        wall_column(w, 11, 10)
    }

    /// The whole point of the LOS rule: a jaw does not close through masonry.
    /// The same body at the same distance is bitten in the open and untouched
    /// once a wall stands between it and the rat.
    #[test]
    fn gnaw_will_not_bite_through_a_wall() {
        let mut w = World::new(Theme::Sewer, 501);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.player.pos = Vec2::new(170.0, 168.0);
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // Level 5's 24 px jaw reaches a cat 26 px out (the body's radius is the
        // slack), so only the wall can decide it.
        let body = w.spawn_enemy(EnemyKind::Cat, Vec2::new(p.x + 26.0, p.y), 1.0);
        arm(&mut w, WeaponKind::Gnaw);
        w.player.loadout.weapons[0].level = 5;
        let hp = |w: &World| w.ecs.get::<Enemy>(body).unwrap().hp;
        let before = hp(&w);
        run_weapons(&mut w, 1.0 / 60.0);
        assert!(hp(&w) < before, "the bite must reach a body in the open");

        // Now the same body, with a solid column between them (x 176..192).
        wall_column(&mut w, 11, 10);
        w.player.loadout.weapons[0].timer = 0.0;
        let before = hp(&w);
        w.sfx.clear();
        run_weapons(&mut w, 1.0 / 60.0);
        assert_eq!(hp(&w), before, "Gnaw bit through a wall");
        assert!(
            !w.sfx.contains(&Sfx::Shoot),
            "a blocked bite must not announce itself"
        );
    }

    /// The squeak cone is physical: a body behind masonry takes no damage and
    /// no stun, while the same body at the same distance takes both once the
    /// wall is gone.
    #[test]
    fn sonic_squeak_does_not_hit_through_a_wall() {
        let mut w = World::new(Theme::Sewer, 507);
        open_map(&mut w);
        w.clear_enemies();
        w.player.pos = Vec2::new(170.0, 168.0);
        // The cone points north; the wall is the row of cells between the
        // rat (row 10) and the body (row 8).
        w.player.face = Vec2::new(0.0, -1.0);
        let p = w.player.pos;
        let hidden = w.spawn_enemy(EnemyKind::Cat, Vec2::new(p.x, p.y - 30.0), 1.0);
        let hp = |w: &World| w.ecs.get::<Enemy>(hidden).unwrap().hp;
        let stun = |w: &World| w.ecs.get::<Enemy>(hidden).unwrap().stun;
        let cast = |w: &mut World| {
            arm_active(w, WeaponKind::SonicSqueak, 0.0);
            tick_active(w, true);
            let mut ticks = 0;
            while w.active_fx().squeak_active() {
                tick_active(w, false);
                ticks += 1;
                assert!(ticks < 100, "the squeak cone never finished");
            }
        };

        // In the open: hit and stunned.
        let before = hp(&w);
        cast(&mut w);
        assert!(hp(&w) < before, "the cone must hit a body in the open");
        assert!(stun(&w) > 0.0, "and stun it");

        // With a solid row between the rat and the body: nothing.
        for cx in 9..=12 {
            w.level
                .tiles
                .set(cx, 9, TileKind::Wall(WallKind::Solid));
        }
        w.level.tiles.rebuild_masks();
        let body = w.ecs.get::<Enemy>(hidden).unwrap();
        let (before, stun_before) = (body.hp, body.stun);
        cast(&mut w);
        let body = w.ecs.get::<Enemy>(hidden).unwrap();
        assert_eq!(body.hp, before, "the cone dealt damage through a wall");
        assert_eq!(body.stun, stun_before, "and must not stun through one");
    }

    /// "No target" behaviour is unchanged: the timer stays ready so the
    /// weapon retries instead of serving a cooldown for a dud.
    #[test]
    fn targeting_through_cover_keeps_the_weapon_ready() {
        let mut w = World::new(Theme::Sewer, 502);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 24.0, p.y), 1.0);
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(
            w.player.loadout.weapons[0].timer, 0.0,
            "a shot with no visible target must retry next tick"
        );
    }

    /// The rat shoots what it can see: a nearer enemy behind the wall loses to
    /// a farther one in the open. (Cheese Spray, whose fan still takes the
    /// nearest *visible* body as its aim; the bite and the crack use the same
    /// rule.)
    #[test]
    fn cheese_spray_aims_at_the_nearest_visible_enemy() {
        let mut w = World::new(Theme::Sewer, 503);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 20.0, p.y), 1.0);
        let visible = w.spawn_enemy(EnemyKind::Cat, Vec2::new(p.x, p.y + 80.0), 1.0);
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert!(!bullets.is_empty(), "Cheese Spray did not fire");
        // The middle wedge is the fan's aim.
        let dir = heading(&bullets[bullets.len() / 2]);
        assert!(
            dir.y > 0.99 && dir.x.abs() < 1e-3,
            "the fan aimed {dir:?}, not straight down at the visible enemy"
        );
        assert!(w.ecs.get::<Enemy>(visible).is_some());
    }

    /// The line-of-sight budget is a cap on candidates, not on enemies: when
    /// every one of the cap's nearest candidates is behind cover the weapon
    /// falls back to the facing rather than paying a query per body in the
    /// horde.
    #[test]
    fn targeting_stops_at_the_candidate_cap() {
        let mut w = World::new(Theme::Sewer, 504);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        // `TARGET_LOS_CANDIDATES` nearer, all hidden behind the same column.
        for i in 0..TARGET_LOS_CANDIDATES {
            let y = p.y - 6.0 + i as f32 * 0.5;
            w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 20.0, y), 1.0);
        }
        // One visible enemy, farther than every hidden candidate, and *behind*
        // the rat: if the cap is honoured the fan keeps the rat's facing (+x).
        w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(p.x - 60.0, p.y), 1.0);
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert!(!bullets.is_empty(), "Cheese Spray must still fire");
        let dir = heading(&bullets[bullets.len() / 2]);
        assert!(
            dir.x > 0.99,
            "the fan {dir:?} found a body past the cover the cap has to bound"
        );
    }

    /// The crack does not reach through masonry: the same body is cut in the
    /// open and untouched behind a wall, though both are inside the whip's long
    /// reach.
    #[test]
    fn tail_whip_does_not_reach_through_a_wall() {
        let mut w = World::new(Theme::Sewer, 505);
        open_map(&mut w);
        w.clear_enemies();
        w.player.pos = Vec2::new(170.0, 168.0);
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Cat, Vec2::new(p.x + 30.0, p.y), 1.0);
        arm(&mut w, WeaponKind::TailWhip);
        let hp = |w: &World| w.ecs.get::<Enemy>(body).unwrap().hp;
        let before = hp(&w);
        run_weapons(&mut w, 1.0 / 60.0);
        assert!(hp(&w) < before, "the crack must reach a body in the open");

        // A solid column between the rat and the body (x 176..192).
        wall_column(&mut w, 11, 10);
        w.player.loadout.weapons[0].timer = 0.0;
        let before = hp(&w);
        run_weapons(&mut w, 1.0 / 60.0);
        assert_eq!(hp(&w), before, "Tail Whip reached through the wall");
    }

    /// Acid Spit falls back to the facing lob (today's no-target behaviour)
    /// instead of lobbing at an unseen enemy — a lob cannot cross the wall it
    /// would land on anyway.
    #[test]
    fn acid_spit_does_not_target_through_a_wall() {
        let mut w = World::new(Theme::Sewer, 506);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 24.0, p.y), 1.0);
        arm(&mut w, WeaponKind::AcidSpit);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert_eq!(bullets.len(), 1, "Acid Spit should still fire forward");
        let target = bullets[0].lob.expect("lobbed shot").target;
        assert!(
            (target.x - (p.x + 50.0)).abs() < 1e-3 && (target.y - p.y).abs() < 1e-3,
            "lob target {target:?} should be the facing fallback, not the hidden cat"
        );
    }

    // ---- Targeting the boss (P4) -------------------------------------------

    /// Put a live boss of `kind` at `pos` (the framework's own `Boss`
    /// component, so its art hit shapes and target points come along).
    fn spawn_boss_at(w: &mut World, kind: BossKind, pos: Vec2) {
        let arena = w
            .level
            .boss_room()
            .map(|r| r.rect)
            .unwrap_or_else(|| TileRect::new(0, 0, 10, 10));
        let boss = Boss::new(kind, pos, 5000.0, arena, pos, false);
        w.ecs.spawn((boss, SpawnId(0)));
    }

    /// An open map with the rat facing away from its target and no horde: only
    /// the boss can win the aim pick.
    fn boss_target_world(seed: u64, kind: BossKind, offset: Vec2) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        // Face up, so a dart flying right can only come from the boss pick.
        w.player.face = Vec2::new(0.0, -1.0);
        let at = w.player.pos + offset;
        spawn_boss_at(&mut w, kind, at);
        w
    }

    /// Direction a bullet's velocity points in.
    fn heading(b: &Bullet) -> Vec2 {
        b.vel.normalized()
    }

    /// The boss is a body too: when the rat is pressed against it, the jaw
    /// closes on it for the bite's own damage (and spawns nothing).
    #[test]
    fn gnaw_bites_the_boss_in_its_jaws() {
        let mut w = boss_target_world(601, BossKind::Cat, Vec2::new(18.0, 0.0));
        w.player.face = Vec2::new(1.0, 0.0);
        let boss_hp = |w: &mut World| {
            let mut q = w.ecs.query::<&Boss>();
            q.iter(&w.ecs).next().map(|b| b.hp).expect("boss alive")
        };
        let before = boss_hp(&mut w);
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(w.bullets().is_empty(), "the bite must not spawn a projectile");
        assert!(
            boss_hp(&mut w) < before,
            "the jaw must close on the boss standing in it"
        );
        assert_eq!(
            w.ecs.query::<&BiteSnap>().iter(&w.ecs).count(),
            1,
            "the bite is drawn"
        );
    }

    /// A boss the rat cannot see is not bitten, though it stands inside the
    /// jaw's reach.
    #[test]
    fn gnaw_will_not_bite_a_boss_through_a_wall() {
        let mut w = World::new(Theme::Sewer, 605);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        w.player.loadout.weapons[0].timer = 0.0;
        spawn_boss_at(&mut w, BossKind::Cat, Vec2::new(face + 20.0, p.y));
        w.player.face = Vec2::new(1.0, 0.0);
        arm(&mut w, WeaponKind::Gnaw);
        w.player.loadout.weapons[0].level = 5;
        let boss_hp = |w: &mut World| {
            let mut q = w.ecs.query::<&Boss>();
            q.iter(&w.ecs).next().map(|b| b.hp).expect("boss alive")
        };
        let before = boss_hp(&mut w);

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(boss_hp(&mut w), before, "the jaw closed through a wall");
        assert_eq!(w.ecs.query::<&BiteSnap>().iter(&w.ecs).count(), 0);
    }

    /// Cheese Spray's whole cone turns to the boss.
    #[test]
    fn cheese_spray_aims_at_the_boss() {
        let mut w = boss_target_world(602, BossKind::Broom, Vec2::new(0.0, 70.0));
        let ppos = w.player.pos;
        let points: Vec<Vec2> = w.boss_target_points().expect("boss alive").iter().collect();
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert!(!bullets.is_empty(), "Cheese Spray did not fire");
        let heading_down = bullets.iter().filter(|b| heading(b).y > 0.9).count();
        assert_eq!(
            heading_down,
            bullets.len(),
            "every dart should point down at the boss, got {:?}",
            bullets.iter().map(heading).collect::<Vec<_>>()
        );
        // The cone's centre dart aims exactly at the chosen point.
        let mid = bullets.len() / 2;
        let dir = heading(&bullets[mid]);
        assert!(
            points
                .iter()
                .any(|q| ((*q - ppos).normalized() - dir).len() < 0.05),
            "the centre dart {dir:?} misses the boss points {points:?}"
        );
    }

    /// Acid Spit lobs at the boss's point instead of along the facing.
    #[test]
    fn acid_spit_lobs_at_the_boss() {
        let mut w = boss_target_world(603, BossKind::Mower, Vec2::new(-55.0, 0.0));
        let points: Vec<Vec2> = w.boss_target_points().expect("boss alive").iter().collect();
        arm(&mut w, WeaponKind::AcidSpit);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert_eq!(bullets.len(), 1, "Acid Spit did not fire");
        let target = bullets[0].lob.expect("lobbed shot").target;
        assert!(
            points.iter().any(|q| q.dist(target) < 1e-3),
            "lob target {target:?} is not one of the boss points {points:?}"
        );
    }

    /// Nearest wins: a nearer enemy screens the boss exactly as the horde does.
    #[test]
    fn the_aim_prefers_a_nearer_enemy_over_the_boss() {
        let mut w = boss_target_world(604, BossKind::Cat, Vec2::new(90.0, 30.0));
        let ppos = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(0.0, 30.0), 1.0);
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert!(!bullets.is_empty(), "Cheese Spray did not fire");
        let dir = heading(&bullets[bullets.len() / 2]);
        assert!(
            dir.y > 0.99 && dir.x.abs() < 1e-3,
            "the fan {dir:?} ignored the nearer enemy for the boss"
        );
    }

    /// A boss the rat cannot see is not a target, even in range: the fan keeps
    /// the rat's facing instead of turning to it.
    #[test]
    fn the_boss_behind_a_wall_is_not_aimed_at() {
        let mut w = World::new(Theme::Sewer, 605);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        spawn_boss_at(&mut w, BossKind::Cat, Vec2::new(face + 20.0, p.y));
        // The rat faces up; the boss is +x behind masonry.
        w.player.face = Vec2::new(0.0, -1.0);
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert!(!bullets.is_empty(), "Cheese Spray must still fire");
        // The fan keeps the facing: its middle wedge flies up, not at the boss.
        let mid = heading(&bullets[bullets.len() / 2]);
        assert!(
            mid.y < -0.9,
            "the fan centred on a boss behind a wall: {mid:?}"
        );
    }

    /// The cat mid-`Pounce` has no grounded point, so it is not targetable: the
    /// fan keeps the rat's facing instead of turning to it.
    #[test]
    fn an_airborne_cat_is_not_targeted() {
        let mut w = boss_target_world(606, BossKind::Cat, Vec2::new(60.0, 0.0));
        {
            let mut query = w.ecs.query::<&mut Boss>();
            let mut boss = query.iter_mut(&mut w.ecs).next().expect("boss alive");
            boss.set_state(
                crate::game::ecs::systems::boss::BossState::Cat(
                    crate::game::art_boss_cat::BossCatState::Pounce,
                ),
                0.70,
            );
            // progress 0.5: inside the airborne window.
            boss.elapsed = 0.35;
            assert!(boss.is_airborne());
            assert!(boss.target_points().is_empty());
        }
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert!(!bullets.is_empty(), "Cheese Spray must still fire");
        // The rat faces up; nothing may turn the fan right toward the leaping
        // cat at +x.
        for b in &bullets {
            let dir = heading(b);
            assert!(
                dir.y < -0.9,
                "the fan turned to a leaping cat it cannot hit: {dir:?}"
            );
        }
    }

    /// The pick's ordering: distance first, then an enemy before a boss point
    /// on an exact tie, then the lower boss point index. This is what keeps the
    /// aim independent of query iteration order.
    #[test]
    fn candidate_order_prefers_enemies_then_boss_points_on_a_tie() {
        let (enemy, boss_a, boss_b) = (
            Candidate {
                dist_sq: 100.0,
                rank: RANK_ENEMY,
                id: 9,
                entity: Entity::PLACEHOLDER,
                pos: Vec2::ZERO,
            },
            Candidate {
                dist_sq: 100.0,
                rank: RANK_BOSS,
                id: 1,
                entity: Entity::PLACEHOLDER,
                pos: Vec2::ZERO,
            },
            Candidate {
                dist_sq: 100.0,
                rank: RANK_BOSS,
                id: 0,
                entity: Entity::PLACEHOLDER,
                pos: Vec2::ZERO,
            },
        );
        let before = |a: &Candidate, b: &Candidate| {
            super::candidate_precedes(a.dist_sq, a.rank, a.id, b.dist_sq, b.rank, b.id)
        };
        assert!(before(&enemy, &boss_b), "an enemy must win a distance tie");
        assert!(!before(&boss_a, &boss_b));
        assert!(before(&boss_b, &boss_a), "boss points tie-break by index");
        // Distance outranks rank: a nearer boss point beats a farther enemy.
        let near_boss = Candidate {
            dist_sq: 64.0,
            ..boss_b
        };
        assert!(before(&near_boss, &enemy));
    }
    // ---- Character-level damage (`Player::damage_mult`) --------------------

    /// Damage a build deals in `ticks` to one planted, unkillable body beside
    /// the rat, with the rat's character level forced to `level`.
    fn dealt_at_level(build: impl Fn(&mut World), level: u32, ticks: usize) -> f32 {
        const POOL: f32 = 100_000.0;
        let mut w = World::new(Theme::Sewer, 77);
        w.peace = true;
        w.player.god = true;
        w.player.loadout.weapons.clear();
        build(&mut w);
        w.player.level = level;
        let at = w.player.pos + Vec2::new(14.0, 0.0);
        let e = w.spawn_enemy(EnemyKind::Brute, at, 1.0);
        {
            let mut body = w.ecs.get_mut::<Enemy>(e).unwrap();
            body.hp = POOL;
            body.max_hp = POOL;
            body.speed_scale = 0.0;
        }
        for _ in 0..ticks {
            w.update_with(1.0 / 60.0, Vec2::ZERO, false, false);
        }
        assert_eq!(w.player.level, level, "the rat must not level mid-measurement");
        POOL - w.ecs.get::<Enemy>(e).unwrap().hp
    }

    fn assert_ratio(hi: f32, lo: f32, want: f32, what: &str) {
        assert!(lo > 0.0, "{what}: the level-1 build dealt nothing");
        let got = hi / lo;
        assert!(
            (got - want).abs() < want * 0.01,
            "{what}: {hi} / {lo} = {got}, wanted {want}"
        );
    }

    #[test]
    fn a_hit_at_level_6_deals_a_fifth_more_than_at_level_1() {
        let gnaw = |w: &mut World| w.player.loadout.weapons.push(Weapon::new(WeaponKind::Gnaw));
        let lo = dealt_at_level(gnaw, 1, 240);
        let hi = dealt_at_level(gnaw, 6, 240);
        assert_ratio(hi, lo, 1.20, "Gnaw");
    }

    #[test]
    fn the_level_bonus_multiplies_with_sharp_teeth() {
        use crate::game::loadout::PassiveKind;
        let plain = |w: &mut World| w.player.loadout.weapons.push(Weapon::new(WeaponKind::Gnaw));
        let sharp = |w: &mut World| {
            w.player.loadout.weapons.push(Weapon::new(WeaponKind::Gnaw));
            assert!(w.give_passive(PassiveKind::Damage, 1));
        };
        let lo = dealt_at_level(plain, 1, 240);
        let teeth = dealt_at_level(sharp, 1, 240);
        let both = dealt_at_level(sharp, 6, 240);
        let teeth_mult = teeth / lo;
        assert!(teeth_mult > 1.05, "Sharp Teeth must add damage: {teeth_mult}");
        assert_ratio(both, lo, teeth_mult * 1.20, "Gnaw + Sharp Teeth");
    }

    #[test]
    fn pools_marks_and_evolutions_carry_the_level_bonus() {
        use crate::game::loadout::PassiveKind;
        // A pool (Acid Spit's lob lands as a damaging puddle) and a mark
        // (Hantavirus ticks): both carry the damage they were fired with.
        for kind in [WeaponKind::AcidSpit, WeaponKind::Hantavirus] {
            let build = move |w: &mut World| {
                // Hantavirus is deployed by a **dropping** 5 px behind the rat's
                // facing, so the fixture faces the drop at the planted body
                // (14 px out) and measures the level bonus, not the drop's
                // geometry.
                w.player.face = Vec2::new(-1.0, 0.0);
                w.player.loadout.weapons.push(Weapon::new(kind));
            };
            let lo = dealt_at_level(build, 1, 600);
            let hi = dealt_at_level(build, 6, 600);
            assert_ratio(hi, lo, 1.20, kind.name());
        }
        // An evolution: Ratchet Tail (Tail Whip, evolved).
        let ratchet = |w: &mut World| {
            let kind = WeaponKind::TailWhip;
            let mut weapon = Weapon::new(kind);
            weapon.level = kind.max_level();
            weapon.evolved = true;
            w.player.loadout.weapons.push(weapon);
            let partner = kind.evolution().expect("Tail Whip evolves").passive;
            assert!(w.give_passive(partner, 1));
            let _ = PassiveKind::Damage;
        };
        let lo = dealt_at_level(ratchet, 1, 600);
        let hi = dealt_at_level(ratchet, 6, 600);
        assert_ratio(hi, lo, 1.20, "Ratchet Tail");
    }

    /// Measurement, not a pin: how often Gnaw's bite actually lands in bot play
    /// on level one (Sewer, the Skilled and Casual bots, eight seeds, the first
    /// three minutes), with the kills and the first level-up beside it. The
    /// ceiling is one bite per cooldown (0.55 s at L1: 109 a minute).
    ///
    /// `cargo test gnaw_bites_landed_in_bot_play -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement"]
    fn gnaw_bites_landed_in_bot_play() {
        use crate::autoplay::{AutoOpts, AutoRun};
        use crate::game::bot::Skill;
        for skill in [Skill::Skilled, Skill::Casual] {
            let seeds = 8u64;
            let mut per_min = [0.0f32; 3];
            let (mut kills, mut first) = (0.0f32, 0.0f32);
            for seed in 1000..1000 + seeds {
                let mut run = AutoRun::new(seed, AutoOpts::practice(0, skill));
                let mut lvl = None;
                while !run.finished() && run.world().elapsed < 180.0 {
                    run.step(1.0 / 60.0);
                    let w = run.world_mut();
                    let t = w.elapsed;
                    let fresh = w
                        .ecs
                        .query::<&BiteSnap>()
                        .iter(&w.ecs)
                        .filter(|b| b.t >= b.total)
                        .count();
                    per_min[((t / 60.0) as usize).min(2)] += fresh as f32;
                    if lvl.is_none() && w.player.level > 1 {
                        lvl = Some(t);
                    }
                }
                kills += run.world().kills as f32;
                first += lvl.unwrap_or(180.0);
            }
            let n = seeds as f32;
            println!(
                "{skill:?}: bites/min {:.0} / {:.0} / {:.0} (minutes 1-3), kills by 3:00 {:.0}, first level-up {:.1}s",
                per_min[0] / n,
                per_min[1] / n,
                per_min[2] / n,
                kills / n,
                first / n
            );
        }
    }

}
