//! Heads-up display: top tactical rail, bottom vitals/XP/dash rail, and
//! zero-allocation retained HUD state + telemetry.

use std::fmt::Write as _;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::audio::Sfx;
use crate::game::art_icons;
use crate::game::campaign;
use crate::game::director;
use crate::game::gem::PickupKind;
use crate::game::level::Theme;
use crate::game::loadout::{self, PassiveKind};
use crate::game::weapon::WeaponKind;
use crate::game::world::World;
use crate::input::DashKey;
use crate::math::Vec2;
use crate::render::campaign::{
    campaign_pips_width, draw_campaign_pips, CampaignLevel, CampaignProgressView,
};
use crate::render::palette::{self, Rgb, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};

/// Maximum simultaneous transient callouts held on the HUD.
pub const MAX_CALLOUTS: usize = 3;

/// How long a volume quick-key read-out stays in the status pill slot.
pub const VOLUME_FLASH_SECS: f32 = 1.6;

/// How long the "ZOOM 1.5x" read-out stays in the status pill slot: long enough
/// to read after one wheel notch, short enough to stop covering the ambient
/// status while the player scrolls a few rungs.
pub const ZOOM_FLASH_SECS: f32 = 1.4;

/// Which of the two independent channels a read-out is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeChannel {
    Music,
    Effects,
}

impl VolumeChannel {
    /// Short label for the HUD pill (`MUSIC` / `FX`).
    pub fn tag(self) -> &'static str {
        match self {
            VolumeChannel::Music => "MUSIC",
            VolumeChannel::Effects => "FX",
        }
    }
}

/// How close to `director::BOSS_TIME` the build-up starts reading as urgent.
///
/// The clock counts *down to the boss*, not to the end of the run (the level
/// now lasts until the boss or the rat dies), so this is the last stretch of
/// the build-up and the moment the top rail switches to the danger styling.
pub const BOSS_URGENT_SECS: f32 = 30.0;

/// Mazes per level the HUD's per-maze latches cover (a map carries at most
/// one; the extra bit costs nothing and keeps an old two-maze map working).
const MAX_MAZE_FEED: usize = 4;

/// Escalation stage of the boss fight itself: the build-up is over and the
/// level lasts until the boss or the rat dies.
const BOSS_STAGE: u8 = 7;

/// Is the elite warning badge due?
///
/// The director's elite gate is on the *curve* clock, which each campaign
/// level shifts by `director::LEVEL_CLOCK_OFFSET` (the first elite arrives at
/// 60 s on the Sewer, 40 s on the Kitchen and 15 s on the Lab). Reading the
/// wall clock here announced the Sewer's schedule on every level.
pub fn elite_badge_due(world: &World) -> bool {
    !world.director.boss_spawned
        && director::curve_clock(world.level_index(), world.elapsed)
            >= director::ELITE_START - 5.0
        && world.director.elite_timer <= 4.0
}

/// Danger / critical alert red (cohesive with rat tail pink, higher urgency).
pub const DANGER: Color = Color::Rgb(248, 92, 102);
pub const DANGER_DIM: Color = Color::Rgb(168, 52, 62);
pub const WARN_AMBER: Color = Color::Rgb(242, 184, 102);
pub const BOSS_ROSE: Color = Color::Rgb(236, 76, 128);
pub const BOSS_HOT: Color = Color::Rgb(255, 118, 88);
pub const HEAL_MINT: Color = Color::Rgb(126, 224, 152);
pub const MAGNET_ICE: Color = Color::Rgb(128, 206, 250);
pub const TRAIL_GOLD: Color = Color::Rgb(248, 214, 152);
/// Cold steel for the one-way gate slamming shut behind the rat.
pub const GATE_STEEL: Color = Color::Rgb(176, 204, 220);
/// Recessed dark bay behind the 2-row HUD weapon & passive icon rack.
pub const RACK_BG: Color = Color::Rgb(14, 18, 21);

/// Height in terminal rows of the top HUD region.
///
/// Uses 3 rows (1-row telemetry rail + 2-row weapon/passive icon rack) on every
/// playable terminal (`57×20` and up), collapsing to the legacy 1-row rail on
/// ultra-short or degenerate terminals so the playfield keeps its height.
pub fn top_height(area: Rect) -> u16 {
    if area.height >= 20 && area.width >= 48 {
        3
    } else {
        1
    }
}

/// Short 2–3 character label for a passive in the HUD icon rack.
#[allow(unreachable_patterns)]
pub(crate) fn passive_short_tag(kind: PassiveKind) -> &'static str {
    match kind {
        PassiveKind::MaxHp => "HP",
        PassiveKind::MoveSpeed => "SPD",
        PassiveKind::Damage => "DMG",
        PassiveKind::FireRate => "FR",
        PassiveKind::Magnet => "MAG",
        PassiveKind::Regen => "REG",
        PassiveKind::Armor => "ARM",
        PassiveKind::SpringHeel => "DSH",
        PassiveKind::Counterweight => "SHV",
        PassiveKind::LongFuse => "FUS",
        PassiveKind::SecondScent => "XP",
        PassiveKind::LensCap => "LNS",
    }
}
/// Wave 3 pocket finds: tin, brass, clock-case blue and laundry white.
pub const BOTTLE_TIN: Color = Color::Rgb(232, 226, 206);
pub const BRASS_KEY: Color = Color::Rgb(232, 190, 96);
pub const WATCH_ICE: Color = Color::Rgb(150, 210, 226);
pub const SOCK_WHITE: Color = Color::Rgb(240, 240, 232);

/// Distinct single-cell geometric glyph for each weapon kind.
pub fn weapon_glyph(kind: WeaponKind) -> char {
    match kind {
        WeaponKind::Gnaw => '▸',
        WeaponKind::CheeseSpray => '»',
        WeaponKind::SqueakNova => '✦',
        WeaponKind::SporeOrbit => '◎',
        WeaponKind::TailWhip => '◈',
        WeaponKind::AcidSpit => '♦',
        // The two player-triggered weapons read as "charged things": a bolt for
        // the thrash, a ring for the squeak.
        WeaponKind::TailThrash => 'ϟ',
        WeaponKind::SonicSqueak => '◯',
        // Wave 3: the beam is an eye, the dash weapon a double arrow.
        WeaponKind::LaserEyes => '◎',
        WeaponKind::SuperDash => '⇶',
        // Wave 3: a falling drop for Acid Rain, a cross (the tag) for
        // Hantavirus.
        WeaponKind::AcidRain => '↓',
        WeaponKind::Hantavirus => '✚',
        // A spiked tail: the asterisk that is also an eight-spiked star.
        WeaponKind::SpikedTail => '✳',
    }
}

/// Two-to-four character tag for a weapon slot in the HUD rack.
///
/// **These are the identity of a slot, not an abbreviation of its name.** The
/// review found the two Tail weapons sharing the tag `Ta` in every mode (and
/// `Tail` in the wide one), because the tag was the first characters of the
/// *name*: both begin "Tail ". A table with its first two characters unique
/// across all twelve weapons (and all twelve evolutions) is the fix, and
/// `hud::tests::every_weapon_and_evolution_tag_is_distinct` pins it.
#[allow(unreachable_patterns)]
pub fn weapon_tag(kind: WeaponKind, evolved: bool) -> &'static str {
    if evolved {
        return match kind {
            WeaponKind::Gnaw => "MILK",
            WeaponKind::CheeseSpray => "FOND",
            WeaponKind::SqueakNova => "ECHO",
            WeaponKind::SporeOrbit => "FAIRY",
            WeaponKind::TailWhip => "RTC",
            WeaponKind::AcidSpit => "DRAIN",
            WeaponKind::TailThrash => "FLY",
            WeaponKind::SonicSqueak => "ROLL",
            WeaponKind::LaserEyes => "CARB",
            WeaponKind::SuperDash => "RIP",
            WeaponKind::AcidRain => "ACID",
            WeaponKind::Hantavirus => "DEAD",
            WeaponKind::SpikedTail => "WIRE",
        };
    }
    match kind {
        WeaponKind::Gnaw => "GN",
        WeaponKind::CheeseSpray => "CH",
        WeaponKind::SqueakNova => "NOVA",
        WeaponKind::SporeOrbit => "ORB",
        WeaponKind::TailWhip => "WHIP",
        WeaponKind::AcidSpit => "SPIT",
        WeaponKind::TailThrash => "THRSH",
        WeaponKind::SonicSqueak => "SQK",
        WeaponKind::LaserEyes => "EYE",
        WeaponKind::SuperDash => "DASH",
        WeaponKind::AcidRain => "RAIN",
        WeaponKind::Hantavirus => "VIR",
        // The barbed tail ("BARB" would collide with SPIT's first two letters
        // in a two-letter rack; the tag is identity, not an abbreviation).
        WeaponKind::SpikedTail => "BARB",
    }
}

/// Codename of the level's themed boss, plus the overlay's classification tag.
///
/// The names are the campaign screens' own ([`CampaignLevel::boss_name`] in
/// `render/campaign.rs`), so the boss bar, the pre-boss warning and the
/// level-complete stamp all call the same fight the same thing: the Sewer's
/// **Apex Jaguar**, the Kitchen's **The Sweeper** and the Lab's
/// **Turf-Reaper Mk.IV**. The second element is the arrival banner's flavour
/// tag and never names a boss.
pub fn boss_codename(theme: Theme) -> (&'static str, &'static str) {
    boss_codename_kind(crate::game::ecs::systems::boss::BossKind::for_theme(theme))
}

/// The same plate for a **specific** boss kind.
///
/// A floor can pool more than one boss (`BossKind::pool_for_theme`), so the bar
/// and the warning ask the world which one is actually coming rather than
/// assuming the floor's original. The other campaign bosses keep their floor's
/// plate; the two floorless fights (the bird, the crab) carry their own.
pub fn boss_codename_kind(kind: crate::game::ecs::systems::boss::BossKind) -> (&'static str, &'static str) {
    use crate::game::ecs::systems::boss::BossKind;
    let level = match kind {
        BossKind::Cat => CampaignLevel::Sewer,
        BossKind::Broom => CampaignLevel::Kitchen,
        BossKind::Mower => CampaignLevel::Lab,
        BossKind::Mangle => CampaignLevel::Laundry,
        BossKind::NightEdition => CampaignLevel::Printworks,
        // The getaway ride's own fight is not a campaign floor, so it has no
        // campaign plate to borrow: the bar and the warning name the bird.
        BossKind::Magpie => {
            return ("THE MAGPIE", "CLASS-V CORVID THIEF");
        }
        // The Sewer's pool mate, whose plate is its own (`lore`).
        BossKind::HorseshoeCrab => {
            return (crate::lore::HORSESHOE_NAME, crate::lore::HORSESHOE_CLASS);
        }
        // The Kitchen's second fight: the facility's first success.
        BossKind::MegaRat => return (crate::lore::MEGARAT_NAME, crate::lore::MEGARAT_CLASS),
        // The flatbed's pool mate: the hawk that has been following the truck.
        // (While it is high, `boss_plate` swaps the tag for the overhead one.)
        BossKind::Hawk => return (crate::lore::HAWK_NAME, crate::lore::HAWK_CLASS),
    };
    let class_tag = match level {
        CampaignLevel::Sewer => "CLASS-V APEX PREDATOR",
        CampaignLevel::Kitchen => "CLASS-V CUSTODIAN",
        CampaignLevel::Lab => "MK.IV AUTONOMOUS UNIT",
        CampaignLevel::Laundry => "CAST-IRON WRINGER",
        CampaignLevel::Printworks => "OVERNIGHT PRESS — EDITION 1",
        // Unreachable: the Magpie branch above returns before this tag, but
        // the getaway ride now has a plate of its own and the match must say
        // what a corvid is.
        CampaignLevel::Trailer => "CLASS-V CORVID THIEF",
    };
    (level.boss_name(), class_tag)
}

/// The Megarat branch's name for [`boss_codename_kind`].
pub fn boss_kind_codename(
    kind: crate::game::ecs::systems::boss::BossKind,
) -> (&'static str, &'static str) {
    boss_codename_kind(kind)
}

/// The name plate for the fight the player is **in** (or about to be in).
///
/// A floor can field more than one boss now (`BossKind::pool_for_theme`), and
/// the pick is made from the level seed, so a plate keyed on the theme alone
/// would name the Sweeper while SUBJECT R-01 is on the tile. This reads the
/// living boss and otherwise answers with the boss the seed will field
/// ([`BossKind::for_theme_seeded`](crate::game::ecs::systems::boss::BossKind::for_theme_seeded)),
/// so the pre-boss countdown names the right animal too.
pub fn boss_plate(world: &World) -> (&'static str, &'static str) {
    use crate::game::ecs::systems::boss::{Boss, BossKind};
    let live = world.ecs.iter_entities().find_map(|entity| {
        let boss = entity.get::<Boss>()?;
        if boss.dead {
            None
        } else {
            // The overhead read rides on the plate: the Redtail is only a
            // shadow while it is high, and the bar must say why the rat's
            // weapons are finding nothing.
            Some((boss.kind, boss.overhead()))
        }
    });
    match live {
        // Only the Redtail can be out of reach, and the plate must say so.
        Some((BossKind::Hawk, true)) => {
            (crate::lore::HAWK_NAME, crate::game::art_boss_hawk::OVERHEAD_TAG)
        }
        Some((kind, _)) => boss_kind_codename(kind),
        None => {
            let theme = world.level.theme;
            let kind = BossKind::for_theme_seeded(theme, world.level.seed);
            // A single-fight floor (or the seed that fields the original boss)
            // is the canonical plate, which is the same answer `for_theme`
            // gives everything else in the UI.
            if kind == BossKind::for_theme(theme) {
                boss_codename(theme)
            } else {
                boss_kind_codename(kind)
            }
        }
    }
}

/// Difficulty escalation tier (1..=5) derived purely from the director's
/// *curve* clock (see [`crate::game::director::curve_clock`]).
///
/// `boss` is the boss phase itself, which is decided by the real run clock
/// (`World::boss_time_reached` / a living boss), not by the curve: the curve
/// freezes at `BOSS_TIME` but a later campaign level's curve starts above it.
pub fn threat_tier(curve: f32, boss: bool) -> u8 {
    if boss {
        5
    } else if curve >= 540.0 {
        4
    } else if curve >= director::ELITE_START {
        3
    } else if curve >= 20.0 {
        2
    } else {
        1
    }
}

/// Category of a transient in-game HUD callout pill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CalloutKind {
    Upgrade,
    Heal,
    Magnet,
    Nuke,
    Escalation,
    Milestone,
    BossSlain,
    /// A maze cache was looted (the heist's free upgrade).
    MazeCache,
    /// A maze's one-way gate slammed shut behind the rat.
    GateSlam,
    // Wave 3 pocket finds (WAVE3_DESIGN §10), appended so the legacy kinds
    // keep their order.
    BottleCap,
    WindupMouse,
    PocketWatch,
    CleanSocks,
    // Wave 3 review: the first-three-minutes teaching hints. Each fires once
    // per run — the review's complaint is that a new player is never told how
    // actives fire or what a chest / shop hole is (`WAVE3 review`, "The first
    // three minutes"). Appended, so the legacy kinds keep their order.
    ActivesHint,
    ChestHint,
    ShopHint,
    /// A new Library entry was discovered (the `LIBRARY + Pillbug` callout).
    Library,
    /// The auto-pick toggle took a level-up card without the screen: the pill
    /// carries the picked card's icon, name and new rank. Appended, so the
    /// legacy kinds keep their order.
    AutoPick,
    /// The nest's permanent upgrades, announced once when a floor starts
    /// (`NEST  8+8 slots · +18% move · …`). Appended last so the legacy kinds
    /// keep their order.
    Nest,
}

impl CalloutKind {
    pub fn color(self, sector_accent: Color) -> Color {
        match self {
            CalloutKind::Upgrade => TAIL,
            CalloutKind::Heal => HEAL_MINT,
            CalloutKind::Magnet => MAGNET_ICE,
            CalloutKind::Nuke => BOSS_HOT,
            CalloutKind::Escalation => WARN_AMBER,
            CalloutKind::Milestone => sector_accent,
            CalloutKind::BossSlain => HEAL_MINT,
            CalloutKind::MazeCache => TRAIL_GOLD,
            CalloutKind::GateSlam => GATE_STEEL,
            CalloutKind::BottleCap => BOTTLE_TIN,
            CalloutKind::WindupMouse => BRASS_KEY,
            CalloutKind::PocketWatch => WATCH_ICE,
            CalloutKind::CleanSocks => SOCK_WHITE,
            CalloutKind::ActivesHint => WARN_AMBER,
            CalloutKind::ChestHint => TRAIL_GOLD,
            CalloutKind::ShopHint => GATE_STEEL,
            CalloutKind::Library => TRAIL_GOLD,
            CalloutKind::AutoPick => TRAIL_GOLD,
            CalloutKind::Nest => TRAIL_GOLD,
        }
    }
}

/// One transient callout slot with retained buffers (no per-frame allocation).
#[derive(Default)]
pub struct CalloutSlot {
    pub active: bool,
    pub kind: Option<CalloutKind>,
    pub tag: String,
    pub message: String,
    pub timer: f32,
    pub duration: f32,
    /// Optional half-block icon drawn to the left of the text (auto-pick
    /// toasts). `None` for the text-only callouts; an icon pill is two rows
    /// tall so the 4x2 small sprite fits.
    pub icon: Option<&'static crate::game::art_icons::Icon>,
}

/// Snapshot of world entity telemetry fed into the HUD without reading
/// `World`'s internal entity lists directly from render code.
#[derive(Default, Clone, Copy)]
pub struct HudTelemetry {
    /// Active boss `(hp, max_hp)` if a boss is currently alive.
    pub boss_hp: Option<(f32, f32)>,
    /// World-pixel vector from player to active boss (`boss.pos - player.pos`).
    pub boss_offset: Option<Vec2>,
    /// Total living enemy count (for threat cues).
    pub enemy_count: usize,
    /// Whether a boss is known to be alive even before `boss_hp` is populated.
    pub boss_alive: bool,
    /// Whether audio is currently muted.
    pub muted: bool,
}

impl HudTelemetry {
    pub fn set_boss_hp(&mut self, hp: Option<(f32, f32)>) {
        self.boss_hp = hp;
        if hp.is_some() {
            self.boss_alive = true;
        }
    }

    pub fn set_boss_offset(&mut self, offset: Option<Vec2>) {
        self.boss_offset = offset;
    }

    pub fn set_enemy_count(&mut self, count: usize) {
        self.enemy_count = count;
    }

    pub fn set_boss_alive(&mut self, alive: bool) {
        self.boss_alive = alive;
        if !alive {
            self.boss_hp = None;
            self.boss_offset = None;
        }
    }
}

/// Reused text buffers and smooth visual animation state for the HUD.
///
/// `App` keeps one of these and hands it to [`draw_top`] / [`draw_bottom`] /
/// `overlay::draw_overlays` every frame: each readout is written into a
/// retained buffer that keeps its capacity instead of allocating per frame.
pub struct HudState {
    // Top bar retained buffers.
    time: String,
    level: String,
    kills: String,
    streak_buf: String,
    score: String,
    gold: String,
    /// `(tag, level)` text per weapon slot, reused across frames.
    weapon_tags: Vec<(String, String)>,
    /// Per weapon slot: is an **active** weapon fully charged? (`true` for the
    /// auto-firing weapons, which have no charge state to show.)
    active_ready: Vec<bool>,
    /// Per weapon slot: the "just became ready" flash timer, so a charged
    /// active weapon pops once instead of silently turning on.
    active_flash: Vec<f32>,
    right_badge: String,

    // Bottom bar retained buffers.
    hp: String,
    xp_label: String,
    dash: String,
    /// The dash module's state word (`READY` / `WAIT` / `3.4s` / `DASHING`),
    /// written after the module's charge bar.
    dash_status: String,

    /// Which key the dash module names (`CTRL`, or `SPACE` on a terminal that
    /// cannot report a lone modifier press). Set from `App::dash_key`; the
    /// live, remappable label is `dash_name`.
    pub dash_key: DashKey,
    /// The live dash binding's label, set from the Controls table so a
    /// rebinding shows on the HUD at once.
    pub dash_name: String,
    /// The live active-weapon binding's label (the `SPACE READY` token).
    pub active_name: String,
    /// The wave-3 pocket-find strip: `B12 M5 P4 S6` — one chip per active
    /// timed buff, refilled in `tick` so the draw path allocates nothing.
    buffs: String,
    /// The nest chip's text (`NEST +6 upgrades`), refilled in `tick` from the
    /// run's permanent upgrades. Empty for a fresh profile, so the chip never
    /// appears for a player who has not bought anything.
    nest: String,
    /// Does the strip have room for the finds' names (`CAP 12 MOUSE 5`) rather
    /// than their one-letter tags? Set by `draw_bottom` each frame.
    wide_chips: bool,

    // Overlay retained buffers (accessed by `overlay.rs`).
    pub(crate) boss_hp_text: String,
    pub(crate) boss_banner: String,
    /// Nameplate text for the living named miniboss (wave 3).
    pub(crate) miniboss_text: String,

    // Smooth visual state & feedback timers.
    pub clock: f32,
    pub hp_trail: f32,
    hp_trail_hold: f32,
    pub boss_hp_trail: f32,
    boss_trail_hold: f32,
    pub hurt_edge: f32,
    pub level_flash: f32,
    pub dash_ready_flash: f32,
    pub boss_intro_timer: f32,

    // Frame-to-frame delta tracking (initialized per run via `reset`).
    initialized: bool,
    prev_hp_ratio: f32,
    prev_hp: f32,
    prev_dash_cd: f32,
    prev_level: u32,
    prev_kills: u32,
    prev_boss_alive: bool,
    prev_boss_ratio: f32,
    escalation_stage: u8,
    next_kill_milestone: u32,
    /// Rolling recent kill burst (for combo streak readout).
    pub streak_kills: u32,
    streak_timer: f32,

    /// Transient "MUSIC 60%" read-out after a volume quick key, drawn in the
    /// top rail's status-pill slot for [`VOLUME_FLASH_SECS`].
    pub volume_flash: Option<(VolumeChannel, u8, f32)>,
    /// Transient "ZOOM 1.5x" read-out after a wheel notch or a `Z`/`X` press:
    /// `(zoom setting, seconds left)`. Drawn in the same status-pill slot for
    /// [`ZOOM_FLASH_SECS`], because a zoom change has no other tell.
    pub zoom_flash: Option<(f32, f32)>,

    /// Fixed-size callout slots (zero allocation on steady-state pushes).
    pub callouts: [CalloutSlot; MAX_CALLOUTS],
    /// Telemetry snapshot updated by `App` each tick.
    pub telemetry: HudTelemetry,

    /// Per-maze latches for the object layer's event feed: bit `i` is set once
    /// maze `i`'s cache has been claimed / its gate slammed, so a callout fires
    /// exactly once even though the feed only holds one tick.
    maze_claimed_bits: u32,
    maze_slammed_bits: u32,
    /// `(campaign level, level seed)` the maze latches belong to; a new level
    /// clears the bits (maze indices restart from 0 every level).
    maze_key: (usize, u64),

    // ---- First-three-minutes teaching (one hint each per run) --------------
    /// A charged active weapon has been explained.
    taught_actives: bool,
    /// A chest on the floor has been explained.
    taught_chest: bool,
    /// A mouse-hole shop has been explained.
    taught_shop: bool,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            time: String::with_capacity(8),
            level: String::with_capacity(6),
            kills: String::with_capacity(8),
            streak_buf: String::with_capacity(8),
            score: String::with_capacity(12),
            gold: String::with_capacity(12),
            weapon_tags: Vec::with_capacity(8),
            active_ready: Vec::with_capacity(8),
            active_flash: Vec::with_capacity(8),
            right_badge: String::with_capacity(24),
            hp: String::with_capacity(16),
            xp_label: String::with_capacity(12),
            dash: String::with_capacity(16),
            dash_status: String::with_capacity(8),
            dash_key: DashKey::Space,
            dash_name: "SPACE".to_string(),
            active_name: "SPACE".to_string(),
            buffs: String::with_capacity(48),
            nest: String::with_capacity(24),
            wide_chips: false,
            boss_hp_text: String::with_capacity(24),
            boss_banner: String::with_capacity(48),
            miniboss_text: String::with_capacity(48),
            clock: 0.0,
            hp_trail: 1.0,
            hp_trail_hold: 0.0,
            boss_hp_trail: 1.0,
            boss_trail_hold: 0.0,
            hurt_edge: 0.0,
            level_flash: 0.0,
            dash_ready_flash: 0.0,
            boss_intro_timer: 0.0,
            initialized: false,
            prev_hp_ratio: 1.0,
            prev_hp: 100.0,
            prev_dash_cd: 0.0,
            prev_level: 1,
            prev_kills: 0,
            prev_boss_alive: false,
            prev_boss_ratio: 1.0,
            escalation_stage: 0,
            next_kill_milestone: 50,
            streak_kills: 0,
            streak_timer: 0.0,
            volume_flash: None,
            zoom_flash: None,
            callouts: Default::default(),
            telemetry: HudTelemetry::default(),
            maze_claimed_bits: 0,
            maze_slammed_bits: 0,
            maze_key: (usize::MAX, u64::MAX),
            taught_actives: false,
            taught_chest: false,
            taught_shop: false,
        }
    }
}

impl HudState {
    /// Reset transient per-run HUD timers, trails, and callouts when starting a new run.
    pub fn reset(&mut self) {
        self.clock = 0.0;
        self.hp_trail = 1.0;
        self.hp_trail_hold = 0.0;
        self.boss_hp_trail = 1.0;
        self.boss_trail_hold = 0.0;
        self.hurt_edge = 0.0;
        self.level_flash = 0.0;
        self.dash_ready_flash = 0.0;
        self.boss_intro_timer = 0.0;
        self.initialized = false;
        self.prev_hp_ratio = 1.0;
        self.prev_hp = 100.0;
        self.prev_dash_cd = 0.0;
        self.prev_level = 1;
        self.prev_kills = 0;
        self.prev_boss_alive = false;
        self.prev_boss_ratio = 1.0;
        self.escalation_stage = 0;
        self.active_ready.clear();
        self.active_flash.clear();
        // A run that ends with a pocket find still running must not leave its
        // chip on the next run's strip.
        self.buffs.clear();
        self.nest.clear();
        self.next_kill_milestone = 50;
        self.streak_kills = 0;
        self.streak_timer = 0.0;
        self.volume_flash = None;
        self.zoom_flash = None;
        for slot in &mut self.callouts {
            slot.active = false;
            slot.timer = 0.0;
        }
        self.maze_claimed_bits = 0;
        self.maze_slammed_bits = 0;
        self.maze_key = (usize::MAX, u64::MAX);
        self.taught_actives = false;
        self.taught_chest = false;
        self.taught_shop = false;
        let muted = self.telemetry.muted;
        self.telemetry = HudTelemetry {
            muted,
            ..HudTelemetry::default()
        };
    }

    /// Push a transient callout pill without allocating once slot buffers are warm.
    pub fn push_callout(&mut self, kind: CalloutKind, tag: &str, message: &str, duration: f32) {
        self.push_callout_with_icon(kind, None, tag, message, duration);
    }

    /// Push a callout pill that also carries an [`art_icons`] sprite to the left
    /// of its text (the auto-pick toast). Same slot discipline as
    /// [`Self::push_callout`]; the icon is a `'static` reference, so nothing is
    /// allocated for it.
    pub fn push_callout_with_icon(
        &mut self,
        kind: CalloutKind,
        icon: Option<&'static crate::game::art_icons::Icon>,
        tag: &str,
        message: &str,
        duration: f32,
    ) {
        // Prefer an inactive slot; otherwise replace the slot closest to expiring.
        // A Library note is the one pill that never evicts another kind: with
        // the rail full of gameplay callouts it is dropped (the entry is still
        // marked NEW in the index).
        let free = self.callouts.iter().position(|c| !c.active);
        if kind == CalloutKind::Library && free.is_none() {
            let oldest_note = self
                .callouts
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == Some(CalloutKind::Library))
                .min_by(|(_, a), (_, b)| a.timer.total_cmp(&b.timer))
                .map(|(i, _)| i);
            if oldest_note.is_none() {
                return;
            }
        }
        let idx = free
            .or_else(|| {
                (kind == CalloutKind::Library)
                    .then(|| {
                        self.callouts
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.kind == Some(CalloutKind::Library))
                            .min_by(|(_, a), (_, b)| a.timer.total_cmp(&b.timer))
                            .map(|(i, _)| i)
                    })
                    .flatten()
            })
            .unwrap_or_else(|| {
                self.callouts
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.timer.total_cmp(&b.timer))
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        let slot = &mut self.callouts[idx];
        slot.active = true;
        slot.kind = Some(kind);
        slot.tag.clear();
        slot.tag.push_str(tag);
        slot.message.clear();
        slot.message.push_str(message);
        slot.timer = duration;
        slot.duration = duration.max(0.1);
        slot.icon = icon;
    }

    /// Notify the HUD that a specific pickup was collected.
    pub fn notify_pickup(&mut self, kind: PickupKind) {
        match kind {
            PickupKind::Heal => {
                self.push_callout(CalloutKind::Heal, "+MEDKIT", "HP RESTORED", 2.4);
            }
            PickupKind::Magnet => {
                self.push_callout(CalloutKind::Magnet, "+MAGNET", "ALL XP + GOLD INBOUND", 2.4);
            }
            PickupKind::Nuke => {
                self.push_callout(CalloutKind::Nuke, "+PURGE", "SECTOR CLEARED", 2.8);
            }
            // Wave 3 pocket finds (WAVE3_DESIGN §10). The callout names the
            // find and its rule; the timed chip on the buff strip counts it
            // down.
            PickupKind::BottleCap => {
                self.push_callout(CalloutKind::BottleCap, "+BOTTLE CAP", "BLOCKS NEXT HIT · 12s", 2.4);
            }
            PickupKind::WindupMouse => {
                self.push_callout(
                    CalloutKind::WindupMouse,
                    "+WIND-UP MOUSE",
                    "DECOY LURES FOES · 5s",
                    2.4,
                );
            }
            PickupKind::PocketWatch => {
                self.push_callout(CalloutKind::PocketWatch, "+POCKET WATCH", "ENEMY SHOTS SLOWED · 4s", 2.4);
            }
            PickupKind::CleanSocks => {
                self.push_callout(CalloutKind::CleanSocks, "+CLEAN SOCKS", "DASH READY · NO GLUE · 6s", 2.2);
            }
        }
    }

    /// Notify the HUD of the maze object layer's events for this tick.
    ///
    /// `GateRuntime::feed` is cleared and refilled every tick, so a claim could
    /// be lost by any frame the HUD does not observe. Each maze's claim/slam is
    /// therefore latched by index, and the persistent state (the fog's
    /// `maze_claimed` flag, the reward view's `Opened` state and the door's
    /// one-shot `slammed` latch) is OR-ed in every tick: a claim can only ever
    /// produce one callout, and it can never be missed. Allocation-free.
    fn notify_maze_feed(&mut self, world: &World) {
        // A new level restarts the maze indices, so drop the previous level's
        // latches instead of swallowing the new level's first claim.
        let key = (world.level_index(), world.level.seed);
        if key != self.maze_key {
            self.maze_key = key;
            self.maze_claimed_bits = 0;
            self.maze_slammed_bits = 0;
        }

        let mut claimed = self.maze_claimed_bits;
        let mut slammed = self.maze_slammed_bits;
        for event in world.maze_feed() {
            match *event {
                crate::game::ecs::systems::gates::MazeEvent::CacheClaimed { index } => {
                    claimed |= 1u32 << index.min(31);
                }
                crate::game::ecs::systems::gates::MazeEvent::GateSlammed { index } => {
                    slammed |= 1u32 << index.min(31);
                }
            }
        }
        // Recover an event whose tick the HUD never saw (the frame may be
        // skipped while the simulation keeps ticking).
        for maze in world.maze_markers() {
            if maze.claimed {
                claimed |= 1u32 << maze.index.min(31);
            }
            if world.maze_gate_slammed(maze.index) {
                slammed |= 1u32 << maze.index.min(31);
            }
        }

        let new_claims = claimed & !self.maze_claimed_bits;
        let new_slams = slammed & !self.maze_slammed_bits;
        self.maze_claimed_bits = claimed;
        self.maze_slammed_bits = slammed;
        for index in 0..MAX_MAZE_FEED {
            if new_claims & (1u32 << index) != 0 {
                self.push_callout(
                    CalloutKind::MazeCache,
                    "+CACHE",
                    "ONE FREE UPGRADE",
                    3.0,
                );
            }
            if new_slams & (1u32 << index) != 0 {
                self.push_callout(
                    CalloutKind::GateSlam,
                    "GATE SLAM",
                    "PACK SEALED BEHIND YOU",
                    2.8,
                );
            }
        }
    }

    /// Show a brief "MUSIC 60%" / "FX 60%" read-out after a volume change.
    pub fn notify_volume(&mut self, channel: VolumeChannel, percent: u8) {
        self.volume_flash = Some((channel, percent.min(100), VOLUME_FLASH_SECS));
    }

    /// Age the volume read-out. Called by [`Self::tick`] during play and by the
    /// app on the other screens, so a read-out set on the title or the options
    /// plate fades instead of lingering until the next run starts.
    pub fn tick_volume_flash(&mut self, dt: f32) {
        if let Some((channel, pct, left)) = self.volume_flash {
            self.volume_flash = if left - dt.max(0.0) > 0.0 {
                Some((channel, pct, left - dt.max(0.0)))
            } else {
                None
            };
        }
    }

    /// Show a brief "ZOOM 1.5x" read-out after the zoom setting changed.
    pub fn notify_zoom(&mut self, zoom: f32) {
        self.zoom_flash = Some((crate::render::zoom::snap(zoom), ZOOM_FLASH_SECS));
    }

    /// Age the zoom read-out (called next to [`Self::tick_volume_flash`] on
    /// every screen, and by [`Self::tick`] during play).
    pub fn tick_zoom_flash(&mut self, dt: f32) {
        if let Some((zoom, left)) = self.zoom_flash {
            self.zoom_flash = if left - dt.max(0.0) > 0.0 {
                Some((zoom, left - dt.max(0.0)))
            } else {
                None
            };
        }
    }

    /// Track the charge state of every **active** weapon slot (see
    /// [`WeaponKind::is_active`](crate::game::weapon::WeaponKind::is_active)).
    ///
    /// An active weapon's `Weapon::timer` counts *down* to zero as its charge
    /// fills, so "ready" is `timer <= 0`. Auto-firing weapons have no such
    /// state and are always reported ready (their pip keeps using the plain
    /// cooldown fraction).
    fn sync_active_ready(&mut self, world: &World, dt: f32) {
        let weapons = &world.player.loadout.weapons;
        while self.active_ready.len() < weapons.len() {
            self.active_ready.push(true);
            self.active_flash.push(0.0);
        }
        self.active_ready.truncate(weapons.len());
        self.active_flash.truncate(weapons.len());
        for (i, w) in weapons.iter().enumerate() {
            let ready = !w.kind.is_active() || w.timer <= 0.0;
            if ready && !self.active_ready[i] && world.elapsed > 0.2 {
                self.active_flash[i] = 0.55;
            }
            self.active_ready[i] = ready;
            self.active_flash[i] = (self.active_flash[i] - dt).max(0.0);
        }
    }

    /// The first-three-minutes hints, once each per run.
    ///
    /// The review's opening table found two things a new player is never told:
    /// that the two ACTIVE weapons wait for a key press (a first Tail Thrash
    /// "can look like a bad purchase if the player waits for auto-fire"), and
    /// that a chest is collected by touch (and a mouse hole entered by leaning
    /// into it). Both use the existing transient callout slot — one line, one
    /// time, no permanent manual — and name the key the terminal actually uses.
    fn teach_first_steps(&mut self, world: &World) {
        if !self.taught_actives {
            // The first charge of the player's first active weapon.
            let charged = world
                .player
                .loadout
                .weapons
                .iter()
                .position(|w| w.kind.is_active());
            if let Some(i) = charged {
                if self.active_ready.get(i).copied().unwrap_or(false) {
                    self.taught_actives = true;
                    // The tag is the live binding (SPACE unless remapped), so
                    // the lesson names the key the player actually has.
                    let tag = self.active_name.clone();
                    let msg = format!("WEAPON CHARGED · PRESS {tag}");
                    self.push_callout(CalloutKind::ActivesHint, &tag, &msg, 3.6);
                }
            }
        }
        if !self.taught_chest && world.chest_on_floor() {
            self.taught_chest = true;
            self.push_callout(CalloutKind::ChestHint, "CHEST", "WALK OVER IT TO OPEN", 3.2);
        }
        if !self.taught_shop {
            if let Some(d) = world.nearest_shop_hole() {
                if d <= 96.0 {
                    self.taught_shop = true;
                    self.push_callout(
                        CalloutKind::ShopHint,
                        "MOUSE HOLE",
                        "TOUCH THE SPOON TO SHOP",
                        3.2,
                    );
                }
            }
        }
    }

    /// Notify the HUD that the player just equipped an upgrade from the level-up screen.
    pub fn notify_upgrade(&mut self, title: &str) {
        self.level_flash = 0.9;
        self.push_callout(CalloutKind::Upgrade, "EQUIPPED", title, 2.5);
    }

    /// Advance smooth HUD animations, damage trails, and event callouts for one tick.
    pub fn tick(&mut self, dt: f32, world: &World, sfx_frame: &[Sfx], muted: bool) {
        let dt = dt.clamp(0.0, 0.1);
        self.clock += dt;
        self.telemetry.muted = muted;
        self.tick_volume_flash(dt);
        self.tick_zoom_flash(dt);

        let hp_ratio = (world.player.hp.max(0.0) / world.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        if !self.initialized {
            self.initialized = true;
            self.hp_trail = hp_ratio;
            self.prev_hp_ratio = hp_ratio;
            self.prev_hp = world.player.hp;
            self.prev_dash_cd = world.player.dash_cd;
            self.prev_level = world.player.level;
            self.prev_kills = world.kills;
            self.prev_boss_alive = self.telemetry.boss_alive;
            self.escalation_stage = if world.boss_time_reached() {
                BOSS_STAGE
            } else {
                Self::stage_for_curve(director::curve_clock(
                    world.level_index(),
                    world.elapsed,
                ))
            };
            return;
        }

        // 1. Player HP damage trail & hurt-edge vignette.
        if hp_ratio < self.prev_hp_ratio - 0.001 || world.player.hurt_flash > 0.18 {
            self.hp_trail = self.hp_trail.max(self.prev_hp_ratio);
            self.hp_trail_hold = 0.28;
            self.hurt_edge = 0.35;
        } else if hp_ratio > self.hp_trail {
            self.hp_trail = hp_ratio;
            self.hp_trail_hold = 0.0;
        }
        if self.hp_trail_hold > 0.0 {
            self.hp_trail_hold = (self.hp_trail_hold - dt).max(0.0);
        } else if self.hp_trail > hp_ratio {
            self.hp_trail = (self.hp_trail - dt * 0.55).max(hp_ratio);
        }
        self.hurt_edge = (self.hurt_edge - dt).max(0.0);

        // 2. Boss HP damage trail, arrival intro banner, and defeat detection.
        if let Some((bhp, bmax)) = self.telemetry.boss_hp {
            let bratio = (bhp.max(0.0) / bmax.max(1.0)).clamp(0.0, 1.0);
            if !self.prev_boss_alive {
                self.boss_hp_trail = bratio;
                self.prev_boss_ratio = bratio;
            } else if bratio < self.prev_boss_ratio - 0.001 {
                self.boss_hp_trail = self.boss_hp_trail.max(self.prev_boss_ratio);
                self.boss_trail_hold = 0.22;
            }
            if self.boss_trail_hold > 0.0 {
                self.boss_trail_hold = (self.boss_trail_hold - dt).max(0.0);
            } else if self.boss_hp_trail > bratio {
                self.boss_hp_trail = (self.boss_hp_trail - dt * 0.45).max(bratio);
            }
            self.prev_boss_ratio = bratio;
        }
        if sfx_frame.contains(&Sfx::Boss) || (self.telemetry.boss_alive && !self.prev_boss_alive) {
            self.boss_intro_timer = 3.2;
        }
        if self.prev_boss_alive && !self.telemetry.boss_alive && world.director.boss_spawned {
            self.push_callout(
                CalloutKind::BossSlain,
                "APEX DOWN",
                "SPECIMEN NEUTRALIZED",
                3.4,
            );
        }
        self.boss_intro_timer = (self.boss_intro_timer - dt).max(0.0);

        // 3. Dash-ready glint when cooldown finishes.
        if self.prev_dash_cd > 0.0 && world.player.dash_cd <= 0.0 {
            self.dash_ready_flash = 0.45;
        }
        self.dash_ready_flash = (self.dash_ready_flash - dt).max(0.0);

        // 3b. Active-weapon charge: remember each slot's readiness so a weapon
        // that just finished charging flashes once (the Space release is the
        // player's own cue to spend it).
        self.sync_active_ready(world, dt);

        // 4. Level-up flash.
        if world.player.level > self.prev_level {
            self.level_flash = 0.85;
        }
        self.level_flash = (self.level_flash - dt).max(0.0);

        // 5. Exact pickup callouts from `World::last_pickups()` (with SFX fallback for tests).
        let pickups = world.last_pickups();
        if !pickups.is_empty() {
            for &pk in pickups {
                self.notify_pickup(pk);
            }
        } else if sfx_frame.contains(&Sfx::Pickup) {
            if sfx_frame.contains(&Sfx::Explosion) {
                self.notify_pickup(PickupKind::Nuke);
            } else if world.player.hp > self.prev_hp + 4.0 {
                self.notify_pickup(PickupKind::Heal);
            } else {
                self.notify_pickup(PickupKind::Magnet);
            }
        }

        // 5b. Wave 3 pocket finds: one chip per running timer, with the whole
        //     seconds it has left (`B12 M5 P4 S6`). Refilled here (not in the
        //     draw path) so the strip reuses its buffer every frame.
        let wide_chips = self.wide_chips;
        refill(&mut self.buffs, |b| {
            let buffs = &world.player.buffs;
            let mut any = false;
            for (kind, left) in [
                (PickupKind::BottleCap, buffs.shield),
                (PickupKind::WindupMouse, buffs.decoy),
                (PickupKind::PocketWatch, buffs.slow_bullets),
                (PickupKind::CleanSocks, buffs.socks),
            ] {
                if left <= 0.0 {
                    continue;
                }
                if any {
                    b.push(' ');
                }
                let secs = left.ceil().max(1.0) as u32;
                // `B12 M05 P04 S06` is two letters of nothing: the review could
                // not tell a buff from a passive. Wide terminals name the find,
                // narrow ones keep the one-letter chip (the pause plate lists
                // the running finds in full).
                if wide_chips {
                    let _ = write!(b, "{} {}", buff_chip_name(kind), secs.min(99));
                } else {
                    let _ = write!(b, "{}{:02}", kind.buff_tag(), secs.min(99));
                }
                any = true;
            }
        });

        // 5b. The nest chip: how many permanent tracks this run is carrying.
        //     Retained buffer, so the draw path still allocates nothing.
        let nest_tracks = world.player.loadout.meta.tracks_owned();
        refill(&mut self.nest, |b| {
            if nest_tracks > 0 {
                let _ = write!(b, " ⌂ NEST {nest_tracks} ");
            }
        });

        // 5c. The first-three-minutes hints (`WAVE3 review`, "The first three
        //     minutes"): how actives fire, and what a chest / shop hole is.
        //     One callout each per run, in the existing transient slot, so the
        //     HUD never grows a permanent manual.
        self.teach_first_steps(world);

        // 6. Maze cache claims and one-way gate slams (latched, see
        //    `notify_maze_feed`).
        self.notify_maze_feed(world);

        // 7. Kill streak burst & kill milestones.
        let dk = world.kills.saturating_sub(self.prev_kills);
        if dk > 0 {
            self.streak_kills = self.streak_kills.saturating_add(dk);
            self.streak_timer = 2.6;
        } else if self.streak_timer > 0.0 {
            self.streak_timer = (self.streak_timer - dt).max(0.0);
            if self.streak_timer <= 0.0 {
                self.streak_kills = 0;
            }
        }
        if world.kills >= self.next_kill_milestone {
            let m = self.next_kill_milestone;
            let msg = match m {
                50 => "50 KILLS  ·  SWARM RISING",
                100 => "100 KILLS  ·  CONTAINMENT STRAINED",
                250 => "250 KILLS  ·  PROTOCOL OVERWHELMED",
                500 => "500 KILLS  ·  APEX PREDATOR",
                _ => "EXTERMINATION THRESHOLD EXCEEDED",
            };
            self.push_callout(CalloutKind::Milestone, "MILESTONE", msg, 2.5);
            self.next_kill_milestone = match m {
                50 => 100,
                100 => 250,
                250 => 500,
                _ => m.saturating_add(500),
            };
        }

        // 8. Time-curve escalation callouts (fire once per threshold).
        //    Thresholds read the director's *curve* clock, so a later campaign
        //    level (whose curve opens further along) announces its own
        //    milestones instead of the Sewer's, and the boss hand-off replaces
        //    the old "final 30 s" extraction callout.
        let stage = if world.boss_time_reached() {
            BOSS_STAGE
        } else {
            Self::stage_for_curve(director::curve_clock(
                world.level_index(),
                world.elapsed,
            ))
        };
        if stage > self.escalation_stage {
            self.escalation_stage = stage;
            match stage {
                1 => self.push_callout(
                    CalloutKind::Escalation,
                    "THREAT +",
                    "SWARM BATS INBOUND",
                    2.5,
                ),
                2 => self.push_callout(
                    CalloutKind::Escalation,
                    "THREAT +",
                    "STALKER CATS DETECTED",
                    2.5,
                ),
                3 => self.push_callout(
                    CalloutKind::Escalation,
                    "WARNING",
                    "ELITE BRUTES UNLOCKED",
                    2.8,
                ),
                4 => self.push_callout(
                    CalloutKind::Escalation,
                    "THREAT +",
                    "ACID SPITTERS ACTIVE",
                    2.5,
                ),
                5 => self.push_callout(
                    CalloutKind::Escalation,
                    "WARNING",
                    "HEAVY BRUTES DEPLOYED",
                    2.8,
                ),
                BOSS_STAGE => self.push_callout(
                    CalloutKind::Escalation,
                    "BOSS FIGHT",
                    "APEX SPECIMEN ENGAGED",
                    3.0,
                ),
                _ => {}
            }
        }

        // 9. Tick & expire active callout slots.
        for slot in &mut self.callouts {
            if slot.active {
                slot.timer -= dt;
                if slot.timer <= 0.0 {
                    slot.active = false;
                    slot.timer = 0.0;
                }
            }
        }

        self.prev_hp_ratio = hp_ratio;
        self.prev_hp = world.player.hp;
        self.prev_dash_cd = world.player.dash_cd;
        self.prev_level = world.player.level;
        self.prev_kills = world.kills;
        self.prev_boss_alive = self.telemetry.boss_alive;
    }

    /// Escalation stage for the director's *curve* clock.
    ///
    /// Every director curve function reads `curve_clock` (see
    /// `director::curve_clock`), so the milestones this stage fires land where
    /// the spawns actually change even on a level whose curve opens partway in.
    ///
    /// The thresholds are the director's own: each stage's callout names a
    /// kind, and it fires on that kind's first-appearance row
    /// (`director::base_unlock_time`) — bats, cats, the elite gate
    /// (`director::ELITE_START`), spitters, brutes. Nothing here restates the
    /// fifteen-minute schedule, so a retimed table moves the callouts with it.
    fn stage_for_curve(curve: f32) -> u8 {
        use crate::game::enemy::EnemyKind;
        let at = |kind| director::base_unlock_time(kind).unwrap_or(f32::INFINITY);
        // Highest stage whose threshold has passed (the table is not assumed
        // to be sorted: the elite gate is tuned separately from the unlocks).
        let thresholds = [
            at(EnemyKind::Bat),
            at(EnemyKind::Cat),
            director::ELITE_START,
            at(EnemyKind::Spitter),
            at(EnemyKind::Brute),
        ];
        let mut stage = 0;
        for (i, &t) in thresholds.iter().enumerate() {
            if curve >= t {
                stage = i as u8 + 1;
            }
        }
        stage
    }
}

/// Clear a retained buffer and refill it, so digits from a longer previous
/// value can never survive into this frame's text.
fn refill(buf: &mut String, fill: impl FnOnce(&mut String)) {
    buf.clear();
    fill(buf);
}

fn fmt_time_into(dst: &mut String, secs: f32) {
    let s = secs.max(0.0) as u32;
    let _ = write!(dst, "{:02}:{:02}", s / 60, s % 60);
}

/// Short weapon tag for the HUD: up to four characters, counted in chars so a
/// non-ASCII name can never slice mid-codepoint.
fn push_short_name(dst: &mut String, name: &str) {
    dst.extend(name.chars().take(4));
}

/// Vertical eighth-cell block glyph for weapon cooldown charge (`0.0..=1.0`).
fn cooldown_pip(ready: f32) -> char {
    const VSTEPS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let idx = ((ready.clamp(0.0, 1.0) * 7.0).round() as usize).min(7);
    VSTEPS[idx]
}

fn rgb_of_color(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (180, 180, 180),
    }
}

pub fn draw_top(frame: &mut Frame, area: Rect, world: &World, state: &mut HudState) {
    if area.is_empty() {
        return;
    }

    let accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    // The clock counts down to `director::BOSS_TIME` and stops there: from then
    // on the level lasts until the boss or the rat dies, so the rail switches
    // from a countdown to a boss-fight state instead of sitting at 00:00.
    let boss_phase = world.boss_time_reached();
    let time_left = world.time_left();
    let urgent = !boss_phase && time_left <= BOSS_URGENT_SECS;
    let boss_alive = state.telemetry.boss_alive;
    let clock_boss = boss_phase || boss_alive;
    let boss_soon = !world.director.boss_spawned
        && !boss_phase
        && world.elapsed >= director::BOSS_TIME - crate::render::overlay::BOSS_WARNING_SECS;
    let elite_due = elite_badge_due(world);
    let pulse = 0.5 + 0.5 * (state.clock * 7.5).sin();

    // Format numbers into retained buffers (zero per-frame heap allocation).
    refill(&mut state.time, |b| {
        if boss_phase {
            // Time *into* the fight since the boss arrived.
            fmt_time_into(b, world.elapsed - director::BOSS_TIME);
        } else {
            fmt_time_into(b, time_left);
        }
    });
    refill(&mut state.level, |b| {
        let _ = write!(b, "{}", world.player.level);
    });
    refill(&mut state.kills, |b| {
        let _ = write!(b, "{}", world.kills);
    });
    refill(&mut state.streak_buf, |b| {
        if state.streak_kills >= 8 {
            let _ = write!(b, " ×{}", state.streak_kills);
        }
    });
    refill(&mut state.score, |b| {
        let _ = write!(b, "{}", world.score);
    });

    refill(&mut state.gold, |b| {
        let _ = write!(b, "{}", world.gold);
    });

    // Weapon tags: reused `(tag, level)` buffers per slot.
    let weapons = &world.player.loadout.weapons;
    while state.weapon_tags.len() < weapons.len() {
        state.weapon_tags.push((String::new(), String::new()));
    }
    state.weapon_tags.truncate(weapons.len());
    for (i, w) in weapons.iter().enumerate() {
        let (tag, lvl) = &mut state.weapon_tags[i];
        tag.clear();
        // The slot's *identity tag* (`weapon_tag`), not the head of its name:
        // the review found both Tail weapons reading `Ta`/`Tail` because the
        // tag was the first characters of the name. An evolved weapon wears its
        // evolution's own tag (`MILK`, `DRAIN`, …) so a chest upgrade stays
        // visible in the rack.
        tag.push_str(weapon_tag(w.kind, w.evolved));
        lvl.clear();
        if w.is_max() {
            lvl.push('★');
        } else {
            let _ = write!(lvl, "{}", w.level);
        }
    }

    // Build right-aligned status pill text. A volume quick-key read-out owns
    // the slot while it is fresh: it is a direct answer to a press the player
    // just made, so it outranks the ambient status.
    refill(&mut state.right_badge, |b| {
        let flashing = state.volume_flash;
        if let Some((channel, pct, _)) = flashing {
            let _ = write!(b, " {} {}% ", channel.tag(), pct);
        } else if let Some((zoom, _)) = state.zoom_flash {
            b.push_str(" ZOOM ");
            crate::render::zoom::format_into(b, zoom);
            b.push(' ');
        } else if boss_phase {
            if boss_alive {
                b.push_str(" ◆ APEX ACTIVE ");
            } else {
                b.push_str(" ◆ BOSS FIGHT ");
            }
        } else if boss_alive {
            b.push_str(" ◆ APEX ACTIVE ");
        } else if boss_soon {
            // The precise countdown outranks the generic urgency pill once the
            // 25 s warning window opens; `urgent` still styles the rail.
            let rem = (director::BOSS_TIME - world.elapsed).ceil().max(1.0) as u32;
            let _ = write!(b, " ▲ BOSS {:02}s ", rem);
        } else if urgent {
            b.push_str(" ▲ BOSS INBOUND ");
        } else if elite_due {
            b.push_str(" ▲ ELITE ");
        } else if state.telemetry.muted {
            b.push_str(" MUTE ");
        } else if state.telemetry.enemy_count >= 55 {
            let _ = write!(b, " SWARM {} ", state.telemetry.enemy_count);
        } else {
            let _ = write!(b, " {} ", world.level.theme.name().to_uppercase());
        }
    });

    let has_rack = area.height >= 3;
    let top_row = Rect::new(area.x, area.y, area.width, 1);

    let buf = frame.buffer_mut();
    buf.set_style(top_row, Style::default().bg(PLATE));
    // The cursor is a `Cell`: the `put` closure below advances it, and the
    // sections that measure the remaining rail read it back.
    let x = std::cell::Cell::new(area.x);
    let right = area.right();

    let mut put = |s: &str, style: Style| {
        let mut cur = x.get();
        let avail = right.saturating_sub(cur) as usize;
        if avail == 0 {
            return;
        }
        (cur, _) = buf.set_stringn(cur, area.y, s, avail, style);
        x.set(cur);
    };

    let compact = !has_rack && area.width < 72 && weapons.len() > 3;
    let tight = area.width < 64;
    let show_threat = if has_rack {
        area.width >= 72
    } else {
        area.width >= 96
    };
    let show_cd_pip = area.width >= 82 || (area.width >= 68 && weapons.len() <= 4);
    // Active weapons get a charge cell and a READY token that names the key.
    // Below 68 columns there is no spare cell at all (a 3-weapon rack at 57 is
    // exactly full), so the ready state is carried by the slot's own highlight
    // instead — zero columns, same information.
    let show_active_gauge = area.width >= 68;
    let ready_token = if area.width >= 80 {
        state.active_name.as_str()
    } else {
        "SPC"
    };
    let show_icons = !tight || weapons.len() <= 3;
    let show_streak = (if has_rack { area.width >= 68 } else { area.width >= 92 })
        && !state.streak_buf.is_empty();

    let value = Style::default().fg(COAT).add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(MUTED);

    // 1. Clock badge + countdown (or the boss-fight state after the boss time).
    let badge_bg = if clock_boss {
        palette::to_color(palette::mix((220, 64, 118), (248, 116, 152), pulse))
    } else if urgent {
        palette::to_color(palette::mix((239, 168, 185), (248, 92, 102), pulse))
    } else if boss_soon {
        WARN_AMBER
    } else {
        accent
    };
    let badge_label = if clock_boss {
        if compact {
            "◆"
        } else {
            " ◆ "
        }
    } else if urgent {
        if compact {
            "!"
        } else {
            " ! "
        }
    } else if compact {
        "T"
    } else {
        " T "
    };
    put(
        badge_label,
        Style::default()
            .bg(badge_bg)
            .fg(PLATE)
            .add_modifier(Modifier::BOLD),
    );
    let time_style = if clock_boss {
        Style::default().fg(TAIL).add_modifier(Modifier::BOLD)
    } else if urgent {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    } else {
        value
    };
    put(" ", Style::default().bg(PLATE));
    put(&state.time, time_style);

    // 2. Threat escalation micro-pips on wider screens (`▪▪▪▫▫`).
    if show_threat {
        let tier = threat_tier(
            director::curve_clock(world.level_index(), world.elapsed),
            boss_alive || boss_phase,
        );
        put(" ", Style::default().bg(PLATE));
        for t in 1..=5u8 {
            let pip_col = if t <= tier {
                match tier {
                    5 => BOSS_ROSE,
                    4 => WARN_AMBER,
                    _ => accent,
                }
            } else {
                TRACK
            };
            put(
                if t <= tier { "▪" } else { "▫" },
                Style::default().fg(pip_col).bg(PLATE),
            );
        }
    }

    // 3. Level, Kills (+ optional streak), Score.
    let lv_style = if state.level_flash > 0.0 {
        Style::default().fg(TRAIL_GOLD).add_modifier(Modifier::BOLD)
    } else {
        value
    };
    put(if compact { " Lv" } else { "  Lv" }, muted);
    put(&state.level, lv_style);

    put(if compact { " KO" } else { "  KO " }, muted);
    put(&state.kills, value);
    if show_streak {
        put(
            &state.streak_buf,
            Style::default().fg(WARN_AMBER).add_modifier(Modifier::BOLD),
        );
    }

    put(if compact { " Score " } else { "  Score " }, muted);
    put(&state.score, value);
    put(if area.width < 68 { " ◉" } else { "  Gold " }, muted);
    put(&state.gold, Style::default().fg(TRAIL_GOLD).add_modifier(Modifier::BOLD));

    // 4. Inline single-row weapon rack (used when height < 3; taller HUDs draw
    //    the 2-row half-block pixel icon rack for weapons + passives below).
    if !has_rack {
        put(if compact { " │" } else { "  │" }, Style::default().fg(EDGE));
        let fire_rate = world.player.loadout.stats.fire_rate_mult.max(0.1);
        // The caps run to 8 + 8 and this degenerate single-row rail cannot hold
        // them: the slots it must drop are counted in a `+N` marker at its end,
        // never silently cut.
        let mut hidden = 0usize;
        for (i, ((tag, lvl), weapon)) in state.weapon_tags.iter().zip(weapons).enumerate() {
            if i + 1 < weapons.len() && right.saturating_sub(x.get()) < 8 {
                hidden = weapons.len() - i;
                break;
            }
            let eff_cd = (weapon.cooldown() / fire_rate).max(0.05);
            let ready = (1.0 - weapon.timer / eff_cd).clamp(0.0, 1.0);
            // Immediately after firing, `timer` resets near `eff_cd` (`ready` near 0):
            // flare the weapon icon white-hot while the cooldown bar starts low and climbs.
            let just_fired = ready < 0.12 && world.elapsed > 0.25;
            let base_rgb = weapon.kind.color();
            let w_col = palette::to_color(base_rgb);
            // Active weapons: is this one fully charged, and did it just finish?
            let active = weapon.kind.is_active();
            let charged = active && state.active_ready.get(i).copied().unwrap_or(false);
            let charge_flash = if active {
                state.active_flash.get(i).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            let icon_style = if just_fired || charge_flash > 0.0 {
                Style::default().fg(COAT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(w_col)
            };
            let w_style = if just_fired {
                Style::default()
                    .fg(palette::to_color(palette::mix(base_rgb, (255, 255, 255), 0.35)))
                    .add_modifier(Modifier::BOLD)
            } else if charged && !show_active_gauge {
                // No room for the READY token: light the whole slot instead, so a
                // charged weapon still reads as "spend me" on a small terminal.
                Style::default()
                    .fg(COAT)
                    .bg(TRACK)
                    .add_modifier(Modifier::BOLD)
            } else if charged {
                Style::default().fg(w_col).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(w_col)
            };

            put(" ", Style::default().bg(PLATE));
            if show_icons {
                let mut gbuf = [0u8; 4];
                let g = weapon_glyph(weapon.kind).encode_utf8(&mut gbuf);
                put(g, icon_style);
            }
            // Two letters distinguish Spore from Squeak even in a 57-col full loadout.
            let end = if compact {
                tag.char_indices().nth(2).map_or(tag.len(), |(i, _)| i)
            } else {
                tag.len()
            };
            put(&tag[..end], w_style);
            let lvl_style = if weapon.is_max() {
                Style::default().fg(TRAIL_GOLD).add_modifier(Modifier::BOLD)
            } else if charged && !show_active_gauge {
                Style::default().fg(COAT).bg(TRACK)
            } else {
                muted
            };
            put(lvl, lvl_style);

            // 4a. Active weapons: a one-cell charge gauge and a READY token that
            // names the key. A fully charged weapon shows the solid pip in its own
            // colour, and white-hot for half a second the moment it finishes.
            if active && show_active_gauge {
                let pip_ch = if charged { '█' } else { cooldown_pip(ready) };
                let pip_fg = if charge_flash > 0.0 {
                    COAT
                } else if charged {
                    w_col
                } else {
                    palette::to_color(palette::mix((82, 94, 100), base_rgb, 0.35 + ready * 0.65))
                };
                let mut pbuf = [0u8; 4];
                let p = pip_ch.encode_utf8(&mut pbuf);
                put(p, Style::default().fg(pip_fg).bg(TRACK));
                if charged {
                    let token_style = if charge_flash > 0.0 {
                        Style::default()
                            .fg(PLATE)
                            .bg(COAT)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(w_col).add_modifier(Modifier::BOLD)
                    };
                    put(ready_token, token_style);
                }
            } else if !active && show_cd_pip {
                let pip_ch = cooldown_pip(ready);
                let pip_fg = if ready >= 0.88 {
                    w_col
                } else {
                    palette::to_color(palette::mix((82, 94, 100), base_rgb, 0.35 + ready * 0.65))
                };
                let mut pbuf = [0u8; 4];
                let p = pip_ch.encode_utf8(&mut pbuf);
                put(p, Style::default().fg(pip_fg).bg(TRACK));
            }
        }
        if hidden > 0 {
            let mut mbuf = [0u8; 8];
            mbuf[0] = b' ';
            mbuf[1] = b'+';
            let mut digits = [0u8; 8];
            let n = fmt_u32_stack(hidden as u32, &mut digits);
            mbuf[2..2 + n].copy_from_slice(&digits[..n]);
            let text = std::str::from_utf8(&mbuf[..2 + n]).unwrap_or("");
            put(text, Style::default().fg(WARN_AMBER).add_modifier(Modifier::BOLD));
        }
    }

    // 5. Campaign progress pips, right-aligned at the rail's end (the art's
    //    "HUD / Pause" placement). The full `LV 2/3  ● ◆ ○  Kitchen` legend
    //    needs 24 columns, the compact `2/3 ●◆○` 9; both are only stamped when
    //    the status pill still fits to their left. The status pill then ends at
    //    `pill_right` instead of the row's edge.
    let badge_w = state.right_badge.chars().count() as u16;
    let spare = right.saturating_sub(x.get());
    let pips_w = if spare >= badge_w + campaign_pips_width(false) + 3 {
        campaign_pips_width(false)
    } else if spare >= badge_w + campaign_pips_width(true) + 3 {
        campaign_pips_width(true)
    } else {
        0
    };
    let pill_right = if pips_w > 0 {
        let pips_x = right - pips_w;
        let pips = CampaignProgressView {
            current: CampaignLevel::from_theme(world.level.theme),
            cleared_count: world.level_index().min(campaign::LEVEL_COUNT) as u8,
            bg: None,
        };
        draw_campaign_pips(frame, Rect::new(pips_x, area.y, pips_w, 1), &pips);
        pips_x.saturating_sub(1)
    } else {
        right
    };

    // 6. Right-aligned contextual status pill (only if it won't collide with
    //    the weapons or the campaign pips).
    let buf = frame.buffer_mut();
    if badge_w > 0 && pill_right.saturating_sub(x.get()) >= badge_w + 2 {
        let bx = pill_right - badge_w;
        // If audio is muted while a higher-priority status badge owns the right slot,
        // tuck a compact `MUTE` chip immediately to its left when space permits.
        if state.telemetry.muted
            && state.right_badge != " MUTE "
            && bx.saturating_sub(x.get()) >= 8
        {
            buf.set_stringn(
                bx - 7,
                area.y,
                " MUTE ",
                6,
                Style::default().fg(TAIL).bg(TRACK),
            );
        }
        let style = if urgent {
            Style::default()
                .fg(PLATE)
                .bg(palette::to_color(palette::mix(
                    (239, 168, 185),
                    (248, 92, 102),
                    pulse,
                )))
                .add_modifier(Modifier::BOLD)
        } else if clock_boss {
            Style::default()
                .fg(BOSS_ROSE)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else if boss_soon {
            Style::default()
                .fg(PLATE)
                .bg(WARN_AMBER)
                .add_modifier(Modifier::BOLD)
        } else if elite_due {
            Style::default()
                .fg(WARN_AMBER)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else if state.telemetry.muted {
            Style::default().fg(TAIL).bg(TRACK)
        } else if state.telemetry.enemy_count >= 55 {
            Style::default()
                .fg(WARN_AMBER)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(EDGE).bg(PLATE)
        };
        let style = if state.volume_flash.is_some() || state.zoom_flash.is_some() {
            Style::default()
                .fg(TRAIL_GOLD)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else {
            style
        };
        buf.set_stringn(bx, area.y, &state.right_badge, badge_w as usize, style);
    }

    // 7. 2-row half-block pixel icon rack for owned weapons and passives.
    if has_rack {
        let rack = Rect::new(area.x, area.y + 1, area.width, 2);
        draw_loadout_rack(frame, rack, world, state);
    }
}

/// How the 2-row equipment rack is laid out at one width: the cell footprint of
/// a weapon slot and a passive slot (a 4-cell half-block icon plus `text` cells
/// of tag / level / charge read-out), whether the divider fits, and how many
/// slots are shown with a `+N` marker for the rest.
///
/// One plan for every width, computed by [`plan_rack`]: the rack is the same
/// instrument at 57×20 and 120×40, only denser. Pure arithmetic, no allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RackPlan {
    /// Cells per weapon / passive slot (`4` = icon only).
    weapon_pitch: u16,
    passive_pitch: u16,
    /// Left margin before the first icon.
    margin: u16,
    /// Divider column between the two racks (0 when it does not fit).
    divider: u16,
    /// `+N` marker cells reserved at the end (0 when everything fits).
    marker: u16,
    /// Slots actually drawn (may be fewer than owned at 57 cols).
    weapons_shown: usize,
    passives_shown: usize,
    /// Owned slots the marker stands for.
    hidden: usize,
}

impl RackPlan {
    /// Total tag/level cells the plan can show.
    fn text_cells(&self) -> u16 {
        self.weapon_pitch.saturating_sub(4) * self.weapons_shown as u16
            + self.passive_pitch.saturating_sub(4) * self.passives_shown as u16
    }
}

/// Widest tag/read-out a weapon slot may grow to: icon + level + 4-char tag row
/// (and `SPACE` beside the charge pip on the second row needs 6).
const WEAPON_TEXT_MAX: u16 = 6;
/// Widest a passive slot may grow to: icon + `×N` + a 2-char tag.
const PASSIVE_TEXT_MAX: u16 = 4;
/// How many tail slots the planner will hide behind a `+N` marker to buy a
/// read-out for the slots that remain (see [`plan_rack_with`]).
const MAX_MARKER_DROPS: u16 = 2;

/// Hand out `spare` text cells in the order that answers the player's
/// questions: rank digit, stack digit, `×N`, two-char tags, then the
/// `SPC`/`SPACE` charge token. Returns `(weapon_text, passive_text, leftover)`.
fn assign_rack_text(spare: u16, nw: u16, np: u16) -> (u16, u16, u16) {
    let mut spare = spare;
    let (mut tw, mut tp) = (0u16, 0u16);
    for (weapons_target, passives_target) in [
        (1u16, 0u16),
        (1, 1),
        (1, 2),
        (3, 2),
        (3, 3),
        (4, 3),
        (6, 3),
        (6, 4),
    ] {
        let tw_target = tw.max(weapons_target.min(WEAPON_TEXT_MAX));
        let tp_target = tp.max(passives_target.min(PASSIVE_TEXT_MAX));
        let cost = (tw_target - tw) * nw + (tp_target - tp) * np;
        if cost <= spare {
            spare -= cost;
            tw = tw_target;
            tp = tp_target;
        }
    }
    (tw, tp, spare)
}

/// Lay out the equipment rack for `width` cells and `slots_w` + `slots_p` slots
/// (owned kinds and the empty slots up to the loadout's cap).
///
/// The rule, in order: every slot owns a 4-cell icon; when the icons alone do
/// not fit, the tail of the racks is dropped behind a `+N` marker (never
/// silently cut); the spare width then buys text cells (see
/// [`assign_rack_text`]). If a rack of bare icons would leave whole slots with
/// no number at all, up to [`MAX_MARKER_DROPS`] more tail slots are traded for
/// a read-out on the rest — a numbered rack plus an honest `+N` beats sixteen
/// anonymous sprites. The divider column is kept unless the cell it costs
/// would show another slot or give a whole rack its first read-out (a rank or
/// a stack digit): a longer tag is not worth an unseparated rack.
fn plan_rack(width: u16, slots_w: usize, slots_p: usize) -> RackPlan {
    let with = plan_rack_with(width, slots_w, slots_p, 1);
    let without = plan_rack_with(width, slots_w, slots_p, 0);
    let shown = |p: &RackPlan| p.weapons_shown + p.passives_shown;
    let numbered = |p: &RackPlan| {
        u8::from(p.weapon_pitch > 4 && p.weapons_shown > 0)
            + u8::from(p.passive_pitch > 4 && p.passives_shown > 0)
    };
    if shown(&without) > shown(&with) || numbered(&without) > numbered(&with) {
        without
    } else {
        with
    }
}

fn plan_rack_with(width: u16, slots_w: usize, slots_p: usize, divider: u16) -> RackPlan {
    let divider = if slots_w == 0 || slots_p == 0 { 0 } else { divider };
    let (mut shown_w, mut shown_p) = (slots_w, slots_p);
    let mut marker = 0u16;
    let mut drops = 0u16;
    // Icons first: drop the tail of the passives, then of the weapons, and
    // reserve two cells for the `+N` marker as soon as anything is hidden.
    let fit = |shown_w: &mut usize, shown_p: &mut usize, marker: &mut u16| {
        while 4 * (*shown_w + *shown_p) as u16 + divider + *marker > width {
            if *shown_p > 0 {
                *shown_p -= 1;
            } else if *shown_w > 0 {
                *shown_w -= 1;
            } else {
                break;
            }
            *marker = if *shown_w + *shown_p < slots_w + slots_p { 2 } else { 0 };
        }
    };
    fit(&mut shown_w, &mut shown_p, &mut marker);
    let mut tw;
    let mut tp;
    let mut spare;
    loop {
        let room = width.saturating_sub(4 * (shown_w + shown_p) as u16 + divider + marker);
        let assigned = assign_rack_text(room, shown_w as u16, shown_p as u16);
        tw = assigned.0;
        tp = assigned.1;
        spare = assigned.2;
        let lacking = (if tw == 0 { shown_w as u16 } else { 0 })
            + (if tp == 0 { shown_p as u16 } else { 0 });
        if lacking == 0 || spare >= lacking || shown_w + shown_p <= 1 || drops >= MAX_MARKER_DROPS {
            break;
        }
        // Trade a tail slot for read-outs on the rest.
        if shown_p > 0 {
            shown_p -= 1;
        } else {
            shown_w = shown_w.saturating_sub(1);
        }
        marker = 2;
        drops += 1;
        fit(&mut shown_w, &mut shown_p, &mut marker);
    }
    // Whatever is left pads the left margin and the divider gap so a partly
    // filled rack still reads as a row of slots and not as a clump.
    let margin = u16::from(spare >= 2 && shown_w + shown_p > 0);
    let divider = if divider > 0 && spare >= margin + 2 {
        divider + 1
    } else {
        divider
    };
    RackPlan {
        weapon_pitch: 4 + tw,
        passive_pitch: 4 + tp,
        margin,
        divider,
        marker,
        weapons_shown: shown_w,
        passives_shown: shown_p,
        hidden: slots_w + slots_p - shown_w - shown_p,
    }
}

/// 2-row tactical equipment rack: small (`4×4` px = `4×2` cell) half-block
/// icons for every owned weapon (left) and passive (right) plus a dim outline
/// for each **empty slot up to the loadout's caps**, so the rack shows both the
/// build and how much room it has left (`loadout::SlotCaps`, 4 + 4 on a fresh
/// profile up to 8 + 8).
///
/// One layout at every size: rank digit, stack count, tags and the charge
/// token are bought with whatever width is left after the icons, and a `+N`
/// marker stands in for the slots that do not fit (57×20 with a full 8 + 8
/// rack). Zero per-frame heap allocation.
fn draw_loadout_rack(frame: &mut Frame, rack: Rect, world: &World, state: &HudState) {
    if rack.width < 8 || rack.height < 2 {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(rack, Style::default().bg(RACK_BG));

    let loadout = &world.player.loadout;
    let weapons = &loadout.weapons;
    let passives = &loadout.passives;
    let fire_rate = loadout.stats.fire_rate_mult.max(0.1);

    // Slots on show: everything owned plus the empty slots up to the cap (a
    // carried loadout can hold more than a lowered cap — keep it visible).
    let slots_w = weapons.len().max(loadout.max_weapons()).min(loadout::MAX_WEAPONS);
    let slots_p = passives.len().max(loadout.max_passives()).min(loadout::MAX_PASSIVES);
    let plan = plan_rack(rack.width, slots_w, slots_p);

    let y0 = rack.y;
    let y1 = rack.y + 1;
    let right = rack.right();
    let nw = plan.weapons_shown;
    let np = plan.passives_shown;

    let mut x = rack.x + plan.margin;
    // ---- Weapons (left side) ----------------------------------------------
    let mut stack_buf = [0u8; 8];
    for (i, ((tag, lvl), weapon)) in state.weapon_tags.iter().zip(weapons).enumerate() {
        if i >= nw {
            break;
        }
        let icon_w = art_icons::SMALL_CELL_W;
        if x + icon_w > right {
            break;
        }
        let text_w = plan.weapon_pitch - icon_w;

        let eff_cd = (weapon.cooldown() / fire_rate).max(0.05);
        let ready = (1.0 - weapon.timer / eff_cd).clamp(0.0, 1.0);
        let just_fired = ready < 0.12 && world.elapsed > 0.25;
        let base_rgb = weapon.kind.color();
        let w_col = palette::to_color(base_rgb);
        let active = weapon.kind.is_active();
        let charged = active && state.active_ready.get(i).copied().unwrap_or(false);
        let charge_flash = if active {
            state.active_flash.get(i).copied().unwrap_or(0.0)
        } else {
            0.0
        };

        // 1. Small 4×4 half-block icon; an evolved weapon wears the gilded
        // evolved frame (`art_icons::draw_evolved`).
        let icon_rect = Rect::new(x, y0, icon_w, art_icons::SMALL_CELL_H);
        if weapon.evolved {
            art_icons::draw_evolved(buf, icon_rect, art_icons::weapon(weapon.kind));
        } else {
            art_icons::draw(buf, icon_rect, art_icons::weapon(weapon.kind));
        }
        let tx = x + icon_w;
        x = tx + text_w;

        if text_w == 0 {
            continue;
        }
        let pip_ch = if active && charged {
            '█'
        } else {
            cooldown_pip(ready)
        };
        let pip_fg = if charge_flash > 0.0 {
            COAT
        } else if (active && charged) || (!active && ready >= 0.88) {
            w_col
        } else {
            palette::to_color(palette::mix((82, 94, 100), base_rgb, 0.35 + ready * 0.65))
        };
        let mut pbuf = [0u8; 4];
        let pip_s = pip_ch.encode_utf8(&mut pbuf);

        // Row 0: the rank (the number the player is working towards), then as
        // much of the slot's identity tag as the width allows.
        let lvl_style = if weapon.is_max() {
            Style::default()
                .fg(TRAIL_GOLD)
                .bg(RACK_BG)
                .add_modifier(Modifier::BOLD)
        } else if just_fired || charge_flash > 0.0 {
            Style::default()
                .fg(COAT)
                .bg(RACK_BG)
                .add_modifier(Modifier::BOLD)
        } else if charged {
            Style::default().fg(w_col).bg(RACK_BG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED).bg(RACK_BG)
        };
        let name_style = if just_fired || charge_flash > 0.0 {
            Style::default()
                .fg(COAT)
                .bg(RACK_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(w_col).bg(RACK_BG)
        };
        let tag_room = (text_w - 1) as usize;
        let tag_end = tag.char_indices().nth(tag_room.min(4)).map_or(tag.len(), |(k, _)| k);
        let (end, _) = buf.set_stringn(tx, y0, lvl, 1, lvl_style);
        if tag_room > 0 {
            buf.set_stringn(end, y0, &tag[..tag_end], right.saturating_sub(end) as usize, name_style);
        }

        // Row 1: the charge read-out. A charged active names the key where the
        // width can pay for it (`SPACE` 6 cells, `SPC` 4), and otherwise keeps
        // the solid pip and the slot's highlight (`charged` styles above).
        buf.set_stringn(tx, y1, pip_s, 1, Style::default().fg(pip_fg).bg(TRACK));
        let bar_room = text_w - 1;
        if active && charged && bar_room >= 3 {
            let tok_style = if charge_flash > 0.0 {
                Style::default()
                    .fg(PLATE)
                    .bg(COAT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(w_col)
                    .bg(TRACK)
                    .add_modifier(Modifier::BOLD)
            };
            // The live binding's label where it fits, else three letters of
            // it (`SPACE` -> `SPC`, the abbreviation the HUD has always used).
            let name = state.active_name.as_str();
            let tok = if name.chars().count() <= bar_room as usize {
                name
            } else if name == "SPACE" {
                "SPC"
            } else {
                let cut = name.char_indices().nth(3).map_or(name.len(), |(k, _)| k);
                &name[..cut]
            };
            buf.set_stringn(tx + 1, y1, tok, bar_room as usize, tok_style);
        } else if bar_room > 0 {
            draw_mini_bar(buf, tx + 1, y1, bar_room.min(4), ready, w_col);
        }
    }

    // ---- Divider + Passives (right side) ----------------------------------
    let mut px = x.max(rack.x + plan.margin + nw as u16 * plan.weapon_pitch) + plan.divider;
    if np > 0 && plan.divider > 0 {
        let div_x = px - plan.divider;
        if div_x < right {
            buf[(div_x, y0)].set_char('│').set_fg(EDGE).set_bg(RACK_BG);
            buf[(div_x, y1)].set_char('│').set_fg(EDGE).set_bg(RACK_BG);
        }
    }
    for (i, &(kind, stacks)) in passives.iter().enumerate() {
        if i >= np {
            break;
        }
        let icon_w = art_icons::SMALL_CELL_W;
        if px + icon_w > right {
            break;
        }
        let icon = art_icons::passive(kind);
        art_icons::draw(
            buf,
            Rect::new(px, y0, icon_w, art_icons::SMALL_CELL_H),
            icon,
        );
        let tx = px + icon_w;
        let text_w = plan.passive_pitch - icon_w;
        px = tx + text_w;
        if text_w == 0 {
            continue;
        }

        let capped = stacks >= kind.max_stacks();
        let stk_style = if capped {
            Style::default()
                .fg(TRAIL_GOLD)
                .bg(RACK_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(palette::to_color(icon.accent))
                .bg(RACK_BG)
                .add_modifier(Modifier::BOLD)
        };
        let tag_style = Style::default().fg(MUTED).bg(RACK_BG);
        if text_w == 1 {
            // One cell: the stack digit on its own (`×` is implied by the
            // colour and by the dossier's `Sharp Teeth ×3`).
            let digit = stacks.min(9) as u8;
            stack_buf[0] = b'0' + digit;
            let s = std::str::from_utf8(&stack_buf[..1]).unwrap_or("1");
            buf.set_stringn(tx, y0, s, 1, stk_style);
            continue;
        }
        // Row 0: `×N` (the stack, never confusable with a weapon's rank).
        let (e, _) = buf.set_stringn(tx, y0, "×", 1, tag_style);
        let s_len = fmt_u32_stack(stacks, &mut stack_buf);
        let stk_str = std::str::from_utf8(&stack_buf[..s_len]).unwrap_or("1");
        let (e, _) = buf.set_stringn(e, y0, stk_str, text_w.saturating_sub(1) as usize, stk_style);
        // Row 1: the passive's own tag, as wide as the slot can pay for.
        let ptag = passive_short_tag(kind);
        let room = (text_w.saturating_sub(e - tx)) as usize;
        if room > 0 {
            let end = ptag.char_indices().nth(room).map_or(ptag.len(), |(k, _)| k);
            buf.set_stringn(tx, y1, &ptag[..end], room, tag_style);
        }
    }

    // ---- Empty slots up to the cap, then the `+N` marker -------------------
    // Every slot the build has not filled keeps its icon-sized footprint as a
    // dim outline, so the racks read as 4 + 4 (or 8 + 8) bays, not as a pile.
    let mut draw_placeholders = |from: usize, upto: usize, x0: u16, pitch: u16| {
        let mut hx = x0;
        for _ in from..upto {
            if hx + 4 > right {
                break;
            }
            buf.set_stringn(
                hx + 1,
                y1,
                "·",
                1,
                Style::default().fg(TRACK).bg(RACK_BG),
            );
            buf.set_stringn(
                hx + 2,
                y1,
                "·",
                1,
                Style::default().fg(TRACK).bg(RACK_BG),
            );
            hx += pitch;
        }
        hx
    };
    let weapons_x =
        rack.x + plan.margin + weapons.len().min(nw) as u16 * plan.weapon_pitch;
    let _ = draw_placeholders(weapons.len().min(nw), nw, weapons_x, plan.weapon_pitch);
    let passives_x0 = rack.x + plan.margin + nw as u16 * plan.weapon_pitch + plan.divider;
    let passives_x = passives_x0 + passives.len().min(np) as u16 * plan.passive_pitch;
    let _ = draw_placeholders(passives.len().min(np), np, passives_x, plan.passive_pitch);

    // A rack that could not show every slot says so: nothing is silently cut.
    if plan.hidden > 0 && plan.marker > 0 {
        let mut mbuf = [0u8; 8];
        mbuf[0] = b'+';
        let digits = fmt_u32_stack(plan.hidden as u32, &mut stack_buf);
        mbuf[1..1 + digits].copy_from_slice(&stack_buf[..digits]);
        let s = std::str::from_utf8(&mbuf[..1 + digits]).unwrap_or("+");
        let mx = right.saturating_sub(plan.marker);
        buf.set_stringn(
            mx,
            y0,
            s,
            plan.marker as usize,
            Style::default().fg(WARN_AMBER).bg(RACK_BG),
        );
        buf.set_stringn(
            mx,
            y1,
            "·",
            plan.marker as usize,
            Style::default().fg(TRACK).bg(RACK_BG),
        );
    }
}

/// The short name a pocket find wears on the HUD's timed strip.
fn buff_chip_name(kind: PickupKind) -> &'static str {
    match kind {
        PickupKind::BottleCap => "CAP",
        PickupKind::WindupMouse => "MOUSE",
        PickupKind::PocketWatch => "WATCH",
        PickupKind::CleanSocks => "SOCKS",
        PickupKind::Heal => "HEAL",
        PickupKind::Magnet => "MAGNET",
        PickupKind::Nuke => "PURGE",
    }
}

/// Format a small `u32` into a stack byte buffer without heap allocation.
fn fmt_u32_stack(mut n: u32, dst: &mut [u8; 8]) -> usize {
    if n == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 8];
    let mut len = 0usize;
    while n > 0 && len < tmp.len() {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for i in 0..len {
        dst[i] = tmp[len - 1 - i];
    }
    len
}

pub fn draw_bottom(frame: &mut Frame, area: Rect, world: &World, state: &mut HudState) {
    if area.is_empty() {
        return;
    }

    // The timed-buff strip above this rail names its finds when there is room
    // (`CAP 12 MOUSE 5`) and falls back to the one-letter chips when there is
    // not (`B12 M05`). Read next tick by `HudState::tick`.
    state.wide_chips = area.width >= 96;

    let max_hp = world.player.max_hp().max(1.0);
    let hp_val = world.player.hp.max(0.0);
    let hp = (hp_val / max_hp).clamp(0.0, 1.0);
    let low_hp = hp <= 0.28;
    let pulse = 0.5 + 0.5 * (state.clock * 8.0).sin();

    let hp_color = if low_hp {
        palette::to_color(palette::mix((214, 58, 70), (255, 110, 118), pulse))
    } else if hp <= 0.55 {
        palette::to_color(palette::mix((244, 154, 114), (239, 168, 185), (hp - 0.28) / 0.27))
    } else {
        TAIL
    };

    let hurt_active = world.player.hurt_flash > 0.0 || state.hurt_edge > 0.15;
    refill(&mut state.hp, |b| {
        let _ = write!(b, "♥ {:.0}/{:.0}", hp_val, max_hp);
        // The review's 80×24 capture lost the armour suffix; it costs three
        // cells on a 30-cell HP meter and fits from 80 columns up.
        if area.width >= 80 && world.player.loadout.stats.armor > 0.0 {
            let _ = write!(b, " A{:.0}", world.player.loadout.stats.armor);
        }
    });

    let xp_next = world.player.xp_to_next.max(1);
    let xp = (world.player.xp as f32 / xp_next as f32).clamp(0.0, 1.0);
    refill(&mut state.xp_label, |b| {
        // The review's 57×20 and 80×24 captures both lost the percentage, so
        // the one number that answers "how close is the next card" was missing
        // on the two commonest sizes. `XP 42%` is 6 cells and still leaves the
        // meter a real bar at every supported play size (57×20 included).
        if area.width >= 56 {
            let pct = (xp * 100.0).round() as u32;
            let _ = write!(b, "XP {:>2}%", pct.min(99));
        } else {
            b.push_str("XP");
        }
    });

    // Dash module: a labelled charge bar that always shows its state. The key
    // is printed in the label because the binding depends on the terminal
    // (CTRL where a lone modifier press reports, SPACE where it cannot).
    let dashing = world.player.dash_active > 0.0;
    let dash_ready = world.player.dash_cd <= 0.0;
    let dash_max_cd = world.player.loadout.stats.dash_cd.max(0.1);
    let dash_fill = if dash_ready || dashing {
        1.0
    } else {
        (1.0 - world.player.dash_cd / dash_max_cd).clamp(0.0, 1.0)
    };
    // A press that could not be spent (the cooldown is still running): the
    // module goes red for a beat so a refused dash is never silent.
    let dash_denied = !dash_ready && world.player.dash_denied > 0.0;
    refill(&mut state.dash, |b| {
        if area.width >= 44 {
            let _ = write!(b, "{} DASH", state.dash_name);
        } else {
            b.push_str("DASH");
        }
    });
    refill(&mut state.dash_status, |b| {
        if dashing {
            b.push_str("DASHING");
        } else if dash_ready {
            b.push_str("READY");
        } else if dash_denied {
            b.push_str("WAIT");
        } else {
            let _ = write!(b, "{:.1}s", world.player.dash_cd);
        }
    });

    let accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    let xp_color = if state.level_flash > 0.0 {
        palette::to_color(palette::mix(
            rgb_of_color(accent),
            (252, 226, 152),
            (state.level_flash / 0.85).clamp(0.0, 1.0),
        ))
    } else if xp >= 0.85 {
        palette::to_color(palette::mix(
            world.level.palette.accent,
            (248, 232, 186),
            0.45 + 0.25 * pulse,
        ))
    } else {
        palette::to_color(palette::mix(
            world.level.palette.accent,
            (176, 198, 194),
            0.42,
        ))
    };

    // Reserve dash module width: `│ ` divider, key+DASH label, an eighth-cell
    // charge bar (always drawn: the refill is the point) and the state badge.
    let show_sep = area.width >= 52;
    let sep_w: u16 = if show_sep { 2 } else { 0 };
    let bar_w: u16 = if area.width >= 88 {
        8
    } else if area.width >= 72 {
        6
    } else if area.width >= 52 {
        4
    } else {
        0
    };
    let bar_block: u16 = if bar_w > 0 { bar_w + 2 } else { 0 };
    let dash_text_w = state.dash.chars().count() as u16;
    // Pad the state word to a steady width so READY ⇄ 3.4s does not shove the
    // HP/XP divider around (only DASHING, which is 0.16 s, is wider).
    let status_w = (state.dash_status.chars().count() as u16).max(5);
    let dash_width = (sep_w + dash_text_w + bar_block + status_w + 2).min(area.width);
    let meters = area.width.saturating_sub(dash_width);
    let left = (meters / 2 + 3).min(meters);

    // 1. HP meter with trailing damage ghost bar.
    let hp_label_style = if hurt_active {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    } else if low_hp {
        Style::default().fg(hp_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(COAT)
    };
    meter_with_trail(
        frame,
        Rect::new(area.x, area.y, left, 1),
        &state.hp,
        hp_label_style,
        hp,
        state.hp_trail,
        hp_color,
        TRAIL_GOLD,
    );

    // 2. XP meter.
    meter_with_trail(
        frame,
        Rect::new(area.x + left, area.y, meters.saturating_sub(left), 1),
        &state.xp_label,
        Style::default().fg(COAT),
        xp,
        xp,
        xp_color,
        xp_color,
    );

    // 3. Dash module: divider, key label, charge bar, state badge.
    let dash_rect = Rect::new(area.right() - dash_width, area.y, dash_width, 1);
    let buf = frame.buffer_mut();
    buf.set_style(dash_rect, Style::default().bg(PLATE));
    let mut dx = dash_rect.x;
    if show_sep && dx < dash_rect.right() {
        buf[(dx, dash_rect.y)]
            .set_char('│')
            .set_fg(EDGE)
            .set_bg(PLATE);
        dx = dx.saturating_add(2);
    }
    // The label: muted while recharging, bright when the dash is back, red on a
    // refused press.
    let label_style = if dash_denied {
        Style::default().fg(DANGER).bg(PLATE).add_modifier(Modifier::BOLD)
    } else if dash_ready || dashing {
        Style::default().fg(COAT).bg(PLATE).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED).bg(PLATE)
    };
    // The badge is the loud part: an accent plate for READY (flashing to near
    // white when it first comes back), danger red for a refused press, and a
    // plain amber countdown while it recharges.
    let badge_style = if dash_ready {
        // Either the HUD's own edge detection or the sim's `dash_ready_flash`
        // (which also drives the on-rat gauge) lights the just-became-ready pop.
        let bg = if state.dash_ready_flash > 0.0 || world.player.dash_ready_flash > 0.0 {
            COAT
        } else {
            accent
        };
        Style::default().fg(PLATE).bg(bg).add_modifier(Modifier::BOLD)
    } else if dash_denied {
        Style::default()
            .fg(PLATE)
            .bg(DANGER)
            .add_modifier(Modifier::BOLD)
    } else if dashing {
        Style::default()
            .fg(PLATE)
            .bg(COAT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(WARN_AMBER).bg(PLATE)
    };
    let bar_color = if dash_ready {
        accent
    } else if dash_denied {
        DANGER_DIM
    } else {
        WARN_AMBER
    };
    let avail = dash_rect.right().saturating_sub(dx) as usize;
    if avail > 0 {
        let (mut end, _) = buf.set_stringn(dx, dash_rect.y, &state.dash, avail, label_style);
        if bar_w > 0 && end + bar_w < dash_rect.right() {
            let bx = end.saturating_add(1);
            draw_mini_bar(buf, bx, dash_rect.y, bar_w, dash_fill, bar_color);
            end = bx + bar_w;
        }
        let sx = end.saturating_add(1).min(dash_rect.right());
        let room = dash_rect.right().saturating_sub(sx) as usize;
        if room > 0 {
            let (end, _) = buf.set_stringn(sx, dash_rect.y, &state.dash_status, room, badge_style);
            if end < dash_rect.right() {
                buf[(end, dash_rect.y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(PLATE));
            }
        }
    }

    draw_buff_strip(frame, area, &state.buffs);
    draw_nest_chip(frame, area, &state.nest);
}

/// The nest chip: `⌂ NEST 7` on the strip row above the bottom rail, hugging its
/// **right** edge (the buff strip owns the left, the low-health pill the centre).
///
/// Drawn only when the run has permanent upgrades — a fresh profile's run shows
/// nothing, exactly as before the nest existed — and only from 44 columns up,
/// where the strip row has room for it beside the buffs.
pub(crate) fn draw_nest_chip(frame: &mut Frame, area: Rect, text: &str) {
    if text.is_empty() || area.y == 0 || area.width < 44 {
        return;
    }
    let w = (text.chars().count() as u16).min(area.width.saturating_sub(4));
    if w == 0 {
        return;
    }
    let x = area.right().saturating_sub(w + 1);
    frame.buffer_mut().set_stringn(
        x,
        area.y - 1,
        text,
        w as usize,
        Style::default().fg(TRAIL_GOLD).bg(PLATE),
    );
}

/// The wave-3 pocket-find strip: `B12 M05 P04 S06`.
///
/// One chip per active timed buff, smallest-first in the design's order, drawn
/// on the row directly above the bottom rail and hugging its left edge. That
/// row belongs to the pixel viewport, so the strip is only ever a short
/// left-aligned run of cells: the low-health pill owns the centre and the
/// damage brackets own the edges, and an empty string (no buff running) writes
/// nothing at all. Zero allocation — the text is the retained `HudState::buffs`
/// buffer.
pub(crate) fn draw_buff_strip(frame: &mut Frame, area: Rect, text: &str) {
    if text.is_empty() || area.y == 0 || area.width < 12 {
        return;
    }
    let w = (text.chars().count() as u16).min(area.width.saturating_sub(2));
    if w == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_stringn(
        area.x + 1,
        area.y - 1,
        text,
        w as usize,
        Style::default().fg(BRASS_KEY).bg(PLATE),
    );
}

/// Draw a compact eighth-cell bar directly into `buf` with no label.
pub(crate) fn draw_mini_bar(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    width: u16,
    ratio: f32,
    color: Color,
) {
    const STEPS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let units = (ratio.clamp(0.0, 1.0) * f32::from(width) * 8.0).round() as usize;
    for i in 0..width {
        let step = units.saturating_sub(8 * i as usize).min(8);
        buf[(x + i, y)]
            .set_char(STEPS[step])
            .set_fg(color)
            .set_bg(TRACK);
    }
}

/// Labels occupy their own dark strip so their contrast is independent of fill.
/// Supports an optional trailing damage-ghost bar (`trail > ratio`).
#[allow(clippy::too_many_arguments)]
fn meter_with_trail(
    frame: &mut Frame,
    r: Rect,
    label: &str,
    label_style: Style,
    ratio: f32,
    trail: f32,
    color: Color,
    trail_color: Color,
) {
    const STEPS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    if r.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(r, Style::default().bg(PLATE));
    let (end, _) = buf.set_stringn(r.x, r.y, label, r.width as usize, label_style);
    let start = end.saturating_add(1).min(r.right());
    let width = r
        .right()
        .saturating_sub(start + u16::from(start < r.right()));
    if width == 0 {
        return;
    }
    let ratio = ratio.clamp(0.0, 1.0);
    let trail = trail.clamp(ratio, 1.0);
    let units = (ratio * f32::from(width) * 8.0).round() as usize;
    let trail_units = (trail * f32::from(width) * 8.0).round() as usize;
    for i in 0..width {
        let cell_base = 8 * i as usize;
        let fill_step = units.saturating_sub(cell_base).min(8);
        let trail_step = trail_units.saturating_sub(cell_base).min(8);
        let cell = &mut buf[(start + i, r.y)];
        if fill_step == 8 {
            cell.set_char('█').set_fg(color).set_bg(TRACK);
        } else if fill_step > 0 {
            let bg = if trail_step > fill_step {
                trail_color
            } else {
                TRACK
            };
            cell.set_char(STEPS[fill_step]).set_fg(color).set_bg(bg);
        } else if trail_step > 0 {
            cell.set_char(STEPS[trail_step])
                .set_fg(trail_color)
                .set_bg(TRACK);
        } else {
            cell.set_char(' ').set_fg(color).set_bg(TRACK);
        }
    }
}

/// Preserved thin wrapper for unit testing single-fill eighth-cell precision.
#[cfg(test)]
fn meter(frame: &mut Frame, r: Rect, label: &str, ratio: f32, color: Color) {
    meter_with_trail(
        frame,
        r,
        label,
        Style::default().fg(COAT),
        ratio,
        ratio,
        color,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::{Level, Theme};
    use crate::game::weapon::Weapon;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn push_short_name_is_char_safe() {
        let mut s = String::new();
        push_short_name(&mut s, "Gnaw");
        assert_eq!(s, "Gnaw");

        s.clear();
        push_short_name(&mut s, "Teeth");
        assert_eq!(s, "Teet");

        s.clear();
        push_short_name(&mut s, "");
        assert_eq!(s, "");

        // A multi-byte lead character must not be sliced mid-codepoint.
        // (This used to be the dagger emoji, which is two codepoints — dagger
        // plus U+FE0F VARIATION SELECTOR-16 — and renders double-width in most
        // terminals. See `TERMINALS.md`: no emoji or variation selector is left
        // in the source.)
        s.clear();
        push_short_name(&mut s, "×knife");
        assert_eq!(s.chars().count(), 4);
        assert!(s.starts_with('×'));
    }

    fn known_world() -> World {
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = 65.0; // boss at 900 s → 13:55 left in the build-up
        w.player.level = 3;
        w.kills = 42;
        w.score = 1234;
        w.player.loadout.weapons[0].level = 2;
        w.player.hp = 100.0;
        w.player.dash_cd = 0.0;
        w
    }

    /// Does any cell use the danger red (the urgent clock text)?
    fn has_danger_text(terminal: &Terminal<TestBackend>) -> bool {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.fg == DANGER)
    }

    fn row_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn top_line_shows_clock_counters_and_weapon_icon() {
        let w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_top(f, area, &w, &mut state);
            })
            .unwrap();

        let text = row_text(&terminal);
        assert!(text.contains("13:55"), "missing countdown in {text:?}");
        assert!(text.contains("Lv3"), "missing level in {text:?}");
        assert!(text.contains("KO 42"), "missing kills in {text:?}");
        assert!(text.contains("Score 1234"), "missing score in {text:?}");
        // The slot prints its identity tag (`weapon_tag`), not the head of its
        // name: `Tail Whip` and `Tail Thrash` both began "Ta" that way.
        assert!(text.contains("▸GN2"), "missing icon + weapon in {text:?}");
    }

    /// A zoom change flashes its setting in the status pill slot (the wheel and
    /// the `Z`/`X` keys have no other tell), and the read-out ages out again.
    #[test]
    fn a_zoom_change_flashes_its_setting_in_the_status_pill() {
        let w = known_world();
        let mut state = HudState {
            zoom_flash: None,
            ..HudState::default()
        };
        state.notify_zoom(1.5);
        let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let text = row_text(&terminal);
        assert!(
            text.contains("ZOOM 1.5x"),
            "the zoom read-out is missing: {text:?}"
        );
        // Whole rungs read as whole numbers, and the flash is transient.
        state.notify_zoom(2.0);
        let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        assert!(row_text(&terminal).contains("ZOOM 2x"));
        state.tick_zoom_flash(ZOOM_FLASH_SECS + 0.01);
        assert!(state.zoom_flash.is_none(), "the read-out ages out");
    }

    /// The campaign pips are stamped into the top rail's right end (the art's
    /// "HUD / Pause" placement), so which sector of the campaign the run is in
    /// is visible without opening the pause plate. They only appear where the
    /// row can afford them together with the status pill.
    #[test]
    fn the_top_rail_stamps_the_campaign_pips_where_there_is_room() {
        // Level 2 of 3: the Sewer is cleared (filled pip), the Kitchen is
        // active (diamond) and the Lab is still ahead (hollow pip).
        let w = World::new_level(
            1,
            7,
            crate::game::loadout::Loadout::new(WeaponKind::Gnaw),
        );
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let text = row_text(&terminal);
        assert!(
            text.contains("LV 2/3"),
            "no campaign pips on the rail: {text:?}"
        );
        assert!(
            text.contains('●'),
            "the cleared Sewer pip is missing: {text:?}"
        );
        assert!(
            text.contains('◆'),
            "the active Kitchen pip is missing: {text:?}"
        );
        assert!(
            text.trim_end().ends_with("Kitchen"),
            "the pips are not right-aligned: {text:?}"
        );

        // A 57-column rail cannot afford them: the clock keeps its slot and the
        // pips drop out rather than overprinting the weapons or the pill.
        let mut narrow = Terminal::new(TestBackend::new(57, 1)).unwrap();
        narrow
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let text = row_text(&narrow);
        assert!(
            !text.contains("LV 2/3"),
            "the pips overprinted a 57-column rail: {text:?}"
        );
        assert!(
            text.contains("15:00"),
            "the clock lost its slot: {text:?}"
        );
    }

    #[test]
    fn bottom_line_text_matches_reference_and_reuse_is_clean() {
        let mut w = known_world();
        // The base pool is `Stats::default().max_hp` (130 since the early-game
        // balance pass); the rail prints current/max, so read both from the
        // world instead of pinning the 100 HP of the pre-balance build.
        w.player.hp = w.player.max_hp();
        let full = w.player.max_hp();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(57, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_bottom(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("XP"));
        assert!(
            text.contains(&format!("♥ {full:.0}/{full:.0}")),
            "HP missing in {text:?}"
        );
        assert!(
            text.contains("SPACE DASH") && text.contains("READY"),
            "the ready dash module is missing or unnamed: {text:?}"
        );
        assert!(
            text.contains('█'),
            "a ready dash must show a full charge bar: {text:?}"
        );

        // Shrink every number, reusing the same buffers: no stale digits may survive.
        w.player.hp = 1.0;
        w.player.dash_cd = 0.25;
        w.elapsed = 299.9;
        w.kills = 1;
        w.score = 2;
        w.player.level = 1;
        terminal
            .draw(|f| {
                let area = f.area();
                draw_bottom(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("XP"));
        assert!(
            text.contains(&format!("♥ 1/{full:.0}")),
            "HP not updated in {text:?}"
        );
        assert!(
            text.contains("0.2s"),
            "dash cd not updated in {text:?}"
        );
        assert!(
            !text.contains("READY"),
            "the module still claims READY while charging: {text:?}"
        );
    }

    #[test]
    fn repeated_draws_are_identical() {
        let w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();

        let mut draw = |state: &mut HudState| {
            terminal
                .draw(|f| {
                    let area = f.area();
                    draw_top(f, area, &w, state);
                    let bottom = Rect::new(0, 1, area.width, 1);
                    draw_bottom(f, bottom, &w, state);
                })
                .unwrap();
            row_text(&terminal)
        };

        let first = draw(&mut state);
        let second = draw(&mut state);
        assert_eq!(first, second, "reused buffers must render identically");
        assert!(
            first.contains('♥') && first.contains("SPACE DASH") && first.contains("READY"),
            "bottom bars should still render: {first:?}"
        );
    }

    /// The colours the bottom rail paints: `(backgrounds, foregrounds)`.
    fn rail_colors(terminal: &Terminal<TestBackend>) -> (Vec<Color>, Vec<Color>) {
        let cells = &terminal.backend().buffer().content;
        (
            cells.iter().map(|c| c.bg).collect(),
            cells.iter().map(|c| c.fg).collect(),
        )
    }

    /// Spring Heel shortens the cooldown the dash module reports: the countdown
    /// a real dash starts from, and the bar's full scale.
    #[test]
    fn spring_heel_stacks_shorten_the_cooldown_the_dash_module_reports() {
        use crate::game::loadout::{PassiveKind, Upgrade};
        let (cols, rows) = (120u16, 40u16);
        let render = |w: &World| {
            let mut state = HudState::default();
            let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
            terminal
                .draw(|f| {
                    let row = Rect::new(0, rows - 1, f.area().width, 1);
                    draw_bottom(f, row, w, &mut state);
                })
                .unwrap();
            let buf = terminal.backend().buffer();
            (0..cols).map(|x| buf[(x, rows - 1)].symbol()).collect::<String>()
        };
        let mut plain = known_world();
        let mut heeled = known_world();
        for _ in 0..PassiveKind::SpringHeel.max_stacks() {
            heeled.player.loadout.apply(&Upgrade::Passive(PassiveKind::SpringHeel));
        }
        // A real dash: the countdown starts from the stat the passive changed.
        for w in [&mut plain, &mut heeled] {
            w.player.dash_cd = 0.0;
            w.update(1.0 / 60.0, crate::math::Vec2::new(1.0, 0.0), true);
            assert!(w.player.dash_active > 0.0, "setup: the dash did not start");
            w.player.dash_active = 0.0;
        }
        let (a, b) = (render(&plain), render(&heeled));
        assert!(a.contains("2.0s"), "base cooldown is not 2.0 s: {a:?}");
        assert!(b.contains("1.1s"), "five Spring Heels must read 1.1 s: {b:?}");
        // The same seconds remaining are a smaller share of a shorter cooldown.
        plain.player.dash_cd = 0.9;
        heeled.player.dash_cd = 0.9;
        let full = |t: &str| t.matches('█').count();
        let (a, b) = (render(&plain), render(&heeled));
        assert!(
            full(&b) < full(&a),
            "the bar's scale ignores Spring Heel: {a:?} vs {b:?}"
        );
    }

    /// The dash module must be unmissable at every size the game supports: it
    /// names the live key, it draws a charge bar that fills, and recharging /
    /// READY / refused are different colours and different words.
    #[test]
    fn the_dash_module_reads_ready_charging_and_refused_at_every_size() {
        let mut state = HudState::default();
        for key in [DashKey::Ctrl, DashKey::Space] {
            state.dash_key = key;
            state.dash_name = key.name().to_string();
            for (cols, rows) in [(80u16, 24u16), (120, 40), (57, 20)] {
                let mut w = known_world();
                let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
                let mut draw = |w: &World, state: &mut HudState| {
                    terminal
                        .draw(|f| {
                            let row = Rect::new(0, rows - 1, f.area().width, 1);
                            draw_bottom(f, row, w, state);
                        })
                        .unwrap();
                    let buf = terminal.backend().buffer();
                    let row: String = (0..cols)
                        .map(|x| buf[(x, rows - 1)].symbol())
                        .collect();
                    let (bgs, fgs) = rail_colors(&terminal);
                    (row, bgs, fgs)
                };

                // 1. READY: the key, the word, a full bar and the accent badge.
                w.player.dash_cd = 0.0;
                state.dash_ready_flash = 0.0;
                let (text, bgs, _) = draw(&w, &mut state);
                let accent = palette::marking(w.level.palette.wall_edge, w.level.palette.accent);
                assert!(
                    text.contains(&format!("{} DASH", key.name())),
                    "{cols}x{rows}: the ready module does not name the key: {text:?}"
                );
                assert!(text.contains("READY"), "{cols}x{rows}: no READY: {text:?}");
                assert!(text.contains('█'), "{cols}x{rows}: the ready bar is not full: {text:?}");
                assert!(
                    bgs.contains(&accent),
                    "{cols}x{rows}: no accent badge for READY: {text:?}"
                );

                // 2. Mid-cooldown: a countdown, a partly filled amber bar and no
                //    ready badge anywhere.
                w.player.dash_cd = 1.0;
                let (text, bgs, fgs) = draw(&w, &mut state);
                assert!(text.contains("1.0s"), "{cols}x{rows}: no countdown: {text:?}");
                assert!(!text.contains("READY"), "{cols}x{rows}: still READY: {text:?}");
                assert!(!bgs.contains(&accent), "{cols}x{rows}: accent left on the bar");
                assert!(
                    fgs.contains(&WARN_AMBER),
                    "{cols}x{rows}: the charge bar is not amber: {text:?}"
                );
                assert_ne!(
                    text.matches('█').count() + text.matches('▉').count(),
                    0,
                    "{cols}x{rows}: the charge bar never fills: {text:?}"
                );

                // 3. A refused press: the same module in danger red with WAIT, so
                //    a failed dash is impossible to miss.
                w.player.dash_denied = 0.16;
                let (text, bgs, _) = draw(&w, &mut state);
                assert!(text.contains("WAIT"), "{cols}x{rows}: no refused state: {text:?}");
                assert!(
                    bgs.contains(&DANGER),
                    "{cols}x{rows}: the refused badge is not red: {text:?}"
                );
                w.player.dash_denied = 0.0;
            }
        }
    }

    /// A world whose loadout is the starter weapon plus both active weapons,
    /// with `charge` seconds left on each active's meter (0 = ready).
    fn active_loadout_world(charge: f32) -> World {
        let mut w = known_world();
        w.player.loadout.weapons = vec![Weapon::new(WeaponKind::Gnaw)];
        for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
            let mut weapon = Weapon::new(kind);
            weapon.level = 2;
            weapon.timer = charge;
            w.player.loadout.weapons.push(weapon);
        }
        w
    }

    /// Draw the top rail at `(w, h)` and return its text plus the state.
    fn draw_top_rail(w: &World, width: u16, height: u16) -> (String, HudState) {
        let mut state = HudState::default();
        // Twice: the first tick initialises the frame-to-frame deltas (and so
        // never reaches the charge sync).
        state.tick(1.0 / 60.0, w, &[], false);
        state.tick(1.0 / 60.0, w, &[], false);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, f.area().width, 1);
                draw_top(f, area, w, &mut state);
            })
            .unwrap();
        let mut text = String::new();
        let buf = terminal.backend().buffer();
        for x in 0..width {
            text.push_str(buf[(x, 0)].symbol());
        }
        (text, state)
    }

    /// The active weapons show a compact charge gauge, and a fully charged one
    /// shows a READY state that names its key ("SPACE", or the shorter "SPC" on
    /// a 57-column rail) plus a one-shot flash when it finishes charging.
    #[test]
    fn active_weapons_show_a_charge_bar_and_a_ready_key() {
        // Charging: a partial pip (wide rail), no READY token.
        let charging = active_loadout_world(5.0);
        let (text, state) = draw_top_rail(&charging, 80, 24);
        assert!(
            !text.contains("SPACE"),
            "a charging weapon must not show SPACE: {text:?}"
        );
        assert!(
            !state.active_ready[1] && !state.active_ready[2],
            "actives with 5s left reported ready"
        );
        assert!(
            text.contains('▄') || text.contains('▂') || text.contains('▃'),
            "80x24: no charge gauge for the actives: {text:?}"
        );
        assert!(
            !text.contains('█'),
            "80x24: a half-charged weapon showed a full pip: {text:?}"
        );

        // Charged: SPACE appears once per active slot and the pip is full.
        let ready = active_loadout_world(0.0);
        let (text, state) = draw_top_rail(&ready, 80, 24);
        assert!(
            state.active_ready[1] && state.active_ready[2],
            "charged actives did not report ready"
        );
        assert_eq!(
            text.matches("SPACE").count(),
            2,
            "80x24: expected SPACE once per charged active: {text:?}"
        );
        assert!(
            text.contains('█'),
            "80x24: a charged weapon should show the full pip: {text:?}"
        );

        // 57x20 has no spare column anywhere (a 3-weapon rack is exactly full),
        // so the ready state is the slot's highlight (TRACK background + coat
        // white) instead of a token, and the charging state has no highlight.
        let (charging_text, charging_state) = draw_top_rail(&charging, 57, 20);
        let (ready_text, ready_state) = draw_top_rail(&ready, 57, 20);
        assert!(
            !charging_text.contains("SPC") && !ready_text.contains("SPC"),
            "57x20 must not try to spell the key: {ready_text:?}"
        );
        assert!(!charging_state.active_ready[1] && ready_state.active_ready[1]);
        assert_eq!(
            track_cells(&charging_text, &charging),
            0,
            "57x20: a charging slot must not look ready: {charging_text:?}"
        );
        assert!(
            track_cells(&ready_text, &ready) >= 4,
            "57x20: charged actives must highlight their slot: {ready_text:?}"
        );
    }

    /// How many cells of the top rail are painted on a `TRACK` background?
    ///
    /// That is the narrow-layout "this active weapon is charged" highlight (and
    /// the charge pip on wide layouts), so it is the style-level proof the test
    /// above needs at 57 columns, where no token fits.
    fn track_cells(_text: &str, w: &World) -> usize {
        let mut state = HudState::default();
        state.tick(1.0 / 60.0, w, &[], false);
        state.tick(1.0 / 60.0, w, &[], false);
        let mut terminal = Terminal::new(TestBackend::new(57, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, f.area().width, 1);
                draw_top(f, area, w, &mut state);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..57).filter(|&x| buf[(x, 0)].bg == TRACK).count()
    }

    /// Becoming ready flashes the slot once (the HUD's own "spend me" cue).
    #[test]
    fn a_weapon_that_just_charged_flashes_once() {
        let mut w = active_loadout_world(0.2);
        let mut state = HudState::default();
        state.tick(1.0 / 60.0, &w, &[], false);
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(state.active_flash[1] == 0.0, "nothing has charged yet");

        // Fill the meter: the next tick sees the transition and flashes.
        w.player.loadout.weapons[1].timer = 0.0;
        w.player.loadout.weapons[2].timer = 0.0;
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(state.active_ready[1] && state.active_ready[2]);
        assert!(
            state.active_flash[1] > 0.0 && state.active_flash[2] > 0.0,
            "a weapon that just became ready must flash"
        );

        // …and the flash decays away again.
        for _ in 0..60 {
            state.tick(1.0 / 60.0, &w, &[], false);
        }
        assert_eq!(state.active_flash[1], 0.0, "the ready flash never expired");
    }

    #[test]
    fn full_six_weapon_loadout_fits_at_57_cols_and_shows_max_star() {
        let mut w = known_world();
        w.player.loadout.weapons = WeaponKind::all().into_iter().map(Weapon::new).collect();
        w.player.loadout.weapons[0].level = WeaponKind::Gnaw.max_level();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(57, 1)).unwrap();
        terminal
            .draw(|f| {
                draw_top(f, f.area(), &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("GN★"), "max-level star missing in {text:?}");
        assert!(text.contains("CH1"), "second weapon clipped in {text:?}");
    }

    #[test]
    fn meter_resolves_all_eighths_in_the_last_cell() {
        let mut terminal = Terminal::new(TestBackend::new(4, 1)).unwrap();
        for (n, expected) in " ▏▎▍▌▋▊▉█".chars().enumerate() {
            terminal
                .draw(|f| meter(f, f.area(), "X", n as f32 / 8.0, TAIL))
                .unwrap();
            let cell = &terminal.backend().buffer()[(2, 0)];
            assert_eq!(cell.symbol().chars().next(), Some(expected));
            assert_eq!(cell.bg, TRACK);
        }
    }

    #[test]
    fn damage_trail_renders_gold_between_hp_and_trail() {
        let mut terminal = Terminal::new(TestBackend::new(14, 1)).unwrap();
        terminal
            .draw(|f| {
                meter_with_trail(
                    f,
                    f.area(),
                    "H",
                    Style::default().fg(COAT),
                    0.3,
                    0.8,
                    TAIL,
                    TRAIL_GOLD,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let has_trail = (0..14).any(|x| buf[(x, 0)].fg == TRAIL_GOLD || buf[(x, 0)].bg == TRAIL_GOLD);
        assert!(has_trail, "damage trail should render in TRAIL_GOLD");
    }

    #[test]
    fn hud_tick_emits_escalation_and_pickup_callouts() {
        let mut w = World::new(Theme::Sewer, 7);
        let mut state = HudState::default();
        state.tick(1.0 / 60.0, &w, &[], false);

        // Not yet: the old five-minute schedule announced bats at 20 s, long
        // before the director can spawn one.
        w.elapsed = 21.0;
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            !state.callouts.iter().any(|c| c.active && c.tag == "THREAT +"),
            "the bat callout fired before the bat unlock"
        );
        // Cross the director's Bat unlock + Heal pickup.
        w.elapsed = director::base_unlock_time(crate::game::enemy::EnemyKind::Bat)
            .expect("the bat unlock row")
            + 1.0;
        w.player.hp = 60.0;
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "THREAT +"),
            "crossing the bat unlock should emit escalation callout"
        );

        w.player.hp = 95.0;
        state.tick(1.0 / 60.0, &w, &[Sfx::Pickup], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "+MEDKIT"),
            "heal pickup should emit MEDKIT callout"
        );
    }

    /// A map with a maze: the first Sewer seed that has one.
    fn maze_world() -> World {
        for candidate in 0..64u64 {
            if !Level::generate(Theme::Sewer, candidate).mazes.is_empty() {
                return World::new(Theme::Sewer, candidate);
            }
        }
        panic!("no Sewer map in the first 64 seeds has a maze");
    }

    fn push_maze_event(w: &mut World, event: crate::game::ecs::systems::gates::MazeEvent) {
        w.ecs
            .resource_mut::<crate::game::ecs::systems::gates::GateRuntime>()
            .feed
            .push(event);
    }

    fn clear_maze_feed(w: &mut World) {
        w.ecs
            .resource_mut::<crate::game::ecs::systems::gates::GateRuntime>()
            .feed
            .clear();
    }

    /// The clock counts down to the *boss time*, and the last 30 s of the
    /// build-up is the urgency window (it used to be the last 30 s of a 300 s
    /// survival clock that no longer decides anything).
    #[test]
    fn the_countdown_targets_the_boss_time_and_goes_urgent_at_thirty_seconds() {
        // The intended contract: 255 s of build-up, the last 30 s urgent, then
        // the boss-fight state (a count-up) forever after.
        assert_eq!(director::BOSS_TIME, 900.0, "the boss time moved");
        assert_eq!(BOSS_URGENT_SECS, 30.0);
        let mut w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 1)).unwrap();

        w.elapsed = director::BOSS_TIME - BOSS_URGENT_SECS - 1.0;
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let calm = row_text(&terminal);
        assert!(calm.contains("00:31"), "countdown to 255 s: {calm:?}");
        assert!(!calm.contains("BOSS INBOUND"), "urgent too early: {calm:?}");
        assert!(!has_danger_text(&terminal), "danger styling too early");

        // 28 s out: urgent, but before the precise 25 s warning window.
        w.elapsed = director::BOSS_TIME - 28.0;
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let urgent = row_text(&terminal);
        assert!(
            urgent.contains("BOSS INBOUND"),
            "the last 30 s must read as urgent: {urgent:?}"
        );
        assert!(has_danger_text(&terminal), "the clock is not in danger red");

        // Inside the 25 s warning window the precise countdown takes the pill
        // (and the rail stays urgent).
        w.elapsed = director::BOSS_TIME - 10.0;
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let warning = row_text(&terminal);
        assert!(
            warning.contains("BOSS 10s"),
            "no precise pre-boss countdown: {warning:?}"
        );
        assert!(has_danger_text(&terminal), "the clock left danger red");
    }

    /// At the boss time the rail stops counting down (it would sit at 00:00 for
    /// the whole fight) and shows the fight instead.
    #[test]
    fn the_boss_phase_replaces_the_zero_clock_with_the_fight_state() {
        let mut w = known_world();
        w.elapsed = director::BOSS_TIME + 42.0;
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 1)).unwrap();
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let text = row_text(&terminal);
        assert!(
            text.contains("BOSS FIGHT"),
            "no boss-fight badge after the boss time: {text:?}"
        );
        assert!(
            text.contains("00:42"),
            "the rail should count time into the fight: {text:?}"
        );
        assert!(
            !text.contains("00:00"),
            "a dead countdown is still drawn: {text:?}"
        );

        // The boss itself being alive takes over the label.
        state.telemetry.boss_alive = true;
        terminal
            .draw(|f| draw_top(f, f.area(), &w, &mut state))
            .unwrap();
        let live = row_text(&terminal);
        assert!(live.contains("APEX ACTIVE"), "no apex label: {live:?}");
    }

    /// The threat pips read the same curve the director spawns off, so a
    /// campaign level does not start the player at tier 1 of the Sewer's curve.
    ///
    /// The two boundaries are the director's own gates, not wall-clock numbers:
    /// tier 3 opens at `director::ELITE_START` (90 s since the early-game pass)
    /// and tier 4 at the 120 s brute unlock of the level-0 curve.
    /// The escalation callouts sit on the director's fifteen-minute tables,
    /// not the old five-minute schedule: each stage opens on the second its
    /// kind can first spawn, and the last one is well past the old 2:00.
    #[test]
    fn escalation_stages_follow_the_directors_unlocks() {
        use crate::game::enemy::EnemyKind;
        let at = |k| director::base_unlock_time(k).expect("a base unlock row");
        let rows = [
            (1u8, at(EnemyKind::Bat)),
            (2, at(EnemyKind::Cat)),
            (3, director::ELITE_START),
            (4, at(EnemyKind::Spitter)),
            (5, at(EnemyKind::Brute)),
        ];
        assert_eq!(HudState::stage_for_curve(0.0), 0);
        let mut prev = 0.0;
        for (stage, t) in rows {
            assert!(t > prev, "stage {stage} is out of order at {t}");
            assert!(t < director::RUN_SECONDS, "stage {stage} never fires");
            assert_eq!(HudState::stage_for_curve(t - 0.5), stage - 1);
            assert_eq!(HudState::stage_for_curve(t), stage);
            prev = t;
        }
        // The old schedule had every stage in by 2:00 of a five-minute run.
        assert_eq!(HudState::stage_for_curve(120.0), 2);
        assert_eq!(HudState::stage_for_curve(director::RUN_SECONDS), 5);
    }

    #[test]
    fn the_threat_tier_reads_the_levels_curve_clock() {
        // Sewer: t = 0 is the curve's start.
        assert_eq!(threat_tier(director::curve_clock(0, 0.0), false), 1);
        assert_eq!(threat_tier(director::curve_clock(0, 30.0), false), 2);
        assert_eq!(
            threat_tier(director::curve_clock(0, director::ELITE_START - 1.0), false),
            2
        );
        assert_eq!(
            threat_tier(director::curve_clock(0, director::ELITE_START), false),
            3
        );
        assert_eq!(threat_tier(director::curve_clock(0, 540.0), false), 4);
        // Every stage opens with the same grace (wave-3 re-base): a carried
        // level starts on the calm opening tier, not a mid-run one.
        assert_eq!(threat_tier(director::curve_clock(1, 0.0), false), 1);
        assert_eq!(threat_tier(director::curve_clock(2, 0.0), false), 1);
        // The boss phase is the top tier whatever the curve says.
        assert_eq!(threat_tier(director::curve_clock(0, 0.0), true), 5);
    }

    /// The elite badge reads the director's curve clock, which each campaign
    /// level shifts: the first elite is due when the level's curve reaches
    /// `director::ELITE_START` (90 s), i.e. at 90 s on the Sewer and
    /// immediately on the Kitchen and Lab, whose `LEVEL_CLOCK_OFFSET` (120 s and
    /// 150 s) is already past the gate.
    #[test]
    fn the_elite_badge_follows_the_levels_curve_clock() {
        for (level_index, &offset) in director::LEVEL_CLOCK_OFFSET.iter().enumerate() {
            let interval = director::elite_interval_for(level_index);
            let gate_at = (director::ELITE_START - offset).max(0.0);
            let mut w = World::new_level(
                level_index,
                7,
                crate::game::loadout::Loadout::new(WeaponKind::Gnaw),
            );
            // A whole elite interval away: no badge, even once the gate is open.
            w.director.elite_timer = interval;
            w.elapsed = gate_at + 10.0;
            assert!(
                !elite_badge_due(&w),
                "level {level_index} announced an elite at {} s",
                w.elapsed
            );
            // Before the 5 s gate warning (only the Sewer has room for one).
            if gate_at >= 5.0 {
                w.director.elite_timer = interval;
                w.elapsed = gate_at - 6.0;
                assert!(
                    !elite_badge_due(&w),
                    "level {level_index} announced an elite early at {} s",
                    w.elapsed
                );
            }
            // Inside the 5 s warning window, with the elite about to fire.
            w.elapsed = gate_at;
            w.director.elite_timer = 3.0;
            assert!(
                elite_badge_due(&w),
                "level {level_index} missed its elite at {} s (interval {interval})",
                w.elapsed
            );
            // A badge that is due must reach the rail.
            let mut state = HudState::default();
            let mut terminal = Terminal::new(TestBackend::new(100, 1)).unwrap();
            terminal
                .draw(|f| draw_top(f, f.area(), &w, &mut state))
                .unwrap();
            let text = row_text(&terminal);
            assert!(
                text.contains("ELITE"),
                "level {level_index}: no ELITE badge in {text:?}"
            );
        }
    }

    /// A cache claim and a gate slam each get their own callout, exactly once,
    /// even when the per-tick feed has already moved on.
    #[test]
    fn maze_events_produce_one_callout_each_and_never_repeat() {
        use crate::game::ecs::systems::gates::MazeEvent;

        let mut w = maze_world();
        let mut state = HudState::default();
        state.tick(1.0 / 60.0, &w, &[], false);

        // Tick 1: the claim is in the per-tick feed.
        push_maze_event(&mut w, MazeEvent::CacheClaimed { index: 0 });
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "+CACHE"),
            "the cache claim produced no callout"
        );

        // The app skipped a frame: the feed is gone, the claim is not repeated.
        clear_maze_feed(&mut w);
        for slot in &mut state.callouts {
            slot.active = false;
            slot.timer = 0.0;
        }
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            !state.callouts.iter().any(|c| c.active && c.tag == "+CACHE"),
            "the cache callout fired twice"
        );

        // A slam is its own pill.
        push_maze_event(&mut w, MazeEvent::GateSlammed { index: 0 });
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "GATE SLAM"),
            "the gate slam produced no callout"
        );
    }

    /// The wave-3 pocket-find strip: one `tag+seconds` chip per running timer on
    /// the row above the bottom rail, at both the small (57x20) and the normal
    /// (80x24) terminal size, and nothing at all when no buff is running.
    #[test]
    fn the_buff_strip_lists_running_pocket_finds_at_57x20_and_80x24() {
        fn strip(w: &World, width: u16, height: u16, state: &mut HudState) -> String {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| {
                    let area = f.area();
                    let bottom = Rect::new(0, area.height - 1, area.width, 1);
                    draw_bottom(f, bottom, w, state);
                })
                .unwrap();
            let buf = terminal.backend().buffer();
            let y = height - 2;
            let mut text = String::new();
            for x in 0..width {
                text.push_str(buf[(x, y)].symbol());
            }
            text
        }

        for (width, height) in [(57u16, 20u16), (80u16, 24u16)] {
            let mut w = known_world();
            w.player.buffs = crate::game::player::Buffs {
                shield: 11.2,
                decoy: 4.4,
                slow_bullets: 3.2,
                socks: 5.9,
            };
            let mut state = HudState::default();
            state.tick(1.0 / 60.0, &w, &[], false);
            state.tick(1.0 / 60.0, &w, &[], false);
            let text = strip(&w, width, height, &mut state);
            for chip in ["B12", "M05", "P04", "S06"] {
                assert!(
                    text.contains(chip),
                    "{width}x{height}: no {chip} chip on the buff strip: {text:?}"
                );
            }

            // A run with no pocket find writes no strip at all.
            let mut plain = known_world();
            plain.player.buffs = crate::game::player::Buffs::default();
            let mut state = HudState::default();
            state.tick(1.0 / 60.0, &plain, &[], false);
            state.tick(1.0 / 60.0, &plain, &[], false);
            let text = strip(&plain, width, height, &mut state);
            assert!(
                !text.contains("B12") && !text.contains("S06"),
                "{width}x{height}: an idle run drew a buff chip: {text:?}"
            );
        }
    }

    /// A claim made during a tick the HUD never saw (the reward view is already
    /// `Opened`) is still reported once the HUD looks again.
    #[test]
    fn a_claim_is_latched_from_the_persistent_reward_state() {
        let mut w = maze_world();
        let mut state = HudState::default();
        state.tick(1.0 / 60.0, &w, &[], false);

        // The claim happens with no feed at all (as if the frame were skipped).
        w.level.objects.rewards[0].state = crate::game::art_traps::MazeRewardState::Opened;
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "+CACHE"),
            "a latched claim was dropped"
        );
    }

    /// The rack at every cap a profile can buy (4 + 4 fresh, 8 + 8 fully
    /// upgraded): half-block icons for every owned slot, a dim bay for every
    /// empty one up to the cap, levels, stacks, the charged-active read-out and
    /// — when a 57-column rack cannot hold 16 slots — a `+N` marker instead of a
    /// silent cut. Nothing panics, nothing paints outside the rack.
    #[test]
    fn the_icon_rack_lays_out_every_cap_at_all_three_sizes() {
        use crate::game::loadout::{SlotCaps, Upgrade};

        for caps in [SlotCaps::new(4, 4), SlotCaps::new(6, 6), SlotCaps::FULL] {
            let mut w = known_world();
            w.player.loadout.set_caps(caps);
            w.player.loadout.weapons.clear();
            w.player.loadout.passives.clear();
            let kinds = [
                WeaponKind::Gnaw,
                WeaponKind::CheeseSpray,
                WeaponKind::AcidSpit,
                WeaponKind::TailThrash,
                WeaponKind::SonicSqueak,
                WeaponKind::AcidRain,
                WeaponKind::LaserEyes,
                WeaponKind::TailWhip,
            ];
            for (i, kind) in kinds.iter().take(caps.weapons).enumerate() {
                let mut weapon = Weapon::new(*kind);
                // Two charged actives, three evolved, one maxed.
                if kind.is_active() && i < 4 {
                    weapon.timer = 0.0;
                }
                weapon.level = match i {
                    0 => kind.max_level(),
                    2 => 4,
                    _ => 2,
                };
                weapon.evolved = i == 1 || i == 3 || i == 5;
                w.player.loadout.weapons.push(weapon);
            }
            for kind in PassiveKind::all().into_iter().take(caps.passives) {
                w.player.loadout.apply(&Upgrade::Passive(kind));
                w.player.loadout.apply(&Upgrade::Passive(kind));
            }
            w.player.buffs.shield = 12.0;
            w.player.buffs.socks = 6.0;
            w.gold = 47;
            w.player.dash_cd = 0.9;
            w.player.xp = w.player.xp_to_next / 2;

            let mut state = HudState::default();
            state.tick(1.0 / 60.0, &w, &[], false);
            state.tick(1.0 / 60.0, &w, &[], false);

            for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
                let full = Rect::new(0, 0, tw, th);
                assert_eq!(top_height(full), 3, "{tw}x{th} should use the 3-row icon rack");
                let mut terminal = Terminal::new(TestBackend::new(tw, 3)).unwrap();
                terminal
                    .draw(|f| draw_top(f, f.area(), &w, &mut state))
                    .unwrap();
                let buf = terminal.backend().buffer();
                let rack_text: String = (1..3)
                    .flat_map(|y| (0..tw).map(move |x| buf[(x, y)].symbol()))
                    .collect();
                let icon_cells = (1..3)
                    .flat_map(|y| (0..tw).map(move |x| (x, y)))
                    .filter(|&(x, y)| matches!(buf[(x, y)].symbol(), "▀" | "▄"))
                    .count();
                assert!(
                    icon_cells >= 16,
                    "caps {caps:?} at {tw}x{th}: the rack drew almost no icons: {icon_cells}"
                );
                // One number per slot at the wide sizes; the stack marker is
                // never confusable with a weapon's rank (`×` only for passives).
                if tw >= 80 {
                    // Sixteen slots at 80 columns leave exactly one text cell
                    // each (80 - 16 icons of 4): the planner spends it on the
                    // bare stack digit (`text_w == 1` in `draw_equipment_rack`),
                    // so there the read-out is one digit per slot and no `×`.
                    let one_cell_each = usize::from(tw) == 4 * (caps.weapons + caps.passives) + 16;
                    if one_cell_each {
                        // (`★` is a maxed weapon's rank.)
                        let digits = rack_text
                            .chars()
                            .filter(|c| c.is_ascii_digit() || *c == '★')
                            .count();
                        assert!(
                            digits >= caps.weapons + caps.passives,
                            "caps {caps:?} at {tw}x{th}: a slot lost its number: {rack_text:?}"
                        );
                    } else {
                        assert!(
                            rack_text.contains('×') || rack_text.contains('+'),
                            "caps {caps:?} at {tw}x{th}: no passive read-out and no marker: {rack_text:?}"
                        );
                    }
                }
                // The two racks are separated whenever there is room for it.
                if tw >= 106 {
                    assert!(
                        rack_text.contains('│'),
                        "caps {caps:?} at {tw}x{th}: divider missing: {rack_text:?}"
                    );
                }
                // A filled 8 + 8 rack cannot fit 16 icons at 57 columns: the
                // plate says how many slots are off-screen instead of cutting.
                if caps == SlotCaps::FULL && tw < 64 {
                    assert!(
                        rack_text.contains('+'),
                        "57x20 with 8 + 8: the rack cut slots without a +N marker: {rack_text:?}"
                    );
                }
                // The rail keeps its gold count and the bottom rail its XP %.
                let rail: String = (0..tw).map(|x| buf[(x, 0)].symbol()).collect();
                assert!(rail.contains("47"), "{tw}x{th}: gold missing: {rail:?}");
                // Charged actives name their key wherever a slot can pay for
                // it (SPACE at the widest racks, SPC from 80 columns).
                if tw >= 120 && caps.weapons <= 6 {
                    assert!(
                        rack_text.contains("SPACE") || rack_text.contains("SPC"),
                        "caps {caps:?} at {tw}x{th}: charged active lost its token: {rack_text:?}"
                    );
                }
            }

            // Tiny / degenerate sizes fall back to 1-row rail and never panic.
            for (tw, th) in [(0u16, 0u16), (1, 1), (80, 16)] {
                assert_eq!(top_height(Rect::new(0, 0, tw, th)), 1);
                let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
                terminal
                    .draw(|f| draw_top(f, f.area(), &w, &mut state))
                    .unwrap();
            }
        }
    }

    /// Every weapon and every evolution wears a tag whose **first two
    /// characters are unique**, so no two slots can ever read the same in a
    /// two-letter rack (the review's "both tails can begin `Ta`").
    #[test]
    fn every_weapon_and_evolution_tag_is_distinct() {
        let mut seen: Vec<(&'static str, String)> = Vec::new();
        for kind in WeaponKind::all() {
            for evolved in [false, true] {
                let tag = weapon_tag(kind, evolved);
                assert!(tag.len() >= 2 && tag.len() <= 5, "{tag:?} is not a 2-5 char tag");
                let head: String = tag.chars().take(2).collect();
                if let Some((other, _)) = seen.iter().find(|(_, h)| *h == head) {
                    panic!(
                        "{tag:?} and {other:?} share the two-letter tag {head:?} in a two-letter rack"
                    );
                }
                seen.push((tag, head));
            }
        }
        assert_eq!(
            seen.len(),
            2 * WeaponKind::all().len(),
            "every weapon and every evolution needs its own tag"
        );
    }

    /// The full-rack HUD the review photographed, at the three sizes it named.
    ///
    /// Five weapons (one evolved, one charged active), five passives, running
    /// pocket finds, a dash on cooldown and the boss appointment. Asserts the
    /// elements the review found clipped, colliding or ambiguous: distinct
    /// weapon tags, the passive stack counts, the XP percentage and the
    /// charged-active token. Nothing may run off the right edge.
    #[test]
    fn the_full_rack_layout_at_57_80_and_120_columns() {
        use crate::game::loadout::{SlotCaps, Upgrade};

        let mut w = known_world();
        // The review's five-and-five rack: a profile with 5 + 5 slots fits it
        // exactly, so the layout is the one that was photographed.
        w.player.loadout.set_caps(SlotCaps::new(5, 5));
        w.player.loadout.weapons = vec![
            Weapon::new(WeaponKind::TailWhip),
            Weapon::new(WeaponKind::TailThrash),
            Weapon::new(WeaponKind::AcidRain),
            Weapon::new(WeaponKind::Gnaw),
            Weapon::new(WeaponKind::LaserEyes),
        ];
        // Both Tail weapons owned: the collision the review reported.
        w.player.loadout.weapons[0].level = WeaponKind::TailWhip.max_level();
        w.player.loadout.weapons[0].evolved = true;
        w.player.loadout.weapons[1].level = 4;
        w.player.loadout.weapons[1].timer = 0.0; // charged active
        w.player.loadout.weapons[2].level = 2;
        w.player.loadout.weapons[3].level = 5;
        w.player.loadout.weapons[4].level = 1;
        for p in [
            PassiveKind::MaxHp,
            PassiveKind::Damage,
            PassiveKind::SpringHeel,
            PassiveKind::Counterweight,
            PassiveKind::SecondScent,
        ] {
            w.player.loadout.apply(&Upgrade::Passive(p));
            w.player.loadout.apply(&Upgrade::Passive(p));
        }
        w.player.loadout.stats.armor = 2.0;
        w.player.buffs.shield = 12.0;
        w.player.buffs.socks = 6.0;
        w.gold = 47;
        w.player.xp = w.player.xp_to_next / 2;
        w.player.dash_cd = 0.9;

        let mut state = HudState::default();
        state.tick(1.0 / 60.0, &w, &[], false);
        state.tick(1.0 / 60.0, &w, &[], false);

        for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| {
                    let area = f.area();
                    let top = Rect::new(0, 0, area.width, top_height(area));
                    draw_top(f, top, &w, &mut state);
                    let bottom = Rect::new(0, area.height - 1, area.width, 1);
                    draw_bottom(f, bottom, &w, &mut state);
                })
                .unwrap();
            let buf = terminal.backend().buffer();
            let rows: Vec<String> = (0..th)
                .map(|y| (0..tw).map(|x| buf[(x, y)].symbol()).collect::<String>())
                .collect();
            let rack: String = rows[1..3].concat();
            let rail = &rows[0];
            let bottom = &rows[th as usize - 1];

            // The two Tail weapons never read the same again.
            let tags = [
                weapon_tag(WeaponKind::TailWhip, true),
                weapon_tag(WeaponKind::TailThrash, false),
                weapon_tag(WeaponKind::AcidRain, false),
            ];
            if tw >= 74 {
                for tag in tags {
                    let shown: String = tag.chars().take(2).collect();
                    assert!(
                        rack.contains(&shown),
                        "{tw}x{th}: weapon tag {shown:?} missing from the rack: {rack:?}"
                    );
                }
            }

            // Five passive stack marks and the divider are always there.
            assert!(
                rack.matches('×').count() >= 5,
                "{tw}x{th}: passive stack counts missing: {rack:?}"
            );
            assert!(
                rack.contains('│'),
                "{tw}x{th}: the weapons/passives divider is gone: {rack:?}"
            );

            // The XP percentage answers "how close is the next card" and now
            // fits at every supported play size.
            if tw >= 56 {
                assert!(
                    bottom.contains("XP") && bottom.contains('%'),
                    "{tw}x{th}: XP percentage missing: {bottom:?}"
                );
                assert!(
                    bottom.contains("50%"),
                    "{tw}x{th}: XP percentage is not the real value: {bottom:?}"
                );
            }
            // The armour suffix has room from 80 columns up.
            if tw >= 80 {
                assert!(
                    bottom.contains(" A"),
                    "{tw}x{th}: the armour suffix is missing: {bottom:?}"
                );
            }
            // The dash module keeps its key, its bar and its countdown.
            let has_bar = bottom
                .chars()
                .any(|c| matches!(c, '▏' | '▎' | '▍' | '▌' | '▋' | '▊' | '▉' | '█'));
            assert!(
                bottom.contains("DASH") && has_bar,
                "{tw}x{th}: dash module incomplete: {bottom:?}"
            );

            // The charged active names the key from 80 columns up (`SPACE` at
            // the widest rails, the three-letter `SPC` when a full rack needs
            // the cell).
            if tw >= 80 {
                assert!(
                    rack.contains("SPACE") || rack.contains("SPC"),
                    "{tw}x{th}: charged active lost its key token: {rack:?}"
                );
            }

            // The gold count is always on the rail.
            assert!(rail.contains("47"), "{tw}x{th}: gold count missing: {rail:?}");

            // Nothing overflows the rail: the last cell of row 0 is not a
            // half-written token of a longer word (the status pill is
            // right-aligned and the campaign pips sit past it).
            assert!(
                !rail.ends_with("S"),
                "{tw}x{th}: the top rail clips a status word: {rail:?}"
            );
        }
    }
}

#[cfg(test)]
mod first_steps_tests {
    //! The first-three-minutes hints (`WAVE3 review`): a new player is told how
    //! the active weapons fire and what a chest / a mouse-hole shop is — once
    //! per run, through the existing transient callout slot, with no permanent
    //! manual on the HUD.

    use super::*;
    use crate::game::loadout::Upgrade;
    use crate::game::level::Theme;
    use crate::game::weapon::WeaponKind;

    fn hints(state: &HudState) -> Vec<(CalloutKind, String)> {
        state
            .callouts
            .iter()
            .filter(|c| c.active)
            .filter_map(|c| c.kind.map(|k| (k, c.tag.clone())))
            .collect()
    }

    fn tick_for(state: &mut HudState, w: &World, frames: usize) {
        for _ in 0..frames {
            state.tick(1.0 / 60.0, w, &[], false);
        }
    }

    #[test]
    fn a_charged_active_is_explained_once_per_run() {
        let mut w = World::new(Theme::Sewer, 7);
        w.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
        for weapon in w.player.loadout.weapons.iter_mut() {
            if weapon.kind.is_active() {
                weapon.timer = 0.0; // charged and waiting
            }
        }
        let mut state = HudState::default();
        tick_for(&mut state, &w, 3);
        let seen = hints(&state);
        assert!(
            seen.iter().any(|(k, tag)| *k == CalloutKind::ActivesHint && tag == "SPACE"),
            "the charged active was never explained: {seen:?}"
        );
        // It fires once: let the pill expire, then keep charging and ticking.
        tick_for(&mut state, &w, 400);
        assert!(
            !hints(&state).iter().any(|(k, _)| *k == CalloutKind::ActivesHint),
            "the actives hint fired again"
        );
        // A new run teaches it again.
        state.reset();
        tick_for(&mut state, &w, 3);
        assert!(hints(&state).iter().any(|(k, _)| *k == CalloutKind::ActivesHint));
    }

    #[test]
    fn a_chest_on_the_floor_is_explained_once_per_run() {
        let mut w = World::new(Theme::Sewer, 7);
        let mut state = HudState::default();
        tick_for(&mut state, &w, 3);
        assert!(!hints(&state).iter().any(|(k, _)| *k == CalloutKind::ChestHint));
        w.spawn_chest(w.player.pos + Vec2::new(40.0, 0.0));
        tick_for(&mut state, &w, 2);
        assert!(
            hints(&state).iter().any(|(k, _)| *k == CalloutKind::ChestHint),
            "a chest on the floor was never explained"
        );
        tick_for(&mut state, &w, 400);
        assert!(!hints(&state).iter().any(|(k, _)| *k == CalloutKind::ChestHint));
    }

    #[test]
    fn walking_up_to_a_mouse_hole_is_explained_once_per_run() {
        let mut w = World::new(Theme::Sewer, 7);
        let holes: Vec<Vec2> = w
            .level
            .objects
            .shop_holes
            .iter()
            .map(|hole| hole.face())
            .collect();
        assert!(!holes.is_empty(), "the generated level has no mouse hole");
        let mut state = HudState::default();
        // Far from every hole: no hint.
        w.player.pos = w.level.start;
        tick_for(&mut state, &w, 3);
        if w.nearest_shop_hole().unwrap_or(f32::MAX) > 96.0 {
            assert!(!hints(&state).iter().any(|(k, _)| *k == CalloutKind::ShopHint));
        }
        // Standing at a hole: the hint fires.
        w.player.pos = holes[0];
        tick_for(&mut state, &w, 2);
        assert!(
            hints(&state).iter().any(|(k, _)| *k == CalloutKind::ShopHint),
            "a mouse-hole shop was never explained"
        );
        tick_for(&mut state, &w, 400);
        assert!(!hints(&state).iter().any(|(k, _)| *k == CalloutKind::ShopHint));
    }
}
