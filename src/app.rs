//! Top-level application state machine: menus, the active run, and the
//! transitions between them.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::audio::{self, AudioEngine, Sfx};
use crate::game::campaign::{self, CampaignState, KeepKind, Keepable};
use crate::game::gem::PickupKind;
use crate::game::level::Theme;
use crate::game::loadout::{Loadout, PassiveKind, Upgrade};
use crate::game::world::World;
use crate::input::InputState;
use crate::lore;
use crate::math::Vec2;
use crate::render::campaign as campaign_ui;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette;
use crate::render::{cursor, hud, menu, minimap, minimap_source, options, overlay};
use crate::scores::{ScoreEntry, ScoreTable};

use menu::{HitMap, RunSummary, Target};

/// Backdrop behind the campaign's full-screen plates (the same near-black the
/// story dossier paints).
const BACKDROP: Color = Color::Rgb(11, 15, 17);

/// How long a level-clear fanfare waits before it plays. The simulation has
/// already played the boss's own death sting on the killing tick, so the
/// fanfare follows it a beat later rather than stacking on top of it.
const FANFARE_DELAY_SECS: f32 = 0.55;

/// The title menu's rows, in order: START RUN, the sector row, OPTIONS, QUIT.
const TITLE_ROWS: usize = 4;
const TITLE_ACTIONS: [Target; TITLE_ROWS] = [
    Target::Start,
    Target::Sectors,
    Target::Options,
    Target::Quit,
];

/// Environment pins for scripted runs (`0..=100`). They override the saved
/// levels for the session without being written back over them.
const MUSIC_VOLUME_ENV: &str = "RATRUN_MUSIC_VOLUME";
const SFX_VOLUME_ENV: &str = "RATRUN_SFX_VOLUME";

/// Minimum gap between effects-volume preview blips, so holding a key or
/// dragging a slider cannot machine-gun the same effect.
const PREVIEW_COOLDOWN_SECS: f32 = 0.12;

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

/// The level-intro label for a single-sector practice run.
const PRACTICE_BLURB: &str = "PRACTICE RUN — this sector alone, on the default loadout. \
     Clearing it opens the next sector, but it is not a campaign win.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Title,
    /// Sector select: the art's level-select screen (the title's row 2).
    SectorSelect,
    Story,
    /// Level 1's access-route card (`menu::draw_map_intro`).
    MapIntro,
    /// Levels 2 and 3's intro plate (`campaign_ui::draw_level_intro`).
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
    /// The sector-select cursor (`0..3`); also the sector the title shows, so
    /// the title's material tint and menu music follow it.
    select_idx: usize,
    menu_idx: usize,
    /// The campaign run in flight (`None` on the title and during a practice
    /// run). It owns the carried loadout and the banked totals.
    campaign: Option<CampaignState>,
    /// A single-sector practice run: the level index it plays alone.
    practice: Option<usize>,
    /// Per-level numbers banked as each level completes (victory breakdown).
    records: [LevelRecord; campaign::LEVEL_COUNT],
    /// Keep-three manifest: what the finished level offered, in the art's
    /// card order, plus the parallel selection flags and the cursor/scroll.
    keep_items: Vec<Keepable>,
    keep_entries: Vec<campaign_ui::UpgradeEntry<'static>>,
    keep_selected: Vec<bool>,
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
    levelup_desc: Vec<(String, String, String)>,
    levelup_idx: usize,
    intro_timer: f32,
    story_time: f32,
    /// Game viewport rect from the last frame, for mouse→world mapping.
    last_game: Rect,
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
    /// Options screen: the selected row (`0..options::ROW_COUNT`).
    options_row: usize,
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
    /// Expanded minimap (Tab / clicking the compact map). The simulation keeps
    /// running while it is open.
    map_open: bool,
    /// Panel rects from the last frame, for pointer hit tests and tests.
    last_minimap: Rect,
    last_map: Rect,
    /// Reused minimap scratch (the art module promises zero allocation per
    /// frame once this is warm) and the per-frame marker/label buffers.
    minimap_scratch: minimap::MinimapScratch,
    map_frame: minimap_source::MinimapFrame,
    /// In-game cheese mouse cursor visibility and animation state.
    cursor: cursor::CursorState,
    /// Optional `RATRUN_CURSOR` environment override (resolved once at startup).
    cursor_env: Option<bool>,
}

impl App {
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
        let mut app = App {
            screen: Screen::Title,
            should_quit: false,
            input: InputState::new(kitty),
            audio,
            music_intensity: 0,
            scores,
            select_idx: 0,
            menu_idx: 0,
            campaign: None,
            practice: None,
            records: [LevelRecord::default(); campaign::LEVEL_COUNT],
            keep_items: Vec::new(),
            keep_entries: Vec::new(),
            keep_selected: Vec::new(),
            keep_cursor: 0,
            keep_scroll: 0,
            keep_visible_rows: 8,
            carried_entries: Vec::new(),
            victory_loadout: Vec::new(),
            victory_focus: campaign_ui::CampaignVictoryAction::PlayAgain,
            owned_upgrades: 0,
            world: None,
            levelup_choices: Vec::new(),
            levelup_desc: Vec::new(),
            levelup_idx: 0,
            intro_timer: 0.0,
            story_time: 0.0,
            last_game: Rect::new(0, 0, 0, 0),
            rat_art: Vec::new(),
            rat_art_w: 0,
            run_counter: 0,
            recorded: false,
            new_best: false,
            last_score: 0,
            persist,
            fixed_seed,
            pixels: PixelBuffer::new(0, 0),
            sfx_frame: Vec::new(),
            sting: None,
            hud: hud::HudState::default(),
            hits: HitMap::default(),
            muted: false,
            music_volume,
            effects_volume,
            music_env,
            effects_env,
            options_row: options::MUSIC_ROW,
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
            last_minimap: Rect::new(0, 0, 0, 0),
            last_map: Rect::new(0, 0, 0, 0),
            minimap_scratch: minimap::MinimapScratch::new(),
            map_frame: minimap_source::MinimapFrame::new(),
            cursor: cursor::CursorState::default(),
            cursor_env: cursor::env_override(),
        };
        app.push_volumes();
        app.menu_music();
        app
    }

    /// The theme of the sector the title/select screen points at.
    fn theme(&self) -> Theme {
        campaign::level_theme(self.select_idx)
    }

    /// The sector-select cursor, clamped into the campaign.
    fn sector(&self) -> usize {
        self.select_idx.min(campaign::LEVEL_COUNT - 1)
    }

    /// Step the sector cursor, wrapping through the three levels.
    fn cycle_sector(&mut self, step: isize) {
        let n = campaign::LEVEL_COUNT as isize;
        self.select_idx = (self.select_idx as isize + step).rem_euclid(n) as usize;
        self.menu_music();
        self.sfx(Sfx::UiMove);
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
        self.play_music(theme, 0); // calm: bass only
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
        Some(w.cam + Vec2::new(gx + 0.5, gy * 2.0 + 1.0))
    }

    /// The throttle vector fed to the world: keyboard unit vector, or — when
    /// the mouse was the last input — toward the cursor, easing to a stop.
    fn resolve_move_dir(&self) -> Vec2 {
        if self.input.prefer_mouse {
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

    fn start_campaign(&mut self, via_story: bool) {
        let seed = self.seed();
        self.campaign = Some(CampaignState::new(seed));
        self.practice = None;
        self.records = [LevelRecord::default(); campaign::LEVEL_COUNT];
        self.keep_items.clear();
        self.keep_entries.clear();
        self.keep_selected.clear();
        self.victory_loadout.clear();
        let Some(world) = self.campaign.as_ref().map(|c| c.world()) else {
            return;
        };
        self.enter_world(world, via_story);
    }

    /// Begin a **practice run**: one later sector, alone, on the default
    /// loadout, carrying nothing in and nothing out. It still records the
    /// sector clear (so the next one unlocks) but never a campaign win.
    fn start_practice(&mut self, level: usize) {
        let level = level.clamp(1, campaign::LEVEL_COUNT - 1);
        let seed = campaign::level_seed(self.seed(), level);
        self.campaign = None;
        self.practice = Some(level);
        self.records = [LevelRecord::default(); campaign::LEVEL_COUNT];
        self.keep_items.clear();
        self.keep_entries.clear();
        self.keep_selected.clear();
        self.victory_loadout.clear();
        let world = World::new_level(level, seed, Loadout::starting());
        self.enter_world(world, false);
    }

    /// Start the sector the select screen points at: sector 1 is the campaign
    /// from the beginning, a later sector is a labelled practice run. A locked
    /// sector refuses and stays put.
    fn start_sector(&mut self) {
        let level = self.sector();
        if !self.scores.level_unlocked(level) {
            self.sfx(Sfx::UiMove);
            return;
        }
        self.sfx(Sfx::UiSelect);
        if level == 0 {
            self.start_run();
        } else {
            self.start_practice(level);
        }
    }

    /// Hand a freshly built level to the app: reset the per-level telemetry,
    /// refresh the carried-upgrade strip, pick the intro screen and start the
    /// sector's music.
    fn enter_world(&mut self, world: World, via_story: bool) {
        let level = world.level_index;
        let theme = world.level.theme;
        self.world = Some(world);
        self.recorded = false;
        self.new_best = false;
        self.damage_taken = 0.0;
        self.dashes_used = 0;
        self.heals_picked = 0;
        self.boss_slain = false;
        self.prev_boss_alive = false;
        self.last_summary = None;
        self.owned_upgrades = 0;
        // Fog of war resets with the level: a new `World` starts unexplored.
        self.map_open = false;
        self.last_minimap = Rect::new(0, 0, 0, 0);
        self.last_map = Rect::new(0, 0, 0, 0);
        self.hud.reset();
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

    /// Another run of the kind just played, skipping the story dossier.
    fn retry(&mut self) {
        match self.practice {
            Some(level) => self.start_practice(level),
            None => self.start_campaign(false),
        }
    }

    /// Drop the run and return to the title (pause → Q, end → Q, victory → Q).
    fn back_to_title(&mut self) {
        self.world = None;
        self.campaign = None;
        self.practice = None;
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
                        _ => {}
                    }
                }
                MouseEventKind::Down(MouseButton::Right) if target == Some(Target::Dash) => {
                    return Some(Target::Pause)
                }
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
                        Some(Target::Quit) => self.menu_idx = 3,
                        Some(Target::OptionsRow(i)) => {
                            self.options_row = i.min(options::ROW_COUNT - 1)
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
                            self.select_idx = i.min(campaign::LEVEL_COUNT - 1)
                        }
                        Some(Target::KeepItem(i)) => self.keep_cursor = i,
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
        self.input.tick(dt);
        // Deferred stings first: a queued fanfare lands after the sound that
        // caused it (see `queue_sting`).
        self.tick_stings(dt);
        // Global mute toggle.
        if self.input.pressed_char('m') {
            self.toggle_mute();
        }
        // Global cheese-cursor preference toggle ('c'; ignores Ctrl+C).
        let cursor_toggled = self.input.pressed_unmodified_char('c');
        if cursor_toggled {
            self.toggle_cheese_cursor();
        }
        // Volume quick keys, live on every screen so a scripted run can set a
        // level without navigating: `[` / `]` step music, `-` / `=` step
        // effects (both in 5% steps). Discrete presses only, so holding a key
        // never runs the level away.
        let mut volume_changed = false;
        if self.input.pressed_char('[') {
            self.set_music_volume(options::step_volume(self.music_volume, -1));
            volume_changed = true;
        }
        if self.input.pressed_char(']') {
            self.set_music_volume(options::step_volume(self.music_volume, 1));
            volume_changed = true;
        }
        if self.input.pressed_char('-') {
            self.set_effects_volume(options::step_volume(self.effects_volume, -1), true);
            volume_changed = true;
        }
        if self.input.pressed_char('=') {
            self.set_effects_volume(options::step_volume(self.effects_volume, 1), true);
            volume_changed = true;
        }
        // Preview blips are rate limited so a mouse drag across the slider
        // cannot machine-gun one effect per cell.
        self.preview_cd = (self.preview_cd - dt.max(0.0)).max(0.0);
        // The HUD read-out ages with the frame clock on every screen; during
        // play `hud.tick` advances it too.
        if !matches!(self.screen, Screen::Playing) {
            self.hud.tick_volume_flash(dt);
        }
        // Options: the title row and the pause legend open it; nothing else does.
        if !volume_changed && self.input.pressed_char('o') {
            let from = self.screen;
            if matches!(from, Screen::Title | Screen::Paused) {
                self.open_options(from);
            }
        }
        let previous = self.screen;
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
            Screen::LevelComplete => self.update_levelcomplete(click),
            Screen::KeepThree => self.update_keep_three(click),
            Screen::Victory => self.update_victory(click),
            Screen::End => self.update_end(click),
        }
        if self.screen != previous {
            self.hits.clear();
            self.hits.hovered = None;
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
        self.screen = Screen::Options;
        // Music stays audible on the options screen wherever it is opened
        // from: the title's calm track and the paused level's track both keep
        // playing (pause never stops or ducks the synth), so the music slider
        // is audible without restarting anything.
    }

    /// Leave the options screen for wherever it was opened from.
    fn close_options(&mut self) {
        self.sfx(Sfx::UiSelect);
        self.screen = self.options_return;
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
        if self.input.just_pressed(KeyCode::Esc) {
            self.close_options();
            return;
        }
        if self.input.just_pressed(KeyCode::Up) {
            self.options_row = (row + options::ROW_COUNT - 1) % options::ROW_COUNT;
            self.sfx(Sfx::UiMove);
        }
        if self.input.just_pressed(KeyCode::Down) {
            self.options_row = (row + 1) % options::ROW_COUNT;
            self.sfx(Sfx::UiMove);
        }
        let row = self.options_row;
        if self.input.just_pressed(KeyCode::Left) {
            self.adjust_option(row, -1);
        }
        if self.input.just_pressed(KeyCode::Right) {
            self.adjust_option(row, 1);
        }
        if row == options::BACK_ROW
            && (self.input.just_pressed(KeyCode::Enter) || chosen.is_some())
        {
            self.close_options();
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
            _ => {}
        }
    }

    /// Step one options row from a mouse-wheel delta (`step > 0` is scroll
    /// down). Scroll up raises a volume and grows the minimap; scroll down
    /// lowers a volume and shrinks the minimap.
    fn scroll_option(&mut self, row: usize, step: isize) {
        match row {
            options::MUSIC_ROW | options::EFFECTS_ROW => self.adjust_option(row, -step),
            options::MINIMAP_ROW => self.adjust_option(row, step),
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
        } else if self.input.pressed_char('q') {
            Some(Target::Quit)
        } else if self.input.pressed_char('l') {
            Some(Target::Story)
        } else {
            if self.input.just_pressed(KeyCode::Up) {
                self.menu_idx = (self.menu_idx + TITLE_ROWS - 1) % TITLE_ROWS;
                self.sfx(Sfx::UiMove);
            }
            if self.input.just_pressed(KeyCode::Down) {
                self.menu_idx = (self.menu_idx + 1) % TITLE_ROWS;
                self.sfx(Sfx::UiMove);
            }
            if self.input.just_pressed(KeyCode::Left) {
                self.cycle_sector(-1);
            }
            if self.input.just_pressed(KeyCode::Right) {
                self.cycle_sector(1);
            }
            self.input
                .just_pressed(KeyCode::Enter)
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
                // The sector select opens on the sector the title shows.
                self.sfx(Sfx::UiSelect);
                self.screen = Screen::SectorSelect;
            }
            Some(Target::Options) => {
                self.sfx(Sfx::UiSelect);
                self.open_options(Screen::Title);
            }
            Some(Target::NextSector) => self.cycle_sector(1),
            Some(Target::PreviousSector) => self.cycle_sector(-1),
            _ => {}
        }
    }

    /// Sector select: `←`/`→` move, Enter starts the highlighted sector, Esc
    /// or the art's Back control returns to the title.
    fn update_sector_select(&mut self, click: Option<Target>) {
        if click == Some(Target::Back)
            || self.input.just_pressed(KeyCode::Esc)
            || self.input.pressed_char('q')
        {
            self.sfx(Sfx::UiSelect);
            self.screen = Screen::Title;
            return;
        }
        match click {
            Some(Target::SelectSector(i)) => {
                self.select_idx = i.min(campaign::LEVEL_COUNT - 1);
                self.menu_music();
                self.sfx(Sfx::UiMove);
            }
            Some(Target::PreviousSector) => self.cycle_sector(-1),
            Some(Target::NextSector) => self.cycle_sector(1),
            _ => {}
        }
        if self.input.just_pressed(KeyCode::Left) || self.input.just_pressed(KeyCode::Up) {
            self.cycle_sector(-1);
        }
        if self.input.just_pressed(KeyCode::Right) || self.input.just_pressed(KeyCode::Down) {
            self.cycle_sector(1);
        }
        if click == Some(Target::StartSector) || self.input.just_pressed(KeyCode::Enter) {
            self.start_sector();
        }
    }

    fn update_story(&mut self, click: Option<Target>) {
        if click == Some(Target::Back)
            || self.input.just_pressed(KeyCode::Enter)
            || self.input.just_pressed(KeyCode::Esc)
            || self.input.pressed_char('q')
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
            || self.input.just_pressed(KeyCode::Enter)
            || self.input.just_pressed(KeyCode::Char(' '))
        {
            self.screen = Screen::Playing;
        }
    }

    /// Levels 2 and 3's intro plate: Enter/Space/click drops into the level.
    fn update_levelintro(&mut self, click: Option<Target>) {
        if click == Some(Target::Begin)
            || self.input.just_pressed(KeyCode::Enter)
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

    fn update_playing(&mut self, dt: f32, click: Option<Target>) {
        // The minimap is the first claim on Tab, a click on the compact map and
        // Esc: while it is open those *close* it instead of pausing. The
        // simulation keeps running either way.
        let mut esc_used = false;
        if self.input.just_pressed(KeyCode::Tab) || click == Some(Target::Minimap) {
            self.map_open = !self.map_open;
            self.sfx(Sfx::UiMove);
        } else if self.map_open && self.input.just_pressed(KeyCode::Esc) {
            self.map_open = false;
            esc_used = true;
            self.sfx(Sfx::UiMove);
        }
        if self.input.pressed_char('n') {
            self.cycle_minimap_size();
        }
        if click == Some(Target::Pause)
            || (self.input.just_pressed(KeyCode::Esc) && !esc_used)
            || self.input.pressed_char('p')
        {
            self.map_open = false;
            self.screen = Screen::Paused;
            return;
        }
        let move_dir = self.resolve_move_dir();
        let dash = click == Some(Target::Dash) || self.input.just_pressed(KeyCode::Char(' '));
        let mut open_levelup = false;
        let mut ended = false;
        let mut level_done = false;
        let mut intensity = 1u32;
        if let Some(w) = self.world.as_mut() {
            let hp_before = w.player.hp;
            w.update(dt, move_dir, dash);
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
            }
            if self.sfx_frame.contains(&Sfx::Dash) {
                self.dashes_used += 1;
            }
            for &pk in w.last_pickups() {
                if pk == PickupKind::Heal {
                    self.heals_picked += 1;
                }
            }
            let boss_hp = w.boss_hp();
            let boss_alive = boss_hp.is_some();
            if self.prev_boss_alive && !boss_alive && w.director.boss_spawned {
                self.boss_slain = true;
            }
            self.prev_boss_alive = boss_alive;
            self.hud.telemetry.set_boss_alive(boss_alive);
            self.hud.telemetry.set_boss_hp(boss_hp);
            self.hud.telemetry.set_boss_offset(w.boss_offset());
            let enemy_count = w.enemy_count();
            self.hud.telemetry.set_enemy_count(enemy_count);
            self.hud.tick(dt, w, &self.sfx_frame, self.muted);
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
            } else if w.pending_levelups > 0 {
                open_levelup = true;
            }
        }
        for i in 0..self.sfx_frame.len() {
            let s = self.sfx_frame[i];
            self.sfx(s);
        }
        self.request_music_intensity(intensity);
        if open_levelup {
            self.open_levelup();
        } else if ended {
            self.finalize();
            self.screen = Screen::End;
        } else if level_done {
            self.on_level_complete();
        }
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
        let record = LevelRecord {
            time: w.elapsed,
            kills: w.kills,
            score: w.score,
            player_level: w.player.level,
        };
        let campaign_won = self.campaign.is_some() && w.campaign_won();
        self.records[level] = record;
        // One card per owned weapon and per passive; counted without building
        // the keepable list twice.
        self.owned_upgrades = w.player.loadout.weapons.len() + w.player.loadout.passives.len();
        if campaign_won {
            self.victory_loadout = entries_for(&w.keepables());
        }
        self.scores.record_level(level, record.score, record.time);
        if self.persist {
            // The clear (and the unlock it grants) is durable even if the
            // player quits right after it.
            self.scores.save();
        }
        if campaign_won {
            // `finalize` owns the victory sting (it knows the run won).
            self.finalize();
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
            || self.input.just_pressed(KeyCode::Enter)
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
        self.keep_items = items;
        self.keep_selected = vec![false; self.keep_items.len()];
        self.keep_cursor = 0;
        self.keep_scroll = 0;
        self.screen = Screen::KeepThree;
    }

    /// `(selected, required)` straight from the art's own helpers, so the pack
    /// size is decided in one place (`keep_three_required`).
    fn keep_counts(&self) -> (usize, usize) {
        let view = campaign_ui::KeepThreeView {
            from_level: campaign_ui::CampaignLevel::from_index(self.keep_from_level()),
            next_level: campaign_ui::CampaignLevel::from_index(self.keep_from_level() + 1),
            items: &self.keep_entries,
            selected: &self.keep_selected,
            max_keep: campaign::KEEP_COUNT,
            cursor: self.keep_cursor,
            scroll: self.keep_scroll,
            hovered: None,
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
            if self.input.just_pressed(KeyCode::Up) || self.input.just_pressed(KeyCode::Left) {
                self.keep_cursor = (self.keep_cursor + n - 1) % n;
                self.sfx(Sfx::UiMove);
            }
            if self.input.just_pressed(KeyCode::Down) || self.input.just_pressed(KeyCode::Right) {
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

        let confirm = click == Some(Target::KeepConfirm)
            || self.input.just_pressed(KeyCode::Enter);
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
        if self.input.just_pressed(KeyCode::Left)
            || self.input.just_pressed(KeyCode::Right)
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
            self.input.just_pressed(KeyCode::Enter).then_some({
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
        if self.input.pressed_char('q') {
            self.back_to_title();
        }
    }

    fn open_levelup(&mut self) {
        self.map_open = false;
        if let Some(w) = self.world.as_mut() {
            let choices = w.player.loadout.generate_choices(&mut w.rng);
            self.levelup_desc = choices
                .iter()
                .map(|c| w.player.loadout.describe(c))
                .collect();
            self.levelup_choices = choices;
            self.levelup_idx = 0;
            self.screen = Screen::LevelUp;
        }
    }

    fn update_levelup(&mut self, click: Option<Target>) {
        let n = self.levelup_choices.len().max(1);
        if self.input.just_pressed(KeyCode::Up) || self.input.just_pressed(KeyCode::Left) {
            self.levelup_idx = (self.levelup_idx + n - 1) % n;
            self.sfx(Sfx::UiMove);
        }
        if self.input.just_pressed(KeyCode::Down) || self.input.just_pressed(KeyCode::Right) {
            self.levelup_idx = (self.levelup_idx + 1) % n;
            self.sfx(Sfx::UiMove);
        }
        let mut pick: Option<usize> = None;
        for (i, key) in ['1', '2', '3'].iter().enumerate() {
            if self.input.pressed_char(*key) && i < self.levelup_choices.len() {
                pick = Some(i);
            }
        }
        if self.input.just_pressed(KeyCode::Enter) {
            pick = Some(self.levelup_idx);
        }
        if let Some(Target::Upgrade(i)) = click {
            pick = Some(i);
        }
        if let Some(i) = pick {
            self.sfx(Sfx::UiSelect);
            self.apply_levelup(i);
        }
    }

    fn apply_levelup(&mut self, i: usize) {
        if let Some(w) = self.world.as_mut() {
            if let Some(up) = self.levelup_choices.get(i).cloned() {
                if let Some((title, _, _)) = self.levelup_desc.get(i) {
                    self.hud.notify_upgrade(title);
                }
                let heal = w.player.loadout.apply(&up);
                w.player.heal(heal);
                w.pending_levelups = w.pending_levelups.saturating_sub(1);
            }
            if w.pending_levelups > 0 {
                self.open_levelup();
            } else {
                self.screen = Screen::Playing;
            }
        }
    }

    fn update_paused(&mut self, click: Option<Target>) {
        if click == Some(Target::Options) || self.input.pressed_char('o') {
            self.sfx(Sfx::UiSelect);
            self.open_options(Screen::Paused);
            return;
        }
        if click == Some(Target::Resume)
            || self.input.just_pressed(KeyCode::Esc)
            || self.input.pressed_char('p')
        {
            self.screen = Screen::Playing;
        }
        if click == Some(Target::Menu) || self.input.pressed_char('q') {
            self.back_to_title();
        }
    }

    fn update_end(&mut self, click: Option<Target>) {
        if click == Some(Target::Restart) || self.input.just_pressed(KeyCode::Enter) {
            self.retry();
        }
        if click == Some(Target::Menu) || self.input.pressed_char('q') {
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
            damage_mult: w.player.loadout.stats.damage_mult,
            fire_rate_mult: w.player.loadout.stats.fire_rate_mult,
            armor: w.player.loadout.stats.armor,
            sector: (w.level_index + 1).clamp(1, campaign::LEVEL_COUNT) as u8,
            practice: self.practice.is_some(),
        }
    }

    fn finalize(&mut self) {
        if self.recorded {
            return;
        }
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
            // "New high score" means it beat the table's best, computed before
            // this run's entry is inserted (an under-full table is no free pass).
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

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.hits.clear();
        match self.screen {
            Screen::Title => {
                self.ensure_rat_art(area);
                menu::draw_title(
                    frame,
                    area,
                    self.theme(),
                    self.menu_idx,
                    self.sector(),
                    self.scores.level_unlocked(self.sector()),
                    &self.scores,
                    &self.rat_art,
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
                    menu::draw_title(
                        frame,
                        area,
                        self.theme(),
                        self.menu_idx,
                        self.sector(),
                        self.scores.level_unlocked(self.sector()),
                        &self.scores,
                        &self.rat_art,
                        &mut self.hits,
                    );
                }
                self.draw_options(frame, area);
            }
            Screen::Story => {
                self.ensure_rat_art(area);
                menu::draw_title(
                    frame,
                    area,
                    self.theme(),
                    self.menu_idx,
                    self.sector(),
                    self.scores.level_unlocked(self.sector()),
                    &self.scores,
                    &self.rat_art,
                    &mut self.hits,
                );
                menu::draw_story(frame, area, self.story_time, &mut self.hits);
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
                        self.theme(),
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
                            menu::draw_pause_with_run(frame, area, w, self.muted, &mut self.hits);
                        } else {
                            menu::draw_pause(frame, area, &mut self.hits);
                        }
                    }
                    Screen::LevelUp => {
                        if let Some(w) = self.world.as_ref() {
                            menu::draw_levelup_with_loadout(
                                frame,
                                area,
                                w.player.level,
                                &self.levelup_desc,
                                &self.levelup_choices,
                                &w.player.loadout,
                                self.levelup_idx,
                                &mut self.hits,
                            );
                        } else {
                            menu::draw_levelup(
                                frame,
                                area,
                                1,
                                &self.levelup_desc,
                                &self.levelup_choices,
                                self.levelup_idx,
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
                                &mut self.hits,
                            );
                        }
                    }
                    Screen::Playing => self.hits.add(area, Target::Dash),
                    _ => {}
                }
                // The minimap is registered last, so it takes precedence over
                // the full-field Dash target. Only a left press activates a
                // target (`mouse_action`), so right-click still dashes/pauses.
                if matches!(self.screen, Screen::Playing) {
                    if !self.last_map.is_empty() {
                        self.hits.add(self.last_map, Target::Minimap);
                    } else if !self.last_minimap.is_empty() {
                        self.hits.add(self.last_minimap, Target::Minimap);
                    }
                }
            }
        }
        self.draw_cursor(frame);
    }

    /// Composite the cheese mouse cursor last, on top of every normal-game
    /// screen, without disturbing `self.hits`.
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
            row: self.options_row,
            accent: options::accent_from(menu::material(self.theme())),
        };
        options::draw(frame, area, &view, &mut self.hits);
    }

    // ---- Campaign plates -------------------------------------------------

    /// Sector select: three bay cards fed from the persisted per-level bests,
    /// with the art's own hover/selection styling and click rectangles.
    fn draw_sector_select(&mut self, frame: &mut Frame, area: Rect) {
        draw_backdrop(frame, area);
        let mut slots = [campaign_ui::LevelSlotView {
            level: campaign_ui::CampaignLevel::Sewer,
            unlocked: true,
            best: None,
        }; campaign::LEVEL_COUNT];
        for (i, slot) in slots.iter_mut().enumerate() {
            slot.level = campaign_ui::CampaignLevel::from_index(i);
            slot.unlocked = self.scores.level_unlocked(i);
            slot.best = self.scores.level_best(i).map(|b| campaign_ui::LevelBest {
                score: b.score,
                best_time_secs: b.time,
                cleared: b.cleared,
            });
        }
        let view = campaign_ui::LevelSelectView {
            slots,
            selected: self.sector(),
            hovered: match self.hits.hovered {
                Some(Target::SelectSector(i)) => {
                    Some(campaign_ui::LevelSelectAction::SelectLevel(
                        campaign_ui::CampaignLevel::from_index(i),
                    ))
                }
                Some(Target::StartSector) => Some(campaign_ui::LevelSelectAction::Start),
                Some(Target::Back) => Some(campaign_ui::LevelSelectAction::Back),
                _ => None,
            },
        };
        let list = campaign_ui::draw_level_select(frame, area, &view);
        for (rect, action) in list.iter() {
            let target = match action {
                campaign_ui::LevelSelectAction::SelectLevel(level) => {
                    Target::SelectSector(level.index())
                }
                campaign_ui::LevelSelectAction::Start => Target::StartSector,
                campaign_ui::LevelSelectAction::Back => Target::Back,
            };
            self.hits.add(rect, target);
        }
    }

    /// The cleared-sector plate for the boss the level ended on.
    fn draw_level_complete(&mut self, frame: &mut Frame, area: Rect) {
        draw_backdrop(frame, area);
        let list = match self.world.as_ref() {
            Some(w) => {
                let view = campaign_ui::LevelCompleteView {
                    level: campaign_ui::CampaignLevel::from_index(w.level_index),
                    time_secs: w.elapsed,
                    kills: w.kills,
                    score_so_far: w.total_score(),
                    level_score: w.score,
                    player_level: w.player.level,
                    owned_upgrades: self.owned_upgrades,
                    custom_triumph: None,
                    hovered: (self.hits.hovered == Some(Target::Continue))
                        .then_some(campaign_ui::LevelCompleteAction::Continue),
                };
                campaign_ui::draw_level_complete(frame, area, &view)
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
                    level: campaign_ui::CampaignLevel::from_index(w.level_index),
                    flavor: practice.then_some(PRACTICE_BLURB),
                    carried: &self.carried_entries,
                    score_so_far: w.total_score(),
                    hovered: (self.hits.hovered == Some(Target::Begin))
                        .then_some(campaign_ui::LevelIntroAction::Begin),
                };
                campaign_ui::draw_level_intro(frame, area, &view)
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
        let from = self.keep_from_level();
        let list = {
            let view = campaign_ui::KeepThreeView {
                from_level: campaign_ui::CampaignLevel::from_index(from),
                next_level: campaign_ui::CampaignLevel::from_index(from + 1),
                items: &self.keep_entries,
                selected: &self.keep_selected,
                max_keep: campaign::KEEP_COUNT,
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
                    _ => None,
                },
            };
            campaign_ui::draw_keep_three(frame, area, &view)
        };
        for (rect, action) in list.iter() {
            let target = match action {
                campaign_ui::KeepThreeAction::ToggleItem(i) => Target::KeepItem(i),
                campaign_ui::KeepThreeAction::ScrollUp => Target::KeepScrollUp,
                campaign_ui::KeepThreeAction::ScrollDown => Target::KeepScrollDown,
                campaign_ui::KeepThreeAction::Confirm => Target::KeepConfirm,
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
        }; campaign::LEVEL_COUNT];
        for (i, row) in levels.iter_mut().enumerate() {
            let record = self.records[i];
            *row = campaign_ui::LevelBreakdown {
                level: campaign_ui::CampaignLevel::from_index(i),
                time_secs: record.time,
                kills: record.kills,
                score: record.score,
                player_level: record.player_level,
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
                final_score: self.last_score,
                total_time_secs: total_time,
                total_kills,
                new_best: self.new_best,
                levels,
                final_loadout: &self.victory_loadout,
                custom_blurb: None,
                focus: self.victory_focus,
                hovered: match self.hits.hovered {
                    Some(Target::Restart) => Some(campaign_ui::CampaignVictoryAction::PlayAgain),
                    Some(Target::Menu) => Some(campaign_ui::CampaignVictoryAction::Menu),
                    _ => None,
                },
            };
            campaign_ui::draw_campaign_victory(frame, area, &view)
        };
        for (rect, action) in list.iter() {
            let target = match action {
                campaign_ui::CampaignVictoryAction::PlayAgain => Target::Restart,
                campaign_ui::CampaignVictoryAction::Menu => Target::Menu,
            };
            self.hits.add(rect, target);
        }
    }

    fn draw_game(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
        let (top, game, bottom) = (rows[0], rows[1], rows[2]);
        self.last_game = game;

        if let Some(w) = self.world.as_mut() {
            if game.width > 0 && game.height > 0 {
                // Reuse the pixel buffer between frames: only a viewport resize
                // reallocates it.
                self.pixels
                    .resize(game.width as usize, game.height as usize * 2);
                w.draw(&mut self.pixels);
                self.pixels.render_to(game, frame.buffer_mut());
            }
        } else {
            frame.render_widget(
                Block::default().style(Style::default().bg(Color::Black)),
                game,
            );
        }

        if let Some(w) = self.world.as_ref() {
            if matches!(self.screen, Screen::Playing) {
                overlay::draw_overlays(frame, game, w, &mut self.hud);
            }
            hud::draw_top(frame, top, w, &mut self.hud);
            hud::draw_bottom(frame, bottom, w, &mut self.hud);
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
        self.map_frame.refresh(w);
        let hovered = self.hits.hovered == Some(Target::Minimap);
        let compact_size = self.scores.minimap_size;
        let view = self
            .map_frame
            .view_sized(w, expanded, compact_size, hovered);
        minimap::draw(frame, game, &view, w, &mut self.minimap_scratch);
        if expanded {
            let panel = minimap_source::expanded_rect(game);
            if panel.is_empty() {
                return;
            }
            self.last_map = panel;
            draw_map_legend(frame, game, panel);
        } else {
            let panel = minimap_source::compact_rect_for_size(game, compact_size);
            if panel.is_empty() {
                return;
            }
            self.last_minimap = panel;
        }
    }
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
/// the one promise that rail does not state: opening the map does not pause the
/// run. Drawn on the first free row under the modal, and skipped when the
/// terminal has no room for it.
fn draw_map_legend(frame: &mut Frame, game: Rect, panel: Rect) {
    let y = panel.bottom();
    if y >= game.bottom() || game.height == 0 {
        return;
    }
    let text = if game.width >= 72 {
        " TAB/Esc close · N radar size · click the map · the run keeps running "
    } else if game.width >= 60 {
        " TAB/Esc close · N size · click map · the run keeps running "
    } else {
        " TAB/Esc close · N size · run continues "
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
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
            panic!("{text:?} is missing from the frame")
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
    fn mouse_title_sector_row_story_and_quit() {
        for size in [(57, 20), (80, 24)] {
            let mut h = Harness::new(size.0, size.1);
            h.frame(&[]);
            let row = h.locate("Sewer");
            h.events(&[], &[(MouseEventKind::Moved, row)]);
            assert_eq!(h.app.menu_idx, 1, "the sector row is menu row 2");
            // The arrows step the campaign sector (wrapping through all three).
            h.click("‹");
            assert_eq!(h.app.theme(), Theme::Lab);
            h.click("›");
            assert_eq!(h.app.theme(), Theme::Sewer);
            // The wheel over the sector row steps it too.
            let pos = h.locate("Sewer");
            h.events(&[], &[(MouseEventKind::ScrollDown, pos)]);
            assert_eq!(h.app.theme(), Theme::Kitchen);
            h.events(&[], &[(MouseEventKind::ScrollUp, pos)]);
            assert_eq!(h.app.theme(), Theme::Sewer);
            // A sector that has not been unlocked is marked on the title row.
            h.frame(&[KeyCode::Right]);
            h.frame(&[KeyCode::Right]);
            assert!(
                h.screen_text().contains("Lab ×"),
                "a locked sector is marked on the title"
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
            h.events(&[], &[(right, field)]);
            assert!(matches!(h.app.screen, Screen::Paused));
            h.events(&[], &[(left, (0, 0))]);
            assert!(matches!(h.app.screen, Screen::Paused));
            h.click("resume");
            assert!(matches!(h.app.screen, Screen::Playing));
            h.events(&[], &[(right, field)]);
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

    #[test]
    fn title_screen_renders() {
        let mut h = Harness::new(80, 40);
        let text = h.frame(&[]);
        assert!(text.contains("START"), "title should show START RUN");
        assert!(text.contains("QUIT"));
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

    /// Render a real frame and dump it as a PPM (each cell → 1×2 pixels:
    /// fg on top, bg below). Gated behind RATRUN_DUMP (value selects the
    /// screen: play|story|levelup) so it's a manual screenshot tool.
    #[test]
    fn dump_frame_ppm() {
        let target = match std::env::var("RATRUN_DUMP") {
            Ok(v) => v,
            Err(_) => return,
        };
        let (w, h) = (120u16, 46u16);
        let mut hh = Harness::new(w, h);
        hh.frame(&[]);

        match target.as_str() {
            "title" => {
                hh.frame(&[]);
            }
            "story" => {
                hh.frame(&[KeyCode::Char('l')]);
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
        std::fs::write(format!("/tmp/ratrun_{}.ppm", target), out).unwrap();
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

        // A mouse dash (left click on the field) is the same press.
        {
            let w = h.app.world.as_mut().unwrap();
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind.is_active() {
                    weapon.timer = 0.0;
                }
            }
            w.player.dash_cd = 3.0;
            w.player.dash_active = 0.0;
            // Let the previous burst finish so the new one is unambiguous.
            let mut fx = w.ecs.resource_mut::<crate::game::ecs::resources::ActiveFx>();
            fx.thrash_left = 0.0;
        }
        let field = h.locate("▀");
        h.events(
            &[],
            &[(MouseEventKind::Down(MouseButton::Left), field)],
        );
        assert!(
            h.app.world.as_ref().unwrap().active_fx().thrash_active(),
            "a click-dash must release a charged active weapon too"
        );
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
        assert_eq!(w, 100, "buffer spans the game viewport width");
        assert_eq!(hgt, 38 * 2, "two pixels per viewport row");

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
        assert_eq!((h.app.pixels.w, h.app.pixels.h), (80, 28 * 2));
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
                        hh.app.start_practice(1);
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
                        hh.app.world.as_mut().unwrap().level_index = 2;
                        hh.app.campaign.as_mut().unwrap().level = 2;
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
            "HUD should display EQUIPPED callout after upgrade pick"
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
        // hint); the promise that the run keeps going is one line under it.
        assert!(hh.map_text().contains("CLOSE"), "no art close hint");
        let screen = hh.screen_text();
        assert!(
            screen.contains("the run keeps running"),
            "the expanded map does not say the run keeps running"
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
    fn the_simulation_keeps_running_while_the_map_is_open() {
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[KeyCode::Tab]);
        let t0 = hh.app.world.as_ref().unwrap().elapsed;
        hh.frame(&[]);
        hh.frame(&[]);
        let t1 = hh.app.world.as_ref().unwrap().elapsed;
        assert!(t1 > t0, "the run froze while the map was open");
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

    #[test]
    fn the_boss_bearing_appears_on_the_map_after_three_minutes() {
        use crate::render::minimap::{fog_edge_cell, MinimapSource};
        let mut hh = Harness::new(120, 40);
        hh.enter_playing();
        hh.frame(&[]);
        {
            let w = hh.app.world.as_ref().unwrap();
            assert!(minimap_source::boss_bearing_dir(w).is_none(), "bearing too early");
        }
        let w = hh.app.world.as_mut().unwrap();
        w.elapsed = crate::render::minimap_source::ARENA_BEARING_AFTER_SECS + 1.0;
        let player_cell = (w.player.pos.x / 16.0, w.player.pos.y / 16.0);
        let dir = minimap_source::boss_bearing_dir(w).expect("no bearing after 180 s");
        // The art projects it onto the fog boundary, so it lands on explored
        // ground and never uncovers the route.
        let edge = fog_edge_cell(w, player_cell, dir).expect("no fog boundary on the bearing ray");
        let (ecx, ecy) = (edge.0.floor() as i32, edge.1.floor() as i32);
        assert!(w.seen(ecx, ecy), "the bearing marker is outside the fog");
        let arena_cell = w
            .level
            .boss_room()
            .map(|room| {
                let c = room.center();
                (c.x / 16.0, c.y / 16.0)
            })
            .unwrap();
        assert!(
            !w.seen(arena_cell.0.floor() as i32, arena_cell.1.floor() as i32),
            "the bearing must not uncover the arena"
        );
        // And the frame still draws with the bearing active.
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

    #[test]
    fn hidden_minimap_keeps_pre_warning_bearing_without_duplicating_hud_compass() {
        use crate::render::minimap::MinimapSize;

        let mut hh = Harness::new(80, 24);
        hh.enter_playing();
        hh.frame(&[KeyCode::Char('n')]); // Small
        hh.frame(&[KeyCode::Char('n')]); // Hidden
        assert_eq!(hh.app.scores.minimap_size, MinimapSize::Hidden);
        assert!(hh.app.last_minimap.is_empty());

        // 185s: arena bearing active (180..230s), HUD warning compass not yet active.
        hh.app.world.as_mut().unwrap().elapsed =
            crate::render::minimap_source::ARENA_BEARING_AFTER_SECS + 5.0;
        hh.frame(&[]);
        let game = hh.app.last_game;
        let corner = hh.terminal.backend().buffer()[(game.right() - 2, game.bottom() - 2)]
            .symbol()
            .to_string();
        assert!(
            matches!(
                corner.as_str(),
                "▲" | "◥" | "►" | "◢" | "▼" | "◣" | "◄" | "◤"
            ),
            "expected 1-cell arena bearing chevron when hidden at 185s, got {corner:?}"
        );

        // 240s: pre-boss warning active (230..255s); HUD compass now owns the
        // arena bearing so the hidden corner reverts to the quiet `N` hint.
        hh.app.world.as_mut().unwrap().elapsed = crate::game::director::BOSS_TIME - 15.0;
        hh.frame(&[]);
        let corner_warn = hh.terminal.backend().buffer()[(game.right() - 2, game.bottom() - 2)]
            .symbol()
            .to_string();
        assert_eq!(
            corner_warn, "N",
            "hidden corner must not duplicate the HUD warning compass"
        );
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
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        assert!(
            h.screen_text().contains("THE SWEEPER"),
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

        // -- Level 3 (Lab): the final boss wins the campaign -----------------
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "Victory");
        let text = h.screen_text();
        assert!(text.contains("EXTRACTION COMPLETE"), "no victory plate: {text}");
        // The breakdown names every sector's boss.
        for boss in ["APEX JAGUAR", "THE SWEEPER", "TURF-REAPER MK.IV"] {
            assert!(text.contains(boss), "{boss} missing from the breakdown: {text}");
        }
        assert!(text.contains("3 / 3"), "the sectors column must read 3/3");
        assert!(h.app.new_best, "the first campaign win is a new best");
        let entry = &h.app.scores.entries[0];
        assert!(entry.campaign_won, "the run records a campaign win");
        assert_eq!(entry.campaign_level, 3);
        assert!(entry.won);
        assert!(entry.score >= 2000, "cumulative score: {entry:?}");
        assert_eq!(h.app.records[0].time, 90.0, "level 1's clock was recorded");
        assert_eq!(h.app.records[0].score, 1200, "level 1's score was recorded");
        assert_eq!(h.app.records[0].kills, 40, "level 1's kills were recorded");
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
            text.contains("SECTOR 2/3"),
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

    /// Sector select reads the injected table: locked/unlocked states, the
    /// per-level bests, a locked sector refusing to start, a practice run
    /// labelled as such, and the unlock persisting through the store.
    #[test]
    fn sector_select_locks_unlocks_and_persists() {
        let mut table = ScoreTable::default();
        table.record_level(0, 6420, 252.0); // a previous run cleared the Sewer
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
            h.screen_text().contains("03 Lab ×"),
            "the Lab is still locked on the title row"
        );
        h.frame(&[KeyCode::Left, KeyCode::Left]); // back to sector 1
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "SectorSelect");
        let text = h.screen_text();
        assert!(text.contains("CLEARED"), "Sewer was cleared: {text}");
        assert!(text.contains("OPEN"), "Kitchen is open: {text}");
        assert!(text.contains("LOCKED"), "Lab is locked: {text}");
        assert!(text.contains("BEST 6420"), "the stored best score: {text}");
        assert!(text.contains("04:12"), "the stored best time: {text}");

        // A locked sector refuses to start and leaves the title state alone.
        h.frame(&[KeyCode::Right, KeyCode::Right]); // Sewer -> Kitchen -> Lab
        h.frame(&[KeyCode::Enter]);
        assert_eq!(
            h.app.screen.name_for_test(),
            "SectorSelect",
            "a locked sector must not start"
        );
        assert!(h.app.world.is_none());

        // Kitchen starts a labelled single-sector practice run.
        h.frame(&[KeyCode::Left]);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        assert_eq!(h.app.practice, Some(1));
        assert!(h.app.campaign.is_none());
        assert!(
            h.screen_text().contains("PRACTICE"),
            "the practice run must be labelled"
        );
        {
            let w = h.app.world.as_ref().unwrap();
            assert_eq!(w.level_index, 1);
            assert_eq!(
                w.player.loadout.weapons.len(),
                1,
                "practice uses the default loadout"
            );
        }

        // Clearing it records the sector (but never a campaign win).
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");

        // The play screens keep working on a later level: the minimap draws,
        // the level-up cards open, and the pause dossier carries the campaign
        // pips for the sector in flight.
        h.frame(&[]);
        assert!(
            !h.app.last_minimap.is_empty(),
            "the minimap must draw on level 2"
        );
        h.app.world.as_mut().unwrap().pending_levelups = 1;
        h.frame(&[]);
        assert_eq!(h.app.screen.name_for_test(), "LevelUp");
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        let paused = h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Paused");
        assert!(
            paused.contains("2/3"),
            "the pause dossier must carry the sector pips: {paused}"
        );
        h.frame(&[KeyCode::Esc]);
        assert_eq!(h.app.screen.name_for_test(), "Playing");
        h.finish_level();
        assert_eq!(h.app.screen.name_for_test(), "LevelComplete");
        h.frame(&[KeyCode::Char(' ')]);
        assert_eq!(h.app.screen.name_for_test(), "End");
        let entry = &h.app.scores.entries[0];
        assert_eq!(entry.campaign_level, 2);
        assert!(entry.won, "a cleared practice sector wins its own plate");
        assert!(!entry.campaign_won, "but it never wins the campaign");
        assert!(h.app.scores.level_cleared(1));
        assert!(
            h.app.scores.level_unlocked(2),
            "clearing Kitchen unlocks the Lab"
        );

        // The unlock survives a save/load through the injected store.
        let reloaded = ScoreTable::parse(&h.app.scores.to_json());
        assert!(reloaded.level_unlocked(2));
        assert_eq!(
            reloaded.level_best(1).map(|b| b.cleared),
            Some(true),
            "the Kitchen best was persisted"
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
                    "LevelIntro" => h.app.start_practice(1),
                    "Victory" => {
                        h.enter_playing();
                        h.app.campaign.as_mut().unwrap().level = 2;
                        h.app.world.as_mut().unwrap().level_index = 2;
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
        h.app.start_practice(1);
        assert_eq!(h.app.screen.name_for_test(), "LevelIntro");
        assert_eq!(h.app.practice, Some(1));
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

    /// The wedge tip is the lower half of the mouse cell: `▄` in cheese yellow
    /// over text, or the background of an existing `▀` playfield cell.
    fn is_cheese_tip(cell: &ratatui::buffer::Cell) -> bool {
        match cell.symbol() {
            "▄" => is_cheese_yellow(cell.fg),
            "▀" => is_cheese_yellow(cell.bg),
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
                "LevelIntro" => h.app.start_practice(1),
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
        // Enter on the Back row leaves the screen.
        h.frame(&[KeyCode::Down]);
        assert_eq!(h.app.options_row, options::BACK_ROW);
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.screen.name_for_test(), "Title");
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
}
