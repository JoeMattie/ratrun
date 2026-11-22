//! Control schemes, remappable bindings and the legends that name them.
//!
//! Three schemes, selectable in Options and persisted beside the other
//! preferences (`crate::scores`):
//!
//! * **Classic** — the game as it shipped: WASD/arrows move, or the mouse
//!   steers the rat toward the pointer; weapons auto-aim; Ctrl dashes (Space
//!   on a terminal that cannot report a lone modifier); Space releases the
//!   charged active weapons; a left click is both.
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
//! and a secondary slot, so "Ctrl or right click" is one action) and a
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
pub const ACTION_COUNT: usize = 26;

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
        }
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
        let mut bind = match ev.code {
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
        // A lone modifier arrives with its own modifier bit set; that is the
        // same press, not a combo.
        if matches!(bind, KeyBind::Ctrl | KeyBind::Alt | KeyBind::Shift) {
            bind = match bind {
                KeyBind::Ctrl => KeyBind::Ctrl,
                KeyBind::Shift => KeyBind::Shift,
                _ => KeyBind::Alt,
            };
        }
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
                b.kbm[Action::Dash.index()] = Binding::Key(KeyBind::Ctrl);
                b.alt[Action::Dash.index()] = Binding::Mouse(MouseBind::Left);
                b.kbm[Action::FireActives.index()] = Binding::Key(KeyBind::Space);
                b.alt[Action::FireActives.index()] = Binding::Mouse(MouseBind::Left);
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
    pub fn bind_kbm(&mut self, action: Action, new: Binding) -> Option<Action> {
        let old = self.kbm[action.index()];
        let mut swapped = None;
        if !new.is_none() {
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
    pub fn note_device(&mut self, device: Device) {
        self.device = device;
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

    /// The label of the active-weapon key.
    pub fn active_label(&self) -> &str {
        match self.active().get(Action::FireActives) {
            Binding::Mouse(m) => m.label(),
            Binding::Key(k) => k.label(),
            Binding::Pad(p) => p.label(),
            Binding::None => "—",
        }
    }

    /// The dash key as it will actually work on **this** terminal: a Ctrl
    /// binding on a terminal without lone-modifier reporting falls back to
    /// Space (which then also fires the actives).
    pub fn live_dash_label(&self) -> &str {
        if !self.lone_ctrl_hint && self.active().get(Action::Dash) == Binding::Key(KeyBind::Ctrl) {
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
                    "{} map · {} minimap · {} reset zoom · {} options",
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
                format!("{} reset zoom · {} options", pad(Action::ResetZoom), pad(Action::Options)),
            ];
            self.pause_wide = vec![
                format!(
                    "stick/d-pad move · {} dash · {} actives · R-stick aim · {} unpause",
                    pad(Action::Dash),
                    pad(Action::FireActives),
                    pad(Action::Pause),
                ),
                format!(
                    "{} cursor · {} mute · {} options · {} back to menu",
                    pad(Action::ToggleCursor),
                    pad(Action::Mute),
                    pad(Action::Options),
                    pad(Action::Back),
                ),
            ];
            self.pause_narrow = vec![
                format!("move stick · dash {} · actives {}", pad(Action::Dash), pad(Action::FireActives)),
                format!("{} unpause · {} options", pad(Action::Pause), pad(Action::Options)),
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
        let dash_line = if share {
            format!(
                "WASD/mouse move · {dash} dash + actives · click = dash + ready actives · {pause} pause"
            )
        } else if mouse_aim {
            format!(
                "WASD/arrows move · mouse aims · {dash} dash · {actives} actives · {pause} pause"
            )
        } else {
            format!(
                "WASD/mouse move · {dash} dash · {actives} actives · click = dash + ready actives · {pause} pause"
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
                "{} {} music · {} {} effects · {} story · {} · {} quit",
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
            format!("click = dash + actives · {pause} pause"),
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
        let click_job = if self.scheme == Scheme::MouseAim {
            "LMB actives · RMB dash"
        } else {
            "click = dash + actives"
        };
        self.pause_wide = vec![
            format!(
                "WASD/mouse move · {dash} dash · {actives} actives · {click_job}"
            ),
            format!(
                "{} {} music · {} {} effects · {} options · {pause} unpause · {} quit",
                key(Action::MusicDown),
                key(Action::MusicUp),
                key(Action::EffectsDown),
                key(Action::EffectsUp),
                key(Action::Options),
                key(Action::Quit)
            ),
        ];
        let narrow_click = if self.scheme == Scheme::MouseAim {
            "LMB actives · RMB dash"
        } else {
            "click dash+actives"
        };
        self.pause_narrow = vec![
            format!("move WASD · dash {dash} · actives {actives}"),
            format!("{narrow_click} · {pause} unpause"),
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
                    if let Some(raw) = s.kbm.get(idx).and_then(|v| Binding::parse(v)) {
                        set.kbm[idx] = raw;
                    }
                    if let Some(raw) = s.alt.get(idx).and_then(|v| Binding::parse(v)) {
                        set.alt[idx] = raw;
                    }
                    if let Some(raw) = s.pad.get(idx).and_then(|v| Binding::parse(v)) {
                        if matches!(raw, Binding::Pad(_) | Binding::None) {
                            set.pad[idx] = raw;
                        }
                    }
                }
                // An action newer than the file takes its default key only
                // when the player has not already given that key to something
                // else; otherwise it starts unbound, never double-bound.
                for a in ACTIONS {
                    let idx = a.index();
                    if idx < s.kbm.len() {
                        continue;
                    }
                    let fresh = set.kbm[idx];
                    let taken = ACTIONS.into_iter().any(|o| {
                        o.index() < s.kbm.len() && set.kbm_slots(o).contains(&fresh)
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
            "click",
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
