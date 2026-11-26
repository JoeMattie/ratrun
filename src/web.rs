//! Browser host for the ordinary App. JavaScript owns the DOM and xterm.js.

use std::collections::VecDeque;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use wasm_bindgen::prelude::*;

use crate::app::App;
use crate::clock::Instant;
use crate::input_event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind,
};
use crate::present::FrameLoop;
use crate::{prof, term};

enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Focus(bool),
}

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum KeyPhase {
    Press,
    Repeat,
    Release,
}

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum PointerAction {
    Down,
    Up,
    Move,
    ScrollUp,
    ScrollDown,
}

#[wasm_bindgen]
pub struct WebGame {
    app: App,
    // Ratatui provides the Frame and its buffer. FrameLoop's direct writer
    // sends the diff to xterm; it never calls this backend's draw method.
    terminal: Terminal<TestBackend>,
    frames: FrameLoop,
    events: VecDeque<Event>,
    size: (u16, u16),
    last: Option<Instant>,
    audio_started: bool,
}

#[wasm_bindgen]
impl WebGame {
    #[wasm_bindgen(constructor)]
    pub fn new(cols: u16, rows: u16, webgl: bool) -> Self {
        // xterm's built-in custom glyphs cover the half and quadrant blocks.
        let mut app = {
            let blocks = if webgl { "quad" } else { "half" };
            term::init(&["--color".into(), "truecolor".into(), "--glyphs".into(), blocks.into()]);
            App::new(true, None)
        };
        app.set_lone_ctrl(true);
        Self {
            app,
            terminal: Terminal::new(TestBackend::new(cols, rows)).unwrap(),
            frames: FrameLoop::default(),
            events: VecDeque::new(),
            size: (cols, rows),
            last: None,
            audio_started: false,
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
    }

    pub fn fallback_renderer(&mut self) {
        term::init(&["--color".into(), "truecolor".into(), "--glyphs".into(), "half".into()]);
        self.frames.presenter.invalidate();
    }

    pub fn focus(&mut self, focused: bool) {
        self.events.push_back(Event::Focus(focused));
    }

    pub fn unlock_audio(&mut self) {
        if !self.audio_started {
            self.audio_started = true;
            self.app.start_web_audio();
        }
    }

    pub fn key(&mut self, key: &str, location: u8, modifiers: u8, phase: KeyPhase) {
        let modifiers = KeyModifiers::from_bits_truncate(modifiers);
        if let Some(code) = key_code(key, location, modifiers) {
            let kind = match phase {
                KeyPhase::Press => KeyEventKind::Press,
                KeyPhase::Repeat => KeyEventKind::Repeat,
                KeyPhase::Release => KeyEventKind::Release,
            };
            self.events.push_back(Event::Key(KeyEvent { code, modifiers, kind }));
        }
    }

    pub fn pointer(
        &mut self,
        action: PointerAction,
        button: i8,
        column: u16,
        row: u16,
        modifiers: u8,
    ) {
        let button = match button {
            0 => Some(MouseButton::Left),
            1 => Some(MouseButton::Middle),
            2 => Some(MouseButton::Right),
            _ => None,
        };
        let kind = match action {
            PointerAction::Move => button.map_or(MouseEventKind::Moved, MouseEventKind::Drag),
            PointerAction::Down => {
                let Some(button) = button else { return };
                MouseEventKind::Down(button)
            }
            PointerAction::Up => {
                let Some(button) = button else { return };
                MouseEventKind::Up(button)
            }
            PointerAction::ScrollUp => MouseEventKind::ScrollUp,
            PointerAction::ScrollDown => MouseEventKind::ScrollDown,
        };
        if matches!(action, PointerAction::Move)
            && matches!(self.events.back(), Some(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved | MouseEventKind::Drag(_), ..
            })))
        {
            self.events.pop_back();
        }
        self.events.push_back(Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::from_bits_truncate(modifiers),
        }));
    }

    pub fn frame(&mut self, terminal_ready: bool) -> Vec<u8> {
        self.app.input.begin_frame();
        for event in self.events.drain(..) {
            match event {
                Event::Key(key) => self.app.input.handle_key(key),
                Event::Mouse(mouse) => self.app.input.handle_mouse(mouse),
                Event::Focus(focused) => {
                    self.app.input.set_focused(focused);
                    if focused {
                        self.frames.presenter.invalidate();
                    }
                }
            }
        }
        let now = Instant::now();
        let dt = self.last.replace(now).map_or(0.0, |last| {
            now.saturating_duration_since(last).as_secs_f32().min(0.05)
        });
        if terminal_ready {
            let (cols, rows) = self.size;
            if self.terminal.get_frame().area().as_size() != (cols, rows).into() {
                self.terminal.backend_mut().resize(cols, rows);
                self.terminal.autoresize().unwrap();
                self.app.input.note_resize();
            }
        }
        self.app.update(dt);
        let mut bytes = Vec::new();
        if terminal_ready {
            self.frames.present_web(&mut self.terminal, &mut self.app, now, &mut bytes).unwrap();
        }
        bytes
    }

    pub fn quit(&self) -> bool {
        self.app.should_quit
    }

    pub fn crosshair(&self) -> bool {
        self.app.cursor_visible()
    }

    pub fn start_profile(&self, freeze_lighting: bool) {
        prof::enable(true);
        prof::freeze_controller(freeze_lighting);
    }

    pub fn profile(&self) -> String {
        let stages: Vec<_> = prof::STAGE_NAMES.iter().zip(prof::STAGES).filter_map(|(&name, &stage)| {
            let calls = prof::cnt(stage);
            let ns = prof::ns(stage);
            (ns > 0 && calls > 0).then(|| serde_json::json!([name, ns, calls]))
        }).collect();
        prof::reset();
        let world = self.app.world_ref();
        serde_json::json!({
            "screen": self.app.prof_screen(),
            "lighting": world.map(|w| format!("{:?}", w.lighting_tier())),
            "elapsed": world.map(|w| w.elapsed),
            "position": world.map(|w| [w.player.pos.x, w.player.pos.y]),
            "dashCooldown": world.map(|w| w.player.dash_cd),
            "stages": stages,
        }).to_string()
    }
}

fn key_code(key: &str, location: u8, modifiers: KeyModifiers) -> Option<KeyCode> {
    use KeyCode::*;
    use ModifierKeyCode::*;
    let right = location == 2;
    Some(match key {
        "Backspace" => Backspace,
        "Enter" => Enter,
        "ArrowLeft" => Left,
        "ArrowRight" => Right,
        "ArrowUp" => Up,
        "ArrowDown" => Down,
        "Home" => Home,
        "End" => End,
        "PageUp" => PageUp,
        "PageDown" => PageDown,
        "Tab" if modifiers.contains(KeyModifiers::SHIFT) => BackTab,
        "Tab" => Tab,
        "Delete" => Delete,
        "Insert" => Insert,
        "Escape" => Esc,
        "Control" => Modifier(if right { RightControl } else { LeftControl }),
        "Alt" => Modifier(if right { RightAlt } else { LeftAlt }),
        "Shift" => Modifier(if right { RightShift } else { LeftShift }),
        "Meta" => Modifier(Meta),
        _ => {
            let mut chars = key.chars();
            let first = chars.next()?;
            if chars.next().is_none() {
                Char(first)
            } else {
                F(key.strip_prefix('F')?.parse::<u8>().ok().filter(|n| (1..=12).contains(n))?)
            }
        }
    })
}
