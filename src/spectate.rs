//! `ratrun --ai`: a wall of self-playing games.
//!
//! The owner wanted to see whether the game is beatable and to watch many runs
//! at once: one terminal window, N games tiled, every game driven by
//! [`crate::autoplay::AutoRun`] on its own worker thread and sped up.
//!
//! This module owns only the **wall**: the grid layout, the per-tile render
//! (each tile is a real `World::draw` into its own half-block `PixelBuffer`),
//! the captions/outcome plates, the summary strip and the input handling. The
//! simulation itself lives in `autoplay.rs`; the wall never constructs an
//! `AudioEngine`, so music and effects are silent in this mode.
//!
//! ## Threading
//!
//! Every game runs on one worker thread that owns its `AutoRun`. The main
//! thread only drains each worker's latest rendered frame, composites the wall
//! and handles input, so it never blocks on a slow game. Speed, pause, restart
//! and tile-size requests travel to the workers as atomics; frames travel back
//! through a bounded channel (`try_send`, capacity 1), so a worker whose frame
//! the main loop has not consumed yet simply drops it instead of queueing.
//! A separate channel delivers each run's outcome even if its frames are dropped.
//! At MAX, workers run fixed ticks continuously and draw at most once per
//! display interval while running.
//!
//! ## Grid
//!
//! [`plan_grid`] searches column counts for the tile shape closest to the play
//! field's 2:1 cell aspect, weighted towards a square-ish tile count and away
//! from empty slots. If even the minimum tile (see [`MIN_TILE_W`] /
//! [`MIN_TILE_H`]) does not fit, it falls back to showing as many tiles as
//! possible and reports the rest as hidden.

// Wall-only: the platform maths is fine here (a tile-size fit, no simulation).
#![allow(clippy::disallowed_methods)]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{KeyCode, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;

use crate::autoplay::batch::SKILL_CYCLE;
use crate::autoplay::{AutoOpts, AutoRun, AutoStatus};
use crate::game::bot::Skill;
use crate::game::campaign;
use crate::input::InputState;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette;

/// Games in the wall (`ratrun --ai` with no count).
pub const DEFAULT_GAMES: usize = 16;
/// Largest wall the flag accepts (`--ai N`).
pub const MAX_GAMES: usize = 32;
/// A tile narrower than this is not worth showing; the wall hides games first.
pub const MIN_TILE_W: u16 = 16;
/// A tile shorter than this has no room for world + caption.
pub const MIN_TILE_H: u16 = 5;
/// The play field is `160 x 80` px = `80 x 40` half-block cells, i.e. 2:1.
const TARGET_TILE_ASPECT: f32 = 2.0;
/// Speed ladder, in simulation seconds per wall second.
pub const SPEEDS: [f32; 7] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, f32::INFINITY];
/// Default ladder position: 8x.
pub const DEFAULT_SPEED_IDX: usize = 3;
/// The simulation's fixed tick: speeding up runs more ticks, never longer ones.
pub const SIM_DT: f32 = 1.0 / 60.0;
const MAX_FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// How long a finished run's outcome plate stays up before it restarts.
pub const RESTART_DELAY_SECS: f32 = 3.0;

// ---- Grid layout ---------------------------------------------------------

/// One grid of tiles: how many fit, and their cell size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub tile_w: u16,
    pub tile_h: u16,
    /// Games this grid can show.
    pub capacity: usize,
    /// Games actually drawn (`min(games, capacity)`, and 0 at a zero size).
    pub visible: usize,
    /// Games the wall cannot show.
    pub hidden: usize,
}

impl Grid {
    /// The rect of game `i`, centred in `area` so leftover columns/rows are
    /// split evenly. `None` when the game is not visible.
    pub fn tile_rect(&self, area: Rect, i: usize) -> Option<Rect> {
        if i >= self.visible || self.tile_w == 0 || self.tile_h == 0 {
            return None;
        }
        let used_w = self.cols.saturating_mul(self.tile_w);
        let used_h = self.rows.saturating_mul(self.tile_h);
        let off_x = area.x + area.width.saturating_sub(used_w) / 2;
        let off_y = area.y + area.height.saturating_sub(used_h) / 2;
        let col = i as u16 % self.cols.max(1);
        let row = i as u16 / self.cols.max(1);
        Some(Rect::new(
            off_x + col * self.tile_w,
            off_y + row * self.tile_h,
            self.tile_w,
            self.tile_h,
        ))
    }
}

/// The whole wall's layout for one frame: the grid plus the strip rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallPlan {
    pub grid: Grid,
    pub grid_area: Rect,
    pub summary: Option<Rect>,
}

impl WallPlan {
    pub fn tile_rect(&self, i: usize) -> Option<Rect> {
        self.grid.tile_rect(self.grid_area, i)
    }
}

/// Plan the wall for `games` games in `area`: reserve the summary strip, then
/// lay the games out in the grid [`plan_grid`] chooses.
pub fn plan_wall(games: usize, area: Rect) -> WallPlan {
    let games = games.max(1);
    let summary_rows = if (57..144).contains(&area.width) && area.height >= 20 {
        2
    } else {
        1
    };
    let (grid_area, summary) = if area.height > summary_rows {
        (
            Rect::new(area.x, area.y, area.width, area.height - summary_rows),
            Some(Rect::new(
                area.x,
                area.y + area.height - summary_rows,
                area.width,
                summary_rows,
            )),
        )
    } else {
        (area, None)
    };
    WallPlan {
        grid: plan_grid(games, grid_area),
        grid_area,
        summary,
    }
}

/// Choose the grid for `games` games in `area`.
///
/// The search covers column *and* row counts: a grid whose capacity is smaller
/// than `games` simply shows fewer games (`Grid::hidden` says how many), which
/// is what small terminals need. Candidates must meet [`MIN_TILE_W`] x
/// [`MIN_TILE_H`] and are scored by
/// `|log2(tile_aspect / 2)| + 2 * |cols - sqrt(visible)|/sqrt(visible)` +
/// `4 * hidden/total + 0.3 * waste`: tiles close to the play field's aspect, a
/// square-ish visible count, as many games shown as possible and few empty
/// slots. When nothing meets the minimum, [`fallback_grid`] picks a cruder
/// layout so a tiny terminal still draws something instead of panicking.
pub fn plan_grid(games: usize, area: Rect) -> Grid {
    let games = games.max(1);
    let (w, h) = (area.width, area.height);
    let mut best: Option<(f32, u16, u16, usize)> = None;
    for cols in 1..=games as u16 {
        for rows in 1..=games as u16 {
            let tw = w / cols;
            let th = h / rows;
            if tw < MIN_TILE_W || th < MIN_TILE_H {
                continue;
            }
            let capacity = cols as usize * rows as usize;
            let visible = capacity.min(games);
            let aspect = tw as f32 / th as f32;
            let aspect_err = (aspect / TARGET_TILE_ASPECT).log2().abs();
            let target = (visible as f32).sqrt().max(1.0);
            let balance = (cols as f32 - target).abs() / target;
            let hidden_frac = (games - visible) as f32 / games as f32;
            let waste = (capacity - visible) as f32 / capacity.max(1) as f32;
            // Showing every game outranks a nicer aspect: a hidden game is
            // a game the owner asked to watch.
            let score = aspect_err + 2.0 * balance + 4.0 * hidden_frac + 0.3 * waste;
            if best.is_none_or(|(s, _, _, _)| score < s) {
                best = Some((score, cols, rows, visible));
            }
        }
    }

    let (cols, rows, visible) = match best {
        Some((_, c, r, v)) => (c, r, v),
        None => {
            let (c, r) = fallback_grid(games, w, h);
            let tw = w / c.max(1);
            let th = h / r.max(1);
            let v = if tw == 0 || th == 0 {
                0
            } else {
                (c as usize * r as usize).min(games)
            };
            (c, r, v)
        }
    };
    let tw = w / cols.max(1);
    let th = h / rows.max(1);
    let capacity = (cols as usize * rows as usize).max(1);
    let visible = if tw == 0 || th == 0 { 0 } else { visible };
    Grid {
        cols,
        rows,
        tile_w: tw,
        tile_h: th,
        capacity,
        visible,
        hidden: games - visible,
    }
}

fn rows_for(games: usize, cols: u16) -> u16 {
    let cols = cols.max(1) as usize;
    games.div_ceil(cols) as u16
}

/// The best grid when no candidate reaches the minimum tile size: prefer
/// legible tiles (8x3), then small ones (4x2), then any non-empty tile, and
/// within a tier prefer showing more games, then the larger tile.
fn fallback_grid(games: usize, w: u16, h: u16) -> (u16, u16) {
    for (min_w, min_h) in [(8u16, 3u16), (4, 2), (1, 1)] {
        let mut best: Option<(u32, u16, u16)> = None;
        for cols in 1..=games as u16 {
            let rows = rows_for(games, cols).max(1);
            // Try every row count too: a grid may show fewer games.
            for rows in 1..=rows {
                let tw = w / cols;
                let th = h / rows;
                if tw < min_w || th < min_h {
                    continue;
                }
                let capacity = cols as usize * rows as usize;
                let visible = capacity.min(games) as u32;
                let score = visible * 100_000 + tw as u32 * th as u32;
                if best.is_none_or(|(s, _, _)| score > s) {
                    best = Some((score, cols, rows));
                }
            }
        }
        if let Some((_, c, r)) = best {
            return (c, r);
        }
    }
    (1, 1)
}

// ---- Tile snapshots ------------------------------------------------------

/// What one game is doing, as the wall needs it.
#[derive(Clone, Debug, PartialEq)]
pub enum TileState {
    Running,
    Died { at: f32, cause: String },
    Won { total_time: f32 },
    /// The worker caught a panic and the run was discarded (a level-generation
    /// or simulation bug). The wall shows a plate and retries with a new seed.
    Failed { reason: String },
}

impl TileState {
    pub fn is_finished(&self) -> bool {
        !matches!(self, TileState::Running)
    }

    /// One ASCII glyph, safe to count as one cell in any terminal.
    pub fn glyph(&self) -> char {
        match self {
            TileState::Running => '>',
            TileState::Died { .. } => 'X',
            TileState::Won { .. } => '*',
            TileState::Failed { .. } => '!',
        }
    }
}

/// A snapshot of one game's run, enough to draw its caption/plate and to
/// aggregate the wall's summary.
#[derive(Clone, Debug, PartialEq)]
pub struct TileInfo {
    /// 0-based game number (the caption shows `+1`).
    pub index: usize,
    /// Campaign level index, 0..LEVEL_COUNT-1.
    pub level: usize,
    /// The level's theme initial (`S`, `K`, `L`).
    pub theme: char,
    /// Level clock.
    pub elapsed: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub player_level: u32,
    pub kills: u32,
    pub score: u32,
    /// `(hp fraction, phase)` while a boss is alive.
    pub boss: Option<(f32, u8)>,
    pub state: TileState,
}

impl TileInfo {
    pub fn hp_frac(&self) -> f32 {
        if self.max_hp > 0.0 {
            (self.hp / self.max_hp).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The one-line caption, fitted to `width` cells.
    ///
    /// Every form carries the game number, level+theme, clock, an HP bar, the
    /// player level, kills and the state glyph; the forms get progressively
    /// terser (`K12` → `12`, a 3-cell bar → 2 cells → none) so a 20-cell tile
    /// still shows all of them and a 16-cell tile drops only the bar.
    pub fn caption(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let num = format!("#{}", self.index + 1);
        let lvl = format!("{}{}", self.level + 1, self.theme);
        let t = clock(self.elapsed);
        let st = self.state.glyph();
        let lv = format!("L{}", self.player_level);
        let hp3 = hp_bar(self.hp_frac(), 3);
        let hp2 = hp_bar(self.hp_frac(), 2);
        let k = self.kills;
        let candidates = [
            format!("{num} {lvl} {t} {hp3} {lv} K{k} {st}"),
            format!("{num} {lvl} {t} {hp3} {lv} K{k}{st}"),
            format!("{num} {lvl} {t} {hp3} {lv} {k}{st}"),
            format!("{num} {lvl} {t} {hp2} {lv} {k}{st}"),
            format!("{num} {lvl} {t} {lv} {st}"),
            format!("{num} {lvl} {t}{st}"),
            format!("{lvl} {t}{st}"),
            format!("{lvl}{st}"),
            st.to_string(),
        ];
        for c in candidates {
            if c.chars().count() <= width {
                return c;
            }
        }
        String::new()
    }

    /// The richer two-line caption the zoomed game gets (the grid tile gets
    /// [`TileInfo::caption`] only).
    pub fn zoom_caption(&self, width: usize) -> String {
        let name = campaign::level_theme(self.level).name();
        let boss = match self.boss {
            Some((frac, phase)) => format!(
                "  boss {} P{}",
                hp_bar(frac, 6),
                phase
            ),
            None => String::new(),
        };
        let text = format!(
            "GAME {}  {} (L{})  {}  HP {}/{} {}  LV{}  KILLS {}  SCORE {}{}  {}",
            self.index + 1,
            name,
            self.level + 1,
            clock(self.elapsed),
            self.hp.max(0.0).ceil() as u32,
            self.max_hp.max(0.0).ceil() as u32,
            hp_bar(self.hp_frac(), 10),
            self.player_level,
            self.kills,
            self.score,
            boss,
            self.state_text(),
        );
        text.chars().take(width.max(1)).collect()
    }

    /// Human state text for plates and the zoom caption.
    pub fn state_text(&self) -> String {
        match &self.state {
            TileState::Running => "RUNNING".to_string(),
            TileState::Died { at, cause } => format!("DIED at {} to {cause}", clock(*at)),
            TileState::Won { total_time } => format!("ESCAPED in {}", clock(*total_time)),
            TileState::Failed { reason } => format!("FAILED: {reason}"),
        }
    }
}

/// `m:ss`, saturating at 99:59 so the caption width never changes.
pub fn clock(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    let s = s.min(99 * 60 + 59);
    format!("{}:{:02}", s / 60, s % 60)
}

/// An ASCII HP bar (`#` filled, `-` empty) so captions never depend on unicode
/// width.
pub fn hp_bar(frac: f32, cells: usize) -> String {
    let cells = cells.max(1);
    let filled = ((frac.clamp(0.0, 1.0) * cells as f32).round() as usize).min(cells);
    let mut out = String::with_capacity(cells);
    for i in 0..cells {
        out.push(if i < filled { '#' } else { '-' });
    }
    out
}

/// One worker's latest rendered tile plus its snapshot.
#[derive(Clone, Debug)]
pub struct TileFrame {
    /// The world rendered at the tile's world size (half-block pixels already
    /// blitted into cells).
    pub buf: Buffer,
    pub info: TileInfo,
    /// The worker's measured simulated seconds per wall second (EMA).
    pub achieved: f32,
}

// ---- Wall state ----------------------------------------------------------

/// Cumulative results across every run the wall has played.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    pub wins: u64,
    /// Runs discarded because a worker caught a panic.
    pub failures: u64,
    /// Deaths by campaign level.
    pub deaths: [u64; campaign::LEVEL_COUNT],
    pub survival_sum: f64,
    pub survival_n: u64,
}

impl Tally {
    pub fn losses(&self) -> u64 {
        self.deaths.iter().sum()
    }

    pub fn runs(&self) -> u64 {
        self.wins + self.losses()
    }

    pub fn average_survival(&self) -> Option<f32> {
        (self.survival_n > 0).then(|| (self.survival_sum / self.survival_n as f64) as f32)
    }

    fn record(&mut self, info: &TileInfo) {
        let level = info.level.min(campaign::LEVEL_COUNT - 1);
        let secs = match &info.state {
            TileState::Running => return,
            TileState::Died { at, .. } => {
                self.deaths[level] += 1;
                *at
            }
            TileState::Won { total_time } => {
                self.wins += 1;
                *total_time
            }
            TileState::Failed { .. } => {
                self.failures += 1;
                return;
            }
        };
        self.survival_sum += secs.max(0.0) as f64;
        self.survival_n += 1;
    }
}

/// Everything the wall draws and aggregates.
pub struct WallState {
    pub games: usize,
    /// Latest frame per game (`None` until its worker publishes one).
    pub frames: Vec<Option<TileFrame>>,
    /// The game shown full-window, if any.
    pub zoom: Option<usize>,
    pub speed_idx: usize,
    pub paused: bool,
    pub tally: Tally,
    skill: WallSkill,
    by_skill: [Tally; SKILL_CYCLE.len()],
}

impl WallState {
    pub fn new(games: usize) -> Self {
        let games = games.clamp(1, MAX_GAMES);
        WallState {
            games,
            frames: vec![None; games],
            zoom: None,
            speed_idx: DEFAULT_SPEED_IDX,
            paused: false,
            tally: Tally::default(),
            skill: WallSkill::Fixed(Skill::Expert),
            by_skill: Default::default(),
        }
    }

    pub fn speed(&self) -> f32 {
        SPEEDS[self.speed_idx.min(SPEEDS.len() - 1)]
    }

    pub fn speed_up(&mut self) {
        self.speed_idx = (self.speed_idx + 1).min(SPEEDS.len() - 1);
    }

    pub fn speed_down(&mut self) {
        self.speed_idx = self.speed_idx.saturating_sub(1);
    }

    fn speed_caption(&self, achieved: f32) -> String {
        if self.paused {
            "PAUSED".to_string()
        } else if self.speed().is_infinite() {
            format!("MAX {achieved:.1} sim s/s")
        } else {
            format!("{:.0}x · {achieved:.1} sim s/s", self.speed())
        }
    }

    fn record(&mut self, info: &TileInfo) {
        self.tally.record(info);
        if self.skill == WallSkill::Mixed {
            self.by_skill[info.index % SKILL_CYCLE.len()].record(info);
        }
    }

    /// Mean simulated seconds per wall second over the games that are
    /// currently **running**: a game showing its plate or paused is not
    /// advancing, and counting it would understate the wall's speed.
    fn achieved(&self) -> f32 {
        let mut sum = 0.0;
        let mut n = 0usize;
        for f in self.frames.iter().flatten() {
            if matches!(f.info.state, TileState::Running) {
                sum += f.achieved;
                n += 1;
            }
        }
        if n > 0 {
            sum / n as f32
        } else {
            0.0
        }
    }

    /// Toggle zoom for game `i` (the caption key / a click on its tile).
    pub fn toggle_zoom(&mut self, i: usize) {
        if i >= self.games {
            return;
        }
        self.zoom = if self.zoom == Some(i) { None } else { Some(i) };
    }

    /// The game under a terminal cell, when the wall is showing the grid.
    pub fn game_at(&self, area: Rect, column: u16, row: u16) -> Option<usize> {
        if self.zoom.is_some() {
            return None;
        }
        let plan = plan_wall(self.games, area);
        (0..plan.grid.visible).find(|&i| {
            plan.tile_rect(i)
                .is_some_and(|r| rect_contains(r, column, row))
        })
    }
}

fn rect_contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x
        && x < r.x.saturating_add(r.width)
        && y >= r.y
        && y < r.y.saturating_add(r.height)
}

// ---- Drawing -------------------------------------------------------------

/// Draw the latest frames and tally into `buf`.
pub fn render_wall(buf: &mut Buffer, area: Rect, wall: &WallState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match wall.zoom {
        Some(i) => draw_zoom(buf, area, wall, i),
        None => {
            let plan = plan_wall(wall.games, area);
            for i in 0..wall.games.min(plan.grid.visible) {
                if let Some(tile) = plan.tile_rect(i) {
                    draw_tile(buf, tile, wall, i);
                }
            }
            if let Some(strip) = plan.summary {
                draw_summary(buf, strip, wall, plan.grid.hidden);
            }
        }
    }
}

/// Rows a tile gives up for its caption (zero on very short tiles).
fn caption_rows(tile: Rect) -> u16 {
    if tile.height >= 4 {
        1
    } else {
        0
    }
}

fn draw_tile(buf: &mut Buffer, tile: Rect, wall: &WallState, i: usize) {
    let cap_rows = caption_rows(tile);
    let world = Rect::new(
        tile.x,
        tile.y,
        tile.width,
        tile.height.saturating_sub(cap_rows),
    );
    let frame = wall.frames.get(i).and_then(|f| f.as_ref());
    match frame {
        Some(f) => {
            blit(buf, world, &f.buf);
            if f.info.state.is_finished() {
                dim_rect(buf, world);
                draw_plate(buf, world, &f.info);
            }
        }
        None => fill_rect(buf, world, palette::PLATE),
    }
    if cap_rows == 1 {
        let text = frame
            .map(|f| f.info.caption(tile.width as usize))
            .unwrap_or_default();
        let running = frame.is_some_and(|f| !f.info.state.is_finished());
        draw_caption_row(
            buf,
            Rect::new(tile.x, world.y + world.height, tile.width, 1),
            &text,
            wall.paused && running,
        );
    }
}

fn draw_zoom(buf: &mut Buffer, area: Rect, wall: &WallState, i: usize) {
    let cap_rows = if area.height >= 5 { 2 } else { 0 };
    let world = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(cap_rows),
    );
    let Some(frame) = wall.frames.get(i).and_then(|f| f.as_ref()) else {
        fill_rect(buf, world, palette::PLATE);
        draw_caption_row(
            buf,
            Rect::new(area.x, world.y + world.height, area.width, 1),
            &format!("game {} starting…", i + 1),
            false,
        );
        return;
    };
    blit(buf, world, &frame.buf);
    if frame.info.state.is_finished() {
        dim_rect(buf, world);
        draw_plate(buf, world, &frame.info);
    }
    if cap_rows == 2 {
        draw_caption_row(
            buf,
            Rect::new(area.x, world.y + world.height, area.width, 1),
            &frame.info.zoom_caption(area.width as usize),
            false,
        );
        let second = format!(
            "{} · Esc back · {}",
            wall.speed_caption(frame.achieved),
            frame.info.state_text(),
        );
        draw_caption_row(
            buf,
            Rect::new(area.x, world.y + world.height + 1, area.width, 1),
            &second.chars().take(area.width as usize).collect::<String>(),
            wall.paused,
        );
    }
}

/// The summary strip: speed, cumulative results and deaths by level.
fn summary_text(wall: &WallState, width: usize, hidden: usize, one_row: bool) -> String {
    let finished = wall
        .frames
        .iter()
        .flatten()
        .filter(|f| f.info.state.is_finished())
        .count();
    let running = wall.games - finished;
    let avg = wall
        .tally
        .average_survival()
        .map(clock)
        .unwrap_or_else(|| "--:--".to_string());
    let mut segs = vec![format!(
        "{} finished · {} wins · {} losses",
        wall.tally.runs(),
        wall.tally.wins,
        wall.tally.losses()
    )];
    segs.push(wall.speed_caption(wall.achieved()));
    if hidden > 0 {
        segs.push(format!("{hidden} hidden"));
    }
    if one_row {
        segs.push(result_detail(wall, width));
    }
    if wall.tally.failures > 0 {
        segs.push(format!("{} failed", wall.tally.failures));
    }
    segs.push(format!("AI {running}/{} running · avg {avg}", wall.games));
    segs.push("Space pause · +/- speed · 1-9 zoom · R restart · Q quit".to_string());

    let mut out = fit_segments(segs, width);
    if hidden > 0 && !out.contains("hidden") {
        // The full strip did not fit. Compact it so the hidden count survives
        // (this is the one thing a small terminal cannot infer otherwise).
        let visible = wall.games - hidden;
        let compact = format!("AI {visible}/{} · {hidden} hidden", wall.games);
        out = if compact.chars().count() <= width {
            compact
        } else {
            format!("{hidden} hidden").chars().take(width).collect()
        };
    }
    if out.is_empty() {
        out = wall.speed_caption(wall.achieved()).chars().take(width).collect();
    }
    out
}

fn result_detail(wall: &WallState, width: usize) -> String {
    let levels = wall.tally.deaths.iter().enumerate()
        .map(|(l, n)| format!("L{} {n}", l + 1))
        .collect::<Vec<_>>()
        .join(" ");
    let mut segs = vec![format!("Losses {levels}")];
    if wall.skill == WallSkill::Mixed {
        if width < 80 {
            segs[0] = levels;
            let skills = ["Exp", "Skl", "Cas"].into_iter()
                .zip(&wall.by_skill)
                .map(|(name, tally)| format!("{name} {}/{}", tally.wins, tally.losses()))
                .collect::<Vec<_>>()
                .join(" ");
            segs.push(format!("W/L {skills}"));
        } else {
            for (skill, tally) in SKILL_CYCLE.into_iter().zip(&wall.by_skill) {
                segs.push(format!("{} {}W/{}L", skill.name(), tally.wins, tally.losses()));
            }
        }
    }
    fit_segments(segs, width)
}

fn fit_segments(segs: Vec<String>, width: usize) -> String {
    let mut out = String::new();
    for seg in segs {
        let sep = if out.is_empty() { "" } else { " · " };
        if !seg.is_empty() && out.chars().count() + sep.chars().count() + seg.chars().count() <= width {
            out.push_str(sep);
            out.push_str(&seg);
        }
    }
    out
}

fn draw_summary(buf: &mut Buffer, strip: Rect, wall: &WallState, hidden: usize) {
    fill_rect(buf, strip, palette::PLATE);
    let width = strip.width as usize;
    let style = Style::default().fg(palette::MUTED).bg(palette::PLATE);
    buf.set_string(strip.x, strip.y, summary_text(wall, width, hidden, strip.height == 1), style);
    if strip.height == 2 {
        buf.set_string(strip.x, strip.y + 1, result_detail(wall, width), style);
    }
}

fn draw_caption_row(buf: &mut Buffer, row: Rect, text: &str, dimmed: bool) {
    fill_rect(buf, row, palette::PLATE);
    let fg = if dimmed { palette::EDGE } else { palette::COAT };
    buf.set_string(
        row.x,
        row.y,
        text,
        Style::default().fg(fg).bg(palette::PLATE),
    );
}

/// The outcome plate over the dimmed last frame: `DIED` + where/when/cause, or
/// `ESCAPED` + the run time.
fn draw_plate(buf: &mut Buffer, area: Rect, info: &TileInfo) {
    let (title, detail) = match &info.state {
        TileState::Died { at, cause } => (
            "DIED",
            format!("L{} {} to {}", info.level + 1, clock(*at), cause),
        ),
        TileState::Won { total_time } => ("ESCAPED", format!("in {}", clock(*total_time))),
        TileState::Failed { reason } => ("FAILED", format!("cast: {reason}")),
        TileState::Running => return,
    };
    let max_w = area.width as usize;
    if max_w == 0 || area.height == 0 {
        return;
    }
    let lines: [(String, Style); 2] = [
        (
            clip_center(title, max_w),
            Style::default()
                .fg(palette::TAIL)
                .bg(palette::PLATE)
                .add_modifier(Modifier::BOLD),
        ),
        (
            clip_center(&detail, max_w),
            Style::default().fg(palette::COAT).bg(palette::PLATE),
        ),
    ];
    // Vertically centred, but never above the tile.
    let block_h = 2.min(area.height);
    let top = area.y + (area.height - block_h) / 2;
    for (n, (text, style)) in lines.iter().take(block_h as usize).enumerate() {
        let y = top + n as u16;
        let x = area.x + (area.width.saturating_sub(text.chars().count() as u16)) / 2;
        // Blank the whole plate row first so the dimmed art does not bleed
        // through the letters.
        let band = Rect::new(area.x + (area.width / 8), y, area.width - area.width / 4, 1);
        fill_rect(buf, band, palette::PLATE);
        buf.set_string(x, y, text, *style);
    }
}

/// Truncate to `width` characters (the strings here are ASCII).
fn clip_center(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else if width <= 1 {
        text.chars().take(width).collect()
    } else {
        let mut s: String = text.chars().take(width - 1).collect();
        s.push('…');
        s
    }
}

fn fill_rect(buf: &mut Buffer, area: Rect, color: Color) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
                cell.set_fg(color);
                cell.set_bg(color);
            }
        }
    }
}

fn blit(buf: &mut Buffer, dst: Rect, src: &Buffer) {
    let w = dst.width.min(src.area.width);
    let h = dst.height.min(src.area.height);
    for y in 0..h {
        for x in 0..w {
            let from = src.cell((src.area.x + x, src.area.y + y)).cloned();
            if let (Some(from), Some(to)) = (from, buf.cell_mut((dst.x + x, dst.y + y))) {
                *to = from;
            }
        }
    }
}

fn dim_rect(buf: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(dim_color(cell.fg));
                cell.set_bg(dim_color(cell.bg));
            }
        }
    }
}

fn dim_color(color: Color) -> Color {
    const DIM: f32 = 0.32;
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * DIM) as u8,
            (g as f32 * DIM) as u8,
            (b as f32 * DIM) as u8,
        ),
        other => other,
    }
}


// ---- Live wall: options, workers and the spectate loop -------------------

/// The play field the wall aims its tiles at, in cells: the normal game's
/// `160 x 40`-cell viewport (`160 x 80` half-block pixels).
pub const REFERENCE_VIEW_W: u16 = 160;

/// Hidden games still simulate but render nothing (a zero-sized tile), so a
/// small terminal spends its drawing budget only on the tiles it shows.
const HIDDEN_W: u16 = 0;
const HIDDEN_H: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallSkill {
    Fixed(Skill),
    Mixed,
}

/// `ratrun --ai ...` options. Parsed in `main.rs` (see `parse_spectate_args`).
#[derive(Clone, Debug, PartialEq)]
pub struct AiOpts {
    /// How many games to show, 1..=`MAX_GAMES`.
    pub games: usize,
    /// `--seed S`: reproducible wall. Tile `i` derives its seeds from `S` and
    /// `i`, so the same wall replays.
    pub seed: Option<u64>,
    /// `--skill expert|skilled|casual|mixed` (default expert).
    pub skill: WallSkill,
    /// Initial rung of the speed ladder.
    pub speed_idx: usize,
    /// `--level 1..4`: every tile plays that one level alone.
    pub level: Option<usize>,
    /// `--once`: keep a finished run on its plate instead of restarting it.
    pub once: bool,
}

impl Default for AiOpts {
    fn default() -> Self {
        AiOpts {
            games: DEFAULT_GAMES,
            seed: None,
            skill: WallSkill::Fixed(Skill::Expert),
            speed_idx: DEFAULT_SPEED_IDX,
            level: None,
            once: false,
        }
    }
}

impl AiOpts {
    /// Resolve the `--skill` string. `None` on an unknown name.
    pub fn parse_skill(name: &str) -> Option<WallSkill> {
        if name.eq_ignore_ascii_case("mixed") {
            Some(WallSkill::Mixed)
        } else {
            Skill::parse(name).map(WallSkill::Fixed)
        }
    }

    pub fn parse_speed(name: &str) -> Option<usize> {
        if name.eq_ignore_ascii_case("max") {
            return Some(SPEEDS.len() - 1);
        }
        let speed: u32 = name.parse().ok()?;
        SPEEDS.iter().position(|&rung| rung == speed as f32)
    }

    fn auto_opts(&self, index: usize) -> AutoOpts {
        let skill = match self.skill {
            WallSkill::Fixed(skill) => skill,
            WallSkill::Mixed => SKILL_CYCLE[index % SKILL_CYCLE.len()],
        };
        match self.level {
            Some(level) => AutoOpts::practice(level.clamp(1, campaign::LEVEL_COUNT) - 1, skill),
            None => AutoOpts::campaign(0, skill),
        }
    }
}

/// What the wall needs to know about a worker, shared by atomics (no locks on
/// the hot path; the main loop never waits for a worker).
struct WorkerShared {
    stop: AtomicBool,
    paused: AtomicBool,
    /// Simulation seconds per wall second, as `f32` bits.
    speed: AtomicU32,
    /// Tile world size in cells, `(w << 16) | h`.
    size: AtomicU32,
    restart_gen: AtomicU32,
    /// The new per-tile seed base when `restart_gen` changes.
    restart_seed: AtomicU64,
}

impl WorkerShared {
    fn new(speed: f32, sw: u16, sh: u16) -> Arc<WorkerShared> {
        Arc::new(WorkerShared {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            speed: AtomicU32::new(speed.to_bits()),
            size: AtomicU32::new(((sw as u32) << 16) | sh as u32),
            restart_gen: AtomicU32::new(0),
            restart_seed: AtomicU64::new(0),
        })
    }

    fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }

    fn size(&self) -> (u16, u16) {
        let packed = self.size.load(Ordering::Relaxed);
        ((packed >> 16) as u16, packed as u16)
    }

    fn request_size(&self, w: u16, h: u16) {
        let packed = ((w as u32) << 16) | h as u32;
        if self.size.load(Ordering::Relaxed) != packed {
            self.size.store(packed, Ordering::Relaxed);
        }
    }

    fn request_speed(&self, speed: f32) {
        let bits = speed.to_bits();
        if self.speed.load(Ordering::Relaxed) != bits {
            self.speed.store(bits, Ordering::Relaxed);
        }
    }
}

struct Worker {
    shared: Arc<WorkerShared>,
    frames: Receiver<TileFrame>,
    handle: Option<JoinHandle<()>>,
}

/// The wall's live half: the drawn [`WallState`] plus the workers feeding it.
pub struct LiveWall {
    pub state: WallState,
    workers: Vec<Worker>,
    outcomes: Receiver<TileInfo>,
    base_seed: u64,
    generation: u64,
}

impl LiveWall {
    /// Start `games` workers, one `AutoRun` each.
    pub fn spawn(games: usize, opts: &AiOpts, base_seed: u64) -> LiveWall {
        Self::spawn_with_tuning(games, opts, base_seed, crate::tuning::RunTuning::default())
    }

    fn spawn_with_tuning(games: usize, opts: &AiOpts, base_seed: u64, tuning: crate::tuning::RunTuning) -> LiveWall {
        let games = games.clamp(1, MAX_GAMES);
        let once = opts.once;
        let mut workers = Vec::with_capacity(games);
        let (outcome_tx, outcomes) = mpsc::channel();
        let speed_idx = opts.speed_idx.min(SPEEDS.len() - 1);
        for i in 0..games {
            let shared = WorkerShared::new(SPEEDS[speed_idx], HIDDEN_W, HIDDEN_H);
            let (tx, rx) = mpsc::sync_channel(1);
            let outcome_tx = outcome_tx.clone();
            let auto_opts = opts.auto_opts(i);
            let base = tile_base(base_seed, i);
            let worker = {
                let shared = Arc::clone(&shared);
                let tuning = tuning.clone();
                std::thread::Builder::new()
                    .name(format!("ratrun-ai-{i}"))
                    .spawn(move || worker_loop(i, base, auto_opts, once, shared, tx, outcome_tx, tuning))
            };
            match worker {
                Ok(handle) => workers.push(Worker {
                    shared,
                    frames: rx,
                    handle: Some(handle),
                }),
                // A thread we cannot start is simply a game that never shows;
                // keep the wall consistent rather than aborting the run.
                Err(_) => break,
            }
        }
        let shown = workers.len().max(1);
        let mut state = WallState::new(shown);
        state.speed_idx = speed_idx;
        state.skill = opts.skill;
        LiveWall {
            state,
            workers,
            outcomes,
            base_seed,
            generation: 0,
        }
    }

    /// Pull each worker's newest frame, then tell it the size/speed/pause it
    /// should use for the next frame. Runs no simulation on this thread and
    /// never blocks on a slow game.
    pub fn pump(&mut self, area: Rect) {
        let plan = plan_wall(self.state.games, area);
        let zoom = self.state.zoom;
        let speed = self.state.speed();
        let paused = self.state.paused;
        for i in 0..self.workers.len() {
            let worker = &mut self.workers[i];
            let mut latest = None;
            while let Ok(frame) = worker.frames.try_recv() {
                latest = Some(frame);
            }
            if let Some(frame) = latest {
                self.state.frames[i] = Some(frame);
            }
            let (w, h) = match zoom {
                Some(z) if z == i => (
                    area.width,
                    area.height
                        .saturating_sub(if area.height >= 5 { 2 } else { 0 }),
                ),
                Some(_) => (HIDDEN_W, HIDDEN_H),
                None => match plan.tile_rect(i) {
                    Some(r) => (r.width, r.height.saturating_sub(caption_rows(r))),
                    None => (HIDDEN_W, HIDDEN_H),
                },
            };
            worker.shared.request_size(w, h);
            worker.shared.request_speed(speed);
            worker
                .shared
                .paused
                .store(paused, Ordering::Relaxed);
        }
        for info in self.outcomes.try_iter() {
            self.state.record(&info);
        }
    }

    /// `R`: restart every game with a fresh seed generation.
    pub fn restart_all(&mut self) {
        self.generation += 1;
        let base = mix64(self.base_seed ^ self.generation.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        for i in 0..self.workers.len() {
            let worker = &mut self.workers[i];
            let seed = tile_base(base, i);
            worker.shared.restart_seed.store(seed, Ordering::Relaxed);
            worker
                .shared
                .restart_gen
                .store(self.generation as u32, Ordering::Relaxed);
        }
        self.state
            .frames
            .iter_mut()
            .for_each(|frame| *frame = None);
    }

    pub fn shutdown(&mut self) {
        for worker in &self.workers {
            worker.shared.stop.store(true, Ordering::Relaxed);
        }
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for LiveWall {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One game's worker: step the run at the requested speed, render it into a
/// tile-sized buffer and publish the newest frame. Frames are sent with
/// `try_send` on a one-slot channel: if the main loop has not drawn the
/// previous one yet this frame is dropped, never queued, and the game keeps
/// playing regardless.
///
/// A panic inside the simulation, the bot or level generation is contained
/// here: the tile becomes a `FAILED` plate and the game retries with a new seed
/// after [`RESTART_DELAY_SECS`], so one broken run can never take the wall (or
/// the terminal) down.
fn worker_loop(
    index: usize,
    base: u64,
    auto_opts: AutoOpts,
    once: bool,
    shared: Arc<WorkerShared>,
    tx: SyncSender<TileFrame>,
    outcome_tx: mpsc::Sender<TileInfo>,
    tuning: crate::tuning::RunTuning,
) {
    let mut run_index = 0u64;
    let mut seed = tile_seed(base, run_index);
    let mut run: Option<AutoRun> = None;
    let mut failure: Option<(String, Instant)> = None;
    let mut pixels = PixelBuffer::new(0, 0);
    let mut seen_generation = 0u32;
    let mut acc = 0.0f64;
    let mut achieved = 0.0f32;
    let mut finished_at: Option<Instant> = None;
    let mut reported = false;
    let mut last = Instant::now();
    let mut last_max_frame = None;
    let mut max_ticks_since_frame = 0;

    loop {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        if shared.restart_gen.load(Ordering::Relaxed) != seen_generation {
            seen_generation = shared.restart_gen.load(Ordering::Relaxed);
            run_index = 0;
            seed = tile_seed(shared.restart_seed.load(Ordering::Relaxed), run_index);
            run = None;
            failure = None;
            acc = 0.0;
            finished_at = None;
            reported = false;
        }

        // Build the run (the first thing that can fail: level generation).
        if run.is_none() && failure.is_none() {
            match contained(|| AutoRun::with_tuning(seed, auto_opts, tuning.clone())) {
                Ok(active) => {
                    run = Some(active);
                    acc = 0.0;
                    finished_at = None;
                    last = Instant::now();
                }
                Err(reason) => failure = Some((reason, Instant::now())),
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.1);
        last = now;
        let paused = shared.paused.load(Ordering::Relaxed);
        let speed = shared.speed();
        let uncapped = speed.is_infinite();
        let mut ticks = 0u32;

        if uncapped {
            // Finite rungs can leave a backlog. A later return to them
            // should start from the current clock.
            acc = 0.0;
        } else {
            last_max_frame = None;
            max_ticks_since_frame = 0;
        }
        if let Some(active) = run.as_mut() {
            if !paused && finished_at.is_none() {
                // Keep a bounded backlog: a game that fell behind catches up
                // at most half a second of simulation, so no worker ever runs
                // away.
                let batch = contained(|| {
                    if uncapped {
                        // Check controls and the frame deadline after each
                        // tick. There is no per-frame simulation budget.
                        active.step(SIM_DT);
                        return 1;
                    }
                    acc = (acc + dt * speed as f64).min(speed as f64 * 0.5);
                    let max_ticks = ((speed as f64 * 0.05 * 60.0).ceil() as u32).clamp(1, 512);
                    let mut n = 0u32;
                    while acc >= SIM_DT as f64 && n < max_ticks {
                        active.step(SIM_DT);
                        acc -= SIM_DT as f64;
                        n += 1;
                        if active.finished() {
                            break;
                        }
                    }
                    n
                });
                match batch {
                    Ok(n) => ticks = n,
                    Err(reason) => {
                        failure = Some((reason, Instant::now()));
                        run = None;
                    }
                }
            }
        }
        if !paused && !uncapped {
            achieved = smooth_rate(achieved, ticks, dt);
        }

        if let Some(active) = run.as_ref() {
            if active.finished() {
                let plate = *finished_at.get_or_insert_with(Instant::now);
                // A paused wall holds its plates: Space is how a result is
                // read, so it must not be swapped for a fresh game meanwhile.
                if !once && !paused && should_restart(plate.elapsed()) {
                    run_index += 1;
                    seed = tile_seed(base, run_index);
                    run = None;
                    acc = 0.0;
                    finished_at = None;
                    reported = false;
                    // Rebuild at the top of the next iteration instead of
                    // publishing one blank "0 HP" frame between the plate and
                    // the new run.
                    continue;
                }
            }
        }
        if let Some((_, at)) = &failure {
            if !once && !paused && should_restart(at.elapsed()) {
                run_index += 1;
                seed = tile_seed(base, run_index);
                failure = None;
                reported = false;
                continue;
            }
        }

        let running_at_max = uncapped && !paused && finished_at.is_none() && failure.is_none();
        if uncapped {
            max_ticks_since_frame += ticks;
            let frame_at = Instant::now();
            let first_outcome = !reported && (finished_at.is_some() || failure.is_some());
            if !first_outcome && !max_frame_due(last_max_frame, frame_at) {
                if !running_at_max {
                    std::thread::sleep(Duration::from_micros(16_667));
                }
                continue;
            }
            // Use the real wall time, including the previous frame's draw.
            achieved = if running_at_max {
                last_max_frame.map_or(0.0, |previous| {
                    smooth_rate(achieved, max_ticks_since_frame, (frame_at - previous).as_secs_f64())
                })
            } else {
                0.0
            };
            last_max_frame = Some(frame_at);
            max_ticks_since_frame = 0;
        }

        // Render at an integer multiple of the tile, so every tile shows about
        // the normal game's field of view (see `render_scale`), then
        // downsample to the tile's cells. A zero-sized tile (hidden game) is
        // not drawn at all.
        let (cw, ch) = shared.size();
        let mut draw_failed: Option<String> = None;
        let (mut buf, mut info) = match run.as_mut() {
            Some(active) => {
                let scale = render_scale(cw);
                let pw = cw as usize * scale as usize;
                let ph = ch as usize * 2 * scale as usize;
                pixels.resize(pw, ph);
                let mut buf = Buffer::empty(Rect::new(0, 0, cw, ch));
                let drawn = contained(|| {
                    if pw > 0 && ph > 0 {
                        let world = active.world_mut();
                        world.lighting = crate::render::lighting::LightingMode::Compact;
                        world.smoke_enabled = false;
                        world.draw(&mut pixels);
                        downsample_into(&pixels, scale, buf.area, &mut buf);
                    }
                });
                if let Err(reason) = drawn {
                    draw_failed = Some(reason);
                }
                (buf, tile_info(index, active))
            }
            None => {
                let mut info = blank_info(index);
                info.state = match &failure {
                    Some((reason, _)) => TileState::Failed {
                        reason: reason.clone(),
                    },
                    None => TileState::Running,
                };
                (Buffer::empty(Rect::new(0, 0, cw, ch)), info)
            }
        };
        if let Some(reason) = draw_failed {
            failure = Some((reason.clone(), Instant::now()));
            run = None;
            buf = Buffer::empty(Rect::new(0, 0, cw, ch));
            info = blank_info(index);
            info.state = TileState::Failed { reason };
        }
        if !reported && info.state.is_finished() {
            let _ = outcome_tx.send(info.clone());
            reported = true;
        }
        let _ = tx.try_send(TileFrame {
            buf,
            info,
            achieved,
        });

        if running_at_max {
            continue;
        }
        // ~60 FPS with the render included; never sleep when behind.
        let spent = now.elapsed();
        let target = Duration::from_micros(16_667);
        if spent < target {
            std::thread::sleep(target - spent);
        }
    }
}

fn max_frame_due(last_frame: Option<Instant>, now: Instant) -> bool {
    last_frame.is_none_or(|at| now - at >= MAX_FRAME_INTERVAL)
}

fn smooth_rate(previous: f32, ticks: u32, wall_seconds: f64) -> f32 {
    let instant = if wall_seconds > 0.0 {
        ticks as f32 * SIM_DT / wall_seconds as f32
    } else {
        0.0
    };
    if previous == 0.0 {
        instant
    } else {
        previous * 0.9 + instant * 0.1
    }
}

/// Run `f`, turning a panic into an error instead of unwinding out of a worker
/// thread. The panic hook in `main.rs` stays silent for `ratrun-ai-*` threads,
/// so a contained panic cannot corrupt the alternate screen.
fn contained<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(_) => Err(take_recorded_panic().unwrap_or_else(|| "worker panicked".to_string())),
    }
}

thread_local! {
    /// The message of the panic `contained` is about to catch. `main.rs`'s
    /// panic hook records it on the panicking `ratrun-ai-*` thread (it must not
    /// print: the wall owns the alternate screen). A thread-local keeps
    /// concurrent workers from overwriting each other.
    static RECORDED_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Called from `main.rs`'s panic hook for a wall worker's panic.
pub(crate) fn record_worker_panic(message: Option<&str>) {
    let text = message.unwrap_or("worker panicked");
    let first = text.lines().next().unwrap_or("worker panicked").trim();
    let clipped: String = first.chars().take(80).collect();
    RECORDED_PANIC.with(|slot| *slot.borrow_mut() = Some(clipped));
}

fn take_recorded_panic() -> Option<String> {
    RECORDED_PANIC.with(|slot| slot.borrow_mut().take())
}

fn blank_info(index: usize) -> TileInfo {
    TileInfo {
        index,
        level: 0,
        theme: 'S',
        elapsed: 0.0,
        hp: 0.0,
        max_hp: 1.0,
        player_level: 1,
        kills: 0,
        score: 0,
        boss: None,
        state: TileState::Running,
    }
}

/// The integer render scale for a tile: the multiple of its cell size that is
/// closest to the reference play field. A 20-cell tile renders at 160 px wide
/// (scale 8); a 240-cell zoom renders at scale 1.
pub fn render_scale(cell_w: u16) -> u16 {
    if cell_w == 0 {
        return 1;
    }
    let scale = (REFERENCE_VIEW_W as f32 / cell_w as f32).round() as u16;
    scale.clamp(1, 32)
}

/// Does a finished run's outcome plate expire (so the wall restarts it)?
pub fn should_restart(plate_age: Duration) -> bool {
    plate_age >= Duration::from_secs_f32(RESTART_DELAY_SECS)
}

fn tile_info(index: usize, run: &AutoRun) -> TileInfo {
    let w = run.world();
    let level = w.level_index.min(campaign::LEVEL_COUNT - 1);
    let status = run.status();
    let (hp, max_hp) = (w.player.hp.max(0.0), w.player.max_hp());
    let boss = match status {
        AutoStatus::Playing { boss, .. } => boss,
        _ => w
            .boss_hp()
            .map(|(hp, max)| ((hp / max.max(1.0)).clamp(0.0, 1.0), w.boss_phase().unwrap_or(1))),
    };
    let state = match status {
        AutoStatus::Playing { .. } => TileState::Running,
        AutoStatus::Died { at, cause, .. } => TileState::Died {
            at,
            cause: cause.name().to_string(),
        },
        AutoStatus::Won { total_time, .. } => TileState::Won { total_time },
    };
    TileInfo {
        index,
        level,
        theme: w.level.theme.name().chars().next().unwrap_or('?'),
        elapsed: w.elapsed,
        hp,
        max_hp,
        player_level: w.player.level,
        kills: w.total_kills(),
        score: w.total_score(),
        boss,
        state,
    }
}

/// Downsample an integer-scaled world render into tile cells: each cell takes
/// the average of its `scale x scale` top-pixel block and its bottom block,
/// then draws with `▀` like the normal game's blit.
pub fn downsample_into(pb: &PixelBuffer, scale: u16, area: Rect, buf: &mut Buffer) {
    let scale = scale.max(1) as usize;
    if area.width == 0 || area.height == 0 || pb.w == 0 || pb.h == 0 {
        return;
    }
    for cy in 0..area.height {
        for cx in 0..area.width {
            let (top, bot) = block_avg(pb, cx as usize * scale, cy as usize * 2 * scale, scale);
            if let Some(cell) = buf.cell_mut((area.x + cx, area.y + cy)) {
                cell.set_symbol("▀");
                cell.set_fg(Color::Rgb(top.0, top.1, top.2));
                cell.set_bg(Color::Rgb(bot.0, bot.1, bot.2));
            }
        }
    }
}

/// The average colour of a `scale x scale` block, and of the block one row of
/// pixels lower (the cell's bottom half). Out-of-range pixels are skipped.
fn block_avg(pb: &PixelBuffer, x0: usize, y0: usize, scale: usize) -> (palette::Rgb, palette::Rgb) {
    let mut top = (0u32, 0u32, 0u32, 0u32);
    let mut bot = (0u32, 0u32, 0u32, 0u32);
    for y in 0..scale {
        for x in 0..scale {
            let xx = x0 + x;
            if xx >= pb.w {
                continue;
            }
            let ty = y0 + y;
            let by = y0 + scale + y;
            if ty < pb.h {
                let (r, g, b) = pb.pixel_at(xx, ty);
                top = (top.0 + r as u32, top.1 + g as u32, top.2 + b as u32, top.3 + 1);
            }
            if by < pb.h {
                let (r, g, b) = pb.pixel_at(xx, by);
                bot = (bot.0 + r as u32, bot.1 + g as u32, bot.2 + b as u32, bot.3 + 1);
            }
        }
    }
    let avg = |c: (u32, u32, u32, u32)| -> palette::Rgb {
        let n = c.3.max(1);
        ((c.0 / n) as u8, (c.1 / n) as u8, (c.2 / n) as u8)
    };
    (avg(top), avg(bot))
}

/// A per-tile seed base: derived from the wall seed and the tile index.
pub fn tile_base(base: u64, index: usize) -> u64 {
    mix64(base ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// The `run_index`-th run of one tile.
pub fn tile_seed(base: u64, run_index: u64) -> u64 {
    mix64(base ^ run_index.wrapping_mul(0xD1B5_4A32_D192_ED03))
}

fn mix64(mut h: u64) -> u64 {
    h = h.wrapping_add(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

fn entropy_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5EED_5EED);
    mix64(nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Apply one frame of wall keys/gestures. Returns `true` when the user asked
/// to quit (`Q`, or Ctrl-C via the input thread's drain).
///
/// `+`/`-` walk the speed ladder, Space pauses every game, `1`..`9` zoom a
/// game (`Esc` or the same key returns), a click on a tile zooms that game and
/// `R` restarts the whole wall (handled by the caller, which owns the seeds).
pub fn spectate_keys(state: &mut WallState, input: &InputState, area: Rect) -> bool {
    let quit = input.pressed_char('q');
    if input.just_pressed(KeyCode::Esc) {
        state.zoom = None;
    }
    for (i, key) in ['1', '2', '3', '4', '5', '6', '7', '8', '9'].iter().enumerate() {
        if !input.pressed_char(*key) {
            continue;
        }
        // Through `toggle_zoom`, which refuses a game the wall does not have:
        // `9` on a four-game wall used to zoom game 8, hiding every tile
        // behind a view of nothing.
        state.toggle_zoom(i);
    }
    if input.pressed_char('+') || input.pressed_char('=') {
        state.speed_up();
    }
    if input.pressed_char('-') || input.pressed_char('_') {
        state.speed_down();
    }
    if input.just_pressed(KeyCode::Char(' ')) {
        state.paused = !state.paused;
    }
    for ev in &input.mouse_events {
        if ev.kind == MouseEventKind::Down(crossterm::event::MouseButton::Left) {
            if let Some(i) = state.game_at(area, ev.column, ev.row) {
                state.toggle_zoom(i);
            }
        }
    }
    quit
}

/// `ratrun --ai`: run the wall until the user quits.
pub fn run(
    terminal: &mut Terminal<CrosstermBackend<crate::present::FrameOut>>,
    kitty: bool,
    opts: AiOpts,
    tuning: crate::tuning::RunTuning,
) -> Result<()> {
    let games = opts.games.clamp(1, MAX_GAMES);
    let base_seed = opts.seed.unwrap_or_else(entropy_seed);
    let mut wall = LiveWall::spawn_with_tuning(games, &opts, base_seed, tuning);
    // Same reader thread as normal play: dropping the guard stops and joins it
    // before the terminal is restored.
    let (_input_thread, rx) = crate::InputThread::spawn();
    let mut input = InputState::new(kitty);
    let mut last = Instant::now();
    loop {
        let frame_start = Instant::now();
        input.begin_frame();
        if crate::drain_input(&rx, &mut input)? {
            break;
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.1);
        last = now;
        input.tick(dt);
        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        if spectate_keys(&mut wall.state, &input, area) {
            break;
        }
        if input.pressed_char('r') {
            wall.restart_all();
        }

        wall.pump(area);
        let wall_state = &wall.state;
        crate::present::synchronized_frame(terminal, |f| {
            let area = f.area();
            render_wall(f.buffer_mut(), area, wall_state)
        })?;

        // Up to 60 FPS; drop to 30 when compositing cannot keep up.
        let spent = frame_start.elapsed();
        let slow = spent > Duration::from_millis(20);
        let target = if slow {
            Duration::from_millis(33)
        } else {
            Duration::from_micros(16_667)
        };
        if spent < target {
            std::thread::sleep(target - spent);
        }
    }
    wall.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(index: usize, level: usize, elapsed: f32) -> TileInfo {
        TileInfo {
            index,
            level,
            theme: campaign::level_theme(level).name().chars().next().unwrap(),
            elapsed,
            hp: 60.0,
            max_hp: 100.0,
            player_level: 3,
            kills: 12,
            score: 900,
            boss: None,
            state: TileState::Running,
        }
    }

    fn frame(index: usize, level: usize) -> TileFrame {
        TileFrame {
            buf: Buffer::empty(Rect::new(0, 0, 8, 4)),
            info: info(index, level, 42.0),
            achieved: 8.0,
        }
    }

    fn wall(games: usize) -> WallState {
        let mut w = WallState::new(games);
        for (i, slot) in w.frames.iter_mut().enumerate() {
            *slot = Some(frame(i, i % campaign::LEVEL_COUNT));
        }
        w
    }

    fn text(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn grid_shapes_for_each_wall_size() {
        // Every requested N is fully visible at the reference sizes, and 16 is
        // the canonical 4 x 4.
        for (w, h) in [(80u16, 24u16), (160, 48), (240, 70)] {
            let plan = plan_wall(16, Rect::new(0, 0, w, h));
            assert_eq!(
                (plan.grid.cols, plan.grid.rows),
                (4, 4),
                "16 games at {w}x{h} should be 4x4, got {:?}",
                plan.grid
            );
            assert_eq!(plan.grid.visible, 16);
            assert_eq!(plan.grid.hidden, 0);

            for n in [1usize, 4, 20] {
                let plan = plan_wall(n, Rect::new(0, 0, w, h));
                assert_eq!(plan.grid.visible, n, "{n} games must all show at {w}x{h}");
                assert_eq!(plan.grid.hidden, 0);
                assert!(
                    plan.grid.tile_w >= MIN_TILE_W && plan.grid.tile_h >= MIN_TILE_H,
                    "{n} games at {w}x{h}: tile {:?}",
                    plan.grid
                );
            }
            let one = plan_wall(1, Rect::new(0, 0, w, h)).grid;
            assert_eq!((one.cols, one.rows), (1, 1));
            let four = plan_wall(4, Rect::new(0, 0, w, h)).grid;
            assert_eq!((four.cols, four.rows), (2, 2));
            let twenty = plan_wall(20, Rect::new(0, 0, w, h)).grid;
            assert_eq!((twenty.cols, twenty.rows), (5, 4));
        }
    }

    #[test]
    fn tiny_terminals_hide_games_instead_of_panicking() {
        for (w, h) in [(0u16, 0u16), (5, 3), (20, 8)] {
            let area = Rect::new(0, 0, w, h);
            let plan = plan_wall(16, area);
            assert!(plan.grid.visible < 16, "a tiny terminal cannot show 16 tiles");
            assert_eq!(plan.grid.hidden, 16 - plan.grid.visible);
            // Rendering must not panic at any size, including zero.
            let mut buf = Buffer::empty(area);
            let state = wall(16);
            render_wall(&mut buf, area, &state);
        }
        assert_eq!(plan_wall(16, Rect::new(0, 0, 0, 0)).grid.visible, 0);
        assert_eq!(
            plan_wall(16, Rect::new(0, 0, 0, 0)).grid.hidden,
            16
        );
        let tiny = plan_wall(16, Rect::new(0, 0, 20, 8));
        assert_eq!(tiny.grid.visible, 1, "20x8 fits exactly one tile: {tiny:?}");
        assert_eq!(tiny.grid.hidden, 15);
        assert!(summary_text(&wall(16), 200, tiny.grid.hidden, true).contains("15 hidden"));
        // Even a 20-column strip has to report the hidden games.
        let narrow = summary_text(&wall(16), 20, 15, true);
        assert!(narrow.contains("15 hidden"), "{narrow:?}");
        assert!(narrow.chars().count() <= 20, "{narrow:?}");
    }

    #[test]
    fn caption_carries_every_field_at_a_twenty_cell_tile() {
        let i = info(0, 0, 42.0);
        let caption = i.caption(20);
        assert!(caption.chars().count() <= 20);
        for field in ["#1", "1S", "0:42", "#-", "L3", "12", ">"] {
            assert!(
                caption.contains(field),
                "caption {caption:?} is missing {field:?}"
            );
        }
    }

    #[test]
    fn captions_are_present_for_every_tile() {
        for (w, h, n) in [(80u16, 24u16, 16usize), (160, 48, 16), (240, 70, 20)] {
            let area = Rect::new(0, 0, w, h);
            let state = wall(n);
            let mut buf = Buffer::empty(area);
            render_wall(&mut buf, area, &state);
            let plan = plan_wall(n, area);
            for i in 0..plan.grid.visible {
                let rect = plan.tile_rect(i).unwrap();
                let row = rect.y + rect.height - 1;
                let mut line = String::new();
                for x in rect.x..rect.x + rect.width {
                    line.push_str(buf.cell((x, row)).map(|c| c.symbol()).unwrap_or(" "));
                }
                assert!(
                    line.contains(&format!("#{}", i + 1)),
                    "tile {i} at {w}x{h} has no caption: {line:?}"
                );
                assert!(line.contains("L3"), "tile {i} caption: {line:?}");
            }
        }
    }

    #[test]
    fn ended_games_show_their_plate() {
        let mut state = wall(4);
        state.frames[1] = Some(TileFrame {
            info: TileInfo {
                state: TileState::Died {
                    at: 192.0,
                    cause: "a swarm of cockroaches".into(),
                },
                ..info(1, 1, 192.0)
            },
            ..frame(1, 1)
        });
        state.frames[3] = Some(TileFrame {
            info: TileInfo {
                state: TileState::Won { total_time: 764.0 },
                level: 2,
                ..info(3, 2, 700.0)
            },
            ..frame(3, 2)
        });
        let area = Rect::new(0, 0, 160, 48);
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        let screen = text(&buf);
        assert!(screen.contains("DIED"), "missing DIED plate:\n{screen}");
        assert!(screen.contains("ESCAPED"), "missing ESCAPED plate");
        assert!(screen.contains("3:12"), "death clock missing");
        assert!(screen.contains("cockroaches"), "death cause missing");
        assert!(screen.contains("12:44"), "win clock missing");
    }

    #[test]
    fn the_wall_renders_through_a_test_backend_terminal() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut state = wall(16);
        state.frames[0] = Some(TileFrame {
            info: TileInfo {
                state: TileState::Died {
                    at: 93.0,
                    cause: "bat contact".into(),
                },
                ..info(0, 0, 93.0)
            },
            ..frame(0, 0)
        });
        let mut terminal = Terminal::new(TestBackend::new(240, 70)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_wall(frame.buffer_mut(), area, &state);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut screen = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                screen.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
            screen.push('\n');
        }
        for i in 1..=16 {
            assert!(screen.contains(&format!("#{i} ")), "missing tile {i} caption");
        }
        assert!(screen.contains("DIED"));
        assert!(screen.contains("bat contact"));
        assert!(screen.contains("sim s/s"));
    }

    #[test]
    fn a_contained_panic_becomes_a_failed_plate_not_a_crash() {
        // The main.rs panic hook records the message on the panicking thread;
        // tests install no hook, so record it the way the hook would.
        record_worker_panic(Some("level generation failed validation"));
        let reason = contained(|| -> u32 {
            panic!("level generation failed validation")
        })
        .unwrap_err();
        assert!(reason.contains("level generation failed"), "{reason:?}");

        let mut state = wall(2);
        state.frames[0] = Some(TileFrame {
            buf: Buffer::empty(Rect::new(0, 0, 20, 4)),
            info: TileInfo {
                state: TileState::Failed {
                    reason: reason.clone(),
                },
                ..info(0, 0, 0.0)
            },
            achieved: 0.0,
        });
        let area = Rect::new(0, 0, 160, 48);
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        let screen = text(&buf);
        assert!(screen.contains("FAILED"), "failed plate missing:\n{screen}");
        state.record(&state.frames[0].as_ref().unwrap().info.clone());
        assert_eq!(state.tally.failures, 1);
        assert_eq!(state.tally.runs(), 0, "a crash is not a death or a win");
        assert!(summary_text(&state, 240, 0, true).contains("1 failed"));
    }

    #[test]
    fn zoom_shows_one_game_full_window_and_esc_returns() {
        let mut state = wall(16);
        let area = Rect::new(0, 0, 240, 70);
        state.zoom = Some(5);
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        let screen = text(&buf);
        assert!(screen.contains("GAME 6"), "zoom caption missing:\n{screen}");
        assert!(!screen.contains("#1 "), "grid captions leaked into zoom");

        state.zoom = None;
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        assert!(text(&buf).contains("#6"));
    }

    #[test]
    fn speed_ladder_goes_up_and_down_and_clamps() {
        let mut state = WallState::new(4);
        assert_eq!(state.speed(), 8.0);
        state.speed_down();
        assert_eq!(state.speed(), 4.0);
        state.speed_down();
        state.speed_down();
        state.speed_down();
        assert_eq!(state.speed(), 1.0, "bottom of the ladder");
        state.speed_down();
        assert_eq!(state.speed(), 1.0, "cannot go below 1x");
        for speed in [2.0, 4.0, 8.0, 16.0, 32.0] {
            state.speed_up();
            assert_eq!(state.speed(), speed);
        }
        state.speed_up();
        assert_eq!(state.speed(), f32::INFINITY);
        state.speed_up();
        assert_eq!(state.speed(), f32::INFINITY, "cannot go above MAX");
        state.speed_down();
        assert_eq!(state.speed(), 32.0);
    }

    #[test]
    fn uncapped_frames_are_due_independently_of_tick_count() {
        let first = Instant::now();
        assert!(max_frame_due(None, first));
        assert!(!max_frame_due(Some(first), first));
        assert!(!max_frame_due(Some(first), first + MAX_FRAME_INTERVAL - Duration::from_micros(1)));
        assert!(max_frame_due(Some(first), first + MAX_FRAME_INTERVAL));
        assert!(max_frame_due(Some(first), first + MAX_FRAME_INTERVAL * 10));
    }

    #[test]
    fn tally_records_outcomes_by_level_and_skill() {
        let mut state = WallState::new(4);
        state.skill = WallSkill::Mixed;
        let died = TileInfo {
            state: TileState::Died {
                at: 100.0,
                cause: "acid".into(),
            },
            ..info(0, 0, 100.0)
        };
        state.record(&died);
        assert_eq!(state.tally.deaths[0], 1);
        assert_eq!(state.tally.runs(), 1);
        assert_eq!(state.tally.average_survival(), Some(100.0));

        state.record(&TileInfo {
            state: TileState::Died {
                at: 200.0,
                cause: "the boss".into(),
            },
            ..info(2, 3, 200.0)
        });
        assert_eq!(state.tally.deaths[3], 1);
        assert_eq!(state.tally.average_survival(), Some(150.0));

        let won = TileInfo {
            state: TileState::Won { total_time: 700.0 },
            ..info(1, 2, 700.0)
        };
        state.record(&won);
        assert_eq!(state.tally.wins, 1);
        assert_eq!(state.tally.runs(), 3);
        let compact = result_detail(&state, 57);
        assert!(compact.contains("L1 1 L2 0 L3 0 L4 1"));
        assert!(compact.contains("W/L Exp 0/1 Skl 1/0 Cas 0/1"));
        let opts = AiOpts { skill: WallSkill::Mixed, ..AiOpts::default() };
        for (idx, expected) in SKILL_CYCLE.into_iter().cycle().take(6).enumerate() {
            assert_eq!(opts.auto_opts(idx).skill, expected);
        }
    }

    #[test]
    fn summary_reports_the_wall_and_fits_the_width() {
        let state = wall(16);
        let line = summary_text(&state, 240, 0, true);
        for field in ["AI 16/16 running", "0 finished", "0 wins", "0 losses", "Losses L1 0 L2 0 L3 0 L4 0", "avg", "8x", "sim s/s"] {
            assert!(line.contains(field), "{line:?} lacks {field:?}");
        }
        // A narrow strip degrades to a prefix instead of overflowing.
        let narrow = summary_text(&state, 12, 0, true);
        assert!(narrow.chars().count() <= 12, "{narrow:?}");
        assert!(narrow.starts_with("8x"), "{narrow:?}");
    }

    fn step_and_capture(run: &mut AutoRun, index: usize, cells: (u16, u16)) -> TileFrame {
        let (cw, ch) = cells;
        let scale = render_scale(cw);
        let mut pixels = PixelBuffer::new(cw as usize * scale as usize, ch as usize * 2 * scale as usize);
        run.world_mut().draw(&mut pixels);
        let mut buf = Buffer::empty(Rect::new(0, 0, cw, ch));
        downsample_into(&pixels, scale, buf.area, &mut buf);
        TileFrame {
            buf,
            info: tile_info(index, run),
            achieved: 8.0,
        }
    }

    #[test]
    fn autorun_frames_are_real_world_renders_and_never_queue_audio() {
        let mut run = AutoRun::new(2, AutoOpts::practice(0, Skill::Casual));
        for _ in 0..120 {
            run.step(SIM_DT);
            assert!(
                run.world().sfx.is_empty(),
                "a self-playing run must drain and discard every sfx event"
            );
        }
        assert!(!run.finished(), "2 s must not finish a practice level");
        let frame = step_and_capture(&mut run, 0, (40, 11));
        // Half-block cells, and at least some of them are lit by the floor art.
        let mut lit = 0;
        for y in 0..frame.buf.area.height {
            for x in 0..frame.buf.area.width {
                let cell = frame.buf.cell((x, y)).unwrap();
                assert_eq!(cell.symbol(), "▀");
                if cell.bg != Color::Rgb(0, 0, 0) {
                    lit += 1;
                }
            }
        }
        assert!(lit > 0, "a real world render must paint the floor");
        assert_eq!(frame.info.state, TileState::Running);
        assert!(frame.info.elapsed > 1.0 && frame.info.elapsed < 3.0);

        let mut state = WallState::new(1);
        state.frames[0] = Some(frame);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        assert!(text(&buf).contains("#1"));
    }

    #[test]
    fn an_ended_run_shows_its_plate_and_then_restarts() {
        let mut run = AutoRun::new(3, AutoOpts::practice(0, Skill::Casual));
        // Die on the first tick, without waiting for a real death.
        run.world_mut().game_over = true;
        run.step(SIM_DT);
        assert!(run.finished(), "a game_over world must finish the run");
        let dead = step_and_capture(&mut run, 0, (40, 11));
        assert!(matches!(dead.info.state, TileState::Died { .. }), "{:?}", dead.info.state);

        let mut state = WallState::new(1);
        state.frames[0] = Some(dead);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        assert!(text(&buf).contains("DIED"), "outcome plate missing");

        // The plate stays up for the restart delay and then the worker starts
        // a fresh run (unless --once), whose frame is a normal caption again.
        assert!(!should_restart(Duration::from_secs_f32(RESTART_DELAY_SECS / 2.0)));
        assert!(should_restart(Duration::from_secs_f32(RESTART_DELAY_SECS)));
        let mut fresh = AutoRun::new(4, AutoOpts::practice(0, Skill::Casual));
        fresh.step(SIM_DT);
        let alive = step_and_capture(&mut fresh, 0, (40, 11));
        state.frames[0] = Some(alive);
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &state);
        let screen = text(&buf);
        assert!(!screen.contains("DIED"), "a restarted game must drop its plate");
        assert!(screen.contains("#1"));
    }

    #[test]
    fn a_number_key_past_the_last_game_zooms_nothing() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut state = WallState::new(4);
        let area = Rect::new(0, 0, 80, 24);
        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, None, "game 9 does not exist on a four-game wall");

        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, Some(3), "the last real game still zooms");
    }

    #[test]
    fn speed_changes_are_driven_by_the_plus_and_minus_keys() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut state = WallState::new(8);
        let area = Rect::new(0, 0, 80, 24);
        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.speed(), 16.0, "+ steps the ladder up");

        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.speed(), 8.0, "- steps it back down");

        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert!(state.paused, "space pauses every game");
        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert!(!state.paused);

        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, Some(2), "3 zooms the third game");
        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, None, "Esc returns to the wall");
        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, Some(4));
        let mut input = InputState::new(false);
        input.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, None, "the same key toggles zoom off");
    }

    #[test]
    fn a_click_on_a_tile_zooms_that_game() {
        let mut state = WallState::new(16);
        let area = Rect::new(0, 0, 160, 48);
        let rect = plan_wall(16, area).tile_rect(6).unwrap();
        let mut input = InputState::new(false);
        input.handle_mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 1,
            row: rect.y + 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        spectate_keys(&mut state, &input, area);
        assert_eq!(state.zoom, Some(6));
    }

    #[test]
    fn live_workers_publish_frames_and_shut_down_cleanly() {
        let opts = AiOpts {
            games: 2,
            seed: Some(11),
            skill: WallSkill::Fixed(Skill::Casual),
            speed_idx: DEFAULT_SPEED_IDX,
            level: Some(1),
            once: true,
        };
        let mut wall = LiveWall::spawn(2, &opts, 11);
        let area = Rect::new(0, 0, 120, 36);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            wall.pump(area);
            if wall.state.frames.iter().all(|f| f.is_some()) || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for (i, frame) in wall.state.frames.iter().enumerate() {
            let frame = frame
                .as_ref()
                .unwrap_or_else(|| panic!("worker {i} never published a frame"));
            assert_eq!(frame.info.index, i);
            assert!(frame.buf.area.width > 0 && frame.buf.area.height > 0);
            // Practice level 1 only: the caption's campaign level is 1 (Sewer).
            assert_eq!(frame.info.level, 0);
        }
        let mut buf = Buffer::empty(area);
        render_wall(&mut buf, area, &wall.state);
        assert!(text(&buf).contains("#1") && text(&buf).contains("#2"));

        // Give the workers a moment of wall time so the achieved rate is
        // measurable, then check the summary reflects it.
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(800) {
            wall.pump(area);
            std::thread::sleep(Duration::from_millis(10));
        }
        for frame in wall.state.frames.iter().flatten() {
            assert!(
                frame.achieved >= 1.0,
                "a worker must report its simulated seconds per wall second"
            );
        }

        // Speed/pause travel to workers; shutdown joins every thread.
        wall.state.speed_up();
        wall.state.paused = true;
        wall.pump(area);
        wall.shutdown();
    }

    #[test]
    fn max_workers_publish_rates_and_respond_to_pause_and_zoom() {
        let opts = AiOpts {
            speed_idx: SPEEDS.len() - 1,
            seed: Some(11),
            ..AiOpts::default()
        };
        let mut wall = LiveWall::spawn(2, &opts, 11);
        let area = Rect::new(0, 0, 120, 36);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            wall.pump(area);
            if wall.state.frames.iter().all(|frame| {
                frame.as_ref().is_some_and(|f| f.achieved > 0.0 && f.buf.area.width > 0)
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "MAX workers did not publish rates");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(summary_text(&wall.state, 120, 0, false).contains("MAX "));

        wall.state.paused = true;
        wall.state.toggle_zoom(1);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            wall.pump(area);
            if wall.state.frames[1].as_ref().is_some_and(|frame| {
                frame.buf.area.width == area.width
                    && frame.achieved == 0.0
                    && matches!(frame.info.state, TileState::Running)
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "MAX worker did not pause and resize");
            std::thread::sleep(Duration::from_millis(10));
        }
        wall.shutdown();
    }

    /// Diagnostic, not part of the suite: `cargo test --release
    /// ai_wall_16_speed_report -- --ignored --nocapture` starts the real
    /// 16-game wall at 240x70 and reports each game's simulated seconds per
    /// wall second (the number the summary strip shows).
    #[test]
    #[ignore = "diagnostic: reports the 16-game wall's achieved speed"]
    fn ai_wall_16_speed_report() {
        let opts = AiOpts {
            games: 16,
            seed: Some(5),
            skill: WallSkill::Fixed(Skill::Expert),
            speed_idx: DEFAULT_SPEED_IDX,
            level: None,
            once: false,
        };
        let mut wall = LiveWall::spawn(16, &opts, 5);
        let area = Rect::new(0, 0, 240, 70);
        let start = Instant::now();
        let seconds = std::env::var("RATRUN_AI_REPORT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(10);
        let mut next_report = 2u64;
        while start.elapsed() < Duration::from_secs(seconds) {
            wall.pump(area);
            std::thread::sleep(Duration::from_millis(16));
            if start.elapsed().as_secs() >= next_report {
                next_report += 2;
                println!(
                    "t={}s {:?}",
                    start.elapsed().as_secs(),
                    wall.state
                        .frames
                        .iter()
                        .flatten()
                        .map(|f| (f.info.index, (f.info.elapsed * 10.0).round() / 10.0, (f.achieved * 10.0).round() / 10.0, f.info.state.glyph()))
                        .collect::<Vec<_>>()
                );
            }
        }
        let mut rates = Vec::new();
        let mut failed = 0;
        for frame in wall.state.frames.iter().flatten() {
            if matches!(frame.info.state, TileState::Failed { .. }) {
                failed += 1;
            }
            rates.push(frame.achieved);
        }
        rates.sort_by(f32::total_cmp);
        let mean = rates.iter().sum::<f32>() / rates.len().max(1) as f32;
        println!(
            "per-game achieved: {:?}",
            wall.state.frames.iter().flatten().map(|f| (f.info.index, f.achieved)).collect::<Vec<_>>()
        );
        println!(
            "wall 16 @ 240x70, {} s: achieved sim s/s per game min={:.2} median={:.2} max={:.2} mean={:.2}; failed={failed}; levels={:?}",
            seconds,
            rates.first().copied().unwrap_or(0.0),
            rates.get(rates.len() / 2).copied().unwrap_or(0.0),
            rates.last().copied().unwrap_or(0.0),
            mean,
            wall.state
                .frames
                .iter()
                .flatten()
                .map(|f| (f.info.level + 1, crate::game::campaign::level_theme(f.info.level).name()))
                .collect::<Vec<_>>()
        );
        wall.shutdown();
    }

    #[test]
    fn the_wall_never_touches_the_audio_engine() {
        // Structural guard: spectate mode must never construct an AudioEngine,
        // whatever the normal game's mute setting is.
        let source = include_str!("spectate.rs");
        // Built at runtime so the needles themselves are not in the source.
        let ctor = ["Audio", "Engine", "::", "new"].concat();
        let module = ["crate", "::", "au", "dio"].concat();
        let use_audio = ["use ", "crate", "::", "au", "dio"].concat();
        assert!(!source.contains(&ctor), "spectate.rs constructs an AudioEngine");
        assert!(!source.contains(&module) && !source.contains(&use_audio));
    }

    #[test]
    fn clicks_find_visible_tiles_only() {
        let state = wall(16);
        let area = Rect::new(0, 0, 80, 24);
        let plan = plan_wall(16, area);
        for i in 0..16 {
            let r = plan.tile_rect(i).unwrap();
            assert_eq!(state.game_at(area, r.x, r.y), Some(i));
        }
        let last = plan.tile_rect(15).unwrap();
        assert_eq!(
            state.game_at(area, last.x + last.width - 1, last.y + last.height - 1),
            Some(15)
        );
        // The summary strip below the grid belongs to no game.
        assert_eq!(state.game_at(area, 78, 23), None);
        assert_eq!(state.game_at(area, 200, 200), None);
    }
}
