//! Presenting a finished frame to the terminal.
//!
//! `io::stdout()` is a 1 KiB `LineWriter`: a 70–330 KB frame written through
//! it is one `write(2)` per kilobyte, a stdout lock per escape fragment and a
//! newline scan over every byte. [`FrameOut`] collects the whole frame in
//! memory and hands it to the terminal in one `write`.
//!
//! [`Presenter`] owns the previous frame and the diff against it, so the loop
//! knows whether a frame changed anything at all. By default it also encodes
//! the changed cells itself ([`PresentMode::Direct`]): one pass over the two
//! buffers and hand-rolled SGR bytes, instead of ratatui's `Buffer::diff`
//! (which measures every cell's display width) and crossterm's `core::fmt`
//! commands (~100 ns a cell). `RATRUN_PRESENT=ratatui` restores that path.
//! [`FramePacer`] uses the changed-cell count to
//! stop rendering screens that are standing still; [`FrameLoop`] is the two
//! together, shared by `main::run` and the `--prof-bench` harness.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::queue;
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::{Buffer, Cell, CellDiffOption, CellWidth};
use ratatui::style::{Color, Modifier};
use ratatui::{Frame, Terminal};

use crate::app::App;
use crate::prof::{self, Stage};

/// Large enough for a whole 300×90 truecolor frame without growing.
const FRAME_BUFFER_BYTES: usize = 1 << 20;

/// The frame writer the ratatui backend draws into.
///
/// Everything written is held until [`Write::flush`], which sends it in a
/// single `write_all` and flushes the terminal. Unlike `BufWriter` it **never
/// flushes on drop**: when a panic unwinds mid-frame the panic hook has
/// already restored the terminal, and half a frame of cell data spilled onto
/// the user's shell afterwards would be garbage. A dropped frame is simply
/// dropped.
pub struct FrameOut<W: Write = io::Stdout> {
    buf: Vec<u8>,
    out: W,
}

impl<W: Write> FrameOut<W> {
    pub fn new(out: W) -> Self {
        Self {
            buf: Vec::with_capacity(FRAME_BUFFER_BYTES),
            out,
        }
    }

    /// Bytes waiting for the next flush.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// The writer underneath (tests read what reached the "terminal").
    pub fn inner(&self) -> &W {
        &self.out
    }
}

impl<W: Write> Write for FrameOut<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return self.out.flush();
        }
        let written = self.out.write_all(&self.buf);
        // Never replay a frame: after a failed write the bytes are stale.
        self.buf.clear();
        written?;
        self.out.flush()
    }
}

/// Draw one frame inside a synchronized update (DEC mode 2026), so it is
/// presented atomically instead of tearing; terminals without support ignore
/// the markers. The begin marker, the cells and the end marker all go through
/// the backend's own writer, in that order. The end marker is always issued
/// and flushed, even when the draw fails, so the terminal is never left in a
/// held update.
pub fn synchronized_frame<W: Write>(
    terminal: &mut Terminal<CrosstermBackend<W>>,
    draw: impl FnOnce(&mut Frame),
) -> io::Result<()> {
    queue!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
    let drawn = terminal.draw(draw).map(|_| ());
    let ended = queue!(terminal.backend_mut(), EndSynchronizedUpdate)
        .and_then(|()| Write::flush(terminal.backend_mut()));
    drawn?;
    ended
}

/// What one presented frame cost the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presented {
    /// Cells that differed from the previous frame (and were sent).
    pub cells: usize,
}

/// Owns the previous frame and sends only what changed.
///
/// This is `Terminal::draw` taken apart: ratatui's terminal still owns the
/// frame being rendered, but the previous frame lives here, so the diff's size
/// is known (an unchanged frame sends **nothing**, not even the
/// synchronized-update markers) and a full repaint can be forced.
pub struct Presenter {
    /// What the terminal is showing.
    prev: Buffer,
    /// A blank buffer handed to ratatui for the next frame (no allocation per
    /// frame: the three buffers rotate).
    spare: Buffer,
    /// The next frame clears the screen and repaints every cell.
    full: bool,
    /// Who diffs and encodes.
    mode: PresentMode,
    /// The previous frame held a cell the direct diff does not handle (a wide
    /// glyph, a cell with a diff option): this frame goes through ratatui's
    /// diff too, which knows how to clean up after one.
    prev_exotic: bool,
    /// The direct path's frame bytes (reused).
    bytes: Vec<u8>,
    /// Frames the direct path handed to ratatui's diff (a wide glyph was on
    /// screen). Diagnostics and tests.
    pub fallbacks: u64,
}

/// Who turns a rendered frame into terminal bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    /// One pass over the two buffers and hand-rolled escape bytes.
    Direct,
    /// ratatui's `Buffer::diff` and crossterm's `Backend::draw`.
    Ratatui,
}

impl PresentMode {
    /// `RATRUN_PRESENT=ratatui` selects the ratatui/crossterm path, `direct`
    /// (or nothing) the direct one. `NO_COLOR` also selects ratatui: crossterm
    /// honours it by dropping every colour sequence, and that behaviour is
    /// kept rather than re-implemented.
    pub fn from_env() -> Self {
        Self::resolve(
            std::env::var("RATRUN_PRESENT").ok().as_deref(),
            std::env::var("NO_COLOR").ok().as_deref(),
        )
    }

    pub fn resolve(present: Option<&str>, no_color: Option<&str>) -> Self {
        match present.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("ratatui" | "crossterm" | "legacy" | "old") => PresentMode::Ratatui,
            Some("direct") => PresentMode::Direct,
            _ if no_color.is_some_and(|v| !v.is_empty()) => PresentMode::Ratatui,
            _ => PresentMode::Direct,
        }
    }
}

impl Default for Presenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Presenter {
    pub fn new() -> Self {
        Self::with_mode(PresentMode::from_env())
    }

    pub fn with_mode(mode: PresentMode) -> Self {
        Self {
            prev: Buffer::default(),
            spare: Buffer::default(),
            // Nothing is known about the screen before the first frame.
            full: true,
            mode,
            prev_exotic: false,
            bytes: Vec::new(),
            fallbacks: 0,
        }
    }

    pub fn mode(&self) -> PresentMode {
        self.mode
    }

    /// Forget what the terminal shows: the next frame clears it and repaints
    /// everything (focus regained, or anything else that may have scribbled on
    /// the screen behind our back).
    pub fn invalidate(&mut self) {
        self.full = true;
    }

    /// Does the terminal's frame no longer match what was last presented (it
    /// was resized)? Such a frame must be rendered now.
    pub fn stale_for<W: Write>(&self, terminal: &mut Terminal<CrosstermBackend<W>>) -> bool {
        self.full || terminal.get_frame().area() != self.prev.area
    }

    /// Render one frame with `render` and send what changed, inside a
    /// synchronized update (DEC mode 2026) so it is presented atomically
    /// instead of tearing; terminals without support ignore the markers.
    pub fn frame<W: Write>(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<W>>,
        render: impl FnOnce(&mut Frame),
    ) -> io::Result<Presented> {
        let area = {
            let mut frame = terminal.get_frame();
            render(&mut frame);
            frame.area()
        };
        // Take the rendered frame out of the terminal, leaving it a blank one
        // to render the next frame into.
        if self.spare.area != area {
            self.spare = Buffer::empty(area);
        }
        let cur = std::mem::replace(
            terminal.current_buffer_mut(),
            std::mem::take(&mut self.spare),
        );
        self.show(terminal.backend_mut(), cur)
    }

    /// Send a rendered frame: what changed since the last one, or everything
    /// (after a clear) when the size changed or [`Presenter::invalidate`] was
    /// called. Takes the buffer; it becomes the record of the screen.
    pub fn show<W: Write>(
        &mut self,
        backend: &mut CrosstermBackend<W>,
        cur: Buffer,
    ) -> io::Result<Presented> {
        let _p = prof::scope(Stage::Present);
        if self.prev.area != cur.area {
            self.prev = Buffer::empty(cur.area);
            self.full = true;
        }
        let full = std::mem::take(&mut self.full);
        if full {
            self.prev.reset();
        }
        let sent = match self.mode {
            PresentMode::Ratatui => self.send_ratatui(backend, &cur, full),
            PresentMode::Direct => self.send_direct(backend, &cur, full),
        };
        // Whatever happened, `cur` is the best record of the screen.
        let mut old = std::mem::replace(&mut self.prev, cur);
        if self.spare.area != old.area {
            old.reset();
            self.spare = old;
        }
        if sent.is_err() {
            // An unknown amount reached the terminal: start over next frame.
            self.full = true;
        }
        sent
    }

    fn send_ratatui<W: Write>(
        &mut self,
        backend: &mut CrosstermBackend<W>,
        cur: &Buffer,
        full: bool,
    ) -> io::Result<Presented> {
        let updates = self.prev.diff(cur);
        let cells = updates.len();
        if cells == 0 && !full {
            return Ok(Presented { cells: 0 });
        }
        queue!(backend, BeginSynchronizedUpdate)?;
        let drawn = (|| {
            if full {
                queue!(backend, Clear(ClearType::All))?;
            }
            backend.draw(updates.into_iter())
        })();
        // The end marker is always issued and flushed, even when the draw
        // fails, so the terminal is never left in a held update.
        let ended = queue!(backend, EndSynchronizedUpdate).and_then(|()| Write::flush(backend));
        drawn?;
        ended?;
        Ok(Presented { cells })
    }

    fn send_direct<W: Write>(
        &mut self,
        backend: &mut CrosstermBackend<W>,
        cur: &Buffer,
        full: bool,
    ) -> io::Result<Presented> {
        self.bytes.clear();
        self.bytes.extend_from_slice(SYNC_BEGIN);
        if full {
            self.bytes.extend_from_slice(CLEAR_ALL);
        }
        let start = self.bytes.len();
        let fast = if self.prev_exotic {
            None
        } else {
            diff_plain(&self.prev, cur, &mut self.bytes)
        };
        self.prev_exotic = fast.is_none() && has_exotic(cur);
        let cells = match fast {
            Some(cells) => cells,
            None => {
                // A wide glyph (or a cell with a diff option) is on screen or
                // was last frame: ratatui's diff knows the trailing-cell
                // rules; the encoding is still ours.
                self.bytes.truncate(start);
                self.fallbacks += 1;
                let mut enc = Encoder::new(&mut self.bytes);
                let mut cells = 0;
                for (x, y, cell) in self.prev.diff_iter(cur) {
                    enc.cell(x, y, cell);
                    cells += 1;
                }
                enc.finish();
                cells
            }
        };
        if cells == 0 && !full {
            return Ok(Presented { cells: 0 });
        }
        self.bytes.extend_from_slice(SYNC_END);
        // One buffer, begin marker to end marker: either all of it reaches
        // the frame writer or (on an error) the writer drops it.
        backend.write_all(&self.bytes)?;
        Write::flush(backend)?;
        Ok(Presented { cells })
    }
}

const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";
const CLEAR_ALL: &[u8] = b"\x1b[2J";

/// Is this symbol certainly one cell wide, without measuring it? One byte is
/// ASCII; `U+2500..=U+25FF` (box drawing, block elements — the half block the
/// play field is made of — and geometric shapes) is `E2 94..97 xx`. The
/// sub-cell blit's glyphs are four bytes: `U+1FB00..=U+1FBFF` (sextants,
/// Symbols for Legacy Computing) is `F0 9F AC..AF xx` and
/// `U+1CD00..=U+1CEBF` (octants, the Supplement) is `F0 9C B4..BA xx`.
#[inline(always)]
fn narrow_symbol(symbol: &str) -> bool {
    let b = symbol.as_bytes();
    b.len() == 1
        || (b.len() == 3 && b[0] == 0xE2 && (0x94..=0x97).contains(&b[1]))
        || (b.len() == 4
            && b[0] == 0xF0
            && ((b[1] == 0x9F && (0xAC..=0xAF).contains(&b[2]))
                || (b[1] == 0x9C && (0xB4..=0xBA).contains(&b[2]))))
}

/// A cell the direct diff handles: no diff option, at most one cell wide. For
/// such cells ratatui's `BufferDiff` reduces to "send it if it differs".
#[inline(always)]
#[allow(deprecated)] // `Cell::skip` is still honoured by ratatui's diff
fn plain_cell(cell: &Cell) -> bool {
    matches!(cell.diff_option, CellDiffOption::None)
        && !cell.skip
        && (narrow_symbol(cell.symbol()) || cell.symbol().cell_width() <= 1)
}

fn has_exotic(buf: &Buffer) -> bool {
    !buf.content.iter().all(plain_cell)
}

/// The direct diff: every cell of `cur` that differs from `prev`, encoded
/// into `out`. `None` (with `out` in an unspecified state past its length on
/// entry) when `cur` holds a cell that needs ratatui's diff rules.
fn diff_plain(prev: &Buffer, cur: &Buffer, out: &mut Vec<u8>) -> Option<usize> {
    let width = cur.area.width as usize;
    if width == 0 || prev.content.len() != cur.content.len() {
        return Some(0);
    }
    let mut enc = Encoder::new(out);
    let mut cells = 0;
    let rows = cur.content.chunks_exact(width).zip(prev.content.chunks_exact(width));
    for (row, (now, was)) in rows.enumerate() {
        let y = cur.area.y + row as u16;
        for (col, (cell, old)) in now.iter().zip(was).enumerate() {
            if !plain_cell(cell) {
                return None;
            }
            if cell != old {
                enc.cell(cur.area.x + col as u16, y, cell);
                cells += 1;
            }
        }
    }
    enc.finish();
    Some(cells)
}

/// Decimal text of every byte value, for the SGR colour parameters.
static DECIMAL: [([u8; 3], u8); 256] = {
    let mut table = [([0u8; 3], 0u8); 256];
    let mut n = 0;
    while n < 256 {
        let (h, t, o) = ((n / 100) as u8, ((n / 10) % 10) as u8, (n % 10) as u8);
        table[n] = if n >= 100 {
            ([b'0' + h, b'0' + t, b'0' + o], 3)
        } else if n >= 10 {
            ([b'0' + t, b'0' + o, 0], 2)
        } else {
            ([b'0' + o, 0, 0], 1)
        };
        n += 1;
    }
    table
};

#[inline(always)]
fn push_u8(out: &mut Vec<u8>, n: u8) {
    let (digits, len) = &DECIMAL[n as usize];
    out.extend_from_slice(&digits[..*len as usize]);
}

fn push_u16(out: &mut Vec<u8>, n: u16) {
    if n < 256 {
        return push_u8(out, n as u8);
    }
    let mut digits = [0u8; 5];
    let mut i = digits.len();
    let mut n = n;
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.extend_from_slice(&digits[i..]);
}

/// The SGR colour parameter after `38;` / `48;` / `58;`, exactly as crossterm
/// writes it for the same ratatui colour (the named colours are `5;0..=15`).
#[inline(always)]
fn push_color(out: &mut Vec<u8>, color: Color) {
    let index = match color {
        Color::Rgb(r, g, b) => {
            out.extend_from_slice(b"2;");
            push_u8(out, r);
            out.push(b';');
            push_u8(out, g);
            out.push(b';');
            push_u8(out, b);
            return;
        }
        Color::Indexed(i) => i,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        // Callers handle `Reset` (39 / 49 / 59) themselves.
        Color::Reset => 0,
    };
    out.extend_from_slice(b"5;");
    push_u8(out, index);
}

/// The SGR codes that switch each modifier on, in a fixed order.
const MODIFIER_CODES: [(Modifier, u8); 9] = [
    (Modifier::BOLD, b'1'),
    (Modifier::DIM, b'2'),
    (Modifier::ITALIC, b'3'),
    (Modifier::UNDERLINED, b'4'),
    (Modifier::SLOW_BLINK, b'5'),
    (Modifier::RAPID_BLINK, b'6'),
    (Modifier::REVERSED, b'7'),
    (Modifier::HIDDEN, b'8'),
    (Modifier::CROSSED_OUT, b'9'),
];

/// Writes cells as cursor moves, SGR changes and glyph bytes.
///
/// It tracks the terminal's pen (colours, modifiers) and where the cursor
/// lands after each glyph, so a run of adjacent cells in the same colours is
/// just their glyphs: no move, no SGR. The pen starts at the terminal's
/// default and [`Encoder::finish`] puts it back.
struct Encoder<'a> {
    out: &'a mut Vec<u8>,
    fg: Color,
    bg: Color,
    underline: Color,
    modifier: Modifier,
    /// Where the cursor is, when that is certain.
    at: Option<(u16, u16)>,
    wrote: bool,
}

impl<'a> Encoder<'a> {
    fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            fg: Color::Reset,
            bg: Color::Reset,
            underline: Color::Reset,
            modifier: Modifier::empty(),
            at: None,
            wrote: false,
        }
    }

    #[inline(always)]
    fn cell(&mut self, x: u16, y: u16, cell: &Cell) {
        self.wrote = true;
        if self.at != Some((x, y)) {
            // CSI row ; col H, one-based.
            self.out.extend_from_slice(b"\x1b[");
            push_u16(self.out, y + 1);
            self.out.push(b';');
            push_u16(self.out, x + 1);
            self.out.push(b'H');
        }
        if cell.modifier != self.modifier
            || cell.fg != self.fg
            || cell.bg != self.bg
            || cell.underline_color != self.underline
        {
            self.pen(cell);
        }
        let symbol = cell.symbol();
        self.out.extend_from_slice(symbol.as_bytes());
        // After a narrow glyph the cursor is one cell on (in the last column
        // that is "pending wrap", which no cell's position ever matches, so
        // the next cell moves explicitly). After anything else, ask again.
        self.at = narrow_symbol(symbol).then_some((x + 1, y));
    }

    /// One SGR sequence taking the pen to `cell`'s style.
    #[cold]
    #[inline(never)]
    fn pen_with_modifiers(&mut self, cell: &Cell) {
        // Modifiers changed: reset everything and set what the cell has. (The
        // terminal state is what matters, not the path to it.)
        self.out.extend_from_slice(b"\x1b[0");
        for (modifier, code) in MODIFIER_CODES {
            if cell.modifier.contains(modifier) {
                self.out.push(b';');
                self.out.push(code);
            }
        }
        self.modifier = cell.modifier;
        (self.fg, self.bg, self.underline) = (Color::Reset, Color::Reset, Color::Reset);
        if cell.fg != Color::Reset {
            self.out.extend_from_slice(b";38;");
            push_color(self.out, cell.fg);
            self.fg = cell.fg;
        }
        if cell.bg != Color::Reset {
            self.out.extend_from_slice(b";48;");
            push_color(self.out, cell.bg);
            self.bg = cell.bg;
        }
        if cell.underline_color != Color::Reset {
            self.out.extend_from_slice(b";58;");
            push_color(self.out, cell.underline_color);
            self.underline = cell.underline_color;
        }
        self.out.push(b'm');
    }

    #[inline(always)]
    fn pen(&mut self, cell: &Cell) {
        if cell.modifier != self.modifier {
            return self.pen_with_modifiers(cell);
        }
        self.out.extend_from_slice(b"\x1b[");
        let mut first = true;
        let mut sep = |out: &mut Vec<u8>| {
            if !std::mem::take(&mut first) {
                out.push(b';');
            }
        };
        if cell.fg != self.fg {
            sep(self.out);
            if cell.fg == Color::Reset {
                self.out.extend_from_slice(b"39");
            } else {
                self.out.extend_from_slice(b"38;");
                push_color(self.out, cell.fg);
            }
            self.fg = cell.fg;
        }
        if cell.bg != self.bg {
            sep(self.out);
            if cell.bg == Color::Reset {
                self.out.extend_from_slice(b"49");
            } else {
                self.out.extend_from_slice(b"48;");
                push_color(self.out, cell.bg);
            }
            self.bg = cell.bg;
        }
        if cell.underline_color != self.underline {
            sep(self.out);
            if cell.underline_color == Color::Reset {
                self.out.extend_from_slice(b"59");
            } else {
                self.out.extend_from_slice(b"58;");
                push_color(self.out, cell.underline_color);
            }
            self.underline = cell.underline_color;
        }
        self.out.push(b'm');
    }

    /// Put the pen back to the terminal's default.
    fn finish(self) {
        if self.wrote {
            self.out.extend_from_slice(b"\x1b[0m");
        }
    }
}

/// How often a screen wants to be rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demand {
    /// Every frame: the simulation is running.
    Live,
    /// A menu, plate or overlay: 30 Hz while it moves, a slow probe when it
    /// has stood still.
    Menu,
}

/// Decides, each loop iteration, whether to render and present at all.
///
/// * [`Demand::Live`] renders every frame.
/// * [`Demand::Menu`] renders at [`MENU_INTERVAL`] (30 Hz). Once
///   [`QUIET_FRAMES`] consecutive rendered frames changed **nothing on
///   screen**, the screen is treated as standing still and is only probed
///   every [`IDLE_INTERVAL`]; the first probe that changes a cell (or any
///   urgent frame) returns it to 30 Hz.
/// * An **urgent** frame — input arrived, the terminal resized, a known
///   animation clock is running — is always rendered at once.
///
/// "Standing still" is measured, not declared per screen: an animation the
/// app forgot to announce keeps the screen at 30 Hz by changing cells, and
/// one that starts on its own after a long stillness is at most one probe
/// interval late.
#[derive(Debug, Default)]
pub struct FramePacer {
    last_render: Option<Instant>,
    /// Consecutive rendered frames that changed no cell.
    quiet: u32,
}

/// The menu frame interval (30 Hz). A little under two 60 Hz loop periods, so
/// loop jitter cannot stretch it to three.
pub const MENU_INTERVAL: Duration = Duration::from_millis(30);
/// The probe interval of a screen that has stood still.
pub const IDLE_INTERVAL: Duration = Duration::from_millis(250);
/// Unchanged menu frames (1.5 s at 30 Hz) before a screen counts as still.
pub const QUIET_FRAMES: u32 = 45;

impl FramePacer {
    /// Is the screen currently treated as standing still?
    pub fn idle(&self) -> bool {
        self.quiet >= QUIET_FRAMES
    }

    pub fn should_render(&mut self, now: Instant, demand: Demand, urgent: bool) -> bool {
        if urgent {
            // Whatever asked for this frame may keep moving: back to 30 Hz.
            self.quiet = 0;
            return true;
        }
        let Some(last) = self.last_render else {
            return true;
        };
        let since = now.saturating_duration_since(last);
        match demand {
            Demand::Live => {
                self.quiet = 0;
                true
            }
            Demand::Menu if self.idle() => since >= IDLE_INTERVAL,
            Demand::Menu => since >= MENU_INTERVAL,
        }
    }

    /// Record a rendered frame and whether it changed anything on screen.
    pub fn rendered(&mut self, now: Instant, changed: bool) {
        self.last_render = Some(now);
        self.quiet = if changed {
            0
        } else {
            self.quiet.saturating_add(1)
        };
    }
}

/// When to ask the terminal for its size.
///
/// `crossterm::terminal::size` opens `/dev/tty` and issues an ioctl — every
/// frame, that was 2.6–3.5 % of the loop's busy time. The terminal reports a
/// resize as an event, so the size is asked for only then, with a slow poll
/// behind it for a terminal (or multiplexer) that loses the report.
#[derive(Debug, Default)]
pub struct SizePoll {
    last: Option<Instant>,
}

/// The fallback poll interval of [`SizePoll`].
pub const SIZE_POLL_INTERVAL: Duration = Duration::from_secs(1);

impl SizePoll {
    /// Should the size be queried now? `resized` is "a resize event arrived
    /// this frame".
    pub fn due(&mut self, now: Instant, resized: bool) -> bool {
        let due = resized
            || self
                .last
                .is_none_or(|last| now.saturating_duration_since(last) >= SIZE_POLL_INTERVAL);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// One iteration of the frame loop after `App::update`: decide whether to
/// render, render, fold the frame for this terminal, present it.
#[derive(Default)]
pub struct FrameLoop {
    pub presenter: Presenter,
    pub pacer: FramePacer,
    pub size: SizePoll,
}

impl FrameLoop {
    /// Returns `None` when the frame was skipped (nothing could have changed).
    pub fn present<W: Write>(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<W>>,
        app: &mut App,
        now: Instant,
    ) -> io::Result<Option<Presented>> {
        if app.input.refocused {
            // The terminal may have been drawn over while we were away.
            self.presenter.invalidate();
        }
        if self.size.due(now, app.input.resized) {
            // One size query; on a change ratatui resizes its buffers and
            // clears the screen (a no-op for the fixed viewports of tests and
            // the bench, which never touch a TTY).
            terminal.autoresize()?;
        }
        let urgent = app.repaint_urgent() || self.presenter.stale_for(terminal);
        if !self.pacer.should_render(now, app.frame_demand(), urgent) {
            return Ok(None);
        }
        let shown = self.presenter.frame(terminal, |f| {
            {
                let _p = prof::scope(Stage::AppRender);
                app.render(f);
            }
            // Fold the finished frame into what this terminal can draw: RGB →
            // palette when truecolor is unavailable, and risky glyphs → safe
            // ones when the terminal mis-sizes them. A no-op when the terminal
            // supports everything (the default path).
            let _p = prof::scope(Stage::Sanitize);
            crate::term::sanitize(f.buffer_mut());
        })?;
        self.pacer.rendered(now, shown.cells > 0);
        Ok(Some(shown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::{TerminalOptions, Viewport};

    /// A "terminal" that records each `write` call it receives. Clones share
    /// the record, so a test keeps one while the backend owns the other.
    #[derive(Default, Clone)]
    pub(crate) struct Recorder(pub std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>);

    impl Recorder {
        pub fn writes(&self) -> usize {
            self.0.borrow().len()
        }
        pub fn bytes(&self) -> Vec<u8> {
            self.0.borrow().concat()
        }
        pub fn clear(&self) {
            self.0.borrow_mut().clear();
        }
    }

    impl Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn fixed<W: Write>(out: W, cols: u16, rows: u16) -> Terminal<CrosstermBackend<W>> {
        Terminal::with_options(
            CrosstermBackend::new(out),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, cols, rows)),
            },
        )
        .unwrap()
    }

    fn styled(text: &'static str) -> impl FnOnce(&mut Frame) {
        move |f: &mut Frame| {
            f.buffer_mut()
                .set_string(2, 1, text, ratatui::style::Style::default());
        }
    }

    #[test]
    fn an_unchanged_frame_sends_nothing_at_all() {
        let seen = Recorder::default();
        let mut terminal = fixed(FrameOut::new(seen.clone()), 20, 4);
        let mut presenter = Presenter::new();
        let first = presenter.frame(&mut terminal, styled("RAT")).unwrap();
        assert_eq!(first.cells, 3);
        assert_eq!(seen.writes(), 1, "a frame is one write");
        let text = String::from_utf8_lossy(&seen.bytes()).into_owned();
        let begin = text.find("\x1b[?2026h").expect("begin marker");
        let cells = text.find("RAT").expect("cells");
        assert!(begin < cells && text.ends_with("\x1b[?2026l"), "{text:?}");

        seen.clear();
        let second = presenter.frame(&mut terminal, styled("RAT")).unwrap();
        assert_eq!(second.cells, 0);
        assert_eq!(seen.writes(), 0, "an identical frame must not write");

        // One changed cell sends one cell.
        let third = presenter.frame(&mut terminal, styled("RUT")).unwrap();
        assert_eq!(third.cells, 1);
        let text = String::from_utf8_lossy(&seen.bytes()).into_owned();
        assert!(text.contains('U') && !text.contains('R'), "{text:?}");

        // A cell that goes away is blanked.
        seen.clear();
        let fourth = presenter.frame(&mut terminal, styled("RU")).unwrap();
        assert_eq!(fourth.cells, 1);
    }

    #[test]
    fn invalidate_clears_and_repaints_everything() {
        let seen = Recorder::default();
        let mut terminal = fixed(FrameOut::new(seen.clone()), 20, 4);
        let mut presenter = Presenter::new();
        presenter.frame(&mut terminal, styled("RAT")).unwrap();
        seen.clear();
        presenter.invalidate();
        let again = presenter.frame(&mut terminal, styled("RAT")).unwrap();
        assert_eq!(again.cells, 3, "every non-blank cell is sent again");
        let text = String::from_utf8_lossy(&seen.bytes()).into_owned();
        let clear = text.find("\x1b[2J").expect("the screen is cleared");
        let begin = text.find("\x1b[?2026h").unwrap();
        let cells = text.find("RAT").unwrap();
        assert!(begin < clear && clear < cells, "{text:?}");
        // And only once.
        seen.clear();
        assert_eq!(presenter.frame(&mut terminal, styled("RAT")).unwrap().cells, 0);
        assert_eq!(seen.writes(), 0);
    }

    #[test]
    fn a_resize_repaints_at_the_new_size() {
        let seen = Recorder::default();
        let mut terminal = fixed(FrameOut::new(seen.clone()), 20, 4);
        let mut presenter = Presenter::new();
        presenter.frame(&mut terminal, styled("RAT")).unwrap();
        // (A fixed viewport cannot be resized without a TTY to clear, so the
        // new size is a second terminal over the same "screen".)
        let mut terminal = fixed(FrameOut::new(seen.clone()), 30, 6);
        seen.clear();
        let shown = presenter
            .frame(&mut terminal, |f| {
                assert_eq!(f.area(), Rect::new(0, 0, 30, 6));
                f.buffer_mut()
                    .set_string(26, 5, "RAT", ratatui::style::Style::default());
            })
            .unwrap();
        assert_eq!(shown.cells, 3);
        let text = String::from_utf8_lossy(&seen.bytes()).into_owned();
        assert!(text.contains("\x1b[2J") && text.contains("\x1b[6;27H"), "{text:?}");
    }

    #[test]
    fn a_failed_write_forces_a_full_repaint_next_frame() {
        struct Flaky {
            fail: std::rc::Rc<std::cell::Cell<bool>>,
            seen: Recorder,
        }
        impl Write for Flaky {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.fail.get() {
                    return Err(io::Error::other("boom"));
                }
                self.seen.write(bytes)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let seen = Recorder::default();
        let fail = std::rc::Rc::new(std::cell::Cell::new(false));
        let mut terminal = fixed(
            FrameOut::new(Flaky {
                fail: fail.clone(),
                seen: seen.clone(),
            }),
            20,
            4,
        );
        let mut presenter = Presenter::new();
        presenter.frame(&mut terminal, styled("RAT")).unwrap();
        fail.set(true);
        assert!(presenter.frame(&mut terminal, styled("CAT")).is_err());
        fail.set(false);
        seen.clear();
        let shown = presenter.frame(&mut terminal, styled("CAT")).unwrap();
        assert_eq!(shown.cells, 3, "nothing is assumed about a failed frame");
    }

    #[test]
    fn the_size_is_asked_for_on_a_resize_event_and_once_a_second() {
        let t0 = Instant::now();
        let frame = Duration::from_micros(16_667);
        let mut poll = SizePoll::default();
        assert!(poll.due(t0, false), "the first frame learns the size");
        let mut asked = 0;
        for i in 1..=600u32 {
            if poll.due(t0 + frame * i, false) {
                asked += 1;
            }
        }
        assert!((9..=10).contains(&asked), "10 s of frames asked {asked} times");
        // A resize event asks at once, and restarts the fallback clock.
        let now = t0 + frame * 601;
        assert!(poll.due(now, true));
        assert!(!poll.due(now + frame, false));
        assert!(poll.due(now + frame * 2, true), "every resize event asks");
    }

    #[test]
    fn a_frame_of_a_new_size_is_stale_until_presented() {
        let seen = Recorder::default();
        let mut terminal = fixed(FrameOut::new(seen.clone()), 20, 4);
        let mut presenter = Presenter::new();
        assert!(presenter.stale_for(&mut terminal), "nothing presented yet");
        presenter.frame(&mut terminal, styled("RAT")).unwrap();
        assert!(!presenter.stale_for(&mut terminal));
        let mut resized = fixed(FrameOut::new(seen.clone()), 30, 6);
        assert!(presenter.stale_for(&mut resized), "a new size must render now");
        presenter.frame(&mut resized, styled("RAT")).unwrap();
        assert!(!presenter.stale_for(&mut resized));
        presenter.invalidate();
        assert!(presenter.stale_for(&mut resized));
    }

    #[test]
    fn the_pacer_renders_play_every_frame_and_menus_at_thirty_hertz() {
        let t0 = Instant::now();
        let frame = Duration::from_micros(16_667);
        let mut pacer = FramePacer::default();
        // Live: every loop iteration, changed or not.
        for i in 0..120u32 {
            let now = t0 + frame * i;
            assert!(pacer.should_render(now, Demand::Live, false));
            pacer.rendered(now, false);
        }
        assert!(!pacer.idle(), "play never goes idle");

        // A moving menu: every other iteration.
        let mut pacer = FramePacer::default();
        let mut rendered = 0;
        for i in 0..120u32 {
            let now = t0 + frame * i;
            if pacer.should_render(now, Demand::Menu, false) {
                pacer.rendered(now, true);
                rendered += 1;
            }
        }
        assert_eq!(rendered, 60, "30 Hz");
    }

    #[test]
    fn a_still_menu_goes_idle_and_anything_wakes_it() {
        let t0 = Instant::now();
        let frame = Duration::from_micros(16_667);
        let mut pacer = FramePacer::default();
        let mut i = 0u32;
        let mut run = |pacer: &mut FramePacer, loops: u32, changed: bool, urgent: bool| {
            let mut rendered = 0;
            for _ in 0..loops {
                let now = t0 + frame * i;
                i += 1;
                if pacer.should_render(now, Demand::Menu, urgent) {
                    pacer.rendered(now, changed);
                    rendered += 1;
                }
            }
            rendered
        };
        // 1.5 s of unchanged frames at 30 Hz, then idle.
        assert_eq!(run(&mut pacer, 90, false, false), QUIET_FRAMES);
        assert!(pacer.idle());
        // Idle: one probe per quarter second.
        let probes = run(&mut pacer, 600, false, false);
        assert!((38..=42).contains(&probes), "10 s idle rendered {probes} frames");

        // An urgent frame is rendered on the spot and restores 30 Hz.
        assert_eq!(run(&mut pacer, 1, false, true), 1);
        assert!(!pacer.idle());
        assert_eq!(run(&mut pacer, 60, true, false), 30);

        // A probe that finds a change (an animation nobody announced) also
        // restores 30 Hz.
        assert_eq!(run(&mut pacer, 90, false, false), QUIET_FRAMES);
        assert!(pacer.idle());
        let mut woke = false;
        for _ in 0..20 {
            if run(&mut pacer, 1, true, false) == 1 {
                woke = true;
                break;
            }
        }
        assert!(woke && !pacer.idle());
        assert_eq!(run(&mut pacer, 60, true, false), 30);

        // Urgent frames are never skipped, however close together.
        assert_eq!(run(&mut pacer, 10, false, true), 10);
    }

    #[test]
    fn frame_out_holds_everything_until_the_flush() {
        let seen = Recorder::default();
        let mut out = FrameOut::new(seen.clone());
        for _ in 0..5000 {
            out.write_all(b"\x1b[38;2;1;2;3m\xe2\x96\x80").unwrap();
        }
        assert_eq!(seen.writes(), 0, "nothing may leak before the flush");
        assert_eq!(out.pending(), 5000 * 16);
        out.flush().unwrap();
        assert_eq!(seen.writes(), 1, "one frame is one write");
        assert_eq!(seen.bytes().len(), 5000 * 16);
        assert_eq!(out.pending(), 0);
        // An empty flush writes nothing.
        out.flush().unwrap();
        assert_eq!(seen.writes(), 1);
    }

    #[test]
    fn a_dropped_frame_never_reaches_the_terminal() {
        // The panic path: the hook restores the terminal, then unwinding drops
        // the writer. Half a frame must not follow the restore sequences.
        let mut sink = Vec::new();
        {
            let mut out = FrameOut::new(&mut sink);
            out.write_all(b"half a frame").unwrap();
        }
        assert!(sink.is_empty());
    }

    #[test]
    fn a_failed_write_is_not_replayed() {
        struct Broken(bool);
        impl Write for Broken {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.0 {
                    Ok(bytes.len())
                } else {
                    Err(io::Error::other("boom"))
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut out = FrameOut::new(Broken(false));
        out.write_all(b"stale").unwrap();
        assert!(out.flush().is_err());
        assert_eq!(out.pending(), 0);
    }

    #[test]
    fn synchronized_frame_wraps_the_cells_in_order() {
        let seen = Recorder::default();
        let mut terminal = fixed(FrameOut::new(seen.clone()), 20, 4);
        synchronized_frame(&mut terminal, |f| {
            f.buffer_mut().set_string(2, 1, "RAT", ratatui::style::Style::default());
        })
        .unwrap();
        let all = seen.bytes();
        let text = String::from_utf8_lossy(&all);
        let begin = text.find("\x1b[?2026h").expect("missing begin marker");
        let cells = text.find("RAT").expect("missing cells");
        let end = text.find("\x1b[?2026l").expect("missing end marker");
        assert!(begin < cells && cells < end, "order was {text:?}");
        assert!(text.ends_with("\x1b[?2026l"), "the end marker must be last");
        assert!(
            seen.writes() <= 2,
            "a frame is the cells and the end marker, not {} writes",
            seen.writes()
        );
    }

    #[test]
    fn synchronized_frame_ends_the_update_when_the_terminal_fails_mid_draw() {
        // A writer that fails once (the cell flush) and then recovers: the end
        // marker must still be sent.
        struct FailFirst {
            failed: bool,
            seen: Recorder,
        }
        impl Write for FailFirst {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if !self.failed {
                    self.failed = true;
                    return Err(io::Error::other("boom"));
                }
                self.seen.write(bytes)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let seen = Recorder::default();
        let mut terminal = fixed(
            FrameOut::new(FailFirst {
                failed: false,
                seen: seen.clone(),
            }),
            20,
            4,
        );
        let err = synchronized_frame(&mut terminal, |f| {
            f.buffer_mut().set_string(0, 0, "X", ratatui::style::Style::default());
        });
        assert_eq!(err.unwrap_err().to_string(), "boom");
        let seen = String::from_utf8_lossy(&seen.bytes()).into_owned();
        assert!(seen.contains("\x1b[?2026l"), "end marker missing: {seen:?}");
    }

    // ----------------------------------------------------------------------
    // The direct presenter against ratatui + crossterm, through a VT model.
    // ----------------------------------------------------------------------

    /// A terminal colour as the escape stream names it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ink {
        Default,
        Index(u8),
        Rgb(u8, u8, u8),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Pen {
        fg: Ink,
        bg: Ink,
        underline: Ink,
        /// Bit `n` is SGR attribute `n` (1 bold … 9 crossed out).
        attrs: u16,
    }

    const DEFAULT_PEN: Pen = Pen {
        fg: Ink::Default,
        bg: Ink::Default,
        underline: Ink::Default,
        attrs: 0,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Glyph {
        /// The cell's text; empty for the right half of a wide glyph.
        text: String,
        pen: Pen,
    }

    /// A small VT cell model: cursor addressing, SGR, clear, printing with
    /// wide glyphs. Strict where a real terminal is forgiving — printing
    /// while the cursor is past the last column (relying on autowrap) or
    /// outside the screen is a test failure, as is any sequence it does not
    /// know.
    struct Vt {
        w: usize,
        h: usize,
        cells: Vec<Glyph>,
        x: usize,
        y: usize,
        pen: Pen,
        /// Inside a synchronized update?
        held: bool,
        /// Cells printed (for the byte/cell statistics).
        printed: u64,
    }

    impl Vt {
        fn new(w: u16, h: u16) -> Self {
            let blank = Glyph {
                text: " ".into(),
                pen: DEFAULT_PEN,
            };
            Self {
                w: w as usize,
                h: h as usize,
                cells: vec![blank; w as usize * h as usize],
                x: 0,
                y: 0,
                pen: DEFAULT_PEN,
                held: false,
                printed: 0,
            }
        }

        fn blank(&mut self, i: usize) {
            self.cells[i] = Glyph {
                text: " ".into(),
                pen: self.cells[i].pen,
            };
        }

        fn print(&mut self, ch: char) {
            let mut utf8 = [0u8; 4];
            let text: &str = ch.encode_utf8(&mut utf8);
            let width = if ch.is_ascii() { 1 } else { text.cell_width() as usize };
            if width == 0 {
                // A combining mark or variation selector joins the glyph
                // before the cursor.
                assert!(self.x > 0, "a zero-width char at column 0");
                let mut i = self.y * self.w + self.x - 1;
                if self.cells[i].text.is_empty() {
                    i -= 1;
                }
                self.cells[i].text.push(ch);
                return;
            }
            assert!(
                self.y < self.h && self.x + width <= self.w,
                "printed {ch:?} at ({}, {}) on a {}x{} screen: the stream relied on autowrap",
                self.x,
                self.y,
                self.w,
                self.h
            );
            let i = self.y * self.w + self.x;
            // Overwriting half of a wide glyph erases the other half.
            if self.cells[i].text.is_empty() && self.x > 0 {
                self.blank(i - 1);
            }
            let last = i + width - 1;
            if last + 1 < (self.y + 1) * self.w && self.cells[last + 1].text.is_empty() {
                self.blank(last + 1);
            }
            self.cells[i] = Glyph {
                text: text.into(),
                pen: self.pen,
            };
            for cell in &mut self.cells[i + 1..=last] {
                *cell = Glyph {
                    text: String::new(),
                    pen: self.pen,
                };
            }
            self.x += width;
            self.printed += 1;
        }

        fn ink(params: &mut std::slice::Iter<'_, u16>) -> Ink {
            match params.next() {
                Some(5) => Ink::Index(*params.next().expect("5;n") as u8),
                Some(2) => {
                    let mut c = || *params.next().expect("2;r;g;b") as u8;
                    Ink::Rgb(c(), c(), c())
                }
                other => panic!("unknown colour form {other:?}"),
            }
        }

        fn sgr(&mut self, params: &[u16]) {
            let mut it = params.iter();
            while let Some(&p) = it.next() {
                match p {
                    0 => self.pen = DEFAULT_PEN,
                    1..=9 => self.pen.attrs |= 1 << p,
                    22 => self.pen.attrs &= !(1 << 1 | 1 << 2),
                    23 => self.pen.attrs &= !(1 << 3),
                    24 => self.pen.attrs &= !(1 << 4),
                    25 => self.pen.attrs &= !(1 << 5 | 1 << 6),
                    27 => self.pen.attrs &= !(1 << 7),
                    28 => self.pen.attrs &= !(1 << 8),
                    29 => self.pen.attrs &= !(1 << 9),
                    30..=37 => self.pen.fg = Ink::Index((p - 30) as u8),
                    38 => self.pen.fg = Self::ink(&mut it),
                    39 => self.pen.fg = Ink::Default,
                    40..=47 => self.pen.bg = Ink::Index((p - 40) as u8),
                    48 => self.pen.bg = Self::ink(&mut it),
                    49 => self.pen.bg = Ink::Default,
                    58 => self.pen.underline = Self::ink(&mut it),
                    59 => self.pen.underline = Ink::Default,
                    90..=97 => self.pen.fg = Ink::Index((p - 90 + 8) as u8),
                    100..=107 => self.pen.bg = Ink::Index((p - 100 + 8) as u8),
                    other => panic!("unknown SGR parameter {other} in {params:?}"),
                }
            }
        }

        fn feed(&mut self, bytes: &[u8]) {
            let text = std::str::from_utf8(bytes).expect("the stream is UTF-8");
            let mut chars = text.chars();
            while let Some(ch) = chars.next() {
                if ch != '\x1b' {
                    assert!(!ch.is_control(), "raw control char {ch:?} in the stream");
                    self.print(ch);
                    continue;
                }
                assert_eq!(chars.next(), Some('['), "only CSI sequences are expected");
                let mut body = String::new();
                let end = loop {
                    let c = chars.next().expect("unterminated CSI");
                    if ('\x40'..='\x7e').contains(&c) {
                        break c;
                    }
                    body.push(c);
                };
                if let Some(mode) = body.strip_prefix('?') {
                    match (mode, end) {
                        ("2026", 'h') => {
                            assert!(!self.held, "nested synchronized update");
                            self.held = true;
                        }
                        ("2026", 'l') => {
                            assert!(self.held, "end marker without a begin");
                            self.held = false;
                        }
                        other => panic!("unexpected private mode {other:?}"),
                    }
                    continue;
                }
                assert!(self.held, "cells written outside a synchronized update");
                let params: Vec<u16> = if body.is_empty() {
                    Vec::new()
                } else {
                    body.split(';')
                        .map(|p| if p.is_empty() { 0 } else { p.parse().expect("numeric parameter") })
                        .collect()
                };
                match end {
                    'm' if params.is_empty() => self.pen = DEFAULT_PEN,
                    'm' => self.sgr(&params),
                    'H' => {
                        self.y = params.first().copied().unwrap_or(1) as usize - 1;
                        self.x = params.get(1).copied().unwrap_or(1) as usize - 1;
                    }
                    'J' => {
                        assert_eq!(params, [2], "only a full clear is expected");
                        assert_eq!(self.pen, DEFAULT_PEN, "cleared with a coloured pen");
                        for cell in &mut self.cells {
                            *cell = Glyph {
                                text: " ".into(),
                                pen: DEFAULT_PEN,
                            };
                        }
                    }
                    other => panic!("unexpected CSI {body:?} {other:?}"),
                }
            }
            assert!(!self.held, "the frame left the terminal in a held update");
            assert_eq!(self.pen, DEFAULT_PEN, "the frame left a coloured pen behind");
        }

        /// What a ratatui cell should look like on this terminal.
        fn expected(cell: &Cell) -> Glyph {
            let ink = |c: Color| match c {
                Color::Reset => Ink::Default,
                Color::Rgb(r, g, b) => Ink::Rgb(r, g, b),
                Color::Indexed(i) => Ink::Index(i),
                Color::Black => Ink::Index(0),
                Color::Red => Ink::Index(1),
                Color::Green => Ink::Index(2),
                Color::Yellow => Ink::Index(3),
                Color::Blue => Ink::Index(4),
                Color::Magenta => Ink::Index(5),
                Color::Cyan => Ink::Index(6),
                Color::Gray => Ink::Index(7),
                Color::DarkGray => Ink::Index(8),
                Color::LightRed => Ink::Index(9),
                Color::LightGreen => Ink::Index(10),
                Color::LightYellow => Ink::Index(11),
                Color::LightBlue => Ink::Index(12),
                Color::LightMagenta => Ink::Index(13),
                Color::LightCyan => Ink::Index(14),
                Color::White => Ink::Index(15),
            };
            let mut attrs = 0u16;
            for (modifier, code) in MODIFIER_CODES {
                if cell.modifier.contains(modifier) {
                    attrs |= 1 << (code - b'0');
                }
            }
            Glyph {
                text: cell.symbol().into(),
                pen: Pen {
                    fg: ink(cell.fg),
                    bg: ink(cell.bg),
                    underline: ink(cell.underline_color),
                    attrs,
                },
            }
        }

        /// Assert the screen shows `buf`, cell for cell (the right half of a
        /// wide glyph is the glyph's, not the buffer's placeholder cell).
        fn assert_shows(&self, buf: &Buffer, what: &str) {
            assert_eq!((self.w, self.h), (buf.area.width as usize, buf.area.height as usize));
            for (i, cell) in buf.content.iter().enumerate() {
                if self.cells[i].text.is_empty() {
                    continue;
                }
                let want = Self::expected(cell);
                assert!(
                    self.cells[i] == want,
                    "{what}: cell ({}, {}) shows {:?}, the frame has {:?}",
                    i % self.w,
                    i / self.w,
                    self.cells[i],
                    want
                );
            }
        }
    }

    /// One presenter with its own "terminal".
    struct Rig {
        presenter: Presenter,
        backend: CrosstermBackend<FrameOut<Recorder>>,
        seen: Recorder,
        vt: Vt,
        bytes: u64,
        cells: u64,
    }

    impl Rig {
        fn new(mode: PresentMode) -> Self {
            let seen = Recorder::default();
            Self {
                presenter: Presenter::with_mode(mode),
                backend: CrosstermBackend::new(FrameOut::new(seen.clone())),
                seen,
                vt: Vt::new(0, 0),
                bytes: 0,
                cells: 0,
            }
        }

        fn show(&mut self, buf: &Buffer, what: &str) -> usize {
            if (self.vt.w, self.vt.h) != (buf.area.width as usize, buf.area.height as usize) {
                // A real resize leaves the old picture reflowed; the presenter
                // must clear it and repaint everything (the model's fresh
                // screen is the cleared one, the clear itself is asserted).
                self.vt = Vt::new(buf.area.width, buf.area.height);
                self.seen.clear();
                let shown = self.presenter.show(&mut self.backend, buf.clone()).unwrap();
                let bytes = self.seen.bytes();
                assert!(
                    String::from_utf8_lossy(&bytes).contains("\x1b[2J"),
                    "{what}: a new size must clear the screen"
                );
                self.vt.feed(&bytes);
                self.vt.assert_shows(buf, what);
                return shown.cells;
            }
            self.seen.clear();
            let shown = self.presenter.show(&mut self.backend, buf.clone()).unwrap();
            assert!(self.seen.writes() <= 1, "{what}: a frame is at most one write");
            let bytes = self.seen.bytes();
            if shown.cells == 0 {
                assert!(bytes.is_empty(), "{what}: an unchanged frame wrote {} bytes", bytes.len());
            }
            self.bytes += bytes.len() as u64;
            self.cells += shown.cells as u64;
            self.vt.feed(&bytes);
            self.vt.assert_shows(buf, what);
            shown.cells
        }
    }

    fn cell(symbol: &str, fg: Color, bg: Color, modifier: Modifier) -> Cell {
        let mut c = Cell::default();
        c.set_symbol(symbol);
        c.fg = fg;
        c.bg = bg;
        c.modifier = modifier;
        c
    }

    /// Present the same frames through both paths and compare the screens.
    fn both(frames: &[Buffer]) -> (Rig, Rig) {
        let mut direct = Rig::new(PresentMode::Direct);
        let mut ratatui = Rig::new(PresentMode::Ratatui);
        for (n, frame) in frames.iter().enumerate() {
            let a = direct.show(frame, &format!("direct, frame {n}"));
            let b = ratatui.show(frame, &format!("ratatui, frame {n}"));
            assert_eq!(a, b, "frame {n}: the two paths sent a different number of cells");
            assert!(
                direct.vt.cells == ratatui.vt.cells,
                "frame {n}: the two paths left different screens"
            );
        }
        (direct, ratatui)
    }

    #[test]
    fn the_mode_switch_reads_the_environment() {
        use PresentMode::*;
        assert_eq!(PresentMode::resolve(None, None), Direct);
        assert_eq!(PresentMode::resolve(Some("ratatui"), None), Ratatui);
        assert_eq!(PresentMode::resolve(Some(" RATATUI "), None), Ratatui);
        assert_eq!(PresentMode::resolve(Some("direct"), None), Direct);
        assert_eq!(PresentMode::resolve(Some("nonsense"), None), Direct);
        // crossterm drops colours under NO_COLOR; that path keeps the job.
        assert_eq!(PresentMode::resolve(None, Some("1")), Ratatui);
        assert_eq!(PresentMode::resolve(None, Some("")), Direct);
        assert_eq!(PresentMode::resolve(Some("direct"), Some("1")), Direct);
    }

    #[test]
    fn the_decimal_table_and_cursor_numbers_are_right() {
        for n in 0..=255u8 {
            let mut out = Vec::new();
            push_u8(&mut out, n);
            assert_eq!(out, n.to_string().into_bytes());
        }
        for n in [0u16, 9, 10, 99, 100, 255, 256, 999, 1000, 9999, 10000, 65535] {
            let mut out = Vec::new();
            push_u16(&mut out, n);
            assert_eq!(out, n.to_string().into_bytes());
        }
    }

    #[test]
    fn adjacent_cells_in_the_same_colours_share_one_move_and_one_sgr() {
        let area = Rect::new(0, 0, 12, 3);
        let mut frame = Buffer::empty(area);
        let (red, blue) = (Color::Rgb(200, 10, 10), Color::Rgb(10, 10, 200));
        for x in 2..8 {
            frame[(x, 1)] = cell("▀", red, blue, Modifier::empty());
        }
        let mut rig = Rig::new(PresentMode::Direct);
        rig.show(&Buffer::empty(area), "blank");
        rig.seen.clear();
        assert_eq!(rig.show(&frame, "run"), 6);
        let text = String::from_utf8(rig.seen.bytes()).unwrap();
        assert_eq!(
            text,
            "\x1b[?2026h\x1b[2;3H\x1b[38;2;200;10;10;48;2;10;10;200m▀▀▀▀▀▀\x1b[0m\x1b[?2026l"
        );
        // Only the colour that changed is sent, and a gap costs one move.
        let mut next = frame.clone();
        next[(4, 1)] = cell("▀", red, Color::Rgb(1, 2, 3), Modifier::empty());
        next[(9, 1)] = cell("x", Color::Reset, Color::Reset, Modifier::BOLD | Modifier::REVERSED);
        rig.seen.clear();
        assert_eq!(rig.show(&next, "two cells"), 2);
        let text = String::from_utf8(rig.seen.bytes()).unwrap();
        assert_eq!(
            text,
            "\x1b[?2026h\x1b[2;5H\x1b[38;2;200;10;10;48;2;1;2;3m▀\x1b[2;10H\x1b[0;1;7mx\x1b[0m\x1b[?2026l"
        );
    }

    #[test]
    fn hand_built_frames_look_the_same_through_both_paths() {
        let area = Rect::new(0, 0, 24, 6);
        let blank = Buffer::empty(area);
        let mut field = blank.clone();
        for y in 1..5 {
            for x in 0..24u16 {
                let v = (x * 9 + y * 31) as u8;
                field[(x, y)] = cell("▀", Color::Rgb(v, 40, 255 - v), Color::Rgb(9, v, v / 2), Modifier::empty());
            }
        }
        // HUD text with every kind of colour and modifier, including the last
        // column (where a terminal sits in "pending wrap").
        let mut hud = field.clone();
        hud.set_string(0, 0, "HP 100", ratatui::style::Style::default().fg(Color::LightRed).bg(Color::Black));
        hud[(23, 0)] = cell("!", Color::Indexed(208), Color::Reset, Modifier::BOLD);
        hud[(23, 5)] = cell("#", Color::White, Color::Indexed(17), Modifier::DIM | Modifier::ITALIC);
        hud[(22, 5)] = cell("·", Color::Yellow, Color::Reset, Modifier::UNDERLINED);
        hud[(0, 5)] = cell("¢", Color::Reset, Color::Gray, Modifier::CROSSED_OUT | Modifier::SLOW_BLINK);
        // An overlay over the field, then gone again; the same glyph in new
        // colours; a modifier dropped while the colours stay.
        let mut overlay = hud.clone();
        for x in 6..18u16 {
            overlay[(x, 2)] = cell(" ", Color::Reset, Color::Rgb(20, 20, 30), Modifier::empty());
            overlay[(x, 3)] = cell("=", Color::Cyan, Color::Rgb(20, 20, 30), Modifier::BOLD);
        }
        let mut plain = overlay.clone();
        for x in 6..18u16 {
            plain[(x, 3)].modifier = Modifier::empty();
        }
        let mut underlined = hud.clone();
        underlined[(3, 0)].underline_color = Color::Rgb(1, 2, 3);
        underlined[(3, 0)].modifier = Modifier::UNDERLINED;
        underlined[(4, 0)].underline_color = Color::Indexed(9);
        let frames = [
            blank.clone(),
            field.clone(),
            hud.clone(),
            overlay,
            plain,
            hud.clone(),
            underlined,
            hud,
            field,
            blank,
        ];
        let (direct, ratatui) = both(&frames);
        assert_eq!(direct.presenter.fallbacks, 0, "nothing here is wide");
        assert!(direct.bytes < ratatui.bytes);
    }

    #[test]
    fn wide_glyphs_take_the_ratatui_diff_and_still_match() {
        let area = Rect::new(0, 0, 16, 4);
        let style = ratatui::style::Style::default().fg(Color::Yellow).bg(Color::Blue);
        let mut field = Buffer::empty(area);
        for y in 0..4 {
            for x in 0..16u16 {
                field[(x, y)] = cell("▀", Color::Rgb(x as u8 * 9, 0, 0), Color::Rgb(0, y as u8 * 50, 0), Modifier::empty());
            }
        }
        let mut wide = field.clone();
        wide.set_string(2, 1, "鼠鼠 rat", style);
        wide.set_string(13, 3, "x鼠", style);
        let mut narrower = field.clone();
        narrower.set_string(2, 1, "a鼠b", style);
        let mut emoji = field.clone();
        emoji.set_string(4, 2, "⚠️ok", style);
        let frames = [
            field.clone(),
            wide.clone(),
            wide.clone(),
            narrower,
            emoji,
            wide,
            field.clone(),
            field,
        ];
        let (direct, _) = both(&frames);
        assert!(direct.presenter.fallbacks >= 5, "wide frames use ratatui's diff");
        assert!(direct.presenter.fallbacks <= 7, "and plain frames stop using it");
    }

    #[test]
    fn a_repaint_request_and_a_new_size_repaint_through_both_paths() {
        let small = Rect::new(0, 0, 10, 3);
        let big = Rect::new(0, 0, 14, 5);
        let paint = |area: Rect, seed: u8| {
            let mut b = Buffer::empty(area);
            for y in 0..area.height {
                for x in 1..area.width {
                    b[(x, y)] = cell("▀", Color::Rgb(seed, x as u8, y as u8), Color::Indexed(seed), Modifier::empty());
                }
            }
            b
        };
        for mode in [PresentMode::Direct, PresentMode::Ratatui] {
            let mut rig = Rig::new(mode);
            rig.show(&paint(small, 1), "first");
            assert_eq!(rig.show(&paint(small, 1), "same"), 0);
            // Focus regained: everything again, after a clear.
            rig.presenter.invalidate();
            rig.seen.clear();
            assert_eq!(rig.show(&paint(small, 1), "invalidated"), 27);
            assert!(String::from_utf8_lossy(&rig.seen.bytes()).contains("\x1b[2J"));
            // Bigger, smaller, and a blank column that must really be blank.
            assert_eq!(rig.show(&paint(big, 2), "grown"), 65);
            assert_eq!(rig.show(&paint(small, 3), "shrunk"), 27);
            assert_eq!(rig.show(&paint(small, 3), "settled"), 0);
        }
    }

    /// Play a scripted session and hand every rendered frame to `each`.
    fn scripted_session(
        detail: Option<crate::render::subcell::Choice>,
        mut each: impl FnMut(&str, &Buffer),
    ) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
        use ratatui::backend::TestBackend;

        let mut app = App::with_options(false, None, crate::scores::ScoreTable::default(), false, Some(2));
        app.set_subcell_override(detail);
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        let mut step = |app: &mut App,
                        terminal: &mut Terminal<TestBackend>,
                        what: &str,
                        keys: &[KeyCode],
                        pointer: Option<(u16, u16)>| {
            app.input.begin_frame();
            for key in keys {
                app.input.handle_key(KeyEvent::from(*key));
            }
            if let Some((column, row)) = pointer {
                app.input.handle_mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                });
            }
            app.update(1.0 / 60.0);
            terminal.draw(|f| app.render(f)).unwrap();
            each(what, terminal.backend().buffer());
        };
        let c = KeyCode::Char;
        macro_rules! run {
            ($what:expr, $frames:expr, $keys:expr) => {
                for i in 0..$frames {
                    let keys: &[KeyCode] = if i == 0 { $keys } else { &[] };
                    step(&mut app, &mut terminal, $what, keys, None);
                }
            };
        }
        // Title, the menus on the way in, the access-route card.
        run!("title", 3, &[]);
        run!("title, cursor down", 2, &[KeyCode::Down]);
        run!("title, cursor up", 2, &[KeyCode::Up]);
        run!("story", 3, &[KeyCode::Enter]);
        run!("map intro", 3, &[KeyCode::Enter]);
        run!("play begins", 2, &[KeyCode::Enter]);
        assert_eq!(app.prof_screen(), "Playing");
        // Play: steer about so the camera, the horde and the HUD all move.
        for i in 0..150 {
            let key = [c('d'), c('s'), c('a'), c('w')][(i / 20) % 4];
            step(&mut app, &mut terminal, "play", &[key], None);
        }
        // The cheese cursor crossing the field and the HUD.
        for i in 0..20u16 {
            step(&mut app, &mut terminal, "pointer", &[], Some((10 + i * 4, 2 + i)));
        }
        // Zoom out and back (the ease redraws the field under the read-out).
        run!("zoom out", 14, &[c('x')]);
        run!("zoom in", 14, &[c('z')]);
        // All the way out and back, through every rung (and, with sub-cell detail,
        // every sub-cell grid), playing at the widest view for a while.
        for _ in 0..3 {
            run!("zoom wide", 8, &[c('x')]);
        }
        for i in 0..30 {
            let key = [c('d'), c('s'), c('a'), c('w')][(i / 8) % 4];
            step(&mut app, &mut terminal, "play wide", &[key], None);
        }
        for _ in 0..3 {
            run!("zoom back", 8, &[c('z')]);
        }
        // The expanded map over the field, then closed.
        run!("map open", 6, &[KeyCode::Tab]);
        run!("map closed", 6, &[KeyCode::Tab]);
        // A level-up plate, and a card taken.
        app.world_mut().unwrap().pending_levelups = 1;
        run!("level-up", 6, &[]);
        run!("card taken", 6, &[c('1')]);
        // Pause, the options plate over it, back, resize both ways, resume.
        run!("pause", 5, &[KeyCode::Esc]);
        assert_eq!(app.prof_screen(), "Paused");
        run!("options", 4, &[c('o')]);
        run!("options, row down", 2, &[KeyCode::Down]);
        run!("back to pause", 3, &[KeyCode::Esc]);
        terminal.backend_mut().resize(132, 43);
        run!("paused, grown", 4, &[]);
        terminal.backend_mut().resize(80, 24);
        run!("paused, shrunk", 4, &[]);
        run!("resume", 30, &[c('p')]);
        assert_eq!(app.prof_screen(), "Playing");
        terminal.backend_mut().resize(100, 36);
        run!("play, resized", 30, &[c('d')]);
        // The console drops over the field (when it is compiled in).
        if app.console_enabled() {
            run!("console", 5, &[c('`')]);
            run!("console typing", 3, &[c('h'), c('e'), c('l'), c('p'), KeyCode::Enter]);
            run!("console closed", 5, &[c('`')]);
        }
        // Out to the title and into the Library.
        run!("pause again", 3, &[KeyCode::Esc]);
        run!("library from pause", 5, &[c('l')]);
        run!("library, down", 3, &[KeyCode::Down]);
        run!("leave library", 3, &[KeyCode::Esc]);
        run!("quit to title", 4, &[c('q')]);
        assert_eq!(app.prof_screen(), "Title");
    }

    #[test]
    fn a_scripted_session_looks_the_same_through_both_paths_at_every_colour_depth() {
        use crate::term::{sanitize_with, ColorDepth, GlyphSet};
        // (depth, glyph set): the default terminal, and the two fallbacks the
        // frame is folded for (`term::sanitize` runs before the presenter).
        let variants = [
            (ColorDepth::Truecolor, GlyphSet::Unicode),
            (ColorDepth::Ansi256, GlyphSet::Unicode),
            (ColorDepth::Ansi16, GlyphSet::Safe),
        ];
        let mut rigs: Vec<(Rig, Rig)> = variants
            .iter()
            .map(|_| (Rig::new(PresentMode::Direct), Rig::new(PresentMode::Ratatui)))
            .collect();
        let mut frames = 0u32;
        let mut seen = std::collections::BTreeSet::new();
        scripted_session(None, |what, frame| {
            frames += 1;
            seen.insert(what.to_string());
            for ((depth, glyphs), (direct, ratatui)) in variants.iter().zip(rigs.iter_mut()) {
                let mut folded = frame.clone();
                sanitize_with(&mut folded, *glyphs, *depth);
                let label = format!("{what} (frame {frames}, {depth:?})");
                let a = direct.show(&folded, &format!("direct: {label}"));
                let b = ratatui.show(&folded, &format!("ratatui: {label}"));
                assert_eq!(a, b, "{label}: the two paths sent a different number of cells");
                assert!(
                    direct.vt.cells == ratatui.vt.cells,
                    "{label}: the two paths left different screens"
                );
            }
        });
        assert!(frames > 300, "the script ran {frames} frames");
        for what in ["play", "pause", "level-up", "map open", "paused, shrunk", "options"] {
            assert!(seen.contains(what), "the script never reached {what:?}");
        }
        for ((depth, _), (direct, ratatui)) in variants.iter().zip(&rigs) {
            assert_eq!(direct.cells, ratatui.cells);
            assert!(direct.cells > 50_000, "{depth:?}: only {} cells changed", direct.cells);
            assert!(
                direct.bytes < ratatui.bytes,
                "{depth:?}: the direct stream ({} bytes) should be no larger than crossterm's ({})",
                direct.bytes,
                ratatui.bytes
            );
            eprintln!(
                "{depth:?}: {} cells, direct {} bytes ({} fallback frames), ratatui {} bytes",
                direct.cells, direct.bytes, direct.presenter.fallbacks, ratatui.bytes
            );
        }
    }

    /// The same session with the detail forced to octants: zoomed out, the field
    /// is quadrant, sextant and octant glyphs (four UTF-8 bytes for the last
    /// two). The direct path must take them as plain one-cell symbols — no
    /// fallback to ratatui's diff — and leave the same screen as ratatui's
    /// path at every colour depth.
    #[test]
    fn sub_cell_glyphs_go_through_the_direct_path_and_match_ratatui() {
        use crate::render::subcell::{Choice, SubcellSet};
        use crate::term::{sanitize_with, ColorDepth, GlyphSet};
        let depths = [ColorDepth::Truecolor, ColorDepth::Ansi256, ColorDepth::Ansi16];
        let mut rigs: Vec<(Rig, Rig)> = depths
            .iter()
            .map(|_| (Rig::new(PresentMode::Direct), Rig::new(PresentMode::Ratatui)))
            .collect();
        let (mut quad, mut sextant, mut octant) = (0u32, 0u32, 0u32);
        let mut frames = 0u32;
        scripted_session(Some(Choice::Force(SubcellSet::Octant)), |what, frame| {
            frames += 1;
            for cell in frame.content.iter() {
                match cell.symbol().chars().next() {
                    Some('\u{2596}'..='\u{259F}') => quad += 1,
                    Some('\u{1FB00}'..='\u{1FB3B}') => sextant += 1,
                    Some('\u{1CD00}'..='\u{1CDE5}') => octant += 1,
                    _ => {}
                }
                assert!(
                    narrow_symbol(cell.symbol()) || cell.symbol().len() <= 3,
                    "{:?} is measured every frame",
                    cell.symbol()
                );
            }
            for (depth, (direct, ratatui)) in depths.iter().zip(rigs.iter_mut()) {
                let mut folded = frame.clone();
                sanitize_with(&mut folded, GlyphSet::Unicode, *depth);
                let label = format!("{what} (frame {frames}, {depth:?})");
                let a = direct.show(&folded, &format!("direct: {label}"));
                let b = ratatui.show(&folded, &format!("ratatui: {label}"));
                assert_eq!(a, b, "{label}: the two paths sent a different number of cells");
                assert!(
                    direct.vt.cells == ratatui.vt.cells,
                    "{label}: the two paths left different screens"
                );
            }
        });
        assert!(quad > 100 && sextant > 100 && octant > 100, "{quad} / {sextant} / {octant}");
        for (depth, (direct, ratatui)) in depths.iter().zip(&rigs) {
            assert_eq!(direct.presenter.fallbacks, 0, "{depth:?}: a sub-cell glyph is not exotic");
            assert_eq!(direct.cells, ratatui.cells);
            eprintln!(
                "{depth:?} with octants: {} cells, direct {} bytes, ratatui {} bytes",
                direct.cells, direct.bytes, ratatui.bytes
            );
        }
    }
}
