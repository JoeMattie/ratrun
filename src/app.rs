//! Top-level application state machine: menus, the active run, and the
//! transitions between them.

use std::collections::HashSet;

use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::audio::{AudioEngine, Sfx};
use crate::game::director;
use crate::game::enemy::EnemyKind;
use crate::game::level::Theme;
use crate::game::loadout::Upgrade;
use crate::game::world::World;
use crate::input::InputState;
use crate::lore;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::{hud, menu};
use crate::scores::{ScoreEntry, ScoreTable};

enum Screen {
    Title,
    Story,
    MapIntro,
    Playing,
    LevelUp,
    Paused,
    End, // game over or win (distinguished by world.won)
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
    /// Game viewport rect from the last frame, for mouse→world mapping.
    last_game: Rect,
    run_counter: u64,
    recorded: bool,
    new_best: bool,
    last_score: u32,
}

impl App {
    pub fn new(kitty: bool, audio: Option<AudioEngine>) -> Self {
        let mut app = App {
            screen: Screen::Title,
            should_quit: false,
            input: InputState::new(kitty),
            audio,
            scores: ScoreTable::load(),
            theme_idx: 0,
            menu_idx: 0,
            world: None,
            levelup_choices: Vec::new(),
            levelup_desc: Vec::new(),
            levelup_idx: 0,
            intro_timer: 0.0,
            last_game: Rect::new(0, 0, 0, 0),
            run_counter: 0,
            recorded: false,
            new_best: false,
            last_score: 0,
        };
        app.menu_music();
        app
    }

    fn theme(&self) -> Theme {
        Theme::all()[self.theme_idx]
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
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        nanos ^ (self.run_counter.wrapping_mul(0x9E3779B97F4A7C15))
    }

    fn start_run(&mut self) {
        let theme = self.theme();
        let seed = self.seed();
        self.world = Some(World::new(theme, seed));
        self.recorded = false;
        self.new_best = false;
        self.intro_timer = 4.0;
        self.screen = Screen::MapIntro;
        if let Some(a) = self.audio.as_mut() {
            a.play_music(theme, 1);
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.input.tick(dt);
        // Global mute toggle.
        if self.input.pressed_char('m') {
            if let Some(a) = self.audio.as_mut() {
                a.toggle_mute();
            }
        }
        match self.screen {
            Screen::Title => self.update_title(),
            Screen::Story => self.update_story(),
            Screen::MapIntro => self.update_mapintro(dt),
            Screen::Playing => self.update_playing(dt),
            Screen::LevelUp => self.update_levelup(),
            Screen::Paused => self.update_paused(),
            Screen::End => self.update_end(),
        }
    }

    fn update_title(&mut self) {
        if self.input.pressed_char('q') {
            self.should_quit = true;
            return;
        }
        if self.input.pressed_char('l') {
            self.sfx(Sfx::UiSelect);
            self.screen = Screen::Story;
            return;
        }
        if self.input.just_pressed(KeyCode::Up) {
            self.menu_idx = (self.menu_idx + 2) % 3;
            self.sfx(Sfx::UiMove);
        }
        if self.input.just_pressed(KeyCode::Down) {
            self.menu_idx = (self.menu_idx + 1) % 3;
            self.sfx(Sfx::UiMove);
        }
        let n = Theme::all().len();
        if self.input.just_pressed(KeyCode::Left) {
            self.theme_idx = (self.theme_idx + n - 1) % n;
            self.menu_music();
            self.sfx(Sfx::UiMove);
        }
        if self.input.just_pressed(KeyCode::Right) {
            self.theme_idx = (self.theme_idx + 1) % n;
            self.menu_music();
            self.sfx(Sfx::UiMove);
        }
        if self.input.just_pressed(KeyCode::Enter) {
            match self.menu_idx {
                0 => {
                    self.sfx(Sfx::UiSelect);
                    self.start_run();
                }
                2 => self.should_quit = true,
                _ => {}
            }
        }
    }

    fn update_story(&mut self) {
        if self.input.just_pressed(KeyCode::Enter)
            || self.input.just_pressed(KeyCode::Esc)
            || self.input.pressed_char('q')
        {
            self.sfx(Sfx::UiSelect);
            self.screen = Screen::Title;
        }
    }

    fn update_mapintro(&mut self, dt: f32) {
        self.intro_timer -= dt;
        if self.intro_timer <= 0.0
            || self.input.just_pressed(KeyCode::Enter)
            || self.input.just_pressed(KeyCode::Char(' '))
        {
            self.screen = Screen::Playing;
        }
    }

    fn update_playing(&mut self, dt: f32) {
        if self.input.just_pressed(KeyCode::Esc) || self.input.pressed_char('p') {
            self.screen = Screen::Paused;
            return;
        }
        let move_dir = self.resolve_move_dir();
        let dash = self.input.just_pressed(KeyCode::Char(' '));
        let mut open_levelup = false;
        let mut ended = false;
        let mut events: Vec<Sfx> = Vec::new();
        let mut intensity = 1u32;
        if let Some(w) = self.world.as_mut() {
            w.update(dt, move_dir, dash);
            // Drain SFX, de-duplicated per frame so a 16-shot nova is one sound.
            let mut seen = HashSet::new();
            for s in w.sfx.drain(..) {
                if seen.insert(s) {
                    events.push(s);
                }
            }
            let boss_alive = w.enemies.iter().any(|e| e.kind == EnemyKind::Boss);
            intensity = if boss_alive || w.enemies.len() > 45 { 2 } else { 1 };
            if w.pending_levelups > 0 {
                open_levelup = true;
            } else if w.finished() {
                ended = true;
            }
        }
        for s in events {
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
        if let Some(w) = self.world.as_mut() {
            let choices = w.player.loadout.generate_choices(&mut w.rng);
            self.levelup_desc = choices.iter().map(|c| w.player.loadout.describe(c)).collect();
            self.levelup_choices = choices;
            self.levelup_idx = 0;
            self.screen = Screen::LevelUp;
        }
    }

    fn update_levelup(&mut self) {
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
        if let Some(i) = pick {
            self.sfx(Sfx::UiSelect);
            self.apply_levelup(i);
        }
    }

    fn apply_levelup(&mut self, i: usize) {
        if let Some(w) = self.world.as_mut() {
            if let Some(up) = self.levelup_choices.get(i).cloned() {
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

    fn update_paused(&mut self) {
        if self.input.just_pressed(KeyCode::Esc) || self.input.pressed_char('p') {
            self.screen = Screen::Playing;
        }
        if self.input.pressed_char('q') {
            self.world = None;
            self.screen = Screen::Title;
            self.menu_music();
        }
    }

    fn update_end(&mut self) {
        if self.input.just_pressed(KeyCode::Enter) {
            self.start_run();
        }
        if self.input.pressed_char('q') {
            self.world = None;
            self.screen = Screen::Title;
            self.menu_music();
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
            self.new_best = self.scores.is_high_score(final_score);
            self.scores.insert(ScoreEntry {
                score: final_score,
                time: w.elapsed,
                level: w.player.level,
                map: w.level.theme.name().to_string(),
                won: w.won,
            });
            self.scores.save();
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
        match self.screen {
            Screen::Title => {
                menu::draw_title(frame, area, self.theme(), self.menu_idx, &self.scores);
            }
            Screen::Story => {
                menu::draw_title(frame, area, self.theme(), self.menu_idx, &self.scores);
                menu::draw_story(frame, area);
            }
            Screen::MapIntro | Screen::Playing | Screen::Paused | Screen::LevelUp | Screen::End => {
                self.draw_game(frame, area);
                match self.screen {
                    Screen::MapIntro => menu::draw_map_intro(frame, area, self.theme()),
                    Screen::Paused => menu::draw_pause(frame, area),
                    Screen::LevelUp => {
                        let level = self.world.as_ref().map(|w| w.player.level).unwrap_or(1);
                        menu::draw_levelup(frame, area, level, &self.levelup_desc, self.levelup_idx);
                    }
                    Screen::End => {
                        if let Some(w) = self.world.as_ref() {
                            let lore_line = if w.won {
                                lore::victory(w.level.theme)
                            } else {
                                lore::DEFEAT
                            };
                            menu::draw_end(
                                frame,
                                area,
                                w.won,
                                self.last_score,
                                w.elapsed,
                                w.player.level,
                                w.kills,
                                self.new_best,
                                lore_line,
                                &self.scores,
                            );
                        }
                    }
                    _ => {}
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
                let mut pb = PixelBuffer::new(game.width as usize, game.height as usize * 2);
                w.draw(&mut pb);
                pb.render_to(game, frame.buffer_mut());
            }
        } else {
            frame.render_widget(Block::default().style(Style::default().bg(Color::Black)), game);
        }

        if let Some(w) = self.world.as_ref() {
            hud::draw_top(frame, top, w);
            hud::draw_bottom(frame, bottom, w);
        }
        let _ = director::RUN_SECONDS; // keep module referenced for clarity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// End-to-end harness: drives the real `App` through ratatui's `Terminal`
    /// + in-memory `Buffer`, exactly like `main`'s loop but without crossterm's
    /// event source or a TTY. Returns the rendered screen as text.
    struct Harness {
        app: App,
        terminal: Terminal<TestBackend>,
    }

    impl Harness {
        fn new(w: u16, h: u16) -> Self {
            Harness {
                app: App::new(false, None), // silent in tests
                terminal: Terminal::new(TestBackend::new(w, h)).unwrap(),
            }
        }

        /// Advance one frame, injecting the given keys this frame.
        fn frame(&mut self, keys: &[KeyCode]) -> String {
            self.app.input.begin_frame();
            for k in keys {
                self.app.input.handle_key(KeyEvent::from(*k));
            }
            self.app.update(1.0 / 60.0);
            self.terminal.draw(|f| self.app.render(f)).unwrap();
            self.screen_text()
        }

        fn set_mouse(&mut self, col: u16, row: u16) {
            self.app.input.handle_mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: col,
                row: row,
                modifiers: KeyModifiers::NONE,
            });
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
        assert!(x1 > x0 + 10.0, "player should chase the cursor right ({x0}→{x1})");

        // Now far left → player reverses.
        for _ in 0..120 {
            h.set_mouse(2, 22);
            h.frame(&[]);
        }
        let x2 = h.player_x();
        assert!(x2 < x1 - 10.0, "player should chase the cursor left ({x1}→{x2})");
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
    }
}
