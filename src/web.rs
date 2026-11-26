//! Browser host for the ordinary App. JavaScript paints the cell grid.

use std::collections::VecDeque;

use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell, CellWidth};
use ratatui::style::Color;
use ratatui::Terminal;
use wasm_bindgen::prelude::*;

use crate::app::App;
use crate::clock::Instant;
use crate::input_event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind,
};
use crate::present::FrameLoop;
use crate::render::subcell;
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
    // Ratatui provides the Frame and its buffer. Presentation never calls
    // this backend's draw method.
    terminal: Terminal<TestBackend>,
    frames: FrameLoop,
    canvas: CanvasFrame,
    events: VecDeque<Event>,
    size: (u16, u16),
    last: Option<Instant>,
    audio_started: bool,
}

#[wasm_bindgen]
impl WebGame {
    #[wasm_bindgen(constructor)]
    pub fn new(cols: u16, rows: u16) -> Self {
        let mut app = {
            term::init(&["--color".into(), "truecolor".into(), "--glyphs".into(), "octant".into()]);
            App::new(true, None)
        };
        app.set_lone_ctrl(true);
        Self {
            app,
            terminal: Terminal::new(TestBackend::new(cols, rows)).unwrap(),
            frames: FrameLoop::default(),
            canvas: CanvasFrame::default(),
            events: VecDeque::new(),
            size: (cols, rows),
            last: None,
            audio_started: false,
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
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

    /// Advance the game and return the number of packed words ready to paint.
    /// The views obtained through the pointer exports are valid until the next
    /// frame, and JS reacquires the memory buffer after every Wasm call.
    pub fn frame(&mut self) -> usize {
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
        let (cols, rows) = self.size;
        if self.terminal.get_frame().area().as_size() != (cols, rows).into() {
            self.terminal.backend_mut().resize(cols, rows);
            self.terminal.autoresize().unwrap();
            self.app.input.note_resize();
        }
        {
            let _p = prof::scope(prof::Stage::AppUpdate);
            self.app.update(dt);
        }
        let canvas = &mut self.canvas;
        canvas.words.clear();
        canvas.text.clear();
        self.frames.present_web(&mut self.terminal, &mut self.app, now, |prev, next, full| {
            canvas.pack(prev, next, full)
        });
        canvas.words.len()
    }

    pub fn cells_ptr(&self) -> *const u32 {
        self.canvas.words.as_ptr()
    }

    pub fn text_ptr(&self) -> *const u8 {
        self.canvas.text.as_ptr()
    }

    pub fn quit(&self) -> bool {
        self.app.should_quit
    }

    pub fn crosshair(&self) -> bool {
        self.app.cursor_visible()
    }

    /// The last rendered buffer before packing, for browser frame comparison.
    pub fn debug_frame(&self) -> String {
        let buffer = self.frames.presenter.last_buffer();
        serde_json::json!({
            "cols": buffer.area.width,
            "rows": buffer.area.height,
            "screen": self.app.prof_screen(),
            "cells": buffer.content.iter().map(|cell| (
                cell.symbol(),
                format!("{:?}", cell.fg),
                format!("{:?}", cell.bg),
                cell.modifier.bits(),
            )).collect::<Vec<_>>(),
        }).to_string()
    }

    pub fn start_profile(&self, _freeze_lighting: bool) {
        prof::enable(true);
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

/// Sent once at startup. Each triple is a code point and its 64-bit mask
/// (least significant bit at the top left).
#[wasm_bindgen]
pub fn block_masks() -> Vec<u32> {
    subcell::canvas_masks()
        .flat_map(|(glyph, mask)| [glyph as u32, mask as u32, (mask >> 32) as u32])
        .collect()
}

#[derive(Default)]
struct CanvasFrame {
    words: Vec<u32>,
    text: Vec<u8>,
}

impl CanvasFrame {
    fn pack(&mut self, prev: &Buffer, next: &Buffer, full: bool) -> usize {
        for (i, (old, cell)) in prev.content.iter().zip(&next.content).enumerate() {
            if full || old != cell {
                self.push(i, cell);
            }
        }
        self.words.len() / 5
    }

    // Five u32s per change: index, fg, bg, symbol, style/width. A scalar is
    // inline. A grapheme with multiple scalars uses the high bit and an
    // offset into the adjacent UTF-8 stream.
    fn push(&mut self, index: usize, cell: &Cell) {
        let symbol = cell.symbol();
        let first = symbol.chars().next().unwrap_or(' ');
        let compound = first.len_utf8() < symbol.len();
        let code = if compound {
            let offset = self.text.len();
            self.text.extend_from_slice(symbol.as_bytes());
            0x8000_0000 | offset as u32
        } else {
            first as u32
        };
        // All game blocks (including the supplementary planes) occupy one
        // cell. Measure other non-ASCII text using ratatui's width rules.
        let width = if crate::present::narrow_symbol(symbol) { 1 } else { cell.cell_width() };
        let style = u32::from(cell.modifier.bits()) | (u32::from(width.clamp(1, 255)) << 9)
            | ((if compound { symbol.len() } else { 0 } as u32) << 17);
        self.words.extend_from_slice(&[
            index as u32,
            rgb(cell.fg, 0xe1d8cc),
            rgb(cell.bg, 0x090a0d),
            code,
            style,
        ]);
    }
}

fn rgb(color: Color, default: u32) -> u32 {
    const ANSI: [u32; 16] = [
        0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf,
        0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
    ];
    const CUBE: [u32; 6] = [0, 95, 135, 175, 215, 255];
    match color {
        Color::Reset => default,
        Color::Rgb(r, g, b) => u32::from_be_bytes([0, r, g, b]),
        Color::Black => ANSI[0],
        Color::Red => ANSI[1],
        Color::Green => ANSI[2],
        Color::Yellow => ANSI[3],
        Color::Blue => ANSI[4],
        Color::Magenta => ANSI[5],
        Color::Cyan => ANSI[6],
        Color::Gray => ANSI[7],
        Color::DarkGray => ANSI[8],
        Color::LightRed => ANSI[9],
        Color::LightGreen => ANSI[10],
        Color::LightYellow => ANSI[11],
        Color::LightBlue => ANSI[12],
        Color::LightMagenta => ANSI[13],
        Color::LightCyan => ANSI[14],
        Color::White => ANSI[15],
        Color::Indexed(i @ 0..=15) => ANSI[usize::from(i)],
        Color::Indexed(i @ 16..=231) => {
            let i = usize::from(i - 16);
            CUBE[i / 36] << 16 | CUBE[i / 6 % 6] << 8 | CUBE[i % 6]
        }
        Color::Indexed(i) => u32::from(8 + (i - 232) * 10) * 0x010101,
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
