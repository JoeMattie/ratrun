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
//! `pending_levelups`, `game_over`, `won`, `sfx`) are **real fields** and are
//! authoritative between ticks. `World::update`:
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

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World as EcsWorld;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::Sfx;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette;

use super::bullet::Bullet;
use super::collision::SpatialGrid;
use super::director::{self, Director};
pub use super::ecs::components::Pool;
use super::fog::{Fog, MazeMarker};
use super::ecs::components::SpawnId;
use super::ecs::constants::{GRID_CELL, PARTICLE_CAP};
use super::ecs::resources::{
    collect_sorted, spawn_with_id, swap_in, swap_out, EnemyIndex, GridScratch, LastPickups,
    NeighborScratch, NextEnemyId, NextSpawnId, NukeTick, PickupScratch, RngRes, RunCounters,
    SfxQueue, Shake, TickInput, Viewport,
};
use super::ecs::schedule::{build_settle_schedule, build_tick_schedule};
use super::enemy::{Enemy, EnemyKind};
use super::gem::{Gem, Pickup, PickupKind};
use super::art_traps::MazeRewardState;
use super::level::{Level, Theme};
use super::nav::{FlowField, NavBudget};
use super::particle::damage_number::{DamageNumbers, DEFAULT_CAP as DAMAGE_NUMBER_CAP};
use super::particle::Particles;
use super::player::Player;
use super::weapon::WeaponKind;
use super::{art_enemy, art_floor, art_props, art_rat, art_traps, art_walls};

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
    pub won: bool,
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
    pub fn new(theme: Theme, seed: u64) -> World {
        // The level is generated from the run seed (same seed + theme = same
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
        ecs.insert_resource(SpatialGrid::new(level.arena, GRID_CELL));

        World {
            ecs,
            schedule: build_tick_schedule(),
            settle_schedule: build_settle_schedule(),

            rng: StdRng::seed_from_u64(seed),
            level,
            player: Player::new(center, WeaponKind::Gnaw),
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

    pub fn finished(&self) -> bool {
        self.game_over || self.won
    }

    pub fn time_left(&self) -> f32 {
        (director::RUN_SECONDS - self.elapsed).max(0.0)
    }

    // ---- Read-only HUD accessors (neon merge) ---------------------------
    //
    // The neon (07e26c9) legacy build answered these from the `Vec` caches
    // `end_tick` rebuilt. The port has no caches: they read the Bevy world
    // directly through `World::iter_entities` (`&self`, no allocation), so the
    // HUD call that follows `update` sees the tick's results and the facade
    // does no per-tick entity copying.

    /// The living boss, if one is active.
    fn living_boss(&self) -> Option<&Enemy> {
        self.ecs
            .iter_entities()
            .filter_map(|entity| entity.get::<Enemy>())
            .find(|e| e.kind == EnemyKind::Boss && e.alive())
    }

    /// Current and maximum HP of the living boss, if one is active.
    pub fn boss_hp(&self) -> Option<(f32, f32)> {
        self.living_boss().map(|e| (e.hp.max(0.0), e.max_hp))
    }

    /// World-space offset from the player to the living boss, if active.
    pub fn boss_offset(&self) -> Option<Vec2> {
        self.living_boss().map(|e| e.pos - self.player.pos)
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
        self.living_boss().map(|e| e.pos)
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
        self.ecs.insert_resource(TickInput { dt, move_dir, dash });
        self.ecs.insert_resource(Viewport(self.viewport));
        self.ecs.insert_resource(NukeTick(false));
        self.ecs.insert_resource(RunCounters {
            elapsed: self.elapsed,
            kills: self.kills,
            score: self.score,
            pending_levelups: self.pending_levelups,
            game_over: self.game_over,
            won: self.won,
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

        // Floor stains (ichor splatters & explosion scorches) beneath props/entities.
        self.particles.draw_stains(pb, cam);

        // Wall ground layer: contact shadows and floor thresholds, under
        // everything that stands on the floor.
        self.draw_wall_layer(pb, cam, WallLayer::Ground);
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
            for (_, e) in query.iter(&self.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
                let s = self.w2s(e.pos, cam);
                if !on_screen(s, e.radius * 3.0 + 8.0, viewport) {
                    continue;
                }
                art_enemy::draw_enemy(pb, e, cam, accent);
            }
        }

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

        // Player rat.
        self.draw_player(pb, cam);

        // Overhang layer: lintels over rat-holes, doorways and one-way doors,
        // so the rat is visibly ducking *under* them; then the foreground half
        // of the level objects (glue strands, striker bars, door leaves, the
        // rat-hole arch and the reward pedestal).
        self.draw_wall_layer(pb, cam, WallLayer::Overhang);
        self.draw_object_layer(pb, cam, true);

        // Particles and shockwaves on top.
        self.particles.draw(pb, cam);

        // Floating enemy damage numbers on top of everything in the playfield.
        self.damage_numbers.draw(pb, cam);
    }

    fn draw_player(&self, pb: &mut PixelBuffer, cam: Vec2) {
        art_rat::draw_rat(pb, &self.player, self.elapsed, cam);
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
    use crate::game::art_walls::{WallAxis, WallKind};
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::level::{Level, RatHole, Theme};
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
