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

use crate::controls::{Action, Binding, Controls, Device, KeyBind, MouseBind};
use crate::gamepad::{PadEvent, PadState};
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

    /// The title plate's controls legend, **most important line first**.
    ///
    /// Every live control for the current mode is named here at least once:
    /// movement, the dash key, the actives key, the click's two jobs, wheel and
    /// `Z`/`X` zoom, `Tab` map, `N` minimap size, `C` cursor, `O` options, `M`
    /// mute, the `[` `]` / `-` `=` volume keys, pause and quit. The review found
    /// the old single line concealing the click's second action and dropping
    /// zoom, `N`, `C` and options entirely, and the narrow variant losing
    /// movement — so the plate lays out as many lines as it has rows for
    /// (see `menu::draw_title`) and the most essential line is never the one
    /// dropped.
    ///
    /// Both modes name their dash key, because the binding really is different:
    /// in the Ctrl mode Space *only* fires the active weapons, so calling it
    /// "dash+active" would be a lie.
    pub fn title_legend_lines(self, wide: bool) -> &'static [&'static str] {
        match (self, wide) {
            (DashKey::Ctrl, true) => &[
                "WASD/mouse move · Ctrl dash · Space actives · click = dash + ready actives · Esc/P pause",
                "wheel or Z/X zoom · Tab map · N minimap · C cursor · O options · M mute",
                "[ ] music · - = effects · L story · Q quit",
            ],
            (DashKey::Ctrl, false) => &[
                "WASD/mouse move · Ctrl dash · Space actives",
                "click = dash + ready actives · Esc/P pause",
                "wheel or Z/X zoom · Tab map · N minimap · C cursor",
                "O options · M mute · [ ] - = volume · Q quit",
            ],
            (DashKey::Space, true) => &[
                "WASD/mouse move · Space dash + actives · click = dash + ready actives · Esc/P pause",
                "wheel or Z/X zoom · Tab map · N minimap · C cursor · O options · M mute",
                "[ ] music · - = effects · L story · Q quit",
            ],
            (DashKey::Space, false) => &[
                "WASD/mouse move · Space dash + actives",
                "click = dash + ready actives · Esc/P pause",
                "wheel or Z/X zoom · Tab map · N minimap · C cursor",
                "O options · M mute · [ ] - = volume · Q quit",
            ],
        }
    }

    /// The pause plate's controls legend, **most important line first**.
    ///
    /// Two lengths: `wide` fits the 76-column dossier (68 and 63 characters),
    /// the narrow one fits the 58-column card and the compact short-terminal
    /// plate (43 each). Every live control is named — movement, the dash key,
    /// the actives, the click's two jobs, the volume keys, mute, options and
    /// resume — and the legend is never replaced by the audio state: `M` lives
    /// on the controls row as a chip that reads `M mute` / `M unmute`, so the
    /// two can never contradict each other (the review found the old plate
    /// swapping its whole legend for "audio muted").
    pub fn pause_legend_lines(self, wide: bool) -> &'static [&'static str] {
        match (self, wide) {
            (DashKey::Ctrl, true) => &[
                "WASD/mouse move · Ctrl dash · Space actives · click = dash + actives",
                "[ ] music · - = effects · O options · Esc/P unpause · Q quit",
            ],
            (DashKey::Space, true) => &[
                "WASD/mouse move · Space dash + actives · click = dash + actives",
                "[ ] music · - = effects · O options · Esc/P unpause · Q quit",
            ],
            (DashKey::Ctrl, false) => &[
                "move WASD · dash Ctrl · actives Space",
                "click dash+actives · Esc/P unpause",
            ],
            (DashKey::Space, false) => &[
                "move WASD · dash+actives Space",
                "click dash+actives · Esc/P unpause",
            ],
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
    /// Scheme + remappable bindings. The legends, the HUD labels and every
    /// action lookup read this, so a rebinding takes effect on the next frame.
    pub controls: Controls,
    /// Gamepad state fed by `gamepad::PadPoller` (or injected by a test).
    pub pad: PadState,
    /// Does the terminal have focus? Pad input is dropped while it does not.
    pub focused: bool,
    /// Which device drove the last input; the legends follow it.
    pub last_device: Device,
    /// Lone-modifier taps this frame (`[Ctrl, Shift, Alt]`), the only way a
    /// modifier binding can fire without polluting the menu `pressed` list.
    mod_taps: [bool; 3],
    /// Set when a rebinding or a scheme change has been applied, so the app can
    /// push fresh labels into the HUD once.
    pub hud_labels_changed: bool,
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
    /// Which dash inputs are physically **down** right now. The pad is queried
    /// live from `pad` (it owns its own held state), so only the keyboard and
    /// the mouse need a flag here. The keyboard flag is only ever set where the
    /// terminal reports key releases (`kitty`): without them a key can only
    /// tap, and a hold would stick forever.
    dash_kb_down: bool,
    dash_mouse_down: bool,
    /// One-frame edges of the whole dash input (any device), which is what
    /// drives the Super Dash charge: the press starts it, the release fires it.
    dash_down_edge: bool,
    dash_up_edge: bool,
    /// A **fresh** dash press this frame, auto-repeat excluded (any device).
    dash_fresh: bool,
    /// Set when a held dash input was dropped **without** a release edge
    /// (terminal focus loss, pad unplugged): the charge is cancelled, never
    /// fired.
    dash_cancelled: bool,
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
            controls: Controls::default(),
            pad: PadState::new(),
            focused: true,
            last_device: Device::Keyboard,
            mod_taps: [false; 3],
            hud_labels_changed: false,
            kitty,
            // Optimistic default: a protocol-speaking terminal is assumed to
            // honour the all-keys flag. `main` downgrades it to `false` when
            // the post-push query comes back without the bit.
            lone_ctrl: kitty,
            ctrl_down: false,
            dash_tap: false,
            dash_kb_down: false,
            dash_mouse_down: false,
            dash_down_edge: false,
            dash_up_edge: false,
            dash_fresh: false,
            dash_cancelled: false,
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
        self.dash_down_edge = false;
        self.dash_up_edge = false;
        self.dash_fresh = false;
        self.dash_cancelled = false;
        self.mod_taps = [false; 3];
        self.pad.begin_frame();
    }

    /// Advance the pad's repeat clock (called from `tick`). Split out so a test
    /// can drive a pad without a frame clock.
    pub fn tick_pad(&mut self, dt: f32) {
        let deadzone = self.controls.deadzone;
        self.pad.step(dt, deadzone);
    }

    /// Inject one pad event (the seam tests use; `main` feeds these from gilrs).
    pub fn pad_event(&mut self, ev: PadEvent) {
        let was = self.dash_input_held();
        self.pad.apply(ev);
        if matches!(ev, PadEvent::Disconnected) {
            // A pad unplugged mid-charge never reports its release: cancel the
            // charge instead of firing it.
            if was {
                self.cancel_dash();
            }
        } else {
            self.note_dash_transition(was);
        }
        if self.pad.active() {
            self.last_device = Device::Pad;
            self.prefer_mouse = false;
        }
    }

    /// Terminal focus reporting. Losing focus drops every held key, mouse
    /// button and pad button (their releases never arrive).
    pub fn set_focused(&mut self, focused: bool) {
        let was = self.focused;
        self.focused = focused;
        self.pad.set_focused(focused);
        if was && !focused {
            self.release_all();
        }
    }

    /// Swap the control configuration (scheme change, load, rebinding) and keep
    /// the terminal's lone-modifier answer in the legends.
    pub fn set_controls(&mut self, controls: Controls) {
        self.controls = controls;
        self.controls.set_lone_ctrl_hint(self.lone_ctrl);
        self.hud_labels_changed = true;
    }

    /// A rebinding happened: rebuild the cached legends and tell the HUD its
    /// labels are stale.
    pub fn refresh_controls(&mut self) {
        self.controls.set_lone_ctrl_hint(self.lone_ctrl);
        self.hud_labels_changed = true;
    }

    /// Force the dash mapping (the fallback used when the terminal answers the
    /// protocol query but ignores the all-keys flag).
    pub fn set_lone_ctrl(&mut self, on: bool) {
        self.lone_ctrl = on;
        self.controls.set_lone_ctrl_hint(on);
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
        // The binding is the answer; on a terminal without lone-modifier
        // reporting a Ctrl binding falls back to Space (which then also fires
        // the actives, exactly the pre-Ctrl mapping).
        let bound = self.controls.active().get(Action::Dash);
        if !self.lone_ctrl && bound == Binding::Key(KeyBind::Ctrl) {
            return self.action_pressed(Action::Dash) || self.just_pressed(KeyCode::Char(' '));
        }
        self.action_pressed(Action::Dash)
    }

    /// Was there a **fresh** dash press this frame, from any device (key, mouse
    /// button or pad button)? An auto-repeat is never one, so a terminal whose
    /// only "hold" is the OS repeat key rate cannot machine-gun dashes, and the
    /// Super Dash charge starts exactly once per real press.
    pub fn dash_pressed_fresh(&self) -> bool {
        self.dash_fresh
    }

    /// Does this terminal report a **key release** for the dash key? Only with
    /// the Kitty keyboard protocol (`REPORT_EVENT_TYPES`). Where it does not, a
    /// held key cannot be told apart from a tap, so keys only tap and the Super
    /// Dash is charged with the mouse button or the pad — the Controls screen
    /// note and the legend say so.
    pub fn dash_key_holds(&self) -> bool {
        self.kitty
    }

    /// Did the dash input go **down** this frame? (The press edge of a key, a
    /// mouse button or a pad button bound to the dash Action.)
    pub fn dash_down(&self) -> bool {
        self.dash_down_edge
    }

    /// Is the dash input still **held**? A key only counts where the terminal
    /// reports its release (see [`dash_key_holds`](Self::dash_key_holds)); the
    /// mouse and the pad always report theirs.
    pub fn dash_held(&self) -> bool {
        self.dash_input_held()
    }

    /// Did the dash input go **up** this frame? This is what fires a charged
    /// Super Dash. It is never set by focus loss or an unplugged pad: those
    /// *cancel* the charge instead (see [`dash_cancelled`](Self::dash_cancelled)).
    pub fn dash_up(&self) -> bool {
        self.dash_up_edge
    }

    /// Was a held dash input dropped without a release edge (the terminal lost
    /// focus, the pad was unplugged)? The Super Dash charge must be cancelled,
    /// never fired.
    pub fn dash_cancelled(&self) -> bool {
        self.dash_cancelled
    }

    /// Is any device's dash input down right now? The pad is asked through the
    /// live bindings (its own held state), never cached here, so a rebind or a
    /// focus change cannot leave a stale flag behind.
    fn dash_input_held(&self) -> bool {
        if self.dash_kb_down || self.dash_mouse_down {
            return true;
        }
        let b = self.controls.active();
        for slot in b.kbm_slots(Action::Dash) {
            if let Binding::Pad(p) = slot {
                if self.pad.button_down(p) {
                    return true;
                }
            }
        }
        if let Binding::Pad(p) = b.pad_of(Action::Dash) {
            return self.pad.button_down(p);
        }
        false
    }

    /// Turn the held-state change around one event into the frame's dash edges.
    fn note_dash_transition(&mut self, was: bool) {
        let now = self.dash_input_held();
        if now && !was {
            self.dash_down_edge = true;
            self.dash_fresh = true;
        }
        if was && !now {
            self.dash_up_edge = true;
        }
    }

    /// A held dash input vanished **without** a release (focus loss, unplugged
    /// pad): drop it and report the cancellation, never a release edge.
    fn cancel_dash(&mut self) {
        self.dash_kb_down = false;
        self.dash_mouse_down = false;
        self.dash_down_edge = false;
        self.dash_up_edge = false;
        self.dash_cancelled = true;
    }

    /// Is this key event the live dash input's key? A Ctrl binding on a terminal
    /// without lone-modifier reporting is delivered as Space (the pre-Ctrl
    /// mapping), so that is what counts there.
    fn dash_key_event(&self, ev: &KeyEvent) -> bool {
        let b = self.controls.active();
        if !self.lone_ctrl && b.get(Action::Dash) == Binding::Key(KeyBind::Ctrl) {
            return ev.code == KeyCode::Char(' ');
        }
        b.kbm_slots(Action::Dash)
            .iter()
            .any(|slot| matches!(slot, Binding::Key(k) if k.matches_movement(ev)))
    }

    /// Is this mouse button bound to the dash Action? (The classic left click,
    /// mouse aim's right click.)
    fn dash_button(&self, button: MouseButton) -> bool {
        self.controls
            .active()
            .kbm_slots(Action::Dash)
            .iter()
            .any(|slot| matches!(slot, Binding::Mouse(m) if m.button() == Some(button)))
    }

    /// Was the active-weapon release asked for this frame? Always Space: in the
    /// Ctrl mode that is Space's only job, and without lone-Ctrl reporting
    /// Space dashes at the same time (the press is shared, as it always was).
    pub fn active_pressed(&self) -> bool {
        // Keyboard and pad only: a mouse click is routed through the hit map,
        // where the app decides whether it also dashes.
        self.action_kbm_or_pad(Action::FireActives)
    }

    /// Decay held-key grace timers (no-op when the protocol's release events
    /// are used).
    pub fn tick(&mut self, dt: f32) {
        if !self.kitty {
            for g in self.grace.iter_mut() {
                *g = (*g - dt).max(0.0);
            }
        }
        self.tick_pad(dt);
    }

    /// Track a lone modifier press. Returns `true` when the event was only a
    /// modifier, so the caller stops there.
    ///
    /// A lone modifier is only ever a dash tap. It is deliberately never
    /// recorded in `pressed`: every menu, the options keys and the "any key"
    /// paths would otherwise see Ctrl as a plain keystroke.
    fn handle_modifier(&mut self, ev: KeyEvent) -> bool {
        let slot = match ev.code {
            KeyCode::Modifier(
                ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl,
            ) => 0,
            KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift) => 1,
            KeyCode::Modifier(ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt) => 2,
            _ => return false,
        };
        if slot != 0 {
            let was = self.dash_input_held();
            let dash_key = self.dash_key_event(&ev);
            // Shift/Alt are bindable (Controls screen) but never menu keys.
            if ev.kind == KeyEventKind::Press {
                self.mod_taps[slot] = true;
                self.last_device = Device::Keyboard;
            }
            // A rebindable lone modifier can be the dash key: only a terminal
            // that reports lone modifiers can report its release too, so the
            // hold is only ever tracked there.
            if dash_key {
                match ev.kind {
                    KeyEventKind::Press | KeyEventKind::Repeat => self.dash_kb_down = true,
                    KeyEventKind::Release => self.dash_kb_down = false,
                }
                self.note_dash_transition(was);
            }
            return true;
        }
        let was = self.dash_input_held();
        let dash_key = self.dash_key_event(&ev);
        match ev.kind {
            KeyEventKind::Press => {
                self.ctrl_down = true;
                self.dash_tap = true;
                self.mod_taps[0] = true;
                self.last_device = Device::Keyboard;
            }
            // Some terminals auto-repeat a held modifier. Swallow it: a held
            // Ctrl must not machine-gun dashes.
            KeyEventKind::Repeat => {}
            KeyEventKind::Release => self.ctrl_down = false,
        }
        // The Ctrl hold: this is the Super Dash charge on a terminal that
        // reports a lone modifier press (the flag that also reports its up).
        if dash_key {
            match ev.kind {
                KeyEventKind::Press | KeyEventKind::Repeat => self.dash_kb_down = true,
                KeyEventKind::Release => self.dash_kb_down = false,
            }
            self.note_dash_transition(was);
        }
        true
    }

    /// Which movement direction a key event asks for: the current scheme's
    /// movement bindings first, then the arrow keys (which always move, since
    /// they also steer the menus).
    fn movement_dir_for(&self, ev: &KeyEvent) -> Option<Dir> {
        let b = self.controls.active();
        for (action, dir) in [
            (Action::MoveUp, Dir::Up),
            (Action::MoveDown, Dir::Down),
            (Action::MoveLeft, Dir::Left),
            (Action::MoveRight, Dir::Right),
        ] {
            if b.kbm_slots(action)
                .iter()
                .any(|slot| matches!(slot, Binding::Key(k) if k.matches_movement(ev)))
            {
                return Some(dir);
            }
        }
        match ev.code {
            KeyCode::Up => Some(Dir::Up),
            KeyCode::Down => Some(Dir::Down),
            KeyCode::Left => Some(Dir::Left),
            KeyCode::Right => Some(Dir::Right),
            _ => None,
        }
    }

    pub fn handle_key(&mut self, ev: KeyEvent) {
        if self.handle_modifier(ev) {
            return;
        }
        self.last_device = Device::Keyboard;
        let dir = self.movement_dir_for(&ev);

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
        // Dash hold edges (the Super Dash charge). A key counts as held only
        // where the terminal reports its release; a terminal that cannot report
        // one can only tap, and its auto-repeat is never a release or a re-press.
        if self.dash_key_event(&ev) && !matches!(ev.code, KeyCode::Modifier(_)) {
            let was = self.dash_input_held();
            match ev.kind {
                KeyEventKind::Press => {
                    if self.kitty {
                        self.dash_kb_down = true;
                        self.note_dash_transition(was)
                    } else {
                        // No key-up to come: the press *is* the gesture (a tap).
                        self.dash_down_edge = true;
                        self.dash_fresh = true;
                    }
                }
                KeyEventKind::Repeat => {}
                KeyEventKind::Release => {
                    self.dash_kb_down = false;
                    self.note_dash_transition(was)
                }
            }
        }
    }

    pub fn handle_mouse(&mut self, ev: MouseEvent) {
        if !matches!(ev.kind, MouseEventKind::Up(_)) {
            self.last_device = Device::Mouse;
        }
        let moved = self.mouse_pos != Some((ev.column, ev.row));
        if matches!(
            ev.kind,
            MouseEventKind::Down(
                MouseButton::Left | MouseButton::Right | MouseButton::Middle
            ) | MouseEventKind::ScrollUp
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
        // Dash hold edges: a mouse button bound to the dash (the classic left
        // click, mouse aim's right click) is held until its Up — and a terminal
        // always reports a mouse release, whatever the keyboard protocol says.
        if let MouseEventKind::Down(b) | MouseEventKind::Up(b) = ev.kind {
            if self.dash_button(b) {
                let was = self.dash_input_held();
                self.dash_mouse_down = matches!(ev.kind, MouseEventKind::Down(_));
                self.note_dash_transition(was);
            }
        }
    }

    /// Forget every held movement key (and mouse button). Called when the
    /// terminal loses focus: a release for a key or button that was down at
    /// that moment never arrives, so held state would otherwise stick forever.
    pub fn release_all(&mut self) {
        // A held dash input is dropped here: focus loss and a pad unplug never
        // report their release, so the Super Dash charge is *cancelled* rather
        // than fired (see `cancel_dash`).
        let dash_held = self.dash_input_held();
        self.grace = [0.0; 4];
        self.mouse_down = false;
        // A Ctrl held across a focus change never reports its release either.
        self.ctrl_down = false;
        self.dash_kb_down = false;
        self.dash_mouse_down = false;
        self.pad.set_focused(false);
        if dash_held {
            self.cancel_dash();
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
        // Keyboard directions are digital (a diagonal is unit length), the pad
        // adds its analog throttle; the sum is clamped so a diagonal is never
        // faster than a cardinal.
        let pad = self.pad.move_vec(self.controls.deadzone);
        if pad != Vec2::ZERO {
            return (v + pad).clamp_len(1.0);
        }
        v.normalized()
    }

    // ---- Actions ---------------------------------------------------------

    /// The lone modifier tapped this frame, if any (the Controls screen's
    /// rebinding needs it: a modifier press never enters `pressed`).
    pub fn modifier_tap(&self) -> Option<KeyBind> {
        match self.mod_taps {
            [true, _, _] => Some(KeyBind::Ctrl),
            [_, true, _] => Some(KeyBind::Shift),
            [_, _, true] => Some(KeyBind::Alt),
            _ => None,
        }
    }

    /// Was `a` tapped by a keyboard key or a pad button this frame?
    pub fn action_kbm_or_pad(&self, a: Action) -> bool {
        let b = self.controls.active();
        for slot in b.kbm_slots(a) {
            match slot {
                Binding::Key(k) if self.key_tapped(k) => return true,
                Binding::Pad(p) if self.pad.button_pressed(p) => return true,
                _ => {}
            }
        }
        // Pad bindings live in their own slot, not in the keyboard/mouse pair.
        if let Binding::Pad(p) = b.pad_of(a) {
            if self.pad.button_pressed(p) {
                return true;
            }
        }
        false
    }

    /// Was `a` tapped this frame by any device, mouse included?
    pub fn action_pressed(&self, a: Action) -> bool {
        if self.action_kbm_or_pad(a) {
            return true;
        }
        self.controls
            .active()
            .kbm_slots(a)
            .iter()
            .any(|slot| matches!(slot, Binding::Mouse(m) if self.mouse_tapped(*m)))
    }

    /// Was `a`'s key or pad button held down this frame? (Movement, and the
    /// options screen's held adjust.)
    pub fn action_held(&self, a: Action) -> bool {
        let b = self.controls.active();
        for slot in b.kbm_slots(a) {
            match slot {
                Binding::Key(k) => {
                    if self
                        .pressed
                        .iter()
                        .any(|e| k.matches(e) && e.kind != KeyEventKind::Release)
                    {
                        return true;
                    }
                }
                Binding::Pad(p) if self.pad.button_down(p) => return true,
                _ => {}
            }
        }
        if let Binding::Pad(p) = b.pad_of(a) {
            if self.pad.button_down(p) {
                return true;
            }
        }
        false
    }

    fn key_tapped(&self, k: KeyBind) -> bool {
        // A lone-modifier binding can only fire where the terminal reports lone
        // modifiers (the Kitty protocol's report-all-keys flag). On any other
        // terminal Ctrl still dashes through the legacy Space fallback, which
        // `dash_pressed` handles; the Controls screen says so.
        match k {
            KeyBind::Ctrl => self.lone_ctrl && self.mod_taps[0],
            KeyBind::Shift => self.lone_ctrl && self.mod_taps[1],
            KeyBind::Alt => self.lone_ctrl && self.mod_taps[2],
            _ => self.pressed.iter().any(|e| k.matches(e)),
        }
    }

    /// Pad B only (the pause/end screens' "back to the menu": the keyboard's
    /// Esc is resume there, so the two cannot share one predicate).
    pub fn pad_back(&self) -> bool {
        self.pad.button_pressed(crate::controls::PadBind::East)
    }

    /// Did a mouse press this frame match `m`?
    pub fn mouse_tapped(&self, m: MouseBind) -> bool {
        self.mouse_events.iter().any(|e| match (m, e.kind) {
            (MouseBind::WheelUp, MouseEventKind::ScrollUp) => true,
            (MouseBind::WheelDown, MouseEventKind::ScrollDown) => true,
            (_, MouseEventKind::Down(b)) => m.button() == Some(b),
            _ => false,
        })
    }

    /// A discrete menu-navigation step towards `(dx, dy)`: an arrow key, a
    /// movement binding, or the pad's d-pad/stick with its repeat clock.
    pub fn nav_pressed(&self, dx: i32, dy: i32) -> bool {
        if self.pad.nav_step() == Some((dx, dy)) {
            return true;
        }
        let want = match (dx, dy) {
            (0, -1) => KeyCode::Up,
            (0, 1) => KeyCode::Down,
            (-1, 0) => KeyCode::Left,
            (1, 0) => KeyCode::Right,
            _ => return false,
        };
        if self.just_pressed(want) {
            return true;
        }
        let action = match (dx, dy) {
            (0, -1) => Action::MoveUp,
            (0, 1) => Action::MoveDown,
            (-1, 0) => Action::MoveLeft,
            (1, 0) => Action::MoveRight,
            _ => return false,
        };
        self.action_kbm_or_pad(action)
    }

    /// Enter / pad A.
    pub fn confirm_pressed(&self) -> bool {
        self.action_pressed(Action::Confirm)
    }

    /// Esc / pad B.
    pub fn back_pressed(&self) -> bool {
        self.action_pressed(Action::Back)
    }

    /// Any of the four menu directions this frame, as `(dx, dy)`.
    pub fn nav_any(&self) -> Option<(i32, i32)> {
        [(0, -1), (0, 1), (-1, 0), (1, 0)]
            .into_iter()
            .find(|&d| self.nav_pressed(d.0, d.1))
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
        // Both legends must name the live key and what Space does, and the
        // click's two jobs (`WAVE3 review`, "Controls and full racks").
        for wide in [true, false] {
            let ctrl = DashKey::Ctrl.title_legend_lines(wide).join(" ");
            assert!(ctrl.contains("Ctrl dash"), "{ctrl}");
            assert!(ctrl.contains("Space actives"), "{ctrl}");
            assert!(ctrl.contains("click = dash + ready actives"), "{ctrl}");
            let space = DashKey::Space.title_legend_lines(wide).join(" ");
            assert!(space.contains("Space dash + actives"), "{space}");
            assert!(space.contains("click = dash + ready actives"), "{space}");
        }
        // Every live control is named by the title legend and by the pause one.
        for mode in [DashKey::Ctrl, DashKey::Space] {
            let title = mode.title_legend_lines(true).join(" ");
            for needle in [
                "move",
                "dash",
                "actives",
                "click",
                "zoom",
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
            let pause = mode.pause_legend_lines(true).join(" ");
            for needle in [
                "move",
                "dash",
                "actives",
                "click",
                "[ ] music",
                "- = effects",
                "O options",
                "Esc/P unpause",
            ] {
                assert!(pause.contains(needle), "pause legend misses {needle:?}: {pause}");
            }
        }
        // Narrow plates keep movement and drop nothing important first: the
        // first line is the one a short plate can afford.
        assert!(DashKey::Ctrl.title_legend_lines(false)[0].contains("move"));
        assert!(DashKey::Space.title_legend_lines(false)[0].contains("move"));
        // The narrow pause legend still names movement, the dash, the actives,
        // the click and the way back out of mute.
        for mode in [DashKey::Ctrl, DashKey::Space] {
            let narrow = mode.pause_legend_lines(false).join(" ");
            for needle in ["move", "dash", "actives", "click", "Esc/P unpause"] {
                assert!(narrow.contains(needle), "narrow pause legend misses {needle:?}: {narrow}");
            }
            // …and every legend line fits the 58-column card's inner width.
            for line in mode.pause_legend_lines(false) {
                assert!(line.chars().count() <= 54, "{line:?} is wider than the card");
            }
            for line in mode.pause_legend_lines(true) {
                assert!(line.chars().count() <= 72, "{line:?} is wider than the dossier");
            }
        }
        // Switching the mode after construction (main's confirmed answer).
        let mut s = InputState::new(true);
        s.set_lone_ctrl(false);
        assert_eq!(s.dash_key(), DashKey::Space);
        s.set_lone_ctrl(true);
        assert_eq!(s.dash_key(), DashKey::Ctrl);
    }

    // ---- Super Dash: hold to charge, release to fire ----------------------

    /// A Kitty-protocol key reports its release, so the dash has all three
    /// edges: the press starts a charge, the hold fills it, the key-up fires it.
    /// Auto-repeat is neither a release nor a re-press.
    #[test]
    fn dash_key_down_held_and_up_in_kitty_mode() {
        let mut s = InputState::new(true);
        assert!(s.dash_key_holds(), "the protocol reports key releases");
        s.begin_frame();
        s.handle_key(ctrl(KeyEventKind::Press));
        assert!(s.dash_down(), "the press edge must report");
        assert!(s.dash_held(), "Ctrl is held until its release");
        assert!(s.dash_pressed_fresh());
        assert!(!s.dash_up());

        s.begin_frame();
        s.handle_key(ctrl(KeyEventKind::Repeat));
        assert!(!s.dash_down(), "auto-repeat is never a re-press");
        assert!(!s.dash_up(), "auto-repeat is never a release");
        assert!(s.dash_held());

        s.begin_frame();
        s.handle_key(ctrl(KeyEventKind::Release));
        assert!(s.dash_up(), "the key-up is the release edge");
        assert!(!s.dash_held());
        assert!(!s.dash_down());
        assert!(!s.dash_cancelled());
    }

    /// A mouse button charges in either input mode: a terminal always reports
    /// the button's Up, whatever the keyboard protocol can do.
    #[test]
    fn dash_mouse_button_down_held_and_up() {
        for kitty in [true, false] {
            let mut s = InputState::new(kitty);
            s.begin_frame();
            s.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, 5));
            assert!(s.dash_down(), "LMB down must report (kitty={kitty})");
            assert!(s.dash_held(), "LMB charges (kitty={kitty})");
            s.begin_frame();
            assert!(!s.dash_down() && !s.dash_up());
            s.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 4, 5));
            assert!(s.dash_up(), "LMB up must fire (kitty={kitty})");
            assert!(!s.dash_held());
        }
    }

    /// The pad's dash button arrives through the injection seam: down holds,
    /// up releases (and the pad's own released flag is set for the frame).
    #[test]
    fn dash_pad_button_down_held_and_up() {
        use crate::controls::PadBind;
        let mut s = InputState::new(false);
        s.pad_event(PadEvent::Connected);
        s.begin_frame();
        s.pad_event(PadEvent::Button(PadBind::R1, true));
        assert!(s.dash_down() && s.dash_held());
        s.begin_frame();
        assert!(!s.dash_down() && s.dash_held());
        s.pad_event(PadEvent::Button(PadBind::R1, false));
        assert!(s.dash_up() && !s.dash_held());
        assert!(s.pad.button_released(PadBind::R1));
    }

    /// The fallback: a terminal that cannot report key releases can only tap.
    /// The press still reports its edge (so the Super Dash fires an ordinary
    /// tap dash) but there is never a held state or a release edge to charge
    /// with — the mouse and the pad are how that terminal charges.
    #[test]
    fn fallback_keys_tap_and_the_mouse_charges() {
        let mut s = InputState::new(false);
        assert!(!s.dash_key_holds());
        s.begin_frame();
        s.handle_key(key(KeyCode::Char(' '), KeyEventKind::Press));
        assert!(s.dash_down(), "the tap must still report its press");
        assert!(s.dash_pressed_fresh());
        assert!(!s.dash_held(), "a key with no release cannot hold");

        // Its auto-repeat never re-presses (the old `pressed` list does keep
        // the repeat for the legacy dash path; the fresh edge does not).
        s.begin_frame();
        s.handle_key(key(KeyCode::Char(' '), KeyEventKind::Repeat));
        assert!(!s.dash_down() && !s.dash_held() && !s.dash_up());
        assert!(!s.dash_pressed_fresh(), "a repeat is not a fresh press");

        // The mouse still charges: its release always arrives.
        s.begin_frame();
        s.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));
        assert!(s.dash_down() && s.dash_held());
        s.begin_frame();
        s.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, 2));
        assert!(s.dash_up() && !s.dash_held());
    }

    /// Losing the terminal's focus while charging drops the hold: there is no
    /// release edge, so the charge is cancelled rather than fired.
    #[test]
    fn focus_loss_cancels_a_dash_hold_without_a_release_edge() {
        let mut s = InputState::new(true);
        s.begin_frame();
        s.handle_key(ctrl(KeyEventKind::Press));
        assert!(s.dash_held());
        s.begin_frame();
        s.set_focused(false);
        assert!(!s.dash_held(), "the held input must be dropped");
        assert!(!s.dash_up(), "a focus loss must never look like a release");
        assert!(s.dash_cancelled(), "the charge must be cancelled, not fired");
        assert!(!s.dash_down());
    }

    /// A pad unplugged mid-charge cancels it too: its release never arrives.
    #[test]
    fn pad_unplug_cancels_a_dash_hold() {
        use crate::controls::PadBind;
        let mut s = InputState::new(false);
        s.pad_event(PadEvent::Connected);
        s.begin_frame();
        s.pad_event(PadEvent::Button(PadBind::R1, true));
        assert!(s.dash_held());
        s.begin_frame();
        s.pad_event(PadEvent::Disconnected);
        assert!(!s.dash_held());
        assert!(!s.dash_up());
        assert!(s.dash_cancelled());
    }
}
