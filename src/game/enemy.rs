//! Enemies: kinds, per-kind stats, and steering behavior toward the player.

use crate::math::Vec2;
use crate::render::palette::Rgb;
use rand::Rng;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnemyKind {
    Skitterer,
    Bat,
    Cat,
    Spitter,
    Brute,
    Boss,
}

impl EnemyKind {
    pub fn base_hp(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 6.0,
            EnemyKind::Bat => 4.0,
            EnemyKind::Cat => 18.0,
            EnemyKind::Spitter => 12.0,
            EnemyKind::Brute => 60.0,
            EnemyKind::Boss => 1400.0,
        }
    }
    pub fn speed(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 30.0,
            EnemyKind::Bat => 42.0,
            EnemyKind::Cat => 38.0,
            EnemyKind::Spitter => 22.0,
            EnemyKind::Brute => 16.0,
            EnemyKind::Boss => 20.0,
        }
    }
    pub fn radius(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 2.5,
            EnemyKind::Bat => 2.0,
            EnemyKind::Cat => 4.0,
            EnemyKind::Spitter => 3.0,
            EnemyKind::Brute => 6.5,
            EnemyKind::Boss => 14.0,
        }
    }
    pub fn damage(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 6.0,
            EnemyKind::Bat => 5.0,
            EnemyKind::Cat => 12.0,
            EnemyKind::Spitter => 8.0,
            EnemyKind::Brute => 22.0,
            EnemyKind::Boss => 30.0,
        }
    }
    pub fn xp(self) -> u32 {
        match self {
            EnemyKind::Skitterer => 1,
            EnemyKind::Bat => 1,
            EnemyKind::Cat => 3,
            EnemyKind::Spitter => 4,
            EnemyKind::Brute => 10,
            EnemyKind::Boss => 200,
        }
    }
    pub fn color(self) -> Rgb {
        match self {
            EnemyKind::Skitterer => (150, 120, 90),
            EnemyKind::Bat => (120, 90, 160),
            EnemyKind::Cat => (90, 90, 100),
            EnemyKind::Spitter => (120, 200, 90),
            EnemyKind::Brute => (170, 70, 60),
            EnemyKind::Boss => (220, 60, 120),
        }
    }
    /// Spitters fire enemy bullets; others are melee.
    pub fn ranged(self) -> bool {
        matches!(self, EnemyKind::Spitter | EnemyKind::Boss)
    }
}

pub struct Enemy {
    pub kind: EnemyKind,
    pub pos: Vec2,
    pub vel: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub radius: f32,
    pub damage: f32,
    pub flash: f32,
    pub fire_cd: f32,
    pub wander: f32,
    pub contact_cd: f32,
    /// Gait/wing animation phase, advanced by movement.
    pub anim: f32,
    /// Smoothed facing direction for sprite orientation.
    pub facing: Vec2,
}

impl Enemy {
    pub fn spawn(kind: EnemyKind, pos: Vec2, hp_mult: f32, rng: &mut impl Rng) -> Self {
        let hp = kind.base_hp() * hp_mult;
        Enemy {
            kind,
            pos,
            vel: Vec2::ZERO,
            hp,
            max_hp: hp,
            radius: kind.radius(),
            damage: kind.damage(),
            flash: 0.0,
            fire_cd: rng.gen_range(1.0..2.5),
            wander: rng.gen_range(0.0..std::f32::consts::TAU),
            contact_cd: 0.0,
            anim: rng.gen_range(0.0..std::f32::consts::TAU),
            facing: Vec2::new(1.0, 0.0),
        }
    }

    /// Steer toward the player, routing around walls via the flow field.
    /// `flow_dir` is the pathfinding direction at this enemy's cell (ZERO
    /// near the goal / off-grid, in which case we head straight at the player).
    pub fn steer(&mut self, target: Vec2, flow_dir: Vec2, dt: f32, speed_mult: f32) {
        let to = target - self.pos;
        let dist = to.len();
        let direct = to.normalized();
        // Use the routed direction when far; switch to direct on final approach
        // (within ~1.5 cells) for a smooth, un-gridded last stretch.
        let chase = if flow_dir.len_sq() > 0.001 && dist > 26.0 {
            flow_dir
        } else {
            direct
        };
        let speed = self.kind.speed() * speed_mult;
        let desired = match self.kind {
            EnemyKind::Bat => {
                self.wander += dt * 4.0;
                (chase + Vec2::from_angle(self.wander) * 0.6).normalized() * speed
            }
            EnemyKind::Spitter => {
                // Keep a stand-off range: approach via the route, retreat directly.
                if dist > 110.0 {
                    chase * speed
                } else if dist < 80.0 {
                    direct * (-speed)
                } else {
                    chase.perp() * speed * 0.5
                }
            }
            _ => chase * speed,
        };
        // Smooth toward desired velocity.
        self.vel += (desired - self.vel) * (6.0 * dt).min(1.0);
        self.pos += self.vel * dt;

        // Animation + facing.
        let sp = self.vel.len();
        // Flyers buzz constantly; crawlers scuttle in proportion to speed.
        let cadence = if matches!(self.kind, EnemyKind::Bat) {
            22.0
        } else {
            4.0 + sp * 0.5
        };
        self.anim += dt * cadence;
        if sp > 1.0 {
            self.facing += (self.vel.normalized() - self.facing) * (8.0 * dt).min(1.0);
        }

        if self.flash > 0.0 {
            self.flash -= dt;
        }
        if self.contact_cd > 0.0 {
            self.contact_cd -= dt;
        }
    }

    pub fn hurt(&mut self, dmg: f32) {
        self.hp -= dmg;
        self.flash = 0.12;
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }
}
