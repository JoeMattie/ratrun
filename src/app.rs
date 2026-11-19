//! Top-level application state machine: menus, the active run, and the
//! transitions between them.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::audio::{AudioEngine, Sfx};
use crate::game::gem::PickupKind;
use crate::game::level::Theme;
use crate::game::loadout::Upgrade;
use crate::game::world::World;
use crate::input::InputState;
use crate::lore;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::{hud, menu, minimap, minimap_source, overlay};
use crate::scores::{ScoreEntry, ScoreTable};

use menu::{HitMap, RunSummary, Target};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Title,
    Story,
    MapIntro,
    Playing,
    LevelUp,
    Paused,
    End, // game over or win (distinguished by world.won)
}

#[cfg(test)]
impl Screen {
    /// Screen name, so tests can assert which screen the app is on without
    /// deriving `PartialEq` for a production enum.
    fn name_for_test(&self) -> &'static str {
        match self {
            Screen::Title => "Title",
            Screen::Story => "Story",
            Screen::MapIntro => "MapIntro",
            Screen::Playing => "Playing",
            Screen::LevelUp => "LevelUp",
            Screen::Paused => "Paused",
            Screen::End => "End",
        }
    }
}

pub struct App {
    screen: Screen,
    pub should_quit: bool,
    pub input: InputState,
    audio: Option<AudioEngine>,
    scores: ScoreTable,
    theme_idx: usize,
    menu_idx: usize,
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
    /// Reused HUD text buffers, so the status line does not re-format every
    /// frame.
    hud: hud::HudState,
    hits: HitMap,
    muted: bool,
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
        let mut app = App {
            screen: Screen::Title,
            should_quit: false,
            input: InputState::new(kitty),
            audio,
            scores,
            theme_idx: 0,
            menu_idx: 0,
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
            hud: hud::HudState::default(),
            hits: HitMap::default(),
            muted: false,
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
        };
        app.menu_music();
        app
    }

    fn theme(&self) -> Theme {
        Theme::all()[self.theme_idx]
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
        if let Some(a) = self.audio.as_mut() {
            a.play_music(theme, 0); // calm: bass only
        }
    }

    fn sfx(&self, sfx: Sfx) {
        if let Some(a) = self.audio.as_ref() {
            a.play_sfx(sfx);
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

    fn start_run(&mut self) {
        let theme = self.theme();
        let seed = self.seed();
        self.world = Some(World::new(theme, seed));
        self.recorded = false;
        self.new_best = false;
        self.damage_taken = 0.0;
        self.dashes_used = 0;
        self.heals_picked = 0;
        self.boss_slain = false;
        self.prev_boss_alive = false;
        self.last_summary = None;
        // Fog of war resets with the run: a new `World` starts unexplored.
        self.map_open = false;
        self.last_minimap = Rect::new(0, 0, 0, 0);
        self.last_map = Rect::new(0, 0, 0, 0);
        self.hud.reset();
        self.intro_timer = 4.0;
        self.screen = Screen::MapIntro;
        if let Some(a) = self.audio.as_mut() {
            a.play_music(theme, 1);
        }
    }

    fn cycle_map(&mut self, backwards: bool) {
        let n = Theme::all().len();
        self.theme_idx = (self.theme_idx + if backwards { n - 1 } else { 1 }) % n;
        self.menu_music();
        self.sfx(Sfx::UiMove);
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
                    if matches!(target, Some(Target::NextMap | Target::PreviousMap)) {
                        self.menu_idx = 1;
                        self.cycle_map(ev.kind == MouseEventKind::ScrollUp);
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
                        Some(Target::NextMap | Target::PreviousMap) => self.menu_idx = 1,
                        Some(Target::Quit) => self.menu_idx = 2,
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
        // Global mute toggle.
        if self.input.pressed_char('m') {
            self.muted = !self.muted;
            self.hud.telemetry.muted = self.muted;
            if let Some(a) = self.audio.as_mut() {
                a.toggle_mute();
            }
        }
        let previous = self.screen;
        let click = self.mouse_action();
        match self.screen {
            Screen::Title => self.update_title(click),
            Screen::Story => {
                self.story_time += dt;
                self.update_story(click);
            }
            Screen::MapIntro => self.update_mapintro(dt, click),
            Screen::Playing => self.update_playing(dt, click),
            Screen::LevelUp => self.update_levelup(click),
            Screen::Paused => self.update_paused(click),
            Screen::End => self.update_end(click),
        }
        if self.screen != previous {
            self.hits.clear();
            self.hits.hovered = None;
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
                self.menu_idx = (self.menu_idx + 2) % 3;
                self.sfx(Sfx::UiMove);
            }
            if self.input.just_pressed(KeyCode::Down) {
                self.menu_idx = (self.menu_idx + 1) % 3;
                self.sfx(Sfx::UiMove);
            }
            if self.input.just_pressed(KeyCode::Left) {
                self.cycle_map(true);
            }
            if self.input.just_pressed(KeyCode::Right) {
                self.cycle_map(false);
            }
            self.input
                .just_pressed(KeyCode::Enter)
                .then(|| [Target::Start, Target::NextMap, Target::Quit][self.menu_idx])
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
            Some(Target::NextMap) => self.cycle_map(false),
            Some(Target::PreviousMap) => self.cycle_map(true),
            _ => {}
        }
    }

    fn update_story(&mut self, click: Option<Target>) {
        if click == Some(Target::Back)
            || self.input.just_pressed(KeyCode::Enter)
            || self.input.just_pressed(KeyCode::Esc)
            || self.input.pressed_char('q')
        {
            self.sfx(Sfx::UiSelect);
            self.screen = Screen::Title;
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
            intensity = if boss_alive || enemy_count > 45 {
                2
            } else {
                1
            };
            // A finished run wins over pending upgrades: dying (or winning) on
            // the same tick as an XP award must not open the level-up cards.
            if w.finished() {
                ended = true;
            } else if w.pending_levelups > 0 {
                open_levelup = true;
            }
        }
        for i in 0..self.sfx_frame.len() {
            let s = self.sfx_frame[i];
            self.sfx(s);
        }
        if let Some(a) = self.audio.as_ref() {
            a.set_intensity(intensity);
        }
        if open_levelup {
            self.open_levelup();
        } else if ended {
            self.finalize();
            self.screen = Screen::End;
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
        if click == Some(Target::Resume)
            || self.input.just_pressed(KeyCode::Esc)
            || self.input.pressed_char('p')
        {
            self.screen = Screen::Playing;
        }
        if click == Some(Target::Menu) || self.input.pressed_char('q') {
            self.world = None;
            self.screen = Screen::Title;
            self.menu_music();
        }
    }

    fn update_end(&mut self, click: Option<Target>) {
        if click == Some(Target::Restart) || self.input.just_pressed(KeyCode::Enter) {
            self.start_run();
        }
        if click == Some(Target::Menu) || self.input.pressed_char('q') {
            self.world = None;
            self.screen = Screen::Title;
            self.menu_music();
        }
    }

    fn build_summary(&self, w: &World, final_score: u32) -> RunSummary {
        let bonus_time = (w.elapsed as u32) * 5;
        let bonus_level = w.player.level * 100;
        let bonus_win = if w.won { 5000 } else { 0 };
        RunSummary {
            won: w.won,
            theme: w.level.theme,
            combat_score: w.score,
            bonus_time,
            bonus_level,
            bonus_win,
            final_score,
            time: w.elapsed,
            level: w.player.level,
            kills: w.kills,
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
        }
    }

    fn finalize(&mut self) {
        if self.recorded {
            return;
        }
        if let Some(w) = self.world.as_ref() {
            let bonus_time = (w.elapsed as u32) * 5;
            let bonus_level = w.player.level * 100;
            let bonus_win = if w.won { 5000 } else { 0 };
            let final_score = w.score + bonus_time + bonus_level + bonus_win;
            self.last_score = final_score;
            self.last_summary = Some(self.build_summary(w, final_score));
            // "New high score" means it beat the table's best, computed before
            // this run's entry is inserted (an under-full table is no free pass).
            self.new_best = self.scores.is_high_score(final_score);
            self.scores.insert(ScoreEntry {
                score: final_score,
                time: w.elapsed,
                level: w.player.level,
                map: w.level.theme.name().to_string(),
                won: w.won,
            });
            if self.persist {
                self.scores.save();
            }
        }
        self.recorded = true;
        // Music settles, plus a sting.
        if let Some(a) = self.audio.as_ref() {
            a.set_intensity(0);
        }
        let won = self.world.as_ref().map(|w| w.won).unwrap_or(false);
        self.sfx(if won { Sfx::Win } else { Sfx::Lose });
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
                    &self.scores,
                    &self.rat_art,
                    &mut self.hits,
                );
            }
            Screen::Story => {
                self.ensure_rat_art(area);
                menu::draw_title(
                    frame,
                    area,
                    self.theme(),
                    self.menu_idx,
                    &self.scores,
                    &self.rat_art,
                    &mut self.hits,
                );
                menu::draw_story(frame, area, self.story_time, &mut self.hits);
            }
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
                            let lore_line = if w.won {
                                lore::victory(w.level.theme)
                            } else {
                                lore::DEFEAT
                            };
                            let fallback;
                            let summary = match self.last_summary.as_ref() {
                                Some(s) => s,
                                None => {
                                    fallback = self.build_summary(w, self.last_score);
                                    &fallback
                                }
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
        let view = self.map_frame.view(w, expanded, hovered);
        minimap::draw(frame, game, &view, w, &mut self.minimap_scratch);
        if expanded {
            let panel = minimap_source::expanded_rect(game);
            if panel.is_empty() {
                return;
            }
            self.last_map = panel;
            draw_map_legend(frame, game, panel);
        } else {
            let panel = minimap_source::compact_rect(game);
            if panel.is_empty() {
                return;
            }
            self.last_minimap = panel;
        }
    }
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
    let text = if game.width >= 60 {
        " TAB/Esc close · click the map · the run keeps running "
    } else {
        " TAB/Esc close · run continues "
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

        /// Start a run and skip the map intro, landing in `Playing`.
        fn enter_playing(&mut self) {
            self.frame(&[]); // title
            self.frame(&[KeyCode::Enter]); // START RUN -> MapIntro
            self.frame(&[KeyCode::Enter]); // skip intro -> Playing
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
    fn mouse_title_map_story_and_quit() {
        for size in [(57, 20), (80, 16)] {
            let mut h = Harness::new(size.0, size.1);
            h.frame(&[]);
            let name = h.locate("Sewer");
            h.events(&[], &[(MouseEventKind::Moved, name)]);
            assert_eq!(h.app.menu_idx, 1);
            h.click("‹");
            assert_eq!(h.app.theme(), Theme::Lab);
            h.click("›");
            h.click("Sewer");
            assert_eq!(h.app.theme(), Theme::Kitchen);
            let pos = h.locate("Kitchen");
            h.events(&[], &[(MouseEventKind::ScrollUp, pos)]);
            assert_eq!(h.app.theme(), Theme::Sewer);
            h.events(
                &[],
                &[
                    (MouseEventKind::ScrollDown, pos),
                    (MouseEventKind::ScrollDown, pos),
                ],
            );
            assert_eq!(h.app.theme(), Theme::Lab);
            h.events(&[], &[(MouseEventKind::ScrollUp, (0, 0))]);
            assert_eq!(h.app.theme(), Theme::Lab);
            h.click("L story");
            assert!(matches!(h.app.screen, Screen::Story));
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
            h.events(&[], &[(left, start), (left, start)]);
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
        assert!(matches!(h.app.screen, Screen::MapIntro));
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
            "levelup" => {
                hh.frame(&[KeyCode::Enter]); // -> MapIntro
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
                hh.frame(&[KeyCode::Enter]); // -> MapIntro
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

    #[test]
    fn player_follows_mouse_cursor() {
        let mut h = Harness::new(100, 44);
        h.frame(&[]); // title
        h.frame(&[KeyCode::Enter]); // -> MapIntro
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
        h.frame(&[KeyCode::Enter]); // START RUN -> Playing

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
                text.contains("Map"),
                "{w}x{h} title should show the map row"
            );
            assert!(text.contains("QUIT"), "{w}x{h} title should show QUIT");
            assert!(
                text.contains("Enter confirm"),
                "{w}x{h} title should keep the key legend"
            );
        }
    }

    /// Item 5: Enter on the "Map" row cycles the map instead of doing nothing.
    #[test]
    fn enter_on_map_row_cycles_theme() {
        let mut h = Harness::new(80, 40);
        h.frame(&[]);
        h.frame(&[KeyCode::Down]); // select the Map row
        assert_eq!(h.app.menu_idx, 1);
        let before = h.app.theme_idx;
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.theme_idx, (before + 1) % Theme::all().len());
        assert!(
            matches!(h.app.screen, Screen::Title),
            "must stay on the title"
        );
        h.frame(&[KeyCode::Enter]);
        assert_eq!(h.app.theme_idx, (before + 2) % Theme::all().len());
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
                "Title", "Story", "MapIntro", "Playing", "LevelUp", "Paused", "End",
            ] {
                let mut hh = Harness::with_app(test_app(), w, h);
                hh.frame(&[]);
                match target {
                    "Title" => {}
                    "Story" => {
                        hh.frame(&[KeyCode::Char('l')]);
                    }
                    "MapIntro" => {
                        hh.frame(&[KeyCode::Enter]);
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
            h.app.theme_idx = theme_idx;
            h.enter_playing();
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

}
