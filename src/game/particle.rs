//! Particle system — the visual "juice". Bursts, trails, explosions.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};
use rand::Rng;

#[derive(Clone, Copy)]
pub enum Ramp {
    Fire,
    Spark,
    Smoke,
    Gem,
    /// Two-color custom fade (start → end), e.g. enemy-tinted blood.
    Custom(Rgb, Rgb),
}

impl Ramp {
    /// Color at age fraction `t` (0 = freshly born, 1 = about to die).
    pub fn color_at(self, t: f32) -> Rgb {
        match self {
            Ramp::Fire => palette::ramp(
                &[
                    (0.0, (255, 255, 210)),
                    (0.25, (255, 200, 70)),
                    (0.55, (235, 90, 25)),
                    (1.0, (60, 18, 10)),
                ],
                t,
            ),
            Ramp::Spark => palette::ramp(
                &[
                    (0.0, (255, 255, 255)),
                    (0.4, (255, 235, 140)),
                    (1.0, (120, 80, 30)),
                ],
                t,
            ),
            Ramp::Smoke => palette::ramp(
                &[(0.0, (120, 120, 130)), (1.0, (24, 24, 28))],
                t,
            ),
            Ramp::Gem => palette::ramp(
                &[
                    (0.0, (220, 255, 255)),
                    (0.5, (90, 230, 255)),
                    (1.0, (30, 90, 160)),
                ],
                t,
            ),
            Ramp::Custom(a, b) => palette::scale(palette::mix(a, b, t), 1.0 - t * 0.6),
        }
    }
}

pub struct Particle {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    pub max_life: f32,
    pub drag: f32,
    pub gravity: f32,
    pub ramp: Ramp,
    pub additive: bool,
}

#[derive(Default)]
pub struct Particles {
    pub items: Vec<Particle>,
    cap: usize,
}

impl Particles {
    pub fn new(cap: usize) -> Self {
        Self {
            items: Vec::with_capacity(1024),
            cap,
        }
    }

    pub fn push(&mut self, p: Particle) {
        if self.items.len() >= self.cap {
            // Oldest-first recycling keeps the framerate bounded.
            self.items.remove(0);
        }
        self.items.push(p);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Radial burst of `n` particles.
    #[allow(clippy::too_many_arguments)]
    pub fn burst(
        &mut self,
        rng: &mut impl Rng,
        pos: Vec2,
        n: usize,
        speed: f32,
        spread: f32,
        ramp: Ramp,
        life: f32,
        additive: bool,
    ) {
        for _ in 0..n {
            let a = rng.gen_range(0.0..std::f32::consts::TAU);
            let s = speed * rng.gen_range(0.3..1.0);
            let l = life * rng.gen_range(0.6..1.2);
            let jitter = if spread > 0.0 {
                Vec2::new(rng.gen_range(-spread..spread), rng.gen_range(-spread..spread))
            } else {
                Vec2::ZERO
            };
            self.push(Particle {
                pos: pos + jitter,
                vel: Vec2::from_angle(a) * s,
                life: l,
                max_life: l,
                drag: 3.0,
                gravity: 0.0,
                ramp,
                additive,
            });
        }
    }

    /// Directional cone (muzzle flash, dash trail).
    #[allow(clippy::too_many_arguments)]
    pub fn cone(
        &mut self,
        rng: &mut impl Rng,
        pos: Vec2,
        dir: Vec2,
        n: usize,
        speed: f32,
        ramp: Ramp,
        life: f32,
    ) {
        let base = dir.angle();
        for _ in 0..n {
            let a = base + rng.gen_range(-0.5..0.5);
            let s = speed * rng.gen_range(0.4..1.1);
            let l = life * rng.gen_range(0.5..1.0);
            self.push(Particle {
                pos,
                vel: Vec2::from_angle(a) * s,
                life: l,
                max_life: l,
                drag: 5.0,
                gravity: 0.0,
                ramp,
                additive: true,
            });
        }
    }

    pub fn update(&mut self, dt: f32) {
        for p in self.items.iter_mut() {
            p.life -= dt;
            let drag = (1.0 - p.drag * dt).max(0.0);
            p.vel = p.vel * drag;
            p.vel.y += p.gravity * dt;
            p.pos += p.vel * dt;
        }
        self.items.retain(|p| p.life > 0.0);
    }

    pub fn draw(&self, pb: &mut PixelBuffer, cam: Vec2) {
        for p in &self.items {
            let t = 1.0 - (p.life / p.max_life).clamp(0.0, 1.0);
            let c = p.ramp.color_at(t);
            let x = (p.pos.x - cam.x).round() as i32;
            let y = (p.pos.y - cam.y).round() as i32;
            if p.additive {
                pb.plot_add(x, y, c);
            } else {
                pb.plot(x, y, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn cap_is_respected() {
        let mut ps = Particles::new(50);
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..20 {
            ps.burst(&mut rng, Vec2::ZERO, 20, 40.0, 1.0, Ramp::Spark, 1.0, true);
        }
        assert!(ps.len() <= 50);
    }

    #[test]
    fn particles_expire() {
        let mut ps = Particles::new(100);
        let mut rng = StdRng::seed_from_u64(2);
        ps.burst(&mut rng, Vec2::ZERO, 10, 10.0, 0.0, Ramp::Fire, 0.5, false);
        assert_eq!(ps.len(), 10);
        ps.update(1.0); // exceeds max life
        assert_eq!(ps.len(), 0);
    }
}
