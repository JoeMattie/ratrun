//! Three-level campaign UI screens: Level Complete, Keep Three escape manifest,
//! Level 2 & 3 Intros, Campaign Victory, Title Level Select, and compact HUD
//! campaign progress pips.
//!
//! All draw entry points are pure functions of view structs defined in this
//! module (zero dependency on simulation types), perform zero heap allocation
//! per frame, use single-cell-width glyphs only (Ghostty-safe), and return
//! stack-allocated [`HitList`]s mapping clickable screen rectangles to actions.

use core::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::art_icons;
use crate::game::ecs::systems::boss::BossKind;
use crate::game::level::Theme;
use crate::render::palette::{mix, to_color, Rgb, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};

// Semantic colors matching `src/render/hud.rs` and `src/render/menu.rs`.
const VICTORY_MINT: Color = Color::Rgb(169, 210, 186);
const HEAL_MINT: Color = Color::Rgb(126, 224, 152);
const TRAIL_GOLD: Color = Color::Rgb(248, 214, 152);
const WARN_AMBER: Color = Color::Rgb(242, 184, 102);
const DANGER: Color = Color::Rgb(248, 92, 102);
const PLATE_SUB: Color = Color::Rgb(24, 31, 35);
const PLATE_HILITE: Color = Color::Rgb(30, 40, 44);
const PLATE_PACKED: Color = Color::Rgb(22, 36, 33);
const PLATE_DIM: Color = Color::Rgb(15, 19, 22);

// ---------------------------------------------------------------------------
// Stack-allocated zero-heap-allocation formatting & hit-testing primitives
// ---------------------------------------------------------------------------

/// Fixed-capacity UTF-8 stack buffer for zero-allocation per-frame formatting.
struct StackBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackBuf<N> {
    #[inline]
    const fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    #[inline]
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for StackBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.chars() {
            let mut tmp = [0u8; 4];
            let enc = ch.encode_utf8(&mut tmp).as_bytes();
            if self.len + enc.len() > N {
                break;
            }
            self.buf[self.len..self.len + enc.len()].copy_from_slice(enc);
            self.len += enc.len();
        }
        Ok(())
    }
}

/// The key names a campaign plate prints in its buttons and legends.
///
/// The plates used to hard-code `Enter`, `Space` and `Tab`, so a rebind made
/// every legend wrong. The `draw_*_with` entry points take one of these; the
/// plain `draw_*` ones pass [`PlateKeys::DEFAULT`] (the stock bindings), so a
/// caller that has a `Controls` hands over `action_label(..)` and one that
/// does not keeps the old text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlateKeys<'a> {
    /// The confirm key (`Action::Confirm`).
    pub confirm: &'a str,
    /// The key that packs / unpacks a row on the keep plate.
    pub toggle: &'a str,
    /// The key that switches the next sector's route on the keep plate.
    pub route: &'a str,
}

impl PlateKeys<'static> {
    /// The stock bindings, i.e. the literals the plates printed before.
    pub const DEFAULT: PlateKeys<'static> = PlateKeys {
        confirm: "Enter",
        toggle: "Space",
        route: "Tab",
    };
}

impl Default for PlateKeys<'static> {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The cleared-sector verdict and narrative line for a boss that is **not**
/// its floor's original fight (`None` for the six originals, whose words are
/// [`CampaignLevel::boss_verdict`] / [`CampaignLevel::boss_triumph_line`]).
///
/// Four floors draw one of two bosses by seed; without this the plate told a
/// run that killed the Horseshoe that the Apex Jaguar was down.
pub const fn alternate_boss_plate(kind: BossKind) -> Option<(&'static str, &'static str)> {
    match kind {
        BossKind::HorseshoeCrab => Some((
            "THE HORSESHOE  —  TURNED OVER",
            "Flipped on its back in the outflow, legs working at nothing. It was here before the pipes. The pipes are yours now.",
        )),
        BossKind::MegaRat => Some((
            "SUBJECT R-01  —  RETIRED",
            "The first success lies still on the cold room floor. The collar tag reads R-01. The pantry door behind it was never locked.",
        )),
        BossKind::Cobra => Some((
            "KING COBRA  —  UNCOILED",
            "Twenty-two feet of it, slack across the tiles. The door it broke on the way out of its room is still open.",
        )),
        BossKind::Hawk => Some((
            "THE REDTAIL  —  SHAKEN OFF",
            "The shadow lifts off the deck and falls behind with the road noise. Nothing on the flatbed is bigger than you now.",
        )),
        BossKind::Cat
        | BossKind::Broom
        | BossKind::Mower
        | BossKind::Mangle
        | BossKind::NightEdition
        | BossKind::Magpie => None,
    }
}

/// Stack-allocated list of interactive rectangles returned by campaign screens.
///
/// Later entries take precedence on hit-testing (`at`), matching `menu::HitMap`.
#[derive(Clone, Copy, Debug)]
pub struct HitList<A: Copy, const N: usize> {
    entries: [Option<(Rect, A)>; N],
    len: usize,
}

impl<A: Copy, const N: usize> Default for HitList<A, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Copy, const N: usize> HitList<A, N> {
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            len: 0,
        }
    }

    pub fn push(&mut self, rect: Rect, action: A) {
        if rect.is_empty() || self.len >= N {
            return;
        }
        self.entries[self.len] = Some((rect, action));
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return the topmost action covering cell `(x, y)`, if any.
    pub fn at(&self, x: u16, y: u16) -> Option<A> {
        self.entries[..self.len]
            .iter()
            .rev()
            .flatten()
            .find(|(r, _)| r.contains((x, y).into()))
            .map(|(_, a)| *a)
    }

    /// Iterate all registered `(Rect, Action)` pairs in registration order.
    pub fn iter(&self) -> impl Iterator<Item = (Rect, A)> + '_ {
        self.entries[..self.len].iter().flatten().copied()
    }

    /// Find the first registered rectangle matching `action`.
    pub fn find(&self, action: A) -> Option<Rect>
    where
        A: PartialEq,
    {
        self.iter().find(|(_, a)| *a == action).map(|(r, _)| r)
    }
}

// ---------------------------------------------------------------------------
// Public shared campaign view types (zero simulation dependencies)
// ---------------------------------------------------------------------------

/// The four levels of the Rat Run campaign, in order (the original route; the
/// side floors have plates of their own). The ride shares its plate with the
/// finale — it is the campaign's fourth level and still a practice floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CampaignLevel {
    Sewer = 0,
    Kitchen = 1,
    Lab = 2,
    /// Stage two's side route (`WAVE3_DESIGN.md` §11): the laundry floor.
    Laundry = 3,
    /// Stage three's side route: the printworks.
    Printworks = 4,
    /// The getaway ride's plate: the flatbed trailer. The campaign's **finale**
    /// (batch twelve) with its own bosses (the Magpie and the Redtail), and a
    /// floor the player can practise alone.
    Trailer = 5,
}

impl CampaignLevel {
    /// The campaign's levels on the **original route**, in order: Sewer,
    /// Kitchen, Lab, the Flatbed ride. The side-route floors are reachable
    /// through [`Self::from_theme`] but are not extra steps, so the progress
    /// pips stay four long.
    pub const fn all() -> [CampaignLevel; 4] {
        [
            CampaignLevel::Sewer,
            CampaignLevel::Kitchen,
            CampaignLevel::Lab,
            CampaignLevel::Trailer,
        ]
    }

    /// The plate for a floor the player is on, including the side routes and
    /// the getaway ride.
    pub const fn from_theme(theme: Theme) -> CampaignLevel {
        match theme {
            Theme::Sewer => CampaignLevel::Sewer,
            Theme::Kitchen => CampaignLevel::Kitchen,
            Theme::Lab => CampaignLevel::Lab,
            Theme::Laundry => CampaignLevel::Laundry,
            Theme::Printworks => CampaignLevel::Printworks,
            Theme::Trailer => CampaignLevel::Trailer,
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    /// The plate for a **campaign level index** (`0 => Sewer`, `1 => Kitchen`,
    /// `2 => Lab`, `3+ => the Flatbed ride` — the original route).
    ///
    /// A level is not a floor: the side routes have plates of their own, so a
    /// list of floors must carry themes or floor indices, not call this.
    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => CampaignLevel::Sewer,
            1 => CampaignLevel::Kitchen,
            2 => CampaignLevel::Lab,
            _ => CampaignLevel::Trailer,
        }
    }

    /// The campaign **level index** this floor is played at (`0..=3`): the
    /// fourth is the ride, which is both the finale and a practice floor.
    pub const fn stage(self) -> usize {
        match self {
            CampaignLevel::Sewer => 0,
            CampaignLevel::Kitchen | CampaignLevel::Laundry => 1,
            CampaignLevel::Lab | CampaignLevel::Printworks => 2,
            CampaignLevel::Trailer => 3,
        }
    }

    /// The 1-based level number a heading or pip shows (`1..=4`).
    pub const fn stage_number(self) -> u8 {
        match self {
            CampaignLevel::Sewer => 1,
            CampaignLevel::Kitchen | CampaignLevel::Laundry => 2,
            CampaignLevel::Lab | CampaignLevel::Printworks => 3,
            CampaignLevel::Trailer => 4,
        }
    }

    /// 1-based campaign level number (`1..=4`; the ride is the fourth).
    pub const fn number(self) -> u8 {
        match self {
            CampaignLevel::Sewer => 1,
            CampaignLevel::Kitchen => 2,
            CampaignLevel::Lab => 3,
            CampaignLevel::Laundry => 2,
            CampaignLevel::Printworks => 3,
            CampaignLevel::Trailer => 4,
        }
    }

    /// The original route's next stage (the route's actual next floor is the
    /// campaign's choice — see `CampaignState::theme`). `None` once the run is
    /// on the finale: the ride ends every run, whichever stage-three floor was
    /// played before it.
    pub const fn next(self) -> Option<CampaignLevel> {
        match self {
            CampaignLevel::Sewer => Some(CampaignLevel::Kitchen),
            CampaignLevel::Kitchen | CampaignLevel::Laundry => Some(CampaignLevel::Lab),
            CampaignLevel::Lab | CampaignLevel::Printworks => Some(CampaignLevel::Trailer),
            CampaignLevel::Trailer => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "THE SEWERS",
            CampaignLevel::Kitchen => "THE KITCHEN",
            CampaignLevel::Lab => "THE LAB",
            CampaignLevel::Laundry => "THE LAUNDRY",
            CampaignLevel::Printworks => "THE PRINTWORKS",
            CampaignLevel::Trailer => "THE FLATBED",
        }
    }

    pub const fn short_name(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "Sewer",
            CampaignLevel::Kitchen => "Kitchen",
            CampaignLevel::Lab => "Lab",
            CampaignLevel::Laundry => "Laundry",
            CampaignLevel::Printworks => "Printworks",
            CampaignLevel::Trailer => "Flatbed",
        }
    }

    pub const fn sector_code(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "SECTOR 01",
            CampaignLevel::Kitchen => "SECTOR 02",
            CampaignLevel::Lab => "SECTOR 03",
            CampaignLevel::Laundry => "SECTOR 02B",
            CampaignLevel::Printworks => "SECTOR 03B",
            // The finale: the rig is the way out, and it is still sector four.
            CampaignLevel::Trailer => "SECTOR 04",
        }
    }

    pub const fn boss_name(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "APEX JAGUAR",
            CampaignLevel::Kitchen => "THE SWEEPER",
            CampaignLevel::Lab => "TURF-REAPER MK.IV",
            // The floors' own bosses (`WAVE3_DESIGN.md` §12,
            // `BossKind::for_theme`): the Laundry fights the Mangle, the
            // Printworks the Night Edition and the getaway ride the Magpie
            // (`render::hud::boss_codename` names the same fights).
            CampaignLevel::Laundry => "THE MANGLE",
            CampaignLevel::Printworks => "THE NIGHT EDITION",
            CampaignLevel::Trailer => "THE MAGPIE",
        }
    }

    /// The floor's boss fight(s): one name, or **both** when a seeded floor
    /// draws one of two (`BossKind::pool_for_theme`: the Sewer fields the
    /// Jaguar *or* the Horseshoe, the Kitchen the Sweeper or Subject R-01, the
    /// Lab the mower or the King Cobra, and the Flatbed the Magpie or the
    /// Redtail). The first name is always [`Self::boss_name`], i.e. the floor's
    /// original boss and the HUD boss bar's pinned plate.
    pub const fn boss_pair(self) -> (&'static str, Option<&'static str>) {
        match self {
            CampaignLevel::Sewer => (Self::Sewer.boss_name(), Some("THE HORSESHOE")),
            CampaignLevel::Kitchen => (Self::Kitchen.boss_name(), Some("SUBJECT R-01")),
            CampaignLevel::Lab => (Self::Lab.boss_name(), Some("KING COBRA")),
            CampaignLevel::Trailer => (Self::Trailer.boss_name(), Some("THE REDTAIL")),
            // One boss each: the Mangle and the Night Edition are pinned.
            CampaignLevel::Laundry | CampaignLevel::Printworks => (self.boss_name(), None),
        }
    }

    pub const fn boss_verdict(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "APEX JAGUAR  —  DOWN",
            CampaignLevel::Kitchen => "THE SWEEPER  —  ROUTED",
            CampaignLevel::Lab => "TURF-REAPER MK.IV  —  SCRAPPED",
            CampaignLevel::Laundry => "THE MANGLE  —  JAMMED",
            CampaignLevel::Printworks => "THE NIGHT EDITION  —  STOPPED",
            CampaignLevel::Trailer => "THE MAGPIE  —  SHAKEN OFF",
        }
    }

    pub const fn boss_triumph_line(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => {
                "The big cat overshot its last pounce and skidded into the drain. Quiet returns to the pipes — climb toward the heat of the kitchen."
            }
            CampaignLevel::Kitchen => {
                "One ankle bite too many. The broom clatters across the linoleum and two huge slippers hop straight out the pantry door."
            }
            CampaignLevel::Lab => {
                "Belly-up on the tiles, rotary blades spinning down to silence. The emergency STOP flashes dark. The blast door ahead stands unlatched."
            }
            CampaignLevel::Laundry => {
                "For the first time in thirty years, the laundry stops. The drum ticks as it cools, and the service stair beyond it is unlatched."
            }
            CampaignLevel::Printworks => {
                "The last edition rolls off the belt, curls under the street door, and you are out with it."
            }
            CampaignLevel::Trailer => {
                "The magpie's last shriek is carried off down the deck with the road noise. The cab's brake lines pop and the rig starts to drift. The facility is behind you."
            }
        }
    }

    pub const fn default_intro_flavor(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => {
                "Cold pipes and colder things. The drainage threads under the whole facility. Follow the current up and out."
            }
            CampaignLevel::Kitchen => {
                "Grease, snap-traps, and the stink of bait. Nobody agrees who keeps this kitchen: the broom, or the thing in the cold room that was first."
            }
            CampaignLevel::Lab => {
                "Where you were made. The lights hum a number that used to be your name. End it here. Walk out the front door."
            }
            CampaignLevel::Laundry => {
                "Indigo tile, steam between the machines, and a hundred doors that will not open. The whites come out pink down here."
            }
            CampaignLevel::Printworks => {
                "Ink in the seams of your paws and the presses still running overnight. Tomorrow's front page says you were caught."
            }
            CampaignLevel::Trailer => {
                // The same note as `lore::map_intro`: the earlier 260-cell
                // paragraph was cut off after three rows on every plate.
                "Forty feet of wet deck doing eighty into the dark. The cab is forward. So is the magpie, and it thinks the rig is its nest."
            }
        }
    }

    /// Facility material tint matching `menu.rs::material`.
    pub const fn material_rgb(self) -> Rgb {
        match self {
            CampaignLevel::Sewer => (144, 184, 173),
            CampaignLevel::Kitchen => (213, 173, 122),
            CampaignLevel::Lab => (151, 175, 214),
            CampaignLevel::Laundry => (159, 200, 200),
            CampaignLevel::Printworks => (182, 180, 165),
            CampaignLevel::Trailer => (206, 178, 132),
        }
    }

    pub fn accent(self) -> Color {
        to_color(self.material_rgb())
    }
}

/// Whether an upgrade entry is a leveled weapon or a stacked passive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeCategory {
    Weapon { level: u8, max_level: u8 },
    Passive { stacks: u8 },
}

/// One owned upgrade presented on the Keep Three manifest or Level Intro pack strip.
#[derive(Clone, Copy, Debug)]
pub struct UpgradeEntry<'a> {
    /// Display name, e.g. `"Gnaw"`, `"Cheese Spray"`, `"Thick Hide"`.
    pub name: &'a str,
    /// Single-cell geometric glyph (e.g. `'▸'`, `'»'`, `'✦'`, `'◎'`, `'◈'`, `'♦'`, `'+'`).
    pub glyph: char,
    /// Accent RGB tint for the icon and level pips.
    pub color: Rgb,
    /// Weapon level/max or passive stack count.
    pub category: UpgradeCategory,
    /// Short mechanical summary shown on cards / detail strip.
    pub detail: &'a str,
}

// ---------------------------------------------------------------------------
// Screen 1: Level Complete
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelCompleteAction {
    /// Proceed to Keep Three (after Sewer/Kitchen) or Campaign Victory (after Lab).
    Continue,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelCompleteView<'a> {
    pub level: CampaignLevel,
    /// The floor actually played, when it is not a campaign plate: the Flatbed
    /// borrows the Lab's level (it is a practice floor, not a campaign step),
    /// and its boss is the Magpie — so the triumph stamp needs the theme to
    /// stop reading "TURF-REAPER MK.IV — SCRAPPED" on the getaway ride.
    pub theme: Option<Theme>,
    /// Time spent on this level in seconds.
    pub time_secs: f32,
    /// Kills scored on this level.
    pub kills: u32,
    /// Cumulative campaign score so far (including this level).
    pub score_so_far: u32,
    pub gold: u32,
    /// Score earned on this level alone.
    pub level_score: u32,
    /// Player level reached by the end of this stage.
    pub player_level: u32,
    /// Total upgrades currently owned (shown as a teaser for Keep Three).
    pub owned_upgrades: usize,
    /// The floor the run goes to next, from the campaign's chosen route
    /// (`campaign::Route`) — **not** [`CampaignLevel::next`], which only knows
    /// the original route. `None` when this clear ends the run (a practice
    /// floor, the getaway ride, a final-stage floor or no campaign in flight).
    pub next: Option<CampaignLevel>,
    /// Optional override for the per-boss triumph text.
    pub custom_triumph: Option<&'a str>,
    /// Currently hovered mouse action.
    pub hovered: Option<LevelCompleteAction>,
}

// ---------------------------------------------------------------------------
// Screen 2: Keep Three (Escape Pack Manifest)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepThreeAction {
    /// Toggle whether `items[index]` is packed for the next level.
    ToggleItem(usize),
    /// Scroll the manifest viewport up.
    ScrollUp,
    /// Scroll the manifest viewport down.
    ScrollDown,
    /// Confirm the chosen pack and descend/ascend into the next level.
    /// Emitted in [`HitList`] only when [`keep_three_can_confirm`] is `true`.
    Confirm,
    /// Pick the route floor at this index (`0..2`) for the level being packed
    /// for (`WAVE3_DESIGN.md` §1: Kitchen or Laundry, then Lab or Printworks).
    Route(u8),
}

/// One route chip on the keep screen: the floor's name and whether it is the
/// chosen one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteChip<'a> {
    pub name: &'a str,
    pub chosen: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct KeepThreeView<'a> {
    /// Sector just cleared (unchosen upgrades stay behind here).
    pub from_level: CampaignLevel,
    /// Next sector the player is packing for.
    pub next_level: CampaignLevel,
    pub gold: u32,
    /// All upgrades owned at the end of `from_level` (up to ~13 entries or more).
    pub items: &'a [UpgradeEntry<'a>],
    /// Parallel selection flags (`selected.get(i).copied().unwrap_or(false)`).
    pub selected: &'a [bool],
    /// Target number of upgrades to carry (normally `3`).
    pub max_keep: usize,
    /// Currently focused keyboard cursor index in `items`.
    pub cursor: usize,
    /// Top visible item index (clamped by the renderer to valid scroll range).
    pub scroll: usize,
    /// Currently hovered mouse target.
    pub hovered: Option<KeepThreeAction>,
    /// The two floors the next level may be played on (original route first),
    /// or `[None, None]` when the next level has no choice (stage one, or the
    /// campaign is over).
    pub route: [Option<RouteChip<'a>>; 2],
    /// The **gene splices** in the rat, by name (`shop::SpliceKind::name`).
    ///
    /// Splices occupy neither rack (`WAVE3_DESIGN` §6) so they are not
    /// selectable rows, but the manifest still accounts for them: they ride
    /// along whatever the pack choice was, and the screen says so.
    pub splices: &'a [&'static str],
}

// ---------------------------------------------------------------------------
// Screen 3: Level Intro (Levels 2 & 3; consistent with menu::draw_map_intro)
// ---------------------------------------------------------------------------

/// The most carried upgrades the intro plate lists (the keep limit's cap).
const MAX_CARRIED_SHOWN: usize = crate::game::campaign::MAX_KEEP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelIntroAction {
    Begin,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelIntroView<'a> {
    /// Level being entered (`Kitchen` or `Lab`; also handles `Sewer`).
    pub level: CampaignLevel,
    /// Optional flavor line override (`None` uses the built-in sector flavor).
    pub flavor: Option<&'a str>,
    /// Upgrades carried in from the previous level (0..=5: three by default,
    /// two more from the nest's carry track).
    pub carried: &'a [UpgradeEntry<'a>],
    /// Cumulative campaign score entering this level.
    pub score_so_far: u32,
    pub gold: u32,
    /// Currently hovered mouse action.
    pub hovered: Option<LevelIntroAction>,
}

// ---------------------------------------------------------------------------
// Screen 4: Campaign Victory
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampaignVictoryAction {
    PlayAgain,
    Menu,
}

/// Per-level metrics displayed in the Campaign Victory breakdown table.
#[derive(Clone, Copy, Debug)]
pub struct LevelBreakdown {
    pub level: CampaignLevel,
    pub time_secs: f32,
    pub kills: u32,
    pub score: u32,
    pub player_level: u32,
    /// The boss that actually died on this floor (a floor's pool holds more
    /// than one). `None` falls back to the floor's original boss.
    pub boss: Option<BossKind>,
}

#[derive(Clone, Copy, Debug)]
pub struct CampaignVictoryView<'a> {
    pub final_score: u32,
    pub total_time_secs: f32,
    pub total_kills: u32,
    pub gold: u32,
    pub new_best: bool,
    pub levels: [LevelBreakdown; crate::game::campaign::LEVEL_COUNT],
    /// Final upgrades carried out the front door (rendered when space permits).
    pub final_loadout: &'a [UpgradeEntry<'a>],
    /// Optional custom finale copy.
    pub custom_blurb: Option<&'a str>,
    /// Currently keyboard-focused action button.
    pub focus: CampaignVictoryAction,
    /// Currently pointer-hovered action button.
    pub hovered: Option<CampaignVictoryAction>,
    /// The bound to-menu key's name (`Action::Quit`), printed on its button.
    pub menu_key: &'a str,
}

// ---------------------------------------------------------------------------
// Screen 5: Title Level Select
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelSelectAction {
    /// Select the sheet at this **floor** index (an index into
    /// [`Theme::playable`]). Emitted for every visible slot, locked ones
    /// included, so a locked floor can be selected and read; only
    /// [`Self::Start`] refuses to start it.
    SelectLevel(usize),
    /// Begin run at the currently selected unlocked floor.
    Start,
    /// Return to title menu.
    Back,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LevelBest {
    pub score: u32,
    pub best_time_secs: f32,
    pub cleared: bool,
}

/// Room for the floors a host can hand [`LevelSelectView`]: the six playable
/// floors today plus two spare, so a seventh floor needs no change in the art.
pub const MAX_LEVEL_SLOTS: usize = 8;

/// One floor's card on the sector select screen.
#[derive(Clone, Copy, Debug)]
pub struct LevelSlotView {
    /// The floor's index into the title's sector row (`Theme::playable()`):
    /// the slot's identity, and what starting it starts.
    pub floor: usize,
    /// The floor's plate: name, accent, boss and emblem.
    pub level: CampaignLevel,
    /// The campaign stage the floor belongs to (`0..campaign::LEVEL_COUNT`):
    /// the ride reads its own fourth level rather than the Lab's stage. Drives
    /// the `S1`/`S2`/`S3`/`S4` chip and groups the grid.
    pub stage: usize,
    /// Is the floor selectable (its level reachable)?
    pub unlocked: bool,
    /// Best cleared run on this floor, if any. Floors on the same stage have
    /// separate records.
    pub best: Option<LevelBest>,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelSelectView<'a> {
    /// Every playable floor, in route order (`Theme::playable()`). A slice so
    /// a seventh floor needs no change here.
    pub slots: &'a [LevelSlotView],
    /// The selected floor's index into `slots` (equivalently, into
    /// `Theme::playable()`); the screen also matches each slot's `floor`, so
    /// a differently ordered list still highlights the right one.
    pub selected: usize,
    /// Currently hovered mouse action.
    pub hovered: Option<LevelSelectAction>,
    /// The bound back key's name (`Action::Back`), printed on its button.
    pub back_key: &'a str,
}

// ---------------------------------------------------------------------------
// Component 6: Compact Campaign Progress Pips (HUD / Pause)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct CampaignProgressView {
    /// Active campaign level (`1..=4`).
    pub current: CampaignLevel,
    /// Number of levels already cleared (`0..=4`).
    pub cleared_count: u8,
    /// Optional background fill color (defaults to `PLATE`).
    pub bg: Option<Color>,
}

// ---------------------------------------------------------------------------
// Pure helper functions (geometry & state logic shared with callers)
// ---------------------------------------------------------------------------

/// Required number of selections before Keep Three confirm becomes enabled.
pub fn keep_three_required(view: &KeepThreeView<'_>) -> usize {
    view.max_keep.min(view.items.len())
}

/// Count how many valid indices in `view.items` are currently marked selected.
pub fn keep_three_selected_count(view: &KeepThreeView<'_>) -> usize {
    view.items
        .iter()
        .enumerate()
        .filter(|(i, _)| view.selected.get(*i).copied().unwrap_or(false))
        .count()
}

/// Whether the Keep Three confirm button is enabled (`selected == min(max_keep, items.len())`).
pub fn keep_three_can_confirm(view: &KeepThreeView<'_>) -> bool {
    keep_three_selected_count(view) == keep_three_required(view)
}

#[derive(Clone, Copy)]
struct KeepThreeLayout {
    panel_rect: Rect,
    two_pane: bool,
    row_height: u16,
    visible_items: usize,
}

fn keep_three_layout(area: Rect) -> KeepThreeLayout {
    let two_pane = area.width >= 104 && area.height >= 26;
    let (pw, ph) = if two_pane {
        (area.width.min(108), area.height.min(36))
    } else if area.width >= 76 && area.height >= 23 {
        (area.width.min(78), area.height.min(30))
    } else {
        (area.width.min(74), area.height.min(24))
    };
    let panel_rect = centered_rect(pw, ph, area);
    let inner_h = panel_rect.height.saturating_sub(2);
    let row_height: u16 = if two_pane || (inner_h >= 21 && panel_rect.width >= 68) {
        2
    } else {
        1
    };
    let chrome_rows: u16 = if two_pane {
        // header (2) + rule (1) + footer hint (1)
        4
    } else if row_height == 2 {
        // top pack strip (3) + rule (1) + footer rule + confirm + hint (3)
        7
    } else {
        // compact: top pack counter (1) + inline slots (1) + rule (1) + footer rule + detail + confirm (3)
        6
    };
    let list_h = inner_h.saturating_sub(chrome_rows);
    let visible_items = usize::from(list_h / row_height.max(1)).max(1);
    KeepThreeLayout {
        panel_rect,
        two_pane,
        row_height,
        visible_items,
    }
}

/// Number of upgrade items simultaneously visible on Keep Three for `area`.
///
/// Callers can use this to keep `KeepThreeView::scroll` synchronized when
/// navigating with keyboard up/down arrows.
pub fn keep_three_visible_rows(area: Rect) -> usize {
    if area.width < 12 || area.height < 8 {
        return 1;
    }
    keep_three_layout(area).visible_items
}

/// Recommended column width for [`draw_campaign_pips`].
pub const fn campaign_pips_width(compact: bool) -> u16 {
    if compact {
        9
    } else {
        24
    }
}

// ---------------------------------------------------------------------------
// Zero-allocation buffer drawing primitives
// ---------------------------------------------------------------------------

#[inline]
fn ink(c: Color) -> Style {
    Style::default().fg(c)
}

#[inline]
fn heavy(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

#[inline]
fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

#[inline]
fn clamp_rect(r: Rect, bounds: Rect) -> Rect {
    r.intersection(bounds)
}

fn fill_rect(buf: &mut Buffer, rect: Rect, ch: char, style: Style) {
    let r = rect.intersection(buf.area);
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            buf[(x, y)].set_char(ch).set_style(style);
        }
    }
}

fn hline(buf: &mut Buffer, x: u16, y: u16, width: u16, ch: char, style: Style) {
    if y < buf.area.y || y >= buf.area.bottom() || width == 0 {
        return;
    }
    let start = x.max(buf.area.x);
    let end = x.saturating_add(width).min(buf.area.right());
    for cx in start..end {
        buf[(cx, y)].set_char(ch).set_style(style);
    }
}

fn vline(buf: &mut Buffer, x: u16, y: u16, height: u16, ch: char, style: Style) {
    if x < buf.area.x || x >= buf.area.right() || height == 0 {
        return;
    }
    let start = y.max(buf.area.y);
    let end = y.saturating_add(height).min(buf.area.bottom());
    for cy in start..end {
        buf[(x, cy)].set_char(ch).set_style(style);
    }
}

/// Write a single-line string clipped to `max_w` cells and `buf.area`.
/// Returns the next `x` column after the last written character.
fn put_str(buf: &mut Buffer, x: u16, y: u16, s: &str, max_w: u16, style: Style) -> u16 {
    if y < buf.area.y || y >= buf.area.bottom() || max_w == 0 || x >= buf.area.right() {
        return x;
    }
    let limit = x.saturating_add(max_w).min(buf.area.right());
    let mut cx = x;
    for ch in s.chars() {
        if cx >= limit {
            break;
        }
        if cx >= buf.area.x {
            buf[(cx, y)].set_char(ch).set_style(style);
        }
        cx = cx.saturating_add(1);
    }
    cx
}

/// Write a string clipped to `max_w` cells, ending in `..` when it had to be
/// cut (two ASCII dots: `…` is ambiguous-width in some terminals).
fn put_str_ellipsis(buf: &mut Buffer, x: u16, y: u16, s: &str, max_w: u16, style: Style) -> u16 {
    if max_w == 0 {
        return x;
    }
    let count = s.chars().count();
    if count <= usize::from(max_w) {
        return put_str(buf, x, y, s, max_w, style);
    }
    if max_w <= 2 {
        return put_str(buf, x, y, s, max_w, style);
    }
    let keep = max_w - 2;
    let cx = put_str(buf, x, y, s, keep, style);
    put_str(buf, cx, y, "..", 2, style)
}

fn put_centered(buf: &mut Buffer, area: Rect, row: u16, s: &str, style: Style) -> Rect {
    if row >= area.height || area.width == 0 {
        return Rect::default();
    }
    let y = area.y + row;
    let len = (s.chars().count() as u16).min(area.width);
    let x = area.x + (area.width - len) / 2;
    put_str(buf, x, y, s, len, style);
    clamp_rect(Rect::new(x, y, len, 1), buf.area)
}

/// Zero-allocation word-wrapped paragraph renderer. Returns number of lines drawn.
fn put_wrapped(
    buf: &mut Buffer,
    area: Rect,
    text: &str,
    style: Style,
    centered: bool,
) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    let max_w = usize::from(area.width);
    let mut rest = text.trim();
    let mut row: u16 = 0;

    while !rest.is_empty() && row < area.height {
        let total_chars = rest.chars().count();
        let (line, next) = if total_chars <= max_w {
            (rest, "")
        } else {
            let edge = rest
                .char_indices()
                .nth(max_w)
                .map_or(rest.len(), |(idx, _)| idx);
            let cut = rest[..edge]
                .rfind(' ')
                .filter(|&idx| idx > 0)
                .unwrap_or(edge);
            (&rest[..cut], rest[cut..].trim_start())
        };
        let line = line.trim_end();
        let lw = (line.chars().count() as u16).min(area.width);
        let x = if centered {
            area.x + (area.width - lw) / 2
        } else {
            area.x
        };
        put_str(buf, x, area.y + row, line, lw, style);
        rest = next;
        row += 1;
    }
    row
}

/// Stamped metal plate matching `menu::panel`: dark `PLATE` fill, `TRACK` right/bottom
/// edges, lit `edge` accent along top and left edges, bold stamped caption.
fn panel(
    buf: &mut Buffer,
    rect: Rect,
    caption: &str,
    right_caption: Option<&str>,
    edge: Color,
) -> Rect {
    let r = rect.intersection(buf.area);
    if r.is_empty() {
        return Rect::default();
    }
    fill_rect(buf, r, ' ', ink(COAT).bg(PLATE));
    if r.width < 2 || r.height < 2 {
        return r;
    }

    let lit = ink(edge).bg(PLATE);
    let dim = ink(TRACK).bg(PLATE);
    let x0 = r.x;
    let x1 = r.right() - 1;
    let y0 = r.y;
    let y1 = r.bottom() - 1;

    buf[(x0, y0)].set_char('┌').set_style(lit);
    buf[(x1, y0)].set_char('┐').set_style(lit);
    buf[(x0, y1)].set_char('└').set_style(lit);
    buf[(x1, y1)].set_char('┘').set_style(dim);

    hline(buf, x0 + 1, y0, r.width - 2, '─', lit);
    hline(buf, x0 + 1, y1, r.width - 2, '─', dim);
    vline(buf, x0, y0 + 1, r.height - 2, '│', lit);
    vline(buf, x1, y0 + 1, r.height - 2, '│', dim);

    if r.width > 5 && !caption.is_empty() {
        put_str(buf, x0 + 2, y0, caption, r.width - 4, heavy(edge).bg(PLATE));
    }
    if let Some(rc) = right_caption {
        let rw = rc.chars().count() as u16;
        let cap_end = x0 + 2 + caption.chars().count() as u16 + 2;
        if rw > 0 && x1 > cap_end + rw + 1 {
            put_str(buf, x1 - rw - 1, y0, rc, rw, ink(MUTED).bg(PLATE));
        }
    }

    Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
}

/// Bordered sub-card inside a panel.
fn sub_box(buf: &mut Buffer, rect: Rect, border_col: Color, bg: Color) -> Rect {
    let r = rect.intersection(buf.area);
    if r.is_empty() {
        return Rect::default();
    }
    fill_rect(buf, r, ' ', ink(COAT).bg(bg));
    if r.width < 2 || r.height < 2 {
        return r;
    }
    let bs = ink(border_col).bg(bg);
    let x0 = r.x;
    let x1 = r.right() - 1;
    let y0 = r.y;
    let y1 = r.bottom() - 1;
    buf[(x0, y0)].set_char('┌').set_style(bs);
    buf[(x1, y0)].set_char('┐').set_style(bs);
    buf[(x0, y1)].set_char('└').set_style(bs);
    buf[(x1, y1)].set_char('┘').set_style(bs);
    hline(buf, x0 + 1, y0, r.width - 2, '─', bs);
    hline(buf, x0 + 1, y1, r.width - 2, '─', bs);
    vline(buf, x0, y0 + 1, r.height - 2, '│', bs);
    vline(buf, x1, y0 + 1, r.height - 2, '│', bs);
    Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonState {
    Normal,
    Primary,
    Focused,
    Hovered,
    Disabled,
}

fn button_style(state: ButtonState, accent: Color) -> Style {
    match state {
        ButtonState::Normal => ink(COAT).bg(TRACK),
        ButtonState::Primary => heavy(PLATE).bg(accent),
        ButtonState::Focused => heavy(PLATE).bg(accent),
        ButtonState::Hovered => heavy(PLATE).bg(TAIL),
        ButtonState::Disabled => ink(MUTED).bg(PLATE_SUB),
    }
}

fn draw_button_at(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    label: &str,
    max_w: u16,
    state: ButtonState,
    accent: Color,
) -> Rect {
    if max_w == 0 || y < buf.area.y || y >= buf.area.bottom() || x >= buf.area.right() {
        return Rect::default();
    }
    let w = (label.chars().count() as u16).min(max_w);
    let style = button_style(state, accent);
    put_str(buf, x, y, label, w, style);
    clamp_rect(Rect::new(x, y, w, 1), buf.area)
}

fn fmt_time(secs: f32) -> StackBuf<12> {
    let mut b = StackBuf::<12>::new();
    let total = secs.max(0.0) as u32;
    let _ = write!(b, "{:02}:{:02}", total / 60, total % 60);
    b
}

fn fmt_pips(level: u8, max_level: u8) -> StackBuf<24> {
    let mut b = StackBuf::<24>::new();
    let max_c = max_level.clamp(1, 6);
    for i in 1..=max_c {
        let _ = b.write_char(if i <= level { '●' } else { '○' });
    }
    b
}

/// Draw inline pips into a panel's top-right border (only if it won't collide with `caption`).
fn stamp_header_pips(
    buf: &mut Buffer,
    panel_rect: Rect,
    caption: &str,
    current: CampaignLevel,
    cleared_count: u8,
) {
    if panel_rect.width < 46 || panel_rect.height < 2 {
        return;
    }
    let y = panel_rect.y;
    let pips_w: u16 = 16;
    let x = panel_rect.right().saturating_sub(pips_w + 1);
    let cap_end = panel_rect.x + 2 + caption.chars().count() as u16 + 2;
    if x <= cap_end {
        return;
    }
    let base = ink(MUTED).bg(PLATE);
    let mut cx = put_str(buf, x, y, " LV ", 4, base);
    let mut nb = StackBuf::<8>::new();
    let _ = write!(
        nb,
        "{}/{} ",
        current.stage_number(),
        crate::game::campaign::LEVEL_COUNT
    );
    cx = put_str(buf, cx, y, nb.as_str(), 4, heavy(COAT).bg(PLATE));
    // The pips are **stages**, so the current one is matched by stage: a side
    // route (Laundry, Printworks) is never equal to the original route's plate
    // and used to show no current pip at all.
    let current_stage = usize::from(current.stage_number().saturating_sub(1));
    let pips = CampaignLevel::all().len();
    for idx in 0..pips {
        let (ch, col) = if (idx as u8) < cleared_count {
            ('●', HEAL_MINT)
        } else if idx == current_stage {
            ('◆', current.accent())
        } else {
            ('○', EDGE)
        };
        let mut g = [0u8; 4];
        cx = put_str(buf, cx, y, ch.encode_utf8(&mut g), 1, heavy(col).bg(PLATE));
        if idx + 1 < pips {
            cx = put_str(buf, cx, y, " ", 1, base);
        }
    }
    let _ = put_str(buf, cx, y, " ", 1, base);
}

// ---------------------------------------------------------------------------
// 1. Level Complete Screen
// ---------------------------------------------------------------------------

fn draw_boss_triumph_emblem(buf: &mut Buffer, area: Rect, level: CampaignLevel, y: u16) {
    match level {
        CampaignLevel::Sewer => {
            // Apex Jaguar: pointed ears + extinguished × eyes + claw slash over drain pipe.
            let rows = [
                ("   ▄▀▄  · · ·  ▄▀▄   ", TRAIL_GOLD),
                ("  ▐█  × ──┬── ×  █▌  ", WARN_AMBER),
                ("══╧═══╲═══╧═══╱═══╧══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Kitchen => {
            // The Sweeper: snapped broom handle + splayed bristles + overturned slipper.
            let rows = [
                ("    ╲   ╭───╮   ▄▀▀▀▄   ", WARN_AMBER),
                ("     ╲  │▓▓▓│  ▐█████▌  ", TRAIL_GOLD),
                ("══════╲═╧═┴═╧═══▀▀▀▀▀═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Laundry => {
            // The Mangle stopped dead: two striped rollers in an iron frame,
            // the loose crank hanging out on its side.
            let rows = [
                ("  ╭─┬─┬─╮    ╭─╮  ", WARN_AMBER),
                ("  │▓│▓│▓│    │◉│  ", TRAIL_GOLD),
                ("══╧═╧═╧═╧════╧═╧══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Printworks => {
            // Placeholder: the Printworks' fight is the borrowed mower.
            let rows = [
                ("  ▄█▄  ▀▀▀▀▀▀▀▀▀▀  ▄█▄  ", WARN_AMBER),
                ("  ▐█▌  [  STOP  ]  ▐█▌  ", DANGER),
                ("═══╧═══╲════════╱═══╧═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Lab => {
            // TURF-REAPER MK.IV: flipped mower shell belly-up + dead STOP button + treads.
            let rows = [
                ("  ▄█▄  ▀▀▀▀▀▀▀▀▀▀  ▄█▄  ", WARN_AMBER),
                ("  ▐█▌  [  STOP  ]  ▐█▌  ", DANGER),
                ("═══╧═══╲════════╱═══╧═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Trailer => {
            // THE MAGPIE: a corvid silhouette banking away over the deck rail,
            // the rig's marker light still swinging.
            let rows = [
                ("   ▄▀▄   ╲   ╱   ▄▀▄   ", WARN_AMBER),
                ("  ▀▀▀▀▀   ╲ ╱   ▀▀▀▀▀  ", TRAIL_GOLD),
                ("═══════════╧═══════════", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
    }
}

/// Draw the Level Complete triumph screen after a sector boss is defeated.
///
/// Returns a [`HitList`] containing [`LevelCompleteAction::Continue`] mapped to
/// the rendered primary continue button.
pub fn draw_level_complete(
    f: &mut Frame,
    area: Rect,
    view: &LevelCompleteView<'_>,
) -> HitList<LevelCompleteAction, 4> {
    draw_level_complete_with(f, area, view, &PlateKeys::DEFAULT, None)
}

/// [`draw_level_complete`] with the key names it prints and the boss the run
/// actually killed.
///
/// `boss` matters on the four floors that draw one of two bosses: with
/// `Some(kind)` the verdict pill and the narrative line name that boss
/// ([`alternate_boss_plate`]); with `None` they name the floor's original.
pub fn draw_level_complete_with(
    f: &mut Frame,
    area: Rect,
    view: &LevelCompleteView<'_>,
    keys: &PlateKeys<'_>,
    boss: Option<BossKind>,
) -> HitList<LevelCompleteAction, 4> {
    let mut hits = HitList::new();
    if area.width < 12 || area.height < 6 {
        return hits;
    }

    let accent = view.level.accent();
    let compact = area.width < 66 || area.height < 22;
    let pw = if compact {
        area.width.min(64)
    } else {
        area.width.min(72)
    };
    let ph = if area.height < 20 {
        area.height
    } else if area.height < 24 {
        area.height.min(20)
    } else {
        area.height.min(24)
    };
    let card = centered_rect(pw, ph, area);

    let mut cap = StackBuf::<48>::new();
    let _ = write!(
        cap,
        " SECTOR CLEARED   /   {} ",
        view.level.name()
    );
    let buf = f.buffer_mut();
    let inner = panel(buf, card, cap.as_str(), None, VICTORY_MINT);
    stamp_header_pips(buf, card, cap.as_str(), view.level, view.level.number());

    if inner.width < 10 || inner.height < 4 {
        return hits;
    }

    let mut y: u16 = if inner.height >= 19 { 1 } else { 0 };

    // Triumph status pill (boss-specific verdict). The getaway ride has its
    // own: its boss is the Magpie, not the Lab's mower.
    let alt = match boss {
        Some(kind) => alternate_boss_plate(kind),
        None => None,
    };
    let verdict = match (alt, view.theme) {
        (Some((verdict, _)), _) => verdict,
        (None, Some(Theme::Trailer)) => "THE MAGPIE  —  BLOWN OFF",
        (None, _) => view.level.boss_verdict(),
    };
    let mut badge = StackBuf::<64>::new();
    if inner.width >= 42 {
        let _ = write!(badge, " ▰▰  {verdict}  ▰▰ ");
    } else {
        let _ = write!(badge, " ▸ {verdict} ");
    }
    put_centered(
        buf,
        inner,
        y,
        badge.as_str(),
        heavy(PLATE).bg(VICTORY_MINT),
    );
    y += if inner.height >= 18 { 2 } else { 1 };

    // Boss-specific geometric triumph emblem (when vertical space allows).
    // The emblem is the floor's original boss, so the pool mate goes without.
    if alt.is_none() && inner.height >= 16 && inner.width >= 30 {
        draw_boss_triumph_emblem(buf, inner, view.level, y);
        y += 4;
    }

    // Narrative beat specific to the downed boss.
    let triumph = view
        .custom_triumph
        .or(alt.map(|(_, line)| line))
        .unwrap_or_else(|| view.level.boss_triumph_line());
    let text_inset = if inner.width >= 40 { 3 } else { 1 };
    let text_rect = Rect::new(
        inner.x + text_inset,
        inner.y + y,
        inner.width.saturating_sub(text_inset * 2),
        inner.height.saturating_sub(y + 6).clamp(1, 3),
    );
    let drawn_lines = put_wrapped(buf, text_rect, triumph, ink(COAT).bg(PLATE), true);
    y += drawn_lines + u16::from(inner.height >= 16);

    // Horizontal divider.
    if y + 4 < inner.height {
        hline(
            buf,
            inner.x + 2,
            inner.y + y,
            inner.width.saturating_sub(4),
            '─',
            ink(TRACK).bg(PLATE),
        );
        y += 1;
    }

    // Level stats strip: Time, Kills, Score so far (plus Level Score / Rat Lv on wider layouts).
    let tbuf = fmt_time(view.time_secs);
    if inner.width >= 54 && y + 3 < inner.height {
        let cols = 4u16;
        let col_w = inner.width.saturating_sub(4) / cols;
        let x0 = inner.x + 2;

        let mut kills_b = StackBuf::<16>::new();
        let _ = write!(kills_b, "{}", view.kills);
        let mut score_b = StackBuf::<24>::new();
        let _ = write!(score_b, "{}", view.score_so_far);

        let mut gold_b = StackBuf::<16>::new();
        let _ = write!(gold_b, "{}", view.gold);

        let stat_items: [(&str, &str, Color); 4] = [
            ("TIME", tbuf.as_str(), COAT),
            ("KILLS", kills_b.as_str(), COAT),
            ("RUN SCORE", score_b.as_str(), TRAIL_GOLD),
            ("GOLD", gold_b.as_str(), TRAIL_GOLD),
        ];
        for (i, (lbl, val, vcol)) in stat_items.iter().enumerate() {
            let cx = x0 + i as u16 * col_w;
            let cell = Rect::new(cx, inner.y + y, col_w, 2);
            put_centered(buf, cell, 0, lbl, ink(MUTED).bg(PLATE));
            put_centered(buf, cell, 1, val, heavy(*vcol).bg(PLATE));
            if i + 1 < stat_items.len() {
                vline(buf, cx + col_w.saturating_sub(1), inner.y + y, 2, '│', ink(TRACK).bg(PLATE));
            }
        }
        y += 2;
        if y + 3 < inner.height {
            let mut sub = StackBuf::<64>::new();
            if inner.width >= 60 {
                let _ = write!(
                    sub,
                    "Sector score +{}   ·   Subject Lv {}   ·   {} upgrades owned",
                    view.level_score, view.player_level, view.owned_upgrades
                );
            } else {
                let _ = write!(
                    sub,
                    "Sector +{}  ·  Lv {}  ·  {} upgrades",
                    view.level_score, view.player_level, view.owned_upgrades
                );
            }
            put_centered(buf, inner, y, sub.as_str(), ink(MUTED).bg(PLATE));
            y += 1;
        }
    } else if y + 2 < inner.height {
        let mut line = StackBuf::<64>::new();
        let _ = write!(
            line,
            "{} · {} kills · Score {} · Gold {}",
            tbuf.as_str(),
            view.kills,
            view.score_so_far,
            view.gold
        );
        put_centered(buf, inner, y, line.as_str(), heavy(COAT).bg(PLATE));
        y += 1;
    }

    // Next-step teaser right above CTA.
    let btn_row = inner.height.saturating_sub(1);
    if y < btn_row.saturating_sub(1) {
        let mut teaser = StackBuf::<72>::new();
        match view.next {
            Some(next) => {
                // No pack size here: the limit is three to five (the nest's
                // carry track), this view does not carry it, and the old
                // "Pack 3 of N" was wrong for a run that keeps four or five.
                // The keep-three plate that follows states the real number.
                if inner.width >= 54 {
                    let _ = write!(
                        teaser,
                        "Next: {} — {} · pack from {} upgrades",
                        next.sector_code(),
                        next.name(),
                        view.owned_upgrades
                    );
                } else {
                    let _ = write!(
                        teaser,
                        "Next: {} · pack from {}",
                        next.short_name(),
                        view.owned_upgrades
                    );
                }
            }
            None if view.level == CampaignLevel::Trailer => {
                let _ = write!(teaser, "The getaway is yours — the facility is behind you");
            }
            None if view.level.stage() < 2 => {
                let _ = write!(teaser, "Practice clear — no pack to carry");
            }
            None => {
                let _ = write!(teaser, "All three containment sectors breached");
            }
        }
        put_centered(
            buf,
            inner,
            btn_row.saturating_sub(2),
            teaser.as_str(),
            ink(accent).bg(PLATE),
        );
    }

    // Primary CTA button.
    let mut cta = StackBuf::<72>::new();
    let wide = inner.width >= 38;
    let _ = match (view.next.is_some(), wide) {
        (true, true) => write!(cta, " ▸  {} / click   PACK UPGRADES ", keys.confirm),
        (true, false) => write!(cta, " ▸ {}  PACK ", keys.confirm),
        (false, true) => write!(cta, " ▸  {} / click   ESCAPE FACILITY ", keys.confirm),
        (false, false) => write!(cta, " ▸ {}  ESCAPE ", keys.confirm),
    };
    let cta_label = cta.as_str();
    let btn_state = if view.hovered == Some(LevelCompleteAction::Continue) {
        ButtonState::Hovered
    } else {
        ButtonState::Primary
    };
    let bw = (cta_label.chars().count() as u16).min(inner.width);
    let bx = inner.x + (inner.width - bw) / 2;
    let btn_rect = draw_button_at(
        buf,
        bx,
        inner.y + btn_row,
        cta_label,
        bw,
        btn_state,
        VICTORY_MINT,
    );
    hits.push(btn_rect, LevelCompleteAction::Continue);

    hits
}

// ---------------------------------------------------------------------------
// 2. Keep Three Screen (Escape Pack Manifest)
// ---------------------------------------------------------------------------

fn draw_pack_slot_pill(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_w: u16,
    slot_num: usize,
    item: Option<&UpgradeEntry<'_>>,
) -> u16 {
    if max_w < 6 {
        return x;
    }
    match item {
        Some(it) => {
            let mut sb = StackBuf::<24>::new();
            let _ = write!(sb, "[{} ", it.glyph);
            // Take first word or up to 6 chars cleanly (avoids awkward "Sharp T×3").
            let short = it.name.split(' ').next().unwrap_or(it.name);
            for ch in short.chars().take(6) {
                let _ = sb.write_char(ch);
            }
            match it.category {
                UpgradeCategory::Weapon { level, max_level } => {
                    if level >= max_level {
                        let _ = sb.write_char('★');
                    } else {
                        let _ = write!(sb, " {}", level);
                    }
                }
                UpgradeCategory::Passive { stacks } => {
                    let _ = write!(sb, "×{}", stacks);
                }
            }
            let _ = sb.write_char(']');
            put_str(
                buf,
                x,
                y,
                sb.as_str(),
                max_w,
                heavy(to_color(it.color)).bg(PLATE_PACKED),
            )
        }
        None => {
            let mut sb = StackBuf::<16>::new();
            let _ = write!(sb, "[{}: --- ]", slot_num + 1);
            put_str(buf, x, y, sb.as_str(), max_w, ink(EDGE).bg(PLATE_DIM))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_manifest_row(
    buf: &mut Buffer,
    row_rect: Rect,
    item: &UpgradeEntry<'_>,
    is_selected: bool,
    is_focused: bool,
    is_hovered: bool,
    pack_full: bool,
    two_line: bool,
) {
    if row_rect.is_empty() {
        return;
    }
    let bg = if is_hovered {
        PLATE_HILITE
    } else if is_selected {
        PLATE_PACKED
    } else if is_focused {
        PLATE_SUB
    } else if pack_full {
        PLATE_DIM
    } else {
        PLATE
    };
    fill_rect(buf, row_rect, ' ', ink(COAT).bg(bg));

    let item_col = to_color(item.color);
    let dim_unselected = pack_full && !is_selected && !is_focused && !is_hovered;

    // Left gutter indicator bar (cursor / selection state).
    let bar_ch = if is_focused || is_hovered {
        '▸'
    } else if is_selected {
        '▌'
    } else {
        ' '
    };
    let bar_col = if is_hovered {
        TAIL
    } else if is_selected {
        HEAL_MINT
    } else if is_focused {
        TRAIL_GOLD
    } else {
        TRACK
    };
    let mut cx = row_rect.x;
    let y0 = row_rect.y;
    let mut tmp = [0u8; 4];
    cx = put_str(buf, cx, y0, bar_ch.encode_utf8(&mut tmp), 1, heavy(bar_col).bg(bg));
    cx = put_str(buf, cx, y0, " ", 1, ink(COAT).bg(bg));

    // Check diamond.
    let chk = if is_selected { "◆ " } else { "◇ " };
    let chk_col = if is_selected {
        HEAL_MINT
    } else if is_focused || is_hovered {
        COAT
    } else {
        EDGE
    };
    cx = put_str(buf, cx, y0, chk, 2, heavy(chk_col).bg(bg));

    // Upgrade icon (4×4 pixel sprite on 2-line rows, single glyph on 1-line compact rows).
    let show_pixel_icon = two_line && row_rect.height >= 2 && row_rect.width >= 38;
    if show_pixel_icon {
        let icon_rect = Rect::new(cx, y0, art_icons::SMALL_CELL_W, art_icons::SMALL_CELL_H);
        art_icons::draw(buf, icon_rect, art_icons::by_name(item.name));
        cx = cx.saturating_add(art_icons::SMALL_CELL_W + 1);
    } else {
        let g_col = if dim_unselected {
            to_color(mix(item.color, (70, 78, 82), 0.6))
        } else {
            item_col
        };
        cx = put_str(buf, cx, y0, item.glyph.encode_utf8(&mut tmp), 1, heavy(g_col).bg(bg));
        cx = put_str(buf, cx, y0, " ", 1, ink(COAT).bg(bg));
    }

    // Right-side status badge (`[KEEP]` / `[LEAVE]`) and level pips/stacks.
    let tag = if is_selected {
        " KEEP "
    } else if pack_full {
        " LEAVE "
    } else {
        "       "
    };
    let tag_w = tag.chars().count() as u16;
    let tag_style = if is_selected {
        heavy(PLATE).bg(HEAL_MINT)
    } else if pack_full {
        ink(EDGE).bg(PLATE_DIM)
    } else {
        ink(EDGE).bg(bg)
    };

    let mut tier = StackBuf::<24>::new();
    let tier_col: Color = match item.category {
        UpgradeCategory::Weapon { level, max_level } => {
            if row_rect.width >= 44 {
                let pips = fmt_pips(level, max_level);
                let _ = write!(tier, "Lv{} {}", level, pips.as_str());
            } else {
                let _ = write!(tier, "Lv{}", level);
            }
            if dim_unselected {
                EDGE
            } else if level >= max_level {
                TRAIL_GOLD
            } else {
                item_col
            }
        }
        UpgradeCategory::Passive { stacks } => {
            let _ = write!(tier, "×{} PASSIVE", stacks);
            if dim_unselected { EDGE } else { HEAL_MINT }
        }
    };
    let tier_str = if row_rect.width < 40 {
        // Shorten passive label on narrow screens.
        if let UpgradeCategory::Passive { stacks } = item.category {
            tier = StackBuf::new();
            let _ = write!(tier, "×{}", stacks);
        }
        tier.as_str()
    } else {
        tier.as_str()
    };
    let tier_w = tier_str.chars().count() as u16;

    let right_reserved = tag_w + tier_w + 2;
    let name_w = row_rect.right().saturating_sub(cx).saturating_sub(right_reserved);
    let name_style = if is_selected || is_focused || is_hovered {
        heavy(COAT).bg(bg)
    } else if dim_unselected {
        ink(MUTED).bg(bg)
    } else {
        ink(COAT).bg(bg)
    };
    put_str_ellipsis(buf, cx, y0, item.name, name_w, name_style);

    // Draw tier right-aligned before badge.
    let tag_x = row_rect.right().saturating_sub(tag_w);
    let tier_x = tag_x.saturating_sub(tier_w + 1);
    if tier_x > cx + 2 {
        put_str(buf, tier_x, y0, tier_str, tier_w, ink(tier_col).bg(bg));
    }
    if tag_x > cx && (is_selected || pack_full) {
        put_str(buf, tag_x, y0, tag, tag_w, tag_style);
    }

    // Second line: mechanical detail + left accent continuation.
    if two_line && row_rect.height >= 2 {
        let y1 = y0 + 1;
        let cont_ch = if is_selected { "▌" } else { " " };
        put_str(buf, row_rect.x, y1, cont_ch, 1, ink(bar_col).bg(bg));
        let detail_x = if show_pixel_icon { cx } else { row_rect.x + 6 };
        let detail_w = row_rect.right().saturating_sub(detail_x + 2);
        let detail_style = if dim_unselected {
            ink(EDGE).bg(bg)
        } else {
            ink(MUTED).bg(bg)
        };
        put_str_ellipsis(buf, detail_x, y1, item.detail, detail_w, detail_style);
    }
}

/// Draw the Keep Three escape-packing screen.
///
/// Players choose `min(view.max_keep, view.items.len())` upgrades to carry into
/// `view.next_level`; everything else is left behind in `view.from_level`.
///
/// Returned [`HitList`] maps:
/// - each visible upgrade row → [`KeepThreeAction::ToggleItem(idx)`]
/// - scroll up/down arrows (when `items.len() > visible`) → `ScrollUp` / `ScrollDown`
/// - confirm button → [`KeepThreeAction::Confirm`] (only emitted when enabled!)
pub fn draw_keep_three(
    f: &mut Frame,
    area: Rect,
    view: &KeepThreeView<'_>,
) -> HitList<KeepThreeAction, 32> {
    draw_keep_three_with(f, area, view, &PlateKeys::DEFAULT)
}

/// [`draw_keep_three`] with the key names its buttons and legends print.
pub fn draw_keep_three_with(
    f: &mut Frame,
    area: Rect,
    view: &KeepThreeView<'_>,
    keys: &PlateKeys<'_>,
) -> HitList<KeepThreeAction, 32> {
    let mut hits = HitList::new();
    if area.width < 16 || area.height < 8 {
        return hits;
    }

    let layout = keep_three_layout(area);
    let required = keep_three_required(view);
    let selected_count = keep_three_selected_count(view);
    let can_confirm = selected_count == required;
    let pack_full = required > 0 && selected_count >= required;
    let next_accent = view.next_level.accent();

    // Collect indices of currently selected items for the slot previews, in
    // manifest order (so a slot never changes place when another is packed
    // below it). The pack holds three to five (`campaign::MAX_KEEP`): the
    // previews used to stop at three and hid the fourth and fifth.
    const SLOTS: usize = crate::game::campaign::MAX_KEEP;
    let slot_count = required.clamp(1, SLOTS);
    let mut packed_indices: [Option<usize>; SLOTS] = [None; SLOTS];
    let mut p_len = 0usize;
    for (i, _) in view.items.iter().enumerate() {
        if view.selected.get(i).copied().unwrap_or(false) && p_len < SLOTS {
            packed_indices[p_len] = Some(i);
            p_len += 1;
        }
    }

    let mut cap = StackBuf::<64>::new();
    if layout.panel_rect.width >= 64 {
        let _ = write!(
            cap,
            " PACK FOR ESCAPE   /   TO {} ",
            view.next_level.name()
        );
    } else if layout.panel_rect.width >= 50 {
        let _ = write!(
            cap,
            " PACK FOR ESCAPE / {} ",
            view.next_level.short_name().to_uppercase()
        );
    } else {
        let _ = write!(cap, " PACK {} OF {} ", required, view.items.len());
    }

    // The purse rides the caption when there is room for it *and* the route
    // chips that follow the caption on the same border; on a narrow panel
    // (57x20) the chips win the header and the purse moves to the bottom
    // border, so neither is ever clipped.
    let mut gold_text = StackBuf::<24>::new();
    let _ = write!(gold_text, " · GOLD {} ", view.gold);
    let chips_w: u16 = view
        .route
        .iter()
        .flatten()
        .map(|chip| chip.name.chars().count() as u16 + 3)
        .sum();
    let gold_w = gold_text.as_str().chars().count() as u16;
    let header_need = 2 + cap.as_str().chars().count() as u16 + gold_w + 3 + chips_w + 3;
    let gold_in_caption =
        chips_w == 0 || layout.panel_rect.width < 40 || header_need <= layout.panel_rect.width;
    if gold_in_caption {
        let _ = write!(cap, "{}", gold_text.as_str());
    }

    let buf = f.buffer_mut();
    let inner = panel(
        buf,
        layout.panel_rect,
        cap.as_str(),
        None,
        if can_confirm { HEAL_MINT } else { TAIL },
    );
    // The stage pips share the top border with the route chips. The chips are
    // a control and the pips are decoration, so when both cannot fit the pips
    // go (the chips used to be painted over half of them).
    let chips_end = layout.panel_rect.x + 2 + cap.as_str().chars().count() as u16 + 3 + chips_w;
    if chips_w == 0 || chips_end + 15 < layout.panel_rect.right() {
        stamp_header_pips(
            buf,
            layout.panel_rect,
            cap.as_str(),
            view.next_level,
            view.from_level.number(),
        );
    }

    if inner.width < 12 || inner.height < 6 {
        return hits;
    }

    // Clamp scroll window.
    let visible = layout.visible_items;
    let max_scroll = view.items.len().saturating_sub(visible);
    let scroll_top = view.scroll.min(max_scroll);
    let needs_scroll = view.items.len() > visible;

    if layout.two_pane {
        // ---- WIDE TWO-PANE LAYOUT (120x40, 200x60) ----
        let right_w: u16 = 38;
        let left_w = inner.width.saturating_sub(right_w + 3);
        let left_x = inner.x + 1;
        let div_x = left_x + left_w + 1;
        let right_x = div_x + 2;

        // Vertical divider between manifest and Escape Pack dossier.
        vline(
            buf,
            div_x,
            inner.y,
            inner.height.saturating_sub(1),
            '│',
            ink(TRACK).bg(PLATE),
        );

        // Left header.
        let mut lhdr = StackBuf::<48>::new();
        let _ = write!(
            lhdr,
            "UPGRADE MANIFEST ({} OWNED)",
            view.items.len()
        );
        put_str(buf, left_x, inner.y, lhdr.as_str(), left_w, heavy(COAT).bg(PLATE));
        // The subtitle also accounts for the gene splices: they occupy neither
        // rack, so they are not selectable rows, but they ride along whatever
        // the pack choice is (`WAVE3_DESIGN` §6).
        let mut sub = StackBuf::<96>::new();
        let _ = write!(sub, "Only what fits in your pack comes through the vent.");
        if !view.splices.is_empty() {
            let _ = write!(sub, "  ·  Splices ride along:");
            for (i, name) in view.splices.iter().enumerate() {
                let _ = write!(sub, "{}{}", if i == 0 { " " } else { ", " }, name);
            }
        }
        put_str_ellipsis(buf, left_x, inner.y + 1, sub.as_str(), left_w, ink(MUTED).bg(PLATE));
        hline(buf, left_x, inner.y + 2, left_w, '─', ink(TRACK).bg(PLATE));

        // Scrollbar reserve on right side of left pane when overflowing.
        let list_w = if needs_scroll {
            left_w.saturating_sub(2)
        } else {
            left_w
        };
        let list_y = inner.y + 3;
        let list_h = inner.height.saturating_sub(4);

        if view.items.is_empty() {
            put_centered(
                buf,
                Rect::new(left_x, list_y, list_w, list_h),
                1,
                "No upgrades to pack — claws and nerve only.",
                ink(MUTED).bg(PLATE),
            );
        } else {
            for slot in 0..visible {
                let idx = scroll_top + slot;
                if idx >= view.items.len() {
                    break;
                }
                let ry = list_y + slot as u16 * layout.row_height;
                if ry + layout.row_height > list_y + list_h {
                    break;
                }
                let rrect = Rect::new(left_x, ry, list_w, layout.row_height);
                let is_sel = view.selected.get(idx).copied().unwrap_or(false);
                let is_foc = idx == view.cursor;
                let is_hov = view.hovered == Some(KeepThreeAction::ToggleItem(idx));
                draw_manifest_row(
                    buf,
                    rrect,
                    &view.items[idx],
                    is_sel,
                    is_foc,
                    is_hov,
                    pack_full,
                    layout.row_height >= 2,
                );
                hits.push(clamp_rect(rrect, area), KeepThreeAction::ToggleItem(idx));
            }
        }

        // Scroll track + clickable up/down buttons.
        if needs_scroll && list_h >= 3 {
            let sx = left_x + list_w + 1;
            draw_scrollbar(
                buf,
                sx,
                list_y,
                list_h,
                scroll_top,
                max_scroll,
                view.hovered,
                &mut hits,
            );
        }

        // Right pane: ESCAPE PACK dossier.
        let rw = right_w.saturating_sub(1);
        let mut rhdr = StackBuf::<32>::new();
        let _ = write!(rhdr, "ESCAPE PACK   {}/{}", selected_count, required);
        put_str(
            buf,
            right_x,
            inner.y,
            rhdr.as_str(),
            rw,
            heavy(if can_confirm { HEAL_MINT } else { TAIL }).bg(PLATE),
        );

        // Progress pips right-aligned on right header row.
        let mut pips_x = right_x + rw.saturating_sub(slot_count as u16 * 2 + 1);
        for i in 0..required.min(SLOTS) {
            let filled = i < selected_count;
            let s = if filled { "▰ " } else { "▱ " };
            pips_x = put_str(
                buf,
                pips_x,
                inner.y,
                s,
                2,
                heavy(if filled { HEAL_MINT } else { EDGE }).bg(PLATE),
            );
        }

        let sub_status = if can_confirm {
            "Pack sealed. Ready to move."
        } else {
            "Choose upgrades to carry forward."
        };
        put_str(
            buf,
            right_x,
            inner.y + 1,
            sub_status,
            rw,
            ink(if can_confirm { VICTORY_MINT } else { MUTED }).bg(PLATE),
        );
        hline(buf, right_x, inner.y + 2, rw, '─', ink(TRACK).bg(PLATE));

        // One pack slot card per slot: four rows each when they all fit above
        // the confirm block, three (name only) when a four- or five-slot pack
        // would otherwise lose its last cards.
        let mut ry = inner.y + 3;
        let slots_to_draw = slot_count;
        let slots_bottom = inner.bottom().saturating_sub(6);
        let slot_h: u16 = if ry + slots_to_draw as u16 * 4 <= slots_bottom { 4 } else { 3 };
        for (s_idx, maybe_item) in packed_indices.iter().enumerate().take(slots_to_draw) {
            if ry + slot_h > slots_bottom {
                break;
            }
            let sbox = Rect::new(right_x, ry, rw, slot_h);
            if let Some(item_idx) = *maybe_item {
                let it = &view.items[item_idx];
                let icol = to_color(it.color);
                let si = sub_box(buf, sbox, HEAL_MINT, PLATE_PACKED);
                let mut hdr = StackBuf::<24>::new();
                let _ = write!(hdr, " SLOT 0{} ", s_idx + 1);
                put_str(buf, sbox.x + 2, sbox.y, hdr.as_str(), 10, heavy(HEAL_MINT).bg(PLATE_PACKED));
                let text_x = if si.height >= art_icons::SMALL_CELL_H && si.width >= 16 {
                    let icon_rect = Rect::new(
                        si.x + 1,
                        si.y,
                        art_icons::SMALL_CELL_W,
                        art_icons::SMALL_CELL_H,
                    );
                    art_icons::draw(buf, icon_rect, art_icons::by_name(it.name));
                    si.x + 1 + art_icons::SMALL_CELL_W + 1
                } else {
                    let mut g = [0u8; 4];
                    let cx = put_str(buf, si.x + 1, si.y, it.glyph.encode_utf8(&mut g), 1, heavy(icol).bg(PLATE_PACKED));
                    put_str(buf, cx, si.y, " ", 1, ink(COAT).bg(PLATE_PACKED))
                };
                let mut tag = StackBuf::<16>::new();
                match it.category {
                    UpgradeCategory::Weapon { level, max_level } => {
                        let _ = write!(tag, "Lv{}/{}", level, max_level);
                    }
                    UpgradeCategory::Passive { stacks } => {
                        let _ = write!(tag, "×{}", stacks);
                    }
                }
                let tw = tag.as_str().chars().count() as u16;
                let nw = si.right().saturating_sub(text_x + tw + 2);
                put_str_ellipsis(buf, text_x, si.y, it.name, nw, heavy(COAT).bg(PLATE_PACKED));
                put_str(buf, si.right().saturating_sub(tw + 1), si.y, tag.as_str(), tw, heavy(icol).bg(PLATE_PACKED));
                if si.height >= 2 {
                    put_str_ellipsis(
                        buf,
                        text_x,
                        si.y + 1,
                        it.detail,
                        si.right().saturating_sub(text_x + 1),
                        ink(MUTED).bg(PLATE_PACKED),
                    );
                }
            } else {
                let si = sub_box(buf, sbox, TRACK, PLATE_DIM);
                let mut hdr = StackBuf::<24>::new();
                let _ = write!(hdr, " SLOT 0{} ", s_idx + 1);
                put_str(buf, sbox.x + 2, sbox.y, hdr.as_str(), 10, ink(MUTED).bg(PLATE_DIM));
                put_centered(buf, si, 0, "— empty slot —", ink(MUTED).bg(PLATE_DIM));
                let mut how = StackBuf::<48>::new();
                let _ = write!(how, "{} or click an upgrade", keys.toggle);
                put_centered(buf, si, 1, how.as_str(), ink(EDGE).bg(PLATE_DIM));
            }
            ry += slot_h;
        }

        // "LEFT BEHIND" section — reinforcing the emotional weight of sacrifice!
        let leaving = view.items.len().saturating_sub(selected_count);
        let confirm_y = inner.bottom().saturating_sub(3);
        let list_stop_y = confirm_y.saturating_sub(2);
        if ry + 3 <= list_stop_y {
            hline(buf, right_x, ry, rw, '─', ink(TRACK).bg(PLATE));
            ry += 1;
            let mut lb = StackBuf::<48>::new();
            let _ = write!(
                lb,
                "LEAVING IN {} ({})",
                view.from_level.name(),
                leaving
            );
            put_str_ellipsis(buf, right_x, ry, lb.as_str(), rw, heavy(MUTED).bg(PLATE));
            ry += 1;

            let mut shown = 0usize;
            for (i, it) in view.items.iter().enumerate() {
                if view.selected.get(i).copied().unwrap_or(false) {
                    continue;
                }
                if ry >= list_stop_y {
                    let rem = leaving.saturating_sub(shown);
                    if rem > 0 {
                        let mut more = StackBuf::<28>::new();
                        let _ = write!(more, "  + {} more left behind", rem);
                        put_str(buf, right_x, ry, more.as_str(), rw, ink(MUTED).bg(PLATE));
                    }
                    break;
                }
                let mut line = StackBuf::<40>::new();
                let _ = write!(line, "  × {} ", it.name);
                match it.category {
                    UpgradeCategory::Weapon { level, .. } => {
                        let _ = write!(line, "Lv{}", level);
                    }
                    UpgradeCategory::Passive { stacks } => {
                        let _ = write!(line, "×{}", stacks);
                    }
                }
                put_str_ellipsis(buf, right_x, ry, line.as_str(), rw, ink(EDGE).bg(PLATE));
                ry += 1;
                shown += 1;
            }
        }
        hline(buf, right_x, confirm_y.saturating_sub(1), rw, '─', ink(TRACK).bg(PLATE));

        // Big Confirm button in right pane.
        let mut cbuf = StackBuf::<64>::new();
        if can_confirm {
            let _ = write!(cbuf, " ▸  ENTER {}  ({}) ", view.next_level.name(), keys.confirm);
        } else {
            let rem = required.saturating_sub(selected_count);
            let _ = write!(cbuf, "  SELECT {} MORE TO CONFIRM  ", rem);
        }
        let cstate = if !can_confirm {
            ButtonState::Disabled
        } else if view.hovered == Some(KeepThreeAction::Confirm) {
            ButtonState::Hovered
        } else {
            ButtonState::Primary
        };
        let cw = (cbuf.as_str().chars().count() as u16).min(rw);
        let cx = right_x + (rw - cw) / 2;
        let crect = draw_button_at(buf, cx, confirm_y, cbuf.as_str(), cw, cstate, next_accent);
        if can_confirm {
            hits.push(crect, KeepThreeAction::Confirm);
        }

        // Bottom key hints across full panel.
        let mut legend = StackBuf::<96>::new();
        let _ = write!(
            legend,
            "↑↓ move   ·   {} / click pack or unpack   ·   {} confirm",
            keys.toggle, keys.confirm
        );
        put_centered(
            buf,
            inner,
            inner.height.saturating_sub(1),
            legend.as_str(),
            ink(MUTED).bg(PLATE),
        );
    } else {
        // ---- MEDIUM (80x24) & COMPACT (57x20) SINGLE-COLUMN LAYOUT ----
        let compact = layout.row_height == 1;
        let mut y = inner.y;

        // Top Escape Pack counter & inline slot pills.
        let mut hdr = StackBuf::<32>::new();
        let _ = write!(hdr, "PACK {}/{} ", selected_count, required);
        let mut hx = put_str(
            buf,
            inner.x + 1,
            y,
            hdr.as_str(),
            12,
            heavy(if can_confirm { HEAL_MINT } else { TAIL }).bg(PLATE),
        );
        for i in 0..required.min(SLOTS) {
            let filled = i < selected_count;
            hx = put_str(
                buf,
                hx,
                y,
                if filled { "●" } else { "○" },
                1,
                heavy(if filled { HEAL_MINT } else { EDGE }).bg(PLATE),
            );
        }
        let leaving = view.items.len().saturating_sub(selected_count);
        let mut lmsg = StackBuf::<44>::new();
        if compact {
            let _ = write!(lmsg, "  ·  {} left behind", leaving);
            // The compact plate has no narrative row: the gene splices (which
            // ride along without a slot, `WAVE3_DESIGN` §6) get their count
            // here instead.
            if !view.splices.is_empty() {
                let _ = write!(lmsg, "  ·  SPLICE {}", view.splices.len());
            }
        } else {
            let _ = write!(
                lmsg,
                "   ·   Leaving {} behind in {}",
                leaving,
                view.from_level.short_name()
            );
        }
        put_str_ellipsis(
            buf,
            hx,
            y,
            lmsg.as_str(),
            inner.right().saturating_sub(hx + 1),
            ink(MUTED).bg(PLATE),
        );
        y += 1;

        // Second header row: inline 3-slot pills (shown in both medium and compact modes).
        if inner.height >= 12 {
            let mut px = inner.x + 1;
            let slot_gap: u16 = if compact { 1 } else { 2 };
            // Three columns for a pack of up to three (the layout the plate
            // always had); a four- or five-slot pack divides the same row.
            let cols = slot_count.max(3) as u16;
            let slot_w = (inner.width.saturating_sub(2 + slot_gap * (cols - 1)) / cols).max(8);
            for (s_idx, maybe_idx) in packed_indices.iter().enumerate().take(slot_count) {
                if px + slot_w > inner.right() {
                    break;
                }
                let item = maybe_idx.and_then(|idx| view.items.get(idx));
                px = draw_pack_slot_pill(buf, px, y, slot_w, s_idx, item);
                if s_idx + 1 < slot_count {
                    px = put_str(buf, px, y, "  ", slot_gap, ink(PLATE).bg(PLATE));
                }
            }
            y += 1;
        }

        if !compact {
            // Subtle narrative reminder, plus the splices that ride along
            // without a slot (`WAVE3_DESIGN` §6).
            let mut hint = StackBuf::<96>::new();
            let _ = write!(
                hint,
                "Choose what fits through the vent. Unselected gear stays behind."
            );
            if !view.splices.is_empty() {
                let _ = write!(hint, "  ·  Splices ride along:");
                for (i, name) in view.splices.iter().enumerate() {
                    let _ = write!(hint, "{}{}", if i == 0 { " " } else { ", " }, name);
                }
            }
            put_str_ellipsis(
                buf,
                inner.x + 1,
                y,
                hint.as_str(),
                inner.width.saturating_sub(2),
                ink(MUTED).bg(PLATE),
            );
            y += 1;
        }

        hline(buf, inner.x + 1, y, inner.width.saturating_sub(2), '─', ink(TRACK).bg(PLATE));
        y += 1;

        // Footer reservation (rule + detail/confirm + hints = 3 rows).
        let footer_rows: u16 = 3;
        let list_y = y;
        let list_bottom = inner.bottom().saturating_sub(footer_rows);
        let list_h = list_bottom.saturating_sub(list_y);
        let list_x = inner.x + 1;
        let list_w = if needs_scroll {
            inner.width.saturating_sub(4)
        } else {
            inner.width.saturating_sub(2)
        };

        if view.items.is_empty() {
            put_centered(
                buf,
                Rect::new(list_x, list_y, list_w, list_h.max(1)),
                0,
                "No upgrades owned — ready to proceed.",
                ink(MUTED).bg(PLATE),
            );
        } else {
            for slot in 0..visible {
                let idx = scroll_top + slot;
                if idx >= view.items.len() {
                    break;
                }
                let ry = list_y + slot as u16 * layout.row_height;
                if ry + layout.row_height > list_bottom {
                    break;
                }
                let rrect = Rect::new(list_x, ry, list_w, layout.row_height);
                let is_sel = view.selected.get(idx).copied().unwrap_or(false);
                let is_foc = idx == view.cursor;
                let is_hov = view.hovered == Some(KeepThreeAction::ToggleItem(idx));
                draw_manifest_row(
                    buf,
                    rrect,
                    &view.items[idx],
                    is_sel,
                    is_foc,
                    is_hov,
                    pack_full,
                    layout.row_height >= 2,
                );
                hits.push(clamp_rect(rrect, area), KeepThreeAction::ToggleItem(idx));
            }
        }

        if needs_scroll && list_h >= 3 {
            let sx = list_x + list_w + 1;
            draw_scrollbar(
                buf,
                sx,
                list_y,
                list_h,
                scroll_top,
                max_scroll,
                view.hovered,
                &mut hits,
            );
        }

        // Compact focused-item detail line (so 1-line mode still shows full description).
        if compact && list_bottom < inner.bottom() {
            hline(buf, inner.x + 1, list_bottom, inner.width.saturating_sub(2), '─', ink(TRACK).bg(PLATE));
            let detail_y = list_bottom + 1;
            if detail_y < inner.bottom() {
                if let Some(foc) = view.items.get(view.cursor) {
                    let mut db = StackBuf::<72>::new();
                    let _ = write!(db, "{} {}: {}", foc.glyph, foc.name, foc.detail);
                    put_str_ellipsis(
                        buf,
                        inner.x + 1,
                        detail_y,
                        db.as_str(),
                        inner.width.saturating_sub(2),
                        ink(MUTED).bg(PLATE),
                    );
                }
            }
        } else if list_bottom < inner.bottom() {
            hline(buf, inner.x + 1, list_bottom, inner.width.saturating_sub(2), '─', ink(TRACK).bg(PLATE));
        }

        // Bottom confirm button + hints.
        let confirm_row = if compact {
            inner.bottom().saturating_sub(1)
        } else {
            inner.bottom().saturating_sub(2)
        };
        let mut cbuf = StackBuf::<64>::new();
        if can_confirm {
            if inner.width >= 58 {
                let _ = write!(cbuf, " ▸  CONFIRM & ENTER {}  ({}) ", view.next_level.short_name().to_uppercase(), keys.confirm);
            } else {
                let _ = write!(cbuf, " ▸ CONFIRM ({}/{}) ", selected_count, required);
            }
        } else {
            let rem = required.saturating_sub(selected_count);
            if inner.width >= 58 {
                let _ = write!(cbuf, "  SELECT {} MORE ({}/{})  ", rem, selected_count, required);
            } else {
                let _ = write!(cbuf, " PICK {} MORE ", rem);
            }
        }
        let cstate = if !can_confirm {
            ButtonState::Disabled
        } else if view.hovered == Some(KeepThreeAction::Confirm) {
            ButtonState::Hovered
        } else {
            ButtonState::Primary
        };

        if compact {
            // Left: hints, Right: confirm button.
            let cw = (cbuf.as_str().chars().count() as u16).min(inner.width.saturating_sub(2));
            let cx = inner.right().saturating_sub(cw + 1);
            let hint_w = cx.saturating_sub(inner.x + 2);
            if hint_w >= 10 {
                let mut legend = StackBuf::<64>::new();
                let _ = write!(legend, "↑↓ nav · {} toggle · {}", keys.toggle, keys.confirm);
                put_str_ellipsis(
                    buf,
                    inner.x + 1,
                    confirm_row,
                    legend.as_str(),
                    hint_w,
                    ink(MUTED).bg(PLATE),
                );
            }
            let crect = draw_button_at(buf, cx, confirm_row, cbuf.as_str(), cw, cstate, next_accent);
            if can_confirm {
                hits.push(crect, KeepThreeAction::Confirm);
            }
        } else {
            let cw = (cbuf.as_str().chars().count() as u16).min(inner.width.saturating_sub(2));
            let cx = inner.x + (inner.width - cw) / 2;
            let crect = draw_button_at(buf, cx, confirm_row, cbuf.as_str(), cw, cstate, next_accent);
            if can_confirm {
                hits.push(crect, KeepThreeAction::Confirm);
            }
            let mut legend = StackBuf::<96>::new();
            let _ = write!(
                legend,
                "↑↓ select   ·   {}/click pack   ·   {} confirm",
                keys.toggle, keys.confirm
            );
            put_centered(
                buf,
                inner,
                inner.height.saturating_sub(1),
                legend.as_str(),
                ink(MUTED).bg(PLATE),
            );
        }
    }

    // The next stage's route (`WAVE3_DESIGN.md` §1). The two chips live on the
    // panel header, after the caption, so every layout (120x40, 80x24, 57x20)
    // shows the choice without losing a manifest row. Original route first;
    // Tab toggles, and the chips are clickable.
    let has_route = view.route.iter().any(|chip| chip.is_some());
    if has_route && layout.panel_rect.width >= 40 {
        let y = layout.panel_rect.y;
        let mut x = layout.panel_rect.x + 2 + cap.as_str().chars().count() as u16 + 3;
        let room = layout.panel_rect.right().saturating_sub(3);
        for (i, chip) in view.route.iter().enumerate() {
            let Some(chip) = chip else { continue };
            let mut lbl = StackBuf::<16>::new();
            let _ = write!(lbl, "[{}]", chip.name);
            let w = lbl.as_str().chars().count() as u16;
            if x + w > room {
                break;
            }
            let style = if chip.chosen {
                heavy(HEAL_MINT).bg(PLATE)
            } else if view.hovered == Some(KeepThreeAction::Route(i as u8)) {
                ink(COAT).bg(PLATE_HILITE)
            } else {
                ink(MUTED).bg(PLATE)
            };
            put_str(buf, x, y, lbl.as_str(), w, style);
            hits.push(Rect::new(x, y, w, 1), KeepThreeAction::Route(i as u8));
            x += w + 1;
        }
        // The purse, when the chips took its place on the header.
        if !gold_in_caption {
            let mut purse = StackBuf::<24>::new();
            let _ = write!(purse, " GOLD {} ", view.gold);
            let pw = purse.as_str().chars().count() as u16;
            put_str(
                buf,
                layout.panel_rect.x + 2,
                layout.panel_rect.bottom().saturating_sub(1),
                purse.as_str(),
                pw,
                ink(TRAIL_GOLD).bg(PLATE),
            );
        }
        // The key hint rides the bottom border, where nothing else is drawn.
        let mut route_hint = StackBuf::<32>::new();
        let _ = write!(route_hint, "{} route", keys.route);
        let hint = route_hint.as_str();
        let hw = hint.chars().count() as u16;
        let hx = layout.panel_rect.right().saturating_sub(hw + 2);
        put_str(
            buf,
            hx,
            layout.panel_rect.bottom().saturating_sub(1),
            hint,
            hw,
            ink(EDGE).bg(PLATE),
        );
    }

    hits
}

#[allow(clippy::too_many_arguments)]
fn draw_scrollbar(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    height: u16,
    scroll_top: usize,
    max_scroll: usize,
    hovered: Option<KeepThreeAction>,
    hits: &mut HitList<KeepThreeAction, 32>,
) {
    if height < 3 || x < buf.area.x || x >= buf.area.right() {
        return;
    }
    let can_up = scroll_top > 0;
    let can_down = scroll_top < max_scroll;

    let up_style = if !can_up {
        ink(TRACK).bg(PLATE)
    } else if hovered == Some(KeepThreeAction::ScrollUp) {
        heavy(PLATE).bg(TAIL)
    } else {
        heavy(COAT).bg(TRACK)
    };
    put_str(buf, x, y, "▴", 1, up_style);
    if can_up {
        hits.push(
            clamp_rect(Rect::new(x, y, 1, 1), buf.area),
            KeepThreeAction::ScrollUp,
        );
    }

    let track_h = height.saturating_sub(2);
    let thumb_rel = if max_scroll == 0 || track_h <= 1 {
        0
    } else {
        ((scroll_top * usize::from(track_h - 1)) + max_scroll / 2) / max_scroll
    } as u16;
    for dy in 0..track_h {
        let cy = y + 1 + dy;
        if dy == thumb_rel {
            put_str(buf, x, cy, "█", 1, ink(TAIL).bg(PLATE));
        } else {
            put_str(buf, x, cy, "│", 1, ink(TRACK).bg(PLATE));
        }
    }

    let bot_y = y + height - 1;
    let down_style = if !can_down {
        ink(TRACK).bg(PLATE)
    } else if hovered == Some(KeepThreeAction::ScrollDown) {
        heavy(PLATE).bg(TAIL)
    } else {
        heavy(COAT).bg(TRACK)
    };
    put_str(buf, x, bot_y, "▾", 1, down_style);
    if can_down {
        hits.push(
            clamp_rect(Rect::new(x, bot_y, 1, 1), buf.area),
            KeepThreeAction::ScrollDown,
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Level Intro Screen (Levels 2 & 3, consistent with menu::draw_map_intro)
// ---------------------------------------------------------------------------

fn level_emblem(level: CampaignLevel) -> [&'static str; 3] {
    match level {
        CampaignLevel::Sewer => [
            "      ╭──┬──╮      ",
            "      ╰──┼──╯      ",
            "═════╡═══╧═══╞═════",
        ],
        CampaignLevel::Kitchen => [
            " ▄▄▄   ▄▄▄   ▄▄▄ ",
            " ▀█▀   ▀█▀   ▀█▀ ",
            "══╧═════╧═════╧══",
        ],
        CampaignLevel::Lab => [
            "  ╭──╮      ╭──╮  ",
            "──┤  ├──»───┤  ├──",
            "  ╰──╯      ╰──╯  ",
        ],
        // The side route floors: the Mangle's roller bay; the Printworks still
        // borrows the mower's until the Night Edition lands.
        CampaignLevel::Laundry => [
            "  ╭─┬─┬─╮  ╭─╮  ",
            "  │▓│▓│▓│  │◉│  ",
            "══╧═╧═╧═╧══╧═╧══",
        ],
        CampaignLevel::Printworks => [
            "  ╭──╮      ╭──╮  ",
            "──┤  ├──»───┤  ├──",
            "  ╰──╯      ╰──╯  ",
        ],
        // The getaway ride: a flatbed deck at speed, cargo lashed down.
        CampaignLevel::Trailer => [
            "  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ",
            "──╡▓▓▓▓▓▓▓▓▓▓▓▓╞──",
            "  ╧═╧═╧═╧═╧═╧═╧═╧  ",
        ],
    }
}

/// Draw the pre-level intro card for Campaign Levels 2 & 3 (and Level 1 if used).
///
/// Visual language mirrors `menu::draw_map_intro` (stamped `ACCESS ROUTE` plate,
/// 3-line architectural emblem, heading, flavor text, and Begin pill) extended
/// with a `CARRIED IN` section showing the upgrades packed from the prior sector.
pub fn draw_level_intro(
    f: &mut Frame,
    area: Rect,
    view: &LevelIntroView<'_>,
) -> HitList<LevelIntroAction, 4> {
    draw_level_intro_with(f, area, view, &PlateKeys::DEFAULT)
}

/// [`draw_level_intro`] with the key name its begin button prints.
pub fn draw_level_intro_with(
    f: &mut Frame,
    area: Rect,
    view: &LevelIntroView<'_>,
    keys: &PlateKeys<'_>,
) -> HitList<LevelIntroAction, 4> {
    let mut hits = HitList::new();
    if area.width < 12 || area.height < 6 {
        return hits;
    }

    let accent = view.level.accent();
    // The nest's carry track raises the keep limit from three to five
    // (`campaign::keep_limit`), so the plate lists up to five, not three.
    let carried_n = view.carried.len().min(MAX_CARRIED_SHOWN);
    let very_tall = area.width >= 74 && area.height >= 26;
    let spacious = area.width >= 66 && area.height >= 22;
    let pw = if very_tall {
        72
    } else if area.width >= 68 {
        66
    } else {
        area.width.min(60)
    };
    let ph = if very_tall {
        22
    } else if spacious {
        20
    } else if area.height >= 18 {
        18
    } else {
        area.height
    };
    let card = centered_rect(pw, ph, area);

    let buf = f.buffer_mut();
    // The ride's card is captioned as the getaway: it is the campaign's last
    // sector and the way out.
    let cap = if view.level == CampaignLevel::Trailer {
        " THE GETAWAY "
    } else {
        " ACCESS ROUTE "
    };
    let inner = panel(buf, card, cap, None, accent);
    stamp_header_pips(
        buf,
        card,
        cap,
        view.level,
        view.level.stage_number().saturating_sub(1),
    );

    if inner.width < 10 || inner.height < 4 {
        return hits;
    }

    let mut y: u16 = 0;

    // 3-line architectural emblem matching `menu::draw_map_intro`.
    if inner.height >= 14 {
        for line in level_emblem(view.level) {
            put_centered(buf, inner, y, line, ink(accent).bg(PLATE));
            y += 1;
        }
        y += 1;
    }

    // Heading: SECTOR NN — NAME, the same words as level 1's access-route
    // card (`lore::map_intro`) and STYLE.md's "Sector". The ride is the
    // campaign's fourth and last sector, and its caption names the getaway
    // instead (batch twelve: the Flatbed is the finale).
    let mut heading = StackBuf::<40>::new();
    let _ = write!(
        heading,
        "{} — {}",
        view.level.sector_code(),
        view.level.name()
    );
    put_centered(buf, inner, y, heading.as_str(), heavy(COAT).bg(PLATE));
    y += if inner.height >= 15 { 2 } else { 1 };

    // Flavor text.
    let flavor = view
        .flavor
        .unwrap_or_else(|| view.level.default_intro_flavor());
    let inset = if inner.width >= 40 { 3 } else { 1 };
    let flavor_h = if inner.height >= 15 { 3 } else { 2 };
    let flavor_rect = Rect::new(
        inner.x + inset,
        inner.y + y,
        inner.width.saturating_sub(inset * 2),
        flavor_h.min(inner.height.saturating_sub(y + 4).max(1)),
    );
    let lines_used = put_wrapped(buf, flavor_rect, flavor, ink(MUTED).bg(PLATE), true);
    y += lines_used + u16::from(inner.height >= 16);

    // CARRIED IN section.
    let btn_row = inner.height.saturating_sub(1);
    if y + 2 <= btn_row {
        hline(
            buf,
            inner.x + 2,
            inner.y + y,
            inner.width.saturating_sub(4),
            '─',
            ink(TRACK).bg(PLATE),
        );
        let lbl = " CARRIED IN ";
        let lx = inner.x + 4;
        if inner.width >= 24 {
            put_str(buf, lx, inner.y + y, lbl, 12, heavy(accent).bg(PLATE));
        }
        if inner.width >= 28 {
            let mut sb = StackBuf::<40>::new();
            if view.score_so_far > 0 && inner.width >= 52 {
                let _ = write!(sb, " SCORE {} ·", view.score_so_far);
            }
            let _ = write!(sb, " GOLD {} ", view.gold);
            let sw = sb.as_str().chars().count() as u16;
            put_str(
                buf,
                inner.right().saturating_sub(sw + 4),
                inner.y + y,
                sb.as_str(),
                sw,
                ink(TRAIL_GOLD).bg(PLATE),
            );
        }
        y += 1;

        if carried_n == 0 {
            put_centered(
                buf,
                inner,
                y,
                "Nothing carried — claws and nerve only.",
                ink(MUTED).bg(PLATE),
            );
        } else if spacious && carried_n <= 3 && y + 3 <= btn_row {
            // Up to three side-by-side mini cards; a bigger pack falls
            // through to the stacked rows or the pills below.
            let card_h: u16 = if very_tall && y + 4 <= btn_row { 4 } else { 3 };
            let gap = 1u16;
            let total_w = inner.width.saturating_sub(4);
            let card_w = total_w.saturating_sub(gap * 2) / 3;
            let start_x = inner.x + 2 + (total_w - (card_w * 3 + gap * 2)) / 2;
            for (i, it) in view.carried.iter().take(3).enumerate() {
                let cx = start_x + i as u16 * (card_w + gap);
                let crect = Rect::new(cx, inner.y + y, card_w, card_h);
                let icol = to_color(it.color);
                let ci = sub_box(buf, crect, EDGE, PLATE_SUB);
                let show_ci_icon = card_h >= 4 && ci.height >= art_icons::SMALL_CELL_H && ci.width >= 15;
                let tx = if show_ci_icon {
                    let icon_rect = Rect::new(
                        ci.x,
                        ci.y,
                        art_icons::SMALL_CELL_W,
                        art_icons::SMALL_CELL_H,
                    );
                    art_icons::draw(buf, icon_rect, art_icons::by_name(it.name));
                    ci.x + art_icons::SMALL_CELL_W + 1
                } else {
                    let mut g = [0u8; 4];
                    let t0 = put_str(
                        buf,
                        ci.x,
                        ci.y,
                        it.glyph.encode_utf8(&mut g),
                        1,
                        heavy(icol).bg(PLATE_SUB),
                    );
                    put_str(buf, t0, ci.y, " ", 1, ink(COAT).bg(PLATE_SUB))
                };
                let mut badge = StackBuf::<28>::new();
                match it.category {
                    UpgradeCategory::Weapon { level, max_level } => {
                        if ci.width >= 18 && card_h == 3 {
                            let pips = fmt_pips(level, max_level);
                            let _ = write!(badge, "{}", pips.as_str());
                        } else if level >= max_level {
                            let _ = write!(badge, "Lv{}★", level);
                        } else {
                            let _ = write!(badge, "Lv{}", level);
                        }
                    }
                    UpgradeCategory::Passive { stacks } => {
                        let _ = write!(badge, "×{}", stacks);
                    }
                }
                let bw = badge.as_str().chars().count() as u16;
                let nw = ci.right().saturating_sub(tx + bw + 1);
                put_str_ellipsis(buf, tx, ci.y, it.name, nw, heavy(COAT).bg(PLATE_SUB));
                put_str(
                    buf,
                    ci.right().saturating_sub(bw),
                    ci.y,
                    badge.as_str(),
                    bw,
                    heavy(if matches!(it.category, UpgradeCategory::Weapon { level, max_level } if level >= max_level) {
                        TRAIL_GOLD
                    } else {
                        icol
                    })
                    .bg(PLATE_SUB),
                );
                if card_h >= 4 && ci.height >= 2 {
                    let dx = if show_ci_icon { tx } else { ci.x };
                    put_str_ellipsis(
                        buf,
                        dx,
                        ci.y + 1,
                        it.detail,
                        ci.right().saturating_sub(dx),
                        ink(MUTED).bg(PLATE_SUB),
                    );
                }
            }
        } else {
            // Compact stacked or inline rows for carried upgrades.
            let avail_rows = btn_row.saturating_sub(y);
            if avail_rows >= carried_n as u16 {
                for (i, it) in view.carried.iter().take(carried_n).enumerate() {
                    let row_y = inner.y + y + i as u16;
                    let icol = to_color(it.color);
                    let mut g = [0u8; 4];
                    let mut cx = put_str(buf, inner.x + 3, row_y, "◆ ", 2, ink(HEAL_MINT).bg(PLATE));
                    cx = put_str(buf, cx, row_y, it.glyph.encode_utf8(&mut g), 1, heavy(icol).bg(PLATE));
                    cx = put_str(buf, cx, row_y, " ", 1, ink(COAT).bg(PLATE));
                    let mut tier = StackBuf::<32>::new();
                    match it.category {
                        UpgradeCategory::Weapon { level, max_level } => {
                            let pips = fmt_pips(level, max_level);
                            let _ = write!(tier, "Lv{} {}", level, pips.as_str());
                        }
                        UpgradeCategory::Passive { stacks } => {
                            let _ = write!(tier, "×{} passive", stacks);
                        }
                    }
                    let tw = tier.as_str().chars().count() as u16;
                    let nw = inner.right().saturating_sub(cx + tw + 4);
                    put_str_ellipsis(buf, cx, row_y, it.name, nw, heavy(COAT).bg(PLATE));
                    put_str(
                        buf,
                        inner.right().saturating_sub(tw + 3),
                        row_y,
                        tier.as_str(),
                        tw,
                        ink(icol).bg(PLATE),
                    );
                }
            } else {
                // Single-line inline pills when height is very tight.
                let mut px = inner.x + 2;
                let slot_w = (inner.width.saturating_sub(6) / carried_n.max(1) as u16).max(8);
                for (i, it) in view.carried.iter().take(carried_n).enumerate() {
                    // Never start a pill that would run off the plate.
                    if px + slot_w > inner.right() {
                        break;
                    }
                    px = draw_pack_slot_pill(buf, px, inner.y + y, slot_w, i, Some(it));
                    px = put_str(buf, px, inner.y + y, " ", 1, ink(PLATE).bg(PLATE));
                }
            }
        }
    }

    // Begin CTA pill matching `menu::draw_map_intro`.
    let mut btn = StackBuf::<64>::new();
    let _ = if inner.width >= 30 {
        write!(btn, " ▸  {} / click   begin ", keys.confirm)
    } else {
        write!(btn, " ▸ {}  begin ", keys.confirm)
    };
    let btn_label = btn.as_str();
    let bw = (btn_label.chars().count() as u16).min(inner.width);
    let bx = inner.x + (inner.width - bw) / 2;
    let bstate = if view.hovered == Some(LevelIntroAction::Begin) {
        ButtonState::Hovered
    } else {
        ButtonState::Primary
    };
    let brect = draw_button_at(buf, bx, inner.y + btn_row, btn_label, bw, bstate, accent);
    hits.push(brect, LevelIntroAction::Begin);

    hits
}

// ---------------------------------------------------------------------------
// 4. Campaign Victory Screen
// ---------------------------------------------------------------------------

/// Draw the Campaign Victory screen ("the rat is out") with final score and
/// per-level breakdown across Sewer, Kitchen, and Lab.
pub fn draw_campaign_victory(
    f: &mut Frame,
    area: Rect,
    view: &CampaignVictoryView<'_>,
) -> HitList<CampaignVictoryAction, 4> {
    draw_campaign_victory_with(f, area, view, &PlateKeys::DEFAULT)
}

/// [`draw_campaign_victory`] with the confirm key its replay button prints
/// (the menu key is already `view.menu_key`).
pub fn draw_campaign_victory_with(
    f: &mut Frame,
    area: Rect,
    view: &CampaignVictoryView<'_>,
    keys: &PlateKeys<'_>,
) -> HitList<CampaignVictoryAction, 4> {
    let mut hits = HitList::new();
    if area.width < 12 || area.height < 6 {
        return hits;
    }

    let wide = area.width >= 76;
    let tall = area.height >= 26;
    let pw = if wide {
        area.width.min(78)
    } else {
        area.width.min(64)
    };
    let ph = if tall {
        area.height.min(28)
    } else if area.height >= 22 {
        area.height.min(23)
    } else {
        area.height
    };
    let card = centered_rect(pw, ph, area);

    let buf = f.buffer_mut();
    let cap = if card.width >= 60 {
        " EXTRACTION COMPLETE   /   SUBJECT R-47 "
    } else {
        " EXTRACTION COMPLETE "
    };
    let inner = panel(buf, card, cap, None, VICTORY_MINT);
    stamp_header_pips(
        buf,
        card,
        cap,
        CampaignLevel::Trailer,
        crate::game::campaign::LEVEL_COUNT as u8,
    );

    if inner.width < 10 || inner.height < 5 {
        return hits;
    }

    let mut y: u16 = u16::from(inner.height >= 20);

    // Top status pill (+ optional NEW RECORD badge).
    let primary_pill = if inner.width >= 34 {
        " ▰▰  THE RAT IS OUT  ▰▰ "
    } else {
        " ▸ THE RAT IS OUT "
    };
    if view.new_best && inner.width >= 52 {
        let rec_pill = " ◆ NEW RECORD ◆ ";
        let total_w = primary_pill.chars().count() as u16 + 2 + rec_pill.chars().count() as u16;
        let sx = inner.x + (inner.width - total_w) / 2;
        let cx = put_str(buf, sx, inner.y + y, primary_pill, total_w, heavy(PLATE).bg(VICTORY_MINT));
        let cx = put_str(buf, cx, inner.y + y, "  ", 2, ink(PLATE).bg(PLATE));
        put_str(buf, cx, inner.y + y, rec_pill, total_w, heavy(PLATE).bg(TRAIL_GOLD));
    } else {
        put_centered(buf, inner, y, primary_pill, heavy(PLATE).bg(VICTORY_MINT));
    }
    y += if inner.height >= 22 { 2 } else { 1 };

    // Open blast door / night air emblem on tall screens (31 cols, mirror-symmetric).
    if inner.height >= 21 && inner.width >= 38 {
        let emblem = [
            ("   ▐█▌     ·   ✦   ·     ▐█▌   ", TRAIL_GOLD),
            ("═══▐█▌  ╱  OPEN  AIR  ╲  ▐█▌═══", VICTORY_MINT),
            ("   ▐█▌ ╱═══════════════╲ ▐█▌   ", HEAL_MINT),
        ];
        for (i, (line, col)) in emblem.iter().enumerate() {
            put_centered(buf, inner, y + i as u16, line, ink(*col).bg(PLATE));
        }
        y += 4;
    }

    // Finale narrative line. The run ends on the ride now (batch twelve), so
    // the plate's own words count four sectors; a caller may override it with
    // the ride's line (`lore::GETAWAY_WIN`).
    let default_blurb = if inner.width < 60 && inner.height < 20 {
        "Four sectors breached. Open air smells like rain and asphalt."
    } else {
        "Every alarm flatlines behind you. Four sectors, four apex threats — none of them held. The deck runs out onto open road."
    };
    let blurb = view.custom_blurb.unwrap_or(default_blurb);
    let inset = if inner.width >= 42 { 3 } else { 1 };
    let blurb_rect = Rect::new(
        inner.x + inset,
        inner.y + y,
        inner.width.saturating_sub(inset * 2),
        if inner.height >= 17 { 2 } else { 1 },
    );
    let used = put_wrapped(buf, blurb_rect, blurb, ink(MUTED).bg(PLATE), true);
    y += used + u16::from(inner.height >= 20);

    // Final Totals highlight strip.
    if y + 3 < inner.height {
        hline(buf, inner.x + 2, inner.y + y, inner.width.saturating_sub(4), '─', ink(TRACK).bg(PLATE));
        y += 1;

        let ttime = fmt_time(view.total_time_secs);
        if inner.width >= 62 {
            let mut sb = StackBuf::<16>::new();
            let _ = write!(sb, "{}", view.final_score);
            let mut kb = StackBuf::<16>::new();
            let _ = write!(kb, "{}", view.total_kills);
            let mut gold_b = StackBuf::<16>::new();
            let _ = write!(gold_b, "{}", view.gold);
            let cols = 5u16;
            let cw = inner.width.saturating_sub(4) / cols;
            let x0 = inner.x + 2;
            let totals: [(&str, &str, Color); 5] = [
                ("FINAL SCORE", sb.as_str(), TRAIL_GOLD),
                ("TOTAL TIME", ttime.as_str(), COAT),
                ("TOTAL KILLS", kb.as_str(), COAT),
                ("SECTORS", "4 / 4", HEAL_MINT),
                ("GOLD", gold_b.as_str(), TRAIL_GOLD),
            ];
            for (i, (k, v, vc)) in totals.iter().enumerate() {
                let cx = x0 + i as u16 * cw;
                let cell = Rect::new(cx, inner.y + y, cw, 2);
                put_centered(buf, cell, 0, k, ink(MUTED).bg(PLATE));
                put_centered(buf, cell, 1, v, heavy(*vc).bg(PLATE));
                if i + 1 < totals.len() {
                    vline(buf, cx + cw.saturating_sub(1), inner.y + y, 2, '│', ink(TRACK).bg(PLATE));
                }
            }
            y += 2;
        } else {
            let mut line = StackBuf::<64>::new();
            let _ = write!(
                line,
                "FINAL {} · {} · {} kills · 4 / 4 · Gold {}",
                view.final_score,
                ttime.as_str(),
                view.total_kills,
                view.gold
            );
            put_centered(buf, inner, y, line.as_str(), heavy(TRAIL_GOLD).bg(PLATE));
            y += 1;
        }
    }

    // Per-Level Breakdown table (Sewer, stage two, stage three, the Flatbed):
    // a rule, a header and one row per level.
    let btn_row = inner.height.saturating_sub(1);
    let rows = view.levels.len() as u16;
    if y + 2 + rows <= btn_row {
        hline(buf, inner.x + 2, inner.y + y, inner.width.saturating_sub(4), '─', ink(TRACK).bg(PLATE));
        y += 1;

        let show_boss = inner.width >= 66;
        let table_w: u16 = if show_boss { 64 } else { inner.width.saturating_sub(4).min(50) };
        let tx = inner.x + (inner.width - table_w) / 2;

        if show_boss {
            put_str(
                buf,
                tx,
                inner.y + y,
                "SECTOR          BOSS SLAIN           TIME   KILLS     SCORE",
                table_w,
                heavy(VICTORY_MINT).bg(PLATE),
            );
        } else {
            put_str(
                buf,
                tx,
                inner.y + y,
                "SECTOR         TIME    KILLS     SCORE",
                table_w,
                heavy(VICTORY_MINT).bg(PLATE),
            );
        }
        y += 1;

        for row in &view.levels {
            if y >= btn_row {
                break;
            }
            let rtime = fmt_time(row.time_secs);
            let mut line = StackBuf::<72>::new();
            let sec_col = row.level.accent();
            let mut badge = StackBuf::<16>::new();
            let _ = write!(badge, "0{} {:<10}", row.level.number(), row.level.short_name());
            put_str(buf, tx, inner.y + y, badge.as_str(), 14, heavy(sec_col).bg(PLATE));

            if show_boss {
                let _ = write!(
                    line,
                    "  {:<18}  {}   {:>5}   {:>7}",
                    // The boss that was killed, when the App knows it; else
                    // the floor's original.
                    row.boss
                        .map(|kind| crate::render::hud::boss_codename_kind(kind).0)
                        .unwrap_or(row.level.boss_name()),
                    rtime.as_str(),
                    row.kills,
                    row.score
                );
                put_str(buf, tx + 14, inner.y + y, line.as_str(), table_w.saturating_sub(14), ink(COAT).bg(PLATE));
            } else {
                let _ = write!(
                    line,
                    " {}   {:>5}   {:>7}",
                    rtime.as_str(),
                    row.kills,
                    row.score
                );
                put_str(buf, tx + 14, inner.y + y, line.as_str(), table_w.saturating_sub(14), ink(COAT).bg(PLATE));
            }
            y += 1;
        }
    }

    // Optional Final Loadout strip when vertical space remains.
    if !view.final_loadout.is_empty() && y + 2 <= btn_row && inner.width >= 48 {
        y += 1;
        let mut sx = inner.x + 3;
        sx = put_str(buf, sx, inner.y + y, "CARRIED OUT  ", 13, heavy(VICTORY_MINT).bg(PLATE));
        for (i, it) in view.final_loadout.iter().take(6).enumerate() {
            if sx + 8 >= inner.right() {
                break;
            }
            if i > 0 {
                sx = put_str(buf, sx, inner.y + y, "  ", 2, ink(PLATE).bg(PLATE));
            }
            let mut tok = StackBuf::<16>::new();
            let _ = tok.write_char(it.glyph);
            let mut prefix = ['\0'; 5];
            let mut n = 0usize;
            for ch in it.name.chars().take(5) {
                prefix[n] = ch;
                n += 1;
            }
            while n > 0 && prefix[n - 1] == ' ' {
                n -= 1;
            }
            for &ch in &prefix[..n] {
                let _ = tok.write_char(ch);
            }
            match it.category {
                UpgradeCategory::Weapon { level, max_level } => {
                    if level >= max_level {
                        let _ = tok.write_char('★');
                    } else {
                        let _ = write!(tok, "{}", level);
                    }
                }
                UpgradeCategory::Passive { stacks } => {
                    let _ = write!(tok, "×{}", stacks);
                }
            }
            sx = put_str(
                buf,
                sx,
                inner.y + y,
                tok.as_str(),
                inner.right().saturating_sub(sx + 1),
                ink(to_color(it.color)).bg(PLATE),
            );
        }
    }

    // Bottom buttons: Play Again & Menu.
    let mut play = StackBuf::<64>::new();
    let _ = if inner.width >= 44 {
        write!(play, " ▸ {}  new campaign ", keys.confirm)
    } else {
        write!(play, " {} replay ", keys.confirm)
    };
    let play_lbl = play.as_str();
    let menu_lbl = format!(" {}  to menu ", view.menu_key);
    let menu_lbl = menu_lbl.as_str();
    let gap = 3u16;
    let pw_b = play_lbl.chars().count() as u16;
    let mw_b = menu_lbl.chars().count() as u16;
    let total = (pw_b + gap + mw_b).min(inner.width);
    let bx = inner.x + (inner.width - total) / 2;

    let play_state = if view.hovered == Some(CampaignVictoryAction::PlayAgain) {
        ButtonState::Hovered
    } else if view.focus == CampaignVictoryAction::PlayAgain {
        ButtonState::Focused
    } else {
        ButtonState::Normal
    };
    let menu_state = if view.hovered == Some(CampaignVictoryAction::Menu) {
        ButtonState::Hovered
    } else if view.focus == CampaignVictoryAction::Menu {
        ButtonState::Focused
    } else {
        ButtonState::Normal
    };

    let play_r = draw_button_at(
        buf,
        bx,
        inner.y + btn_row,
        play_lbl,
        pw_b.min(inner.right().saturating_sub(bx)),
        play_state,
        VICTORY_MINT,
    );
    hits.push(play_r, CampaignVictoryAction::PlayAgain);

    let mx = bx + pw_b + gap;
    if mx < inner.right() {
        let menu_r = draw_button_at(
            buf,
            mx,
            inner.y + btn_row,
            menu_lbl,
            mw_b.min(inner.right().saturating_sub(mx)),
            menu_state,
            VICTORY_MINT,
        );
        hits.push(menu_r, CampaignVictoryAction::Menu);
    }

    hits
}

// ---------------------------------------------------------------------------
// 5. Title Level Select (every floor, in route order: locked / open / best)
// ---------------------------------------------------------------------------

/// How many columns a sector-select grid uses at `area`.
///
/// Three on a wide terminal, two once the cards would be too narrow, one on a
/// narrow one. The screen's keyboard `Up`/`Down` step by this many slots, so
/// the host can move through the grid in visual order without duplicating the
/// geometry (see `app.rs::update_sector_select`).
pub fn level_select_cols(area: Rect) -> usize {
    let width = area.width.min(92);
    if width >= 72 {
        3
    } else if width >= 42 {
        2
    } else {
        1
    }
}

/// The short stage chip stamped on a slot's border: `S1`, `S2`, `S3`, `S4`
/// (the ride is the fourth level now; the defensive `GETAWAY` fallback covers a
/// host that passes an out-of-range stage).
fn slot_stage_chip(stage: usize) -> StackBuf<8> {
    let mut b = StackBuf::<8>::new();
    if stage >= crate::game::campaign::LEVEL_COUNT {
        let _ = write!(b, "GETAWAY");
    } else {
        let _ = write!(b, "S{}", stage + 1);
    }
    b
}

/// The status line on a slot: cleared, open or locked.
fn slot_status(slot: &LevelSlotView) -> &'static str {
    if !slot.unlocked {
        "× LOCKED"
    } else if slot.best.is_some_and(|b| b.cleared) {
        "★ CLEARED"
    } else {
        "◆ OPEN"
    }
}

fn slot_status_style(slot: &LevelSlotView, bg: Color) -> Style {
    if !slot.unlocked {
        ink(MUTED).bg(bg)
    } else if slot.best.is_some_and(|b| b.cleared) {
        heavy(HEAL_MINT).bg(bg)
    } else {
        heavy(slot.level.accent()).bg(bg)
    }
}

/// What a locked floor needs before it opens (its level's reach rule).
fn slot_unlock_hint(slot: &LevelSlotView) -> &'static str {
    match slot.stage {
        0 => "Locked",
        1 => "Clear the Sewers first",
        2 => "Clear an S2 sector first",
        _ => "Clear an S3 sector first",
    }
}

/// Draw one floor's card into `rect`.
///
/// Adapts to the grid's card height: a two-row card carries the name and a
/// combined `BEST <score> · <time>` line, a taller one splits the best across
/// two rows, and a tall card also shows the floor's emblem and boss codename.
/// `rect.height < 3` falls back to a single list row (no box to draw).
fn draw_level_slot(
    buf: &mut Buffer,
    rect: Rect,
    slot: &LevelSlotView,
    is_sel: bool,
    border_col: Color,
    bg: Color,
    accent: Color,
) {
    if rect.width < 4 || rect.height == 0 {
        return;
    }

    if rect.height < 3 {
        // Too short for a box: one list row with the name and its status.
        fill_rect(buf, rect, ' ', ink(COAT).bg(bg));
        put_str(
            buf,
            rect.x,
            rect.y,
            if is_sel { "▸" } else { " " },
            1,
            heavy(accent).bg(bg),
        );
        let mut line = StackBuf::<48>::new();
        let _ = write!(line, "{}  {}", slot.level.name(), slot_status(slot));
        put_str_ellipsis(
            buf,
            rect.x + 1,
            rect.y,
            line.as_str(),
            rect.width.saturating_sub(1),
            if slot.unlocked {
                heavy(COAT).bg(bg)
            } else {
                ink(MUTED).bg(bg)
            },
        );
        return;
    }

    let chip = slot_stage_chip(slot.stage);
    let mut tag = StackBuf::<24>::new();
    let _ = write!(
        tag,
        " {}{} {} ",
        if is_sel { "▸" } else { "·" },
        if slot.unlocked { "" } else { "×" },
        chip.as_str()
    );
    let ci = sub_box(buf, rect, border_col, bg);
    put_str(
        buf,
        rect.x + 1,
        rect.y,
        tag.as_str(),
        rect.width.saturating_sub(2).min(14),
        heavy(if slot.unlocked { accent } else { EDGE }).bg(bg),
    );
    if ci.height == 0 || ci.width < 6 {
        return;
    }

    let h = ci.height;
    // Status, then the floor's name.
    let has_status = h >= 3;
    if has_status {
        put_centered(buf, ci, 0, slot_status(slot), slot_status_style(slot, bg));
    }
    put_centered(
        buf,
        ci,
        u16::from(has_status),
        slot.level.name(),
        if slot.unlocked {
            heavy(COAT).bg(bg)
        } else {
            ink(MUTED).bg(bg)
        },
    );

    // Tall cards keep the old bay cards' emblem.
    if h >= 11 && ci.width >= 19 {
        for (ey, line) in (3u16..).zip(level_emblem(slot.level)) {
            put_centered(
                buf,
                ci,
                ey,
                line,
                ink(if slot.unlocked { accent } else { EDGE }).bg(bg),
            );
        }
    }

    // The floor's boss fight(s), on every card with room for them: a floor with
    // a boss pool draws one of two by seed, so the card names **both** — one
    // line each under the tall cards' emblem, or two rows under the header on a
    // medium card. A card too short for a boss row leaves it to the screen's
    // subtitle line (`draw_level_select`), which names the selected floor.
    let boss_row = if h >= 11 { 7 } else { 2 };
    if h >= 6 {
        if slot.unlocked {
            let (first, second) = slot.level.boss_pair();
            put_centered(buf, ci, boss_row, first, ink(MUTED).bg(bg));
            if let Some(second) = second {
                let mut line = StackBuf::<32>::new();
                let _ = write!(line, "or {second}");
                put_centered(buf, ci, boss_row + 1, line.as_str(), ink(EDGE).bg(bg));
            }
        } else if h >= 11 {
            put_centered(buf, ci, boss_row, "BOSS CLASSIFIED", ink(EDGE).bg(bg));
        }
    }

    // The best result — or, for a locked floor, what opens it.
    if !slot.unlocked {
        put_centered(
            buf,
            ci,
            h.saturating_sub(1),
            slot_unlock_hint(slot),
            ink(MUTED).bg(bg),
        );
        return;
    }
    if h >= 4 {
        if let Some(best) = slot.best {
            let bt = fmt_time(best.best_time_secs);
            let mut r1 = StackBuf::<28>::new();
            let _ = write!(r1, "BEST {}", best.score);
            put_centered(buf, ci, h - 2, r1.as_str(), heavy(TRAIL_GOLD).bg(bg));
            let mut r2 = StackBuf::<24>::new();
            let _ = write!(r2, "TIME {}", bt.as_str());
            put_centered(buf, ci, h - 1, r2.as_str(), ink(MUTED).bg(bg));
        } else {
            put_centered(buf, ci, h - 2, "BEST  —", ink(MUTED).bg(bg));
            put_centered(buf, ci, h - 1, "No record yet", ink(EDGE).bg(bg));
        }
    } else if h >= 2 {
        // Two content rows: the name and one combined best line.
        if let Some(best) = slot.best {
            let bt = fmt_time(best.best_time_secs);
            let mut line = StackBuf::<40>::new();
            let _ = write!(line, "BEST {} · {}", best.score, bt.as_str());
            put_centered(buf, ci, h - 1, line.as_str(), heavy(TRAIL_GOLD).bg(bg));
        } else {
            put_centered(buf, ci, h - 1, "BEST  —", ink(MUTED).bg(bg));
        }
    }
}

/// Draw the Campaign Level Select screen: **every** playable floor as its own
/// slot, in route order (stage one, the two stage-two alternatives, the two
/// stage-three alternatives, then the getaway ride).
///
/// The layout is a grid — three columns on a wide terminal, two on a medium
/// one, one on a narrow one (see [`level_select_cols`]) — and a window that
/// scrolls with the selection when the slots do not all fit. Every visible slot
/// emits [`LevelSelectAction::SelectLevel`] (locked floors included, so they
/// can be selected and read); [`LevelSelectAction::Start`] is emitted only when
/// the selected floor is unlocked.
pub fn draw_level_select(
    f: &mut Frame,
    area: Rect,
    view: &LevelSelectView<'_>,
) -> HitList<LevelSelectAction, 16> {
    draw_level_select_with(f, area, view, &PlateKeys::DEFAULT)
}

/// [`draw_level_select`] with the confirm key its start button and legend
/// print (the back key is already `view.back_key`).
pub fn draw_level_select_with(
    f: &mut Frame,
    area: Rect,
    view: &LevelSelectView<'_>,
    keys: &PlateKeys<'_>,
) -> HitList<LevelSelectAction, 16> {
    let mut hits = HitList::new();
    if area.width < 16 || area.height < 8 || view.slots.is_empty() {
        return hits;
    }

    // Match by floor identity, so a caller that reorders the list still
    // highlights (and starts) the same floor.
    let sel_idx = view
        .slots
        .iter()
        .position(|s| s.floor == view.selected)
        .unwrap_or(0);
    let sel_slot = &view.slots[sel_idx];
    let sel_accent = if sel_slot.unlocked {
        sel_slot.level.accent()
    } else {
        MUTED
    };

    let card = centered_rect(area.width.min(92), area.height.min(34), area);
    let cleared_n = view
        .slots
        .iter()
        .filter(|s| s.best.is_some_and(|b| b.cleared))
        .count();

    let mut cap = StackBuf::<48>::new();
    if card.width >= 44 {
        let _ = write!(cap, " CAMPAIGN   /   SELECT SECTOR ");
    } else {
        let _ = write!(cap, " SELECT SECTOR ");
    }
    let mut stamp = StackBuf::<24>::new();
    let _ = write!(stamp, " {}/{} CLEARED ", cleared_n, view.slots.len());

    let buf = f.buffer_mut();
    let inner = panel(buf, card, cap.as_str(), Some(stamp.as_str()), sel_accent);
    if inner.width < 12 || inner.height < 5 {
        return hits;
    }

    // Row budget: an optional subtitle (it explains the unlock rule), the grid,
    // then the button row and the key hints.
    let subtitle = inner.height >= 12;
    let show_hints = inner.height >= 11;
    let hint_row = inner.height.saturating_sub(1);
    let btn_row = inner.height - 1 - u16::from(show_hints);
    let list_top = u16::from(subtitle);
    let avail_h = btn_row.saturating_sub(list_top);
    if avail_h < 2 {
        return hits;
    }
    // Grid geometry. Everything fits when each row can keep two lines; a
    // taller terminal spreads the cards instead. When it does not fit, the
    // window follows the selection (a seventh floor would scroll into view).
    let cols = level_select_cols(area);
    let rows_all = view.slots.len().div_ceil(cols);
    let (visible_rows, card_h) = if rows_all as u16 * 2 <= avail_h {
        let h = (avail_h / rows_all as u16).clamp(2, 16);
        (rows_all.min(usize::from(avail_h / h)), h)
    } else {
        let h = 2u16;
        (((avail_h / h).max(1) as usize).min(rows_all), h)
    };
    let gap = 1u16;
    let grid_w = inner.width.saturating_sub(2);
    let gap_total = gap * (cols as u16 - 1);
    let card_w = (grid_w.saturating_sub(gap_total) / cols as u16)
        .max(8)
        .min(grid_w);
    let used_w = card_w * cols as u16 + gap_total;
    let grid_x = inner.x + 1 + grid_w.saturating_sub(used_w) / 2;
    // Row 0: the unlock rule when the cards are tall enough to name the boss
    // fight themselves, and the **selected floor's** boss line when they are
    // not (the 57x20 grid leaves three content rows, so a card has no room —
    // but the screen still has to say who is waiting on that floor).
    if subtitle {
        if card_h >= 8 {
            let line = if inner.width >= 70 {
                "Clear a sector to open the next · Flatbed always open"
            } else if inner.width >= 52 {
                "Clear a sector to open the next · Flatbed open"
            } else {
                "Clear a sector to open the next"
            };
            put_centered(buf, inner, 0, line, ink(MUTED).bg(PLATE));
        } else if sel_slot.unlocked {
            let (first, second) = sel_slot.level.boss_pair();
            let mut line = StackBuf::<48>::new();
            match second {
                Some(second) => {
                    let _ = write!(line, "BOSS  {first} or {second}");
                }
                None => {
                    let _ = write!(line, "BOSS  {first}");
                }
            }
            put_centered(buf, inner, 0, line.as_str(), ink(MUTED).bg(PLATE));
        } else {
            put_centered(buf, inner, 0, "BOSS CLASSIFIED", ink(EDGE).bg(PLATE));
        }
    }
    let sel_row = sel_idx / cols;
    let first_row = if visible_rows >= rows_all {
        0
    } else {
        sel_row
            .saturating_sub(visible_rows - 1)
            .min(rows_all - visible_rows)
    };

    for (i, slot) in view.slots.iter().enumerate() {
        let row = i / cols;
        if row < first_row || row >= first_row + visible_rows {
            continue;
        }
        let col = i % cols;
        let sx = grid_x + col as u16 * (card_w + gap);
        let sy = inner.y + list_top + (row - first_row) as u16 * card_h;
        let srect = Rect::new(sx, sy, card_w, card_h);
        let is_sel = i == sel_idx;
        let is_hov = view.hovered == Some(LevelSelectAction::SelectLevel(slot.floor));
        let accent = slot.level.accent();
        let (border_col, bg) = if !slot.unlocked {
            (TRACK, PLATE_DIM)
        } else if is_hov {
            (TAIL, PLATE_HILITE)
        } else if is_sel {
            (accent, PLATE_SUB)
        } else {
            (EDGE, PLATE)
        };
        draw_level_slot(buf, srect, slot, is_sel, border_col, bg, accent);
        hits.push(
            clamp_rect(srect, area),
            LevelSelectAction::SelectLevel(slot.floor),
        );
    }

    // Bottom action bar: Start + Back.
    let can_start = sel_slot.unlocked;
    let mut start = StackBuf::<64>::new();
    let _ = if !can_start {
        write!(start, " SECTOR LOCKED ")
    } else if inner.width >= 48 {
        write!(start, " ▸  {}  START SECTOR ", keys.confirm)
    } else {
        write!(start, " ▸ {} START ", keys.confirm)
    };
    let start_lbl = start.as_str();
    let back_lbl = format!(" {}  back ", view.back_key);
    let back_lbl = back_lbl.as_str();
    let btn_gap = 3u16;
    let sw = start_lbl.chars().count() as u16;
    let bw = back_lbl.chars().count() as u16;
    let total = (sw + btn_gap + bw).min(inner.width);
    let bx = inner.x + (inner.width - total) / 2;

    let start_state = if !can_start {
        ButtonState::Disabled
    } else if view.hovered == Some(LevelSelectAction::Start) {
        ButtonState::Hovered
    } else {
        ButtonState::Primary
    };
    let start_r = draw_button_at(
        buf,
        bx,
        inner.y + btn_row,
        start_lbl,
        sw.min(inner.right().saturating_sub(bx)),
        start_state,
        sel_accent,
    );
    if can_start {
        hits.push(start_r, LevelSelectAction::Start);
    }

    let back_x = bx + sw + btn_gap;
    if back_x < inner.right() {
        let back_state = if view.hovered == Some(LevelSelectAction::Back) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        };
        let back_r = draw_button_at(
            buf,
            back_x,
            inner.y + btn_row,
            back_lbl,
            bw.min(inner.right().saturating_sub(back_x)),
            back_state,
            sel_accent,
        );
        hits.push(back_r, LevelSelectAction::Back);
    }

    if show_hints {
        put_centered(
            buf,
            inner,
            hint_row,
            &format!(
                "Arrows / click select  ·  {} begin  ·  {} back",
                keys.confirm, view.back_key
            ),
            ink(MUTED).bg(PLATE),
        );
    }

    hits
}

// ---------------------------------------------------------------------------
// 6. Compact Campaign Progress Indicator (HUD / Pause)
// ---------------------------------------------------------------------------

/// Draw a compact campaign progress indicator (`LV 1/4 ● ◆ ○ ○ SEWER`) into `area`.
///
/// Adapts gracefully across widths:
/// - `>= 24`: `LV 2/4  ● ◆ ○ ○  Kitchen`
/// - `>= 15`: `LV 2/4  ● ◆ ○ ○`
/// - `>= 9`:  `2/4 ●◆○○`
/// - `>= 7`:  `● ◆ ○ ○`
/// - `>= 4`:  `●◆○○`
///
/// Returns the exact bounding [`Rect`] drawn inside `area`.
pub fn draw_campaign_pips(
    f: &mut Frame,
    area: Rect,
    view: &CampaignProgressView,
) -> Rect {
    let r = area.intersection(f.buffer_mut().area);
    if r.is_empty() {
        return Rect::default();
    }

    let bg = view.bg.unwrap_or(PLATE);
    let buf = f.buffer_mut();
    let y = r.y;
    let mut cx = r.x;
    let accent = view.current.accent();

    if r.width >= 15 {
        cx = put_str(buf, cx, y, "LV ", 3, ink(MUTED).bg(bg));
        let mut nb = StackBuf::<8>::new();
        let _ = write!(
            nb,
            "{}/{}  ",
            view.current.stage_number(),
            crate::game::campaign::LEVEL_COUNT
        );
        cx = put_str(buf, cx, y, nb.as_str(), 5, heavy(COAT).bg(bg));
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, true, bg);
        if r.width >= 24 {
            cx = put_str(buf, cx, y, "  ", 2, ink(MUTED).bg(bg));
            let rem = r.right().saturating_sub(cx);
            cx = put_str(buf, cx, y, view.current.short_name(), rem, heavy(accent).bg(bg));
        }
    } else if r.width >= 9 {
        let mut nb = StackBuf::<8>::new();
        let _ = write!(
            nb,
            "{}/{} ",
            view.current.stage_number(),
            crate::game::campaign::LEVEL_COUNT
        );
        cx = put_str(buf, cx, y, nb.as_str(), 4, heavy(COAT).bg(bg));
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, false, bg);
    } else if r.width >= 7 {
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, true, bg);
    } else {
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, false, bg);
    }

    Rect::new(r.x, y, cx.saturating_sub(r.x), 1)
}

#[allow(clippy::too_many_arguments)]
fn draw_three_pips(
    buf: &mut Buffer,
    mut cx: u16,
    y: u16,
    max_x: u16,
    current: CampaignLevel,
    cleared_count: u8,
    spaced: bool,
    bg: Color,
) -> u16 {
    // Matched by level, like `stamp_header_pips`: a side-route floor is on
    // stage two or three and must light that pip, and the ride lights the
    // fourth.
    let current_stage = usize::from(current.stage_number().saturating_sub(1));
    let pips = CampaignLevel::all().len();
    for idx in 0..pips {
        if cx >= max_x {
            break;
        }
        let (ch, col) = if (idx as u8) < cleared_count {
            ('●', HEAL_MINT)
        } else if idx == current_stage {
            ('◆', current.accent())
        } else {
            ('○', EDGE)
        };
        let mut g = [0u8; 4];
        cx = put_str(buf, cx, y, ch.encode_utf8(&mut g), 1, heavy(col).bg(bg));
        if spaced && idx + 1 < pips && cx < max_x {
            cx = put_str(buf, cx, y, " ", 1, ink(MUTED).bg(bg));
        }
    }
    cx
}

// ---------------------------------------------------------------------------
// Tests & visual capture dumper
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::loadout::PassiveKind;
    use crate::game::weapon::WeaponKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const SIZES: [(u16, u16); 4] = [(57, 20), (80, 24), (120, 40), (200, 60)];
    const TINY_SIZES: [(u16, u16); 7] = [(0, 0), (1, 1), (4, 2), (8, 4), (15, 7), (30, 10), (45, 14)];

    fn sample_upgrades() -> [UpgradeEntry<'static>; 13] {
        [
            UpgradeEntry {
                name: "Gnaw",
                glyph: '▸',
                color: (255, 240, 180),
                category: UpgradeCategory::Weapon { level: 5, max_level: 5 },
                detail: "Homing dart at nearest foe · MAX damage & pierce",
            },
            UpgradeEntry {
                name: "Cheese Spray",
                glyph: '»',
                color: (255, 210, 90),
                category: UpgradeCategory::Weapon { level: 4, max_level: 5 },
                detail: "Forward fan of 5 dairy darts · tight spread",
            },
            UpgradeEntry {
                name: "Squeak Nova",
                glyph: '✦',
                color: (200, 160, 255),
                category: UpgradeCategory::Weapon { level: 3, max_level: 5 },
                detail: "12-shot sonic burst in every direction",
            },
            UpgradeEntry {
                name: "Spore Orbit",
                glyph: '◎',
                color: (140, 255, 180),
                category: UpgradeCategory::Weapon { level: 3, max_level: 5 },
                detail: "3 fungal blades circle and shred nearby foes",
            },
            UpgradeEntry {
                name: "Tail Whip",
                glyph: '◈',
                color: (255, 150, 200),
                category: UpgradeCategory::Weapon { level: 2, max_level: 5 },
                detail: "Melee shockwave pulse around the rat",
            },
            UpgradeEntry {
                name: "Acid Spit",
                glyph: '♦',
                color: (160, 255, 90),
                category: UpgradeCategory::Weapon { level: 2, max_level: 5 },
                detail: "Lobs corrosive bile that pools and burns",
            },
            UpgradeEntry {
                name: "Sharp Teeth",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 3 },
                detail: "+45% weapon damage across all attacks",
            },
            UpgradeEntry {
                name: "Quick Paws",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+24% move speed for kiting & escapes",
            },
            UpgradeEntry {
                name: "Frenzy",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+24% weapon fire rate",
            },
            UpgradeEntry {
                name: "Thick Hide",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+2 flat armor against every hit",
            },
            UpgradeEntry {
                name: "Whiskers",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 1 },
                detail: "+40% XP gem & loot pickup radius",
            },
            UpgradeEntry {
                name: "Fat Reserves",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+40 max HP cushion",
            },
            UpgradeEntry {
                name: "Resilience",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 1 },
                detail: "+0.5 HP/s steady regeneration",
            },
        ]
    }

    fn buffer_text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn assert_single_cell_glyphs(t: &Terminal<TestBackend>) {
        let buf = t.backend().buffer();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let sym = buf[(x, y)].symbol();
                assert_eq!(
                    sym.chars().count(),
                    1,
                    "multi-char cell at ({x},{y}): {sym:?}"
                );
                let ch = sym.chars().next().unwrap();
                assert!(
                    !('\u{1F000}'..='\u{1FFFF}').contains(&ch),
                    "emoji glyph forbidden at ({x},{y}): {ch:?}"
                );
            }
        }
    }

    fn assert_hits_inside_drawn_controls<A: Copy + core::fmt::Debug, const N: usize>(
        t: &Terminal<TestBackend>,
        hits: &HitList<A, N>,
    ) {
        let buf = t.backend().buffer();
        let area = buf.area;
        for (rect, action) in hits.iter() {
            assert!(!rect.is_empty(), "empty hit rect for {action:?}");
            assert!(
                rect.x >= area.x
                    && rect.y >= area.y
                    && rect.right() <= area.right()
                    && rect.bottom() <= area.bottom(),
                "hit rect {rect:?} for {action:?} outside area {area:?}"
            );
            // Verify at least one visible non-space character was painted inside the hit rect.
            let mut has_ink = false;
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    if buf[(x, y)].symbol() != " " {
                        has_ink = true;
                        break;
                    }
                }
            }
            assert!(
                has_ink,
                "hit rect {rect:?} for {action:?} covers only blank space"
            );
            // Center of hit rect must hit-test back to some action.
            let cx = rect.x + rect.width / 2;
            let cy = rect.y + rect.height / 2;
            assert!(
                hits.at(cx, cy).is_some(),
                "center ({cx},{cy}) of {rect:?} did not hit-test"
            );
        }
    }

    /// Every floor the player can clear has a complete plate: the three
    /// stages, both side routes and the getaway ride.
    fn all_floor_plates() -> Vec<CampaignLevel> {
        Theme::playable()
            .into_iter()
            .map(CampaignLevel::from_theme)
            .collect()
    }

    /// The cleared-sector plate promises no pack size it cannot know (the
    /// limit is three to five), and names the next sector by its sector code.
    #[test]
    fn level_complete_names_the_next_sector_and_no_fixed_pack_size() {
        let view = LevelCompleteView {
            level: CampaignLevel::Sewer,
            theme: None,
            time_secs: 252.0,
            kills: 284,
            score_so_far: 6420,
            gold: 24,
            level_score: 3120,
            player_level: 9,
            owned_upgrades: 11,
            next: Some(CampaignLevel::Laundry),
            custom_triumph: None,
            hovered: None,
        };
        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| {
                hits = draw_level_complete(f, f.area(), &view);
            })
            .unwrap();
            assert_hits_inside_drawn_controls(&term, &hits);
            let txt = buffer_text(&term);
            assert!(!txt.contains("PACK 3"), "{w}x{h}: a fixed pack size");
            assert!(!txt.contains("Pack 3 of"), "{w}x{h}: a fixed pack size");
            assert!(!txt.contains("LEVEL "), "{w}x{h}: says LEVEL, not SECTOR");
            assert!(txt.contains("PACK"), "{w}x{h}: the button lost its verb");
            if w >= 120 {
                assert!(
                    txt.contains("SECTOR 02B — THE LAUNDRY"),
                    "{w}x{h}: the next sector is not named"
                );
            }
        }
    }

    #[test]
    fn level_complete_all_bosses_and_sizes() {
        for lvl in all_floor_plates() {
            let view = LevelCompleteView {
                level: lvl,
                theme: None,
                time_secs: 252.0,
                kills: 284,
                score_so_far: 6420,
                gold: 24,
                level_score: 3120,
                player_level: 9,
                owned_upgrades: 11,
                next: lvl.next(),
                custom_triumph: None,
                hovered: None,
            };
            for &(w, h) in &SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = HitList::new();
                term.draw(|f| {
                    hits = draw_level_complete(f, f.area(), &view);
                })
                .unwrap();
                assert_single_cell_glyphs(&term);
                assert_hits_inside_drawn_controls(&term, &hits);
                assert!(
                    hits.find(LevelCompleteAction::Continue).is_some(),
                    "Continue button missing at {w}x{h}"
                );
                let txt = buffer_text(&term);
                assert!(txt.contains(lvl.boss_name()), "boss name missing at {w}x{h}");
                assert!(txt.contains("6420"), "score missing at {w}x{h}");
            }
            for &(w, h) in &TINY_SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| {
                    let _ = draw_level_complete(f, f.area(), &view);
                })
                .unwrap();
            }
        }
    }

    /// A five-upgrade pack (the nest's carry track) shows all five slots: the
    /// previews used to stop at three. The route chips keep their rectangles
    /// and are never painted over by the header's stage pips.
    #[test]
    fn keep_three_shows_a_five_slot_pack_and_whole_route_chips() {
        let items = sample_upgrades();
        let mut sel = [false; 13];
        for flag in sel.iter_mut().take(5) {
            *flag = true;
        }
        let view = KeepThreeView {
            from_level: CampaignLevel::Sewer,
            next_level: CampaignLevel::Kitchen,
            gold: 24,
            items: &items,
            selected: &sel,
            max_keep: 5,
            cursor: 0,
            scroll: 0,
            hovered: None,
            route: [
                Some(RouteChip { name: "Kitchen", chosen: true }),
                Some(RouteChip { name: "Laundry", chosen: false }),
            ],
            splices: &[],
        };
        assert!(keep_three_can_confirm(&view));
        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| {
                hits = draw_keep_three(f, f.area(), &view);
            })
            .unwrap();
            assert_single_cell_glyphs(&term);
            let txt = buffer_text(&term);
            assert!(hits.find(KeepThreeAction::Confirm).is_some(), "{w}x{h}");
            if w >= 120 {
                assert!(txt.contains("SLOT 05"), "{w}x{h}: the fifth slot card is missing");
            } else {
                // The fifth packed upgrade (Tail Whip) has a pill of its own.
                assert!(txt.contains("[◈ Tail"), "{w}x{h}: the fifth pill is missing");
            }
            for (i, name) in ["[Kitchen]", "[Laundry]"].into_iter().enumerate() {
                assert!(txt.contains(name), "{w}x{h}: route chip {name} is not whole");
                let rect = hits
                    .find(KeepThreeAction::Route(i as u8))
                    .unwrap_or_else(|| panic!("{w}x{h}: route chip {i} has no rectangle"));
                assert_eq!(rect.width as usize, name.chars().count(), "{w}x{h}: {name}");
            }
        }
    }

    /// Every plate prints the key names it is handed, and the plain entry
    /// points print the stock ones.
    #[test]
    fn plates_print_the_key_names_they_are_given() {
        let keys = PlateKeys {
            confirm: "F9",
            toggle: "F8",
            route: "F7",
        };
        let items = sample_upgrades();
        let sel = [true, true, true, false, false, false, false, false, false, false, false, false, false];
        let keep = KeepThreeView {
            from_level: CampaignLevel::Sewer,
            next_level: CampaignLevel::Laundry,
            gold: 24,
            items: &items,
            selected: &sel,
            max_keep: 3,
            cursor: 0,
            scroll: 0,
            hovered: None,
            route: [
                Some(RouteChip { name: "Kitchen", chosen: false }),
                Some(RouteChip { name: "Laundry", chosen: true }),
            ],
            splices: &[],
        };
        let intro = LevelIntroView {
            level: CampaignLevel::Laundry,
            flavor: None,
            carried: &[],
            score_so_far: 0,
            gold: 0,
            hovered: None,
        };
        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| {
                let _ = draw_keep_three_with(f, f.area(), &keep, &keys);
            })
            .unwrap();
            let txt = buffer_text(&term);
            assert!(txt.contains("F7 route"), "{w}x{h}: the route key");
            assert!(txt.contains("F9"), "{w}x{h}: the confirm key");
            assert!(!txt.contains("Enter"), "{w}x{h}: a stock key leaked");
            assert!(!txt.contains("Space") && !txt.contains("Tab route"), "{w}x{h}");

            term.draw(|f| {
                let _ = draw_keep_three(f, f.area(), &keep);
            })
            .unwrap();
            assert!(buffer_text(&term).contains("Tab route"), "{w}x{h}: the default");

            term.draw(|f| {
                let _ = draw_level_intro_with(f, f.area(), &intro, &keys);
            })
            .unwrap();
            let txt = buffer_text(&term);
            assert!(txt.contains("F9") && !txt.contains("Enter"), "{w}x{h}: intro");
        }
    }

    /// The cleared-sector plate names the boss the run killed: a Sewer that
    /// fielded the Horseshoe no longer says the Apex Jaguar is down.
    #[test]
    fn level_complete_names_the_boss_that_was_killed() {
        let view = LevelCompleteView {
            level: CampaignLevel::Sewer,
            theme: Some(Theme::Sewer),
            time_secs: 252.0,
            kills: 284,
            score_so_far: 6420,
            gold: 24,
            level_score: 3120,
            player_level: 9,
            owned_upgrades: 11,
            next: Some(CampaignLevel::Kitchen),
            custom_triumph: None,
            hovered: None,
        };
        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| {
                hits = draw_level_complete_with(
                    f,
                    f.area(),
                    &view,
                    &PlateKeys::DEFAULT,
                    Some(BossKind::HorseshoeCrab),
                );
            })
            .unwrap();
            assert_single_cell_glyphs(&term);
            assert_hits_inside_drawn_controls(&term, &hits);
            let txt = buffer_text(&term);
            assert!(txt.contains("THE HORSESHOE"), "{w}x{h}: the killed boss");
            assert!(!txt.contains("APEX JAGUAR"), "{w}x{h}: names the other boss");

            // The floor's original boss keeps the original plate.
            term.draw(|f| {
                let _ = draw_level_complete_with(
                    f,
                    f.area(),
                    &view,
                    &PlateKeys::DEFAULT,
                    Some(BossKind::Cat),
                );
            })
            .unwrap();
            assert!(buffer_text(&term).contains("APEX JAGUAR"), "{w}x{h}");
        }
        // Every pool mate has words of its own, inside the plate's budgets.
        for kind in [
            BossKind::HorseshoeCrab,
            BossKind::MegaRat,
            BossKind::Cobra,
            BossKind::Hawk,
        ] {
            let (verdict, line) = alternate_boss_plate(kind).expect("a pool mate's plate");
            assert!(verdict.chars().count() <= 32, "{kind:?}: {verdict}");
            assert!(line.chars().count() <= 150, "{kind:?}: {line}");
            assert!(!line.contains('!'));
        }
    }

    #[test]
    fn keep_three_selection_states_and_scrolling() {
        let items = sample_upgrades();

        // 0 chosen, 2 chosen, 3 chosen across all 4 target sizes.
        let cases: [(&str, [bool; 13], bool); 3] = [
            ("0/3", [false; 13], false),
            (
                "2/3",
                [true, false, true, false, false, false, false, false, false, false, false, false, false],
                false,
            ),
            (
                "3/3",
                [true, false, true, false, false, false, true, false, false, false, false, false, false],
                true,
            ),
        ];

        for &(w, h) in &SIZES {
            for (tag, sel, expect_confirm) in &cases {
                let view = KeepThreeView {
                    from_level: CampaignLevel::Sewer,
                    next_level: CampaignLevel::Kitchen,
                    gold: 24,
                    items: &items,
                    selected: sel,
                    max_keep: 3,
                    cursor: 2,
                    scroll: 0,
                    hovered: if *expect_confirm {
                        Some(KeepThreeAction::Confirm)
                    } else {
                        Some(KeepThreeAction::ToggleItem(1))
                    },
                    route: [None, None],
                    splices: &[],
                };
                assert_eq!(keep_three_can_confirm(&view), *expect_confirm);

                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = HitList::new();
                term.draw(|f| {
                    hits = draw_keep_three(f, f.area(), &view);
                })
                .unwrap();
                assert_single_cell_glyphs(&term);
                assert_hits_inside_drawn_controls(&term, &hits);

                let has_confirm = hits.find(KeepThreeAction::Confirm).is_some();
                assert_eq!(
                    has_confirm, *expect_confirm,
                    "confirm hit mismatch for {tag} at {w}x{h}"
                );
                let txt = buffer_text(&term);
                assert!(txt.contains(tag), "count {tag} missing at {w}x{h}");
            }
        }

        // Scroll verification at 80x24 and 57x20 where 13 items overflow visible rows.
        for &(w, h) in &[(57u16, 20u16), (80, 24)] {
            let vis = keep_three_visible_rows(Rect::new(0, 0, w, h));
            assert!(vis < items.len(), "expected scroll needed at {w}x{h}, vis={vis}");

            // At top (scroll=0): ScrollDown present, ScrollUp absent.
            let sel = [false; 13];
            let top_view = KeepThreeView {
                from_level: CampaignLevel::Sewer,
                next_level: CampaignLevel::Kitchen,
                gold: 24,
                items: &items,
                selected: &sel,
                max_keep: 3,
                cursor: 0,
                scroll: 0,
                hovered: None,
                route: [None, None],
                splices: &[],
            };
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| hits = draw_keep_three(f, f.area(), &top_view)).unwrap();
            assert!(hits.find(KeepThreeAction::ScrollUp).is_none());
            assert!(hits.find(KeepThreeAction::ScrollDown).is_some());
            assert!(hits.find(KeepThreeAction::ToggleItem(0)).is_some());
            assert!(hits.find(KeepThreeAction::ToggleItem(items.len() - 1)).is_none());

            // At bottom (scroll=99 clamped): ScrollUp present, ScrollDown absent, last item visible.
            let bot_view = KeepThreeView {
                scroll: 99,
                cursor: items.len() - 1,
                ..top_view
            };
            term.draw(|f| hits = draw_keep_three(f, f.area(), &bot_view)).unwrap();
            assert_hits_inside_drawn_controls(&term, &hits);
            assert!(hits.find(KeepThreeAction::ScrollUp).is_some());
            assert!(hits.find(KeepThreeAction::ScrollDown).is_none());
            assert!(hits.find(KeepThreeAction::ToggleItem(items.len() - 1)).is_some());
        }

        // Fewer than 3 owned upgrades: confirm enabled when all owned are selected.
        let few = &items[..2];
        let sel_two = [true, true];
        let few_view = KeepThreeView {
            from_level: CampaignLevel::Sewer,
            next_level: CampaignLevel::Kitchen,
            gold: 24,
            items: few,
            selected: &sel_two,
            max_keep: 3,
            cursor: 0,
            scroll: 0,
            hovered: None,
            route: [None, None],
            splices: &[],
        };
        assert!(keep_three_can_confirm(&few_view));
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut hits = HitList::new();
        term.draw(|f| hits = draw_keep_three(f, f.area(), &few_view)).unwrap();
        assert!(hits.find(KeepThreeAction::Confirm).is_some());
        assert!(buffer_text(&term).contains("2/2"));

        // Zero owned upgrades: confirm immediately enabled without panic.
        let empty_view = KeepThreeView {
            items: &[],
            selected: &[],
            ..few_view
        };
        assert!(keep_three_can_confirm(&empty_view));
        term.draw(|f| hits = draw_keep_three(f, f.area(), &empty_view)).unwrap();
        assert!(hits.find(KeepThreeAction::Confirm).is_some());

        // Tiny sizes never panic.
        for &(w, h) in &TINY_SIZES {
            let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
            small
                .draw(|f| {
                    let _ = draw_keep_three(f, f.area(), &few_view);
                })
                .unwrap();
        }

        // Large list (18 entries) forces scroll even in wide two-pane layouts (120x40, 200x60).
        let mut many = Vec::with_capacity(18);
        many.extend_from_slice(&items);
        many.extend_from_slice(&items[..5]);
        let many_sel = [false; 18];
        for &(w, h) in &[(120u16, 40u16), (200, 60)] {
            let vis = keep_three_visible_rows(Rect::new(0, 0, w, h));
            assert!(vis < many.len(), "expected wide scroll at {w}x{h}, vis={vis}");
            let mut wide_term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            let wide_view = KeepThreeView {
                from_level: CampaignLevel::Kitchen,
                next_level: CampaignLevel::Lab,
                gold: 24,
                items: &many,
                selected: &many_sel,
                max_keep: 3,
                cursor: 16,
                scroll: 2,
                hovered: Some(KeepThreeAction::ScrollUp),
                route: [None, None],
                splices: &[],
            };
            wide_term
                .draw(|f| hits = draw_keep_three(f, f.area(), &wide_view))
                .unwrap();
            assert_single_cell_glyphs(&wide_term);
            assert_hits_inside_drawn_controls(&wide_term, &hits);
            assert!(hits.find(KeepThreeAction::ScrollUp).is_some());
            assert!(hits.find(KeepThreeAction::ScrollDown).is_some());
        }
    }

    #[test]
    fn level_intro_levels_2_and_3_all_sizes() {
        let items = sample_upgrades();
        let carried = [items[0], items[2], items[6]];

        for lvl in [CampaignLevel::Kitchen, CampaignLevel::Lab] {
            let view = LevelIntroView {
                level: lvl,
                flavor: None,
                carried: &carried,
                score_so_far: 6420,
                gold: 24,
                hovered: None,
            };
            for &(w, h) in &SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = HitList::new();
                term.draw(|f| hits = draw_level_intro(f, f.area(), &view)).unwrap();
                assert_single_cell_glyphs(&term);
                assert_hits_inside_drawn_controls(&term, &hits);
                assert!(hits.find(LevelIntroAction::Begin).is_some());
                let txt = buffer_text(&term);
                assert!(txt.contains(lvl.name()), "level name missing at {w}x{h}");
                assert!(txt.contains("Gnaw"), "carried upgrade missing at {w}x{h}");
                if w == 57 && h == 20 {
                    assert!(
                        txt.contains("●●●●●"),
                        "max-level 5-pip string truncated at 57x20"
                    );
                }
            }
            for &(w, h) in &TINY_SIZES {
                let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
                small
                    .draw(|f| {
                        let _ = draw_level_intro(f, f.area(), &view);
                    })
                    .unwrap();
            }
        }
    }

    /// The nest's carry track lets a run keep five upgrades: the intro plate
    /// names all five (it used to stop at three), at every reference size.
    #[test]
    fn level_intro_lists_a_five_upgrade_pack() {
        let items = sample_upgrades();
        let carried = [items[0], items[1], items[2], items[3], items[6]];
        let view = LevelIntroView {
            level: CampaignLevel::Printworks,
            flavor: None,
            carried: &carried,
            score_so_far: 6420,
            gold: 24,
            hovered: None,
        };
        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| hits = draw_level_intro(f, f.area(), &view)).unwrap();
            assert_single_cell_glyphs(&term);
            assert_hits_inside_drawn_controls(&term, &hits);
            let txt = buffer_text(&term);
            for it in &carried {
                // A pill shows the first word (six cells at most), a row or a
                // card the whole name: the first word's head is on all three.
                let head: String = it
                    .name
                    .split(' ')
                    .next()
                    .unwrap_or(it.name)
                    .chars()
                    .take(6)
                    .collect();
                assert!(
                    txt.contains(&head),
                    "{w}x{h}: carried upgrade {} is not on the plate",
                    it.name
                );
            }
        }
    }

    /// The intro plate says "Sector" like every other screen (STYLE.md §3),
    /// with the same code the level 1 card prints, and the default note fits
    /// its three rows whole on every floor.
    #[test]
    fn level_intro_heading_reads_sector_and_the_note_is_not_cut() {
        for lvl in all_floor_plates() {
            let view = LevelIntroView {
                level: lvl,
                flavor: None,
                carried: &[],
                score_so_far: 0,
                gold: 0,
                hovered: None,
            };
            assert!(
                lvl.default_intro_flavor().chars().count() <= 145,
                "{lvl:?}: the default note is over the 145-cell budget"
            );
            for &(w, h) in &SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| {
                    let _ = draw_level_intro(f, f.area(), &view);
                })
                .unwrap();
                let txt = buffer_text(&term);
                assert!(!txt.contains("LEVEL "), "{lvl:?} {w}x{h}: says LEVEL");
                if lvl != CampaignLevel::Trailer {
                    assert!(
                        txt.contains(lvl.sector_code()),
                        "{lvl:?} {w}x{h}: no sector code"
                    );
                }
                // The note's last word made it onto the plate.
                let last = lvl
                    .default_intro_flavor()
                    .rsplit(' ')
                    .next()
                    .unwrap_or_default();
                assert!(
                    txt.contains(last),
                    "{lvl:?} {w}x{h}: the note was cut before {last:?}"
                );
            }
        }
    }

    #[test]
    fn campaign_victory_all_sizes() {
        let items = sample_upgrades();
        let view = CampaignVictoryView {
            menu_key: "Q",
            final_score: 24860,
            total_time_secs: 764.0,
            total_kills: 942,
            gold: 24,
            new_best: true,
            levels: [
                LevelBreakdown {
                    level: CampaignLevel::Sewer,
                    time_secs: 252.0,
                    kills: 284,
                    score: 6420,
                    player_level: 9,
                    boss: None,
                },
                LevelBreakdown {
                    level: CampaignLevel::Kitchen,
                    time_secs: 261.0,
                    kills: 318,
                    score: 8140,
                    player_level: 10,
                    boss: None,
                },
                LevelBreakdown {
                    level: CampaignLevel::Lab,
                    time_secs: 251.0,
                    kills: 340,
                    score: 10300,
                    player_level: 11,
                    boss: None,
                },
                LevelBreakdown {
                    level: CampaignLevel::Trailer,
                    time_secs: 268.0,
                    kills: 355,
                    score: 11200,
                    player_level: 12,
                    boss: None,
                },
            ],
            final_loadout: &items[..4],
            custom_blurb: None,
            focus: CampaignVictoryAction::PlayAgain,
            hovered: None,
        };

        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| hits = draw_campaign_victory(f, f.area(), &view))
                .unwrap();
            assert_single_cell_glyphs(&term);
            assert_hits_inside_drawn_controls(&term, &hits);
            assert!(hits.find(CampaignVictoryAction::PlayAgain).is_some());
            assert!(hits.find(CampaignVictoryAction::Menu).is_some());
            let txt = buffer_text(&term);
            assert!(txt.contains("THE RAT IS OUT"));
            assert!(txt.contains("24860"));
            assert!(txt.contains("Sewer"));
            assert!(txt.contains("Kitchen"));
            assert!(txt.contains("Lab"));
            // The plate names four floors, and the ride is the last of them.
            assert_eq!(view.levels.len(), 4);
            assert!(
                txt.contains("Flatbed") || txt.contains("04 Flatbed"),
                "{w}x{h}: the victory plate does not list the ride:\n{txt}"
            );
        }

        for &(w, h) in &TINY_SIZES {
            let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
            small
                .draw(|f| {
                    let _ = draw_campaign_victory(f, f.area(), &view);
                })
                .unwrap();
        }
    }

    /// Six sample slots, one per playable floor, in the art's route order:
    /// the Sewer cleared, the Kitchen open, the side routes and the ride
    /// locked (the ride is stage four, so it follows the same reach rule).
    fn sample_slots() -> Vec<LevelSlotView> {
        Theme::playable()
            .into_iter()
            .enumerate()
            .map(|(floor, theme)| {
                let level = CampaignLevel::from_theme(theme);
                let stage = crate::game::campaign::theme_stage(theme);
                LevelSlotView {
                    floor,
                    level,
                    stage,
                    unlocked: theme == Theme::Sewer
                        || theme == Theme::Kitchen
                        || theme == Theme::Trailer,
                    best: match theme {
                        Theme::Sewer => Some(LevelBest {
                            score: 6420,
                            best_time_secs: 252.0,
                            cleared: true,
                        }),
                        Theme::Kitchen => Some(LevelBest {
                            score: 4180,
                            best_time_secs: 198.0,
                            cleared: false,
                        }),
                        _ => None,
                    },
                }
            })
            .collect()
    }

    fn draw_select_hits(
        view: &LevelSelectView<'_>,
        w: u16,
        h: u16,
    ) -> (Terminal<TestBackend>, HitList<LevelSelectAction, 16>) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = HitList::new();
        term.draw(|f| hits = draw_level_select(f, f.area(), view))
            .unwrap();
        (term, hits)
    }

    /// One slot per playable floor: the count and the names come from
    /// `Theme::playable()`, so a seventh floor would appear without changing
    /// the screen.
    #[test]
    fn level_select_lists_one_slot_per_playable_floor() {
        let slots = sample_slots();
        assert_eq!(slots.len(), Theme::playable().len());
        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(slot.floor, i, "slots keep the sector row's order");
            assert_eq!(slot.level, CampaignLevel::from_theme(Theme::playable()[i]));
        }
        // The floors on a shared campaign stage are separate slots with their
        // own names (the old three-slot screen showed the Kitchen twice).
        let names: Vec<&str> = slots.iter().map(|s| s.level.short_name()).collect();
        assert_eq!(
            names,
            vec!["Sewer", "Kitchen", "Laundry", "Lab", "Printworks", "Flatbed"]
        );
        assert_eq!(
            slots[2].stage, slots[1].stage,
            "the Kitchen and the Laundry are both stage two"
        );
        assert_eq!(
            slots[5].stage,
            crate::game::campaign::LEVEL_COUNT - 1,
            "the ride is the campaign's fourth level"
        );
        assert!(slots[5].unlocked, "the practice ride is always open");
    }

    #[test]
    fn level_select_locked_unlocked_and_all_sizes() {
        let slots = sample_slots();
        let view = LevelSelectView {
            back_key: "Esc",
            slots: &slots,
            selected: 1,
            hovered: Some(LevelSelectAction::SelectLevel(0)),
        };

        for &(w, h) in &SIZES {
            let (term, hits) = draw_select_hits(&view, w, h);
            assert_single_cell_glyphs(&term);
            assert_hits_inside_drawn_controls(&term, &hits);
            // Every floor is a click target — locked ones included, so they can
            // be selected and read — and each target carries its own floor id.
            for slot in &slots {
                assert!(
                    hits.find(LevelSelectAction::SelectLevel(slot.floor)).is_some(),
                    "floor {} ({:?}) has no click target at {w}x{h}",
                    slot.floor,
                    slot.level
                );
            }
            assert!(hits.find(LevelSelectAction::Start).is_some());
            assert!(hits.find(LevelSelectAction::Back).is_some());
            let txt = buffer_text(&term);
            assert!(txt.contains("CLEARED"), "no cleared status at {w}x{h}");
            assert!(txt.contains("LOCKED"), "no locked status at {w}x{h}");
            assert!(txt.contains("6420"), "no best score at {w}x{h}");
            // The ride's chip is its own level number now, not GETAWAY.
            assert!(txt.contains("S4"), "no S4 chip at {w}x{h}");
            // Every floor is named on the screen: six slots, not three.
            for slot in &slots {
                assert!(
                    txt.contains(slot.level.name()),
                    "{} is missing at {w}x{h}:\n{txt}",
                    slot.level.name()
                );
            }
        }

        // A locked selection cannot start; an unlocked one can.
        let locked = LevelSelectView {
            back_key: "Esc",
            slots: &slots,
            selected: 3, // the Lab, locked in the sample
            hovered: None,
        };
        let (_, hits) = draw_select_hits(&locked, 80, 24);
        assert!(
            hits.find(LevelSelectAction::Start).is_none(),
            "a locked floor must not offer Start"
        );
        let open = LevelSelectView {
            back_key: "Esc",
            slots: &slots,
            selected: 0,
            hovered: None,
        };
        let (_, hits) = draw_select_hits(&open, 80, 24);
        assert!(hits.find(LevelSelectAction::Start).is_some());

        for &(w, h) in &TINY_SIZES {
            let _ = draw_select_hits(&view, w.max(1), h.max(1));
        }
    }

    /// The grid's column count is what the host's `Up`/`Down` step by, so it
    /// is part of the interface.
    #[test]
    fn level_select_grid_columns_follow_the_width() {
        assert_eq!(level_select_cols(Rect::new(0, 0, 120, 40)), 3);
        assert_eq!(level_select_cols(Rect::new(0, 0, 80, 24)), 3);
        assert_eq!(level_select_cols(Rect::new(0, 0, 57, 20)), 2);
        assert_eq!(level_select_cols(Rect::new(0, 0, 30, 12)), 1);
    }

    /// When the slots do not fit, the window follows the selection: the
    /// selected floor is always on screen (and clickable), so a seventh floor
    /// would stay reachable at 57x20.
    #[test]
    fn level_select_scrolls_the_window_to_the_selection() {
        let slots = sample_slots();
        let last = slots.len() - 1;
        let view = LevelSelectView {
            back_key: "Esc",
            slots: &slots,
            selected: last,
            hovered: None,
        };
        // 40x10: one column, three visible rows of a six-row list.
        let (term, hits) = draw_select_hits(&view, 40, 10);
        let txt = buffer_text(&term);
        assert!(
            txt.contains(slots[last].level.name()),
            "the selected floor scrolled out of view:\n{txt}"
        );
        assert!(
            !txt.contains(slots[0].level.name()),
            "the window did not scroll at all:\n{txt}"
        );
        assert!(
            hits.find(LevelSelectAction::SelectLevel(last)).is_some(),
            "the scrolled-in floor must be clickable"
        );
        assert!(hits.find(LevelSelectAction::Start).is_some());
    }

    /// The plates and the boss bar call every fight the same thing, the
    /// getaway ride's included.
    #[test]
    fn every_floor_plate_names_the_same_boss_as_the_bar() {
        for theme in Theme::playable() {
            let plate = CampaignLevel::from_theme(theme);
            assert_eq!(
                crate::render::hud::boss_codename(theme).0,
                plate.boss_name(),
                "{theme:?}: the boss bar and the campaign plate disagree"
            );
            assert!(
                !plate.name().is_empty() && !plate.boss_name().is_empty(),
                "{theme:?}: a plate is missing its copy"
            );
        }
    }

    /// Every floor's card names the fight(s) the seed can field there: the
    /// first name is the floor's own boss (the HUD bar's plate), and a pooled
    /// floor names its pool mate too. The table is read from
    /// `BossKind::pool_for_theme`, so a new pool mate cannot be missed here.
    #[test]
    fn every_floor_card_names_its_whole_boss_pool() {
        use crate::game::ecs::systems::boss::BossKind;
        for theme in Theme::playable() {
            let plate = CampaignLevel::from_theme(theme);
            let pool = BossKind::pool_for_theme(theme);
            let (first, second) = plate.boss_pair();
            assert_eq!(first, plate.boss_name(), "{theme:?}: first name");
            assert_eq!(
                first,
                crate::render::hud::boss_codename_kind(pool[0]).0,
                "{theme:?}: the card and the boss bar disagree"
            );
            assert_eq!(
                second.is_some(),
                pool.len() > 1,
                "{theme:?}: a pooled floor must name both bosses"
            );
            if let (Some(second), Some(kind)) = (second, pool.get(1)) {
                assert_eq!(
                    second,
                    crate::render::hud::boss_codename_kind(*kind).0,
                    "{theme:?}: the pool mate's name"
                );
            }
        }
    }

    /// The cards carry both names of a pooled floor, and at 57x20 — where a card
    /// is three content rows and has no boss row — the selected floor's pair is
    /// named on the screen's own subtitle line instead.
    #[test]
    fn the_sector_cards_name_both_bosses_of_a_pooled_floor() {
        // Every floor open, so no card reads BOSS CLASSIFIED.
        let mut all_open = sample_slots();
        for slot in all_open.iter_mut() {
            slot.unlocked = true;
        }
        for &(w, h) in &[(80u16, 24u16), (120, 40)] {
            let view = LevelSelectView {
                back_key: "Esc",
                slots: &all_open,
                selected: 0,
                hovered: None,
            };
            let (term, _) = draw_select_hits(&view, w, h);
            let txt = buffer_text(&term);
            for slot in &all_open {
                let (first, second) = slot.level.boss_pair();
                assert!(
                    txt.contains(first),
                    "{w}x{h}: {} does not name {first}:\n{txt}",
                    slot.level.name()
                );
                if let Some(second) = second {
                    assert!(
                        txt.contains(second),
                        "{w}x{h}: {} names only one of its two bosses:\n{txt}",
                        slot.level.name()
                    );
                }
            }
        }
        // 57x20: the selected floor's pool is printed above the grid.
        let slots = sample_slots();
        for selected in [0usize, 1, 5] {
            let view = LevelSelectView {
                back_key: "Esc",
                slots: &slots,
                selected,
                hovered: None,
            };
            let (term, _) = draw_select_hits(&view, 57, 20);
            let txt = buffer_text(&term);
            let (first, second) = slots[selected].level.boss_pair();
            assert!(
                txt.contains(first),
                "57x20: {} is missing its first boss {first}:\n{txt}",
                slots[selected].level.name()
            );
            assert!(
                txt.contains(second.unwrap_or("")),
                "57x20: {} does not name its second boss:\n{txt}",
                slots[selected].level.name()
            );
        }
    }

    #[test]
    fn campaign_pips_all_widths_and_levels() {
        for lvl in CampaignLevel::all() {
            let view = CampaignProgressView {
                current: lvl,
                cleared_count: lvl.number().saturating_sub(1),
                bg: None,
            };
            for w in [0u16, 1, 3, 5, 9, 14, 24, 36] {
                let mut term = Terminal::new(TestBackend::new(w.max(1), 1)).unwrap();
                let mut drawn = Rect::default();
                term.draw(|f| {
                    drawn = draw_campaign_pips(f, Rect::new(0, 0, w, 1), &view);
                })
                .unwrap();
                assert!(drawn.width <= w);
                assert_single_cell_glyphs(&term);
                if w >= 9 {
                    let txt = buffer_text(&term);
                    let want = format!("{}/4", lvl.number());
                    assert!(txt.contains(&want), "missing {want} at width {w}: {txt}");
                }
            }
        }
    }

    /// A side-route floor lights its own stage's pip: the Laundry is stage
    /// two and the Printworks stage three, and both used to show no current
    /// pip at all (the pips compared plates, not stages).
    #[test]
    fn side_route_floors_light_their_stage_pip() {
        for (lvl, want) in [
            (CampaignLevel::Sewer, "◆ ○ ○"),
            (CampaignLevel::Laundry, "● ◆ ○"),
            (CampaignLevel::Kitchen, "● ◆ ○"),
            (CampaignLevel::Printworks, "● ● ◆"),
            (CampaignLevel::Lab, "● ● ◆"),
        ] {
            let view = CampaignProgressView {
                current: lvl,
                cleared_count: lvl.stage_number().saturating_sub(1),
                bg: None,
            };
            let mut term = Terminal::new(TestBackend::new(24, 1)).unwrap();
            term.draw(|f| {
                let _ = draw_campaign_pips(f, Rect::new(0, 0, 24, 1), &view);
            })
            .unwrap();
            let txt = buffer_text(&term);
            assert!(txt.contains(want), "{lvl:?}: wanted {want:?} in {txt:?}");

            // The plate header's pips agree (the intro plate at 80x24).
            let intro = LevelIntroView {
                level: lvl,
                flavor: None,
                carried: &[],
                score_so_far: 0,
                gold: 0,
                hovered: None,
            };
            let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
            term.draw(|f| {
                let _ = draw_level_intro(f, f.area(), &intro);
            })
            .unwrap();
            let txt = buffer_text(&term);
            assert!(txt.contains(want), "{lvl:?}: header pips lack {want:?}");
        }
    }

    fn color_rgb(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (19, 24, 27),
        }
    }

    fn dump_terminal(dir: &std::path::Path, name: &str, t: &Terminal<TestBackend>) {
        let buf = t.backend().buffer();
        let w = buf.area.width;
        let h = buf.area.height;
        let mut ansi = String::with_capacity(usize::from(w) * usize::from(h) * 32);
        let mut plain = String::with_capacity(usize::from(w) * usize::from(h) + usize::from(h));
        for y in 0..h {
            for x in 0..w {
                let c = &buf[(x, y)];
                let (r, g, b) = color_rgb(c.fg);
                let (br, bg, bb) = color_rgb(c.bg);
                let _ = write!(
                    ansi,
                    "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                    c.symbol()
                );
                plain.push_str(c.symbol());
            }
            ansi.push_str("\x1b[0m\n");
            plain.push('\n');
        }
        let path = dir.join(format!("{w}x{h}-{name}"));
        std::fs::write(path.with_extension("ansi"), ansi).unwrap();
        std::fs::write(path.with_extension("txt"), plain).unwrap();
    }

    /// Capture `.ansi` and `.txt` frames when `RATRUN_CAMPAIGN_DUMP_DIR` is set.
    #[test]
    fn dump_campaign_screens() {
        let Some(dir) = std::env::var_os("RATRUN_CAMPAIGN_DUMP_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        let items = sample_upgrades();
        let carried = [items[0], items[2], items[6]];
        let sel0 = [false; 13];
        let sel2 = [true, false, true, false, false, false, false, false, false, false, false, false, false];
        let sel3 = [true, false, true, false, false, false, true, false, false, false, false, false, false];

        let victory = CampaignVictoryView {
            menu_key: "Q",
            final_score: 24860,
            total_time_secs: 764.0,
            total_kills: 942,
            gold: 24,
            new_best: true,
            levels: [
                LevelBreakdown {
                    level: CampaignLevel::Sewer,
                    time_secs: 252.0,
                    kills: 284,
                    score: 6420,
                    player_level: 9,
                    boss: None,
                },
                LevelBreakdown {
                    level: CampaignLevel::Kitchen,
                    time_secs: 261.0,
                    kills: 318,
                    score: 8140,
                    player_level: 10,
                    boss: None,
                },
                LevelBreakdown {
                    level: CampaignLevel::Lab,
                    time_secs: 251.0,
                    kills: 340,
                    score: 10300,
                    player_level: 11,
                    boss: None,
                },
                LevelBreakdown {
                    level: CampaignLevel::Trailer,
                    time_secs: 268.0,
                    kills: 355,
                    score: 11200,
                    player_level: 12,
                    boss: None,
                },
            ],
            final_loadout: &items[..4],
            custom_blurb: None,
            focus: CampaignVictoryAction::PlayAgain,
            hovered: None,
        };

        let select_slots = sample_slots();

        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();

            // Level complete for each boss (Sewer at all sizes, Kitchen & Lab at 80x24 & 120x40).
            for lvl in CampaignLevel::all() {
                if lvl != CampaignLevel::Sewer && (w != 80 && w != 120) {
                    continue;
                }
                let lc = LevelCompleteView {
                    level: lvl,
                    theme: None,
                    time_secs: 252.0,
                    kills: 284,
                    score_so_far: 6420,
                    gold: 24,
                    level_score: 3120,
                    player_level: 9,
                    owned_upgrades: 13,
                    next: lvl.next(),
                    custom_triumph: None,
                    hovered: None,
                };
                term.draw(|f| {
                    let _ = draw_level_complete(f, f.area(), &lc);
                })
                .unwrap();
                dump_terminal(dir, &format!("complete-{}", lvl.short_name().to_lowercase()), &term);
            }

            // Keep three: 0/3, 2/3, 3/3, and scrolled.
            for (tag, sel, cursor, scroll, hov) in [
                ("keep-0of3", &sel0, 0, 0, None),
                ("keep-2of3", &sel2, 4, 0, Some(KeepThreeAction::ToggleItem(3))),
                ("keep-3of3", &sel3, 6, 0, Some(KeepThreeAction::Confirm)),
                ("keep-scroll", &sel2, 11, 6, None),
            ] {
                if (tag == "keep-0of3" || tag == "keep-scroll") && w == 200 {
                    continue;
                }
                let k3 = KeepThreeView {
                    from_level: CampaignLevel::Sewer,
                    next_level: CampaignLevel::Kitchen,
                    gold: 24,
                    items: &items,
                    selected: sel,
                    max_keep: 3,
                    cursor,
                    scroll,
                    hovered: hov,
                    route: [None, None],
                    splices: &[],
                };
                term.draw(|f| {
                    let _ = draw_keep_three(f, f.area(), &k3);
                })
                .unwrap();
                dump_terminal(dir, tag, &term);
            }

            // Level intros (Kitchen & Lab).
            for lvl in [CampaignLevel::Kitchen, CampaignLevel::Lab] {
                let intro = LevelIntroView {
                    level: lvl,
                    flavor: None,
                    carried: &carried,
                    score_so_far: 6420,
                    gold: 24,
                    hovered: None,
                };
                term.draw(|f| {
                    let _ = draw_level_intro(f, f.area(), &intro);
                })
                .unwrap();
                dump_terminal(dir, &format!("intro-{}", lvl.short_name().to_lowercase()), &term);
            }

            // Campaign victory.
            term.draw(|f| {
                let _ = draw_campaign_victory(f, f.area(), &victory);
            })
            .unwrap();
            dump_terminal(dir, "victory", &term);

            // Level select, once per floor selected (six captures: every
            // slot's own highlight, name, best and unlock state).
            for selected in 0..select_slots.len() {
                let select = LevelSelectView {
                    back_key: "Esc",
                    slots: &select_slots,
                    selected,
                    hovered: None,
                };
                term.draw(|f| {
                    let _ = draw_level_select(f, f.area(), &select);
                })
                .unwrap();
                dump_terminal(
                    dir,
                    &format!(
                        "select-{selected}-{}",
                        select_slots[selected].level.short_name().to_lowercase()
                    ),
                    &term,
                );
            }
        }

        // Compact pips showcase strip.
        let mut pips_term = Terminal::new(TestBackend::new(60, 7)).unwrap();
        pips_term
            .draw(|f| {
                let area = f.area();
                fill_rect(f.buffer_mut(), area, ' ', ink(COAT).bg(PLATE));
                let rows = [
                    (CampaignLevel::Sewer, 0u8, 28u16),
                    (CampaignLevel::Kitchen, 1, 28),
                    (CampaignLevel::Lab, 2, 28),
                    (CampaignLevel::Kitchen, 1, 14),
                    (CampaignLevel::Kitchen, 1, 9),
                    (CampaignLevel::Lab, 3, 5),
                ];
                for (i, (lvl, clr, w)) in rows.into_iter().enumerate() {
                    let v = CampaignProgressView {
                        current: lvl,
                        cleared_count: clr,
                        bg: Some(PLATE_SUB),
                    };
                    let _ = draw_campaign_pips(f, Rect::new(2, i as u16, w, 1), &v);
                }
            })
            .unwrap();
        dump_terminal(dir, "pips", &pips_term);
    }

    /// The keep-three manifest at the **widest possible rack** (16 owned
    /// upgrades: 8 weapons + 8 passives) plus the gene splices that ride along
    /// without a slot. The list windows with its scrollbar (nothing is cut), the
    /// header counts every entry, and the splices are accounted for on the
    /// narrative row (two-pane / medium) or in the header chip (57×20).
    #[test]
    fn the_manifest_lays_out_a_sixteen_entry_rack_and_the_splices() {
        let mut items: Vec<UpgradeEntry<'static>> = Vec::with_capacity(16);
        for (i, kind) in WeaponKind::all().into_iter().take(8).enumerate() {
            items.push(UpgradeEntry {
                name: kind.name(),
                glyph: crate::render::hud::weapon_glyph(kind),
                color: kind.color(),
                category: UpgradeCategory::Weapon {
                    level: 1 + (i as u8 % kind.max_level()),
                    max_level: kind.max_level(),
                },
                detail: kind.desc(),
            });
        }
        for (i, kind) in PassiveKind::all().into_iter().take(8).enumerate() {
            items.push(UpgradeEntry {
                name: kind.name(),
                glyph: '◆',
                color: (200, 180, 140),
                category: UpgradeCategory::Passive {
                    // Every passive cap is well under a byte.
                    stacks: 1 + (i as u8 % kind.max_stacks() as u8),
                },
                detail: kind.desc(),
            });
        }
        assert_eq!(items.len(), 16);
        let selected: Vec<bool> = vec![false; items.len()];
        let splices: [&'static str; 2] = ["Glass Incisors", "Boiler Heart"];

        for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let view = KeepThreeView {
                from_level: CampaignLevel::Sewer,
                next_level: CampaignLevel::Kitchen,
                gold: 61,
                items: &items,
                selected: &selected,
                max_keep: 3,
                cursor: 0,
                scroll: 0,
                hovered: None,
                route: [None, None],
                splices: &splices,
            };
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            let mut hits = HitList::new();
            terminal
                .draw(|f| hits = draw_keep_three(f, f.area(), &view))
                .unwrap();
            let text = buffer_text(&terminal);
            assert!(
                text.contains("16 OWNED") || text.contains("PACK"),
                "{tw}x{th}: the manifest does not count its entries: {text}"
            );
            // The splices are accounted for at every size.
            if tw >= 120 {
                assert!(text.contains("Splices"), "{tw}x{th}: splices unaccounted: {text}");
            } else if tw >= 80 {
                assert!(text.contains("Spl"), "{tw}x{th}: splices unaccounted: {text}");
            } else {
                assert!(text.contains("SPLICE 2"), "{tw}x{th}: splices unaccounted: {text}");
            }
            // The pack choices still work, and the manifest windows on the list.
            assert!(hits.find(KeepThreeAction::ToggleItem(0)).is_some(), "{tw}x{th}: no rows");
            let vis = keep_three_visible_rows(Rect::new(0, 0, tw, th));
            assert!(vis < items.len(), "{tw}x{th}: expected a windowed list, vis={vis}");
        }
    }
}
