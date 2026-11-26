//! Small 2D vector + scalar helpers used across the game.
//!
//! This is simulation code (movement, aim, spawn angles all read it), so every
//! trigonometry call here goes through [`crate::detmath`] instead of `std`'s
//! platform maths library: `Vec2::angle`, `from_angle` and `rotate` are the
//! funnel most of the simulation's trig flows through, so routing them here
//! makes those paths bit-identical on every target. `sqrt` (in `len`) and the
//! arithmetic are IEEE-exact and stay as they are.

use crate::detmath::DetF32;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    pub fn len_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    pub fn len(self) -> f32 {
        self.len_sq().sqrt()
    }

    pub fn normalized(self) -> Vec2 {
        let l = self.len();
        if l > 1e-6 {
            Vec2::new(self.x / l, self.y / l)
        } else {
            Vec2::ZERO
        }
    }

    pub fn clamp_len(self, max: f32) -> Vec2 {
        let l = self.len();
        if l > max {
            self * (max / l)
        } else {
            self
        }
    }

    pub fn dist(self, o: Vec2) -> f32 {
        (self - o).len()
    }

    pub fn dist_sq(self, o: Vec2) -> f32 {
        (self - o).len_sq()
    }

    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }

    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    pub fn angle(self) -> f32 {
        self.y.datan2(self.x)
    }

    pub fn from_angle(a: f32) -> Vec2 {
        let (s, c) = a.dsin_cos();
        Vec2::new(c, s)
    }

    pub fn rotate(self, a: f32) -> Vec2 {
        let (s, c) = a.dsin_cos();
        Vec2::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    pub fn clamp_box(self, min: Vec2, max: Vec2) -> Vec2 {
        Vec2::new(self.x.clamp(min.x, max.x), self.y.clamp(min.y, max.y))
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, o: Vec2) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unit_length() {
        let v = Vec2::new(3.0, 4.0).normalized();
        assert!((v.len() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn normalize_zero_is_zero() {
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
    }

    #[test]
    fn rotate_quarter_turn() {
        let v = Vec2::new(1.0, 0.0).rotate(std::f32::consts::FRAC_PI_2);
        assert!(v.x.abs() < 1e-5 && (v.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn clamp_len_caps() {
        let v = Vec2::new(10.0, 0.0).clamp_len(2.0);
        assert!((v.len() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn distance_matches() {
        assert!((Vec2::new(0.0, 0.0).dist(Vec2::new(3.0, 4.0)) - 5.0).abs() < 1e-5);
    }
}
