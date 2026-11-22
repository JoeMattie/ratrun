//! Presenting a finished frame to the terminal.
//!
//! `io::stdout()` is a 1 KiB `LineWriter`: a 70–330 KB frame written through
//! it is one `write(2)` per kilobyte, a stdout lock per escape fragment and a
//! newline scan over every byte. [`FrameOut`] collects the whole frame in
//! memory and hands it to the terminal in one `write`.
//!
//! [`Presenter`] owns the previous frame and the diff against it, so the loop
//! knows whether a frame changed anything at all; [`FramePacer`] uses that to
//! stop rendering screens that are standing still; [`FrameLoop`] is the two
//! together, shared by `main::run` and the `--prof-bench` harness.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::queue;
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
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
}

impl Default for Presenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Presenter {
    pub fn new() -> Self {
        Self {
            prev: Buffer::default(),
            spare: Buffer::default(),
            // Nothing is known about the screen before the first frame.
            full: true,
        }
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
        let _p = prof::scope(Stage::Present);
        if self.prev.area != area {
            self.prev = Buffer::empty(area);
            self.full = true;
        }
        if self.spare.area != area {
            self.spare = Buffer::empty(area);
        }
        // Take the rendered frame out of the terminal, leaving it a blank one
        // to render the next frame into.
        let cur = std::mem::replace(
            terminal.current_buffer_mut(),
            std::mem::take(&mut self.spare),
        );
        let full = std::mem::take(&mut self.full);
        if full {
            self.prev.reset();
        }
        let sent = self.send(terminal.backend_mut(), &cur, full);
        // Whatever happened, `cur` is the best record of the screen.
        self.spare = std::mem::replace(&mut self.prev, cur);
        self.spare.reset();
        if sent.is_err() {
            // An unknown amount reached the terminal: start over next frame.
            self.full = true;
        }
        sent
    }

    fn send<W: Write>(
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
}
