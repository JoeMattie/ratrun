//! On-screen touch controls: floating analog stick and translucent action
//! buttons, rendered directly into ratatui's [`Buffer`] over the playfield and
//! menus.
//!
//! All colours stay in Rat Run's facility palette (`PLATE`, `EDGE`, `MUTED`,
//! `COAT`, `TAIL`, Super Dash ice-blue, active-weapon amber) and blend toward
//! each cell's existing foreground and background so the field still reads
//! underneath. The two action buttons are the ones a thumb has to find, so Dash
//! and Actives rest at ~65 % plate ink with a full-contrast glyph, and press
//! down darker still. Pause, Back and Map stay lighter (~45 %), and the stick
//! keeps its own translucency. Every emitted colour folds cleanly through
//! [`palette::to_term`] at truecolor, 256-colour and 16-colour depths.
//!
//! # Zero-Allocation Contract
//!
//! [`draw_stick`] and [`draw_button`] work purely on stack arrays and write
//! directly into `buf` without allocating.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::render::palette::{self, Rgb};
use crate::render::subcell::Grid;
use crate::term::{self, BlockSet, ColorDepth};

// ---- Public Contract API ---------------------------------------------------

/// State of the floating analog movement stick in fractional terminal-cell
/// coordinates `(col, row)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickView {
    pub base: (f32, f32),
    pub knob: (f32, f32),
    pub radius_cells: f32,
}

/// On-screen touch button identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ButtonKind {
    Dash,
    Actives,
    Pause,
    Back,
    Map,
}

/// State of one on-screen touch button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonView {
    pub kind: ButtonKind,
    pub rect: Rect,
    pub pressed: bool,
    pub ready: bool,
    /// `0.0..=1.0`, used by [`ButtonKind::Dash`] while charging Super Dash.
    pub charge: f32,
}

/// Preferred `(width, height)` in terminal cells for each touch button, sized
/// for thumbs on a `100 × 36` landscape grid.
pub fn button_size(kind: ButtonKind) -> (u16, u16) {
    match kind {
        ButtonKind::Dash => (7, 3),
        ButtonKind::Actives | ButtonKind::Pause | ButtonKind::Back | ButtonKind::Map => (5, 3),
    }
}

/// Draw the floating analog stick into `buf`, using the finest round block
/// glyph family the terminal supports ([`term::block_set`]).
pub fn draw_stick(buf: &mut Buffer, v: &StickView, depth: ColorDepth) {
    draw_stick_with_blocks(buf, v, depth, term::block_set());
}

/// Draw one on-screen touch button strictly inside `v.rect` (clipped to
/// `buf.area`).
pub fn draw_button(buf: &mut Buffer, v: &ButtonView, depth: ColorDepth) {
    draw_button_with_blocks(buf, v, depth, term::block_set());
}

// ---- Facility Touch Palette ------------------------------------------------

/// Dark steel HUD plate (`palette::PLATE`).
const PLATE_RGB: Rgb = (19, 24, 27);
/// Recessed dark bay for unavailable/recharging buttons (`hud::RACK_BG`).
const PLATE_DIM_RGB: Rgb = (14, 18, 21);
/// Dark groove for bar tracks (`palette::TRACK`).
const TRACK_RGB: Rgb = (44, 52, 56);
/// Cool steel border shade (`palette::EDGE`).
const EDGE_RGB: Rgb = (82, 96, 100);
/// Lit upper-left plate bevel.
const EDGE_HI_RGB: Rgb = (146, 164, 168);
/// Secondary muted ink (`palette::MUTED`).
const MUTED_RGB: Rgb = (156, 167, 169);
/// Primary rat-coat ink (`palette::COAT`).
const COAT_RGB: Rgb = (235, 232, 232);
/// Sunlit upper-left knob highlight.
const COAT_HI_RGB: Rgb = (252, 250, 246);
/// Warm active-drag knob face tint.
const KNOB_DRAG_RGB: Rgb = (248, 238, 214);
/// Shaded lower-right knob bevel.
const KNOB_SHADE_RGB: Rgb = (164, 174, 180);
/// Dark keyline that keeps the stick knob readable over white explosions and
/// bright floor effects (folds to `Black` in 16-colour mode).
const KEYLINE_RGB: Rgb = (14, 18, 22);
/// Super Dash charging blue-white (matches `hud.rs` Dash charge bar).
const DASH_CHARGE_RGB: Rgb = (198, 230, 255);
/// Super Dash full-charge hot ivory-white.
const DASH_FULL_RGB: Rgb = (255, 252, 232);
/// Active-weapon warm amber-gold (matches `hud::WARN_AMBER` / `ActivesHint`).
const ACTIVE_GOLD_RGB: Rgb = (246, 194, 96);
/// Bright core for the active-weapon sonic ring.
const ACTIVE_CORE_RGB: Rgb = (255, 236, 178);
/// Subtle cool-blue tint for the Dash plate while holding a charge.
const DASH_CHARGE_PLATE: Rgb = (42, 74, 102);

// ---- Colour Reading & Blending ---------------------------------------------

#[inline]
fn color_to_rgb(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset => (9, 10, 13),
        Color::Black => (0, 0, 0),
        Color::Red => (128, 0, 0),
        Color::Green => (0, 128, 0),
        Color::Yellow => (128, 128, 0),
        Color::Blue => (0, 0, 128),
        Color::Magenta => (128, 0, 128),
        Color::Cyan => (0, 128, 128),
        Color::Gray => (192, 192, 192),
        Color::DarkGray => (128, 128, 128),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (0, 0, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::White => (255, 255, 255),
        Color::Indexed(idx) => indexed_to_rgb(idx),
    }
}

#[inline]
fn indexed_to_rgb(idx: u8) -> Rgb {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if idx < 16 {
        match idx {
            0 => (0, 0, 0),
            1 => (128, 0, 0),
            2 => (0, 128, 0),
            3 => (128, 128, 0),
            4 => (0, 0, 128),
            5 => (128, 0, 128),
            6 => (0, 128, 128),
            7 => (192, 192, 192),
            8 => (128, 128, 128),
            9 => (255, 0, 0),
            10 => (0, 255, 0),
            11 => (255, 255, 0),
            12 => (0, 0, 255),
            13 => (255, 0, 255),
            14 => (0, 255, 255),
            _ => (255, 255, 255),
        }
    } else if idx < 232 {
        let i = (idx - 16) as usize;
        (CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
    } else {
        let v = 8 + 10 * (idx - 232);
        (v, v, v)
    }
}

/// Extract the effective `(top, bottom)` RGB colours of an existing buffer cell,
/// respecting `Modifier::REVERSED` and half/full/sub-cell block glyphs.
#[inline]
fn read_cell_halves(cell: &Cell) -> (Rgb, Rgb) {
    let reversed = cell.modifier.contains(Modifier::REVERSED);
    let (fg_c, bg_c) = if reversed {
        (cell.bg, cell.fg)
    } else {
        (cell.fg, cell.bg)
    };
    let fg = color_to_rgb(fg_c);
    let bg = color_to_rgb(bg_c);
    match cell.symbol() {
        "▀" => (fg, bg),
        "▄" => (bg, fg),
        "█" => (fg, fg),
        " " | "" => (bg, bg),
        s => {
            // Sub-cell block glyphs (quadrants, sextants, octants, left/right
            // halves) mix fg and bg across the cell; text cells are mostly bg.
            let ch = s.chars().next().unwrap_or(' ');
            if is_block_glyph(ch) {
                let mid = palette::mix(bg, fg, 0.5);
                (mid, mid)
            } else {
                (bg, bg)
            }
        }
    }
}

/// Average underlying RGB of a buffer cell for translucent plate blending.
#[inline]
fn read_cell_avg(cell: &Cell) -> Rgb {
    let (top, bot) = read_cell_halves(cell);
    palette::mix(top, bot, 0.5)
}

#[inline]
fn is_block_glyph(ch: char) -> bool {
    matches!(
        ch,
        '\u{2580}'..='\u{259F}' | '\u{1FB00}'..='\u{1FB3B}' | '\u{1CD00}'..='\u{1CDE5}'
    )
}

#[inline]
fn is_box_drawing(ch: char) -> bool {
    matches!(ch, '\u{2500}'..='\u{257F}')
}

/// Keep a visible button glyph from vanishing into its own plate background
/// when 16-colour folding maps both to the same ANSI slot.
#[inline]
fn ensure_contrast_16(fg: Color, bg: Color, pressed: bool) -> Color {
    if fg != bg {
        return fg;
    }
    if pressed {
        if bg == Color::Black {
            Color::White
        } else {
            Color::Black
        }
    } else if matches!(
        bg,
        Color::Black | Color::DarkGray | Color::Blue | Color::Red | Color::Magenta
    ) {
        Color::White
    } else {
        Color::Black
    }
}

// ---- Floating Analog Stick -------------------------------------------------

/// Evaluate the stick's colour and opacity `(rgb, alpha)` at visual-space
/// point `(px, py)` (where 1 column = 1.0 and 1 row = 2.0). Returns `None`
/// outside the stick's footprint.
#[allow(clippy::too_many_arguments)]
#[inline]
fn sample_stick(
    px: f32,
    py: f32,
    bx: f32,
    by: f32,
    kx: f32,
    ky: f32,
    r: f32,
    knob_r: f32,
    min_ring_w: f32,
    active: bool,
) -> Option<(Rgb, f32)> {
    let ring_w = (r * 0.14).clamp(min_ring_w, 1.05);
    let key_w = (knob_r * 0.36).clamp(0.42, 0.80);

    // 1. Solid knob with top-left shading + dark keyline.
    let kdx = px - kx;
    let kdy = py - ky;
    let dk = (kdx * kdx + kdy * kdy).sqrt();
    if dk <= knob_r {
        let lit = kdx + kdy;
        let face = if lit < -knob_r * 0.45 {
            COAT_HI_RGB
        } else if lit > knob_r * 0.55 {
            KNOB_SHADE_RGB
        } else if active {
            KNOB_DRAG_RGB
        } else {
            COAT_RGB
        };
        let alpha = if active { 0.78 } else { 0.56 };
        return Some((face, alpha));
    }
    if dk <= knob_r + key_w {
        return Some((KEYLINE_RGB, 0.86));
    }

    // 2. Soft base ring (centred on `r`), subtle outer keyline, and translucent dish interior.
    let bdx = px - bx;
    let bdy = py - by;
    let db = (bdx * bdx + bdy * bdy).sqrt();
    let half_rw = ring_w * 0.5;
    let inner_r = (r - half_rw).max(0.0);
    let outer_r = r + half_rw;
    if db >= inner_r && db <= outer_r {
        let lit = bdx + bdy < 0.0;
        let ring_rgb = if lit {
            if active {
                COAT_RGB
            } else {
                palette::mix(MUTED_RGB, COAT_RGB, 0.65)
            }
        } else {
            MUTED_RGB
        };
        let alpha = if active { 0.56 } else { 0.46 };
        return Some((ring_rgb, alpha));
    }
    // Subtle dark outer halo so the soft ring still reads on bright floors.
    if db > outer_r && db <= outer_r + 0.55 {
        return Some((KEYLINE_RGB, 0.36));
    }
    // Faint origin dot once the knob pulls away from centre.
    if active && db <= 0.55 {
        return Some((MUTED_RGB, 0.42));
    }
    if db < inner_r {
        return Some((PLATE_RGB, 0.24));
    }
    None
}

#[inline]
fn rgb_dist_sq(a: Rgb, b: Rgb) -> u32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    (dr * dr + dg * dg + db * db) as u32
}

#[inline]
fn rgb_luma(c: Rgb) -> u32 {
    u32::from(c.0) * 3 + u32::from(c.1) * 6 + u32::from(c.2)
}

/// Fit `px` (`grid.subpixels()` sub-pixels in row-major order) into one
/// quadrant or sextant glyph with two colours.
fn encode_stick_subcell(px: &[Rgb], grid: Grid) -> (char, Rgb, Rgb) {
    let n = grid.subpixels().min(px.len());
    let px = &px[..n];
    let (mut lo, mut hi) = (px[0], px[0]);
    let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
    for &c in px {
        lo = (lo.0.min(c.0), lo.1.min(c.1), lo.2.min(c.2));
        hi = (hi.0.max(c.0), hi.1.max(c.1), hi.2.max(c.2));
        sr += u32::from(c.0);
        sg += u32::from(c.1);
        sb += u32::from(c.2);
    }
    let spread = (i32::from(hi.0) - i32::from(lo.0))
        + (i32::from(hi.1) - i32::from(lo.1))
        + (i32::from(hi.2) - i32::from(lo.2));
    let nn = n as u32;
    let mean = ((sr / nn) as u8, (sg / nn) as u8, (sb / nn) as u8);
    if spread <= 8 {
        return ('▀', mean, mean);
    }

    // Two-means seeds: farthest from mean, then farthest from seed A.
    let mut a = 0usize;
    let mut a_d = 0u32;
    for (i, &c) in px.iter().enumerate() {
        let d = rgb_dist_sq(c, mean);
        if d > a_d {
            a_d = d;
            a = i;
        }
    }
    let mut b = a;
    let mut b_d = 0u32;
    for (i, &c) in px.iter().enumerate() {
        let d = rgb_dist_sq(c, px[a]);
        if d > b_d {
            b_d = d;
            b = i;
        }
    }
    if b_d == 0 {
        return ('▀', mean, mean);
    }

    let (mut ca, mut cb) = (px[a], px[b]);
    let mut mask = 0u8;
    for _ in 0..2 {
        let mut next = 0u8;
        let mut sa = [0u32; 4];
        let mut sbb = [0u32; 4];
        for (i, &c) in px.iter().enumerate() {
            if rgb_dist_sq(c, ca) <= rgb_dist_sq(c, cb) {
                next |= 1 << i;
                sa[0] += u32::from(c.0);
                sa[1] += u32::from(c.1);
                sa[2] += u32::from(c.2);
                sa[3] += 1;
            } else {
                sbb[0] += u32::from(c.0);
                sbb[1] += u32::from(c.1);
                sbb[2] += u32::from(c.2);
                sbb[3] += 1;
            }
        }
        if sa[3] == 0 || sbb[3] == 0 {
            break;
        }
        mask = next;
        ca = ((sa[0] / sa[3]) as u8, (sa[1] / sa[3]) as u8, (sa[2] / sa[3]) as u8);
        cb = ((sbb[0] / sbb[3]) as u8, (sbb[1] / sbb[3]) as u8, (sbb[2] / sbb[3]) as u8);
    }
    let full = ((1u16 << n) - 1) as u8;
    if mask == 0 || mask == full || ca == cb {
        return ('▀', mean, mean);
    }
    if rgb_luma(ca) >= rgb_luma(cb) {
        (grid.glyph(mask), ca, cb)
    } else {
        (grid.glyph(!mask & full), cb, ca)
    }
}

/// [`draw_stick`] with an explicit [`BlockSet`] for deterministic testing.
pub(crate) fn draw_stick_with_blocks(
    buf: &mut Buffer,
    v: &StickView,
    depth: ColorDepth,
    blocks: BlockSet,
) {
    if buf.area.is_empty() {
        return;
    }
    if !v.base.0.is_finite()
        || !v.base.1.is_finite()
        || !v.knob.0.is_finite()
        || !v.knob.1.is_finite()
        || !v.radius_cells.is_finite()
        || v.radius_cells <= 0.05
    {
        return;
    }

    let r = v.radius_cells;
    let bx = v.base.0;
    let by = v.base.1 * 2.0;
    let kx = v.knob.0;
    let ky = v.knob.1 * 2.0;
    let knob_r = (r * 0.36).clamp(0.45, 2.5).min(r * 0.72);
    let disp = ((kx - bx) * (kx - bx) + (ky - by) * (ky - by)).sqrt();
    let active = disp > (r * 0.12).max(0.25);

    // Bounding box in cell coordinates covering both base (+ ring + halo) and knob (+ keyline).
    let base_pad = r + 1.2;
    let knob_pad = knob_r + 1.0;
    let min_x_f = (bx - base_pad).min(kx - knob_pad).floor();
    let max_x_f = (bx + base_pad).max(kx + knob_pad).ceil();
    let min_y_f = ((by - base_pad).min(ky - knob_pad) * 0.5).floor();
    let max_y_f = ((by + base_pad).max(ky + knob_pad) * 0.5).ceil();

    let area_l = i32::from(buf.area.left());
    let area_r = i32::from(buf.area.right());
    let area_t = i32::from(buf.area.top());
    let area_b = i32::from(buf.area.bottom());

    let x0 = (min_x_f.clamp(area_l as f32, area_r as f32) as i32).max(area_l);
    let x1 = (max_x_f.clamp(area_l as f32, area_r as f32) as i32).min(area_r);
    let y0 = (min_y_f.clamp(area_t as f32, area_b as f32) as i32).max(area_t);
    let y1 = (max_y_f.clamp(area_t as f32, area_b as f32) as i32).min(area_b);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let grid = match blocks {
        BlockSet::Half => None,
        BlockSet::Quad => Some(Grid::Quad),
        BlockSet::Sextant | BlockSet::Octant => Some(Grid::Sextant),
    };
    // Half-block mode samples once per column (`1 × 2`), so widen the ring
    // slightly so its left and right flanks stay unbroken.
    let min_ring_w = if grid.is_none() { 0.95 } else { 0.42 };

    for cy in y0..y1 {
        for cx in x0..x1 {
            let Some(cell) = buf.cell_mut((cx as u16, cy as u16)) else {
                continue;
            };
            let (old_top, old_bot) = read_cell_halves(cell);
            let old_mid = palette::mix(old_top, old_bot, 0.5);

            match grid {
                None => {
                    // Half-block path (`1 × 2`): safe on every terminal.
                    let px = cx as f32 + 0.5;
                    let top_s = sample_stick(
                        px,
                        (cy as f32 + 0.25) * 2.0,
                        bx,
                        by,
                        kx,
                        ky,
                        r,
                        knob_r,
                        min_ring_w,
                        active,
                    );
                    let bot_s = sample_stick(
                        px,
                        (cy as f32 + 0.75) * 2.0,
                        bx,
                        by,
                        kx,
                        ky,
                        r,
                        knob_r,
                        min_ring_w,
                        active,
                    );
                    if top_s.is_none() && bot_s.is_none() {
                        continue;
                    }
                    let top_rgb = match top_s {
                        Some((c, a)) => palette::mix(old_top, c, a),
                        None => old_top,
                    };
                    let bot_rgb = match bot_s {
                        Some((c, a)) => palette::mix(old_bot, c, a),
                        None => old_bot,
                    };
                    cell.modifier = Modifier::empty();
                    cell.set_symbol("▀");
                    cell.set_fg(palette::to_term(top_rgb, depth));
                    cell.set_bg(palette::to_term(bot_rgb, depth));
                }
                Some(g) => {
                    // Quadrant (`2 × 2`) or Sextant (`2 × 3`) round path.
                    let rows = g.rows();
                    let mut samples = [(0u8, 0u8, 0u8); 6];
                    let mut touched = false;
                    for sy in 0..rows {
                        let py = (cy as f32 + (sy as f32 + 0.5) / rows as f32) * 2.0;
                        let bg_row = if rows == 3 && sy == 1 {
                            old_mid
                        } else if sy * 2 < rows {
                            old_top
                        } else {
                            old_bot
                        };
                        for sx in 0..2 {
                            let px = cx as f32 + (sx as f32 + 0.5) * 0.5;
                            let idx = sy * 2 + sx;
                            if let Some((c, a)) = sample_stick(
                                px, py, bx, by, kx, ky, r, knob_r, min_ring_w, active,
                            ) {
                                touched = true;
                                samples[idx] = palette::mix(bg_row, c, a);
                            } else {
                                samples[idx] = bg_row;
                            }
                        }
                    }
                    if !touched {
                        continue;
                    }
                    let (ch, fg_rgb, bg_rgb) = encode_stick_subcell(&samples[..rows * 2], g);
                    let mut gbuf = [0u8; 4];
                    cell.modifier = Modifier::empty();
                    cell.set_symbol(ch.encode_utf8(&mut gbuf));
                    cell.set_fg(palette::to_term(fg_rgb, depth));
                    cell.set_bg(palette::to_term(bg_rgb, depth));
                }
            }
        }
    }
}

// ---- Translucent Touch Buttons ---------------------------------------------

#[derive(Clone, Copy)]
struct ButtonPalette {
    plate_rgb: Rgb,
    plate_alpha: f32,
    edge_hi_rgb: Rgb,
    edge_hi_alpha: f32,
    edge_lo_rgb: Rgb,
    edge_lo_alpha: f32,
    icon_rgb: Rgb,
    icon_sub_rgb: Rgb,
    icon_alpha: f32,
    bold: bool,
}

fn button_palette(v: &ButtonView, dash_charge: f32) -> ButtonPalette {
    let charging = dash_charge > 0.01;
    let charge_full = dash_charge >= 0.99;

    if v.pressed {
        let plate = match v.kind {
            ButtonKind::Dash if charging => {
                palette::mix(DASH_CHARGE_RGB, DASH_FULL_RGB, dash_charge)
            }
            ButtonKind::Actives => ACTIVE_GOLD_RGB,
            _ => COAT_RGB,
        };
        return ButtonPalette {
            plate_rgb: plate,
            plate_alpha: 0.78,
            edge_hi_rgb: PLATE_RGB,
            edge_hi_alpha: 0.86,
            edge_lo_rgb: PLATE_RGB,
            edge_lo_alpha: 0.86,
            icon_rgb: PLATE_RGB,
            icon_sub_rgb: TRACK_RGB,
            icon_alpha: 0.96,
            bold: true,
        };
    }

    if !v.ready {
        // Recharging: still dimmer than at rest, but its glyph is legible.
        return ButtonPalette {
            plate_rgb: PLATE_DIM_RGB,
            plate_alpha: 0.54,
            edge_hi_rgb: EDGE_RGB,
            edge_hi_alpha: 0.60,
            edge_lo_rgb: TRACK_RGB,
            edge_lo_alpha: 0.52,
            icon_rgb: MUTED_RGB,
            icon_sub_rgb: EDGE_RGB,
            icon_alpha: 0.82,
            bold: false,
        };
    }

    match v.kind {
        ButtonKind::Dash => {
            if charge_full {
                ButtonPalette {
                    plate_rgb: palette::mix(DASH_CHARGE_PLATE, DASH_CHARGE_RGB, 0.30),
                    plate_alpha: 0.70,
                    edge_hi_rgb: DASH_FULL_RGB,
                    edge_hi_alpha: 1.0,
                    edge_lo_rgb: DASH_CHARGE_RGB,
                    edge_lo_alpha: 0.92,
                    icon_rgb: DASH_FULL_RGB,
                    icon_sub_rgb: DASH_CHARGE_RGB,
                    icon_alpha: 1.0,
                    bold: true,
                }
            } else if charging {
                ButtonPalette {
                    plate_rgb: palette::mix(PLATE_RGB, DASH_CHARGE_PLATE, 0.35 + 0.55 * dash_charge),
                    plate_alpha: 0.66,
                    edge_hi_rgb: DASH_CHARGE_RGB,
                    edge_hi_alpha: 0.88,
                    edge_lo_rgb: EDGE_RGB,
                    edge_lo_alpha: 0.62,
                    icon_rgb: DASH_CHARGE_RGB,
                    icon_sub_rgb: MUTED_RGB,
                    icon_alpha: 0.96,
                    bold: true,
                }
            } else {
                // Resting: the thumb has to find it on a dark floor, so the
                // plate carries ~65 % ink and the streak is full contrast.
                ButtonPalette {
                    plate_rgb: PLATE_RGB,
                    plate_alpha: 0.65,
                    edge_hi_rgb: EDGE_HI_RGB,
                    edge_hi_alpha: 0.78,
                    edge_lo_rgb: EDGE_RGB,
                    edge_lo_alpha: 0.66,
                    icon_rgb: COAT_RGB,
                    icon_sub_rgb: MUTED_RGB,
                    icon_alpha: 1.0,
                    bold: true,
                }
            }
        }
        ButtonKind::Actives => ButtonPalette {
            plate_rgb: PLATE_RGB,
            plate_alpha: 0.65,
            edge_hi_rgb: ACTIVE_GOLD_RGB,
            edge_hi_alpha: 0.80,
            edge_lo_rgb: EDGE_RGB,
            edge_lo_alpha: 0.66,
            icon_rgb: ACTIVE_CORE_RGB,
            icon_sub_rgb: ACTIVE_GOLD_RGB,
            icon_alpha: 1.0,
            bold: true,
        },
        ButtonKind::Pause | ButtonKind::Back | ButtonKind::Map => ButtonPalette {
            plate_rgb: PLATE_RGB,
            plate_alpha: 0.46,
            edge_hi_rgb: EDGE_HI_RGB,
            edge_hi_alpha: 0.52,
            edge_lo_rgb: EDGE_RGB,
            edge_lo_alpha: 0.42,
            icon_rgb: COAT_RGB,
            icon_sub_rgb: MUTED_RGB,
            icon_alpha: 0.82,
            bold: true,
        },
    }
}

/// Blend the translucent plate background into one cell (single pass, never
/// double-blended by subsequent border or icon glyphs).
fn fill_plate_bg(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    bg_rgb: Rgb,
    bg_alpha: f32,
    depth: ColorDepth,
) {
    let Some(cell) = buf.cell_mut((x, y)) else {
        return;
    };
    let under = read_cell_avg(cell);
    let out_bg = palette::to_term(palette::mix(under, bg_rgb, bg_alpha), depth);
    cell.set_symbol(" ");
    cell.set_fg(out_bg);
    cell.set_bg(out_bg);
    cell.modifier = Modifier::empty();
}

/// Stamp a foreground glyph onto a cell whose translucent plate background has
/// already been blended by [`fill_plate_bg`].
#[allow(clippy::too_many_arguments)]
fn stamp_plate_glyph(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    ch: char,
    fg_rgb: Rgb,
    fg_alpha: f32,
    bold: bool,
    pressed: bool,
    depth: ColorDepth,
) {
    let Some(cell) = buf.cell_mut((x, y)) else {
        return;
    };
    let out_bg = cell.bg;
    let out_bg_rgb = color_to_rgb(out_bg);
    let mut out_fg = palette::to_term(palette::mix(out_bg_rgb, fg_rgb, fg_alpha), depth);
    if ch != ' ' && depth == ColorDepth::Ansi16 {
        out_fg = ensure_contrast_16(out_fg, out_bg, pressed);
    }
    let mut gbuf = [0u8; 4];
    cell.set_symbol(ch.encode_utf8(&mut gbuf));
    cell.set_fg(out_fg);
    // Never bold block elements or box drawing (Menlo Bold drops box-drawing glyphs).
    cell.modifier = if bold && ch != ' ' && !is_block_glyph(ch) && !is_box_drawing(ch) {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
}

/// Stamp a sleek lower-half charge bar across `[x0 .. x0 + width)` on row `y`.
///
/// Uses `▄` (full cell) and `▖` (left half cell when quadrants+ are available)
/// so the bar sits cleanly along the bottom rim of the plate without crowding
/// the icon row above, and every glyph stays inside `U+2580..=U+259F` (safe and
/// pixel-exact on the web canvas).
#[allow(clippy::too_many_arguments)]
fn draw_charge_bar(
    buf: &mut Buffer,
    x0: u16,
    y: u16,
    width: u16,
    charge: f32,
    pal: ButtonPalette,
    pressed: bool,
    depth: ColorDepth,
    blocks: BlockSet,
) {
    if width == 0 {
        return;
    }
    let halves = (charge.clamp(0.0, 1.0) * f32::from(width) * 2.0).round() as u32;
    let fill_rgb = if pressed {
        PLATE_RGB
    } else if charge >= 0.99 {
        DASH_FULL_RGB
    } else {
        DASH_CHARGE_RGB
    };
    let fill_alpha = if pressed { 0.92 } else { 0.94 };
    let use_quad = blocks != BlockSet::Half;

    for i in 0..width {
        let step = halves.saturating_sub(u32::from(i) * 2).min(2);
        let (ch, fg, fa) = match step {
            2 => ('▄', fill_rgb, fill_alpha),
            1 if use_quad => ('▖', fill_rgb, fill_alpha),
            1 => ('▄', palette::mix(pal.edge_lo_rgb, fill_rgb, 0.55), fill_alpha),
            _ => ('─', pal.edge_lo_rgb, pal.edge_lo_alpha),
        };
        stamp_plate_glyph(buf, x0 + i, y, ch, fg, fa, false, pressed, depth);
    }
}

/// Stamp the centered glyph/icon for `v.kind` into the inner row `y`, across
/// `[x0 .. x0 + inner_w)`.
#[allow(clippy::too_many_arguments)]
fn draw_button_icon(
    buf: &mut Buffer,
    x0: u16,
    y: u16,
    inner_w: u16,
    v: &ButtonView,
    pal: ButtonPalette,
    dash_charge: f32,
    depth: ColorDepth,
    blocks: BlockSet,
) {
    if inner_w == 0 {
        return;
    }

    // Build up to 5 `(glyph, is_primary_ink)` slots on the stack.
    let mut glyphs: [(char, bool); 5] = [(' ', true); 5];
    let len: usize = match v.kind {
        ButtonKind::Dash => {
            // Dash streak: ASCII speed trail (`-=` / `==`) into double-chevron `»`
            // (all present in bold Menlo/SF Mono/Consolas and safe in GlyphSet::Safe).
            let strong = dash_charge >= 0.5;
            if inner_w >= 3 {
                glyphs[0] = (if strong { '=' } else { '-' }, false);
                glyphs[1] = ('=', true);
                glyphs[2] = ('»', true);
                3
            } else if inner_w == 2 {
                glyphs[0] = ('=', false);
                glyphs[1] = ('»', true);
                2
            } else {
                glyphs[0] = ('»', true);
                1
            }
        }
        ButtonKind::Actives => {
            // Sonic Squeak / Tail Thrash mark: concentric pulse ring with wave arcs.
            if inner_w >= 3 {
                glyphs[0] = ('(', false);
                glyphs[1] = ('◎', true);
                glyphs[2] = (')', false);
                3
            } else {
                glyphs[0] = ('◎', true);
                1
            }
        }
        ButtonKind::Pause => {
            // Classic twin pause bars `I I` / `II`.
            if inner_w >= 3 {
                glyphs[0] = ('I', true);
                glyphs[1] = (' ', true);
                glyphs[2] = ('I', true);
                3
            } else if inner_w == 2 {
                glyphs[0] = ('I', true);
                glyphs[1] = ('I', true);
                2
            } else {
                glyphs[0] = ('I', true);
                1
            }
        }
        ButtonKind::Back => {
            // Crisp left arrow.
            if inner_w >= 4 {
                glyphs[0] = ('◄', true);
                glyphs[1] = ('-', false);
                2
            } else {
                glyphs[0] = ('←', true);
                1
            }
        }
        ButtonKind::Map => {
            // Three-panel accordion-folded map: lit / shaded crease / lit.
            if inner_w >= 3 {
                let (a, b, c) = match blocks {
                    BlockSet::Sextant | BlockSet::Octant => {
                        // Sextant parallelograms: up-right, down-right, up-right.
                        (
                            Grid::Sextant.glyph(0b01_11_10),
                            Grid::Sextant.glyph(0b10_11_01),
                            Grid::Sextant.glyph(0b01_11_10),
                        )
                    }
                    BlockSet::Quad => ('▟', '▜', '▟'),
                    BlockSet::Half => ('▄', '▀', '▄'),
                };
                glyphs[0] = (a, true);
                glyphs[1] = (b, false);
                glyphs[2] = (c, true);
                3
            } else {
                glyphs[0] = ('M', true);
                1
            }
        }
    };

    let start = x0 + (inner_w.saturating_sub(len as u16)) / 2;
    for (i, &(ch, primary)) in glyphs.iter().take(len).enumerate() {
        if ch == ' ' {
            continue;
        }
        let fg = if primary { pal.icon_rgb } else { pal.icon_sub_rgb };
        let fa = if primary {
            pal.icon_alpha
        } else {
            (pal.icon_alpha * 0.78).min(1.0)
        };
        let is_block = is_block_glyph(ch);
        stamp_plate_glyph(
            buf,
            start + i as u16,
            y,
            ch,
            fg,
            fa,
            pal.bold && !is_block,
            v.pressed,
            depth,
        );
    }
}

/// [`draw_button`] with an explicit [`BlockSet`] for deterministic testing.
pub(crate) fn draw_button_with_blocks(
    buf: &mut Buffer,
    v: &ButtonView,
    depth: ColorDepth,
    blocks: BlockSet,
) {
    let r = v.rect.intersection(buf.area);
    if r.width == 0 || r.height == 0 {
        return;
    }

    let charge = if v.charge.is_finite() {
        v.charge.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dash_charge = if v.kind == ButtonKind::Dash {
        charge
    } else {
        0.0
    };
    let pal = button_palette(v, dash_charge);

    // 1. Single-pass translucent plate fill over every cell inside `r`.
    for row in 0..r.height {
        let y = r.y + row;
        for col in 0..r.width {
            let x = r.x + col;
            // On a 1-row Dash button with active charge, tint filled columns
            // left-to-right so charge still reads even at degenerate height.
            let (bg, ba) = if r.height == 1
                && dash_charge > 0.01
                && (f32::from(col) + 0.5) / f32::from(r.width) <= dash_charge
            {
                (
                    if dash_charge >= 0.99 {
                        DASH_FULL_RGB
                    } else {
                        DASH_CHARGE_RGB
                    },
                    0.68,
                )
            } else {
                (pal.plate_rgb, pal.plate_alpha)
            };
            fill_plate_bg(buf, x, y, bg, ba, depth);
        }
    }

    // 2. Stamped metal bevel border (`width >= 3 && height >= 3`).
    if r.width >= 3 && r.height >= 3 {
        let x_l = r.x;
        let x_r = r.x + r.width - 1;
        let y_t = r.y;
        let y_b = r.y + r.height - 1;

        // Top & bottom corners.
        stamp_plate_glyph(buf, x_l, y_t, '┌', pal.edge_hi_rgb, pal.edge_hi_alpha, false, v.pressed, depth);
        stamp_plate_glyph(buf, x_r, y_t, '┐', pal.edge_hi_rgb, pal.edge_hi_alpha, false, v.pressed, depth);
        stamp_plate_glyph(buf, x_l, y_b, '└', pal.edge_hi_rgb, pal.edge_hi_alpha, false, v.pressed, depth);
        stamp_plate_glyph(buf, x_r, y_b, '┘', pal.edge_lo_rgb, pal.edge_lo_alpha, false, v.pressed, depth);

        // Top & bottom horizontal rails.
        for x in (x_l + 1)..x_r {
            stamp_plate_glyph(buf, x, y_t, '─', pal.edge_hi_rgb, pal.edge_hi_alpha, false, v.pressed, depth);
            if dash_charge <= 0.01 {
                stamp_plate_glyph(buf, x, y_b, '─', pal.edge_lo_rgb, pal.edge_lo_alpha, false, v.pressed, depth);
            }
        }
        // Left & right vertical rails.
        for y in (y_t + 1)..y_b {
            stamp_plate_glyph(buf, x_l, y, '│', pal.edge_hi_rgb, pal.edge_hi_alpha, false, v.pressed, depth);
            stamp_plate_glyph(buf, x_r, y, '│', pal.edge_lo_rgb, pal.edge_lo_alpha, false, v.pressed, depth);
        }

        // Dash charge bar along the bottom rail.
        if dash_charge > 0.01 {
            draw_charge_bar(
                buf,
                x_l + 1,
                y_b,
                r.width - 2,
                dash_charge,
                pal,
                v.pressed,
                depth,
                blocks,
            );
        }

        // Centered icon in the interior.
        let mid_y = r.y + r.height / 2;
        draw_button_icon(
            buf,
            x_l + 1,
            mid_y,
            r.width - 2,
            v,
            pal,
            dash_charge,
            depth,
            blocks,
        );
    } else if r.height == 2 {
        // Compact 2-row button: icon on row 0, charge bar or subtle bottom rule on row 1.
        draw_button_icon(buf, r.x, r.y, r.width, v, pal, dash_charge, depth, blocks);
        if dash_charge > 0.01 {
            draw_charge_bar(
                buf,
                r.x,
                r.y + 1,
                r.width,
                dash_charge,
                pal,
                v.pressed,
                depth,
                blocks,
            );
        } else {
            for col in 0..r.width {
                stamp_plate_glyph(
                    buf,
                    r.x + col,
                    r.y + 1,
                    '─',
                    pal.edge_lo_rgb,
                    pal.edge_lo_alpha,
                    false,
                    v.pressed,
                    depth,
                );
            }
        }
    } else {
        // Tiny 1-row or narrow strip: centered icon directly on the translucent plate.
        let mid_y = r.y + r.height / 2;
        draw_button_icon(buf, r.x, mid_y, r.width, v, pal, dash_charge, depth, blocks);
    }
}

// ---- Unit Tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{sanitize_with, GlyphSet};

    const ALL_KINDS: [ButtonKind; 5] = [
        ButtonKind::Dash,
        ButtonKind::Actives,
        ButtonKind::Pause,
        ButtonKind::Back,
        ButtonKind::Map,
    ];

    const ALL_DEPTHS: [ColorDepth; 3] = [
        ColorDepth::Truecolor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
    ];

    const ALL_BLOCKS: [BlockSet; 4] = [
        BlockSet::Half,
        BlockSet::Quad,
        BlockSet::Sextant,
        BlockSet::Octant,
    ];

    fn fill_playfield(area: Rect, top: Rgb, bot: Rgb, depth: ColorDepth) -> Buffer {
        let mut buf = Buffer::empty(area);
        let fg = palette::to_term(top, depth);
        let bg = palette::to_term(bot, depth);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let cell = &mut buf[(x, y)];
                cell.set_symbol("▀");
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
        buf
    }

    fn matches_depth(c: Color, depth: ColorDepth) -> bool {
        match depth {
            ColorDepth::Truecolor => matches!(c, Color::Rgb(_, _, _)),
            ColorDepth::Ansi256 => matches!(c, Color::Indexed(_)),
            ColorDepth::Ansi16 => matches!(
                c,
                Color::Black
                    | Color::Red
                    | Color::Green
                    | Color::Yellow
                    | Color::Blue
                    | Color::Magenta
                    | Color::Cyan
                    | Color::Gray
                    | Color::DarkGray
                    | Color::LightRed
                    | Color::LightGreen
                    | Color::LightYellow
                    | Color::LightBlue
                    | Color::LightMagenta
                    | Color::LightCyan
                    | Color::White
            ),
        }
    }

    #[test]
    fn button_sizes_are_thumb_sized() {
        assert_eq!(button_size(ButtonKind::Dash), (7, 3));
        for kind in [
            ButtonKind::Actives,
            ButtonKind::Pause,
            ButtonKind::Back,
            ButtonKind::Map,
        ] {
            assert_eq!(button_size(kind), (5, 3));
        }
    }

    #[test]
    fn every_button_kind_draws_inside_its_rect_at_all_depths() {
        let area = Rect::new(0, 0, 24, 12);
        for depth in ALL_DEPTHS {
            let sentinel_top = (36, 52, 48);
            let sentinel_bot = (28, 42, 40);
            let base = fill_playfield(area, sentinel_top, sentinel_bot, depth);

            for kind in ALL_KINDS {
                let (w, h) = button_size(kind);
                let rect = Rect::new(6, 4, w, h);

                let states: [(bool, bool, f32); 5] = [
                    (false, true, 0.0),
                    (false, false, 0.0),
                    (true, true, 0.0),
                    (false, true, 0.5),
                    (true, true, 1.0),
                ];

                let mut idle_buf = None;
                let mut unready_buf = None;
                let mut pressed_buf = None;
                let mut half_buf = None;

                for (idx, &(pressed, ready, charge)) in states.iter().enumerate() {
                    let mut buf = base.clone();
                    let view = ButtonView {
                        kind,
                        rect,
                        pressed,
                        ready,
                        charge,
                    };
                    draw_button(&mut buf, &view, depth);

                    let mut changed_inside = 0usize;
                    for y in area.top()..area.bottom() {
                        for x in area.left()..area.right() {
                            let inside = rect.contains((x, y).into());
                            if !inside {
                                assert_eq!(
                                    buf[(x, y)],
                                    base[(x, y)],
                                    "{kind:?} leaked outside rect at ({x},{y}) depth={depth:?}"
                                );
                            } else {
                                if buf[(x, y)] != base[(x, y)] {
                                    changed_inside += 1;
                                }
                                assert!(
                                    matches_depth(buf[(x, y)].fg, depth),
                                    "{kind:?} fg {:?} violated depth {depth:?}",
                                    buf[(x, y)].fg
                                );
                                assert!(
                                    matches_depth(buf[(x, y)].bg, depth),
                                    "{kind:?} bg {:?} violated depth {depth:?}",
                                    buf[(x, y)].bg
                                );
                            }
                        }
                    }
                    assert!(
                        changed_inside >= usize::from(w * h),
                        "{kind:?} should paint its full plate at {depth:?}"
                    );
                    match idx {
                        0 => idle_buf = Some(buf),
                        1 => unready_buf = Some(buf),
                        2 => pressed_buf = Some(buf),
                        3 => half_buf = Some(buf),
                        _ => {}
                    }
                }

                // Visual states must remain distinct at Truecolor and 256-colour
                // (and pressed must stay distinct even in 16-colour).
                let idle = idle_buf.unwrap();
                let pressed = pressed_buf.unwrap();
                assert_ne!(
                    idle.content, pressed.content,
                    "{kind:?} pressed must visually differ from idle at {depth:?}"
                );
                if depth != ColorDepth::Ansi16 {
                    let unready = unready_buf.unwrap();
                    assert_ne!(
                        idle.content, unready.content,
                        "{kind:?} ready=false must dim the button at {depth:?}"
                    );
                }
                if kind == ButtonKind::Dash {
                    let half = half_buf.unwrap();
                    assert_ne!(
                        idle.content, half.content,
                        "Dash charge=0.5 must visibly fill the charge bar at {depth:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn buttons_blend_translucently_over_underlying_field() {
        let area = Rect::new(0, 0, 16, 8);
        let rect = Rect::new(3, 2, 7, 3);
        let view = ButtonView {
            kind: ButtonKind::Dash,
            rect,
            pressed: false,
            ready: true,
            charge: 0.0,
        };

        let mut dark = fill_playfield(area, (20, 24, 24), (20, 24, 24), ColorDepth::Truecolor);
        let mut bright =
            fill_playfield(area, (220, 200, 170), (220, 200, 170), ColorDepth::Truecolor);
        draw_button(&mut dark, &view, ColorDepth::Truecolor);
        draw_button(&mut bright, &view, ColorDepth::Truecolor);

        // Interior space cell beside the icon shows translucent blending.
        let dark_bg = color_to_rgb(dark[(4, 3)].bg);
        let bright_bg = color_to_rgb(bright[(4, 3)].bg);
        assert!(
            bright_bg.0 > dark_bg.0 + 60,
            "underlying floor must show through the plate (dark={dark_bg:?}, bright={bright_bg:?})"
        );
        assert!(
            bright_bg.0 < 180,
            "plate ink must visibly tint the bright floor (bright={bright_bg:?})"
        );
    }

    #[test]
    fn buttons_never_panic_at_tiny_or_clipped_rects() {
        let rects = [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(0, 0, 2, 1),
            Rect::new(0, 0, 1, 2),
            Rect::new(0, 0, 2, 2),
            Rect::new(0, 0, 3, 1),
            Rect::new(0, 0, 3, 2),
            Rect::new(0, 0, 4, 2),
            Rect::new(8, 4, 7, 3),   // partially clipped on 10×6
            Rect::new(50, 50, 7, 3), // completely outside
        ];
        let area = Rect::new(0, 0, 10, 6);
        for depth in ALL_DEPTHS {
            for kind in ALL_KINDS {
                for &rect in &rects {
                    for &charge in &[0.0f32, 0.5, 1.0, -2.0, 5.0, f32::NAN] {
                        let mut buf = fill_playfield(area, (30, 40, 44), (24, 32, 36), depth);
                        let view = ButtonView {
                            kind,
                            rect,
                            pressed: false,
                            ready: true,
                            charge,
                        };
                        draw_button(&mut buf, &view, depth);
                    }
                }
            }
        }
        // Zero-sized buffer must also be a clean no-op.
        let mut empty = Buffer::empty(Rect::new(0, 0, 0, 0));
        draw_button(
            &mut empty,
            &ButtonView {
                kind: ButtonKind::Dash,
                rect: Rect::new(0, 0, 7, 3),
                pressed: false,
                ready: true,
                charge: 0.5,
            },
            ColorDepth::Truecolor,
        );
    }

    #[test]
    fn stick_draws_cleanly_at_all_depths_and_block_sets() {
        let area = Rect::new(0, 0, 30, 16);
        let views = [
            StickView {
                base: (12.0, 8.0),
                knob: (12.0, 8.0),
                radius_cells: 5.5,
            },
            StickView {
                base: (12.0, 8.0),
                knob: (15.5, 6.8),
                radius_cells: 5.5,
            },
        ];

        for depth in ALL_DEPTHS {
            for blocks in ALL_BLOCKS {
                for v in &views {
                    let base = fill_playfield(area, (32, 48, 44), (24, 38, 36), depth);
                    let mut buf = base.clone();
                    draw_stick_with_blocks(&mut buf, v, depth, blocks);

                    // Far corner must stay untouched.
                    assert_eq!(buf[(29, 15)], base[(29, 15)]);
                    assert_eq!(buf[(0, 0)], base[(0, 0)]);

                    // Knob centre must be painted.
                    let kx = v.knob.0.floor() as u16;
                    let ky = v.knob.1.floor() as u16;
                    assert_ne!(
                        buf[(kx, ky)],
                        base[(kx, ky)],
                        "stick knob missing at depth={depth:?} blocks={blocks:?}"
                    );

                    // Every cell in the buffer must obey the requested depth.
                    for y in area.top()..area.bottom() {
                        for x in area.left()..area.right() {
                            assert!(matches_depth(buf[(x, y)].fg, depth));
                            assert!(matches_depth(buf[(x, y)].bg, depth));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn stick_knob_keyline_survives_white_flash() {
        let area = Rect::new(0, 0, 24, 12);
        let mut buf = fill_playfield(area, (255, 255, 255), (255, 255, 255), ColorDepth::Ansi16);
        let v = StickView {
            base: (12.0, 6.0),
            knob: (12.0, 6.0),
            radius_cells: 5.0,
        };
        draw_stick_with_blocks(&mut buf, &v, ColorDepth::Ansi16, BlockSet::Half);
        let has_dark_keyline = buf
            .content
            .iter()
            .any(|c| c.fg == Color::Black || c.bg == Color::Black);
        assert!(
            has_dark_keyline,
            "knob dark keyline must read as Black over a pure white flash in 16 colours"
        );
    }

    #[test]
    fn stick_never_panics_on_tiny_buffers_or_degenerate_inputs() {
        let tiny_areas = [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(0, 0, 2, 2),
        ];
        let bad_views = [
            StickView {
                base: (0.0, 0.0),
                knob: (0.0, 0.0),
                radius_cells: 0.0,
            },
            StickView {
                base: (0.0, 0.0),
                knob: (0.0, 0.0),
                radius_cells: -4.0,
            },
            StickView {
                base: (f32::NAN, 1.0),
                knob: (1.0, 1.0),
                radius_cells: 4.0,
            },
            StickView {
                base: (1.0, 1.0),
                knob: (1.0, 1.0),
                radius_cells: f32::INFINITY,
            },
            StickView {
                base: (-500.0, -500.0),
                knob: (500.0, 500.0),
                radius_cells: 500.0,
            },
            StickView {
                base: (0.5, 0.5),
                knob: (0.5, 0.5),
                radius_cells: 5.0,
            },
        ];
        for area in tiny_areas {
            for depth in ALL_DEPTHS {
                for blocks in ALL_BLOCKS {
                    for v in &bad_views {
                        let mut buf = Buffer::empty(area);
                        draw_stick_with_blocks(&mut buf, v, depth, blocks);
                    }
                }
            }
        }
    }

    #[test]
    fn controls_survive_safe_glyph_sanitize() {
        let area = Rect::new(0, 0, 50, 20);
        let mut buf = fill_playfield(area, (34, 48, 44), (24, 36, 34), ColorDepth::Truecolor);
        draw_stick_with_blocks(
            &mut buf,
            &StickView {
                base: (10.0, 12.0),
                knob: (13.0, 11.0),
                radius_cells: 5.5,
            },
            ColorDepth::Truecolor,
            BlockSet::Half,
        );
        let mut x = 20u16;
        for kind in ALL_KINDS {
            let (w, h) = button_size(kind);
            draw_button_with_blocks(
                &mut buf,
                &ButtonView {
                    kind,
                    rect: Rect::new(x, 6, w, h),
                    pressed: false,
                    ready: true,
                    charge: if kind == ButtonKind::Dash { 0.6 } else { 0.0 },
                },
                ColorDepth::Truecolor,
                BlockSet::Half,
            );
            x += w + 1;
        }
        sanitize_with(&mut buf, GlyphSet::Safe, ColorDepth::Ansi16);
        for cell in &buf.content {
            assert!(!cell.symbol().is_empty());
        }
    }
}
