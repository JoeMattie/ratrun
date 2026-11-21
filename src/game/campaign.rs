//! The campaign: three themed levels in order, the carry-over between them and
//! the deterministic seeds that make a run reproducible.
//!
//! ## Run structure (owner's decision)
//!
//! Sewer → Kitchen → Lab. A level is **complete when its boss dies**; killing
//! the Lab boss wins the run. The five-minute clock is no longer the win
//! condition — `director::RUN_SECONDS` is now only the top of the difficulty
//! curve, and the boss fight starts at `director::BOSS_TIME` (255 s) and
//! continues until the boss or the rat dies while the horde keeps coming.
//!
//! The simulation side of that lives in [`super::world::World`] (`level_index`,
//! `level_complete`, `finished`, `campaign_won`) and
//! [`super::ecs::systems::loot::outcome_system`]. This module owns the
//! **between-levels** part the App drives:
//!
//! 1. [`CampaignState::new`] — pick the run seed; the app owns the state.
//! 2. [`CampaignState::world`] — build the `World` for the current level.
//! 3. Play until the boss dies (`world.level_complete()`).
//! 4. [`keepables`] — list what the player may take through, in a stable order
//!    with display strings (the 3-of-N keep screen).
//! 5. [`CampaignState::complete_level`] — validate the choice (≤ 3, no
//!    duplicates), fold the level's score/kills/time into the totals, carry
//!    the chosen powerups and advance the level index.
//! 6. [`CampaignState::world`] again for the next level, or stop when
//!    [`CampaignState::is_complete`] and the run is won.
//!
//! ## Determinism
//!
//! Everything here is a pure function of the run seed, the level index and the
//! player's choices: [`level_seed`] derives each level's map seed with the same
//! splitmix64 finaliser `level::level_seed` uses, and [`CampaignState::world`]
//! builds the map from exactly that seed, so replaying a campaign replays all
//! three maps.

use super::level::Theme;
use super::loadout::{Loadout, PassiveKind};
use super::weapon::{Weapon, WeaponKind};
use super::world::World;

/// Number of levels in the campaign: Sewer, stage two, stage three.
pub const LEVEL_COUNT: usize = 3;
/// How many powerups the player keeps between levels.
pub const KEEP_COUNT: usize = 3;
/// The weapon a level falls back to when the kept powerups include none.
pub const STARTING_WEAPON: WeaponKind = Loadout::START_WEAPON;

/// The **getaway ride**: the flatbed trailer level.
///
/// It is **not** a campaign step: the keep screen's route choice offers two
/// floors per stage ([`Route::options`]) and the Flatbed is not one of them, so
/// it is not in [`Theme::all`]. It is a bonus practice floor on the title's
/// sector row ([`Theme::playable`]), built with level index [`LEVEL_COUNT`]
/// (see [`is_getaway_sector`]). Offering it as a third final-stage route is
/// the hook this function was left for; the route UI is two chips wide today.
pub fn getaway_theme() -> Theme {
    Theme::Trailer
}

/// How many **level indices** a world can be built with: the three campaign
/// stages plus the getaway ride's own index (the bot batch's `--level` range).
/// The title's sector row is wider: it lists every floor, [`Theme::playable`].
pub const SECTOR_COUNT: usize = LEVEL_COUNT + 1;

/// Is level index `sector` the getaway ride rather than a campaign stage?
pub fn is_getaway_sector(sector: usize) -> bool {
    sector == LEVEL_COUNT
}

/// The original-route theme of level index `sector` (the getaway ride for
/// its own index).
pub fn sector_theme(sector: usize) -> Theme {
    if is_getaway_sector(sector) {
        getaway_theme()
    } else {
        level_theme(sector)
    }
}

/// The theme of campaign level `index` **on the original route** (0 Sewer,
/// 1 Kitchen, 2 Lab).
///
/// Out-of-range indices wrap, so a caller cannot index past the campaign; the
/// real bound is [`LEVEL_COUNT`]. A side route goes through
/// [`Route`]/[`CampaignState::theme`], which is what a run in flight uses.
pub fn level_theme(index: usize) -> Theme {
    Route::ORIGINAL.theme(index % LEVEL_COUNT)
}

/// The two floors a campaign stage may be played on (`WAVE3_DESIGN.md` §1).
///
/// Stage one is always the Sewer. Stage two is the Kitchen or the Laundry,
/// stage three the Lab or the Printworks. The original route is the first
/// option everywhere, so "keep the original route as the first menu choice" is
/// a property of the table and not of the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub stage2: Theme,
    pub stage3: Theme,
}

impl Route {
    /// Sewer → Kitchen → Lab.
    pub const ORIGINAL: Route = Route {
        stage2: Theme::Kitchen,
        stage3: Theme::Lab,
    };

    /// The floors a stage may be played on, original route first.
    pub const fn options(stage: usize) -> &'static [Theme] {
        match stage {
            0 => &[Theme::Sewer],
            1 => &[Theme::Kitchen, Theme::Laundry],
            _ => &[Theme::Lab, Theme::Printworks],
        }
    }

    /// The floor this route plays at `stage`.
    pub fn theme(self, stage: usize) -> Theme {
        match stage {
            0 => Theme::Sewer,
            1 => self.stage2,
            _ => self.stage3,
        }
    }

    /// Switch `stage` to `theme`. Returns `false` (and changes nothing) when the
    /// theme is not one of that stage's options.
    pub fn choose(&mut self, stage: usize, theme: Theme) -> bool {
        if !Self::options(stage).contains(&theme) {
            return false;
        }
        match stage {
            0 => {}
            1 => self.stage2 = theme,
            _ => self.stage3 = theme,
        }
        true
    }

    /// The route that uses `theme` at `stage` (unchanged if it is not an
    /// option).
    pub fn with(self, stage: usize, theme: Theme) -> Route {
        let mut route = self;
        route.choose(stage, theme);
        route
    }
}

/// The campaign stage a floor belongs to (a side route floor shares its stage
/// with the original one).
pub fn theme_stage(theme: Theme) -> usize {
    theme.stage()
}

/// Is `index` the campaign's last level (finishing it wins the run)?
pub fn is_final_level(index: usize) -> bool {
    index % LEVEL_COUNT == LEVEL_COUNT - 1
}

/// How many levels come after `index`.
pub fn levels_remaining(index: usize) -> usize {
    LEVEL_COUNT.saturating_sub(index + 1)
}

/// The map seed of campaign level `index`, derived from the run seed.
///
/// Level 0 uses the run seed unchanged, so `CampaignState::new(seed)` level 0
/// generates exactly the map `World::new(theme, seed)` generates (existing
/// seeded tests keep their meaning). Later levels mix the index in with the
/// splitmix64 finaliser, so neighbouring levels are unrelated maps and the same
/// campaign always regenerates the same three levels.
pub fn level_seed(run_seed: u64, index: usize) -> u64 {
    if index == 0 {
        return run_seed;
    }
    mix_seed(run_seed, index as u64)
}

/// splitmix64 finaliser over `(run seed, key)`.
fn mix_seed(run_seed: u64, key: u64) -> u64 {
    let mut h = run_seed ^ key.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The map seed of campaign level `index` played on `theme`.
///
/// The original route's floors keep the seed [`level_seed`] has always given
/// them (so every existing seeded map and test still means the same thing);
/// a side route floor mixes its own theme in, so the Laundry and the Printworks
/// are never generated from the Kitchen's or the Lab's seed.
pub fn level_seed_on(run_seed: u64, index: usize, theme: Theme) -> u64 {
    if index == 0 || theme == Route::ORIGINAL.theme(index) {
        return level_seed(run_seed, index);
    }
    let theme_key = match theme {
        Theme::Laundry => 0x51af_1e3b_c7d9_2a41u64,
        Theme::Printworks => 0x7c3d_8b62_4e19_f0a5u64,
        other => other as u64,
    };
    mix_seed(level_seed(run_seed, index), theme_key)
}

// ---- What the player keeps ----------------------------------------------

/// The identity of one keepable powerup.
///
/// A powerup is **one owned weapon at its current level** or **one owned
/// passive at its current stacks**, so the pair is a unique key: the loadout
/// holds at most one `Weapon`/stack entry per kind, which is also what
/// [`validate_keep`] uses to reject duplicates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeepKind {
    Weapon(WeaponKind),
    Passive(PassiveKind),
}

impl KeepKind {
    pub fn name(self) -> &'static str {
        match self {
            KeepKind::Weapon(k) => k.name(),
            KeepKind::Passive(p) => p.name(),
        }
    }

    pub fn flavor(self) -> &'static str {
        match self {
            KeepKind::Weapon(k) => k.flavor(),
            KeepKind::Passive(p) => p.flavor(),
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            KeepKind::Weapon(k) => k.desc(),
            KeepKind::Passive(p) => p.desc(),
        }
    }

    pub fn is_weapon(self) -> bool {
        matches!(self, KeepKind::Weapon(_))
    }
}

/// One entry of the between-levels keep screen: display info plus the kind and
/// level that [`next_loadout`] needs to rebuild it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Keepable {
    pub kind: KeepKind,
    /// Weapon level (`1..=WeaponKind::max_level()`), or passive stacks (`>= 1`).
    pub level: u8,
    pub name: &'static str,
    /// Weapon/passive flavour line (`WeaponKind::flavor` / `PassiveKind::flavor`).
    pub flavor: &'static str,
    /// Mechanical description (`WeaponKind::desc` / `PassiveKind::desc`).
    pub desc: &'static str,
}

impl Keepable {
    /// Card title: `"Gnaw Lv3"` or `"Sharp Teeth x2"`.
    pub fn title(&self) -> String {
        match self.kind {
            KeepKind::Weapon(_) => format!("{} Lv{}", self.name, self.level),
            KeepKind::Passive(_) => format!("{} x{}", self.name, self.level),
        }
    }

    pub fn is_weapon(&self) -> bool {
        self.kind.is_weapon()
    }
}

/// Why a keep selection is not allowed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeepError {
    /// More than [`KEEP_COUNT`] entries were chosen.
    TooMany { count: usize },
    /// The same weapon/passive appears twice.
    Duplicate(KeepKind),
    /// The entry exists but the previous level's loadout does not own it.
    NotOwned(KeepKind),
}

impl KeepError {
    pub fn message(&self) -> String {
        match self {
            KeepError::TooMany { count } => {
                format!("keep at most {KEEP_COUNT} powerups (chose {count})")
            }
            KeepError::Duplicate(kind) => format!("{} was chosen twice", kind.name()),
            KeepError::NotOwned(kind) => format!("{} is not in the loadout", kind.name()),
        }
    }
}

impl std::fmt::Display for KeepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Every keepable powerup of `loadout`, in a **stable** order: weapons first in
/// the loadout's own (acquisition) order, then passives in their
/// first-acquisition order.
///
/// Stable means "the same loadout always lists the same entries in the same
/// order", so the keep screen's card indices are reproducible; it is not sorted
/// by kind name, because the player's own pick order reads better on the card
/// list (`WeaponKind::all()`/`PassiveKind::all()` order stays available through
/// [`KeepKind`] if the app wants to sort).
pub fn keepables(loadout: &Loadout) -> Vec<Keepable> {
    let mut out = Vec::with_capacity(loadout.weapons.len() + loadout.passives.len());
    for w in &loadout.weapons {
        // An earned evolution keeps its chest name, inscription and attack on
        // the keep manifest: it is what the player is choosing to carry.
        out.push(Keepable {
            kind: KeepKind::Weapon(w.kind),
            level: w.level,
            name: w.name(),
            flavor: w.flavor(),
            desc: w.desc(),
        });
    }
    for &(p, stacks) in &loadout.passives {
        out.push(Keepable {
            kind: KeepKind::Passive(p),
            level: stacks.min(u8::MAX as u32) as u8,
            name: p.name(),
            flavor: p.flavor(),
            desc: p.desc(),
        });
    }
    out
}

/// Validate a keep selection against the loadout it comes from: at most
/// [`KEEP_COUNT`] entries, no duplicates, every entry owned.
///
/// `previous` matters because the entries carry a level: keeping a weapon the
/// level never owned would be an infinite powerup generator. An empty selection
/// is valid (the next level then starts on [`STARTING_WEAPON`]).
pub fn validate_keep(previous: &Loadout, chosen: &[Keepable]) -> Result<(), KeepError> {
    if chosen.len() > KEEP_COUNT {
        return Err(KeepError::TooMany {
            count: chosen.len(),
        });
    }
    for (i, entry) in chosen.iter().enumerate() {
        if chosen[..i].iter().any(|other| other.kind == entry.kind) {
            return Err(KeepError::Duplicate(entry.kind));
        }
        let owned = match entry.kind {
            KeepKind::Weapon(k) => previous.has(k),
            KeepKind::Passive(p) => previous.stacks(p) > 0,
        };
        if !owned {
            return Err(KeepError::NotOwned(entry.kind));
        }
    }
    Ok(())
}

/// Build the next level's starting loadout from up to three chosen entries.
///
/// * Weapons keep their level; passives keep their stacks (and their stat
///   effects are replayed by [`Loadout::from_parts`]).
/// * If none of the chosen entries is a weapon, the level starts with
///   [`STARTING_WEAPON`] at level 1, so a player can never carry themselves into
///   a level with nothing to fire.
/// * Weapons are emitted in `previous`'s order and passives in `previous`'s
///   order, so the carried loadout's slots are stable however the cards were
///   picked.
///
/// Returns [`KeepError`] (and touches nothing) when the selection is invalid.
pub fn next_loadout(previous: &Loadout, chosen: &[Keepable]) -> Result<Loadout, KeepError> {
    validate_keep(previous, chosen)?;

    let mut weapons: Vec<Weapon> = Vec::with_capacity(chosen.len().max(1));
    for w in &previous.weapons {
        if let Some(entry) = chosen
            .iter()
            .find(|c| c.kind == KeepKind::Weapon(w.kind))
        {
            let mut carried = Weapon::new(w.kind);
            carried.level = entry.level.clamp(1, w.kind.max_level());
            carried.evolved = w.evolved;
            weapons.push(carried);
        }
    }
    if weapons.is_empty() {
        weapons.push(Weapon::new(STARTING_WEAPON));
    }

    let mut passives: Vec<(PassiveKind, u32)> = Vec::new();
    for &(p, stacks) in &previous.passives {
        if let Some(entry) = chosen
            .iter()
            .find(|c| c.kind == KeepKind::Passive(p))
        {
            passives.push((p, (entry.level as u32).max(1).min(stacks)));
        }
    }

    // The profile's rack caps ride along between levels unchanged: a run that
    // started on 6 + 6 racks keeps them on every floor it carries into.
    let mut carried = Loadout::from_parts_with_caps(weapons, passives, previous.caps);
    // Gene splices sit **outside the racks**: they ride along whatever the
    // keep-three choice was, and last until the run ends (`WAVE3_DESIGN` §6).
    if !previous.splices.is_empty() {
        carried.splices = previous.splices.clone();
        carried.rebuild_stats();
    }
    Ok(carried)
}

// ---- Cumulative counters -------------------------------------------------

/// Score/kills/time already banked by completed levels.
///
/// The current level's own counters stay on the `World` (`score`, `kills`,
/// `elapsed`): `elapsed` drives the difficulty curve and the boss clock, so it
/// must restart at zero every level. The HUD shows the totals through
/// `World::total_score`/`total_kills`/`total_time`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CampaignTotals {
    pub score: u32,
    pub kills: u32,
    pub elapsed: f32,
    /// Spendable balance at the end of the most recently completed level.
    pub gold: u32,
}

impl CampaignTotals {
    /// Fold one completed level's counters in.
    pub fn absorb(&mut self, world: &World) {
        self.score += world.score;
        self.kills += world.kills;
        self.elapsed += world.elapsed;
        self.gold = world.gold;
    }
}

// ---- The campaign state the App owns -------------------------------------

/// The campaign: which level is next, what the player carries into it, and the
/// totals banked so far.
///
/// The App owns one of these for the life of a run and builds each level's
/// `World` from it ([`CampaignState::world`]); nothing in the simulation
/// mutates it.
#[derive(Clone, Debug)]
pub struct CampaignState {
    /// 0 Sewer, 1 stage two, 2 stage three.
    pub level: usize,
    /// The loadout the next level starts with.
    pub loadout: Loadout,
    /// The seed the whole campaign was rolled from.
    pub run_seed: u64,
    /// Score/kills/time banked by the levels already completed.
    pub totals: CampaignTotals,
    /// Which floor each stage is played on (stage one is always the Sewer).
    pub route: Route,
}

impl CampaignState {
    /// A fresh campaign: level 0, the default loadout, no banked totals.
    pub fn new(run_seed: u64) -> Self {
        CampaignState::with_caps(run_seed, crate::game::loadout::SlotCaps::DEFAULT)
    }

    /// A fresh campaign on racks of `caps` slots (the profile's persistent slot
    /// upgrades, 4 to 8 each side). The caps ride along every carried loadout
    /// (`next_loadout`), so every floor of the campaign uses them.
    pub fn with_caps(run_seed: u64, caps: crate::game::loadout::SlotCaps) -> Self {
        CampaignState {
            level: 0,
            loadout: Loadout::with_caps(Loadout::START_WEAPON, caps),
            run_seed,
            totals: CampaignTotals::default(),
            route: Route::ORIGINAL,
        }
    }

    pub fn theme(&self) -> Theme {
        self.route.theme(self.level)
    }

    pub fn level_seed(&self) -> u64 {
        level_seed_on(self.run_seed, self.level, self.theme())
    }

    /// The floors the **next** level can be played on, original route first
    /// (empty when the current level is the last one).
    ///
    /// The keep screen is shown before [`CampaignState::complete_level`], so
    /// these are the options for stage `level + 1`.
    pub fn next_options(&self) -> &'static [Theme] {
        let next = self.level + 1;
        if next >= LEVEL_COUNT {
            return &[];
        }
        Route::options(next)
    }

    /// Pick the floor for the level the player is about to start (stage
    /// `level + 1`). Returns `false` (and changes nothing) when it is not an
    /// option for that stage.
    pub fn choose_route(&mut self, theme: Theme) -> bool {
        let next = self.level + 1;
        if next >= LEVEL_COUNT {
            return false;
        }
        self.route.choose(next, theme)
    }

    pub fn is_final_level(&self) -> bool {
        is_final_level(self.level)
    }

    /// Every level has been completed (the run is won). Call after
    /// [`CampaignState::complete_level`].
    pub fn is_complete(&self) -> bool {
        self.level >= LEVEL_COUNT
    }

    /// Build the `World` for the current level: its theme from the level index,
    /// its map from [`CampaignState::level_seed`], the carried loadout, and the
    /// banked totals so the HUD can show cumulative numbers.
    pub fn world(&self) -> World {
        World::from_campaign(self)
    }

    /// Bank the finished level's counters, carry the chosen powerups and move to
    /// the next level.
    ///
    /// Only call this for a level the player actually completed (the App checks
    /// `world.level_complete()`); the current level's **loadout** is the source
    /// of truth, so `chosen` must be a subset of `world.keepables()`.
    ///
    /// Returns [`KeepError`] and leaves the state untouched when the selection
    /// is invalid.
    pub fn complete_level(
        &mut self,
        world: &World,
        chosen: &[Keepable],
    ) -> Result<(), KeepError> {
        debug_assert!(
            world.level_complete(),
            "complete_level called for a level that is not complete"
        );
        debug_assert_eq!(
            world.level_index, self.level,
            "complete_level called with a world from another level"
        );
        // Build first: a rejected selection must not bank anything.
        let carried = next_loadout(&world.player.loadout, chosen)?;
        self.totals.absorb(world);
        self.loadout = carried;
        self.level += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::loadout::Upgrade;
    use crate::game::weapon::WeaponKind;

    /// A loadout with three weapons at different levels and two passives.
    fn rich_loadout() -> Loadout {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        lo.apply(&Upgrade::NewWeapon(WeaponKind::TailWhip));
        lo.apply(&Upgrade::LevelWeapon(0));
        lo.apply(&Upgrade::LevelWeapon(0));
        lo.apply(&Upgrade::NewWeapon(WeaponKind::AcidSpit));
        lo.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        lo.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        lo.apply(&Upgrade::Passive(PassiveKind::Damage));
        lo
    }

    fn entry(lo: &Loadout, kind: KeepKind) -> Keepable {
        keepables(lo)
            .into_iter()
            .find(|k| k.kind == kind)
            .expect("entry exists")
    }

    #[test]
    fn levels_are_the_campaign_in_order() {
        // The original route is the first option at every stage, and it is what
        // the bare `level_theme` helper reports.
        assert_eq!(
            [level_theme(0), level_theme(1), level_theme(2)],
            [Theme::Sewer, Theme::Kitchen, Theme::Lab]
        );
        for stage in 0..LEVEL_COUNT {
            assert_eq!(
                Route::options(stage)[0],
                Route::ORIGINAL.theme(stage),
                "stage {stage} must offer the original route first"
            );
        }
        assert_eq!(Route::options(1), &[Theme::Kitchen, Theme::Laundry]);
        assert_eq!(Route::options(2), &[Theme::Lab, Theme::Printworks]);
        assert_eq!(Theme::all().len(), 5, "five floors, three stages");
        assert!(!is_final_level(0) && !is_final_level(1));
        assert!(is_final_level(2));
        assert_eq!(levels_remaining(0), 2);
        assert_eq!(levels_remaining(2), 0);
    }

    #[test]
    fn the_route_choice_is_seeded_and_deterministic() {
        // Two campaigns from one run seed that pick the same side route play
        // exactly the same maps.
        let mut a = CampaignState::new(97);
        let mut b = CampaignState::new(97);
        for camp in [&mut a, &mut b] {
            assert_eq!(camp.route, Route::ORIGINAL, "the original route is the start");
            // The choice always names the *next* stage's floor.
            assert!(camp.choose_route(Theme::Laundry));
            camp.level = 1;
            assert!(camp.choose_route(Theme::Printworks));
        }
        assert_eq!(a.route, b.route);
        assert_eq!(
            a.route,
            Route {
                stage2: Theme::Laundry,
                stage3: Theme::Printworks
            }
        );
        // Stage two and stage three are the side floors.
        assert_eq!(a.theme(), Theme::Laundry);
        assert_eq!(a.level_seed(), level_seed_on(97, 1, Theme::Laundry));
        a.level = 2;
        b.level = 2;
        assert_eq!(a.level_seed(), b.level_seed());
        assert_eq!(a.theme(), Theme::Printworks);
        assert_eq!(a.level_seed(), level_seed_on(97, 2, Theme::Printworks));
        // A side route is a different map from the original at that stage.
        assert_ne!(a.level_seed(), level_seed(97, 2));
        assert_eq!(
            a.level_seed(),
            level_seed_on(97, 2, Theme::Printworks),
            "the seed is a pure function of (run seed, stage, floor)"
        );
        // The original route keeps the historical seeds.
        let mut original = CampaignState::new(97);
        assert_eq!(original.level_seed(), 97, "stage one is the run seed");
        // An illegal choice changes nothing: stage two offers the Kitchen and
        // the Laundry only, stage three the Lab and the Printworks only.
        assert!(!original.choose_route(Theme::Sewer));
        assert!(!original.choose_route(Theme::Printworks));
        assert_eq!(original.route, Route::ORIGINAL);
        original.level = 1;
        assert_eq!(original.level_seed(), level_seed(97, 1));
        assert!(!original.choose_route(Theme::Kitchen), "stage three's list");
        assert_eq!(original.route, Route::ORIGINAL);
    }

    #[test]
    fn the_route_offers_the_design_s_options_original_first() {
        // A fresh campaign is about to pick stage two's floor.
        let mut camp = CampaignState::new(5);
        assert_eq!(camp.next_options(), &[Theme::Kitchen, Theme::Laundry]);
        camp.level = 1;
        assert_eq!(camp.next_options(), &[Theme::Lab, Theme::Printworks]);
        camp.level = 2;
        assert!(camp.next_options().is_empty(), "the last level has no next");
        assert!(!camp.choose_route(Theme::Lab), "the run is over");
        // Every floor belongs to a stage.
        for theme in Theme::all() {
            assert!(theme_stage(theme) < LEVEL_COUNT);
            assert!(
                Route::options(theme_stage(theme)).contains(&theme),
                "{theme:?} is not offered at its own stage"
            );
        }
    }

    #[test]
    fn a_side_route_campaign_plays_the_side_route_floors() {
        let mut camp = CampaignState::new(11);
        let mut world = camp.world();
        assert_eq!(world.level.theme, Theme::Sewer);
        world.mark_level_complete();
        // The keep screen's choice names stage two, then the campaign advances.
        assert!(camp.choose_route(Theme::Laundry), "stage two's side route");
        camp.complete_level(&world, &[]).expect("valid keep");
        let mut world = camp.world();
        assert_eq!(world.level.theme, Theme::Laundry);
        world.mark_level_complete();
        assert!(camp.choose_route(Theme::Printworks), "stage three's side route");
        camp.complete_level(&world, &[]).expect("valid keep");
        let world = camp.world();
        assert_eq!(world.level.theme, Theme::Printworks);
        assert!(world.is_final_level(), "the Printworks is the last floor");
    }

    #[test]
    fn level_zero_keeps_the_run_seed_and_later_levels_differ() {
        assert_eq!(level_seed(11, 0), 11);
        // Same campaign, same maps.
        assert_eq!(level_seed(11, 1), level_seed(11, 1));
        assert_eq!(level_seed(11, 2), level_seed(11, 2));
        // Different level or different run ⇒ different map seed.
        assert_ne!(level_seed(11, 1), level_seed(11, 2));
        assert_ne!(level_seed(11, 1), level_seed(12, 1));
        assert_ne!(level_seed(11, 1), 11);
        // And it is not just `seed + index`.
        assert_ne!(level_seed(11, 1), 12);
    }

    #[test]
    fn keepables_list_every_weapon_and_passive_in_a_stable_order() {
        let lo = rich_loadout();
        let list = keepables(&lo);
        assert_eq!(list.len(), 3 + 2, "3 weapons + 2 passives");
        assert_eq!(list, keepables(&lo), "listing must be stable");
        // Weapons in loadout order, then passives in acquisition order.
        let kinds: Vec<KeepKind> = list.iter().map(|k| k.kind).collect();
        assert_eq!(
            kinds,
            vec![
                KeepKind::Weapon(WeaponKind::Gnaw),
                KeepKind::Weapon(WeaponKind::TailWhip),
                KeepKind::Weapon(WeaponKind::AcidSpit),
                KeepKind::Passive(PassiveKind::MaxHp),
                KeepKind::Passive(PassiveKind::Damage),
            ]
        );
        // Display info: level is the weapon level or the passive stacks.
        assert_eq!(list[0].level, 3, "Gnaw was levelled twice");
        assert_eq!(list[1].level, 1);
        assert_eq!(list[3].level, 2, "two MaxHp stacks");
        assert_eq!(list[3].name, PassiveKind::MaxHp.name());
        assert_eq!(list[3].flavor, PassiveKind::MaxHp.flavor());
        assert_eq!(list[0].title(), "Gnaw Lv3");
        assert_eq!(
            list[3].title(),
            format!("{} x2", PassiveKind::MaxHp.name())
        );
    }

    #[test]
    fn a_fresh_loadout_lists_only_its_starting_weapon() {
        let list = keepables(&Loadout::starting());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, KeepKind::Weapon(STARTING_WEAPON));
        assert_eq!(list[0].level, 1);
    }

    #[test]
    fn validation_rejects_too_many_duplicates_and_unowned_entries() {
        let lo = rich_loadout();
        let list = keepables(&lo);

        let four: Vec<Keepable> = list.iter().take(4).copied().collect();
        assert_eq!(
            validate_keep(&lo, &four),
            Err(KeepError::TooMany { count: 4 })
        );

        let dup = vec![list[0], list[1], list[0]];
        assert_eq!(
            validate_keep(&lo, &dup),
            Err(KeepError::Duplicate(list[0].kind))
        );

        // Not owned by this loadout: a weapon the level never picked up.
        let alien = Keepable {
            kind: KeepKind::Weapon(WeaponKind::SqueakNova),
            level: 1,
            name: WeaponKind::SqueakNova.name(),
            flavor: WeaponKind::SqueakNova.flavor(),
            desc: WeaponKind::SqueakNova.desc(),
        };
        assert_eq!(
            validate_keep(&lo, &[alien]),
            Err(KeepError::NotOwned(alien.kind))
        );

        // Exactly three distinct owned entries is fine, and so is none.
        assert_eq!(validate_keep(&lo, &list[..3]), Ok(()));
        assert_eq!(validate_keep(&lo, &[]), Ok(()));
    }

    /// A carried evolution keeps working **without its partner**: the passive
    /// only opens the chest, the behaviour is the weapon's own `evolved` flag
    /// (the evolution systems never read the passive rack — see
    /// `game::ecs::systems::evolutions`, whose tests fire every evolved weapon
    /// from exactly this partnerless loadout).
    #[test]
    fn wave3_a_kept_evolution_works_without_its_partner() {
        let mut campaign = CampaignState::new(11);
        let mut sewer = campaign.world();
        {
            let loadout = &mut sewer.player.loadout;
            loadout.weapons[0].level = WeaponKind::Gnaw.max_level();
            loadout.apply(&super::super::loadout::Upgrade::Passive(PassiveKind::Damage));
            loadout.evolve(0);
            assert!(loadout.weapons[0].evolved);
        }
        let keep = sewer.keepables();
        assert_eq!(keep[0].name, "Milk Teeth", "the manifest shows the chest name");
        sewer.mark_level_complete();
        campaign.complete_level(&sewer, &keep[..1]).unwrap();

        let kitchen = campaign.world();
        let loadout = &kitchen.player.loadout;
        let weapon = &loadout.weapons[0];
        assert!(weapon.evolved, "the evolved flag is carried");
        assert_eq!(weapon.name(), "Milk Teeth");
        assert_eq!(weapon.desc(), WeaponKind::Gnaw.evolution().unwrap().attack);
        assert_eq!(
            loadout.stacks(PassiveKind::Damage),
            0,
            "the partner stayed behind"
        );
        assert!(
            loadout.evolvable().is_none(),
            "without a partner the chest cannot re-evolve the slot"
        );
    }

    #[test]
    fn wave3_campaign_carries_remaining_gold_and_evolved_weapons() {
        let mut campaign = CampaignState::new(42);
        let mut sewer = campaign.world();
        sewer.gold = 37;
        assert!(!sewer.spend_gold(38));
        assert!(sewer.spend_gold(7));
        let loadout = &mut sewer.player.loadout;
        loadout.weapons[0].level = WeaponKind::Gnaw.max_level();
        loadout.apply(&super::super::loadout::Upgrade::Passive(PassiveKind::Damage));
        loadout.evolve(0);
        let keep = sewer.keepables();
        sewer.mark_level_complete();
        campaign.complete_level(&sewer, &keep[..1]).unwrap();
        let mut kitchen = campaign.world();
        assert_eq!(kitchen.gold, 30);
        assert!(kitchen.player.loadout.weapons[0].evolved);
        assert!(kitchen.spend_gold(30));
        kitchen.mark_level_complete();
        campaign.complete_level(&kitchen, &[]).unwrap();
        assert_eq!(campaign.world().gold, 0);
    }

    #[test]
    fn carry_over_produces_exactly_the_chosen_upgrades_at_their_levels() {
        let lo = rich_loadout();
        let chosen = vec![
            entry(&lo, KeepKind::Weapon(WeaponKind::Gnaw)),
            entry(&lo, KeepKind::Passive(PassiveKind::MaxHp)),
            entry(&lo, KeepKind::Passive(PassiveKind::Damage)),
        ];
        let next = next_loadout(&lo, &chosen).expect("valid selection");

        // Exactly the chosen upgrades: one weapon, two passives.
        assert_eq!(next.weapons.len(), 1);
        assert_eq!(next.weapons[0].kind, WeaponKind::Gnaw);
        assert_eq!(next.weapons[0].level, 3, "weapon level carried");
        assert_eq!(next.passives.len(), 2);
        assert_eq!(next.stacks(PassiveKind::MaxHp), 2);
        assert_eq!(next.stacks(PassiveKind::Damage), 1);
        assert_eq!(next.stacks(PassiveKind::Armor), 0);

        // The stats are replayed from the stacks, not copied blindly: three
        // MaxHp picks in total would give 160 max HP, here only the kept two do.
        assert_eq!(next.stats.max_hp, StatsDefault::MAX_HP + 2.0 * 20.0);
        assert_eq!(next.stats.damage_mult, 1.15);
        // Nothing else leaks through.
        assert_eq!(next.stats.move_speed, StatsDefault::MOVE_SPEED);
        assert_eq!(next.weapons[0].timer, Weapon::new(WeaponKind::Gnaw).timer);
    }

    /// The default `Stats` values, so the test above states its baseline.
    struct StatsDefault;
    impl StatsDefault {
        const MOVE_SPEED: f32 = 58.0;
        /// The base pool the b3-balance pass raised from 100 (contact damage
        /// came down with it; see `Stats::default`).
        const MAX_HP: f32 = 130.0;
    }

    #[test]
    fn keeping_no_weapon_falls_back_to_the_starting_weapon() {
        let lo = rich_loadout();
        let chosen = vec![entry(&lo, KeepKind::Passive(PassiveKind::MaxHp))];
        let next = next_loadout(&lo, &chosen).expect("valid selection");
        assert_eq!(next.weapons.len(), 1);
        assert_eq!(next.weapons[0].kind, STARTING_WEAPON);
        assert_eq!(next.weapons[0].level, 1, "the fallback starts at level 1");
        assert_eq!(next.stacks(PassiveKind::MaxHp), 2);
    }

    #[test]
    fn an_empty_selection_starts_the_next_level_fresh() {
        let lo = rich_loadout();
        let next = next_loadout(&lo, &[]).expect("valid selection");
        assert_eq!(next.weapons.len(), 1);
        assert_eq!(next.weapons[0].kind, STARTING_WEAPON);
        assert!(next.passives.is_empty());
        assert_eq!(next.stats.max_hp, StatsDefault::MAX_HP);
    }

    #[test]
    fn a_rejected_selection_changes_nothing() {
        let lo = rich_loadout();
        let bad = vec![
            entry(&lo, KeepKind::Weapon(WeaponKind::TailWhip)),
            entry(&lo, KeepKind::Weapon(WeaponKind::TailWhip)),
        ];
        assert!(next_loadout(&lo, &bad).is_err());
    }

    #[test]
    fn carried_weapons_keep_loadout_slot_order_however_they_were_picked() {
        let lo = rich_loadout();
        let gnaw = entry(&lo, KeepKind::Weapon(WeaponKind::Gnaw));
        let spit = entry(&lo, KeepKind::Weapon(WeaponKind::AcidSpit));
        // Picked in reverse loadout order.
        let next = next_loadout(&lo, &[spit, gnaw]).expect("valid selection");
        assert_eq!(next.weapons[0].kind, WeaponKind::Gnaw);
        assert_eq!(next.weapons[1].kind, WeaponKind::AcidSpit);
    }

    // ---- World / CampaignState integration -----------------------------

    /// A campaign's level 0 is the map `World::new(theme, seed)` builds.
    #[test]
    fn the_first_level_uses_the_run_seed() {
        let campaign = CampaignState::new(17);
        let a = campaign.world();
        let b = World::new(Theme::Sewer, 17);
        assert_eq!(a.level.seed, b.level.seed);
        assert_eq!(a.level.theme, b.level.theme);
        assert_eq!(a.level_index(), 0);
        assert_eq!(a.player.loadout.weapons.len(), 1);
        assert_eq!(a.player.loadout.weapons[0].kind, STARTING_WEAPON);
    }

    /// The next level's seed is a deterministic function of the run seed and
    /// the level index — the whole point of a replayable campaign.
    #[test]
    fn the_next_levels_seed_is_deterministic() {
        let mut campaign = CampaignState::new(23);
        let mut world = campaign.world();
        world.mark_level_complete();
        campaign.complete_level(&world, &[]).expect("valid");
        let expected = level_seed(23, 1);
        assert_eq!(campaign.level_seed(), expected);
        // Two fresh campaigns from the same run seed make the same level 1.
        let other = CampaignState {
            level: 1,
            loadout: Loadout::starting(),
            run_seed: 23,
            totals: CampaignTotals::default(),
            route: Route::ORIGINAL,
        };
        assert_eq!(campaign.world().level.seed, other.world().level.seed);
        assert_eq!(campaign.world().level.seed, expected);
        assert_ne!(expected, level_seed(23, 0), "levels get different maps");
        // And the level-1 theme is the Kitchen, not the Sewer.
        assert_eq!(campaign.theme(), Theme::Kitchen);
        assert_eq!(campaign.world().level.theme, Theme::Kitchen);
    }

    /// **Splices carry over cleanly** (`WAVE3_DESIGN` §6: they last until the
    /// campaign ends, occupy no rack slot and are applied after the passives).
    ///
    /// Two things have to hold on the next floor: the stats are exactly what a
    /// from-scratch rebuild of the carried loadout gives (no doubled or lost
    /// splice multiplier), and HP is clamped to the new max — Glass Incisors
    /// and Long Fingers both *lower* the pool, so a rat that walked in with a
    /// full bar must not carry the old, larger number into the next level.
    #[test]
    fn carried_splices_rebuild_the_same_stats_and_clamp_hp() {
        use crate::game::shop::SpliceKind;
        let mut campaign = CampaignState::new(67);
        let mut world = campaign.world();
        let mut rich = rich_loadout();
        // Buy the two max-HP penalties and a damage multiplier.
        rich.splices = vec![
            SpliceKind::GlassIncisors,
            SpliceKind::LongFingers,
            SpliceKind::HoardGland,
        ];
        rich.rebuild_stats();
        world.player.loadout = rich.clone();
        let hp_before = world.player.max_hp();
        // Walk in with a full health bar at the old cap.
        world.player.hp = hp_before;

        let chosen = vec![
            entry(&rich, KeepKind::Weapon(WeaponKind::Gnaw)),
            entry(&rich, KeepKind::Passive(PassiveKind::MaxHp)),
        ];
        world.mark_level_complete();
        campaign.complete_level(&world, &chosen).expect("valid keep");
        let next = campaign.world();
        let lo = &next.player.loadout;

        // The same loadout rebuilt from scratch gives the same stats.
        let mut rebuilt = Loadout::from_parts(
            lo.weapons.clone(),
            lo.passives.clone(),
        );
        rebuilt.splices = lo.splices.clone();
        rebuilt.rebuild_stats();
        assert_eq!(lo.stats.max_hp, rebuilt.stats.max_hp, "max HP");
        assert_eq!(lo.stats.damage_mult, rebuilt.stats.damage_mult, "damage");
        assert_eq!(lo.stats.magnet, rebuilt.stats.magnet, "pickup radius");
        assert_eq!(lo.stats.dash_cd, rebuilt.stats.dash_cd, "dash cooldown");
        // HP starts at the *new* (smaller) pool, never above it.
        assert!(
            next.player.hp <= next.player.max_hp() + 1e-3,
            "carried HP {} over the new cap {}",
            next.player.hp,
            next.player.max_hp()
        );
        assert_eq!(next.player.hp, next.player.max_hp(), "a new level starts full");
        // The two HP-penalty splices are still in the carried stats: the pool
        // is smaller than the same loadout with no splices.
        let mut plain = rebuilt.clone();
        plain.splices.clear();
        plain.rebuild_stats();
        assert!(
            next.player.max_hp() < plain.stats.max_hp,
            "the carried max-HP penalties were lost ({} vs {})",
            next.player.max_hp(),
            plain.stats.max_hp
        );
        let _ = hp_before;
    }

    /// Carry-over end to end: finish a level with a rich loadout, keep three
    /// powerups, and start the next level with exactly those at their levels.
    #[test]
    fn carrying_over_starts_the_next_level_with_the_chosen_powerups() {
        let mut campaign = CampaignState::new(31);
        let mut world = campaign.world();
        let rich = rich_loadout();
        world.player.loadout = rich.clone();

        let list = world.keepables();
        assert_eq!(list, keepables(&rich), "World::keepables lists the loadout");
        let chosen = vec![
            entry(&rich, KeepKind::Weapon(WeaponKind::TailWhip)),
            entry(&rich, KeepKind::Passive(PassiveKind::MaxHp)),
            entry(&rich, KeepKind::Passive(PassiveKind::Damage)),
        ];
        world.mark_level_complete();
        campaign.complete_level(&world, &chosen).expect("valid keep");
        assert_eq!(campaign.level, 1);
        assert!(!campaign.is_complete());

        let next = campaign.world();
        assert_eq!(next.level_index(), 1);
        assert!(!next.is_final_level());
        let lo = &next.player.loadout;
        assert_eq!(lo.weapons.len(), 1);
        assert_eq!(lo.weapons[0].kind, WeaponKind::TailWhip);
        assert_eq!(lo.weapons[0].level, 1);
        assert_eq!(lo.stacks(PassiveKind::MaxHp), 2);
        assert_eq!(lo.stacks(PassiveKind::Damage), 1);
        // The passives were replayed onto fresh stats: full HP at level start.
        // The base pool is 130 HP (moved 100 → 130 in the early-game balance
        // pass) and MaxHp is +20 per stack, so two stacks are 130 + 40 = 170.
        assert_eq!(lo.stats.max_hp, 170.0);
        assert_eq!(next.player.hp, 170.0);
        // XP/level restart; the carried powerups are the progression.
        assert_eq!(next.player.level, 1);
        assert!(next.keepables().iter().any(|k| k.kind
            == KeepKind::Passive(PassiveKind::MaxHp)));
    }

    /// A rejected keep selection banks nothing and does not advance.
    #[test]
    fn a_rejected_selection_does_not_advance_the_campaign() {
        let mut campaign = CampaignState::new(37);
        let mut world = campaign.world();
        world.mark_level_complete();
        let before = campaign.totals;
        let bad = vec![
            Keepable {
                kind: KeepKind::Weapon(WeaponKind::SqueakNova),
                level: 1,
                name: "x",
                flavor: "x",
                desc: "x",
            };
            2
        ];
        assert!(campaign.complete_level(&world, &bad).is_err());
        assert_eq!(campaign.level, 0);
        assert_eq!(campaign.totals, before);
    }

    /// Score, kills and time are cumulative across levels; the level's own
    /// clock restarts at zero (it drives the difficulty curve).
    #[test]
    fn cumulative_counters_add_up() {
        let mut campaign = CampaignState::new(41);
        assert_eq!(campaign.totals, CampaignTotals::default());

        let mut level0 = campaign.world();
        assert_eq!(level0.total_score(), 0);
        level0.score = 120;
        level0.kills = 7;
        level0.elapsed = 302.5;
        level0.mark_level_complete();
        campaign.complete_level(&level0, &[]).expect("valid");

        assert_eq!(
            campaign.totals,
            CampaignTotals {
                score: 120,
                kills: 7,
                elapsed: 302.5,
                gold: 0,
            }
        );

        let mut level1 = campaign.world();
        assert_eq!(level1.score, 0, "the level counter restarts");
        assert_eq!(level1.elapsed, 0.0, "the level clock restarts");
        assert_eq!(level1.total_score(), 120);
        assert_eq!(level1.total_kills(), 7);
        assert_eq!(level1.total_time(), 302.5);

        level1.score = 80;
        level1.kills = 3;
        level1.elapsed = 250.0;
        level1.mark_level_complete();
        campaign.complete_level(&level1, &[]).expect("valid");
        assert_eq!(campaign.totals.score, 200);
        assert_eq!(campaign.totals.kills, 10);
        assert!((campaign.totals.elapsed - 552.5).abs() < 1e-3);

        let level2 = campaign.world();
        assert_eq!(level2.total_score(), 200);
        assert_eq!(level2.total_kills(), 10);
        assert!((level2.total_time() - 552.5).abs() < 1e-3);
        assert!(level2.is_final_level());
    }

    /// Beat the Lab level and the campaign is over and won.
    #[test]
    fn completing_the_last_level_ends_the_campaign() {
        let mut campaign = CampaignState {
            level: 2,
            loadout: Loadout::starting(),
            run_seed: 43,
            totals: CampaignTotals::default(),
            route: Route::ORIGINAL,
        };
        let mut world = campaign.world();
        assert!(world.is_final_level());
        world.mark_level_complete();
        assert!(world.campaign_won(), "the Lab boss ends the run");
        // A final level cannot be advanced past the campaign.
        campaign.complete_level(&world, &[]).expect("valid");
        assert!(campaign.is_complete());
        assert_eq!(campaign.level, LEVEL_COUNT);
    }

    /// The carry-over and the derived seeds keep the campaign deterministic:
    /// two identical campaigns fed the same inputs produce identical levels and
    /// identical simulations.
    #[test]
    fn twin_campaigns_run_identically() {
        fn trace(run_seed: u64) -> (u64, u64, f32, u32, u32, usize, Vec<i64>) {
            let campaign = CampaignState::new(run_seed);
            let mut world = campaign.world();
            for tick in 0..240u32 {
                let dir = crate::math::Vec2::from_angle(tick as f32 * 0.03);
                world.update(1.0 / 60.0, dir, tick % 97 == 0);
                if world.finished() {
                    break;
                }
            }
            let enemies = world.enemies();
            let mut positions: Vec<i64> = enemies
                .iter()
                .map(|e| e.pos.x.to_bits() as i64 ^ ((e.pos.y.to_bits() as i64) << 32))
                .collect();
            positions.sort_unstable();
            (
                world.level.seed,
                world.kills as u64,
                world.elapsed,
                world.score,
                world.player.hp.to_bits(),
                enemies.len(),
                positions,
            )
        }
        assert_eq!(trace(51), trace(51));
        assert_ne!(trace(51).0, trace(52).0, "different seeds, different maps");
    }

    #[test]
    fn weapon_levels_are_clamped_to_the_kind_maximum() {
        let mut lo = Loadout::starting();
        let mut bogus = entry(&lo, KeepKind::Weapon(WeaponKind::Gnaw));
        bogus.level = 99;
        lo.weapons[0].level = 99; // even a corrupted source cannot over-level
        let next = next_loadout(&lo, &[bogus]).expect("valid");
        assert_eq!(next.weapons[0].level, WeaponKind::Gnaw.max_level());
    }
}
