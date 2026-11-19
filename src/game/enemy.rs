//! Enemies: kinds, per-kind stats, and steering behavior toward the player.

use crate::math::Vec2;
use crate::render::palette::Rgb;
use rand::{Rng, RngExt};

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
    /// Monotonic runtime identifier assigned on spawn (0 if constructed outside `World`).
    pub id: u32,
    pub kind: EnemyKind,
    /// True for periodic elite Brutes spawned by the director.
    pub elite: bool,
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
            id: 0,
            kind,
            elite: false,
            pos,
            vel: Vec2::ZERO,
            hp,
            max_hp: hp,
            radius: kind.radius(),
            damage: kind.damage(),
            flash: 0.0,
            fire_cd: rng.random_range(1.0..2.5),
            wander: rng.random_range(0.0..std::f32::consts::TAU),
            contact_cd: 0.0,
            anim: rng.random_range(0.0..std::f32::consts::TAU),
            facing: Vec2::new(1.0, 0.0),
        }
    }

    /// Steer toward the player, routing around walls via the flow field.
    /// `flow_dir` is the pathfinding direction at this enemy's cell (ZERO
    /// inside the goal cell / off-grid, in which case we head straight at the
    /// player).
    pub fn steer(&mut self, target: Vec2, flow_dir: Vec2, dt: f32, speed_mult: f32) {
        let to = target - self.pos;
        let dist = to.len();
        let direct = to.normalized();
        // Follow the route whenever the field has an opinion. The field is
        // only ZERO inside the player's own cell (and off-grid), i.e. exactly
        // where the straight line cannot cross a wall, so the un-gridded last
        // stretch costs nothing in look: the velocity smoothing below hides
        // the grid. Taking the direct line within 26 px instead used to drag
        // enemies into props, where wall resolution parked them for good.
        let chase = if flow_dir.len_sq() > 0.001 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision;
    use crate::game::level::{Level, Rectf, Theme};
    use crate::game::nav::{FlowField, NavGrid};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn direct_chase_needs_an_empty_flow() {
        let mut rng = StdRng::seed_from_u64(7);
        let target = Vec2::new(20.0, 0.0); // well inside the old 26 px shortcut
        let route = Vec2::new(0.0, 1.0); // the field says "go around"
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        e.steer(target, route, 0.2, 1.0);
        assert!(
            e.vel.y > 5.0 && e.vel.x.abs() < 1.0,
            "must keep following the route on the last stretch: {:?}",
            e.vel
        );

        // With no flow (inside the goal cell) it heads straight in.
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        e.steer(target, Vec2::ZERO, 0.2, 1.0);
        assert!(
            e.vel.x > 5.0 && e.vel.y.abs() < 1.0,
            "an empty flow means the straight line is clear: {:?}",
            e.vel
        );
    }

    #[test]
    fn enemy_behind_a_prop_is_not_dragged_into_it() {
        // Player just right of a barrier, enemy just left of it: the straight
        // line crosses the prop, the flow must bend around it.
        let wall = Rectf::new(100.0, 40.0, 16.0, 66.0);
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[wall], 16.0);
        let mut f = FlowField::new(&g);
        let player = Vec2::new(122.0, 73.0);
        f.update(&g, player);
        let mut rng = StdRng::seed_from_u64(3);
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::new(96.0, 73.0), 1.0, &mut rng);
        assert!(
            e.pos.dist(player) <= 26.0,
            "the case under test is a short hop"
        );
        let flow = f.dir_at(e.pos);
        assert!(flow.len() > 0.5, "the flow should route around the prop");
        let before = e.pos.dist(player);
        for _ in 0..30 {
            let flow = f.dir_at(e.pos);
            e.steer(player, flow, 1.0 / 60.0, 1.0);
            e.pos = collision::resolve_circle_rect(e.pos, e.radius, &wall);
        }
        // It must not have been pushed into the prop (which would be the
        // straight-line tell) and must not have lost ground.
        assert!(
            !wall.contains(e.pos),
            "enemy ended inside the prop at {:?}",
            e.pos
        );
        assert!(e.pos.dist(player) <= before, "enemy stalled at {:?}", e.pos);
    }

    #[test]
    fn skitterers_parked_in_a_prop_halo_reach_the_player() {
        // The reported stall: 20 skitterers tucked behind a 16x66 barrier.
        let wall = Rectf::new(100.0, 40.0, 16.0, 66.0);
        let walls = [wall];
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &walls, 16.0);
        let mut f = FlowField::new(&g);
        let player = Vec2::new(200.0, 73.0);
        f.update(&g, player);
        let mut rng = StdRng::seed_from_u64(11);
        let mut enemies: Vec<Enemy> = (0..20)
            .map(|i| {
                let y = 46.0 + i as f32 * 3.0;
                Enemy::spawn(EnemyKind::Skitterer, Vec2::new(94.0, y), 1.0, &mut rng)
            })
            .collect();
        let dt = 1.0 / 60.0;
        for _ in 0..20 * 60 {
            for e in enemies.iter_mut() {
                let flow = f.dir_at(e.pos);
                e.steer(player, flow, dt, 1.0);
                e.pos = e.pos.clamp_box(Vec2::splat(2.0), Vec2::new(318.0, 318.0));
                for w in &walls {
                    e.pos = collision::resolve_circle_rect(e.pos, e.radius, w);
                }
            }
        }
        let stuck: Vec<Vec2> = enemies
            .iter()
            .filter(|e| e.pos.dist(player) > 24.0)
            .map(|e| e.pos)
            .collect();
        assert!(stuck.is_empty(), "still stuck after 20 s: {stuck:?}");
    }

    #[test]
    fn enemies_reach_the_player_on_every_theme() {
        // Same promise on the real maps: nobody is left in a blocked cell.
        for theme in Theme::all() {
            let level = Level::new(theme);
            let mut f = FlowField::new(&level.nav);
            let player = level.arena * 0.5;
            f.update(&level.nav, player);
            let mut rng = StdRng::seed_from_u64(5);
            let mut enemies: Vec<Enemy> = (0..12)
                .map(|_| {
                    let p = Vec2::new(
                        rng.random_range(0.0..level.arena.x),
                        rng.random_range(0.0..level.arena.y),
                    );
                    Enemy::spawn(EnemyKind::Skitterer, p, 1.0, &mut rng)
                })
                .collect();
            let dt = 1.0 / 60.0;
            for _ in 0..60 * 60 {
                for e in enemies.iter_mut() {
                    let flow = f.dir_at(e.pos);
                    e.steer(player, flow, dt, 1.0);
                    e.pos = e
                        .pos
                        .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
                    for w in &level.walls {
                        e.pos = collision::resolve_circle_rect(e.pos, e.radius, w);
                    }
                }
            }
            let stuck: Vec<Vec2> = enemies
                .iter()
                .filter(|e| e.pos.dist(player) > 40.0)
                .map(|e| e.pos)
                .collect();
            assert!(stuck.is_empty(), "{theme:?}: stuck enemies at {stuck:?}");
        }
    }
}
