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

use crate::game::ecs::resources::{NextSpawnId, RngRes, RunCounters, TickInput};
use crate::game::enemy::EnemyKind;
use crate::game::gem::PickupKind;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
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
};

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

    // ---- Drop weights ----------------------------------------------------

    /// The weighted table is the design's (Heal 32, Magnet 18, Nuke 8, Bottle
    /// Cap 14, Wind-up Mouse 10, Pocket Watch 9, Clean Socks 9) and every kind
    /// is reachable.
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
            (PickupKind::Heal, 32.0),
            (PickupKind::Magnet, 18.0),
            (PickupKind::Nuke, 8.0),
            (PickupKind::BottleCap, 14.0),
            (PickupKind::WindupMouse, 10.0),
            (PickupKind::PocketWatch, 9.0),
            (PickupKind::CleanSocks, 9.0),
        ] {
            let got = pct(kind);
            assert!(
                (got - want).abs() < 0.6,
                "{kind:?} came up {got:.2}%, the design says {want}%"
            );
        }
        assert_eq!(counts.len(), 7, "every kind must be reachable");
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

        // The legacy table width and the new one leave the stream in the same
        // place (rand's `UniformInt` rejection never had to retry for 0..100).
        let mut old = Rng::seed_from_u64(9);
        let mut new = Rng::seed_from_u64(9);
        let _ = old.random_range(0..3u32);
        let _ = new.random_range(0..loot::PICKUP_WEIGHT_TOTAL);
        assert_eq!(
            old.random::<u64>(),
            new.random::<u64>(),
            "the wider pickup table changed how much rng a drop consumes"
        );
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
