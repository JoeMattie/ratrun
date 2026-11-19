//! Heads-up display: top status line + bottom HP / XP bars.

use std::fmt::Write as _;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::world::World;
use crate::render::palette::{self, COAT, MUTED, PLATE, TAIL, TRACK};

/// Reused text buffers for the HUD.
///
/// `App` keeps one of these and hands it to [`draw_top`] / [`draw_bottom`] every
/// frame: each number is written into a buffer that keeps its capacity instead
/// of being formatted into a fresh string.
#[derive(Default)]
pub struct HudState {
    time: String,
    level: String,
    kills: String,
    score: String,
    /// `(tag, level)` text per weapon slot, reused across frames.
    weapon_tags: Vec<(String, String)>,
    hp: String,
    dash: String,
}

/// Clear a retained buffer and refill it, so digits from a longer previous
/// value can never survive into this frame's text.
fn refill(buf: &mut String, fill: impl FnOnce(&mut String)) {
    buf.clear();
    fill(buf);
}

fn fmt_time_into(dst: &mut String, secs: f32) {
    let s = secs.max(0.0) as u32;
    let _ = write!(dst, "{:02}:{:02}", s / 60, s % 60);
}

/// Short weapon tag for the HUD: up to four characters, counted in chars so a
/// non-ASCII name can never slice mid-codepoint (the old `&name[..4]` could
/// panic on one).
fn push_short_name(dst: &mut String, name: &str) {
    dst.extend(name.chars().take(4));
}

pub fn draw_top(frame: &mut Frame, area: Rect, world: &World, state: &mut HudState) {
    let accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    let urgent = world.time_left() <= 30.0;

    // Numbers → the retained buffers (no allocation while the capacity lasts).
    refill(&mut state.time, |b| fmt_time_into(b, world.time_left()));
    refill(&mut state.level, |b| {
        let _ = write!(b, "{}", world.player.level);
    });
    refill(&mut state.kills, |b| {
        let _ = write!(b, "{}", world.kills);
    });
    refill(&mut state.score, |b| {
        let _ = write!(b, "{}", world.score);
    });

    // Weapon tags: one reused (tag, level) pair per slot.
    let weapons = &world.player.loadout.weapons;
    while state.weapon_tags.len() < weapons.len() {
        state.weapon_tags.push((String::new(), String::new()));
    }
    state.weapon_tags.truncate(weapons.len());
    for (i, w) in weapons.iter().enumerate() {
        let (tag, lvl) = &mut state.weapon_tags[i];
        tag.clear();
        tag.push(' ');
        push_short_name(tag, w.kind.name());
        lvl.clear();
        let _ = write!(lvl, "{}", w.level);
    }

    if area.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(PLATE));
    let mut x = area.x;
    let mut put = |s: &str, style: Style| {
        (x, _) = buf.set_stringn(x, area.y, s, area.right().saturating_sub(x) as usize, style);
    };
    let value = Style::default().fg(COAT).add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(MUTED);
    let compact = area.width < 67 && weapons.len() > 3;
    put(
        if compact { "T " } else { " T " },
        Style::default()
            .bg(if urgent { TAIL } else { accent })
            .fg(PLATE),
    );
    put(&state.time, value);
    put(if compact { " Lv" } else { "  Lv" }, muted);
    put(&state.level, value);
    put(if compact { " KO" } else { "  KO " }, muted);
    put(&state.kills, value);
    put(if compact { " Score " } else { "  Score " }, muted);
    put(&state.score, value);
    put(if compact { " │" } else { "  │" }, muted);
    for ((tag, lvl), weapon) in state.weapon_tags.iter().zip(weapons) {
        let style = Style::default().fg(palette::to_color(weapon.kind.color()));
        // Two letters distinguish Spore from Squeak even in a full loadout.
        let end = if compact {
            tag.char_indices().nth(3).map_or(tag.len(), |(i, _)| i)
        } else {
            tag.len()
        };
        put(&tag[..end], style);
        put(lvl, muted);
    }
    if urgent && area.right().saturating_sub(x) >= 15 {
        buf.set_string(
            area.right() - 13,
            area.y,
            "FINAL SECONDS",
            Style::default().fg(TAIL),
        );
    }
}

pub fn draw_bottom(frame: &mut Frame, area: Rect, world: &World, state: &mut HudState) {
    if area.is_empty() {
        return;
    }
    let hp = (world.player.hp.max(0.0) / world.player.max_hp()).clamp(0.0, 1.0);
    let hp_color = if hp <= 0.25 {
        Color::Rgb(252, 122, 125)
    } else {
        TAIL
    };
    refill(&mut state.hp, |b| {
        let _ = write!(
            b,
            "♥ {:.0}/{:.0}",
            world.player.hp.max(0.0),
            world.player.max_hp()
        );
    });
    let xp = (world.player.xp as f32 / world.player.xp_to_next.max(1) as f32).clamp(0.0, 1.0);
    refill(&mut state.dash, |b| {
        if world.player.dash_cd <= 0.0 {
            b.push_str("DASH READY");
        } else {
            let _ = write!(b, "dash {:.1}s", world.player.dash_cd);
        }
    });
    let dash_width = (state.dash.len() as u16 + 1).min(area.width);
    let meters = area.width - dash_width;
    let left = (meters / 2 + 3).min(meters);
    meter(
        frame,
        Rect::new(area.x, area.y, left, 1),
        &state.hp,
        hp,
        hp_color,
    );
    meter(
        frame,
        Rect::new(area.x + left, area.y, meters - left, 1),
        "XP",
        xp,
        palette::to_color(palette::mix(
            world.level.palette.accent,
            (176, 184, 180),
            0.5,
        )),
    );
    let dash = Rect::new(area.right() - dash_width, area.y, dash_width, 1);
    let buf = frame.buffer_mut();
    buf.set_style(dash, Style::default().bg(PLATE));
    buf.set_stringn(
        dash.x,
        dash.y,
        &state.dash,
        dash_width as usize,
        Style::default().fg(if world.player.dash_cd <= 0.0 {
            COAT
        } else {
            MUTED
        }),
    );
}

/// Labels occupy their own dark strip so their contrast is independent of fill.
fn meter(frame: &mut Frame, r: Rect, label: &str, ratio: f32, color: Color) {
    const STEPS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    if r.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(r, Style::default().bg(PLATE));
    let (end, _) = buf.set_stringn(r.x, r.y, label, r.width as usize, Style::default().fg(COAT));
    let start = end.saturating_add(1).min(r.right());
    let width = r
        .right()
        .saturating_sub(start + u16::from(start < r.right()));
    let units = (ratio * f32::from(width) * 8.0).round() as usize;
    for i in 0..width {
        let step = units.saturating_sub(8 * i as usize).min(8);
        buf[(start + i, r.y)]
            .set_char(STEPS[step])
            .set_fg(color)
            .set_bg(TRACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::weapon::WeaponKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn push_short_name_is_char_safe() {
        let mut s = String::new();
        push_short_name(&mut s, "Gnaw");
        assert_eq!(s, "Gnaw");

        s.clear();
        push_short_name(&mut s, "Teeth");
        assert_eq!(s, "Teet");

        s.clear();
        push_short_name(&mut s, "");
        assert_eq!(s, "");

        // Multi-byte glyph: a byte slice at index 4 would have panicked.
        s.clear();
        push_short_name(&mut s, "🗡️ knife");
        assert_eq!(s.chars().count(), 4);
        assert!(s.starts_with('🗡'));
    }

    /// A world rigged with known values so the HUD text is predictable.
    fn known_world() -> World {
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = 65.0; // 300s run → 235s left
        w.player.level = 3;
        w.kills = 42;
        w.score = 1234;
        w.player.loadout.weapons[0].level = 2;
        w.player.hp = 100.0;
        w.player.dash_cd = 0.0;
        w
    }

    /// Independent full-width reference for the retained-buffer renderer.
    fn reference_top(world: &World) -> String {
        let s = world.time_left().max(0.0) as u32;
        let time = format!("{:02}:{:02}", s / 60, s % 60);
        let mut out = String::new();
        out.push_str(" T ");
        out.push_str(&time);
        out.push_str("  Lv");
        out.push_str(&format!("{}", world.player.level));
        out.push_str("  KO ");
        out.push_str(&format!("{}", world.kills));
        out.push_str("  Score ");
        out.push_str(&format!("{}", world.score));
        out.push_str("  │");
        for w in &world.player.loadout.weapons {
            out.push(' ');
            let name = w.kind.name();
            out.push_str(&name[..name.len().min(4)]);
            out.push_str(&format!("{}", w.level));
        }
        out
    }

    fn reference_bottom(world: &World) -> (String, String) {
        let hp = format!(
            "♥ {:.0}/{:.0}",
            world.player.hp.max(0.0),
            world.player.max_hp()
        );
        let dash = if world.player.dash_cd <= 0.0 {
            "DASH READY".to_string()
        } else {
            format!("dash {:.1}s", world.player.dash_cd)
        };
        (hp, dash)
    }

    fn row_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Check labels, spacing and values on a full-width status strip.
    #[test]
    fn top_line_text_matches_fixed_expected_string() {
        let w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_top(f, area, &w, &mut state);
            })
            .unwrap();

        let text = row_text(&terminal);
        let rendered = text.trim_end();
        assert_eq!(
            rendered, " T 03:55  Lv3  KO 42  Score 1234  │ Gnaw2",
            "HUD top line must render the fixed expected text"
        );
        assert_eq!(
            rendered,
            reference_top(&w),
            "must match the full-width layout"
        );
    }

    /// Item 14: the bottom bars still show the same labels, and reused buffers
    /// never leak digits from a previous frame.
    #[test]
    fn bottom_line_text_matches_reference_and_reuse_is_clean() {
        let mut w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let (hp_ref, dash_ref) = reference_bottom(&w);
        terminal
            .draw(|f| {
                let area = f.area();
                draw_bottom(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("XP"));
        assert!(text.contains(&hp_ref), "HP label {hp_ref:?} in {text:?}");
        assert!(
            text.contains(&dash_ref),
            "dash label {dash_ref:?} in {text:?}"
        );

        // Shrink every number, reusing the same buffers: no stale digits may
        // survive from the longer previous values.
        w.player.hp = 1.0;
        w.player.dash_cd = 0.25;
        w.elapsed = 299.9; // 00:00 left
        w.kills = 1;
        w.score = 2;
        w.player.level = 1;
        let (hp_ref, dash_ref) = reference_bottom(&w);
        assert_eq!(hp_ref, "♥ 1/100");
        assert_eq!(dash_ref, "dash 0.2s");
        terminal
            .draw(|f| {
                let area = f.area();
                draw_bottom(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("XP"));
        assert!(text.contains(&hp_ref), "HP label {hp_ref:?} in {text:?}");
        assert!(
            text.contains(&dash_ref),
            "dash label {dash_ref:?} in {text:?}"
        );
    }

    /// Item 14: drawing twice with the same reused state is idempotent.
    #[test]
    fn repeated_draws_are_identical() {
        let w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();

        let mut draw = |state: &mut HudState| {
            terminal
                .draw(|f| {
                    let area = f.area();
                    draw_top(f, area, &w, state);
                    let bottom = Rect::new(0, 1, area.width, 1);
                    draw_bottom(f, bottom, &w, state);
                })
                .unwrap();
            row_text(&terminal)
        };

        let first = draw(&mut state);
        let second = draw(&mut state);
        assert_eq!(first, second, "reused buffers must render identically");
        assert!(
            first.contains('♥') && first.contains("DASH READY"),
            "bottom bars should still render: {first:?}"
        );
    }

    /// Item 14: with several weapons the tags keep the old " name<level>" shape.
    #[test]
    fn multiple_weapons_match_reference_text() {
        let mut w = known_world();
        w.player
            .loadout
            .weapons
            .push(crate::game::weapon::Weapon::new(WeaponKind::CheeseSpray));
        w.player.loadout.weapons[1].level = 4;
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_top(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        let rendered = text.trim_end();
        assert_eq!(rendered, reference_top(&w));
        assert!(rendered.ends_with("│ Gnaw2 Chee4"), "got {rendered:?}");

        // And dropping back to one weapon must not leave a stale tag behind.
        w.player.loadout.weapons.truncate(1);
        terminal
            .draw(|f| {
                let area = f.area();
                draw_top(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert_eq!(text.trim_end(), reference_top(&w));
    }

    #[test]
    fn meter_resolves_all_eighths_in_the_last_cell() {
        let mut terminal = Terminal::new(TestBackend::new(4, 1)).unwrap();
        for (n, expected) in " ▏▎▍▌▋▊▉█".chars().enumerate() {
            terminal
                .draw(|f| meter(f, f.area(), "X", n as f32 / 8.0, TAIL))
                .unwrap();
            let cell = &terminal.backend().buffer()[(2, 0)];
            assert_eq!(cell.symbol().chars().next(), Some(expected));
            assert_eq!(cell.bg, TRACK);
        }
    }
}
