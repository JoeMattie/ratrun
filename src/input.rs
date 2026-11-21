//! Input handling. Movement keys need to be treated as "held", but terminals
//! only reliably report key *presses* (plus auto-repeat). When the Kitty
//! keyboard protocol (Ghostty, Kitty and others) is available we get release
//! events too, so a key stays down until released. Otherwise we fall back to a
//! short grace window that auto-repeat keeps refreshing.
//!
//! Under the protocol's `REPORT_EVENT_TYPES` flag, letter keys report releases;
//! Enter, Tab and Backspace do not unless report-all-keys is also pushed. The
//! game only uses those as discrete presses, so it never relies on their
//! release events.
//!
//! # Dash on Ctrl (`report all keys`)
//!
//! `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is pushed on top of `REPORT_EVENT_TYPES`
//! and is the flag that makes a **lone modifier press** report at all. The
//! owner's mapping is Ctrl (either one) = dash, Space = fire the charged active
//! weapons, so the two jobs Space used to share are split where the terminal can
//! tell a bare Ctrl apart from everything else. A terminal that cannot (Apple
//! Terminal, plain xterm, anything that answers only DA1) keeps the old mapping
//! — Space dashes *and* fires the actives — so the game stays playable; the
//! controls legend and the HUD's dash module name whichever key is live
//! ([`DashKey`]).

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind,
};

use crate::math::Vec2;

/// Which key dashes, which is what the HUD and the legends have to name.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DashKey {
    /// Kitty-protocol terminal: a lone Ctrl press dashes, Space only fires the
    /// charged active weapons.
    Ctrl,
    /// No lone-modifier reporting: Space dashes and fires the actives together
    /// (the pre-Ctrl mapping, kept so the game stays playable).
    #[default]
    Space,
}

impl DashKey {
    /// The key name as the HUD prints it.
    pub fn name(self) -> &'static str {
        match self {
            DashKey::Ctrl => "CTRL",
            DashKey::Space => "SPACE",
        }
    }

    /// The title screen's controls line for this mode. Both modes name their
    /// dash key, because the binding really is different: in the Ctrl mode
    /// Space only fires the active weapons, so calling it "dash+active" would
    /// be a lie.
    pub fn title_legend(self, wide: bool) -> &'static str {
        match (self, wide) {
            (DashKey::Ctrl, true) => {
                "WASD/arrows/mouse move · Ctrl/click dash · Space actives · RMB pause · M mute"
            }
            (DashKey::Ctrl, false) => "Ctrl dash · Space actives · RMB pause · M mute",
            (DashKey::Space, true) => {
                "WASD/arrows/mouse move · Space/click dash+active · right click pause · M mute"
            }
            (DashKey::Space, false) => "WASD/arrows · Space dash+active · RMB pause · M mute",
        }
    }

    /// The pause plate's status line.
    pub fn pause_legend(self) -> &'static str {
        match self {
            DashKey::Ctrl => "M mute  ·  WASD/mouse move  ·  Ctrl dash",
            DashKey::Space => "M mute  ·  WASD/mouse move  ·  Space dash",
        }
    }
}

/// Only the real kitty-coded lone modifiers count; Hyper/Meta/Super/CapsLock
/// presses are ignored (they are reported by the same flag).
fn is_ctrl(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Modifier(ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl)
    )
}

/// Grace given to a key that is being refreshed by auto-repeat.
const GRACE: f32 = 0.13;
/// Grace given to the *first* press of a direction. Without the protocol a
/// terminal only reports presses plus auto-repeat, and the OS initial repeat
/// delay is typically 250–600 ms: with only `GRACE` the rat would move for
/// 0.13 s, stop, then start again once repeats begin.
const FIRST_PRESS_GRACE: f32 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Whether a key event should be recorded as a discrete press.
///
/// A terminal without the protocol reports auto-repeat as `Press`; with it the
/// repeat arrives as `Repeat`. Recording a repeated Space (dash) as a press
/// makes holding it behave the same in both modes, while menu keys stay
/// single-shot because their `Repeat` events are never recorded.
fn records_press(ev: KeyEvent) -> bool {
    ev.kind == KeyEventKind::Press
        || (ev.kind == KeyEventKind::Repeat && ev.code == KeyCode::Char(' '))
}

pub struct InputState {
    grace: [f32; 4],
    pub kitty: bool,
    /// True when the terminal reports a **lone** Ctrl press (Kitty protocol
    /// with `REPORT_ALL_KEYS_AS_ESCAPE_CODES`). Seeded from the protocol query
    /// and confirmed by `main` against the flags the terminal reports *after*
    /// the push; the fallback is the Space mapping.
    pub lone_ctrl: bool,
    /// True while either Ctrl is held (cleared by a release or by focus loss).
    pub ctrl_down: bool,
    /// A lone Ctrl **press** landed this frame. Never set by a repeat, so
    /// holding Ctrl dashes exactly once.
    pub dash_tap: bool,
    /// Discrete key presses captured this frame (menus, pause, dash, quit).
    pub pressed: Vec<KeyEvent>,
    /// Ordered UI gestures for this frame, retaining the position of each press.
    pub mouse_events: Vec<MouseEvent>,
    /// Latest mouse cursor cell (column, row), persisted across frames.
    pub mouse_pos: Option<(u16, u16)>,
    /// True when the mouse was the most recent movement input. A movement
    /// keypress flips this back to false so the keyboard reclaims control.
    pub prefer_mouse: bool,
    /// True while a mouse button (left/right/middle) is currently held down.
    pub mouse_down: bool,
    /// True if a mouse button press occurred on this frame.
    pub mouse_clicked: bool,
    /// True if any pointer movement, drag, press or release occurred this frame.
    pub mouse_active: bool,
}

impl InputState {
    pub fn new(kitty: bool) -> Self {
        InputState {
            grace: [0.0; 4],
            kitty,
            // Optimistic default: a protocol-speaking terminal is assumed to
            // honour the all-keys flag. `main` downgrades it to `false` when
            // the post-push query comes back without the bit.
            lone_ctrl: kitty,
            ctrl_down: false,
            dash_tap: false,
            pressed: Vec::new(),
            mouse_pos: None,
            mouse_events: Vec::new(),
            prefer_mouse: false,
            mouse_down: false,
            mouse_clicked: false,
            mouse_active: false,
        }
    }

    pub fn begin_frame(&mut self) {
        self.pressed.clear();
        self.mouse_events.clear();
        self.mouse_clicked = false;
        self.mouse_active = false;
        self.dash_tap = false;
    }

    /// Force the dash mapping (the fallback used when the terminal answers the
    /// protocol query but ignores the all-keys flag).
    pub fn set_lone_ctrl(&mut self, on: bool) {
        self.lone_ctrl = on;
    }

    /// The key that dashes in this mode.
    pub fn dash_key(&self) -> DashKey {
        if self.lone_ctrl {
            DashKey::Ctrl
        } else {
            DashKey::Space
        }
    }

    /// Was the dash asked for this frame?
    ///
    /// With lone-Ctrl reporting that is a Ctrl tap; without it, Space (which
    /// then cannot be split from the active-weapon release).
    pub fn dash_pressed(&self) -> bool {
        if self.lone_ctrl {
            self.dash_tap
        } else {
            self.just_pressed(KeyCode::Char(' '))
        }
    }

    /// Was the active-weapon release asked for this frame? Always Space: in the
    /// Ctrl mode that is Space's only job, and without lone-Ctrl reporting
    /// Space dashes at the same time (the press is shared, as it always was).
    pub fn active_pressed(&self) -> bool {
        self.just_pressed(KeyCode::Char(' '))
    }

    /// Decay held-key grace timers (no-op when the protocol's release events
    /// are used).
    pub fn tick(&mut self, dt: f32) {
        if !self.kitty {
            for g in self.grace.iter_mut() {
                *g = (*g - dt).max(0.0);
            }
        }
    }

    /// Track a lone modifier press. Returns `true` when the event was only a
    /// modifier, so the caller stops there.
    ///
    /// A lone modifier is only ever a dash tap. It is deliberately never
    /// recorded in `pressed`: every menu, the options keys and the "any key"
    /// paths would otherwise see Ctrl as a plain keystroke.
    fn handle_modifier(&mut self, ev: KeyEvent) -> bool {
        if !is_ctrl(ev.code) {
            return false;
        }
        match ev.kind {
            KeyEventKind::Press => {
                self.ctrl_down = true;
                self.dash_tap = true;
            }
            // Some terminals auto-repeat a held modifier. Swallow it: a held
            // Ctrl must not machine-gun dashes.
            KeyEventKind::Repeat => {}
            KeyEventKind::Release => self.ctrl_down = false,
        }
        true
    }

    pub fn handle_key(&mut self, ev: KeyEvent) {
        if self.handle_modifier(ev) {
            return;
        }
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
                    // The protocol reports the release, so a press holds until then.
                    // Otherwise: a long window on the initial press (bridges
                    // the OS repeat delay), the short one on auto-repeats (so
                    // stopping stays responsive once repeats flow).
                    let v = if self.kitty {
                        f32::INFINITY
                    } else if ev.kind == KeyEventKind::Press {
                        FIRST_PRESS_GRACE
                    } else {
                        GRACE
                    };
                    self.grace[d as usize] = v;
                    // A movement key hands control back to the keyboard.
                    self.prefer_mouse = false;
                }
                if records_press(ev) {
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
        let moved = self.mouse_pos != Some((ev.column, ev.row));
        if matches!(
            ev.kind,
            MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ) || (moved && matches!(ev.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)))
        {
            self.mouse_events.push(ev);
        }
        match ev.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                self.mouse_pos = Some((ev.column, ev.row));
                self.prefer_mouse = true;
                self.mouse_active = true;
            }
            MouseEventKind::Down(_) => {
                self.mouse_pos = Some((ev.column, ev.row));
                self.prefer_mouse = true;
                self.mouse_down = true;
                self.mouse_clicked = true;
                self.mouse_active = true;
            }
            MouseEventKind::Up(_) => {
                self.mouse_pos = Some((ev.column, ev.row));
                self.mouse_down = false;
                self.mouse_active = true;
            }
            _ => {}
        }
    }

    /// Forget every held movement key (and mouse button). Called when the
    /// terminal loses focus: a release for a key or button that was down at
    /// that moment never arrives, so held state would otherwise stick forever.
    pub fn release_all(&mut self) {
        self.grace = [0.0; 4];
        self.mouse_down = false;
        // A Ctrl held across a focus change never reports its release either.
        self.ctrl_down = false;
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
        self.pressed
            .iter()
            .any(|e| matches!(e.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c)))
    }

    /// Like [`pressed_char`], but ignores Ctrl/Alt-modified presses (so Ctrl+C
    /// quitting never trips an unmodified `'c'` toggle).
    pub fn pressed_unmodified_char(&self, c: char) -> bool {
        self.pressed.iter().any(|e| {
            matches!(e.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
                && !e.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
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
            row,
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

    fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
    }

    #[test]
    fn first_press_bridges_os_repeat_delay_but_repeat_stops_quickly() {
        let mut s = InputState::new(false);
        s.handle_key(key(KeyCode::Char('w'), KeyEventKind::Press));
        assert!(s.move_dir().y < 0.0);

        // No auto-repeat has arrived yet (an OS delay of up to ~0.6 s).
        s.tick(0.3);
        assert!(s.move_dir().y < 0.0, "stopped during the OS repeat delay");
        s.tick(0.25);
        assert!(
            s.move_dir().y < 0.0,
            "stopped before the first auto-repeat could arrive"
        );

        // Auto-repeat refreshes with the short window...
        s.handle_key(key(KeyCode::Char('w'), KeyEventKind::Repeat));
        s.tick(GRACE / 2.0);
        assert!(s.move_dir().y < 0.0);
        // ...so letting go (once repeats have stopped) stops quickly.
        s.tick(GRACE);
        assert_eq!(s.move_dir(), Vec2::ZERO);
    }

    #[test]
    fn first_press_alone_expires_after_its_long_window() {
        let mut s = InputState::new(false);
        s.handle_key(key(KeyCode::Char('d'), KeyEventKind::Press));
        s.tick(FIRST_PRESS_GRACE + 0.01);
        assert_eq!(s.move_dir(), Vec2::ZERO);
    }

    #[test]
    fn release_stops_movement_immediately() {
        let mut s = InputState::new(false);
        s.handle_key(key(KeyCode::Char('a'), KeyEventKind::Press));
        s.handle_key(key(KeyCode::Char('a'), KeyEventKind::Repeat));
        s.handle_key(key(KeyCode::Char('a'), KeyEventKind::Release));
        assert_eq!(s.move_dir(), Vec2::ZERO);
    }

    #[test]
    fn space_repeat_re_dashes_in_both_input_modes() {
        for kitty in [true, false] {
            let mut s = InputState::new(kitty);
            s.handle_key(key(KeyCode::Char(' '), KeyEventKind::Repeat));
            assert!(
                s.just_pressed(KeyCode::Char(' ')),
                "Space repeat ignored (kitty={kitty})"
            );
        }
    }

    #[test]
    fn menu_keys_do_not_repeat_fire() {
        for kitty in [true, false] {
            let mut s = InputState::new(kitty);
            for code in [
                KeyCode::Enter,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Esc,
                KeyCode::Char('m'),
                KeyCode::Char('p'),
                KeyCode::Char('q'),
                KeyCode::Char('w'),
            ] {
                s.begin_frame();
                s.handle_key(key(code, KeyEventKind::Repeat));
                assert!(
                    !s.just_pressed(code),
                    "Repeat fired for {code:?} (kitty={kitty})"
                );
                assert!(
                    s.pressed.is_empty(),
                    "Repeat recorded for {code:?} (kitty={kitty})"
                );
            }
        }
    }

    #[test]
    fn release_all_clears_kitty_held_keys() {
        let mut s = InputState::new(true);
        s.handle_key(key(KeyCode::Char('w'), KeyEventKind::Press));
        s.handle_key(key(KeyCode::Char('d'), KeyEventKind::Press));
        // Protocol grace is infinite, so the directions stay down without a release.
        assert_eq!(s.move_dir(), Vec2::new(1.0, -1.0).normalized());

        s.release_all();
        assert_eq!(s.move_dir(), Vec2::ZERO);

        // A fresh press still moves after the focus loss.
        s.handle_key(key(KeyCode::Char('s'), KeyEventKind::Press));
        assert!(s.move_dir().y > 0.0);
    }

    /// Documents why the enhancement flags stay at `REPORT_EVENT_TYPES` only:
    /// Ghostty and Kitty report releases for letter keys under that flag, so
    /// w/a/s/d are held until released; Enter, Tab and Backspace have no
    /// releases without report-all-keys, but the game only uses them as
    /// discrete presses, so nothing can stick.
    #[test]
    fn kitty_protocol_reports_letter_releases() {
        let mut s = InputState::new(true);
        let cases = [
            (KeyCode::Char('w'), Vec2::new(0.0, -1.0)),
            (KeyCode::Char('a'), Vec2::new(-1.0, 0.0)),
            (KeyCode::Char('s'), Vec2::new(0.0, 1.0)),
            (KeyCode::Char('d'), Vec2::new(1.0, 0.0)),
        ];
        for (code, expected) in cases {
            s.handle_key(key(code, KeyEventKind::Press));
            assert_eq!(s.move_dir(), expected, "press of {code:?}");
            s.handle_key(key(code, KeyEventKind::Release));
            assert_eq!(s.move_dir(), Vec2::ZERO, "release of {code:?}");
        }

        // A press without a release leaves no movement state behind for the
        // keys that lack release events under REPORT_EVENT_TYPES.
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Backspace] {
            s.begin_frame();
            s.handle_key(key(code, KeyEventKind::Press));
            assert!(s.just_pressed(code), "press of {code:?} not recorded");
            assert_eq!(s.move_dir(), Vec2::ZERO, "{code:?} held movement");
        }
    }

    #[test]
    fn mouse_button_down_up_and_focus_loss() {
        let mut s = InputState::new(false);
        assert!(!s.mouse_down && !s.mouse_clicked && !s.mouse_active);
        s.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, 5));
        assert!(s.mouse_down && s.mouse_clicked && s.mouse_active);
        s.begin_frame();
        assert!(s.mouse_down, "button stays held across frames until Up");
        assert!(!s.mouse_clicked && !s.mouse_active);
        s.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 4, 5));
        assert!(!s.mouse_down && s.mouse_active);

        s.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, 5));
        assert!(s.mouse_down);
        s.release_all();
        assert!(!s.mouse_down, "focus loss must clear held mouse button");
    }

    #[test]
    fn unmodified_char_ignores_ctrl_c() {
        let mut s = InputState::new(false);
        s.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(s.pressed_char('c'));
        assert!(!s.pressed_unmodified_char('c'));

        s.begin_frame();
        s.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(s.pressed_unmodified_char('c'));
    }

    fn ctrl(kind: KeyEventKind) -> KeyEvent {
        // What crossterm builds from `CSI 57442;5:1u`: a lone left Ctrl press.
        KeyEvent::new_with_kind(
            KeyCode::Modifier(ModifierKeyCode::LeftControl),
            KeyModifiers::CONTROL,
            kind,
        )
    }

    /// Kitty mode: Ctrl dashes, Space fires the actives and never dashes.
    #[test]
    fn lone_ctrl_dashes_and_space_only_fires_the_actives() {
        let mut s = InputState::new(true);
        assert_eq!(s.dash_key(), DashKey::Ctrl);

        s.handle_key(ctrl(KeyEventKind::Press));
        assert!(s.dash_pressed(), "a lone Ctrl press must dash");
        assert!(!s.active_pressed(), "Ctrl must not fire an active weapon");
        assert!(
            s.pressed.is_empty(),
            "a modifier press must not reach the menus as a keystroke"
        );
        assert!(s.ctrl_down, "Ctrl is held until its release");

        s.begin_frame();
        assert!(!s.dash_pressed(), "a tap is one frame, not a held dash");
        s.handle_key(key(KeyCode::Char(' '), KeyEventKind::Press));
        assert!(!s.dash_pressed(), "Space must not dash in the Ctrl mode");
        assert!(s.active_pressed(), "Space fires the charged actives");
    }

    /// Fallback (no lone-modifier reporting): Space keeps doing both jobs.
    #[test]
    fn fallback_mode_keeps_space_for_dash_and_actives() {
        let mut s = InputState::new(false);
        assert_eq!(s.dash_key(), DashKey::Space);

        s.handle_key(key(KeyCode::Char(' '), KeyEventKind::Press));
        assert!(s.dash_pressed() && s.active_pressed());

        // A modifier event that some terminal sends anyway cannot dash: the
        // mapping is Space's.
        s.begin_frame();
        s.handle_key(ctrl(KeyEventKind::Press));
        assert!(!s.dash_pressed(), "Ctrl is not the dash key here");
        assert!(!s.active_pressed());
    }

    /// Holding Ctrl must not auto-repeat dashes, and the release clears it.
    #[test]
    fn holding_ctrl_dashes_once() {
        let mut s = InputState::new(true);
        s.handle_key(ctrl(KeyEventKind::Press));
        s.begin_frame();
        for _ in 0..10 {
            s.handle_key(ctrl(KeyEventKind::Repeat));
            assert!(!s.dash_pressed(), "a held Ctrl re-dashed");
        }
        assert!(s.ctrl_down);
        s.handle_key(ctrl(KeyEventKind::Release));
        assert!(!s.ctrl_down);

        // ...and a fresh press after the release dashes again.
        s.handle_key(ctrl(KeyEventKind::Press));
        assert!(s.dash_pressed());
        // Focus loss drops the held state, including Ctrl.
        s.release_all();
        assert!(!s.ctrl_down);
    }

    /// Ctrl held while a movement key is pressed still moves, and the movement
    /// key's release still stops it (Ctrl does not eat the direction).
    #[test]
    fn ctrl_held_movement_still_moves() {
        let mut s = InputState::new(true);
        s.handle_key(ctrl(KeyEventKind::Press));
        s.begin_frame();
        for code in [KeyCode::Char('w'), KeyCode::Char('a')] {
            s.handle_key(KeyEvent::new_with_kind(
                code,
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ));
        }
        assert_eq!(s.move_dir(), Vec2::new(-1.0, -1.0).normalized());
        assert!(!s.dash_pressed(), "moving with Ctrl held is not a dash");

        // Right Ctrl is the same key.
        s.release_all();
        s.handle_key(KeyEvent::new_with_kind(
            KeyCode::Modifier(ModifierKeyCode::RightControl),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        ));
        assert!(s.dash_pressed(), "either Ctrl dashes");
    }

    /// The legend and the HUD name the live key in both modes.
    #[test]
    fn the_dash_key_names_the_live_binding() {
        assert_eq!(DashKey::Ctrl.name(), "CTRL");
        assert_eq!(DashKey::Space.name(), "SPACE");
        // Both legends must name the live key and what Space does.
        assert!(DashKey::Ctrl.title_legend(true).contains("Ctrl/click dash"));
        assert!(DashKey::Ctrl.title_legend(true).contains("Space actives"));
        assert!(DashKey::Ctrl.title_legend(false).starts_with("Ctrl dash"));
        assert!(DashKey::Space.title_legend(true).contains("Space/click dash"));
        assert!(DashKey::Space.title_legend(false).starts_with("WASD/arrows · Space"));
        assert!(DashKey::Ctrl.pause_legend().ends_with("Ctrl dash"));
        assert!(DashKey::Space.pause_legend().ends_with("Space dash"));
        // Switching the mode after construction (main's confirmed answer).
        let mut s = InputState::new(true);
        s.set_lone_ctrl(false);
        assert_eq!(s.dash_key(), DashKey::Space);
        s.set_lone_ctrl(true);
        assert_eq!(s.dash_key(), DashKey::Ctrl);
    }
}
