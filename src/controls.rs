//! Control schemes, remappable bindings and the legends that name them.
//!
//! Three schemes, selectable in Options and persisted beside the other
//! preferences (`crate::scores`):
//!
//! * **Classic** — the game as it shipped: WASD/arrows move, or the mouse
//!   steers the rat toward the pointer; weapons auto-aim; Ctrl dashes (Space
//!   on a terminal that cannot report a lone modifier); Space or a right click
//!   releases the charged active weapons; a left click is the dash, and it
//!   releases a charged active when the dash actually goes off.
//! * **Mouse aim** — WASD/arrows move (eight directions, diagonals unit
//!   length), the pointer **aims** and never steers; auto-firing weapons keep
//!   their timers and aim at the reticle; left click releases the charged
//!   actives; right click (or Ctrl) dashes; pause is Esc/P.
//! * **Gamepad** — left stick or d-pad moves (analog magnitude scales speed),
//!   the right stick aims while deflected, a shoulder dashes, a face button
//!   fires actives, Start pauses. Menus are navigable with the d-pad/stick,
//!   A confirm and B back.
//!
//! Bindings are per scheme and split into a keyboard/mouse set (with a primary
//! and a secondary slot, so "Ctrl or left click" is one action) and a
//! gamepad set. Every binding is a plain value, so a conflict is resolved by
//! **swapping** the two actions' bindings, and a whole scheme can be reset to
//! its defaults.
//!
//! # Terminal reality
//!
//! A lone modifier (`Ctrl`, `Shift`, `Alt`) can only be bound where the
//! terminal reports it — the Kitty keyboard protocol with
//! `REPORT_ALL_KEYS_AS_ESCAPE_CODES`, the same flag `input::DashKey` has always
//! depended on. `Cmd`/Super and most `Fn` combinations never reach a terminal
//! app at all, and `Ctrl+C` is reserved by the quit path, so
//! [`KeyBind::from_event`] refuses all three and the Controls screen says so.
//!
//! The legends ([`Controls::title_legend_lines`] and friends) are cached
//! strings, rebuilt only when a binding changes, so the render path never
//! formats a binding.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton};

use crate::math::Vec2;

/// How many actions the table holds (index into every binding array).
pub const ACTION_COUNT: usize = 27;

/// One thing the player can bind a key, a mouse button or a pad button to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Dash,
    FireActives,
    Pause,
    Map,
    MinimapSize,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    ToggleCursor,
    Mute,
    MusicDown,
    MusicUp,
    EffectsDown,
    EffectsUp,
    Options,
    Confirm,
    Back,
    /// Flip auto-pick level-ups (in play and on the level-up screen).
    /// Appended: saved binding arrays are indexed by action.
    AutoPick,
    /// The story crawl on the title, the Library on the pause screen.
    Library,
    /// Open the Nest from the pause plate, the incident report and the
    /// campaign victory screen.
    Nest,
    /// Leave: quit from the title, back to the menu from the pause plate, the
    /// incident report and the victory screen.
    Quit,
    /// Refund every Nest track; reset the scheme on the Controls screen.
    Reset,
    /// Open the Graphics page from play or pause, or close it there.
    Graphics,
}

/// Every action in table order; the index is the array index everywhere.
pub const ACTIONS: [Action; ACTION_COUNT] = [
    Action::MoveUp,
    Action::MoveDown,
    Action::MoveLeft,
    Action::MoveRight,
    Action::Dash,
    Action::FireActives,
    Action::Pause,
    Action::Map,
    Action::MinimapSize,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::ResetZoom,
    Action::ToggleCursor,
    Action::Mute,
    Action::MusicDown,
    Action::MusicUp,
    Action::EffectsDown,
    Action::EffectsUp,
    Action::Options,
    Action::Confirm,
    Action::Back,
    Action::AutoPick,
    Action::Library,
    Action::Nest,
    Action::Quit,
    Action::Reset,
    Action::Graphics,
];

impl Action {
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The label the Controls screen prints in its action column.
    pub const fn label(self) -> &'static str {
        match self {
            Action::MoveUp => "MOVE UP",
            Action::MoveDown => "MOVE DOWN",
            Action::MoveLeft => "MOVE LEFT",
            Action::MoveRight => "MOVE RIGHT",
            Action::Dash => "DASH",
            Action::FireActives => "FIRE ACTIVES",
            Action::Pause => "PAUSE",
            Action::Map => "MAP",
            Action::MinimapSize => "MINIMAP SIZE",
            Action::ZoomIn => "ZOOM IN",
            Action::ZoomOut => "ZOOM OUT",
            Action::ResetZoom => "RESET ZOOM",
            Action::ToggleCursor => "CURSOR",
            Action::Mute => "MUTE",
            Action::MusicDown => "MUSIC DOWN",
            Action::MusicUp => "MUSIC UP",
            Action::EffectsDown => "EFFECTS DOWN",
            Action::EffectsUp => "EFFECTS UP",
            Action::Options => "OPTIONS",
            Action::Confirm => "CONFIRM",
            Action::Back => "BACK",
            Action::AutoPick => "AUTO-PICK",
            Action::Library => "STORY/LIBRARY",
            Action::Nest => "THE NEST",
            Action::Quit => "QUIT/TO MENU",
            Action::Reset => "REFUND/RESET",
            Action::Graphics => "GRAPHICS",
        }
    }

    /// The actions a menu cannot be driven without. A rebind may move their
    /// key but never leaves one of them with no keyboard/mouse binding at all
    /// (see [`Bindings::bind_kbm`]).
    pub const fn is_menu_critical(self) -> bool {
        matches!(self, Action::Pause | Action::Back | Action::Confirm)
    }

    /// Is this action a movement direction? Movement bindings are the ones
    /// that hold a key down instead of firing once.
    pub const fn is_move(self) -> bool {
        matches!(
            self,
            Action::MoveUp | Action::MoveDown | Action::MoveLeft | Action::MoveRight
        )
    }
}

/// The three selectable schemes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scheme {
    #[default]
    Classic,
    MouseAim,
    Gamepad,
}

pub const SCHEMES: [Scheme; 3] = [Scheme::Classic, Scheme::MouseAim, Scheme::Gamepad];

impl Scheme {
    pub const fn index(self) -> usize {
        match self {
            Scheme::Classic => 0,
            Scheme::MouseAim => 1,
            Scheme::Gamepad => 2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Scheme::Classic => "CLASSIC",
            Scheme::MouseAim => "MOUSE AIM",
            Scheme::Gamepad => "GAMEPAD",
        }
    }

    /// The saved spelling (`classic` / `mouse-aim` / `gamepad`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Scheme::Classic => "classic",
            Scheme::MouseAim => "mouse-aim",
            Scheme::Gamepad => "gamepad",
        }
    }

    pub fn parse(raw: &str) -> Option<Scheme> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "classic" | "default" => Some(Scheme::Classic),
            "mouse aim" => Some(Scheme::MouseAim),
            "mouse-aim" | "mouseaim" | "mouse_aim" | "aim" => Some(Scheme::MouseAim),
            "gamepad" | "pad" => Some(Scheme::Gamepad),
            _ => None,
        }
    }

    /// Step through the three schemes, wrapping.
    pub fn step(self, dir: i32) -> Scheme {
        let i = self.index() as i32;
        let n = SCHEMES.len() as i32;
        SCHEMES[(((i + dir) % n + n) % n) as usize]
    }
}

/// Which device drove the last input, so the legends can show the glyphs the
/// player is actually looking at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Device {
    #[default]
    Keyboard,
    Mouse,
    Pad,
}

/// A rebindable key. Letters match case-insensitively, so binding `w` still
/// moves with Shift held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyBind {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Esc,
    Tab,
    Space,
    Backspace,
    Delete,
    Insert,
    F(u8),
    /// A lone modifier press. Only reported under the Kitty protocol's
    /// report-all-keys flag (see the module docs).
    Ctrl,
    Shift,
    Alt,
}

impl KeyBind {
    /// The label a legend, the HUD and the Controls screen print.
    pub fn label(self) -> &'static str {
        match self {
            KeyBind::Char(' ') => "Space",
            KeyBind::Char('-') => "-",
            KeyBind::Char('=') => "=",
            KeyBind::Char('[') => "[",
            KeyBind::Char(']') => "]",
            KeyBind::Char(',') => ",",
            KeyBind::Char('.') => ".",
            KeyBind::Char('/') => "/",
            KeyBind::Char(';') => ";",
            KeyBind::Char('\'') => "'",
            KeyBind::Char('\\') => "\\",
            KeyBind::Char('`') => "`",
            KeyBind::Char('a') => "A",
            KeyBind::Char('b') => "B",
            KeyBind::Char('c') => "C",
            KeyBind::Char('d') => "D",
            KeyBind::Char('e') => "E",
            KeyBind::Char('f') => "F",
            KeyBind::Char('g') => "G",
            KeyBind::Char('h') => "H",
            KeyBind::Char('i') => "I",
            KeyBind::Char('j') => "J",
            KeyBind::Char('k') => "K",
            KeyBind::Char('l') => "L",
            KeyBind::Char('m') => "M",
            KeyBind::Char('n') => "N",
            KeyBind::Char('o') => "O",
            KeyBind::Char('p') => "P",
            KeyBind::Char('q') => "Q",
            KeyBind::Char('r') => "R",
            KeyBind::Char('s') => "S",
            KeyBind::Char('t') => "T",
            KeyBind::Char('u') => "U",
            KeyBind::Char('v') => "V",
            KeyBind::Char('w') => "W",
            KeyBind::Char('x') => "X",
            KeyBind::Char('y') => "Y",
            KeyBind::Char('z') => "Z",
            KeyBind::Char('0') => "0",
            KeyBind::Char('1') => "1",
            KeyBind::Char('2') => "2",
            KeyBind::Char('3') => "3",
            KeyBind::Char('4') => "4",
            KeyBind::Char('5') => "5",
            KeyBind::Char('6') => "6",
            KeyBind::Char('7') => "7",
            KeyBind::Char('8') => "8",
            KeyBind::Char('9') => "9",
            KeyBind::Char(_) => "KEY",
            KeyBind::Up => "UP",
            KeyBind::Down => "DOWN",
            KeyBind::Left => "LEFT",
            KeyBind::Right => "RIGHT",
            KeyBind::Home => "Home",
            KeyBind::End => "End",
            KeyBind::PageUp => "PgUp",
            KeyBind::PageDown => "PgDn",
            KeyBind::Enter => "Enter",
            KeyBind::Esc => "Esc",
            KeyBind::Tab => "Tab",
            KeyBind::Space => "Space",
            KeyBind::Backspace => "Backspace",
            KeyBind::Delete => "Delete",
            KeyBind::Insert => "Insert",
            KeyBind::F(1) => "F1",
            KeyBind::F(2) => "F2",
            KeyBind::F(3) => "F3",
            KeyBind::F(4) => "F4",
            KeyBind::F(5) => "F5",
            KeyBind::F(6) => "F6",
            KeyBind::F(7) => "F7",
            KeyBind::F(8) => "F8",
            KeyBind::F(9) => "F9",
            KeyBind::F(10) => "F10",
            KeyBind::F(11) => "F11",
            KeyBind::F(12) => "F12",
            KeyBind::F(_) => "F",
            KeyBind::Ctrl => "Ctrl",
            KeyBind::Shift => "Shift",
            KeyBind::Alt => "Alt",
        }
    }

    /// The stable saved spelling (`w`, `up`, `space`, `ctrl`, `f5`).
    pub fn as_str(self) -> String {
        match self {
            KeyBind::Char(c) => c.to_string(),
            KeyBind::Up => "up".into(),
            KeyBind::Down => "down".into(),
            KeyBind::Left => "left".into(),
            KeyBind::Right => "right".into(),
            KeyBind::Home => "home".into(),
            KeyBind::End => "end".into(),
            KeyBind::PageUp => "pgup".into(),
            KeyBind::PageDown => "pgdn".into(),
            KeyBind::Enter => "enter".into(),
            KeyBind::Esc => "esc".into(),
            KeyBind::Tab => "tab".into(),
            KeyBind::Space => "space".into(),
            KeyBind::Backspace => "backspace".into(),
            KeyBind::Delete => "delete".into(),
            KeyBind::Insert => "insert".into(),
            KeyBind::F(n) => format!("f{n}"),
            KeyBind::Ctrl => "ctrl".into(),
            KeyBind::Shift => "shift".into(),
            KeyBind::Alt => "alt".into(),
        }
    }

    pub fn parse(raw: &str) -> Option<KeyBind> {
        match raw {
            "up" => return Some(KeyBind::Up),
            "down" => return Some(KeyBind::Down),
            "left" => return Some(KeyBind::Left),
            "right" => return Some(KeyBind::Right),
            "home" => return Some(KeyBind::Home),
            "end" => return Some(KeyBind::End),
            "pgup" => return Some(KeyBind::PageUp),
            "pgdn" => return Some(KeyBind::PageDown),
            "enter" => return Some(KeyBind::Enter),
            "esc" => return Some(KeyBind::Esc),
            "tab" => return Some(KeyBind::Tab),
            "space" => return Some(KeyBind::Space),
            "backspace" => return Some(KeyBind::Backspace),
            "delete" => return Some(KeyBind::Delete),
            "insert" => return Some(KeyBind::Insert),
            "ctrl" => return Some(KeyBind::Ctrl),
            "shift" => return Some(KeyBind::Shift),
            "alt" => return Some(KeyBind::Alt),
            _ => {}
        }
        if let Some(n) = raw.strip_prefix('f') {
            if let Ok(n) = n.parse::<u8>() {
                if (1..=12).contains(&n) {
                    return Some(KeyBind::F(n));
                }
            }
        }
        let mut chars = raw.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        if c == ' ' {
            // One spelling per key: `Char(' ')` would match the same event as
            // `Space` yet compare unequal, hiding the pair from conflict swaps.
            return Some(KeyBind::Space);
        }
        Some(KeyBind::Char(c.to_ascii_lowercase()))
    }

    /// Turn a terminal key event into a binding, refusing the keys a terminal
    /// cannot deliver faithfully.
    ///
    /// * Lone modifiers are bindable (they need report-all-keys; the screen
    ///   notes when the terminal does not report them).
    /// * `Cmd`/Super and every other modifier combination is refused: a
    ///   terminal only sees the character a combo types, never the combo.
    /// * `Ctrl+C` is refused because it is the quit path, not a control.
    pub fn from_event(ev: &KeyEvent) -> Result<KeyBind, Unbindable> {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let alt = ev.modifiers.contains(KeyModifiers::ALT);
        let sup = ev.modifiers.intersects(
            KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META,
        );
        if sup {
            return Err(Unbindable::Super);
        }
        if ctrl && matches!(ev.code, KeyCode::Char('c' | 'C')) {
            return Err(Unbindable::CtrlC);
        }
        let bind = match ev.code {
            KeyCode::Modifier(m) => match m {
                ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl => KeyBind::Ctrl,
                ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift => KeyBind::Shift,
                ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt => KeyBind::Alt,
                _ => return Err(Unbindable::Other),
            },
            KeyCode::Char(' ') => KeyBind::Space,
            KeyCode::Char(c) => KeyBind::Char(c.to_ascii_lowercase()),
            KeyCode::Up => KeyBind::Up,
            KeyCode::Down => KeyBind::Down,
            KeyCode::Left => KeyBind::Left,
            KeyCode::Right => KeyBind::Right,
            KeyCode::Home => KeyBind::Home,
            KeyCode::End => KeyBind::End,
            KeyCode::PageUp => KeyBind::PageUp,
            KeyCode::PageDown => KeyBind::PageDown,
            KeyCode::Enter => KeyBind::Enter,
            KeyCode::Esc => KeyBind::Esc,
            KeyCode::Tab => KeyBind::Tab,
            KeyCode::Backspace => KeyBind::Backspace,
            KeyCode::Delete => KeyBind::Delete,
            KeyCode::Insert => KeyBind::Insert,
            KeyCode::F(n) if (1..=12).contains(&n) => KeyBind::F(n),
            _ => return Err(Unbindable::Other),
        };
        // A combo that reaches a terminal only as its character is not a
        // binding the terminal can ever tell apart from the bare key.
        if (ctrl || alt) && !matches!(bind, KeyBind::Ctrl | KeyBind::Alt) {
            return Err(Unbindable::Combo);
        }
        // (A lone modifier arrives with its own modifier bit set; that is the
        // same press, not a combo, which is why the two are exempt above.)
        Ok(bind)
    }

    /// Does this event match the binding? Letters are case-insensitive and
    /// Shift on a letter is ignored (Shift+W still moves up).
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        if ev.kind == KeyEventKind::Release {
            return false;
        }
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let alt = ev.modifiers.contains(KeyModifiers::ALT);
        self.matches_code(ev, ctrl || alt)
    }

    /// Does this event's *code* match the binding, ignoring Ctrl/Alt and the
    /// event kind? Held movement has to see through a modifier and its release
    /// (Ctrl+W moves and its key-up clears the key), unlike a discrete tap.
    pub fn matches_movement(&self, ev: &KeyEvent) -> bool {
        self.matches_code(ev, false)
    }

    fn matches_code(&self, ev: &KeyEvent, modifiers_set: bool) -> bool {
        match *self {
            KeyBind::Ctrl => matches!(
                ev.code,
                KeyCode::Modifier(ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl)
            ),
            KeyBind::Shift => matches!(
                ev.code,
                KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift)
            ),
            KeyBind::Alt => matches!(
                ev.code,
                KeyCode::Modifier(ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt)
            ),
            _ => {
                if modifiers_set {
                    return false;
                }
                match (self, ev.code) {
                    (KeyBind::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
                    (KeyBind::Space, KeyCode::Char(' ')) => true,
                    (KeyBind::Up, KeyCode::Up) => true,
                    (KeyBind::Down, KeyCode::Down) => true,
                    (KeyBind::Left, KeyCode::Left) => true,
                    (KeyBind::Right, KeyCode::Right) => true,
                    (KeyBind::Home, KeyCode::Home) => true,
                    (KeyBind::End, KeyCode::End) => true,
                    (KeyBind::PageUp, KeyCode::PageUp) => true,
                    (KeyBind::PageDown, KeyCode::PageDown) => true,
                    (KeyBind::Enter, KeyCode::Enter) => true,
                    (KeyBind::Esc, KeyCode::Esc) => true,
                    (KeyBind::Tab, KeyCode::Tab) => true,
                    (KeyBind::Backspace, KeyCode::Backspace) => true,
                    (KeyBind::Delete, KeyCode::Delete) => true,
                    (KeyBind::Insert, KeyCode::Insert) => true,
                    (KeyBind::F(a), KeyCode::F(b)) => *a == b,
                    _ => false,
                }
            }
        }
    }
}

/// Why a key event cannot be bound (the Controls screen prints the reason).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unbindable {
    /// Cmd/Super never reaches a terminal application.
    Super,
    /// Ctrl+C is the quit path.
    CtrlC,
    /// A modifier combination the terminal only reports as its character.
    Combo,
    /// Anything else (media keys, unknown codes, …).
    Other,
}

impl Unbindable {
    pub const fn note(self) -> &'static str {
        match self {
            Unbindable::Super => "CMD/SUPER never reaches a terminal — bind another key",
            Unbindable::CtrlC => "CTRL+C always quits and cannot be rebound",
            Unbindable::Combo => "that combo arrives as its character — bind the plain key",
            Unbindable::Other => "this terminal cannot report that key — try another",
        }
    }
}

/// A mouse button or wheel direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseBind {
    Left,
    Right,
    Middle,
    WheelUp,
    WheelDown,
}

impl MouseBind {
    pub const fn label(self) -> &'static str {
        match self {
            MouseBind::Left => "LMB",
            MouseBind::Right => "RMB",
            MouseBind::Middle => "MMB",
            MouseBind::WheelUp => "WHEEL↑",
            MouseBind::WheelDown => "WHEEL↓",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            MouseBind::Left => "left",
            MouseBind::Right => "right",
            MouseBind::Middle => "middle",
            MouseBind::WheelUp => "wheel-up",
            MouseBind::WheelDown => "wheel-down",
        }
    }

    pub fn parse(raw: &str) -> Option<MouseBind> {
        match raw {
            "left" => Some(MouseBind::Left),
            "right" => Some(MouseBind::Right),
            "middle" => Some(MouseBind::Middle),
            "wheel-up" => Some(MouseBind::WheelUp),
            "wheel-down" => Some(MouseBind::WheelDown),
            _ => None,
        }
    }

    pub const fn button(self) -> Option<MouseButton> {
        match self {
            MouseBind::Left => Some(MouseButton::Left),
            MouseBind::Right => Some(MouseButton::Right),
            MouseBind::Middle => Some(MouseButton::Middle),
            _ => None,
        }
    }
}

/// A gamepad button. The analog sticks are hardware, not bindings: the left
/// stick always moves and the right stick always aims in the Gamepad scheme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadBind {
    South,
    East,
    North,
    West,
    Start,
    Select,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// Every pad button, in table order; the index is the bit index.
pub const PAD_BINDS: [PadBind; 16] = [
    PadBind::South,
    PadBind::East,
    PadBind::North,
    PadBind::West,
    PadBind::Start,
    PadBind::Select,
    PadBind::L1,
    PadBind::R1,
    PadBind::L2,
    PadBind::R2,
    PadBind::L3,
    PadBind::R3,
    PadBind::DPadUp,
    PadBind::DPadDown,
    PadBind::DPadLeft,
    PadBind::DPadRight,
];

pub const PAD_BIND_COUNT: usize = PAD_BINDS.len();

impl PadBind {
    pub const fn index(self) -> usize {
        match self {
            PadBind::South => 0,
            PadBind::East => 1,
            PadBind::North => 2,
            PadBind::West => 3,
            PadBind::Start => 4,
            PadBind::Select => 5,
            PadBind::L1 => 6,
            PadBind::R1 => 7,
            PadBind::L2 => 8,
            PadBind::R2 => 9,
            PadBind::L3 => 10,
            PadBind::R3 => 11,
            PadBind::DPadUp => 12,
            PadBind::DPadDown => 13,
            PadBind::DPadLeft => 14,
            PadBind::DPadRight => 15,
        }
    }

    /// The glyph a controller legend prints. The d-pad keeps its arrow glyph so
    /// a keyboard player can read the pad row too.
    pub const fn label(self) -> &'static str {
        match self {
            PadBind::South => "A",
            PadBind::East => "B",
            PadBind::North => "Y",
            PadBind::West => "X",
            PadBind::Start => "START",
            PadBind::Select => "VIEW",
            PadBind::L1 => "LB",
            PadBind::R1 => "RB",
            PadBind::L2 => "LT",
            PadBind::R2 => "RT",
            PadBind::L3 => "L3",
            PadBind::R3 => "R3",
            PadBind::DPadUp => "D↑",
            PadBind::DPadDown => "D↓",
            PadBind::DPadLeft => "D←",
            PadBind::DPadRight => "D→",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            PadBind::South => "south",
            PadBind::East => "east",
            PadBind::North => "north",
            PadBind::West => "west",
            PadBind::Start => "start",
            PadBind::Select => "select",
            PadBind::L1 => "l1",
            PadBind::R1 => "r1",
            PadBind::L2 => "l2",
            PadBind::R2 => "r2",
            PadBind::L3 => "l3",
            PadBind::R3 => "r3",
            PadBind::DPadUp => "dpad-up",
            PadBind::DPadDown => "dpad-down",
            PadBind::DPadLeft => "dpad-left",
            PadBind::DPadRight => "dpad-right",
        }
    }

    pub fn parse(raw: &str) -> Option<PadBind> {
        PAD_BINDS.iter().copied().find(|b| b.as_str() == raw)
    }
}

/// One binding slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Binding {
    Key(KeyBind),
    Mouse(MouseBind),
    Pad(PadBind),
    /// No binding in this scheme.
    #[default]
    None,
}

impl Binding {
    pub fn label(self) -> &'static str {
        match self {
            Binding::Key(k) => k.label(),
            Binding::Mouse(m) => m.label(),
            Binding::Pad(p) => p.label(),
            Binding::None => "—",
        }
    }

    /// The saved spelling: `key:w`, `mouse:left`, `pad:south`, `none`.
    pub fn as_str(self) -> String {
        match self {
            Binding::Key(k) => format!("key:{}", k.as_str()),
            Binding::Mouse(m) => format!("mouse:{}", m.as_str()),
            Binding::Pad(p) => format!("pad:{}", p.as_str()),
            Binding::None => "none".into(),
        }
    }

    pub fn parse(raw: &str) -> Option<Binding> {
        if raw == "none" {
            return Some(Binding::None);
        }
        if let Some(rest) = raw.strip_prefix("key:") {
            return KeyBind::parse(rest).map(Binding::Key);
        }
        if let Some(rest) = raw.strip_prefix("mouse:") {
            return MouseBind::parse(rest).map(Binding::Mouse);
        }
        if let Some(rest) = raw.strip_prefix("pad:") {
            return PadBind::parse(rest).map(Binding::Pad);
        }
        // Tolerate a bare key name written by hand.
        KeyBind::parse(raw).map(Binding::Key)
    }

    pub const fn is_none(self) -> bool {
        matches!(self, Binding::None)
    }
}

/// One scheme's complete binding set: a primary and a secondary keyboard/mouse
/// slot per action (so "Ctrl or right click" is one action) plus a pad slot.
#[derive(Clone, Copy, Debug)]
pub struct Bindings {
    pub kbm: [Binding; ACTION_COUNT],
    pub alt: [Binding; ACTION_COUNT],
    pub pad: [Binding; ACTION_COUNT],
}

const NONE: Binding = Binding::None;

impl Bindings {
    /// The default set for a scheme.
    pub fn defaults(scheme: Scheme) -> Bindings {
        let mut b = Bindings {
            kbm: [NONE; ACTION_COUNT],
            alt: [NONE; ACTION_COUNT],
            pad: [NONE; ACTION_COUNT],
        };
        // Keyboard/mouse defaults. Movement, menus and the preference keys are
        // the same in every scheme; the click jobs and the dash differ.
        b.kbm[Action::MoveUp.index()] = Binding::Key(KeyBind::Char('w'));
        b.kbm[Action::MoveDown.index()] = Binding::Key(KeyBind::Char('s'));
        b.kbm[Action::MoveLeft.index()] = Binding::Key(KeyBind::Char('a'));
        b.kbm[Action::MoveRight.index()] = Binding::Key(KeyBind::Char('d'));
        b.kbm[Action::Pause.index()] = Binding::Key(KeyBind::Esc);
        b.alt[Action::Pause.index()] = Binding::Key(KeyBind::Char('p'));
        b.kbm[Action::Map.index()] = Binding::Key(KeyBind::Tab);
        b.kbm[Action::MinimapSize.index()] = Binding::Key(KeyBind::Char('n'));
        b.kbm[Action::ZoomIn.index()] = Binding::Key(KeyBind::Char('z'));
        b.kbm[Action::ZoomOut.index()] = Binding::Key(KeyBind::Char('x'));
        b.kbm[Action::ResetZoom.index()] = Binding::Mouse(MouseBind::Middle);
        b.kbm[Action::ToggleCursor.index()] = Binding::Key(KeyBind::Char('c'));
        b.kbm[Action::Mute.index()] = Binding::Key(KeyBind::Char('m'));
        b.kbm[Action::MusicDown.index()] = Binding::Key(KeyBind::Char('['));
        b.kbm[Action::MusicUp.index()] = Binding::Key(KeyBind::Char(']'));
        b.kbm[Action::EffectsDown.index()] = Binding::Key(KeyBind::Char('-'));
        b.kbm[Action::EffectsUp.index()] = Binding::Key(KeyBind::Char('='));
        b.kbm[Action::Options.index()] = Binding::Key(KeyBind::Char('o'));
        b.kbm[Action::Confirm.index()] = Binding::Key(KeyBind::Enter);
        b.kbm[Action::Back.index()] = Binding::Key(KeyBind::Esc);
        b.kbm[Action::AutoPick.index()] = Binding::Key(KeyBind::Char('u'));
        b.kbm[Action::Library.index()] = Binding::Key(KeyBind::Char('l'));
        // `B`, not the `N` the end screens once used: `N` is the minimap size,
        // and one action cannot share a default with another.
        b.kbm[Action::Nest.index()] = Binding::Key(KeyBind::Char('b'));
        b.kbm[Action::Quit.index()] = Binding::Key(KeyBind::Char('q'));
        b.kbm[Action::Reset.index()] = Binding::Key(KeyBind::Char('r'));
        b.kbm[Action::Graphics.index()] = Binding::Key(KeyBind::Char('g'));

        // Pad defaults: d-pad/stick move, RB dashes, X fires the actives,
        // Start pauses, View opens the map, Y flips the cursor, LB mutes,
        // RT/LT zoom, R3 resets the zoom and L3 cycles the minimap.
        b.pad[Action::MoveUp.index()] = Binding::Pad(PadBind::DPadUp);
        b.pad[Action::MoveDown.index()] = Binding::Pad(PadBind::DPadDown);
        b.pad[Action::MoveLeft.index()] = Binding::Pad(PadBind::DPadLeft);
        b.pad[Action::MoveRight.index()] = Binding::Pad(PadBind::DPadRight);
        b.pad[Action::Dash.index()] = Binding::Pad(PadBind::R1);
        b.pad[Action::FireActives.index()] = Binding::Pad(PadBind::West);
        b.pad[Action::Pause.index()] = Binding::Pad(PadBind::Start);
        b.pad[Action::Map.index()] = Binding::Pad(PadBind::Select);
        b.pad[Action::MinimapSize.index()] = Binding::Pad(PadBind::L3);
        b.pad[Action::ZoomIn.index()] = Binding::Pad(PadBind::R2);
        b.pad[Action::ZoomOut.index()] = Binding::Pad(PadBind::L2);
        b.pad[Action::ResetZoom.index()] = Binding::Pad(PadBind::R3);
        b.pad[Action::ToggleCursor.index()] = Binding::Pad(PadBind::North);
        b.pad[Action::Mute.index()] = Binding::Pad(PadBind::L1);
        b.pad[Action::Confirm.index()] = Binding::Pad(PadBind::South);
        b.pad[Action::Back.index()] = Binding::Pad(PadBind::East);

        match scheme {
            Scheme::Classic | Scheme::Gamepad => {
                // Ctrl or the left click dashes; Space or the right click fires
                // the charged actives. The left click also releases a charged
                // active when the dash it starts actually goes off (the app's
                // click path), so it stays the pair the mouse player expects
                // while a right click is the actives on their own.
                b.kbm[Action::Dash.index()] = Binding::Key(KeyBind::Ctrl);
                b.alt[Action::Dash.index()] = Binding::Mouse(MouseBind::Left);
                b.kbm[Action::FireActives.index()] = Binding::Key(KeyBind::Space);
                b.alt[Action::FireActives.index()] = Binding::Mouse(MouseBind::Right);
            }
            Scheme::MouseAim => {
                // Right click dashes, Ctrl is the keyboard alternate; left
                // click fires the actives and Space is its alternate.
                b.kbm[Action::Dash.index()] = Binding::Mouse(MouseBind::Right);
                b.alt[Action::Dash.index()] = Binding::Key(KeyBind::Ctrl);
                b.kbm[Action::FireActives.index()] = Binding::Mouse(MouseBind::Left);
                b.alt[Action::FireActives.index()] = Binding::Key(KeyBind::Space);
            }
        }
        b
    }

    pub fn get(&self, a: Action) -> Binding {
        self.kbm[a.index()]
    }

    pub fn alt_of(&self, a: Action) -> Binding {
        self.alt[a.index()]
    }

    pub fn pad_of(&self, a: Action) -> Binding {
        self.pad[a.index()]
    }

    /// Every keyboard/mouse slot an action owns, primary first.
    pub fn kbm_slots(&self, a: Action) -> [Binding; 2] {
        [self.kbm[a.index()], self.alt[a.index()]]
    }

    /// Find the action a keyboard/mouse binding belongs to (`None` when unbound).
    pub fn action_for_kbm(&self, b: Binding) -> Option<Action> {
        if b.is_none() {
            return None;
        }
        ACTIONS.into_iter().find(|&a| self.kbm_slots(a).contains(&b))
    }

    /// Find the action a pad binding belongs to.
    pub fn action_for_pad(&self, b: Binding) -> Option<Action> {
        if b.is_none() {
            return None;
        }
        ACTIONS.iter().copied().find(|a| self.pad[a.index()] == b)
    }

    /// Bind `new` to `action`, resolving a conflict by **swapping** the two
    /// actions' bindings. Returns the action whose binding moved, if any.
    ///
    /// Two details keep the table free of surprises:
    /// * rebinding an action to the binding its **own alternate** already holds
    ///   swaps the two slots (the old primary becomes the alternate) instead of
    ///   leaving the same binding in both and dropping the old one;
    /// * the defaults share a binding between a pair of actions on purpose
    ///   (`Esc` is Pause and Back), so every slot that held `new` receives the
    ///   old binding and the pair stays paired. The last such action is the one
    ///   reported;
    /// * when `action` had **no** binding there is nothing to hand back, so a
    ///   holder that is menu-critical ([`Action::is_menu_critical`]) and has no
    ///   other slot keeps the key and shares it rather than going unbound.
    pub fn bind_kbm(&mut self, action: Action, new: Binding) -> Option<Action> {
        let old = self.kbm[action.index()];
        if new == old {
            return None;
        }
        let mut swapped = None;
        if !new.is_none() {
            if self.alt[action.index()] == new {
                self.alt[action.index()] = old;
            }
            for other in ACTIONS {
                if other == action {
                    continue;
                }
                for slot in 0..2 {
                    let cur = if slot == 0 {
                        self.kbm[other.index()]
                    } else {
                        self.alt[other.index()]
                    };
                    if cur == new {
                        // Taking a key for an action that had none hands the
                        // holder nothing in return. Pause, Back and Confirm
                        // must never end up with no key at all that way (Esc
                        // is Pause *and* Back: one such rebind would strip
                        // both, and the menus could not be left), so a holder
                        // with no other slot keeps the key and shares it.
                        let spare = if slot == 0 {
                            self.alt[other.index()]
                        } else {
                            self.kbm[other.index()]
                        };
                        if old.is_none()
                            && other.is_menu_critical()
                            && (spare.is_none() || spare == new)
                        {
                            continue;
                        }
                        if slot == 0 {
                            self.kbm[other.index()] = old;
                        } else {
                            self.alt[other.index()] = old;
                        }
                        swapped = Some(other);
                    }
                }
            }
        }
        self.kbm[action.index()] = new;
        swapped
    }

    /// Bind `new` to `action`'s pad slot, swapping on a conflict.
    pub fn bind_pad(&mut self, action: Action, new: Binding) -> Option<Action> {
        let old = self.pad[action.index()];
        let mut swapped = None;
        if !new.is_none() {
            for other in ACTIONS {
                if other == action {
                    continue;
                }
                if self.pad[other.index()] == new {
                    self.pad[other.index()] = old;
                    swapped = Some(other);
                }
            }
        }
        self.pad[action.index()] = new;
        swapped
    }

    /// Clear a binding entirely (unbind).
    pub fn unbind_kbm(&mut self, action: Action) {
        self.kbm[action.index()] = Binding::None;
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings::defaults(Scheme::Classic)
    }
}

/// Saved binding sets, one per scheme, tolerant of old or hand-edited files:
/// a missing or unparseable entry keeps its default.
#[derive(Clone, Debug, Default)]
pub struct SavedBindings {
    /// Indexed by `Scheme::index`.
    pub schemes: [Option<SavedScheme>; 3],
}

#[derive(Clone, Debug, Default)]
pub struct SavedScheme {
    pub kbm: Vec<String>,
    pub alt: Vec<String>,
    pub pad: Vec<String>,
}

/// The whole control configuration: scheme, per-scheme bindings, pad tuning.
#[derive(Clone, Debug)]
pub struct Controls {
    scheme: Scheme,
    sets: [Bindings; 3],
    pub deadzone: f32,
    pub rumble: bool,
    /// Cached legends, rebuilt only when a binding or the scheme changes.
    title_wide: Vec<String>,
    title_narrow: Vec<String>,
    pause_wide: Vec<String>,
    pause_narrow: Vec<String>,
    dash_label: String,
    active_label: String,
    /// Legend of the last device used, recomputed per frame from the cache.
    device: Device,
    /// Whether the terminal reports a lone modifier press (set at startup).
    lone_ctrl_hint: bool,
}

/// Default gamepad dead zone (stick deflection below this reads as zero).
pub const DEFAULT_DEADZONE: f32 = 0.2;

impl Default for Controls {
    fn default() -> Self {
        let mut c = Controls {
            scheme: Scheme::Classic,
            sets: [
                Bindings::defaults(Scheme::Classic),
                Bindings::defaults(Scheme::MouseAim),
                Bindings::defaults(Scheme::Gamepad),
            ],
            deadzone: DEFAULT_DEADZONE,
            rumble: false,
            title_wide: Vec::new(),
            title_narrow: Vec::new(),
            pause_wide: Vec::new(),
            pause_narrow: Vec::new(),
            dash_label: String::new(),
            active_label: String::new(),
            device: Device::Keyboard,
            lone_ctrl_hint: true,
        };
        c.rebuild();
        c
    }
}

impl Controls {
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The active scheme's bindings.
    pub fn active(&self) -> &Bindings {
        &self.sets[self.scheme.index()]
    }

    pub fn active_mut(&mut self) -> &mut Bindings {
        &mut self.sets[self.scheme.index()]
    }

    pub fn set_scheme(&mut self, scheme: Scheme) {
        self.scheme = scheme;
        self.rebuild();
    }

    /// Bind a keyboard/mouse slot on the active scheme (rebuilds the legends).
    pub fn bind_active_kbm(&mut self, action: Action, new: Binding) -> Option<Action> {
        let swapped = self.active_mut().bind_kbm(action, new);
        self.rebuild();
        swapped
    }

    /// Bind a pad slot on the active scheme (rebuilds the legends).
    pub fn bind_active_pad(&mut self, action: Action, new: Binding) -> Option<Action> {
        let swapped = self.active_mut().bind_pad(action, new);
        self.rebuild();
        swapped
    }

    /// Reset the active scheme to its defaults.
    pub fn reset_active(&mut self) {
        self.sets[self.scheme.index()] = Bindings::defaults(self.scheme);
        self.rebuild();
    }

    /// Note which device drove the last input (glyph hints follow it).
    ///
    /// Called every frame, so it only rebuilds the cached legends when the
    /// answer changes *between the pad and the keyboard/mouse* — the one switch
    /// the legends differ on. Without the rebuild the pad legends were only ever
    /// seen after an unrelated rebinding.
    pub fn note_device(&mut self, device: Device) {
        let was_pad = self.device == Device::Pad;
        self.device = device;
        if was_pad != (device == Device::Pad) {
            self.rebuild();
        }
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// Does the mouse aim (rather than steer) in this scheme?
    pub fn mouse_aims(&self) -> bool {
        self.scheme == Scheme::MouseAim
    }

    /// Does the pointer steer the rat in this scheme?
    pub fn mouse_steers(&self) -> bool {
        self.scheme != Scheme::MouseAim
    }

    /// One action's keyboard/mouse label on the active scheme (for the mute
    /// chip and other legends that name a single key).
    pub fn action_label(&self, a: Action) -> &str {
        match self.active().get(a) {
            Binding::Mouse(m) => m.label(),
            Binding::Key(k) => k.label(),
            Binding::Pad(p) => p.label(),
            Binding::None => "—",
        }
    }

    /// The label of the active-weapon input, as the legends name it: the pad
    /// button while the pad drives, else the key (or the click in Mouse aim),
    /// falling back to the alternate when the primary slot is unbound.
    pub fn active_label(&self) -> &str {
        &self.active_label
    }

    /// The dash key as it will actually work on **this** terminal: a Ctrl
    /// binding on a terminal without lone-modifier reporting falls back to
    /// Space (which then also fires the actives).
    pub fn live_dash_label(&self) -> &str {
        // `dash_label` already carries this rule for the keyboard legends; the
        // explicit check keeps a pad player's `RB` from being renamed `Space`.
        if self.device != Device::Pad
            && !self.lone_ctrl_hint
            && self.active().get(Action::Dash) == Binding::Key(KeyBind::Ctrl)
        {
            return "Space";
        }
        &self.dash_label
    }

    /// Unbind an action's keyboard/mouse slot on the active scheme.
    pub fn unbind_active_kbm(&mut self, action: Action) {
        self.active_mut().unbind_kbm(action);
        self.rebuild();
    }

    /// Rebuild the cached legends and labels. Called on any binding change.
    pub fn rebuild(&mut self) {
        let b = *self.active();
        // The keyboard legend names the keyboard (or mouse, or pad) binding.
        // When an action has a key *and* a mouse button (the classic left
        // click), the key is the one to print: the click has its own line.
        let key = |a: Action| -> String {
            let p = b.get(a);
            let alt = b.alt_of(a);
            let slot = |x: Binding| matches!(x, Binding::Key(_));
            match (p, alt) {
                (Binding::None, Binding::None) => "—".to_string(),
                (x, Binding::None) | (Binding::None, x) => x.label().to_string(),
                (x, _) if slot(x) => x.label().to_string(),
                (_, y) if slot(y) => y.label().to_string(),
                (x, _) => x.label().to_string(),
            }
        };
        let pad = |a: Action| -> String { b.pad_of(a).label().to_string() };
        // A key with a keyboard alternate prints as "Esc/P" (the pause pair);
        // a lone key prints itself.
        let pair = |a: Action| -> String {
            let primary = key(a);
            let alt = b.alt_of(a);
            if matches!(alt, Binding::Key(_)) {
                let alt = alt.label().to_string();
                if alt != primary {
                    return format!("{primary}/{alt}");
                }
            }
            primary
        };
        let pause = pair(Action::Pause);
        let graphics = if self.device == Device::Pad && !b.pad_of(Action::Graphics).is_none() {
            pad(Action::Graphics)
        } else {
            key(Action::Graphics)
        };

        if self.device == Device::Pad {
            // The HUD names the pad glyphs when the pad drove the last input.
            self.dash_label = pad(Action::Dash);
            self.active_label = pad(Action::FireActives);
            self.title_wide = vec![
                format!(
                    "L-stick/D-pad move · {} dash · {} actives · R-stick aim · hold {} to charge · {} pause",
                    pad(Action::Dash),
                    pad(Action::FireActives),
                    pad(Action::Dash),
                    pad(Action::Pause),
                ),
                format!(
                    "{} confirm · {} back · {} cursor · {} mute · {}/{} zoom",
                    pad(Action::Confirm),
                    pad(Action::Back),
                    pad(Action::ToggleCursor),
                    pad(Action::Mute),
                    pad(Action::ZoomIn),
                    pad(Action::ZoomOut),
                ),
                format!(
                    "{} map · {} minimap · {} reset zoom · {} options · {graphics} graphics",
                    pad(Action::Map),
                    pad(Action::MinimapSize),
                    pad(Action::ResetZoom),
                    pad(Action::Options),
                ),
            ];
            self.title_narrow = vec![
                format!("stick/d-pad move · {} dash · {} actives", pad(Action::Dash), pad(Action::FireActives)),
                format!("R-stick aim · {} pause · {} confirm · {} back", pad(Action::Pause), pad(Action::Confirm), pad(Action::Back)),
                format!("{} cursor · {} mute · {} map · {} zoom", pad(Action::ToggleCursor), pad(Action::Mute), pad(Action::Map), pad(Action::ZoomIn)),
                format!("{} reset zoom · {} options · {graphics} graphics", pad(Action::ResetZoom), pad(Action::Options)),
            ];
            self.pause_wide = vec![
                format!(
                    "stick/d-pad move · {} dash · {} actives · R-stick aim · {} unpause",
                    pad(Action::Dash),
                    pad(Action::FireActives),
                    pad(Action::Pause),
                ),
                format!(
                    "{} cursor · {} mute · {} options · {} back to menu · {graphics} graphics",
                    pad(Action::ToggleCursor),
                    pad(Action::Mute),
                    pad(Action::Options),
                    pad(Action::Back),
                ),
            ];
            self.pause_narrow = vec![
                format!("move stick · dash {} · actives {}", pad(Action::Dash), pad(Action::FireActives)),
                format!("{} unpause · {} options · {graphics} graphics", pad(Action::Pause), pad(Action::Options)),
            ];
            return;
        }

        // Keyboard/mouse legends. Every live control is named; the most
        // essential line is first and narrow plates keep it.
        // In Mouse aim the two clicks are the story, so the legend names the
        // buttons rather than the keyboard alternates.
        let mouse_first = |a: Action| -> String {
            let p = b.get(a);
            let alt = b.alt_of(a);
            match (p, alt) {
                (Binding::Mouse(m), _) => m.label().to_string(),
                (_, Binding::Mouse(m)) => m.label().to_string(),
                _ => key(a),
            }
        };
        let mouse_aim = self.scheme == Scheme::MouseAim;
        let dash = if !self.lone_ctrl_hint && b.get(Action::Dash) == Binding::Key(KeyBind::Ctrl) {
            // A Ctrl binding on a terminal without lone-modifier reporting is
            // delivered as Space, so that is what the legend must name.
            "Space".to_string()
        } else if mouse_aim {
            mouse_first(Action::Dash)
        } else {
            key(Action::Dash)
        };
        let actives = if mouse_aim {
            mouse_first(Action::FireActives)
        } else {
            key(Action::FireActives)
        };
        // The HUD prints the scheme's primary dash/fire label.
        self.dash_label = dash.clone();
        self.active_label = actives.clone();
        // Hold-to-charge: the dash input charges the Super Dash and its release
        // fires it. Where the terminal cannot report a key release the key can
        // only tap, so the legend names the mouse/pad instead of lying.
        let (charge_hint, narrow_charge) = if self.lone_ctrl_hint {
            (format!("hold {dash} to charge"), "hold = charge".to_string())
        } else {
            (
                format!("hold mouse or pad to charge (not {dash})"),
                "hold mouse = charge".to_string(),
            )
        };
        // The dash and the actives share a key only on a terminal without
        // lone-modifier reporting (the pre-Ctrl mapping). Say exactly that.
        let share = b.get(Action::Dash) == Binding::Key(KeyBind::Space)
            || (b.get(Action::Dash) == Binding::Key(KeyBind::Ctrl) && !self.lone_ctrl_hint);
        // The mouse's own jobs, read from the live bindings: a mouse player can
        // rebind either button, so the legend prints the button that really
        // fires the actives (the right click on the default tables) instead of
        // hard-coding `click`.
        let mouse_job = |a: Action| -> String {
            b.kbm_slots(a)
                .into_iter()
                .find_map(|slot| match slot {
                    Binding::Mouse(m) => Some(m.label().to_string()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        let mut mouse_jobs: Vec<String> = Vec::new();
        let dash_mouse = mouse_job(Action::Dash);
        let active_mouse = mouse_job(Action::FireActives);
        if !dash_mouse.is_empty() {
            mouse_jobs.push(format!("{dash_mouse} dash"));
        }
        if !active_mouse.is_empty() {
            mouse_jobs.push(format!("{active_mouse} actives"));
        }
        let click_job = mouse_jobs.join(" · ");
        // The segments that follow the jobs: empty (never a dangling dot) when
        // the scheme has no mouse button bound at all.
        let click_tail = if click_job.is_empty() {
            String::new()
        } else {
            format!("{click_job} · ")
        };
        let dash_line = if share {
            format!("WASD/mouse move · {dash} dash + actives · {click_tail}{pause} pause")
        } else if mouse_aim {
            // The two clicks are the dash and the actives already, so naming
            // them twice would only pad the line.
            format!(
                "WASD/arrows move · mouse aims · {dash} dash · {actives} actives · {pause} pause"
            )
        } else {
            format!(
                "WASD/mouse move · {dash} dash · {actives} actives · {click_tail}{pause} pause"
            )
        };
        self.title_wide = vec![
            dash_line,
            format!(
                "wheel or {} zoom · {} reset zoom · {} map · {} minimap · {} cursor · {} options · {} mute",
                key(Action::ZoomIn),
                key(Action::ResetZoom),
                key(Action::Map),
                key(Action::MinimapSize),
                key(Action::ToggleCursor),
                key(Action::Options),
                key(Action::Mute)
            ),
            format!(
                "{} {} music · {} {} effects · {} story · {} · {} quit · {graphics} graphics",
                key(Action::MusicDown),
                key(Action::MusicUp),
                key(Action::EffectsDown),
                key(Action::EffectsUp),
                key(Action::Library),
                charge_hint,
                key(Action::Quit)
            ),
        ];
        let narrow_first = if share {
            format!("WASD move · {dash} dash + actives · {narrow_charge}")
        } else {
            format!("WASD move · {dash} dash · {actives} actives · {narrow_charge}")
        };
        self.title_narrow = vec![
            narrow_first,
            format!("{click_tail}{pause} pause · {graphics} graphics"),
            format!(
                "wheel or {} zoom · {} reset · {} map · {} minimap",
                key(Action::ZoomIn),
                key(Action::ResetZoom),
                key(Action::Map),
                key(Action::MinimapSize)
            ),
            format!(
                "{} cursor · {} options · {} mute · [] - = volume",
                key(Action::ToggleCursor),
                key(Action::Options),
                key(Action::Mute)
            ),
        ];
        let mut pause_first =
            format!("WASD/mouse move · {dash} dash · {actives} actives");
        if !mouse_aim && !click_job.is_empty() {
            pause_first.push_str(" · ");
            pause_first.push_str(&click_job);
        }
        self.pause_wide = vec![
            pause_first,
            format!(
                "{} {} music · {} {} effects · {} options · {pause} unpause · {} quit · {graphics} graphics",
                key(Action::MusicDown),
                key(Action::MusicUp),
                key(Action::EffectsDown),
                key(Action::EffectsUp),
                key(Action::Options),
                key(Action::Quit)
            ),
        ];
        self.pause_narrow = vec![
            format!("move WASD · dash {dash} · actives {actives}"),
            format!("{click_tail}{pause} unpause"),
        ];
    }

    /// Set once at startup from `InputState::lone_ctrl`.
    pub fn set_lone_ctrl_hint(&mut self, on: bool) {
        self.lone_ctrl_hint = on;
        self.rebuild();
    }

    /// Does this terminal report a **lone modifier** press (the flag that also
    /// reports its release)? Where it does not, a held key cannot be seen, so
    /// hold-to-charge is the mouse's and the pad's job and the Controls screen
    /// says so.
    pub fn lone_ctrl_reported(&self) -> bool {
        self.lone_ctrl_hint
    }

    /// The Controls screen's terminal caveat, in one line: what can be bound
    /// here, and how the Super Dash's hold-to-charge is delivered.
    pub fn terminal_note(&self) -> &'static str {
        if self.lone_ctrl_hint {
            "Ctrl and Shift can be bound here · hold DASH to charge, release to fire"
        } else {
            "no lone Ctrl here · keys tap: charge the Super Dash with mouse or pad"
        }
    }

    pub fn title_legend_lines(&self, wide: bool) -> &[String] {
        if wide {
            &self.title_wide
        } else {
            &self.title_narrow
        }
    }

    pub fn pause_legend_lines(&self, wide: bool) -> &[String] {
        if wide {
            &self.pause_wide
        } else {
            &self.pause_narrow
        }
    }

    /// How the Controls screen writes one action's bindings.
    pub fn row_labels(&self, a: Action) -> (String, String) {
        let b = self.active();
        let mut kbm = String::new();
        let mut first = true;
        for slot in b.kbm_slots(a) {
            if slot.is_none() {
                continue;
            }
            if !first {
                kbm.push_str(" / ");
            }
            kbm.push_str(slot.label());
            first = false;
        }
        let pad = b.pad_of(a).label().to_string();
        (kbm, pad)
    }

    /// Saved form (what `scores.json` stores).
    pub fn to_saved(&self) -> SavedControls {
        let mut out = SavedControls {
            scheme: self.scheme.as_str().to_string(),
            schemes: [None, None, None],
            deadzone: self.deadzone,
            rumble: self.rumble,
        };
        for (i, set) in self.sets.iter().enumerate() {
            let mut s = SavedScheme::default();
            for a in ACTIONS {
                s.kbm.push(set.get(a).as_str());
                s.alt.push(set.alt_of(a).as_str());
                s.pad.push(set.pad_of(a).as_str());
            }
            out.schemes[i] = Some(s);
        }
        out
    }

    /// Build from the saved form, falling back to the defaults for anything
    /// missing or unreadable (an old `scores.json` loads with defaults).
    pub fn from_saved(saved: &SavedControls) -> Controls {
        let mut c = Controls {
            scheme: Scheme::parse(&saved.scheme).unwrap_or_default(),
            ..Controls::default()
        };
        for (i, scheme) in SCHEMES.iter().enumerate() {
            if let Some(s) = saved.schemes[i].as_ref() {
                let mut set = Bindings::defaults(*scheme);
                for a in ACTIONS {
                    let idx = a.index();
                    // A hand-edited `pad:…` in a keyboard/mouse slot could never
                    // fire and would print a pad glyph in the keyboard legend:
                    // keep the default instead (the pad slot is filtered below).
                    let kbm_ok = |b: &Binding| !matches!(b, Binding::Pad(_));
                    if let Some(raw) = s.kbm.get(idx).and_then(|v| Binding::parse(v)) {
                        if kbm_ok(&raw) {
                            set.kbm[idx] = raw;
                        }
                    }
                    if let Some(raw) = s.alt.get(idx).and_then(|v| Binding::parse(v)) {
                        if kbm_ok(&raw) {
                            set.alt[idx] = raw;
                        }
                    }
                    if let Some(raw) = s.pad.get(idx).and_then(|v| Binding::parse(v)) {
                        if matches!(raw, Binding::Pad(_) | Binding::None) {
                            set.pad[idx] = raw;
                        }
                    }
                }
                // Batch 12 moved the actives' alternate off the left click
                // (which stays the dash's) onto the right click. A file that
                // still holds the exact old stock pairing was never edited by
                // the player — left click was both the dash's and the actives'
                // second button — so it takes the new default. A file where
                // either slot says anything else is the player's own choice and
                // is left alone.
                let stock = Bindings::defaults(*scheme);
                let left = Binding::Mouse(MouseBind::Left);
                let fire_alt = Action::FireActives.index();
                let right = stock.alt[fire_alt];
                // Never double-bind: a player who already gave the right
                // click to another action keeps it there.
                let right_taken = ACTIONS.into_iter().any(|o| {
                    o != Action::FireActives && set.kbm_slots(o).contains(&right)
                });
                if *scheme != Scheme::MouseAim
                    && set.alt[Action::Dash.index()] == left
                    && set.alt[fire_alt] == left
                    && right != left
                    && !right_taken
                {
                    set.alt[fire_alt] = stock.alt[fire_alt];
                }
                // An action newer than the file takes its default key only
                // when the player has not already given that key to something
                // else (for example, G for Graphics in a 26-action save);
                // otherwise it starts unbound, never double-bound. A key
                // an older action holds *by default* is not the player's doing:
                // the defaults share it on purpose (Esc is Pause and Back), so
                // it does not count as taken.
                for a in ACTIONS {
                    let idx = a.index();
                    if idx < s.kbm.len() {
                        continue;
                    }
                    let fresh = set.kbm[idx];
                    let taken = ACTIONS.into_iter().any(|o| {
                        o.index() < s.kbm.len()
                            && set.kbm_slots(o).contains(&fresh)
                            && !stock.kbm_slots(o).contains(&fresh)
                    });
                    if taken && !fresh.is_none() {
                        set.kbm[idx] = Binding::None;
                    }
                }
                c.sets[i] = set;
            }
        }
        c.deadzone = if saved.deadzone.is_finite() && saved.deadzone > 0.0 {
            saved.deadzone.clamp(0.05, 0.9)
        } else {
            // An old file has no dead zone: use the default, not zero.
            DEFAULT_DEADZONE
        };
        c.rumble = saved.rumble;
        c.rebuild();
        c
    }
}

/// The saved form of [`Controls`] (the fields `scores.rs` persists).
#[derive(Clone, Debug, Default)]
pub struct SavedControls {
    pub scheme: String,
    pub schemes: [Option<SavedScheme>; 3],
    pub deadzone: f32,
    pub rumble: bool,
}

/// Is `v` a unit-ish direction worth aiming with?
fn aim_dir(v: Vec2) -> Option<Vec2> {
    if v.len_sq() > 1e-4 {
        Some(v.normalized())
    } else {
        None
    }
}

/// How far in front of the rat a right-stick aim point sits, in world pixels.
pub const PAD_AIM_DISTANCE: f32 = 140.0;

/// The aim point from a right-stick vector: `player_pos + dir * distance`, or
/// `None` while the stick is centred (auto-aim).
pub fn pad_aim_point(origin: Vec2, stick: Vec2) -> Option<Vec2> {
    aim_dir(stick).map(|d| origin + d * PAD_AIM_DISTANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn schemes_parse_step_and_name() {
        for s in SCHEMES {
            assert_eq!(Scheme::parse(s.as_str()), Some(s));
            assert_eq!(Scheme::parse(&s.name().to_lowercase()), Some(s));
        }
        assert_eq!(Scheme::Classic.step(1), Scheme::MouseAim);
        assert_eq!(Scheme::MouseAim.step(1), Scheme::Gamepad);
        assert_eq!(Scheme::Gamepad.step(1), Scheme::Classic);
        assert_eq!(Scheme::Classic.step(-1), Scheme::Gamepad);
        assert_eq!(Scheme::parse("nonsense"), None);
        assert_eq!(Scheme::default(), Scheme::Classic);
    }

    #[test]
    fn bindings_round_trip_through_their_saved_spelling() {
        for b in [
            Binding::Key(KeyBind::Char('w')),
            Binding::Key(KeyBind::Ctrl),
            Binding::Key(KeyBind::F(5)),
            Binding::Key(KeyBind::Space),
            Binding::Mouse(MouseBind::Right),
            Binding::Mouse(MouseBind::WheelDown),
            Binding::Pad(PadBind::R3),
            Binding::None,
        ] {
            assert_eq!(Binding::parse(&b.as_str()), Some(b), "{:?}", b);
        }
        for k in [KeyBind::Char('q'), KeyBind::Tab, KeyBind::Alt, KeyBind::PageDown] {
            assert_eq!(KeyBind::parse(&k.as_str()), Some(k));
        }
    }

    #[test]
    fn a_conflict_swaps_the_two_actions() {
        let mut b = Bindings::defaults(Scheme::Classic);
        // Binding Space (the actives key) to Dash must hand the actives the old
        // dash binding, not silently drop it.
        let moved = b.bind_kbm(Action::Dash, Binding::Key(KeyBind::Space));
        assert_eq!(moved, Some(Action::FireActives));
        assert_eq!(b.get(Action::FireActives), Binding::Key(KeyBind::Ctrl));
        assert_eq!(b.get(Action::Dash), Binding::Key(KeyBind::Space));

        // A pad conflict swaps too.
        let moved = b.bind_pad(Action::Pause, Binding::Pad(PadBind::South));
        assert_eq!(moved, Some(Action::Confirm));
        assert_eq!(b.pad_of(Action::Confirm), Binding::Pad(PadBind::Start));
        assert_eq!(b.pad_of(Action::Pause), Binding::Pad(PadBind::South));
    }

    #[test]
    fn binding_an_action_to_its_own_alternate_swaps_the_two_slots() {
        let mut b = Bindings::defaults(Scheme::MouseAim);
        // Dash: right click, alternate Ctrl. Asking for Ctrl as the primary
        // must not leave Ctrl in both slots and lose the right click.
        let moved = b.bind_kbm(Action::Dash, Binding::Key(KeyBind::Ctrl));
        assert_eq!(moved, None, "no other action was touched");
        assert_eq!(b.get(Action::Dash), Binding::Key(KeyBind::Ctrl));
        assert_eq!(b.alt_of(Action::Dash), Binding::Mouse(MouseBind::Right));
        // Rebinding to the binding already held is a no-op.
        let before = b.kbm;
        assert_eq!(b.bind_kbm(Action::Dash, Binding::Key(KeyBind::Ctrl)), None);
        assert_eq!(b.kbm, before);
    }

    #[test]
    fn a_shared_default_stays_shared_after_a_swap() {
        let mut b = Bindings::defaults(Scheme::Classic);
        // Esc is Pause *and* Back by default; taking it for the map hands both
        // the map's old key, so neither is left unbound or split.
        b.bind_kbm(Action::Map, Binding::Key(KeyBind::Esc));
        assert_eq!(b.get(Action::Map), Binding::Key(KeyBind::Esc));
        assert_eq!(b.get(Action::Pause), Binding::Key(KeyBind::Tab));
        assert_eq!(b.get(Action::Back), Binding::Key(KeyBind::Tab));
        assert_eq!(b.alt_of(Action::Pause), Binding::Key(KeyBind::Char('p')));
    }

    #[test]
    fn the_legends_follow_the_device_without_a_rebinding() {
        let mut c = Controls::default();
        c.set_lone_ctrl_hint(false);
        assert_eq!(c.live_dash_label(), "Space");
        assert_eq!(c.active_label(), "Space");
        // The pad takes over: no rebuild call, the note alone must do it.
        c.note_device(Device::Pad);
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("RB dash"), "{title}");
        assert_eq!(c.live_dash_label(), "RB", "a pad dash is never renamed Space");
        assert_eq!(c.active_label(), "X");
        // Keyboard and mouse share one legend; back from the pad restores it.
        c.note_device(Device::Mouse);
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("WASD"), "{title}");
        assert_eq!(c.live_dash_label(), "Space");
    }

    #[test]
    fn hand_edited_saves_are_normalised() {
        // One spelling per key, so a conflict with Space is still a conflict.
        assert_eq!(Binding::parse("key: "), Some(Binding::Key(KeyBind::Space)));
        assert_eq!(Binding::parse(" "), Some(Binding::Key(KeyBind::Space)));
        assert_eq!(KeyBind::Char('7').label(), "7");

        // A pad button written into a keyboard slot keeps the default.
        let mut saved = Controls::default().to_saved();
        let scheme = saved.schemes[0].as_mut().unwrap();
        scheme.kbm[Action::Mute.index()] = "pad:south".into();
        scheme.alt[Action::Pause.index()] = "pad:east".into();
        scheme.pad[Action::Mute.index()] = "key:m".into();
        let c = Controls::from_saved(&saved);
        assert_eq!(c.active().get(Action::Mute), Binding::Key(KeyBind::Char('m')));
        assert_eq!(c.active().alt_of(Action::Pause), Binding::Key(KeyBind::Char('p')));
        assert_eq!(c.active().pad_of(Action::Mute), Binding::Pad(PadBind::L1));
    }

    // ---- Properties over the whole table ---------------------------------

    fn same_table(a: &Bindings, b: &Bindings) -> bool {
        a.kbm == b.kbm && a.alt == b.alt && a.pad == b.pad
    }

    /// How many keyboard/mouse slots of the whole table hold `x`.
    fn kbm_holders(b: &Bindings, x: Binding) -> usize {
        ACTIONS
            .into_iter()
            .flat_map(|a| b.kbm_slots(a))
            .filter(|slot| *slot == x)
            .count()
    }

    #[test]
    fn the_tables_are_indexed_by_position() {
        for (i, a) in ACTIONS.into_iter().enumerate() {
            assert_eq!(a.index(), i, "{a:?}");
        }
        for (i, p) in PAD_BINDS.into_iter().enumerate() {
            assert_eq!(p.index(), i, "{p:?}");
            assert_eq!(PadBind::parse(p.as_str()), Some(p));
        }
        for (i, sch) in SCHEMES.into_iter().enumerate() {
            assert_eq!(sch.index(), i);
            assert_eq!(Scheme::parse(sch.as_str()), Some(sch));
            assert_eq!(Scheme::parse(sch.name()), Some(sch), "the display name parses too");
        }
    }

    #[test]
    fn every_action_has_a_label_in_every_scheme() {
        // The action column: non-empty, distinct, and short enough for the
        // Controls screen's column.
        for a in ACTIONS {
            assert!(!a.label().is_empty(), "{a:?}");
            assert!(a.label().chars().count() <= 14, "{a:?}: {}", a.label());
            for other in ACTIONS {
                assert!(a == other || a.label() != other.label(), "{a:?} / {other:?}");
            }
        }
        for scheme in SCHEMES {
            let mut c = Controls::default();
            c.set_scheme(scheme);
            for device in [Device::Keyboard, Device::Mouse, Device::Pad] {
                c.note_device(device);
                for a in ACTIONS {
                    // Every slot prints something real: never empty, never the
                    // "KEY" / "F" placeholders of an unnamed key.
                    for slot in c
                        .active()
                        .kbm_slots(a)
                        .into_iter()
                        .chain([c.active().pad_of(a)])
                    {
                        let label = slot.label();
                        assert!(!label.is_empty(), "{scheme:?} {a:?}");
                        assert!(label != "KEY" && label != "F", "{scheme:?} {a:?}: {label}");
                    }
                    assert!(!c.action_label(a).is_empty(), "{scheme:?} {a:?}");
                    let (kbm, pad) = c.row_labels(a);
                    assert!(!pad.is_empty(), "{scheme:?} {a:?}");
                    // An action with a keyboard/mouse binding names it in its row.
                    if !c.active().get(a).is_none() || !c.active().alt_of(a).is_none() {
                        assert!(!kbm.is_empty(), "{scheme:?} {a:?}");
                    }
                }
                // Every legend line is real text.
                for wide in [true, false] {
                    for line in c.title_legend_lines(wide).iter().chain(c.pause_legend_lines(wide)) {
                        assert!(!line.trim().is_empty(), "{scheme:?} {device:?}");
                    }
                    assert!(!c.title_legend_lines(wide).is_empty());
                    assert!(!c.pause_legend_lines(wide).is_empty());
                }
                assert!(!c.live_dash_label().is_empty());
                assert!(!c.active_label().is_empty());
            }
        }
    }

    #[test]
    fn every_default_binding_round_trips_through_its_saved_spelling() {
        for scheme in SCHEMES {
            let b = Bindings::defaults(scheme);
            for a in ACTIONS {
                for slot in [b.get(a), b.alt_of(a), b.pad_of(a)] {
                    assert_eq!(Binding::parse(&slot.as_str()), Some(slot), "{scheme:?} {a:?}");
                }
            }
        }
        // Every key spelling, not just the defaults.
        let mut keys = vec![
            KeyBind::Up, KeyBind::Down, KeyBind::Left, KeyBind::Right, KeyBind::Home,
            KeyBind::End, KeyBind::PageUp, KeyBind::PageDown, KeyBind::Enter, KeyBind::Esc,
            KeyBind::Tab, KeyBind::Space, KeyBind::Backspace, KeyBind::Delete, KeyBind::Insert,
            KeyBind::Ctrl, KeyBind::Shift, KeyBind::Alt,
        ];
        keys.extend((1..=12).map(KeyBind::F));
        keys.extend(('a'..='z').map(KeyBind::Char));
        keys.extend(('0'..='9').map(KeyBind::Char));
        keys.extend("-=[],./;'\\`".chars().map(KeyBind::Char));
        for k in keys {
            assert_eq!(KeyBind::parse(&k.as_str()), Some(k), "{k:?}");
            let label = k.label();
            assert!(!label.is_empty() && label != "KEY", "{k:?}: {label}");
            assert!(label != "F" || k == KeyBind::Char('f'), "{k:?}: {label}");
        }
        for m in [
            MouseBind::Left, MouseBind::Right, MouseBind::Middle,
            MouseBind::WheelUp, MouseBind::WheelDown,
        ] {
            assert_eq!(MouseBind::parse(m.as_str()), Some(m));
        }
    }

    #[test]
    fn the_defaults_share_only_the_documented_bindings() {
        for scheme in SCHEMES {
            let b = Bindings::defaults(scheme);
            for a in ACTIONS {
                for slot in b.kbm_slots(a) {
                    if slot.is_none() {
                        continue;
                    }
                    let holders = kbm_holders(&b, slot);
                    // `Esc` is Pause *and* Back: the one pair the defaults
                    // share on purpose (the click's two jobs were split in
                    // batch 12: left dashes, right fires the actives).
                    let shared_on_purpose = slot == Binding::Key(KeyBind::Esc);
                    if shared_on_purpose {
                        assert_eq!(holders, 2, "{scheme:?} {a:?} {slot:?}");
                    } else {
                        assert_eq!(holders, 1, "{scheme:?}: {slot:?} is double-bound ({a:?})");
                    }
                }
                // No pad button does two jobs.
                let pad = b.pad_of(a);
                if !pad.is_none() {
                    let n = ACTIONS.into_iter().filter(|o| b.pad_of(*o) == pad).count();
                    assert_eq!(n, 1, "{scheme:?}: {pad:?} is double-bound ({a:?})");
                }
            }
        }
    }

    /// The owner's mapping: **right click fires the actives**. Classic and
    /// Gamepad keep Space as the primary and the left click as the dash
    /// (alternate of `Dash`), and Mouse aim keeps its split — right dashes,
    /// left fires.
    #[test]
    fn classic_and_gamepad_fire_the_actives_on_the_right_click() {
        for scheme in [Scheme::Classic, Scheme::Gamepad] {
            let b = Bindings::defaults(scheme);
            assert_eq!(b.get(Action::FireActives), Binding::Key(KeyBind::Space), "{scheme:?}");
            assert_eq!(
                b.alt_of(Action::FireActives),
                Binding::Mouse(MouseBind::Right),
                "{scheme:?}: the right click does not fire the actives"
            );
            assert_eq!(b.get(Action::Dash), Binding::Key(KeyBind::Ctrl), "{scheme:?}");
            assert_eq!(
                b.alt_of(Action::Dash),
                Binding::Mouse(MouseBind::Left),
                "{scheme:?}: the left click must keep its dash"
            );
        }
        // Mouse aim is untouched: right dashes, left fires.
        let b = Bindings::defaults(Scheme::MouseAim);
        assert_eq!(b.get(Action::Dash), Binding::Mouse(MouseBind::Right));
        assert_eq!(b.alt_of(Action::FireActives), Binding::Key(KeyBind::Space));
        assert_eq!(b.get(Action::FireActives), Binding::Mouse(MouseBind::Left));
    }

    /// Every legend that names the pointer names the button that really does
    /// the job, in all three schemes.
    #[test]
    fn the_legends_name_the_right_click_as_the_actives() {
        for scheme in SCHEMES {
            let mut c = Controls::default();
            c.set_scheme(scheme);
            c.set_lone_ctrl_hint(true);
            c.note_device(Device::Keyboard);
            for wide in [true, false] {
                let lines = c
                    .title_legend_lines(wide)
                    .iter()
                    .chain(c.pause_legend_lines(wide))
                    .cloned()
                    .collect::<Vec<String>>()
                    .join(" ");
                // Whichever mouse button fires the actives, the legend names
                // it with the word `actives` (and the dash's button with
                // `dash`), so a rebind or a scheme change cannot leave the old
                // wording behind.
                for (action, verb) in [(Action::FireActives, "actives"), (Action::Dash, "dash")] {
                    let label = c.active().kbm_slots(action).into_iter().find_map(|slot| match slot {
                        Binding::Mouse(m) => Some(m.label()),
                        _ => None,
                    });
                    let Some(label) = label else {
                        continue;
                    };
                    assert!(
                        lines.contains(&format!("{label} {verb}")),
                        "{scheme:?} wide={wide}: {label} {verb} is not named: {lines}"
                    );
                }
                assert!(!lines.contains("click ="), "{scheme:?}: the vague `click` is back: {lines}");
            }
        }
    }

    /// A save written before batch 12 still holds the old stock pairing (the
    /// left click in both the dash's and the actives' alternate slot). It is
    /// upgraded to the right click, because nobody chose that pairing — it was
    /// the default.
    #[test]
    fn an_untouched_old_save_gains_the_right_click() {
        let mut saved = Controls::default().to_saved();
        for scheme in saved.schemes.iter_mut().flatten() {
            scheme.alt[Action::FireActives.index()] = "mouse:left".into();
            scheme.alt[Action::Dash.index()] = "mouse:left".into();
        }
        let c = Controls::from_saved(&saved);
        for scheme in SCHEMES {
            if scheme == Scheme::MouseAim {
                continue;
            }
            let b = &c.sets[scheme.index()];
            assert_eq!(b.alt_of(Action::Dash), Binding::Mouse(MouseBind::Left), "{scheme:?}");
            assert_eq!(
                b.alt_of(Action::FireActives),
                Binding::Mouse(MouseBind::Right),
                "{scheme:?}: an untouched old save kept the left click for the actives"
            );
        }
    }

    /// A binding the player really did change by hand wins: only the exact old
    /// stock pairing is migrated.
    #[test]
    fn a_hand_changed_save_keeps_its_own_mouse_bindings() {
        let mut saved = Controls::default().to_saved();
        {
            let scheme = saved.schemes[Scheme::Classic.index()].as_mut().unwrap();
            scheme.alt[Action::FireActives.index()] = "mouse:middle".into();
            scheme.alt[Action::Dash.index()] = "mouse:left".into();
        }
        let c = Controls::from_saved(&saved);
        assert_eq!(
            c.active().alt_of(Action::FireActives),
            Binding::Mouse(MouseBind::Middle),
            "a hand-picked actives button was overwritten"
        );
        // …and a hand-picked dash button clears the migration on its own.
        let mut saved = Controls::default().to_saved();
        {
            let scheme = saved.schemes[Scheme::Gamepad.index()].as_mut().unwrap();
            scheme.alt[Action::FireActives.index()] = "mouse:left".into();
            scheme.alt[Action::Dash.index()] = "mouse:right".into();
        }
        let c = Controls::from_saved(&saved);
        assert_eq!(
            c.sets[Scheme::Gamepad.index()].alt_of(Action::FireActives),
            Binding::Mouse(MouseBind::Left),
            "the actives' own button was migrated although the dash was rebound"
        );
        // Mouse aim is never migrated: its right click is the dash.
        let mut saved = Controls::default().to_saved();
        {
            let scheme = saved.schemes[Scheme::MouseAim.index()].as_mut().unwrap();
            scheme.alt[Action::FireActives.index()] = "mouse:left".into();
            scheme.alt[Action::Dash.index()] = "mouse:left".into();
        }
        let c = Controls::from_saved(&saved);
        assert_eq!(
            c.sets[Scheme::MouseAim.index()].alt_of(Action::FireActives),
            Binding::Mouse(MouseBind::Left),
            "mouse aim's saved table was rewritten"
        );
    }

    /// Swap is an involution: taking another action's binding and then taking
    /// your own back restores the whole table. (It holds whenever the binding
    /// you started with is yours alone — a deliberately shared one, like Esc,
    /// is handed to everyone who held the other, by design.)
    #[test]
    fn a_keyboard_swap_undone_restores_the_table() {
        for scheme in SCHEMES {
            let start = Bindings::defaults(scheme);
            for a in ACTIONS {
                let x = start.get(a);
                if x.is_none() || kbm_holders(&start, x) != 1 {
                    continue;
                }
                for other in ACTIONS {
                    for y in start.kbm_slots(other) {
                        if y.is_none() || y == x {
                            continue;
                        }
                        let mut b = start;
                        let moved = b.bind_kbm(a, y);
                        assert_eq!(b.get(a), y);
                        if other != a {
                            assert!(moved.is_some(), "{scheme:?} {a:?} <- {y:?}");
                            assert!(
                                b.kbm_slots(other).contains(&x),
                                "{scheme:?}: {other:?} must receive {x:?}"
                            );
                        }
                        // Nothing is lost: both bindings are still on the table.
                        assert!(kbm_holders(&b, x) >= 1, "{scheme:?} {a:?}: {x:?} dropped");
                        b.bind_kbm(a, x);
                        assert!(
                            same_table(&b, &start),
                            "{scheme:?}: {a:?} <- {y:?} <- {x:?} did not restore the table"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_pad_swap_undone_restores_the_table() {
        for scheme in SCHEMES {
            let start = Bindings::defaults(scheme);
            for a in ACTIONS {
                let x = start.pad_of(a);
                if x.is_none() {
                    continue;
                }
                for p in PAD_BINDS {
                    let y = Binding::Pad(p);
                    if y == x {
                        continue;
                    }
                    let holder = start.action_for_pad(y);
                    let mut b = start;
                    assert_eq!(b.bind_pad(a, y), holder, "{scheme:?} {a:?} <- {y:?}");
                    assert_eq!(b.pad_of(a), y);
                    if let Some(h) = holder {
                        assert_eq!(b.pad_of(h), x);
                    }
                    b.bind_pad(a, x);
                    assert!(same_table(&b, &start), "{scheme:?} {a:?} <- {y:?} <- {x:?}");
                }
            }
        }
    }

    #[test]
    fn a_swap_never_touches_the_other_schemes_or_a_third_action() {
        let mut c = Controls::default();
        let before = c.sets;
        c.bind_active_kbm(Action::Mute, Binding::Key(KeyBind::Char('n')));
        for (i, scheme) in SCHEMES.into_iter().enumerate() {
            if scheme != c.scheme() {
                assert!(same_table(&c.sets[i], &before[i]), "{scheme:?} changed");
            }
        }
        let now = c.active();
        for a in ACTIONS {
            if a != Action::Mute && a != Action::MinimapSize {
                assert_eq!(now.kbm_slots(a), before[0].kbm_slots(a), "{a:?}");
            }
            assert_eq!(now.pad_of(a), before[0].pad_of(a), "{a:?}");
        }
    }

    /// A save of today's shape reads back exactly, for every scheme and after
    /// rebinding, unbinding and tuning.
    #[test]
    fn the_current_save_shape_round_trips_exactly() {
        let mut c = Controls::default();
        for scheme in SCHEMES {
            c.set_scheme(scheme);
            c.bind_active_kbm(Action::Dash, Binding::Key(KeyBind::Char('v')));
            c.bind_active_kbm(Action::Map, Binding::Mouse(MouseBind::WheelUp));
            c.bind_active_pad(Action::Pause, Binding::Pad(PadBind::South));
            c.unbind_active_kbm(Action::Library);
        }
        c.deadzone = 0.4;
        c.rumble = true;
        let saved = c.to_saved();
        for s in saved.schemes.iter().flatten() {
            assert_eq!(s.kbm.len(), ACTION_COUNT);
            assert_eq!(s.alt.len(), ACTION_COUNT);
            assert_eq!(s.pad.len(), ACTION_COUNT);
        }
        let back = Controls::from_saved(&saved);
        assert_eq!(back.scheme(), c.scheme());
        for i in 0..SCHEMES.len() {
            assert!(same_table(&back.sets[i], &c.sets[i]), "scheme {i}");
        }
        assert!((back.deadzone - 0.4).abs() < 1e-6 && back.rumble);
        // And the second generation is byte-identical to the first.
        let again = back.to_saved();
        assert_eq!(again.scheme, saved.scheme);
        for i in 0..SCHEMES.len() {
            let (a, b) = (again.schemes[i].as_ref().unwrap(), saved.schemes[i].as_ref().unwrap());
            assert_eq!((&a.kbm, &a.alt, &a.pad), (&b.kbm, &b.alt, &b.pad), "scheme {i}");
        }
        assert_eq!(back.title_legend_lines(true), c.title_legend_lines(true));
    }

    /// Every older shape of the save: the binding arrays were shorter when
    /// fewer actions existed. Whatever the length, the saved prefix is kept
    /// and the actions the file has never heard of load at their defaults.
    #[test]
    fn every_older_save_length_loads_at_the_defaults() {
        let full = Controls::default().to_saved();
        for n in 0..=ACTION_COUNT {
            let mut saved = full.clone();
            for s in saved.schemes.iter_mut().flatten() {
                s.kbm.truncate(n);
                s.alt.truncate(n);
                s.pad.truncate(n);
            }
            let c = Controls::from_saved(&saved);
            for (i, scheme) in SCHEMES.into_iter().enumerate() {
                assert!(
                    same_table(&c.sets[i], &Bindings::defaults(scheme)),
                    "a {n}-action save of {scheme:?} must load as the defaults"
                );
            }
            // Re-saving upgrades the file to today's length.
            for s in c.to_saved().schemes.iter().flatten() {
                assert_eq!(s.kbm.len(), ACTION_COUNT, "from a {n}-action save");
            }
        }
        // The three arrays did not always grow together.
        let mut ragged = full.clone();
        for s in ragged.schemes.iter_mut().flatten() {
            s.alt.clear();
            s.pad.truncate(5);
        }
        let c = Controls::from_saved(&ragged);
        for (i, scheme) in SCHEMES.into_iter().enumerate() {
            assert!(same_table(&c.sets[i], &Bindings::defaults(scheme)), "{scheme:?}");
        }
    }

    /// The migration rule, for every cut point and every newer action: if the
    /// player had already given that action's default key to something the
    /// file knows, the new action starts unbound — never double-bound — and
    /// the player's own choice is untouched.
    #[test]
    fn a_newer_action_never_takes_a_key_the_old_save_already_uses() {
        for (i, scheme) in SCHEMES.into_iter().enumerate() {
            let defaults = Bindings::defaults(scheme);
            for n in 1..ACTION_COUNT {
                for newer in ACTIONS.into_iter().filter(|a| a.index() >= n) {
                    let wanted = defaults.get(newer);
                    if wanted.is_none() {
                        continue;
                    }
                    // The player bound the newer action's default key to the
                    // file's last known action, then saved (before `newer`
                    // existed).
                    let holder = ACTIONS[n - 1];
                    if defaults.kbm_slots(holder).contains(&wanted) {
                        // A deliberately shared default (Esc): not a conflict.
                        continue;
                    }
                    let mut old = defaults;
                    old.kbm[holder.index()] = wanted;
                    let mut c = Controls::default();
                    c.sets[i] = old;
                    let mut saved = c.to_saved();
                    for s in saved.schemes.iter_mut().flatten() {
                        s.kbm.truncate(n);
                        s.alt.truncate(n);
                        s.pad.truncate(n);
                    }
                    let loaded = Controls::from_saved(&saved);
                    let set = &loaded.sets[i];
                    assert_eq!(set.get(holder), wanted, "{scheme:?} n={n}: the player's key moved");
                    assert!(
                        set.get(newer).is_none(),
                        "{scheme:?} n={n}: {newer:?} double-binds {wanted:?} with {holder:?}"
                    );
                    // Every other newer action still gets its default.
                    for a in ACTIONS.into_iter().filter(|a| a.index() >= n && *a != newer) {
                        if defaults.get(a) != wanted {
                            assert_eq!(set.get(a), defaults.get(a), "{scheme:?} n={n} {a:?}");
                        }
                    }
                }
            }
        }
    }

    /// Shapes no version ever wrote, which a hand edit or a damaged file can
    /// still produce: nothing panics and everything unreadable is a default.
    #[test]
    fn damaged_saves_load_as_defaults_without_panicking() {
        let defaults = |i: usize| Bindings::defaults(SCHEMES[i]);

        // No scheme blocks at all (the oldest saves had no controls).
        let c = Controls::from_saved(&SavedControls::default());
        assert_eq!(c.scheme(), Scheme::Classic);
        assert!((c.deadzone - DEFAULT_DEADZONE).abs() < 1e-6 && !c.rumble);
        for (i, set) in c.sets.iter().enumerate().take(3) {
            assert!(same_table(set, &defaults(i)));
        }

        // Garbage in every slot, an unknown scheme, impossible dead zones.
        for junk in ["", "???", "key:", "key:ww", "mouse:fifth", "pad:paddle", "f13", "KEY", "🐀🐀"] {
            for deadzone in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1e9] {
                let saved = SavedControls {
                    scheme: junk.to_string(),
                    schemes: std::array::from_fn(|_| {
                        Some(SavedScheme {
                            kbm: vec![junk.to_string(); ACTION_COUNT],
                            alt: vec![junk.to_string(); ACTION_COUNT],
                            pad: vec![junk.to_string(); ACTION_COUNT],
                        })
                    }),
                    deadzone,
                    rumble: false,
                };
                let c = Controls::from_saved(&saved);
                assert_eq!(c.scheme(), Scheme::Classic, "{junk:?}");
                assert!((0.05..=0.9).contains(&c.deadzone), "{deadzone} -> {}", c.deadzone);
                for (i, set) in c.sets.iter().enumerate().take(3) {
                    assert!(same_table(set, &defaults(i)), "{junk:?} scheme {i}");
                }
            }
        }

        // A file from a *newer* build: more entries than this build has
        // actions. The extras are ignored.
        let mut long = Controls::default().to_saved();
        for s in long.schemes.iter_mut().flatten() {
            for v in [&mut s.kbm, &mut s.alt, &mut s.pad] {
                v.extend(["key:j".to_string(), "pad:south".to_string(), "none".to_string()]);
            }
        }
        let c = Controls::from_saved(&long);
        for (i, set) in c.sets.iter().enumerate().take(3) {
            assert!(same_table(set, &defaults(i)), "scheme {i}");
        }

        // One scheme block missing: only that scheme falls back.
        let mut c = Controls::default();
        c.bind_active_kbm(Action::Mute, Binding::Key(KeyBind::Char('v')));
        let mut partial = c.to_saved();
        partial.schemes[1] = None;
        let back = Controls::from_saved(&partial);
        assert_eq!(back.sets[0].get(Action::Mute), Binding::Key(KeyBind::Char('v')));
        assert!(same_table(&back.sets[1], &defaults(1)));
    }

    #[test]
    fn binding_an_unbound_action_never_strands_pause_back_or_confirm() {
        for scheme in SCHEMES {
            // Esc is Pause and Back. Give it to an action that has no key.
            let mut b = Bindings::defaults(scheme);
            b.unbind_kbm(Action::Mute);
            let moved = b.bind_kbm(Action::Mute, Binding::Key(KeyBind::Esc));
            assert_eq!(b.get(Action::Mute), Binding::Key(KeyBind::Esc));
            // Pause has its alternate, so it gives Esc up...
            assert_eq!(moved, Some(Action::Pause), "{scheme:?}");
            assert!(b.get(Action::Pause).is_none());
            assert_eq!(b.alt_of(Action::Pause), Binding::Key(KeyBind::Char('p')));
            // ...Back has nothing else, so it keeps Esc and shares it.
            assert_eq!(b.get(Action::Back), Binding::Key(KeyBind::Esc), "{scheme:?}");

            // Confirm's only key is Enter.
            let mut b = Bindings::defaults(scheme);
            b.unbind_kbm(Action::Mute);
            assert_eq!(b.bind_kbm(Action::Mute, Binding::Key(KeyBind::Enter)), None);
            assert_eq!(b.get(Action::Confirm), Binding::Key(KeyBind::Enter));

            // An ordinary holder is still stripped: no silent double-binding.
            let mut b = Bindings::defaults(scheme);
            b.unbind_kbm(Action::Mute);
            let moved = b.bind_kbm(Action::Mute, Binding::Key(KeyBind::Char('n')));
            assert_eq!(moved, Some(Action::MinimapSize));
            assert!(b.get(Action::MinimapSize).is_none());

            // And a real swap (the action had a key) still moves the key off
            // the critical actions, because they receive one in return.
            let mut b = Bindings::defaults(scheme);
            b.bind_kbm(Action::Mute, Binding::Key(KeyBind::Esc));
            assert_eq!(b.get(Action::Back), Binding::Key(KeyBind::Char('m')));
            assert_eq!(b.get(Action::Pause), Binding::Key(KeyBind::Char('m')));

            // Whatever single rebind is made, none of the three ends keyless.
            for target in ACTIONS {
                for holder in [Action::Pause, Action::Back, Action::Confirm] {
                    for key in Bindings::defaults(scheme).kbm_slots(holder) {
                        if key.is_none() {
                            continue;
                        }
                        let mut b = Bindings::defaults(scheme);
                        b.unbind_kbm(target);
                        b.bind_kbm(target, key);
                        for a in [Action::Pause, Action::Back, Action::Confirm] {
                            if a == target {
                                continue;
                            }
                            assert!(
                                b.kbm_slots(a).iter().any(|s| !s.is_none()),
                                "{scheme:?}: {target:?} <- {key:?} left {a:?} with no key"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rebinding_one_action_leaves_the_others_alone() {
        let mut b = Bindings::defaults(Scheme::MouseAim);
        b.bind_kbm(Action::Mute, Binding::Key(KeyBind::Char('v')));
        assert_eq!(b.get(Action::Mute), Binding::Key(KeyBind::Char('v')));
        assert_eq!(b.get(Action::MoveUp), Binding::Key(KeyBind::Char('w')));
        b.unbind_kbm(Action::Mute);
        assert!(b.get(Action::Mute).is_none());
    }

    #[test]
    fn unbindable_keys_are_refused() {
        let cmd = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::SUPER);
        assert_eq!(KeyBind::from_event(&cmd), Err(Unbindable::Super));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(KeyBind::from_event(&ctrl_c), Err(Unbindable::CtrlC));
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(KeyBind::from_event(&ctrl_w), Err(Unbindable::Combo));
        // A lone modifier is fine.
        let lone = KeyEvent::new(
            KeyCode::Modifier(ModifierKeyCode::LeftControl),
            KeyModifiers::CONTROL,
        );
        assert_eq!(KeyBind::from_event(&lone), Ok(KeyBind::Ctrl));
    }

    #[test]
    fn a_binding_matches_its_event_but_not_a_combo_or_a_release() {
        let w = KeyBind::Char('w');
        assert!(w.matches(&key('w')));
        assert!(w.matches(&KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT)));
        assert!(!w.matches(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)));
        assert!(!w.matches(&KeyEvent::new_with_kind(
            KeyCode::Char('w'),
            KeyModifiers::NONE,
            KeyEventKind::Release
        )));
        assert!(KeyBind::Ctrl.matches(&KeyEvent::new(
            KeyCode::Modifier(ModifierKeyCode::RightControl),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn defaults_name_the_classic_controls_the_legends_promise() {
        let mut c = Controls::default();
        c.set_lone_ctrl_hint(true);
        c.note_device(Device::Keyboard);
        c.rebuild();
        let title = c.title_legend_lines(true).join(" ");
        for needle in [
            "move",
            "Ctrl dash",
            "dash",
            "actives",
            "LMB dash",
            "RMB actives",
            "zoom",
            "reset zoom",
            "Tab map",
            "N minimap",
            "C cursor",
            "O options",
            "M mute",
            "[ ] music",
            "- = effects",
            "L story",
            "pause",
            "Q quit",
            "G graphics",
        ] {
            assert!(title.contains(needle), "title legend misses {needle:?}: {title}");
        }
        // The paused legend names the live control set, not a stale one.
        let pause = c.pause_legend_lines(true).join(" ");
        for needle in ["move", "dash", "actives", "music", "options", "unpause"] {
            assert!(pause.contains(needle), "pause legend misses {needle:?}: {pause}");
        }
        // ...and every narrow line fits the 58-column plates.
        for line in c.title_legend_lines(false).iter().chain(c.pause_legend_lines(false)) {
            assert!(line.chars().count() <= 56, "{line:?} is too wide");
        }
    }

    #[test]
    fn mouse_aim_legends_say_the_pointer_aims_and_the_clicks_split() {
        let mut c = Controls::default();
        c.set_scheme(Scheme::MouseAim);
        c.set_lone_ctrl_hint(true);
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("mouse aims"), "{title}");
        assert!(title.contains("RMB dash"), "{title}");
        let pause = c.pause_legend_lines(false).join(" ");
        assert!(pause.contains("LMB actives"), "{pause}");
        assert!(pause.contains("RMB dash"), "{pause}");
    }

    #[test]
    fn pad_legends_name_pad_glyphs() {
        let mut c = Controls::default();
        c.set_scheme(Scheme::Gamepad);
        c.note_device(Device::Pad);
        c.rebuild();
        let title = c.title_legend_lines(true).join(" ");
        for needle in ["stick", "RB dash", "X actives", "START pause", "A confirm", "B back", "R3 reset zoom"] {
            assert!(title.contains(needle), "pad legend misses {needle:?}: {title}");
        }
    }

    #[test]
    fn legends_follow_a_rebinding() {
        let mut c = Controls::default();
        c.bind_active_kbm(Action::Dash, Binding::Key(KeyBind::Char('v')));
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("V dash"), "{title}");
        // A reset puts Ctrl back.
        c.reset_active();
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("Ctrl dash"), "{title}");
    }

    #[test]
    fn saved_controls_round_trip_and_old_files_default() {
        let mut c = Controls::default();
        c.set_scheme(Scheme::Gamepad);
        c.bind_active_kbm(Action::Mute, Binding::Key(KeyBind::Char('v')));
        c.bind_active_pad(Action::Dash, Binding::Pad(PadBind::R2));
        c.deadzone = 0.35;
        c.rumble = true;
        let saved = c.to_saved();
        let back = Controls::from_saved(&saved);
        assert_eq!(back.scheme(), Scheme::Gamepad);
        assert_eq!(
            back.active().get(Action::Mute),
            Binding::Key(KeyBind::Char('v'))
        );
        assert_eq!(back.active().pad_of(Action::Dash), Binding::Pad(PadBind::R2));
        assert!((back.deadzone - 0.35).abs() < 1e-6);
        assert!(back.rumble);

        // An old file with nothing saved loads as Classic at the defaults.
        let old = SavedControls::default();
        let fresh = Controls::from_saved(&old);
        assert_eq!(fresh.scheme(), Scheme::Classic);
        assert_eq!(fresh.active().get(Action::Dash), Binding::Key(KeyBind::Ctrl));
        assert!((fresh.deadzone - DEFAULT_DEADZONE).abs() < 1e-6);
        assert!(!fresh.rumble);
    }

    /// Nest, Quit and Reset are ordinary actions: their defaults collide with
    /// nothing, a rebind swaps like any other, and a file saved before they
    /// existed never loads with one key on two actions.
    #[test]
    fn nest_quit_and_reset_are_bindable_and_never_double_bound() {
        for scheme in SCHEMES {
            let b = Bindings::defaults(scheme);
            for a in [Action::Nest, Action::Quit, Action::Reset] {
                let key = b.get(a);
                assert!(!key.is_none(), "{a:?} has no default");
                let owners = ACTIONS
                    .into_iter()
                    .filter(|&o| b.kbm_slots(o).contains(&key))
                    .count();
                assert_eq!(owners, 1, "{scheme:?}: {a:?}'s default is shared");
            }
        }
        // A conflict swaps: binding Mute's key to Quit hands Quit's to Mute.
        let mut c = Controls::default();
        let swapped = c.bind_active_kbm(Action::Quit, Binding::Key(KeyBind::Char('m')));
        assert_eq!(swapped, Some(Action::Mute));
        assert_eq!(c.active().get(Action::Mute), Binding::Key(KeyBind::Char('q')));
        assert_eq!(c.action_label(Action::Quit), "M");
        assert!(c.pause_legend_lines(true).join(" ").contains("M quit"));

        // An old save (23 actions) in which the player had put B on Mute: the
        // new Nest action must not also claim B.
        let mut old = Controls::default();
        old.bind_active_kbm(Action::Mute, Binding::Key(KeyBind::Char('b')));
        let mut saved = old.to_saved();
        for s in saved.schemes.iter_mut().flatten() {
            s.kbm.truncate(23);
            s.alt.truncate(23);
            s.pad.truncate(23);
        }
        let back = Controls::from_saved(&saved);
        assert_eq!(back.active().get(Action::Mute), Binding::Key(KeyBind::Char('b')));
        assert!(back.active().get(Action::Nest).is_none(), "B is on two actions");
        assert_eq!(back.active().get(Action::Quit), Binding::Key(KeyBind::Char('q')));
    }

    #[test]
    fn graphics_is_appended_and_old_controls_get_its_free_default() {
        let key = Binding::Key(KeyBind::Char('g'));
        assert_eq!(ACTION_COUNT, 27);
        assert_eq!(ACTIONS[26], Action::Graphics);
        assert_eq!(Action::Graphics.index(), 26);

        let mut saved = Controls::default().to_saved();
        for s in saved.schemes.iter_mut().flatten() {
            s.kbm.truncate(26);
            s.alt.truncate(26);
            s.pad.truncate(26);
        }
        let mut loaded = Controls::from_saved(&saved);
        for scheme in SCHEMES {
            loaded.set_scheme(scheme);
            let b = loaded.active();
            assert_eq!(b.get(Action::Graphics), key);
            assert!(b.alt_of(Action::Graphics).is_none());
            assert!(b.pad_of(Action::Graphics).is_none());
            assert_eq!(ACTIONS.iter().filter(|&&a| b.kbm_slots(a).contains(&key)).count(), 1);
        }
        assert!(loaded.pause_legend_lines(true).join(" ").contains("G graphics"));

        // An existing assignment in a pre-Graphics save keeps its key.
        let classic = saved.schemes[0].as_mut().unwrap();
        classic.alt[Action::Mute.index()] = key.as_str();
        let loaded = Controls::from_saved(&saved);
        assert_eq!(loaded.active().alt_of(Action::Mute), key);
        assert!(loaded.active().get(Action::Graphics).is_none());
    }

    #[test]
    fn pad_aim_is_none_while_centred_and_a_point_when_deflected() {
        let o = Vec2::new(100.0, 100.0);
        assert_eq!(pad_aim_point(o, Vec2::ZERO), None);
        let p = pad_aim_point(o, Vec2::new(0.0, -1.0)).unwrap();
        assert!((p - Vec2::new(100.0, 100.0 - PAD_AIM_DISTANCE)).len() < 1e-3);
    }

    /// Hold-to-charge is named by the legend, and where the terminal cannot
    /// report a key release the legend (and the Controls note) says the mouse or
    /// the pad does the charging instead of pretending a key can.
    #[test]
    fn the_legends_and_the_controls_note_name_hold_to_charge() {
        let mut c = Controls::default();
        c.set_lone_ctrl_hint(true);
        c.note_device(Device::Keyboard);
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("hold Ctrl to charge"), "{title}");
        let narrow = c.title_legend_lines(false).join(" ");
        assert!(narrow.contains("hold = charge"), "{narrow}");
        assert!(c.terminal_note().contains("hold DASH to charge"));

        // The fallback terminal: keys tap, the mouse/pad charges — and the
        // narrow legend still fits its 58-column card.
        c.set_lone_ctrl_hint(false);
        let title = c.title_legend_lines(true).join(" ");
        assert!(title.contains("hold mouse or pad to charge"), "{title}");
        let narrow = c.title_legend_lines(false).join(" ");
        assert!(narrow.contains("hold mouse = charge"), "{narrow}");
        for line in c.title_legend_lines(false) {
            assert!(
                line.chars().count() <= 56,
                "{line:?} is too wide for the 58-column card"
            );
        }
        assert!(c.terminal_note().contains("keys tap"));
        assert!(c.terminal_note().contains("mouse or pad"));
    }
}
