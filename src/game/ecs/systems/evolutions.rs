//! Evolved-weapon behaviour (`WAVE3_DESIGN.md` §3).
//!
//! A weapon's evolution is a flag on the weapon slot (`Weapon::evolved`); it
//! keeps the weapon's kind, timing family and damage-ledger column and changes
//! **what the attack does**. `weapons.rs` branches on the flag at the point
//! each weapon fires; the parts that outlive the firing tick live here.
//!
//! | evolution | where |
//! |---|---|
//! | Milk Teeth (Gnaw) | spawned here; [`milk_teeth_system`] flies, lodges and returns |
//! | Fondue Fork (Cheese Spray) | [`CheeseString`] spawned in `fire_one`, cut in [`fondue_string_system`] |
//! | Echo Chamber (Squeak Nova) | [`EchoSpoke`] marker: `projectiles::bullets_system` reflects the spoke |
//! | Fairy Ring (Spore Orbit) | [`FairyRing`] left on every second cast, ticked here |
//! | Ratchet Tail (Tail Whip) | resolved immediately in `fire_one` (a swing, no state) |
//! | Drain Cleaner (Acid Spit) | [`CorrosiveChannel`] built where the lob lands, ticked here |
//! | Flywheel (Tail Thrash) | caught/flung inside `active_weapons_system` (`ActiveFx`) |
//! | Roll Call (Sonic Squeak) | tags set in `squeak_ring`, answered in [`roll_call_system`] |
//!
//! Rules the whole file keeps:
//! - **Damage credits the original weapon** in `DamageLedger` (Flywheel books
//!   `TailThrash`, Roll Call books `SonicSqueak`, the channel books `AcidSpit`,
//!   …) — an evolution is a behaviour upgrade, not a new ledger column.
//! - **Fixed-size histories, no per-tick allocation.** Every "each body once"
//!   list is a `[Option<u64>; CAP]` on the entity; every walk over enemies uses
//!   the cached `EnemyIndex` (with the same fallback sort the other systems
//!   use) so the order is the legacy `SpawnId` order.
//! - **An unevolved run draws no extra rng**: these systems iterate empty
//!   queries and emit nothing, and `open_chest` makes no draw at all.
//! - **Walls.** Only what §3 says stops does: the Milk Teeth dart plants on the
//!   first wall of its outbound flight (its return thread is pulled through
//!   cover — "they always come back in"), the Drain Cleaner channel stops at
//!   masonry, the Fondue string is anchored between two points that were
//!   already wall-clamped, and the Fairy Ring is a circle in the air around a
//!   point that was on open floor.
//! - **Heavy bodies.** Only the Ratchet Tail names them ("heavy targets take
//!   the whip hit only"), so the hook and the swing skip elites, Brutes and
//!   boss bodies. The other evolutions hit whatever their geometry covers.
//!
//! ## The four rows that arrive with their weapons
//!
//! These weapons are not in this branch yet, so their rows are stubs rather
//! than code. Each one needs exactly the hook named beside it; the row text is
//! the contract from `WAVE3_DESIGN.md` §3, copied here so the follow-up worker
//! does not have to go looking.
//!
//! - **Downspout** — `Acid Rain + Long Fuse`. "Replace separate splashes with a
//!   marching gutter. Strike the first marked target, then step each remaining
//!   drop 10 px toward the rat along the same line. Leave 1 s of burning
//!   runoff." Hook: Acid Rain's drop list (`systems::weapons::fire_one`'s Acid
//!   Rain arm once it lands) — replace the per-drop splash with one channel
//!   entity reused from [`CorrosiveChannel`] (a 10 px step per drop, 1 s of
//!   runoff). Nothing in this file has to change; add the arm.
//! - **Dead Letter** — `Hantavirus + Second Scent`. "A host dying with jumps
//!   left leaves a sealed packet for 2 s. The first eligible foe within 10 px
//!   opens it and resumes the chain. One packet per chain." Hook: Hantavirus's
//!   chain state (a `HantaChain` resource) — on host death spawn a packet
//!   component; a small system here ticks its 2 s fuse and hands the remaining
//!   jumps to the opener.
//! - **Carbon Copy** — `Laser Eyes + Lens Cap`. "Record the facing and origin
//!   of a completed sweep. On the next sweep a fixed afterimage at that spot
//!   repeats it backwards for half damage. Keep one copy." Hook: Laser Eyes'
//!   sweep state — store one `(origin, facing, pulses)` in `ActiveFx`
//!   (fixed, one slot) and spawn the mirrored sweep at half `damage`.
//! - **Ripcord** — `Super Dash + Spring Heel`. "After the dash, its actual
//!   route stays taut for 0.6 s, then snaps. Enemies on it take one dash hit
//!   and slide 8 px toward the departure point. Bosses take damage only." Hook:
//!   `systems::player` records the dash segment; a component holding it plus a
//!   0.6 s fuse belongs here (distance-to-segment test, one hit per body, plus
//!   an 8 px slide toward the start).

// Systems mirror the legacy signature style (one resource per argument); the
// file is the evolution module's whole schedule section.
#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::game::bullet::Faction;
use crate::game::ecs::components::{
    CheeseString, CorrosiveChannel, FairyRing, MilkPhase, MilkTooth, SpawnId,
};
use crate::game::ecs::resources::{
    ActiveFx, DamageLedger, EnemyIndex, RngRes, TickInput, FLYWHEEL_CAP, ROLL_CALL_TAG_CAP,
};
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;

// ---- Tuning (design §3) --------------------------------------------------

/// Seconds a Milk Teeth dart hangs where it bit before pulling home.
pub const MILK_LODGE_SECS: f32 = 1.0;
/// Outbound speed (matches the Gnaw dart's 120 px/s family).
pub const MILK_SPEED: f32 = 120.0;
/// Speed of the return thread (a little faster than the throw).
pub const MILK_RETURN_SPEED: f32 = 170.0;
/// How far the dart flies before it lodges in the air and comes home.
pub const MILK_REACH: f32 = 74.0;
/// Collision radius of the dart (the Gnaw dart's body).
pub const MILK_RADIUS: f32 = 2.0;
/// A return thread disappears this close to the rat.
pub const MILK_HOME_RADIUS: f32 = 5.0;

/// A Fondue Fork string lasts this long.
pub const STRING_SECS: f32 = 1.5;
/// The string cuts bodies it overlaps, as a 1 px filament.
pub const STRING_RADIUS: f32 = 1.0;
/// The tines plant on a hit or at this distance.
pub const TINE_REACH: f32 = 40.0;
/// Radius of the ring the Fondue tines are planted at (matches the spray's
/// spread geometry only for the visual sag).
pub const STRING_SAG: f32 = 1.5;

/// A Fairy Ring stays fixed for this long.
pub const FAIRY_SECS: f32 = 3.0;
/// The ring's band is the drawn circle plus this much slack.
pub const FAIRY_BAND: f32 = 2.0;
/// One hit per target per second while crossing the ring.
pub const FAIRY_HIT_COOLDOWN: f32 = 1.0;
/// Orbit radius of the fixed ring (the Spore Orbit radius).
pub const FAIRY_RADIUS: f32 = 26.0;

/// Seconds the Drain Cleaner's splash takes to run the whole stripe.
pub const CHANNEL_ADVANCE: f32 = 0.6;
/// Seconds the settled stripe keeps ticking after that.
pub const CHANNEL_SETTLE: f32 = 2.0;
/// Total stripe length and half width ("a 36 × 6 px corrosive channel").
pub const CHANNEL_LEN: f32 = 36.0;
pub const CHANNEL_HALF_W: f32 = 3.0;

/// Seconds between the Roll Call ring's tag and the answers.
pub const ROLL_CALL_DELAY: f32 = 0.8;
/// Radius of one answer pulse.
pub const ROLL_CALL_RADIUS: f32 = 9.0;
/// Answers that may hit the same body.
pub const ROLL_CALL_MAX_ANSWERS: u8 = 3;

/// Hostile pellets one Flywheel sweep catches.
pub const FLYWHEEL_PER_SWEEP: u8 = 2;
/// Speed a flung pellet leaves the tail at.
pub const FLYWHEEL_FLING_SPEED: f32 = 150.0;

// ---- Stable enemy order ---------------------------------------------------

/// The tick's `SpawnId`-ordered enemy list (the `EnemyIndex` fast path, exactly
/// like `weapons.rs` / `combat.rs`), with the narrow-harness fallback sort
/// filling `scratch` when the grid never ran.
#[cfg(debug_assertions)]
fn enemy_order<'a>(
    index: &'a EnemyIndex,
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    scratch: &'a mut Vec<Entity>,
) -> &'a [Entity] {
    if index.0.len() == enemies.iter().len() {
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        sorted_order(enemies, scratch)
    }
}

/// [`enemy_order`] without the debug guard (release build, where the guard's
/// body is compiled out anyway).
#[cfg(not(debug_assertions))]
fn enemy_order<'a>(
    index: &'a EnemyIndex,
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    scratch: &'a mut Vec<Entity>,
) -> &'a [Entity] {
    if index.0.len() == enemies.iter().len() {
        &index.0
    } else {
        sorted_order(enemies, scratch)
    }
}

fn sorted_order<'a>(
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    scratch: &'a mut Vec<Entity>,
) -> &'a [Entity] {
    scratch.clear();
    scratch.extend(enemies.iter().map(|(entity, _, _)| entity));
    scratch.sort_by_key(|&entity| {
        enemies
            .get(entity)
            .map(|(_, id, _)| id.0)
            .unwrap_or(u64::MAX)
    });
    scratch.as_slice()
}

/// Damage one body, book the weapon's ledger column and pop a damage number.
/// Returns `true` when the hit killed it.
#[allow(clippy::too_many_arguments)]
fn hit_body(
    enemy: &mut Enemy,
    damage: f32,
    kind: WeaponKind,
    ledger: &mut DamageLedger,
    damage_numbers: &mut DamageNumbers,
    source: DamageSource,
) -> bool {
    let hp_before = enemy.hp.max(0.0);
    enemy.hurt(damage);
    let killed = !enemy.alive();
    ledger.add(kind, (hp_before - enemy.hp.max(0.0)).max(0.0));
    damage_numbers.record(enemy.id, enemy.pos, enemy.radius, damage, killed, source);
    killed
}

/// If `from -> to` crosses a wall, the truncated end point; else `to`. Also
/// reports whether it stopped.
fn wall_or(level: &Level, from: Vec2, to: Vec2, radius: f32) -> (Vec2, bool) {
    match level.tiles.segment_hit(from, to, radius) {
        Some(t) => (from + (to - from) * t, true),
        None => (to, false),
    }
}

// ---- Milk Teeth -----------------------------------------------------------

/// Fly a **Milk Teeth** dart, lodge it, then pull it home biting whatever it
/// crosses on the way.
pub fn milk_teeth_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    player: Res<Player>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut darts: Query<(Entity, &SpawnId, &mut MilkTooth)>,
    mut fallback: Local<Vec<Entity>>,
    mut dart_order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let order = enemy_order(&index, &enemies, &mut fallback);
    let kind = WeaponKind::Gnaw;

    // Darts in `SpawnId` order (spawn order), so two darts never decide
    // anything by query order.
    dart_order.clear();
    dart_order.extend(darts.iter().map(|(entity, id, _)| (id.0, entity)));
    dart_order.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in dart_order.iter() {
        let Ok((_, _, mut dart)) = darts.get_mut(entity) else {
            continue;
        };
        match dart.phase {
            MilkPhase::Out => {
                dart.age += dt;
                let from = dart.pos;
                let to = from + dart.vel * dt;
                let (end, stopped) = wall_or(&level, from, to, MILK_RADIUS);
                // First body the release crosses (legacy order, first match).
                let mut bit: Option<(usize, f32)> = None;
                for (slot, &enemy) in order.iter().enumerate() {
                    let Ok((_, _, e)) = enemies.get(enemy) else {
                        continue;
                    };
                    if !e.alive() {
                        continue;
                    }
                    if crate::game::collision::segment_circle_hit(from, end, MILK_RADIUS, e.pos, e.radius)
                    {
                        bit = Some((slot, 0.0));
                        break;
                    }
                }
                if let Some((slot, _)) = bit {
                    let enemy = order[slot];
                    if let Ok((_, _, mut e)) = enemies.get_mut(enemy) {
                        if e.alive() {
                            let pos = e.pos;
                            let killed = hit_body(
                                &mut e,
                                dart.damage,
                                kind,
                                &mut ledger,
                                &mut damage_numbers,
                                DamageSource::Normal,
                            );
                            vfx::emit_milk_teeth_bite(&mut particles, &mut rng.0, pos, killed);
                            dart.pos = pos;
                        }
                    }
                    dart.phase = MilkPhase::Lodged;
                    dart.age = 0.0;
                    dart.vel = Vec2::ZERO;
                } else if stopped || dart.age * MILK_SPEED >= MILK_REACH {
                    dart.pos = end;
                    dart.phase = MilkPhase::Lodged;
                    dart.age = 0.0;
                    dart.vel = Vec2::ZERO;
                } else {
                    dart.pos = end;
                }
                vfx::emit_milk_teeth_dart(&mut particles, &mut rng.0, dart.pos, dart.phase, false);
            }
            MilkPhase::Lodged => {
                dart.age += dt;
                vfx::emit_milk_teeth_dart(&mut particles, &mut rng.0, dart.pos, dart.phase, false);
                if dart.age >= MILK_LODGE_SECS {
                    dart.phase = MilkPhase::Back;
                    dart.age = 0.0;
                    vfx::emit_milk_teeth_release(&mut particles, &mut rng.0, dart.pos);
                }
            }
            MilkPhase::Back => {
                dart.age += dt;
                let to_rat = ppos - dart.pos;
                let step_len = to_rat.len();
                let dir = if step_len > 0.001 {
                    to_rat * (1.0 / step_len)
                } else {
                    Vec2::new(1.0, 0.0)
                };
                let step = MILK_RETURN_SPEED * dt;
                let from = dart.pos;
                let to = if step_len <= step + MILK_HOME_RADIUS {
                    ppos
                } else {
                    from + dir * step
                };
                // Bite every body the thread crosses, once each.
                for &enemy in order.iter() {
                    let Ok((_, id, mut e)) = enemies.get_mut(enemy) else {
                        continue;
                    };
                    if !e.alive() || dart.has_bitten(id.0) {
                        continue;
                    }
                    if crate::game::collision::segment_circle_hit(from, to, MILK_RADIUS, e.pos, e.radius)
                    {
                        let pos = e.pos;
                        let killed = hit_body(
                            &mut e,
                            dart.damage,
                            kind,
                            &mut ledger,
                            &mut damage_numbers,
                            DamageSource::Normal,
                        );
                        dart.remember_bite(id.0);
                        vfx::emit_milk_teeth_bite(&mut particles, &mut rng.0, pos, killed);
                    }
                }
                dart.pos = to;
                vfx::emit_milk_teeth_dart(&mut particles, &mut rng.0, dart.pos, dart.phase, true);
                if dart.pos.dist(ppos) <= MILK_HOME_RADIUS {
                    commands.entity(entity).despawn();
                }
            }
        }
    }
}

// ---- Fondue Fork ----------------------------------------------------------

/// Ticks every **Fondue Fork** string: it sags, cuts each crosser once, dies.
pub fn fondue_string_system(
    mut commands: Commands,
    input: Res<TickInput>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut strings: Query<(Entity, &SpawnId, &mut CheeseString)>,
    mut fallback: Local<Vec<Entity>>,
    mut string_order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let order = enemy_order(&index, &enemies, &mut fallback);
    let kind = WeaponKind::CheeseSpray;

    string_order.clear();
    string_order.extend(strings.iter().map(|(entity, id, _)| (id.0, entity)));
    string_order.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in string_order.iter() {
        let Ok((_, _, mut line)) = strings.get_mut(entity) else {
            continue;
        };
        line.life -= dt;
        // Cut: one hit per body, ever (the tines and the filament share it).
        for &enemy in order.iter() {
            let Ok((_, id, mut e)) = enemies.get_mut(enemy) else {
                continue;
            };
            if !e.alive() || line.has_cut(id.0) {
                continue;
            }
            if crate::game::collision::segment_circle_hit(
                line.a,
                line.b,
                STRING_RADIUS,
                e.pos,
                e.radius,
            ) {
                let pos = e.pos;
                let killed = hit_body(
                    &mut e,
                    line.damage,
                    kind,
                    &mut ledger,
                    &mut damage_numbers,
                    DamageSource::Normal,
                );
                line.remember_cut(id.0);
                vfx::emit_fondue_cut(&mut particles, &mut rng.0, pos, killed);
            }
        }
        let life_frac = (line.life / line.total.max(1e-4)).clamp(0.0, 1.0);
        vfx::emit_fondue_string(&mut particles, &mut rng.0, line.a, line.b, life_frac, dt);
        if line.life <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

// ---- Fairy Ring -----------------------------------------------------------

/// Tick every **Fairy Ring**: cooldowns, one hit per body per second, expiry.
pub fn fairy_ring_system(
    mut commands: Commands,
    input: Res<TickInput>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut rings: Query<(Entity, &SpawnId, &mut FairyRing)>,
    mut fallback: Local<Vec<Entity>>,
    mut ring_order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let order = enemy_order(&index, &enemies, &mut fallback);
    let kind = WeaponKind::SporeOrbit;

    ring_order.clear();
    ring_order.extend(rings.iter().map(|(entity, id, _)| (id.0, entity)));
    ring_order.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in ring_order.iter() {
        let Ok((_, _, mut ring)) = rings.get_mut(entity) else {
            continue;
        };
        ring.life -= dt;
        ring.spin = (ring.spin + dt * 0.8) % std::f32::consts::TAU;
        ring.tick_cooldowns(dt);
        // Crossing the ring: the body's edge overlaps the drawn circle.
        for &enemy in order.iter() {
            let Ok((_, id, mut e)) = enemies.get_mut(enemy) else {
                continue;
            };
            if !e.alive() || ring.cooldown(id.0) > 0.0 {
                continue;
            }
            let edge = (e.pos.dist(ring.pos) - ring.radius).abs();
            if edge <= e.radius + FAIRY_BAND {
                let pos = e.pos;
                let killed = hit_body(
                    &mut e,
                    ring.damage,
                    kind,
                    &mut ledger,
                    &mut damage_numbers,
                    DamageSource::Orbit,
                );
                ring.cool(id.0, FAIRY_HIT_COOLDOWN);
                vfx::emit_fairy_ring_hit(&mut particles, &mut rng.0, pos, killed);
            }
        }
        let life_frac = (ring.life / ring.total.max(1e-4)).clamp(0.0, 1.0);
        vfx::emit_fairy_ring(
            &mut particles,
            &mut rng.0,
            ring.pos,
            ring.radius,
            ring.satellites,
            ring.spin,
            life_frac,
            dt,
        );
        if ring.life <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

// ---- Drain Cleaner --------------------------------------------------------

/// Truncate a channel at the first wall along its direction ("stop at
/// masonry"). Called where the splash lands (`projectiles::bullets_system`).
pub fn channel_length(level: &Level, start: Vec2, dir: Vec2) -> f32 {
    let end = start + dir * CHANNEL_LEN;
    match level.tiles.segment_hit(start, end, CHANNEL_HALF_W) {
        Some(t) => (CHANNEL_LEN * t).max(4.0),
        None => CHANNEL_LEN,
    }
}

/// Is `pos` inside the stripe open after `open_len` px?
fn in_stripe(start: Vec2, dir: Vec2, open_len: f32, pos: Vec2, radius: f32) -> bool {
    let rel = pos - start;
    let along = rel.dot(dir);
    let perp = (rel - dir * along).len();
    along >= -radius && along <= open_len + radius && perp <= CHANNEL_HALF_W + radius
}

/// Tick every **Drain Cleaner** channel: the advancing front, then the pool.
pub fn drain_cleaner_system(
    mut commands: Commands,
    input: Res<TickInput>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut channels: Query<(Entity, &SpawnId, &mut CorrosiveChannel)>,
    mut fallback: Local<Vec<Entity>>,
    mut channel_order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let order = enemy_order(&index, &enemies, &mut fallback);
    let kind = WeaponKind::AcidSpit;

    channel_order.clear();
    channel_order.extend(channels.iter().map(|(entity, id, _)| (id.0, entity)));
    channel_order.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in channel_order.iter() {
        let Ok((_, _, mut ch)) = channels.get_mut(entity) else {
            continue;
        };
        ch.age += dt;
        if ch.age <= CHANNEL_ADVANCE {
            // The splash is still running: everything the open stripe covers
            // takes one front hit, once per body.
            let open_len = ch.length * (ch.age / CHANNEL_ADVANCE).clamp(0.0, 1.0);
            for &enemy in order.iter() {
                let Ok((_, id, mut e)) = enemies.get_mut(enemy) else {
                    continue;
                };
                if !e.alive() || ch.has_hit(id.0) {
                    continue;
                }
                if in_stripe(ch.start, ch.dir, open_len, e.pos, e.radius) {
                    let pos = e.pos;
                    let killed = hit_body(
                        &mut e,
                        ch.front_damage,
                        kind,
                        &mut ledger,
                        &mut damage_numbers,
                        DamageSource::Acid,
                    );
                    ch.remember_hit(id.0);
                    vfx::emit_drain_hit(&mut particles, &mut rng.0, pos, killed);
                }
            }
        } else {
            // Settled stripe: pool ticks follow it.
            let tick = ch.dps * dt;
            for &enemy in order.iter() {
                let Ok((_, _, mut e)) = enemies.get_mut(enemy) else {
                    continue;
                };
                if !e.alive() {
                    continue;
                }
                if in_stripe(ch.start, ch.dir, ch.length, e.pos, e.radius) {
                    let hp_before = e.hp.max(0.0);
                    e.hp -= tick;
                    let killed = !e.alive();
                    ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
                    e.flash = e.flash.max(0.05);
                    damage_numbers.record(
                        e.id,
                        e.pos,
                        e.radius,
                        tick,
                        killed,
                        DamageSource::Acid,
                    );
                }
            }
        }
        let front = ch.length * (ch.age / CHANNEL_ADVANCE).clamp(0.0, 1.0);
        vfx::emit_drain_channel(
            &mut particles,
            &mut rng.0,
            ch.start,
            ch.dir,
            front,
            ch.age,
            dt,
        );
        if ch.age >= CHANNEL_ADVANCE + CHANNEL_SETTLE {
            commands.entity(entity).despawn();
        }
    }
}

// ---- Roll Call ------------------------------------------------------------

/// Answer a **Roll Call** squeak: each tagged body pulses once, three answers
/// maximum into the same victim.
pub fn roll_call_system(
    input: Res<TickInput>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut evo: ResMut<ActiveFx>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut fallback: Local<Vec<Entity>>,
) {
    if evo.answer_left <= 0.0 {
        return;
    }
    evo.answer_left -= input.dt;
    if evo.answer_left > 0.0 {
        return;
    }
    let order = enemy_order(&index, &enemies, &mut fallback);
    let damage = evo.answer_damage;
    // How many answers have already hit each victim (a fixed tally: at most
    // `ROLL_CALL_TAG_CAP` distinct victims, 3 answers each).
    let mut answers: [Option<(u64, u8)>; ROLL_CALL_TAG_CAP] = [None; ROLL_CALL_TAG_CAP];
    let tag_n = evo.tag_n as usize;
    for slot in 0..tag_n {
        let Some((Some(entity), Some(id))) = Some((evo.tags[slot], evo.tag_ids[slot])) else {
            continue;
        };
        let Ok((_, _, source)) = enemies.get_mut(entity) else {
            continue;
        };
        if !source.alive() {
            continue;
        }
        let origin = source.pos;
        vfx::emit_roll_call_answer(&mut particles, &mut rng.0, origin);
        for &enemy in order.iter() {
            let Ok((_, victim_id, mut victim)) = enemies.get_mut(enemy) else {
                continue;
            };
            if !victim.alive() || victim.pos.dist(origin) > ROLL_CALL_RADIUS + victim.radius {
                continue;
            }
            // The cap counts answers per body across the whole ring.
            let mut count = 0u8;
            let mut at: Option<usize> = None;
            for (i, entry) in answers.iter().enumerate() {
                match entry {
                    Some((held, n)) if *held == victim_id.0 => {
                        count = *n;
                        at = Some(i);
                        break;
                    }
                    None => {
                        at = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            if count >= ROLL_CALL_MAX_ANSWERS {
                continue;
            }
            let pos = victim.pos;
            let killed = hit_body(
                &mut victim,
                damage,
                WeaponKind::SonicSqueak,
                &mut ledger,
                &mut damage_numbers,
                DamageSource::Normal,
            );
            if let Some(i) = at {
                answers[i] = Some((victim_id.0, count + 1));
            }
            vfx::emit_roll_call_pulse(&mut particles, &mut rng.0, pos, killed);
            let _ = id;
        }
    }
    evo.clear_roll_call();
}

// ---- Flywheel -------------------------------------------------------------

/// Fling the pellets a finished **Flywheel** burst caught.
///
/// Queued as a command by `weapons::thrash_sweep` on the burst's final sweep:
/// applying it spawns ordinary player bullets (so walls, hits, damage numbers
/// and the ledger need no new code) and hands out the `SpawnId`s itself.
pub struct FlingPellets {
    pub pos: Vec2,
    pub forward: Vec2,
    pub damage: f32,
    pub count: u8,
}

impl FlingPellets {
    pub fn spawn(self, ecs: &mut bevy_ecs::world::World) -> u32 {
        let face = if self.forward.len_sq() > 0.001 {
            self.forward.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let count = self.count.min(FLYWHEEL_CAP as u8);
        let mut fired = 0;
        for i in 0..count as usize {
            // Fan the returned pellets a few degrees apart so they read as a
            // fan and cannot stack into one body at point-blank.
            let spread = (i as f32 - (count as f32 - 1.0) * 0.5) * 0.14;
            let dir = Vec2::from_angle(face.angle() + spread);
            let mut b = crate::game::bullet::Bullet::new(
                self.pos,
                dir * FLYWHEEL_FLING_SPEED,
                self.damage,
                Faction::Player,
                WeaponKind::TailThrash.color(),
            );
            b.size = 2;
            b.weapon = Some(WeaponKind::TailThrash);
            crate::game::ecs::resources::spawn_with_id(ecs, b);
            fired += 1;
        }
        fired
    }
}

impl bevy_ecs::system::Command for FlingPellets {
    type Out = ();

    fn apply(self, ecs: &mut bevy_ecs::world::World) {
        self.spawn(ecs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::bullet::Bullet;
    use crate::game::collision::TileKind;
    use crate::game::ecs::components::{
        EchoSpoke, CHANNEL_HIT_CAP, FAIRY_TRACK_CAP, MILK_BITE_CAP, STRING_CUT_CAP,
    };
    use crate::game::ecs::resources::spawn_with_id;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::Theme;
    use crate::game::loadout::Loadout;
    use crate::game::weapon::Weapon;
    use crate::game::world::World;
    use bevy_ecs::prelude::Entity;

    const DT: f32 = 1.0 / 60.0;

    /// Strip the generated layout's walls so a test measures the evolution, not
    /// the map.
    fn open_map(w: &mut World) {
        let cols = w.level.tiles.cols();
        let rows = w.level.tiles.rows();
        for cy in 0..rows {
            for cx in 0..cols {
                w.level.tiles.set(cx, cy, TileKind::Open);
            }
        }
        w.level.tiles.rebuild_masks();
        w.level.objects.one_way_doors.clear();
    }

    /// A world holding one max-level, already-evolved weapon and no enemies.
    fn evolved_world(kind: WeaponKind) -> World {
        let mut w = World::new(Theme::Sewer, 7);
        open_map(&mut w);
        w.clear_enemies();
        w.player.pos = w.level.arena * 0.5;
        let mut weapon = Weapon::new(kind);
        weapon.level = kind.max_level();
        weapon.evolved = true;
        w.player.loadout = Loadout::from_parts(vec![weapon], Vec::new());
        w
    }

    fn hp_of(w: &World, entity: Entity) -> f32 {
        w.ecs.get::<Enemy>(entity).map(|e| e.hp).unwrap_or(-1.0)
    }

    /// One tick of a single system, through the documented
    /// `begin_tick` → `run_system_cached` → `end_tick` harness.
    fn run<M>(w: &mut World, system: impl bevy_ecs::system::IntoSystem<(), (), M> + 'static) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(system).expect("evolution system");
        w.end_tick();
    }

    /// Milk Teeth: the dart bites on the way out, lodges for a second, then the
    /// return thread bites a body that was never on the outbound line.
    #[test]
    fn milk_teeth_lodges_and_returns_biting_on_the_way_back() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::Gnaw);
        let origin = w.player.pos;
        let front = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(24.0, 0.0), 40.0);
        let front_hp = hp_of(&w, front);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;

        // Fire (the Gnaw arm spawns a MilkTooth dart).
        run(&mut w, weapons::fire_weapons_system);
        assert_eq!(
            w.ecs.query::<&MilkTooth>().iter(&w.ecs).count(),
            1,
            "the evolved Gnaw fires one dart entity"
        );
        // Outbound: the body in front takes the bite.
        for _ in 0..20 {
            run(&mut w, milk_teeth_system);
        }
        assert!(hp_of(&w, front) < front_hp, "the outbound dart bites the first body");

        // A body that was never on the outbound line walks in while the dart is
        // lodged, and the rat moves: the thread comes back to the rat's
        // *current* position and bites it on the way.
        let behind = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(12.0, 10.0), 40.0);
        let behind_hp = hp_of(&w, behind);
        w.player.pos = origin + Vec2::new(0.0, 20.0);
        for _ in 0..110 {
            run(&mut w, milk_teeth_system);
        }
        assert!(
            hp_of(&w, behind) < behind_hp,
            "the return thread bites what it crosses: {:.1} -> {:.1}",
            behind_hp,
            hp_of(&w, behind)
        );
        assert_eq!(
            w.ecs.query::<&MilkTooth>().iter(&w.ecs).count(),
            0,
            "the thread comes home and disappears"
        );
    }

    /// Fondue Fork: the string cuts each body that crosses it exactly once, and
    /// expires after 1.5 s.
    #[test]
    fn fondue_string_cuts_each_crosser_once() {
        let mut w = evolved_world(WeaponKind::CheeseSpray);
        let origin = w.player.pos;
        let damage = w.player.loadout.weapons[0].damage();
        let line = spawn_with_id(
            &mut w.ecs,
            CheeseString::new(
                origin + Vec2::new(-14.0, 0.0),
                origin + Vec2::new(14.0, 0.0),
                STRING_SECS,
                damage,
            ),
        );
        let body = w.spawn_enemy(EnemyKind::Skitterer, origin + Vec2::new(0.0, 3.0), 40.0);
        let hp = hp_of(&w, body);
        run(&mut w, fondue_string_system);
        let cut = hp_of(&w, body);
        assert!(cut < hp, "standing on the string cuts once");
        for _ in 0..30 {
            run(&mut w, fondue_string_system);
        }
        assert_eq!(hp_of(&w, body), cut, "a second tick is free");
        for _ in 0..120 {
            run(&mut w, fondue_string_system);
        }
        assert!(w.ecs.get::<CheeseString>(line).is_none(), "the string expires");
    }

    /// The Fondue Fork's tines really are planted by the firing arm: the two
    /// outer darts' angles give a string with both ends within the tine reach
    /// of the rat and one on each side of the aim.
    #[test]
    fn cheese_spray_plants_a_string_between_its_outer_darts() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::CheeseSpray);
        let origin = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(30.0, 0.0), 400.0);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);
        let mut q = w.ecs.query::<&CheeseString>();
        let line = q.iter(&w.ecs).next().expect("a string is planted").clone();
        for end in [line.a, line.b] {
            let d = end.dist(origin);
            assert!(d <= TINE_REACH + 0.5, "a tine planted past its reach: {d}");
        }
        // The two ends straddle the aim (east), so the string crosses it.
        assert!(
            (line.a - origin).y * (line.b - origin).y <= 0.0,
            "the string must span the aim: {:?} / {:?}",
            line.a,
            line.b
        );
        let _ = body;
    }

    /// Echo Chamber: a nova spoke that hits a wall reflects once and comes back
    /// down its own line, damaging on the return leg.
    #[test]
    fn echo_chamber_spoke_reflects_once() {
        use crate::game::ecs::systems::{combat, projectiles};
        let mut w = evolved_world(WeaponKind::SqueakNova);
        let origin = w.player.pos;
        let cy = (origin.y / 16.0).floor() as i32;
        let wall_x = ((origin.x + 40.0) / 16.0).floor() as i32;
        for dy in -3..=3 {
            w.level.tiles.set(wall_x, cy + dy, TileKind::Wall(crate::game::art_walls::WallKind::Solid));
        }
        w.level.tiles.rebuild_masks();
        // West of the rat: the outbound spoke never touches it, only the
        // reflected one on its way back past the rat can.
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(-28.0, 0.0), 400.0);
        let hp = hp_of(&w, body);
        // One evolved spoke east, 95 px/s (the Squeak Nova speed).
        let mut b = Bullet::new(
            origin,
            Vec2::new(95.0, 0.0),
            6.0,
            Faction::Player,
            WeaponKind::SqueakNova.color(),
        );
        b.life = 1.8;
        b.weapon = Some(WeaponKind::SqueakNova);
        let spoke = spawn_with_id(&mut w.ecs, (b, EchoSpoke::default()));

        let mut reflected = false;
        for _ in 0..70 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::grid_build_system)
                .unwrap();
            w.ecs.run_system_cached(projectiles::bullets_system).unwrap();
            w.ecs.run_system_cached(combat::player_bullet_hits_system).unwrap();
            w.end_tick();
            if let Some(s) = w.ecs.get::<EchoSpoke>(spoke) {
                reflected |= s.reflected;
            }
        }
        assert!(reflected, "the spoke must reflect off the wall");
        assert!(hp_of(&w, body) < hp, "the return leg must damage a body");
    }

    /// Fairy Ring: crossing the fixed ring burns once per target per second.
    #[test]
    fn fairy_ring_hits_once_per_second() {
        let mut w = evolved_world(WeaponKind::SporeOrbit);
        let origin = w.player.pos;
        let damage = w.player.loadout.weapons[0].damage();
        spawn_with_id(
            &mut w.ecs,
            FairyRing::new(origin, FAIRY_RADIUS, FAIRY_SECS, 5, damage),
        );
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(FAIRY_RADIUS, 0.0), 400.0);
        let hp = hp_of(&w, body);
        run(&mut w, fairy_ring_system);
        let burnt = hp_of(&w, body);
        assert!(burnt < hp, "crossing the ring burns");
        for _ in 0..30 {
            run(&mut w, fairy_ring_system);
        }
        assert_eq!(hp_of(&w, body), burnt, "one hit per target per second");
        for _ in 0..40 {
            run(&mut w, fairy_ring_system);
        }
        assert!(hp_of(&w, body) < burnt, "the cooldown lifts after a second");
    }

    /// Ratchet Tail: the whip hooks the nearest ordinary body, swings it a
    /// quarter turn into what the arc crosses, and an elite only takes the whip.
    #[test]
    fn ratchet_tail_hooks_and_swings_but_only_whips_heavies() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::TailWhip);
        let origin = w.player.pos;
        let hooked = w.spawn_enemy(EnemyKind::Skitterer, origin + Vec2::new(20.0, 0.0), 400.0);
        let swept = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(0.0, 20.0), 400.0);
        let elite = w.spawn_enemy(EnemyKind::Skitterer, origin + Vec2::new(-18.0, 0.0), 400.0);
        w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
        let swept_hp = hp_of(&w, swept);
        let hooked_hp = hp_of(&w, hooked);
        let elite_hp = hp_of(&w, elite);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);

        assert!(hp_of(&w, swept) < swept_hp, "the swing hits what it crosses");
        assert!(hp_of(&w, hooked) < hooked_hp, "the whip hit lands on the hook");
        assert!(hp_of(&w, elite) < elite_hp, "the elite takes the whip hit");
        let moved = w.ecs.get::<Enemy>(hooked).unwrap().pos;
        assert!(
            (moved - (origin + Vec2::new(0.0, 20.0))).len() < 4.0,
            "the hooked body ends a quarter turn away: {moved:?}"
        );
        let elite_moved = w.ecs.get::<Enemy>(elite).unwrap().pos;
        assert!(
            (elite_moved - (origin + Vec2::new(-18.0, 0.0))).len() < 0.5,
            "a heavy body is never hooked: {elite_moved:?}"
        );
    }

    /// Drain Cleaner: the channel runs forward, ticks the stripe and stops at
    /// masonry.
    #[test]
    fn drain_cleaner_channel_runs_forward_and_stops_at_masonry() {
        let mut w = evolved_world(WeaponKind::AcidSpit);
        let origin = w.player.pos;
        let dir = Vec2::new(1.0, 0.0);
        let length = channel_length(&w.level, origin, dir);
        assert!((length - CHANNEL_LEN).abs() < 1e-3, "open floor: {length}");
        let damage = w.player.loadout.weapons[0].damage();
        let channel = spawn_with_id(
            &mut w.ecs,
            CorrosiveChannel::new(origin, dir, length, damage * 3.3, damage),
        );
        let inside = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(20.0, 0.0), 400.0);
        let outside = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(70.0, 0.0), 400.0);
        let in_hp = hp_of(&w, inside);
        let out_hp = hp_of(&w, outside);
        for _ in 0..20 {
            run(&mut w, drain_cleaner_system);
        }
        assert!(hp_of(&w, inside) < in_hp, "the stripe must burn");
        assert_eq!(hp_of(&w, outside), out_hp, "past the end stays dry");

        // Masonry stops it.
        let cy = (origin.y / 16.0).floor() as i32;
        let wall_x = ((origin.x + 16.0) / 16.0).floor() as i32;
        for dy in -3..=3 {
            w.level.tiles.set(wall_x, cy + dy, TileKind::Wall(crate::game::art_walls::WallKind::Solid));
        }
        w.level.tiles.rebuild_masks();
        let short = channel_length(&w.level, origin, dir);
        assert!((4.0..CHANNEL_LEN).contains(&short), "stops at masonry: {short}");
        // The settled stripe expires.
        for _ in 0..200 {
            run(&mut w, drain_cleaner_system);
        }
        assert!(w.ecs.get::<CorrosiveChannel>(channel).is_none(), "the stripe dries out");
    }

    /// The evolved Acid Spit's lob really lands as a channel (not a pool) and
    /// the channel really burns: the two systems meet at `bullets_system`.
    #[test]
    fn evolved_acid_spit_lands_as_a_corrosive_channel() {
        use crate::game::ecs::systems::{projectiles, weapons};
        let mut w = evolved_world(WeaponKind::AcidSpit);
        let origin = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(40.0, 0.0), 400.0);
        let hp = hp_of(&w, body);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);
        let mut landed = false;
        for _ in 0..80 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(projectiles::bullets_system)
                .unwrap();
            w.end_tick();
            if w.ecs.query::<&CorrosiveChannel>().iter(&w.ecs).count() > 0 {
                landed = true;
                break;
            }
        }
        assert!(landed, "the evolved splash must plant a channel");
        assert_eq!(
            w.ecs.query::<&crate::game::ecs::components::Pool>().iter(&w.ecs).count(),
            0,
            "the evolved splash must not also drop a round pool"
        );
        for _ in 0..30 {
            run(&mut w, drain_cleaner_system);
        }
        assert!(hp_of(&w, body) < hp, "the channel must burn the body in it");
    }

    /// Flywheel: a sweep catches up to two pellets and the final sweep flings
    /// what the burst caught forward as player bullets.
    #[test]
    fn flywheel_catches_pellets_and_flings_them_on_the_last_sweep() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::TailThrash);
        let origin = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        // Two hostile pellets inside the arc: one sweep catches both.
        for i in 0..2 {
            let b = Bullet::new(
                origin + Vec2::new(0.0, 6.0 + i as f32 * 4.0),
                Vec2::new(-40.0, 0.0),
                5.0,
                Faction::Enemy,
                (255, 120, 90),
            );
            spawn_with_id(&mut w.ecs, b);
        }
        // A charged Space press starts the burst.
        w.begin_tick(DT, Vec2::ZERO, true);
        w.ecs
            .run_system_cached(weapons::active_weapons_system)
            .unwrap();
        w.end_tick();
        assert_eq!(
            w.ecs.query::<&Bullet>().iter(&w.ecs).filter(|b| b.faction == Faction::Enemy).count(),
            0,
            "the arc thrashes the pellets aside"
        );
        let mut flung = 0;
        for _ in 0..90 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(weapons::active_weapons_system)
                .unwrap();
            w.end_tick();
            let player_bullets = w
                .ecs
                .query::<&Bullet>()
                .iter(&w.ecs)
                .filter(|b| b.faction == Faction::Player && b.weapon == Some(WeaponKind::TailThrash))
                .count();
            flung = flung.max(player_bullets);
        }
        assert!(flung > 0, "the final sweep must fling the caught pellets");
        let fx = w.active_fx();
        assert_eq!(fx.caught, 0, "the burst empties its catch on the fling");
    }

    /// Roll Call: a tagged body answers 0.8 s later with a 9 px pulse.
    #[test]
    fn roll_call_answers_with_a_pulse() {
        let mut w = evolved_world(WeaponKind::SonicSqueak);
        let origin = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(60.0, 0.0), 400.0);
        let near = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(64.0, 0.0), 400.0);
        let near_hp = hp_of(&w, near);
        {
            let mut fx = w.ecs.resource_mut::<ActiveFx>();
            fx.clear_roll_call();
            assert!(fx.tag(body, 42));
            fx.answer_left = ROLL_CALL_DELAY;
            fx.answer_damage = 4.0;
        }
        run(&mut w, roll_call_system);
        let mid = hp_of(&w, near);
        assert_eq!(mid, near_hp, "nothing answers before 0.8 s");
        for _ in 0..50 {
            run(&mut w, roll_call_system);
        }
        assert!(hp_of(&w, near) < mid, "the answer must land on the packing body");
        assert_eq!(w.active_fx().tag_n, 0, "the tags are spent");
    }

    /// Twin-run determinism with each evolution active: two worlds built from
    /// the same seed and the same evolved weapon must agree tick for tick on a
    /// full state fold (positions, hp, spawn order, particles, xp, kills).
    ///
    /// The loadout carries **no passive**, which is also the campaign
    /// carry-over case: an evolution works without its partner.
    #[test]
    fn every_evolution_runs_identically_twice() {
        for kind in WeaponKind::all() {
            let a = evolution_trace(kind);
            let b = evolution_trace(kind);
            assert_eq!(a, b, "{kind:?} evolved: two identical runs diverged");
            assert_ne!(a, 0, "{kind:?} evolved: the trace folded nothing");
        }
    }

    /// Fold one 10 s run of a single evolved weapon into a hash.
    fn evolution_trace(kind: WeaponKind) -> u64 {
        let mut w = evolved_world(kind);
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for tick in 0..600u32 {
            let dir = match (tick / 40) % 4 {
                0 => Vec2::new(1.0, 0.0),
                1 => Vec2::new(0.0, 1.0),
                2 => Vec2::new(-1.0, 0.0),
                _ => Vec2::new(0.0, -1.0),
            };
            let dash = tick % 90 == 0;
            w.update(DT, dir, dash);
            if tick % 10 == 0 {
                for v in [
                    w.elapsed.to_bits() as u64,
                    w.kills as u64,
                    w.score as u64,
                    w.player.pos.x.to_bits() as u64,
                    w.player.pos.y.to_bits() as u64,
                    w.player.hp.to_bits() as u64,
                    w.player.level as u64,
                    w.particles.len() as u64,
                    w.damage_numbers.len() as u64,
                ] {
                    h ^= v;
                    h = h.wrapping_mul(0x100_0000_01b3);
                }
                for e in w.enemies() {
                    for v in [
                        e.pos.x.to_bits() as u64,
                        e.pos.y.to_bits() as u64,
                        e.hp.to_bits() as u64,
                        e.id as u64,
                    ] {
                        h ^= v;
                        h = h.wrapping_mul(0x100_0000_01b3);
                    }
                }
                for b in w.bullets() {
                    for v in [b.pos.x.to_bits() as u64, b.pos.y.to_bits() as u64] {
                        h ^= v;
                        h = h.wrapping_mul(0x100_0000_01b3);
                    }
                }
            }
        }
        h
    }

    /// The "each body once" histories are fixed-size and cannot be grown by a
    /// horde.
    #[test]
    fn evolution_histories_are_bounded() {
        let mut dart = MilkTooth::new(Vec2::ZERO, Vec2::ZERO, 1.0);
        for id in 0..(MILK_BITE_CAP as u64 + 8) {
            dart.remember_bite(id);
        }
        assert_eq!(dart.bitten.len(), MILK_BITE_CAP);
        assert_eq!(dart.bitten_n as usize, MILK_BITE_CAP);
        assert!(dart.has_bitten(0));
        assert!(!dart.has_bitten(MILK_BITE_CAP as u64 + 1));

        let mut line = CheeseString::new(Vec2::ZERO, Vec2::new(1.0, 1.0), 1.0, 1.0);
        for id in 0..(STRING_CUT_CAP as u64 + 8) {
            line.remember_cut(id);
        }
        assert_eq!(line.cut_n as usize, STRING_CUT_CAP);
        assert!(!line.has_cut(u64::MAX));

        let mut ring = FairyRing::new(Vec2::ZERO, FAIRY_RADIUS, FAIRY_SECS, 4, 1.0);
        for id in 0..(FAIRY_TRACK_CAP as u64 + 8) {
            ring.cool(id, -1.0);
        }
        assert_eq!(ring.cooled.len(), FAIRY_TRACK_CAP);
        ring.tick_cooldowns(0.5);

        let mut channel = CorrosiveChannel::new(Vec2::ZERO, Vec2::new(1.0, 1.0), 32.0, 1.0, 1.0);
        for id in 0..(CHANNEL_HIT_CAP as u64 + 8) {
            channel.remember_hit(id);
        }
        assert_eq!(channel.hit_n as usize, CHANNEL_HIT_CAP);
    }
}

