//! XP gems and rare pickups dropped by enemies.

use crate::math::Vec2;
use crate::render::palette::Rgb;

#[derive(Clone)]
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PickupKind {
    Heal,
    Magnet,
    Nuke,
    // ---- Wave 3 pocket finds (WAVE3_DESIGN §10), appended after the three
    // legacy kinds so every existing discriminant and match keeps its place.
    /// Bottle Cap: absorbs the next damaging hit (12 s).
    BottleCap,
    /// Wind-up Mouse: a brass key that ordinary enemies chase (5 s).
    WindupMouse,
    /// Pocket Watch: hostile pellets crawl at 40 % speed (4 s).
    PocketWatch,
    /// Clean Socks: ready the dash and ignore sticky boards (6 s).
    CleanSocks,
}

impl PickupKind {
    pub fn color(self) -> Rgb {
        match self {
            PickupKind::Heal => (90, 255, 120),
            PickupKind::Magnet => (120, 180, 255),
            PickupKind::Nuke => (255, 120, 90),
            // Tin with a white rim.
            PickupKind::BottleCap => (232, 226, 206),
            // Brass key.
            PickupKind::WindupMouse => (232, 190, 96),
            // Cold clock face blue.
            PickupKind::PocketWatch => (150, 210, 226),
            // Fresh-laundry white with a little warmth.
            PickupKind::CleanSocks => (240, 240, 232),
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            PickupKind::Heal => "+",
            PickupKind::Magnet => "M",
            PickupKind::Nuke => "*",
            PickupKind::BottleCap => "o",
            PickupKind::WindupMouse => "k",
            PickupKind::PocketWatch => "w",
            PickupKind::CleanSocks => "s",
        }
    }

    /// Two-letter tag the HUD's timed-buff strip prints next to the seconds.
    pub fn buff_tag(self) -> &'static str {
        match self {
            PickupKind::BottleCap => "B",
            PickupKind::WindupMouse => "M",
            PickupKind::PocketWatch => "P",
            PickupKind::CleanSocks => "S",
            // The three legacy pickups are instant and have no timer.
            PickupKind::Heal | PickupKind::Magnet | PickupKind::Nuke => "",
        }
    }

    /// The pickup's name on a HUD callout / the design's label.
    pub fn label(self) -> &'static str {
        match self {
            PickupKind::Heal => "MEDKIT",
            PickupKind::Magnet => "MAGNET",
            PickupKind::Nuke => "PURGE",
            PickupKind::BottleCap => "BOTTLE CAP",
            PickupKind::WindupMouse => "WIND-UP MOUSE",
            PickupKind::PocketWatch => "POCKET WATCH",
            PickupKind::CleanSocks => "CLEAN SOCKS",
        }
    }
}

#[derive(Clone)]
pub struct Pickup {
    pub pos: Vec2,
    pub kind: PickupKind,
    pub bob: f32,
}
