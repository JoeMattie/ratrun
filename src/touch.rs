//! On-screen touch controls: the floating stick, the action buttons, taps,
//! swipes and pinch zoom.
//!
//! The web host (`src/web.rs`) is the **only** caller. For
//! `pointerType === "touch"` the page calls `WebGame::touch` instead of the
//! mouse path, with the browser's `pointerId` and a **fractional** cell
//! position (so a stick moves smoothly inside a cell). [`TouchControls`] folds
//! those contacts into the frame's output and [`App`] feeds that output into
//! the ordinary input paths, so nothing downstream of the input layer knows a
//! finger was involved:
//!
//! * the **stick** is an analog vector with the gamepad stick's dead zone and
//!   rescale ([`dead_zone`], the same rule as
//!   [`crate::gamepad::PadState::move_vec`]), summed with the keyboard exactly
//!   as the pad's stick is, so the simulation sees an ordinary analog
//!   `move_dir`;
//! * **dash** is a press edge for the ordinary dash and a press/hold/release
//!   triple for [`crate::game::ecs::resources::DashHold`], so holding it
//!   charges a Super Dash exactly as the pad's `RB` does;
//! * **actives** press [`Action::FireActives`];
//! * **pause** presses [`Action::Pause`] in play and becomes **Back** on every
//!   menu and overlay (the same slot, the same rect);
//! * **map** toggles the expanded map;
//! * a **tap** (a short contact that started on no control) is handed to the
//!   App as a left click at its cell, so cards, rows and the Library work
//!   through the existing pointer path rather than a second one;
//! * a **swipe** (a vertical drag of more than [`SWIPE_CELLS`]) becomes wheel
//!   steps, one per two cells, for the screens that scroll;
//! * a **pinch** (two contacts that both started clear of every control) steps
//!   the zoom ladder, one rung per [`PINCH_STEP`] change in finger distance,
//!   through the same entry point `Z`/`X` use.
//!
//! # Native and other hosts
//!
//! Nothing here is web-specific: it is a plain state machine over fractional
//! cell coordinates, compiled into every target (it is handy for the tests).
//! Native never feeds it, so every field sits at rest forever, [`App`] draws
//! nothing and a desktop build is byte-for-byte the frame it was before this
//! module existed. That is the contract's visibility rule — the rendered
//! controls appear after the first touch and hide again on any keyboard key or
//! real mouse move.
//!
//! [`App`]: crate::app::App
//! [`Action::FireActives`]: crate::controls::Action::FireActives
//! [`Action::Pause`]: crate::controls::Action::Pause

use std::collections::VecDeque;

use ratatui::layout::Rect;

use crate::math::Vec2;
use crate::render::minimap::{self, MinimapSize};
use crate::render::touch_art::{self, ButtonKind, StickView};

/// One touch contact, as the page reports it (`pointerdown`, `pointermove`,
/// `pointerup`, `pointercancel`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TouchPhase {
    Start,
    Move,
    End,
    Cancel,
}

/// How many simultaneous contacts are tracked. A browser rarely reports more
/// than five; a sixth is ignored rather than allowed to grow the state.
pub const MAX_CONTACTS: usize = 8;
/// Taps delivered to the pointer path in one frame.
pub const MAX_TAPS: usize = 4;
/// A contact that moved no further than this from where it started is a tap.
pub const TAP_SLOP_CELLS: f32 = 1.0;
/// A vertical drag steps the wheel once per this many cells.
pub const SWIPE_CELLS: f32 = 2.0;
/// A pinch changes the zoom one rung per this factor of finger distance.
pub const PINCH_STEP: f32 = 1.25;
/// The floating stick's throw, horizontally: about five cells.
pub const STICK_RADIUS_COLS: f32 = 5.0;
/// The floating stick's throw, vertically: three cells, because a cell is about
/// twice as tall as it is wide, so a circle in world space is this ellipse in
/// the grid.
pub const STICK_RADIUS_ROWS: f32 = 3.0;
/// The share of the field's width, from its left edge, where a stick may start.
pub const STICK_ZONE_SHARE: f32 = 0.45;
/// Inset from the HUD rails, the minimap and the field's edges.
const INSET: u16 = 1;
/// How many contacts may queue between two frames (a stalled page must not grow
/// the queue without bound).
const QUEUE_CAP: usize = 64;

const EMPTY: Rect = Rect::new(0, 0, 0, 0);

/// What a contact can own, and therefore what its lift means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Stick,
    Dash,
    Actives,
    Pause,
    Back,
    Map,
}

impl Slot {
    /// The art's button for this slot. The stick is not a button (it has its
    /// own [`touch_art::draw_stick`]).
    pub fn kind(self) -> Option<ButtonKind> {
        Some(match self {
            Slot::Stick => return None,
            Slot::Dash => ButtonKind::Dash,
            Slot::Actives => ButtonKind::Actives,
            Slot::Pause => ButtonKind::Pause,
            Slot::Back => ButtonKind::Back,
            Slot::Map => ButtonKind::Map,
        })
    }
}

/// Which kind of screen the controls belong to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    /// A live run: the stick, Dash, Actives (when owned), Pause and Map.
    Play,
    /// A menu or an overlay: one **Back** button, in the Pause slot.
    #[default]
    Menu,
}

/// The on-screen controls' rectangles in cells — the single source of truth for
/// them, exactly like each screen's own `layout()` is for its pointer targets.
/// [`crate::app::App`] both draws from this and routes contacts through it, so
/// the art and the hit test can never disagree.
#[derive(Clone, Copy, Debug)]
pub struct TouchLayout {
    pub surface: Surface,
    /// Where a stick may start (empty in a menu).
    pub stick_zone: Rect,
    /// The compact minimap panel this layout kept clear of, as
    /// [`crate::render::minimap::compact_rect_for_size`] draws it for this
    /// frame's field (empty in a menu, and when the radar is
    /// [`MinimapSize::Hidden`]).
    pub minimap: Rect,
    /// [`Slot::Dash`], [`Slot::Actives`], the pause/back slot and [`Slot::Map`].
    /// An absent control is an empty rect.
    buttons: [(Slot, Rect); 4],
}

impl Default for TouchLayout {
    fn default() -> Self {
        TouchLayout {
            surface: Surface::Menu,
            stick_zone: EMPTY,
            minimap: EMPTY,
            buttons: [
                (Slot::Dash, EMPTY),
                (Slot::Actives, EMPTY),
                (Slot::Back, EMPTY),
                (Slot::Map, EMPTY),
            ],
        }
    }
}

impl TouchLayout {
    /// The controls for a live run: a floating stick anywhere in the left 45 %
    /// of the field, and Dash with Actives beside it in the bottom-right
    /// region — directly above the compact minimap and right-aligned with it,
    /// one cell clear (see the `action_row` helper) — Pause at the field's top
    /// left and Map at its top right.
    ///
    /// `area` is the whole frame and `field` the pixel-viewport rect
    /// (`App::last_game`) — the field's top row already sits below the top HUD
    /// rail, and the bottom HUD row below it, so a control clamped into the
    /// field can cover neither rail. `minimap_size` is the player's saved
    /// compact size: the radar's rect comes from
    /// [`crate::render::minimap::compact_rect_for_size`], the same function the
    /// minimap draws from, so the buttons and the radar cannot drift — and when
    /// the radar would leave no room above it, [`play_minimap_size`] asks for a
    /// [`MinimapSize::Small`] radar instead.
    pub fn play(area: Rect, field: Rect, actives: bool, minimap_size: MinimapSize) -> Self {
        let field = usable_field(area, field);
        let minimap = minimap::compact_rect_for_size(
            field,
            play_minimap_size(field, actives, minimap_size),
        );
        let (pw, ph) = touch_art::button_size(ButtonKind::Pause);
        let pause = pause_slot(field, pw, ph);
        let (mw, mh) = touch_art::button_size(ButtonKind::Map);
        let mut map = clamp_into(
            field,
            field.right().saturating_sub(mw.saturating_add(INSET)),
            field.y.saturating_add(INSET),
            mw,
            mh,
        );
        if !minimap.is_empty() && map.intersects(minimap) {
            // A radar tall enough to reach the field's top row owns it: the
            // Map sits beside it at the same height, else under it.
            let beside = clamp_into(
                field,
                minimap.x.saturating_sub(mw.saturating_add(INSET)),
                field.y.saturating_add(INSET),
                mw,
                mh,
            );
            map = if clear_of(beside, &[minimap, pause]) {
                beside
            } else {
                clamp_into(
                    field,
                    map.x,
                    minimap.bottom().saturating_add(INSET),
                    mw,
                    mh,
                )
            };
        }
        let row = action_row(field, minimap, actives, &[pause, map]);
        let (dw, dh) = touch_art::button_size(ButtonKind::Dash);
        let dash = clamp_into(field, row.right().saturating_sub(dw), row.y, dw, dh);
        let actives_rect = if actives {
            let (aw, ah) = touch_art::button_size(ButtonKind::Actives);
            clamp_into(field, row.x, row.y, aw, ah)
        } else {
            EMPTY
        };
        let zone_w = (f32::from(field.width) * STICK_ZONE_SHARE) as u16;
        TouchLayout {
            surface: Surface::Play,
            stick_zone: Rect::new(field.x, field.y, zone_w.min(field.width), field.height),
            minimap,
            buttons: [
                (Slot::Dash, dash),
                (Slot::Actives, actives_rect),
                (Slot::Pause, pause),
                (Slot::Map, map),
            ],
        }
    }

    /// The controls for a menu or an overlay: one **Back** button, in the
    /// pause slot (the field's top-left when a world is drawn behind the plate,
    /// else the frame's).
    pub fn menu(area: Rect, field: Rect) -> Self {
        let anchor = usable_field(area, field);
        let (bw, bh) = touch_art::button_size(ButtonKind::Back);
        let back = pause_slot(anchor, bw, bh);
        TouchLayout {
            surface: Surface::Menu,
            stick_zone: EMPTY,
            minimap: EMPTY,
            buttons: [
                (Slot::Dash, EMPTY),
                (Slot::Actives, EMPTY),
                (Slot::Back, back),
                (Slot::Map, EMPTY),
            ],
        }
    }

    /// Every control this layout shows, in drawing order. Empty rects are
    /// absent controls (Actives without the weapon, Map and Dash in a menu).
    pub fn slots(&self) -> impl Iterator<Item = (Slot, Rect)> + '_ {
        self.buttons.iter().copied().filter(|(_, rect)| !rect.is_empty())
    }

    /// The rectangle of one slot (`EMPTY` when this layout does not show it).
    pub fn button(&self, slot: Slot) -> Rect {
        self.buttons
            .iter()
            .find(|(s, _)| *s == slot)
            .map_or(EMPTY, |(_, rect)| *rect)
    }

    /// The control that owns a press at `cell`: a button first (they sit over
    /// the field), then the stick zone.
    pub fn control_at(&self, cell: (u16, u16)) -> Option<Slot> {
        let pos = cell.into();
        if let Some((slot, _)) = self
            .buttons
            .iter()
            .find(|(_, rect)| !rect.is_empty() && rect.contains(pos))
        {
            return Some(*slot);
        }
        if self.surface == Surface::Play
            && !self.stick_zone.is_empty()
            && self.stick_zone.contains(pos)
        {
            return Some(Slot::Stick);
        }
        None
    }
}

/// The area the controls are laid out in: the play field when there is one,
/// else the whole frame.
fn usable_field(area: Rect, field: Rect) -> Rect {
    if field.is_empty() || area.is_empty() {
        area
    } else {
        field
    }
}

/// The top-left slot: Pause in play, Back in a menu (the same rect in both, so
/// the button does not move under the finger when a plate opens).
fn pause_slot(field: Rect, w: u16, h: u16) -> Rect {
    clamp_into(
        field,
        field.x.saturating_add(INSET),
        field.y.saturating_add(INSET),
        w,
        h,
    )
}

/// Fit a `w × h` rect at `x`,`y` inside `field` (a control never hangs off the
/// field, and a field too small for it clips it).
fn clamp_into(field: Rect, x: u16, y: u16, w: u16, h: u16) -> Rect {
    if field.is_empty() {
        return EMPTY;
    }
    let w = w.min(field.width);
    let h = h.min(field.height);
    if w == 0 || h == 0 {
        return EMPTY;
    }
    Rect::new(
        x.clamp(field.x, field.right().saturating_sub(w)),
        y.clamp(field.y, field.bottom().saturating_sub(h)),
        w,
        h,
    )
}

/// The compact minimap size a touch play screen draws at: the player's saved
/// preference, unless the radar would leave no room for the Dash row directly
/// above it — then [`MinimapSize::Small`], which is smaller **on this screen
/// while the controls are up only**. Nothing is saved: the preference comes
/// straight back when the controls hide.
///
/// The radar's own sizes are relative to the field (a `Normal` radar needs 18
/// rows and leaves 15 behind), so the rung above fits at every size that shows
/// one today; this is the guard that keeps the controls clear if that table
/// ever grows.
pub fn play_minimap_size(field: Rect, actives: bool, preferred: MinimapSize) -> MinimapSize {
    let rect = minimap::compact_rect_for_size(field, preferred);
    if rect.is_empty() || actions_fit_above(field, rect, actives) {
        return preferred;
    }
    // No radar at all is worse than a smaller one. A preference too big for
    // this field already resolved through `effective_compact_size`.
    if minimap::compact_rect_for_size(field, MinimapSize::Small).is_empty() {
        preferred
    } else {
        MinimapSize::Small
    }
}

/// Does the Dash row fit directly above `minimap`, right-aligned with it and one
/// cell clear, inside `field`?
fn actions_fit_above(field: Rect, minimap: Rect, actives: bool) -> bool {
    let (w, h) = action_row_size(actives);
    contains_rect(field, minimap)
        && w <= minimap.width
        && minimap.y >= field.y.saturating_add(h.saturating_add(INSET))
}

/// The `(width, height)` of the Dash-plus-Actives row: the contract's thumb
/// sizes (`7 × 3` for Dash, `5 × 3` for Actives), one cell between them. The
/// layout never shrinks a button below these (only a field too small for the
/// thumb clips it).
fn action_row_size(actives: bool) -> (u16, u16) {
    let (dw, dh) = touch_art::button_size(ButtonKind::Dash);
    let (aw, ah) = touch_art::button_size(ButtonKind::Actives);
    if actives {
        (dw.saturating_add(INSET).saturating_add(aw), dh.max(ah))
    } else {
        (dw, dh)
    }
}

/// The Dash-plus-Actives row: directly above `minimap` with a one-cell gap and
/// right-aligned with it; else to its left, bottom-aligned; else the field's
/// own bottom-right corner. `avoid` is every other control the row must not
/// cover — the corner is taken only when no clear rung is left.
fn action_row(field: Rect, minimap: Rect, actives: bool, avoid: &[Rect]) -> Rect {
    let (w, h) = action_row_size(actives);
    let above = if minimap.width >= w && contains_rect(field, minimap) {
        minimap
            .y
            .checked_sub(h.saturating_add(INSET))
            .filter(|&y| y >= field.y)
            .map_or(EMPTY, |y| Rect::new(minimap.right() - w, y, w, h))
    } else {
        EMPTY
    };
    let left = if minimap.height >= h && contains_rect(field, minimap) {
        minimap
            .x
            .checked_sub(w.saturating_add(INSET))
            .filter(|&x| x >= field.x)
            .map_or(EMPTY, |x| Rect::new(x, minimap.bottom() - h, w, h))
    } else {
        EMPTY
    };
    let corner = clamp_into(
        field,
        field.right().saturating_sub(w.saturating_add(INSET)),
        field.bottom().saturating_sub(h.saturating_add(INSET)),
        w,
        h,
    );
    let rungs = [above, left, corner];
    rungs
        .into_iter()
        .find(|rect| !rect.is_empty() && !rect.intersects(minimap) && clear_of(*rect, avoid))
        .or_else(|| rungs.into_iter().find(|rect| !rect.is_empty()))
        .unwrap_or(EMPTY)
}

/// Is `inner` wholly inside `outer`?
fn contains_rect(outer: Rect, inner: Rect) -> bool {
    !inner.is_empty()
        && inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

/// Does `rect` stay clear of every non-empty rect in `others`?
fn clear_of(rect: Rect, others: &[Rect]) -> bool {
    !others.iter().any(|other| !other.is_empty() && rect.intersects(*other))
}

/// The gamepad stick's dead zone, rescaled so a fully deflected stick is 1:
/// the same rule [`crate::gamepad::PadState::move_vec`] applies, so a touch
/// stick and a pad stick feel identical and the simulation cannot tell them
/// apart.
pub fn dead_zone(v: Vec2, deadzone: f32) -> Vec2 {
    let dead = deadzone.clamp(0.0, 0.95);
    let len = v.len();
    if len <= dead || !len.is_finite() {
        return Vec2::ZERO;
    }
    let scaled = ((len - dead) / (1.0 - dead)).clamp(0.0, 1.0);
    v * (scaled / len)
}

/// One frame of touch output. Every edge lives **exactly one frame** (the App
/// calls [`TouchControls::frame`] once per update).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchOutput {
    /// Analog movement (0..1), dead-zoned like the pad's left stick.
    pub stick: Vec2,
    /// The Dash button's press edge: the ordinary dash, or the start of a Super
    /// Dash charge.
    pub dash_press: bool,
    /// The Dash button went down this frame.
    pub dash_down: bool,
    /// A contact still owns the Dash button.
    pub dash_held: bool,
    /// The Dash button came up this frame (it fires a charged Super Dash).
    pub dash_up: bool,
    /// An actives press edge.
    pub active_press: bool,
    /// The Pause button (a live run only).
    pub pause_press: bool,
    /// The Back button (every menu and overlay).
    pub back_press: bool,
    /// The Map button.
    pub map_press: bool,
    /// Pinch zoom steps: positive pulls the view in (the `Z` key's direction).
    pub zoom_steps: i32,
    /// Swipe scroll steps: positive walks a list forward (the wheel's
    /// `ScrollDown`), negative back.
    pub scroll_steps: i32,
    /// Where the swipe's wheel steps apply (the finger's current cell).
    pub scroll_at: (u16, u16),
    /// Taps to hand to the pointer path, in cell coordinates.
    pub taps: [Option<(u16, u16)>; MAX_TAPS],
    /// At least one contact is on the glass.
    pub touching: bool,
}

/// One tracked contact. `owner` is fixed at [`TouchPhase::Start`] and held to
/// the lift: a contact that started on a button owns that button even if the
/// finger slides off it (`None` is a free finger: a tap, a swipe or half a
/// pinch).
#[derive(Clone, Copy, Debug)]
struct Contact {
    id: i32,
    start: (f32, f32),
    at: (f32, f32),
    owner: Option<Slot>,
    /// Wheel steps already sent by this drag (a swipe never scrolls back).
    scrolled: i32,
}

impl Contact {
    fn cell(&self) -> (u16, u16) {
        (self.at.0.max(0.0) as u16, self.at.1.max(0.0) as u16)
    }
}

/// The live touch state: queued host events, the contacts being tracked, and
/// the frame's derived output.
#[derive(Default)]
pub struct TouchControls {
    queue: VecDeque<(TouchPhase, i32, f32, f32)>,
    contacts: [Option<Contact>; MAX_CONTACTS],
    /// Last two-finger distance, and the ratio accumulated since the last step.
    pinch_from: Option<f32>,
    pinch_ratio: f32,
}

impl TouchControls {
    pub fn new() -> Self {
        Self::default()
    }

    /// One contact from the host. Queued; [`frame`](Self::frame) folds it into
    /// the state (the layout — and so what the contact owns — is this frame's).
    pub fn push(&mut self, phase: TouchPhase, id: i32, col: f32, row: f32) {
        if !col.is_finite() || !row.is_finite() {
            return;
        }
        if self.queue.len() < QUEUE_CAP {
            self.queue.push_back((phase, id, col.max(0.0), row.max(0.0)));
        }
    }

    /// Forget every contact and every queued event (the host lost the page, the
    /// canvas resized): nothing is left to fire a dash or hold a button.
    pub fn reset(&mut self) {
        self.queue.clear();
        self.contacts = [None; MAX_CONTACTS];
        self.pinch_from = None;
        self.pinch_ratio = 1.0;
    }

    /// Is any contact on the glass right now?
    pub fn touching(&self) -> bool {
        self.contacts.iter().any(Option::is_some)
    }

    /// Is a contact holding `slot` right now? (Drawing only: the art's pressed
    /// state.)
    pub fn holds(&self, slot: Slot) -> bool {
        self.contacts
            .iter()
            .flatten()
            .any(|contact| contact.owner == Some(slot))
    }

    /// The floating stick's base, knob and throw while a contact holds it, in
    /// fractional cell coordinates. The knob stops at the throw's rim.
    pub fn stick_view(&self) -> Option<StickView> {
        let contact = self
            .contacts
            .iter()
            .flatten()
            .find(|contact| contact.owner == Some(Slot::Stick))?;
        let v = stick_offset(contact.start, contact.at);
        Some(StickView {
            base: contact.start,
            knob: (
                contact.start.0 + v.x * STICK_RADIUS_COLS,
                contact.start.1 + v.y * STICK_RADIUS_ROWS,
            ),
            radius_cells: STICK_RADIUS_COLS,
        })
    }

    /// Fold this frame's queued contacts into the state and return the frame's
    /// output. `deadzone` is the player's stick dead zone
    /// (`InputState::controls.deadzone`), the same one the pad's stick uses.
    pub fn frame(&mut self, layout: &TouchLayout, deadzone: f32) -> TouchOutput {
        // Native, and a desktop browser, never push a contact: the whole layer
        // is one branch per frame and every output stays at rest.
        if self.queue.is_empty() && !self.touching() {
            self.pinch_from = None;
            self.pinch_ratio = 1.0;
            return TouchOutput::default();
        }
        let mut out = TouchOutput::default();
        while let Some((phase, id, col, row)) = self.queue.pop_front() {
            self.apply(phase, id, col, row, layout, &mut out);
        }
        if let Some(contact) = self.stick_contact() {
            out.stick = dead_zone(stick_offset(contact.start, contact.at), deadzone);
        }
        out.dash_held = self.holds(Slot::Dash);
        self.pinch_steps(&mut out);
        self.swipe_steps(layout, &mut out);
        out.touching = self.touching();
        out
    }

    // ---- contacts -------------------------------------------------------

    fn apply(
        &mut self,
        phase: TouchPhase,
        id: i32,
        col: f32,
        row: f32,
        layout: &TouchLayout,
        out: &mut TouchOutput,
    ) {
        let cell = (col.max(0.0) as u16, row.max(0.0) as u16);
        match phase {
            TouchPhase::Start => {
                let Some(slot) = self.free_slot() else { return };
                let owner = layout.control_at(cell);
                self.contacts[slot] = Some(Contact {
                    id,
                    start: (col, row),
                    at: (col, row),
                    owner,
                    scrolled: 0,
                });
                // A button's press edge is the contact's start, never the
                // slide onto it: a finger that arrives on a menu card must not
                // fire a dash on its way to a scroll.
                match owner {
                    Some(Slot::Dash) => {
                        out.dash_press = true;
                        out.dash_down = true;
                    }
                    Some(Slot::Actives) => out.active_press = true,
                    Some(Slot::Pause) => out.pause_press = true,
                    Some(Slot::Back) => out.back_press = true,
                    Some(Slot::Map) => out.map_press = true,
                    Some(Slot::Stick) | None => {}
                }
            }
            TouchPhase::Move => {
                if let Some(contact) = self.contact_mut(id) {
                    contact.at = (col, row);
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                let Some(mut contact) = self.contact_mut(id).copied() else {
                    return;
                };
                // The lift's own position counts: a browser that reports no
                // final move still ends where the finger came up.
                contact.at = (col, row);
                match contact.owner {
                    // A charged Super Dash fires on the release edge. A
                    // `Cancel` (the page took the pointer back) sends neither
                    // edge: the sim drops the charge rather than firing it,
                    // exactly as it does when the terminal loses focus.
                    Some(Slot::Dash) if phase == TouchPhase::End => out.dash_up = true,
                    // A tap: a short contact that started on no control. Every
                    // menu and overlay gets it as a left click at its cell, so
                    // cards, rows and the Library work through the ordinary
                    // pointer path.
                    None if phase == TouchPhase::End && layout.surface == Surface::Menu => {
                        let moved = Vec2::new(contact.at.0, contact.at.1)
                            .dist(Vec2::new(contact.start.0, contact.start.1));
                        if moved <= TAP_SLOP_CELLS {
                            if let Some(slot) =
                                out.taps.iter_mut().find(|tap| tap.is_none())
                            {
                                *slot = Some(cell);
                            }
                        }
                    }
                    _ => {}
                }
                self.drop_id(id);
            }
        }
    }

    fn free_slot(&self) -> Option<usize> {
        self.contacts.iter().position(Option::is_none)
    }

    fn contact_mut(&mut self, id: i32) -> Option<&mut Contact> {
        self.contacts
            .iter_mut()
            .flatten()
            .find(|contact| contact.id == id)
    }

    fn stick_contact(&self) -> Option<&Contact> {
        self.contacts
            .iter()
            .flatten()
            .find(|contact| contact.owner == Some(Slot::Stick))
    }

    /// Free the slot of one contact (by id), so a later event for the same id
    /// starts a fresh contact.
    fn drop_id(&mut self, id: i32) {
        for slot in self.contacts.iter_mut() {
            if slot.is_some_and(|contact| contact.id == id) {
                *slot = None;
            }
        }
    }

    // ---- derived gestures -----------------------------------------------

    /// Two fingers that both started clear of every control zoom the view: one
    /// ladder rung per [`PINCH_STEP`] change in their distance, in either
    /// direction.
    fn pinch_steps(&mut self, out: &mut TouchOutput) {
        let mut free = [(0.0f32, 0.0f32); 3];
        let mut count = 0;
        for contact in self.contacts.iter().flatten() {
            if contact.owner.is_none() && count < free.len() {
                free[count] = contact.at;
                count += 1;
            }
        }
        if count != 2 {
            self.pinch_from = None;
            self.pinch_ratio = 1.0;
            return;
        }
        let from = Vec2::new(free[0].0, free[0].1);
        let to = Vec2::new(free[1].0, free[1].1);
        let distance = from.dist(to);
        let Some(previous) = self.pinch_from.replace(distance) else {
            self.pinch_ratio = 1.0;
            return;
        };
        if previous <= 0.5 || distance <= 0.5 {
            self.pinch_ratio = 1.0;
            return;
        }
        self.pinch_ratio *= distance / previous;
        // Fingers apart pulls the view in (a closer rung), together widens it:
        // the same ladder `Z`/`X` steps.
        while self.pinch_ratio >= PINCH_STEP {
            out.zoom_steps += 1;
            self.pinch_ratio /= PINCH_STEP;
        }
        while self.pinch_ratio <= 1.0 / PINCH_STEP {
            out.zoom_steps -= 1;
            self.pinch_ratio *= PINCH_STEP;
        }
    }

    /// One free finger dragged vertically on a scrollable screen walks the list:
    /// a wheel step per [`SWIPE_CELLS`] of travel, forward for an upward drag.
    /// A play swipe never scrolls (the wheel over the field is the zoom, and a
    /// finger must not train it), and two fingers are a pinch, not a scroll.
    fn swipe_steps(&mut self, layout: &TouchLayout, out: &mut TouchOutput) {
        if layout.surface != Surface::Menu {
            return;
        }
        let free = self
            .contacts
            .iter()
            .flatten()
            .filter(|contact| contact.owner.is_none())
            .count();
        if free != 1 {
            return;
        }
        let Some(contact) = self
            .contacts
            .iter_mut()
            .flatten()
            .find(|contact| contact.owner.is_none())
        else {
            return;
        };
        let dy = contact.at.1 - contact.start.1;
        let want = (dy.abs() / SWIPE_CELLS) as i32;
        if want > contact.scrolled {
            let steps = want - contact.scrolled;
            contact.scrolled = want;
            // Dragging up reveals what is below (the wheel's `ScrollDown`);
            // dragging down walks back.
            out.scroll_steps += if dy < 0.0 { steps } else { -steps };
            out.scroll_at = contact.cell();
        } else if want < contact.scrolled {
            // The finger changed its mind: re-anchor without scrolling back, so
            // a wobble never jitters the list.
            contact.scrolled = want;
        }
    }
}

/// The stick's throw from its base to the finger, clamped to the rim: `x` and
/// `y` are each a fraction of the ellipse's own radius.
fn stick_offset(base: (f32, f32), at: (f32, f32)) -> Vec2 {
    Vec2::new(
        (at.0 - base.0) / STICK_RADIUS_COLS,
        (at.1 - base.1) / STICK_RADIUS_ROWS,
    )
    .clamp_len(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 80 × 24 frame with the play field the HUD leaves: rows 3..23.
    fn field() -> Rect {
        Rect::new(0, 3, 80, 19)
    }

    fn layout() -> TouchLayout {
        TouchLayout::play(Rect::new(0, 0, 80, 24), field(), true, MinimapSize::Normal)
    }

    #[test]
    fn the_controls_sit_inside_the_field_and_never_overlap() {
        let l = layout();
        let dash = l.button(Slot::Dash);
        let actives = l.button(Slot::Actives);
        let pause = l.button(Slot::Pause);
        let map = l.button(Slot::Map);
        for rect in [dash, actives, pause, map] {
            assert!(!rect.is_empty(), "every control is placed");
            assert!(rect.x >= field().x && rect.y >= field().y, "{rect:?} starts inside");
            assert!(rect.right() <= field().right() && rect.bottom() <= field().bottom());
            assert!(
                !rect.intersects(l.minimap),
                "{rect:?} covers the radar {:?}",
                l.minimap
            );
        }
        assert!(!dash.intersects(actives), "Actives sits left of Dash");
        assert!(!pause.intersects(map), "Pause is left, Map is right");
        assert_eq!(l.surface, Surface::Play);
        assert!(l.stick_zone.width < field().width / 2, "the stick owns the left side");
    }

    /// The radar's rect is the art's own (`compact_rect_for_size`), and the Dash
    /// row sits directly above it, right-aligned with it and one cell clear.
    #[test]
    fn dash_and_actives_sit_above_the_radar_right_aligned_with_it() {
        let l = layout();
        let radar = l.minimap;
        assert_eq!(radar, minimap::compact_rect_for_size(field(), MinimapSize::Normal));
        let dash = l.button(Slot::Dash);
        let actives = l.button(Slot::Actives);
        assert_eq!(dash.bottom() + INSET, radar.y, "one cell between Dash and the radar");
        assert_eq!(actives.bottom(), dash.bottom(), "Actives sits beside Dash");
        assert_eq!(actives.right() + INSET, dash.x, "one cell between Actives and Dash");
        assert_eq!(dash.right(), radar.right(), "the row is right-aligned with the radar");
        assert_eq!(actives.x, radar.right() - 13, "the row is Dash and Actives side by side");
    }

    /// The Dash keeps the contract's `7 × 3` thumb and Actives its `5 × 3`, at
    /// every integration grid.
    #[test]
    fn the_buttons_keep_the_contract_thumb_size() {
        for (w, h) in [(120u16, 40u16), (127, 30), (123, 35), (100, 24)] {
            let area = Rect::new(0, 0, w, h);
            let field = Rect::new(0, 3, w, h.saturating_sub(4));
            let l = TouchLayout::play(area, field, true, MinimapSize::Normal);
            assert_eq!((l.button(Slot::Dash).width, l.button(Slot::Dash).height), (7, 3));
            assert_eq!(
                (l.button(Slot::Actives).width, l.button(Slot::Actives).height),
                (5, 3)
            );
        }
    }

    /// The rungs of [`action_row`]: above the radar, else to its left, else the
    /// field's own corner — each clear of the radar and of the other controls.
    #[test]
    fn the_dash_row_falls_back_left_of_the_radar_and_then_to_the_corner() {
        let field = Rect::new(0, 3, 60, 20);
        let avoid = [Rect::new(1, 4, 5, 3)];
        // A radar with no room above it: the row goes to its left, bottom-aligned.
        let radar = Rect::new(40, 5, 18, 16);
        let row = action_row(field, radar, true, &avoid);
        assert_eq!(row, Rect::new(40 - 1 - 13, radar.bottom() - 3, 13, 3));
        assert!(!row.intersects(radar) && clear_of(row, &avoid));
        // Still no room to the left: the field's bottom-right corner.
        let wide = Rect::new(10, 5, 50, 16);
        let row = action_row(field, wide, true, &avoid);
        assert_eq!(row, Rect::new(60 - 1 - 13, 23 - 3 - 1, 13, 3));
        // No radar at all (Hidden, or a field with no room for one): the corner.
        let row = action_row(field, EMPTY, true, &avoid);
        assert_eq!(row, Rect::new(60 - 1 - 13, 23 - 3 - 1, 13, 3));
    }

    /// Whichever size [`play_minimap_size`] hands back, the Dash row has room
    /// above the radar it draws — the shrink to `Small` is enough, and it is
    /// only ever asked for when the saved `Normal` leaves none.
    #[test]
    fn the_radar_only_shrinks_to_leave_room_above_it() {
        for w in [48u16, 57, 72, 80, 100, 120, 127, 160, 200] {
            for h in [12u16, 14, 18, 20, 24, 30, 40, 60] {
                for actives in [false, true] {
                    let field = Rect::new(0, 3, w, h);
                    for preferred in [MinimapSize::Normal, MinimapSize::Small] {
                        let size = play_minimap_size(field, actives, preferred);
                        let radar = minimap::compact_rect_for_size(field, size);
                        if radar.is_empty() {
                            assert_eq!(size, preferred, "{w}x{h}: no radar, no reason to step");
                            continue;
                        }
                        assert!(
                            actions_fit_above(field, radar, actives),
                            "{w}x{h} actives={actives} preferred={preferred:?} left no room above {radar:?}"
                        );
                        if actions_fit_above(
                            field,
                            minimap::compact_rect_for_size(field, preferred),
                            actives,
                        ) {
                            assert_eq!(size, preferred, "{w}x{h}: the saved size already fits");
                        } else {
                            assert_eq!(size, MinimapSize::Small);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_menu_layout_has_only_back_in_the_pause_slot() {
        let play = layout();
        let menu = TouchLayout::menu(Rect::new(0, 0, 80, 24), field());
        assert_eq!(menu.surface, Surface::Menu);
        let back = menu.button(Slot::Back);
        let pause = play.button(Slot::Pause);
        assert_eq!(
            (back.x, back.y),
            (pause.x, pause.y),
            "Back sits in Pause's slot (same corner, its own size)"
        );
        assert!(menu.button(Slot::Dash).is_empty());
        assert!(menu.stick_zone.is_empty());
        assert_eq!(menu.slots().count(), 1);
    }

    #[test]
    fn the_stick_reaches_full_throttle_at_the_throw_and_stops_at_rest() {
        let mut l = layout();
        l.stick_zone = Rect::new(0, 3, 40, 19);
        let mut t = TouchControls::new();
        let base = (10.0, 10.0);
        t.push(TouchPhase::Start, 1, base.0, base.1);
        let out = t.frame(&l, 0.0);
        assert_eq!(out.stick, Vec2::ZERO, "a resting finger does not move the rat");
        t.push(TouchPhase::Move, 1, base.0 + 5.0, base.1);
        let out = t.frame(&l, 0.0);
        assert!((out.stick.x - 1.0).abs() < 1e-4, "five cells right is full throttle");
        assert_eq!(out.stick.y, 0.0);
        t.push(TouchPhase::Move, 1, base.0 + 20.0, base.1);
        let out = t.frame(&l, 0.0);
        assert_eq!(out.stick.len(), 1.0, "the throw is clamped");
        t.push(TouchPhase::End, 1, base.0 + 20.0, base.1);
        let out = t.frame(&l, 0.0);
        assert_eq!(out.stick, Vec2::ZERO, "lifting stops the rat");
    }

    #[test]
    fn the_dead_zone_matches_the_pads_rescale() {
        // Inside the zone is zero; at full deflection it is one; halfway
        // through the live part is half.
        assert_eq!(dead_zone(Vec2::new(0.2, 0.0), 0.25), Vec2::ZERO);
        assert_eq!(dead_zone(Vec2::new(1.0, 0.0), 0.25).x, 1.0);
        assert!((dead_zone(Vec2::new(0.625, 0.0), 0.25).x - 0.5).abs() < 1e-5);
        assert_eq!(dead_zone(Vec2::ZERO, 0.25), Vec2::ZERO);
    }
}
