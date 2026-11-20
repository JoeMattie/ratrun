//! Auto-firing weapons. Each weapon has a cooldown and a firing pattern;
//! the actual spawning lives in `world.rs` so it can see all entities.

use crate::render::palette::Rgb;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WeaponKind {
    Gnaw,        // homing dart at nearest enemy
    CheeseSpray, // forward spread of darts
    SqueakNova,  // radial burst in all directions
    SporeOrbit,  // orbiting projectiles around the player
    TailWhip,    // melee aura pulse around the player
    AcidSpit,    // lobbed shot that leaves a damaging pool
}

impl WeaponKind {
    pub fn all() -> [WeaponKind; 6] {
        [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::SqueakNova,
            WeaponKind::SporeOrbit,
            WeaponKind::TailWhip,
            WeaponKind::AcidSpit,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Gnaw",
            WeaponKind::CheeseSpray => "Cheese Spray",
            WeaponKind::SqueakNova => "Squeak Nova",
            WeaponKind::SporeOrbit => "Spore Orbit",
            WeaponKind::TailWhip => "Tail Whip",
            WeaponKind::AcidSpit => "Acid Spit",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Fires a dart at the nearest foe.",
            WeaponKind::CheeseSpray => "Scatters darts in a forward arc.",
            WeaponKind::SqueakNova => "Bursts shots in every direction.",
            WeaponKind::SporeOrbit => "Spores circle and shred nearby foes.",
            WeaponKind::TailWhip => "Pulses damage to everything close.",
            WeaponKind::AcidSpit => "Lobs acid that pools and burns.",
        }
    }

    pub fn flavor(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Teeth are the original weapon.",
            WeaponKind::CheeseSpray => "Weaponized dairy. Don't ask.",
            WeaponKind::SqueakNova => "A scream pitched past hearing. It still hurts.",
            WeaponKind::SporeOrbit => "Fungal hitchhikers from Vat 9, now loyal.",
            WeaponKind::TailWhip => "Forty grams of pure indignation.",
            WeaponKind::AcidSpit => "Stomach contents, redirected.",
        }
    }

    pub fn color(self) -> Rgb {
        match self {
            WeaponKind::Gnaw => (255, 240, 180),
            WeaponKind::CheeseSpray => (255, 210, 90),
            WeaponKind::SqueakNova => (200, 160, 255),
            WeaponKind::SporeOrbit => (140, 255, 180),
            WeaponKind::TailWhip => (255, 150, 200),
            WeaponKind::AcidSpit => (160, 255, 90),
        }
    }

    pub fn max_level(self) -> u8 {
        5
    }

    /// Base cooldown (seconds) before fire-rate stats are applied.
    pub fn base_cooldown(self) -> f32 {
        match self {
            WeaponKind::Gnaw => 0.55,
            WeaponKind::CheeseSpray => 0.95,
            WeaponKind::SqueakNova => 1.8,
            WeaponKind::SporeOrbit => 4.0,
            WeaponKind::TailWhip => 1.1,
            WeaponKind::AcidSpit => 1.5,
        }
    }

    /// Base damage per hit before weapon levels, damage passives and the
    /// player's fire-rate stats.
    ///
    /// Raised by ~60% across the board (b3-balance): the old numbers did
    /// ~9 dps into a 6-14 hp skitterer, i.e. ~0.5 bodies/s, against a spawn
    /// curve that reaches 3.5/s by 60 s. The XP engine could not start, so a
    /// level-1 build fell behind from the first minute and only the runs that
    /// survived that cliff snowballed. These values let a starting Gnaw keep
    /// its screen clear at a modest trickle, which is what makes the first
    /// minute teach instead of execute.
    pub fn base_damage(self) -> f32 {
        match self {
            WeaponKind::Gnaw => 8.0,
            WeaponKind::CheeseSpray => 6.5,
            WeaponKind::SqueakNova => 8.0,
            WeaponKind::SporeOrbit => 6.5,
            WeaponKind::TailWhip => 11.0,
            WeaponKind::AcidSpit => 5.0,
        }
    }
}

#[derive(Clone)]
pub struct Weapon {
    pub kind: WeaponKind,
    pub level: u8,
    pub timer: f32,
}

impl Weapon {
    pub fn new(kind: WeaponKind) -> Self {
        Self {
            kind,
            level: 1,
            timer: 0.3,
        }
    }

    pub fn is_max(&self) -> bool {
        self.level >= self.kind.max_level()
    }

    /// Damage scales ~25% per level.
    pub fn damage(&self) -> f32 {
        self.kind.base_damage() * (1.0 + 0.25 * (self.level - 1) as f32)
    }

    /// Cooldown shrinks slightly with level (6% faster per level).
    ///
    /// No floor is needed: the factor only reaches 0.76 at `max_level()`, and
    /// `level` is capped there by the upgrade loop, so a clamp could never
    /// bind. (Clamping the *factor* as before looked like a safety net but was
    /// dead code — 1.0 - 0.06 * 4 = 0.76 never drops under 0.5.)
    pub fn cooldown(&self) -> f32 {
        self.kind.base_cooldown() * (1.0 - 0.06 * (self.level - 1) as f32)
    }

    /// Extra projectiles granted by level (for multi-shot weapons).
    pub fn level_projectiles(&self) -> i32 {
        match self.kind {
            WeaponKind::CheeseSpray => 2 + self.level as i32,
            WeaponKind::SqueakNova => 6 + 2 * self.level as i32,
            WeaponKind::SporeOrbit => 2 + self.level as i32,
            _ => self.level as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_shrinks_6_percent_per_level() {
        for kind in WeaponKind::all() {
            let base = kind.base_cooldown();
            let mut prev = f32::INFINITY;
            for level in 1..=kind.max_level() {
                let mut w = Weapon::new(kind);
                w.level = level;
                let expect = base * (1.0 - 0.06 * (level - 1) as f32);
                assert!(
                    (w.cooldown() - expect).abs() < 1e-6,
                    "{kind:?} L{level}: {} != {expect}",
                    w.cooldown()
                );
                assert!(w.cooldown() < prev, "{kind:?} L{level} must be faster");
                prev = w.cooldown();
            }
            // The removed clamp was unreachable: the factor bottoms out at
            // 0.76, well above the old 0.5 floor.
            let mut w = Weapon::new(kind);
            w.level = kind.max_level();
            assert!((w.cooldown() / base - 0.76).abs() < 1e-6);
            assert!(w.cooldown() > base * 0.5);
        }
    }

    #[test]
    fn level_one_cooldown_is_the_base() {
        for kind in WeaponKind::all() {
            let w = Weapon::new(kind);
            assert_eq!(w.cooldown(), kind.base_cooldown());
            assert_eq!(w.damage(), kind.base_damage());
        }
    }
}
