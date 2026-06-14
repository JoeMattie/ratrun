//! The simulation: owns every entity, advances one tick, resolves
//! collisions, runs the spawn director, and draws the world into a
//! half-block pixel framebuffer.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::Sfx;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

use super::bullet::{Bullet, Faction, Orbit};
use super::collision::{self, SpatialGrid};
use super::director::{self, Director};
use super::enemy::{Enemy, EnemyKind};
use super::gem::{Gem, Pickup, PickupKind};
use super::level::{Level, Palette, Prop, PropKind, Theme};
use super::nav::FlowField;
use super::particle::{Particles, Ramp};
use super::player::Player;
use super::weapon::WeaponKind;

const PLAYER_RADIUS: f32 = 3.0;
const PARTICLE_CAP: usize = 6000;
const ENEMY_CAP: usize = 600;

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
        World {
            rng: StdRng::seed_from_u64(seed),
            flow,
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
        if self.elapsed >= director::RUN_SECONDS {
            self.won = true;
        }

        self.update_player(dt, move_dir, dash);
        // Refresh the shared flow field toward the player (cheap: only
        // recomputes when the player crosses into a new nav cell).
        self.flow.update(&self.level.nav, self.player.pos);
        self.run_director(dt);
        self.update_enemies(dt);

        let grid = SpatialGrid::build(self.enemies.iter().map(|e| e.pos), self.level.arena, 16.0);
        self.separate_enemies(&grid);
        self.resolve_enemy_walls();
        self.fire_weapons(dt);
        self.update_bullets(dt);
        self.update_pools(dt);
        self.resolve_player_bullets(&grid);
        self.resolve_enemy_hits();
        self.resolve_contact();
        self.reap_enemies();
        self.update_gems(dt);
        self.update_pickups(dt);
        self.particles.update(dt);

        // Screen-shake decay + jitter offset.
        if self.shake > 0.05 {
            self.shake_ofs = Vec2::new(
                self.rng.gen_range(-1.0..1.0) * self.shake,
                self.rng.gen_range(-1.0..1.0) * self.shake,
            );
            self.shake = (self.shake - dt * 30.0).max(0.0);
        } else {
            self.shake_ofs = Vec2::ZERO;
        }

        if !self.player.alive() {
            self.game_over = true;
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

        let dash_boost = if self.player.dash_active > 0.0 { 3.2 } else { 1.0 };
        // `move_dir` is a throttle vector: unit length = full speed (keyboard),
        // shorter = proportionally slower (mouse easing near the cursor).
        let target_vel = move_dir.clamp_len(1.0) * stats.move_speed * dash_boost;
        self.player.vel += (target_vel - self.player.vel) * (12.0 * dt).min(1.0);
        self.player.pos += self.player.vel * dt;

        // Clamp + wall collision.
        let arena = self.level.arena;
        self.player.pos = self.player.pos.clamp_box(
            Vec2::splat(PLAYER_RADIUS),
            Vec2::new(arena.x - PLAYER_RADIUS, arena.y - PLAYER_RADIUS),
        );
        for w in &self.level.walls {
            self.player.pos = collision::resolve_circle_rect(self.player.pos, PLAYER_RADIUS, w);
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
        self.director.spawn_timer -= dt;
        if self.director.spawn_timer <= 0.0 {
            self.director.spawn_timer = director::spawn_interval(self.elapsed);
            let count = director::pulse_count(self.elapsed);
            let mult = director::hp_mult(self.elapsed);
            for _ in 0..count {
                if self.enemies.len() >= ENEMY_CAP {
                    break;
                }
                let kind = director::pick_kind(self.elapsed, &mut self.rng);
                let pos = self.spawn_point();
                self.enemies.push(Enemy::spawn(kind, pos, mult, &mut self.rng));
            }
        }

        // Periodic elite brute.
        self.director.elite_timer -= dt;
        if self.director.elite_timer <= 0.0 && self.elapsed > 60.0 {
            self.director.elite_timer = 30.0;
            let pos = self.spawn_point();
            let mult = director::hp_mult(self.elapsed) * 1.6;
            self.enemies
                .push(Enemy::spawn(EnemyKind::Brute, pos, mult, &mut self.rng));
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

    /// A point on a ring around the player, just beyond view, clamped to arena.
    fn spawn_point(&mut self) -> Vec2 {
        let r = (self.viewport.len() * 0.5 + 16.0).max(120.0);
        let a = self.rng.gen_range(0.0..std::f32::consts::TAU);
        let p = self.player.pos + Vec2::from_angle(a) * r;
        p.clamp_box(Vec2::splat(6.0), self.level.arena - Vec2::splat(6.0))
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

    fn separate_enemies(&mut self, grid: &SpatialGrid) {
        let mut neigh = Vec::new();
        for i in 0..self.enemies.len() {
            let pi = self.enemies[i].pos;
            let ri = self.enemies[i].radius;
            grid.neighbors(pi, &mut neigh);
            let mut push = Vec2::ZERO;
            for &j in &neigh {
                let j = j as usize;
                if j == i {
                    continue;
                }
                let pj = self.enemies[j].pos;
                let rr = ri + self.enemies[j].radius;
                let d2 = pi.dist_sq(pj);
                if d2 > 1e-4 && d2 < rr * rr {
                    let d = d2.sqrt();
                    push += (pi - pj) * ((rr - d) / d) * 0.5;
                }
            }
            self.enemies[i].pos += push.clamp_len(6.0);
        }
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
            .min_by(|(_, a), (_, b)| {
                a.pos
                    .dist_sq(from)
                    .partial_cmp(&b.pos.dist_sq(from))
                    .unwrap()
            })
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
            self.player.loadout.weapons[i].timer += cd;
            self.fire_one(kind, dmg, cd, projs, ppos, pface);
        }
    }

    fn fire_one(&mut self, kind: WeaponKind, dmg: f32, cd: f32, projs: i32, ppos: Vec2, pface: Vec2) {
        let color = kind.color();
        self.sfx.push(Sfx::Shoot);
        match kind {
            WeaponKind::Gnaw => {
                if let Some((_, epos)) = self.nearest_enemy(ppos) {
                    let dir = (epos - ppos).normalized();
                    let mut b = Bullet::new(ppos, dir * 120.0, dmg, Faction::Player, color);
                    b.size = 1;
                    self.bullets.push(b);
                    self.particles
                        .cone(&mut self.rng, ppos, dir, 4, 60.0, Ramp::Spark, 0.2);
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
            }
            WeaponKind::SporeOrbit => {
                // Refresh a ring of orbiting spores; they live a bit longer
                // than the cooldown so the orbit looks continuous.
                let radius = 24.0;
                for k in 0..projs {
                    let angle = k as f32 / projs as f32 * std::f32::consts::TAU;
                    let mut b = Bullet::new(ppos, Vec2::ZERO, dmg, Faction::Player, color);
                    b.life = cd + 0.15;
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
                    self.particles
                        .burst(&mut self.rng, p, 1, 18.0, 0.5, Ramp::Custom(color, (120, 40, 90)), 0.3, true);
                }
            }
            WeaponKind::AcidSpit => {
                let target = self
                    .nearest_enemy(ppos)
                    .map(|(_, p)| p)
                    .unwrap_or(ppos + pface * 50.0);
                self.pools.push(Pool {
                    pos: target,
                    radius: 12.0 + projs as f32 * 1.5,
                    dps: dmg * 4.0,
                    life: 2.6,
                    color: (140, 220, 70),
                });
                self.particles
                    .burst(&mut self.rng, target, 14, 30.0, 2.0, Ramp::Custom((180, 255, 90), (40, 90, 20)), 0.5, false);
            }
        }
    }

    fn update_bullets(&mut self, dt: f32) {
        let ppos = self.player.pos;
        let arena = self.level.arena;
        for b in self.bullets.iter_mut() {
            if let Some(o) = b.orbit.as_mut() {
                o.angle += o.speed * dt;
                b.pos = ppos + Vec2::from_angle(o.angle) * o.radius;
            } else {
                b.pos += b.vel * dt;
            }
            b.life -= dt;
            b.hit_cd = (b.hit_cd - dt).max(0.0);
        }
        self.bullets.retain(|b| {
            b.life > 0.0
                && b.pos.x > -12.0
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

    fn resolve_player_bullets(&mut self, grid: &SpatialGrid) {
        let mut neigh = Vec::new();
        let mut hit_any = false;
        for b in self.bullets.iter_mut() {
            if b.faction != Faction::Player || b.hit_cd > 0.0 {
                continue;
            }
            grid.neighbors(b.pos, &mut neigh);
            for &j in &neigh {
                let e = &mut self.enemies[j as usize];
                if !e.alive() {
                    continue;
                }
                if collision::circles_overlap(b.pos, b.radius, e.pos, e.radius) {
                    e.hurt(b.damage);
                    hit_any = true;
                    self.particles
                        .burst(&mut self.rng, b.pos, 4, 50.0, 1.0, Ramp::Spark, 0.18, true);
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
            if collision::circles_overlap(b.pos, b.radius, ppos, PLAYER_RADIUS) {
                if self.player.take_damage(b.damage) {
                    hit = true;
                }
                b.life = 0.0;
            }
        }
        if hit {
            let p = self.player.pos;
            self.particles
                .burst(&mut self.rng, p, 10, 40.0, 1.0, Ramp::Custom((255, 90, 90), (90, 20, 20)), 0.4, false);
            self.shake = self.shake.max(4.0);
            self.sfx.push(Sfx::Hurt);
        }
    }

    fn resolve_contact(&mut self) {
        let ppos = self.player.pos;
        let mut hit_dmg = 0.0f32;
        for e in self.enemies.iter_mut() {
            if e.contact_cd <= 0.0 && collision::circles_overlap(ppos, PLAYER_RADIUS, e.pos, e.radius)
            {
                hit_dmg = hit_dmg.max(e.damage);
                e.contact_cd = 0.6;
                // Knockback.
                e.pos += (e.pos - ppos).normalized() * 4.0;
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
                    self.shake = self.shake.max(if e.kind == EnemyKind::Boss { 16.0 } else { 6.0 });
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
                if self.rng.gen::<f32>() < drop_chance {
                    let kind = match self.rng.gen_range(0..3) {
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
        self.gems.retain_mut(|g| {
            let to = ppos - g.pos;
            let dist = to.len();
            if g.magnetized || dist < magnet {
                g.magnetized = true;
                let pull = 60.0 + (magnet - dist).max(0.0) * 2.0;
                g.vel += to.normalized() * pull * dt;
                g.vel = g.vel.clamp_len(160.0);
            }
            g.pos += g.vel * dt;
            if dist < PLAYER_RADIUS + 3.0 {
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

    fn update_pickups(&mut self, dt: f32) {
        let ppos = self.player.pos;
        let mut collected: Vec<PickupKind> = Vec::new();
        self.pickups.retain_mut(|p| {
            p.bob += dt * 4.0;
            if p.pos.dist(ppos) < PLAYER_RADIUS + 5.0 {
                collected.push(p.kind);
                false
            } else {
                true
            }
        });
        for k in collected {
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
    }

    // ---- Rendering ------------------------------------------------------

    pub fn draw(&mut self, pb: &mut PixelBuffer) {
        let vw = pb.w as f32;
        let vh = pb.h as f32;
        self.viewport = Vec2::new(vw, vh);
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

        // Floor dots for motion reference — only across the visible region.
        let step = 14.0;
        let x0 = ((cam.x / step).floor() * step) as i32;
        let y0 = ((cam.y / step).floor() * step) as i32;
        let mut gx = x0;
        while (gx as f32) < cam.x + vw + step {
            let mut gy = y0;
            while (gy as f32) < cam.y + vh + step {
                if gx >= 0 && gy >= 0 && (gx as f32) < arena.x && (gy as f32) < arena.y {
                    pb.plot(
                        (gx as f32 - cam.x).round() as i32,
                        (gy as f32 - cam.y).round() as i32,
                        pal.bg_alt,
                    );
                }
                gy += step as i32;
            }
            gx += step as i32;
        }

        // Arena border.
        let b0 = self.w2s(Vec2::ZERO, cam);
        let b1 = self.w2s(arena, cam);
        pb.line(b0.0, b0.1, b1.0, b0.1, pal.wall_edge);
        pb.line(b0.0, b1.1, b1.0, b1.1, pal.wall_edge);
        pb.line(b0.0, b0.1, b0.0, b1.1, pal.wall_edge);
        pb.line(b1.0, b0.1, b1.0, b1.1, pal.wall_edge);

        // Props (themed obstacles).
        for p in &self.level.props {
            draw_prop(pb, p, cam, pal);
        }

        // Acid pools.
        for p in &self.pools {
            let s = self.w2s(p.pos, cam);
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

        // Enemies (animated insects).
        for e in &self.enemies {
            self.draw_bug(pb, e, cam);
        }

        // Bullets.
        for b in &self.bullets {
            let s = self.w2s(b.pos, cam);
            if b.size >= 2 {
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

    /// Draw one enemy as an animated insect: flapping/buzzing wings, a
    /// segmented body, a tripod leg gait, and antennae.
    fn draw_bug(&self, pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
        let s = self.w2s(e.pos, cam);
        let base = Vec2::new(s.0 as f32, s.1 as f32);
        let f = if e.facing.len_sq() > 0.01 {
            e.facing.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let p = f.perp();
        let r = e.radius;
        let flash = e.flash > 0.0;
        let body = if flash { (255, 255, 255) } else { e.kind.color() };
        let dark = palette::scale(body, 0.55);
        let st = bug_style(e.kind);

        // Wings under the body, flapping (additive so they read as gauzy).
        if st.wings {
            let flap = e.anim.sin().abs();
            let wr = (r * (0.45 + flap * 0.7)).max(1.0);
            let wb = (wr * 0.6).max(1.0);
            let wc = palette::scale(self.level.palette.accent, 0.4 + flap * 0.45);
            for side in [-1.0f32, 1.0] {
                let c = base + p * (side * r * 0.7) - f * (r * 0.1);
                let cx = c.x as i32;
                let cy = (c.y - r * 0.4) as i32;
                let rr = wr as i32;
                for dy in -rr..=rr {
                    for dx in -rr..=rr {
                        let nx = dx as f32 / wr;
                        let ny = dy as f32 / wb;
                        if nx * nx + ny * ny <= 1.0 {
                            pb.plot_add(cx + dx, cy + dy, wc);
                        }
                    }
                }
            }
        }

        // Legs: pairs distributed along the body, alternating tripod gait.
        for i in 0..st.leg_pairs {
            let along = f * (r * (0.55 - i as f32 * 0.55));
            let phase = e.anim + i as f32 * 2.1;
            for (k, side) in [(-1.0f32, 0.0f32), (1.0, std::f32::consts::PI)] {
                let swing = (phase + side).sin() * 0.5;
                let hip = base + along + p * (k * r * 0.45);
                let dir = (p * k + f * swing).normalized();
                let knee = hip + dir * (r * 0.85);
                let foot = knee + (p * k * 0.4 + f * (swing - 0.2)).normalized() * (r * 0.7);
                pb.line(hip.x as i32, hip.y as i32, knee.x as i32, knee.y as i32, dark);
                pb.line(knee.x as i32, knee.y as i32, foot.x as i32, foot.y as i32, dark);
            }
        }

        // Body segments, head (front) → abdomen (back).
        for seg in 0..st.segs {
            let t = seg as f32 / st.segs.max(1) as f32;
            let c = base + f * (r * (0.7 - t * 1.5));
            let rad = (r * (0.42 + 0.28 * (1.0 - t))).max(1.0);
            let col = if st.stripes && seg % 2 == 1 {
                if flash {
                    (255, 255, 255)
                } else {
                    palette::scale(body, 0.55)
                }
            } else {
                body
            };
            pb.filled_circle(c.x as i32, c.y as i32, rad as i32, col);
        }

        // Head detail: antennae, eyes, optional horn.
        let head = base + f * (r * 0.7);
        let wave = (e.anim * 0.5).sin() * 0.4;
        for side in [-1.0f32, 1.0] {
            let dir = (f + p * (side * 0.55 + wave * side)).normalized();
            let tip = head + dir * (r * 1.05);
            pb.line(head.x as i32, head.y as i32, tip.x as i32, tip.y as i32, dark);
        }
        if st.eyes && r >= 3.0 {
            for side in [-1.0f32, 1.0] {
                let ep = head + p * (side * r * 0.35);
                pb.plot(ep.x as i32, ep.y as i32, (15, 15, 18));
            }
        }
        if st.horn && r >= 4.0 {
            let tip = head + f * (r * 0.9);
            pb.line(head.x as i32, head.y as i32, tip.x as i32, tip.y as i32, palette::scale(body, 1.25));
        }

        // Boss health ring.
        if e.kind == EnemyKind::Boss {
            let frac = (e.hp / e.max_hp).clamp(0.0, 1.0);
            pb.ring(s.0, s.1, r as i32 + 4, palette::mix((90, 20, 20), (255, 80, 80), frac));
        }
    }

    fn draw_player(&self, pb: &mut PixelBuffer, cam: Vec2) {
        let s = self.w2s(self.player.pos, cam);
        let body = if self.player.hurt_flash > 0.0 {
            (255, 120, 120)
        } else if self.player.iframes > 0.0 && ((self.elapsed * 20.0) as i32 % 2 == 0) {
            (160, 160, 180)
        } else {
            (220, 220, 235)
        };
        pb.filled_circle(s.0, s.1, PLAYER_RADIUS as i32, body);
        // Ears.
        let perp = self.player.face.perp();
        let ear1 = self.player.pos + self.player.face * 2.0 + perp * 2.0;
        let ear2 = self.player.pos + self.player.face * 2.0 - perp * 2.0;
        let e1 = self.w2s(ear1, cam);
        let e2 = self.w2s(ear2, cam);
        pb.plot(e1.0, e1.1, (240, 190, 200));
        pb.plot(e2.0, e2.1, (240, 190, 200));
        // Eyes / nose toward facing.
        let nose = self.player.pos + self.player.face * 3.0;
        let ns = self.w2s(nose, cam);
        pb.plot(ns.0, ns.1, (40, 30, 30));
        // Tail.
        let tail = self.player.pos - self.player.face * 5.0;
        let ts = self.w2s(tail, cam);
        pb.line(s.0, s.1, ts.0, ts.1, (200, 150, 160));
    }

    #[inline]
    fn w2s(&self, p: Vec2, cam: Vec2) -> (i32, i32) {
        ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
    }
}

// ---- Prop rendering -------------------------------------------------------

fn draw_prop(pb: &mut PixelBuffer, prop: &Prop, cam: Vec2, pal: &Palette) {
    let x = (prop.rect.x - cam.x).round() as i32;
    let y = (prop.rect.y - cam.y).round() as i32;
    let w = prop.rect.w as i32;
    let h = prop.rect.h as i32;
    match prop.kind {
        PropKind::PipeV => pipe(pb, x, y, w, h, true),
        PropKind::PipeH => pipe(pb, x, y, w, h, false),
        PropKind::Barrier => barrier(pb, x, y, w, h),
        PropKind::Valve => valve(pb, x, y, w, h),
        PropKind::Grate => grate(pb, x, y, w, h, pal),
        PropKind::Table => table(pb, x, y, w, h),
        PropKind::Crate => crate_box(pb, x, y, w, h),
        PropKind::Barrel => barrel(pb, x, y, w, h),
        PropKind::Counter => counter(pb, x, y, w, h),
        PropKind::Console => console(pb, x, y, w, h, pal),
        PropKind::Tank => tank(pb, x, y, w, h, pal),
    }
}

/// A shaded metal cylinder with flange bands.
fn pipe(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, vert: bool) {
    let base = (95, 118, 112);
    let shade = |t: f32| palette::scale(base, 0.55 + 0.75 * (1.0 - (t - 0.32).abs() * 1.4).max(0.0));
    if vert {
        for col in 0..w {
            let t = col as f32 / (w.max(2) - 1) as f32;
            pb.rect_fill(x + col, y, 1, h, shade(t));
        }
        let fl = palette::scale(base, 0.45);
        pb.rect_fill(x, y, w, 3, fl);
        pb.rect_fill(x, y + h - 3, w, 3, fl);
        pb.rect_fill(x, y + h / 2 - 1, w, 3, fl);
    } else {
        for row in 0..h {
            let t = row as f32 / (h.max(2) - 1) as f32;
            pb.rect_fill(x, y + row, w, 1, shade(t));
        }
        let fl = palette::scale(base, 0.45);
        pb.rect_fill(x, y, 3, h, fl);
        pb.rect_fill(x + w - 3, y, 3, h, fl);
        pb.rect_fill(x + w / 2 - 1, y, 3, h, fl);
    }
}

fn barrier(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    let base = (66, 76, 116);
    let yellow = (220, 200, 80);
    // Diagonal hazard stripes, clipped to the footprint.
    for py in 0..h {
        for px in 0..w {
            let c = if (px + py).rem_euclid(10) < 4 { yellow } else { base };
            pb.plot(x + px, y + py, c);
        }
    }
    pb.rect_fill(x, y, w, 1, palette::scale(base, 1.6));
    pb.rect_fill(x, y + h - 1, w, 1, palette::scale(base, 0.5));
}

fn valve(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    pb.rect_fill(x, y, w, h, (90, 110, 106));
    let cx = x + w / 2;
    let cy = y + h / 2;
    let r = (w.min(h) / 2 - 1).max(1);
    pb.filled_circle(cx, cy, r, (190, 70, 60));
    pb.line(cx - r, cy, cx + r, cy, (40, 30, 30));
    pb.line(cx, cy - r, cx, cy + r, (40, 30, 30));
}

fn grate(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, pal: &Palette) {
    pb.rect_fill(x, y, w, h, (28, 36, 34));
    let line = palette::scale(pal.wall_edge, 0.8);
    let mut i = 2;
    while i < w {
        pb.rect_fill(x + i, y, 1, h, line);
        i += 5;
    }
    let mut j = 2;
    while j < h {
        pb.rect_fill(x, y + j, w, 1, line);
        j += 5;
    }
}

fn table(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    let top = (162, 120, 74);
    let rim = (108, 76, 44);
    let leg = (78, 54, 32);
    pb.rect_fill(x, y, w, h, top);
    // rim
    pb.line(x, y, x + w - 1, y, rim);
    pb.line(x, y + h - 1, x + w - 1, y + h - 1, rim);
    pb.line(x, y, x, y + h - 1, rim);
    pb.line(x + w - 1, y, x + w - 1, y + h - 1, rim);
    // plank seams
    pb.line(x + 1, y + h / 2, x + w - 2, y + h / 2, palette::scale(top, 0.85));
    // legs (overhead nubs)
    for (lx, ly) in [(x + 1, y + 1), (x + w - 3, y + 1), (x + 1, y + h - 3), (x + w - 3, y + h - 3)] {
        pb.rect_fill(lx, ly, 2, 2, leg);
    }
}

fn crate_box(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    let wood = (152, 112, 66);
    let edge = (96, 68, 38);
    pb.rect_fill(x, y, w, h, wood);
    // border
    pb.line(x, y, x + w - 1, y, edge);
    pb.line(x, y + h - 1, x + w - 1, y + h - 1, edge);
    pb.line(x, y, x, y + h - 1, edge);
    pb.line(x + w - 1, y, x + w - 1, y + h - 1, edge);
    // diagonal braces
    pb.line(x, y, x + w - 1, y + h - 1, edge);
    pb.line(x + w - 1, y, x, y + h - 1, edge);
    pb.line(x, y, x + w - 1, y, (186, 146, 92)); // top highlight
}

fn barrel(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    let cx = x + w / 2;
    let cy = y + h / 2;
    let r = (w.min(h) / 2).max(1);
    pb.filled_circle(cx, cy, r, (142, 96, 60));
    pb.ring(cx, cy, r - 1, (92, 92, 100));
    pb.ring(cx, cy, (r / 2).max(1), (92, 92, 100));
    pb.plot(cx - r / 3, cy - r / 3, (200, 160, 120));
}

fn counter(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    pb.rect_fill(x, y, w, h, (150, 152, 160));
    pb.rect_fill(x, y, w, 1, (205, 210, 220)); // top highlight
    pb.rect_fill(x, y + h - 1, w, 1, (92, 96, 108)); // base shadow
    pb.rect_fill(x, y + h / 2, w, 1, (118, 122, 132)); // seam
}

fn console(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, pal: &Palette) {
    pb.rect_fill(x, y, w, h, (48, 54, 70));
    // screen
    let sc = palette::scale(pal.accent, 0.8);
    pb.rect_fill(x + 2, y + 2, w - 4, h - 6, palette::scale(sc, 0.4));
    for gx in (x + 3..x + w - 3).step_by(3) {
        pb.plot_add(gx, y + 3, palette::scale(sc, 0.5));
    }
    // buttons
    for (i, c) in [(2, (220, 90, 80)), (6, (220, 200, 90)), (10, (110, 220, 130))] {
        pb.filled_circle(x + i, y + h - 2, 1, c);
    }
}

fn tank(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, pal: &Palette) {
    let cx = x + w / 2;
    let cy = y + h / 2;
    let r = (w.min(h) / 2).max(1);
    pb.filled_circle(cx, cy, r, (70, 120, 140));
    pb.filled_circle(cx, cy, (r - 2).max(1), palette::scale(pal.accent, 0.85));
    pb.plot(cx - r / 3, cy - r / 3, (220, 240, 245));
    pb.ring(cx, cy, r, (40, 70, 84));
}

struct BugStyle {
    segs: i32,
    leg_pairs: i32,
    wings: bool,
    stripes: bool,
    horn: bool,
    eyes: bool,
}

/// Per-kind insect anatomy.
fn bug_style(kind: EnemyKind) -> BugStyle {
    match kind {
        // Ant: three crisp segments, six legs, antennae.
        EnemyKind::Skitterer => BugStyle {
            segs: 3,
            leg_pairs: 3,
            wings: false,
            stripes: false,
            horn: false,
            eyes: false,
        },
        // Fly/mosquito: tiny body, fast buzzing wings.
        EnemyKind::Bat => BugStyle {
            segs: 2,
            leg_pairs: 2,
            wings: true,
            stripes: false,
            horn: false,
            eyes: true,
        },
        // Beetle: rounded shell, six legs, eyes.
        EnemyKind::Cat => BugStyle {
            segs: 2,
            leg_pairs: 3,
            wings: false,
            stripes: false,
            horn: false,
            eyes: true,
        },
        // Wasp: striped abdomen, wings, stinger-ish.
        EnemyKind::Spitter => BugStyle {
            segs: 3,
            leg_pairs: 3,
            wings: true,
            stripes: true,
            horn: false,
            eyes: true,
        },
        // Rhino beetle: bulky, horn, heavy legs.
        EnemyKind::Brute => BugStyle {
            segs: 2,
            leg_pairs: 3,
            wings: false,
            stripes: false,
            horn: true,
            eyes: true,
        },
        // Hornet queen: big, winged, striped, mandible eyes.
        EnemyKind::Boss => BugStyle {
            segs: 3,
            leg_pairs: 4,
            wings: true,
            stripes: true,
            horn: true,
            eyes: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::gem::{Gem, Pickup, PickupKind};

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
            let mut e = Enemy::spawn(*k, Vec2::new(cx - 78.0 + i as f32 * 30.0, cy), 1.0, &mut rng);
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
        w.enemies.push(Enemy::spawn(EnemyKind::Boss, c + Vec2::new(30.0, 0.0), 1.0, &mut rng));
        for k in [
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
        ] {
            w.enemies.push(Enemy::spawn(k, c + Vec2::from_angle(0.3) * 20.0, 1.0, &mut rng));
        }
        w.gems.push(Gem::new(c, 1));
        w.gems.push(Gem::new(c, 30));
        for kind in [PickupKind::Heal, PickupKind::Magnet, PickupKind::Nuke] {
            w.pickups.push(Pickup { pos: c, kind, bob: 0.5 });
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
}
