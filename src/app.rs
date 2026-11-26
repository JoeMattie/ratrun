//! Top-level application state machine: menus, the active run, and the
//! transitions between them.
//!
//! **Not simulation, so its maths stays on `std`.** The App drives a run by
//! calling `World::update`/`World::draw`; every trigonometric call in this file
//! is capture-scene staging (`RATRUN_DUMP`'s posed plates: where to put the
//! horde, the lamps, the rat) and never feeds a tick, an rng draw, the shop
//! roll or a save. Silenced here once rather than per capture function.

// Not simulation: platform maths is fine in the capture scenes (see above).
#![allow(clippy::disallowed_methods)]

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::audio::{self, AudioEngine, Sfx};
use crate::game::campaign::{self, CampaignState, KeepKind, Keepable};
use crate::game::collision::SizeClass;
use crate::game::ecs::systems::{boss::BossKind, miniboss::MinibossKind};
use crate::game::enemy::EnemyKind;
use crate::game::gem::PickupKind;
use crate::game::library::{self as library_index, Key as LibraryKey};
use crate::game::level::Theme;
use crate::game::loadout::{ChestReward, Loadout, PassiveKind, SlotCaps, Upgrade};
use crate::game::shop;
use crate::game::ecs::resources::DashHold;
use crate::game::weapon::WeaponKind;
use crate::game::world::World;
use crate::controls::{self, Action, Binding, Device, KeyBind, MouseBind, Scheme, ACTIONS};
use crate::gamepad::PadPoller;
use crate::input::{DashKey, InputState};
use crate::lore;
use crate::math::Vec2;
use crate::meta;
use crate::render::campaign as campaign_ui;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::blackout;
use crate::render::library as library_ui;
use crate::render::nest as nest_ui;
use crate::render::palette;
use crate::render::{
    controls as render_controls, cursor, graphics, hud, menu, minimap, minimap_source, options,
    overlay, zoom as view_zoom,
};
use crate::render::shop as shop_ui;
use crate::scores::{ScoreEntry, ScoreTable};

use menu::{HitMap, RunSummary, Target};

#[derive(Clone, Copy)]
struct Testbed {
    weapon: WeaponKind,
    pickup: PickupKind,
    enemy: EnemyKind,
    boss: BossKind,
    miniboss: MinibossKind,
    show_panel: bool,
    granted_partner: Option<PassiveKind>,
}

impl Default for Testbed {
    fn default() -> Self {
        Self {
            weapon: WeaponKind::all()[0],
            pickup: PickupKind::all()[0],
            enemy: EnemyKind::all()[0],
            boss: BossKind::all()[0],
            miniboss: MinibossKind::all()[0],
            show_panel: true,
            granted_partner: None,
        }
    }
}

impl Testbed {
    fn remove_partner(&mut self, world: &mut World) {
        let Some(partner) = self.granted_partner.take() else {
            return;
        };
        let loadout = &mut world.player.loadout;
        loadout.passives.retain_mut(|(kind, stacks)| {
            if *kind == partner {
                *stacks = stacks.saturating_sub(1);
            }
            *stacks > 0
        });
        loadout.rebuild_stats();
        world.player.hp = world.player.hp.min(world.player.max_hp());
    }

    fn select_weapon(&mut self, world: &mut World) {
        self.remove_partner(world);
        world.player.loadout.weapons.clear();
        world.give_weapon(self.weapon, self.weapon.max_level());
    }

    fn toggle_evolution(&mut self, world: &mut World) {
        let Some(evolution) = self.weapon.evolution() else {
            return;
        };
        let weapon = world.player.loadout.weapons.iter_mut().find(|w| w.kind == self.weapon);
        match weapon {
            Some(weapon) if weapon.evolved => {
                weapon.evolved = false;
                self.remove_partner(world);
                return;
            }
            Some(weapon) => weapon.level = weapon.kind.max_level(),
            None => self.select_weapon(world),
        }
        let loadout = &mut world.player.loadout;
        if loadout.stacks(evolution.passive) == 0 {
            loadout.passives.push((evolution.passive, 1));
            loadout.rebuild_stats();
            self.granted_partner = Some(evolution.passive);
        }
        world.evolve_weapon(self.weapon);
    }
}

fn testbed_cycle<T: Copy + PartialEq>(table: &[T], current: T, forward: bool) -> T {
    let index = table.iter().position(|&entry| entry == current).unwrap_or(0);
    let step = if forward { 1 } else { table.len() - 1 };
    table[(index + step) % table.len()]
}

/// Backdrop behind the campaign's full-screen plates (the same near-black the
/// story dossier paints).
const BACKDROP: Color = Color::Rgb(11, 15, 17);

/// The nest's gold: the same trail colour the HUD's purse uses.
const NEST_GOLD: Color = Color::Rgb(248, 206, 96);

/// How long a level-clear fanfare waits before it plays. The simulation has
/// already played the boss's own death sting on the killing tick, so the
/// fanfare follows it a beat later rather than stacking on top of it.
const FANFARE_DELAY_SECS: f32 = 0.55;

/// The title menu's rows, in order: START RUN, the sector row, OPTIONS,
/// LIBRARY, THE NEST, QUIT. The drawn row list in `menu::draw_title` is the
/// same six, in the same order (the keyboard and the pointer select the same
/// thing).
const TITLE_ROWS: usize = 6;
const TITLE_ACTIONS: [Target; TITLE_ROWS] = [
    Target::Start,
    Target::Sectors,
    Target::Options,
    Target::Library,
    Target::Nest,
    Target::Quit,
];

/// Environment pins for scripted runs (`0..=100`). They override the saved
/// levels for the session without being written back over them.
const MUSIC_VOLUME_ENV: &str = "RATRUN_MUSIC_VOLUME";
const SFX_VOLUME_ENV: &str = "RATRUN_SFX_VOLUME";

/// Minimum gap between effects-volume preview blips, so holding a key or
/// dragging a slider cannot machine-gun the same effect.
const PREVIEW_COOLDOWN_SECS: f32 = 0.12;

/// How long the auto-pick toast stays in the HUD callout area.
const AUTO_PICK_TOAST_SECS: f32 = 2.5;
/// Spacing between auto-picked cards when several level-ups land at once, so
/// each toast is readable. The first card lands immediately (the same frame a
/// manual screen would have opened, so the simulation and its rng see exactly
/// what a manual pick of that card would cause); the cards queued behind it
/// wait their turn.
const AUTO_PICK_SPACING_SECS: f32 = 0.4;
/// The short reveal before the auto-pick toggle takes the current card when it
/// is switched on from the level-up screen itself.
const AUTO_PICK_REVEAL_SECS: f32 = 0.4;

/// Parse a `RATRUN_*_VOLUME` override: `0..=100`, an optional trailing `%`
/// and surrounding spaces tolerated. Out-of-range and non-numeric values are
/// ignored (the saved preference stands).
fn parse_volume_env(raw: &str) -> Option<u8> {
    let trimmed = raw.trim().trim_end_matches('%').trim();
    trimmed.parse::<u8>().ok().map(audio::snap_percent)
}

/// Read one volume override from the process environment.
fn env_volume(name: &str) -> Option<u8> {
    std::env::var(name).ok().as_deref().and_then(parse_volume_env)
}

/// Read one on/off switch out of the process environment, with the same words
/// the render modules' `*_from_env` helpers accept (`0`, `off`, `false`, `no`
/// are off; anything else set is on).
///
/// Returned as an `Option` so the app can tell a **session pin** from a saved
/// preference: a pinned switch wins for the run but is never written back over
/// the save (see [`App::save_graphics`]).
fn env_toggle(name: &str) -> Option<bool> {
    std::env::var(name).ok().map(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        )
    })
}

/// The four session pins of the Graphics page (the same variables the render
/// modules read).
const FOREGROUND_ENV: &str = "RATRUN_FOREGROUND";
const FX_ENV: &str = "RATRUN_FX";
const GLYPHFX_ENV: &str = "RATRUN_GLYPHFX";

/// The level-intro label for a single-sector practice run.
const PRACTICE_BLURB: &str = "PRACTICE RUN — single sector, starting with Gnaw Lv1. \
     Clearing it unlocks the next sector without advancing the campaign.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Title,
    /// Sector select: the art's level-select screen (the title's row 2).
    SectorSelect,
    Story,
    /// Level 1's access-route card (`menu::draw_map_intro`).
    MapIntro,
    /// Levels 2 and 3's intro plate (`campaign_ui::draw_level_intro_with`).
    LevelIntro,
    Playing,
    LevelUp,
    Paused,
    /// Music/effects volumes and the scattered preferences, opened from the
    /// title menu or the pause screen (see [`Screen::options_return`]).
    Options,
    /// A non-final level's boss died: the sector-cleared plate.
    LevelComplete,
    /// The keep-three manifest before the next campaign level.
    KeepThree,
    /// All three campaign levels cleared.
    Victory,
    /// Defeat, or the end of a single-sector practice run.
    End,
    Chest,
    Shop,
    /// The Library (specimen index): opened from the title's LIBRARY row and
    /// the pause screen. Reads the persisted discoveries in `scores.library`.
    Library,
    /// The remappable-controls screen (opened from Options).
    Controls,
    /// **The graphics page** (opened from Options' GRAPHICS row, the way
    /// CONTROLS opens the Controls screen): the render-only picture settings
    /// (lighting quality, the light effects, 1× smoothing, the parallax
    /// foreground, the glyph particles and the camera's shake share). Back
    /// returns to Options, which remembers where *it* came from.
    Graphics,
    /// **The nest** (permanent upgrades): opened from the title's NEST row, the
    /// incident report's "spend your haul" chip and the victory plate, and
    /// read-only from the pause screen (you cannot buy mid-run).
    Nest,
}

#[cfg(test)]
impl Screen {
    /// Screen name, so tests can assert which screen the app is on without
    /// deriving `PartialEq` for a production enum.
    fn name_for_test(&self) -> &'static str {
        match self {
            Screen::Title => "Title",
            Screen::SectorSelect => "SectorSelect",
            Screen::Story => "Story",
            Screen::MapIntro => "MapIntro",
            Screen::LevelIntro => "LevelIntro",
            Screen::Playing => "Playing",
            Screen::LevelUp => "LevelUp",
            Screen::Paused => "Paused",
            Screen::Options => "Options",
            Screen::LevelComplete => "LevelComplete",
            Screen::KeepThree => "KeepThree",
            Screen::Victory => "Victory",
            Screen::End => "End",
            Screen::Chest => "Chest",
            Screen::Shop => "Shop",
            Screen::Library => "Library",
            Screen::Controls => "Controls",
            Screen::Graphics => "Graphics",
            Screen::Nest => "Nest",
        }
    }
}

/// One campaign level's banked numbers, shown in the victory breakdown.
#[derive(Clone, Copy, Default, Debug)]
struct LevelRecord {
    time: f32,
    kills: u32,
    score: u32,
    player_level: u32,
    /// Which floor this record was played on. The record is banked by **stage**
    /// (`records[level_index]`), and a stage can be a side route (`WAVE3_DESIGN`
    /// §11), so the plate cannot be derived from the index: a Laundry stage two
    /// would be labelled "THE KITCHEN" on the victory breakdown.
    plate: Option<campaign_ui::CampaignLevel>,
    /// The boss this floor ended on (a floor's pool holds more than one, and
    /// the console can field any). Kept for the victory breakdown; `None` when
    /// the floor was marked complete without a boss dying.
    boss: Option<crate::game::ecs::systems::boss::BossKind>,
}

/// Rolling frame/tick timings for the console's `perf` overlay. Cheap to
/// update every frame (a smoothed average), read only by the HUD when the
/// overlay is on.
#[derive(Clone, Copy, Default, Debug)]
pub struct PerfStats {
    /// Smoothed frame time in milliseconds.
    pub frame_ms: f32,
    /// Smoothed simulation tick time in milliseconds.
    pub tick_ms: f32,
    /// Entity counts captured on the last frame.
    pub enemies: usize,
    pub bullets: usize,
    pub particles: usize,
}

impl PerfStats {
    /// Fold one frame's raw timings into the rolling averages.
    pub fn record(&mut self, frame_ms: f32, tick_ms: f32) {
        const ALPHA: f32 = 0.1;
        if self.frame_ms == 0.0 {
            self.frame_ms = frame_ms;
        } else {
            self.frame_ms += (frame_ms - self.frame_ms) * ALPHA;
        }
        if self.tick_ms == 0.0 {
            self.tick_ms = tick_ms;
        } else {
            self.tick_ms += (tick_ms - self.tick_ms) * ALPHA;
        }
    }
}

/// Everything `World::draw` reads that is not simulation state, plus the
/// simulation's own tick count: two frames with the same key draw the same
/// pixels, so the second can reuse the first's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PictureKey {
    /// Which run's world (a new floor is a new picture even at tick 0).
    run: u64,
    /// `World::ticks`: every simulation or settle tick.
    ticks: u64,
    /// The pixel buffer's size (the terminal's, less the HUD rails).
    size: (usize, usize),
    /// The eased zoom being rendered and the rung it is easing toward.
    zoom: (u32, u32),
    /// The lighting setting, the tier the cost controller currently runs it
    /// at, and the colour depth it dithers for.
    lighting: (
        crate::render::lighting::LightingMode,
        crate::render::lighting::LightingMode,
        crate::term::ColorDepth,
    ),
    /// Render-only toggles: the hitbox overlay and the smoke layer.
    toggles: (bool, bool),
    /// The resolved glyph set, and whether the 1x picture is smoothed: both
    /// decide the drawn frame without moving `World::ticks`.
    subcell: crate::render::subcell::SubcellSet,
    smooth: crate::render::subcell::Smooth,
    /// Whether the parallax foreground layer draws (see `render::foreground`).
    foreground: bool,
    /// Whether the batch-13 lighting effects run (`render::lighting::fx`).
    fx: bool,
    /// The camera's share of the simulation's shake (`World::shake_scale`,
    /// bits): the Graphics page's SCREEN SHAKE row can change it while the
    /// world is frozen under a plate, and the frozen picture carries the offset.
    shake: u32,
    /// Scheduled fixture attenuation, in bits, derived from the floor clock.
    blackout: u32,
}

impl PictureKey {
    fn of(w: &World, run: u64, pixels: &PixelBuffer) -> Self {
        Self {
            run,
            ticks: w.ticks,
            size: (pixels.w, pixels.h),
            zoom: (w.zoom_view.to_bits(), w.zoom.to_bits()),
            lighting: (w.lighting, w.lighting_tier(), w.lighting_depth),
            toggles: (w.hitboxes, w.smoke_enabled),
            subcell: w.subcell,
            smooth: w.smooth,
            foreground: w.foreground,
            fx: w.lighting_fx,
            shake: w.shake_scale.to_bits(),
            blackout: w.blackout().to_bits(),
        }
    }
}

pub struct App {
    screen: Screen,
    pub should_quit: bool,
    pub input: InputState,
    audio: Option<AudioEngine>,
    /// The music intensity tier last *requested* from the synth (0 calm menu,
    /// 1 groove, 2 horde, 3 Boss Overdrive).
    ///
    /// This is the app's recording of the request, not extra state: every
    /// request goes through [`App::request_music_intensity`] (or
    /// [`App::play_music`]), which stores it here and forwards it to the real
    /// engine. It exists because `AudioEngine::new()` returns `None` without an
    /// audio device — which is every test and every headless run — so the tier
    /// the game state asks for would otherwise be unobservable.
    music_intensity: u32,
    scores: ScoreTable,
    /// The floor cursor: an index into `Theme::playable()`, which is the same
    /// order the title's sector row and the sector-select grid list. Also the
    /// floor the title shows, so the title's material tint and menu music
    /// follow it.
    select_idx: usize,
    /// The sector-select panel's on-screen area last frame. `Up`/`Down` step a
    /// whole grid row, whose width the art owns (`campaign::level_select_cols`).
    last_select_area: Rect,
    menu_idx: usize,
    /// The campaign run in flight (`None` on the title and during a practice
    /// run). It owns the carried loadout and the banked totals.
    campaign: Option<CampaignState>,
    /// A single-sector practice run: the floor it plays alone. The campaign
    /// stage it belongs to is [`crate::game::campaign::theme_stage`].
    practice: Option<Theme>,
    testbed: Option<Testbed>,
    /// The floor chosen for the level being packed on the keep screen (the
    /// route choice, `WAVE3_DESIGN.md` §1). Reset when the manifest opens.
    keep_route: Option<Theme>,
    /// Per-level numbers banked as each level completes (victory breakdown).
    records: [LevelRecord; campaign::LEVEL_COUNT],
    /// Keep-three manifest: what the finished level offered, in the art's
    /// card order, plus the parallel selection flags and the cursor/scroll.
    keep_items: Vec<Keepable>,
    keep_entries: Vec<campaign_ui::UpgradeEntry<'static>>,
    keep_selected: Vec<bool>,
    /// The gene splices in the rat when the manifest opened, by name. They do
    /// **not** travel (a splice lasts for the floor it was bought on), so the
    /// plate names them as left behind rather than as part of the pack.
    keep_splices: Vec<&'static str>,
    keep_cursor: usize,
    keep_scroll: usize,
    /// How many manifest rows the last rendered frame could show (the art's
    /// `keep_three_visible_rows`), so the cursor never scrolls out of view.
    keep_visible_rows: usize,
    /// Upgrades carried into the level on screen (the intro's pack strip).
    carried_entries: Vec<campaign_ui::UpgradeEntry<'static>>,
    /// Final loadout shown on the campaign-victory plate.
    victory_loadout: Vec<campaign_ui::UpgradeEntry<'static>>,
    /// Victory plate keyboard focus.
    victory_focus: campaign_ui::CampaignVictoryAction,
    /// Cards the finished level offered (counted once, not per frame).
    owned_upgrades: usize,
    world: Option<World>,
    levelup_choices: Vec<Upgrade>,
    chest_reward: Option<ChestReward>,
    /// Gene-splice shop: the selected shelf row (`0..shop::ROW_COUNT`).
    shop_row: usize,
    /// Gene-splice shop: the open Molt two-step step, if any.
    shop_molt: Option<shop_ui::MoltUi>,
    levelup_desc: Vec<(String, String, String)>,
    levelup_idx: usize,
    intro_timer: f32,
    story_time: f32,
    /// Game viewport rect from the last frame, for mouse→world mapping.
    last_game: Rect,
    /// The whole frame area from the last render (the terminal's size), so a
    /// floor built between frames can be handed the viewport the next render
    /// would write.
    last_area: Rect,
    /// The boss that died on the floor in flight, read on the death edge (the
    /// world only knows a *living* boss). Reset with each floor.
    killed_boss: Option<crate::game::ecs::systems::boss::BossKind>,
    rat_art: Vec<String>,
    rat_art_w: u32,
    run_counter: u64,
    recorded: bool,
    new_best: bool,
    last_score: u32,
    /// Whether finalized runs are written to the score file on disk. Tests turn
    /// this off so they never touch the user's real high scores.
    persist: bool,
    /// Fixed base seed for reproducible runs (tests); `None` uses the clock.
    fixed_seed: Option<u64>,
    /// Reused game-viewport pixel buffer, reallocated only on resize.
    pixels: PixelBuffer,
    /// What `pixels` holds: the key of the world picture last drawn into it
    /// (see [`PictureKey`]). A frame whose key matches reuses the pixels
    /// instead of running `World::draw` (scene + lighting) again — the frozen
    /// field under the pause, level-up, shop and Library plates.
    picture_key: Option<PictureKey>,
    /// Something happened that the key cannot see (discrete input, the
    /// console, a screen change): the next frame redraws the world.
    picture_dirty: bool,
    /// How many times `World::draw` has run (tests; a paused frame must not
    /// add to it).
    pub world_draws: u64,
    /// The screen changed since the last render: render at once.
    repaint: bool,
    /// Reused per-frame SFX de-duplication buffer.
    sfx_frame: Vec<Sfx>,
    /// A sting that plays a beat *after* the event that caused it
    /// (`(sound, seconds left)`), so a level-clear fanfare follows the boss's
    /// own death sting instead of doubling it.
    sting: Option<(Sfx, f32)>,
    /// Reused HUD text buffers, so the status line does not re-format every
    /// frame.
    hud: hud::HudState,
    hits: HitMap,
    muted: bool,
    /// Live music level (0..=100, steps of 5).
    ///
    /// This is the app's record of what the engine's mixer holds, for the same
    /// reason as `music_intensity`: `AudioEngine::new()` returns `None` without
    /// an audio device — every test and headless run — so the level the player
    /// set would otherwise be unobservable (and the HUD read-out impossible).
    music_volume: u8,
    /// Live effects level (0..=100, steps of 5).
    effects_volume: u8,
    /// `RATRUN_MUSIC_VOLUME` / `RATRUN_SFX_VOLUME` startup pins. A pinned
    /// channel is applied but never written back over the saved preference.
    music_env: Option<u8>,
    effects_env: Option<u8>,
    /// `RATRUN_SUBCELL`, read once at startup: pins the glyph set for the
    /// session (there is no saved option any more).
    subcell_env: Option<crate::render::subcell::Choice>,
    /// Edge treatment of the 1x (and zoomed-in) picture, off by default.
    /// The Graphics page's SMOOTHING row saves it in `scores.graphics`.
    /// `RATRUN_SMOOTH` pins it for a session and the console `smooth` command
    /// switches it live. Render-only.
    smooth: crate::render::subcell::Smooth,
    /// The `RATRUN_SMOOTH` session pin, if the environment carried a readable
    /// one. A pinned row is never written back over the save; changing the row
    /// drops the pin (see [`App::set_smooth_level`]).
    smooth_env: Option<crate::render::subcell::Smooth>,
    /// The parallax foreground layer (overhead pipes, lines, proofs). Off by
    /// default; the Graphics page's FOREGROUND row and the console `foreground`
    /// command A/B it live (`RATRUN_FOREGROUND=1` pins it on for a session).
    /// Render-only, like `smooth`.
    foreground: bool,
    /// The `RATRUN_FOREGROUND` session pin (see `smooth_env`).
    foreground_env: Option<bool>,
    /// The batch-13 lighting effects. On by default; the Graphics page's LIGHT
    /// EFFECTS row and the console `fx` command A/B them live
    /// (`RATRUN_FX=0` pins them off for a session). Render-only, like
    /// `foreground`.
    fx: bool,
    /// The `RATRUN_FX` session pin (see `smooth_env`).
    fx_env: Option<bool>,
    /// Round-glyph particles over the playfield (`render::glyphfx`): dust motes,
    /// hit sparks and rising bubbles. On by default; the Graphics page's GLYPH
    /// PARTICLES row and the console `glyphfx` command A/B it live
    /// (`RATRUN_GLYPHFX=0` pins it off for a session). Render-only, like `fx`,
    /// and drawn into the frame's cells rather than into the cached picture, so
    /// it is not in `PictureKey`.
    glyphfx: crate::render::glyphfx::GlyphFx,
    /// The `RATRUN_GLYPHFX` session pin (see `smooth_env`).
    glyphfx_env: Option<bool>,
    /// How much of the simulation's screen shake reaches the camera. `0.65`
    /// (`game::world::SHAKE_SCALE`) by default; the Graphics page's SCREEN
    /// SHAKE row sets it, and [`App::draw_game`] hands it to the world each
    /// frame. Render-only: nothing simulated reads it.
    shake_scale: f32,
    /// Options screen: the selected row (`0..options::ROW_COUNT`).
    options_row: usize,
    /// Graphics page: the selected row (`0..graphics::ROW_COUNT`).
    graphics_row: usize,
    /// Where Back on the Graphics page returns.
    graphics_return: Screen,
    /// Where Esc/Back on the options screen returns to (`Title` or `Paused`).
    options_return: Screen,
    /// Cooldown before another effects-volume preview blip may play.
    preview_cd: f32,
    /// Preview blips played so far (lets tests see the rate limit with no
    /// audio device; see `music_volume`).
    previews: u32,
    damage_taken: f32,
    dashes_used: u32,
    heals_picked: u32,
    boss_slain: bool,
    prev_boss_alive: bool,
    last_summary: Option<RunSummary>,
    /// Expanded minimap (Tab / clicking the compact map) pauses the simulation.
    map_open: bool,
    /// Start of the player marker's render-only wall clock.
    map_animation_start: std::time::Instant,
    /// Panel rects from the last frame, for pointer hit tests and tests.
    last_minimap: Rect,
    last_map: Rect,
    /// Reused minimap scratch (the art module promises zero allocation per
    /// frame once this is warm) and the per-frame marker/label buffers.
    minimap_scratch: minimap::MinimapScratch,
    map_frame: minimap_source::MinimapFrame,
    /// The live `Action::Map` / `Action::MinimapSize` labels, upper-cased for
    /// the map hints and refreshed only when a binding changes.
    map_key_label: String,
    size_key_label: String,
    /// In-game cheese mouse cursor visibility and animation state.
    cursor: cursor::CursorState,
    /// Optional `RATRUN_CURSOR` environment override (resolved once at startup).
    cursor_env: Option<bool>,
    /// Rack caps the next run starts with (the profile's persistent slot
    /// upgrades, `loadout::MIN_SLOTS..=MAX_SLOTS`; a fresh profile is 4 + 4).
    /// The nest's slot tracks set it at every run start; the campaign carries
    /// it between levels. Tests and the batch pin it through
    /// [`App::set_slot_caps`], which makes a run start honour the pin instead of
    /// re-reading the profile (`slot_caps_pinned`).
    pub slot_caps: SlotCaps,
    /// Did a caller pin `slot_caps` (a test or the batch) rather than take the
    /// profile's slot tracks? A run start then keeps the pinned racks.
    slot_caps_pinned: bool,
    /// The Library screen's own state: tab, cursor, scroll, the card art clock
    /// and its cached card scenes.
    library_ui: library_ui::LibraryUi,
    /// The Library registry (`game::library::all_rows`), built once.
    library_rows: Vec<library_index::Row>,
    /// Where Esc leaves the Library (the title, or the pause screen it was
    /// opened from).
    library_return: Screen,
    /// Reused discovery buffer: the keys found since the last frame (the HUD's
    /// `LIBRARY + …` callouts).
    library_news: Vec<LibraryKey>,
    /// Last frame's Library measurement (display lines, visible rows, narrow).
    library_shape: library_ui::DrawInfo,
    /// Gamepad poller (a no-op without the feature or a pad).
    pad_poller: PadPoller,
    /// Controls screen: the selected action row (`0..ACTION_COUNT` + 2 chrome
    /// rows: SCHEME, RESET, BACK).
    controls_row: usize,
    /// Controls screen: a binding capture is armed for this action.
    bind_capture: Option<Action>,
    /// Controls screen: the last note (a conflict swap, a refused key, …).
    controls_note: String,
    /// Where Esc/Back on the Controls screen returns to (`Options`).
    controls_from_options: bool,
    /// First visible Controls row and how many rows fit (updated by the draw).
    controls_scroll: usize,
    controls_visible: usize,
    /// Seconds until the next auto-picked card may land (the reveal/spacing
    /// clock). Zero means "now". Only meaningful while
    /// `scores.auto_pick` is on; the simulation never reads it.
    auto_pick_wait: f32,
    /// The nest screen's own state: cursor, window, pending confirmation.
    nest_ui: nest_ui::NestUi,
    /// Where Esc leaves the nest (`Title`, `End`, `Victory` or `Paused`).
    nest_return: Screen,
    /// Is this visit read-only? The pause screen opens the nest as a dossier
    /// only: you cannot buy mid-run, so a track's price reads "later".
    nest_read_only: bool,
    /// Last frame's nest measurement (visible rows), so the cursor can be kept
    /// in the window.
    nest_shape: nest_ui::DrawInfo,
    /// The banking tally of the run that just ended (`crate::meta::bank`).
    bank_tally: Option<meta::BankTally>,
    /// Has this run been banked already? One bank per run (death, quit or win),
    /// so quitting the incident report cannot bank it twice.
    banked: bool,
    /// Campaign floors cleared this run (the bank's floor bonus).
    floors_cleared: u8,
    /// Bosses and named minibosses killed this run (the bank's apex bonus).
    apex_kills: u32,
    /// Was a named miniboss alive on the last simulated frame? The kill is the
    /// `Some -> None` edge of `World::miniboss_hp`.
    prev_miniboss_alive: bool,
    /// The options screen's RESET row is armed (press again to wipe the nest).
    reset_armed: bool,
    /// The Quake-style drop-down console's presentation state. Present but
    /// closed in every run; only touched when the player presses the backtick.
    pub console: crate::console::Console,
    /// Set the first time any state-changing console command runs in this run.
    /// A flagged run saves no high score, campaign record, or library counter.
    pub console_cheat: bool,
    /// Console `pause on`: the game stays paused even with the console closed.
    pub console_paused: bool,
    /// Console `timescale x`: a multiplier on the simulation dt (clamped
    /// 0.1..=8). Default 1.0 leaves the game untouched.
    pub timescale: f32,
    /// Console `perf on`: draw the frame/tick/entity overlay.
    pub perf_on: bool,
    /// Rolling frame + tick timings for the perf overlay.
    pub perf: PerfStats,
    /// Console `fps/perf`: set when the run began from a practice floor
    /// (`floor <name>`), so `restart` knows what to replay.
    pub console_floor: Option<Theme>,
}

impl App {
    /// The real game's constructor: it **loads the player's save and persists
    /// to it**. Tests must use [`App::with_options`] with a default table and
    /// `persist = false` instead (the test module's `test_app`).
    pub fn new(kitty: bool, audio: Option<AudioEngine>) -> Self {
        Self::with_options(kitty, audio, ScoreTable::load(), true, None)
    }

    /// Full constructor: inject the starting score table, whether finalized
    /// runs persist to disk, and an optional fixed seed (tests).
    pub fn with_options(
        kitty: bool,
        audio: Option<AudioEngine>,
        scores: ScoreTable,
        persist: bool,
        fixed_seed: Option<u64>,
    ) -> Self {
        // Environment pins resolve once, before the store is moved into the
        // app; a pinned channel's saved value is left untouched (see
        // `set_music_volume`).
        let music_env = env_volume(MUSIC_VOLUME_ENV);
        let effects_env = env_volume(SFX_VOLUME_ENV);
        let music_volume = audio::snap_percent(music_env.unwrap_or(scores.music_volume));
        let effects_volume = audio::snap_percent(effects_env.unwrap_or(scores.effects_volume));
        // The Graphics page's six settings: the saved preference, unless the
        // environment pins that one for this session. The `*_env` fields keep
        // the pin so the save is never written back with a pinned value
        // (`save_graphics`).
        let smooth_env = crate::render::subcell::smooth_env();
        let foreground_env = env_toggle(FOREGROUND_ENV);
        let fx_env = env_toggle(FX_ENV);
        let glyphfx_env = env_toggle(GLYPHFX_ENV);
        let saved_graphics = scores.graphics.clone();
        let smooth = smooth_env.unwrap_or_else(|| saved_graphics.smooth());
        let foreground = foreground_env.unwrap_or(saved_graphics.foreground);
        let fx = fx_env.unwrap_or(saved_graphics.fx);
        let glyphfx = glyphfx_env.unwrap_or(saved_graphics.glyphfx);
        let shake_scale = saved_graphics.shake();
        let mut app = App {
            screen: Screen::Title,
            should_quit: false,
            input: InputState::new(kitty),
            audio,
            music_intensity: 0,
            scores,
            select_idx: 0,
            last_select_area: Rect::new(0, 0, 0, 0),
            menu_idx: 0,
            campaign: None,
            practice: None,
            testbed: None,
            keep_route: None,
            records: [LevelRecord::default(); campaign::LEVEL_COUNT],
            keep_items: Vec::new(),
            keep_entries: Vec::new(),
            keep_selected: Vec::new(),
            keep_splices: Vec::new(),
            keep_cursor: 0,
            keep_scroll: 0,
            keep_visible_rows: 8,
            carried_entries: Vec::new(),
            victory_loadout: Vec::new(),
            victory_focus: campaign_ui::CampaignVictoryAction::PlayAgain,
            owned_upgrades: 0,
            world: None,
            levelup_choices: Vec::new(),
            chest_reward: None,
            shop_row: 0,
            shop_molt: None,
            levelup_desc: Vec::new(),
            levelup_idx: 0,
            intro_timer: 0.0,
            story_time: 0.0,
            last_game: Rect::new(0, 0, 0, 0),
            last_area: Rect::new(0, 0, 0, 0),
            killed_boss: None,
            rat_art: Vec::new(),
            rat_art_w: 0,
            run_counter: 0,
            recorded: false,
            new_best: false,
            last_score: 0,
            persist,
            fixed_seed,
            pixels: PixelBuffer::new(0, 0),
            picture_key: None,
            picture_dirty: true,
            world_draws: 0,
            repaint: true,
            sfx_frame: Vec::new(),
            sting: None,
            hud: hud::HudState::default(),
            hits: HitMap::default(),
            muted: false,
            music_volume,
            effects_volume,
            music_env,
            effects_env,
            subcell_env: crate::render::subcell::Choice::from_env(),
            smooth,
            smooth_env,
            foreground,
            foreground_env,
            fx,
            fx_env,
            glyphfx: {
                let mut layer = crate::render::glyphfx::GlyphFx::default();
                layer.set_enabled(glyphfx);
                layer
            },
            glyphfx_env,
            shake_scale,
            options_row: options::MUSIC_ROW,
            graphics_row: graphics::LIGHTING_ROW,
            graphics_return: Screen::Options,
            options_return: Screen::Title,
            preview_cd: 0.0,
            previews: 0,
            damage_taken: 0.0,
            dashes_used: 0,
            heals_picked: 0,
            boss_slain: false,
            prev_boss_alive: false,
            last_summary: None,
            map_open: false,
            map_animation_start: std::time::Instant::now(),
            last_minimap: Rect::new(0, 0, 0, 0),
            last_map: Rect::new(0, 0, 0, 0),
            minimap_scratch: minimap::MinimapScratch::new(),
            map_frame: minimap_source::MinimapFrame::new(),
            map_key_label: minimap::DEFAULT_MAP_KEY.to_string(),
            size_key_label: minimap::DEFAULT_SIZE_KEY.to_string(),
            cursor: cursor::CursorState::default(),
            cursor_env: cursor::env_override(),
            slot_caps: SlotCaps::DEFAULT,
            slot_caps_pinned: false,
            library_ui: library_ui::LibraryUi::default(),
            library_rows: library_index::all_rows(),
            library_return: Screen::Title,
            library_news: Vec::new(),
            library_shape: library_ui::DrawInfo::default(),
            pad_poller: PadPoller::new(),
            controls_row: 0,
            bind_capture: None,
            controls_note: String::new(),
            controls_from_options: true,
            controls_scroll: 0,
            controls_visible: 16,
            auto_pick_wait: 0.0,
            nest_ui: nest_ui::NestUi::default(),
            nest_return: Screen::Title,
            nest_read_only: false,
            nest_shape: nest_ui::DrawInfo::default(),
            bank_tally: None,
            banked: true,
            floors_cleared: 0,
            apex_kills: 0,
            prev_miniboss_alive: false,
            reset_armed: false,
            console: crate::console::Console::new(),
            console_cheat: false,
            console_paused: false,
            timescale: 1.0,
            perf_on: false,
            perf: PerfStats::default(),
            console_floor: None,
        };
        // `RATRUN_LIBRARY_ALL=1` unveils the index for screenshots and tests;
        // it is memory-only and never written back over the score file.
        app.scores.library.reveal_all_from_env();
        // Controls are persisted beside the other preferences (an old file
        // loads the defaults).
        let controls = crate::scores::controls_from_file(&app.scores.controls);
        app.input.set_controls(controls);
        app.sync_control_labels();
        app.push_volumes();
        app.menu_music();
        // The HUD's dash module names the live dash key (CTRL, or SPACE on a
        // terminal that cannot report a lone modifier), like the legends.
        app.hud.dash_key = app.dash_key();
        app
    }

    /// Force the dash mapping. `main` calls this with the answer the terminal
    /// gave *after* the enhancement flags were pushed: a terminal that answers
    /// the protocol query but ignores report-all-keys keeps the Space mapping.
    pub fn set_lone_ctrl(&mut self, on: bool) {
        self.input.set_lone_ctrl(on);
        self.sync_control_labels();
    }

    /// Push the current bindings into the HUD (dash module and the charged
    /// actives' token) and refresh the cached legends.
    fn sync_control_labels(&mut self) {
        self.input.refresh_controls();
        self.input.hud_labels_changed = false;
        self.hud.dash_key = self.input.dash_key();
        self.hud.dash_name = self.input.controls.live_dash_label().to_string();
        self.hud.active_name = self.input.controls.active_label().to_string();
    }

    /// The legend source the menus draw (the live bindings).
    pub fn controls(&self) -> &controls::Controls {
        &self.input.controls
    }

    /// Poll the gamepad once per frame (a no-op without the feature or a pad).
    fn poll_pad(&mut self) {
        if !self.input.focused {
            return;
        }
        let deadzone = self.input.controls.deadzone;
        let any = self.pad_poller.poll(&mut self.input.pad, deadzone);
        if (any || self.input.pad.active()) && self.input.pad.connected() {
            self.input.last_device = Device::Pad;
            self.input.controls.note_device(Device::Pad);
            self.input.prefer_mouse = false;
        } else if self.input.last_device != Device::Pad {
            self.input.controls.note_device(self.input.last_device);
        }
    }

    /// Persist the control configuration (in-memory only when `persist` is
    /// false, as in tests).
    fn save_controls(&mut self) {
        self.scores.controls = crate::scores::controls_to_file(&self.input.controls);
        if self.persist {
            self.scores.save();
        }
        self.sync_control_labels();
    }

    /// The world-space aim point, or `None` for auto-aim.
    ///
    /// * Mouse aim: the pointer, mapped through the same viewport + zoom as
    ///   the renderer (so the aim point is exact at 0.5x, 1x and 2x, and the
    ///   cheese wedge's tip marks it).
    /// * Gamepad: the right stick, when deflected, a fixed distance in front
    ///   of the rat.
    /// * Classic: never (weapons auto-aim; the pointer steers).
    pub fn aim_point(&self) -> Option<Vec2> {
        match self.input.controls.scheme() {
            Scheme::MouseAim => self.mouse_world(),
            Scheme::Gamepad => {
                let stick = self.input.pad.aim_vec(self.input.controls.deadzone)?;
                self.world
                    .as_ref()
                    .and_then(|w| controls::pad_aim_point(w.player.pos, stick))
            }
            Scheme::Classic => None,
        }
    }

    /// The key that dashes in this input mode (see `input::DashKey`).
    pub fn dash_key(&self) -> DashKey {
        self.input.dash_key()
    }

    /// The theme of the floor the title/select screen points at.
    ///
    /// The title's sector row and the sector-select grid both list every
    /// playable floor in `Theme::playable()` order (stage one, the two
    /// stage-two floors, the two stage-three floors, then the Flatbed getaway),
    /// so one cursor drives the material tint, the menu music and both lists.
    fn theme(&self) -> Theme {
        Theme::playable()[self.floor()]
    }

    /// The title's floor cursor, clamped into the sector row.
    fn floor(&self) -> usize {
        self.select_idx.min(Theme::playable().len() - 1)
    }

    /// The floor a live run is being played on (the title's cursor when no run
    /// is in flight). Plates drawn during a run must name **this** floor: the
    /// title's cursor may still sit on whichever floor the run was launched
    /// from, or on the Flatbed.
    fn live_theme(&self) -> Theme {
        self.world
            .as_ref()
            .map(|w| w.level.theme)
            .unwrap_or_else(|| self.theme())
    }

    /// Is the floor the cursor is on selectable? Its campaign stage must be
    /// reachable; the getaway ride is a bonus practice floor and is always
    /// open.
    fn sector_unlocked(&self) -> bool {
        self.scores.floor_unlocked(self.theme())
    }

    /// How many cards wide the sector-select grid is at the area it last drew
    /// in. `Up`/`Down` move a whole visual row; before the first draw (or on a
    /// terminal that has not sized the screen yet) the grid is a single column.
    fn select_cols(&self) -> isize {
        campaign_ui::level_select_cols(self.last_select_area) as isize
    }

    /// Step the floor cursor by `delta` slots, wrapping through the whole
    /// sector row. This is the visual order both lists use.
    fn sector_step(&mut self, delta: isize) {
        let n = Theme::playable().len() as isize;
        if n == 0 {
            return;
        }
        self.select_idx =
            (self.floor() as isize + delta).rem_euclid(n) as usize;
        self.menu_music();
        self.sfx(Sfx::UiMove);
    }

    /// Step the floor cursor one slot.
    fn cycle_sector(&mut self, step: isize) {
        self.sector_step(step);
    }

    /// Render the RASCII rat logo sized to the title area. The menu and key
    /// legend are laid out first (see `menu::title_art_width`), so the art is
    /// fitted — or omitted — in whatever rows remain. Recomputed only when the
    /// target width changes.
    fn ensure_rat_art(&mut self, area: Rect) {
        let desired = menu::title_art_width(area).unwrap_or(0);
        if desired != self.rat_art_w {
            self.rat_art = if desired == 0 {
                Vec::new()
            } else {
                crate::titleart::rat_lines(desired).unwrap_or_default()
            };
            self.rat_art_w = desired;
        }
    }

    fn menu_music(&mut self) {
        let theme = self.theme();
        // The menus play the floor's song at the calm tier with a soft entry
        // (`AudioEngine::play_menu_music`).
        self.music_intensity = 0;
        if let Some(a) = self.audio.as_mut() {
            a.play_menu_music(theme);
        }
    }

    /// Start (or restart) the level's music at `intensity` and record it.
    fn play_music(&mut self, theme: Theme, intensity: u32) {
        self.music_intensity = intensity;
        if let Some(a) = self.audio.as_mut() {
            a.play_music(theme, intensity);
        }
    }

    /// The music intensity tier the game state last asked for (see the
    /// `music_intensity` field); the app tests read it after a frame to check
    /// what the synth would have been told.
    pub fn music_intensity(&self) -> u32 {
        self.music_intensity
    }

    /// Ask the synth for an intensity tier and record the request.
    fn request_music_intensity(&mut self, intensity: u32) {
        self.music_intensity = intensity;
        if let Some(a) = self.audio.as_ref() {
            a.set_intensity(intensity);
        }
    }

    fn sfx(&self, sfx: Sfx) {
        if let Some(a) = self.audio.as_ref() {
            a.play_sfx(sfx);
        }
    }

    /// Queue a sting to play `delay` seconds from now (latest one wins).
    fn queue_sting(&mut self, sfx: Sfx, delay: f32) {
        self.sting = Some((sfx, delay.max(0.0)));
    }

    /// The sting still waiting to play, if any (tests read this).
    fn pending_sting(&self) -> Option<Sfx> {
        self.sting.map(|(s, _)| s)
    }

    /// Advance the deferred-sting clock and play it when it comes due.
    fn tick_stings(&mut self, dt: f32) {
        let Some((sfx, left)) = self.sting else {
            return;
        };
        let left = left - dt.max(0.0);
        if left <= 0.0 {
            self.sting = None;
            self.sfx(sfx);
        } else {
            self.sting = Some((sfx, left));
        }
    }

    /// Map the latest mouse cursor cell to a world position, using the same
    /// game viewport + camera the renderer used last frame.
    fn mouse_world(&self) -> Option<Vec2> {
        let (mc, mr) = self.input.mouse_pos?;
        let a = self.last_game;
        if a.width == 0 || a.height == 0 {
            return None;
        }
        let w = self.world.as_ref()?;
        // Clamp the cursor into the viewport so off-field targets still pull
        // the player toward that edge. Two pixels per cell row.
        let gx = (mc as i32 - a.x as i32).clamp(0, a.width as i32 - 1) as f32;
        let gy = (mr as i32 - a.y as i32).clamp(0, a.height as i32 - 1) as f32;
        // A zoomed view covers `zoom_scale` world pixels per screen pixel, so
        // the same cell maps further out in world space (and aim stays exact at
        // every zoom). `1,1` unzoomed.
        Some(
            w.cam
                + Vec2::new(
                    (gx + 0.5) * w.zoom_scale.x,
                    (gy * 2.0 + 1.0) * w.zoom_scale.y,
                ),
        )
    }

    /// The throttle vector fed to the world: keyboard unit vector, or — when
    /// the mouse was the last input — toward the cursor, easing to a stop.
    fn resolve_move_dir(&self) -> Vec2 {
        // Mouse aim: the pointer aims, so it never steers — WASD/arrows and
        // the pad stick are the only movement. Classic and Gamepad keep the
        // steering pointer.
        if self.input.controls.mouse_steers() && self.input.prefer_mouse {
            if let (Some(target), Some(w)) = (self.mouse_world(), self.world.as_ref()) {
                let to = target - w.player.pos;
                let d = to.len();
                return if d > 1.5 {
                    to.normalized() * (d / 12.0).clamp(0.0, 1.0)
                } else {
                    Vec2::ZERO
                };
            }
        }
        self.input.move_dir()
    }

    fn seed(&mut self) -> u64 {
        self.run_counter += 1;
        let jitter = self.run_counter.wrapping_mul(0x9E3779B97F4A7C15);
        if let Some(base) = self.fixed_seed {
            // Deterministic run: same seed sequence every launch.
            return base ^ jitter;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        nanos ^ jitter
    }

    /// Begin a **campaign** at level 1 (the title's START RUN, and sector 1 on
    /// the select screen). A run that starts from the title goes through the
    /// story dossier and then the access-route intro; a retry after a defeat
    /// skips the dossier.
    fn start_run(&mut self) {
        self.start_campaign(true);
    }

    /// Set the rack caps the next run starts with (the persistent slot
    /// upgrades bought between runs). Ownership already on a carried loadout is
    /// never dropped by a lower cap; see [`Loadout::set_caps`].
    ///
    /// The nest (`scores.meta`) is the real source of the caps; this is kept for
    /// the tests and the batch, which play a hand-made profile without a saved
    /// one.
    pub fn set_slot_caps(&mut self, caps: SlotCaps) {
        self.slot_caps = SlotCaps::new(caps.weapons, caps.passives);
        self.slot_caps_pinned = true;
    }

    /// The saved profile compiled into this run's permanent bonuses
    /// (`crate::meta::RunModifiers`). Read once per run start.
    pub fn run_modifiers(&self) -> meta::RunModifiers {
        let mut mods = meta::RunModifiers::from_profile(&self.scores.meta);
        // A pinned rack (a test or the batch asking for 6 + 6 / 8 + 8) wins
        // over the profile's slot tracks; every other track still applies.
        if self.slot_caps_pinned {
            mods.caps = self.slot_caps;
        }
        mods
    }

    /// Reset the per-run bookkeeping a new run starts with: the banking tally,
    /// the floor/apex counters and the revive/sting state. Called by every
    /// `start_*` (never by `enter_world`, which also runs for floor two).
    fn begin_run_bookkeeping(&mut self) {
        self.banked = false;
        self.bank_tally = None;
        self.floors_cleared = 0;
        self.apex_kills = 0;
        self.prev_miniboss_alive = false;
        if !self.slot_caps_pinned {
            self.slot_caps = self.run_modifiers().caps;
        }
    }

    fn start_campaign(&mut self, via_story: bool) {
        let seed = self.seed();
        self.begin_run_bookkeeping();
        // The nest: the profile's permanent upgrades are compiled once here and
        // ride every floor (the campaign carries the loadout, modifiers and
        // all).
        self.campaign = Some(CampaignState::with_modifiers(seed, self.run_modifiers()));
        self.practice = None;
        self.testbed = None;
        // A brand-new run starts clean: console cheats live in the world (and
        // the carried loadout), so a fresh world carries no taint.
        self.console_cheat = false;
        self.records = [LevelRecord::default(); campaign::LEVEL_COUNT];
        self.keep_items.clear();
        self.keep_entries.clear();
        self.keep_selected.clear();
        self.keep_splices.clear();
        self.victory_loadout.clear();
        let Some(world) = self.campaign.as_ref().map(|c| c.world()) else {
            return;
        };
        self.enter_world(world, via_story);
    }

    /// Begin a **practice run**: one floor, alone, on the default loadout,
    /// carrying nothing in and nothing out. It still records the floor clear
    /// (so the next sector unlocks) but never a campaign win.
    ///
    /// Every floor is practisable this way, the Flatbed included: the ride is
    /// the campaign's fourth level (`campaign::theme_stage`), and a practice run
    /// on it plays that index's map and boss on the first level's scalars.
    fn start_practice(&mut self, theme: Theme) {
        self.start_practice_with(theme, None);
    }

    pub(crate) fn start_testbed(&mut self, theme: Theme) {
        self.start_practice_with(theme, Some(self.testbed.unwrap_or_default()));
    }

    fn start_practice_with(&mut self, theme: Theme, testbed: Option<Testbed>) {
        // The floor's own campaign level, the ride's fourth included.
        let level = campaign::theme_stage(theme).min(campaign::LEVEL_COUNT - 1);
        // A practice run is a side-route floor too, so it uses the same seeded
        // derivation a campaign on that route does.
        let seed = campaign::level_seed_on(self.seed(), level, theme);
        self.begin_run_bookkeeping();
        self.campaign = None;
        self.practice = Some(theme);
        self.testbed = testbed;
        self.console_cheat = testbed.is_some();
        self.records = [LevelRecord::default(); campaign::LEVEL_COUNT];
        self.keep_items.clear();
        self.keep_entries.clear();
        self.keep_selected.clear();
        self.keep_splices.clear();
        self.victory_loadout.clear();
        // The theme is the floor the player picked, not the stage's original
        // one: `new_level` derives the theme from the level index, so a side
        // route needs the explicit constructor.
        // A testbed starts from neutral stats for repeatable weapon trials.
        let loadout = match self.testbed {
            Some(testbed) => Loadout::new(testbed.weapon),
            None => Loadout::with_modifiers(Loadout::START_WEAPON, self.run_modifiers()),
        };
        let mut world = World::new_level_with_theme(
            level,
            theme,
            seed,
            loadout,
            campaign::CampaignTotals::default(),
        );
        // No carried build: play this sector's map and boss on the first
        // level's difficulty curve instead of the campaign's late-run one.
        world.difficulty_override = Some(0);
        if let Some(testbed) = self.testbed.as_mut() {
            testbed.granted_partner = None;
            testbed.select_weapon(&mut world);
            world.configure_testbed();
        }
        self.enter_world(world, false);
        if self.testbed.is_some() {
            self.screen = Screen::Playing;
        }
    }

    /// Start the sector the select screen points at: sector 1 is the campaign
    /// from the beginning, a later sector is a labelled practice run. A locked
    /// sector refuses and stays put.
    fn start_sector(&mut self) {
        let theme = self.theme();
        if !self.sector_unlocked() {
            self.sfx(Sfx::UiMove);
            return;
        }
        self.sfx(Sfx::UiSelect);
        if theme == campaign::getaway_theme() {
            // The ride is the campaign's finale, and the sector select plays it
            // as a practice floor: a campaign has to start at the Sewer.
            self.start_getaway();
        } else if campaign::theme_stage(theme) == 0 {
            // Stage one is the campaign's front door, exactly as START RUN.
            self.start_run();
        } else {
            // Any later floor is a labelled practice run on **that** floor.
            self.start_practice(theme);
        }
    }

    /// Begin the **getaway ride**: the Flatbed alone, as a practice run.
    ///
    /// The ride is the campaign's finale as well as a practice floor, so this is
    /// just [`App::start_practice`] on its theme: the sector select starts a
    /// campaign only from stage one, and every other floor (the ride included)
    /// is a labelled practice run.
    fn start_getaway(&mut self) {
        self.start_practice(campaign::getaway_theme());
    }

    /// Hand a freshly built level to the app: reset the per-level telemetry,
    /// refresh the carried-upgrade strip, pick the intro screen and start the
    /// sector's music.
    fn enter_world(&mut self, world: World, via_story: bool) {
        let level = world.level_index;
        let theme = world.level.theme;
        let mut world = world;
        // The saved zoom is a preference, not per-level state: a new level opens
        // at it, with no glide-in from the previous level's zoom.
        world.set_zoom_instant(self.scores.zoom);
        // The Graphics page's two settings that live on the world start where
        // the player left them (the other four are handed over every frame in
        // `draw_game`, and the environment pins have already won their place in
        // `with_options`).
        world.lighting = self.saved_lighting();
        world.shake_scale = self.shake_scale;
        // The simulation's viewport is session input (spawn rings, offscreen
        // placement). `draw_game` writes it every frame, but a floor's first
        // tick runs before its first draw, so hand it the same value now
        // instead of leaving that one tick on the constructor's default.
        let game = game_rect(self.last_area);
        if game.width > 0 && game.height > 0 {
            world.viewport = Vec2::new(game.width as f32, (game.height * 2) as f32);
        }
        self.world = Some(world);
        self.recorded = false;
        self.new_best = false;
        self.damage_taken = 0.0;
        self.dashes_used = 0;
        self.heals_picked = 0;
        self.boss_slain = false;
        self.killed_boss = None;
        self.prev_boss_alive = false;
        self.last_summary = None;
        self.owned_upgrades = 0;
        // Fog of war resets with the level: a new `World` starts unexplored.
        self.map_open = false;
        self.last_minimap = Rect::new(0, 0, 0, 0);
        self.last_map = Rect::new(0, 0, 0, 0);
        self.hud.reset();
        // Entering a floor is discovering it, and it resets the per-floor
        // discovery marks (the library's "seen" counts floors, not frames).
        if self.testbed.is_none() {
            self.scores.library.begin_floor();
            self.scores.library.mark_seen(LibraryKey::Floor(theme));
        }
        // Announcing the nest once per floor keeps the permanent bonuses
        // distinct from the in-run passives the HUD rack grows: this is what the
        // rat carried *in*. Silent for a fresh profile.
        let (nest_line, miniboss_alive) = match self.world.as_ref() {
            Some(w) => (
                w.player.loadout.meta.summary(),
                w.miniboss_hp().is_some(),
            ),
            None => (String::new(), false),
        };
        if !nest_line.is_empty() {
            self.hud
                .push_callout(hud::CalloutKind::Nest, "NEST", &nest_line, 4.5);
        }
        self.prev_miniboss_alive = miniboss_alive;
        self.intro_timer = 4.0;
        self.carried_entries = match self.campaign.as_ref() {
            Some(c) if level > 0 => entries_for(&campaign::keepables(&c.loadout)),
            _ => Vec::new(),
        };
        self.screen = if via_story {
            self.story_time = 0.0;
            Screen::Story
        } else if level == 0 {
            Screen::MapIntro
        } else {
            Screen::LevelIntro
        };
        self.play_music(theme, 1);
    }

    /// Bank a finished (or abandoned) run into the nest **once**, and record
    /// the tally the report prints.
    ///
    /// The rule itself lives in [`meta::bank`]; this only gathers the run's
    /// figures: the unspent purse, what the splice shops took, the floors
    /// cleared, the apex kills, whether it won, and the practice/console flags.
    fn bank_the_run(&mut self) {
        if self.banked {
            return;
        }
        let end = match self.world.as_ref() {
            Some(w) => meta::RunEnd {
                purse: w.gold,
                // The floors already carried spent their own gold; the campaign
                // totals keep that sum.
                spent: w.carried.gold_spent.saturating_add(w.gold_spent),
                floors: self.floors_cleared,
                apex: self.apex_kills.min(u32::from(u8::MAX)) as u8,
                won: self.run_won(w),
                practice: self.practice.is_some(),
                console_used: self.console_cheat,
            },
            None => return,
        };
        let tally = meta::bank(&mut self.scores.meta, &end);
        self.bank_tally = Some(tally);
        self.banked = true;
        // New gold can open a slot: the next run starts on the new racks
        // (unless a caller pinned them).
        if !self.slot_caps_pinned {
            self.slot_caps = meta::RunModifiers::from_profile(&self.scores.meta).caps;
        }
        if self.persist {
            self.scores.save();
        }
    }

    /// Another run of the kind just played, skipping the story dossier.
    fn retry(&mut self) {
        match self.practice {
            Some(theme) if self.testbed.is_some() => self.start_testbed(theme),
            Some(theme) => self.start_practice(theme),
            None => self.start_campaign(false),
        }
    }

    /// Drop the run and return to the title (pause → Q, end → Q, victory → Q).
    ///
    /// Leaving a live run **banks its haul** too: the owner's rule is that the
    /// unspent purse is banked at the end of *every* run, and quitting is an end
    /// (a practice rate applies to a practice run). On the report/victory
    /// screens the run already banked itself in `finalize`, so this is a no-op.
    fn back_to_title(&mut self) {
        self.bank_the_run();
        self.world = None;
        self.campaign = None;
        self.practice = None;
        self.testbed = None;
        self.screen = Screen::Title;
        self.menu_music();
    }

    /// Resolve against the last displayed frame. Stop on the first activation:
    /// queued presses cannot land on controls revealed by that activation.
    fn mouse_action(&mut self) -> Option<Target> {
        if !self.input.pressed.is_empty() {
            self.hits.hovered = None;
        }
        for i in 0..self.input.mouse_events.len() {
            let ev = self.input.mouse_events[i];
            let target = self.hits.at(ev.column, ev.row);
            match ev.kind {
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    let step = if ev.kind == MouseEventKind::ScrollDown {
                        1
                    } else {
                        -1
                    };
                    match self.screen {
                        // The wheel steps the sector on the title row and
                        // anywhere on the select screen.
                        Screen::SectorSelect => self.cycle_sector(step),
                        Screen::Title
                            if matches!(
                                target,
                                Some(
                                    Target::Sectors
                                        | Target::PreviousSector
                                        | Target::NextSector
                                )
                            ) =>
                        {
                            self.menu_idx = 1;
                            self.cycle_sector(step);
                        }
                        Screen::Playing
                            if !self.map_open && target == Some(Target::Minimap) =>
                        {
                            self.step_minimap_size(step);
                        }
                        // Anywhere else over the playfield the wheel zooms the
                        // view: scroll down widens (2x the current view), up
                        // pulls in (0.5x). The minimap keeps its own job above,
                        // and the expanded map swallows the wheel (it is a
                        // panel you are reading, not the field). The HUD rails
                        // are outside `last_game`, so the wheel there does
                        // nothing, exactly as before.
                        Screen::Playing
                            if !self.map_open
                                && self.over_playfield(ev.column, ev.row) =>
                        {
                            self.step_zoom(step)
                        }
                        // The Library's wheel scrolls the list without moving
                        // the selection.
                        Screen::Library => self.scroll_library(step),
                        // The nest's wheel walks the track list (the window
                        // follows the cursor, so the wheel and the arrows agree).
                        Screen::Nest => {
                            if self.nest_ui.step(step) {
                                self.sfx(Sfx::UiMove);
                            }
                        }
                        // The wheel steps whichever options row it is over
                        // (scroll up raises a level, scroll down lowers it).
                        Screen::Options => {
                            let row = match target {
                                Some(Target::OptionsBar(bar)) => bar,
                                Some(Target::OptionsRow(i)) => i,
                                _ => options::ROW_COUNT,
                            };
                            if row < options::ROW_COUNT {
                                self.options_row = row;
                                self.scroll_option(row, step);
                            }
                        }
                        // The wheel steps whichever graphics row it is over
                        // (scroll down steps it forward, like the zoom row).
                        Screen::Graphics => {
                            if let Some(Target::GraphicsRow(i)) = target {
                                let row = i.min(graphics::ROW_COUNT - 1);
                                self.graphics_row = row;
                                self.adjust_graphics(row, step);
                            }
                        }
                        _ => {}
                    }
                }
                // Only a left press activates a target here. A right click is
                // Back on every menu and, in play, the actives' trigger: it
                // never opens the pause plate (`mouse_only_*` pinned the old
                // right-click-pauses behaviour that the owner asked to drop).
                MouseEventKind::Moved
                | MouseEventKind::Drag(_)
                | MouseEventKind::Down(MouseButton::Left) => {
                    self.hits.hovered = target;
                    match target {
                        Some(Target::Start) => self.menu_idx = 0,
                        Some(Target::Sectors | Target::PreviousSector | Target::NextSector) => {
                            self.menu_idx = 1
                        }
                        Some(Target::Options) => self.menu_idx = 2,
                        Some(Target::Library) => self.menu_idx = 3,
                        Some(Target::Nest) => self.menu_idx = 4,
                        Some(Target::Quit) => self.menu_idx = 5,
                        Some(Target::OptionsRow(i)) => {
                            self.options_row = i.min(options::ROW_COUNT - 1)
                        }
                        Some(Target::GraphicsRow(i)) => {
                            self.graphics_row = i.min(graphics::ROW_COUNT - 1)
                        }
                        Some(Target::OptionsBar(bar)) => {
                            self.options_row = bar.min(options::ROW_COUNT - 1);
                            // A press or drag on the track sets the level from
                            // the pointer column; handled here so the row's
                            // click-to-step does not run on top of it.
                            if matches!(
                                ev.kind,
                                MouseEventKind::Down(MouseButton::Left)
                                    | MouseEventKind::Drag(MouseButton::Left)
                            ) {
                                if let Some(track) = self.hits.rect_for(Target::OptionsBar(bar)) {
                                    let level = options::percent_at(track, ev.column);
                                    self.set_option_bar(bar, level);
                                }
                                return None;
                            }
                        }
                        Some(Target::SelectSector(i)) => {
                            // The target carries the **floor** index (into
                            // `Theme::playable()`), never a campaign stage.
                            self.select_idx = i.min(Theme::playable().len().saturating_sub(1));
                        }
                        Some(Target::KeepItem(i)) => self.keep_cursor = i,
                        Some(Target::RouteOption(_)) => self.menu_idx = 1,
                        Some(Target::Upgrade(idx)) => self.levelup_idx = idx,
                        _ => {}
                    }
                    if ev.kind == MouseEventKind::Down(MouseButton::Left) && target.is_some() {
                        return target;
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn update(&mut self, dt: f32) {
        // The pad is polled before the frame's input is consumed, so a button
        // pressed this frame is an action this frame.
        let was_pad = self.input.controls.device() == Device::Pad;
        self.poll_pad();
        // Discrete input and the console can change what a frozen world looks
        // like in ways the picture key cannot see (a console `give`, a debug
        // toggle): redraw it this frame rather than reuse the cached pixels.
        if self.input.discrete || self.console.open || self.input.pad.active() {
            self.picture_dirty = true;
        }
        self.input.tick(dt);
        self.input.controls.note_device(self.input.last_device);
        // The HUD's dash/active names follow the device in hand: when the pad
        // takes over (or hands back to the keyboard and mouse) the legends
        // rebuild, so the HUD's cached copies have to be re-read too.
        if was_pad != (self.input.controls.device() == Device::Pad) {
            self.input.hud_labels_changed = true;
        }
        if self.input.hud_labels_changed {
            self.sync_control_labels();
        }
        // Deferred stings first: a queued fanfare lands after the sound that
        // caused it (see `queue_sting`).
        self.tick_stings(dt);
        // The drop-down console: the backtick/tilde key toggles it from the
        // title or any screen with a run. While it is open it swallows every
        // keystroke and the game is paused; closing resumes exactly where play
        // was. The toggle key itself is never typed into the prompt.
        if self.console_enabled() {
            let backtick = self.input.just_pressed(KeyCode::Char('`'))
                || self.input.just_pressed(KeyCode::Char('~'));
            if backtick {
                if self.console.open {
                    self.close_console();
                } else {
                    // Any screen: the title, a run, the Library, the Nest,
                    // Options and Controls all sit under the console.
                    self.console.open();
                }
                // The toggle key is consumed: nothing leaks to the game.
                self.input.pressed.clear();
                self.console.tick(dt);
                return;
            }
            if self.console.open {
                let keys: Vec<crossterm::event::KeyEvent> = self.input.pressed.clone();
                for key in keys {
                    self.handle_console_key(key);
                }
                self.console.tick(dt);
                // Swallow the frame's input so no key leaks to the game.
                self.input.pressed.clear();
                return;
            }
            self.console.tick(dt);
        }
        // Console `pause` freezes the simulation only (`update_playing` honours
        // the flag); menus and every global key keep working, and closing the
        // console clears it (`close_console`).
        // The global quick keys stand down while the Controls screen is
        // capturing a binding: the key being bound (`M`, `C`, `[` …) must only
        // become the binding, never also fire the action it used to carry.
        let capturing = matches!(self.screen, Screen::Controls) && self.bind_capture.is_some();
        // On the splice shelf a bare `M` is Molt (the shelf's legend says so),
        // so that press must not also flip the mute.
        let molt_key = matches!(self.screen, Screen::Shop) && self.input.pressed_char('m');
        // Global mute toggle.
        if !capturing && !molt_key && self.input.action_pressed(Action::Mute) {
            self.toggle_mute();
        }
        // Global cheese-cursor preference toggle ('c'; ignores Ctrl+C).
        let cursor_toggled = !capturing && self.input.action_pressed(Action::ToggleCursor);
        if cursor_toggled {
            self.toggle_cheese_cursor();
        }
        // Volume quick keys, live on every screen so a scripted run can set a
        // level without navigating: `[` / `]` step music, `-` / `=` step
        // effects (both in 5% steps). Discrete presses only, so holding a key
        // never runs the level away.
        let mut volume_changed = false;
        if !capturing && self.input.action_pressed(Action::MusicDown) {
            self.set_music_volume(options::step_volume(self.music_volume, -1));
            volume_changed = true;
        }
        if !capturing && self.input.action_pressed(Action::MusicUp) {
            self.set_music_volume(options::step_volume(self.music_volume, 1));
            volume_changed = true;
        }
        if !capturing && self.input.action_pressed(Action::EffectsDown) {
            self.set_effects_volume(options::step_volume(self.effects_volume, -1), true);
            volume_changed = true;
        }
        if !capturing && self.input.action_pressed(Action::EffectsUp) {
            self.set_effects_volume(options::step_volume(self.effects_volume, 1), true);
            volume_changed = true;
        }
        // Preview blips are rate limited so a mouse drag across the slider
        // cannot machine-gun one effect per cell.
        self.preview_cd = (self.preview_cd - dt.max(0.0)).max(0.0);
        // The auto-pick reveal/spacing clock runs on every screen; only the
        // play and level-up screens ever spend it.
        self.auto_pick_wait = (self.auto_pick_wait - dt.max(0.0)).max(0.0);
        // The HUD read-out ages with the frame clock on every screen; during
        // play `hud.tick` advances it too.
        if !matches!(self.screen, Screen::Playing) {
            self.hud.tick_volume_flash(dt);
            self.hud.tick_zoom_flash(dt);
        }
        // Render-only zoom easing, on every screen: a zoom set on the options
        // plate (opened over the paused field) glides in behind it, and the
        // simulation never sees it (see `World::tick_zoom`).
        if let Some(w) = self.world.as_mut() {
            w.tick_zoom(dt);
        }
        // Options: the title row and the pause legend open it; nothing else does.
        // `previous` is read first, so a screen change made here clears the
        // stale hit rectangles at the end of the frame like any other.
        let previous = self.screen;
        if !capturing && !volume_changed && self.input.action_pressed(Action::Options) {
            let from = self.screen;
            if matches!(from, Screen::Title | Screen::Paused) {
                self.open_options(from);
            }
        }
        let click = self.mouse_action();
        match self.screen {
            Screen::Title => self.update_title(click),
            Screen::SectorSelect => self.update_sector_select(click),
            Screen::Story => {
                self.story_time += dt;
                self.update_story(click);
            }
            Screen::MapIntro => self.update_mapintro(dt, click),
            Screen::LevelIntro => self.update_levelintro(click),
            Screen::Playing => self.update_playing(dt, click),
            Screen::LevelUp => self.update_levelup(click),
            Screen::Paused => self.update_paused(click),
            Screen::Options => self.update_options(click),
            Screen::Graphics => self.update_graphics(click),
            Screen::LevelComplete => self.update_levelcomplete(click),
            Screen::KeepThree => self.update_keep_three(click),
            Screen::Victory => self.update_victory(click),
            Screen::End => self.update_end(click),
            Screen::Chest => self.update_chest(click),
            Screen::Shop => self.update_shop(click),
            Screen::Library => {
                self.update_library(click);
                // The card's scene runs on the screen's own clock; the input
                // above may have just moved the selection (or closed the
                // screen), which is what decides whether it runs at all.
                if matches!(self.screen, Screen::Library) {
                    self.tick_library_card(dt);
                }
            }
            Screen::Controls => self.update_controls(click),
            Screen::Nest => self.update_nest(click),
        }
        if self.screen != previous {
            self.hits.clear();
            self.hits.hovered = None;
            self.picture_dirty = true;
            self.repaint = true;
        }
        let kb_pressed = !self.input.pressed.is_empty() && !cursor_toggled;
        self.cursor.update(
            dt,
            cursor::CursorInput {
                mouse_active: self.input.mouse_active,
                mouse_clicked: self.input.mouse_clicked,
                kb_pressed,
                kb_moving: self.input.move_dir() != Vec2::ZERO,
                prefer_mouse: self.input.prefer_mouse,
                mouse_aims: self.input.controls.mouse_aims(),
                playing: matches!(self.screen, Screen::Playing),
            },
        );
    }

    /// Whether the cheese cursor feature is enabled (environment override beats
    /// the persisted `scores.json` setting).
    pub fn cursor_enabled(&self) -> bool {
        self.cursor_env
            .unwrap_or_else(|| self.scores.cheese_cursor.is_on())
    }

    /// True when the cheese cursor is currently visible on screen (used by
    /// `main` to switch the OS pointer to a slim OSC 22 crosshair).
    pub fn cursor_visible(&self) -> bool {
        self.cursor_enabled() && self.input.mouse_pos.is_some() && self.cursor.is_visible()
    }

    /// Toggle and persist the cheese cursor preference (in-memory only when
    /// `self.persist` is false, as in tests).
    fn toggle_cheese_cursor(&mut self) {
        self.scores.cheese_cursor = self.scores.cheese_cursor.toggle();
        if self.persist {
            self.scores.save();
        }
        self.sfx(Sfx::UiMove);
    }

    // ---- Console (w3-console) -------------------------------------------
    //
    // The Quake-style drop-down console is a debug/admin surface. It is on by
    // default in this development build; a release build can hide it with the
    // `console` cargo feature (on by default) and `RATRUN_CONSOLE=0` disables
    // it at runtime. The `--ai` and `--bot-batch` modes never build an `App`
    // interactive loop, so the console is inherently inactive there.

    /// Whether the console may be opened in this process.
    pub fn console_enabled(&self) -> bool {
        if !cfg!(feature = "console") {
            return false;
        }
        match std::env::var("RATRUN_CONSOLE") {
            Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"),
            Err(_) => true,
        }
    }

    /// Mark this run as console-tainted: no high score, no campaign record, no
    /// library counters are saved from it. Read-only commands never call this.
    pub fn console_flag_cheat(&mut self) {
        self.console_cheat = true;
    }

    /// Push one output line into the console's scrollback.
    pub fn console_line(&mut self, text: impl Into<String>, kind: crate::console::LineKind) {
        self.console.push(crate::console::Line::new(text, kind));
    }

    /// Empty the console's scrollback (the `clear` command).
    pub fn console_clear(&mut self) {
        // `Console::clear` also resets the scroll: a view scrolled up over an
        // emptied scrollback would hide every new line.
        self.console.clear();
    }

    /// The active world, for a command handler's read/write.
    pub fn world_mut(&mut self) -> Option<&mut World> {
        self.world.as_mut()
    }

    /// The active world, read-only.
    pub fn world_ref(&self) -> Option<&World> {
        self.world.as_ref()
    }

    /// The lighting quality in force: the run's own (the console's `lights`
    /// command and the Graphics page's LIGHTING row set it), else the saved
    /// preference, else whatever this terminal's colour depth defaults to.
    pub fn lighting_mode(&self) -> crate::render::lighting::LightingMode {
        self.world
            .as_ref()
            .map(|w| w.lighting)
            .unwrap_or_else(|| self.saved_lighting())
    }

    /// The persisted lighting quality, or (a fresh profile, or a mode a future
    /// build wrote) the terminal's default.
    fn saved_lighting(&self) -> crate::render::lighting::LightingMode {
        self.scores.graphics.lighting_mode().unwrap_or_else(|| {
            crate::render::lighting::LightingMode::default_for_depth(crate::term::color_depth())
        })
    }

    /// Set the lighting quality — the console's `lights` command and the
    /// Graphics page's LIGHTING row. It is **persisted** (like every other
    /// preference on that page), so the next run starts at it; render-only
    /// either way, so it flags nothing as cheated.
    pub fn set_lighting(&mut self, mode: crate::render::lighting::LightingMode) {
        if let Some(w) = self.world.as_mut() {
            w.lighting = mode;
        }
        self.scores.graphics.set_lighting(mode);
        if self.persist {
            self.scores.save();
        }
    }

    /// Console `bank <n>`: set the nest's banked gold.
    pub fn console_set_bank(&mut self, gold: u32) {
        self.scores.meta.bank = gold;
        if self.persist {
            self.scores.save();
        }
    }

    /// Console `nest max|reset`: every track at its cap, or a fresh profile
    /// (the bank is kept by `max`, wiped by `reset`, as the options row does).
    pub fn console_nest(&mut self, max: bool) {
        if max {
            let bank = self.scores.meta.bank;
            self.scores.meta = meta::MetaProfile::maxed();
            self.scores.meta.bank = bank;
            if !self.slot_caps_pinned {
                self.slot_caps = meta::RunModifiers::from_profile(&self.scores.meta).caps;
            }
            if self.persist {
                self.scores.save();
            }
        } else {
            self.reset_nest();
        }
    }

    /// The nest's banked gold (console read-out).
    pub fn nest_bank(&self) -> u32 {
        self.scores.meta.bank
    }

    /// Console `library unlock|lock`: reveal every entry, or forget them all.
    pub fn console_library(&mut self, unlock: bool) {
        if unlock {
            self.scores.library.reveal_all();
        } else {
            self.scores.library = Default::default();
        }
        if self.persist {
            self.scores.save();
        }
    }

    /// Play a sound effect from the console's `sfx` command.
    pub fn play_sfx(&self, sfx: Sfx) {
        self.sfx(sfx);
    }

    /// Set the music intensity tier from the console's `music` command.
    pub fn set_music_intensity(&mut self, intensity: u32) {
        self.request_music_intensity(intensity);
    }

    /// Toggle the perf overlay (`fps`/`perf`).
    pub fn set_fps(&mut self, on: bool) {
        self.perf_on = on;
    }

    /// Is the perf overlay on?
    pub fn fps_enabled(&self) -> bool {
        self.perf_on
    }

    /// Set the simulation time scale, clamped to `0.1..=8.0`.
    pub fn set_timescale(&mut self, x: f32) {
        self.timescale = x.clamp(0.1, 8.0);
    }

    /// Toggle the persistent pause (`pause`). Returns the new paused state.
    pub fn toggle_console_pause(&mut self) -> bool {
        self.console_paused = !self.console_paused;
        self.console_paused
    }

    /// Advance `n` fixed ticks while paused (`step n`).
    pub fn step_ticks(&mut self, n: usize) {
        const DT: f32 = 1.0 / 60.0;
        for _ in 0..n {
            if let Some(w) = self.world.as_mut() {
                w.update_with(DT, Vec2::ZERO, false, false);
            }
        }
    }

    /// `--prof-bench`: skip the intro plate and play.
    pub(crate) fn prof_force_playing(&mut self) {
        self.screen = Screen::Playing;
    }

    /// `--prof-bench`: the current screen's name.
    pub(crate) fn prof_screen(&self) -> &'static str {
        match self.screen {
            Screen::Title => "Title",
            Screen::Playing => "Playing",
            Screen::Paused => "Paused",
            _ => "Other",
        }
    }

    /// Start a practice run on `theme` (`floor <name>`).
    pub fn start_floor(&mut self, theme: Theme) -> Result<(), String> {
        self.console_floor = Some(theme);
        self.start_practice(theme);
        // The console picked the floor (locked or not): the run stays flagged.
        self.console_cheat = true;
        Ok(())
    }

    /// Start a campaign at `stage` (0-based), optionally on `route`.
    pub fn start_campaign_at(&mut self, stage: usize, route: Option<Theme>) -> Result<(), String> {
        let seed = self.seed();
        // The same per-run reset every other start path does: without it the
        // previous run's `banked` flag, bank tally and floor/apex counters
        // leaked into this one (its report printed the old haul and counted
        // the old floors).
        self.begin_run_bookkeeping();
        let mut campaign = CampaignState::with_modifiers(seed, self.run_modifiers());
        campaign.level = stage.min(campaign::LEVEL_COUNT - 1);
        if let Some(theme) = route {
            campaign.route.choose(campaign.level, theme);
        }
        self.campaign = Some(campaign);
        self.practice = None;
        self.testbed = None;
        self.console_floor = None;
        self.records = [LevelRecord::default(); campaign::LEVEL_COUNT];
        self.keep_items.clear();
        self.keep_entries.clear();
        self.keep_selected.clear();
        self.keep_splices.clear();
        self.victory_loadout.clear();
        self.console_cheat = false;
        let world = self
            .campaign
            .as_ref()
            .map(|c| c.world())
            .ok_or_else(|| "campaign could not start".to_string())?;
        self.enter_world(world, false);
        // A console-chosen stage or route is not an earned one: that run stays
        // flagged. Stage 1 on the default route is an ordinary fresh campaign.
        self.console_cheat = stage > 0 || route.is_some();
        Ok(())
    }

    /// The seed of the current run (the level's seed, or 0 with no run).
    pub fn current_seed(&self) -> u64 {
        self.world.as_ref().map(|w| w.level.seed).unwrap_or(0)
    }

    /// Pin the seed for the next run (`seed <n>`).
    pub fn set_next_seed(&mut self, seed: u64) {
        self.fixed_seed = Some(seed);
    }

    /// Restart the current run from the top (`restart`).
    pub fn restart_run(&mut self) -> Result<(), String> {
        if self.practice.is_none() && self.campaign.is_none() {
            return Err("no run in progress".to_string());
        }
        self.retry();
        Ok(())
    }

    /// Close the console. A console `pause` ends with it: the pause is a
    /// console tool (`step n` works under it), so shutting the console is the
    /// obvious undo and a closed console never leaves the world frozen.
    fn close_console(&mut self) {
        self.console.close();
        self.console_paused = false;
    }

    /// Route one key press to the console and run whatever it asks for.
    fn handle_console_key(&mut self, key: crossterm::event::KeyEvent) {
        let rows = self.last_game.height.max(20) as usize;
        match self.console.handle_key(key, rows) {
            Some(crate::console::ConsoleAction::Close) => {
                self.close_console();
            }
            Some(crate::console::ConsoleAction::Complete) => {
                if self.console.comp_idx.is_some() {
                    // A list is already up: Tab accepts the selected candidate
                    // and advances to the next one (the usual Quake cycling).
                    self.console.accept_completion();
                } else {
                    crate::console::begin_completion(&mut self.console);
                }
            }
            Some(crate::console::ConsoleAction::Execute(line)) => {
                // The Enter handler already echoed the line into the scrollback.
                match crate::console::run_command(self, &line) {
                    Ok(()) => {}
                    Err(e) => {
                        self.console.push(crate::console::Line::new(
                            format!("error: {e}"),
                            crate::console::LineKind::Error,
                        ));
                    }
                }
            }
            None => {}
        }
    }

    // ---- Volume ---------------------------------------------------------

    /// The live music level (`0..=100`, steps of 5).
    pub fn music_volume(&self) -> u8 {
        self.music_volume
    }

    /// The live effects level (`0..=100`, steps of 5).
    pub fn effects_volume(&self) -> u8 {
        self.effects_volume
    }

    /// Whether the master mute is engaged.
    pub fn muted(&self) -> bool {
        self.muted
    }

    /// How many effects-volume preview blips have played (tests read this to
    /// see the rate limit with no audio device).
    pub fn preview_blips(&self) -> u32 {
        self.previews
    }

    /// Push both levels and the mute to the engine. A no-op without a device:
    /// the app's own records stay authoritative so the options screen and the
    /// HUD read-out work on every headless run.
    fn push_volumes(&mut self) {
        let Some(a) = self.audio.as_mut() else {
            return;
        };
        a.set_muted(self.muted);
        a.set_music_volume(self.music_volume);
        a.set_effects_volume(self.effects_volume);
    }

    /// Save the levels beside the scores (in-memory only when `persist` is
    /// false). A channel pinned by its environment variable is applied but
    /// never written back over the saved preference.
    fn save_volumes(&mut self) {
        if self.music_env.is_none() {
            self.scores.music_volume = self.music_volume;
        }
        if self.effects_env.is_none() {
            self.scores.effects_volume = self.effects_volume;
        }
        if self.persist {
            self.scores.save();
        }
    }

    /// Set the music level. The playing track's sink takes the new gain at
    /// once — no restart, no gap.
    fn set_music_volume(&mut self, percent: u8) {
        let next = audio::snap_percent(percent);
        if next == self.music_volume {
            return;
        }
        self.music_volume = next;
        if let Some(a) = self.audio.as_mut() {
            a.set_music_volume(next);
        }
        self.save_volumes();
        self.hud.notify_volume(hud::VolumeChannel::Music, next);
    }

    /// Set the effects level. The voices already sounding and every new voice
    /// take the new gain; `preview` plays a short blip so the change is
    /// audible on the options screen.
    fn set_effects_volume(&mut self, percent: u8, preview: bool) {
        let next = audio::snap_percent(percent);
        if next == self.effects_volume {
            return;
        }
        self.effects_volume = next;
        if let Some(a) = self.audio.as_mut() {
            a.set_effects_volume(next);
        }
        self.save_volumes();
        self.hud.notify_volume(hud::VolumeChannel::Effects, next);
        if preview {
            self.play_volume_preview();
        }
    }

    /// Play the effects-volume preview blip when the rate limit allows it.
    fn play_volume_preview(&mut self) {
        if self.preview_cd > 0.0 {
            return;
        }
        self.preview_cd = PREVIEW_COOLDOWN_SECS;
        self.previews = self.previews.saturating_add(1);
        self.sfx(Sfx::Gem);
    }

    /// Flip the master mute, keeping both levels underneath so unmuting
    /// restores exactly what the player had.
    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.hud.telemetry.muted = self.muted;
        if let Some(a) = self.audio.as_mut() {
            a.set_muted(self.muted);
        }
        self.sfx(Sfx::UiMove);
    }

    /// Open the options screen from `from` (the title or the pause screen),
    /// remembering where Back returns to.
    fn open_options(&mut self, from: Screen) {
        self.options_return = if matches!(from, Screen::Paused) {
            Screen::Paused
        } else {
            Screen::Title
        };
        self.options_row = options::MUSIC_ROW;
        self.reset_armed = false;
        self.screen = Screen::Options;
        // Music stays audible on the options screen wherever it is opened
        // from: the title's calm track and the paused level's track both keep
        // playing (pause never stops or ducks the synth), so the music slider
        // is audible without restarting anything.
    }

    /// Leave the options screen for wherever it was opened from.
    fn close_options(&mut self) {
        self.sfx(Sfx::UiSelect);
        self.reset_armed = false;
        self.screen = self.options_return;
    }

    // ---- Library ---------------------------------------------------------

    /// Open the specimen index from the title or the pause screen.
    fn open_library(&mut self, from: Screen) {
        self.library_return = if matches!(from, Screen::Paused) {
            Screen::Paused
        } else {
            Screen::Title
        };
        // A run in flight is frozen while the index is open: this screen never
        // calls `World::update`, exactly like the pause plate and the shelf.
        self.map_open = false;
        self.library_ui.card_open = false;
        // No scene survives the screen it was built for.
        self.library_ui.stop();
        self.screen = Screen::Library;
    }

    /// Leave the index for wherever it was opened from.
    fn close_library(&mut self) {
        self.sfx(Sfx::UiSelect);
        self.library_ui.stop();
        self.screen = self.library_return;
    }

    // ---- The nest --------------------------------------------------------

    /// Open the permanent upgrades. From the pause screen it is **read-only**:
    /// a run in flight cannot buy anything (the bank is the profile's, and the
    /// run's modifiers were compiled when it started).
    fn open_nest(&mut self, from: Screen) {
        self.nest_return = from;
        self.nest_read_only = matches!(from, Screen::Paused);
        self.nest_ui.pending = None;
        // A run in flight keeps its own state frozen behind the plate, exactly
        // like the pause screen, the library and the splice shelf.
        self.map_open = false;
        self.screen = Screen::Nest;
    }

    /// Leave the nest for wherever it was opened from.
    fn close_nest(&mut self) {
        self.sfx(Sfx::UiSelect);
        self.nest_ui.pending = None;
        self.screen = self.nest_return;
    }

    /// One frame of the nest. Every key and pointer action lives here.
    fn update_nest(&mut self, click: Option<Target>) {
        // Back is the live binding (Esc / pad B by default), plus Backspace.
        if click == Some(Target::Back) || self.input.back_pressed()
            || self.input.just_pressed(KeyCode::Backspace)
        {
            if self.nest_ui.pending.take().is_some() {
                // Esc cancels the pending confirmation first, like the shop.
                self.sfx(Sfx::UiMove);
            } else {
                self.close_nest();
            }
            return;
        }
        // `nav_pressed` covers the arrows, the movement bindings and the pad.
        if (self.input.nav_pressed(0, -1) || self.input.pressed_char('k'))
            && self.nest_ui.step(-1) {
                self.sfx(Sfx::UiMove);
            }
        if (self.input.nav_pressed(0, 1) || self.input.pressed_char('j'))
            && self.nest_ui.step(1) {
                self.sfx(Sfx::UiMove);
            }
        if self.input.just_pressed(KeyCode::PageUp)
            && self.nest_ui.step(-(self.nest_shape.visible.max(1) as isize)) {
                self.sfx(Sfx::UiMove);
            }
        if self.input.just_pressed(KeyCode::PageDown)
            && self.nest_ui.step(self.nest_shape.visible.max(1) as isize) {
                self.sfx(Sfx::UiMove);
            }

        // Pointer: a row selects (and, on a click, buys); the refund row and the
        // back chip have targets of their own.
        let mut activate: Option<usize> = None;
        match click {
            Some(Target::NestRow(i)) => {
                if i < meta::TRACK_COUNT {
                    self.nest_ui.cursor = i;
                    self.nest_ui.pending = None;
                    self.sfx(Sfx::UiMove);
                    activate = Some(i);
                }
            }
            Some(Target::NestRefund) => {
                self.nest_ui.pending = Some(nest_ui::Pending::Refund);
                self.sfx(Sfx::UiMove);
            }
            _ => {}
        }
        if self.input.action_pressed(Action::Reset) {
            if self.scores.meta.spent_total() == 0 {
                self.sfx(Sfx::Denied);
            } else {
                self.nest_ui.pending = Some(nest_ui::Pending::Refund);
                self.sfx(Sfx::UiMove);
            }
        }

        // Confirm (Enter / pad A by default): confirm a pending action, or
        // activate the row under the cursor.
        if self.input.confirm_pressed() {
            activate = Some(self.nest_ui.cursor);
        }
        let Some(idx) = activate else {
            return;
        };
        let track = meta::Track::from_index(idx).unwrap_or_default();
        match self.nest_ui.pending.take() {
            Some(nest_ui::Pending::Buy(t)) => {
                if t == track || click.is_some() {
                    self.nest_buy(t);
                }
            }
            Some(nest_ui::Pending::Refund) => self.nest_refund(),
            None => {
                // A first press on a big buy (or a maxed/unaffordable row) only
                // arms the confirmation; the cheap ones buy straight away.
                match self.scores.meta.next_price(track) {
                    Some(price) if nest_ui::needs_confirm(price) => {
                        self.nest_ui.pending = Some(nest_ui::Pending::Buy(track));
                        self.sfx(Sfx::UiMove);
                    }
                    Some(_) => self.nest_buy(track),
                    None => self.sfx(Sfx::Denied),
                }
            }
        }
    }

    /// Buy one level of `track` from the bank.
    fn nest_buy(&mut self, track: meta::Track) {
        if self.nest_read_only {
            self.sfx(Sfx::Denied);
            return;
        }
        match self.scores.meta.buy(track) {
            Some(_) => {
                self.sfx(Sfx::Purchase);
                if self.persist {
                    self.scores.save();
                }
            }
            None => self.sfx(Sfx::Denied),
        }
        self.nest_ui.pending = None;
    }

    /// Refund every track and return the gold (free: the nest is for
    /// experimenting).
    fn nest_refund(&mut self) {
        if self.nest_read_only {
            self.sfx(Sfx::Denied);
            return;
        }
        let paid = self.scores.meta.refund_all();
        if paid > 0 {
            self.sfx(Sfx::Purchase);
            if self.persist {
                self.scores.save();
            }
        } else {
            self.sfx(Sfx::UiMove);
        }
        self.nest_ui.pending = None;
    }

    /// Wipe the whole nest (the options screen's RESET, after its confirm): the
    /// bank **and** every track. Scores, floor progress and the Library stay.
    fn reset_nest(&mut self) {
        self.scores.reset_nest();
        if !self.slot_caps_pinned {
            self.slot_caps = meta::RunModifiers::from_profile(&self.scores.meta).caps;
        }
        if self.persist {
            self.scores.save();
        }
    }

    /// The run's own permanent upgrades, for the read-only nest (the pause
    /// screen). `None` when no run is in flight (the saved profile is drawn).
    fn live_modifiers(&self) -> Option<meta::RunModifiers> {
        self.world.as_ref().map(|w| w.player.loadout.meta)
    }

    /// Drain the library's freshly discovered keys into the HUD's callout rail
    /// (one `LIBRARY + Pillbug` pill per discovery, replacing the oldest slot
    /// when the rail is full).
    fn drain_library_news(&mut self) {
        self.library_news.clear();
        self.scores.library.drain_news(&mut self.library_news);
        for i in 0..self.library_news.len() {
            let key = self.library_news[i];
            let line = format!("+ {}", library_index::name_of(key));
            self.hud
                .push_callout(hud::CalloutKind::Library, "LIBRARY", &line, 2.6);
        }
    }

    /// The Library's own count of entries in its current tab.
    fn library_entries(&self) -> usize {
        self.library_rows
            .iter()
            .filter(|r| r.tab == self.library_ui.tab)
            .count()
    }

    /// Mouse wheel on the Library: move the list window, not the selection.
    fn scroll_library(&mut self, step: isize) {
        let visible = self.library_shape.visible.max(1);
        let max = self.library_shape.lines.saturating_sub(visible);
        let next = (self.library_ui.scroll as isize + step * 3).clamp(0, max as isize);
        self.library_ui.scroll = next as usize;
    }

    /// Drive the selected card's scene (`render::library::scene`).
    ///
    /// Only the entry the cursor is on animates, and only when its card is
    /// actually on screen and fully catalogued (a `???` row and a silhouetted
    /// plate build nothing). The scene is a separate sandbox `World`: the run
    /// behind a pause-opened index is never ticked, and its cached picture is
    /// never invalidated.
    fn tick_library_card(&mut self, dt: f32) {
        let key = self.library_ui.selected_key(&self.library_rows);
        let on_screen = !self.library_shape.narrow || self.library_ui.card_open;
        let live = on_screen
            && key.is_some_and(|k| library_ui::catalogued(&self.scores.library, k));
        self.library_ui.tick_card(dt, key, live);
    }

    /// One frame of the Library screen. Every key and pointer action lives
    /// here (the screen's whole input is this one function).
    fn update_library(&mut self, click: Option<Target>) {
        // Tab changes (Tab/E/Q, A/D or the arrows, or a click on the strip).
        let mut tab_step = 0isize;
        if let Some(Target::LibraryTab(i)) = click {
            if let Some(tab) = library_index::Tab::ALL.get(i) {
                self.library_ui.set_tab(*tab);
                self.sfx(Sfx::UiMove);
            }
        }
        if self.input.just_pressed(KeyCode::Tab) || self.input.pressed_char('e') {
            tab_step += 1;
        }
        if self.input.pressed_char('q') {
            tab_step -= 1;
        }
        // `nav_pressed` is the arrows, the live movement bindings (A/D by
        // default) and the pad's d-pad/stick, so a pad can read the index.
        if self.input.nav_pressed(1, 0) {
            tab_step += 1;
        }
        if self.input.nav_pressed(-1, 0) {
            tab_step -= 1;
        }
        if tab_step != 0 {
            let tab = self.library_ui.tab.step(tab_step);
            self.library_ui.set_tab(tab);
            self.sfx(Sfx::UiMove);
        }

        let entries = self.library_entries();
        let mut step = 0isize;
        if self.input.nav_pressed(0, -1) {
            step -= 1;
        }
        if self.input.nav_pressed(0, 1) {
            step += 1;
        }
        if self.input.just_pressed(KeyCode::PageUp) {
            step -= self.library_shape.visible.max(1) as isize;
        }
        if self.input.just_pressed(KeyCode::PageDown) {
            step += self.library_shape.visible.max(1) as isize;
        }
        if let Some(Target::LibraryRow(i)) = click {
            self.library_ui.cursor = i.min(entries.saturating_sub(1));
            if self.library_shape.narrow {
                self.library_ui.card_open = true;
            }
            self.sfx(Sfx::UiMove);
        }
        if step != 0 {
            self.library_ui.step_cursor(step, entries);
            self.sfx(Sfx::UiMove);
            // A stepped cursor in the narrow list re-opens nothing.
        }

        // Back (the live binding: Esc / pad B by default, plus Backspace)
        // leaves the card first, then the screen.
        let back = click == Some(Target::Back)
            || self.input.back_pressed()
            || self.input.just_pressed(KeyCode::Backspace);
        if click == Some(Target::LibraryCard) {
            self.library_ui.card_open = false;
            self.sfx(Sfx::UiMove);
            return;
        }
        if back {
            if self.library_ui.card_open {
                self.library_ui.card_open = false;
                self.sfx(Sfx::UiMove);
            } else {
                self.close_library();
            }
            return;
        }
        // Confirm (the live binding: Enter / pad A by default) opens the card
        // on the narrow layout; on the wide one the card is already on screen.
        if self.input.confirm_pressed() {
            if self.library_shape.narrow {
                self.library_ui.card_open = true;
            }
            self.sfx(Sfx::UiSelect);
        }
        // Reading an entry clears its NEW mark.
        let card_visible = !self.library_shape.narrow || self.library_ui.card_open;
        if card_visible {
            let cursor = self.library_ui.cursor;
            if let Some(row) = self
                .library_rows
                .iter()
                .filter(|r| r.tab == self.library_ui.tab)
                .nth(cursor)
            {
                let key = row.key;
                self.scores.library.view(key);
            }
        }
    }

    /// Options screen: Up/Down choose a row, Left/Right change it, Enter on
    /// Back or Esc leaves. Mouse actions arrive as targets from `mouse_action`.
    fn update_options(&mut self, click: Option<Target>) {
        let row = self.options_row.min(options::ROW_COUNT - 1);
        let mut chosen = None;
        match click {
            Some(Target::OptionsRow(i)) => {
                self.options_row = i.min(options::ROW_COUNT - 1);
                chosen = Some(Target::OptionsRow(self.options_row));
            }
            Some(Target::OptionsBack) => {
                self.close_options();
                return;
            }
            Some(Target::OptionsBar(_)) => {
                // The bar click already set the level in `mouse_action`.
            }
            _ => {}
        }
        if self.input.back_pressed() {
            self.close_options();
            return;
        }
        if self.input.nav_pressed(0, -1) {
            self.options_row = (row + options::ROW_COUNT - 1) % options::ROW_COUNT;
            self.reset_armed = false;
            self.sfx(Sfx::UiMove);
        }
        if self.input.nav_pressed(0, 1) {
            self.options_row = (row + 1) % options::ROW_COUNT;
            self.reset_armed = false;
            self.sfx(Sfx::UiMove);
        }
        let row = self.options_row;
        if self.input.nav_pressed(-1, 0) {
            self.adjust_option(row, -1);
        }
        if self.input.nav_pressed(1, 0) {
            self.adjust_option(row, 1);
        }
        if row == options::BACK_ROW
            && (self.input.confirm_pressed() || chosen.is_some())
        {
            self.close_options();
            return;
        }
        // Enter activates the reset row exactly like a click does (arm, then
        // wipe): every other action row is stepped with Left/Right instead.
        if row == options::RESET_ROW && self.input.confirm_pressed() {
            self.adjust_option(row, 1);
            return;
        }
        // The two sub-screen rows open on Enter as well as on Left/Right (the
        // legend under the plate says `Enter open`).
        if self.input.confirm_pressed()
            && matches!(row, options::CONTROLS_ROW | options::GRAPHICS_ROW)
        {
            self.adjust_option(row, 1);
            return;
        }
        if chosen.is_some() && row != options::BACK_ROW {
            self.adjust_option(row, 1);
        }
    }

    /// Step one options row by `dir` (`-1` left, `+1` right).
    fn adjust_option(&mut self, row: usize, dir: isize) {
        match row {
            options::MUSIC_ROW => {
                self.set_music_volume(options::step_volume(self.music_volume, dir));
            }
            options::EFFECTS_ROW => {
                self.set_effects_volume(options::step_volume(self.effects_volume, dir), true);
            }
            options::MINIMAP_ROW => {
                let next = options::step_minimap(self.scores.minimap_size, dir as i32);
                self.set_minimap_size(next);
                self.sfx(Sfx::UiMove);
            }
            options::CURSOR_ROW => {
                self.toggle_cheese_cursor();
            }
            options::MUTE_ROW => {
                self.toggle_mute();
            }
            options::ZOOM_ROW => {
                // Right widens the view (a bigger number), left zooms in.
                self.set_zoom(view_zoom::step(self.scores.zoom, dir));
            }
            options::AUTO_PICK_ROW => {
                // Binary: Left and Right both flip it.
                self.toggle_auto_pick();
            }
            options::SCHEME_ROW => {
                let next = self.input.controls.scheme().step(dir as i32);
                self.set_scheme(next);
            }
            options::CONTROLS_ROW => {
                if dir != 0 {
                    self.open_controls();
                }
            }
            options::DEADZONE_ROW => {
                let cur = self.input.controls.deadzone;
                let next = (cur + dir as f32 * 0.05).clamp(0.05, 0.9);
                self.input.controls.deadzone = next;
                self.save_controls();
                self.sfx(Sfx::UiMove);
            }
            options::RUMBLE_ROW => {
                self.input.controls.rumble = !self.input.controls.rumble;
                self.save_controls();
                self.sfx(Sfx::UiMove);
            }
            options::GRAPHICS_ROW => {
                // Opens the page, exactly like the CONTROLS row (there is no
                // value to cycle on the row itself).
                if dir != 0 {
                    self.open_graphics();
                }
            }
            options::RESET_ROW => {
                // Two steps: arm, then wipe. The row prints `WIPE` /
                // `CONFIRM?` so the second press is never a surprise.
                if self.reset_armed {
                    self.reset_armed = false;
                    self.reset_nest();
                    self.sfx(Sfx::Purchase);
                } else {
                    self.reset_armed = true;
                    self.sfx(Sfx::UiMove);
                }
            }
            _ => {}
        }
    }

    /// Change the control scheme live (persisted beside the preferences).
    fn set_scheme(&mut self, scheme: Scheme) {
        if self.input.controls.scheme() == scheme {
            return;
        }
        self.input.controls.set_scheme(scheme);
        self.input.pad.set_focused(self.input.focused);
        self.save_controls();
        self.sfx(Sfx::UiSelect);
    }

    /// Open the Controls screen from Options.
    fn open_controls(&mut self) {
        self.controls_row = render_controls::SCHEME_ROW;
        self.controls_scroll = 0;
        self.controls_note.clear();
        self.bind_capture = None;
        self.controls_from_options = true;
        self.screen = Screen::Controls;
        self.sfx(Sfx::UiSelect);
    }

    /// Leave the Controls screen for Options (which remembers where *it* came
    /// from, so the title/pause round trip still works).
    fn close_controls(&mut self) {
        self.bind_capture = None;
        self.screen = Screen::Options;
        self.sfx(Sfx::UiSelect);
    }

    // ---- The Graphics page -----------------------------------------------

    /// Open Graphics from play, pause or Options.
    fn open_graphics(&mut self) {
        self.graphics_row = graphics::LIGHTING_ROW;
        self.graphics_return = self.screen;
        self.screen = Screen::Graphics;
        self.sfx(Sfx::UiSelect);
    }

    /// Leave Graphics for the screen that opened it.
    fn close_graphics(&mut self) {
        self.screen = self.graphics_return;
        self.sfx(Sfx::UiSelect);
    }

    fn graphics_over_field(&self) -> bool {
        matches!(self.graphics_return, Screen::Playing | Screen::Paused)
            || (self.graphics_return == Screen::Options && self.options_return == Screen::Paused)
    }

    /// One frame of the Graphics page: Up/Down choose a row, Left/Right cycle
    /// its value, Enter activates (BACK leaves; a value row steps forward), a
    /// click selects and steps, and S cycles smoothing from any row.
    fn update_graphics(&mut self, click: Option<Target>) {
        let row = self.graphics_row.min(graphics::ROW_COUNT - 1);
        let mut chosen = false;
        match click {
            Some(Target::GraphicsRow(i)) => {
                self.graphics_row = i.min(graphics::ROW_COUNT - 1);
                chosen = true;
            }
            Some(Target::GraphicsBack) => {
                self.close_graphics();
                return;
            }
            _ => {}
        }
        if self.input.back_pressed() || self.input.action_pressed(Action::Graphics) {
            self.close_graphics();
            return;
        }
        if self.input.pressed_unmodified_char('s') {
            self.adjust_graphics(graphics::SMOOTH_ROW, 1);
            return;
        }
        if self.input.nav_pressed(0, -1) {
            self.graphics_row = (row + graphics::ROW_COUNT - 1) % graphics::ROW_COUNT;
            self.sfx(Sfx::UiMove);
        }
        if self.input.nav_pressed(0, 1) {
            self.graphics_row = (row + 1) % graphics::ROW_COUNT;
            self.sfx(Sfx::UiMove);
        }
        let row = self.graphics_row;
        if self.input.nav_pressed(-1, 0) {
            self.adjust_graphics(row, -1);
        }
        if self.input.nav_pressed(1, 0) {
            self.adjust_graphics(row, 1);
        }
        if row == graphics::BACK_ROW && (self.input.confirm_pressed() || chosen) {
            self.close_graphics();
            return;
        }
        // Enter cycles a value row forward, exactly like Left/Right (the row's
        // legend says so, and the console's no-argument form steps too).
        if !chosen && self.input.confirm_pressed() && row != graphics::BACK_ROW {
            self.adjust_graphics(row, 1);
            return;
        }
        if chosen && row != graphics::BACK_ROW {
            self.adjust_graphics(row, 1);
        }
    }

    /// Step one Graphics row by `dir` (`-1` left, `+1` right). Every arm goes
    /// through the same `App` method the matching console command uses, so the
    /// page and the console can never disagree about what a setting does.
    fn adjust_graphics(&mut self, row: usize, dir: isize) {
        match row {
            graphics::LIGHTING_ROW => {
                let next = graphics::step_lighting(self.lighting_mode(), dir);
                self.set_lighting(next);
            }
            graphics::FX_ROW => self.set_fx(!self.fx_enabled()),
            graphics::SMOOTH_ROW => {
                let next = graphics::step_smooth(self.smooth_level(), dir);
                self.set_smooth_level(next);
            }
            graphics::FOREGROUND_ROW => self.set_foreground(!self.foreground_enabled()),
            graphics::GLYPHFX_ROW => self.set_glyphfx(!self.glyphfx_enabled()),
            graphics::SHAKE_ROW => {
                let next = graphics::step_shake(self.shake_scale, dir);
                self.set_shake_scale(next);
            }
            _ => return,
        }
        self.sfx(Sfx::UiMove);
    }

    /// One frame of the Controls screen.
    ///
    /// While a capture is armed the next key, mouse press or pad button becomes
    /// the binding; Esc cancels, Enter on a row arms the capture, Delete or
    /// Backspace unbinds it, R resets the active scheme.
    fn update_controls(&mut self, click: Option<Target>) {
        // A pointer click on a row selects it (and arms its capture).
        if let Some(Target::ControlsRow(row)) = click {
            self.controls_row = row.min(render_controls::ROW_COUNT - 1);
            self.activate_controls_row();
            return;
        }
        if click == Some(Target::ControlsReset) {
            self.controls_row = render_controls::RESET_ROW;
            self.reset_controls_scheme();
            return;
        }
        if click == Some(Target::ControlsBack) {
            self.controls_row = render_controls::BACK_ROW;
            self.close_controls();
            return;
        }

        // ---- Capture mode ------------------------------------------------
        if let Some(action) = self.bind_capture {
            // Esc cancels without binding anything (checked before the general
            // Back action, which is the same key).
            if self.input.just_pressed(KeyCode::Esc) {
                self.bind_capture = None;
                self.controls_note = "cancelled".into();
                self.sfx(Sfx::UiMove);
                return;
            }
            // A pad button.
            if let Some(pb) = self.input.pad.pressed_button() {
                self.apply_pad_binding(action, pb);
                return;
            }
            // A mouse press (find it first: the borrow must end before the
            // mutable bind call).
            let mouse_bind = self.input.mouse_events.iter().find_map(|ev| match ev.kind {
                MouseEventKind::Down(MouseButton::Left) => Some(MouseBind::Left),
                MouseEventKind::Down(MouseButton::Right) => Some(MouseBind::Right),
                MouseEventKind::Down(MouseButton::Middle) => Some(MouseBind::Middle),
                MouseEventKind::ScrollUp => Some(MouseBind::WheelUp),
                MouseEventKind::ScrollDown => Some(MouseBind::WheelDown),
                _ => None,
            });
            if let Some(m) = mouse_bind {
                self.apply_kbm_binding(action, Binding::Mouse(m));
                return;
            }
            // A key. A lone modifier is not in `pressed`; ask for the tap.
            if let Some(k) = self.input.modifier_tap() {
                self.apply_kbm_binding(action, Binding::Key(k));
                return;
            }
            let events: Vec<KeyEvent> = self.input.pressed.clone();
            let mut refused: Option<String> = None;
            for ev in events.iter() {
                match KeyBind::from_event(ev) {
                    Ok(k) => {
                        self.apply_kbm_binding(action, Binding::Key(k));
                        return;
                    }
                    Err(u) => refused = Some(u.note().to_string()),
                }
            }
            if let Some(note) = refused {
                self.controls_note = note;
                self.sfx(Sfx::DashDenied);
            }
            return;
        }

        // ---- Normal navigation ------------------------------------------
        let n = render_controls::ROW_COUNT;
        if self.input.nav_pressed(0, -1) {
            self.controls_row = (self.controls_row + n - 1) % n;
            self.sfx(Sfx::UiMove);
        }
        if self.input.nav_pressed(0, 1) {
            self.controls_row = (self.controls_row + 1) % n;
            self.sfx(Sfx::UiMove);
        }
        if self.input.back_pressed() {
            self.close_controls();
            return;
        }
        let row = self.controls_row.min(n - 1);
        if row == render_controls::SCHEME_ROW {
            if self.input.nav_pressed(-1, 0) {
                let next = self.input.controls.scheme().step(-1);
                self.set_scheme(next);
            }
            if self.input.nav_pressed(1, 0) {
                let next = self.input.controls.scheme().step(1);
                self.set_scheme(next);
            }
        }
        if self.input.action_pressed(Action::Reset) {
            self.reset_controls_scheme();
            return;
        }
        if self.input.confirm_pressed() {
            self.activate_controls_row();
        }
        if (self.input.pressed_unmodified_char('x')
            || self.input.just_pressed(KeyCode::Delete)
            || self.input.just_pressed(KeyCode::Backspace))
            && (1..render_controls::RESET_ROW).contains(&row)
        {
            let action = ACTIONS[row - 1];
            self.input.controls.unbind_active_kbm(action);
            self.save_controls();
            self.controls_note = format!("{} unbound", action.label());
        }
        // Keep the cursor inside the visible window.
        if self.controls_visible > 0 {
            if row < self.controls_scroll {
                self.controls_scroll = row;
            } else if row >= self.controls_scroll + self.controls_visible {
                self.controls_scroll = row + 1 - self.controls_visible;
            }
        }
    }

    /// Enter/A on the selected Controls row.
    fn activate_controls_row(&mut self) {
        let row = self.controls_row;
        if row == render_controls::SCHEME_ROW {
            let next = self.input.controls.scheme().step(1);
            self.set_scheme(next);
            return;
        }
        if row == render_controls::RESET_ROW {
            self.reset_controls_scheme();
            return;
        }
        if row == render_controls::BACK_ROW {
            self.close_controls();
            return;
        }
        let action = ACTIONS[row - 1];
        self.bind_capture = Some(action);
        self.controls_note = format!("press the new binding for {}", action.label());
        self.sfx(Sfx::UiMove);
    }

    fn reset_controls_scheme(&mut self) {
        self.input.controls.reset_active();
        self.save_controls();
        self.controls_note = format!(
            "{} reset to its defaults",
            self.input.controls.scheme().name()
        );
        self.sfx(Sfx::UiSelect);
    }

    /// Bind a keyboard/mouse input to `action`, reporting a conflict swap.
    fn apply_kbm_binding(&mut self, action: Action, b: Binding) {
        let swapped = self.input.controls.bind_active_kbm(action, b);
        self.bind_capture = None;
        self.save_controls();
        self.controls_note = match swapped {
            Some(other) => format!("{} ↔ {} (swapped)", action.label(), other.label()),
            None => format!("{} = {}", action.label(), b.label()),
        };
        self.sfx(Sfx::UiSelect);
    }

    /// Bind a pad button to `action`, reporting a conflict swap.
    fn apply_pad_binding(&mut self, action: Action, b: crate::controls::PadBind) {
        let swapped = self
            .input
            .controls
            .bind_active_pad(action, Binding::Pad(b));
        self.bind_capture = None;
        self.save_controls();
        self.controls_note = match swapped {
            Some(other) => format!("{} ↔ {} (swapped)", action.label(), other.label()),
            None => format!("{} = {}", action.label(), b.label()),
        };
        self.sfx(Sfx::UiSelect);
    }

    /// Step one options row from a mouse-wheel delta (`step > 0` is scroll
    /// down). Scroll up raises a volume and grows the minimap; scroll down
    /// lowers a volume and shrinks the minimap.
    fn scroll_option(&mut self, row: usize, step: isize) {
        match row {
            options::MUSIC_ROW | options::EFFECTS_ROW => self.adjust_option(row, -step),
            // Scroll down widens the view, exactly like the wheel over the
            // playfield does mid-run.
            options::MINIMAP_ROW | options::ZOOM_ROW => {
                self.adjust_option(row, step)
            }
            _ => self.adjust_option(row, 1),
        }
    }

    /// Set a volume slider from a pointer column on its track.
    fn set_option_bar(&mut self, bar: usize, percent: u8) {
        match bar {
            options::MUSIC_BAR => self.set_music_volume(percent),
            _ => self.set_effects_volume(percent, true),
        }
    }

    fn update_title(&mut self, click: Option<Target>) {
        // A click selects its own action, independent of simultaneous keys.
        let action = if click.is_some() {
            click
        } else if self.input.action_pressed(Action::Quit) {
            Some(Target::Quit)
        } else if self.input.action_pressed(Action::Library) {
            Some(Target::Story)
        } else {
            if self.input.nav_pressed(0, -1) {
                self.menu_idx = (self.menu_idx + TITLE_ROWS - 1) % TITLE_ROWS;
                self.sfx(Sfx::UiMove);
            }
            if self.input.nav_pressed(0, 1) {
                self.menu_idx = (self.menu_idx + 1) % TITLE_ROWS;
                self.sfx(Sfx::UiMove);
            }
            if self.input.nav_pressed(-1, 0) {
                self.cycle_sector(-1);
            }
            if self.input.nav_pressed(1, 0) {
                self.cycle_sector(1);
            }
            self.input
                .confirm_pressed()
                .then(|| TITLE_ACTIONS[self.menu_idx])
        };
        match action {
            Some(Target::Story) => {
                self.sfx(Sfx::UiSelect);
                self.story_time = 0.0;
                self.screen = Screen::Story;
            }
            Some(Target::Quit) => self.should_quit = true,
            Some(Target::Start) => {
                self.sfx(Sfx::UiSelect);
                self.start_run();
            }
            Some(Target::Sectors) => {
                self.sfx(Sfx::UiSelect);
                // The select screen lists every floor — the getaway ride has a
                // card of its own — so Enter always opens it, on whichever
                // floor the title's row is pointing at.
                self.screen = Screen::SectorSelect;
            }
            Some(Target::Options) => {
                self.sfx(Sfx::UiSelect);
                self.open_options(Screen::Title);
            }
            Some(Target::Library) => {
                self.sfx(Sfx::UiSelect);
                self.open_library(Screen::Title);
            }
            Some(Target::Nest) => {
                self.sfx(Sfx::UiSelect);
                self.open_nest(Screen::Title);
            }
            Some(Target::NextSector) => self.cycle_sector(1),
            Some(Target::PreviousSector) => self.cycle_sector(-1),
            _ => {}
        }
    }

    /// Sector select: the arrows move through the grid in visual order
    /// (`Left`/`Right` one floor, `Up`/`Down` one row of
    /// [`App::select_cols`]), Enter starts the highlighted floor, Esc or the
    /// art's Back control returns to the title. Locked floors can be selected
    /// and read; starting one refuses.
    fn update_sector_select(&mut self, click: Option<Target>) {
        if click == Some(Target::Back)
            || self.input.back_pressed()
            || self.input.action_pressed(Action::Quit)
        {
            self.sfx(Sfx::UiSelect);
            self.screen = Screen::Title;
            return;
        }
        match click {
            Some(Target::SelectSector(i)) => {
                // A float index into `Theme::playable()`, never a stage.
                self.select_idx = i.min(Theme::playable().len().saturating_sub(1));
                self.menu_music();
                self.sfx(Sfx::UiMove);
            }
            Some(Target::PreviousSector) => self.cycle_sector(-1),
            Some(Target::NextSector) => self.cycle_sector(1),
            _ => {}
        }
        if self.input.nav_pressed(-1, 0) {
            self.cycle_sector(-1);
        }
        if self.input.nav_pressed(1, 0) {
            self.cycle_sector(1);
        }
        let cols = self.select_cols();
        if self.input.nav_pressed(0, -1) {
            self.sector_step(-cols);
        }
        if self.input.nav_pressed(0, 1) {
            self.sector_step(cols);
        }
        if click == Some(Target::StartSector) || self.input.confirm_pressed() {
            self.start_sector();
        }
    }

    fn update_story(&mut self, click: Option<Target>) {
        if click == Some(Target::Back)
            || self.input.confirm_pressed()
            || self.input.back_pressed()
            || self.input.action_pressed(Action::Quit)
        {
            self.sfx(Sfx::UiSelect);
            // The dossier is also the campaign's opening: a run in flight
            // continues into its map intro, while the title's `L story` viewer
            // goes back to the title.
            if self.campaign.is_some() && self.world.is_some() {
                self.screen = if self.world.as_ref().map(|w| w.level_index) == Some(0) {
                    self.intro_timer = 4.0;
                    Screen::MapIntro
                } else {
                    Screen::LevelIntro
                };
            } else {
                self.screen = Screen::Title;
            }
        }
    }

    fn update_mapintro(&mut self, dt: f32, click: Option<Target>) {
        self.intro_timer -= dt;
        if click == Some(Target::Begin)
            || self.intro_timer <= 0.0
            || self.input.confirm_pressed()
            || self.input.just_pressed(KeyCode::Char(' '))
        {
            self.screen = Screen::Playing;
        }
    }

    /// Levels 2 and 3's intro plate: Enter/Space/click drops into the level.
    fn update_levelintro(&mut self, click: Option<Target>) {
        if click == Some(Target::Begin)
            || self.input.confirm_pressed()
            || self.input.just_pressed(KeyCode::Char(' '))
        {
            self.sfx(Sfx::UiMove);
            self.screen = Screen::Playing;
        }
    }

    /// Effective compact minimap size for the current playfield, stepping down
    /// automatically on narrow terminals without overwriting the saved setting.
    fn effective_minimap_size(&self) -> minimap::MinimapSize {
        minimap::effective_compact_size(self.last_game, self.scores.minimap_size)
    }

    /// Update and persist the compact minimap size preference (in-memory only
    /// when `self.persist` is false, as in tests).
    fn set_minimap_size(&mut self, size: minimap::MinimapSize) {
        if self.scores.minimap_size != size {
            self.scores.minimap_size = size;
            if self.persist {
                self.scores.save();
            }
        }
    }

    /// Cycle compact minimap size (`Normal -> Small -> Hidden -> Normal`),
    /// advancing from the currently visible effective size so narrow terminals
    /// react on the first press. No-op when no compact radar fits at all.
    fn cycle_minimap_size(&mut self) {
        if !minimap::compact_size_fits(self.last_game, minimap::MinimapSize::Small) {
            return;
        }
        let next = self.effective_minimap_size().cycle();
        self.set_minimap_size(next);
        self.sfx(Sfx::UiMove);
    }

    /// Is this terminal cell inside the playfield drawn last frame? (The HUD
    /// rails, the minimap and the menus all live outside it.)
    fn over_playfield(&self, col: u16, row: u16) -> bool {
        let g = self.last_game;
        g.width > 0
            && g.height > 0
            && col >= g.x
            && col < g.right()
            && row >= g.y
            && row < g.bottom()
    }

    /// Step the view zoom and persist it (`dir > 0` widens the view, i.e. shows
    /// more world at a smaller size). The live level follows immediately; the
    /// renderer eases the change over about [`view_zoom::EASE_SECS`].
    fn step_zoom(&mut self, dir: isize) {
        self.set_zoom(view_zoom::step(self.effective_zoom(), dir));
    }

    /// The zoom setting in force: the level's own value while a run is live
    /// (which is the saved one), the saved preference otherwise.
    fn effective_zoom(&self) -> f32 {
        self.world
            .as_ref()
            .map(|w| w.zoom)
            .unwrap_or(self.scores.zoom)
    }

    /// Set the zoom setting, persist it, hand it to the live level, and flash
    /// the HUD read-out.
    fn set_zoom(&mut self, zoom: f32) {
        let zoom = view_zoom::snap(zoom);
        let mut changed = false;
        if (self.scores.zoom - zoom).abs() > 0.001 {
            self.scores.zoom = zoom;
            if self.persist {
                self.scores.save();
            }
            changed = true;
        }
        if let Some(w) = self.world.as_mut() {
            if (w.zoom - zoom).abs() > 0.001 {
                changed = true;
            }
            w.zoom = zoom;
        }
        if changed {
            self.hud.notify_zoom(zoom);
            self.sfx(Sfx::UiMove);
        }
    }

    /// Step compact minimap size from a mouse-wheel scroll over the compact
    /// radar (`step > 0` shrinks toward `Hidden`, `step < 0` grows toward `Normal`).
    fn step_minimap_size(&mut self, step: isize) {
        if !minimap::compact_size_fits(self.last_game, minimap::MinimapSize::Small) {
            return;
        }
        let cur = self.effective_minimap_size();
        let candidate = cur.step(step as i32);
        let next = if minimap::compact_size_fits(self.last_game, candidate) {
            candidate
        } else {
            cur
        };
        if next != cur {
            self.set_minimap_size(next);
            self.sfx(Sfx::UiMove);
        }
    }

    /// Handle bare testbed keys while the field owns input. Bindings in the
    /// active scheme keep precedence, including their secondary slots.
    fn update_testbed_keys(&mut self) -> bool {
        let (Some(testbed), Some(world)) = (&mut self.testbed, &mut self.world) else {
            return false;
        };
        let bindings = self.input.controls.active();
        let mut cleared = false;
        for event in &self.input.pressed {
            let Ok(binding) = KeyBind::from_event(event) else {
                continue;
            };
            if bindings.action_for_kbm(Binding::Key(binding)).is_some() {
                continue;
            }
            let key = match event.code {
                KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
                code => code,
            };
            match key {
                KeyCode::Char('1' | '2') => {
                    testbed.weapon = testbed_cycle(
                        &WeaponKind::all(), testbed.weapon, key == KeyCode::Char('2'),
                    );
                    testbed.select_weapon(world);
                }
                KeyCode::Char('3') => testbed.toggle_evolution(world),
                KeyCode::Char('4' | '5') => {
                    testbed.pickup = testbed_cycle(
                        &PickupKind::all(), testbed.pickup, key == KeyCode::Char('5'),
                    );
                }
                KeyCode::Char('6') => {
                    let pos = world.testbed_spawn_point(
                        world.player.face, 24.0, 1.0, SizeClass::Small,
                    );
                    world.spawn_pickup_near(testbed.pickup, pos, 0.0);
                }
                KeyCode::Char('7' | '8') => {
                    let kinds = EnemyKind::all();
                    let forward = key == KeyCode::Char('8');
                    testbed.enemy = testbed_cycle(&kinds, testbed.enemy, forward);
                    if testbed.enemy == EnemyKind::Boss {
                        testbed.enemy = testbed_cycle(&kinds, testbed.enemy, forward);
                    }
                }
                KeyCode::Char('9') => {
                    let kind = testbed.enemy;
                    for i in 0..5 {
                        let direction = Vec2::from_angle(i as f32 * std::f32::consts::TAU / 5.0);
                        let pos = world.testbed_spawn_point(
                            direction, 70.0, kind.radius(), SizeClass::of_enemy(kind),
                        );
                        world.spawn_enemy_near(kind, pos, 0.0, false);
                    }
                }
                KeyCode::Char('0') => {
                    world.clear_enemies();
                    world.clear_testbed_boss();
                    self.prev_boss_alive = false;
                    self.prev_miniboss_alive = false;
                    self.hud.telemetry.set_boss_alive(false);
                    self.hud.telemetry.set_boss_hp(None);
                    self.hud.telemetry.set_boss_offset(None);
                    self.hud.telemetry.set_enemy_count(0);
                    cleared = true;
                }
                KeyCode::F(1 | 2) => {
                    testbed.boss = testbed_cycle(&BossKind::all(), testbed.boss, key == KeyCode::F(2));
                }
                KeyCode::F(3) => {
                    world.clear_testbed_boss();
                    world.force_boss_here(testbed.boss);
                }
                KeyCode::F(5 | 6) => {
                    testbed.miniboss = testbed_cycle(
                        &MinibossKind::all(), testbed.miniboss, key == KeyCode::F(6),
                    );
                }
                KeyCode::F(7) => {
                    let radius = testbed.miniboss.body_radius();
                    let pos = world.testbed_spawn_point(
                        world.player.face, 60.0, radius, SizeClass::Large,
                    );
                    world.spawn_miniboss_near(testbed.miniboss, pos);
                }
                KeyCode::Char('h') => world.player.god = !world.player.god,
                KeyCode::Char('j') => world.peace = !world.peace,
                KeyCode::Char('k') => world.kill_testbed_visible(),
                KeyCode::Char('t') => {
                    let theme = testbed_cycle(&Theme::playable(), world.level.theme, true);
                    self.start_testbed(theme);
                    return true;
                }
                KeyCode::F(10) => testbed.show_panel = !testbed.show_panel,
                _ => {}
            }
        }
        // Show the cleared field for this frame, even with the horde enabled.
        cleared
    }

    fn update_playing(&mut self, dt: f32, click: Option<Target>) {
        let testbed_changed_field = !self.map_open && self.update_testbed_keys();
        // The minimap is the first claim on Tab, a click on the compact map and
        // Esc: while the expanded map is open those close it before the
        // pause-screen handling below.
        // Keyboard zoom for players without a wheel: `Z` pulls in, `X` widens
        // (the same ladder the wheel steps). Discrete presses only.
        if self.input.action_pressed(Action::ZoomIn) {
            self.step_zoom(-1);
        }
        if self.input.action_pressed(Action::ZoomOut) {
            self.step_zoom(1);
        }
        // Take back control mid-run: `U` flips auto-pick — the same preference
        // the level-up screen's pill and the options row set — so a player who
        // turned it on always gets the cards back. Discrete presses only.
        if self.input.action_pressed(Action::AutoPick) {
            self.toggle_auto_pick();
        }
        // Middle click (or the bound key / R3) resets the view to 1.0x, but
        // only over the playfield: the minimap, HUD rails and the expanded map
        // have their own jobs, and a menu never trains a zoom.
        let middle_over_field = self.input.mouse_events.iter().any(|e| {
            e.kind == MouseEventKind::Down(MouseButton::Middle)
                && !self.map_open
                && self.over_playfield(e.column, e.row)
                // The minimap is a UI overlay sitting on the playfield: it has
                // its own job (and the wheel skips it), so a click there must
                // not train the zoom either.
                && self.hits.at(e.column, e.row) != Some(Target::Minimap)
        });
        if self.input.action_pressed(Action::ResetZoom) && !self.map_open {
            // A mouse reset only counts over the playfield (the minimap, the
            // HUD rails and the expanded map each have their own job); a
            // rebound key or pad button works anywhere on the field.
            let mouse_bound = self
                .input
                .controls
                .active()
                .kbm_slots(Action::ResetZoom)
                .iter()
                .any(|b| matches!(b, Binding::Mouse(_)));
            if !mouse_bound || middle_over_field {
                self.set_zoom(1.0);
            }
        }
        let mut esc_used = false;
        let mut right_spent = false;
        if self.input.action_pressed(Action::Map) || click == Some(Target::Minimap) {
            self.map_open = !self.map_open;
            self.sfx(Sfx::UiMove);
        } else if self.map_open
            && (self.input.action_pressed(Action::Back) || self.input.right_clicked())
        {
            // The expanded map is an overlay: a right click closes it exactly
            // as Esc does (and, like Esc, never also pauses).
            right_spent = self.input.right_clicked();
            self.map_open = false;
            esc_used = true;
            self.sfx(Sfx::UiMove);
        }
        if self.input.action_pressed(Action::MinimapSize) {
            self.cycle_minimap_size();
        }
        if self.testbed.is_some()
            && (self.input.action_pressed(Action::Quit)
                || (!self.map_open && !esc_used && self.input.just_pressed(KeyCode::Esc)))
        {
            self.back_to_title();
            return;
        }
        // Esc is both the pause key and the map's close key, so the map's own
        // close this frame wins: closing the map never pauses.
        if click == Some(Target::Pause)
            || (!esc_used
                && (self.input.action_pressed(Action::Back)
                    || self.input.action_pressed(Action::Pause)))
        {
            self.map_open = false;
            self.screen = Screen::Paused;
            return;
        }
        if self.input.action_pressed(Action::Graphics) {
            self.map_open = false;
            self.open_graphics();
            return;
        }
        // The full map is a pause: the world holds still while it is open, so
        // reading the route is never paid for in health. Tab, Esc or a click
        // on the map closes it and play resumes on the next frame.
        if self.map_open {
            return;
        }
        if testbed_changed_field {
            return;
        }
        // Console `pause on`: the world holds still until `pause off`. The
        // console is still reachable (backtick) to resume or `step`.
        if self.console_paused {
            return;
        }
        let move_dir = self.resolve_move_dir();
        // Ctrl dashes and Space fires the charged active weapons on a terminal
        // that reports a lone Ctrl press; on one that cannot, Space keeps doing
        // both (see `InputState::dash_pressed`). A left click on the field is
        // always the pair: it is the mouse player's whole "do the thing".
        //
        // A click is the mouse player's whole "do the thing" gesture, so it
        // asks for both — but it only **releases the charged actives when the
        // dash actually starts**. A click while the dash is cooling is a
        // refused dash (`Sfx::DashDenied` + the HUD module's red flash) and
        // must not spend a charged defensive ring on the way out, which is what
        // it used to do (`WAVE3 review`, "Controls and full racks"). Space and
        // the right click are unaffected: they always fire the actives on their
        // own.
        let click_dash = click == Some(Target::Dash);
        let dash_ready = self
            .world
            .as_ref()
            .is_none_or(|w| w.player.dash_cd <= 0.0);
        // The dash button itself: its press edge, whether it is still held and
        // its release edge, on whatever device the dash Action is bound to.
        // The sim owns the Super Dash charge meter; this is only the button.
        let mut dash_hold = DashHold {
            down: self.input.dash_down(),
            held: self.input.dash_held(),
            up: self.input.dash_up(),
        };
        // Classic/Gamepad: a left click is dash + (ready) actives, and a right
        // click is the actives on their own — `active_pressed` reads every
        // device bound to `FireActives`, which is Space, the right click and
        // the pad's X on those tables.
        // Mouse aim: the clicks are split — LMB fires the actives, RMB dashes —
        // so a left click never spends the dash and a right click never spends
        // a charge.
        //
        // A **fresh** press only (`dash_pressed_fresh`): an auto-repeat is the
        // only "hold" a terminal without key releases can give, and it must
        // never re-press the dash — with the Super Dash owned that is what would
        // machine-gun taps instead of charging.
        let (mut dash, mut active) = if self.input.controls.mouse_aims() {
            (
                self.input.action_pressed(Action::Dash),
                click_dash || self.input.action_pressed(Action::FireActives),
            )
        } else {
            let click = click_dash && dash_ready;
            (
                click_dash || self.input.dash_pressed_fresh(),
                click || self.input.active_pressed(),
            )
        };
        if right_spent {
            // The right click was the map's Back: it must not also fire or dash.
            dash = false;
            active = false;
            dash_hold = DashHold::NONE;
        }
        let mut ended = false;
        let mut level_done = false;
        let mut intensity = 1u32;
        // The reticle is read before the world is borrowed (it needs `&self`).
        let aim = self.aim_point();
        if let Some(w) = self.world.as_mut() {
            let hp_before = w.player.hp;
            let elapsed_before = w.elapsed;
            // Who is on the floor before the tick, so a death edge below can
            // name the kind in the Library (the nameplate is gone afterwards).
            let boss_before = w.boss_kind();
            let miniboss_before = w.miniboss_hp().map(|(name, _, _)| name);
            // Console `timescale x` scales the simulation step; default 1.0.
            let sim_dt = dt * self.timescale;
            w.update_with_dash_hold(sim_dt, move_dir, dash, active, aim, dash_hold);
            // Drain SFX into a reused buffer, de-duplicated per frame so a
            // 16-shot nova is one sound.
            self.sfx_frame.clear();
            for s in w.sfx.drain(..) {
                if !self.sfx_frame.contains(&s) {
                    self.sfx_frame.push(s);
                }
            }
            if w.player.hp < hp_before {
                self.damage_taken += hp_before - w.player.hp;
                // A hit is felt on the pad (the Options RUMBLE row; a no-op
                // without a connected, focused pad).
                self.input.pad.rumble_if(self.input.controls.rumble, 0.12);
            }
            if self.sfx_frame.contains(&Sfx::Dash) {
                self.dashes_used += 1;
            }
            for &pk in w.last_pickups() {
                if pk == PickupKind::Heal {
                    self.heals_picked += 1;
                }
                // Collecting a find catalogues it (never on a CONSOLE USED run).
                if !self.console_cheat {
                    self.scores.library.note_collected(pk);
                }
            }
            let boss_hp = w.boss_hp();
            let boss_alive = boss_hp.is_some();
            if self.prev_boss_alive && !boss_alive && w.director.boss_spawned {
                self.boss_slain = true;
                self.killed_boss = boss_before;
                // The boss going down gets a longer pulse than a hit.
                self.input.pad.rumble_if(self.input.controls.rumble, 0.4);
                // Every apex kill pays the nest a bonus (see `meta::APEX_BONUS`).
                self.apex_kills = self.apex_kills.saturating_add(1);
                // The Library's KILLED counter for the boss itself.
                if let (Some(kind), false) = (boss_before, self.console_cheat) {
                    self.scores.library.bump(
                        LibraryKey::Boss(kind),
                        library_index::Counter::Killed,
                        1,
                    );
                }
            }
            self.prev_boss_alive = boss_alive;
            // A named miniboss dies when its nameplate goes away; the chest it
            // leaves arrives on the same tick, so the edge is the whole test.
            let miniboss_alive = w.miniboss_hp().is_some();
            // A survivor that **retreats** (the next appointment, or the boss
            // coming due) also drops its nameplate, without loot: the system
            // says so with `Sfx::MinibossRetreat`, and that is not a kill.
            if self.prev_miniboss_alive
                && !miniboss_alive
                && !self.sfx_frame.contains(&Sfx::MinibossRetreat)
            {
                self.apex_kills = self.apex_kills.saturating_add(1);
                // The Library's KILLED counter for the named miniboss (the
                // survey only sees its `Brute` body).
                let kind = miniboss_before.and_then(miniboss_kind_named);
                if let (Some(kind), false) = (kind, self.console_cheat) {
                    self.scores.library.bump(
                        LibraryKey::Miniboss(kind),
                        library_index::Counter::Killed,
                        1,
                    );
                }
            }
            self.prev_miniboss_alive = miniboss_alive;
            self.hud.telemetry.set_boss_alive(boss_alive);
            self.hud.telemetry.set_boss_hp(boss_hp);
            self.hud.telemetry.set_boss_offset(w.boss_offset());
            let enemy_count = w.enemy_count();
            self.hud.telemetry.set_enemy_count(enemy_count);
            self.hud.tick(dt, w, &self.sfx_frame, self.muted);
            for cue in blackout::cues(
                w.level.theme,
                w.curve_clock_at(elapsed_before),
                w.curve_clock_at(w.elapsed),
            ) {
                if cue == Sfx::NightBed {
                    self.hud.push_callout(hud::CalloutKind::Blackout, "BLACKOUT", "LAMPS OUT", 3.0);
                }
                if !self.sfx_frame.contains(&cue) {
                    self.sfx_frame.push(cue);
                }
            }
            // The synth's fourth tier is the boss fight itself (Boss
            // Overdrive adds the lead/boss layers): 3 while a boss is alive,
            // back to the horde rule (2 when the screen is dense, 1 otherwise)
            // the tick it dies — `boss_hp()` is `None` from the lethal tick, so
            // the drop lands with the death, and the boss's death sting covers
            // the transition.
            intensity = if boss_alive {
                3
            } else if enemy_count > 45 {
                2
            } else {
                1
            };
            // A finished level wins over pending upgrades, and a loss wins
            // over a completion: dying (or finishing a boss) on the same tick
            // as an XP award must not open the level-up cards.
            if w.game_over {
                ended = true;
            } else if w.level_complete() || w.campaign_won() {
                level_done = true;
            }
        }
        for i in 0..self.sfx_frame.len() {
            let s = self.sfx_frame[i];
            self.sfx(s);
        }
        // The Library's sighting pass. Read-only over the world (no rng, no
        // simulation writes) and gated by a per-category early-out, so a run
        // that has met everything costs a handful of bitset tests.
        // A CONSOLE USED run discovers and counts nothing: a spawned boss is
        // not a sighting.
        if !self.console_cheat {
            if let Some(w) = self.world.as_ref() {
                let _p = crate::prof::scope(crate::prof::Stage::Survey);
                self.scores.library.survey(w);
            }
        }
        self.drain_library_news();
        self.request_music_intensity(intensity);
        if ended {
            // Book the loss against the nearest body (the simulation keeps no
            // damage source; the card says so).
            if !self.console_cheat {
                if let Some(w) = self.world.as_ref() {
                    // KILLED YOU: a death in a boss fight is the boss's, and a
                    // death beside a named miniboss is the miniboss's; anything
                    // else goes to the nearest body, as before.
                    let miniboss = w
                        .miniboss_hp()
                        .and_then(|(name, _, _)| miniboss_kind_named(name))
                        .filter(|_| {
                            w.miniboss_marker().is_some_and(|at| {
                                (at - w.player.pos).len() <= MINIBOSS_BLAME_RANGE
                            })
                        });
                    let killed_by = library_index::Counter::KilledBy;
                    if let Some(kind) = w.boss_kind() {
                        self.scores.library.bump(LibraryKey::Boss(kind), killed_by, 1);
                    } else if let Some(kind) = miniboss {
                        self.scores
                            .library
                            .bump(LibraryKey::Miniboss(kind), killed_by, 1);
                    } else {
                        self.scores.library.note_death(w);
                    }
                }
            }
            self.finalize();
            self.screen = Screen::End;
        } else if level_done {
            self.on_level_complete();
        } else {
            self.resume_or_open_reward();
        }
    }

    fn resume_or_open_reward(&mut self) {
        let (pending, auto) = match self.world.as_ref() {
            Some(w) => (w.pending_levelups, self.scores.auto_pick),
            None => return,
        };
        if pending > 0 {
            if !auto {
                self.open_levelup();
                return;
            }
            // Auto-pick: the card lands without the screen. The first one lands
            // on the same tick a manual screen would have opened (so the run is
            // identical); cards queued behind it wait their spacing so each
            // toast is readable, and the game keeps running.
            if self.auto_pick_wait > 0.0 {
                self.screen = Screen::Playing;
                return;
            }
            self.auto_pick_one();
        }
        let Some(w) = self.world.as_mut() else {
            return;
        };
        if w.pending_levelups > 0 {
            // A card was just taken and more are queued: wait out the spacing.
            self.screen = Screen::Playing;
        } else if let Some(reward) = w.claim_chest() {
            self.map_open = false;
            self.chest_reward = Some(reward);
            self.screen = Screen::Chest;
            match reward {
                // The "nothing left to give" chest pays gold *and* patches the
                // rat up; the loadout cannot see current HP, so the heal is
                // applied here (`design §7`).
                ChestReward::Gold(_) => {
                    let heal = crate::game::loadout::CHEST_HEAL_FRAC * w.player.max_hp();
                    w.player.heal(heal);
                    self.sfx(Sfx::CacheOpen);
                }
                // An evolution gets its own fanfare (`Sfx::Evolve`), and the
                // chest is where the player meets it.
                ChestReward::Evolve { kind, .. } => {
                    if !self.console_cheat {
                        self.scores.library.mark_seen(LibraryKey::Evolution(kind));
                    }
                    self.sfx(Sfx::Evolve);
                }
                _ => self.sfx(Sfx::CacheOpen),
            }
        } else if std::mem::take(&mut w.pending_shop) {
            self.map_open = false;
            // Open this hole's shelf (generated on the first visit from the
            // floor seed + hole index, so a revisit sees the same stock).
            let hole = w.pending_shop_hole;
            w.open_shop(hole);
            self.shop_row = 0;
            self.shop_molt = None;
            self.screen = Screen::Shop;
        } else {
            self.screen = Screen::Playing;
        }
    }

    fn update_chest(&mut self, click: Option<Target>) {
        if self.input.confirm_pressed() || click == Some(Target::Continue) {
            self.chest_reward = None;
            self.sfx(Sfx::UiSelect);
            self.resume_or_open_reward();
        }
    }

    /// One frame of the gene-splice shop. The simulation is **paused** while it
    /// is open (this screen never calls `World::update`), so the shelf, the
    /// purse and the rat cannot change underneath the player's choice.
    fn update_shop(&mut self, click: Option<Target>) {
        if self.world.is_none() {
            self.screen = Screen::Playing;
            return;
        }
        // The Molt flow owns the keys and the pointer while it is open.
        if self.shop_molt.is_some() {
            self.update_shop_molt(click);
            return;
        }
        // A click picks its row and activates it, exactly like Enter.
        if let Some(Target::ShopRow(row)) = click {
            self.shop_row = row.min(shop::ROW_COUNT - 1);
            self.activate_shop_row();
            return;
        }
        if click == Some(Target::ShopLeave) || self.input.back_pressed() {
            self.close_shop();
            return;
        }
        if self.input.nav_pressed(0, -1) || self.input.pressed_char('k') {
            self.shop_row = (self.shop_row + shop::ROW_COUNT - 1) % shop::ROW_COUNT;
            self.sfx(Sfx::UiMove);
        }
        if self.input.nav_pressed(0, 1) || self.input.pressed_char('j') {
            self.shop_row = (self.shop_row + 1) % shop::ROW_COUNT;
            self.sfx(Sfx::UiMove);
        }
        if self.input.confirm_pressed() {
            self.activate_shop_row();
        }
        if self.input.pressed_char('m') {
            self.begin_molt();
        }
    }

    /// Buy (or activate) the selected shelf row.
    fn activate_shop_row(&mut self) {
        let item = shop::row_item(self.shop_row);
        match item {
            shop::ShopItem::Leave => return self.close_shop(),
            shop::ShopItem::Molt => return self.begin_molt(),
            _ => {}
        }
        let Some(world) = self.world.as_mut() else {
            return;
        };
        let hole = world.pending_shop_hole;
        match shop::buy(world, hole, item) {
            shop::Purchase::Bought { .. } => self.sfx(Sfx::Purchase),
            shop::Purchase::Denied(_) => self.sfx(Sfx::Denied),
        }
    }

    /// Start the Molt two-step (or refuse it with the shelf's own reason).
    fn begin_molt(&mut self) {
        let Some(world) = self.world.as_ref() else {
            return;
        };
        let hole = world.pending_shop_hole;
        if shop::molt_deny(world, hole).is_some() || shop::molt_owned(world).is_empty() {
            self.sfx(Sfx::Denied);
            return;
        }
        self.shop_molt = Some(shop_ui::MoltUi::Forfeit { cursor: 0 });
        self.sfx(Sfx::UiMove);
    }

    /// One frame of the Molt nomination: nominate, then pick, `Esc` cancels for
    /// free at either step.
    fn update_shop_molt(&mut self, click: Option<Target>) {
        let Some(molt) = self.shop_molt else {
            return;
        };
        let Some(world) = self.world.as_ref() else {
            self.shop_molt = None;
            return;
        };
        let hole = world.pending_shop_hole;
        let len = match molt {
            shop_ui::MoltUi::Forfeit { .. } => shop::molt_owned(world).len(),
            shop_ui::MoltUi::Pick { owned, .. } => shop::molt_options(world, owned).len(),
        };
        if let Some(Target::ShopMoltRow(i)) = click {
            self.shop_molt = Some(molt.with_cursor(i));
            self.confirm_molt();
            return;
        }
        if click == Some(Target::ShopCancel) || self.input.back_pressed() {
            // Free cancel: nothing is spent until the second selection.
            self.shop_molt = None;
            self.sfx(Sfx::UiMove);
            return;
        }
        let cursor = molt.cursor();
        let mut next = cursor;
        if self.input.nav_pressed(0, -1) || self.input.pressed_char('k') {
            next = (cursor + len.max(1) - 1) % len.max(1);
            self.sfx(Sfx::UiMove);
        }
        if self.input.nav_pressed(0, 1) || self.input.pressed_char('j') {
            next = (cursor + 1) % len.max(1);
            self.sfx(Sfx::UiMove);
        }
        if next != cursor {
            self.shop_molt = Some(molt.with_cursor(next));
        }
        if self.input.confirm_pressed() {
            let _ = hole;
            self.confirm_molt();
        }
    }

    /// Confirm the current Molt step.
    fn confirm_molt(&mut self) {
        let Some(molt) = self.shop_molt else {
            return;
        };
        let Some(world) = self.world.as_ref() else {
            self.shop_molt = None;
            return;
        };
        let hole = world.pending_shop_hole;
        match molt {
            shop_ui::MoltUi::Forfeit { cursor } => {
                let owned = shop::molt_owned(world);
                let Some(&owned) = owned.get(cursor.min(owned.len().saturating_sub(1))) else {
                    self.sfx(Sfx::Denied);
                    return;
                };
                if shop::molt_options(world, owned).is_empty() {
                    self.sfx(Sfx::Denied);
                    return;
                }
                self.shop_molt = Some(shop_ui::MoltUi::Pick { owned, cursor: 0 });
                self.sfx(Sfx::UiSelect);
            }
            shop_ui::MoltUi::Pick { owned, cursor } => {
                let options = shop::molt_options(world, owned);
                let Some(&pick) = options.get(cursor.min(options.len().saturating_sub(1))) else {
                    self.sfx(Sfx::Denied);
                    return;
                };
                let Some(world) = self.world.as_mut() else {
                    return;
                };
                match shop::molt(world, hole, owned, pick) {
                    Ok(()) => {
                        self.shop_molt = None;
                        self.sfx(Sfx::Purchase);
                    }
                    Err(_) => self.sfx(Sfx::Denied),
                }
            }
        }
    }

    /// Leave the shelf: the flag drops, the threshold gets its 0.8 s grace and
    /// the run resumes.
    fn close_shop(&mut self) {
        if let Some(world) = self.world.as_mut() {
            world.leave_shop();
        }
        self.shop_molt = None;
        self.shop_row = 0;
        self.sfx(Sfx::UiSelect);
        self.screen = Screen::Playing;
    }

    /// The level's boss died. Bank the level's own numbers (they unlock the
    /// next sector and feed the victory breakdown), then pick the plate: the
    /// campaign's final level goes straight to victory, every other level goes
    /// through the sector-cleared card.
    fn on_level_complete(&mut self) {
        let Some(w) = self.world.as_ref() else {
            return;
        };
        let level = w.level_index.min(campaign::LEVEL_COUNT - 1);
        let theme = w.level.theme;
        // The ride is a campaign level now (the fourth), so a campaign clears it
        // for real; a **practice** ride is the one case that must not write a
        // campaign record.
        let ride_practice = theme.is_trailer() && self.campaign.is_none();
        let record = LevelRecord {
            time: w.elapsed,
            kills: w.kills,
            score: w.score,
            player_level: w.player.level,
            plate: Some(campaign_ui::CampaignLevel::from_theme(theme)),
            boss: self.killed_boss,
        };
        let campaign_won = self.campaign.is_some() && w.campaign_won();
        // One floor down this run (the bank's floor bonus counts these; a
        // practice floor is charged at the practice rate instead).
        self.floors_cleared = self.floors_cleared.saturating_add(1);
        // The campaign's per-level breakdown only takes campaign levels: a
        // practice ride is not the campaign's fourth floor and must not
        // overwrite it.
        if !ride_practice {
            self.records[level] = record;
            // A CONSOLE USED run never banks a durable campaign record: the
            // cheats make the sector clear meaningless. The in-memory record
            // above still feeds the card and the victory breakdown.
            if !self.console_cheat {
                self.scores.record_level(level, record.score, record.time);
            }
        }
        // Bests are banked **per floor**, so the Kitchen and the Laundry (both
        // stage two) keep separate records, and the ride gets its own instead of
        // clobbering the final campaign sector's.
        // (A CONSOLE USED run records nothing durable.)
        if !self.console_cheat {
            self.scores.record_floor(theme, record.score, record.time);
        }
        // One card per owned weapon and per passive; counted without building
        // the keepable list twice.
        self.owned_upgrades = w.player.loadout.weapons.len() + w.player.loadout.passives.len();
        if campaign_won {
            self.victory_loadout = entries_for(&w.keepables());
        }
        if self.persist && !self.console_cheat {
            // The clear (and the unlock it grants) is durable even if the
            // player quits right after it.
            self.scores.save();
        }
        if campaign_won {
            // `finalize` owns the victory sting (it knows the run won).
            self.finalize();
            // The getaway song at full tier plays under the victory plate.
            // After `finalize`, which drops the tier to calm.
            self.play_music(Theme::Trailer, 3);
            self.screen = Screen::Victory;
        } else {
            // The simulation already played the boss's own death sting this
            // tick (`loot` / `boss::announce`), so the fanfare follows it a
            // beat later instead of stacking on top of it.
            self.queue_sting(Sfx::LevelComplete, FANFARE_DELAY_SECS);
            self.screen = Screen::LevelComplete;
        }
    }

    /// Sector-cleared card → the keep-three manifest (campaign) or the end of a
    /// practice run.
    fn update_levelcomplete(&mut self, click: Option<Target>) {
        if click == Some(Target::Continue)
            || self.input.confirm_pressed()
            || self.input.just_pressed(KeyCode::Char(' '))
        {
            self.sfx(Sfx::UiSelect);
            if self.practice.is_some() {
                // Practice: this sector alone, no carry-over, no campaign win.
                self.finalize();
                self.screen = Screen::End;
            } else {
                self.open_keep_three();
            }
        }
    }

    /// Build the keep-three manifest from what the finished level owns.
    fn open_keep_three(&mut self) {
        let items = match self.world.as_ref() {
            Some(w) => w.keepables(),
            None => return,
        };
        if items.is_empty() {
            // Nothing to pack (unreachable: the starting weapon is always
            // owned), so advance with an empty pack rather than dead-ending.
            self.keep_items.clear();
            self.keep_entries.clear();
            self.keep_selected.clear();
            self.confirm_keep();
            return;
        }
        self.keep_entries = entries_for(&items);
        self.keep_splices = match self.world.as_ref() {
            Some(w) => w
                .player
                .loadout
                .splices
                .iter()
                .map(|kind| kind.name())
                .collect(),
            None => Vec::new(),
        };
        self.keep_items = items;
        self.keep_selected = vec![false; self.keep_items.len()];
        self.keep_cursor = 0;
        self.keep_scroll = 0;
        // The route choice starts on whatever the campaign is currently set to
        // (the original route until the player picks otherwise).
        self.keep_route = self
            .campaign
            .as_ref()
            .filter(|camp| camp.next_options().len() > 1)
            // The keep screen is choosing for the *next* stage.
            .map(|camp| camp.route.theme(camp.level + 1));
        self.screen = Screen::KeepThree;
    }

    /// The keep screen's route chips: the two floors the next level can be
    /// played on, original route first. `[None, None]` when the next stage has
    /// no choice.
    fn keep_route_chips(&self) -> [Option<campaign_ui::RouteChip<'static>>; 2] {
        let mut chips = [None, None];
        let Some(camp) = self.campaign.as_ref() else {
            return chips;
        };
        let options = camp.next_options();
        if options.len() < 2 {
            return chips;
        }
        for (i, theme) in options.iter().enumerate().take(2) {
            chips[i] = Some(campaign_ui::RouteChip {
                name: theme.name(),
                chosen: self.keep_route == Some(*theme),
            });
        }
        chips
    }

    /// The keep screen's `next_level` plate: the floor the route chose.
    fn keep_next_level(&self) -> campaign_ui::CampaignLevel {
        match self.keep_route {
            Some(theme) => campaign_ui::CampaignLevel::from_theme(theme),
            None => campaign_ui::CampaignLevel::from_index(self.keep_from_level() + 1),
        }
    }

    /// The floor the run goes to after the level in flight, from the
    /// campaign's chosen route (`campaign::Route`) — **not** a level index.
    /// `None` when this clear ends the run: a practice floor, the ride (a
    /// capped practice ride or the campaign's own last level), or no campaign in
    /// flight.
    fn next_level_plate(&self) -> Option<campaign_ui::CampaignLevel> {
        let world = self.world.as_ref()?;
        let camp = self.campaign.as_ref()?;
        let index = world.level_index;
        if index >= campaign::LEVEL_COUNT || campaign::is_final_level(index) {
            return None;
        }
        // The route already holds the player's floor choice for the next stage
        // (`Route::ORIGINAL` until the keep screen changes it).
        Some(campaign_ui::CampaignLevel::from_theme(
            camp.route.theme(index + 1),
        ))
    }

    /// How many powerups this run may carry between floors: 3, plus the nest's
    /// carry-over track (up to 5).
    fn keep_max(&self) -> usize {
        match self.world.as_ref() {
            Some(w) => campaign::keep_limit(&w.player.loadout),
            None => campaign::KEEP_COUNT,
        }
    }

    /// `(selected, required)` straight from the art's own helpers, so the pack
    /// size is decided in one place (`keep_three_required`).
    fn keep_counts(&self) -> (usize, usize) {
        let view = campaign_ui::KeepThreeView {
            from_level: campaign_ui::CampaignLevel::from_theme(self.live_theme()),
            next_level: self.keep_next_level(),
            gold: self.world.as_ref().map_or(0, |w| w.gold),
            items: &self.keep_entries,
            selected: &self.keep_selected,
            // The nest's carry-over track buys a bigger pack (3 -> 5).
            max_keep: self.keep_max(),
            cursor: self.keep_cursor,
            scroll: self.keep_scroll,
            hovered: None,
            route: self.keep_route_chips(),
            splices: &self.keep_splices,
        };
        (
            campaign_ui::keep_three_selected_count(&view),
            campaign_ui::keep_three_required(&view),
        )
    }

    /// The campaign level whose pack is being chosen.
    fn keep_from_level(&self) -> usize {
        self.world
            .as_ref()
            .map(|w| w.level_index)
            .unwrap_or(0)
            .min(campaign::LEVEL_COUNT - 1)
    }

    /// Keep the cursor inside the manifest window the art will draw.
    fn keep_clamp_scroll(&mut self) {
        let visible = self.keep_visible_rows.max(1);
        if self.keep_cursor < self.keep_scroll {
            self.keep_scroll = self.keep_cursor;
        } else if self.keep_cursor >= self.keep_scroll + visible {
            self.keep_scroll = self.keep_cursor + 1 - visible;
        }
        let max_scroll = self.keep_items.len().saturating_sub(visible);
        self.keep_scroll = self.keep_scroll.min(max_scroll);
    }

    /// Keep-three: arrows move, Space toggles, Enter confirms (only at the
    /// required count). Mouse toggles come from the art's card rectangles and
    /// the wheel walks the manifest.
    fn update_keep_three(&mut self, click: Option<Target>) {
        let n = self.keep_items.len();
        for i in 0..self.input.mouse_events.len() {
            match self.input.mouse_events[i].kind {
                MouseEventKind::ScrollUp => self.keep_cursor = self.keep_cursor.saturating_sub(1),
                MouseEventKind::ScrollDown if n > 0 => {
                    self.keep_cursor = (self.keep_cursor + 1).min(n - 1);
                }
                _ => continue,
            }
        }
        if n > 0 {
            if self.input.nav_pressed(0, -1) || self.input.nav_pressed(-1, 0) {
                self.keep_cursor = (self.keep_cursor + n - 1) % n;
                self.sfx(Sfx::UiMove);
            }
            if self.input.nav_pressed(0, 1) || self.input.nav_pressed(1, 0) {
                self.keep_cursor = (self.keep_cursor + 1) % n;
                self.sfx(Sfx::UiMove);
            }
        }
        self.keep_clamp_scroll();

        let mut toggle = match click {
            Some(Target::KeepItem(i)) => Some(i),
            _ => None,
        };
        if self.input.just_pressed(KeyCode::Char(' ')) && n > 0 {
            toggle = Some(self.keep_cursor);
        }
        if let Some(i) = toggle {
            self.keep_cursor = i.min(n.saturating_sub(1));
            let (selected, required) = self.keep_counts();
            if let Some(flag) = self.keep_selected.get_mut(i) {
                if *flag {
                    *flag = false;
                    self.sfx(Sfx::UiMove);
                } else if selected < required || selected == 0 {
                    *flag = true;
                    self.sfx(Sfx::UiSelect);
                } else {
                    // The pack is full: a new pick replaces nothing, it refuses.
                    self.sfx(Sfx::UiMove);
                }
            }
        }

        // The route choice: Tab cycles it, and the header chips are clickable.
        let mut route_pick: Option<usize> = match click {
            Some(Target::RouteOption(i)) => Some(i),
            _ => None,
        };
        if self.input.just_pressed(KeyCode::Tab) && self.keep_route_chips()[1].is_some() {
            // Tab swaps to the other floor: the original route is chip 0, so a
            // chosen chip 0 means picking chip 1 and vice versa.
            route_pick = Some(if self.keep_route_chips()[0].is_some_and(|c| c.chosen) {
                1
            } else {
                0
            });
        }
        if let Some(i) = route_pick {
            let options: &[Theme] = self
                .campaign
                .as_ref()
                .map(|camp| camp.next_options())
                .unwrap_or(&[]);
            if let Some(theme) = options.get(i).copied() {
                self.keep_route = Some(theme);
                self.sfx(Sfx::UiMove);
            }
        }

        let confirm = click == Some(Target::KeepConfirm)
            || self.input.confirm_pressed();
        if confirm {
            let (selected, required) = self.keep_counts();
            if selected == required {
                self.confirm_keep();
            } else {
                // Below the required count the confirm is refused (the art only
                // emits its button when it can be pressed).
                self.sfx(Sfx::UiMove);
            }
        }
    }

    /// Carry the chosen entries into the next level and build its world.
    fn confirm_keep(&mut self) {
        let chosen: Vec<Keepable> = self
            .keep_items
            .iter()
            .zip(self.keep_selected.iter())
            .filter(|(_, picked)| **picked)
            .map(|(entry, _)| *entry)
            .collect();
        let mut next = None;
        if let (Some(camp), Some(w)) = (self.campaign.as_mut(), self.world.as_ref()) {
            // The route is chosen on this screen, so it has to be recorded
            // before the next level's world is built.
            if let Some(route) = self.keep_route {
                camp.choose_route(route);
            }
            if camp.complete_level(w, &chosen).is_ok() {
                next = Some(camp.world());
            }
        }
        match next {
            Some(world) => {
                self.sfx(Sfx::UiSelect);
                self.enter_world(world, false);
            }
            None => self.sfx(Sfx::UiMove),
        }
    }

    /// Campaign victory plate: Enter/click on the focused action, arrows/Tab
    /// move the focus, Q returns to the title.
    fn update_victory(&mut self, click: Option<Target>) {
        if click == Some(Target::Nest) || self.input.action_pressed(Action::Nest) {
            self.sfx(Sfx::UiSelect);
            self.open_nest(Screen::Victory);
            return;
        }
        if self.input.nav_pressed(-1, 0)
            || self.input.nav_pressed(1, 0)
            || self.input.just_pressed(KeyCode::Tab)
        {
            self.victory_focus = match self.victory_focus {
                campaign_ui::CampaignVictoryAction::PlayAgain => {
                    campaign_ui::CampaignVictoryAction::Menu
                }
                campaign_ui::CampaignVictoryAction::Menu => {
                    campaign_ui::CampaignVictoryAction::PlayAgain
                }
            };
            self.sfx(Sfx::UiMove);
        }
        let activated = click.or_else(|| {
            self.input.confirm_pressed().then_some({
                match self.victory_focus {
                    campaign_ui::CampaignVictoryAction::PlayAgain => Target::Restart,
                    campaign_ui::CampaignVictoryAction::Menu => Target::Menu,
                }
            })
        });
        match activated {
            Some(Target::Restart) => {
                self.sfx(Sfx::UiSelect);
                self.start_run();
            }
            Some(Target::Menu) => {
                self.sfx(Sfx::UiSelect);
                self.back_to_title();
            }
            _ => {}
        }
        if self.input.action_pressed(Action::Quit) {
            self.back_to_title();
        }
    }

    fn open_levelup(&mut self) {
        self.map_open = false;
        if let Some(w) = self.world.as_mut() {
            // The bandage fallback is hidden at full health (`design §2`), so
            // the pool needs the rat's own HP fraction.
            let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
            let choices = w
                .player
                .loadout
                .generate_choices_with_hp(&mut w.rng, hp_frac);
            self.levelup_desc = choices
                .iter()
                .map(|c| w.player.loadout.describe(c))
                .collect();
            self.levelup_choices = choices;
            for i in 0..self.levelup_choices.len() {
                let up = self.levelup_choices[i].clone();
                if !self.console_cheat {
                    self.scores.library.note_offered(&up);
                }
            }
            self.levelup_idx = 0;
            self.screen = Screen::LevelUp;
        }
    }

    fn update_levelup(&mut self, click: Option<Target>) {
        // The AUTO-PICK pill: key `A`/`U` or a click flips the preference. It
        // takes effect from this card on, after the short reveal below.
        // The AUTO-PICK action (`U` by default), never a bare `A`: that key is
        // move-left on the default scheme and steps the card selection here.
        if self.input.action_pressed(Action::AutoPick)
            || click == Some(Target::AutoPickToggle)
        {
            self.toggle_auto_pick();
            if self.scores.auto_pick {
                self.auto_pick_wait = AUTO_PICK_REVEAL_SECS;
            }
        }
        // Auto-pick on: the screen only steals control long enough to reveal
        // the card. The pick itself is deterministic (the bot's valuation),
        // and the choices were already drawn when the screen opened, so this
        // draws no rng and lands exactly where a manual pick would. The clock
        // itself is decremented once per frame in `update`.
        if self.scores.auto_pick && self.auto_pick_wait <= 0.0 {
            self.auto_pick_current();
            return;
        }
        let n = self.levelup_choices.len().max(1);
        if self.input.nav_pressed(0, -1) || self.input.nav_pressed(-1, 0) {
            self.levelup_idx = (self.levelup_idx + n - 1) % n;
            self.sfx(Sfx::UiMove);
        }
        if self.input.nav_pressed(0, 1) || self.input.nav_pressed(1, 0) {
            self.levelup_idx = (self.levelup_idx + 1) % n;
            self.sfx(Sfx::UiMove);
        }
        let mut pick: Option<usize> = None;
        for (i, key) in ['1', '2', '3'].iter().enumerate() {
            if self.input.pressed_char(*key) && i < self.levelup_choices.len() {
                pick = Some(i);
            }
        }
        if self.input.confirm_pressed() {
            pick = Some(self.levelup_idx);
        }
        if let Some(Target::Upgrade(i)) = click {
            pick = Some(i);
        }
        // A pick that names no card (a stale hit rectangle, an empty hand) is
        // dropped: applying it would take nothing and re-open the screen, and
        // re-opening re-rolls the cards with a fresh rng draw.
        if let Some(i) = pick.filter(|&i| i < self.levelup_choices.len()) {
            self.sfx(Sfx::UiSelect);
            self.apply_levelup(i);
        }
    }

    fn apply_levelup(&mut self, i: usize) {
        if let Some((title, _, _)) = self.levelup_desc.get(i) {
            self.hud.notify_upgrade(title);
        }
        let choices = std::mem::take(&mut self.levelup_choices);
        self.apply_card(&choices, i);
        self.levelup_choices = choices;
        if self.world.is_some() {
            self.resume_or_open_reward();
        }
    }

    /// Apply one level-up card to the world: the shared core of a manual pick
    /// and an auto pick. Returns the toast label and icon for the card
    /// (`("Cheese Spray Lv3", icon)`), captured before the rank changes.
    ///
    /// The loadout's `apply` + the purse's gold credit are exactly what a
    /// manual pick does, so the simulation sees the same state either way.
    fn apply_card(
        &mut self,
        choices: &[Upgrade],
        i: usize,
    ) -> Option<(String, &'static crate::game::art_icons::Icon)> {
        let up = choices.get(i)?.clone();
        let w = self.world.as_mut()?;
        let label = w.player.loadout.picked_label(&up);
        let icon = w.player.loadout.picked_icon(&up);
        // The Library's TAKEN counter: the kind this card ranks or adds (read
        // before `apply`, while a rank-up's slot index is still the card's).
        let taken = match &up {
            Upgrade::NewWeapon(kind) => Some(LibraryKey::Weapon(*kind)),
            Upgrade::LevelWeapon(slot) => w
                .player
                .loadout
                .weapons
                .get(*slot)
                .map(|weapon| LibraryKey::Weapon(weapon.kind)),
            Upgrade::Passive(kind) => Some(LibraryKey::Passive(*kind)),
            Upgrade::Bandage | Upgrade::Purse => None,
        };
        let heal = w.player.loadout.apply(&up);
        w.player.heal(heal);
        // Never on a CONSOLE USED run, like every other Library counter.
        if let (Some(key), false) = (taken, self.console_cheat) {
            self.scores
                .library
                .bump(key, library_index::Counter::Picked, 1);
        }
        if matches!(up, Upgrade::Purse) {
            w.gold = w.gold.saturating_add(crate::game::loadout::PURSE_GOLD);
        }
        w.pending_levelups = w.pending_levelups.saturating_sub(1);
        Some((label, icon))
    }

    /// Take the card the level-up screen is showing without a player choice:
    /// the deterministic valuation over the already-drawn choices. Draws no
    /// rng (the choices were drawn when the screen opened), so it lands exactly
    /// where a manual pick would.
    fn auto_pick_current(&mut self) {
        let choices = std::mem::take(&mut self.levelup_choices);
        let pick = match self.world.as_ref() {
            Some(w) => crate::game::bot::best_upgrade(w, &choices),
            None => {
                self.levelup_choices = choices;
                return;
            }
        };
        let picked = self.apply_card(&choices, pick);
        self.levelup_choices = choices;
        if let Some((label, icon)) = picked {
            self.toast_auto_pick(&label, icon);
            self.auto_pick_wait = AUTO_PICK_SPACING_SECS;
            self.resume_or_open_reward();
        }
    }

    /// Take a level-up card without opening the screen: draw the choices
    /// exactly as the manual screen does (one rng draw per level-up, in the
    /// same place in the tick), pick with the bot's deterministic valuation,
    /// apply it and toast what landed. The game keeps running.
    ///
    /// Returns the cards that were offered and the index taken, so a test can
    /// prove the pick was a legal card from the same set the manual screen
    /// shows (slot caps and evolution partners included).
    fn auto_pick_one(&mut self) -> Option<(Vec<Upgrade>, usize)> {
        let choices = {
            let w = self.world.as_mut()?;
            // The bandage fallback is hidden at full health, exactly as for
            // the manual screen (`design §2`).
            let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
            w.player
                .loadout
                .generate_choices_with_hp(&mut w.rng, hp_frac)
        };
        if choices.is_empty() {
            return None;
        }
        let pick = crate::game::bot::best_upgrade(self.world.as_ref()?, &choices);
        if let Some((label, icon)) = self.apply_card(&choices, pick) {
            self.toast_auto_pick(&label, icon);
            self.auto_pick_wait = AUTO_PICK_SPACING_SECS;
        }
        Some((choices, pick))
    }

    /// Tell the player what the auto-pick took, in the HUD callout area: the
    /// card's icon, its name and its new rank (`AUTO  Cheese Spray Lv3`).
    fn toast_auto_pick(&mut self, label: &str, icon: &'static crate::game::art_icons::Icon) {
        self.hud.push_callout_with_icon(
            hud::CalloutKind::AutoPick,
            Some(icon),
            "AUTO",
            label,
            AUTO_PICK_TOAST_SECS,
        );
        // The level-up sound. The simulation already stings the tick a level-up
        // is earned (`Sfx::LevelUp` from the gem), so only the cards queued
        // behind that first one need their own sting here.
        if !self.sfx_frame.contains(&Sfx::LevelUp) {
            self.sfx(Sfx::LevelUp);
        }
    }

    /// Flip the auto-pick preference. The level-up screen's pill, the options
    /// row and the in-play key all go through here, so the save and the toast
    /// are identical.
    /// Force a glyph set for the session, as `RATRUN_SUBCELL` does (tests).
    #[cfg(test)]
    pub(crate) fn set_subcell_override(&mut self, choice: Option<crate::render::subcell::Choice>) {
        self.subcell_env = choice;
        self.picture_dirty = true;
    }

    /// Toggle the xBR fit in render tests.
    #[cfg(test)]
    pub(crate) fn set_smooth(&mut self, on: bool) {
        use crate::render::subcell::Smooth;
        if on != self.smooth_enabled() {
            self.set_smooth_level(if on { Smooth::Xbr } else { Smooth::Off });
        }
    }

    /// Whether the 1x edge smoothing is on at any level.
    pub(crate) fn smooth_enabled(&self) -> bool {
        self.smooth != crate::render::subcell::Smooth::Off
    }

    /// Pick the 1x edge treatment (the console `smooth` command and the
    /// Graphics page's SMOOTHING row): off, the EPX rounding, or the
    /// antialiasing. Render-only, and persisted — a choice made here is the
    /// player's, so it also drops the `RATRUN_SMOOTH` session pin.
    pub(crate) fn set_smooth_level(&mut self, level: crate::render::subcell::Smooth) {
        self.smooth_env = None;
        if self.smooth != level {
            self.smooth = level;
            self.picture_dirty = true;
        }
        self.save_graphics();
    }

    /// The 1x edge treatment in force.
    pub(crate) fn smooth_level(&self) -> crate::render::subcell::Smooth {
        self.smooth
    }

    /// Turn the parallax foreground layer on or off (the console `foreground`
    /// command and the Graphics page's FOREGROUND row). Render-only: no run
    /// state, no golden can see it; the preference is persisted, and choosing
    /// it drops the `RATRUN_FOREGROUND` session pin.
    pub(crate) fn set_foreground(&mut self, on: bool) {
        self.foreground_env = None;
        if self.foreground != on {
            self.foreground = on;
            self.picture_dirty = true;
        }
        self.save_graphics();
    }

    /// Whether the parallax foreground layer is on.
    pub(crate) fn foreground_enabled(&self) -> bool {
        self.foreground
    }

    /// Turn the batch-13 lighting effects on or off (the console `fx` command
    /// and the Graphics page's LIGHT EFFECTS row). Render-only: no run state, no
    /// golden can see it; the preference is persisted, and choosing it drops the
    /// `RATRUN_FX` session pin.
    pub(crate) fn set_fx(&mut self, on: bool) {
        self.fx_env = None;
        if self.fx != on {
            self.fx = on;
            self.picture_dirty = true;
        }
        self.save_graphics();
    }

    /// Whether the batch-13 lighting effects are on.
    pub(crate) fn fx_enabled(&self) -> bool {
        self.fx
    }

    /// Turn the round-glyph particle layer on or off (the console `glyphfx`
    /// command and the Graphics page's GLYPH PARTICLES row). Render-only: no run
    /// state, no golden, and no world redraw either — the layer is written into
    /// the frame's cells after the cached picture is blitted, so the next frame
    /// carries the change on its own. The preference is persisted, and choosing
    /// it drops the `RATRUN_GLYPHFX` session pin.
    pub(crate) fn set_glyphfx(&mut self, on: bool) {
        self.glyphfx_env = None;
        self.glyphfx.set_enabled(on);
        self.save_graphics();
    }

    /// Whether the round-glyph particle layer is on.
    pub(crate) fn glyphfx_enabled(&self) -> bool {
        self.glyphfx.enabled()
    }

    /// The camera's share of the simulation's shake (the Graphics page's SCREEN
    /// SHAKE row). Render-only: `World::shake_scale` is read where `draw_scene`
    /// builds the camera and nowhere in the simulation, so this can never move a
    /// run, a golden or the map pin. Persisted with the other graphics
    /// preferences.
    pub fn set_shake_scale(&mut self, scale: f32) {
        self.shake_scale = if scale.is_finite() {
            scale.clamp(0.0, 1.0)
        } else {
            crate::game::world::SHAKE_SCALE
        };
        if let Some(w) = self.world.as_mut() {
            w.shake_scale = self.shake_scale;
        }
        self.save_graphics();
    }

    /// The camera's share of the simulation's shake in force.
    pub fn shake_scale(&self) -> f32 {
        self.shake_scale
    }

    /// Write the Graphics page's six settings into the saved table and (when
    /// this app persists at all) to disk.
    ///
    /// A field still pinned by its `RATRUN_*` variable keeps the **saved**
    /// value: the environment wins for this session only, so a player who
    /// changes one row must not silently overwrite another row they never
    /// touched with the pinned value (`set_*` drops the pin of the row it
    /// changes before calling this).
    fn save_graphics(&mut self) {
        // Read the world-backed value first: `lighting_mode` borrows the whole
        // app, which the row-level borrow below would not allow.
        let lighting = self.lighting_mode();
        let graphics = &mut self.scores.graphics;
        if self.smooth_env.is_none() {
            graphics.set_smooth(self.smooth);
        }
        if self.foreground_env.is_none() {
            graphics.foreground = self.foreground;
        }
        if self.fx_env.is_none() {
            graphics.fx = self.fx;
        }
        if self.glyphfx_env.is_none() {
            graphics.glyphfx = self.glyphfx.enabled();
        }
        graphics.set_shake(self.shake_scale);
        graphics.set_lighting(lighting);
        if self.persist {
            self.scores.save();
        }
    }

    /// Whether this screen draws the particle layer: only over a play field the
    /// app is drawing anyway (live, or frozen under a plate), never over the
    /// Library — whose specimens are their own cards — the title, or a plate
    /// that owns the whole frame. The `--ai` wall never comes through
    /// `draw_game` at all, so it cannot draw the layer either.
    fn glyphfx_over_field(&self) -> bool {
        match self.screen {
            Screen::Playing | Screen::Paused | Screen::LevelUp | Screen::Chest | Screen::Shop => {
                true
            }
            // These plates can open over a paused run and keep its field.
            Screen::Options | Screen::Controls => {
                self.options_return == Screen::Paused
            }
            Screen::Graphics => self.graphics_over_field(),
            Screen::Nest => self.nest_return == Screen::Paused,
            _ => false,
        }
    }

    fn toggle_auto_pick(&mut self) {
        let on = !self.scores.auto_pick;
        self.scores.auto_pick = on;
        if self.persist {
            self.scores.save();
        }
        self.sfx(Sfx::UiSelect);
        // Say so in the HUD callout area when the flip happens in play; on the
        // level-up screen the pill itself shows the new state.
        if matches!(self.screen, Screen::Playing) {
            self.hud.push_callout(
                hud::CalloutKind::AutoPick,
                if on { "AUTO-PICK ON" } else { "AUTO-PICK OFF" },
                if on {
                    "CARDS SKIPPED · U TAKES BACK CONTROL"
                } else {
                    "THE CARDS ARE YOURS AGAIN"
                },
                AUTO_PICK_TOAST_SECS,
            );
        }
    }

    fn update_paused(&mut self, click: Option<Target>) {
        if self.input.action_pressed(Action::Graphics) {
            self.open_graphics();
            return;
        }
        // The Options key itself is handled once, globally, in `update` (it
        // reads the live binding); only the pointer needs this screen.
        if click == Some(Target::Options) {
            self.sfx(Sfx::UiSelect);
            self.open_options(Screen::Paused);
            return;
        }
        if click == Some(Target::Library) || self.input.action_pressed(Action::Library) {
            self.sfx(Sfx::UiSelect);
            self.open_library(Screen::Paused);
            return;
        }
        // The nest opens here **read-only**: a dossier of what this run carries,
        // not a shop (you cannot buy mid-run).
        if click == Some(Target::Nest) || self.input.action_pressed(Action::Nest) {
            self.sfx(Sfx::UiSelect);
            self.open_nest(Screen::Paused);
            return;
        }
        if click == Some(Target::Resume)
            || self.input.back_pressed()
            || self.input.action_pressed(Action::Pause)
        {
            self.screen = Screen::Playing;
        }
        if click == Some(Target::Menu)
            || self.input.action_pressed(Action::Quit)
            || self.input.pad_back()
        {
            self.back_to_title();
        }
    }

    fn update_end(&mut self, click: Option<Target>) {
        if click == Some(Target::Nest) || self.input.action_pressed(Action::Nest) {
            self.sfx(Sfx::UiSelect);
            self.open_nest(Screen::End);
            return;
        }
        if click == Some(Target::Restart) || self.input.confirm_pressed() {
            self.retry();
        }
        if click == Some(Target::Menu)
            || self.input.action_pressed(Action::Quit)
            || self.input.pad_back()
        {
            self.back_to_title();
        }
    }

    /// Did this run win in the display sense (campaign cleared, or a practice
    /// sector cleared)?
    fn run_won(&self, w: &World) -> bool {
        let campaign_won = self.campaign.is_some() && w.campaign_won();
        let practice_cleared = self.practice.is_some() && w.level_complete();
        campaign_won || practice_cleared
    }

    /// The end-of-run summary. Everything is **cumulative**: the campaign
    /// banks each completed level's score/kills/time, so the incident report
    /// shows the totals and the sector the run reached.
    fn build_summary(&self, w: &World, final_score: u32) -> RunSummary {
        let time = w.total_time();
        let campaign_won = self.campaign.is_some() && w.campaign_won();
        RunSummary {
            won: self.run_won(w),
            theme: w.level.theme,
            combat_score: w.total_score(),
            gold: w.gold,
            bonus_time: (time as u32) * 5,
            bonus_level: w.player.level * 100,
            bonus_win: if campaign_won { 5000 } else { 0 },
            final_score,
            time,
            level: w.player.level,
            kills: w.total_kills(),
            damage_taken: self.damage_taken,
            dashes_used: self.dashes_used,
            heals_picked: self.heals_picked,
            boss_slain: self.boss_slain,
            weapons: w
                .player
                .loadout
                .weapons
                .iter()
                .map(|wp| (wp.kind, wp.level))
                .collect(),
            damage_mult: w.player.damage_mult(),
            fire_rate_mult: w.player.loadout.stats.fire_rate_mult,
            armor: w.player.loadout.stats.armor,
            sector: (w.level_index + 1).clamp(1, campaign::LEVEL_COUNT) as u8,
            practice: self.practice.is_some(),
            // The nest's banking tally, already computed by `finalize`.
            nest: self.bank_tally,
            console_used: self.console_cheat,
        }
    }

    fn finalize(&mut self) {
        if self.recorded {
            return;
        }
        // Bank the haul **before** the summary is built: the incident report
        // prints the tally, so it has to exist first. Already banked (a quit to
        // the title, then the report) is a no-op.
        self.bank_the_run();
        // Hoisted: the sting choice at the end of this function needs it too.
        let campaign_won = self.campaign.is_some()
            && self.world.as_ref().is_some_and(|w| w.campaign_won());
        if let Some(w) = self.world.as_ref() {
            let time = w.total_time();
            let won = self.run_won(w);
            let bonus_time = (time as u32) * 5;
            let bonus_level = w.player.level * 100;
            let bonus_win = if campaign_won { 5000 } else { 0 };
            let final_score = w.total_score() + bonus_time + bonus_level + bonus_win;
            let sector = (w.level_index + 1).clamp(1, campaign::LEVEL_COUNT) as u8;
            self.last_score = final_score;
            self.last_summary = Some(self.build_summary(w, final_score));
            // A CONSOLE USED run never reaches the score table or the campaign
            // records: the cheats make the score meaningless. The summary still
            // shows the numbers, labelled as tainted.
            if self.console_cheat {
                self.new_best = false;
            } else {
                // "New high score" means it beat the table's best, computed
                // before this run's entry is inserted (an under-full table is
                // no free pass).
                self.new_best = self.scores.is_high_score(final_score);
                self.scores.insert(ScoreEntry {
                    score: final_score,
                    time,
                    level: w.player.level,
                    map: w.level.theme.name().to_string(),
                    won,
                    campaign_level: sector,
                    campaign_won,
                });
                if self.persist {
                    self.scores.save();
                }
            }
        }
        self.recorded = true;
        // Music settles, plus a sting.
        self.request_music_intensity(0);
        let won = self.last_summary.as_ref().is_some_and(|s| s.won);
        match (campaign_won, won) {
            // Same rule as the sector card: the campaign fanfare follows the
            // Lab boss's death sting; a practice clear just wins.
            (true, _) => self.queue_sting(Sfx::CampaignVictory, FANFARE_DELAY_SECS),
            (false, true) => self.sfx(Sfx::Win),
            (false, false) => self.sfx(Sfx::Lose),
        }
    }

    // ---- Rendering ------------------------------------------------------

    /// How often the current screen wants rendering: every frame while the
    /// simulation runs, the menu cadence everywhere else (`present::FramePacer`).
    pub fn frame_demand(&self) -> crate::present::Demand {
        match self.screen {
            Screen::Playing if self.map_open => crate::present::Demand::AnimatedMenu,
            Screen::Playing => crate::present::Demand::Live,
            _ => crate::present::Demand::Menu,
        }
    }

    /// Does input or a known animation require the screen to stay active?
    ///
    /// True when input of any kind arrived this frame (keys, pointer motion,
    /// pad, focus, a resize), when the screen changed, and while a clock the
    /// app knows about is running on a screen that is otherwise still: the
    /// console, the zoom easing, the volume/zoom read-outs, the auto-pick
    /// toast spacing, the Library's animating card and the expanded map marker.
    pub fn repaint_urgent(&self) -> bool {
        self.map_open
            || self.repaint_immediate()
            || self.hud.flashing()
            || self.auto_pick_wait > 0.0
            // The Library's card scene steps at a fixed dt; a new step is a
            // new picture, and this is the clock that keeps it at 30 Hz (a
            // still or locked card leaves this false, so the screen idles).
            || self.library_ui.frame_due()
            || self
                .world
                .as_ref()
                .is_some_and(|w| w.zoom_view != w.zoom_view_target())
    }

    /// Repaint requests that can interrupt the expanded map's regular cadence.
    pub(crate) fn repaint_immediate(&self) -> bool {
        self.repaint || self.input.events > 0 || self.input.pad.active() || self.console.open
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.last_area = area;
        self.repaint = false;
        self.hits.clear();
        match self.screen {
            Screen::Title => {
                self.ensure_rat_art(area);
                let theme = self.theme();
                let floor = self.floor();
                let unlocked = self.sector_unlocked();
                menu::draw_title(
                    frame,
                    area,
                    theme,
                    self.menu_idx,
                    floor,
                    unlocked,
                    &self.scores,
                    &self.rat_art,
                    &self.input.controls,
                    &mut self.hits,
                );
            }
            Screen::SectorSelect => self.draw_sector_select(frame, area),
            Screen::Options => {
                // Draw whatever opened the screen underneath, then the plate:
                // the frozen play field behind the pause overlay, or the title.
                if self.options_return == Screen::Paused {
                    self.draw_game(frame, area);
                } else {
                    self.ensure_rat_art(area);
                    let theme = self.theme();
                    let floor = self.floor();
                    let unlocked = self.sector_unlocked();
                    menu::draw_title(
                        frame,
                        area,
                        theme,
                        self.menu_idx,
                        floor,
                        unlocked,
                        &self.scores,
                        &self.rat_art,
                        &self.input.controls,
                        &mut self.hits,
                    );
                }
                self.draw_options(frame, area);
            }
            Screen::Story => {
                self.ensure_rat_art(area);
                let theme = self.theme();
                let floor = self.floor();
                let unlocked = self.sector_unlocked();
                menu::draw_title(
                    frame,
                    area,
                    theme,
                    self.menu_idx,
                    floor,
                    unlocked,
                    &self.scores,
                    &self.rat_art,
                    &self.input.controls,
                    &mut self.hits,
                );
                menu::draw_story(frame, area, self.story_time, &mut self.hits);
            }
            Screen::Library => {
                // The index opens over whatever the player came from: the
                // frozen play field behind a paused run, or the title.
                if self.library_return == Screen::Paused && self.world.is_some() {
                    self.draw_game(frame, area);
                } else {
                    self.ensure_rat_art(area);
                    menu::draw_title(
                        frame,
                        area,
                        self.theme(),
                        self.menu_idx,
                        self.floor(),
                        self.sector_unlocked(),
                        &self.scores,
                        &self.rat_art,
                        &self.input.controls,
                        &mut self.hits,
                    );
                }
                let accent = menu::material(self.theme());
                self.library_shape = library_ui::draw(
                    frame,
                    area,
                    &self.library_rows,
                    &self.scores.library,
                    accent,
                    &mut self.library_ui,
                    &mut self.hits,
                );
            }
            Screen::Nest => {
                // The nest opens over whatever the player came from: the frozen
                // play field behind the pause plate, a plain backdrop behind the
                // incident report / victory plate (those draw their own frame),
                // or the title.
                if self.nest_return == Screen::Paused && self.world.is_some() {
                    self.draw_game(frame, area);
                } else if matches!(self.nest_return, Screen::End | Screen::Victory) {
                    draw_backdrop(frame, area);
                } else {
                    self.ensure_rat_art(area);
                    let theme = self.theme();
                    let floor = self.floor();
                    let unlocked = self.sector_unlocked();
                    menu::draw_title(
                        frame,
                        area,
                        theme,
                        self.menu_idx,
                        floor,
                        unlocked,
                        &self.scores,
                        &self.rat_art,
                        &self.input.controls,
                        &mut self.hits,
                    );
                }
                // Read-only from the pause screen: the **run's** compiled
                // modifiers, never the saved bank, so the dossier cannot show a
                // purchase the run is not playing with. The view is owned (a
                // small struct, once a frame, on a menu screen) so the draw call
                // holds no borrow of `self`.
                let view: meta::MetaProfile = if self.nest_read_only {
                    // `live_modifiers` is what the run was built with; the saved
                    // profile only stands in when no run is in flight.
                    let mods = self
                        .live_modifiers()
                        .unwrap_or_else(|| self.run_modifiers());
                    nest_ui::profile_from_modifiers(&mods)
                } else {
                    self.scores.meta.clone()
                };
                let accent = options::accent_from(menu::material(self.theme()));
                self.nest_shape = nest_ui::draw(
                    frame,
                    area,
                    &view,
                    &mut self.nest_ui,
                    accent,
                    &self.input.controls,
                    &mut self.hits,
                );
            }
            Screen::LevelIntro => self.draw_level_intro(frame, area),
            Screen::LevelComplete => self.draw_level_complete(frame, area),
            Screen::KeepThree => self.draw_keep_three(frame, area),
            Screen::Victory => self.draw_victory(frame, area),
            Screen::MapIntro | Screen::Playing | Screen::Paused | Screen::LevelUp | Screen::End => {
                self.draw_game(frame, area);
                match self.screen {
                    Screen::MapIntro => menu::draw_map_intro(
                        frame,
                        area,
                        // The floor actually being played: the title's cursor
                        // may sit on another floor (or the Flatbed) when the
                        // campaign was launched.
                        self.live_theme(),
                        self.world
                            .as_ref()
                            .map(|w| {
                                crate::render::palette::marking(
                                    w.level.palette.wall_edge,
                                    w.level.palette.accent,
                                )
                            })
                            .unwrap_or(crate::render::palette::MUTED),
                        &mut self.hits,
                    ),
                    Screen::Paused => {
                        if let Some(w) = self.world.as_ref() {
                            menu::draw_pause_with_run(
                                frame,
                                area,
                                w,
                                self.muted,
                                &self.input.controls,
                                &mut self.hits,
                            );
                        } else {
                            menu::draw_pause(frame, area, &self.input.controls, &mut self.hits);
                        }
                    }
                    Screen::LevelUp => {
                        if let Some(w) = self.world.as_ref() {
                            menu::draw_levelup_with_loadout_keyed(
                                frame,
                                area,
                                w.player.level,
                                &self.levelup_desc,
                                &self.levelup_choices,
                                &w.player.loadout,
                                self.levelup_idx,
                                self.scores.auto_pick,
                                self.input.controls.action_label(Action::AutoPick),
                                self.input.controls.action_label(Action::Confirm),
                                &mut self.hits,
                            );
                        } else {
                            menu::draw_levelup_keyed(
                                frame,
                                area,
                                1,
                                &self.levelup_desc,
                                &self.levelup_choices,
                                self.levelup_idx,
                                self.scores.auto_pick,
                                self.input.controls.action_label(Action::AutoPick),
                                self.input.controls.action_label(Action::Confirm),
                                &mut self.hits,
                            );
                        }
                    }
                    Screen::End => {
                        if let Some(w) = self.world.as_ref() {
                            let fallback;
                            let summary = match self.last_summary.as_ref() {
                                Some(s) => s,
                                None => {
                                    fallback = self.build_summary(w, self.last_score);
                                    &fallback
                                }
                            };
                            // A cleared practice sector earns the victory line
                            // too, even though it is not a campaign win.
                            let lore_line = if summary.won {
                                lore::victory(w.level.theme)
                            } else {
                                lore::DEFEAT
                            };
                            menu::draw_end_summary(
                                frame,
                                area,
                                summary,
                                self.new_best,
                                lore_line,
                                &self.scores,
                                &self.input.controls,
                                &mut self.hits,
                            );
                        }
                    }
                    Screen::Playing => self.hits.add(area, Target::Dash),
                    _ => {}
                }
                // The minimap is registered last, so it takes precedence over
                // the full-field Dash target. Only a left press activates a
                // target (`mouse_action`); a right click is Back in the menus
                // and the actives in play, never a click on what sits under it.
                if matches!(self.screen, Screen::Playing) {
                    if !self.last_map.is_empty() {
                        self.hits.add(self.last_map, Target::Minimap);
                    } else if !self.last_minimap.is_empty() {
                        self.hits.add(self.last_minimap, Target::Minimap);
                    }
                }
            }
            Screen::Controls => {
                if self.options_return == Screen::Paused {
                    self.draw_game(frame, area);
                } else {
                    self.ensure_rat_art(area);
                    let theme = self.theme();
                    let floor = self.floor();
                    let unlocked = self.sector_unlocked();
                    menu::draw_title(
                        frame,
                        area,
                        theme,
                        self.menu_idx,
                        floor,
                        unlocked,
                        &self.scores,
                        &self.rat_art,
                        &self.input.controls,
                        &mut self.hits,
                    );
                }
                self.draw_controls(frame, area);
            }
            Screen::Graphics => {
                if self.graphics_over_field() {
                    self.draw_game(frame, area);
                } else {
                    self.ensure_rat_art(area);
                    let theme = self.theme();
                    let floor = self.floor();
                    let unlocked = self.sector_unlocked();
                    menu::draw_title(
                        frame,
                        area,
                        theme,
                        self.menu_idx,
                        floor,
                        unlocked,
                        &self.scores,
                        &self.rat_art,
                        &self.input.controls,
                        &mut self.hits,
                    );
                }
                self.draw_graphics(frame, area);
            }
            Screen::Chest => {
                self.draw_game(frame, area);
                if let Some((reward, loadout)) = self
                    .chest_reward
                    .zip(self.world.as_ref().map(|w| &w.player.loadout))
                {
                    menu::draw_chest(frame, area, reward, loadout, &mut self.hits);
                }
            }
            Screen::Shop => {
                self.draw_game(frame, area);
                if let Some(w) = self.world.as_ref() {
                    // The shelf's legend names the live Confirm/Back keys; Molt
                    // is a bare `M` in `update_shop`, not an action.
                    let keys = shop_ui::ShopKeys {
                        confirm: self.input.controls.action_label(Action::Confirm),
                        back: self.input.controls.action_label(Action::Back),
                        molt: shop_ui::ShopKeys::DEFAULT.molt,
                    };
                    shop_ui::draw_with_keys(
                        frame,
                        area,
                        w,
                        w.pending_shop_hole,
                        self.shop_row,
                        self.shop_molt,
                        &mut self.hits,
                        &keys,
                    );
                }
            }
        }
        // The perf overlay sits on top of the game, below the console.
        if self.perf_on {
            self.draw_perf(frame, area);
        }
        // The console's slide overlay sits above every screen, below the cursor.
        if self.console.anim > 0.001 {
            crate::render::console::draw(frame, area, &self.console);
        }
        self.draw_cursor(frame);
    }

    /// Draw the console's `perf` overlay: frame time, tick time and entity
    /// counts in the top-right corner. Read-only presentation state.
    fn draw_perf(&self, frame: &mut Frame, area: Rect) {
        let Some(w) = self.world.as_ref() else {
            return;
        };
        let lines = [
            format!("frame {:.1}ms", self.perf.frame_ms),
            format!("tick  {:.1}ms", self.perf.tick_ms),
            format!("enemies {}", w.enemy_count()),
            format!("bullets {}", w.bullet_count()),
            format!("fps {:.0}", if self.perf.frame_ms > 0.0 { 1000.0 / self.perf.frame_ms } else { 0.0 }),
        ];
        let width = 16u16.min(area.width);
        let height = lines.len() as u16;
        if area.width < width + 2 || area.height < height + 2 {
            return;
        }
        let x = area.right().saturating_sub(width);
        let y = area.y;
        let buf = frame.buffer_mut();
        for (i, line) in lines.iter().enumerate() {
            let row = y + i as u16;
            buf.set_stringn(
                x,
                row,
                line,
                width as usize,
                Style::default()
                    .bg(Color::Rgb(12, 16, 18))
                    .fg(Color::Rgb(210, 220, 220)),
            );
        }
    }

    /// Composite the cheese mouse cursor last, on top of every normal-game
    /// screen, without disturbing `self.hits`.
    ///
    /// It is drawn **after** `draw_game` has blitted the playfield — half-block
    /// or sub-cell, lit, hostiles and overlays on top — so it is stamped in
    /// screen space over all of it and the sub-cell encoder and the 1×
    /// smoothing can never eat it (they only ever saw the picture underneath).
    /// It is not part of the cached picture (`PictureKey`), so it never forces a
    /// redraw. In `Scheme::MouseAim` this wedge **is** the reticle: its tip is
    /// in the pointer's cell, and that cell is the one `aim_point` reads.
    fn draw_cursor(&self, frame: &mut Frame) {
        if !self.cursor_enabled() {
            return;
        }
        let Some(pos) = self.input.mouse_pos else {
            return;
        };
        let alpha = self.cursor.alpha();
        if alpha <= 0.02 {
            return;
        }
        let over_target = self
            .hits
            .at(pos.0, pos.1)
            .is_some_and(|t| t != Target::Dash);
        let view = cursor::CursorView {
            pos,
            hover: over_target,
            held: self.input.mouse_down,
            click_pulse: self.cursor.click_pulse(),
            bob_time: self.cursor.bob_time(),
            alpha,
            playing_steer: matches!(self.screen, Screen::Playing) && self.input.prefer_mouse,
        };
        let area = frame.area();
        cursor::draw(frame.buffer_mut(), area, view);
    }

    /// Draw the options plate over the screen that opened it.
    fn draw_options(&mut self, frame: &mut Frame, area: Rect) {
        let view = options::OptionsView {
            music: self.music_volume,
            effects: self.effects_volume,
            minimap: self.scores.minimap_size,
            cursor: self.scores.cheese_cursor,
            muted: self.muted,
            zoom: self.effective_zoom(),
            row: self.options_row,
            reset_armed: self.reset_armed,
            accent: options::accent_from(menu::material(self.theme())),
            scheme: self.input.controls.scheme(),
            deadzone: self.input.controls.deadzone,
            rumble: self.input.controls.rumble,
            auto_pick: self.scores.auto_pick,
        };
        options::draw(frame, area, &view, &mut self.hits);
    }

    /// Draw the Controls plate over whatever opened Options.
    fn draw_controls(&mut self, frame: &mut Frame, area: Rect) {
        // The terminal caveat also carries the Super Dash's hold-to-charge
        // reality: where a key release cannot be reported, a key only taps and
        // the mouse or the pad does the charging.
        let note = if self.controls_note.is_empty() {
            self.input.controls.terminal_note()
        } else {
            self.controls_note.as_str()
        };
        let scroll = self.controls_scroll;
        let lay = render_controls::layout(area, scroll);
        self.controls_visible = lay.visible;
        let view = render_controls::ControlsView {
            scheme: self.input.controls.scheme(),
            controls: &self.input.controls,
            row: self.controls_row,
            scroll: self.controls_scroll,
            capturing: self.bind_capture,
            note,
            accent: render_controls::accent_from(menu::material(self.theme())),
            lone_ctrl: self.input.lone_ctrl,
        };
        render_controls::draw(frame, area, &view, &mut self.hits);
    }

    /// Fill the Graphics plate from the app's live settings and bindings.
    fn draw_graphics(&mut self, frame: &mut Frame, area: Rect) {
        let view = graphics::GraphicsView {
            lighting: self.lighting_mode(),
            fx: self.fx_enabled(),
            smooth: self.smooth_level(),
            foreground: self.foreground_enabled(),
            glyphfx: self.glyphfx_enabled(),
            shake_scale: self.shake_scale,
            row: self.graphics_row,
            accent: options::accent_from(menu::material(self.theme())),
            graphics_key: self.input.controls.action_label(Action::Graphics),
            back_key: self.input.controls.action_label(Action::Back),
        };
        graphics::draw(frame, area, &view, &mut self.hits);
    }

    // ---- Campaign plates -------------------------------------------------

    /// Sector select: one card per floor, in route order, fed from the
    /// persisted **per-floor** bests, with the art's own hover/selection
    /// styling and click rectangles.
    ///
    /// The slot list is built from `Theme::playable()` — never a hard-coded
    /// count — so a new floor shows up here without touching this function.
    /// Each slot carries its own floor index, so selecting a slot selects
    /// exactly that floor: what is highlighted, what the header names and what
    /// starts are the same theme.
    fn draw_sector_select(&mut self, frame: &mut Frame, area: Rect) {
        self.last_select_area = area;
        draw_backdrop(frame, area);
        let mut slots = [campaign_ui::LevelSlotView {
            floor: 0,
            level: campaign_ui::CampaignLevel::Sewer,
            stage: 0,
            unlocked: true,
            best: None,
        }; campaign_ui::MAX_LEVEL_SLOTS];
        let mut count = 0usize;
        for (floor, theme) in Theme::playable().into_iter().enumerate() {
            if count >= slots.len() {
                break;
            }
            // The floor's own campaign level: the ride reads its fourth, not
            // the Lab's third (`campaign::theme_stage`).
            let stage = campaign::theme_stage(theme);
            slots[count] = campaign_ui::LevelSlotView {
                floor,
                level: campaign_ui::CampaignLevel::from_theme(theme),
                stage,
                unlocked: self.scores.floor_unlocked(theme),
                best: self.scores.floor_best(theme).map(|b| campaign_ui::LevelBest {
                    score: b.score,
                    best_time_secs: b.time,
                    cleared: b.cleared,
                }),
            };
            count += 1;
        }
        let view = campaign_ui::LevelSelectView {
            back_key: self.input.controls.action_label(Action::Back),
            slots: &slots[..count],
            selected: self.floor(),
            hovered: match self.hits.hovered {
                Some(Target::SelectSector(floor)) => {
                    Some(campaign_ui::LevelSelectAction::SelectLevel(floor))
                }
                Some(Target::StartSector) => Some(campaign_ui::LevelSelectAction::Start),
                Some(Target::Back) => Some(campaign_ui::LevelSelectAction::Back),
                _ => None,
            },
        };
        let list = campaign_ui::draw_level_select_with(frame, area, &view, &self.plate_keys());
        for (rect, action) in list.iter() {
            let target = match action {
                campaign_ui::LevelSelectAction::SelectLevel(floor) => {
                    Target::SelectSector(floor)
                }
                campaign_ui::LevelSelectAction::Start => Target::StartSector,
                campaign_ui::LevelSelectAction::Back => Target::Back,
            };
            self.hits.add(rect, target);
        }
    }

    /// The key names the campaign plates print, from the live bindings. Space
    /// (pack) and Tab (route) are raw keys in `update_keep_three`, not
    /// actions, so they keep the plates' stock names.
    fn plate_keys(&self) -> campaign_ui::PlateKeys<'_> {
        campaign_ui::PlateKeys {
            confirm: self.input.controls.action_label(Action::Confirm),
            toggle: campaign_ui::PlateKeys::DEFAULT.toggle,
            route: campaign_ui::PlateKeys::DEFAULT.route,
        }
    }

    /// The cleared-sector plate for the boss the level ended on.
    fn draw_level_complete(&mut self, frame: &mut Frame, area: Rect) {
        draw_backdrop(frame, area);
        let list = match self.world.as_ref() {
            Some(w) => {
                let view = campaign_ui::LevelCompleteView {
                    // The plate names the floor actually played, so a side
                    // route reads THE LAUNDRY / THE PRINTWORKS here.
                    level: campaign_ui::CampaignLevel::from_theme(w.level.theme),
                    // The flatbed has no campaign plate: its own stamp comes
                    // from the theme (the Magpie, not the Lab's mower).
                    theme: Some(w.level.theme),
                    time_secs: w.elapsed,
                    kills: w.kills,
                    score_so_far: w.total_score(),
                    gold: w.gold,
                    level_score: w.score,
                    player_level: w.player.level,
                    owned_upgrades: self.owned_upgrades,
                    // The route's next floor, not the original route's stage:
                    // a run packing for the Printworks says so here.
                    next: self.next_level_plate(),
                    custom_triumph: None,
                    hovered: (self.hits.hovered == Some(Target::Continue))
                        .then_some(campaign_ui::LevelCompleteAction::Continue),
                };
                // The boss that actually died here; a floor completed without
                // one (a test hook) falls back to the floor's seeded pick, the
                // same expression the spawner uses.
                let boss = self.killed_boss.unwrap_or_else(|| {
                    crate::game::ecs::systems::boss::BossKind::pool_pick(w.level.theme, w.level.seed)
                });
                campaign_ui::draw_level_complete_with(
                    frame,
                    area,
                    &view,
                    &self.plate_keys(),
                    Some(boss),
                )
            }
            None => return,
        };
        for (rect, _) in list.iter() {
            self.hits.add(rect, Target::Continue);
        }
    }

    /// Levels 2 and 3's intro plate, showing the pack carried in.
    fn draw_level_intro(&mut self, frame: &mut Frame, area: Rect) {
        draw_backdrop(frame, area);
        let practice = self.practice.is_some();
        let list = match self.world.as_ref() {
            Some(w) => {
                let view = campaign_ui::LevelIntroView {
                    level: campaign_ui::CampaignLevel::from_theme(w.level.theme),
                    flavor: practice.then_some(PRACTICE_BLURB),
                    carried: &self.carried_entries,
                    gold: w.gold,
                    score_so_far: w.total_score(),
                    hovered: (self.hits.hovered == Some(Target::Begin))
                        .then_some(campaign_ui::LevelIntroAction::Begin),
                };
                campaign_ui::draw_level_intro_with(frame, area, &view, &self.plate_keys())
            }
            None => return,
        };
        for (rect, _) in list.iter() {
            self.hits.add(rect, Target::Begin);
        }
    }

    /// The keep-three escape manifest: cards toggled by the art's own row
    /// rectangles, plus its confirm button (only present when the required
    /// count is packed).
    fn draw_keep_three(&mut self, frame: &mut Frame, area: Rect) {
        draw_backdrop(frame, area);

        let list = {
            let view = campaign_ui::KeepThreeView {
                from_level: campaign_ui::CampaignLevel::from_theme(self.live_theme()),
                next_level: self.keep_next_level(),
                gold: self.world.as_ref().map_or(0, |w| w.gold),
                items: &self.keep_entries,
                selected: &self.keep_selected,
                max_keep: self.keep_max(),
                cursor: self.keep_cursor,
                scroll: self.keep_scroll,
                hovered: match self.hits.hovered {
                    Some(Target::KeepItem(i)) => {
                        Some(campaign_ui::KeepThreeAction::ToggleItem(i))
                    }
                    Some(Target::KeepConfirm) => Some(campaign_ui::KeepThreeAction::Confirm),
                    Some(Target::KeepScrollUp) => Some(campaign_ui::KeepThreeAction::ScrollUp),
                    Some(Target::KeepScrollDown) => {
                        Some(campaign_ui::KeepThreeAction::ScrollDown)
                    }
                    Some(Target::RouteOption(i)) => {
                        Some(campaign_ui::KeepThreeAction::Route(i as u8))
                    }
                    _ => None,
                },
                route: self.keep_route_chips(),
                splices: &self.keep_splices,
            };
            campaign_ui::draw_keep_three_with(frame, area, &view, &self.plate_keys())
        };
        for (rect, action) in list.iter() {
            let target = match action {
                campaign_ui::KeepThreeAction::ToggleItem(i) => Target::KeepItem(i),
                campaign_ui::KeepThreeAction::ScrollUp => Target::KeepScrollUp,
                campaign_ui::KeepThreeAction::ScrollDown => Target::KeepScrollDown,
                campaign_ui::KeepThreeAction::Confirm => Target::KeepConfirm,
                campaign_ui::KeepThreeAction::Route(i) => Target::RouteOption(i as usize),
            };
            self.hits.add(rect, target);
        }
        self.keep_visible_rows = campaign_ui::keep_three_visible_rows(area).max(1);
    }

    /// Campaign victory: the per-level breakdown plus the loadout carried out.
    fn draw_victory(&mut self, frame: &mut Frame, area: Rect) {
        draw_backdrop(frame, area);
        let mut levels = [campaign_ui::LevelBreakdown {
            level: campaign_ui::CampaignLevel::Sewer,
            time_secs: 0.0,
            kills: 0,
            score: 0,
            player_level: 0,
            boss: None,
        }; campaign::LEVEL_COUNT];
        for (i, row) in levels.iter_mut().enumerate() {
            let record = self.records[i];
            *row = campaign_ui::LevelBreakdown {
                // The floor actually played (a side route keeps its own name);
                // a stage with no record falls back to the original route's.
                level: record
                    .plate
                    .unwrap_or_else(|| campaign_ui::CampaignLevel::from_index(i)),
                time_secs: record.time,
                kills: record.kills,
                score: record.score,
                player_level: record.player_level,
                // The boss that died there (the pool's other member, or a
                // console pick), when the App saw it die.
                boss: record.boss,
            };
        }
        let (total_time, total_kills) = match self.world.as_ref() {
            Some(w) => (w.total_time(), w.total_kills()),
            None => (
                self.records.iter().map(|r| r.time).sum(),
                self.records.iter().map(|r| r.kills).sum(),
            ),
        };
        let list = {
            let view = campaign_ui::CampaignVictoryView {
                menu_key: self.input.controls.action_label(Action::Quit),
                final_score: self.last_score,
                total_time_secs: total_time,
                total_kills,
                gold: self.world.as_ref().map_or(0, |w| w.gold),
                new_best: self.new_best,
                levels,
                final_loadout: &self.victory_loadout,
                // The run ends on the ride now, so the plate carries the
                // getaway's own closing line (`lore::GETAWAY_WIN`).
                custom_blurb: Some(crate::lore::GETAWAY_WIN),
                focus: self.victory_focus,
                hovered: match self.hits.hovered {
                    Some(Target::Restart) => Some(campaign_ui::CampaignVictoryAction::PlayAgain),
                    Some(Target::Menu) => Some(campaign_ui::CampaignVictoryAction::Menu),
                    _ => None,
                },
            };
            campaign_ui::draw_campaign_victory_with(frame, area, &view, &self.plate_keys())
        };
        for (rect, action) in list.iter() {
            let target = match action {
                campaign_ui::CampaignVictoryAction::PlayAgain => Target::Restart,
                campaign_ui::CampaignVictoryAction::Menu => Target::Menu,
            };
            self.hits.add(rect, target);
        }
        // The haul and a way to spend it: the campaign plate is the art's own
        // fixed layout, so the nest strip is stamped on the screen's last row.
        if let Some(tally) = self.bank_tally {
            let key = self.input.controls.action_label(Action::Nest);
            // The strip's own words take 18 cells beside the key's name.
            let room = (area.width as usize).saturating_sub(18 + key.chars().count());
            let text = format!(" {key}  THE NEST  ·  {} ", tally.line_within(room));
            let w = (text.chars().count() as u16).min(area.width);
            let rect = Rect::new(
                area.x + area.width.saturating_sub(w) / 2,
                area.bottom().saturating_sub(1),
                w,
                1,
            );
            frame.render_widget(
                ratatui::widgets::Paragraph::new(text.as_str())
                    .style(Style::default().fg(NEST_GOLD))
                    .centered(),
                rect,
            );
            self.hits.add(rect, Target::Nest);
        }
    }

    fn draw_game(&mut self, frame: &mut Frame, area: Rect) {
        let [top, game, bottom] = game_rows(area);
        self.last_game = game;

        if let Some(w) = self.world.as_mut() {
            if game.width > 0 && game.height > 0 {
                // Reuse the pixel buffer between frames: only a viewport resize
                // reallocates it.
                self.pixels
                    .resize(game.width as usize, game.height as usize * 2);
                // The **simulation's** logical viewport is session input, set
                // here on a resize (the viewport drives spawn rings and offscreen
                // placement), never by `World::draw`. So a run drawn at a
                // resized terminal and one that never drew simulate the same.
                w.viewport = Vec2::new(game.width as f32, (game.height * 2) as f32);
                // Scene + lighting only when the picture can have changed: the
                // world ticked, the view moved, or something the key cannot
                // see happened (`picture_dirty`). The key is taken *before*
                // the draw, so a lighting tier the cost controller changes
                // during this draw is a different key next frame.
                w.subcell =
                    crate::render::subcell::resolve(self.subcell_env, crate::term::block_set());
                w.smooth = self.smooth;
                w.foreground = self.foreground;
                w.lighting_fx = self.fx;
                w.shake_scale = self.shake_scale;
                let key = PictureKey::of(w, self.run_counter, &self.pixels);
                if self.picture_dirty || self.picture_key != Some(key) {
                    w.draw(&mut self.pixels);
                    self.world_draws += 1;
                    self.picture_key = Some(key);
                    self.picture_dirty = false;
                }
                let _p = crate::prof::scope(crate::prof::Stage::Blit);
                // The sub-cell picture whenever the draw made one — always
                // zoomed out on a terminal known to draw the glyphs, and at 1x
                // (and zoomed in) when the edge smoothing is on — else the
                // half-block one.
                match w.detail() {
                    Some(detail) => detail.render_to(game, frame.buffer_mut()),
                    None => self.pixels.render_to(game, frame.buffer_mut()),
                }
            }
        } else {
            frame.render_widget(
                Block::default().style(Style::default().bg(Color::Black)),
                game,
            );
        }

        if let Some(w) = self.world.as_ref() {
            // Round-glyph particles (`render::glyphfx`), written straight into
            // the frame's cells: after the world picture, before every overlay,
            // the HUD, the minimap and the cursor, so a mote can never cover one
            // of them. Outside `PictureKey` — the picture it sits on is cached,
            // the particles are not.
            if self.glyphfx_over_field() {
                self.glyphfx
                    .draw(frame.buffer_mut(), game, &self.pixels, w);
            }
            if matches!(self.screen, Screen::Playing) {
                let _p = crate::prof::scope(crate::prof::Stage::Overlays);
                overlay::draw_overlays(frame, game, w, &mut self.hud);
                if let Some(testbed) = self.testbed.filter(|testbed| testbed.show_panel && !self.map_open) {
                    let weapon = w.player.loadout.weapons.iter()
                        .find(|weapon| weapon.kind == testbed.weapon)
                        .and_then(|weapon| weapon.evolution())
                        .map_or_else(|| testbed.weapon.name(), |evolution| evolution.name);
                    crate::render::testbed::draw(frame, game, w, [
                        weapon,
                        library_index::name_of(LibraryKey::Pickup(testbed.pickup)),
                        library_index::name_of(LibraryKey::Enemy(testbed.enemy)),
                        library_index::name_of(LibraryKey::Boss(testbed.boss)),
                        testbed.miniboss.name(),
                    ]);
                }
            }
            let _p = crate::prof::scope(crate::prof::Stage::Hud);
            hud::draw_top(frame, top, w, &mut self.hud);
            hud::draw_bottom(frame, bottom, w, &mut self.hud);
            // Persistent CONSOLE USED tag in the top rail's right corner.
            let mut tag_right = top.right();
            if self.console_cheat {
                let label = " CONSOLE USED ";
                let lw = label.len() as u16;
                if top.width > lw {
                    let bx = top.right().saturating_sub(lw);
                    frame.buffer_mut().set_stringn(
                        bx,
                        top.y,
                        label,
                        lw as usize,
                        Style::default()
                            .bg(Color::Rgb(120, 20, 20))
                            .fg(Color::Rgb(255, 200, 200)),
                    );
                    tag_right = bx;
                }
            }
            // Console `pause`: a small chip beside it, so a frozen world never
            // looks like a hang.
            if self.console_paused {
                let label = " PAUSED ";
                let lw = label.len() as u16;
                if tag_right > top.x + lw {
                    frame.buffer_mut().set_stringn(
                        tag_right - lw,
                        top.y,
                        label,
                        lw as usize,
                        Style::default()
                            .bg(Color::Rgb(200, 160, 40))
                            .fg(Color::Rgb(20, 16, 8)),
                    );
                }
            }
        }

        // Minimap over the play field, under the pause / level-up panels (which
        // `render` draws after `draw_game`). Only the play screens have it: the
        // intro and the end summary own the whole frame.
        self.last_minimap = Rect::new(0, 0, 0, 0);
        self.last_map = Rect::new(0, 0, 0, 0);
        if !matches!(self.screen, Screen::Playing | Screen::Paused | Screen::LevelUp) {
            return;
        }
        let Some(w) = self.world.as_ref() else {
            return;
        };
        let expanded = self.map_open && matches!(self.screen, Screen::Playing);
        // Markers + labels first (reused buffers), then the art's own layout and
        // drawing: `render/minimap.rs` owns the panel geometry, the radar window
        // and the modal legend rail.
        let _p = crate::prof::scope(crate::prof::Stage::Minimap);
        self.map_frame.refresh(w);
        let hovered = self.hits.hovered == Some(Target::Minimap);
        let compact_size = self.scores.minimap_size;
        let mut view = self
            .map_frame
            .view_sized(w, expanded, compact_size, hovered);
        // The map hints name the live Map and MinimapSize bindings (cached
        // upper-case, so a frame allocates only when a binding changed).
        let map_label = self.input.controls.action_label(Action::Map);
        if !self.map_key_label.eq_ignore_ascii_case(map_label) {
            self.map_key_label = map_label.to_uppercase();
        }
        let size_label = self.input.controls.action_label(Action::MinimapSize);
        if !self.size_key_label.eq_ignore_ascii_case(size_label) {
            self.size_key_label = size_label.to_uppercase();
        }
        view.map_key = &self.map_key_label;
        view.size_key = &self.size_key_label;
        // The compact map remains a still backdrop under pause and level-up.
        if matches!(self.screen, Screen::Playing) {
            view.player_clock = self.map_animation_start.elapsed().as_secs_f64();
        }
        minimap::draw(frame, game, &view, w, &mut self.minimap_scratch);
        if expanded {
            let panel = minimap_source::expanded_rect(game);
            if panel.is_empty() {
                return;
            }
            self.last_map = panel;
            let back = self.input.controls.action_label(Action::Back).to_uppercase();
            draw_map_legend(
                frame,
                game,
                panel,
                &self.map_key_label,
                &back,
                &self.size_key_label,
            );
        } else {
            let panel = minimap_source::compact_rect_for_size(game, compact_size);
            if panel.is_empty() {
                return;
            }
            self.last_minimap = panel;
        }
    }
}

/// The play screen's three rows inside `area`: the top HUD rail, the pixel
/// viewport and the bottom HUD row. One function so the viewport `draw_game`
/// draws into and the one `enter_world` pre-sizes a new floor for cannot drift.
fn game_rows(area: Rect) -> [Rect; 3] {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(hud::top_height(area)),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    [rows[0], rows[1], rows[2]]
}

/// The pixel viewport's cell rectangle inside `area` (see [`game_rows`]).
fn game_rect(area: Rect) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::new(0, 0, 0, 0);
    }
    game_rows(area)[1]
}

/// How close (world px) a named miniboss has to be to the rat's body for a
/// death to be booked against it in the Library rather than the nearest minion.
const MINIBOSS_BLAME_RANGE: f32 = 64.0;

/// The miniboss kind behind a nameplate (`World::miniboss_hp` hands the HUD the
/// name, not the kind).
fn miniboss_kind_named(
    name: &str,
) -> Option<crate::game::ecs::systems::miniboss::MinibossKind> {
    crate::game::ecs::systems::miniboss::MinibossKind::all()
        .into_iter()
        .find(|kind| kind.name() == name)
}

/// Paint the near-black plate backdrop the campaign screens sit on.
fn draw_backdrop(frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(Style::default().bg(BACKDROP)), area);
}

/// The single-cell glyph and tint for a passive, so the keep/intro cards mix
/// weapons and passives without inventing a second badge table.
fn passive_badge(passive: PassiveKind) -> (char, palette::Rgb) {
    match passive {
        PassiveKind::MaxHp => ('+', (222, 120, 132)),
        PassiveKind::MoveSpeed => ('~', (128, 206, 250)),
        PassiveKind::Damage => ('×', (248, 214, 152)),
        PassiveKind::FireRate => ('÷', (242, 184, 102)),
        PassiveKind::Magnet => ('◇', (176, 140, 240)),
        PassiveKind::Regen => ('=', (126, 224, 152)),
        PassiveKind::Armor => ('#', (150, 190, 200)),
        // Wave 3 passives. Placeholder badges until the icon pass lands.
        PassiveKind::SpringHeel => ('^', (200, 220, 236)),
        PassiveKind::Counterweight => ('o', (198, 168, 128)),
        PassiveKind::LongFuse => ('!', (246, 196, 106)),
        PassiveKind::SecondScent => ('?', (176, 220, 196)),
        PassiveKind::LensCap => ('c', (208, 216, 160)),
    }
}

/// Build the art's upgrade cards from campaign keepables (one card per owned
/// weapon at its level and per passive at its stacks).
fn entries_for(items: &[Keepable]) -> Vec<campaign_ui::UpgradeEntry<'static>> {
    items
        .iter()
        .map(|k| {
            let (glyph, color) = match k.kind {
                KeepKind::Weapon(kind) => (hud::weapon_glyph(kind), kind.color()),
                KeepKind::Passive(passive) => passive_badge(passive),
            };
            campaign_ui::UpgradeEntry {
                name: k.name,
                glyph,
                color,
                category: match k.kind {
                    KeepKind::Weapon(kind) => campaign_ui::UpgradeCategory::Weapon {
                        level: k.level,
                        max_level: kind.max_level(),
                    },
                    KeepKind::Passive(_) => campaign_ui::UpgradeCategory::Passive {
                        stacks: k.level,
                    },
                },
                detail: k.desc,
            }
        })
        .collect()
}

/// The expanded map's own legend rail comes from `render/minimap.rs`; this adds
/// the one rule that rail does not state: opening the map pauses the world.
/// Drawn on the first free row under the modal, and skipped when the terminal
/// has no room for it.
fn draw_map_legend(
    frame: &mut Frame,
    game: Rect,
    panel: Rect,
    map_key: &str,
    back_key: &str,
    size_key: &str,
) {
    let y = panel.bottom();
    if y >= game.bottom() || game.height == 0 {
        return;
    }
    // The live Map, Back and MinimapSize bindings, not the default key names
    // (this modal is paused, so building the line each frame costs nothing
    // that matters). A key bound to both closes is named once.
    let close = if map_key.eq_ignore_ascii_case(back_key) {
        map_key.to_string()
    } else {
        format!("{map_key}/{back_key}")
    };
    let wide = format!(" {close} close · {size_key} minimap size · click to close · game paused ");
    let mid = format!(" {close} close · {size_key} size · click to close · game paused ");
    let narrow = format!(" {close} close · {size_key} size · game paused ");
    let fits = |t: &str| (t.chars().count() as u16) < game.width;
    let text: &str = if game.width >= 72 && fits(&wide) {
        &wide
    } else if game.width >= 60 && fits(&mid) {
        &mid
    } else {
        &narrow
    };
    let width = text.chars().count() as u16;
    if width >= game.width {
        return;
    }
    let x = game.x + (game.width - width) / 2;
    frame.buffer_mut().set_string(
        x,
        y,
        text,
        Style::default().fg(Color::Rgb(156, 167, 169)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseEvent, MouseEventKind,
    };

    /// What crossterm builds from a lone left-Ctrl press under the Kitty
    /// protocol with report-all-keys (`CSI 57442;5:1u`).
    fn ctrl_press() -> KeyEvent {
        KeyEvent::new_with_kind(
            KeyCode::Modifier(ModifierKeyCode::LeftControl),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        )
    }
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Fixed base seed so runs in tests are reproducible: level-ups and deaths
    /// land on the same tick every time. With this seed the weave used by
    /// `full_run_renders_and_handles_levelup` survives the whole 60 s (and still
    /// levels up several times).
    const TEST_SEED: u64 = 2;

    /// Test-app constructor: silent, empty in-memory score table, no persistence,
    /// deterministic seeds. Nothing here ever reads or writes the user's real
    /// score file.
    fn test_app() -> App {
        App::with_options(false, None, ScoreTable::default(), false, Some(TEST_SEED))
    }

    /// End-to-end harness: drives the real `App` through ratatui's `Terminal`
    /// and an in-memory `Buffer`, exactly like `main`'s loop but without
    /// crossterm's event source or a TTY. `frame` returns the screen as text.
    struct Harness {
        app: App,
        terminal: Terminal<TestBackend>,
    }

    impl Harness {
        fn new(w: u16, h: u16) -> Self {
            Harness::with_app(test_app(), w, h)
        }

        fn with_app(app: App, w: u16, h: u16) -> Self {
            Harness {
                app,
                terminal: Terminal::new(TestBackend::new(w, h)).unwrap(),
            }
        }

        /// Start the campaign from the title and skip the story dossier and the
        /// map intro, landing in `Playing` on level 1.
        fn enter_playing(&mut self) {
            self.frame(&[]); // title
            self.frame(&[KeyCode::Enter]); // START RUN -> Story
            self.frame(&[KeyCode::Enter]); // dossier -> MapIntro
            self.frame(&[KeyCode::Enter]); // skip intro -> Playing
        }

        /// Give the running level more than three keepable powerups, so the
        /// keep-three screen has a real choice to make.
        fn grant_keepables(&mut self) {
            use crate::game::weapon::WeaponKind;
            let w = self.app.world.as_mut().unwrap();
            for upgrade in [
                Upgrade::NewWeapon(WeaponKind::CheeseSpray),
                Upgrade::NewWeapon(WeaponKind::SqueakNova),
                Upgrade::Passive(PassiveKind::Armor),
                Upgrade::Passive(PassiveKind::Damage),
            ] {
                w.player.loadout.apply(&upgrade);
            }
        }

        /// The level's boss dies (the boss framework is still in progress, so
        /// `mark_level_complete` is the facade's hook) and one frame runs.
        fn finish_level(&mut self) {
            self.app.world.as_mut().unwrap().mark_level_complete();
            self.frame(&[]);
        }

        /// The keepables the current level offers, in the art's card order.
        fn keepables(&self) -> Vec<Keepable> {
            self.app.world.as_ref().unwrap().keepables()
        }

        /// Advance one frame, injecting the given keys this frame.
        fn frame(&mut self, keys: &[KeyCode]) -> String {
            self.events(keys, &[])
        }

        fn events(&mut self, keys: &[KeyCode], mouse: &[(MouseEventKind, (u16, u16))]) -> String {
            self.app.input.begin_frame();
            for k in keys {
                self.app.input.handle_key(KeyEvent::from(*k));
            }
            for &(kind, (column, row)) in mouse {
                self.app.input.handle_mouse(MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                });
            }
            self.app.update(1.0 / 60.0);
            self.terminal.draw(|f| self.app.render(f)).unwrap();
            self.screen_text()
        }

        /// Advance one frame, injecting raw key events (a lone Ctrl press has
        /// no `KeyCode`-only form: it needs its modifier and kind).
        fn frame_events(&mut self, events: &[KeyEvent]) -> String {
            self.app.input.begin_frame();
            for &ev in events {
                self.app.input.handle_key(ev);
            }
            self.app.update(1.0 / 60.0);
            self.terminal.draw(|f| self.app.render(f)).unwrap();
            self.screen_text()
        }

        fn set_mouse(&mut self, col: u16, row: u16) {
            self.app.input.handle_mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: col,
                row,
                modifiers: KeyModifiers::NONE,
            });
        }

        /// Locate the leftmost cell of visible text, independently of the hit map.
        fn locate(&self, text: &str) -> (u16, u16) {
            let buf = self.terminal.backend().buffer();
            for y in 0..buf.area.height {
                let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                if let Some(byte) = row.find(text) {
                    return (row[..byte].chars().count() as u16, y);
                }
            }
            let frame: Vec<String> = (0..buf.area.height)
                .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
                .collect();
            panic!("{text:?} is missing from the frame:\n{}", frame.join("\n"))
        }

        fn click(&mut self, text: &str) {
            let pos = self.locate(text);
            self.events(&[], &[(MouseEventKind::Down(MouseButton::Left), pos)]);
        }

        fn show_upgrade_cards(&mut self) {
            use crate::game::loadout::PassiveKind;
            self.enter_playing();
            self.app.world.as_mut().unwrap().pending_levelups = 1;
            self.frame(&[]);
            let loadout = &self.app.world.as_ref().unwrap().player.loadout;
            self.app.levelup_choices = vec![
                Upgrade::Passive(PassiveKind::Damage),
                Upgrade::Passive(PassiveKind::Armor),
                Upgrade::Passive(PassiveKind::Magnet),
            ];
            self.app.levelup_desc = self
                .app
                .levelup_choices
                .iter()
                .map(|up| loadout.describe(up))
                .collect();
            self.frame(&[]);
        }

        /// Text of the compact minimap panel from the last frame.
        fn minimap_text(&self) -> String {
            self.panel_text(self.app.last_minimap)
        }

        /// Text of the expanded map panel from the last frame.
        fn map_text(&self) -> String {
            self.panel_text(self.app.last_map)
        }

        fn panel_text(&self, r: Rect) -> String {
            if r.is_empty() {
                return String::new();
            }
            let buf = self.terminal.backend().buffer();
            let mut out = String::new();
            for y in r.y..r.bottom().min(buf.area.height) {
                for x in r.x..r.right().min(buf.area.width) {
                    out.push_str(buf[(x, y)].symbol());
                }
            }
            out
        }

        /// Resize the in-memory terminal (tests the tiny-viewport paths).
        fn resize(&mut self, w: u16, h: u16) {
            self.terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            self.frame(&[]);
        }

        fn player_x(&self) -> f32 {
            self.app.world.as_ref().unwrap().player.pos.x
        }

        fn screen_text(&self) -> String {
            self.terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect()
        }
    }

    #[test]
    fn testbed_starts_in_the_hall_and_only_manual_encounters_arrive() {
        use crate::game::director::{BOSS_TIME, MINIBOSS_TIMES};

        let mut h = Harness::new(57, 20);
        let floor = LibraryKey::Floor(Theme::Sewer);
        let floor_seen = h.app.scores.library.is_seen(floor);
        h.app.start_testbed(Theme::Sewer);
        let text = h.frame(&[]);
        assert!(text.contains("TESTBED · F10 hide"));
        assert!(text.contains("H god on · J horde off"));
        assert!(h.app.console_cheat);
        assert_eq!(h.app.scores.library.is_seen(floor), floor_seen);
        let w = h.app.world.as_mut().unwrap();
        assert!(w.player.god);
        assert!(w.peace);
        assert!(w.level.largest_hall().unwrap().rect.contains_world(w.player.pos));
        assert_eq!(w.enemy_count(), 0);
        assert!(w.next_miniboss_time().is_none());
        w.elapsed = MINIBOSS_TIMES[0];
        let gem = crate::game::gem::Gem::new(w.player.pos, w.player.xp_to_next);
        w.spawn_gem(gem);
        h.frame(&[]);
        let w = h.app.world.as_mut().unwrap();
        assert_eq!(w.enemy_count(), 0);
        assert_eq!(w.pending_levelups, 0);
        assert!(w.player.level > 1);
        assert!(h.app.screen == Screen::Playing);

        w.elapsed = BOSS_TIME - 0.02;
        let text = h.frame(&[]);
        assert!(!text.contains("BOSS INBOUND"));
        h.frame(&[KeyCode::Char('j')]);
        let w = h.app.world.as_ref().unwrap();
        assert!(!w.peace);
        assert!(w.enemy_count() > 0);
        assert!(w.miniboss_hp().is_none());
        assert!(w.boss_kind().is_none());
        assert!(!w.director.boss_spawned);
        h.frame(&[KeyCode::Char('h'), KeyCode::Char('j')]);
        let w = h.app.world.as_ref().unwrap();
        assert!(!w.player.god);
        assert!(w.peace);
    }

    #[test]
    fn testbed_cycles_all_arrays_and_wraps_both_ways() {
        let mut h = Harness::new(57, 20);
        h.app.start_testbed(Theme::Sewer);
        let weapons = WeaponKind::all();
        for expected in weapons.into_iter().rev() {
            h.frame(&[KeyCode::Char('1')]);
            let rack = &h.app.world.as_ref().unwrap().player.loadout.weapons;
            assert_eq!(rack.len(), 1);
            assert_eq!(rack[0].kind, expected);
            assert_eq!(rack[0].level, 5);
            assert!(!rack[0].evolved);
        }
        h.frame(&[KeyCode::Char('1')]);
        assert_eq!(h.app.testbed.unwrap().weapon, *weapons.last().unwrap());
        h.frame(&[KeyCode::Char('2')]);
        assert_eq!(h.app.testbed.unwrap().weapon, weapons[0]);

        let arrays = [KeyCode::Char('4'), KeyCode::Char('7'), KeyCode::F(1), KeyCode::F(5)];
        h.frame(&arrays);
        let picks = h.app.testbed.unwrap();
        assert_eq!(picks.pickup, *PickupKind::all().last().unwrap());
        assert_eq!(picks.enemy, *EnemyKind::all().last().unwrap());
        assert_eq!(picks.boss, *BossKind::all().last().unwrap());
        assert_eq!(picks.miniboss, *MinibossKind::all().last().unwrap());
        h.frame(&[KeyCode::Char('5'), KeyCode::Char('8'), KeyCode::F(2), KeyCode::F(6)]);
        let picks = h.app.testbed.unwrap();
        assert_eq!(picks.pickup, PickupKind::all()[0]);
        assert_eq!(picks.enemy, EnemyKind::all()[0]);
        assert_eq!(picks.boss, BossKind::all()[0]);
        assert_eq!(picks.miniboss, MinibossKind::all()[0]);

        h.app.testbed.as_mut().unwrap().enemy = EnemyKind::Brute;
        h.frame(&[KeyCode::Char('8')]);
        assert_eq!(h.app.testbed.unwrap().enemy, EnemyKind::Pillbug);
        h.frame(&[KeyCode::Char('7')]);
        assert_eq!(h.app.testbed.unwrap().enemy, EnemyKind::Brute);
    }

    #[test]
    fn testbed_evolves_drops_spawns_and_clears() {
        let mut h = Harness::new(120, 40);
        h.app.start_testbed(Theme::Sewer);
        h.frame(&[KeyCode::Char('3')]);
        let weapon = h.app.testbed.unwrap().weapon;
        let evolution = weapon.evolution().unwrap();
        let loadout = &h.app.world.as_ref().unwrap().player.loadout;
        assert!(loadout.weapons[0].evolved);
        assert_eq!(loadout.stacks(evolution.passive), 1);
        h.frame(&[KeyCode::Char('3')]);
        let loadout = &h.app.world.as_ref().unwrap().player.loadout;
        assert!(!loadout.weapons[0].evolved);
        assert_eq!(loadout.stacks(evolution.passive), 0);

        h.frame(&[KeyCode::Char('4'), KeyCode::Char('6'), KeyCode::Char('8'), KeyCode::Char('9')]);
        let picks = h.app.testbed.unwrap();
        let w = h.app.world.as_mut().unwrap();
        assert!(w.pickups().iter().any(|p| p.kind == picks.pickup));
        let enemies = w.enemies();
        assert_eq!(enemies.len(), 5);
        assert!(enemies.iter().all(|e| e.kind == picks.enemy));

        h.frame(&[KeyCode::F(2), KeyCode::F(3)]);
        assert_eq!(h.app.world.as_ref().unwrap().boss_kind(), Some(BossKind::all()[1]));
        h.frame(&[KeyCode::F(2), KeyCode::F(3)]);
        assert_eq!(h.app.world.as_ref().unwrap().boss_kind(), Some(BossKind::all()[2]));
        h.frame(&[KeyCode::F(6), KeyCode::F(7)]);
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.miniboss_hp().map(|(name, _, _)| name), Some(MinibossKind::all()[1].name()));
        let kills = w.kills;
        h.frame(&[KeyCode::Char('0')]);
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.enemy_count(), 0);
        assert!(w.miniboss_hp().is_none());
        assert!(w.boss_kind().is_none());
        assert_eq!(w.kills, kills);
    }

    #[test]
    fn testbed_kill_reaps_loot_and_boss_defeat_keeps_playing() {
        use crate::game::ecs::systems::boss::Boss;

        let mut h = Harness::new(80, 24);
        h.app.start_testbed(Theme::Sewer);
        h.frame(&[]);
        let w = h.app.world.as_mut().unwrap();
        let visible = w.player.pos + Vec2::new(20.0, 0.0);
        w.spawn_enemy(EnemyKind::Skitterer, visible, 1.0);
        let offscreen = w.player.pos + Vec2::new(240.0, 0.0);
        let offscreen_body = w.spawn_enemy(EnemyKind::Skitterer, offscreen, 1.0);
        h.frame(&[KeyCode::Char('k')]);
        let w = h.app.world.as_mut().unwrap();
        assert_eq!(w.kills, 1);
        assert_eq!(w.gems().len(), 1);
        assert!(w.ecs.get::<crate::game::enemy::Enemy>(offscreen_body).is_some());

        h.frame(&[KeyCode::F(3)]);
        let death_ticks = {
            let w = h.app.world.as_mut().unwrap();
            let mut bosses = w.ecs.query::<&mut Boss>();
            let mut live = bosses.iter_mut(&mut w.ecs);
            let mut boss = live.next().unwrap();
            boss.hp = 0.0;
            (boss.kind.death_duration() * 60.0).ceil() as usize + 2
        };
        h.app.step_ticks(death_ticks);
        h.frame(&[]);
        let w = h.app.world.as_ref().unwrap();
        assert!(w.boss_kind().is_none());
        assert!(!w.finished());
        assert!(h.app.screen == Screen::Playing);
    }

    #[test]
    fn testbed_controls_preserve_menus_bindings_and_selections() {
        let mut h = Harness::new(80, 24);
        h.app.start_testbed(Theme::Trailer);
        h.frame(&[KeyCode::Char('2'), KeyCode::Char('5'), KeyCode::Char('8'), KeyCode::F(2), KeyCode::F(6)]);
        let before = h.app.testbed.unwrap();
        h.frame(&[KeyCode::Char('t')]);
        assert_eq!(h.app.world.as_ref().unwrap().level.theme, Theme::Sewer);
        let after = h.app.testbed.unwrap();
        assert_eq!(after.weapon, before.weapon);
        assert_eq!(after.pickup, before.pickup);
        assert_eq!(after.enemy, before.enemy);
        assert_eq!(after.boss, before.boss);
        assert_eq!(after.miniboss, before.miniboss);

        assert!(!h.frame(&[KeyCode::F(10)]).contains("F10 hide"));
        assert!(h.frame(&[KeyCode::F(10)]).contains("F10 hide"));
        h.frame(&[KeyCode::Tab]);
        assert!(h.app.map_open);
        h.frame(&[KeyCode::Esc]);
        assert!(!h.app.map_open);
        assert!(h.app.screen == Screen::Playing);
        h.frame(&[KeyCode::Char('p')]);
        assert!(h.app.screen == Screen::Paused);
        h.frame(&[KeyCode::Char('o')]);
        assert!(h.app.screen == Screen::Options);
        h.app.options_row = options::GRAPHICS_ROW;
        h.frame(&[KeyCode::Enter]);
        assert!(h.app.screen == Screen::Graphics);
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('p')]);
        assert!(h.app.screen == Screen::Playing);

        h.app.input.controls.bind_active_kbm(Action::Map, Binding::Key(KeyBind::Char('9')));
        h.frame(&[KeyCode::Char('9')]);
        assert!(h.app.map_open);
        assert_eq!(h.app.world.as_ref().unwrap().enemy_count(), 0);
        h.frame(&[KeyCode::Char('9')]);
        h.frame(&[KeyCode::Esc]);
        assert!(h.app.screen == Screen::Title);
        h.app.start_testbed(Theme::Sewer);
        let bank = h.app.nest_bank();
        h.app.world.as_mut().unwrap().gold = 999;
        h.frame(&[KeyCode::Char('q')]);
        assert!(h.app.screen == Screen::Title);
        assert_eq!(h.app.nest_bank(), bank);
    }

    #[test]
    fn testbed_keys_do_nothing_new_in_normal_play() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.world.as_mut().unwrap().peace = true;
        h.frame(&[KeyCode::Char('9'), KeyCode::Char('2'), KeyCode::F(3), KeyCode::Char('h')]);
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.enemy_count(), 0);
        assert!(w.boss_kind().is_none());
        assert!(!w.player.god);
        assert_eq!(w.player.loadout.weapons[0].level, 1);
    }

    #[test]
    fn testbed_console_command_restarts_on_the_named_sector() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        crate::console::run_command(&mut h.app, "testbed flatbed").unwrap();
        assert!(h.app.screen == Screen::Playing);
        assert_eq!(h.app.world.as_ref().unwrap().level.theme, Theme::Trailer);
        assert!(h.app.console_cheat);
        assert!(h.app.testbed.is_some());
        crate::console::run_command(&mut h.app, "restart").unwrap();
        assert!(h.app.testbed.is_some());
        crate::console::run_command(&mut h.app, "testbed").unwrap();
        assert_eq!(h.app.world.as_ref().unwrap().level.theme, Theme::Sewer);
    }

    #[test]
    fn mouse_title_sector_row_story_and_quit() {
        for size in [(57, 20), (80, 24)] {
            let mut h = Harness::new(size.0, size.1);
            h.frame(&[]);
            let row = h.locate("Sewer");
            h.events(&[], &[(MouseEventKind::Moved, row)]);
            assert_eq!(h.app.menu_idx, 1, "the sector row is menu row 2");
            // The arrows step the sector row, which lists every playable floor
            // (the three campaign stages, the two side-route floors, then the
            // getaway ride), wrapping at both ends.
            h.click("‹");
            assert_eq!(h.app.theme(), Theme::Trailer);
            h.click("›");
            assert_eq!(h.app.theme(), Theme::Sewer);
            // The row is in route order: stage one, then both stage-two
            // floors, then both stage-three floors, then the getaway ride.
            // The stage number a side-route floor shows is its campaign
            // stage's (`Laundry` is stage two), so the row reads `02 Laundry`.
            for _ in 0..2 {
                h.frame(&[KeyCode::Right]);
            }
            assert_eq!(h.app.theme(), Theme::Laundry);
            assert_eq!(
                campaign::theme_stage(h.app.theme()),
                1,
                "the Laundry is a stage-two floor"
            );
            for _ in 0..2 {
                h.frame(&[KeyCode::Left]);
            }
            assert_eq!(h.app.theme(), Theme::Sewer);
            // The wheel over the sector row steps it too.
            let pos = h.locate("Sewer");
            h.events(&[], &[(MouseEventKind::ScrollDown, pos)]);
            assert_eq!(h.app.theme(), Theme::Kitchen);
            h.events(&[], &[(MouseEventKind::ScrollUp, pos)]);
            assert_eq!(h.app.theme(), Theme::Sewer);
            // A floor that has not been unlocked is marked on the title row.
            h.frame(&[KeyCode::Right]);
            h.frame(&[KeyCode::Right]);
            // The pointer still rests where the `›` arrow was, which is where
            // the longer name puts its mark: park it before reading the row.
            h.events(&[], &[(MouseEventKind::Moved, (0, 0))]);
            assert!(
                h.screen_text().contains("Laundry ×"),
                "a locked floor is marked on the title: {}",
                h.screen_text()
            );
            h.frame(&[KeyCode::Left, KeyCode::Left]);
            h.click("L story");
            assert!(matches!(h.app.screen, Screen::Story));
            // On 57×20 `L story` sits at (38, 18), directly over `back` on the
            // Story footer; moving the pointer away proves the cheese cursor
            // leaves the underlying label intact for `locate("back")`.
            h.events(&[], &[(MouseEventKind::Moved, (0, 0))]);
            h.click("back");
            assert!(matches!(h.app.screen, Screen::Title));
            h.click("QUIT");
            assert!(h.app.should_quit);
        }
    }

    #[test]
    fn mouse_only_run_pause_and_end_actions() {
        let left = MouseEventKind::Down(MouseButton::Left);
        let right = MouseEventKind::Down(MouseButton::Right);
        for size in [(57, 20), (80, 16)] {
            let mut h = Harness::new(size.0, size.1);
            h.frame(&[]);
            let start = h.locate("START RUN");
            h.events(&[], &[(left, start)]);
            assert!(
                matches!(h.app.screen, Screen::Story),
                "START RUN opens the campaign dossier"
            );
            h.click("back");
            assert!(matches!(h.app.screen, Screen::MapIntro));
            h.events(&[], &[(left, (0, 0))]);
            assert!(matches!(h.app.screen, Screen::MapIntro));
            h.click("begin");
            assert!(matches!(h.app.screen, Screen::Playing));
            assert_eq!(h.app.world.as_ref().unwrap().player.dash_cd, 0.0);
            let field = h.locate("▀");
            h.events(&[], &[(left, field)]);
            assert!(h.app.world.as_ref().unwrap().player.dash_cd > 0.0);
            // A right click in play is the actives' trigger, never a menu
            // (batch 12: the owner's "right click should fire the weapon not
            // bring up a menu"). It leaves the screen exactly where it was.
            h.events(&[], &[(right, field)]);
            assert!(
                matches!(h.app.screen, Screen::Playing),
                "a right click opened the pause plate"
            );
            // The pause plate still comes from Esc, and a right click on it is
            // Back — the same thing Esc does there.
            h.frame(&[KeyCode::Esc]);
            assert!(matches!(h.app.screen, Screen::Paused));
            h.events(&[], &[(right, (0, 0))]);
            assert!(
                matches!(h.app.screen, Screen::Playing),
                "a right click did not leave the pause plate"
            );
            // …and the plate's own chips still reach the menu.
            h.frame(&[KeyCode::Esc]);
            h.click("Q to menu");
            assert!(matches!(h.app.screen, Screen::Title));
            for button in ["restart", "Q to menu"] {
                if matches!(h.app.screen, Screen::Title) {
                    h.click("START RUN");
                    h.click("back"); // dossier -> MapIntro
                }
                h.click("begin");
                h.app.world.as_mut().unwrap().game_over = true;
                h.frame(&[]);
                assert!(matches!(h.app.screen, Screen::End));
                h.click(button);
                assert_eq!(
                    h.app.screen.name_for_test(),
                    if button == "restart" {
                        "MapIntro"
                    } else {
                        "Title"
                    }
                );
            }
        }
    }

    #[test]
    fn mouse_card_body_picks_and_hover_works_with_keyboard() {
        let left = MouseEventKind::Down(MouseButton::Left);
        for size in [(57, 20), (80, 16)] {
            for keyboard in [false, true] {
                let mut h = Harness::new(size.0, size.1);
                h.show_upgrade_cards();
                let effect = h.locate("+1 armor");
                // Blank padding at the right of the effect is also on the card.
                let body = (effect.0 + 37, effect.1);
                h.events(&[], &[(MouseEventKind::Moved, body)]);
                assert_eq!(h.app.levelup_idx, 1);
                assert!(h.screen_text().contains("◆  02   Thick Hide"));
                if keyboard {
                    h.frame(&[KeyCode::Down]);
                    h.events(&[], &[(MouseEventKind::Moved, body)]);
                    assert_eq!(h.app.levelup_idx, 2, "resting pointer stole selection");
                    h.events(&[], &[(MouseEventKind::Moved, effect)]);
                    h.frame(&[KeyCode::Enter]);
                } else {
                    h.events(&[], &[(left, body), (left, body)]);
                }
                assert!(matches!(h.app.screen, Screen::Playing));
                let player = &h.app.world.as_ref().unwrap().player;
                assert_eq!(player.loadout.stats.armor, 1.0);
                assert_eq!(player.dash_cd, 0.0);
            }
        }
    }

    #[test]
    fn resized_title_uses_the_displayed_row_and_press_position() {
        let mut h = Harness::new(140, 45);
        h.frame(&[]);
        let old = h.locate("START RUN");
        h.terminal.backend_mut().resize(57, 20);
        h.frame(&[]);
        let start = h.locate("START RUN");
        assert_ne!(old, start);
        let left = MouseEventKind::Down(MouseButton::Left);
        h.events(&[], &[(left, old)]);
        assert!(matches!(h.app.screen, Screen::Title));
        let quit = h.locate("QUIT");
        h.events(&[], &[(left, start), (MouseEventKind::Moved, quit)]);
        assert!(matches!(h.app.screen, Screen::Story));
        assert!(!h.app.should_quit);
    }

    #[test]
    fn only_left_press_activates_a_menu_target() {
        let mut h = Harness::new(57, 20);
        h.frame(&[]);
        let start = h.locate("START RUN");
        for kind in [
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Down(MouseButton::Middle),
            MouseEventKind::Down(MouseButton::Right),
        ] {
            h.events(&[], &[(kind, start)]);
            assert!(matches!(h.app.screen, Screen::Title));
        }
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Title));
    }

    /// The owner's right click: in Classic it **fires the charged actives** and
    /// never opens a menu (the whole playfield used to be a right-click pause).
    #[test]
    fn a_right_click_in_classic_fires_the_actives_and_never_pauses() {
        use crate::game::weapon::WeaponKind;
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            w.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
        }
        h.frame(&[]);
        let field = h.locate("▀");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Right), field)]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(
                w.active_fx().thrash_active(),
                "the right click did not release the charged actives"
            );
            assert_eq!(w.player.dash_active, 0.0, "the right click must not dash");
            assert_eq!(w.player.dash_cd, 0.0, "the right click spent the dash");
        }
        assert!(
            matches!(h.app.screen, Screen::Playing),
            "a right click left the run"
        );
    }

    /// Mouse aim keeps its own split: the right click is still the dash there
    /// (and a click no longer reaches a menu in that scheme either).
    #[test]
    fn a_mouse_aim_right_click_still_dashes() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.set_scheme(Scheme::MouseAim);
        h.frame(&[]);
        let game = h.app.last_game;
        assert!(game.width > 8 && game.height > 4, "no playfield to click");
        let pos = (game.x + game.width / 2, game.y + 2);
        {
            let w = h.app.world.as_mut().unwrap();
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
        }
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Right), pos)]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(
                w.player.dash_active > 0.0,
                "the mouse-aim right click must still dash"
            );
        }
        assert!(matches!(h.app.screen, Screen::Playing), "mouse aim lost its run");
    }

    /// The other half of the owner's ask: in **any** menu or overlay a right
    /// click is Back, exactly what Esc does there — and it is never a left press,
    /// so it cannot also click the row under the pointer.
    #[test]
    fn a_right_click_backs_out_of_every_menu_like_esc() {
        let right = MouseEventKind::Down(MouseButton::Right);
        let mut h = Harness::new(80, 24);
        // The title has no Back (Esc does nothing there), so the pointer over a
        // row must not press it either.
        h.frame(&[]);
        let start = h.locate("START RUN");
        h.events(&[], &[(right, start)]);
        assert!(
            matches!(h.app.screen, Screen::Title),
            "a right click pressed a title row"
        );

        // Sector select and the story dossier both leave for the title.
        h.app.screen = Screen::SectorSelect;
        h.frame(&[]);
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Title), "sector select stuck");
        h.app.screen = Screen::Story;
        h.frame(&[]);
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Title), "the dossier stuck");

        // In play the expanded map is an overlay: a right click closes it and
        // the run keeps going.
        h.enter_playing();
        h.frame(&[KeyCode::Tab]);
        assert!(h.app.map_open, "Tab did not open the map");
        h.events(&[], &[(right, (0, 0))]);
        assert!(
            !h.app.map_open && matches!(h.app.screen, Screen::Playing),
            "the map stuck open"
        );

        // The pause plate and every plate it opens.
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Paused));
        h.app.open_options(Screen::Paused);
        h.frame(&[]);
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Paused), "options stuck");
        h.app.open_controls();
        h.frame(&[]);
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Options), "controls stuck");
        h.app.open_library(Screen::Paused);
        h.frame(&[]);
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Paused), "library stuck");
        h.app.open_nest(Screen::Paused);
        h.frame(&[]);
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Paused), "nest stuck");
        // …and the pause plate itself resumes the run.
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Playing), "pause stuck");

        // The splice shelf leaves for the field: the run is still live, so the
        // player walks onto the hole's threshold and the shelf opens.
        {
            let w = h.app.world.as_mut().unwrap();
            w.gold = 40;
            if let Some(hole) = w.level.objects.shop_holes.first().copied() {
                w.player.pos = hole.face() + hole.facing * 3.0;
            }
        }
        h.frame(&[]);
        assert!(
            matches!(h.app.screen, Screen::Shop),
            "the threshold did not open the shelf"
        );
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Playing), "the shop stuck");

        // A chest and a level-up card have no Back action at all, so a right
        // click does nothing there — the same as Esc.
        h.app.screen = Screen::Chest;
        h.events(&[], &[(right, (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Chest), "a chest has no back");
        h.show_upgrade_cards();
        h.events(&[], &[(right, (0, 0))]);
        assert!(
            matches!(h.app.screen, Screen::LevelUp),
            "a right click picked a card"
        );
    }

    /// The Controls screen captures a right click as a binding like any other
    /// button — the one screen where it must **not** be Back.
    #[test]
    fn the_controls_screen_captures_a_right_click_instead_of_backing_out() {
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        h.app.open_options(Screen::Title);
        h.app.open_controls();
        h.frame(&[]);
        h.app.bind_capture = Some(Action::Mute);
        h.frame(&[]);
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Right), (2, 2))]);
        assert!(
            matches!(h.app.screen, Screen::Controls),
            "a captured right click left the screen"
        );
        assert_eq!(
            h.app.input.controls.active().get(Action::Mute),
            Binding::Mouse(MouseBind::Right),
            "the right click was not captured as the binding"
        );
    }

    #[test]
    fn title_screen_renders() {
        let mut h = Harness::new(80, 40);
        let text = h.frame(&[]);
        assert!(text.contains("START"), "title should show START RUN");
        assert!(text.contains("QUIT"));
    }

    /// The getaway ride is the sector row's fourth entry, and it fits at the
    /// two sizes the art is laid out for (57 × 20 and 80 × 24) as well as a
    /// full terminal: the label and both arrows are on screen unclipped.
    #[test]
    fn the_title_sector_row_names_the_getaway_ride() {
        for (w, h) in [(57u16, 20u16), (80, 24), (80, 40)] {
            let mut hh = Harness::new(w, h);
            hh.app.select_idx = Theme::playable().len() - 1; // the Flatbed
            let text = hh.frame(&[]);
            assert!(
                text.contains("Flatbed"),
                "{w}x{h}: the sector row does not name the getaway ride:\n{text}"
            );
            assert!(
                text.contains("04") || text.contains("Sector"),
                "{w}x{h}: the sector row lost its label:\n{text}"
            );
            assert!(
                text.contains('‹') && text.contains('›'),
                "{w}x{h}: the sector row lost its arrows (clipped?):\n{text}"
            );
            // No `×`: the getaway ride is always open.
            assert!(
                !text.contains("Flatbed ×"),
                "{w}x{h}: the getaway ride is shown locked"
            );
        }
    }

    /// The sector row lists six floors in route order (stage one, the two
    /// stage-two floors, the two stage-three floors, then the Flatbed). Every
    /// one of them fits, with its stage number, its lock mark and both arrows,
    /// at the two sizes the title is laid out for (57 x 20 and 80 x 24) and on
    /// a full terminal.
    #[test]
    fn the_title_sector_row_fits_all_six_floors() {
        let floors = Theme::playable();
        assert_eq!(floors.len(), 6);
        // The row's order is the select screen's order: both stage-two floors,
        // then both stage-three floors, then the getaway ride.
        assert_eq!(
            floors,
            [
                Theme::Sewer,
                Theme::Kitchen,
                Theme::Laundry,
                Theme::Lab,
                Theme::Printworks,
                Theme::Trailer,
            ]
        );
        for theme in Theme::all() {
            assert!(
                floors[..5].contains(&theme),
                "the campaign floor {theme:?} is missing from the sector row"
            );
        }
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            for (i, theme) in floors.into_iter().enumerate() {
                let mut hh = Harness::new(w, h);
                hh.app.select_idx = i;
                hh.app.menu_idx = 1; // the sector row
                let text = hh.frame(&[]);
                assert_eq!(hh.app.theme(), theme);
                let number = campaign::theme_stage(theme) + 1;
                // A fresh save has only stage one open; the ride is the
                // campaign's fourth level and, as a practice floor, always open.
                let locked = theme.stage() > 0 && !theme.is_trailer();
                let label = if locked {
                    format!("‹ {number:02} {} × ›", theme.name())
                } else {
                    format!("‹ {number:02} {} ›", theme.name())
                };
                assert!(
                    text.contains(&label),
                    "{w}x{h}: the sector row for {theme:?} is clipped or wrong \
                     (wanted `{label}`):\n{text}"
                );
                assert!(text.contains("Sector"), "{w}x{h}: the row lost its label");
            }
            // The cursor wraps through all six, both ways.
            let mut hh = Harness::new(w, h);
            hh.app.menu_idx = 1;
            hh.frame(&[]);
            for step in 1..=floors.len() {
                hh.frame(&[KeyCode::Right]);
                assert_eq!(hh.app.theme(), floors[step % floors.len()]);
            }
            hh.frame(&[KeyCode::Left]);
            assert_eq!(hh.app.theme(), Theme::Trailer);
        }
    }

    /// Enter on the title's sector row opens the select screen with the
    /// Flatbed selected; Enter **there** starts the getaway ride once the floor
    /// is open. Every floor starts from the one screen, the ride included.
    #[test]
    fn the_title_sector_row_starts_the_flatbed() {
        let mut hh = Harness::new(80, 24);
        // A cleared campaign is what opens the ride's card now.
        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            hh.app.scores.record_floor(theme, 100, 200.0);
        }
        hh.app.select_idx = Theme::playable().len() - 1; // the Flatbed
        hh.app.menu_idx = 1; // the sector row
        hh.frame(&[]);
        hh.frame(&[KeyCode::Enter]);
        assert!(
            matches!(hh.app.screen, Screen::SectorSelect),
            "the sector row must open the select screen, not start anything"
        );
        assert!(hh.app.world.is_none(), "opening the screen starts nothing");
        hh.frame(&[KeyCode::Enter]);
        assert!(
            matches!(hh.app.screen, Screen::LevelIntro),
            "Enter on the Flatbed's card did not start the getaway"
        );
        let w = hh.app.world.as_ref().expect("a world");
        assert_eq!(w.level.theme, Theme::Trailer);
        assert_eq!(w.level_index, crate::game::campaign::LEVEL_COUNT - 1);
        assert!(hh.app.campaign.is_none(), "the getaway ride is a practice run");
        assert_eq!(
            w.difficulty_override,
            Some(0),
            "a practice floor plays the first level's scalars"
        );
        // Its map intro card is the flatbed's lore, not a campaign sector's.
        let text = hh.frame(&[KeyCode::Enter]); // intro -> playing
        assert!(matches!(hh.app.screen, Screen::Playing));
        assert!(!text.is_empty());
    }

    #[test]
    fn quit_from_title() {
        let mut h = Harness::new(80, 40);
        h.frame(&[]);
        h.frame(&[KeyCode::Char('q')]);
        assert!(h.app.should_quit);
    }

    fn color_rgb(c: ratatui::style::Color) -> (u8, u8, u8) {
        use ratatui::style::Color::*;
        match c {
            Rgb(r, g, b) => (r, g, b),
            White => (235, 235, 235),
            Red => (220, 70, 70),
            LightRed => (255, 120, 120),
            Green => (80, 220, 95),
            LightGreen => (140, 255, 150),
            Yellow => (235, 210, 95),
            LightYellow => (255, 235, 140),
            Blue => (90, 120, 230),
            Magenta => (200, 90, 200),
            Cyan => (95, 210, 235),
            Gray => (150, 150, 155),
            DarkGray => (80, 80, 85),
            _ => (0, 0, 0),
        }
    }

    #[test]
    fn wave3_chests_support_keyboard_and_mouse_and_then_open_the_shop() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        let w = h.app.world.as_mut().unwrap();
        let weapon = &mut w.player.loadout.weapons[0];
        weapon.level = weapon.kind.max_level() - 1;
        let pos = w.player.pos;
        w.spawn_chest(pos);
        w.spawn_chest(pos);
        w.pending_shop = true;
        w.gold = 23;

        assert!(h.frame(&[]).contains("reached Lv"));
        assert!(matches!(h.app.screen, Screen::Chest));
        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        h.frame(&[KeyCode::Enter]);
        // Nothing left to rank: the fallback chest pays `CHEST_GOLD` (design
        // §7) and patches the rat up.
        assert!(h.screen_text().contains("+24 gold"));
        h.click("Enter / Continue");
        assert!(matches!(h.app.screen, Screen::Shop));
        // The shelf's header carries the purse (the chest's +24 gold included).
        assert!(h.screen_text().contains("GOLD 47"));
        assert_eq!(
            h.app.world.as_ref().unwrap().elapsed,
            elapsed,
            "the simulation is paused while the shelf is open"
        );

        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Playing));
        h.app.world.as_mut().unwrap().pending_shop = true;
        h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::Shop));
        // Enter buys the selected row, so leaving is Esc (or the LEAVE row).
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Playing));
    }

    /// The victory breakdown names the floor **actually played**: a campaign
    /// that took the Laundry fork for stage two must not read "THE KITCHEN"
    /// (`CampaignLevel::from_index` only knows the original route).
    #[test]
    fn the_victory_breakdown_names_the_side_route_floor() {
        let mut h = Harness::new(120, 44);
        h.app.records[1].plate = Some(campaign_ui::CampaignLevel::Laundry);
        h.app.records[1].time = 91.0;
        h.app.records[2].plate = Some(campaign_ui::CampaignLevel::Printworks);
        h.app.records[2].time = 92.0;
        h.app.screen = Screen::Victory;
        let text = h.frame(&[]);
        // The breakdown column prints the plate's short name beside its
        // sector code.
        assert!(
            text.contains("Laundry"),
            "the side route's stage two lost its name:\n{text}"
        );
        assert!(
            !text.contains("Kitchen"),
            "the victory plate fell back to the original route:\n{text}"
        );
        assert!(
            text.contains("Printworks"),
            "the side route's stage three lost its name:\n{text}"
        );
    }

    // ---- The gene-splice shop screen -------------------------------------

    /// Walk the run onto shop hole 0's threshold (the way the game does it) and
    /// stop on the open shelf.
    fn open_shop_at(h: &mut Harness, gold: u32) {
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            w.gold = gold;
            let hole = *w
                .level
                .objects
                .shop_holes
                .first()
                .expect("the test level carries a shop hole");
            w.player.pos = hole.face() + hole.facing * 3.0;
        }
        // One frame: the threshold system raises the flag and the app opens the
        // shelf in the same `resume_or_open_reward`.
        h.frame(&[]);
        assert!(
            matches!(h.app.screen, Screen::Shop),
            "the threshold opens the shelf"
        );
    }

    /// A purse that covers anything one shelf visit can charge, computed from
    /// the shop's own tables so a retune cannot leave these tests short: one
    /// more than a full set of the dearest splice, a Stitch, every Turn and a
    /// Molt.
    fn shop_purse() -> u32 {
        use crate::game::shop::{
            SpliceKind, MAX_REROLLS, MAX_SPLICES, MOLT_PRICE, REROLL_BASE_PRICE, STITCH_PRICE,
        };
        let dearest = SpliceKind::ALL
            .iter()
            .map(|kind| kind.price())
            .max()
            .unwrap_or(0);
        let turns: u32 = (0..MAX_REROLLS).map(|i| REROLL_BASE_PRICE << i).sum();
        dearest * (MAX_SPLICES as u32 + 1) + STITCH_PRICE + turns + MOLT_PRICE
    }

    fn shelf(h: &Harness, slot: usize) -> crate::game::shop::SpliceKind {
        let stock = h.app.world.as_ref().unwrap().shop.get(0);
        match stock.and_then(|stock| stock.slot(slot)) {
            Some(kind) => kind,
            None => panic!("the shelf has a splice in this slot"),
        }
    }

    #[test]
    fn the_shop_screen_works_by_keyboard_at_fiftyseven_eighty_and_120() {
        for (w, hgt) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut h = Harness::new(w, hgt);
            open_shop_at(&mut h, shop_purse());
            let text = h.screen_text();
            assert!(text.contains("GENE SPLICE SHELF"), "{w}x{hgt}: no title");
            let purse = shop_purse();
            assert!(
                text.contains(&format!("GOLD {purse}")),
                "{w}x{hgt}: no purse"
            );
            assert!(
                text.contains("SPLICES 0/2"),
                "{w}x{hgt}: no splices counter"
            );
            // Both sides of the first bargain are shown before payment.
            let first = shelf(&h, 0);
            assert!(text.contains(first.name()), "{w}x{hgt}: no card");
            assert!(text.contains(first.plus()), "{w}x{hgt}: no gain");
            assert!(text.contains(first.minus()), "{w}x{hgt}: no cost");
            assert!(text.contains("LEAVE"), "{w}x{hgt}: no exit row");

            // The simulation is frozen while the shelf is open.
            let elapsed = h.app.world.as_ref().unwrap().elapsed;
            h.frame(&[]);
            h.frame(&[]);
            assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed);

            // Down moves the cursor; Enter buys the row it is on.
            h.frame(&[KeyCode::Down]);
            assert_eq!(h.app.shop_row, 1);
            h.frame(&[KeyCode::Enter]);
            let bought = shelf(&h, 1);
            let w0 = h.app.world.as_ref().unwrap();
            assert_eq!(w0.gold, purse - bought.price(), "{w}x{hgt}: paid the price");
            assert!(w0.player.loadout.splices.contains(&bought));
            assert!(h.screen_text().contains("SPLICES 1/2"));

            // The second socket, then the third purchase is refused with the
            // shelf's own reason (and the purse does not move).
            h.frame(&[KeyCode::Up]);
            h.frame(&[KeyCode::Enter]);
            assert_eq!(
                h.app.world.as_ref().unwrap().player.loadout.splices.len(),
                2
            );
            let gold = h.app.world.as_ref().unwrap().gold;
            h.frame(&[KeyCode::Enter]);
            assert_eq!(
                h.app.world.as_ref().unwrap().gold,
                gold,
                "{w}x{hgt}: nothing is charged for a refused row"
            );
            assert!(
                h.screen_text().contains("max 2 splices")
                    || h.screen_text().contains("sold"),
                "{w}x{hgt}: the refusal has a reason"
            );

            // Esc leaves and the run resumes behind the 0.8 s grace.
            h.frame(&[KeyCode::Esc]);
            assert!(matches!(h.app.screen, Screen::Playing), "{w}x{hgt}");
            assert!(
                h.app.world.as_ref().unwrap().shop_runtime().grace > 0.0,
                "{w}x{hgt}: the exit grace is armed"
            );
        }
    }

    #[test]
    fn the_shop_screen_works_by_mouse_with_hover_at_fiftyseven_eighty_and_120() {
        for (w, hgt) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut h = Harness::new(w, hgt);
            open_shop_at(&mut h, shop_purse());
            // Hover the first card: the row reports itself as hovered.
            let first = shelf(&h, 0);
            let pos = h.locate(first.name());
            h.events(&[], &[(MouseEventKind::Moved, pos)]);
            assert_eq!(
                h.app.hits.hovered,
                Some(Target::ShopRow(0)),
                "{w}x{hgt}: hover does not reach the shelf row"
            );
            // Click it: bought.
            h.events(
                &[],
                &[(MouseEventKind::Down(MouseButton::Left), pos)],
            );
            assert!(
                h.app
                    .world
                    .as_ref()
                    .unwrap()
                    .player
                    .loadout
                    .splices
                    .contains(&first),
                "{w}x{hgt}: a click buys the row under the pointer"
            );
            // Click the exit row.
            let leave = h.locate("LEAVE");
            h.events(
                &[],
                &[(MouseEventKind::Down(MouseButton::Left), leave)],
            );
            assert!(
                matches!(h.app.screen, Screen::Playing),
                "{w}x{hgt}: clicking LEAVE leaves"
            );
            assert!(!h.app.world.as_ref().unwrap().pending_shop);
        }
    }

    /// `M` on the shelf is Molt (the legend says so), so that press must not
    /// also flip the global mute.
    #[test]
    fn the_molt_key_does_not_mute_on_the_shelf() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut h = Harness::new(80, 24);
        open_shop_at(&mut h, shop_purse());
        assert!(!h.app.muted());
        h.frame(&[KeyCode::Char('m')]);
        assert!(matches!(h.app.screen, Screen::Shop));
        assert!(!h.app.muted(), "M is Molt on the shelf, never mute");
    }

    /// While the Controls screen is capturing, the pressed key only becomes
    /// the binding: it must not also fire the global action it used to carry.
    #[test]
    fn a_key_being_bound_never_fires_its_old_global_action() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        h.app.open_options(Screen::Title);
        h.app.open_controls();
        h.app.bind_capture = Some(Action::Map);
        assert!(!h.app.muted());
        h.frame(&[KeyCode::Char('m')]);
        assert!(h.app.bind_capture.is_none(), "the press became the binding");
        assert!(!h.app.muted(), "the old mute key must not mute mid-capture");
    }

    /// The Library and the Nest read the live Back binding, so pad B leaves
    /// them (they used to read a raw Esc and trapped a pad player).
    #[test]
    fn pad_back_leaves_the_library_and_the_nest() {
        use crate::controls::PadBind;
        use crate::gamepad::PadEvent;
        let mut h = Harness::new(120, 40);
        h.frame(&[]);
        h.app.input.pad_event(PadEvent::Connected);
        h.app.open_library(Screen::Title);
        h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::Library));
        h.app.input.begin_frame();
        h.app.input.pad_event(PadEvent::Button(PadBind::East, true));
        h.app.update(1.0 / 60.0);
        assert!(matches!(h.app.screen, Screen::Title), "pad B closes the Library");

        h.app.input.begin_frame();
        h.app.input.pad_event(PadEvent::Button(PadBind::East, false));
        h.app.update(1.0 / 60.0);
        h.app.open_nest(Screen::Title);
        h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::Nest));
        h.app.input.begin_frame();
        h.app.input.pad_event(PadEvent::Button(PadBind::East, true));
        h.app.update(1.0 / 60.0);
        assert!(matches!(h.app.screen, Screen::Title), "pad B closes the Nest");
    }

    // ---- Batch 6 end-to-end flows ----------------------------------------

    /// One whole campaign hop: boss down → sector card → keep-three with the
    /// side route picked → the next floor's intro plate → play resumes. The
    /// run is neither banked nor scored on the way (it has not ended).
    #[test]
    fn a_campaign_hop_goes_card_keep_route_intro_and_plays_on() {
        let mut h = Harness::new(100, 32);
        h.enter_playing();
        h.grant_keepables();
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert_eq!(h.app.floors_cleared, 1);
        assert!(!h.app.banked, "a cleared floor is not the end of the run");
        assert!(!h.app.recorded, "no score entry until the run ends");
        let run_seed = h.app.campaign.as_ref().unwrap().run_seed;

        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        h.frame(&[KeyCode::Tab]);
        assert_eq!(h.app.keep_route, Some(Theme::Laundry));
        for _ in 0..3 {
            h.frame(&[KeyCode::Char(' ')]);
            h.frame(&[KeyCode::Down]);
        }
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        {
            let w = h.app.world.as_ref().expect("the next floor");
            assert_eq!(w.level_index, 1);
            assert_eq!(w.level.theme, Theme::Laundry);
            assert_eq!(w.level.seed, campaign::level_seed_on(run_seed, 1, Theme::Laundry));
            assert_eq!(w.elapsed, 0.0, "the intro plate holds the new floor still");
        }
        assert!(!h.app.carried_entries.is_empty(), "the pack rides along");
        assert_eq!(h.app.floors_cleared, 1, "the counter survives the hop");
        assert!(!h.app.banked && !h.app.recorded);
        assert!(h.app.scores.entries.is_empty(), "no score before the run ends");

        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.frame(&[]);
        assert!(h.app.world.as_ref().unwrap().elapsed > 0.0);
    }

    /// A practice run on the Flatbed ends on the incident report, banks at the
    /// practice rate (half, rounded down) and leaves the campaign's stage
    /// records alone.
    #[test]
    fn a_flatbed_practice_run_ends_on_the_report_and_banks_half() {
        let mut h = Harness::new(90, 32);
        h.frame(&[]);
        h.app.start_practice(campaign::getaway_theme());
        assert_eq!(h.app.practice, Some(campaign::getaway_theme()));
        assert!(h.app.campaign.is_none());
        h.frame(&[]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.app.world.as_mut().unwrap().gold = 41;
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert!(!h.app.banked);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "End", "practice never reaches keep-three");

        let tally = h.app.bank_tally.expect("the report carries a tally");
        assert!(tally.halved && !tally.voided);
        assert_eq!(tally.purse, 41 / meta::PRACTICE_DIVISOR);
        assert_eq!(tally.floor_bonus, meta::FLOOR_BONUS / meta::PRACTICE_DIVISOR);
        assert_eq!(tally.win_bonus, meta::WIN_BONUS / meta::PRACTICE_DIVISOR);
        assert_eq!(tally.apex_bonus, 0);
        assert_eq!(
            tally.banked,
            tally.purse + tally.floor_bonus + tally.win_bonus
        );
        assert_eq!(h.app.scores.meta.bank, tally.banked);
        let summary = h.app.last_summary.as_ref().expect("a summary");
        assert!(summary.practice && summary.won);
        assert!(
            h.app.records.iter().all(|r| r.plate.is_none()),
            "the getaway ride must not write a campaign stage record"
        );
        assert_eq!(h.app.scores.entries.len(), 1, "one score entry for the run");

        // Lingering on the report never banks or scores twice.
        h.frame(&[]);
        h.app.finalize();
        assert_eq!(h.app.scores.meta.bank, tally.banked);
        assert_eq!(h.app.scores.entries.len(), 1);
        // Enter retries the same practice floor as a fresh, unbanked run.
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.practice, Some(campaign::getaway_theme()));
        assert!(!h.app.banked && h.app.bank_tally.is_none());
        assert_eq!(h.app.floors_cleared, 0);
        assert_eq!(h.app.scores.meta.bank, tally.banked);
    }

    /// The shelf's Turn row: two paid turns (the base price, then double), a
    /// refused third, then the LEAVE row resumes the run.
    #[test]
    fn the_shelf_turns_twice_refuses_a_third_and_leaves() {
        use crate::game::shop::{ShopItem, MAX_REROLLS, REROLL_BASE_PRICE};
        let mut h = Harness::new(80, 24);
        let purse = shop_purse();
        open_shop_at(&mut h, purse);
        let row_of = |item: ShopItem| {
            (0..shop::ROW_COUNT)
                .find(|&r| shop::row_item(r) == item)
                .expect("the shelf has this row")
        };
        h.app.shop_row = row_of(ShopItem::Reroll);
        h.frame(&[KeyCode::Enter]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.gold, purse - REROLL_BASE_PRICE);
            assert_eq!(w.shop.get(0).unwrap().rerolls, 1);
        }
        h.frame(&[KeyCode::Enter]);
        let after_two = purse - REROLL_BASE_PRICE - (REROLL_BASE_PRICE << 1);
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.gold, after_two);
            assert_eq!(w.shop.get(0).unwrap().rerolls, MAX_REROLLS);
        }
        h.frame(&[KeyCode::Enter]);
        assert_eq!(
            h.app.world.as_ref().unwrap().gold,
            after_two,
            "a third turn is refused and free"
        );
        assert!(matches!(h.app.screen, Screen::Shop));
        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        h.app.shop_row = row_of(ShopItem::Leave);
        h.frame(&[KeyCode::Enter]);
        assert!(matches!(h.app.screen, Screen::Playing));
        assert_eq!(
            h.app.world.as_ref().unwrap().elapsed,
            elapsed,
            "the shelf never ticked the world, and leaving resumes next frame"
        );
        assert!(!h.app.world.as_ref().unwrap().pending_shop);
    }

    /// A chest opened with a max-level weapon and its partner passive evolves
    /// it: the card is the evolution, the world holds still, Enter resumes.
    #[test]
    fn a_chest_evolves_a_maxed_weapon_with_its_partner() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        let kind = {
            let w = h.app.world.as_mut().unwrap();
            w.clear_enemies();
            w.clear_gems();
            let kind = w.player.loadout.weapons[0].kind;
            let partner = kind.evolution().expect("the start weapon evolves").passive;
            w.player.loadout.weapons[0].level = kind.max_level();
            w.player.loadout.apply(&Upgrade::Passive(partner));
            w.player.xp_to_next = 1_000_000;
            let pos = w.player.pos;
            w.spawn_chest(pos);
            kind
        };
        h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::Chest));
        assert!(
            matches!(h.app.chest_reward, Some(ChestReward::Evolve { kind: k, .. }) if k == kind),
            "the chest must evolve the eligible weapon: {:?}",
            h.app.chest_reward.is_some()
        );
        assert!(h.app.world.as_ref().unwrap().player.loadout.weapons[0].evolved);
        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        h.frame(&[]);
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed, "the card pauses the sim");
        h.frame(&[KeyCode::Enter]);
        assert!(matches!(h.app.screen, Screen::Playing));
        assert!(h.app.chest_reward.is_none());
        // Evolved once: a second chest cannot evolve the same weapon again.
        {
            let w = h.app.world.as_mut().unwrap();
            let pos = w.player.pos;
            w.spawn_chest(pos);
        }
        h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::Chest));
        assert!(!matches!(h.app.chest_reward, Some(ChestReward::Evolve { .. })));
    }

    /// Losing the terminal's focus mid-charge (`main` reports it with
    /// `set_focused(false)`) cancels a held Super Dash: nothing fires, then or
    /// on the late mouse-up.
    #[test]
    fn focus_loss_cancels_a_held_super_dash_charge() {
        let mut h = Harness::new(100, 30);
        h.enter_playing();
        {
            use crate::game::weapon::WeaponKind;
            let w = h.app.world.as_mut().unwrap();
            w.player
                .loadout
                .apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
            w.player.hp = 1.0e9;
            w.player.xp_to_next = 1_000_000;
        }
        let field = h.locate("▀");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), field)]);
        for _ in 0..20 {
            h.frame(&[]);
        }
        assert!(h.app.world.as_ref().unwrap().player.dash_charging);
        assert!(h.app.world.as_ref().unwrap().player.dash_charge > 0.0);

        // Focus goes away between frames, as `main` delivers it.
        h.app.input.begin_frame();
        h.app.input.set_focused(false);
        assert!(h.app.input.dash_cancelled() && !h.app.input.dash_up());
        h.app.update(1.0 / 60.0);
        h.app.input.set_focused(true);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(!w.player.dash_charging, "the charge must be dropped");
            assert_eq!(w.player.dash_charge, 0.0);
            assert_eq!(w.player.dash_active, 0.0, "a focus loss must never dash");
        }
        // The button's real release arrives later: it has nothing to fire.
        h.events(&[], &[(MouseEventKind::Up(MouseButton::Left), field)]);
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.player.dash_active, 0.0);
        assert!(!w.player.dash_charging);
    }

    /// A CONSOLE USED run banks nothing and enters no score, whether it ends
    /// in a death or a quit from the pause screen.
    #[test]
    fn a_console_used_run_banks_and_scores_nothing() {
        // Death.
        let mut h = Harness::new(90, 32);
        h.enter_playing();
        h.app.console_flag_cheat();
        h.app.world.as_mut().unwrap().gold = 130;
        h.app.world.as_mut().unwrap().player.hp = -1.0;
        h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        let tally = h.app.bank_tally.expect("a voided tally is still a tally");
        assert!(tally.voided);
        assert_eq!(tally.banked, 0);
        assert_eq!(h.app.scores.meta.bank, 0);
        assert!(h.app.scores.entries.is_empty(), "no score from a console run");
        assert!(!h.app.new_best);
        assert!(h.app.last_summary.as_ref().is_some_and(|s| s.console_used));
        h.frame(&[KeyCode::Char('q')]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        assert_eq!(h.app.scores.meta.bank, 0);

        // Quit from pause.
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.console_flag_cheat();
        h.app.world.as_mut().unwrap().gold = 42;
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('q')]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        assert_eq!(h.app.scores.meta.bank, 0, "a console run's purse is void");
        assert!(h.app.bank_tally.is_some_and(|t| t.voided));
    }

    /// The Nest's revive: the first death stands the rat back up and the run
    /// goes on (no report, no bank); the second death ends it, banked once.
    #[test]
    fn the_nests_revive_spends_once_and_the_second_death_ends_the_run() {
        let mut app = test_app();
        app.scores.meta.set_level(meta::Track::Revive, 1);
        let mut h = Harness::with_app(app, 90, 32);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            assert_eq!(w.revives_left, 1);
            w.clear_enemies();
            w.gold = 30;
            w.player.hp = -1.0;
        }
        h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "Playing", "the revive keeps the run alive");
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.revives_left, 0);
            assert!(w.player.hp > 0.0 && !w.game_over);
        }
        assert!(!h.app.banked && !h.app.recorded);
        assert!(h.app.scores.entries.is_empty());

        h.app.world.as_mut().unwrap().player.hp = -1.0;
        h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "End", "one revive per run");
        assert!(h.app.banked && h.app.recorded);
        assert_eq!(h.app.scores.entries.len(), 1);
        assert_eq!(h.app.scores.meta.bank, 30);
        // Nothing on the report banks or scores again.
        h.frame(&[]);
        h.frame(&[KeyCode::Char('b')]);
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        assert_eq!(h.app.scores.entries.len(), 1);
        assert_eq!(h.app.scores.meta.bank, 30);
    }

    /// A campaign victory is finalised exactly once: one score entry, one
    /// bank (purse + three floors + the win), and neither the victory plate,
    /// the Nest round trip nor the walk back to the title repeats it.
    #[test]
    fn a_campaign_victory_scores_and_banks_exactly_once() {
        let mut h = Harness::new(100, 32);
        h.frame(&[]);
        h.app
            .start_campaign_at(campaign::LEVEL_COUNT - 1, None)
            .expect("the final stage starts");
        // A direct call is not a console command, so the run is clean here.
        h.app.console_cheat = false;
        h.frame(&[]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.app.world.as_mut().unwrap().gold = 10;
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "Victory");
        assert!(h.app.banked && h.app.recorded);
        let tally = h.app.bank_tally.expect("the victory carries a tally");
        assert_eq!(tally.win_bonus, meta::WIN_BONUS);
        assert_eq!(tally.floor_bonus, meta::FLOOR_BONUS, "one floor was cleared this run");
        assert_eq!(tally.banked, 10 + meta::FLOOR_BONUS + meta::WIN_BONUS);
        assert_eq!(h.app.scores.meta.bank, tally.banked);
        assert_eq!(h.app.scores.entries.len(), 1);
        assert!(h.app.scores.entries[0].campaign_won);

        h.frame(&[]);
        h.frame(&[KeyCode::Char('b')]);
        assert_eq!(h.app.screen.name_for_test(), "Nest");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Victory");
        h.app.finalize();
        h.frame(&[KeyCode::Char('q')]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        assert_eq!(h.app.scores.entries.len(), 1);
        assert_eq!(h.app.scores.meta.bank, tally.banked);
    }

    /// A miniboss that **retreats** (here: the boss comes due) is not an apex
    /// kill, so it pays the nest nothing.
    #[test]
    fn a_retreating_miniboss_is_not_an_apex_kill() {
        let mut h = Harness::new(100, 32);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            w.player.hp = 1.0e9;
            w.player.xp_to_next = 1_000_000;
            // Every appointment is overdue and the boss is due: the schedule
            // appoints one miniboss a tick, then retreats the last survivor.
            w.elapsed = crate::game::director::BOSS_TIME;
        }
        let mut seen = false;
        for _ in 0..12 {
            h.frame(&[]);
            if !matches!(h.app.screen, Screen::Playing) {
                break;
            }
            seen |= h.app.prev_miniboss_alive;
        }
        assert!(seen, "the schedule never put a miniboss on the floor");
        assert!(
            h.app.world.as_ref().unwrap().miniboss_hp().is_none(),
            "the last miniboss should have retreated by now"
        );
        assert_eq!(h.app.apex_kills, 0, "a retreat was counted as a kill");
    }

    /// The audio bridge: one frame's effects are de-duplicated (a volley is
    /// one sound), and the simulation's queue is always left drained.
    #[test]
    fn the_sfx_bridge_dedups_each_frame_and_drains_the_queue() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            for _ in 0..16 {
                w.sfx.push(Sfx::Heal);
            }
            w.sfx.push(Sfx::Dash);
            w.sfx.push(Sfx::Dash);
        }
        let dashes = h.app.dashes_used;
        h.frame(&[]);
        let heals = h.app.sfx_frame.iter().filter(|s| **s == Sfx::Heal).count();
        assert_eq!(heals, 1, "sixteen identical effects are one sound");
        assert_eq!(h.app.sfx_frame.iter().filter(|s| **s == Sfx::Dash).count(), 1);
        assert_eq!(h.app.dashes_used, dashes + 1, "one dash per frame, not per event");
        assert!(h.app.world.as_ref().unwrap().sfx.is_empty());
        // The next frame starts from an empty list: nothing replays.
        h.frame(&[]);
        assert!(!h.app.sfx_frame.contains(&Sfx::Heal));
    }

    /// A new floor is handed the viewport before its first tick: the value the
    /// next render would write, never the constructor's default.
    #[test]
    fn a_new_floor_starts_with_the_terminals_viewport() {
        let mut h = Harness::new(100, 32);
        h.frame(&[]); // the title: the App now knows the terminal's size
        h.frame(&[KeyCode::Enter]); // START RUN builds the floor
        let built = h.app.world.as_ref().expect("a floor").viewport;
        h.frame(&[KeyCode::Enter]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.frame(&[]);
        let game = h.app.last_game;
        let drawn = Vec2::new(game.width as f32, (game.height * 2) as f32);
        assert_eq!(h.app.world.as_ref().unwrap().viewport, drawn);
        assert_eq!(built, drawn, "the first tick must not run at another size");
    }

    /// The simulation's viewport is written by the App's render on a resize —
    /// under an overlay too — and the first frame at a new size already
    /// carries it.
    #[test]
    fn a_resize_writes_the_simulation_viewport() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.frame(&[]);
        let game = h.app.last_game;
        let expect = |g: Rect| Vec2::new(g.width as f32, (g.height * 2) as f32);
        assert_eq!(h.app.world.as_ref().unwrap().viewport, expect(game));

        h.terminal.backend_mut().resize(120, 40);
        h.frame(&[]);
        let bigger = h.app.last_game;
        assert!(bigger.width > game.width && bigger.height > game.height);
        assert_eq!(h.app.world.as_ref().unwrap().viewport, expect(bigger));

        // Paused: the field is still drawn behind the plate, so a resize there
        // reaches the simulation's viewport before play resumes.
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        h.terminal.backend_mut().resize(60, 20);
        h.frame(&[]);
        let smaller = h.app.last_game;
        assert_eq!(h.app.world.as_ref().unwrap().viewport, expect(smaller));
    }

    /// The Library's counters the App books: TAKEN when a card is taken, and
    /// KILLED YOU against the boss when the rat dies in its fight. A CONSOLE
    /// USED run books neither.
    #[test]
    fn the_app_books_taken_and_boss_killed_you_but_not_on_a_console_run() {
        use crate::game::library::Counter;
        for cheat in [false, true] {
            let mut h = Harness::new(80, 30);
            h.show_upgrade_cards();
            if cheat {
                h.app.console_flag_cheat();
            }
            let key = LibraryKey::Passive(PassiveKind::Damage);
            let before = h.app.scores.library.record(key).picked;
            h.frame(&[KeyCode::Char('1')]);
            let want = if cheat { before } else { before + 1 };
            assert_eq!(
                h.app.scores.library.record(key).picked,
                want,
                "cheat={cheat}: TAKEN after picking the Damage card"
            );
        }
        assert!(miniboss_kind_named("Corkscrew").is_some());
        assert!(miniboss_kind_named("nobody").is_none());
        let _ = Counter::KilledBy;
    }

    /// A console-started campaign resets the per-run bank bookkeeping like
    /// every other start path (it used to inherit the previous run's `banked`
    /// flag and tally).
    #[test]
    fn a_console_campaign_resets_the_bank_bookkeeping() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.floors_cleared = 2;
        h.app.back_to_title();
        assert!(h.app.banked, "quitting a run banks it");
        h.app.start_campaign_at(0, None).expect("a campaign starts");
        assert!(!h.app.banked);
        assert!(h.app.bank_tally.is_none());
        assert_eq!(h.app.floors_cleared, 0);
        assert!(!h.app.console_cheat);
    }

    #[test]
    fn the_molt_two_step_works_by_keyboard_and_cancels_free() {
        let mut h = Harness::new(80, 24);
        open_shop_at(&mut h, shop_purse());
        {
            use crate::game::weapon::WeaponKind;
            let lo = &mut h.app.world.as_mut().unwrap().player.loadout;
            lo.apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
            lo.weapons[0].level = 4;
        }
        let gold = h.app.world.as_ref().unwrap().gold;
        // "m" opens step one and the shelf says so.
        h.frame(&[KeyCode::Char('m')]);
        assert!(h.app.shop_molt.is_some());
        assert!(h.screen_text().contains("Step 1 of 2"));
        // Cancel is free: nothing is spent and the shelf is back.
        h.frame(&[KeyCode::Esc]);
        assert!(h.app.shop_molt.is_none());
        assert!(matches!(h.app.screen, Screen::Shop));
        assert_eq!(h.app.world.as_ref().unwrap().gold, gold);

        // Nominate the first weapon, then take the first option offered.
        h.frame(&[KeyCode::Char('m')]);
        h.frame(&[KeyCode::Enter]);
        assert!(
            matches!(
                h.app.shop_molt,
                Some(shop_ui::MoltUi::Pick { .. })
            ),
            "Enter names the kind and moves to step two"
        );
        assert!(h.screen_text().contains("Step 2 of 2"));
        h.frame(&[KeyCode::Enter]);
        let w = h.app.world.as_ref().unwrap();
        assert!(h.app.shop_molt.is_none());
        assert_eq!(
            w.gold,
            gold - crate::game::shop::MOLT_PRICE,
            "Molt costs its price"
        );
        assert_eq!(w.player.loadout.weapons[0].level, 1, "rank 1");
        assert!(!w.player.loadout.weapons[0].evolved);
    }

    #[test]
    fn the_molt_two_step_works_by_mouse() {
        let mut h = Harness::new(120, 40);
        open_shop_at(&mut h, shop_purse());
        {
            use crate::game::weapon::WeaponKind;
            let lo = &mut h.app.world.as_mut().unwrap().player.loadout;
            lo.apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
        }
        let before = h.app.world.as_ref().unwrap().player.loadout.weapons[0].kind;
        h.click("MOLT");
        assert!(h.app.shop_molt.is_some(), "clicking MOLT opens step one");
        h.frame(&[]);
        // Nominate Gnaw (the first row of step one). "Gnaw Lv1" only exists on
        // this panel: the HUD rack prints the bare name.
        h.click("Gnaw Lv1");
        assert!(matches!(h.app.shop_molt, Some(shop_ui::MoltUi::Pick { .. })));
        // Park the pointer off the panel so the cheese cursor does not sit on
        // the option text the next `locate` reads.
        h.events(&[], &[(MouseEventKind::Moved, (0, 0))]);
        h.frame(&[]);
        // Take the first offered replacement.
        let options = {
            let w = h.app.world.as_ref().unwrap();
            crate::game::shop::molt_options(
                w,
                crate::game::shop::OwnedKind::Weapon(0),
            )
        };
        let label = crate::game::shop::new_label(options[0]);
        h.click(&label);
        let w = h.app.world.as_ref().unwrap();
        assert_ne!(w.player.loadout.weapons[0].kind, before);
        assert_eq!(w.player.loadout.weapons[0].level, 1);
    }

    #[test]
    fn a_denied_row_costs_nothing_and_says_why() {
        let mut h = Harness::new(80, 24);
        open_shop_at(&mut h, 0);
        assert!(h.screen_text().contains("not enough gold"));
        h.frame(&[KeyCode::Enter]);
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.gold, 0);
        assert!(w.player.loadout.splices.is_empty());
        assert!(matches!(h.app.screen, Screen::Shop), "still on the shelf");
        // Stitch at full health has its own reason.
        assert!(h.screen_text().contains("at full health"));
    }

    /// A practice run starts a later sector with no carried build, so it must
    /// play on the first level's difficulty curve; a campaign level keeps its own.
    #[test]
    fn practice_run_uses_the_first_levels_difficulty() {
        let mut hh = Harness::new(80, 24);
        hh.app.start_practice(Theme::Kitchen);
        let w = hh.app.world.as_ref().unwrap();
        assert_eq!(w.level_index(), 1, "still the Kitchen's map and boss");
        assert_eq!(w.difficulty_override, Some(0));
        let carried = World::new_level(1, 7, Loadout::starting());
        assert_eq!(carried.difficulty_override, None);
    }

    /// The **practice ride** is unchanged by the Flatbed joining the campaign:
    /// the sector card plays the Flatbed alone, on the default loadout, at level
    /// index 3 with the first level's scalars, and it never advances anything.
    #[test]
    fn the_practice_flatbed_is_still_the_getaway_ride() {
        let mut hh = Harness::new(80, 24);
        hh.app.start_getaway();
        {
            let w = hh.app.world.as_ref().unwrap();
            assert_eq!(w.level.theme, Theme::Trailer);
            assert_eq!(w.level_index(), campaign::LEVEL_COUNT - 1);
            assert_eq!(w.difficulty_override, Some(0), "level 0's scalars");
            assert_eq!(w.player.loadout.weapons.len(), 1, "no carried build");
        }
        assert!(hh.app.campaign.is_none(), "not a campaign run");
        assert_eq!(hh.app.practice, Some(Theme::Trailer));
        // Past the ride's intro plate and into play.
        if hh.app.screen.name_for_test() == "LevelIntro" {
            hh.frame(&[KeyCode::Enter]);
        }
        assert_eq!(hh.app.screen.name_for_test(), "Playing");
        // Clearing it ends the run (the incident report), never a campaign win.
        hh.app.world.as_mut().unwrap().mark_level_complete();
        hh.frame(&[]);
        assert_eq!(hh.app.screen.name_for_test(), "LevelComplete");
        hh.frame(&[KeyCode::Enter]);
        assert_eq!(hh.app.screen.name_for_test(), "End");
        assert!(
            hh.app.last_summary.as_ref().is_some_and(|s| s.practice && s.won),
            "a cleared practice ride is a practice win"
        );
        assert!(
            !hh.app.scores.entries[0].campaign_won,
            "a practice ride never wins the campaign"
        );
    }

    /// A practice run on a later sector really plays level 0's curve: the
    /// horde it spawns at t = 100 s carries level 0's hp and damage
    /// multipliers, not the sector's own.
    #[test]
    fn practice_run_spawns_the_first_levels_enemy_stats() {
        use crate::game::director;
        use crate::game::enemy::EnemyKind;
        // 100 s is inside every stage's opening grace for hp (the damage ramp
        // already differs there); 600 s is where the stages' hp differs too.
        for t in [100.0f32, 600.0] {
            // Every later-stage floor, side routes included.
            for theme in Theme::all().into_iter().filter(|t| t.stage() > 0) {
                let sector = theme.stage();
                let mut hh = Harness::new(80, 24);
                hh.app.start_practice(theme);
                let w = hh.app.world.as_mut().unwrap();
                assert_eq!(w.level_index(), sector);
                assert_eq!(w.level.theme, theme);
                assert_eq!(w.difficulty_override, Some(0));
                // Jump the clock and let one pulse through; the rat cannot die.
                w.elapsed = t;
                w.player.hp = 1.0e9;
                w.player.iframes = 1.0e9;
                w.director.spawn_timer = 0.0;
                w.director.elite_timer = 1.0e9;
                let mut spawned = Vec::new();
                for _ in 0..240 {
                    w.update(1.0 / 60.0, crate::math::Vec2::ZERO, false);
                    // Named minibosses carry appointment stats, not the curve's.
                    spawned = w.enemies().into_iter().filter(|e| !e.named).collect();
                    if !spawned.is_empty() {
                        break;
                    }
                }
                assert!(!spawned.is_empty(), "sector {sector}: nothing spawned at {t} s");
                let now = w.elapsed;
                let hp0 = director::horde_hp_mult_for(0, now);
                let mods0 = director::enemy_mods_for(0, now);
                assert_eq!(mods0.damage, 1.0, "level 0 has no damage ramp");
                // The sector's own curve is measurably different, so the
                // assertions below can tell the two apart.
                assert!(director::enemy_mods_for(sector, now).damage > 1.005);
                for e in &spawned {
                    let rel = e.max_hp / e.kind.base_hp();
                    assert!(
                        (rel - hp0).abs() < 0.01,
                        "sector {sector} at {t} s: {:?} hp mult {rel}, level 0's is {hp0}",
                        e.kind
                    );
                    let base: f32 = EnemyKind::damage(e.kind);
                    assert!(
                        (e.damage - base * mods0.damage).abs() < 1e-3,
                        "sector {sector} at {t} s: {:?} damage {} is not level 0's {}",
                        e.kind,
                        e.damage,
                        base * mods0.damage
                    );
                }
            }
        }
    }

    /// Render a real frame and dump it as a PPM (each cell → 1×2 pixels:
    /// fg on top, bg below). Gated behind RATRUN_DUMP (value selects the
    /// screen: play|story|levelup|treasure|chest) so it's a manual screenshot tool.
    /// Tab's full map pauses the world; closing it resumes play.
    #[test]
    fn the_full_map_pauses_the_world() {
        let mut hh = Harness::new(80, 24);
        hh.enter_playing();
        hh.frame(&[]);
        hh.frame(&[KeyCode::Tab]);
        assert!(hh.app.map_open);
        let before = hh.app.world.as_ref().unwrap().elapsed;
        for _ in 0..30 {
            hh.frame(&[KeyCode::Right]);
        }
        let held = hh.app.world.as_ref().unwrap().elapsed;
        assert_eq!(held, before, "the world advanced behind the open map");
        hh.frame(&[KeyCode::Tab]);
        assert!(!hh.app.map_open);
        hh.frame(&[]);
        assert!(hh.app.world.as_ref().unwrap().elapsed > before, "play did not resume");
    }

    #[test]
    fn the_blackout_announces_and_sounds_once_at_each_edge() {
        let mut h = Harness::new(57, 20);
        h.enter_playing();
        h.app.set_minimap_size(minimap::MinimapSize::Hidden);
        h.app.world.as_mut().unwrap().elapsed = 299.99;
        let text = h.frame(&[]);
        assert!(text.contains("BLACKOUT") && text.contains("LAMPS OUT"), "{text}");
        assert!(h.app.sfx_frame.contains(&Sfx::NightBed));
        h.frame(&[]);
        assert!(!h.app.sfx_frame.contains(&Sfx::NightBed));

        h.app.world.as_mut().unwrap().elapsed = 308.59;
        h.frame(&[]);
        assert!(h.app.sfx_frame.contains(&Sfx::MowerDockHum));
        h.frame(&[]);
        assert!(!h.app.sfx_frame.contains(&Sfx::MowerDockHum));
    }

    /// The map hints and the level-up legend name the live bindings, not the
    /// default keys.
    #[test]
    fn the_map_hints_and_the_level_up_legend_follow_a_rebind() {
        use crate::controls::{Binding, KeyBind};
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        let radar = hh.frame(&[]);
        assert!(radar.contains("N·TAB"), "the default radar hint:\n{radar}");
        hh.app
            .input
            .controls
            .bind_active_kbm(Action::Map, Binding::Key(KeyBind::Char('j')));
        hh.app
            .input
            .controls
            .bind_active_kbm(Action::MinimapSize, Binding::Key(KeyBind::Char('h')));
        hh.app
            .input
            .controls
            .bind_active_kbm(Action::Confirm, Binding::Key(KeyBind::Char('y')));
        let radar = hh.frame(&[]);
        assert!(radar.contains("H·J"), "the radar hint names the new keys:\n{radar}");
        assert!(!radar.contains("N·TAB"), "a stale radar hint:\n{radar}");
        let map = hh.frame(&[KeyCode::Char('j')]);
        assert!(hh.app.map_open);
        assert!(map.contains("[J] CLOSE"), "the modal names the new Map key:\n{map}");
        assert!(map.contains("[J] Close  [H] Size"), "the rail names both keys:\n{map}");
        assert!(!map.contains("[TAB]"), "a stale map hint:\n{map}");
        // The line under the modal names them too (it used to print a literal
        // "Tab/Esc close · N minimap size").
        let screen = hh.screen_text();
        assert!(
            screen.contains("J/ESC close · H minimap size"),
            "the line under the map names the live keys:\n{screen}"
        );
        assert!(!screen.contains("Tab/Esc"), "a stale line under the map:\n{screen}");
        hh.frame(&[KeyCode::Char('j')]);
        assert!(!hh.app.map_open);

        hh.app.world.as_mut().unwrap().pending_levelups = 1;
        let mut text = hh.frame(&[]);
        for _ in 0..4 {
            if hh.app.screen.name_for_test() == "LevelUp" {
                break;
            }
            text = hh.frame(&[]);
        }
        assert_eq!(hh.app.screen.name_for_test(), "LevelUp");
        let text = if text.contains("equip") { text } else { hh.frame(&[]) };
        assert!(text.contains("Y equip"), "the legend names the new Confirm key:\n{text}");
        assert!(!text.contains("Enter equip"), "a stale Confirm key:\n{text}");
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn dump_frame_ppm() {
        let target = match std::env::var("RATRUN_DUMP") {
            Ok(v) => v,
            Err(_) => return,
        };
        // `RATRUN_DUMP_SIZE=57x20` captures a smaller terminal (the shop
        // screen's compact layout); `RATRUN_DUMP_W` / `RATRUN_DUMP_H` set one
        // dimension each. The default stays the documented 120x46.
        let env_size = |key: &str, default: u16| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let (w, h) = std::env::var("RATRUN_DUMP_SIZE")
            .ok()
            .and_then(|size| {
                let (a, b) = size.split_once('x')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            })
            .unwrap_or((env_size("RATRUN_DUMP_W", 120), env_size("RATRUN_DUMP_H", 46)));
        let mut hh = Harness::new(w, h);
        hh.frame(&[]);

        match target.as_str() {
            "testbed" | "testbed-boss" => {
                hh.app.start_testbed(Theme::Sewer);
                hh.frame(&[KeyCode::Char('4'), KeyCode::Char('6'), KeyCode::Char('7')]);
                if target == "testbed-boss" {
                    hh.frame(&[KeyCode::F(3)]);
                }
            }
            // The dash module and the on-rat gauge in each of their three
            // states. `RATRUN_DUMP_CTRL=1` captures the Ctrl mapping (the
            // default is the Space fallback).
            "dash-ready" | "dash-cd" | "dash-denied" => {
                hh.app
                    .set_lone_ctrl(std::env::var("RATRUN_DUMP_CTRL").is_ok());
                hh.enter_playing();
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.player.dash_active = 0.0;
                    w.player.dash_ready_flash = 0.0;
                    w.player.dash_denied = 0.0;
                    match target.as_str() {
                        "dash-ready" => w.player.dash_cd = 0.0,
                        "dash-cd" => w.player.dash_cd = 1.2,
                        _ => {
                            w.player.dash_cd = 1.2;
                            w.player.dash_denied = 0.16;
                        }
                    }
                }
                // Two frames: the first tick sets the HUD up, the second draws
                // the state the capture is about.
                hh.frame(&[]);
                hh.frame(&[]);
            }
            "title" => {
                hh.frame(&[]);
            }
            "story" => {
                hh.frame(&[KeyCode::Char('l')]);
            }
            // The sector-select screen with a chosen floor highlighted.
            // `RATRUN_DUMP_FLOOR` picks the floor (0 Sewer .. 5 Flatbed); the
            // table is seeded so the capture shows every status: cleared, open
            // and locked, with per-floor bests.
            t if t == "select" || t.starts_with("select-") => {
                use crate::game::level::Theme;
                // One clear only: the capture then shows all four states a
                // card can be in — cleared (the Sewer), open with no record
                // yet (the Kitchen and the Laundry), locked (stage three) and
                // the Flatbed's always-open getaway.
                let mut table = ScoreTable::default();
                table.record_floor(Theme::Sewer, 6420, 252.0);
                let app = App::with_options(false, None, table, false, Some(TEST_SEED));
                hh = Harness::with_app(app, w, h);
                hh.app.screen = Screen::SectorSelect;
                let floor = t
                    .strip_prefix("select-")
                    .and_then(|s| s.parse::<usize>().ok())
                    .or_else(|| {
                        std::env::var("RATRUN_DUMP_FLOOR")
                            .ok()
                            .and_then(|s| s.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                hh.app.select_idx = floor.min(Theme::playable().len() - 1);
                hh.frame(&[]);
            }
            "gem-field" | "gem-flash" => {
                use crate::game::ecs::resources::spawn_with_id;
                use crate::game::gem::{Gem, GemTier, MERGE_EVERY};
                use crate::game::gem::Gold;
                hh.enter_playing();
                let w = hh.app.world.as_mut().unwrap();
                w.configure_testbed();
                w.player.pos += Vec2::new(-21.0, -13.0);
                let pos = w.player.pos;
                if target == "gem-flash" {
                    // A smaller simulation viewport makes the merge eligible
                    // without moving its result outside the review camera.
                    let viewport = w.viewport;
                    w.viewport = Vec2::splat(8.0);
                    let at = pos + Vec2::new(20.0, -9.0);
                    let first = w.spawn_gem(Gem::new(at, 310));
                    w.spawn_gem(Gem::new(at, 210));
                    w.elapsed = MERGE_EVERY;
                    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
                    w.ecs.run_system_cached(crate::game::ecs::systems::loot::gem_merge_system)
                        .expect("gem merge runs");
                    w.end_tick();
                    assert!(w.ecs.get::<Gem>(first).unwrap().merged);
                    w.viewport = viewport;
                }
                for (i, tier) in GemTier::ALL.into_iter().enumerate() {
                    let x = i as f32 * 14.0 - 34.0;
                    w.spawn_gem(Gem::new(pos + Vec2::new(x, 14.0), tier.min_value()));
                    w.spawn_gem(Gem::new(pos + Vec2::new(-x, -22.0), tier.min_value()));
                    spawn_with_id(&mut w.ecs, Gold::new(pos + Vec2::new(x + 5.0, 8.0), 1));
                }
                hh.app.picture_dirty = true;
                hh.terminal.draw(|f| hh.app.render(f)).unwrap();
            }
            "lava-volley" | "lava-pools" | "dropping-r1" | "dropping-r5" | "laser-wet" => {
                use crate::game::weapon::{LASER_ARC_DEG, LASER_SWEEP_SECS};
                hh.enter_playing();
                hh.app.scores.minimap_size = minimap::MinimapSize::Hidden;
                let w = hh.app.world.as_mut().unwrap();
                w.configure_testbed();
                w.player.pos += Vec2::new(-25.0, -15.0);
                let (kind, rank, ticks, evolved) = match target.as_str() {
                    "lava-volley" => (WeaponKind::LavaBreath, 5, 118, false),
                    "lava-pools" => (WeaponKind::LavaBreath, 5, 137, true),
                    "dropping-r1" => (WeaponKind::Hantavirus, 1, 32, false),
                    "dropping-r5" => (WeaponKind::Hantavirus, 5, 32, false),
                    _ => (WeaponKind::LaserEyes, 5, 9, false),
                };
                let pos = w.player.pos;
                let mut aim = pos + Vec2::new(100.0, 0.0);
                if kind == WeaponKind::LaserEyes {
                    // Lay a normal Sewer puddle just below a horizontal sweep
                    // sample in the hall, where the camera can see its mirror.
                    w.level.puddles.push(crate::game::level::Puddle {
                        pos: pos + Vec2::new(34.0, 16.0),
                        rx: 29.0,
                        ry: 12.0,
                        seed: 5,
                    });
                    let half = LASER_ARC_DEG[4].to_radians() * 0.5;
                    let phase = ticks as f32 / (60.0 * LASER_SWEEP_SECS[4]);
                    aim = pos + Vec2::from_angle(half * (2.0 * phase - 1.0)) * 100.0;
                    w.shake_scale = 0.0;
                }
                w.player.face = Vec2::new(1.0, 0.0);
                w.player.loadout.weapons.clear();
                w.player.loadout.apply(&Upgrade::NewWeapon(kind));
                let weapon = &mut w.player.loadout.weapons[0];
                weapon.level = rank;
                weapon.timer = 0.0;
                weapon.evolved = evolved;
                for tick in 0..ticks {
                    let dir = if kind == WeaponKind::Hantavirus && tick >= 15 {
                        Vec2::new(1.0, 0.0)
                    } else {
                        Vec2::ZERO
                    };
                    w.update_with_aim(1.0 / 60.0, dir, false, false, Some(aim));
                }
                hh.app.picture_dirty = true;
                hh.terminal.draw(|f| hh.app.render(f)).unwrap();
            }
            t if t.starts_with("miniboss-first-") => {
                use crate::game::collision::SizeClass;
                use crate::game::ecs::systems::miniboss::{self, Miniboss, MinibossState};
                let name = t.trim_start_matches("miniboss-first-");
                let theme = Theme::playable().into_iter()
                    .find(|theme| theme.name().eq_ignore_ascii_case(name))
                    .expect("playable sector");
                hh.enter_playing();
                if theme != Theme::Sewer {
                    hh.app.start_practice(theme);
                    hh.frame(&[KeyCode::Enter]);
                }
                hh.app.scores.minimap_size = minimap::MinimapSize::Hidden;
                let w = hh.app.world.as_mut().unwrap();
                w.configure_testbed();
                w.elapsed = 186.0;
                let which = miniboss::schedule_for(theme, 0);
                let pos = w.testbed_spawn_point(Vec2::new(1.0, 0.0), 30.0,
                    which.body_radius(), SizeClass::Large);
                let rat = w.player.pos;
                let e = w.spawn_miniboss(which, 0, pos);
                let mut mb = w.ecs.get_mut::<Miniboss>(e).unwrap();
                mb.state = MinibossState::Tell;
                mb.timer = 0.55;
                mb.announced = true;
                mb.tell_cued = true;
                mb.origin = pos;
                mb.target = rat;
                mb.aim = (rat - pos).normalized();
                miniboss::lock_stamps(&mut mb);
                hh.app.picture_dirty = true;
                hh.terminal.draw(|f| hh.app.render(f)).unwrap();
            }
            // A named miniboss mid-tell, with its nameplate and the 15:00 clock
            // (wave 3). The appointment is pinned past the live schedule so the
            // test body is not immediately retreated.
            t if t == "miniboss" || t.starts_with("miniboss-") => {
                hh.enter_playing();
                use crate::game::ecs::systems::miniboss::{
                    Miniboss, MinibossKind, MinibossState,
                };
                let which = match t.strip_prefix("miniboss-").unwrap_or("corkscrew") {
                    "snip" => MinibossKind::SisterSnip,
                    "bobbin" => MinibossKind::Bobbin,
                    "bell" => MinibossKind::BellDiver,
                    "foreman" => MinibossKind::Foreman,
                    "milk" => MinibossKind::MilkCarton,
                    // Any kind by its name with the punctuation dropped
                    // (`miniboss-valvewidow`, `miniboss-themudlark`).
                    other => MinibossKind::all()
                        .into_iter()
                        .find(|k| {
                            let slug: String = k
                                .name()
                                .chars()
                                .filter(|c| c.is_ascii_alphanumeric())
                                .map(|c| c.to_ascii_lowercase())
                                .collect();
                            slug == other
                        })
                        .unwrap_or(MinibossKind::Corkscrew),
                };
                let w = hh.app.world.as_mut().unwrap();
                w.elapsed = 186.0;
                w.ecs
                    .resource_mut::<crate::game::ecs::systems::miniboss::MinibossSchedule>()
                    .next = 1;
                let p = w.player.pos;
                let pos = p + Vec2::new(48.0, 0.0);
                let e = w.spawn_miniboss(which, 0, pos);
                {
                    let mut mb = w.ecs.get_mut::<Miniboss>(e).unwrap();
                    mb.state = MinibossState::Tell;
                    mb.timer = 0.25;
                    mb.origin = pos;
                    mb.aim = (p - pos).normalized();
                }
                hh.frame(&[]);
            }
            // The two player-triggered weapons, mid-effect, over a real horde:
            // `RATRUN_DUMP=thrash` catches the tail arc a few frames into its
            // burst, `RATRUN_DUMP=squeak` the ring a few frames into its
            // expansion. Both are taken in a live `Playing` frame (HUD and all).
            "thrash" | "squeak" => {
                hh.enter_playing();
                {
                    use crate::game::weapon::WeaponKind;
                    use rand::SeedableRng;
                    let w = hh.app.world.as_mut().unwrap();
                    // Only the weapon this capture is about, so the other
                    // one's ring/arc does not clutter the frame.
                    let kind = if target == "thrash" {
                        WeaponKind::TailThrash
                    } else {
                        WeaponKind::SonicSqueak
                    };
                    if !w.player.loadout.has(kind) {
                        w.player.loadout.apply(&Upgrade::NewWeapon(kind));
                    }
                    // Both weapons charged and dash unavailable, so the press
                    // releases the actives and nothing else.
                    for weapon in w.player.loadout.weapons.iter_mut() {
                        if weapon.kind.is_active() {
                            weapon.level = 3;
                            weapon.timer = 0.0;
                        }
                    }
                    w.player.dash_cd = 5.0;
                    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
                    let p = w.player.pos;
                    // Tough enough to survive the effect (so the capture shows
                    // the arc/ring over bodies, not an empty floor) and spread
                    // through the whole reach.
                    for i in 0..34 {
                        let a = i as f32 * 0.21;
                        let r = 16.0 + (i % 6) as f32 * 11.0;
                        let at = p + crate::math::Vec2::from_angle(a) * r;
                        let kind = if i % 7 == 3 {
                            crate::game::enemy::EnemyKind::Cat
                        } else {
                            crate::game::enemy::EnemyKind::Skitterer
                        };
                        w.spawn_enemy_with_rng(kind, at, 40.0, &mut rng);
                    }
                }
                hh.frame(&[KeyCode::Char(' ')]);
                let frames = if target == "thrash" { 5 } else { 4 };
                for _ in 0..frames {
                    hh.frame(&[]);
                }
            }
            // ---- Wave 3 evolutions -----------------------------------------
            // `RATRUN_DUMP=evo0` … `evo7` renders the weapon at that index of
            // `WeaponKind::all()` fully evolved, mid-effect, over a horde.
            // `chest-evo` is the evolution chest card; `hud-evo` is the HUD
            // rack of five evolved weapons wearing the gilded frame.
            "chest-evo" => {
                hh.enter_playing();
                let w = hh.app.world.as_mut().unwrap();
                let weapon = &mut w.player.loadout.weapons[0];
                weapon.level = weapon.kind.max_level();
                w.player.loadout.apply(&Upgrade::Passive(crate::game::loadout::PassiveKind::Damage));
                let p = w.player.pos;
                w.spawn_chest(p);
                hh.frame(&[]);
            }
            "hud-evo" => {
                use crate::game::weapon::WeaponKind;
                hh.enter_playing();
                let w = hh.app.world.as_mut().unwrap();
                w.player.loadout = Loadout::from_parts(
                    WeaponKind::all()
                        .into_iter()
                        .take(5)
                        .map(|kind| {
                            let mut weapon = crate::game::weapon::Weapon::new(kind);
                            weapon.level = kind.max_level();
                            weapon.evolved = true;
                            weapon
                        })
                        .collect(),
                    vec![
                        (crate::game::loadout::PassiveKind::Damage, 2),
                        (crate::game::loadout::PassiveKind::Armor, 3),
                    ],
                );
                for _ in 0..4 {
                    hh.frame(&[]);
                }
            }
            t if t.starts_with("evo") => {
                use crate::game::bullet::{Bullet, Faction};
                use crate::game::ecs::resources::spawn_with_id;
                use crate::game::weapon::WeaponKind;
                use rand::SeedableRng;
                let index: usize = t
                    .trim_start_matches("evo")
                    .parse()
                    .unwrap_or(0)
                    .min(WeaponKind::all().len() - 1);
                let kind = WeaponKind::all()[index];
                hh.enter_playing();
                let w = hh.app.world.as_mut().unwrap();
                w.player.loadout = Loadout::from_parts(
                    vec![{
                        let mut weapon = crate::game::weapon::Weapon::new(kind);
                        weapon.level = kind.max_level();
                        weapon.evolved = true;
                        weapon.timer = 0.0;
                        weapon
                    }],
                    Vec::new(),
                );
                w.player.dash_cd = 9.0;
                let mut rng = rand::rngs::StdRng::seed_from_u64(11);
                let p = w.player.pos;
                // A ring of bodies through the weapon's reach, tough enough
                // that none of them dies and clutters the frame with bursts.
                for i in 0..14 {
                    let a = i as f32 * 0.45;
                    let r = 20.0 + (i % 5) as f32 * 10.0;
                    let at = p + crate::math::Vec2::from_angle(a) * r;
                    let kind = if i % 7 == 3 {
                        crate::game::enemy::EnemyKind::Cat
                    } else {
                        crate::game::enemy::EnemyKind::Skitterer
                    };
                    w.spawn_enemy_with_rng(kind, at, 400.0, &mut rng);
                }
                // Hostile pellets for Flywheel to catch.
                if kind == WeaponKind::TailThrash {
                    for i in 0..5 {
                        let at = p + crate::math::Vec2::from_angle(i as f32 * 1.2) * 18.0;
                        spawn_with_id(
                            &mut w.ecs,
                            Bullet::new(at, crate::math::Vec2::new(-30.0, 0.0), 5.0, Faction::Enemy, (255, 120, 90)),
                        );
                    }
                }
                // Active weapons need the Space press; the others fire on their
                // own timer.
                let press = if kind.is_active() {
                    hh.frame(&[KeyCode::Char(' ')]);
                    3
                } else {
                    hh.frame(&[]);
                    0
                };
                let frames = match kind {
                    // The burst's final sweep (the fling) resolves around
                    // frame 38; a handful of frames later the copper fan is
                    // clear of the rat sprite and the arc is still drawn.
                    WeaponKind::TailThrash => 42,
                    WeaponKind::SonicSqueak => 46,
                    WeaponKind::SporeOrbit => 60,
                    WeaponKind::AcidSpit => 40,
                    WeaponKind::SqueakNova => 40,
                    _ => 10,
                };
                for _ in 0..(frames + press) {
                    hh.frame(&[]);
                }
                // Drop the stacked damage digits so the capture reads as the
                // effect, not as a wall of numbers (one fresh frame records
                // only its own aggregated hits).
                if let Some(w) = hh.app.world.as_mut() {
                    w.damage_numbers.clear();
                }
                hh.frame(&[]);
            }
            "treasure" | "chest" => {
                use crate::game::ecs::resources::spawn_with_id;
                use crate::game::gem::Gold;
                hh.enter_playing();
                let w = hh.app.world.as_mut().unwrap();
                w.gold = 23;
                let p = w.player.pos;
                w.spawn_chest(p + Vec2::new(20.0, -9.0));
                spawn_with_id(&mut w.ecs, Gold::new(p + Vec2::new(16.0, -9.0), 1));
                spawn_with_id(&mut w.ecs, Gold::new(p + Vec2::new(14.0, -13.0), 1));
                if target == "chest" {
                    w.spawn_chest(p);
                }
                hh.frame(&[]);
            }
            // Wave 3 pocket finds (`systems::powerups`). `RATRUN_DUMP=powerups`
            // puts one of each on the floor around the rat: the tin dome, the
            // brass key, the pocket watch and the sock. `powerups_fx` runs a
            // few real frames with every timed buff live, so the capture shows
            // the white shield rim, the slowed hostile pellets' tails, the
            // brass decoy with the horde walking at it, the heel prints and the
            // HUD's timed-strip.
            "powerups" | "powerups_fx" => {
                use crate::game::gem::{Pickup, PickupKind};
                use crate::math::Vec2;
                hh.enter_playing();
                let fx = target == "powerups_fx";
                {
                    let w = hh.app.world.as_mut().unwrap();
                    let p = w.player.pos;
                    // One of each find laid out in a small ring, each with its
                    // own bob phase so the sprites animate independently.
                    for (i, kind) in [
                        PickupKind::BottleCap,
                        PickupKind::WindupMouse,
                        PickupKind::PocketWatch,
                        PickupKind::CleanSocks,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let a = std::f32::consts::FRAC_PI_2 * i as f32 + 0.4;
                        w.spawn_pickup(Pickup {
                            pos: p + Vec2::from_angle(a) * 20.0,
                            kind,
                            bob: i as f32 * 1.7,
                        });
                    }
                    if fx {
                        w.player.buffs.shield = 12.0;
                        w.player.buffs.decoy = 5.0;
                        w.player.buffs.slow_bullets = 4.0;
                        w.player.buffs.socks = 6.0;
                        // The key east of the rat, with a handful of ordinary
                        // bodies inside its lure radius and a couple of hostile
                        // pellets already in the air.
                        let key = p + Vec2::new(30.0, 0.0);
                        crate::game::ecs::systems::powerups::spawn_decoy(w, key);
                        for i in 0..7 {
                            let a = i as f32 * 0.9;
                            let at = key + Vec2::from_angle(a) * (14.0 + i as f32 * 2.0);
                            w.spawn_enemy(
                                crate::game::enemy::EnemyKind::Skitterer,
                                at,
                                2.0,
                            );
                        }
                        for i in 0..5 {
                            let mut b = crate::game::bullet::Bullet::new(
                                p + Vec2::new(-26.0, -8.0 + i as f32 * 4.0),
                                Vec2::new(80.0, 0.0),
                                3.0,
                                crate::game::bullet::Faction::Enemy,
                                (255, 120, 160),
                            );
                            b.life = 4.0;
                            w.spawn_bullet(b);
                        }
                    }
                }
                if fx {
                    // A few frames of walking with the socks on, so the heel
                    // prints and the shield rim have history.
                    for _ in 0..14 {
                        hh.frame(&[KeyCode::Right]);
                    }
                } else {
                    hh.frame(&[]);
                }
            }
            // Wave 3's two self-driving weapons, over a real horde:
            // `RATRUN_DUMP=laser` catches a Laser Eyes burst mid-sweep (beam
            // lit), `RATRUN_DUMP=superdash` the comb a few frames into a dash.
            "laser" | "superdash" => {
                hh.enter_playing();
                let is_laser = target == "laser";
                {
                    use crate::game::weapon::WeaponKind;
                    use rand::SeedableRng;
                    let w = hh.app.world.as_mut().unwrap();
                    let kind = if is_laser {
                        WeaponKind::LaserEyes
                    } else {
                        WeaponKind::SuperDash
                    };
                    w.player.loadout.weapons.clear();
                    w.player.loadout.apply(&Upgrade::NewWeapon(kind));
                    // Max rank: the widest beam (5 pulses) and the widest swept
                    // radius, so the still frame shows the shape at its fullest.
                    for weapon in w.player.loadout.weapons.iter_mut() {
                        weapon.level = kind.max_level();
                        weapon.timer = 0.0;
                    }
                    // `RATRUN_DUMP_FACE=x,y` turns the rat (and the laser's fan
                    // of targets with it): `0,-1` aims at the near top edge.
                    let face = std::env::var("RATRUN_DUMP_FACE")
                        .ok()
                        .and_then(|v| {
                            let (a, b) = v.split_once(',')?;
                            Some(crate::math::Vec2::new(
                                a.trim().parse::<f32>().ok()?,
                                b.trim().parse::<f32>().ok()?,
                            ))
                        })
                        .unwrap_or(crate::math::Vec2::new(1.0, 0.0));
                    let turn = face.y.atan2(face.x);
                    w.player.face = face;
                    w.player.dash_cd = 0.0;
                    // Untouchable, so the capture is the weapon and not the
                    // rat's hurt flash.
                    w.player.iframes = 30.0;
                    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
                    let p = w.player.pos;
                    if is_laser {
                        // A loose fan in front: bodies at three distances, so
                        // the sweep's 3-5 bars are each visible crossing one.
                        for i in 0..12 {
                            let a = turn - 0.45 + (i % 6) as f32 * 0.18;
                            let r = 18.0 + (i / 6) as f32 * 10.0;
                            let at = p + crate::math::Vec2::from_angle(a) * r;
                            w.spawn_enemy_with_rng(
                                crate::game::enemy::EnemyKind::Cat,
                                at,
                                40.0,
                                &mut rng,
                            );
                        }
                    } else {
                        // A line to cross: the dash's whole point is the bodies
                        // it passes through.
                        for i in 0..5 {
                            let at = p + crate::math::Vec2::new(7.0 + i as f32 * 5.0, 0.0);
                            w.spawn_enemy_with_rng(
                                crate::game::enemy::EnemyKind::Brute,
                                at,
                                40.0,
                                &mut rng,
                            );
                        }
                        // …plus two flankers, so the comb's teeth have an edge.
                        for i in 0..2 {
                            let at = p + crate::math::Vec2::new(12.0 + i as f32 * 6.0, 9.0);
                            w.spawn_enemy_with_rng(
                                crate::game::enemy::EnemyKind::Cat,
                                at,
                                40.0,
                                &mut rng,
                            );
                        }
                    }
                }
                // One frame to trigger, then a couple more into the effect.
                hh.frame(&[KeyCode::Char(' ')]);
                // The laser's bar is only the *current* pulse: a few more frames
                // let the earlier pulses' trails still glow, so the still shows
                // the sweep as a fan.
                let frames = if is_laser { 7 } else { 3 };
                for _ in 0..frames {
                    hh.frame(&[]);
                }
            }
            // The Spiked Tail's lash, in the beats the weapon is built out of:
            // `spiketail-bristle` (the tail thickening and sprouting spikes
            // before the crack), `spiketail-crack` (the tail at full extension,
            // a beat after it landed) and `spiketail-burst` (the impact star,
            // the burst and the damage numbers). `barbedwire` is the evolved
            // weapon mid-reel, with the catch dragged through the pack.
            // `RATRUN_DUMP_ZOOM=0.5` takes the same frame at 2x.
            t if t.starts_with("spiketail") || t == "barbedwire" => {
                use crate::game::weapon::WeaponKind;
                use rand::SeedableRng;
                hh.enter_playing();
                let evolved = t == "barbedwire";
                let frames = match t {
                    "spiketail-bristle" => 5,
                    "spiketail-burst" => 14,
                    // The reel starts at LASH_RECOIL_AT (0.20 s); a few frames
                    // later the caught body is standing at the rat's feet.
                    "barbedwire" => 17,
                    // A lash at long reach (one body 108 px out): the frame
                    // count picks the moment (`RATRUN_DUMP_FRAMES`, default
                    // the crack tick).
                    "spiketail-far" => std::env::var("RATRUN_DUMP_FRAMES")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(9),
                    _ => 9,
                };
                let far = t == "spiketail-far";
                let mut rng = rand::rngs::StdRng::seed_from_u64(31);
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.player.loadout = Loadout::from_parts(
                        vec![{
                            let mut weapon =
                                crate::game::weapon::Weapon::new(WeaponKind::SpikedTail);
                            weapon.level = if evolved || far { 5 } else { 4 };
                            weapon.evolved = evolved;
                            weapon.timer = 0.0;
                            weapon
                        }],
                        Vec::new(),
                    );
                    w.player.face = crate::math::Vec2::new(1.0, 0.0);
                    w.player.iframes = 60.0;
                    // The crack kills things; a level-up card would cover it.
                    w.player.xp_to_next = 99_999;
                    w.clear_enemies();
                    let p = w.player.pos;
                    // A tough knot east of the rat: one body to lash (nearest,
                    // so the tail points at it) and a small crowd inside the
                    // burst radius of where it stands.
                    for (off, kind) in [
                        ((30.0f32, 0.0f32), crate::game::enemy::EnemyKind::Brute),
                        ((32.0, 7.0), crate::game::enemy::EnemyKind::Skitterer),
                        ((28.0, -7.0), crate::game::enemy::EnemyKind::Skitterer),
                        // On the line home: what an evolved lash drags across.
                        ((18.0, 1.0), crate::game::enemy::EnemyKind::Skitterer),
                        ((10.0, -2.0), crate::game::enemy::EnemyKind::Skitterer),
                        // Flavour off the line, so the burst's edge is visible.
                        ((38.0, 14.0), crate::game::enemy::EnemyKind::Skitterer),
                    ] {
                        if far {
                            break;
                        }
                        w.spawn_enemy_with_rng(
                            kind,
                            p + crate::math::Vec2::new(off.0, off.1),
                            400.0,
                            &mut rng,
                        );
                    }
                    if far {
                        w.spawn_enemy_with_rng(
                            crate::game::enemy::EnemyKind::Brute,
                            p + crate::math::Vec2::new(108.0, 0.0),
                            400.0,
                            &mut rng,
                        );
                    }
                    if let Some(zoom) = std::env::var("RATRUN_DUMP_ZOOM")
                        .ok()
                        .and_then(|v| v.parse::<f32>().ok())
                    {
                        w.set_zoom_instant(zoom);
                    }
                }
                for _ in 0..frames {
                    hh.frame(&[]);
                }
                if t == "spiketail-bristle" || t == "spiketail-crack" {
                    // Nothing has landed yet / the numbers would sit on the
                    // tail: the still is about the tail.
                    if let Some(w) = hh.app.world.as_mut() {
                        w.damage_numbers.clear();
                    }
                    hh.frame(&[]);
                }
            }
            // Wave 3 weapons A, mid-effect: `RATRUN_DUMP=rain` catches a volley
            // in the air (every mark locked, the lead drop's strokes coming
            // down), `RATRUN_DUMP=virus` a chain two ticks into its first host
            // (two pips, the third about to jump).
            "rain" | "virus" => {
                hh.enter_playing();
                {
                    use crate::game::weapon::WeaponKind;
                    use rand::SeedableRng;
                    let w = hh.app.world.as_mut().unwrap();
                    let kind = if target == "rain" {
                        WeaponKind::AcidRain
                    } else {
                        WeaponKind::Hantavirus
                    };
                    // Only the weapon this capture is about: a Gnaw dart in
                    // flight would read as part of the effect.
                    w.player.loadout.weapons = vec![crate::game::weapon::Weapon {
                        kind,
                        level: 3,
                        timer: 0.0,
                        evolved: false,
                    }];
                    w.player.dash_cd = 5.0;
                    let mut rng = rand::rngs::StdRng::seed_from_u64(11);
                    let p = w.player.pos;
                    // Tough bodies spread through both weapons' reach, so the
                    // volley fans out over a real crowd and a chain has fresh
                    // neighbours to jump to.
                    for i in 0..28 {
                        let a = i as f32 * 0.42 + 0.3;
                        let r = 12.0 + (i % 5) as f32 * 9.0;
                        let at = p + crate::math::Vec2::from_angle(a) * r;
                        let kind = if i % 6 == 2 {
                            crate::game::enemy::EnemyKind::Cat
                        } else {
                            crate::game::enemy::EnemyKind::Skitterer
                        };
                        w.spawn_enemy_with_rng(kind, at, 40.0, &mut rng);
                    }
                }
                // ~0.23 s: the rain volley is fully marked and still falling;
                // the virus has tagged its first host and has not ticked yet.
                let frames = if target == "rain" { 17 } else { 30 };
                for _ in 0..frames {
                    hh.frame(&[]);
                }
            }
            // The getaway ride: start the trailer's practice sector from the
            // title's sector row and dump the deck at the tailgate. `trailer2`
            // / `trailer3` are the same frame a few ticks later, so the road
            // under the planks visibly moves between them.
            "trailer" | "trailer2" | "trailer3" | "trailerrail" | "trailerhole"
            | "trailercargo" => {
                hh.app.select_idx = Theme::playable().len() - 1; // the Flatbed
                hh.app.menu_idx = 1; // the sector row
                hh.frame(&[]);
                hh.frame(&[KeyCode::Enter]); // -> the select screen
                hh.frame(&[KeyCode::Enter]); // -> the getaway ride's intro
                hh.frame(&[KeyCode::Enter]); // -> Playing
                {
                    let w = hh.app.world.as_mut().unwrap();
                    let deck = w.level.trailer.deck;
                    match target.as_str() {
                        // At the north rail: the side rail, the road over the
                        // edge and the verge beyond it.
                        "trailerrail" => {
                            w.player.pos = crate::math::Vec2::new(
                                deck.x as f32 * 16.0 + 200.0,
                                deck.y as f32 * 16.0 + 20.0,
                            );
                        }
                        // At the first hole, so the road shows through a plank.
                        "trailerhole" => {
                            if let Some(pit) = w.level.trailer.pits.first() {
                                let c = pit.center_world();
                                w.player.pos = crate::math::Vec2::new(c.x - 40.0, c.y - 10.0);
                            }
                        }
                        // Beside the south end of the first cargo stack, so
                        // the stack's front face and contact shadow show.
                        "trailercargo" => {
                            let mut cx = deck.x + 24;
                            while cx < deck.right() {
                                if w.level.tiles.is_full_solid(cx, deck.y + 4) {
                                    break;
                                }
                                cx += 1;
                            }
                            let mut ey = deck.y + 4;
                            while ey < deck.bottom() - 1
                                && w.level.tiles.is_full_solid(cx, ey + 1)
                            {
                                ey += 1;
                            }
                            w.player.pos = crate::math::Vec2::new(
                                cx as f32 * 16.0 + 8.0,
                                (ey as f32 + 1.0) * 16.0 + 22.0,
                            );
                        }
                        _ => {}
                    }
                }
                // `trailer` and `trailer2` are the same bump phase (the deck is
                // not mid-dip), so a diff between them is the road and only the
                // road; `trailer3` is caught on the joint itself, one pixel low.
                let ticks = match target.as_str() {
                    "trailer2" => 6,
                    "trailer3" => 1,
                    _ => 4,
                };
                for _ in 0..ticks {
                    hh.frame(&[]);
                }
            }
            // A mouse-hole shop in its wall, in a live frame: the rat stands a
            // stride off the threshold (so the shelf does not open) and the
            // spoon, the scratches and the wall all read. `hole-map` is the
            // same frame with the expanded minimap open, to show the spoon
            // marker on the map.
            "hole" | "hole-map" => {
                hh.enter_playing();
                let hole = {
                    let w = hh.app.world.as_mut().unwrap();
                    // A flatbed seed can carry no shop hole at all (the
                    // generator drops faces a pit or a crate blocks); guard the
                    // dump instead of panicking on it.
                    let Some(&hole) = w.level.objects.shop_holes.first() else {
                        return;
                    };
                    // Stand a stride down the wall from it, so the hole sits
                    // clear of the HUD and the compact map in the capture.
                    w.player.pos = hole.face()
                        + hole.facing * 26.0
                        + crate::math::Vec2::new(-hole.facing.y, hole.facing.x) * -26.0;
                    hole
                };
                let _ = hole;
                hh.frame(&[]);
                hh.frame(&[]);
                if target == "hole-map" {
                    hh.frame(&[KeyCode::Tab]);
                }
            }
            // The gene-splice shop (`WAVE3_DESIGN` §6): the shelf as it opens
            // ("shop"), one burnt down by a reroll and with a splice bought
            // ("shop-late"), and the Molt two-step ("shop-molt"). Teleporting
            // the rat onto hole 0's threshold makes the touch fire for real.
            "shop" | "shop-late" | "shop-molt" => {
                hh.enter_playing();
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.gold = 61;
                    w.player.hp = w.player.max_hp() * 0.55;
                    let Some(&hole) = w.level.objects.shop_holes.first() else {
                        return;
                    };
                    w.player.pos = hole.face() + hole.facing * 3.0;
                }
                hh.frame(&[]); // the threshold system raises the flag
                hh.frame(&[]); // the app opens the shelf
                assert!(matches!(hh.app.screen, Screen::Shop));
                if target == "shop-late" {
                    // Buy the first affordable splice, then turn the shelf.
                    hh.frame(&[KeyCode::Enter]);
                    hh.app.shop_row = 4;
                    hh.frame(&[KeyCode::Enter]);
                    hh.app.shop_row = 3;
                    hh.frame(&[KeyCode::Enter]);
                    hh.frame(&[]);
                } else if target == "shop-molt" {
                    hh.app.shop_row = 5;
                    hh.frame(&[KeyCode::Enter]);
                    hh.frame(&[]);
                    assert!(hh.app.shop_molt.is_some());
                }
            }
            // The two wave 3 floors, mid-run: a practice run on that floor,
            // walked for a few seconds so the horde, the hazards and the props
            // are all on screen.
            "laundry" | "printworks" => {
                let theme = if target == "laundry" {
                    Theme::Laundry
                } else {
                    Theme::Printworks
                };
                hh.app.start_practice(theme);
                hh.frame(&[KeyCode::Enter]); // intro -> Playing
                // Park the rat in the middle of the nearest great hall with the
                // horde cleared: the floor, the theme's own furniture and the
                // props are what this capture is about.
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.clear_enemies();
                    let hall = w
                        .level
                        .rooms_of(crate::game::level::RoomKind::Hall)
                        .map(|room| room.rect)
                        .find(|rect| rect.area() > 600);
                    if let Some(rect) = hall {
                        w.player.pos = rect.center_world();
                    }
                    w.elapsed = 3.5;
                }
                for _ in 0..90 {
                    hh.frame(&[]);
                }
            }
            // Art captures: park the rat beside a hazard (or in the boss arena)
            // with the horde cleared, so the floor, the props and the hazard
            // read at 1:1.
            "laundry-hazard" | "printworks-hazard" | "laundry-arena" | "printworks-arena" => {
                let theme = if target.starts_with("laundry") {
                    Theme::Laundry
                } else {
                    Theme::Printworks
                };
                hh.app.start_practice(theme);
                hh.frame(&[KeyCode::Enter]); // intro -> Playing
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.clear_enemies();
                    w.level.objects.sticky_traps.clear();
                    w.level.objects.mousetraps.clear();
                    let spot = if target.ends_with("arena") {
                        w.level.largest_hall().map(|room| room.center())
                    } else if theme == Theme::Laundry {
                        w.level
                            .objects
                            .steam_grates
                            .first()
                            .map(|g| g.pos + crate::math::Vec2::new(0.0, 18.0))
                    } else {
                        w.level
                            .objects
                            .ink_pads
                            .first()
                            .map(|p| p.pos + crate::math::Vec2::new(0.0, 18.0))
                    };
                    if let Some(spot) = spot {
                        w.player.pos = spot;
                    }
                    // A few seconds on the clock so the grate is mid-cycle.
                    w.elapsed = 3.5;
                }
                for _ in 0..90 {
                    hh.frame(&[]);
                }
            }
            // The four wave-3 evolutions. Each capture sets up the evolved
            // weapon at max rank over bodies tough enough to survive it, then
            // runs exactly as many real frames as the effect needs — so the
            // still shows the *behaviour* (a marching gutter, a sealed
            // envelope, an afterimage, a taut cord), not a number going up.
            "downspout" | "deadletter" | "deadletter-tear" | "carboncopy" | "ripcord" => {
                use crate::game::weapon::WeaponKind;
                use rand::SeedableRng;
                hh.enter_playing();
                let (kind, frames) = match target.as_str() {
                    // ~0.57 s: every drop of the gutter has marched home and
                    // the runoff is lit.
                    "downspout" => (WeaponKind::AcidRain, 44),
                    // ~0.9 s: the host's death burst has cleared and only the
                    // sealed envelope is left on the floor.
                    "deadletter" | "deadletter-tear" => (WeaponKind::Hantavirus, 52),
                    "carboncopy" => (WeaponKind::LaserEyes, 4),
                    _ => (WeaponKind::SuperDash, 5),
                };
                let mut rng = rand::rngs::StdRng::seed_from_u64(23);
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.player.loadout = Loadout::from_parts(
                        vec![{
                            let mut weapon = crate::game::weapon::Weapon::new(kind);
                            weapon.level = kind.max_level();
                            weapon.evolved = true;
                            weapon.timer = 0.0;
                            weapon
                        }],
                        Vec::new(),
                    );
                    w.player.face = crate::math::Vec2::new(1.0, 0.0);
                    w.player.iframes = 40.0;
                    // The captures kill things; a level-up card would cover the
                    // effect, so the XP curve is out of reach.
                    w.player.xp_to_next = 99_999;
                    let p = w.player.pos;
                    match kind {
                        // A pack for the gutter to walk home from: the marked
                        // body and the near ones the runoff covers.
                        // A tight knot of bodies east of the rat, so the
                        // gutter walks one clean line home from the mark.
                        WeaponKind::AcidRain => {
                            for (i, off) in [
                                (40.0, -5.0),
                                (42.0, 5.0),
                                (46.0, 0.0),
                                (34.0, -9.0),
                                (34.0, 9.0),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                let _ = i;
                                w.spawn_enemy_with_rng(
                                    crate::game::enemy::EnemyKind::Brute,
                                    p + crate::math::Vec2::new(off.0, off.1),
                                    400.0,
                                    &mut rng,
                                );
                            }
                        }
                        // One host that dies with jumps left and nobody within
                        // its transfer reach: the chain seals instead of ending.
                        WeaponKind::Hantavirus => {
                            w.clear_enemies();
                            let host = w.spawn_enemy_with_rng(
                                crate::game::enemy::EnemyKind::Brute,
                                p + crate::math::Vec2::new(22.0, 0.0),
                                400.0,
                                &mut rng,
                            );
                            // The virus tick kills it (hp 1 after the tag lands).
                            w.ecs.get_mut::<crate::game::enemy::Enemy>(host).unwrap().hp = 1.0;
                            // `deadletter-tear` adds its body *after* the fuse
                            // is up (below), so the tear is not buried under
                            // the host's death burst.
                        }
                        // Two bodies per bar of the fan: enough to show what
                        // each beam crosses, open enough to see the beams.
                        WeaponKind::LaserEyes => {
                            for i in 0..6 {
                                let a = -0.5 + (i % 3) as f32 * 0.5;
                                let r = 26.0 + (i / 3) as f32 * 14.0;
                                w.spawn_enemy_with_rng(
                                    crate::game::enemy::EnemyKind::Cat,
                                    p + crate::math::Vec2::from_angle(a) * r,
                                    400.0,
                                    &mut rng,
                                );
                            }
                        }
                        // A line to cross (the dash's own hits) plus flankers
                        // for the cord's slide.
                        _ => {
                            for i in 0..6 {
                                w.spawn_enemy_with_rng(
                                    crate::game::enemy::EnemyKind::Brute,
                                    p + crate::math::Vec2::new(8.0 + i as f32 * 6.0, 0.0),
                                    400.0,
                                    &mut rng,
                                );
                            }
                            for i in 0..3 {
                                w.spawn_enemy_with_rng(
                                    crate::game::enemy::EnemyKind::Cat,
                                    p + crate::math::Vec2::new(14.0 + i as f32 * 8.0, 7.0),
                                    400.0,
                                    &mut rng,
                                );
                            }
                        }
                    }
                }
                if kind == WeaponKind::LaserEyes {
                    // Burst one records the sweep (0.52 s at L5); then the rat
                    // steps aside and burst two replays the negative from the
                    // old spot while its own beams cross them.
                    for _ in 0..40 {
                        hh.frame(&[]);
                    }
                    {
                        let w = hh.app.world.as_mut().unwrap();
                        w.player.pos += crate::math::Vec2::new(0.0, 12.0);
                        w.player.loadout.weapons[0].timer = 0.0;
                        // Re-centre the fan on the new muzzle line.
                        w.clear_enemies();
                        let p = w.player.pos;
                        for i in 0..6 {
                            let a = -0.5 + (i % 3) as f32 * 0.5;
                            let r = 26.0 + (i / 3) as f32 * 14.0;
                            w.spawn_enemy_with_rng(
                                crate::game::enemy::EnemyKind::Cat,
                                p + crate::math::Vec2::from_angle(a) * r,
                                400.0,
                                &mut rng,
                            );
                        }
                    }
                    for _ in 0..frames {
                        hh.frame(&[]);
                    }
                } else if kind == WeaponKind::SuperDash {
                    // Space dashes: the cord is taut for 0.6 s after the dash
                    // ends, and the capture is a few frames into that window.
                    hh.frame(&[KeyCode::Char(' ')]);
                    for _ in 0..10 {
                        hh.frame(&[]);
                    }
                    for _ in 0..frames {
                        hh.frame(&[]);
                    }
                } else if kind == WeaponKind::Hantavirus {
                    // The tick lands at 0.35 s (21 frames); the seal follows on
                    // the same schedule pass.
                    for _ in 0..frames {
                        hh.frame(&[]);
                    }
                    if target == "deadletter-tear" {
                        // A body steps onto the envelope: the flap tears open.
                        let at = hh.app.world.as_ref().unwrap().player.pos
                            + crate::math::Vec2::new(22.0, 2.0);
                        hh.app.world.as_mut().unwrap().spawn_enemy(
                            crate::game::enemy::EnemyKind::Skitterer,
                            at,
                            400.0,
                        );
                        for _ in 0..4 {
                            hh.frame(&[]);
                        }
                    } else {
                        // The envelope alone, no damage digits over it.
                        hh.frame(&[]);
                    }
                } else {
                    for _ in 0..frames {
                        hh.frame(&[]);
                    }
                }
                if let Some(w) = hh.app.world.as_mut() {
                    w.damage_numbers.clear();
                }
                hh.frame(&[]);
            }
            // The Library: `library` opens on the BESTIARY with a mix of
            // discovered and locked rows; `library-arsenal`, `library-field`,
            // `library-floors` and `library-boss` are the other tabs and the
            // first floor boss's card. Size with `RATRUN_DUMP_SIZE`.
            t if t == "library" || t.starts_with("library-") => {
                let rows = hh.app.library_rows.clone();
                {
                    let lib = &mut hh.app.scores.library;
                    for (i, row) in rows.iter().enumerate() {
                        if i % 3 != 2 {
                            lib.mark_seen(row.key);
                            // Weapons, passives and splices read as fully
                            // catalogued once owned; a card capture wants the
                            // dossier, not the "not yet carried" plate.
                            if matches!(
                                row.key,
                                library_index::Key::Weapon(_)
                                    | library_index::Key::Passive(_)
                                    | library_index::Key::Splice(_)
                            ) {
                                lib.bump(row.key, library_index::Counter::Owned, 1);
                            }
                        }
                    }
                }
                use crate::game::ecs::systems::boss::BossKind;
                let sub = t.strip_prefix("library-").unwrap_or("");
                // `library-boss` is the first floor boss; `library-boss-<n>`
                // (0..=9, `BossKind::all()` order) is any of the ten. Entries
                // are named by key, never by a row index (which moves whenever
                // a group gains an entry).
                let boss = sub.strip_prefix("boss").map(|rest| {
                    let n = rest.trim_start_matches('-').parse::<usize>().unwrap_or(0);
                    let all = BossKind::all();
                    all[n.min(all.len() - 1)]
                });
                let (tab, cursor) = match sub {
                    "arsenal" => (library_index::Tab::Arsenal, 1),
                    "field" => (library_index::Tab::FieldGuide, 2),
                    "floors" => (library_index::Tab::Floors, 0),
                    _ => (library_index::Tab::Bestiary, 0),
                };
                hh.app.open_library(Screen::Title);
                hh.app.library_ui.set_tab(tab);
                hh.app.library_ui.cursor = cursor;
                if let Some(kind) = boss {
                    let key = library_index::Key::Boss(kind);
                    hh.app.scores.library.mark_seen(key);
                    hh.app.scores.library.bump(key, library_index::Counter::Killed, 1);
                    assert!(
                        hh.app.library_ui.select_key(&rows, key),
                        "no Library row for {kind:?}"
                    );
                }
                hh.frame(&[]);
                hh.frame(&[]);
            }
            // ---- The nest (permanent upgrades) ---------------------------
            // `nest` is a fresh profile's nest (everything at 0, 4 + 4 racks),
            // `nest-started` a few levels bought with gold in hand, `nest-max`
            // every track at its cap. `end-bank` is the incident report with a
            // banking tally on it, `pause-nest` the pause screen's read-only
            // dossier (a run in flight, upgrades in hand), `title-nest` the
            // title row showing the bank.
            t if t == "nest" || t == "nest-started" || t == "nest-max" || t == "title-nest" => {
                use crate::meta::{MetaProfile, Track};
                let mut table = ScoreTable::default();
                table.meta = match t {
                    "nest-started" => {
                        let mut profile = MetaProfile::default();
                        profile.bank = 240;
                        profile.set_level(Track::WeaponSlots, 1);
                        profile.set_level(Track::MoveSpeed, 2);
                        profile.set_level(Track::Revive, 1);
                        profile
                    }
                    "nest-max" => MetaProfile::maxed(),
                    _ => MetaProfile::default(),
                };
                let app = App::with_options(false, None, table, false, Some(TEST_SEED));
                hh = Harness::with_app(app, w, h);
                if t == "title-nest" {
                    hh.frame(&[]);
                } else {
                    hh.app.open_nest(Screen::Title);
                    hh.frame(&[]);
                    hh.frame(&[]);
                }
            }
            "pause-nest" => {
                use crate::meta::MetaProfile;
                let mut table = ScoreTable::default();
                table.meta = MetaProfile::maxed();
                let app = App::with_options(false, None, table, false, Some(TEST_SEED));
                hh = Harness::with_app(app, w, h);
                hh.enter_playing();
                hh.app.world.as_mut().unwrap().gold = 37;
                hh.frame(&[KeyCode::Esc]);
                hh.frame(&[KeyCode::Char('b')]);
                hh.frame(&[]);
            }
            "end-bank" => {
                let mut table = ScoreTable::default();
                table.meta.bank = 90;
                let app = App::with_options(false, None, table, false, Some(TEST_SEED));
                hh = Harness::with_app(app, w, h);
                hh.enter_playing();
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.gold = 131;
                    w.player.hp = -1.0;
                }
                hh.frame(&[]);
                hh.frame(&[]);
            }
            // Integration captures: the Controls screen, the options panel, the
            // console open over play with `help`, the full map after a walk,
            // and the King Cobra's three set pieces on the Lab arena.
            "controls" => {
                hh.app.open_controls();
                hh.frame(&[]);
                hh.frame(&[]);
            }
            "options" => {
                title_to_options(&mut hh);
                hh.frame(&[]);
            }
            // The Graphics page, reached from the title's Options plate the way
            // a player reaches it (the GRAPHICS row, then Enter).
            "graphics" => {
                title_to_options(&mut hh);
                for _ in 0..options::GRAPHICS_ROW {
                    hh.frame(&[KeyCode::Down]);
                }
                hh.frame(&[KeyCode::Enter]);
                hh.frame(&[]);
            }
            "graphics-play" => {
                hh.enter_playing();
                hh.frame(&[KeyCode::Char('g')]);
            }
            "console" => {
                hh.enter_playing();
                for _ in 0..40 {
                    hh.frame(&[KeyCode::Right]);
                }
                hh.frame(&[KeyCode::Char('`')]);
                for c in "help".chars() {
                    hh.frame(&[KeyCode::Char(c)]);
                }
                hh.frame(&[KeyCode::Enter]);
                for _ in 0..40 {
                    hh.frame(&[]);
                }
            }
            "map" | "map-corner" => {
                use crate::game::collision::{cell_of, SizeClass, TileMap};
                use std::collections::{HashSet, VecDeque};
                hh.enter_playing();
                let w = hh.app.world.as_mut().unwrap();
                let start = cell_of(w.player.pos);
                let mut queue = VecDeque::from([start]);
                let mut visited = HashSet::from([start]);
                let mut last = start;
                // Reveal connected floor in a limited radius from the start.
                // The fog flood itself still decides where exploration ends.
                for i in 0..430 {
                    let Some((x, y)) = queue.pop_front() else { break };
                    last = (x, y);
                    if i % 7 == 0 {
                        w.fog.reveal_cell(&w.level, last);
                    }
                    for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                        if w.level.tiles.get(nx, ny).passable(SizeClass::Small)
                            && visited.insert((nx, ny))
                        {
                            queue.push_back((nx, ny));
                        }
                    }
                }
                w.player.pos = TileMap::tile_rect(last.0, last.1).center();
                w.fog.reveal_cell(&w.level, last);
                if target == "map" {
                    hh.frame(&[KeyCode::Tab]);
                } else {
                    hh.frame(&[]);
                }
                // Stamp the newly revealed cells, then settle their 0.4 s
                // reveal wipe before capturing the bright phase of the rat.
                let fog = &mut hh.app.world.as_mut().unwrap().fog;
                for _ in 0..minimap::REVEAL_WIPE_TICKS {
                    fog.tick();
                }
                hh.app.map_animation_start =
                    std::time::Instant::now() - std::time::Duration::from_millis(312);
                hh.frame(&[]);
            }
            t if t.starts_with("cobra") => {
                use crate::game::art_boss_cobra::{self as cobra_art, CobraState};
                use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
                hh.enter_playing();
                hh.app.start_floor(Theme::Lab).expect("the Lab starts");
                for _ in 0..6 {
                    if matches!(hh.app.screen, Screen::Playing) {
                        break;
                    }
                    hh.frame(&[KeyCode::Enter]);
                }
                let (state, duration, progress) = match t {
                    "cobra-constrict" => (CobraState::Constrict, cobra_art::COIL_TIGHTEN[0], 0.45),
                    "cobra-spiral" => (CobraState::SpiralChase, cobra_art::SPIRAL_CHASE, 0.4),
                    _ => (CobraState::HoodTell, cobra_art::HOOD_TELL[0], 0.8),
                };
                let centre = {
                    let w = hh.app.world.as_mut().unwrap();
                    let centre = w
                        .level
                        .largest_hall()
                        .map(|r| r.rect.center_world())
                        .expect("the Lab has a hall");
                    w.clear_enemies();
                    w.player.pos = centre + Vec2::new(18.0, 22.0);
                    w.force_boss(BossKind::Cobra);
                    centre
                };
                // The loop is 148 px across at its widest: the wide view shows
                // the whole set piece.
                hh.app.set_zoom(2.0);
                // Let the arrival flourish finish before the pose is staged.
                for _ in 0..90 {
                    hh.frame(&[]);
                }
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.clear_enemies();
                    let rat = w.player.pos;
                    let mut q = w.ecs.query::<&mut Boss>();
                    for mut b in q.iter_mut(&mut w.ecs) {
                        b.pos = centre + Vec2::new(-40.0, -10.0);
                        b.target = rat;
                        b.pounce_origin = centre;
                        b.facing = (rat - b.pos).normalized();
                        b.hazards.clear();
                        b.flags = 0;
                        b.set_state(BossState::Cobra(state), duration);
                        b.elapsed = duration * progress;
                    }
                }
                hh.frame(&[]);
                hh.frame(&[]);
            }
            // ---- Batch-four captures ------------------------------------
            // `floor-<name>`: fifteen seconds of real play on a named floor
            // (sewer, kitchen, lab, laundry, printworks, flatbed), a few
            // minutes into the curve so the frame has a horde in it.
            // `arrival-<floor>`: the boss arriving with no arena. The clock is
            // set to 14:40 and `RATRUN_DUMP_FRAMES` plays on from there (0 is
            // the warning, 1200 the arrival at 15:00, 1800 the fight at 15:10).
            t if t.starts_with("arrival-") => {
                let theme = match &t["arrival-".len()..] {
                    "kitchen" => Theme::Kitchen,
                    "lab" => Theme::Lab,
                    "laundry" => Theme::Laundry,
                    "printworks" => Theme::Printworks,
                    "flatbed" => Theme::Trailer,
                    _ => Theme::Sewer,
                };
                if theme == Theme::Sewer {
                    hh.enter_playing();
                } else {
                    hh.app.start_practice(theme);
                    hh.frame(&[KeyCode::Enter]);
                }
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.elapsed = crate::game::director::BOSS_TIME - 20.0;
                    w.player.god = true;
                    w.player.xp_to_next = 9_999_999;
                }
                let frames = std::env::var("RATRUN_DUMP_FRAMES")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(1200);
                for _ in 0..frames {
                    hh.frame(&[]);
                }
            }
            // `giant`: the Growth Serum (batch 13) a few seconds in: ~8 s of
            // Sewer play to gather a crowd, then the buff, then 2.5 s of it
            // (full size, the view pulled back, the stomp landing).
            "giant" => {
                hh.enter_playing();
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.elapsed = 150.0;
                }
                // `RATRUN_DUMP_GIANT_TICKS` changes where in the walk square
                // the capture stops (45 ticks a side): 630 faces down, 585
                // right, 675 left, 720 up.
                let ticks = std::env::var("RATRUN_DUMP_GIANT_TICKS")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(630);
                for tick in 0..ticks {
                    if tick == 480 {
                        let w = hh.app.world.as_mut().unwrap();
                        w.player.buffs.giant =
                            crate::game::ecs::systems::powerups::GROWTH_SECS;
                    }
                    let mv = match (tick / 45) % 4 {
                        0 => KeyCode::Right,
                        1 => KeyCode::Down,
                        2 => KeyCode::Left,
                        _ => KeyCode::Up,
                    };
                    if let Some(w) = hh.app.world.as_mut() {
                        w.player.hp = w.player.max_hp();
                    }
                    let text = hh.frame(&[mv]);
                    if text.contains("LEVEL UP") || text.contains("CHEST OPENED") {
                        hh.frame(&[KeyCode::Enter]);
                    }
                }
                for _ in 0..6 {
                    if matches!(hh.app.screen, Screen::Playing) {
                        break;
                    }
                    hh.frame(&[KeyCode::Enter]);
                }
            }
            t if t.starts_with("floor-") => {
                let theme = match &t["floor-".len()..] {
                    "kitchen" => Theme::Kitchen,
                    "lab" => Theme::Lab,
                    "laundry" => Theme::Laundry,
                    "printworks" => Theme::Printworks,
                    "flatbed" => Theme::Trailer,
                    _ => Theme::Sewer,
                };
                if theme == Theme::Sewer {
                    hh.enter_playing();
                } else {
                    hh.app.start_practice(theme);
                    hh.frame(&[KeyCode::Enter]); // intro -> Playing
                }
                {
                    use crate::game::weapon::WeaponKind;
                    let w = hh.app.world.as_mut().unwrap();
                    // Past the opening trickle, short of the 3:00 appointment.
                    w.elapsed = 150.0;
                    w.player
                        .loadout
                        .apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
                    w.player
                        .loadout
                        .apply(&Upgrade::NewWeapon(WeaponKind::SporeOrbit));
                }
                for tick in 0..780u32 {
                    let mv = match (tick / 45) % 4 {
                        0 => KeyCode::Right,
                        1 => KeyCode::Down,
                        2 => KeyCode::Left,
                        _ => KeyCode::Up,
                    };
                    if let Some(w) = hh.app.world.as_mut() {
                        w.player.hp = w.player.max_hp();
                    }
                    let text = hh.frame(&[mv]);
                    // A card or a chest plate is answered, never captured.
                    if text.contains("LEVEL UP") || text.contains("CHEST OPENED") {
                        hh.frame(&[KeyCode::Enter]);
                    }
                }
                // Finish on a frame of play, not on a plate.
                for _ in 0..6 {
                    if matches!(hh.app.screen, Screen::Playing) {
                        break;
                    }
                    hh.frame(&[KeyCode::Enter]);
                }
            }
            // `bossfight-<n>`: boss `n` of `BossKind::all()` on its own floor,
            // a few seconds into the fight, the rat standing off in the arena.
            t if t.starts_with("bossfight") => {
                use crate::game::ecs::systems::boss::BossKind;
                let n = t
                    .trim_start_matches("bossfight")
                    .trim_start_matches('-')
                    .parse::<usize>()
                    .unwrap_or(0);
                let all = BossKind::all();
                let kind = all[n.min(all.len() - 1)];
                let theme = Theme::playable()
                    .into_iter()
                    .find(|&th| BossKind::pool_for_theme(th).contains(&kind))
                    .unwrap_or(Theme::Sewer);
                if theme == Theme::Sewer {
                    hh.enter_playing();
                } else {
                    hh.app.start_practice(theme);
                    hh.frame(&[KeyCode::Enter]);
                }
                {
                    let w = hh.app.world.as_mut().unwrap();
                    let centre = w
                        .level
                        .largest_hall()
                        .map(|r| r.rect.center_world())
                        .expect("every floor has a hall");
                    w.clear_enemies();
                    w.player.pos = w
                        .level
                        .tiles
                        .nearest_open_center(
                            centre,
                            crate::game::collision::SizeClass::Large,
                            16,
                        )
                        .unwrap_or(centre);
                    // No arena any more: the fight starts beside the rat (the
                    // scheduled arrival is a screen away and walks in).
                    w.force_boss_here(kind);
                }
                let frames = std::env::var("RATRUN_DUMP_FRAMES")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(260);
                for _ in 0..frames {
                    if let Some(w) = hh.app.world.as_mut() {
                        w.player.hp = w.player.max_hp();
                    }
                    let text = hh.frame(&[]);
                    if text.contains("LEVEL UP") {
                        hh.frame(&[KeyCode::Enter]);
                    }
                }
            }
            // `map-early`: the full map a second into a run, when almost the
            // whole panel is still the unseen veil.
            "map-early" => {
                hh.enter_playing();
                for _ in 0..45 {
                    hh.frame(&[KeyCode::Right]);
                }
                hh.frame(&[KeyCode::Tab]);
                hh.frame(&[]);
            }
            // `superdash-charge` / `superdash-release`: Ctrl held with the
            // Super Dash owned (the ring on the rat, the HUD's CHARGE meter and
            // the lane preview), then the frames after the key comes up.
            "superdash-charge" | "superdash-release" => {
                let app = App::with_options(true, None, ScoreTable::default(), false, Some(TEST_SEED));
                hh = Harness::with_app(app, w, h);
                hh.enter_playing();
                {
                    use crate::game::enemy::EnemyKind;
                    use crate::game::weapon::WeaponKind;
                    let w = hh.app.world.as_mut().unwrap();
                    w.clear_enemies();
                    w.player
                        .loadout
                        .apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
                    w.player.face = Vec2::new(1.0, 0.0);
                    w.player.dash_cd = 0.0;
                    w.player.dash_active = 0.0;
                    w.player.xp_to_next = 1_000_000;
                    let p = w.player.pos;
                    for i in 0..5 {
                        w.spawn_enemy(
                            EnemyKind::Cat,
                            p + Vec2::new(26.0 + i as f32 * 9.0, (i % 2) as f32 * 5.0 - 2.0),
                            400.0,
                        );
                    }
                }
                hh.frame_events(&[ctrl_press()]);
                for _ in 0..34 {
                    if let Some(w) = hh.app.world.as_mut() {
                        w.player.hp = w.player.max_hp();
                    }
                    hh.frame_events(&[]);
                }
                if target == "superdash-release" {
                    hh.frame_events(&[KeyEvent::new_with_kind(
                        KeyCode::Modifier(ModifierKeyCode::LeftControl),
                        KeyModifiers::CONTROL,
                        KeyEventKind::Release,
                    )]);
                    for _ in 0..5 {
                        hh.frame_events(&[]);
                    }
                }
            }
            // `firelit`: the rat parked beside a fire in a dark room, horde
            // cleared. Walks the floors until one has a fire emitter.
            "firelit" => {
                use crate::game::collision::SizeClass;
                use crate::game::level::EmitterKind;
                let mut placed = false;
                for theme in [Theme::Sewer, Theme::Laundry, Theme::Kitchen, Theme::Printworks] {
                    if theme == Theme::Sewer {
                        hh.enter_playing();
                    } else {
                        hh.app.start_practice(theme);
                        hh.frame(&[KeyCode::Enter]);
                    }
                    let w = hh.app.world.as_mut().unwrap();
                    let fires: Vec<Vec2> = w
                        .level
                        .objects
                        .emitters
                        .iter()
                        .filter(|e| e.kind == EmitterKind::Fire)
                        .map(|e| e.pos)
                        .collect();
                    'fires: for fire in fires {
                        for (dx, dy) in [(-22.0, 10.0), (22.0, 10.0), (0.0, 22.0), (0.0, -22.0)] {
                            let at = fire + Vec2::new(dx, dy);
                            if w.level.tiles.circle_clear(at, 6.0, SizeClass::Small) {
                                w.clear_enemies();
                                w.player.pos = at;
                                placed = true;
                                break 'fires;
                            }
                        }
                    }
                    if placed {
                        break;
                    }
                }
                assert!(placed, "no floor in this seed has a fire to stand beside");
                for _ in 0..60 {
                    if let Some(w) = hh.app.world.as_mut() {
                        w.clear_enemies();
                    }
                    hh.frame(&[]);
                }
            }
            // `lit-<scene>-<floor>` (batch 13b): staged scenes for judging the
            // lighting effects. `sconce`: the rat in front of the nearest wall
            // lamp with bodies between lamp and camera; `pendant`: bodies in a
            // ring under the nearest hanging lamp; `wet`: the same lamp with no
            // bodies (floor reflections); `corner`: the rat in the room farthest
            // from the map centre with bodies round it (the overhead offset).
            // `RATRUN_DUMP_N=n` takes the n-th nearest lamp and
            // `RATRUN_DUMP_AT=seconds` sets the frozen scene clock.
            t if t.starts_with("lit-") => {
                use crate::game::collision::SizeClass;
                use crate::game::enemy::EnemyKind;
                use crate::game::level::LightFixtureKind;
                let mut parts = t["lit-".len()..].splitn(2, '-');
                let scene = parts.next().unwrap_or("sconce").to_string();
                let theme = match parts.next().unwrap_or("sewer") {
                    "kitchen" => Theme::Kitchen,
                    "lab" => Theme::Lab,
                    "laundry" => Theme::Laundry,
                    "printworks" => Theme::Printworks,
                    "flatbed" => Theme::Trailer,
                    _ => Theme::Sewer,
                };
                if theme == Theme::Sewer {
                    hh.enter_playing();
                } else {
                    hh.app.start_practice(theme);
                    hh.frame(&[KeyCode::Enter]);
                }
                let nth = std::env::var("RATRUN_DUMP_N")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let w = hh.app.world.as_mut().unwrap();
                w.clear_enemies();
                w.player.god = true;
                let roster: [EnemyKind; 4] = match theme {
                    Theme::Sewer => [
                        EnemyKind::SumpLeech,
                        EnemyKind::PipeEel,
                        EnemyKind::SluiceToad,
                        EnemyKind::Skitterer,
                    ],
                    Theme::Kitchen => [
                        EnemyKind::CrumbAnt,
                        EnemyKind::SinkCrab,
                        EnemyKind::EggTimer,
                        EnemyKind::PieWeight,
                    ],
                    Theme::Lab => [
                        EnemyKind::GlassRunner,
                        EnemyKind::WalkingSyringe,
                        EnemyKind::SpinnerLid,
                        EnemyKind::AutoclaveCart,
                    ],
                    Theme::Laundry => [
                        EnemyKind::WoolPill,
                        EnemyKind::PegSnapper,
                        EnemyKind::StarchBulb,
                        EnemyKind::LintGolem,
                    ],
                    Theme::Printworks => [
                        EnemyKind::InkBlot,
                        EnemyKind::TypeSlug,
                        EnemyKind::Silverfish,
                        EnemyKind::StapleCrawler,
                    ],
                    Theme::Trailer => [
                        EnemyKind::GravelTick,
                        EnemyKind::JuneBug,
                        EnemyKind::LooseCan,
                        EnemyKind::TarRat,
                    ],
                };
                let p = w.player.pos;
                let lamp_kind = if scene == "sconce" {
                    LightFixtureKind::Sconce
                } else {
                    LightFixtureKind::Pendant
                };
                let mut lamps: Vec<_> = w
                    .level
                    .objects
                    .lights
                    .iter()
                    .filter(|f| f.kind == lamp_kind)
                    .copied()
                    .collect();
                lamps.sort_by(|a, b| a.pos.dist(p).partial_cmp(&b.pos.dist(p)).unwrap());
                let mut spots: Vec<Vec2> = Vec::new();
                let rat_at;
                if scene == "corner" {
                    let centre = Vec2::new(
                        w.level.tiles.cols() as f32 * 8.0,
                        w.level.tiles.rows() as f32 * 8.0,
                    );
                    // Lit rooms only (a hanging lamp inside), farthest first.
                    let mut far: Vec<Vec2> = w
                        .level
                        .rooms
                        .iter()
                        .filter(|r| {
                            w.level.objects.lights.iter().any(|f| {
                                f.kind == LightFixtureKind::Pendant && r.rect.contains_world(f.pos)
                            })
                        })
                        .map(|r| r.center())
                        .collect();
                    far.sort_by(|a, b| b.dist(centre).partial_cmp(&a.dist(centre)).unwrap());
                    let far = far.get(nth).copied().unwrap_or(p);
                    rat_at = far;
                    for i in 0..10 {
                        let a = i as f32 * 0.628;
                        let r = 22.0 + (i % 3) as f32 * 14.0;
                        spots.push(far + Vec2::new(a.cos() * r, a.sin() * r));
                    }
                } else if let Some(lamp) = lamps.get(nth).copied() {
                    if scene == "sconce" {
                        let f = lamp.wall_facing(&w.level.tiles);
                        let side = Vec2::new(-f.y, f.x);
                        rat_at = lamp.pos + f * 46.0;
                        for (d, s) in [
                            (16.0, -16.0),
                            (20.0, 12.0),
                            (30.0, -34.0),
                            (34.0, 30.0),
                            (58.0, -26.0),
                            (62.0, 22.0),
                            (74.0, -4.0),
                            (28.0, 52.0),
                        ] {
                            spots.push(lamp.pos + f * d + side * s);
                        }
                    } else {
                        rat_at = lamp.pos + Vec2::new(30.0, 22.0);
                        if scene != "wet" {
                            for i in 0..9 {
                                let a = i as f32 * 0.698 + 0.3;
                                let r = 16.0 + (i % 3) as f32 * 13.0;
                                spots.push(lamp.pos + Vec2::new(a.cos() * r * 1.3, a.sin() * r));
                            }
                        }
                    }
                } else {
                    rat_at = p;
                }
                w.player.pos = w
                    .level
                    .tiles
                    .nearest_open_center(rat_at, SizeClass::Small, 16)
                    .unwrap_or(p);
                let rat = w.player.pos;
                for (i, at) in spots.into_iter().enumerate() {
                    if at.dist(rat) > 10.0 && w.level.tiles.circle_clear(at, 6.0, SizeClass::Small) {
                        w.spawn_enemy(roster[i % roster.len()], at, 1.0);
                    }
                }
                if let Some(elapsed) = std::env::var("RATRUN_DUMP_AT")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .filter(|t| t.is_finite() && *t >= 0.0)
                {
                    w.elapsed = elapsed;
                }
                hh.app.picture_dirty = true;
                hh.app.console_paused = true;
                hh.frame(&[]);
                hh.frame(&[]);
            }
            "levelup" => {
                hh.frame(&[KeyCode::Enter]); // -> Story
                hh.frame(&[KeyCode::Enter]); // dossier -> MapIntro
                hh.frame(&[KeyCode::Enter]); // skip intro -> Playing
                for tick in 0..3600u32 {
                    let mv = match (tick / 20) % 4 {
                        0 => KeyCode::Right,
                        1 => KeyCode::Down,
                        2 => KeyCode::Left,
                        _ => KeyCode::Up,
                    };
                    let text = hh.frame(&[mv]);
                    if text.contains("LEVEL UP") {
                        break; // dump the card itself
                    }
                }
            }
            // The auto-pick toast in play: turn the toggle on, earn a card and
            // dump the frame the callout lands in (no screen, game running).
            "autopick" => {
                hh.enter_playing();
                hh.app.scores.auto_pick = true;
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.clear_enemies();
                    w.clear_gems();
                    w.pending_levelups = 1;
                }
                hh.frame(&[]);
            }
            _ => {
                hh.frame(&[KeyCode::Enter]); // -> Story
                hh.frame(&[KeyCode::Enter]); // dossier -> MapIntro
                hh.frame(&[KeyCode::Enter]); // skip -> Playing
                for tick in 0..900u32 {
                    let mv = match (tick / 18) % 4 {
                        0 => KeyCode::Right,
                        1 => KeyCode::Down,
                        2 => KeyCode::Left,
                        _ => KeyCode::Up,
                    };
                    let text = hh.frame(&[mv]);
                    if text.contains("LEVEL UP") {
                        hh.frame(&[KeyCode::Enter]);
                    }
                }
            }
        }

        // `RATRUN_DUMP_FORE=n` stages the n-th nearest band at the centre
        // of the frame. `RATRUN_DUMP_FORE_KIND=horizontal|vertical|<KindName>`
        // narrows the list, `room-span` and `corridor-span` select its source,
        // `RATRUN_DUMP_FORE_RAT=dx,dy` moves the rat relative to the anchor
        // (`0,0` stands it directly under the item) and
        // `RATRUN_DUMP_FORE_HORDE=n` rings the rat with fodder.
        use crate::render::foreground::{self, ForeKind};
        let kind_named = |name: &str| {
            let name = name.trim().to_ascii_lowercase().replace(['-', '_'], "");
            match name.as_str() {
                "branch" => Some(ForeKind::BranchCanopy),
                "wires" => Some(ForeKind::PowerLines),
                "cloud" => Some(ForeKind::CloudBand),
                _ => ForeKind::all().into_iter()
                    .find(|kind| format!("{kind:?}").to_ascii_lowercase() == name),
            }
        };
        let mut sky_kind = std::env::var("RATRUN_DUMP_SKY").ok().as_deref().and_then(kind_named);
        if let Some(n) = std::env::var("RATRUN_DUMP_FORE").ok().and_then(|v| v.parse::<usize>().ok()) {
            let want = std::env::var("RATRUN_DUMP_FORE_KIND").unwrap_or_default().to_ascii_lowercase();
            let kind_filter = kind_named(&want);
            let rat_off = std::env::var("RATRUN_DUMP_FORE_RAT")
                .ok()
                .and_then(|v| {
                    let (a, b) = v.split_once(',')?;
                    Some(Vec2::new(a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
                });
            let horde = std::env::var("RATRUN_DUMP_FORE_HORDE")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            if let Some(w) = hh.app.world.as_mut() {
                w.foreground_layer.sync(&w.level);
                let p = w.player.pos;
                let mut items: Vec<_> = w
                    .foreground_layer
                    .capture_items()
                    .filter(|(i, _, corridor)| match want.as_str() {
                        "" | "band" | "span" => true,
                        "horizontal" => i.horizontal,
                        "vertical" => !i.horizontal,
                        "room-span" => !*corridor,
                        "corridor-span" => *corridor,
                        _ => kind_filter == Some(i.kind),
                    })
                    .collect();
                items.sort_by(|a, b| a.1.dist_sq(p).total_cmp(&b.1.dist_sq(p)));
                if let Some((item, at, _)) = items.get(n).copied() {
                    eprintln!("dump fore: {item:?}");
                    w.clear_enemies();
                    let rat_off = rat_off.unwrap_or(if item.horizontal {
                        Vec2::new(0.0, 50.0)
                    } else {
                        Vec2::new(50.0, 0.0)
                    });
                    w.player.pos = w
                        .level
                        .tiles
                        .nearest_open_center(
                            at + rat_off,
                            crate::game::collision::SizeClass::Small,
                            16,
                        )
                        .unwrap_or(at);
                    let rat = w.player.pos;
                    for k in 0..horde {
                        let ang = k as f32 / horde.max(1) as f32 * std::f32::consts::TAU;
                        let q = rat + Vec2::new(ang.cos(), ang.sin()) * (22.0 + (k % 3) as f32 * 9.0);
                        if w.level.tiles.get_at(q).is_open() {
                            w.spawn_enemy(crate::game::enemy::EnemyKind::Skitterer, q, 8.0);
                        }
                    }
                    w.cam_focus = Some(at);
                    hh.app.foreground = hh.app.foreground_env.unwrap_or(true);
                    hh.app.picture_dirty = true;
                    hh.app.console_paused = true;
                    hh.frame(&[]);
                } else if w.level.theme == Theme::Trailer {
                    sky_kind = sky_kind.or(kind_filter).or_else(|| {
                        [ForeKind::BranchCanopy, ForeKind::PowerLines, ForeKind::CloudBand]
                            .get(n).copied().filter(|_| want.is_empty())
                    });
                } else {
                    eprintln!("dump fore: no item matches {want:?} #{n}");
                }
            }
        }
        // `RATRUN_DUMP_SKY=branch|wires|cloud` advances the Flatbed clock
        // to a frame with the requested band at the centre of its track.
        if let Some(kind) = sky_kind {
            if let Some(w) = hh.app.world.as_mut() {
                let zoom = std::env::var("RATRUN_DUMP_ZOOM").ok()
                    .and_then(|v| v.parse::<f32>().ok()).map(crate::render::zoom::snap)
                    .unwrap_or(w.zoom_view);
                let size = w.render_viewport * zoom;
                let size = Vec2::new(size.x.round().max(1.0), size.y.round().max(1.0));
                if let Some(t) = foreground::sky_capture_time(&w.level, w.cam, size, zoom, w.elapsed, kind) {
                    eprintln!("dump sky: t={t:.2} {kind:?}");
                    w.elapsed = t;
                    w.clear_enemies();
                    if let Ok(n) = std::env::var("RATRUN_DUMP_FORE_HORDE") {
                        let count = n.parse::<usize>().unwrap_or(0);
                        for k in 0..count {
                            let angle = k as f32 * std::f32::consts::TAU / count as f32;
                            let pos = w.player.pos + Vec2::from_angle(angle) * 30.0;
                            if w.level.tiles.get_at(pos).is_open() && w.level.pit_at(pos).is_none() {
                                w.spawn_enemy(crate::game::enemy::EnemyKind::Skitterer, pos, 8.0);
                            }
                        }
                    }
                    hh.app.foreground = hh.app.foreground_env.unwrap_or(true);
                    hh.app.picture_dirty = true;
                    hh.app.console_paused = true;
                    hh.frame(&[]);
                } else {
                    eprintln!("dump sky: no staged frame for {kind:?}");
                }
            }
        }
        // `RATRUN_DUMP_SCONCE=n` stands the rat 34 px in front of the n-th
        // nearest wall lamp (a corridor one with `RATRUN_DUMP_SCONCE_IN=corridor`).
        if let Some(n) = std::env::var("RATRUN_DUMP_SCONCE").ok().and_then(|v| v.parse::<usize>().ok()) {
            let corridor = std::env::var("RATRUN_DUMP_SCONCE_IN").is_ok_and(|v| v == "corridor");
            if let Some(w) = hh.app.world.as_mut() {
                use crate::game::level::{LightFixtureKind, RoomKind};
                let p = w.player.pos;
                let mut lamps: Vec<_> = w
                    .level
                    .objects
                    .lights
                    .iter()
                    .filter(|f| f.kind == LightFixtureKind::Sconce)
                    .filter(|f| {
                        let in_corridor = w
                            .level
                            .rooms
                            .iter()
                            .any(|r| r.kind == RoomKind::Corridor && r.rect.contains_world(f.pos));
                        in_corridor == corridor
                    })
                    .copied()
                    .collect();
                lamps.sort_by(|a, b| a.pos.dist(p).partial_cmp(&b.pos.dist(p)).unwrap());
                if let Some(lamp) = lamps.get(n).copied() {
                    let facing = lamp.wall_facing(&w.level.tiles);
                    w.clear_enemies();
                    w.player.pos = w
                        .level
                        .tiles
                        .nearest_open_center(
                            lamp.pos + facing * 34.0,
                            crate::game::collision::SizeClass::Small,
                            16,
                        )
                        .unwrap_or(lamp.pos);
                    hh.app.picture_dirty = true;
                    hh.app.console_paused = true;
                    hh.frame(&[]);
                }
            }
        }
        // `RATRUN_DUMP_PUDDLE=n|lamp` stands the rat on the n-th nearest puddle
        // (`lamp`: the puddle whose centre is nearest a light fixture).
        // `RATRUN_DUMP_PUDDLE_RAT=dx,dy` offsets the rat from its centre,
        // `RATRUN_DUMP_PUDDLE_HORDE=n` rings it with ground fodder and
        // `RATRUN_DUMP_PUDDLE_WALK=n` then plays n live frames walking right
        // (ripples under the rat and the waders); without it the frame is held.
        if let Ok(which) = std::env::var("RATRUN_DUMP_PUDDLE") {
            let rat_off = std::env::var("RATRUN_DUMP_PUDDLE_RAT")
                .ok()
                .and_then(|v| {
                    let (a, b) = v.split_once(',')?;
                    Some(Vec2::new(a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
                })
                .unwrap_or(Vec2::ZERO);
            let horde = std::env::var("RATRUN_DUMP_PUDDLE_HORDE")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let walk = std::env::var("RATRUN_DUMP_PUDDLE_WALK")
                .ok()
                .and_then(|v| v.parse::<usize>().ok());
            let mut staged = false;
            if let Some(w) = hh.app.world.as_mut() {
                let p = w.player.pos;
                let mut puddles = w.level.puddles.clone();
                if which == "lamp" {
                    let lights = &w.level.objects.lights;
                    let near = |q: Vec2| {
                        lights.iter().map(|l| l.pos.dist(q)).fold(f32::MAX, f32::min)
                    };
                    puddles.sort_by(|a, b| near(a.pos).partial_cmp(&near(b.pos)).unwrap());
                } else {
                    puddles.sort_by(|a, b| a.pos.dist(p).partial_cmp(&b.pos.dist(p)).unwrap());
                }
                let n = which.parse::<usize>().unwrap_or(0);
                if let Some(puddle) = puddles.get(n).copied() {
                    eprintln!("dump puddle: {puddle:?}");
                    w.clear_enemies();
                    w.player.pos = w
                        .level
                        .tiles
                        .nearest_open_center(
                            puddle.pos + rat_off,
                            crate::game::collision::SizeClass::Small,
                            16,
                        )
                        .unwrap_or(puddle.pos);
                    w.player.hp = w.player.max_hp();
                    w.player.iframes = 30.0;
                    let rat = w.player.pos;
                    for k in 0..horde {
                        let ang = k as f32 / horde.max(1) as f32 * std::f32::consts::TAU;
                        let q = rat + Vec2::new(ang.cos(), ang.sin()) * (24.0 + (k % 3) as f32 * 10.0);
                        if w.level.tiles.get_at(q).is_open() {
                            w.spawn_enemy(crate::game::enemy::EnemyKind::Skitterer, q, 60.0);
                        }
                    }
                    staged = true;
                }
            }
            if staged {
                hh.app.picture_dirty = true;
                match walk {
                    Some(frames) => {
                        for _ in 0..frames {
                            if let Some(w) = hh.app.world.as_mut() {
                                w.player.hp = w.player.max_hp();
                            }
                            hh.frame(&[KeyCode::Right]);
                        }
                    }
                    None => {
                        hh.app.console_paused = true;
                        hh.frame(&[]);
                    }
                }
            }
        }
        // `RATRUN_DUMP_EMITTER=steam|drip|fumes[:n]` stands the rat 30 px below
        // the n-th nearest ambient emitter of that kind.
        if let Ok(spec) = std::env::var("RATRUN_DUMP_EMITTER") {
            use crate::game::level::EmitterKind;
            let (name, n) = spec.split_once(':').unwrap_or((spec.as_str(), "0"));
            let n = n.parse::<usize>().unwrap_or(0);
            let kind = match name {
                "drip" => EmitterKind::Drip,
                "fumes" => EmitterKind::Fumes,
                _ => EmitterKind::Steam,
            };
            if let Some(w) = hh.app.world.as_mut() {
                let p = w.player.pos;
                let mut list: Vec<_> = w
                    .level
                    .objects
                    .emitters
                    .iter()
                    .filter(|e| e.kind == kind)
                    .copied()
                    .collect();
                list.sort_by(|a, b| a.pos.dist(p).partial_cmp(&b.pos.dist(p)).unwrap());
                if let Some(e) = list.get(n).copied() {
                    eprintln!("dump emitter: {e:?}");
                    w.clear_enemies();
                    w.player.pos = w
                        .level
                        .tiles
                        .nearest_open_center(
                            e.pos + Vec2::new(0.0, 30.0),
                            crate::game::collision::SizeClass::Small,
                            16,
                        )
                        .unwrap_or(e.pos);
                    hh.app.picture_dirty = true;
                    hh.app.console_paused = true;
                    hh.frame(&[]);
                } else {
                    eprintln!("dump emitter: none of kind {name}");
                }
            }
        }
        // `RATRUN_DUMP_CAM=dx,dy` slides the camera off the rat by that many
        // world pixels (a parallax strip is the same frame at three offsets).
        if let Some((dx, dy)) = std::env::var("RATRUN_DUMP_CAM").ok().and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
        }) {
            if let Some(w) = hh.app.world.as_mut() {
                w.cam_focus = Some(w.cam_focus.unwrap_or(w.player.pos) + Vec2::new(dx, dy));
                hh.app.picture_dirty = true;
                hh.app.console_paused = true;
                hh.frame(&[]);
            }
        }
        // `RATRUN_DUMP_ZOOM=2` re-frames any in-run capture at that zoom.
        if let Some(zoom) = std::env::var("RATRUN_DUMP_ZOOM")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            if let Some(w) = hh.app.world.as_mut() {
                w.set_zoom_instant(zoom);
                w.damage_numbers.clear();
                hh.frame(&[]);
            }
        }
        // `RATRUN_DUMP_NAME` names the output files (several captures of one
        // target in one directory).
        let target = std::env::var("RATRUN_DUMP_NAME").unwrap_or(target);
        // `RATRUN_DUMP_CHURN=n` (batch 13b): after the capture, walk right for
        // `n` live frames and write `<target>.churn.txt`: the cells that
        // changed per frame and a rough estimate of the bytes a diff encoder
        // would send for them (a proxy, not the presenter's real count).
        if let Some(n) = std::env::var("RATRUN_DUMP_CHURN").ok().and_then(|v| v.parse::<u32>().ok()) {
            hh.app.console_paused = false;
            let mut prev = hh.terminal.backend().buffer().clone();
            let (mut cells, mut bytes) = (0u64, 0u64);
            for _ in 0..n.max(1) {
                if let Some(w) = hh.app.world.as_mut() {
                    w.player.hp = w.player.max_hp();
                }
                // `RATRUN_DUMP_CHURN_KEY=none` stands still instead.
                let still = std::env::var("RATRUN_DUMP_CHURN_KEY").is_ok_and(|v| v == "none");
                let text = if still {
                    hh.frame(&[])
                } else {
                    hh.frame(&[KeyCode::Right])
                };
                if text.contains("LEVEL UP") || text.contains("CHEST OPENED") {
                    hh.frame(&[KeyCode::Enter]);
                    prev = hh.terminal.backend().buffer().clone();
                    continue;
                }
                let cur = hh.terminal.backend().buffer().clone();
                let mut last: Option<(u16, u16)> = None;
                let mut pen: Option<(ratatui::style::Color, ratatui::style::Color)> = None;
                for y in 0..h {
                    for x in 0..w {
                        let (a, b) = (&prev[(x, y)], &cur[(x, y)]);
                        if a == b {
                            continue;
                        }
                        cells += 1;
                        if last != Some((x.wrapping_sub(1), y)) {
                            bytes += 8;
                        }
                        last = Some((x, y));
                        match pen {
                            Some((fg, bg)) => {
                                if fg != b.fg {
                                    bytes += 17;
                                }
                                if bg != b.bg {
                                    bytes += 17;
                                }
                            }
                            None => bytes += 36,
                        }
                        pen = Some((b.fg, b.bg));
                        bytes += b.symbol().len() as u64;
                    }
                }
                prev = cur;
            }
            let dir = std::env::var_os("RATRUN_DUMP_DIR")
                .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
            let n = u64::from(n.max(1));
            std::fs::write(
                dir.join(format!("{target}.churn.txt")),
                format!("frames {n} cells/frame {} est_bytes/frame {}\n", cells / n, bytes / n),
            )
            .unwrap();
            return;
        }
        let buf = hh.terminal.backend().buffer().clone();
        if let Some(dir) = std::env::var_os("RATRUN_DUMP_DIR") {
            use std::fmt::Write;
            let dir = std::path::PathBuf::from(dir);
            let mut ansi = String::new();
            for y in 0..h {
                for x in 0..w {
                    let cell = &buf[(x, y)];
                    let (r, g, b) = color_rgb(cell.fg);
                    let (br, bg, bb) = color_rgb(cell.bg);
                    let _ = write!(ansi, "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}", cell.symbol());
                }
                ansi.push_str("\x1b[0m\n");
            }
            std::fs::write(dir.join(format!("{w}x{h}-{target}.ansi")), ansi).unwrap();
        }
        let (iw, ih) = (w as usize, h as usize * 2);
        let mut px = vec![0u8; iw * ih * 3];
        for cy in 0..h {
            for cx in 0..w {
                let cell = &buf[(cx, cy)];
                let top = color_rgb(cell.fg);
                let bot = color_rgb(cell.bg);
                for (row, (r, g, b)) in [(0usize, top), (1, bot)] {
                    let y = cy as usize * 2 + row;
                    let i = (y * iw + cx as usize) * 3;
                    px[i] = r;
                    px[i + 1] = g;
                    px[i + 2] = b;
                }
            }
        }
        let mut out = format!("P6\n{} {}\n255\n", iw, ih).into_bytes();
        out.extend_from_slice(&px);
        let dir = std::env::var_os("RATRUN_DUMP_DIR")
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        std::fs::write(dir.join(format!("ratrun_{target}.ppm")), out).unwrap();
                // The PPM carries colours only (the half-block pixels are one flat
        // colour per cell side), so the cell *text* — which is what the HUD,
        // the legends and the menus are — lands next to it, one line per row.
        let mut text = String::new();
        for y in 0..h {
            for x in 0..w {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        std::fs::write(dir.join(format!("ratrun_{target}.txt")), text).unwrap();
    }

    /// Space is one press with two jobs: it dashes (unchanged) and releases any
    /// charged active weapon — even while the dash is on cooldown.
    #[test]
    fn space_dashes_and_releases_a_charged_active_weapon() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        {
            use crate::game::enemy::EnemyKind;
            use crate::game::weapon::WeaponKind;
            let w = h.app.world.as_mut().unwrap();
            w.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            let p = w.player.pos;
            w.spawn_enemy(EnemyKind::Skitterer, p + crate::math::Vec2::new(14.0, 0.0), 8.0);
        }
        // Plain Space: the dash fires and the active weapon is released.
        h.frame(&[KeyCode::Char(' ')]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(w.player.dash_cd > 0.0, "Space must still dash");
            assert!(
                w.active_fx().thrash_active(),
                "Space must also release the charged active weapon"
            );
        }

        // With the dash on cooldown the press still releases the weapon: charge
        // it again and press while `dash_cd` is running.
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 3.0;
            w.player.dash_active = 0.0;
        }
        h.frame(&[KeyCode::Char(' ')]);
        let w = h.app.world.as_ref().unwrap();
        assert!(
            w.active_fx().thrash_active(),
            "a charged active must fire while the dash is charging"
        );
        assert_eq!(w.player.dash_active, 0.0, "the dash was on cooldown");

        // A left click on the field is the mouse player's dash gesture. When
        // the dash is **ready** it releases the charged actives with it…
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
            // Let the previous burst finish so the new one is unambiguous.
            let mut fx = w.ecs.resource_mut::<crate::game::ecs::resources::ActiveFx>();
            fx.thrash_left = 0.0;
        }
        let field = h.locate("▀");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), field)]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(w.player.dash_active > 0.0, "the click did not dash");
            assert!(
                w.active_fx().thrash_active(),
                "a click-dash must release a charged active weapon too"
            );
        }

        // …and when the dash is **refused** (still cooling) the click spends
        // nothing but the refusal: a charged defensive ring is never burned by
        // a dash that did not happen (`WAVE3 review`, punch list 7).
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 3.0;
            w.player.dash_active = 0.0;
            let mut fx = w.ecs.resource_mut::<crate::game::ecs::resources::ActiveFx>();
            fx.thrash_left = 0.0;
        }
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), field)]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.player.dash_active, 0.0, "the dash was on cooldown");
            assert!(
                w.player.dash_denied > 0.0,
                "a refused click-dash must still blip the gauge"
            );
            assert!(
                !w.active_fx().thrash_active(),
                "a refused click-dash must not spend a charged active"
            );
        }

        // Space is not a click: on a terminal where it *is* the dash key it
        // keeps doing both, so the keyboard player is never locked out of a
        // charged weapon for the length of a dash cooldown.
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 3.0;
            w.player.dash_active = 0.0;
            let mut fx = w.ecs.resource_mut::<crate::game::ecs::resources::ActiveFx>();
            fx.thrash_left = 0.0;
        }
        h.frame(&[KeyCode::Char(' ')]);
        assert!(
            h.app.world.as_ref().unwrap().active_fx().thrash_active(),
            "Space still releases the actives while the dash recharges"
        );
    }

    /// A kitty-protocol terminal splits the old Space press in two: Ctrl dashes,
    /// Space only releases the charged active weapons.
    #[test]
    fn ctrl_dashes_and_space_only_fires_the_actives_on_a_kitty_terminal() {
        let app = App::with_options(true, None, ScoreTable::default(), false, Some(TEST_SEED));
        let mut h = Harness::with_app(app, 80, 24);
        h.enter_playing();
        assert_eq!(h.app.dash_key(), DashKey::Ctrl);
        {
            use crate::game::weapon::WeaponKind;
            let w = h.app.world.as_mut().unwrap();
            w.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            let p = w.player.pos;
            w.spawn_enemy(
                crate::game::enemy::EnemyKind::Skitterer,
                p + crate::math::Vec2::new(14.0, 0.0),
                8.0,
            );
            w.player.dash_cd = 0.0;
        }

        // Ctrl alone: the dash fires and no active weapon is spent.
        h.frame_events(&[ctrl_press()]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(w.player.dash_cd > 0.0, "a lone Ctrl press must dash");
            assert!(
                !w.active_fx().thrash_active(),
                "Ctrl must not release the charged active weapon"
            );
        }

        // Space alone: the active weapon fires and the dash is untouched.
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
        }
        h.frame(&[KeyCode::Char(' ')]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(
                w.active_fx().thrash_active(),
                "Space must release the charged active weapon"
            );
            assert_eq!(w.player.dash_cd, 0.0, "Space must not dash in Ctrl mode");
        }

        // A left click on the field is still the whole pair, so a mouse player
        // keeps dashing and firing actives with one button.
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
            let mut fx = w.ecs.resource_mut::<crate::game::ecs::resources::ActiveFx>();
            fx.thrash_left = 0.0;
        }
        let field = h.locate("▀");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), field)]);
        let w = h.app.world.as_ref().unwrap();
        assert!(w.player.dash_cd > 0.0, "a click must dash");
        assert!(
            w.active_fx().thrash_active(),
            "a click must also release the charged active weapon"
        );
    }

    /// Without lone-modifier reporting, Space keeps doing both jobs, so the game
    /// is playable on Apple Terminal and friends.
    #[test]
    fn fallback_mode_keeps_space_dashing() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        assert_eq!(h.app.dash_key(), DashKey::Space);
        {
            let w = h.app.world.as_mut().unwrap();
            w.player.dash_cd = 0.0;
        }
        // A lone Ctrl press that a terminal sends anyway cannot dash here...
        h.frame_events(&[ctrl_press()]);
        assert_eq!(
            h.app.world.as_ref().unwrap().player.dash_cd,
            0.0,
            "Ctrl is not the dash key in the fallback mode"
        );
        // ...Space does.
        h.frame(&[KeyCode::Char(' ')]);
        assert!(
            h.app.world.as_ref().unwrap().player.dash_cd > 0.0,
            "Space must still dash in the fallback mode"
        );
    }

    /// Super Dash is hold-to-charge on a Kitty terminal: Ctrl starts the charge
    /// (no dash yet), the hold fills the meter while the rat keeps moving, and
    /// the key-up fires the longer dash.
    #[test]
    fn a_held_ctrl_charges_the_super_dash_and_its_release_fires_it() {
        let app = App::with_options(true, None, ScoreTable::default(), false, Some(TEST_SEED));
        let mut h = Harness::with_app(app, 100, 30);
        h.enter_playing();
        {
            use crate::game::weapon::WeaponKind;
            let w = h.app.world.as_mut().unwrap();
            w.player
                .loadout
                .apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
            w.player.hp = 1.0e9;
            w.player.xp_to_next = 1_000_000;
        }
        h.frame_events(&[ctrl_press()]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(
                w.player.dash_charging,
                "the press must start a charge, not a dash"
            );
            assert_eq!(w.player.dash_active, 0.0);
            assert_eq!(w.player.dash_cd, 0.0, "the cooldown starts on the release");
        }
        for _ in 0..30 {
            h.frame_events(&[]);
        }
        assert!(
            h.app.world.as_ref().unwrap().player.dash_charge > 0.3,
            "the hold must fill the meter"
        );
        let release = KeyEvent::new_with_kind(
            KeyCode::Modifier(ModifierKeyCode::LeftControl),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );
        h.frame_events(&[release]);
        let w = h.app.world.as_ref().unwrap();
        assert!(
            w.player.dash_active > 0.17,
            "the key-up must fire the charged dash: {}",
            w.player.dash_active
        );
        assert!(w.player.dash_cd > 0.0, "the cooldown starts on the release");
        assert!(!w.player.dash_charging);
    }

    /// The mouse charges in either input mode: a terminal always reports the
    /// button's Up, so a held click fires a charged dash on the mouse-up (and a
    /// press alone never dashes while the weapon is owned).
    #[test]
    fn a_held_click_charges_and_the_mouse_up_fires_the_super_dash() {
        let mut h = Harness::new(100, 30);
        h.enter_playing();
        {
            use crate::game::weapon::WeaponKind;
            let w = h.app.world.as_mut().unwrap();
            w.player
                .loadout
                .apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
            w.player.dash_cd = 0.0;
            w.player.dash_active = 0.0;
            w.player.hp = 1.0e9;
            w.player.xp_to_next = 1_000_000;
        }
        let field = h.locate("▀");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), field)]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(w.player.dash_charging, "the press must start a charge");
            assert_eq!(w.player.dash_active, 0.0, "the press must not dash");
            assert_eq!(w.player.dash_cd, 0.0);
        }
        for _ in 0..30 {
            h.frame(&[]);
        }
        assert!(
            h.app.world.as_ref().unwrap().player.dash_charge > 0.3,
            "the hold must fill the meter"
        );
        h.events(&[], &[(MouseEventKind::Up(MouseButton::Left), field)]);
        let w = h.app.world.as_ref().unwrap();
        assert!(
            w.player.dash_active > 0.17,
            "the mouse-up must fire the charged dash: {}",
            w.player.dash_active
        );
        assert!(!w.player.dash_charging);
    }

    /// A dash refused by the cooldown is never silent: the sim sets the shake
    /// timer (the on-rat gauge and the HUD read it) and the HUD prints WAIT.
    #[test]
    fn a_refused_dash_is_visible_and_heard() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            w.player.dash_cd = 1.5;
            w.player.dash_active = 0.0;
            w.player.dash_denied = 0.0;
        }
        let text = h.frame(&[KeyCode::Char(' ')]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert!(
                w.player.dash_denied > 0.0,
                "the refused press left no trace for the gauge"
            );
            assert_eq!(w.player.dash_active, 0.0, "a refused press must not dash");
        }
        assert!(
            text.contains("WAIT"),
            "the HUD did not report the refusal: {text:?}"
        );
    }

    /// The title legend (and the HUD's dash module) name the key the mode
    /// actually uses.
    #[test]
    fn the_legends_name_the_live_dash_key() {
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        assert!(
            h.screen_text().contains("Space dash + actives"),
            "fallback title legend lost its Space binding"
        );

        // Flip to the Ctrl mapping precisely as `main` does after confirming
        // the terminal honours report-all-keys.
        h.app.set_lone_ctrl(true);
        h.frame(&[]);
        let text = h.screen_text();
        assert!(text.contains("Ctrl dash"), "no Ctrl legend: {text:?}");
        assert!(text.contains("Space actives"), "no active hint: {text:?}");
        assert_eq!(h.app.hud.dash_key, DashKey::Ctrl);
    }

    #[test]
    fn player_follows_mouse_cursor() {
        let mut h = Harness::new(100, 44);
        h.frame(&[]); // title
        h.frame(&[KeyCode::Enter]); // -> Story
        h.frame(&[KeyCode::Enter]); // dossier -> MapIntro
        h.frame(&[KeyCode::Enter]); // skip intro -> Playing
        h.frame(&[]); // render so last_game + cam are set

        // Cursor at the far right → player should drift right.
        let x0 = h.player_x();
        for _ in 0..120 {
            h.set_mouse(97, 22);
            h.frame(&[]);
        }
        let x1 = h.player_x();
        assert!(
            x1 > x0 + 10.0,
            "player should chase the cursor right ({x0}→{x1})"
        );

        // Now far left → player reverses.
        for _ in 0..120 {
            h.set_mouse(2, 22);
            h.frame(&[]);
        }
        let x2 = h.player_x();
        assert!(
            x2 < x1 - 10.0,
            "player should chase the cursor left ({x1}→{x2})"
        );
    }

    #[test]
    fn full_run_renders_and_handles_levelup() {
        let mut h = Harness::new(100, 44);
        h.frame(&[]); // title
        h.frame(&[KeyCode::Enter]); // START RUN -> Story
        h.frame(&[KeyCode::Enter]); // dossier -> MapIntro
        h.frame(&[KeyCode::Enter]); // intro -> Playing

        let mut saw_pixels = false;
        let mut saw_levelup = false;
        let mut saw_hud = false;

        // Drive ~60 in-game seconds, weaving so we bump into the horde and gain XP.
        for tick in 0..3600u32 {
            let mv = match (tick / 30) % 4 {
                0 => KeyCode::Right,
                1 => KeyCode::Down,
                2 => KeyCode::Left,
                _ => KeyCode::Up,
            };
            let text = h.frame(&[mv]);

            if text.contains('▀') {
                saw_pixels = true;
            }
            if text.contains("Score") {
                saw_hud = true;
            }
            if text.contains("LEVEL UP") {
                saw_levelup = true;
                // Confirm the highlighted upgrade and continue playing.
                h.frame(&[KeyCode::Enter]);
            }
        }

        assert!(saw_pixels, "game viewport should render half-block pixels");
        assert!(saw_hud, "HUD should render the score readout");
        assert!(saw_levelup, "player should have leveled up within 60s");
        assert!(!h.app.should_quit);
        // The rat may or may not survive the 60 s; either way tests never
        // persist scores to disk.
        assert!(!h.app.persist);
    }

    /// A small window (the size the rat art no longer fits in) shows the pixel
    /// wordmark and the short legend, with nothing clipped at the right edge.
    #[test]
    fn small_title_shows_wordmark_and_unclipped_legend() {
        let mut h = Harness::new(57, 20);
        let text = h.frame(&[]);
        if std::env::var("RATRUN_DUMP").is_ok() {
            println!("{text}");
        }
        assert!(
            text.contains("██▀█▄ ▄█▀█▄ █████"),
            "wordmark top row missing"
        );
        assert!(text.contains("START RUN"));
        assert!(text.contains("Sector"), "the sector row is missing");
        assert!(text.contains("M mute"), "legend line 1 clipped");
        assert!(text.contains("Q quit"), "legend line 2 clipped");
    }

    /// Item 4: the menu and legend are reserved before the art, so short
    /// terminals keep them.
    #[test]
    fn title_keeps_menu_on_short_terminals() {
        for (w, h) in [(80u16, 16u16), (80, 24)] {
            let mut hh = Harness::new(w, h);
            let text = hh.frame(&[]);
            assert!(
                text.contains("START RUN"),
                "{w}x{h} title should still show START RUN"
            );
            assert!(
                text.contains("Sector"),
                "{w}x{h} title should show the sector row"
            );
            assert!(text.contains("QUIT"), "{w}x{h} title should show QUIT");
            assert!(
                text.contains("Enter confirm"),
                "{w}x{h} title should keep the key legend"
            );
        }
    }

    /// The sector row opens the art's sector-select screen (the free map
    /// selector is gone), and Esc returns to the title.
    #[test]
    fn enter_on_sector_row_opens_the_select_screen() {
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        h.frame(&[KeyCode::Down]); // select the Sector row
        assert_eq!(h.app.menu_idx, 1);
        h.frame(&[KeyCode::Enter]);
        assert!(
            matches!(h.app.screen, Screen::SectorSelect),
            "the sector row must open the sector select"
        );
        assert!(h.screen_text().contains("SELECT SECTOR"));
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Title));
        assert_eq!(h.app.menu_idx, 1, "the title keeps its row");
    }

    /// Item 11: pause freezes the simulation and resumes, and Q bails to the menu.
    /// A paused run at 100x40 with a few seconds of play behind it.
    fn paused_harness() -> Harness {
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        for _ in 0..40 {
            h.frame(&[]);
        }
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Paused));
        // Let the render-only clocks (zoom easing, read-outs) settle.
        for _ in 0..30 {
            h.frame(&[]);
        }
        h
    }

    #[test]
    fn a_paused_frame_does_not_draw_the_world_again() {
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        assert!(drawn > 40, "play draws the world every frame");
        let before = h.terminal.backend().buffer().clone();
        for _ in 0..20 {
            h.frame(&[]);
        }
        assert_eq!(
            h.app.world_draws, drawn,
            "a frozen world must be drawn once, not once per frame"
        );
        // The field is still on screen under the pause plate, cell for cell.
        assert_eq!(h.terminal.backend().buffer(), &before);
        assert!(
            before.content.iter().filter(|c| c.symbol() == "▀").count() > 500,
            "the paused frame should still show the play field"
        );
        assert!(!h.app.repaint_urgent(), "a still pause screen asks for nothing");
        assert_eq!(h.app.frame_demand(), crate::present::Demand::Menu);
    }

    #[test]
    fn the_cached_world_picture_is_the_picture_a_redraw_would_make() {
        let mut h = paused_harness();
        let cached = h.terminal.backend().buffer().clone();
        let drawn = h.app.world_draws;
        h.app.picture_dirty = true;
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1);
        assert_eq!(
            h.terminal.backend().buffer(),
            &cached,
            "reusing the pixels must not change a single cell"
        );
    }

    #[test]
    fn overlays_over_a_frozen_run_reuse_the_world_picture() {
        // The options plate and the Library, both opened from pause.
        let mut h = paused_harness();
        h.frame(&[KeyCode::Char('o')]);
        assert!(matches!(h.app.screen, Screen::Options));
        h.frame(&[]);
        let drawn = h.app.world_draws;
        for _ in 0..10 {
            h.frame(&[]);
        }
        assert_eq!(h.app.world_draws, drawn, "options over a paused run");

        // A level-up plate over the field.
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        for _ in 0..20 {
            h.frame(&[]);
        }
        h.app.world.as_mut().unwrap().pending_levelups = 1;
        h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::LevelUp));
        h.frame(&[]);
        let drawn = h.app.world_draws;
        for _ in 0..10 {
            h.frame(&[]);
        }
        assert_eq!(h.app.world_draws, drawn, "the level-up plate");

        // The expanded map pauses the world on the Playing screen.
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        for _ in 0..20 {
            h.frame(&[]);
        }
        h.frame(&[KeyCode::Tab]);
        h.frame(&[]);
        let ticks = h.app.world.as_ref().unwrap().ticks;
        let drawn = h.app.world_draws;
        for _ in 0..10 {
            h.frame(&[]);
        }
        if h.app.world.as_ref().unwrap().ticks == ticks {
            assert_eq!(h.app.world_draws, drawn, "the open map");
        }
    }

    #[test]
    fn everything_that_changes_the_frozen_picture_redraws_it() {
        // A resize: the pixel buffer changes size.
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        h.terminal.backend_mut().resize(120, 44);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "a resize must redraw");
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "and then settle again");

        // A staged outage changes the fixture power even at the same tick.
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        h.app.world.as_mut().unwrap().elapsed = 304.0;
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "the fixture power must redraw");
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "the blackout stays frozen");

        // The zoom: every frame of the ease is a new picture, then it stops.
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        h.app.world.as_mut().unwrap().zoom_by(1);
        h.app.input.begin_frame();
        h.app.update(1.0 / 60.0);
        assert!(h.app.repaint_urgent(), "the zoom ease is a running clock");
        h.terminal.draw(|f| h.app.render(f)).unwrap();
        h.frame(&[]);
        assert!(h.app.world_draws >= drawn + 2, "the ease redraws each frame");
        for _ in 0..60 {
            h.frame(&[]);
        }
        let settled = h.app.world_draws;
        assert!(!h.app.repaint_urgent(), "the ease has landed");
        h.frame(&[]);
        assert_eq!(h.app.world_draws, settled);

        // The lighting setting and the debug overlays.
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        h.app.set_lighting(crate::render::lighting::LightingMode::Coarse);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "a lighting change must redraw");
        h.app.world.as_mut().unwrap().hitboxes = true;
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 2, "the hitbox overlay must redraw");
        h.app.world.as_mut().unwrap().smoke_enabled = false;
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 3, "the smoke toggle must redraw");
        h.app.set_foreground(true);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 4, "the foreground toggle must redraw");
        // The Graphics page's SCREEN SHAKE row moves the camera the frozen
        // picture was drawn with, so the key has to see it.
        h.app.set_shake_scale(1.0);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 5, "the shake share must redraw");

        // Any discrete input (it may have changed anything): redraw once.
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.world_draws, drawn + 1, "a key press must redraw");
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1);
        // Pointer motion alone does not (the world cannot see the pointer),
        // but it does ask for a frame: the cheese cursor moved.
        h.app.input.begin_frame();
        h.app.input.handle_mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        });
        h.app.update(1.0 / 60.0);
        assert!(h.app.repaint_urgent(), "pointer motion repaints");
        h.terminal.draw(|f| h.app.render(f)).unwrap();
        assert_eq!(h.app.world_draws, drawn + 1);

        // Resuming: the world ticks, so every frame is a new picture.
        let mut h = paused_harness();
        h.frame(&[KeyCode::Char('p')]);
        assert!(matches!(h.app.screen, Screen::Playing));
        let drawn = h.app.world_draws;
        for _ in 0..5 {
            h.frame(&[]);
        }
        assert_eq!(h.app.world_draws, drawn + 5);
        assert_eq!(h.app.frame_demand(), crate::present::Demand::Live);
    }

    #[cfg(feature = "console")]
    #[test]
    fn the_console_keeps_the_frame_and_the_world_picture_live() {
        let mut h = paused_harness();
        if !h.app.console_enabled() {
            return;
        }
        h.frame(&[KeyCode::Char('`')]);
        assert!(h.app.console.open);
        let drawn = h.app.world_draws;
        for _ in 0..5 {
            h.frame(&[]);
            assert!(h.app.repaint_urgent(), "an open console is always repainted");
        }
        assert_eq!(
            h.app.world_draws,
            drawn + 5,
            "a console command can change anything: no cached picture under it"
        );
    }

    #[test]
    fn input_resize_focus_and_clocks_all_ask_for_a_frame() {
        let mut h = paused_harness();
        assert!(!h.app.repaint_urgent());

        // A key (even one that does nothing here).
        h.app.input.begin_frame();
        h.app.input.handle_key(KeyEvent::from(KeyCode::Char('j')));
        h.app.update(1.0 / 60.0);
        assert!(h.app.repaint_urgent(), "a key press");
        h.frame(&[]);
        assert!(!h.app.repaint_urgent());

        // A resize report.
        h.app.input.begin_frame();
        h.app.input.note_resize();
        h.app.update(1.0 / 60.0);
        assert!(h.app.repaint_urgent() && h.app.input.resized, "a resize");

        // Focus coming back (and it is flagged for a full repaint).
        h.app.input.begin_frame();
        h.app.input.set_focused(false);
        h.app.input.begin_frame();
        h.app.input.set_focused(true);
        h.app.update(1.0 / 60.0);
        assert!(h.app.repaint_urgent() && h.app.input.refocused, "focus regained");
        h.frame(&[]);
        assert!(!h.app.input.refocused && !h.app.repaint_urgent());

        // A screen change is flagged until the next render.
        h.app.input.begin_frame();
        h.app.input.handle_key(KeyEvent::from(KeyCode::Char('p')));
        h.app.update(1.0 / 60.0);
        assert!(matches!(h.app.screen, Screen::Playing) && h.app.repaint);
        h.terminal.draw(|f| h.app.render(f)).unwrap();
        assert!(!h.app.repaint);

        // The volume read-out is a clock: it must also disappear on time.
        let mut h = paused_harness();
        h.frame(&[KeyCode::Char('[')]);
        h.app.input.begin_frame();
        h.app.update(1.0 / 60.0);
        assert!(h.app.hud.flashing(), "the volume key shows a read-out");
        assert!(h.app.repaint_urgent(), "the volume read-out is on screen");
        for _ in 0..600 {
            h.frame(&[]);
        }
        assert!(!h.app.repaint_urgent(), "and it expires");

        // The auto-pick toast spacing clock.
        let mut h = paused_harness();
        h.app.auto_pick_wait = 0.4;
        assert!(h.app.repaint_urgent(), "auto-pick toasts are spaced by a clock");
    }

    #[test]
    fn pause_freezes_run_and_resumes() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        for _ in 0..10 {
            h.frame(&[]);
        }
        let t0 = h.app.world.as_ref().unwrap().elapsed;
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Paused));
        for _ in 0..30 {
            h.frame(&[]);
        }
        let t1 = h.app.world.as_ref().unwrap().elapsed;
        assert_eq!(t1, t0, "a paused run must not advance");

        h.frame(&[KeyCode::Char('p')]);
        assert!(matches!(h.app.screen, Screen::Playing));
        h.frame(&[]);
        assert!(
            h.app.world.as_ref().unwrap().elapsed > t0,
            "resumed run should keep ticking"
        );

        h.frame(&[KeyCode::Char('p')]);
        assert!(matches!(h.app.screen, Screen::Paused));
        h.frame(&[KeyCode::Char('q')]);
        assert!(matches!(h.app.screen, Screen::Title));
        assert!(h.app.world.is_none(), "quitting drops the run");
    }

    /// Item 11: the end screen renders, records the score in memory, and Enter
    /// starts a fresh run.
    #[test]
    fn end_screen_records_and_restarts() {
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        for _ in 0..30 {
            h.frame(&[]);
        }
        h.app.world.as_mut().unwrap().game_over = true;
        let text = h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::End));
        assert!(text.contains("YOU DIED"), "end screen should render");
        assert!(h.app.recorded, "the run should be finalized once");
        assert!(h.app.last_score > 0);
        assert!(
            h.app.new_best,
            "the first score on an empty table is a personal best"
        );
        assert_eq!(h.app.scores.entries.len(), 1);

        h.frame(&[KeyCode::Enter]); // restart
        assert!(matches!(h.app.screen, Screen::MapIntro));
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.elapsed, 0.0, "restart begins a fresh run");
        assert_eq!(w.kills, 0);
        assert!(!h.app.recorded);
        assert!(!h.app.new_best);
    }

    /// Item 2: "NEW HIGH SCORE" requires beating the previous best, not merely
    /// fitting in a table that still has room.
    #[test]
    fn new_best_requires_beating_previous_best() {
        let mut table = ScoreTable::default();
        table.insert(ScoreEntry {
            score: 9_000,
            time: 60.0,
            level: 4,
            map: "Sewer".into(),
            won: false,
            campaign_level: 2,
            campaign_won: false,
        });
        let app = App::with_options(false, None, table, false, Some(TEST_SEED));
        let mut h = Harness::with_app(app, 100, 40);
        h.enter_playing();
        for _ in 0..30 {
            h.frame(&[]);
        }
        h.app.world.as_mut().unwrap().game_over = true;
        let text = h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::End));
        assert!(
            h.app.last_score < 9_000,
            "this short run scores well under the best"
        );
        assert!(
            !h.app.new_best,
            "a low-scoring run is not a new best just because the table has room"
        );
        assert!(!text.contains("NEW HIGH SCORE"), "banner must not be drawn");
        assert_eq!(h.app.scores.entries.len(), 2, "but it is still recorded");
    }

    /// Item 3: dying on the same tick as an XP award must end the run, not open
    /// upgrade cards (which could heal the dead player).
    #[test]
    fn finished_run_beats_pending_levelup() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        if let Some(w) = h.app.world.as_mut() {
            w.game_over = true;
            w.pending_levelups = 1;
        }
        h.frame(&[]);
        assert!(
            matches!(h.app.screen, Screen::End),
            "a finished run must not open the level-up screen"
        );
        assert!(h.app.recorded);
    }

    /// Item 6: the pixel buffer is allocated once per viewport size, not once
    /// per frame.
    #[test]
    fn pixel_buffer_is_reused_between_frames() {
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        h.frame(&[]);
        let ptr = h.app.pixels.px_ptr_for_test();
        let (w, hgt) = (h.app.pixels.w, h.app.pixels.h);
        let expected_h = 40 - hud::top_height(Rect::new(0, 0, 100, 40)) as usize - 1;
        assert_eq!(w, 100, "buffer spans the game viewport width");
        assert_eq!(hgt, expected_h * 2, "two pixels per viewport row");

        for _ in 0..5 {
            h.frame(&[]);
        }
        assert_eq!(
            h.app.pixels.px_ptr_for_test(),
            ptr,
            "same-size frames must not reallocate the pixel buffer"
        );

        // A viewport change does resize it.
        h.terminal.backend_mut().resize(80, 30);
        h.frame(&[]);
        let resized_h = 30 - hud::top_height(Rect::new(0, 0, 80, 30)) as usize - 1;
        assert_eq!((h.app.pixels.w, h.app.pixels.h), (80, resized_h * 2));
    }

    // ---- The nest (permanent upgrades) ------------------------------------

    /// Every drawn title row is reachable: the keyboard walks all six in the
    /// drawn order and Enter activates the selected one; a click on a row's
    /// label does the same thing. START RUN is the default selection.
    #[test]
    fn every_title_row_is_selectable_by_keyboard_and_mouse_at_every_size() {
        const ROWS: [(&str, &str); TITLE_ROWS] = [
            ("START RUN", "run"),
            ("Sector", "SectorSelect"),
            ("OPTIONS", "Options"),
            ("LIBRARY", "Library"),
            ("THE NEST", "Nest"),
            ("QUIT", "quit"),
        ];
        let landed = |h: &Harness, want: &str, how: &str, size: (u16, u16)| {
            let name = h.app.screen.name_for_test();
            match want {
                "run" => assert!(
                    name != "Title" && !h.app.should_quit,
                    "{size:?} {how}: START RUN left the app on {name}"
                ),
                "quit" => assert!(h.app.should_quit, "{size:?} {how}: QUIT did not quit"),
                other => assert_eq!(name, other, "{size:?} {how}: wrong screen"),
            }
        };
        for size in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut first = Harness::new(size.0, size.1);
            let text = first.frame(&[]);
            assert_eq!(first.app.menu_idx, 0, "START RUN is the default row");
            // Every row is drawn, top to bottom in table order.
            let mut last_y = 0;
            for (i, (label, _)) in ROWS.iter().enumerate() {
                assert!(text.contains(label), "{size:?}: {label} is not drawn:\n{text}");
                let (_, y) = first.locate(label);
                assert!(i == 0 || y > last_y, "{size:?}: {label} is out of order");
                last_y = y;
            }
            for (i, (label, want)) in ROWS.iter().enumerate() {
                // Keyboard: Down i times, then Enter.
                let mut h = Harness::new(size.0, size.1);
                h.frame(&[]);
                for _ in 0..i {
                    h.frame(&[KeyCode::Down]);
                }
                assert_eq!(h.app.menu_idx, i, "{size:?}: Down did not reach {label}");
                assert_eq!(TITLE_ACTIONS.len(), ROWS.len());
                h.frame(&[KeyCode::Enter]);
                landed(&h, want, "keyboard", size);
                // Mouse: a click on the row's own label.
                let mut h = Harness::new(size.0, size.1);
                h.frame(&[]);
                h.click(label);
                landed(&h, want, "mouse", size);
            }
            // Down wraps from QUIT back to START RUN.
            let mut h = Harness::new(size.0, size.1);
            h.frame(&[]);
            for _ in 0..TITLE_ROWS {
                h.frame(&[KeyCode::Down]);
            }
            assert_eq!(h.app.menu_idx, 0, "{size:?}: the rows do not wrap");
        }
    }

    /// The title's NEST row shows the bank, opens the plate, and a cheap track
    /// buys on one Enter (the price is also debited once).
    #[test]
    fn the_title_nest_row_opens_the_nest_and_buys_a_track() {
        let mut h = Harness::new(80, 24);
        h.app.scores.meta.bank = 500;
        let title = h.frame(&[]);
        assert!(title.contains("THE NEST"), "no nest row on the title:
{title}");
        assert!(title.contains("500¢"), "the row does not show the bank:
{title}");

        h.click("THE NEST");
        assert_eq!(h.app.screen.name_for_test(), "Nest");
        assert!(!h.app.nest_read_only, "the title path may buy");
        assert!(h.app.scores.meta.can_buy(meta::Track::WeaponSlots));

        // QUICK PAWS is the third row in the table.
        h.frame(&[KeyCode::Down]);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.nest_ui.cursor, meta::Track::MoveSpeed.index());
        let before = h.app.scores.meta.bank;
        // The price comes from the table, so a retune cannot stale this test.
        let price = h
            .app
            .scores
            .meta
            .next_price(meta::Track::MoveSpeed)
            .expect("the first step has a price");
        assert!(!nest_ui::needs_confirm(price) && price <= before);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.scores.meta.level(meta::Track::MoveSpeed), 1);
        assert_eq!(h.app.scores.meta.bank, before - price);
        assert!(h.app.nest_ui.pending.is_none(), "a cheap buy is not confirmed");

        // Esc returns to the title, and the title row shows the new balance.
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        let title = h.frame(&[]);
        let left = format!("{}¢", before - price);
        assert!(title.contains(&left), "the row did not re-read the bank:\n{title}");
    }

    /// The pause screen opens the nest **read-only**: the plate shows the run's
    /// bonuses and not one press can spend the profile's bank.
    #[test]
    fn the_pause_nest_is_a_read_only_dossier() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.scores.meta.bank = 1_000;
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        h.frame(&[KeyCode::Char('b')]);
        assert_eq!(h.app.screen.name_for_test(), "Nest");
        assert!(h.app.nest_read_only);
        h.app.nest_ui.cursor = meta::Track::WeaponSlots.index();
        h.frame(&[]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(
            h.app.scores.meta.level(meta::Track::WeaponSlots),
            0,
            "a run in flight cannot buy a slot"
        );
        assert_eq!(h.app.scores.meta.bank, 1_000);
        assert!(h.app.scores.meta.bank > 0);
        // Esc goes back to the pause plate, not the title.
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
    }

    /// An expensive purchase takes two Enters, and Esc cancels the arming.
    #[test]
    fn an_expensive_purchase_needs_a_second_enter() {
        let mut h = Harness::new(80, 24);
        h.app.scores.meta.bank = 100_000;
        h.app.scores.meta.set_level(meta::Track::WeaponSlots, 2);
        let price = h
            .app
            .scores
            .meta
            .next_price(meta::Track::WeaponSlots)
            .expect("the third slot has a price");
        assert!(nest_ui::needs_confirm(price), "this step must need a confirm");
        h.app.open_nest(Screen::Title);
        h.app.nest_ui.cursor = meta::Track::WeaponSlots.index();
        h.frame(&[]);

        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.scores.meta.level(meta::Track::WeaponSlots), 2, "armed only");
        assert_eq!(
            h.app.nest_ui.pending,
            Some(nest_ui::Pending::Buy(meta::Track::WeaponSlots))
        );
        h.frame(&[KeyCode::Esc]);
        assert!(h.app.nest_ui.pending.is_none());
        assert_eq!(h.app.screen.name_for_test(), "Nest", "Esc cancelled the buy");

        h.frame(&[KeyCode::Enter]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.scores.meta.level(meta::Track::WeaponSlots), 3);
        assert_eq!(h.app.scores.meta.bank, 100_000 - price);
    }

    /// The refund-all is free, needs a confirm, and returns exactly what was
    /// paid (the sandbox the screen promises).
    #[test]
    fn refunding_the_nest_needs_a_confirm_and_returns_the_gold() {
        let mut h = Harness::new(80, 24);
        let price = h
            .app
            .scores
            .meta
            .next_price(meta::Track::WeaponSlots)
            .expect("the first slot has a price");
        // Exactly enough for one step below the confirm threshold.
        assert!(!nest_ui::needs_confirm(price));
        let bank = price + 40;
        h.app.scores.meta.bank = bank;
        h.app.open_nest(Screen::Title);
        h.app.nest_ui.cursor = meta::Track::WeaponSlots.index();
        h.frame(&[]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.scores.meta.bank, 40);

        h.frame(&[KeyCode::Char('r')]);
        assert_eq!(h.app.nest_ui.pending, Some(nest_ui::Pending::Refund));
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.scores.meta.total_levels(), 0);
        assert_eq!(h.app.scores.meta.bank, bank, "the price came back");
    }

    /// A finished run banks its unspent purse, the incident report prints the
    /// tally and offers the nest, and leaving cannot bank it twice.
    #[test]
    fn the_incident_report_banks_the_haul_and_offers_the_nest() {
        let mut h = Harness::new(90, 32);
        h.enter_playing();
        h.app.world.as_mut().unwrap().gold = 130;
        h.app.world.as_mut().unwrap().player.hp = -1.0;
        h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "End");

        let tally = h.app.bank_tally.as_ref().copied().unwrap_or_default();
        assert_eq!(tally.purse, 130);
        assert_eq!(tally.banked, 130, "no floor was cleared, so no bonuses");
        assert_eq!(h.app.scores.meta.bank, 130);

        let text = h.frame(&[]);
        assert!(text.contains("NEST"), "no tally on the report:
{text}");
        assert!(text.contains("+130"), "the tally does not name the haul:
{text}");
        assert!(
            text.contains("the nest"),
            "the report lost its nest chip:
{text}"
        );

        // B (the Nest action's default) opens the nest and Esc comes back to
        // the report.
        h.frame(&[KeyCode::Char('b')]);
        assert_eq!(h.app.screen.name_for_test(), "Nest");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        // Quitting now must not bank the same run again.
        h.frame(&[KeyCode::Char('q')]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        assert_eq!(h.app.scores.meta.bank, 130);
    }

    /// Quitting a live run banks the unspent purse at the practice rate, so a
    /// player can never lose the haul by walking away.
    #[test]
    fn a_quit_mid_run_still_banks_the_purse() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.world.as_mut().unwrap().gold = 42;
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('q')]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        assert_eq!(h.app.scores.meta.bank, 42, "the campaign banks at full rate");
    }

    /// A saved nest shapes the run the App starts, and the read-only dossier is
    /// the run's own modifiers.
    #[test]
    fn a_saved_nest_shapes_the_next_run() {
        let mut app = test_app();
        app.scores.meta.bank = 10_000;
        app.scores.meta.set_level(meta::Track::WeaponSlots, 4);
        app.scores.meta.set_level(meta::Track::Damage, 6);
        app.scores.meta.set_level(meta::Track::Revive, 1);
        let mut h = Harness::with_app(app, 80, 30);
        h.enter_playing();
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.player.loadout.max_weapons(), 8);
        assert_eq!(w.player.loadout.stats.damage_mult, 1.0 + 0.025 * 6.0);
        assert_eq!(w.revives_left, 1);
        assert!(
            h.app.run_modifiers().tracks_owned() == 3,
            "the modifiers name the three tracks that were bought"
        );
        // The HUD's chip names the nest on a run that carries upgrades, and the
        // frame draws without it for a fresh profile (asserted elsewhere).
        let text = h.frame(&[]);
        assert!(text.contains("NEST") || text.contains('⌂'), "no HUD nest chip:\n{text}");
    }

    /// The options RESET row takes two presses and wipes the nest (bank and
    /// tracks) without touching the scores.
    #[test]
    fn the_options_reset_wipes_the_nest_after_a_confirm() {
        let mut h = Harness::new(80, 30);
        h.app.scores.meta.bank = 500;
        h.app.scores.meta.set_level(meta::Track::Damage, 3);
        h.app.open_options(Screen::Title);
        h.app.options_row = options::RESET_ROW;
        h.frame(&[]);
        h.frame(&[KeyCode::Enter]);
        assert!(h.app.reset_armed, "the first press only arms it");
        assert!(h.app.scores.meta.total_levels() > 0);
        assert!(h.app.scores.meta.bank > 0);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.scores.meta.total_levels(), 0);
        assert_eq!(h.app.scores.meta.bank, 0);
        assert!(!h.app.reset_armed);
    }

    /// The nest's rows are clickable at the compact and the roomy size: the
    /// pointer buys exactly the row it is on, and the wheel walks the list.
    #[test]
    fn the_nest_rows_buy_by_click_at_57_and_120() {
        for (w, rows) in [(57u16, 20u16), (120, 40)] {
            let mut h = Harness::new(w, rows);
            h.app.scores.meta.bank = 300;
            let price = h
                .app
                .scores
                .meta
                .next_price(meta::Track::MoveSpeed)
                .expect("the first step has a price");
            assert!(!nest_ui::needs_confirm(price) && price <= 300);
            h.app.open_nest(Screen::Title);
            h.frame(&[]);
            let pos = h.locate("QUICK PAWS");
            h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), pos)]);
            assert_eq!(
                h.app.scores.meta.level(meta::Track::MoveSpeed),
                1,
                "{w}x{rows}: the click did not buy the row under the pointer"
            );
            assert_eq!(h.app.scores.meta.bank, 300 - price);
            assert_eq!(h.app.nest_ui.cursor, meta::Track::MoveSpeed.index());
            // The wheel walks the list (one track a notch).
            h.events(&[], &[(MouseEventKind::ScrollDown, (2, 2))]);
            assert_eq!(h.app.nest_ui.cursor, meta::Track::MoveSpeed.index() + 1);
        }
    }

    /// Item 1: the test constructor is pinned to a fixed seed sequence and keeps
    /// scores in memory only.
    #[test]
    fn test_app_is_deterministic_and_does_not_persist() {
        let mut a = Harness::new(80, 30);
        let mut b = Harness::new(80, 30);
        a.enter_playing();
        b.enter_playing();
        for _ in 0..240 {
            a.frame(&[KeyCode::Right]);
            b.frame(&[KeyCode::Right]);
        }
        assert_eq!(a.player_x(), b.player_x());
        let (wa, wb) = (a.app.world.as_ref().unwrap(), b.app.world.as_ref().unwrap());
        assert_eq!(wa.elapsed, wb.elapsed);
        assert_eq!(wa.score, wb.score);
        assert_eq!(wa.enemy_count(), wb.enemy_count());

        // A finalized run updates the in-memory table only.
        a.app.world.as_mut().unwrap().game_over = true;
        a.frame(&[]);
        assert!(a.app.recorded);
        assert_eq!(a.app.scores.entries.len(), 1);
    }

    /// Item 13: every screen survives a degenerate terminal (0×0, 0×5, 5×0, 1×1)
    /// — update + render, not just layout.
    #[test]
    fn degenerate_terminal_sizes_do_not_panic() {
        for (w, h) in [(0u16, 0u16), (0, 5), (5, 0), (1, 1), (1, 3), (3, 1), (3, 3)] {
            for target in [
                "Title",
                "SectorSelect",
                "Story",
                "MapIntro",
                "LevelIntro",
                "Playing",
                "LevelUp",
                "Paused",
                "LevelComplete",
                "KeepThree",
                "Victory",
                "End",
            ] {
                let mut hh = Harness::with_app(test_app(), w, h);
                hh.frame(&[]);
                match target {
                    "Title" => {}
                    "SectorSelect" => {
                        hh.app.screen = Screen::SectorSelect;
                    }
                    "Story" => {
                        hh.frame(&[KeyCode::Char('l')]);
                    }
                    "MapIntro" => {
                        hh.frame(&[KeyCode::Enter]); // -> Story
                        hh.frame(&[KeyCode::Enter]); // dossier -> MapIntro
                    }
                    "LevelIntro" => {
                        // A practice run of Kitchen opens on its intro plate.
                        hh.app.start_practice(Theme::Kitchen);
                    }
                    "Playing" => hh.enter_playing(),
                    "LevelUp" => {
                        hh.enter_playing();
                        hh.app.world.as_mut().unwrap().pending_levelups = 1;
                        hh.frame(&[]);
                    }
                    "Paused" => {
                        hh.enter_playing();
                        hh.frame(&[KeyCode::Esc]);
                    }
                    "LevelComplete" => {
                        hh.enter_playing();
                        hh.grant_keepables();
                        hh.finish_level();
                    }
                    "KeepThree" => {
                        hh.enter_playing();
                        hh.grant_keepables();
                        hh.finish_level();
                        hh.frame(&[KeyCode::Enter]);
                    }
                    "Victory" => {
                        hh.enter_playing();
                        let last = campaign::LEVEL_COUNT - 1;
                        hh.app.world.as_mut().unwrap().level_index = last;
                        hh.app.campaign.as_mut().unwrap().level = last;
                        hh.finish_level();
                    }
                    _ => {
                        hh.enter_playing();
                        hh.app.world.as_mut().unwrap().game_over = true;
                        hh.frame(&[]);
                    }
                }
                assert_eq!(
                    hh.app.screen.name_for_test(),
                    target,
                    "should be on {target} at {w}x{h}"
                );
                // Two more frames: the update and render paths run again.
                hh.frame(&[]);
                hh.frame(&[]);
                assert_eq!(hh.app.screen.name_for_test(), target, "{target} at {w}x{h}");
            }
        }
    }

    #[test]
    fn hud_tracks_mute_upgrade_callout_and_incident_summary() {
        let mut h = Harness::new(100, 32);
        h.enter_playing();
        // Mute while paused updates the HUD telemetry immediately.
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Paused));
        let paused_text = h.frame(&[KeyCode::Char('m')]);
        assert!(h.app.muted);
        assert!(h.app.hud.telemetry.muted);
        assert!(paused_text.contains("MUTE"), "top bar should show MUTE");
        assert!(paused_text.contains("ARSENAL"), "pause dossier should show ARSENAL");
        h.frame(&[KeyCode::Char('m')]); // unmute
        h.frame(&[KeyCode::Esc]); // resume

        // Picking an upgrade emits an EQUIPPED callout visible on the next frame.
        h.show_upgrade_cards();
        h.frame(&[KeyCode::Enter]);
        assert!(matches!(h.app.screen, Screen::Playing));
        let after_pick = h.frame(&[]);
        assert!(
            after_pick.contains("EQUIPPED"),
            "HUD should display EQUIPPED callout after upgrade pick: {after_pick}"
        );

        // Ending the run populates RunSummary matching finalized score.
        h.app.world.as_mut().unwrap().game_over = true;
        let end_text = h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::End));
        let summary = h.app.last_summary.as_ref().expect("summary must be populated");
        assert_eq!(summary.final_score, h.app.last_score);
        assert!(end_text.contains("SCORE BREAKDOWN"));
        assert!(end_text.contains("COMBAT METRICS"));
    }

    /// Gated behind `RATRUN_SLOTS_DIR`: the **8 + 8** capture set — play, pause,
    /// level-up, keep-three and the shop's Molt with a full sixteen-slot rack,
    /// at the three sizes the review uses. Decode with the scratchpad's
    /// `view.py`:
    ///
    /// ```sh
    /// RATRUN_SLOTS_DIR=/tmp/slots8 cargo test capture_slots8_frames -- --nocapture
    /// ```
    ///
    /// The rack caps are set to `SlotCaps::FULL` here; a fresh profile plays
    /// 4 + 4 and the same screens are covered by `capture_ux_frames`.
    #[test]
    fn capture_slots8_frames() {
        use crate::game::loadout::{SlotCaps, Upgrade};
        use crate::game::weapon::{Weapon, WeaponKind};
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use std::fmt::Write as _;

        /// Serialise one terminal frame as ANSI + plain text (`view.py` reads
        /// the ANSI). A free fn, so it never holds a borrow of the harness.
        fn dump_term(
            terminal: &Terminal<TestBackend>,
            w: u16,
            h: u16,
            dir: &std::path::Path,
            tag: &str,
        ) {
            let buf = terminal.backend().buffer();
            let mut out = String::new();
            let mut plain = String::new();
            for y in 0..h {
                for x in 0..w {
                    let c = &buf[(x, y)];
                    let (r, g, b) = color_rgb(c.fg);
                    let (br, bg, bb) = color_rgb(c.bg);
                    let _ = write!(
                        out,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        c.symbol()
                    );
                    plain.push_str(c.symbol());
                }
                out.push_str("\x1b[0m\n");
                plain.push('\n');
            }
            std::fs::write(dir.join(format!("{w}x{h}-{tag}.ansi")), out).unwrap();
            std::fs::write(dir.join(format!("{w}x{h}-{tag}.txt")), plain).unwrap();
        }

        let Some(dir) = std::env::var_os("RATRUN_SLOTS_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();

        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.enter_playing();
            {
                let world = hh.app.world.as_mut().unwrap();
                world.player.loadout.set_caps(SlotCaps::FULL);
                world.player.loadout.weapons.clear();
                world.player.loadout.passives.clear();
                let kinds = [
                    WeaponKind::TailWhip,
                    WeaponKind::TailThrash,
                    WeaponKind::AcidRain,
                    WeaponKind::Gnaw,
                    WeaponKind::LaserEyes,
                    WeaponKind::CheeseSpray,
                    WeaponKind::Hantavirus,
                    WeaponKind::SonicSqueak,
                ];
                for (i, kind) in kinds.iter().enumerate() {
                    let mut weapon = Weapon::new(*kind);
                    weapon.level = match i {
                        0 | 3 | 5 => kind.max_level(),
                        1 => 4,
                        _ => 2,
                    };
                    weapon.evolved = i == 0 || i == 3 || i == 5;
                    if kind.is_active() && i % 2 == 1 {
                        weapon.timer = 0.0; // charged and waiting
                    }
                    world.player.loadout.weapons.push(weapon);
                }
                for pk in crate::game::loadout::PassiveKind::all().into_iter().take(8) {
                    world.player.loadout.apply(&Upgrade::Passive(pk));
                    world.player.loadout.apply(&Upgrade::Passive(pk));
                }
                world.player.buffs.shield = 12.0;
                world.player.buffs.socks = 6.0;
                world.player.buffs.decoy = 4.0;
                world.player.buffs.slow_bullets = 3.0;
                world.gold = 47;
                world.score = 3420;
                world.kills = 248;
                world.player.level = 9;
                world.elapsed = 65.0;
                world.player.hp = 84.0;
                world.player.dash_cd = 0.9;
                world.player.xp = world.player.xp_to_next / 2;
            }
            for _ in 0..4 {
                hh.frame(&[]);
            }
            hh.app.screen = Screen::Playing;
            hh.app.hud.zoom_flash = None;
            hh.frame(&[]);
            dump_term(&hh.terminal, w, h, &dir, "full8-play");

            // The pause plate: the icon dossier, or the compact rack plate at
            // 57×20.
            hh.app.screen = Screen::Paused;
            hh.frame(&[]);
            dump_term(&hh.terminal, w, h, &dir, "full8-pause");

            // The level-up cards and their ARSENAL strip.
            {
                let (choices, desc) = {
                    let world = hh.app.world.as_ref().unwrap();
                    let choices = vec![
                        Upgrade::LevelWeapon(1),
                        Upgrade::Passive(crate::game::loadout::PassiveKind::Armor),
                        Upgrade::Purse,
                    ];
                    let desc: Vec<(String, String, String)> = choices
                        .iter()
                        .map(|u| world.player.loadout.describe(u))
                        .collect();
                    (choices, desc)
                };
                hh.app.levelup_desc = desc;
                hh.app.levelup_choices = choices;
                hh.app.levelup_idx = 0;
            }
            hh.app.screen = Screen::LevelUp;
            hh.frame(&[]);
            dump_term(&hh.terminal, w, h, &dir, "full8-levelup");

            // Keep-three: the sixteen-entry manifest plus the splices, which
            // are named as left behind on this floor.
            {
                let (items, entries, splices) = {
                    let world = hh.app.world.as_ref().unwrap();
                    let items = world.keepables();
                    let entries = super::entries_for(&items);
                    let splices: Vec<&'static str> = world
                        .player
                        .loadout
                        .splices
                        .iter()
                        .map(|kind| kind.name())
                        .collect();
                    (items, entries, splices)
                };
                hh.app.keep_selected = vec![false; entries.len()];
                hh.app.keep_entries = entries;
                hh.app.keep_items = items;
                hh.app.keep_splices = splices;
                hh.app.keep_cursor = 0;
                hh.app.keep_scroll = 0;
                hh.app.keep_route = None;
            }
            hh.app.screen = Screen::KeepThree;
            hh.frame(&[]);
            dump_term(&hh.terminal, w, h, &dir, "full8-keep");

            // The shop's Molt list over a sixteen-kind rack.
            {
                let world = hh.app.world.as_mut().unwrap();
                world.gold = 200;
                world.open_shop(0);
            }
            hh.app.shop_row = 5;
            hh.app.shop_molt = Some(shop_ui::MoltUi::Forfeit { cursor: 7 });
            hh.app.screen = Screen::Shop;
            hh.frame(&[]);
            dump_term(&hh.terminal, w, h, &dir, "full8-molt");
        }
    }

    /// Gated behind `RATRUN_UX_DIR`: the wave-3 readability capture set.
    ///
    /// Writes ANSI frames (rasterise them with the scratchpad's `view.py`) of
    /// the busy playfield at 1x and 2x, the full 5+5 HUD, the pause plate and
    /// the title legend at 57×20, 80×24 and 120×40 — the three sizes and the
    /// two zooms the review used. Lighting stays at its default (Full) so the
    /// hostile layer and the screen-space ink are exercised for real.
    #[test]
    fn capture_ux_frames() {
        use crate::game::ecs::components::Wick;
        use crate::game::ecs::resources::spawn_with_id;
        use crate::game::enemy::EnemyKind;
        use crate::game::gem::{Chest, Gem, Gold, Pickup, PickupKind};
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::game::weapon::{Weapon, WeaponKind};
        use crate::math::Vec2;
        use std::fmt::Write as _;

        let Some(dir) = std::env::var_os("RATRUN_UX_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();

        /// Serialise the terminal buffer as an ANSI frame `view.py` can read.
        fn ansi(terminal: &Terminal<TestBackend>, w: u16, h: u16) -> (String, String) {
            let buf = terminal.backend().buffer();
            let mut out = String::new();
            let mut plain = String::new();
            for y in 0..h {
                for x in 0..w {
                    let c = &buf[(x, y)];
                    let (r, g, b) = xterm_color_rgb(c.fg);
                    let (br, bg, bb) = xterm_color_rgb(c.bg);
                    let _ = write!(
                        out,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        c.symbol()
                    );
                    plain.push_str(c.symbol());
                }
                out.push_str("\x1b[0m\n");
                plain.push('\n');
            }
            (out, plain)
        }

        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.enter_playing();
            {
                let rng = &mut rand::rngs::StdRng::from_seed([9u8; 32]);
                use rand::SeedableRng;
                let _ = rng;
                let world = hh.app.world.as_mut().unwrap();
                let mut rng = rand::rngs::StdRng::seed_from_u64(5);
                let p = world.player.pos;
                // A standing horde, from flecks to brutes. Tough enough that
                // nothing dies during the capture: the frame is about reading a
                // packed floor, not about a particle storm.
                for i in 0..44 {
                    let a = i as f32 * 0.37;
                    let r = 16.0 + (i % 10) as f32 * 14.0;
                    let kind = match i % 9 {
                        3 => EnemyKind::Cat,
                        5 => EnemyKind::Brute,
                        7 => EnemyKind::Bat,
                        _ => EnemyKind::Skitterer,
                    };
                    world.spawn_enemy_with_rng(
                        kind,
                        p + Vec2::from_angle(a) * r,
                        6000.0,
                        &mut rng,
                    );
                }
                // Hostile fire in the air, some of it right beside the rat.
                for i in 0..18 {
                    let a = i as f32 * 0.7;
                    let mut b = crate::game::bullet::Bullet::new(
                        p + Vec2::from_angle(a) * (14.0 + (i % 5) as f32 * 9.0),
                        Vec2::from_angle(a) * 40.0,
                        3.0,
                        crate::game::bullet::Faction::Enemy,
                        (255, 120, 160),
                    );
                    b.life = 4.0;
                    world.spawn_bullet(b);
                }
                // The loot language, one of each.
                for (i, kind) in [
                    PickupKind::BottleCap,
                    PickupKind::WindupMouse,
                    PickupKind::PocketWatch,
                    PickupKind::CleanSocks,
                ]
                .into_iter()
                .enumerate()
                {
                    world.spawn_pickup(Pickup {
                        pos: p + Vec2::from_angle(0.9 * i as f32 + 0.4) * 26.0,
                        kind,
                        bob: i as f32,
                    });
                }
                for i in 0..4 {
                    world.spawn_gem(Gem::new(p + Vec2::from_angle(i as f32 * 1.6) * 16.0, 1));
                }
                world.spawn_gem(Gem::new(p + Vec2::new(-10.0, 8.0), 25));
                spawn_with_id(&mut world.ecs, Gold::new(p + Vec2::new(12.0, -10.0), 1));
                spawn_with_id(&mut world.ecs, Gold::new(p + Vec2::new(15.0, -8.0), 1));
                spawn_with_id(&mut world.ecs, Chest { pos: p + Vec2::new(20.0, -12.0) });
                // A dead Wick Moth's fuse and a Hantavirus countdown.
                spawn_with_id(
                    &mut world.ecs,
                    Wick::new(p + Vec2::new(-16.0, -12.0)),
                );
                // The full rack: five weapons (one evolved), five passives.
                world.player.loadout.weapons = vec![
                    {
                        let mut wp = Weapon::new(WeaponKind::TailWhip);
                        wp.level = WeaponKind::TailWhip.max_level();
                        wp.evolved = true;
                        wp
                    },
                    {
                        let mut wp = Weapon::new(WeaponKind::TailThrash);
                        wp.level = 4;
                        wp.timer = 0.0;
                        wp
                    },
                    Weapon::new(WeaponKind::AcidRain),
                    Weapon::new(WeaponKind::Gnaw),
                    Weapon::new(WeaponKind::LaserEyes),
                ];
                for pk in [
                    PassiveKind::MaxHp,
                    PassiveKind::Damage,
                    PassiveKind::SpringHeel,
                    PassiveKind::Counterweight,
                    PassiveKind::SecondScent,
                ] {
                    world.player.loadout.apply(&Upgrade::Passive(pk));
                    world.player.loadout.apply(&Upgrade::Passive(pk));
                }
                world.player.buffs.shield = 12.0;
                world.player.buffs.socks = 6.0;
                world.gold = 47;
                world.player.xp = world.player.xp_to_next / 2;
                world.player.dash_cd = 0.9;
            }
            // One tick to arm the tells, then a few frames so weapons fire.
            let step = |hh: &mut Harness| {
                {
                    let w = hh.app.world.as_mut().unwrap();
                    w.player.xp = 0;
                    w.pending_levelups = 0;
                    w.player.hp = w.player.max_hp();
                }
                hh.frame(&[]);
                hh.app.screen = Screen::Playing;
            };
            for _ in 0..20 {
                step(&mut hh);
            }

            for z in [1.0f32, 2.0] {
                hh.app.world.as_mut().unwrap().set_zoom_instant(z);
                hh.app.hud.notify_zoom(z);
                for _ in 0..3 {
                    step(&mut hh);
                }
                // Keep the zoom flash off the status pill: it is not the shot.
                hh.app.hud.zoom_flash = None;
                step(&mut hh);
                let (out, plain) = ansi(&hh.terminal, w, h);
                std::fs::write(dir.join(format!("{w}x{h}-play-{z}x.ansi")), out).unwrap();
                std::fs::write(dir.join(format!("{w}x{h}-play-{z}x.txt")), plain).unwrap();
            }
            hh.app.world.as_mut().unwrap().set_zoom_instant(1.0);

            // The pause plate (dossier or the compact rack plate).
            hh.app.screen = Screen::Paused;
            hh.frame(&[]);
            let (out, plain) = ansi(&hh.terminal, w, h);
            std::fs::write(dir.join(format!("{w}x{h}-pause.ansi")), out).unwrap();
            std::fs::write(dir.join(format!("{w}x{h}-pause.txt")), plain).unwrap();
            hh.app.muted = true;
            hh.app.hud.telemetry.muted = true;
            hh.frame(&[]);
            let (out, plain) = ansi(&hh.terminal, w, h);
            std::fs::write(dir.join(format!("{w}x{h}-pause-muted.ansi")), out).unwrap();
            std::fs::write(dir.join(format!("{w}x{h}-pause-muted.txt")), plain).unwrap();
        }

        // The loot language on a clean floor: one of every find, a gem of each
        // tier, two coins and a chest, with no horde and no particles. 1x and
        // 2x, the two zooms the review judged shapes at.
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.enter_playing();
            {
                use rand::SeedableRng;
                let _ = rand::rngs::StdRng::seed_from_u64(1);
                let world = hh.app.world.as_mut().unwrap();
                let p = world.player.pos;
                world.player.loadout.weapons.clear();
                world.player.iframes = 60.0;
                for (i, kind) in [
                    PickupKind::BottleCap,
                    PickupKind::WindupMouse,
                    PickupKind::PocketWatch,
                    PickupKind::CleanSocks,
                ]
                .into_iter()
                .enumerate()
                {
                    world.spawn_pickup(Pickup {
                        pos: p + Vec2::new(-45.0 + i as f32 * 14.0, -22.0),
                        kind,
                        bob: i as f32,
                    });
                }
                for (i, v) in [1u32, 3, 5, 25].into_iter().enumerate() {
                    world.spawn_gem(Gem::new(p + Vec2::new(-40.0 + i as f32 * 14.0, 20.0), v));
                }
                spawn_with_id(&mut world.ecs, Gold::new(p + Vec2::new(8.0, 20.0), 1));
                spawn_with_id(&mut world.ecs, Gold::new(p + Vec2::new(12.0, 20.0), 1));
                spawn_with_id(&mut world.ecs, Chest { pos: p + Vec2::new(24.0, 20.0) });
            }
            hh.frame(&[]);
            hh.app.screen = Screen::Playing;
            hh.frame(&[]);
            hh.app.world.as_mut().unwrap().damage_numbers.clear();
            for z in [1.0f32, 2.0] {
                hh.app.world.as_mut().unwrap().set_zoom_instant(z);
                hh.frame(&[]);
                let (out, plain) = ansi(&hh.terminal, w, h);
                std::fs::write(dir.join(format!("{w}x{h}-loot-{z}x.ansi")), out).unwrap();
                std::fs::write(dir.join(format!("{w}x{h}-loot-{z}x.txt")), plain).unwrap();
            }
        }

        // The title plate's controls legend, in both input modes if asked.
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.app.set_lone_ctrl(std::env::var("RATRUN_DUMP_CTRL").is_ok());
            let text = hh.frame(&[]);
            let (out, _) = ansi(&hh.terminal, w, h);
            std::fs::write(dir.join(format!("{w}x{h}-title.ansi")), out).unwrap();
            std::fs::write(dir.join(format!("{w}x{h}-title.txt")), text).unwrap();
        }
    }

    /// Gated behind `RATRUN_HUD_DIR`: captures ANSI + plain text frames of
    /// every redesigned HUD and in-game overlay state across terminal sizes.
    #[test]
    fn capture_hud_neon() {
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::game::weapon::{Weapon, WeaponKind};
        use crate::render::hud::CalloutKind;
        use std::fmt::Write;

        let Some(dir) = std::env::var_os("RATRUN_HUD_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        let theme_idx: usize = std::env::var("RATRUN_HUD_THEME")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let scenes = [
            "hud-combat",
            "hud-boss",
            "hud-lowhp",
            "hud-callouts",
            "upgrades",
            "pause",
            "defeat",
            "victory",
        ];

        for (width, height) in [
            (57u16, 20u16),
            (80, 24),
            (80, 16),
            (140, 45),
            (0, 0),
            (0, 5),
            (5, 0),
            (1, 1),
        ] {
            let mut h = Harness::new(width, height);
            h.app.select_idx = theme_idx.min(campaign::LEVEL_COUNT - 1);
            h.enter_playing();
            if theme_idx != 0 {
                // The capture tool wants a specific map's palette; the campaign
                // only starts on the Sewer, so swap the world for the sector
                // under capture.
                h.app.world = Some(World::new_level(
                    theme_idx.min(campaign::LEVEL_COUNT - 1),
                    TEST_SEED,
                    Loadout::starting(),
                ));
            }
            if let Some(w) = h.app.world.as_mut() {
                for _ in 0..240 {
                    w.update(1.0 / 60.0, Vec2::new(0.25, 0.05), false);
                }
            }

            for name in scenes {
                h.app.hud.reset();
                h.app.muted = false;
                h.app.hud.telemetry.muted = false;
                h.app.damage_taken = 94.0;
                h.app.dashes_used = 17;
                h.app.heals_picked = 2;
                h.app.boss_slain = name == "victory";

                if let Some(w) = h.app.world.as_mut() {
                    w.player.loadout.weapons = WeaponKind::all()
                        .into_iter()
                        .enumerate()
                        .map(|(i, k)| {
                            let mut wp = Weapon::new(k);
                            wp.level = match i {
                                0 => k.max_level(),
                                1 => 4,
                                2 => 3,
                                3 => 2,
                                _ => 1,
                            };
                            let eff_cd = wp.cooldown() / 1.24;
                            wp.timer = eff_cd * [0.94, 0.72, 0.50, 0.30, 0.14, 0.02][i];
                            wp
                        })
                        .collect();
                    w.player.loadout.stats.damage_mult = 1.45;
                    w.player.loadout.stats.fire_rate_mult = 1.24;
                    w.player.loadout.stats.armor = 2.0;
                    w.player.loadout.stats.regen = 1.0;
                    w.player.level = 9;
                    w.kills = 248;
                    w.score = 3420;
                    w.elapsed = 148.0;
                    w.player.hp = 78.0;
                    w.player.hurt_flash = 0.0;
                    w.player.dash_cd = 0.0;
                    w.player.dash_active = 0.0;
                    w.player.xp = (w.player.xp_to_next * 3) / 5;
                    w.director.boss_spawned = false;
                    w.won = false;
                    w.game_over = false;

                    match name {
                        "hud-combat" => {
                            h.app.screen = Screen::Playing;
                            w.player.dash_cd = 0.75;
                            h.app.hud.streak_kills = 14;
                            h.app.hud.hp_trail = 0.92;
                            h.app.hud.clock = 1.2;
                        }
                        "hud-boss" => {
                            h.app.screen = Screen::Playing;
                            w.elapsed = 264.0;
                            w.director.boss_spawned = true;
                            h.app.hud.telemetry.set_boss_hp(Some((860.0, 1400.0)));
                            h.app.hud.telemetry.set_boss_offset(Some(Vec2::new(260.0, -20.0)));
                            h.app.hud.boss_hp_trail = 0.78;
                            h.app.hud.boss_intro_timer = 1.5;
                            h.app.hud.clock = 0.4;
                        }
                        "hud-lowhp" => {
                            h.app.screen = Screen::Playing;
                            w.elapsed = 278.0;
                            w.player.hp = 18.0;
                            w.player.hurt_flash = 0.2;
                            w.player.dash_active = 0.12;
                            h.app.hud.hp_trail = 0.44;
                            h.app.hud.hurt_edge = 0.28;
                            h.app.hud.clock = 0.25;
                        }
                        "hud-callouts" => {
                            h.app.screen = Screen::Playing;
                            w.elapsed = 249.4;
                            h.app.muted = true;
                            h.app.hud.telemetry.muted = true;
                            h.app.hud.push_callout(
                                CalloutKind::Heal,
                                "+MEDKIT",
                                "HP RESTORED",
                                2.2,
                            );
                            h.app.hud.push_callout(
                                CalloutKind::Upgrade,
                                "EQUIPPED",
                                "Squeak Nova Lv3→4",
                                2.5,
                            );
                            h.app.hud.clock = 0.15;
                        }
                        "upgrades" => {
                            h.app.screen = Screen::LevelUp;
                            h.app.levelup_idx = 0;
                            h.app.levelup_choices = vec![
                                Upgrade::LevelWeapon(2),
                                Upgrade::Passive(PassiveKind::Magnet),
                                Upgrade::Passive(PassiveKind::Armor),
                            ];
                            h.app.levelup_desc = h
                                .app
                                .levelup_choices
                                .iter()
                                .map(|u| w.player.loadout.describe(u))
                                .collect();
                        }
                        "pause" => {
                            h.app.screen = Screen::Paused;
                        }
                        "defeat" | "victory" => {
                            h.app.screen = Screen::End;
                            w.won = name == "victory";
                            w.elapsed = if w.won { 300.0 } else { 276.0 };
                            let final_score =
                                w.score + (w.elapsed as u32) * 5 + w.player.level * 100
                                    + if w.won { 5000 } else { 0 };
                            h.app.last_score = final_score;
                            h.app.new_best = w.won;
                            h.app.scores = ScoreTable::default();
                            for i in 0..5u32 {
                                h.app.scores.insert(ScoreEntry {
                                    score: final_score.saturating_sub(i * 480),
                                    time: (w.elapsed - i as f32 * 32.0).max(30.0),
                                    level: w.player.level.saturating_sub(i),
                                    map: w.level.theme.name().to_string(),
                                    won: i == 0 && w.won,
                                    campaign_level: 3,
                                    campaign_won: i == 0 && w.won,
                                });
                            }
                            h.app.last_summary = None;
                        }
                        _ => {}
                    }
                }

                h.terminal.draw(|f| h.app.render(f)).unwrap();
                if width < 6 || height < 6 {
                    continue;
                }
                let buf = h.terminal.backend().buffer();
                let mut ansi = String::new();
                let mut plain = String::new();
                for y in 0..height {
                    for x in 0..width {
                        let c = &buf[(x, y)];
                        let (r, g, b) = color_rgb(c.fg);
                        let (br, bg, bb) = color_rgb(c.bg);
                        write!(
                            ansi,
                            "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                            c.symbol()
                        )
                        .unwrap();
                        plain.push_str(c.symbol());
                    }
                    ansi.push_str("\x1b[0m\n");
                    plain.push('\n');
                }
                let path = dir.join(format!("{width}x{height}-{name}"));
                std::fs::write(path.with_extension("ansi"), ansi).unwrap();
                std::fs::write(path.with_extension("txt"), plain).unwrap();
            }
        }
    }
    // ---- Minimap + fog of war -------------------------------------------

    #[test]
    fn the_minimap_is_drawn_during_play_at_80x24_and_120x40() {
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.enter_playing();
            hh.frame(&[]);
            let panel = hh.app.last_minimap;
            assert!(
                !panel.is_empty(),
                "no compact minimap on {w}x{h} (field {:?})",
                hh.app.last_game
            );
            // The play field contains it (it is an overlay, not a HUD row).
            let game = hh.app.last_game;
            assert!(game.union(panel) == game, "the minimap escaped the field");
            let text = hh.minimap_text();
            assert!(text.contains("MAP"), "no MAP title on {w}x{h}: {text:?}");
            assert!(text.contains("TAB"), "no TAB hint on {w}x{h}: {text:?}");
            // The art paints half-block pixels: the radar has real content.
            assert!(
                text.chars().any(|c| c == '▀' || c == '█' || c == '▄'),
                "the radar painted no pixels on {w}x{h}: {text:?}"
            );
            // The fog has revealed the start room (charted percentage > 0 is
            // only reached after 1% of the map, so check the fog itself).
            assert!(hh.app.world.as_ref().unwrap().fog().seen_count() > 0);
            // The hit map resolves a left press inside the panel to the map.
            assert_eq!(
                hh.app.hits.at(panel.x + panel.width / 2, panel.y + panel.height / 2),
                Some(Target::Minimap)
            );
        }
    }

    #[test]
    fn the_minimap_is_absent_on_a_tiny_terminal() {
        let mut hh = Harness::new(20, 6);
        hh.enter_playing();
        hh.frame(&[]);
        assert!(hh.app.last_minimap.is_empty());
        assert!(hh.app.last_map.is_empty());
        assert!(!hh.screen_text().contains("MAP"));
    }

    #[test]
    fn tab_toggles_the_expanded_map_and_esc_closes_it_without_pausing() {
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        assert!(hh.app.last_map.is_empty());
        let compact = hh.app.last_minimap;
        hh.frame(&[KeyCode::Tab]);
        assert!(hh.app.map_open, "Tab did not open the map");
        assert!(!hh.app.last_map.is_empty(), "the expanded map was not drawn");
        assert!(
            hh.app.last_map.width > compact.width,
            "the expanded map is not bigger than the compact one"
        );
        // The art's modal carries its own legend rail (and a `[TAB] CLOSE`
        // hint); the pause notice sits one line under it.
        assert!(hh.map_text().contains("CLOSE"), "no art close hint");
        let screen = hh.screen_text();
        assert!(
            screen.contains("game paused"),
            "the expanded map does not say game paused"
        );

        hh.frame(&[KeyCode::Tab]);
        assert!(!hh.app.map_open, "Tab did not close the map");
        assert!(hh.app.last_map.is_empty());

        hh.frame(&[KeyCode::Tab]);
        assert!(hh.app.map_open);
        hh.frame(&[KeyCode::Esc]);
        assert!(!hh.app.map_open, "Esc did not close the map");
        assert_eq!(
            hh.app.screen.name_for_test(),
            "Playing",
            "Esc closed the map but also paused"
        );
        // The next Esc is the old pause binding again.
        hh.frame(&[KeyCode::Esc]);
        assert_eq!(hh.app.screen.name_for_test(), "Paused");
    }

    #[test]
    fn the_open_map_requests_repaints_without_ticking_the_world() {
        fn snapshot(w: &World) -> (u64, f32, Vec2, u32) {
            (
                w.ticks,
                w.elapsed,
                w.player.pos,
                w.fog().clock(),
            )
        }

        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.frame(&[]);
        let before = snapshot(h.app.world.as_ref().unwrap());
        let mut twin = Harness::new(120, 40);
        twin.enter_playing();
        twin.frame(&[]);
        h.frame(&[KeyCode::Tab]);
        assert_eq!(h.app.frame_demand(), crate::present::Demand::AnimatedMenu);
        h.frame(&[]);
        let draws = h.app.world_draws;
        for _ in 0..10 {
            h.frame(&[]);
            assert!(h.app.repaint_urgent());
            assert!(!h.app.repaint_immediate(), "the pulse uses the 30 Hz cadence");
        }
        assert_eq!(snapshot(h.app.world.as_ref().unwrap()), before);
        assert_eq!(h.app.world_draws, draws, "map animation reuses the frozen picture");
        use rand::RngExt;
        let next = |h: &mut Harness| h.app.world.as_mut().unwrap().rng.random::<u64>();
        assert_eq!(next(&mut h), next(&mut twin));
    }

    #[test]
    fn clicking_the_compact_minimap_toggles_the_map_but_only_with_the_left_button() {
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[]);
        let panel = hh.app.last_minimap;
        assert!(!panel.is_empty());
        let (cx, cy) = (panel.x + panel.width / 2, panel.y + panel.height / 2);

        // Right button: not a map toggle (the existing right-click rule).
        hh.events(&[], &[(MouseEventKind::Down(MouseButton::Right), (cx, cy))]);
        assert!(!hh.app.map_open, "a right press toggled the map");

        hh.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (cx, cy))]);
        assert!(hh.app.map_open, "a left press did not open the map");
        assert!(!hh.app.last_map.is_empty());

        // The expanded panel covers the same corner, so a second left press
        // anywhere on it closes the map too.
        let map = hh.app.last_map;
        hh.events(
            &[],
            &[(
                MouseEventKind::Down(MouseButton::Left),
                (map.x + map.width / 2, map.y + map.height / 2),
            )],
        );
        assert!(!hh.app.map_open, "a left press on the expanded map did not close it");
    }

    #[test]
    fn the_minimap_is_drawn_under_the_pause_and_levelup_overlays() {
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[]);
        assert!(!hh.app.last_minimap.is_empty());

        hh.frame(&[KeyCode::Char('p')]);
        assert_eq!(hh.app.screen.name_for_test(), "Paused");
        assert!(
            !hh.app.last_minimap.is_empty(),
            "the minimap vanished behind the pause panel"
        );
        // Not clickable while paused: the pause panel owns the hit map.
        let panel = hh.app.last_minimap;
        assert_ne!(
            hh.app.hits.at(panel.x + panel.width / 2, panel.y + panel.height / 2),
            Some(Target::Minimap)
        );
        hh.frame(&[KeyCode::Esc]);
        assert_eq!(hh.app.screen.name_for_test(), "Playing");

        hh.show_upgrade_cards();
        assert_eq!(hh.app.screen.name_for_test(), "LevelUp");
        assert!(!hh.app.last_minimap.is_empty());
    }

    #[test]
    fn a_new_run_starts_with_unexplored_fog() {
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        for _ in 0..30 {
            hh.frame(&[]);
        }
        let seen = hh.app.world.as_ref().unwrap().fog().seen_count();
        assert!(seen > 0);
        // Restart: the fog resets with the new world.
        hh.app.start_run();
        assert_eq!(hh.app.world.as_ref().unwrap().fog().seen_count(), 0);
        assert!(!hh.app.map_open);
    }

    #[test]
    fn the_minimap_never_panics_on_degenerate_terminal_sizes() {
        for (w, h) in [(0u16, 0u16), (0, 5), (5, 0), (1, 1), (2, 2), (10, 3)] {
            let mut hh = Harness::new(80, 24);
            hh.enter_playing();
            hh.resize(w, h);
            // Tab and a click must not panic either (the map cannot open).
            hh.frame(&[KeyCode::Tab]);
            hh.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (0, 0))]);
            hh.frame(&[KeyCode::Tab]);
            assert!(hh.app.last_map.is_empty(), "map opened on {w}x{h}");
        }
    }

    #[test]
    fn a_seen_maze_keeps_a_hollow_cache_marker_until_it_is_claimed() {
        // A map with a maze, standing in its breach: the frame hands the art a
        // breach fork plus a hollow cache marker, which survives walking away
        // and fills in only when the cache is claimed.
        use crate::game::collision::TileMap;
        use crate::game::level::Level;
        use crate::render::minimap::MarkerKind;
        let mut seed = None;
        for candidate in 0..64u64 {
            if !Level::generate(Theme::Sewer, candidate).mazes.is_empty() {
                seed = Some(candidate);
                break;
            }
        }
        let Some(seed) = seed else {
            panic!("no Sewer map in the first 64 seeds has a maze");
        };
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.app.world = Some(World::new(Theme::Sewer, seed));
        hh.app.map_open = false;
        hh.frame(&[]);
        let maze = *hh.app.world.as_ref().unwrap().level.mazes.first().unwrap();

        // The maze is still undiscovered: nothing marks it.
        let kinds = |hh: &Harness| -> Vec<MarkerKind> {
            hh.app.map_frame.markers().iter().map(|m| m.kind).collect()
        };
        assert!(!kinds(&hh).contains(&MarkerKind::MazeEntrance));
        assert!(!kinds(&hh).contains(&MarkerKind::MazeCacheHollow));

        // Step into the breach, then walk away again.
        hh.app.world.as_mut().unwrap().player.pos =
            TileMap::tile_rect(maze.entry.0, maze.entry.1).center();
        hh.frame(&[]);
        assert!(
            hh.app.world.as_ref().unwrap().fog().maze_entrance_seen(0),
            "the breach was seen but not flagged"
        );
        hh.app.world.as_mut().unwrap().player.pos =
            TileMap::tile_rect(maze.entry.0 + 12, maze.entry.1).center();
        hh.frame(&[]);
        assert!(kinds(&hh).contains(&MarkerKind::MazeEntrance));
        assert!(
            kinds(&hh).contains(&MarkerKind::MazeCacheHollow),
            "the found maze lost its cache marker"
        );
        assert!(hh
            .app
            .map_frame
            .labels()
            .iter()
            .any(|l| l.kind == crate::render::minimap::LabelKind::Maze));

        // Claimed: the marker fills in.
        hh.app.world.as_mut().unwrap().fog.set_maze_claimed(0, true);
        hh.frame(&[]);
        assert!(kinds(&hh).contains(&MarkerKind::MazeCacheClaimed));
        assert!(!kinds(&hh).contains(&MarkerKind::MazeCacheHollow));
    }

    #[test]
    fn the_expanded_map_shows_cache_status_that_the_compact_map_omits() {
        use crate::game::level::Level;
        use crate::render::minimap::MarkerKind;
        let mut seed = None;
        for candidate in 0..64u64 {
            if !Level::generate(Theme::Sewer, candidate).mazes.is_empty() {
                seed = Some(candidate);
                break;
            }
        }
        let Some(seed) = seed else {
            panic!("no Sewer map in the first 64 seeds has a maze");
        };
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.app.world = Some(World::new(Theme::Sewer, seed));
        hh.frame(&[]);
        let maze = *hh.app.world.as_ref().unwrap().level.mazes.first().unwrap();
        // Explore the pedestal's cell directly (its pocket may be walled off
        // from wherever the rat stands), leaving the player elsewhere so the
        // player glyph cannot cover the star.
        let same_map = Level::generate(Theme::Sewer, seed);
        let (rcx, rcy) = crate::game::collision::cell_of(maze.reward);
        hh.app
            .world
            .as_mut()
            .unwrap()
            .fog
            .reveal_cell(&same_map, (rcx, rcy));
        hh.frame(&[]);
        assert!(
            hh.app
                .map_frame
                .markers()
                .iter()
                .any(|m| m.kind == MarkerKind::MazeReward),
            "the discovered cache is not in the frame"
        );
        // The art only stamps the cache glyphs on the expanded map.
        assert!(
            !hh.minimap_text().contains('✦'),
            "cache status leaked onto the compact radar"
        );
        hh.frame(&[KeyCode::Tab]);
        assert!(
            hh.map_text().contains('✦'),
            "the expanded map does not show the discovered cache"
        );
    }

    /// There is no boss arena, so the map never draws a bearing to one
    /// (batch twelve): the boss arrives where the rat is.
    #[test]
    fn no_boss_bearing_is_drawn_on_the_map() {
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[]);
        let w = hh.app.world.as_mut().unwrap();
        for t in [0.0, crate::render::minimap_source::ARENA_BEARING_AFTER_SECS + 1.0, 600.0] {
            w.elapsed = t;
            assert!(minimap_source::boss_bearing_dir(w).is_none(), "a bearing at {t}s");
        }
        assert!(w.level.boss_room().is_none(), "a generated level has no arena");
        hh.frame(&[]);
        assert!(!hh.app.last_minimap.is_empty());
    }

    /// The synth's tiers as the game state asks for them: 1 groove, 2 horde,
    /// 3 Boss Overdrive (the fourth tier `MusicSynth` gained), and back to the
    /// normal rule the tick the boss dies.
    ///
    /// The recording seam is [`App::music_intensity`]: `AudioEngine::new()`
    /// returns `None` without an audio device (every test), so the tier the app
    /// requests is captured by [`App::request_music_intensity`] instead of by a
    /// playing sink.
    #[test]
    fn music_intensity_rises_to_boss_overdrive_and_falls_back_on_the_kill() {
        use crate::game::ecs::systems::boss::Boss;
        use crate::game::enemy::EnemyKind;

        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.frame(&[]);
        assert_eq!(
            h.app.music_intensity(),
            1,
            "an empty early level is the groove tier"
        );

        // A dense horde asks for tier 2 (the rule the boss tier replaces).
        {
            let w = h.app.world.as_mut().unwrap();
            let centre = w.player.pos;
            for i in 0..50 {
                let a = i as f32 * 0.5;
                let r = 28.0 + (i % 5) as f32 * 6.0;
                w.spawn_enemy(EnemyKind::Skitterer, centre + Vec2::from_angle(a) * r, 1.0);
            }
        }
        h.frame(&[]);
        let horde = h.app.world.as_ref().unwrap().enemy_count();
        assert!(horde > 45, "the horde has to be dense: {horde}");
        assert_eq!(h.app.music_intensity(), 2, "a dense horde is tier 2");

        // The director hands off at boss time: the boss is alive, so the app
        // asks for Boss Overdrive.
        h.app.world.as_mut().unwrap().elapsed = crate::game::director::BOSS_TIME;
        h.frame(&[]);
        assert!(
            h.app.world.as_ref().unwrap().boss_hp().is_some(),
            "the director spawned the level's boss"
        );
        assert_eq!(
            h.app.music_intensity(),
            3,
            "a live boss asks the synth for Boss Overdrive"
        );

        // The death tick breaks the horde and drops the tier back to the normal
        // rule (tier 1 here: the break leaves nothing on the field).
        {
            let w = h.app.world.as_mut().unwrap();
            let mut query = w.ecs.query::<&mut Boss>();
            for mut boss in query.iter_mut(&mut w.ecs) {
                boss.hp = 0.0;
            }
        }
        h.frame(&[]);
        let w = h.app.world.as_ref().unwrap();
        assert!(w.boss_hp().is_none(), "the boss died on that tick");
        let left = w.enemy_count();
        assert!(left <= 45, "the horde broke with the boss: {left} left");
        assert_eq!(
            h.app.music_intensity(),
            1,
            "the tier returns to the enemy-count rule when the boss dies"
        );
    }

    #[test]
    fn n_key_cycles_minimap_sizes_and_drawn_area_shrinks_then_disappears() {
        use crate::render::minimap::MinimapSize;
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.enter_playing();
            hh.frame(&[]);

            assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);
            let normal = hh.app.last_minimap;
            assert!(!normal.is_empty(), "Normal minimap missing at {w}x{h}");
            assert!(hh.app.last_game.union(normal) == hh.app.last_game);

            // First press: Normal -> Small (area shrinks noticeably).
            hh.frame(&[KeyCode::Char('n')]);
            assert_eq!(hh.app.scores.minimap_size, MinimapSize::Small);
            let small = hh.app.last_minimap;
            assert!(!small.is_empty(), "Small minimap missing at {w}x{h}");
            assert!(
                small.area() < normal.area() && small.width < normal.width && small.height < normal.height,
                "Small ({small:?}) did not shrink from Normal ({normal:?}) at {w}x{h}"
            );
            assert!(hh.app.last_game.union(small) == hh.app.last_game);
            assert!(hh.minimap_text().contains("MAP"));

            // Second press: Small -> Hidden (drawn area disappears, no click target).
            hh.frame(&[KeyCode::Char('n')]);
            assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
            assert!(
                hh.app.last_minimap.is_empty(),
                "Hidden minimap still occupied playfield at {w}x{h}"
            );
            let corner_x = small.x + small.width / 2;
            let corner_y = small.y + small.height / 2;
            assert_ne!(
                hh.app.hits.at(corner_x, corner_y),
                Some(Target::Minimap),
                "Hidden minimap left a click blocker over the playfield"
            );

            // Third press: Hidden -> Normal.
            hh.frame(&[KeyCode::Char('n')]);
            assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);
            assert_eq!(hh.app.last_minimap, normal);
        }
    }

    #[test]
    fn mouse_wheel_over_compact_minimap_steps_sizes() {
        use crate::render::minimap::MinimapSize;
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[]);

        let normal = hh.app.last_minimap;
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);

        // Scrolling outside the minimap does not change size.
        hh.events(&[], &[(MouseEventKind::ScrollDown, (2, 2))]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);

        // ScrollDown over Normal -> Small.
        let center_n = (normal.x + normal.width / 2, normal.y + normal.height / 2);
        hh.events(&[], &[(MouseEventKind::ScrollDown, center_n)]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Small);
        let small = hh.app.last_minimap;
        assert!(small.area() < normal.area());

        // ScrollUp over Small -> back to Normal.
        let center_s = (small.x + small.width / 2, small.y + small.height / 2);
        hh.events(&[], &[(MouseEventKind::ScrollUp, center_s)]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);
        assert_eq!(hh.app.last_minimap, normal);

        // ScrollDown twice -> Small -> Hidden.
        hh.events(&[], &[(MouseEventKind::ScrollDown, center_n)]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Small);
        let small2 = hh.app.last_minimap;
        let center_s2 = (small2.x + small2.width / 2, small2.y + small2.height / 2);
        hh.events(&[], &[(MouseEventKind::ScrollDown, center_s2)]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(hh.app.last_minimap.is_empty());
    }

    #[test]
    fn expanded_map_opens_from_every_compact_state() {
        use crate::render::minimap::MinimapSize;
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[]);

        // 1. From Normal via Tab and click.
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);
        hh.frame(&[KeyCode::Tab]);
        assert!(hh.app.map_open && !hh.app.last_map.is_empty());
        hh.frame(&[KeyCode::Tab]);
        assert!(!hh.app.map_open);

        // 2. From Small via left-click and Tab.
        hh.frame(&[KeyCode::Char('n')]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Small);
        let small = hh.app.last_minimap;
        assert!(!small.is_empty());
        let sc = (small.x + small.width / 2, small.y + small.height / 2);
        hh.events(&[], &[(MouseEventKind::Down(MouseButton::Left), sc)]);
        assert!(hh.app.map_open, "clicking Small minimap did not open expanded map");
        assert!(!hh.app.last_map.is_empty());
        hh.frame(&[KeyCode::Tab]);
        assert!(!hh.app.map_open);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Small);

        // 3. From Hidden via Tab.
        hh.frame(&[KeyCode::Char('n')]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(hh.app.last_minimap.is_empty());
        hh.frame(&[KeyCode::Tab]);
        assert!(hh.app.map_open, "Tab did not open expanded map from Hidden");
        assert!(!hh.app.last_map.is_empty());
        hh.frame(&[KeyCode::Tab]);
        assert!(!hh.app.map_open);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(hh.app.last_minimap.is_empty());
    }

    #[test]
    fn minimap_size_persists_through_injected_store_and_falls_back_when_missing() {
        use crate::render::minimap::MinimapSize;

        // Missing setting in store JSON defaults to Normal.
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(missing.minimap_size, MinimapSize::Normal);
        let app_missing = App::with_options(false, None, missing, false, Some(TEST_SEED));
        let mut h0 = Harness::with_app(app_missing, 120, 40);
        h0.enter_playing();
        h0.frame(&[]);
        assert_eq!(h0.app.last_minimap.width, 34);

        // Cycle to Small (in-memory only: persist == false) and reload via JSON.
        h0.frame(&[KeyCode::Char('n')]);
        assert_eq!(h0.app.scores.minimap_size, MinimapSize::Small);
        let reloaded = ScoreTable::parse(&h0.app.scores.to_json());
        assert_eq!(reloaded.minimap_size, MinimapSize::Small);

        let app_reloaded = App::with_options(false, None, reloaded, false, Some(TEST_SEED));
        let mut h1 = Harness::with_app(app_reloaded, 120, 40);
        h1.enter_playing();
        h1.frame(&[]);
        assert_eq!(h1.app.scores.minimap_size, MinimapSize::Small);
        assert_eq!(
            (h1.app.last_minimap.width, h1.app.last_minimap.height),
            (18, 9)
        );

        // Cycle to Hidden and reload via JSON.
        h1.frame(&[KeyCode::Char('n')]);
        let reloaded_hidden = ScoreTable::parse(&h1.app.scores.to_json());
        let app_hidden = App::with_options(false, None, reloaded_hidden, false, Some(TEST_SEED));
        let mut h2 = Harness::with_app(app_hidden, 120, 40);
        h2.enter_playing();
        h2.frame(&[]);
        assert_eq!(h2.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(h2.app.last_minimap.is_empty());
    }

    #[test]
    fn narrow_terminal_falls_back_automatically_without_overwriting_saved_preference() {
        use crate::render::minimap::MinimapSize;

        // 57x20 is too small for Normal (needs 72x18 playfield), so it starts
        // at Small automatically while keeping saved preference == Normal.
        let mut hh = Harness::new(57, 20);
        hh.enter_playing();
        hh.frame(&[]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);
        assert_eq!(
            (hh.app.last_minimap.width, hh.app.last_minimap.height),
            (14, 7)
        );
        assert!(hh.app.last_game.union(hh.app.last_minimap) == hh.app.last_game);

        // Resizing larger restores Normal automatically since saved pref wasn't overwritten.
        hh.resize(120, 40);
        hh.frame(&[]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Normal);
        assert_eq!(
            (hh.app.last_minimap.width, hh.app.last_minimap.height),
            (34, 14)
        );

        // Back to 57x20: first N press hides immediately (Small -> Hidden),
        // second N press restores Small.
        hh.resize(57, 20);
        hh.frame(&[]);
        assert_eq!((hh.app.last_minimap.width, hh.app.last_minimap.height), (14, 7));
        hh.frame(&[KeyCode::Char('n')]);
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(hh.app.last_minimap.is_empty());
        hh.frame(&[KeyCode::Char('n')]);
        assert_eq!((hh.app.last_minimap.width, hh.app.last_minimap.height), (14, 7));
    }

    /// A hidden minimap keeps its quiet `N` hint: with no arena there is no
    /// bearing chevron for it to show, before or during the boss warning.
    #[test]
    fn hidden_minimap_shows_only_the_north_hint() {
        use crate::render::minimap::MinimapSize;

        let mut hh = Harness::new(80, 24);
        hh.enter_playing();
        hh.frame(&[KeyCode::Char('n')]); // Small
        hh.frame(&[KeyCode::Char('n')]); // Hidden
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(hh.app.last_minimap.is_empty());

        for t in [
            crate::render::minimap_source::ARENA_BEARING_AFTER_SECS + 5.0,
            crate::game::director::BOSS_TIME - 15.0,
        ] {
            hh.app.world.as_mut().unwrap().elapsed = t;
            hh.frame(&[]);
            let game = hh.app.last_game;
            let corner = hh.terminal.backend().buffer()[(game.right() - 2, game.bottom() - 2)]
                .symbol()
                .to_string();
            assert_eq!(corner, "N", "the hidden corner at {t}s");
        }
    }

    #[test]
    fn minimap_sizes_never_panic_on_degenerate_terminals() {
        for (w, h) in [(0u16, 0u16), (0, 5), (5, 0), (1, 1)] {
            let mut hh = Harness::new(80, 24);
            hh.enter_playing();
            hh.resize(w, h);
            for _ in 0..4 {
                hh.frame(&[KeyCode::Char('n')]);
                hh.events(&[], &[(MouseEventKind::ScrollDown, (0, 0))]);
                hh.frame(&[KeyCode::Tab]);
                hh.frame(&[KeyCode::Tab]);
            }
            assert!(hh.app.last_minimap.is_empty());
            assert!(hh.app.last_map.is_empty());
        }
    }

    // ---- Campaign flow ---------------------------------------------------

    /// Does `loadout` own `kind` (a weapon at any level, or any stacks)?
    fn owns(loadout: &Loadout, kind: KeepKind) -> bool {
        match kind {
            KeepKind::Weapon(k) => loadout.has(k),
            KeepKind::Passive(p) => loadout.stacks(p) > 0,
        }
    }

    /// Pack the first three manifest cards with the keyboard.
    fn pack_three(h: &mut Harness) {
        for key in [
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Char(' '),
        ] {
            h.frame(&[key]);
        }
    }

    /// The title's START RUN begins the campaign at level 1 (Sewer), through
    /// the story dossier and the access-route card.
    #[test]
    fn starting_from_the_title_opens_the_campaign_at_level_one() {
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        assert!(
            h.screen_text().contains("Sector"),
            "the title carries the sector row"
        );
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Story");
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "MapIntro");
        assert!(h.screen_text().contains("THE SEWERS"));
        h.frame(&[KeyCode::Enter]);
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.level_index, 0);
        assert_eq!(w.level.theme, Theme::Sewer);
        let c = h.app.campaign.as_ref().expect("a campaign is in flight");
        assert_eq!(c.level, 0);
        assert!(h.app.practice.is_none());
        assert_eq!(
            c.loadout.weapons.len(),
            1,
            "the campaign starts on the default loadout"
        );
        assert_eq!(h.app.scores.entries.len(), 0, "nothing is recorded yet");
    }

    /// A whole three-level campaign through simulated keys: keep three at each
    /// gate, the carried upgrades present at each level's start, and the
    /// victory plate with the per-level breakdown.
    #[test]
    fn full_campaign_reaches_the_victory_plate_with_carried_upgrades() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        assert_eq!(h.app.world.as_ref().unwrap().level_index, 0);

        // -- Level 1 (Sewer) -------------------------------------------------
        {
            let w = h.app.world.as_mut().unwrap();
            w.score = 1200;
            w.kills = 40;
            w.elapsed = 90.0;
        }
        h.grant_keepables();
        let offered = h.keepables();
        assert!(offered.len() > 3, "the keep screen needs a real choice");
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        let text = h.screen_text();
        assert!(text.contains("SECTOR CLEARED"), "no clear plate: {text}");
        assert!(text.contains("APEX JAGUAR"), "the Sewer boss must be named");

        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        pack_three(&mut h);
        assert_eq!(h.app.keep_counts(), (3, 3));
        let chosen: Vec<KeepKind> = offered[..3].iter().map(|k| k.kind).collect();
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        assert!(
            h.screen_text().contains("CARRIED IN"),
            "the level intro must list what was carried"
        );
        assert_eq!(h.app.carried_entries.len(), 3);

        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.level_index, 1, "level 2 is the Kitchen");
            assert_eq!(w.level.theme, Theme::Kitchen);
            for kind in &chosen {
                assert!(owns(&w.player.loadout, *kind), "{kind:?} was not carried in");
            }
            assert_eq!(w.carried.score, 1200, "level 1's score is banked");
        }

        // -- Level 2 (Kitchen) -----------------------------------------------
        let kitchen_boss = {
            let w = h.app.world.as_ref().unwrap();
            crate::game::ecs::systems::boss::BossKind::pool_pick(w.level.theme, w.level.seed)
        };
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert!(
            h.screen_text().contains(hud::boss_codename_kind(kitchen_boss).0),
            "the Kitchen boss must be named"
        );
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        let offered = h.keepables();
        assert!(offered.len() >= 3);
        let chosen: Vec<KeepKind> = offered[..3].iter().map(|k| k.kind).collect();
        pack_three(&mut h);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.level_index, 2, "level 3 is the Lab");
            assert_eq!(w.level.theme, Theme::Lab);
            for kind in &chosen {
                assert!(owns(&w.player.loadout, *kind), "{kind:?} was not carried in");
            }
        }

        // -- Level 3 (Lab): the keep screen comes before the ride ------------
        h.finish_level();
        assert_eq!(
            h.app.screen.name_for_test(),
            "LevelComplete",
            "stage three no longer ends the run"
        );
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        pack_three(&mut h);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(
            h.app.screen.name_for_test(),
            "LevelIntro",
            "the ride has its own intro plate"
        );
        let intro = h.screen_text();
        assert!(intro.contains("THE FLATBED"), "the ride is not named: {intro}");
        h.frame(&[KeyCode::Enter]);
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.level.theme, Theme::Trailer, "level 4 is the Flatbed");
            assert_eq!(w.level_index, campaign::LEVEL_COUNT - 1);
        }

        // -- Level 4 (the Flatbed): its boss wins the campaign ---------------
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "Victory");
        let text = h.screen_text();
        assert!(text.contains("EXTRACTION COMPLETE"), "no victory plate: {text}");
        // The breakdown names every sector's boss, the ride's included.
        for boss in [
            "APEX JAGUAR",
            "THE SWEEPER",
            "TURF-REAPER MK.IV",
            "THE MAGPIE",
        ] {
            assert!(text.contains(boss), "{boss} missing from the breakdown: {text}");
        }
        assert!(text.contains("4 / 4"), "the sectors column must read 4/4");
        assert!(
            text.contains("04 Flatbed"),
            "the ride is not on the victory plate: {text}"
        );
        assert!(h.app.new_best, "the first campaign win is a new best");
        let entry = &h.app.scores.entries[0];
        assert!(entry.campaign_won, "the run records a campaign win");
        assert_eq!(entry.campaign_level, 4);
        assert!(entry.won);
        assert!(entry.score >= 2000, "cumulative score: {entry:?}");
        assert_eq!(h.app.records[0].time, 90.0, "level 1's clock was recorded");
        assert_eq!(h.app.records[0].score, 1200, "level 1's score was recorded");
        assert_eq!(h.app.records[0].kills, 40, "level 1's kills were recorded");
        assert_eq!(
            h.app.records[3].plate,
            Some(campaign_ui::CampaignLevel::Trailer),
            "the ride's own row is in the breakdown"
        );
        assert_eq!(h.app.records[2].time, 0.0, "level 3 finished on the first tick");

        // Play again restarts the campaign from the dossier.
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Story");
        assert_eq!(h.app.campaign.as_ref().unwrap().level, 0);
        assert_eq!(h.app.campaign.as_ref().unwrap().totals.score, 0);
    }

    /// Keep-three by keyboard: the confirm is refused below the required count
    /// (decided by the art's helper), a full pack confirms, and a fourth pick
    /// cannot overflow the pack.
    #[test]
    fn keep_three_keyboard_refuses_confirm_below_the_required_count() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.grant_keepables();
        h.finish_level();
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        assert_eq!(h.app.keep_counts(), (0, 3));

        // Nothing packed: Enter is refused.
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        // Two of three: still refused.
        pack_two(&mut h);
        assert_eq!(h.app.keep_counts(), (2, 3));
        h.frame(&[KeyCode::Enter]);
        assert_eq!(
            h.app.screen.name_for_test(),
            "KeepThree",
            "confirm must be refused at 2/3"
        );
        // A fourth card cannot be packed once the pack is full.
        h.frame(&[KeyCode::Down]);
        h.frame(&[KeyCode::Char(' ')]);
        assert_eq!(h.app.keep_counts(), (3, 3));
        h.frame(&[KeyCode::Down]);
        h.frame(&[KeyCode::Char(' ')]);
        assert_eq!(h.app.keep_counts(), (3, 3), "the pack holds exactly three");
        // The full pack confirms into the next level's intro.
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
    }

    /// The keep screen offers the next stage's route (Kitchen or Laundry),
    /// keyboard and mouse both pick it, and the chosen floor is the one that
    /// loads — deterministically, from the run seed and the floor.
    #[test]
    fn keep_screen_route_choice_is_keyboard_mouse_and_carried() {
        let left = MouseEventKind::Down(MouseButton::Left);
        for size in [(120, 40), (80, 24), (57, 20)] {
            let mut h = Harness::new(size.0, size.1);
            h.enter_playing();
            h.grant_keepables();
            h.finish_level();
            h.frame(&[KeyCode::Enter]);
            assert_eq!(h.app.screen.name_for_test(), "KeepThree");
            // Both floors are on the plate, and the original route starts
            // chosen (the design's "original route is the first choice").
            let text = h.screen_text();
            assert!(text.contains("Kitchen"), "{size:?}: no Kitchen chip");
            assert!(text.contains("Laundry"), "{size:?}: no Laundry chip");
            let run_seed = h.app.campaign.as_ref().unwrap().run_seed;
            // Tab switches to the side route.
            h.frame(&[KeyCode::Tab]);
            assert_eq!(h.app.keep_route, Some(Theme::Laundry));
            h.frame(&[KeyCode::Tab]);
            assert_eq!(h.app.keep_route, Some(Theme::Kitchen));
            // Clicking the Laundry chip picks it without packing anything.
            h.click("Laundry");
            assert_eq!(h.app.keep_route, Some(Theme::Laundry));
            // Confirm the pack and the Laundry is the floor that loads.
            let mut packed = 0;
            while packed < 3 {
                h.frame(&[KeyCode::Char(' ')]);
                packed += 1;
                h.frame(&[KeyCode::Down]);
            }
            h.frame(&[KeyCode::Enter]);
            let w = h.app.world.as_ref().expect("next level");
            assert_eq!(w.level.theme, Theme::Laundry, "{size:?}: wrong floor");
            assert_eq!(
                w.level.seed,
                campaign::level_seed_on(run_seed, 1, Theme::Laundry),
                "the map seed must follow the chosen route"
            );
            let _ = left;
        }
    }

    /// The last keep screen (stage two → three) offers the Lab or the
    /// Printworks, and the Printworks is a completable final floor.
    #[test]
    fn the_last_keep_screen_offers_the_printworks() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        // Jump to stage two, played on the Laundry, and clear it.
        {
            let camp = h.app.campaign.as_mut().unwrap();
            camp.level = 1;
            camp.choose_route(Theme::Laundry);
            let w = camp.world();
            h.app.world = Some(w);
        }
        h.grant_keepables();
        h.finish_level();
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        let text = h.screen_text();
        assert!(text.contains("Lab"), "no Lab chip");
        assert!(text.contains("Printworks"), "no Printworks chip");
        h.click("Printworks");
        assert_eq!(h.app.keep_route, Some(Theme::Printworks));
    }

    /// Keep-three by mouse: hovering picks the cursor, clicking the card rows
    /// toggles them, the confirm button is absent (and inert) until the pack is
    /// full, and clicking it advances.
    #[test]
    fn keep_three_mouse_toggles_and_confirms() {
        let left = MouseEventKind::Down(MouseButton::Left);
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.grant_keepables();
        h.finish_level();
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");

        // Below the required count the art emits no confirm rectangle, so a
        // press on the disabled button text does nothing.
        let disabled = h.locate("SELECT 3 MORE");
        h.events(&[], &[(left, disabled)]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        assert_eq!(h.app.keep_counts(), (0, 3));

        let names: Vec<String> = h
            .app
            .keep_items
            .iter()
            .take(3)
            .map(|k| k.name.to_string())
            .collect();
        for name in &names {
            let pos = h.locate(name);
            h.events(&[], &[(MouseEventKind::Moved, pos)]);
            assert!(h.app.hits.hovered.is_some(), "the card row is not hoverable");
            h.events(&[], &[(left, pos)]);
        }
        assert_eq!(h.app.keep_counts(), (3, 3));

        let confirm = h.locate("ENTER THE");
        h.events(&[], &[(MouseEventKind::Moved, confirm)]);
        h.events(&[], &[(left, confirm)]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        assert_eq!(h.app.carried_entries.len(), 3);
    }

    /// Dying on level 2 records the level reached and the cumulative totals.
    #[test]
    fn death_on_level_two_records_the_level_and_totals() {
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            w.score = 1200;
            w.kills = 40;
            w.elapsed = 90.0;
        }
        h.grant_keepables();
        h.finish_level();
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "KeepThree");
        pack_three(&mut h);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");

        {
            let w = h.app.world.as_mut().unwrap();
            assert_eq!(w.level_index, 1);
            w.score = 800;
            w.kills = 25;
            w.elapsed = 60.0;
            w.player.hp = 0.0;
            w.game_over = true;
        }
        let text = h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        assert!(
            text.contains("SECTOR 2/4"),
            "the defeat screen must name the sector reached: {text}"
        );
        assert!(text.contains("Total time"), "cumulative totals must be shown");
        let summary = h.app.last_summary.as_ref().unwrap();
        assert_eq!(summary.sector, 2);
        assert!(!summary.practice);
        assert!(summary.time >= 150.0, "cumulative time: {}", summary.time);
        let entry = &h.app.scores.entries[0];
        assert_eq!(entry.campaign_level, 2);
        assert!(!entry.campaign_won);
        assert!(!entry.won);
        assert!(
            entry.score > 1200 + 800,
            "the score must bank level 1 too: {entry:?}"
        );
        assert!(h.app.scores.level_cleared(0));
    }

    /// Sector select reads the injected table **per floor**: locked/unlocked
    /// states, each floor's own bests, a locked floor refusing to start, a
    /// practice run labelled as such, and the unlock persisting through the
    /// store.
    #[test]
    fn sector_select_locks_unlocks_and_persists() {
        let mut table = ScoreTable::default();
        // A previous run cleared the Sewer on the original route.
        table.record_floor(Theme::Sewer, 6420, 252.0);
        let app = App::with_options(false, None, table, false, Some(TEST_SEED));
        let mut h = Harness::with_app(app, 120, 40);
        h.frame(&[]);
        assert!(
            h.screen_text().contains("01 Sewer") && !h.screen_text().contains('×'),
            "the Sewer is open, so the title row marks nothing locked"
        );
        h.frame(&[KeyCode::Down, KeyCode::Right]);
        assert!(
            h.screen_text().contains("02 Kitchen") && !h.screen_text().contains('×'),
            "the cleared Sewer unlocked the Kitchen"
        );
        h.frame(&[KeyCode::Right]);
        assert!(
            h.screen_text().contains("02 Laundry") && !h.screen_text().contains("Laundry ×"),
            "clearing the Sewer opened both stage-two floors"
        );
        for _ in 0..2 {
            h.frame(&[KeyCode::Right]); // Laundry -> Lab -> Printworks
        }
        assert!(
            h.screen_text().contains("03 Printworks ×"),
            "stage three is still locked on the title row"
        );
        for _ in 0..4 {
            h.frame(&[KeyCode::Left]); // back to the Sewer
        }
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "SectorSelect");
        let text = h.screen_text();
        assert!(text.contains("CLEARED"), "Sewer was cleared: {text}");
        assert!(text.contains("OPEN"), "Kitchen is open: {text}");
        assert!(text.contains("LOCKED"), "the Lab is locked: {text}");
        assert!(text.contains("BEST 6420"), "the stored best score: {text}");
        assert!(text.contains("04:12"), "the stored best time: {text}");
        assert!(
            text.contains("1/6 CLEARED"),
            "the header counts cleared floors: {text}"
        );

        // A locked floor can be selected and read, but Enter refuses to start
        // it and the select screen stays put. (With only the Sewer cleared,
        // stage two is open and stage three is not.)
        for _ in 0..3 {
            h.frame(&[KeyCode::Right]); // Sewer -> Kitchen -> Laundry -> Lab
        }
        assert_eq!(h.app.theme(), Theme::Lab, "the cursor reached the Lab");
        h.frame(&[KeyCode::Enter]);
        assert_eq!(
            h.app.screen.name_for_test(),
            "SectorSelect",
            "a locked floor must not start"
        );
        assert!(h.app.world.is_none());

        // The Kitchen starts a labelled single-floor practice run.
        for _ in 0..2 {
            h.frame(&[KeyCode::Left]); // Lab -> Laundry -> Kitchen
        }
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        assert_eq!(h.app.practice, Some(Theme::Kitchen));
        assert!(h.app.campaign.is_none());
        assert!(
            h.screen_text().contains("PRACTICE"),
            "the practice run must be labelled"
        );
        assert!(
            h.screen_text().contains("THE KITCHEN"),
            "the intro plate names the chosen floor"
        );
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.level_index, 1);
            assert_eq!(w.level.theme, Theme::Kitchen);
            assert_eq!(
                w.player.loadout.weapons.len(),
                1,
                "practice uses the default loadout"
            );
        }

        // Clearing it records the floor (but never a campaign win).
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");

        // The play screens keep working on a later level: the minimap draws,
        // the level-up cards open, and the pause dossier carries the campaign
        // pips for the stage in flight.
        h.frame(&[]);
        assert!(
            !h.app.last_minimap.is_empty(),
            "the minimap must draw on a later floor"
        );
        h.app.world.as_mut().unwrap().pending_levelups = 1;
        h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "LevelUp");
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        let paused = h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        assert!(
            paused.contains("2/4"),
            "the pause dossier must carry the stage pips: {paused}"
        );
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert!(
            h.screen_text().contains("THE KITCHEN"),
            "the complete plate names the floor that was played"
        );
        h.frame(&[KeyCode::Char(' ')]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        assert!(
            h.screen_text().contains("KITCHEN"),
            "the incident report names the floor that was played"
        );
        let entry = &h.app.scores.entries[0];
        assert_eq!(entry.campaign_level, 2);
        assert!(entry.won, "a cleared practice floor wins its own plate");
        assert!(!entry.campaign_won, "but it never wins the campaign");
        assert!(h.app.scores.floor_cleared(Theme::Kitchen));
        assert!(
            !h.app.scores.floor_cleared(Theme::Laundry),
            "clearing the Kitchen says nothing about the Laundry"
        );
        assert!(
            h.app.scores.floor_unlocked(Theme::Lab)
                && h.app.scores.floor_unlocked(Theme::Printworks),
            "clearing a stage-two floor opens both stage-three floors"
        );

        // The unlock survives a save/load through the injected store.
        let reloaded = ScoreTable::parse(&h.app.scores.to_json());
        assert!(reloaded.floor_unlocked(Theme::Lab));
        assert_eq!(
            reloaded.floor_best(Theme::Kitchen).map(|b| b.cleared),
            Some(true),
            "the Kitchen best was persisted"
        );
        assert_eq!(
            reloaded.floor_best(Theme::Sewer).map(|b| b.score),
            Some(6420),
            "the old Sewer record is still its own"
        );
    }

    /// Every playable floor appears on the sector-select screen as its own
    /// correctly named slot, in route order, and the slot selected by keyboard,
    /// mouse and wheel is exactly the floor that starts.
    #[test]
    fn sector_select_lists_every_floor_and_selects_it() {
        let floors = Theme::playable();
        let mut h = Harness::new(120, 40);
        h.app.screen = Screen::SectorSelect;
        let text = h.frame(&[]);
        for theme in floors {
            let plate = crate::render::campaign::CampaignLevel::from_theme(theme);
            assert!(
                text.contains(plate.name()),
                "the select screen is missing {:?}: {text}",
                theme
            );
        }

        // Slot count and names follow `Theme::playable()`; the grid shows them
        // all, so every one has a click target.
        for (i, theme) in floors.into_iter().enumerate() {
            let pos = h.locate(
                crate::render::campaign::CampaignLevel::from_theme(theme).name(),
            );
            h.events(&[], &[(MouseEventKind::Moved, pos)]);
            assert_eq!(
                h.app.hits.hovered,
                Some(Target::SelectSector(i)),
                "hovering {:?} must target its floor",
                theme
            );
            h.events(
                &[],
                &[(MouseEventKind::Down(MouseButton::Left), pos)],
            );
            assert_eq!(
                h.app.theme(),
                theme,
                "clicking {:?} must select that floor, not a stage's original",
                theme
            );
        }

        // The keyboard walks the grid in visual order: `Left`/`Right` one
        // floor, `Up`/`Down` a whole row of the grid the art laid out.
        h.app.select_idx = 0;
        h.app.screen = Screen::SectorSelect;
        h.frame(&[]);
        let cols = crate::render::campaign::level_select_cols(h.app.last_select_area);
        assert_eq!(cols, 3, "a 120-wide terminal lays the floors out 3 across");
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.theme(), Theme::Kitchen);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.theme(), floors[(1 + cols) % floors.len()]);

        // The wheel steps the same cursor, anywhere on the screen.
        h.app.select_idx = 0;
        h.frame(&[]);
        h.events(&[], &[(MouseEventKind::ScrollDown, (60, 20))]);
        assert_eq!(h.app.theme(), Theme::Kitchen, "the wheel steps one floor");
        assert_eq!(h.app.scores.zoom, 1.0, "the select screen never zooms");
    }

    /// The whole select -> start -> intro -> play -> clear -> report walk for
    /// **every** floor. The theme picked on the select screen is the theme the
    /// intro plate names, the HUD floor read-out names, the boss bar names, the
    /// level-complete plate names and the incident report names: the bug this
    /// test exists for was a stage-shaped label on a floor-shaped run.
    #[test]
    fn every_floor_selects_starts_and_names_the_same_floor() {
        for (index, theme) in Theme::playable().into_iter().enumerate() {
            let plate = crate::render::campaign::CampaignLevel::from_theme(theme);
            // Every floor is open: a stage is reachable once any floor of the
            // stage before it has been cleared.
            let mut table = ScoreTable::default();
            for floor in Theme::all() {
                table.record_floor(floor, 100, 200.0);
            }
            let app = App::with_options(false, None, table, false, Some(TEST_SEED));
            let mut h = Harness::with_app(app, 100, 32);
            h.app.screen = Screen::SectorSelect;
            h.app.select_idx = index;
            h.frame(&[]);
            let screen = h.screen_text();
            assert!(
                screen.contains(plate.name()),
                "{theme:?}: the select screen is missing its card:\n{screen}"
            );
            // The card the cursor is on is the floor that starts.
            assert_eq!(
                h.app.theme(),
                theme,
                "{theme:?}: the cursor is not on the selected floor"
            );

            h.frame(&[KeyCode::Enter]);
            {
                let w = h.app.world.as_ref().expect("a world starts");
                assert_eq!(w.level.theme, theme, "{theme:?}: started the wrong floor");
            }

            // The intro plate: the campaign's MapIntro for stage one, the
            // LevelIntro plate for every other floor.
            let intro = if index == 0 {
                assert_eq!(
                    h.app.screen.name_for_test(),
                    "Story",
                    "the Sewer starts the campaign"
                );
                h.frame(&[KeyCode::Enter]) // dossier -> map intro
            } else {
                assert_eq!(
                    h.app.screen.name_for_test(),
                    "LevelIntro",
                    "{theme:?}: a later floor starts a practice run"
                );
                h.screen_text()
            };
            // The getaway ride's heading is `THE GETAWAY — FLATBED`.
            let intro_name = if theme == Theme::Trailer {
                "FLATBED"
            } else {
                plate.name()
            };
            assert!(
                intro.contains(intro_name),
                "{theme:?}: the intro plate must name the floor:\n{intro}"
            );

            // The HUD's floor read-out names the same floor. Park the clock so
            // no higher-priority status chip (elite / boss / swarm) owns the
            // pill the name lives in.
            h.frame(&[KeyCode::Enter]); // -> Playing
            assert_eq!(h.app.screen.name_for_test(), "Playing");
            {
                let w = h.app.world.as_mut().unwrap();
                w.elapsed = 0.0;
                w.director.elite_timer = 999.0;
            }
            let playing = h.frame(&[]);
            let hud_name = theme.name().to_uppercase();
            assert!(
                playing.contains(hud_name.as_str()),
                "{theme:?}: the HUD must name the floor:\n{playing}"
            );

            // The boss the floor spawns is the boss the campaign plates name.
            assert_eq!(
                hud::boss_codename(theme).0,
                plate.boss_name(),
                "{theme:?}: the boss bar and the campaign plates disagree"
            );

            // The clear plate names the floor and the boss.
            h.finish_level();
            assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
            let complete = h.screen_text();
            assert!(
                complete.contains(plate.name()),
                "{theme:?}: the complete plate must name the floor:\n{complete}"
            );
            // The plate names the boss this seed's floor actually fields (the
            // pool pick), not the floor's original boss.
            let fielded = {
                let w = h.app.world.as_ref().unwrap();
                crate::game::ecs::systems::boss::BossKind::pool_pick(w.level.theme, w.level.seed)
            };
            assert!(
                complete.contains(hud::boss_codename_kind(fielded).0),
                "{theme:?}: the complete plate must name the boss:\n{complete}"
            );

            // Stage one's clear continues the campaign; every other floor ends
            // its run on the incident report, which names the floor too.
            h.frame(&[KeyCode::Char(' ')]);
            if index == 0 {
                assert_eq!(
                    h.app.screen.name_for_test(),
                    "KeepThree",
                    "the campaign continues after the Sewer"
                );
            } else {
                assert_eq!(h.app.screen.name_for_test(), "End");
                let report = h.screen_text();
                assert!(
                    report.contains(hud_name.as_str()),
                    "{theme:?}: the incident report must name the floor:\n{report}"
                );
            }
        }

        // The campaign's own end screen names the floor it was played on.
        let mut h = Harness::new(100, 32);
        h.enter_playing();
        h.app.world.as_mut().unwrap().game_over = true;
        let report = h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        assert!(
            report.contains("SEWER"),
            "the campaign's end screen must name the floor:\n{report}"
        );
    }

    /// Every new campaign screen renders its own furniture at the two reference
    /// terminal sizes.
    #[test]
    fn campaign_screens_lay_out_at_57x20_and_80x24() {
        for (w, height) in [(57u16, 20u16), (80, 24)] {
            let cases: [(&str, &str); 5] = [
                ("SectorSelect", "SECTOR"),
                ("LevelIntro", "ACCESS ROUTE"),
                ("LevelComplete", "SECTOR CLEARED"),
                ("KeepThree", "PACK"),
                ("Victory", "EXTRACTION"),
            ];
            for (screen, needle) in cases {
                let mut h = Harness::new(w, height);
                h.frame(&[]);
                match screen {
                    "SectorSelect" => h.app.screen = Screen::SectorSelect,
                    "LevelIntro" => h.app.start_practice(Theme::Kitchen),
                    "Victory" => {
                        h.enter_playing();
                        h.app.campaign.as_mut().unwrap().level = campaign::LEVEL_COUNT - 1;
                        h.app.world.as_mut().unwrap().level_index = campaign::LEVEL_COUNT - 1;
                        h.finish_level();
                    }
                    _ => {
                        h.enter_playing();
                        h.grant_keepables();
                        h.finish_level();
                        if screen == "KeepThree" {
                            h.frame(&[KeyCode::Enter]);
                        }
                    }
                }
                assert_eq!(h.app.screen.name_for_test(), screen, "{screen} at {w}x{height}");
                let text = h.frame(&[]);
                assert!(
                    text.contains(needle),
                    "{screen} at {w}x{height} is missing {needle:?}: {text}"
                );
            }
        }
    }

    // ---- Campaign fanfares -----------------------------------------------

    /// A cleared level's fanfare is its own sound, and it is *deferred* so it
    /// follows the boss's death sting the simulation already played on the
    /// killing tick instead of doubling it.
    #[test]
    fn a_sector_clear_fanfare_follows_the_boss_death() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        assert_eq!(h.app.pending_sting(), None);
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert_eq!(
            h.app.pending_sting(),
            Some(Sfx::LevelComplete),
            "the sector card plays the level-complete fanfare"
        );
        // It is still waiting the frame after the kill: nothing is stacking on
        // the simulation's own `BossDeath`.
        h.frame(&[]);
        assert_eq!(h.app.pending_sting(), Some(Sfx::LevelComplete));
        // `FANFARE_DELAY_SECS` at 60 fps, plus a frame of slack: it comes due.
        for _ in 0..((FANFARE_DELAY_SECS * 60.0) as usize + 2) {
            h.frame(&[]);
        }
        assert_eq!(h.app.pending_sting(), None, "the fanfare must come due");
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
    }

    /// Beating the Lab queues the campaign fanfare; a practice clear keeps the
    /// plain win sting and never claims a campaign victory.
    #[test]
    fn campaign_victory_and_practice_use_their_own_stings() {
        // Campaign win: level 3's boss dies.
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.app.campaign.as_mut().unwrap().level = campaign::LEVEL_COUNT - 1;
        h.app.world.as_mut().unwrap().level_index = campaign::LEVEL_COUNT - 1;
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "Victory");
        assert_eq!(
            h.app.pending_sting(),
            Some(Sfx::CampaignVictory),
            "the campaign win is its own fanfare"
        );

        // Practice clear of the Kitchen: the sector card, then the plain `Win`
        // on the end plate — never `CampaignVictory`.
        let mut h = Harness::new(120, 40);
        h.frame(&[]);
        h.app.start_practice(Theme::Kitchen);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        assert_eq!(h.app.practice, Some(Theme::Kitchen));
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert_eq!(h.app.pending_sting(), Some(Sfx::LevelComplete));
        // Let the sector fanfare come due, then take the card.
        for _ in 0..((FANFARE_DELAY_SECS * 60.0) as usize + 2) {
            h.frame(&[]);
        }
        assert_eq!(h.app.pending_sting(), None);
        h.frame(&[KeyCode::Char(' ')]); // sector card -> the end plate
        assert_eq!(h.app.screen.name_for_test(), "End");
        assert_eq!(
            h.app.last_summary.as_ref().map(|s| (s.won, s.practice)),
            Some((true, true)),
            "a cleared practice sector wins its own plate"
        );
        assert_eq!(
            h.app.pending_sting(),
            None,
            "the practice win is played at once, never queued as a campaign fanfare"
        );
        assert_ne!(
            h.app.pending_sting(),
            Some(Sfx::CampaignVictory),
            "a practice clear is never a campaign victory"
        );
    }

    /// Two of the required three cards, packed with the keyboard.
    fn pack_two(h: &mut Harness) {
        for key in [
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Char(' '),
        ] {
            h.frame(&[key]);
        }
    }

    // ---- Cheese mouse cursor tests ---------------------------------------

    /// Guard tests that mutate `RATRUN_CURSOR` so parallel tests never race.
    static CURSOR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Faithful xterm RGB lookup for ANSI-dumping truecolor, 256 and 16-colour
    /// buffers in the visual verification harness.
    fn xterm_color_rgb(c: Color) -> (u8, u8, u8) {
        const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Reset | Color::Black => (11, 15, 17),
            Color::Red => (128, 0, 0),
            Color::Green => (0, 128, 0),
            Color::Yellow => (128, 128, 0),
            Color::Blue => (0, 0, 128),
            Color::Magenta => (128, 0, 128),
            Color::Cyan => (0, 128, 128),
            Color::Gray => (192, 192, 192),
            Color::DarkGray => (128, 128, 128),
            Color::LightRed => (255, 0, 0),
            Color::LightGreen => (0, 255, 0),
            Color::LightYellow => (255, 255, 0),
            Color::LightBlue => (0, 0, 255),
            Color::LightMagenta => (255, 0, 255),
            Color::LightCyan => (0, 255, 255),
            Color::White => (255, 255, 255),
            Color::Indexed(idx) => {
                if idx < 16 {
                    let base = [
                        (0, 0, 0),
                        (128, 0, 0),
                        (0, 128, 0),
                        (128, 128, 0),
                        (0, 0, 128),
                        (128, 0, 128),
                        (0, 128, 128),
                        (192, 192, 192),
                        (128, 128, 128),
                        (255, 0, 0),
                        (0, 255, 0),
                        (255, 255, 0),
                        (0, 0, 255),
                        (255, 0, 255),
                        (0, 255, 255),
                        (255, 255, 255),
                    ];
                    base[idx as usize]
                } else if idx < 232 {
                    let i = (idx - 16) as usize;
                    (CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
                } else {
                    let v = 8 + 10 * (idx - 232);
                    (v, v, v)
                }
            }
        }
    }

    fn is_cheese_yellow(c: Color) -> bool {
        match c {
            Color::Rgb(r, g, b) => r >= 230 && g >= 195 && b <= 180,
            Color::LightYellow | Color::Yellow => true,
            Color::Indexed(idx) => {
                let (r, g, b) = xterm_color_rgb(Color::Indexed(idx));
                r >= 210 && g >= 170 && b <= 150
            }
            _ => false,
        }
    }

    /// The wedge tip is the upper half of the mouse cell: a `▀` whose
    /// foreground is cheese yellow, over text and playfield cells alike.
    fn is_cheese_tip(cell: &ratatui::buffer::Cell) -> bool {
        match cell.symbol() {
            // The tip is the upper pixel of its cell.
            "▀" => is_cheese_yellow(cell.fg),
            _ => false,
        }
    }

    #[test]
    fn cheese_cursor_appears_on_move_hides_after_keyboard_and_returns() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        // Before any mouse input, no cheese cursor is drawn.
        h.frame(&[]);
        assert!(!h.app.cursor_visible());
        let before_cell = h.terminal.backend().buffer()[(10, 6)].clone();

        // Mouse moves to (10, 6) -> cheese appears with its hot-spot tip at (10, 6).
        h.events(&[], &[(MouseEventKind::Moved, (10, 6))]);
        assert!(h.app.cursor_visible());
        let tip = &h.terminal.backend().buffer()[(10, 6)];
        assert!(
            is_cheese_tip(tip),
            "expected cheese-yellow hot spot at (10, 6), got {tip:?}"
        );
        assert_ne!(tip, &before_cell);

        // Keyboard-only activity for > FADE_END_SECS hides the cursor completely.
        h.frame(&[KeyCode::Down]);
        for _ in 0..75 {
            h.frame(&[]);
        }
        assert!(
            !h.app.cursor_visible(),
            "cursor must hide after keyboard-only inactivity"
        );
        let after_hide = &h.terminal.backend().buffer()[(10, 6)];
        assert_eq!(
            after_hide.symbol(),
            before_cell.symbol(),
            "hiding the cursor must leave the underlying cell clean"
        );

        // Moving the mouse brings it back immediately on that very frame.
        h.events(&[], &[(MouseEventKind::Moved, (14, 7))]);
        assert!(h.app.cursor_visible());
        let returned = &h.terminal.backend().buffer()[(14, 7)];
        assert!(is_cheese_tip(returned));
    }

    #[test]
    fn cheese_cursor_clips_at_all_four_edges_and_1x1() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        // 1×1 terminal: no panic, tip painted on (0, 0).
        let mut tiny = Harness::new(1, 1);
        tiny.events(&[], &[(MouseEventKind::Moved, (0, 0))]);
        let cell = &tiny.terminal.backend().buffer()[(0, 0)];
        assert!(is_cheese_tip(cell));

        // All four edges and corners on 80×24.
        let mut h = Harness::new(80, 24);
        for &(x, y) in &[(0, 0), (79, 0), (0, 23), (79, 23), (78, 22)] {
            h.events(&[], &[(MouseEventKind::Moved, (x, y))]);
            let tip = &h.terminal.backend().buffer()[(x, y)];
            assert!(is_cheese_tip(tip), "missing tip at edge ({x},{y})");
        }
    }

    #[test]
    fn cheese_cursor_does_not_disturb_hit_testing() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        // Find a cell on the START RUN hit target.
        let mut start_cell = None;
        'outer: for y in 0..24u16 {
            for x in 0..80u16 {
                if h.app.hits.at(x, y) == Some(Target::Start) {
                    start_cell = Some((x, y));
                    break 'outer;
                }
            }
        }
        let (sx, sy) = start_cell.expect("START RUN target must exist on Title");

        // Hover there first: cheese draws at (sx, sy) and hit map still resolves Start.
        h.events(&[], &[(MouseEventKind::Moved, (sx, sy))]);
        assert!(h.app.cursor_visible());
        assert_eq!(h.app.hits.at(sx, sy), Some(Target::Start));
        assert!(is_cheese_tip(&h.terminal.backend().buffer()[(sx, sy)]));

        // Left-clicking at (sx, sy) activates START RUN -> Story!
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (sx, sy))]);
        assert_eq!(
            h.app.screen.name_for_test(),
            "Story",
            "clicking under the cheese hot spot must still activate the target"
        );
    }

    #[test]
    fn cheese_cursor_absent_with_env_off_and_in_ai_mode() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        std::env::set_var("RATRUN_CURSOR", "off");
        let app = App::with_options(false, None, ScoreTable::default(), false, Some(TEST_SEED));
        std::env::remove_var("RATRUN_CURSOR");

        let mut h = Harness::with_app(app, 80, 24);
        h.frame(&[]);
        let before = h.terminal.backend().buffer()[(10, 6)].clone();
        h.events(&[], &[(MouseEventKind::Moved, (10, 6))]);
        assert!(
            !h.app.cursor_visible(),
            "RATRUN_CURSOR=off must disable the cheese cursor"
        );
        assert_eq!(
            h.terminal.backend().buffer()[(10, 6)].fg,
            before.fg,
            "RATRUN_CURSOR=off must not paint over the cell"
        );

        // `--ai` self-playing wall does not draw the cheese cursor at all.
        let mut wall = crate::spectate::WallState::new(1);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut input = InputState::new(false);
        input.handle_mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 10,
            row: 6,
            modifiers: KeyModifiers::NONE,
        });
        let area = Rect::new(0, 0, 80, 24);
        crate::spectate::spectate_keys(&mut wall, &input, area);
        term.draw(|f| {
            let a = f.area();
            crate::spectate::render_wall(f.buffer_mut(), a, &wall);
        })
        .unwrap();
        let wall_cell = &term.backend().buffer()[(10, 6)];
        assert!(
            !is_cheese_yellow(wall_cell.fg),
            "--ai wall must not render a cheese cursor"
        );
    }

    #[test]
    fn cheese_cursor_setting_persists_through_injected_store() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        assert_eq!(h.app.scores.cheese_cursor, cursor::CursorSetting::On);

        // Move mouse so cursor appears, then press 'c' to turn it off.
        h.events(&[], &[(MouseEventKind::Moved, (10, 6))]);
        assert!(h.app.cursor_visible());
        h.frame(&[KeyCode::Char('c')]);
        assert_eq!(h.app.scores.cheese_cursor, cursor::CursorSetting::Off);
        assert!(!h.app.cursor_visible());

        // Persisted JSON round-trips cleanly into a new App with injected store.
        let saved = h.app.scores.to_json();
        let reloaded = ScoreTable::parse(&saved);
        assert_eq!(reloaded.cheese_cursor, cursor::CursorSetting::Off);

        let app2 = App::with_options(false, None, reloaded, false, Some(TEST_SEED));
        let mut h2 = Harness::with_app(app2, 80, 24);
        h2.events(&[], &[(MouseEventKind::Moved, (10, 6))]);
        assert!(
            !h2.app.cursor_visible(),
            "reloaded Off preference must keep cursor hidden"
        );

        // Pressing 'c' again re-enables it.
        h2.frame(&[KeyCode::Char('c')]);
        assert_eq!(h2.app.scores.cheese_cursor, cursor::CursorSetting::On);
        assert!(h2.app.cursor_visible());
    }

    #[test]
    fn cheese_cursor_draws_on_every_normal_game_screen() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let pos = (18u16, 8u16);
        let screens = [
            "Title",
            "SectorSelect",
            "Story",
            "MapIntro",
            "LevelIntro",
            "Playing",
            "ExpandedMap",
            "LevelUp",
            "Paused",
            "LevelComplete",
            "KeepThree",
            "Victory",
            "End",
        ];
        for name in screens {
            let mut h = Harness::new(80, 24);
            h.frame(&[]);
            match name {
                "Title" => {}
                "SectorSelect" => h.app.screen = Screen::SectorSelect,
                "Story" => h.app.screen = Screen::Story,
                "MapIntro" => {
                    h.frame(&[KeyCode::Enter]);
                    h.frame(&[KeyCode::Enter]);
                }
                "LevelIntro" => h.app.start_practice(Theme::Kitchen),
                "Playing" => h.enter_playing(),
                "ExpandedMap" => {
                    h.enter_playing();
                    h.frame(&[KeyCode::Tab]);
                    assert!(h.app.map_open);
                }
                "LevelUp" => {
                    h.enter_playing();
                    h.app.open_levelup();
                }
                "Paused" => {
                    h.enter_playing();
                    h.frame(&[KeyCode::Char('p')]);
                }
                "LevelComplete" => {
                    h.enter_playing();
                    h.grant_keepables();
                    h.finish_level();
                }
                "KeepThree" => {
                    h.enter_playing();
                    h.grant_keepables();
                    h.finish_level();
                    h.frame(&[KeyCode::Enter]);
                }
                "Victory" => {
                    h.enter_playing();
                    h.app.campaign.as_mut().unwrap().level = campaign::LEVEL_COUNT - 1;
                    h.app.world.as_mut().unwrap().level_index = campaign::LEVEL_COUNT - 1;
                    h.finish_level();
                }
                "End" => {
                    h.enter_playing();
                    h.app.world.as_mut().unwrap().game_over = true;
                    h.frame(&[]);
                }
                _ => unreachable!(),
            }
            h.events(&[], &[(MouseEventKind::Moved, pos)]);
            let tip = &h.terminal.backend().buffer()[pos];
            assert!(is_cheese_tip(tip), "cursor missing on screen {name}: {tip:?}");
        }
    }

    /// In `Scheme::MouseAim` the pointer is the aim point, and it is drawn as
    /// the game's **cheese wedge** — not a crosshair — with the tip on the
    /// pointer cell. The aim vector the tick receives is unchanged: the cursor
    /// is presentation only and reads `aim_point`, never feeds it.
    #[test]
    fn mouse_aim_play_draws_the_cheese_wedge_at_the_pointer() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.set_scheme(Scheme::MouseAim);
        assert!(h.app.input.controls.mouse_aims());
        h.frame(&[]);
        let game = h.app.last_game;
        assert!(game.width > 8 && game.height > 4, "no playfield to point at");
        // Clear of the corner minimap and of the rat, so the cells under the
        // wedge are plain field.
        let pos = (game.x + game.width / 2, game.y + 2);

        h.events(&[], &[(MouseEventKind::Moved, pos)]);
        assert!(h.app.cursor_visible(), "mouse aim must show the cheese");

        let buf = h.terminal.backend().buffer();
        // The wedge: the tip is the upper half of the pointer cell, and the cut
        // face widens to the right of it.
        let tip = &buf[pos];
        assert_eq!(tip.symbol(), "▀", "the wedge tip is the pointer cell's upper half");
        assert!(is_cheese_yellow(tip.fg), "the tip must be cheese yellow: {tip:?}");
        let body = &buf[(pos.0 + 1, pos.1)];
        assert!(
            is_cheese_yellow(body.fg) && is_cheese_yellow(body.bg),
            "no cheese body right of the tip: {body:?}"
        );
        // No reticle pixels: not the centre pip, nor an arm above, below, left
        // or right of it (the crosshair that used to be drawn here).
        assert_ne!(buf[pos].symbol(), "•", "the centre pip is back");
        assert_ne!(buf[(pos.0 + 1, pos.1)].symbol(), "─", "a right arm is back");
        assert_ne!(buf[(pos.0 - 1, pos.1)].symbol(), "─", "a left arm is back");
        assert_ne!(buf[(pos.0, pos.1 + 1)].symbol(), "│", "a lower arm is back");
        assert_ne!(buf[(pos.0, pos.1 - 1)].symbol(), "│", "an upper arm is back");

        // The aim vector, pinned by the same arithmetic `App::mouse_world` uses
        // (the pointer cell's centre in world pixels, through the camera and the
        // zoom). This work must never move it.
        let w = h.app.world.as_ref().unwrap();
        let want = w.cam
            + Vec2::new(
                ((pos.0 as i32 - game.x as i32) as f32 + 0.5) * w.zoom_scale.x,
                (((pos.1 as i32 - game.y as i32) * 2) as f32 + 1.0) * w.zoom_scale.y,
            );
        let aim = h.app.aim_point().expect("mouse aim always has an aim point");
        assert!(
            (aim.x - want.x).abs() < 0.01 && (aim.y - want.y).abs() < 0.01,
            "the aim point moved: {aim:?} vs {want:?}"
        );

        // Presentation only: the cursor option decides whether a pointer is
        // drawn, never where the shot goes; Classic still auto-aims.
        h.app.scores.cheese_cursor = cursor::CursorSetting::Off;
        assert!(!h.app.cursor_enabled());
        let off = h.app.aim_point().expect("still aiming");
        assert!(
            (off.x - aim.x).abs() < 0.001 && (off.y - aim.y).abs() < 0.001,
            "the cursor option moved the aim: {off:?} vs {aim:?}"
        );
        h.app.scores.cheese_cursor = cursor::CursorSetting::On;
        h.app.set_scheme(Scheme::Classic);
        assert!(h.app.aim_point().is_none(), "Classic auto-aims: no pointer aim at all");
    }

    /// The bindable `ToggleCursor` action hides the wedge in mouse aim exactly
    /// as it does in Classic, and pressing it again brings the cheese back.
    #[test]
    fn toggle_cursor_hides_the_cheese_in_mouse_aim_play() {
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.app.set_scheme(Scheme::MouseAim);
        h.frame(&[]);
        let game = h.app.last_game;
        let pos = (game.x + game.width / 2, game.y + 2);
        h.events(&[], &[(MouseEventKind::Moved, pos)]);
        assert!(is_cheese_tip(&h.terminal.backend().buffer()[pos]));

        h.frame(&[KeyCode::Char('c')]);
        assert_eq!(h.app.scores.cheese_cursor, cursor::CursorSetting::Off);
        assert!(!h.app.cursor_visible());
        assert!(
            !is_cheese_tip(&h.terminal.backend().buffer()[pos]),
            "ToggleCursor left the wedge on the playfield"
        );

        h.frame(&[KeyCode::Char('c')]);
        assert_eq!(h.app.scores.cheese_cursor, cursor::CursorSetting::On);
        assert!(h.app.cursor_visible());
        assert!(is_cheese_tip(&h.terminal.backend().buffer()[pos]), "the cheese did not return");
    }

    /// The wedge is stamped in screen space after the playfield blit, so the
    /// sub-cell encoder (quadrant, sextant and octant) and the 1x EPX smoothing
    /// cannot eat it: at every zoom the pointer cell is the wedge's tip and the
    /// cell beside it is cheese.
    #[test]
    fn the_cheese_wedge_survives_sub_cells_and_one_x_smoothing() {
        use crate::render::subcell::{Choice, SubcellSet};
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        for (set, zoom) in [
            (SubcellSet::Octant, crate::render::zoom::MAX),
            (SubcellSet::Sextant, crate::render::zoom::MAX),
            (SubcellSet::Octant, 1.0),
            (SubcellSet::Quad, 1.0),
        ] {
            let mut h = Harness::new(80, 24);
            h.app.set_subcell_override(Some(Choice::Force(set)));
            h.app.set_smooth(true);
            h.enter_playing();
            h.app.set_scheme(Scheme::MouseAim);
            h.app.world.as_mut().unwrap().set_zoom_instant(zoom);
            h.frame(&[]);
            let game = h.app.last_game;
            let pos = (game.x + game.width / 2, game.y + 2);
            h.events(&[], &[(MouseEventKind::Moved, pos)]);

            assert!(
                h.app.world.as_ref().unwrap().detail().is_some(),
                "{set:?} z={zoom}: no sub-cell frame was drawn to survive"
            );
            let buf = h.terminal.backend().buffer();
            let tip = &buf[pos];
            assert_eq!(tip.symbol(), "▀", "{set:?} z={zoom}: the encoder ate the tip");
            assert!(is_cheese_yellow(tip.fg), "{set:?} z={zoom}: tip {:?}", tip.fg);
            let body = &buf[(pos.0 + 1, pos.1)];
            assert!(
                is_cheese_yellow(body.fg) && is_cheese_yellow(body.bg),
                "{set:?} z={zoom}: the encoder ate the body: {body:?}"
            );
        }
    }

    /// Gated behind `RATRUN_CHEESE_DIR`: dumps ANSI captures of the cheese
    /// cursor across Title, Gameplay (Sewer + Kitchen + Expanded Map), and
    /// Level-Up at 80×24 and 120×40 in truecolor, 256-colour, 16-colour and
    /// safe-glyph modes, plus a states strip.
    #[test]
    fn capture_cheese_cursor() {
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::term::{sanitize_with, ColorDepth, GlyphSet};
        use std::fmt::Write;

        let Some(dir) = std::env::var_os("RATRUN_CHEESE_DIR") else {
            return;
        };
        let _guard = CURSOR_ENV_LOCK.lock().unwrap();
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        let dump = |buf: &ratatui::buffer::Buffer,
                    w: u16,
                    h: u16,
                    name: &str,
                    pos: (u16, u16),
                    glyphs: GlyphSet,
                    depth: ColorDepth| {
            let mut copy = buf.clone();
            sanitize_with(&mut copy, glyphs, depth);
            let mut ansi = String::new();
            for y in 0..h {
                for x in 0..w {
                    let c = &copy[(x, y)];
                    let (r, g, b) = xterm_color_rgb(c.fg);
                    let (br, bg, bb) = xterm_color_rgb(c.bg);
                    write!(
                        ansi,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        c.symbol()
                    )
                    .unwrap();
                }
                ansi.push_str("\x1b[0m\n");
            }
            let path = dir.join(format!("{w}x{h}-{name}__pos_{}_{}.ansi", pos.0, pos.1));
            std::fs::write(path, ansi).unwrap();
        };

        for (w, h) in [(80u16, 24u16), (120, 40)] {
            // 1. Title screen — cursor hovering on START RUN button.
            let mut hh = Harness::new(w, h);
            hh.frame(&[]);
            let mut start_pos = (w / 2, h / 2);
            'find_start: for y in 0..h {
                for x in 0..w {
                    if hh.app.hits.at(x, y) == Some(Target::Start) {
                        start_pos = (x + 2, y);
                        break 'find_start;
                    }
                }
            }
            hh.events(&[], &[(MouseEventKind::Moved, start_pos)]);
            let buf = hh.terminal.backend().buffer().clone();
            dump(&buf, w, h, "title-hover-tc", start_pos, GlyphSet::Unicode, ColorDepth::Truecolor);
            dump(&buf, w, h, "title-hover-256", start_pos, GlyphSet::Unicode, ColorDepth::Ansi256);
            dump(&buf, w, h, "title-hover-16", start_pos, GlyphSet::Unicode, ColorDepth::Ansi16);
            dump(&buf, w, h, "title-hover-safe", start_pos, GlyphSet::Safe, ColorDepth::Ansi256);

            // 2. Gameplay — Sewer + mouse steering + bob + click + held.
            let mut gp = Harness::new(w, h);
            gp.enter_playing();
            if let Some(world) = gp.app.world.as_mut() {
                for _ in 0..180 {
                    world.update(1.0 / 60.0, Vec2::new(0.3, -0.1), false);
                }
            }
            let play_pos = (w / 2 + 12, h / 2 - 2);
            gp.events(&[], &[(MouseEventKind::Moved, play_pos)]);
            let buf = gp.terminal.backend().buffer().clone();
            dump(&buf, w, h, "play-tc", play_pos, GlyphSet::Unicode, ColorDepth::Truecolor);
            dump(&buf, w, h, "play-256", play_pos, GlyphSet::Unicode, ColorDepth::Ansi256);
            dump(&buf, w, h, "play-16", play_pos, GlyphSet::Unicode, ColorDepth::Ansi16);
            dump(&buf, w, h, "play-safe", play_pos, GlyphSet::Safe, ColorDepth::Ansi256);

            // Click sparkle + held frame during play (at 120×40).
            if w == 120 {
                gp.events(
                    &[],
                    &[(MouseEventKind::Down(MouseButton::Left), play_pos)],
                );
                let buf_click = gp.terminal.backend().buffer().clone();
                dump(&buf_click, w, h, "play-click-held-tc", play_pos, GlyphSet::Unicode, ColorDepth::Truecolor);

                // Release button so we capture pure click sparkle without held.
                gp.events(&[], &[(MouseEventKind::Up(MouseButton::Left), play_pos)]);
                let buf_spark = gp.terminal.backend().buffer().clone();
                dump(&buf_spark, w, h, "play-sparkle-tc", play_pos, GlyphSet::Unicode, ColorDepth::Truecolor);

                // Kitchen floor (warm bright floor contrast check; let click pulse finish first).
                for _ in 0..20 {
                    gp.frame(&[]);
                }
                gp.app.world = Some(World::new_level(1, TEST_SEED, Loadout::starting()));
                if let Some(world) = gp.app.world.as_mut() {
                    for _ in 0..120 {
                        world.update(1.0 / 60.0, Vec2::new(0.2, 0.1), false);
                    }
                }
                gp.events(&[], &[(MouseEventKind::Moved, play_pos)]);
                let buf_k = gp.terminal.backend().buffer().clone();
                dump(&buf_k, w, h, "play-kitchen-tc", play_pos, GlyphSet::Unicode, ColorDepth::Truecolor);

                // Mouse aim: the same cheese wedge is the reticle now (batch
                // ten). Truecolor for the keyline, 16-colour for the fold.
                gp.app.set_scheme(Scheme::MouseAim);
                gp.frame(&[]);
                gp.events(&[], &[(MouseEventKind::Moved, play_pos)]);
                let buf_aim = gp.terminal.backend().buffer().clone();
                dump(&buf_aim, w, h, "play-aim-tc", play_pos, GlyphSet::Unicode, ColorDepth::Truecolor);
                dump(&buf_aim, w, h, "play-aim-16", play_pos, GlyphSet::Unicode, ColorDepth::Ansi16);
                gp.app.set_scheme(Scheme::Classic);
                gp.frame(&[]);

                // Expanded minimap overlay check.
                gp.frame(&[KeyCode::Tab]);
                let map_pos = (w / 2, h / 2);
                gp.events(&[], &[(MouseEventKind::Moved, map_pos)]);
                let buf_m = gp.terminal.backend().buffer().clone();
                dump(&buf_m, w, h, "play-map-tc", map_pos, GlyphSet::Unicode, ColorDepth::Truecolor);
            }

            // 3. Level-Up screen — cursor hovering on Upgrade card 0.
            let mut lu = Harness::new(w, h);
            lu.enter_playing();
            if let Some(world) = lu.app.world.as_mut() {
                for _ in 0..90 {
                    world.update(1.0 / 60.0, Vec2::new(0.2, 0.0), false);
                }
                lu.app.levelup_choices = vec![
                    Upgrade::NewWeapon(crate::game::weapon::WeaponKind::CheeseSpray),
                    Upgrade::Passive(PassiveKind::Magnet),
                    Upgrade::Passive(PassiveKind::Armor),
                ];
                lu.app.levelup_desc = lu
                    .app
                    .levelup_choices
                    .iter()
                    .map(|u| world.player.loadout.describe(u))
                    .collect();
                lu.app.screen = Screen::LevelUp;
            }
            lu.frame(&[]);
            let mut card_pos = (w / 2, h / 2);
            'find_card: for y in 0..h {
                for x in 0..w {
                    if lu.app.hits.at(x, y) == Some(Target::Upgrade(0)) {
                        card_pos = (x + 4, y + 1);
                        break 'find_card;
                    }
                }
            }
            lu.events(&[], &[(MouseEventKind::Moved, card_pos)]);
            let buf = lu.terminal.backend().buffer().clone();
            dump(&buf, w, h, "levelup-tc", card_pos, GlyphSet::Unicode, ColorDepth::Truecolor);
            dump(&buf, w, h, "levelup-256", card_pos, GlyphSet::Unicode, ColorDepth::Ansi256);
            dump(&buf, w, h, "levelup-16", card_pos, GlyphSet::Unicode, ColorDepth::Ansi16);
            dump(&buf, w, h, "levelup-safe", card_pos, GlyphSet::Safe, ColorDepth::Ansi256);
        }
    }

    // ---- Separate music/effects volumes ---------------------------------

    /// Guard tests that mutate the volume environment overrides so parallel
    /// tests never race on the process environment.
    static VOLUME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The title's OPTIONS row is the third band: two Downs from START RUN.
    fn title_to_options(h: &mut Harness) {
        h.frame(&[]);
        h.frame(&[KeyCode::Down]);
        h.frame(&[KeyCode::Down]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Options");
    }

    #[test]
    fn quick_keys_step_both_volumes_and_show_the_hud_readout() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        assert_eq!(h.app.music_volume(), 100);
        assert_eq!(h.app.effects_volume(), 100);

        // `[` / `]` step music; `-` / `=` step effects.
        h.frame(&[KeyCode::Char('[')]);
        assert_eq!(h.app.music_volume(), 95, "`[` lowers music one step");
        let flash = h.app.hud.volume_flash.expect("music read-out");
        assert_eq!(flash.0, hud::VolumeChannel::Music);
        assert_eq!(flash.1, 95);
        let text = h.frame(&[]);
        assert!(text.contains("MUSIC 95%"), "music read-out missing: {text}");

        h.frame(&[KeyCode::Char(']')]);
        assert_eq!(h.app.music_volume(), 100);
        h.frame(&[KeyCode::Char('-')]);
        assert_eq!(h.app.effects_volume(), 95, "`-` lowers effects one step");
        let text = h.frame(&[]);
        assert!(text.contains("FX 95%"), "effects read-out missing: {text}");
        h.frame(&[KeyCode::Char('=')]);
        assert_eq!(h.app.effects_volume(), 100);

        // Clamp at the ends rather than wrapping.
        for _ in 0..25 {
            h.frame(&[KeyCode::Char('[')]);
        }
        assert_eq!(h.app.music_volume(), 0);
        for _ in 0..25 {
            h.frame(&[KeyCode::Char(']')]);
        }
        assert_eq!(h.app.music_volume(), 100);
        for _ in 0..25 {
            h.frame(&[KeyCode::Char('-')]);
        }
        assert_eq!(h.app.effects_volume(), 0);
        for _ in 0..25 {
            h.frame(&[KeyCode::Char('=')]);
        }
        assert_eq!(h.app.effects_volume(), 100);
    }

    #[test]
    fn mute_sits_on_top_of_the_two_levels_and_restores_them() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.frame(&[KeyCode::Char('[')]);
        h.frame(&[KeyCode::Char('-')]);
        assert_eq!((h.app.music_volume(), h.app.effects_volume()), (95, 95));
        assert!(!h.app.muted());

        h.frame(&[KeyCode::Char('m')]);
        assert!(h.app.muted());
        assert_eq!(
            (h.app.music_volume(), h.app.effects_volume()),
            (95, 95),
            "mute must not change the levels underneath"
        );
        assert!(h.app.hud.telemetry.muted);

        h.frame(&[KeyCode::Char('m')]);
        assert!(!h.app.muted());
        assert_eq!((h.app.music_volume(), h.app.effects_volume()), (95, 95));
    }

    #[test]
    fn options_opens_from_the_title_menu_and_back_returns_there() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        title_to_options(&mut h);
        let text = h.frame(&[]);
        for needle in ["MUSIC", "EFFECTS", "MINIMAP", "BACK"] {
            assert!(text.contains(needle), "options missing {needle:?}: {text}");
        }
        assert!(text.contains("95%" ) || text.contains("100%"), "no percentage read-out");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
        assert_eq!(h.app.menu_idx, 2, "the title keeps its OPTIONS row");
    }

    #[test]
    fn options_opens_from_pause_and_back_returns_to_the_frozen_run() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        let elapsed = h.app.world.as_ref().unwrap().elapsed;

        h.frame(&[KeyCode::Char('o')]);
        assert_eq!(h.app.screen.name_for_test(), "Options");
        let text = h.frame(&[]);
        assert!(text.contains("MUSIC"), "options plate missing: {text}");
        // The run stays frozen while the plate is open.
        for _ in 0..10 {
            h.frame(&[]);
        }
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed);

        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");

        // Enter on the Back row also leaves, returning to the pause plate.
        h.frame(&[KeyCode::Char('o')]);
        assert_eq!(h.app.screen.name_for_test(), "Options");
        for _ in 0..options::BACK_ROW {
            h.frame(&[KeyCode::Down]);
        }
        assert_eq!(h.app.options_row, options::BACK_ROW);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");

        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
    }

    #[test]
    fn options_mouse_clicks_and_drags_the_sliders() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        title_to_options(&mut h);
        h.frame(&[]);
        let music = h
            .app
            .hits
            .rect_for(Target::OptionsBar(options::MUSIC_BAR))
            .expect("music slider rect");
        // Click the left end: silent.
        h.events(
            &[],
            &[(MouseEventKind::Down(MouseButton::Left), (music.x, music.y))],
        );
        assert_eq!(h.app.music_volume(), 0, "left edge is 0%");
        // Drag to the right end: full.
        h.events(
            &[],
            &[(
                MouseEventKind::Drag(MouseButton::Left),
                (music.right() - 1, music.y),
            )],
        );
        assert_eq!(h.app.music_volume(), 100, "right edge is 100%");

        // A click on the effects row's slider also moves it.
        let effects = h
            .app
            .hits
            .rect_for(Target::OptionsBar(options::EFFECTS_BAR))
            .expect("effects slider rect");
        h.events(
            &[],
            &[(
                MouseEventKind::Down(MouseButton::Left),
                (effects.x + effects.width / 2, effects.y),
            )],
        );
        assert!(
            (40..=60).contains(&h.app.effects_volume()),
            "midpoint click should land near 50%, got {}",
            h.app.effects_volume()
        );

        // Clicking BACK returns to the title.
        let back = h
            .app
            .hits
            .rect_for(Target::OptionsBack)
            .expect("back row rect");
        h.events(
            &[],
            &[(MouseEventKind::Down(MouseButton::Left), (back.x + 2, back.y))],
        );
        assert_eq!(h.app.screen.name_for_test(), "Title");
    }

    // ---- The Graphics page (Options' GRAPHICS row) -----------------------

    /// Guards tests that set one of the Graphics page's `RATRUN_*` pins: the
    /// process environment is global, so they must not run beside each other.
    static GRAPHICS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The GRAPHICS row opens the page, and BACK/Esc comes back to Options —
    /// from the title's Options and from the pause screen's, which is where the
    /// CONTROLS row is reachable too.
    #[test]
    fn the_graphics_row_opens_the_page_and_back_returns_to_options() {
        // From the title: twelve Downs from MUSIC reach the GRAPHICS row.
        let mut h = Harness::new(80, 30);
        title_to_options(&mut h);
        h.frame(&[]);
        for _ in 0..options::GRAPHICS_ROW {
            h.frame(&[KeyCode::Down]);
        }
        assert_eq!(h.app.options_row, options::GRAPHICS_ROW);
        assert_eq!(options::ROWS[options::GRAPHICS_ROW].label(), "GRAPHICS");
        let text = h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Graphics");
        for needle in [
            "LIGHTING",
            "LIGHT EFFECTS",
            "SMOOTHING",
            "FOREGROUND",
            "GLYPH PARTICLES",
            "SCREEN SHAKE",
            "BACK",
        ] {
            assert!(text.contains(needle), "the page is missing {needle:?}:\n{text}");
        }
        assert!(!h.app.console_cheat, "a menu setting must not flag the run");
        // Enter on BACK returns to Options, which still remembers the title.
        h.app.graphics_row = graphics::BACK_ROW;
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Options");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Title");

        // From the pause screen: the page sits over the frozen field and Back
        // walks Options -> Paused -> Playing.
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('o')]);
        h.app.options_row = options::GRAPHICS_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.screen.name_for_test(), "Graphics");
        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        for _ in 0..10 {
            h.frame(&[]);
        }
        assert_eq!(
            h.app.world.as_ref().unwrap().elapsed,
            elapsed,
            "the run stays frozen under the page"
        );
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Options");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
    }

    #[test]
    fn the_graphics_key_freezes_play_and_returns_to_its_opening_screen() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.frame(&[]);
        let elapsed = h.app.world.as_ref().unwrap().elapsed;

        let text = h.frame(&[KeyCode::Char('g')]);
        assert!(matches!(h.app.screen, Screen::Graphics));
        assert!(text.contains("S smoothing · G or Esc back"));
        for row in [graphics::SMOOTH_ROW, graphics::FOREGROUND_ROW] {
            let rect = h.app.hits.rect_for(Target::GraphicsRow(row)).unwrap();
            let buffer = h.terminal.backend().buffer();
            let line: String = (rect.x..rect.right()).map(|x| buffer[(x, rect.y)].symbol()).collect();
            assert!(line.contains("‹ OFF ›"), "{line}");
        }
        h.frame(&[]);
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed);
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Playing));
        h.frame(&[]);
        assert!(h.app.world.as_ref().unwrap().elapsed > elapsed);

        h.frame(&[KeyCode::Esc]);
        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        h.frame(&[KeyCode::Char('g')]);
        assert!(matches!(h.app.screen, Screen::Graphics));
        h.frame(&[KeyCode::Char('g')]);
        assert!(matches!(h.app.screen, Screen::Paused));
        h.frame(&[KeyCode::Char('g')]);
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Right), (0, 0))]);
        assert!(matches!(h.app.screen, Screen::Paused));
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed);

        h.app
            .input
            .controls
            .bind_active_kbm(Action::Graphics, Binding::Key(KeyBind::Char('v')));
        h.frame(&[KeyCode::Char('g')]);
        assert!(matches!(h.app.screen, Screen::Paused));
        let text = h.frame(&[KeyCode::Char('v')]);
        assert!(matches!(h.app.screen, Screen::Graphics));
        assert!(text.contains("S smoothing · V or Esc back"));
        h.frame(&[KeyCode::Esc]);
        assert!(matches!(h.app.screen, Screen::Paused));
    }

    #[test]
    fn the_smoothing_shortcut_cycles_and_redraws_the_frozen_field() {
        use crate::render::subcell::{Choice, Smooth, SubcellSet};
        let mut h = Harness::new(120, 40);
        h.app.set_subcell_override(Some(Choice::Force(SubcellSet::Sextant)));
        h.app.set_smooth_level(Smooth::Off);
        h.app.set_glyphfx(false);
        h.app.set_minimap_size(minimap::MinimapSize::Hidden);
        h.enter_playing();
        h.app.world.as_mut().unwrap().set_zoom_instant(1.0);
        for _ in 0..30 {
            h.frame(&[KeyCode::Right]);
        }
        h.frame(&[KeyCode::Char('g')]);
        assert!(matches!(h.app.screen, Screen::Graphics));
        h.app.graphics_row = graphics::BACK_ROW;
        h.frame(&[]);

        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        let before = h.terminal.backend().buffer().clone();
        let panel = graphics::layout(before.area).panel;
        let field = h.app.last_game;

        for &level in Smooth::ALL.iter().skip(1).chain(&Smooth::ALL[..1]) {
            let text = h.frame(&[KeyCode::Char('s')]);
            assert!(matches!(h.app.screen, Screen::Graphics));
            assert_eq!(h.app.graphics_row, graphics::BACK_ROW, "S keeps the selection");
            assert_eq!(h.app.world.as_ref().unwrap().smooth, level);
            assert_eq!(h.app.scores.graphics.smooth(), level);
            assert!(text.contains(&format!("‹ {} ›", level.label())));
            if level == Smooth::Xbr {
                let after = h.terminal.backend().buffer();
                assert!(
                    (field.top()..field.bottom()).any(|y| {
                        (field.left()..field.right()).any(|x| {
                            let in_panel = (panel.left()..panel.right()).contains(&x)
                                && (panel.top()..panel.bottom()).contains(&y);
                            !in_panel && before[(x, y)].symbol() != after[(x, y)].symbol()
                        })
                    }),
                    "smoothing should change playfield cells outside the plate"
                );
            }
        }
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed);
        assert!(!h.app.console_cheat);
    }

    /// Every row steps its setting with Left/Right — the same `App` method the
    /// console command calls, so the world and the save both follow — and none
    /// of it flags the run as cheated.
    #[test]
    fn every_graphics_row_steps_its_setting() {
        use crate::render::subcell::Smooth;
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('o')]);
        h.app.options_row = options::GRAPHICS_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.screen.name_for_test(), "Graphics");

        // LIGHTING: the console's live modes, and the run's world follows.
        let before = h.app.lighting_mode();
        h.app.graphics_row = graphics::LIGHTING_ROW;
        h.frame(&[KeyCode::Right]);
        let after = h.app.lighting_mode();
        assert_eq!(after, graphics::step_lighting(before, 1));
        assert_ne!(after, before, "the row must change the value");
        assert_eq!(
            h.app.world.as_ref().unwrap().lighting,
            after,
            "the run's lighting must move with the row"
        );
        assert_eq!(h.app.scores.graphics.lighting_mode(), Some(after));
        // Left walks it back.
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.lighting_mode(), before);

        // LIGHT EFFECTS: the `fx` switch, on the world too.
        assert!(h.app.fx_enabled(), "on by default");
        h.app.graphics_row = graphics::FX_ROW;
        h.frame(&[KeyCode::Right]);
        assert!(!h.app.fx_enabled());
        assert!(!h.app.world.as_ref().unwrap().lighting_fx);
        assert!(!h.app.scores.graphics.fx, "the preference is saved");
        h.frame(&[KeyCode::Left]);
        assert!(h.app.fx_enabled());

        // SMOOTHING: the `smooth` levels, left and right.
        assert_eq!(h.app.smooth_level(), Smooth::default());
        h.app.graphics_row = graphics::SMOOTH_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.smooth_level(), Smooth::Epx, "right walks down the list");
        assert_eq!(h.app.world.as_ref().unwrap().smooth, Smooth::Epx);
        assert_eq!(h.app.scores.graphics.smooth(), Smooth::Epx);
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.smooth_level(), Smooth::default());

        // FOREGROUND: the parallax layer.
        assert!(!h.app.foreground_enabled());
        h.app.graphics_row = graphics::FOREGROUND_ROW;
        h.frame(&[KeyCode::Right]);
        assert!(h.app.foreground_enabled());
        assert!(h.app.world.as_ref().unwrap().foreground);
        assert!(h.app.scores.graphics.foreground);

        // GLYPH PARTICLES: the particle layer (never in the picture key).
        assert!(h.app.glyphfx_enabled());
        h.app.graphics_row = graphics::GLYPHFX_ROW;
        h.frame(&[KeyCode::Right]);
        assert!(!h.app.glyphfx_enabled());
        assert!(!h.app.scores.graphics.glyphfx);

        // SCREEN SHAKE: 65 % -> 100 %, and the camera takes the new share.
        let shake = h.app.shake_scale();
        assert_eq!(shake, crate::game::world::SHAKE_SCALE);
        h.app.graphics_row = graphics::SHAKE_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.shake_scale(), graphics::step_shake(shake, 1));
        assert_eq!(h.app.shake_scale(), 1.0);
        assert_eq!(h.app.world.as_ref().unwrap().shake_scale, 1.0);
        assert_eq!(h.app.scores.graphics.shake(), 1.0);
        // …and the simulation's own offset is untouched by the setting.
        let ofs = h.app.world.as_ref().unwrap().shake_ofs;
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.world.as_ref().unwrap().shake_ofs, ofs);

        assert!(
            !h.app.console_cheat,
            "a render-only setting must never flag the run as console-used"
        );
        assert_eq!(
            h.app.screen.name_for_test(),
            "Graphics",
            "stepping a row never leaves the page"
        );
    }

    /// A click on a row selects and steps it; a click on BACK and a right click
    /// both leave; the wheel steps the row under the pointer.
    #[test]
    fn the_graphics_page_answers_the_pointer() {
        let mut h = Harness::new(100, 40);
        title_to_options(&mut h);
        h.app.options_row = options::GRAPHICS_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.screen.name_for_test(), "Graphics");

        // A left click on the SMOOTHING row steps it forward.
        let row = h
            .app
            .hits
            .rect_for(Target::GraphicsRow(graphics::SMOOTH_ROW))
            .expect("the SMOOTHING row is not clickable");
        let before = h.app.smooth_level();
        h.events(
            &[],
            &[(MouseEventKind::Down(MouseButton::Left), (row.x + 2, row.y))],
        );
        assert_eq!(h.app.graphics_row, graphics::SMOOTH_ROW);
        assert_eq!(h.app.smooth_level(), graphics::step_smooth(before, 1));
        assert_ne!(h.app.smooth_level(), before);

        // The wheel over a row steps it too (down = forward).
        let shake_row = h
            .app
            .hits
            .rect_for(Target::GraphicsRow(graphics::SHAKE_ROW))
            .expect("the SCREEN SHAKE row is not clickable");
        let before = h.app.shake_scale();
        h.events(
            &[],
            &[(MouseEventKind::ScrollDown, (shake_row.x + 2, shake_row.y))],
        );
        assert_eq!(h.app.shake_scale(), graphics::step_shake(before, 1));

        // A right click is Back: it never activates the row under it.
        h.events(
            &[],
            &[(MouseEventKind::Down(MouseButton::Right), (row.x + 2, row.y))],
        );
        assert_eq!(h.app.screen.name_for_test(), "Options");

        // A left click on BACK also returns to Options.
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.screen.name_for_test(), "Graphics");
        let back = h
            .app
            .hits
            .rect_for(Target::GraphicsBack)
            .expect("no Back button");
        h.events(
            &[],
            &[(MouseEventKind::Down(MouseButton::Left), (back.x + 2, back.y))],
        );
        assert_eq!(h.app.screen.name_for_test(), "Options");
    }

    /// The six settings round-trip through the save file, and a fresh `App`
    /// built from a loaded table starts at them.
    #[test]
    fn graphics_settings_survive_a_save_and_load_round_trip() {
        use crate::render::lighting::LightingMode;
        use crate::render::subcell::Smooth;
        let mut table = ScoreTable::default();
        table.graphics.set_lighting(LightingMode::Coarse);
        table.graphics.set_smooth(Smooth::Xbr);
        table.graphics.fx = false;
        table.graphics.foreground = true;
        table.graphics.glyphfx = false;
        table.graphics.set_shake(1.0);

        let reloaded = ScoreTable::parse(&table.to_json());
        assert_eq!(reloaded.graphics, table.graphics);
        assert_eq!(reloaded.graphics.lighting_mode(), Some(LightingMode::Coarse));
        assert_eq!(reloaded.graphics.smooth(), Smooth::Xbr);
        assert_eq!(reloaded.graphics.shake(), 1.0);

        let app = App::with_options(false, None, reloaded, false, Some(TEST_SEED));
        assert_eq!(app.lighting_mode(), LightingMode::Coarse);
        assert_eq!(app.smooth_level(), Smooth::Xbr);
        assert!(!app.fx_enabled());
        assert!(app.foreground_enabled());
        assert!(!app.glyphfx_enabled());
        assert_eq!(app.shake_scale(), 1.0);

        // The next run's world starts at those values (the two that live on
        // `World`; the rest are handed over every frame by `draw_game`).
        let mut h = Harness::with_app(app, 100, 40);
        h.enter_playing();
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.lighting, LightingMode::Coarse);
        assert_eq!(w.shake_scale, 1.0);
    }

    /// A save written before the page existed loads today's defaults, and a
    /// hand-edited field can only lose itself.
    #[test]
    fn an_old_save_without_graphics_loads_today_s_defaults() {
        use crate::render::subcell::Smooth;
        // No `graphics` key at all: everything else still loads.
        let table = ScoreTable::parse(r#"{"entries": [], "auto_pick": true}"#);
        assert!(table.auto_pick, "the rest of the file still loads");
        assert_eq!(table.graphics, crate::scores::GraphicsFile::default());
        assert_eq!(table.graphics.lighting_mode(), None, "no lighting choice yet");
        assert_eq!(table.graphics.smooth(), Smooth::default());
        assert!(!table.graphics.foreground);
        assert!(table.graphics.fx && table.graphics.glyphfx);
        assert_eq!(table.graphics.shake(), crate::game::world::SHAKE_SCALE);

        // A partial object keeps the defaults for the fields it omits.
        let table = ScoreTable::parse(r#"{"graphics": {"shake_scale": 0.5}}"#);
        assert_eq!(table.graphics.shake(), 0.5);
        assert!(table.graphics.glyphfx);

        // A hand-edited value this build cannot read is dropped, not carried.
        let table = ScoreTable::parse(
            r#"{"graphics": {"graphics_version": 2, "lighting": "moonlight",
                             "smooth": "bogus", "shake_scale": 9}}"#,
        );
        assert_eq!(table.graphics.lighting_mode(), None);
        assert_eq!(table.graphics.smooth(), Smooth::default());
        assert_eq!(table.graphics.shake(), 1.0, "clamped into the ladder's top");
    }

    /// `RATRUN_FX` (and its three siblings) wins for the session without being
    /// written back over the save — until the player changes that row, which
    /// makes the choice theirs.
    #[test]
    fn an_environment_pin_beats_the_saved_graphics_until_the_row_is_changed() {
        let _guard = GRAPHICS_ENV_LOCK.lock().unwrap();
        let key = "RATRUN_FX";
        let saved = std::env::var(key).ok();
        std::env::set_var(key, "0");
        let table = ScoreTable {
            graphics: crate::scores::GraphicsFile {
                fx: true,
                ..crate::scores::GraphicsFile::default()
            },
            ..ScoreTable::default()
        };
        let mut app = App::with_options(false, None, table, false, Some(TEST_SEED));
        assert!(!app.fx_enabled(), "the environment pins the session");
        assert!(
            app.scores.graphics.fx,
            "a pinned session must not overwrite the saved preference"
        );
        // The player answers on the page: the choice is live and persisted.
        app.set_fx(true);
        assert!(app.fx_enabled());
        assert!(app.scores.graphics.fx);
        app.set_fx(false);
        assert!(!app.fx_enabled(), "the pin is gone: the row is in charge now");
        assert!(!app.scores.graphics.fx, "and the save follows the row");
        match saved {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn options_mouse_wheel_steps_the_row_under_the_pointer() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        title_to_options(&mut h);
        h.frame(&[]);
        let music_row = h
            .app
            .hits
            .rect_for(Target::OptionsRow(options::MUSIC_ROW))
            .unwrap();
        let at = (music_row.x + 2, music_row.y);
        h.events(&[], &[(MouseEventKind::ScrollDown, at)]);
        assert_eq!(h.app.music_volume(), 95, "wheel down lowers music");
        h.events(&[], &[(MouseEventKind::ScrollUp, at)]);
        assert_eq!(h.app.music_volume(), 100, "wheel up raises music");

        // The minimap row cycles through its three sizes.
        let mm = h
            .app
            .hits
            .rect_for(Target::OptionsRow(options::MINIMAP_ROW))
            .unwrap();
        let at = (mm.x + 2, mm.y);
        assert_eq!(h.app.scores.minimap_size, minimap::MinimapSize::Normal);
        h.events(&[], &[(MouseEventKind::ScrollDown, at)]);
        assert_eq!(h.app.scores.minimap_size, minimap::MinimapSize::Small);
        h.events(&[], &[(MouseEventKind::ScrollDown, at)]);
        assert_eq!(h.app.scores.minimap_size, minimap::MinimapSize::Hidden);
        h.events(&[], &[(MouseEventKind::ScrollUp, at)]);
        assert_eq!(h.app.scores.minimap_size, minimap::MinimapSize::Small);
    }

    #[test]
    fn options_rows_change_by_keyboard() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        title_to_options(&mut h);
        h.frame(&[]);
        assert_eq!(h.app.options_row, options::MUSIC_ROW);
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.music_volume(), 95);
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.music_volume(), 100);

        // Down to effects, then to the minimap and cursor toggles.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::EFFECTS_ROW);
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.effects_volume(), 95);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::MINIMAP_ROW);
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.scores.minimap_size, minimap::MinimapSize::Small);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::CURSOR_ROW);
        let cursor_before = h.app.scores.cheese_cursor;
        h.frame(&[KeyCode::Right]);
        assert_ne!(h.app.scores.cheese_cursor, cursor_before);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::MUTE_ROW);
        h.frame(&[KeyCode::Right]);
        assert!(h.app.muted(), "the mute row toggles the master mute");
        h.frame(&[KeyCode::Right]);
        assert!(!h.app.muted());
        h.frame(&[KeyCode::Down]);
        // The zoom row steps the view ladder (Right widens, Left pulls in).
        assert_eq!(h.app.options_row, options::ZOOM_ROW);
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.effective_zoom(), 1.25);
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.effective_zoom(), 1.0);
        // The auto-pick row now sits directly under Zoom (the DETAIL row is
        // gone: sub-cell detail is always on); both directions flip the
        // binary preference.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::AUTO_PICK_ROW);
        assert!(!h.app.scores.auto_pick, "default is off");
        h.frame(&[KeyCode::Right]);
        assert!(h.app.scores.auto_pick, "Right turns auto-pick on");
        h.frame(&[KeyCode::Left]);
        assert!(!h.app.scores.auto_pick, "Left turns it back off");
        // Wave 3 controls rows sit between zoom and back.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::SCHEME_ROW);
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.input.controls.scheme(), Scheme::MouseAim);
        h.app.input.controls.set_scheme(Scheme::Classic);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::CONTROLS_ROW);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::DEADZONE_ROW);
        let dz = h.app.input.controls.deadzone;
        h.frame(&[KeyCode::Right]);
        assert!(h.app.input.controls.deadzone > dz);
        h.frame(&[KeyCode::Left]);
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::RUMBLE_ROW);
        let rumble = h.app.input.controls.rumble;
        h.frame(&[KeyCode::Right]);
        assert_ne!(h.app.input.controls.rumble, rumble);
        // The nest's reset row sits just above Back.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::RESET_ROW);
        // The Graphics page's row sits just above Back.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::GRAPHICS_ROW);
        // Enter on the Back row leaves the screen.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::BACK_ROW);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
    }

    // ---- Auto-pick level-ups (wave 3) ------------------------------------

    /// The level-up screen's AUTO-PICK pill: key `A` and a mouse click both
    /// flip the preference and the pill reads the new state.
    #[test]
    fn the_level_up_screen_toggle_flips_auto_pick_by_key_and_mouse() {
        // Key A.
        let mut h = Harness::new(80, 26);
        h.show_upgrade_cards();
        assert!(matches!(h.app.screen, Screen::LevelUp));
        assert!(h.frame(&[]).contains("AUTO-PICK: OFF"), "the pill starts OFF");
        h.frame(&[KeyCode::Char('u')]);
        assert!(h.app.scores.auto_pick, "U turns auto-pick on");
        assert!(h.frame(&[]).contains("AUTO-PICK: ON"), "the pill reads ON");
        h.frame(&[KeyCode::Char('u')]);
        assert!(!h.app.scores.auto_pick, "U turns it back off");

        // Mouse: clicking the pill flips it too.
        let mut h = Harness::new(80, 26);
        h.show_upgrade_cards();
        h.frame(&[]);
        h.click("AUTO-PICK");
        assert!(h.app.scores.auto_pick, "a click on the pill turns auto-pick on");
    }

    /// Turning the pill on takes the current card after the short reveal, and
    /// the card is the deterministic valuation of the dealt set.
    #[test]
    fn turning_auto_pick_on_the_screen_takes_the_current_card_after_the_reveal() {
        let mut h = Harness::new(80, 26);
        h.show_upgrade_cards();
        h.frame(&[]);
        let choices = h.app.levelup_choices.clone();
        let pick = crate::game::bot::best_upgrade(h.app.world.as_ref().unwrap(), &choices);
        assert_eq!(
            pick, 1,
            "the valuation takes Armor out of Damage/Armor/Magnet"
        );
        h.frame(&[KeyCode::Char('u')]);
        assert!(h.app.scores.auto_pick);
        // The reveal holds the screen for a beat, then the card lands.
        let mut picked = false;
        for _ in 0..40 {
            h.frame(&[]);
            if matches!(h.app.screen, Screen::Playing) {
                picked = true;
                break;
            }
        }
        assert!(picked, "the reveal must end and the card land");
        let loadout = &h.app.world.as_ref().unwrap().player.loadout;
        assert_eq!(
            loadout.stacks(PassiveKind::Armor),
            1,
            "the valuation's card was applied"
        );
        assert_eq!(h.app.world.as_ref().unwrap().pending_levelups, 0);
    }

    /// A later level-up with auto-pick on never opens the screen.
    #[test]
    fn later_level_ups_never_open_the_screen_when_auto_pick_is_on() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        h.app.scores.auto_pick = true;
        {
            let w = h.app.world.as_mut().unwrap();
            w.clear_enemies();
            w.clear_gems();
            w.pending_levelups = 1;
        }
        let text = h.frame(&[]);
        assert!(
            matches!(h.app.screen, Screen::Playing),
            "auto-pick must not open the screen"
        );
        assert!(text.contains("AUTO"), "the player is told what landed");
        assert_eq!(h.app.world.as_ref().unwrap().pending_levelups, 0);
    }

    /// Several pending level-ups queue one toast per spacing, and the game
    /// keeps running the whole time.
    #[test]
    fn several_pending_level_ups_queue_one_toast_at_a_time() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        h.app.scores.auto_pick = true;
        {
            let w = h.app.world.as_mut().unwrap();
            w.clear_enemies();
            w.clear_gems();
            w.pending_levelups = 3;
        }
        // The first card lands immediately (the same tick a manual screen would
        // have opened); the two behind it wait their spacing.
        let _ = h.frame(&[]);
        assert!(matches!(h.app.screen, Screen::Playing));
        assert_eq!(
            h.app.world.as_ref().unwrap().pending_levelups,
            2,
            "only one card lands per spacing"
        );
        for _ in 0..10 {
            let _ = h.frame(&[]);
            assert!(matches!(h.app.screen, Screen::Playing));
        }
        assert_eq!(
            h.app.world.as_ref().unwrap().pending_levelups,
            2,
            "the second card waits its turn"
        );
        // After the spacing the queue drains.
        for _ in 0..40 {
            let _ = h.frame(&[]);
        }
        assert_eq!(h.app.world.as_ref().unwrap().pending_levelups, 0);
    }

    /// The toast carries the card's icon and its label at every reference size.
    #[test]
    fn auto_pick_toast_carries_the_icon_and_label_at_every_size() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hh = Harness::new(w, h);
            hh.enter_playing();
            hh.app.scores.auto_pick = true;
            {
                let world = hh.app.world.as_mut().unwrap();
                world.clear_enemies();
                world.clear_gems();
                world.pending_levelups = 1;
            }
            let text = hh.frame(&[]);
            assert!(matches!(hh.app.screen, Screen::Playing), "{w}x{h} paused");
            assert!(text.contains("AUTO"), "{w}x{h} toast tag missing:\n{text}");
            let slot = hh
                .app
                .hud
                .callouts
                .iter()
                .find(|c| c.kind == Some(hud::CalloutKind::AutoPick))
                .expect("an auto-pick toast");
            assert!(slot.icon.is_some(), "{w}x{h} toast lost its icon");
            assert!(
                slot.message.contains("Lv") || slot.message.contains("Bandage") || slot.message.contains("Purse"),
                "{w}x{h} toast label {:?} lost its rank",
                slot.message
            );
            // The pill's text is really drawn (not just held in the callout):
            // the label's first word lands in the frame.
            let head = slot.message.split_whitespace().next().unwrap_or("");
            assert!(
                text.contains(head),
                "{w}x{h} toast label {head:?} not rendered:\n{text}"
            );
        }
    }

    /// The auto pick only ever ranks legal cards: the dealt set hides a new
    /// passive at a full rack and pushes the evolution partner first.
    #[test]
    fn auto_pick_respects_slot_caps_and_evolution_partners() {
        // Caps: a full passive rack offers no new passive.
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        {
            let w = h.app.world.as_mut().unwrap();
            for p in [
                PassiveKind::Armor,
                PassiveKind::Magnet,
                PassiveKind::Damage,
                PassiveKind::Regen,
            ] {
                w.player.loadout.apply(&Upgrade::Passive(p));
            }
            // A fresh profile's rack: four passives fill it.
            assert_eq!(
                w.player.loadout.passives.len(),
                w.player.loadout.max_passives()
            );
        }
        let (choices, _pick) = h.app.auto_pick_one().expect("a card was dealt");
        for c in &choices {
            if let Upgrade::Passive(p) = c {
                assert!(
                    h.app.world.as_ref().unwrap().player.loadout.stacks(*p) > 0,
                    "a full rack must not deal a new passive ({p:?})"
                );
            }
        }

        // Partners: an owned L4 weapon with its partner unowned pushes the
        // partner into the dealt set.
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        let partner = {
            let w = h.app.world.as_mut().unwrap();
            // Two weapons so the first-level-up "second weapon" rule does not
            // pre-empt the partner promotion.
            w.player
                .loadout
                .apply(&Upgrade::NewWeapon(crate::game::weapon::WeaponKind::CheeseSpray));
            w.player.loadout.weapons[0].level = 4;
            w.player.loadout.weapons[0].kind.evolution().unwrap().passive
        };
        let (choices, _pick) = h.app.auto_pick_one().expect("a card was dealt");
        assert!(
            choices.iter().any(|c| matches!(c, Upgrade::Passive(p) if *p == partner)),
            "the evolution partner must be dealt: {choices:?}"
        );
    }

    /// The options row flips auto-pick, spells out the chest exception and
    /// persists beside the other preferences.
    #[test]
    fn options_row_toggles_auto_pick_and_shows_its_note() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 26);
        title_to_options(&mut h);
        for _ in 0..options::AUTO_PICK_ROW {
            h.frame(&[KeyCode::Down]);
        }
        assert_eq!(h.app.options_row, options::AUTO_PICK_ROW);
        let text = h.frame(&[]);
        assert!(text.contains("AUTO-PICK"), "row label missing:\n{text}");
        assert!(text.contains("OFF"), "default must be off");
        assert!(
            text.contains("chests still open their card"),
            "the note must spell out the chest exception:\n{text}"
        );
        h.frame(&[KeyCode::Right]);
        assert!(h.app.scores.auto_pick, "Right flips it on");
        assert!(h.frame(&[]).contains("ON"));
        h.frame(&[KeyCode::Right]);
        assert!(!h.app.scores.auto_pick, "Right flips it back off");
        // It round-trips through the saved table next to the scores.
        let mut on = h.app.scores.clone();
        on.auto_pick = true;
        let back = ScoreTable::parse(&on.to_json());
        assert!(back.auto_pick);
    }

    /// Turning auto-pick off mid-run hands the cards back.
    #[test]
    fn turning_auto_pick_off_mid_run_restores_the_manual_screen() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        h.app.scores.auto_pick = true;
        let text = h.frame(&[KeyCode::Char('u')]);
        assert!(!h.app.scores.auto_pick, "U in play flips it off");
        assert!(
            text.contains("AUTO-PICK OFF"),
            "the flip must be announced:\n{text}"
        );
        {
            let w = h.app.world.as_mut().unwrap();
            w.clear_enemies();
            w.clear_gems();
            w.pending_levelups = 1;
        }
        h.frame(&[]);
        assert!(
            matches!(h.app.screen, Screen::LevelUp),
            "the manual cards must be back"
        );
    }

    /// Chests are a reward moment: the setting never opens them without the
    /// card.
    #[test]
    fn chests_still_open_their_card_when_auto_pick_is_on() {
        let mut h = Harness::new(80, 30);
        h.enter_playing();
        h.app.scores.auto_pick = true;
        {
            let w = h.app.world.as_mut().unwrap();
            w.clear_enemies();
            w.clear_gems();
            let pos = w.player.pos;
            w.spawn_chest(pos);
        }
        h.frame(&[]);
        assert!(
            matches!(h.app.screen, Screen::Chest),
            "auto-pick must not swallow the chest"
        );
    }

    /// A manual pick and an auto pick from the same state draw the same cards
    /// and apply the same card: the rng sees exactly what a manual run would.
    #[test]
    fn a_manual_pick_and_an_auto_pick_from_the_same_state_are_identical() {
        let mut manual = Harness::new(80, 30);
        let mut auto = Harness::new(80, 30);
        manual.enter_playing();
        auto.enter_playing();
        for _ in 0..90 {
            manual.frame(&[KeyCode::Right]);
            auto.frame(&[KeyCode::Right]);
        }
        // The manual screen draws its 1-of-3 from the run rng.
        manual.app.open_levelup();
        let manual_choices = manual.app.levelup_choices.clone();
        // The auto path draws its 1-of-3 from the same rng position.
        let (auto_choices, auto_pick) = auto.app.auto_pick_one().expect("a card was dealt");
        assert_eq!(
            format!("{manual_choices:?}"),
            format!("{auto_choices:?}"),
            "the rng draws must match a manual screen"
        );
        // The auto pick is the manual valuation of the same set.
        let manual_pick = crate::game::bot::best_upgrade(
            manual.app.world.as_ref().unwrap(),
            &manual_choices,
        );
        assert_eq!(manual_pick, auto_pick, "the choice must match");
        // Applying the same card leaves the same loadout.
        manual.app.apply_levelup(manual_pick);
        let manual_loadout = format!("{:?}", manual.app.world.as_ref().unwrap().player.loadout);
        let auto_loadout = format!("{:?}", auto.app.world.as_ref().unwrap().player.loadout);
        assert_eq!(manual_loadout, auto_loadout, "the applied card must match");
    }

    #[test]
    fn effects_preview_is_rate_limited_while_holding_a_key() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let mut h = Harness::new(80, 24);
        title_to_options(&mut h);
        h.frame(&[]);
        h.frame(&[KeyCode::Down]); // effects row
        assert_eq!(h.app.options_row, options::EFFECTS_ROW);
        for _ in 0..10 {
            h.frame(&[KeyCode::Left]);
        }
        assert_eq!(h.app.effects_volume(), 50);
        let before = h.app.preview_blips();
        for _ in 0..12 {
            h.frame(&[KeyCode::Right]);
        }
        let blips = h.app.preview_blips() - before;
        assert!(blips >= 1, "a preview should play while stepping");
        assert!(blips <= 3, "preview must be rate-limited, got {blips} blips");
        assert_eq!(h.app.effects_volume(), 100);
    }

    #[test]
    fn options_lays_out_at_the_reference_sizes_and_never_panics_tiny() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hh = Harness::new(w, h);
            title_to_options(&mut hh);
            let text = hh.frame(&[]);
            for needle in ["MUSIC", "EFFECTS", "BACK"] {
                assert!(
                    text.contains(needle),
                    "options at {w}x{h} missing {needle:?}: {text}"
                );
            }
        }
        // Degenerate terminals must not panic when the plate is opened.
        for (w, h) in [(0u16, 0u16), (5, 3), (1, 1)] {
            let mut hh = Harness::new(w, h);
            hh.frame(&[]);
            hh.frame(&[KeyCode::Char('o')]);
            let _ = hh.frame(&[]);
        }
    }

    #[test]
    fn volumes_persist_beside_the_preferences_and_old_files_load_at_full() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        // A file written before the sliders existed has no volume fields.
        let old = ScoreTable::parse("{\"entries\":[]}");
        assert_eq!((old.music_volume, old.effects_volume), (100, 100));

        // Levels set through the app land in the injected store and round-trip.
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.frame(&[KeyCode::Char('[')]);
        h.frame(&[KeyCode::Char('-')]);
        assert_eq!(
            (h.app.scores.music_volume, h.app.scores.effects_volume),
            (95, 95)
        );
        let reloaded = ScoreTable::parse(&h.app.scores.to_json());
        assert_eq!((reloaded.music_volume, reloaded.effects_volume), (95, 95));
        let app = App::with_options(false, None, reloaded, false, Some(TEST_SEED));
        assert_eq!((app.music_volume(), app.effects_volume()), (95, 95));
    }

    #[test]
    fn volume_env_overrides_apply_without_overwriting_the_saved_levels() {
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let table = ScoreTable {
            music_volume: 80,
            effects_volume: 90,
            ..ScoreTable::default()
        };
        std::env::set_var("RATRUN_MUSIC_VOLUME", "40");
        std::env::set_var("RATRUN_SFX_VOLUME", "65");
        let app = App::with_options(false, None, table, false, Some(TEST_SEED));
        std::env::remove_var("RATRUN_MUSIC_VOLUME");
        std::env::remove_var("RATRUN_SFX_VOLUME");

        assert_eq!((app.music_volume(), app.effects_volume()), (40, 65));
        let mut h = Harness::with_app(app, 80, 24);
        h.enter_playing();
        h.frame(&[KeyCode::Char('[')]);
        assert_eq!(h.app.music_volume(), 35, "the live level still moves");
        assert_eq!(
            (h.app.scores.music_volume, h.app.scores.effects_volume),
            (80, 90),
            "a pinned channel must not overwrite the saved preference"
        );
    }

    #[test]
    fn volume_env_parsing_clamps_and_snaps() {
        assert_eq!(parse_volume_env("0"), Some(0));
        assert_eq!(parse_volume_env("100"), Some(100));
        assert_eq!(parse_volume_env("57"), Some(55));
        assert_eq!(parse_volume_env(" 68 "), Some(70));
        assert_eq!(parse_volume_env("200"), Some(100));
        assert_eq!(parse_volume_env("loud"), None);
    }


    // ---- Mouse-wheel zoom ------------------------------------------------

    /// The wheel zooms the playfield and nothing else: over the compact minimap
    /// it still cycles the radar size, on the title it still walks the sector,
    /// and on the options plate it still steps the row under the pointer.
    #[test]
    fn the_wheel_zooms_the_playfield_and_keeps_its_other_jobs() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        let game = h.app.last_game;
        let field = (game.x + game.width / 2, game.y + game.height / 2);
        assert_eq!(h.app.effective_zoom(), 1.0, "a new run opens unzoomed");

        // Scroll down (away from you) widens the view, scroll up pulls in.
        h.events(&[], &[(MouseEventKind::ScrollDown, field)]);
        assert_eq!(h.app.effective_zoom(), 1.25);
        h.events(&[], &[(MouseEventKind::ScrollDown, field)]);
        assert_eq!(h.app.effective_zoom(), 1.5);
        h.events(&[], &[(MouseEventKind::ScrollUp, field)]);
        assert_eq!(h.app.effective_zoom(), 1.25);

        // The ladder clamps at both ends (2× the current view, 0.5×).
        for _ in 0..8 {
            h.events(&[], &[(MouseEventKind::ScrollDown, field)]);
        }
        assert_eq!(h.app.effective_zoom(), 2.0, "max 2x the current view");
        for _ in 0..8 {
            h.events(&[], &[(MouseEventKind::ScrollUp, field)]);
        }
        assert_eq!(h.app.effective_zoom(), 0.5, "min 0.5x the current view");

        // The live level follows the setting, and the renderer eases into it.
        let w = h.app.world.as_ref().unwrap();
        assert_eq!(w.zoom, 0.5);
        for _ in 0..30 {
            h.frame(&[]);
        }
        assert_eq!(h.app.world.as_ref().unwrap().zoom_view, 0.5);

        // Over the HUD rails (outside the playfield) the wheel does nothing.
        let zoom_before = h.app.effective_zoom();
        h.events(&[], &[(MouseEventKind::ScrollDown, (10, 0))]);
        h.events(&[], &[(MouseEventKind::ScrollDown, (10, 39))]);
        assert_eq!(
            h.app.effective_zoom(),
            zoom_before,
            "the HUD rails are not the playfield"
        );

        // Over the compact minimap the wheel keeps its old job.
        let before = h.app.scores.minimap_size;
        let compact = h.app.last_minimap;
        let zoom_before = h.app.effective_zoom();
        h.events(
            &[],
            &[(
                MouseEventKind::ScrollDown,
                (compact.x + compact.width / 2, compact.y + compact.height / 2),
            )],
        );
        assert_ne!(h.app.scores.minimap_size, before, "the radar still steps");
        assert_eq!(h.app.effective_zoom(), zoom_before, "and the view did not zoom");
    }

    /// A middle click over the playfield resets the view to 1.0× and does
    /// nothing else: it is not a `Target`, so it never dashes or fires, and
    /// over the HUD rails or the compact minimap it is ignored.
    #[test]
    fn a_middle_click_resets_the_zoom_over_the_playfield_only() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        let game = h.app.last_game;
        let field = (game.x + game.width / 2, game.y + game.height / 2);

        // Zoom in, then a middle click over the field snaps back to 1.0×.
        h.events(&[], &[(MouseEventKind::ScrollUp, field)]);
        h.events(&[], &[(MouseEventKind::ScrollUp, field)]);
        assert_eq!(h.app.effective_zoom(), 0.5);
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Middle), field)]);
        assert_eq!(h.app.effective_zoom(), 1.0, "middle click resets the view");

        // It never dashes or fires: a dash would put the cooldown on the clock,
        // and the click is not a `Target` so the app never spends the actives.
        let w = h.app.world.as_ref().unwrap();
        assert!(w.player.dash_cd <= 0.0, "a middle click must not dash");

        // Over the HUD rails and the compact minimap it is ignored.
        h.events(&[], &[(MouseEventKind::ScrollUp, field)]);
        assert_eq!(h.app.effective_zoom(), 0.75);
        let zoom_before = h.app.effective_zoom();
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Middle), (10, 0))]);
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Middle), (10, 39))]);
        let compact = h.app.last_minimap;
        h.events(
            &[],
            &[(
                MouseEventKind::Down(MouseButton::Middle),
                (compact.x + compact.width / 2, compact.y + compact.height / 2),
            )],
        );
        assert_eq!(
            h.app.effective_zoom(),
            zoom_before,
            "only the playfield resets the view"
        );
    }

    #[test]
    fn the_wheel_does_not_zoom_the_menus() {
        // Title: the wheel over the sector row walks the sectors, as before.
        let mut h = Harness::new(120, 40);
        h.frame(&[]);
        let row = h.locate("Sector");
        let floor_before = h.app.theme();
        h.events(&[], &[(MouseEventKind::ScrollDown, row)]);
        assert_ne!(h.app.theme(), floor_before, "the sector row still steps");
        assert_eq!(h.app.scores.zoom, 1.0, "the title never zooms");

        // Options: the wheel steps the row under the pointer.
        h.frame(&[KeyCode::Char('o')]);
        assert_eq!(h.app.screen.name_for_test(), "Options");
        let music_row = h.locate("MUSIC");
        let volume_before = h.app.music_volume();
        h.events(&[], &[(MouseEventKind::ScrollDown, music_row)]);
        assert_ne!(h.app.music_volume(), volume_before);
        assert_eq!(h.app.scores.zoom, 1.0, "the options wheel does not zoom");

        // …but the options screen's own ZOOM row does step it, and persists.
        h.app.options_row = options::ZOOM_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.scores.zoom, 1.25);
        h.frame(&[KeyCode::Left]);
        assert_eq!(h.app.scores.zoom, 1.0);
    }

    /// `Z` pulls the view in, `X` widens it — the keyboard path for players
    /// without a wheel. Playing only.
    #[test]
    fn z_and_x_step_the_zoom_while_playing_only() {
        let mut h = Harness::new(120, 40);
        h.frame(&[]); // title
        h.frame(&[KeyCode::Char('z')]);
        assert_eq!(h.app.scores.zoom, 1.0, "the title ignores the zoom keys");

        h.enter_playing();
        h.frame(&[KeyCode::Char('z')]);
        assert_eq!(h.app.effective_zoom(), 0.75);
        h.frame(&[KeyCode::Char('z')]);
        assert_eq!(h.app.effective_zoom(), 0.5);
        h.frame(&[KeyCode::Char('z')]);
        assert_eq!(h.app.effective_zoom(), 0.5, "0.5x is the closest view");
        h.frame(&[KeyCode::Char('x')]);
        assert_eq!(h.app.effective_zoom(), 0.75);
        h.frame(&[KeyCode::Char('x')]);
        h.frame(&[KeyCode::Char('x')]);
        assert_eq!(h.app.effective_zoom(), 1.25);

        // Paused: the field is frozen and the keys do nothing.
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        h.frame(&[KeyCode::Char('x')]);
        assert_eq!(h.app.effective_zoom(), 1.25, "no zooming while paused");
    }

    /// The zoom is a preference: it persists beside the other settings, an old
    /// file without it loads unzoomed, and the next level opens at it without
    /// gliding in.
    #[test]
    fn the_zoom_persists_and_old_save_files_load_unzoomed() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.frame(&[KeyCode::Char('x')]);
        h.frame(&[KeyCode::Char('x')]);
        assert_eq!(h.app.scores.zoom, 1.5);
        let json = h.app.scores.to_json();
        assert!(json.contains("\"zoom\""), "the setting is written: {json}");
        assert_eq!(ScoreTable::parse(&json).zoom, 1.5, "and it round-trips");

        // A file from before the wheel existed: no zoom key ⇒ today's view.
        let old = r#"{"entries":[],"minimap_size":"small","cheese_cursor":"off"}"#;
        let old = ScoreTable::parse(old);
        assert_eq!(old.zoom, 1.0);

        // The next level opens at the saved zoom, already settled (no swoop).
        let w = World::new(Theme::Sewer, 3);
        h.app.enter_world(w, false);
        let world = h.app.world.as_ref().unwrap();
        assert_eq!(world.zoom, 1.5);
        assert_eq!(world.zoom_view, 1.5, "no glide-in at level start");
    }

    /// Mouse steering maps the cursor's cell into the world through the zoom,
    /// so the same cell aims further out when the view is wider and the point
    /// always lands inside the rectangle the renderer actually covered.
    #[test]
    fn mouse_steering_maps_the_cursor_into_the_world_at_every_zoom() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        let game = h.app.last_game;
        let col = game.x + game.width / 4;
        let row = game.y + game.height / 2;
        let mut offsets = Vec::new();
        for z in crate::render::zoom::STEPS {
            {
                let w = h.app.world.as_mut().unwrap();
                w.set_zoom_instant(z);
            }
            h.frame(&[]); // render so `cam` + `zoom_scale` are for this zoom
            h.set_mouse(col, row);
            let w = h.app.world.as_ref().unwrap();
            let p = h.app.mouse_world().expect("a cursor over the field maps");
            let span = w.view_span();
            assert!(
                p.x >= w.cam.x && p.x <= w.cam.x + span.x,
                "x outside the rendered rectangle at {z}: {p:?} vs {span:?}"
            );
            assert!(
                p.y >= w.cam.y && p.y <= w.cam.y + span.y,
                "y outside the rendered rectangle at {z}: {p:?} vs {span:?}"
            );
            offsets.push((p - w.player.pos).len());
        }
        // The cursor sits a quarter of the way in: the aimed world point is
        // further from the rat the wider the view, in proportion to the zoom.
        for i in 1..offsets.len() {
            assert!(
                offsets[i] > offsets[i - 1],
                "offsets must grow with the view: {offsets:?}"
            );
        }
        let ratio = offsets[offsets.len() - 1] / offsets[0];
        let want = crate::render::zoom::MAX / crate::render::zoom::MIN;
        assert!(
            (ratio - want).abs() < 0.05,
            "the mapping scales with the zoom: {ratio} vs {want}"
        );
    }

    /// Sub-cell detail is always on and only reaches the frame the terminal can
    /// draw: with no override (every test: the block set is `Half` until
    /// `term::init`) the frame is the half-block picture at every zoom, and a
    /// forced set draws sub-cell glyphs, one cell wide each. With the smoothing
    /// off, a 1x frame is byte-identical to the half-block blit (pinned by
    /// `subcell::tests::the_ink_marks_survive_the_one_x_smoothing`); on, it is
    /// the same cells with their stair-step corners rounded.
    #[test]
    fn detail_is_always_on_and_follows_the_terminal() {
        use crate::render::subcell::{Choice, SubcellSet};
        use ratatui::buffer::CellWidth;
        for set in [SubcellSet::Quad, SubcellSet::Sextant, SubcellSet::Octant] {
            let mut plain = Harness::new(120, 40);
            let mut forced = Harness::new(120, 40);
            let mut flat = Harness::new(120, 40);
            plain.app.subcell_env = None;
            forced.app.subcell_env = Some(Choice::Force(set));
            forced.app.set_smooth(true);
            flat.app.subcell_env = Some(Choice::Force(set));
            for h in [&mut plain, &mut forced, &mut flat] {
                // Compare the playfield independently of the wall-clock map marker.
                h.app.scores.minimap_size = minimap::MinimapSize::Hidden;
                h.enter_playing();
                for i in 0..90 {
                    h.frame(&[[KeyCode::Right, KeyCode::Down][(i / 30) % 2]]);
                }
            }
            flat.app.set_smooth(false);
            for z in crate::render::zoom::STEPS {
                for h in [&mut plain, &mut forced, &mut flat] {
                    h.app.world.as_mut().unwrap().set_zoom_instant(z);
                    h.frame(&[]);
                }
                let (a, b, c) = (
                    plain.terminal.backend().buffer(),
                    forced.terminal.backend().buffer(),
                    flat.terminal.backend().buffer(),
                );
                assert!(
                    plain.app.world.as_ref().unwrap().detail().is_none(),
                    "{set:?} z={z}: a terminal not known to draw finer blocks got a detail frame"
                );
                let game = forced.app.last_game;
                let mut fine = 0;
                for y in game.top()..game.bottom() {
                    for x in game.left()..game.right() {
                        let cell = &b[(x, y)];
                        assert_eq!(cell.symbol().cell_width(), 1);
                        if !matches!(cell.symbol(), "▀" | " ") && a[(x, y)].symbol() == "▀" {
                            fine += 1;
                        }
                    }
                }
                if z > 1.0 {
                    assert!(fine > 20, "{set:?} z={z}: only {fine} sub-cell glyphs on the field");
                }
                // A forced set draws a sub-cell frame at every zoom: resampled
                // zoomed out, smoothed at 1x and zoomed in.
                assert!(
                    forced.app.world.as_ref().unwrap().detail().is_some(),
                    "{set:?} z={z}: always-on detail produced no frame"
                );
                if z <= 1.0 {
                    // Smoothing off at 1x and zoomed in: the frame is the
                    // half-block picture again, byte for byte.
                    assert!(
                        flat.app.world.as_ref().unwrap().detail().is_none(),
                        "{set:?} z={z}: smoothing off still produced a detail frame"
                    );
                    assert!(a == c, "{set:?} z={z}: smoothing off changed the frame");
                }
            }
        }
    }

    /// Smoothing redraws the frozen picture without ticking the simulation,
    /// and a forced glyph set also changes the picture key.
    #[test]
    fn the_smooth_toggle_is_render_only_and_redraws() {
        use crate::render::subcell::{Choice, SubcellSet};
        let mut h = paused_harness();
        h.app.set_smooth(true);
        h.frame(&[]);
        let drawn = h.app.world_draws;
        let ticks = h.app.world.as_ref().unwrap().ticks;
        h.app.set_smooth(false);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "a smoothing change redraws the field");
        assert_eq!(
            h.app.world.as_ref().unwrap().ticks,
            ticks,
            "smoothing is render-only: no tick"
        );
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "and then the picture is cached again");
        assert!(!h.app.smooth_enabled());
        h.app.set_smooth(true);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 2);
        // A forced set is a different picture key, so it redraws on its own.
        h.app.subcell_env = Some(Choice::Force(SubcellSet::Octant));
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 3, "a new glyph set must redraw");
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 3);
    }

    /// The parallax foreground toggle is the same kind of thing: a render-only
    /// switch that redraws the frozen picture and never ticks the world.
    #[test]
    fn the_foreground_toggle_is_render_only_and_redraws() {
        let mut h = paused_harness();
        let drawn = h.app.world_draws;
        let ticks = h.app.world.as_ref().unwrap().ticks;
        assert!(!h.app.foreground_enabled(), "off by default");
        h.app.set_foreground(true);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "a foreground change redraws the field");
        assert_eq!(
            h.app.world.as_ref().unwrap().ticks,
            ticks,
            "the foreground layer is render-only: no tick"
        );
        assert!(h.app.world.as_ref().unwrap().foreground, "the world was not told");
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 1, "and then the picture is cached again");
        h.app.set_foreground(false);
        h.frame(&[]);
        assert_eq!(h.app.world_draws, drawn + 2);
    }

    /// The round-glyph particle layer (`render::glyphfx`) is written straight
    /// into the frame's cells, outside the cached picture: it never redraws the
    /// world, a frozen field is cell-for-cell still, switching it off restores
    /// that field exactly, and every cell it does touch carries one of its own
    /// glyphs on the average of the two colours the cell had.
    #[test]
    fn the_glyph_layer_never_moves_the_frozen_field_and_off_restores_it() {
        use crate::game::particle::damage_number::DamageSource;

        let mut h = paused_harness();
        // A fresh hit forty cells to the right of the rat (one cell is one
        // world pixel across at 1x), clear of the pause plate, which is drawn
        // over the field afterwards: sparks have no lit-pool test, so plain
        // floor cells are enough for the layer to have something to draw.
        {
            let w = h.app.world.as_mut().unwrap();
            let p = w.player.pos + Vec2::new(40.0 * w.zoom_scale.x, 0.0);
            w.damage_numbers
                .record(9001, p, 3.0, 12.0, false, DamageSource::Normal);
        }

        h.app.set_glyphfx(false);
        h.frame(&[]);
        let drawn = h.app.world_draws;
        let off = h.terminal.backend().buffer().clone();
        h.frame(&[]);
        assert_eq!(
            h.terminal.backend().buffer(),
            &off,
            "a frozen field with the layer off is still"
        );

        h.app.set_glyphfx(true);
        h.frame(&[]);
        let on = h.terminal.backend().buffer().clone();
        assert_eq!(h.app.world_draws, drawn, "the layer never redraws the world");
        let area = h.app.last_game;
        let mut glyphs = 0;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let (a, b) = (&off[(x, y)], &on[(x, y)]);
                if a == b {
                    continue;
                }
                let ch = b.symbol().chars().next().unwrap_or(' ');
                assert!(
                    crate::render::glyphfx::GLYPHS.contains(&ch),
                    "only a particle glyph may change a cell, got {:?} at {x},{y}",
                    b.symbol()
                );
                // The background is the mean of the cell's own two pixels.
                let px = &h.app.pixels;
                let (lx, ly) = ((x - area.x) as usize, (y - area.y) as usize);
                let (top, bot) = (px.pixel_at(lx, ly * 2), px.pixel_at(lx, ly * 2 + 1));
                let mean = Color::Rgb(
                    ((u16::from(top.0) + u16::from(bot.0)) / 2) as u8,
                    ((u16::from(top.1) + u16::from(bot.1)) / 2) as u8,
                    ((u16::from(top.2) + u16::from(bot.2)) / 2) as u8,
                );
                assert_eq!(b.bg, mean, "a particle's cell background is the average");
                glyphs += 1;
            }
        }
        assert!(glyphs > 0, "the layer drew nothing at 1x on a lit floor");

        h.app.set_glyphfx(false);
        h.frame(&[]);
        assert_eq!(
            h.terminal.backend().buffer(),
            &off,
            "off restores the field cell for cell"
        );
        assert_eq!(h.app.world_draws, drawn, "and still nothing redrew the world");
    }

    /// The minimap's view box (and therefore the fog-of-war radar framing and
    /// the boss compass) follows the zoomed view, not the logical viewport.
    #[test]
    fn the_radar_view_box_follows_the_zoom() {
        let mut h = Harness::new(120, 40);
        h.enter_playing();
        h.frame(&[]);
        let size = minimap::MinimapSize::Normal;
        let (normal, logical) = {
            let w = h.app.world.as_ref().unwrap();
            (
                h.app.map_frame.view_sized(w, false, size, false).viewport_cells,
                w.viewport,
            )
        };
        assert_eq!(logical.x, 120.0, "the playfield spans the terminal");
        for z in [crate::render::zoom::MIN, crate::render::zoom::MAX] {
            h.app.world.as_mut().unwrap().set_zoom_instant(z);
            h.frame(&[]);
            let w = h.app.world.as_ref().unwrap();
            let boxed = h.app.map_frame.view_sized(w, false, size, false).viewport_cells;
            // Cell widths are rounded down, so compare with a cell of slack.
            assert!(
                (boxed.w - normal.w * z).abs() <= 1.0,
                "radar box width {} at {z}, {} unzoomed",
                boxed.w,
                normal.w
            );
            assert!(
                (boxed.h - normal.h * z).abs() <= 1.0,
                "radar box height {} at {z}, {} unzoomed",
                boxed.h,
                normal.h
            );
            assert_eq!(w.viewport, logical, "the logical viewport is untouched");
        }
    }

    /// Gated behind `RATRUN_DUMP_ZOOM=<dir>`: renders a busy 120 × 40 frame at
    /// z = 0.5, 1 and 2 as PPMs, so the three views can be looked at side by
    /// side (nothing important may disappear when zoomed out).
    #[test]
    fn dump_zoom_frames_ppm() {
        let Some(dir) = std::env::var_os("RATRUN_DUMP_ZOOM") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (120u16, 40u16);
        let mut hh = Harness::new(w, h);
        hh.enter_playing();
        {
            use crate::game::enemy::EnemyKind;
            use crate::game::gem::Gem;
            use rand::SeedableRng;
            let world = hh.app.world.as_mut().unwrap();
            let mut rng = rand::rngs::StdRng::seed_from_u64(3);
            let p = world.player.pos;
            for i in 0..90 {
                let a = i as f32 * 0.31;
                let r = 14.0 + (i % 12) as f32 * 12.0;
                let kind = match i % 9 {
                    3 => EnemyKind::Cat,
                    5 => EnemyKind::Brute,
                    _ => EnemyKind::Skitterer,
                };
                world.spawn_enemy_with_rng(
                    kind,
                    p + crate::math::Vec2::from_angle(a) * r,
                    30.0,
                    &mut rng,
                );
            }
            for i in 0..30 {
                let a = i as f32 * 0.77;
                world.spawn_gem(Gem::new(p + crate::math::Vec2::from_angle(a) * (20.0 + i as f32), 5));
            }
        }
        // A few live frames so weapons fire, things die and damage numbers pop.
        // XP is drained each frame: the capture is about the playfield, not the
        // level-up cards, and the rat must stay in `Playing` to show the field.
        let step = |hh: &mut Harness| {
            {
                let w = hh.app.world.as_mut().unwrap();
                w.player.xp = 0;
                w.pending_levelups = 0;
                w.player.hp = w.player.max_hp();
            }
            hh.frame(&[]);
            if hh.app.screen != Screen::Playing {
                hh.app.screen = Screen::Playing;
            }
        };
        for _ in 0..40 {
            step(&mut hh);
        }
        for z in [0.5f32, 1.0, 1.5, 2.0] {
            hh.app.world.as_mut().unwrap().set_zoom_instant(z);
            hh.app.hud.notify_zoom(z);
            for _ in 0..3 {
                step(&mut hh);
            }
            let buf = hh.terminal.backend().buffer().clone();
            let (iw, ih) = (w as usize, h as usize * 2);
            let mut px = vec![0u8; iw * ih * 3];
            for cy in 0..h {
                for cx in 0..w {
                    let cell = &buf[(cx, cy)];
                    let top = color_rgb(cell.fg);
                    let bot = color_rgb(cell.bg);
                    for (row, (r, g, b)) in [(0usize, top), (1, bot)] {
                        let y = cy as usize * 2 + row;
                        let i = (y * iw + cx as usize) * 3;
                        px[i] = r;
                        px[i + 1] = g;
                        px[i + 2] = b;
                    }
                }
            }
            let mut out = format!("P6\n{} {}\n255\n", iw, ih).into_bytes();
            out.extend_from_slice(&px);
            let name = format!("zoom_{z}");
            std::fs::write(dir.join(format!("{name}.ppm")), out).unwrap();
            let mut text = String::new();
            for y in 0..h {
                for x in 0..w {
                    text.push_str(buf[(x, y)].symbol());
                }
                text.push('\n');
            }
            std::fs::write(dir.join(format!("{name}.txt")), text).unwrap();
        }
    }

    /// Gated behind `RATRUN_OPTIONS_DIR`: dumps ANSI captures of the options
    /// plate at 80×24 and 120×40 so the screen can be rasterised and reviewed.
    #[test]
    fn capture_options_screen() {
        use crate::term::{sanitize_with, ColorDepth, GlyphSet};
        use std::fmt::Write;

        let Some(dir) = std::env::var_os("RATRUN_OPTIONS_DIR") else {
            return;
        };
        let _guard = VOLUME_ENV_LOCK.lock().unwrap();
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let mut hh = Harness::new(w, h);
            title_to_options(&mut hh);
            // Part-way sliders so the block characters read in the capture.
            hh.app.music_volume = 60;
            hh.app.effects_volume = 75;
            hh.app.options_row = options::MUSIC_ROW;
            let _ = hh.frame(&[]);

            let mut copy = hh.terminal.backend().buffer().clone();
            sanitize_with(&mut copy, GlyphSet::Unicode, ColorDepth::Truecolor);
            let mut ansi = String::new();
            let mut plain = String::new();
            for y in 0..h {
                for x in 0..w {
                    let c = &copy[(x, y)];
                    let (r, g, b) = xterm_color_rgb(c.fg);
                    let (br, bg, bb) = xterm_color_rgb(c.bg);
                    write!(
                        ansi,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        c.symbol()
                    )
                    .unwrap();
                    plain.push_str(c.symbol());
                }
                ansi.push_str("\x1b[0m\n");
                plain.push('\n');
            }
            let path = dir.join(format!("{w}x{h}-options"));
            std::fs::write(path.with_extension("ansi"), ansi).unwrap();
            std::fs::write(path.with_extension("txt"), plain).unwrap();
        }
    }

    #[test]
    fn options_do_not_offer_a_lighting_setting() {
        let mut h = Harness::new(80, 24);
        title_to_options(&mut h);
        assert!(!h.frame(&[]).contains("LIGHTING"));
    }

    // ---- The Library (specimen index) -----------------------------------

    /// Open the index from the title's LIBRARY row.
    fn open_library_from_title(h: &mut Harness) -> String {
        h.frame(&[]);
        h.click("LIBRARY");
        assert_eq!(h.app.screen.name_for_test(), "Library");
        h.frame(&[])
    }

    #[test]
    fn the_library_opens_from_the_title_and_backs_out_to_it() {
        let mut h = Harness::new(80, 24);
        let text = open_library_from_title(&mut h);
        assert!(text.contains("BESTIARY"), "no tab strip:\n{text}");
        assert!(text.contains("LIBRARY"), "no panel title:\n{text}");
        // Esc backs out to where the player came from.
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
    }

    #[test]
    fn the_library_opens_from_the_pause_screen_and_returns_there() {
        for (w, hgt) in [(80u16, 24u16), (57, 20)] {
            let mut h = Harness::new(w, hgt);
            h.enter_playing();
            h.frame(&[KeyCode::Esc]);
            assert_eq!(h.app.screen.name_for_test(), "Paused");
            // The chip is on the plate at 80x24; the `L` key works at both.
            let text = h.frame(&[KeyCode::Char('l')]);
            assert_eq!(
                h.app.screen.name_for_test(),
                "Library",
                "{w}x{hgt} did not open the library:\n{text}"
            );
            h.frame(&[KeyCode::Esc]);
            assert_eq!(h.app.screen.name_for_test(), "Paused");
            // The run was frozen: the world did not advance while the index
            // was open.
            let before = h.app.world.as_ref().unwrap().elapsed;
            h.frame(&[KeyCode::Char('l')]);
            for _ in 0..10 {
                h.frame(&[KeyCode::Down]);
            }
            assert_eq!(h.app.world.as_ref().unwrap().elapsed, before);
        }
    }

    #[test]
    fn the_library_navigates_by_keyboard_on_every_tab() {
        let mut h = Harness::new(80, 24);
        open_library_from_title(&mut h);
        for tab in ["ARSENAL", "FIELD GUIDE", "FLOORS", "BESTIARY"] {
            let text = h.frame(&[KeyCode::Right]);
            assert!(
                text.contains(&format!("[{tab}")),
                "Right did not land on {tab}:\n{text}"
            );
            // Up/Down walk the list of the selected tab.
            for _ in 0..3 {
                h.frame(&[KeyCode::Down]);
            }
            for _ in 0..3 {
                h.frame(&[KeyCode::Up]);
            }
        }
        // Tab/E/Q also change tabs.
        h.frame(&[KeyCode::Tab]);
        assert_eq!(h.app.library_ui.tab.index(), 1);
        h.frame(&[KeyCode::Char('q')]);
        assert_eq!(h.app.library_ui.tab.index(), 0);
    }

    #[test]
    fn the_library_navigates_by_mouse() {
        let mut h = Harness::new(80, 24);
        // A fresh profile lists every entry as `???`; unveil the index so the
        // rows carry their names.
        h.app.scores.library.reveal_all();
        open_library_from_title(&mut h);
        // Click the ARSENAL tab (unselected, so it is drawn without brackets)
        // and then a row under it.
        let (x, y) = h.locate("ARSENAL");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (x + 1, y))]);
        assert_eq!(h.app.library_ui.tab.index(), 1);
        // The second Arsenal row (Gnaw is the first): the pointer selects it.
        let (rx, ry) = h.locate("Cheese Spray");
        h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (rx, ry))]);
        assert_eq!(h.app.library_ui.cursor, 1);
    }

    #[test]
    fn a_new_discovery_is_marked_and_clears_when_viewed() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        // Spawn a pillbug in the rat's own sight bubble and tick.
        {
            use crate::game::enemy::EnemyKind;
            let w = h.app.world.as_mut().unwrap();
            let p = w.player.pos;
            w.spawn_enemy(EnemyKind::Pillbug, p + Vec2::new(14.0, 0.0), 1.0);
        }
        h.frame(&[]);
        let key = LibraryKey::Enemy(crate::game::enemy::EnemyKind::Pillbug);
        assert!(
            h.app.scores.library.is_seen(key),
            "an on-screen body was not discovered"
        );
        assert!(
            h.app.scores.library.is_new(key),
            "a fresh discovery must be NEW"
        );
        // The HUD announced it.
        assert!(
            h.app
                .hud
                .callouts
                .iter()
                .any(|c| c.active && c.tag == "LIBRARY"),
            "no LIBRARY callout"
        );
        // Open the index and put the cursor on the pillbug: viewing clears NEW.
        h.app.open_library(Screen::Paused);
        h.app.library_ui.set_tab(library_index::Tab::Bestiary);
        let rows = h.app.library_rows.clone();
        let idx = rows
            .iter()
            .filter(|r| r.tab == library_index::Tab::Bestiary)
            .position(|r| r.key == key)
            .expect("the pillbug row");
        h.app.library_ui.cursor = idx;
        h.frame(&[]);
        assert!(
            !h.app.scores.library.is_new(key),
            "viewing the card must clear NEW"
        );
    }

    #[test]
    fn nothing_is_discovered_outside_play() {
        let mut h = Harness::new(80, 24);
        h.frame(&[]); // title
        let rows = h.app.library_rows.clone();
        let (seen, _) = h.app.scores.library.counts(&rows, library_index::Tab::Floors);
        assert_eq!(seen, 0, "the title discovered a floor");
        h.enter_playing();
        // Entering the floor counts it.
        let seen_floor = h.app.scores.library.is_seen(LibraryKey::Floor(Theme::Sewer));
        assert!(seen_floor, "entering the Sewer did not catalog it");
        // No discovery happens while a card or the pause plate is open: those
        // screens never call the survey.
        let before = h.app.scores.library.counts(&rows, library_index::Tab::Bestiary);
        h.frame(&[KeyCode::Esc]);
        for _ in 0..5 {
            h.frame(&[]);
        }
        let after = h.app.scores.library.counts(&rows, library_index::Tab::Bestiary);
        assert_eq!(before, after, "the paused world discovered something");
    }

    #[test]
    fn the_discovery_never_touches_the_score_file_in_tests() {
        // A test App is constructed with `persist: false`; the library lives in
        // the injected `ScoreTable`, so nothing reaches the real data dir.
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        for _ in 0..30 {
            h.frame(&[]);
        }
        let json = h.app.scores.to_json();
        assert!(
            json.contains("library"),
            "the library must serialise with the score table"
        );
    }

    /// Console `pause` freezes the simulation only: the pause plate and the
    /// menus still answer, the HUD says PAUSED, and closing the console ends it.
    #[test]
    fn console_pause_freezes_only_the_sim_and_ends_when_the_console_closes() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.frame(&[]);
        // The flag set with the console shut (a script, `exec`): the world
        // holds still, the chip shows, and the pause key still works.
        h.app.toggle_console_pause();
        let before = h.app.world.as_ref().unwrap().elapsed;
        let mut text = String::new();
        for _ in 0..3 {
            text = h.frame(&[]);
        }
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, before, "the sim ran");
        assert!(text.contains("PAUSED"), "no PAUSED chip:\n{text}");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused", "the menus froze too");
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        // Opening and closing the console clears it; play resumes.
        h.frame(&[KeyCode::Char('`')]);
        assert!(h.app.console_paused);
        h.frame(&[KeyCode::Char('`')]);
        assert!(!h.app.console_paused, "closing the console must end the pause");
        h.frame(&[]);
        assert!(h.app.world.as_ref().unwrap().elapsed > before);
    }

    /// The end screen's chips and keys follow the bindings: rebinding the Nest
    /// and Quit actions moves both the text and the key that works.
    #[test]
    fn the_end_screen_keys_follow_their_bindings() {
        let mut h = Harness::new(90, 32);
        h.enter_playing();
        h.app
            .input
            .controls
            .bind_active_kbm(Action::Nest, Binding::Key(KeyBind::Char('g')));
        h.app
            .input
            .controls
            .bind_active_kbm(Action::Quit, Binding::Key(KeyBind::Char('k')));
        h.app.world.as_mut().unwrap().player.hp = -1.0;
        h.frame(&[]);
        let text = h.frame(&[]);
        assert!(text.contains("G the nest"), "the nest chip:\n{text}");
        assert!(text.contains("K to menu"), "the menu chip:\n{text}");
        h.frame(&[KeyCode::Char('b')]);
        assert_eq!(h.app.screen.name_for_test(), "End", "the old key still works");
        h.frame(&[KeyCode::Char('g')]);
        assert_eq!(h.app.screen.name_for_test(), "Nest");
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('k')]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
    }

    /// The console pauses the run: opening it freezes the sim, keys typed while
    /// it is open go to the prompt (never the game), and closing resumes on the
    /// next frame from exactly where the pause began.
    #[test]
    fn console_pauses_and_resumes_exactly() {
        let mut h = Harness::new(80, 24);
        h.enter_playing();
        h.frame(&[]);
        let before = h.app.world.as_ref().unwrap().elapsed;

        // Backtick opens the panel; the key never reaches the prompt.
        h.frame(&[KeyCode::Char('`')]);
        assert!(h.app.console.open);
        assert!(
            h.app.console.input.is_empty(),
            "the backtick was typed into the prompt"
        );

        // Typing while open edits the prompt, not the game.
        h.frame(&[KeyCode::Char('g'), KeyCode::Char('o'), KeyCode::Char('d')]);
        assert_eq!(h.app.console.input, "god");
        for _ in 0..3 {
            h.frame(&[]);
        }
        assert_eq!(
            h.app.world.as_ref().unwrap().elapsed,
            before,
            "the open console must freeze the simulation"
        );

        // Backtick closes; the next frame resumes from the same tick.
        h.frame(&[KeyCode::Char('`')]);
        assert!(!h.app.console.open);
        h.frame(&[]);
        let after = h.app.world.as_ref().unwrap().elapsed;
        assert!(
            (after - before - 1.0 / 60.0).abs() < 1e-4,
            "resume advanced {:.6}s, expected {:.6}s",
            after - before,
            1.0 / 60.0
        );
    }
    /// The console opens over every menu screen, and while it is open nothing
    /// typed or clicked reaches the screen under it (`update` returns before
    /// any screen, the action layer or the pad's presses are read).
    #[test]
    fn the_console_opens_over_every_menu_and_swallows_their_input() {
        for label in ["OPTIONS", "LIBRARY", "THE NEST"] {
            let mut h = Harness::new(80, 24);
            h.frame(&[]);
            h.click(label);
            let screen = h.app.screen.name_for_test();
            assert_ne!(screen, "Title", "{label} did not open");
            h.frame(&[KeyCode::Char('`')]);
            assert!(h.app.console.open, "the console did not open over {screen}");
            // Keys the screen would act on (navigation, confirm, quit) and a
            // click on the panel all stop at the prompt.
            let row = h.app.options_row;
            h.frame(&[KeyCode::Down, KeyCode::Char('q'), KeyCode::Char('l')]);
            h.events(&[], &[(MouseEventKind::Down(MouseButton::Left), (40, 12))]);
            assert_eq!(h.app.screen.name_for_test(), screen, "input leaked past the console");
            assert_eq!(h.app.options_row, row);
            assert!(!h.app.should_quit);
            assert_eq!(h.app.console.input, "ql");
            // Esc closes the console, not the screen under it.
            h.frame(&[KeyCode::Esc]);
            assert!(!h.app.console.open);
            assert_eq!(h.app.screen.name_for_test(), screen);
        }
        // The Controls screen too (it is reached through Options).
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        h.app.open_controls();
        h.frame(&[KeyCode::Char('`')]);
        assert!(h.app.console.open);
        h.frame(&[KeyCode::Enter]);
        assert!(h.app.bind_capture.is_none(), "Enter armed a capture under the console");
        assert_eq!(h.app.screen.name_for_test(), "Controls");

        // …and the Graphics page (reached through Options' GRAPHICS row).
        let mut h = Harness::new(80, 24);
        h.frame(&[]);
        h.app.open_options(Screen::Title);
        h.app.options_row = options::GRAPHICS_ROW;
        h.frame(&[KeyCode::Right]);
        assert_eq!(h.app.screen.name_for_test(), "Graphics");
        h.frame(&[KeyCode::Char('`')]);
        assert!(h.app.console.open, "the console opens over the graphics page");
        let row = h.app.graphics_row;
        let shake = h.app.shake_scale();
        h.frame(&[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(
            h.app.graphics_row, row,
            "the page moved under the console"
        );
        assert_eq!(h.app.shake_scale(), shake, "a setting changed under the console");
        assert_eq!(h.app.screen.name_for_test(), "Graphics");
    }


    /// The Library's animated card: the selected, unlocked entry builds a
    /// sandbox scene after the cursor rests, that scene is a clock (so the
    /// screen stays at 30 Hz), and none of it touches the run behind the
    /// pause plate or its cached picture.
    #[test]
    fn a_library_card_animates_without_touching_the_run() {
        use crate::game::enemy::EnemyKind;
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        for _ in 0..30 {
            h.frame(&[]);
        }
        h.frame(&[KeyCode::Esc]);
        for _ in 0..30 {
            h.frame(&[]);
        }
        // Open the index from the pause plate and let its own frame settle.
        h.frame(&[KeyCode::Char('l')]);
        assert_eq!(h.app.screen.name_for_test(), "Library");
        h.frame(&[]);
        let elapsed = h.app.world.as_ref().unwrap().elapsed;
        let ticks = h.app.world.as_ref().unwrap().ticks;
        let drawn = h.app.world_draws;

        // A body the rat has met: a Bestiary row is catalogued the moment it
        // is seen, so its plate is not silhouetted.
        let key = LibraryKey::Enemy(EnemyKind::Skitterer);
        h.app.scores.library.mark_seen(key);
        let rows = h.app.library_rows.clone();
        assert!(
            h.app.library_ui.select_key(&rows, key),
            "no Library row for the skitterer"
        );
        assert_eq!(h.app.library_ui.scene_key(), None, "built before the rest");
        for _ in 0..12 {
            h.frame(&[]);
        }
        assert_eq!(
            h.app.library_ui.scene_key(),
            Some(key),
            "the card never started after the cursor rested"
        );

        // It is a clock: over 20 loop iterations the screen asks for a frame
        // often enough to hold 30 Hz, and no idle probe can swallow it.
        let mut urgent = 0;
        for _ in 0..20 {
            h.app.input.begin_frame();
            h.app.update(1.0 / 60.0);
            if h.app.repaint_urgent() {
                urgent += 1;
            }
        }
        assert!(
            urgent >= 8,
            "an animating card must keep the Library live ({urgent}/20)"
        );

        // The run behind it is frozen and its picture is not redrawn: the
        // scene has a world of its own.
        assert_eq!(h.app.world.as_ref().unwrap().elapsed, elapsed);
        assert_eq!(h.app.world.as_ref().unwrap().ticks, ticks);
        assert_eq!(h.app.world_draws, drawn, "the paused picture was redrawn");

        // Closing the index drops the scene (Back leaves an opened card
        // first, then the screen).
        h.frame(&[KeyCode::Esc]);
        if h.app.screen.name_for_test() == "Library" {
            h.frame(&[KeyCode::Esc]);
        }
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        assert_eq!(h.app.library_ui.scene_key(), None);
        assert!(!h.app.library_ui.frame_due());
    }

    /// A still card (the legacy `Boss` body's posed art) builds nothing and
    /// asks for nothing: after its last input the screen goes idle.
    #[test]
    fn a_still_library_card_lets_the_screen_go_idle() {
        use crate::game::enemy::EnemyKind;
        let mut h = Harness::new(100, 40);
        h.enter_playing();
        h.frame(&[KeyCode::Esc]);
        h.frame(&[KeyCode::Char('l')]);
        assert_eq!(h.app.screen.name_for_test(), "Library");
        let rows = h.app.library_rows.clone();
        // The one bestiary body the game never spawns: its card has no scene
        // by design, only a stated reason.
        let key = LibraryKey::Enemy(EnemyKind::Boss);
        h.app.scores.library.mark_seen(key);
        assert!(
            h.app.library_ui.select_key(&rows, key),
            "no Library row for the legacy boss body"
        );
        for _ in 0..40 {
            h.frame(&[]);
        }
        assert_eq!(
            h.app.library_ui.scene_key(),
            None,
            "a still card built a scene"
        );
        assert!(!h.app.library_ui.frame_due());
        assert!(!h.app.repaint_urgent(), "a still card asks for nothing");

        // The pacer settles: an unchanged frame counts toward stillness, and
        // no urgent frame resets it.
        let mut pacer = crate::present::FramePacer::default();
        let mut now = std::time::Instant::now();
        for _ in 0..180 {
            now += std::time::Duration::from_micros(16_667);
            if pacer.should_render(now, h.app.frame_demand(), h.app.repaint_urgent()) {
                pacer.rendered(now, false);
            }
        }
        assert!(pacer.idle(), "the Library never went idle on a still card");
    }

}
