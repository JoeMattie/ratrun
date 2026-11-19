//! The simulation: owns every entity, advances one tick, resolves
//! collisions, runs the spawn director, and draws the world into a
//! half-block pixel framebuffer.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::audio::Sfx;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

use super::{art_enemy, art_floor, art_props, art_rat};
use super::bullet::{Bullet, Faction, Lob, Orbit};
use super::collision::{self, SpatialGrid};
use super::director::{self, Director};
use super::enemy::{Enemy, EnemyKind};
use super::gem::{Gem, Pickup, PickupKind};
use super::level::{Level, Theme};
use super::nav::FlowField;
use super::particle::{Particles, Ramp};
use super::player::Player;
use super::weapon::WeaponKind;

const PLAYER_RADIUS: f32 = 3.0;
/// Max distance the player moves per collision sub-step; must stay well
/// under `PLAYER_RADIUS` so a fast tick cannot tunnel through a thin wall.
const PLAYER_SUBSTEP: f32 = 2.0;
const PARTICLE_CAP: usize = 6000;
const ENEMY_CAP: usize = 600;
const GRID_CELL: f32 = 16.0;

pub struct Pool {
    pub pos: Vec2,
    pub radius: f32,
    pub dps: f32,
    pub life: f32,
    pub color: Rgb,
}

pub struct World {
    pub rng: StdRng,
    pub level: Level,
    pub player: Player,
    pub enemies: Vec<Enemy>,
    pub bullets: Vec<Bullet>,
    pub gems: Vec<Gem>,
    pub pickups: Vec<Pickup>,
    pub pools: Vec<Pool>,
    pub particles: Particles,
    pub director: Director,
    pub flow: FlowField,
    /// Reused broadphase grid (cleared + refilled each tick, never reallocated
    /// per frame). `neigh` is the shared neighbour scratch buffer.
    grid: SpatialGrid,
    neigh: Vec<u32>,
    /// Reused scratch list of pickups collected this tick (no per-frame alloc).
    pickup_scratch: Vec<PickupKind>,
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
}

impl World {
    pub fn new(theme: Theme, seed: u64) -> World {
        let level = Level::new(theme);
        let center = level.arena * 0.5;
        let mut flow = FlowField::new(&level.nav);
        flow.update(&level.nav, center);
        let grid = SpatialGrid::new(level.arena, GRID_CELL);
        World {
            rng: StdRng::seed_from_u64(seed),
            flow,
            grid,
            neigh: Vec::new(),
            pickup_scratch: Vec::new(),
            level,
            player: Player::new(center, WeaponKind::Gnaw),
            enemies: Vec::new(),
            bullets: Vec::new(),
            gems: Vec::new(),
            pickups: Vec::new(),
            pools: Vec::new(),
            particles: Particles::new(PARTICLE_CAP),
            director: Director::default(),
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
        }
    }

    pub fn finished(&self) -> bool {
        self.game_over || self.won
    }

    pub fn time_left(&self) -> f32 {
        (director::RUN_SECONDS - self.elapsed).max(0.0)
    }

    pub fn update(&mut self, dt: f32, move_dir: Vec2, dash: bool) {
        if self.finished() {
            // Keep particles settling for a beat, but freeze the sim.
            self.particles.update(dt);
            return;
        }
        self.elapsed += dt;

        self.update_player(dt, move_dir, dash);
        // Refresh the shared flow field toward the player (cheap: only
        // recomputes when the player crosses into a new nav cell).
        self.flow.update(&self.level.nav, self.player.pos);
        self.run_director(dt);
        self.update_enemies(dt);

        // Reuse the persistent grid: take it out of `self` so enemy/bullet
        // field borrows stay disjoint, then put it back when done.
        let mut grid = std::mem::take(&mut self.grid);
        grid.reset(self.level.arena, GRID_CELL);
        grid.fill(self.enemies.iter().map(|e| e.pos));
        let max_radius = self.enemies.iter().map(|e| e.radius).fold(0.0, f32::max);
        self.separate_enemies(&grid, max_radius);
        self.resolve_enemy_walls();

        // Separation and wall ejection move enemies, so refresh the grid from
        // their final positions before the bullet queries.
        grid.reset(self.level.arena, GRID_CELL);
        grid.fill(self.enemies.iter().map(|e| e.pos));

        self.fire_weapons(dt);
        self.update_bullets(dt);
        self.update_pools(dt);
        self.resolve_player_bullets(&grid, max_radius);
        self.grid = grid;
        self.resolve_enemy_hits();
        self.resolve_contact();
        self.reap_enemies();
        self.update_gems(dt);
        // A Nuke sets enemies to hp = -1 during the pickup pass; reap them in
        // the same tick so their kills/score count and they cannot linger into
        // the next (possibly final) tick.
        if self.update_pickups(dt) {
            self.reap_enemies();
        }
        self.particles.update(dt);

        // Screen-shake decay + jitter offset.
        if self.shake > 0.05 {
            self.shake_ofs = Vec2::new(
                self.rng.random_range(-1.0..1.0) * self.shake,
                self.rng.random_range(-1.0..1.0) * self.shake,
            );
            self.shake = (self.shake - dt * 30.0).max(0.0);
        } else {
            self.shake_ofs = Vec2::ZERO;
        }

        // Defeat takes precedence: winning and dying can happen on the same
        // tick when the clock runs out, and that run is a loss.
        if !self.player.alive() {
            self.game_over = true;
        } else if self.elapsed >= director::RUN_SECONDS {
            self.won = true;
        }
    }

    fn update_player(&mut self, dt: f32, move_dir: Vec2, dash: bool) {
        let stats = self.player.loadout.stats.clone();
        if move_dir.len_sq() > 0.01 {
            self.player.face = move_dir.normalized();
        }

        // Dash.
        if dash && self.player.dash_cd <= 0.0 {
            self.player.dash_active = 0.16;
            self.player.dash_cd = stats.dash_cd;
            self.player.iframes = self.player.iframes.max(0.18);
            let pos = self.player.pos;
            let face = self.player.face;
            self.particles
                .cone(&mut self.rng, pos, -face, 16, 90.0, Ramp::Smoke, 0.4);
            self.sfx.push(Sfx::Dash);
        }

        let dashing = self.player.dash_active > 0.0;
        let dash_boost = if dashing { 3.2 } else { 1.0 };
        // `move_dir` is a throttle vector: unit length = full speed (keyboard),
        // shorter = proportionally slower (mouse easing near the cursor).
        // A dash with no held direction launches along the facing instead of
        // spending the cooldown in place.
        let throttle = if dashing && move_dir.len_sq() <= 0.01 {
            self.player.face
        } else {
            move_dir.clamp_len(1.0)
        };
        let target_vel = throttle * stats.move_speed * dash_boost;
        self.player.vel += (target_vel - self.player.vel) * (12.0 * dt).min(1.0);

        // Move in sub-steps shorter than the player radius. A dash at upgraded
        // speed can cover ~12 px per tick, which would otherwise skip straight
        // over a thin pipe and let nearest-face ejection pop the player out the
        // far side.
        let delta = self.player.vel * dt;
        let steps = ((delta.len() / PLAYER_SUBSTEP).ceil() as i32).clamp(1, 64);
        let step = delta * (1.0 / steps as f32);
        let arena = self.level.arena;
        let lo = Vec2::splat(PLAYER_RADIUS);
        let hi = Vec2::new(arena.x - PLAYER_RADIUS, arena.y - PLAYER_RADIUS);
        for _ in 0..steps {
            self.player.pos += step;
            self.player.pos = self.player.pos.clamp_box(lo, hi);
            for w in &self.level.walls {
                self.player.pos = collision::resolve_circle_rect(self.player.pos, PLAYER_RADIUS, w);
            }
        }

        // Timers.
        self.player.dash_cd = (self.player.dash_cd - dt).max(0.0);
        self.player.dash_active = (self.player.dash_active - dt).max(0.0);
        self.player.iframes = (self.player.iframes - dt).max(0.0);
        self.player.hurt_flash = (self.player.hurt_flash - dt).max(0.0);
        if stats.regen > 0.0 {
            self.player.heal(stats.regen * dt);
        }
    }

    fn run_director(&mut self, dt: f32) {
        // Keep one slot reserved for the mandatory boss until it has spawned,
        // so `enemies.len() <= ENEMY_CAP` holds for the entire run.
        let cap = if self.director.boss_spawned {
            ENEMY_CAP
        } else {
            ENEMY_CAP - 1
        };

        self.director.spawn_timer -= dt;
        if self.director.spawn_timer <= 0.0 {
            self.director.spawn_timer = director::spawn_interval(self.elapsed);
            let count = director::pulse_count(self.elapsed);
            let mult = director::hp_mult(self.elapsed);
            for _ in 0..count {
                if self.enemies.len() >= cap {
                    break;
                }
                let kind = director::pick_kind(self.elapsed, &mut self.rng);
                let pos = self.spawn_point();
                self.enemies
                    .push(Enemy::spawn(kind, pos, mult, &mut self.rng));
            }
        }

        // Periodic elite brute. Skip it instead of taking the boss's slot.
        self.director.elite_timer -= dt;
        if self.director.elite_timer <= 0.0 && self.elapsed > director::ELITE_START {
            self.director.elite_timer = director::ELITE_INTERVAL;
            if self.enemies.len() < cap {
                let pos = self.spawn_point();
                let mult = director::hp_mult(self.elapsed) * 1.6;
                self.enemies
                    .push(Enemy::spawn(EnemyKind::Brute, pos, mult, &mut self.rng));
            }
        }

        // Boss.
        if !self.director.boss_spawned && self.elapsed >= director::BOSS_TIME {
            self.director.boss_spawned = true;
            let pos = self.spawn_point();
            self.enemies
                .push(Enemy::spawn(EnemyKind::Boss, pos, 1.0, &mut self.rng));
            self.shake = 14.0;
            self.sfx.push(Sfx::Boss);
        }
    }

    /// A point on a ring around the player, just beyond view. Rejection
    /// samples so the point stays inside the arena margin, far enough from the
    /// player, and clear of level walls; clamping is only a last resort.
    fn spawn_point(&mut self) -> Vec2 {
        const ATTEMPTS: i32 = 12;
        const MARGIN: f32 = 8.0;
        const MIN_DIST: f32 = 90.0;
        const WALL_CLEAR: f32 = 4.0;

        let r = (self.viewport.len() * 0.5 + 16.0).max(120.0);
        let base = self.rng.random_range(0.0..std::f32::consts::TAU);
        let lo = Vec2::splat(MARGIN);
        let hi = self.level.arena - Vec2::splat(MARGIN);
        for k in 0..ATTEMPTS {
            let a = base + k as f32 / ATTEMPTS as f32 * std::f32::consts::TAU;
            let p = self.player.pos + Vec2::from_angle(a) * r;
            if self.spawn_point_ok(p, lo, hi, MIN_DIST, WALL_CLEAR) {
                return p;
            }
        }
        // Mirror of the first angle, as a cheap second sweep.
        let p = self.player.pos + Vec2::from_angle(-base) * r;
        if self.spawn_point_ok(p, lo, hi, MIN_DIST, WALL_CLEAR) {
            return p;
        }
        // Last resort: clamp the original ring point into the arena.
        (self.player.pos + Vec2::from_angle(base) * r)
            .clamp_box(Vec2::splat(6.0), self.level.arena - Vec2::splat(6.0))
    }

    fn spawn_point_ok(&self, p: Vec2, lo: Vec2, hi: Vec2, min_dist: f32, wall_clear: f32) -> bool {
        if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y {
            return false;
        }
        if p.dist_sq(self.player.pos) < min_dist * min_dist {
            return false;
        }
        for w in &self.level.walls {
            let cx = p.x.clamp(w.x, w.right());
            let cy = p.y.clamp(w.y, w.bottom());
            if (p.x - cx) * (p.x - cx) + (p.y - cy) * (p.y - cy) < wall_clear * wall_clear {
                return false;
            }
        }
        true
    }

    fn update_enemies(&mut self, dt: f32) {
        let ppos = self.player.pos;
        let speed_mult = 1.0 + self.elapsed / 400.0;
        for i in 0..self.enemies.len() {
            let flow_dir = self.flow.dir_at(self.enemies[i].pos);
            self.enemies[i].steer(ppos, flow_dir, dt, speed_mult);
            // Keep enemies inside the arena.
            self.enemies[i].pos = self.enemies[i]
                .pos
                .clamp_box(Vec2::splat(2.0), self.level.arena - Vec2::splat(2.0));

            // Ranged enemies shoot.
            if self.enemies[i].kind.ranged() {
                self.enemies[i].fire_cd -= dt;
                let dist = self.enemies[i].pos.dist(ppos);
                if self.enemies[i].fire_cd <= 0.0 && dist < 170.0 {
                    let is_boss = self.enemies[i].kind == EnemyKind::Boss;
                    self.enemies[i].fire_cd = if is_boss { 1.2 } else { 2.2 };
                    let from = self.enemies[i].pos;
                    let dmg = self.enemies[i].damage * 0.6;
                    let color = self.level.palette.accent;
                    if is_boss {
                        for k in 0..10 {
                            let a = k as f32 / 10.0 * std::f32::consts::TAU;
                            let v = Vec2::from_angle(a) * 60.0;
                            let mut b = Bullet::new(from, v, dmg, Faction::Enemy, (255, 120, 160));
                            b.life = 5.0;
                            b.size = 2;
                            self.bullets.push(b);
                        }
                    } else {
                        let v = (ppos - from).normalized() * 75.0;
                        let mut b = Bullet::new(from, v, dmg, Faction::Enemy, color);
                        b.life = 4.0;
                        self.bullets.push(b);
                    }
                }
            }
        }
    }

    fn separate_enemies(&mut self, grid: &SpatialGrid, max_radius: f32) {
        // Reuse the shared scratch vec (mem::take keeps the other `self`
        // fields independently borrowable).
        let mut neigh = std::mem::take(&mut self.neigh);
        // The widest possible overlapping pair spans two largest radii.
        let reach = max_radius * 2.0;
        for i in 0..self.enemies.len() {
            let pi = self.enemies[i].pos;
            let ri = self.enemies[i].radius;
            grid.neighbors_within(pi, reach, &mut neigh);
            let mut push = Vec2::ZERO;
            for &j in &neigh {
                let j = j as usize;
                if j == i {
                    continue;
                }
                let pj = self.enemies[j].pos;
                let rj = self.enemies[j].radius;
                let sum = ri + rj;
                let d2 = pi.dist_sq(pj);
                if d2 < sum * sum {
                    if d2 > 1e-6 {
                        let d = d2.sqrt();
                        push += (pi - pj) * ((sum - d) / d) * 0.5;
                    } else {
                        // Perfectly stacked: nudge apart deterministically by
                        // index order so they never stay glued together.
                        let dir = if i < j { -1.0 } else { 1.0 };
                        push += Vec2::new(dir * sum * 0.5, 0.0);
                    }
                }
            }
            self.enemies[i].pos += push.clamp_len(6.0);
        }
        self.neigh = neigh;
    }

    /// Push enemies out of interior walls (final position constraint, after
    /// steering + separation) so they path around obstacles instead of through.
    fn resolve_enemy_walls(&mut self) {
        let arena = self.level.arena;
        for i in 0..self.enemies.len() {
            let r = self.enemies[i].radius;
            for w in &self.level.walls {
                self.enemies[i].pos = collision::resolve_circle_rect(self.enemies[i].pos, r, w);
            }
            self.enemies[i].pos = self.enemies[i]
                .pos
                .clamp_box(Vec2::splat(2.0), arena - Vec2::splat(2.0));
        }
    }

    fn nearest_enemy(&self, from: Vec2) -> Option<(usize, Vec2)> {
        self.enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alive())
            .min_by(|(_, a), (_, b)| a.pos.dist_sq(from).total_cmp(&b.pos.dist_sq(from)))
            .map(|(i, e)| (i, e.pos))
    }

    fn fire_weapons(&mut self, dt: f32) {
        let ppos = self.player.pos;
        let pface = self.player.face;
        let fire_rate = self.player.loadout.stats.fire_rate_mult;
        let dmg_mult = self.player.loadout.stats.damage_mult;
        let n = self.player.loadout.weapons.len();
        for i in 0..n {
            let (kind, dmg, cd, projs) = {
                let w = &self.player.loadout.weapons[i];
                (
                    w.kind,
                    w.damage() * dmg_mult,
                    (w.cooldown() / fire_rate).max(0.05),
                    w.level_projectiles(),
                )
            };
            self.player.loadout.weapons[i].timer -= dt;
            if self.player.loadout.weapons[i].timer > 0.0 {
                continue;
            }
            let fired = self.fire_one(kind, dmg, cd, projs, ppos, pface);
            if fired {
                self.player.loadout.weapons[i].timer += cd;
            } else {
                // Nothing spawned (e.g. Gnaw with no live target): retry next
                // tick instead of serving a full cooldown for a dud shot.
                self.player.loadout.weapons[i].timer = 0.0;
            }
        }
    }

    fn fire_one(
        &mut self,
        kind: WeaponKind,
        dmg: f32,
        cd: f32,
        projs: i32,
        ppos: Vec2,
        pface: Vec2,
    ) -> bool {
        let color = kind.color();
        let mut fired = false;
        match kind {
            WeaponKind::Gnaw => {
                if let Some((_, epos)) = self.nearest_enemy(ppos) {
                    let dir = (epos - ppos).normalized();
                    let mut b = Bullet::new(ppos, dir * 120.0, dmg, Faction::Player, color);
                    b.size = 1;
                    self.bullets.push(b);
                    self.particles
                        .cone(&mut self.rng, ppos, dir, 4, 60.0, Ramp::Spark, 0.2);
                    fired = true;
                }
            }
            WeaponKind::CheeseSpray => {
                let aim = self
                    .nearest_enemy(ppos)
                    .map(|(_, p)| (p - ppos).normalized())
                    .unwrap_or(pface);
                let base = aim.angle();
                for k in 0..projs {
                    let spread = (k as f32 - (projs - 1) as f32 / 2.0) * 0.18;
                    let v = Vec2::from_angle(base + spread) * 110.0;
                    let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                    b.life = 1.4;
                    self.bullets.push(b);
                }
                self.particles
                    .cone(&mut self.rng, ppos, aim, 6, 70.0, Ramp::Fire, 0.25);
                fired = true;
            }
            WeaponKind::SqueakNova => {
                for k in 0..projs {
                    let a = k as f32 / projs as f32 * std::f32::consts::TAU;
                    let v = Vec2::from_angle(a) * 95.0;
                    let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                    b.life = 1.3;
                    self.bullets.push(b);
                }
                self.particles
                    .burst(&mut self.rng, ppos, 12, 80.0, 1.0, Ramp::Spark, 0.35, true);
                fired = true;
            }
            WeaponKind::SporeOrbit => {
                let radius = 24.0;
                // Carry the ring's current angle forward, then drop the old
                // ring before spawning its replacement so two never stack.
                let base = self
                    .bullets
                    .iter()
                    .find_map(|b| b.orbit.map(|o| o.angle))
                    .unwrap_or(0.0);
                self.bullets.retain(|b| b.orbit.is_none());
                for k in 0..projs {
                    let angle = base + k as f32 / projs as f32 * std::f32::consts::TAU;
                    let mut b = Bullet::new(ppos, Vec2::ZERO, dmg, Faction::Player, color);
                    b.life = cd;
                    b.pierce = 999;
                    b.size = 2;
                    b.radius = 3.0;
                    b.orbit = Some(Orbit {
                        angle,
                        radius,
                        speed: 3.0,
                    });
                    self.bullets.push(b);
                }
                fired = true;
            }
            WeaponKind::TailWhip => {
                let radius = 26.0 + projs as f32 * 3.0;
                for e in self.enemies.iter_mut() {
                    if e.pos.dist(ppos) <= radius + e.radius {
                        e.hurt(dmg);
                    }
                }
                // Visual pulse.
                let steps = 28;
                for k in 0..steps {
                    let a = k as f32 / steps as f32 * std::f32::consts::TAU;
                    let p = ppos + Vec2::from_angle(a) * radius;
                    self.particles.burst(
                        &mut self.rng,
                        p,
                        1,
                        18.0,
                        0.5,
                        Ramp::Custom(color, (120, 40, 90)),
                        0.3,
                        true,
                    );
                }
                fired = true;
            }
            WeaponKind::AcidSpit => {
                // Lob a projectile: it does no damage in flight and only
                // becomes a pool once it lands (target, wall, or timeout).
                let target = self
                    .nearest_enemy(ppos)
                    .map(|(_, p)| p)
                    .unwrap_or(ppos + pface * 50.0)
                    .clamp_box(Vec2::splat(4.0), self.level.arena - Vec2::splat(4.0));
                let to = target - ppos;
                let speed = 110.0;
                let mut b = Bullet::new(ppos, to.normalized() * speed, dmg, Faction::Player, color);
                b.size = 2;
                b.radius = 3.0;
                b.life = to.len() / speed + 0.5;
                b.lob = Some(Lob {
                    target,
                    radius: 12.0 + projs as f32 * 1.5,
                    dps: dmg * 4.0,
                    life: 2.6,
                    color: (140, 220, 70),
                });
                self.bullets.push(b);
                fired = true;
            }
        }
        // Only announce a shot when the weapon actually produced something
        // (Gnaw with no target in range fires nothing).
        if fired {
            self.sfx.push(Sfx::Shoot);
        }
        fired
    }

    fn update_bullets(&mut self, dt: f32) {
        let ppos = self.player.pos;
        let arena = self.level.arena;
        for b in self.bullets.iter_mut() {
            b.prev_pos = b.pos;
            if let Some(o) = b.orbit.as_mut() {
                o.angle += o.speed * dt;
                b.pos = ppos + Vec2::from_angle(o.angle) * o.radius;
            } else {
                b.pos += b.vel * dt;
            }
            b.life -= dt;
            b.hit_cd = (b.hit_cd - dt).max(0.0);

            // Lobbed shots land on the first wall along this tick's sweep,
            // when they reach the target, or when they time out.
            if let Some(lob) = b.lob {
                let step = b.pos - b.prev_pos;
                let step_len = step.len();
                let mut land: Option<Vec2> = None;
                let mut best_t = f32::INFINITY;
                for w in &self.level.walls {
                    if let Some(t) = collision::segment_rect_hit(b.prev_pos, b.pos, b.radius, w) {
                        if t < best_t {
                            best_t = t;
                            land = Some(b.prev_pos + step * t);
                        }
                    }
                }
                if land.is_none() && step_len >= (lob.target - b.prev_pos).len() - 0.01 {
                    land = Some(lob.target);
                }
                if land.is_none() && b.life <= 0.0 {
                    land = Some(b.pos);
                }
                if let Some(p) = land {
                    self.pools.push(Pool {
                        pos: p,
                        radius: lob.radius,
                        dps: lob.dps,
                        life: lob.life,
                        color: lob.color,
                    });
                    self.particles.burst(
                        &mut self.rng,
                        p,
                        14,
                        30.0,
                        2.0,
                        Ramp::Custom((180, 255, 90), (40, 90, 20)),
                        0.5,
                        false,
                    );
                    b.life = 0.0;
                }
            }
        }
        self.bullets.retain(|b| {
            if b.life <= 0.0 {
                return false;
            }
            // Orbit bullets deliberately sit outside the arena edge when the
            // player hugs a wall; only cull free-flying projectiles.
            if b.orbit.is_some() {
                return true;
            }
            b.pos.x > -12.0
                && b.pos.y > -12.0
                && b.pos.x < arena.x + 12.0
                && b.pos.y < arena.y + 12.0
        });
    }

    fn update_pools(&mut self, dt: f32) {
        for p in self.pools.iter_mut() {
            p.life -= dt;
            for e in self.enemies.iter_mut() {
                if e.pos.dist(p.pos) <= p.radius + e.radius {
                    e.hp -= p.dps * dt;
                    e.flash = e.flash.max(0.05);
                }
            }
        }
        self.pools.retain(|p| p.life > 0.0);
    }

    fn resolve_player_bullets(&mut self, grid: &SpatialGrid, max_enemy_radius: f32) {
        let mut neigh = std::mem::take(&mut self.neigh);
        let mut hit_any = false;
        for b in self.bullets.iter_mut() {
            if b.faction != Faction::Player || b.hit_cd > 0.0 || b.lob.is_some() {
                continue;
            }
            // Cover the whole swept path (not just the endpoint) plus the
            // largest enemy radius, so fast bullets cannot skip a target or
            // miss an overlap across a cell boundary.
            let mid = (b.prev_pos + b.pos) * 0.5;
            let half_sweep = (b.pos - b.prev_pos).len() * 0.5;
            grid.neighbors_within(
                mid,
                b.radius + max_enemy_radius + half_sweep,
                &mut neigh,
            );
            for &j in &neigh {
                let e = &mut self.enemies[j as usize];
                if !e.alive() {
                    continue;
                }
                if collision::segment_circle_hit(b.prev_pos, b.pos, b.radius, e.pos, e.radius) {
                    e.hurt(b.damage);
                    hit_any = true;
                    self.particles.burst(
                        &mut self.rng,
                        b.pos,
                        4,
                        50.0,
                        1.0,
                        Ramp::Spark,
                        0.18,
                        true,
                    );
                    if b.pierce > 0 {
                        b.pierce -= 1;
                        b.hit_cd = 0.12;
                    } else {
                        b.life = 0.0;
                    }
                    break;
                }
            }
        }
        self.neigh = neigh;
        if hit_any {
            self.sfx.push(Sfx::Hit);
        }
    }

    fn resolve_enemy_hits(&mut self) {
        let ppos = self.player.pos;
        let mut hit = false;
        for b in self.bullets.iter_mut() {
            if b.faction != Faction::Enemy {
                continue;
            }
            if collision::segment_circle_hit(b.prev_pos, b.pos, b.radius, ppos, PLAYER_RADIUS)
                && self.player.take_damage(b.damage)
            {
                hit = true;
                // Consume the bullet only when the hit landed; an i-framed
                // player must not delete projectiles for free.
                b.life = 0.0;
            }
        }
        if hit {
            let p = self.player.pos;
            self.particles.burst(
                &mut self.rng,
                p,
                10,
                40.0,
                1.0,
                Ramp::Custom((255, 90, 90), (90, 20, 20)),
                0.4,
                false,
            );
            self.shake = self.shake.max(4.0);
            self.sfx.push(Sfx::Hurt);
        }
    }

    fn resolve_contact(&mut self) {
        let ppos = self.player.pos;
        // `take_damage` returns false while i-frames are up. Skip the loop
        // entirely in that case so enemies do not spend their contact cooldown
        // (or get knocked back) for a hit that cannot land.
        if self.player.iframes > 0.0 {
            return;
        }
        let mut hit_dmg = 0.0f32;
        for e in self.enemies.iter_mut() {
            // An enemy killed earlier this tick (by a bullet or a pool) must
            // not still land contact damage or knockback.
            if !e.alive() {
                continue;
            }
            if e.contact_cd <= 0.0
                && collision::circles_overlap(ppos, PLAYER_RADIUS, e.pos, e.radius)
            {
                hit_dmg = hit_dmg.max(e.damage);
                e.contact_cd = 0.6;
                // Knockback.
                e.pos += (e.pos - ppos).normalized() * 4.0;
                // Wall resolution already ran this tick, so re-resolve the
                // knocked-back enemy to avoid a frame inside a prop.
                for w in &self.level.walls {
                    e.pos = collision::resolve_circle_rect(e.pos, e.radius, w);
                }
            }
        }
        if hit_dmg > 0.0 && self.player.take_damage(hit_dmg) {
            let p = self.player.pos;
            self.particles.burst(
                &mut self.rng,
                p,
                12,
                45.0,
                1.0,
                Ramp::Custom((255, 90, 90), (90, 20, 20)),
                0.4,
                false,
            );
            self.shake = self.shake.max(5.0);
            self.sfx.push(Sfx::Hurt);
        }
    }

    fn reap_enemies(&mut self) {
        let mut i = 0;
        while i < self.enemies.len() {
            if !self.enemies[i].alive() {
                let e = self.enemies.swap_remove(i);
                let blood = self.level.palette.blood;
                let big = matches!(e.kind, EnemyKind::Brute | EnemyKind::Boss);
                let n = if e.kind == EnemyKind::Boss {
                    120
                } else if big {
                    28
                } else {
                    10
                };
                self.particles.burst(
                    &mut self.rng,
                    e.pos,
                    n,
                    if big { 90.0 } else { 55.0 },
                    e.radius,
                    Ramp::Custom(blood, palette::scale(blood, 0.25)),
                    0.6,
                    false,
                );
                if big {
                    self.shake = self
                        .shake
                        .max(if e.kind == EnemyKind::Boss { 16.0 } else { 6.0 });
                    self.sfx.push(Sfx::Explosion);
                } else {
                    self.sfx.push(Sfx::Kill);
                }
                // Drop XP gem(s).
                self.gems.push(Gem::new(e.pos, e.kind.xp()));
                // Rare pickup drops.
                let drop_chance = if e.kind == EnemyKind::Boss {
                    1.0
                } else if big {
                    0.12
                } else {
                    0.012
                };
                if self.rng.random::<f32>() < drop_chance {
                    let kind = match self.rng.random_range(0..3) {
                        0 => PickupKind::Heal,
                        1 => PickupKind::Magnet,
                        _ => PickupKind::Nuke,
                    };
                    self.pickups.push(Pickup {
                        pos: e.pos,
                        kind,
                        bob: 0.0,
                    });
                }
                self.kills += 1;
                self.score += e.kind.xp() * 10;
            } else {
                i += 1;
            }
        }
    }

    fn update_gems(&mut self, dt: f32) {
        let ppos = self.player.pos;
        let magnet = self.player.loadout.stats.magnet;
        let mut gained = 0u32;
        let collect_r = PLAYER_RADIUS + 3.0;
        self.gems.retain_mut(|g| {
            let to = ppos - g.pos;
            let dist = to.len();
            if g.magnetized || dist < magnet {
                g.magnetized = true;
                // Steer velocity toward the player (instead of accumulating
                // acceleration forever) and speed up as the gem gets closer.
                let closeness = (1.0 - dist / magnet.max(1.0)).clamp(0.0, 1.0);
                let speed = 90.0 + closeness * 150.0;
                let desired = to.normalized() * speed;
                g.vel += (desired - g.vel) * (12.0 * dt).min(1.0);
            }
            g.pos += g.vel * dt;
            // Collect on the post-move distance, so a gem that just arrived
            // this tick is picked up rather than waiting an extra frame.
            if g.pos.dist_sq(ppos) < collect_r * collect_r {
                gained += g.value;
                false
            } else {
                true
            }
        });
        if gained > 0 {
            let p = self.player.pos;
            self.particles
                .burst(&mut self.rng, p, 4, 30.0, 1.0, Ramp::Gem, 0.25, true);
            self.sfx.push(Sfx::Gem);
            let ups = self.player.add_xp(gained);
            if ups > 0 {
                self.sfx.push(Sfx::LevelUp);
            }
            self.pending_levelups += ups;
        }
    }

    /// Returns true when a Nuke was applied this tick (its kills still need
    /// reaping).
    fn update_pickups(&mut self, dt: f32) -> bool {
        let ppos = self.player.pos;
        let mut nuked = false;
        let mut collected = std::mem::take(&mut self.pickup_scratch);
        collected.clear();
        self.pickups.retain_mut(|p| {
            p.bob += dt * 4.0;
            if p.pos.dist(ppos) < PLAYER_RADIUS + 5.0 {
                collected.push(p.kind);
                false
            } else {
                true
            }
        });
        for &k in &collected {
            self.sfx.push(Sfx::Pickup);
            match k {
                PickupKind::Heal => {
                    let h = self.player.max_hp() * 0.35;
                    self.player.heal(h);
                }
                PickupKind::Magnet => {
                    for g in self.gems.iter_mut() {
                        g.magnetized = true;
                    }
                }
                PickupKind::Nuke => {
                    nuked = true;
                    self.shake = self.shake.max(14.0);
                    for e in self.enemies.iter_mut() {
                        if e.kind != EnemyKind::Boss {
                            e.hp = -1.0;
                        } else {
                            e.hurt(200.0);
                        }
                    }
                    let p = self.player.pos;
                    self.particles
                        .burst(&mut self.rng, p, 80, 140.0, 2.0, Ramp::Fire, 0.7, true);
                    self.sfx.push(Sfx::Explosion);
                }
            }
        }
        collected.clear();
        self.pickup_scratch = collected;
        nuked
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

        let pal = &self.level.palette;
        pb.clear(pal.bg);

        // Floor and arena border.
        art_floor::draw_floor(pb, &self.level, cam, self.elapsed);

        // Props (themed obstacles): skip any rect that cannot touch the view.
        for p in &self.level.props {
            let x = (p.rect.x - cam.x).round() as i32;
            let y = (p.rect.y - cam.y).round() as i32;
            let margin = p.rect.w.max(p.rect.h) + 2.0;
            if !on_screen((x, y), margin, viewport) {
                continue;
            }
            art_props::draw_prop(pb, p, cam, pal);
        }

        // Acid pools.
        for p in &self.pools {
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

        // Gems.
        for g in &self.gems {
            let s = self.w2s(g.pos, cam);
            if !on_screen(s, 3.0, viewport) {
                continue;
            }
            pb.plot(s.0, s.1, g.color);
            pb.plot_add(s.0 + 1, s.1, palette::scale(g.color, 0.5));
            pb.plot_add(s.0, s.1 + 1, palette::scale(g.color, 0.5));
        }

        // Pickups with a pulsing additive glow halo.
        for p in &self.pickups {
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

        // Enemies (animated insects). Bugs overhang their radius (legs, wings,
        // health ring), so cull with a generous margin.
        for e in &self.enemies {
            let s = self.w2s(e.pos, cam);
            if !on_screen(s, e.radius * 3.0 + 8.0, viewport) {
                continue;
            }
            art_enemy::draw_enemy(pb, e, cam, self.level.palette.accent);
        }

        // Bullets.
        for b in &self.bullets {
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

        // Player rat.
        self.draw_player(pb, cam);

        // Particles on top.
        self.particles.draw(pb, cam);
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
fn on_screen(s: (i32, i32), margin: f32, viewport: Vec2) -> bool {
    let x = s.0 as f32;
    let y = s.1 as f32;
    x + margin >= 0.0 && y + margin >= 0.0 && x - margin < viewport.x && y - margin < viewport.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::gem::{Gem, Pickup, PickupKind};
    use crate::game::level::Rectf;

    fn apply_pending(w: &mut World) {
        while w.pending_levelups > 0 {
            let choices = w.player.loadout.generate_choices(&mut w.rng);
            if let Some(up) = choices.first().cloned() {
                let heal = w.player.loadout.apply(&up);
                w.player.heal(heal);
            }
            w.pending_levelups -= 1;
        }
    }

    /// Drive a long simulation across all themes, drawing every few frames at
    /// several viewport sizes, to catch panics in the update + render paths.
    #[test]
    fn headless_run_does_not_panic() {
        for theme in Theme::all() {
            let mut w = World::new(theme, 12345);
            let dt = 1.0 / 60.0;
            let sizes = [(120usize, 60usize), (1, 1), (240, 120)];
            for tick in 0..6000u32 {
                let dir = Vec2::from_angle(tick as f32 * 0.05);
                let dash = tick % 90 == 0;
                w.update(dt, dir, dash);
                apply_pending(&mut w);
                if tick % 7 == 0 {
                    let (sw, sh) = sizes[(tick as usize / 7) % sizes.len()];
                    let mut pb = PixelBuffer::new(sw, sh);
                    w.draw(&mut pb);
                }
                if w.finished() {
                    break;
                }
            }
            // Invariants.
            assert!(w.enemies.len() <= ENEMY_CAP);
            assert!(w.particles.len() <= PARTICLE_CAP);
        }
    }

    /// Manual screenshot tool: line up one of each insect + a glowing pickup
    /// in a zoomable frame. `RATRUN_DUMP=bugs cargo test dump_bestiary`.
    #[test]
    fn dump_bestiary() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("bugs") {
            return;
        }
        let mut w = World::new(Theme::Sewer, 1);
        let cx = w.level.arena.x * 0.5;
        let cy = w.level.arena.y * 0.5;
        w.player.pos = Vec2::new(cx, cy + 42.0);
        let mut rng = StdRng::seed_from_u64(3);
        let kinds = [
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
            EnemyKind::Boss,
        ];
        for (i, k) in kinds.iter().enumerate() {
            let mut e = Enemy::spawn(
                *k,
                Vec2::new(cx - 78.0 + i as f32 * 30.0, cy),
                1.0,
                &mut rng,
            );
            e.facing = Vec2::new(0.25, 1.0).normalized();
            e.anim = i as f32 * 0.9;
            w.enemies.push(e);
        }
        w.pickups.push(Pickup {
            pos: Vec2::new(cx - 30.0, cy + 22.0),
            kind: PickupKind::Heal,
            bob: 1.2,
        });
        w.pickups.push(Pickup {
            pos: Vec2::new(cx + 30.0, cy + 22.0),
            kind: PickupKind::Nuke,
            bob: 0.3,
        });
        let mut pb = PixelBuffer::new(200, 120);
        w.draw(&mut pb);
        // Write PPM (top fg + bottom bg per cell handled by caller's converter;
        // here we dump the raw pixel grid directly).
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                out.push(c.0);
                out.push(c.1);
                out.push(c.2);
            }
        }
        std::fs::write("/tmp/ratrun_bugs.ppm", out).unwrap();
    }

    #[test]
    fn enemies_are_pushed_out_of_walls() {
        let mut w = World::new(Theme::Sewer, 5);
        let wall = w.level.walls[0];
        let mut rng = StdRng::seed_from_u64(9);
        // Spawn an enemy buried in the middle of a wall.
        w.enemies
            .push(Enemy::spawn(EnemyKind::Cat, wall.center(), 1.0, &mut rng));
        w.update(1.0 / 60.0, Vec2::ZERO, false);
        let c = w.enemies[0].pos;
        assert!(!wall.contains(c), "enemy center should be outside the wall");
    }

    /// After a long run, no enemy should ever sit inside a wall.
    #[test]
    fn run_keeps_enemies_out_of_walls() {
        for theme in Theme::all() {
            let mut w = World::new(theme, 21);
            for tick in 0..1800u32 {
                let dir = Vec2::from_angle(tick as f32 * 0.05);
                w.update(1.0 / 60.0, dir, false);
                apply_pending(&mut w);
            }
            for e in &w.enemies {
                for wall in &w.level.walls {
                    assert!(
                        !wall.contains(e.pos),
                        "{:?} enemy inside a wall at {:?}",
                        theme,
                        e.pos
                    );
                }
            }
        }
    }

    /// Screenshot tool for the themed props. `RATRUN_DUMP=props cargo test dump_props`.
    #[test]
    fn dump_props() {
        let theme = match std::env::var("RATRUN_DUMP").as_deref() {
            Ok("props") => Theme::Kitchen,
            Ok("props_sewer") => Theme::Sewer,
            Ok("props_lab") => Theme::Lab,
            _ => return,
        };
        let mut w = World::new(theme, 1);
        w.player.pos = w.level.arena * 0.5;
        // Full-arena overview.
        let mut pb = PixelBuffer::new(w.level.arena.x as usize, w.level.arena.y as usize);
        w.draw(&mut pb);
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                out.push(c.0);
                out.push(c.1);
                out.push(c.2);
            }
        }
        std::fs::write("/tmp/ratrun_props.ppm", out).unwrap();
    }

    /// Exercise the boss + every pickup/pool/gem draw branch in one frame.
    #[test]
    fn draw_all_entity_kinds() {
        let mut w = World::new(Theme::Lab, 99);
        let c = w.level.arena * 0.5;
        let mut rng = StdRng::seed_from_u64(1);
        w.enemies.push(Enemy::spawn(
            EnemyKind::Boss,
            c + Vec2::new(30.0, 0.0),
            1.0,
            &mut rng,
        ));
        for k in [
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
        ] {
            w.enemies.push(Enemy::spawn(
                k,
                c + Vec2::from_angle(0.3) * 20.0,
                1.0,
                &mut rng,
            ));
        }
        w.gems.push(Gem::new(c, 1));
        w.gems.push(Gem::new(c, 30));
        for kind in [PickupKind::Heal, PickupKind::Magnet, PickupKind::Nuke] {
            w.pickups.push(Pickup {
                pos: c,
                kind,
                bob: 0.5,
            });
        }
        w.pools.push(Pool {
            pos: c,
            radius: 14.0,
            dps: 5.0,
            life: 2.0,
            color: (140, 220, 70),
        });
        let mut b = Bullet::new(c, Vec2::new(10.0, 0.0), 3.0, Faction::Enemy, (255, 0, 0));
        b.size = 2;
        w.bullets.push(b);

        let mut pb = PixelBuffer::new(100, 50);
        w.draw(&mut pb);
    }

    // ---- Review-fix regression tests -----------------------------------

    #[test]
    fn dead_enemies_deal_no_contact_damage() {
        let mut w = World::new(Theme::Sewer, 12);
        w.enemies.clear();
        let mut rng = StdRng::seed_from_u64(1);
        let mut e = Enemy::spawn(EnemyKind::Cat, w.player.pos, 1.0, &mut rng);
        e.hp = -1.0;
        e.contact_cd = 0.0;
        w.enemies.push(e);
        let hp = w.player.hp;
        w.resolve_contact();
        assert_eq!(w.player.hp, hp, "a dead enemy still dealt contact damage");
        assert_eq!(w.enemies[0].contact_cd, 0.0);
    }

    #[test]
    fn loss_beats_win_on_the_final_tick() {
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = director::RUN_SECONDS - 0.001;
        w.player.hp = -1.0;
        w.update(1.0 / 60.0, Vec2::ZERO, false);
        assert!(w.game_over, "death on the final tick should be a loss");
        assert!(!w.won, "death must not also score a win");
    }

    #[test]
    fn win_is_set_only_when_alive_at_the_end_of_the_tick() {
        let mut w = World::new(Theme::Sewer, 8);
        w.elapsed = director::RUN_SECONDS - 0.001;
        w.player.hp = w.player.max_hp();
        w.update(1.0 / 60.0, Vec2::ZERO, false);
        assert!(w.won, "surviving the full clock should win");
        assert!(!w.game_over);
    }

    #[test]
    fn nuke_kills_are_reaped_on_the_same_tick() {
        let mut w = World::new(Theme::Sewer, 47);
        w.director.spawn_timer = 1.0e9;
        w.director.elite_timer = 1.0e9;
        w.enemies.clear();
        let mut rng = StdRng::seed_from_u64(6);
        let mut e = Enemy::spawn(EnemyKind::Cat, w.player.pos, 1.0, &mut rng);
        e.hp = 1.0e6;
        e.max_hp = 1.0e6;
        w.enemies.push(e);
        w.pickups.push(Pickup {
            pos: w.player.pos,
            kind: PickupKind::Nuke,
            bob: 0.0,
        });
        w.update(1.0 / 60.0, Vec2::ZERO, false);
        assert!(w.enemies.is_empty(), "a Nuke kill survived the tick");
        assert_eq!(w.kills, 1);
    }

    #[test]
    fn spawn_point_stays_in_bounds_clear_of_walls_and_player() {
        let mut w = World::new(Theme::Sewer, 13);
        let center = w.level.arena * 0.5;
        w.player.pos = center;
        w.level.walls = vec![Rectf::new(center.x + 110.0, center.y - 20.0, 40.0, 40.0)];
        for _ in 0..300 {
            let p = w.spawn_point();
            assert!(
                p.x >= 8.0 - 1e-3 && p.y >= 8.0 - 1e-3,
                "spawn {p:?} past min margin"
            );
            assert!(
                p.x <= w.level.arena.x - 8.0 + 1e-3 && p.y <= w.level.arena.y - 8.0 + 1e-3,
                "spawn {p:?} past max margin"
            );
            assert!(
                p.dist(center) >= 90.0 - 1e-3,
                "spawn {p:?} too close to player"
            );
            let wall = w.level.walls[0];
            let cx = p.x.clamp(wall.x, wall.right());
            let cy = p.y.clamp(wall.y, wall.bottom());
            let d = ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt();
            assert!(d >= 4.0 - 1e-3, "spawn {p:?} overlaps a wall");
        }
    }

    #[test]
    fn enemy_cap_holds_through_a_full_run() {
        let mut w = World::new(Theme::Sewer, 17);
        let dt = 0.1;
        let mut ticks = 0;
        while !w.finished() && ticks < 4000 {
            w.update(dt, Vec2::ZERO, false);
            apply_pending(&mut w);
            // Keep the rat alive so the run actually reaches the boss + win.
            w.player.hp = w.player.max_hp();
            assert!(
                w.enemies.len() <= ENEMY_CAP,
                "cap exceeded at {:.1}s: {}",
                w.elapsed,
                w.enemies.len()
            );
            ticks += 1;
        }
        assert!(w.director.boss_spawned, "boss never spawned");
        assert!(
            w.elapsed >= director::RUN_SECONDS,
            "run did not reach the clock"
        );
    }

    #[test]
    fn bullet_query_reaches_large_enemies_across_cells() {
        let mut w = World::new(Theme::Lab, 19);
        w.enemies.clear();
        w.bullets.clear();
        let mut rng = StdRng::seed_from_u64(2);
        // 15.9 (cell 0) and 32.5 (cell 2) are 16.6 px apart, which overlaps a
        // radius-3 bullet with a radius-14 boss but spans two cells.
        let boss = Enemy::spawn(EnemyKind::Boss, Vec2::new(32.5, 100.0), 1.0, &mut rng);
        let boss_hp = boss.hp;
        w.enemies.push(boss);
        let mut b = Bullet::new(
            Vec2::new(15.9, 100.0),
            Vec2::ZERO,
            7.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.radius = 3.0;
        w.bullets.push(b);
        let mut grid = SpatialGrid::new(w.level.arena, GRID_CELL);
        grid.fill(w.enemies.iter().map(|e| e.pos));
        w.resolve_player_bullets(&grid, 14.0);
        assert!(
            w.enemies[0].hp < boss_hp,
            "wide bullet query missed the boss across cells"
        );
    }

    #[test]
    fn fast_player_cannot_tunnel_a_thin_wall() {
        let mut w = World::new(Theme::Sewer, 23);
        w.level.walls = vec![Rectf::new(512.0, 300.0, 1.0, 200.0)];
        w.player.pos = Vec2::new(500.0, 400.0);
        w.player.vel = Vec2::new(900.0, 0.0);
        w.update_player(1.0 / 60.0, Vec2::ZERO, false);
        assert!(
            w.player.pos.x <= 512.0 - PLAYER_RADIUS + 0.01,
            "player tunnelled to {:?}",
            w.player.pos
        );
    }

    #[test]
    fn spore_orbit_replaces_its_ring_and_keeps_its_angle() {
        let mut w = World::new(Theme::Sewer, 4);
        w.enemies.clear();
        w.bullets.clear();
        let ppos = w.player.pos;
        let face = w.player.face;
        w.fire_one(WeaponKind::SporeOrbit, 5.0, 1.0, 4, ppos, face);
        assert_eq!(w.bullets.len(), 4);
        for b in w.bullets.iter_mut() {
            if let Some(o) = b.orbit.as_mut() {
                o.angle += 0.7;
            }
        }
        let carried = w.bullets[0].orbit.unwrap().angle;
        w.fire_one(WeaponKind::SporeOrbit, 5.0, 1.0, 4, ppos, face);
        let orbits: Vec<_> = w.bullets.iter().filter_map(|b| b.orbit).collect();
        assert_eq!(orbits.len(), 4, "two spore rings stacked");
        assert!(
            (orbits[0].angle - carried).abs() < 1e-4,
            "ring angle snapped instead of carrying forward"
        );
    }

    #[test]
    fn untargeted_gnaw_does_not_emit_a_shoot_sfx() {
        let mut w = World::new(Theme::Sewer, 2);
        w.enemies.clear();
        w.bullets.clear();
        w.sfx.clear();
        let ppos = w.player.pos;
        let face = w.player.face;
        w.fire_one(WeaponKind::Gnaw, 1.0, 0.5, 1, ppos, face);
        assert!(w.bullets.is_empty());
        assert!(!w.sfx.contains(&Sfx::Shoot));
    }

    #[test]
    fn nearest_enemy_skips_dead_and_nan_positions() {
        let mut w = World::new(Theme::Sewer, 29);
        w.enemies.clear();
        let mut rng = StdRng::seed_from_u64(3);
        let from = w.player.pos;
        let mut dead = Enemy::spawn(EnemyKind::Skitterer, from, 1.0, &mut rng);
        dead.hp = 0.0;
        w.enemies.push(dead);
        let mut nan = Enemy::spawn(EnemyKind::Skitterer, from, 1.0, &mut rng);
        nan.pos = Vec2::new(f32::NAN, f32::NAN);
        w.enemies.push(nan);
        let alive_pos = from + Vec2::new(10.0, 0.0);
        w.enemies
            .push(Enemy::spawn(EnemyKind::Skitterer, alive_pos, 1.0, &mut rng));
        let (i, p) = w.nearest_enemy(from).expect("should pick the live enemy");
        assert_eq!(i, 2);
        assert_eq!(p, alive_pos);
    }

    #[test]
    fn on_screen_culls_offscreen_entities() {
        let vp = Vec2::new(100.0, 50.0);
        assert!(on_screen((10, 10), 2.0, vp));
        assert!(
            on_screen((-2, 10), 4.0, vp),
            "an overhanging edge is visible"
        );
        assert!(!on_screen((-10, 10), 2.0, vp));
        assert!(!on_screen((120, 10), 2.0, vp));
        assert!(!on_screen((10, 60), 3.0, vp));
    }

    #[test]
    fn orbit_bullets_survive_the_edge_cull() {
        let mut w = World::new(Theme::Sewer, 31);
        w.player.pos = Vec2::new(50.0, 50.0);
        w.bullets.clear();
        let mut orbit = Bullet::new(
            w.player.pos,
            Vec2::ZERO,
            1.0,
            Faction::Player,
            (255, 255, 255),
        );
        orbit.orbit = Some(Orbit {
            angle: 0.0,
            radius: w.level.arena.x + 20.0,
            speed: 0.0,
        });
        w.bullets.push(orbit);
        let free = Bullet::new(
            Vec2::new(w.level.arena.x + 50.0, 50.0),
            Vec2::ZERO,
            1.0,
            Faction::Player,
            (255, 255, 255),
        );
        w.bullets.push(free);
        w.update_bullets(1.0 / 60.0);
        assert_eq!(w.bullets.len(), 1, "free bullet should be culled");
        assert!(
            w.bullets[0].orbit.is_some(),
            "orbit bullet was culled at the edge"
        );
    }

    #[test]
    fn dash_without_direction_uses_facing() {
        let mut w = World::new(Theme::Sewer, 37);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 0.0;
        let x0 = w.player.pos.x;
        w.update_player(1.0 / 60.0, Vec2::ZERO, true);
        assert!(w.player.pos.x > x0 + 0.1, "dash with no input did not move");
        assert!(w.player.dash_cd > 0.0);
    }

    #[test]
    fn magnetized_gem_reaches_the_player() {
        let mut w = World::new(Theme::Sewer, 53);
        w.gems.clear();
        w.player.pos = Vec2::new(200.0, 200.0);
        let mut g = Gem::new(w.player.pos + Vec2::new(100.0, 0.0), 5);
        g.magnetized = true;
        w.gems.push(g);
        let xp0 = w.player.xp;
        for _ in 0..300 {
            w.update_gems(1.0 / 60.0);
            if w.gems.is_empty() {
                break;
            }
        }
        assert!(w.gems.is_empty(), "magnetized gem was never collected");
        assert!(w.player.xp > xp0);
    }

    #[test]
    fn exactly_stacked_enemies_are_nudged_apart() {
        let mut w = World::new(Theme::Sewer, 41);
        w.enemies.clear();
        let mut rng = StdRng::seed_from_u64(4);
        let p = w.level.arena * 0.5;
        w.enemies
            .push(Enemy::spawn(EnemyKind::Cat, p, 1.0, &mut rng));
        w.enemies
            .push(Enemy::spawn(EnemyKind::Cat, p, 1.0, &mut rng));
        let mut grid = SpatialGrid::new(w.level.arena, GRID_CELL);
        grid.fill(w.enemies.iter().map(|e| e.pos));
        w.separate_enemies(&grid, 6.5);
        assert!(
            w.enemies[0].pos.dist(w.enemies[1].pos) > 0.1,
            "stacked enemies stayed glued together"
        );
    }

    #[test]
    fn contact_knockback_stays_out_of_walls() {
        let mut w = World::new(Theme::Sewer, 43);
        w.level.walls = vec![Rectf::new(100.0, 90.0, 10.0, 20.0)];
        w.player.pos = Vec2::new(94.0, 100.0);
        w.enemies.clear();
        let mut rng = StdRng::seed_from_u64(5);
        let mut e = Enemy::spawn(EnemyKind::Cat, Vec2::new(96.5, 100.0), 1.0, &mut rng);
        e.contact_cd = 0.0;
        w.enemies.push(e);
        w.resolve_contact();
        let wall = w.level.walls[0];
        assert!(
            !wall.contains(w.enemies[0].pos),
            "knockback left the enemy inside a wall at {:?}",
            w.enemies[0].pos
        );
    }

    #[test]
    fn untargeted_gnaw_retries_next_tick() {
        let mut w = World::new(Theme::Sewer, 61);
        w.enemies.clear();
        w.bullets.clear();
        w.player.loadout.weapons[0].timer = 0.0;
        w.fire_weapons(1.0 / 60.0);
        assert!(w.bullets.is_empty(), "Gnaw fired with no target");
        assert_eq!(
            w.player.loadout.weapons[0].timer, 0.0,
            "a dud shot should leave the timer ready for next tick"
        );
    }

    #[test]
    fn iframed_player_keeps_enemy_bullets() {
        let mut w = World::new(Theme::Sewer, 67);
        w.bullets.clear();
        w.player.pos = w.level.arena * 0.5;
        w.bullets.push(Bullet::new(
            w.player.pos,
            Vec2::ZERO,
            5.0,
            Faction::Enemy,
            (255, 0, 0),
        ));
        w.player.iframes = 0.5;
        let hp = w.player.hp;
        w.resolve_enemy_hits();
        assert_eq!(w.player.hp, hp, "i-frames should block the damage");
        assert!(w.bullets[0].life > 0.0, "i-frames must not delete the bullet");

        w.player.iframes = 0.0;
        w.resolve_enemy_hits();
        assert!(w.player.hp < hp, "a vulnerable player should take the hit");
        assert_eq!(w.bullets[0].life, 0.0, "a landed hit should consume the bullet");
    }

    #[test]
    fn iframed_player_does_not_spend_contact_cooldowns() {
        let mut w = World::new(Theme::Sewer, 71);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.enemies.clear();
        let mut rng = StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(EnemyKind::Cat, w.player.pos, 1.0, &mut rng);
        e.contact_cd = 0.0;
        w.enemies.push(e);
        w.player.iframes = 0.5;
        let hp = w.player.hp;
        let pos = w.enemies[0].pos;
        w.resolve_contact();
        assert_eq!(w.player.hp, hp);
        assert_eq!(w.enemies[0].contact_cd, 0.0, "i-frames spent the cooldown");
        assert_eq!(w.enemies[0].pos, pos, "i-frames applied knockback");

        w.player.iframes = 0.0;
        w.resolve_contact();
        assert!(w.player.hp < hp, "vulnerable contact should damage");
        assert!(w.enemies[0].contact_cd > 0.0, "a landed hit should set cooldown");
    }

    // ---- Round 3 review fixes -------------------------------------------

    #[test]
    fn fast_bullet_through_a_skitterer_registers_a_hit() {
        let mut w = World::new(Theme::Sewer, 89);
        w.enemies.clear();
        w.bullets.clear();
        let mut rng = StdRng::seed_from_u64(8);
        let e = Enemy::spawn(EnemyKind::Skitterer, Vec2::new(86.0, 100.0), 1.0, &mut rng);
        let hp = e.hp;
        w.enemies.push(e);
        let mut b = Bullet::new(
            Vec2::new(80.0, 100.0),
            Vec2::new(240.0, 0.0),
            3.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.radius = 2.0;
        w.bullets.push(b);
        w.update_bullets(0.05);
        // Endpoint-only collision misses: the bullet ends 6 px past center,
        // outside the 4.5 px combined hit radius.
        assert!(!collision::circles_overlap(
            w.bullets[0].pos,
            2.0,
            w.enemies[0].pos,
            2.5
        ));
        let mut grid = SpatialGrid::new(w.level.arena, GRID_CELL);
        grid.fill(w.enemies.iter().map(|e| e.pos));
        w.resolve_player_bullets(&grid, 2.5);
        assert!(w.enemies[0].hp < hp, "fast bullet passed through the enemy");
        assert_eq!(w.bullets[0].life, 0.0);
    }

    #[test]
    fn fast_enemy_bullet_cannot_skip_the_player() {
        let mut w = World::new(Theme::Sewer, 91);
        w.bullets.clear();
        w.player.pos = Vec2::new(100.0, 100.0);
        w.player.iframes = 0.0;
        let hp = w.player.hp;
        let mut b = Bullet::new(
            Vec2::new(88.0, 100.0),
            Vec2::new(400.0, 0.0),
            5.0,
            Faction::Enemy,
            (255, 0, 0),
        );
        b.radius = 2.0;
        w.bullets.push(b);
        w.update_bullets(0.05);
        assert!(!collision::circles_overlap(
            w.bullets[0].pos,
            2.0,
            w.player.pos,
            PLAYER_RADIUS
        ));
        w.resolve_enemy_hits();
        assert!(w.player.hp < hp, "fast enemy bullet passed through the player");
        assert_eq!(w.bullets[0].life, 0.0);
    }

    #[test]
    fn pickup_scratch_vec_is_reused_across_ticks() {
        let mut w = World::new(Theme::Sewer, 97);
        w.pickups.clear();
        w.pickups.push(Pickup {
            pos: w.player.pos,
            kind: PickupKind::Heal,
            bob: 0.0,
        });
        w.update_pickups(1.0 / 60.0);
        let ptr = w.pickup_scratch.as_ptr();
        let cap = w.pickup_scratch.capacity();
        assert!(cap > 0, "scratch vec should be kept on the world");
        assert!(w.pickup_scratch.is_empty());
        w.pickups.push(Pickup {
            pos: w.player.pos,
            kind: PickupKind::Heal,
            bob: 0.0,
        });
        w.update_pickups(1.0 / 60.0);
        assert_eq!(w.pickup_scratch.as_ptr(), ptr, "scratch vec reallocated");
        assert_eq!(w.pickup_scratch.capacity(), cap);
    }

    #[test]
    fn acid_spit_launches_a_projectile_not_an_instant_pool() {
        let mut w = World::new(Theme::Sewer, 73);
        w.enemies.clear();
        w.bullets.clear();
        w.pools.clear();
        let ppos = w.player.pos;
        let face = w.player.face;
        assert!(w.fire_one(WeaponKind::AcidSpit, 10.0, 1.0, 1, ppos, face));
        assert_eq!(w.bullets.len(), 1);
        assert!(w.bullets[0].lob.is_some(), "acid shot should be lobbed");
        assert!(w.pools.is_empty(), "acid pool appeared instantly");
    }

    #[test]
    fn acid_spit_pool_lands_at_its_target_after_travel() {
        let mut w = World::new(Theme::Sewer, 79);
        w.enemies.clear();
        w.bullets.clear();
        w.pools.clear();
        w.level.walls.clear();
        let ppos = w.player.pos;
        let face = Vec2::new(1.0, 0.0);
        let target = ppos + face * 50.0;
        assert!(w.fire_one(WeaponKind::AcidSpit, 10.0, 1.0, 1, ppos, face));
        let mut ticks = 0;
        while w.pools.is_empty() && ticks < 120 {
            w.update_bullets(1.0 / 60.0);
            ticks += 1;
        }
        assert_eq!(w.pools.len(), 1, "acid pool never landed");
        assert!(
            w.pools[0].pos.dist(target) < 1.0,
            "pool landed at {:?}, target {target:?}",
            w.pools[0].pos
        );
        assert!(w.bullets.is_empty(), "landed projectile should be removed");
    }

    #[test]
    fn acid_spit_pool_lands_on_a_wall_not_behind_it() {
        let mut w = World::new(Theme::Sewer, 83);
        w.enemies.clear();
        w.bullets.clear();
        w.pools.clear();
        let ppos = w.player.pos;
        let face = Vec2::new(1.0, 0.0);
        let wall = Rectf::new(ppos.x + 20.0, ppos.y - 20.0, 4.0, 40.0);
        w.level.walls = vec![wall];
        assert!(w.fire_one(WeaponKind::AcidSpit, 10.0, 1.0, 1, ppos, face));
        let mut ticks = 0;
        while w.pools.is_empty() && ticks < 120 {
            w.update_bullets(1.0 / 60.0);
            ticks += 1;
        }
        assert_eq!(w.pools.len(), 1, "acid pool never landed");
        assert!(
            w.pools[0].pos.x <= wall.x + 0.01,
            "pool landed past the wall at {:?}",
            w.pools[0].pos
        );
        assert!(w.pools[0].pos.x > ppos.x + 10.0, "pool landed short");
    }
}
