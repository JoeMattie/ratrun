//! XP gems and rare pickups dropped by enemies.

use crate::math::Vec2;
use crate::render::palette::Rgb;

pub struct Gem {
    pub pos: Vec2,
    pub vel: Vec2,
    pub value: u32,
    pub magnetized: bool,
    pub color: Rgb,
}

impl Gem {
    pub fn new(pos: Vec2, value: u32) -> Self {
        // Bigger gems are warmer/brighter.
        let color = if value >= 25 {
            (255, 220, 90)
        } else if value >= 5 {
            (120, 230, 255)
        } else {
            (90, 200, 255)
        };
        Self {
            pos,
            vel: Vec2::ZERO,
            value,
            magnetized: false,
            color,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickupKind {
    Heal,
    Magnet,
    Nuke,
}

impl PickupKind {
    pub fn color(self) -> Rgb {
        match self {
            PickupKind::Heal => (90, 255, 120),
            PickupKind::Magnet => (120, 180, 255),
            PickupKind::Nuke => (255, 120, 90),
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            PickupKind::Heal => "+",
            PickupKind::Magnet => "M",
            PickupKind::Nuke => "*",
        }
    }
}

pub struct Pickup {
    pub pos: Vec2,
    pub kind: PickupKind,
    pub bob: f32,
}
