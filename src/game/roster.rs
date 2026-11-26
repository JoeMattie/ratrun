//! One registry, one floor table: the per-floor enemy and miniboss rosters
//! (`WAVE4_ROSTERS.md`).
//!
//! Every floor owns **eight** ordinary kinds and **four** named appointments.
//! This module is the single place that says which kind fills which slot, when
//! the slot opens, how heavy it is in the roll and how many of it may be alive;
//! the director ([`crate::game::director`]) only feeds it the clock and the live
//! counts and spawns what it returns.
//!
//! ## The eight slots (`WAVE4_ROSTERS.md` "Reading a room")
//!
//! | Slot | Role | Unlocks | Pool weight | Cap |
//! |---|---|---|---|---|
//! | A | ground fodder | 0:00 | 3 (A:B = 3:1) | live budget |
//! | B | flying fodder | 0:45 | 1 | live budget |
//! | C | chaser | 1:15 | 2 (C:R:H = 2:1:1) | live budget |
//! | I | first trick | 2:00 | 1 | 5 |
//! | R | shooter | 3:35 | 1 | 4 |
//! | II | second trick | 4:30 | 1 | 4 |
//! | H | heavy (elite body) | 6:35 | 1 | 8 ordinary |
//! | III | third trick | 9:45 | 1 | 3 |
//!
//! Pool shares and the featured rotation are the design's "Director and
//! campaign" table, row for row: [`special_share`], [`fodder_minimum`],
//! [`featured`] and [`SPECIAL_CEILING`].
//!
//! ## Selection
//!
//! [`masses`] lays out one probability per slot: each pool's share is split
//! among its unlocked members by [`Slot::pool_weight`] (the featured slot
//! doubled), a member at its cap hands its share **back to fodder**, and an
//! empty pool hands its whole share to fodder. [`pick_kind`] then takes **one**
//! weighted draw over the slots in the design's stable order ([`Slot::ALL`]): a
//! locked or capped slot is simply at zero mass, so it costs no extra rng and
//! there are no retries.
//!
//! ## The roster is complete
//!
//! Each floor's design table names eight kinds and every one of them is a
//! real, registered body: `WAVE4_ROSTERS.md`'s fifteen `E`/`F` rows, plus the
//! 33 wave-four registrations and the eight Outfall kinds. Every slot owns
//! one body and no ordinary kind appears on two floors: seven floors × eight
//! slots gives 56 distinct ordinary kinds. The original home assignments stay
//! exactly: Sewer Skitterer/Bat/Pillbug/Big Snake, Kitchen Cat/Porter, Lab
//! Spitter/Lilliputians/Turnkey, Laundry House Centipede/Bell Flea/Wick Moth,
//! Printworks Termite Horde, Flatbed Murder Hornet/Brute.
//!
//! ## A guest is not a slot
//!
//! `EnemyKind::Drake` (the Sewer's big duck that pecks) is on a floor without
//! being in one: it holds no slot here, spends no slot cap and no live budget,
//! and it is not an ordinary kind this table can name. Its own clock and rule
//! live in `ecs::systems::drake`. The Outfall's Shore Crab has its own group
//! clock in `ecs::systems::crab`. `first_unlock` answers `None` for both.
//!
//! ## Rules still pending (the per-floor batch's list)
//!
//! A registration's stats, name, colour, gold, XP and Library entry are its
//! own from the day it ships, and **every behavioural decision in the
//! simulation keys on [`EnemyKind::archetype`]**, so today it behaves exactly
//! as its base body. That is the one mechanism the next batch hooks.
//!
//! * `V(X)` rows — one new rule each, on `X`'s behaviour: Pipe Eel (half
//!   turning rate), Crumb Ant (holds heading 0.25 s), Pepper Mill (split shot),
//!   Sink Crab (shield faces 90° off), Spinner Lid (weave flips), Erratum Slip
//!   (wide weave on spawn), Type Slug (last heading 0.6 s), Starch Bulb
//!   (accelerating shot), Ink Blot (sliding burst centre), Gravel Tick (drift
//!   toward the tailgate), Toll Gull (leading shot), Loose Bungee (crossbar
//!   pull). Where: the archetype's own system (`ecs/systems/enemy_move`,
//!   `enemy_wave3`, `enemy_wave4`) plus the constants beside it.
//! * `N` rows — a new behaviour: Sump Leech (tongue), Egg Timer (ring),
//!   Walking Syringe (drawn ray), Peg Snapper (sliding jaws), Staple Crawler
//!   (planted feet), Loose Can (rebounding roll). Each currently inherits a
//!   named base (`Pillbug`, `Pillbug`, `Bell Flea`, `Bell Flea`, `Bell Flea`,
//!   `Murder Hornet`); an `N` row graduates by adding its own arm to
//!   `EnemyKind::archetype`'s inverse — a real state machine in its floor's
//!   system — and then returning `self` from `archetype()`.
//! * Art: every registration draws its archetype's body recoloured to its own
//!   `color()` through one fallback (`art_enemy::recolor_body`). A floor's art
//!   pass adds the kind to `art_enemy::bespoke_body`/`bespoke_lod`'s table and
//!   writes its `draw_*` beside the others; its own death cue is a new arm in
//!   `vfx::emit_enemy_death`.
//!
//! ## Adding a new ordinary kind (the registration seam)
//!
//! Exactly one line binds a new kind to its roster slot — step 2 below. The
//! rest are the exhaustive `match` arms elsewhere in the crate, which stop
//! compiling until they are filled in: that is intended, and it is the list a
//! new kind has to satisfy.
//!
//! 1. `game/enemy.rs`: the [`EnemyKind`] variant, its row in `base_hp`,
//!    `speed`, `damage`, `xp`, `color`, `name` and `all`, and its arm in
//!    `archetype` — the base body whose steering, attack, nav size class and
//!    behaviour systems it reuses.
//! 2. **This file**: the floor's `slots` row (`kind: EnemyKind::New`).
//! 3. `game/shop.rs::gold_row`: the kind's chance × value row.
//! 4. `game/library.rs`: display name, the one-line "how to answer it" role
//!    line and the bestiary entry.
//! 5. `game/art_enemy.rs`: body scale, the idle/walk draw, the tell pose draw
//!    and the far-LOD draw (and its row in the bespoke table).
//! 6. `game/vfx.rs`: the death cue.
//! 7. `game/bot.rs`: the hazard/response knowledge (which shapes it must step
//!    out of).
//! 8. `game/ecs/systems/traps.rs::enemy_units`: how much glue board it takes.
//! 9. `autoplay.rs`: the damage-cause label and the kind name.
//!
//! A new **miniboss** is one line as well: the floor's `appointments` row here,
//! plus its arm in `ecs/systems/miniboss.rs` (name, body radius, one
//! `signature()` row — a shared shape evaluator with its own numbers — its body
//! in `art_miniboss`, and its row in `MinibossKind::all`).

use rand::{Rng, RngExt};

use crate::game::ecs::systems::miniboss::MinibossKind;
use crate::game::enemy::EnemyKind;
use crate::game::level::Theme;

/// How many ordinary slots a floor fields (`WAVE4_ROSTERS.md`: eight).
pub const SLOT_COUNT: usize = 8;

/// How many named appointments a floor fields (3:00, 6:00, 9:00, 12:00).
pub const APPOINTMENT_COUNT: usize = 4;

/// §1/"Director and campaign": the special pool (I + II + III) never takes more
/// than a fifth of the roll, on any floor at any point of the clock.
pub const SPECIAL_CEILING: f32 = 0.20;

/// Converts a design (fifteen-minute) clock into this build's clock.
///
/// The roster's unlock times and the featured rotation are written on the
/// design's own clock (`WAVE4_ROSTERS.md`), which is also the curve clock
/// ([`crate::game::director::curve_clock`]) while `RUN_SECONDS` is 900 s. The
/// scale keeps that true if the run length ever moves again.
#[inline]
fn design(secs: f32) -> f32 {
    secs * crate::game::director::WAVE3_TIME_SCALE
}

/// The eight ordinary slots, in the design's table order.
///
/// Declaration order **is** [`Slot::index`] order and the draw order, so the
/// weighted roll is stable: A, B, C, I, R, II, H, III.
// `I`, `II`, `III` are the design's own slot names (the trick slots).
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Ground fodder: opens alone, the first six approach from one direction.
    A,
    /// Flying fodder: smaller, faster, weaving.
    B,
    /// Chaser: a longer stride, pointed at the rat.
    C,
    /// First trick: one warned read under light pressure.
    I,
    /// Shooter: a side-on firing body.
    R,
    /// Second trick: a second decision about terrain, spacing or priority.
    II,
    /// Heavy: wide shoulders, slow gait; its elite previews the silhouette.
    H,
    /// Third trick: the lieutenant's ordinary support.
    III,
}

/// The three selection pools. Each pool owns a share of the roll and splits it
/// among its unlocked members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pool {
    /// A + B, `3:1`.
    Fodder,
    /// C + R + H, `2:1:1`.
    Mid,
    /// I + II + III, `1:1:1` (then capped by [`SPECIAL_CEILING`]).
    Special,
}

impl Pool {
    /// The pool's slots, in design order.
    pub fn slots(self) -> &'static [Slot] {
        match self {
            Pool::Fodder => &[Slot::A, Slot::B],
            Pool::Mid => &[Slot::C, Slot::R, Slot::H],
            Pool::Special => &[Slot::I, Slot::II, Slot::III],
        }
    }
}

impl Slot {
    /// Every slot, in the design's table order (the draw order).
    pub const ALL: [Slot; SLOT_COUNT] = [
        Slot::A,
        Slot::B,
        Slot::C,
        Slot::I,
        Slot::R,
        Slot::II,
        Slot::H,
        Slot::III,
    ];

    /// Design-order index; also the index into [`FloorRoster::slots`].
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The design's one-letter label ("A", "B", … "III").
    pub fn label(self) -> &'static str {
        match self {
            Slot::A => "A",
            Slot::B => "B",
            Slot::C => "C",
            Slot::I => "I",
            Slot::R => "R",
            Slot::II => "II",
            Slot::H => "H",
            Slot::III => "III",
        }
    }

    /// The role column of the design's "Reading a room" table.
    pub fn role(self) -> &'static str {
        match self {
            Slot::A => "ground fodder",
            Slot::B => "flying fodder",
            Slot::C => "chaser",
            Slot::I => "first trick",
            Slot::R => "shooter",
            Slot::II => "second trick",
            Slot::H => "heavy",
            Slot::III => "third trick",
        }
    }

    /// First eligible, on the build's clock: 0:00, 0:45, 1:15, 2:00, 3:35,
    /// 4:30, 6:35, 9:45.
    #[inline]
    pub fn unlock(self) -> f32 {
        design(match self {
            Slot::A => 0.0,
            Slot::B => 45.0,
            Slot::C => 75.0,
            Slot::I => 120.0,
            Slot::R => 215.0,
            Slot::II => 270.0,
            Slot::H => 395.0,
            Slot::III => 585.0,
        })
    }

    /// The pool this slot belongs to.
    #[inline]
    pub fn pool(self) -> Pool {
        match self {
            Slot::A | Slot::B => Pool::Fodder,
            Slot::C | Slot::R | Slot::H => Pool::Mid,
            Slot::I | Slot::II | Slot::III => Pool::Special,
        }
    }

    /// Raw weight inside the pool: A:B = 3:1, C:R:H = 2:1:1, specials 1:1:1.
    #[inline]
    pub fn pool_weight(self) -> f32 {
        match self {
            Slot::A => 3.0,
            Slot::B => 1.0,
            Slot::C => 2.0,
            Slot::R | Slot::H => 1.0,
            Slot::I | Slot::II | Slot::III => 1.0,
        }
    }

    /// Live ordinary bodies this slot may hold.
    ///
    /// The design's "Caps by slot": A/B/C up to the live budget (so no cap of
    /// their own), R 4, H 8, I 5, II 4, III 3. A specific kind can cap lower;
    /// see [`kind_cap`].
    #[inline]
    pub fn cap(self) -> usize {
        match self {
            Slot::A | Slot::B | Slot::C => usize::MAX,
            Slot::R => 4,
            Slot::H => 8,
            Slot::I => 5,
            Slot::II => 4,
            Slot::III => 3,
        }
    }

    /// The live-body cap for `kind` in this slot: the slot's own cap, lowered
    /// by the kind's rule if it has one.
    #[inline]
    pub fn cap_for(self, kind: EnemyKind) -> usize {
        self.cap().min(kind_cap(kind))
    }
}

/// A kind-specific live cap, layered under [`Slot::cap`].
///
/// The design: "Big Snake caps at 2" — its body is a wall, so two snakes is an
/// arena with three lanes and three snakes is a maze. The number lives in
/// `systems::enemy_wave4` beside the snake's stats.
#[inline]
pub fn kind_cap(kind: EnemyKind) -> usize {
    match kind {
        EnemyKind::BigSnake => crate::game::ecs::systems::enemy_wave4::SNAKE_LIVE_CAP,
        EnemyKind::Lampfish => 3,
        _ => usize::MAX,
    }
}

/// One slot's binding: which kind answers it, and (for a future content pass)
/// whether that kind is a stand-in for the one the design names.
#[derive(Clone, Copy, Debug)]
pub struct RosterSlot {
    pub slot: Slot,
    /// The kind that actually spawns in this slot today.
    pub kind: EnemyKind,
    /// `Some(name)` when `kind` stands in for a kind that does not exist yet.
    /// `None` on every ordinary slot today: all 56 ordinary bodies the design
    /// names are registered, so a floor fields exactly its own eight.
    pub stand_in_for: Option<&'static str>,
}

/// One floor: its eight ordinary slots and its four named appointments.
#[derive(Debug)]
pub struct FloorRoster {
    pub theme: Theme,
    /// Eight slots, in [`Slot::ALL`] order (`slots[i].slot == Slot::ALL[i]`).
    pub slots: [RosterSlot; SLOT_COUNT],
    /// The four appointments at 3:00, 6:00, 9:00 and 12:00, in order.
    pub appointments: [MinibossKind; APPOINTMENT_COUNT],
    /// The design's intended name for an appointment that still holds a
    /// stand-in. Empty (`None` everywhere) now that all twenty-eight named
    /// minibosses exist; kept so a future floor can borrow a body without
    /// losing the design's name for it.
    pub appointment_stand_ins: [Option<&'static str>; APPOINTMENT_COUNT],
}

impl FloorRoster {
    /// The kind that fills `slot` on this floor.
    #[inline]
    pub fn kind(&self, slot: Slot) -> EnemyKind {
        self.slots[slot.index()].kind
    }

    /// The slot a kind fills on this floor, if it is on the floor at all.
    ///
    /// Each floor's eight kinds are distinct (the roster test pins that), so
    /// this is unambiguous, which is what lets the director count live bodies
    /// per slot.
    pub fn slot_of(&self, kind: EnemyKind) -> Option<Slot> {
        self.slots.iter().find(|s| s.kind == kind).map(|s| s.slot)
    }

    /// This floor's fodder body — the A slot, the one a capped or absent pick
    /// falls back to.
    #[inline]
    pub fn fodder(&self) -> EnemyKind {
        self.kind(Slot::A)
    }
}

// ---- Floor tables -------------------------------------------------------
//
// Row by row from `WAVE4_ROSTERS.md`: every slot names the design's own body.
// `stand_in_for` is `None` everywhere — the ordinary roster has no stand-ins —
// and the comment above each row carries the kind's build (`R`/`V`/`N`) and
// which rule is still pending.

static SEWER: FloorRoster = FloorRoster {
    theme: Theme::Sewer,
    slots: [
        // A: the design's own ground fodder.
        RosterSlot { slot: Slot::A, kind: EnemyKind::Skitterer, stand_in_for: None },
        // B: the design's own weaving flyer.
        RosterSlot { slot: Slot::B, kind: EnemyKind::Bat, stand_in_for: None },
        // C: Pipe Eel · V(Cat) — half turning rate, reverse round a pipe end.
        RosterSlot { slot: Slot::C, kind: EnemyKind::PipeEel, stand_in_for: None },
        // I: the design's own curling shield.
        RosterSlot { slot: Slot::I, kind: EnemyKind::Pillbug, stand_in_for: None },
        // R: Grate Archer · R(Spitter) — shot through the puckered lip.
        RosterSlot { slot: Slot::R, kind: EnemyKind::GrateArcher, stand_in_for: None },
        // II: Sump Leech · N — a warned extension the rat steps out of and
        // circles behind. Its tongue is the next batch's rule; until then it
        // behaves as its archetype, the Pillbug.
        RosterSlot { slot: Slot::II, kind: EnemyKind::SumpLeech, stand_in_for: None },
        // H: Sluice Toad · R(Brute) — wide slow blocker.
        RosterSlot { slot: Slot::H, kind: EnemyKind::SluiceToad, stand_in_for: None },
        // III: the design's own moving wall.
        RosterSlot { slot: Slot::III, kind: EnemyKind::BigSnake, stand_in_for: None },
    ],
    appointments: [
        MinibossKind::Corkscrew,
        MinibossKind::BellDiver,
        // 9:00 · Valve Widow — data, Bobbin's evaluator.
        MinibossKind::ValveWidow,
        // 12:00 · The Mudlark — new, the closing pincer.
        MinibossKind::Mudlark,
    ],
    appointment_stand_ins: [None, None, None, None],
};

static KITCHEN: FloorRoster = FloorRoster {
    theme: Theme::Kitchen,
    slots: [
        // A: Crumb Ant · V(Skitterer) — a small direct column.
        RosterSlot { slot: Slot::A, kind: EnemyKind::CrumbAnt, stand_in_for: None },
        // B: Fruit Fly · R(Bat) — a weaving flyer.
        RosterSlot { slot: Slot::B, kind: EnemyKind::FruitFly, stand_in_for: None },
        // C: the design's own kitchen staff.
        RosterSlot { slot: Slot::C, kind: EnemyKind::Cat, stand_in_for: None },
        // I: Egg Timer · N — winds up close, then rings a departing footprint.
        RosterSlot { slot: Slot::I, kind: EnemyKind::EggTimer, stand_in_for: None },
        // R: Pepper Mill · V(Spitter) — the shot splits in two.
        RosterSlot { slot: Slot::R, kind: EnemyKind::PepperMill, stand_in_for: None },
        // II: the design's own sugar-cube healer.
        RosterSlot { slot: Slot::II, kind: EnemyKind::Porter, stand_in_for: None },
        // H: Pie Weight · R(Brute) — keeps advancing under fire.
        RosterSlot { slot: Slot::H, kind: EnemyKind::PieWeight, stand_in_for: None },
        // III: Sink Crab · V(Pillbug) — pillbug curl behind a pot-lid claw.
        RosterSlot { slot: Slot::III, kind: EnemyKind::SinkCrab, stand_in_for: None },
    ],
    appointments: [
        MinibossKind::MilkCarton,
        // 6:00 · Pastry Wheel — data, Corkscrew's trench.
        MinibossKind::PastryWheel,
        // 9:00 · Salt Hen — data, The Foreman's stamps.
        MinibossKind::SaltHen,
        // 12:00 · Pot Brush — data, Bobbin's sweeping cord.
        MinibossKind::PotBrush,
    ],
    appointment_stand_ins: [None, None, None, None],
};

static LAB: FloorRoster = FloorRoster {
    theme: Theme::Lab,
    slots: [
        // A: Swab Foot · R(Skitterer) — a small direct cleaner.
        RosterSlot { slot: Slot::A, kind: EnemyKind::SwabFoot, stand_in_for: None },
        // B: Spinner Lid · V(Bat) — a weave that flips handedness.
        RosterSlot { slot: Slot::B, kind: EnemyKind::SpinnerLid, stand_in_for: None },
        // C: Glass Runner · R(Cat) — a direct large-class route.
        RosterSlot { slot: Slot::C, kind: EnemyKind::GlassRunner, stand_in_for: None },
        // I: Walking Syringe · N — plants a tip and draws a dotted ray.
        RosterSlot { slot: Slot::I, kind: EnemyKind::WalkingSyringe, stand_in_for: None },
        // R: the design's own acid sac.
        RosterSlot { slot: Slot::R, kind: EnemyKind::Spitter, stand_in_for: None },
        // II: the design's own tiny squad.
        RosterSlot { slot: Slot::II, kind: EnemyKind::Lilliputian, stand_in_for: None },
        // H: Autoclave Cart · R(Brute) — a wide slow blocker.
        RosterSlot { slot: Slot::H, kind: EnemyKind::AutoclaveCart, stand_in_for: None },
        // III: the design's own crossbar drone.
        RosterSlot { slot: Slot::III, kind: EnemyKind::Turnkey, stand_in_for: None },
    ],
    appointments: [
        // 3:00 · Dose Pump — data, Corkscrew's trench.
        MinibossKind::DosePump,
        // 6:00 · Petri Mother — data, Bell Diver's ring.
        MinibossKind::PetriMother,
        // 9:00 · Cage Counter — data, The Foreman's stamps.
        MinibossKind::CageCounter,
        // 12:00 · Rotor Technician — data, Bobbin's sweeping cord.
        MinibossKind::RotorTechnician,
    ],
    appointment_stand_ins: [None, None, None, None],
};

static LAUNDRY: FloorRoster = FloorRoster {
    theme: Theme::Laundry,
    slots: [
        // A: Wool Pill · V(Skitterer) — a small direct rank that stalls.
        RosterSlot { slot: Slot::A, kind: EnemyKind::WoolPill, stand_in_for: None },
        // B: Clothes Moth · R(Bat) — a weaving flyer.
        RosterSlot { slot: Slot::B, kind: EnemyKind::ClothesMoth, stand_in_for: None },
        // C: the design's own wall-hugger.
        RosterSlot { slot: Slot::C, kind: EnemyKind::HouseCentipede, stand_in_for: None },
        // I: Peg Snapper · N — two jaws shown, then slid together. The jaws
        // are the next batch's rule; until then it behaves as its archetype,
        // the Bell Flea.
        RosterSlot { slot: Slot::I, kind: EnemyKind::PegSnapper, stand_in_for: None },
        // R: Starch Bulb · V(Spitter) — the shot accelerates.
        RosterSlot { slot: Slot::R, kind: EnemyKind::StarchBulb, stand_in_for: None },
        // II: the design's own tuning-fork hopper.
        RosterSlot { slot: Slot::II, kind: EnemyKind::BellFlea, stand_in_for: None },
        // H: Lint Golem · R(Brute) — a slow solid plug.
        RosterSlot { slot: Slot::H, kind: EnemyKind::LintGolem, stand_in_for: None },
        // III: the design's own fuse-on-death moth.
        RosterSlot { slot: Slot::III, kind: EnemyKind::WickMoth, stand_in_for: None },
    ],
    appointments: [
        MinibossKind::SisterSnip,
        MinibossKind::Bobbin,
        // 9:00 · Suds Bailiff — data, Bell Diver's ring.
        MinibossKind::SudsBailiff,
        // 12:00 · The Wringer Boy — new, the paired rollers.
        MinibossKind::WringerBoy,
    ],
    appointment_stand_ins: [None, None, None, None],
};

static PRINTWORKS: FloorRoster = FloorRoster {
    theme: Theme::Printworks,
    slots: [
        // A: Silverfish · R(Skitterer) — a small direct streamer.
        RosterSlot { slot: Slot::A, kind: EnemyKind::Silverfish, stand_in_for: None },
        // B: Erratum Slip · V(Bat) — a wider-weaving flyer.
        RosterSlot { slot: Slot::B, kind: EnemyKind::ErratumSlip, stand_in_for: None },
        // C: Type Slug · V(Cat) — a ruled, last-heading chaser.
        RosterSlot { slot: Slot::C, kind: EnemyKind::TypeSlug, stand_in_for: None },
        // I: Staple Crawler · N — two needle feet marked, then planted.
        RosterSlot { slot: Slot::I, kind: EnemyKind::StapleCrawler, stand_in_for: None },
        // R: Paper Wasp · R(Spitter) — shot from a rolled tip.
        RosterSlot { slot: Slot::R, kind: EnemyKind::PaperWasp, stand_in_for: None },
        // II: the design's own many-bodied swarm.
        RosterSlot { slot: Slot::II, kind: EnemyKind::TermiteHorde, stand_in_for: None },
        // H: Paper Reel · R(Brute) — wide continuous contact.
        RosterSlot { slot: Slot::H, kind: EnemyKind::PaperReel, stand_in_for: None },
        // III: Ink Blot · V(Wick Moth) — the wick's burst centre slides.
        RosterSlot { slot: Slot::III, kind: EnemyKind::InkBlot, stand_in_for: None },
    ],
    appointments: [
        MinibossKind::Foreman,
        // 6:00 · Quoin Key — data, Sister Snip's cone.
        MinibossKind::QuoinKey,
        // 9:00 · Carbon Twin — data, Milk Carton's spill.
        MinibossKind::CarbonTwin,
        // 12:00 · Late Plate — new, the closing ring (Bell Diver's evaluator,
        // centred on the rat and shrinking).
        MinibossKind::LatePlate,
    ],
    appointment_stand_ins: [None, None, None, None],
};

static TRAILER: FloorRoster = FloorRoster {
    theme: Theme::Trailer,
    slots: [
        // A: Gravel Tick · V(Skitterer) — a small direct crab-walker.
        RosterSlot { slot: Slot::A, kind: EnemyKind::GravelTick, stand_in_for: None },
        // B: June Bug · R(Bat) — a wobbling flyer.
        RosterSlot { slot: Slot::B, kind: EnemyKind::JuneBug, stand_in_for: None },
        // C: Tar Rat · R(Cat) — a straight large-class chaser.
        RosterSlot { slot: Slot::C, kind: EnemyKind::TarRat, stand_in_for: None },
        // I: Loose Can · N — a warned roll that rebounds off the first face.
        RosterSlot { slot: Slot::I, kind: EnemyKind::LooseCan, stand_in_for: None },
        // R: Toll Gull · V(Spitter) — the shot leads the rat.
        RosterSlot { slot: Slot::R, kind: EnemyKind::TollGull, stand_in_for: None },
        // II: the design's own diving hornet.
        RosterSlot { slot: Slot::II, kind: EnemyKind::MurderHornet, stand_in_for: None },
        // H: the design's own plated blocker.
        RosterSlot { slot: Slot::H, kind: EnemyKind::Brute, stand_in_for: None },
        // III: Loose Bungee · V(Turnkey) — the crossbar with a pull on it.
        RosterSlot { slot: Slot::III, kind: EnemyKind::LooseBungee, stand_in_for: None },
    ],
    appointments: [
        // 3:00 · Tarpaulin Pup — data, Milk Carton's spill.
        MinibossKind::TarpaulinPup,
        // 6:00 · Axle Nut — data, Corkscrew's trench.
        MinibossKind::AxleNut,
        // 9:00 · Chain Dog — data, Bobbin's sweeping cord.
        MinibossKind::ChainDog,
        // 12:00 · The Road Atlas — data, The Foreman's stamps.
        MinibossKind::RoadAtlas,
    ],
    appointment_stand_ins: [None, None, None, None],
};

static OUTFALL: FloorRoster = FloorRoster {
    theme: Theme::Outfall,
    slots: [
        RosterSlot { slot: Slot::A, kind: EnemyKind::SandHopper, stand_in_for: None },
        RosterSlot { slot: Slot::B, kind: EnemyKind::GlassShrimp, stand_in_for: None },
        RosterSlot { slot: Slot::C, kind: EnemyKind::Conger, stand_in_for: None },
        RosterSlot { slot: Slot::I, kind: EnemyKind::Skater, stand_in_for: None },
        RosterSlot { slot: Slot::R, kind: EnemyKind::PistolShrimp, stand_in_for: None },
        RosterSlot { slot: Slot::II, kind: EnemyKind::Lampfish, stand_in_for: None },
        RosterSlot { slot: Slot::H, kind: EnemyKind::Grouper, stand_in_for: None },
        RosterSlot { slot: Slot::III, kind: EnemyKind::FrySchool, stand_in_for: None },
    ],
    appointments: [
        MinibossKind::Gasket,
        MinibossKind::Swell,
        MinibossKind::Shade,
        MinibossKind::FalseBulb,
    ],
    appointment_stand_ins: [None; APPOINTMENT_COUNT],
};

/// The floor table for `theme`. Exhaustive over [`Theme`], so a new floor
/// cannot ship without its eight slots and four appointments.
pub fn floor(theme: Theme) -> &'static FloorRoster {
    match theme {
        Theme::Sewer => &SEWER,
        Theme::Kitchen => &KITCHEN,
        Theme::Lab => &LAB,
        Theme::Laundry => &LAUNDRY,
        Theme::Printworks => &PRINTWORKS,
        Theme::Trailer => &TRAILER,
        Theme::Outfall => &OUTFALL,
    }
}

/// The kind that fills `slot` on `theme` (shorthand for [`floor`]).
#[inline]
pub fn kind(theme: Theme, slot: Slot) -> EnemyKind {
    floor(theme).kind(slot)
}

/// The floor's ground fodder: where a capped or empty pick falls back to.
#[inline]
pub fn fodder_kind(theme: Theme) -> EnemyKind {
    floor(theme).fodder()
}

/// The floor's heavy — the body its elites wear (the design's "elite = the
/// floor's H body").
#[inline]
pub fn elite_kind(theme: Theme) -> EnemyKind {
    floor(theme).kind(Slot::H)
}

/// The floor's four appointments, in order (3:00, 6:00, 9:00, 12:00).
pub fn appointments(theme: Theme) -> [MinibossKind; APPOINTMENT_COUNT] {
    floor(theme).appointments
}

/// The earliest clock any floor may field `kind`, on the build's clock.
///
/// This is what the HUD's escalation callouts read (`"bats inbound"`), so it
/// is the minimum over the floors that name the kind, not the kind's own
/// unlock on its home floor.
pub fn first_unlock(kind: EnemyKind) -> Option<f32> {
    let mut at: Option<f32> = None;
    for theme in Theme::playable() {
        let roster = floor(theme);
        let Some(slot) = roster.slot_of(kind) else {
            continue;
        };
        let t = slot.unlock();
        at = Some(match at {
            Some(prev) => prev.min(t),
            None => t,
        });
    }
    at
}

// ---- The clock: pool shares, caps and the featured rotation ----------------

/// Share of the roll the special pool (I + II + III) takes at `clock`, from the
/// design's "Special I+II+III pool" column.
///
/// 0 until 2:00, then 8 % (to 3:00), 12 % (to 6:00), 16 % (to 9:00) and 20 % for
/// the rest of the build-up; 0 again at 15:00, where "new spawns use only local
/// A/B at cleanup pace".
#[inline]
pub fn special_share(clock: f32) -> f32 {
    if clock < design(120.0) {
        0.0
    } else if clock < design(180.0) {
        0.08
    } else if clock < design(360.0) {
        0.12
    } else if clock < design(540.0) {
        0.16
    } else if clock < design(900.0) {
        0.20
    } else {
        0.0
    }
}

/// The design's "Fodder A+B minimum" column at `clock`: the share of the roll
/// the two fodder slots keep however the pools fall.
///
/// 100 % for the first minute and from 15:00, 70 % to 3:00, then 60 %, then
/// 55 % for the rest of the build-up.
#[inline]
pub fn fodder_minimum(clock: f32) -> f32 {
    if clock < design(60.0) {
        1.0
    } else if clock < design(180.0) {
        0.70
    } else if clock < design(360.0) {
        0.60
    } else if clock < design(900.0) {
        0.55
    } else {
        1.0
    }
}

/// §1: "At 7:15 feature II and at 8:15 feature I to rehearse both before III
/// arrives", then a repeating 30 s I → II → III rotation from 9:45.
const FEATURE_II_FROM: f32 = 435.0;
const FEATURE_I_FROM: f32 = 495.0;
const FEATURED_FROM: f32 = 585.0;
const FEATURED_PERIOD: f32 = 30.0;

/// The slot whose pool weight is doubled at `clock`, if any.
///
/// Design: II is featured from 7:15, I from 8:15, and from 9:45 every floor
/// cycles I → II → III every 30 s, doubling only the featured weight inside its
/// own pool. Before 7:15 nothing is featured.
#[inline]
pub fn featured(clock: f32) -> Option<Slot> {
    if clock >= design(FEATURED_FROM) {
        let period = design(FEATURED_PERIOD).max(1e-3);
        let step = (((clock - design(FEATURED_FROM)) / period).floor().max(0.0)) as usize;
        Some([Slot::I, Slot::II, Slot::III][step % 3])
    } else if clock >= design(FEATURE_I_FROM) {
        Some(Slot::I)
    } else if clock >= design(FEATURE_II_FROM) {
        Some(Slot::II)
    } else {
        None
    }
}

// ---- Live caps -----------------------------------------------------------

/// Which slots have as many bodies alive as they may hold.
///
/// Built once per spawn pulse from the live (and queued) ordinary bodies;
/// [`pick_kind`] reads it and hands a capped slot's share back to fodder inside
/// its one draw, so capping costs no retry and no rng.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotCaps {
    /// Indexed by [`Slot::index`].
    pub at_cap: [bool; SLOT_COUNT],
}

impl SlotCaps {
    /// Nothing is capped (a fresh floor, or the tests' default).
    #[inline]
    pub fn none() -> Self {
        SlotCaps {
            at_cap: [false; SLOT_COUNT],
        }
    }

    /// Mark one slot as full.
    #[inline]
    pub fn mark(&mut self, slot: Slot) {
        self.at_cap[slot.index()] = true;
    }

    /// Is this slot full?
    #[inline]
    pub fn is_capped(&self, slot: Slot) -> bool {
        self.at_cap[slot.index()]
    }

    /// Build from live plus queued ordinary bodies per slot, each compared with
    /// its floor's effective cap ([`Slot::cap_for`]).
    pub fn from_counts(theme: Theme, counts: [u16; SLOT_COUNT]) -> Self {
        let roster = floor(theme);
        let mut caps = SlotCaps::none();
        for &slot in Slot::ALL.iter() {
            let cap = slot.cap_for(roster.kind(slot));
            if (counts[slot.index()] as usize) >= cap {
                caps.mark(slot);
            }
        }
        caps
    }
}

// ---- The weighted draw ---------------------------------------------------

/// One probability per slot ([`Slot::index`] order) at `clock`.
///
/// The design's rules, in order:
///
/// * the special pool takes [`special_share`] of the roll, ceiled at
///   [`SPECIAL_CEILING`]; the fodder pool keeps [`fodder_minimum`]; the middle
///   pool (C, R, H) takes what is left;
/// * inside a pool the unlocked members share the pool by [`Slot::pool_weight`],
///   the [`featured`] member doubled;
/// * a member at its cap hands its share back to fodder, as does a pool with no
///   unlocked member at all.
///
/// With nothing capped the masses sum to 1.0. The pool shares are the design's
/// one global table (only the *members* differ per floor), so this reads the
/// clock and the caps and nothing else.
pub fn masses(clock: f32, caps: &SlotCaps) -> [f32; SLOT_COUNT] {
    let boosted = featured(clock);
    let special = special_share(clock).min(SPECIAL_CEILING);
    let fodder = fodder_minimum(clock);
    let mid = (1.0 - special - fodder).max(0.0);

    let mut w = [0.0f32; SLOT_COUNT];
    // Everything a capped member (or an empty pool) hands to the fodder pool.
    let mut returned = 0.0f32;
    for (pool, share) in [(Pool::Special, special), (Pool::Mid, mid)] {
        let mut total = 0.0f32;
        for &slot in pool.slots() {
            if clock < slot.unlock() {
                continue;
            }
            let boost = if boosted == Some(slot) { 2.0 } else { 1.0 };
            w[slot.index()] = slot.pool_weight() * boost;
            total += w[slot.index()];
        }
        if total <= 0.0 {
            // Empty pool (nothing of it unlocked yet): its share is fodder's.
            returned += share;
            continue;
        }
        for &slot in pool.slots() {
            let i = slot.index();
            if w[i] <= 0.0 {
                continue;
            }
            let member = w[i] / total * share;
            if caps.at_cap[i] {
                returned += member;
                w[i] = 0.0;
            } else {
                w[i] = member;
            }
        }
    }

    // The fodder pool takes its own minimum plus everything returned, split
    // A:B = 3:1 among the fodder slots that are unlocked.
    let mut fodder_total = 0.0f32;
    for &slot in Pool::Fodder.slots() {
        // A capped fodder slot (the live budget itself is full) takes nothing;
        // its share stays with the other fodder slot, and with both capped the
        // floor has no mass at all (`pick_kind` then answers fodder).
        if clock < slot.unlock() || caps.at_cap[slot.index()] {
            continue;
        }
        w[slot.index()] = slot.pool_weight();
        fodder_total += w[slot.index()];
    }
    if fodder_total > 0.0 {
        let share = fodder + returned;
        for &slot in Pool::Fodder.slots() {
            let i = slot.index();
            if w[i] > 0.0 {
                w[i] = w[i] / fodder_total * share;
            }
        }
    }
    w
}

/// Which kind spawns next on `theme`, at `clock`, given the live caps.
///
/// One weighted draw over [`Slot::ALL`] in its stable order: a locked or capped
/// slot is at zero mass and simply costs no extra rng, and there are no
/// retries, so the same seed and clock always pick the same body.
pub fn pick_kind(theme: Theme, clock: f32, caps: &SlotCaps, rng: &mut impl Rng) -> EnemyKind {
    let roster = floor(theme);
    let w = masses(clock, caps);
    let total: f32 = w.iter().sum();
    if total <= 0.0 {
        // Everything is capped (the live budget is full): the caller's budget
        // check stops the spawn, and fodder is the harmless answer.
        return roster.fodder();
    }
    let mut roll = rng.random_range(0.0..total);
    let mut last = Slot::A;
    for &slot in Slot::ALL.iter() {
        let mass = w[slot.index()];
        if mass > 0.0 {
            last = slot;
            if roll < mass {
                return roster.kind(slot);
            }
            roll -= mass;
        }
    }
    // Floating-point dust at the very top of the range: the last member.
    roster.kind(last)
}

/// The procedural approach sound of a floor's fodder body (the design's "sound
/// pair for its two fodder kinds"): slot A is the ground fodder, slot B the
/// flying fodder, every other slot is silent on approach (`None`) because the
/// louder accents are reserved for tells. Played by
/// `systems::approach::approach_sfx_system`.
pub fn approach_sfx(theme: Theme, slot: Slot) -> Option<crate::audio::Sfx> {
    use crate::audio::Sfx;
    let (a, b) = match theme {
        Theme::Sewer => (Sfx::ApproachPipeClaws, Sfx::ApproachWingLeather),
        Theme::Kitchen => (Sfx::ApproachAntFeet, Sfx::ApproachWhine),
        Theme::Lab => (Sfx::ApproachSwab, Sfx::ApproachLid),
        Theme::Laundry => (Sfx::ApproachWoolSlap, Sfx::ApproachBrush),
        Theme::Printworks => (Sfx::ApproachRasp, Sfx::ApproachPaperFlap),
        Theme::Trailer => (Sfx::ApproachGravel, Sfx::ApproachWasher),
        Theme::Outfall => (Sfx::ApproachWoolSlap, Sfx::ApproachWingLeather),
    };
    match slot {
        Slot::A => Some(a),
        Slot::B => Some(b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn themes() -> [Theme; 7] {
        Theme::playable()
    }

    /// The design's contract: eight ordinary slots and four appointments on
    /// every floor, each one distinct, in the design's order.
    #[test]
    fn every_floor_has_eight_slots_and_four_appointments() {
        for theme in themes() {
            let roster = floor(theme);
            assert_eq!(roster.theme, theme);
            assert_eq!(roster.slots.len(), SLOT_COUNT);
            for (i, entry) in roster.slots.iter().enumerate() {
                assert_eq!(entry.slot, Slot::ALL[i], "{theme:?}: row {i} is out of order");
                assert_eq!(entry.slot.index(), i);
            }
            // Eight *unique* ordinary kinds: the slot of a kind is unambiguous,
            // which is what the director's cap counting relies on.
            for a in 0..SLOT_COUNT {
                for b in 0..a {
                    assert_ne!(
                        roster.slots[a].kind, roster.slots[b].kind,
                        "{theme:?} fields {:?} in both {} and {}",
                        roster.slots[a].kind,
                        roster.slots[b].slot.label(),
                        roster.slots[a].slot.label()
                    );
                }
                assert_eq!(
                    roster.slot_of(roster.slots[a].kind),
                    Some(roster.slots[a].slot)
                );
            }
            // Four unique appointments, all real minibosses.
            assert_eq!(roster.appointments.len(), APPOINTMENT_COUNT);
            for a in 0..APPOINTMENT_COUNT {
                for b in 0..a {
                    assert_ne!(
                        roster.appointments[a], roster.appointments[b],
                        "{theme:?} repeats a miniboss"
                    );
                }
            }
        }
    }

    /// The fifteen existing kinds stay on the floor the design gives them.
    #[test]
    fn the_designs_home_assignments_are_kept() {
        use EnemyKind::*;
        let homes = [
            (Theme::Sewer, Slot::A, Skitterer),
            (Theme::Sewer, Slot::B, Bat),
            (Theme::Sewer, Slot::I, Pillbug),
            (Theme::Sewer, Slot::III, BigSnake),
            (Theme::Kitchen, Slot::C, Cat),
            (Theme::Kitchen, Slot::II, Porter),
            (Theme::Lab, Slot::R, Spitter),
            (Theme::Lab, Slot::II, Lilliputian),
            (Theme::Lab, Slot::III, Turnkey),
            (Theme::Laundry, Slot::C, HouseCentipede),
            (Theme::Laundry, Slot::II, BellFlea),
            (Theme::Laundry, Slot::III, WickMoth),
            (Theme::Printworks, Slot::II, TermiteHorde),
            (Theme::Trailer, Slot::II, MurderHornet),
            (Theme::Trailer, Slot::H, Brute),
        ];
        for (theme, slot, home) in homes {
            assert_eq!(kind(theme, slot), home, "{theme:?} {}: home kind", slot.label());
            assert!(
                floor(theme).slots[slot.index()].stand_in_for.is_none(),
                "{theme:?} {}: the design's own body is flagged as a stand-in",
                slot.label()
            );
        }
        // Every existing kind has a home somewhere, so no floor is short and
        // The legacy Boss and both guests are outside the roster.
        for k in EnemyKind::all() {
            if matches!(k, Boss | EnemyKind::Drake | EnemyKind::ShoreCrab) {
                continue;
            }
            assert!(first_unlock(k).is_some(), "{k:?} is on no floor");
        }
        assert!(
            first_unlock(EnemyKind::Drake).is_none(),
            "the guest must not hold a roster slot"
        );
        assert!(first_unlock(EnemyKind::ShoreCrab).is_none());
    }

    /// The ordinary roster has no stand-ins left: every slot names the
    /// design's own body, and that body is a registered kind.
    #[test]
    fn every_ordinary_slot_names_its_own_kind() {
        for theme in themes() {
            let roster = floor(theme);
            for entry in roster.slots.iter() {
                assert_eq!(
                    entry.stand_in_for,
                    None,
                    "{theme:?} {}: the design's body is still a stand-in",
                    entry.slot.label()
                );
                assert!(
                    EnemyKind::all().contains(&entry.kind),
                    "{theme:?} {}: {:?} is not a registered kind",
                    entry.slot.label(),
                    entry.kind
                );
            }
        }
    }

    /// Every floor fields eight distinct bodies. Outfall and the original
    /// wave-four rosters share no kinds.
    #[test]
    fn the_seven_floors_field_56_distinct_ordinary_kinds() {
        let mut seen: Vec<EnemyKind> = Vec::new();
        for theme in themes() {
            let roster = floor(theme);
            for entry in roster.slots.iter() {
                assert!(
                    !seen.contains(&entry.kind),
                    "{theme:?} {} repeats {:?}, which another floor already fields",
                    entry.slot.label(),
                    entry.kind
                );
                seen.push(entry.kind);
            }
        }
        assert_eq!(
            seen.len(),
            56,
            "56 ordinary kinds, eight per floor"
        );
        let registered = seen.iter().filter(|k| k.is_registration()).count();
        assert_eq!(registered, 33, "the batch registers 33 ordinary kinds");

    }

    /// Unlocks, pool weights and caps, exactly as "Reading a room" and the
    /// director table say.
    #[test]
    fn unlocks_weights_and_caps_are_the_designs() {
        let expected = [
            (Slot::A, 0.0, 3.0, usize::MAX),
            (Slot::B, 45.0, 1.0, usize::MAX),
            (Slot::C, 75.0, 2.0, usize::MAX),
            (Slot::I, 120.0, 1.0, 5),
            (Slot::R, 215.0, 1.0, 4),
            (Slot::II, 270.0, 1.0, 4),
            (Slot::H, 395.0, 1.0, 8),
            (Slot::III, 585.0, 1.0, 3),
        ];
        assert_eq!(Slot::ALL.len(), expected.len());
        let mut prev = -1.0f32;
        for (slot, unlock, weight, cap) in expected {
            assert!(
                (slot.unlock() - design(unlock)).abs() < 1e-4,
                "{} unlocks at {}, not {unlock}",
                slot.label(),
                slot.unlock()
            );
            assert!(slot.unlock() > prev, "{} unlocked out of order", slot.label());
            prev = slot.unlock();
            assert_eq!(slot.pool_weight(), weight, "{} pool weight", slot.label());
            assert_eq!(slot.cap(), cap, "{} cap", slot.label());
        }
        // The pools themselves: A:B 3:1, C:R:H 2:1:1, I:II:III 1:1:1.
        assert_eq!(Pool::Fodder.slots(), &[Slot::A, Slot::B]);
        assert_eq!(Pool::Mid.slots(), &[Slot::C, Slot::R, Slot::H]);
        assert_eq!(Pool::Special.slots(), &[Slot::I, Slot::II, Slot::III]);
        for &slot in Slot::ALL.iter() {
            let pool = slot.pool();
            assert!(pool.slots().contains(&slot), "{} is not in its pool", slot.label());
        }
        // "Big Snake caps at 2" sits under the III slot's own cap of three.
        assert_eq!(Slot::III.cap_for(EnemyKind::BigSnake), 2);
        assert_eq!(Slot::II.cap_for(EnemyKind::Lampfish), 3);
        assert_eq!(Slot::III.cap_for(EnemyKind::WickMoth), 3);
        assert_eq!(Slot::R.cap_for(EnemyKind::Spitter), 4);
        assert_eq!(Slot::A.cap_for(EnemyKind::Skitterer), usize::MAX);
    }

    /// The design's pool-share table, over the whole clock, on every floor: the
    /// masses are a distribution, the special pool never takes more than a
    /// fifth, the fodder never drops under its minimum, and from 15:00 only
    /// local A/B spawn.
    #[test]
    fn the_pool_shares_hold_over_the_clock() {
        let caps = SlotCaps::none();
        for theme in themes() {
            let mut clock = 0.0f32;
            while clock <= design(1200.0) {
                let w = masses(clock, &caps);
                let total: f32 = w.iter().sum();
                assert!(
                    (total - 1.0).abs() < 1e-4,
                    "{theme:?} at {clock}: the masses sum to {total}"
                );
                let special: f32 = Pool::Special
                    .slots()
                    .iter()
                    .map(|s| w[s.index()])
                    .sum();
                assert!(
                    special <= SPECIAL_CEILING + 1e-6,
                    "{theme:?} at {clock}: special share {special}"
                );
                assert!(
                    (special - special_share(clock).min(SPECIAL_CEILING)).abs() < 1e-5,
                    "{theme:?} at {clock}: the special pool is not its clock's share"
                );
                let fodder: f32 = Pool::Fodder
                    .slots()
                    .iter()
                    .map(|s| w[s.index()])
                    .sum();
                assert!(
                    fodder >= fodder_minimum(clock) - 1e-5,
                    "{theme:?} at {clock}: fodder fell to {fodder}"
                );
                // Nothing before its unlock, whatever the clock.
                for &slot in Slot::ALL.iter() {
                    if clock < slot.unlock() {
                        assert_eq!(w[slot.index()], 0.0, "{theme:?}: {} early", slot.label());
                    }
                }
                // 15:00 cleanup: only the floor's own A and B.
                if clock >= design(900.0) {
                    let a = w[Slot::A.index()];
                    let b = w[Slot::B.index()];
                    assert!((a - 0.75).abs() < 1e-5, "{theme:?} at {clock}: A is {a}");
                    assert!((b - 0.25).abs() < 1e-5, "{theme:?} at {clock}: B is {b}");
                }
                clock += 5.0;
            }
        }
        // The four rows the design writes down explicitly.
        assert_eq!(special_share(0.0), 0.0);
        assert_eq!(special_share(design(119.0)), 0.0);
        assert_eq!(special_share(design(120.0)), 0.08);
        assert_eq!(special_share(design(300.0)), 0.12);
        assert_eq!(special_share(design(400.0)), 0.16);
        assert_eq!(special_share(design(600.0)), 0.20);
        assert_eq!(special_share(design(900.0)), 0.0);
        assert_eq!(fodder_minimum(0.0), 1.0);
        assert_eq!(fodder_minimum(design(90.0)), 0.70);
        assert_eq!(fodder_minimum(design(300.0)), 0.60);
        assert_eq!(fodder_minimum(design(600.0)), 0.55);
        assert_eq!(fodder_minimum(design(900.0)), 1.0);
    }

    /// Fodder really is 3:1 and the middle pool really is 2:1:1 where every
    /// member is unlocked and nothing is capped.
    #[test]
    fn the_pool_weights_split_as_the_design_says() {
        let caps = SlotCaps::none();
        for theme in themes() {
            // A late, uncapped instant: everything is on the table.
            let w = masses(design(700.0), &caps);
            let a = w[Slot::A.index()];
            let b = w[Slot::B.index()];
            assert!((a / b - 3.0).abs() < 1e-4, "{theme:?}: A:B is {}:1", a / b);
            let c = w[Slot::C.index()];
            let r = w[Slot::R.index()];
            let h = w[Slot::H.index()];
            assert!((c / r - 2.0).abs() < 1e-4, "{theme:?}: C:R is {}:1", c / r);
            assert!((r / h - 1.0).abs() < 1e-4, "{theme:?}: R:H is {}:1", r / h);
            let i = w[Slot::I.index()];
            let ii = w[Slot::II.index()];
            let iii = w[Slot::III.index()];
            // 700 s is inside the featured rotation, so one of the three is
            // doubled; the other two are equal.
            let mut lows = [i, ii, iii];
            lows.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
            assert!((lows[0] - lows[1]).abs() < 1e-5, "{theme:?}: specials are uneven");
            assert!((lows[2] / lows[0] - 2.0).abs() < 1e-4, "{theme:?}: the featured kind is not doubled");
        }
    }

    /// No kind can spawn before its slot opens, and one can right after.
    #[test]
    fn no_kind_spawns_before_its_slot_unlocks() {
        let caps = SlotCaps::none();
        for theme in themes() {
            for &slot in Slot::ALL.iter() {
                let kind = kind(theme, slot);
                if slot.unlock() > 0.0 {
                    let w = masses(slot.unlock() - 0.5, &caps);
                    assert_eq!(
                        w[slot.index()],
                        0.0,
                        "{theme:?}: {:?} is in the draw before {} opens",
                        kind,
                        slot.label()
                    );
                }
                let w = masses(slot.unlock() + 5.0, &caps);
                assert!(
                    w[slot.index()] > 0.0,
                    "{theme:?}: {:?} never joins the draw at {}",
                    kind,
                    slot.label()
                );
            }
        }
        // And the picker agrees, draw for draw.
        let mut rng = StdRng::seed_from_u64(5);
        for theme in themes() {
            for &slot in Slot::ALL.iter() {
                let kind = kind(theme, slot);
                if slot.unlock() <= 0.0 {
                    continue;
                }
                for _ in 0..400 {
                    assert_ne!(
                        pick_kind(theme, slot.unlock() - 0.5, &caps, &mut rng),
                        kind,
                        "{theme:?}: {:?} appeared before its unlock",
                        kind
                    );
                }
            }
        }
    }

    /// The heavy slot is the elite body, and it really is a heavy.
    #[test]
    fn the_elite_body_is_the_floors_heavy() {
        for theme in themes() {
            let elite = elite_kind(theme);
            assert_eq!(elite, kind(theme, Slot::H));
            assert!(elite.heavy(), "{theme:?}: the elite body is not a heavy");
        }
        // Flatbed's own brute is the design's `E` row.
        assert_eq!(elite_kind(Theme::Trailer), EnemyKind::Brute);
    }

    /// One rng draw per pick, the same answer for the same seed, and a capped
    /// slot returns its mass to fodder without an extra draw.
    #[test]
    fn the_weighted_pick_is_deterministic_and_draws_once() {
        let caps = SlotCaps::none();
        let clock = design(700.0);
        let total: f32 = masses(clock, &caps).iter().sum();

        let mut a = StdRng::seed_from_u64(7);
        let mut b = StdRng::seed_from_u64(7);
        for _ in 0..64 {
            assert_eq!(
                pick_kind(Theme::Sewer, clock, &caps, &mut a),
                pick_kind(Theme::Sewer, clock, &caps, &mut b)
            );
        }

        // Exactly one `random_range` per pick: after N picks the stream is
        // where N single draws leave it.
        let mut c = StdRng::seed_from_u64(11);
        let mut d = StdRng::seed_from_u64(11);
        for _ in 0..100 {
            let _ = pick_kind(Theme::Sewer, clock, &caps, &mut c);
        }
        for _ in 0..100 {
            let _ = d.random_range(0.0..total);
        }
        assert_eq!(
            c.random::<u64>(),
            d.random::<u64>(),
            "the pick must consume exactly one draw"
        );

        // A locked slot costs nothing at all: the same seed gives the same
        // first pick at 0:00 whether the later kinds exist or not, because
        // they are simply zero mass in the same one draw.
        let mut early = StdRng::seed_from_u64(3);
        for _ in 0..200 {
            assert_eq!(
                pick_kind(Theme::Sewer, 0.0, &caps, &mut early),
                EnemyKind::Skitterer
            );
        }
    }

    /// A capped slot's mass goes back to fodder (design: "return each capped
    /// member's mass to fodder"), and the draw never returns a capped kind.
    #[test]
    fn a_capped_slot_returns_its_mass_to_fodder() {
        let clock = design(700.0);
        let open = masses(clock, &SlotCaps::none());
        let mut caps = SlotCaps::none();
        caps.mark(Slot::III);
        let capped = masses(clock, &caps);
        assert_eq!(capped[Slot::III.index()], 0.0);
        let fodder_open: f32 = Pool::Fodder
            .slots()
            .iter()
            .map(|s| open[s.index()])
            .sum();
        let fodder_capped: f32 = Pool::Fodder
            .slots()
            .iter()
            .map(|s| capped[s.index()])
            .sum();
        assert!(
            (fodder_capped - fodder_open - open[Slot::III.index()]).abs() < 1e-5,
            "the capped slot's share did not go to fodder"
        );
        // The other pools are untouched.
        for &slot in [Slot::C, Slot::R, Slot::H, Slot::I, Slot::II].iter() {
            assert!((capped[slot.index()] - open[slot.index()]).abs() < 1e-5);
        }
        let mut rng = StdRng::seed_from_u64(21);
        for _ in 0..5000 {
            assert_ne!(
                pick_kind(Theme::Sewer, clock, &caps, &mut rng),
                kind(Theme::Sewer, Slot::III)
            );
        }
        // Every slot capped is a degenerate floor, not a panic.
        let mut all = SlotCaps::none();
        for &slot in Slot::ALL.iter() {
            all.mark(slot);
        }
        assert_eq!(
            pick_kind(Theme::Sewer, clock, &all, &mut rng),
            fodder_kind(Theme::Sewer)
        );
    }

    /// The design's rehearsals and the 9:45 rotation: II from 7:15, I from
    /// 8:15, then I → II → III every 30 s, doubling only inside the special
    /// pool.
    #[test]
    fn the_featured_slot_rehearses_then_rotates() {
        assert_eq!(featured(design(434.0)), None);
        assert_eq!(featured(design(435.0)), Some(Slot::II));
        assert_eq!(featured(design(494.0)), Some(Slot::II));
        assert_eq!(featured(design(495.0)), Some(Slot::I));
        assert_eq!(featured(design(584.0)), Some(Slot::I));
        assert_eq!(featured(design(585.0)), Some(Slot::I));
        assert_eq!(featured(design(615.0)), Some(Slot::II));
        assert_eq!(featured(design(645.0)), Some(Slot::III));
        assert_eq!(featured(design(675.0)), Some(Slot::I));
        // The featured kind is doubled inside the pool, and the pool's own
        // share does not move.
        let caps = SlotCaps::none();
        let window = design(645.0);
        let w = masses(window, &caps);
        let featured = kind(Theme::Kitchen, Slot::III);
        let plain = kind(Theme::Kitchen, Slot::I);
        let mut hits = 0usize;
        let mut others = 0usize;
        let mut rng = StdRng::seed_from_u64(88);
        for _ in 0..20_000 {
            match pick_kind(Theme::Kitchen, window, &caps, &mut rng) {
                k if k == featured => hits += 1,
                k if k == plain => others += 1,
                _ => {}
            }
        }
        assert!(
            hits as f32 > others as f32 * 1.8,
            "the featured kind did not double: {hits} vs {others}"
        );
        let special: f32 = Pool::Special
            .slots()
            .iter()
            .map(|s| w[s.index()])
            .sum();
        assert!(special <= SPECIAL_CEILING + 1e-6);
    }

    /// Every floor already fields a different cast today, and the four
    /// appointments are the design's fixed escalating order.
    #[test]
    fn floors_differ_in_cast_and_appointments() {
        let has = |a: &[EnemyKind], k: EnemyKind| a.contains(&k);
        let sewer: Vec<EnemyKind> = floor(Theme::Sewer).slots.iter().map(|s| s.kind).collect();
        let laundry: Vec<EnemyKind> = floor(Theme::Laundry).slots.iter().map(|s| s.kind).collect();
        assert!(has(&sewer, EnemyKind::BigSnake) && !has(&laundry, EnemyKind::BigSnake));
        assert!(has(&laundry, EnemyKind::HouseCentipede) && !has(&sewer, EnemyKind::HouseCentipede));
        let mut casts: Vec<Vec<EnemyKind>> = Vec::new();
        for theme in themes() {
            let cast: Vec<EnemyKind> = floor(theme).slots.iter().map(|s| s.kind).collect();
            assert!(!casts.contains(&cast), "{theme:?} fields another floor's cast");
            casts.push(cast);
        }
        // The design's fixed four, floor by floor.
        use MinibossKind::*;
        let rows = [
            (Theme::Sewer, [Corkscrew, BellDiver, ValveWidow, Mudlark]),
            (Theme::Kitchen, [MilkCarton, PastryWheel, SaltHen, PotBrush]),
            (Theme::Lab, [DosePump, PetriMother, CageCounter, RotorTechnician]),
            (Theme::Laundry, [SisterSnip, Bobbin, SudsBailiff, WringerBoy]),
            (Theme::Printworks, [Foreman, QuoinKey, CarbonTwin, LatePlate]),
            (Theme::Trailer, [TarpaulinPup, AxleNut, ChainDog, RoadAtlas]),
            (Theme::Outfall, [Gasket, Swell, Shade, FalseBulb]),
        ];
        for (theme, row) in rows {
            assert_eq!(appointments(theme), row, "{theme:?}");
        }
        for theme in themes() {
            let roster = floor(theme);
            assert_eq!(roster.appointment_stand_ins.len(), APPOINTMENT_COUNT);
            // Every appointment is the real body: no floor borrows another's.
            assert!(
                roster.appointment_stand_ins.iter().all(|s| s.is_none()),
                "{theme:?} still holds a stand-in"
            );
        }
        // The four appointments are unique across the whole campaign: 28 names,
        // 28 slots, one home each.
        let mut all: Vec<MinibossKind> = Vec::new();
        for theme in themes() {
            for kind in appointments(theme) {
                assert!(!all.contains(&kind), "{kind:?} is on two floors");
                all.push(kind);
            }
        }
        assert_eq!(all.len(), MinibossKind::all().len());
        for kind in MinibossKind::all() {
            assert!(all.contains(&kind), "{kind:?} keeps no appointment");
        }
        // The six existing minibosses keep their home slots and are not
        // flagged as stand-ins.
        let homes = [
            (Theme::Sewer, 0, Corkscrew),
            (Theme::Sewer, 1, BellDiver),
            (Theme::Kitchen, 0, MilkCarton),
            (Theme::Laundry, 0, SisterSnip),
            (Theme::Laundry, 1, Bobbin),
            (Theme::Printworks, 0, Foreman),
        ];
        for (theme, at, mb) in homes {
            let roster = floor(theme);
            assert_eq!(roster.appointments[at], mb, "{theme:?} appointment {at}");
            assert!(
                roster.appointment_stand_ins[at].is_none(),
                "{theme:?} appointment {at}: a real miniboss flagged as a stand-in"
            );
        }
    }

    /// The HUD's escalation callouts read [`first_unlock`], so the four base
    /// kinds must keep their historical first-appearance seconds.
    #[test]
    fn the_base_kinds_keep_their_first_appearance() {
        assert_eq!(first_unlock(EnemyKind::Skitterer), Some(0.0));
        assert!((first_unlock(EnemyKind::Bat).unwrap() - design(45.0)).abs() < 1e-4);
        assert!((first_unlock(EnemyKind::Cat).unwrap() - design(75.0)).abs() < 1e-4);
        assert!((first_unlock(EnemyKind::Spitter).unwrap() - design(215.0)).abs() < 1e-4);
        assert!((first_unlock(EnemyKind::Brute).unwrap() - design(395.0)).abs() < 1e-4);
        assert_eq!(first_unlock(EnemyKind::Boss), None);
    }
}
