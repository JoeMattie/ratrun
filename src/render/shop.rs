//! **The gene-splice shop screen** (`WAVE3_DESIGN.md` §6).
//!
//! A stamped-metal panel in the game's own style
//! ([`menu::panel`](crate::render::menu::panel)) drawn over the frozen play
//! field, showing:
//!
//! * the purse and how many of the **two splice sockets** are taken;
//! * the three shelf slots — icon, name, price, and **both** stat changes of
//!   the bargain (the gain and the cost) *before* payment;
//! * the three services (Stitch, Turn the shelf, Molt);
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
//! The layout is row-counted, not pixel-counted, and adapts to three cases the
//! app tests pin: 57 × 20 (compact cards: a 4 × 2 icon, three rows per card),
//! 80 × 24 (compact cards with the vial label under each bargain) and
//! 120 × 40 (large 8 × 4 icons and a four-row card with the label).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::art_icons;
use crate::game::level::Theme;
use crate::game::shop::{
    self, HoleStock, MoltStep, OwnedKind, ShopItem, MAX_SPLICES, MOLT_PRICE, ROW_COUNT,
    STITCH_PRICE,
};
use crate::game::world::World;
use crate::render::hud::{HEAL_MINT, TRAIL_GOLD, WARN_AMBER};
use crate::render::menu::{self, HitMap, Target};
use crate::render::palette::{to_color, COAT, MUTED, TAIL};

/// The screen's own dialogue colour for the shopkeeper's voice.
const VIAL: Color = Color::Rgb(196, 202, 214);
/// A sold or finished row.
const SPENT: Color = Color::Rgb(108, 104, 116);
/// Why a row cannot be bought (the disabled state's own words).
const REFUSED: Color = WARN_AMBER;

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

/// The material tint of the sector the run is on (the panel's accent).
fn accent(world: &World) -> Color {
    to_color(match world.level.theme {
        Theme::Sewer => (144, 184, 173),
        Theme::Kitchen => (213, 173, 122),
        Theme::Lab => (151, 175, 214),
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
    // The panel is only as tall as its rows: a card takes three rows (four with
    // the larger icon and the vial label), then the services, the exit and the
    // footer. Anything taller is empty plate.
    let roomy = w >= 76 && area.height >= 28;
    let card_h: u16 = if roomy { 4 } else { 3 };
    let needed = 2 + 3 * card_h + 3 + 1 + 1;
    let h = area.height.saturating_sub(1).min(needed + 2).max(10);
    let r = menu::centered_rect(w, h, area);
    let inner = menu::panel(f, r, " THE BLIND SPOT   /   GENE SPLICE SHELF ", accent(world));
    if inner.width < 12 || inner.height < 6 {
        return;
    }

    // Cards grow with the room: 3 rows compact, 4 rows (with the larger icon
    // and the vial label) when there is width for it.
    let card_h: u16 = if roomy { 4 } else { 3 };
    let icon_w: u16 = if roomy { 8 } else { 4 };
    let text_x = inner.x + 2 + icon_w + 1;

    // ---- header: the purse and the sockets -------------------------------
    let mut header = format!("GOLD {}", world.gold);
    let used = world.player.loadout.splices.len();
    header.push_str(&format!("   ·   SPLICE SOCKETS {used}/{MAX_SPLICES}"));
    put(f, inner.x + 1, inner.y, inner.width - 1, &header, heavy(TRAIL_GOLD));
    // A rule under the header, so the purse reads as the panel's own line.
    if inner.width > 4 {
        let rule: String = std::iter::repeat_n('─', (inner.width - 2) as usize).collect();
        put(f, inner.x + 1, inner.y + 1, inner.width - 1, &rule, ink(MUTED));
    }
    let stock = world.shop.get(hole);
    if inner.width >= 46 {
        let name = match world.level.objects.shop_holes.get(hole) {
            Some(h) => match h.role {
                crate::game::level::ShopHoleRole::Fork => "UNDER THE SPOON",
                crate::game::level::ShopHoleRole::Approach => "BY THE ARENA ROAD",
            },
            None => "",
        };
        let x = inner.right().saturating_sub(name.len() as u16 + 1);
        put(f, x, inner.y, inner.right() - x, name, ink(MUTED));
    }

    // ---- the shelf -------------------------------------------------------
    let mut y = inner.y + 2;
    for slot in 0..shop::STOCK_SIZE {
        if y + card_h > inner.y + inner.height.saturating_sub(4) {
            break;
        }
        draw_slice(f, inner, &mut y, card_h, text_x, icon_w, world, hole, slot, row, hits);
    }

    // ---- the services and the exit ---------------------------------------
    // Long blurb, then the short one a narrow (57-column) panel uses so the
    // line is never cut mid-word.
    let services = [
        (
            ShopItem::Stitch,
            "STITCH",
            STITCH_PRICE,
            ("heal 35% max hp, once per hole", "heal 35%"),
        ),
        (
            ShopItem::Reroll,
            "TURN THE SHELF",
            0,
            ("replace the unsold stock, up to twice", "new stock"),
        ),
        (
            ShopItem::Molt,
            "MOLT",
            MOLT_PRICE,
            ("leave one kind, take another at rank 1", "swap a kind"),
        ),
    ];
    for (item, label, price, blurb) in services {
        if y >= inner.y + inner.height.saturating_sub(2) {
            break;
        }
        let row_index = match item {
            ShopItem::Stitch => 3,
            ShopItem::Reroll => 4,
            _ => 5,
        };
        draw_service(
            f, inner, y, row_index, row, label, price, blurb, world, hole, stock, item, hits,
        );
        y += 1;
    }
    if y < inner.y + inner.height.saturating_sub(1) {
        draw_leave(f, inner, y, ROW_COUNT - 1, row, hits);
    }

    // ---- footer: what is already in the rat, and the keys ----------------
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
    put(f, inner.x + 1, footer_row, inner.width.saturating_sub(2), &footer, ink(VIAL));
    if inner.width >= 60 {
        let keys = "↑↓ select · Enter buy · M molt · Esc leave";
        let x = inner.right().saturating_sub(keys.len() as u16 + 1);
        put(f, x, footer_row, inner.right() - x, keys, ink(MUTED));
    }

    // ---- the Molt step, over everything ---------------------------------
    if let Some(molt) = molt {
        // The Molt step replaces the shelf outright (same plate), so the two
        // never read as one jumbled screen.
        draw_molt(f, r, world, hole, molt, hits);
    }
}

/// One shelf slot's card.
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
    if selected || hovered {
        f.buffer_mut()[(inner.x, *y)]
            .set_char('▌')
            .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
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

    // Icon, then the name and the price on the first line. The icon is 4×2
    // cells (its 4×4 px sprite) or 8×4 cells (its 8×8 px sprite).
    let icon_h = (icon_w / 2).max(2);
    let icon_area = Rect::new(inner.x + 2, *y, icon_w, icon_h);
    art_icons::draw(f.buffer_mut(), icon_area, art_icons::splice(kind));
    let name_style = if sold || owned {
        ink(SPENT)
    } else if selected {
        heavy(COAT)
    } else {
        ink(COAT)
    };
    put(f, text_x, *y, inner.right() - text_x, kind.name(), name_style);
    let price = format!("{}g", kind.price());
    let px = inner.right().saturating_sub(price.len() as u16 + 1);
    put(f, px, *y, price.len() as u16, &price, heavy(TRAIL_GOLD));

    // Both sides of the bargain, always, before anything is paid.
    if card_h >= 2 {
        put(f, text_x, *y + 1, inner.right() - text_x - 6, kind.plus(), ink(HEAL_MINT));
    }
    if card_h >= 3 {
        put(f, text_x, *y + 2, inner.right() - text_x - 12, kind.minus(), ink(WARN_AMBER));
        let status = if sold {
            "SOLD".to_string()
        } else if owned {
            "IN YOU".to_string()
        } else if let Some(deny) = deny {
            deny.reason().to_string()
        } else {
            String::new()
        };
        if !status.is_empty() {
            let sx = inner.right().saturating_sub(status.len() as u16 + 1);
            let c = if sold || owned { SPENT } else { REFUSED };
            put(f, sx, *y + 2, status.len() as u16, &status, ink(c));
        }
    }
    if card_h >= 4 {
        put(f, text_x, *y + 3, inner.right() - text_x, kind.label(), ink(VIAL));
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
    hits.add(band(inner, y.saturating_sub(inner.y), 1), target);
    if selected || hovered {
        f.buffer_mut()[(inner.x, y)]
            .set_char('▌')
            .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
    }
    let deny = shop::deny_reason(world, hole, item);
    let style = if deny.is_some() { ink(SPENT) } else { heavy(COAT) };
    put(f, inner.x + 2, y, 22, label, style);
    let price = match item {
        ShopItem::Reroll => stock.and_then(shop::reroll_price).map(|p| format!("{p}g")),
        _ => (price > 0).then(|| format!("{price}g")),
    };
    if let Some(price) = price {
        put(f, inner.x + 24, y, 5, &price, heavy(TRAIL_GOLD));
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
    hits.add(band(inner, y.saturating_sub(inner.y), 1), target);
    hits.add(band(inner, y.saturating_sub(inner.y), 1), Target::ShopLeave);
    if selected || hovered {
        f.buffer_mut()[(inner.x, y)]
            .set_char('▌')
            .set_fg(if hovered { TAIL } else { TRAIL_GOLD });
    }
    put(f, inner.x + 2, y, 22, "LEAVE", heavy(COAT));
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
///
/// The panel is as tall as the shelf it covers, so the two never look like one
/// jumbled screen: step one lists the owned kinds (rank and `✦EVOLVED` mark),
/// step two the replacements at rank 1, and the last two rows are always the
/// price (or the refusal) and the keys — including the free `Esc` cancel.
fn draw_molt(f: &mut Frame, r: Rect, world: &World, hole: usize, molt: MoltUi, hits: &mut HitMap) {
    let inner = menu::panel(f, r, " MOLT   /   LEAVE THE OLD SKIN ", VIAL);
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
                hits.add(band(inner, y.saturating_sub(inner.y), 1), Target::ShopMoltRow(i));
                let style = if selected { heavy(COAT) } else { ink(MUTED) };
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
            let line = if evolution {
                "It forfeits its ranks and its evolution."
            } else {
                "It forfeits its ranks."
            };
            put(f, inner.x + 1, hint_row, inner.width - 2, line, ink(MUTED));
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
                hits.add(band(inner, y.saturating_sub(inner.y), 1), Target::ShopMoltRow(i));
                let style = if selected { heavy(COAT) } else { ink(MUTED) };
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
            let line = match deny {
                Some(deny) => deny.reason().to_string(),
                None => format!("{MOLT_PRICE}g for the change"),
            };
            put(f, inner.x + 1, hint_row, inner.width - 2, &line, ink(MUTED));
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
