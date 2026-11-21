//! Player loadout: owned weapons, passive stats, and the level-up
//! upgrade-choice generation / application.

use rand::seq::SliceRandom;
use rand::Rng;

use super::weapon::{Evolution, Weapon, WeaponKind};

pub const MAX_WEAPONS: usize = 5;
pub const MAX_PASSIVES: usize = 5;

/// Gold a `Purse` level-up card hands over (design §2).
pub const PURSE_GOLD: u32 = 8;
/// Gold a chest hands over when there is nothing left to upgrade (design §7).
pub const CHEST_GOLD: u32 = 24;
/// Fraction of max HP a chest also heals in that case (the caller applies it:
/// the loadout cannot see the current HP).
pub const CHEST_HEAL_FRAC: f32 = 0.25;
/// Weapon level from which the level-up pool starts pushing the evolution
/// partner at the player (`"an owned L4/L5 weapon"`, design §2).
pub const PARTNER_PRIORITY_LEVEL: u8 = 4;

/// TEMPORARY: `Counterweight` — Flywheel's partner in the design table — lands
/// on the `w3-weapons-b` branch, which is not in this base. Until it merges,
/// Flywheel is keyed to `Armor` (Thick Hide, the passive `Counterweight` is
/// appended next to). **This is the one constant to flip.** Nothing about
/// Flywheel's *behaviour* reads its partner: the passive only gates the chest,
/// so flipping this finishes the row.
pub const FLYWHEEL_PARTNER: PassiveKind = PassiveKind::Armor;

/// What one opened chest did. The loadout has already been changed when
/// [`open_chest`] returns; the reward is the card the App shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChestReward {
    WeaponLevel { kind: WeaponKind, level: u8 },
    /// The earliest eligible weapon in rack order evolved. Carries everything
    /// the chest card needs: the slot, the weapon (for its icon), the table row
    /// (chest name, inscription, partner, new attack) and how many stacks of
    /// the partner the build owns.
    Evolve {
        slot: usize,
        kind: WeaponKind,
        evo: Evolution,
        partner_stacks: u32,
    },
    /// The free rank went to a passive instead (no unfinished weapon): the
    /// card shows the passive and its new stack count.
    PassiveLevel { kind: PassiveKind, stacks: u32 },
    /// Nothing left to upgrade: gold (and the caller heals
    /// [`CHEST_HEAL_FRAC`] of max HP).
    Gold(u32),
}

/// Open one chest (design §7). Fully determined: no rng draw, so an unevolved
/// run never moves the rng stream by visiting a chest.
///
/// 1. Evolve the **earliest** eligible weapon in rack order (L5 + at least one
///    stack of its partner, not already evolved).
/// 2. Else grant one free rank: the highest-rank unfinished weapon, then the
///    highest-stack unfinished passive; ties in rack order.
/// 3. Else [`CHEST_GOLD`] gold and a heal the caller applies.
///
/// `_rng` is kept in the signature the wave-0 contract fixed; the table above
/// needs no draw.
pub fn open_chest(loadout: &mut Loadout, _rng: &mut impl Rng) -> ChestReward {
    if let Some(slot) = loadout.evolvable() {
        loadout.evolve(slot);
        let weapon = &loadout.weapons[slot];
        let evo = weapon
            .kind
            .evolution()
            .expect("an evolvable weapon has an evolution row");
        return ChestReward::Evolve {
            slot,
            kind: weapon.kind,
            evo,
            partner_stacks: loadout.stacks(evo.passive),
        };
    }
    if let Some(slot) = loadout.unfinished_weapon() {
        let weapon = &mut loadout.weapons[slot];
        weapon.level += 1;
        return ChestReward::WeaponLevel { kind: weapon.kind, level: weapon.level };
    }
    if let Some(kind) = loadout.unfinished_passive() {
        loadout.bump_stack(kind);
        let _ = apply_passive_stack(&mut loadout.stats, kind);
        return ChestReward::PassiveLevel { kind, stacks: loadout.stacks(kind) };
    }
    ChestReward::Gold(CHEST_GOLD)
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

#[derive(Clone, Debug)]
pub enum Upgrade {
    NewWeapon(WeaponKind),
    LevelWeapon(usize),
    Passive(PassiveKind),
    /// Fallback card, at most one per level-up: restores a quarter of max HP.
    /// Leaves every rack slot untouched (design §2).
    Bandage,
    /// Fallback card, at most one per level-up: [`PURSE_GOLD`] gold. Leaves
    /// every rack slot untouched.
    Purse,
}

/// Fraction of max HP a bandage card (and a capped passive played as one)
/// restores.
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

    /// Index of the earliest weapon in rack order that a chest could evolve
    /// (L5, has a table row, partner passive owned, not already evolved).
    pub fn evolvable(&self) -> Option<usize> {
        self.weapons.iter().position(|weapon| self.can_evolve(weapon))
    }

    /// Evolve the weapon at `idx` if it is eligible. The evolved flag locks: a
    /// second call for the same slot does nothing.
    pub fn evolve(&mut self, idx: usize) {
        if self.weapons.get(idx).is_some_and(|w| self.can_evolve(w)) {
            self.weapons[idx].evolved = true;
        }
    }

    fn can_evolve(&self, weapon: &Weapon) -> bool {
        !weapon.evolved && weapon.is_max()
            && weapon.kind.evolution().is_some_and(|e| self.stacks(e.passive) > 0)
    }

    /// Highest-rank unfinished weapon, ties by rack order (chest rule 2).
    fn unfinished_weapon(&self) -> Option<usize> {
        let mut best: Option<(usize, u8)> = None;
        for (i, w) in self.weapons.iter().enumerate() {
            if w.is_max() {
                continue;
            }
            if best.is_none_or(|(_, level)| w.level > level) {
                best = Some((i, w.level));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Highest-stack unfinished owned passive, ties by (pick) order.
    fn unfinished_passive(&self) -> Option<PassiveKind> {
        let mut best: Option<(PassiveKind, u32)> = None;
        for &(kind, stacks) in &self.passives {
            if stacks >= kind.max_stacks() {
                continue;
            }
            if best.is_none_or(|(_, held)| stacks > held) {
                best = Some((kind, stacks));
            }
        }
        best.map(|(kind, _)| kind)
    }

    /// The unowned evolution partner the level-up pool should push first: the
    /// partner of the earliest owned L4/L5 weapon that is not evolved yet, when
    /// a passive slot is free (design §2). `None` before L4, when the partner is
    /// already owned, or when the passive rack is full.
    pub fn priority_partner(&self) -> Option<PassiveKind> {
        if self.passives.len() >= MAX_PASSIVES {
            return None;
        }
        for w in &self.weapons {
            if w.evolved || w.level < PARTNER_PRIORITY_LEVEL {
                continue;
            }
            if let Some(evo) = w.kind.evolution() {
                if self.stacks(evo.passive) == 0 {
                    return Some(evo.passive);
                }
            }
        }
        None
    }

    /// `"  · Milk Teeth needs Sharp Teeth (owned ×2)."` — the partner line a
    /// weapon card carries, empty for a weapon with no row or one already
    /// evolved.
    fn partner_note(&self, weapon: &Weapon) -> String {
        let Some(evo) = weapon.kind.evolution() else {
            return String::new();
        };
        if weapon.evolved {
            return String::new();
        }
        let stacks = self.stacks(evo.passive);
        let owned = match stacks {
            0 => "unowned".to_string(),
            1 => "owned".to_string(),
            n => format!("owned ×{n}"),
        };
        if weapon.is_max() && stacks > 0 {
            format!("  · {} READY with {}.", evo.name, evo.passive.name())
        } else {
            format!("  · {} needs {} ({}).", evo.name, evo.passive.name(), owned)
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
            Upgrade::NewWeapon(k) => {
                // A brand-new weapon has no `Weapon` yet; borrow a level-1 one
                // for the partner line.
                let probe = Weapon::new(*k);
                (
                    format!("{} (NEW)", k.name()),
                    format!("{}{}", k.desc(), self.partner_note(&probe)),
                    k.flavor().to_string(),
                )
            }
            Upgrade::LevelWeapon(i) => {
                let Some(w) = self.weapons.get(*i) else {
                    return ("Weapon".to_string(), String::new(), String::new());
                };
                let next = (w.level + 1).min(w.kind.max_level());
                (
                    format!("{} Lv{}→{}", w.name(), w.level, next),
                    format!("{}{}", w.desc(), self.partner_note(w)),
                    w.flavor().to_string(),
                )
            }
            Upgrade::Bandage => (
                "Bandage".to_string(),
                format!("Restores {:.0}% of max HP.", BANDAGE_HEAL_FRAC * 100.0),
                "Clean enough. Mostly.".to_string(),
            ),
            Upgrade::Purse => (
                "Purse".to_string(),
                format!("+{PURSE_GOLD} gold."),
                "Found in the lining.".to_string(),
            ),
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

    /// Build up to three distinct level-up choices at **unknown health**.
    ///
    /// This is the entry point the harnesses and older tests use; it assumes a
    /// damaged rat (`hp_frac = 0.5`) so the bandage fallback still exists. The
    /// App calls [`Loadout::generate_choices_with_hp`] with the real fraction,
    /// which is what hides the bandage at full health (design §2).
    pub fn generate_choices(&self, rng: &mut impl Rng) -> Vec<Upgrade> {
        self.generate_choices_with_hp(rng, 0.5)
    }

    /// Build up to three distinct level-up choices (design §2).
    ///
    /// Slot 1 is an owned rank-up, slot 2 a new kind (an unowned evolution
    /// partner for an owned L4/L5 weapon first), slot 3 any other legal card.
    /// Competing candidates inside a slot are broken by the seeded draw. Live
    /// cards are exhausted before the fallbacks: at most one [`Upgrade::Bandage`]
    /// (hidden at full health) and one [`Upgrade::Purse`], so a full build at
    /// full health is offered just the purse — or, if even that is spent
    /// (a purse is always legal), nothing else. Fewer than three cards is a
    /// legal, deliberate answer.
    pub fn generate_choices_with_hp(&self, rng: &mut impl Rng, hp_frac: f32) -> Vec<Upgrade> {
        let mut choices: Vec<Upgrade> = Vec::with_capacity(3);

        // 1. One owned rank-up (a weapon level or a stack in an owned passive).
        let mut rank_ups = self.rank_up_cards();
        rank_ups.shuffle(rng);
        if !rank_ups.is_empty() {
            choices.push(rank_ups.pop().expect("checked non-empty"));
        }

        // 2. One new kind. The first level-up always includes a second weapon;
        //    after that the evolution partner of an owned L4/L5 weapon is
        //    pushed to the front of the queue.
        let mut new_kinds = self.new_kind_cards();
        let mut promoted: Option<Upgrade> = None;
        if self.weapons.len() < 2 {
            let mut fresh: Vec<WeaponKind> = new_kinds
                .iter()
                .filter_map(|c| match c {
                    Upgrade::NewWeapon(k) => Some(*k),
                    _ => None,
                })
                .collect();
            fresh.shuffle(rng);
            if let Some(kind) = fresh.first().copied() {
                if let Some(pos) = new_kinds.iter().position(|c| matches!(c, Upgrade::NewWeapon(k) if *k == kind))
                {
                    promoted = Some(new_kinds.remove(pos));
                }
            }
        }
        if promoted.is_none() {
            if let Some(partner) = self.priority_partner() {
                if let Some(pos) = new_kinds
                    .iter()
                    .position(|c| matches!(c, Upgrade::Passive(p) if *p == partner))
                {
                    promoted = Some(new_kinds.remove(pos));
                }
            }
        }
        let mut second = promoted;
        if second.is_none() {
            new_kinds.shuffle(rng);
            second = new_kinds.pop();
        }
        if let Some(card) = second {
            choices.push(card);
        }

        // 3. One other legal card: any live card not already dealt.
        let mut rest: Vec<Upgrade> = rank_ups;
        rest.extend(new_kinds);
        rest.shuffle(rng);
        if let Some(card) = rest.pop() {
            choices.push(card);
        }

        // Fallbacks fill only the spare places, in this order.
        if choices.len() < 3 && hp_frac < 1.0 {
            choices.push(Upgrade::Bandage);
        }
        if choices.len() < 3 {
            choices.push(Upgrade::Purse);
        }
        choices
    }

    /// Every live rank-up on an owned kind (weapons under their max, owned
    /// passives under their cap). Empty at a fully maxed build.
    fn rank_up_cards(&self) -> Vec<Upgrade> {
        let mut out = Vec::new();
        for (i, w) in self.weapons.iter().enumerate() {
            if !w.is_max() {
                out.push(Upgrade::LevelWeapon(i));
            }
        }
        for &(kind, stacks) in &self.passives {
            if stacks < kind.max_stacks() {
                out.push(Upgrade::Passive(kind));
            }
        }
        out
    }

    /// Every new kind the build may still open, respecting both caps.
    fn new_kind_cards(&self) -> Vec<Upgrade> {
        let mut out = Vec::new();
        if self.weapons.len() < MAX_WEAPONS {
            for kind in WeaponKind::all() {
                if !self.has(kind) {
                    out.push(Upgrade::NewWeapon(kind));
                }
            }
        }
        if self.passives.len() < MAX_PASSIVES {
            for kind in PassiveKind::all() {
                if self.stacks(kind) == 0 {
                    out.push(Upgrade::Passive(kind));
                }
            }
        }
        out
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
                    // played as a bandage instead of a dead stack.
                    return BANDAGE_HEAL_FRAC * self.stats.max_hp;
                }
                if self.passives.len() >= MAX_PASSIVES && self.stacks(*p) == 0 {
                    return 0.0;
                }
                self.bump_stack(*p);
                apply_passive_stack(&mut self.stats, *p)
            }
            // Consumables leave every rack slot untouched.
            Upgrade::Bandage => BANDAGE_HEAL_FRAC * self.stats.max_hp,
            // Gold is credited by the caller (`App::apply_levelup`), which owns
            // the wallet; the loadout has no gold field.
            Upgrade::Purse => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};

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
                    // The consumables leave both racks untouched.
                    Upgrade::Bandage | Upgrade::Purse => {}
                }
            }
        }
        lo.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        lo.apply(&Upgrade::LevelWeapon(0));
        assert_eq!(lo.stacks(PassiveKind::MaxHp), 3);
        assert_eq!(lo.weapons[0].level, 2);
    }

    #[test]
    fn wave3_chest_evolves_then_ranks_then_pays_gold() {
        // --- Case 1: every eligible weapon could evolve; rack order decides.
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
        lo.apply(&Upgrade::Passive(PassiveKind::Damage)); // Gnaw's partner
        lo.apply(&Upgrade::Passive(PassiveKind::MaxHp)); // Cheese Spray's
        for i in 0..lo.weapons.len() {
            while !lo.weapons[i].is_max() {
                lo.apply(&Upgrade::LevelWeapon(i));
            }
        }
        let mut rng = StdRng::seed_from_u64(18);
        let mut untouched = StdRng::seed_from_u64(18);
        match open_chest(&mut lo, &mut rng) {
            ChestReward::Evolve { slot, kind, evo, partner_stacks } => {
                assert_eq!(slot, 0, "the earliest rack slot evolves first");
                assert_eq!(kind, WeaponKind::Gnaw);
                assert_eq!(evo.name, "Milk Teeth");
                assert_eq!(evo.passive, PassiveKind::Damage);
                assert_eq!(partner_stacks, 1);
            }
            other => panic!("expected an evolution, got {other:?}"),
        }
        assert!(lo.weapons[0].evolved);
        assert!(!lo.weapons[1].evolved);
        // The second chest takes the next eligible weapon in rack order.
        assert!(matches!(
            open_chest(&mut lo, &mut rng),
            ChestReward::Evolve { slot: 1, kind: WeaponKind::CheeseSpray, .. }
        ));
        // Both cases drew no rng at all.
        assert_eq!(rng.random::<u64>(), untouched.random::<u64>());

        // --- Case 2: no evolution left, so one free rank: the highest-rank
        // unfinished weapon first (ties by rack order), then the passive.
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        lo.weapons[0].level = 3;
        match open_chest(&mut lo, &mut rng) {
            ChestReward::WeaponLevel { kind, level } => {
                assert_eq!(kind, WeaponKind::Gnaw);
                assert_eq!(level, 4);
            }
            other => panic!("expected a weapon rank, got {other:?}"),
        }
        // Gnaw L5 + Sharp Teeth would evolve, so the rank branch needs the
        // partner left behind (the campaign carry-over case) or no evolution at
        // all. Build the latter: an unpaired L5 weapon and no evolution rows.
        lo.passives.clear();
        lo.apply(&Upgrade::Passive(PassiveKind::Armor));
        lo.apply(&Upgrade::Passive(PassiveKind::Armor));
        lo.apply(&Upgrade::Passive(PassiveKind::Regen));
        lo.weapons[0].level = 3;
        assert!(lo.evolvable().is_none(), "Gnaw has no partner here: {lo:?}");
        assert!(matches!(
            open_chest(&mut lo, &mut rng),
            ChestReward::WeaponLevel { kind: WeaponKind::Gnaw, level: 4 }
        ));
        lo.weapons[0].level = lo.weapons[0].kind.max_level();
        match open_chest(&mut lo, &mut rng) {
            ChestReward::PassiveLevel { kind, stacks } => {
                // Thick Hide has the most stacks, so it takes the rank.
                assert_eq!(kind, PassiveKind::Armor);
                assert_eq!(stacks, 3);
            }
            other => panic!("expected a passive rank, got {other:?}"),
        }

        // --- Case 3: nothing left to rank: gold, and no rng draw either.
        let mut maxed = exhausted_loadout();
        // The exhausted loadout owns every partner, so every weapon could still
        // evolve; spend those chests first and then measure the fallback.
        while maxed.evolvable().is_some() {
            assert!(matches!(open_chest(&mut maxed, &mut rng), ChestReward::Evolve { .. }));
        }
        assert_eq!(open_chest(&mut maxed, &mut rng), ChestReward::Gold(CHEST_GOLD));
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
            assert_eq!(choices.len(), 3, "one live card + the two consumables");
            let live = choices
                .iter()
                .filter(|c| matches!(c, Upgrade::Passive(PassiveKind::Damage)))
                .count();
            assert_eq!(live, 1, "the one live card must always be offered");
            // …and the spare places are the two consumables, never a second
            // card of the same consumable.
            let mut bandages = 0;
            let mut purses = 0;
            for c in choices.iter().filter(|c| !matches!(c, Upgrade::Passive(PassiveKind::Damage))) {
                match c {
                    Upgrade::Bandage => bandages += 1,
                    Upgrade::Purse => purses += 1,
                    other => panic!("a capped passive was dealt: {other:?}"),
                }
            }
            assert_eq!((bandages, purses), (1, 1));
        }
    }

    /// A maxed build has no live cards: at most one bandage (never at full
    /// health) plus the purse, and just the purse when the rat is untouched.
    #[test]
    fn an_exhausted_pool_offers_consumables_not_dead_stacks() {
        let lo = exhausted_loadout();
        for seed in 0..50 {
            let mut rng = StdRng::seed_from_u64(seed);
            let choices = lo.generate_choices_with_hp(&mut rng, 0.4);
            assert_eq!(choices.len(), 2, "a bandage and a purse: {choices:?}");
            assert!(matches!(choices[0], Upgrade::Bandage));
            assert!(matches!(choices[1], Upgrade::Purse));
            let full = lo.generate_choices_with_hp(&mut rng, 1.0);
            assert_eq!(full.len(), 1, "at full health only the purse: {full:?}");
            assert!(matches!(full[0], Upgrade::Purse));
            // Neither consumes a rack slot.
            for c in choices.iter().chain(full.iter()) {
                assert!(matches!(c, Upgrade::Bandage | Upgrade::Purse));
            }
        }
        // A bandage is worth picking: it heals, it is not a dead stack.
        let mut lo = lo;
        lo.stats.max_hp = 200.0;
        let heal = lo.apply(&Upgrade::Bandage);
        assert!((heal - 50.0).abs() < 1e-3, "a quarter of max HP, got {heal}");
        assert_eq!(lo.weapons.len(), MAX_WEAPONS);
        assert_eq!(lo.passives.len(), MAX_PASSIVES);
        assert_eq!(lo.apply(&Upgrade::Purse), 0.0);
    }

    /// Design §2 in one test: one owned rank-up, one new kind, one other; the
    /// unowned partner of an owned L4 weapon takes the new-kind slot; caps are
    /// respected past the rack limits; the first level-up always offers a
    /// second weapon.
    #[test]
    fn offer_slots_follow_the_design_rules() {
        // A fresh build: rank-up (Gnaw) + a new weapon + a third legal card.
        let lo = Loadout::starting();
        for seed in 0..40 {
            let mut rng = StdRng::seed_from_u64(seed);
            let choices = lo.generate_choices_with_hp(&mut rng, 1.0);
            assert_eq!(choices.len(), 3);
            assert!(
                matches!(choices[0], Upgrade::LevelWeapon(0)),
                "slot 1 is the owned rank-up: {choices:?}"
            );
            assert!(
                matches!(choices[1], Upgrade::NewWeapon(_)),
                "the first level-up includes a second weapon: {choices:?}"
            );
        }

        // Gnaw at L4 with no Sharp Teeth: the partner takes the new-kind slot
        // (once a second weapon exists, so the first-level-up rule is spent).
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::NewWeapon(WeaponKind::TailWhip));
        lo.weapons[0].level = PARTNER_PRIORITY_LEVEL;
        for seed in 0..40 {
            let mut rng = StdRng::seed_from_u64(seed);
            let choices = lo.generate_choices_with_hp(&mut rng, 1.0);
            assert!(
                matches!(choices[1], Upgrade::Passive(PassiveKind::Damage)),
                "the evolution partner must be pushed: {choices:?}"
            );
            // It is offered once, never twice.
            assert_eq!(
                choices
                    .iter()
                    .filter(|c| matches!(c, Upgrade::Passive(PassiveKind::Damage)))
                    .count(),
                1
            );
        }
        // Below L4 nothing is prioritised: Sharp Teeth is just one of the new
        // kinds and the draw decides.
        let mut early = Loadout::starting();
        early.apply(&Upgrade::NewWeapon(WeaponKind::TailWhip));
        early.weapons[0].level = PARTNER_PRIORITY_LEVEL - 1;
        let mut saw_partner = false;
        for seed in 0..40 {
            let mut rng = StdRng::seed_from_u64(seed);
            for c in early.generate_choices_with_hp(&mut rng, 1.0) {
                if matches!(c, Upgrade::Passive(PassiveKind::Damage)) {
                    saw_partner = true;
                }
            }
        }
        assert!(saw_partner, "an unprioritised partner is still a legal card");

        // A full weapon rack never offers another weapon…
        let mut full = Loadout::starting();
        for k in WeaponKind::all().into_iter().take(MAX_WEAPONS) {
            full.apply(&Upgrade::NewWeapon(k));
        }
        // …and a full passive rack never offers a new passive.
        for p in PassiveKind::all() {
            full.apply(&Upgrade::Passive(p));
        }
        for seed in 0..40 {
            let mut rng = StdRng::seed_from_u64(seed);
            for c in full.generate_choices_with_hp(&mut rng, 1.0) {
                match c {
                    Upgrade::NewWeapon(k) => panic!("{k:?} offered past the weapon cap"),
                    Upgrade::Passive(p) => {
                        assert!(full.stacks(p) > 0, "{p:?} offered past the passive cap")
                    }
                    _ => {}
                }
            }
        }
    }

    /// Weapon cards name the evolution partner and mark whether it is owned.
    #[test]
    fn weapon_cards_name_the_partner_and_the_owned_mark() {
        let mut lo = Loadout::starting();
        lo.weapons[0].level = lo.weapons[0].kind.max_level();
        let (_, desc, _) = lo.describe(&Upgrade::LevelWeapon(0));
        assert!(desc.contains("Milk Teeth"), "{desc}");
        assert!(desc.contains("Sharp Teeth"), "{desc}");
        assert!(desc.contains("unowned"), "{desc}");
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        let (_, desc, _) = lo.describe(&Upgrade::LevelWeapon(0));
        assert!(desc.contains("READY"), "a maxed weapon with its partner is ready: {desc}");
        let (_, desc, _) = lo.describe(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
        assert!(desc.contains("Fondue Fork") && desc.contains("Fat Reserves"), "{desc}");
        // The evolved weapon's own card, once evolved, is the new attack.
        lo.evolve(0);
        let (title, desc, flavor) = lo.describe(&Upgrade::LevelWeapon(0));
        assert_eq!(title, "Milk Teeth Lv5→5");
        assert_eq!(desc, lo.weapons[0].kind.evolution().unwrap().attack);
        assert_eq!(flavor, "They always come back in.");
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
