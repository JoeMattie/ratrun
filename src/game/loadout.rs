//! Player loadout: owned weapons, passive stats, and the level-up
//! upgrade-choice generation / application.

use rand::seq::SliceRandom;
use rand::{Rng, RngExt};

use super::weapon::{Weapon, WeaponKind};

pub const MAX_WEAPONS: usize = 5;
pub const MAX_PASSIVES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChestReward {
    WeaponLevel { kind: WeaponKind, level: u8 },
    Gold(u32),
}

pub fn open_chest(loadout: &mut Loadout, rng: &mut impl Rng) -> ChestReward {
    let eligible = loadout.weapons.iter().filter(|w| !w.is_max()).count();
    let pick = match eligible {
        0 => return ChestReward::Gold(10),
        1 => 0,
        n => rng.random_range(0..n),
    };
    let weapon = loadout.weapons.iter_mut().filter(|w| !w.is_max()).nth(pick).unwrap();
    weapon.level += 1;
    ChestReward::WeaponLevel { kind: weapon.kind, level: weapon.level }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PassiveKind {
    MaxHp,
    MoveSpeed,
    Damage,
    FireRate,
    Magnet,
    Regen,
    Armor,
    // ---- wave 3 (design §5) ----
    SpringHeel,
    Counterweight,
    LongFuse,
    SecondScent,
    LensCap,
}

impl PassiveKind {
    pub fn all() -> [PassiveKind; 12] {
        [
            PassiveKind::MaxHp,
            PassiveKind::MoveSpeed,
            PassiveKind::Damage,
            PassiveKind::FireRate,
            PassiveKind::Magnet,
            PassiveKind::Regen,
            PassiveKind::Armor,
            PassiveKind::SpringHeel,
            PassiveKind::Counterweight,
            PassiveKind::LongFuse,
            PassiveKind::SecondScent,
            PassiveKind::LensCap,
        ]
    }

    /// Can this passive's partner weapon evolve (design §3)? The wave-0
    /// evolution table is filled by `w3-evolutions`; the wave-3 passives are
    /// all evolution partners, the original seven are not.
    pub fn is_wave3(self) -> bool {
        matches!(
            self,
            PassiveKind::SpringHeel
                | PassiveKind::Counterweight
                | PassiveKind::LongFuse
                | PassiveKind::SecondScent
                | PassiveKind::LensCap
        )
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
            PassiveKind::SpringHeel => "Spring Heel",
            PassiveKind::Counterweight => "Counterweight",
            PassiveKind::LongFuse => "Long Fuse",
            PassiveKind::SecondScent => "Second Scent",
            PassiveKind::LensCap => "Lens Cap",
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
            PassiveKind::SpringHeel => "-0.18s base dash cooldown.",
            PassiveKind::Counterweight => "+15% weapon shove, +2px melee shove.",
            PassiveKind::LongFuse => "+12% zone life, +4% splash radius.",
            PassiveKind::SecondScent => "+10% gem XP (fractions banked).",
            PassiveKind::LensCap => "+8% ranged reach, +4% melee reach.",
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
            // Wave 3 (design §5). The dash cooldown cap takes the base 2.0 s
            // down to 1.1 s; Counterweight tops out at +60% shove / +8 px;
            // Long Fuse at +48% zone life / +16% splash; Second Scent at
            // +40% XP; Lens Cap at +32% ranged / +16% melee reach.
            PassiveKind::SpringHeel => 5,
            PassiveKind::Counterweight => 4,
            PassiveKind::LongFuse => 4,
            PassiveKind::SecondScent => 4,
            PassiveKind::LensCap => 4,
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
            PassiveKind::SpringHeel => "Smuggled out one spring at a time.",
            PassiveKind::Counterweight => "A washer tied to the tip.",
            PassiveKind::LongFuse => "Let it simmer.",
            PassiveKind::SecondScent => "There's a second crumb under the first.",
            PassiveKind::LensCap => "Cut from the bottom of a bottle.",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub max_hp: f32,
    pub move_speed: f32,
    pub damage_mult: f32,
    pub fire_rate_mult: f32,
    pub magnet: f32,
    pub regen: f32,
    pub armor: f32,
    pub dash_cd: f32,

    // ---- wave 3 passive hooks --------------------------------------------
    //
    // These are the *named hooks* other weapons read (they are the only thing
    // a wave-3 passive changes), so a weapon added later can join in with one
    // call instead of knowing which passive bought it:
    //
    // - `reach_mult` / `melee_reach_mult`: Lens Cap (ranged vs melee/ring).
    // - `zone_life_mult`: Long Fuse's lingering zones and orbiters.
    // - `splash_mult`: Long Fuse's splash / radial radii.
    // - `shove_mult` / `shove_px`: Counterweight's native shove multiplier and
    //   its extra nudge on melee hits that have no shove of their own.
    // - `xp_mult`: Second Scent, read by the gem pickup pass.
    /// Lens Cap: multiplier for a ranged weapon's reach (projectile life).
    pub reach_mult: f32,
    /// Lens Cap: multiplier for a melee/ring weapon's radius.
    pub melee_reach_mult: f32,
    /// Long Fuse: multiplier for a friendly lingering zone's / orbiter's life.
    pub zone_life_mult: f32,
    /// Long Fuse: multiplier for a splash / radial radius.
    pub splash_mult: f32,
    /// Counterweight: multiplier for a weapon's native shove (knockback).
    pub shove_mult: f32,
    /// Counterweight: extra px of outward shove on an ordinary melee hit that
    /// has no native knockback (once per target per 0.4 s).
    pub shove_px: f32,
    /// Second Scent: multiplier for collected gem XP (fractions are banked).
    pub xp_mult: f32,
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
            reach_mult: 1.0,
            melee_reach_mult: 1.0,
            zone_life_mult: 1.0,
            splash_mult: 1.0,
            shove_mult: 1.0,
            shove_px: 0.0,
            xp_mult: 1.0,
        }
    }
}

/// The passive hooks a weapon reads, applied as tiny helpers so a use site is
/// `stats.ranged_reach(base)` and never re-derives the arithmetic.
///
/// Every one of these is `base x multiplier` with a 1.0 default, so a run with
/// no wave-3 passive computes exactly the number it did before (no rounding
/// drift, no extra rng).
impl Stats {
    /// Lens Cap: a ranged weapon's reach (projectile life / lob distance).
    #[inline]
    pub fn ranged_reach(&self, base: f32) -> f32 {
        base * self.reach_mult
    }
    /// Lens Cap: a melee or ring weapon's radius.
    #[inline]
    pub fn melee_reach(&self, base: f32) -> f32 {
        base * self.melee_reach_mult
    }
    /// Long Fuse: a friendly lingering zone's or orbiter's lifetime.
    #[inline]
    pub fn zone_life(&self, base: f32) -> f32 {
        base * self.zone_life_mult
    }
    /// Long Fuse: a splash or radial effect's radius.
    #[inline]
    pub fn splash_radius(&self, base: f32) -> f32 {
        base * self.splash_mult
    }
    /// Counterweight: a weapon's native shove (knockback / ring push).
    #[inline]
    pub fn shove(&self, base: f32) -> f32 {
        base * self.shove_mult
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
        // ---- wave 3 (design §5): each stack is a flat step on a base value,
        // so the capped totals are 1.1 s dash (2.0 - 5 x 0.18), x1.6 shove /
        // +8 px (4 x 0.15 / 4 x 2), x1.48 zone life / x1.16 splash (4 x 0.12 /
        // 4 x 0.04), x1.4 XP (4 x 0.10) and x1.32 ranged / x1.16 melee reach
        // (4 x 0.08 / 4 x 0.04).
        PassiveKind::SpringHeel => {
            // The design's floor is the shop's 0.7 s; this guard only exists so
            // a future source of dash-cooldown reduction can never reach zero.
            stats.dash_cd = (stats.dash_cd - 0.18).max(0.35);
            0.0
        }
        PassiveKind::Counterweight => {
            stats.shove_mult += 0.15;
            stats.shove_px += 2.0;
            0.0
        }
        PassiveKind::LongFuse => {
            stats.zone_life_mult += 0.12;
            stats.splash_mult += 0.04;
            0.0
        }
        PassiveKind::SecondScent => {
            stats.xp_mult += 0.10;
            0.0
        }
        PassiveKind::LensCap => {
            stats.reach_mult += 0.08;
            stats.melee_reach_mult += 0.04;
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
        let mut kept_weapons: Vec<Weapon> = Vec::with_capacity(weapons.len().min(MAX_WEAPONS));
        for weapon in weapons {
            if kept_weapons.len() == MAX_WEAPONS {
                break;
            }
            if !kept_weapons.iter().any(|w| w.kind == weapon.kind) {
                kept_weapons.push(weapon);
            }
        }

        let mut kept_passives: Vec<(PassiveKind, u32)> =
            Vec::with_capacity(passives.len().min(MAX_PASSIVES));
        for (kind, stacks) in passives.into_iter().filter(|&(_, stacks)| stacks > 0) {
            if let Some((_, count)) = kept_passives.iter_mut().find(|(p, _)| *p == kind) {
                *count = count.saturating_add(stacks).min(kind.max_stacks());
            } else if kept_passives.len() < MAX_PASSIVES {
                kept_passives.push((kind, stacks.min(kind.max_stacks())));
            }
        }

        let mut stats = Stats::default();
        for &(kind, stacks) in &kept_passives {
            for _ in 0..stacks {
                apply_passive_stack(&mut stats, kind);
            }
        }
        Loadout {
            weapons: kept_weapons,
            passives: kept_passives,
            stats,
        }
    }

    #[allow(dead_code)] // Used by the upcoming evolution chest reward.
    pub fn evolvable(&self) -> Option<usize> {
        self.weapons.iter().position(|weapon| self.can_evolve(weapon))
    }

    #[allow(dead_code)] // Used by the upcoming evolution chest reward.
    pub fn evolve(&mut self, idx: usize) {
        if self.weapons.get(idx).is_some_and(|w| self.can_evolve(w)) {
            self.weapons[idx].evolved = true;
        }
    }

    fn can_evolve(&self, weapon: &Weapon) -> bool {
        !weapon.evolved && weapon.is_max()
            && weapon.kind.evolution().is_some_and(|e| self.stacks(e.passive) > 0)
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
        // Offer new weapons while a slot is open.
        if self.weapons.len() < MAX_WEAPONS {
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
            let stacks = self.stacks(p);
            if stacks == 0 && self.passives.len() >= MAX_PASSIVES {
                continue;
            }
            if stacks >= p.max_stacks() {
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
                if self.weapons.len() < MAX_WEAPONS && !self.has(*k) {
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
                if self.passives.len() >= MAX_PASSIVES && self.stacks(*p) == 0 {
                    return 0.0;
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
    fn wave3_rebuild_and_choices_respect_both_caps() {
        let mut weapons: Vec<_> = WeaponKind::all().into_iter().map(Weapon::new).collect();
        weapons.insert(1, weapons[0].clone());
        let mut passives = vec![(PassiveKind::Armor, 0), (PassiveKind::MaxHp, 1)];
        passives.extend(PassiveKind::all().into_iter().map(|kind| (kind, 1)));
        let mut lo = Loadout::from_parts(weapons, passives);
        assert_eq!(lo.weapons.len(), MAX_WEAPONS);
        assert_eq!(lo.weapons[1].kind, WeaponKind::all()[1]);
        assert_eq!(lo.passives.len(), MAX_PASSIVES);
        assert_eq!(lo.stacks(PassiveKind::MaxHp), 2);
        assert_eq!(lo.stats.max_hp, Stats::default().max_hp + 40.0);
        assert_eq!(lo.stacks(PassiveKind::Armor), 0);

        let unavailable_weapon = WeaponKind::all().into_iter().find(|&k| !lo.has(k)).unwrap();
        lo.apply(&Upgrade::NewWeapon(unavailable_weapon));
        lo.apply(&Upgrade::Passive(PassiveKind::Armor));
        assert!(!lo.has(unavailable_weapon));
        assert_eq!(lo.stacks(PassiveKind::Armor), 0);

        let mut rng = StdRng::seed_from_u64(15);
        for _ in 0..64 {
            for choice in lo.generate_choices(&mut rng) {
                match choice {
                    Upgrade::NewWeapon(_) => panic!("a full rack has no new weapon slot"),
                    Upgrade::LevelWeapon(i) => assert!(i < MAX_WEAPONS),
                    Upgrade::Passive(kind) => assert!(lo.stacks(kind) > 0),
                }
            }
        }
        lo.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        lo.apply(&Upgrade::LevelWeapon(0));
        assert_eq!(lo.stacks(PassiveKind::MaxHp), 3);
        assert_eq!(lo.weapons[0].level, 2);
    }

    #[test]
    fn wave3_chest_levels_only_an_eligible_owned_weapon() {
        let mut rng = StdRng::seed_from_u64(18);
        let mut lo = exhausted_loadout();
        lo.weapons[1].level -= 1;
        assert_eq!(
            open_chest(&mut lo, &mut rng),
            ChestReward::WeaponLevel { kind: lo.weapons[1].kind, level: lo.weapons[1].level }
        );
        assert!(lo.weapons.iter().all(Weapon::is_max));
        let mut untouched = StdRng::seed_from_u64(18);
        assert_eq!(open_chest(&mut lo, &mut rng), ChestReward::Gold(10));
        assert_eq!(rng.random::<u64>(), untouched.random::<u64>());
    }

    #[test]
    fn wave3_evolution_requires_max_level_and_a_matching_passive() {
        let mut lo = Loadout::starting();
        lo.evolve(0);
        assert!(!lo.weapons[0].evolved);
        lo.weapons[0].level = lo.weapons[0].kind.max_level();
        assert_eq!(lo.evolvable(), None);
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        assert_eq!(lo.evolvable(), Some(0));
        lo.evolve(usize::MAX);
        lo.evolve(0);
        assert!(lo.weapons[0].evolved);
        assert_eq!(lo.evolvable(), None);
    }

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
        let heal = lo.apply(&Upgrade::Passive(PassiveKind::Magnet));
        assert!(heal > 0.0, "a capped pick must heal");
        assert_eq!(lo.stacks(PassiveKind::Magnet), PassiveKind::Magnet.max_stacks());
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

    /// Wave 3's five passives: every stack is a visible step, every cap holds,
    /// and the capped total is the number the design quotes.
    #[test]
    fn wave3_passives_step_and_cap() {
        // Caps from the design's table (§5).
        let caps = [
            (PassiveKind::SpringHeel, 5u32),
            (PassiveKind::Counterweight, 4),
            (PassiveKind::LongFuse, 4),
            (PassiveKind::SecondScent, 4),
            (PassiveKind::LensCap, 4),
        ];
        for (kind, cap) in caps {
            assert_eq!(kind.max_stacks(), cap, "{kind:?} cap");
            assert!(kind.is_wave3(), "{kind:?} must be marked wave 3");
            assert!(!kind.name().is_empty() && !kind.desc().is_empty());
            assert!(!kind.flavor().is_empty());
        }
        assert_eq!(PassiveKind::all().len(), 12, "seven originals + five new");
        for p in PassiveKind::all() {
            assert_eq!(p.is_wave3(), !matches!(
                p,
                PassiveKind::MaxHp
                    | PassiveKind::MoveSpeed
                    | PassiveKind::Damage
                    | PassiveKind::FireRate
                    | PassiveKind::Magnet
                    | PassiveKind::Regen
                    | PassiveKind::Armor
            ));
        }

        // Spring Heel: -0.18 s of base dash cooldown per stack, capped at 1.1 s.
        let mut lo = Loadout::starting();
        let mut prev = lo.stats.dash_cd;
        for stack in 1..=PassiveKind::SpringHeel.max_stacks() {
            lo.apply(&Upgrade::Passive(PassiveKind::SpringHeel));
            let expect = 2.0 - 0.18 * stack as f32;
            assert!(
                (lo.stats.dash_cd - expect).abs() < 1e-6,
                "Spring Heel x{stack}: {} != {expect}",
                lo.stats.dash_cd
            );
            assert!(lo.stats.dash_cd < prev, "stack {stack} must be faster");
            prev = lo.stats.dash_cd;
        }
        assert!((lo.stats.dash_cd - 1.1).abs() < 1e-6, "capped at 2.0 - 0.9");
        // One more stack is a bandage, not a stack: the cap is the cap.
        let heal = lo.apply(&Upgrade::Passive(PassiveKind::SpringHeel));
        assert!(heal > 0.0, "a capped pick must heal instead");
        assert_eq!(lo.stacks(PassiveKind::SpringHeel), 5);
        assert!((lo.stats.dash_cd - 1.1).abs() < 1e-6);

        // Counterweight: +15% shove and +2 px melee nudge per stack.
        let mut lo = Loadout::starting();
        for stack in 1..=PassiveKind::Counterweight.max_stacks() {
            lo.apply(&Upgrade::Passive(PassiveKind::Counterweight));
            assert!((lo.stats.shove_mult - (1.0 + 0.15 * stack as f32)).abs() < 1e-6);
            assert!((lo.stats.shove_px - 2.0 * stack as f32).abs() < 1e-6);
        }
        assert!((lo.stats.shove_mult - 1.6).abs() < 1e-6, "capped at x1.6");
        assert!((lo.stats.shove_px - 8.0).abs() < 1e-6, "capped at +8 px");
        assert!((lo.stats.shove(100.0) - 160.0).abs() < 1e-4);

        // Long Fuse: +12% zone life and +4% splash radius per stack.
        let mut lo = Loadout::starting();
        for stack in 1..=PassiveKind::LongFuse.max_stacks() {
            lo.apply(&Upgrade::Passive(PassiveKind::LongFuse));
            assert!((lo.stats.zone_life_mult - (1.0 + 0.12 * stack as f32)).abs() < 1e-6);
            assert!((lo.stats.splash_mult - (1.0 + 0.04 * stack as f32)).abs() < 1e-6);
        }
        assert!((lo.stats.zone_life(2.0) - 2.96).abs() < 1e-5, "capped +48%");
        assert!((lo.stats.splash_radius(10.0) - 11.6).abs() < 1e-5, "capped +16%");

        // Second Scent: +10% gem XP per stack, capped at +40%.
        let mut lo = Loadout::starting();
        for stack in 1..=PassiveKind::SecondScent.max_stacks() {
            lo.apply(&Upgrade::Passive(PassiveKind::SecondScent));
            assert!((lo.stats.xp_mult - (1.0 + 0.10 * stack as f32)).abs() < 1e-6);
        }
        assert!((lo.stats.xp_mult - 1.4).abs() < 1e-5, "capped at +40%");

        // Lens Cap: +8% ranged and +4% melee reach per stack.
        let mut lo = Loadout::starting();
        for stack in 1..=PassiveKind::LensCap.max_stacks() {
            lo.apply(&Upgrade::Passive(PassiveKind::LensCap));
            assert!((lo.stats.reach_mult - (1.0 + 0.08 * stack as f32)).abs() < 1e-6);
            assert!((lo.stats.melee_reach_mult - (1.0 + 0.04 * stack as f32)).abs() < 1e-6);
        }
        assert!((lo.stats.ranged_reach(100.0) - 132.0).abs() < 1e-4, "+32% ranged");
        assert!((lo.stats.melee_reach(100.0) - 116.0).abs() < 1e-4, "+16% melee");

        // The hooks are identity without a wave-3 passive: a run that never
        // picks one computes the same numbers it did before.
        let plain = Stats::default();
        for base in [0.0f32, 1.0, 26.0, 154.0] {
            assert_eq!(plain.ranged_reach(base), base);
            assert_eq!(plain.melee_reach(base), base);
            assert_eq!(plain.zone_life(base), base);
            assert_eq!(plain.splash_radius(base), base);
            assert_eq!(plain.shove(base), base);
        }
        assert_eq!(plain.shove_px, 0.0);
        assert_eq!(plain.xp_mult, 1.0);
    }

    /// Campaign carry-over: `from_parts` replays every wave-3 stack onto a
    /// fresh `Stats`, so a kept Spring Heel still dashes at 1.1 s next floor.
    #[test]
    fn from_parts_replays_wave3_passives() {
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::NewWeapon(WeaponKind::LaserEyes));
        lo.apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
        for p in [
            PassiveKind::SpringHeel,
            PassiveKind::SpringHeel,
            PassiveKind::Counterweight,
            PassiveKind::LongFuse,
            PassiveKind::LongFuse,
            PassiveKind::SecondScent,
            PassiveKind::LensCap,
            PassiveKind::LensCap,
        ] {
            lo.apply(&Upgrade::Passive(p));
        }
        let copy = Loadout::from_parts(lo.weapons.clone(), lo.passives.clone());
        assert_eq!(copy.passives, lo.passives);
        assert_eq!(copy.weapons.len(), lo.weapons.len());
        assert_eq!(copy.stats.dash_cd, lo.stats.dash_cd);
        assert_eq!(copy.stats.shove_mult, lo.stats.shove_mult);
        assert_eq!(copy.stats.shove_px, lo.stats.shove_px);
        assert_eq!(copy.stats.zone_life_mult, lo.stats.zone_life_mult);
        assert_eq!(copy.stats.splash_mult, lo.stats.splash_mult);
        assert_eq!(copy.stats.reach_mult, lo.stats.reach_mult);
        assert_eq!(copy.stats.melee_reach_mult, lo.stats.melee_reach_mult);
        assert_eq!(copy.stats.xp_mult, lo.stats.xp_mult);
        // …and the carry cannot buy past a cap, even if the record says so.
        let over = Loadout::from_parts(
            lo.weapons.clone(),
            vec![(PassiveKind::SpringHeel, 99), (PassiveKind::LensCap, 99)],
        );
        assert!((over.stats.dash_cd - 1.1).abs() < 1e-6);
        assert!((over.stats.reach_mult - 1.32).abs() < 1e-5);
    }

    /// Both wave-3 weapons reach the level-up pool like every other kind, and
    /// their cards say what they do and how they are triggered.
    #[test]
    fn wave3_weapons_are_offered_on_cards() {
        let lo = Loadout::starting();
        for kind in [WeaponKind::LaserEyes, WeaponKind::SuperDash] {
            let (title, desc, flavor) = lo.describe(&Upgrade::NewWeapon(kind));
            assert!(title.contains(kind.name()), "card title {title:?}");
            assert!(!desc.is_empty() && !flavor.is_empty());
            assert!(!kind.is_active(), "{kind:?} has no charge gauge");
        }
        // The dash weapon teaches the dash trigger, not Space-the-charge.
        assert!(WeaponKind::SuperDash.desc().contains("DASH"));
        let mut offered = [false; 2];
        for seed in 0..800 {
            let mut rng = StdRng::seed_from_u64(seed);
            for c in lo.generate_choices(&mut rng) {
                if let Upgrade::NewWeapon(kind) = c {
                    for (i, k) in [WeaponKind::LaserEyes, WeaponKind::SuperDash]
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
        assert_eq!(offered, [true, true], "a wave-3 weapon was never offered");
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
