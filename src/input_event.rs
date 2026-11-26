//! The input layer consumes parsed keys and pointer events on both platforms.

#[cfg(not(target_arch = "wasm32"))]
pub use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind,
};

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use browser::*;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod browser {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum KeyCode {
        Char(char),
        Backspace,
        Enter,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        PageUp,
        PageDown,
        Tab,
        BackTab,
        Delete,
        Insert,
        F(u8),
        Esc,
        Modifier(ModifierKeyCode),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ModifierKeyCode {
        LeftShift,
        RightShift,
        LeftControl,
        RightControl,
        LeftAlt,
        RightAlt,
        Meta,
    }

    bitflags::bitflags! {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct KeyModifiers: u8 {
            const NONE = 0;
            const SHIFT = 1 << 0;
            const CONTROL = 1 << 1;
            const ALT = 1 << 2;
            const SUPER = 1 << 3;
            const HYPER = 1 << 4;
            const META = 1 << 5;
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum KeyEventKind {
        Press,
        Repeat,
        Release,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct KeyEvent {
        pub code: KeyCode,
        pub modifiers: KeyModifiers,
        pub kind: KeyEventKind,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum MouseButton {
        Left,
        Right,
        Middle,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum MouseEventKind {
        Down(MouseButton),
        Up(MouseButton),
        Drag(MouseButton),
        Moved,
        ScrollUp,
        ScrollDown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct MouseEvent {
        pub kind: MouseEventKind,
        pub column: u16,
        pub row: u16,
        pub modifiers: KeyModifiers,
    }
}
