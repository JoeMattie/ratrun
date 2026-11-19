//! Rat Run — a terminal bullet-hell horde survivor built with ratatui.

// A few small geometry/helper methods are kept as deliberate building-block
// API even when not every one is wired up yet.
#![allow(dead_code)]

mod app;
mod audio;
mod config;
mod input;
mod lore;
mod math;
mod render;
mod scores;
mod titleart;

mod game;

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor;
use crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    self, BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::App;
use input::InputState;

static KITTY_PUSHED: AtomicBool = AtomicBool::new(false);

/// Restores the terminal on drop so that *every* exit path after raw mode is
/// enabled — including a `?` error inside setup — leaves the user's terminal
/// usable. Restoration itself is idempotent.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn main() -> Result<()> {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original(info);
    }));

    // Raw mode is the first mutation of terminal state, so the guard is armed
    // as soon as it succeeds; everything from here on restores on the way out.
    terminal::enable_raw_mode()?;
    let _guard = TerminalGuard;

    let (mut terminal, kitty) = setup_terminal()?;
    run(&mut terminal, kitty)
}

/// Set up the alternate screen, mouse + focus reporting and the Kitty keyboard
/// protocol flags (Ghostty, Kitty and others report them).
/// Raw mode is enabled by `main` (which owns the restore guard).
fn setup_terminal() -> Result<(Terminal<CrosstermBackend<io::Stdout>>, bool)> {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        // Focus reporting tells us when a held key's Release event will never
        // arrive (the terminal lost focus), so movement cannot stick.
        EnableFocusChange,
        cursor::Hide
    )?;

    let kitty = terminal::supports_keyboard_enhancement().unwrap_or(false);
    if kitty {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        )?;
        KITTY_PUSHED.store(true, Ordering::SeqCst);
    }

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok((terminal, kitty))
}

fn restore_terminal() -> Result<()> {
    let mut stdout = io::stdout();
    restore_to(&mut stdout)
}

/// Perform every restore step even if an earlier one fails; safe to call twice
/// and on a terminal that was never fully set up. Each step is issued on its
/// own so one failure cannot skip the rest.
fn restore_to(out: &mut impl io::Write) -> Result<()> {
    // Close a synchronized update first: leaving the terminal in a held update
    // would freeze the screen.
    let _ = execute!(out, EndSynchronizedUpdate);
    if KITTY_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableFocusChange);
    let _ = execute!(out, DisableMouseCapture);
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = execute!(out, cursor::Show);
    let _ = terminal::disable_raw_mode();
    Ok(())
}

/// How long the reader thread waits for input before re-checking its stop flag.
/// Short enough that `run` can join it promptly on exit.
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The reader loop, factored out so it can be tested without a TTY.
///
/// Polls with a short timeout (so the stop flag is noticed promptly without
/// blocking the join), reads and forwards events, and stops on the stop flag, a
/// poll/read error, or a dropped receiver. `poll` returns as soon as an event
/// is ready, so input latency is unchanged.
fn reader_loop(
    stop: &AtomicBool,
    mut poll: impl FnMut(Duration) -> io::Result<bool>,
    mut read: impl FnMut() -> io::Result<Event>,
    mut send: impl FnMut(io::Result<Event>) -> bool,
) {
    while !stop.load(Ordering::SeqCst) {
        match poll(INPUT_POLL_INTERVAL) {
            Ok(true) => match read() {
                Ok(ev) => {
                    if !send(Ok(ev)) {
                        break;
                    }
                }
                Err(e) => {
                    let _ = send(Err(e));
                    break;
                }
            },
            Ok(false) => {}
            Err(e) => {
                let _ = send(Err(e));
                break;
            }
        }
    }
}

/// The input reader thread plus the flag that stops it. Dropping this sets the
/// flag and joins the thread, so the terminal is never restored while a read is
/// still in flight (joining waits at most one poll interval).
struct InputThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl InputThread {
    /// Spawn the reader. The receiver yields parsed events and, if the reader
    /// fails, the error that ended it.
    fn spawn() -> (InputThread, mpsc::Receiver<io::Result<Event>>) {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                reader_loop(&stop, event::poll, event::read, |ev| tx.send(ev).is_ok());
            })
        };
        (
            InputThread {
                stop,
                handle: Some(handle),
            },
            rx,
        )
    }
}

impl Drop for InputThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Run `draw` inside a synchronized update (DEC mode 2026), so the frame is
/// presented atomically instead of tearing. Terminals without support ignore
/// the sequences. The end sequence is always issued, even when the draw fails,
/// so the terminal is never left in a held update.
fn with_synchronized_update<T>(
    out: &mut impl io::Write,
    draw: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    execute!(out, BeginSynchronizedUpdate)?;
    let drawn = draw();
    let ended = execute!(out, EndSynchronizedUpdate);
    let value = drawn?;
    ended?;
    Ok(value)
}

/// Events handled per frame. A mouse-motion flood (mode 1003) can put far more
/// than a frame's worth into the channel; the rest wait for the next frame so a
/// single frame can never be monopolised by input.
const MAX_EVENTS_PER_FRAME: usize = 512;

fn is_mouse_motion(kind: MouseEventKind) -> bool {
    matches!(kind, MouseEventKind::Moved | MouseEventKind::Drag(_))
}

/// Drain up to [`MAX_EVENTS_PER_FRAME`] parsed events into `input`.
///
/// Runs of mouse motion are coalesced (only the newest position in a run
/// matters) and a pending move is always flushed *before* the next discrete
/// event, so key presses are never dropped, reordered or hidden behind motion.
/// `Ok(true)` means the user asked to quit (Ctrl-C); `Err` means the input
/// reader died and the caller must end the run so the terminal is restored.
fn drain_input(rx: &mpsc::Receiver<io::Result<Event>>, input: &mut InputState) -> io::Result<bool> {
    let mut quit = false;
    let mut pending_move: Option<MouseEvent> = None;
    let mut drained = 0usize;

    while drained < MAX_EVENTS_PER_FRAME {
        match rx.try_recv() {
            Ok(Ok(Event::Mouse(m))) if is_mouse_motion(m.kind) => {
                drained += 1;
                // Coalesce: the previous motion in this run is stale by now.
                pending_move = Some(m);
            }
            Ok(Ok(ev)) => {
                drained += 1;
                if let Some(m) = pending_move.take() {
                    input.handle_mouse(m);
                }
                match ev {
                    Event::Key(k) => {
                        if k.kind == KeyEventKind::Press
                            && k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            quit = true;
                        }
                        input.handle_key(k);
                    }
                    Event::Mouse(m) => input.handle_mouse(m),
                    // A key held when focus is lost never reports Release.
                    Event::FocusLost => input.release_all(),
                    _ => {}
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "terminal input reader stopped",
                ))
            }
        }
    }
    if let Some(m) = pending_move.take() {
        input.handle_mouse(m);
    }
    Ok(quit)
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, kitty: bool) -> Result<()> {
    let audio = audio::AudioEngine::new();
    let mut app = App::new(kitty, audio);
    let target = Duration::from_micros(16_667); // ~60 FPS
    let mut last = Instant::now();

    // Read input on a dedicated thread. The terminal's mouse "report all
    // motion" (mode 1003) can stream events faster than a frame; doing the
    // blocking reads + escape-sequence parsing here means the main loop only
    // drains an already-parsed channel and can never be stalled by the input
    // stream. The main loop keeps rendering even under a flood. Dropping the
    // guard stops and joins the thread, so the terminal is only restored after
    // the reader has finished.
    let (_input, rx) = InputThread::spawn();

    while !app.should_quit {
        let frame_start = Instant::now();

        app.input.begin_frame();
        // A dead input reader (or a read error) ends the run: without input the
        // game cannot even be quit from the keyboard. `?` unwinds through the
        // restore guard in `main`.
        if drain_input(&rx, &mut app.input)? {
            app.should_quit = true;
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.05);
        last = now;

        app.update(dt);
        // Present the frame atomically: a half-block frame redrawn cell by cell
        // would otherwise tear mid-frame on a fast terminal.
        let mut out = io::stdout();
        with_synchronized_update(&mut out, || terminal.draw(|f| app.render(f)))?;

        let elapsed = frame_start.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    type Tx = mpsc::Sender<io::Result<Event>>;
    type Rx = mpsc::Receiver<io::Result<Event>>;

    fn motion(col: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn reader() -> (Tx, Rx) {
        mpsc::channel()
    }

    #[test]
    fn drain_caps_events_per_frame_and_keeps_late_keys() {
        let (tx, rx) = reader();
        for col in 0..MAX_EVENTS_PER_FRAME as u16 {
            tx.send(Ok(motion(col, 0))).unwrap();
        }
        tx.send(Ok(press(KeyCode::Char('w')))).unwrap();

        let mut input = InputState::new(false);
        assert!(!drain_input(&rx, &mut input).unwrap());
        // Only this frame's budget was consumed; motion collapsed to the newest.
        assert_eq!(
            input.mouse_pos,
            Some(((MAX_EVENTS_PER_FRAME - 1) as u16, 0))
        );
        assert!(
            !input.pressed_char('w'),
            "a key beyond the cap must stay queued, not be dropped"
        );

        assert!(!drain_input(&rx, &mut input).unwrap());
        assert!(input.pressed_char('w'), "queued key lost across frames");
    }

    #[test]
    fn motion_before_a_key_does_not_hide_or_reorder_it() {
        let (tx, rx) = reader();
        tx.send(Ok(motion(3, 4))).unwrap();
        tx.send(Ok(motion(5, 6))).unwrap();
        tx.send(Ok(press(KeyCode::Char('w')))).unwrap();

        let mut input = InputState::new(false);
        drain_input(&rx, &mut input).unwrap();
        // The coalesced move is applied before the key: the newest position is
        // kept and the key still takes control back from the mouse.
        assert_eq!(input.mouse_pos, Some((5, 6)));
        assert!(!input.prefer_mouse);
        assert!(input.pressed_char('w'));
    }

    #[test]
    fn discrete_events_are_never_coalesced_away() {
        let (tx, rx) = reader();
        for col in 0..50 {
            tx.send(Ok(motion(col, 0))).unwrap();
        }
        tx.send(Ok(press(KeyCode::Char('a')))).unwrap();
        for col in 0..50 {
            tx.send(Ok(motion(col, 1))).unwrap();
        }
        tx.send(Ok(press(KeyCode::Char('d')))).unwrap();
        tx.send(Ok(Event::Resize(80, 24))).unwrap();

        let mut input = InputState::new(false);
        drain_input(&rx, &mut input).unwrap();
        assert_eq!(input.pressed.len(), 2);
        assert!(input.pressed_char('a') && input.pressed_char('d'));
        assert_eq!(input.mouse_pos, Some((49, 1)));
    }

    #[test]
    fn focus_loss_releases_held_keys() {
        let (tx, rx) = reader();
        // Kitty protocol: a press holds until an explicit release, which never
        // arrives when the terminal loses focus while the key is down.
        let mut input = InputState::new(true);
        tx.send(Ok(press(KeyCode::Char('w')))).unwrap();
        tx.send(Ok(Event::FocusLost)).unwrap();
        drain_input(&rx, &mut input).unwrap();
        assert_eq!(input.move_dir(), crate::math::Vec2::ZERO);
    }

    #[test]
    fn ctrl_c_reports_quit() {
        let (tx, rx) = reader();
        tx.send(Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))))
        .unwrap();
        let mut input = InputState::new(false);
        assert!(drain_input(&rx, &mut input).unwrap());
    }

    #[test]
    fn reader_errors_end_the_run() {
        let (tx, rx) = reader();
        tx.send(Ok(motion(1, 1))).unwrap();
        tx.send(Err(io::Error::other("boom"))).unwrap();
        let mut input = InputState::new(false);
        let err = drain_input(&rx, &mut input).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn a_stopped_reader_is_not_silent() {
        let (tx, rx) = reader();
        drop(tx);
        let mut input = InputState::new(false);
        let err = drain_input(&rx, &mut input).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn restore_is_idempotent_and_clears_the_kitty_flag() {
        let mut first = Vec::new();
        KITTY_PUSHED.store(true, Ordering::SeqCst);
        restore_to(&mut first).unwrap();
        assert!(!KITTY_PUSHED.load(Ordering::SeqCst));
        assert!(!first.is_empty(), "restore must write the reset sequences");
        // Restore must also close a synchronized update, or the screen freezes.
        let first = String::from_utf8_lossy(&first);
        assert!(
            first.contains("\x1b[?2026l"),
            "restore must end the synchronized update: {first:?}"
        );

        // A second (or unpaired) restore is harmless and still runs every step.
        let mut second = Vec::new();
        restore_to(&mut second).unwrap();
        assert!(!second.is_empty());
        assert!(!KITTY_PUSHED.load(Ordering::SeqCst));
    }

    #[test]
    fn synchronized_update_wraps_the_draw() {
        let mut out = Vec::new();
        let value = with_synchronized_update(&mut out, || Ok::<_, io::Error>(42u8)).unwrap();
        assert_eq!(value, 42);
        let text = String::from_utf8_lossy(&out);
        let begin = text.find("\x1b[?2026h").expect("missing begin sequence");
        let end = text.find("\x1b[?2026l").expect("missing end sequence");
        assert!(begin < end, "end must follow begin: {text:?}");
    }

    #[test]
    fn synchronized_update_ends_on_the_error_path() {
        let mut out = Vec::new();
        let err = with_synchronized_update(&mut out, || Err::<(), _>(io::Error::other("boom")));
        assert_eq!(err.unwrap_err().to_string(), "boom");
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("\x1b[?2026l"),
            "end sequence missing after a failed draw: {text:?}"
        );
    }

    #[test]
    fn reader_loop_stops_when_the_flag_is_already_set() {
        let stop = AtomicBool::new(true);
        let mut polls = 0;
        reader_loop(
            &stop,
            |_| {
                polls += 1;
                Ok(false)
            },
            || Ok(press(KeyCode::Char('w'))),
            |_| true,
        );
        assert_eq!(polls, 0, "a stopped reader must not poll at all");
    }

    #[test]
    fn reader_loop_notices_the_stop_flag_between_polls() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut polls = 0;
        let mut sent = 0;
        {
            let stop = Arc::clone(&stop);
            reader_loop(
                &stop,
                |_| {
                    polls += 1;
                    // Simulate run() returning: the flag flips mid-loop.
                    stop.store(true, Ordering::SeqCst);
                    Ok(false)
                },
                || Ok(press(KeyCode::Char('w'))),
                |ev| {
                    sent += 1;
                    ev.is_ok()
                },
            );
        }
        assert_eq!(polls, 1, "the loop must stop after the flag is set");
        assert_eq!(sent, 0);
    }

    #[test]
    fn reader_loop_forwards_events_in_order() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut codes = Vec::new();
        {
            let stop = Arc::clone(&stop);
            reader_loop(
                &stop,
                |_| Ok(true),
                || Ok(press(KeyCode::Char('a'))),
                |ev| {
                    if let Ok(Event::Key(k)) = ev {
                        codes.push(k.code);
                    }
                    stop.store(true, Ordering::SeqCst);
                    true
                },
            );
        }
        assert_eq!(codes, vec![KeyCode::Char('a')]);
    }

    #[test]
    fn reader_loop_reports_read_errors_and_stops() {
        let stop = AtomicBool::new(false);
        let mut polls = 0;
        let mut errors = 0;
        reader_loop(
            &stop,
            |_| {
                polls += 1;
                Ok(true)
            },
            || Err(io::Error::other("boom")),
            |ev| {
                if let Err(e) = ev {
                    errors += 1;
                    assert_eq!(e.to_string(), "boom");
                }
                true
            },
        );
        assert_eq!(polls, 1);
        assert_eq!(errors, 1, "the read error must reach the receiver");
    }

    #[test]
    fn reader_loop_reports_poll_errors_and_stops() {
        let stop = AtomicBool::new(false);
        let mut errors = 0;
        reader_loop(
            &stop,
            |_| Err(io::Error::other("poll boom")),
            || Ok(press(KeyCode::Char('a'))),
            |ev| {
                if ev.is_err() {
                    errors += 1;
                }
                true
            },
        );
        assert_eq!(errors, 1);
    }

    #[test]
    fn reader_loop_stops_when_the_receiver_is_gone() {
        let stop = AtomicBool::new(false);
        let mut sent = 0;
        reader_loop(
            &stop,
            |_| Ok(true),
            || Ok(press(KeyCode::Char('a'))),
            |_| {
                sent += 1;
                false // the receiver was dropped
            },
        );
        assert_eq!(sent, 1);
    }

    #[test]
    fn input_thread_guard_stops_and_joins_on_drop() {
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = Arc::clone(&stop);
            let finished = Arc::clone(&finished);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                finished.store(true, Ordering::SeqCst);
            })
        };
        let guard = InputThread {
            stop: Arc::clone(&stop),
            handle: Some(handle),
        };
        drop(guard);
        assert!(stop.load(Ordering::SeqCst), "drop must set the stop flag");
        assert!(
            finished.load(Ordering::SeqCst),
            "drop must join the reader thread"
        );
    }
}
