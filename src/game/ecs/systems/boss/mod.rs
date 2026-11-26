//! Boss fight framework (P7).
//!
//! One shared tick system drives the themed bosses:
//!
//! | level | theme | boss | art module |
//! |-------|-------|------|-----------|
//! | 0 | Sewer | Jungle Cat (Apex Jaguar) | [`crate::game::art_boss_cat`] |
//! | 0 | Sewer | The Horseshoe (horseshoe crab) | [`crate::game::art_boss_horseshoe_crab`] |
//! | 1 | Kitchen | The Sweeper (broom) / SUBJECT R-01 (megarat) | [`crate::game::art_boss_broom`], [`crate::game::art_boss_megarat`] |
//! | 2 | Lab | TURF-REAPER MK.IV (mower) / KING COBRA | [`crate::game::art_boss_mower`], [`crate::game::art_boss_cobra`] |
//! | 3 | Laundry | The Mangle (wringer) | [`crate::game::art_boss_mangle`] |
//! | 4 | Printworks | THE NIGHT EDITION (press) | [`crate::game::art_boss_night_edition`] |
//! | — (getaway) | Flatbed | The Magpie (bird) | [`crate::game::art_boss_magpie`] |
//! | — (getaway) | Flatbed | The Redtail (hawk) | [`crate::game::art_boss_hawk`] |
//!
//! Every floor's boss comes from [`BossKind::pool_for_theme`] + the level seed
//! ([`BossKind::pool_pick`]; `for_theme` is the first entry, the floor's
//! original boss) (the side-route floors
//! and the getaway ride play at a campaign stage index, so `for_level` cannot
//! name their fights); the flatbed's arm goes through [`trailer_boss`].
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
//! * [`Boss::grant_control`] / [`Boss::shove`] — the boss **control budget**
//!   (batch 13, see below).
//!
//! # The control budget (batch 13: no boss may be stun-locked)
//!
//! A player weapon may hurt a boss, slow it a little and nudge it a few pixels;
//! it may not take the fight away from it. Two rules, both enforced here so
//! every kind inherits them and no weapon file has to know about them:
//!
//! 1. **A committed beat is never touched.** Once a boss leaves its neutral
//!    state ([`BossKind::initial_state`]) for a tell, a wind-up, the strike or
//!    the recovery that follows it, nothing a player weapon does may cancel it,
//!    restart it or displace the body. [`Boss::committed`] is that test;
//!    [`Boss::grant_control`] and [`Boss::shove`] both refuse while it holds.
//! 2. **Control has a hard budget.** A boss may spend at most
//!    [`CONTROL_BUDGET_SECS`] (0.30 s) of stagger in any
//!    [`CONTROL_WINDOW_SECS`] (4 s) window, and be displaced at most
//!    [`SHOVE_PER_HIT_PX`] (4 px) a hit and [`SHOVE_PER_SEC_PX`] (12 px) a
//!    second. A controller asks for the *art's* duration and gets back what the
//!    budget can pay ([`Boss::grant_control`] returns the granted seconds, 0 =
//!    refused); `resolve_hazards` is untouched, so hazards already painted
//!    always finish.
//!
//! The measured failure this exists for: the Apex Jaguar's `Hurt` beat was
//! re-tripped by any 55-damage tick (`cat::HURT_BURST`), and a max-rank Spiked
//! Tail (44 lash + 18 burst on the same tick, every 0.85 s) or one Tail Thrash
//! sweep (52 at L5, three sweeps in a second) clears that on every swing. Every
//! `SwipeWindup` (0.55 s) and every `Crouch` (0.95 s) was cancelled before it
//! could fire, so the cat never attacked at all. The same rule now also covers
//! the Sweeper's poise hop, which could cancel a rake.
//!
//! # The pursuit rule (a kited boss closes instead of idling)
//!
//! A rat outruns most bosses (58-114 px/s against the cat's 24 px/s stalk) and
//! the reach of its own weapons is longer than the boss's engage range (the
//! Spiked Tail lashes at 80-120 px, the Laser beam reaches 54-86 px, the
//! jaguar's pounce band ends at 95 px). A boss that cannot reach its target in
//! [`OUT_OF_REACH_SECS`] (3 s) of its own neutral state stops idling: it
//! **sprints** at [`PURSUIT_SPEED`] and, when the line is body-passable,
//! **leaps** ([`LEAP_SPEED`] for [`LEAP_SECS`], [`LEAP_COOLDOWN`] between
//! leaps). The Cat additionally uses its own `Crouch`/`Pounce` art as a closing
//! leap (see `cat::start_attack`).
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
//! * No controller (one module per `BossKind`) takes `Commands`:
//!   they move the boss, push hazards and queue projectiles/cues, so they
//!   cannot spawn or despawn an entity.
//!
//! `boss::tests::boss_death_breaks_the_horde_without_invalidating_the_enemy_index`
//! locks the whole thing down through the real schedule.

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::audio::Sfx;
use crate::detmath::DetF32;
use crate::game::art_boss_broom::{self, BroomBossState, BroomBossView};
use crate::game::art_boss_hawk::{self, HawkState, HawkView};
use crate::game::art_boss_cobra::{self, CobraState, CobraView};
use crate::game::art_boss_horseshoe_crab::{self, CrabState, CrabView};
use crate::game::art_boss_mangle::{self, MangleState, MangleView};
use crate::game::art_boss_cat::{self, BossCatState, BossCatView};
use crate::game::art_boss_magpie::{self, MagpieState, MagpieView};
use crate::game::art_boss_megarat::{self, MegaRatState, MegaRatView};
use crate::game::art_boss_mower::{self, MowerState, MowerView};
use crate::game::art_boss_night_edition::{self, NightEditionState, NightEditionView};
use crate::game::art_boss_lantern::{self, LanternState, LanternView};
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
    BossDue, BossOverride, DifficultyLevel, NextEnemyId, NextSpawnId, RunCounters, Shake, SfxQueue,
    TickInput, Viewport,
};
use crate::game::ecs::systems::spawn::SpawnScratch;
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
pub mod hawk;
pub mod cobra;
pub mod horseshoe_crab;
pub mod magpie;
pub mod mangle;
pub mod megarat;
pub mod mower;
pub mod night_edition;
pub mod lantern;

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
    /// Laundry — The Mangle (cast-iron wringer).
    Mangle,
    /// Printworks — THE NIGHT EDITION (the walking offset press).
    NightEdition,
    /// The Flatbed (getaway ride) — The Magpie.
    Magpie,
    /// Sewer pool mate of the Jungle Cat — The Horseshoe (horseshoe crab).
    HorseshoeCrab,
    /// The Flatbed's pool mate — **The Redtail**, the hawk that has been
    /// following the truck.
    ///
    /// A predator from above: it arrives as a shadow (no hit shapes at all
    /// while it is high), dive bombs a talon reticle onto the deck, and
    /// sometimes stoops to a low hover where it is fully hittable and reaches
    /// down with feather darts, a marked grab or a strafing talon rake. See
    /// [`hawk`] and [`crate::game::art_boss_hawk`].
    Hawk,
    /// Kitchen's second fight — SUBJECT R-01, the Megarat.
    ///
    /// The facility's first success: a rat the size of the room, collared and
    /// tagged, that fights with twisted copies of the player's own kit. See
    /// [`megarat`] and [`crate::game::art_boss_megarat`].
    MegaRat,
    /// The Lab's second fight — **KING COBRA**.
    ///
    /// An escaped specimen far longer than the arena is wide: its coils are the
    /// arena's moving walls, its hood's back is its one weak point, and in its
    /// last third it lays the whole room out as a spiral lane and comes down it.
    /// See [`cobra`] and [`crate::game::art_boss_cobra`].
    Cobra,
    /// The Outfall's anglerfish.
    Lantern,
}

/// The boss the getaway ride fields: **The Magpie**.
///
/// The flatbed's own fight (see [`magpie`] and [`crate::game::art_boss_magpie`]):
/// a magpie riding the slipstream over the cab hold, pecking, strafing the deck
/// and beating its wings against the trailer's own wind. Before the flatbed had
/// a fight of its own this borrowed the Lab's mower.
pub fn trailer_boss() -> BossKind {
    BossKind::Magpie
}

// The floors' boss pools (`BossKind::pool_for_theme`). Const slices so the
// accessor returns `&'static [BossKind]` without promoting a call expression;
// parallel boss workers append their own kind to their floor's list.
const SEWER_POOL: &[BossKind] = &[BossKind::Cat, BossKind::HorseshoeCrab];
const KITCHEN_POOL: &[BossKind] = &[BossKind::Broom, BossKind::MegaRat];
const LAB_POOL: &[BossKind] = &[BossKind::Mower, BossKind::Cobra];
const LAUNDRY_POOL: &[BossKind] = &[BossKind::Mangle];
const PRINTWORKS_POOL: &[BossKind] = &[BossKind::NightEdition];
const TRAILER_POOL: &[BossKind] = &[BossKind::Magpie, BossKind::Hawk];
const OUTFALL_POOL: &[BossKind] = &[BossKind::Lantern];

impl BossKind {
    /// The floor's **original** boss — the pinned, seedless choice.
    ///
    /// **The wave 3 seam.** `boss_spawn_system` decides a floor's boss with
    /// [`Self::pool_pick`] (theme + level seed); this is that pool's first
    /// entry, which is what every caller with no seed to spend means (the HUD's
    /// default plate, `for_level`'s campaign mapping, tests that pin a boss).
    /// The Laundry fields the Mangle and the Printworks the Night Edition.
    /// Nothing else maps themes to bosses (the flatbed's own arm goes through
    /// [`trailer_boss`]).
    ///
    /// The *spawn* no longer calls this: a floor may field more than one boss,
    /// so `boss_spawn_system` goes through [`BossKind::for_theme_seeded`] and
    /// this stays the floor's canonical name for the HUD, the campaign screens
    /// and the flatbed's arm.
    #[inline]
    pub fn for_theme(theme: Theme) -> Self {
        // The floor's *original* boss: the pinned choice, and what every caller
        // that has no level seed to spend means. The spawn system uses
        // [`Self::pool_pick`] instead.
        Self::pool_for_theme(theme)[0]
    }

    /// Every boss a floor can field, in preference order. The first entry is
    /// the floor's original boss (the one `for_theme` names and the one every
    /// pre-pool seed expected); later entries are pool mates.
    ///
    /// **The wave 3 pool seam.** Parallel boss workers append their own entry
    /// to their floor's pool; the integrator unions the lists. A floor with one
    /// entry is pinned to it, and no rng is drawn: the draw is a stable hash of
    /// the level seed, so a pooled floor spends no rng either.
    #[inline]
    pub fn pool_for_theme(theme: Theme) -> &'static [BossKind] {
        match theme {
            Theme::Sewer => SEWER_POOL,
            Theme::Kitchen => KITCHEN_POOL,
            Theme::Lab => LAB_POOL,
            Theme::Laundry => LAUNDRY_POOL,
            Theme::Printworks => PRINTWORKS_POOL,
            Theme::Trailer => TRAILER_POOL,
            Theme::Outfall => OUTFALL_POOL,
        }
    }

    /// The boss a floor fields for a level seed: a pooled floor picks one of
    /// [`Self::pool_for_theme`] with a stable splitmix64 hash of the seed.
    ///
    /// Deterministic by construction (no rng draw, so it cannot perturb the run
    /// stream), uniform enough that both Sewer bosses appear across seeds, and
    /// stable across builds. Tests that need a particular boss pin the seed and
    /// assert [`Self::pool_pick`] rather than guessing.
    #[inline]
    pub fn pool_pick(theme: Theme, seed: u64) -> Self {
        let pool = Self::pool_for_theme(theme);
        if pool.len() <= 1 {
            return pool[0];
        }
        // splitmix64 finaliser over the seed and the theme, so two floors that
        // share a seed (a campaign links floors by seed) do not correlate.
        let mut h = seed ^ ((theme as u64).wrapping_add(1)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 30;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        pool[(h % pool.len() as u64) as usize]
    }

    /// The boss a floor fields for a level seed (the Megarat branch's name for
    /// [`Self::pool_pick`]; one implementation, two names).
    #[inline]
    pub fn for_theme_seeded(theme: Theme, seed: u64) -> Self {
        Self::pool_pick(theme, seed)
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
            BossKind::Mangle => art_boss_mangle::BOSS_BASE_HP,
            BossKind::NightEdition => art_boss_night_edition::BOSS_BASE_HP,
            BossKind::Magpie => art_boss_magpie::BOSS_BASE_HP,
            BossKind::HorseshoeCrab => art_boss_horseshoe_crab::BOSS_BASE_HP,
            BossKind::MegaRat => art_boss_megarat::BOSS_BASE_HP,
            BossKind::Hawk => art_boss_hawk::BOSS_BASE_HP,
            BossKind::Cobra => art_boss_cobra::BOSS_BASE_HP,
            BossKind::Lantern => art_boss_lantern::BOSS_BASE_HP,
        }
    }

    /// Single-circle collision radius used for movement and contact.
    #[inline]
    pub fn body_radius(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::BODY_RADIUS,
            BossKind::Broom => art_boss_broom::STANCE_HALF_WIDTH + art_boss_broom::FOOT_HIT_RADIUS,
            BossKind::Mower => art_boss_mower::BODY_RADIUS,
            BossKind::Mangle => art_boss_mangle::BODY_RADIUS,
            BossKind::NightEdition => art_boss_night_edition::BODY_RADIUS,
            BossKind::Magpie => art_boss_magpie::BODY_RADIUS,
            BossKind::HorseshoeCrab => art_boss_horseshoe_crab::BODY_RADIUS,
            BossKind::MegaRat => art_boss_megarat::BODY_RADIUS,
            BossKind::Hawk => art_boss_hawk::BODY_RADIUS,
            BossKind::Cobra => art_boss_cobra::BODY_RADIUS,
            BossKind::Lantern => art_boss_lantern::HEAD_RADIUS,
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
    ///
    /// **Batch seven, measured** (`--bot-batch 96 --cycle-skills`, the two
    /// side routes with `--route`, the Flatbed with `--practice --level 4`;
    /// fight length is boss-alive seconds, walk to the arena included). The
    /// numbers the comments on the arms below call "blind" are superseded by
    /// this table:
    ///
    /// | boss | scale | pool | Skilled median | fastest Skilled |
    /// |---|---:|---:|---:|---:|
    /// | Cat | 1.4 | 24,640 | 80 s | 25 s |
    /// | Horseshoe | 0.70 → 1.65 | 26,400 | 58 s (was 23) | 42 s |
    /// | Broom | 1.95 → 0.7 | 8,960 | 79 s (was 136) | 30 s |
    /// | MegaRat | 1.7 → 1.5 | 19,200 | 57 s (was 78) | 25 s |
    /// | Mangle | 1.1 → 1.55 | 19,840 | 67 s (was 51) | 26 s |
    /// | Mower | 1.8 → 1.0 | 12,800 | 66 s (was 132) | 27 s |
    /// | Cobra | 0.35 → 1.0 | 14,400 | 64 s (was 22) | 25 s |
    /// | Night Edition | 0.85 → 0.62 | 14,880 | 59 s (was 84) | 32 s |
    /// | Magpie | 0.85 → 1.45 | 18,560 | 50 s (was 39) | 47 s |
    /// | Hawk | 0.4 → 0.8 | 10,240 | 75 s (was 27) | 62 s |
    ///
    /// The Broom's and the Mower's pools are small because the rat lands
    /// little on them (one planted slipper at a time; a patrol that holds
    /// 45-85 px and a charge that carries it away): about 150-200 dps against
    /// 370-470 into the Cat or R-01. The Mower also lost a third of its patrol
    /// beat and half its dock heal (`mower.rs`).
    #[inline]
    pub fn hp_scale(self) -> f32 {
        match self {
            BossKind::Cat => 1.4,
            BossKind::Broom => 0.7,
            // Batch-6: 2.0 -> 1.8. The dock's recharge can no longer be
            // cancelled by a single hit (`mower::DOCK_INTERRUPT_FRAC`), so a
            // phase change now really hands back a few percent of the pool;
            // the 10 % cut keeps the measured ~80 s median where it was.
            // Blind, not re-measured.
            BossKind::Mower => 1.0,
            // The Laundry is a stage-two floor. Measured with the same
            // near-complete bot build that fights the Sweeper and the Mower,
            // the Mangle at 1.95 died in 19 s against their 33 s / 26 s: its
            // jam windows are a 1.5x damage window for a third of every
            // rotation, so the raw pool has to be larger to buy the same
            // fight. 3.35 lands it beside the Sweeper (see the report).
            // **Batch-6 re-scale (blind, not measured).** The reported
            // bot-batch fight ran ~185 s on the old 3.35 pool, i.e. about
            // 232 dps actually landed — far below the build the old number
            // was tuned against. Scaled linearly to a ~60 s fight; the next
            // batch run owns the final number.
            BossKind::Mangle => 1.55,
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
            // **Batch-6 re-scale (blind, not measured).** The 39.5-45.6 s
            // above is the near-complete harness build (~1.1 k dps). The
            // campaign bot lands ~320 dps into the other stage-3 fight (the
            // Mower: 25,600 hp, ~80 s median), where 2.0 would be a ~150 s
            // press. 0.85 is ~20,400 hp, about 64 s at that rate.
            BossKind::NightEdition => 0.62,
            // Measured with `magpie::tests::a_bot_kills_the_magpie_on_the_real_deck`
            // (the same method the campaign bosses were tuned with, applied to
            // the flatbed: the Expert bot with a carried build, spawn to death
            // on the real deck, five seeds). At `2.0` the bird died in 23.4 s
            // (mean), i.e. the bot lands ~1.1 k dps into it — the flatbed is a
            // 15-minute practice floor reached with a full build and a small,
            // always-reachable body, so it takes a much bigger pool than the
            // campaign fights to hold its 45-80 s beat. `4.6` lands the mean at
            // ~54 s; see the shipped measurement in the test.
            // **Batch-6 re-scale (blind, not measured).** The reported
            // bot-batch fight ran ~340 s on the old 4.6 pool, i.e. about
            // 173 dps actually landed — far below the build the old number
            // was tuned against. Scaled linearly to a ~60 s fight; the next
            // batch run owns the final number.
            BossKind::Magpie => 1.45,
            // The Sewer pool mate, and the only boss whose *pool* is smaller
            // than its art's: the dome sheds ~45 % of the rat's fire on any
            // run that does not flank it, so the raw number already buys a
            // longer fight. Measured with
            // `boss::horseshoe_crab::tests::a_bot_kills_the_horseshoe_on_the_real_arena`
            // (the same Expert bot + Typical carry the other boss fights were
            // tuned with, five Sewer seeds, spawn to death): at the Cat's 1.4
            // the fight ran 127.5 s mean (the bot orbits into the dome and
            // loses half its damage); `0.70` lands it in the 45-80 s band at
            // ~64 s. A balance pass owns the final number.
            BossKind::HorseshoeCrab => 1.65,
            // SUBJECT R-01 is the Kitchen's *second* fight and the mirror
            // fight: every rotation spends a third of its time in a punish
            // window (a ×1.35 belly, a ×1.25 tail base) and it calls three
            // rats at each phase change, so part of the rat's fire goes into
            // the room instead of the boss. Assumed band (the merged-campaign
            // numbers this file quotes for the Sweeper and the Mangle): a
            // near-complete build lands 250-600 dps into a boss, so
            // 1,600 × 2.6 × the framework's ×8 lands ~33 k and a 55-80 s fight
            // once the windows are spent. Provisional: the balance pass owns
            // the final number. Measured with
            // `megarat::tests::a_bot_kills_the_megarat_on_the_real_arena`.
            // **Batch-6 re-scale (blind, not measured).** The campaign bot
            // lands ~350 dps into the other Kitchen fight (the Sweeper:
            // 24,960 hp, ~72 s median); at 2.6 this pool is 33,280 hp, a
            // ~96 s fight before the called rats take their share. 1.7 is
            // 21,760 hp, about 62 s at that rate.
            BossKind::MegaRat => 1.5,
            // The flatbed's second bird, expressed like the first: art pool ×
            // scale × the framework's ×8. **Assumed, not measured** (the wave-3
            // rule is no bot batches for content workers): a near-complete
            // build lands ~1.1 k dps into the magpie, and the hawk is *only
            // hittable while it is low* — roughly 45 % of a rotation (the
            // stoop, the hover, its attacks and the buried punish window) —
            // so the raw pool has to be a little over half the magpie's for the
            // same 45-80 s beat. 1,600 × 2.6 lands the fight beside the Magpie's
            // ~54 s at the assumed uptime; a balance pass owns the number. See
            // `hawk::tests::a_bot_kills_the_hawk_on_the_real_deck` for the
            // measurement that must be taken.
            // **Batch-6 re-scale (blind, not measured).** The magpie's reported
            // fight puts the flatbed bot at ~173 dps into an always-hittable
            // bird, not the ~1.1 k assumed above; at the hawk's ~45 % uptime
            // that is ~78 dps, and 2.6 would be a seven-minute fight. 0.4 is
            // 5,120 hp, about 65 s at that rate.
            BossKind::Hawk => 0.8,
            // The Lab's *second* fight, and the one that spends the most of
            // every rotation in a punish window: `FangsStuck` is a x1.5 window
            // after every strike (twice per rotation from phase 2) and
            // `SpiralEnd` a x1.4 one after the third phase's set piece. Against
            // the merged-campaign band this file quotes for the Sweeper and the
            // Mangle (a near-complete build lands 250-600 dps into a boss),
            // 1,800 x 2.0 x the framework's x8 lands ~29 k, i.e. the same
            // 45-80 s beat the Mower holds at 3,200 with no weak point at all.
            // Provisional: the balance pass owns the final number.
            //
            // Measured at integration (`cobra::tests::a_bot_kills_the_king_cobra_
            // on_the_real_arena`, the Typical melee carry): at 2.0 the bot had
            // taken 80 % of the pool after 300 s (about 77 dps landed, not the
            // 250-600 assumed above, because the animal holds a 70 px stand-off
            // and a short-range kit only reaches it during its strikes). Halved
            // so the fight ends; it is still long for a melee build and the
            // stand-off band is the thing a balance pass should look at.
            // **Batch-6 re-scale (blind, not measured).** The reported
            // bot-batch fight ran ~183 s on the old 1.0 pool, i.e. about
            // 79 dps actually landed — far below the build the old number
            // was tuned against. Scaled linearly to a ~60 s fight; the next
            // batch run owns the final number.
            BossKind::Cobra => 1.0,
            BossKind::Lantern => art_boss_lantern::BOSS_HP_SCALE,
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
            BossKind::Mangle => art_boss_mangle::CONTACT_DAMAGE,
            // The design's own number for this fight (`§12`: body contact 8).
            BossKind::NightEdition => art_boss_night_edition::CONTACT_DAMAGE,
            BossKind::Magpie => art_boss_magpie::CONTACT_DAMAGE,
            BossKind::HorseshoeCrab => art_boss_horseshoe_crab::CONTACT_DAMAGE,
            BossKind::MegaRat => art_boss_megarat::CONTACT_DAMAGE,
            BossKind::Hawk => art_boss_hawk::CONTACT_DAMAGE,
            BossKind::Cobra => art_boss_cobra::CONTACT_DAMAGE,
            BossKind::Lantern => art_boss_lantern::CONTACT_DAMAGE,
        }
    }

    /// Death-state duration in seconds.
    #[inline]
    pub fn death_duration(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::DEATH_DURATION,
            BossKind::Broom => 2.40,
            BossKind::Mower => 3.4,
            BossKind::Mangle => art_boss_mangle::DEATH_DURATION,
            BossKind::NightEdition => art_boss_night_edition::DEATH_DURATION,
            BossKind::Magpie => art_boss_magpie::DEATH_DURATION,
            BossKind::HorseshoeCrab => art_boss_horseshoe_crab::DEATH_DURATION,
            BossKind::MegaRat => art_boss_megarat::DEATH_DURATION,
            BossKind::Hawk => art_boss_hawk::DEATH_DURATION,
            BossKind::Cobra => art_boss_cobra::DEATH_DURATION,
            BossKind::Lantern => art_boss_lantern::DEATH_DURATION,
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
            // The Mangle's design thresholds: above 60 %, 60-25 %, below 25 %.
            BossKind::Mangle => {
                if hp_frac <= 0.25 {
                    3
                } else if hp_frac <= 0.60 {
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
            BossKind::MegaRat => {
                if hp_frac <= art_boss_megarat::PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= art_boss_megarat::PHASE2_HP_FRAC {
                    2
                } else {
                    1
                }
            }
            BossKind::Magpie => {
                if hp_frac <= art_boss_magpie::PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= art_boss_magpie::PHASE2_HP_FRAC {
                    2
                } else {
                    1
                }
            }
            BossKind::HorseshoeCrab => {
                if hp_frac <= art_boss_horseshoe_crab::PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= art_boss_horseshoe_crab::PHASE2_HP_FRAC {
                    2
                } else {
                    1
                }
            }
            BossKind::Hawk => {
                if hp_frac <= art_boss_hawk::PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= art_boss_hawk::PHASE2_HP_FRAC {
                    2
                } else {
                    1
                }
            }
            // The sway opens at 66 %, the laid spiral at 33 %.
            BossKind::Cobra => {
                if hp_frac <= art_boss_cobra::PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= art_boss_cobra::PHASE2_HP_FRAC {
                    2
                } else {
                    1
                }
            }
            BossKind::Lantern => {
                if hp_frac <= art_boss_lantern::PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= art_boss_lantern::PHASE2_HP_FRAC {
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
            BossKind::Mangle => BossState::Mangle(MangleState::Idle),
            BossKind::NightEdition => BossState::NightEdition(NightEditionState::Roll),
            BossKind::Magpie => BossState::Magpie(MagpieState::Perch),
            BossKind::HorseshoeCrab => BossState::HorseshoeCrab(CrabState::Dome),
            BossKind::MegaRat => BossState::MegaRat(MegaRatState::Groom),
            // It arrives as a shadow: the first state is the circling overhead.
            BossKind::Hawk => BossState::Hawk(HawkState::Circle),
            BossKind::Cobra => BossState::Cobra(CobraState::Rear),
            BossKind::Lantern => BossState::Lantern(LanternState::ArrivalLure),
        }
    }

    /// The first state's duration (the art's neutral-state beat).
    #[inline]
    pub fn initial_duration(self) -> f32 {
        match self {
            BossKind::Cat => 2.6,
            BossKind::Broom => 0.55,
            BossKind::Mower => 1.80,
            BossKind::Mangle => art_boss_mangle::IDLE_P1,
            // The first walk beat is the phase-1 one (`night_edition::ROLL_DURATION[0]`).
            BossKind::NightEdition => 1.8,
            BossKind::Magpie => art_boss_magpie::PERCH_DURATION[0],
            BossKind::HorseshoeCrab => art_boss_horseshoe_crab::DOME_DURATION[0],
            BossKind::MegaRat => art_boss_megarat::GROOM_DURATION[0],
            BossKind::Hawk => art_boss_hawk::CIRCLE_DURATION[0],
            BossKind::Cobra => art_boss_cobra::REAR_DURATION[0],
            BossKind::Lantern => art_boss_lantern::ARRIVAL_DURATION,
        }
    }

    /// The art's death/defeat state.
    #[inline]
    pub fn death_state(self) -> BossState {
        match self {
            BossKind::Cat => BossState::Cat(BossCatState::Death),
            BossKind::Broom => BossState::Broom(BroomBossState::Defeat),
            BossKind::Mower => BossState::Mower(MowerState::Dying),
            BossKind::Mangle => BossState::Mangle(MangleState::Death),
            BossKind::NightEdition => BossState::NightEdition(NightEditionState::Wreck),
            BossKind::Magpie => BossState::Magpie(MagpieState::Death),
            BossKind::HorseshoeCrab => BossState::HorseshoeCrab(CrabState::Death),
            BossKind::MegaRat => BossState::MegaRat(MegaRatState::Death),
            BossKind::Hawk => BossState::Hawk(HawkState::Death),
            BossKind::Cobra => BossState::Cobra(CobraState::Death),
            BossKind::Lantern => BossState::Lantern(LanternState::Death),
        }
    }

    /// Constant salt mixed into the run seed for the boss rng stream.
    #[inline]
    pub fn seed_salt(self) -> u64 {
        match self {
            BossKind::Cat => 0x9E37_79B9_7F4A_7C15,
            BossKind::Broom => 0xC2B2_AE3D_27D4_EB4F,
            BossKind::Mower => 0x1656_67B1_9E37_79F9,
            BossKind::Mangle => 0xD1B5_4A32_7F6C_1E09,
            BossKind::NightEdition => 0x7B1D_3C4F_9A2E_5D68,
            BossKind::Magpie => 0x7C15_9E37_A1B2_C3D4,
            BossKind::HorseshoeCrab => 0x51ED_270B_9C4D_0A6F,
            BossKind::MegaRat => 0x3F2A_8D71_C6B4_05E9,
            BossKind::Hawk => 0x8D4C_2F91_B7A3_6E52,
            BossKind::Cobra => 0xA17C_5E93_0BD4_62E8,
            BossKind::Lantern => 0xB40E_61AD_17AF_0953,
        }
    }

    /// Culling margin for the art's largest sprite (world px).
    #[inline]
    pub fn cull_margin(self) -> f32 {
        match self {
            BossKind::Cat => art_boss_cat::BODY_LENGTH + art_boss_cat::TAIL_LENGTH,
            BossKind::Broom => art_boss_broom::CULL_MARGIN,
            BossKind::Mower => art_boss_mower::BODY_RADIUS + 20.0,
            BossKind::Mangle => art_boss_mangle::CULL_MARGIN,
            BossKind::NightEdition => art_boss_night_edition::CULL_MARGIN,
            BossKind::Magpie => art_boss_magpie::CULL_MARGIN,
            BossKind::HorseshoeCrab => art_boss_horseshoe_crab::CULL_MARGIN,
            BossKind::MegaRat => art_boss_megarat::CULL_MARGIN,
            BossKind::Hawk => art_boss_hawk::CULL_MARGIN,
            BossKind::Cobra => art_boss_cobra::CULL_MARGIN,
            BossKind::Lantern => art_boss_lantern::CULL_MARGIN,
        }
    }

    /// The distance (px) at which this kind's own attacks engage.
    ///
    /// Beyond it the boss is not fighting, it is following, and the shared
    /// pursuit rule takes over ([`OUT_OF_REACH_SECS`]). The numbers are the
    /// art's own reach constants, rounded out by a body radius: the Cat's
    /// pounce band ends at `art_boss_cat::POUNCE_MAX_DIST` (96), the Sweeper's
    /// dustpan at 56, the mower's scan cone is 72, the Cobra strikes at 92, the
    /// MegaRat shrieks at 132, the Redtail darts at 118, the Magpie's gust
    /// covers 190.
    pub fn engage_range(self) -> f32 {
        match self {
            // `POUNCE_MAX_DIST` 96, less the one px the band test allows.
            BossKind::Cat => 95.0,
            BossKind::Broom => 64.0,
            // `art_boss_mower::SCAN_CONE_RANGE`: past it the lidar cannot lock.
            BossKind::Mower => 72.0,
            BossKind::Mangle => 64.0,
            BossKind::NightEdition => 96.0,
            // A gust reaches 190 px, so the bird is *always* in range.
            BossKind::Magpie => 190.0,
            BossKind::HorseshoeCrab => 96.0,
            BossKind::MegaRat => 132.0,
            BossKind::Hawk => 118.0,
            BossKind::Cobra => 116.0,
            BossKind::Lantern => 76.0,
        }
    }

    /// Does this kind close a gap with an art move of its own instead of the
    /// shared pursuit leap?
    ///
    /// The Cat's `Crouch`/`Pounce` already is a leap and its controller drives
    /// it (`cat::start_attack`'s closing pounce), so the shared leap would be a
    /// second, art-less version of the same thing. Every other kind gets the
    /// shared one.
    #[inline]
    pub fn own_gap_closer(self) -> bool {
        matches!(self, BossKind::Cat)
    }

    /// The zero-sized controller for this kind.
    #[inline]
    pub fn controller(self) -> &'static dyn BossController {
        match self {
            BossKind::Cat => &cat::CatController,
            BossKind::Broom => &broom::BroomController,
            BossKind::Mower => &mower::MowerController,
            BossKind::Mangle => &mangle::MangleController,
            BossKind::NightEdition => &night_edition::NightEditionController,
            BossKind::Magpie => &magpie::MagpieController,
            BossKind::HorseshoeCrab => &horseshoe_crab::HorseshoeCrabController,
            BossKind::MegaRat => &megarat::MegaRatController,
            BossKind::Hawk => &hawk::HawkController,
            BossKind::Cobra => &cobra::CobraController,
            BossKind::Lantern => &lantern::LanternController,
        }
    }

    /// Every boss, in enum order. The console's `boss` command and its Tab
    /// completion read this table rather than a retyped list.
    pub fn all() -> [BossKind; 11] {
        [
            BossKind::Cat,
            BossKind::Broom,
            BossKind::Mower,
            BossKind::Mangle,
            BossKind::NightEdition,
            BossKind::Magpie,
            BossKind::HorseshoeCrab,
            BossKind::MegaRat,
            BossKind::Hawk,
            BossKind::Cobra,
            BossKind::Lantern,
        ]
    }

    /// Lowercase display name used by the console and diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            BossKind::Cat => "cat",
            BossKind::Broom => "broom",
            BossKind::Mower => "mower",
            BossKind::Mangle => "mangle",
            BossKind::NightEdition => "nightedition",
            BossKind::Magpie => "magpie",
            BossKind::HorseshoeCrab => "horseshoecrab",
            BossKind::MegaRat => "megarat",
            BossKind::Hawk => "hawk",
            BossKind::Cobra => "cobra",
            BossKind::Lantern => "lantern",
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
    Mangle(MangleState),
    NightEdition(NightEditionState),
    Magpie(MagpieState),
    /// Sewer pool mate: the horseshoe crab.
    HorseshoeCrab(CrabState),
    MegaRat(MegaRatState),
    /// The Flatbed's pool mate: the hawk (overhead, diving, hovering).
    Hawk(HawkState),
    /// The Lab's second fight: the King Cobra.
    Cobra(CobraState),
    Lantern(LanternState),
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

    /// Does any grounded shape sit on the **ring band** of `radius` around
    /// `center`? The band is `band` px wide either side (a Fairy Ring's drawn
    /// circle plus its slack), sampled around the circle so a large box is
    /// still caught by the samples inside it.
    #[inline]
    pub fn ring_hits_grounded(&self, center: Vec2, radius: f32, band: f32) -> bool {
        let steps = ((radius * 1.5).ceil() as u32).max(12);
        self.iter().any(|shape| {
            shape.grounded()
                && (0..steps).any(|i| {
                    let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                    let p = center + Vec2::from_angle(a) * radius.max(0.0);
                    shape.overlaps_circle(p, band)
                })
        })
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
    Mangle(MangleView),
    NightEdition(NightEditionView),
    Magpie(MagpieView),
    HorseshoeCrab(CrabView),
    MegaRat(MegaRatView),
    Hawk(HawkView),
    Cobra(CobraView),
    Lantern(LanternView),
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
                cos >= half_angle.dcos()
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
    /// A sustained conveyor push in px/s applied to the player every active
    /// tick it stands inside (the Mangle's intake pull). Unlike `knockback` it
    /// is not radial from the shape's centre: it is a fixed world direction, so
    /// a lane can feed toward the machine. Only used by damage-free hazards.
    pub drag: Vec2,
    /// Share of the rat's *own* movement this hazard takes while it stands
    /// inside (`0.0` = none, `0.45` = wading at 55 % speed). Applied by undoing
    /// that fraction of the tick's step, so it costs no i-frames and no new
    /// player state: the blue blood pools use it. Damage-free by construction.
    pub slow: f32,
    /// The rat's overlap padding. The Lantern's mouth stamp tests its centre.
    pub(super) player_radius: f32,
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
            drag: Vec2::ZERO,
            slow: 0.0,
            player_radius: PLAYER_RADIUS,
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

    /// Add a sustained conveyor drag (px/s, world direction).
    #[inline]
    pub fn with_drag(mut self, drag: Vec2) -> Self {
        self.drag = drag;
        self
    }

    /// Take this share of the rat's own movement while it stands inside.
    #[inline]
    pub fn with_slow(mut self, slow: f32) -> Self {
        self.slow = slow.clamp(0.0, 0.95);
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
    /// The Mangle's rolled towel (a 56 px rail).
    RolledTowel,
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
            BossProjectileKind::RolledTowel => art_boss_mangle::TOWEL_RADIUS,
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
            BossProjectileKind::RolledTowel => art_boss_mangle::TOWEL_DAMAGE,
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

// ---- the control budget (see the module docs) ------------------------------

/// Seconds of stagger a boss may be given inside one [`CONTROL_WINDOW_SECS`]
/// window. The budget is a hard cap, not a rate: a controller that asks for
/// more gets the remainder (and nothing once the window is spent).
///
/// It is one fixed window, anchored at the first grant, not a sliding one: a
/// sliding window would let a fast weapon pay itself out of the next 4 s and
/// still read as "under budget" on the tick it locked the boss out of a tell.
pub const CONTROL_BUDGET_SECS: f32 = 0.30;
/// Length of that window (s).
///
/// The brief: "at most 0.3 s of control per 4 s". 0.3 / 4 is 7.5 %, i.e. a
/// boss keeps 92.5 % of its fight against any build in the game. The old cat
/// `Hurt` beat was 0.45 s (0.90 s on a trap-sized burst) requested every
/// 0.33 s by a max-rank Tail Thrash, i.e. 100 %.
pub const CONTROL_WINDOW_SECS: f32 = 4.0;
/// A grant shorter than this is refused outright: a 0.02 s `Hurt` pose is a
/// frame of noise, not a readable stagger, and it would still burn the window.
pub const CONTROL_MIN_SECS: f32 = 0.12;

/// Most px a single weapon hit may shove a boss.
pub const SHOVE_PER_HIT_PX: f32 = 4.0;
/// Most px a boss may be shoved inside one [`SHOVE_WINDOW_SECS`] window.
pub const SHOVE_PER_SEC_PX: f32 = 12.0;
/// Length of the shove window (s).
pub const SHOVE_WINDOW_SECS: f32 = 1.0;

// ---- the pursuit rule (see the module docs) -------------------------------

/// Seconds of its own neutral state a boss may spend with the rat beyond
/// [`BossKind::engage_range`] before it stops following and starts closing.
pub const OUT_OF_REACH_SECS: f32 = 3.0;
/// The sprint a pursuing boss moves at (px/s).
///
/// Faster than the fastest rat a build can buy (`Stats::move_speed` 58 px/s,
/// 6 MoveSpeed stacks = 114 px/s), so the sprint always closes, and slower
/// than the leap, so a boss that can see its target still prefers to jump it.
pub const PURSUIT_SPEED: f32 = 132.0;
/// The leap a pursuing boss uses when the straight line to the rat is
/// body-passable (px/s).
pub const LEAP_SPEED: f32 = 300.0;
/// How long the leap lasts (s): `LEAP_SPEED * LEAP_SECS` is 90 px, which is
/// the longest gap any weapon out-reaches the cat's pounce band by.
pub const LEAP_SECS: f32 = 0.30;
/// Seconds between leaps.
pub const LEAP_COOLDOWN: f32 = 3.0;

/// Seconds between two recentrings of the fight rectangle (see
/// `boss_update_system`): one is plenty, since the rectangle is 34 x 28 cells.
pub const RECENTRE_COOLDOWN: f32 = 6.0;

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
    /// Pending call: how many ordinary bodies a boss has asked for (`0` = no
    /// call). Consumed by [`megarat::boss_summon_system`] at the head of the
    /// next tick, which is the only place a boss may spawn an `Enemy` — the
    /// controller cannot (`BossController::tick` has no `Commands`) and the
    /// update slot is inside the `EnemyIndex` window. A boss that never calls
    /// leaves it at zero and pays one branch a tick.
    pub call_count: u8,
    /// Where the call answers from (the boss's position on the beat).
    pub call_at: Vec2,
    /// What kind the call sends.
    pub call_kind: EnemyKind,
    /// Seconds the boss has been too far away to fight (see
    /// [`BossKind::REJOIN_DIST`] and [`RELOCATE_AFTER_SECS`]): the framework
    /// walks an off-screen, stranded boss back into the fight.
    pub lost: f32,
    /// Seconds of stagger spent in the current control window, and the window's
    /// remaining length (0 = the next [`Boss::grant_control`] opens a fresh
    /// one). See [`CONTROL_BUDGET_SECS`].
    pub control_used: f32,
    pub control_window: f32,
    /// Px shoved in the current shove window, and its remaining length. See
    /// [`SHOVE_PER_SEC_PX`].
    pub shove_used: f32,
    pub shove_window: f32,
    /// Seconds of its own neutral state the boss has spent with the rat beyond
    /// [`BossKind::engage_range`] (the pursuit trigger).
    pub out_of_reach: f32,
    /// Seconds of leap left, and the cooldown before the next one can start.
    pub leap_t: f32,
    pub leap_cd: f32,
    /// Seconds before the fight rectangle may be recentred again.
    pub recentre_cd: f32,
    pub hazards: Vec<BossHazard>,
    lantern: lantern::LanternRuntime,
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
            contact_damage: if kind == BossKind::Lantern { 0.0 } else { kind.contact_damage() },
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
            call_count: 0,
            call_at: pos,
            call_kind: EnemyKind::Skitterer,
            lost: 0.0,
            control_used: 0.0,
            control_window: 0.0,
            shove_used: 0.0,
            shove_window: 0.0,
            out_of_reach: 0.0,
            leap_t: 0.0,
            leap_cd: 0.0,
            recentre_cd: RECENTRE_COOLDOWN,
            hazards: Vec::with_capacity(16),
            lantern: lantern::LanternRuntime::default(),
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

    /// Is this state the kind's own "the rat staggered it" beat?
    ///
    /// Only the Cat ([`BossCatState::Hurt`]) and the Sweeper
    /// ([`BroomBossState::Hurt`]) have one: a short, art-drawn pose the boss
    /// owes when a single burst of damage lands. It is *not* committed — it is
    /// already control — so it cannot be re-entered or extended, and it is
    /// never a beat a player weapon may cancel again.
    #[inline]
    pub fn is_stagger_state(&self) -> bool {
        matches!(
            self.state,
            BossState::Cat(BossCatState::Hurt) | BossState::Broom(BroomBossState::Hurt { .. })
        )
    }

    /// Neutral beats share pursuit, rejoining and the control budget.
    pub fn is_neutral(&self) -> bool {
        match self.state {
            BossState::Lantern(state) => state.is_neutral(),
            _ => self.state == self.kind.initial_state(),
        }
    }

    /// Is the boss inside a committed beat — a tell, a wind-up, the strike or
    /// the recovery after it?
    ///
    /// Every state except the kind's neutral one, its death beat and its own
    /// stagger pose. The framework reads this before it lets a player weapon
    /// cancel, restart or displace anything (see the module docs).
    #[inline]
    pub fn committed(&self) -> bool {
        !self.dead
            && !self.is_stagger_state()
            && !self.is_neutral()
            && self.state != self.kind.death_state()
    }

    /// Ask for `secs` of control (a stagger) from a player weapon. Returns the
    /// seconds granted: `secs` itself when the budget can pay all of it, the
    /// remainder when it can pay some ([`CONTROL_MIN_SECS`] or more), and
    /// **0.0** when the boss is committed, dead, or out of budget.
    ///
    /// The controller is expected to enter its stagger pose with the *granted*
    /// duration, so a boss under sustained fire still gets its beat instead of
    /// a pose nobody can read. See [`CONTROL_BUDGET_SECS`].
    pub fn grant_control(&mut self, secs: f32) -> f32 {
        // A stagger pose already playing is not re-tripped: it is control the
        // boss is *already* under, and re-entering it would extend the window's
        // spend by the back door.
        if secs <= 0.0 || self.dead || self.is_stagger_state() || self.committed() {
            return 0.0;
        }
        let left = if self.control_window > 0.0 {
            (CONTROL_BUDGET_SECS - self.control_used).max(0.0)
        } else {
            CONTROL_BUDGET_SECS
        };
        let granted = secs.min(left);
        if granted < CONTROL_MIN_SECS {
            return 0.0;
        }
        if self.control_window <= 0.0 {
            self.control_window = CONTROL_WINDOW_SECS;
            self.control_used = 0.0;
        }
        self.control_used += granted;
        granted
    }

    /// Shove the boss up to `want_px` along `dir`, through the large-body
    /// collision test. Returns the px actually moved.
    ///
    /// A player weapon may not displace a boss in a committed beat, at most
    /// [`SHOVE_PER_HIT_PX`] a hit and [`SHOVE_PER_SEC_PX`] in any
    /// [`SHOVE_WINDOW_SECS`] window. A shove into a wall is spent, not banked.
    pub fn shove(&mut self, level: &Level, dir: Vec2, want_px: f32) -> f32 {
        if want_px <= 0.0 || self.dead || self.committed() {
            return 0.0;
        }
        let left = if self.shove_window > 0.0 {
            (SHOVE_PER_SEC_PX - self.shove_used).max(0.0)
        } else {
            SHOVE_PER_SEC_PX
        };
        let granted = want_px.min(SHOVE_PER_HIT_PX).min(left);
        if granted <= 0.0 {
            return 0.0;
        }
        if self.shove_window <= 0.0 {
            self.shove_window = SHOVE_WINDOW_SECS;
            self.shove_used = 0.0;
        }
        self.shove_used += granted;
        let step = dir.normalized() * granted;
        if step.len_sq() <= 0.0 {
            return 0.0;
        }
        let radius = self.kind.body_radius();
        let next = self.pos + step;
        let fits = if self.kind == BossKind::Lantern {
            lantern::head_path_fits(level, self.pos, next)
        } else {
            level.tiles.circle_clear(next, radius, SizeClass::Large)
        };
        if fits {
            self.pos += step;
            granted
        } else {
            0.0
        }
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
            // The hawk is *out of reach* while it is high: it bumps nothing and
            // the body is drawn in the overhead pass (over the rat, as a bird
            // over a deck should be). The art owns the rule.
            BossState::Hawk(_) => art_boss_hawk::out_of_reach(&self.hawk_view()),
            _ => false,
        }
    }

    /// Is the hawk only a shadow right now (out of reach for every weapon)?
    ///
    /// Always false for the other kinds. The HUD reads it for the boss bar's
    /// header (`render::overlay`) and `boss_plate` swaps the classification tag
    /// for [`art_boss_hawk::OVERHEAD_TAG`], so the rat is told *why* its
    /// weapons are finding nothing.
    #[inline]
    pub fn overhead(&self) -> bool {
        match self.state {
            BossState::Hawk(_) => art_boss_hawk::out_of_reach(&self.hawk_view()),
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
            BossKind::Mangle => {
                // The frame box + both rollers, and the loose crank at the
                // right cheek (the ×1.5 weak point). The final jam lifts the
                // rollers off the floor's ground weapons, so the crank is the
                // only thing left to hit — exactly the design's "strike the
                // dropped crank".
                let view = self.mangle_view();
                let shapes = art_boss_mangle::hit_shapes(&view);
                let out: [BossHitShape; 2] = [
                    BossHitShape::Box {
                        center: shapes[0].center,
                        facing: shapes[0].facing,
                        half: shapes[0].half,
                        half_perp: shapes[0].half_perp,
                        grounded: shapes[0].grounded,
                    },
                    BossHitShape::Circle {
                        center: shapes[1].center,
                        radius: shapes[1].radius,
                        grounded: shapes[1].grounded,
                    },
                ];
                BossHitShapes::from_slice(&out)
            }
            BossKind::MegaRat => {
                // Tail root, haunch and head, all grounded (it never leaves the
                // floor). In the two recovery windows the exposed weak point
                // **replaces** the tail root: a boss carries three hit shapes,
                // so the belly / tail base *is* the third one — which also
                // makes it the first auto-aim target point, so the rat's own
                // weapons reach for the window the moment it opens.
                let view = self.megarat_view();
                let circles = art_boss_megarat::body_hit_circles(&view);
                let mut shapes: [BossHitShape; 3] = [
                    BossHitShape::Circle {
                        center: circles[0].0,
                        radius: circles[0].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[1].0,
                        radius: circles[1].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[2].0,
                        radius: circles[2].1,
                        grounded: true,
                    },
                ];
                if let Some((at, radius)) = art_boss_megarat::weak_point(&view) {
                    shapes[0] = BossHitShape::Circle {
                        center: at,
                        radius,
                        grounded: true,
                    };
                }
                BossHitShapes::from_slice(&shapes)
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
            BossKind::Magpie => {
                // The bird flies *low* (belly to the planks, claws trailing), so
                // all three circles stay grounded even through a strafing pass:
                // a ground-only build must always be able to fight it. While the
                // bill is buried the head circle is parked at the strike point,
                // which is what makes the punish window reachable.
                let view = self.magpie_view();
                let circles = art_boss_magpie::body_hit_circles(&view);
                let shapes: [BossHitShape; 3] = [
                    BossHitShape::Circle {
                        center: circles[0].0,
                        radius: circles[0].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[1].0,
                        radius: circles[1].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[2].0,
                        radius: circles[2].1,
                        grounded: true,
                    },
                ];
                BossHitShapes::from_slice(&shapes)
            }
            BossKind::HorseshoeCrab => {
                // The dome is the body; the gill fans are the one clear weak
                // point and they only exist while the shell is off (wedged,
                // flipped). Under the floor it has no grounded shape at all.
                let view = self.crab_view();
                let shapes = art_boss_horseshoe_crab::hit_shapes(&view);
                let out: [BossHitShape; 2] = [
                    BossHitShape::Box {
                        center: shapes[0].center,
                        facing: shapes[0].facing,
                        half: shapes[0].half,
                        half_perp: shapes[0].half_perp,
                        grounded: shapes[0].grounded,
                    },
                    BossHitShape::Circle {
                        center: shapes[1].center,
                        radius: shapes[1].radius,
                        grounded: shapes[1].grounded,
                    },
                ];
                BossHitShapes::from_slice(&out)
            }
            BossKind::Hawk => {
                // **While it is high it has no hit shapes at all**: the shadow
                // is the boss on the screen and the bird is out of every
                // weapon's reach (the plate says so — see `Boss::overhead`).
                // Down on the deck all three circles are grounded, and while the
                // talons are buried the *nape* replaces the tail root, which is
                // what makes it the first auto-aim point of the punish window.
                let view = self.hawk_view();
                if art_boss_hawk::out_of_reach(&view) {
                    return BossHitShapes::empty();
                }
                let circles = art_boss_hawk::body_hit_circles(&view);
                let shapes: [BossHitShape; 3] = [
                    BossHitShape::Circle {
                        center: circles[0].0,
                        radius: circles[0].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[1].0,
                        radius: circles[1].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[2].0,
                        radius: circles[2].1,
                        grounded: true,
                    },
                ];
                BossHitShapes::from_slice(&shapes)
            }
            BossKind::Cobra => {
                // The head, the neck behind it and the hood. While the fangs
                // are in the floor (or the laid spiral has dropped) the art's
                // weak point **replaces** the head, so the open hood's back is
                // the first thing auto-aim reaches for — and the first circle
                // is the spectacle mark itself.
                let view = self.cobra_view();
                let circles = art_boss_cobra::body_hit_circles(&view);
                let shapes: [BossHitShape; 3] = [
                    BossHitShape::Circle {
                        center: circles[0].0,
                        radius: circles[0].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[1].0,
                        radius: circles[1].1,
                        grounded: true,
                    },
                    BossHitShape::Circle {
                        center: circles[2].0,
                        radius: circles[2].1,
                        grounded: true,
                    },
                ];
                BossHitShapes::from_slice(&shapes)
            }
            BossKind::Lantern => {
                if self.dead {
                    return BossHitShapes::empty();
                }
                BossHitShapes::from_slice(&[BossHitShape::Circle {
                    center: self.pos,
                    radius: art_boss_lantern::HEAD_RADIUS,
                    grounded: true,
                }])
            }
        }
    }

    /// The damage multiplier for a hit arriving at `at` along `dir` (unit
    /// travel direction; `Vec2::ZERO` = from underneath, e.g. a pool).
    ///
    /// The Horseshoe's shell deflects shots from the front or top and its
    /// exposed gills take [`art_boss_horseshoe_crab::GILL_DAMAGE_MULT`].
    /// The Lantern takes half from the front while its jaw recovers.
    #[inline]
    pub fn incoming_multiplier(&self, dir: Vec2, at: Vec2) -> f32 {
        match self.kind {
            BossKind::HorseshoeCrab => {
                art_boss_horseshoe_crab::incoming_multiplier(&self.crab_view(), dir, at)
            }
            BossKind::Lantern => lantern::feint_armor(self, -dir),
            _ => 1.0,
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
            BossKind::Mangle => BossArtView::Mangle(self.mangle_view()),
            BossKind::NightEdition => BossArtView::NightEdition(self.night_edition_view()),
            BossKind::Magpie => BossArtView::Magpie(self.magpie_view()),
            BossKind::HorseshoeCrab => BossArtView::HorseshoeCrab(self.crab_view()),
            BossKind::MegaRat => BossArtView::MegaRat(self.megarat_view()),
            BossKind::Hawk => BossArtView::Hawk(self.hawk_view()),
            BossKind::Cobra => BossArtView::Cobra(self.cobra_view()),
            BossKind::Lantern => BossArtView::Lantern(
                self.lantern_view().expect("Lantern state for Lantern boss"),
            ),
        }
    }

    /// One snapshot for art, lighting and bot. Neither bulb is an aim point.
    pub fn lantern_view(&self) -> Option<LanternView> {
        match self.state {
            BossState::Lantern(state) => Some(lantern::view(self, state)),
            _ => None,
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

    fn mangle_state(&self) -> MangleState {
        match self.state {
            BossState::Mangle(s) => s,
            _ => MangleState::Idle,
        }
    }

    fn magpie_state(&self) -> MagpieState {
        match self.state {
            BossState::Magpie(s) => s,
            _ => MagpieState::Perch,
        }
    }

    fn crab_state(&self) -> CrabState {
        match self.state {
            BossState::HorseshoeCrab(s) => s,
            _ => CrabState::Dome,
        }
    }

    pub(crate) fn crab_view(&self) -> CrabView {
        CrabView {
            pos: self.pos,
            facing: self.facing,
            state: self.crab_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            phase: self.phase,
            target: self.target,
            lane_start: self.pounce_origin,
            sweep_side: self.swipe_side,
        }
    }

    fn cobra_state(&self) -> CobraState {
        match self.state {
            BossState::Cobra(s) => s,
            _ => CobraState::Rear,
        }
    }

    /// The King Cobra's view. The framework hands the art two things it cannot
    /// derive from the boss's own fields and must not guess at:
    ///
    /// * the loop's opening angle, from the controller's own scratch (the art
    ///   draws the gap where the animal is holding it open, and the separation
    ///   pass in [`cobra::coil_block_system`] walks the same chain);
    /// * the live venom puddles, read out of `self.hazards` — those are the
    ///   boss's own hazards, so a dead cobra draws no pools, and the art and the
    ///   damage can never disagree about where a puddle is.
    pub(crate) fn cobra_view(&self) -> CobraView {
        let mut pools = [Vec2::ZERO; art_boss_cobra::POOL_VIEW];
        let mut pool_r = [0.0f32; art_boss_cobra::POOL_VIEW];
        let mut pool_n = 0u8;
        for hazard in self.hazards.iter() {
            // A puddle is the damage-free slow circle; only one that has
            // already landed is drawn (the splash's own warn is the lob).
            if hazard.slow <= 0.0 || hazard.elapsed < hazard.warn {
                continue;
            }
            if (pool_n as usize) >= pools.len() {
                break;
            }
            if let HazardShape::Circle { center, radius } = hazard.shape {
                pools[pool_n as usize] = center;
                pool_r[pool_n as usize] = radius;
                pool_n += 1;
            }
        }
        CobraView {
            pos: self.pos,
            facing: self.facing,
            state: self.cobra_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            phase: self.phase,
            target: self.target,
            lane_start: self.pounce_origin,
            gap_angle: self.scratch[cobra::S_GAP],
            pools,
            pool_r,
            pool_n,
            hunting: self.hunting,
            arena: self.arena,
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

    fn magpie_view(&self) -> MagpieView {
        MagpieView {
            pos: self.pos,
            facing: self.facing,
            state: self.magpie_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            target: self.target,
            lane_start: self.pounce_origin,
            sweep: self.scratch[magpie::S_LANE_DIR],
            hunting: self.hunting,
        }
    }

    fn megarat_state(&self) -> MegaRatState {
        match self.state {
            BossState::MegaRat(s) => s,
            _ => MegaRatState::Groom,
        }
    }

    /// The Megarat's view. The framework hands it two of the controller's own
    /// scratch slots (the thrash's first-pass side and the dash cooldown) so the
    /// art can draw the arc's pass order and the read-the-gauge pip without the
    /// art knowing anything else about the fight.
    pub(crate) fn megarat_view(&self) -> MegaRatView {
        MegaRatView {
            pos: self.pos,
            facing: self.facing,
            state: self.megarat_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            target: self.target,
            lane_start: self.pounce_origin,
            sweep: self.scratch[megarat::S_SIDE],
            phase: self.phase,
            hunting: self.hunting,
            dash_cd: (self.scratch[megarat::S_DASH_CD] / art_boss_megarat::DASH_COOLDOWN)
                .clamp(0.0, 1.0),
            walled: self.scratch[megarat::S_WALLED] > 0.5,
        }
    }

    fn hawk_state(&self) -> HawkState {
        match self.state {
            BossState::Hawk(s) => s,
            _ => HawkState::Circle,
        }
    }

    /// The Redtail's view. The framework hands it one of the controller's own
    /// scratch slots (the feint's decoy point) so the art can draw the hollow
    /// print the shadow jinked to without knowing anything else about the fight.
    pub(crate) fn hawk_view(&self) -> HawkView {
        HawkView {
            pos: self.pos,
            facing: self.facing,
            state: self.hawk_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            phase: self.phase,
            target: self.target,
            lane_start: self.pounce_origin,
            decoy: Vec2::new(self.scratch[hawk::S_DECOY_X], self.scratch[hawk::S_DECOY_Y]),
            sweep: self.swipe_side,
            hunting: self.hunting,
        }
    }

    fn night_edition_state(&self) -> NightEditionState {
        match self.state {
            BossState::NightEdition(s) => s,
            _ => NightEditionState::Roll,
        }
    }

    pub(crate) fn night_edition_view(&self) -> NightEditionView {
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

    pub(crate) fn mangle_view(&self) -> MangleView {
        MangleView {
            pos: self.pos,
            facing: self.facing,
            state: self.mangle_state(),
            progress: self.progress(),
            anim: self.anim,
            hit_flash: self.hit_flash,
            hp_frac: self.hp_frac(),
            target: self.target,
            phase: self.phase,
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

    // ---- The Mangle (Laundry) ----
    /// `Intake` entry: the rollers turn backwards and the lane outlines.
    MangleRollers,
    /// The single 16-damage bite crosses the nip.
    MangleBite,
    /// A ratchet click of the wring-out's crank (two under the 0.9 s tell).
    MangleCrank,
    /// The rolled towel leaves the nip.
    MangleTowel,
    /// `Jam` / `FinalJam` entry: the nip stuffs and the crank drops.
    MangleJam,
    /// `PhaseShift` entry: the frame shudders into its new pace.
    MangleShift,
    /// The springs pop in the death beat.
    MangleDeath,
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
    // ---- The Magpie (Flatbed) ----
    /// `Perch` entry: the bird's corvid rattle as it settles on the cargo.
    MagpieCry,
    /// `Ruffle` entry (phase change): the feathers going up.
    MagpieRuffle,
    /// `PeckWindup` entry: the head comes back over the line.
    MagpieWindup,
    /// `Stuck` entry: the bill goes into the plank.
    MagpieStuck,
    /// `Takeoff` entry: two hard wing-beats off the deck.
    MagpieTakeoff,
    /// `GustWindup` entry: the wings drawing back and the wind starting.
    MagpieGustGather,
    /// One wing-beat gust pulse.
    MagpieGust,
    /// The dropped crate bursting on the deck.
    MagpieCrate,
    /// The `Death` beat: the last rattle.
    MagpieDeath,
    // ---- The Horseshoe (Sewer pool mate) ----
    /// `Pose` / `Righting` entry: the plates shifting over the wet shell.
    CrabRattle,
    /// `TelsonTell` entry: the spike cocking over one shoulder.
    CrabTelsonCock,
    /// `TelsonSweep` entry: the spike coming through.
    CrabTelsonSweep,
    /// `BulldozeTell` / `Bulldoze` entry: the low, grinding rev.
    CrabCharge,
    /// `Wedged` entry: the dome into the masonry.
    CrabWedge,
    /// `BurrowTell` / `Burrow` entry: digging through the drain bed.
    CrabBurrow,
    /// `Emerge` entry: the eruption.
    CrabEmerge,
    /// `FlipTell` / `Flipped` entry: the shell coming over.
    CrabFlip,
    /// The `Death` beat.
    CrabDeath,
    // ---- SUBJECT R-01 (Kitchen's second fight) ----
    /// `Groom` entry: the self-satisfied chitter of a rat that has won before.
    MegaRatChitter,
    /// `GnawWindup` entry: the incisors coming up, a wet grinding scrape.
    MegaRatGnaw,
    /// `GnawStuck` entry: the teeth going into the cabinet and the indignant
    /// squeal that follows.
    MegaRatStuck,
    /// `ThrashWindup` entry: the tail cocked back.
    MegaRatThrash,
    /// One pass of the tail across its half of the arc.
    MegaRatThrashSweep,
    /// `SqueakShout` entry: the boss-scale sonic squeak.
    MegaRatSqueak,
    /// `DashPass` entry: the scrabble-and-screech of the charge.
    MegaRatDash,
    /// `Call` (and the rats answering it at the summon beat).
    MegaRatCall,
    /// The `Death` beat: the first success going down.
    MegaRatDeath,
    // ---- THE REDTAIL (the Flatbed's second bird) ----
    /// `Circle` entry: the keening "ca-caww" of a raptor taking up the sky.
    HawkCall,
    /// The dive's lock: the reticle is in and the scream rises.
    HawkLock,
    /// The feint's jink: a cut-off cry as the shadow throws sideways.
    HawkFeint,
    /// The impact: a single frame of talons into the planks.
    HawkImpact,
    /// `TalonBury` entry: splinters, and a bird that cannot pull its feet out.
    HawkStuck,
    /// `Wheel` / `Climb` entry: two hard wing-beats off the deck.
    HawkWings,
    /// `Stoop` entry: the air coming down with it.
    HawkStoop,
    /// `Hover` entry: the low wash of a raptor beating its wings.
    HawkHover,
    /// One downdraft ring (a wing-beat that pushes the deck).
    HawkDowndraft,
    /// `DartWindup` entry: the head thrown forward over the mark.
    HawkDartWindup,
    /// The feather darts crossing the deck.
    HawkDarts,
    /// `GrabWindup` entry: the feet come down.
    HawkGrabWindup,
    /// The talons closing on the mark.
    HawkGrab,
    /// `RakeWindup` entry: it banks over the lane.
    HawkRakeWindup,
    /// The rake itself: talons ripping down the planks.
    HawkRake,
    /// `Alight` entry (phase change): the hackles going up and resettling.
    HawkRuffle,
    /// The `Death` beat: it comes down out of the sky for the last time.
    HawkDeath,
    // ---- KING COBRA (the Lab's second fight) ----
    /// `Rear` entry: the tongue out, tasting for the rat.
    CobraTaste,
    /// `Rise` entry: the full height and the hood all the way open.
    CobraRise,
    /// `HoodTell` / `ReStrikeTell` entry: the hood's dry rattle.
    CobraHood,
    /// `Strike` entry: the head coming down the line.
    CobraStrike,
    /// `FangsStuck` / `SpiralEnd` entry: the fangs into the floor.
    CobraStuck,
    /// `SwayTell` entry: the first beat of the rhythm.
    CobraSway,
    /// `VenomTell` entry: the throat working, a wet rising hiss.
    CobraVenomTell,
    /// `VenomSpit`: the glob and the three landings.
    CobraSpit,
    /// `TailTell` entry: a coil's worth of tail dragging round.
    CobraTailTell,
    /// `TailSweep`: the whip across the floor.
    CobraTailSweep,
    /// `ConstrictTell` / `SpiralCoil` entry: the body slamming down as a wall.
    CobraCoil,
    /// `Constrict` entry: the loop taking up the slack.
    CobraTighten,
    /// `Loosen` entry: the loop letting go.
    CobraLoosen,
    /// `SpiralChase` entry: the head coming down its own lane.
    CobraSpiral,
    /// The `Death` beat: the body unspools.
    CobraDeath,
    LanternTell,
    LanternStrike,
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
            BossCue::MangleRollers => Sfx::MangleRollers,
            BossCue::MangleBite => Sfx::MangleBite,
            BossCue::MangleCrank => Sfx::MangleCrank,
            BossCue::MangleTowel => Sfx::MangleTowel,
            BossCue::MangleJam => Sfx::MangleJam,
            BossCue::MangleShift => Sfx::MangleShift,
            BossCue::MangleDeath => Sfx::MangleDeath,
            // Each of the press's three beats has its own short variation: the
            // sheet feeding in, the platen's clack, the bed coming down.
            BossCue::NightPage => Sfx::NightPage,
            BossCue::NightStamp => Sfx::NightStamp,
            BossCue::NightBed => Sfx::NightBed,
            BossCue::NightMisprint => Sfx::NightBell,
            BossCue::NightBell => Sfx::NightBell,
            BossCue::NightCarriage => Sfx::NightCarriage,
            // The ribbon window is a mechanical reveal, exactly the chest-open
            // sting's shape.
            BossCue::NightRibbon => Sfx::CacheOpen,
            // Re-seating machinery and the wreck are the press's own sounds
            // (they were the mower's dock hum and death, i.e. another machine).
            BossCue::NightTurn => Sfx::NightTurn,
            BossCue::NightWreck => Sfx::NightWreck,
            BossCue::MagpieCry => Sfx::MagpieCry,
            BossCue::MagpieRuffle => Sfx::MagpieRattle,
            BossCue::MagpieWindup => Sfx::MagpieWindup,
            BossCue::MagpieStuck => Sfx::MagpieStuck,
            BossCue::MagpieTakeoff => Sfx::MagpieTakeoff,
            // The gather is a breath in, the gust is the wing-beat out: two
            // effects, not one.
            BossCue::MagpieGustGather => Sfx::MagpieGustGather,
            BossCue::MagpieGust => Sfx::MagpieGust,
            BossCue::MagpieCrate => Sfx::MagpieCrate,
            BossCue::MagpieDeath => Sfx::MagpieDeath,
            BossCue::CrabRattle => Sfx::CrabRattle,
            BossCue::CrabTelsonCock => Sfx::CrabTelsonCock,
            BossCue::CrabTelsonSweep => Sfx::CrabTelsonSweep,
            BossCue::CrabCharge => Sfx::CrabCharge,
            BossCue::CrabWedge => Sfx::CrabWedge,
            BossCue::CrabBurrow => Sfx::CrabBurrow,
            BossCue::CrabEmerge => Sfx::CrabEmerge,
            BossCue::CrabFlip => Sfx::CrabFlip,
            BossCue::CrabDeath => Sfx::CrabDeath,
            BossCue::MegaRatChitter => Sfx::MegaRatChitter,
            BossCue::MegaRatGnaw => Sfx::MegaRatGnaw,
            BossCue::MegaRatStuck => Sfx::MegaRatStuck,
            BossCue::MegaRatThrash => Sfx::MegaRatThrash,
            // The sweep is the rasp of the tail over the floor: the same voice
            // as the cocked-back tell, pitched by its own sample.
            BossCue::MegaRatThrashSweep => Sfx::MegaRatThrashSweep,
            BossCue::MegaRatSqueak => Sfx::MegaRatSqueak,
            BossCue::MegaRatDash => Sfx::MegaRatDash,
            BossCue::MegaRatCall => Sfx::MegaRatCall,
            BossCue::MegaRatDeath => Sfx::MegaRatDeath,
            // The hawk's own voice: the cry, the lock's scream, the slam, the
            // wind. The feint is a truncated cry (the same sample, its own
            // particle read) and the tells that share the bird's body borrow
            // its wing-beat rather than inventing a new one each.
            BossCue::HawkCall => Sfx::HawkCry,
            BossCue::HawkLock => Sfx::HawkLock,
            BossCue::HawkFeint => Sfx::HawkCry,
            BossCue::HawkImpact => Sfx::HawkImpact,
            BossCue::HawkStuck => Sfx::HawkStuck,
            BossCue::HawkWings => Sfx::HawkWings,
            BossCue::HawkStoop => Sfx::HawkWings,
            BossCue::HawkHover => Sfx::HawkHover,
            BossCue::HawkDowndraft => Sfx::HawkWings,
            BossCue::HawkDartWindup => Sfx::HawkRuffle,
            BossCue::HawkDarts => Sfx::HawkDarts,
            BossCue::HawkGrabWindup => Sfx::HawkRuffle,
            BossCue::HawkGrab => Sfx::HawkGrab,
            BossCue::HawkRakeWindup => Sfx::HawkCry,
            BossCue::HawkRake => Sfx::HawkRake,
            BossCue::HawkRuffle => Sfx::HawkRuffle,
            BossCue::HawkDeath => Sfx::HawkDeath,
            // The cobra: one voice for the body and one for each of its three
            // attacks (see `Sfx::Cobra*`), so the rat can hear which tell it is.
            BossCue::CobraTaste => Sfx::CobraTaste,
            BossCue::CobraRise | BossCue::CobraHood => Sfx::CobraHood,
            BossCue::CobraStrike => Sfx::CobraStrike,
            BossCue::CobraStuck => Sfx::CobraStuck,
            BossCue::CobraSway => Sfx::CobraSway,
            BossCue::CobraVenomTell => Sfx::CobraVenomTell,
            BossCue::CobraSpit => Sfx::CobraSpit,
            BossCue::CobraTailTell | BossCue::CobraTailSweep => Sfx::CobraTail,
            BossCue::CobraCoil | BossCue::CobraTighten => Sfx::CobraCoil,
            BossCue::CobraLoosen => Sfx::CobraLoosen,
            BossCue::CobraSpiral => Sfx::CobraSpiral,
            BossCue::CobraDeath => Sfx::CobraDeath,
            BossCue::LanternTell => Sfx::Splash,
            BossCue::LanternStrike => Sfx::SplashBig,
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
    let fits = |from, to| {
        if boss.kind == BossKind::Lantern {
            lantern::head_path_fits(ctx.level, from, to)
        } else {
            ctx.footprint_fits(to, radius)
        }
    };
    let next = boss.pos + step;
    if fits(boss.pos, next) {
        boss.pos = next;
        return true;
    }
    // Axis slide: try x then y so a body grazes a wall instead of sticking.
    let try_x = Vec2::new(step.x, 0.0);
    if try_x.len_sq() > 0.0 && fits(boss.pos, boss.pos + try_x) {
        boss.pos += try_x;
        return false;
    }
    let try_y = Vec2::new(0.0, step.y);
    if try_y.len_sq() > 0.0 && fits(boss.pos, boss.pos + try_y) {
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
        if hazard.hit_player && !hazard.fired && hazard.shape.contains(player.pos, hazard.player_radius) {
            if hazard.damage <= 0.0 {
                // Push-only hazard (the cat's damage-free outer roar ring and
                // the Mangle's intake lane): the knockback is a sustained shove
                // in px/s, applied every active tick the rat is inside the
                // shape, and `drag` is a fixed-direction conveyor on top of it.
                // Nothing is "hit", so no damage, no i-frames and no hurt vfx
                // are spent.
                if hazard.knockback > 0.0 || hazard.drag.len_sq() > 0.0 || hazard.slow > 0.0 {
                    let dir = (player.pos - hazard.shape.center()).normalized();
                    if hazard.knockback > 0.0 {
                        player.pos += dir * hazard.knockback * dt;
                    }
                    player.pos += hazard.drag * dt;
                    if hazard.slow > 0.0 {
                        // Undo the share of this tick's own step: walking in is
                        // slower, and the rat can always wade back out.
                        player.pos -= player.vel * dt * hazard.slow;
                    }
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

            // ---- mangle -------------------------------------------------
            BossCue::MangleRollers => {
                // The backwards whirr's visible half: a vent of steam off the
                // frame and a puff at the nip.
                let view = boss.mangle_view();
                vfx::emit_boss_mower_overheat_steam(
                    particles,
                    rng,
                    boss.pos,
                    boss.facing,
                    0.12,
                );
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    art_boss_mangle::nip_pos(&view),
                    Vec2::ZERO,
                    None,
                    EnemyKind::Boss,
                    false,
                );
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
            BossCue::MangleBite => {
                // The nip's metal clang: the mower's wall-impact sparks are
                // exactly a hard steel bite.
                let view = boss.mangle_view();
                let nip = art_boss_mangle::nip_pos(&view);
                vfx::emit_boss_mower_wall_impact(particles, rng, nip, -boss.facing, true);
            }
            BossCue::MangleCrank => {
                let view = boss.mangle_view();
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    art_boss_mangle::crank_pos(&view),
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
            BossCue::MangleTowel => {
                let view = boss.mangle_view();
                let nip = art_boss_mangle::nip_pos(&view);
                vfx::emit_boss_mower_charge_trail(particles, rng, nip, boss.facing, false, 0.12);
            }
            BossCue::MangleJam | BossCue::MangleShift => {
                vfx::emit_boss_mower_overheat_steam(
                    particles,
                    rng,
                    boss.pos,
                    boss.facing,
                    0.16,
                );
            }
            BossCue::MangleDeath => {
                vfx::emit_boss_mower_death(particles, rng, boss.pos);
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
            // ---- magpie ------------------------------------------------
            // The cry and the wind-up are both a feather-and-dust kick off the
            // deck under the bird (the art draws the bill and the streaks).
            BossCue::MagpieCry
            | BossCue::MagpieRuffle
            | BossCue::MagpieWindup
            | BossCue::MagpieTakeoff => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.14);
            }
            BossCue::MagpieStuck => {
                // Bill into the plank: a splinter burst at the strike point.
                vfx::emit_boss_magpie_peck(particles, rng, boss.target, boss.facing);
            }
            BossCue::MagpieGustGather => {
                vfx::emit_boss_magpie_gust(particles, rng, boss.pos, 0.10);
            }
            BossCue::MagpieGust => {
                vfx::emit_boss_magpie_gust(particles, rng, boss.pos, 0.16);
            }
            BossCue::MagpieCrate => {
                vfx::emit_boss_magpie_crate(particles, rng, boss.target);
            }
            BossCue::MagpieDeath => {
                let blood = level_blood();
                vfx::emit_boss_magpie_death(particles, rng, boss.pos, boss.facing, blood);
            }

            // ---- the horseshoe crab -------------------------------------
            // Its feedback is all shell and wet floor: the rattle and the
            // charge are a dust kick off the concrete, the tail is the mower's
            // steel clang family, and the death is the crab's own blue.
            BossCue::CrabRattle => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.12);
            }
            BossCue::CrabTelsonCock => {
                vfx::emit_boss_mower_overheat_steam(particles, rng, boss.pos, boss.facing, 0.10);
            }
            BossCue::CrabTelsonSweep => {
                let view = boss.crab_view();
                let tip = art_boss_horseshoe_crab::telson_tip(&view);
                vfx::emit_boss_mower_wall_impact(particles, rng, tip, boss.facing, false);
            }
            BossCue::CrabCharge => {
                vfx::emit_boss_mower_charge_trail(particles, rng, boss.pos, boss.facing, false, 0.10);
            }
            BossCue::CrabWedge => {
                let view = boss.crab_view();
                let gill = art_boss_horseshoe_crab::gill_pos(&view);
                vfx::emit_boss_mower_wall_impact(particles, rng, gill, -boss.facing, true);
            }
            BossCue::CrabBurrow => {
                let at = boss.target;
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    at,
                    art_boss_horseshoe_crab::BURROW_RADIUS * 0.6,
                    boss.phase >= 3,
                );
            }
            BossCue::CrabEmerge => {
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.pos,
                    art_boss_horseshoe_crab::BURROW_RADIUS,
                    true,
                );
            }
            BossCue::CrabFlip => {
                vfx::emit_boss_mower_flipped_smoke(particles, rng, boss.pos, 0.12);
            }
            BossCue::CrabDeath => {
                let blood = (44, 92, 190);
                vfx::emit_boss_magpie_death(particles, rng, boss.pos, boss.facing, blood);
            }
            // ---- SUBJECT R-01 -------------------------------------------
            // The rat's own feedback, at boss scale: paw dust for the chitter
            // and the incisors coming up, the human's slam for the teeth going
            // into the cabinet, the player's own Tail Thrash for the passes,
            // the player's own Sonic Squeak for the shout, the player's own
            // dash for the charge. The mirror is the point: the rat has seen
            // every one of these effects come off its own body.
            BossCue::MegaRatChitter | BossCue::MegaRatGnaw => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.12);
            }
            BossCue::MegaRatStuck => {
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.target,
                    art_boss_megarat::GNAW_ACTIVE_HALF_WIDTH + 4.0,
                    boss.phase >= 2,
                );
            }
            BossCue::MegaRatThrash => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.16);
            }
            BossCue::MegaRatThrashSweep => {
                let view = boss.megarat_view();
                let side = if view.sweep < 0.0 { -1.0 } else { 1.0 };
                vfx::emit_thrash_sweep(
                    particles,
                    rng,
                    boss.pos,
                    view.facing.angle() + side * art_boss_megarat::THRASH_HALF_ARC * 0.5,
                    art_boss_megarat::THRASH_OUTER,
                    true,
                );
            }
            BossCue::MegaRatSqueak => {
                vfx::emit_squeak_ring(
                    particles,
                    rng,
                    boss.pos,
                    art_boss_megarat::SQUEAK_OUTER,
                    art_boss_megarat::SQUEAK_ACTIVE,
                );
            }
            BossCue::MegaRatDash => {
                vfx::emit_dash_start(particles, rng, boss.pos, boss.facing);
            }
            BossCue::MegaRatCall => {
                vfx::emit_boss_spawn(particles, rng, boss.pos, level_blood());
            }
            BossCue::MegaRatDeath => {
                let blood = level_blood();
                vfx::emit_boss_cat_death(particles, rng, boss.pos, boss.facing, blood);
            }

            // ---- the King Cobra ------------------------------------------
            // Its feedback is all scale, dust and venom: the body sliding on
            // the tile for the taste and the rise, the mower's steel family for
            // the impact of its loops, the rat's own tail thrash for the whip,
            // and a ring of grit where the loop slams down.
            BossCue::CobraTaste | BossCue::CobraRise | BossCue::CobraHood => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.10);
            }
            BossCue::CobraStrike => {
                vfx::emit_boss_mower_charge_trail(particles, rng, boss.pos, boss.facing, false, 0.14);
            }
            BossCue::CobraStuck | BossCue::CobraLoosen => {
                // The fangs going into the floor: the broom's slam, at the
                // strike's own width.
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.target,
                    art_boss_cobra::STRIKE_HALF_WIDTH + 4.0,
                    boss.phase >= 2,
                );
            }
            BossCue::CobraSway => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing.perp(), 0.12);
            }
            BossCue::CobraVenomTell => {
                // A wet glint at the fangs as the throat fills.
                let fangs = art_boss_cobra::fangs_pos(&boss.cobra_view());
                vfx::emit_weapon_hit(
                    particles,
                    rng,
                    fangs,
                    Vec2::ZERO,
                    None,
                    EnemyKind::Boss,
                    false,
                );
            }
            BossCue::CobraSpit => {
                let view = boss.cobra_view();
                for k in 0..art_boss_cobra::VENOM_MARKS {
                    let at = art_boss_cobra::venom_mark_centre(&view, k, None);
                    vfx::emit_weapon_hit(
                        particles,
                        rng,
                        at,
                        Vec2::ZERO,
                        None,
                        EnemyKind::Boss,
                        false,
                    );
                }
            }
            BossCue::CobraTailTell | BossCue::CobraTailSweep => {
                vfx::emit_thrash_sweep(
                    particles,
                    rng,
                    boss.pos,
                    boss.facing.angle(),
                    art_boss_cobra::TAIL_OUTER,
                    true,
                );
            }
            BossCue::CobraCoil | BossCue::CobraTighten => {
                // The loop arrives as a ring of grit, not a puff: the body is
                // the wall, so the floor has to show it landing.
                vfx::emit_squeak_ring(
                    particles,
                    rng,
                    boss.target,
                    art_boss_cobra::coil_radius(&boss.cobra_view()).max(12.0),
                    0.22,
                );
            }
            BossCue::CobraSpiral => {
                vfx::emit_boss_mower_charge_trail(particles, rng, boss.pos, boss.facing, false, 0.12);
            }
            BossCue::CobraDeath => {
                let blood = level_blood();
                vfx::emit_boss_cat_death(particles, rng, boss.pos, boss.facing, blood);
            }

            // ---- the redtail --------------------------------------------
            // The bird is *in the air*, so its own beats are read on the deck:
            // a call and a jink are a puff of dust off the boards under the
            // shadow, the lock is the wind gathering on the mark, the impact
            // and the buried feet are the deck's own dust and splinters, and
            // the wing beats are the flatbed's gust family.
            BossCue::HawkCall | BossCue::HawkFeint | BossCue::HawkRuffle => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.12);
            }
            BossCue::HawkLock => {
                vfx::emit_boss_mower_charge_trail(
                    particles,
                    rng,
                    boss.target,
                    boss.facing,
                    false,
                    0.12,
                );
            }
            BossCue::HawkImpact => {
                vfx::emit_boss_cat_pounce_land(
                    particles,
                    rng,
                    boss.target,
                    art_boss_hawk::DIVE_RADIUS,
                    boss.phase >= 2,
                );
            }
            BossCue::HawkStuck => {
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.target,
                    art_boss_hawk::DIVE_RADIUS,
                    boss.phase >= 2,
                );
            }
            BossCue::HawkWings | BossCue::HawkStoop | BossCue::HawkHover | BossCue::HawkDowndraft => {
                vfx::emit_boss_magpie_gust(particles, rng, boss.pos, 0.13);
            }
            BossCue::HawkDartWindup | BossCue::HawkGrabWindup | BossCue::HawkRakeWindup => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.10);
            }
            BossCue::HawkDarts => {
                vfx::emit_boss_mower_charge_trail(
                    particles,
                    rng,
                    boss.target,
                    boss.facing,
                    false,
                    0.10,
                );
            }
            BossCue::HawkGrab => {
                vfx::emit_boss_broom_stomp(
                    particles,
                    rng,
                    boss.target,
                    art_boss_hawk::GRAB_DROP_RADIUS,
                    false,
                );
            }
            BossCue::HawkRake => {
                vfx::emit_boss_cat_skid(particles, rng, boss.pos, boss.facing, 0.16);
            }
            BossCue::HawkDeath => {
                let blood = level_blood();
                vfx::emit_boss_magpie_death(particles, rng, boss.pos, boss.facing, blood);
            }
            BossCue::LanternTell | BossCue::LanternStrike => {}
        }
    }
}

/// The death splatter colour for a boss cue (the framework's caller passes the
/// palette's blood through `emit_cues`'s `blood` argument).
#[inline]
fn level_blood() -> Rgb {
    (150, 30, 40)
}

// ============================================================================
// Systems
// ============================================================================

/// Where and for whom a boss arrives: everything [`open_fight`] needs.
pub(crate) struct Arrival {
    pub kind: BossKind,
    pub pos: Vec2,
    pub arena: TileRect,
    pub dock: Vec2,
    /// The rat's position (the boss faces and targets it).
    pub rat: Vec2,
    pub difficulty: usize,
    /// The body's damage-number identity (`NextEnemyId`).
    pub id: u32,
    /// The level seed the fight's own rng stream is derived from.
    pub seed: u64,
}

/// The books every boss arrival keeps, in one place: the body with the
/// arrival's HP, its facing, and a fresh fight (own rng stream, no completion
/// flag, the horde parted). Shared by the scheduled spawn
/// ([`boss_spawn_system`]) and the console's `boss <kind> here`
/// (`World::force_boss_here`), so the two cannot drift apart. The caller picks
/// the place, emits the arrival burst from `fight.rng`, and spawns the body.
pub(crate) fn open_fight(fight: &mut BossFight, a: Arrival) -> Boss {
    let hunting = !a.arena.contains_world(a.rat);
    // The director's curve freezes at BOSS_TIME; the boss spawns then.
    let hp = a.kind.base_hp()
        * a.kind.hp_scale()
        * director::boss_hp_mult_for(a.difficulty, director::BOSS_TIME);
    let mut boss = Boss::new(a.kind, a.pos, hp, a.arena, a.dock, hunting);
    boss.id = a.id;
    boss.facing = (a.rat - a.pos).normalized();
    boss.target = a.rat;
    fight.rng = BossRng::for_run(a.seed, a.kind);
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
    boss
}

/// Spawn the level's boss when the director raises [`BossDue`].
///
/// **Batch 12 removed the boss arena.** The boss arrives where the rat is: it
/// is placed by the horde's own out-of-sight rule (outside the *widest* view
/// and reachable by the large size class, `spawn::spawn_point_reachable`) on
/// the boss's own rng stream, and the rectangle its controllers call the arena
/// is the floor's standard fight size recentred on the rat at that moment
/// ([`Level::fight_rect_around`]), with the dock at its centre — exactly what
/// the console's `boss <kind> here` ([`crate::game::world::World::force_boss_here`])
/// builds, so the two cannot drift apart.
///
/// `BossDue` is raised by `director::raise_boss_due`; P3 owns that call site.
#[allow(clippy::too_many_arguments)]
pub fn boss_spawn_system(
    mut commands: Commands,
    mut due: ResMut<BossDue>,
    mut fight: ResMut<BossFight>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut next_spawn_id: ResMut<NextSpawnId>,
    // Console `boss <kind>`: one-shot override of the floor's boss.
    mut override_kind: ResMut<BossOverride>,
    level: Res<Level>,
    difficulty: Res<DifficultyLevel>,
    player: Res<Player>,
    viewport: Res<Viewport>,
    bosses: Query<&Boss>,
    mut particles: ResMut<Particles>,
    mut shake: ResMut<Shake>,
    mut sfx: ResMut<SfxQueue>,
) {
    if !due.take() || !bosses.is_empty() {
        return;
    }
    // The theme is authoritative: the side-route floors play at a campaign
    // stage index, and the Laundry and the Printworks field their own bosses
    // (the Mangle, the Night Edition), so the theme is the only thing that can
    // name a floor's boss.
    // The floor's pool, chosen by the level seed: a floor with more than one
    // possible boss draws without spending rng (see [`BossKind::pool_pick`]).
    // The console's `boss <kind>` override takes priority for one spawn, then
    // clears.
    let kind = override_kind
        .0
        .take()
        .unwrap_or_else(|| BossKind::pool_pick(level.theme, level.seed));
    // The fight happens where the rat is, and it is a rectangle rather than a
    // room: the nominal fight size, clamped to the map and centred on the rat.
    let arena = level.fight_rect_around(player.pos);
    // The dock (the mower's charging pad) sits at the rectangle's centre, on
    // the nearest cell a large body fits in.
    let dock = level
        .tiles
        .nearest_open_center(arena.center_world(), SizeClass::Large, 16)
        .unwrap_or_else(|| arena.center_world());
    // Out of sight, the way the horde arrives: outside the widest view and on
    // the horde's connected map. The draw comes from the boss's own stream
    // (`BossRng::for_run`, the same stream `open_fight` installs), so the run's
    // rng order is untouched.
    let mut boss_rng = BossRng::for_run(level.seed, kind).0;
    let radius = kind.body_radius();
    // The horde's scratch buffer is a resource the director owns; an arrival is
    // once a floor, so its own is built here rather than borrowed.
    let mut scratch = SpawnScratch::default();
    let spawn_point_reachable = crate::game::ecs::systems::spawn::spawn_point_reachable;
    let landing = spawn_point_reachable(
        &mut boss_rng,
        &level,
        &player,
        viewport.0,
        &mut scratch,
    );
    let pos = if kind == BossKind::Lantern {
        let landing = landing.filter(|&p| lantern::head_fits(&level, p)).or_else(|| {
            (0..7).find_map(|_| {
                spawn_point_reachable(&mut boss_rng, &level, &player, viewport.0, &mut scratch)
                    .filter(|&p| lantern::head_fits(&level, p))
            })
        });
        let Some(pos) = landing.or_else(|| lantern_arrival_beside(&level, player.pos)) else {
            // A temporarily blocked map can retry once the rat moves.
            due.claimed = false;
            override_kind.0 = Some(kind);
            return;
        };
        pos
    } else {
        // A rat holed up where the ring is solid still gets its fight beside
        // itself, exactly like a console `boss here`.
        landing.unwrap_or_else(|| arrival_beside(&level, player.pos, radius, &arena))
    };
    let boss = open_fight(
        &mut fight,
        Arrival {
            kind,
            pos,
            arena,
            dock,
            rat: player.pos,
            difficulty: difficulty.0,
            id: next_enemy_id.take(),
            seed: level.seed,
        },
    );
    let accent = level.palette.accent;
    vfx::emit_boss_spawn(&mut particles, &mut fight.rng.0, pos, accent);
    shake.amount = shake.amount.max(14.0);
    sfx.0.push(Sfx::Boss);
    commands.spawn((boss, next_spawn_id.take()));
}

/// Where a boss arrives when the horde's out-of-sight placement found no ring
/// point at all (a rat holed up inside a one-cell squeeze): the first
/// large-clear spot on a circle 48 px around the rat, else the nearest
/// large-clear cell to the fight rectangle's centre.
fn arrival_beside(level: &Level, rat: Vec2, radius: f32, arena: &TileRect) -> Vec2 {
    for k in 0..8 {
        let angle = k as f32 * (std::f32::consts::TAU / 8.0);
        let (s, c) = angle.dsin_cos();
        let p = rat + Vec2::new(c, s) * 48.0;
        if level.tiles.circle_clear(p, radius, SizeClass::Large) {
            return p;
        }
    }
    level
        .tiles
        .nearest_open_center(arena.center_world(), SizeClass::Large, 16)
        .unwrap_or_else(|| arena.center_world())
}

/// Legal Lantern placement for the scheduled fallback and `boss here`.
pub(crate) fn lantern_arrival_beside(level: &Level, rat: Vec2) -> Option<Vec2> {
    for distance in [48.0, 44.0, 52.0, 40.0, 56.0, 60.0] {
        for i in 0..16 {
            let dir = Vec2::from_angle(i as f32 * std::f32::consts::TAU / 16.0);
            let p = rat + dir * distance;
            if lantern::head_fits(level, p) {
                return Some(p);
            }
        }
    }
    nearest_body_clear(level, rat, art_boss_lantern::HEAD_RADIUS, SizeClass::Large, 24, true)
}

/// Apply player damage to the boss through its art-defined hit shapes.
///
/// Sources: player bullets (including orbit), acid pools and mousetraps.
/// Damage numbers and hit vfx use the existing hooks.
///
/// **Tail Whip is not one of them.** The whip resolves its own boss hit in
/// `weapons::fire_one`, with the reach and damage that cast actually used (Lens
/// Cap, Sharp Teeth and Counterweight included). This system used to re-derive
/// it from `Sfx::ShootWhip`, which made a boss's HP depend on whether the caller
/// drained the sound queue that tick — a simulation reading its own output.
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
            if boss.kind == BossKind::Lantern
                && boss.hp > 0.0
                && lantern::try_pop_decoy(&mut boss, &bullet, player.pos)
            {
                spend_boss_bullet(&mut bullet);
                sfx.0.push(Sfx::Splash);
                continue;
            }
            if !shapes.segment_hits_grounded(bullet.prev_pos, bullet.pos, bullet.radius) {
                continue;
            }
            // The crab's shell: a shot arriving over the dome from its front
            // hemisphere sparks off, does nothing, and is spent exactly like a
            // hit (a free wall pass would make the armour a gift). A pool or a
            // trap has no travel direction and is "underneath", which counts.
            let dir = if bullet.vel.len_sq() > 1e-4 {
                bullet.vel.normalized()
            } else {
                Vec2::ZERO
            };
            let armour = boss.incoming_multiplier(dir, bullet.pos);
            if armour <= 0.0 {
                vfx::emit_weapon_hit(
                    &mut particles,
                    &mut fight.rng.0,
                    bullet.pos,
                    bullet.vel,
                    bullet.weapon,
                    EnemyKind::Boss,
                    false,
                );
                sfx.0.push(Sfx::CrabShell);
                spend_boss_bullet(&mut bullet);
                continue;
            }
            let damage_mult = if boss.kind == BossKind::Lantern { 1.0 } else { boss.damage_mult };
            let amount = bullet.damage * damage_mult * armour;
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
            spend_boss_bullet(&mut bullet);
            sfx.0.push(Sfx::Hit);
        }
        // Acid pools (DPS through the hit shapes).
        for pool in pools.iter() {
            if !shapes.circle_hits_grounded(pool.pos, pool.radius) {
                continue;
            }
            let amount = pool.dps * tick.dt * boss.incoming_multiplier(Vec2::ZERO, pool.pos);
            if amount <= 0.0 {
                continue;
            }
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
            // Grounded shapes only, like the bullet and pool passes above: a
            // trap is on the floor, so it cannot bite a jaguar in mid-leap, a
            // raised slipper or the jammed Mangle's lifted frame.
            if shapes.iter().any(|s| {
                s.grounded() && in_mouse_kill_rect(trap.pos, trap.facing, s.center(), s_radius(s))
            }) {
                let damage_mult = if boss.kind == BossKind::Lantern { 1.0 } else { boss.damage_mult };
                let amount = MOUSETRAP_DAMAGE_ENEMY
                    * damage_mult
                    * boss.incoming_multiplier(Vec2::ZERO, trap.pos);
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

fn spend_boss_bullet(bullet: &mut Bullet) {
    if bullet.pierce > 0 {
        bullet.pierce -= 1;
    } else {
        bullet.life = 0.0;
    }
    bullet.hit_cd = 0.12;
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

/// Has `rat` left `arena` (the boss fight's rectangle) by more than half the
/// rectangle's own size?
///
/// The rectangle is built around the rat, so the only way to fail this is for
/// the rat to walk away from the fight: more than half a rectangle past the
/// nearer edge. Then the next neutral beat recentres it (`boss_update_system`).
fn rat_left_the_fight(arena: TileRect, rat: Vec2) -> bool {
    let world = arena.to_world();
    let half = world.w.min(world.h) * 0.5;
    let dx = (world.x - rat.x).max(rat.x - world.right()).max(0.0);
    let dy = (world.y - rat.y).max(rat.y - world.bottom()).max(0.0);
    dx.max(dy) > half
}

/// Where a rejoining boss is put: a large-clear cell within [`REJOIN_RADII`] of
/// the rat **with a walkable line to him**, nearest bearing first. Both halves
/// of that test matter: the boss and the rat can be 50 px apart on opposite
/// sides of a wall (the stalemate this exists to break), and a line through a
/// rat-hole is not a walk. Falls back to the closest clear cell to the rat.
fn rejoin_spot(level: &Level, player: Vec2, radius: f32, lantern: bool) -> Option<Vec2> {
    let class = SizeClass::Large;
    for step in 0..24 {
        let angle = step as f32 * std::f32::consts::TAU / 24.0;
        for r in REJOIN_RADII {
            let p = player + Vec2::from_angle(angle) * r;
            let fits = if lantern {
                lantern::head_fits(level, p) && lantern::head_path_fits(level, p, player)
            } else {
                level.tiles.circle_clear(p, radius, class)
                    && walkable_line(level, p, player, radius, class)
            };
            if fits {
                return Some(p);
            }
        }
    }
    // Nothing reachable on any ring: the rat is somewhere a large body cannot
    // walk to (a maze pocket, behind a rat-hole). Still bring the boss *near*
    // it — the siege rule below breaks the last wall — but never into one.
    let p = player + Vec2::new(REJOIN_RADII[0], 0.0);
    nearest_body_clear(level, p, radius, class, 24, lantern)
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
    lantern: bool,
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
                let fits = if lantern {
                    lantern::head_fits(level, centre)
                } else {
                    level.tiles.circle_clear(centre, radius, class)
                };
                if fits {
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

/// Recentre the fight rectangle on a rat that has genuinely left it.
///
/// Three gates, all of them there to stop a kiting rat from churning the
/// rectangle (see the long comment at the call site): the neutral beat (every
/// hazard an attack queued was painted against the old rectangle), a rate limit
/// ([`RECENTRE_COOLDOWN`]), and "the rat is not already in reach" — a rat the
/// boss can attack is a fight, not a departure.
fn recentre_the_fight(boss: &mut Boss, level: &Level, rat: Vec2) {
    if boss.recentre_cd > 0.0 || !boss.is_neutral() {
        return;
    }
    if (rat - boss.pos).len() <= boss.kind.engage_range() {
        return;
    }
    if !rat_left_the_fight(boss.arena, rat) {
        return;
    }
    boss.arena = level.fight_rect_around(rat);
    boss.dock = level
        .tiles
        .nearest_open_center(boss.arena.center_world(), SizeClass::Large, 16)
        .unwrap_or_else(|| boss.arena.center_world());
    boss.recentre_cd = RECENTRE_COOLDOWN;
}

/// The pursuit rule: a boss that cannot reach its target for
/// [`OUT_OF_REACH_SECS`] stops following and starts closing — a sprint, and a
/// leap when the straight line is body-passable (see the module docs).
///
/// Runs before the controller, and only in the neutral beat: a boss that is
/// mid-tell is fighting, not idling, and a controller's own art decides what its
/// committed beats look like. The leap never draws from the rng, so it cannot
/// perturb a controller's stream.
fn pursue(ctx: &BossContext<'_>, boss: &mut Boss, dt: f32) {
    if boss.dead {
        boss.out_of_reach = 0.0;
        boss.leap_t = 0.0;
        return;
    }
    let to = ctx.player_pos - boss.pos;
    let gap = to.len();
    // The clock measures distance, not the beat: it runs in every state and
    // only a rat back inside engage range resets it (several kinds leave their
    // neutral beat on a timer shorter than `OUT_OF_REACH_SECS`).
    if gap <= boss.kind.engage_range() {
        boss.out_of_reach = 0.0;
    } else {
        boss.out_of_reach += dt;
    }
    if !boss.is_neutral() {
        // A committed beat never moves under the pursuit rule.
        boss.leap_t = 0.0;
        return;
    }
    if boss.leap_t > 0.0 {
        // Committed to the leap: a straight dash at [`LEAP_SPEED`], steered at
        // the rat each tick so it cannot be side-stepped by a pixel.
        boss.leap_t = (boss.leap_t - dt).max(0.0);
        let dir = if gap > 1e-3 { to * (1.0 / gap) } else { boss.facing };
        if !move_boss(ctx, boss, dir, LEAP_SPEED) {
            // A leap into a wall ends early (the slide still moved it).
            boss.leap_t = 0.0;
        }
        return;
    }
    if gap <= boss.kind.engage_range() || boss.out_of_reach < OUT_OF_REACH_SECS {
        return;
    }
    let radius = boss.kind.body_radius();
    let has_leap_lane = || {
        if boss.kind == BossKind::Lantern {
            lantern::head_path_fits(ctx.level, boss.pos, ctx.player_pos)
        } else {
            walkable_line(ctx.level, boss.pos, ctx.player_pos, radius, SizeClass::Large)
        }
    };
    if boss.leap_cd <= 0.0
        && !boss.kind.own_gap_closer()
        && has_leap_lane()
    {
        boss.leap_t = LEAP_SECS;
        boss.leap_cd = LEAP_COOLDOWN;
        boss.out_of_reach = 0.0;
        let dir = if gap > 1e-3 { to * (1.0 / gap) } else { boss.facing };
        move_boss(ctx, boss, dir, LEAP_SPEED);
        return;
    }
    // No lane for a leap (a corner, a doorway): sprint along the flow field.
    let seek = ctx.seek(boss);
    if move_boss(ctx, boss, seek, PURSUIT_SPEED) && seek.len_sq() > 1e-6 {
        boss.facing = seek.normalized();
    }
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
        // The control budget's two windows and the pursuit clocks (see the
        // module docs). A window that runs out clears what it paid, so the next
        // grant opens a fresh one.
        if boss.control_window > 0.0 {
            boss.control_window = (boss.control_window - dt).max(0.0);
            if boss.control_window <= 0.0 {
                boss.control_used = 0.0;
            }
        }
        if boss.shove_window > 0.0 {
            boss.shove_window = (boss.shove_window - dt).max(0.0);
            if boss.shove_window <= 0.0 {
                boss.shove_used = 0.0;
            }
        }
        boss.leap_cd = (boss.leap_cd - dt).max(0.0);
        boss.recentre_cd = (boss.recentre_cd - dt).max(0.0);
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
                // The full boss's gold is **credited at the clear** (§6:
                // "Full boss 100% × 30, credit at clear"), not dropped as a
                // coin at its body. The old 20-gold coin sat where the boss
                // died: a ranged kill outside magnet range, followed by stage
                // completion banking only the wallet, meant the rat could
                // receive none of it. The wallet credit happens below, when
                // the death beat finishes and the level actually completes.
                shake.amount = shake.amount.max(16.0);
                // The horde breaks and flees: every remaining enemy is
                // removed this tick. Raising the flag (instead of despawning
                // here) keeps enemy removal out of the `EnemyIndex` validity
                // window — the removal itself is `loot::reap_enemies_system`'s
                // job, after the last index reader. See
                // [`BossFight::horde_break`].
                fight.horde_break = !counters.testbed;
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
            if boss.kind == BossKind::Lantern && level.objects.outfall_boss_snuff.is_some() {
                level.objects.outfall_boss_snuff = None;
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
                    // §6: the boss's own payout lands in the wallet at the
                    // clear, so it cannot be lost on the floor.
                    counters.gold = counters
                        .gold
                        .saturating_add(crate::game::shop::BOSS_GOLD);
                    sfx.0.push(Sfx::Gold);
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

        // The fight rectangle follows the rat. Batch 12 removed the arena, so
        // the rectangle the controllers call it is centred on the rat at the
        // moment the boss arrives; a rat that walks on gets it recentred, once,
        // from the neutral beat — the same gate the rejoin rule uses and for the
        // same reason: every hazard an attack queued was painted against the old
        // rectangle, so moving it mid-attack would leave the tell detached from
        // the shape that resolves.
        //
        // Batch 13 (the bosslock fix): the gate is now rate-limited and refuses
        // while the boss is in reach. A kiting rat left the rectangle every
        // neutral tick, so the rectangle — and, for the kinds that walk to
        // `boss.dock` (the mower docks, the broom's stomp clamps to the
        // rectangle), the *target floor the boss was walking to* — was rewritten
        // 2-3 times a second, several metres away from where the fight was
        // actually happening. Those kinds spent their neutral beats walking to a
        // rectangle that had moved again by the time they arrived, which is a
        // third way to starve a boss of its attacks. One recentre per
        // [`RECENTRE_COOLDOWN`] is plenty (the rectangle is 34 x 28 cells), and
        // a rat the boss can already reach is a fight, not a departure.
        recentre_the_fight(&mut boss, &level, player.pos);

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
            fight.stuck = boss.is_neutral() && moved < 1.5;
            fight.last_pos = boss.pos;
            fight.since_sample = 0.0;
        }
        let stuck = fight.stuck;
        if to_player.len() > REJOIN_DIST || no_los || stuck {
            boss.lost += dt;
            // Only out of the neutral beat. The timer keeps running through an
            // attack, but the move itself waits for it to finish: relocating a
            // boss mid-charge, mid-dive or on its walk to the dock dropped a
            // live body (contact damage and all) beside the rat while the
            // hazards it had queued stayed where they were painted, and fought
            // the Mower's `Docking` transit every five seconds. Every attack
            // chain returns to the neutral state, so the rule still cannot
            // strand a level; it just lands on the first neutral tick.
            if boss.lost >= RELOCATE_AFTER_SECS && boss.is_neutral() {
                if let Some(spot) = rejoin_spot(
                    &level,
                    player.pos,
                    boss.kind.body_radius(),
                    boss.kind == BossKind::Lantern,
                ) {
                    boss.lost = 0.0;
                    boss.pos = spot;
                    boss.target = player.pos;
                    let to = player.pos - spot;
                    if to.len_sq() > 1e-6 {
                        boss.facing = to.normalized();
                    }
                    if boss.kind == BossKind::Lantern {
                        lantern::relocate(&mut boss, &level);
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

        // Pursuit: a rat that kites from beyond the boss's engage range is
        // closed down (sprint, then leap) instead of followed forever. Runs
        // before the controller so a controller's own neutral walk composes on
        // top of it rather than fighting it.
        {
            let ctx = BossContext {
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
            pursue(&ctx, &mut boss, dt);
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
        if boss.kind == BossKind::Lantern {
            if let BossState::Lantern(state) = boss.state {
                if !art_boss_lantern::has_body_contact(state) {
                    boss.no_contact();
                }
            }
            if level.objects.outfall_boss_snuff != boss.lantern.snuff {
                level.objects.outfall_boss_snuff = boss.lantern.snuff;
            }
            // Existing melee and area passes read the shared multiplier
            // without a source direction. Point-blank serum stomps stay 40.
            boss.damage_mult = if player.buffs.is_giant() {
                1.0
            } else {
                lantern::feint_armor(&boss, player.pos - boss.pos)
            };
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
        boss.is_neutral()
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
                | BossState::Magpie(MagpieState::Stuck)
                | BossState::HorseshoeCrab(
                    CrabState::Wedged | CrabState::Flipped | CrabState::Recover
                )
                | BossState::MegaRat(
                    art_boss_megarat::MegaRatState::GnawStuck
                        | art_boss_megarat::MegaRatState::ThrashRecover
                )
                | BossState::Hawk(HawkState::TalonBury)
                | BossState::Cobra(CobraState::FangsStuck | CobraState::SpiralEnd)
                | BossState::Lantern(
                    LanternState::FeintRecover | LanternState::DecoyRecover
                        | LanternState::DarkRecover | LanternState::GapeRecover
                )
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
