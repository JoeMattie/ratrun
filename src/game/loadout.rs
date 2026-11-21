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
            PassiveKind::Magnet => "+30% pickup radius.",
            PassiveKind::Regen => "+0.5 HP/s regen.",
            PassiveKind::Armor => "+1 armor (flat dmg cut).",
        }
    }

    /// How many stacks of this passive a build may own.
    ///
    /// Caps bound the snowball: a run that reaches level 60-100 used to stack
    /// Sharp Teeth hundreds of times (x266 over the 30-run baseline) and deal
    /// 25-80x its base damage, which made the boss pool meaningless and turned
    /// survival into a binary "did the build start". Every cap here is chosen
    /// so a maxed passive multiplies its stat by roughly 2-3x and nothing more:
    ///
    /// | passive | effect per stack | cap | max multiplier |
    /// |---|---|---|---|
    /// | MaxHp | +20 HP | 8 | 260 HP (x2.6) |
    /// | MoveSpeed | x1.12 | 6 | x1.97 |
    /// | Damage | +15% | 8 | x2.20 |
    /// | FireRate | +12% | 8 | x1.96 |
    /// | Magnet | x1.3 | 4 | x2.86 |
    /// | Regen | +0.5 HP/s | 6 | 3.0 HP/s |
    /// | Armor | +1 flat | 6 | -6 damage per hit |
    ///
    /// When every passive is capped the level-up pool offers them as
    /// *Bandages* instead (see [`Loadout::apply`]), so a card is never dead.
    pub fn max_stacks(self) -> u32 {
        match self {
            PassiveKind::MaxHp => 8,
            PassiveKind::MoveSpeed => 6,
            PassiveKind::Damage => 8,
            PassiveKind::FireRate => 8,
            PassiveKind::Magnet => 4,
            PassiveKind::Regen => 6,
            PassiveKind::Armor => 6,
        }
    }

    pub fn flavor(self) -> &'static str {
        match self {
            PassiveKind::MaxHp => "Winter is always coming.",
            PassiveKind::MoveSpeed => "Twitchier than the cameras.",
            PassiveKind::Damage => "Filed sharp on rebar.",
            PassiveKind::FireRate => "More is, itself, a strategy.",
            PassiveKind::Magnet => "Feel the room before you enter it.",
            PassiveKind::Regen => "Bred to survive worse than this.",
            PassiveKind::Armor => "Scar tissue, all the way down.",
        }
    }
}

#[derive(Clone, Debug)]
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
            max_hp: 130.0,
            move_speed: 58.0,
            damage_mult: 1.0,
            fire_rate_mult: 1.0,
            magnet: 22.0,
            regen: 0.4,
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

/// Fraction of max HP a capped passive restores when it is played as a
/// bandage (see [`PassiveKind::max_stacks`]).
const BANDAGE_HEAL_FRAC: f32 = 0.25;

/// Apply one stack of `p` to `stats`, returning the HP the pick heals.
///
/// This is the single source of truth for what a passive does: [`Loadout::apply`]
/// uses it for a live level-up and [`Loadout::from_parts`] replays it when a
/// carried loadout is rebuilt for the next campaign level, so the two can never
/// drift apart.
fn apply_passive_stack(stats: &mut Stats, p: PassiveKind) -> f32 {
    match p {
        PassiveKind::MaxHp => {
            stats.max_hp += 20.0;
            20.0
        }
        PassiveKind::MoveSpeed => {
            stats.move_speed *= 1.12;
            0.0
        }
        PassiveKind::Damage => {
            stats.damage_mult += 0.15;
            0.0
        }
        PassiveKind::FireRate => {
            stats.fire_rate_mult += 0.12;
            0.0
        }
        PassiveKind::Magnet => {
            stats.magnet *= 1.3;
            0.0
        }
        PassiveKind::Regen => {
            stats.regen += 0.5;
            0.0
        }
        PassiveKind::Armor => {
            stats.armor += 1.0;
            0.0
        }
    }
}

#[derive(Clone)]
pub struct Loadout {
    pub weapons: Vec<Weapon>,
    /// Owned passive stacks, in **first-acquisition order**, one entry per
    /// `PassiveKind`, each count `>= 1`.
    ///
    /// `Stats` alone cannot be carried between campaign levels: `max_hp` is an
    /// absolute value and `move_speed`/`magnet` are compounded, so there is no
    /// way to recover "how many stacks of what" from the numbers. This list is
    /// that record (see [`super::campaign`]). The order is the order the player
    /// picked them in, so [`Loadout::from_parts`] replays exactly the same
    /// floating-point operations and a carried loadout keeps its numbers
    /// bit-for-bit when every passive is kept.
    pub passives: Vec<(PassiveKind, u32)>,
    pub stats: Stats,
}

/// `Weapon` lives in `game::weapon` and does not derive `Debug`; this prints
/// the parts the campaign cares about (`"Gnaw Lv3, Sharp Teeth x2"`).
impl std::fmt::Debug for Loadout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let weapons: Vec<String> = self
            .weapons
            .iter()
            .map(|w| format!("{} Lv{}", w.kind.name(), w.level))
            .collect();
        let passives: Vec<String> = self
            .passives
            .iter()
            .map(|(p, n)| format!("{} x{n}", p.name()))
            .collect();
        write!(
            f,
            "Loadout {{ weapons: [{}], passives: [{}] }}",
            weapons.join(", "),
            passives.join(", ")
        )
    }
}

impl Loadout {
    /// The weapon every run starts with (and the one a carry-over falls back to
    /// when the player keeps no weapon).
    pub const START_WEAPON: WeaponKind = WeaponKind::Gnaw;

    pub fn new(start: WeaponKind) -> Self {
        Loadout {
            weapons: vec![Weapon::new(start)],
            passives: Vec::new(),
            stats: Stats::default(),
        }
    }

    /// The default level-1 loadout ([`Loadout::START_WEAPON`], no passives).
    pub fn starting() -> Self {
        Loadout::new(Loadout::START_WEAPON)
    }

    /// Rebuild a loadout from carried weapons and passive stacks, replaying the
    /// passive effects onto a fresh [`Stats`].
    ///
    /// `passives` is kept in the order given (the campaign carries the previous
    /// level's acquisition order) and every stat is derived here, so a carried
    /// loadout is fully described by its parts — no stale numbers cross levels.
    pub fn from_parts(weapons: Vec<Weapon>, passives: Vec<(PassiveKind, u32)>) -> Self {
        let mut stats = Stats::default();
        for &(kind, stacks) in &passives {
            // Defensive: the cap is a property of the passive, so a carried
            // record can never buy more than it here either.
            for _ in 0..stacks.min(kind.max_stacks()) {
                apply_passive_stack(&mut stats, kind);
            }
        }
        Loadout {
            weapons,
            passives,
            stats,
        }
    }

    pub fn has(&self, k: WeaponKind) -> bool {
        self.weapons.iter().any(|w| w.kind == k)
    }

    /// How many stacks of `p` the player owns (0 when none).
    pub fn stacks(&self, p: PassiveKind) -> u32 {
        self.passives
            .iter()
            .find(|(kind, _)| *kind == p)
            .map(|(_, stacks)| *stacks)
            .unwrap_or(0)
    }

    /// Add one stack of `p` to the ownership record (no stat effect).
    fn bump_stack(&mut self, p: PassiveKind) {
        if let Some(entry) = self.passives.iter_mut().find(|(kind, _)| *kind == p) {
            entry.1 += 1;
        } else {
            self.passives.push((p, 1));
        }
    }

    /// Title, mechanical description, and flavor line for an upgrade card.
    pub fn describe(&self, up: &Upgrade) -> (String, String, String) {
        match up {
            Upgrade::NewWeapon(k) => (
                format!("{} (NEW)", k.name()),
                k.desc().to_string(),
                k.flavor().to_string(),
            ),
            Upgrade::LevelWeapon(i) => {
                let w = &self.weapons[*i];
                (
                    format!("{} Lv{}→{}", w.kind.name(), w.level, w.level + 1),
                    w.kind.desc().to_string(),
                    w.kind.flavor().to_string(),
                )
            }
            Upgrade::Passive(p) => {
                if self.stacks(*p) >= p.max_stacks() {
                    (
                        format!("{} (capped)", p.name()),
                        format!(
                            "At the cap of {}: restores {:.0}% of max HP.",
                            p.max_stacks(),
                            BANDAGE_HEAL_FRAC * 100.0
                        ),
                        p.flavor().to_string(),
                    )
                } else {
                    (
                        p.name().to_string(),
                        format!("{} ({}/{})", p.desc(), self.stacks(*p) + 1, p.max_stacks()),
                        p.flavor().to_string(),
                    )
                }
            }
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
        // Passives that are still under their cap. A capped passive is not a
        // choice a player would ever want, so it is held back for the fallback
        // below instead of competing with a real upgrade.
        let mut capped: Vec<Upgrade> = Vec::new();
        for p in PassiveKind::all() {
            if self.stacks(p) >= p.max_stacks() {
                capped.push(Upgrade::Passive(p));
            } else {
                pool.push(Upgrade::Passive(p));
            }
        }
        // Every live card is preferred to a capped one. Only when the live pool
        // cannot fill the screen do the capped passives come back, as
        // bandages — and only as many as are needed. (Adding all of them to
        // the pool and shuffling, as this did, let bandages crowd out live
        // cards: with one passive left to cap, a level-up could offer three
        // heals instead of the weapon level-up.)
        pool.shuffle(rng);
        let mut choices: Vec<Upgrade> = pool.into_iter().take(3).collect();
        if choices.len() < 3 {
            capped.shuffle(rng);
            choices.extend(capped.into_iter().take(3 - choices.len()));
        }
        choices
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
            Upgrade::Passive(p) => {
                if self.stacks(*p) >= p.max_stacks() {
                    // Every card must stay useful: a passive at its cap is
                    // offered (only once the rest of the pool is thin) and
                    // played as a bandage instead of a dead stack.
                    return BANDAGE_HEAL_FRAC * self.stats.max_hp;
                }
                self.bump_stack(*p);
                apply_passive_stack(&mut self.stats, *p)
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
    fn choices_are_valid_and_capped() {
        let lo = Loadout::new(WeaponKind::Gnaw);
        let mut rng = StdRng::seed_from_u64(7);
        let c = lo.generate_choices(&mut rng);
        assert!(c.len() <= 3 && !c.is_empty());
    }

    /// Every weapon maxed, every passive at its cap: the pool is empty and the
    /// level-up must still fill the screen with bandages.
    fn exhausted_loadout() -> Loadout {
        let mut lo = Loadout::starting();
        for k in WeaponKind::all() {
            if !lo.has(k) {
                lo.apply(&Upgrade::NewWeapon(k));
            }
        }
        for i in 0..lo.weapons.len() {
            while !lo.weapons[i].is_max() {
                lo.apply(&Upgrade::LevelWeapon(i));
            }
        }
        for p in PassiveKind::all() {
            for _ in 0..p.max_stacks() {
                lo.apply(&Upgrade::Passive(p));
            }
        }
        lo
    }

    #[test]
    fn capped_passives_never_crowd_out_a_live_card() {
        // Everything capped except one passive: the one live card must always
        // be offered, never buried under three bandages.
        let mut lo = Loadout::starting();
        for k in WeaponKind::all() {
            if !lo.has(k) {
                lo.apply(&Upgrade::NewWeapon(k));
            }
        }
        for i in 0..lo.weapons.len() {
            while !lo.weapons[i].is_max() {
                lo.apply(&Upgrade::LevelWeapon(i));
            }
        }
        for p in PassiveKind::all() {
            let cap = if p == PassiveKind::Damage {
                p.max_stacks() - 1
            } else {
                p.max_stacks()
            };
            for _ in 0..cap {
                lo.apply(&Upgrade::Passive(p));
            }
        }
        for seed in 0..50 {
            let mut rng = StdRng::seed_from_u64(seed);
            let choices = lo.generate_choices(&mut rng);
            assert_eq!(choices.len(), 3, "a level-up always offers three cards");
            let live = choices
                .iter()
                .filter(|c| matches!(c, Upgrade::Passive(PassiveKind::Damage)))
                .count();
            assert_eq!(live, 1, "the one live card must always be offered");
            // …and the other two are bandages, not more capped passives than
            // the screen needs.
            for c in choices.iter().filter(|c| !matches!(c, Upgrade::Passive(PassiveKind::Damage))) {
                assert!(lo.describe(c).0.ends_with("(capped)"));
            }
        }
    }

    #[test]
    fn an_exhausted_pool_fills_three_bandages() {
        let lo = exhausted_loadout();
        for seed in 0..50 {
            let mut rng = StdRng::seed_from_u64(seed);
            let choices = lo.generate_choices(&mut rng);
            assert_eq!(choices.len(), 3, "a level-up always offers three cards");
            for c in &choices {
                assert!(
                    matches!(c, Upgrade::Passive(_)),
                    "no weapon card can be live once every weapon is maxed"
                );
                assert!(
                    lo.describe(c).0.ends_with("(capped)"),
                    "the only remaining cards are bandages"
                );
            }
        }
        // A bandage is still worth picking: it heals, it is not a dead stack.
        let mut lo = lo;
        lo.stats.max_hp = 200.0;
        let heal = lo.apply(&Upgrade::Passive(PassiveKind::Armor));
        assert!(heal > 0.0, "a capped pick must heal");
        assert_eq!(lo.stacks(PassiveKind::Armor), PassiveKind::Armor.max_stacks());
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

    /// The two player-triggered weapons come out of the normal 1-of-3 pool and
    /// their cards say how to fire them ("ACTIVE: press SPACE").
    #[test]
    fn active_weapons_are_offered_and_their_cards_teach_the_space_key() {
        let lo = Loadout::starting();
        for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
            assert!(kind.is_active());
            let (title, desc, flavor) = lo.describe(&Upgrade::NewWeapon(kind));
            assert!(title.contains(kind.name()), "card title {title:?}");
            assert!(
                desc.contains("ACTIVE: press SPACE"),
                "{kind:?}'s card must teach the trigger: {desc:?}"
            );
            assert!(!flavor.is_empty());
        }
        // The pool really offers them (not just `describe`).
        let mut offered = [false; 2];
        for seed in 0..400 {
            let mut rng = StdRng::seed_from_u64(seed);
            for c in lo.generate_choices(&mut rng) {
                if let Upgrade::NewWeapon(kind) = c {
                    for (i, k) in [WeaponKind::TailThrash, WeaponKind::SonicSqueak]
                        .into_iter()
                        .enumerate()
                    {
                        if k == kind {
                            offered[i] = true;
                        }
                    }
                }
            }
        }
        assert!(
            offered == [true, true],
            "an active weapon was never offered: {offered:?}"
        );
        // And they level like every other weapon.
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
        let idx = lo.weapons.iter().position(|w| w.kind == WeaponKind::TailThrash).unwrap();
        lo.apply(&Upgrade::LevelWeapon(idx));
        assert_eq!(lo.weapons[idx].level, 2);
        assert!(lo.weapons[idx].active_reach() > 60.0);
    }

    #[test]
    fn leveling_weapon_respects_max() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        for _ in 0..20 {
            lo.apply(&Upgrade::LevelWeapon(0));
        }
        assert_eq!(lo.weapons[0].level, WeaponKind::Gnaw.max_level());
    }

    #[test]
    fn passive_picks_are_recorded_as_stacks_in_pick_order() {
        let mut lo = Loadout::starting();
        assert_eq!(lo.stacks(PassiveKind::Damage), 0);
        lo.apply(&Upgrade::Passive(PassiveKind::Armor));
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        lo.apply(&Upgrade::Passive(PassiveKind::Armor));
        assert_eq!(lo.stacks(PassiveKind::Armor), 2);
        assert_eq!(lo.stacks(PassiveKind::Damage), 1);
        // First-acquisition order, not kind order: Armor was picked first.
        assert_eq!(
            lo.passives,
            vec![(PassiveKind::Armor, 2), (PassiveKind::Damage, 1)]
        );
        assert_eq!(lo.stats.armor, 2.0);
        assert_eq!(lo.stats.damage_mult, 1.15);
    }

    /// A loadout rebuilt from its own parts must reproduce its stats exactly:
    /// the campaign carries `(weapons, passives)` between levels, and this is
    /// the property that makes the carry-over lossless.
    #[test]
    fn from_parts_replays_the_same_stats() {
        let mut lo = Loadout::new(WeaponKind::CheeseSpray);
        lo.apply(&Upgrade::NewWeapon(WeaponKind::AcidSpit));
        lo.apply(&Upgrade::LevelWeapon(0));
        for p in [
            PassiveKind::MaxHp,
            PassiveKind::MoveSpeed,
            PassiveKind::MoveSpeed,
            PassiveKind::Magnet,
            PassiveKind::Regen,
            PassiveKind::Damage,
        ] {
            lo.apply(&Upgrade::Passive(p));
        }
        let copy = Loadout::from_parts(lo.weapons.clone(), lo.passives.clone());
        assert_eq!(copy.weapons.len(), lo.weapons.len());
        assert_eq!(copy.weapons[0].level, lo.weapons[0].level);
        assert_eq!(copy.passives, lo.passives);
        assert_eq!(copy.stats.max_hp, lo.stats.max_hp);
        assert_eq!(copy.stats.move_speed, lo.stats.move_speed);
        assert_eq!(copy.stats.damage_mult, lo.stats.damage_mult);
        assert_eq!(copy.stats.fire_rate_mult, lo.stats.fire_rate_mult);
        assert_eq!(copy.stats.magnet, lo.stats.magnet);
        assert_eq!(copy.stats.regen, lo.stats.regen);
        assert_eq!(copy.stats.armor, lo.stats.armor);
    }
}
