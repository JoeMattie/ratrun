//! **The gene-splice shop behind the mouse holes** (`WAVE3_DESIGN.md` §6).
//!
//! A level carries two mouse holes in structural wall faces
//! ([`crate::game::level::ShopHole`], placed by
//! [`crate::game::levelgen::placement::place_shop_holes`]). Touching a hole's
//! threshold raises [`World::pending_shop`] and the App opens [`Screen::Shop`];
//! everything behind that screen lives here:
//!
//! | what | where |
//! |---|---|
//! | the ten splices, their prices, their two-sided bargains | [`SpliceKind`] |
//! | the passive/splice stat pipeline (splices go *after* passives) | [`apply_splices`] |
//! | the reader-only effects (coins, gem XP, healing, i-frames, incoming) | [`coin_mult`], [`gem_xp_mult`], [`heal_mult`], [`iframe_extra`], [`incoming_extra`] |
//! | the seeded shelf (floor seed + hole index + reroll count) | [`HoleStock::shelve`] |
//! | per-hole inventory that survives a revisit, the three services | [`ShopBook`], [`Purchase`] |
//! | the two-step Molt nomination | [`MoltStep`], [`molt_options`], [`molt`] |
//!
//! ## Where the owned splices live
//!
//! On the [`Loadout`](crate::game::loadout::Loadout) — `loadout.splices`, in
//! purchase order — because the loadout is the one thing the campaign carries
//! from level to level (`campaign::next_loadout` copies it) and because the
//! stat rebuild already replays the passive stacks there. Splices occupy
//! **neither rack**: they are not weapons and not passive stacks, they never
//! count against `MAX_WEAPONS`/`MAX_PASSIVES`, and the keep-three screen cannot
//! see them. They last until the run is over.
//!
//! ## Determinism
//!
//! The shelf is a pure function of `(floor seed, hole index, reroll count)` and
//! is rolled from its **own** [`StdRng`], never from the simulation's `rng`:
//! opening a shop, buying, rerolling and Molting draw no simulation rng at all,
//! so a seed that never visits a shop is bit-identical to one that never had
//! the feature. Every purchase is therefore deterministic for a seed, and a
//! twin run that buys the same things gets the same build.
//!
//! ## Floors
//!
//! [`apply_splices`] clamps at the end of the whole splice list: max HP never
//! drops below [`MIN_MAX_HP`] (1), the dash cooldown never below
//! [`MIN_DASH_CD`] (0.7 s), and the damage/fire-rate multipliers never reach
//! zero. `Player::hp` is clamped to the new maximum by the purchase that
//! changed it.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::game::world::World;

use crate::game::enemy::EnemyKind;
use crate::game::loadout::{PassiveKind, Stats};
use crate::game::weapon::{Weapon, WeaponKind};

/// How many splices one rat may carry: the two sockets.
pub const MAX_SPLICES: usize = 2;

/// How many splices one shelf offers.
pub const STOCK_SIZE: usize = 3;

/// How many times the shelf may be turned at one hole ("at most twice").
pub const MAX_REROLLS: u8 = 2;

/// Price of the first "turn the shelf", in gold. The second costs twice.
pub const REROLL_BASE_PRICE: u32 = 8;

/// Price of a Stitch.
pub const STITCH_PRICE: u32 = 12;

/// Price of a Molt.
pub const MOLT_PRICE: u32 = 22;

/// Fraction of max HP a Stitch restores.
pub const STITCH_FRACTION: f32 = 0.35;

/// The absolute floor a splice list may take max HP to.
pub const MIN_MAX_HP: f32 = 1.0;

/// The absolute floor a splice list may take the dash cooldown to.
pub const MIN_DASH_CD: f32 = 0.7;

/// Post-hit immunity a Double Lid adds (on top of the base 0.9 s).
pub const DOUBLE_LID_IFRAMES: f32 = 0.22;

/// Extra incoming damage Hollow Bones adds *before* armor.
pub const HOLLOW_BONES_INCOMING: f32 = 3.0;

// ============================================================================
// The ten splices
// ============================================================================

/// One permanent, two-sided gene splice.
///
/// Every one of them is a bargain: the label on the vial is the second half of
/// the deal, and both halves are shown on the shelf before payment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpliceKind {
    GlassIncisors,
    BoilerHeart,
    HollowBones,
    DoubleLid,
    HoardGland,
    CopperNerves,
    NightShift,
    BlackboardBelly,
    BallastSac,
    LongFingers,
}

impl SpliceKind {
    /// Every splice, in the design table's order (the shelf's canonical order,
    /// so a seed's stock is a stable sequence).
    pub const ALL: [SpliceKind; 10] = [
        SpliceKind::GlassIncisors,
        SpliceKind::BoilerHeart,
        SpliceKind::HollowBones,
        SpliceKind::DoubleLid,
        SpliceKind::HoardGland,
        SpliceKind::CopperNerves,
        SpliceKind::NightShift,
        SpliceKind::BlackboardBelly,
        SpliceKind::BallastSac,
        SpliceKind::LongFingers,
    ];

    pub fn all() -> &'static [SpliceKind; 10] {
        &Self::ALL
    }

    /// Index in [`Self::ALL`] (used to keep the art/icons in step).
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            SpliceKind::GlassIncisors => "Glass Incisors",
            SpliceKind::BoilerHeart => "Boiler Heart",
            SpliceKind::HollowBones => "Hollow Bones",
            SpliceKind::DoubleLid => "Double Lid",
            SpliceKind::HoardGland => "Hoard Gland",
            SpliceKind::CopperNerves => "Copper Nerves",
            SpliceKind::NightShift => "Night Shift",
            SpliceKind::BlackboardBelly => "Blackboard Belly",
            SpliceKind::BallastSac => "Ballast Sac",
            SpliceKind::LongFingers => "Long Fingers",
        }
    }

    pub fn price(self) -> u32 {
        match self {
            SpliceKind::GlassIncisors => 35,
            SpliceKind::BoilerHeart => 32,
            SpliceKind::HollowBones => 28,
            SpliceKind::DoubleLid => 30,
            SpliceKind::HoardGland => 30,
            SpliceKind::CopperNerves => 38,
            SpliceKind::NightShift => 34,
            SpliceKind::BlackboardBelly => 36,
            SpliceKind::BallastSac => 28,
            SpliceKind::LongFingers => 24,
        }
    }

    /// The label on the vial — the first line of the bargain's fine print.
    pub fn label(self) -> &'static str {
        match self {
            SpliceKind::GlassIncisors => "Handle by the roots.",
            SpliceKind::BoilerHeart => "Keeps warm in cold pipes.",
            SpliceKind::HollowBones => "Mind the landing.",
            SpliceKind::DoubleLid => "Blink once for each world.",
            SpliceKind::HoardGland => "The left cheek is for rent.",
            SpliceKind::CopperNerves => "Ground before handling.",
            SpliceKind::NightShift => "Sleep was the control group.",
            SpliceKind::BlackboardBelly => "Everything goes in the notebook.",
            SpliceKind::BallastSac => "A little sand steadies the hand.",
            SpliceKind::LongFingers => "Someone has to reach the shelf.",
        }
    }

    /// What the rat gains, in the shelf's own words.
    pub fn plus(self) -> &'static str {
        match self {
            SpliceKind::GlassIncisors => "+22% weapon damage",
            SpliceKind::BoilerHeart => "+1.0 HP/s regen",
            SpliceKind::HollowBones => "+15% move speed",
            SpliceKind::DoubleLid => "+0.22s post-hit immunity",
            SpliceKind::HoardGland => "+50% coin value",
            SpliceKind::CopperNerves => "-20% dash cooldown",
            SpliceKind::NightShift => "+18% fire rate",
            SpliceKind::BlackboardBelly => "+25% gem XP",
            SpliceKind::BallastSac => "+3 armor",
            SpliceKind::LongFingers => "+60% pickup radius",
        }
    }

    /// What the rat pays, always shown beside [`Self::plus`].
    pub fn minus(self) -> &'static str {
        match self {
            SpliceKind::GlassIncisors => "-18% max HP",
            SpliceKind::BoilerHeart => "-10% move speed",
            SpliceKind::HollowBones => "+3 incoming damage before armor",
            SpliceKind::DoubleLid => "-12% fire rate",
            SpliceKind::HoardGland => "-10% weapon damage",
            SpliceKind::CopperNerves => "-25% pickup radius",
            SpliceKind::NightShift => "halve passive HP/s",
            SpliceKind::BlackboardBelly => "-25% healing from items and shop",
            SpliceKind::BallastSac => "+15% dash cooldown",
            SpliceKind::LongFingers => "-10% max HP",
        }
    }
}

// ============================================================================
// The stat pipeline
// ============================================================================

/// Apply every owned splice to `stats`, **after** the passive stacks (the
/// caller — [`Loadout::rebuild_stats`] — has already replayed them).
///
/// Splices are applied in purchase order, so a list built by the same purchases
/// lands on the same floating-point numbers every run. The floors are applied
/// once at the end: max HP ≥ [`MIN_MAX_HP`], dash cooldown ≥ [`MIN_DASH_CD`],
/// and no multiplier is allowed to reach zero.
pub fn apply_splices(stats: &mut Stats, splices: &[SpliceKind]) {
    for &splice in splices {
        match splice {
            SpliceKind::GlassIncisors => {
                stats.damage_mult *= 1.22;
                stats.max_hp *= 0.82;
            }
            SpliceKind::BoilerHeart => {
                stats.regen += 1.0;
                stats.move_speed *= 0.90;
            }
            SpliceKind::HollowBones => {
                // The move speed side is a stat; the +3 incoming damage is read
                // where damage lands (`incoming_extra`).
                stats.move_speed *= 1.15;
            }
            SpliceKind::DoubleLid => {
                // The i-frames side is read by `Player::take_damage`.
                stats.fire_rate_mult *= 0.88;
            }
            SpliceKind::HoardGland => {
                // The coin side is read by `loot::treasure_system`.
                stats.damage_mult *= 0.90;
            }
            SpliceKind::CopperNerves => {
                stats.dash_cd *= 0.80;
                stats.magnet *= 0.75;
            }
            SpliceKind::NightShift => {
                // "Halve total passive HP/s": the regen the passives built (and
                // a Boiler Heart's +1, which is bought HP/s too) is halved.
                stats.fire_rate_mult *= 1.18;
                stats.regen *= 0.5;
            }
            SpliceKind::BlackboardBelly => {
                // Both sides are read-only multipliers (gem XP, healing).
            }
            SpliceKind::BallastSac => {
                stats.armor += 3.0;
                stats.dash_cd *= 1.15;
            }
            SpliceKind::LongFingers => {
                stats.magnet *= 1.60;
                stats.max_hp *= 0.90;
            }
        }
    }
    stats.max_hp = stats.max_hp.max(MIN_MAX_HP);
    stats.dash_cd = stats.dash_cd.max(MIN_DASH_CD);
    stats.damage_mult = stats.damage_mult.max(0.05);
    stats.fire_rate_mult = stats.fire_rate_mult.max(0.05);
    stats.magnet = stats.magnet.max(1.0);
    stats.regen = stats.regen.max(0.0);
}

fn owned(splices: &[SpliceKind], kind: SpliceKind) -> bool {
    splices.contains(&kind)
}

/// Extra incoming damage per hit, before armor (Hollow Bones: `+3`).
pub fn incoming_extra(splices: &[SpliceKind]) -> f32 {
    if owned(splices, SpliceKind::HollowBones) {
        HOLLOW_BONES_INCOMING
    } else {
        0.0
    }
}

/// Extra post-hit immunity in seconds (Double Lid: `+0.22`).
pub fn iframe_extra(splices: &[SpliceKind]) -> f32 {
    if owned(splices, SpliceKind::DoubleLid) {
        DOUBLE_LID_IFRAMES
    } else {
        0.0
    }
}

/// Coin value multiplier (Hoard Gland: `×1.5`).
pub fn coin_mult(splices: &[SpliceKind]) -> f32 {
    if owned(splices, SpliceKind::HoardGland) {
        1.5
    } else {
        1.0
    }
}

/// One coin's payout through [`coin_mult`], banking the half-gold.
///
/// The design's Hoard Gland is "+50% coin value (**bank halves**)" (§6): a
/// 1-gold coin cannot pay 1.5, and rounding it (`scaled`) paid **2** — a +100 %
/// bonus on the commonest coin in the game and a strictly better deal than the
/// design's own 60-gold break-even. This banks the odd half instead: each coin
/// adds its value in half-gold, whole pairs are paid out, and the remainder
/// rides to the next coin. Over a floor the rat collects +50 % and no more,
/// with the running total never below what plain rounding would have paid
/// after two coins.
pub fn coin_payout(value: u32, mult: f32, half_bank: &mut u32) -> u32 {
    if mult != 1.5 {
        return scaled(value, mult);
    }
    *half_bank = half_bank.saturating_add(value);
    let bonus = *half_bank / 2;
    *half_bank %= 2;
    value.saturating_add(bonus)
}

/// Gem XP multiplier (Blackboard Belly: `×1.25`).
///
/// `loot::gems_system` multiplies this onto `Stats::xp_mult` (Second Scent)
/// and banks the combined fraction in the one `resources::XpBank`, so the two
/// bonuses stack as a product and no fractional XP is lost or paid twice.
pub fn gem_xp_mult(splices: &[SpliceKind]) -> f32 {
    if owned(splices, SpliceKind::BlackboardBelly) {
        1.25
    } else {
        1.0
    }
}

/// Healing-from-items-and-shop multiplier (Blackboard Belly: `×0.75`).
pub fn heal_mult(splices: &[SpliceKind]) -> f32 {
    if owned(splices, SpliceKind::BlackboardBelly) {
        0.75
    } else {
        1.0
    }
}

/// `value` through `mult`, rounded, in integer XP/coin units. The `mult == 1.0`
/// fast path keeps a run with no splices bit-identical to the base game.
#[inline]
pub fn scaled(value: u32, mult: f32) -> u32 {
    if mult == 1.0 {
        value
    } else {
        (value as f32 * mult).round() as u32
    }
}

// ============================================================================
// Gold (WAVE3_DESIGN §6)
// ============================================================================

/// Elites always pay this much, replacing their kind's own roll.
pub const ELITE_GOLD: u32 = 5;

/// A miniboss pays this alongside its chest (`w3-timeline` raises it).
pub const MINIBOSS_GOLD: u32 = 12;

/// The full boss pays this, credited at the clear.
pub const BOSS_GOLD: u32 = 30;

/// The design's chance × value table, by kind.
///
/// Kinds the table does not name (new ordinary kinds land with their own
/// tuning) fall back to the bat's row, so a new kind never silently pays
/// nothing. The wildcard arm is therefore kept while every current variant is
/// named — hence the `unreachable_patterns` allowance.
#[allow(unreachable_patterns)]
pub fn gold_row(kind: EnemyKind) -> (f32, u32) {
    match kind {
        EnemyKind::Skitterer => (0.08, 1),
        EnemyKind::Bat => (0.10, 1),
        EnemyKind::Cat => (0.18, 2),
        EnemyKind::Spitter => (0.20, 2),
        EnemyKind::Brute => (0.45, 3),
        // The five wave-3 kinds (`WAVE3_DESIGN.md` §6).
        EnemyKind::Pillbug => (0.18, 2),
        EnemyKind::BellFlea => (0.14, 2),
        EnemyKind::WickMoth => (0.22, 2),
        EnemyKind::Porter => (0.40, 3),
        EnemyKind::Turnkey => (0.32, 3),
        // Wave 4: gold in the band of a kind of similar threat. The snake is
        // the rare one and pays accordingly; the centipede is cheap because it
        // is a two-hit skirmisher that flees.
        EnemyKind::BigSnake => (0.55, 4),
        EnemyKind::HouseCentipede => (0.12, 2),
        EnemyKind::MurderHornet => (0.22, 2),
        EnemyKind::TermiteHorde => (0.20, 2),
        EnemyKind::Boss => (1.0, BOSS_GOLD),
        _ => (0.10, 1),
    }
}

/// A seeded uniform in `[0, 1)`, keyed by the floor seed and the victim's
/// `SpawnId`: the gold-only roll the design asks for. It never touches the
/// simulation's rng, so adding gold cannot shift a single drop roll.
pub fn gold_roll(floor_seed: u64, spawn_id: u64) -> f32 {
    let mut h = floor_seed
        ^ 0xA24B_AED4_963E_E407u64
        ^ spawn_id.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    ((h >> 40) as f32) / (1u32 << 24) as f32
}

/// What one kill pays: 0 (nothing dropped), or the whole value of its coin.
///
/// * the full boss pays [`BOSS_GOLD`] on the clear, always;
/// * an elite pays [`ELITE_GOLD`] instead of its kind's roll;
/// * everything else rolls its kind's chance once.
pub fn gold_for(floor_seed: u64, spawn_id: u64, kind: EnemyKind, elite: bool) -> u32 {
    if kind == EnemyKind::Boss {
        return BOSS_GOLD;
    }
    if elite {
        return ELITE_GOLD;
    }
    let (chance, value) = gold_row(kind);
    if gold_roll(floor_seed, spawn_id) < chance {
        value
    } else {
        0
    }
}

// ============================================================================
// The shelf
// ============================================================================

/// Deterministic shelf seed: floor seed, hole index and reroll count, mixed
/// with the splitmix64 finaliser (the same one `level::level_seed` uses) so
/// neighbouring holes and neighbouring floors are unrelated shelves.
pub fn shelf_seed(floor_seed: u64, hole: usize, reroll: u8) -> u64 {
    let mut h = floor_seed
        ^ 0x5B0F_9A31_7C2E_44D1u64
        ^ ((hole as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        ^ ((reroll as u64 + 1).wrapping_mul(0xc2b2_ae3d_27d4_eb4f));
    h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// Roll one shelf of unsold stock: `STOCK_SIZE` distinct splices from `pool`.
fn roll_shelf(
    floor_seed: u64,
    hole: usize,
    reroll: u8,
    pool: &mut [SpliceKind],
) -> [Option<SpliceKind>; STOCK_SIZE] {
    let mut out = [None; STOCK_SIZE];
    if pool.is_empty() {
        return out;
    }
    let mut rng = StdRng::seed_from_u64(shelf_seed(floor_seed, hole, reroll));
    // Shuffle the pool with the seeded stream (Fisher–Yates, one draw per
    // slot), then take the first `STOCK_SIZE`: deterministic and unbiased.
    for i in (1..pool.len()).rev() {
        let j = rng.random_range(0..=i);
        pool.swap(i, j);
    }
    for (slot, out_slot) in out.iter_mut().enumerate() {
        *out_slot = pool.get(slot).copied();
    }
    out
}

/// What one hole stocks and what has happened there.
///
/// This is the "inventory persists on revisit" record: the shelf is generated
/// once, when the hole first opens, and every later visit sees the same slots
/// (sold ones stay sold, the services stay spent).
#[derive(Clone, Debug, Default)]
pub struct HoleStock {
    /// Which hole this is (`LevelObjects::shop_holes` index).
    pub hole: usize,
    /// What is on the shelf right now (`None` once the shelf runs dry).
    pub stock: [Option<SpliceKind>; STOCK_SIZE],
    /// Slot bought?
    pub sold: [bool; STOCK_SIZE],
    /// Every splice this hole has ever offered, so a reroll only brings stock
    /// the rat has not seen yet.
    pub seen: Vec<SpliceKind>,
    /// How many times the shelf has been turned (0..=MAX_REROLLS).
    pub rerolls: u8,
    /// Stitch is once per hole.
    pub stitched: bool,
    /// Molt is once per hole.
    pub molted: bool,
    /// How many times the shelf was opened (tests/diagnostics).
    pub opens: u32,
}

impl HoleStock {
    /// Build (or rebuild) the shelf at generation `rerolls`.
    fn shelve(&mut self, floor_seed: u64, exclude_owned: &[SpliceKind]) {
        let mut pool: Vec<SpliceKind> = SpliceKind::ALL
            .iter()
            .copied()
            .filter(|k| !self.seen.contains(k))
            .filter(|k| !exclude_owned.contains(k))
            .collect();
        let shelf = roll_shelf(floor_seed, self.hole, self.rerolls, &mut pool);
        self.stock = shelf;
        self.sold = [false; STOCK_SIZE];
        for kind in self.stock.iter().flatten() {
            self.seen.push(*kind);
        }
    }

    /// The splice in `slot`, if it is still on the shelf.
    pub fn slot(&self, slot: usize) -> Option<SpliceKind> {
        self.stock.get(slot).copied().flatten()
    }

    /// Replace the unsold slots with unseen stock (the "turn the shelf").
    fn turn(&mut self, floor_seed: u64, exclude_owned: &[SpliceKind]) {
        // The unsold slots are cleared and rolled again at the next generation,
        // keeping the sold ones exactly where they are.
        let sold = self.sold;
        self.rerolls = self.rerolls.saturating_add(1);
        let mut pool: Vec<SpliceKind> = SpliceKind::ALL
            .iter()
            .copied()
            .filter(|k| !self.seen.contains(k))
            .filter(|k| !exclude_owned.contains(k))
            .collect();
        let fresh = roll_shelf(floor_seed, self.hole, self.rerolls, &mut pool);
        let mut next = 0usize;
        for slot in 0..STOCK_SIZE {
            if sold[slot] {
                continue;
            }
            self.stock[slot] = fresh[next];
            if let Some(kind) = fresh[next] {
                self.seen.push(kind);
            }
            next += 1;
        }
    }
}

/// Every hole's shelf on this level, plus the visited-hole bookkeeping the
/// scene needs between ticks. Owned by [`World`](crate::game::world::World);
/// nothing in the simulation reads it.
#[derive(Clone, Debug, Default)]
pub struct ShopBook {
    pub holes: Vec<HoleStock>,
}

impl ShopBook {
    /// The stock record for `hole`, generated on the first visit from
    /// `(floor seed, hole index, 0)`.
    pub fn open(&mut self, floor_seed: u64, hole: usize, owned: &[SpliceKind]) -> &mut HoleStock {
        while self.holes.len() <= hole {
            let index = self.holes.len();
            let mut record = HoleStock {
                hole: index,
                ..HoleStock::default()
            };
            record.shelve(floor_seed, owned);
            self.holes.push(record);
        }
        let record = &mut self.holes[hole];
        record.opens = record.opens.saturating_add(1);
        record
    }

    /// The record for `hole` if the shelf has been opened at least once.
    pub fn get(&self, hole: usize) -> Option<&HoleStock> {
        self.holes.get(hole)
    }

    /// The record for `hole`, mutably, if it exists.
    pub fn get_mut(&mut self, hole: usize) -> Option<&mut HoleStock> {
        self.holes.get_mut(hole)
    }

    /// What the shelf *would* offer at `hole` with nothing bought yet: a pure
    /// query the bot uses to decide whether a hole is worth walking to, without
    /// opening it. Never allocates a record.
    pub fn peek(floor_seed: u64, hole: usize) -> [Option<SpliceKind>; STOCK_SIZE] {
        let mut pool: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
        roll_shelf(floor_seed, hole, 0, &mut pool)
    }
}

// ============================================================================
// What can be bought
// ============================================================================

/// One actionable row on the shelf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopItem {
    /// The splice in shelf slot `n`.
    Splice(usize),
    /// Heal 35% max HP, once per hole.
    Stitch,
    /// Turn the shelf, twice per hole.
    Reroll,
    /// The two-step Molt nomination (its own flow: [`MoltStep`]).
    Molt,
    /// Leave the screen.
    Leave,
}

/// The shelf's rows, in keyboard order: the three splices, then the services,
/// then the exit.
pub const ROW_COUNT: usize = 7;

pub fn row_item(row: usize) -> ShopItem {
    match row {
        0 => ShopItem::Splice(0),
        1 => ShopItem::Splice(1),
        2 => ShopItem::Splice(2),
        3 => ShopItem::Stitch,
        4 => ShopItem::Reroll,
        5 => ShopItem::Molt,
        _ => ShopItem::Leave,
    }
}

/// Why a row is disabled. `None` in [`Purchase`] means "bought".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deny {
    /// Not enough gold.
    Gold,
    /// Both splice sockets are taken.
    Sockets,
    /// This splice is already in the rat.
    Owned,
    /// The shelf slot is empty (sold or never stocked).
    Sold,
    /// Stitch: already used at this hole.
    Stitched,
    /// Stitch: the rat is at full health.
    FullHp,
    /// Reroll: both turns are spent.
    NoTurns,
    /// Molt: already used at this hole.
    Molted,
    /// Molt: nothing to forfeit / nothing to take.
    NoChoice,
    /// The shop is not open.
    Closed,
}

impl Deny {
    /// The one-line reason the shelf prints on a disabled row.
    pub fn reason(self) -> &'static str {
        match self {
            Deny::Gold => "not enough gold",
            Deny::Sockets => "both sockets full",
            Deny::Owned => "already spliced in",
            Deny::Sold => "sold",
            Deny::Stitched => "already stitched",
            Deny::FullHp => "at full health",
            Deny::NoTurns => "shelf turned twice",
            Deny::Molted => "already molted",
            Deny::NoChoice => "no eligible kind",
            Deny::Closed => "the shop is closed",
        }
    }
}

/// Price of a shelf slot's splice (`None` when the slot is empty).
pub fn slot_price(stock: &HoleStock, slot: usize) -> Option<u32> {
    stock.slot(slot).map(SpliceKind::price)
}

/// Price of the next "turn the shelf", or `None` once both turns are spent.
pub fn reroll_price(stock: &HoleStock) -> Option<u32> {
    (stock.rerolls < MAX_REROLLS).then(|| REROLL_BASE_PRICE << stock.rerolls)
}

/// What one buy attempt did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purchase {
    /// Bought for this many gold.
    Bought { price: u32 },
    /// Nothing bought, with the shelf's own reason.
    Denied(Deny),
}

impl Purchase {
    pub fn price(self) -> Option<u32> {
        match self {
            Purchase::Bought { price } => Some(price),
            Purchase::Denied(_) => None,
        }
    }

    pub fn is_bought(self) -> bool {
        self.price().is_some()
    }
}

/// Why `item` cannot be bought right now, or `None` when it can.
pub fn deny_reason(world: &World, hole: usize, item: ShopItem) -> Option<Deny> {
    let Some(stock) = world.shop.get(hole) else {
        return Some(Deny::Closed);
    };
    match item {
        ShopItem::Splice(slot) => match stock.slot(slot) {
            None => Some(Deny::Sold),
            Some(kind) => {
                if stock.sold.get(slot).copied().unwrap_or(false) {
                    return Some(Deny::Sold);
                }
                if world.player.loadout.splices.contains(&kind) {
                    return Some(Deny::Owned);
                }
                if world.player.loadout.splices.len() >= MAX_SPLICES {
                    return Some(Deny::Sockets);
                }
                if world.gold < kind.price() {
                    return Some(Deny::Gold);
                }
                None
            }
        },
        ShopItem::Stitch => {
            if stock.stitched {
                Some(Deny::Stitched)
            } else if world.player.hp >= world.player.max_hp() - 0.01 {
                Some(Deny::FullHp)
            } else if world.gold < STITCH_PRICE {
                Some(Deny::Gold)
            } else {
                None
            }
        }
        ShopItem::Molt => molt_deny(world, hole),
        ShopItem::Reroll => match reroll_price(stock) {
            None => Some(Deny::NoTurns),
            Some(price) if world.gold < price => Some(Deny::Gold),
            Some(_) => None,
        },
        ShopItem::Leave => None,
    }
}

impl World {
    /// Open the shelf at `hole` (generating it on the first visit) and record
    /// which hole is open. Called by the App and the bot before they touch
    /// [`World::shop`].
    pub fn open_shop(&mut self, hole: usize) {
        let seed = self.level.seed;
        let owned = self.player.loadout.splices.clone();
        self.shop.open(seed, hole, &owned);
        self.pending_shop_hole = hole;
    }

    /// The shop screen closed: drop the flag and grant the 0.8 s exit grace the
    /// system needs before the threshold may fire again.
    pub fn leave_shop(&mut self) {
        self.pending_shop = false;
        self.shop_runtime_mut().grace = crate::game::ecs::systems::shop::EXIT_GRACE_SECS;
    }
}

// ============================================================================
// Services
// ============================================================================

/// Heal 35% max HP through the Blackboard Belly multiplier; once per hole.
fn stitch(world: &mut World, hole: usize) -> Purchase {
    let Some(stock) = world.shop.get_mut(hole) else {
        return Purchase::Denied(Deny::Closed);
    };
    if stock.stitched {
        return Purchase::Denied(Deny::Stitched);
    }
    if world.player.hp >= world.player.max_hp() - 0.01 {
        return Purchase::Denied(Deny::FullHp);
    }
    if world.gold < STITCH_PRICE {
        return Purchase::Denied(Deny::Gold);
    }
    let heal = world.player.max_hp() * STITCH_FRACTION * heal_mult(&world.player.loadout.splices);
    world.spend_gold(STITCH_PRICE);
    world.player.heal(heal);
    if let Some(stock) = world.shop.get_mut(hole) {
        stock.stitched = true;
    }
    Purchase::Bought {
        price: STITCH_PRICE,
    }
}

/// Turn the shelf: unsold slots become unseen stock, twice per hole.
fn reroll(world: &mut World, hole: usize) -> Purchase {
    let Some(price) = world.shop.get(hole).and_then(reroll_price) else {
        return Purchase::Denied(if world.shop.get(hole).is_some() {
            Deny::NoTurns
        } else {
            Deny::Closed
        });
    };
    if world.gold < price {
        return Purchase::Denied(Deny::Gold);
    }
    let seed = world.level.seed;
    let owned = world.player.loadout.splices.clone();
    world.spend_gold(price);
    if let Some(stock) = world.shop.get_mut(hole) {
        stock.turn(seed, &owned);
    }
    Purchase::Bought { price }
}

/// Buy shelf `slot`: the splice goes straight into the rat (it is permanent),
/// the stats are rebuilt from the passive stacks and the whole splice list, and
/// current HP is clamped to the new maximum.
fn buy_splice(world: &mut World, hole: usize, slot: usize) -> Purchase {
    let Some(kind) = world.shop.get(hole).and_then(|stock| stock.slot(slot)) else {
        return Purchase::Denied(Deny::Sold);
    };
    if let Some(deny) = deny_reason(world, hole, ShopItem::Splice(slot)) {
        return Purchase::Denied(deny);
    }
    let price = kind.price();
    world.spend_gold(price);
    if let Some(stock) = world.shop.get_mut(hole) {
        stock.sold[slot] = true;
    }
    world.player.loadout.splices.push(kind);
    world.player.loadout.rebuild_stats();
    clamp_hp(&mut world.player);
    Purchase::Bought { price }
}

/// Buy one shelf row (Molt has its own two-step entry point, [`molt`]).
pub fn buy(world: &mut World, hole: usize, item: ShopItem) -> Purchase {
    match item {
        ShopItem::Splice(slot) => buy_splice(world, hole, slot),
        ShopItem::Stitch => stitch(world, hole),
        ShopItem::Reroll => reroll(world, hole),
        // Molt is a two-step choice; it cannot be bought in one press.
        ShopItem::Molt => Purchase::Denied(Deny::NoChoice),
        ShopItem::Leave => Purchase::Denied(Deny::Closed),
    }
}

/// Cap current HP at the (possibly shrunken) maximum. A splices' max-HP cuts
/// must never leave the rat above its own ceiling.
pub fn clamp_hp(player: &mut crate::game::player::Player) {
    let max = player.max_hp();
    if player.hp > max {
        player.hp = max;
    }
    if player.hp < 0.0 {
        player.hp = 0.0;
    }
}

// ============================================================================
// Molt
// ============================================================================

/// One step of the Molt flow. The App renders the step and feeds it selections;
/// the shelf only answers the questions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoltStep {
    /// Step one: nominate the owned kind to leave behind.
    Forfeit,
    /// Step two: pick the replacement in the same category, at rank 1.
    Pick,
}

/// One owned kind that may be forfeited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnedKind {
    Weapon(usize),
    Passive(usize),
}

impl OwnedKind {
    pub fn is_weapon(self) -> bool {
        matches!(self, OwnedKind::Weapon(_))
    }
}

/// One replacement the rat could take: a kind it does not own, at rank 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewKind {
    Weapon(WeaponKind),
    Passive(PassiveKind),
}

/// The kinds the rat may forfeit at this hole, in rack order (weapons first,
/// then passives in acquisition order).
pub fn molt_owned(world: &World) -> Vec<OwnedKind> {
    let loadout = &world.player.loadout;
    let mut out = Vec::with_capacity(loadout.weapons.len() + loadout.passives.len());
    for i in 0..loadout.weapons.len() {
        out.push(OwnedKind::Weapon(i));
    }
    for i in 0..loadout.passives.len() {
        out.push(OwnedKind::Passive(i));
    }
    out
}

/// The display line of a forfeit candidate ("Gnaw Lv5 ✦").
pub fn owned_label(world: &World, owned: OwnedKind) -> String {
    let loadout = &world.player.loadout;
    match owned {
        OwnedKind::Weapon(i) => match loadout.weapons.get(i) {
            Some(w) => {
                let evolved = if w.evolved { " ✦EVOLVED" } else { "" };
                format!("{} Lv{}{evolved}", w.kind.name(), w.level)
            }
            None => String::new(),
        },
        OwnedKind::Passive(i) => match loadout.passives.get(i) {
            Some((kind, stacks)) => format!("{} x{stacks}", kind.name()),
            None => String::new(),
        },
    }
}

/// The replacement candidates for a forfeited kind: unowned kinds in the same
/// category, at rank 1. A full rack of its category still allows a swap (the
/// forfeit frees the slot), so only "every kind of that category is owned"
/// leaves the step empty.
pub fn molt_options(world: &World, forfeit: OwnedKind) -> Vec<NewKind> {
    let loadout = &world.player.loadout;
    let mut out = Vec::new();
    match forfeit {
        OwnedKind::Weapon(_) => {
            for kind in WeaponKind::all() {
                if !loadout.has(kind) {
                    out.push(NewKind::Weapon(kind));
                }
            }
        }
        OwnedKind::Passive(_) => {
            for kind in PassiveKind::all() {
                if loadout.stacks(kind) == 0 {
                    out.push(NewKind::Passive(kind));
                }
            }
        }
    }
    out
}

/// The label of a replacement candidate ("Acid Rain Lv1").
pub fn new_label(kind: NewKind) -> String {
    match kind {
        NewKind::Weapon(k) => format!("{} Lv1", k.name()),
        NewKind::Passive(k) => format!("{} x1", k.name()),
    }
}

/// Why Molt is unavailable, or `None` when the two-step flow may start.
pub fn molt_deny(world: &World, hole: usize) -> Option<Deny> {
    let Some(stock) = world.shop.get(hole) else {
        return Some(Deny::Closed);
    };
    if stock.molted {
        return Some(Deny::Molted);
    }
    if world.gold < MOLT_PRICE {
        return Some(Deny::Gold);
    }
    // There has to be something to leave behind, and something to take in its
    // place: an empty rack or a rack that already owns every kind of its
    // category has nothing to offer.
    let owned = molt_owned(world);
    if owned.is_empty() {
        return Some(Deny::NoChoice);
    }
    if !owned
        .iter()
        .any(|kind| !molt_options(world, *kind).is_empty())
    {
        return Some(Deny::NoChoice);
    }
    None
}

/// Finish a Molt: forfeit `owned`, take `new` at rank 1, recompute the stats
/// from the passive stacks + splices, and clamp HP.
pub fn molt(
    world: &mut World,
    hole: usize,
    owned: OwnedKind,
    new: NewKind,
) -> Result<(), Deny> {
    if let Some(deny) = molt_deny(world, hole) {
        return Err(deny);
    }
    // The nomination and the replacement must agree on the category.
    let same_category = matches!((owned, new), (OwnedKind::Weapon(_), NewKind::Weapon(_)))
        || matches!((owned, new), (OwnedKind::Passive(_), NewKind::Passive(_)));
    if !same_category {
        return Err(Deny::NoChoice);
    }
    match (owned, new) {
        (OwnedKind::Weapon(i), NewKind::Weapon(kind)) => {
            if world.player.loadout.has(kind) || world.player.loadout.weapons.get(i).is_none() {
                return Err(Deny::NoChoice);
            }
            world.player.loadout.weapons[i] = Weapon::new(kind);
        }
        (OwnedKind::Passive(i), NewKind::Passive(kind)) => {
            if world.player.loadout.stacks(kind) > 0
                || world.player.loadout.passives.get(i).is_none()
            {
                return Err(Deny::NoChoice);
            }
            world.player.loadout.passives[i] = (kind, 1);
        }
        _ => return Err(Deny::NoChoice),
    }
    world.spend_gold(MOLT_PRICE);
    world.player.loadout.rebuild_stats();
    clamp_hp(&mut world.player);
    if let Some(stock) = world.shop.get_mut(hole) {
        stock.molted = true;
    }
    Ok(())
}

#[cfg(test)]
mod coin_tests {
    use super::*;

    /// Hoard Gland is "+50% coin value (**bank halves**)": a 1-gold coin pays 1
    /// and banks the half, so a second one pays 2 — +50% over the floor, never
    /// the +100% that rounding 1.5 up paid per coin.
    #[test]
    fn hoard_gland_banks_halves() {
        let mut bank = 0u32;
        let first = coin_payout(1, 1.5, &mut bank);
        let second = coin_payout(1, 1.5, &mut bank);
        assert_eq!((first, second), (1, 2), "two 1-gold coins pay three gold");
        // A 3-gold coin and a second one: 4 paid + 2 banked bonus = +50 %.
        let mut bank = 0u32;
        assert_eq!(coin_payout(3, 1.5, &mut bank), 4);
        assert_eq!(coin_payout(3, 1.5, &mut bank), 5);
        // The long run converges on exactly +50 %.
        let mut bank = 0u32;
        let paid: u32 = (0..100).map(|_| coin_payout(1, 1.5, &mut bank)).sum();
        assert_eq!(paid, 150, "100 one-gold coins must pay 150");
        // No splice: untouched.
        let mut bank = 0u32;
        assert_eq!(coin_payout(3, 1.0, &mut bank), 3);
        assert_eq!(bank, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::loadout::{self, Loadout, Upgrade};
    use crate::game::weapon::WeaponKind;

    fn world(seed: u64) -> World {
        World::new(Theme::Sewer, seed)
    }

    /// A world with `gold` in the purse and hole 0's shelf open.
    fn shop_world(seed: u64, gold: u32) -> World {
        let mut w = world(seed);
        w.gold = gold;
        w.open_shop(0);
        w
    }

    #[test]
    fn every_splice_has_a_price_and_two_distinct_sides() {
        let mut names: Vec<&str> = Vec::new();
        for kind in SpliceKind::ALL {
            assert!(kind.price() >= 20, "{:?} is too cheap", kind);
            assert!(!kind.name().is_empty());
            assert!(!kind.label().is_empty());
            assert_ne!(kind.plus(), kind.minus(), "{kind:?} has one side only");
            assert!(!names.contains(&kind.name()), "{kind:?} reuses a name");
            names.push(kind.name());
        }
        assert_eq!(SpliceKind::ALL.len(), 10);
    }

    #[test]
    fn every_splice_changes_both_of_its_numbers() {
        for kind in SpliceKind::ALL {
            let mut lo = Loadout::starting();
            // A couple of passives, so "after the passives" is actually tested.
            lo.apply(&Upgrade::Passive(PassiveKind::MaxHp));
            lo.apply(&Upgrade::Passive(PassiveKind::Damage));
            let before = lo.stats;
            lo.splices.push(kind);
            lo.rebuild_stats();
            let after = &lo.stats;
            let changed = |a: f32, b: f32| (a - b).abs() > 1e-4;
            let any_stat = changed(before.max_hp, after.max_hp)
                || changed(before.move_speed, after.move_speed)
                || changed(before.damage_mult, after.damage_mult)
                || changed(before.fire_rate_mult, after.fire_rate_mult)
                || changed(before.magnet, after.magnet)
                || changed(before.regen, after.regen)
                || changed(before.armor, after.armor)
                || changed(before.dash_cd, after.dash_cd);
            let any_reader = coin_mult(&lo.splices) != 1.0
                || gem_xp_mult(&lo.splices) != 1.0
                || heal_mult(&lo.splices) != 1.0
                || iframe_extra(&lo.splices) != 0.0
                || incoming_extra(&lo.splices) != 0.0;
            assert!(
                any_stat || any_reader,
                "{kind:?} changes nothing at all"
            );

            // The exact two numbers the card promises.
            match kind {
                SpliceKind::GlassIncisors => {
                    assert!((after.damage_mult - before.damage_mult * 1.22).abs() < 1e-5);
                    assert!((after.max_hp - before.max_hp * 0.82).abs() < 1e-3);
                }
                SpliceKind::BoilerHeart => {
                    assert!(after.regen > before.regen);
                    assert!((after.move_speed - before.move_speed * 0.90).abs() < 1e-3);
                }
                SpliceKind::HollowBones => {
                    assert_eq!(incoming_extra(&lo.splices), HOLLOW_BONES_INCOMING);
                    assert!((after.move_speed - before.move_speed * 1.15).abs() < 1e-3);
                }
                SpliceKind::DoubleLid => {
                    assert_eq!(iframe_extra(&lo.splices), DOUBLE_LID_IFRAMES);
                    assert!((after.fire_rate_mult - before.fire_rate_mult * 0.88).abs() < 1e-5);
                }
                SpliceKind::HoardGland => {
                    assert_eq!(coin_mult(&lo.splices), 1.5);
                    assert!((after.damage_mult - before.damage_mult * 0.90).abs() < 1e-5);
                }
                SpliceKind::CopperNerves => {
                    assert!((after.dash_cd - before.dash_cd * 0.80).abs() < 1e-5);
                    assert!((after.magnet - before.magnet * 0.75).abs() < 1e-3);
                }
                SpliceKind::NightShift => {
                    assert!((after.fire_rate_mult - before.fire_rate_mult * 1.18).abs() < 1e-5);
                    assert!((after.regen - before.regen * 0.5).abs() < 1e-5);
                }
                SpliceKind::BlackboardBelly => {
                    assert_eq!(gem_xp_mult(&lo.splices), 1.25);
                    assert_eq!(heal_mult(&lo.splices), 0.75);
                }
                SpliceKind::BallastSac => {
                    assert!((after.armor - before.armor - 3.0).abs() < 1e-5);
                    assert!((after.dash_cd - before.dash_cd * 1.15).abs() < 1e-5);
                }
                SpliceKind::LongFingers => {
                    assert!((after.magnet - before.magnet * 1.60).abs() < 1e-3);
                    assert!((after.max_hp - before.max_hp * 0.90).abs() < 1e-3);
                }
            }
        }
    }

    #[test]
    fn the_splice_floors_hold_even_with_every_bargain_at_once() {
        // A rat cannot own all ten (two sockets), but the pipeline must still
        // never take the numbers past the design's floors.
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::Passive(PassiveKind::Regen));
        lo.splices = SpliceKind::ALL.to_vec();
        lo.rebuild_stats();
        assert!(lo.stats.max_hp >= MIN_MAX_HP, "max HP floor");
        assert!(lo.stats.dash_cd >= MIN_DASH_CD, "dash cooldown floor");
        assert!(lo.stats.damage_mult > 0.0);
        assert!(lo.stats.fire_rate_mult > 0.0);
        // And the two the design names explicitly, on their own:
        let mut lo = Loadout::starting();
        lo.splices = vec![SpliceKind::GlassIncisors, SpliceKind::LongFingers];
        lo.rebuild_stats();
        assert!(lo.stats.max_hp >= MIN_MAX_HP);
        let mut lo = Loadout::starting();
        lo.splices = vec![SpliceKind::CopperNerves, SpliceKind::BallastSac];
        lo.rebuild_stats();
        assert!(lo.stats.dash_cd >= MIN_DASH_CD);
    }

    #[test]
    fn splices_apply_after_the_passive_stats() {
        // Sharp Teeth x2 (+30%) then Glass Incisors (x1.22) is 1.30 * 1.22, not
        // 1.0 * 1.22 + 0.30.
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        lo.splices.push(SpliceKind::GlassIncisors);
        lo.rebuild_stats();
        assert!((lo.stats.damage_mult - 1.30 * 1.22).abs() < 1e-5);
        // ... and a passive picked *after* the splice still lands first.
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        assert!((lo.stats.damage_mult - 1.45 * 1.22).abs() < 1e-5);
    }

    #[test]
    fn a_shelf_is_a_pure_function_of_seed_hole_and_reroll() {
        for seed in [0u64, 7, 99] {
            for hole in 0..2 {
                let mut a: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
                let mut b: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
                let shelf_a = roll_shelf(seed, hole, 0, &mut a);
                let shelf_b = roll_shelf(seed, hole, 0, &mut b);
                assert_eq!(shelf_a, shelf_b, "same seed + hole = same shelf");
                assert_eq!(shelf_a.iter().flatten().count(), STOCK_SIZE);
                let mut list: Vec<SpliceKind> = shelf_a.iter().flatten().copied().collect();
                list.dedup();
                assert_eq!(list.len(), STOCK_SIZE, "three distinct splices");
            }
        }
        // Different hole, seed or reroll: a different shelf.
        let mut p: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
        let base = roll_shelf(11, 0, 0, &mut p);
        let mut p: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
        assert_ne!(base, roll_shelf(11, 1, 0, &mut p));
        let mut p: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
        assert_ne!(base, roll_shelf(12, 0, 0, &mut p));
        let mut p: Vec<SpliceKind> = SpliceKind::ALL.to_vec();
        assert_ne!(base, roll_shelf(11, 0, 1, &mut p));
        assert_ne!(shelf_seed(11, 0, 0), shelf_seed(11, 0, 1));
    }

    #[test]
    fn the_shelf_opens_once_and_the_inventory_survives_a_revisit() {
        let mut w = shop_world(3, 200);
        let shelf = {
            let stock = w.shop.get(0).expect("shelf opened");
            [stock.slot(0), stock.slot(1), stock.slot(2)]
        };
        assert_eq!(shelf.iter().flatten().count(), STOCK_SIZE);
        let first = w.shop.get(0).unwrap().slot(0).unwrap();
        assert!(buy(&mut w, 0, ShopItem::Splice(0)).is_bought());
        assert_eq!(w.gold, 200 - first.price());
        assert!(w.player.loadout.splices.contains(&first));

        // Stepping out and coming back (a revisit) sees the same shelf, with
        // the sold slot still sold.
        w.leave_shop();
        w.open_shop(0);
        let revisit = {
            let stock = w.shop.get(0).unwrap();
            (
                [stock.slot(0), stock.slot(1), stock.slot(2)],
                stock.sold[0],
                stock.opens,
            )
        };
        assert_eq!(revisit.0, shelf);
        assert!(revisit.1);
        assert_eq!(revisit.2, 2, "the same shelf, opened twice");
        assert_eq!(
            buy(&mut w, 0, ShopItem::Splice(0)),
            Purchase::Denied(Deny::Sold),
            "a sold slot cannot be bought twice"
        );
        // The other hole stocks something else entirely.
        w.open_shop(1);
        let other = {
            let stock = w.shop.get(1).unwrap();
            [stock.slot(0), stock.slot(1), stock.slot(2)]
        };
        assert_eq!(other.iter().flatten().count(), STOCK_SIZE);
        assert_ne!(
            shelf, other,
            "two holes on one floor do not share a shelf"
        );
    }

    #[test]
    fn two_sockets_and_no_third_splice() {
        let mut w = shop_world(5, 500);
        let mut bought = 0;
        for slot in 0..STOCK_SIZE {
            if buy(&mut w, 0, ShopItem::Splice(slot)).is_bought() {
                bought += 1;
            }
        }
        // Two sockets only: even a rich rat with a full shelf stops at two.
        assert_eq!(w.player.loadout.splices.len(), MAX_SPLICES);
        assert_eq!(bought, 2, "the third slot is refused: sockets full");
        let third = (0..STOCK_SIZE).find(|slot| {
            deny_reason(&w, 0, ShopItem::Splice(*slot)) == Some(Deny::Sockets)
                || deny_reason(&w, 0, ShopItem::Splice(*slot)) == Some(Deny::Sold)
        });
        assert!(third.is_some());
        // And the reason the shelf prints is the sockets one.
        let mut w = shop_world(6, 500);
        w.player.loadout.splices = vec![SpliceKind::GlassIncisors, SpliceKind::BoilerHeart];
        let slot = (0..STOCK_SIZE)
            .find(|slot| {
                w.shop
                    .get(0)
                    .unwrap()
                    .slot(*slot)
                    .is_some_and(|k| !w.player.loadout.splices.contains(&k))
            })
            .expect("the shelf holds a splice the rat does not own");
        assert_eq!(
            deny_reason(&w, 0, ShopItem::Splice(slot)),
            Some(Deny::Sockets)
        );
        assert_eq!(
            buy(&mut w, 0, ShopItem::Splice(slot)),
            Purchase::Denied(Deny::Sockets)
        );
    }

    #[test]
    fn stitch_heals_once_per_hole_and_refuses_at_full_health() {
        let mut w = shop_world(9, 100);
        w.player.hp = w.player.max_hp() * 0.5;
        let max = w.player.max_hp();
        let before = w.player.hp;
        assert_eq!(
            buy(&mut w, 0, ShopItem::Stitch),
            Purchase::Bought {
                price: STITCH_PRICE
            }
        );
        let healed = w.player.hp - before;
        assert!((healed - max * STITCH_FRACTION).abs() < 0.5, "35% max HP");
        assert_eq!(w.gold, 100 - STITCH_PRICE);
        assert_eq!(
            buy(&mut w, 0, ShopItem::Stitch),
            Purchase::Denied(Deny::Stitched),
            "once per hole"
        );
        // A fresh hole allows another, and a full rat is refused with its own
        // reason.
        let mut w = shop_world(9, 100);
        w.player.hp = w.player.max_hp();
        assert_eq!(deny_reason(&w, 0, ShopItem::Stitch), Some(Deny::FullHp));
        // Blackboard Belly cuts the heal by a quarter.
        let mut w = shop_world(9, 100);
        w.player.loadout.splices = vec![SpliceKind::BlackboardBelly];
        w.player.loadout.rebuild_stats();
        w.player.hp = w.player.max_hp() * 0.5;
        let before = w.player.hp;
        buy(&mut w, 0, ShopItem::Stitch);
        let max = w.player.max_hp();
        assert!((w.player.hp - before - max * STITCH_FRACTION * 0.75).abs() < 0.5);
    }

    #[test]
    fn turning_the_shelf_brings_unseen_stock_and_costs_eight_then_sixteen() {
        let mut w = shop_world(13, 40);
        let first: Vec<Option<SpliceKind>> = (0..STOCK_SIZE)
            .map(|s| w.shop.get(0).unwrap().slot(s))
            .collect();
        assert_eq!(reroll_price(w.shop.get(0).unwrap()), Some(8));
        assert_eq!(
            buy(&mut w, 0, ShopItem::Reroll),
            Purchase::Bought { price: 8 }
        );
        let second: Vec<Option<SpliceKind>> = (0..STOCK_SIZE)
            .map(|s| w.shop.get(0).unwrap().slot(s))
            .collect();
        assert_ne!(first, second, "the shelf changed");
        for kind in second.iter().flatten() {
            assert!(
                !first.contains(&Some(*kind)),
                "{kind:?} was already on the shelf"
            );
        }
        assert_eq!(w.gold, 32);
        assert_eq!(
            buy(&mut w, 0, ShopItem::Reroll),
            Purchase::Bought { price: 16 }
        );
        assert_eq!(w.shop.get(0).unwrap().rerolls, MAX_REROLLS);
        assert_eq!(
            buy(&mut w, 0, ShopItem::Reroll),
            Purchase::Denied(Deny::NoTurns),
            "at most twice per hole"
        );
        // A sold slot is never replaced by the turn.
        let mut w = shop_world(14, 200);
        buy(&mut w, 0, ShopItem::Splice(0));
        let sold = w.shop.get(0).unwrap().slot(0).unwrap();
        buy(&mut w, 0, ShopItem::Reroll);
        let stock = w.shop.get(0).unwrap();
        assert!(stock.sold[0]);
        assert_eq!(stock.slot(0), Some(sold), "the sold hook stays sold");
    }

    #[test]
    fn molt_swaps_one_weapon_at_rank_one_and_only_once() {
        let mut w = shop_world(21, 100);
        {
            let lo = &mut w.player.loadout;
            lo.apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
            lo.weapons[0].level = 4;
            lo.weapons[0].evolved = true;
        }
        let owned = molt_owned(&w);
        assert_eq!(owned.len(), 2, "two weapons, no passives");
        assert!(owned_label(&w, owned[0]).contains("Lv4"));
        assert!(owned_label(&w, owned[0]).contains("EVOLVED"));
        let options = molt_options(&w, owned[0]);
        assert!(!options.is_empty());
        let pick = options
            .iter()
            .find(|k| matches!(k, NewKind::Weapon(_)))
            .copied()
            .expect("an unowned weapon");
        assert!(molt(&mut w, 0, owned[0], pick).is_ok());
        assert_eq!(w.gold, 100 - MOLT_PRICE);
        // Rank 1, and the partner that was evolved is gone with it.
        assert_eq!(w.player.loadout.weapons[0].level, 1);
        assert!(!w.player.loadout.weapons[0].evolved);
        assert_eq!(w.player.loadout.weapons.len(), 2);
        assert!(molt_deny(&w, 0) == Some(Deny::Molted));
        // Wrong category and unknown kinds are refused.
        assert!(molt(&mut w, 0, owned[0], NewKind::Passive(PassiveKind::Armor)).is_err());
    }

    #[test]
    fn molt_swaps_a_passive_and_recomputes_the_stats() {
        let mut w = shop_world(22, 100);
        w.player.loadout.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        w.player.loadout.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        w.player.xp = 0;
        let before_max = w.player.max_hp();
        let owned = molt_owned(&w);
        let passive = owned
            .iter()
            .find(|k| matches!(k, OwnedKind::Passive(_)))
            .copied()
            .expect("a passive");
        let pick = molt_options(&w, passive)
            .into_iter()
            .find(|k| matches!(k, NewKind::Passive(_)))
            .expect("an unowned passive");
        assert!(molt(&mut w, 0, passive, pick).is_ok());
        assert_eq!(w.player.loadout.passives.len(), 1);
        assert_eq!(w.player.loadout.passives[0].1, 1, "rank 1");
        assert!(
            w.player.max_hp() < before_max,
            "the forfeited Fat Reserves stacks are gone"
        );
        assert!(w.player.hp <= w.player.max_hp(), "HP clamped to the new max");
    }

    /// What `lo`'s numbers must be, derived without `rebuild_stats`: the
    /// passive stacks replayed onto fresh stats (`Loadout::from_parts`), then
    /// the splices. `Stats` holds only `f32`s, so the `Debug` text is a
    /// bit-exact comparison.
    fn from_scratch(lo: &Loadout) -> String {
        let mut fresh = Loadout::from_parts(lo.weapons.clone(), lo.passives.clone());
        apply_splices(&mut fresh.stats, &lo.splices);
        format!("{:?}", fresh.stats)
    }

    /// The splices compose with every passive's `Stats` hook, the slot caps
    /// and the evolutions: after a level-up pick, a splice purchase, a pick
    /// made while spliced, a Molt, a chest (an evolution, a free weapon rank
    /// and a free passive rank) and the campaign carry-over, the live stats
    /// are exactly a from-scratch rebuild.
    #[test]
    fn stats_always_equal_a_from_scratch_rebuild() {
        use crate::game::campaign::{self, keepables};
        use crate::game::loadout::{ChestReward, MAX_PASSIVES, MAX_WEAPONS};

        let all = PassiveKind::all();
        // Three racks of five cover all twelve passives; the first and the
        // last hold Sharp Teeth, Gnaw's evolution partner.
        let racks: [Vec<PassiveKind>; 3] = [
            all[0..5].to_vec(),
            all[5..10].to_vec(),
            vec![all[10], all[11], all[2], all[0], all[7]],
        ];
        for (n, rack) in racks.iter().enumerate() {
            let mut w = shop_world(40 + n as u64, 1000);
            let check = |w: &World, when: &str| {
                let lo = &w.player.loadout;
                assert_eq!(
                    format!("{:?}", lo.stats),
                    from_scratch(lo),
                    "rack {n}: stats drifted {when}"
                );
                assert!(lo.passives.len() <= MAX_PASSIVES && lo.weapons.len() <= MAX_WEAPONS);
                assert!(lo.splices.len() <= MAX_SPLICES);
            };
            for (i, &p) in rack.iter().enumerate() {
                for _ in 0..=(i % 3) {
                    w.player.loadout.apply(&Upgrade::Passive(p));
                }
            }
            check(&w, "after the level-up picks");

            assert!(matches!(buy(&mut w, 0, ShopItem::Splice(0)), Purchase::Bought { .. }));
            check(&w, "after the first splice");
            // A pick made while spliced (the live path replays the splices).
            w.player.loadout.apply(&Upgrade::Passive(rack[1]));
            check(&w, "after a pick made while spliced");
            assert!(matches!(buy(&mut w, 0, ShopItem::Splice(1)), Purchase::Bought { .. }));
            assert_eq!(w.player.loadout.splices.len(), 2);
            check(&w, "after the second splice");

            // Molt the last passive of the rack for a kind the rat does not own.
            let last = OwnedKind::Passive(w.player.loadout.passives.len() - 1);
            let pick = molt_options(&w, last)[0];
            assert!(molt(&mut w, 0, last, pick).is_ok());
            assert_eq!(w.player.loadout.passives.len(), MAX_PASSIVES, "a Molt is a swap");
            check(&w, "after a Molt");

            // Chests: an evolution when the partner is on the rack, then free
            // ranks (weapon first, then a passive stack) on the spliced rat.
            let gnaw = &mut w.player.loadout.weapons[0];
            gnaw.level = gnaw.kind.max_level();
            w.pending_chests = 1;
            let reward = w.claim_chest().expect("a chest");
            if w.player.loadout.stacks(PassiveKind::Damage) > 0 {
                assert!(matches!(reward, ChestReward::Evolve { .. }), "rack {n}: {reward:?}");
                assert!(w.player.loadout.weapons[0].evolved);
            }
            check(&w, "after a chest");
            let mut passive_ranks = 0;
            for _ in 0..40 {
                w.pending_chests = 1;
                if let Some(ChestReward::PassiveLevel { .. }) = w.claim_chest() {
                    passive_ranks += 1;
                }
                check(&w, "after a free rank");
            }
            assert!(passive_ranks > 0, "rack {n}: no chest ranked a passive");

            // Campaign carry-over: three kept entries, the splices ride along.
            let keep: Vec<_> = keepables(&w.player.loadout).into_iter().rev().take(3).collect();
            let carried = campaign::next_loadout(&w.player.loadout, &keep).expect("a valid keep");
            assert_eq!(carried.splices, w.player.loadout.splices, "splices last the campaign");
            assert_eq!(
                format!("{:?}", carried.stats),
                from_scratch(&carried),
                "rack {n}: the carried stats drifted"
            );
        }
    }

    /// Molt on the wave's edges: an evolved weapon is forfeited with its ranks
    /// and its evolution (and the list says so), and each of the four wave-3
    /// weapons can be taken in its place and forfeited again like any other.
    ///
    /// All twelve weapons evolve now (the last four rows landed with
    /// Downspout, Dead Letter, Carbon Copy and Ripcord), so "no evolution row"
    /// is no longer a case: the test pins that, and keeps exercising the swap
    /// against the four newest kinds.
    #[test]
    fn molt_handles_evolved_weapons_and_the_wave3_kinds_within_the_caps() {
        use crate::game::loadout::MAX_WEAPONS;
        use crate::game::weapon::Weapon;
        assert!(
            WeaponKind::all().into_iter().all(|k| k.evolution().is_some()),
            "every weapon has an evolution row"
        );
        let wave3 = [
            WeaponKind::AcidRain,
            WeaponKind::Hantavirus,
            WeaponKind::LaserEyes,
            WeaponKind::SuperDash,
        ];
        let rack_kinds: Vec<WeaponKind> = WeaponKind::all()
            .into_iter()
            .filter(|k| !wave3.contains(k))
            .take(MAX_WEAPONS)
            .collect();

        for (n, &new) in wave3.iter().enumerate() {
            // A full rack of five evolvable weapons, the first one evolved.
            let mut w = shop_world(60 + n as u64, 200);
            let rack: Vec<Weapon> = rack_kinds
                .iter()
                .copied()
                .map(|kind| {
                    let mut weapon = Weapon::new(kind);
                    weapon.level = kind.max_level();
                    weapon
                })
                .collect();
            w.player.loadout = Loadout::from_parts(rack, Vec::new());
            w.player.loadout.weapons[0].evolved = true;
            assert_eq!(w.player.loadout.weapons.len(), MAX_WEAPONS);

            let label = owned_label(&w, OwnedKind::Weapon(0));
            assert!(label.contains("Lv5") && label.contains("EVOLVED"), "{label}");
            // A full rack still swaps, and every unowned kind is on offer.
            let options = molt_options(&w, OwnedKind::Weapon(0));
            assert!(options.contains(&NewKind::Weapon(new)));
            assert!(molt(&mut w, 0, OwnedKind::Weapon(0), NewKind::Weapon(new)).is_ok());
            let lo = &w.player.loadout;
            assert_eq!(lo.weapons.len(), MAX_WEAPONS, "the rack did not grow");
            assert_eq!(lo.weapons[0].kind, new);
            assert_eq!(lo.weapons[0].level, 1);
            assert!(!lo.weapons[0].evolved, "the evolution was forfeited");
            assert_eq!(format!("{:?}", lo.stats), from_scratch(lo));

            // …and at the next hole the new weapon is forfeitable too.
            w.open_shop(1);
            let back = molt_options(&w, OwnedKind::Weapon(0))[0];
            assert!(molt(&mut w, 1, OwnedKind::Weapon(0), back).is_ok());
            assert_eq!(w.player.loadout.weapons.len(), MAX_WEAPONS);
            assert!(!w.player.loadout.has(new));
        }
    }

    #[test]
    fn molt_is_refused_with_its_own_reason() {
        let mut w = shop_world(23, 10);
        assert_eq!(molt_deny(&w, 0), Some(Deny::Gold));
        w.gold = 100;
        assert_eq!(molt_deny(&w, 0), None);
        w.player.loadout.weapons.clear();
        w.player.loadout.passives.clear();
        assert_eq!(
            molt_deny(&w, 0),
            Some(Deny::NoChoice),
            "nothing to forfeit: no Molt to offer"
        );
        assert!(molt_owned(&w).is_empty());
    }

    #[test]
    fn purchases_are_deterministic_for_a_seed() {
        let mut a = shop_world(31, 120);
        let mut b = shop_world(31, 120);
        for slot in [0usize, 1, 0, 2] {
            let pa = buy(&mut a, 0, ShopItem::Splice(slot));
            let pb = buy(&mut b, 0, ShopItem::Splice(slot));
            assert_eq!(pa, pb);
        }
        buy(&mut a, 0, ShopItem::Reroll);
        buy(&mut b, 0, ShopItem::Reroll);
        assert_eq!(a.player.loadout.splices, b.player.loadout.splices);
        assert_eq!(a.gold, b.gold);
        assert_eq!(a.player.loadout.stats.max_hp, b.player.loadout.stats.max_hp);
        assert_eq!(a.player.loadout.stats.damage_mult, b.player.loadout.stats.damage_mult);
        assert_eq!(
            (0..STOCK_SIZE)
                .map(|s| a.shop.get(0).unwrap().slot(s))
                .collect::<Vec<_>>(),
            (0..STOCK_SIZE)
                .map(|s| b.shop.get(0).unwrap().slot(s))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_shop_draws_no_simulation_rng() {
        let mut w = shop_world(41, 200);
        let mut twin = world(41);
        let draws = |w: &mut World| {
            use rand::RngExt;
            w.rng.random::<u64>()
        };
        assert_eq!(draws(&mut w), draws(&mut twin));
        buy(&mut w, 0, ShopItem::Splice(0));
        buy(&mut w, 0, ShopItem::Stitch);
        buy(&mut w, 0, ShopItem::Reroll);
        assert_eq!(
            draws(&mut w),
            draws(&mut twin),
            "buying draws nothing from the run's rng"
        );
    }

    #[test]
    fn splices_carry_through_the_keep_three_screen() {
        use crate::game::campaign::{keepables, next_loadout};
        let mut lo = Loadout::starting();
        lo.apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
        lo.splices = vec![SpliceKind::GlassIncisors, SpliceKind::BoilerHeart];
        lo.rebuild_stats();
        let purchased_damage = lo.stats.damage_mult;
        // Keep only one weapon: the splices stay anyway (the racks do not own
        // them).
        let keep = keepables(&lo);
        let chosen: Vec<_> = keep
            .iter()
            .copied()
            .filter(|k| k.title().contains("Gnaw"))
            .collect();
        let carried = next_loadout(&lo, &chosen).expect("a valid keep");
        assert_eq!(carried.splices, lo.splices);
        assert!(
            (carried.stats.damage_mult - purchased_damage).abs() < 1e-5,
            "the carried loadout has the same splice multipliers"
        );
        assert_eq!(carried.stats.max_hp, lo.stats.max_hp);
        // And a run that owned none carries none (no behaviour change).
        let plain = Loadout::starting();
        let carried = next_loadout(&plain, &keepables(&plain)[..]).expect("keep");
        assert!(carried.splices.is_empty());
        assert_eq!(carried.stats.damage_mult, 1.0);
    }

    #[test]
    fn the_gold_table_is_the_designs() {
        assert_eq!(gold_row(EnemyKind::Skitterer), (0.08, 1));
        assert_eq!(gold_row(EnemyKind::Bat), (0.10, 1));
        assert_eq!(gold_row(EnemyKind::Cat), (0.18, 2));
        assert_eq!(gold_row(EnemyKind::Spitter), (0.20, 2));
        assert_eq!(gold_row(EnemyKind::Brute), (0.45, 3));
        assert_eq!(gold_row(EnemyKind::Pillbug), (0.18, 2));
        assert_eq!(gold_row(EnemyKind::BellFlea), (0.14, 2));
        assert_eq!(gold_row(EnemyKind::WickMoth), (0.22, 2));
        assert_eq!(gold_row(EnemyKind::Porter), (0.40, 3));
        assert_eq!(gold_row(EnemyKind::Turnkey), (0.32, 3));
        assert_eq!(gold_row(EnemyKind::Boss), (1.0, BOSS_GOLD));
        // The boss always pays 30; an elite always pays 5, whatever its kind.
        assert_eq!(gold_for(7, 1, EnemyKind::Boss, false), BOSS_GOLD);
        assert_eq!(gold_for(7, 1, EnemyKind::Skitterer, true), ELITE_GOLD);
        assert_eq!(gold_for(7, 1, EnemyKind::Brute, true), ELITE_GOLD);
        // A kind's roll is deterministic for (floor seed, SpawnId) and never
        // leaves 0 or its value.
        for spawn in 1..400u64 {
            let paid = gold_for(7, spawn, EnemyKind::Brute, false);
            assert!(paid == 0 || paid == 3, "brute paid {paid}");
            assert_eq!(paid, gold_for(7, spawn, EnemyKind::Brute, false));
        }
        // The chances come out roughly right over many spawn ids.
        let hits = (1..4000u64)
            .filter(|id| gold_for(7, *id, EnemyKind::Brute, false) > 0)
            .count();
        let rate = hits as f32 / 3999.0;
        assert!((rate - 0.45).abs() < 0.05, "brute rate {rate}");
        let hits = (1..4000u64)
            .filter(|id| gold_for(7, *id, EnemyKind::Skitterer, false) > 0)
            .count();
        let rate = hits as f32 / 3999.0;
        assert!((rate - 0.08).abs() < 0.03, "skitterer rate {rate}");
    }

    #[test]
    fn the_shop_is_closed_until_it_is_opened() {
        let mut w = world(3);
        assert_eq!(deny_reason(&w, 0, ShopItem::Stitch), Some(Deny::Closed));
        assert_eq!(buy(&mut w, 0, ShopItem::Stitch), Purchase::Denied(Deny::Closed));
        w.open_shop(0);
        assert!(w.shop.get(0).is_some());
        // Leaving drops the flag and starts the 0.8 s grace.
        w.pending_shop = true;
        w.leave_shop();
        assert!(!w.pending_shop);
        assert!(
            (w.shop_runtime().grace - crate::game::ecs::systems::shop::EXIT_GRACE_SECS).abs()
                < 1e-6
        );
    }

    /// Two runs of one seed that shop the same way stay identical tick for
    /// tick: the shelf, the purchase and the stat rebuild are all pure.
    #[test]
    fn twin_runs_with_purchases_agree() {
        const DT: f32 = 1.0 / 60.0;
        let mut a = world(51);
        let mut b = world(51);
        a.gold = 300;
        b.gold = 300;
        let dir = crate::math::Vec2::new(1.0, 0.0);
        for tick in 0..900 {
            if tick == 100 {
                a.open_shop(0);
                b.open_shop(0);
                assert_eq!(
                    buy(&mut a, 0, ShopItem::Splice(0)),
                    buy(&mut b, 0, ShopItem::Splice(0))
                );
                a.leave_shop();
                b.leave_shop();
            }
            if tick == 300 {
                a.player.hp = a.player.max_hp() * 0.5;
                b.player.hp = b.player.max_hp() * 0.5;
                assert_eq!(
                    buy(&mut a, 0, ShopItem::Stitch),
                    buy(&mut b, 0, ShopItem::Stitch)
                );
                a.leave_shop();
                b.leave_shop();
            }
            a.update(DT, dir, false);
            b.update(DT, dir, false);
            assert_eq!(a.player.hp, b.player.hp, "tick {tick}: hp");
            assert_eq!(a.elapsed, b.elapsed);
            assert_eq!(a.kills, b.kills, "tick {tick}: kills");
            assert_eq!(a.score, b.score);
            assert_eq!(a.gold, b.gold);
            assert_eq!(a.enemy_count(), b.enemy_count(), "tick {tick}: crowd");
            assert_eq!(
                a.player.loadout.stats.max_hp,
                b.player.loadout.stats.max_hp
            );
            assert_eq!(
                a.player.loadout.stats.damage_mult,
                b.player.loadout.stats.damage_mult
            );
        }
        assert_eq!(a.player.loadout.splices, b.player.loadout.splices);
        assert_eq!(a.player.loadout.splices.len(), 1, "one splice each");
        use rand::RngExt as _;
        assert_eq!(a.rng.random::<u64>(), b.rng.random::<u64>());
    }

    #[test]
    fn the_row_list_covers_every_item() {
        let mut seen = Vec::new();
        for row in 0..ROW_COUNT {
            seen.push(row_item(row));
        }
        assert!(seen.contains(&ShopItem::Splice(0)));
        assert!(seen.contains(&ShopItem::Splice(1)));
        assert!(seen.contains(&ShopItem::Splice(2)));
        assert!(seen.contains(&ShopItem::Stitch));
        assert!(seen.contains(&ShopItem::Reroll));
        assert!(seen.contains(&ShopItem::Molt));
        assert!(seen.contains(&ShopItem::Leave));
        // `loadout::MAX_PASSIVES`-style sanity: the sink keeps the shelf usable.
        let _ = loadout::MAX_WEAPONS;
    }
}
