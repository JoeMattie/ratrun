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
            EnemyKind::Skitterer => 5.0,
            EnemyKind::Bat => 4.0,
            EnemyKind::Cat => 9.0,
            EnemyKind::Spitter => 6.0,
            EnemyKind::Brute => 16.0,
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

/// How a campaign level's horde differs from the base kinds.
///
/// The base kinds ([`EnemyKind::damage`], [`EnemyKind::speed`]) are the Sewer's
/// and are owned by the enemy-stats tuning pass; a later level *multiplies*
/// them here instead of retuning them, so the two cannot fight each other.
/// [`crate::game::director::enemy_mods_for`] builds these per spawn and ramps
/// them over the level's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyMods {
    /// Multiplies [`EnemyKind::damage`] (what contact/bullets take off the rat).
    pub damage: f32,
    /// Multiplies [`EnemyKind::speed`] (the Lab's swarm is quicker than the
    /// Sewer's).
    pub speed: f32,
}

impl EnemyMods {
    /// The base kinds, unchanged: campaign level 1.
    pub const NONE: EnemyMods = EnemyMods {
        damage: 1.0,
        speed: 1.0,
    };
}

impl Default for EnemyMods {
    fn default() -> Self {
        EnemyMods::NONE
    }
}

#[derive(Clone)]
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
    /// Campaign-level speed multiplier ([`EnemyMods::speed`]); 1.0 for the base
    /// kinds. Stored on the body because [`Enemy::steer`] reads it every tick.
    pub speed_scale: f32,
    /// Gait/wing animation phase, advanced by movement.
    pub anim: f32,
    /// Smoothed facing direction for sprite orientation.
    pub facing: Vec2,
    /// Seconds of hard stun left (0 = free). A stunned body does not steer, its
    /// velocity bleeds off and it cannot fire (the Sonic Squeak ring).
    pub stun: f32,
    /// Movement multiplier while `slow_t > 0` (1.0 = unaffected). Elites and the
    /// boss are slowed instead of stunned.
    pub slow_mult: f32,
    /// Seconds of slow left.
    pub slow_t: f32,
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
            speed_scale: 1.0,
            anim: rng.random_range(0.0..std::f32::consts::TAU),
            facing: Vec2::new(1.0, 0.0),
            // No extra rng draws: a freshly spawned body is neither stunned nor
            // slowed, so adding these fields cannot move the rng stream.
            stun: 0.0,
            slow_mult: 1.0,
            slow_t: 0.0,
        }
    }

    /// Decay the crowd-control timers and stop a stunned body dead.
    ///
    /// Returns `true` while the body is stunned (the caller must skip
    /// steering). At the defaults (`stun == 0`, `slow_t == 0`) this only
    /// performs the two timer comparisons, so an ordinary enemy's tick is
    /// unchanged.
    fn tick_control(&mut self, dt: f32) -> bool {
        if self.stun > 0.0 {
            self.stun = (self.stun - dt).max(0.0);
            self.vel = self.vel * (1.0 - (12.0 * dt).min(1.0));
            self.pos += self.vel * dt;
            if self.flash > 0.0 {
                self.flash -= dt;
            }
            if self.contact_cd > 0.0 {
                self.contact_cd -= dt;
            }
            return true;
        }
        if self.slow_t > 0.0 {
            self.slow_t = (self.slow_t - dt).max(0.0);
            if self.slow_t <= 0.0 {
                self.slow_mult = 1.0;
            }
        }
        false
    }

    /// Stun the body for `secs` (keeps the longer of the two).
    #[inline]
    pub fn stun_for(&mut self, secs: f32) {
        if secs > 0.0 {
            self.stun = self.stun.max(secs);
        }
    }

    /// Slow the body to `mult` of its speed for `secs` (keeps the stronger of
    /// the two effects; the slow lifts when its timer runs out).
    #[inline]
    pub fn slow_for(&mut self, mult: f32, secs: f32) {
        if secs > 0.0 {
            let mult = mult.clamp(0.05, 1.0);
            if self.slow_t <= 0.0 || mult < self.slow_mult {
                self.slow_mult = mult;
            }
            self.slow_t = self.slow_t.max(secs);
        }
    }

    /// Clear every crowd-control effect (used by tests and by a boss's death
    /// break, so nothing is left frozen after the horde flees).
    #[inline]
    pub fn clear_control(&mut self) {
        self.stun = 0.0;
        self.slow_mult = 1.0;
        self.slow_t = 0.0;
    }

    /// [`Enemy::spawn`] with a campaign level's [`EnemyMods`] applied.
    ///
    /// Draws exactly the same rng values as [`Enemy::spawn`] (it delegates to
    /// it), so a later level changes numbers, never the rng stream.
    pub fn spawn_with_mods(
        kind: EnemyKind,
        pos: Vec2,
        hp_mult: f32,
        mods: EnemyMods,
        rng: &mut impl Rng,
    ) -> Self {
        let mut e = Enemy::spawn(kind, pos, hp_mult, rng);
        e.damage = kind.damage() * mods.damage;
        e.speed_scale = mods.speed;
        e
    }

    /// Steer toward the player, routing around walls via the flow field.
    /// `flow_dir` is the pathfinding direction at this enemy's cell (ZERO
    /// inside the goal cell / off-grid, in which case we head straight at the
    /// player).
    pub fn steer(&mut self, target: Vec2, flow_dir: Vec2, dt: f32, speed_mult: f32) {
        // Crowd control (the Sonic Squeak's ring): a stunned body does not steer
        // and its momentum bleeds off fast, so it stops dead instead of sliding
        // past the rat. The slow is a straight multiplier on the per-kind speed
        // below. Both are no-ops at their defaults (0 / 1.0), so the existing
        // simulation is byte-for-byte unchanged.
        if self.tick_control(dt) {
            return;
        }

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
        let speed = self.kind.speed() * self.speed_scale * speed_mult * self.slow_mult;
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
    use crate::game::nav::{test_tiles, FlowField, NavBudget};
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
        let wall = Rectf::new(96.0, 32.0, 16.0, 80.0);
        let tiles = test_tiles(20, 20, &[wall]);
        let mut f = FlowField::new(&tiles);
        let player = Vec2::new(122.0, 73.0);
        f.update(&tiles, player, NavBudget::FULL);
        let mut rng = StdRng::seed_from_u64(3);
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::new(94.0, 73.0), 1.0, &mut rng);
        assert!(
            e.pos.dist(player) <= 30.0,
            "the case under test is a short hop"
        );
        let flow = f.dir_at(collision::SizeClass::Small, e.pos);
        assert!(flow.len() > 0.5, "the flow should route around the prop");
        let before = e.pos.dist(player);
        // The wall is five cells tall, so the detour around it is longer than
        // half a second of a skitterer's life: give it five.
        for _ in 0..300 {
            let flow = f.dir_at(collision::SizeClass::Small, e.pos);
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
        let wall = Rectf::new(96.0, 32.0, 16.0, 80.0);
        let walls = [wall];
        let tiles = test_tiles(20, 20, &walls);
        let mut f = FlowField::new(&tiles);
        let player = Vec2::new(200.0, 73.0);
        f.update(&tiles, player, NavBudget::FULL);
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
                let flow = f.dir_at(collision::SizeClass::Small, e.pos);
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
        //
        // The map is 3200 × 2560 px now, so a skitterer (30 px/s) cannot cross
        // the whole arena in 60 s of simulated time: the enemies are scattered
        // inside a ring around the rat — the director spawns them a few dozen
        // pixels out, and the flow field's fine window covers 384 px — which
        // still exercises walls, corners and doorways. (Beyond the fine window
        // the coarse block field is best-effort: a point whose block-graph exit
        // needs a detour *inside* its own 8 × 8-cell block can stall.)
        for theme in Theme::all() {
            let level = Level::new(theme);
            let mut f = FlowField::new(&level.tiles);
            let player = level.arena * 0.5;
            f.update(&level.tiles, player, NavBudget::FULL);
            let mut rng = StdRng::seed_from_u64(5);
            let mut enemies: Vec<Enemy> = (0..12)
                .map(|_| {
                    let angle = rng.random_range(0.0..std::f32::consts::TAU);
                    let radius = rng.random_range(60.0..350.0);
                    let p = player + Vec2::from_angle(angle) * radius;
                    Enemy::spawn(EnemyKind::Skitterer, p, 1.0, &mut rng)
                })
                .collect();
            let dt = 1.0 / 60.0;
            for _ in 0..60 * 60 {
                for e in enemies.iter_mut() {
                    let flow = f.dir_at(collision::SizeClass::Small, e.pos);
                    e.steer(player, flow, dt, 1.0);
                    e.pos = e
                        .pos
                        .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
                    e.pos = level.tiles.resolve_circle(
                        e.pos,
                        e.radius,
                        collision::SizeClass::of_enemy(e.kind),
                        None,
                    );
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
