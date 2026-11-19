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
use super::level::{Level, Theme};
use super::nav::FlowField;
use super::particle::damage_number::{DamageNumbers, DEFAULT_CAP as DAMAGE_NUMBER_CAP};
use super::particle::Particles;
use super::player::Player;
use super::weapon::WeaponKind;
use super::{art_enemy, art_floor, art_props, art_rat};

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
}

impl World {
    pub fn new(theme: Theme, seed: u64) -> World {
        let level = Level::new(theme);
        let center = level.arena * 0.5;
        let mut flow = FlowField::new(&level.nav);
        flow.update(&level.nav, center);
        let flow_spare = FlowField::new(&level.nav);

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
            level_spare: Some(Level::new(theme)),
            player_spare: Some(Player::new(center, WeaponKind::Gnaw)),
            particles_spare: Some(Particles::new(PARTICLE_CAP)),
            damage_numbers_spare: Some(DamageNumbers::new(DAMAGE_NUMBER_CAP)),
            director_spare: Some(Director::default()),
            flow_spare: Some(flow_spare),
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

        // Particles and shockwaves on top.
        self.particles.draw(pb, cam);

        // Floating enemy damage numbers on top of everything in the playfield.
        self.damage_numbers.draw(pb, cam);
    }

    fn draw_player(&self, pb: &mut PixelBuffer, cam: Vec2) {
        art_rat::draw_rat(pb, &self.player, self.elapsed, cam);
    }

    #[inline]
    fn w2s(&self, p: Vec2, cam: Vec2) -> (i32, i32) {
        ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
    }
}

// ---- Prop rendering -------------------------------------------------------

/// Cheap viewport test: can a point with `margin` pixels of drawn extent still
/// touch a framebuffer of `viewport` size? (Screen-space, origin top-left.)
#[inline]
pub(crate) fn on_screen(s: (i32, i32), margin: f32, viewport: Vec2) -> bool {
    let x = s.0 as f32;
    let y = s.1 as f32;
    x + margin >= 0.0 && y + margin >= 0.0 && x - margin < viewport.x && y - margin < viewport.y
}
