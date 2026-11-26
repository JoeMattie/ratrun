//! Gamepad support for a terminal game.
//!
//! A terminal app has no window, so a controller is read whether or not the
//! terminal has focus — which is why the app feeds terminal focus reporting
//! into [`PadState::set_focused`] and drops pad input while unfocused (a pad
//! resting in someone's lap must not drive a game nobody is watching).
//!
//! The gilrs integration is a thin poller behind the `gamepad` cargo feature
//! (on by default). Everything the rest of the game sees is [`PadState`] and
//! [`PadEvent`], so tests drive the whole path — movement, aim, dash, actives,
//! pause, menu navigation, hot-plug, dead zone — with **injected events** and
//! never need a real controller or a device node.
//!
//! Platform notes: on Linux gilrs links `libudev`, so the CI jobs install
//! `libudev-dev` and the crate still builds with `--no-default-features`
//! (gilrs is an optional dependency). Without the feature, or with no pad
//! plugged in, every function here is a no-op that costs one branch per frame.

use crate::controls::{PadBind, PAD_BINDS, PAD_BIND_COUNT};
use crate::math::Vec2;

/// Analog stick identity for [`PadEvent::Stick`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stick {
    Left,
    Right,
}

/// One injected pad event. gilrs events are translated into these; tests
/// inject them directly (the seam the task asks for).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PadEvent {
    Connected,
    Disconnected,
    /// A button went down (`true`) or up (`false`).
    Button(PadBind, bool),
    /// An analog stick's position, each axis `-1..=1`, `y` positive down
    /// (screen space, matching the game's world axes).
    Stick(Stick, Vec2),
}

/// Menu-navigation hold: how long a direction must be held before the second
/// step, then how fast the steps repeat.
const NAV_DELAY: f32 = 0.24;
const NAV_REPEAT: f32 = 0.10;

/// The longest single rumble a request can ask for, in seconds.
pub const MAX_RUMBLE_SECS: f32 = 1.0;

/// The live pad state. Cheap to copy-free update; no allocation.
#[derive(Clone, Debug, Default)]
pub struct PadState {
    connected: bool,
    down: [bool; PAD_BIND_COUNT],
    pressed: [bool; PAD_BIND_COUNT],
    released: [bool; PAD_BIND_COUNT],
    left: Vec2,
    right: Vec2,
    /// One-frame menu-navigation step (d-pad/stick, with repeat).
    nav_step: Option<(i32, i32)>,
    nav_dir: (i32, i32),
    nav_timer: f32,
    nav_repeats: bool,
    /// True while the app has terminal focus; pad input is ignored otherwise.
    focused: bool,
    /// Set when any pad activity arrived this frame (device-hint switching).
    active: bool,
    /// Rumble seconds still to run (drained by the poller).
    pub rumble_left: f32,
}

impl PadState {
    pub fn new() -> PadState {
        PadState {
            focused: true,
            ..PadState::default()
        }
    }

    /// Is a controller plugged in?
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Did the pad drive input this frame?
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn set_connected(&mut self, on: bool) {
        self.connected = on;
        if !on {
            self.down = [false; PAD_BIND_COUNT];
            self.left = Vec2::ZERO;
            self.right = Vec2::ZERO;
            self.nav_step = None;
            self.nav_dir = (0, 0);
            self.rumble_left = 0.0;
        }
    }

    /// Terminal focus reporting: ignore pad input while unfocused.
    pub fn set_focused(&mut self, on: bool) {
        self.focused = on;
        if !on {
            // A button or stick released while unfocused never reports, so the
            // held state would stick.
            self.down = [false; PAD_BIND_COUNT];
            self.left = Vec2::ZERO;
            self.right = Vec2::ZERO;
            self.nav_step = None;
            self.nav_dir = (0, 0);
            self.rumble_left = 0.0;
        }
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Apply one event. Ignored entirely while the terminal is unfocused (or
    /// for a disconnected pad).
    pub fn apply(&mut self, ev: PadEvent) {
        match ev {
            PadEvent::Connected => {
                self.connected = true;
                if self.focused {
                    self.active = true;
                }
            }
            PadEvent::Disconnected => {
                self.set_connected(false);
                self.active = true;
            }
            PadEvent::Button(b, down) => {
                if !self.focused || !self.connected {
                    return;
                }
                self.active = true;
                let i = b.index();
                if down && !self.down[i] {
                    self.pressed[i] = true;
                }
                if !down && self.down[i] {
                    self.released[i] = true;
                }
                self.down[i] = down;
            }
            PadEvent::Stick(stick, v) => {
                if !self.focused || !self.connected {
                    return;
                }
                let v = Vec2::new(v.x.clamp(-1.0, 1.0), v.y.clamp(-1.0, 1.0));
                if v.len_sq() > 0.01 {
                    self.active = true;
                }
                match stick {
                    Stick::Left => self.left = v,
                    Stick::Right => self.right = v,
                }
            }
        }
    }

    /// Clear the per-frame press/release/nav/activity flags.
    pub fn begin_frame(&mut self) {
        self.pressed = [false; PAD_BIND_COUNT];
        self.released = [false; PAD_BIND_COUNT];
        self.nav_step = None;
        self.active = false;
    }

    /// Advance the menu-navigation repeat clock and derive this frame's step
    /// from the held d-pad/stick.
    pub fn step(&mut self, dt: f32, deadzone: f32) {
        let stick = self.stick_with_deadzone(self.left, deadzone);
        // Digital d-pad first, then the stick with a 0.5 gate (a stick held for
        // movement must not also scroll a menu; menus are not the playfield).
        let mut dir = (0, 0);
        if self.down[PadBind::DPadUp.index()] {
            dir.1 -= 1;
        }
        if self.down[PadBind::DPadDown.index()] {
            dir.1 += 1;
        }
        if self.down[PadBind::DPadLeft.index()] {
            dir.0 -= 1;
        }
        if self.down[PadBind::DPadRight.index()] {
            dir.0 += 1;
        }
        if dir == (0, 0) {
            if stick.x.abs() >= 0.5 {
                dir.0 = stick.x.signum() as i32;
            }
            if stick.y.abs() >= 0.5 {
                dir.1 = stick.y.signum() as i32;
            }
        }
        if dir == (0, 0) {
            self.nav_dir = (0, 0);
            self.nav_repeats = false;
            self.nav_timer = 0.0;
            return;
        }
        if dir != self.nav_dir {
            self.nav_dir = dir;
            self.nav_repeats = false;
            self.nav_timer = NAV_DELAY;
            self.nav_step = Some(dir);
            return;
        }
        self.nav_timer -= dt;
        if self.nav_timer <= 0.0 {
            self.nav_repeats = true;
            self.nav_timer = NAV_REPEAT;
            self.nav_step = Some(dir);
        }
    }

    fn stick_with_deadzone(&self, v: Vec2, deadzone: f32) -> Vec2 {
        let dead = deadzone.clamp(0.0, 0.95);
        let len = v.len();
        if len <= dead {
            return Vec2::ZERO;
        }
        // Rescale the live part back to 0..1 so a fully deflected stick is 1.
        let scaled = ((len - dead) / (1.0 - dead)).clamp(0.0, 1.0);
        v * (scaled / len)
    }

    /// The left stick's movement vector, dead-zoned and rescaled.
    pub fn move_vec(&self, deadzone: f32) -> Vec2 {
        if !self.connected() || !self.focused {
            return Vec2::ZERO;
        }
        let stick = self.stick_with_deadzone(self.left, deadzone);
        let mut v = stick.clamp_len(1.0);
        // The d-pad is digital and adds to the stick; the sum is clamped so a
        // diagonal is never faster than a cardinal.
        let mut dpad = Vec2::ZERO;
        if self.down[PadBind::DPadUp.index()] {
            dpad.y -= 1.0;
        }
        if self.down[PadBind::DPadDown.index()] {
            dpad.y += 1.0;
        }
        if self.down[PadBind::DPadLeft.index()] {
            dpad.x -= 1.0;
        }
        if self.down[PadBind::DPadRight.index()] {
            dpad.x += 1.0;
        }
        v.x += dpad.x;
        v.y += dpad.y;
        v.clamp_len(1.0)
    }

    /// The right stick's aim vector, or `None` while it is centred.
    pub fn aim_vec(&self, deadzone: f32) -> Option<Vec2> {
        if !self.connected() || !self.focused {
            return None;
        }
        let v = self.stick_with_deadzone(self.right, deadzone);
        if v.len_sq() > 1e-4 {
            Some(v.clamp_len(1.0))
        } else {
            None
        }
    }

    /// Was any pad button bound to `b` pressed this frame?
    pub fn button_pressed(&self, b: PadBind) -> bool {
        self.pressed[b.index()]
    }

    pub fn button_down(&self, b: PadBind) -> bool {
        self.down[b.index()]
    }

    /// Did `b` go **up** this frame? (A released pad button is the end of a
    /// held dash charge; focus loss and unplugs drop the state without it, so a
    /// lost pad cancels rather than fires.)
    pub fn button_released(&self, b: PadBind) -> bool {
        self.released[b.index()]
    }

    /// This frame's menu-navigation step (d-pad/stick with repeat).
    pub fn nav_step(&self) -> Option<(i32, i32)> {
        self.nav_step
    }

    /// Which pad button went down this frame (the rebinding capture path).
    pub fn pressed_button(&self) -> Option<PadBind> {
        PAD_BINDS
            .iter()
            .copied()
            .find(|b| self.pressed[b.index()])
    }

    /// Request a rumble of `secs` seconds. Ignored without a connected pad or
    /// while the terminal is unfocused, so a request can never sit in the queue
    /// and buzz a pad plugged in minutes later. The **caller** gates on the
    /// player's preference (`Controls::rumble`); see [`PadState::rumble_if`].
    pub fn rumble(&mut self, secs: f32) {
        if !self.connected || !self.focused || !secs.is_finite() || secs <= 0.0 {
            return;
        }
        self.rumble_left = self.rumble_left.max(secs.min(MAX_RUMBLE_SECS));
    }

    /// [`PadState::rumble`] behind the Options toggle: the one-line call site
    /// for the app (`pad.rumble_if(controls.rumble, 0.12)` on a hit).
    pub fn rumble_if(&mut self, enabled: bool, secs: f32) {
        if enabled {
            self.rumble(secs);
        }
    }
}

/// One analog stick as gilrs reports it, in the game's convention.
///
/// gilrs normalises every pad so **up is +1**; the game's axes are screen
/// space (`y` positive down, see [`PadEvent::Stick`]), so `y` is negated here —
/// without it pushing the stick up walks the rat down. A deflection inside the
/// dead zone is snapped to zero *before* it reaches [`PadState::apply`], so a
/// drifting stick at rest neither counts as pad activity (which would flip the
/// legends to pad glyphs under a keyboard player) nor moves anything.
fn stick_from_gilrs(x: f32, y_up: f32, deadzone: f32) -> Vec2 {
    let v = Vec2::new(x, -y_up);
    if !v.x.is_finite() || !v.y.is_finite() || v.len() <= deadzone.clamp(0.0, 0.95) {
        return Vec2::ZERO;
    }
    v
}

/// The gilrs-backed poller. With the `gamepad` feature off, this type is an
/// empty no-op so the whole file still builds and every call site works.
pub struct PadPoller {
    #[cfg(all(feature = "gamepad", not(target_arch = "wasm32")))]
    gilrs: Option<gilrs::Gilrs>,
    #[cfg(all(feature = "gamepad", not(target_arch = "wasm32")))]
    pad: Option<gilrs::GamepadId>,
    #[cfg(all(feature = "gamepad", not(target_arch = "wasm32")))]
    was_connected: bool,
    /// One cached short rumble effect (rebuilt when the pad changes).
    #[cfg(all(feature = "gamepad", not(target_arch = "wasm32")))]
    effect: Option<gilrs::ff::Effect>,
}

impl Default for PadPoller {
    fn default() -> Self {
        PadPoller::new()
    }
}

impl PadPoller {
    #[cfg(all(feature = "gamepad", not(target_arch = "wasm32")))]
    pub fn new() -> PadPoller {
        // A machine with no controller (or no permission to read it) is normal:
        // `Gilrs::new` failing is not an error, it is "no gamepad today".
        PadPoller {
            gilrs: gilrs::Gilrs::new().ok(),
            pad: None,
            was_connected: false,
            effect: None,
        }
    }

    #[cfg(any(not(feature = "gamepad"), target_arch = "wasm32"))]
    pub fn new() -> PadPoller {
        PadPoller {}
    }

    /// Poll the pad once per frame: drain gilrs' event queue into `state`
    /// and refresh the analog sticks. Returns whether anything arrived.
    ///
    /// `state` must already have had `begin_frame` and `step` this frame.
    #[cfg(all(feature = "gamepad", not(target_arch = "wasm32")))]
    pub fn poll(&mut self, state: &mut PadState, deadzone: f32) -> bool {
        use gilrs::{Axis, Button};

        let Some(g) = self.gilrs.as_mut() else {
            return false;
        };
        let mut any = false;
        while let Some(ev) = g.next_event() {
            use gilrs::EventType;
            any = true;
            match ev.event {
                EventType::Connected => {
                    self.pad.get_or_insert(ev.id);
                    state.apply(PadEvent::Connected);
                }
                EventType::Disconnected if self.pad == Some(ev.id) => {
                    self.pad = None;
                    self.effect = None;
                    self.was_connected = false;
                    state.apply(PadEvent::Disconnected);
                }
                _ => {}
            }
        }
        // The first pad gilrs knows about is the one the player is holding.
        if self.pad.is_none() {
            if let Some((id, _)) = g.gamepads().find(|(_, p)| p.is_connected()) {
                self.pad = Some(id);
                state.apply(PadEvent::Connected);
            }
        }
        let Some(id) = self.pad else {
            if self.was_connected {
                state.apply(PadEvent::Disconnected);
                self.was_connected = false;
            }
            return any;
        };
        let pad = g.gamepad(id);
        if !pad.is_connected() {
            self.pad = None;
            // The cached effect is bound to the pad that just left.
            self.effect = None;
            self.was_connected = false;
            state.apply(PadEvent::Disconnected);
            return any;
        }
        self.was_connected = true;
        state.set_connected(true);
        for (gb, pb) in [
            (Button::South, PadBind::South),
            (Button::East, PadBind::East),
            (Button::North, PadBind::North),
            (Button::West, PadBind::West),
            (Button::Start, PadBind::Start),
            (Button::Select, PadBind::Select),
            (Button::LeftTrigger, PadBind::L1),
            (Button::RightTrigger, PadBind::R1),
            (Button::LeftTrigger2, PadBind::L2),
            (Button::RightTrigger2, PadBind::R2),
            (Button::LeftThumb, PadBind::L3),
            (Button::RightThumb, PadBind::R3),
            (Button::DPadUp, PadBind::DPadUp),
            (Button::DPadDown, PadBind::DPadDown),
            (Button::DPadLeft, PadBind::DPadLeft),
            (Button::DPadRight, PadBind::DPadRight),
        ] {
            let down = pad.is_pressed(gb);
            if down != state.button_down(pb) {
                state.apply(PadEvent::Button(pb, down));
                any = true;
            }
        }
        let left = stick_from_gilrs(
            pad.value(Axis::LeftStickX),
            pad.value(Axis::LeftStickY),
            deadzone,
        );
        let right = stick_from_gilrs(
            pad.value(Axis::RightStickX),
            pad.value(Axis::RightStickY),
            deadzone,
        );
        state.apply(PadEvent::Stick(Stick::Left, left));
        state.apply(PadEvent::Stick(Stick::Right, right));
        // Rumble drains here so the pad driver is only ever touched from the
        // poll (never from the simulation or a test). Off by default in the
        // options; a pad without force feedback simply does nothing.
        if state.rumble_left > 0.0 {
            let ms = (state.rumble_left.min(MAX_RUMBLE_SECS) * 1000.0).round().max(1.0) as u32;
            state.rumble_left = 0.0;
            if pad.is_ff_supported() {
                use gilrs::ff::{Repeat, Ticks};
                if self.effect.is_none() {
                    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay};
                    // The base effects run for as long as any request may ask;
                    // the effect's *repeat* window (set per play below) is what
                    // ends it. gilrs' default is `Repeat::Infinitely`, which
                    // would leave the motors running until the pad is unplugged.
                    let play_for = Ticks::from_ms((MAX_RUMBLE_SECS * 1000.0) as u32);
                    self.effect = EffectBuilder::new()
                        .add_effect(BaseEffect {
                            kind: BaseEffectType::Strong { magnitude: 42_000 },
                            scheduling: Replay {
                                play_for,
                                ..Default::default()
                            },
                            envelope: Default::default(),
                        })
                        .add_effect(BaseEffect {
                            kind: BaseEffectType::Weak { magnitude: 42_000 },
                            scheduling: Replay {
                                play_for,
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .repeat(Repeat::For(Ticks::from_ms(ms)))
                        .gamepads(&[id])
                        .finish(g)
                        .ok();
                }
                if let Some(effect) = self.effect.as_ref() {
                    // `play` restarts the effect's clock; the repeat window
                    // makes this request stop itself after `ms`.
                    let _ = effect.set_repeat(Repeat::For(Ticks::from_ms(ms)));
                    let _ = effect.play();
                }
            }
        }
        any
    }

    #[cfg(any(not(feature = "gamepad"), target_arch = "wasm32"))]
    pub fn poll(&mut self, _state: &mut PadState, _deadzone: f32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad() -> PadState {
        let mut p = PadState::new();
        p.apply(PadEvent::Connected);
        p
    }

    #[test]
    fn no_pad_means_no_input() {
        let mut p = PadState::new();
        p.apply(PadEvent::Stick(Stick::Left, Vec2::new(1.0, 0.0)));
        p.apply(PadEvent::Button(PadBind::South, true));
        p.step(0.016, 0.2);
        assert_eq!(p.move_vec(0.2), Vec2::ZERO);
        assert!(!p.button_pressed(PadBind::South));
        assert_eq!(p.aim_vec(0.2), None);
        assert!(!p.connected());
    }

    #[test]
    fn the_dead_zone_zeroes_a_resting_stick_and_rescales_the_live_part() {
        let mut p = pad();
        p.apply(PadEvent::Stick(Stick::Left, Vec2::new(0.15, 0.0)));
        assert_eq!(p.move_vec(0.2), Vec2::ZERO, "inside the dead zone");
        p.apply(PadEvent::Stick(Stick::Left, Vec2::new(0.6, 0.0)));
        let v = p.move_vec(0.2);
        assert!((v.x - 0.5).abs() < 1e-5, "rescaled to {v:?}");
        p.apply(PadEvent::Stick(Stick::Left, Vec2::new(1.0, 1.0)));
        assert!((p.move_vec(0.2).len() - 1.0).abs() < 1e-5, "clamped to 1");
    }

    #[test]
    fn movement_is_analog_and_the_dpad_is_digital() {
        let mut p = pad();
        p.apply(PadEvent::Stick(Stick::Left, Vec2::new(0.0, -0.6)));
        let v = p.move_vec(0.2);
        assert!(v.y < -0.4 && v.y > -0.6, "half deflection: {v:?}");
        p.apply(PadEvent::Button(PadBind::DPadRight, true));
        let v = p.move_vec(0.2);
        assert!(v.x > 0.85 && v.y < -0.4, "d-pad adds: {v:?}");
    }

    #[test]
    fn aim_is_none_while_the_right_stick_is_centred() {
        let mut p = pad();
        assert_eq!(p.aim_vec(0.2), None);
        p.apply(PadEvent::Stick(Stick::Right, Vec2::new(0.9, 0.0)));
        assert!(p.aim_vec(0.2).is_some());
        p.apply(PadEvent::Stick(Stick::Right, Vec2::new(0.05, 0.0)));
        assert_eq!(p.aim_vec(0.2), None);
    }

    #[test]
    fn menu_navigation_steps_once_then_repeats() {
        let mut p = pad();
        p.apply(PadEvent::Button(PadBind::DPadDown, true));
        p.step(0.016, 0.2);
        assert_eq!(p.nav_step(), Some((0, 1)), "the first press is immediate");
        p.begin_frame();
        p.step(0.10, 0.2);
        assert_eq!(p.nav_step(), None, "no repeat before the delay");
        p.begin_frame();
        p.step(0.2, 0.2);
        assert_eq!(p.nav_step(), Some((0, 1)), "then it repeats");
        p.begin_frame();
        p.apply(PadEvent::Button(PadBind::DPadDown, false));
        p.step(0.2, 0.2);
        assert_eq!(p.nav_step(), None);
    }

    #[test]
    fn unfocused_ignores_the_pad_and_clears_held_state() {
        let mut p = pad();
        p.apply(PadEvent::Button(PadBind::South, true));
        p.apply(PadEvent::Stick(Stick::Left, Vec2::new(1.0, 0.0)));
        assert!(p.button_down(PadBind::South));
        p.set_focused(false);
        assert!(!p.button_down(PadBind::South));
        assert_eq!(p.move_vec(0.2), Vec2::ZERO);
        p.apply(PadEvent::Button(PadBind::South, true));
        assert!(!p.button_down(PadBind::South), "unfocused pad is ignored");
        p.set_focused(true);
        p.apply(PadEvent::Button(PadBind::South, true));
        assert!(p.button_down(PadBind::South));
    }

    #[test]
    fn hot_plug_disconnect_drops_every_held_button() {
        let mut p = pad();
        p.apply(PadEvent::Button(PadBind::South, true));
        p.apply(PadEvent::Disconnected);
        assert!(!p.connected());
        assert!(!p.button_down(PadBind::South));
        p.apply(PadEvent::Connected);
        assert!(p.connected());
        p.apply(PadEvent::Button(PadBind::South, true));
        assert!(p.button_down(PadBind::South));
    }

    #[test]
    fn a_gilrs_stick_is_flipped_to_screen_space_and_snapped_inside_the_dead_zone() {
        // gilrs: up is +1. The game: up is -y.
        let up = stick_from_gilrs(0.0, 1.0, 0.2);
        assert_eq!(up, Vec2::new(0.0, -1.0));
        let mut p = pad();
        p.apply(PadEvent::Stick(Stick::Left, up));
        assert!(p.move_vec(0.2).y < -0.9, "stick up must move up");
        // A resting stick that drifts inside the dead zone is not activity.
        let drift = stick_from_gilrs(0.15, -0.05, 0.2);
        assert_eq!(drift, Vec2::ZERO);
        p.begin_frame();
        p.apply(PadEvent::Stick(Stick::Left, drift));
        assert!(!p.active(), "drift must not claim the legends for the pad");
        // A broken axis never poisons the movement vector.
        assert_eq!(stick_from_gilrs(f32::NAN, 0.0, 0.2), Vec2::ZERO);
    }

    #[test]
    fn rumble_needs_a_focused_connected_pad_and_the_toggle() {
        let mut none = PadState::new();
        none.rumble(0.2);
        assert_eq!(none.rumble_left, 0.0, "no pad: nothing queued for later");

        let mut p = pad();
        p.rumble_if(false, 0.2);
        assert_eq!(p.rumble_left, 0.0, "the Options toggle is off");
        p.rumble_if(true, 0.2);
        assert!((p.rumble_left - 0.2).abs() < 1e-6);
        // The longer request wins, capped at the maximum.
        p.rumble(0.1);
        assert!((p.rumble_left - 0.2).abs() < 1e-6);
        p.rumble(60.0);
        assert!((p.rumble_left - MAX_RUMBLE_SECS).abs() < 1e-6);
        p.rumble(f32::NAN);
        assert!((p.rumble_left - MAX_RUMBLE_SECS).abs() < 1e-6);

        // Losing focus or the pad drops a pending request.
        p.set_focused(false);
        assert_eq!(p.rumble_left, 0.0);
        p.rumble(0.2);
        assert_eq!(p.rumble_left, 0.0, "unfocused: ignored");
        p.set_focused(true);
        p.rumble(0.2);
        p.apply(PadEvent::Disconnected);
        assert_eq!(p.rumble_left, 0.0);
    }

    #[test]
    fn pressed_is_one_frame_only() {
        let mut p = pad();
        p.apply(PadEvent::Button(PadBind::West, true));
        assert!(p.button_pressed(PadBind::West));
        p.begin_frame();
        assert!(!p.button_pressed(PadBind::West));
        assert!(p.button_down(PadBind::West));
    }
}
