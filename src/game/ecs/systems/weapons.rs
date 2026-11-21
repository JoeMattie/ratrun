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
//! - `fire_one` — all six weapons (Gnaw, Cheese Spray, Squeak Nova, Spore
//!   Orbit, Tail Whip, Acid Spit), including Tail Whip's direct `Enemy::hurt`
//!   and Spore Orbit's "carry the angle, drop the old ring" rule.
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
//! | Gnaw | aimed at a visible foe; the dart stops on the first wall | the point of the change: no more hitting through cover |
//! | Cheese Spray | aimed at a visible foe; darts stop on walls; spread off-axis darts clip the corridor walls | point-blank it still delivers the whole cone |
//! | Squeak Nova | the ring stops on walls (no pass-through) | a radial burst that ignores walls makes corridors meaningless; down a corridor its axial shots are the ones that can reach you |
//! | Spore Orbit | **passes through walls** (projectiles exempt from the sweep); damage still needs a line from the rat | a 24 px orbit cannot exist in a 16 px half-width corridor, so destroy-on-contact would silently empty the weapon there — and its damage is point-blank anyway |
//! | Tail Whip | melee arc: an enemy takes it only with a clear line from the rat | a whip may not reach through a maze partition |
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
use crate::game::bullet::{Bullet, Faction, Lob, Orbit};
// Wave 3: Counterweight's melee nudge keeps its body on legal floor.
use crate::game::collision::SizeClass;
use crate::game::ecs::systems::weapons_wave3b::MELEE_NUDGE_COOLDOWN;
use crate::game::weapon::{
    SQUEAK_BOSS_SLOW, SQUEAK_BOSS_SLOW_SECS, SQUEAK_ELITE_SLOW, SQUEAK_EXPAND, SQUEAK_SHOVE,
    SQUEAK_STUN, THRASH_BOSS_KNOCKBACK, THRASH_BURST, THRASH_KNOCKBACK, THRASH_SHAKE,
    THRASH_SWEEP_RATE, THRASH_SWEEPS,
};
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::resources::{
    ActiveFx, DamageLedger, EnemyIndex, MeleeNudge, NextSpawnId, RngRes, SfxQueue, Shake, TickInput,
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
    bosses: Query<&Boss>,
    bullets: Query<(Entity, &SpawnId, &Bullet)>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    // Wave 3: Counterweight's "once per target per 0.4 s" melee-nudge log.
    mut melee_nudge: ResMut<MeleeNudge>,
    mut scratch: FireScratch,
) {
    let dt = input.dt;
    // Legacy: snapshot the player state and the two multipliers.
    let ppos = player.pos;
    let pface = player.face;
    let fire_rate = player.loadout.stats.fire_rate_mult;
    let dmg_mult = player.loadout.stats.damage_mult;
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
        let (kind, dmg, cd, projs) = {
            let w = &player.loadout.weapons[i];
            (
                w.kind,
                w.damage() * dmg_mult,
                (w.cooldown() / fire_rate).max(0.05),
                w.level_projectiles(),
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
            &stats,
            &mut melee_nudge,
        );
        if fired {
            player.loadout.weapons[i].timer += cd;
        } else {
            // Nothing spawned (e.g. Gnaw with no live target): retry next
            // tick instead of serving a full cooldown for a dud shot.
            player.loadout.weapons[i].timer = 0.0;
        }
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

/// Line-of-sight radius for the melee aura (a whip is not a projectile).
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
    }
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
pub struct FireScratch<'s> {
    fallback_order: Local<'s, Vec<Entity>>,
    orbit_scratch: Local<'s, Vec<Entity>>,
    target_scratch: Local<'s, Vec<Candidate>>,
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
/// 2. **Live effects.** A Tail Thrash burst sweeps three times over
///    `THRASH_BURST` seconds: each sweep hurts every enemy in the arc, deletes
///    enemy bullets inside it, knocks survivors outward and re-damages the boss.
///    A Sonic Squeak ring expands over `SQUEAK_EXPAND` seconds and hits every
///    enemy once as its front passes (the `prev..now` band), stunning regulars,
///    slowing elites and briefly slowing the boss.
///
/// Determinism: the enemy walk uses the same stable order the other weapons use
/// and every per-enemy rng draw happens inside [`vfx`] calls that already draw
/// in `SpawnId` order, so a replay of the same inputs lands the same hits in the
/// same order. Nothing here draws rng unless a weapon is actually spent.
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
    mut fallback: Local<Vec<Entity>>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let pface = player.face;
    let fire_rate = player.loadout.stats.fire_rate_mult.max(0.1);
    let dmg_mult = player.loadout.stats.damage_mult;
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
    if input.active {
        for i in 0..player.loadout.weapons.len() {
            let (kind, charged) = {
                let w = &player.loadout.weapons[i];
                (w.kind, w.kind.is_active() && w.timer <= 0.0)
            };
            if !charged {
                continue;
            }
            let (charge, dmg, reach) = {
                let w = &player.loadout.weapons[i];
                (
                    (w.cooldown() / fire_rate).max(0.05),
                    w.damage() * dmg_mult,
                    // Wave 3 passive hooks: the thrash is a melee arc (Lens
                    // Cap's melee reach), the squeak is a radial ring (Long
                    // Fuse's splash radius).
                    match w.kind {
                        WeaponKind::SonicSqueak => stats.splash_radius(w.active_reach()),
                        _ => stats.melee_reach(w.active_reach()),
                    },
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
                    shake.amount = shake.amount.max(THRASH_SHAKE);
                    vfx::emit_thrash_start(&mut particles, &mut rng.0, ppos);
                    sfx.0.push(Sfx::ShootThrash);
                }
                WeaponKind::SonicSqueak => {
                    *fx = ActiveFx {
                        squeak_left: SQUEAK_EXPAND,
                        squeak_prev_r: 0.0,
                        squeak_r: 0.0,
                        squeak_max_r: reach,
                        squeak_dmg: dmg,
                        squeak_pos: ppos,
                        ..*fx
                    };
                    shake.amount = shake.amount.max(1.6);
                    vfx::emit_sonic_squeak(&mut particles, &mut rng.0, ppos, reach, SQUEAK_EXPAND);
                    sfx.0.push(Sfx::SonicSqueak);
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
                &level,
                &mut rng,
                &mut particles,
                order,
                &mut enemies,
                &mut bosses,
                &enemy_bullets,
                &mut damage_numbers,
                &mut ledger,
                &fx,
                stats.shove_mult,
            );
        }
        fx.thrash_left = (fx.thrash_left - dt).max(0.0);
        if fx.thrash_left <= 0.0 {
            fx.thrash_total = 0.0;
            fx.thrash_age = 0.0;
            fx.thrash_done = 0;
        }
    }

    // 2b. The squeak ring: expand it and hit everything its front passes.
    if fx.squeak_left > 0.0 {
        fx.squeak_left = (fx.squeak_left - dt).max(0.0);
        let prev = fx.squeak_r;
        let step = fx.squeak_max_r / SQUEAK_EXPAND * dt.min(SQUEAK_EXPAND);
        let now = (prev + step).min(fx.squeak_max_r);
        fx.squeak_prev_r = prev;
        fx.squeak_r = now;
        squeak_ring(
            &level,
            &mut rng,
            &mut particles,
            order,
            &mut enemies,
            &mut bosses,
            &mut damage_numbers,
            &mut ledger,
            &fx,
            stats.shove_mult,
        );
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
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    enemy_bullets: &Query<(Entity, &SpawnId, &Bullet)>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    fx: &ActiveFx,
    // Counterweight: a weapon's native shove (see `Stats::shove`).
    shove_mult: f32,
) {
    let kind = WeaponKind::TailThrash;
    let reach = fx.thrash_reach;
    let dmg = fx.thrash_dmg;
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
        e.hurt(dmg);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let out = (pos - ppos).normalized();
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        let heavy = matches!(e.kind, EnemyKind::Brute | EnemyKind::Boss);
        let knock = if heavy { THRASH_BOSS_KNOCKBACK } else { 1.0 };
        e.vel = out * THRASH_KNOCKBACK * knock * shove_mult;
        vfx::emit_thrash_hit(particles, &mut rng.0, pos, out, killed);
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            dmg,
            killed,
            DamageSource::TailWhip,
        );
        hits += 1;
    }

    // The arc also thrashes aside every enemy shot inside its reach.
    for (entity, _, b) in enemy_bullets.iter() {
        if b.faction != Faction::Enemy {
            continue;
        }
        if b.pos.dist(ppos) <= reach + b.radius {
            commands.entity(entity).despawn();
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

/// Resolve one tick of the squeak ring: everything the front passed.
#[allow(clippy::too_many_arguments)]
fn squeak_ring(
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    fx: &ActiveFx,
    // Counterweight: the ring's native outward shove.
    shove_mult: f32,
) {
    let kind = WeaponKind::SonicSqueak;
    let origin = fx.squeak_pos;
    let prev = fx.squeak_prev_r;
    let now = fx.squeak_r;
    let dmg = fx.squeak_dmg;

    for &entity in order {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        // Hit once, as the ring's front passes the body's near edge.
        let edge = e.pos.dist(origin) - e.radius;
        if edge > now || edge <= prev {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        e.hurt(dmg);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let out = (pos - origin).normalized();
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        // Elites keep their footing (slowed, not stunned); everything else is
        // pinned for the full stun. Wall cover blocks the ring exactly as it
        // blocks the weapons: the arc of a squeak is physical.
        if los_clear(level, origin, pos, MELEE_LOS_RADIUS) {
            if e.elite || matches!(e.kind, EnemyKind::Brute) {
                e.slow_for(SQUEAK_ELITE_SLOW, SQUEAK_STUN);
            } else {
                e.stun_for(SQUEAK_STUN);
            }
        }
        e.vel += out * SQUEAK_SHOVE * shove_mult;
        vfx::emit_squeak_hit(particles, &mut rng.0, pos, out, killed);
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            dmg,
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
        if shapes.iter().next().is_none() {
            continue;
        }
        let edge = boss.pos.dist(origin) - boss.kind.body_radius();
        if edge > now || edge <= prev {
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
        ledger.add_boss(kind, amount);
        let killed = boss.hp <= 0.0;
        vfx::emit_squeak_hit(particles, &mut rng.0, boss.pos, boss.pos - origin, killed);
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

/// Legacy `World::fire_one`, with the neon-branch VFX/damage-number hooks.
/// Returns whether a shot was produced (Gnaw with no live target fires
/// nothing).
#[allow(clippy::too_many_arguments)]
fn fire_one(
    commands: &mut Commands,
    kind: WeaponKind,
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
    // Wave 3 passive hooks (Lens Cap's reach, Long Fuse's zone life and splash
    // radius, Counterweight's shove) and the per-body melee-nudge log it needs.
    stats: &crate::game::loadout::Stats,
    melee_nudge: &mut MeleeNudge,
) -> bool {
    let color = kind.color();
    let mut fired = false;
    match kind {
        WeaponKind::Gnaw => {
            if let Some(epos) =
                nearest_visible(level, kind, order, enemies, boss_points, ppos, target_scratch)
            {
                let dir = (epos - ppos).normalized();
                // One dart, two from level 4 (`Weapon::level_projectiles` is
                // the level for Gnaw): a fan of ±3° keeps the pair from
                // stacking into one body at close range while staying
                // effectively single-target at range.
                let shots = 1 + projs / 4;
                for k in 0..shots {
                    let angle = if shots > 1 {
                        let spread = (k as f32 - (shots - 1) as f32 / 2.0) * 0.052;
                        dir.angle() + spread
                    } else {
                        dir.angle()
                    };
                    let v = Vec2::from_angle(angle) * 120.0;
                    let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                    b.size = 1;
                    // Lens Cap: a ranged weapon's reach is its projectile life.
                    b.life = stats.ranged_reach(b.life);
                    b.weapon = Some(kind);
                    commands.spawn((b, next_id.take()));
                }
                vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, dir, 0.0);
                fired = true;
            }
        }
        WeaponKind::CheeseSpray => {
            let aim = nearest_visible(level, kind, order, enemies, boss_points, ppos, target_scratch)
                .map(|p| (p - ppos).normalized())
                .unwrap_or(pface);
            let base = aim.angle();
            for k in 0..projs {
                let spread = (k as f32 - (projs - 1) as f32 / 2.0) * 0.18;
                let v = Vec2::from_angle(base + spread) * 110.0;
                let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                // Lens Cap.
                b.life = stats.ranged_reach(1.4);
                b.weapon = Some(kind);
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, aim, 0.0);
            fired = true;
        }
        WeaponKind::SqueakNova => {
            for k in 0..projs {
                let a = k as f32 / projs as f32 * std::f32::consts::TAU;
                let v = Vec2::from_angle(a) * 95.0;
                let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                // Lens Cap (the nova's ring is made of these darts).
                b.life = stats.ranged_reach(1.3);
                b.weapon = Some(kind);
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, 0.0);
            fired = true;
        }
        WeaponKind::SporeOrbit => {
            // Lens Cap widens the ring, Long Fuse keeps it alive longer.
            let radius = stats.melee_reach(26.0);
            // Carry the ring's current angle forward, then drop the old ring
            // before spawning its replacement so two never stack. One pass:
            // remember the lowest-`SpawnId` orbit angle (legacy `find_map`
            // over stable `SpawnId` order) and collect the old ring for
            // deferred despawn without sorting the bullet query.
            let mut carry: Option<(u64, f32)> = None;
            orbit_scratch.clear();
            for (entity, id, b) in bullets.iter() {
                if let Some(o) = b.orbit {
                    orbit_scratch.push(entity);
                    if carry.is_none_or(|(best, _)| id.0 < best) {
                        carry = Some((id.0, o.angle));
                    }
                }
            }
            let base = carry.map(|(_, angle)| angle).unwrap_or(0.0);
            for &entity in orbit_scratch.iter() {
                commands.entity(entity).despawn();
            }
            for k in 0..projs {
                let angle = base + k as f32 / projs as f32 * std::f32::consts::TAU;
                let mut b = Bullet::new(ppos, Vec2::ZERO, dmg, Faction::Player, color);
                // Long Fuse: an orbiter is a lingering friendly zone.
                b.life = stats.zone_life(cd);
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
            fired = true;
        }
        WeaponKind::TailWhip => {
            // Lens Cap's melee reach.
            let radius = stats.melee_reach(26.0 + projs as f32 * 3.0);
            for &entity in order {
                let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                    continue;
                };
                // Melee arc: the whip does not reach through a wall. Enemies
                // inside the aura are a handful, so the per-enemy line test is
                // bounded by the aura radius, not by the horde.
                if e.alive()
                    && e.pos.dist(ppos) <= radius + e.radius
                    && los_clear(level, ppos, e.pos, MELEE_LOS_RADIUS)
                {
                    let hp_before = e.hp.max(0.0);
                    e.hurt(dmg);
                    let killed = !e.alive();
                    // Balance instrumentation only (see `DamageLedger`).
                    ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
                    let pos = e.pos;
                    let enemy_radius = e.radius;
                    let enemy_id = e.id;
                    // Counterweight: +2 px of outward shove per stack on a
                    // melee hit that has no native knockback (the whip), once
                    // per body per 0.4 s. `resolve_circle` keeps the nudge on
                    // legal floor instead of pushing a body into a wall.
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
                    }
                    vfx::emit_tail_whip_hit(particles, &mut rng.0, pos, pos - ppos, killed);
                    damage_numbers.record(
                        enemy_id,
                        pos,
                        enemy_radius,
                        dmg,
                        killed,
                        DamageSource::TailWhip,
                    );
                }
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, radius);
            fired = true;
        }
        WeaponKind::AcidSpit => {
            // Lob a projectile: it does no damage in flight and only becomes a
            // pool once it lands (target, wall, or timeout). Lens Cap stretches
            // the lob's reach the same way it stretches a dart's life.
            let target = nearest_visible_within(
                level,
                kind,
                stats.ranged_reach(target_range(kind)),
                order,
                enemies,
                boss_points,
                ppos,
                target_scratch,
            )
            .unwrap_or(ppos + pface * 50.0)
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
            commands.spawn((b, next_id.take()));
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
    }
    // Only announce a shot when the weapon actually produced something
    // (Gnaw with no target in range fires nothing).
    if fired {
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
        };
        sfx.0.push(sound);
    }
    fired
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::{
        Candidate, RANK_BOSS, RANK_ENEMY, TARGET_LOS_CANDIDATES, fire_weapons_system, nearest_of,
    };
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::ecs::systems::boss::{Boss, BossKind};
    use crate::game::bullet::Bullet;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::components::{Entity, SpawnId};
    use crate::game::ecs::resources::EnemyIndex;
    use crate::game::enemy::{Enemy, EnemyKind};
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
    const PROJECTILE_WEAPONS: [WeaponKind; 5] = [
        WeaponKind::Gnaw,
        WeaponKind::CheeseSpray,
        WeaponKind::SqueakNova,
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
        }];
    }

    fn run_weapons(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
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

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(w.bullets().len(), 1, "should pick exactly the live enemy");
        let v = w.bullets()[0].vel;
        assert!(
            (v.x - 120.0).abs() < 1e-3 && v.y.abs() < 1e-3,
            "aimed at {:?} instead of the live enemy",
            v
        );
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

    /// Tail Whip damages in place (through `Enemy::hurt`) only inside the aura.
    #[test]
    fn tail_whip_hurts_enemies_inside_the_radius_only() {
        let mut w = World::new(Theme::Sewer, 5);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        let ppos = w.player.pos;
        let near = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(10.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(80.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::TailWhip);

        run_weapons(&mut w, 1.0 / 60.0);

        let near_hp = w.ecs.get::<Enemy>(near).unwrap().hp;
        let far_hp = w.ecs.get::<Enemy>(far).unwrap().hp;
        assert!(near_hp < EnemyKind::Cat.base_hp(), "near enemy unhurt");
        assert_eq!(far_hp, EnemyKind::Cat.base_hp(), "far enemy hurt");
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::ShootWhip).count(), 1);
        assert!(w.bullets().is_empty(), "Tail Whip must not spawn bullets");
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
        let bug = w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(30.0, 0.0), 1.0);
        let elite = w.spawn_enemy(EnemyKind::Brute, ppos + Vec2::new(-40.0, 0.0), 20.0);
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
            .expect("the squeak ring must be drawn while it expands");
        assert_eq!(origin, ppos, "the ring is centred on the rat");
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
            "the squeak must draw its expanding ring"
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

    /// The whole point of the change: an enemy the rat cannot see is not a
    /// target, so the dart does not fire through the wall at it.
    #[test]
    fn gnaw_ignores_an_enemy_behind_a_wall() {
        let mut w = World::new(Theme::Sewer, 501);
        let face = wall_between(&mut w);
        let hidden = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 24.0, hidden.y), 1.0);
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(
            w.bullets().is_empty(),
            "Gnaw fired at an enemy through a wall"
        );
        assert!(
            !w.sfx.contains(&Sfx::Shoot),
            "a blocked shot must not announce itself"
        );
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
    /// a farther one in the open.
    #[test]
    fn gnaw_prefers_the_nearest_visible_enemy() {
        let mut w = World::new(Theme::Sewer, 503);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 20.0, p.y), 1.0);
        let visible = w.spawn_enemy(EnemyKind::Cat, Vec2::new(p.x, p.y + 80.0), 1.0);
        let _ = visible;
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert_eq!(bullets.len(), 1, "Gnaw should have fired at the visible cat");
        let v = bullets[0].vel;
        assert!(
            v.y > 100.0 && v.x.abs() < 1e-3,
            "dart aimed at {v:?}, not straight down at the visible enemy"
        );
        assert!(w.ecs.get::<Enemy>(visible).is_some());
    }

    /// The line-of-sight budget is a cap on candidates, not on enemies: when
    /// every one of the cap's nearest candidates is behind cover the weapon
    /// reports no target even though a visible one exists further out.
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
        // One visible enemy, farther than every hidden candidate.
        w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(p.x, p.y - 60.0), 1.0);
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(
            w.bullets().is_empty(),
            "the cap must bound the line queries: {} hidden candidates ate the budget",
            TARGET_LOS_CANDIDATES
        );
    }

    /// A melee arc does not reach through a wall, even when the body is inside
    /// the aura radius.
    #[test]
    fn tail_whip_does_not_reach_through_a_wall() {
        let mut w = World::new(Theme::Sewer, 505);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        // Level 1 aura is 26 + 3 = 29 px; this cat is 26 px away, 4 px past
        // the wall's far face, so only the wall decides it.
        let behind = w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 20.0, p.y), 1.0);
        let front = w.spawn_enemy(EnemyKind::Cat, Vec2::new(p.x - 12.0, p.y), 1.0);
        arm(&mut w, WeaponKind::TailWhip);

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(
            w.ecs.get::<Enemy>(behind).unwrap().hp,
            EnemyKind::Cat.base_hp(),
            "Tail Whip reached through the wall"
        );
        assert!(
            w.ecs.get::<Enemy>(front).unwrap().hp < EnemyKind::Cat.base_hp(),
            "Tail Whip missed the cat on the same side of the wall"
        );
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

    /// The grounded boss points are candidates under the same rules as enemies,
    /// so Gnav picks the nearest one when nothing else is around.
    #[test]
    fn gnaw_aims_at_the_boss_when_it_is_the_nearest_body() {
        let mut w = boss_target_world(601, BossKind::Cat, Vec2::new(60.0, 0.0));
        let ppos = w.player.pos;
        let points: Vec<Vec2> = w.boss_target_points().expect("boss alive").iter().collect();
        assert!(!points.is_empty(), "the cat must expose grounded points");
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert_eq!(bullets.len(), 1, "Gnaw did not fire at the boss");
        let dir = heading(&bullets[0]);
        assert!(
            dir.x > 0.9,
            "dart aimed {dir:?}: it should have turned right to the boss"
        );
        assert!(
            points
                .iter()
                .any(|q| ((*q - ppos).normalized() - dir).len() < 1e-3),
            "dart {dir:?} does not point at any boss target point {points:?}"
        );
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
    fn gnaw_prefers_a_nearer_enemy_over_the_boss() {
        let mut w = boss_target_world(604, BossKind::Cat, Vec2::new(90.0, 30.0));
        let ppos = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(0.0, 30.0), 1.0);
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        let bullets = w.bullets();
        assert_eq!(bullets.len(), 1, "Gnaw did not fire");
        let dir = heading(&bullets[0]);
        assert!(
            dir.y > 0.99 && dir.x.abs() < 1e-3,
            "dart {dir:?} ignored the nearer enemy for the boss"
        );
    }

    /// A boss the rat cannot see is not a target, even in range.
    #[test]
    fn the_boss_behind_a_wall_is_not_targeted() {
        let mut w = World::new(Theme::Sewer, 605);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        spawn_boss_at(&mut w, BossKind::Cat, Vec2::new(face + 20.0, p.y));
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(
            w.bullets().is_empty(),
            "Gnaw fired at the boss through a wall"
        );
    }

    /// The cat mid-`Pounce` has no grounded point, so it is not targetable and
    /// the dart does not fire at empty air.
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
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(
            w.bullets().is_empty(),
            "Gnaw fired at a leaping cat it cannot hit"
        );
        assert_eq!(
            w.player.loadout.weapons[0].timer, 0.0,
            "a shot with no target must retry next tick"
        );
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
}
