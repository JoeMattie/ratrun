//! On-screen touch control art (mobile): the floating stick and the action
//! buttons, drawn in **cells** over the game picture and under the cursor.
//!
//! STUB: replaced by mobile-art
//!
//! The API is the v0.5 contract's (section 3) and [`crate::touch`] places and
//! drives it: `touch.rs` owns every rectangle and every state (pressed, ready,
//! the Super Dash's charge), this module only paints. Everything is drawn
//! translucent — a shaded glyph with a dim foreground at most — so the floor,
//! the rat and a bright effect stay readable underneath, and nothing here ever
//! clears a cell's background.
//!
//! [`crate::render::palette::to_term`] folds the colours to the terminal's depth,
//! like the HUD's art does.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::render::palette::to_term;
use crate::term::ColorDepth;

/// The floating stick: `base` is where the finger landed, `knob` where it is
/// now, both in fractional cells; `radius_cells` is the throw's horizontal
/// radius (the vertical one is about half, because a cell is about twice as
/// tall as it is wide).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickView {
    pub base: (f32, f32),
    pub knob: (f32, f32),
    pub radius_cells: f32,
}

/// Which button is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Dash,
    Actives,
    Pause,
    Back,
    Map,
}

/// One button's placement and state. `pressed` is a finger on it, `ready` the
/// action's availability (the dash off cooldown, an active weapon charged) and
/// `charge` the Super Dash's hold meter (0..1).
#[derive(Clone, Copy, Debug)]
pub struct ButtonView {
    pub kind: ButtonKind,
    pub rect: Rect,
    pub pressed: bool,
    pub ready: bool,
    pub charge: f32,
}

/// The button's preferred size in cells (`w`, `h`).
pub fn button_size(kind: ButtonKind) -> (u16, u16) {
    match kind {
        ButtonKind::Dash => (5, 2),
        ButtonKind::Actives => (5, 2),
        ButtonKind::Pause => (3, 1),
        ButtonKind::Map => (3, 1),
        ButtonKind::Back => (4, 1),
    }
}

/// The ring's ink: a warm grey, dim enough to read over a lit floor.
const RING: (u8, u8, u8) = (150, 146, 138);
/// The knob's ink, brighter than the ring but still short of white.
const KNOB: (u8, u8, u8) = (226, 214, 196);
/// A button's outline.
const EDGE: (u8, u8, u8) = (128, 124, 118);
/// A button's label.
const LABEL: (u8, u8, u8) = (214, 206, 192);
/// A button that cannot be used yet.
const DIM: (u8, u8, u8) = (92, 90, 86);
/// A pressed button's highlight.
const HOT: (u8, u8, u8) = (242, 196, 108);

fn ink(rgb: (u8, u8, u8), depth: ColorDepth) -> Color {
    to_term(rgb, depth)
}

/// Paint one cell's foreground glyph without touching its background, so the
/// art blends over whatever the picture left there.
fn stamp(buf: &mut Buffer, x: u16, y: u16, glyph: &str, fg: Color) {
    if !buf.area.contains((x, y).into()) {
        return;
    }
    let cell = &mut buf[(x, y)];
    cell.set_symbol(glyph);
    cell.set_fg(fg);
}

/// The floating stick: a dotted throw ring with a knob at the finger.
pub fn draw_stick(buf: &mut Buffer, v: &StickView, depth: ColorDepth) {
    let ring = ink(RING, depth);
    let knob_ink = ink(KNOB, depth);
    // The throw is an ellipse: `radius_cells` across, half that down, because
    // the grid's cells are twice as tall as they are wide.
    let rx = v.radius_cells.max(1.0);
    let ry = (v.radius_cells * 0.5).max(1.0);
    let base_x = v.base.0;
    let base_y = v.base.1;
    // Eight points around the rim, enough to read as a pad in a few cells.
    for i in 0..8 {
        let angle = std::f32::consts::TAU * i as f32 / 8.0;
        let x = base_x + rx * angle.cos();
        let y = base_y + ry * angle.sin();
        if x >= 0.0 && y >= 0.0 {
            stamp(buf, x as u16, y as u16, "·", ring);
        }
    }
    if v.knob.0 >= 0.0 && v.knob.1 >= 0.0 {
        stamp(buf, v.knob.0 as u16, v.knob.1 as u16, "●", knob_ink);
    }
}

/// One action button: a bracketed label, brightened by a finger, dimmed when it
/// cannot be used, with the Super Dash's charge as a bar along its bottom row.
pub fn draw_button(buf: &mut Buffer, v: &ButtonView, depth: ColorDepth) {
    let rect = v.rect;
    if rect.is_empty() {
        return;
    }
    // The glyph is the fallback when the button is too narrow for a label;
    // both stay single-width and terminal-safe (`STYLE.md` §1).
    let (label, glyph) = match v.kind {
        ButtonKind::Dash => ("DASH", '>'),
        ButtonKind::Actives => ("ACT", '*'),
        ButtonKind::Pause => ("II", 'P'),
        ButtonKind::Back => ("BACK", '<'),
        ButtonKind::Map => ("MAP", 'M'),
    };
    let ink_fg = if v.pressed {
        ink(HOT, depth)
    } else if v.ready {
        ink(LABEL, depth)
    } else {
        ink(DIM, depth)
    };
    let edge = ink(if v.pressed { HOT } else { EDGE }, depth);
    // The frame: corners plus a rule along the top, translucent over the floor.
    for x in rect.x..rect.right() {
        stamp(buf, x, rect.y, "─", edge);
    }
    stamp(buf, rect.x, rect.y, "┌", edge);
    stamp(buf, rect.right().saturating_sub(1), rect.y, "┐", edge);
    if rect.height > 1 {
        let row = rect.bottom().saturating_sub(1);
        stamp(buf, rect.x, row, "└", edge);
        stamp(buf, rect.right().saturating_sub(1), row, "┘", edge);
    }
    let inner = rect.width.saturating_sub(2) as usize;
    let text: String = if inner == 0 {
        glyph.to_string()
    } else {
        label.chars().take(inner).collect()
    };
    let x = rect.x.saturating_add(1);
    let y = rect.y.saturating_add(1).min(rect.bottom().saturating_sub(1));
    buf.set_stringn(x, y, &text, inner.max(1), Style::default().fg(ink_fg));
    // The charge meter, only while a Super Dash hold is filling.
    if v.charge > 0.0 && rect.height > 1 {
        let row = rect.bottom().saturating_sub(1);
        let filled = (v.charge.clamp(0.0, 1.0) * f32::from(rect.width)) as u16;
        for i in 0..filled {
            stamp(buf, rect.x.saturating_add(i), row, "▁", ink(HOT, depth));
        }
    }
}
