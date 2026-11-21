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
//! Kitchen → Lab). The level index, the carried loadout and the banked totals
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
//! and only the **final** level's completion sets `won`. The five-minute clock no longer ends
//! anything: [`World::time_left`] counts down to `director::BOSS_TIME` and then
//! reports zero, and [`World::boss_time_reached`] marks the phase change.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World as EcsWorld;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::Sfx;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette;

use super::bullet::Bullet;
use super::campaign::{self, CampaignState, CampaignTotals, Keepable};
use super::collision::{SpatialGrid, TileKind, TileMap};
use super::director::{self, Director};
pub use super::ecs::components::Pool;
use super::fog::{Fog, MazeMarker};
use super::ecs::components::SpawnId;
use super::ecs::constants::{GRID_CELL, PARTICLE_CAP};
use super::ecs::resources::{
    collect_sorted, spawn_with_id, swap_in, swap_out, ActiveFx, BossDue, EnemyIndex, GridScratch,
    LastPickups, DamageLedger, LevelIndex, NeighborScratch, NextEnemyId, NextSpawnId, NukeTick,
    PickupScratch, RngRes, RunCounters, SfxQueue, Shake, TickInput, Viewport,
};
use super::ecs::schedule::{build_settle_schedule, build_tick_schedule};
use super::ecs::systems::boss::{
    Boss, BossArtView, BossFight, BossProjectile, BossProjectileKind, BossTargetable,
};
use super::ecs::systems::gates::{GateRuntime, MazeEvent};
use super::enemy::{Enemy, EnemyKind};
use super::gem::{Gem, Pickup, PickupKind};
use super::art_traps::MazeRewardState;
use super::level::{Level, RoomKind, Theme};
use super::loadout::Loadout;
use super::nav::{FlowField, NavBudget};
use super::particle::damage_number::{DamageNumbers, DEFAULT_CAP as DAMAGE_NUMBER_CAP};
use super::particle::Particles;
use super::player::Player;
use super::weapon::WeaponKind;
use super::art_walls::{Cardinal, WallKind};
use super::{
    art_boss_broom, art_boss_cat, art_boss_mower, art_enemy, art_floor, art_marks, art_props,
    art_rat, art_traps, art_walls,
};

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
    /// Explored map cells (fog of war). Revealed by
    /// `ecs::systems::fog::reveal_system`; read by the minimap between ticks.
    pub fog: Fog,
    pub elapsed: f32,
    pub kills: u32,
    pub score: u32,
    pub shake: f32,
    pub shake_ofs: Vec2,
    pub pending_levelups: u32,
    pub game_over: bool,
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
    /// The level's boss died ⇒ the level is complete (`RunCounters`
    /// `level_complete` round-trips here).
    pub level_complete: bool,
    /// Score/kills/time banked by the levels already completed, so the HUD can
    /// show cumulative numbers (`total_score`/`total_kills`/`total_time`).
    pub carried: CampaignTotals,
    pub cam: Vec2,
    pub viewport: Vec2,
    /// SFX events emitted this tick; drained by the app after `update`.
    pub sfx: Vec<Sfx>,

    // ---- Facade round-trip placeholders ----------------------------------
    rng_spare: Option<StdRng>,
    level_spare: Option<Level>,
    player_spare: Option<Player>,
    particles_spare: Option<Particles>,
    damage_numbers_spare: Option<DamageNumbers>,
    director_spare: Option<Director>,
    flow_spare: Option<FlowField>,
    fog_spare: Option<Fog>,
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
        let center = level.start;
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
        ecs.insert_resource(SpatialGrid::new(level.arena, GRID_CELL));
        // The director's boss hand-off (raised at `director::BOSS_TIME`, taken
        // by the boss framework). Permanent, like `NukeTick`.
        ecs.insert_resource(BossDue::default());
        // The boss fight's own state (private rng stream, damage-number id,
        // reusable scratch). Permanent, like `BossDue`; a spawn re-seeds it.
        ecs.insert_resource(BossFight::default());
        // Floor-trap side table (glue victims + mousetrap hit bookkeeping);
        // `traps::TrapRuntime::sync` sizes it from `level.objects`.
        ecs.insert_resource(super::ecs::systems::traps::TrapRuntime::default());
        // Interactive-object side table (one-way door latches/slam, rat-hole
        // squeeze + blocked latches, cache claims and the HUD maze feed);
        // `gates::GateRuntime::sync` sizes it from `level.objects`.
        ecs.insert_resource(super::ecs::systems::gates::GateRuntime::default());

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
            fog,
            elapsed: 0.0,
            kills: 0,
            score: 0,
            shake: 0.0,
            shake_ofs: Vec2::ZERO,
            pending_levelups: 0,
            game_over: false,
            won: false,
            level_index,
            level_complete: false,
            carried: carried_totals,
            cam: Vec2::ZERO,
            viewport: Vec2::new(160.0, 80.0),
            sfx: Vec::new(),

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
        self.elapsed >= director::BOSS_TIME
    }

    // ---- Campaign accessors --------------------------------------------

    /// Which campaign level this world simulates (0 Sewer, 1 Kitchen, 2 Lab).
    #[inline]
    pub fn level_index(&self) -> usize {
        self.level_index
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

    /// Current fight phase of the living boss (`1..=3`, 1 = healthiest).
    ///
    /// The boss framework derives it from the remaining HP fraction with the
    /// per-kind thresholds ([`BossKind::phase_for`](crate::game::ecs::systems::boss::BossKind::phase_for)):
    /// the cat and the broom have two phases, the mower three. `None` when no
    /// `Boss` entity is alive (the legacy `EnemyKind::Boss` fallback has none).
    pub fn boss_phase(&self) -> Option<u8> {
        self.living_boss().map(|b| b.phase)
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

    /// Number of active enemies in the arena.
    pub fn enemy_count(&self) -> usize {
        self.ecs
            .iter_entities()
            .filter(|entity| entity.contains::<Enemy>())
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

    /// The drawn squeak ring: `(origin, radius)` while one is expanding.
    pub fn squeak_visual(&self) -> Option<(Vec2, f32)> {
        let fx = self.ecs.resource::<ActiveFx>();
        if !fx.squeak_active() {
            return None;
        }
        Some((fx.squeak_pos, fx.squeak_r))
    }

    pub fn update(&mut self, dt: f32, move_dir: Vec2, dash: bool) {
        // Neon merge: the HUD reads only this tick's pickups.
        self.last_pickups.clear();
        if self.finished() {
            // Keep particles settling for a beat, but freeze the sim.
            self.begin_tick(dt, move_dir, dash);
            self.settle_schedule.run(&mut self.ecs);
            self.end_tick();
            return;
        }
        // Legacy `update` line 123.
        self.elapsed += dt;

        self.begin_tick(dt, move_dir, dash);
        self.schedule.run(&mut self.ecs);
        self.end_tick();
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
        self.ecs.insert_resource(TickInput {
            dt,
            move_dir,
            dash,
            active: dash,
        });
        self.ecs.insert_resource(Viewport(self.viewport));
        self.ecs.insert_resource(NukeTick(false));
        self.ecs.insert_resource(LevelIndex(self.level_index));
        self.ecs.insert_resource(RunCounters {
            elapsed: self.elapsed,
            kills: self.kills,
            score: self.score,
            pending_levelups: self.pending_levelups,
            game_over: self.game_over,
            won: self.won,
            level_complete: self.level_complete,
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
        self.pending_levelups = counters.pending_levelups;
        self.game_over = counters.game_over;
        self.won = counters.won;
        self.level_complete = counters.level_complete;

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

    fn clear_components<T: bevy_ecs::component::Component>(&mut self) {
        let mut query = self.ecs.query::<(Entity, &T)>();
        let entities: Vec<Entity> = query.iter(&self.ecs).map(|(entity, _)| entity).collect();
        for entity in entities {
            let _ = self.ecs.despawn(entity);
        }
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
            .map(|(entity, id, enemy)| (entity, *id, enemy.pos, enemy.alive()));
        super::ecs::systems::weapons::nearest_of(candidates, from)
    }

    // ---- Rendering ------------------------------------------------------

    pub fn draw(&mut self, pb: &mut PixelBuffer) {
        let vw = pb.w as f32;
        let vh = pb.h as f32;
        let viewport = Vec2::new(vw, vh);
        self.viewport = viewport;
        let arena = self.level.arena;

        // Camera follows the player, clamped to arena (centered if smaller).
        let mut cam = self.player.pos - Vec2::new(vw * 0.5, vh * 0.5);
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
        cam += self.shake_ofs;
        self.cam = cam;

        pb.clear(self.level.palette.bg);

        // Floor and arena border.
        art_floor::draw_floor(pb, &self.level, cam, self.elapsed);

        // Purely decorative floor marks (`art_marks`'s own layer order): the
        // boss arena's repeating motif, one landmark per major hall, and the
        // gnaw-mark / gold-fleck trail of each maze approach.
        self.draw_floor_marks(pb, cam);

        // Floor stains (ichor splatters & explosion scorches) beneath props/entities.
        self.particles.draw_stains(pb, cam);

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
            art_props::draw_prop(pb, p, cam, &self.level.palette);
        }

        // Acid pools, in `SpawnId` order (overlapping sprites must draw in
        // spawn order, exactly like the legacy `Vec`).
        {
            let mut query = self.ecs.query::<(&SpawnId, &Pool)>();
            for (_, p) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(p.pos, cam);
                if !on_screen(s, p.radius + 2.0, viewport) {
                    continue;
                }
                let glow = palette::scale(p.color, (p.life / 2.6).clamp(0.2, 1.0));
                let r = p.radius as i32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy <= r * r && (dx + dy) % 2 == 0 {
                            pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(glow, 0.4));
                        }
                    }
                }
            }
        }

        // Gems.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Gem)>();
            for (_, g) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(g.pos, cam);
                if !on_screen(s, 3.0, viewport) {
                    continue;
                }
                pb.plot(s.0, s.1, g.color);
                pb.plot_add(s.0 + 1, s.1, palette::scale(g.color, 0.5));
                pb.plot_add(s.0, s.1 + 1, palette::scale(g.color, 0.5));
            }
        }

        // Pickups with a pulsing additive glow halo.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Pickup)>();
            for (_, p) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(p.pos, cam);
                let pulse = 0.5 + 0.5 * (p.bob.sin() * 0.5 + 0.5); // 0.5..1.0
                let base = p.kind.color();
                let gr = (7.0 + pulse * 4.0) as i32;
                let grf = gr as f32;
                if !on_screen(s, grf + 2.0, viewport) {
                    continue;
                }
                for dy in -gr..=gr {
                    for dx in -gr..=gr {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        if d <= grf {
                            let fall = (1.0 - d / grf).powi(2);
                            pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(base, fall * pulse * 0.7));
                        }
                    }
                }
                pb.filled_circle(s.0, s.1, 2, palette::scale(base, 0.7 + pulse * 0.3));
                pb.plot(s.0, s.1, (255, 255, 255));
            }
        }

        // Enemies (animated insects). Bugs overhang their radius (legs, wings,
        // health ring), so cull with a generous margin.
        let accent = self.level.palette.accent;
        {
            let mut query = self.ecs.query::<(&SpawnId, &Enemy)>();
            // Count what is on screen first: past CROWD_LOD_THRESHOLD the art
            // switches small enemies to its cheaper level-of-detail drawing.
            let visible = query
                .iter(&self.ecs)
                .filter(|(_, e)| on_screen(self.w2s(e.pos, cam), e.radius * 3.0 + 8.0, viewport))
                .count();
            for (_, e) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(e.pos, cam);
                if !on_screen(s, e.radius * 3.0 + 8.0, viewport) {
                    continue;
                }
                art_enemy::draw_enemy_crowd(pb, e, cam, accent, visible);
            }
        }

        // Boss bodies in the entity pass (airborne ones go over the player).
        self.draw_boss_body(pb, cam, false);

        // Bullets.
        {
            let mut query = self.ecs.query::<(&SpawnId, &Bullet)>();
            for (_, b) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(b.pos, cam);
                if b.lob.is_some() {
                    // Lobbed acid: bright core plus a fading drip trail behind it.
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

        // Boss-owned projectiles (blade discs, shrapnel) in the bullet pass.
        self.draw_boss_projectiles(pb, cam);

        // Player rat.
        self.draw_player(pb, cam);

        // The cat's airborne pounce passes visibly overhead.
        self.draw_boss_body(pb, cam, true);

        // Overhang layer: lintels over rat-holes, doorways and one-way doors,
        // so the rat is visibly ducking *under* them; then the foreground half
        // of the level objects (glue strands, striker bars, door leaves, the
        // rat-hole arch and the reward pedestal).
        self.draw_wall_layer(pb, cam, WallLayer::Overhang);
        self.draw_boss_thresholds(pb, cam, WallLayer::Overhang);
        self.draw_object_layer(pb, cam, true);

        // Particles and shockwaves on top.
        self.particles.draw(pb, cam);

        // Floating enemy damage numbers on top of everything in the playfield.
        self.damage_numbers.draw(pb, cam);
    }

    fn draw_player(&self, pb: &mut PixelBuffer, cam: Vec2) {
        // The Tail Thrash's long tail rides *under* the rat, so the body stays
        // readable mid-sweep and the arc reads as coming out of its rump.
        if self.thrash_visual().is_some() {
            art_rat::draw_tail_thrash(pb, &self.player, self.thrash_visual(), cam);
        }
        art_rat::draw_rat(pb, &self.player, self.elapsed, cam);
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
            let body_visible = on_screen(self.w2s(boss.pos, cam), margin, self.viewport);
            let target_visible = on_screen(self.w2s(boss.target, cam), margin, self.viewport);
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
            }
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
            if !on_screen(self.w2s(boss.pos, cam), boss.kind.cull_margin(), self.viewport) {
                continue;
            }
            match boss.view() {
                BossArtView::Cat(v) => art_boss_cat::draw_boss_cat(pb, &v, cam, accent),
                BossArtView::Broom(v) => art_boss_broom::draw(pb, &v, cam, accent),
                BossArtView::Mower(v) => art_boss_mower::draw_mower(pb, &v, cam, accent),
            }
        }
    }

    /// Boss-owned projectiles in the bullet pass (blade discs, shrapnel).
    fn draw_boss_projectiles(&mut self, pb: &mut PixelBuffer, cam: Vec2) {
        let accent = self.level.palette.accent;
        let mut query = self.ecs.query::<(&SpawnId, &BossProjectile)>();
        for (_, p) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            if !on_screen(self.w2s(p.pos, cam), p.radius + 4.0, self.viewport) {
                continue;
            }
            match p.kind {
                BossProjectileKind::BladeDisc => {
                    art_boss_mower::draw_blade_disc(pb, p.pos, p.anim, cam, accent)
                }
                BossProjectileKind::GrassShrapnel => {
                    art_boss_mower::draw_grass_shrapnel(pb, p.pos, p.vel, cam, accent)
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
        let vp = self.viewport;
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
            .visible_range(cam, self.viewport, WALL_DRAW_MARGIN);
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
    fn draw_object_layer(&self, pb: &mut PixelBuffer, cam: Vec2, foreground: bool) {
        let accent = self.level.palette.accent;
        let vp = self.viewport;
        let anim = self.elapsed;
        let objects = &self.level.objects;

        for view in &objects.sticky_traps {
            if !on_screen(self.w2s(view.pos, cam), 16.0, vp) {
                continue;
            }
            let mut view = *view;
            view.anim = anim;
            if foreground {
                art_traps::draw_sticky_trap(pb, &view, cam, accent);
            } else {
                art_traps::draw_sticky_trap_ground(pb, &view, cam, accent);
            }
        }
        for view in &objects.mousetraps {
            if !on_screen(self.w2s(view.pos, cam), 16.0, vp) {
                continue;
            }
            let mut view = *view;
            view.anim = anim;
            if foreground {
                art_traps::draw_mousetrap(pb, &view, cam, accent);
            } else {
                art_traps::draw_mousetrap_ground(pb, &view, cam, accent);
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
        for reward in &objects.rewards {
            if !on_screen(self.w2s(reward.pos, cam), 22.0, vp) {
                continue;
            }
            let mut view = *reward;
            view.anim = anim;
            if foreground {
                art_traps::draw_maze_reward(pb, &view, cam, accent);
            } else {
                art_traps::draw_maze_reward_ground(pb, &view, cam, accent);
            }
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

/// Extra cells to draw around the viewport: wall art casts shadows up to 4 px
/// south/east and the overhang layer reaches a few pixels up.
const WALL_DRAW_MARGIN: i32 = 1;

// ---- Prop rendering -------------------------------------------------------

/// Cheap viewport test: can a point with `margin` pixels of drawn extent still
/// touch a framebuffer of `viewport` size? (Screen-space, origin top-left.)
#[inline]
pub(crate) fn on_screen(s: (i32, i32), margin: f32, viewport: Vec2) -> bool {
    let x = s.0 as f32;
    let y = s.1 as f32;
    x + margin >= 0.0 && y + margin >= 0.0 && x - margin < viewport.x && y - margin < viewport.y
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

    /// The detector the threshold overlay hangs off has to find real mouths:
    /// every generated map has a boss arena whose double door pierces its wall
    /// ring, and the inward direction points from the corridor into the arena.
    #[test]
    fn every_generated_map_has_a_boss_arena_mouth() {
        for (theme, seed) in [
            (Theme::Sewer, 0u64),
            (Theme::Sewer, 1),
            (Theme::Kitchen, 2),
            (Theme::Kitchen, 5),
            (Theme::Lab, 3),
            (Theme::Lab, 9),
        ] {
            let w = World::new(theme, seed);
            let arena = w
                .level
                .boss_room()
                .expect("every generated map has a boss arena")
                .rect;
            let ring = arena.inflate(1);
            let mut mouths = 0;
            let mut bad = None;
            w.for_each_boss_threshold(|view| {
                mouths += 1;
                let (cx, cy) = (view.cell.cell_x, view.cell.cell_y);
                let on_ring = ring.contains(cx, cy) && !arena.contains(cx, cy);
                let points_in = match view.inward {
                    Cardinal::North => cy == arena.bottom(),
                    Cardinal::South => cy == arena.y - 1,
                    Cardinal::East => cx == arena.x - 1,
                    Cardinal::West => cx == arena.right(),
                };
                if !on_ring || !points_in {
                    bad = Some((cx, cy, view.inward));
                }
            });
            assert!(
                mouths >= 1,
                "{theme:?}/{seed}: the boss arena mouth was not found"
            );
            assert_eq!(bad, None, "{theme:?}/{seed}: mouth outside the wall ring");
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
