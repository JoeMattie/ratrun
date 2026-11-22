//! In-viewport tactical HUD overlays: boss health bar & arrival warning,
//! off-screen boss compass, low-health / hurt perimeter cues, and transient
//! event callouts. All drawing writes directly into ratatui's buffer with zero
//! per-frame heap allocation.

use std::fmt::Write as _;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::director;
use crate::game::world::World;
use crate::render::hud::{
    boss_codename_kind, HudState, BOSS_HOT, BOSS_ROSE, DANGER, DANGER_DIM, TRAIL_GOLD,
    WARN_AMBER,
};
use crate::render::palette::{self, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};

/// Render all active in-viewport HUD overlays over `game` (the pixel viewport).
pub fn draw_overlays(frame: &mut Frame, game: Rect, world: &World, state: &mut HudState) {
    if game.width < 12 || game.height < 6 {
        return;
    }

    draw_damage_and_low_hp(frame, game, world, state);
    let top_reserved = draw_boss_ui(frame, game, world, state);
    let miniboss_reserved = draw_miniboss_ui(frame, game, world, state, top_reserved);
    draw_boss_compass(frame, game, world, state);
    draw_callouts(frame, game, world, state, top_reserved + miniboss_reserved);
}

/// Subtle perimeter brackets on damage impact and critical low health.
/// Keeps the center combat playfield completely unobstructed.
fn draw_damage_and_low_hp(frame: &mut Frame, game: Rect, world: &World, state: &HudState) {
    let max_hp = world.player.max_hp().max(1.0);
    let hp_ratio = (world.player.hp.max(0.0) / max_hp).clamp(0.0, 1.0);
    let low_hp = world.player.alive() && hp_ratio <= 0.28;
    let hurt = world.player.hurt_flash > 0.0 || state.hurt_edge > 0.0;

    if !low_hp && !hurt {
        return;
    }

    let pulse = 0.5 + 0.5 * (state.clock * 7.5).sin();
    let hurt_t = (state.hurt_edge / 0.35).clamp(0.0, 1.0);
    let border_col = if hurt {
        palette::to_color(palette::mix((190, 48, 58), (255, 132, 138), hurt_t))
    } else {
        palette::to_color(palette::mix((138, 36, 46), (242, 82, 94), pulse))
    };

    let buf = frame.buffer_mut();
    let x0 = game.x;
    let x1 = game.right().saturating_sub(1);
    let y0 = game.y;
    let y1 = game.bottom().saturating_sub(1);

    let arm_h: u16 = if hurt { 5 } else { 3 }.min(game.width / 4);
    let arm_v: u16 = if hurt { 2 } else { 1 }.min(game.height / 4);

    // Paint 4 crisp corner registration brackets without filling interior cells.
    let style = Style::default().fg(border_col).bg(PLATE);
    buf[(x0, y0)].set_char('┌').set_style(style);
    buf[(x1, y0)].set_char('┐').set_style(style);
    buf[(x0, y1)].set_char('└').set_style(style);
    buf[(x1, y1)].set_style(style).set_char('┘');

    for dx in 1..=arm_h {
        if x0 + dx < x1 {
            buf[(x0 + dx, y0)].set_char('─').set_style(style);
            buf[(x0 + dx, y1)].set_char('─').set_style(style);
        }
        if x1.saturating_sub(dx) > x0 {
            buf[(x1 - dx, y0)].set_char('─').set_style(style);
            buf[(x1 - dx, y1)].set_char('─').set_style(style);
        }
    }
    for dy in 1..=arm_v {
        if y0 + dy < y1 {
            buf[(x0, y0 + dy)].set_char('│').set_style(style);
            buf[(x1, y0 + dy)].set_char('│').set_style(style);
        }
        if y1.saturating_sub(dy) > y0 {
            buf[(x0, y1 - dy)].set_char('│').set_style(style);
            buf[(x1, y1 - dy)].set_char('│').set_style(style);
        }
    }

    // Critical HP pill tucked right above the bottom HP meter (never over center).
    if low_hp && hp_ratio <= 0.22 && game.width >= 30 && game.height >= 12 {
        let label = " ◆ CRITICAL HP ◆ ";
        let lw = label.chars().count() as u16;
        if lw + 4 <= game.width {
            let cx = game.x + (game.width - lw) / 2;
            let pill_style = if pulse > 0.55 {
                Style::default()
                    .fg(PLATE)
                    .bg(DANGER)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(DANGER)
                    .bg(PLATE)
                    .add_modifier(Modifier::BOLD)
            };
            buf.set_stringn(cx, y1, label, lw as usize, pill_style);
        }
    }
}

/// How long before `BOSS_TIME` the pre-boss warning banner opens, in seconds.
pub const BOSS_WARNING_SECS: f32 = 25.0;

/// The pre-boss warning banner, drawn 25 s out (`BOSS_WARNING_SECS`).
///
/// It names the boss that is coming and sends the player to the arena — the
/// compass (`draw_boss_compass`) points at the arena until the boss spawns,
/// then at the boss itself. Returns the rows it reserved at the top of `game`
/// (0 when it did not draw: too narrow, or no warning window).
fn draw_boss_warning(
    frame: &mut Frame,
    game: Rect,
    world: &World,
    state: &mut HudState,
    codename: &str,
    pulse: f32,
) -> u16 {
    if game.width < 30 {
        return 0;
    }
    let rem = (director::BOSS_TIME - world.elapsed).ceil().max(1.0) as u32;
    state.boss_banner.clear();
    if game.width >= 56 {
        let _ = write!(
            state.boss_banner,
            " ▸ {} IN {:02}s  ·  HEAD FOR THE ARENA ◂ ",
            codename,
            rem
        );
    } else if game.width >= 40 {
        let _ = write!(state.boss_banner, " ▸ {} IN {:02}s ◂ ", codename, rem);
    } else {
        let _ = write!(state.boss_banner, " ▸ ARENA IN {:02}s ◂ ", rem);
    }
    let bw = (state.boss_banner.chars().count() as u16).min(game.width);
    let bx = game.x + (game.width - bw) / 2;
    let by = game.y + 1;
    let fg = if pulse > 0.5 { PLATE } else { WARN_AMBER };
    let bg = if pulse > 0.5 { WARN_AMBER } else { PLATE };
    let buf = frame.buffer_mut();
    buf.set_stringn(
        bx,
        by,
        &state.boss_banner,
        bw as usize,
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    );
    3
}

/// Draw Boss Countdown Warning, Arrival Banner, or Persistent Boss Health Bar.
/// Returns how many rows at the top of `game` are occupied by boss chrome.
fn draw_boss_ui(frame: &mut Frame, game: Rect, world: &World, state: &mut HudState) -> u16 {
    let boss_soon = !world.director.boss_spawned
        && world.elapsed >= director::BOSS_TIME - BOSS_WARNING_SECS
        && world.elapsed < director::BOSS_TIME;
    let pulse = 0.5 + 0.5 * (state.clock * 8.0).sin();
    // The fight's own boss name: a floor can pool more than one boss
    // (`BossKind::pool_for_theme`), so a living boss names itself and the
    // warning names the one the level seed has already drawn.
    let kind = world.boss_kind().unwrap_or_else(|| {
        crate::game::ecs::systems::boss::BossKind::pool_pick(world.level.theme, world.level.seed)
    });
    let (codename, class_tag) = boss_codename_kind(kind);

    // Case 1: pre-boss warning (see `draw_boss_warning`).
    if boss_soon {
        let rows = draw_boss_warning(frame, game, world, state, codename, pulse);
        if rows > 0 {
            return rows;
        }
    }

    // Case 2: Boss is alive (full health bar if `boss_hp` is populated, else engaged banner).
    if let Some((hp, max_hp)) = state.telemetry.boss_hp {
        let ratio = (hp.max(0.0) / max_hp.max(1.0)).clamp(0.0, 1.0);
        let trail = state.boss_hp_trail.clamp(ratio, 1.0);
        let bar_w = (game.width * 62 / 100)
            .clamp(28, 72)
            .min(game.width.saturating_sub(2));
        if bar_w < 16 || game.height < 8 {
            return 0;
        }
        let bx = game.x + (game.width - bar_w) / 2;
        let y_hdr = game.y + 1;
        let y_bar = y_hdr + 1;

        state.boss_hp_text.clear();
        let pct = (ratio * 100.0).ceil() as u32;
        let _ = write!(
            state.boss_hp_text,
            "{:.0}/{:.0}  {:>3}% ",
            hp.max(0.0),
            max_hp,
            pct.min(100)
        );

        let low_boss = ratio <= 0.25;
        let fill_col = if low_boss {
            palette::to_color(palette::mix((232, 74, 96), (255, 134, 88), pulse))
        } else {
            BOSS_ROSE
        };
        let header_accent = if state.boss_intro_timer > 0.0 && pulse > 0.5 {
            COAT
        } else {
            fill_col
        };

        let buf = frame.buffer_mut();
        // Header strip on dark metal PLATE (top-left bright cap, right shadow cap).
        buf.set_style(Rect::new(bx, y_hdr, bar_w, 1), Style::default().bg(PLATE));
        buf[(bx, y_hdr)].set_char('▌').set_fg(header_accent).set_bg(PLATE);
        buf[(bx + bar_w - 1, y_hdr)]
            .set_char('▐')
            .set_fg(DANGER_DIM)
            .set_bg(PLATE);

        let hdr_right = bx + bar_w - 1;
        let mut cx = bx + 2;
        (cx, _) = buf.set_stringn(
            cx,
            y_hdr,
            "◆ ",
            hdr_right.saturating_sub(cx) as usize,
            Style::default()
                .fg(header_accent)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        (cx, _) = buf.set_stringn(
            cx,
            y_hdr,
            codename,
            hdr_right.saturating_sub(cx) as usize,
            Style::default()
                .fg(COAT)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );

        // Fight phase, read straight from the boss framework's `Boss::phase`
        // (1 = healthiest; the cat and the broom have two phases, the mower
        // three). Drawn as `  PHASE n` from stack buffers, so the header stays
        // allocation-free. Skipped when the header cannot also fit the HP
        // readout to its right.
        let hp_tw = state.boss_hp_text.chars().count() as u16;
        if let Some(phase) = world.boss_phase() {
            let mut digit = [0u8; 4];
            let d = char::from_digit(u32::from(phase.min(9)), 10)
                .unwrap_or('1')
                .encode_utf8(&mut digit);
            let phase_w = 8 + d.len() as u16;
            if hdr_right.saturating_sub(cx) >= phase_w + hp_tw + 2 {
                (cx, _) = buf.set_stringn(
                    cx,
                    y_hdr,
                    "  PHASE ",
                    hdr_right.saturating_sub(cx) as usize,
                    Style::default().fg(WARN_AMBER).bg(PLATE),
                );
                (cx, _) = buf.set_stringn(
                    cx,
                    y_hdr,
                    d,
                    hdr_right.saturating_sub(cx) as usize,
                    Style::default()
                        .fg(WARN_AMBER)
                        .bg(PLATE)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }

        // During arrival intro, show classification subtitle if room permits.
        if state.boss_intro_timer > 0.0
            && hdr_right > cx + hp_tw + class_tag.len() as u16 + 4
        {
            let (nx, _) = buf.set_stringn(
                cx,
                y_hdr,
                "  ·  ",
                hdr_right.saturating_sub(cx) as usize,
                Style::default().fg(EDGE).bg(PLATE),
            );
            buf.set_stringn(
                nx,
                y_hdr,
                class_tag,
                hdr_right.saturating_sub(nx + hp_tw) as usize,
                Style::default().fg(TAIL).bg(PLATE),
            );
        }

        if hdr_right > cx + hp_tw {
            buf.set_stringn(
                hdr_right - hp_tw,
                y_hdr,
                &state.boss_hp_text,
                hp_tw as usize,
                Style::default().fg(MUTED).bg(PLATE),
            );
        } else {
            // Narrow viewport (e.g. 57 cols): show compact percentage right-aligned.
            state.boss_hp_text.clear();
            let _ = write!(state.boss_hp_text, " {:>3}% ", pct.min(100));
            let short_w = state.boss_hp_text.chars().count() as u16;
            if hdr_right > cx + short_w {
                buf.set_stringn(
                    hdr_right - short_w,
                    y_hdr,
                    &state.boss_hp_text,
                    short_w as usize,
                    Style::default().fg(MUTED).bg(PLATE),
                );
            }
        }

        // Eighth-cell precision Boss HP bar with gold damage trail and subtle 25% phase ticks.
        const STEPS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
        buf[(bx, y_bar)].set_char('▌').set_fg(header_accent).set_bg(PLATE);
        let fill_x = bx + 1;
        let fill_w = bar_w.saturating_sub(2);
        let units = (ratio * f32::from(fill_w) * 8.0).round() as usize;
        let trail_units = (trail * f32::from(fill_w) * 8.0).round() as usize;
        let q1 = fill_w / 4;
        let q2 = fill_w / 2;
        let q3 = (fill_w * 3) / 4;
        for i in 0..fill_w {
            let base = 8 * i as usize;
            let fill_step = units.saturating_sub(base).min(8);
            let trail_step = trail_units.saturating_sub(base).min(8);
            let is_notch = fill_w >= 20 && (i == q1 || i == q2 || i == q3);
            let cell = &mut buf[(fill_x + i, y_bar)];
            if fill_step == 8 {
                let fg = if is_notch {
                    palette::to_color(palette::mix(
                        if low_boss { (255, 118, 88) } else { (236, 76, 128) },
                        (255, 255, 255),
                        0.28,
                    ))
                } else {
                    fill_col
                };
                cell.set_char('█').set_fg(fg).set_bg(TRACK);
            } else if fill_step > 0 {
                let bg = if trail_step > fill_step {
                    TRAIL_GOLD
                } else {
                    TRACK
                };
                cell.set_char(STEPS[fill_step]).set_fg(fill_col).set_bg(bg);
            } else if trail_step > 0 {
                cell.set_char(STEPS[trail_step])
                    .set_fg(TRAIL_GOLD)
                    .set_bg(TRACK);
            } else if is_notch {
                cell.set_char('▏').set_fg(EDGE).set_bg(TRACK);
            } else {
                cell.set_char(' ').set_fg(fill_col).set_bg(TRACK);
            }
        }
        buf[(bx + bar_w - 1, y_bar)]
            .set_char('▐')
            .set_fg(DANGER_DIM)
            .set_bg(PLATE);
        return 4;
    }

    // Fallback: Boss is active (or arrival intro playing) before `World::boss_hp()` is wired.
    if state.telemetry.boss_alive || state.boss_intro_timer > 0.0 {
        state.boss_banner.clear();
        if game.width >= 54 {
            let _ = write!(state.boss_banner, " ◆ {}  ·  {} ", codename, class_tag);
        } else {
            let _ = write!(state.boss_banner, " ◆ {} ", codename);
        }
        let bw = (state.boss_banner.chars().count() as u16).min(game.width.saturating_sub(2));
        if bw < 6 {
            return 0;
        }
        let bx = game.x + (game.width - bw) / 2;
        let by = game.y + 1;
        let border = if state.boss_intro_timer > 0.0 && pulse > 0.5 {
            BOSS_HOT
        } else {
            BOSS_ROSE
        };
        let buf = frame.buffer_mut();
        buf[(bx.saturating_sub(1).max(game.x), by)]
            .set_char('▌')
            .set_fg(border)
            .set_bg(PLATE);
        buf.set_stringn(
            bx,
            by,
            &state.boss_banner,
            bw as usize,
            Style::default()
                .fg(COAT)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        return 3;
    }

    0
}

/// The named-miniboss nameplate (wave 3). While one is alive it draws a short
/// name and a compact HP bar under the boss chrome; nothing is shown before it
/// arrives.
/// Returns the number of rows reserved at the top of `game`.
fn draw_miniboss_ui(
    frame: &mut Frame,
    game: Rect,
    world: &World,
    state: &mut HudState,
    top: u16,
) -> u16 {
    if game.width < 30 || game.height < 8 {
        return 0;
    }
    let y = game.y + top;
    if y + 1 >= game.bottom() {
        return 0;
    }
    // A live miniboss: two rows (name + bar).
    if let Some((name, hp, max_hp)) = world.miniboss_hp() {
        let ratio = (hp.max(0.0) / max_hp.max(1.0)).clamp(0.0, 1.0);
        let bar_w = (game.width * 46 / 100).clamp(18, 54).min(game.width.saturating_sub(2));
        if bar_w < 12 {
            return 0;
        }
        let bx = game.x + (game.width - bar_w) / 2;
        state.miniboss_text.clear();
        let _ = write!(
            state.miniboss_text,
            " ◆ {}  {:.0}/{:.0} ",
            name.to_uppercase(),
            hp.max(0.0),
            max_hp
        );
        let tw = (state.miniboss_text.chars().count() as u16).min(game.width);
        let tx = game.x + (game.width - tw) / 2;
        let buf = frame.buffer_mut();
        buf.set_stringn(
            tx,
            y,
            &state.miniboss_text,
            tw as usize,
            Style::default()
                .fg(COAT)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        // Bar: filled part in amber, the rest in the track colour.
        let filled = (bar_w as f32 * ratio).round() as u16;
        for i in 0..bar_w {
            let c = if i < filled { WARN_AMBER } else { TRACK };
            buf[(bx + i, y + 1)].set_char('█').set_style(Style::default().fg(c).bg(PLATE));
        }
        return 2;
    }
    // No countdown before a miniboss: it simply arrives (its entrance, its
    // arrival sting and then this nameplate are the announcement).
    0
}

/// Draw a compact directional edge pointer when the target is off-screen.
///
/// The pointer follows the living boss. Before the boss spawns it follows the
/// **arena** instead, so the 25 s warning's "head for the arena" is a real
/// bearing and not only text; the pointer switches to the boss the moment the
/// boss exists.
fn draw_boss_compass(frame: &mut Frame, game: Rect, world: &World, state: &HudState) {
    // The target: the boss once it exists, else the arena during the pre-boss
    // warning window. `to_boss` only picks the edge label.
    let (offset, to_boss) = match state.telemetry.boss_offset {
        Some(offset) => (offset, true),
        None => {
            let warning = !world.director.boss_spawned
                && world.elapsed >= director::BOSS_TIME - BOSS_WARNING_SECS
                && world.elapsed < director::BOSS_TIME;
            if !warning {
                return;
            }
            let Some(room) = world.level.boss_room() else {
                return;
            };
            (room.center() - world.player.pos, false)
        }
    };
    if game.width < 24 || game.height < 10 {
        return;
    }
    // Viewport extents in world pixels (half-block: 1 col = 1px wide, 1 row =
    // 2px tall). This is the *rendered* span — the logical viewport times the
    // mouse-wheel zoom — so zooming out moves the edge indicator out with the
    // world instead of hiding a boss that is plainly on screen.
    let span = world.view_span();
    let half_w = (span.x * 0.5).max(f32::from(game.width) * 0.5);
    let half_h = (span.y * 0.5).max(f32::from(game.height));
    if offset.x.abs() <= half_w * 0.92 && offset.y.abs() <= half_h * 0.92 {
        return; // Boss is already visible inside the viewport.
    }

    let nx = offset.x / half_w.max(1.0);
    let ny = offset.y / half_h.max(1.0);
    let pulse = 0.5 + 0.5 * (state.clock * 9.0).sin();
    let badge_fg = if pulse > 0.5 { COAT } else { BOSS_ROSE };
    let style = Style::default()
        .fg(badge_fg)
        .bg(PLATE)
        .add_modifier(Modifier::BOLD);

    let buf = frame.buffer_mut();
    if nx.abs() >= ny.abs() {
        // Left or right edge.
        let frac_y = ((ny / nx.abs().max(1.0)) * 0.5 + 0.5).clamp(0.18, 0.82);
        let y = game.y + ((game.height.saturating_sub(2)) as f32 * frac_y) as u16;
        if nx < 0.0 {
            let label = if to_boss { " ◄ BOSS " } else { " ◄ ARENA " };
            buf.set_stringn(game.x + 1, y, label, (game.width - 2) as usize, style);
        } else {
            let label = if to_boss { " BOSS ► " } else { " ARENA ► " };
            let lw = label.chars().count() as u16;
            let x = game.right().saturating_sub(lw + 1).max(game.x);
            buf.set_stringn(x, y, label, lw as usize, style);
        }
    } else {
        // Top or bottom edge.
        let frac_x = ((nx / ny.abs().max(1.0)) * 0.5 + 0.5).clamp(0.15, 0.85);
        let label = match (ny < 0.0, to_boss) {
            (true, true) => " ▲ BOSS ",
            (true, false) => " ▲ ARENA ",
            (false, true) => " ▼ BOSS ",
            (false, false) => " ▼ ARENA ",
        };
        let lw = label.chars().count() as u16;
        let cx = game.x + ((game.width.saturating_sub(lw)) as f32 * frac_x) as u16;
        let y = if ny < 0.0 {
            game.y
        } else {
            game.bottom().saturating_sub(1)
        };
        buf.set_stringn(cx, y, label, lw as usize, style);
    }
}

/// Draw up to 2 active transient callout pills tucked into the top-right edge
/// (or centered below the boss bar on narrow terminals).
fn draw_callouts(
    frame: &mut Frame,
    game: Rect,
    world: &World,
    state: &HudState,
    top_reserved: u16,
) {
    if game.width < 28 || game.height < 8 {
        return;
    }

    let sector_accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    let buf = frame.buffer_mut();

    // Collect indices of active slots sorted newest-first (up to 2 visible at once).
    let mut active_indices = [usize::MAX; 3];
    let mut n = 0usize;
    for (i, slot) in state.callouts.iter().enumerate() {
        if slot.active && slot.timer > 0.0 {
            active_indices[n] = i;
            n += 1;
        }
    }
    if n == 0 {
        return;
    }
    // Insertion sort by remaining time descending (at most 3 elements), with
    // the Library's discovery notes after everything else: only two pills are
    // visible, and "you found a new entry" must never hide the pill that
    // confirms what the player just did (an upgrade pick, a pickup, a warning).
    let rank = |i: usize| {
        let slot = &state.callouts[i];
        (
            slot.kind != Some(crate::render::hud::CalloutKind::Library),
            slot.timer,
        )
    };
    for i in 1..n {
        let mut j = i;
        while j > 0 && rank(active_indices[j]) > rank(active_indices[j - 1]) {
            active_indices.swap(j, j - 1);
            j -= 1;
        }
    }

    let start_y = game.y + top_reserved.max(1);
    let max_pills = if game.height >= 16 { 2 } else { 1 };
    let visible = n.min(max_pills);
    let align_right = game.width >= 66;

    // Uniform column width across stacked pills so left `▌` bars align crisply.
    let mut col_w = 0u16;
    for &idx in &active_indices[..visible] {
        let slot = &state.callouts[idx];
        let w = 1
            + 1
            + slot.tag.chars().count() as u16
            + 2
            + slot.message.chars().count() as u16
            + 1;
        col_w = col_w.max(w);
    }
    col_w = col_w.min(game.width.saturating_sub(2));
    if col_w < 8 {
        return;
    }
    let x0 = if align_right {
        game.right().saturating_sub(col_w + 1)
    } else {
        game.x + (game.width.saturating_sub(col_w)) / 2
    };

    for (row_idx, &slot_idx) in active_indices[..visible].iter().enumerate() {
        let y = start_y + row_idx as u16;
        if y >= game.bottom().saturating_sub(2) {
            break;
        }
        let slot = &state.callouts[slot_idx];
        let kind_col = slot
            .kind
            .map(|k| k.color(sector_accent))
            .unwrap_or(sector_accent);
        // Smooth fade-out in the final 0.45s.
        let fade = (slot.timer / 0.45).clamp(0.0, 1.0);
        let tag_fg = fade_color(kind_col, fade);
        let msg_fg = fade_color(COAT, fade);
        let bar_fg = fade_color(kind_col, fade);

        buf.set_style(Rect::new(x0, y, col_w, 1), Style::default().bg(PLATE));
        buf[(x0, y)].set_char('▌').set_fg(bar_fg).set_bg(PLATE);

        let mut cx = x0 + 2;
        let limit = x0 + col_w;
        (cx, _) = buf.set_stringn(
            cx,
            y,
            &slot.tag,
            limit.saturating_sub(cx) as usize,
            Style::default()
                .fg(tag_fg)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        (cx, _) = buf.set_stringn(
            cx,
            y,
            "  ",
            limit.saturating_sub(cx) as usize,
            Style::default().bg(PLATE),
        );
        buf.set_stringn(
            cx,
            y,
            &slot.message,
            limit.saturating_sub(cx) as usize,
            Style::default().fg(msg_fg).bg(PLATE),
        );
    }
}

fn fade_color(c: Color, alpha: f32) -> Color {
    let rgb = match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (210, 210, 210),
    };
    palette::to_color(palette::mix((38, 46, 50), rgb, alpha))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::math::Vec2;
    use crate::render::hud::CalloutKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buf_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Spawn the level's themed boss straight into the ECS world, so the bar's
    /// phase readout (which reads `World::boss_phase()`) has a real entity.
    fn spawn_boss(w: &mut World, phase: u8) {
        use crate::game::ecs::systems::boss::{Boss, BossKind};
        let arena = w
            .level
            .boss_room()
            .expect("generated level has a boss room")
            .rect;
        let mut boss = Boss::new(
            BossKind::for_theme(w.level.theme),
            w.player.pos,
            1400.0,
            arena,
            arena.center_world(),
            false,
        );
        boss.phase = phase;
        w.ecs.spawn(boss);
    }

    /// The bar names the level's own boss — the campaign screens' names, not
    /// the old generic "apex" codename — and shows the framework's phase.
    #[test]
    fn boss_health_bar_renders_codename_hp_and_phase() {
        let mut w = World::new(Theme::Sewer, 7);
        w.director.boss_spawned = true;
        spawn_boss(&mut w, 2);
        let mut hud = HudState::default();
        hud.telemetry.set_boss_hp(Some((980.0, 1400.0)));
        hud.boss_hp_trail = 0.85;

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(text.contains("APEX JAGUAR"), "missing boss name: {text}");
        assert!(text.contains("980/1400"), "missing boss HP numbers: {text}");
        assert!(text.contains("70%"), "missing boss HP percent: {text}");
        assert!(text.contains("PHASE 2"), "missing the fight phase: {text}");
    }

    /// Each campaign level names **the boss actually on the floor**. A pooled
    /// floor (Sewer: Jaguar / Horseshoe, Kitchen: Sweeper / R-01) fields one of
    /// two by level seed, so the bar is checked for both pool mates.
    #[test]
    fn the_boss_bar_uses_each_levels_boss_name() {
        use crate::game::ecs::systems::boss::BossKind;
        for (level, names) in [
            (0usize, &["APEX JAGUAR", crate::lore::HORSESHOE_NAME][..]),
            (1, &["THE SWEEPER", crate::lore::MEGARAT_NAME][..]),
            (2, &["TURF-REAPER MK.IV"][..]),
        ] {
            let mut seen: Vec<&str> = Vec::new();
            for seed in 0..48u64 {
                let mut w = World::new_level(
                    level,
                    seed,
                    crate::game::loadout::Loadout::new(crate::game::weapon::WeaponKind::Gnaw),
                );
                w.director.boss_spawned = true;
                // The plate of the boss this level seed fields.
                let kind = BossKind::pool_pick(w.level.theme, w.level.seed);
                let name = boss_codename_kind(kind).0;
                assert!(names.contains(&name), "level {level}: {kind:?} is not this floor's");
                let mut hud = HudState::default();
                hud.telemetry.set_boss_hp(Some((900.0, 1400.0)));
                let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
                terminal
                    .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
                    .unwrap();
                let text = buf_text(&terminal);
                assert!(text.contains(name), "level {level} seed {seed}: missing {name} in {text}");
                // ...and never its pool mate's.
                for other in names.iter().filter(|n| **n != name) {
                    assert!(!text.contains(other), "level {level} seed {seed}: named {other}");
                }
                if !seen.contains(&name) {
                    seen.push(name);
                }
                if seen.len() == names.len() {
                    break;
                }
            }
            assert_eq!(seen.len(), names.len(), "level {level}: only saw {seen:?}");
        }
    }

    #[test]
    fn pre_boss_countdown_renders_before_boss_time() {
        let mut w = World::new(Theme::Lab, 7);
        w.elapsed = director::BOSS_TIME - 6.2;
        let mut hud = HudState::default();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(
            text.contains("TURF-REAPER MK.IV IN 07s"),
            "missing pre-boss countdown in {text}"
        );
        assert!(
            text.contains("HEAD FOR THE ARENA"),
            "the 25 s warning must send the player to the arena: {text}"
        );
    }

    /// The warning is a *warning*: it opens 25 s before the boss, not 10 s.
    #[test]
    fn pre_boss_warning_opens_twenty_five_seconds_out() {
        let mut w = World::new(Theme::Lab, 7);
        w.elapsed = director::BOSS_TIME - 20.0;
        let mut hud = HudState::default();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(
            text.contains("TURF-REAPER MK.IV IN 20s"),
            "no 25 s pre-boss warning in {text}"
        );

        // Well before the window: nothing yet (the top rows stay free).
        w.elapsed = director::BOSS_TIME - 30.0;
        let mut hud = HudState::default();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(
            !text.contains("IN 30s") && !text.contains("HEAD FOR THE ARENA"),
            "the warning started too early: {text}"
        );
    }

    /// Before the boss exists, the compass points at the arena the warning
    /// sends the player to; it switches to the boss once there is one.
    #[test]
    fn the_warning_compass_points_at_the_arena_until_the_boss_arrives() {
        let mut w = World::new(Theme::Lab, 7);
        w.viewport = Vec2::new(80.0, 48.0);
        w.elapsed = director::BOSS_TIME - 20.0;
        let arena = w.level.boss_room().expect("boss room").center();
        let to = arena - w.player.pos;
        let mut hud = HudState::default();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(
            !text.contains("BOSS ►") && !text.contains("◄ BOSS"),
            "the compass pointed at a boss that does not exist yet: {text}"
        );
        // The right edge for the arena's bearing (the warning banner also says
        // "ARENA", so match a labelled edge, not just the word).
        let expected = if to.x.abs() >= to.y.abs() {
            if to.x < 0.0 {
                "◄ ARENA"
            } else {
                "ARENA ►"
            }
        } else if to.y < 0.0 {
            "▲ ARENA"
        } else {
            "▼ ARENA"
        };
        assert!(
            text.contains(expected),
            "no {expected} bearing on the warning: {text}"
        );
    }

    #[test]
    fn offscreen_boss_compass_points_toward_boss() {
        let mut w = World::new(Theme::Kitchen, 7);
        w.viewport = Vec2::new(80.0, 48.0);
        let mut hud = HudState::default();
        hud.telemetry.set_boss_hp(Some((1400.0, 1400.0)));
        hud.telemetry.set_boss_offset(Some(Vec2::new(220.0, 5.0)));

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(text.contains("BOSS ►"), "expected right-edge boss compass");
    }

    #[test]
    fn critical_low_hp_shows_brackets_and_warning() {
        let mut w = World::new(Theme::Sewer, 7);
        w.player.hp = 15.0; // 15% HP
        let mut hud = HudState::default();

        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal
            .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
            .unwrap();
        let text = buf_text(&terminal);
        assert!(text.contains("CRITICAL HP"), "missing critical HP cue");
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 0)].symbol(), "┌");
        assert_eq!(buf[(59, 0)].symbol(), "┐");
        assert_eq!(buf[(0, 19)].symbol(), "└");
        assert_eq!(buf[(59, 19)].symbol(), "┘");
    }

    #[test]
    fn callouts_render_without_panicking_at_tiny_sizes() {
        let w = World::new(Theme::Sewer, 7);
        let mut hud = HudState::default();
        hud.push_callout(CalloutKind::Heal, "+MEDKIT", "HP RESTORED", 2.5);
        hud.push_callout(CalloutKind::Escalation, "WARNING", "ELITE BRUTES UNLOCKED", 2.5);

        for (tw, th) in [(0, 0), (1, 1), (10, 4), (28, 8), (57, 20), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| draw_overlays(f, f.area(), &w, &mut hud))
                .unwrap();
            if tw >= 57 && th >= 20 {
                let text = buf_text(&terminal);
                assert!(text.contains("+MEDKIT") || text.contains("WARNING"));
            }
        }
    }
}
