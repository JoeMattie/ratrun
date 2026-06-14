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
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    self, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::App;

static KITTY_PUSHED: AtomicBool = AtomicBool::new(false);

fn main() -> Result<()> {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original(info);
    }));

    let (mut terminal, kitty) = setup_terminal()?;
    let res = run(&mut terminal, kitty);
    restore_terminal()?;
    res
}

fn setup_terminal() -> Result<(Terminal<CrosstermBackend<io::Stdout>>, bool)> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, cursor::Hide)?;

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
    if KITTY_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen, cursor::Show);
    let _ = terminal::disable_raw_mode();
    Ok(())
}

/// Spawn a thread that blocks on terminal input and forwards parsed events
/// over a channel. Exits when the receiver is dropped or reading fails.
fn input_thread() -> std::sync::mpsc::Receiver<Event> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });
    rx
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    kitty: bool,
) -> Result<()> {
    let audio = audio::AudioEngine::new();
    let mut app = App::new(kitty, audio);
    let target = Duration::from_micros(16_667); // ~60 FPS
    let mut last = Instant::now();

    // Read input on a dedicated thread. The terminal's mouse "report all
    // motion" (mode 1003) can stream events faster than a frame; doing the
    // blocking reads + escape-sequence parsing here means the main loop only
    // drains an already-parsed channel and can never be stalled by the input
    // stream. The main loop keeps rendering even under a flood.
    let rx = input_thread();

    while !app.should_quit {
        let frame_start = Instant::now();

        app.input.begin_frame();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::Key(k) => {
                    if k.kind == KeyEventKind::Press
                        && k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.should_quit = true;
                    }
                    app.input.handle_key(k);
                }
                Event::Mouse(m) => app.input.handle_mouse(m),
                _ => {}
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.05);
        last = now;

        app.update(dt);
        terminal.draw(|f| app.render(f))?;

        let elapsed = frame_start.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    Ok(())
}
