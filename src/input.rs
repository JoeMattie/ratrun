//! Input handling. Movement keys need to be treated as "held", but terminals
//! only reliably report key *presses* (plus auto-repeat). When the Kitty
//! keyboard protocol is available we get release events too, so a key stays
//! down until released. Otherwise we fall back to a short grace window that
//! auto-repeat keeps refreshing.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::math::Vec2;

const GRACE: f32 = 0.13;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

pub struct InputState {
    grace: [f32; 4],
    pub kitty: bool,
    /// Discrete key presses captured this frame (menus, pause, dash, quit).
    pub pressed: Vec<KeyEvent>,
}

impl InputState {
    pub fn new(kitty: bool) -> Self {
        InputState {
            grace: [0.0; 4],
            kitty,
            pressed: Vec::new(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.pressed.clear();
    }

    /// Decay held-key grace timers (no-op when kitty release events are used).
    pub fn tick(&mut self, dt: f32) {
        if !self.kitty {
            for g in self.grace.iter_mut() {
                *g = (*g - dt).max(0.0);
            }
        }
    }

    pub fn handle_key(&mut self, ev: KeyEvent) {
        let dir = match ev.code {
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') => Some(Dir::Up),
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => Some(Dir::Down),
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => Some(Dir::Left),
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => Some(Dir::Right),
            _ => None,
        };

        match ev.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => {
                if let Some(d) = dir {
                    let v = if self.kitty { f32::INFINITY } else { GRACE };
                    self.grace[d as usize] = v;
                }
                if ev.kind == KeyEventKind::Press {
                    self.pressed.push(ev);
                }
            }
            KeyEventKind::Release => {
                if let Some(d) = dir {
                    self.grace[d as usize] = 0.0;
                }
            }
        }
    }

    fn down(&self, d: Dir) -> bool {
        self.grace[d as usize] > 0.0
    }

    pub fn move_dir(&self) -> Vec2 {
        let mut v = Vec2::ZERO;
        if self.down(Dir::Up) {
            v.y -= 1.0;
        }
        if self.down(Dir::Down) {
            v.y += 1.0;
        }
        if self.down(Dir::Left) {
            v.x -= 1.0;
        }
        if self.down(Dir::Right) {
            v.x += 1.0;
        }
        v.normalized()
    }

    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.pressed.iter().any(|e| e.code == code)
    }

    pub fn pressed_char(&self, c: char) -> bool {
        self.pressed.iter().any(|e| {
            matches!(e.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
        })
    }
}
