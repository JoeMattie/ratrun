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

/// Flywheel's (evolved Tail Thrash) partner passive: Counterweight, per the
/// design table. Kept as a named constant because the evolutions branch was
/// built before Counterweight existed and keyed the row through it. Nothing
/// about Flywheel's *behaviour* reads its partner: the passive only gates the
/// chest.
pub const FLYWHEEL_PARTNER: PassiveKind = PassiveKind::Counterweight;

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
        if !loadout.splices.is_empty() {
            // As in `Loadout::apply`: the splice multipliers go after the
            // passives, so a free rank on a spliced rat replays them.
            loadout.rebuild_stats();
        }
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
            PassiveKind::MaxHp => "+20 max HP and heal 20.",
            PassiveKind::MoveSpeed => "+12% move speed.",
            PassiveKind::Damage => "+15% weapon damage.",
            PassiveKind::FireRate => "+12% fire rate.",
            PassiveKind::Magnet => "+30% pickup radius.",
            PassiveKind::Regen => "+0.5 HP/s regen.",
            PassiveKind::Armor => "+1 armor (-1 damage per hit).",
            PassiveKind::SpringHeel => "-0.18s dash cooldown.",
            PassiveKind::Counterweight => "+15% knockback, +2px melee shove.",
            PassiveKind::LongFuse => "+12% zone duration, +4% splash size.",
            PassiveKind::SecondScent => "+10% gem XP.",
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
/// Health a passive stack heals when it is granted, for the callers that grant
/// stacks outside the level-up card (a chest's free rank).
///
/// `apply_passive_stack` returns this as its result; the level-up path heals
/// the player with it, while the chest path used to drop it — a maxed rack plus
/// an eligible Fat Reserves rank gave +20 max HP and no health.
#[inline]
pub fn passive_stack_heal(p: PassiveKind) -> f32 {
    match p {
        PassiveKind::MaxHp => 20.0,
        _ => 0.0,
    }
}

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
    /// Owned **gene splices**, in purchase order (`game::shop`).
    ///
    /// Permanent for the run and on neither rack: they are not weapons and not
    /// passive stacks, so they never count against [`MAX_WEAPONS`] /
    /// [`MAX_PASSIVES`] and the keep-three screen cannot see them. They ride
    /// along with the loadout between campaign levels
    /// (`campaign::next_loadout`), which is what makes them last until the run
    /// ends. [`Loadout::rebuild_stats`] applies them **after** the passives.
    pub splices: Vec<super::shop::SpliceKind>,
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
            splices: Vec::new(),
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
            splices: Vec::new(),
        }
    }

    /// Rebuild [`Self::stats`] from scratch: every passive stack in acquisition
    /// order, then every owned gene splice on top (the design's "apply splice
    /// multipliers after passive stats"), with the shop's floors.
    ///
    /// This is the single place a splice (or a Molt) changes the numbers, so a
    /// carried loadout, a purchased splice and a level-up can never disagree.
    pub fn rebuild_stats(&mut self) {
        let mut stats = Stats::default();
        for &(kind, stacks) in &self.passives {
            for _ in 0..stacks {
                apply_passive_stack(&mut stats, kind);
            }
        }
        super::shop::apply_splices(&mut stats, &self.splices);
        self.stats = stats;
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
            format!(" · {} READY ({}).", evo.name, evo.passive.name())
        } else {
            format!(" · {}: {} ({}).", evo.name, evo.passive.name(), owned)
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
                            "Max stacks ({}): heals {:.0}% max HP.",
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
                let heal = apply_passive_stack(&mut self.stats, *p);
                if !self.splices.is_empty() {
                    // The splice multipliers go *after* the passives, so a
                    // passive picked while spliced has to replay them.
                    self.rebuild_stats();
                }
                heal
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
    /// their cards say how to fire them ("ACTIVE: Space").
    #[test]
    fn active_weapons_are_offered_and_their_cards_teach_the_space_key() {
        let lo = Loadout::starting();
        for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
            assert!(kind.is_active());
            let (title, desc, flavor) = lo.describe(&Upgrade::NewWeapon(kind));
            assert!(title.contains(kind.name()), "card title {title:?}");
            assert!(
                desc.contains("ACTIVE: Space"),
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

    /// The integrated pool: every one of the 12 weapons and 12 passives is
    /// offerable from a fresh build, and a seeded pick-through never exceeds
    /// either rack cap or deals a duplicate / illegal card.
    #[test]
    fn all_twelve_weapons_and_passives_are_offerable_within_the_caps() {
        assert_eq!(WeaponKind::all().len(), 12);
        assert_eq!(PassiveKind::all().len(), 12);
        let mut weapons_seen = [false; 12];
        let mut passives_seen = [false; 12];
        for seed in 0..600u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut lo = Loadout::starting();
            for _ in 0..70 {
                let choices = lo.generate_choices(&mut rng);
                assert!(!choices.is_empty() && choices.len() <= 3);
                for (i, c) in choices.iter().enumerate() {
                    match c {
                        Upgrade::NewWeapon(k) => {
                            assert!(lo.weapons.len() < MAX_WEAPONS && !lo.has(*k));
                            weapons_seen[k.index()] = true;
                        }
                        Upgrade::LevelWeapon(j) => assert!(!lo.weapons[*j].is_max()),
                        Upgrade::Passive(p) => {
                            assert!(lo.stacks(*p) < p.max_stacks());
                            assert!(lo.stacks(*p) > 0 || lo.passives.len() < MAX_PASSIVES);
                            let at = PassiveKind::all().iter().position(|k| k == p).unwrap();
                            passives_seen[at] = true;
                        }
                        Upgrade::Bandage | Upgrade::Purse => {}
                    }
                    for other in &choices[..i] {
                        assert_ne!(format!("{c:?}"), format!("{other:?}"), "duplicate card");
                    }
                }
                let pick = choices[rng.random_range(0..choices.len())].clone();
                lo.apply(&pick);
                assert!(lo.weapons.len() <= MAX_WEAPONS);
                assert!(lo.passives.len() <= MAX_PASSIVES);
            }
        }
        assert!(!weapons_seen[WeaponKind::Gnaw.index()], "the start weapon is owned");
        for kind in WeaponKind::all() {
            if kind != WeaponKind::Gnaw {
                assert!(weapons_seen[kind.index()], "{kind:?} was never offered");
            }
        }
        for (i, kind) in PassiveKind::all().into_iter().enumerate() {
            assert!(passives_seen[i], "{kind:?} was never offered");
        }
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
