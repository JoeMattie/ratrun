//! XP gems and rare pickups dropped by enemies.

use bevy_ecs::prelude::Component;

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
    // ---- Batch 13: the giant-rat powerup, appended after the wave-3 finds
    // so every existing discriminant and match keeps its place.
    /// Growth Serum: the rat is drawn five times its size and stomps the
    /// ordinary horde under it (20 s).
    GrowthSerum,
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
            // Hot lab pink-magenta: the one find that is a colour no floor has.
            PickupKind::GrowthSerum => (255, 88, 196),
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
            PickupKind::GrowthSerum => "G",
        }
    }

    /// One-letter tag the HUD's timed-buff strip prints next to the seconds
    /// (empty for the instant pickups, which have no timer).
    pub fn buff_tag(self) -> &'static str {
        match self {
            PickupKind::BottleCap => "B",
            PickupKind::WindupMouse => "M",
            PickupKind::PocketWatch => "P",
            PickupKind::CleanSocks => "S",
            PickupKind::GrowthSerum => "G",
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
            PickupKind::GrowthSerum => "GROWTH SERUM",
        }
    }

    /// Every pickup kind, in enum order. The console's `pickup` command and
    /// its Tab completion read this table rather than a retyped list.
    pub fn all() -> [PickupKind; 8] {
        [
            PickupKind::Heal,
            PickupKind::Magnet,
            PickupKind::Nuke,
            PickupKind::BottleCap,
            PickupKind::WindupMouse,
            PickupKind::PocketWatch,
            PickupKind::CleanSocks,
            PickupKind::GrowthSerum,
        ]
    }

    /// The enum-variant name in lowercase, so the console's `pickup` command
    /// accepts both `heal` and the display label `medkit`.
    pub fn name(self) -> &'static str {
        match self {
            PickupKind::Heal => "heal",
            PickupKind::Magnet => "magnet",
            PickupKind::Nuke => "nuke",
            PickupKind::BottleCap => "bottlecap",
            PickupKind::WindupMouse => "windupmouse",
            PickupKind::PocketWatch => "pocketwatch",
            PickupKind::CleanSocks => "cleansocks",
            PickupKind::GrowthSerum => "growthserum",
        }
    }
}

#[derive(Clone)]
pub struct Pickup {
    pub pos: Vec2,
    pub kind: PickupKind,
    pub bob: f32,
}

#[derive(Component)]
pub struct Gold {
    pub pos: Vec2,
    pub vel: Vec2,
    pub value: u32,
    pub magnetized: bool,
}

impl Gold {
    pub fn new(pos: Vec2, value: u32) -> Self {
        Self { pos, vel: Vec2::ZERO, value, magnetized: false }
    }
}

#[derive(Component)]
pub struct Chest {
    pub pos: Vec2,
}

#[cfg(test)]
mod tests {
    use super::PickupKind;

    /// The table the console's `pickup` command, its Tab completion and the
    /// Library's registry all walk: every kind exactly once, in enum order, and
    /// the batch-13 Growth Serum last.
    #[test]
    fn the_pickup_table_lists_every_kind_once_and_ends_with_the_serum() {
        let all = PickupKind::all();
        assert_eq!(all.len(), 8, "one row per find");
        assert_eq!(*all.last().unwrap(), PickupKind::GrowthSerum);
        for (i, kind) in all.iter().enumerate() {
            assert!(
                !all[..i].contains(kind),
                "{kind:?} is listed twice in PickupKind::all()"
            );
            assert!(!kind.name().is_empty(), "{kind:?} has no console name");
            assert!(!kind.label().is_empty(), "{kind:?} has no label");
            // The label is the ALL CAPS callout name (STYLE: panel/HUD strings
            // are all caps), and the console name is the lowercase variant.
            assert_eq!(kind.label(), kind.label().to_uppercase(), "{kind:?} label");
            assert_eq!(kind.name(), kind.name().to_lowercase(), "{kind:?} name");
        }
    }

    /// The Growth Serum's own row: the HUD tag `G`, the map glyph `G`, the
    /// `GROWTH SERUM` label, the `growthserum` console name and a colour no
    /// other find wears.
    #[test]
    fn the_growth_serum_reads_as_its_own_find() {
        let serum = PickupKind::GrowthSerum;
        assert_eq!(serum.buff_tag(), "G");
        assert_eq!(serum.glyph(), "G");
        assert_eq!(serum.label(), "GROWTH SERUM");
        assert_eq!(serum.name(), "growthserum");
        for kind in PickupKind::all() {
            if kind != serum {
                assert_ne!(kind.color(), serum.color(), "{kind:?} shares the serum's colour");
                assert_ne!(kind.glyph(), serum.glyph(), "{kind:?} shares the serum's glyph");
            }
        }
    }
}
