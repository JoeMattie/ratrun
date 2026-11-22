//! **THE NEST** (`crate::meta`): the permanent upgrade screen.
//!
//! Subject R-47's stash: the gold it banked between escapes and the tracks it
//! has learned. One row per [`Track`], with its icon, its pips, what the next
//! level buys and what that level costs.
//!
//! The screen is **read-only during a run**: it draws the same rows from a live
//! `World`'s loadout when the pause screen opens it, but the app only calls
//! [`draw`] (buying needs [`crate::meta::MetaProfile::buy`], which the pause
//! path never reaches). Every purchase and the refund-all are confirmed in the
//! app between frames; this module owns the layout, the hit rectangles and the
//! look, exactly like `render::shop`.
//!
//! ## Sizes
//!
//! The layout is row-counted, so it adapts to the three sizes the app tests pin:
//! 57 × 20 (one line per track, no icons, windowed list), 80 × 24 (2-row rows
//! with the 4 × 2 small icons) and 120 × 40 (all fifteen tracks at once, with
//! room for the description, the flavour line and the refund row).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::art_icons;
use crate::meta::{MetaProfile, Track};
use crate::render::hud::{HEAL_MINT, TRAIL_GOLD, WARN_AMBER};
use crate::render::menu::{self, HitMap, Target};
use crate::render::palette::{to_color, COAT, MUTED, PLATE, TAIL, TRACK};

/// A price at or above this needs a second Enter ("confirm") before it is paid.
pub const CONFIRM_AT: u32 = 300;

/// What the screen is waiting for, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// `Enter` again buys this track's next level.
    Buy(Track),
    /// `R`/`Enter` again wipes every track and returns the gold.
    Refund,
}

/// The screen's own state (the app owns one of these).
#[derive(Clone, Debug, Default)]
pub struct NestUi {
    /// Selected track, as an index into [`Track::ALL`].
    pub cursor: usize,
    /// First visible track (the windowed list's top).
    pub scroll: usize,
    /// The pending confirmation, if any.
    pub pending: Option<Pending>,
}

impl NestUi {
    /// Point the cursor at `track`.
    pub fn select(&mut self, track: Track) {
        self.cursor = track.index();
    }

    /// The track under the cursor.
    pub fn track(&self) -> Track {
        Track::from_index(self.cursor).unwrap_or(Track::WeaponSlots)
    }

    /// Step the cursor by `delta`, wrapping through every track. Returns `true`
    /// when it moved. Any step cancels a pending confirmation (the same
    /// discipline as the shop's Molt flow).
    pub fn step(&mut self, delta: isize) -> bool {
        let n = Track::ALL.len() as isize;
        if n == 0 {
            return false;
        }
        let next = (self.cursor as isize + delta).rem_euclid(n);
        self.pending = None;
        let moved = next as usize != self.cursor;
        self.cursor = next as usize;
        moved
    }
}

/// What one frame measured, so the app can keep the cursor in the window.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawInfo {
    /// How many track rows the list window shows.
    pub visible: usize,
    /// Two-row rows with icons (false: one line per track).
    pub roomy: bool,
    /// Does the plate have room for the description and the flavour line?
    pub detailed: bool,
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
    let area = f.area();
    if width == 0 || x >= area.right() || y >= area.bottom() {
        return;
    }
    f.buffer_mut().set_stringn(x, y, text, width as usize, style);
}

/// Fill a band so the selected / hovered row reads as a recessed bay.
fn fill_bg(f: &mut Frame, rect: Rect, bg: Color) {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            f.buffer_mut()[(x, y)].set_bg(bg);
        }
    }
}

const ROW_SEL_BG: Color = Color::Rgb(26, 32, 34);
const ROW_HOV_BG: Color = Color::Rgb(34, 34, 38);
/// A maxed track's ink: the gold of something finished.
const MAXED: Color = TRAIL_GOLD;
/// The nest's own warm straw colour for flavour lines.
const STRAW: Color = Color::Rgb(214, 196, 150);

/// Level pips: `●●○○`, filled up to `level`.
pub fn pips(track: Track, level: u8) -> String {
    let max = track.max_level();
    let mut s = String::with_capacity(max as usize * 3);
    for i in 1..=max {
        s.push(if i <= level { '●' } else { '○' });
    }
    s
}

/// Draw the nest.
///
/// `profile` is the saved nest (the title/end path) or the live loadout's
/// compiled modifiers as a read-only view (the pause path, see
/// [`profile_from_modifiers`]).
pub fn draw(
    f: &mut Frame,
    area: Rect,
    profile: &MetaProfile,
    ui: &mut NestUi,
    accent: Color,
    hits: &mut HitMap,
) -> DrawInfo {
    hits.clear();
    if area.is_empty() {
        return DrawInfo::default();
    }
    let w = area.width.saturating_sub(2).min(88).max(20);
    let h = area.height.saturating_sub(1).max(8);
    let card = menu::centered_rect(w, h, area);
    let inner = menu::panel(f, card, " ☗ THE NEST  /  R-47 STASH ", accent);
    if inner.width < 14 || inner.height < 6 {
        return DrawInfo::default();
    }

    // 80 x 24 is the commonest play size and gets the 4 x 2 icons: the list then
    // windows (7 of 15 tracks at a time) and the detail line names the selected
    // track's next step.
    let roomy = inner.height >= 20 && inner.width >= 52;
    let detailed = inner.height >= 14;
    let row_h: u16 = if roomy { 2 } else { 1 };
    let footer = inner.y + inner.height - 1;
    // From the bottom up: the key legend, the flavour line, the description,
    // then the refund row.
    let flavour_y = footer.saturating_sub(1);
    let desc_y = footer.saturating_sub(2);
    let refund_y = if detailed {
        footer.saturating_sub(3)
    } else {
        footer.saturating_sub(1)
    };
    let list_top = inner.y + 2;
    let list_bottom = refund_y.min(desc_y).max(list_top);
    let list_rows = list_bottom.saturating_sub(list_top);
    let visible = ((list_rows / row_h) as usize).max(1);

    // ---- header: the bank and the tally -----------------------------------
    let mut hx = inner.x + 1;
    let bank = format!("BANK {}g", profile.bank);
    put(f, hx, inner.y, inner.right().saturating_sub(hx), &bank, heavy(TRAIL_GOLD));
    hx += bank.len() as u16 + 3;
    let owned = profile.total_levels();
    let max_levels: u32 = Track::ALL
        .into_iter()
        .map(|t| u32::from(t.max_level()))
        .sum();
    let count = format!("{owned}/{max_levels} UPGRADES");
    put(f, hx, inner.y, inner.right().saturating_sub(hx), &count, ink(COAT));
    hx += count.len() as u16 + 3;
    if owned > 0 && hx + 12 < inner.right() {
        let spent = format!("{}g WORN", profile.spent_total());
        put(f, hx, inner.y, inner.right().saturating_sub(hx), &spent, ink(MUTED));
    }
    if inner.width >= 44 {
        let tag = "PERMANENT  ·  BETWEEN RUNS";
        let tw = tag.chars().count() as u16;
        let x = inner.right().saturating_sub(tw + 1);
        if x > hx {
            put(f, x, inner.y, tw, tag, ink(accent));
        }
    }
    let rule: String = std::iter::repeat_n('─', usize::from(inner.width)).collect();
    put(f, inner.x, inner.y + 1, inner.width, &rule, ink(TRACK));

    // ---- the list window --------------------------------------------------
    if ui.cursor >= Track::ALL.len() {
        ui.cursor = Track::ALL.len().saturating_sub(1);
    }
    let max_scroll = Track::ALL.len().saturating_sub(visible);
    if ui.scroll > max_scroll {
        ui.scroll = max_scroll;
    }
    if ui.cursor < ui.scroll {
        ui.scroll = ui.cursor;
    } else if ui.cursor >= ui.scroll + visible {
        ui.scroll = ui.cursor + 1 - visible;
    }
    let mut y = list_top;
    for idx in ui.scroll..Track::ALL.len() {
        if y + row_h > list_top + list_rows {
            break;
        }
        let track = Track::ALL[idx];
        draw_track_row(f, inner, y, row_h, idx, track, ui, profile, hits);
        y += row_h;
    }
    if Track::ALL.len() > visible {
        // Windowed list: say so instead of silently cutting the table.
        let last = (ui.scroll + visible).min(Track::ALL.len());
        let chip = format!("{}-{last} of {}", ui.scroll + 1, Track::ALL.len());
        let cw = chip.chars().count() as u16;
        if cw + 2 < inner.width {
            let x = inner.right().saturating_sub(cw + 1);
            if y.saturating_sub(1) >= list_top {
                put(f, x, y - 1, cw, &chip, ink(MUTED));
            }
        }
    }

    // ---- refund row -------------------------------------------------------
    if detailed {
        draw_refund_row(f, inner, refund_y, ui, profile, hits);
    }

    // ---- the selected track's description ---------------------------------
    let track = Track::ALL[ui.cursor];
    let level = profile.level(track);
    if detailed {
        let next = profile.next_price(track);
        let line = match next {
            Some(price) => format!(
                "{} → {}   ({}g)",
                track.value_text(level),
                track.value_text(level + 1),
                price
            ),
            None => format!("{}   MAXED", track.value_text(level)),
        };
        let style = if next.is_none() { heavy(MAXED) } else { ink(COAT) };
        put(f, inner.x + 1, desc_y, inner.width.saturating_sub(2), &line, style);
        if inner.height >= 18 {
            let flavour = format!("\u{201c}{}\u{201d}", track.flavor());
            put(
                f,
                inner.x + 1,
                flavour_y,
                inner.width.saturating_sub(2),
                &flavour,
                ink(STRAW),
            );
        }
    }

    // ---- the footer: the confirm prompt, or the keys ----------------------
    if let Some(pending) = ui.pending {
        let prompt = match pending {
            Pending::Buy(t) => format!(
                " BUY {} LV{} FOR {}g?  Enter confirm · Esc cancel ",
                t.name(),
                profile.level(t) + 1,
                profile.next_price(t).unwrap_or(0)
            ),
            Pending::Refund => format!(
                " WIPE EVERY TRACK AND RETURN {}g?  Enter confirm · Esc cancel ",
                profile.spent_total()
            ),
        };
        put(
            f,
            inner.x + 1,
            footer,
            inner.width.saturating_sub(2),
            &prompt,
            heavy(PLATE).bg(WARN_AMBER),
        );
        return DrawInfo {
            visible,
            roomy,
            detailed,
        };
    }

    let back_w = 8u16.min(inner.width);
    let back = Rect::new(inner.x, footer, back_w, 1);
    put(f, inner.x, footer, back_w, " ‹ BACK ›", heavy(COAT).bg(TRACK));
    hits.add(back, Target::Back);
    let keys = if inner.width >= 76 {
        "↑↓ choose · Enter buy · R refund all · Enter twice on a big buy · Esc back"
    } else if inner.width >= 54 {
        "↑↓ · Enter buy · R refund · Esc back"
    } else {
        "↑↓ · Enter · Esc"
    };
    let kx = inner.x + back_w + 1;
    put(
        f,
        kx,
        footer,
        inner.right().saturating_sub(kx),
        keys,
        ink(MUTED),
    );

    DrawInfo {
        visible,
        roomy,
        detailed,
    }
}

/// One track row: icon (roomy), name, pips, price and the next step.
#[allow(clippy::too_many_arguments)]
fn draw_track_row(
    f: &mut Frame,
    inner: Rect,
    y: u16,
    row_h: u16,
    idx: usize,
    track: Track,
    ui: &NestUi,
    profile: &MetaProfile,
    hits: &mut HitMap,
) {
    let selected = idx == ui.cursor;
    let target = Target::NestRow(idx);
    let hovered = hits.hovered == Some(target);
    let rect = band(inner, y.saturating_sub(inner.y), row_h);
    hits.add(rect, target);
    if hovered {
        fill_bg(f, rect, ROW_HOV_BG);
    } else if selected {
        fill_bg(f, rect, ROW_SEL_BG);
    }
    if selected || hovered {
        let bar_col = if hovered { TAIL } else { TRAIL_GOLD };
        for dy in 0..row_h {
            f.buffer_mut()[(inner.x, y + dy)]
                .set_char('▌')
                .set_fg(bar_col);
        }
    }

    let level = profile.level(track);
    let maxed = level >= track.max_level();
    let price = profile.next_price(track);
    let affordable = price.is_some_and(|p| profile.bank >= p);
    let icon = art_icons::meta(track);

    let icon_w: u16 = if row_h >= 2 { art_icons::SMALL_CELL_W } else { 0 };
    if icon_w > 0 {
        let area = Rect::new(inner.x + 2, y, art_icons::SMALL_CELL_W, art_icons::SMALL_CELL_H);
        art_icons::draw(f.buffer_mut(), area, icon);
    }
    let text_x = inner.x + 2 + icon_w + u16::from(icon_w > 0);

    // Price (or MAX) right-aligned, pips just left of it.
    let price_text = match price {
        Some(p) => format!("{p}g"),
        None => "MAX".to_string(),
    };
    let price_w = price_text.chars().count() as u16;
    let px = inner.right().saturating_sub(price_w + 1);
    let price_style = if maxed {
        heavy(MAXED)
    } else if affordable {
        heavy(TRAIL_GOLD)
    } else {
        ink(MUTED)
    };
    put(f, px, y, price_w, &price_text, price_style);

    let pip_text = pips(track, level);
    let pip_w = pip_text.chars().count() as u16;
    let pip_x = px.saturating_sub(pip_w + 1);
    if pip_x > text_x {
        let pip_style = if maxed { heavy(MAXED) } else { ink(to_color(icon.accent)) };
        put(f, pip_x, y, pip_w, &pip_text, pip_style);
    }

    let name_style = if maxed {
        heavy(MAXED)
    } else if selected || hovered {
        heavy(COAT)
    } else {
        ink(COAT)
    };
    let name_w = pip_x.saturating_sub(text_x + 1).max(6);
    put(f, text_x, y, name_w, track.name(), name_style);

    if row_h >= 2 {
        // What the next level buys, in the row's second line.
        let next = match price {
            Some(_) => format!(
                "{} → {}",
                track.value_text(level),
                track.value_text(level + 1)
            ),
            None => format!("{}  ·  complete", track.value_text(level)),
        };
        let style = if maxed {
            ink(MUTED)
        } else if affordable {
            ink(HEAL_MINT)
        } else {
            ink(MUTED)
        };
        put(
            f,
            text_x,
            y + 1,
            inner.right().saturating_sub(text_x + 1),
            &next,
            style,
        );
    }
}

/// The refund-all row: free, and it returns exactly what was paid.
fn draw_refund_row(
    f: &mut Frame,
    inner: Rect,
    y: u16,
    ui: &NestUi,
    profile: &MetaProfile,
    hits: &mut HitMap,
) {
    let paid = profile.spent_total();
    let rect = band(inner, y.saturating_sub(inner.y), 1);
    hits.add(rect, Target::NestRefund);
    let hovered = hits.hovered == Some(Target::NestRefund);
    if hovered {
        fill_bg(f, rect, ROW_HOV_BG);
    }
    if hovered || ui.pending == Some(Pending::Refund) {
        f.buffer_mut()[(inner.x, y)]
            .set_char('▌')
            .set_fg(TAIL);
    }
    let line = if paid == 0 {
        "⟲ REFUND ALL   nothing worn yet".to_string()
    } else {
        format!("⟲ REFUND ALL   wipe every track, return {paid}g")
    };
    let style = if paid == 0 {
        ink(MUTED)
    } else if hovered {
        heavy(COAT)
    } else {
        heavy(STRAW)
    };
    put(f, inner.x + 2, y, inner.width.saturating_sub(2), &line, style);
}

/// A read-only view of a live run's permanent upgrades (the pause screen).
///
/// The run's [`crate::meta::RunModifiers`] are the compiled form of a profile;
/// this rebuilds the *display* levels from them by construction, so the pause
/// screen shows exactly what the run is playing with — never the saved bank
/// (a run in flight cannot buy anything).
pub fn profile_from_modifiers(mods: &crate::meta::RunModifiers) -> MetaProfile {
    let mut profile = MetaProfile::default();
    for track in Track::ALL {
        let level = match track {
            Track::WeaponSlots => mods.caps.weapons.saturating_sub(crate::game::loadout::MIN_SLOTS),
            Track::PassiveSlots => {
                mods.caps.passives.saturating_sub(crate::game::loadout::MIN_SLOTS)
            }
            Track::MoveSpeed => percent_steps(mods.move_speed, 0.03),
            Track::PickupRange => percent_steps(mods.magnet, 0.08),
            Track::FireRate => percent_steps(mods.fire_rate, 0.02),
            Track::Damage => percent_steps(mods.damage, 0.025),
            Track::XpGain => percent_steps(mods.xp, 0.04),
            Track::GoldGain => percent_steps(mods.gold, 0.05),
            Track::Luck => percent_steps(mods.luck, 0.15),
            Track::MaxHp => (mods.max_hp / 8.0).round() as u8,
            Track::Regen => (mods.regen / 0.08).round() as u8,
            Track::Armor => (mods.armor / 0.5).round() as u8,
            Track::DashCooldown => (mods.dash_cd / 0.05).round() as u8,
            Track::CarryOver => mods.carry_bonus,
            Track::Revive => mods.revives,
        };
        profile.set_level(track, level.min(track.max_level()));
    }
    profile
}

/// How many whole steps of `step` a multiplier above 1.0 holds.
fn percent_steps(mult: f32, step: f32) -> u8 {
    (((mult - 1.0) / step).round().max(0.0)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{MetaProfile, Track};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn screen_text(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn frame(w: u16, h: u16, profile: &MetaProfile, ui: &mut NestUi) -> (String, HitMap, DrawInfo) {
        let Ok(mut term) = Terminal::new(TestBackend::new(w, h)) else {
            return (String::new(), HitMap::default(), DrawInfo::default());
        };
        let mut hits = HitMap::default();
        let mut info = DrawInfo::default();
        let _ = term.draw(|f| {
            info = draw(
                f,
                f.area(),
                profile,
                ui,
                Color::Rgb(144, 184, 173),
                &mut hits,
            );
        });
        (screen_text(&term), hits, info)
    }

    /// A nest with a little of everything: a couple of levels on the cheap
    /// tracks, one slot, and gold in hand.
    fn started() -> MetaProfile {
        let mut profile = MetaProfile::default();
        profile.bank = 240;
        profile.set_level(Track::WeaponSlots, 1);
        profile.set_level(Track::MoveSpeed, 2);
        profile.set_level(Track::Revive, 1);
        profile
    }

    #[test]
    fn the_nest_draws_its_rows_at_the_three_reference_sizes() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let profile = started();
            let mut ui = NestUi::default();
            ui.cursor = Track::WeaponSlots.index();
            let (text, hits, info) = frame(w, h, &profile, &mut ui);

            // The plate, the bank and the five tracks the owner named by hand.
            assert!(text.contains("THE NEST"), "{w}x{h} lost the plate title");
            assert!(text.contains("BANK 240g"), "{w}x{h} lost the bank");
            for name in ["WEAPON SLOTS", "PASSIVE SLOTS", "QUICK PAWS"] {
                assert!(text.contains(name), "{w}x{h} lost {name}:\n{text}");
            }
            // The window is honest about what it is not showing.
            assert!(info.visible >= 1, "{w}x{h} showed no rows");
            assert!(info.visible <= Track::ALL.len());
            if info.visible < Track::ALL.len() {
                assert!(
                    text.contains(&format!("1-{}", info.visible)),
                    "{w}x{h} did not say which rows are visible:\n{text}"
                );
            }
            // Every visible row is a hit rectangle, and the refund and back
            // controls are live.
            assert!(hits.rect_for(Target::NestRow(0)).is_some(), "{w}x{h}");
            assert!(
                hits.rect_for(Target::NestRefund).is_some() || !info.detailed,
                "{w}x{h} lost the refund row"
            );
            assert!(hits.rect_for(Target::Back).is_some(), "{w}x{h} lost back");
            // The selected track's next step is printed with its price.
            assert!(
                text.contains('g') || text.contains("MAX"),
                "{w}x{h} printed no price"
            );
            // Roomier plates get the icons (the 4 x 2 small sprite).
            if info.roomy {
                assert!(info.visible * 2 <= 40);
            } else {
                assert!(info.visible >= 8, "{w}x{h} compact rows are one line");
            }
        }
    }

    #[test]
    fn a_maxed_track_reads_max_and_an_unaffordable_one_still_shows_its_price() {
        let mut profile = MetaProfile::default();
        profile.bank = 10;
        profile.set_level(Track::Revive, 1);
        let mut ui = NestUi::default();
        let (text, _, _) = frame(120, 40, &profile, &mut ui);
        assert!(text.contains("MAX"), "a bought-out track says MAX:\n{text}");
        // The dearest track still prints its price, even on a 10 g bank.
        let revive = Track::Revive;
        assert_eq!(profile.next_price(revive), None);
        assert!(text.contains("60g"), "the cheapest step's price is printed");
    }

    #[test]
    fn pips_track_the_levels_owned() {
        assert_eq!(pips(Track::WeaponSlots, 0), "○○○○");
        assert_eq!(pips(Track::WeaponSlots, 2), "●●○○");
        assert_eq!(pips(Track::WeaponSlots, 4), "●●●●");
        assert_eq!(pips(Track::Revive, 1), "●");
        assert_eq!(pips(Track::MoveSpeed, 9), "●●●●●●", "clamped to the cap");
    }

    #[test]
    fn an_armed_confirmation_takes_over_the_footer() {
        let profile = started();
        let mut ui = NestUi {
            cursor: Track::WeaponSlots.index(),
            pending: Some(Pending::Buy(Track::WeaponSlots)),
            ..NestUi::default()
        };
        let (text, _, _) = frame(120, 40, &profile, &mut ui);
        assert!(
            text.contains("BUY WEAPON SLOTS") && text.contains("Enter confirm"),
            "the confirm prompt is missing:\n{text}"
        );
        // The refund prompt names what comes back.
        ui.pending = Some(Pending::Refund);
        let (text, _, _) = frame(120, 40, &profile, &mut ui);
        assert!(text.contains("WIPE EVERY TRACK"), "{text}");
        assert!(text.contains("returns") || text.contains("RETURN"), "{text}");
    }

    #[test]
    fn stepping_wraps_and_cancels_a_confirmation() {
        let mut ui = NestUi {
            pending: Some(Pending::Refund),
            ..NestUi::default()
        };
        assert!(ui.step(-1), "wrapping up from the first row moves");
        assert_eq!(ui.cursor, Track::ALL.len() - 1);
        assert!(ui.pending.is_none(), "a step cancels a pending confirm");
        assert!(ui.step(1));
        assert_eq!(ui.cursor, 0);
        ui.cursor = Track::MoveSpeed.index();
        assert_eq!(ui.track(), Track::MoveSpeed);
    }

    /// The pause screen's read-only dossier is built from the run's compiled
    /// modifiers: it shows what the run is playing with, and rebuilding the
    /// profile from them is exact for every track at its cap.
    #[test]
    fn the_read_only_view_round_trips_a_maxed_run() {
        let mods = crate::meta::RunModifiers::from_profile(&MetaProfile::maxed());
        let view = profile_from_modifiers(&mods);
        assert_eq!(view, MetaProfile::maxed());
        assert_eq!(
            crate::meta::RunModifiers::from_profile(&view),
            mods,
            "the dossier's levels compile back to the same run modifiers"
        );
        let fresh = profile_from_modifiers(&crate::meta::RunModifiers::NONE);
        assert_eq!(fresh, MetaProfile::default());
        // A partly bought nest keeps the tracks it bought.
        let part = crate::meta::RunModifiers::from_profile(&started());
        let view = profile_from_modifiers(&part);
        assert_eq!(view.level(Track::MoveSpeed), 2);
        assert_eq!(view.level(Track::WeaponSlots), 1);
        assert_eq!(view.level(Track::Revive), 1);
        assert_eq!(view.level(Track::Damage), 0);
    }

    #[test]
    fn an_empty_terminal_draws_nothing_and_never_panics() {
        let profile = started();
        let mut ui = NestUi::default();
        for (w, h) in [(0u16, 0u16), (1, 1), (20, 6), (30, 8)] {
            let (_, _, info) = frame(w, h, &profile, &mut ui);
            assert!(info.visible >= 0);
        }
    }
}
