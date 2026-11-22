//! **The nest**: permanent upgrades bought with banked gold between runs.
//!
//! The owner's ask: *"There should be stats that can be upgraded over time with
//! gold that carry from one run to the next. max weapon/passive slots, move
//! speed, pickup range, fire rate, and any more that make sense. We can start at
//! max 4 of each active passive and upgrade it all the way to 8 of each
//! eventually."*
//!
//! Three things live here and nothing else does:
//!
//! 1. [`Track`] — the table of upgrade tracks: levels, what each level does, and
//!    its price. **Every gameplay number and every price in the whole
//!    meta-progression is in this one table** (see the economy note below), so a
//!    balance pass retunes it in one place.
//! 2. [`MetaProfile`] — the persisted profile: the bank and one level per track.
//!    It is a field on [`crate::scores::ScoreTable`], so it lands in the same
//!    save file as the scores and survives an old file (a missing field loads as
//!    a fresh nest: bank 0, every track 0).
//! 3. [`RunModifiers`] — the profile compiled into the handful of numbers a run
//!    reads, plus [`apply`], which folds them into [`Stats`] **before** the
//!    passives and the gene splices ([`crate::game::loadout::Loadout::rebuild_stats`]).
//!
//! Plus the banking rule ([`bank`]): what a finished run pays into the bank and
//! the tally the incident report prints.
//!
//! ## The economy (derivation — retune here)
//!
//! Income: `game::shop` pays a coin per eligible kill from a chance × value
//! table (0.08 × 1 for a skitterer … 0.45 × 3 for a brute), plus
//! [`crate::game::shop::ELITE_GOLD`] (5), [`crate::game::shop::MINIBOSS_GOLD`]
//! (12) and [`crate::game::shop::BOSS_GOLD`] (30). A floor of the fifteen-minute
//! timeline is a few thousand kills, so a full three-floor campaign that spends
//! nothing banks in the low thousands — but a run that *uses* the gene-splice
//! shelf (splices are 24-38 g each, a shelf visit can take 100+) and dies early
//! banks far less.
//!
//! **The assumption the prices are set against: a typical finished run banks
//! 200-300 gold** (a campaign that reaches the Lab and spends a little, or a
//! good practice floor). [`crate::game::shop`]'s gold income is being retuned by
//! the balance pass as this lands; if the measured haul per run is `K` times
//! 250, scale every price in [`Track::prices`] by `K` and nothing else here
//! changes.
//!
//! Against 250 gold a run:
//!
//! * cheapest first step 40 g → **a first purchase after one or two runs**;
//! * a fifth weapon *and* a fifth passive slot is 60 + 60 → by run one, and both
//!   at six slots (60 + 200 each side) by about run five to eight;
//! * the whole table is **18 070 g** → about **72 runs** at 250 a run (90 at
//!   200, 60 at 300), with the last weapon/passive slot (1 200 g) and the revive
//!   (1 500 g) as the long-term goals the owner asked for.
//!
//! ## Effects at full (a maxed profile is strong, not a win button)
//!
//! | track | max | effect at max |
//! |---|---|---|
//! | Weapon / passive slots | 4 | 4 → 8 slots each side |
//! | Move speed | 6 | +18 % (58 → 68 px/s) |
//! | Pickup range | 5 | +40 % (22 → 31 px) |
//! | Fire rate | 6 | +12 % |
//! | Max HP | 6 | +48 (130 → 178) |
//! | Regeneration | 5 | +0.4 HP/s (0.4 → 0.8) |
//! | Armour | 4 | +2 flat |
//! | Damage | 6 | +15 % |
//! | Dash cooldown | 5 | -0.25 s (2.0 → 1.75) |
//! | XP gain | 5 | +20 % |
//! | Gold gain | 5 | +25 % |
//! | Luck (rare finds) | 4 | +60 % rare-pickup chance |
//! | Carry-over | 2 | keep 5 powerups between floors (was 3) |
//! | Revive | 1 | one revive per run |
//!
//! Every one of those is deliberately *smaller* than one in-run passive stack
//! (Sharp Teeth alone is +15 % per stack to a cap of +120 %), so a fresh profile
//! is still a real run and the nest is an edge, not the game.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::game::campaign::KEEP_COUNT;
use crate::game::loadout::{SlotCaps, Stats, MIN_SLOTS};

/// How many upgrade tracks the nest has.
pub const TRACK_COUNT: usize = 15;

/// How many tracks are "one level each" specials (the revive).
pub const MAX_CARRY_BONUS: u8 = 2;

// ============================================================================
// The table
// ============================================================================

/// One permanent upgrade track.
///
/// The order is the screen's order and the order of [`Track::ALL`]; it is
/// append-only so a new track never renumbers the saved profile (the profile is
/// keyed by [`Track::key`] strings anyway, but the display order is this one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Track {
    #[default]
    WeaponSlots,
    PassiveSlots,
    MoveSpeed,
    PickupRange,
    FireRate,
    MaxHp,
    Regen,
    Armor,
    Damage,
    DashCooldown,
    XpGain,
    GoldGain,
    Luck,
    CarryOver,
    Revive,
}

impl Track {
    /// Every track, in screen order.
    pub const ALL: [Track; TRACK_COUNT] = [
        Track::WeaponSlots,
        Track::PassiveSlots,
        Track::MoveSpeed,
        Track::PickupRange,
        Track::FireRate,
        Track::MaxHp,
        Track::Regen,
        Track::Armor,
        Track::Damage,
        Track::DashCooldown,
        Track::XpGain,
        Track::GoldGain,
        Track::Luck,
        Track::CarryOver,
        Track::Revive,
    ];

    /// Index in [`Track::ALL`] (the screen's cursor space).
    pub fn index(self) -> usize {
        match self {
            Track::WeaponSlots => 0,
            Track::PassiveSlots => 1,
            Track::MoveSpeed => 2,
            Track::PickupRange => 3,
            Track::FireRate => 4,
            Track::MaxHp => 5,
            Track::Regen => 6,
            Track::Armor => 7,
            Track::Damage => 8,
            Track::DashCooldown => 9,
            Track::XpGain => 10,
            Track::GoldGain => 11,
            Track::Luck => 12,
            Track::CarryOver => 13,
            Track::Revive => 14,
        }
    }

    /// The track at `index` in [`Track::ALL`], if any.
    pub fn from_index(index: usize) -> Option<Track> {
        Track::ALL.get(index).copied()
    }

    /// The profile's key for this track (what the save file stores).
    pub fn key(self) -> &'static str {
        match self {
            Track::WeaponSlots => "weapon_slots",
            Track::PassiveSlots => "passive_slots",
            Track::MoveSpeed => "move_speed",
            Track::PickupRange => "pickup_range",
            Track::FireRate => "fire_rate",
            Track::MaxHp => "max_hp",
            Track::Regen => "regen",
            Track::Armor => "armor",
            Track::Damage => "damage",
            Track::DashCooldown => "dash_cooldown",
            Track::XpGain => "xp_gain",
            Track::GoldGain => "gold_gain",
            Track::Luck => "luck",
            Track::CarryOver => "carry_over",
            Track::Revive => "revive",
        }
    }

    /// The track a profile key names, if any (unknown keys are ignored on load).
    pub fn from_key(key: &str) -> Option<Track> {
        Track::ALL.into_iter().find(|t| t.key() == key)
    }

    /// The track's name on the nest screen ("the nest" is Subject R-47's stash).
    pub fn name(self) -> &'static str {
        match self {
            Track::WeaponSlots => "WEAPON SLOTS",
            Track::PassiveSlots => "PASSIVE SLOTS",
            Track::MoveSpeed => "QUICK PAWS",
            Track::PickupRange => "LONG WHISKERS",
            Track::FireRate => "TWITCH FINGER",
            Track::MaxHp => "FAT RESERVES",
            Track::Regen => "SLOW BLEED",
            Track::Armor => "SCAR TISSUE",
            Track::Damage => "FILED TEETH",
            Track::DashCooldown => "FAST HEEL",
            Track::XpGain => "SECOND SCENT",
            Track::GoldGain => "HOARD GLAND",
            Track::Luck => "FOUR-LEAF LUCK",
            Track::CarryOver => "DEEP POCKETS",
            Track::Revive => "NINTH LIFE",
        }
    }

    /// One line of what the track does.
    pub fn desc(self) -> &'static str {
        match self {
            Track::WeaponSlots => "One more weapon on the rack every level.",
            Track::PassiveSlots => "One more passive on the rack every level.",
            Track::MoveSpeed => "+3% base move speed per level.",
            Track::PickupRange => "+8% pickup radius per level.",
            Track::FireRate => "+2% base fire rate per level.",
            Track::MaxHp => "+8 max HP per level.",
            Track::Regen => "+0.08 HP/s regeneration per level.",
            Track::Armor => "+0.5 armour per level (-0.5 damage a hit).",
            Track::Damage => "+2.5% weapon damage per level.",
            Track::DashCooldown => "-0.05s dash cooldown per level.",
            Track::XpGain => "+4% gem XP per level.",
            Track::GoldGain => "+5% gold from every coin per level.",
            Track::Luck => "+15% rare-pickup chance per level.",
            Track::CarryOver => "Keep one more powerup between floors.",
            Track::Revive => "One revive a run: get up at half health.",
        }
    }

    /// The rat's voice for the track (the nest is what R-47 has learned).
    pub fn flavor(self) -> &'static str {
        match self {
            Track::WeaponSlots => "A wider rack, welded on in the dark.",
            Track::PassiveSlots => "Room to grow another gland.",
            Track::MoveSpeed => "Juice from a can with no label.",
            Track::PickupRange => "The whole floor smells like cheese.",
            Track::FireRate => "Teeth chatter when you're this fast.",
            Track::MaxHp => "Winter is always coming. Stash more.",
            Track::Regen => "Sleep in shifts. Heal in shifts.",
            Track::Armor => "Every scar teaches the skin.",
            Track::Damage => "Sharpened on the same rebar, longer.",
            Track::DashCooldown => "The floor is lava. Move.",
            Track::XpGain => "You know where the crumbs fall.",
            Track::GoldGain => "A pocket inside a pocket.",
            Track::Luck => "Clover tastes like nothing. Keep it.",
            Track::CarryOver => "A nest with more than one mouth to it.",
            Track::Revive => "Not today, cat. Not this one.",
        }
    }

    /// How many levels the track has.
    pub fn max_level(self) -> u8 {
        match self {
            Track::WeaponSlots => 4,
            Track::PassiveSlots => 4,
            Track::MoveSpeed => 6,
            Track::PickupRange => 5,
            Track::FireRate => 6,
            Track::MaxHp => 6,
            Track::Regen => 5,
            Track::Armor => 4,
            Track::Damage => 6,
            Track::DashCooldown => 5,
            Track::XpGain => 5,
            Track::GoldGain => 5,
            Track::Luck => 4,
            Track::CarryOver => MAX_CARRY_BONUS,
            Track::Revive => 1,
        }
    }

    /// The prices, one per level (index 0 buys level 1).
    ///
    /// Rising per track, rising **faster** on the two slot tracks: the fifth
    /// slot of each kind is the first thing a fresh nest buys, and the eighth is
    /// a long-term goal (see the module economy note for the derivation).
    fn prices(self) -> &'static [u32] {
        match self {
            Track::WeaponSlots => &[60, 200, 520, 1_200],
            Track::PassiveSlots => &[60, 200, 520, 1_200],
            Track::MoveSpeed => &[40, 70, 120, 190, 280, 400],
            Track::PickupRange => &[40, 70, 120, 190, 280],
            Track::FireRate => &[60, 100, 170, 260, 380, 540],
            Track::MaxHp => &[50, 85, 140, 215, 310, 430],
            Track::Regen => &[55, 95, 160, 250, 360],
            Track::Armor => &[70, 130, 220, 340],
            Track::Damage => &[60, 100, 170, 260, 380, 540],
            Track::DashCooldown => &[50, 85, 140, 215, 310],
            Track::XpGain => &[60, 100, 170, 260, 380],
            Track::GoldGain => &[80, 130, 200, 290, 400],
            Track::Luck => &[70, 120, 200, 320],
            Track::CarryOver => &[400, 900],
            Track::Revive => &[1_500],
        }
    }

    /// What buying the **next** level costs, or `None` when the track is maxed.
    ///
    /// `level` is the level already owned, so `price(0)` is the first purchase.
    pub fn price(self, level: u8) -> Option<u32> {
        let level = level.min(self.max_level());
        if level >= self.max_level() {
            return None;
        }
        self.prices().get(level as usize).copied()
    }

    /// The track's total cost from nothing to [`Track::max_level`].
    pub fn total_cost(self) -> u32 {
        self.prices().iter().sum()
    }

    /// What `level` levels of this track gives, as the screen prints it.
    ///
    /// A `String` because these are display lines on a menu (never the
    /// simulation), exactly like the level-up card text.
    pub fn value_text(self, level: u8) -> String {
        let level = level.min(self.max_level());
        match self {
            Track::WeaponSlots => format!("{} weapon slots", MIN_SLOTS + level as usize),
            Track::PassiveSlots => format!("{} passive slots", MIN_SLOTS + level as usize),
            Track::MoveSpeed => format!("+{}% move", u32::from(level) * 3),
            Track::PickupRange => format!("+{}% pickup", u32::from(level) * 8),
            Track::FireRate => format!("+{}% fire rate", u32::from(level) * 2),
            Track::MaxHp => format!("+{} HP", u32::from(level) * 8),
            Track::Regen => format!("+{:.1} HP/s", f32::from(level) * 0.08),
            Track::Armor => format!("-{:.1} dmg/hit", f32::from(level) * 0.5),
            Track::Damage => format!("+{:.1}% damage", f32::from(level) * 2.5),
            Track::DashCooldown => format!("-{:.2}s dash", f32::from(level) * 0.05),
            Track::XpGain => format!("+{}% XP", u32::from(level) * 4),
            Track::GoldGain => format!("+{}% gold", u32::from(level) * 5),
            Track::Luck => format!("+{}% rare finds", u32::from(level) * 15),
            Track::CarryOver => format!("keep {} between floors", KEEP_COUNT + level as usize),
            Track::Revive => {
                if level == 0 {
                    "no revive".to_string()
                } else {
                    "one revive a run".to_string()
                }
            }
        }
    }
}

// ============================================================================
// The profile
// ============================================================================

/// The persisted nest: the bank and one level per track.
///
/// Serialised as `{"bank": 137, "levels": {"weapon_slots": 1, "move_speed": 3}}`
/// — a sparse map keyed by [`Track::key`], so a file written by an older build
/// (no `meta` field at all) loads as a fresh nest, and a file written by a
/// *newer* build with a track this build does not know keeps loading (unknown
/// keys are dropped by [`MetaProfile::normalize`], never a parse failure).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaProfile {
    /// Gold banked across runs, spendable on the nest screen only.
    #[serde(default)]
    pub bank: u32,
    /// Level owned per track, keyed by [`Track::key`]. A missing key is level 0
    /// and is not written back, so a fresh nest saves as `{}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub levels: BTreeMap<String, u8>,
}

impl MetaProfile {
    /// How many levels of `track` are owned (clamped to its maximum).
    pub fn level(&self, track: Track) -> u8 {
        self.levels
            .get(track.key())
            .copied()
            .unwrap_or(0)
            .min(track.max_level())
    }

    /// Set `track`'s level (clamped). Level 0 removes the key, so a wiped
    /// profile saves exactly like a fresh one.
    pub fn set_level(&mut self, track: Track, level: u8) {
        let level = level.min(track.max_level());
        if level == 0 {
            self.levels.remove(track.key());
        } else {
            self.levels.insert(track.key().to_string(), level);
        }
    }

    /// Is `track` at its maximum?
    pub fn is_maxed(&self, track: Track) -> bool {
        self.level(track) >= track.max_level()
    }

    /// What buying the next level of `track` costs (`None` when maxed).
    pub fn next_price(&self, track: Track) -> Option<u32> {
        track.price(self.level(track))
    }

    /// Can the bank afford the next level of `track`?
    pub fn can_buy(&self, track: Track) -> bool {
        self.next_price(track).is_some_and(|p| self.bank >= p)
    }

    /// Buy one level of `track`. Returns the price paid, or `None` when the
    /// track is maxed or the bank is short (nothing changes then).
    pub fn buy(&mut self, track: Track) -> Option<u32> {
        let price = self.next_price(track)?;
        if self.bank < price {
            return None;
        }
        self.bank -= price;
        self.set_level(track, self.level(track) + 1);
        Some(price)
    }

    /// Every level owned, added up (the screen's "N upgrades" badge and the
    /// HUD's nest chip).
    pub fn total_levels(&self) -> u32 {
        Track::ALL
            .into_iter()
            .map(|t| u32::from(self.level(t)))
            .sum()
    }

    /// Gold spent on every level owned — exactly what a refund hands back.
    pub fn spent_total(&self) -> u32 {
        Track::ALL
            .into_iter()
            .map(|track| {
                (0..self.level(track))
                    .filter_map(|lvl| track.price(lvl))
                    .sum::<u32>()
            })
            .sum()
    }

    /// **Refund all**: wipe every track and give the gold back.
    ///
    /// Free (no fee, no penalty) and stated on the screen: the nest is meant to
    /// be experimented with, and the refund is exactly the sum of the prices
    /// paid, so a respec can never mint or destroy gold. Returns the refund.
    pub fn refund_all(&mut self) -> u32 {
        let paid = self.spent_total();
        self.levels.clear();
        self.bank = self.bank.saturating_add(paid);
        paid
    }

    /// Drop unknown keys, clamp levels into range and strip level-0 entries.
    /// Called on load (a hand-edited file cannot fabricate an out-of-range
    /// track) and after a wipe.
    pub fn normalize(&mut self) {
        let mut clean: BTreeMap<String, u8> = BTreeMap::new();
        for (key, level) in std::mem::take(&mut self.levels) {
            let Some(track) = Track::from_key(&key) else {
                continue;
            };
            let level = level.min(track.max_level());
            if level > 0 {
                clean.insert(track.key().to_string(), level);
            }
        }
        self.levels = clean;
    }

    /// A mid-progression profile: every track halfway, bank 0. The batch's
    /// `--meta mid` measures the game at the stage most players are in.
    pub fn mid() -> MetaProfile {
        let mut profile = MetaProfile::default();
        for track in Track::ALL {
            profile.set_level(track, track.max_level() / 2);
        }
        profile
    }

    /// A fully upgraded profile (every track at its cap), bank 0.
    pub fn maxed() -> MetaProfile {
        let mut profile = MetaProfile::default();
        for track in Track::ALL {
            profile.set_level(track, track.max_level());
        }
        profile
    }
}

// ============================================================================
// The compiled modifiers
// ============================================================================

/// Smallest dash cooldown a profile may push the base down to (the in-run
/// passives can take it lower; this is only the floor for the meta step).
pub const MIN_DASH_CD: f32 = 0.6;

/// A profile compiled into the numbers a run reads at start.
///
/// [`RunModifiers::NONE`] is the neutral value every test, the determinism
/// harness and the bot batch use unless they ask for something else: every
/// multiplier 1.0, every addition 0.0, [`SlotCaps::DEFAULT`] (4 + 4) and no
/// revive / carry-over bonus. Applying it is bit-exact (multiplying by 1.0 and
/// adding 0.0 cannot move an `f32`), so a run with no profile computes exactly
/// the numbers it computed before the nest existed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunModifiers {
    /// Rack caps in force for the run (`4..=8` each side).
    pub caps: SlotCaps,
    /// Multiplier on the base move speed.
    pub move_speed: f32,
    /// Multiplier on the base pickup radius.
    pub magnet: f32,
    /// Multiplier on the base fire rate.
    pub fire_rate: f32,
    /// Multiplier on the base weapon damage.
    pub damage: f32,
    /// Multiplier on the base gem XP.
    pub xp: f32,
    /// Multiplier on every coin's value.
    pub gold: f32,
    /// Multiplier on the rare-pickup drop chance.
    pub luck: f32,
    /// **Added** to the base max HP.
    pub max_hp: f32,
    /// **Added** to the base regeneration (HP/s).
    pub regen: f32,
    /// **Added** to the base armour (flat damage reduction).
    pub armor: f32,
    /// **Subtracted** from the base dash cooldown (seconds).
    pub dash_cd: f32,
    /// Extra keep slots between campaign floors.
    pub carry_bonus: u8,
    /// Revives per run.
    pub revives: u8,
}

impl Default for RunModifiers {
    fn default() -> Self {
        RunModifiers::NONE
    }
}

impl RunModifiers {
    /// The neutral value: a fresh profile.
    pub const NONE: RunModifiers = RunModifiers {
        caps: SlotCaps::DEFAULT,
        move_speed: 1.0,
        magnet: 1.0,
        fire_rate: 1.0,
        damage: 1.0,
        xp: 1.0,
        gold: 1.0,
        luck: 1.0,
        max_hp: 0.0,
        regen: 0.0,
        armor: 0.0,
        dash_cd: 0.0,
        carry_bonus: 0,
        revives: 0,
    };

    /// Rack caps only (the campaign carry-over / `--caps` measurement path).
    pub fn with_caps(caps: SlotCaps) -> RunModifiers {
        RunModifiers {
            caps,
            ..RunModifiers::NONE
        }
    }

    /// Compile a profile into the run's modifiers.
    pub fn from_profile(profile: &MetaProfile) -> RunModifiers {
        let level = |track: Track| u32::from(profile.level(track));
        let (weapons, passives) = (level(Track::WeaponSlots), level(Track::PassiveSlots));
        RunModifiers {
            caps: SlotCaps::new(
                MIN_SLOTS + weapons as usize,
                MIN_SLOTS + passives as usize,
            ),
            move_speed: 1.0 + 0.03 * level(Track::MoveSpeed) as f32,
            magnet: 1.0 + 0.08 * level(Track::PickupRange) as f32,
            fire_rate: 1.0 + 0.02 * level(Track::FireRate) as f32,
            damage: 1.0 + 0.025 * level(Track::Damage) as f32,
            xp: 1.0 + 0.04 * level(Track::XpGain) as f32,
            gold: 1.0 + 0.05 * level(Track::GoldGain) as f32,
            luck: 1.0 + 0.15 * level(Track::Luck) as f32,
            max_hp: 8.0 * level(Track::MaxHp) as f32,
            regen: 0.08 * level(Track::Regen) as f32,
            armor: 0.5 * level(Track::Armor) as f32,
            dash_cd: 0.05 * level(Track::DashCooldown) as f32,
            carry_bonus: profile.level(Track::CarryOver),
            revives: profile.level(Track::Revive),
        }
    }

    /// Is this the neutral value (a fresh profile)? The HUD's nest chip and the
    /// start-of-floor callout are silent for it.
    pub fn is_neutral(&self) -> bool {
        *self == RunModifiers::NONE
    }

    /// How many tracks the profile upgraded at all (the HUD chip's number).
    pub fn tracks_owned(&self) -> u32 {
        let mut n = 0;
        n += u32::from(self.caps.weapons > MIN_SLOTS);
        n += u32::from(self.caps.passives > MIN_SLOTS);
        for value in [self.move_speed, self.magnet, self.fire_rate, self.damage, self.xp, self.gold, self.luck] {
            n += u32::from(value > 1.0);
        }
        for value in [self.max_hp, self.regen, self.armor, self.dash_cd] {
            n += u32::from(value > 0.0);
        }
        n += u32::from(self.carry_bonus > 0);
        n += u32::from(self.revives > 0);
        n
    }

    /// One-line summary for the HUD callout and the pause dossier:
    /// `"8+8 slots · +18% move · +15% dmg · +1 revive"`. Empty for a neutral
    /// profile, so a fresh run prints nothing anywhere.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        self.summary_into(&mut out);
        out
    }

    /// [`RunModifiers::summary`] into a caller-owned buffer (the HUD keeps one
    /// for the whole run, so the pause line and the floor callout allocate
    /// nothing per frame).
    pub fn summary_into(&self, out: &mut String) {
        if self.is_neutral() {
            return;
        }
        let mut parts: Vec<String> = Vec::new();
        if self.caps != SlotCaps::DEFAULT {
            parts.push(format!("{}+{} slots", self.caps.weapons, self.caps.passives));
        }
        if self.move_speed > 1.0 {
            parts.push(format!("+{}% move", pct(self.move_speed)));
        }
        if self.magnet > 1.0 {
            parts.push(format!("+{}% pickup", pct(self.magnet)));
        }
        if self.fire_rate > 1.0 {
            parts.push(format!("+{}% rate", pct(self.fire_rate)));
        }
        if self.damage > 1.0 {
            parts.push(format!("+{}% dmg", pct(self.damage)));
        }
        if self.max_hp > 0.0 {
            parts.push(format!("+{} HP", self.max_hp.round() as u32));
        }
        if self.regen > 0.0 {
            parts.push(format!("+{:.1} regen", self.regen));
        }
        if self.armor > 0.0 {
            parts.push(format!("+{:.0} armour", self.armor));
        }
        if self.dash_cd > 0.0 {
            parts.push(format!("-{:.2}s dash", self.dash_cd));
        }
        if self.xp > 1.0 {
            parts.push(format!("+{}% XP", pct(self.xp)));
        }
        if self.gold > 1.0 {
            parts.push(format!("+{}% gold", pct(self.gold)));
        }
        if self.luck > 1.0 {
            parts.push(format!("+{}% luck", pct(self.luck)));
        }
        if self.carry_bonus > 0 {
            parts.push(format!("keep {}", KEEP_COUNT + self.carry_bonus as usize));
        }
        if self.revives > 0 {
            parts.push(format!("{} revive", self.revives));
        }
        out.push_str(&parts.join(" · "));
    }
}

/// A multiplier as whole percent (`1.18 -> 18`).
fn pct(mult: f32) -> u32 {
    ((mult - 1.0) * 100.0).round().max(0.0) as u32
}

/// Fold the permanent upgrades into `stats`: the **first** step of a rebuild
/// (`base → nest → passive stacks → gene splices`).
///
/// Every value is exact at [`RunModifiers::NONE`] (x1.0, +0.0), so a run with no
/// profile produces bit-identical stats to the build before the nest existed.
pub fn apply(stats: &mut Stats, mods: &RunModifiers) {
    stats.max_hp += mods.max_hp;
    stats.move_speed *= mods.move_speed;
    stats.magnet *= mods.magnet;
    stats.fire_rate_mult *= mods.fire_rate;
    stats.damage_mult *= mods.damage;
    stats.xp_mult *= mods.xp;
    stats.gold_mult *= mods.gold;
    stats.luck_mult *= mods.luck;
    stats.regen += mods.regen;
    stats.armor += mods.armor;
    stats.dash_cd = (stats.dash_cd - mods.dash_cd).max(MIN_DASH_CD);
}

// ============================================================================
// Banking a run
// ============================================================================

/// Banked for every campaign floor the run cleared.
pub const FLOOR_BONUS: u32 = 25;
/// Banked for every boss or named miniboss the run put down.
pub const APEX_BONUS: u32 = 6;
/// Banked for winning the run (the campaign cleared, or a practice floor).
pub const WIN_BONUS: u32 = 75;
/// A practice run banks at half rate: it is repeatable on any unlocked floor
/// and its boss is the only stake.
pub const PRACTICE_DIVISOR: u32 = 2;

/// What a finished run did, as the bank rule sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunEnd {
    /// Gold still in the purse when the run ended (this is what "unspent" means).
    pub purse: u32,
    /// Gold spent in the gene-splice shops during the run.
    pub spent: u32,
    /// Campaign floors cleared (`0..=3`).
    pub floors: u8,
    /// Bosses and named minibosses killed.
    pub apex: u8,
    /// Did the run win (campaign cleared, or a practice floor cleared)?
    pub won: bool,
    /// A single-floor practice run.
    pub practice: bool,
    /// The run used the developer console. Another worker is adding the flag;
    /// until it lands the app passes `false` (see `App::console_used`).
    pub console_used: bool,
}

impl RunEnd {
    /// Whether this run pays anything at all.
    pub fn pays(&self) -> bool {
        !self.console_used
    }
}

/// The end screen's banking tally: what the run raised, what it spent, what the
/// nest paid in bonuses, and the new balance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BankTally {
    /// Gold that came into the purse during the run (collected + chests + cards).
    pub raised: u32,
    /// Gold spent on gene splices during the run.
    pub spent: u32,
    /// The unspent purse this tally banked.
    pub purse: u32,
    /// `FLOOR_BONUS` × floors cleared.
    pub floor_bonus: u32,
    /// `APEX_BONUS` × bosses and minibosses killed.
    pub apex_bonus: u32,
    /// The win bonus, if the run won.
    pub win_bonus: u32,
    /// Was the practice half-rate applied?
    pub halved: bool,
    /// The run used the console and banked nothing.
    pub voided: bool,
    /// What this run put in the bank (the sum above).
    pub banked: u32,
    /// The bank **after** banking (the nest screen's "new total").
    pub total: u32,
}

impl BankTally {
    /// The one-line tally the incident report prints.
    pub fn line(&self) -> String {
        if self.voided {
            return "NEST  console used — nothing banked".to_string();
        }
        let mut s = format!(
            "NEST  purse {} + floors {} + apex {}",
            self.purse, self.floor_bonus, self.apex_bonus
        );
        if self.win_bonus > 0 {
            s.push_str(&format!(" + clear {}", self.win_bonus));
        }
        if self.halved {
            s.push_str(" (practice, half)");
        }
        s.push_str(&format!(" = +{} → {} banked", self.banked, self.total));
        s
    }
}

/// Bank a finished run into `profile` and return the tally.
///
/// The rule (the owner's "dying still pays, but spending is a real choice"):
///
/// * the run's **unspent** gold is banked — spending it on a splice is a real
///   choice against saving for something permanent;
/// * plus [`FLOOR_BONUS`] for every floor cleared, [`APEX_BONUS`] for every boss
///   or named miniboss killed, and [`WIN_BONUS`] for the clear — so a death in
///   the first floor still pays the run's purse and nothing else;
/// * a **practice** run banks at [`PRACTICE_DIVISOR`] (half, rounded down), the
///   getaway ride included: it is repeatable on any unlocked floor;
/// * a run that used the console banks nothing at all.
///
/// Saturating throughout: a corrupt saved purse can never wrap the bank.
pub fn bank(profile: &mut MetaProfile, end: &RunEnd) -> BankTally {
    let raised = end.purse.saturating_add(end.spent);
    if end.console_used {
        return BankTally {
            raised,
            spent: end.spent,
            purse: 0,
            voided: true,
            total: profile.bank,
            ..BankTally::default()
        };
    }
    let mut floor_bonus = FLOOR_BONUS.saturating_mul(u32::from(end.floors.min(3)));
    let mut apex_bonus = APEX_BONUS.saturating_mul(u32::from(end.apex));
    let mut win_bonus = if end.won { WIN_BONUS } else { 0 };
    let mut purse = end.purse;
    let halved = end.practice;
    if halved {
        floor_bonus /= PRACTICE_DIVISOR;
        apex_bonus /= PRACTICE_DIVISOR;
        win_bonus /= PRACTICE_DIVISOR;
        purse /= PRACTICE_DIVISOR;
    }
    let banked = purse
        .saturating_add(floor_bonus)
        .saturating_add(apex_bonus)
        .saturating_add(win_bonus);
    profile.bank = profile.bank.saturating_add(banked);
    BankTally {
        raised,
        spent: end.spent,
        purse,
        floor_bonus,
        apex_bonus,
        win_bonus,
        halved,
        voided: false,
        banked,
        total: profile.bank,
    }
}

// ============================================================================
// The batch's process-wide measurement override
// ============================================================================

const META_NONE: u32 = 0;
const META_MID: u32 = 1;
const META_MAX: u32 = 2;

/// Process-wide profile stage for the balance batch (`--meta none|mid|max`).
///
/// `0` = no override (the real game, and every test, constructs runs with
/// explicit modifiers). Only [`crate::autoplay::batch`] sets it, exactly like
/// [`crate::game::loadout::set_slot_caps`], and it is read **only** by
/// [`crate::autoplay::AutoRun::new`]: the bot batch never reads the player's
/// real profile and the real game never reads this.
static BATCH_META: AtomicU32 = AtomicU32::new(META_NONE);

/// Set the batch's profile stage. `None` clears the override.
pub fn set_batch_meta(stage: Option<MetaStage>) {
    BATCH_META.store(
        match stage {
            None => META_NONE,
            Some(MetaStage::Mid) => META_MID,
            Some(MetaStage::Max) => META_MAX,
        },
        Ordering::Relaxed,
    );
}

/// The batch's profile stage, if one is set.
pub fn batch_meta_stage() -> Option<MetaStage> {
    match BATCH_META.load(Ordering::Relaxed) {
        META_MID => Some(MetaStage::Mid),
        META_MAX => Some(MetaStage::Max),
        _ => None,
    }
}

/// The modifiers the batch asked for (`None` = the neutral fresh profile).
pub fn batch_modifiers() -> Option<RunModifiers> {
    batch_meta_stage().map(|stage| RunModifiers::from_profile(&stage.profile()))
}

/// The three stages `--meta` can measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaStage {
    /// A fresh profile: 4 + 4 slots, no bonuses.
    None,
    /// Every track halfway.
    Mid,
    /// Every track maxed.
    Max,
}

impl MetaStage {
    /// Parse the `--meta` value.
    pub fn parse(name: &str) -> Option<MetaStage> {
        match name {
            "none" => Some(MetaStage::None),
            "mid" => Some(MetaStage::Mid),
            "max" => Some(MetaStage::Max),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MetaStage::None => "none",
            MetaStage::Mid => "mid",
            MetaStage::Max => "max",
        }
    }

    /// The profile this stage stands for.
    pub fn profile(self) -> MetaProfile {
        match self {
            MetaStage::None => MetaProfile::default(),
            MetaStage::Mid => MetaProfile::mid(),
            MetaStage::Max => MetaProfile::maxed(),
        }
    }

    /// The run modifiers this stage stands for.
    pub fn modifiers(self) -> RunModifiers {
        if self == MetaStage::None {
            RunModifiers::NONE
        } else {
            RunModifiers::from_profile(&self.profile())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile with exactly one track bought to its cap (and nothing else).
    fn only(track: Track) -> MetaProfile {
        let mut profile = MetaProfile::default();
        profile.set_level(track, track.max_level());
        profile
    }

    fn end(purse: u32) -> RunEnd {
        RunEnd {
            purse,
            ..RunEnd::default()
        }
    }

    // ---- the table -------------------------------------------------------

    /// The table is the whole meta-progression: every track must be complete
    /// (a name, a description, the rat's voice, one price per level, rising).
    #[test]
    fn the_track_table_is_consistent_and_each_price_rises() {
        let mut keys = std::collections::BTreeSet::new();
        let mut total = 0u32;
        for (index, track) in Track::ALL.into_iter().enumerate() {
            assert_eq!(track.index(), index, "{track:?} index");
            assert_eq!(Track::from_index(index), Some(track));
            assert!(keys.insert(track.key()), "duplicate key for {track:?}");
            assert_eq!(Track::from_key(track.key()), Some(track));
            assert!(!track.name().is_empty(), "{track:?} has no name");
            assert!(!track.desc().is_empty(), "{track:?} has no description");
            assert!(!track.flavor().is_empty(), "{track:?} has no flavour");
            assert!(track.max_level() >= 1, "{track:?} cannot be bought");
            let prices = track.prices();
            assert_eq!(
                prices.len(),
                track.max_level() as usize,
                "{track:?} needs one price per level"
            );
            for pair in prices.windows(2) {
                assert!(pair[1] > pair[0], "{track:?} prices must rise: {pair:?}");
            }
            for level in 0..=track.max_level() {
                let expected = prices.get(level as usize).copied();
                assert_eq!(track.price(level), expected, "{track:?} at {level}");
            }
            assert_eq!(track.price(track.max_level()), None, "{track:?} is maxed");
            assert_eq!(track.total_cost(), prices.iter().sum::<u32>());
            total += track.total_cost();
        }
        assert_eq!(Track::from_index(Track::ALL.len()), None);
        assert_eq!(Track::from_key("no_such_track"), None);
        // The economy note in the module docs says the whole table is 18 070 g:
        // about 72 runs at 250 banked a run.
        assert_eq!(total, 18_070, "the table's total changed: update the note");
            assert!(
            (60..=100).contains(&(total / 250)),
            "the table should max out in 60-100 runs at 250 a run, got {}",
            total / 250
        );
    }

    /// The five tracks the owner named by hand are here, at the sizes asked for:
    /// both slot racks reach 8 from 4, and the last slot is the dearest step.
    #[test]
    fn the_owner_s_five_tracks_are_present_at_the_asked_for_sizes() {
        let slots = [
            (Track::WeaponSlots, Track::WeaponSlots),
            (Track::PassiveSlots, Track::PassiveSlots),
        ];
        for (track, _) in slots {
            assert_eq!(track.max_level(), 4, "4 -> 8 slots is four purchases");
            let prices = track.prices();
            assert!(prices[3] >= 2 * prices[2], "the last slot is the long goal");
        }
        for track in [
            Track::MoveSpeed,
            Track::PickupRange,
            Track::FireRate,
            Track::MaxHp,
            Track::Regen,
            Track::Armor,
            Track::Damage,
            Track::DashCooldown,
            Track::XpGain,
            Track::GoldGain,
            Track::Luck,
        ] {
            assert!(track.max_level() >= 4, "{track:?} is a shallow track");
        }
        assert_eq!(Track::Revive.max_level(), 1, "one revive, one purchase");
        assert_eq!(Track::CarryOver.max_level(), MAX_CARRY_BONUS);
    }

    #[test]
    fn value_text_reads_the_running_total_and_the_flavour_speaks_for_the_rat() {
        assert_eq!(Track::WeaponSlots.value_text(0), "4 weapon slots");
        assert_eq!(Track::WeaponSlots.value_text(4), "8 weapon slots");
        assert_eq!(Track::PassiveSlots.value_text(2), "6 passive slots");
        assert_eq!(Track::MoveSpeed.value_text(6), "+18% move");
        assert_eq!(Track::PickupRange.value_text(5), "+40% pickup");
        assert_eq!(Track::Damage.value_text(6), "+15.0% damage");
        assert_eq!(Track::MaxHp.value_text(6), "+48 HP");
        assert_eq!(Track::Regen.value_text(5), "+0.4 HP/s");
        assert_eq!(Track::Armor.value_text(4), "-2.0 dmg/hit");
        assert_eq!(Track::DashCooldown.value_text(5), "-0.25s dash");
        assert_eq!(Track::XpGain.value_text(5), "+20% XP");
        assert_eq!(Track::GoldGain.value_text(5), "+25% gold");
        assert_eq!(Track::Luck.value_text(4), "+60% rare finds");
        assert_eq!(
            Track::CarryOver.value_text(1),
            format!("keep {} between floors", KEEP_COUNT + 1)
        );
        assert_eq!(Track::Revive.value_text(0), "no revive");
        assert_eq!(Track::Revive.value_text(1), "one revive a run");
        // Clamped: a hand-edited level above the cap reads as the cap.
        assert_eq!(Track::Luck.value_text(200), Track::Luck.value_text(4));
    }

    // ---- the profile -----------------------------------------------------

    #[test]
    fn a_fresh_profile_is_empty_and_has_no_bonuses() {
        let profile = MetaProfile::default();
        assert_eq!(profile.bank, 0);
        assert_eq!(profile.total_levels(), 0);
        assert_eq!(profile.spent_total(), 0);
        for track in Track::ALL {
            assert_eq!(profile.level(track), 0);
            assert!(!profile.is_maxed(track));
            assert_eq!(profile.next_price(track), track.price(0));
            assert!(!profile.can_buy(track), "an empty bank affords nothing");
        }
        assert_eq!(RunModifiers::from_profile(&profile), RunModifiers::NONE);
        assert!(RunModifiers::from_profile(&profile).is_neutral());
        // A fresh nest serialises as an empty object, so an old score file and a
        // fresh save look the same.
        let json = serde_json::to_string(&profile).unwrap_or_default();
        assert_eq!(json, r#"{"bank":0}"#);
    }

    #[test]
    fn buy_spends_the_bank_and_refund_returns_exactly_what_was_paid() {
        let mut profile = MetaProfile::default();
        profile.bank = 1_000;
        assert_eq!(profile.buy(Track::WeaponSlots), Some(60));
        assert_eq!(profile.level(Track::WeaponSlots), 1);
        assert_eq!(profile.bank, 940);
        assert_eq!(profile.next_price(Track::WeaponSlots), Some(200));
        assert_eq!(profile.buy(Track::WeaponSlots), Some(200));
        assert_eq!(profile.bank, 740);
        assert_eq!(profile.total_levels(), 2);

        // A short bank refuses and touches nothing.
        let mut poor = MetaProfile::default();
        poor.bank = 59;
        assert_eq!(poor.buy(Track::WeaponSlots), None);
        assert_eq!(poor.bank, 59);
        assert_eq!(poor.level(Track::WeaponSlots), 0);

        // Maxed refuses even with gold in hand.
        let mut maxed = MetaProfile::maxed();
        maxed.bank = 10_000;
        assert_eq!(maxed.buy(Track::Revive), None);
        assert_eq!(maxed.bank, 10_000);

        // The refund is exactly the sum paid: it can neither mint nor burn gold.
        let paid = profile.spent_total();
        assert_eq!(paid, 60 + 200);
        let before = profile.bank;
        assert_eq!(profile.refund_all(), paid);
        assert_eq!(profile.bank, before + paid);
        assert_eq!(profile.total_levels(), 0);
        assert_eq!(profile.spent_total(), 0);
        // Refunding nothing is free and harmless.
        assert_eq!(profile.refund_all(), 0);
    }

    #[test]
    fn refunding_a_maxed_profile_returns_the_whole_table() {
        let mut profile = MetaProfile::maxed();
        let total: u32 = Track::ALL.into_iter().map(Track::total_cost).sum();
        assert_eq!(profile.spent_total(), total);
        assert_eq!(profile.refund_all(), total);
        assert_eq!(profile.bank, total);
        assert!(profile.levels.is_empty(), "a wiped profile stores no levels");
    }

    #[test]
    fn normalizing_drops_unknown_keys_and_clamps_levels() {
        let mut profile = MetaProfile {
            bank: 12,
            levels: BTreeMap::from([
                ("damage".to_string(), 99u8),
                ("move_speed".to_string(), 0u8),
                ("not_a_track".to_string(), 3u8),
                ("revive".to_string(), 1u8),
            ]),
        };
        profile.normalize();
        assert_eq!(profile.bank, 12, "normalizing never touches the bank");
        assert_eq!(profile.level(Track::Damage), Track::Damage.max_level());
        assert_eq!(profile.level(Track::MoveSpeed), 0);
        assert_eq!(profile.level(Track::Revive), 1);
        assert!(!profile.levels.contains_key("not_a_track"));
        assert!(
            !profile.levels.contains_key("move_speed"),
            "a level-0 key is stripped, so a wipe saves like a fresh nest"
        );
        assert_eq!(profile.levels.len(), 2);
    }

    #[test]
    fn the_batch_stages_are_the_three_nest_sizes() {
        assert_eq!(MetaStage::parse("none"), Some(MetaStage::None));
        assert_eq!(MetaStage::parse("mid"), Some(MetaStage::Mid));
        assert_eq!(MetaStage::parse("max"), Some(MetaStage::Max));
        assert_eq!(MetaStage::parse("MAXED"), None);
        for stage in [MetaStage::None, MetaStage::Mid, MetaStage::Max] {
            assert_eq!(MetaStage::parse(stage.name()), Some(stage));
        }
        // A fresh stage is the neutral profile, so `--meta none` and no flag at
        // all measure exactly the same game.
        assert_eq!(MetaStage::None.modifiers(), RunModifiers::NONE);
        assert!(MetaStage::None.modifiers().is_neutral());
        assert_eq!(
            MetaStage::Max.modifiers(),
            RunModifiers::from_profile(&MetaProfile::maxed())
        );
        // Mid is half of every track: 6 + 6 racks, no revive (1/2 rounds down).
        let mid = MetaStage::Mid.modifiers();
        assert_eq!(mid.caps, SlotCaps::new(6, 6));
        assert_eq!(mid.revives, 0);
        assert!(mid.carry_bonus >= 1, "a mid nest has one extra keep slot");
    }

    // ---- the compiled modifiers -----------------------------------------

    #[test]
    fn each_track_buys_exactly_its_own_bonus_at_its_cap() {
        for track in Track::ALL {
            let mods = RunModifiers::from_profile(&only(track));
            // The rack caps: only the slot tracks move them.
            let caps = match track {
                Track::WeaponSlots => SlotCaps::new(MIN_SLOTS + 4, MIN_SLOTS),
                Track::PassiveSlots => SlotCaps::new(MIN_SLOTS, MIN_SLOTS + 4),
                _ => SlotCaps::DEFAULT,
            };
            assert_eq!(mods.caps, caps, "{track:?} changed the racks");

            // Every other field keeps the table's per-level step exactly.
            let cap = track.max_level();
            let step = |on: bool, value: f32, off: f32| if on { value } else { off };
            assert_eq!(mods.move_speed, step(track == Track::MoveSpeed, 1.0 + 0.03 * f32::from(cap), 1.0), "{track:?} move");
            assert_eq!(mods.magnet, step(track == Track::PickupRange, 1.0 + 0.08 * f32::from(cap), 1.0), "{track:?} magnet");
            assert_eq!(mods.fire_rate, step(track == Track::FireRate, 1.0 + 0.02 * f32::from(cap), 1.0), "{track:?} rate");
            assert_eq!(mods.damage, step(track == Track::Damage, 1.0 + 0.025 * f32::from(cap), 1.0), "{track:?} damage");
            assert_eq!(mods.xp, step(track == Track::XpGain, 1.0 + 0.04 * f32::from(cap), 1.0), "{track:?} xp");
            assert_eq!(mods.gold, step(track == Track::GoldGain, 1.0 + 0.05 * f32::from(cap), 1.0), "{track:?} gold");
            assert_eq!(mods.luck, step(track == Track::Luck, 1.0 + 0.15 * f32::from(cap), 1.0), "{track:?} luck");
            assert_eq!(mods.max_hp, step(track == Track::MaxHp, 8.0 * f32::from(cap), 0.0), "{track:?} hp");
            assert_eq!(mods.regen, step(track == Track::Regen, 0.08 * f32::from(cap), 0.0), "{track:?} regen");
            assert_eq!(mods.armor, step(track == Track::Armor, 0.5 * f32::from(cap), 0.0), "{track:?} armor");
            assert_eq!(mods.dash_cd, step(track == Track::DashCooldown, 0.05 * f32::from(cap), 0.0), "{track:?} dash");
            assert_eq!(mods.carry_bonus, if track == Track::CarryOver { cap } else { 0 }, "{track:?} carry");
            assert_eq!(mods.revives, if track == Track::Revive { 1 } else { 0 }, "{track:?} revive");
        }
    }

    #[test]
    fn a_neutral_profile_is_bit_exact_and_a_maxed_one_is_strong_but_small() {
        // The neutral value leaves every stat untouched — bit-for-bit, so every
        // existing number (and every golden trace) is unchanged.
        let base = Stats::default();
        let mut neutral = Stats::default();
        apply(&mut neutral, &RunModifiers::NONE);
        assert_eq!(neutral.max_hp, base.max_hp);
        assert_eq!(neutral.move_speed, base.move_speed);
        assert_eq!(neutral.damage_mult, base.damage_mult);
        assert_eq!(neutral.fire_rate_mult, base.fire_rate_mult);
        assert_eq!(neutral.magnet, base.magnet);
        assert_eq!(neutral.regen, base.regen);
        assert_eq!(neutral.armor, base.armor);
        assert_eq!(neutral.dash_cd, base.dash_cd);
        assert_eq!(neutral.xp_mult, base.xp_mult);
        assert_eq!(neutral.gold_mult, base.gold_mult);
        assert_eq!(neutral.luck_mult, base.luck_mult);

        // A maxed nest is a real edge and nowhere near a win button: every
        // multiplier stays under 1.6x and the flat bonuses are small.
        let mods = RunModifiers::from_profile(&MetaProfile::maxed());
        let mut maxed = Stats::default();
        apply(&mut maxed, &mods);
        assert_eq!(mods.caps, SlotCaps::FULL);
        assert_eq!(mods.carry_bonus, MAX_CARRY_BONUS);
        assert_eq!(mods.revives, 1);
        assert_eq!(maxed.max_hp, 130.0 + 48.0);
        assert_eq!(maxed.move_speed, 58.0 * (1.0 + 0.03 * 6.0));
        assert_eq!(maxed.magnet, 22.0 * (1.0 + 0.08 * 5.0));
        assert_eq!(maxed.fire_rate_mult, 1.0 + 0.02 * 6.0);
        assert_eq!(maxed.damage_mult, 1.0 + 0.025 * 6.0);
        assert_eq!(maxed.regen, 0.4 + 0.08 * 5.0);
        assert_eq!(maxed.armor, 2.0);
        assert_eq!(maxed.dash_cd, 2.0 - 0.05 * 5.0);
        assert_eq!(maxed.xp_mult, 1.0 + 0.04 * 5.0);
        assert_eq!(maxed.gold_mult, 1.0 + 0.05 * 5.0);
        assert_eq!(maxed.luck_mult, 1.0 + 0.15 * 4.0);
        for mult in [
            maxed.damage_mult,
            maxed.fire_rate_mult,
            maxed.xp_mult,
            maxed.gold_mult,
            maxed.luck_mult,
            maxed.move_speed / base.move_speed,
        ] {
            assert!(mult < 1.6, "a permanent multiplier reached {mult}");
        }
    }

    #[test]
    fn the_summary_names_only_what_was_bought() {
        assert_eq!(RunModifiers::NONE.summary(), "");
        assert_eq!(RunModifiers::NONE.tracks_owned(), 0);
        let mods = RunModifiers::from_profile(&only(Track::MoveSpeed));
        assert_eq!(mods.tracks_owned(), 1);
        let summary = mods.summary();
        assert!(summary.contains("+18% move"), "{summary}");
        assert!(!summary.contains("slots"), "nothing else was bought: {summary}");
        let maxed = RunModifiers::from_profile(&MetaProfile::maxed());
        assert_eq!(maxed.tracks_owned(), Track::ALL.len() as u32);
        let summary = maxed.summary();
        for needle in [
            "8+8 slots",
            "+18% move",
            "+40% pickup",
            "+12% rate",
            "+15% dmg",
            "+48 HP",
            "+0.4 regen",
            "+2 armour",
            "-0.25s dash",
            "+20% XP",
            "+25% gold",
            "+60% luck",
            "keep 5",
            "1 revive",
        ] {
            assert!(summary.contains(needle), "{needle} missing from {summary}");
        }
        // `summary_into` writes the same text into a caller's buffer.
        let mut buf = String::from("stale");
        maxed.summary_into(&mut buf);
        assert_eq!(buf, summary);
    }

    // ---- banking ---------------------------------------------------------

    #[test]
    fn a_death_banks_the_unspent_purse_and_nothing_else() {
        let mut profile = MetaProfile::default();
        let tally = bank(&mut profile, &end(120));
        assert_eq!(tally.purse, 120);
        assert_eq!(tally.banked, 120);
        assert_eq!(tally.floor_bonus + tally.apex_bonus + tally.win_bonus, 0);
        assert_eq!(tally.total, 120);
        assert_eq!(profile.bank, 120);
        assert!(!tally.voided && !tally.halved);
        assert!(tally.line().contains("NEST"));
        assert!(tally.line().contains("+120"));
    }

    #[test]
    fn floors_apex_kills_and_the_clear_pay_their_own_bonuses() {
        let mut profile = MetaProfile::default();
        let run = RunEnd {
            purse: 40,
            spent: 90,
            floors: 3,
            apex: 4,
            won: true,
            ..RunEnd::default()
        };
        let tally = bank(&mut profile, &run);
        assert_eq!(tally.raised, 130, "the run raised purse + spent");
        assert_eq!(tally.spent, 90);
        assert_eq!(tally.floor_bonus, FLOOR_BONUS * 3);
        assert_eq!(tally.apex_bonus, APEX_BONUS * 4);
        assert_eq!(tally.win_bonus, WIN_BONUS);
        assert_eq!(tally.banked, 40 + 75 + 24 + 75);
        assert_eq!(profile.bank, tally.banked);
        // The next run adds to it rather than replacing it.
        let second = bank(&mut profile, &end(10));
        assert_eq!(second.total, tally.banked + 10);
    }

    #[test]
    fn spending_in_the_shops_is_a_real_choice_and_never_pays_twice() {
        let mut profile = MetaProfile::default();
        // Two identical runs: one banks everything, one spent 100 on splices.
        let saver = bank(&mut profile, &end(200)).banked;
        let mut other = MetaProfile::default();
        let spender = bank(
            &mut other,
            &RunEnd {
                purse: 100,
                spent: 100,
                ..RunEnd::default()
            },
        );
        assert_eq!(spender.raised, 200, "the raised figure is the same");
        assert_eq!(spender.banked, 100, "but only the unspent gold is banked");
        assert!(spender.banked < saver);
    }

    #[test]
    fn a_practice_run_banks_at_half_rate() {
        let mut profile = MetaProfile::default();
        let run = RunEnd {
            purse: 101,
            floors: 1,
            apex: 3,
            won: true,
            practice: true,
            ..RunEnd::default()
        };
        let tally = bank(&mut profile, &run);
        assert!(tally.halved);
        assert_eq!(tally.purse, 50);
        assert_eq!(tally.win_bonus, WIN_BONUS / PRACTICE_DIVISOR);
        assert_eq!(tally.banked, 101 / 2 + FLOOR_BONUS / 2 + (APEX_BONUS * 3) / 2 + WIN_BONUS / 2);
        assert!(tally.line().contains("half"));
    }

    #[test]
    fn a_console_run_banks_nothing() {
        let mut profile = MetaProfile::default();
        profile.bank = 55;
        let run = RunEnd {
            purse: 900,
            floors: 3,
            apex: 9,
            won: true,
            console_used: true,
            ..RunEnd::default()
        };
        let tally = bank(&mut profile, &run);
        assert!(tally.voided);
        assert_eq!(tally.banked, 0);
        assert_eq!(profile.bank, 55, "the bank is untouched");
        assert_eq!(tally.raised, 900, "the report still says what was raised");
        assert!(tally.line().contains("console"));
        assert!(!run.pays());
    }

    #[test]
    fn banking_saturates_instead_of_wrapping() {
        let mut profile = MetaProfile::default();
        profile.bank = u32::MAX - 5;
        let tally = bank(&mut profile, &end(1_000_000));
        assert_eq!(tally.total, u32::MAX);
        assert_eq!(profile.bank, u32::MAX);
    }
}
