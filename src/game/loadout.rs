//! Player loadout: owned weapons, passive stats, and the level-up
//! upgrade-choice generation / application.

use rand::seq::SliceRandom;
use rand::Rng;

use super::weapon::{Weapon, WeaponKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PassiveKind {
    MaxHp,
    MoveSpeed,
    Damage,
    FireRate,
    Magnet,
    Regen,
    Armor,
}

impl PassiveKind {
    pub fn all() -> [PassiveKind; 7] {
        [
            PassiveKind::MaxHp,
            PassiveKind::MoveSpeed,
            PassiveKind::Damage,
            PassiveKind::FireRate,
            PassiveKind::Magnet,
            PassiveKind::Regen,
            PassiveKind::Armor,
        ]
    }
    pub fn name(self) -> &'static str {
        match self {
            PassiveKind::MaxHp => "Fat Reserves",
            PassiveKind::MoveSpeed => "Quick Paws",
            PassiveKind::Damage => "Sharp Teeth",
            PassiveKind::FireRate => "Frenzy",
            PassiveKind::Magnet => "Whiskers",
            PassiveKind::Regen => "Resilience",
            PassiveKind::Armor => "Thick Hide",
        }
    }
    pub fn desc(self) -> &'static str {
        match self {
            PassiveKind::MaxHp => "+20 max HP (and heal).",
            PassiveKind::MoveSpeed => "+12% move speed.",
            PassiveKind::Damage => "+15% damage.",
            PassiveKind::FireRate => "+12% fire rate.",
            PassiveKind::Magnet => "+40% pickup radius.",
            PassiveKind::Regen => "+0.5 HP/s regen.",
            PassiveKind::Armor => "+1 armor (flat dmg cut).",
        }
    }
}

#[derive(Clone)]
pub struct Stats {
    pub max_hp: f32,
    pub move_speed: f32,
    pub damage_mult: f32,
    pub fire_rate_mult: f32,
    pub magnet: f32,
    pub regen: f32,
    pub armor: f32,
    pub dash_cd: f32,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            max_hp: 100.0,
            move_speed: 58.0,
            damage_mult: 1.0,
            fire_rate_mult: 1.0,
            magnet: 22.0,
            regen: 0.0,
            armor: 0.0,
            dash_cd: 2.0,
        }
    }
}

#[derive(Clone)]
pub enum Upgrade {
    NewWeapon(WeaponKind),
    LevelWeapon(usize),
    Passive(PassiveKind),
}

pub struct Loadout {
    pub weapons: Vec<Weapon>,
    pub stats: Stats,
}

impl Loadout {
    pub fn new(start: WeaponKind) -> Self {
        Loadout {
            weapons: vec![Weapon::new(start)],
            stats: Stats::default(),
        }
    }

    pub fn has(&self, k: WeaponKind) -> bool {
        self.weapons.iter().any(|w| w.kind == k)
    }

    /// Title + description for an upgrade card.
    pub fn describe(&self, up: &Upgrade) -> (String, String) {
        match up {
            Upgrade::NewWeapon(k) => (format!("{} (NEW)", k.name()), k.desc().to_string()),
            Upgrade::LevelWeapon(i) => {
                let w = &self.weapons[*i];
                (
                    format!("{} Lv{}→{}", w.kind.name(), w.level, w.level + 1),
                    w.kind.desc().to_string(),
                )
            }
            Upgrade::Passive(p) => (p.name().to_string(), p.desc().to_string()),
        }
    }

    /// Build up to three distinct level-up choices.
    pub fn generate_choices(&self, rng: &mut impl Rng) -> Vec<Upgrade> {
        let mut pool: Vec<Upgrade> = Vec::new();

        // Level up existing weapons that aren't maxed.
        for (i, w) in self.weapons.iter().enumerate() {
            if !w.is_max() {
                pool.push(Upgrade::LevelWeapon(i));
            }
        }
        // Offer new weapons (cap loadout at 6 distinct).
        if self.weapons.len() < 6 {
            for k in WeaponKind::all() {
                if !self.has(k) {
                    pool.push(Upgrade::NewWeapon(k));
                }
            }
        }
        // Always offer some passives.
        for p in PassiveKind::all() {
            pool.push(Upgrade::Passive(p));
        }

        pool.shuffle(rng);
        pool.into_iter().take(3).collect()
    }

    /// Apply an upgrade. Returns extra HP to grant (heal-on-pick effects).
    pub fn apply(&mut self, up: &Upgrade) -> f32 {
        match up {
            Upgrade::NewWeapon(k) => {
                if !self.has(*k) {
                    self.weapons.push(Weapon::new(*k));
                }
                0.0
            }
            Upgrade::LevelWeapon(i) => {
                if let Some(w) = self.weapons.get_mut(*i) {
                    if !w.is_max() {
                        w.level += 1;
                    }
                }
                0.0
            }
            Upgrade::Passive(p) => match p {
                PassiveKind::MaxHp => {
                    self.stats.max_hp += 20.0;
                    20.0
                }
                PassiveKind::MoveSpeed => {
                    self.stats.move_speed *= 1.12;
                    0.0
                }
                PassiveKind::Damage => {
                    self.stats.damage_mult += 0.15;
                    0.0
                }
                PassiveKind::FireRate => {
                    self.stats.fire_rate_mult += 0.12;
                    0.0
                }
                PassiveKind::Magnet => {
                    self.stats.magnet *= 1.4;
                    0.0
                }
                PassiveKind::Regen => {
                    self.stats.regen += 0.5;
                    0.0
                }
                PassiveKind::Armor => {
                    self.stats.armor += 1.0;
                    0.0
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn choices_are_valid_and_capped() {
        let lo = Loadout::new(WeaponKind::Gnaw);
        let mut rng = StdRng::seed_from_u64(7);
        let c = lo.generate_choices(&mut rng);
        assert!(c.len() <= 3 && !c.is_empty());
    }

    #[test]
    fn applying_passive_changes_stats() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        let before = lo.stats.damage_mult;
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        assert!(lo.stats.damage_mult > before);
    }

    #[test]
    fn applying_new_weapon_adds_it() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        assert!(!lo.has(WeaponKind::TailWhip));
        lo.apply(&Upgrade::NewWeapon(WeaponKind::TailWhip));
        assert!(lo.has(WeaponKind::TailWhip));
    }

    #[test]
    fn leveling_weapon_respects_max() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        for _ in 0..20 {
            lo.apply(&Upgrade::LevelWeapon(0));
        }
        assert_eq!(lo.weapons[0].level, WeaponKind::Gnaw.max_level());
    }
}
