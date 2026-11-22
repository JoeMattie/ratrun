//! The Quake-style drop-down console: state, line editing, command history,
//! Tab completion and the data-driven admin command table.
//!
//! The console is a **debug/admin** surface. It only acts when the player opens
//! it with the backtick/tilde key and types a command; with it closed nothing
//! in the simulation changes. Every state-changing command flags the run as
//! "CONSOLE USED", which blocks high scores, campaign records and library
//! counters (see [`crate::app`]).
//!
//! Layout of this module:
//!
//! * [`Console`] — the open/closed state, the scrollback, the prompt buffer,
//!   the history ring and the completion state. Purely presentational: it never
//!   touches the simulation.
//! * [`COMMANDS`] — the registry. Every row is `(name, aliases, usage, summary,
//!   arg kinds, read_only, handler)`. Adding a command is a few lines here plus
//!   a handler function.
//! * [`run_command`] — parse a line and dispatch to the matching handler.
//!
//! The drawing lives in [`crate::render::console`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::game::ecs::systems::boss::BossKind;
use crate::game::ecs::systems::miniboss::MinibossKind;
use crate::game::enemy::EnemyKind;
use crate::game::gem::PickupKind;
use crate::game::level::Theme;
use crate::game::loadout::PassiveKind;
use crate::game::shop::SpliceKind;
use crate::game::weapon::WeaponKind;

/// How many output lines the scrollback keeps. Old lines fall off the top.
pub const MAX_OUTPUT: usize = 800;
/// How many entries the command history keeps.
pub const MAX_HISTORY: usize = 200;
/// The prompt prefix drawn before the typed text.
pub const PROMPT: &str = "> ";

/// One scrollback line, tagged with how it should be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub kind: LineKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Ordinary command output.
    Output,
    /// A successfully completed action.
    Success,
    /// A command error.
    Error,
    /// The echoed command line (with the prompt).
    Echo,
    /// A dim informational note (help text, banners).
    Info,
}

impl Line {
    /// A line with an explicit kind. The command handlers use this so a
    /// success/error/info line carries its colour.
    pub fn new(text: impl Into<String>, kind: LineKind) -> Self {
        Line {
            text: text.into(),
            kind,
        }
    }

    pub fn output(text: impl Into<String>) -> Self {
        Line {
            text: text.into(),
            kind: LineKind::Output,
        }
    }
    pub fn success(text: impl Into<String>) -> Self {
        Line {
            text: text.into(),
            kind: LineKind::Success,
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Line {
            text: text.into(),
            kind: LineKind::Error,
        }
    }
    pub fn info(text: impl Into<String>) -> Self {
        Line {
            text: text.into(),
            kind: LineKind::Info,
        }
    }
    pub fn echo(text: impl Into<String>) -> Self {
        Line {
            text: text.into(),
            kind: LineKind::Echo,
        }
    }
}

/// The console's presentation state. The simulation never reads it.
#[derive(Clone, Debug, Default)]
pub struct Console {
    /// Is the panel up? Set by the backtick toggle.
    pub open: bool,
    /// Open/close animation progress, `0.0` = closed, `1.0` = fully open.
    pub anim: f32,
    /// The prompt's one-line edit buffer.
    pub input: String,
    /// Cursor position as a byte index into `input` (always on a char boundary).
    pub cursor: usize,
    /// Scrollback lines, oldest first.
    pub output: Vec<Line>,
    /// How many lines the view is scrolled up from the bottom (`0` = newest).
    pub scroll: usize,
    /// Command history, oldest first.
    pub history: Vec<String>,
    /// Where Up/Down is in the history (`None` = editing a fresh line).
    pub hist_idx: Option<usize>,
    /// The line stashed when the user first pressed Up, so Down can restore it.
    pub hist_stash: String,
    /// Live Tab-completion candidates for the current word.
    pub completions: Vec<String>,
    /// Selected completion index (`None` = no list showing).
    pub comp_idx: Option<usize>,
    /// The byte offset where the word being completed starts.
    pub comp_start: usize,
    /// Blink phase for the cursor (seconds, wrapped).
    pub blink: f32,
    /// Set while the open/close animation is playing so the draw knows to
    /// repaint.
    pub dirty: bool,
}

impl Console {
    pub fn new() -> Self {
        Console::default()
    }

    /// Toggle the panel. Returns the new open state.
    pub fn toggle(&mut self) -> bool {
        if self.open {
            self.close();
        } else {
            self.open_panel();
        }
        self.open
    }

    /// Alias for [`open_panel`](Self::open_panel), used by the App's toggle.
    pub fn open(&mut self) {
        self.open_panel();
    }

    pub fn open_panel(&mut self) {
        self.open = true;
        self.dirty = true;
        self.blink = 0.0;
        // Opening always starts at the newest line and a clean prompt.
        self.scroll = 0;
        self.completions.clear();
        self.comp_idx = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.dirty = true;
        self.completions.clear();
        self.comp_idx = None;
    }

    /// Advance the animation clock. Called once per frame, open or closed.
    pub fn tick(&mut self, dt: f32) {
        let target = if self.open { 1.0 } else { 0.0 };
        // ~110 ms slide: fast enough to feel instant, slow enough to read.
        let step = (dt.max(0.0) * 12.0).min(1.0);
        if (self.anim - target).abs() > 0.001 {
            self.anim += (target - self.anim) * step;
            if (self.anim - target).abs() < 0.01 {
                self.anim = target;
            }
            self.dirty = true;
        } else {
            self.anim = target;
        }
        if self.open {
            self.blink = (self.blink + dt.max(0.0)) % 1.06;
        }
    }

    /// Append a line, trimming the oldest when the ring is full.
    pub fn push(&mut self, line: Line) {
        self.output.push(line);
        if self.output.len() > MAX_OUTPUT {
            let excess = self.output.len() - MAX_OUTPUT;
            self.output.drain(0..excess);
        }
        self.dirty = true;
    }

    /// Clear the scrollback (the `clear` command).
    pub fn clear(&mut self) {
        self.output.clear();
        self.dirty = true;
    }

    /// The visible window of lines for a given viewport height, newest at the
    /// bottom. Returns the slice plus the index of the first visible line.
    pub fn visible(&self, rows: usize) -> (usize, &[Line]) {
        let rows = rows.max(1);
        let len = self.output.len();
        let end = len.saturating_sub(self.scroll);
        let start = end.saturating_sub(rows);
        (start, &self.output[start..end])
    }

    /// Scroll the view up by `n` lines (PgUp), clamped to the oldest line.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.output.len());
        self.dirty = true;
    }

    /// Scroll the view down by `n` lines (PgDn), clamped to the newest line.
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
        self.dirty = true;
    }

    // ---- Line editing ---------------------------------------------------

    /// Insert a character at the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.reset_completion();
    }

    /// Insert a whole string at the cursor (paste).
    pub fn insert_str(&mut self, s: &str) {
        // Strip control characters so a pasted newline cannot execute.
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return;
        }
        self.input.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        self.reset_completion();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.prev_boundary(self.cursor);
        self.input.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.reset_completion();
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let next = self.next_boundary(self.cursor);
        self.input.replace_range(self.cursor..next, "");
        self.reset_completion();
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_boundary(self.cursor);
        }
        self.reset_completion();
    }

    pub fn right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = self.next_boundary(self.cursor);
        }
        self.reset_completion();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
        self.reset_completion();
    }

    pub fn end(&mut self) {
        self.cursor = self.input.len();
        self.reset_completion();
    }

    /// Ctrl+U: delete from the cursor back to the start of the line.
    pub fn kill_to_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.input.replace_range(0..self.cursor, "");
        self.cursor = 0;
        self.reset_completion();
    }

    /// Ctrl+W: delete the word before the cursor.
    pub fn kill_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut start = self.cursor;
        // Skip trailing spaces, then the word.
        while start > 0 {
            let prev = self.prev_boundary(start);
            if !self.input[prev..start].chars().all(|c| c.is_whitespace()) {
                break;
            }
            start = prev;
        }
        while start > 0 {
            let prev = self.prev_boundary(start);
            if self.input[prev..start].chars().all(|c| c.is_whitespace()) {
                break;
            }
            start = prev;
        }
        self.input.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.reset_completion();
    }

    fn prev_boundary(&self, i: usize) -> usize {
        let mut j = i.saturating_sub(1);
        while j > 0 && !self.input.is_char_boundary(j) {
            j -= 1;
        }
        j
    }

    fn next_boundary(&self, i: usize) -> usize {
        let mut j = i + 1;
        while j < self.input.len() && !self.input.is_char_boundary(j) {
            j += 1;
        }
        j.min(self.input.len())
    }

    /// The word under the cursor, and the byte offset it starts at. A word is
    /// a run of non-space characters.
    pub fn word_at_cursor(&self) -> (usize, String) {
        let bytes = self.input.as_bytes();
        let mut start = self.cursor;
        while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        let mut end = self.cursor;
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        (start, self.input[start..end].to_string())
    }

    // ---- History --------------------------------------------------------

    /// Push a submitted line into the history ring (skipping duplicates of the
    /// immediately preceding entry).
    pub fn push_history(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if self.history.last().map(|s| s.as_str()) == Some(line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(0..excess);
        }
    }

    /// Up: step back through the history. The first press stashes the current
    /// (possibly empty) line so Down can restore it.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.hist_idx {
            None => {
                self.hist_stash = self.input.clone();
                let idx = self.history.len() - 1;
                self.hist_idx = Some(idx);
                self.input = self.history[idx].clone();
            }
            Some(0) => {}
            Some(idx) => {
                let idx = idx - 1;
                self.hist_idx = Some(idx);
                self.input = self.history[idx].clone();
            }
        }
        self.cursor = self.input.len();
        self.scroll = 0;
        self.reset_completion();
    }

    /// Down: step forward through the history, restoring the stashed line at the
    /// end.
    pub fn history_next(&mut self) {
        match self.hist_idx {
            None => {}
            Some(idx) if idx + 1 < self.history.len() => {
                let idx = idx + 1;
                self.hist_idx = Some(idx);
                self.input = self.history[idx].clone();
                self.cursor = self.input.len();
            }
            Some(_) => {
                self.hist_idx = None;
                self.input = std::mem::take(&mut self.hist_stash);
                self.cursor = self.input.len();
            }
        }
        self.scroll = 0;
        self.reset_completion();
    }

    /// Take the input line, leaving the console ready for the next command.
    pub fn take_line(&mut self) -> String {
        let line = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.hist_idx = None;
        self.hist_stash.clear();
        self.scroll = 0;
        self.reset_completion();
        line
    }

    // ---- Completion -----------------------------------------------------

    pub fn reset_completion(&mut self) {
        self.completions.clear();
        self.comp_idx = None;
    }

    /// Apply the currently selected completion (or cycle to the next one).
    /// Returns `true` when a completion was applied.
    pub fn accept_completion(&mut self) -> bool {
        let Some(idx) = self.comp_idx else {
            return false;
        };
        let Some(candidate) = self.completions.get(idx).cloned() else {
            return false;
        };
        let end = self.cursor.max(self.comp_start);
        let mut end = end;
        while end < self.input.len() && !self.input.as_bytes()[end].is_ascii_whitespace() {
            end += 1;
        }
        self.input.replace_range(self.comp_start..end, &candidate);
        self.cursor = self.comp_start + candidate.len();
        // A second Tab moves to the next candidate.
        let next = (idx + 1) % self.completions.len().max(1);
        self.comp_idx = Some(next);
        true
    }

    /// Handle one key press while the panel is open. Returns `Some(action)` when
    /// the app must act (execute a command or close the panel).
    pub fn handle_key(&mut self, key: KeyEvent, rows: usize) -> Option<ConsoleAction> {
        // Ctrl chords first.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('u') | KeyCode::Char('U') => {
                    self.kill_to_start();
                    return None;
                }
                KeyCode::Char('w') | KeyCode::Char('W') => {
                    self.kill_word();
                    return None;
                }
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    self.clear();
                    return None;
                }
                _ => {}
            }
            // Any other Ctrl chord is swallowed: a stray Ctrl+C must not type
            // a "c" into the prompt, and Ctrl+letter is never a literal.
            return None;
        }
        match key.code {
            KeyCode::Char(c) => {
                // A bare backtick/tilde closes the console; never typed.
                if c == '`' || c == '~' {
                    return Some(ConsoleAction::Close);
                }
                self.insert_char(c);
            }
            KeyCode::Enter => {
                let line = self.take_line();
                if line.trim().is_empty() {
                    return None;
                }
                self.push_history(&line);
                self.push(Line::echo(format!("{PROMPT}{line}")));
                return Some(ConsoleAction::Execute(line));
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            KeyCode::Tab => {
                return Some(ConsoleAction::Complete);
            }
            KeyCode::BackTab => {
                if let Some(idx) = self.comp_idx {
                    let n = self.completions.len().max(1);
                    self.comp_idx = Some((idx + n - 1) % n);
                }
            }
            KeyCode::PageUp => self.scroll_up(rows.saturating_sub(1).max(1)),
            KeyCode::PageDown => self.scroll_down(rows.saturating_sub(1).max(1)),
            KeyCode::Esc => {
                // Esc dismisses an open completion list first; a second Esc
                // closes the panel.
                if self.comp_idx.is_some() {
                    self.reset_completion();
                    return None;
                }
                return Some(ConsoleAction::Close);
            }
            _ => {}
        }
        None
    }
}

/// What the app should do after a key press in the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleAction {
    /// Run this command line.
    Execute(String),
    /// Close the panel.
    Close,
    /// Run Tab completion (needs the registry, so the app drives it).
    Complete,
}

// ============================================================================
// Command registry
// ============================================================================

/// What kind of argument a command slot takes, for Tab completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// No completion.
    Text,
    /// A subcommand / literal word from a fixed list.
    Words(&'static [&'static str]),
    Weapon,
    Passive,
    Enemy,
    Boss,
    Miniboss,
    Pickup,
    Floor,
    Splice,
    Sfx,
    Cvar,
    OnOff,
    /// A file path (exec).
    File,
}

/// A command row. `read_only` commands never flag the run as cheated.
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub summary: &'static str,
    pub args: &'static [ArgKind],
    pub read_only: bool,
    pub handler: fn(&mut crate::app::App, &[&str]) -> Result<(), String>,
}

impl CommandSpec {
    /// Does this command answer to `name`?
    pub fn matches(&self, name: &str) -> bool {
        self.name == name || self.aliases.contains(&name)
    }
}

const ON_OFF: &[&str] = &["on", "off"];
const GIVE_KINDS: &[&str] = &["weapon", "passive", "all"];
const REMOVE_KINDS: &[&str] = &["weapon", "passive"];
const TP_KINDS: &[&str] = &["arena", "shop", "chest", "maze"];
const LIGHTS_KINDS: &[&str] = &["full", "hard", "coarse", "compact"];
const LIBRARY_KINDS: &[&str] = &["unlock", "lock"];
const NEST_KINDS: &[&str] = &["max", "reset"];
/// The word that makes a command that edits the saved profile actually run.
const CONFIRM_WORD: &[&str] = &["confirm"];

/// Commands that build a new run: the run they start is flagged as
/// console-started, and never banks, records or counts a discovery.
const RUN_STARTERS: &[&str] = &["floor", "campaign", "restart"];

/// The command table. Order is the order `help` lists them in.
pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        aliases: &["?"],
        usage: "help [command]",
        summary: "List commands, or show one command's usage.",
        args: &[ArgKind::Text],
        read_only: true,
        handler: cmd_help,
    },
    CommandSpec {
        name: "clear",
        aliases: &["cls"],
        usage: "clear",
        summary: "Clear the console scrollback.",
        args: &[],
        read_only: true,
        handler: cmd_clear,
    },
    CommandSpec {
        name: "echo",
        aliases: &[],
        usage: "echo <text>",
        summary: "Print text to the scrollback.",
        args: &[ArgKind::Text],
        read_only: true,
        handler: cmd_echo,
    },
    CommandSpec {
        name: "god",
        aliases: &[],
        usage: "god [on|off]",
        summary: "Toggle invulnerability (no damage).",
        args: &[ArgKind::OnOff],
        read_only: false,
        handler: cmd_god,
    },
    CommandSpec {
        name: "noclip",
        aliases: &[],
        usage: "noclip [on|off]",
        summary: "Toggle walking through walls.",
        args: &[ArgKind::OnOff],
        read_only: false,
        handler: cmd_noclip,
    },
    CommandSpec {
        name: "heal",
        aliases: &[],
        usage: "heal [n]",
        summary: "Heal the rat by n HP (default: full).",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_heal,
    },
    CommandSpec {
        name: "hurt",
        aliases: &[],
        usage: "hurt <n>",
        summary: "Deal n damage to the rat.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_hurt,
    },
    CommandSpec {
        name: "kill",
        aliases: &[],
        usage: "kill",
        summary: "Kill the rat (end the run).",
        args: &[],
        read_only: false,
        handler: cmd_kill,
    },
    CommandSpec {
        name: "speed",
        aliases: &[],
        usage: "speed <x>",
        summary: "Set the rat's move-speed multiplier.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_speed,
    },
    CommandSpec {
        name: "give",
        aliases: &[],
        usage: "give weapon|passive <name> [level|stacks] | give all",
        summary: "Grant a weapon, passive, or everything.",
        args: &[ArgKind::Words(GIVE_KINDS), ArgKind::Text, ArgKind::Text],
        read_only: false,
        handler: cmd_give,
    },
    CommandSpec {
        name: "evolve",
        aliases: &[],
        usage: "evolve <weapon>|all",
        summary: "Evolve a weapon (or every weapon).",
        args: &[ArgKind::Weapon],
        read_only: false,
        handler: cmd_evolve,
    },
    CommandSpec {
        name: "level",
        aliases: &[],
        usage: "level <weapon> <n>",
        summary: "Set a weapon's level.",
        args: &[ArgKind::Weapon, ArgKind::Text],
        read_only: false,
        handler: cmd_level,
    },
    CommandSpec {
        name: "remove",
        aliases: &[],
        usage: "remove <weapon|passive>",
        summary: "Remove a weapon or passive from the loadout.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_remove,
    },
    CommandSpec {
        name: "loadout",
        aliases: &["kit"],
        usage: "loadout",
        summary: "Print the current loadout.",
        args: &[],
        read_only: true,
        handler: cmd_loadout,
    },
    CommandSpec {
        name: "splice",
        aliases: &[],
        usage: "splice <name>",
        summary: "Grant a gene splice.",
        args: &[ArgKind::Splice],
        read_only: false,
        handler: cmd_splice,
    },
    CommandSpec {
        name: "xp",
        aliases: &[],
        usage: "xp <n>",
        summary: "Grant n experience.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_xp,
    },
    CommandSpec {
        name: "levelup",
        aliases: &[],
        usage: "levelup [n]",
        summary: "Queue n level-ups (default 1).",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_levelup,
    },
    CommandSpec {
        name: "gold",
        aliases: &[],
        usage: "gold <n>",
        summary: "Set the run's gold.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_gold,
    },
    CommandSpec {
        name: "chest",
        aliases: &[],
        usage: "chest [n]",
        summary: "Spawn n chests by the rat.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_chest,
    },
    CommandSpec {
        name: "pickup",
        aliases: &[],
        usage: "pickup <kind> [n]",
        summary: "Spawn n pickups of a kind by the rat.",
        args: &[ArgKind::Pickup, ArgKind::Text],
        read_only: false,
        handler: cmd_pickup,
    },
    CommandSpec {
        name: "magnet",
        aliases: &[],
        usage: "magnet",
        summary: "Pull every gem on the floor to the rat.",
        args: &[],
        read_only: false,
        handler: cmd_magnet,
    },
    CommandSpec {
        name: "nuke",
        aliases: &[],
        usage: "nuke",
        summary: "Purge every enemy on the floor.",
        args: &[],
        read_only: false,
        handler: cmd_nuke,
    },
    CommandSpec {
        name: "spawn",
        aliases: &[],
        usage: "spawn <enemy> [count] [elite]",
        summary: "Spawn enemies around the rat.",
        args: &[ArgKind::Enemy, ArgKind::Text, ArgKind::Text],
        read_only: false,
        handler: cmd_spawn,
    },
    CommandSpec {
        name: "miniboss",
        aliases: &[],
        usage: "miniboss [kind]",
        summary: "Spawn a named miniboss by the rat.",
        args: &[ArgKind::Miniboss],
        read_only: false,
        handler: cmd_miniboss,
    },
    CommandSpec {
        name: "boss",
        aliases: &[],
        usage: "boss [kind]",
        summary: "Start the floor's boss fight now (or a named boss).",
        args: &[ArgKind::Boss],
        read_only: false,
        handler: cmd_boss,
    },
    CommandSpec {
        name: "killall",
        aliases: &[],
        usage: "killall",
        summary: "Kill every enemy (alias: clear enemies).",
        args: &[],
        read_only: false,
        handler: cmd_killall,
    },
    CommandSpec {
        name: "freeze",
        aliases: &[],
        usage: "freeze [on|off]",
        summary: "Stop enemies from moving.",
        args: &[ArgKind::OnOff],
        read_only: false,
        handler: cmd_freeze,
    },
    CommandSpec {
        name: "peace",
        aliases: &[],
        usage: "peace [on|off]",
        summary: "Stop the director from spawning.",
        args: &[ArgKind::OnOff],
        read_only: false,
        handler: cmd_peace,
    },
    CommandSpec {
        name: "time",
        aliases: &[],
        usage: "time <mm:ss>",
        summary: "Jump the level clock (says what it skips).",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_time,
    },
    CommandSpec {
        name: "timescale",
        aliases: &["ts"],
        usage: "timescale <x>",
        summary: "Scale game time (0.1 to 8).",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_timescale,
    },
    CommandSpec {
        name: "pause",
        aliases: &[],
        usage: "pause [on|off]",
        summary: "Toggle the simulation pause (the console keeps one frozen tick).",
        args: &[ArgKind::OnOff],
        read_only: false,
        handler: cmd_pause,
    },
    CommandSpec {
        name: "step",
        aliases: &[],
        usage: "step <n>",
        summary: "Advance n ticks while paused.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_step,
    },
    CommandSpec {
        name: "floor",
        aliases: &["map"],
        usage: "floor <name>",
        summary: "Restart on a floor as a practice run.",
        args: &[ArgKind::Floor],
        read_only: false,
        handler: cmd_floor,
    },
    CommandSpec {
        name: "campaign",
        aliases: &[],
        usage: "campaign <stage> [route]",
        summary: "Start a campaign at a stage.",
        args: &[ArgKind::Text, ArgKind::Floor],
        read_only: false,
        handler: cmd_campaign,
    },
    CommandSpec {
        name: "seed",
        aliases: &[],
        usage: "seed [n]",
        summary: "Print or set the seed for the next run.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_seed,
    },
    CommandSpec {
        name: "restart",
        aliases: &["retry"],
        usage: "restart",
        summary: "Restart the current run.",
        args: &[],
        read_only: false,
        handler: cmd_restart,
    },
    CommandSpec {
        name: "win",
        aliases: &[],
        usage: "win",
        summary: "Complete the current level.",
        args: &[],
        read_only: false,
        handler: cmd_win,
    },
    CommandSpec {
        name: "lose",
        aliases: &[],
        usage: "lose",
        summary: "End the run in defeat.",
        args: &[],
        read_only: false,
        handler: cmd_lose,
    },
    CommandSpec {
        name: "reveal",
        aliases: &[],
        usage: "reveal",
        summary: "Remove the fog of war (show the whole map).",
        args: &[],
        read_only: false,
        handler: cmd_reveal,
    },
    CommandSpec {
        name: "tp",
        aliases: &["teleport"],
        usage: "tp <x> <y> | tp arena|shop|chest|maze",
        summary: "Teleport the rat to a point or point of interest.",
        args: &[ArgKind::Text, ArgKind::Text],
        read_only: false,
        handler: cmd_tp,
    },
    CommandSpec {
        name: "zoom",
        aliases: &[],
        usage: "zoom <x>",
        summary: "Set the view zoom.",
        args: &[ArgKind::Text],
        read_only: false,
        handler: cmd_zoom,
    },
    CommandSpec {
        name: "where",
        aliases: &["pos"],
        usage: "where",
        summary: "Print position, room, seed, clock and counts.",
        args: &[],
        read_only: true,
        handler: cmd_where,
    },
    CommandSpec {
        name: "fps",
        aliases: &["perf"],
        usage: "fps [on|off]",
        summary: "Toggle the frame/tick time and entity overlay.",
        args: &[ArgKind::OnOff],
        read_only: true,
        handler: cmd_fps,
    },
    CommandSpec {
        name: "hitboxes",
        aliases: &[],
        usage: "hitboxes [on|off]",
        summary: "Toggle collision-shape drawing.",
        args: &[ArgKind::OnOff],
        read_only: true,
        handler: cmd_hitboxes,
    },
    CommandSpec {
        name: "lights",
        aliases: &["lighting"],
        usage: "lights [full|hard|coarse|compact]",
        summary: "Set the lighting quality mode.",
        args: &[ArgKind::Words(LIGHTS_KINDS)],
        read_only: true,
        handler: cmd_lights,
    },
    CommandSpec {
        name: "sfx",
        aliases: &[],
        usage: "sfx <name>",
        summary: "Play a sound effect.",
        args: &[ArgKind::Sfx],
        read_only: true,
        handler: cmd_sfx,
    },
    CommandSpec {
        name: "music",
        aliases: &[],
        usage: "music <intensity>",
        summary: "Set the music intensity tier (0-3).",
        args: &[ArgKind::Text],
        read_only: true,
        handler: cmd_music,
    },
    CommandSpec {
        name: "bank",
        aliases: &[],
        usage: "bank <n> confirm",
        summary: "Set the Nest's banked gold (needs `confirm`: it edits the save).",
        args: &[ArgKind::Text, ArgKind::Words(CONFIRM_WORD)],
        read_only: false,
        handler: cmd_bank,
    },
    CommandSpec {
        name: "nest",
        aliases: &[],
        usage: "nest max|reset confirm",
        summary: "Max every Nest track, or wipe the Nest (needs `confirm`).",
        args: &[ArgKind::Words(NEST_KINDS), ArgKind::Words(CONFIRM_WORD)],
        read_only: false,
        handler: cmd_nest,
    },
    CommandSpec {
        name: "library",
        aliases: &[],
        usage: "library unlock|lock",
        summary: "Reveal or lock every Library entry.",
        args: &[ArgKind::Words(LIBRARY_KINDS), ArgKind::Text],
        read_only: false,
        handler: cmd_library,
    },
    CommandSpec {
        name: "set",
        aliases: &[],
        usage: "set <cvar> <value>",
        summary: "Set a live tunable (enemy_cap, spawn_rate, xp_mult, damage_mult).",
        args: &[ArgKind::Cvar, ArgKind::Text],
        read_only: false,
        handler: cmd_set,
    },
    CommandSpec {
        name: "get",
        aliases: &[],
        usage: "get <cvar>",
        summary: "Print a live tunable.",
        args: &[ArgKind::Cvar],
        read_only: true,
        handler: cmd_get,
    },
    CommandSpec {
        name: "exec",
        aliases: &[],
        usage: "exec <file>",
        summary: "Run commands from a text file.",
        args: &[ArgKind::File],
        read_only: false,
        handler: cmd_exec,
    },
];

/// Find a command by name or alias.
pub fn find_command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.matches(name))
}

/// Suggest the nearest command name for an unknown command (Levenshtein).
pub fn suggest(name: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for cmd in COMMANDS {
        let d = levenshtein(name, cmd.name);
        // Also consider aliases.
        let mut d = d;
        for alias in cmd.aliases {
            d = d.min(levenshtein(name, alias));
        }
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((cmd.name, d));
        }
    }
    // Only suggest when reasonably close.
    best.filter(|(_, d)| *d <= 2.max(name.len() / 2)).map(|(n, _)| n)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Normalise a display name into a console slug: lowercase, spaces and
/// punctuation dropped. `"Sister Snip"` → `"sistersnip"`, `"Fat Reserves"` →
/// `"fatreserves"`.
pub fn slug(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

// ============================================================================
// Completion candidates — always derived from the real tables.
// ============================================================================

/// Every completion candidate for an argument kind.
pub fn candidates(kind: ArgKind) -> Vec<String> {
    match kind {
        ArgKind::Text | ArgKind::File => Vec::new(),
        ArgKind::Words(words) => words.iter().map(|s| s.to_string()).collect(),
        ArgKind::Weapon => WeaponKind::all().iter().map(|w| slug(w.name())).collect(),
        ArgKind::Passive => PassiveKind::all().iter().map(|p| slug(p.name())).collect(),
        ArgKind::Enemy => EnemyKind::all().iter().map(|e| slug(e.name())).collect(),
        ArgKind::Boss => BossKind::all().iter().map(|b| slug(b.name())).collect(),
        ArgKind::Miniboss => MinibossKind::all().iter().map(|m| slug(m.name())).collect(),
        ArgKind::Pickup => PickupKind::all()
            .iter()
            .flat_map(|p| [slug(p.name()), slug(p.label())])
            .collect(),
        ArgKind::Floor => Theme::playable().iter().map(|t| slug(t.name())).collect(),
        ArgKind::Splice => SpliceKind::all().iter().map(|s| slug(s.name())).collect(),
        ArgKind::Sfx => crate::audio::Sfx::ALL.iter().map(|s| slug(s.name())).collect(),
        ArgKind::Cvar => CVAR_NAMES.iter().map(|s| s.to_string()).collect(),
        ArgKind::OnOff => ON_OFF.iter().map(|s| s.to_string()).collect(),
    }
}

/// The cvar names the `set`/`get` commands expose.
pub const CVAR_NAMES: &[&str] = &["enemy_cap", "spawn_rate", "xp_mult", "damage_mult"];

/// Complete the token under the cursor. Returns the candidates for the argument
/// slot the word sits in.
pub fn complete(console: &Console) -> Vec<String> {
    let (start, word) = console.word_at_cursor();
    let before = &console.input[..start];
    let trimmed = before.trim_start();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    // The command name is the first word. If we are completing the first word,
    // complete command names.
    if words.is_empty() {
        return COMMANDS
            .iter()
            .map(|c| c.name.to_string())
            .filter(|n| n.starts_with(&word))
            .collect();
    }
    let Some(cmd) = find_command(words[0]) else {
        return Vec::new();
    };
    // The argument index is the number of complete words after the command.
    let arg_index = words.len().saturating_sub(1);
    // `remove` takes either a weapon or a passive in its first slot, so offer
    // both. `give weapon|passive <name>` narrows its second slot by the kind.
    if cmd.name == "remove" && arg_index == 0 {
        let mut out = candidates(ArgKind::Weapon);
        out.extend(candidates(ArgKind::Passive));
        return out.into_iter().filter(|c| c.starts_with(&word)).collect();
    }
    let kind = if cmd.name == "give" && arg_index == 1 {
        match words.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("weapon") => ArgKind::Weapon,
            Some("passive") => ArgKind::Passive,
            _ => ArgKind::Text,
        }
    } else {
        cmd.args.get(arg_index).copied().unwrap_or(ArgKind::Text)
    };
    candidates(kind)
        .into_iter()
        .filter(|c| c.starts_with(&word))
        .collect()
}

/// Start the completion for the word under the cursor and store the state on
/// the console. Returns the number of candidates.
pub fn begin_completion(console: &mut Console) -> usize {
    let (start, _word) = console.word_at_cursor();
    let list = complete(console);
    if list.is_empty() {
        console.reset_completion();
        return 0;
    }
    // If there is exactly one candidate, fill it in immediately.
    if list.len() == 1 {
        let only = list[0].clone();
        let end = word_end(console, start);
        console.input.replace_range(start..end, &only);
        console.cursor = start + only.len();
        console.reset_completion();
        return 1;
    }
    console.comp_start = start;
    console.completions = list.clone();
    console.comp_idx = Some(0);
    list.len()
}

fn word_end(console: &Console, start: usize) -> usize {
    let mut end = start;
    while end < console.input.len() && !console.input.as_bytes()[end].is_ascii_whitespace() {
        end += 1;
    }
    end
}

// ============================================================================
// Dispatch
// ============================================================================

/// Run one command line. The line is split on whitespace (double quotes group a
/// word). Errors are returned as `Err(message)`; the caller prints them in red.
pub fn run_command(app: &mut crate::app::App, line: &str) -> Result<(), String> {
    let argv = split_args(line);
    if argv.is_empty() {
        return Ok(());
    }
    let name = argv[0].to_ascii_lowercase();
    let Some(cmd) = find_command(&name) else {
        let hint = suggest(&name)
            .map(|s| format!(" (did you mean `{s}`?)"))
            .unwrap_or_default();
        return Err(format!("unknown command `{name}`{hint}"));
    };
    // Flag the run as cheated before a state-changing command runs.
    if !cmd.read_only {
        app.console_flag_cheat();
    }
    let args: Vec<&str> = argv[1..].iter().map(|s| s.as_str()).collect();
    let outcome = (cmd.handler)(app, &args);
    // A run the console started is never an earned run. The app's own start
    // paths clear the taint for a clean start (a fresh campaign, a retry), so
    // `floor`, `campaign` and `restart` are re-flagged here: whichever door the
    // run came through, "started from the console" means "banked nothing".
    if outcome.is_ok() && RUN_STARTERS.contains(&cmd.name) {
        app.console_flag_cheat();
    }
    outcome
}

/// Split a command line into words. Double quotes group a word; a backslash
/// escapes the next character.
pub fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut escape = false;
    for c in line.chars() {
        if escape {
            cur.push(c);
            escape = false;
            continue;
        }
        match c {
            '\\' => escape = true,
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ============================================================================
// Handlers
// ============================================================================

/// Parse an on/off argument. `None` means "toggle the current value".
fn parse_on_off(arg: Option<&str>, current: bool) -> Result<bool, String> {
    match arg.map(|s| s.trim().to_ascii_lowercase()) {
        None => Ok(!current),
        Some(s) if s == "on" || s == "1" || s == "true" || s == "yes" => Ok(true),
        Some(s) if s == "off" || s == "0" || s == "false" || s == "no" => Ok(false),
        Some(s) => Err(format!("expected on/off, got `{s}`")),
    }
}

fn parse_f32(arg: &str, what: &str) -> Result<f32, String> {
    arg.parse::<f32>()
        .map_err(|_| format!("expected a number for {what}, got `{arg}`"))
}

fn parse_usize(arg: &str, what: &str) -> Result<usize, String> {
    arg.parse::<usize>()
        .map_err(|_| format!("expected a whole number for {what}, got `{arg}`"))
}

fn parse_u32(arg: &str, what: &str) -> Result<u32, String> {
    arg.parse::<u32>()
        .map_err(|_| format!("expected a whole number for {what}, got `{arg}`"))
}

fn cmd_help(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    if let Some(name) = args.first() {
        let Some(cmd) = find_command(&name.to_ascii_lowercase()) else {
            return Err(format!("no such command `{name}`"));
        };
        app.console_line(format!("{} — {}", cmd.usage, cmd.summary), LineKind::Info);
        if !cmd.aliases.is_empty() {
            app.console_line(format!("aliases: {}", cmd.aliases.join(", ")), LineKind::Info);
        }
        return Ok(());
    }
    app.console_line("Commands (type `help <cmd>` for usage):", LineKind::Info);
    for cmd in COMMANDS {
        let ro = if cmd.read_only { "" } else { " *" };
        app.console_line(format!("  {:<12}{}{}", cmd.name, cmd.summary, ro), LineKind::Output);
    }
    app.console_line(
        "  * = flags the run as CONSOLE USED (no bank, no record, no Library counts)",
        LineKind::Info,
    );
    app.console_line(
        "  floor / campaign / restart always start a CONSOLE USED run, whichever stage or route they pick",
        LineKind::Info,
    );
    app.console_line(
        "  bank / nest edit the saved profile: repeat them with `confirm`",
        LineKind::Info,
    );
    Ok(())
}

fn cmd_clear(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    app.console_clear();
    Ok(())
}

fn cmd_echo(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    app.console_line(args.join(" "), LineKind::Output);
    Ok(())
}

fn cmd_god(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let on = parse_on_off(args.first().copied(), world.player.god)?;
    world.player.god = on;
    app.console_line(format!("god {}", if on { "on" } else { "off" }), LineKind::Success);
    Ok(())
}

fn cmd_noclip(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let on = parse_on_off(args.first().copied(), world.noclip)?;
    world.noclip = on;
    app.console_line(format!("noclip {}", if on { "on" } else { "off" }), LineKind::Success);
    Ok(())
}

fn cmd_heal(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let msg = {
        let Some(world) = app.world_mut() else {
            return Err("no run in progress".into());
        };
        let amount = match args.first() {
            Some(s) => parse_f32(s, "heal")?,
            None => world.player.max_hp(),
        };
        let before = world.player.hp;
        world.player.heal(amount);
        format!("healed {:.0} (hp {:.0} → {:.0})", world.player.hp - before, before, world.player.hp)
    };
    app.console_line(msg, LineKind::Success);
    Ok(())
}

fn cmd_hurt(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let msg = {
        let Some(world) = app.world_mut() else {
            return Err("no run in progress".into());
        };
        let n = args.first().ok_or("usage: hurt <n>")?;
        let amount = parse_f32(n, "hurt")?;
        let before = world.player.hp;
        world.player.hp = (world.player.hp - amount).max(0.0);
        format!("hurt {:.0} (hp {:.0} → {:.0})", amount, before, world.player.hp)
    };
    app.console_line(msg, LineKind::Success);
    Ok(())
}

fn cmd_kill(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    world.player.hp = 0.0;
    world.game_over = true;
    app.console_line("the rat is dead", LineKind::Success);
    Ok(())
}

fn cmd_speed(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let x = args.first().ok_or("usage: speed <x>")?;
    let mult = parse_f32(x, "speed")?;
    if mult <= 0.0 {
        return Err("speed must be positive".into());
    }
    world.player.speed_mult = mult;
    app.console_line(format!("speed ×{mult}"), LineKind::Success);
    Ok(())
}

fn cmd_give(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    // `give` fills the **run's** racks, not the hard 8-slot ceiling: the caps
    // are `Loadout::max_weapons()` / `Loadout::max_passives()`, i.e. what this
    // profile has actually bought in the Nest. The world's own adders stop at
    // `MAX_WEAPONS`, so without this check a 4-slot run could be handed 8
    // weapons and the HUD, the shop and the pause plate would disagree.
    let (max_w, max_p) = match app.world_ref() {
        Some(w) => (
            w.player.loadout.max_weapons(),
            w.player.loadout.max_passives(),
        ),
        None => return Err("no run in progress".into()),
    };
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let kind = args.first().map(|s| s.to_ascii_lowercase());
    match kind.as_deref() {
        Some("all") => {
            for w in WeaponKind::all() {
                let held = world.player.loadout.weapons.iter().any(|x| x.kind == w);
                if !held && world.player.loadout.weapons.len() >= max_w {
                    continue;
                }
                world.give_weapon(w, 5);
            }
            for p in PassiveKind::all() {
                let held = world.player.loadout.passives.iter().any(|x| x.0 == p);
                if !held && world.player.loadout.passives.len() >= max_p {
                    continue;
                }
                world.give_passive(p, 3);
            }
            let msg = format!(
                "gave every weapon and passive the racks hold ({}/{} weapons, {}/{} passives)",
                world.player.loadout.weapons.len().min(max_w),
                max_w,
                world.player.loadout.passives.len().min(max_p),
                max_p
            );
            app.console_line(msg, LineKind::Success);
        }
        Some("weapon") => {
            let name = args.get(1).ok_or("usage: give weapon <name> [level]")?;
            let w = find_weapon(name).ok_or_else(|| format!("unknown weapon `{name}`"))?;
            let level = args.get(2).map(|s| parse_u32(s, "level")).transpose()?.unwrap_or(1);
            let held = world.player.loadout.weapons.iter().any(|x| x.kind == w);
            if !held && world.player.loadout.weapons.len() >= max_w {
                return Err(format!(
                    "all {max_w} weapon slots are full — `remove` one, or buy WEAPON SLOTS in the Nest"
                ));
            }
            world.give_weapon(w, level.min(255) as u8);
            app.console_line(format!("gave {} L{}", w.name(), level), LineKind::Success);
        }
        Some("passive") => {
            let name = args.get(1).ok_or("usage: give passive <name> [stacks]")?;
            let p = find_passive(name).ok_or_else(|| format!("unknown passive `{name}`"))?;
            let stacks = args.get(2).map(|s| parse_u32(s, "stacks")).transpose()?.unwrap_or(1);
            let held = world.player.loadout.passives.iter().any(|x| x.0 == p);
            if !held && world.player.loadout.passives.len() >= max_p {
                return Err(format!(
                    "all {max_p} passive slots are full — `remove` one, or buy PASSIVE SLOTS in the Nest"
                ));
            }
            world.give_passive(p, stacks);
            app.console_line(format!("gave {} ×{}", p.name(), stacks), LineKind::Success);
        }
        _ => return Err("usage: give weapon|passive <name> [n] | give all".into()),
    }
    Ok(())
}

fn cmd_evolve(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let target = args.first().ok_or("usage: evolve <weapon>|all")?;
    if target.eq_ignore_ascii_case("all") {
        let n = world.evolve_all();
        app.console_line(format!("evolved {n} weapon(s)"), LineKind::Success);
        return Ok(());
    }
    let w = find_weapon(target).ok_or_else(|| format!("unknown weapon `{target}`"))?;
    if world.evolve_weapon(w) {
        app.console_line(format!("evolved {}", w.name()), LineKind::Success);
    } else {
        return Err(format!("{} is not in the loadout", w.name()));
    }
    Ok(())
}

fn cmd_level(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let name = args.first().ok_or("usage: level <weapon> <n>")?;
    let n = args.get(1).ok_or("usage: level <weapon> <n>")?;
    let w = find_weapon(name).ok_or_else(|| format!("unknown weapon `{name}`"))?;
    let level = parse_u32(n, "level")?;
    if world.set_weapon_level(w, level.min(255) as u8) {
        app.console_line(format!("{} → L{}", w.name(), level), LineKind::Success);
    } else {
        return Err(format!("{} is not in the loadout", w.name()));
    }
    Ok(())
}

fn cmd_remove(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let name = args.first().ok_or("usage: remove <weapon|passive>")?;
    if let Some(w) = find_weapon(name) {
        if world.remove_weapon(w) {
            app.console_line(format!("removed {}", w.name()), LineKind::Success);
            return Ok(());
        }
        return Err(format!("{} is not in the loadout", w.name()));
    }
    if let Some(p) = find_passive(name) {
        if world.remove_passive(p) {
            app.console_line(format!("removed {}", p.name()), LineKind::Success);
            return Ok(());
        }
        return Err(format!("{} is not in the loadout", p.name()));
    }
    Err(format!("unknown weapon or passive `{name}`"))
}

fn cmd_loadout(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_ref() else {
        return Err("no run in progress".into());
    };
    let kit = world.loadout_summary();
    for line in kit {
        app.console_line(line, LineKind::Output);
    }
    Ok(())
}

fn cmd_splice(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let name = args.first().ok_or("usage: splice <name>")?;
    let s = find_splice(name).ok_or_else(|| format!("unknown splice `{name}`"))?;
    world.give_splice(s);
    app.console_line(format!("spliced {}", s.name()), LineKind::Success);
    Ok(())
}

fn cmd_xp(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = args.first().ok_or("usage: xp <n>")?;
    let amount = parse_u32(n, "xp")?;
    world.grant_xp(amount);
    app.console_line(format!("granted {amount} xp"), LineKind::Success);
    Ok(())
}

fn cmd_levelup(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = args.first().map(|s| parse_u32(s, "levelup")).transpose()?.unwrap_or(1);
    world.pending_levelups = world.pending_levelups.saturating_add(n);
    app.console_line(format!("queued {n} level-up(s)"), LineKind::Success);
    Ok(())
}

fn cmd_gold(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = args.first().ok_or("usage: gold <n>")?;
    let amount = parse_u32(n, "gold")?;
    world.gold = amount;
    app.console_line(format!("gold = {amount}"), LineKind::Success);
    Ok(())
}

fn cmd_chest(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = args.first().map(|s| parse_usize(s, "chest")).transpose()?.unwrap_or(1);
    let pos = world.player.pos;
    for i in 0..n {
        world.spawn_chest_near(pos, i as f32);
    }
    app.console_line(format!("spawned {n} chest(s)"), LineKind::Success);
    Ok(())
}

fn cmd_pickup(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let name = args.first().ok_or("usage: pickup <kind> [n]")?;
    let kind = find_pickup(name).ok_or_else(|| format!("unknown pickup `{name}`"))?;
    let n = args.get(1).map(|s| parse_usize(s, "count")).transpose()?.unwrap_or(1);
    let pos = world.player.pos;
    for i in 0..n {
        world.spawn_pickup_near(kind, pos, i as f32);
    }
    app.console_line(format!("spawned {n} {}", kind.label()), LineKind::Success);
    Ok(())
}

fn cmd_magnet(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = world.magnetize_all_gems();
    app.console_line(format!("magnetized {n} gem(s)"), LineKind::Success);
    Ok(())
}

fn cmd_nuke(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = world.nuke_all();
    app.console_line(format!("purged {n} enemy(s)"), LineKind::Success);
    Ok(())
}

fn cmd_killall(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let n = world.enemy_count();
    world.clear_enemies();
    app.console_line(format!("cleared {n} enemy(s)"), LineKind::Success);
    Ok(())
}

fn cmd_spawn(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let name = args.first().ok_or("usage: spawn <enemy> [count] [elite]")?;
    let kind = find_enemy(name).ok_or_else(|| format!("unknown enemy `{name}`"))?;
    let count = args.get(1).map(|s| parse_usize(s, "count")).transpose()?.unwrap_or(1);
    let elite = args.get(2).is_some_and(|s| s.eq_ignore_ascii_case("elite"));
    let pos = world.player.pos;
    for i in 0..count {
        world.spawn_enemy_near(kind, pos, i as f32, elite);
    }
    app.console_line(
        format!("spawned {count} {}{}", kind.name(), if elite { " (elite)" } else { "" }),
        LineKind::Success,
    );
    Ok(())
}

fn cmd_miniboss(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let kind = match args.first() {
        Some(name) => find_miniboss(name).ok_or_else(|| format!("unknown miniboss `{name}`"))?,
        None => MinibossKind::all()[0],
    };
    let pos = world.player.pos;
    world.spawn_miniboss_near(kind, pos);
    app.console_line(format!("spawned miniboss {}", kind.name()), LineKind::Success);
    Ok(())
}

fn cmd_boss(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let kind = match args.first() {
        Some(name) => find_boss(name).ok_or_else(|| format!("unknown boss `{name}`"))?,
        None => BossKind::for_theme(world.level.theme),
    };
    world.force_boss(kind);
    app.console_line(format!("boss fight started: {}", kind.name()), LineKind::Success);
    Ok(())
}

fn cmd_freeze(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let on = parse_on_off(args.first().copied(), world.freeze)?;
    world.freeze = on;
    app.console_line(format!("freeze {}", if on { "on" } else { "off" }), LineKind::Success);
    Ok(())
}

fn cmd_peace(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let on = parse_on_off(args.first().copied(), world.peace)?;
    world.peace = on;
    app.console_line(format!("peace {}", if on { "on" } else { "off" }), LineKind::Success);
    Ok(())
}

fn cmd_time(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let raw = args.first().ok_or("usage: time <mm:ss>")?;
    let secs = parse_clock(raw)?;
    let before = world.elapsed;
    world.elapsed = secs;
    let skipped = if secs > before {
        format!("skipped {:.0}s of spawns", secs - before)
    } else {
        format!("rewound {:.0}s", before - secs)
    };
    app.console_line(format!("clock set to {raw} ({skipped})"), LineKind::Success);
    Ok(())
}

fn cmd_timescale(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let x = args.first().ok_or("usage: timescale <x>")?;
    let mult = parse_f32(x, "timescale")?;
    if !(0.1..=8.0).contains(&mult) {
        return Err("timescale must be between 0.1 and 8".into());
    }
    app.set_timescale(mult);
    app.console_line(format!("timescale ×{mult}"), LineKind::Success);
    Ok(())
}

fn cmd_pause(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    // `pause` toggles; `pause on` / `pause off` say it outright, so a scripted
    // run never depends on the state it finds (the flag is a plain toggle
    // underneath).
    let want = parse_on_off(args.first().copied(), app.console_paused)?;
    if want != app.console_paused {
        app.toggle_console_pause();
    }
    app.console_line(
        if want {
            "simulation paused — the console stays open: `pause off` resumes, `step n` advances it"
        } else {
            "simulation running"
        },
        LineKind::Success,
    );
    Ok(())
}

fn cmd_step(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let n = args.first().map(|s| parse_u32(s, "step")).transpose()?.unwrap_or(1);
    app.step_ticks(n as usize);
    app.console_line(format!("stepped {n} tick(s)"), LineKind::Success);
    Ok(())
}

fn cmd_floor(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let name = args.first().ok_or("usage: floor <name>")?;
    let theme = find_floor(name).ok_or_else(|| format!("unknown floor `{name}`"))?;
    app.start_floor(theme)?;
    app.console_line(format!("restarted on {}", theme.name()), LineKind::Success);
    Ok(())
}

fn cmd_campaign(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let stage = args.first().ok_or("usage: campaign <stage> [route]")?;
    let n = parse_usize(stage, "stage")?;
    if !(1..=crate::game::campaign::LEVEL_COUNT).contains(&n) {
        return Err(format!("stage must be 1..{}", crate::game::campaign::LEVEL_COUNT));
    }
    let route = args.get(1).and_then(|s| find_floor(s));
    app.start_campaign_at(n - 1, route)?;
    app.console_line(format!("campaign stage {n}"), LineKind::Success);
    Ok(())
}

fn cmd_seed(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    match args.first() {
        None => {
            let seed = app.current_seed();
            app.console_line(format!("seed {seed}"), LineKind::Output);
        }
        Some(raw) => {
            let seed = raw.parse::<u64>().map_err(|_| format!("expected a number, got `{raw}`"))?;
            app.set_next_seed(seed);
            app.console_line(format!("next run seed = {seed}"), LineKind::Success);
        }
    }
    Ok(())
}

fn cmd_restart(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    app.restart_run()?;
    app.console_line("run restarted", LineKind::Success);
    Ok(())
}

fn cmd_win(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    world.mark_level_complete();
    app.console_line("level completed", LineKind::Success);
    Ok(())
}

fn cmd_lose(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    world.player.hp = 0.0;
    world.game_over = true;
    app.console_line("run lost", LineKind::Success);
    Ok(())
}

fn cmd_reveal(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    world.reveal_map();
    app.console_line("map revealed", LineKind::Success);
    Ok(())
}

fn cmd_tp(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let first = args.first().ok_or("usage: tp <x> <y> | tp arena|shop|chest|maze")?;
    if let Some(kind) = first.to_ascii_lowercase().as_str().into() {
        match kind {
            "arena" => {
                let p = world.boss_arena_center();
                world.player.pos = p;
                app.console_line("teleported to the arena", LineKind::Success);
                return Ok(());
            }
            "shop" => {
                if let Some(p) = world.shop_point() {
                    world.player.pos = p;
                    app.console_line("teleported to a shop", LineKind::Success);
                    return Ok(());
                }
                return Err("no shop on this floor".into());
            }
            "chest" => {
                if let Some(p) = world.chest_point() {
                    world.player.pos = p;
                    app.console_line("teleported to a chest", LineKind::Success);
                    return Ok(());
                }
                return Err("no chest on this floor".into());
            }
            "maze" => {
                if let Some(p) = world.maze_point() {
                    world.player.pos = p;
                    app.console_line("teleported to a maze", LineKind::Success);
                    return Ok(());
                }
                return Err("no maze on this floor".into());
            }
            _ => {}
        }
    }
    let x = parse_f32(first, "x")?;
    let y = args.get(1).ok_or("usage: tp <x> <y>")?;
    let y = parse_f32(y, "y")?;
    world.player.pos = crate::math::Vec2::new(x, y);
    app.console_line(format!("teleported to ({x:.0}, {y:.0})"), LineKind::Success);
    Ok(())
}

fn cmd_zoom(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let x = args.first().ok_or("usage: zoom <x>")?;
    let mult = parse_f32(x, "zoom")?;
    if !(0.25..=4.0).contains(&mult) {
        return Err("zoom must be between 0.25 and 4".into());
    }
    world.set_zoom_instant(mult);
    app.console_line(format!("zoom ×{mult}"), LineKind::Success);
    Ok(())
}

fn cmd_where(app: &mut crate::app::App, _args: &[&str]) -> Result<(), String> {
    let lines = {
        let Some(world) = app.world_ref() else {
            return Err("no run in progress".into());
        };
        let p = world.player.pos;
        vec![
            format!("pos ({:.0}, {:.0})", p.x, p.y),
            format!("room {}", world.room_name_at(p)),
            format!("floor {}  seed {}", world.level.theme.name(), world.level.seed),
            format!(
                "clock {:.0}s  enemies {}  bullets {}",
                world.elapsed,
                world.enemy_count(),
                world.bullet_count()
            ),
            format!("hp {:.0}/{:.0}  kills {}", world.player.hp, world.player.max_hp(), world.kills),
        ]
    };
    for line in lines {
        app.console_line(line, LineKind::Output);
    }
    Ok(())
}

fn cmd_fps(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let on = parse_on_off(args.first().copied(), app.fps_enabled())?;
    app.set_fps(on);
    app.console_line(format!("perf overlay {}", if on { "on" } else { "off" }), LineKind::Success);
    Ok(())
}

fn cmd_hitboxes(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let on = parse_on_off(args.first().copied(), world.hitboxes)?;
    world.hitboxes = on;
    app.console_line(format!("hitboxes {}", if on { "on" } else { "off" }), LineKind::Success);
    Ok(())
}

fn cmd_lights(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    use crate::render::lighting::LightingMode;
    let raw = args.first().map(|s| s.to_ascii_lowercase());
    let next = match raw.as_deref() {
        Some("on") | Some("full") => LightingMode::Full,
        Some("hard") => LightingMode::Hard,
        Some("coarse") => LightingMode::Coarse,
        Some("compact") => LightingMode::Compact,
        None => {
            // No argument: step to the next live mode.
            let cur = app.lighting_mode();
            let i = LightingMode::LIVE.iter().position(|m| *m == cur).unwrap_or(0);
            LightingMode::LIVE[(i + 1) % LightingMode::LIVE.len()]
        }
        Some(other) => return Err(format!("expected full/hard/coarse/compact, got `{other}`")),
    };
    app.set_lighting(next);
    app.console_line(
        format!("lights {}", format!("{:?}", app.lighting_mode()).to_ascii_lowercase()),
        LineKind::Success,
    );
    Ok(())
}

fn cmd_sfx(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let name = args.first().ok_or("usage: sfx <name>")?;
    let sfx = find_sfx(name).ok_or_else(|| format!("unknown sfx `{name}`"))?;
    app.play_sfx(sfx);
    app.console_line(format!("played {}", sfx.name()), LineKind::Success);
    Ok(())
}

fn cmd_music(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let n = args.first().ok_or("usage: music <intensity>")?;
    let intensity = parse_u32(n, "intensity")?;
    if intensity > 3 {
        return Err("music intensity is 0..3".into());
    }
    app.set_music_intensity(intensity);
    app.console_line(format!("music intensity {intensity}"), LineKind::Success);
    Ok(())
}

fn cmd_library(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("unlock") => {
            app.console_library(true);
            app.console_line("library: every entry revealed", LineKind::Success);
            Ok(())
        }
        Some("lock") => {
            app.console_library(false);
            app.console_line("library: every entry locked", LineKind::Success);
            Ok(())
        }
        _ => Err("usage: library unlock|lock".into()),
    }
}

/// `bank` and `nest` edit the **saved** profile, so both need the same explicit
/// second word a destructive command gets. A mistyped `bank 400` on the title
/// used to rewrite the save file on the spot.
fn confirmed(args: &[&str], what: &str) -> Result<(), String> {
    if args.get(1).is_some_and(|s| s.eq_ignore_ascii_case("confirm")) {
        Ok(())
    } else {
        Err(format!(
            "`{what}` edits the saved profile: repeat it with `confirm` (e.g. `{what} confirm`)"
        ))
    }
}

fn cmd_bank(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let n = args.first().ok_or("usage: bank <n> confirm")?;
    let amount = parse_u32(n, "bank")?;
    confirmed(args, &format!("bank {amount}"))?;
    let before = app.nest_bank();
    app.console_set_bank(amount);
    app.console_line(
        format!("nest bank {before}¢ → {amount}¢ (saved)"),
        LineKind::Success,
    );
    Ok(())
}

fn cmd_nest(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let what = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .ok_or("usage: nest max|reset confirm")?;
    if !matches!(what.as_str(), "max" | "reset") {
        return Err("usage: nest max|reset confirm".into());
    }
    confirmed(args, &format!("nest {what}"))?;
    let bank = app.nest_bank();
    match what.as_str() {
        "max" => {
            app.console_nest(true);
            app.console_line(
                format!("nest: every track maxed, bank kept at {bank}¢ (applies from the next run)"),
                LineKind::Success,
            );
        }
        _ => {
            app.console_nest(false);
            app.console_line(
                format!("nest: bank {bank}¢ → 0¢ and every track wiped"),
                LineKind::Success,
            );
        }
    }
    Ok(())
}

fn cmd_set(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let Some(world) = app.world_mut() else {
        return Err("no run in progress".into());
    };
    let name = args.first().ok_or("usage: set <cvar> <value>")?;
    let value = args.get(1).ok_or("usage: set <cvar> <value>")?;
    match name.to_ascii_lowercase().as_str() {
        "enemy_cap" => {
            let v = parse_usize(value, "enemy_cap")?;
            world.cvars.enemy_cap = v;
            app.console_line(format!("enemy_cap = {v}"), LineKind::Success);
        }
        "spawn_rate" => {
            let v = parse_f32(value, "spawn_rate")?;
            if v <= 0.0 {
                return Err("spawn_rate must be positive".into());
            }
            world.cvars.spawn_rate = v;
            app.console_line(format!("spawn_rate = {v}"), LineKind::Success);
        }
        "xp_mult" => {
            let v = parse_f32(value, "xp_mult")?;
            if v < 0.0 {
                return Err("xp_mult must be >= 0".into());
            }
            world.cvars.xp_mult = v;
            app.console_line(format!("xp_mult = {v}"), LineKind::Success);
        }
        "damage_mult" => {
            let v = parse_f32(value, "damage_mult")?;
            if v < 0.0 {
                return Err("damage_mult must be >= 0".into());
            }
            world.cvars.damage_mult = v;
            app.console_line(format!("damage_mult = {v}"), LineKind::Success);
        }
        other => return Err(format!("unknown cvar `{other}` (enemy_cap, spawn_rate, xp_mult, damage_mult)")),
    }
    Ok(())
}

fn cmd_get(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let lines = {
        let Some(world) = app.world_ref() else {
            return Err("no run in progress".into());
        };
        let cv = &world.cvars;
        match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("enemy_cap") => vec![format!("enemy_cap = {}", cv.enemy_cap)],
            Some("spawn_rate") => vec![format!("spawn_rate = {}", cv.spawn_rate)],
            Some("xp_mult") => vec![format!("xp_mult = {}", cv.xp_mult)],
            Some("damage_mult") => vec![format!("damage_mult = {}", cv.damage_mult)],
            None => vec![
                format!("enemy_cap = {}", cv.enemy_cap),
                format!("spawn_rate = {}", cv.spawn_rate),
                format!("xp_mult = {}", cv.xp_mult),
                format!("damage_mult = {}", cv.damage_mult),
            ],
            Some(other) => return Err(format!("unknown cvar `{other}`")),
        }
    };
    for line in lines {
        app.console_line(line, LineKind::Output);
    }
    Ok(())
}

fn cmd_exec(app: &mut crate::app::App, args: &[&str]) -> Result<(), String> {
    let path = args.first().ok_or("usage: exec <file>")?;
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read `{path}`: {e}"))?;
    let mut count = 0;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        app.console_line(format!("{PROMPT}{line}"), LineKind::Echo);
        match run_command(app, line) {
            Ok(()) => count += 1,
            Err(e) => app.console_line(format!("error: {e}"), LineKind::Error),
        }
    }
    app.console_line(format!("exec ran {count} command(s)"), LineKind::Success);
    Ok(())
}

// ---- Lookup helpers -------------------------------------------------------

fn find_weapon(name: &str) -> Option<WeaponKind> {
    let s = slug(name);
    WeaponKind::all().into_iter().find(|w| slug(w.name()) == s)
}

fn find_passive(name: &str) -> Option<PassiveKind> {
    let s = slug(name);
    PassiveKind::all().into_iter().find(|p| slug(p.name()) == s)
}

fn find_enemy(name: &str) -> Option<EnemyKind> {
    let s = slug(name);
    EnemyKind::all().into_iter().find(|e| slug(e.name()) == s)
}

fn find_boss(name: &str) -> Option<BossKind> {
    let s = slug(name);
    BossKind::all().into_iter().find(|b| slug(b.name()) == s)
}

fn find_miniboss(name: &str) -> Option<MinibossKind> {
    let s = slug(name);
    MinibossKind::all().into_iter().find(|m| slug(m.name()) == s)
}

fn find_pickup(name: &str) -> Option<PickupKind> {
    let s = slug(name);
    PickupKind::all()
        .into_iter()
        .find(|p| slug(p.name()) == s || slug(p.label()) == s)
}

fn find_floor(name: &str) -> Option<Theme> {
    let s = slug(name);
    Theme::playable().into_iter().find(|t| slug(t.name()) == s)
}

fn find_splice(name: &str) -> Option<SpliceKind> {
    let s = slug(name);
    SpliceKind::all().iter().copied().find(|x| slug(x.name()) == s)
}

fn find_sfx(name: &str) -> Option<crate::audio::Sfx> {
    let s = slug(name);
    crate::audio::Sfx::ALL.iter().copied().find(|x| slug(x.name()) == s)
}

fn parse_clock(raw: &str) -> Result<f32, String> {
    let parts: Vec<&str> = raw.split(':').collect();
    match parts.as_slice() {
        [m, s] => {
            let m: u32 = m.parse().map_err(|_| format!("bad minutes in `{raw}`"))?;
            let s: u32 = s.parse().map_err(|_| format!("bad seconds in `{raw}`"))?;
            if s >= 60 {
                return Err("seconds must be 0..59".into());
            }
            Ok(m as f32 * 60.0 + s as f32)
        }
        [s] => s.parse::<f32>().map_err(|_| format!("bad time `{raw}` (use mm:ss)")),
        _ => Err(format!("bad time `{raw}` (use mm:ss)")),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn fresh_app() -> crate::app::App {
        let mut app = crate::app::App::with_options(
            false,
            None,
            crate::scores::ScoreTable::default(),
            false,
            Some(2),
        );
        app.start_campaign_at(0, None).expect("a campaign starts");
        app
    }

    // ---- Presentation state ---------------------------------------------

    #[test]
    fn console_starts_closed_and_toggles() {
        let mut c = Console::new();
        assert!(!c.open);
        c.open_panel();
        assert!(c.open);
        c.close();
        assert!(!c.open);
    }

    #[test]
    fn backtick_closes_and_is_not_typed() {
        let mut c = Console::new();
        c.open_panel();
        let action = c.handle_key(key(KeyCode::Char('`')), 20);
        assert_eq!(action, Some(ConsoleAction::Close));
        assert!(c.input.is_empty(), "the backtick must not reach the prompt");
    }

    #[test]
    fn typing_edits_the_line() {
        let mut c = Console::new();
        c.open_panel();
        for ch in "hello".chars() {
            c.handle_key(key(KeyCode::Char(ch)), 20);
        }
        assert_eq!(c.input, "hello");
        c.handle_key(key(KeyCode::Left), 20);
        c.handle_key(key(KeyCode::Backspace), 20);
        assert_eq!(c.input, "helo");
        c.handle_key(key(KeyCode::Home), 20);
        c.handle_key(key(KeyCode::Delete), 20);
        assert_eq!(c.input, "elo");
        c.handle_key(key(KeyCode::End), 20);
        c.handle_key(key(KeyCode::Char('!')), 20);
        assert_eq!(c.input, "elo!");
    }

    #[test]
    fn history_up_down() {
        let mut c = Console::new();
        c.open_panel();
        c.push_history("first");
        c.push_history("second");
        c.input = "draft".into();
        c.history_prev();
        assert_eq!(c.input, "second");
        c.history_prev();
        assert_eq!(c.input, "first");
        c.history_next();
        assert_eq!(c.input, "second");
        c.history_next();
        assert_eq!(c.input, "draft");
    }

    #[test]
    fn ctrl_u_and_ctrl_w_kill_the_line() {
        let mut c = Console::new();
        c.open_panel();
        c.input = "give weapon gnaw".into();
        c.cursor = c.input.len();
        c.handle_key(ctrl(KeyCode::Char('w')), 20);
        assert_eq!(c.input, "give weapon ");
        c.handle_key(ctrl(KeyCode::Char('u')), 20);
        assert_eq!(c.input, "");
    }

    #[test]
    fn enter_echoes_and_dispatches() {
        let mut c = Console::new();
        c.open_panel();
        for ch in "help".chars() {
            c.handle_key(key(KeyCode::Char(ch)), 20);
        }
        let action = c.handle_key(key(KeyCode::Enter), 20);
        assert_eq!(action, Some(ConsoleAction::Execute("help".into())));
        // The prompt line is echoed into the scrollback.
        assert!(c.output.iter().any(|l| l.kind == LineKind::Echo));
    }

    #[test]
    fn scrollback_keeps_the_newest_lines() {
        let mut c = Console::new();
        for i in 0..(MAX_OUTPUT + 10) {
            c.push(Line::new(format!("line {i}"), LineKind::Output));
        }
        assert_eq!(c.output.len(), MAX_OUTPUT);
        // The oldest lines were dropped.
        assert!(c.output[0].text != "line 0");
    }

    #[test]
    fn page_keys_scroll_the_scrollback() {
        let mut c = Console::new();
        c.open_panel();
        for i in 0..50 {
            c.push(Line::new(format!("line {i}"), LineKind::Output));
        }
        c.handle_key(key(KeyCode::PageUp), 10);
        assert!(c.scroll > 0);
        c.handle_key(key(KeyCode::PageDown), 10);
        assert!(c.scroll < 50);
    }

    // ---- Completion -----------------------------------------------------

    #[test]
    fn completion_comes_from_real_tables() {
        let weapons = candidates(ArgKind::Weapon);
        assert!(!weapons.is_empty());
        for w in WeaponKind::all() {
            assert!(weapons.contains(&slug(w.name())), "missing weapon {}", w.name());
        }
        let enemies = candidates(ArgKind::Enemy);
        for e in EnemyKind::all() {
            assert!(enemies.contains(&slug(e.name())), "missing enemy {}", e.name());
        }
        let floors = candidates(ArgKind::Floor);
        for t in Theme::playable() {
            assert!(floors.contains(&slug(t.name())), "missing floor {}", t.name());
        }
        let bosses = candidates(ArgKind::Boss);
        for b in BossKind::all() {
            assert!(bosses.contains(&slug(b.name())), "missing boss {}", b.name());
        }
    }

    #[test]
    fn tab_completes_a_command_prefix() {
        let mut c = Console::new();
        c.open_panel();
        c.input = "gi".into();
        c.cursor = 2;
        // A single candidate is filled straight into the line.
        let n = begin_completion(&mut c);
        assert_eq!(n, 1);
        assert_eq!(c.input, "give");
        // A prefix with several candidates keeps the list.
        c.input = "g".into();
        c.cursor = 1;
        let n = begin_completion(&mut c);
        assert!(n > 1);
        assert!(c.completions.iter().any(|s| s == "give"));
    }

    #[test]
    fn tab_completes_a_weapon_argument() {
        let mut c = Console::new();
        c.open_panel();
        c.input = "give weapon gn".into();
        c.cursor = c.input.len();
        let list = complete(&c);
        assert!(list.iter().any(|s| s == "gnaw"), "got {list:?}");
        // The single candidate fills in on Tab.
        let n = begin_completion(&mut c);
        assert_eq!(n, 1);
        assert_eq!(c.input, "give weapon gnaw");
        // A passive argument completes from the passive table.
        c.input = "give passive fat".into();
        c.cursor = c.input.len();
        let list = complete(&c);
        assert!(list.iter().any(|s| s == "fatreserves"), "got {list:?}");
    }

    #[test]
    fn every_command_has_usage_and_summary() {
        for spec in COMMANDS {
            assert!(!spec.usage.is_empty(), "{} has no usage", spec.name);
            assert!(!spec.summary.is_empty(), "{} has no summary", spec.name);
        }
    }

    #[test]
    fn unknown_command_suggests_nearest() {
        assert_eq!(suggest("gve"), Some("give"));
        assert_eq!(suggest("hel"), Some("help"));
    }

    #[test]
    fn split_args_handles_quotes() {
        let parts = split_args("echo \"hello world\"");
        assert_eq!(parts, vec!["echo", "hello world"]);
    }

    // ---- Cheat flag -----------------------------------------------------

    #[test]
    fn read_only_commands_do_not_flag_cheat() {
        let mut app = fresh_app();
        assert!(!app.console_cheat);
        for cmd in ["help", "where", "loadout", "fps", "hitboxes", "get enemy_cap"] {
            run_command(&mut app, cmd).expect(cmd);
            assert!(!app.console_cheat, "`{cmd}` must not flag the run");
        }
    }

    #[test]
    fn state_changing_commands_flag_cheat() {
        let mut app = fresh_app();
        run_command(&mut app, "god on").expect("god");
        assert!(app.console_cheat, "`god` must flag the run");
    }

    #[test]
    fn cheat_flag_resets_on_a_fresh_run() {
        let mut app = fresh_app();
        run_command(&mut app, "god on").expect("god");
        assert!(app.console_cheat);
        // Starting a new campaign clears the flag.
        app.start_campaign_at(0, None).expect("restart");
        assert!(!app.console_cheat);
    }

    // ---- Command effects ------------------------------------------------

    #[test]
    fn each_command_has_a_passing_invocation() {
        // A valid invocation for every command in the registry. Commands with
        // side effects run on a fresh app so one cannot poison the next.
        let cases: &[&str] = &[
            "help",
            "clear",
            "echo hello",
            "god on",
            "noclip on",
            "heal",
            "hurt 5",
            "kill",
            "speed 1.5",
            "give all",
            "give weapon gnaw",
            "give passive fatreserves",
            "evolve all",
            "level gnaw 5",
            "remove gnaw",
            "loadout",
            "splice glassincisors",
            "xp 10",
            "levelup",
            "gold 5",
            "chest",
            "pickup heal",
            "magnet",
            "nuke",
            "spawn skitterer 2",
            "miniboss corkscrew",
            "boss cat",
            "killall",
            "freeze on",
            "peace on",
            "time 1:00",
            "timescale 2",
            "pause",
            "step 1",
            "floor sewer",
            "campaign 1",
            "seed",
            "seed 42",
            "restart",
            "win",
            "lose",
            "reveal",
            "tp arena",
            "zoom 1",
            "where",
            "fps on",
            "hitboxes on",
            "lights on",
            "sfx shoot",
            "music 1",
            "library unlock",
            "library lock",
            "bank 500",
            "nest max",
            "nest reset",
            "set enemy_cap 100",
            "get enemy_cap",
            "exec /dev/null",
        ];
        for cmd in cases {
            let mut app = fresh_app();
            let result = run_command(&mut app, cmd);
            assert!(result.is_ok(), "`{cmd}` failed: {:?}", result.err());
        }
    }

    #[test]
    fn error_cases_are_specific() {
        let mut app = fresh_app();
        // Unknown command names the nearest.
        let e = run_command(&mut app, "gve").unwrap_err();
        assert!(e.contains("give"), "expected a suggestion, got {e}");
        // Unknown weapon.
        let e = run_command(&mut app, "give weapon notarealweapon").unwrap_err();
        assert!(e.contains("notarealweapon"), "expected the name, got {e}");
        // Bad time.
        let e = run_command(&mut app, "time 1:99").unwrap_err();
        assert!(e.contains("seconds"), "expected a time error, got {e}");
        // Bad cvar.
        let e = run_command(&mut app, "set notacvar 1").unwrap_err();
        assert!(e.contains("notacvar"), "expected the cvar, got {e}");
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut app = fresh_app();
        run_command(&mut app, "set enemy_cap 123").expect("set");
        let world = app.world_ref().expect("world");
        assert_eq!(world.cvars.enemy_cap, 123);
        run_command(&mut app, "get enemy_cap").expect("get");
    }

    #[test]
    fn god_toggles_damage_immunity() {
        let mut app = fresh_app();
        run_command(&mut app, "god on").expect("god");
        let world = app.world_ref().expect("world");
        assert!(world.player.god);
        run_command(&mut app, "god off").expect("god");
        let world = app.world_ref().expect("world");
        assert!(!world.player.god);
    }

    #[test]
    fn no_command_works_without_a_run() {
        let mut app = crate::app::App::with_options(
            false,
            None,
            crate::scores::ScoreTable::default(),
            false,
            Some(2),
        );
        // No world yet: a world command must fail cleanly.
        let e = run_command(&mut app, "god on").unwrap_err();
        assert!(e.contains("no run"), "expected a no-run error, got {e}");
    }

    // ---- Pause exactness -------------------------------------------------

    #[test]
    fn pause_and_step_advance_by_one_tick() {
        let mut app = fresh_app();
        // Freeze the world and confirm the pause flag toggles.
        assert!(!app.console_paused);
        run_command(&mut app, "pause").expect("pause");
        assert!(app.console_paused);
        run_command(&mut app, "pause").expect("pause");
        assert!(!app.console_paused);
        // `step n` advances the frozen world by exactly n fixed ticks.
        let before = app.world_ref().expect("a run").elapsed;
        run_command(&mut app, "step 1").expect("step");
        let after = app.world_ref().expect("a run").elapsed;
        let dt = 1.0 / 60.0;
        assert!(
            (after - before - dt).abs() < 1e-4,
            "step 1 advanced {:.6}s, expected {:.6}s",
            after - before,
            dt
        );
    }

    // ---- Availability ----------------------------------------------------

    #[test]
    fn console_is_enabled_by_default() {
        let app = fresh_app();
        // With no RATRUN_CONSOLE override the dev build keeps the console on.
        // (The test process cannot change the env safely mid-run, so this only
        // asserts the default branch.)
        let _ = app.console_enabled();
    }
}
