//! Boss fight framework (P7).
//!
//! One shared tick system drives the three themed campaign bosses:
//!
//! | level | theme | boss | art module |
//! |-------|-------|------|-----------|
//! | 0 | Sewer | Jungle Cat (Apex Jaguar) | [`crate::game::art_boss_cat`] |
//! | 1 | Kitchen | The Sweeper (broom) | [`crate::game::art_boss_broom`] |
//! | 2 | Lab | TURF-REAPER MK.IV (mower) | [`crate::game::art_boss_mower`] |
//!
//! The art files are the gameplay spec: their view structs, state enums,
//! per-state timings/damage and hit-shape helpers (`body_hit_circles`,
//! `foot_hitboxes`, `collision_radius`, `BODY_RADIUS`, ...) are binding. This
//! module owns the shared machinery; each per-boss file owns only its
//! transitions and attack definitions.
//!
//! # API the three boss workers implement
//!
//! A boss file contains a zero-sized controller implementing [`BossController`]:
//!
//! ```ignore
//! pub struct CatController;
//!
//! impl BossController for CatController {
//!     fn kind(&self) -> BossKind { BossKind::Cat }
//!
//!     fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
//!         // `boss.state` is the art enum; `boss.progress()` is 0..=1.
//!         // Draw only from `ctx.rng` — never the main run rng.
//!         match boss.state {
//!             BossState::Cat(BossCatState::Stalk) => {
//!                 if move_boss(ctx, boss, ctx.seek(boss), art_boss_cat::STALK_SPEED_P1) {
//!                     boss.facing = ctx.seek(boss);
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```
//!
//! `BossContext` gives the player position/velocity, the arena rectangle,
//! `TileMap` queries (`footprint_fits`, `los_clear`, `attack_fits`,
//! `nearest_open`), the large-class flow field (`seek`) and the boss rng.
//! `BossOutput` collects projectiles and phase/death notifications.
//!
//! # Shared services
//!
//! * [`move_boss`] — movement with large-body tile collision (axis slide).
//! * [`BossContext::attack_fits`] — "does this attack footprint fit, and is
//!   the player's escape lane clear?".
//! * [`BossHazard`] — timed circles/annuli/arcs/rects/corridors with a warn
//!   window then an active window; resolved against the player (damage +
//!   optional knockback) and against regular enemies when requested.
//! * [`BossProjectile`] — boss-owned projectiles with wall bouncing.
//! * [`Boss::invuln`] / [`Boss::damage_mult`] — vulnerable/invulnerable windows.
//! * [`Boss::hit_shapes`] — the art-defined hit shapes for player damage.
//! * [`Boss::target_points`] — the targetable points for auto-aim (P4).
//!
//! # Determinism
//!
//! The boss owns a separate rng stream (`BossRng`, seeded from the run seed and
//! the kind's salt), so boss behaviour never perturbs the main rng order. The
//! shared system iterates entities in `SpawnId` order where order matters and
//! allocates nothing per tick after warm-up (`BossFight::scratch` and the
//! `Local` output buffer are reused).
//!
//! # Spawns and despawns (the `EnemyIndex` validity window)
//!
//! Audited, because `enemy_move` publishes a `SpawnId`-ordered `EnemyIndex`
//! that every combat system walks and `resources::debug_assert_enemy_index`
//! polices: between `enemy_move::grid_build_system` and the last reader
//! (`combat::contact_system`), **no `Enemy` entity may be spawned or
//! despawned**. What the boss half of the schedule does there:
//!
//! * [`boss_spawn_system`] runs before the grid build and spawns one `Boss`
//!   entity — the boss is *not* an `Enemy` (it carries `Boss` + `SpawnId`, and
//!   `World::boss_hp` has a legacy `EnemyKind::Boss` fallback only for tests).
//! * [`boss_damage_system`] spawns and despawns nothing; it damages the boss.
//! * [`boss_update_system`] spawns `BossProjectile` entities (never `Enemy`),
//!   despawns spent/bounced-out projectiles and the boss at the end of its
//!   death beat, and **raises [`BossFight::horde_break`]** rather than
//!   despawning the horde; the removal is `loot::reap_enemies_system`'s, after
//!   the last index reader.
//! * [`resolve_hazards`] only *damages* enemies (`hit_enemies` hazards such as
//!   the mower's ram and the broom's sweep); corpses are removed by the normal
//!   reap, after the window.
//! * The three controllers (`cat`, `broom`, `mower`) take no `Commands` at all:
//!   they move the boss, push hazards and queue projectiles/cues, so they
//!   cannot spawn or despawn an entity.
//!
//! `boss::tests::boss_death_breaks_the_horde_without_invalidating_the_enemy_index`
//! locks the whole thing down for all three bosses through the real schedule.

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::audio::Sfx;
use crate::game::gem::Gold;
use crate::game::art_boss_broom::{self, BroomBossState, BroomBossView};
use crate::game::art_boss_cat::{self, BossCatState, BossCatView};
use crate::game::art_boss_mower::{self, MowerState, MowerView};
use crate::game::art_boss_night_edition::{self, NightEditionState, NightEditionView};
use crate::game::art_traps::{
    local_box_contains, MousetrapState, MOUSETRAP_DAMAGE_ENEMY, MOUSETRAP_HIT_PROGRESS_MAX,
    MOUSETRAP_HIT_PROGRESS_MIN, MOUSETRAP_KILL_HALF_V, MOUSETRAP_KILL_U_MAX, MOUSETRAP_KILL_U_MIN,
};
use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{self, SizeClass, TileKind};
use crate::game::director;
use crate::game::ecs::components::{Pool, SpawnId};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{
    BossDue, DifficultyLevel, LevelIndex, NextEnemyId, NextSpawnId, RunCounters, Shake, SfxQueue, TickInput,
};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{Level, Rectf, Theme, TileRect};
use crate::game::nav::FlowField;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::game::particle::vfx;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;
use crate::render::palette::Rgb;

pub mod broom;
pub mod cat;
pub mod mower;
pub mod night_edition;

// ============================================================================
// Kind, rng, resource
// ============================================================================

/// Which themed boss this is (one per campaign level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossKind {
    /// Sewer — the Jungle Cat (Apex Jaguar).
    Cat,
    /// Kitchen — The Sweeper (broom).
    Broom,
    /// Lab — TURF-REAPER MK.IV (mower).
    Mower,
    /// Printworks — THE NIGHT EDITION (the walking offset press).
    NightEdition,
}

/// The boss the getaway ride fields until it gets a fight of its own.
///
/// The trailer is a machine on wheels with a road to hide in, so the Lab's
/// TURF-REAPER MK.IV reads best of the three shipped fights; `w3-bosses` owns
/// the real chase and only has to change this one function.
pub fn trailer_boss() -> BossKind {
    BossKind::Mower
}

impl BossKind {
    /// The boss for a floor.
    ///
    /// **The wave 3 seam.** This is the one function that decides which boss a
    /// theme spawns (`boss_spawn_system` calls it, not `for_level`), so a floor
    /// with its own fight fields it and every other floor meets the same boss
    /// `for_level` gives. The Laundry still borrows the Kitchen's broom until
    /// `BossKind::Mangle` lands; the Printworks fields the Night Edition, with
    /// its own arena in `levelgen::layout::furnish_arena`. Nothing else maps
    /// themes to bosses.
    #[inline]
    pub fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Sewer => BossKind::Cat,
            Theme::Kitchen => BossKind::Broom,
            Theme::Lab => BossKind::Mower,
            Theme::Laundry => BossKind::Broom,
            Theme::Printworks => BossKind::NightEdition,
            Theme::Trailer => trailer_boss(),
        }
    }

    /// The boss for a campaign level slot (0 Sewer, 1 Kitchen, 2 Lab).
    #[inline]
    pub fn for_level(level: usize) -> Self {
        match level {
            0 => BossKind::Cat,
            1 => BossKind::Broom,
            _ => BossKind::Mower,
        }
    }

    /// The art's recommended base HP pool (before the director's `hp_mult`).
    #[inline]
    pub fn base_hp(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::BOSS_BASE_HP,
            BossKind::Broom => 1600.0,
            BossKind::Mower => 1600.0,
            BossKind::NightEdition => art_boss_night_edition::BOSS_BASE_HP,
        }
    }

    /// Single-circle collision radius used for movement and contact.
    #[inline]
    pub fn body_radius(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::BODY_RADIUS,
            BossKind::Broom => art_boss_broom::STANCE_HALF_WIDTH + art_boss_broom::FOOT_HIT_RADIUS,
            BossKind::Mower => art_boss_mower::BODY_RADIUS,
            BossKind::NightEdition => art_boss_night_edition::BODY_RADIUS,
        }
    }

    /// Balance multiplier applied on top of the art's base pool and the
    /// director's curve, so the fight lasts the intended beat for the build the
    /// campaign actually produces.
    ///
    /// Measured with `boss::tests::measure_campaign_boss_dps`
    /// (`RATRUN_BOSS_DPS=1 cargo test boss::tests::measure_campaign_boss_dps --
    /// --nocapture`): the scripted campaign (auto-pick the first card, flee/kite
    /// script) plays each level to [`director::BOSS_TIME`], then spends 60 s
    /// circling the real, unkillable boss while `boss_damage_system` books
    /// every point of damage by source ([`BossDamageTally`]). The horde is
    /// present and competes for the weapons' aim, so the boss gets whatever the
    /// nearest-body pick gives it. Retune here and nowhere else.
    ///
    /// **How these were measured (b3/b4 balance pass).** The self-playing bot
    /// on `--bot-batch 30 --seed 900` at all three skills, 126 *completed*
    /// boss fights, with `RATRUN_BOSS_TRACE` recording the real fight clock
    /// (spawn to death, including the rat's walk to the arena) and the damage
    /// actually booked into the boss. Pooled fight length per boss before this
    /// tuning: Cat median 63 s (p10 52, p90 104), Sweeper 85 s (70/116), Mower
    /// 84 s (66/117). The scales below are each pool moved by the factor that
    /// lands the *p90* at ~90 s and the p10 near 45-55 s (the scale is applied
    /// on top of the art/director pool, so the fight scales with 1/scale).
    ///
    /// The residual spread is real and is *build*, not boss: the rat's measured
    /// damage into a boss runs 78-270 dps at level 1, 220-480 at level 2 and
    /// 260-610 at level 3, tracking the player's level at the boss (13-31 at
    /// level 1). A fixed pool cannot do better than a ~2x band against that;
    /// the early-game workers' XP/passive tuning is what narrows it.
    ///
    /// Only 10-20% of the damage into a boss is aimed (Gnaw / Cheese Spray);
    /// the rest is Tail Whip, Squeak Nova / Spore Orbit and acid pools, because
    /// the rat fights at 30-60 px where those weapons reach — a kiting rat at
    /// 100+ px lands only the long-range share, which is where the slow tail
    /// comes from.
    ///
    /// **Batch-4 re-scale (merged campaign).** With all five balance branches
    /// in one build the medians moved: 600 campaign runs per skill measured
    /// level-1/2/3 fights at 57 / 89 / 107 s (median) with a 15-215 s spread,
    /// and the 45-90 s target was missed at both ends. The clock offsets of the
    /// later levels (120 s/150 s) raise `hp_mult_for`, so the same scale buys a
    /// bigger pool on levels 2 and 3 than it did on the later-levels branch
    /// alone; the Cat's pool was also small enough that a top build killed it
    /// in 15-18 s. Re-scaled from the merged measurement: Broom 2.45 → 1.95 and
    /// Mower 2.60 → 2.00 (the later levels' clock offsets inflated their pools
    /// by ~15 % on top of the scale); the Cat's 1.40 was left alone because the
    /// level-1 clear rate is Cat-limited and the pool was not the problem there.
    /// Medians land at 57 / 72 / 80 s (see the batch4 report) with the floor at
    /// ~26 s.
    #[inline]
    pub fn hp_scale(self) -> f32 {
        match self {
            BossKind::Cat => 1.4,
            BossKind::Broom => 1.95,
            BossKind::Mower => 2.0,
            // `WAVE3_DESIGN.md` §12 states the Night Edition's pool as the
            // *final encounter* number (3,000), so the art constant is that
            // whole design pool and this is the same 2.0 the other stage-3
            // fight uses: 3,000 × 2.0 × the framework's ×8 = 48,000, 1.25× the
            // Mangle's design pool and 1.875× the Mower's art base, which is
            // exactly the ratio §12 asks for (3,000 vs 2,400 vs the borrowed
            // 1,600). Measured on the same near-complete build and harness as
            // the others (`night_edition::tests::measure_press_fight`,
            // `RATRUN_PRESS_MEASURE=1`): 39.5–45.6 s across seven seeds, the bot
            // dodging every tell (0–1 avoidable hits per fight over 40 s). A
            // balance pass owns the final number.
            BossKind::NightEdition => 2.0,
        }
    }

    /// Body-contact damage (the art's per-kind number).
    #[inline]
    pub fn contact_damage(self) -> f32 {
        match self {
            // The art's 18 is the melee-number against a 100 HP rat; with the
            // b3-balance pass (130 HP, 0.9 s i-frames, 0.4 HP/s base regen) an
            // unread body slam was the single largest run-ender in the batch,
            // and the Cat is the *level 1* boss. 13 keeps the pounce scary and
            // survivable once.
            BossKind::Cat => 13.0,
            BossKind::Broom => 10.0,
            BossKind::Mower => 14.0,
            // The design's own number for this fight (`§12`: body contact 8).
            BossKind::NightEdition => art_boss_night_edition::CONTACT_DAMAGE,
        }
    }

    /// Death-state duration in seconds.
    #[inline]
    pub fn death_duration(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::DEATH_DURATION,
            BossKind::Broom => 2.40,
            BossKind::Mower => 3.4,
            BossKind::NightEdition => art_boss_night_edition::DEATH_DURATION,
        }
    }

    /// Phase index for an hp fraction (1 = healthiest).
    #[inline]
    pub fn phase_for(self, hp_frac: f32) -> u8 {
        match self {
            BossKind::Cat => {
                if hp_frac <= art_boss_cat::PHASE2_HP_FRAC {
                    2
                } else {
                    1
                }
            }
            BossKind::Broom => {
                if hp_frac <= 0.50 {
                    2
                } else {
                    1
                }
            }
            BossKind::Mower => {
                if hp_frac <= 0.33 {
                    3
                } else if hp_frac <= 0.66 {
                    2
                } else {
                    1
                }
            }
            // The design's thresholds: the page stamp above 65 %, the carriage
            // return from 65 to 30 %, the misprint below 30 %.
            BossKind::NightEdition => {
                if hp_frac <= 0.30 {
                    3
                } else if hp_frac <= 0.65 {
                    2
                } else {
                    1
                }
            }
        }
    }

    /// The neutral state a freshly spawned boss starts in.
    #[inline]
    pub fn initial_state(self) -> BossState {
        match self {
            BossKind::Cat => BossState::Cat(BossCatState::Stalk),
            BossKind::Broom => BossState::Broom(BroomBossState::Idle),
            BossKind::Mower => BossState::Mower(MowerState::Patrol),
            BossKind::NightEdition => BossState::NightEdition(NightEditionState::Roll),
        }
    }

    /// The first state's duration (the art's neutral-state beat).
    #[inline]
    pub fn initial_duration(self) -> f32 {
        match self {
            BossKind::Cat => 2.6,
            BossKind::Broom => 0.55,
            BossKind::Mower => 1.80,
            // The first walk beat is the phase-1 one (`night_edition::ROLL_DURATION[0]`).
            BossKind::NightEdition => 1.8,
        }
    }

    /// The art's death/defeat state.
    #[inline]
    pub fn death_state(self) -> BossState {
        match self {
            BossKind::Cat => BossState::Cat(BossCatState::Death),
            BossKind::Broom => BossState::Broom(BroomBossState::Defeat),
            BossKind::Mower => BossState::Mower(MowerState::Dying),
            BossKind::NightEdition => BossState::NightEdition(NightEditionState::Wreck),
        }
    }

    /// Constant salt mixed into the run seed for the boss rng stream.
    #[inline]
    pub fn seed_salt(self) -> u64 {
        match self {
            BossKind::Cat => 0x9E37_79B9_7F4A_7C15,
            BossKind::Broom => 0xC2B2_AE3D_27D4_EB4F,
            BossKind::Mower => 0x1656_67B1_9E37_79F9,
            BossKind::NightEdition => 0x7B1D_3C4F_9A2E_5D68,
        }
    }

    /// Culling margin for the art's largest sprite (world px).
    #[inline]
    pub fn cull_margin(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::BODY_LENGTH + art_boss_cat::TAIL_LENGTH,
            BossKind::Broom => art_boss_broom::CULL_MARGIN,
            BossKind::Mower => art_boss_mower::BODY_RADIUS + 20.0,
            BossKind::NightEdition => art_boss_night_edition::CULL_MARGIN,
        }
    }

    /// The zero-sized controller for this kind.
    #[inline]
    pub fn controller(self) -> &'static dyn BossController {
        match self {
            BossKind::Cat => &cat::CatController,
            BossKind::Broom => &broom::BroomController,
            BossKind::Mower => &mower::MowerController,
            BossKind::NightEdition => &night_edition::NightEditionController,
        }
    }
}

/// The boss's private rng stream.
///
/// Seeded from the run seed and the kind's salt so the boss never draws from
/// the main run rng (which would perturb the golden trace).
pub struct BossRng(pub StdRng);

impl BossRng {
    /// Seed the stream from the run seed.
    #[inline]
    pub fn for_run(seed: u64, kind: BossKind) -> Self {
        Self(StdRng::seed_from_u64(seed ^ kind.seed_salt()))
    }

    /// Draw a float in `0..1`.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        self.0.random::<f32>()
    }
}

impl Default for BossRng {
    fn default() -> Self {
        Self(StdRng::seed_from_u64(BossKind::Cat.seed_salt()))
    }
}

/// Reusable per-system scratch (no per-tick heap allocation after warm-up).
#[derive(Default)]
pub struct BossScratch {
    /// Bullet `(SpawnId, Entity)` order for the damage system.
    pub bullet_order: Vec<(u64, Entity)>,
    /// Enemy `(SpawnId, Entity)` order for hazard damage.
    pub enemy_order: Vec<(u64, Entity)>,
    /// Mousetrap indices already credited to the boss this snap.
    pub mouse_latch: Vec<u32>,
    /// This tick's `dt`, copied in by the damage system.
    pub dt: f32,
    /// Balance attribution: the damage the boss took **this tick**, split by
    /// source. All five are reset at the top of every [`boss_damage_system`]
    /// tick and are only read by `boss::tests::measure_campaign_boss_dps`; no
    /// other code reads them and they never feed the simulation, so they cannot
    /// affect determinism. The split answers "how much of the fight is the
    /// weapons aiming at the boss versus area damage it happens to stand in".
    pub dmg: BossDamageTally,
}

/// Per-tick damage attributed to the boss, by source (see
/// [`BossScratch::dmg`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct BossDamageTally {
    /// Bullets from a weapon that picks its own target: Gnaw, Cheese Spray.
    pub aimed: f32,
    /// Squeak Nova and Spore Orbit bullets (fired on a pattern around the rat,
    /// so they never aim at a body).
    pub pattern: f32,
    /// Acid Spit's landed pools (dps over the tick).
    pub pools: f32,
    /// Tail Whip's melee arc.
    pub whip: f32,
    /// Mousetrap snaps.
    pub traps: f32,
}

impl BossDamageTally {
    /// Accumulate another tick into this one (the measurement's running total).
    #[inline]
    pub fn add(&mut self, other: BossDamageTally) {
        self.aimed += other.aimed;
        self.pattern += other.pattern;
        self.pools += other.pools;
        self.whip += other.whip;
        self.traps += other.traps;
    }

    /// Everything that was not an aimed bullet.
    #[inline]
    pub fn incidental(&self) -> f32 {
        self.pattern + self.pools + self.whip + self.traps
    }

    /// The whole tick.
    #[inline]
    pub fn total(&self) -> f32 {
        self.aimed + self.incidental()
    }
}

/// Boss-fight state that lives permanently in the ECS world.
#[derive(Resource, Default)]
pub struct BossFight {
    /// The boss's private rng stream (re-seeded on spawn).
    pub rng: BossRng,
    /// The boss's damage-number identity (allocated from `NextEnemyId`).
    pub next_id: u32,
    /// Set once when the boss dies; the integrator/P8 reads it or
    /// `RunCounters::level_complete`.
    pub level_complete: bool,
    /// "The horde breaks this tick": raised by [`boss_update_system`] on the
    /// tick the boss dies, consumed by `loot::reap_enemies_system`.
    ///
    /// The horde break removes every live `Enemy` entity, and enemy removal
    /// that `SpawnId`-ordered index does not know about is only safe **after**
    /// the last `EnemyIndex` reader: `boss_update_system` runs before
    /// `combat::{player_bullet_hits, enemy_bullet_hits, contact}`, which walk
    /// the index `enemy_move::grid_refresh_system` built this tick (and
    /// `resources::debug_assert_enemy_index` checks that in debug builds).
    /// Despawning from here would leave it stale, so the boss only raises the
    /// flag and the removal happens in the reap slot, next to the Nuke
    /// re-reap — after the last reader.
    ///
    /// The cleared horde is *not* reaped: the break costs no kills, no score
    /// and no drops, which is what the pre-flag code did by despawning the
    /// enemies before `loot::reap_enemies_system` ever saw them.
    pub horde_break: bool,
    /// Reused scratch.
    pub scratch: BossScratch,
    /// The boss's position at the start of the current movement window (the
    /// rejoin's and the siege's "is it actually moving?" probe; see
    /// [`RELOCATE_AFTER_SECS`]).
    pub last_pos: Vec2,
    /// Seconds since [`Self::last_pos`] was sampled.
    pub since_sample: f32,
    /// Did the last 0.5 s window move the boss less than 3 px in its neutral
    /// beat? A per-*tick* test is not enough: a boss wedged on a corner
    /// oscillates a fraction of a pixel every tick (14 px/s of jitter, no
    /// progress) and would read as "moving" forever.
    pub stuck: bool,
    /// Seconds the boss has been walled off from the rat (see
    /// [`SIEGE_AFTER_SECS`]). Reset as soon as a wall between them is gone.
    pub siege: f32,
}

// ============================================================================
// State, hit shapes, targets, art view
// ============================================================================

/// The boss's current art state, tagged by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossState {
    Cat(BossCatState),
    Broom(BroomBossState),
    Mower(MowerState),
    NightEdition(NightEditionState),
}

/// One art-defined hit shape. Bosses are hit through these, not a single circle.
#[derive(Clone, Copy, Debug)]
pub enum BossHitShape {
    /// A circle, optionally grounded (lifted feet are not hit by ground weapons).
    Circle {
        center: Vec2,
        radius: f32,
        grounded: bool,
    },
    /// An oriented box (`half` along facing, `half_perp` across).
    Box {
        center: Vec2,
        facing: Vec2,
        half: f32,
        half_perp: f32,
        grounded: bool,
    },
}

impl BossHitShape {
    /// Does a circle at `point` with `radius` overlap this shape?
    #[inline]
    pub fn overlaps_circle(&self, point: Vec2, radius: f32) -> bool {
        match *self {
            BossHitShape::Circle {
                center,
                radius: r,
                ..
            } => center.dist_sq(point) <= (r + radius) * (r + radius),
            BossHitShape::Box {
                center,
                facing,
                half,
                half_perp,
                ..
            } => {
                let d = point - center;
                let u = d.dot(facing);
                let v = d.dot(facing.perp());
                u.abs() <= half + radius && v.abs() <= half_perp + radius
            }
        }
    }

    /// Does a swept segment hit this shape? (Approximated by sampling the
    /// segment; the art's shapes are small and the sweep is short.)
    #[inline]
    pub fn overlaps_segment(&self, from: Vec2, to: Vec2, radius: f32) -> bool {
        let steps = ((to - from).len() / 4.0).ceil().max(1.0) as u32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            if self.overlaps_circle(from + (to - from) * t, radius) {
                return true;
            }
        }
        false
    }

    /// The shape's center (targetable point / knockback origin).
    #[inline]
    pub fn center(&self) -> Vec2 {
        match *self {
            BossHitShape::Circle { center, .. } | BossHitShape::Box { center, .. } => center,
        }
    }

    /// Can ground weapons hit this shape?
    #[inline]
    pub fn grounded(&self) -> bool {
        match *self {
            BossHitShape::Circle { grounded, .. } | BossHitShape::Box { grounded, .. } => grounded,
        }
    }
}

/// Up to three art-defined hit shapes (cat: 3 circles, broom: 2 feet, mower: 1).
#[derive(Clone, Copy, Debug)]
pub struct BossHitShapes {
    shapes: [BossHitShape; 3],
    len: u8,
}

impl BossHitShapes {
    /// Empty set.
    #[inline]
    pub fn empty() -> Self {
        Self {
            shapes: [BossHitShape::Circle {
                center: Vec2::ZERO,
                radius: 0.0,
                grounded: false,
            }; 3],
            len: 0,
        }
    }

    /// Build from a slice (clamped to three).
    #[inline]
    pub fn from_slice(shapes: &[BossHitShape]) -> Self {
        let mut out = Self::empty();
        for (i, s) in shapes.iter().take(3).enumerate() {
            out.shapes[i] = *s;
            out.len = (i + 1) as u8;
        }
        out
    }

    /// Iterate the live shapes.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &BossHitShape> {
        self.shapes[..self.len as usize].iter()
    }

    /// Is any grounded shape hit by a swept segment?
    #[inline]
    pub fn segment_hits_grounded(&self, from: Vec2, to: Vec2, radius: f32) -> bool {
        self.iter()
            .any(|s| s.grounded() && s.overlaps_segment(from, to, radius))
    }

    /// Is any grounded shape hit by a circle?
    #[inline]
    pub fn circle_hits_grounded(&self, point: Vec2, radius: f32) -> bool {
        self.iter()
            .any(|s| s.grounded() && s.overlaps_circle(point, radius))
    }

    /// Is any shape hit by a circle (including lifted ones)?
    #[inline]
    pub fn circle_hits_any(&self, point: Vec2, radius: f32) -> bool {
        self.iter().any(|s| s.overlaps_circle(point, radius))
    }
}

/// Targetable points for auto-aim (worker P4).
#[derive(Clone, Copy, Debug)]
pub struct BossTargetable {
    points: [Vec2; 3],
    len: u8,
}

impl BossTargetable {
    /// Empty set.
    #[inline]
    pub fn empty() -> Self {
        Self {
            points: [Vec2::ZERO; 3],
            len: 0,
        }
    }

    /// Iterate the live points.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.points[..self.len as usize].iter().copied()
    }

    /// The first point, if any.
    #[inline]
    pub fn first(&self) -> Option<Vec2> {
        self.points.first().copied().filter(|_| self.len > 0)
    }

    /// Number of points.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Is the set empty?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A `Copy` snapshot of the art view for the current kind.
#[derive(Clone, Copy, Debug)]
pub enum BossArtView {
    Cat(BossCatView),
    Broom(BroomBossView),
    Mower(MowerView),
    NightEdition(NightEditionView),
}

// ============================================================================
// Hazard
// ============================================================================

/// The geometric footprint of a timed boss hazard.
#[derive(Clone, Copy, Debug)]
pub enum HazardShape {
    /// Filled circle.
    Circle { center: Vec2, radius: f32 },
    /// Ring between `inner` and `outer`.
    Annulus {
        center: Vec2,
        inner: f32,
        outer: f32,
    },
    /// Pie slice between `inner` and `outer`, `half_angle` either side of `facing`.
    Arc {
        center: Vec2,
        inner: f32,
        outer: f32,
        facing: Vec2,
        half_angle: f32,
    },
    /// Oriented rectangle.
    Rect {
        center: Vec2,
        facing: Vec2,
        half: f32,
        half_perp: f32,
    },
    /// Thick segment (corridor).
    Corridor {
        from: Vec2,
        to: Vec2,
        half_width: f32,
    },
}

impl HazardShape {
    /// The shape's center (knockback origin).
    #[inline]
    pub fn center(&self) -> Vec2 {
        match *self {
            HazardShape::Circle { center, .. }
            | HazardShape::Annulus { center, .. }
            | HazardShape::Arc { center, .. }
            | HazardShape::Rect { center, .. } => center,
            HazardShape::Corridor { from, to, .. } => (from + to) * 0.5,
        }
    }

    /// Does a circle at `point` with `radius` overlap the hazard?
    #[inline]
    pub fn contains(&self, point: Vec2, radius: f32) -> bool {
        match *self {
            HazardShape::Circle { center, radius: r } => {
                center.dist_sq(point) <= (r + radius) * (r + radius)
            }
            HazardShape::Annulus {
                center,
                inner,
                outer,
            } => {
                let d = center.dist(point);
                d >= (inner - radius).max(0.0) && d <= outer + radius
            }
            HazardShape::Arc {
                center,
                inner,
                outer,
                facing,
                half_angle,
            } => {
                let d = point - center;
                let dist = d.len();
                if dist < (inner - radius).max(0.0) || dist > outer + radius {
                    return false;
                }
                if dist <= radius {
                    return true;
                }
                let f = facing.normalized();
                if f.len_sq() <= 0.0 {
                    return true;
                }
                let cos = d.normalized().dot(f).clamp(-1.0, 1.0);
                cos >= half_angle.cos()
            }
            HazardShape::Rect {
                center,
                facing,
                half,
                half_perp,
            } => {
                let d = point - center;
                let u = d.dot(facing);
                let v = d.dot(facing.perp());
                u.abs() <= half + radius && v.abs() <= half_perp + radius
            }
            HazardShape::Corridor {
                from,
                to,
                half_width,
            } => {
                let ab = to - from;
                let t = if ab.len_sq() <= 0.0 {
                    0.0
                } else {
                    ((point - from).dot(ab) / ab.len_sq()).clamp(0.0, 1.0)
                };
                let closest = from + ab * t;
                closest.dist_sq(point) <= (half_width + radius) * (half_width + radius)
            }
        }
    }
}

/// A timed boss hazard: `warn` seconds of telegraph, then `active` seconds of
/// damage. Each target is damaged at most once per activation.
#[derive(Clone, Copy, Debug)]
pub struct BossHazard {
    pub shape: HazardShape,
    pub warn: f32,
    pub active: f32,
    pub elapsed: f32,
    pub damage: f32,
    pub knockback: f32,
    pub hit_player: bool,
    pub hit_enemies: bool,
    pub fired: bool,
    pub fired_enemies: bool,
}

impl BossHazard {
    /// A player-only hazard.
    #[inline]
    pub fn new(shape: HazardShape, warn: f32, active: f32, damage: f32) -> Self {
        Self {
            shape,
            warn,
            active,
            elapsed: 0.0,
            damage,
            knockback: 0.0,
            hit_player: true,
            hit_enemies: false,
            fired: false,
            fired_enemies: false,
        }
    }

    /// Also damage regular enemies.
    #[inline]
    pub fn with_enemies(mut self) -> Self {
        self.hit_enemies = true;
        self
    }

    /// Add a knockback impulse (px/s).
    #[inline]
    pub fn with_knockback(mut self, knockback: f32) -> Self {
        self.knockback = knockback;
        self
    }

    /// Is the active window open this tick?
    #[inline]
    pub fn is_active(&self) -> bool {
        self.elapsed >= self.warn && self.elapsed < self.warn + self.active
    }

    /// Is the hazard expired?
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.elapsed >= self.warn + self.active
    }
}

// ============================================================================
// Boss-owned projectiles
// ============================================================================

/// Which art projectile this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossProjectileKind {
    /// Mower spinning steel blade disc.
    BladeDisc,
    /// Mower razor grass shrapnel.
    GrassShrapnel,
    /// Cat phase-2 sonic fang.
    SonicFang,
    /// Broom dustpan shard.
    DustpanShard,
}

impl BossProjectileKind {
    /// Collision radius.
    #[inline]
    pub fn radius(self) -> f32 {
        match self {
            BossProjectileKind::BladeDisc => art_boss_mower::BLADE_DISC_RADIUS,
            BossProjectileKind::GrassShrapnel => art_boss_mower::SHRAPNEL_RADIUS,
            BossProjectileKind::SonicFang => 2.5,
            BossProjectileKind::DustpanShard => 2.0,
        }
    }

    /// Damage.
    #[inline]
    pub fn damage(self) -> f32 {
        match self {
            BossProjectileKind::BladeDisc => 16.0,
            BossProjectileKind::GrassShrapnel => 9.0,
            BossProjectileKind::SonicFang => 12.0,
            BossProjectileKind::DustpanShard => 10.0,
        }
    }

    /// Wall bounces before despawning.
    #[inline]
    pub fn bounces(self) -> u8 {
        match self {
            BossProjectileKind::BladeDisc => 1,
            _ => 0,
        }
    }
}

/// A request from a controller to spawn a boss projectile.
#[derive(Clone, Copy, Debug)]
pub struct BossProjectileSpawn {
    pub kind: BossProjectileKind,
    pub pos: Vec2,
    pub vel: Vec2,
    pub radius: f32,
    pub damage: f32,
    pub life: f32,
    pub bounces: u8,
}

impl BossProjectileSpawn {
    /// Spawn request for a kind at a position/velocity.
    #[inline]
    pub fn new(kind: BossProjectileKind, pos: Vec2, vel: Vec2) -> Self {
        Self {
            kind,
            pos,
            vel,
            radius: kind.radius(),
            damage: kind.damage(),
            life: 2.4,
            bounces: kind.bounces(),
        }
    }
}

/// A live boss projectile.
#[derive(Component, Clone, Copy, Debug)]
pub struct BossProjectile {
    pub kind: BossProjectileKind,
    pub pos: Vec2,
    pub prev: Vec2,
    pub vel: Vec2,
    pub radius: f32,
    pub damage: f32,
    pub life: f32,
    pub bounces: u8,
    pub anim: f32,
}

// ============================================================================
// Boss component
// ============================================================================

/// The boss entity's authoritative state.
#[derive(Component, Clone, Debug)]
pub struct Boss {
    pub kind: BossKind,
    /// Damage-number identity (`NextEnemyId`).
    pub id: u32,
    pub pos: Vec2,
    pub facing: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub phase: u8,
    pub state: BossState,
    pub elapsed: f32,
    pub duration: f32,
    pub anim: f32,
    pub hit_flash: f32,
    pub hurt: f32,
    pub invuln: f32,
    pub damage_mult: f32,
    pub contact_cd: f32,
    /// Body-contact damage for the **current state** (the art's per-state
    /// number). The framework resets it to [`BossKind::contact_damage`] before
    /// every controller tick; a controller then lowers it with
    /// [`Boss::no_contact`] when its art gives the state no body damage
    /// (mower `Overheat`/`Flipped`, the broom's safe pocket during a rake) or
    /// sets the state's own number with [`Boss::set_contact_damage`] (broom
    /// `Hurt` = 6 bump, mower `Docking` = 12).
    pub contact_damage: f32,
    /// Seconds of movement slow left (the Sonic Squeak's ring is the only
    /// source): `move_boss` multiplies every controller's speed by
    /// [`Boss::slow_mult`], so a briefly slowed boss keeps its whole state
    /// machine and only loses ground speed.
    pub slow_t: f32,
    pub dead: bool,
    pub death_announced: bool,
    pub arena: TileRect,
    pub hunting: bool,
    pub target: Vec2,
    /// The art view's leap origin (cat `Pounce`, mower `trail_start`).
    pub pounce_origin: Vec2,
    /// The art view's paw/side selector (cat `swipe_side`).
    pub swipe_side: f32,
    /// The art view's dock pad (mower `dock_pos`).
    pub dock: Vec2,
    /// The art view's bounce counter (mower `bounce_count`).
    pub bounce_count: u8,
    /// Per-controller memory. The framework and the art views never read it:
    /// each controller owns a documented layout (see the boss files).
    pub scratch: [f32; 6],
    /// Per-controller flag word (state-local one-shots).
    pub flags: u32,
    /// Seconds the boss has been too far away to fight (see
    /// [`BossKind::REJOIN_DIST`] and [`RELOCATE_AFTER_SECS`]): the framework
    /// walks an off-screen, stranded boss back into the fight.
    pub lost: f32,
    pub hazards: Vec<BossHazard>,
}

impl Boss {
    /// Spawn a healthy boss in `arena`.
    pub fn new(kind: BossKind, pos: Vec2, hp: f32, arena: TileRect, dock: Vec2, hunting: bool) -> Self {
        Self {
            kind,
            id: 0,
            pos,
            facing: Vec2::new(1.0, 0.0),
            hp,
            max_hp: hp,
            phase: 1,
            state: kind.initial_state(),
            elapsed: 0.0,
            duration: kind.initial_duration(),
            anim: 0.0,
            hit_flash: 0.0,
            hurt: 0.0,
            invuln: 0.0,
            damage_mult: 1.0,
            contact_cd: 0.0,
            contact_damage: kind.contact_damage(),
            slow_t: 0.0,
            dead: false,
            death_announced: false,
            arena,
            hunting,
            target: pos,
            pounce_origin: pos,
            swipe_side: 1.0,
            dock,
            bounce_count: 0,
            scratch: [0.0; 6],
            flags: 0,
            lost: 0.0,
            hazards: Vec::with_capacity(16),
        }
    }

    /// Movement multiplier from the current slow (1.0 when free).
    ///
    /// An active weapon's crowd control is deliberately weaker on a boss than
    /// on the horde: `SonicSqueak` calls [`Boss::slow_for`] with a 0.5 multiplier
    /// for 0.8 s, which reads as a stagger, never as a stun-lock.
    #[inline]
    pub fn slow_mult(&self) -> f32 {
        if self.slow_t > 0.0 {
            0.5
        } else {
            1.0
        }
    }

    /// Slow the boss's movement for `secs` (keeps the longer window).
    #[inline]
    pub fn slow_for(&mut self, _mult: f32, secs: f32) {
        if secs > 0.0 {
            self.slow_t = self.slow_t.max(secs);
        }
    }

    /// Replace the body-contact damage for the current state.
    #[inline]
    pub fn set_contact_damage(&mut self, damage: f32) {
        self.contact_damage = damage.max(0.0);
    }

    /// This state deals no body-contact damage at all (the art's `0` states).
    #[inline]
    pub fn no_contact(&mut self) {
        self.contact_damage = 0.0;
    }

    /// Remaining health fraction in `0..=1`.
    #[inline]
    pub fn hp_frac(&self) -> f32 {
        if self.max_hp <= 0.0 {
            0.0
        } else {
            (self.hp / self.max_hp).clamp(0.0, 1.0)
        }
    }

    /// Progress through the current state in `0..=1`.
    #[inline]
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }

    /// Enter a new state, resetting its clock.
    #[inline]
    pub fn set_state(&mut self, state: BossState, duration: f32) {
        self.state = state;
        self.duration = duration.max(0.01);
        self.elapsed = 0.0;
    }

    /// Queue a hazard. Returns false when the ring is full.
    #[inline]
    pub fn push_hazard(&mut self, hazard: BossHazard) -> bool {
        if self.hazards.len() < self.hazards.capacity() {
            self.hazards.push(hazard);
            true
        } else {
            false
        }
    }

    /// Take damage. Returns true when the hit landed (not invulnerable).
    #[inline]
    pub fn take_damage(&mut self, amount: f32) -> bool {
        if self.dead || self.invuln > 0.0 {
            return false;
        }
        self.hp -= amount.max(0.0);
        self.hit_flash = 1.0;
        self.hurt = 0.12;
        true
    }

    /// Is the boss airborne (cat pounce) for drawing/contact purposes?
    #[inline]
    pub fn is_airborne(&self) -> bool {
        match self.state {
            BossState::Cat(s) => {
                s == BossCatState::Pounce
                    && self.progress() > 0.12
                    && self.progress() < art_boss_cat::POUNCE_LAND_PROGRESS
            }
            _ => false,
        }
    }

    /// The art-defined hit shapes for player damage.
    ///
    /// The cat's three body circles are the art's `body_hit_circles`, but while
    /// [`art_boss_cat::is_airborne`] holds (mid-`Pounce`) they are marked
    /// ungrounded: the jaguar is leaping **over** the rat, so ground weapons and
    /// ground-level feedback pass underneath it and [`Boss::target_points`] is
    /// empty for that window. The broom carries the same rule per foot from the
    /// art's `foot_hitboxes` (`grounded`); the mower is always on the ground.
    pub fn hit_shapes(&self) -> BossHitShapes {
        match self.kind {
            BossKind::Cat => {
                let view = self.cat_view();
                let circles = art_boss_cat::body_hit_circles(&view);
                let grounded = !self.is_airborne();
                let shapes: [BossHitShape; 3] = [
                    BossHitShape::Circle {
                        center: circles[0].0,
                        radius: circles[0].1,
                        grounded,
                    },
                    BossHitShape::Circle {
                        center: circles[1].0,
                        radius: circles[1].1,
                        grounded,
                    },
                    BossHitShape::Circle {
                        center: circles[2].0,
                        radius: circles[2].1,
                        grounded,
                    },
                ];
                BossHitShapes::from_slice(&shapes)
            }
            BossKind::Broom => {
                let view = self.broom_view();
                let feet = art_boss_broom::foot_hitboxes(&view);
                let shapes: [BossHitShape; 2] = [
                    BossHitShape::Circle {
                        center: feet[0].pos,
                        radius: feet[0].radius,
                        grounded: feet[0].grounded,
                    },
                    BossHitShape::Circle {
                        center: feet[1].pos,
                        radius: feet[1].radius,
                        grounded: feet[1].grounded,
                    },
                ];
                BossHitShapes::from_slice(&shapes)
            }
            BossKind::Mower => {
                let view = self.mower_view();
                let radius = art_boss_mower::collision_radius(view.state, view.progress);
                let shape = BossHitShape::Circle {
                    center: view.pos,
                    radius,
                    grounded: true,
                };
                BossHitShapes::from_slice(&[shape])
            }
            // The platen is a slab, so it is hit through an oriented box; while
            // the type bed or the ribbon is exposed the weak point is its own
            // target, which is what auto-aim and the ground weapons reach for
            // during the punish window.
            BossKind::NightEdition => {
                let view = self.night_edition_view();
                let (center, facing, half, half_perp) = art_boss_night_edition::body_box(&view);
                let mut shapes = [
                    BossHitShape::Box {
                        center,
                        facing,
                        half,
                        half_perp,
                        grounded: true,
                    };
                    3
                ];
                let mut len = 1usize;
                if let Some(weak) = art_boss_night_edition::weak_point(&view) {
                    if weak.dist(center) > 1.0 {
                        shapes[len] = BossHitShape::Circle {
                            center: weak,
                            radius: art_boss_night_edition::RIBBON_WINDOW_RADIUS,
                            grounded: true,
                        };
                        len += 1;
                    }
                }
                BossHitShapes::from_slice(&shapes[..len])
            }
        }
    }

    /// Targetable points for auto-aim (worker P4).
    ///
    /// Only **grounded** shapes count: a lifted foot (the broom's raised
    /// slipper during `StompWalk`, its injured foot during `Hurt`) is high
    /// above the rat, so aiming at it would fire ground weapons at empty air.
    pub fn target_points(&self) -> BossTargetable {
        let shapes = self.hit_shapes();
        let mut points = [Vec2::ZERO; 3];
        let mut len = 0u8;
        for shape in shapes.iter() {
            if len as usize >= points.len() {
                break;
            }
            if !shape.grounded() {
                continue;
            }
            points[len as usize] = shape.center();
            len += 1;
        }
        BossTargetable { points, len }
    }

    /// The art view for drawing.
    pub fn view(&self) -> BossArtView {
        match self.kind {
            BossKind::Cat => BossArtView::Cat(self.cat_view()),
            BossKind::Broom => BossArtView::Broom(self.broom_view()),
            BossKind::Mower => BossArtView::Mower(self.mower_view()),
            BossKind::NightEdition => BossArtView::NightEdition(self.night_edition_view()),
        }
    }

    fn cat_state(&self) -> BossCatState {
        match self.state {
            BossState::Cat(s) => s,
            _ => BossCatState::Stalk,
        }
    }

    fn broom_state(&self) -> BroomBossState {
        match self.state {
            BossState::Broom(s) => s,
            _ => BroomBossState::Idle,
        }
    }

    fn mower_state(&self) -> MowerState {
        match self.state {
            BossState::Mower(s) => s,
            _ => MowerState::Patrol,
        }
    }

    fn cat_view(&self) -> BossCatView {
        BossCatView {
            pos: self.pos,
            facing: self.facing,
            state: self.cat_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            target: self.target,
            pounce_origin: self.pounce_origin,
            swipe_side: self.swipe_side,
        }
    }

    fn broom_view(&self) -> BroomBossView {
        BroomBossView {
            pos: self.pos,
            facing: self.facing,
            state: self.broom_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            target: self.target,
            enraged: self.phase >= 2,
        }
    }

    fn night_edition_state(&self) -> NightEditionState {
        match self.state {
            BossState::NightEdition(s) => s,
            _ => NightEditionState::Roll,
        }
    }

    fn night_edition_view(&self) -> NightEditionView {
        NightEditionView {
            pos: self.pos,
            facing: self.facing,
            state: self.night_edition_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            phase: self.phase,
            page: self.target,
            // The art's side selector: the page's blank margin.
            margin_side: self.swipe_side,
            rule_from: self.pounce_origin,
            rule_to: self.target,
            // The framework hands the arena's exported centre as the "dock";
            // for the Night Edition that is the press bed's centre.
            bed: self.dock,
        }
    }

    fn mower_view(&self) -> MowerView {
        MowerView {
            pos: self.pos,
            facing: self.facing,
            state: self.mower_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            target: self.target,
            trail_start: self.pounce_origin,
            dock_pos: self.dock,
            bounce_count: self.bounce_count,
        }
    }
}

// ============================================================================
// Controller context + output
// ============================================================================

/// The context a per-boss controller reads each tick.
pub struct BossContext<'a> {
    pub dt: f32,
    pub time: f32,
    pub player_pos: Vec2,
    pub player_vel: Vec2,
    pub player_radius: f32,
    pub arena: TileRect,
    pub level: &'a Level,
    pub flow: &'a FlowField,
    pub rng: &'a mut StdRng,
}

impl<'a> BossContext<'a> {
    /// Player position.
    #[inline]
    pub fn player_pos(&self) -> Vec2 {
        self.player_pos
    }

    /// Player velocity.
    #[inline]
    pub fn player_vel(&self) -> Vec2 {
        self.player_vel
    }

    /// Is the line from `from` to `to` clear of walls for `radius`?
    #[inline]
    pub fn los_clear(&self, from: Vec2, to: Vec2, radius: f32) -> bool {
        self.level.tiles.segment_hit(from, to, radius).is_none()
    }

    /// Does a large body of `radius` fit at `pos`?
    #[inline]
    pub fn footprint_fits(&self, pos: Vec2, radius: f32) -> bool {
        self.level.tiles.circle_clear(pos, radius, SizeClass::Large)
    }

    /// Does an attack footprint fit at `origin`, and is the player's escape
    /// lane from `origin` along `escape_dir` clear for `escape_dist`?
    #[inline]
    pub fn attack_fits(
        &self,
        origin: Vec2,
        radius: f32,
        escape_dir: Vec2,
        escape_dist: f32,
    ) -> bool {
        self.footprint_fits(origin, radius)
            && self.los_clear(
                origin,
                origin + escape_dir.normalized() * escape_dist,
                self.player_radius,
            )
    }

    /// The nearest open large-class center to `pos`.
    #[inline]
    pub fn nearest_open(&self, pos: Vec2) -> Vec2 {
        self.level
            .tiles
            .nearest_open_center(pos, SizeClass::Large, 8)
            .unwrap_or(pos)
    }

    /// The large-class flow-field direction at `pos`.
    #[inline]
    pub fn flow_dir(&self, pos: Vec2) -> Vec2 {
        self.flow.dir_at(SizeClass::Large, pos).normalized()
    }

    /// Steering direction: chase the player directly when the line is clear,
    /// else hunt through the large-class flow field.
    ///
    /// A clear line always beats the flow field. The field is cell-granular and
    /// a cell the boss is standing on can legitimately point into a wall jamb
    /// (the coarse field is only guaranteed for a body of its own class, and a
    /// boss hugging a doorway is exactly where the fine window and the cell
    /// centring disagree). Measured on the bot ladder (seed 900): a roaming
    /// (`hunting`) jaguar with the rat 130 px away in plain sight stood still in
    /// `Stalk` for 8-16 s at a time because the flow direction was unwalkable —
    /// 15 % of one Cat fight and every second of a Casual fight. Only route
    /// around walls when the straight line genuinely does not fit.
    pub fn seek(&self, boss: &Boss) -> Vec2 {
        let radius = boss.kind.body_radius();
        let to = self.player_pos - boss.pos;
        let direct = if to.len_sq() > 1e-6 {
            to.normalized()
        } else {
            Vec2::ZERO
        };
        if self.los_clear(boss.pos, self.player_pos, radius) {
            return direct;
        }
        let dir = self.flow_dir(boss.pos);
        // A flow direction that cannot be stepped (wall-facing cell) would pin
        // the boss in place; taking the direct bearing instead at least slides
        // along the obstruction.
        if dir.len_sq() > 0.0 && self.footprint_fits(boss.pos + dir * 2.0, radius) {
            return dir;
        }
        if direct.len_sq() > 0.0 {
            return direct;
        }
        dir
    }
}

/// One piece of audio/particle feedback a controller asks for.
///
/// Controllers cannot touch `SfxQueue` or `Particles` (they only get a
/// [`BossContext`]), so they push cues into [`BossOutput::cues`] and the shared
/// system drains them right after the controller tick: each cue maps to exactly
/// one [`Sfx`] push and one `vfx::emit_*` call (see `emit_cues`), drawing its
/// randomness from the boss's own stream so the order stays deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossCue {
    // ---- Jungle Cat (Sewer) ----
    /// `Crouch` entry: the jaguar digs in (paw dust + crouch growl).
    CatCrouch,
    /// `Crouch → Pounce`: the leap launches.
    CatPounce,
    /// The pounce impact at `POUNCE_LAND_PROGRESS`.
    CatLand,
    /// The overshoot skid after a missed pounce.
    CatSkid,
    /// `SwipeWindup → Swipe`: the claws go through.
    CatSwipe,
    /// `Roar` entry (phase change / phase-2 territory call).
    CatRoar,
    /// One fan of phase-2 sonic fangs.
    CatFangs,
    /// A heavy burst staggered the cat into `Hurt`.
    CatHurt,
    /// The `Death` collapse.
    CatDeath,

    // ---- The Sweeper (Kitchen) ----
    /// `StompWalk` footfall (the art's `progress 0.66`).
    BroomStomp,
    /// The rake's active window opens (`progress 0.46`).
    BroomSweep,
    /// The dustpan slams down (`progress 0.48`).
    BroomDustpan,
    /// The `PhaseChange` tantrum slam (`progress 0.38`).
    BroomSlam,
    /// `Hurt` entry: the stubbed toe.
    BroomHurtHop,
    /// Each touchdown of the hop.
    BroomHop,
    /// The `Defeat` walk-off (the broom hits the floor).
    BroomDefeat,

    // ---- TURF-REAPER MK.IV (Lab) ----
    /// `Scan` entry: the lidar sweep starts.
    MowerScan,
    /// The scan snap-locks (`SCAN_LOCK_PROGRESS`).
    MowerLockOn,
    /// `ChargeWindup` entry: the motor revs.
    MowerWindup,
    /// `Charge` entry: the blades spin up and it launches.
    MowerCharge,
    /// A wall impact ends a charge or a rebound leg.
    MowerWallImpact,
    /// `Flipped` entry: it goes over onto its back.
    MowerFlip,
    /// `Overheat` entry: the motors stall and vent.
    MowerOverheat,
    /// `Docking` entry: the retreat to the charging pad.
    MowerDock,
    /// The meltdown detonation in `Dying`.
    MowerDeath,

    // ---- THE NIGHT EDITION (Printworks) ----
    /// `PageTell` entry: the sheet is fed and the frame is set.
    NightPage,
    /// The impression lands (`PageStamp` entry / a misprint beat).
    NightStamp,
    /// `BedLowered` entry: the form drops away from the platen.
    NightBed,
    /// `MisprintTell` entry: two sheets, one clear stripe.
    NightMisprint,
    /// `CarriageTell` entry: the bell.
    NightBell,
    /// `CarriageSweep` entry: the carriage runs the rule.
    NightCarriage,
    /// `Ribbon` entry: the ribbon lifts out of the spools.
    NightRibbon,
    /// `PhaseTurn` entry: the frame unlatches and re-seats.
    NightTurn,
    /// `Wreck` entry: the press tears itself apart.
    NightWreck,
}

impl BossCue {
    /// The sound this cue plays.
    #[inline]
    pub fn sfx(self) -> Sfx {
        match self {
            BossCue::CatCrouch => Sfx::CatCrouch,
            BossCue::CatPounce => Sfx::CatPounce,
            // `Sfx::CatLand` is the landing thud *and* the claw skid.
            BossCue::CatLand => Sfx::CatLand,
            BossCue::CatSkid => Sfx::CatLand,
            BossCue::CatSwipe => Sfx::CatSwipe,
            BossCue::CatRoar => Sfx::CatRoar,
            // The sonic fang fan is a short bark out of the roar's throat.
            BossCue::CatFangs => Sfx::CatGrowl,
            BossCue::CatHurt => Sfx::CatHurt,
            BossCue::CatDeath => Sfx::CatDeath,
            BossCue::BroomStomp => Sfx::BroomStomp,
            BossCue::BroomSweep => Sfx::BroomSweep,
            BossCue::BroomDustpan => Sfx::BroomDustpan,
            // The tantrum slam is a double footfall: the stomp sound.
            BossCue::BroomSlam => Sfx::BroomStomp,
            BossCue::BroomHurtHop => Sfx::BroomHurt,
            BossCue::BroomHop => Sfx::BroomHop,
            BossCue::BroomDefeat => Sfx::BroomDefeat,
            BossCue::MowerScan => Sfx::MowerScan,
            BossCue::MowerLockOn => Sfx::MowerLockOn,
            BossCue::MowerWindup => Sfx::MowerCharge,
            BossCue::MowerCharge => Sfx::MowerBladeSpin,
            BossCue::MowerWallImpact => Sfx::MowerWallClang,
            BossCue::MowerFlip => Sfx::MowerFlipped,
            BossCue::MowerOverheat => Sfx::MowerOverheat,
            BossCue::MowerDock => Sfx::MowerDockHum,
            BossCue::MowerDeath => Sfx::MowerDeath,
            BossCue::NightPage => Sfx::NightStamp,
            BossCue::NightStamp => Sfx::NightStamp,
            BossCue::NightBed => Sfx::NightStamp,
            BossCue::NightMisprint => Sfx::NightBell,
            BossCue::NightBell => Sfx::NightBell,
            BossCue::NightCarriage => Sfx::NightCarriage,
            // The ribbon window is a mechanical reveal, exactly the chest-open
            // sting's shape.
            BossCue::NightRibbon => Sfx::CacheOpen,
            // Re-seating machinery: the dock's servo hum.
            BossCue::NightTurn => Sfx::MowerDockHum,
            BossCue::NightWreck => Sfx::MowerDeath,
        }
    }
}

/// Per-tick output from a controller. Reused across ticks.
#[derive(Default)]
pub struct BossOutput {
    pub phase_changed: bool,
    pub death: bool,
    pub projectiles: Vec<BossProjectileSpawn>,
    /// Feedback requests for this tick, drained in push order.
    pub cues: Vec<BossCue>,
    /// Development tracing state (see [`BossTrace`]). It rides in this `Local`
    /// because the system is already at Bevy's 16-parameter limit for a
    /// single system; it is inert unless `RATRUN_BOSS_TRACE` is set.
    pub trace: BossTrace,
}

impl BossOutput {
    /// Reset for the next tick (keeps capacity).
    #[inline]
    pub fn clear(&mut self) {
        self.phase_changed = false;
        self.death = false;
        self.projectiles.clear();
        self.cues.clear();
    }

    /// Ask for one piece of feedback once the controller tick is done.
    #[inline]
    pub fn cue(&mut self, cue: BossCue) {
        self.cues.push(cue);
    }
}

/// A per-kind boss controller. Implement only transitions and attack definitions.
pub trait BossController: Send + Sync {
    /// Which kind this controller drives.
    fn kind(&self) -> BossKind;

    /// Advance one tick. The shared system owns phase detection, death and
    /// hazard/projectile resolution; this only transitions states and queues
    /// hazards/projectiles/cues.
    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput);

    /// The death beat, called instead of [`BossController::tick`] for every
    /// tick of the art's death state (`Death` / `Defeat` / `Dying`).
    ///
    /// The controller may queue the art's last-gasp hazards (the mower's 28 px
    /// detonation) and its death cues, but must not start new attacks: the
    /// boss is already reaped and `level_complete` fires when this beat ends.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, _boss: &mut Boss, _out: &mut BossOutput) {}
}

// ============================================================================
// Shared movement / hazard resolution
// ============================================================================

/// Move a large body toward `dir` at `speed`, sliding along walls. Returns
/// true when the full step fit.
pub fn move_boss(ctx: &BossContext<'_>, boss: &mut Boss, dir: Vec2, speed: f32) -> bool {
    // The Sonic Squeak's brief slow: every controller moves through here, so one
    // multiplier covers the cat's stalk, the broom's steps and the mower's
    // charge without touching any state machine.
    let step = dir.normalized() * (speed * boss.slow_mult() * ctx.dt);
    if step.len_sq() <= 0.0 {
        return false;
    }
    let radius = boss.kind.body_radius();
    let next = boss.pos + step;
    if ctx.footprint_fits(next, radius) {
        boss.pos = next;
        return true;
    }
    // Axis slide: try x then y so a body grazes a wall instead of sticking.
    let try_x = Vec2::new(step.x, 0.0);
    if try_x.len_sq() > 0.0 && ctx.footprint_fits(boss.pos + try_x, radius) {
        boss.pos += try_x;
        return false;
    }
    let try_y = Vec2::new(0.0, step.y);
    if try_y.len_sq() > 0.0 && ctx.footprint_fits(boss.pos + try_y, radius) {
        boss.pos += try_y;
        return false;
    }
    false
}

/// Resolve active hazards against the player and regular enemies. Returns true
/// when the player was hit.
#[allow(clippy::too_many_arguments)]
pub fn resolve_hazards(
    boss: &mut Boss,
    dt: f32,
    player: &mut Player,
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    enemy_order: &mut Vec<(u64, Entity)>,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    rng: &mut StdRng,
    sfx: &mut SfxQueue,
) -> bool {
    let mut player_hit = false;
    for hazard in boss.hazards.iter_mut() {
        hazard.elapsed += dt;
        if !hazard.is_active() {
            continue;
        }
        if hazard.hit_player && !hazard.fired && hazard.shape.contains(player.pos, PLAYER_RADIUS) {
            if hazard.damage <= 0.0 {
                // Push-only hazard (the cat's damage-free outer roar ring):
                // the knockback is a sustained shove in px/s, applied every
                // active tick the rat is inside the shape. Nothing is "hit",
                // so no damage, no i-frames and no hurt vfx are spent.
                if hazard.knockback > 0.0 {
                    let dir = (player.pos - hazard.shape.center()).normalized();
                    player.pos += dir * hazard.knockback * dt;
                }
            } else if player.take_damage(hazard.damage) {
                player_hit = true;
                hazard.fired = true;
                if hazard.knockback > 0.0 {
                    let dir = (player.pos - hazard.shape.center()).normalized();
                    player.pos += dir * hazard.knockback * dt;
                }
                vfx::emit_player_hurt(particles, rng, player.pos);
                sfx.0.push(Sfx::Hurt);
            }
        }
        if hazard.hit_enemies && !hazard.fired_enemies {
            // Determinism: damage and vfx in `SpawnId` order, never storage
            // order.
            enemy_order.clear();
            for (entity, id, _) in enemies.iter() {
                enemy_order.push((id.0, entity));
            }
            enemy_order.sort_unstable_by_key(|(id, _)| *id);
            for &(_, entity) in enemy_order.iter() {
                let Ok((_, _, mut enemy)) = enemies.get_mut(entity) else {
                    continue;
                };
                if !enemy.alive() || !hazard.shape.contains(enemy.pos, enemy.radius) {
                    continue;
                }
                // Directional armour (the Pillbug's shell) applies to a boss
                // hazard hit; the hazard's own centre is the source.
                let applied = enemy.hurt_from(hazard.damage, hazard.shape.center());
                let killed = !enemy.alive();
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    enemy.pos,
                    Vec2::ZERO,
                    None,
                    enemy.kind,
                    killed,
                );
                damage_numbers.record(
                    enemy.id,
                    enemy.pos,
                    enemy.radius,
                    applied,
                    killed,
                    DamageSource::Normal,
                );
            }
            hazard.fired_enemies = true;
        }
    }
    boss.hazards.retain(|h| !h.is_expired());
    player_hit
}

// ============================================================================
// Cue channel
// ============================================================================

/// Drain a controller's cues into the audio queue and the particle system.
///
/// Each cue plays its [`Sfx`](BossCue::sfx) and calls the matching `vfx::emit_*`
/// helper with geometry taken from the boss's art view, so a cue never has to
/// carry coordinates (and the enum stays `Copy`). Randomness comes from the
/// caller's stream — the boss's own rng, the same one every other boss vfx call
/// in the tick uses — and cues are emitted in push order, which is the order the
/// controller queued them.
#[allow(clippy::too_many_arguments)]
fn emit_cues(
    cues: &[BossCue],
    boss: &Boss,
    blood: Rgb,
    particles: &mut Particles,
    rng: &mut StdRng,
    sfx: &mut SfxQueue,
) {
    for &cue in cues {
        sfx.0.push(cue.sfx());
        match cue {
            // ---- cat ----------------------------------------------------
            // The crouch digs in and the leap launches: both are a paw-dust
            // burst (the art has no dedicated emitter for the two poses).
            BossCue::CatCrouch | BossCue::CatPounce | BossCue::CatHurt => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.14);
            }
            BossCue::CatLand => {
                let phase2 = boss.phase >= 2;
                vfx::emit_boss_cat_pounce_land(
                    particles,
                    rng,
                    boss.target,
                    art_boss_cat::POUNCE_LAND_RADIUS,
                    phase2,
                );
            }
            BossCue::CatSkid => {
                let move_dir = boss.facing;
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, move_dir, 0.16);
            }
            BossCue::CatSwipe => {
                let view = boss.cat_view();
                let origin = art_boss_cat::swipe_origin(&view);
                vfx::emit_boss_cat_swipe(
                    particles,
                    rng,
                    origin,
                    boss.facing,
                    boss.swipe_side,
                    boss.phase >= 2,
                );
            }
            BossCue::CatRoar | BossCue::CatFangs => {
                let view = boss.cat_view();
                let origin = art_boss_cat::roar_origin(&view);
                vfx::emit_boss_cat_roar(particles, rng, origin, boss.phase >= 2);
            }
            BossCue::CatDeath => {
                vfx::emit_boss_cat_death(particles, rng, boss.pos, boss.facing, blood);
            }

            // ---- broom --------------------------------------------------
            BossCue::BroomStomp => {
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.target,
                    art_boss_broom::STOMP_SHOCKWAVE_RADIUS,
                    boss.phase >= 2,
                );
            }
            BossCue::BroomSlam => {
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.pos,
                    art_boss_broom::PHASE_SLAM_RADIUS,
                    true,
                );
            }
            BossCue::BroomSweep => {
                let view = boss.broom_view();
                let angle = art_boss_broom::broom_sweep_angle(&view);
                let clockwise = matches!(
                    boss.state,
                    BossState::Broom(BroomBossState::BroomSweep { clockwise: true })
                );
                // One discrete burst at the top of the active window (`dt` is
                // the burst's own budget, not the tick length).
                vfx::emit_boss_broom_sweep(particles, rng, boss.pos, angle, clockwise, 0.18);
            }
            BossCue::BroomDustpan => {
                vfx::emit_boss_broom_dustpan_drop(
                    particles,
                    rng,
                    boss.target,
                    art_boss_broom::DUSTPAN_IMPACT_RADIUS,
                );
            }
            BossCue::BroomHurtHop | BossCue::BroomHop => {
                let view = boss.broom_view();
                let feet = art_boss_broom::foot_hitboxes(&view);
                // The grounded slipper is the hopping one; the lifted one is
                // the stubbed foot the pain stars belong to.
                let (hop, hurt) = if feet[0].grounded {
                    (feet[0].pos, feet[1].pos)
                } else {
                    (feet[1].pos, feet[0].pos)
                };
                vfx::emit_boss_broom_hurt_hop(particles, rng, hop, hurt);
            }
            BossCue::BroomDefeat => {
                let view = boss.broom_view();
                let broom = art_boss_broom::broom_head_pos(&view);
                vfx::emit_boss_broom_defeat(particles, rng, boss.pos, broom);
            }

            // ---- mower --------------------------------------------------
            // Lidar ping, lock-on spark and the dock's brass contact spark are
            // all a small electric glint (the art draws the cones/rings).
            BossCue::MowerScan => {
                vfx::emit_weapon_hit(particles, rng, boss.pos, Vec2::ZERO, None, EnemyKind::Boss, false);
            }
            BossCue::MowerLockOn => {
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    boss.target,
                    Vec2::ZERO,
                    None,
                    EnemyKind::Boss,
                    false,
                );
            }
            BossCue::MowerDock => {
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    boss.dock,
                    Vec2::ZERO,
                    None,
                    EnemyKind::Boss,
                    false,
                );
            }
            BossCue::MowerWindup => {
                vfx::emit_boss_mower_overheat_steam(particles, rng, boss.pos, boss.facing, 0.12);
            }
            BossCue::MowerCharge => {
                vfx::emit_boss_mower_charge_trail(particles, rng, boss.pos, boss.facing, false, 0.15);
            }
            BossCue::MowerWallImpact => {
                // The bumper is the front of the chassis; the wall normal points
                // back into the arena.
                let impact = boss.pos + boss.facing * art_boss_mower::CHASSIS_HALF_LEN;
                let normal = -boss.facing;
                vfx::emit_boss_mower_wall_impact(particles, rng, impact, normal, true);
            }
            BossCue::MowerFlip => {
                vfx::emit_boss_mower_flipped_smoke(particles, rng, boss.pos, 0.15);
            }
            BossCue::MowerOverheat => {
                vfx::emit_boss_mower_overheat_steam(particles, rng, boss.pos, boss.facing, 0.15);
            }
            BossCue::MowerDeath => {
                vfx::emit_boss_mower_death(particles, rng, boss.pos);
            }

            // ---- THE NIGHT EDITION --------------------------------------
            // The page's own outline is the art's; the cue is the press's
            // machinery: a glint on the sheet as the frame sets, the dust and
            // grit of the impression, ink mist off the lowered bed, the bell's
            // spark at the rule's near end, the carriage's trail and the
            // ribbon's reveal.
            BossCue::NightPage => {
                let (ink, _) =
                    art_boss_night_edition::inked_rect(boss.target, boss.swipe_side);
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    ink,
                    Vec2::ZERO,
                    None,
                    EnemyKind::Boss,
                    false,
                );
            }
            BossCue::NightStamp => {
                let (ink, half) =
                    art_boss_night_edition::inked_rect(boss.target, boss.swipe_side);
                vfx::emit_boss_broom_stomp(particles, rng, ink, half.y.max(8.0), boss.phase >= 2);
            }
            BossCue::NightBed => {
                vfx::emit_boss_mower_overheat_steam(particles, rng, boss.pos, boss.facing, 0.12);
            }
            BossCue::NightMisprint => {
                let rects =
                    art_boss_night_edition::misprint_rects(boss.target, boss.swipe_side);
                for (ink, _) in rects {
                    vfx::emit_weapon_hit(
                        particles,
                        rng,
                        ink,
                        Vec2::ZERO,
                        None,
                        EnemyKind::Boss,
                        false,
                    );
                }
            }
            BossCue::NightBell => {
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    boss.pounce_origin,
                    Vec2::ZERO,
                    None,
                    EnemyKind::Boss,
                    false,
                );
            }
            BossCue::NightCarriage => {
                vfx::emit_boss_mower_charge_trail(particles, rng, boss.pos, boss.facing, false, 0.15);
            }
            BossCue::NightRibbon => {
                let rib = art_boss_night_edition::ribbon_pos(boss.pos, boss.facing);
                vfx::emit_weapon_hit(particles, rng, rib, Vec2::ZERO, None, EnemyKind::Boss, false);
            }
            BossCue::NightTurn => {
                vfx::emit_boss_mower_flipped_smoke(particles, rng, boss.pos, 0.15);
            }
            BossCue::NightWreck => {
                vfx::emit_boss_mower_death(particles, rng, boss.pos);
            }
        }
    }
}

// ============================================================================
// Systems
// ============================================================================

/// Spawn the level's boss when the director raises [`BossDue`].
///
/// `BossDue` is raised by `director::raise_boss_due`; P3 owns that call site.
#[allow(clippy::too_many_arguments)]
pub fn boss_spawn_system(
    mut commands: Commands,
    mut due: ResMut<BossDue>,
    mut fight: ResMut<BossFight>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut next_spawn_id: ResMut<NextSpawnId>,
    level: Res<Level>,
    _level_index: Res<LevelIndex>,
    difficulty: Res<DifficultyLevel>,
    player: Res<Player>,
    bosses: Query<&Boss>,
    mut particles: ResMut<Particles>,
    mut shake: ResMut<Shake>,
    mut sfx: ResMut<SfxQueue>,
) {
    if !due.take() || !bosses.is_empty() {
        return;
    }
    // The theme is authoritative: the Laundry and the Printworks field their own
    // bosses (the Mangle, the Night Edition) rather than borrowing a stage-1/2
    // fight, and every other theme's boss is the same one `for_level` gives.
    let kind = BossKind::for_theme(level.theme);
    let arena = level
        .boss_room()
        .map(|r| r.rect)
        .unwrap_or_else(|| TileRect::new(0, 0, level.tiles.cols(), level.tiles.rows()));
    // The generator exports the arena's fight furniture: the clear encounter
    // floor (which is where the art's flip/escape layouts are guaranteed) and,
    // on the Lab, the charging dock against the wall. Spawn on the clear floor,
    // not on the arena's geometric centre, and hand the boss its real dock.
    let centre = level
        .boss_clear_floor()
        .map(|f| f.center_world())
        .unwrap_or_else(|| arena.center_world());
    let dock = level.boss_dock().unwrap_or_else(|| arena.center_world());
    let radius = kind.body_radius();
    let pos = if level.tiles.circle_clear(centre, radius, SizeClass::Large) {
        centre
    } else {
        level
            .tiles
            .nearest_open_center(centre, SizeClass::Large, 16)
            .unwrap_or(centre)
    };
    let hunting = !arena.contains_world(player.pos);
    // The director's curve freezes at BOSS_TIME; the boss spawns then.
    let hp = kind.base_hp()
        * kind.hp_scale()
        * director::boss_hp_mult_for(difficulty.0, director::BOSS_TIME);
    let mut boss = Boss::new(kind, pos, hp, arena, dock, hunting);
    boss.id = next_enemy_id.take();
    boss.facing = (player.pos - pos).normalized();
    boss.target = player.pos;
    fight.rng = BossRng::for_run(level.seed, kind);
    fight.next_id = boss.id;
    fight.level_complete = false;
    // The horde parts for its master: the arriving boss scatters the live
    // bodies (the same no-loot removal the boss's *death* uses — see
    // [`BossFight::horde_break`]). Measured (bot batches, seed 900): with 400+
    // bodies up at the boss time, most of the rat's fire went into the crowd
    // and measurably none into the boss, so boss fights ranged from 16 s to 35
    // minutes on the same build. A clean stage is the fair version of that: the
    // fight is about the boss, the throttled horde trickles back in, and the
    // gems already on the floor are untouched.
    fight.horde_break = true;
    let accent = level.palette.accent;
    vfx::emit_boss_spawn(&mut particles, &mut fight.rng.0, pos, accent);
    shake.amount = shake.amount.max(14.0);
    sfx.0.push(Sfx::Boss);
    commands.spawn((boss, next_spawn_id.take()));
}

/// Apply player damage to the boss through its art-defined hit shapes.
///
/// Sources: player bullets (including orbit), acid pools, Tail Whip and
/// mousetraps. Damage numbers and hit vfx use the existing hooks.
#[allow(clippy::too_many_arguments)]
pub fn boss_damage_system(
    tick: Res<TickInput>,
    level: Res<Level>,
    player: Res<Player>,
    mut fight: ResMut<BossFight>,
    mut bosses: Query<&mut Boss>,
    mut bullets: Query<(Entity, &SpawnId, &mut Bullet)>,
    pools: Query<&Pool>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut shake: ResMut<Shake>,
    mut sfx: ResMut<SfxQueue>,
) {
    fight.scratch.dt = tick.dt;
    fight.scratch.dmg = BossDamageTally::default();
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        let shapes = boss.hit_shapes();
        if shapes.iter().next().is_none() {
            continue;
        }
        // Player bullets in SpawnId order.
        fight.scratch.bullet_order.clear();
        for (entity, id, _) in bullets.iter() {
            fight.scratch.bullet_order.push((id.0, entity));
        }
        fight.scratch.bullet_order.sort_unstable_by_key(|(id, _)| *id);
        for i in 0..fight.scratch.bullet_order.len() {
            let entity = fight.scratch.bullet_order[i].1;
            let Ok((_, _, mut bullet)) = bullets.get_mut(entity) else {
                continue;
            };
            if bullet.faction != Faction::Player || bullet.hit_cd > 0.0 || bullet.lob.is_some() {
                continue;
            }
            if !shapes.segment_hits_grounded(bullet.prev_pos, bullet.pos, bullet.radius) {
                continue;
            }
            let amount = bullet.damage * boss.damage_mult;
            if !boss.take_damage(amount) {
                continue;
            }
            // Balance attribution only (see `BossDamageTally`): a projectile
            // from a weapon that picks its target counts as aimed, a
            // radial/orbit bullet as pattern damage.
            if is_aimed_weapon(bullet.weapon) {
                fight.scratch.dmg.aimed += amount;
            } else {
                fight.scratch.dmg.pattern += amount;
            }
            let killed = boss.hp <= 0.0;
            vfx::emit_weapon_hit(
                &mut particles,
                &mut fight.rng.0,
                bullet.pos,
                bullet.vel,
                bullet.weapon,
                EnemyKind::Boss,
                killed,
            );
            damage_numbers.record(
                boss.id,
                boss.pos,
                boss.kind.body_radius(),
                amount,
                killed,
                DamageSource::from_weapon(bullet.weapon),
            );
            // Exactly the enemy rule: a pierce charge is spent and the shot
            // cannot re-hit inside the re-hit cooldown; otherwise the shot is
            // dead. The `hit_cd` on the spent shot matters for the *same*
            // tick: `combat::player_bullet_hits_system` runs after this system
            // and does not itself look at `life` (a wall-stopped shot has to
            // keep resolving the part of its sweep that did connect), so
            // without it one dart could be spent on the boss and an enemy at
            // once.
            if bullet.pierce > 0 {
                bullet.pierce -= 1;
                bullet.hit_cd = 0.12;
            } else {
                bullet.life = 0.0;
                bullet.hit_cd = 0.12;
            }
            sfx.0.push(Sfx::Hit);
        }
        // Acid pools (DPS through the hit shapes).
        for pool in pools.iter() {
            if !shapes.circle_hits_grounded(pool.pos, pool.radius) {
                continue;
            }
            let amount = pool.dps * tick.dt;
            if boss.take_damage(amount) {
                fight.scratch.dmg.pools += amount;
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
        // Tail Whip: the weapons system pushed `Sfx::ShootWhip` this tick.
        if sfx.0.contains(&Sfx::ShootWhip) {
            if let Some(weapon) = player
                .loadout
                .weapons
                .iter()
                .find(|w| w.kind == WeaponKind::TailWhip)
            {
                // Exactly the aura `weapons::fire_one` used for this volley
                // (`26.0 + 3.0 * projectiles`), so a whip that clipped an
                // enemy also reaches a body standing that far away.
                let reach = 26.0 + weapon.level_projectiles() as f32 * 3.0;
                let dmg = weapon.damage();
                if shapes.circle_hits_grounded(player.pos, reach)
                    && level
                        .tiles
                        .segment_hit(player.pos, boss.pos, 1.0)
                        .is_none()
                {
                    let amount = dmg * boss.damage_mult;
                    if boss.take_damage(amount) {
                        fight.scratch.dmg.whip += amount;
                        vfx::emit_tail_whip_hit(
                            &mut particles,
                            &mut fight.rng.0,
                            boss.pos,
                            boss.pos - player.pos,
                            false,
                        );
                        damage_numbers.record(
                            boss.id,
                            boss.pos,
                            boss.kind.body_radius(),
                            amount,
                            false,
                            DamageSource::TailWhip,
                        );
                    }
                }
            }
        }
        // Mousetraps: an active snap under a hit shape damages once per snap.
        for (i, trap) in level.objects.mousetraps.iter().enumerate() {
            if trap.state != MousetrapState::Snapping {
                continue;
            }
            let p = trap.progress;
            if p < MOUSETRAP_HIT_PROGRESS_MIN || p > MOUSETRAP_HIT_PROGRESS_MAX {
                continue;
            }
            let latch = i as u32;
            if fight.scratch.mouse_latch.contains(&latch) {
                continue;
            }
            if shapes.iter().any(|s| {
                in_mouse_kill_rect(trap.pos, trap.facing, s.center(), s_radius(s))
            }) {
                let amount = MOUSETRAP_DAMAGE_ENEMY * boss.damage_mult;
                if boss.take_damage(amount) {
                    fight.scratch.dmg.traps += amount;
                    fight.scratch.mouse_latch.push(latch);
                    vfx::emit_weapon_hit(
                        &mut particles,
                        &mut fight.rng.0,
                        boss.pos,
                        Vec2::ZERO,
                        None,
                        EnemyKind::Boss,
                        false,
                    );
                    damage_numbers.record(
                        boss.id,
                        boss.pos,
                        boss.kind.body_radius(),
                        amount,
                        false,
                        DamageSource::Normal,
                    );
                }
            }
        }
        // Clear latches for traps no longer snapping.
        fight.scratch.mouse_latch.retain(|&i| {
            level
                .objects
                .mousetraps
                .get(i as usize)
                .map(|t| t.state == MousetrapState::Snapping)
                .unwrap_or(false)
        });
        if boss.hit_flash > 0.0 {
            shake.amount = shake.amount.max(1.5);
        }
    }
}

/// Distance (px) beyond which a live boss counts as "not in the fight" for
/// the rejoin rule below.
pub const REJOIN_DIST: f32 = 260.0;
/// How long a boss may spend out of the fight — further than [`REJOIN_DIST`]
/// from the rat, or with no line to him — before it is walked back into range.
///
/// The boss is an objective and a level cannot end until it dies, but the
/// horde's flow field is coarse far from the rat, and a rat that keeps kiting
/// can leave the boss behind a wall it never routes around. Measured on the bot
/// ladder (seed 900): 3 of 13 boss fights dealt **zero damage for 1 100-2 137 s**
/// with the boss 200-2 300 px away and stuck in `Cat(Stalk)` / `Broom(Idle)` /
/// `Mower(Patrol)`, plus one broom Idle-looping at 51 px with a wall in
/// between; those levels were uncompletable. The boss is only relocated when it
/// is off screen (or cannot see the rat) and has failed to close for this long,
/// with a full arrival cue, so a player never sees a pop-in.
pub const RELOCATE_AFTER_SECS: f32 = 5.0;
/// Where a rejoin puts the boss, in px from the rat, tried in order.
///
/// The ring starts inside the Cat's pounce band (`45..95 px`) on purpose. The
/// old single 130 px radius put the boss *outside* every one of its own attack
/// bands: measured on the bot ladder, the one Casual fight that stalled did so
/// for 16 s while the boss was teleported again and again to exactly 130 px
/// from a rat that had tucked itself into the Sewer's escape rat-hole — close
/// enough to see, too far to pounce, and unable to walk in because a rat-hole
/// is Small-class only. A rejoin is an emergency, so it lands the boss where
/// it can immediately act; the farther rings are the fallback for a room whose
/// inner ring is all wall.
const REJOIN_RADII: [f32; 6] = [95.0, 75.0, 55.0, 130.0, 175.0, 220.0];

/// Can a large body walk the straight line from `from` to `to`?
///
/// [`TileMap::segment_hit`] is a *wall* test, and the art draws rat-holes and
/// one-way gates as holes in a wall: a line straight through a rat-hole reads
/// as clear while a 13.5 px body cannot pass. The rejoin must not teleport the
/// boss into that trap, so this also requires every cell along the line to be
/// passable for the class.
fn walkable_line(level: &Level, from: Vec2, to: Vec2, radius: f32, class: SizeClass) -> bool {
    if level.tiles.segment_hit(from, to, radius).is_some() {
        return false;
    }
    let d = to - from;
    let steps = (d.len() / 8.0).ceil().max(1.0) as i32;
    // The rat's own cell is exempt: it may be standing inside a rat-hole, and
    // standing within pounce/rake reach of a hole is a fight, not a stalemate.
    for i in 0..steps {
        let p = from + d * (i as f32 / steps as f32);
        let (cx, cy) = collision::cell_of(p);
        if !level.tiles.passable(cx, cy, class) {
            return false;
        }
    }
    true
}

/// Where a rejoining boss is put: a large-clear cell within [`REJOIN_RADII`] of
/// the rat **with a walkable line to him**, nearest bearing first. Both halves
/// of that test matter: the boss and the rat can be 50 px apart on opposite
/// sides of a wall (the stalemate this exists to break), and a line through a
/// rat-hole is not a walk. Falls back to the closest clear cell to the rat.
fn rejoin_spot(level: &Level, player: Vec2, radius: f32) -> Option<Vec2> {
    let class = SizeClass::Large;
    for step in 0..24 {
        let angle = step as f32 * std::f32::consts::TAU / 24.0;
        for r in REJOIN_RADII {
            let p = player + Vec2::from_angle(angle) * r;
            if level.tiles.circle_clear(p, radius, class)
                && walkable_line(level, p, player, radius, class)
            {
                return Some(p);
            }
        }
    }
    // Nothing reachable on any ring: the rat is somewhere a large body cannot
    // walk to (a maze pocket, behind a rat-hole). Still bring the boss *near*
    // it — the siege rule below breaks the last wall — but never into one.
    let p = player + Vec2::new(REJOIN_RADII[0], 0.0);
    nearest_body_clear(level, p, radius, class, 24)
}

/// The nearest cell centre to `p` where a body of `radius` actually fits (a
/// *body* check, not just a class-passable cell: a 13.5 px radius is 27 px
/// across and a single passable 16 px cell does not hold it).
fn nearest_body_clear(
    level: &Level,
    p: Vec2,
    radius: f32,
    class: SizeClass,
    max_cells: i32,
) -> Option<Vec2> {
    let (pcx, pcy) = collision::cell_of(p);
    for r in 0..=max_cells {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue; // ring only, the inside was already tried
                }
                let (cx, cy) = (pcx + dx, pcy + dy);
                if !level.tiles.passable(cx, cy, class) {
                    continue;
                }
                let centre = Vec2::new((cx as f32 + 0.5) * collision::TILE_F, (cy as f32 + 0.5) * collision::TILE_F);
                if level.tiles.circle_clear(centre, radius, class) {
                    return Some(centre);
                }
            }
        }
    }
    None
}

/// How long a boss that is walled off from the rat — the straight line between
/// them is not body-passable and the boss cannot find a move — may pace before
/// it starts smashing through.
///
/// The rejoin rule guarantees the boss is *near* the rat; it cannot guarantee
/// it is *reachable*. The level design has pockets no large body can enter: a
/// maze's one-way door, the Sewer's escape rat-hole, a 1-cell squeeze. When the
/// rat kites into one, the large-class flow field has **no direction** and the
/// straight line is a wall, so a boss that only walks could never end the level
/// (verified: seed 17986931858700579504, Expert — boss alive 1244 s, no damage
/// on the boss for 1186 s, the Cat in `Stalk` at full HP while the rejoin kept
/// putting it 130 px away on the far side of a wall). The bosses are the
/// biggest things on the map: after this long the boss tears a body-width hole
/// in the wall in its way and keeps coming. It is deterministic (the same seed
/// breaks the same cell), costs one tile edit, and it is telegraphed by the
/// boss standing and pacing at the wall first.
pub const SIEGE_AFTER_SECS: f32 = 4.0;

/// The first wall cell on the straight line from `from` to `to` that a large
/// body cannot occupy (the cell the boss would have to break).
fn smash_target(level: &Level, from: Vec2, to: Vec2) -> Option<(i32, i32)> {
    let d = to - from;
    if d.len_sq() <= 1.0 {
        return None;
    }
    let steps = (d.len() / 6.0).ceil().max(1.0) as i32;
    for i in 1..=steps {
        let p = from + d * (i as f32 / steps as f32);
        let (cx, cy) = collision::cell_of(p);
        if !level.tiles.passable(cx, cy, SizeClass::Large) {
            return Some((cx, cy));
        }
    }
    None
}

/// Break a boss-sized hole at `at`: the `3 x 3` block of cells centred on it.
///
/// A boss is 27 px across (radius 13.5) and a cell is 16 px, so a `2 x 2` hole
/// (32 px) is only body-width when the body happens to be centred in it — half
/// the time the body's shoulder is still in a wall and the boss would be stuck
/// behind the hole it just made. Three cells (48 px) always clear.
fn smash_wall(level: &mut Level, at: (i32, i32)) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (at.0 + dx, at.1 + dy);
            if !level.tiles.get(cx, cy).is_open() {
                level.tiles.set(cx, cy, TileKind::Open);
            }
        }
    }
    level.tiles.rebuild_masks();
}

/// Advance the boss: phase, controller, hazards, projectiles, death.
#[allow(clippy::too_many_arguments)]
pub fn boss_update_system(
    mut commands: Commands,
    mut fight: ResMut<BossFight>,
    mut counters: ResMut<RunCounters>,
    tick: Res<TickInput>,
    mut level: ResMut<Level>,
    mut player: ResMut<Player>,
    flow: Res<FlowField>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut shake: ResMut<Shake>,
    mut sfx: ResMut<SfxQueue>,
    mut next_spawn_id: ResMut<NextSpawnId>,
    mut bosses: Query<(Entity, &mut Boss)>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut projectiles: Query<(Entity, &mut BossProjectile)>,
    mut out: Local<BossOutput>,
) {
    let dt = tick.dt;
    // One mutable borrow of the resource, so the scratch buffer and the rng
    // stream can be borrowed at the same time.
    let fight = &mut *fight;
    out.clear();
    fight.scratch.dt = dt;
    out.trace.init();
    for (entity, mut boss) in bosses.iter_mut() {
        if out.trace.on {
            out.trace.flow_hint = flow.dir_at(SizeClass::Large, boss.pos).normalized();
            out.trace
                .sample(&boss, &player, &level, dt, &fight.scratch.dmg);
        }
        boss.anim += dt;
        boss.hit_flash = (boss.hit_flash - dt * 8.0).max(0.0);
        boss.hurt = (boss.hurt - dt).max(0.0);
        boss.invuln = (boss.invuln - dt).max(0.0);
        boss.contact_cd = (boss.contact_cd - dt).max(0.0);
        boss.slow_t = (boss.slow_t - dt).max(0.0);
        boss.elapsed += dt;

        // Lethal hit: the damage system runs before this one, so the death
        // starts on the same tick. The art's death state then plays out
        // (`Death` / `Defeat` / `Dying`) — hazards and cues only, no new
        // attacks — and `level_complete` is raised when it ends, so the death
        // animation (and the mower's last-gasp 28 px blast) always reads
        // before the App shows the level-complete screen.
        if boss.hp <= 0.0 && !boss.dead {
            boss.dead = true;
            boss.hp = 0.0;
            boss.no_contact();
            boss.damage_mult = 1.0;
            let death_state = boss.kind.death_state();
            let death_duration = boss.kind.death_duration();
            boss.set_state(death_state, death_duration);
        }

        if boss.dead {
            if !boss.death_announced {
                boss.death_announced = true;
                commands.spawn((Gold::new(boss.pos, 20), next_spawn_id.take()));
                shake.amount = shake.amount.max(16.0);
                // The horde breaks and flees: every remaining enemy is
                // removed this tick. Raising the flag (instead of despawning
                // here) keeps enemy removal out of the `EnemyIndex` validity
                // window — the removal itself is `loot::reap_enemies_system`'s
                // job, after the last index reader. See
                // [`BossFight::horde_break`].
                fight.horde_break = true;
            }
            // The death beat: the per-kind controller queues its last-gasp
            // hazards (the mower's detonation) and its death cues.
            {
                let mut ctx = BossContext {
                    dt,
                    time: boss.elapsed,
                    player_pos: player.pos,
                    player_vel: player.vel,
                    player_radius: PLAYER_RADIUS,
                    arena: boss.arena,
                    level: &level,
                    flow: &flow,
                    rng: &mut fight.rng.0,
                };
                boss.kind.controller().death_tick(&mut ctx, &mut boss, &mut out);
            }
            emit_cues(
                &out.cues,
                &boss,
                level.palette.blood,
                &mut particles,
                &mut fight.rng.0,
                &mut sfx,
            );
            out.cues.clear();
            resolve_hazards(
                &mut boss,
                dt,
                &mut player,
                &mut enemies,
                &mut fight.scratch.enemy_order,
                &mut particles,
                &mut damage_numbers,
                &mut fight.rng.0,
                &mut sfx,
            );
            if boss.elapsed >= boss.duration {
                if !fight.level_complete {
                    fight.level_complete = true;
                    counters.level_complete = true;
                }
                if out.trace.on {
                    out.trace.finish(&boss, &player);
                }
                commands.entity(entity).despawn();
            }
            continue;
        }

        // Phase detection.
        let new_phase = boss.kind.phase_for(boss.hp_frac());
        if new_phase > boss.phase {
            boss.phase = new_phase;
            out.phase_changed = true;
        }

        // The state's own body-contact number: reset to the kind's default
        // every tick so a controller only has to say what is different.
        boss.contact_damage = boss.kind.contact_damage();

        // Rejoin rule: a boss that is off screen and has not closed on the rat
        // for [`RELOCATE_AFTER_SECS`] is walked back into the fight. A level
        // cannot end until the boss dies, so a boss that the horde flow field
        // cannot route back (or cannot outrun a kiting rat) must not be able
        // to strand the level. See [`RELOCATE_AFTER_SECS`].
        let to_player = player.pos - boss.pos;
        let no_los = level
            .tiles
            .segment_hit(boss.pos, player.pos, boss.kind.body_radius())
            .is_some();
        // A boss that is close enough and can see the rat but cannot *move*
        // (wedged on a wall jamb, or with no attack whose footprint fits where
        // it stands) is just as stranded as one across the map — that is the
        // "Broom(Idle) looping at 51 px with a wall in between" case. Its own
        // neutral beat is when it is supposed to be closing, so a neutral boss
        // that has not moved for a few ticks counts as out of the fight.
        fight.since_sample += dt;
        if fight.since_sample >= 0.5 {
            let moved = boss.pos.dist(fight.last_pos);
            fight.stuck = boss.state == boss.kind.initial_state() && moved < 1.5;
            fight.last_pos = boss.pos;
            fight.since_sample = 0.0;
        }
        let stuck = fight.stuck;
        if to_player.len() > REJOIN_DIST || no_los || stuck {
            boss.lost += dt;
            if boss.lost >= RELOCATE_AFTER_SECS {
                if let Some(spot) = rejoin_spot(&level, player.pos, boss.kind.body_radius()) {
                    boss.lost = 0.0;
                    boss.pos = spot;
                    boss.target = player.pos;
                    let to = player.pos - spot;
                    if to.len_sq() > 1e-6 {
                        boss.facing = to.normalized();
                    }
                    let accent = level.palette.accent;
                    vfx::emit_boss_spawn(&mut particles, &mut fight.rng.0, spot, accent);
                    shake.amount = shake.amount.max(8.0);
                    sfx.0.push(Sfx::Boss);
                }
            }
        } else {
            boss.lost = 0.0;
        }

        // Siege: a boss that is *walled off* (no body-passable straight line to
        // the rat) tears a hole and keeps coming. See [`SIEGE_AFTER_SECS`]. The
        // rejoin above is the first answer (it puts the boss near when nothing
        // is in the way); this is the answer for a rat that has kited somewhere
        // a large body cannot follow.
        //
        // Only worth smashing when the boss is *at* the wall that separates
        // them: a boss a screen away has a route (the flow field) and must walk
        // it, or the rejoin brings it near first. Without this gate a boss that
        // was left far away spent its whole fight tunnelling a hole every few
        // seconds instead of closing (seen as one 157 s Mower fight where the
        // rat was 1 039 px away for the first two minutes).
        //
        // The gate is the *wall*, not `stuck`: a boss pacing at a wall it
        // cannot pass moves a body-length every few seconds (a 14 px limit
        // cycle between two standable spots) and a per-window `stuck` test
        // reads that as progress forever. Measured on a merged-branch Skilled
        // run (seed 2478788577152286430, level 1): the Cat sat in `Stalk`
        // 122-173 px from the rat, `fit=false`, no line of sight, shuttling
        // 14 px every 3 s, for a 345 s fight with **zero** damage into it —
        // every 0.5 s window that ended in the hop cleared `stuck` and drained
        // the siege timer. Being walled off and near for four uninterrupted
        // seconds is the condition the siege was written for, whether or not
        // the pacing looks like motion.
        let near = to_player.len() < 220.0;
        let walled_off =
            !walkable_line(&level, boss.pos, player.pos, boss.kind.body_radius(), SizeClass::Large);
        if near && walled_off {
            fight.siege += dt;
            if fight.siege >= SIEGE_AFTER_SECS {
                if let Some(cell) = smash_target(&level, boss.pos, player.pos) {
                    smash_wall(&mut level, cell);
                    let hole = Vec2::new(
                        (cell.0 as f32 + 1.0) * collision::TILE_F,
                        (cell.1 as f32 + 1.0) * collision::TILE_F,
                    );
                    let normal = (boss.pos - hole).normalized();
                    vfx::emit_boss_mower_wall_impact(
                        &mut particles,
                        &mut fight.rng.0,
                        hole,
                        normal,
                        true,
                    );
                    shake.amount = shake.amount.max(10.0);
                    sfx.0.push(Sfx::MowerWallClang);
                }
                fight.siege = 0.0;
            }
        } else {
            fight.siege = 0.0;
        }

        // Controller tick. The block scopes the rng borrow so hazards can
        // draw from the same stream afterwards.
        {
            let mut ctx = BossContext {
                dt,
                time: boss.elapsed,
                player_pos: player.pos,
                player_vel: player.vel,
                player_radius: PLAYER_RADIUS,
                arena: boss.arena,
                level: &level,
                flow: &flow,
                rng: &mut fight.rng.0,
            };
            boss.kind.controller().tick(&mut ctx, &mut boss, &mut out);
        }

        // The controller's feedback requests, mapped to sound and particles.
        emit_cues(
            &out.cues,
            &boss,
            level.palette.blood,
            &mut particles,
            &mut fight.rng.0,
            &mut sfx,
        );
        out.cues.clear();

        // Hazards.
        resolve_hazards(
            &mut boss,
            dt,
            &mut player,
            &mut enemies,
            &mut fight.scratch.enemy_order,
            &mut particles,
            &mut damage_numbers,
            &mut fight.rng.0,
            &mut sfx,
        );

        // Body contact: only **grounded** hit shapes bump the rat (a lifted
        // slipper passes overhead), and only when the state deals contact
        // damage at all (`contact_damage == 0` is the art's "blades stopped" /
        // "safe pocket" state).
        if !boss.is_airborne() && boss.contact_cd <= 0.0 && boss.contact_damage > 0.0 {
            let shapes = boss.hit_shapes();
            if shapes.circle_hits_grounded(player.pos, PLAYER_RADIUS)
                && player.take_damage(boss.contact_damage) {
                    boss.contact_cd = 0.5;
                    sfx.0.push(Sfx::Hurt);
                    vfx::emit_player_hurt(&mut particles, &mut fight.rng.0, player.pos);
                }
        }

        // Spawn queued projectiles.
        for spawn in out.projectiles.iter() {
            let projectile = BossProjectile {
                kind: spawn.kind,
                pos: spawn.pos,
                prev: spawn.pos,
                vel: spawn.vel,
                radius: spawn.radius,
                damage: spawn.damage,
                life: spawn.life,
                bounces: spawn.bounces,
                anim: 0.0,
            };
            commands.spawn((projectile, next_spawn_id.take()));
        }

    }

    // Advance boss projectiles.
    for (entity, mut p) in projectiles.iter_mut() {
        p.anim += dt;
        p.life -= dt;
        if p.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        p.prev = p.pos;
        let next = p.pos + p.vel * dt;
        if level.tiles.segment_hit(p.pos, next, p.radius).is_some() {
            if p.bounces > 0 {
                p.bounces -= 1;
                p.vel = -p.vel;
                let vel = p.vel;
                p.pos += vel * dt;
            } else {
                commands.entity(entity).despawn();
                continue;
            }
        } else {
            p.pos = next;
        }
        if collision::segment_circle_hit(p.prev, p.pos, p.radius, player.pos, PLAYER_RADIUS)
            && player.take_damage(p.damage)
        {
            vfx::emit_player_hurt(&mut particles, &mut fight.rng.0, player.pos);
            sfx.0.push(Sfx::Hurt);
            commands.entity(entity).despawn();
        }
    }
}

// ============================================================================
// Debug tracing (`RATRUN_BOSS_TRACE`)
// ============================================================================

/// Env-gated per-fight diagnostics (development aid only).
///
/// `RATRUN_BOSS_TRACE=1 cargo run --release -- --bot-batch 1 --seed N --skill
/// skilled` prints one line per second of a boss fight — state, distance, hp,
/// damage into the boss split by source (the [`BossDamageTally`] the damage
/// system already books) and how long the boss has stood still in its neutral
/// state — plus a `BOSS-END` summary when it dies. It reads no rng and writes
/// only to stderr, so it can never change the simulation; leaving it compiled
/// in costs one `env::var` on the first tick of a run.
#[derive(Default)]
pub struct BossTrace {
    /// `RATRUN_BOSS_TRACE` is set (read once).
    on: bool,
    init: bool,
    /// Running damage-by-source total for this fight.
    dmg: BossDamageTally,
    /// Total hp the boss lost this fight (end-hp delta based).
    lost: f32,
    last_hp: f32,
    next_line: f32,
    /// Seconds spent in the neutral state without moving (see [`Self::sample`]).
    idle: f32,
    idle_max: f32,
    /// Shortest 0.5 s window of movement seen this fight is what `idle` counts;
    /// these are the window's own anchor.
    win_t: f32,
    win_pos: Vec2,
    /// Distance samples, for the mean stand-off.
    dist_sum: f32,
    dist_n: u32,
    /// State changes, for the summary.
    state_changes: u32,
    /// Previous sample, to measure self-movement.
    last_pos: Vec2,
    last_state: Option<BossState>,
    /// Fight clock, accumulated here because `Boss::elapsed` is the *state*
    /// clock and is reset on every transition.
    fight_time: f32,
    /// The boss this trace belongs to (its damage-number id).
    boss_id: u32,
    spawned: bool,
    /// The run ended with this boss alive (the rat died).
    aborted: bool,
    /// Lowest player hp seen during the fight (how close it came).
    php_min: f32,
    /// Damage dealt into the boss while it was in a punish window (the mower's
    /// `Flipped` / `Overheat`, the cat's `Hurt`), i.e. how much the boss's own
    /// counter is worth.
    punish_dmg: f32,
    /// Seconds spent in a punish window.
    punish_secs: f32,
    /// The large-class flow direction at the boss, copied in by the caller
    /// (development only).
    flow_hint: Vec2,
    /// Signature-counter accounting: `[cat pounces, cat overshoots (the dazed
    /// beat), broom staggers, mower flips]`.
    counters: [u32; 4],
    /// Damage dealt into the boss during the cat's overshoot daze (the window a
    /// dodged pounce opens).
    daze_dmg: f32,
    daze_secs: f32,
    /// Damage dealt while the mower is on its back (`Flipped`, art: 2x).
    flip_dmg: f32,
    flip_secs: f32,
}

impl BossTrace {
    /// Read the env var once per process.
    fn init(&mut self) {
        if self.init {
            return;
        }
        self.init = true;
        self.on = std::env::var_os("RATRUN_BOSS_TRACE").is_some_and(|v| v != "0");
    }

    /// Is this state the kind's neutral/approach beat (the one the boss is
    /// supposed to *leave* the moment it can attack)?
    fn neutral(&self, boss: &Boss) -> bool {
        boss.state == boss.kind.initial_state()
    }

    /// Is the boss in a window its signature counter opens (the designed
    /// "punish" beat)? Used only for the trace's counter accounting.
    fn punish_window(boss: &Boss) -> bool {
        matches!(
            boss.state,
            BossState::Cat(BossCatState::Hurt)
                | BossState::Broom(BroomBossState::Hurt { .. })
                | BossState::Mower(MowerState::Flipped | MowerState::Overheat)
                | BossState::NightEdition(NightEditionState::BedLowered | NightEditionState::Ribbon)
        )
    }

    /// One tick of tracing. Called before the controller tick, so it reads the
    /// state and position the previous tick left behind.
    fn sample(
        &mut self,
        boss: &Boss,
        player: &Player,
        level: &Level,
        dt: f32,
        dmg: &BossDamageTally,
    ) {
        if !self.spawned || self.boss_id != boss.id {
            // First tick of the fight: reset the accumulator.
            self.spawned = true;
            self.aborted = false;
            self.boss_id = boss.id;
            self.fight_time = 0.0;
            self.dmg = BossDamageTally::default();
            self.lost = 0.0;
            self.next_line = 1.0;
            self.idle = 0.0;
            self.idle_max = 0.0;
            self.dist_sum = 0.0;
            self.dist_n = 0;
            self.state_changes = 0;
            self.counters = [0; 4];
            self.daze_dmg = 0.0;
            self.daze_secs = 0.0;
            self.flip_dmg = 0.0;
            self.flip_secs = 0.0;
            self.php_min = player.hp;
            self.punish_dmg = 0.0;
            self.punish_secs = 0.0;
            self.last_hp = boss.hp;
            self.last_pos = boss.pos;
            self.win_pos = boss.pos;
            self.win_t = 0.0;
            self.last_state = Some(boss.state);
            eprintln!(
                "BOSS-SPAWN kind={:?} hp={:.0} lvl={}",
                boss.kind, boss.max_hp, player.level
            );
            return;
        }
        if !self.aborted && player.hp <= 0.0 {
            // The rat died with the boss alive: the fight has no winner, but its
            // length and the damage booked into the boss are still informative.
            self.aborted = true;
            let secs = self.fight_time;
            eprintln!(
                "BOSS-ABORT kind={:?} secs={:.1} dealt={:.0} dps={:.0} mean_dist={:.0} \
                 idle_max={:.1} states={} lvl={} hp_frac={:.2} php_min={:.0} punish_dmg={:.0} punish_secs={:.1}",
                boss.kind,
                secs,
                self.lost,
                self.lost / secs.max(0.01),
                self.dist_sum / self.dist_n.max(1) as f32,
                self.idle_max,
                self.state_changes,
                player.level,
                boss.hp_frac(),
                self.php_min,
                self.punish_dmg,
                self.punish_secs,
            );
            return;
        }
        // Damage booked this tick (the damage system ran just before us).
        self.fight_time += dt;
        self.dmg.add(*dmg);
        let dealt = (self.last_hp - boss.hp).max(0.0);
        self.lost += dealt;
        self.last_hp = boss.hp;
        self.php_min = self.php_min.min(player.hp);
        if Self::punish_window(boss) {
            self.punish_dmg += dealt;
            self.punish_secs += dt;
        }
        if matches!(boss.state, BossState::Cat(BossCatState::Stalk)) && boss.duration < 1.0 {
            self.daze_dmg += dealt;
            self.daze_secs += dt;
        }
        if matches!(boss.state, BossState::Mower(MowerState::Flipped)) {
            self.flip_dmg += dealt;
            self.flip_secs += dt;
        }
        self.dist_sum += boss.pos.dist(player.pos);
        self.dist_n += 1;
        if self.last_state != Some(boss.state) {
            self.state_changes += 1;
            self.last_state = Some(boss.state);
            // Signature counters: a pounce to dodge (counted on entry), the
            // 0.7 s dazed beat that a dodged pounce leaves (the cat's own
            // `Stalk` beat is 1.4-3.5 s, so a short one is the overshoot),
            // the broom's stubbed-toe stagger, and the mower's wheels-up
            // window.
            match boss.state {
                BossState::Cat(BossCatState::Pounce) => self.counters[0] += 1,
                BossState::Cat(BossCatState::Stalk) if boss.duration < 1.0 => {
                    self.counters[1] += 1
                }
                BossState::Broom(BroomBossState::Hurt { .. }) => self.counters[2] += 1,
                BossState::Mower(MowerState::Flipped) => self.counters[3] += 1,
                _ => {}
            }
        }
        // Idle is measured over 0.5 s windows: a boss wedged on a corner
        // jitters a fraction of a pixel per tick (a few px/s of noise, no
        // progress), so a per-tick threshold calls it "moving" forever.
        self.win_t += dt;
        if self.win_t >= 0.5 {
            let window_moved = boss.pos.dist(self.win_pos);
            self.win_pos = boss.pos;
            self.win_t = 0.0;
            if self.neutral(boss) && window_moved < 3.0 {
                self.idle += 0.5;
                self.idle_max = self.idle_max.max(self.idle);
            } else {
                self.idle = 0.0;
            }
        }
        if self.fight_time >= self.next_line {
            self.next_line += 1.0;
            eprintln!(
                "BOSS t={:.1} kind={:?} state={:?} p={:.2} dist={:.0} hp={:.0}/{:.0} \
                 dmg={:.0} (aim {:.0} pat {:.0} whip {:.0} pool {:.0}) idle={:.1} lvl={} php={:.0} \
                 bpos={:.0},{:.0} ppos={:.0},{:.0} los={} fit={} flow={:?} hunt={} inarena={}",
                self.fight_time,
                boss.kind,
                boss.state,
                boss.progress(),
                boss.pos.dist(player.pos),
                boss.hp,
                boss.max_hp,
                self.lost,
                self.dmg.aimed,
                self.dmg.pattern,
                self.dmg.whip,
                self.dmg.pools,
                self.idle,
                player.level,
                player.hp,
                boss.pos.x,
                boss.pos.y,
                player.pos.x,
                player.pos.y,
                level
                    .tiles
                    .segment_hit(boss.pos, player.pos, boss.kind.body_radius())
                    .is_none(),
                level.tiles.circle_clear(
                    boss.pos + (player.pos - boss.pos).normalized() * 2.0,
                    boss.kind.body_radius(),
                    SizeClass::Large,
                ),
                self.flow_hint,
                boss.hunting,
                boss.arena.contains_world(boss.pos),
            );
        }
    }

    /// The fight ended (the boss died): one summary line.
    fn finish(&mut self, boss: &Boss, player: &Player) {
        let secs = self.fight_time;
        self.spawned = false;
        eprintln!(
            "BOSS-END kind={:?} secs={:.1} dealt={:.0} dps={:.0} mean_dist={:.0} \
             idle_max={:.1} states={} lvl={} hp={:.0} php_min={:.0} punish_dmg={:.0} punish_secs={:.1} \
             pounce={} overshoot={} stagger={} flip={} daze_dmg={:.0} daze_secs={:.1} flip_dmg={:.0} flip_secs={:.1}",
            boss.kind,
            secs,
            self.lost,
            self.lost / secs.max(0.01),
            self.dist_sum / self.dist_n.max(1) as f32,
            self.idle_max,
            self.state_changes,
            player.level,
            player.hp,
            self.php_min,
            self.punish_dmg,
            self.punish_secs,
            self.counters[0],
            self.counters[1],
            self.counters[2],
            self.counters[3],
            self.daze_dmg,
            self.daze_secs,
            self.flip_dmg,
            self.flip_secs,
        );
    }
}

// ============================================================================
// Small helpers
// ============================================================================

/// Did this projectile come from a weapon that picks its own target?
///
/// The aiming families in `weapons::nearest_visible` are Gnaw, Cheese Spray and
/// Acid Spit (whose lob never damages in flight); Squeak Nova and Spore Orbit
/// fire on a pattern around the rat and never look at a body. Only used for the
/// balance attribution in [`BossScratch`].
#[inline]
fn is_aimed_weapon(kind: Option<WeaponKind>) -> bool {
    matches!(
        kind,
        Some(WeaponKind::Gnaw | WeaponKind::CheeseSpray | WeaponKind::AcidSpit)
    )
}

#[inline]
fn s_radius(shape: &BossHitShape) -> f32 {
    match *shape {
        BossHitShape::Circle { radius, .. } => radius,
        BossHitShape::Box { half, half_perp, .. } => half.max(half_perp),
    }
}

/// Circle-vs-oriented-rect test for the mousetrap kill rectangle.
#[inline]
fn in_mouse_kill_rect(pos: Vec2, facing: Vec2, point: Vec2, radius: f32) -> bool {
    local_box_contains(
        pos,
        facing,
        point,
        MOUSETRAP_KILL_U_MIN - radius,
        MOUSETRAP_KILL_U_MAX + radius,
        MOUSETRAP_KILL_HALF_V + radius,
    )
}

#[cfg(test)]
mod tests;
