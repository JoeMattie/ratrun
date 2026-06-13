//! Input handling. Movement keys need to be treated as "held", but terminals
//! only reliably report key *presses* (plus auto-repeat). When the Kitty
//! keyboard protocol is available we get release events too, so a key stays
//! down until released. Otherwise we fall back to a short grace window that
//! auto-repeat keeps refreshing.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};

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
    /// Latest mouse cursor cell (column, row), persisted across frames.
    pub mouse_pos: Option<(u16, u16)>,
    /// True when the mouse was the most recent movement input. A movement
    /// keypress flips this back to false so the keyboard reclaims control.
    pub prefer_mouse: bool,
}

impl InputState {
    pub fn new(kitty: bool) -> Self {
        InputState {
            grace: [0.0; 4],
            kitty,
            pressed: Vec::new(),
            mouse_pos: None,
            prefer_mouse: false,
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
                    // A movement key hands control back to the keyboard.
                    self.prefer_mouse = false;
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

    pub fn handle_mouse(&mut self, ev: MouseEvent) {
        match ev.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) | MouseEventKind::Down(_) => {
                self.mouse_pos = Some((ev.column, ev.row));
                self.prefer_mouse = true;
            }
            _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton};

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row: row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn mouse_move_sets_pos_and_prefers_mouse() {
        let mut s = InputState::new(false);
        s.handle_mouse(mouse(MouseEventKind::Moved, 12, 7));
        assert_eq!(s.mouse_pos, Some((12, 7)));
        assert!(s.prefer_mouse);
    }

    #[test]
    fn movement_key_reclaims_from_mouse() {
        let mut s = InputState::new(false);
        s.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 1, 1));
        assert!(s.prefer_mouse);
        s.handle_key(KeyEvent::from(KeyCode::Char('w')));
        assert!(!s.prefer_mouse);
        // ...but the cursor position is remembered for when the mouse returns.
        assert_eq!(s.mouse_pos, Some((1, 1)));
    }

    #[test]
    fn scroll_does_not_grab_control() {
        let mut s = InputState::new(false);
        s.handle_mouse(mouse(MouseEventKind::ScrollDown, 3, 3));
        assert!(!s.prefer_mouse);
    }
}
