//! XP gems and rare pickups dropped by enemies.

use bevy_ecs::prelude::Component;

use crate::math::Vec2;
use crate::render::palette::Rgb;

/// Minimum merge distance in units of the widest view's half-diagonal.
pub const MERGE_FROM: f32 = 1.5;
pub const MERGE_EVERY: f32 = 2.0;
pub const MERGE_CELL: f32 = 96.0;
pub const GEM_SOFT_CAP: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GemTier {
    Shard,
    Chip,
    Gem,
    Crystal,
    Core,
}

impl GemTier {
    pub const ALL: [Self; 5] = [Self::Shard, Self::Chip, Self::Gem, Self::Crystal, Self::Core];

    pub fn for_value(value: u32) -> Self {
        if value >= Self::Core.min_value() {
            Self::Core
        } else if value >= Self::Crystal.min_value() {
            Self::Crystal
        } else if value >= Self::Gem.min_value() {
            Self::Gem
        } else if value >= Self::Chip.min_value() {
            Self::Chip
        } else {
            Self::Shard
        }
    }

    pub const fn min_value(self) -> u32 {
        match self {
            Self::Shard => 1,
            Self::Chip => 5,
            Self::Gem => 25,
            Self::Crystal => 100,
            Self::Core => 500,
        }
    }

    pub const fn color(self) -> Rgb {
        match self {
            Self::Shard => (150, 240, 170),
            Self::Chip => (90, 210, 255),
            Self::Gem => (190, 120, 255),
            Self::Crystal => (255, 200, 70),
            Self::Core => (255, 80, 90),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Shard => "Shard",
            Self::Chip => "Chip",
            Self::Gem => "Gem",
            Self::Crystal => "Crystal",
            Self::Core => "Core",
        }
    }

    pub const fn color_name(self) -> &'static str {
        match self {
            Self::Shard => "pale green",
            Self::Chip => "cyan",
            Self::Gem => "violet",
            Self::Crystal => "amber",
            Self::Core => "crimson",
        }
    }

    pub const fn radius_px(self) -> i32 {
        match self {
            Self::Shard | Self::Chip => 1,
            Self::Gem => 2,
            Self::Crystal => 3,
            Self::Core => 4,
        }
    }
}

#[derive(Clone)]
pub struct Gem {
    pub pos: Vec2,
    pub vel: Vec2,
    pub value: u32,
    pub magnetized: bool,
    pub color: Rgb,
    /// Set on the tick this gem absorbs others, for the formation flash.
    pub merged: bool,
}

impl Gem {
    pub fn new(pos: Vec2, value: u32) -> Self {
        Self {
            pos,
            vel: Vec2::ZERO,
            value,
            magnetized: false,
            color: GemTier::for_value(value).color(),
            merged: false,
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
    /// Shrink Draught: a smaller hurt disc and faster movement (12 s).
    ShrinkDraught,
    /// White Coat: no incoming hits or weapon use (5 s).
    WhiteCoat,
    /// The Collar: double gem XP and faster heavy pursuers (16 s).
    TheCollar,
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
            PickupKind::ShrinkDraught => (192, 235, 255),
            PickupKind::WhiteCoat => (255, 255, 248),
            PickupKind::TheCollar => (255, 201, 100),
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
            PickupKind::ShrinkDraught => "T",
            PickupKind::WhiteCoat => "W",
            PickupKind::TheCollar => "K",
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
            PickupKind::ShrinkDraught => "T",
            PickupKind::WhiteCoat => "W",
            PickupKind::TheCollar => "K",
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
            PickupKind::ShrinkDraught => "SHRINK DRAUGHT",
            PickupKind::WhiteCoat => "WHITE COAT",
            PickupKind::TheCollar => "THE COLLAR",
        }
    }

    /// Every pickup kind, in enum order. The console's `pickup` command and
    /// its Tab completion read this table rather than a retyped list.
    pub const fn all() -> [PickupKind; 11] {
        [
            PickupKind::Heal,
            PickupKind::Magnet,
            PickupKind::Nuke,
            PickupKind::BottleCap,
            PickupKind::WindupMouse,
            PickupKind::PocketWatch,
            PickupKind::CleanSocks,
            PickupKind::GrowthSerum,
            PickupKind::ShrinkDraught,
            PickupKind::WhiteCoat,
            PickupKind::TheCollar,
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
            PickupKind::ShrinkDraught => "shrinkdraught",
            PickupKind::WhiteCoat => "whitecoat",
            PickupKind::TheCollar => "thecollar",
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
    use super::{GemTier, PickupKind};

    #[test]
    fn gem_tier_value_boundaries() {
        use GemTier::*;
        for (value, tier) in [
            (0, Shard), (1, Shard), (4, Shard), (5, Chip), (24, Chip),
            (25, Gem), (99, Gem), (100, Crystal), (499, Crystal),
            (500, Core), (u32::MAX, Core),
        ] {
            assert_eq!(GemTier::for_value(value), tier, "{value} XP");
        }
    }

    /// The table the console's `pickup` command, its Tab completion and the
    /// Library's registry all walk: every kind exactly once, in enum order.
    #[test]
    fn the_pickup_table_lists_every_kind_once_in_discriminant_order() {
        let all = PickupKind::all();
        assert_eq!(all.len(), 11, "one row per find");
        for (i, kind) in all.iter().enumerate() {
            assert_eq!(*kind as usize, i, "{kind:?} is out of order");
            if !kind.buff_tag().is_empty() {
                assert!(all[..i].iter().all(|prev| prev.buff_tag() != kind.buff_tag()));
            }
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
