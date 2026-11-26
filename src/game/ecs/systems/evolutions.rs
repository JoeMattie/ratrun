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
//! | Echo Chamber (Squeak Nova) | the ring's returning pass, run by `weapons::nova_ring` (`ActiveFx::nova_echo`) |
//! | Fairy Ring (Spore Orbit) | [`FairyRing`] left on every second cast, ticked here |
//! | Ratchet Tail (Tail Whip) | resolved immediately in `fire_one` (a forward throw, no state) |
//! | Drain Cleaner (Acid Spit) | [`CorrosiveChannel`] built where the lob lands, ticked here |
//! | Flywheel (Tail Thrash) | caught/flung inside `active_weapons_system` (`ActiveFx`) |
//! | Roll Call (Sonic Squeak) | tags set in `squeak_ring`, answered in [`roll_call_system`] |
//! | Downspout (Acid Rain) | drops placed in `fire_acid_rain`, landed in `acid_rain_system`, runoff is a [`CorrosiveChannel`] `gutter` |
//! | Dead Letter (Hantavirus) | [`DeadLetterPacket`] sealed by `hantavirus_death_system`, reopened in [`dead_letter_system`] |
//! | Carbon Copy (Laser Eyes) | `LaserFx::copy` recorded/replayed by `laser_eyes_system` |
//! | Ripcord (Super Dash) | the taut route in `DashTrack`, ticked by [`ripcord_system`] |
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
//! ## The four wave-3 rows (and what each one is, exactly)
//!
//! All twelve design rows are live; these four arrived with their weapons.
//! Each keeps the weapon's timing family, credits `DamageLedger` to the base
//! weapon and draws no rng in a run that does not wear it.
//!
//! - **Downspout** — `Acid Rain + Long Fuse`. "Replace separate splashes with a
//!   marching gutter. Strike the first marked target, then step each remaining
//!   drop 10 px toward the rat along the same line. Leave 1 s of burning
//!   runoff." `weapons_wave3a::fire_acid_rain` marks **one** foe and walks the
//!   rest of the volley back along the marking line ([`DOWNSPOUT_STEP`] apart);
//!   every drop still splashes on landing, and the last landing leaves a
//!   [`CorrosiveChannel::gutter`] stripe spanning the walk for
//!   [`RUNOFF_SECS`] (×Long Fuse). Its art is a stepped green staircase.
//! - **Dead Letter** — `Hantavirus + Second Scent`. "A host dying with jumps
//!   left leaves a sealed packet for 2 s. The first eligible foe within 10 px
//!   opens it and resumes the chain. One packet per chain."
//!   `weapons_wave3a::hantavirus_death_system` seals the envelope when a dying
//!   host finds no neighbour in reach; [`dead_letter_system`] ticks the fuse and
//!   hands the chain to the first eligible body ([`Infection::can_seal`] is set
//!   by the tag, [`Infection::sealed`] enforces one packet per chain).
//! - **Carbon Copy** — `Laser Eyes + Lens Cap`. "Record the origin and facing
//!   of a completed sweep. On the next sweep a fixed afterimage at that spot
//!   repeats it for half damage. Keep one copy." `LaserFx` keeps the recorded
//!   sweep and the replay state; `weapons_wave3b::laser_eyes_system` replays it
//!   a beat later — the same inward-closing pair of beams — at
//!   [`CARBON_COPY_DAMAGE`] × the recorded damage.
//! - **Ripcord** — `Super Dash + Spring Heel`. "After the dash, its actual route
//!   stays taut for 0.6 s, then snaps. Enemies on it take one dash hit and slide
//!   8 px toward the departure point. Bosses take damage only." `super_dash_system`
//!   records each travelled leg into `DashTrack`; [`ripcord_system`] runs the
//!   taut window ([`RIPCORD_SECS`]): one dash hit and an [`RIPCORD_SLIDE`] px
//!   pull toward the departure point per body, damage only on boss hit shapes.

// Systems mirror the legacy signature style (one resource per argument); the
// file is the evolution module's whole schedule section.
#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::game::bullet::Faction;
use crate::game::ecs::components::{
    CheeseString, CorrosiveChannel, DeadLetterPacket, FairyRing, FondueCast, FondueTine, Infection,
    MilkPhase, MilkTooth,
    SpawnId,
};
use crate::game::ecs::resources::{
    ActiveFx, DamageLedger, DashTrack, EnemyIndex, NextSpawnId, RngRes, TickInput, FLYWHEEL_CAP,
    RIPCORD_SECS,
};
use crate::game::collision::SpatialGrid;
use crate::game::ecs::constants::GRID_CELL;
use crate::game::ecs::systems::boss::{Boss, BossHitShapes};
use crate::game::ecs::systems::projectiles::los_clear;
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
///
/// The lead dart's flight is the whole Gnaw dart flight (`Bullet::new`'s 2.5 s
/// of life at 120 px/s = 300 px), not a short lob: an evolved Gnaw keeps the
/// targeting range its base form had (the aim picks a body up to
/// `target_range` away, so a shorter reach would fire into empty floor).
pub const MILK_REACH: f32 = 300.0;
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
/// A tine flies at the cheese spray's own dart speed.
pub const TINE_SPEED: f32 = 110.0;
/// Collision radius of a tine (the dab on the end of the fork).
pub const TINE_RADIUS: f32 = 1.5;
/// Longest a cast waits for its tines before the string is given up on (they
/// always settle by their reach, so this is a safety net, not a timer).
pub const TINE_FLIGHT_MAX: f32 = 3.0;
/// Radius of the ring the Fondue tines are planted at (matches the spray's
/// spread geometry only for the visual sag).
pub const STRING_SAG: f32 = 1.5;

/// A Fairy Ring stays fixed for this long.
pub const FAIRY_SECS: f32 = 3.0;
/// The ring's band is the drawn circle plus this much slack.
pub const FAIRY_BAND: f32 = 2.0;
/// One hit per target per second while crossing the ring.
pub const FAIRY_HIT_COOLDOWN: f32 = 1.0;
/// Line radius of the "is this body sheltered from the ring's centre" test
/// (a floor effect, the same 1 px every splash uses).
pub const FAIRY_LOS_RADIUS: f32 = 1.0;
/// Orbit radius of the fixed ring (the Spore Orbit radius).
pub const FAIRY_RADIUS: f32 = 26.0;

/// Seconds the Drain Cleaner's splash takes to run the whole stripe.
pub const CHANNEL_ADVANCE: f32 = 0.6;
/// Seconds the settled stripe keeps ticking after that (Long Fuse stretches it
/// per tick, never at spawn time).
pub const CHANNEL_SETTLE: f32 = crate::game::ecs::components::CHANNEL_DEFAULT_SETTLE;
/// Total stripe length and half width ("a 36 × 6 px corrosive channel").
pub const CHANNEL_LEN: f32 = 36.0;
pub const CHANNEL_HALF_W: f32 = 3.0;

/// Seconds between the Roll Call ring's tag and the answers.
pub const ROLL_CALL_DELAY: f32 = 0.8;
/// Radius of one answer pulse.
pub const ROLL_CALL_RADIUS: f32 = 9.0;
/// Answers that may hit the same body.
pub const ROLL_CALL_MAX_ANSWERS: u8 = 3;
/// Line-of-sight radius for one answer's shout (a 9 px ring, so a body behind a
/// wall is not in earshot even when it is close to the answer).
pub const ROLL_CALL_LOS_RADIUS: f32 = 1.0;
/// Distinct bodies one Roll Call ring can keep an answer tally for. Fixed (no
/// per-tick allocation); a body past the cap gets no answer rather than more
/// than three.
pub const ROLL_CALL_VICTIM_CAP: usize = 48;

/// Hostile pellets one Flywheel sweep catches.
pub const FLYWHEEL_PER_SWEEP: u8 = 2;
/// Speed a flung pellet leaves the tail at.
pub const FLYWHEEL_FLING_SPEED: f32 = 150.0;

/// **Downspout**: how far each drop steps back toward the rat (design §3).
pub const DOWNSPOUT_STEP: f32 = 10.0;
/// **Downspout**: seconds the gutter's runoff keeps burning (×Long Fuse).
pub const RUNOFF_SECS: f32 = 1.0;
/// **Downspout**: runoff damage per second, as a multiple of one splash. One
/// second of runoff is worth ~1.6 splashes, split into the stripe's ticks.
pub const RUNOFF_DPS: f32 = 1.6;
/// **Downspout**: shortest runoff stripe worth laying (a two-drop gutter).
pub const RUNOFF_MIN_LEN: f32 = 4.0;

/// **Dead Letter**: seconds a sealed packet waits on the floor (design §3).
pub const DEAD_LETTER_SECS: f32 = 2.0;
/// **Dead Letter**: how close a foe must come to tear the envelope open.
pub const DEAD_LETTER_REACH: f32 = 10.0;
/// Line-of-sight radius for the packet's open test (the same friendly-geometry
/// radius the virus chain uses for its jumps).
pub const DEAD_LETTER_LOS_RADIUS: f32 = 1.0;

/// **Carbon Copy**: the afterimage deals this fraction of the recorded pulse.
pub const CARBON_COPY_DAMAGE: f32 = 0.5;

/// **Ripcord**: how far the taut route pulls a body back toward the departure
/// point (design §3: "slide 8 px toward the departure point").
pub const RIPCORD_SLIDE: f32 = 8.0;

/// Key-space tag for the full boss inside the evolved effects' fixed per-body
/// tables (`FairyRing::cooled`, `CheeseString::cut`). `SpawnId`s are small and
/// monotonic, so the high bit keeps a boss id from ever colliding with one.
pub const BOSS_EFFECT_KEY: u64 = 1 << 63;

/// One evolved effect's hit on the full boss: the boss's own damage multiplier,
/// the ledger column of the base weapon, the damage number and a `true` when the
/// hit landed.
///
/// The boss is not an `Enemy`, so every evolved effect that can reach it has to
/// go through [`Boss::hit_shapes`] (exactly like Milk Teeth and the Drain
/// Cleaner) and [`Boss::take_damage`]; without this, a chest could turn a
/// boss-capable build into a zero-damage one.
#[allow(clippy::too_many_arguments)]
pub(super) fn damage_boss(
    boss: &mut Boss,
    damage: f32,
    kind: WeaponKind,
    source: DamageSource,
    ledger: &mut DamageLedger,
    damage_numbers: &mut DamageNumbers,
) -> bool {
    let amount = damage * boss.damage_mult;
    if !boss.take_damage(amount) {
        return false;
    }
    ledger.add_boss(kind, amount);
    let killed = boss.hp <= 0.0;
    damage_numbers.record(
        boss.id,
        boss.pos,
        boss.kind.body_radius(),
        amount,
        killed,
        source,
    );
    true
}

// ---- Stable enemy order ---------------------------------------------------

/// The tick's `SpawnId`-ordered enemy list (the `EnemyIndex` fast path, exactly
/// like `weapons.rs` / `combat.rs`), with the narrow-harness fallback sort
/// filling `scratch` when the grid never ran.
#[cfg(debug_assertions)]
pub(super) fn enemy_order<'a>(
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
pub(super) fn enemy_order<'a>(
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
///
/// `from` is the world point the hit arrives from, for directional armour
/// (`Enemy::hurt_from`: a curled Pillbug's shell). Every evolution has one:
/// the dart's previous position, the nearest point of the filament / ring /
/// stripe, the tagged body that answered. The number drawn is what landed.
#[allow(clippy::too_many_arguments)]
pub(super) fn hit_body(
    enemy: &mut Enemy,
    damage: f32,
    from: Vec2,
    kind: WeaponKind,
    ledger: &mut DamageLedger,
    damage_numbers: &mut DamageNumbers,
    source: DamageSource,
) -> bool {
    let hp_before = enemy.hp.max(0.0);
    let applied = enemy.hurt_from(damage, from);
    let killed = !enemy.alive();
    ledger.add(kind, (hp_before - enemy.hp.max(0.0)).max(0.0));
    damage_numbers.record(enemy.id, enemy.pos, enemy.radius, applied, killed, source);
    killed
}

/// The point of segment `a -> b` nearest to `p` (where a line-shaped hit
/// "arrives from" for directional armour).
pub(super) fn nearest_on_segment(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.len_sq();
    if len_sq <= 1e-6 {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

/// If `from -> to` crosses a wall, the truncated end point; else `to`. Also
/// reports whether it stopped.
fn wall_or(level: &Level, from: Vec2, to: Vec2, radius: f32) -> (Vec2, bool) {
    match level.tiles.segment_hit(from, to, radius) {
        Some(t) => (from + (to - from) * t, true),
        None => (to, false),
    }
}

/// One Milk Teeth hit on the full boss: the dart's segment against the boss's
/// damageable hit shapes, with the boss's own damage multiplier and the ledger
/// column of its base weapon. Returns the boss position when the hit landed.
///
/// Identical to the path `boss_damage_system` uses for player bullets, so a
/// chest that turns Gnaw into Milk Teeth cannot turn a boss-capable build into
/// a zero-damage one.
#[allow(clippy::too_many_arguments)]
fn bite_boss(
    boss: &mut Boss,
    from: Vec2,
    to: Vec2,
    damage: f32,
    ledger: &mut DamageLedger,
    damage_numbers: &mut DamageNumbers,
    rng: &mut rand::rngs::StdRng,
    particles: &mut Particles,
) -> Option<Vec2> {
    let shapes = boss.hit_shapes();
    if shapes.iter().next().is_none() || !shapes.segment_hits_grounded(from, to, MILK_RADIUS) {
        return None;
    }
    let amount = damage * boss.damage_mult;
    if !boss.take_damage(amount) {
        return None;
    }
    ledger.add_boss(WeaponKind::Gnaw, amount);
    let killed = boss.hp <= 0.0;
    vfx::emit_milk_teeth_bite(particles, rng, boss.pos, killed);
    damage_numbers.record(
        boss.id,
        boss.pos,
        boss.kind.body_radius(),
        amount,
        killed,
        DamageSource::Normal,
    );
    Some(boss.pos)
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
    mut bosses: Query<&mut Boss>,
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
                // The boss is not an `Enemy`: its hit shapes are the target,
                // exactly as `boss_damage_system` reaches it for a dart, so an
                // evolved Gnaw never stops being a boss weapon.
                let mut boss_bite: Option<Vec2> = None;
                if bit.is_none() {
                    for mut boss in bosses.iter_mut() {
                        if boss.dead {
                            continue;
                        }
                        boss_bite = bite_boss(
                            &mut boss,
                            from,
                            end,
                            dart.damage,
                            &mut ledger,
                            &mut damage_numbers,
                            &mut rng.0,
                            &mut particles,
                        );
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
                                from,
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
                } else if let Some(bpos) = boss_bite {
                    // The lead dart lodges in the boss's hit shape and comes
                    // home from there.
                    dart.pos = bpos;
                    dart.phase = MilkPhase::Lodged;
                    dart.age = 0.0;
                    dart.vel = Vec2::ZERO;
                } else if stopped || dart.age * MILK_SPEED >= dart.reach {
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
                // The thread always comes home (a tooth stranded behind a
                // wall would be a lost weapon), but it does not **bite**
                // through masonry: a step that crosses or starts inside a wall
                // bites nobody, the same cover rule every other friendly hit
                // obeys. It used to bite straight through walls all the way
                // back.
                let in_cover = level.tiles.segment_hit(from, to, MILK_RADIUS).is_some();
                // Bite every body the thread crosses, once each. The record
                // comes first: past `MILK_BITE_CAP` a body is not bitten at
                // all rather than bitten again on every tick it stays inside
                // the sweep.
                for &enemy in order.iter() {
                    if in_cover {
                        break;
                    }
                    let Ok((_, id, mut e)) = enemies.get_mut(enemy) else {
                        continue;
                    };
                    if !e.alive() || dart.has_bitten(id.0) {
                        continue;
                    }
                    if crate::game::collision::segment_circle_hit(from, to, MILK_RADIUS, e.pos, e.radius)
                    {
                        if !dart.remember_bite(id.0) {
                            continue;
                        }
                        let pos = e.pos;
                        let killed = hit_body(
                            &mut e,
                            dart.damage,
                            from,
                            kind,
                            &mut ledger,
                            &mut damage_numbers,
                            DamageSource::Normal,
                        );
                        vfx::emit_milk_teeth_bite(&mut particles, &mut rng.0, pos, killed);
                    }
                }
                // The boss is bitten once per return thread, on the ordinary
                // hit-shape path (and under the same cover rule).
                if !dart.boss_bitten && !in_cover {
                    for mut boss in bosses.iter_mut() {
                        if boss.dead {
                            continue;
                        }
                        if bite_boss(
                            &mut boss,
                            from,
                            to,
                            dart.damage,
                            &mut ledger,
                            &mut damage_numbers,
                            &mut rng.0,
                            &mut particles,
                        )
                        .is_some()
                        {
                            dart.boss_bitten = true;
                        }
                        break;
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

/// Fly every **Fondue Fork** tine and plant the string when both have settled.
///
/// A tine is a real projectile: it crosses the floor, hits the **first body it
/// meets** (one dart's damage, exactly what that outer dart always dealt) and
/// plants on its near edge, or plants at the first wall / at its Lens
/// Cap-stretched reach. The cast becomes the string that
/// [`fondue_string_system`] cuts with only once *both* tines have settled, so a
/// body that walks into an outer dart's path after the cast still moves its end
/// of the string — the review's "the ordinary outer bullets fly and deal damage
/// after the string already appears from a cast-time ray".
/// [`fondue_tines_system`]'s reused buffers.
///
/// `struck` is the "the tines and the filament share one hit per body" memory:
/// `(cast SpawnId, body key)` for every body (or boss, keyed with
/// [`BOSS_EFFECT_KEY`]) a tine of a still-pending cast hit. When the cast
/// becomes a string those bodies are entered in its cut list, so the body a
/// tine planted on is not cut a second time by the string that ends on it.
/// Entries leave with their cast, and the whole list is dropped whenever no
/// tine or cast is alive, so nothing survives into another floor.
#[derive(Default)]
pub struct TineScratch {
    /// (cast `SpawnId`, slot, plant point) in tine `SpawnId` order.
    settled: Vec<(u64, u8, Vec2)>,
    struck: Vec<(u64, u64)>,
}

pub fn fondue_tines_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut next_id: ResMut<NextSpawnId>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut tines: Query<(Entity, &SpawnId, &mut FondueTine)>,
    mut casts: Query<(Entity, &SpawnId, &mut FondueCast)>,
    mut fallback: Local<Vec<Entity>>,
    mut tine_scratch: Local<TineScratch>,
    mut order_scratch: Local<Vec<(u64, Entity)>>,
) {
    if tines.is_empty() && casts.is_empty() {
        tine_scratch.struck.clear();
        return; // an unevolved spray (or none at all): no work, no rng
    }
    let TineScratch { settled, struck } = &mut *tine_scratch;
    let dt = input.dt;
    let kind = WeaponKind::CheeseSpray;
    let order = enemy_order(&index, &enemies, &mut fallback);

    settled.clear();
    order_scratch.clear();
    order_scratch.extend(tines.iter().map(|(entity, id, _)| (id.0, entity)));
    order_scratch.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in order_scratch.iter() {
        let Ok((_, _, mut tine)) = tines.get_mut(entity) else {
            continue;
        };
        let from = tine.pos;
        let step = tine.vel * dt;
        let to = from + step;
        let (end, stopped) = wall_or(&level, from, to, TINE_RADIUS);
        let reach_step = step.len().min((end - from).len().max(0.0));
        let dir = if step.len() > 1e-4 {
            step * (1.0 / step.len())
        } else {
            Vec2::new(1.0, 0.0)
        };
        let limit = if stopped { (end - from).len() } else { reach_step };

        // The first body along this flight (nearest along the ray, ties by
        // `SpawnId` because `order` is the stable order).
        let mut body: Option<(f32, Vec2, Entity)> = None;
        for &candidate in order.iter() {
            let Ok((_, _, e)) = enemies.get(candidate) else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            let rel = e.pos - from;
            let along = rel.dot(dir);
            if along <= 0.0 || along > limit + e.radius {
                continue;
            }
            if (rel - dir * along).len() > e.radius + TINE_RADIUS {
                continue;
            }
            if body.is_none_or(|(best, _, _)| along < best) {
                let plant = from + dir * (along - e.radius).max(0.0);
                body = Some((along, plant, candidate));
            }
        }

        let mut planted: Option<Vec2> = None;
        if let Some((_, plant, candidate)) = body {
            if let Ok((_, sid, mut e)) = enemies.get_mut(candidate) {
                if e.alive() {
                    struck.push((tine.cast, sid.0));
                    let pos = e.pos;
                    let killed = hit_body(
                        &mut e,
                        tine.damage,
                        from,
                        kind,
                        &mut ledger,
                        &mut damage_numbers,
                        DamageSource::Normal,
                    );
                    vfx::emit_fondue_cut(&mut particles, &mut rng.0, pos, killed);
                }
            }
            planted = Some(plant);
        } else {
            // The full boss is a body too: its grounded shapes take the dart
            // (an outer dart was an ordinary player bullet before, and those
            // damage bosses).
            let mut boss_hit: Option<Vec2> = None;
            for mut boss in bosses.iter_mut() {
                if boss.dead {
                    continue;
                }
                let shapes = boss.hit_shapes();
                if shapes.iter().next().is_none()
                    || !shapes.segment_hits_grounded(from, end, TINE_RADIUS)
                {
                    continue;
                }
                let amount = tine.damage * boss.damage_mult;
                if !boss.take_damage(amount) {
                    continue;
                }
                ledger.add_boss(kind, amount);
                struck.push((tine.cast, BOSS_EFFECT_KEY | u64::from(boss.id)));
                let killed = boss.hp <= 0.0;
                vfx::emit_fondue_cut(&mut particles, &mut rng.0, boss.pos, killed);
                damage_numbers.record(
                    boss.id,
                    boss.pos,
                    boss.kind.body_radius(),
                    amount,
                    killed,
                    DamageSource::Normal,
                );
                boss_hit = Some(boss.pos);
                break;
            }
            if let Some(at) = boss_hit {
                let rel = at - from;
                let along = rel.dot(dir).clamp(0.0, limit);
                planted = Some(from + dir * along);
            }
        }

        // Advance, then settle on a body, a wall, or the end of the reach.
        if planted.is_none() {
            tine.prev = from;
            tine.pos = end;
            tine.reach = (tine.reach - reach_step).max(0.0);
            if stopped {
                planted = Some(end);
            } else if tine.reach <= 0.0 {
                planted = Some(tine.pos);
            }
        }
        if let Some(plant) = planted {
            settled.push((tine.cast, tine.slot, plant));
            commands.entity(entity).despawn();
        }
    }

    // Apply the plants to their casts, in cast `SpawnId` order.
    order_scratch.clear();
    order_scratch.extend(casts.iter().map(|(entity, id, _)| (id.0, entity)));
    order_scratch.sort_unstable_by_key(|&(id, _)| id);
    for &(cast_id, entity) in order_scratch.iter() {
        let Ok((_, _, mut cast)) = casts.get_mut(entity) else {
            continue;
        };
        for &(id, slot, plant) in settled.iter() {
            if id == cast_id {
                cast.ends[slot as usize % 2] = Some(plant);
            }
        }
        let done = cast.ends[0].is_some() && cast.ends[1].is_some();
        if done {
            let (a, b) = (cast.ends[0].unwrap_or(Vec2::ZERO), cast.ends[1].unwrap_or(Vec2::ZERO));
            let dmg = cast.damage;
            commands.entity(entity).despawn();
            let mut string = CheeseString::new(a, b, STRING_SECS, dmg);
            // One hit per body, ever: what this cast's tines already struck
            // starts out cut.
            for &(owner, key) in struck.iter() {
                if owner == cast_id {
                    let _ = string.remember_cut(key);
                }
            }
            struck.retain(|&(owner, _)| owner != cast_id);
            commands.spawn((string, next_id.take()));
            continue;
        }
        cast.wait -= dt;
        if cast.wait <= 0.0 {
            struck.retain(|&(owner, _)| owner != cast_id);
            commands.entity(entity).despawn();
        }
    }
}

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
    mut bosses: Query<&mut Boss>,
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
                // Recording first keeps the once-per-body rule honest past
                // `STRING_CUT_CAP`: an untracked body is not cut at all.
                if !line.remember_cut(id.0) {
                    continue;
                }
                let pos = e.pos;
                let killed = hit_body(
                    &mut e,
                    line.damage,
                    nearest_on_segment(line.a, line.b, pos),
                    kind,
                    &mut ledger,
                    &mut damage_numbers,
                    DamageSource::Normal,
                );
                vfx::emit_fondue_cut(&mut particles, &mut rng.0, pos, killed);
            }
        }
        // The filament cuts the full boss too (its hit shapes, one cut ever
        // per body, exactly like the ordinary bodies above).
        for mut boss in bosses.iter_mut() {
            if boss.dead {
                continue;
            }
            let key = BOSS_EFFECT_KEY | u64::from(boss.id);
            if line.has_cut(key) {
                continue;
            }
            let shapes = boss.hit_shapes();
            if shapes.iter().next().is_none()
                || !shapes.segment_hits_grounded(line.a, line.b, STRING_RADIUS)
            {
                continue;
            }
            if !line.remember_cut(key) {
                continue;
            }
            let at = boss.pos;
            if damage_boss(
                &mut boss,
                line.damage,
                kind,
                DamageSource::Normal,
                &mut ledger,
                &mut damage_numbers,
            ) {
                let killed = boss.hp <= 0.0;
                vfx::emit_fondue_cut(&mut particles, &mut rng.0, at, killed);
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
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
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
            // The ring grew out from where the rat stood: masonry between
            // that point and a body shelters it, exactly as it shelters a
            // body from a splash (the band used to burn through walls).
            if edge <= e.radius + FAIRY_BAND
                && los_clear(&level, ring.pos, e.pos, FAIRY_LOS_RADIUS)
            {
                // The cooldown table is the "one hit per body per second"
                // memory: if the ring cannot record this body (the table is
                // full of other live cooldowns) it does not hit it either,
                // instead of recycling an entry and re-hitting its owner.
                if !ring.cool(id.0, FAIRY_HIT_COOLDOWN) {
                    continue;
                }
                let pos = e.pos;
                let killed = hit_body(
                    &mut e,
                    ring.damage,
                    ring.pos + (pos - ring.pos).normalized() * ring.radius,
                    kind,
                    &mut ledger,
                    &mut damage_numbers,
                    DamageSource::Orbit,
                );
                vfx::emit_fairy_ring_hit(&mut particles, &mut rng.0, pos, killed);
            }
        }
        // The band burns the full boss too, on the same one-hit-per-second
        // cooldown (the boss id is tagged into the same fixed table).
        for mut boss in bosses.iter_mut() {
            if boss.dead {
                continue;
            }
            let key = BOSS_EFFECT_KEY | u64::from(boss.id);
            if ring.cooldown(key) > 0.0 {
                continue;
            }
            let shapes = boss.hit_shapes();
            if shapes.iter().next().is_none()
                || !shapes.ring_hits_grounded(ring.pos, ring.radius, FAIRY_BAND)
            {
                continue;
            }
            if level
                .tiles
                .segment_hit(ring.pos, boss.pos, FAIRY_LOS_RADIUS)
                .is_some()
            {
                continue;
            }
            if !ring.cool(key, FAIRY_HIT_COOLDOWN) {
                continue;
            }
            let at = boss.pos;
            if damage_boss(
                &mut boss,
                ring.damage,
                kind,
                DamageSource::Orbit,
                &mut ledger,
                &mut damage_numbers,
            ) {
                let killed = boss.hp <= 0.0;
                vfx::emit_fairy_ring_hit(&mut particles, &mut rng.0, at, killed);
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

/// Truncate a stripe of `nominal` px at the first wall along its direction
/// ("stop at masonry").
pub fn channel_open_len(level: &Level, start: Vec2, dir: Vec2, nominal: f32) -> f32 {
    let end = start + dir * nominal;
    match level.tiles.segment_hit(start, end, CHANNEL_HALF_W) {
        Some(t) => (nominal * t).max(4.0),
        None => nominal,
    }
}

/// Truncate a Drain Cleaner channel at the first wall along its direction ("stop
/// at masonry"). Called where the splash lands (`projectiles::bullets_system`).
pub fn channel_length(level: &Level, start: Vec2, dir: Vec2) -> f32 {
    channel_open_len(level, start, dir, CHANNEL_LEN)
}

/// Is `pos` inside the stripe open after `open_len` px?
fn in_stripe(start: Vec2, dir: Vec2, open_len: f32, pos: Vec2, radius: f32) -> bool {
    let rel = pos - start;
    let along = rel.dot(dir);
    let perp = (rel - dir * along).len();
    along >= -radius && along <= open_len + radius && perp <= CHANNEL_HALF_W + radius
}

/// Does the corrosive stripe cover any of the boss's grounded hit shapes?
///
/// The boss is not an `Enemy` body, so `in_stripe` (which takes a circle) is
/// applied per shape: a shape's centre inside the stripe, or the stripe's axis
/// inside a shape's own radius, counts as a cover.
fn stripe_covers_point(shapes: &BossHitShapes, start: Vec2, dir: Vec2, open_len: f32) -> bool {
    // The stripe's axis is sampled in 4 px steps (the same approximation
    // `BossHitShape::overlaps_segment` uses), so a shape wider than the stripe
    // is still caught by its own radius and a shape the stripe misses is not.
    let steps = ((open_len / 4.0).ceil().max(1.0)) as u32;
    shapes.iter().any(|shape| {
        shape.grounded()
            && (0..=steps).any(|i| {
                let t = i as f32 / steps as f32;
                shape.overlaps_circle(start + dir * (open_len * t), CHANNEL_HALF_W)
            })
    })
}

/// Tick every **Drain Cleaner** channel: the advancing front, then the pool.
pub fn drain_cleaner_system(
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
    mut bosses: Query<&mut Boss>,
    mut channels: Query<(Entity, &SpawnId, &mut CorrosiveChannel)>,
    mut fallback: Local<Vec<Entity>>,
    mut channel_order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let order = enemy_order(&index, &enemies, &mut fallback);
    let kind = WeaponKind::AcidSpit;
    // The wave-3 passive hooks the *unevolved* Acid Spit already honours, read
    // here so the evolved stripe honours them the same way: Lens Cap stretches
    // the 36 px stripe (then masonry truncates it again), Long Fuse stretches
    // the settled 2 s (and, for a Downspout runoff, its 1 s).
    let stats = player.loadout.stats;

    channel_order.clear();
    channel_order.extend(channels.iter().map(|(entity, id, _)| (id.0, entity)));
    channel_order.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in channel_order.iter() {
        let Ok((_, _, mut ch)) = channels.get_mut(entity) else {
            continue;
        };
        ch.age += dt;
        // A Downspout gutter is Acid Rain's own runoff: its damage is credited
        // to the original weapon (§3), not to Acid Spit.
        let kind = if ch.runoff { WeaponKind::AcidRain } else { kind };
        let open = channel_open_len(&level, ch.start, ch.dir, stats.ranged_reach(ch.length));
        let advance = if ch.runoff { 0.0 } else { CHANNEL_ADVANCE };
        let settle = stats.zone_life(ch.settle);
        if ch.age <= advance {
            // The splash is still running: everything the open stripe covers
            // takes one front hit, once per body.
            let open_len = open * (ch.age / advance).clamp(0.0, 1.0);
            for &enemy in order.iter() {
                let Ok((_, id, mut e)) = enemies.get_mut(enemy) else {
                    continue;
                };
                if !e.alive() || ch.has_hit(id.0) {
                    continue;
                }
                if in_stripe(ch.start, ch.dir, open_len, e.pos, e.radius) {
                    // Record before the hit: a body the front cannot remember
                    // (the fixed history is full) is skipped, never hit again
                    // on the next tick of the same advancing front.
                    if !ch.remember_hit(id.0) {
                        continue;
                    }
                    let pos = e.pos;
                    let killed = hit_body(
                        &mut e,
                        ch.front_damage,
                        pos - ch.dir,
                        kind,
                        &mut ledger,
                        &mut damage_numbers,
                        DamageSource::Acid,
                    );
                    vfx::emit_drain_hit(&mut particles, &mut rng.0, pos, killed);
                }
            }
            // The full boss is not an `Enemy`: its hit shapes take the same one
            // front hit a body does (so the replacement chip cannot leave a
            // boss-capable build with no way to hurt the boss), with the boss's
            // own damage multiplier and the weapon's ledger column.
            if !ch.boss_hit && !ch.runoff {
                for mut boss in bosses.iter_mut() {
                    if boss.dead {
                        continue;
                    }
                    let shapes = boss.hit_shapes();
                    if shapes.iter().next().is_none() {
                        continue;
                    }
                    if !shapes.circle_hits_grounded(
                        ch.start + ch.dir * open_len,
                        CHANNEL_HALF_W,
                    ) && !stripe_covers_point(&shapes, ch.start, ch.dir, open_len)
                    {
                        continue;
                    }
                    let amount = ch.front_damage * boss.damage_mult;
                    if !boss.take_damage(amount) {
                        continue;
                    }
                    ch.boss_hit = true;
                    ledger.add_boss(kind, amount);
                    let killed = boss.hp <= 0.0;
                    vfx::emit_drain_hit(&mut particles, &mut rng.0, boss.pos, killed);
                    damage_numbers.record(
                        boss.id,
                        boss.pos,
                        boss.kind.body_radius(),
                        amount,
                        killed,
                        DamageSource::Acid,
                    );
                    break;
                }
            }
        } else {
            // Settled stripe: pool ticks follow it.
            let tick = ch.dps * dt;
            for &enemy in order.iter() {
                let Ok((_, _, mut e)) = enemies.get_mut(enemy) else {
                    continue;
                };
                if !e.targetable() {
                    continue;
                }
                if in_stripe(ch.start, ch.dir, open, e.pos, e.radius) {
                    let hp_before = e.hp.max(0.0);
                    // Directional armour, as `projectiles::pools_system`
                    // does it: the runoff arrives from the nearest point of
                    // the stripe's axis. `note_damage` books the side effects
                    // (a Porter flinches, a dead Wick Moth lights its fuse)
                    // without `hurt`'s tint.
                    let axis_end = ch.start + ch.dir * ch.length;
                    let tick = tick * e.damage_mult_from(nearest_on_segment(ch.start, axis_end, e.pos));
                    e.hp -= tick;
                    e.note_damage();
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
        if ch.age > advance {
            // The settled stripe is a pool: it ticks the boss's hit shapes the
            // same way `boss_damage_system` ticks a round `Pool`.
            for mut boss in bosses.iter_mut() {
                if boss.dead {
                    continue;
                }
                let shapes = boss.hit_shapes();
                if shapes.iter().next().is_none() {
                    continue;
                }
                if !stripe_covers_point(&shapes, ch.start, ch.dir, open) {
                    continue;
                }
                let amount = ch.dps * dt * boss.damage_mult;
                if !boss.take_damage(amount) {
                    continue;
                }
                ledger.add_boss(kind, amount);
                damage_numbers.record(
                    boss.id,
                    boss.pos,
                    boss.kind.body_radius(),
                    amount,
                    false,
                    DamageSource::Acid,
                );
            }
        }
        let front = if advance > 0.0 {
            open * (ch.age / advance).clamp(0.0, 1.0)
        } else {
            open
        };
        if ch.runoff {
            // Downspout: the stripe is a stepped green gutter, already settled.
            let life_frac = 1.0 - (ch.age / settle.max(1e-4)).clamp(0.0, 1.0);
            vfx::emit_downspout_runoff(
                &mut particles,
                &mut rng.0,
                ch.start,
                ch.dir,
                front,
                life_frac,
                dt,
            );
        } else {
            vfx::emit_drain_channel(
                &mut particles,
                &mut rng.0,
                ch.start,
                ch.dir,
                front,
                ch.age,
                dt,
            );
        }
        if ch.age >= advance + settle {
            commands.entity(entity).despawn();
        }
    }
}

// ---- Dead Letter ----------------------------------------------------------

/// Tick every sealed **Dead Letter** packet: count the fuse down, draw the
/// envelope, and let the first eligible foe that steps within
/// [`DEAD_LETTER_REACH`] tear it open and resume the chain inside.
///
/// Eligibility is the chain's own rule (alive, not already visited by this
/// chain, no active mark, a clear line), which is why a reopened chain can
/// never walk the same body twice. Packets are walked in `SpawnId` order and a
/// body an earlier packet claimed this tick is skipped, so two envelopes in one
/// spot cannot both open on the same body.
pub fn dead_letter_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    infected: Query<&Infection>,
    mut packets: Query<(Entity, &SpawnId, &mut DeadLetterPacket)>,
    mut fallback: Local<Vec<Entity>>,
    mut packet_order: Local<Vec<(u64, Entity)>>,
    mut claimed: Local<Vec<Entity>>,
) {
    // Cheap early out: no envelope on the floor, no rng, no walk.
    if packets.is_empty() {
        return;
    }
    let dt = input.dt;
    let order = enemy_order(&index, &enemies, &mut fallback);

    packet_order.clear();
    packet_order.extend(packets.iter().map(|(entity, id, _)| (id.0, entity)));
    packet_order.sort_unstable_by_key(|&(id, _)| id);
    claimed.clear();

    for &(_, entity) in packet_order.iter() {
        let Ok((_, _, mut packet)) = packets.get_mut(entity) else {
            continue;
        };
        packet.life -= dt;
        let pos = packet.pos;
        if packet.life <= 0.0 {
            // Unopened: the chain is lost with the envelope.
            commands.entity(entity).despawn();
            vfx::emit_dead_letter_fizzle(&mut particles, &mut rng.0, pos);
            continue;
        }
        let life_frac = (packet.life / packet.total.max(1e-4)).clamp(0.0, 1.0);
        vfx::emit_dead_letter_sealed(&mut particles, &mut rng.0, pos, life_frac);

        // The first eligible foe within reach opens it (enemy order, so the
        // nearest-in-`SpawnId` body wins a tie).
        let chain = packet.chain;
        let mut opener: Option<(u64, Entity)> = None;
        for &host in order.iter() {
            let Ok((_, id, e)) = enemies.get(host) else {
                continue;
            };
            if !e.alive() || claimed.contains(&host) || e.pos.dist(pos) > DEAD_LETTER_REACH {
                continue;
            }
            if infected.get(host).is_ok_and(|i| i.active()) || chain.has_visited(id.0) {
                continue;
            }
            if !los_clear(&level, pos, e.pos, DEAD_LETTER_LOS_RADIUS) {
                continue;
            }
            opener = Some((id.0, host));
            break;
        }
        let Some((id, target)) = opener else {
            continue;
        };
        let Some(next) = chain.jumped(id) else {
            // Fixed history full: the chain ends rather than allocate.
            commands.entity(entity).despawn();
            vfx::emit_dead_letter_tear(&mut particles, &mut rng.0, pos);
            continue;
        };
        commands.entity(target).insert(next);
        claimed.push(target);
        commands.entity(entity).despawn();
        let to = enemies
            .get(target)
            .map(|(_, _, e)| e.pos)
            .unwrap_or(pos);
        vfx::emit_dead_letter_tear(&mut particles, &mut rng.0, pos);
        vfx::emit_virus_jump(&mut particles, &mut rng.0, pos, to);
    }
}

// ---- Ripcord -------------------------------------------------------------

/// Run a **Ripcord** cord: the route a finished dash travelled stays taut for
/// [`RIPCORD_SECS`], damaging every body on it once and dragging it
/// [`RIPCORD_SLIDE`] px back toward where the dash left from; then it snaps.
///
/// The cord is only built while the weapon is owned *and* evolved
/// (`weapons_wave3b::super_dash_system` records the legs), so a run without the
/// evolution reaches the early return below: no walk, no rng, no value written.
/// Damage uses the dash's own numbers (the weapon's per-level hit after stats)
/// and books `SuperDash` in the ledger. Bosses take the damage only — the slide
/// is what the cord does to bodies it can drag, and a boss cannot be dragged.
pub fn ripcord_system(
    level: Res<Level>,
    player: Res<Player>,
    input: Res<TickInput>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut track: ResMut<DashTrack>,
    mut grid: ResMut<SpatialGrid>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut fallback: Local<Vec<Entity>>,
) {
    if player.buffs.weapons_suspended() {
        track.clear_route();
        return;
    }
    if track.route_life <= 0.0 || track.route_len() == 0 {
        return;
    }
    // The cord is the evolution of a weapon the player must still hold.
    let Some(weapon) = player
        .loadout
        .weapons
        .iter()
        .find(|w| w.kind == WeaponKind::SuperDash && w.evolved)
    else {
        track.clear_route();
        return;
    };
    let dt = input.dt;
    // The cord quotes the dash it came from, charge and all: a charged dash's
    // longer route is joined by a wider, harder-hitting cord.
    let charge = track.charge.clamp(0.0, 1.0);
    let radius = weapon.dash_radius()
        * (1.0 + crate::game::ecs::systems::weapons_wave3b::SUPER_DASH_RADIUS_PER_CHARGE * charge);
    let dmg = weapon.damage()
        * player.damage_mult()
        * player.loadout.weapon_damage_mult(weapon.kind)
        * (1.0 + crate::game::ecs::systems::weapons_wave3b::SUPER_DASH_DAMAGE_PER_CHARGE * charge);
    let kind = WeaponKind::SuperDash;
    let taut = !track.live_this_tick;
    let start = track.route_start();
    let legs = track.route_len();

    // Set when the cord slid a body: the grid is re-filed below.
    let mut slid = false;
    if taut {
        let order = enemy_order(&index, &enemies, &mut fallback);
        for &entity in order.iter() {
            let Ok((_, id, mut e)) = enemies.get_mut(entity) else {
                continue;
            };
            if !e.alive() || track.route_already_hit(id.0) {
                continue;
            }
            let mut on_route = false;
            for i in 0..legs {
                let (a, b) = track.route_segment(i);
                if crate::game::collision::segment_circle_hit(a, b, radius, e.pos, e.radius) {
                    on_route = true;
                    break;
                }
            }
            if !on_route {
                continue;
            }
            track.mark_route_hit(id.0);
            let pos = e.pos;
            let back = start - pos;
            // Directional armour: the snap travels along the cord toward the
            // departure point, so it strikes a body from the side away from
            // it (a Pillbug curled with its shell to the far end of the dash
            // takes the quarter, one with its back to it takes the double).
            let from = if back.len_sq() > 0.25 {
                pos - back.normalized() * (e.radius + 1.0)
            } else {
                pos
            };
            let killed = hit_body(
                &mut e,
                dmg,
                from,
                kind,
                &mut ledger,
                &mut damage_numbers,
                DamageSource::Normal,
            );
            // The cord pulls the body back toward where the dash started, on
            // legal floor (a wall is not where a rat's tail can drag you).
            // "Bosses take damage only" (§3) covers every body a cord cannot
            // drag: the legacy boss kind and a named miniboss keep their feet.
            let dragged = !e.crab_pot_held && !e.named && e.kind != crate::game::enemy::EnemyKind::Boss;
            if dragged && back.len_sq() > 0.25 {
                // Counterweight: the cord's slide is a weapon's native shove,
                // so the passive that raises every shove raises this one too.
                let shifted =
                    pos + back.normalized() * player.loadout.stats.shove(RIPCORD_SLIDE);
                e.pos = level.tiles.resolve_circle(
                    shifted,
                    e.radius,
                    crate::game::collision::SizeClass::Small,
                    None,
                );
                slid = true;
            }
            vfx::emit_ripcord_hit(&mut particles, &mut rng.0, pos, back, killed);
        }

        // Bosses take the damage only (design §3).
        if !track.route_boss_hit {
            for mut boss in bosses.iter_mut() {
                if boss.dead {
                    continue;
                }
                let shapes = boss.hit_shapes();
                if shapes.iter().next().is_none() {
                    continue;
                }
                let mut touched = false;
                for i in 0..legs {
                    let (a, b) = track.route_segment(i);
                    if shapes.segment_hits_grounded(a, b, radius) {
                        touched = true;
                        break;
                    }
                }
                if !touched {
                    continue;
                }
                // The flag only matters on the first boss the cord meets, and
                // there is one boss per fight.
                track.route_boss_hit = true;
                let amount = dmg * boss.damage_mult;
                if !boss.take_damage(amount) {
                    continue;
                }
                ledger.add_boss(kind, amount);
                let killed = boss.hp <= 0.0;
                vfx::emit_ripcord_hit(
                    &mut particles,
                    &mut rng.0,
                    boss.pos,
                    boss.pos - start,
                    killed,
                );
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
    }

    // The slide wrote `Enemy::pos`. `EnemyIndex` is still the live order
    // (nothing spawned or despawned), but the grid lists each slid body under
    // its old cell, and the wave-3/4 and trailer passes still run this tick.
    // Re-file the buckets from the index, as Counterweight's nudge, Ratchet
    // Tail and the Barbed Wire drag do. Skipped in a narrow test whose index
    // was never built (filling from a stale index would empty the grid).
    if slid && index.0.len() == enemies.iter().len() {
        grid.reset(level.arena, GRID_CELL);
        grid.fill(
            index
                .0
                .iter()
                .filter_map(|&entity| enemies.get(entity).ok().map(|(_, _, e)| e.pos)),
        );
    }

    // Draw the cord while it is taut, then let the fuse run out. The legs are
    // the dash's own route, so the cord is exactly what was travelled.
    let frac = (track.route_life / RIPCORD_SECS).clamp(0.0, 1.0);
    vfx::emit_ripcord_cord(&mut particles, &mut rng.0, track.route(), frac, start);
    if taut {
        track.route_life -= dt;
        if track.route_life <= 0.0 {
            // The fuse burns out: the cord snaps back to the rat's tail.
            vfx::emit_ripcord_snap(&mut particles, &mut rng.0, track.route(), start);
            track.clear_route();
        }
    }
}

// ---- Roll Call ------------------------------------------------------------

/// Answer a **Roll Call** squeak: each tagged body pulses once, three answers
/// maximum into the same victim.
pub fn roll_call_system(
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut evo: ResMut<ActiveFx>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
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
    // How many answers have already hit each victim, a fixed tally sized for
    // the **victims** (not for the emitters: one ring of answers can reach far
    // more bodies than the ring tagged). A body the tally cannot hold gets no
    // answer at all, so a 13th victim can never take every answer in the room.
    let mut answers: [Option<(u64, u8)>; ROLL_CALL_VICTIM_CAP] = [None; ROLL_CALL_VICTIM_CAP];
    // The full boss answers too, capped like every victim (its own tally: it is
    // not in the `SpawnId` space the table is keyed by).
    let mut boss_answers = 0u8;
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
            // An answer is a shout that travels: masonry stops it. The pulse is
            // a 9 px ring, so a 1 px line is the honest width for "can the
            // answer reach this body" (the same radius the whip and the virus
            // jump use). Without it a thin wall between the answering body and
            // a nearby victim was damage straight through cover.
            if !crate::game::ecs::systems::projectiles::los_clear(
                &level,
                origin,
                victim.pos,
                ROLL_CALL_LOS_RADIUS,
            ) {
                continue;
            }
            // The cap counts answers per body across the whole ring.
            let mut count = 0u8;
            let mut at: Option<usize> = None;
            let mut full = true;
            for (i, entry) in answers.iter().enumerate() {
                match entry {
                    Some((held, n)) if *held == victim_id.0 => {
                        count = *n;
                        at = Some(i);
                        full = false;
                        break;
                    }
                    None => {
                        at = Some(i);
                        full = false;
                        break;
                    }
                    _ => {}
                }
            }
            // Recording the answer is a precondition of the hit: a tally with
            // no room for this body cannot enforce "three answers each", so it
            // withholds the damage instead of letting every answer through.
            if count >= ROLL_CALL_MAX_ANSWERS || full {
                continue;
            }
            if let Some(i) = at {
                answers[i] = Some((victim_id.0, count + 1));
            }
            let pos = victim.pos;
            let killed = hit_body(
                &mut victim,
                damage,
                origin,
                WeaponKind::SonicSqueak,
                &mut ledger,
                &mut damage_numbers,
                DamageSource::Normal,
            );
            vfx::emit_roll_call_pulse(&mut particles, &mut rng.0, pos, killed);
            let _ = id;
        }
        // One answer per emitter into the full boss, through its hit shapes
        // and with the same masonry rule as the victims above.
        if boss_answers < ROLL_CALL_MAX_ANSWERS {
            for mut boss in bosses.iter_mut() {
                if boss.dead {
                    continue;
                }
                let shapes = boss.hit_shapes();
                if shapes.iter().next().is_none()
                    || !shapes.circle_hits_grounded(origin, ROLL_CALL_RADIUS)
                {
                    continue;
                }
                if !los_clear(&level, origin, boss.pos, ROLL_CALL_LOS_RADIUS) {
                    continue;
                }
                let at = boss.pos;
                if damage_boss(
                    &mut boss,
                    damage,
                    WeaponKind::SonicSqueak,
                    DamageSource::Normal,
                    &mut ledger,
                    &mut damage_numbers,
                ) {
                    boss_answers += 1;
                    let killed = boss.hp <= 0.0;
                    vfx::emit_roll_call_pulse(&mut particles, &mut rng.0, at, killed);
                }
                break; // one boss per answer
            }
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
        AcidDrop, CHANNEL_HIT_CAP, FAIRY_TRACK_CAP, MILK_BITE_CAP, STRING_CUT_CAP,
    };
    use crate::game::ecs::resources::{spawn_with_id, EvoRuntime, LaserFx};
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

    /// A boss body planted at the arena centre, in its neutral state (so its
    /// hit shapes are grounded and damageable), with the `BossFight` book it
    /// needs. Used by the "the evolution still hurts a boss" tests.
    fn spawn_grounded_boss(
        w: &mut World,
        kind: crate::game::ecs::systems::boss::BossKind,
    ) -> Entity {
        use crate::game::ecs::systems::boss::{Boss, BossFight};
        let mid = w.level.arena * 0.5;
        let arena = w
            .level
            .boss_room()
            .map(|r| r.rect)
            .unwrap_or_else(|| crate::game::level::TileRect::new(0, 0, 10, 10));
        let mut boss = Boss::new(kind, mid, 5000.0, arena, mid, false);
        boss.invuln = 0.0;
        let entity = w.ecs.spawn((boss, SpawnId(999))).id();
        let _ = std::marker::PhantomData::<BossFight>;
        entity
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
    /// The Fondue Fork's tines really are planted by the firing arm: the two
    /// outer darts fly as tines (they are *not* cast-time rays) and the string
    /// only appears once both have settled, with both ends within the tine
    /// reach of the rat and one on each side of the aim.
    #[test]
    fn cheese_spray_tines_land_before_the_string_appears() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::CheeseSpray);
        let origin = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);
        // The string is not a cast-time ray any more: the tines are still in
        // the air on the tick the spray fires.
        {
            let mut q = w.ecs.query::<&CheeseString>();
            assert_eq!(
                q.iter(&w.ecs).count(),
                0,
                "the string appeared before its tines had flown"
            );
        }
        {
            let mut q = w.ecs.query::<&FondueTine>();
            assert_eq!(q.iter(&w.ecs).count(), 2, "the two outer darts are tines");
        }
        let mut ticks = 0;
        let line = loop {
            run(&mut w, fondue_tines_system);
            ticks += 1;
            let mut q = w.ecs.query::<&CheeseString>();
            if let Some(line) = q.iter(&w.ecs).next() {
                break line.clone();
            }
            assert!(ticks < 120, "the tines never planted a string");
        };
        // A tine flies at 110 px/s and plants at its 40 px reach, so the pair
        // needs a handful of ticks — never the firing tick.
        assert!(ticks > 1, "the string cannot be planted at cast time");
        for end in [line.a, line.b] {
            let d = end.dist(origin);
            assert!(d <= TINE_REACH + 0.5, "a tine planted past its reach: {d}");
            assert!((d - TINE_REACH).abs() < 1.0, "an unblocked tine plants at its reach: {d}");
        }
        // The two ends straddle the aim (east), so the string crosses it.
        assert!(
            (line.a - origin).y * (line.b - origin).y <= 0.0,
            "the string must span the aim: {:?} / {:?}",
            line.a,
            line.b
        );
    }

    /// A body that steps into an outer tine's path **after the cast** is what
    /// plants that end of the string (the review's sketch): the tine hits the
    /// real collision, not a ray drawn at fire time.
    #[test]
    fn a_late_body_moves_the_tine_and_its_end_of_the_string() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::CheeseSpray);
        let origin = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);
        // One tick of flight, then park a sturdy body on the left tine's line
        // (base is east, the outer darts at ±half·0.18 rad from it).
        run(&mut w, fondue_tines_system);
        let half = (w.player.loadout.weapons[0].level_projectiles() as f32 - 1.0) * 0.5;
        let angle = -half * 0.18;
        let cross = origin + Vec2::from_angle(angle) * 14.0;
        let body = w.spawn_enemy(EnemyKind::Brute, cross, 400.0);
        let hp_before = w.ecs.get::<Enemy>(body).map(|e| e.hp).unwrap_or(0.0);
        let mut ticks = 0;
        while ticks < 60 {
            run(&mut w, fondue_tines_system);
            ticks += 1;
            let mut q = w.ecs.query::<&CheeseString>();
            if q.iter(&w.ecs).next().is_some() {
                break;
            }
        }
        let hp_after = w.ecs.get::<Enemy>(body).map(|e| e.hp).unwrap_or(0.0);
        assert!(
            hp_after < hp_before,
            "the tine crossed the late body without hitting it ({hp_before} -> {hp_after})"
        );
        let mut q = w.ecs.query::<&CheeseString>();
        let line = q.iter(&w.ecs).next().expect("the pair still plants a string").clone();
        // One end landed on the body, short of the nominal 40 px reach.
        let short = line.a.dist(origin).min(line.b.dist(origin));
        assert!(
            short < TINE_REACH - 4.0,
            "no end planted on the late body (nearest end {short} px)"
        );
    }
    /// **Echo Chamber**: since batch 12 the nova is one expanding ring, so the
    /// evolution is the ring **coming back**: it strikes the walls, contracts
    /// again and cuts everything inside on the way in for a second hit.
    #[test]
    fn echo_chamber_ring_comes_back_for_a_second_hit() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::SqueakNova);
        let origin = w.player.pos;
        // Two bodies inside the ring, and one standing on open floor far
        // enough out that only a ring this wide can reach it.
        let near = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(20.0, 0.0), 400.0);
        let far = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(0.0, -70.0), 400.0);
        let (near_hp, far_hp) = (hp_of(&w, near), hp_of(&w, far));
        let damage = w.player.loadout.weapons[0].damage();
        w.player.loadout.weapons[0].timer = 0.0;

        let mut ticks = 0;
        while ticks < 120 {
            run(&mut w, weapons::fire_weapons_system);
            ticks += 1;
            let fx = *w.ecs.resource::<ActiveFx>();
            if !fx.nova_active() {
                break;
            }
        }
        assert!(!w.ecs.resource::<ActiveFx>().nova_active(), "the ring never ended");
        // Once out, once back.
        assert!(
            (near_hp - hp_of(&w, near) - 2.0 * damage).abs() < 1e-2,
            "the returning front must cut again: {:.1} -> {:.1}",
            near_hp,
            hp_of(&w, near)
        );
        assert!(
            (far_hp - hp_of(&w, far) - 2.0 * damage).abs() < 1e-2,
            "every body inside the ring is cut on both passes: {:.1} -> {:.1}",
            far_hp,
            hp_of(&w, far)
        );
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

    /// A 3-cell solid column at tile column `cx`, centred on tile row `cy`.
    fn wall_column(w: &mut World, cx: i32, cy: i32) {
        w.level.tiles.fill_cells(
            cx,
            cy - 1,
            1,
            3,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
    }

    /// Fairy Ring: masonry between the planted centre and a body shelters it
    /// (the band used to burn straight through a wall), while a body on the
    /// open side of the same ring still burns.
    #[test]
    fn fairy_ring_does_not_burn_through_a_wall() {
        let mut w = evolved_world(WeaponKind::SporeOrbit);
        let damage = w.player.loadout.weapons[0].damage();
        // Centre in tile (99, 80); the column fills tile column 100.
        let centre = Vec2::new(99.0 * 16.0 + 8.0, 80.0 * 16.0 + 8.0);
        wall_column(&mut w, 100, 80);
        spawn_with_id(
            &mut w.ecs,
            FairyRing::new(centre, FAIRY_RADIUS, FAIRY_SECS, 5, damage),
        );
        let hidden = w.spawn_enemy(EnemyKind::Skitterer, centre + Vec2::new(FAIRY_RADIUS, 0.0), 400.0);
        let open = w.spawn_enemy(EnemyKind::Skitterer, centre + Vec2::new(-FAIRY_RADIUS, 0.0), 400.0);
        let (hidden_hp, open_hp) = (hp_of(&w, hidden), hp_of(&w, open));
        run(&mut w, fairy_ring_system);
        assert!(hp_of(&w, open) < open_hp, "the open side of the ring burns");
        assert_eq!(hp_of(&w, hidden), hidden_hp, "the ring burnt a body behind a wall");
        // Booked to Spore Orbit, the base weapon.
        let ledger = w.ecs.resource::<DamageLedger>();
        assert!(ledger.dealt[WeaponKind::SporeOrbit.index()] > 0.0);
    }

    /// Milk Teeth: the return thread comes home through a wall but bites
    /// nobody while it is inside the masonry, and bites again in the open.
    #[test]
    fn milk_teeth_return_does_not_bite_inside_a_wall() {
        let mut w = evolved_world(WeaponKind::Gnaw);
        // Rat in tile (99, 80), wall in column 100, the lodged tooth beyond it.
        let y = 80.0 * 16.0 + 8.0;
        w.player.pos = Vec2::new(99.0 * 16.0 + 2.0, y);
        wall_column(&mut w, 100, 80);
        let mut dart = MilkTooth::new(Vec2::new(102.0 * 16.0 + 8.0, y), Vec2::ZERO, 5.0, MILK_REACH);
        dart.phase = MilkPhase::Back;
        spawn_with_id(&mut w.ecs, dart);
        // One body in the middle of the masonry (a test fixture: nothing can
        // stand there, which is the point), one in the open beyond it.
        let walled = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(100.0 * 16.0 + 8.0, y), 400.0);
        let open = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(101.0 * 16.0 + 10.0, y), 400.0);
        let (walled_hp, open_hp) = (hp_of(&w, walled), hp_of(&w, open));
        for _ in 0..60 {
            run(&mut w, milk_teeth_system);
        }
        assert_eq!(
            w.ecs.query::<&MilkTooth>().iter(&w.ecs).count(),
            0,
            "the thread still comes home through the wall"
        );
        assert!(hp_of(&w, open) < open_hp, "the thread bites in the open");
        assert_eq!(hp_of(&w, walled), walled_hp, "the thread bit through masonry");
        let ledger = w.ecs.resource::<DamageLedger>();
        assert!(ledger.dealt[WeaponKind::Gnaw.index()] > 0.0, "booked to Gnaw");
    }

    /// Fondue Fork: the body a tine planted on took that tine's hit, so the
    /// string that ends on it does not cut it a second time ("one hit per
    /// body, ever: the tines and the filament share it").
    #[test]
    fn a_tine_struck_body_is_not_cut_again_by_its_own_string() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::CheeseSpray);
        let origin = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        let half = (w.player.loadout.weapons[0].level_projectiles() as f32 - 1.0) * 0.5;
        // Auto-aim centres the fan on the body itself, so the outer tine
        // leaves `half * 0.18` rad off the line to it: the body stands close
        // enough (10 px) that the tine still crosses its radius.
        let cross = origin + Vec2::from_angle(-half * 0.18) * 10.0;
        let body = w.spawn_enemy(EnemyKind::Brute, cross, 400.0);
        // Fire by hand so the inner darts never fly: only the tines and the
        // string can touch the body in this harness.
        run(&mut w, weapons::fire_weapons_system);
        let before = hp_of(&w, body);
        let mut strung = false;
        for _ in 0..80 {
            run(&mut w, fondue_tines_system);
            if w.ecs.query::<&CheeseString>().iter(&w.ecs).next().is_some() {
                strung = true;
                break;
            }
        }
        assert!(strung, "the pair never planted a string");
        let after_tine = hp_of(&w, body);
        assert!(after_tine < before, "the tine hit the body it planted on");
        for _ in 0..10 {
            run(&mut w, fondue_string_system);
        }
        assert_eq!(
            hp_of(&w, body),
            after_tine,
            "the string cut the body its own tine had already struck"
        );
    }

    /// The swipe follows aim. The throw follows the rat's facing and damages
    /// the crowd in front. A closer elite takes the swipe and stays put.
    #[test]
    fn ratchet_tail_throws_along_the_rats_facing() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::TailWhip);
        let origin = w.player.pos;
        let hook_pos = origin + Vec2::new(-20.0, -12.0);
        let elite_pos = origin + Vec2::new(-10.0, -14.0);
        let hooked = w.spawn_enemy(EnemyKind::Skitterer, hook_pos, 400.0);
        let crowd = w.spawn_enemy(EnemyKind::Skitterer, origin + Vec2::new(17.0, -12.0), 400.0);
        let elite = w.spawn_enemy(EnemyKind::Skitterer, elite_pos, 400.0);
        w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
        let crowd_hp = hp_of(&w, crowd);
        let hooked_hp = hp_of(&w, hooked);
        let elite_hp = hp_of(&w, elite);
        w.player.face = Vec2::new(1.0, 0.0);
        let weapon = &mut w.player.loadout.weapons[0];
        weapon.timer = 0.0;
        let damage = weapon.damage();
        // Deliberately aim off the facing. Run only the weapon pass so the
        // player system does not turn the rat toward the reticle first.
        let aim = origin + Vec2::new(30.0, 30.0);
        w.begin_tick_with_aim(DT, Vec2::ZERO, false, false, Some(aim));
        w.ecs.run_system_cached(weapons::fire_weapons_system).unwrap();
        w.end_tick();

        assert_eq!(hp_of(&w, crowd), crowd_hp - damage, "the throw hits the crowd once");
        assert!(hp_of(&w, hooked) < hooked_hp, "the whip hit lands on the hook");
        assert!(hp_of(&w, elite) < elite_hp, "the elite takes the whip hit");
        let moved = w.ecs.get::<Enemy>(hooked).unwrap().pos;
        assert!(
            moved.x > origin.x && (moved.y - hook_pos.y).abs() < 1e-3,
            "the hooked body lands forward along the facing: {moved:?}"
        );
        assert_eq!(w.ecs.get::<Enemy>(elite).unwrap().pos, elite_pos);
    }

    #[test]
    fn ratchet_tail_throw_stops_at_masonry() {
        use crate::game::art_walls::WallKind;
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::TailWhip);
        let origin = Vec2::new(170.0, 168.0);
        w.player.pos = origin;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        w.level.tiles.fill_cells(11, 9, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        let hook_pos = origin - Vec2::new(40.0, 0.0);
        let hooked = w.spawn_enemy(EnemyKind::Skitterer, hook_pos, 400.0);
        let crowd = w.spawn_enemy(EnemyKind::Skitterer, origin + Vec2::new(25.0, 0.0), 400.0);
        let crowd_hp = hp_of(&w, crowd);

        run(&mut w, weapons::fire_weapons_system);

        let body = w.ecs.get::<Enemy>(hooked).unwrap();
        assert!(body.pos.x > hook_pos.x && body.pos.x + body.radius < 176.0);
        assert_eq!(hp_of(&w, crowd), crowd_hp);
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

    // ---- Regressions: saturation, boss hits, pellet order ---------------

    /// A fixed "each body once" history must not turn into damage over time
    /// past its cap: a body the history cannot record is **not** hit again on
    /// the next tick (`MILK_BITE_CAP` + 1 bodies in one return thread).
    #[test]
    fn a_full_history_never_repeats_a_milk_bite() {
        let mut w = evolved_world(WeaponKind::Gnaw);
        let origin = w.player.pos;
        // One more body than the dart can remember, all in a line the return
        // thread crosses, all sturdy enough to survive one bite.
        let bodies: Vec<Entity> = (0..(MILK_BITE_CAP as i32 + 1))
            .map(|i| {
                w.spawn_enemy(
                    EnemyKind::Brute,
                    origin + Vec2::new(40.0 + i as f32 * 1.5, 0.0),
                    600.0,
                )
            })
            .collect();
        let before: Vec<f32> = bodies.iter().map(|&e| hp_of(&w, e)).collect();
        {
            let mut dart = MilkTooth::new(
                origin + Vec2::new(30.0, 0.0),
                Vec2::ZERO,
                3.0,
                MILK_REACH,
            );
            dart.phase = MilkPhase::Back;
            spawn_with_id(&mut w.ecs, dart);
        }
        // Two ticks of the return thread through the whole line.
        for _ in 0..2 {
            run(&mut w, milk_teeth_system);
        }
        let after: Vec<f32> = bodies.iter().map(|&e| hp_of(&w, e)).collect();
        for (i, (b, a)) in before.iter().zip(&after).enumerate() {
            let hits = (b - a) / 3.0;
            assert!(
                hits <= 1.0 + 1e-4,
                "body {i} was bitten {hits} times (cap {MILK_BITE_CAP}, one bite each)"
            );
        }
    }

    /// The same rule for the Fondue string, the Fairy Ring's cooldown table and
    /// the Drain Cleaner's front history.
    #[test]
    fn full_evolution_histories_withhold_rather_than_repeat() {
        // Fondue string: cap + 1 crossers, two ticks.
        let mut w = evolved_world(WeaponKind::CheeseSpray);
        let origin = w.player.pos;
        let bodies: Vec<Entity> = (0..(STRING_CUT_CAP as i32 + 1))
            .map(|i| {
                w.spawn_enemy(
                    EnemyKind::Brute,
                    origin + Vec2::new(0.0, -30.0 + i as f32 * 1.0),
                    600.0,
                )
            })
            .collect();
        let before: Vec<f32> = bodies.iter().map(|&e| hp_of(&w, e)).collect();
        spawn_with_id(
            &mut w.ecs,
            CheeseString::new(
                origin + Vec2::new(0.0, -40.0),
                origin + Vec2::new(0.0, 40.0),
                STRING_SECS,
                3.0,
            ),
        );
        for _ in 0..2 {
            run(&mut w, fondue_string_system);
        }
        for (i, (&b, &a)) in before.iter().zip(bodies.iter()).enumerate() {
            let hits = (b - hp_of(&w, a)) / 3.0;
            assert!(hits <= 1.0 + 1e-4, "string cut body {i} {hits} times");
        }

        // Fairy ring: cap + 1 bodies standing on the band for a second.
        let mut w = evolved_world(WeaponKind::SporeOrbit);
        let origin = w.player.pos;
        let bodies: Vec<Entity> = (0..(FAIRY_TRACK_CAP as i32 + 1))
            .map(|i| {
                let a = i as f32 * 0.05;
                w.spawn_enemy(
                    EnemyKind::Brute,
                    origin + Vec2::from_angle(a) * FAIRY_RADIUS,
                    600.0,
                )
            })
            .collect();
        let before: Vec<f32> = bodies.iter().map(|&e| hp_of(&w, e)).collect();
        spawn_with_id(&mut w.ecs, FairyRing::new(origin, FAIRY_RADIUS, FAIRY_SECS, 4, 3.0));
        for _ in 0..4 {
            run(&mut w, fairy_ring_system);
        }
        for (i, (&b, &a)) in before.iter().zip(bodies.iter()).enumerate() {
            let hits = (b - hp_of(&w, a)) / 3.0;
            assert!(
                hits <= 1.0 + 1e-4,
                "ring hit body {i} {hits} times inside one cooldown window"
            );
        }
    }

    /// **Roll Call** caps a victim at three answers by remembering the
    /// **victims**: a 13th body in range of every answer used to take all of
    /// them (the tally was sized to the twelve emitters).
    #[test]
    fn roll_call_caps_every_victim_at_three_answers() {
        use crate::game::ecs::resources::ROLL_CALL_TAG_CAP;
        let mut w = evolved_world(WeaponKind::SonicSqueak);
        let origin = w.player.pos;
        // Twelve tagged emitters spread around a single tough victim: every
        // answer pulse reaches it.
        let victim = w.spawn_enemy(EnemyKind::Brute, origin, 900.0);
        let before = hp_of(&w, victim);
        // Twelve tagged emitters packed around the victim (inside the answer
        // pulse's 9 px), each tagged with its own `SpawnId`.
        let mut tags: Vec<(Entity, u64)> = Vec::new();
        for i in 0..ROLL_CALL_TAG_CAP {
            let e = w.spawn_enemy(
                EnemyKind::Brute,
                origin + Vec2::from_angle(i as f32 * 0.5) * 6.0,
                900.0,
            );
            let id = w.ecs.get::<SpawnId>(e).unwrap().0;
            tags.push((e, id));
        }
        {
            let mut fx = w.ecs.resource_mut::<ActiveFx>();
            fx.clear_roll_call();
            for (e, id) in tags {
                assert!(fx.tag(e, id), "tag");
            }
            fx.answer_left = ROLL_CALL_DELAY;
            fx.answer_damage = 2.0;
        }
        // Walk the 0.8 s delay out: the answers fire once it expires.
        for _ in 0..60 {
            run(&mut w, roll_call_system);
        }
        let answers = (before - hp_of(&w, victim)) / 2.0;
        assert!(
            answers <= ROLL_CALL_MAX_ANSWERS as f32 + 1e-4,
            "the 13th victim took {answers} answers (cap {ROLL_CALL_MAX_ANSWERS})"
        );
        assert!(answers > 0.0, "the answer pulses must still land");
    }

    /// **Roll Call's answers stop at masonry.** A victim nine pixels away
    /// behind a thin wall was in "earshot" of the old distance-only test.
    #[test]
    fn roll_call_answers_do_not_cross_a_wall() {
        let mut w = evolved_world(WeaponKind::SonicSqueak);
        let origin = w.player.pos;
        // A one-cell solid partition just east of the answering caller.
        let cx = (origin.x / 16.0).floor() as i32 + 1;
        let cy = (origin.y / 16.0).floor() as i32;
        for dy in -3..=3 {
            w.level.tiles.set(
                cx,
                cy + dy,
                TileKind::Wall(crate::game::art_walls::WallKind::Solid),
            );
        }
        w.level.tiles.rebuild_masks();
        let wall_x = (cx as f32) * 16.0;
        // The caller sits west of the wall; the victim east of it, close enough
        // for the pulse's radius but on the far side of the partition.
        let caller = w.spawn_enemy(EnemyKind::Brute, Vec2::new(wall_x - 4.0, origin.y), 900.0);
        let victim = w.spawn_enemy(EnemyKind::Brute, Vec2::new(wall_x + 4.0, origin.y), 900.0);
        let before = hp_of(&w, victim);
        let id = w.ecs.get::<SpawnId>(caller).unwrap().0;
        {
            let mut fx = w.ecs.resource_mut::<ActiveFx>();
            fx.clear_roll_call();
            assert!(fx.tag(caller, id), "tag");
            fx.answer_left = ROLL_CALL_DELAY;
            fx.answer_damage = 3.0;
        }
        for _ in 0..70 {
            run(&mut w, roll_call_system);
        }
        assert_eq!(
            before,
            hp_of(&w, victim),
            "the answer went through the wall"
        );
    }

    /// **Milk Teeth** bites the full boss (a chest must not turn a
    /// boss-capable build into a zero-damage one): the outbound dart lodges in
    /// its hit shape and the return thread bites it once.
    #[test]
    fn milk_teeth_damages_a_grounded_boss() {
        use crate::game::ecs::systems::boss::BossKind;
        let mut w = evolved_world(WeaponKind::Gnaw);
        let boss = spawn_grounded_boss(&mut w, BossKind::Cat);
        let hp0 = w.ecs.get::<Boss>(boss).unwrap().hp;
        let boss_pos = w.ecs.get::<Boss>(boss).unwrap().pos;
        let origin = w.player.pos;
        let to = boss_pos - origin;
        spawn_with_id(
            &mut w.ecs,
            MilkTooth::new(origin, to.normalized() * MILK_SPEED, 3.0, MILK_REACH),
        );
        // Fly out, lodge, come home.
        for _ in 0..120 {
            run(&mut w, milk_teeth_system);
            if w.ecs.get::<Boss>(boss).map(|b| b.hp < hp0).unwrap_or(false) {
                break;
            }
        }
        let hp1 = w.ecs.get::<Boss>(boss).unwrap().hp;
        assert!(hp1 < hp0, "the evolved Gnaw never touched the boss ({hp0} -> {hp1})");
    }

    /// **Drain Cleaner**'s channel — the replacement for Acid Spit's pool —
    /// keeps hurting a grounded boss: one front hit, then the settled ticks.
    #[test]
    fn drain_cleaner_damages_a_grounded_boss() {
        use crate::game::ecs::systems::boss::BossKind;
        let mut w = evolved_world(WeaponKind::AcidSpit);
        let boss = spawn_grounded_boss(&mut w, BossKind::Cat);
        let boss_pos = w.ecs.get::<Boss>(boss).unwrap().pos;
        let hp0 = w.ecs.get::<Boss>(boss).unwrap().hp;
        // A stripe that starts on the boss and runs away from it.
        let dir = Vec2::new(1.0, 0.0);
        spawn_with_id(
            &mut w.ecs,
            CorrosiveChannel::new(boss_pos, dir, CHANNEL_LEN, 30.0, 12.0),
        );
        for _ in 0..40 {
            run(&mut w, drain_cleaner_system);
        }
        let hp1 = w.ecs.get::<Boss>(boss).unwrap().hp;
        assert!(hp1 < hp0, "the settled channel never touched the boss ({hp0} -> {hp1})");
    }

    /// Which boss-hitting effect [`boss_hp_lost`] exercises.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum BossHitter {
        MilkTeeth,
        DrainCleaner,
        TailWhip,
        /// **Fairy Ring**: the fixed band (one hit per second).
        FairyRing,
        /// **Fondue Fork**: the filament between the tines.
        FondueString,
        /// **Roll Call**: one answer pulse.
        RollCall,
    }

    /// A boss state to force: `(state, duration, elapsed)`.
    type BossPose = (crate::game::ecs::systems::boss::BossState, f32, f32);

    /// HP one `hitter` takes off a `kind` boss put in `state` (`None`: the
    /// neutral state `Boss::new` gives), aimed at its first grounded target
    /// point (the body centre when it has none). Also returns whether the boss
    /// was damageable through the shared rule every weapon uses — a grounded
    /// hit shape — so a caller can compare against what a plain bullet could do.
    fn boss_hp_lost(
        kind: crate::game::ecs::systems::boss::BossKind,
        state: Option<BossPose>,
        mult: f32,
        hitter: BossHitter,
    ) -> (f32, bool) {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(match hitter {
            BossHitter::MilkTeeth => WeaponKind::Gnaw,
            BossHitter::DrainCleaner => WeaponKind::AcidSpit,
            BossHitter::TailWhip => WeaponKind::TailWhip,
            BossHitter::FairyRing => WeaponKind::SporeOrbit,
            BossHitter::FondueString => WeaponKind::CheeseSpray,
            BossHitter::RollCall => WeaponKind::SonicSqueak,
        });
        w.player.loadout.weapons[0].evolved = hitter != BossHitter::TailWhip;
        w.player.loadout.weapons[0].timer = 0.0;
        let boss = spawn_grounded_boss(&mut w, kind);
        let (aim, hittable, hp0) = {
            let mut b = w.ecs.get_mut::<Boss>(boss).unwrap();
            if let Some((s, duration, elapsed)) = state {
                b.set_state(s, duration);
                b.elapsed = elapsed;
            }
            b.damage_mult = mult;
            let points = b.target_points();
            (points.first().unwrap_or(b.pos), !points.is_empty(), b.hp)
        };
        match hitter {
            BossHitter::MilkTeeth => {
                let origin = aim + Vec2::new(-40.0, 0.0);
                w.player.pos = origin;
                spawn_with_id(
                    &mut w.ecs,
                    MilkTooth::new(origin, Vec2::new(MILK_SPEED, 0.0), 3.0, MILK_REACH),
                );
                for _ in 0..240 {
                    run(&mut w, milk_teeth_system);
                }
            }
            BossHitter::DrainCleaner => {
                spawn_with_id(
                    &mut w.ecs,
                    CorrosiveChannel::new(aim, Vec2::new(1.0, 0.0), CHANNEL_LEN, 30.0, 12.0),
                );
                for _ in 0..40 {
                    run(&mut w, drain_cleaner_system);
                }
            }
            BossHitter::TailWhip => {
                w.player.pos = aim + Vec2::new(-6.0, 0.0);
                w.player.face = Vec2::new(-1.0, 0.0);
                run(&mut w, weapons::fire_weapons_system);
            }
            BossHitter::FairyRing => {
                // A ring whose band passes through the boss's first grounded
                // point: the satellites' own circle cuts it once a second.
                spawn_with_id(
                    &mut w.ecs,
                    FairyRing::new(aim, 4.0, FAIRY_SECS, 4, 7.0),
                );
                for _ in 0..6 {
                    run(&mut w, fairy_ring_system);
                }
            }
            BossHitter::FondueString => {
                spawn_with_id(
                    &mut w.ecs,
                    CheeseString::new(
                        aim + Vec2::new(-12.0, -12.0),
                        aim + Vec2::new(12.0, 12.0),
                        STRING_SECS,
                        7.0,
                    ),
                );
                for _ in 0..4 {
                    run(&mut w, fondue_string_system);
                }
            }
            BossHitter::RollCall => {
                // One tagged neighbour inside the answer pulse's 9 px: its
                // answer is the hit.
                let caller = w.spawn_enemy(EnemyKind::Brute, aim + Vec2::new(4.0, 0.0), 900.0);
                let id = w.ecs.get::<SpawnId>(caller).unwrap().0;
                {
                    let mut fx = w.ecs.resource_mut::<ActiveFx>();
                    fx.clear_roll_call();
                    assert!(fx.tag(caller, id), "tag");
                    fx.answer_left = ROLL_CALL_DELAY;
                    fx.answer_damage = 7.0;
                }
                for _ in 0..80 {
                    run(&mut w, roll_call_system);
                }
            }
        }
        (hp0 - w.ecs.get::<Boss>(boss).unwrap().hp, hittable)
    }

    /// **All six bosses**, not just the three the fixes were written against:
    /// Milk Teeth, the Drain Cleaner, the Tail Whip (whose boss hit moved out of
    /// `boss_damage_system` into the cast), the Fairy Ring, the Fondue string
    /// and Roll Call each damage every boss while it is damageable, honour its
    /// weak-window multiplier, and pass under a boss with no grounded shape
    /// exactly as a bullet does.
    #[test]
    fn milk_teeth_drain_cleaner_and_whip_damage_all_six_bosses() {
        use crate::game::art_boss_cat::BossCatState;
        use crate::game::art_boss_magpie::{self, MagpieState};
        use crate::game::art_boss_mangle::{self, MangleState};
        use crate::game::art_boss_night_edition::{self, NightEditionState};
        use crate::game::ecs::systems::boss::{BossKind, BossState};
        const HITTERS: [BossHitter; 6] = [
            BossHitter::MilkTeeth,
            BossHitter::DrainCleaner,
            BossHitter::TailWhip,
            // The three effects the integrator flagged: a chest evolution must
            // not skip the full boss (they had no boss query at all).
            BossHitter::FairyRing,
            BossHitter::FondueString,
            BossHitter::RollCall,
        ];
        // (boss, state, incoming multiplier of that state)
        let damageable: [(BossKind, Option<BossPose>, f32); 10] = [
            (BossKind::Cat, None, 1.0),
            (BossKind::Broom, None, 1.0),
            (BossKind::Mower, None, 1.0),
            (BossKind::Mangle, None, 1.0),
            (BossKind::NightEdition, None, 1.0),
            (BossKind::Magpie, None, 1.0),
            // The punish windows, each exposed its own way.
            (
                BossKind::Mangle,
                Some((BossState::Mangle(MangleState::Jam), 1.5, 0.5)),
                art_boss_mangle::JAM_DAMAGE_MULT,
            ),
            // The final jam ungrounds the rollers: only the crank is left.
            (
                BossKind::Mangle,
                Some((
                    BossState::Mangle(MangleState::FinalJam),
                    art_boss_mangle::FINAL_JAM,
                    0.5,
                )),
                art_boss_mangle::JAM_DAMAGE_MULT,
            ),
            (
                BossKind::NightEdition,
                Some((BossState::NightEdition(NightEditionState::BedLowered), 2.0, 0.5)),
                art_boss_night_edition::WEAK_MULT,
            ),
            (
                BossKind::Magpie,
                Some((BossState::Magpie(MagpieState::Stuck), 2.0, 0.5)),
                art_boss_magpie::STUCK_DAMAGE_MULT,
            ),
        ];
        for (kind, state, mult) in damageable {
            for hitter in HITTERS {
                let (base, _) = boss_hp_lost(kind, state, 1.0, hitter);
                let (lost, hittable) = boss_hp_lost(kind, state, mult, hitter);
                assert!(hittable, "{kind:?} {state:?} should expose a grounded shape");
                assert!(
                    lost > 0.0,
                    "{hitter:?} never hurt {kind:?} in {state:?} ({lost})"
                );
                assert!(
                    (lost - base * mult).abs() <= 1e-3 * base.max(1.0),
                    "{hitter:?} on {kind:?} {state:?}: {lost} is not x{mult} of {base}"
                );
            }
        }
        // States a bullet may or may not reach: the three effects must agree
        // with the shared grounded-shape rule either way. The cat mid-pounce is
        // over the rat's head; the Magpie's strafe is flown belly-to-the-planks
        // (`Boss::hit_shapes` keeps it grounded on purpose).
        let either: [(BossKind, BossPose); 4] = [
            (BossKind::Cat, (BossState::Cat(BossCatState::Pounce), 0.70, 0.35)),
            (BossKind::Magpie, (BossState::Magpie(MagpieState::Takeoff), 1.0, 0.5)),
            (BossKind::Magpie, (BossState::Magpie(MagpieState::StrafePass), 1.0, 0.5)),
            (BossKind::Magpie, (BossState::Magpie(MagpieState::SnatchDrop), 1.0, 0.5)),
        ];
        let mut saw_untouchable = false;
        for (kind, state) in either {
            for hitter in HITTERS {
                let (lost, hittable) = boss_hp_lost(kind, Some(state), 1.0, hitter);
                assert_eq!(
                    lost > 0.0,
                    hittable,
                    "{hitter:?} on {kind:?} {state:?}: lost {lost}, grounded shape {hittable}"
                );
                saw_untouchable |= !hittable;
            }
        }
        assert!(saw_untouchable, "the airborne cat must be out of reach");
    }

    /// **Flywheel** catches pellets in `SpawnId` order, not in query storage
    /// order: the recorded catch directions are the three lowest ids'.
    #[test]
    fn flywheel_catches_pellets_in_spawn_order() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::TailThrash);
        let origin = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        // Three hostile pellets inside the arc, each with its own direction so
        // the recorded catch identifies them.
        let mut dirs = Vec::new();
        for i in 0..3usize {
            let vel = Vec2::from_angle(0.4 * i as f32) * 40.0;
            dirs.push(vel);
            let b = Bullet::new(
                origin + Vec2::new(4.0, 4.0 + i as f32 * 3.0),
                vel,
                5.0,
                Faction::Enemy,
                (255, 120, 90),
            );
            spawn_with_id(&mut w.ecs, b);
        }
        w.begin_tick(DT, Vec2::ZERO, true);
        w.ecs
            .run_system_cached(weapons::active_weapons_system)
            .unwrap();
        w.end_tick();
        let fx = w.active_fx();
        let caught = fx.caught as usize;
        assert!(caught > 0, "the evolved sweep must catch pellets");
        // The first sweep's catches, in `SpawnId` (= spawn) order.
        for k in 0..caught {
            assert_eq!(
                fx.caught_dirs[k], dirs[k],
                "catch {k} is not the {k}th pellet in `SpawnId` order"
            );
        }
    }

    // ---- The four wave-3 evolutions ------------------------------------

    /// Every live Acid Rain drop, in `SpawnId` (volley) order.
    fn rain_drops(w: &mut World) -> Vec<AcidDrop> {
        let mut q = w.ecs.query::<(&SpawnId, &AcidDrop)>();
        let mut v: Vec<(u64, AcidDrop)> = q.iter(&w.ecs).map(|(id, d)| (id.0, *d)).collect();
        v.sort_unstable_by_key(|&(id, _)| id);
        v.into_iter().map(|(_, d)| d).collect()
    }

    /// Every sealed Dead Letter packet's position, in `SpawnId` order.
    fn letters(w: &mut World) -> Vec<Vec2> {
        let mut q = w.ecs.query::<(&SpawnId, &DeadLetterPacket)>();
        let mut v: Vec<(u64, Vec2)> = q.iter(&w.ecs).map(|(id, l)| (id.0, l.pos)).collect();
        v.sort_unstable_by_key(|&(id, _)| id);
        v.into_iter().map(|(_, p)| p).collect()
    }

    fn mark_of(w: &World, entity: Entity) -> Option<Infection> {
        w.ecs.get::<Infection>(entity).copied()
    }

    /// Hantavirus is deployed by droppings now: fire one cycle and set the
    /// dropping it left under `host`, armed, then step the drop so the body
    /// standing there takes the chain the old ranged tag used to plant on it.
    fn infect_by_dropping(w: &mut World, host: Entity) {
        use crate::game::ecs::components::Dropping;
        use crate::game::ecs::systems::{weapons, weapons_wave3a};
        let at = w.ecs.get::<Enemy>(host).expect("host").pos;
        w.player.loadout.weapons[0].timer = 0.0;
        run(w, weapons::fire_weapons_system);
        let mut live = w.ecs.query::<(Entity, &Dropping)>();
        let dropping = live
            .iter(&w.ecs)
            .next()
            .map(|(entity, _)| entity)
            .expect("the weapon leaves a dropping");
        {
            let mut d = w.ecs.get_mut::<Dropping>(dropping).expect("dropping");
            d.pos = at;
            d.age = crate::game::weapon::DROPPING_ARM;
        }
        run(w, weapons_wave3a::droppings_system);
    }

    /// **Downspout**: the volley marks one foe and then *walks* the rest of its
    /// drops back along the marking line, 10 px at a time; the last landing
    /// leaves 1 s of burning runoff.
    #[test]
    fn downspout_marches_its_gutter_home_and_leaves_runoff() {
        use crate::game::ecs::systems::{weapons, weapons_wave3a};
        let mut w = evolved_world(WeaponKind::AcidRain);
        // L3: three drops.
        w.player.loadout.weapons[0].level = 3;
        let origin = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(24.0, 0.0), 400.0);
        let hp0 = hp_of(&w, body);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);

        let drops = rain_drops(&mut w);
        assert_eq!(drops.len(), 3, "L3 fires three drops");
        assert!(drops.iter().all(|d| d.evolved), "the gutter is an evolved volley");
        // One mark, then steps of 10 px back toward the rat along the same line.
        let offsets: Vec<f32> = drops.iter().map(|d| d.pos.x - origin.x).collect();
        assert!((offsets[0] - 24.0).abs() < 1.0, "the mark is the target: {offsets:?}");
        for pair in offsets.windows(2) {
            let step = pair[0] - pair[1];
            assert!(
                (step - DOWNSPOUT_STEP).abs() < 1e-3 || pair[1] <= 4.0 + 1e-3,
                "each drop steps 10 px home: {offsets:?}"
            );
        }
        assert!(offsets[2] < offsets[0], "the gutter marches toward the rat");

        // Land them all: every drop splashes, then the gutter's runoff appears.
        let mut saw_runoff = false;
        for _ in 0..60 {
            run(&mut w, weapons_wave3a::acid_rain_system);
            if !saw_runoff {
                let mut q = w.ecs.query::<&CorrosiveChannel>();
                if let Some(ch) = q.iter(&w.ecs).next() {
                    assert!(ch.runoff, "the gutter's stripe is a runoff, not a channel");
                    assert!((ch.settle - RUNOFF_SECS).abs() < 1e-5);
                    assert!((ch.length - 20.0).abs() < 1.0, "spans the walk: {}", ch.length);
                    assert!((ch.dir.x - 1.0).abs() < 1e-4, "laid along the march line");
                    saw_runoff = true;
                }
            }
        }
        assert!(saw_runoff, "the finished gutter must leave a runoff");
        let burnt = hp_of(&w, body);
        assert!(burnt < hp0, "the gutter's splashes hit the marked body");

        // The runoff keeps burning the stripe for ~1 s, then dries out.
        for _ in 0..20 {
            run(&mut w, drain_cleaner_system);
        }
        assert!(hp_of(&w, body) < burnt, "the runoff burns what stands on it");
        let mut gone = false;
        for tick in 0..90 {
            run(&mut w, drain_cleaner_system);
            if w.ecs.query::<&CorrosiveChannel>().iter(&w.ecs).count() == 0 {
                assert!(tick > 20, "the runoff cannot dry instantly");
                gone = true;
                break;
            }
        }
        assert!(gone, "1 s of runoff and no more");
    }

    /// **Dead Letter**: a chain whose host dies isolated seals into a packet;
    /// the first foe to step within 10 px reopens it and walks on; one packet
    /// per chain, so the reopened chain that dies isolated simply ends.
    #[test]
    fn dead_letter_seals_an_isolated_chain_and_reopens_it_once() {
        use crate::game::ecs::systems::weapons_wave3a;
        let mut w = evolved_world(WeaponKind::Hantavirus);
        w.player.loadout.weapons[0].level = 3; // 2 jumps, 18 px reach
        let origin = w.player.pos;
        let host = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(12.0, 0.0), 400.0);
        infect_by_dropping(&mut w, host);
        let mark = mark_of(&w, host).expect("the evolved drop plants a chain");
        assert!(mark.can_seal, "an evolved chain may seal");
        assert_eq!(mark.jumps_left, 2);

        // Killed with jumps left and nobody in reach: it seals instead of dying.
        w.ecs.get_mut::<Enemy>(host).unwrap().hp = 0.0;
        run(&mut w, weapons_wave3a::hantavirus_death_system);
        let packets = letters(&mut w);
        assert_eq!(packets.len(), 1, "the isolated chain must seal a packet");
        assert!(
            packets[0].dist(origin + Vec2::new(12.0, 0.0)) < 0.5,
            "the envelope lies where the host died"
        );

        // A body steps up to the envelope: it tears open and the chain resumes.
        let opener = w.spawn_enemy(EnemyKind::Brute, packets[0] + Vec2::new(5.0, 0.0), 400.0);
        run(&mut w, dead_letter_system);
        assert!(letters(&mut w).is_empty(), "the packet is spent on opening");
        let resumed = mark_of(&w, opener).expect("the chain resumes on the opener");
        assert_eq!(resumed.ticks_left, crate::game::weapon::VIRUS_TICKS);
        assert_eq!(resumed.jumps_left, 1, "the reopen spent one jump");
        assert!(resumed.has_visited(mark.visits[0]), "the walk keeps its history");

        // That host dies isolated too: the chain has already sealed once.
        w.ecs.get_mut::<Enemy>(opener).unwrap().hp = 0.0;
        run(&mut w, weapons_wave3a::hantavirus_death_system);
        assert!(
            letters(&mut w).is_empty(),
            "design §3: one packet per chain"
        );
    }

    /// A packet nobody opens is lost after its 2 s fuse (and draws no packet
    /// art afterwards).
    #[test]
    fn dead_letter_packets_expire_after_their_fuse() {
        use crate::game::ecs::systems::weapons_wave3a;
        let mut w = evolved_world(WeaponKind::Hantavirus);
        let origin = w.player.pos;
        let host = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(12.0, 0.0), 400.0);
        infect_by_dropping(&mut w, host);
        w.ecs.get_mut::<Enemy>(host).unwrap().hp = 0.0;
        run(&mut w, weapons_wave3a::hantavirus_death_system);
        assert_eq!(letters(&mut w).len(), 1);
        // Nobody comes: hold the fuse.
        let ticks = (DEAD_LETTER_SECS * 60.0) as usize;
        for _ in 0..ticks / 2 {
            run(&mut w, dead_letter_system);
        }
        assert_eq!(letters(&mut w).len(), 1, "the envelope still waits");
        for _ in 0..ticks {
            run(&mut w, dead_letter_system);
        }
        assert!(letters(&mut w).is_empty(), "an unopened packet is lost");
        assert_eq!(
            w.ecs.query::<&Infection>().iter(&w.ecs).filter(|i| i.active()).count(),
            0,
            "no chain came back from the lost envelope"
        );
    }

    /// **Carbon Copy**: the previous sweep replays from its own origin, at half
    /// damage — both beams, the same inward-closing pair — and only one copy is
    /// kept.
    #[test]
    fn carbon_copy_replays_the_last_sweep_with_half_damage_and_falloff() {
        use crate::game::ecs::systems::weapons_wave3b;
        use crate::game::weapon::LASER_EYE_OFFSET;
        let mut w = evolved_world(WeaponKind::LaserEyes); // L5: 10 damage inside 58 px
        let origin = w.player.pos;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons[0].timer = 0.0;
        // Sweep 1: nobody in the way, so it only records the geometry.
        let mut ticks = 0;
        while ticks < 200 {
            run(&mut w, weapons_wave3b::laser_eyes_system);
            ticks += 1;
            if ticks == 1 {
                w.player.pos += Vec2::new(30.0, 0.0);
            }
            let fx = *w.ecs.resource::<LaserFx>();
            if ticks > 1 && !fx.active() {
                break;
            }
        }
        let recorded = w.ecs.resource::<LaserFx>().copy.expect("a completed sweep is kept");
        assert_eq!(recorded.origin, origin + Vec2::new(LASER_EYE_OFFSET, 0.0));
        assert!((recorded.facing.x - 1.0).abs() < 1e-4);
        assert_eq!(recorded.full_damage_radius, 58.0);

        // The rat turns and walks out of range; the afterimage stays where the
        // recorded sweep fired from, and the live beam can no longer reach the
        // body planted on the recorded beam's line.
        w.player.pos = origin + Vec2::new(0.0, 90.0);
        w.player.face = Vec2::new(0.0, 1.0);
        let second_cast = w.player.pos + Vec2::new(0.0, LASER_EYE_OFFSET);
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(20.0, 0.0), 400.0);
        let far = w.spawn_enemy(EnemyKind::Brute, recorded.origin + Vec2::new(116.0, 0.0), 400.0);
        let hp = hp_of(&w, body);
        let far_hp = hp_of(&w, far);
        w.player.loadout.weapons[0].timer = 0.0;
        let mut ticks = 0;
        while ticks < 200 {
            run(&mut w, weapons_wave3b::laser_eyes_system);
            ticks += 1;
            let fx = w.ecs.resource::<LaserFx>();
            if fx.copy_active() {
                assert_eq!(fx.copy.unwrap().origin, recorded.origin);
                assert!(fx.copy_beams.iter().all(|&len| len > 0.0));
            }
            if ticks == 1 {
                assert!(fx.copy_active());
                w.player.pos += Vec2::new(30.0, 0.0);
            } else if !fx.active() && !fx.copy_active() {
                break;
            }
        }
        // Half of one sweep's hit (one pulse = 10, so 5).
        assert!(
            (hp - hp_of(&w, body) - 5.0).abs() < 0.01,
            "the copy deals half of one sweep hit: {:.1} -> {:.1}",
            hp,
            hp_of(&w, body)
        );
        assert_eq!(far_hp - hp_of(&w, far), 1.25, "twice the radius takes a quarter hit");
        // One copy is kept, and the newest completed sweep replaces it.
        let kept = w.ecs.resource::<LaserFx>().copy.expect("still one copy");
        assert_eq!(kept.origin, second_cast, "keep the newest sweep's cast origin");
    }

    /// **Ripcord**: the dash's real route stays taut for 0.6 s; each body on it
    /// takes one dash hit and slides 8 px toward the departure point; the boss
    /// takes damage only; then the cord snaps and stops hurting.
    #[test]
    fn ripcord_holds_the_route_taut_then_snaps_with_its_slide_and_boss_rule() {
        use crate::game::ecs::systems::boss::{Boss, BossKind};
        use crate::game::ecs::systems::player;
        use crate::game::ecs::systems::weapons_wave3b;
        let mut w = evolved_world(WeaponKind::SuperDash); // L5: 38 dmg, 6 px radius
        let origin = w.player.pos;
        let (cx, cy) = crate::game::collision::cell_of(origin);
        w.level.tiles.fill_cells(
            cx + 1,
            cy - 1,
            1,
            3,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 0.0;
        let body = w.spawn_enemy(EnemyKind::Brute, origin + Vec2::new(10.0, 2.0), 400.0);
        let hp0 = hp_of(&w, body);

        let dash_tick = |w: &mut World, dash: bool, move_dir: Vec2| {
            w.begin_tick(DT, move_dir, dash);
            w.ecs.run_system_cached(player::player_system).unwrap();
            w.ecs.run_system_cached(weapons_wave3b::super_dash_system).unwrap();
            w.ecs.run_system_cached(ripcord_system).unwrap();
            w.end_tick();
        };

        // The dash itself.
        dash_tick(&mut w, true, Vec2::new(1.0, 0.0));
        assert!(w.player.dash_active > 0.0, "the dash did not start");
        assert!(hp_of(&w, body) < hp0, "the dash hits what it crosses");
        let after_dash = hp_of(&w, body);
        // Walk out the rest of the dash (it never hits the same body twice).
        let mut guard = 0;
        while w.player.dash_active > 0.0 && guard < 40 {
            dash_tick(&mut w, false, Vec2::new(1.0, 0.0));
            guard += 1;
        }
        assert_eq!(hp_of(&w, body), after_dash, "one dash hit per body");
        assert_eq!(w.player.dash_active, 0.0, "the dash is over");
        let track = w.ecs.resource::<DashTrack>();
        assert!(track.segment.is_none(), "the wall should stop movement early");
        assert_eq!(track.route_life, RIPCORD_SECS, "the wall must not start the fuse early");

        // The cord: one more dash hit, and the body is dragged back toward the
        // departure point.
        let before_cord = hp_of(&w, body);
        let pos_before = w.ecs.get::<Enemy>(body).unwrap().pos;
        dash_tick(&mut w, false, Vec2::ZERO);
        assert!(hp_of(&w, body) < before_cord, "the taut route is a dash hit");
        let pos_after = w.ecs.get::<Enemy>(body).unwrap().pos;
        let pulled = pos_before - pos_after;
        assert!(pulled.x > 0.5, "the cord slides the body back toward the start: {pulled:?}");
        assert!(
            pulled.len() <= RIPCORD_SLIDE + 0.5,
            "8 px at most: {}",
            pulled.len()
        );

        // The boss takes the damage and does not move.
        let at = w.ecs.resource::<DashTrack>().route()[0];
        let mid = at.0 + (at.1 - at.0) * 0.5;
        let arena = w.level.boss_room().map(|r| r.rect).unwrap_or_else(|| {
            crate::game::level::TileRect::new(0, 0, 10, 10)
        });
        let boss_hp = 5000.0;
        let boss = Boss::new(BossKind::Cat, mid, boss_hp, arena, mid, false);
        let boss_entity = w.ecs.spawn((boss, SpawnId(999))).id();
        let boss_pos = w.ecs.get::<Boss>(boss_entity).unwrap().pos;
        // A fresh cord for the boss (the body's cord already spent itself on the
        // boss flag? no: the flag is per cord, so use the live one).
        dash_tick(&mut w, false, Vec2::ZERO);
        let boss_hp_now = w.ecs.get::<Boss>(boss_entity).unwrap().hp;
        assert!(boss_hp_now < boss_hp, "the cord must hurt the boss");
        assert_eq!(
            w.ecs.get::<Boss>(boss_entity).unwrap().pos,
            boss_pos,
            "bosses take damage only — no slide"
        );
        assert!(w.ecs.resource::<DashTrack>().route_boss_hit, "once per cord");
        dash_tick(&mut w, false, Vec2::ZERO);
        assert_eq!(
            w.ecs.get::<Boss>(boss_entity).unwrap().hp,
            boss_hp_now,
            "the boss takes one cord hit, not one per tick"
        );

        // 0.6 s after the dash, the cord snaps: nothing on the old route is hit.
        let mut snapped = false;
        for _ in 0..(RIPCORD_SECS * 60.0) as usize + 6 {
            dash_tick(&mut w, false, Vec2::ZERO);
            if w.ecs.resource::<DashTrack>().route().is_empty() {
                snapped = true;
                break;
            }
        }
        assert!(snapped, "the cord must snap after 0.6 s");
        let inside = w.spawn_enemy(EnemyKind::Brute, mid, 400.0);
        let inside_hp = hp_of(&w, inside);
        dash_tick(&mut w, false, Vec2::ZERO);
        assert_eq!(hp_of(&w, inside), inside_hp, "a snapped cord hurts nobody");
        assert_eq!(
            w.ecs.get::<Boss>(boss_entity).unwrap().hp,
            boss_hp_now,
            "and the boss is free too"
        );
    }

    /// The evolved forms honour the same passive hooks the base weapons do:
    /// Lens Cap (reach) and Long Fuse (zone lifetime, splash radius). Each pair
    /// is measured bare and with every stack, so a fixed constant cannot pass.
    #[test]
    fn evolved_reaches_and_zone_lives_follow_lens_cap_and_long_fuse() {
        use crate::game::ecs::systems::weapons;
        use crate::game::loadout::{PassiveKind, Upgrade};
        let stack = |w: &mut World, p: PassiveKind| {
            for _ in 0..p.max_stacks() {
                w.player.loadout.apply(&Upgrade::Passive(p));
            }
        };

        // Milk Teeth: the dart's flight is the base reach under Lens Cap.
        for lens in [false, true] {
            let mut w = evolved_world(WeaponKind::Gnaw);
            if lens {
                stack(&mut w, PassiveKind::LensCap);
            }
            let want = w.player.loadout.stats.ranged_reach(MILK_REACH);
            w.spawn_enemy(EnemyKind::Brute, w.player.pos + Vec2::new(20.0, 0.0), 400.0);
            w.player.loadout.weapons[0].timer = 0.0;
            run(&mut w, weapons::fire_weapons_system);
            let mut q = w.ecs.query::<&MilkTooth>();
            let dart = q.iter(&w.ecs).next().expect("the evolved Gnaw throws one dart");
            assert!((dart.reach - want).abs() < 1e-5, "milk reach {lens}: {}", dart.reach);
            assert_eq!(lens, want > MILK_REACH, "only Lens Cap stretches it");
        }

        // Fondue Fork: the tines plant inside the base reach under Lens Cap.
        for lens in [false, true] {
            let mut w = evolved_world(WeaponKind::CheeseSpray);
            if lens {
                stack(&mut w, PassiveKind::LensCap);
            }
            let want = w.player.loadout.stats.ranged_reach(TINE_REACH);
            w.spawn_enemy(EnemyKind::Brute, w.player.pos + Vec2::new(30.0, 0.0), 400.0);
            w.player.face = Vec2::new(1.0, 0.0);
            w.player.loadout.weapons[0].timer = 0.0;
            run(&mut w, weapons::fire_weapons_system);
            // The tines fly: run them out until the pair plants its string.
            for _ in 0..90 {
                run(&mut w, fondue_tines_system);
            }
            let mut q = w.ecs.query::<&CheeseString>();
            let line = q.iter(&w.ecs).next().expect("a string is planted").clone();
            let reach = line.a.dist(w.player.pos).max(line.b.dist(w.player.pos));
            assert!(reach <= want + 0.5, "fondue reach {lens}: {reach} > {want}");
            assert_eq!(lens, want > TINE_REACH, "only Lens Cap stretches it");
        }

        // Fairy Ring: Lens Cap widens the ring, Long Fuse keeps it alive.
        for (lens, fuse) in [(false, false), (true, true)] {
            let mut w = evolved_world(WeaponKind::SporeOrbit);
            if lens {
                stack(&mut w, PassiveKind::LensCap);
                stack(&mut w, PassiveKind::LongFuse);
            }
            let stats = w.player.loadout.stats;
            let want_r = stats.melee_reach(FAIRY_RADIUS);
            let want_life = stats.zone_life(FAIRY_SECS);
            w.player.loadout.weapons[0].timer = 0.0;
            run(&mut w, weapons::fire_weapons_system);
            w.player.loadout.weapons[0].timer = 0.0;
            run(&mut w, weapons::fire_weapons_system);
            let mut q = w.ecs.query::<&FairyRing>();
            let ring = q.iter(&w.ecs).next().expect("the second cast leaves a ring").clone();
            assert!((ring.radius - want_r).abs() < 1e-4, "fairy radius: {}", ring.radius);
            assert!((ring.total - want_life).abs() < 1e-4, "fairy life: {}", ring.total);
            if fuse {
                assert!(ring.total > FAIRY_SECS, "Long Fuse must stretch it");
            }
        }

        // Drain Cleaner: Lens Cap stretches the stripe (then masonry truncates
        // it) and Long Fuse stretches its settled phase — read through the
        // behaviour, not the stored numbers.
        for (lens, fuse) in [(false, false), (true, true)] {
            let mut w = evolved_world(WeaponKind::AcidSpit);
            if lens {
                stack(&mut w, PassiveKind::LensCap);
                stack(&mut w, PassiveKind::LongFuse);
            }
            let origin = w.player.pos;
            let dir = Vec2::new(1.0, 0.0);
            let reach = w.player.loadout.stats.ranged_reach(CHANNEL_LEN);
            // A body past the base 36 px stripe (the stripe's test tolerates a
            // body's own radius): dry bare, burnt stretched.
            let past = w.spawn_enemy(EnemyKind::Skitterer, origin + Vec2::new(41.0, 0.0), 400.0);
            let past_hp = hp_of(&w, past);
            spawn_with_id(
                &mut w.ecs,
                CorrosiveChannel::new(origin, dir, CHANNEL_LEN, 20.0, 20.0),
            );
            let mut life = 0.0f32;
            for _ in 0..40 {
                run(&mut w, drain_cleaner_system);
                life += DT;
            }
            if reach > CHANNEL_LEN {
                assert!(hp_of(&w, past) < past_hp, "Lens Cap must stretch the stripe");
            } else {
                assert_eq!(hp_of(&w, past), past_hp, "36 px is the bare stripe");
            }
            // The settled phase lasts a Long Fuse-stretched 2 s.
            while w.ecs.query::<&CorrosiveChannel>().iter(&w.ecs).count() > 0 && life < 6.0 {
                run(&mut w, drain_cleaner_system);
                life += DT;
            }
            let want = w.player.loadout.stats.zone_life(CHANNEL_SETTLE);
            assert!(
                (life - (CHANNEL_ADVANCE + want)).abs() < 0.15,
                "channel life {life:.2}s, wanted about {:.2}s (long fuse = {fuse})",
                CHANNEL_ADVANCE + want
            );
        }
    }

    /// Fairy Ring's "every second cast" parity is per **run**, not per system
    /// instance: the schedule and a cached single-system call share it, and a
    /// fresh world starts at zero.
    #[test]
    fn fairy_ring_parity_is_a_per_run_resource() {
        use crate::game::ecs::systems::weapons;
        let mut w = evolved_world(WeaponKind::SporeOrbit);
        assert_eq!(w.ecs.resource::<EvoRuntime>().fairy_casts, 0, "a run starts at zero");
        let rings = |w: &mut World| w.ecs.query::<&FairyRing>().iter(&w.ecs).count();

        w.player.loadout.weapons[0].timer = 0.0;
        run(&mut w, weapons::fire_weapons_system);
        assert_eq!(w.ecs.resource::<EvoRuntime>().fairy_casts, 1);
        assert_eq!(rings(&mut w), 0, "the odd cast leaves no ring");

        // The second cast goes through the full schedule: the shared counter
        // must see the first cast made through the cached system.
        w.player.loadout.weapons[0].timer = 0.0;
        w.update(DT, Vec2::ZERO, false);
        assert_eq!(w.ecs.resource::<EvoRuntime>().fairy_casts, 2);
        assert_eq!(rings(&mut w), 1, "every second cast leaves its ring");

        // …and a new run counts from zero again (the ring would otherwise land
        // on the wrong parity for the rest of a replayed level).
        let mut fresh = evolved_world(WeaponKind::SporeOrbit);
        assert_eq!(fresh.ecs.resource::<EvoRuntime>().fairy_casts, 0);
        fresh.player.loadout.weapons[0].timer = 0.0;
        run(&mut fresh, weapons::fire_weapons_system);
        assert_eq!(rings(&mut fresh), 0, "the first cast of a run leaves no ring");
    }

    /// A chest evolves each of the four wave-3 rows (and the card names it).
    #[test]
    fn a_chest_evolves_the_four_new_rows() {
        use crate::game::loadout::{open_chest, ChestReward, PassiveKind};
        use rand::SeedableRng;
        let rows = [
            (WeaponKind::AcidRain, PassiveKind::LongFuse, "Downspout"),
            (WeaponKind::Hantavirus, PassiveKind::SecondScent, "Dead Letter"),
            (WeaponKind::LaserEyes, PassiveKind::LensCap, "Carbon Copy"),
            (WeaponKind::SuperDash, PassiveKind::SpringHeel, "Ripcord"),
        ];
        for (kind, partner, name) in rows {
            let mut weapon = Weapon::new(kind);
            weapon.level = kind.max_level();
            let mut lo = Loadout::from_parts(vec![weapon], vec![(partner, 1)]);
            let mut rng = rand::rngs::StdRng::seed_from_u64(9);
            match open_chest(&mut lo, &mut rng) {
                ChestReward::Evolve { slot, kind: got, evo, partner_stacks } => {
                    assert_eq!(slot, 0);
                    assert_eq!(got, kind);
                    assert_eq!(evo.name, name);
                    assert_eq!(evo.passive, partner);
                    assert_eq!(partner_stacks, 1);
                }
                other => panic!("{kind:?}: expected an evolution, got {other:?}"),
            }
            assert!(lo.weapons[0].evolved, "{kind:?} must be flagged evolved");
            assert_eq!(lo.weapons[0].name(), name, "{kind:?} wears the chest name");
        }
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
        let mut dart = MilkTooth::new(Vec2::ZERO, Vec2::ZERO, 1.0, MILK_REACH);
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
    /// "Bosses take damage only" covers a named miniboss too: the cord hurts
    /// it once and never drags it, while an ordinary body on the same cord is
    /// pulled toward the departure point.
    #[test]
    fn ripcord_hurts_a_miniboss_without_sliding_it() {
        use crate::game::ecs::resources::DashTrack;
        use crate::game::ecs::systems::miniboss::MinibossKind;
        use crate::game::loadout::Loadout;
        use crate::game::weapon::Weapon;
        let mut w = World::new(Theme::Sewer, 31);
        w.clear_enemies();
        let p = w.player.pos;
        let mut weapon = Weapon::new(WeaponKind::SuperDash);
        weapon.level = WeaponKind::SuperDash.max_level();
        weapon.evolved = true;
        w.player.loadout = Loadout::from_parts(vec![weapon], Vec::new());
        let boss_at = p + Vec2::new(20.0, 0.0);
        let body_at = p + Vec2::new(34.0, 0.0);
        let boss = w.spawn_miniboss(MinibossKind::Corkscrew, 3, boss_at);
        let body = w.spawn_enemy(EnemyKind::Skitterer, body_at, 400.0);
        let (boss_hp, body_hp) = (
            w.ecs.get::<Enemy>(boss).unwrap().hp,
            w.ecs.get::<Enemy>(body).unwrap().hp,
        );
        {
            let mut track = w.ecs.resource_mut::<DashTrack>();
            track.segment = None;
            track.extend_route(p + Vec2::new(-10.0, 0.0), p + Vec2::new(48.0, 0.0));
        }
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(ripcord_system).expect("ripcord");
        w.end_tick();
        let b = w.ecs.get::<Enemy>(boss).unwrap();
        assert!(b.hp < boss_hp, "the cord did not hurt the miniboss");
        assert_eq!(b.pos, boss_at, "the cord dragged a miniboss");
        let e = w.ecs.get::<Enemy>(body).unwrap();
        assert!(e.hp < body_hp, "the cord did not hurt the ordinary body");
        assert!(
            e.pos.x < body_at.x - RIPCORD_SLIDE * 0.5,
            "an ordinary body was not dragged toward the departure point: {:?}",
            e.pos
        );
    }

    /// A Dead Letter packet opens on the first eligible foe, and a named
    /// miniboss is one: the chain resumes on it and ticks it.
    #[test]
    fn a_miniboss_can_open_a_dead_letter_packet() {
        use crate::game::ecs::components::{DeadLetterPacket, Infection};
        use crate::game::ecs::resources::spawn_with_id;
        use crate::game::ecs::systems::miniboss::MinibossKind;
        let mut w = World::new(Theme::Sewer, 33);
        w.clear_enemies();
        let p = w.player.pos;
        let at = p + Vec2::new(40.0, 0.0);
        let boss = w.spawn_miniboss(MinibossKind::BellDiver, 1, at);
        let packet = spawn_with_id(
            &mut w.ecs,
            DeadLetterPacket {
                pos: at + Vec2::new(DEAD_LETTER_REACH - 2.0, 0.0),
                life: DEAD_LETTER_SECS,
                total: DEAD_LETTER_SECS,
                chain: Infection::new(u64::MAX - 3, 2, 30.0, 6.0),
            },
        );
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(dead_letter_system).expect("dead letter");
        w.end_tick();
        assert!(w.ecs.get::<DeadLetterPacket>(packet).is_none(), "the packet stayed sealed");
        let mark = w.ecs.get::<Infection>(boss).expect("the chain resumed on the miniboss");
        assert_eq!(mark.jumps_left, 1, "opening a packet spends one jump");
        assert!(mark.active());
    }
}
