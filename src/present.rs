//! Presenting a finished frame to the terminal.
//!
//! `io::stdout()` is a 1 KiB `LineWriter`: a 70–330 KB frame written through
//! it is one `write(2)` per kilobyte, a stdout lock per escape fragment and a
//! newline scan over every byte. [`FrameOut`] collects the whole frame in
//! memory and hands it to the terminal in one `write`.

use std::io::{self, Write};

use crossterm::queue;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};

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
