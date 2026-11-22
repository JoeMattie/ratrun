//! **The Library**: the facility's own specimen index.
//!
//! The title's `LIBRARY` row and the pause screen open a four-tab dossier over
//! everything the player has actually met: the ordinary horde and the named
//! appointments (**BESTIARY**), the ten slots' worth of gear (**ARSENAL**), the
//! floor furniture — finds, traps, hazards and the level's own features
//! (**FIELD GUIDE**) and the six floors themselves (**FLOORS**).
//!
//! ## One registry, no drift
//!
//! Every entry is a [`Row`] built by [`all_rows`] from **exhaustive matches**
//! over the game's own enums ([`EnemyKind`], [`MinibossKind`], [`BossKind`],
//! [`WeaponKind`], [`PassiveKind`], [`SpliceKind`], [`PickupKind`],
//! [`PropKind`], [`Theme`] and this module's [`ObjectKind`]). A new variant
//! fails to compile until it has a row, so the index cannot silently fall
//! behind the content.
//!
//! **Facts are read from the game's real tables** ([`weapon::Weapon::damage`],
//! [`EnemyKind::base_hp`], [`shop::gold_row`], [`miniboss::appointment_hp`],
//! [`boss::BossKind::base_hp`], …), never retyped: a balance pass moves the
//! bestiary's numbers with it. Only the words (names, hints and the dossier
//! paragraphs) are authored here.
//!
//! ## What can be added from a sibling branch
//!
//! A worker adding a kind adds exactly three things, all append-only:
//!
//! 1. a [`Key`] arm and its [`ObjectKind`]/enum variant (the enums are
//!    appended at the end — see each file's list),
//! 2. one row in [`all_rows`] (name, hint, dossier),
//! 3. the matching arm in [`Key::category`] / [`Key::kind_index`] and in the
//!    group's `facts`/`dossier` match.
//!
//! `registry_is_complete` fails until 2 and 3 exist, and a missing match arm is
//! a compile error.
//!
//! ## Discovery
//!
//! [`Library`] holds three arrays per category: the persisted `seen` bitset,
//! the persisted [`Record`] counters, and the session's `fresh` (NEW) bitset.
//! [`Library::survey`] is the only writer during a run. It is **render-side**:
//! it reads the ECS world and the fog (line of sight) and writes nothing but
//! the library, so it draws no rng and cannot move a golden trace.
//!
//! It is also cheap by construction: each category carries an "everything seen"
//! early-out, so a mature run walks nothing at all; while something is still
//! unknown the pass is one bitset test per *visible* body (the fog's
//! `in_view` is a single bitset + stamp compare) plus a per-frame entity walk
//! the HUD already does for the enemy count.
//!
//! An entry is locked until:
//!
//! | category | unlocked by |
//! |---|---|
//! | enemy | on screen **inside the rat's line of sight** (fog `in_view`) |
//! | elite | an elite body on screen |
//! | miniboss / boss | its fight starting |
//! | weapon / passive | offered on a level-up card (silhouette + name) or owned (full entry) |
//! | evolution | offered by a chest or owned |
//! | pickup / gold / chest | collected or seen up close |
//! | trap / hazard / feature / prop | seen (in sight) |
//! | floor | entered |
//!
//! `RATRUN_LIBRARY_ALL=1` reveals everything in memory for screenshots and
//! tests; it is never written back to the score file.

use serde::{Deserialize, Serialize};

use crate::game::collision::cell_of;
use crate::game::ecs::systems::boss::BossKind;
use crate::game::ecs::systems::miniboss::{self, MinibossKind};
use crate::game::enemy::EnemyKind;
use crate::game::gem::PickupKind;
use crate::game::level::{theme_props, PropKind, Theme};
use crate::game::loadout::PassiveKind;
use crate::game::shop::{self, SpliceKind};
use crate::game::weapon::{Weapon, WeaponKind};
use crate::math::Vec2;

/// The screen's four tabs, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Bestiary,
    Arsenal,
    FieldGuide,
    Floors,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Bestiary, Tab::Arsenal, Tab::FieldGuide, Tab::Floors];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Bestiary => "BESTIARY",
            Tab::Arsenal => "ARSENAL",
            Tab::FieldGuide => "FIELD GUIDE",
            Tab::Floors => "FLOORS",
        }
    }

    /// The tab `step` places to the right (wrapping).
    pub fn step(self, step: isize) -> Tab {
        let n = Tab::ALL.len() as isize;
        Tab::ALL[(self.index() as isize + step).rem_euclid(n) as usize]
    }
}

/// One persisted bucket of flags/counters. The variants are **append-only**:
/// the persisted bitset and counter arrays index by discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Enemy,
    Miniboss,
    Boss,
    Weapon,
    Evolution,
    Passive,
    Splice,
    Pickup,
    Prop,
    Floor,
    /// Everything in the Field Guide that is not a pickup or a prop: traps,
    /// hazards, level features and the gilded elite note.
    Object,
}

impl Category {
    pub const ALL: [Category; 11] = [
        Category::Enemy,
        Category::Miniboss,
        Category::Boss,
        Category::Weapon,
        Category::Evolution,
        Category::Passive,
        Category::Splice,
        Category::Pickup,
        Category::Prop,
        Category::Floor,
        Category::Object,
    ];
}

pub const CATEGORY_COUNT: usize = Category::ALL.len();
/// Kinds one category can hold. Far above today's largest (23 props); a new
/// variant past it is a test failure, not silent corruption.
pub const MAX_KINDS: usize = 64;
/// 64-bit words per category bitset.
pub const WORDS: usize = MAX_KINDS.div_ceil(64);

/// The Field Guide's non-pickup, non-prop entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    /// An elite (any kind, gilded): the bestiary's note entry.
    Elite,
    Gold,
    Chest,
    StickyBoard,
    Mousetrap,
    SteamGrate,
    InkPad,
    Pit,
    SlidingCargo,
    Slipstream,
    RatHole,
    ShopHole,
    MazeCache,
    OneWayDoor,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 14] = [
        ObjectKind::Elite,
        ObjectKind::Gold,
        ObjectKind::Chest,
        ObjectKind::StickyBoard,
        ObjectKind::Mousetrap,
        ObjectKind::SteamGrate,
        ObjectKind::InkPad,
        ObjectKind::Pit,
        ObjectKind::SlidingCargo,
        ObjectKind::Slipstream,
        ObjectKind::RatHole,
        ObjectKind::ShopHole,
        ObjectKind::MazeCache,
        ObjectKind::OneWayDoor,
    ];
}

/// One library entry's identity. Every variant is a row in [`all_rows`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enemy(EnemyKind),
    Miniboss(MinibossKind),
    Boss(BossKind),
    Weapon(WeaponKind),
    Evolution(WeaponKind),
    Passive(PassiveKind),
    Splice(SpliceKind),
    Pickup(PickupKind),
    Prop(PropKind),
    Floor(Theme),
    Object(ObjectKind),
}

impl std::fmt::Debug for Key {
    /// Prints the entry's own name (no `PropKind: Debug` needed in the sim).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", name_of(*self))
    }
}

impl Key {
    pub fn category(self) -> Category {
        match self {
            Key::Enemy(_) => Category::Enemy,
            Key::Miniboss(_) => Category::Miniboss,
            Key::Boss(_) => Category::Boss,
            Key::Weapon(_) => Category::Weapon,
            Key::Evolution(_) => Category::Evolution,
            Key::Passive(_) => Category::Passive,
            Key::Splice(_) => Category::Splice,
            Key::Pickup(_) => Category::Pickup,
            Key::Prop(_) => Category::Prop,
            Key::Floor(_) => Category::Floor,
            Key::Object(_) => Category::Object,
        }
    }

    /// Index inside its category (the enum's discriminant, so appending a
    /// variant at the end never moves an existing entry's bit).
    pub fn kind_index(self) -> usize {
        match self {
            Key::Enemy(k) => k as usize,
            Key::Miniboss(k) => k as usize,
            Key::Boss(k) => k as usize,
            Key::Weapon(k) | Key::Evolution(k) => k as usize,
            Key::Passive(k) => k as usize,
            Key::Splice(k) => k as usize,
            Key::Pickup(k) => k as usize,
            Key::Prop(k) => k as usize,
            Key::Floor(t) => t as usize,
            Key::Object(k) => k as usize,
        }
    }
}

// ---- Registry rows ---------------------------------------------------------

/// One line of the index: what it is called, where it might be found (the
/// locked hint) and its dossier paragraph.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub key: Key,
    pub tab: Tab,
    /// Section header inside the tab (`"ORDINARY"`, `"TRAPS"`, …).
    pub group: &'static str,
    pub name: &'static str,
    pub hint: &'static str,
    pub dossier: &'static str,
}

/// Short display name for a key, reading the game's own tables where they have
/// one and the row's authored name otherwise. Used by the HUD's discovery
/// callout, which has no row list in hand.
pub fn name_of(key: Key) -> &'static str {
    match key {
        Key::Enemy(k) => enemy_name(k),
        Key::Miniboss(k) => k.name(),
        Key::Boss(k) => boss_name(k),
        Key::Weapon(k) => k.name(),
        Key::Evolution(k) => k.evolution().map(|e| e.name).unwrap_or(""),
        Key::Passive(k) => k.name(),
        Key::Splice(k) => k.name(),
        Key::Pickup(k) => pickup_name(k),
        Key::Prop(k) => prop_name(k),
        Key::Floor(t) => t.name(),
        Key::Object(k) => object_name(k),
    }
}

fn enemy_name(k: EnemyKind) -> &'static str {
    match k {
        EnemyKind::Skitterer => "Skitterer",
        EnemyKind::Bat => "Bat",
        EnemyKind::Cat => "Cat",
        EnemyKind::Spitter => "Spitter",
        EnemyKind::Brute => "Brute",
        EnemyKind::Boss => "Containment Breach",
        EnemyKind::Pillbug => "Pillbug",
        EnemyKind::BellFlea => "Bell Flea",
        EnemyKind::WickMoth => "Wick Moth",
        EnemyKind::Porter => "Porter",
        EnemyKind::Turnkey => "Turnkey",
        EnemyKind::BigSnake => "Big Snake",
        EnemyKind::HouseCentipede => "House Centipede",
        EnemyKind::MurderHornet => "Murder Hornet",
        EnemyKind::TermiteHorde => "Termite Horde",
        EnemyKind::Lilliputian => "Lilliputians",
    }
}

fn boss_name(k: BossKind) -> &'static str {
    // The fight's own name lives in the campaign screens and the HUD's boss
    // bar; the library reads the same string from there so a rename lands in
    // one place.
    match k {
        BossKind::Cat => crate::render::hud::boss_codename(Theme::Sewer).0,
        BossKind::Broom => crate::render::hud::boss_codename(Theme::Kitchen).0,
        BossKind::Mower => crate::render::hud::boss_codename(Theme::Lab).0,
        BossKind::Mangle => crate::render::hud::boss_codename(Theme::Laundry).0,
        BossKind::NightEdition => crate::render::hud::boss_codename(Theme::Printworks).0,
        BossKind::Magpie => crate::render::hud::boss_codename(Theme::Trailer).0,
        BossKind::HorseshoeCrab | BossKind::MegaRat => {
            crate::render::hud::boss_codename_kind(k).0
        }
        BossKind::Hawk => crate::render::hud::boss_codename_kind(BossKind::Hawk).0,
        BossKind::Cobra => crate::render::hud::boss_codename_kind(k).0,
    }
}

fn pickup_name(k: PickupKind) -> &'static str {
    match k {
        PickupKind::Heal => "Medkit",
        PickupKind::Magnet => "Magnet",
        PickupKind::Nuke => "Purge",
        PickupKind::BottleCap => "Bottle Cap",
        PickupKind::WindupMouse => "Wind-up Mouse",
        PickupKind::PocketWatch => "Pocket Watch",
        PickupKind::CleanSocks => "Clean Socks",
    }
}

fn prop_name(k: PropKind) -> &'static str {
    match k {
        PropKind::PipeV => "Standing Pipe",
        PropKind::PipeH => "Pipe Run",
        PropKind::Valve => "Valve Wheel",
        PropKind::Grate => "Drain Grate",
        PropKind::Table => "Prep Table",
        PropKind::Crate => "Crate",
        PropKind::Barrel => "Barrel",
        PropKind::Counter => "Counter",
        PropKind::Console => "Console Desk",
        PropKind::Tank => "Specimen Tank",
        PropKind::Barrier => "Blast Barrier",
        PropKind::Washer => "Drum Washer",
        PropKind::TowelRoll => "Towel Roll",
        PropKind::Clothespin => "Giant Clothespin",
        PropKind::PressIsland => "Press Island",
        PropKind::PaperReel => "Paper Reel",
        PropKind::TypeBlock => "Reversed Type",
        PropKind::Pallet => "Slat Pallet",
        PropKind::TarpLump => "Tarped Load",
        PropKind::Tyre => "Spare Tyres",
        PropKind::Toolbox => "Toolbox",
        PropKind::PipeStack => "Pipe Stack",
        PropKind::ChainCoil => "Chain Coil",
    }
}

fn object_name(k: ObjectKind) -> &'static str {
    match k {
        ObjectKind::Elite => "Elite",
        ObjectKind::Gold => "Coin",
        ObjectKind::Chest => "Chest",
        ObjectKind::StickyBoard => "Sticky Board",
        ObjectKind::Mousetrap => "Mousetrap",
        ObjectKind::SteamGrate => "Steam Grate",
        ObjectKind::InkPad => "Ink Pad",
        ObjectKind::Pit => "Broken Plank",
        ObjectKind::SlidingCargo => "Sliding Cargo",
        ObjectKind::Slipstream => "Slipstream",
        ObjectKind::RatHole => "Rat-Hole",
        ObjectKind::ShopHole => "Mouse-Hole Shop",
        ObjectKind::MazeCache => "Maze Cache",
        ObjectKind::OneWayDoor => "One-Way Door",
    }
}

/// The theme a prop kind belongs to (for the row's hint).
fn prop_theme(k: PropKind) -> Theme {
    for theme in Theme::playable() {
        if theme_props(theme).contains(&k) {
            return theme;
        }
    }
    Theme::Sewer
}

/// The floor an entry belongs to, for the card's backdrop and material tint.
///
/// Enemies and appointments are the horde's (so the Sewer's floor); a boss
/// card stands on its own floor; a prop or hazard on the theme that furnishes
/// it; a splice on the wall it is sold behind.
pub fn theme_of(key: Key) -> Theme {
    match key {
        Key::Prop(k) => prop_theme(k),
        Key::Boss(k) => boss_theme(k),
        Key::Floor(t) => t,
        Key::Object(ObjectKind::SteamGrate) => Theme::Laundry,
        Key::Object(ObjectKind::InkPad) => Theme::Printworks,
        Key::Object(ObjectKind::Pit)
        | Key::Object(ObjectKind::SlidingCargo)
        | Key::Object(ObjectKind::Slipstream) => Theme::Trailer,
        Key::Splice(_) => Theme::Laundry,
        _ => Theme::Sewer,
    }
}

/// The game's own flavour line for an entry, where the content tables carry one
/// (weapons, passives and splices do; enemies speak through their dossiers).
pub fn flavour_of(key: Key) -> Option<&'static str> {
    match key {
        Key::Weapon(k) => Some(k.flavor()),
        Key::Passive(k) => Some(k.flavor()),
        Key::Splice(k) => Some(k.label()),
        Key::Evolution(k) => k.evolution().map(|e| e.flavor),
        _ => None,
    }
}

/// The boss's floor (the inverse of [`BossKind::pool_for_theme`]).
///
/// A floor can field more than one boss, so this asks which floor's *pool*
/// holds the kind rather than which floor names it first: the pool mates (the
/// horseshoe, SUBJECT R-01, the cobra) each report the floor they share.
fn boss_theme(k: BossKind) -> Theme {
    for theme in Theme::playable() {
        if BossKind::pool_for_theme(theme).contains(&k) {
            return theme;
        }
    }
    Theme::Sewer
}

/// The whole index, in tab order. Built once by the App; the screen filters it
/// by [`Row::tab`].
pub fn all_rows() -> Vec<Row> {
    let mut rows = Vec::with_capacity(160);
    // ---- BESTIARY -------------------------------------------------------
    const ORDINARY: &str = "ORDINARY";
    for k in [
        EnemyKind::Skitterer,
        EnemyKind::Bat,
        EnemyKind::Cat,
        EnemyKind::Spitter,
        EnemyKind::Brute,
        EnemyKind::Pillbug,
        EnemyKind::BellFlea,
        EnemyKind::WickMoth,
        EnemyKind::Porter,
        EnemyKind::Turnkey,
        EnemyKind::HouseCentipede,
        EnemyKind::Lilliputian,
        EnemyKind::TermiteHorde,
        EnemyKind::MurderHornet,
        EnemyKind::BigSnake,
        // The legacy generic boss body (tests and the facade can spawn it); the
        // floors' own fights have their own BOSSES rows below.
        EnemyKind::Boss,
    ] {
        let (hint, dossier) = enemy_text(k);
        rows.push(Row {
            key: Key::Enemy(k),
            tab: Tab::Bestiary,
            group: ORDINARY,
            name: enemy_name(k),
            hint,
            dossier,
        });
    }
    // The gilded note: not a kind of its own, but the player meets it.
    let (hint, dossier) = object_text(ObjectKind::Elite);
    rows.push(Row {
        key: Key::Object(ObjectKind::Elite),
        tab: Tab::Bestiary,
        group: "ELITES",
        name: object_name(ObjectKind::Elite),
        hint,
        dossier,
    });
    const APPOINTMENTS: &str = "NAMED MINIBOSSES";
    for k in [
        MinibossKind::Corkscrew,
        MinibossKind::SisterSnip,
        MinibossKind::Bobbin,
        MinibossKind::BellDiver,
        MinibossKind::Foreman,
        MinibossKind::MilkCarton,
    ] {
        let (hint, dossier) = miniboss_text(k);
        rows.push(Row {
            key: Key::Miniboss(k),
            tab: Tab::Bestiary,
            group: APPOINTMENTS,
            name: k.name(),
            hint,
            dossier,
        });
    }
    const BOSSES: &str = "FLOOR BOSSES";
    for k in [
        BossKind::Cat,
        BossKind::Broom,
        BossKind::Mower,
        BossKind::Mangle,
        BossKind::NightEdition,
        BossKind::Magpie,
        BossKind::HorseshoeCrab,
        BossKind::MegaRat,
        BossKind::Hawk,
        BossKind::Cobra,
    ] {
        let (hint, dossier) = boss_text(k);
        rows.push(Row {
            key: Key::Boss(k),
            tab: Tab::Bestiary,
            group: BOSSES,
            name: boss_name(k),
            hint,
            dossier,
        });
    }

    // ---- ARSENAL --------------------------------------------------------
    const WEAPONS: &str = "WEAPONS";
    for k in WeaponKind::all() {
        let (hint, dossier) = weapon_text(k);
        rows.push(Row {
            key: Key::Weapon(k),
            tab: Tab::Arsenal,
            group: WEAPONS,
            name: k.name(),
            hint,
            dossier,
        });
    }
    const EVOLUTIONS: &str = "CHEST EVOLUTIONS";
    for k in WeaponKind::all() {
        if k.evolution().is_none() {
            continue;
        }
        let (hint, dossier) = evolution_text(k);
        rows.push(Row {
            key: Key::Evolution(k),
            tab: Tab::Arsenal,
            group: EVOLUTIONS,
            name: k.evolution().map(|e| e.name).unwrap_or(""),
            hint,
            dossier,
        });
    }
    const PASSIVES: &str = "PASSIVES";
    for k in PassiveKind::all() {
        let (hint, dossier) = passive_text(k);
        rows.push(Row {
            key: Key::Passive(k),
            tab: Tab::Arsenal,
            group: PASSIVES,
            name: k.name(),
            hint,
            dossier,
        });
    }
    const SPLICES: &str = "GENE SPLICES";
    for k in SpliceKind::ALL {
        let (hint, dossier) = splice_text(k);
        rows.push(Row {
            key: Key::Splice(k),
            tab: Tab::Arsenal,
            group: SPLICES,
            name: k.name(),
            hint,
            dossier,
        });
    }

    // ---- FIELD GUIDE ----------------------------------------------------
    const FINDS: &str = "POCKET FINDS";
    for k in [
        PickupKind::Heal,
        PickupKind::Magnet,
        PickupKind::Nuke,
        PickupKind::BottleCap,
        PickupKind::WindupMouse,
        PickupKind::PocketWatch,
        PickupKind::CleanSocks,
    ] {
        let (hint, dossier) = pickup_text(k);
        rows.push(Row {
            key: Key::Pickup(k),
            tab: Tab::FieldGuide,
            group: FINDS,
            name: pickup_name(k),
            hint,
            dossier,
        });
    }
    for k in [ObjectKind::Gold, ObjectKind::Chest] {
        let (hint, dossier) = object_text(k);
        rows.push(Row {
            key: Key::Object(k),
            tab: Tab::FieldGuide,
            group: FINDS,
            name: object_name(k),
            hint,
            dossier,
        });
    }
    const TRAPS: &str = "TRAPS";
    for k in [ObjectKind::StickyBoard, ObjectKind::Mousetrap] {
        let (hint, dossier) = object_text(k);
        rows.push(Row {
            key: Key::Object(k),
            tab: Tab::FieldGuide,
            group: TRAPS,
            name: object_name(k),
            hint,
            dossier,
        });
    }
    const HAZARDS: &str = "HAZARDS";
    for k in [
        ObjectKind::SteamGrate,
        ObjectKind::InkPad,
        ObjectKind::Pit,
        ObjectKind::SlidingCargo,
        ObjectKind::Slipstream,
    ] {
        let (hint, dossier) = object_text(k);
        rows.push(Row {
            key: Key::Object(k),
            tab: Tab::FieldGuide,
            group: HAZARDS,
            name: object_name(k),
            hint,
            dossier,
        });
    }
    const FEATURES: &str = "LEVEL FEATURES";
    for k in [
        ObjectKind::RatHole,
        ObjectKind::ShopHole,
        ObjectKind::MazeCache,
        ObjectKind::OneWayDoor,
    ] {
        let (hint, dossier) = object_text(k);
        rows.push(Row {
            key: Key::Object(k),
            tab: Tab::FieldGuide,
            group: FEATURES,
            name: object_name(k),
            hint,
            dossier,
        });
    }
    const OBSTACLES: &str = "OBSTACLES";
    for k in PROP_KINDS {
        let (hint, dossier) = prop_text(k);
        rows.push(Row {
            key: Key::Prop(k),
            tab: Tab::FieldGuide,
            group: OBSTACLES,
            name: prop_name(k),
            hint,
            dossier,
        });
    }

    // ---- FLOORS ---------------------------------------------------------
    const FLOOR: &str = "THE ROUTE";
    for t in Theme::playable() {
        let (hint, dossier) = floor_text(t);
        rows.push(Row {
            key: Key::Floor(t),
            tab: Tab::Floors,
            group: FLOOR,
            name: t.name(),
            hint,
            dossier,
        });
    }
    rows
}

// ---- Authored words --------------------------------------------------------

fn enemy_text(k: EnemyKind) -> (&'static str, &'static str) {
    match k {
        EnemyKind::Skitterer => (
            "Sewer · the first minute",
            "The baseline: a brown smear with legs and an opinion about your ankles. \
             It has never been fed and it has never been fast. Everything else in this \
             facility is an escalation of this.",
        ),
        EnemyKind::Bat => (
            "From the first minute",
            "Winged, purple and ceiling-bound. It does not walk, so the floor's traps, \
             ink and glue mean nothing to it. Learn to look up before the room fills \
             and you stop noticing them overhead.",
        ),
        EnemyKind::Cat => (
            "From about 1:15",
            "A cat, wearing the smug authority of a facility pet that was never told the \
             protocol had engaged. It is faster than it looks. It will not follow you \
             into a hole, and it will remember exactly where you went in.",
        ),
        EnemyKind::Spitter => (
            "From about 3:35",
            "A recycled sprayer head with a metabolism. It holds the middle distance and \
             spits pellets at it: four alive at a time, no more. Close the gap or walk \
             sideways; either is cheaper than standing still.",
        ),
        EnemyKind::Brute => (
            "From about 6:35",
            "The floor's first real wall. Slow, enormous, and built to keep walking while \
             you are shooting it. You cannot out-trade a brute in the open. Kill it while \
             it is still crossing a doorway.",
        ),
        EnemyKind::Pillbug => (
            "From about 2:00",
            "It brought its own door. Curls at arm's length and shows the silver shell: \
             a quarter of your damage goes in from the front, double goes in from behind. \
             Loop past its shoulder, bite the seam, and never linger at its face.",
        ),
        EnemyKind::BellFlea => (
            "From about 4:30",
            "Even the echo has feet. Once every few seconds it rings: a hop out along a \
             locked line, then a hop home. Both landing crosses show before they land. \
             Step off the first, leave the second empty.",
        ),
        EnemyKind::WickMoth => (
            "From about 7:15",
            "It dies with the light on. The corpse burns for a moment and then burns on \
             purpose, and the burst does not care whose side the light is on. Lure the \
             pack across the wick and step back.",
        ),
        EnemyKind::Porter => (
            "From about 8:15",
            "One lump for medicinal purposes. It carries a sugar cube to the nearest \
             wounded body and patches it up while you watch. Any hit knocks the cube out \
             of the mix. Move toward the carrier, not the crowd.",
        ),
        EnemyKind::Boss => (
            "The boss fight · every floor",
            "The horde's own boss body, from before the facility gave its wardens names \
             and phases. It is the same fight the floor's own card describes, wearing \
             the generic label.",
        ),
        EnemyKind::Turnkey => (
            "From about 9:45",
            "Nobody asked who locked the hallway. Every few seconds it lays a live \
             crossbar across the floor it has just walked, after a dotted tell. Bait the \
             bar onto spent ground, then change lanes.",
        ),
        EnemyKind::HouseCentipede => (
            "From about 4:00",
            "Enough feet for a committee. It runs the skirting board, flutters in place \
             behind a dotted line, and then is somewhere else. Touch it and it bolts for \
             the nearest wall. Meet it in the open or not at all.",
        ),
        EnemyKind::Lilliputian => (
            "From about 5:30",
            "Ten very small people with a plan. Spears level, archers kneel, and the \
             rope crew ties you to the floor while they do. The line is thick in front \
             and thin behind. Thin the squad and what is left runs for the walls.",
        ),
        EnemyKind::TermiteHorde => (
            "From about 6:00",
            "One appetite, many mouths. A single dart only ever takes a nibble out of \
             it, a hard hit splits it in two, and anything left alone grows back. Bring \
             something that hits an area, and finish what you start.",
        ),
        EnemyKind::MurderHornet => (
            "From about 7:30",
            "The buzz climbs, a line locks, and it comes down the line. A miss puts it \
             on the floor for a long second, where it takes half again as much. Step \
             off the stripe, then turn round.",
        ),
        EnemyKind::BigSnake => (
            "From about 8:30",
            "A corridor that moves. The body divides the room and the head rears along \
             a marked line before it lunges, then lies stretched out and regrets it. \
             Go round the tail, or dash across, and bite the head while it rests.",
        ),
    }
}

fn miniboss_text(k: MinibossKind) -> (&'static str, &'static str) {
    match k {
        MinibossKind::Corkscrew => (
            "Named miniboss · between 3:00 and 12:00",
            "The drain has grown a tooth. A brass auger under a pink worm, it fills a \
             trench from its tail to its tip and then the trench is teeth. Step sideways \
             out of the lane and bite the exposed head while it works.",
        ),
        MinibossKind::SisterSnip => (
            "Named miniboss · between 3:00 and 12:00",
            "She trims loose ends. The shears open over a wedge, hold the shape long \
             enough to be read, and close once. Walk behind the pivot and the whole \
             attack is behind you.",
        ),
        MinibossKind::Bobbin => (
            "Named miniboss · between 3:00 and 12:00",
            "It learned to sew in circles. A spindle rat pays out a cord to a fixed spool \
             and then swings the whole length through a quarter turn. Cross behind the \
             moving cord, not in front of it.",
        ),
        MinibossKind::BellDiver => (
            "Named miniboss · between 3:00 and 12:00",
            "Listen for the missing note. Its ring expands all the way out to arm's \
             length with one chipped gap in the rim, and the gap is never the side you \
             are standing on. Line yourself up with the chip.",
        ),
        MinibossKind::Foreman => (
            "Named miniboss · between 3:00 and 12:00",
            "Denied in triplicate. It walks your last heading and stamps three squares \
             along it, one after the other. The first mark is an invitation: turn the \
             corner and let the rest land on nothing.",
        ),
        MinibossKind::MilkCarton => (
            "Named miniboss · between 3:00 and 12:00",
            "Three weeks past instructions. It tilts on its printed corners and spills \
             one long rectangle out of the spout. Circle around the folded seam while \
             the puddle decides where it lives.",
        ),
    }
}

fn boss_text(k: BossKind) -> (&'static str, &'static str) {
    match k {
        BossKind::Cat => (
            "Sewer · 15:00 — one of two bosses",
            "The facility's mascot, grown to the size of its reputation. It crouches, \
             marks the floor where it means to land, and then lands there. Everything \
             the jaguar teaches is the same lesson the rest of the route repeats: read \
             the mark, leave it empty, punish the recovery.",
        ),
        BossKind::Broom => (
            "Kitchen · 15:00 — one of two bosses",
            "Custody, with feet. The sweeper marches, stomps a shadowed foot and drags \
             its whole reach round in an arc. It always leaves a safe pocket at its \
             heel; the trick is being in it when the bristles arrive.",
        ),
        BossKind::Mower => (
            "Lab · 15:00 — one of two bosses",
            "A laboratory mower that treated the specimen floor as a lawn. It scans, \
             it charges in a straight line, and a charge that misses the rat meets a \
             wall instead. A missed attack is your whole opening: step off the line and \
             empty the slot.",
        ),
        BossKind::Mangle => (
            "Laundry · 15:00 — the boss",
            "Thirty years of other people's whites, and a temper. The rollers turn \
             backwards to drag you into the nip, and later flick rolled towels along a \
             fixed rail. Its crank is the loose part, and loose parts take extra \
             damage.",
        ),
        BossKind::NightEdition => (
            "Printworks · 15:00 — the boss",
            "Tomorrow's front page, walking. It outlines a page where you stand, leaves \
             one blank margin, and stamps the inked part. Later it sweeps a carriage \
             rule across the floor. Stand on the unprinted stripe and let the edition \
             come out wrong.",
        ),
        BossKind::Cobra => (
            "Lab · 15:00 — one of two bosses",
            "An escaped specimen twenty-two feet long in a room eleven wide, so the \
             coils are the room's walls. Step off the marked strike line and hit the \
             spectacle mark on the open hood; leave the loop through its one gap, or \
             dash over the coil; and when it lays the arena out as a spiral lane, run \
             it — the head is spent at the end of it.",
        ),
        BossKind::Magpie => (
            "Flatbed · the getaway — one of two bosses",
            "The bird that has been riding the slipstream since the last weigh station, \
             and has decided the cab is its nest. It pecks, it strafes the deck, and it \
             beats its wings against the trailer's own wind. Everything loose on the \
             deck, the rat included, is going tailward.",
        ),
        BossKind::HorseshoeCrab => (
            "Sewer · 15:00 — one of two bosses",
            "It was here before the pipes were. A dome with a spike behind it: shots \
             from the front spark off and are wasted. It sweeps the tail, bulldozes a \
             marked lane into the nearest wall, and digs. Every miss shows you the \
             gills. Get behind it and stay there.",
        ),
        BossKind::MegaRat => (
            "Kitchen · 15:00 — one of two bosses",
            "The facility's first success, collared and tagged, and never let out. It \
             fights with your kit: the bite, the tail, the squeak, the dash, each one \
             larger, slower and marked on the floor first. Every move it misses is a \
             window. It has won this fight before.",
        ),
        BossKind::Hawk => (
            "Flatbed · the getaway — one of two bosses",
            "The other bird. The magpie wants the cargo; this one wants you. High up it \
             cannot be touched at all: watch the shadow crossing the planks, step off \
             the talon print, and be there when it buries its feet in the oak. When it \
             stoops to hover it is finally in reach, and it knows it: darts, a marked \
             grab and a talon rake down the boards.",
        ),
    }
}

fn weapon_text(k: WeaponKind) -> (&'static str, &'static str) {
    match k {
        WeaponKind::Gnaw => (
            "Level-up card · the starting weapon",
            "The original solution to every problem the facility has. One dart, aimed at \
             whatever body is nearest, forever. It scales politely and never needs a \
             second thought — which is exactly why it evolves into something with a \
             return trip.",
        ),
        WeaponKind::CheeseSpray => (
            "Level-up card · any floor",
            "Weaponised dairy, in a forward arc, because the rat cannot aim and does not \
             intend to learn. Best at exactly the range you are already fleeing through.",
        ),
        WeaponKind::SqueakNova => (
            "Level-up card · any floor",
            "A scream pitched past hearing, thrown in every direction at once. The \
             answer to being surrounded, which the facility arranges regularly.",
        ),
        WeaponKind::SporeOrbit => (
            "Level-up card · any floor",
            "Fungal hitchhikers from Vat 9, orbiting the rat on a schedule nobody \
             agreed to. They shred what walks in and then leave, which is the whole \
             appeal.",
        ),
        WeaponKind::TailWhip => (
            "Level-up card · any floor",
            "Forty grams of pure indignation, delivered in a ring at arm's length. No \
             aiming, no travel time, no mercy for anything idiots chose to stand next \
             to you.",
        ),
        WeaponKind::AcidSpit => (
            "Level-up card · any floor",
            "Stomach contents, redirected. The lob arcs over cover and leaves a pool \
             that keeps working after you have left. The facility has no opinion on \
             whose floor it is.",
        ),
        WeaponKind::TailThrash => (
            "Level-up card · any floor",
            "The tail remembered every boot. Held as a charge, spent in a long sweeping \
             arc that shoves the room away from you. Space fires it; the HUD gauge is \
             the promise it will be ready again.",
        ),
        WeaponKind::SonicSqueak => (
            "Level-up card · any floor",
            "Past hearing, straight into the skull. An expanding ring that stuns what it \
             touches, so the horde that was winning the argument has to stand still and \
             think about it.",
        ),
        WeaponKind::LaserEyes => (
            "Level-up card · any floor",
            "Please look directly at the sensor. This is the one weapon the facility \
             holds that is genuinely its own: a fan of pulses that sweeps across the \
             rat's facing and pierces everything the rays reach.",
        ),
        WeaponKind::SuperDash => (
            "Level-up card · any floor",
            "Exit wound, entrance wound, exit. There is no projectile: the dash itself \
             cuts, once per body, along the ground the rat actually covers.",
        ),
        WeaponKind::AcidRain => (
            "Level-up card · any floor",
            "There was a ceiling here once. The volley marks separate bodies and calls \
             the drops down onto the marks, staggered, so the splash lands where the \
             rat already wanted the fight.",
        ),
        WeaponKind::Hantavirus => (
            "Level-up card · any floor",
            "Somebody passed a note in Block C. A fictional arcade status, applied as \
             three ticks, that jumps to a fresh body the moment its host stops being \
             one.",
        ),
        WeaponKind::SpikedTail => (
            "Level-up card · any floor",
            "The tail grew a point of view. One lash at the nearest body in reach, and \
             a small burst of spikes where it lands. Walls stop it. It is a tail, not a \
             bullet.",
        ),
    }
}

fn evolution_text(k: WeaponKind) -> (&'static str, &'static str) {
    match k {
        WeaponKind::Gnaw => (
            "Chest · Gnaw L5 + Sharp Teeth",
            "The dart lodges instead of dying, waits a beat, and then pulls back to \
             wherever the rat has got to, biting on the way home. Nothing is wasted; \
             everything comes back in.",
        ),
        WeaponKind::CheeseSpray => (
            "Chest · Cheese Spray L5 + Fat Reserves",
            "The two outer darts plant into the floor and run a cheese string between \
             them. The string does not care who crosses it. Dinner is a tripwire.",
        ),
        WeaponKind::SqueakNova => (
            "Chest · Squeak Nova L5 + Frenzy",
            "Spokes that find a wall come home along the line they left by. One \
             reflection each, no stacking. The walls know the chorus.",
        ),
        WeaponKind::SporeOrbit => (
            "Chest · Spore Orbit L5 + Resilience",
            "Every second cast leaves its satellites behind to stand still and burn. \
             The ring is a decision about ground, not a missile. Something grew where \
             you stood.",
        ),
        WeaponKind::TailWhip => (
            "Chest · Tail Whip L5 + Quick Paws",
            "The whip hooks the nearest ordinary body and swings it a quarter turn \
             through everything beside you, then lets go. Heavy targets just take the \
             hit. Give the room a turn.",
        ),
        WeaponKind::AcidSpit => (
            "Chest · Acid Spit L5 + Thick Hide",
            "The splash becomes a channel that runs forward through the room, then \
             lingers and ticks along its own stripe. It stops at masonry. Clears hair, \
             grease and witnesses.",
        ),
        WeaponKind::TailThrash => (
            "Chest · Tail Thrash L5 + Counterweight",
            "Each sweep catches hostile pellets out of the air, and the last sweep \
             throws them back the way they came. Save that for the return trip.",
        ),
        WeaponKind::SonicSqueak => (
            "Chest · Sonic Squeak L5 + Whiskers",
            "The ring tags what it stuns or slows, and a moment later every tagged body \
             answers with a pulse of its own. Capped so a crowd cannot cascade. Every \
             cage answers.",
        ),
        WeaponKind::LaserEyes => (
            "Chest · Laser Eyes L5 + Lens Cap",
            "Each completed sweep is recorded, and the next one replays the old sweep \
             from the old place, backwards, at half weight. One copy, kept. The camera \
             kept a negative.",
        ),
        WeaponKind::SuperDash => (
            "Chest · Super Dash L5 + Spring Heel",
            "The dash's route stays taut for a moment after the rat has left it, and \
             then snaps. Bodies on the cord take a dash hit and slide back. The tail \
             was the fuse.",
        ),
        WeaponKind::AcidRain => (
            "Chest · Acid Rain L5 + Long Fuse",
            "The volley becomes one gutter: the first mark is struck, then each drop \
             steps back toward the rat along the same line, leaving runoff. The leak has \
             found the stairs.",
        ),
        WeaponKind::Hantavirus => (
            "Chest · Hantavirus L5 + Second Scent",
            "A host that dies with jumps left leaves sealed post for two seconds. The \
             first body to touch it resumes the chain. Return to sender.",
        ),
        WeaponKind::SpikedTail => (
            "Chest · Spiked Tail L5 + Counterweight",
            "The lash hooks, and the recoil drags the catch home, bursting on every \
             body it crosses on the way. Heavy bodies stay where they are. Whatever it \
             catches comes home.",
        ),
    }
}

fn passive_text(k: PassiveKind) -> (&'static str, &'static str) {
    match k {
        PassiveKind::MaxHp => (
            "Level-up card · any floor",
            "Winter is always coming. Every stack is more body to lose and, for a \
             moment, the body back again. It is the passive that makes every other \
             mistake survivable.",
        ),
        PassiveKind::MoveSpeed => (
            "Level-up card · any floor",
            "Twitchier than the cameras. The facility's corridors were built for people \
             with longer legs and slower plans.",
        ),
        PassiveKind::Damage => (
            "Level-up card · any floor",
            "Filed sharp on rebar. It is the partner most weapons are waiting for, and \
             the shortest path from a build to an evolution.",
        ),
        PassiveKind::FireRate => (
            "Level-up card · any floor",
            "More is, itself, a strategy. Every stack shortens every cooldown in the \
             rack, including the charged weapons' panic buttons.",
        ),
        PassiveKind::Magnet => (
            "Level-up card · any floor",
            "Feel the room before you enter it. Gems come to the rat instead of the rat \
             walking back for them, which in a horde is the difference.",
        ),
        PassiveKind::Regen => (
            "Level-up card · any floor",
            "Bred to survive worse than this. Slow, permanent, and worth more the longer \
             the floor runs.",
        ),
        PassiveKind::Armor => (
            "Level-up card · any floor",
            "Scar tissue, all the way down. Flat damage off every hit — contact, pellet, \
             plume or stamp — until the small stuff stops mattering.",
        ),
        PassiveKind::SpringHeel => (
            "Level-up card · any floor",
            "Smuggled out one spring at a time. Every stack takes a slice off the dash's \
             recharge, until the escape is simply part of the walk.",
        ),
        PassiveKind::Counterweight => (
            "Level-up card · any floor",
            "A washer tied to the tip. Melee gets a shove it never had and weapons that \
             already shove shove harder.",
        ),
        PassiveKind::LongFuse => (
            "Level-up card · any floor",
            "Let it simmer. Lingering zones stay longer and splash a little wider, which \
             is where the acid and the spores were always trying to get to.",
        ),
        PassiveKind::SecondScent => (
            "Level-up card · any floor",
            "There's a second crumb under the first. Gems pay a little more, and the \
             fractions bank instead of rounding away.",
        ),
        PassiveKind::LensCap => (
            "Level-up card · any floor",
            "Cut from the bottom of a bottle. Ranged weapons reach further and melee \
             reaches far enough — the cheapest way to widen every attack at once.",
        ),
    }
}

fn splice_text(k: SpliceKind) -> (&'static str, &'static str) {
    match k {
        SpliceKind::GlassIncisors => (
            "Mouse-hole shop · under the spoon",
            "A bargain, like every splice: the bite gets sharper and the body gets more \
             breakable. Handle by the roots.",
        ),
        SpliceKind::BoilerHeart => (
            "Mouse-hole shop · under the spoon",
            "Something warm and industrial replaces something soft and yours. It heals \
             you constantly and slows you the whole time. Keeps warm in cold pipes.",
        ),
        SpliceKind::HollowBones => (
            "Mouse-hole shop · under the spoon",
            "Light, fast and fragile. Every hit arrives heavier than it should because \
             there is less of you to stop it. Mind the landing.",
        ),
        SpliceKind::DoubleLid => (
            "Mouse-hole shop · under the spoon",
            "A second eyelid, and with it a longer blink of immunity after every hit. \
             The trade is paid in rate of fire. Blink once for each world.",
        ),
        SpliceKind::HoardGland => (
            "Mouse-hole shop · under the spoon",
            "A cheek pouch with ambitions. Coins are worth more (and the till keeps \
             half), and the weapons pay for it. The left cheek is for rent.",
        ),
        SpliceKind::CopperNerves => (
            "Mouse-hole shop · under the spoon",
            "Wire runs where nerves used to. The dash comes back faster; everything \
             comes to you more slowly. Ground before handling.",
        ),
        SpliceKind::NightShift => (
            "Mouse-hole shop · under the spoon",
            "The rat stops sleeping and starts firing. Passive healing is halved, so \
             the shift has to be survived on armour and nerve. Sleep was the control \
             group.",
        ),
        SpliceKind::BlackboardBelly => (
            "Mouse-hole shop · under the spoon",
            "A notebook where a stomach was. Every gem teaches more, and every patch-up \
             teaches less. Everything goes in the notebook.",
        ),
        SpliceKind::BallastSac => (
            "Mouse-hole shop · under the spoon",
            "A little sand steadies the hand. Flat armour, and every escape is bought a \
             fraction slower.",
        ),
        SpliceKind::LongFingers => (
            "Mouse-hole shop · under the spoon",
            "Reach beyond the rack. The pickup radius grows enormously and the body \
             shrinks to compensate. Someone has to reach the shelf.",
        ),
    }
}

fn pickup_text(k: PickupKind) -> (&'static str, &'static str) {
    match k {
        PickupKind::Heal => (
            "Rare drop · any floor",
            "Facility-issue medkit: a third of the rat's maximum back, instantly, and \
             nothing else. The only find that is never wasted.",
        ),
        PickupKind::Magnet => (
            "Rare drop · any floor",
            "Every gem and every coin on the floor stops waiting and starts travelling. \
             Collecting is for rats with time.",
        ),
        PickupKind::Nuke => (
            "Rare drop · any floor",
            "Sector purge. The ordinary horde dies where it stands and the named \
             minibosses take a fixed chunk instead, because a promise is a promise.",
        ),
        PickupKind::BottleCap => (
            "Rare drop · any floor",
            "Good for one boot. A tin dome over the rat's shoulders blocks the next \
             damaging hit entirely, then the usual i-frames take over. The rim shows \
             while the charge holds.",
        ),
        PickupKind::WindupMouse => (
            "Rare drop · any floor",
            "The cheapest member of the team. A brass key spins on the floor and the \
             ordinary horde walks at it instead of at the rat; elites and minibosses \
             keep their targets.",
        ),
        PickupKind::PocketWatch => (
            "Rare drop · any floor",
            "Borrow four seconds. Hostile pellets crawl while the hands turn; telegraphs \
             and named attacks keep their real clocks.",
        ),
        PickupKind::CleanSocks => (
            "Rare drop · any floor",
            "Stolen from the lost and found. The dash comes back immediately and glue \
             boards refuse to hold the rat for as long as the socks hold out.",
        ),
    }
}

fn object_text(k: ObjectKind) -> (&'static str, &'static str) {
    match k {
        ObjectKind::Elite => (
            "Any floor · the horde, occasionally",
            "Not a kind, a promotion. Every so often one ordinary body is gilded: three \
             times the gem value, gilded contact, a coin purse that is always paid. It \
             is the same enemy you already know with a better accountant.",
        ),
        ObjectKind::Gold => (
            "Any kill · a seeded glint on the floor",
            "The facility's own currency, still in the pockets of the people who used to \
             work here. Coins magnetise like gems and a mouse-hole will take them off \
             you for something permanent.",
        ),
        ObjectKind::Chest => (
            "Every named miniboss · touch to open",
            "Salvage left by whoever the miniboss used to be. It pays an evolution if \
             the rack can offer one, otherwise a free rank, otherwise a purse and a \
             patch-up.",
        ),
        ObjectKind::StickyBoard => (
            "Any floor · the bait beside a doorway",
            "A glue board from the pest budget. It does not block; it holds, and while it \
             holds you are simply not moving while the room arrives. It grips the horde \
             too, which is the only reason it is ever welcome.",
        ),
        ObjectKind::Mousetrap => (
            "Any floor · the snap on the approach",
            "A spring-loaded bar. It does not kill the rat and it kills nearly everything \
             else: a body on the bar simply stops being a body. The red-barred ones are \
             filed to a hair trigger: no warning, and a heavier bar. Read the plate and \
             step around it.",
        ),
        ObjectKind::SteamGrate => (
            "Laundry · the washer aisles",
            "A boiler vent under a brass plate. Three puffs, then one cell of scalding \
             steam, then a long rest. It cooks pursuers as readily as rats, so a side \
             lane is not always slower.",
        ),
        ObjectKind::InkPad => (
            "Printworks · the press corners",
            "Permanent tacky ink across the floor: ground movement drags for the rat and \
             for everything chasing it. Flight is unaffected and a dash crosses without \
             touching it.",
        ),
        ObjectKind::Pit => (
            "Flatbed · a hole in the deck",
            "The road, at a hundred kilometres an hour, one plank below your feet. It \
             takes a bite out of whatever falls through and puts it back on solid wood; \
             the horde loses a third of itself to it.",
        ),
        ObjectKind::SlidingCargo => (
            "Flatbed · a crate on ratchet straps",
            "It walks along its straps whenever the rig hits an expansion joint. The rat \
             is at the wrong height for it; the crate does not move for anything the \
             crate cannot see.",
        ),
        ObjectKind::Slipstream => (
            "Flatbed · the windy stretches",
            "The flatbed's own weather. Light bodies and loose gems slide tailward, which \
             is exactly the destination the rat does not want.",
        ),
        ObjectKind::RatHole => (
            "Any floor · the evidence that this place was built for someone else",
            "A gap in a wall that the facility's own designer cut for the rat. Small \
             bodies squeeze through; the cat, the brute and the pillbug take the long \
             way. Crossing one reveals its far mouth on the minimap.",
        ),
        ObjectKind::ShopHole => (
            "Any floor · two per level, under a bent spoon",
            "Under the spoon, the cameras have a blind spot. Someone keeps a shelf of \
             gene splices in the wall and takes gold for the kind of changes you cannot \
             take back.",
        ),
        ObjectKind::MazeCache => (
            "Any floor · the end of the maze",
            "A pedestal at the centre of a maze, and the maze is the price. One cache \
             per maze; the walls inside are the facility's oldest security system and it \
             still works.",
        ),
        ObjectKind::OneWayDoor => (
            "Any floor · the maze exits",
            "A steel door that has one allowed direction. It opens for the rat going out \
             and refuses the rat going back in, and it does not care at all about \
             anything following you.",
        ),
    }
}

fn prop_text(k: PropKind) -> (&'static str, &'static str) {
    match k {
        PropKind::PipeV => (
            "Sewer",
            "Standing pipe. Solid, knee-high and exactly the shape the flow field routes \
             around: use it to break a charge, not to hide behind.",
        ),
        PropKind::PipeH => ("Sewer", "A horizontal pipe run at rat height. It stops ground bodies and nothing else."),
        PropKind::Valve => ("Sewer", "A valve wheel. Round, bolted, and in the way of everything."),
        PropKind::Grate => (
            "Sewer",
            "A floor grate. It is not a hole and it is not a hazard, whatever the rest of \
             the floor has trained you to expect.",
        ),
        PropKind::Table => ("Kitchen", "A prep table in the facility's preferred material: scratched steel."),
        PropKind::Crate => ("Kitchen", "A crate, stacked and abandoned. Cover, until the brute walks through it."),
        PropKind::Barrel => ("Kitchen", "A barrel with the lid still on. Nobody checked what happens if you open it."),
        PropKind::Counter => ("Kitchen", "Serving counter. Long, low and a reliable way to lose a straight line."),
        PropKind::Console => ("Lab", "A console desk with the specimen cameras on it. Nobody is watching them."),
        PropKind::Tank => ("Lab", "A specimen tank. Whatever is in it is not one of yours."),
        PropKind::Barrier => ("Lab", "Blast barrier. The lab's own contingency against something larger than a rat."),
        PropKind::Washer => ("Laundry", "A drum washer with a porthole. It is exactly as loud as it looks."),
        PropKind::TowelRoll => ("Laundry", "A rolled towel on end. Soft, immovable, and oddly the best cover on the floor."),
        PropKind::Clothespin => ("Laundry", "A wooden clothespin the size of a chair. Leaning, patiently, on a wall."),
        PropKind::PressIsland => ("Printworks", "An offset press island, still warm."),
        PropKind::PaperReel => ("Printworks", "A standing paper reel. The printworks has a great deal of paper."),
        PropKind::TypeBlock => ("Printworks", "A reversed lead letter the size of the rat. Solid, heavy, and a good corner to fight at."),
        PropKind::Pallet => ("Flatbed", "A slat pallet on bearers. Low enough to trip over, high enough to stop a body."),
        PropKind::TarpLump => ("Flatbed", "Something under a tarp, strapped down. The road is not the only thing this rig lost."),
        PropKind::Tyre => ("Flatbed", "Spare tyres, stacked. Heavy, and the only thing on the deck the wind does not move."),
        PropKind::Toolbox => ("Flatbed", "A steel toolbox. The driver's, presumably, before the rest of this."),
        PropKind::PipeStack => ("Flatbed", "A strap-bound stack of pipes, riding out the joints."),
        PropKind::ChainCoil => ("Flatbed", "A coiled chain on a hook, swinging with the deck."),
    }
}

fn floor_text(t: Theme) -> (&'static str, &'static str) {
    match t {
        Theme::Sewer => (
            "Campaign · stage one",
            "Cold pipes and colder things. The drainage threads under the whole facility \
             and every other floor in this building sits on top of it.",
        ),
        Theme::Kitchen => (
            "Campaign · stage two",
            "Grease, snap-traps and the stink of bait. Everything here wants the rat fed \
             to something with a bigger mouth.",
        ),
        Theme::Lab => (
            "Campaign · stage three",
            "Where the rat was made. The lights hum a number that used to be its name, and \
             the blast door at the end is the front door.",
        ),
        Theme::Laundry => (
            "Side route · stage two",
            "Indigo tile, steam between the machines, and a hundred doors that will not \
             open. The whites come out pink down here.",
        ),
        Theme::Printworks => (
            "Side route · stage three",
            "Ink in the seams of your paws and the presses still running overnight. \
             Tomorrow's front page says you were caught.",
        ),
        Theme::Trailer => (
            "Getaway · practice floor",
            "Forty feet of oak deck and a hundred kilometres of night, with the road \
             roaring underneath. The cab is forward. So is the magpie.",
        ),
    }
}

// ---- Facts -----------------------------------------------------------------

/// Facts the card shows under the dossier: short label/value lines with an
/// optional five-cell bar.
/// How many facts one card prints. Nine is the widest row set in the registry
/// (a boss card: hit points, contact, phases, death, floor, **shares**, drops,
/// answer — the pooled floors' `SHARES` row used to push `ANSWER` off the card).
pub const FACT_ROWS: usize = 9;

#[derive(Clone, Debug, Default)]
pub struct Facts {
    rows: [Fact; FACT_ROWS],
    len: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Fact {
    pub label: &'static str,
    pub value: String,
    /// `0.0..=1.0` draws a five-cell bar beside the value; negative hides it.
    pub bar: f32,
}

impl Facts {
    fn push(&mut self, label: &'static str, value: impl std::fmt::Display) {
        if self.len >= FACT_ROWS {
            return;
        }
        self.rows[self.len] = Fact {
            label,
            value: value.to_string(),
            bar: -1.0,
        };
        self.len += 1;
    }

    fn push_bar(&mut self, label: &'static str, value: impl std::fmt::Display, frac: f32) {
        if self.len >= FACT_ROWS {
            return;
        }
        self.rows[self.len] = Fact {
            label,
            value: value.to_string(),
            bar: frac.clamp(0.0, 1.0),
        };
        self.len += 1;
    }

    pub fn rows(&self) -> &[Fact] {
        &self.rows[..self.len]
    }
}

/// The mechanical half of a card, read from the game's real tables.
pub fn facts(key: Key) -> Facts {
    let mut f = Facts::default();
    match key {
        Key::Enemy(k) => enemy_facts(k, &mut f),
        Key::Miniboss(k) => miniboss_facts(k, &mut f),
        Key::Boss(k) => boss_facts(k, &mut f),
        Key::Weapon(k) => weapon_facts(k, &mut f),
        Key::Evolution(k) => evolution_facts(k, &mut f),
        Key::Passive(k) => passive_facts(k, &mut f),
        Key::Splice(k) => splice_facts(k, &mut f),
        Key::Pickup(k) => pickup_facts(k, &mut f),
        Key::Prop(k) => prop_facts(k, &mut f),
        Key::Floor(t) => floor_facts(t, &mut f),
        Key::Object(k) => object_facts(k, &mut f),
    }
    f
}

fn enemy_facts(k: EnemyKind, f: &mut Facts) {
    const HP_REF: f32 = 60.0; // the brute: the top of the ordinary scale
    const SPEED_REF: f32 = 42.0; // the bat: the top of the ordinary scale
    f.push_bar(
        "HIT POINTS",
        format!("{:.0}", k.base_hp()),
        k.base_hp() / HP_REF,
    );
    f.push_bar(
        "SPEED",
        format!("{:.0} px/s", k.speed()),
        k.speed() / SPEED_REF,
    );
    f.push("CONTACT", format!("{:.0}", k.damage()));
    f.push("GEM XP", k.xp());
    let (chance, gold) = shop::gold_row(k);
    f.push(
        "COIN DROP",
        format!("{:.0}% × {} gold", chance * 100.0, gold),
    );
    f.push(
        "SIZE",
        if crate::game::art_traps::fits_through_rat_hole(k.radius()) {
            "fits through rat-holes"
        } else {
            "needs a doorway"
        },
    );
    f.push("TELL", enemy_answer(k));
}

/// One line naming the kind's tell and the answer to it (the words on the
/// card's `TELL` fact).
pub fn enemy_answer(k: EnemyKind) -> &'static str {
    match k {
        EnemyKind::Skitterer => "none — just teeth",
        EnemyKind::Bat => "none — it flies over the floor",
        EnemyKind::Cat => "fast approach — break the line with a wall",
        EnemyKind::Spitter => "stops to spit — close the gap or sidestep",
        EnemyKind::Brute => "slow wind-up — do not trade in the open",
        EnemyKind::Boss => "see the floor's own boss card",
        EnemyKind::Pillbug => "curls 1.4 s — loop behind the shell",
        EnemyKind::BellFlea => "two crosses show — leave the second empty",
        EnemyKind::WickMoth => "the corpse flares — step off the wick",
        EnemyKind::Porter => "raises the cube 0.9 s — hit the carrier",
        EnemyKind::Turnkey => "dotted bar 0.8 s — bait it, then change lanes",
        EnemyKind::BigSnake => "rears on a marked line — bite the head while it rests",
        EnemyKind::HouseCentipede => "flutters on a dotted line — step off; it flees when hit",
        EnemyKind::MurderHornet => "rising buzz, locked line — punish it on the floor",
        EnemyKind::TermiteHorde => "none — darts only nibble, bring an area hit",
        EnemyKind::Lilliputian => "spears level, archers kneel — hit the line from behind",
    }
}

fn miniboss_facts(k: MinibossKind, f: &mut Facts) {
    let last_appointment = crate::game::director::MINIBOSS_TIMES
        .len()
        .saturating_sub(1);
    let first = miniboss::appointment_hp(0);
    let last = miniboss::appointment_hp(last_appointment);
    f.push(
        "HIT POINTS",
        format!("{first:.0} at 3:00 → {last:.0} at 12:00"),
    );
    let dmg_first = miniboss::appointment_signature_damage(0);
    let dmg_last = miniboss::appointment_signature_damage(last_appointment);
    f.push(
        "SIGNATURE",
        format!("{dmg_first:.0} → {dmg_last:.0} damage"),
    );
    f.push(
        "TELL → HIT",
        format!(
            "{:.1}s tell, {:.1}s active",
            miniboss::TELL_TIME,
            k.active_duration()
        ),
    );
    f.push(
        "RECOVERY",
        format!("{:.1}s to punish", miniboss::RECOVER_TIME),
    );
    f.push(
        "COMES AT",
        crate::game::director::MINIBOSS_TIMES
            .iter()
            .map(|s| format!("{:02}:{:02}", (*s as u32) / 60, (*s as u32) % 60))
            .collect::<Vec<_>>()
            .join(" "),
    );
    f.push("DROPS", format!("a chest + {} gold", shop::MINIBOSS_GOLD));
    f.push("SIZE", "one at a time");
}

fn boss_facts(k: BossKind, f: &mut Facts) {
    const HP_REF: f32 = 3000.0;
    let hp = k.base_hp() * k.hp_scale();
    f.push_bar("HIT POINTS", format!("{hp:.0}"), hp / HP_REF);
    f.push("CONTACT", format!("{:.0}", k.contact_damage()));
    f.push("PHASES", format!("{}", k.phase_for(1.0).max(3)));
    f.push("DIES IN", format!("{:.1}s", k.death_duration()));
    f.push(
        "FLOOR",
        format!(
            "{} · at {:02}:{:02}",
            boss_theme(k).name(),
            (crate::game::director::BOSS_TIME as u32) / 60,
            (crate::game::director::BOSS_TIME as u32) % 60
        ),
    );
    // A floor with a pool fields one of its bosses per run: say so, and name
    // the other, so a player who met this one knows there is a second.
    let pool = BossKind::pool_for_theme(boss_theme(k));
    if pool.len() > 1 {
        let others: Vec<&str> = pool.iter().filter(|o| **o != k).map(|o| boss_name(*o)).collect();
        f.push(
            "SHARES",
            format!("one of {} bosses here · or {}", pool.len(), others.join(", ")),
        );
    } else {
        // Every boss card carries the row, so "this floor has one fight" is
        // said out loud instead of being inferred from a missing line.
        f.push("SHARES", "the only boss on this floor");
    }
    f.push("DROPS", format!("{} gold + the floor", shop::BOSS_GOLD));
    f.push("ANSWER", boss_answer(k));
}

fn boss_answer(k: BossKind) -> &'static str {
    match k {
        BossKind::Cat => "leave the marked landing empty",
        BossKind::Broom => "stand in the pocket at its heel",
        BossKind::Mower => "step off the charge line, punish the miss",
        BossKind::Mangle => "change sides between bites, hit the crank",
        BossKind::NightEdition => "stand on the blank margin",
        BossKind::Magpie => "cross the wind, not against it",
        BossKind::HorseshoeCrab => "get behind the dome, hit the gills",
        BossKind::MegaRat => "leave the marked shape, punish the miss",
        BossKind::Hawk => "step off the talon print, then bite the nape",
        BossKind::Cobra => "leave the loop by its gap, then hit the hood's spectacle mark",
    }
}

fn weapon_facts(k: WeaponKind, f: &mut Facts) {
    let mut damage = String::new();
    let mut cycle = String::new();
    for level in 1..=k.max_level() {
        let w = Weapon {
            kind: k,
            level,
            timer: 0.0,
            evolved: false,
        };
        if level > 1 {
            damage.push_str(" · ");
            cycle.push_str(" · ");
        }
        damage.push_str(&format!("{:.0}", w.damage()));
        cycle.push_str(&format!("{:.1}", w.cooldown()));
    }
    f.push(
        "KIND",
        if k.is_active() {
            "ACTIVE — SPACE"
        } else if matches!(k, WeaponKind::SuperDash) {
            "DASH — the dash is the attack"
        } else {
            "AUTO"
        },
    );
    f.push("DAMAGE 1→5", damage);
    f.push("CYCLE 1→5", format!("{cycle} s"));
    f.push("AT L5", format!("evolves with {}", evolution_partner(k)));
    f.push("EVOLUTION", k.evolution().map(|e| e.name).unwrap_or("none"));
    f.push(
        "RACK",
        format!("one of {} weapon slots", crate::game::loadout::MAX_WEAPONS),
    );
    f.push("EACH LEVEL", "+25% damage, 6% faster");
}

fn evolution_partner(k: WeaponKind) -> &'static str {
    k.evolution()
        .map(|e| e.passive.name())
        .unwrap_or("no partner")
}

fn evolution_facts(k: WeaponKind, f: &mut Facts) {
    let Some(e) = k.evolution() else {
        return;
    };
    f.push("WEAPON", k.name());
    f.push(
        "NEEDS",
        format!("{} L5 + one {}", k.name(), e.passive.name()),
    );
    f.push("FROM", "a miniboss chest");
    f.push("LOCKED IN", "yes — an evolution cannot be undone");
    f.push("TIMING", "keeps the weapon's own family and slot");
    f.push("DAMAGE", "credited to the original weapon");
}

fn passive_facts(k: PassiveKind, f: &mut Facts) {
    f.push("EACH STACK", k.desc());
    f.push("CAP", format!("{} stacks", k.max_stacks()));
    let partners: Vec<&'static str> = WeaponKind::all()
        .iter()
        .filter(|w| w.evolution().is_some_and(|e| e.passive == k))
        .map(|w| w.name())
        .collect();
    f.push(
        "EVOLVES",
        if partners.is_empty() {
            "no evolution — raw stats".to_string()
        } else {
            partners.join(", ")
        },
    );
    f.push(
        "RACK",
        format!(
            "one of {} passive slots",
            crate::game::loadout::MAX_PASSIVES
        ),
    );
    f.push("SHOWN ON", "the level-up card");
}

fn splice_facts(k: SpliceKind, f: &mut Facts) {
    f.push("PRICE", format!("{} gold", k.price()));
    f.push("GAIN", k.plus());
    f.push("COST", k.minus());
    f.push("LASTS", "to the end of the run");
    f.push(
        "SOCKETS",
        format!("{} (splices take neither rack)", shop::MAX_SPLICES),
    );
    f.push("BOUGHT FROM", "a mouse-hole shop");
    f.push("APPLIED", "after the passive stacks");
}

fn pickup_facts(k: PickupKind, f: &mut Facts) {
    use crate::game::ecs::systems::powerups;
    match k {
        PickupKind::Heal => {
            f.push("EFFECT", "+35% of max HP");
            f.push("LASTS", "instant");
        }
        PickupKind::Magnet => {
            f.push("EFFECT", "every gem and coin is pulled in");
            f.push("LASTS", "instant");
        }
        PickupKind::Nuke => {
            f.push("EFFECT", "the ordinary horde dies");
            f.push(
                "ON A NAMED",
                format!(
                    "takes {:.0}% of its pool",
                    crate::game::ecs::systems::loot::NUKE_NAMED_CHUNK * 100.0
                ),
            );
            f.push("LASTS", "instant");
        }
        PickupKind::BottleCap => {
            f.push("EFFECT", "blocks the next damaging hit");
            f.push("LASTS", format!("{:.0}s", powerups::BOTTLE_CAP_SECS));
        }
        PickupKind::WindupMouse => {
            f.push(
                "EFFECT",
                format!(
                    "ordinary enemies chase it within {:.0} px",
                    powerups::WINDUP_MOUSE_RADIUS
                ),
            );
            f.push("LASTS", format!("{:.0}s", powerups::WINDUP_MOUSE_SECS));
            f.push("IGNORES", "elites and bosses");
        }
        PickupKind::PocketWatch => {
            f.push(
                "EFFECT",
                format!(
                    "hostile pellets at {:.0}% speed",
                    powerups::POCKET_WATCH_BULLET_SLOW * 100.0
                ),
            );
            f.push("LASTS", format!("{:.0}s", powerups::POCKET_WATCH_SECS));
            f.push("IGNORES", "telegraphs and named attacks");
        }
        PickupKind::CleanSocks => {
            f.push("EFFECT", "dash readied, glue boards let go");
            f.push("LASTS", format!("{:.0}s", powerups::CLEAN_SOCKS_SECS));
        }
    }
    f.push("FOUND", "the rare pickup roll");
    f.push("SHOWN AS", "a solid two-tone silhouette");
}

fn prop_facts(k: PropKind, f: &mut Facts) {
    let (w, h) = k.size();
    f.push("FOOTPRINT", format!("{w:.0} × {h:.0} px"));
    f.push("BLOCKS", "everything on the ground");
    f.push("FLOOR", prop_theme(k).name());
    f.push("COVER", "yes — in both directions");
    f.push("BULLETS", "stopped by solid cover");
}

fn floor_facts(t: Theme, f: &mut Facts) {
    let boss = BossKind::for_theme(t);
    f.push(
        "STAGE",
        format!(
            "{} of {}",
            t.stage() + 1,
            crate::game::campaign::LEVEL_COUNT
        ),
    );
    f.push("BOSS", boss_name(boss));
    let appointments: Vec<String> = (0..crate::game::director::MINIBOSS_TIMES.len())
        .map(|i| {
            format!(
                "{} @ {:02}:{:02}",
                miniboss::schedule_for(t, i).name(),
                (crate::game::director::MINIBOSS_TIMES[i] as u32) / 60,
                (crate::game::director::MINIBOSS_TIMES[i] as u32) % 60
            )
        })
        .collect();
    f.push("MINIBOSSES", appointments.join(" · "));
    f.push(
        "BOSS CLOCK",
        format!(
            "{:02}:{:02}",
            (crate::game::director::BOSS_TIME as u32) / 60,
            (crate::game::director::BOSS_TIME as u32) % 60
        ),
    );
    f.push("HAZARD", floor_hazard(t));
    f.push("SHOP HOLES", "2 under the spoons");
    f.push("MAZES", "at least one per generated floor");
}

fn floor_hazard(t: Theme) -> &'static str {
    match t {
        Theme::Sewer => "sticky boards and snap-traps",
        Theme::Kitchen => "sticky boards and snap-traps",
        Theme::Lab => "sticky boards and snap-traps",
        Theme::Laundry => "steam grates",
        Theme::Printworks => "ink pads",
        Theme::Trailer => "pits, sliding cargo, slipstream",
    }
}

fn object_facts(k: ObjectKind, f: &mut Facts) {
    use crate::game::art_traps as traps;
    use crate::game::hazards;
    match k {
        ObjectKind::Elite => {
            f.push("XP", "×3 the kind's gems");
            f.push("COIN", format!("{} gold, always", shop::ELITE_GOLD));
            f.push("HP", "the kind's own pool");
            f.push("CAPPED", "elites rarely, never beside a miniboss");
        }
        ObjectKind::Gold => {
            f.push("VALUE", "1–3 by kind, ×5 elite, ×12 miniboss, ×30 boss");
            f.push("MAGNET", "yes — like gems");
            f.push("SPENT AT", "a mouse-hole shop shelf");
        }
        ObjectKind::Chest => {
            f.push("FROM", "every named miniboss");
            f.push("PAYS", "an evolution → a free rank → 24 gold");
            f.push(
                "ALSO",
                format!(
                    "a {:.0}% patch-up",
                    crate::game::loadout::CHEST_HEAL_FRAC * 100.0
                ),
            );
            f.push("COLLECT", "by touch");
        }
        ObjectKind::StickyBoard => {
            f.push(
                "HOLDS",
                format!("{:.1}s", traps::STICKY_TRAP_HOLD_PLAYER_SECS),
            );
            f.push(
                "DRAG",
                format!(
                    "{:.0}% speed while stuck",
                    traps::STICKY_TRAP_SLOW_MULT * 100.0
                ),
            );
            f.push(
                "ORGANIC",
                format!(
                    "{:.1}s on a small body",
                    traps::STICKY_TRAP_HOLD_SMALL_ENEMY_SECS
                ),
            );
            f.push("IGNORES", "Clean Socks wearers");
        }
        ObjectKind::Mousetrap => {
            f.push(
                "TO THE RAT",
                format!(
                    "{:.0}% of max health",
                    crate::game::art_traps::MOUSETRAP_DAMAGE_PLAYER_SHARE * 100.0
                ),
            );
            f.push(
                "TO A BODY",
                format!("{:.0} damage — a kill", traps::MOUSETRAP_DAMAGE_ENEMY),
            );
            // The red-barred hair trigger: no warning beat, a heavier bar.
            f.push(
                "RED BAR",
                format!(
                    "hair trigger — no delay, {:.0}% / {:.0}",
                    crate::game::art_traps::MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE * 100.0,
                    crate::game::art_traps::MOUSETRAP_DAMAGE_ENEMY_HAIR
                ),
            );
            f.push(
                "TRIGGER",
                format!("{:.2}s after the plate", traps::MOUSETRAP_TRIGGER_SECS),
            );
            f.push("SNAP", format!("{:.2}s", traps::MOUSETRAP_SNAP_SECS));
        }
        ObjectKind::SteamGrate => {
            f.push("DAMAGE", format!("{:.0}", hazards::STEAM_DAMAGE));
            f.push("WARN", format!("{:.1}s of puffs", hazards::STEAM_WARN_SECS));
            f.push("PLUME", format!("{:.1}s", hazards::STEAM_PLUME_SECS));
            f.push("REST", format!("{:.0}s", hazards::STEAM_REST_SECS));
            f.push("HURTS", "the horde too");
        }
        ObjectKind::InkPad => {
            f.push(
                "MOVEMENT",
                format!("{:.0}% speed on the ground", hazards::INK_SLOW * 100.0),
            );
            f.push("APPLIES TO", "rat and pursuers alike");
            f.push("IGNORES", "flight and dashes");
            f.push("LASTS", "the whole floor");
        }
        ObjectKind::Pit => {
            f.push(
                "TO THE RAT",
                format!(
                    "{:.0} damage, put back on a plank",
                    crate::game::ecs::systems::trailer::FALL_PLAYER_DAMAGE
                ),
            );
            f.push(
                "TO A BODY",
                format!(
                    "{:.0}% of its pool",
                    crate::game::ecs::systems::trailer::FALL_ENEMY_FRACTION * 100.0
                ),
            );
            f.push(
                "STUN",
                format!(
                    "{:.1}s",
                    crate::game::ecs::systems::trailer::FALL_ENEMY_STUN
                ),
            );
            f.push(
                "I-FRAMES",
                format!(
                    "{:.1}s",
                    crate::game::ecs::systems::trailer::FALL_PLAYER_IFRAMES
                ),
            );
        }
        ObjectKind::SlidingCargo => {
            f.push(
                "TO THE RAT",
                format!(
                    "{:.0} damage + a shove",
                    crate::game::ecs::systems::trailer::SLIDE_DAMAGE
                ),
            );
            f.push("TO A BODY", "nothing — the crate is at rat height");
            f.push("PATTERN", "creaks first, then shifts on the joint");
        }
        ObjectKind::Slipstream => {
            f.push(
                "ON BODIES",
                format!(
                    "{:.0} px/s tailward",
                    crate::game::ecs::systems::trailer::WIND_PULL_ENEMIES
                ),
            );
            f.push(
                "ON GEMS",
                format!(
                    "{:.0} px/s tailward",
                    crate::game::ecs::systems::trailer::WIND_PULL_GEMS
                ),
            );
            f.push("LIGHT ONLY", "small bodies, not the cat or the brute");
            f.push("STOPS AT", "the deck edge");
        }
        ObjectKind::RatHole => {
            f.push(
                "PASSES",
                format!(
                    "bodies up to {:.1} px radius",
                    traps::RAT_HOLE_MAX_PASS_RADIUS
                ),
            );
            f.push("REFUSES", "cats, brutes, pillbugs and up");
            f.push("SQUEEZE", format!("{:.2}s", traps::RAT_HOLE_SQUEEZE_SECS));
            f.push("MAP", "the far mouth shows once crossed");
        }
        ObjectKind::ShopHole => {
            f.push("PER FLOOR", "two, in structural walls");
            f.push("TAKES", "gold for gene splices");
            f.push(
                "SPLICES",
                format!("{} sockets, bought once each", shop::MAX_SPLICES),
            );
            f.push(
                "SERVICES",
                format!(
                    "stitch {} · reroll {} · molt {}",
                    shop::STITCH_PRICE,
                    shop::REROLL_BASE_PRICE,
                    shop::MOLT_PRICE
                ),
            );
        }
        ObjectKind::MazeCache => {
            f.push("PAYS", "a cache: gems, gold and a rank");
            f.push("COSTS", "a maze, walked in and out");
            f.push("ENTRANCE", "revealed once seen on the map");
            f.push("EXIT", "a one-way door");
        }
        ObjectKind::OneWayDoor => {
            f.push("DIRECTION", "out of the maze only");
            f.push("OPEN", format!("{:.2}s", traps::ONE_WAY_DOOR_OPEN_SECS));
            f.push("CLOSE", format!("{:.2}s", traps::ONE_WAY_DOOR_CLOSE_SECS));
            f.push("REFUSES", "trying it backwards");
        }
    }
}

// ---- Persistence -----------------------------------------------------------

/// One kind's counters. Every field is optional in the file, so an older or
/// hand-edited file loads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Levels in which the player met it (once per floor).
    #[serde(default, skip_serializing_if = "zero")]
    pub seen: u32,
    /// Kills of this kind.
    #[serde(default, skip_serializing_if = "zero")]
    pub killed: u32,
    /// Times this kind killed the rat.
    #[serde(default, skip_serializing_if = "zero")]
    pub killed_by: u32,
    /// Pickups collected (`Pickup`), or cards taken (`Weapon`/`Passive`).
    #[serde(default, skip_serializing_if = "zero")]
    pub picked: u32,
    /// Times owned at all (weapons, passives, splices).
    #[serde(default, skip_serializing_if = "zero")]
    pub owned: u32,
    /// Times worn as an evolution.
    #[serde(default, skip_serializing_if = "zero")]
    pub evolved: u32,
}

fn zero(n: &u32) -> bool {
    *n == 0
}

/// One non-zero counter, for the on-disk form.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SavedRecord {
    pub category: u8,
    pub kind: u8,
    #[serde(flatten)]
    pub record: Record,
}

/// The on-disk shape. `seen` is a fixed bitset per category; the counters are
/// sparse (only non-zero rows are written), so a fresh install has a tiny file
/// and an old one keeps loading.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LibrarySave {
    #[serde(default)]
    pub seen: [[u64; 1]; CATEGORY_COUNT],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<SavedRecord>,
}

/// Which counters a caller is bumping ([`Library::bump`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    Killed,
    KilledBy,
    Picked,
    Owned,
    Evolved,
}

/// Everything the library knows, plus the transient bookkeeping the survey
/// needs (never persisted).
#[derive(Clone, Serialize, Deserialize)]
#[serde(from = "LibrarySave", into = "LibrarySave")]
pub struct Library {
    seen: [[u64; 1]; CATEGORY_COUNT],
    stats: [[Record; MAX_KINDS]; CATEGORY_COUNT],
    /// Session-only NEW marks (cleared when the entry is viewed).
    fresh: [[u64; 1]; CATEGORY_COUNT],
    /// Per-floor marks, so `seen` counts floors rather than frames.
    run_seen: [[u64; 1]; CATEGORY_COUNT],
    /// Enemies last survey, `SpawnId`-ordered, for the kill diff; `cur` and
    /// `gone` are the retained scratch the diff reuses.
    prev: Vec<(u64, EnemyKind)>,
    cur: Vec<(u64, EnemyKind)>,
    gone: Vec<EnemyKind>,
    prev_kills: u32,
    /// Keys discovered since the App last drained them (HUD callouts).
    news: Vec<Key>,
}

impl std::fmt::Debug for Library {
    /// Deliberately terse: the fixed 64-kind counter table is not worth a
    /// dump, and the on-disk [`LibrarySave`] is the shape to inspect.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("categories", &CATEGORY_COUNT)
            .field("news", &self.news.len())
            .field("tracked", &self.prev.len())
            .finish()
    }
}

impl Default for Library {
    fn default() -> Self {
        Library {
            seen: [[0; 1]; CATEGORY_COUNT],
            stats: [[Record::default(); MAX_KINDS]; CATEGORY_COUNT],
            fresh: [[0; 1]; CATEGORY_COUNT],
            run_seen: [[0; 1]; CATEGORY_COUNT],
            prev: Vec::new(),
            cur: Vec::new(),
            gone: Vec::new(),
            prev_kills: 0,
            news: Vec::new(),
        }
    }
}

impl From<LibrarySave> for Library {
    fn from(save: LibrarySave) -> Library {
        let mut lib = Library {
            seen: save.seen,
            ..Library::default()
        };
        for row in save.stats {
            let (c, k) = (row.category as usize, row.kind as usize);
            if c < CATEGORY_COUNT && k < MAX_KINDS {
                lib.stats[c][k] = row.record;
            }
        }
        lib
    }
}

impl From<Library> for LibrarySave {
    fn from(lib: Library) -> LibrarySave {
        let mut stats = Vec::new();
        for (c, cat) in lib.stats.iter().enumerate() {
            for (k, record) in cat.iter().enumerate() {
                if *record != Record::default() {
                    stats.push(SavedRecord {
                        category: c as u8,
                        kind: k as u8,
                        record: *record,
                    });
                }
            }
        }
        LibrarySave {
            seen: lib.seen,
            stats,
        }
    }
}

impl Library {
    fn bit(&self, key: Key) -> (usize, usize, u64) {
        let cat = key.category() as usize;
        let idx = key.kind_index();
        (cat, idx, 1u64 << (idx & 63))
    }

    pub fn is_seen(&self, key: Key) -> bool {
        let (c, i, m) = self.bit(key);
        self.seen[c][i >> 6] & m != 0
    }

    /// The session's NEW mark (set on discovery, cleared when viewed).
    pub fn is_new(&self, key: Key) -> bool {
        let (c, i, m) = self.bit(key);
        self.fresh[c][i >> 6] & m != 0
    }

    /// Mark the entry as read: the NEW tag goes away (persisted nothing).
    pub fn view(&mut self, key: Key) {
        let (c, i, m) = self.bit(key);
        self.fresh[c][i >> 6] &= !m;
    }

    pub fn record(&self, key: Key) -> Record {
        let idx = key.kind_index();
        if idx >= MAX_KINDS {
            return Record::default();
        }
        self.stats[key.category() as usize][idx]
    }

    pub fn bump(&mut self, key: Key, counter: Counter, by: u32) {
        let idx = key.kind_index();
        if idx >= MAX_KINDS {
            return;
        }
        let record = &mut self.stats[key.category() as usize][idx];
        let slot = match counter {
            Counter::Killed => &mut record.killed,
            Counter::KilledBy => &mut record.killed_by,
            Counter::Picked => &mut record.picked,
            Counter::Owned => &mut record.owned,
            Counter::Evolved => &mut record.evolved,
        };
        *slot = slot.saturating_add(by);
    }

    /// Record a sighting. Returns `true` when this is the first time the key
    /// was ever discovered (the caller raises the HUD callout).
    pub fn mark_seen(&mut self, key: Key) -> bool {
        self.mark_seen_counted(key, true)
    }

    /// [`mark_seen`] without touching the `seen` counter (used for the
    /// "offered" half of a weapon/passive's discovery, which is not a fight).
    pub fn mark_seen_quiet(&mut self, key: Key) -> bool {
        self.mark_seen_counted(key, false)
    }

    fn mark_seen_counted(&mut self, key: Key, count: bool) -> bool {
        let (c, i, m) = self.bit(key);
        let first = self.seen[c][i >> 6] & m == 0;
        if first {
            self.seen[c][i >> 6] |= m;
            self.fresh[c][i >> 6] |= m;
            self.news.push(key);
        }
        if count && self.run_seen[c][i >> 6] & m == 0 {
            self.run_seen[c][i >> 6] |= m;
            let idx = key.kind_index();
            if idx < MAX_KINDS {
                let record = &mut self.stats[c][idx];
                record.seen = record.seen.saturating_add(1);
            }
        }
        first
    }

    /// A new floor: clear the per-run marks and the kill diff ("seen" counts
    /// floors, not frames).
    pub fn begin_floor(&mut self) {
        self.run_seen = [[0; 1]; CATEGORY_COUNT];
        self.prev.clear();
        self.prev_kills = 0;
    }

    /// Reveal everything (screenshots and tests; `RATRUN_LIBRARY_ALL=1`).
    pub fn reveal_all(&mut self) {
        for cat in 0..CATEGORY_COUNT {
            for kind in 0..MAX_KINDS {
                if key_at(cat, kind).is_some() {
                    self.seen[cat][kind >> 6] |= 1 << (kind & 63);
                    self.stats[cat][kind].seen = self.stats[cat][kind].seen.max(1);
                }
            }
        }
    }

    /// Read `RATRUN_LIBRARY_ALL` once: `1`/`true`/`yes` reveals the index for
    /// the session without writing it back.
    pub fn reveal_all_from_env(&mut self) {
        if std::env::var("RATRUN_LIBRARY_ALL")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
        {
            self.reveal_all();
        }
    }

    /// How many entries of a tab are discovered, out of the rows shown.
    pub fn counts(&self, rows: &[Row], tab: Tab) -> (usize, usize) {
        let mut total = 0;
        let mut seen = 0;
        for row in rows.iter().filter(|r| r.tab == tab) {
            total += 1;
            if self.is_seen(row.key) {
                seen += 1;
            }
        }
        (seen, total)
    }

    /// Drain the keys discovered since the last call into `out` (a retained
    /// buffer the App owns), so the HUD can announce them.
    pub fn drain_news(&mut self, out: &mut Vec<Key>) {
        out.append(&mut self.news);
    }
}

/// Rebuild a [`Key`] from its persisted `(category, kind)` pair, so
/// [`Library::reveal_all`] can walk the whole table.
fn key_at(category: usize, kind: usize) -> Option<Key> {
    Some(match *Category::ALL.get(category)? {
        Category::Enemy => Key::Enemy(enemy_at(kind)?),
        Category::Miniboss => Key::Miniboss(miniboss_at(kind)?),
        Category::Boss => Key::Boss(boss_at(kind)?),
        Category::Weapon => Key::Weapon(weapon_at(kind)?),
        Category::Evolution => Key::Evolution(weapon_at(kind)?),
        Category::Passive => Key::Passive(passive_at(kind)?),
        Category::Splice => Key::Splice(splice_at(kind)?),
        Category::Pickup => Key::Pickup(pickup_at(kind)?),
        Category::Prop => Key::Prop(prop_at(kind)?),
        Category::Floor => Key::Floor(floor_at(kind)?),
        Category::Object => Key::Object(object_at(kind)?),
    })
}

fn enemy_at(i: usize) -> Option<EnemyKind> {
    [
        EnemyKind::Skitterer,
        EnemyKind::Bat,
        EnemyKind::Cat,
        EnemyKind::Spitter,
        EnemyKind::Brute,
        EnemyKind::Boss,
        EnemyKind::Pillbug,
        EnemyKind::BellFlea,
        EnemyKind::WickMoth,
        EnemyKind::Porter,
        EnemyKind::Turnkey,
        EnemyKind::BigSnake,
        EnemyKind::HouseCentipede,
        EnemyKind::MurderHornet,
        EnemyKind::TermiteHorde,
        EnemyKind::Lilliputian,
    ]
    .get(i)
    .copied()
}

fn miniboss_at(i: usize) -> Option<MinibossKind> {
    [
        MinibossKind::Corkscrew,
        MinibossKind::SisterSnip,
        MinibossKind::Bobbin,
        MinibossKind::BellDiver,
        MinibossKind::Foreman,
        MinibossKind::MilkCarton,
    ]
    .get(i)
    .copied()
}

fn boss_at(i: usize) -> Option<BossKind> {
    [
        BossKind::Cat,
        BossKind::Broom,
        BossKind::Mower,
        BossKind::Mangle,
        BossKind::NightEdition,
        BossKind::Magpie,
        BossKind::HorseshoeCrab,
        BossKind::MegaRat,
        BossKind::Hawk,
        BossKind::Cobra,
    ]
    .get(i)
    .copied()
}

fn weapon_at(i: usize) -> Option<WeaponKind> {
    WeaponKind::all().get(i).copied()
}

fn passive_at(i: usize) -> Option<PassiveKind> {
    PassiveKind::all().get(i).copied()
}

fn splice_at(i: usize) -> Option<SpliceKind> {
    SpliceKind::ALL.get(i).copied()
}

fn pickup_at(i: usize) -> Option<PickupKind> {
    [
        PickupKind::Heal,
        PickupKind::Magnet,
        PickupKind::Nuke,
        PickupKind::BottleCap,
        PickupKind::WindupMouse,
        PickupKind::PocketWatch,
        PickupKind::CleanSocks,
    ]
    .get(i)
    .copied()
}

fn object_at(i: usize) -> Option<ObjectKind> {
    ObjectKind::ALL.get(i).copied()
}

fn prop_at(i: usize) -> Option<PropKind> {
    PROP_KINDS.get(i).copied()
}

fn floor_at(i: usize) -> Option<Theme> {
    Theme::playable().get(i).copied()
}

/// Every prop kind, in the order [`all_rows`] lists them.
pub const PROP_KINDS: [PropKind; 23] = [
    PropKind::PipeV,
    PropKind::PipeH,
    PropKind::Valve,
    PropKind::Grate,
    PropKind::Barrier,
    PropKind::Table,
    PropKind::Crate,
    PropKind::Barrel,
    PropKind::Counter,
    PropKind::Console,
    PropKind::Tank,
    PropKind::Washer,
    PropKind::TowelRoll,
    PropKind::Clothespin,
    PropKind::PressIsland,
    PropKind::PaperReel,
    PropKind::TypeBlock,
    PropKind::Pallet,
    PropKind::TarpLump,
    PropKind::Tyre,
    PropKind::Toolbox,
    PropKind::PipeStack,
    PropKind::ChainCoil,
];

// ---- The discovery survey --------------------------------------------------

impl Library {
    /// Walk the live world and record anything newly seen.
    ///
    /// Read-only over the world (no rng, no writes outside the library), and
    /// allocation-free: `prev` and `news` are retained buffers. The three
    /// "everything in this category is known" early-outs make a mature run a
    /// handful of bitset tests per frame.
    pub fn survey(&mut self, w: &crate::game::world::World) {
        if self.any_undiscovered(Category::Enemy) {
            self.survey_enemies(w);
        }
        if self.any_undiscovered(Category::Miniboss) || self.any_undiscovered(Category::Boss) {
            self.survey_named(w);
        }
        if self.any_undiscovered(Category::Pickup) || self.any_undiscovered(Category::Object) {
            self.survey_drops(w);
        }
        if self.any_undiscovered(Category::Object) || self.any_undiscovered(Category::Prop) {
            self.survey_level(w);
        }
        self.survey_gear(w);
        self.survey_kills(w);
    }

    fn any_undiscovered(&self, cat: Category) -> bool {
        let c = cat as usize;
        (0..WORDS).any(|word| self.seen[c][word] != u64::MAX)
    }

    fn see_if(&mut self, key: Key, visible: bool) -> bool {
        visible && self.mark_seen(key)
    }

    fn survey_enemies(&mut self, w: &crate::game::world::World) {
        for entity in w.ecs.iter_entities() {
            let Some(e) = entity.get::<crate::game::enemy::Enemy>() else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            if !self.is_seen(Key::Enemy(e.kind)) && sees(w, e.pos) {
                self.see_if(Key::Enemy(e.kind), true);
            }
            if e.elite && !self.is_seen(Key::Object(ObjectKind::Elite)) && sees(w, e.pos) {
                self.see_if(Key::Object(ObjectKind::Elite), true);
            }
        }
    }

    fn survey_named(&mut self, w: &crate::game::world::World) {
        for entity in w.ecs.iter_entities() {
            if let Some(mb) = entity.get::<miniboss::Miniboss>() {
                // The fight starts when the body stops approaching.
                let started =
                    mb.announced || !matches!(mb.state, miniboss::MinibossState::Approach);
                if started {
                    self.see_if(Key::Miniboss(mb.kind), true);
                }
            }
            if let Some(boss) = entity.get::<crate::game::ecs::systems::boss::Boss>() {
                if !boss.dead {
                    self.see_if(Key::Boss(boss.kind), true);
                }
            }
        }
        // A test-spawned legacy boss body (no `Boss` component) still counts.
        if w.boss_hp().is_some() {
            self.see_if(Key::Boss(BossKind::for_theme(w.level.theme)), true);
        }
    }

    fn survey_drops(&mut self, w: &crate::game::world::World) {
        for entity in w.ecs.iter_entities() {
            if let Some(p) = entity.get::<crate::game::gem::Pickup>() {
                if !self.is_seen(Key::Pickup(p.kind))
                    && sees(w, p.pos)
                    && (p.pos - w.player.pos).len() <= 64.0
                {
                    self.see_if(Key::Pickup(p.kind), true);
                }
            }
            if let Some(g) = entity.get::<crate::game::gem::Gold>() {
                if !self.is_seen(Key::Object(ObjectKind::Gold))
                    && sees(w, g.pos)
                    && (g.pos - w.player.pos).len() <= 64.0
                {
                    self.see_if(Key::Object(ObjectKind::Gold), true);
                }
            }
            if let Some(c) = entity.get::<crate::game::gem::Chest>() {
                if !self.is_seen(Key::Object(ObjectKind::Chest)) && sees(w, c.pos) {
                    self.see_if(Key::Object(ObjectKind::Chest), true);
                }
            }
        }
        // Coins in the purse were collected at some point.
        if w.gold > 0 && !self.is_seen(Key::Object(ObjectKind::Gold)) {
            self.see_if(Key::Object(ObjectKind::Gold), true);
        }
    }

    fn survey_level(&mut self, w: &crate::game::world::World) {
        let level = &w.level;
        for trap in &level.objects.sticky_traps {
            let key = Key::Object(ObjectKind::StickyBoard);
            if !self.is_seen(key) && sees(w, trap.pos) {
                self.see_if(key, true);
            }
        }
        for trap in &level.objects.mousetraps {
            let key = Key::Object(ObjectKind::Mousetrap);
            if !self.is_seen(key) && sees(w, trap.pos) {
                self.see_if(key, true);
            }
        }
        for hole in &level.objects.rat_holes {
            let key = Key::Object(ObjectKind::RatHole);
            if !self.is_seen(key) && sees(w, hole.view.pos) {
                self.see_if(key, true);
            }
        }
        for door in &level.objects.one_way_doors {
            let key = Key::Object(ObjectKind::OneWayDoor);
            if !self.is_seen(key) && sees(w, door.view.pos) {
                self.see_if(key, true);
            }
        }
        for reward in &level.objects.rewards {
            let key = Key::Object(ObjectKind::MazeCache);
            if !self.is_seen(key) && sees(w, reward.pos) {
                self.see_if(key, true);
            }
        }
        for hole in &level.objects.shop_holes {
            let key = Key::Object(ObjectKind::ShopHole);
            if !self.is_seen(key) && sees(w, hole.face()) {
                self.see_if(key, true);
            }
        }
        for grate in &level.objects.steam_grates {
            let key = Key::Object(ObjectKind::SteamGrate);
            if !self.is_seen(key) && sees(w, grate.pos) {
                self.see_if(key, true);
            }
        }
        for pad in &level.objects.ink_pads {
            let key = Key::Object(ObjectKind::InkPad);
            if !self.is_seen(key) && sees(w, pad.pos) {
                self.see_if(key, true);
            }
        }
        for pit in &level.trailer.pits {
            let key = Key::Object(ObjectKind::Pit);
            if !self.is_seen(key) && sees(w, pit.center_world()) {
                self.see_if(key, true);
            }
        }
        for slide in &level.trailer.slides {
            let key = Key::Object(ObjectKind::SlidingCargo);
            if !self.is_seen(key) && sees(w, slide.rest) {
                self.see_if(key, true);
            }
        }
        if !level.trailer.wind.is_empty() && !self.is_seen(Key::Object(ObjectKind::Slipstream)) {
            let rect = level.trailer.wind[0];
            if sees(w, rect.center_world()) {
                self.see_if(Key::Object(ObjectKind::Slipstream), true);
            }
        }
        for prop in &level.props {
            let key = Key::Prop(prop.kind);
            if self.is_seen(key) {
                continue;
            }
            let centre = Vec2::new(
                prop.rect.x + prop.rect.w * 0.5,
                prop.rect.y + prop.rect.h * 0.5,
            );
            if sees(w, centre) {
                self.see_if(key, true);
            }
        }
    }

    /// Gear is discovered from the rack (owned) — the "offered" half is marked
    /// by the App when it deals a level-up card.
    fn survey_gear(&mut self, w: &crate::game::world::World) {
        let loadout = &w.player.loadout;
        for weapon in &loadout.weapons {
            let key = Key::Weapon(weapon.kind);
            if self.mark_seen(key) {
                self.bump(key, Counter::Owned, 1);
            }
            if weapon.evolved {
                let evo = Key::Evolution(weapon.kind);
                self.mark_seen(evo);
                self.bump(key, Counter::Evolved, 1);
            }
        }
        for &(kind, _) in &loadout.passives {
            let key = Key::Passive(kind);
            if self.mark_seen(key) {
                self.bump(key, Counter::Owned, 1);
            }
        }
        for &kind in &loadout.splices {
            let key = Key::Splice(kind);
            if self.mark_seen(key) {
                self.bump(key, Counter::Owned, 1);
            }
        }
    }

    /// Kills, by diffing the living enemy set against last survey's (the ECS
    /// never exposes a per-kind tally). A floor that is cleared wholesale is
    /// skipped, and the count is capped by the run's own kill counter so a
    /// despawn for any other reason cannot be booked as a kill.
    fn survey_kills(&mut self, w: &crate::game::world::World) {
        use crate::game::ecs::components::SpawnId;
        let kills_now = w.kills;
        let delta = kills_now.saturating_sub(self.prev_kills);
        self.prev_kills = kills_now;
        // Retained scratch: the diff allocates nothing per frame.
        let mut current = std::mem::take(&mut self.cur);
        current.clear();
        for entity in w.ecs.iter_entities() {
            let (Some(id), Some(e)) = (
                entity.get::<SpawnId>(),
                entity.get::<crate::game::enemy::Enemy>(),
            ) else {
                continue;
            };
            if e.alive() {
                current.push((id.0, e.kind));
            }
        }
        current.sort_unstable_by_key(|(id, _)| *id);
        let mut gone = std::mem::take(&mut self.gone);
        gone.clear();
        for &(id, kind) in &self.prev {
            if current.binary_search_by_key(&id, |(cid, _)| *cid).is_err() {
                gone.push(kind);
            }
        }
        // A bulk removal is a level reset (or a test's `clear_enemies`), not a
        // pile of kills; the run's own kill counter caps the rest.
        if gone.len() <= 24 {
            for kind in gone.iter().take(delta as usize) {
                self.bump(Key::Enemy(*kind), Counter::Killed, 1);
            }
        }
        std::mem::swap(&mut self.prev, &mut current);
        self.cur = current;
        self.gone = gone;
    }

    /// The rat died: book it against the nearest body on the lethal tick (the
    /// simulation does not record a damage source, and a fall or a trap in the
    /// same frame is indistinguishable from contact — nearest is the honest
    /// approximation the card prints).
    pub fn note_death(&mut self, w: &crate::game::world::World) {
        let mut best: Option<(f32, EnemyKind)> = None;
        for entity in w.ecs.iter_entities() {
            let Some(e) = entity.get::<crate::game::enemy::Enemy>() else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            let d = (e.pos - w.player.pos).len();
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, e.kind));
            }
        }
        if let Some((_, kind)) = best {
            self.bump(Key::Enemy(kind), Counter::KilledBy, 1);
        }
    }

    /// A level-up card was dealt: the "offered" discovery for its kinds.
    pub fn note_offered(&mut self, upgrade: &crate::game::loadout::Upgrade) {
        use crate::game::loadout::Upgrade;
        match upgrade {
            Upgrade::NewWeapon(k) => {
                self.mark_seen_quiet(Key::Weapon(*k));
            }
            Upgrade::Passive(p) => {
                self.mark_seen_quiet(Key::Passive(*p));
            }
            _ => {}
        }
    }

    /// A pickup was collected.
    pub fn note_collected(&mut self, kind: PickupKind) {
        let key = Key::Pickup(kind);
        self.mark_seen(key);
        self.bump(key, Counter::Picked, 1);
    }
}

/// How much world a sighting covers: roughly a screen's worth at the default
/// view.
///
/// The rule is "on screen, inside the rat's line of sight". The line of sight
/// is the fog's own bubble ([`crate::game::fog::FOG_REVEAL_RADIUS`] cells,
/// which already stops at walls); this radius is the screen half-size it is
/// trimmed to. It deliberately does **not** read `World::cam` — the camera is
/// written by the renderer, so a discovery test (or the first frame after a
/// level loads) would depend on a draw having happened.
pub const SIGHT_REACH: f32 = 120.0;

/// Is `p` something the player can actually see right now?
fn sees(w: &crate::game::world::World, p: Vec2) -> bool {
    if (p - w.player.pos).len() > SIGHT_REACH {
        return false;
    }
    let (cx, cy) = cell_of(p);
    w.fog.in_view(cx, cy)
}

#[cfg(test)]
mod tests;
