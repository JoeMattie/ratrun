//! **The gene-splice shop screen** (`WAVE3_DESIGN.md` §6).
//!
//! A subterranean apothecary shelf under the wall
//! ([`menu::panel`](crate::render::menu::panel)) drawn over the frozen play
//! field, showing:
//!
//! * the purse, visual **splice sockets** and the bent-spoon den header;
//! * the three shelf slots — bespoke half-block **specimen vial art** per
//!   [`SpliceKind`], shelf ledge, name, price, and **both** stat changes of
//!   the bargain (the gain and the cost) *before* payment;
//! * the three back-room services (Stitch, Turn the shelf, Molt);
//! * the exit row, the keyboard legend and the splices already in the rat.
//!
//! Every actionable row registers a [`Target`] so the pointer can hover and
//! click it, and every disabled row prints the shelf's own reason
//! ([`Deny::reason`]) instead of just going grey.
//!
//! The Molt flow takes over the panel: step one lists the kinds the rat may
//! leave behind (with their rank and their `✦EVOLVED` mark), step two lists the
//! unowned kinds it may take in their place at rank 1. `Esc` cancels at any
//! point before the second selection, and the panel says so.
//!
//! ## Sizes
//!
//! The layout is row-counted and adapts cleanly to the three sizes the app
//! tests pin: 57 × 20 (compact 4 × 2 vials, 3-row cards), 80 × 24 (compact
//! vials with a shelf divider) and 120 × 40 (large 8 × 4 vials on wooden
//! shelf ledges with each vial's label under the bargain).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::level::Theme;
use crate::game::shop::{
    self, HoleStock, MoltStep, OwnedKind, ShopItem, SpliceKind, MAX_SPLICES, MOLT_PRICE,
    ROW_COUNT, STITCH_PRICE,
};
use crate::game::world::World;
use crate::render::hud::{HEAL_MINT, TRAIL_GOLD, WARN_AMBER};
use crate::render::menu::{self, HitMap, Target};
use crate::render::palette::{to_color, Rgb, COAT, MUTED, TAIL};

/// The screen's own dialogue colour for the shopkeeper's voice / vial labels.
const VIAL: Color = Color::Rgb(196, 202, 214);
/// A sold or finished row.
const SPENT: Color = Color::Rgb(108, 104, 116);
/// Why a row cannot be bought (the disabled state's own words).
const REFUSED: Color = WARN_AMBER;
/// Warm wood ledge tint for the apothecary shelf rails.
const SHELF_WOOD: Color = Color::Rgb(118, 86, 56);
/// Shaded under-shelf / recessed den shadow.
const SHELF_DIM: Color = Color::Rgb(62, 48, 38);
/// Subtle recessed row highlight when selected.
const ROW_SEL_BG: Color = Color::Rgb(26, 32, 34);
/// Subtle warmer row highlight when pointer-hovered.
const ROW_HOV_BG: Color = Color::Rgb(34, 34, 38);
/// Pewter spoon ornament colour.
const PEWTER: Color = Color::Rgb(204, 210, 222);

const PLATE_RGB: Rgb = (17, 21, 22);
const SEL_RGB: Rgb = (26, 32, 34);
const HOV_RGB: Rgb = (34, 34, 38);

/// What the screen is showing while the Molt flow is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoltUi {
    /// Step one: nominate the kind to leave behind.
    Forfeit { cursor: usize },
    /// Step two: pick the replacement, at rank 1, in the same category.
    Pick {
        owned: OwnedKind,
        cursor: usize,
    },
}

impl MoltUi {
    pub fn step(self) -> MoltStep {
        match self {
            MoltUi::Forfeit { .. } => MoltStep::Forfeit,
            MoltUi::Pick { .. } => MoltStep::Pick,
        }
    }

    pub fn cursor(self) -> usize {
        match self {
            MoltUi::Forfeit { cursor } | MoltUi::Pick { cursor, .. } => cursor,
        }
    }

    pub fn with_cursor(self, cursor: usize) -> MoltUi {
        match self {
            MoltUi::Forfeit { .. } => MoltUi::Forfeit { cursor },
            MoltUi::Pick { owned, .. } => MoltUi::Pick { owned, cursor },
        }
    }
}

fn ink(c: Color) -> Style {
    Style::default().fg(c)
}

fn heavy(c: Color) -> Style {
    ink(c).add_modifier(Modifier::BOLD)
}

fn band(r: Rect, row: u16, height: u16) -> Rect {
    let row = row.min(r.height);
    Rect::new(r.x, r.y + row, r.width, height.min(r.height - row))
}

/// Put `text` on one row, clipped to `width` cells.
fn put(f: &mut Frame, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if width == 0 {
        return;
    }
    f.buffer_mut().set_stringn(x, y, text, width as usize, style);
}

/// Fill a band's background so selected/hovered cards read as tactile recessed bays.
fn fill_bg(buf: &mut Buffer, rect: Rect, bg: Color) {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            buf[(x, y)].set_bg(bg);
        }
    }
}

/// The material tint of the sector the run is on (the panel's accent).
fn accent(world: &World) -> Color {
    to_color(match world.level.theme {
        Theme::Sewer => (144, 184, 173),
        Theme::Kitchen => (213, 173, 122),
        Theme::Lab => (151, 175, 214),
        // Porcelain over indigo tile; newsprint with its registration cyan
        // (the title row's material tints for the two side-route floors).
        Theme::Laundry => (159, 200, 200),
        Theme::Printworks => (120, 178, 182),
        // Sun-bleached deck planks.
        Theme::Trailer => (214, 196, 150),
    })
}

/// Draw the shop over the frozen field.
///
/// `row` is the selected shelf row (`0..ROW_COUNT`), `molt` the open Molt step
/// (if any). Every actionable rectangle is registered in `hits` **only when it
/// can be activated**, so the pointer never hovers a dead row.
pub fn draw(
    f: &mut Frame,
    area: Rect,
    world: &World,
    hole: usize,
    row: usize,
    molt: Option<MoltUi>,
    hits: &mut HitMap,
) {
    let w = area.width.saturating_sub(2).clamp(24, 84);
    let roomy = w >= 76 && area.height >= 28;
    let has_dividers = area.height >= 22;
    let shelf_gap: u16 = if roomy { 1 } else { 0 };
    let card_h: u16 = if roomy { 4 } else { 3 };
    let extra_rows: u16 = (if has_dividers { 1 } else { 0 }) + shelf_gap * 3;
    let needed = 2 + 3 * card_h + extra_rows + 3 + 1 + 1;
    let h = area.height.saturating_sub(1).min(needed + 2).max(10);
    let r = menu::centered_rect(w, h, area);
    let inner = menu::panel(f, r, " ☌ THE BLIND SPOT  /  GENE SPLICE SHELF ", accent(world));
    if inner.width < 12 || inner.height < 6 {
        return;
    }

    let icon_w: u16 = if roomy { 8 } else { 4 };
    let text_x = inner.x + 2 + icon_w + 1;

    // ---- header: the purse, the sockets (text + visual capsules), the den ----
    let used = world.player.loadout.splices.len();
    let purse = format!("GOLD {}", world.gold);
    let sockets_txt = format!(" · SPLICE SOCKETS {used}/{MAX_SPLICES}");
    let mut hx = inner.x + 1;
    put(f, hx, inner.y, inner.right().saturating_sub(hx), &purse, heavy(TRAIL_GOLD));
    hx += purse.len() as u16;
    put(f, hx, inner.y, inner.right().saturating_sub(hx), &sockets_txt, ink(COAT));
    hx += sockets_txt.chars().count() as u16;

    // Visual splice socket capsules right beside the count: `◉` filled, `◎` open.
    if hx + 5 < inner.right() {
        hx += 1;
        for i in 0..MAX_SPLICES {
            let filled = i < used;
            let (sym, col) = if filled { ("◉", HEAL_MINT) } else { ("◎", SHELF_WOOD) };
            put(f, hx, inner.y, 1, sym, heavy(col));
            hx += 2;
        }
    }

    let stock = world.shop.get(hole);
    if inner.width >= 50 {
        let name = match world.level.objects.shop_holes.get(hole) {
            Some(h) => match h.role {
                crate::game::level::ShopHoleRole::Fork => "☌ UNDER THE SPOON",
                crate::game::level::ShopHoleRole::Approach => "☌ BY THE ARENA ROAD",
            },
            None => "",
        };
        let nlen = name.chars().count() as u16;
        let x = inner.right().saturating_sub(nlen + 1);
        if x > hx + 1 {
            put(f, x, inner.y, inner.right() - x, name, ink(PEWTER));
        }
    }

    // Apothecary top shelf rail under the header.
    if inner.width > 4 {
        draw_shelf_rule(
            f,
            inner.x + 1,
            inner.y + 1,
            inner.width - 2,
            if roomy { Some("SPECIMEN SHELF") } else { None },
            SHELF_WOOD,
        );
    }

    // ---- the specimen shelf (three stoppered vials) -------------------------
    let mut y = inner.y + 2;
    let services_reserve = 5 + if has_dividers { 1 } else { 0 };
    for slot in 0..shop::STOCK_SIZE {
        if y + card_h > inner.y + inner.height.saturating_sub(services_reserve) {
            break;
        }
        draw_slice(f, inner, &mut y, card_h, text_x, icon_w, world, hole, slot, row, hits);
        if shelf_gap > 0 && y < inner.y + inner.height.saturating_sub(services_reserve) {
            draw_vial_ledge(f, inner, y, icon_w);
            y += 1;
        }
    }

    // ---- wooden shelf lip separating specimens from back-room services ------
    if has_dividers && y < inner.y + inner.height.saturating_sub(5) {
        draw_shelf_rule(
            f,
            inner.x + 1,
            y,
            inner.width.saturating_sub(2),
            if inner.width >= 50 { Some("BACK-ROOM SERVICES") } else { None },
            SHELF_WOOD,
        );
        y += 1;
    }

    // ---- the services and the exit ------------------------------------------
    let services = [
        (
            ShopItem::Stitch,
            "STITCH",
            "+",
            HEAL_MINT,
            STITCH_PRICE,
            ("heal 35% max hp, once per hole", "heal 35%"),
        ),
        (
            ShopItem::Reroll,
            "TURN THE SHELF",
            "↻",
            PEWTER,
            0,
            ("replace the unsold stock, up to twice", "new stock"),
        ),
        (
            ShopItem::Molt,
            "MOLT",
            "◇",
            VIAL,
            MOLT_PRICE,
            ("leave one kind, take another at rank 1", "swap a kind"),
        ),
    ];
    for (item, label, tag, tag_col, price, blurb) in services {
        if y >= inner.y + inner.height.saturating_sub(2) {
            break;
        }
        let row_index = match item {
            ShopItem::Stitch => 3,
            ShopItem::Reroll => 4,
            _ => 5,
        };
        draw_service(
            f, inner, y, row_index, row, label, tag, tag_col, price, blurb, world, hole, stock,
            item, hits,
        );
        y += 1;
    }
    if y < inner.y + inner.height.saturating_sub(1) {
        draw_leave(f, inner, y, ROW_COUNT - 1, row, hits);
    }

    // ---- footer: what is already in the rat, and the keys (never overlapping) -
    let footer_row = inner.y + inner.height - 1;
    let mut footer = String::new();
    if world.player.loadout.splices.is_empty() {
        footer.push_str("no splices in this rat yet");
    } else {
        footer.push_str("in you: ");
        for (i, kind) in world.player.loadout.splices.iter().enumerate() {
            if i > 0 {
                footer.push_str(", ");
            }
            footer.push_str(kind.name());
        }
    }
    let keys = if inner.width >= 72 {
        Some("↑↓ select · Enter buy · M molt · Esc leave")
    } else if inner.width >= 58 {
        Some("↑↓ · Enter · M molt · Esc")
    } else {
        None
    };
    let max_footer = match keys {
        Some(k) => inner
            .width
            .saturating_sub(k.chars().count() as u16 + 4),
        None => inner.width.saturating_sub(2),
    };
    let footer_style = if world.player.loadout.splices.is_empty() {
        ink(MUTED)
    } else {
        ink(HEAL_MINT)
    };
    put(f, inner.x + 1, footer_row, max_footer, &footer, footer_style);
    if let Some(k) = keys {
        let klen = k.chars().count() as u16;
        let x = inner.right().saturating_sub(klen + 1);
        put(f, x, footer_row, inner.right() - x, k, ink(MUTED));
    }

    // ---- the Molt step, over everything ------------------------------------
    if let Some(molt) = molt {
        draw_molt(f, r, world, hole, molt, hits);
    }
}

/// Horizontal apothecary rule with an optional centered/left-aligned section tag.
fn draw_shelf_rule(f: &mut Frame, x: u16, y: u16, width: u16, title: Option<&str>, col: Color) {
    if width < 4 {
        return;
    }
    let rule: String = std::iter::repeat_n('─', width as usize).collect();
    put(f, x, y, width, &rule, ink(col));
    if let Some(label) = title {
        let tag = format!(" {label} ");
        let tlen = tag.chars().count() as u16;
        if width > tlen + 4 {
            put(f, x + 2, y, tlen, &tag, ink(VIAL));
        }
    }
}

/// Subtle wooden shelf plank sitting directly beneath a roomy specimen vial.
fn draw_vial_ledge(f: &mut Frame, inner: Rect, y: u16, icon_w: u16) {
    let ledge_w = icon_w + 2;
    let ledge: String = std::iter::repeat_n('▀', ledge_w as usize).collect();
    put(f, inner.x + 1, y, ledge_w, &ledge, ink(SHELF_WOOD));
    let rest_x = inner.x + 1 + ledge_w;
    let rest_w = inner.right().saturating_sub(rest_x + 1);
    if rest_w > 0 {
        let line: String = std::iter::repeat_n('·', rest_w as usize).collect();
        put(f, rest_x, y, rest_w, &line, ink(SHELF_DIM));
    }
}

/// One shelf slot's card with bespoke glass-vial pixel art.
#[allow(clippy::too_many_arguments)]
fn draw_slice(
    f: &mut Frame,
    inner: Rect,
    y: &mut u16,
    card_h: u16,
    text_x: u16,
    icon_w: u16,
    world: &World,
    hole: usize,
    slot: usize,
    row: usize,
    hits: &mut HitMap,
) {
    let selected = row == slot;
    let target = Target::ShopRow(slot);
    let hovered = hits.hovered == Some(target);
    let card = band(inner, y.saturating_sub(inner.y), card_h);
    hits.add(card, target);

    let bg_rgb = if hovered {
        fill_bg(f.buffer_mut(), card, ROW_HOV_BG);
        HOV_RGB
    } else if selected {
        fill_bg(f.buffer_mut(), card, ROW_SEL_BG);
        SEL_RGB
    } else {
        PLATE_RGB
    };

    if selected || hovered {
        let bar_col = if hovered { TAIL } else { TRAIL_GOLD };
        for dy in 0..card_h {
            f.buffer_mut()[(inner.x, *y + dy)]
                .set_char('▌')
                .set_fg(bar_col);
        }
    }
    let Some(stock) = world.shop.get(hole) else {
        return;
    };
    let Some(kind) = stock.slot(slot) else {
        put(f, text_x, *y, inner.right() - text_x, "— empty hook —", ink(SPENT));
        *y += card_h;
        return;
    };
    let sold = stock.sold.get(slot).copied().unwrap_or(false);
    let owned = world.player.loadout.splices.contains(&kind);
    let deny = shop::deny_reason(world, hole, ShopItem::Splice(slot));

    // Bespoke half-block specimen vial (4×2 compact or 8×4 roomy) with wooden shelf sill.
    let icon_h = (icon_w / 2).max(2);
    let icon_area = Rect::new(inner.x + 2, *y, icon_w, icon_h);
    draw_splice_vial(f.buffer_mut(), icon_area, kind, sold, bg_rgb);
    if card_h == 3 {
        // Compact shelf ledge directly beneath the 4×2 vial on the card's third row.
        put(f, inner.x + 2, *y + 2, icon_w, "▀▀▀▀", ink(SHELF_WOOD));
    }

    let name_style = if sold || owned {
        ink(SPENT)
    } else if selected || hovered {
        heavy(COAT)
    } else {
        ink(COAT)
    };
    let price = format!("{}g", kind.price());
    let price_w = price.len() as u16;
    let px = inner.right().saturating_sub(price_w + 1);
    let name_w = px.saturating_sub(text_x + 1);
    put(f, text_x, *y, name_w, kind.name(), name_style);
    let price_style = if sold || owned {
        ink(SPENT)
    } else if deny.is_some() {
        ink(MUTED)
    } else {
        heavy(TRAIL_GOLD)
    };
    put(f, px, *y, price_w, &price, price_style);

    // Both sides of the bargain, always, before anything is paid.
    if card_h >= 2 {
        let plus_col = if sold || owned { SPENT } else { HEAL_MINT };
        put(f, text_x, *y + 1, inner.right().saturating_sub(text_x + 1), kind.plus(), ink(plus_col));
    }
    if card_h >= 3 {
        let status = if sold {
            "SOLD"
        } else if owned {
            "IN YOU"
        } else if let Some(deny) = deny {
            deny.reason()
        } else {
            ""
        };
        let status_reserve = if status.is_empty() {
            1
        } else {
            status.len() as u16 + 2
        };
        let minus_col = if sold || owned { SPENT } else { WARN_AMBER };
        put(
            f,
            text_x,
            *y + 2,
            inner.right().saturating_sub(text_x + status_reserve),
            kind.minus(),
            ink(minus_col),
        );
        if !status.is_empty() {
            let sx = inner.right().saturating_sub(status.len() as u16 + 1);
            let c = if sold || owned { SPENT } else { REFUSED };
            put(f, sx, *y + 2, status.len() as u16, status, ink(c));
        }
    }
    if card_h >= 4 {
        let label = format!("“{}”", kind.label());
        let lbl_col = if sold || owned { SPENT } else { VIAL };
        put(f, text_x, *y + 3, inner.right().saturating_sub(text_x + 1), &label, ink(lbl_col));
    }
    *y += card_h;
}

/// One service row (Stitch / Turn the shelf / Molt).
#[allow(clippy::too_many_arguments)]
fn draw_service(
    f: &mut Frame,
    inner: Rect,
    y: u16,
    row_index: usize,
    row: usize,
    label: &str,
    tag: &str,
    tag_col: Color,
    price: u32,
    blurb: (&str, &str),
    world: &World,
    hole: usize,
    stock: Option<&HoleStock>,
    item: ShopItem,
    hits: &mut HitMap,
) {
    let selected = row == row_index;
    let target = Target::ShopRow(row_index);
    let hovered = hits.hovered == Some(target);
    let row_rect = band(inner, y.saturating_sub(inner.y), 1);
    hits.add(row_rect, target);
    if hovered {
        fill_bg(f.buffer_mut(), row_rect, ROW_HOV_BG);
    } else if selected {
        fill_bg(f.buffer_mut(), row_rect, ROW_SEL_BG);
    }
    if selected || hovered {
        f.buffer_mut()[(inner.x, y)]
            .set_char('▌')
            .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
    }
    let deny = shop::deny_reason(world, hole, item);
    let icon_col = if deny.is_some() { SPENT } else { tag_col };
    put(f, inner.x + 2, y, 1, tag, heavy(icon_col));
    let style = if deny.is_some() { ink(SPENT) } else { heavy(COAT) };
    put(f, inner.x + 4, y, 20, label, style);
    let price = match item {
        ShopItem::Reroll => stock.and_then(shop::reroll_price).map(|p| format!("{p}g")),
        _ => (price > 0).then(|| format!("{price}g")),
    };
    if let Some(price) = price {
        let p_style = if deny.is_some() { ink(SPENT) } else { heavy(TRAIL_GOLD) };
        put(f, inner.x + 24, y, 5, &price, p_style);
    }
    let room = inner.right().saturating_sub(inner.x + 30);
    let (long, short) = blurb;
    match deny {
        Some(deny) => put(f, inner.x + 30, y, room, deny.reason(), ink(REFUSED)),
        None => {
            let text = if room as usize >= long.len() { long } else { short };
            put(f, inner.x + 30, y, room, text, ink(MUTED));
        }
    }
}

/// The exit row.
fn draw_leave(f: &mut Frame, inner: Rect, y: u16, row_index: usize, row: usize, hits: &mut HitMap) {
    let selected = row == row_index;
    let target = Target::ShopRow(row_index);
    let hovered = hits.hovered == Some(target);
    let row_rect = band(inner, y.saturating_sub(inner.y), 1);
    hits.add(row_rect, target);
    hits.add(row_rect, Target::ShopLeave);
    if hovered {
        fill_bg(f.buffer_mut(), row_rect, ROW_HOV_BG);
    } else if selected {
        fill_bg(f.buffer_mut(), row_rect, ROW_SEL_BG);
    }
    if selected || hovered {
        f.buffer_mut()[(inner.x, y)]
            .set_char('▌')
            .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
    }
    put(f, inner.x + 2, y, 1, "◂", ink(PEWTER));
    put(f, inner.x + 4, y, 20, "LEAVE", heavy(COAT));
    put(
        f,
        inner.x + 30,
        y,
        inner.right().saturating_sub(inner.x + 30),
        "back to the wall (Esc)",
        ink(MUTED),
    );
}

/// The Molt panel: the two-step nomination, over the shelf.
fn draw_molt(f: &mut Frame, r: Rect, world: &World, hole: usize, molt: MoltUi, hits: &mut HitMap) {
    let inner = menu::panel(f, r, " ◇ MOLT  /  LEAVE THE OLD SKIN ", VIAL);
    if inner.width < 12 || inner.height < 5 {
        return;
    }
    let deny = shop::molt_deny(world, hole);
    let footer = inner.y + inner.height - 1;
    let hint_row = footer.saturating_sub(1);
    let list_bottom = hint_row;

    // Cancel is always live (it is free, at either step).
    hits.add(
        band(inner, hint_row.saturating_sub(inner.y), 2),
        Target::ShopCancel,
    );

    if inner.width > 4 && inner.height > 4 {
        draw_shelf_rule(f, inner.x + 1, inner.y + 1, inner.width - 2, None, SHELF_WOOD);
    }

    match molt {
        MoltUi::Forfeit { cursor } => {
            put(
                f,
                inner.x + 1,
                inner.y,
                inner.width - 1,
                "Step 1 of 2 — nominate the kind you leave behind",
                heavy(COAT),
            );
            let items = shop::molt_owned(world);
            let list_h = list_bottom.saturating_sub(inner.y + 2);
            let offset = (cursor + 1).saturating_sub(list_h as usize);
            for (y, (i, owned)) in (inner.y + 2..).zip(items.iter().enumerate().skip(offset)) {
                if y >= list_bottom {
                    break;
                }
                let selected = i == cursor;
                let target = Target::ShopMoltRow(i);
                let hovered = hits.hovered == Some(target);
                let row_rect = band(inner, y.saturating_sub(inner.y), 1);
                hits.add(row_rect, target);
                if hovered {
                    fill_bg(f.buffer_mut(), row_rect, ROW_HOV_BG);
                } else if selected {
                    fill_bg(f.buffer_mut(), row_rect, ROW_SEL_BG);
                }
                if selected || hovered {
                    f.buffer_mut()[(inner.x, y)]
                        .set_char('▌')
                        .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
                }
                let style = if selected || hovered { heavy(COAT) } else { ink(MUTED) };
                let mark = if selected { "›" } else { " " };
                put(
                    f,
                    inner.x + 1,
                    y,
                    inner.width - 2,
                    &format!("{mark} {}", shop::owned_label(world, *owned)),
                    style,
                );
            }
            let evolution = items
                .get(cursor)
                .is_some_and(|owned| shop::owned_label(world, *owned).contains("EVOLVED"));
            let (line, line_col) = if evolution {
                ("It forfeits its ranks and its evolution.", WARN_AMBER)
            } else {
                ("It forfeits its ranks.", MUTED)
            };
            put(f, inner.x + 1, hint_row, inner.width - 2, line, ink(line_col));
            put(
                f,
                inner.x + 1,
                footer,
                inner.width - 2,
                "Enter name it  ·  Esc cancel, free",
                ink(VIAL),
            );
        }
        MoltUi::Pick { owned, cursor } => {
            let label = shop::owned_label(world, owned);
            put(
                f,
                inner.x + 1,
                inner.y,
                inner.width - 1,
                &format!("Step 2 of 2 — take, instead of {label}"),
                heavy(COAT),
            );
            let options = shop::molt_options(world, owned);
            let y = inner.y + 2;
            if options.is_empty() {
                put(
                    f,
                    inner.x + 1,
                    y,
                    inner.width - 2,
                    "No unowned kind in that rack: nothing to take.",
                    ink(REFUSED),
                );
            }
            let list_h = list_bottom.saturating_sub(y);
            let offset = (cursor + 1).saturating_sub(list_h as usize);
            for (y, (i, kind)) in (y..).zip(options.iter().enumerate().skip(offset)) {
                if y >= list_bottom {
                    break;
                }
                let selected = i == cursor;
                let target = Target::ShopMoltRow(i);
                let hovered = hits.hovered == Some(target);
                let row_rect = band(inner, y.saturating_sub(inner.y), 1);
                hits.add(row_rect, target);
                if hovered {
                    fill_bg(f.buffer_mut(), row_rect, ROW_HOV_BG);
                } else if selected {
                    fill_bg(f.buffer_mut(), row_rect, ROW_SEL_BG);
                }
                if selected || hovered {
                    f.buffer_mut()[(inner.x, y)]
                        .set_char('▌')
                        .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
                }
                let style = if selected || hovered { heavy(COAT) } else { ink(MUTED) };
                let mark = if selected { "›" } else { " " };
                put(
                    f,
                    inner.x + 1,
                    y,
                    inner.width - 2,
                    &format!("{mark} {}", shop::new_label(*kind)),
                    style,
                );
            }
            let (line, line_col) = match deny {
                Some(deny) => (deny.reason().to_string(), REFUSED),
                None => (format!("{MOLT_PRICE}g for the change"), TRAIL_GOLD),
            };
            put(f, inner.x + 1, hint_row, inner.width - 2, &line, ink(line_col));
            put(
                f,
                inner.x + 1,
                footer,
                inner.width - 2,
                "Enter confirm  ·  Esc cancel, free",
                ink(VIAL),
            );
        }
    }
}

// ============================================================================
// BESPOKE SPECIMEN VIAL ART (10 Splices × Compact 4×4 px + Roomy 8×8 px)
// ============================================================================
//
// Each splice has its own stoppered glass specimen bottle rendered directly
// into half-block cells (`▀`, fg = top pixel, bg = bottom pixel) over the
// panel's current row background. Sold vials render as empty uncorked bottles.

/// Palette roles for one stoppered glass specimen vial.
#[derive(Clone, Copy)]
struct VialPalette {
    /// Cork or metallic stopper cap.
    cap: Rgb,
    /// Bright glass specular rim (top-left).
    glass_hi: Rgb,
    /// Shaded glass wall (lower-right).
    glass_lo: Rgb,
    /// Deep serum fluid at the bottom of the vial.
    fluid_lo: Rgb,
    /// Mid serum colour.
    fluid_mid: Rgb,
    /// Bright meniscus / highlight fluid.
    fluid_hi: Rgb,
    /// Suspended specimen / distinctive inclusion inside the vial.
    spec: Rgb,
}

fn vial_palette(kind: SpliceKind) -> VialPalette {
    match kind {
        // Glass Incisors: icy frosted glass with twin sharp pearly fangs.
        SpliceKind::GlassIncisors => VialPalette {
            cap: (158, 122, 86),
            glass_hi: (224, 242, 255),
            glass_lo: (96, 120, 142),
            fluid_lo: (66, 98, 128),
            fluid_mid: (132, 186, 224),
            fluid_hi: (218, 244, 255),
            spec: (255, 255, 252),
        },
        // Boiler Heart: copper-capped pressure flask glowing furnace-red/amber.
        SpliceKind::BoilerHeart => VialPalette {
            cap: (206, 130, 68),
            glass_hi: (248, 214, 178),
            glass_lo: (118, 66, 48),
            fluid_lo: (138, 30, 26),
            fluid_mid: (228, 76, 46),
            fluid_hi: (255, 188, 78),
            spec: (255, 246, 186),
        },
        // Hollow Bones: slender sky-azure tube with pale porous bone rings.
        SpliceKind::HollowBones => VialPalette {
            cap: (146, 114, 78),
            glass_hi: (214, 238, 255),
            glass_lo: (76, 108, 148),
            fluid_lo: (42, 88, 152),
            fluid_mid: (82, 158, 232),
            fluid_hi: (168, 222, 255),
            spec: (246, 242, 226),
        },
        // Double Lid: plum-violet eye-dropper vial with twin winking amber irises.
        SpliceKind::DoubleLid => VialPalette {
            cap: (118, 108, 138),
            glass_hi: (216, 198, 242),
            glass_lo: (86, 66, 116),
            fluid_lo: (56, 30, 96),
            fluid_mid: (126, 68, 186),
            fluid_hi: (188, 134, 242),
            spec: (255, 218, 86),
        },
        // Hoard Gland: round-bellied amber alchemical flask filled with gold flecks.
        SpliceKind::HoardGland => VialPalette {
            cap: (168, 126, 74),
            glass_hi: (248, 232, 182),
            glass_lo: (126, 94, 48),
            fluid_lo: (132, 84, 22),
            fluid_mid: (218, 156, 44),
            fluid_hi: (255, 218, 92),
            spec: (255, 252, 204),
        },
        // Copper Nerves: copper-coiled Leyden vial crackling with electric cyan sparks.
        SpliceKind::CopperNerves => VialPalette {
            cap: (218, 136, 74),
            glass_hi: (206, 242, 246),
            glass_lo: (126, 82, 52),
            fluid_lo: (28, 86, 108),
            fluid_mid: (64, 178, 206),
            fluid_hi: (164, 248, 255),
            spec: (246, 255, 255),
        },
        // Night Shift: midnight-indigo ampoule with glowing chartreuse stimulant crescent.
        SpliceKind::NightShift => VialPalette {
            cap: (92, 98, 128),
            glass_hi: (192, 208, 244),
            glass_lo: (66, 72, 108),
            fluid_lo: (32, 38, 92),
            fluid_mid: (68, 82, 168),
            fluid_hi: (196, 242, 98),
            spec: (255, 244, 134),
        },
        // Blackboard Belly: charcoal-slate ink bottle with chalk tally marks + XP glow.
        SpliceKind::BlackboardBelly => VialPalette {
            cap: (152, 118, 82),
            glass_hi: (198, 210, 214),
            glass_lo: (72, 82, 88),
            fluid_lo: (28, 36, 42),
            fluid_mid: (56, 74, 82),
            fluid_hi: (110, 228, 202),
            spec: (242, 246, 240),
        },
        // Ballast Sac: heavy squat jar layered with dense ochre ballast sand.
        SpliceKind::BallastSac => VialPalette {
            cap: (116, 124, 134),
            glass_hi: (224, 226, 218),
            glass_lo: (96, 98, 104),
            fluid_lo: (128, 94, 52),
            fluid_mid: (186, 146, 88),
            fluid_hi: (226, 198, 142),
            spec: (248, 232, 186),
        },
        // Long Fingers: tall crooked mint phial with pale grasping tendrils inside.
        SpliceKind::LongFingers => VialPalette {
            cap: (164, 128, 92),
            glass_hi: (210, 246, 228),
            glass_lo: (72, 116, 98),
            fluid_lo: (36, 104, 78),
            fluid_mid: (78, 186, 136),
            fluid_hi: (158, 246, 196),
            spec: (238, 224, 246),
        },
    }
}

/// 8×8 character grid templates per splice:
/// `.` bg, `c` cap/cork, `g` glass_hi, `G` glass_lo, `l` fluid_lo, `m` fluid_mid,
/// `h` fluid_hi, `s` specimen inclusion.
fn vial_grid_8x8(kind: SpliceKind, sold: bool) -> &'static [&'static str; 8] {
    if sold {
        return &[
            "........",
            "..g..G..",
            "..g..G..",
            ".g....G.",
            ".g....G.",
            ".g....G.",
            ".g.ll.G.",
            "..GGGG..",
        ];
    }
    match kind {
        // 1. Glass Incisors: tapered crystal vial with twin sharp white fangs.
        SpliceKind::GlassIncisors => &[
            "...cc...",
            "..g..G..",
            ".g.hh.G.",
            ".gsmmsG.",
            ".gslmsG.",
            ".gmlmmG.",
            ".gllllG.",
            "..GGGG..",
        ],
        // 2. Boiler Heart: copper-capped pressure flask with brass band & steam bubble.
        SpliceKind::BoilerHeart => &[
            "..cccc..",
            "..g..G..",
            ".g.hs.G.",
            "ghmmmmhG",
            "gccccccG",
            "gmmsmmmG",
            ".gllllG.",
            "..GGGG..",
        ],
        // 3. Hollow Bones: slender sky-azure tube with pale porous bone rings.
        SpliceKind::HollowBones => &[
            "...cc...",
            "..g..G..",
            "..ghhG..",
            "..gsgG..",
            "..gmmG..",
            "..gslG..",
            "..gllG..",
            "..GGGG..",
        ],
        // 4. Double Lid: bulbous plum dropper vial with twin amber winking slits.
        SpliceKind::DoubleLid => &[
            "...cc...",
            "...gG...",
            ".ghhhhG.",
            "gmsllsmG",
            "gmmmmmmG",
            "gmmllmmG",
            ".gllllG.",
            "..GGGG..",
        ],
        // 5. Hoard Gland: round-bellied amber alchemical flask glittering with gold.
        SpliceKind::HoardGland => &[
            "...cc...",
            "..g..G..",
            ".ghhhhG.",
            "gmmsmmmG",
            "gmsmmsmG",
            "gmlmslmG",
            ".gllllG.",
            "..GGGG..",
        ],
        // 6. Copper Nerves: copper-coiled Leyden vial crackling with cyan lightning.
        SpliceKind::CopperNerves => &[
            "..cccc..",
            "..g..G..",
            ".gcshcG.",
            ".gmshmG.",
            ".gscscG.",
            ".gmlsmG.",
            ".gllllG.",
            "..GGGG..",
        ],
        // 7. Night Shift: dark indigo ampoule with bright crescent stimulant meniscus.
        SpliceKind::NightShift => &[
            "...cc...",
            "..g..G..",
            ".g.hs.G.",
            ".ghhhmG.",
            ".gmmmsG.",
            ".gmlllG.",
            ".gllllG.",
            "..GGGG..",
        ],
        // 8. Blackboard Belly: squat slate inkwell bottle with chalk tally & XP base.
        SpliceKind::BlackboardBelly => &[
            "..cccc..",
            "..g..G..",
            "gmmmmmmG",
            "gmsmsmmG",
            "gmllslmG",
            "ghhhhhhG",
            "gllllllG",
            ".GGGGGG.",
        ],
        // 9. Ballast Sac: heavy wide-bottomed jar with dense layered ochre sand.
        SpliceKind::BallastSac => &[
            "..cccc..",
            "..g..G..",
            ".g....G.",
            "g.hhhh.G",
            "ghmsmmhG",
            "gmmmmmmG",
            "gllllllG",
            ".GGGGGG.",
        ],
        // 10. Long Fingers: tall crooked emerald phial with reaching pale tendrils.
        SpliceKind::LongFingers => &[
            "....cc..",
            "...g..G.",
            "..gshG..",
            ".g.sm.G.",
            ".gmsmmG.",
            ".gmlsmG.",
            ".gllllG.",
            "..GGGG..",
        ],
    }
}

/// Compact 4×4 pixel vial templates (rendered as 4×2 terminal cells for 57×20 & 80×24).
fn vial_grid_4x4(kind: SpliceKind, sold: bool) -> &'static [&'static str; 4] {
    if sold {
        return &[".gG.", "g..G", "g..G", ".GG."];
    }
    match kind {
        SpliceKind::GlassIncisors => &[".cc.", "gssG", "gmhG", ".ll."],
        SpliceKind::BoilerHeart => &["cccc", "ghsG", "gcCG", ".ll."],
        SpliceKind::HollowBones => &[".cc.", ".ghG", ".gsG", ".ll."],
        SpliceKind::DoubleLid => &[".cc.", "gsSG", "gmmG", ".ll."],
        SpliceKind::HoardGland => &[".cc.", "ghsG", "gsmG", ".ll."],
        SpliceKind::CopperNerves => &["cccc", "gshG", "gcmG", ".ll."],
        SpliceKind::NightShift => &[".cc.", "ghsG", "gmmG", ".ll."],
        SpliceKind::BlackboardBelly => &["cccc", "gsmG", "ghhG", ".ll."],
        SpliceKind::BallastSac => &["cccc", "g.hG", "gmmG", "llll"],
        SpliceKind::LongFingers => &["..cc", ".gsG", "gmmG", ".ll."],
    }
}

#[inline]
fn decode_vial_px(ch: u8, pal: &VialPalette, sold: bool, bg: Rgb) -> Rgb {
    if sold {
        return match ch {
            b'g' => (116, 122, 134),
            b'G' => (72, 76, 86),
            b'l' => (62, 66, 74),
            _ => bg,
        };
    }
    match ch {
        b'c' | b'C' => pal.cap,
        b'g' => pal.glass_hi,
        b'G' => pal.glass_lo,
        b'l' => pal.fluid_lo,
        b'm' => pal.fluid_mid,
        b'h' => pal.fluid_hi,
        b's' | b'S' => pal.spec,
        _ => bg,
    }
}

/// Blit a bespoke stoppered glass specimen vial into `area` using `▀` half-blocks.
fn draw_splice_vial(buf: &mut Buffer, area: Rect, kind: SpliceKind, sold: bool, bg: Rgb) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let pal = vial_palette(kind);
    if area.width >= 8 && area.height >= 4 {
        let grid = vial_grid_8x8(kind, sold);
        for cy in 0..4u16 {
            let top_row = grid[(cy * 2) as usize].as_bytes();
            let bot_row = grid[(cy * 2 + 1) as usize].as_bytes();
            for cx in 0..8u16 {
                let top = decode_vial_px(top_row[cx as usize], &pal, sold, bg);
                let bot = decode_vial_px(bot_row[cx as usize], &pal, sold, bg);
                buf[(area.x + cx, area.y + cy)]
                    .set_char('▀')
                    .set_fg(to_color(top))
                    .set_bg(to_color(bot));
            }
        }
    } else {
        let grid = vial_grid_4x4(kind, sold);
        for cy in 0..2u16 {
            let top_row = grid[(cy * 2) as usize].as_bytes();
            let bot_row = grid[(cy * 2 + 1) as usize].as_bytes();
            for cx in 0..4u16 {
                let top = decode_vial_px(top_row[cx as usize], &pal, sold, bg);
                let bot = decode_vial_px(bot_row[cx as usize], &pal, sold, bg);
                buf[(area.x + cx, area.y + cy)]
                    .set_char('▀')
                    .set_fg(to_color(top))
                    .set_bg(to_color(bot));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Showcase all 10 bespoke specimen vials (roomy 8×8 + compact 4×4 + sold empty).
    /// `RATRUN_DUMP=shop_vials cargo test dump_shop_vials_sheet`.
    #[test]
    fn dump_shop_vials_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("shop_vials") {
            return;
        }
        let w = 120usize;
        let h = 40usize;
        let mut px = vec![(17u8, 21u8, 22u8); w * h];
        // Row 1: all 10 roomy 8×8 vials + empty sold vial.
        for (i, &kind) in SpliceKind::ALL.iter().enumerate() {
            let pal = vial_palette(kind);
            let grid = vial_grid_8x8(kind, false);
            let ox = 4 + i * 10;
            for y in 0..8 {
                let row = grid[y].as_bytes();
                for x in 0..8 {
                    px[(4 + y) * w + (ox + x)] = decode_vial_px(row[x], &pal, false, PLATE_RGB);
                }
            }
            // Shelf wood plank under each vial.
            for x in 0..8 {
                px[12 * w + (ox + x)] = (118, 86, 56);
            }
            // Row 2: compact 4×4 vials.
            let g4 = vial_grid_4x4(kind, false);
            for y in 0..4 {
                let row = g4[y].as_bytes();
                for x in 0..4 {
                    px[(22 + y) * w + (ox + 2 + x)] = decode_vial_px(row[x], &pal, false, PLATE_RGB);
                }
            }
        }
        // Empty sold bottle on the far right.
        let pal = vial_palette(SpliceKind::GlassIncisors);
        let g_sold = vial_grid_8x8(SpliceKind::GlassIncisors, true);
        for y in 0..8 {
            let row = g_sold[y].as_bytes();
            for x in 0..8 {
                px[(4 + y) * w + (106 + x)] = decode_vial_px(row[x], &pal, true, PLATE_RGB);
            }
        }
        let mut out = format!("P6\n{} {}\n255\n", w, h).into_bytes();
        for (r, g, b) in px {
            out.extend_from_slice(&[r, g, b]);
        }
        let dir = std::env::var_os("RATRUN_DUMP_DIR")
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        std::fs::write(dir.join("ratrun_shop_vials.ppm"), out).unwrap();
    }
}
