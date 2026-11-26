//! The simulation facade.
//!
//! The simulation itself lives in a `bevy_ecs` world driven by one explicitly
//! chained, single-threaded schedule (see [`crate::game::ecs::schedule`]).
//! This type keeps the legacy public surface — the fields and methods used by
//! `src/app.rs`, `src/render/hud.rs` and the app tests — so those files compile
//! unchanged.
//!
//! ## Facade mechanism
//!
//! Public singleton fields (`rng`, `level`, `player`, `particles`, `director`,
//! `flow`, `elapsed`, `kills`, `score`, `shake`, `shake_ofs`,
//! `pending_levelups`, `game_over`, `won`, `level_complete`, `sfx`) are **real
//! fields** and are authoritative between ticks. `World::update`:
//!
//! 1. checks `finished()` (legacy: only particles settle), increments
//!    `elapsed`, then calls `begin_tick`, which moves the facade values into
//!    the Bevy world as resources (`RngRes`, `Level`, `Player`, `Particles`,
//!    `Director`, `FlowField`, plus `RunCounters`, `Shake`, `SfxQueue`,
//!    `TickInput`, `Viewport`);
//! 2. runs the schedule;
//! 3. calls `end_tick`, which moves the resources back into the facade fields.
//!
//! Entity components are owned by the Bevy world (they are not respawned every
//! tick), so `SpawnId` and `Entity` ids are stable across a run. Nothing is
//! copied out of the ECS world per tick: the HUD accessors read it directly
//! and [`World::enemies`]/[`World::bullets`]/... clone a snapshot only when a
//! test or the determinism harness asks for one. Tests and systems spawn
//! through the helpers below, which assign `SpawnId`.
//!
//! Internal scratch (`SpatialGrid`, `NeighborScratch`, `PickupScratch`,
//! `NextSpawnId`, `NukeTick`, `EnemyIndex`, `GridScratch`) lives permanently in
//! the Bevy world.
//!
//! ## Campaign structure (see [`super::campaign`])
//!
//! A `World` simulates **one level** of a three-level campaign (Sewer →
//! Kitchen or Laundry → Lab or Printworks), or one practice floor. The level index, the carried loadout and the banked totals
//! are constructor inputs:
//!
//! * [`World::new`] — level 0 with the default loadout (the legacy signature).
//! * [`World::new_level`] — any level index + a carried [`Loadout`].
//! * [`World::from_campaign`] — the App's entry point: everything a
//!   [`CampaignState`] holds, including the deterministic per-level map seed.
//!
//! The level's boss dying sets `level_complete` (the boss framework writes
//! `RunCounters::level_complete`; [`World::mark_level_complete`] is the facade
//! equivalent). The outcome rules live in `ecs::systems::loot::outcome_system`,
//! and only the **final** level's completion sets `won`. The clock does not end
//! anything: [`World::time_left`] counts down to `director::BOSS_TIME` and then
//! reports zero, and [`World::boss_time_reached`] marks the phase change.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World as EcsWorld;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::Sfx;
use crate::detmath::DetF32;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::{draw_ambience, draw_fixtures, LightMap, LightScene, LightingMode};
use crate::render::palette;
use crate::render::zoom;

use super::bullet::{Bullet, Faction};
use super::campaign::{self, CampaignState, CampaignTotals, Keepable};
use super::collision::{SizeClass, SpatialGrid, TileKind, TileMap};
use super::director::{self, Director};
pub use super::ecs::components::Pool;
use super::fog::{Fog, MazeMarker};
use super::ecs::components::{
    AcidDrop, BiteSnap, CheeseString, ChestSpawn, CorrosiveChannel, Drake, Dropping, EchoSpoke,
    FairyRing,
    FondueTine, Infection, MilkTooth, SpawnId, WhipCrack,
    LavaGlob, LavaPool,
};
use super::ecs::constants::{GRID_CELL, PARTICLE_CAP, PLAYER_RADIUS};
use super::ecs::resources::{
    collect_sorted, spawn_with_id, swap_in, swap_out, ActiveFx, BossDue, BossOverride, BossVirus,
    Cvars, DamageLedger, DashHold, DashTrack, DebugFlags, DifficultyLevel, DrakeSpawner,
    EnemyIndex, EvoRuntime, GridScratch,
    LaserFx, LastPickups, LevelIndex, MeleeNudge, NeighborScratch, NextEnemyId, NextSpawnId,
    NukeTick, PickupScratch, PuddleSteps, RngRes, RunCounters, SfxQueue, Shake, TickInput,
    Viewport, XpBank,
};
use super::ecs::schedule::{build_settle_schedule, build_tick_schedule};
use super::ecs::systems::boss::{
    Boss, BossArtView, BossFight, BossHitShape, BossKind, BossProjectile, BossProjectileKind,
    BossTargetable,
};
use super::ecs::systems::gates::{GateRuntime, MazeEvent};
use super::enemy::{Enemy, EnemyKind};
use super::art_gems;
use super::gem::{Chest, Gem, GemTier, Gold, Pickup, PickupKind};
use super::art_traps::MazeRewardState;
use super::level::{Level, PropKind, RoomKind, Theme};
use super::loadout::{self, ChestReward, Loadout, PassiveKind};
use super::nav::{FlowField, NavBudget};
use super::particle::damage_number::{DamageNumbers, DEFAULT_CAP as DAMAGE_NUMBER_CAP};
use super::particle::Particles;
use super::player::Player;
use super::shop::{ShopBook, SpliceKind};
use super::smoke::{SmokeLayer, SmokeScene};
use super::weapon::{Weapon, WeaponKind, LAVA_MOUTH_OFFSET, LAVA_MUZZLE_SECS};
use super::art_walls::{Cardinal, WallKind};
use super::{
    art_boss_broom, art_boss_cat, art_boss_cobra, art_boss_hawk, art_boss_horseshoe_crab,
    art_boss_magpie,
    art_boss_mangle, art_boss_megarat,
    art_boss_mower, art_boss_night_edition, art_enemy, art_floor, art_marks, art_miniboss,
    art_props, art_puddle, art_rat, art_rat_giant, art_traps, art_walls, art_wave3a, art_boss_lantern, art_roster_outfall,
};

/// How much of the simulation's screen shake reaches the camera. The owner
/// found the shake distracting, so every effect's amplitude is cut to 65 % in
/// this one place (the only place the offset is applied); durations are the
/// simulation's and unchanged, and nothing simulated reads the camera.
///
/// This is the **default** now: the Graphics page's SCREEN SHAKE row writes
/// [`World::shake_scale`], and the App hands it to each new world. Render-only
/// either way — the offset is applied where the camera is built for a draw, and
/// no system reads it.
pub const SHAKE_SCALE: f32 = 0.65;

pub struct World {
    /// The Bevy world: entity components + the singleton resources during a
    /// tick. Public so narrow (single-system) tests can drive it directly.
    pub ecs: EcsWorld,
    /// Full tick schedule (legacy `World::update` order).
    pub schedule: bevy_ecs::schedule::Schedule,
    /// Legacy `finished()` freeze path: particles only.
    pub settle_schedule: bevy_ecs::schedule::Schedule,

    // ---- Legacy public facade fields (authoritative between ticks) --------
    pub rng: StdRng,
    pub level: Level,
    pub player: Player,
    pub particles: Particles,
    /// Floating enemy damage numbers (neon merge). Authoritative between ticks;
    /// round-tripped through [`DamageNumbersRes`] during a tick.
    pub damage_numbers: DamageNumbers,
    /// Monotonic `Enemy::id` source (neon merge). Private: systems use
    /// [`NextEnemyId`], the facade spawn helpers use this field.
    next_enemy_id: u32,
    /// Pickups collected during the most recent `update` tick (neon merge),
    /// read by the HUD through [`World::last_pickups`].
    last_pickups: Vec<PickupKind>,
    pub director: Director,
    pub flow: FlowField,
    /// Console tunables (`set`/`get`). Authoritative between ticks; copied
    /// into the [`Cvars`] resource in `begin_tick`, so a command set between
    /// ticks reaches the next tick.
    pub cvars: Cvars,
    /// Console debug switches. Copied into [`DebugFlags`] in `begin_tick`.
    pub freeze: bool,
    pub peace: bool,
    /// The App's testbed uses manual boss and miniboss arrivals.
    pub(crate) testbed: bool,
    /// Console `noclip on`: the rat walks through walls.
    pub noclip: bool,
    /// Render-only: draw collision shapes over the world (console `hitboxes`).
    pub hitboxes: bool,
    /// How many times `update` has run (simulation and settle ticks alike).
    /// Render-side bookkeeping only — the App uses it to tell whether the
    /// picture can have changed since it was last drawn; no system reads it.
    pub ticks: u64,
    /// Console-only: the next boss spawn uses this kind instead of the floor's
    /// (`boss <kind>`). Taken by `boss_spawn_system` on the next spawn.
    pub boss_override: Option<BossKind>,
    /// Explored map cells (fog of war). Revealed by
    /// `ecs::systems::fog::reveal_system`; read by the minimap between ticks.
    pub fog: Fog,
    pub elapsed: f32,
    pub kills: u32,
    pub score: u32,
    pub gold: u32,
    pub pending_chests: u32,
    pub pending_shop: bool,
    /// Which shop hole raised `pending_shop` (`LevelObjects::shop_holes`
    /// index); the App opens this hole's shelf.
    pub pending_shop_hole: usize,
    /// Every shop hole's shelf and service state on this level (inventory
    /// survives a revisit). Between ticks only; the simulation never reads it.
    pub shop: ShopBook,
    pub shake: f32,
    pub shake_ofs: Vec2,
    pub pending_levelups: u32,
    pub game_over: bool,
    /// Gold spent in the splice shops since this floor started.
    ///
    /// Facade-only bookkeeping for the incident report's banking tally: nothing
    /// in the simulation reads it, and it is **not** round-tripped (no system
    /// spends gold; the App's shop screen does, between ticks). The campaign
    /// totals add each floor's figure up.
    pub gold_spent: u32,
    /// Revives still in hand this **run** (the nest's "Ninth Life").
    ///
    /// Set from `loadout.meta.revives` at the run's first floor, minus what the
    /// earlier floors of the campaign spent (`CampaignTotals::revives_used`). A
    /// fresh profile carries zero and the facade's revive path never runs.
    pub revives_left: u8,
    /// The run was won: the **final** campaign level was completed.
    ///
    /// Kept as a plain field because `app.rs`/`render` summarise a run from it;
    /// `outcome_system` only raises it for the final level, and
    /// [`World::campaign_won`] also accepts a final level whose completion the
    /// App has not ticked through yet.
    pub won: bool,
    /// Which campaign level this world simulates: 0 Sewer, 1 Kitchen, 2 Lab.
    /// See [`crate::game::campaign`].
    pub level_index: usize,
    /// Overrides the row of the director's difficulty tables in use (see
    /// [`DifficultyLevel`]); `None` follows `level_index`. A practice run sets
    /// `Some(0)`.
    pub difficulty_override: Option<usize>,
    /// The level's boss died ⇒ the level is complete (`RunCounters`
    /// `level_complete` round-trips here).
    pub level_complete: bool,
    /// Score/kills/time banked by the levels already completed, so the HUD can
    /// show cumulative numbers (`total_score`/`total_kills`/`total_time`).
    pub carried: CampaignTotals,
    pub cam: Vec2,
    /// Render-only: the point the camera frames instead of the rat. `None`
    /// (every run) follows the rat; the Library's card scenes frame their
    /// subject. No system reads it.
    pub cam_focus: Option<Vec2>,
    /// Render-only: skip the lighting pass (the Library's card plates are
    /// flat-lit, with the hostile layer self-lit as always). `false` in every
    /// run.
    pub flat_lit: bool,
    /// Render-only: draw no rat at all — neither the sprite
    /// ([`World::draw_player`]) nor the ink pass's minimum-size core and dash
    /// arc.
    ///
    /// The Library's bestiary plates are portraits of their subject, but every
    /// hostile behaviour in the simulation needs a target: the card keeps a
    /// hidden, invulnerable, scripted rat as the aim point and draws its own
    /// practice dummy at the same spot. The run never sets this; no system
    /// reads it.
    pub hide_rat: bool,
    /// The **simulation's** logical viewport: the unzoomed screen size, set by
    /// the App on resize (and by a headless caller explicitly). Never written
    /// by drawing — `render_viewport` is the renderer's own copy — so a run that
    /// is drawn every frame and one that is never drawn simulate identically.
    pub viewport: Vec2,
    /// The world rectangle the last [`World::draw`] covered (the zoomed buffer),
    /// used for culling only. Render state: nothing in the simulation reads it.
    pub render_viewport: Vec2,
    /// Mouse-wheel zoom setting: **how much world the view shows** relative to
    /// the unzoomed view (`1.0` = today's view, `2.0` = twice the width and
    /// height of world, `0.5` = half). Render-only — see [`crate::render::zoom`]
    /// and [`World::tick_zoom`]. The simulation never reads it: collision, the
    /// spawn ring and every system keep using the unzoomed [`World::viewport`],
    /// so a seed plays out identically at every zoom.
    pub zoom: f32,
    /// The eased zoom the renderer used last frame (chases `zoom` over about
    /// 0.15 s, so a wheel step glides instead of snapping).
    pub zoom_view: f32,
    /// Source pixels per screen pixel of the last draw (`1,1` unzoomed). The
    /// app's screen→world mapping (mouse steering, click targets) scales the
    /// cell offset by this so aim stays exact at every zoom.
    pub zoom_scale: Vec2,
    /// Scratch framebuffer the zoomed world is rendered into at world
    /// resolution before it is resampled onto the screen grid. Held for the
    /// whole run (only a size change reallocates it) so a zoomed frame does
    /// not allocate per tick.
    world_buf: Option<PixelBuffer>,
    /// The finest block-glyph set a frame may be drawn with
    /// ([`crate::render::subcell`]). Render-only; the App sets it from the
    /// terminal's capabilities and the `RATRUN_SUBCELL` pin. There is no
    /// setting: zoomed out it is always on.
    pub subcell: crate::render::subcell::SubcellSet,
    /// Edge treatment of the 1x (and zoomed-in) picture, off by default.
    /// Render-only. The App applies the Graphics setting or the session's
    /// `RATRUN_SMOOTH` override.
    pub smooth: crate::render::subcell::Smooth,
    /// The zoomed-out picture at sub-cell resolution, when the last `draw`
    /// produced one (see [`World::detail`]). Held for the run like `world_buf`.
    detail: crate::render::subcell::DetailFrame,
    detail_live: bool,
    /// The resampled screen picture before the damage digits and the ink pass
    /// stamped it: the difference is what the detail frame carries over.
    detail_base: PixelBuffer,
    /// The parallax foreground layer ([`crate::render::foreground`]): overhead
    /// objects drawn after the lighting pass and under the hostile layer. The
    /// item list is built from the finished level once per level and kept here
    /// as **render state** — never a resource, never in the determinism trace.
    pub foreground_layer: crate::render::foreground::ForeLayer,
    /// Whether the foreground layer draws. Render-only, off by default
    /// (`RATRUN_FOREGROUND=1` pins it on for a session; the console's
    /// read-only `foreground on|off` is the in-run switch).
    pub foreground: bool,
    /// The batch-13 lighting effects (`render::lighting::fx`). Render-only,
    /// on by default (`RATRUN_FX=0` pins them off for a session; the
    /// console's read-only `fx on|off` is the in-run A/B).
    pub lighting_fx: bool,
    /// SFX events emitted this tick; drained by the app after `update`.
    pub sfx: Vec<Sfx>,

    // ---- Render-only caches ----------------------------------------------
    /// When (`elapsed`) the Tail Thrash's charge first topped out, so the art
    /// can flash once as the tail becomes ready and then settle into its idle
    /// pulse. `NEG_INFINITY` = not ready. Never simulated, never round-tripped,
    /// never read by the schedule: the tail is drawn from `player.loadout` and
    /// only the *crossing* needs remembering (`art_rat::tail_ready_flash`).
    tail_ready_at: f32,
    /// Render-only Verlet physics whip chain for Spiked Tail / Barbed Wire.
    /// Never simulated, never round-tripped, never reads `World::rng`.
    whip_chain: art_rat::WhipChain,

    // ---- Facade round-trip placeholders ----------------------------------
    rng_spare: Option<StdRng>,
    level_spare: Option<Level>,
    player_spare: Option<Player>,
    particles_spare: Option<Particles>,
    damage_numbers_spare: Option<DamageNumbers>,
    director_spare: Option<Director>,
    flow_spare: Option<FlowField>,
    fog_spare: Option<Fog>,

    // ---- Render-only lighting & smoke (never touched by the sim) ---------
    pub lighting: LightingMode,
    pub(crate) lighting_depth: crate::term::ColorDepth,
    /// The share of the simulation's shake that reaches the camera
    /// ([`SHAKE_SCALE`] by default; the Graphics page's SCREEN SHAKE row, via
    /// the App). Render-only: read where `draw_scene` builds the camera and
    /// nowhere else, so no system can see it and no golden can move.
    pub shake_scale: f32,
    pub smoke_enabled: bool,
    light_map: LightMap,
    smoke: SmokeLayer,
}

impl World {
    /// Level 0 of the campaign with the default loadout: the constructor every
    /// existing caller (tests, `app.rs`) uses, unchanged.
    ///
    /// Use [`World::new_level`] (level index + carried loadout) or
    /// [`World::from_campaign`] to start any other level of a campaign.
    pub fn new(theme: Theme, seed: u64) -> World {
        World::new_level_with_theme(
            0,
            theme,
            seed,
            Loadout::starting(),
            CampaignTotals::default(),
        )
    }

    /// Level 0 on racks of `caps` slots (the profile's persistent slot upgrades,
    /// 4 to 8 each side). Every other knob is [`World::new`]'s.
    pub fn new_with_caps(theme: Theme, seed: u64, caps: loadout::SlotCaps) -> World {
        World::new_level_with_theme(
            0,
            theme,
            seed,
            Loadout::with_caps(Loadout::START_WEAPON, caps),
            CampaignTotals::default(),
        )
    }

    /// Start campaign level `level_index` from `seed`, carrying `carried` in.
    ///
    /// The theme comes from the level index (0 Sewer, 1 Kitchen, 2 Lab) and the
    /// cumulative counters start at zero — [`World::from_campaign`] is the
    /// app-facing constructor that also passes the banked totals.
    pub fn new_level(level_index: usize, seed: u64, carried: Loadout) -> World {
        World::new_level_with_theme(
            level_index,
            campaign::level_theme(level_index),
            seed,
            carried,
            CampaignTotals::default(),
        )
    }

    /// Start the level a [`CampaignState`] points at: its theme, its derived
    /// seed, its carried loadout and its banked totals.
    ///
    /// ```ignore
    /// // play → boss dies → keep three → next level
    /// let mut campaign = CampaignState::new(seed);
    /// let mut world = campaign.world();
    /// // ... world.level_complete() == true ...
    /// let keep = world.keepables();
    /// campaign.complete_level(&world, &keep[..3]).unwrap();
    /// world = campaign.world();
    /// ```
    pub fn from_campaign(campaign: &CampaignState) -> World {
        World::new_level_with_theme(
            campaign.level,
            campaign.theme(),
            campaign.level_seed(),
            campaign.loadout.clone(),
            campaign.totals,
        )
    }

    /// The full constructor: every knob a level start has.
    pub fn new_level_with_theme(
        level_index: usize,
        theme: Theme,
        seed: u64,
        carried: Loadout,
        carried_totals: CampaignTotals,
    ) -> World {
        // The level is generated from the level seed (same seed + theme = same
        // map, always). `Level::new` is the seed-0 convenience wrapper.
        let level = Level::generate(theme, seed);
        World::with_level(level_index, level, seed, carried, carried_totals)
    }

    /// [`World::new_level_with_theme`] over a level the caller already built.
    ///
    /// The Library's card scenes (`render::library::scene`) run the real
    /// schedule over a hand-built patch of floor instead of a generated map;
    /// every run still goes through `new_level_with_theme`, which generates
    /// its level and lands here.
    pub fn with_level(
        level_index: usize,
        mut level: Level,
        seed: u64,
        carried: Loadout,
        carried_totals: CampaignTotals,
    ) -> World {
        if level_index != 0 {
            level.objects.sewer_opening = None;
        }
        let theme = level.theme;
        let center = level.start;
        // Captured **before** `carried` is moved into the player below: the nest
        // grants the run's revives, minus what the earlier floors of the
        // campaign already spent (`CampaignTotals::revives_used`).
        let revives_left = carried
            .meta
            .revives
            .saturating_sub(carried_totals.revives_used);
        let mut flow = FlowField::new(&level.tiles);
        flow.update(&level.tiles, center, NavBudget::FULL);
        let flow_spare = FlowField::new(&level.tiles);
        // Fog of war: one bitset over this level's cells, reset every run (it
        // lives in the facade, so a new `World` starts fully unexplored).
        let fog = Fog::with_mazes(level.tiles.cols(), level.tiles.rows(), level.mazes.len());
        let fog_spare = fog.empty_like();

        let mut ecs = EcsWorld::new();
        ecs.insert_resource(NextSpawnId(0));
        ecs.insert_resource(NeighborScratch::default());
        ecs.insert_resource(PickupScratch::default());
        ecs.insert_resource(NukeTick(false));
        ecs.insert_resource(GridScratch::default());
        ecs.insert_resource(EnemyIndex::default());
        // Per-weapon damage bookkeeping for the batch harness (never read by
        // the schedule; fresh per level, like the counters below).
        ecs.insert_resource(DamageLedger::default());
        // Live state of the two player-triggered weapons (charge bursts and the
        // squeak ring). Permanent scratch: it is written only by
        // `weapons::active_weapons_system` and read between ticks by the draw
        // pass and the HUD, and a fresh `World` starts with both idle.
        ecs.insert_resource(ActiveFx::default());
        // Wave 3: Laser Eyes' live burst and Super Dash's dash bookkeeping.
        // Permanent scratch, like `ActiveFx`: written by their systems, read by
        // `World::draw` (the beam) and by tests between ticks.
        ecs.insert_resource(LaserFx::default());
        ecs.insert_resource(DashTrack::default());
        // Counterweight's per-body melee-nudge cooldowns and Second Scent's
        // fractional XP bank. Both are per-run scratch; a new level starts
        // with a full nudge budget and an empty bank.
        ecs.insert_resource(MeleeNudge::default());
        ecs.insert_resource(XpBank::default());
        // Per-run evolution bookkeeping (Fairy Ring's cast parity): a resource,
        // not a system `Local`, so it starts at zero with every level and the
        // schedule and any `run_system_cached` copy of the firing pass agree.
        ecs.insert_resource(EvoRuntime::default());
        ecs.insert_resource(super::ecs::resources::BeadHitClock::default());
        ecs.insert_resource(super::ecs::resources::HoldPointFx::default());
        // Wave 3 weapons A: the Hantavirus mark on the boss (a boss is not an
        // `Enemy`, so its mark cannot be a component on it). Permanent scratch,
        // like `ActiveFx`; a fresh level starts unmarked.
        ecs.insert_resource(BossVirus::default());
        ecs.insert_resource(SpatialGrid::new(level.arena, GRID_CELL));
        // The director's boss hand-off (raised at `director::BOSS_TIME`, taken
        // by the boss framework). Permanent, like `NukeTick`.
        ecs.insert_resource(BossDue::default());
        ecs.insert_resource(super::ecs::systems::outfall::OutfallOrigins::default());
        // The guest's clock (the Drake's 45 s gap and live count). Permanent
        // scratch, like `BossDue`; a fresh level starts with no Drake and no
        // gap running.
        ecs.insert_resource(DrakeSpawner::default());
        ecs.insert_resource(super::ecs::systems::floor_sewer::OpeningSpawns::default());
        // The boss fight's own state (private rng stream, damage-number id,
        // reusable scratch). Permanent, like `BossDue`; a spawn re-seeds it.
        ecs.insert_resource(BossFight::default());
        // Console debug switches and live tunables. Neutral defaults; the
        // facade fields are copied in by `begin_tick`.
        ecs.insert_resource(DebugFlags::default());
        ecs.insert_resource(Cvars::default());
        ecs.insert_resource(BossOverride::default());
        // Named miniboss appointments + spawn grace (wave 3).
        ecs.insert_resource(super::ecs::systems::miniboss::MinibossSchedule::default());
        ecs.insert_resource(super::ecs::resources::RatTrack::default());
        // Floor-trap side table (glue victims + mousetrap hit bookkeeping);
        // `traps::TrapRuntime::sync` sizes it from `level.objects`.
        ecs.insert_resource(super::ecs::systems::traps::TrapRuntime::default());
        // Interactive-object side table (one-way door latches/slam, rat-hole
        // squeeze + blocked latches, cache claims and the HUD maze feed);
        // `gates::GateRuntime::sync` sizes it from `level.objects`.
        ecs.insert_resource(super::ecs::systems::gates::GateRuntime::default());
        // Mouse-hole shop thresholds (the touch edge, the re-arm and the exit
        // grace). Permanent scratch like `GateRuntime`.
        ecs.insert_resource(super::ecs::systems::shop::ShopRuntime::default());
        // Wet-floor footsteps (the rat's splashes, the ripple rings and the
        // horde's per-slot cooldowns). Permanent scratch, written only by
        // `puddles::puddle_steps_system` and read between ticks by
        // `World::draw_scene` for the ripples; a fresh level starts with the rat
        // standing still at the start, nothing splashed yet.
        ecs.insert_resource(PuddleSteps::at(center));

        World {
            ecs,
            schedule: build_tick_schedule(),
            settle_schedule: build_settle_schedule(),

            rng: StdRng::seed_from_u64(seed),
            level,
            player: carried_player(center, carried),
            particles: Particles::new(PARTICLE_CAP),
            damage_numbers: DamageNumbers::new(DAMAGE_NUMBER_CAP),
            next_enemy_id: 1,
            last_pickups: Vec::new(),
            director: Director::default(),
            flow,
            cvars: Cvars::default(),
            freeze: false,
            peace: false,
            testbed: false,
            noclip: false,
            hitboxes: false,
            ticks: 0,
            boss_override: None,
            fog,
            elapsed: 0.0,
            kills: 0,
            score: 0,
            gold: carried_totals.gold,
            gold_spent: 0,
            revives_left,
            pending_chests: 0,
            pending_shop: false,
            pending_shop_hole: 0,
            shop: ShopBook::default(),
            shake: 0.0,
            shake_ofs: Vec2::ZERO,
            pending_levelups: 0,
            game_over: false,
            won: false,
            level_index,
            difficulty_override: None,
            level_complete: false,
            carried: carried_totals,
            cam: Vec2::ZERO,
            cam_focus: None,
            flat_lit: false,
            hide_rat: false,
            viewport: Vec2::new(160.0, 80.0),
            render_viewport: Vec2::new(160.0, 80.0),
            zoom: zoom::DEFAULT,
            zoom_view: zoom::DEFAULT,
            zoom_scale: Vec2::new(1.0, 1.0),
            world_buf: None,
            subcell: crate::render::subcell::SubcellSet::Off,
            smooth: crate::render::subcell::Smooth::default(),
            detail: crate::render::subcell::DetailFrame::new(),
            detail_live: false,
            detail_base: PixelBuffer::new(0, 0),
            foreground_layer: crate::render::foreground::ForeLayer::new(),
            foreground: crate::render::foreground::enabled_from_env(),
            lighting_fx: crate::render::lighting::fx_enabled_from_env(),
            sfx: Vec::new(),
            tail_ready_at: f32::NEG_INFINITY,
            whip_chain: art_rat::WhipChain::new(),

            // Unused twin of each round-tripped value. `swap_in` leaves the
            // facade holding this while the real value lives in the ECS
            // world; `swap_out` swaps them back. Two Level/Particles/... live
            // for the whole run (no per-tick allocation), and only the
            // facade-owned one is ever mutated.
            rng_spare: Some(StdRng::seed_from_u64(0)),
            // The round-trip spare is never simulated or drawn: a blank map is
            // enough and spares a full generation at startup.
            level_spare: Some(Level::empty(theme)),
            player_spare: Some(Player::new(center, WeaponKind::Gnaw)),
            particles_spare: Some(Particles::new(PARTICLE_CAP)),
            damage_numbers_spare: Some(DamageNumbers::new(DAMAGE_NUMBER_CAP)),
            director_spare: Some(Director::default()),
            flow_spare: Some(flow_spare),
            fog_spare: Some(fog_spare),

            lighting: LightingMode::default_for_depth(crate::term::color_depth()),
            lighting_depth: crate::term::color_depth(),
            shake_scale: SHAKE_SCALE,
            smoke_enabled: true,
            light_map: LightMap::new(),
            smoke: SmokeLayer::new(seed, theme),
        }
    }

    /// Will this world simulate any further? A finished world only settles
    /// particles and damage numbers (see [`super::ecs::schedule::build_settle_schedule`]).
    ///
    /// `won` is included for the App's benefit: it is set directly by a few
    /// tests and summaries, and a world that has already been declared won must
    /// not keep simulating even before `level_complete` round-trips.
    pub fn finished(&self) -> bool {
        self.game_over || self.level_complete || self.won
    }

    /// Seconds until the boss fight starts (`director::BOSS_TIME`), then zero.
    ///
    /// The level no longer ends at a clock time: from the boss time on the
    /// level lasts until the boss or the player dies, so the HUD's countdown
    /// saturates at `0` and `boss_time_reached()` takes over.
    pub fn time_left(&self) -> f32 {
        (director::BOSS_TIME - self.elapsed).max(0.0)
    }

    /// Has the level reached the boss time (the horde now fights on until the
    /// boss or the player dies)?
    pub fn boss_time_reached(&self) -> bool {
        !self.testbed && self.elapsed >= director::BOSS_TIME
    }

    /// Spend from the run's wallet. A failed purchase leaves the balance intact.
    #[allow(dead_code)] // The shop will call this for purchases.
    pub fn spend_gold(&mut self, n: u32) -> bool {
        let Some(balance) = self.gold.checked_sub(n) else {
            return false;
        };
        self.gold = balance;
        self.gold_spent = self.gold_spent.saturating_add(n);
        true
    }

    /// Revives this rat has already used this run (`0` for a fresh profile).
    pub fn revives_used(&self) -> u8 {
        self.player
            .loadout
            .meta
            .revives
            .saturating_sub(self.revives_left)
    }

    /// The shop threshold scratch (permanent ECS resource; used between ticks
    /// by the App and the bot to grant the 0.8 s exit grace).
    pub fn shop_runtime(&self) -> &super::ecs::systems::shop::ShopRuntime {
        self.ecs
            .get_resource::<super::ecs::systems::shop::ShopRuntime>()
            .expect("ShopRuntime is inserted in World::new")
    }

    /// The same, mutably (between ticks only).
    pub fn shop_runtime_mut(&mut self) -> &mut super::ecs::systems::shop::ShopRuntime {
        self.ecs
            .get_resource_mut::<super::ecs::systems::shop::ShopRuntime>()
            .expect("ShopRuntime is inserted in World::new")
            .into_inner()
    }

    /// Where the bot should walk to do some shopping, if buying would do
    /// something useful right now (`None` otherwise).
    ///
    /// This is a **query**: it peeks at the shelf a hole *would* stock
    /// (`ShopBook::peek`, a pure function of the floor seed and the hole index)
    /// without opening it, so the bot never pays a shelf into existence, and it
    /// never draws rng. The candidate is the nearest hole with an affordable,
    /// useful item: a splice while a socket is free, or a Stitch when the rat
    /// is hurt.
    pub fn shop_goal_point(&self) -> Option<Vec2> {
        use crate::game::shop::{self, ShopBook, STITCH_PRICE};
        let splices = &self.player.loadout.splices;
        let sockets_free = splices.len() < shop::MAX_SPLICES;
        let hurt = self.player.hp < self.player.max_hp() * 0.65;
        let mut best: Option<(f32, Vec2)> = None;
        for (index, hole) in self.level.objects.shop_holes.iter().enumerate() {
            let live = self.shop.get(index);
            let shelf = match live {
                Some(stock) => {
                    let mut shelf = [None; shop::STOCK_SIZE];
                    for (slot, value) in shelf.iter_mut().enumerate() {
                        if !stock.sold.get(slot).copied().unwrap_or(false) {
                            *value = stock.slot(slot);
                        }
                    }
                    shelf
                }
                None => ShopBook::peek(self.level.seed, index),
            };
            let splice_wanted = sockets_free
                && shelf.iter().flatten().any(|kind| {
                    !splices.contains(kind) && self.gold >= kind.price()
                });
            let stitch_wanted = hurt
                && live.is_none_or(|stock| !stock.stitched)
                && self.gold >= STITCH_PRICE;
            if !splice_wanted && !stitch_wanted {
                continue;
            }
            let d = self.player.pos.dist(hole.face());
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, hole.face()));
            }
        }
        best.map(|(_, face)| face)
    }

    /// Resolve one queued chest, including any gold granted by the reward.
    pub fn claim_chest(&mut self) -> Option<ChestReward> {
        if self.pending_chests == 0 {
            return None;
        }
        self.pending_chests -= 1;
        let reward = loadout::open_chest(&mut self.player.loadout, &mut self.rng);
        match reward {
            ChestReward::Gold(value) => self.gold = self.gold.saturating_add(value),
            // A Fat Reserves rank raises the cap **and** heals, exactly as the
            // level-up card does (`passive_stack_heal`); the chest used to
            // discard that heal.
            ChestReward::PassiveLevel { kind, .. } => {
                let heal = loadout::passive_stack_heal(kind);
                if heal > 0.0 {
                    self.player.heal(heal);
                }
            }
            _ => {}
        }
        Some(reward)
    }

    // ---- Campaign accessors --------------------------------------------

    /// Which campaign level this world simulates (0 Sewer, 1 Kitchen, 2 Lab).
    #[inline]
    pub fn level_index(&self) -> usize {
        self.level_index
    }

    /// The director's curve clock at a given elapsed time, including the
    /// difficulty override used by practice runs.
    pub(crate) fn curve_clock_at(&self, elapsed: f32) -> f32 {
        director::curve_clock(self.difficulty_override.unwrap_or(self.level_index), elapsed)
    }

    /// Render-side fixture outage on the director's clock.
    pub(crate) fn blackout(&self) -> f32 {
        crate::render::lighting::blackout::factor(
            self.level.theme,
            self.curve_clock_at(self.elapsed),
        )
    }

    /// Is this the campaign's last level? Completing it wins the run.
    #[inline]
    pub fn is_final_level(&self) -> bool {
        campaign::is_final_level(self.level_index)
    }

    /// Has this level's boss died? (The level is then complete; the App awards
    /// the keep-three screen.)
    #[inline]
    pub fn level_complete(&self) -> bool {
        self.level_complete
    }

    /// Did the **campaign** finish? That is the final level's completion: the
    /// Lab boss died.
    #[inline]
    pub fn campaign_won(&self) -> bool {
        self.won || (self.level_complete && self.is_final_level())
    }

    /// Mark the level complete from outside the simulation (the boss framework
    /// sets `RunCounters::level_complete` during a tick; this is the facade
    /// equivalent for tests and for an App that observed the boss die).
    ///
    /// Sets `won` as well on the final level, so a world marked complete between
    /// ticks reads correctly even though the settle path does not run
    /// `outcome_system`.
    pub fn mark_level_complete(&mut self) {
        self.level_complete = true;
        if self.is_final_level() {
            self.won = true;
        }
    }

    /// Cumulative score across the campaign (completed levels + this one).
    #[inline]
    pub fn total_score(&self) -> u32 {
        self.carried.score + self.score
    }

    /// Cumulative kills across the campaign.
    #[inline]
    pub fn total_kills(&self) -> u32 {
        self.carried.kills + self.kills
    }

    /// Cumulative seconds played across the campaign.
    #[inline]
    pub fn total_time(&self) -> f32 {
        self.carried.elapsed + self.elapsed
    }

    /// Everything the player may keep between levels: one entry per owned
    /// weapon (at its level) and per passive (at its stacks), in a stable order
    /// with display strings. See [`crate::game::campaign::keepables`].
    pub fn keepables(&self) -> Vec<Keepable> {
        campaign::keepables(&self.player.loadout)
    }

    // ---- Read-only HUD accessors (neon merge) ---------------------------
    //
    // The neon (07e26c9) legacy build answered these from the `Vec` caches
    // `end_tick` rebuilt. The port has no caches: they read the Bevy world
    // directly through `World::iter_entities` (`&self`, no allocation), so the
    // HUD call that follows `update` sees the tick's results and the facade
    // does no per-tick entity copying.

    /// The living boss, if one is active.
    ///
    /// The P7 boss framework owns a dedicated [`Boss`] component (the art's
    /// hit shapes are per-boss, so a generic `Enemy` cannot express them). The
    /// legacy `EnemyKind::Boss` fallback stays for tests and any pre-P7 spawn.
    fn living_boss(&self) -> Option<&Boss> {
        self.ecs
            .iter_entities()
            .filter_map(|entity| entity.get::<Boss>())
            .find(|b| !b.dead)
    }

    /// The legacy `EnemyKind::Boss` entity, if a test spawned one.
    fn living_enemy_boss(&self) -> Option<&Enemy> {
        self.ecs
            .iter_entities()
            .filter_map(|entity| entity.get::<Enemy>())
            .find(|e| e.kind == EnemyKind::Boss && e.alive())
    }

    /// Current and maximum HP of the living boss, if one is active.
    pub fn boss_hp(&self) -> Option<(f32, f32)> {
        if let Some(b) = self.living_boss() {
            return Some((b.hp.max(0.0), b.max_hp));
        }
        self.living_enemy_boss()
            .map(|e| (e.hp.max(0.0), e.max_hp))
    }

    /// Is a chest lying unopened on the floor?
    ///
    /// The HUD's one-shot chest hint (`render::hud::HudState`) asks this so a
    /// new player learns that the crate is collected by touch. `&self` and
    /// allocation-free, like the other HUD accessors; it early-outs on the first
    /// chest found and is only ever asked until the hint has fired.
    pub fn chest_on_floor(&self) -> bool {
        self.ecs
            .iter_entities()
            .any(|entity| entity.get::<Chest>().is_some())
    }

    /// Distance from the rat to the nearest mouse-hole shop's face, if the
    /// level has any (the HUD's one-shot shop hint).
    pub fn nearest_shop_hole(&self) -> Option<f32> {
        self.level
            .objects
            .shop_holes
            .iter()
            .map(|hole| hole.face().dist(self.player.pos))
            .min_by(f32::total_cmp)
    }

    /// Current fight phase of the living boss (`1..=3`, 1 = healthiest).
    ///
    /// The boss framework derives it from the remaining HP fraction with the
    /// per-kind thresholds ([`BossKind::phase_for`](crate::game::ecs::systems::boss::BossKind::phase_for)):
    /// the cat and the broom have two phases, the mower three. `None` when no
    /// `Boss` entity is alive (the legacy `EnemyKind::Boss` fallback has none).
    pub fn boss_phase(&self) -> Option<u8> {
        self.living_boss().map(|b| b.phase)
    }

    /// The living boss's kind, if one is active.
    ///
    /// A floor can pool more than one boss (`BossKind::pool_for_theme`), so the
    /// HUD names the fight the player is *in* from this rather than assuming
    /// the floor's original boss. `&self`, allocation-free, like the other HUD
    /// accessors.
    pub fn boss_kind(&self) -> Option<BossKind> {
        self.living_boss().map(|b| b.kind)
    }

    pub(crate) fn outfall_surge_enabled(&self) -> bool {
        if self.level.theme != Theme::Outfall {
            return false;
        }
        let due = self.ecs.resource::<BossDue>();
        super::hazards::outfall_surge_enabled(
            self.director.boss_spawned
                || due.is_pending()
                || due.claimed
                || self.ecs.iter_entities().any(|e| {
                    e.get::<Boss>().is_some_and(|boss| boss.kind == BossKind::Lantern)
                }),
            self.testbed,
            self.elapsed,
        )
    }

    /// Is the living boss out of reach (the Redtail overhead, only a shadow)?
    ///
    /// The HUD's boss bar and name plate read this so the rat is told why its
    /// weapons are finding nothing (`render::overlay`, `render::hud::boss_plate`).
    /// `&self`, allocation-free, like the other HUD accessors.
    pub fn boss_overhead(&self) -> bool {
        self.living_boss().map(|b| b.overhead()).unwrap_or(false)
    }

    /// Targetable points for auto-aim: the boss's art-defined hit shapes.
    ///
    /// The boss is hit through its art shapes (three body circles for the cat,
    /// two feet for the broom, one chassis circle for the mower), so a weapon
    /// that aims at "the boss" should aim at one of these points.
    pub fn boss_target_points(&self) -> Option<BossTargetable> {
        self.living_boss().map(|b| b.target_points())
    }

    /// World-space offset from the player to the living boss, if active.
    pub fn boss_offset(&self) -> Option<Vec2> {
        if let Some(b) = self.living_boss() {
            return Some(b.pos - self.player.pos);
        }
        self.living_enemy_boss().map(|e| e.pos - self.player.pos)
    }

    // ---- Named minibosses (wave 3) -------------------------------------

    /// The living miniboss, if one is active.
    pub(crate) fn living_miniboss(&self) -> Option<(&Enemy, &super::ecs::systems::miniboss::Miniboss)> {
        self.ecs.iter_entities().find_map(|entity| {
            let e = entity.get::<Enemy>()?;
            let mb = entity.get::<super::ecs::systems::miniboss::Miniboss>()?;
            if e.alive() && !mb.chest_dropped {
                Some((e, mb))
            } else {
                None
            }
        })
    }

    /// The living miniboss's `(name, hp, max_hp)` for the nameplate.
    pub fn miniboss_hp(&self) -> Option<(&'static str, f32, f32)> {
        self.living_miniboss().map(|(e, mb)| {
            (mb.kind.name(), e.hp.max(0.0), e.max_hp)
        })
    }

    /// The living miniboss's world position.
    pub fn miniboss_marker(&self) -> Option<Vec2> {
        self.living_miniboss().map(|(e, _)| e.pos)
    }

    /// The next appointment clock, if one is pending.
    pub fn next_miniboss_time(&self) -> Option<f32> {
        if self.testbed {
            return None;
        }
        self.ecs
            .resource::<super::ecs::systems::miniboss::MinibossSchedule>()
            .next_appointment()
    }

    /// Is a boss-phase cleanup pace in effect? (The director reads this through
    /// its own timer; the HUD only needs the flag.)
    pub fn cleanup_pace(&self) -> bool {
        self.boss_time_reached()
    }

    // ---- Fog of war / minimap accessors --------------------------------
    //
    // Allocation-free `&self` reads of the explored set and of the markers the
    // minimap draws. The minimap source (`render::minimap_source`) builds its
    // view struct from these once per frame.

    /// The explored-cell set (fog of war).
    #[inline]
    pub fn fog(&self) -> &Fog {
        &self.fog
    }

    /// Has the player explored map cell `(cx, cy)`?
    #[inline]
    pub fn explored(&self, cx: i32, cy: i32) -> bool {
        self.fog.is_seen(cx, cy)
    }

    /// `1.0` for a cell revealed this tick, fading to `0.0` after
    /// [`crate::game::fog::FOG_FADE_TICKS`] (the minimap's "seen recently"
    /// glow). `0.0` for unexplored cells.
    #[inline]
    pub fn explored_recency(&self, cx: i32, cy: i32) -> f32 {
        self.fog.recency(cx, cy)
    }

    /// The boss marker: where the living boss is.
    ///
    /// The minimap shows it while the boss is alive (fog does not hide the
    /// threat the boss bar and the compass already announce).
    pub fn boss_marker(&self) -> Option<Vec2> {
        if let Some(b) = self.living_boss() {
            return Some(b.pos);
        }
        self.living_enemy_boss().map(|e| e.pos)
    }

    /// The boss-arena marker: its cell rectangle, once any of its cells has
    /// been explored.
    pub fn boss_arena_marker(&self) -> Option<super::level::TileRect> {
        let rect = self.level.boss_room()?.rect;
        self.fog.any_seen_in(rect).then_some(rect)
    }

    /// Discovery state of every maze section, for the minimap.
    ///
    /// `entrance_seen` and `claimed` are the fog's persistent per-maze flags; a
    /// maze's entrance marker therefore survives the player walking away, while
    /// the reward star (`reward_seen`) only exists while its cell is explored.
    /// `claimed` also reads the reward view's `Opened` state, so the maze-reward
    /// runtime (a later phase) can drive it either way.
    pub fn maze_markers(&self) -> impl Iterator<Item = MazeMarker> + '_ {
        let fog = &self.fog;
        self.level
            .mazes
            .iter()
            .enumerate()
            .map(move |(index, maze)| {
                let (cx, cy) = crate::game::collision::cell_of(maze.reward);
                let reward_view = self
                    .level
                    .objects
                    .rewards
                    .get(index)
                    .map(|r| r.state == MazeRewardState::Opened)
                    .unwrap_or(false);
                MazeMarker {
                    index,
                    entrance: maze.entry,
                    reward: maze.reward,
                    entrance_seen: fog.maze_entrance_seen(index),
                    reward_seen: fog.is_seen(cx, cy),
                    claimed: fog.maze_claimed(index) || reward_view,
                }
            })
    }

    /// Number of active enemies counted against the horde budget.
    pub fn enemy_count(&self) -> usize {
        self.ecs
            .iter_entities()
            .filter(|entity| entity.get::<Enemy>().is_some_and(|e| !e.summoned))
            .count()
    }

    /// Pickups collected during the most recent `update()` tick.
    pub fn last_pickups(&self) -> &[PickupKind] {
        &self.last_pickups
    }

    // ---- Maze object rail (HUD) -----------------------------------------
    //
    // The object layer (`systems/gates.rs`) reports cache claims and gate
    // slams in `GateRuntime`: the feed only holds the most recent tick, the
    // per-object records hold the one-shot latches. The HUD reads both so a
    // callout can be neither duplicated nor lost.

    /// The maze object layer's event feed for the most recent tick.
    pub fn maze_feed(&self) -> &[MazeEvent] {
        self.ecs.resource::<GateRuntime>().feed()
    }

    /// Did maze `index`'s one-way exit slam shut behind the rat? A persistent
    /// one-shot latch (the art's `DoorRuntime::slammed`).
    ///
    /// Door `i` is maze `i`'s exit: the generator pushes them in `mazes` order.
    pub fn maze_gate_slammed(&self, index: usize) -> bool {
        self.ecs
            .resource::<GateRuntime>()
            .doors
            .get(index)
            .is_some_and(|door| door.slammed)
    }

    // ---- Entity snapshots (tests + determinism harness) ------------------
    //
    // The facade used to expose `enemies`/`bullets`/`gems`/`pickups`/`pools`
    // as `pub Vec` caches rebuilt every tick (a full clone of every entity, per
    // tick). Nothing in `app.rs`/`render/` reads them, so they are gone; these
    // allocate once per call instead, in stable `SpawnId` order, for the code
    // that really wants a whole list.

    /// Every live enemy, in stable `SpawnId` order.
    pub fn enemies(&mut self) -> Vec<Enemy> {
        collect_sorted(&mut self.ecs)
    }

    /// Every live bullet, in stable `SpawnId` order.
    pub fn bullets(&mut self) -> Vec<Bullet> {
        collect_sorted(&mut self.ecs)
    }

    /// Every live gem, in stable `SpawnId` order.
    pub fn gems(&mut self) -> Vec<Gem> {
        collect_sorted(&mut self.ecs)
    }

    /// Every live pickup, in stable `SpawnId` order.
    pub fn pickups(&mut self) -> Vec<Pickup> {
        collect_sorted(&mut self.ecs)
    }

    /// Every live acid pool, in stable `SpawnId` order.
    pub fn pools(&mut self) -> Vec<Pool> {
        collect_sorted(&mut self.ecs)
    }

    /// Every live Hantavirus dropping, in stable `SpawnId` order (the order the
    /// cap evicts them and the draw pass paints them in).
    pub fn droppings(&mut self) -> Vec<Dropping> {
        collect_sorted(&mut self.ecs)
    }

    /// Live state of the two Space-triggered weapons, as of the last tick.
    ///
    /// The resource lives permanently in the ECS world (it is not a facade
    /// field), so this copies it out only when asked — the draw pass asks once
    /// per frame (`thrash_visual`/`squeak_visual`), and tests ask directly.
    pub fn active_fx(&self) -> ActiveFx {
        *self.ecs.resource::<ActiveFx>()
    }

    /// The tail-thrash arc to draw this frame: `(angle, reach, brightness)`,
    /// `None` when no burst is playing. `brightness` fades across the burst.
    pub fn thrash_visual(&self) -> Option<(f32, f32, f32)> {
        let fx = self.ecs.resource::<ActiveFx>();
        if !fx.thrash_active() {
            return None;
        }
        // Fade in over the first tenth of the burst, out over the last quarter,
        // so the arc snaps on with the shake and lingers as it winds down.
        let t = fx.thrash_progress();
        let fade = (t / 0.08).min(1.0) * ((1.0 - t) / 0.28).clamp(0.0, 1.0);
        Some((fx.thrash_angle, fx.thrash_reach, fade))
    }

    /// The Laser Eyes beams to draw this frame, if its bar is lit.
    ///
    /// A sweep is **two** beams closing inward (one from each edge of the arc),
    /// so both are carried here and `art_rat::draw_laser_eyes` draws the pair
    /// plus the flash where they meet on the aim line. `brightness` is steady
    /// for the whole sweep and only fades at its very end. The twin muzzle dots
    /// stay lit through the sweep.
    pub fn laser_visual(&self) -> Option<art_rat::LaserBeam> {
        let fx = self.ecs.resource::<LaserFx>();
        if !fx.active() {
            return None;
        }
        Some(art_rat::LaserBeam {
            origin: fx.origin,
            angle: fx.angle,
            len: fx.beam,
            angle_b: fx.angle_b,
            len_b: fx.beam_b,
            aim: fx.facing.angle(),
            brightness: fx.brightness(),
            muzzle: 1.0,
        })
    }

    /// The Spiked Tail's lash to draw this frame, if one is in flight.
    ///
    /// Every field is the sim's own clock (`components::Lash::timer`) turned
    /// into draw weights with the same `weapon::LASH_*` constants the damage
    /// schedule uses, so the crack the player sees and the crack that lands can
    /// never drift apart. No frame cache, no rng, nothing written.
    pub fn lash_visual(&mut self) -> Option<art_rat::LashVisual> {
        use crate::game::ecs::components::Lash;
        use crate::game::weapon::{LASH_RECOIL_AT, LASH_TOTAL, LASH_WINDUP};
        let mut q = self
            .ecs
            .query::<(Entity, &crate::game::ecs::components::SpawnId, &Lash)>();
        let mut live: Option<(u64, Lash)> = None;
        for (_, id, lash) in q.iter(&self.ecs) {
            if live.is_none_or(|(best, _)| id.0 < best) {
                live = Some((id.0, *lash));
            }
        }
        let (_, lash) = live?;
        let total = LASH_TOTAL.max(1e-4);
        let t = (lash.timer / total).clamp(0.0, 1.0);
        let wind = LASH_WINDUP / total;
        let recoil = LASH_RECOIL_AT / total;
        // The snap out is the fastest thing in the game's animation set: three
        // frames. Everything after it is the beat it holds and the reel back.
        let snap_t = (0.045 / total).max(1e-3);
        let ease_out = |u: f32| -> f32 {
            let u = u.clamp(0.0, 1.0);
            1.0 - (1.0 - u) * (1.0 - u)
        };
        let ext = if t < wind {
            0.0
        } else if t < wind + snap_t {
            ease_out((t - wind) / snap_t)
        } else if t < recoil {
            1.0
        } else {
            let u = ((t - recoil) / (1.0 - recoil).max(1e-4)).clamp(0.0, 1.0);
            1.0 - u * u
        };
        let after = |window: f32| -> f32 {
            if !lash.struck || t < wind {
                0.0
            } else {
                (1.0 - (t - wind) / (window / total).max(1e-4)).clamp(0.0, 1.0)
            }
        };
        let bristle = if t < wind {
            (t / wind.max(1e-4)).clamp(0.0, 1.0)
        } else {
            (1.0 - (t - 0.85).max(0.0) / 0.15).clamp(0.0, 1.0)
        };
        let to_target = lash.target - lash.origin;
        Some(art_rat::LashVisual {
            dir: if to_target.len_sq() > 1e-4 {
                to_target.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            },
            dist: to_target.len(),
            ext,
            bristle,
            flash: after(0.10),
            smear: after(0.07),
            star: after(0.18),
            radius: lash.radius,
            point: lash.target,
        })
    }

    /// The drawn squeak ring: `(origin, radius)` while one is expanding.
    pub fn squeak_visual(&self) -> Option<(Vec2, f32)> {
        let fx = self.ecs.resource::<ActiveFx>();
        if !fx.squeak_active() {
            return None;
        }
        Some((fx.squeak_pos, fx.squeak_r))
    }

    pub fn update(&mut self, dt: f32, move_dir: Vec2, dash: bool) {
        // One press, two jobs: the legacy Space press dashes *and* fires the
        // charged active weapons (`active` mirrors `dash`).
        self.update_with(dt, move_dir, dash, dash);
    }

    /// [`update`](Self::update) with the dash and the active-weapon press told
    /// apart, which is what the App does on a terminal that reports a lone
    /// Ctrl press (Ctrl dashes, Space only fires the actives). The bot and the
    /// harness keep using `update`, where one press is both.
    pub fn update_with(&mut self, dt: f32, move_dir: Vec2, dash: bool, active: bool) {
        // No aim: auto-aim exactly as before (the bot, the harness and every
        // pre-wave-3 caller).
        self.update_with_aim(dt, move_dir, dash, active, None);
    }

    /// [`update_with`](Self::update_with) plus an **aim point** in world
    /// pixels (`None` = auto-aim). Aim is input, so two runs with the same seed
    /// and the same recorded aim reproduce exactly (`ecs/tests` has the
    /// twin-run proof).
    pub fn update_with_aim(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
    ) {
        self.update_with_charge(dt, move_dir, dash, active, aim, 0.0);
    }

    /// [`update_with_aim`](Self::update_with_aim) plus the Super Dash charge
    /// (`0.0` = a plain tap-dash, which is what every pre-wave-3 caller gets).
    ///
    /// The bot uses this to fire a charged dash (`game::bot`): the signature of
    /// `update`/`update_with` stays exactly as it was, so the goldens and every
    /// older test are byte-identical.
    pub fn update_with_charge(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
        charge: f32,
    ) {
        self.update_full(dt, move_dir, dash, active, aim, charge, DashHold::NONE);
    }

    /// [`update_with_charge`](Self::update_with_charge) plus the dash button's
    /// **live held state and edges** — how the App runs the hold-to-charge
    /// Super Dash. `dash` keeps its press-to-dash meaning; `hold.down` starts a
    /// charge, `hold.held` fills the meter and `hold.up` fires it. A focus loss
    /// drops the hold (both edges false, `held` false): the charge is cancelled,
    /// never fired.
    ///
    /// Another entry point rather than a new argument on `update_with`, so the
    /// bot, the harness `update` path and every existing test keep their exact
    /// signature and behaviour.
    pub fn update_with_dash_hold(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
        hold: DashHold,
    ) {
        self.update_full(dt, move_dir, dash, active, aim, 0.0, hold);
    }

    #[allow(clippy::too_many_arguments)]
    fn update_full(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
        charge: f32,
        hold: DashHold,
    ) {
        let _prof = crate::prof::scope(crate::prof::Stage::SimTick);
        self.ticks = self.ticks.wrapping_add(1);
        // Neon merge: the HUD reads only this tick's pickups.
        self.last_pickups.clear();
        if self.finished() {
            // Keep particles settling for a beat, but freeze the sim.
            self.begin_tick_full(dt, move_dir, dash, active, aim, charge, hold);
            self.settle_schedule.run(&mut self.ecs);
            self.end_tick();
            return;
        }
        // Legacy `update` line 123.
        self.elapsed += dt;

        self.begin_tick_full(dt, move_dir, dash, active, aim, charge, hold);
        self.schedule.run(&mut self.ecs);
        self.end_tick();
        self.try_revive();
    }

    /// Get up once per run (the nest's **Ninth Life**).
    ///
    /// If this tick killed the rat and a revive is still in hand, stand back up
    /// at half health behind a generous immunity window instead of ending the
    /// run. Facade bookkeeping only: no rng draw, no system, no schedule slot,
    /// and a run with no profile has zero revives, so it never reaches the
    /// writes below and every existing trace is unchanged.
    fn try_revive(&mut self) {
        if !self.game_over || self.revives_left == 0 {
            return;
        }
        self.revives_left -= 1;
        self.player.hp = (self.player.max_hp() * REVIVE_HP_FRAC).max(1.0);
        self.player.iframes = REVIVE_IFRAMES;
        self.player.hurt_flash = 0.0;
        // `outcome_system` clears `level_complete` and `won` on the death tick,
        // so only the loss flag has to come back off.
        self.game_over = false;
        self.sfx.push(Sfx::Heal);
    }

    /// Move the facade singletons into the Bevy world as resources.
    ///
    /// Called by `update`, and exposed for narrow single-system tests:
    /// `begin_tick` → `ecs.run_system_cached(system)` → `end_tick`.
    pub fn begin_tick(&mut self, dt: f32, move_dir: Vec2, dash: bool) {
        // One Space/click press is both the dash and the active-weapon release:
        // `TickInput::active` mirrors `dash` here, and `player_system` /
        // `active_weapons_system` each decide whether *they* can spend it (a
        // dash on cooldown still fires a charged weapon, and vice versa).
        self.begin_tick_with(dt, move_dir, dash, dash);
    }

    /// [`begin_tick`](Self::begin_tick) with the two presses told apart (see
    /// [`update_with`](Self::update_with)).
    pub fn begin_tick_with(&mut self, dt: f32, move_dir: Vec2, dash: bool, active: bool) {
        self.begin_tick_full(dt, move_dir, dash, active, None, 0.0, DashHold::NONE);
    }

    /// [`begin_tick_with`](Self::begin_tick_with) plus the aim point.
    pub fn begin_tick_with_aim(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
    ) {
        self.begin_tick_full(dt, move_dir, dash, active, aim, 0.0, DashHold::NONE);
    }

    /// [`begin_tick_with_aim`](Self::begin_tick_with_aim) plus the dash charge
    /// (single-system tests of the charged Super Dash).
    pub fn begin_tick_charged(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
        charge: f32,
    ) {
        self.begin_tick_full(dt, move_dir, dash, active, aim, charge, DashHold::NONE);
    }

    /// [`begin_tick_charged`](Self::begin_tick_charged) plus the dash button's
    /// live held state and edges (single-system tests of the hold-to-charge
    /// Super Dash: press, hold, release).
    pub fn begin_tick_with_dash_hold(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        hold: DashHold,
    ) {
        self.begin_tick_full(dt, move_dir, dash, active, None, 0.0, hold);
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_tick_full(
        &mut self,
        dt: f32,
        move_dir: Vec2,
        dash: bool,
        active: bool,
        aim: Option<Vec2>,
        charge: f32,
        hold: DashHold,
    ) {
        self.ecs.insert_resource(TickInput {
            dt,
            move_dir,
            dash,
            active,
            aim,
            dash_down: hold.down,
            dash_held: hold.held,
            dash_released: hold.up,
            charge: charge.clamp(0.0, 1.0),
            previous_pos: self.player.pos,
        });
        self.ecs.insert_resource(Viewport(self.viewport));
        self.ecs.insert_resource(NukeTick(false));
        self.ecs.insert_resource(LevelIndex(self.level_index));
        self.ecs.insert_resource(DifficultyLevel(
            self.difficulty_override.unwrap_or(self.level_index),
        ));
        self.ecs.insert_resource(DebugFlags {
            freeze: self.freeze,
            peace: self.peace,
            noclip: self.noclip,
        });
        self.ecs.insert_resource(self.cvars);
        self.ecs.insert_resource(BossOverride(self.boss_override));
        self.ecs.insert_resource(RunCounters {
            elapsed: self.elapsed,
            kills: self.kills,
            score: self.score,
            gold: self.gold,
            pending_chests: self.pending_chests,
            pending_shop: self.pending_shop,
            pending_shop_hole: self.pending_shop_hole,
            pending_levelups: self.pending_levelups,
            game_over: self.game_over,
            won: self.won,
            level_complete: self.level_complete,
            testbed: self.testbed,
        });
        self.ecs.insert_resource(Shake {
            amount: self.shake,
            ofs: self.shake_ofs,
        });
        self.ecs
            .insert_resource(SfxQueue(std::mem::take(&mut self.sfx)));
        self.ecs.insert_resource(NextEnemyId(self.next_enemy_id));
        self.ecs
            .insert_resource(LastPickups(std::mem::take(&mut self.last_pickups)));

        // `StdRng` is a foreign type, so it round-trips through `RngRes`.
        let rng_placeholder = self
            .rng_spare
            .take()
            .expect("rng spare must be initialised in World::new");
        let rng = std::mem::replace(&mut self.rng, rng_placeholder);
        self.ecs.insert_resource(RngRes(rng));

        swap_in(&mut self.ecs, &mut self.level, &mut self.level_spare);
        swap_in(&mut self.ecs, &mut self.player, &mut self.player_spare);
        swap_in(&mut self.ecs, &mut self.particles, &mut self.particles_spare);
        swap_in(
            &mut self.ecs,
            &mut self.damage_numbers,
            &mut self.damage_numbers_spare,
        );
        swap_in(&mut self.ecs, &mut self.director, &mut self.director_spare);
        swap_in(&mut self.ecs, &mut self.flow, &mut self.flow_spare);
        swap_in(&mut self.ecs, &mut self.fog, &mut self.fog_spare);
    }

    /// Move the resources back into the facade and refresh the entity caches.
    pub fn end_tick(&mut self) {
        let rng = self
            .ecs
            .remove_resource::<RngRes>()
            .expect("RngRes must be inserted by begin_tick")
            .0;
        self.rng_spare = Some(std::mem::replace(&mut self.rng, rng));

        swap_out(&mut self.ecs, &mut self.level, &mut self.level_spare);
        swap_out(&mut self.ecs, &mut self.player, &mut self.player_spare);
        swap_out(&mut self.ecs, &mut self.particles, &mut self.particles_spare);
        swap_out(
            &mut self.ecs,
            &mut self.damage_numbers,
            &mut self.damage_numbers_spare,
        );
        swap_out(&mut self.ecs, &mut self.director, &mut self.director_spare);
        swap_out(&mut self.ecs, &mut self.flow, &mut self.flow_spare);
        swap_out(&mut self.ecs, &mut self.fog, &mut self.fog_spare);

        self.next_enemy_id = self
            .ecs
            .remove_resource::<NextEnemyId>()
            .expect("NextEnemyId must be inserted by begin_tick")
            .0;
        self.last_pickups = self
            .ecs
            .remove_resource::<LastPickups>()
            .expect("LastPickups must be inserted by begin_tick")
            .0;

        let counters = self
            .ecs
            .remove_resource::<RunCounters>()
            .expect("RunCounters must be inserted by begin_tick");
        self.elapsed = counters.elapsed;
        self.kills = counters.kills;
        self.score = counters.score;
        self.gold = counters.gold;
        self.pending_chests = counters.pending_chests;
        self.pending_shop = counters.pending_shop;
        self.pending_shop_hole = counters.pending_shop_hole;
        self.pending_levelups = counters.pending_levelups;
        self.game_over = counters.game_over;
        self.won = counters.won;
        self.level_complete = counters.level_complete;
        // The console's boss override is consumed by `boss_spawn_system`; sync
        // its post-tick value back so a one-shot override does not linger.
        self.boss_override = self
            .ecs
            .get_resource::<BossOverride>().and_then(|o| o.0);

        let shake = self
            .ecs
            .remove_resource::<Shake>()
            .expect("Shake must be inserted by begin_tick");
        self.shake = shake.amount;
        self.shake_ofs = shake.ofs;

        let sfx = self
            .ecs
            .remove_resource::<SfxQueue>()
            .expect("SfxQueue must be inserted by begin_tick");
        self.sfx = sfx.0;

        if self.testbed {
            self.pending_levelups = 0;
            if self.level_complete {
                self.level_complete = false;
                self.won = false;
                self.clear_testbed_boss();
            }
        }
    }

    // ---- Spawn helpers (systems use Commands directly) -------------------

    /// Spawn an enemy using the facade rng and assign a `SpawnId`.
    pub fn spawn_enemy(&mut self, kind: EnemyKind, pos: Vec2, hp_mult: f32) -> Entity {
        let mut enemy = Enemy::spawn(kind, pos, hp_mult, &mut self.rng);
        enemy.id = self.next_enemy_id;
        self.next_enemy_id = self.next_enemy_id.wrapping_add(1).max(1);
        spawn_with_id(&mut self.ecs, enemy)
    }

    /// Spawn an enemy drawing `Enemy::spawn`'s random fields from a caller
    /// rng (the legacy tests use their own seeded `StdRng`).
    pub fn spawn_enemy_with_rng(
        &mut self,
        kind: EnemyKind,
        pos: Vec2,
        hp_mult: f32,
        rng: &mut impl Rng,
    ) -> Entity {
        let mut enemy = Enemy::spawn(kind, pos, hp_mult, rng);
        enemy.id = self.next_enemy_id;
        self.next_enemy_id = self.next_enemy_id.wrapping_add(1).max(1);
        spawn_with_id(&mut self.ecs, enemy)
    }

    pub fn spawn_bullet(&mut self, bullet: Bullet) -> Entity {
        spawn_with_id(&mut self.ecs, bullet)
    }

    /// Inject a chest between ticks. Systems use `ChestSpawn` through Commands.
    #[allow(dead_code)]
    pub fn spawn_chest(&mut self, pos: Vec2) -> Entity {
        ChestSpawn { pos }.spawn(&mut self.ecs)
    }

    /// Inject a named miniboss between ticks (tests use this; the real system
    /// spawns through `Commands`).
    pub fn spawn_miniboss(
        &mut self,
        kind: super::ecs::systems::miniboss::MinibossKind,
        appointment: usize,
        pos: Vec2,
    ) -> Entity {
        use super::ecs::systems::miniboss::{appointment_hp_for, Miniboss};
        let mut enemy = Enemy::spawn(EnemyKind::Brute, pos, 1.0, &mut self.rng);
        let difficulty = self.difficulty_override.unwrap_or(self.level_index);
        enemy.hp = appointment_hp_for(difficulty, appointment);
        enemy.max_hp = enemy.hp;
        enemy.radius = kind.body_radius();
        enemy.damage = crate::game::ecs::systems::miniboss::MINIBOSS_CONTACT_DAMAGE;
        enemy.speed_scale = kind.speed_scale();
        enemy.named = true;
        enemy.id = self.next_enemy_id;
        self.next_enemy_id = self.next_enemy_id.wrapping_add(1).max(1);
        let mb = Miniboss::new(kind, appointment);
        spawn_with_id(&mut self.ecs, (enemy, mb))
    }

    pub fn spawn_gem(&mut self, gem: Gem) -> Entity {
        spawn_with_id(&mut self.ecs, gem)
    }

    pub fn spawn_pickup(&mut self, pickup: Pickup) -> Entity {
        spawn_with_id(&mut self.ecs, pickup)
    }

    pub fn spawn_pool(&mut self, pool: Pool) -> Entity {
        spawn_with_id(&mut self.ecs, pool)
    }

    /// Despawn one entity (any kind).
    pub fn despawn_entity(&mut self, entity: Entity) {
        let _ = self.ecs.despawn(entity);
    }

    pub fn clear_enemies(&mut self) {
        self.clear_components::<Enemy>();
    }

    pub fn clear_bullets(&mut self) {
        self.clear_components::<Bullet>();
    }

    pub fn clear_gems(&mut self) {
        self.clear_components::<Gem>();
    }

    pub fn clear_pickups(&mut self) {
        self.clear_components::<Pickup>();
    }

    pub fn clear_pools(&mut self) {
        self.clear_components::<Pool>();
    }

    /// Remove every Hantavirus dropping (tests and a floor reset; a new floor
    /// is a new `World`, so this mirrors [`World::clear_pools`]).
    pub fn clear_droppings(&mut self) {
        self.clear_components::<Dropping>();
    }

    fn clear_components<T: bevy_ecs::component::Component>(&mut self) {
        let mut query = self.ecs.query::<(Entity, &T)>();
        let entities: Vec<Entity> = query.iter(&self.ecs).map(|(entity, _)| entity).collect();
        for entity in entities {
            let _ = self.ecs.despawn(entity);
        }
    }

    /// Set up the practice field in the largest generated hall. Flatbed
    /// halls can contain cargo walls, so use the nearest clear point inside.
    pub(crate) fn configure_testbed(&mut self) {
        self.testbed = true;
        self.peace = true;
        self.player.god = true;
        if let Some(hall) = self.level.largest_hall() {
            let rect = hall.rect;
            let center = rect.center_world();
            let usable = |p| {
                self.level.pit_at(p).is_none()
                    && self.level.tiles.circle_clear(p, PLAYER_RADIUS, SizeClass::Small)
            };
            let start = if usable(center) {
                Some(center)
            } else {
                rect.cells()
                    .map(|(x, y)| TileMap::tile_rect(x, y).center())
                    .filter(|&p| usable(p))
                    .min_by(|a, b| a.dist_sq(center).total_cmp(&b.dist_sq(center)))
            };
            if let Some(pos) = start {
                self.player.pos = pos;
                self.cam = pos - self.render_viewport * 0.5;
                self.flow.update(&self.level.tiles, pos, NavBudget::FULL);
                self.fog.reveal_cell(&self.level, super::collision::cell_of(pos));
                self.ecs.insert_resource(PuddleSteps::at(pos));
            }
        }
    }

    /// Try the requested placement first, then rotate around the rat and
    /// shorten the distance if the generated floor blocks the full ring.
    pub(crate) fn testbed_spawn_point(
        &self,
        direction: Vec2,
        distance: f32,
        radius: f32,
        class: SizeClass,
    ) -> Vec2 {
        for scale in [1.0, 0.75, 0.5, 0.25] {
            for step in 0..16 {
                let turn = (step + 1) / 2 * if step % 2 == 0 { -1 } else { 1 };
                let heading = direction.rotate(turn as f32 * std::f32::consts::TAU / 16.0);
                let pos = self.player.pos + heading * (distance * scale);
                if self.level.pit_at(pos).is_none()
                    && self.level.tiles.circle_clear(pos, radius, class)
                    && (self.level.theme != Theme::Outfall
                        || super::hazards::outfall_sand_clear(&self.level, pos, radius, class))
                {
                    return pos;
                }
            }
        }
        self.player.pos
    }

    /// Remove the boss and its shots and restore the ordinary horde pace.
    pub(crate) fn clear_testbed_boss(&mut self) {
        self.clear_components::<Boss>();
        self.clear_components::<BossProjectile>();
        if self.level.theme == Theme::Outfall {
            self.level.objects.outfall_boss_snuff = None;
        }
        *self.ecs.resource_mut::<BossFight>() = BossFight::default();
        *self.ecs.resource_mut::<BossDue>() = BossDue::default();
        *self.ecs.resource_mut::<BossVirus>() = BossVirus::default();
        self.boss_override = None;
        self.director.boss_spawned = false;
    }

    /// Mark visible bodies for the normal reap and boss defeat paths.
    pub(crate) fn kill_testbed_visible(&mut self) {
        let cam = self.cam;
        let viewport = self.render_viewport;
        let visible = |pos: Vec2, radius| {
            let p = pos - cam;
            on_screen((p.x.round() as i32, p.y.round() as i32), radius, viewport)
        };
        let mut enemies = self.ecs.query::<&mut Enemy>();
        for mut enemy in enemies.iter_mut(&mut self.ecs) {
            if visible(enemy.pos, enemy.radius) {
                enemy.hp = 0.0;
            }
        }
        let mut bosses = self.ecs.query::<&mut Boss>();
        for mut boss in bosses.iter_mut(&mut self.ecs) {
            if visible(boss.pos, boss.kind.body_radius()) {
                boss.hp = 0.0;
            }
        }
    }

    // ---- Console admin helpers (w3-console) ------------------------------
    //
    // Thin wrappers the console's command handlers call. They all run
    // **between ticks** (the facade is authoritative there), so they never
    // touch the ECS mid-tick. The read-only ones only read; the mutating ones
    // are only reached after the console has flagged the run as CONSOLE USED.

    /// Live bullet count for the `where`/`perf` readouts.
    pub fn bullet_count(&self) -> usize {
        self.ecs
            .iter_entities()
            .filter(|e| e.contains::<Bullet>())
            .count()
    }

    /// Boss-arena centre in world pixels, or the map centre as a fallback.
    pub fn boss_arena_center(&self) -> Vec2 {
        self.level
            .boss_room()
            .map(|r| r.rect.center_world())
            .unwrap_or_else(|| self.level.arena * 0.5)
    }

    /// The first shop hole's threshold point, if the floor has one.
    pub fn shop_point(&self) -> Option<Vec2> {
        self.level.objects.shop_holes.first().map(|h| h.face())
    }

    /// A chest's world position, if one is on the floor.
    pub fn chest_point(&mut self) -> Option<Vec2> {
        let mut q = self.ecs.query::<&Chest>();
        q.iter(&self.ecs).next().map(|c| c.pos)
    }

    /// A maze's entry cell in world pixels, if the floor has one.
    pub fn maze_point(&self) -> Option<Vec2> {
        self.level.mazes.first().map(|m| {
            crate::game::collision::TileMap::tile_rect(m.entry.0, m.entry.1).center()
        })
    }

    /// Human-readable name of the room containing `pos`.
    pub fn room_name_at(&self, pos: Vec2) -> &'static str {
        match self.level.room_at(pos).map(|r| r.kind) {
            Some(RoomKind::Start) => "start",
            Some(RoomKind::Boss) => "boss arena",
            Some(RoomKind::Hall) => "hall",
            Some(RoomKind::Room) => "room",
            Some(RoomKind::Corridor) => "corridor",
            Some(RoomKind::Maze) => "maze",
            None => "open floor",
        }
    }

    /// Console `reveal`: mark every cell explored so the whole map shows.
    pub fn reveal_map(&mut self) {
        self.fog.reveal_all(&self.level);
    }

    /// Console `give weapon <name> [level]`: add or level a weapon.
    pub fn give_weapon(&mut self, kind: WeaponKind, level: u8) -> bool {
        if let Some(w) = self.player.loadout.weapons.iter_mut().find(|w| w.kind == kind) {
            w.level = level.max(1);
            return true;
        }
        if self.player.loadout.weapons.len() >= loadout::MAX_WEAPONS {
            return false;
        }
        let mut w = Weapon::new(kind);
        w.level = level.max(1);
        self.player.loadout.weapons.push(w);
        true
    }

    /// Console `give passive <name> [stacks]`: add stacks to a passive.
    pub fn give_passive(&mut self, kind: PassiveKind, stacks: u32) -> bool {
        let cap = kind.max_stacks();
        if let Some((_, held)) = self.player.loadout.passives.iter_mut().find(|(p, _)| *p == kind) {
            *held = (*held + stacks).min(cap);
            self.player.loadout.rebuild_stats();
            return true;
        }
        if self.player.loadout.passives.len() >= loadout::MAX_PASSIVES {
            return false;
        }
        self.player.loadout.passives.push((kind, stacks.min(cap)));
        self.player.loadout.rebuild_stats();
        true
    }

    /// Console `splice <name>`: add a gene splice.
    pub fn give_splice(&mut self, kind: SpliceKind) {
        if !self.player.loadout.splices.contains(&kind) {
            self.player.loadout.splices.push(kind);
            self.player.loadout.rebuild_stats();
        }
    }

    /// Console `evolve <weapon>`: mark one weapon evolved.
    pub fn evolve_weapon(&mut self, kind: WeaponKind) -> bool {
        if let Some(idx) = self.player.loadout.weapons.iter().position(|w| w.kind == kind) {
            self.player.loadout.evolve(idx);
            self.player.loadout.weapons[idx].evolved = true;
            return true;
        }
        false
    }

    /// Console `evolve all`: evolve every eligible weapon. Returns how many.
    pub fn evolve_all(&mut self) -> usize {
        let mut n = 0;
        for i in 0..self.player.loadout.weapons.len() {
            let kind = self.player.loadout.weapons[i].kind;
            if self.evolve_weapon(kind) {
                n += 1;
            }
        }
        n
    }

    /// Console `level <weapon> <n>`: set a weapon's level.
    pub fn set_weapon_level(&mut self, kind: WeaponKind, level: u8) -> bool {
        if let Some(w) = self.player.loadout.weapons.iter_mut().find(|w| w.kind == kind) {
            w.level = level.max(1);
            return true;
        }
        false
    }

    /// Console `remove <weapon|passive>`: drop one from the loadout.
    pub fn remove_weapon(&mut self, kind: WeaponKind) -> bool {
        let before = self.player.loadout.weapons.len();
        self.player.loadout.weapons.retain(|w| w.kind != kind);
        self.player.loadout.weapons.len() != before
    }

    pub fn remove_passive(&mut self, kind: PassiveKind) -> bool {
        let before = self.player.loadout.passives.len();
        self.player.loadout.passives.retain(|(p, _)| *p != kind);
        if self.player.loadout.passives.len() != before {
            self.player.loadout.rebuild_stats();
            return true;
        }
        false
    }

    /// Console `loadout`: one line per owned weapon and passive.
    pub fn loadout_summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        for w in &self.player.loadout.weapons {
            let evo = if w.evolved { " (evolved)" } else { "" };
            out.push(format!("weapon: {} lv{}{}", w.kind.name(), w.level, evo));
        }
        for &(p, stacks) in &self.player.loadout.passives {
            out.push(format!("passive: {} x{}", p.name(), stacks));
        }
        for &s in &self.player.loadout.splices {
            out.push(format!("splice: {}", s.name()));
        }
        out
    }

    /// Console `xp <n>`: bank XP through the normal level-up path.
    pub fn grant_xp(&mut self, amount: u32) {
        self.player.add_xp(amount);
    }

    /// Console `chest [n]`: drop a chest near the rat.
    pub fn spawn_chest_near(&mut self, pos: Vec2, offset: f32) {
        let p = pos + Vec2::new(offset * 18.0, 0.0);
        ChestSpawn { pos: p }.spawn(&mut self.ecs);
    }

    /// Console `pickup <kind> [n]`: drop a pickup near the rat.
    pub fn spawn_pickup_near(&mut self, kind: PickupKind, pos: Vec2, offset: f32) {
        let p = pos + Vec2::new(offset * 18.0, 0.0);
        self.spawn_pickup(Pickup {
            pos: p,
            kind,
            bob: 0.0,
        });
    }

    /// Console `spawn <enemy> [count] [elite]`: spawn a horde body.
    pub fn spawn_enemy_near(&mut self, kind: EnemyKind, pos: Vec2, offset: f32, elite: bool) -> Entity {
        let p = pos + Vec2::new(offset * 22.0, 0.0);
        let e = self.spawn_enemy(kind, p, 1.0);
        if elite {
            if let Some(mut enemy) = self.ecs.get_mut::<Enemy>(e) {
                enemy.elite = true;
            }
        }
        e
    }

    /// Console `miniboss [kind]`: appoint a named miniboss near the rat.
    pub fn spawn_miniboss_near(
        &mut self,
        kind: super::ecs::systems::miniboss::MinibossKind,
        pos: Vec2,
    ) -> Entity {
        self.spawn_miniboss(kind, 0, pos)
    }

    /// Console `boss [kind]`: start the floor's boss fight now.
    pub fn force_boss(&mut self, kind: BossKind) {
        self.boss_override = Some(kind);
        self.ecs.resource_mut::<BossDue>().due = true;
    }

    /// Console `boss <kind> here`: start the fight **where the rat stands**.
    ///
    /// [`force_boss`](Self::force_boss) hands the floor's arena, spawn point
    /// and dock to `boss_spawn_system`; a fight that starts where you stand
    /// needs all three to follow the rat, and the one-shot [`BossOverride`]
    /// carries only a kind. So this spawns the boss straight into the ECS,
    /// between ticks (where the console's handlers run and the facade is
    /// authoritative), keeping the books the spawn system keeps: the boss's own
    /// rng stream, the damage-number id, the horde break, the arrival's shake
    /// and sound, and the director's hand-off flag. `false` when a boss is
    /// already on the floor — the scheduled spawn refuses that too.
    ///
    /// The arena is the floor's own arena rectangle's **size**, recentred on
    /// the rat and clamped inside the map; the body goes into a clear cell
    /// 40-60 px away (never a wall or a pit) and the dock is the nearest
    /// large-class cell to the arena's centre.
    pub fn force_boss_here(&mut self, kind: BossKind) -> bool {
        if self.ecs.iter_entities().any(|e| e.contains::<Boss>()) {
            return false;
        }
        let large = crate::game::collision::SizeClass::Large;
        let arena = self.boss_arena_around(self.player.pos);
        let dock = self
            .level
            .tiles
            .nearest_open_center(arena.center_world(), large, 16)
            .unwrap_or_else(|| arena.center_world());
        let Some(pos) = self.boss_point_beside(self.player.pos, kind.body_radius()) else {
            return false;
        };
        let difficulty = self.difficulty_override.unwrap_or(self.level_index);
        let id = self.next_enemy_id;
        self.next_enemy_id = self.next_enemy_id.wrapping_add(1).max(1);
        // The same books as the scheduled arrival (`boss::open_fight`): HP,
        // facing, a fresh fight on the boss's own rng stream, the horde parted.
        let accent = self.level.palette.accent;
        let boss = {
            let mut fight = self.ecs.resource_mut::<BossFight>();
            let boss = super::ecs::systems::boss::open_fight(
                &mut fight,
                super::ecs::systems::boss::Arrival {
                    kind,
                    pos,
                    arena,
                    dock,
                    rat: self.player.pos,
                    difficulty,
                    id,
                    seed: self.level.seed,
                },
            );
            // The arrival's burst draws from the boss's own stream, exactly as
            // the scheduled spawn's does, so it never perturbs the run rng.
            crate::game::particle::vfx::emit_boss_spawn(&mut self.particles, &mut fight.rng.0, pos, accent);
            boss
        };
        spawn_with_id(&mut self.ecs, boss);
        // This is the level's fight now: the director's hand-off checks this
        // flag, so there is no second arrival at `BOSS_TIME`, and the horde
        // drops to the boss's cleanup pace.
        self.director.boss_spawned = true;
        self.boss_override = None;
        {
            let mut due = self.ecs.resource_mut::<BossDue>();
            due.due = false;
            due.claimed = true;
        }
        self.shake = self.shake.max(14.0);
        self.sfx.push(Sfx::Boss);
        true
    }

    /// The floor's real arena rectangle's size, recentred on `pos` and clamped
    /// inside the map (console `boss <name> here`).
    fn boss_arena_around(&self, pos: Vec2) -> super::level::TileRect {
        // No floor has an arena room any more: the fight rectangle is the
        // nominal one, exactly as the scheduled arrival builds it.
        self.level.fight_rect_around(pos)
    }

    /// A clear cell beside `rat` for a body of `radius`: 40-60 px away, closest
    /// to 48 px first, never a wall cell or a pit.
    ///
    /// The first pass also asks that the body fit (`circle_clear`); a boss is
    /// 27 px across and does not fit a one-cell aisle, so the second pass
    /// settles for a walkable cell and lets `wall_resolve` nudge the body clear
    /// of the wall it clips. Fixed sweep, no rng draw.
    fn boss_point_beside(&self, rat: Vec2, radius: f32) -> Option<Vec2> {
        const STEPS: usize = 16;
        const DISTANCES: [f32; 6] = [48.0, 44.0, 52.0, 40.0, 56.0, 60.0];
        let large = crate::game::collision::SizeClass::Large;
        let outfall = self.level.theme == Theme::Outfall;
        for strict in [true, false] {
            if outfall && !strict {
                break;
            }
            let mut best: Option<(f32, Vec2)> = None;
            for step in 0..STEPS {
                let angle = step as f32 * std::f32::consts::TAU / STEPS as f32;
                let (s, c) = angle.dsin_cos();
                let dir = Vec2::new(c, s);
                for d in DISTANCES {
                    let p = rat + dir * d;
                    if self.level.pit_at(p).is_some()
                        || (outfall
                            && !super::hazards::outfall_sand_clear(&self.level, p, radius, large))
                    {
                        continue;
                    }
                    let (cx, cy) = crate::game::collision::cell_of(p);
                    if !self.level.tiles.passable(cx, cy, large) {
                        continue;
                    }
                    if strict && !self.level.tiles.circle_clear(p, radius, large) {
                        continue;
                    }
                    let key = (d - 48.0).abs();
                    if best.is_none_or(|(bk, _)| key < bk) {
                        best = Some((key, p));
                    }
                }
            }
            if let Some((_, p)) = best {
                return Some(p);
            }
        }
        if outfall {
            let (cx, cy) = super::collision::cell_of(rat);
            return (cy - 16..=cy + 16)
                .flat_map(|y| (cx - 16..=cx + 16).map(move |x| TileMap::tile_rect(x, y).center()))
                .filter(|&p| super::hazards::outfall_sand_clear(&self.level, p, radius, large))
                .min_by(|a, b| a.dist_sq(rat).total_cmp(&b.dist_sq(rat)));
        }
        // Nowhere within reach is walkable: the nearest open cell, so the body
        // can never end up inside a wall.
        Some(self.level.tiles.nearest_open_center(rat, large, 8).unwrap_or(rat))
    }

    /// Console `magnet`: pull every gem toward the rat.
    pub fn magnetize_all_gems(&mut self) -> usize {
        let mut q = self.ecs.query::<&mut Gem>();
        let mut n = 0;
        for mut gem in q.iter_mut(&mut self.ecs) {
            gem.magnetized = true;
            n += 1;
        }
        n
    }

    /// Console `nuke`: reap every live enemy. Returns how many.
    pub fn nuke_all(&mut self) -> usize {
        let n = self.ecs.iter_entities().filter(|entity| entity.contains::<Enemy>()).count();
        self.clear_enemies();
        n
    }

    // ---- Legacy helper hooks used by the ported tests --------------------

    /// Legacy `World::spawn_point`, exposed for the spawn-ring test. Uses the
    /// facade rng/level/player/viewport while no tick is in flight.
    pub fn spawn_point(&mut self) -> Vec2 {
        super::ecs::systems::spawn::spawn_point(
            &mut self.rng,
            &self.level,
            &self.player,
            self.viewport,
        )
    }

    /// Nearest live enemy (`dist_sq` `total_cmp`, first on ties) in `SpawnId`
    /// order, as a legacy-style `(entity, pos)` pair.
    ///
    /// Delegates to the shared `weapons::nearest_of` so the facade and the
    /// weapon system cannot drift apart.
    pub fn nearest_enemy(&mut self, from: Vec2) -> Option<(Entity, Vec2)> {
        let mut query = self.ecs.query::<(Entity, &SpawnId, &Enemy)>();
        let candidates = query
            .iter(&self.ecs)
            .map(|(entity, id, enemy)| (entity, *id, enemy.pos, enemy.targetable()));
        super::ecs::systems::weapons::nearest_of(candidates, from)
    }

    // ---- Rendering ------------------------------------------------------

    /// Render one frame into `pb` (the screen's half-block pixel grid).
    ///
    /// **Zoom** is render-only and lives entirely here: the scene is drawn for
    /// the rectangle `screen × zoom` around the camera — at world resolution
    /// into a scratch buffer when zooming out — and resampled onto the screen
    /// grid (box filter when zooming out, nearest neighbour when zooming in,
    /// and an exact copy at 1:1). Damage numbers are glyph-sized, so they are
    /// stamped in screen space after the resample and never scale.
    ///
    /// **Drawing never writes [`World::viewport`].** The simulation's logical
    /// viewport is session input (the App sets it on a terminal resize, a
    /// headless caller sets it once), so two same-seed runs cannot diverge
    /// because one of them drew a resized framebuffer and the other did not.
    pub fn draw(&mut self, pb: &mut PixelBuffer) {
        let _prof = crate::prof::scope(crate::prof::Stage::Draw);
        let (sw, sh) = (pb.w, pb.h);
        if sw == 0 || sh == 0 {
            // A degenerate terminal renders nothing (and must not panic).
            self.render_viewport = Vec2::new(sw as f32, sh as f32);
            self.zoom_scale = Vec2::new(1.0, 1.0);
            self.detail_live = false;
            return;
        }
        self.detail_live = false;
        let z = zoom::clamp(self.zoom_view);
        self.zoom_scale = Vec2::new(1.0, 1.0);
        if (z - 1.0).abs() < 0.002 {
            // Unzoomed: the screen buffer *is* the world buffer.
            self.draw_scene(pb);
        } else {
            let ww = ((sw as f32) * z).round().max(1.0) as usize;
            let wh = ((sh as f32) * z).round().max(1.0) as usize;
            let mut buf = match self.world_buf.take() {
                Some(b) => b,
                None => PixelBuffer::new(ww, wh),
            };
            buf.resize(ww, wh);
            self.draw_scene(&mut buf);
            {
                let _p = crate::prof::scope(crate::prof::Stage::Resample);
                buf.resample_to(pb);
                // Sub-cell detail: the same world picture again on the finer
                // grid (the smoothing upscale below covers 1x and zoomed in).
                // (the scene and the lighting pass already ran at world
                // resolution, so this is one more resample and nothing else).
                // The half-block picture above is still drawn in full: the
                // screen-space passes below stamp it, and what they change is
                // carried onto the detail frame afterwards.
                if sh.is_multiple_of(2) {
                    if let Some(grid) = crate::render::subcell::grid_for(self.subcell, z) {
                        buf.resample_to(self.detail.begin(grid, sw, sh / 2));
                        self.detail_base.copy_from(pb);
                        self.detail_live = true;
                    }
                }
            }
            self.zoom_scale = Vec2::new(ww as f32 / sw as f32, wh as f32 / sh as f32);
            self.world_buf = Some(buf);
        }
        // At 1x (and zoomed in, where a cell covers less than two world pixels)
        // `pb` *is* the screen picture, so the sub-cell frame is an EPX upscale
        // of it rather than a second resample: the Scale2x corner rule rounds
        // staircases and curves without inventing a colour, and its guards (see
        // `subcell::smooth_into`) leave a flat floor, a thin feature, an ink
        // mark and a lit gradient alone. With `smooth` off there is no detail
        // frame at these zooms at all, so the frame is the half-block blit every
        // earlier build drew, byte for byte.
        if !self.detail_live && sw > 0 && sh.is_multiple_of(2) {
            if let Some(grid) =
                crate::render::subcell::grid_for_frame(self.subcell, z, self.smooth)
            {
                use crate::render::subcell::Smooth;
                let filled = match self.smooth {
                    Smooth::Strong | Smooth::Wedge => {
                        // Antialiasing reads the picture at world resolution:
                        // `pb` itself when unzoomed, else the world buffer this
                        // draw just resampled it from (zoomed in, an edge then
                        // sits at its true place inside a cell). `Wedge` adds
                        // the wedge glyphs and the diagonal-stroke rule.
                        let src = match self.world_buf.as_ref() {
                            Some(buf) if (z - 1.0).abs() >= 0.002 => buf,
                            _ => &*pb,
                        };
                        let wedges = self.smooth == Smooth::Wedge;
                        self.detail.antialias_with(self.subcell, wedges, sw, sh / 2, src, pb)
                    }
                    // The blended and crisp xBR fits read
                    // the screen picture itself at every zoom; `grid` is the
                    // grid it fits to.
                    Smooth::Xbr | Smooth::XbrCrisp => {
                        let crisp = self.smooth == Smooth::XbrCrisp;
                        self.detail.xbr_from(grid, crisp, sw, sh / 2, pb)
                    }
                    Smooth::Epx | Smooth::Off => self.detail.smooth_from(grid, sw, sh / 2, pb),
                };
                if filled {
                    self.detail_base.copy_from(pb);
                    self.detail_live = true;
                }
            }
        }
        // The renderer's own viewport, unzoomed: the simulation's `viewport`
        // is not touched here (see the doc comment above).
        self.render_viewport = Vec2::new(sw as f32, sh as f32);

        // Screen-space pass: floating damage numbers are text-sized glyphs, so
        // they keep their 3×5 / 4×6 pixel size at every zoom instead of
        // blurring into mush when zoomed out.
        let _pd = crate::prof::scope(crate::prof::Stage::DamageNumbers);
        self.damage_numbers
            .draw_scaled_clear_of(
                pb,
                self.cam,
                self.zoom_scale,
                // Ordinary hit digits are suppressed inside the rat's own
                // footprint (`DAMAGE_CLEAR_RADIUS`): the irreversible decision
                // is where to step next, so the pellet beside the rat outranks
                // the number explaining the last hit. Kills and big ticks still
                // print (see `DamageNumbers::draw_scaled_clear_of`).
                Some((self.player.pos, DAMAGE_CLEAR_RADIUS)),
            );

        drop(_pd);
        let _pi = crate::prof::scope(crate::prof::Stage::Ink);
        // Screen-space ink: hostile marks, the rat and the loot language keep a
        // minimum on-screen size at every zoom, and are stamped after the
        // resample so nothing averages them away. Drawn last (over the damage
        // digits) because a pellet that kills the player outranks the number
        // that explains the last hit (see [`crate::render::ink`]).
        self.draw_ink(pb);
        if self.detail_live {
            self.detail.overlay(&self.detail_base, pb);
        }
    }

    /// The sub-cell picture of the frame [`World::draw`] just drew, when it
    /// produced one: zoomed out on a terminal known to draw block glyphs, and
    /// at 1× (or zoomed in) with `smooth` on, where it is the EPX upscale of
    /// the half-block picture. `None` otherwise — the half-block buffer then is
    /// the whole picture, and it is what every earlier build drew.
    pub fn detail(&self) -> Option<&crate::render::subcell::DetailFrame> {
        self.detail_live.then_some(&self.detail)
    }

    /// The last pass of the frame: [`crate::render::ink::draw`] with the view
    /// this draw used. Render-only; no rng, no simulation state.
    fn draw_ink(&mut self, pb: &mut PixelBuffer) {
        let mut view = crate::render::ink::InkView {
            ecs: &mut self.ecs,
            level: &self.level,
            player: &self.player,
            cam: self.cam,
            scale: self.zoom_scale,
            size: (pb.w as i32, pb.h as i32),
            elapsed: self.elapsed,
            // A plate with no rat (`World::hide_rat`) must not show the ink
            // pass's rat core or dash arc either.
            hide_rat: self.hide_rat,
        };
        crate::render::ink::draw(pb, &mut view);
    }

    /// Ease the rendered zoom toward the setting (about
    /// [`zoom::EASE_SECS`]) and snap it onto a ladder rung. Render-only:
    /// called once per frame by the app, never from the simulation schedule.
    pub fn tick_zoom(&mut self, dt: f32) {
        self.zoom = zoom::snap(self.zoom);
        // Growth Serum (batch 13): while the giant find runs the view pulls back
        // to the widest rung and eases home again, as a **render-only** target
        // layered over the player's own choice (the wheel and `Z`/`X` keep
        // moving `self.zoom`; only what `zoom_view` chases is overridden). The
        // simulation reads neither value.
        let target = self.zoom_view_target();
        self.zoom_view = zoom::ease(self.zoom_view, target, dt.max(0.0));
    }

    /// What `zoom_view` is easing toward this frame: the player's rung, or the
    /// Growth Serum's pulled-back view while the find runs. Render-only.
    pub fn zoom_view_target(&self) -> f32 {
        zoom::giant_target(
            self.zoom,
            self.player.buffs.giant,
            crate::game::ecs::systems::powerups::GROWTH_SECS,
        )
    }

    /// Step the zoom one rung (`dir > 0` widens the view). Returns the setting.
    pub fn zoom_by(&mut self, dir: isize) -> f32 {
        self.zoom = zoom::step(self.zoom, dir);
        self.zoom
    }

    /// Set the zoom with no transition (a level starting at the saved zoom).
    pub fn set_zoom_instant(&mut self, zoom: f32) {
        self.zoom = zoom::snap(zoom);
        self.zoom_view = self.zoom;
    }

    /// The world rectangle the renderer covered last frame: the drawn frame's
    /// size ([`World::render_viewport`]) times the eased zoom. Screen-space overlays frame *this* (the
    /// boss compass, the minimap's view brackets), while the simulation keeps
    /// using the unzoomed [`World::viewport`].
    pub fn view_span(&self) -> Vec2 {
        // The renderer's own viewport (the frame it last drew), not the
        // simulation's: the two are the same size in the App, but a spectate
        // tile draws a smaller frame than the run's logical viewport.
        self.render_viewport * self.zoom_view.clamp(zoom::MIN, zoom::MAX)
    }

    /// Draw the world itself into `pb` (whose size *is* the world rectangle it
    /// must cover), setting [`World::cam`] and [`World::render_viewport`] for it.
    ///
    /// Render-only: the bob pulses and glow falloffs below stay on `std`'s
    /// maths library on purpose (see `src/detmath.rs`), so here and only here
    /// the `disallowed_methods` lint is silenced.
    #[allow(clippy::disallowed_methods)]
    fn draw_scene(&mut self, pb: &mut PixelBuffer) {
        let vw = pb.w as f32;
        let vh = pb.h as f32;
        let viewport = Vec2::new(vw, vh);
        self.render_viewport = viewport;
        let arena = self.level.arena;

        // Camera follows the player, clamped to arena (centered if smaller).
        let mut cam = self.cam_focus.unwrap_or(self.player.pos) - Vec2::new(vw * 0.5, vh * 0.5);
        cam.x = if arena.x <= vw {
            (arena.x - vw) * 0.5
        } else {
            cam.x.clamp(0.0, arena.x - vw)
        };
        cam.y = if arena.y <= vh {
            (arena.y - vh) * 0.5
        } else {
            cam.y.clamp(0.0, arena.y - vh)
        };
        cam += self.shake_ofs * self.shake_scale;
        self.cam = cam;
        self.light_map.begin_frame();
        let mut _s = crate::prof::scope(crate::prof::Stage::SceneFloor);

        // Batch-13 material bytes (`render::lighting::material`): kept only
        // when the effects run at a tier that reads them. Every buffer writer
        // stamps the current pen; the sprite passes below set it around their
        // draw calls. `clear` resets every byte to the untagged floor.
        let fx_tier = self.light_map.quality(self.lighting);
        pb.set_pen(0);
        pb.set_materials(
            self.lighting_fx
                && !self.flat_lit
                && matches!(fx_tier, LightingMode::Full | LightingMode::Hard),
        );
        pb.clear(self.level.palette.bg);

        // Floor and arena border.
        art_floor::draw_floor(pb, &self.level, cam, self.elapsed);

        // Purely decorative floor marks (`art_marks`'s own layer order): the
        // boss arena's repeating motif, one landmark per major hall, and the
        // gnaw-mark / gold-fleck trail of each maze approach.
        self.draw_floor_marks(pb, cam);

        // Floor stains (ichor splatters & explosion scorches) beneath props/entities.
        self.particles.draw_stains(pb, cam);

        // Standing water: the puddles, then the ripples the feet left in them.
        // Right after the stains and before the boss ground layer, so a
        // telegraph is drawn over a puddle and a puddle over a stain. Render-only;
        // every pixel is tagged with the wet-floor material so the lighting pass
        // reflects a lamp in it.
        self.draw_puddles(pb, cam, viewport);
        if self.level.theme == Theme::Outfall {
            art_floor::draw_outfall_hazards(pb, &self.level, cam, self.elapsed);
        }

        // The overhead light's ground shadows (batch 13, `render::lighting::
        // overhead`): on the floor, before the walls, props and bodies, so
        // they cover it and the wall layers clip it. Render-only.
        if self.lighting_fx && !self.flat_lit {
            let scene = crate::render::lighting::OverheadScene {
                ecs: &self.ecs,
                level: &self.level,
                player: &self.player,
                cam,
                elapsed: self.elapsed,
                mode: fx_tier,
                hide_rat: self.hide_rat,
            };
            crate::render::lighting::draw_overhead_shadows(pb, &scene);
        }

        // Boss ground layer: floor telegraphs and drop shadows, after the
        // stains and before props/entities (the art's "Ground" layer).
        self.draw_boss_ground(pb, cam);
        // Wall ground layer: contact shadows and floor thresholds, under
        // everything that stands on the floor.
        self.draw_wall_layer(pb, cam, WallLayer::Ground);
        // The boss arena's doorway landmark: chip/scuff/chevron decals that
        // bleed onto the floor on both sides of the mouth (`art_walls` asks for
        // this right after the wall ground layer).
        self.draw_boss_thresholds(pb, cam, WallLayer::Ground);
        self.draw_object_layer(pb, cam, false);

        _s = crate::prof::scope(crate::prof::Stage::SceneWallsProps);
        // Wall bodies: caps and south faces, under the entities (the lintels
        // that go *over* the rat are drawn after them).
        self.draw_wall_layer(pb, cam, WallLayer::Body);

        // Props (themed obstacles): skip any rect that cannot touch the view.
        for p in &self.level.props {
            let x = (p.rect.x - cam.x).round() as i32;
            let y = (p.rect.y - cam.y).round() as i32;
            let margin = p.rect.w.max(p.rect.h) + 2.0;
            if !on_screen((x, y), margin, viewport) {
                continue;
            }
            let pen = pb.set_pen(crate::render::lighting::prop_pen(p.kind));
            art_props::draw_prop(pb, p, cam, &self.level.palette);
            pb.set_pen(pen);
            if matches!(p.kind, PropKind::Console | PropKind::Tank) {
                let (mx, my) = self.w2s(p.rect.center(), cam);
                self.light_map.mark_emissive_rect(pb, mx - 2, my - 3, 5, 2);
            }
        }

        _s = crate::prof::scope(crate::prof::Stage::SceneFixturesAmbience);
        let ambience_started = std::time::Instant::now();
        let blackout = self.blackout();
        draw_fixtures(
            &mut self.light_map,
            pb,
            &self.level,
            cam,
            self.elapsed,
            blackout,
        );
        draw_ambience(&mut self.light_map, pb, &self.level.objects.emitters, cam, self.elapsed, self.player.pos);
        let ambience_cost = ambience_started.elapsed();

        _s = crate::prof::scope(crate::prof::Stage::SceneGroundLoot);
        // Acid pools, in `SpawnId` order (overlapping sprites must draw in
        // spawn order, exactly like the legacy `Vec`).
        {
            let mut query = self.ecs.query::<(&SpawnId, &Pool)>();
            for (sid, p) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(p.pos, cam);
                if !on_screen(s, p.radius + 2.0, viewport) {
                    continue;
                }
                let pen = pb.set_pen(crate::render::lighting::pool_pen());
                art_wave3a::draw_acid_pool(pb, p, s, self.elapsed, sid.0);
                pb.set_pen(pen);
                let ri = (p.radius.round() as i32).clamp(3, 24);
                self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                for &(ox, oy) in &[(ri, 0), (-ri, 0), (0, ri), (0, -ri)] {
                    self.light_map.mark_emissive_rect(pb, s.0 + ox, s.1 + oy, 1, 1);
                }
            }
        }

        // Hantavirus droppings: the weapon's whole delivery, on the floor where
        // the rat left it. Right after the pools and before the evolved ground
        // effects (a dropping is floor litter, not an effect), in `SpawnId`
        // order so overlapping sprites stack in spawn order, under the default
        // (untagged floor) lighting pen. The sprite itself is
        // `art_droppings::draw_dropping`'s; the order and the cull are here.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Dropping)>();
            for (_, d) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(d.pos, cam);
                if !on_screen(s, d.radius + 4.0, viewport) {
                    continue;
                }
                let pen = pb.set_pen(0);
                super::art_droppings::draw_dropping(pb, d, cam, self.elapsed);
                pb.set_pen(pen);
                super::art_droppings::mark_dropping_emissive(&mut self.light_map, pb, d, cam);
            }
        }

        // Wave-3 evolved ground effects (Drain Cleaner channels & Fairy Rings),
        // under entities so bodies & tells read cleanly on top.
        {
            let mut query = self.ecs.query::<(&SpawnId, &CorrosiveChannel)>();
            for (_, ch) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let mid = self.w2s(ch.centre(), cam);
                if on_screen(mid, ch.length * 0.5 + 8.0, viewport) {
                    art_wave3a::draw_corrosive_channel(pb, ch, cam, self.elapsed);
                    self.light_map.mark_emissive_rect(pb, mid.0 - 1, mid.1 - 1, 2, 2);
                }
            }
        }
        {
            let mut query = self.ecs.query::<(&SpawnId, &FairyRing)>();
            for (_, ring) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(ring.pos, cam);
                if on_screen(s, ring.radius + 8.0, viewport) {
                    art_wave3a::draw_fairy_ring(pb, ring, cam, self.elapsed);
                    let ri = ring.radius.round() as i32;
                    for &(ox, oy) in &[(ri, 0), (-ri, 0), (0, ri), (0, -ri)] {
                        self.light_map.mark_emissive_rect(pb, s.0 + ox, s.1 + oy - 1, 1, 1);
                    }
                }
            }
        }

        // Acid Rain's locked drops, over the pools and under everything that
        // stands on the floor: thin vertical blue-green streaks falling onto
        // compact shadow dots with small ripple rings (never a big pool).
        {
            let mut query = self.ecs.query::<(&SpawnId, &AcidDrop)>();
            for (_, d) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(d.pos, cam);
                if !on_screen(s, d.radius + 22.0, viewport) {
                    continue;
                }
                art_wave3a::draw_acid_drop(pb, d, s);
                self.light_map.mark_emissive_rect(pb, s.0 - 1, s.1 - 1, 2, 2);
            }
        }

        // Molten pools sit under gems and bodies. Only their hot pixels are
        // emissive, so the cooling crust still takes the room's lighting.
        {
            let mut query = self.ecs.query::<(&SpawnId, &LavaPool)>();
            for (_, pool) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(pool.pos, cam);
                if on_screen(s, pool.radius + 3.0, viewport) {
                    let pen = pb.set_pen(crate::render::lighting::pool_pen());
                    super::art_lava::draw_pool(pb, pool, cam, self.elapsed);
                    pb.set_pen(pen);
                    self.light_map.mark_emissive_highlights(pb, s, pool.radius.ceil() as i32);
                }
            }
        }

        // Gems.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Gem)>();
            for (sid, g) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(g.pos, cam);
                let radius = GemTier::for_value(g.value).radius_px();
                if !on_screen(s, radius as f32, viewport) {
                    continue;
                }
                art_gems::draw_gem(pb, s, g, self.elapsed, sid.0 as u32);
                let size = 2 * radius + 1;
                self.light_map
                    .mark_emissive_rect(pb, s.0 - radius, s.1 - radius, size, size);
            }
        }

        // Coins and chests sit above gems and below the pickup halos.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Gold)>();
            for (sid, gold) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(gold.pos, cam);
                if on_screen(s, 5.0, viewport) {
                    art_floor::draw_gold_coin(pb, s, gold.value, self.elapsed, sid.0 as f32 * 1.37);
                }
            }
        }
        {
            let mut query = self.ecs.query::<(&SpawnId, &Chest)>();
            for (_, chest) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(chest.pos, cam);
                if on_screen(s, 12.0, viewport) {
                    art_floor::draw_chest(pb, s, self.elapsed);
                    self.light_map.mark_emissive_rect(pb, s.0 - 1, s.1 - 1, 3, 2);
                }
            }
        }

        // Pickups with a pulsing additive glow halo.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Pickup)>();
            for (_, p) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(p.pos, cam);
                let pulse = 0.5 + 0.5 * (p.bob.sin() * 0.5 + 0.5); // 0.5..1.0
                let base = p.kind.color();
                let is_pocket = matches!(
                    p.kind,
                    PickupKind::BottleCap
                        | PickupKind::WindupMouse
                        | PickupKind::PocketWatch
                        | PickupKind::CleanSocks
                        | PickupKind::GrowthSerum
                        | PickupKind::ShrinkDraught
                        | PickupKind::WhiteCoat
                        | PickupKind::TheCollar
                );
                let gr = (if is_pocket { 6.0 } else { 7.0 } + pulse * 3.5) as i32;
                let grf = gr as f32;
                if !on_screen(s, grf + 2.0, viewport) {
                    continue;
                }
                let halo_gain = if is_pocket { 0.36 } else { 0.70 };
                for dy in -gr..=gr {
                    for dx in -gr..=gr {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        if d <= grf {
                            let fall = (1.0 - d / grf).powi(2);
                            pb.plot_add(
                                s.0 + dx,
                                s.1 + dy,
                                palette::scale(base, fall * pulse * halo_gain),
                            );
                        }
                    }
                }
                art_floor::draw_pickup(pb, p.kind, s, p.bob);
            }
        }

        // The Wind-up Mouse's brass key (`systems::powerups::Decoy`): a floor
        // object under the entities, drawn after the pickups it came from.
        {
            let mut query = self.ecs.query::<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>();
            for (_, d) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(d.pos, cam);
                if !on_screen(s, 8.0, viewport) {
                    continue;
                }
                art_floor::draw_decoy(pb, s, self.elapsed * 3.0);
            }
        }

        // Enemies (animated insects). Bugs overhang their radius (legs, wings,
        // health ring), so cull with a generous margin.
        _s = crate::prof::scope(crate::prof::Stage::SceneEnemies);
        let accent = self.level.palette.accent;
        {
            let mut query = self.ecs.query::<(
                &SpawnId,
                &Enemy,
                Option<&super::ecs::systems::miniboss::Miniboss>,
                // The guest's own state: the Drake is drawn through
                // `art_duck`, from the same view its peck's hit test asks.
                Option<&Drake>,
            )>();
            // Count what is on screen first: past CROWD_LOD_THRESHOLD the art
            // switches small enemies to its cheaper level-of-detail drawing.
            let visible = query
                .iter(&self.ecs)
                .filter(|(_, e, _, _)| {
                    on_screen(self.w2s(e.pos, cam), e.radius * 3.0 + 8.0, viewport)
                })
                .count();
            for (_, e, mb, drake) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if e.buried {
                    continue;
                }
                let s = self.w2s(e.pos, cam);
                if !on_screen(s, e.radius * 3.0 + 8.0, viewport) {
                    continue;
                }
                let points = [(s.0, s.1), (s.0 + 1, s.1), (s.0 - 1, s.1), (s.0, s.1 - 1)];
                let before = points.map(|(x, y)| pb.pixel_at(x as usize, y as usize));
                let pen = pb.set_pen(crate::render::lighting::enemy_pen(e.kind, mb.map(|m| m.kind)));
                match mb {
                    Some(mb) => art_miniboss::draw(pb, e, mb, cam, accent),
                    // A Drake is always drawn in full (a body this size is
                    // never level-of-detail'd), and always through the one
                    // view function `drake::drake_view` — a body spawned
                    // between ticks has no state component yet, so it stands
                    // in its Waddle pose for the one frame before
                    // `drake_system` attaches it.
                    None if drake.is_some() || e.kind == EnemyKind::Drake => {
                        let idle = Drake::default();
                        let view = super::ecs::systems::drake::drake_view(
                            e,
                            drake.unwrap_or(&idle),
                        );
                        super::art_duck::draw_duck(pb, &view, cam);
                    }
                    None => art_enemy::draw_enemy_crowd(pb, e, cam, accent, visible),
                }
                pb.set_pen(pen);
                for ((x, y), floor) in points.into_iter().zip(before) {
                    let body = pb.pixel_at(x as usize, y as usize);
                    if body != floor {
                        self.light_map.mark_readable(pb, x, y);
                    }
                }
            }
        }

        _s = crate::prof::scope(crate::prof::Stage::SceneActors);
        // Boss bodies in the entity pass (airborne ones go over the player).
        self.draw_boss_body(pb, cam, false);

        // Friendly bullets (the rat's own). Hostile pellets are *not* drawn
        // here: they are the thing that can kill the player, so they belong to
        // the hostile layer at the very end of the frame (see
        // [`Self::draw_hostile_layer`]) — above overhangs, smoke, friendly
        // sparks and the lighting pass.
        {
            let spore_evolved = self
                .player
                .loadout
                .weapons
                .iter()
                .any(|w| w.kind == WeaponKind::SporeOrbit && w.evolved);
            let ppos = self.player.pos;
            let mut query = self.ecs.query::<(&SpawnId, &Bullet, Option<&EchoSpoke>)>();
            // Visible dotted orbit track drawn once when Spore Orbit is active.
            let orbit_radius = query
                .iter(&self.ecs)
                .find_map(|(_, b, _)| (b.faction == Faction::Player).then_some(b.orbit).flatten())
                .map(|o| o.radius);
            if let Some(r) = orbit_radius {
                art_wave3a::draw_spore_orbit_track(pb, ppos, r, self.elapsed, spore_evolved, cam);
            }
            for (sid, b, echo) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if b.faction == Faction::Enemy {
                    continue;
                }
                let s = self.w2s(b.pos, cam);
                if !on_screen(s, 22.0, viewport) {
                    continue;
                }
                if let Some(spoke) = echo {
                    art_wave3a::draw_echo_spoke(pb, s, b.vel, spoke.reflected);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                } else if b.weapon == Some(WeaponKind::TailThrash) {
                    art_wave3a::draw_flywheel_dart(pb, s, b.vel);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                } else if b.orbit.is_some() {
                    if spore_evolved {
                        art_wave3a::draw_evolved_spore_orbiter(pb, s, self.elapsed);
                    } else {
                        art_wave3a::draw_spore_orbiter(pb, s, self.elapsed);
                    }
                    self.light_map.mark_emissive_rect(pb, s.0 - 1, s.1 - 1, 2, 2);
                } else if let Some(lob) = b.lob {
                    // Single large green blob on a parabolic arc above its travelling ground shadow.
                    art_wave3a::draw_acid_glob(pb, b.pos, lob.target, ppos, b.vel, b.color, cam);
                    let dist_left = b.pos.dist(lob.target);
                    let span = ppos.dist(lob.target).max(dist_left).max(16.0);
                    let u = (1.0 - dist_left / span).clamp(0.0, 1.0);
                    let lift = (4.0 * u * (1.0 - u) * (span * 0.22).clamp(6.0, 18.0)).round() as i32;
                    self.light_map.mark_emissive_rect(pb, s.0 - 1, s.1 - lift - 1, 2, 2);
                } else if b.weapon == Some(WeaponKind::CheeseSpray) {
                    // Fat tumbling warm yellow-orange triangular wedges.
                    let spin = (sid.0 as f32 * 1.73) + self.elapsed * 7.5;
                    art_wave3a::draw_cheese_wedge(pb, s, b.vel, spin, b.life);
                    self.light_map.mark_emissive_rect(pb, s.0 - 1, s.1 - 1, 2, 2);
                } else if b.weapon == Some(WeaponKind::SpikedTail) {
                    // Razor steel-grey broken-off spike shards (L4+).
                    art_wave3a::draw_spike_shard(pb, s, b.vel);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                } else if b.size >= 2 {
                    pb.filled_circle(s.0, s.1, 1, b.color);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                } else {
                    pb.plot_add(s.0, s.1, b.color);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                }
            }
        }

        // Lava globs fly over the friendly bullet layer. Their own shadows
        // stay on the ground, outside the lifted body's emissive mark.
        {
            let mut query = self.ecs.query::<(&SpawnId, &LavaGlob)>();
            for (_, glob) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let ground = self.w2s(glob.ground, cam);
                let body = self.w2s(glob.ground - Vec2::new(0.0, glob.height()), cam);
                if on_screen(body, 7.0, viewport) || on_screen(ground, 7.0, viewport) {
                    super::art_lava::draw_glob(pb, glob, cam, self.elapsed);
                    self.light_map.mark_emissive_highlights(pb, body, 6);
                }
            }
        }

        {
            use super::ecs::systems::weapons_wave5::{GlassBead, PlumbBob};
            let mut beads = self.ecs.query::<(&SpawnId, &GlassBead)>();
            for (_, bead) in beads.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(bead.pos, cam);
                if bead.delay <= 0.0 && on_screen(s, 5.0, viewport) {
                    super::art_wave5::draw_bead(pb, bead.pos, bead.vel, cam, bead.bounces);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                }
            }
            let mut bobs = self.ecs.query::<(&SpawnId, &PlumbBob)>();
            for (_, bob) in bobs.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if on_screen(self.w2s(bob.pivot, cam), 96.0, viewport) {
                    super::art_wave5::draw_bob(pb, bob.pivot, bob.pos, cam, bob.evolved);
                    let s = self.w2s(bob.pos, cam);
                    self.light_map.mark_emissive_highlights(pb, s, 3);
                }
            }
        }

        // Fondue Fork outer tines in flight (before both plant their CheeseString).
        {
            let mut query = self.ecs.query::<(&SpawnId, &FondueTine)>();
            for (_, tine) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(tine.pos, cam);
                if on_screen(s, 6.0, viewport) {
                    art_wave3a::draw_fondue_tine(pb, s, tine.vel);
                    self.light_map.mark_emissive_rect(pb, s.0, s.1, 1, 1);
                }
            }
        }

        // Fondue Fork strings & Milk Teeth darts (drawn over enemies so the
        // yellow posts, sagging cheese line, ivory hook & dotted thread stay crisp).
        {
            let mut query = self.ecs.query::<(&SpawnId, &CheeseString)>();
            for (_, line) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let mid = self.w2s((line.a + line.b) * 0.5, cam);
                let margin = line.a.dist(line.b) * 0.5 + 10.0;
                if on_screen(mid, margin, viewport) {
                    art_wave3a::draw_cheese_string(pb, line, cam, self.elapsed);
                    self.light_map.mark_emissive_rect(pb, mid.0, mid.1, 1, 1);
                }
            }
        }
        {
            let ppos = self.player.pos;
            let mut query = self.ecs.query::<(&SpawnId, &MilkTooth)>();
            for (_, dart) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(dart.pos, cam);
                let ps = self.w2s(ppos, cam);
                if on_screen(s, 10.0, viewport) || on_screen(ps, 10.0, viewport) {
                    art_wave3a::draw_milk_tooth(pb, dart, ppos, cam);
                    self.light_map.mark_emissive_rect(pb, s.0 - 1, s.1 - 1, 2, 2);
                }
            }
        }

        // Player rat.
        self.draw_player(pb, cam);

        // Instant melee weapon visuals anchored to the rat: Gnaw's closing white
        // tooth crescents (`BiteSnap`) and Tail Whip's frontal pink half-circle
        // crescent swipe (`WhipCrack`). Both are spawned without `SpawnId`
        // (decorations), so query the components directly.
        {
            let mut query = self.ecs.query::<&BiteSnap>();
            for snap in query.iter(&self.ecs) {
                let s = self.w2s(snap.pos + snap.dir * 4.5, cam);
                if on_screen(s, 12.0, viewport) {
                    art_rat::draw_gnaw_bite(pb, snap, cam);
                    self.light_map.mark_emissive_highlights(pb, s, 6);
                }
            }
        }
        {
            let mut query = self.ecs.query::<&WhipCrack>();
            for crack in query.iter(&self.ecs) {
                let sf = self.w2s(crack.from, cam);
                let st = self.w2s(crack.to, cam);
                let span = crack.to - crack.from;
                let reach = span.len();
                if on_screen(sf, reach + 8.0, viewport) {
                    art_rat::draw_tail_whip_crack(pb, crack, cam);
                    self.light_map.mark_emissive_highlights(pb, st, 4);
                    if reach > 1.0 {
                        let dir = span * (1.0 / reach);
                        for side in [-0.75f32, 0.75] {
                            let flank = self.w2s(crack.from + dir.rotate(side) * reach, cam);
                            self.light_map.mark_emissive_highlights(pb, flank, 3);
                        }
                    }
                }
            }
        }

        // The cat's airborne pounce passes visibly overhead.
        self.draw_boss_body(pb, cam, true);

        // Overhang layer: lintels over rat-holes, doorways and one-way doors,
        // so the rat is visibly ducking *under* them; then the foreground half
        // of the level objects (glue strands, striker bars, door leaves, the
        // rat-hole arch and the reward pedestal).
        _s = crate::prof::scope(crate::prof::Stage::SceneOverhang);
        self.draw_wall_layer(pb, cam, WallLayer::Overhang);
        self.draw_boss_thresholds(pb, cam, WallLayer::Overhang);
        self.draw_object_layer(pb, cam, true);

        _s = crate::prof::scope(crate::prof::Stage::SceneSmoke);
        // Brief smoke from combat and moving machinery, under bright sparks.
        if self.smoke_enabled {
            self.draw_smoke(pb, cam);
        }

        _s = crate::prof::scope(crate::prof::Stage::SceneParticles);
        // Particles and shockwaves on top.
        self.particles.draw(pb, cam);

        // Squeak Nova (concentric pale-cyan expanding rings) & Sonic Squeak
        // (directional forward wedge of nested pure-white chevrons). Drawn here
        // before lighting, with tight emissive apex/rim marks so they cut cleanly
        // through the 60%-ambient floor without repainting wide areas.
        {
            let fx = *self.ecs.resource::<ActiveFx>();
            if fx.nova_active() {
                let s = self.w2s(fx.nova_pos, cam);
                if on_screen(s, fx.nova_max_r + 6.0, viewport) {
                    art_wave3a::draw_squeak_nova_ring(pb, &fx, cam);
                    let ri = fx.nova_r.round() as i32;
                    if ri >= 2 {
                        for &(ox, oy) in &[(ri, 0), (-ri, 0), (0, ri), (0, -ri)] {
                            self.light_map.mark_emissive_rect(pb, s.0 + ox, s.1 + oy, 1, 1);
                        }
                    }
                }
            }
            if fx.squeak_active() {
                let s = self.w2s(fx.squeak_pos, cam);
                if on_screen(s, fx.squeak_max_r + 6.0, viewport) {
                    art_wave3a::draw_sonic_cone(pb, &fx, cam);
                    let dir = if fx.squeak_facing.len_sq() > 1e-4 {
                        fx.squeak_facing.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    let tip = self.w2s(fx.squeak_pos + dir * fx.squeak_r, cam);
                    self.light_map.mark_emissive_highlights(pb, tip, 5);
                }
            }
        }

        // Super Dash blue-white kinetic comb over the swept corridor (above
        // enemy hit-flashes so the comb teeth visibly cut through crossed bodies).
        if self.player.dash_active > 0.0 {
            if let Some(w) = self
                .player
                .loadout
                .weapons
                .iter()
                .find(|w| w.kind == WeaponKind::SuperDash)
            {
                let track = self.ecs.resource::<DashTrack>();
                let dir = if let Some((from, to)) = track.segment {
                    let seg = to - from;
                    if seg.len_sq() > 1e-4 {
                        seg.normalized()
                    } else {
                        self.player.face
                    }
                } else if self.player.vel.len_sq() > 1.0 {
                    self.player.vel.normalized()
                } else {
                    self.player.face
                };
                let step_len = track
                    .segment
                    .map_or(5.0, |(a, b)| a.dist(b))
                    .clamp(3.0, 8.0);
                let comb_len = (track.legs.max(1) as f32 * step_len).clamp(10.0, 52.0);
                let dash_frac = (self.player.dash_active / super::ecs::systems::player::DASH_SECS)
                    .clamp(0.0, 1.0);
                art_wave3a::draw_super_dash_comb(
                    pb,
                    self.player.pos,
                    dir,
                    comb_len,
                    w.dash_radius(),
                    dash_frac,
                    cam,
                );
            }
        }

        // Roll Call (evolved Sonic Squeak): cyan quotation marks winking shut in
        // sequence over every tagged body until the 0.8 s answer pulses fire.
        {
            let fx = *self.ecs.resource::<ActiveFx>();
            if fx.answer_left > 0.0 && fx.tag_n > 0 {
                let progress = 1.0
                    - (fx.answer_left / crate::game::ecs::systems::evolutions::ROLL_CALL_DELAY)
                        .clamp(0.0, 1.0);
                for (slot, maybe_entity) in fx.tags.iter().enumerate().take(fx.tag_n as usize) {
                    let Some(entity) = *maybe_entity else {
                        continue;
                    };
                    let Some(e) = self.ecs.get::<Enemy>(entity) else {
                        continue;
                    };
                    if !e.alive() {
                        continue;
                    }
                    let s = self.w2s(e.pos, cam);
                    if on_screen(s, e.radius + 10.0, viewport) {
                        art_wave3a::draw_roll_call_quote(pb, s, e.radius, progress, slot);
                    }
                }
            }
        }

        // Per-frame 2D lighting pass over the *world-resolution* pixels, with
        // the camera the layers above were drawn with — so it runs here, before
        // `draw` resamples the scene onto the screen, and lights line up with
        // their sprites at every zoom. Floating damage numbers are text sized
        // and belong to screen space: `draw` stamps them after the resample, so
        // they never scale and are never darkened by the ambient.
        _s = crate::prof::scope(crate::prof::Stage::Lighting);
        self.apply_lighting(pb, cam, ambience_cost, blackout);

        // The beam cores are self-lit. Draw them after the water stage so
        // painting a beam cannot erase the water material under its reflection.
        if !self.hide_rat {
            if let Some(beam) = self.laser_visual() {
                art_rat::draw_laser_eyes(pb, beam, cam);
            }
        }
        let laser = self.ecs.resource::<LaserFx>();
        if laser.copy_active() {
            if let Some(copy) = laser.copy {
                let angles = [laser.copy_prev_angle, laser.copy_prev_angle_b];
                for (angle, len) in angles.into_iter().zip(laser.copy_beams) {
                    art_rat::draw_laser_bar(pb, copy.origin, angle, len, laser.copy_brightness(), cam);
                }
            }
        }

        // Parallax foreground layer: overhead things nearer the eye than the
        // floor, drawn at world resolution (so it works at every zoom) after
        // the lighting pass — a fixed darkening instead of a per-pixel light —
        // and **before** the hostile layer below, so a pellet, a telegraph or a
        // tell can never be hidden by it. See `render::foreground`.
        if self.foreground {
            let mut scene = crate::render::foreground::ForeScene {
                ecs: &mut self.ecs,
                level: &self.level,
                cam,
                zoom: match zoom::clamp(self.zoom_view) {
                    z if (z - 1.0).abs() < 0.002 => 1.0,
                    z => z,
                },
                rat: self.player.pos,
                elapsed: self.elapsed,
            };
            crate::render::foreground::draw(pb, &mut self.foreground_layer, &mut scene);
        }

        // Hostile information wins the frame: pellets, telegraphs, tells and
        // status pips are drawn **after** the lighting pass (so they are
        // self-lit) and after every friendly effect (so a burst can never hide
        // the thing that is about to hurt the player). Screen-space minimum
        // sizes for the zoomed-out view live in `render::ink`.
        _s = crate::prof::scope(crate::prof::Stage::Hostile);
        self.draw_hostile_layer(pb, cam);
        drop(_s);

        // Developer overlay, dead last: the console's `hitboxes on` outlines the
        // collision shapes the simulation actually uses. Nothing may hide it.
        if self.hitboxes {
            self.draw_hitboxes(pb, cam);
        }
    }

    /// Render-only debug overlay (console `hitboxes on`): outline the collision
    /// shapes the simulation actually uses, in screen space so they stay
    /// legible at every zoom. Drawn last, over the hostile layer, because it is
    /// a developer tool: nothing may hide it. Never touches the simulation.
    fn draw_hitboxes(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let viewport = self.viewport;
        const PLAYER: (u8, u8, u8) = (90, 255, 220);
        const ENEMY: (u8, u8, u8) = (255, 90, 200);
        const HOSTILE: (u8, u8, u8) = (255, 120, 60);
        const BOSS: (u8, u8, u8) = (255, 220, 60);

        // The body circle, plus the smaller hurt disc while shrunk.
        {
            let s = self.w2s(self.player.pos, cam);
            pb.ring(s.0, s.1, PLAYER_RADIUS.round() as i32, PLAYER);
            if self.player.buffs.tiny > 0.0 {
                pb.ring(s.0, s.1, self.player.buffs.hurt_radius().round() as i32, HOSTILE);
            }
        }

        // Enemies and minibosses.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Enemy)>();
            for (_, e) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(e.pos, cam);
                if !on_screen(s, e.radius + 3.0, viewport) {
                    continue;
                }
                pb.ring(s.0, s.1, e.radius.round().max(1.0) as i32, ENEMY);
            }
        }

        // Bullets, both factions.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Bullet)>();
            for (_, b) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(b.pos, cam);
                if !on_screen(s, b.radius + 2.0, viewport) {
                    continue;
                }
                pb.ring(s.0, s.1, b.radius.round().max(1.0) as i32, HOSTILE);
            }
        }

        // Boss hit shapes: the art's own footprint, so a `boss <kind>` spawn can
        // be checked against what the weapons actually hit.
        {
            let mut query = self.ecs.query::<&Boss>();
            for boss in query.iter(&self.ecs) {
                for shape in boss.hit_shapes().iter() {
                    match *shape {
                        BossHitShape::Circle { center, radius, .. } => {
                            let s = self.w2s(center, cam);
                            if on_screen(s, radius + 3.0, viewport) {
                                pb.ring(s.0, s.1, radius.round().max(1.0) as i32, BOSS);
                            }
                        }
                        BossHitShape::Box {
                            center,
                            facing,
                            half,
                            half_perp,
                            ..
                        } => {
                            // Four corners of the oriented box, joined.
                            let u = facing * half;
                            let v = facing.perp() * half_perp;
                            let pts = [
                                self.w2s(center + u + v, cam),
                                self.w2s(center + u - v, cam),
                                self.w2s(center - u - v, cam),
                                self.w2s(center - u + v, cam),
                            ];
                            for i in 0..4 {
                                let a = pts[i];
                                let b = pts[(i + 1) % 4];
                                pb.line(a.0, a.1, b.0, b.1, BOSS);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Everything that can hurt the player and every countdown attached to it,
    /// composited last and unlit (see `WAVE3 review`, "Reading a crowded
    /// floor"). Split out of [`Self::draw_scene`] so the layer order is a
    /// single call site: floor/loot/actors first, lighting, then this.
    fn draw_hostile_layer(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let viewport = self.render_viewport;

        let hold = self.ecs.resource::<super::ecs::resources::HoldPointFx>();
        if !self.player.buffs.is_giant()
            && on_screen(self.w2s(hold.centre, cam), 32.0, viewport)
        {
            if let Some((charge, fired, cracked)) = hold.visual() {
                super::art_wave5::draw_hold(pb, hold.centre, cam, charge, fired, cracked);
            }
        }

        if let Some(opening) = &self.level.objects.sewer_opening {
            let shaft = &opening.drip_shaft;
            if on_screen(self.w2s(shaft.pos, cam), 24.0, viewport) {
                art_floor::draw_drip_shaft(pb, shaft, cam);
            }
        }

        if self.level.theme == Theme::Outfall {
            art_floor::draw_outfall_tells(
                pb, &self.level, self.player.pos, cam, self.elapsed, self.outfall_surge_enabled(),
            );
        }

        // Wave 3: a dead Wick Moth's burning fuse. Its 9 px burst is a ground
        // hazard the rat steps out of, so the ring is a warning, not decor.
        {
            let mut query = self.ecs.query::<(&SpawnId, &crate::game::ecs::components::Wick)>();
            for (_, wick) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(wick.pos, cam);
                if !on_screen(s, 12.0, viewport) {
                    continue;
                }
                art_enemy::draw_wick(pb, wick, cam);
            }
        }

        // A charge's rails are visible from the wind-up through the lunge.
        {
            use super::ecs::systems::miniboss::{Miniboss, MinibossShape};
            let mut query = self.ecs.query::<(&SpawnId, &Enemy, &Miniboss)>();
            for (_, e, mb) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let Some((MinibossShape::Trench { a, b, half }, _, live)) =
                    mb.manoeuvre.charge_lane(e.pos, e.radius)
                else {
                    continue;
                };
                if e.alive() && on_screen(self.w2s((a + b) * 0.5, cam), a.dist(b) * 0.5 + half, viewport) {
                    draw_hostile_lane(pb, self.w2s(a, cam), self.w2s(b, cam), half, live);
                }
            }
        }

        // Enemy bullets: the same sprite as before, now over the smoke and the
        // ambient. Pocket Watch's crawling pellet keeps its dark chrono tail so
        // "the pellets are slow" is legible without reading the numbers.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Bullet)>();
            for (_, b) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if b.faction != Faction::Enemy {
                    continue;
                }
                let s = self.w2s(b.pos, cam);
                // Pocket Watch (`systems::powerups`): a slowed hostile pellet
                // drags a short dark chrono tail (design §10) drawn behind the head.
                if self.player.buffs.slow_bullets > 0.0 && b.vel.len_sq() > 0.1
                {
                    let dir = b.vel.normalized();
                    let perp = dir.perp();
                    for &(dist, dark_col, rim_a) in &[
                        (1.5f32, (28, 36, 62), 0.35f32),
                        (2.8, (22, 28, 52), 0.22),
                        (4.2, (18, 22, 42), 0.10),
                    ] {
                        let tp = self.w2s(b.pos - dir * dist, cam);
                        pb.plot(tp.0, tp.1, dark_col);
                        let r1 = self.w2s(b.pos - dir * dist + perp * 0.9, cam);
                        if r1 != tp {
                            pb.plot_add(r1.0, r1.1, palette::scale((110, 174, 218), rim_a));
                        }
                    }
                }
                if b.outfall_bubble {
                    art_roster_outfall::draw_bubble_pellet(pb, b, cam);
                } else if b.lob.is_some() {
                    pb.filled_circle(s.0, s.1, 1, palette::scale(b.color, 1.2));
                    let trail = self.w2s(b.prev_pos, cam);
                    pb.plot_add(trail.0, trail.1, palette::scale(b.color, 0.5));
                } else if b.size >= 2 {
                    pb.filled_circle(s.0, s.1, 1, b.color);
                } else {
                    pb.plot_add(s.0, s.1, b.color);
                }
            }
        }

        // Wave-3 body-anchored tells (the Bell Flea's landing crosses, the
        // Turnkey's crossbar): the art draws them under the body in the entity
        // pass; they are re-stamped here so the ambient and a friendly burst
        // cannot dim the mark the rat has to step off.
        {
            let accent = self.level.palette.accent;
            let mut query = self.ecs.query::<(&SpawnId, &Enemy)>();
            for (_, e) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                // One gate for every body-anchored floor tell (`floor_tell_reach`):
                // the two wave-3 archetypes exactly as before, plus each
                // floor's own kinds with the reach their tell needs. Wave-4
                // bodies (the Loose Can among them) are stamped by the block
                // below.
                if !e.alive() || e.kind.is_wave4() {
                    continue;
                }
                let Some(reach) = floor_tell_reach(e.kind, e.radius) else {
                    continue;
                };
                if !on_screen(self.w2s(e.pos, cam), reach, viewport) {
                    continue;
                }
                draw_floor_tell(pb, e, cam, accent);
            }
        }

        // The Sewer guest: the Drake's peck lane, for the whole tell and every
        // peck. The geometry is `drake::peck_lane` — the same line the hit test
        // and the bot read — drawn here rather than in `draw_floor_tell`
        // because the lane needs the body's `Drake` state and that function
        // only ever gets an `&Enemy`.
        {
            use super::ecs::components::DrakeState;
            let mut query = self
                .ecs
                .query::<(&SpawnId, &Enemy, &super::ecs::components::Drake)>();
            for (_, e, d) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if !e.alive() || !matches!(d.state, DrakeState::Tell | DrakeState::Peck) {
                    continue;
                }
                let (a, b) = super::ecs::systems::drake::peck_lane(e.pos, d.aim);
                let mid = (a + b) * 0.5;
                let margin = a.dist(b) * 0.5 + super::ecs::systems::drake::PECK_LANE_WIDTH;
                if !on_screen(self.w2s(mid, cam), margin, viewport) {
                    continue;
                }
                draw_hostile_lane(
                    pb, self.w2s(a, cam), self.w2s(b, cam),
                    super::ecs::systems::drake::PECK_LANE_WIDTH * 0.5,
                    d.state == DrakeState::Peck,
                );
            }
        }

        // Printworks: the Ink Blot's marked endpoint, shown for the whole fuse
        // of every wick that carries an `InkSlide` (the burst travels there).
        {
            use crate::game::ecs::systems::enemy_wave3::{WICK_BURST_RADIUS, WICK_FUSE_SECS};
            use crate::game::ecs::systems::floor_printworks::InkSlide;
            let mut query =
                self.ecs.query::<(&SpawnId, &crate::game::ecs::components::Wick, &InkSlide)>();
            for (_, wick, slide) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if !on_screen(self.w2s(slide.end, cam), WICK_BURST_RADIUS + 4.0, viewport) {
                    continue;
                }
                crate::game::art_roster_printworks::draw_ink_endpoint(
                    pb,
                    slide.end,
                    cam,
                    wick.fuse / WICK_FUSE_SECS,
                );
            }
        }

        // Wave-4 tells: the snake's lunge lane, the centipede's flutter line,
        // the hornet's dive line / grounded punish ring and the horde's split
        // cross. Same reason as the wave-3 block above: none of these may be
        // dimmed by the ambient or covered by a friendly burst.
        {
            let accent = self.level.palette.accent;
            let mut query = self.ecs.query::<(&SpawnId, &Enemy)>();
            for (_, e) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                if !e.alive() || !e.kind.is_wave4() {
                    continue;
                }
                // The old margin, widened only for a kind whose own floor tell
                // reaches further (the Loose Can's marked roll).
                let margin = (e.radius * 6.0 + 48.0)
                    .max(floor_tell_reach(e.kind, e.radius).unwrap_or(0.0));
                if !on_screen(self.w2s(e.pos, cam), margin, viewport) {
                    continue;
                }
                art_enemy::draw_wave4_tells(pb, e, cam, accent);
            }
        }

        // The Magpie's falling crate (the thing that lands on the warned
        // square): culled on the bird *and* on the square it falls to.
        {
            let mut query = self.ecs.query::<&Boss>();
            for boss in query.iter(&self.ecs) {
                if let BossArtView::Magpie(v) = boss.view() {
                    let margin = boss.kind.cull_margin();
                    if on_screen(self.w2s(boss.pos, cam), margin, viewport)
                        || on_screen(self.w2s(boss.target, cam), margin, viewport)
                    {
                        art_boss_magpie::draw_magpie_crate(pb, &v, cam);
                    }
                }
            }
        }

        // The horseshoe crab's live tell (the tail arc, the charge lane, the
        // marked burrow, the thrash ring): re-stamped over the lighting, like
        // the wave-3 ground tells, because the Sewer's ambient must never dim
        // the shape the rat has to step out of.
        {
            let mut query = self.ecs.query::<&Boss>();
            for boss in query.iter(&self.ecs) {
                if let BossArtView::HorseshoeCrab(v) = boss.view() {
                    let margin = boss.kind.cull_margin();
                    if on_screen(self.w2s(boss.pos, cam), margin, viewport)
                        || on_screen(self.w2s(boss.target, cam), margin + 40.0, viewport)
                    {
                        art_boss_horseshoe_crab::draw_tell_overlay(pb, &v, cam);
                    }
                }
            }
        }

        // The Redtail's live marks (the locked talon reticle, the rake lane, the
        // nape of a bird with its feet in the planks): re-stamped over the
        // lighting for the same reason as the crab's tells — the deck's ambient
        // must never dim the shape the rat has to leave.
        {
            let mut query = self.ecs.query::<&Boss>();
            for boss in query.iter(&self.ecs) {
                if let BossArtView::Hawk(v) = boss.view() {
                    let margin = boss.kind.cull_margin();
                    if on_screen(self.w2s(boss.pos, cam), margin, viewport)
                        || on_screen(self.w2s(boss.target, cam), margin + 40.0, viewport)
                    {
                        art_boss_hawk::draw_tell_overlay(pb, &v, cam);
                    }
                }
            }
        }

        // The King Cobra's laid body: the coil and the spiral lane are *walls*,
        // so their outline is re-stamped over the lighting (the crab's live
        // tell gets the same treatment for the same reason). The body itself
        // stays in the entity pass; this is only the shape the rat has to
        // respect, plus the doorway out of it.
        {
            let mut query = self.ecs.query::<&Boss>();
            for boss in query.iter(&self.ecs) {
                match boss.view() {
                    BossArtView::Cobra(v) => {
                        let margin = boss.kind.cull_margin();
                        if on_screen(self.w2s(boss.pos, cam), margin, viewport)
                            || on_screen(self.w2s(boss.target, cam), margin, viewport)
                        {
                            art_boss_cobra::draw_coil_tell(pb, &v, cam);
                        }
                    }
                    BossArtView::Lantern(v) => {
                        art_boss_lantern::draw_tell_overlay(pb, &v, cam, self.level.palette.accent);
                    }
                    _ => {}
                }
            }
        }

        // Boss-owned projectiles (blade discs, shrapnel).
        self.draw_boss_projectiles(pb, cam);

        // Boss **floor telegraphs**, outlined from their own shape data: every
        // hazard a live boss has queued gets a dashed warn outline in the boss's
        // warning colour, and a solid brighter one while it is active. Drawn
        // here (after the lighting pass) so the most dangerous marks on the
        // floor are self-lit; the bosses' own ground art stays underneath.
        {
            let mut query = self.ecs.query::<&Boss>();
            for boss in query.iter(&self.ecs) {
                if boss.dead || boss.hazards.is_empty() {
                    continue;
                }
                if !on_screen(self.w2s(boss.pos, cam), boss.kind.cull_margin(), viewport)
                    && !boss.hazards.iter().any(|h| {
                        !h.is_expired()
                            && on_screen(self.w2s(h.shape.center(), cam), 64.0, viewport)
                    })
                {
                    continue;
                }
                crate::render::hazard_outline::draw_ground(pb, boss, cam, self.elapsed);
            }
        }

        // Named-miniboss telegraphs: the tell under the body, over the floor.
        self.draw_miniboss_ground(pb, cam);

        // Hantavirus pips: a status pip the player is counting down must never
        // be covered by a particle burst.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Enemy, &Infection)>();
            for (_, e, inf) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(e.pos, cam);
                if !on_screen(s, e.radius * 3.0 + 10.0, viewport) {
                    continue;
                }
                art_wave3a::draw_infection_pips(pb, s, e.radius, inf.ticks_left);
            }
        }
    }

    /// Render-only smoke pass; never mutates simulation state or `self.rng`.
    fn draw_smoke(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let thrash = self.thrash_visual();
        let scene = SmokeScene {
            ecs: &self.ecs,
            level: &self.level,
            player: &self.player,
            particles: &self.particles,
            thrash,
            cam,
            elapsed: self.elapsed,
            mode: self.light_map.quality(self.lighting),
        };
        self.smoke.sync_and_draw(pb, &scene);
    }

    /// Render-only lighting pass; never mutates simulation state or `self.rng`.
    fn apply_lighting(
        &mut self,
        pb: &mut PixelBuffer,
        cam: Vec2,
        ambience_cost: std::time::Duration,
        blackout: f32,
    ) {
        #[cfg(test)]
        if self.lighting == LightingMode::Off {
            return;
        }
        if self.flat_lit || crate::prof::lighting_skipped() {
            return;
        }
        let thrash = self.thrash_visual();
        let squeak = self.squeak_visual();
        let scene = LightScene {
            ecs: &self.ecs,
            level: &self.level,
            player: &self.player,
            particles: &self.particles,
            thrash,
            squeak,
            cam,
            elapsed: self.elapsed,
            blackout,
            mode: self.light_map.quality(self.lighting),
            depth: self.lighting_depth,
            fx: self.lighting_fx,
        };
        let started = std::time::Instant::now();
        self.light_map.apply_scene(pb, &scene);
        // Tests never feed the wall clock to the tier controller: a loaded
        // test machine would drop a tier in one harness and not its twin, and
        // two "identical" frames would differ (the controller's own tests call
        // `observe_cost` with synthetic durations).
        if cfg!(not(test)) {
            self.light_map
                .observe_cost(scene.mode, started.elapsed() + ambience_cost, pb.w * pb.h);
        }
    }

    /// The tier the next frame's lighting will run at: the setting, lowered
    /// by the cost controller's current penalty.
    pub(crate) fn lighting_tier(&self) -> LightingMode {
        self.light_map.quality(self.lighting)
    }

    /// `--prof-bench`: the tier the last frame's lighting ran at.
    pub(crate) fn prof_tier(&self) -> LightingMode {
        self.lighting_tier()
    }

    /// `--prof-bench`: forget the cost controller's state between configs.
    pub(crate) fn prof_reset_controller(&mut self) {
        self.light_map.prof_reset_controller();
    }

    /// Access the render-only smoke layer (tests and screenshot captures).
    pub fn smoke_mut(&mut self) -> &mut SmokeLayer {
        &mut self.smoke
    }

    fn draw_player(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        // A card plate with no rat (`World::hide_rat`): the simulation still
        // has one, the picture does not.
        if self.hide_rat {
            return;
        }
        // Super Dash charge: the lane preview under the rat. Faint blue-white
        // (the weapon's own colour, never a telegraph red), at the length this
        // charge would carry, stopped by the first wall.
        if self.player.dash_charging {
            art_rat::draw_dash_preview(pb, &self.player, &self.level.tiles, cam);
        }
        // The Tail Thrash's long tail rides *under* the rat, so the body stays
        // readable mid-sweep and the arc reads as coming out of its rump.
        if let Some((angle, reach, fade)) = self.thrash_visual() {
            art_rat::draw_tail_thrash(pb, &self.player, Some((angle, reach, fade)), cam);
            let fx = *self.ecs.resource::<ActiveFx>();
            if fx.thrash_evolved {
                art_wave3a::draw_flywheel_pips(
                    pb,
                    self.player.pos,
                    angle,
                    reach,
                    fade,
                    fx.caught,
                    cam,
                );
            }
        }
        // The tail is the Tail Thrash's charge meter (see `art_rat::TailCharge`):
        // the charge itself is read off the weapon, but the one-off "just
        // charged" flash needs the crossing remembered between frames, and this
        // is the only place that has a frame stream.
        let charge = art_rat::tail_charge(&self.player);
        // The Spiked Tail's lash rides on that same tail via a render-only
        // Verlet physics chain (`self.whip_chain`), stepped deterministically
        // against the simulation's `Lash` clock so the tip lands squarely on
        // the target at the exact crack tick.
        let live_lash: Option<(u64, crate::game::ecs::components::Lash, Option<Vec2>)> = {
            use crate::game::ecs::components::{Lash, LashTarget};
            let mut q = self.ecs.query::<(&SpawnId, &Lash)>();
            let mut best: Option<(u64, Lash)> = None;
            for (id, lash) in q.iter(&self.ecs) {
                if best.is_none_or(|(b, _)| id.0 < b) {
                    best = Some((id.0, *lash));
                }
            }
            best.map(|(id, lash)| {
                let catch_pos = if lash.barbed_wire() {
                    match lash.primary {
                        Some(LashTarget::Enemy(e)) => self
                            .ecs
                            .get::<Enemy>(e)
                            .filter(|enemy| enemy.alive())
                            .map(|enemy| enemy.pos),
                        _ => None,
                    }
                } else {
                    None
                };
                (id, lash, catch_pos)
            })
        };
        self.whip_chain.sync(
            &self.player,
            self.elapsed,
            live_lash.as_ref().map(|(id, l, c)| (*id, l, *c)),
        );
        let lash = self.lash_visual();
        let flash = art_rat::tail_ready_flash(
            charge,
            self.elapsed,
            &mut self.tail_ready_at,
        );
        // The giant is lit as a body five times the size (a slow dome), not as
        // the small rat's.
        let scale = giant_draw_scale(self.player.buffs.giant)
            .or_else(|| tiny_draw_scale(self.player.buffs.tiny));
        let pen = pb.set_pen(if scale.is_some_and(|s| s > 1.5) {
            crate::render::lighting::giant_rat_pen()
        } else {
            crate::render::lighting::rat_pen()
        });
        match scale {
            Some(scale) => self.draw_scaled_rat(pb, cam, scale, flash),
            None => art_rat::draw_rat_with_whip(
                pb,
                &self.player,
                self.elapsed,
                cam,
                flash,
                lash,
                Some(&self.whip_chain),
            ),
        }
        if self.player.buffs.collar > 0.0 {
            let face = self.player.face.normalized();
            let neck = self.player.pos + face * (2.0 * scale.unwrap_or(1.0));
            let (x, y) = self.w2s(neck, cam);
            let glint = if (self.elapsed * 6.0) as i32 % 3 == 0 {
                (255, 240, 180)
            } else {
                PickupKind::TheCollar.color()
            };
            pb.plot(x, y, glint);
        }
        pb.set_pen(pen);
        if self.whip_chain.is_active() {
            let tip_s = self.w2s(self.whip_chain.tip(), cam);
            self.light_map.mark_emissive_highlights(pb, tip_s, 5);
            let mid_s = self.w2s(self.whip_chain.mid(), cam);
            self.light_map.mark_emissive_highlights(pb, mid_s, 3);
        }
        if charge.is_some_and(|charge| charge.ready) {
            self.light_map.mark_emissive_highlights(pb, self.w2s(self.player.pos, cam), 15);
        }
        // Bottle Cap active charge: crisp rotating white-tin rim around the rat.
        if self.player.buffs.shield > 0.0 {
            art_wave3a::draw_bottle_cap_rim(pb, self.player.pos, self.elapsed, cam);
        }
        // The muzzle dots sit on the rat and stay lit through the sweep.
        if let Some(beam) = self.laser_visual() {
            art_rat::draw_laser_muzzle(pb, &self.player, beam.muzzle, cam);
            self.light_map.mark_emissive_highlights(pb, self.w2s(self.player.pos, cam), 9);
        }
        if let Some(weapon) = self.player.loadout.weapons.iter().find(|w| w.kind == WeaponKind::LavaBreath) {
            use super::ecs::systems::weapon_lava::LavaMuzzle;
            let mut query = self.ecs.query::<(&SpawnId, &LavaGlob, &LavaMuzzle)>();
            let newest = query.iter(&self.ecs)
                .filter(|(_, glob, _)| glob.t < LAVA_MUZZLE_SECS)
                .max_by_key(|(id, _, _)| id.0);
            if let Some((_, _, muzzle)) = newest {
                let t = (muzzle.cycle - weapon.timer).max(0.0);
                if t < LAVA_MUZZLE_SECS {
                    let mouth = self.player.pos + muzzle.dir * (LAVA_MOUTH_OFFSET * scale.unwrap_or(1.0));
                    super::art_lava::draw_breath(pb, mouth, muzzle.dir, cam, t);
                    self.light_map.mark_emissive_highlights(pb, self.w2s(mouth, cam), 8);
                }
            }
        }
    }

    /// Draw the rat procedurally at the current size buff's scale.
    ///
    /// Calls [`art_rat_giant::draw_giant_rat`] directly into the scene buffer so
    /// the giant rat renders at full detail rather than as a nearest-neighbour
    /// upscale of the 1× sprite. The long weapon effects (Laser Eyes' beam, the
    /// Spiked Tail's live whip and its impact star, the Tail Thrash arc and the
    /// dash preview) keep their ordinary size at their own call sites, from the
    /// same rat centre: a weapon effect is not the body.
    ///
    /// Render-only: it reads the player, the frame clock and the render-only
    /// whip chain, and writes pixels.
    fn draw_scaled_rat(&self, pb: &mut PixelBuffer, cam: Vec2, scale: f32, tail_flash: f32) {
        art_rat_giant::draw_giant_rat(pb, &self.player, self.elapsed, cam, scale, tail_flash);
        // The live lash at its ordinary size, over the giant.
        if self.whip_chain.is_active() {
            art_rat::draw_live_whip(
                pb,
                &self.player,
                self.elapsed,
                cam,
                tail_flash,
                &self.whip_chain,
            );
        }
    }

    /// Boss ground layer: floor telegraphs and drop shadows, drawn after the
    /// floor stains and before props/entities (the art's "Ground" layer).
    ///
    /// The ground layer carries the *warnings* (the cat's landing ring, the
    /// stomp's shadow, the mower's charge rails), so it is culled on the boss
    /// **and** on its telegraph target: a jaguar that has leapt off screen must
    /// not take the landing ring the rat is standing in with it. The ring can
    /// sit a whole leap (`POUNCE_MAX_DIST = 96 px`) away, further than the
    /// art's 85 px sprite margin, so the target needs its own test.
    fn draw_boss_ground(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let accent = self.level.palette.accent;
        let mut query = self.ecs.query::<&Boss>();
        for boss in query.iter(&self.ecs) {
            let margin = boss.kind.cull_margin();
            let margin_vp = self.render_viewport;
            let body_visible = on_screen(self.w2s(boss.pos, cam), margin, margin_vp);
            let target_visible = on_screen(self.w2s(boss.target, cam), margin, margin_vp);
            if !body_visible && !target_visible {
                continue;
            }
            match boss.view() {
                BossArtView::Cat(v) => art_boss_cat::draw_boss_cat_ground(pb, &v, cam, accent),
                BossArtView::Broom(v) => art_boss_broom::draw_ground(pb, &v, cam, accent),
                BossArtView::Mower(v) => {
                    art_boss_mower::draw_dock_pad(pb, v.dock_pos, v.anim, 0.0, cam, accent);
                    art_boss_mower::draw_mower_ground(pb, &v, cam, accent);
                }
                BossArtView::Mangle(v) => art_boss_mangle::draw_ground(pb, &v, cam, accent),
                BossArtView::NightEdition(v) => {
                    art_boss_night_edition::draw_ground(pb, &v, cam, accent)
                }
                BossArtView::Magpie(v) => {
                    art_boss_magpie::draw_magpie_ground(pb, &v, cam, accent);
                }
                BossArtView::HorseshoeCrab(v) => {
                    art_boss_horseshoe_crab::draw_ground(pb, &v, cam, accent)
                }
                BossArtView::MegaRat(v) => {
                    art_boss_megarat::draw_megarat_ground(pb, &v, cam, accent);
                }
                BossArtView::Hawk(v) => {
                    art_boss_hawk::draw_hawk_ground(pb, &v, cam, accent);
                }
                BossArtView::Cobra(v) => {
                    art_boss_cobra::draw_ground(pb, &v, cam, accent);
                }
                BossArtView::Lantern(v) => art_boss_lantern::draw_ground(pb, &v, cam, accent),
            }
        }
    }

    /// Named-miniboss ground layer: the tell/active telegraph under the bodies.
    fn draw_miniboss_ground(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let accent = self.level.palette.accent;
        let mut query = self
            .ecs
            .query::<(&Enemy, &super::ecs::systems::miniboss::Miniboss)>();
        for (e, mb) in query.iter(&self.ecs) {
            if !e.alive() {
                continue;
            }
            if !on_screen(self.w2s(e.pos, cam), 90.0, self.render_viewport)
                && !mb.telegraph().is_some_and(|shape| shape.signature_bounds().is_some())
            {
                continue;
            }
            art_miniboss::draw_telegraph(pb, mb, cam, accent);
        }
    }

    /// Boss entity layer. `airborne_only` selects the cat's leaping pass, which
    /// the art draws after the player so it visibly passes overhead.
    fn draw_boss_body(&mut self, pb: &mut PixelBuffer, cam: Vec2, airborne_only: bool) {
        let accent = self.level.palette.accent;
        let mut query = self.ecs.query::<&Boss>();
        for boss in query.iter(&self.ecs) {
            if boss.is_airborne() != airborne_only {
                continue;
            }
            if !on_screen(self.w2s(boss.pos, cam), boss.kind.cull_margin(), self.render_viewport) {
                continue;
            }
            let pen = pb.set_pen(crate::render::lighting::boss_pen(boss.kind));
            match boss.view() {
                BossArtView::Cat(v) => art_boss_cat::draw_boss_cat(pb, &v, cam, accent),
                BossArtView::Broom(v) => art_boss_broom::draw(pb, &v, cam, accent),
                BossArtView::Mower(v) => art_boss_mower::draw_mower(pb, &v, cam, accent),
                BossArtView::Mangle(v) => art_boss_mangle::draw(pb, &v, cam, accent),
                BossArtView::NightEdition(v) => art_boss_night_edition::draw(pb, &v, cam, accent),
                // The falling crate is a hazard: `draw_hostile_layer` owns it.
                BossArtView::Magpie(v) => art_boss_magpie::draw_magpie(pb, &v, cam, accent),
                BossArtView::HorseshoeCrab(v) => {
                    art_boss_horseshoe_crab::draw(pb, &v, cam, accent)
                }
                BossArtView::MegaRat(v) => art_boss_megarat::draw_megarat(pb, &v, cam, accent),
                BossArtView::Hawk(v) => art_boss_hawk::draw_hawk(pb, &v, cam, accent),
                BossArtView::Cobra(v) => art_boss_cobra::draw(pb, &v, cam, accent),
                BossArtView::Lantern(v) => art_boss_lantern::draw(pb, &v, cam, accent),
            }
            pb.set_pen(pen);
            self.light_map.mark_emissive_highlights(pb, self.w2s(boss.pos, cam), 19);
        }
    }

    /// Standing water: every visible puddle, then every live ripple, under the
    /// wet-floor material (`render::lighting::pool_pen`, the same `Water` tag an
    /// acid pool wears, so lamps and specular streaks land on it exactly as the
    /// lighting's wet-floor reflections expect).
    ///
    /// Drawn with the floor, before the wall layers: a blob's rim may spill over
    /// a wall face and the wall is painted over it, which is what makes a long
    /// Sewer puddle read as water running along a corridor instead of a shape
    /// cut out of it. Render-only — nothing here writes an entity, an rng or a
    /// simulated value.
    fn draw_puddles(&mut self, pb: &mut PixelBuffer, cam: Vec2, viewport: Vec2) {
        if self.level.standing_water().next().is_none() {
            return;
        }
        let pen = pb.set_pen(crate::render::lighting::pool_pen());
        let theme = self.level.theme;
        let elapsed = self.elapsed;
        // The dark-mirror look whenever the lighting's water stage will light
        // it: materials are kept exactly when the effects run at Full or Hard
        // (`draw_scene`), the tiers `render::lighting::water` runs on.
        let glossy = pb.materials_on() && crate::render::lighting::water_gloss_enabled();
        // The puddles in view, on the stack (the Sewer has a couple of hundred
        // a map but only a handful a screen): each is drawn knowing the others,
        // so merged puddles share one shoreline. Past the array's size the
        // extras are still drawn, just without that knowledge.
        let mut seen = [crate::game::level::Puddle {
            pos: Vec2::ZERO,
            rx: 0.0,
            ry: 0.0,
            seed: 0,
        }; PUDDLES_IN_VIEW];
        let mut count = 0;
        for puddle in self.level.standing_water() {
            let margin = puddle.rx.max(puddle.ry) + 2.0;
            if on_screen(self.w2s(puddle.pos, cam), margin, viewport) && count < PUDDLES_IN_VIEW {
                seen[count] = *puddle;
                count += 1;
            }
        }
        for puddle in self.level.standing_water() {
            let margin = puddle.rx.max(puddle.ry) + 2.0;
            if !on_screen(self.w2s(puddle.pos, cam), margin, viewport) {
                continue;
            }
            art_puddle::draw_puddle_among_with(
                pb,
                puddle,
                &seen[..count],
                cam,
                elapsed,
                theme,
                glossy,
            );
        }
        // The ripples come straight from the resource the footstep system
        // writes; between ticks it is the ECS world's own (never a facade
        // field), so read it there and skip the pass if a hand-built world has
        // none.
        if let Some(steps) = self.ecs.get_resource::<PuddleSteps>() {
            for ripple in steps.ripples.iter().filter(|r| r.live) {
                if !on_screen(self.w2s(ripple.pos, cam), RIPPLE_DRAW_MARGIN, viewport) {
                    continue;
                }
                art_puddle::draw_ripple(
                    pb,
                    self.w2s(ripple.pos, cam),
                    ripple.age,
                    ripple.big,
                );
            }
        }
        pb.set_pen(pen);
    }

    /// Boss-owned projectiles in the bullet pass (blade discs, shrapnel).
    fn draw_boss_projectiles(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let accent = self.level.palette.accent;
        let mut query = self.ecs.query::<(&SpawnId, &BossProjectile)>();
        for (_, p) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            if !on_screen(self.w2s(p.pos, cam), p.radius + 4.0, self.render_viewport) {
                continue;
            }
            match p.kind {
                BossProjectileKind::BladeDisc => {
                    art_boss_mower::draw_blade_disc(pb, p.pos, p.anim, cam, accent)
                }
                BossProjectileKind::GrassShrapnel => {
                    art_boss_mower::draw_grass_shrapnel(pb, p.pos, p.vel, cam, accent)
                }
                BossProjectileKind::RolledTowel => {
                    art_boss_mangle::draw_towel(pb, p.pos, p.vel, p.anim, cam, accent)
                }
                _ => {
                    let s = self.w2s(p.pos, cam);
                    pb.filled_circle(s.0, s.1, p.radius.max(1.0) as i32, accent);
                }
            }
        }
    }

    /// Floor-layer decoration with no collision impact (see `art_marks`).
    ///
    /// Called right after [`art_floor::draw_floor`] so everything else (stains,
    /// walls, objects, entities) sits on top. Every pass is culled to the view
    /// and the whole method allocates nothing.
    fn draw_floor_marks(&self, pb: &mut PixelBuffer, cam: Vec2) {
        let level = &self.level;
        let vp = self.render_viewport;
        let theme = level.theme;

        // 1. Boss arena: the repeating 4 × 4-cell motif, strictly inside the
        //    arena rect (the art also clips it to the visible slice).
        if let Some(room) = level.boss_room() {
            art_marks::draw_boss_arena_floor(pb, room.rect, theme, cam);
        }

        // 2. One landmark per major hall, its variant its own hall index, so no
        //    two halls read the same (the Lab stencils it as a bay number).
        let mut hall = 0u8;
        for room in level.rooms.iter().filter(|r| r.kind == RoomKind::Hall) {
            let variant = hall;
            hall = hall.wrapping_add(1);
            let centre = room.rect.center_world();
            if !on_screen(
                self.w2s(centre, cam),
                art_marks::HALL_LANDMARK_HALF_EXTENT,
                vp,
            ) {
                continue;
            }
            art_marks::draw_hall_landmark(pb, centre, variant, cam, theme);
        }

        // 3. Each maze: the three gnaw marks back from the breach (pointing into
        //    it) and the art's sparing gold trail inside.
        for maze in &level.mazes {
            let entry = TileMap::tile_rect(maze.entry.0, maze.entry.1).center();
            let fork = TileMap::tile_rect(maze.fork.0, maze.fork.1).center();
            let to_entry = entry - fork;
            let approach = to_entry.len();

            // The gnaw marks sit `GNAW_MARK_OFFSETS` back from the entrance
            // centre along the approach. Nothing is painted past the public
            // fork: a mark in an unrelated corridor would point the wrong way.
            if approach >= 1.0 {
                let dir = to_entry * (1.0 / approach);
                for &dist in art_marks::GNAW_MARK_OFFSETS.iter() {
                    if dist > approach + 8.0 {
                        continue;
                    }
                    let pos = entry - dir * dist;
                    if !on_screen(self.w2s(pos, cam), 10.0, vp) {
                        continue;
                    }
                    art_marks::draw_gnaw_mark(pb, pos, dir, cam, theme);
                }
            }

            // Gold: one mark 1.5 cells inside the breach, two on the final
            // stretch to the cache (`GOLD_FLECK_SPACING` apart), and only on
            // open floor so a fleck never lands on a partition.
            let to_cache = maze.reward - entry;
            let trail = to_cache.len();
            if trail < 1.0 {
                continue;
            }
            let dir = to_cache * (1.0 / trail);
            let mut fleck = |pos: Vec2| {
                let (cx, cy) = crate::game::collision::cell_of(pos);
                if !level.tiles.get(cx, cy).is_open() || !on_screen(self.w2s(pos, cam), 8.0, vp) {
                    return;
                }
                art_marks::draw_gold_fleck(pb, pos, dir, cam, self.elapsed);
            };
            fleck(entry + dir * 24.0);
            if trail >= 2.0 * art_marks::GOLD_FLECK_SPACING + 24.0 {
                fleck(maze.reward - dir * art_marks::GOLD_FLECK_SPACING);
                fleck(maze.reward - dir * (2.0 * art_marks::GOLD_FLECK_SPACING));
            }
        }
    }

    /// The boss arena's doorway landmark, per mouth cell, on the ground or the
    /// overhang wall layer (`art_walls`' `BossThresholdView` contract).
    ///
    /// The mouth cells are the `Doorway` cells piercing the arena's one-cell
    /// wall ring; a two-cell mouth is one landmark, which is why each adjacent
    /// pair gets its own view (the art fuses them through the shared mask bit).
    fn draw_boss_thresholds(&self, pb: &mut PixelBuffer, cam: Vec2, layer: WallLayer) {
        let accent = self.level.palette.accent;
        let anim = self.elapsed;
        let mut draw = |view: &mut art_walls::BossThresholdView| {
            view.cell.anim_clock = anim;
            match layer {
                WallLayer::Ground => {
                    art_walls::draw_boss_threshold_ground(pb, view, cam, accent)
                }
                WallLayer::Overhang => {
                    art_walls::draw_boss_threshold_overhang(pb, view, cam, accent)
                }
                WallLayer::Body => {}
            }
        };
        self.for_each_boss_threshold(|mut view| draw(&mut view));
    }

    /// Visit each mouth cell of the boss arena's doorway(s) with the direction
    /// that points *into* the arena. Allocation-free.
    fn for_each_boss_threshold(&self, mut f: impl FnMut(art_walls::BossThresholdView)) {
        let Some(room) = self.level.boss_room() else {
            return;
        };
        let arena = room.rect;
        let tiles = &self.level.tiles;
        let mut visit = |cx: i32, cy: i32, inward: Cardinal| {
            if !tiles.in_bounds(cx, cy)
                || !matches!(tiles.get(cx, cy), TileKind::Wall(WallKind::Doorway { .. }))
            {
                return;
            }
            f(art_walls::BossThresholdView::new(
                tiles.wall_view(cx, cy),
                inward,
            ));
        };
        // North and south wall ring rows, then west and east wall ring columns
        // (the corner cells belong to no side).
        for cx in arena.x..arena.right() {
            visit(cx, arena.y - 1, Cardinal::South);
            visit(cx, arena.bottom(), Cardinal::North);
        }
        for cy in arena.y..arena.bottom() {
            visit(arena.x - 1, cy, Cardinal::East);
            visit(arena.right(), cy, Cardinal::West);
        }
    }

    /// Wall cells of the three art layers, culled to the view (see
    /// [`super::art_walls`] for the layer contract).
    fn draw_wall_layer(&self, pb: &mut PixelBuffer, cam: Vec2, layer: WallLayer) {
        let accent = self.level.palette.accent;
        let (x0, y0, x1, y1) = self
            .level
            .tiles
            .visible_range(cam, self.render_viewport, WALL_DRAW_MARGIN);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if !self.level.tiles.get(cx, cy).is_wall() {
                    continue;
                }
                let view = self.level.wall_view(cx, cy, self.elapsed);
                match layer {
                    WallLayer::Ground => art_walls::draw_wall_ground(pb, &view, cam, accent),
                    WallLayer::Body => art_walls::draw_wall_cell(pb, &view, cam, accent),
                    WallLayer::Overhang => {
                        art_walls::draw_wall_overhang(pb, &view, cam, accent)
                    }
                }
            }
        }
    }

    /// Traps, doors, rat-holes and maze rewards: `foreground == false` draws
    /// the ground layer (before entities), `true` the overlay layer (after
    /// them). Culled to the view.
    ///
    /// A floor trap is under every body standing on it: the glue board draws
    /// entirely in the ground pass, and a mousetrap's upper art joins the
    /// overlay only while its bar is in the air.
    fn draw_object_layer(&mut self, pb: &mut PixelBuffer, cam: Vec2, foreground: bool) {
        let accent = self.level.palette.accent;
        let vp = self.render_viewport;
        let anim = self.elapsed;
        let objects = &self.level.objects;

        let sticky_margin = if foreground { 48.0 } else { 28.0 };
        for view in &objects.sticky_traps {
            if !on_screen(self.w2s(view.pos, cam), sticky_margin, vp) {
                continue;
            }
            // The whole board is a floor object: tray, glue and the strands
            // that hold a caught body all draw in the ground pass, so every
            // body on the pad is drawn over every pixel of it. The strands
            // used to be composited after the entities; on a loaded board
            // (three strands, rims and beads per body) they buried the small
            // bodies, which read as enemies sliding under the trap.
            if foreground {
                continue;
            }
            let mut view = *view;
            view.anim = anim;
            art_traps::draw_sticky_trap_ground(pb, &view, cam, accent);
            art_traps::draw_sticky_trap(pb, &view, cam, accent);
        }
        for view in &objects.mousetraps {
            if !on_screen(self.w2s(view.pos, cam), 16.0, vp) {
                continue;
            }
            let mut view = *view;
            view.anim = anim;
            // The spring, pedal, bait and bar lie on the board while the trap
            // is cocked or spent, so a body walking over it is above them.
            // Only the bar in the air (the quiver and the 180-degree whip) is
            // genuinely overhead.
            let overhead = matches!(
                view.state,
                art_traps::MousetrapState::Triggered | art_traps::MousetrapState::Snapping
            );
            if !foreground {
                art_traps::draw_mousetrap_ground(pb, &view, cam, accent);
            }
            if foreground == overhead {
                art_traps::draw_mousetrap(pb, &view, cam, accent);
            }
        }
        for door in &objects.one_way_doors {
            if !on_screen(self.w2s(door.view.pos, cam), 20.0, vp) {
                continue;
            }
            let mut view = door.view;
            view.anim = anim;
            if foreground {
                art_traps::draw_one_way_door(pb, &view, cam, accent);
            } else {
                art_traps::draw_one_way_door_ground(pb, &view, cam, accent);
            }
        }
        for hole in &objects.rat_holes {
            if !on_screen(self.w2s(hole.view.pos, cam), 20.0, vp) {
                continue;
            }
            let mut view = hole.view;
            view.anim = anim;
            if foreground {
                art_traps::draw_rat_hole(pb, &view, cam, accent);
            } else {
                art_traps::draw_rat_hole_ground(pb, &view, cam, accent);
            }
        }
        // The mouse-hole shops: only the foreground pass draws them, over the
        // wall body, so the bent spoon and the whisker scratches read.
        if foreground {
            self.draw_shop_holes(pb, cam);
            for hole in &objects.shop_holes {
                if on_screen(self.w2s(hole.pos, cam), 24.0, vp) {
                    self.light_map.mark_emissive_highlights(pb, self.w2s(hole.face(), cam), 6);
                }
            }
        }
        for reward in &objects.rewards {
            if !on_screen(self.w2s(reward.pos, cam), 22.0, vp) {
                continue;
            }
            let mut view = *reward;
            view.anim = anim;
            if foreground {
                art_traps::draw_maze_reward(pb, &view, cam, accent);
                if view.state != MazeRewardState::Opened {
                    self.light_map.mark_emissive_highlights(pb, self.w2s(view.pos, cam), 7);
                }
            } else {
                art_traps::draw_maze_reward_ground(pb, &view, cam, accent);
            }
        }
    }

    /// The mouse-hole shops' markers (placeholder art, `art_walls`): a wall,
    /// the opening, the bent spoon and the three whisker scratches.
    fn draw_shop_holes(&self, pb: &mut PixelBuffer, cam: Vec2) {
        for hole in &self.level.objects.shop_holes {
            if !on_screen(self.w2s(hole.pos, cam), 24.0, self.render_viewport) {
                continue;
            }
            art_walls::draw_shop_hole(pb, hole, cam, self.elapsed);
        }
    }

    #[inline]
    fn w2s(&self, p: Vec2, cam: Vec2) -> (i32, i32) {
        ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
    }
}

/// A fresh level's player: the carried loadout, at full health.
///
/// XP and the player level restart every level (the carried powerups *are* the
/// progression); only the loadout crosses.
fn carried_player(center: Vec2, carried: Loadout) -> Player {
    let mut player = Player::new(center, Loadout::START_WEAPON);
    player.hp = carried.stats.max_hp;
    player.loadout = carried;
    player
}

/// Which layer of the wall art [`World::draw_wall_layer`] is drawing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WallLayer {
    /// Contact shadows and floor thresholds (under everything).
    Ground,
    /// Wall caps and south faces (under the entities).
    Body,
    /// Lintels that go over the entities (doors, rat-holes).
    Overhang,
}

/// World-pixel radius around the rat inside which **ordinary** hit digits are
/// clipped, so a crowded fight cannot bury the pellets next to the player
/// (`WAVE3 review`, punch list 5). Kill and big-hit digits are never clipped.
const DAMAGE_CLEAR_RADIUS: f32 = 26.0;

/// Extra cells to draw around the viewport: wall art casts shadows up to 4 px
/// south/east and the overhang layer reaches a few pixels up.
const WALL_DRAW_MARGIN: i32 = 1;

/// Screen-space margin a ripple can still paint into: the widest ring
/// `art_puddle::draw_ripple` draws, in px (it is a screen-space effect, so this
/// is not a world radius).
const RIPPLE_DRAW_MARGIN: f32 = 26.0;

/// How many on-screen puddles `draw_puddles` lets each other know about (the
/// shared shoreline of merged puddles). A 2x view of the Sewer holds about 20.
const PUDDLES_IN_VIEW: usize = 48;

/// The scale the giant rat is drawn at, or `None` when the find is not running.
///
/// Three phases, all read straight off the buff clock so there is no render
/// state to keep: the rat **grows** from 1x to [`powerups::GIANT_SCALE`] over
/// [`powerups::GIANT_GROW_SECS`], holds, then **blinks** between 5x and 4.5x
/// for the last [`powerups::GIANT_BLINK_SECS`] while it shrinks back over
/// [`powerups::GIANT_SHRINK_SECS`]. Never below 1x, so the first and last frames
/// of the find are exactly the rat the player always has.
pub fn giant_draw_scale(buff_left: f32) -> Option<f32> {
    use crate::game::ecs::systems::powerups as p;
    if buff_left <= 0.0 {
        return None;
    }
    let elapsed = (p::GROWTH_SECS - buff_left).max(0.0);
    let grow = (elapsed / p::GIANT_GROW_SECS).clamp(0.0, 1.0);
    let shrink = (buff_left / p::GIANT_SHRINK_SECS).clamp(0.0, 1.0);
    let mut scale = p::GIANT_SCALE * grow.min(shrink).max(0.0);
    // The warning blink: about three blinks a second over the last seconds, at
    // the smaller of the two sizes so the picture never grows past its hold.
    if buff_left <= p::GIANT_BLINK_SECS {
        let blink = (buff_left * 6.0) as i32;
        if blink % 2 == 0 {
            scale = scale.min(p::GIANT_BLINK_SCALE);
        }
    }
    Some(scale.max(1.0))
}

fn tiny_draw_scale(buff_left: f32) -> Option<f32> {
    use crate::game::ecs::systems::powerups as p;
    if buff_left <= 0.0 {
        return None;
    }
    let elapsed = (p::SHRINK_SECS - buff_left).max(0.0);
    let ramp = elapsed.min(buff_left).min(p::SHRINK_RAMP_SECS) / p::SHRINK_RAMP_SECS;
    let blink = buff_left <= p::SHRINK_BLINK_SECS && (buff_left * 6.0) as i32 % 2 == 0;
    let target = if blink { p::SHRINK_BLINK_SCALE } else { p::SHRINK_SCALE };
    Some(1.0 - (1.0 - target) * ramp)
}

/// Fraction of max HP a nest revive (`meta::Track::Revive`) stands the rat back
/// up with. Half a bar is enough to run, not enough to ignore the mistake.
const REVIVE_HP_FRAC: f32 = 0.5;

/// Immunity window a nest revive grants, in seconds. Deliberately longer than a
/// normal post-hit window (`player::HIT_IFRAMES` is 0.9 s): the rat stood up
/// inside whatever just killed it, so it needs a moment to move.
const REVIVE_IFRAMES: f32 = 2.5;

// ---- Prop rendering -------------------------------------------------------

/// Cheap viewport test: can a point with `margin` pixels of drawn extent still
/// touch a framebuffer of `viewport` size? (Screen-space, origin top-left.)
#[inline]
pub(crate) fn on_screen(s: (i32, i32), margin: f32, viewport: Vec2) -> bool {
    let x = s.0 as f32;
    let y = s.1 as f32;
    x + margin >= 0.0 && y + margin >= 0.0 && x - margin < viewport.x && y - margin < viewport.y
}

/// Does `kind` post a body-anchored floor tell that the hostile layer must
/// re-stamp over the lighting, and how far from the body's centre can that
/// tell reach (world px, used as the `on_screen` margin)?
///
/// One row per floor kind with its own tell, each computed from the constants
/// its hit test reads plus a few pixels of slack. The two wave-3 archetypes
/// (and every registration that still inherits one) keep the margin they
/// always had. `None`: nothing to stamp.
pub(crate) fn floor_tell_reach(kind: EnemyKind, radius: f32) -> Option<f32> {
    use crate::game::ecs::systems::{
        floor_flatbed, floor_kitchen, floor_lab, floor_laundry, floor_printworks, floor_sewer,
        floor_outfall,
    };
    match kind {
        // Sewer: the tongue leaves the body's rim.
        EnemyKind::SumpLeech => Some(radius + floor_sewer::LEECH_TONGUE_LEN + 10.0),
        // Kitchen: the ring grows to its end radius around the body.
        EnemyKind::EggTimer => {
            Some(floor_kitchen::EGG_RING_END + floor_kitchen::EGG_RING_WIDTH + 4.0)
        }
        // Lab: the ray starts at the needle tip.
        EnemyKind::WalkingSyringe => {
            Some(floor_lab::SYRINGE_TIP + floor_lab::SYRINGE_RAY_LEN + 4.0)
        }
        // Laundry: the jaws close on the spot the rat stood, up to the trigger
        // range away.
        EnemyKind::PegSnapper => Some(
            floor_laundry::PEG_TRIGGER_RANGE
                + floor_laundry::PEG_JAW_LEN
                + floor_laundry::PEG_JAW_GAP
                + 4.0,
        ),
        // Printworks: two feet around the locked rat.
        EnemyKind::StapleCrawler => Some(
            floor_printworks::STAPLE_TRIGGER_RANGE + 14.0 + floor_printworks::STAPLE_FOOT_RADIUS + 8.0,
        ),
        // Flatbed: the marked roll (stamped by the wave-4 block).
        EnemyKind::LooseCan => Some(radius + floor_flatbed::CAN_ROLL_LENGTH + 12.0),
        EnemyKind::Skater => Some(radius + floor_outfall::SKATER_SPLASH_LEN + 8.0),
        EnemyKind::Lampfish => Some(
            radius + floor_outfall::LAMPFISH_LUNGE_LEN.max(floor_outfall::LAMPFISH_BULB_OFFSET) + 8.0,
        ),
        _ => match kind.archetype() {
            // Unchanged: the Bell Flea's crosses and the Turnkey's crossbar.
            EnemyKind::BellFlea | EnemyKind::Turnkey => Some(radius * 3.0 + 8.0),
            _ => None,
        },
    }
}

/// Stamp one body's floor tell (the wave-3 block of `draw_hostile_layer`).
///
/// A floor kind with its own tell calls its floor's draw directly, so it never
/// also posts the archetype's mark; everything else is `draw_w3_tells`, as
/// before. Each floor draw gates itself on the body's phase.
fn draw_floor_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: palette::Rgb) {
    match e.kind {
        EnemyKind::SumpLeech => crate::game::art_roster_sewer::draw_leech_tongue(pb, e, cam),
        EnemyKind::EggTimer => crate::game::art_roster_kitchen::draw_egg_ring(pb, e, cam),
        EnemyKind::WalkingSyringe => crate::game::art_roster_lab::draw_syringe_tell(pb, e, cam),
        EnemyKind::PegSnapper => crate::game::art_roster_laundry::draw_peg_tell(pb, e, cam),
        EnemyKind::StapleCrawler => {
            crate::game::art_roster_printworks::draw_staple_tell(pb, e, cam)
        }
        EnemyKind::Skater | EnemyKind::Lampfish => art_roster_outfall::draw_tell(pb, e, cam, accent),
        _ => art_enemy::draw_w3_tells(pb, e, cam, accent),
    }
}

/// The peck lane's two rails (hostile layer, self-lit): a 6 px wide outlined
/// corridor from the body to the bill's reach, with a cap at the reach marking
/// where the bill stops.
///
/// Dashed while the tell is harmless, solid and brighter from the first peck
/// on, which is the same "shape first, colour second" rule the other floors'
/// tells keep. The screen endpoints come from `World::w2s`, so this is the one
/// place the lane is drawn and the shape is `drake::peck_lane`'s.
fn draw_hostile_lane(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), half: f32, live: bool) {
    const RAIL: palette::Rgb = (240, 236, 222);
    const RAIL_LIVE: palette::Rgb = (255, 250, 236);
    const CAP: palette::Rgb = (240, 158, 62);
    let (dx, dy) = ((b.0 - a.0) as f32, (b.1 - a.1) as f32);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let dir = (dx / len, dy / len);
    let side = (-dir.1, dir.0);
    // Half the lane, in whole pixels either side of the centre line.
    let half = half.round() as i32;
    let steps = len.ceil() as i32;
    let rail = if live { RAIL_LIVE } else { RAIL };
    for i in 0..=steps {
        // A dash of two pixels on, two off, while the lane is only a warning.
        if !live && (i / 2) % 2 == 1 {
            continue;
        }
        let t = i as f32;
        let c = (a.0 as f32 + dir.0 * t, a.1 as f32 + dir.1 * t);
        for sgn in [-1.0f32, 1.0] {
            let x = (c.0 + side.0 * sgn * half as f32).round() as i32;
            let y = (c.1 + side.1 * sgn * half as f32).round() as i32;
            pb.plot(x, y, rail);
        }
    }
    // The cap: where the bill reaches, a bracket across the lane.
    for w in -half..=half {
        let x = (b.0 as f32 + side.0 * w as f32).round() as i32;
        let y = (b.1 as f32 + side.1 * w as f32).round() as i32;
        pb.plot(x, y, CAP);
    }
}

// ---- Draw-layer tests ------------------------------------------------------
//
// `World::draw` is the only place the four level layers are ordered, so these
// tests pin the contract from `art_walls` ("Render Layers (Call Order)") and
// `art_traps` ("Render Pass Order"): wall cells paint, a rat-hole does *not*
// read as a solid cell, the object overlay paints over the entity line, and
// nothing outside the viewport is drawn at all.
#[cfg(test)]
mod draw_tests {
    use super::World;
    use crate::game::art_traps::RatHoleView;
    use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::level::{Level, MazeSection, RatHole, Room, RoomKind, Theme, TileRect};
    use crate::math::Vec2;
    use crate::render::framebuffer::PixelBuffer;

    /// The harness viewport (160 × 84 cells = 160 × 168 pixel rows), so a cell
    /// is exactly one screen cell column wide and two pixel rows tall.
    const VIEW: (usize, usize) = (160, 168);

    /// Render a frame of `level` with the rat parked on the map centre.
    fn frame(theme: Theme, level: Level) -> (PixelBuffer, Vec2) {
        let mut w = World::new(theme, 1);
        let centre = level.start;
        w.level = level;
        w.player.pos = centre;
        w.elapsed = 7.0; // a fixed clock: doors/overlays animate off it
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut pb);
        (pb, w.cam)
    }

    /// The camera takes [`super::SHAKE_SCALE`] of the simulation's shake
    /// offset (or `World::shake_scale` when the Graphics page set one), and the
    /// simulation's own number is untouched.
    #[test]
    fn the_camera_takes_a_scaled_share_of_the_shake() {
        let mut w = World::new(Theme::Sewer, 1);
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut pb);
        let still = w.cam;
        w.shake_ofs = Vec2::new(10.0, -4.0);
        w.draw(&mut pb);
        let moved = w.cam - still;
        assert!((moved.x - 10.0 * super::SHAKE_SCALE).abs() < 1e-4, "{moved:?}");
        assert!((moved.y + 4.0 * super::SHAKE_SCALE).abs() < 1e-4, "{moved:?}");
        assert_eq!(w.shake_ofs, Vec2::new(10.0, -4.0), "the sim's offset changed");
        assert_eq!(super::SHAKE_SCALE, 0.65);
        // A new world starts at the default.
        assert_eq!(w.shake_scale, super::SHAKE_SCALE);
    }

    /// The Graphics page's SCREEN SHAKE row: the camera takes exactly the share
    /// the setting holds (0 %, 50 %, 65 %, 100 %), and the simulation's own
    /// offset is never touched by it.
    #[test]
    fn the_shake_share_setting_scales_the_camera_only() {
        let mut w = World::new(Theme::Sewer, 1);
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut pb);
        let still = w.cam;
        w.shake_ofs = Vec2::new(10.0, -4.0);
        for (scale, x, y) in [
            (0.0_f32, 0.0_f32, 0.0_f32),
            (0.5, 5.0, -2.0),
            (super::SHAKE_SCALE, 6.5, -2.6),
            (1.0, 10.0, -4.0),
        ] {
            w.shake_scale = scale;
            w.draw(&mut pb);
            let moved = w.cam - still;
            assert!((moved.x - x).abs() < 1e-3, "{scale}: {moved:?}");
            assert!((moved.y - y).abs() < 1e-3, "{scale}: {moved:?}");
            assert_eq!(
                w.shake_ofs,
                Vec2::new(10.0, -4.0),
                "{scale}: nothing simulated may read the setting"
            );
        }
    }

    /// A blank map with a 2 × 2 block of solid cells and one rat-hole cell to
    /// its east, both a few pixels off the camera origin.
    fn walled(theme: Theme) -> Level {
        let mut level = Level::empty(theme);
        level
            .tiles
            .fill_cells(95, 75, 2, 2, TileKind::Wall(WallKind::Solid));
        level.tiles.set(
            99,
            75,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        level.objects.rat_holes.push(RatHole {
            cell: (99, 75),
            // Vertical wall, east-west passage (`WallAxis` names the *wall*).
            view: RatHoleView::new(
                TileMap::tile_rect(99, 75).center(),
                Vec2::new(1.0, 0.0),
            ),
        });
        level.tiles.rebuild_masks();
        level
    }

    /// Same map, but the rat-hole cell is full solid and there is no hole
    /// object: the reference for "the hole is not a wall".
    fn solid_reference(theme: Theme) -> Level {
        let mut level = walled(theme);
        level.objects.rat_holes.clear();
        level.tiles.set(99, 75, TileKind::Wall(WallKind::Solid));
        level.tiles.rebuild_masks();
        level
    }

    /// Screen pixel range of one cell.
    fn cell_span(cam: Vec2, cx: i32, cy: i32) -> (i32, i32, i32, i32) {
        let x = (cx * 16) as f32 - cam.x.floor();
        let y = (cy * 16) as f32 - cam.y.floor();
        let x = x.round() as i32;
        let y = y.round() as i32;
        (x, y, x + 16, y + 16)
    }

    /// How many pixels differ between two frames inside one cell, and how many
    /// of the second frame's pixels in that cell are not the background.
    fn diff_in_cell(a: &PixelBuffer, b: &PixelBuffer, cam: Vec2, cx: i32, cy: i32) -> (usize, usize) {
        let (x0, y0, x1, y1) = cell_span(cam, cx, cy);
        let (mut diff, mut lit) = (0, 0);
        for y in y0.max(0)..y1.min(VIEW.1 as i32) {
            for x in x0.max(0)..x1.min(VIEW.0 as i32) {
                let pa = a.pixel_at(x as usize, y as usize);
                let pb = b.pixel_at(x as usize, y as usize);
                if pa != pb {
                    diff += 1;
                }
                if pb != (0, 0, 0) {
                    lit += 1;
                }
            }
        }
        (diff, lit)
    }

    /// **Frame test per theme**: the same map renders on all three themes, the
    /// wall body layer actually paints the solid cells, and it paints more than
    /// the blank map does there.
    #[test]
    fn every_theme_paints_its_wall_cells() {
        for theme in Theme::all() {
            let (blank, cam) = frame(theme, Level::empty(theme));
            let (walled, _) = frame(theme, walled(theme));
            for (cx, cy) in [(95, 75), (96, 75), (95, 76), (96, 76)] {
                let (diff, lit) = diff_in_cell(&blank, &walled, cam, cx, cy);
                assert!(
                    diff > 100,
                    "{theme:?}: wall cell {cx},{cy} barely changed ({diff} px)"
                );
                assert!(
                    lit >= 16 * 16,
                    "{theme:?}: wall cell {cx},{cy} is mostly empty ({lit} px lit)"
                );
            }
            // A cell four cells away from any wall is untouched: the layer is
            // culled per cell and does not paint a carpet of caps.
            let (diff, _) = diff_in_cell(&blank, &walled, cam, 90, 70);
            assert_eq!(diff, 0, "{theme:?}: empty floor repainted by the wall layer");
        }
    }

    /// The rat-hole cell must not read as a solid wall: the object layer (its
    /// recessed floor, cheeks and over-arch lintel) and the wall layer's piers
    /// together paint a visibly different cell from a `Solid` one, and the pass
    /// through the middle of the gap stays open floor.
    #[test]
    fn a_rat_hole_does_not_read_as_a_solid_wall() {
        for theme in Theme::all() {
            let (hole, cam) = frame(theme, walled(theme));
            let (solid, _) = frame(theme, solid_reference(theme));
            let (diff, _) = diff_in_cell(&hole, &solid, cam, 99, 75);
            assert!(
                diff > 20,
                "{theme:?}: a rat-hole looks the same as a solid cell ({diff} px)"
            );
            // The gap is 8 px wide and centred in the cell; its middle rows
            // must show floor/arch, not the solid top face.
            let (gx0, gy0, _, _) = cell_span(cam, 99, 75);
            let mut gap_diff = 0;
            for y in gy0 + 6..gy0 + 12 {
                for x in gx0 + 4..gx0 + 12 {
                    if hole.pixel_at(x as usize, y as usize) != solid.pixel_at(x as usize, y as usize)
                    {
                        gap_diff += 1;
                    }
                }
            }
            assert!(
                gap_diff > 4,
                "{theme:?}: the rat-hole gap is drawn like solid wall"
            );
        }
    }

    /// Pixels differing between two frames inside a screen box.
    fn diff_in_box(a: &PixelBuffer, b: &PixelBuffer, x0: i32, y0: i32, w: i32, h: i32) -> usize {
        let mut diff = 0;
        for y in y0.max(0)..(y0 + h).min(VIEW.1 as i32) {
            for x in x0.max(0)..(x0 + w).min(VIEW.0 as i32) {
                if a.pixel_at(x as usize, y as usize) != b.pixel_at(x as usize, y as usize) {
                    diff += 1;
                }
            }
        }
        diff
    }

    /// Screen origin of one cell (matches [`cell_span`]'s rounding).
    fn cell_at(cam: Vec2, cx: i32, cy: i32) -> (i32, i32) {
        let (x, y, _, _) = cell_span(cam, cx, cy);
        (x, y)
    }

    /// A maze section whose approach runs west→east along row 80, from the
    /// public fork at (96, 80) through the breach at (100, 80) to a cache six
    /// cells inside.
    fn maze_section() -> MazeSection {
        MazeSection {
            bounds: TileRect::new(98, 76, 12, 9),
            ring: TileRect::new(100, 78, 8, 5),
            entry: (100, 80),
            fork: (96, 80),
            exit: (103, 86),
            exit_dir: Cardinal::East,
            reward: TileMap::tile_rect(106, 80).center(),
            reward_cell: (106, 80),
        }
    }

    /// **Boss arena floor**: `art_marks::draw_boss_arena_floor` paints the
    /// arena's repeating motif inside the arena rect and nowhere else.
    #[test]
    fn the_boss_arena_floor_motif_is_drawn_inside_the_arena_only() {
        for theme in Theme::all() {
            let (plain, cam) = frame(theme, Level::empty(theme));
            let mut level = Level::empty(theme);
            level.rooms.push(Room::new(
                TileRect::new(97, 76, 8, 8),
                RoomKind::Boss,
            ));
            let (marked, _) = frame(theme, level);

            let (ix, iy) = cell_at(cam, 100, 79);
            let (dx, dy) = cell_at(cam, 95, 79);
            let inside = diff_in_box(&plain, &marked, ix, iy, 16, 16);
            let outside = diff_in_box(&plain, &marked, dx, dy, 16, 16);
            assert!(
                inside > 8,
                "{theme:?}: the boss arena motif painted {inside} px"
            );
            assert_eq!(
                outside, 0,
                "{theme:?}: the boss arena motif bled onto {outside} floor px"
            );
        }
    }

    /// **Hall landmarks**: one per major hall, the variant following the hall's
    /// own index, so the second hall on a map is not a copy of the first.
    #[test]
    fn each_major_hall_gets_its_own_landmark_variant() {
        const A: TileRect = TileRect::new(96, 74, 6, 6);
        const B: TileRect = TileRect::new(100, 80, 6, 6);
        for theme in Theme::all() {
            let (plain, cam) = frame(theme, Level::empty(theme));

            let mut only_b = Level::empty(theme);
            only_b.rooms.push(Room::new(B, RoomKind::Hall));
            let (b_first, _) = frame(theme, only_b);

            let mut both = Level::empty(theme);
            both.rooms.push(Room::new(A, RoomKind::Hall));
            both.rooms.push(Room::new(B, RoomKind::Hall));
            let (b_second, _) = frame(theme, both);

            // Hall A (index 0) is painted.
            let (ax, ay) = cell_at(cam, 98, 76);
            assert!(
                diff_in_box(&plain, &b_second, ax, ay, 16, 16) > 0,
                "{theme:?}: the first hall got no landmark"
            );
            // Hall B is drawn with variant 1 when it is the second hall but
            // variant 0 when it is the first: different art.
            let (bx, by) = cell_at(cam, 102, 82);
            assert!(
                diff_in_box(&b_first, &b_second, bx - 8, by - 8, 32, 32) > 0,
                "{theme:?}: every hall drew the same landmark variant"
            );
        }
    }

    /// **Gnaw marks & gold flecks**: the maze approach is marked at the art's
    /// offsets back from the breach, and the inside of the maze gets the
    /// sparing gold trail.
    #[test]
    fn a_maze_approach_gets_gnaw_marks_and_a_gold_trail() {
        for theme in Theme::all() {
            let (plain, cam) = frame(theme, Level::empty(theme));
            let mut level = Level::empty(theme);
            level.mazes.push(maze_section());
            let (marked, _) = frame(theme, level);

            // Gnaw marks at 16 / 35 / 54 px back from the breach centre: cells
            // 99, 98 and 97 of row 80.
            for cx in [99, 98, 97] {
                let (x, y) = cell_at(cam, cx, 80);
                let d = diff_in_box(&plain, &marked, x, y, 16, 16);
                assert!(
                    d > 0,
                    "{theme:?}: no gnaw mark in cell {cx},80 ({d} px)"
                );
            }
            // Beyond the public fork (col 95 and west) nothing is painted.
            let (fx, fy) = cell_at(cam, 95, 80);
            assert_eq!(
                diff_in_box(&plain, &marked, fx, fy, 16, 16),
                0,
                "{theme:?}: a gnaw mark was painted past the public fork"
            );

            // The gold trail starts 24 px inside the breach (cell 101).
            let (gx, gy) = cell_at(cam, 101, 80);
            assert!(
                diff_in_box(&plain, &marked, gx + 8, gy + 4, 12, 12) > 0,
                "{theme:?}: no gold fleck inside the maze"
            );
        }
    }

    /// A blank map whose row 75 is a north wall with a fused two-cell doorway
    /// at columns 98–99 (the shape a boss arena's double door has).
    fn boss_mouth_map(theme: Theme) -> Level {
        let mut level = Level::empty(theme);
        level
            .tiles
            .fill_cells(96, 75, 12, 1, TileKind::Wall(WallKind::Solid));
        for cx in [98, 99] {
            level.tiles.set(
                cx,
                75,
                TileKind::Wall(WallKind::Doorway {
                    axis: WallAxis::Horizontal,
                }),
            );
        }
        level.tiles.rebuild_masks();
        level
    }

    /// **Boss threshold**: the arena's doorway mouth cells get the themed
    /// threshold decal on the ground layer (and the overhang trim), and a map
    /// with the same holes but no boss arena gets neither.
    #[test]
    fn the_boss_arena_doorway_gets_its_threshold_landmark() {
        for theme in Theme::all() {
            // North wall ring of the arena at row 75: two adjacent doorway
            // cells fusing into one 32 px mouth, walled on both flanks.
            let mut level = boss_mouth_map(theme);
            let mut no_arena = boss_mouth_map(theme);
            no_arena.rooms.push(Room::new(
                TileRect::new(97, 76, 8, 8),
                RoomKind::Room,
            ));
            level.rooms.push(Room::new(
                TileRect::new(97, 76, 8, 8),
                RoomKind::Boss,
            ));

            let (plain, cam) = frame(theme, no_arena);
            let (threshold, _) = frame(theme, level);
            for cx in [98, 99] {
                let (x, y) = cell_at(cam, cx, 75);
                let d = diff_in_box(&plain, &threshold, x, y, 16, 16);
                assert!(
                    d > 0,
                    "{theme:?}: mouth cell {cx},75 got no threshold decal"
                );
            }
        }
    }

    /// Culling: decoration whose cells are all far outside the view never
    /// reaches the frame, so it is byte-identical to a map without it.
    #[test]
    fn floor_marks_outside_the_viewport_are_never_drawn() {
        for theme in Theme::all() {
            let (plain, _) = frame(theme, Level::empty(theme));
            let mut far = Level::empty(theme);
            far.rooms.push(Room::new(
                TileRect::new(4, 4, 10, 10),
                RoomKind::Boss,
            ));
            far.rooms.push(Room::new(TileRect::new(20, 20, 8, 8), RoomKind::Hall));
            far.tiles
                .fill_cells(3, 3, 12, 1, TileKind::Wall(WallKind::Solid));
            for cx in [8, 9] {
                far.tiles.set(
                    cx,
                    3,
                    TileKind::Wall(WallKind::Doorway {
                        axis: WallAxis::Horizontal,
                    }),
                );
            }
            far.tiles.rebuild_masks();
            let mut maze = maze_section();
            maze.entry = (30, 30);
            maze.fork = (26, 30);
            maze.reward = TileMap::tile_rect(36, 30).center();
            maze.reward_cell = (36, 30);
            far.mazes.push(maze);

            let (with_far, _) = frame(theme, far);
            let differing = diff_in_box(&plain, &with_far, 0, 0, VIEW.0 as i32, VIEW.1 as i32);
            assert_eq!(
                differing, 0,
                "{theme:?}: off-screen floor marks painted {differing} px"
            );
        }
    }

    /// Culling: a wall cluster on the far side of the 3 200 × 2 560 map is not
    /// drawn at all, so the frame is byte-identical to a map without it. That
    /// is the whole reason `draw_wall_layer` walks `visible_range` instead of
    /// the 32 000 cells.
    #[test]
    fn walls_outside_the_viewport_are_never_drawn() {
        for theme in Theme::all() {
            let (blank, _) = frame(theme, Level::empty(theme));
            let mut far = Level::empty(theme);
            far.tiles
                .fill_cells(10, 12, 24, 18, TileKind::Wall(WallKind::Solid));
            far.tiles.rebuild_masks();
            let (with_far, _) = frame(theme, far);
            let mut differing = 0;
            for y in 0..VIEW.1 {
                for x in 0..VIEW.0 {
                    if blank.pixel_at(x, y) != with_far.pixel_at(x, y) {
                        differing += 1;
                    }
                }
            }
            assert_eq!(
                differing, 0,
                "{theme:?}: off-screen walls painted {differing} px"
            );
        }
    }
}

// ---- Mouse-wheel zoom ------------------------------------------------------
//
// Zoom is render-only, and these tests pin that: the *simulation* keeps the
// unzoomed viewport (so a seed plays out identically at every zoom), while the
// *renderer* covers `viewport × zoom` world pixels, resamples them onto the
// screen grid, and keeps text-sized overlays (damage numbers) at screen size.
#[cfg(test)]
mod zoom_tests {
    use super::*;
    use crate::game::gem::Gem;
    use crate::game::level::Theme;
    use rand::RngExt;

    /// The app's 120 × 40-cell playfield: 120 × 74 half-block pixels.
    const SCREEN: (usize, usize) = (120, 74);

    fn screen() -> PixelBuffer {
        PixelBuffer::new(SCREEN.0, SCREEN.1)
    }

    /// FNV-1a over every pixel, for "did the frame actually change".
    fn frame_hash(pb: &PixelBuffer) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let (r, g, b) = pb.pixel_at(x, y);
                for byte in [r, g, b] {
                    h ^= u64::from(byte);
                    h = h.wrapping_mul(0x1000_0000_01b3);
                }
            }
        }
        h
    }

    /// Everything the simulation owns, as raw bits: the counters, the player,
    /// every entity in `SpawnId` order (position + health) and the next rng
    /// draws. Two runs with the same fingerprint are the same run.
    fn fingerprint(w: &mut World) -> Vec<u64> {
        let mut f: Vec<u64> = vec![
            w.elapsed.to_bits().into(),
            u64::from(w.kills),
            u64::from(w.score),
            u64::from(w.pending_levelups),
            u64::from(w.game_over),
            w.player.pos.x.to_bits().into(),
            w.player.pos.y.to_bits().into(),
            w.player.hp.to_bits().into(),
            u64::from(w.player.level),
            u64::from(w.player.xp),
        ];
        let enemies = w.enemies();
        f.push(enemies.len() as u64);
        for e in &enemies {
            f.push(u64::from(e.id));
            f.push(e.pos.x.to_bits().into());
            f.push(e.pos.y.to_bits().into());
            f.push(e.hp.to_bits().into());
        }
        let bullets = w.bullets();
        f.push(bullets.len() as u64);
        for b in &bullets {
            f.push(b.pos.x.to_bits().into());
            f.push(b.pos.y.to_bits().into());
        }
        let gems = w.gems();
        f.push(gems.len() as u64);
        for g in &gems {
            f.push(g.pos.x.to_bits().into());
            f.push(g.pos.y.to_bits().into());
            f.push(u64::from(g.value));
        }
        f.push(w.pickups().len() as u64);
        f.push(w.pools().len() as u64);
        // The rng's own state is part of the run: sample the next draws
        // (this runs after the last tick, so consuming them is harmless and
        // identical across the compared runs).
        for _ in 0..8 {
            f.push(w.rng.random::<u64>());
        }
        f
    }

    /// **The determinism promise.** The same seed and the same inputs play out
    /// identically at z = 0.5, 1 and 2, however many frames each is drawn —
    /// and when no frame is drawn at all. Zoom lives in the renderer and never
    /// reaches the simulation; the simulation's viewport is session input,
    /// set here the way the App sets it on a resize.
    #[test]
    fn the_same_seed_plays_out_identically_at_every_zoom() {
        assert_zoom_replay(Theme::Sewer);
    }

    #[test]
    fn forced_lantern_arrival_uses_the_surge_gate() {
        let mut w = World::new(Theme::Outfall, 11);
        w.configure_testbed();
        w.elapsed = 8.5;
        assert!(w.outfall_surge_enabled());
        w.force_boss(BossKind::Lantern);
        assert!(!w.outfall_surge_enabled(), "the pending arrival suppresses the warning");
        w.clear_testbed_boss();
        assert!(w.force_boss_here(BossKind::Lantern));
        assert!(!w.outfall_surge_enabled(), "the manual arrival also suppresses the warning");
        w.clear_testbed_boss();
        assert!(w.outfall_surge_enabled());
    }

    #[test]
    fn outfall_surge_and_lure_replay_identically_at_every_zoom() {
        assert_zoom_replay(Theme::Outfall);
    }

    fn assert_zoom_replay(theme: Theme) {
        let logical = Vec2::new(SCREEN.0 as f32, SCREEN.1 as f32);
        let mut prints = Vec::new();
        let mut hashes = Vec::new();
        // `None` is the headless run: same viewport, never drawn.
        for z in [Some(zoom::MIN), Some(1.0), Some(zoom::MAX), None] {
            let drawn = z.is_some();
            let z = z.unwrap_or(1.0);
            let mut w = World::new(theme, 11);
            if theme == Theme::Outfall {
                // Start beside a lure immediately before the surge warning.
                w.player.pos = w.level.objects.lure_traps[0].pos;
                w.elapsed = 7.5;
                w.player.god = true;
            }
            // Lighting and smoke are render-only too: keep them on (whatever
            // the terminal's colour depth says) so the promise covers them.
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            w.set_zoom_instant(z);
            assert_eq!(w.zoom_view, z, "the world renders at the requested zoom");
            assert_eq!(w.viewport, Vec2::new(160.0, 80.0), "the fresh viewport");
            // What `App::render` does on a resize, before the first tick.
            w.viewport = logical;
            let mut pb = screen();
            for tick in 0..900u32 {
                // The same scripted weave at every zoom.
                let dir = match (tick / 30) % 4 {
                    0 => Vec2::new(1.0, 0.0),
                    1 => Vec2::new(0.0, 1.0),
                    2 => Vec2::new(-1.0, 0.0),
                    _ => Vec2::new(0.0, -1.0),
                };
                w.update(1.0 / 60.0, dir, tick % 120 == 0);
                w.tick_zoom(1.0 / 60.0);
                if drawn {
                    w.draw(&mut pb);
                    assert_eq!(
                        w.render_viewport, logical,
                        "the renderer's viewport is the *unzoomed* frame"
                    );
                }
                assert_eq!(
                    w.viewport, logical,
                    "neither a draw nor a zoom may move the simulation's viewport"
                );
            }
            if theme == Theme::Outfall {
                use crate::game::hazards::LurePhase;
                assert_eq!(w.level.objects.lure_traps[0].phase, LurePhase::Spent);
            }
            prints.push(fingerprint(&mut w));
            hashes.push(frame_hash(&pb));
        }
        assert_eq!(prints[0], prints[1], "z = 0.5 diverged from the reference run");
        assert_eq!(prints[1], prints[2], "z = 2 diverged from the reference run");
        assert_eq!(prints[1], prints[3], "a run never drawn diverged from the drawn one");
        assert_ne!(hashes[0], hashes[1], "z = 0.5 must render a different frame");
        assert_ne!(hashes[1], hashes[2], "z = 2 must render a different frame");
    }

    /// **The Growth Serum's zoom override** (batch 13): the view pulls back to
    /// the widest rung while the find runs and returns to the player's own zoom
    /// afterwards — and the same run ticks out identically whether the override
    /// moves the render zoom or not, because it only ever touches `zoom_view`.
    #[test]
    fn the_giant_zoom_override_widens_the_view_and_returns_without_touching_the_sim() {
        use crate::game::ecs::systems::powerups;
        let logical = Vec2::new(SCREEN.0 as f32, SCREEN.1 as f32);
        let mut prints = Vec::new();
        let mut views = Vec::new();
        // Both runs are the same giant run; only one of them lets the render
        // zoom ease at all (`tick_zoom` is what the App calls each frame).
        for with_override in [false, true] {
            let mut w = World::new(Theme::Sewer, 11);
            w.viewport = logical;
            w.set_zoom_instant(1.0);
            let mut pb = screen();
            let (mut hold, mut back) = (0.0f32, 0.0f32);
            for tick in 0..1500u32 {
                // Armed by hand at 0.5 s: a real pickup would spend rng, and
                // this test is about the render override, not the drop table.
                if tick == 30 {
                    w.player.buffs.giant = powerups::GROWTH_SECS;
                }
                let dir = match (tick / 30) % 4 {
                    0 => Vec2::new(1.0, 0.0),
                    1 => Vec2::new(0.0, 1.0),
                    2 => Vec2::new(-1.0, 0.0),
                    _ => Vec2::new(0.0, -1.0),
                };
                w.update(1.0 / 60.0, dir, tick % 120 == 0);
                if with_override {
                    w.tick_zoom(1.0 / 60.0);
                }
                w.draw(&mut pb);
                // The middle of the find (5.5 s in) and 1 s after it ran out.
                if tick == 330 {
                    hold = w.zoom_view;
                }
                if tick == 1290 {
                    back = w.zoom_view;
                }
            }
            prints.push(fingerprint(&mut w));
            views.push((hold, back));
        }
        assert_eq!(
            prints[0], prints[1],
            "the zoom override must not reach the simulation"
        );
        assert_eq!(views[0].0, 1.0, "a pinned view never moves");
        assert_eq!(views[0].1, 1.0);
        assert!(
            (views[1].0 - crate::render::zoom::MAX).abs() < 0.01,
            "the giant view must reach the widest rung, got {}",
            views[1].0
        );
        assert!(
            (views[1].1 - 1.0).abs() < 0.01,
            "the view must return to the player's own zoom, got {}",
            views[1].1
        );
    }

    /// The giant draw scale's three phases: it grows from 1x to 5x over the
    /// first 0.4 s, holds at 5x, blinks down to 4.5x over the last 3 s, and
    /// never leaves 1x..=5x.
    #[test]
    fn the_giant_grows_holds_blinks_and_never_exceeds_five() {
        use crate::game::ecs::systems::powerups as p;
        assert_eq!(giant_draw_scale(0.0), None, "no serum, no giant");
        assert_eq!(giant_draw_scale(p::GROWTH_SECS), Some(1.0), "it starts at life size");
        // The clock is f32, so the phase boundaries land within a rounding error
        // of their exact values: compare to a tolerance, not to the bit.
        let half = giant_draw_scale(p::GROWTH_SECS - p::GIANT_GROW_SECS * 0.5)
            .expect("still giant");
        assert!(
            (half - p::GIANT_SCALE * 0.5).abs() < 1e-3,
            "half way through the grow-in it is {half}, not {}",
            p::GIANT_SCALE * 0.5
        );
        let grown = giant_draw_scale(p::GROWTH_SECS - p::GIANT_GROW_SECS).expect("still giant");
        assert!(
            (grown - p::GIANT_SCALE).abs() < 1e-3,
            "the grow-in must end at {}x, got {grown}",
            p::GIANT_SCALE
        );
        assert_eq!(giant_draw_scale(15.0), Some(p::GIANT_SCALE));
        // The last three seconds blink between 5x and 4.5x.
        let mut seen = [false; 2];
        let mut t = p::GIANT_BLINK_SECS.max(p::GIANT_SHRINK_SECS);
        while t > 0.0 {
            let s = giant_draw_scale(t).expect("still giant");
            assert!((1.0..=p::GIANT_SCALE).contains(&s), "{s} is off the scale");
            if s >= p::GIANT_SCALE - 1e-4 {
                seen[1] = true;
            }
            if (s - p::GIANT_BLINK_SCALE).abs() < 1e-4 {
                seen[0] = true;
            }
            t -= 1.0 / 60.0;
        }
        assert!(seen[0] && seen[1], "the warning blink never showed both sizes");
        assert_eq!(giant_draw_scale(0.0), None);
    }

    #[test]
    fn shrink_draught_draw_scale_eases_and_blinks() {
        use crate::game::ecs::systems::powerups as p;
        let scale = |left| tiny_draw_scale(left).unwrap();
        assert_eq!(tiny_draw_scale(0.0), None);
        assert_eq!(scale(p::SHRINK_SECS), 1.0);
        let halfway = scale(p::SHRINK_SECS - p::SHRINK_RAMP_SECS * 0.5);
        assert!((halfway - (1.0 + p::SHRINK_SCALE) * 0.5).abs() < 1e-5);
        assert!((scale(8.0) - p::SHRINK_SCALE).abs() < 1e-6);
        assert_eq!(scale(1.4), p::SHRINK_BLINK_SCALE);
        assert!((scale(1.6) - p::SHRINK_SCALE).abs() < 1e-6);
        assert!(scale(0.01) > 0.95);
    }

    /// Mean luminance of the `2r × 2r` block of screen pixels around `(cx, cy)`
    /// (clipped to the buffer).
    fn mean_lum(pb: &PixelBuffer, cx: f32, cy: f32, r: i32) -> f32 {
        let (mut sum, mut n) = (0.0f32, 0u32);
        for y in (cy as i32 - r)..(cy as i32 + r) {
            for x in (cx as i32 - r)..(cx as i32 + r) {
                if x < 0 || y < 0 || x >= pb.w as i32 || y >= pb.h as i32 {
                    continue;
                }
                let (r8, g8, b8) = pb.pixel_at(x as usize, y as usize);
                sum += 0.299 * f32::from(r8) + 0.587 * f32::from(g8) + 0.114 * f32::from(b8);
                n += 1;
            }
        }
        sum / n.max(1) as f32
    }

    /// **Lights follow the zoom.** The light pass runs on the world-resolution
    /// scene (before the resample) with the camera the sprites were drawn
    /// with, so the rat's own light sits on the rat at every zoom: its
    /// neighbourhood is brighter than the far corners of the view, and lit
    /// harder (relative to the same frame with lighting off) than they are.
    #[test]
    fn the_player_light_stays_on_the_rat_at_every_zoom() {
        const R: i32 = 6;
        for z in [1.0, zoom::MAX] {
            let mut w = World::new(Theme::Sewer, 11);
            w.smoke_enabled = true;
            // The plain light pass: this pins where the rat's aura lands. The
            // batch-13 effects add lamp shafts and wall sheen of their own,
            // which may lift a corner by a larger *ratio* than the aura lifts
            // the already-bright centre.
            w.lighting_fx = false;
            w.set_zoom_instant(z);

            w.lighting = LightingMode::Off;
            let mut unlit = screen();
            w.draw(&mut unlit);
            w.lighting = LightingMode::Full;
            let mut lit = screen();
            w.draw(&mut lit);

            // Where the rat landed on the screen grid.
            let px = (w.player.pos.x - w.cam.x) / w.zoom_scale.x;
            let py = (w.player.pos.y - w.cam.y) / w.zoom_scale.y;
            assert!(
                (px - SCREEN.0 as f32 * 0.5).abs() < 2.0 && (py - SCREEN.1 as f32 * 0.5).abs() < 2.0,
                "z = {z}: the rat starts at the centre of the view, not ({px}, {py})"
            );

            let whole = mean_lum(&lit, px, py, SCREEN.0 as i32);
            assert!(whole > 8.0, "z = {z}: the lit frame is uniformly dark ({whole})");
            assert_ne!(frame_hash(&lit), frame_hash(&unlit), "z = {z}: lighting did nothing");

            let centre = mean_lum(&lit, px, py, R);
            let centre_gain = centre / mean_lum(&unlit, px, py, R).max(1.0);
            let (sw, sh) = (SCREEN.0 as f32, SCREEN.1 as f32);
            let r = R as f32;
            for (cx, cy) in [(r, r), (sw - r, r), (r, sh - r), (sw - r, sh - r)] {
                let corner = mean_lum(&lit, cx, cy, R);
                let gain = corner / mean_lum(&unlit, cx, cy, R).max(1.0);
                assert!(
                    centre > corner,
                    "z = {z}: the rat ({centre}) must be brighter than the corner at ({cx}, {cy}) ({corner})"
                );
                assert!(
                    centre_gain > gain,
                    "z = {z}: the light must favour the rat ({centre_gain}) over the corner at ({cx}, {cy}) ({gain})"
                );
            }
        }
    }

    #[test]
    fn zoom_steps_the_ladder_clamps_and_eases() {
        let mut w = World::new(Theme::Sewer, 3);
        assert_eq!(w.zoom, zoom::DEFAULT, "a fresh run is unzoomed");
        assert_eq!(w.zoom_by(1), 1.25, "wheel down widens the view");
        assert_eq!(w.zoom_by(1), 1.5);
        assert_eq!(w.zoom_by(1), zoom::MAX);
        assert_eq!(w.zoom_by(1), zoom::MAX, "the widest view clamps");
        assert_eq!(w.zoom_by(-1), 1.5);
        assert_eq!(w.zoom_by(-1), 1.25);
        assert_eq!(w.zoom_by(-1), 1.0);
        assert_eq!(w.zoom_by(-1), 0.75);
        assert_eq!(w.zoom_by(-1), zoom::MIN);
        assert_eq!(w.zoom_by(-1), zoom::MIN, "the closest view clamps");

        // The rendered value chases the setting over ~0.15 s, then is exact.
        w.set_zoom_instant(1.0);
        w.zoom_by(1);
        w.zoom_by(1);
        let mut t = 0.0;
        while t < zoom::EASE_SECS {
            w.tick_zoom(1.0 / 60.0);
            t += 1.0 / 60.0;
            assert!(w.zoom_view > 1.0 && w.zoom_view <= 1.5, "no overshoot");
        }
        for _ in 0..30 {
            w.tick_zoom(1.0 / 60.0);
        }
        assert_eq!(w.zoom_view, 1.5, "the ease settles on the rung");

        // Snapping, and a junk setting (a hand-edited save) tucked back onto
        // the ladder.
        w.zoom = 7.0;
        w.tick_zoom(1.0 / 60.0);
        assert_eq!(w.zoom, zoom::MAX);
        w.set_zoom_instant(0.62);
        assert_eq!((w.zoom, w.zoom_view), (0.5, 0.5));
    }

    #[test]
    fn the_rendered_span_is_the_viewport_times_the_zoom() {
        let mut w = World::new(Theme::Kitchen, 4);
        let logical = Vec2::new(SCREEN.0 as f32, SCREEN.1 as f32);
        // The App sets the simulation's viewport on a resize; `draw` does not.
        w.viewport = logical;
        let mut pb = screen();
        for z in zoom::STEPS {
            w.set_zoom_instant(z);
            w.draw(&mut pb);
            let span = w.view_span();
            assert_eq!(w.viewport, logical, "the logical viewport never scales");
            assert_eq!(
                w.render_viewport, logical,
                "the renderer's viewport is the unzoomed frame at every zoom"
            );
            assert!((span.x - SCREEN.0 as f32 * z).abs() < 0.01);
            assert!((span.y - SCREEN.1 as f32 * z).abs() < 0.01);
            // The screen→world scale is one world pixel per screen pixel at
            // 1:1 and the inverse of the zoom otherwise (within the rounding
            // of the rendered pixel count).
            let ratio = w.zoom_scale;
            assert!((ratio.x - z).abs() < 0.02, "zoom_scale.x at {z}: {ratio:?}");
            assert!((ratio.y - z).abs() < 0.02, "zoom_scale.y at {z}: {ratio:?}");
        }
    }

    /// Zooming out shows world the unzoomed view culls — the entire point of
    /// the feature. A gem one-and-a-bit viewports east of the camera is off
    /// screen at 1x and on screen at 2x.
    #[test]
    fn zooming_out_shows_world_the_unzoomed_view_culls() {
        let mut w = World::new(Theme::Lab, 9);
        let mut pb = screen();
        // A big map, the rat parked at its middle: the camera is not clamped.
        w.player.pos = w.level.arena * 0.5;
        let vw = SCREEN.0 as f32;
        let pos = w.player.pos + Vec2::new(vw * 0.75, 0.0);
        for (zoom, visible) in [(1.0, false), (2.0, true)] {
            w.set_zoom_instant(zoom);
            w.draw(&mut pb);
            let without_gem = pb.pixels().to_vec();
            w.spawn_gem(Gem::new(pos, 25));
            w.draw(&mut pb);
            assert_eq!(pb.pixels() != without_gem.as_slice(), visible, "zoom {zoom}");
            w.clear_gems();
        }
    }

    /// Degenerate terminals and every zoom, including a mid-transition
    /// fractional zoom: never a panic, and the buffer keeps its size.
    #[test]
    fn a_zoomed_draw_never_panics_at_any_size() {
        for (sw, sh) in [(0usize, 0usize), (1, 1), (5, 3), (2, 1), (1, 2), (7, 9)] {
            for z in zoom::STEPS {
                let mut w = World::new(Theme::Sewer, 2);
                w.set_zoom_instant(z);
                let mut pb = PixelBuffer::new(sw, sh);
                let before = w.viewport;
                assert!(before.x > 0.0 && before.y > 0.0, "a fresh world has a real viewport");
                w.draw(&mut pb);
                assert_eq!((pb.w, pb.h), (sw, sh));
                // The renderer tracks the (degenerate) frame; the simulation's
                // viewport is not the renderer's to move.
                assert_eq!(w.render_viewport, Vec2::new(sw as f32, sh as f32));
                assert_eq!(w.viewport, before);
            }
        }
        // Mid-transition (a fractional zoom, the eased frame between rungs).
        let mut w = World::new(Theme::Sewer, 2);
        let mut pb = screen();
        w.zoom = 2.0;
        w.zoom_view = 1.37;
        w.draw(&mut pb);
        assert_eq!((pb.w, pb.h), SCREEN);
        assert!(w.zoom_scale.x > 1.3 && w.zoom_scale.x < 1.4);
    }

    /// Damage numbers are 3 × 5 glyphs, so they are stamped in screen space:
    /// the same number covers the same number of pixels at every zoom (it just
    /// sits at the scaled position).
    #[test]
    fn damage_numbers_stay_text_sized_at_every_zoom() {
        let mut w = World::new(Theme::Sewer, 5);
        let mut pb = screen();
        let mut bare = screen();
        // A **kill** hit, which is never clipped: an ordinary tick inside
        // `DAMAGE_CLEAR_RADIUS` is deliberately suppressed now (the pellet
        // beside the rat outranks the number explaining the last hit), and this
        // test is about the glyph's size, so it uses one that always prints.
        let at = w.player.pos;

        let mut covered = Vec::new();
        for z in [zoom::MIN, 1.0, zoom::MAX] {
            w.set_zoom_instant(z);
            // The same frame without the number: the difference is the glyph.
            w.damage_numbers.clear();
            w.draw(&mut bare);
            w.damage_numbers.record(
                1,
                at,
                3.0,
                12.0,
                true,
                crate::game::particle::damage_number::DamageSource::Normal,
            );
            w.draw(&mut pb);
            let mut n = 0;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    if pb.pixel_at(x, y) != bare.pixel_at(x, y) {
                        n += 1;
                    }
                }
            }
            covered.push(n);
        }
        assert!(
            covered.iter().all(|&n| n > 0),
            "the number must be drawn at every zoom: {covered:?}"
        );
        assert_eq!(covered[0], covered[1], "0.5 and 1.0 cover the same glyph area");
        assert_eq!(covered[1], covered[2], "1.0 and 2.0 cover the same glyph area");
    }

    /// Draw cost at the 600-enemy cap, for the wave report. Run with
    /// `cargo test --release zoom_frame_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "perf measurement (min-of-chunks at the enemy cap), not a pass/fail test"]
    fn zoom_frame_cost_at_the_enemy_cap() {
        use rand::SeedableRng;
        use std::time::Instant;
        let mut w = World::new(Theme::Sewer, 21);
        let mut rng = StdRng::seed_from_u64(7);
        let p = w.player.pos;
        for i in 0..600 {
            let a = i as f32 * 0.37;
            let r = 20.0 + (i % 40) as f32 * 12.0;
            w.spawn_enemy_with_rng(
                crate::game::enemy::EnemyKind::Skitterer,
                p + crate::math::Vec2::from_angle(a) * r,
                10.0,
                &mut rng,
            );
        }
        println!("enemies: {}", w.enemy_count());
        // 120 × 40 cells is the task's reference frame; 160 × 168 px is the
        // viewport the ECS port measured the unzoomed draw at (385 µs).
        for (cw, ch) in [(120usize, 74usize), (160, 168)] {
            let mut pb = PixelBuffer::new(cw, ch);
            for z in [1.0f32, 2.0, 0.5] {
                w.set_zoom_instant(z);
                w.draw(&mut pb);
                let mut best = f64::INFINITY;
                for _ in 0..40 {
                    let t = Instant::now();
                    w.draw(&mut pb);
                    best = best.min(t.elapsed().as_secs_f64() * 1e6);
                }
                println!(
                    "zoom {z:>4}x @ {cw}x{ch} px ({}x{} cells): {best:>7.1} us/draw",
                    cw,
                    ch / 2,
                );
            }
        }
    }
}

#[cfg(test)]
mod hostile_layer_tests {
    //! **Hostile information wins the frame** (`WAVE3 review`, punch list 5).
    //!
    //! The enemy pellets, the wick's burst ring and the named-miniboss
    //! telegraphs are drawn after the lighting pass and after every friendly
    //! effect, so a smoke bank, a spark burst or a dark corner can never hide
    //! the thing that is about to hurt the player.

    use super::*;
    use crate::game::bullet::Bullet;
    use crate::game::ecs::components::Wick;
    use crate::game::level::Theme;

    const VIEW: (usize, usize) = (120, 120);

    fn frame(w: &mut World) -> PixelBuffer {
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut pb);
        pb
    }

    fn lum(pb: &PixelBuffer, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
            return 0;
        }
        let (r, g, b) = pb.pixel_at(x as usize, y as usize);
        u32::from(r) + u32::from(g) + u32::from(b)
    }

    /// During a blackout, an enemy pellet stays bright in Full, Hard and Off.
    /// It is stamped over the shaded scene as part of the hostile layer.
    #[test]
    fn enemy_pellets_are_not_darkened_by_lighting() {
        let mut frames = Vec::new();
        for mode in [LightingMode::Full, LightingMode::Hard, LightingMode::Off] {
            let mut w = World::new(Theme::Sewer, 7);
            w.elapsed = 304.0;
            w.player.pos = w.level.start;
            w.lighting = mode;
            w.smoke_enabled = false;
            let p = w.player.pos + Vec2::new(30.0, 0.0);
            w.spawn_bullet(Bullet::new(
                p,
                Vec2::ZERO,
                4.0,
                Faction::Enemy,
                (255, 130, 170),
            ));
            let pb = frame(&mut w);
            let s = w.w2s(p, w.cam);
            frames.push((s, pb));
        }
        let (s, full) = &frames[0];
        let (_, soft) = &frames[1];
        let (_, off) = &frames[2];
        let pellet: u32 = (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
            .map(|(dx, dy)| lum(off, s.0 + dx, s.1 + dy))
            .max()
            .unwrap_or(0);
        assert!(pellet > 300, "the pellet is not bright even unlit: {pellet}");
        for (name, other) in [("full", full), ("soft", soft)] {
            // The pellet is additive over whatever floor is beneath it, so a lit
            // floor can only make it *brighter*. Lighting must never take
            // brightness away from it: that is what "self-lit" means.
            assert!(
                lum(other, s.0, s.1) >= lum(off, s.0, s.1),
                "lighting {name} darkened the pellet: {} < {}",
                lum(other, s.0, s.1),
                lum(off, s.0, s.1)
            );
            assert!(
                lum(other, s.0, s.1) > 400,
                "lighting {name} left the pellet dim: {}",
                lum(other, s.0, s.1)
            );
        }
        // …and the pass really was lighting the scene: somewhere in the frame
        // Full is darker than Off. (This proves the pellet above is not simply
        // "unlit everywhere", which would make the assertion meaningless.)
        let dimmed = (0..VIEW.1 as i32)
            .step_by(3)
            .flat_map(|y| (0..VIEW.0 as i32).step_by(3).map(move |x| (x, y)))
            .any(|(x, y)| lum(full, x, y) + 20 < lum(off, x, y));
        assert!(dimmed, "the ambient did not shade anything: this test proves nothing");
    }

    /// A wick's burst ring lands over a full field of particles, smoke and
    /// overhangs: its ring pixels are still the drawn warning colour rather
    /// than whatever the effect layer painted there.
    #[test]
    fn the_wick_ring_is_drawn_over_friendly_effects() {
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = 304.0;
        w.player.pos = w.level.start;
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        let p = w.player.pos + Vec2::new(44.0, 10.0);
        // A wick near the end of its fuse: the merged wick art starts as a
        // faint dashed ring (alpha ~0.5 over the floor) and only tightens into
        // the solid, bright warning — the state this test is about — as the
        // fuse runs down.
        let mut wick = Wick::new(p);
        wick.fuse *= 0.1;
        crate::game::ecs::resources::spawn_with_id(&mut w.ecs, wick);
        // A friendly burst right on top of the ring: the layer order is what
        // this test is about, so the sparks are the worst case.
        w.particles.push_shockwave(crate::game::particle::Shockwave {
            pos: p,
            start_r: 2.0,
            end_r: 20.0,
            life: 0.2,
            max_life: 0.2,
            color: (255, 255, 255),
            core: (255, 255, 255),
            thick: 2.0,
            additive: true,
            arc_dir: None,
            arc_half_angle: 0.0,
        });
        let pb = frame(&mut w);
        let s = w.w2s(p, w.cam);
        let ring = crate::game::ecs::systems::enemy_wave3::WICK_BURST_RADIUS as i32;
        // Some pixel on the ring is a warm warning colour (r > b), not a spark
        // white and not the shaded floor.
        let mut found = false;
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx * dx + dy * dy > (ring + 1) * (ring + 1) {
                    continue;
                }
                let Ok((r, g, b)) = pixel(&pb, s.0 + dx, s.1 + dy) else {
                    continue;
                };
                if r > 200 && g > 140 && b < 180 && r > b + 40 {
                    found = true;
                }
            }
        }
        assert!(found, "the wick's warning ring is buried under the effects");
    }

    fn pixel(pb: &PixelBuffer, x: i32, y: i32) -> Result<(u8, u8, u8), ()> {
        if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
            return Err(());
        }
        Ok(pb.pixel_at(x as usize, y as usize))
    }
}

#[cfg(test)]
mod viewport_tests {
    //! **The simulation's viewport is session input, never a render output.**
    //!
    //! The viewport decides the spawn ring and offscreen placement, so a
    //! framebuffer resize during a draw used to let "was this frame drawn?"
    //! reach the simulation. Drawing now writes only `render_viewport`; the App
    //! sets `World::viewport` on a terminal resize and a headless run sets it
    //! once — and the two runs below must agree tick for tick.

    use super::*;
    use crate::game::level::Theme;

    /// Everything a viewport change could reach, in `SpawnId` order.
    fn trace(w: &mut World) -> String {
        let mut out = format!(
            "t={:.4} kills={} score={} gold={} pos=({:.3},{:.3}) hp={:.3} vp=({:.1},{:.1})",
            w.elapsed,
            w.kills,
            w.score,
            w.gold,
            w.player.pos.x,
            w.player.pos.y,
            w.player.hp,
            w.viewport.x,
            w.viewport.y
        );
        for e in w.enemies() {
            out.push_str(&format!(
                " |e {:?} {:.3} {:.3} {:.3} {}",
                e.kind,
                e.pos.x,
                e.pos.y,
                e.hp,
                e.id
            ));
        }
        for b in w.bullets() {
            out.push_str(&format!(" |b {:.3} {:.3}", b.pos.x, b.pos.y));
        }
        out
    }

    /// A run drawn **every frame** into a framebuffer of a different size is
    /// identical to the same run never drawn at all — and the drawn run's
    /// simulation viewport is still the 160×148 the caller set.
    #[test]
    fn a_drawn_frame_never_reaches_the_simulation() {
        let mut drawn = World::new(Theme::Sewer, 4242);
        let mut headless = World::new(Theme::Sewer, 4242);
        drawn.viewport = Vec2::new(160.0, 148.0);
        headless.viewport = drawn.viewport;
        // Deliberately a *different* size from the simulation viewport (and
        // twice, at a zoom, so `draw` covers both render paths).
        let mut pb = PixelBuffer::new(200, 120);
        for tick in 0..360 {
            let dir = Vec2::new(1.0, if tick % 60 < 30 { 0.6 } else { -0.6 }).normalized();
            drawn.update(1.0 / 60.0, dir, false);
            headless.update(1.0 / 60.0, dir, false);
            if tick % 90 == 0 {
                drawn.set_zoom_instant(if drawn.zoom > 1.0 { 1.0 } else { 2.0 });
            }
            drawn.tick_zoom(1.0 / 60.0);
            drawn.draw(&mut pb);
            assert_eq!(
                drawn.viewport, headless.viewport,
                "drawing moved the simulation's viewport on tick {tick}"
            );
            assert_eq!(
                drawn.render_viewport,
                Vec2::new(pb.w as f32, pb.h as f32),
                "the renderer's own viewport still tracks the frame it drew"
            );
        }
        assert_eq!(
            trace(&mut drawn),
            trace(&mut headless),
            "a drawn run diverged from the same run never drawn"
        );
        // Both runs actually simulated something worth comparing.
        assert!(drawn.elapsed > 5.9);
    }
}

#[cfg(test)]
mod meta_tests {
    //! The nest's run-level hooks: the Ninth Life revive, the gold tally and the
    //! fact that a fresh profile's world is the world this build always made.

    use super::*;
    use crate::game::campaign::CampaignTotals;
    use crate::game::level::Theme;
    use crate::game::loadout::Loadout;
    use crate::game::weapon::WeaponKind;

    const DT: f32 = 1.0 / 60.0;

    fn with_nest(theme: Theme, seed: u64, mods: crate::meta::RunModifiers) -> World {
        World::new_level_with_theme(
            0,
            theme,
            seed,
            Loadout::with_modifiers(Loadout::START_WEAPON, mods),
            CampaignTotals::default(),
        )
    }

    fn maxed() -> crate::meta::RunModifiers {
        crate::meta::RunModifiers::from_profile(&crate::meta::MetaProfile::maxed())
    }

    /// A fresh profile's world is untouched by the nest: 4 + 4 racks, base
    /// stats and no revive.
    #[test]
    fn a_fresh_profile_starts_the_world_it_always_did() {
        let plain = World::new(Theme::Sewer, 5);
        let explicit = with_nest(Theme::Sewer, 5, crate::meta::RunModifiers::NONE);
        assert_eq!(plain.revives_left, 0);
        assert_eq!(explicit.revives_left, 0);
        assert_eq!(plain.player.max_hp(), 130.0);
        assert_eq!(plain.player.loadout.max_weapons(), 4);
        assert_eq!(plain.player.loadout.stats.gold_mult, 1.0);
        assert_eq!(plain.player.loadout.stats.luck_mult, 1.0);
        assert_eq!(plain.player.loadout.stats.armor, 0.0);
    }

    /// The Ninth Life: the first death of a run stands the rat back up at half
    /// health with an immunity window; the second one ends the run.
    #[test]
    fn a_nest_revive_stands_the_rat_back_up_once() {
        let mut w = with_nest(Theme::Sewer, 3, maxed());
        assert_eq!(w.revives_left, 1);
        assert_eq!(w.revives_used(), 0);

        w.player.hp = -5.0;
        w.update(DT, Vec2::ZERO, false);
        assert!(!w.game_over, "the revive kept the run alive");
        assert_eq!(w.revives_left, 0);
        assert_eq!(w.revives_used(), 1);
        let half = w.player.max_hp() * 0.5;
        assert!(
            (w.player.hp - half).abs() < 0.001,
            "the revive restores half a bar: {} vs {half}",
            w.player.hp
        );
        assert!(w.player.iframes > 0.5, "the revive grants a real immunity window");

        // One a run: the second death is the end, and it draws no rng.
        w.player.iframes = 0.0;
        w.player.hp = -5.0;
        w.update(DT, Vec2::ZERO, false);
        assert!(w.game_over, "the second death ends the run");
        assert_eq!(w.revives_used(), 1);
    }

    /// The purse and the spend counter: what the report's tally reads. `spend_gold`
    /// is the only place gold leaves the purse, so it is the only place that
    /// counts.
    #[test]
    fn spent_gold_is_counted_and_a_campaign_carries_the_running_total() {
        let mut w = World::new(Theme::Sewer, 7);
        w.gold = 50;
        assert!(w.spend_gold(20));
        assert!(!w.spend_gold(1000));
        assert_eq!(w.gold, 30);
        assert_eq!(w.gold_spent, 20);

        let mut totals = CampaignTotals::default();
        totals.absorb(&w);
        assert_eq!(totals.gold, 30);
        assert_eq!(totals.gold_spent, 20);
        // The next floor starts with the carried purse and its own counter.
        let mut carried = World::new_level_with_theme(
            1,
            Theme::Kitchen,
            9,
            Loadout::with_modifiers(WeaponKind::Gnaw, maxed()),
            totals,
        );
        assert_eq!(carried.gold, 30);
        assert_eq!(carried.gold_spent, 0);
        // A revive that an earlier floor spent is not handed out again.
        carried.revives_left = 0;
        let mut totals = CampaignTotals::default();
        totals.absorb(&carried);
        assert_eq!(totals.revives_used, 1);
        let next_floor = World::new_level_with_theme(
            2,
            Theme::Lab,
            11,
            Loadout::with_modifiers(WeaponKind::Gnaw, maxed()),
            totals,
        );
        assert_eq!(next_floor.revives_left, 0, "one revive per run, not per floor");
    }

    /// The nest's stat block reaches the simulation through the loadout: max HP,
    /// armour, magnet, and the fire-rate/damage multipliers the weapon systems
    /// read every frame.
    #[test]
    fn the_nest_s_stats_are_live_on_the_player() {
        let w = with_nest(Theme::Sewer, 3, maxed());
        let stats = w.player.loadout.stats;
        assert_eq!(w.player.max_hp(), 130.0 + 48.0);
        assert_eq!(stats.armor, 2.0);
        assert_eq!(stats.magnet, 22.0 * (1.0 + 0.08 * 5.0));
        assert_eq!(stats.fire_rate_mult, 1.0 + 0.02 * 6.0);
        assert_eq!(stats.damage_mult, 1.0 + 0.025 * 6.0);
        assert_eq!(w.player.loadout.max_weapons(), 8);
    }
}

#[cfg(test)]
mod trap_order_tests {
    use super::*;
    use crate::game::art_traps::{MousetrapView, StickyTrapView};

    /// A world with one glue board (or snap trap) beside the rat, flat-lit.
    fn board_world(mousetrap: bool) -> (World, Vec2) {
        let mut w = World::new(Theme::Sewer, 5);
        w.peace = true;
        w.player.god = true;
        w.player.loadout.weapons.clear();
        w.level.objects.sticky_traps.clear();
        w.level.objects.sticky_trap_records.clear();
        w.level.objects.mousetraps.clear();
        w.level.objects.mousetrap_records.clear();
        let at = w.player.pos + Vec2::new(40.0, 0.0);
        let record = crate::game::level::TrapRecord {
            encounter: false,
            approach: crate::game::art_walls::Cardinal::East,
        };
        if mousetrap {
            w.level.objects.mousetraps.push(MousetrapView::new(at, Vec2::new(1.0, 0.0)));
            w.level.objects.mousetrap_records.push(record);
        } else {
            w.level.objects.sticky_traps.push(StickyTrapView::new(at, Vec2::new(1.0, 0.0)));
            w.level.objects.sticky_trap_records.push(record);
        }
        (w, at)
    }

    pub(super) fn dump(name: &str, pb: &PixelBuffer) {
        let Ok(dir) = std::env::var("RATRUN_TRAP_ORDER_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = String::new();
        for cy in 0..pb.h / 2 {
            for x in 0..pb.w {
                let t = pb.pixel_at(x, cy * 2);
                let b = pb.pixel_at(x, cy * 2 + 1);
                s.push_str(&format!(
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m▀",
                    t.0, t.1, t.2, b.0, b.1, b.2
                ));
            }
            s.push_str("\x1b[0m\n");
        }
        std::fs::write(format!("{dir}/{}x{}-{name}.ansi", pb.w, pb.h / 2), s).unwrap();
    }

    /// One of each behaviour archetype (the bodies every other kind is drawn
    /// as or recoloured from).
    const ARCHETYPES: [EnemyKind; 15] = [
        EnemyKind::Skitterer,
        EnemyKind::Bat,
        EnemyKind::Cat,
        EnemyKind::Spitter,
        EnemyKind::Brute,
        EnemyKind::Pillbug,
        EnemyKind::BellFlea,
        EnemyKind::WickMoth,
        EnemyKind::Porter,
        EnemyKind::Turnkey,
        EnemyKind::BigSnake,
        EnemyKind::HouseCentipede,
        EnemyKind::MurderHornet,
        EnemyKind::TermiteHorde,
        EnemyKind::Lilliputian,
    ];

    fn frame(w: &mut World) -> PixelBuffer {
        let mut pb = PixelBuffer::new(120, 76);
        w.draw(&mut pb);
        pb
    }

    /// The share of a body's opaque pixels that the trap leaves untouched.
    ///
    /// The world is drawn four times at the same instant: as it is (`a`),
    /// with the traps taken off the floor (`b`), with the floor repainted in
    /// another theme's colours (`c`), and with the bodies taken off as well
    /// (`d`). A pixel belongs to a body when `c` and `d` differ; it is
    /// *opaque* when the body covers the floor completely there (`b == c`:
    /// the floor's colour made no difference); and it is *intact* when the
    /// real frame shows that same pixel, which is only true if nothing of
    /// the trap was drawn over it.
    fn intact_share(w: &mut World, name: &str) -> f32 {
        let a = frame(w);
        dump(name, &a);
        w.level.objects.sticky_traps.clear();
        w.level.objects.mousetraps.clear();
        let b = frame(w);
        let accent = w.level.palette.accent;
        w.level.palette = crate::game::level::Palette::for_theme(Theme::Kitchen);
        w.level.palette.accent = accent;
        let c = frame(w);
        w.clear_enemies();
        let d = frame(w);
        let (mut opaque, mut intact) = (0u32, 0u32);
        for i in 0..a.pixels().len() {
            if c.pixels()[i] != d.pixels()[i] && b.pixels()[i] == c.pixels()[i] {
                opaque += 1;
                // A faint additive wash (smoke, a particle's glow) may tint a
                // body by a few levels; anything drawn *over* it changes it
                // outright.
                let (p, q) = (a.pixels()[i], b.pixels()[i]);
                let near = |x: u8, y: u8| x.abs_diff(y) <= 12;
                if near(p.0, q.0) && near(p.1, q.1) && near(p.2, q.2) {
                    intact += 1;
                }
            }
        }
        assert!(opaque >= 4, "{name}: the body drew {opaque} opaque pixels");
        intact as f32 / opaque as f32
    }

    /// A caught body is drawn over every pixel of the glue board, strands
    /// included: for each archetype, alone on a board that has just caught it.
    #[test]
    fn a_body_on_a_glue_board_is_drawn_over_the_board() {
        for kind in ARCHETYPES {
            let (mut w, at) = board_world(false);
            w.lighting = LightingMode::Off;
            w.spawn_enemy(kind, at + Vec2::new(-4.0, 2.0), 1.0);
            // Long enough for the board to catch a ground body and string it.
            for _ in 0..12 {
                w.update(1.0 / 60.0, Vec2::ZERO, false);
            }
            let share = intact_share(&mut w, &format!("glue-{}", kind.name()));
            assert!(share > 0.9, "{}: only {share:.2} of the body is over the board", kind.name());
        }
    }

    /// The same with a board holding a crowd, drawn at the far level of
    /// detail (more than `CROWD_LOD_THRESHOLD` bodies in view).
    #[test]
    fn a_crowd_on_a_glue_board_is_drawn_over_the_board_at_the_far_lod() {
        let (mut w, at) = board_world(false);
        w.lighting = LightingMode::Off;
        for i in 0..6 {
            let p = at + Vec2::new(-12.0 + 5.0 * i as f32, -6.0 + 12.0 * (i % 2) as f32);
            w.spawn_enemy([EnemyKind::Skitterer, EnemyKind::Cat, EnemyKind::Brute][i % 3], p, 1.0);
        }
        for _ in 0..12 {
            w.update(1.0 / 60.0, Vec2::ZERO, false);
        }
        // The rest of the crowd stands off the board, in view, between ticks.
        for i in 0..(crate::game::art_enemy::CROWD_LOD_THRESHOLD + 10) {
            let p = w.player.pos
                + Vec2::new(-52.0 + (i % 12) as f32 * 4.0, -30.0 + (i / 12) as f32 * 5.0);
            w.spawn_enemy(EnemyKind::Skitterer, p, 1.0);
        }
        let share = intact_share(&mut w, "glue-crowd");
        assert!(share > 0.9, "only {share:.2} of the crowd is over the board");
    }

    /// A cocked snap trap is a floor object too: a body standing on it is
    /// over the spring, the pedal, the bait and the resting bar.
    #[test]
    fn a_body_on_a_cocked_mousetrap_is_drawn_over_it() {
        for kind in ARCHETYPES {
            let (mut w, at) = board_world(true);
            w.lighting = LightingMode::Off;
            // Drawn between ticks: the trap is still armed under the body.
            w.spawn_enemy(kind, at + Vec2::new(6.0, 1.0), 1.0);
            let share = intact_share(&mut w, &format!("snap-{}", kind.name()));
            assert!(share > 0.9, "{}: only {share:.2} of the body is over the trap", kind.name());
        }
    }
}
