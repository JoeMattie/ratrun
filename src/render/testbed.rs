//! Testbed selection and controls over the live field.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use ratatui::Frame;

use crate::game::world::World;
use crate::render::palette::{COAT, MUTED, PLATE, TAIL};

pub fn draw(frame: &mut Frame, game: Rect, world: &World, names: [&str; 5]) {
    let boss = world.boss_hp().is_some();
    let miniboss = world.miniboss_hp().is_some();
    let top = if boss || miniboss {
        1 + 2 * u16::from(boss) + 2 * u16::from(miniboss)
    } else {
        0
    };
    let width = game.width.saturating_sub(2).min(50);
    let height = 7;
    if width < 16 || game.height < top + height {
        return;
    }

    let panel = Rect::new(game.x + 1, game.y + top, width, height);
    frame.render_widget(Clear, panel);
    let buf = frame.buffer_mut();
    let base = Style::default().bg(PLATE);
    let heading = base.fg(TAIL).add_modifier(Modifier::BOLD);
    let label = base.fg(MUTED);
    let value = base.fg(COAT);
    buf.set_style(panel, base);

    let x = panel.x + 1;
    let room = usize::from(width - 2);
    buf.set_stringn(x, panel.y, "TESTBED · F10 hide", room, heading);
    let rows = [
        ("WEAPON  1/2 ", " · 3 evolve"),
        ("PICKUP  4/5 ", " · 6 drop"),
        ("ENEMY   7/8 ", " · 9 spawn · 0 clear"),
        ("BOSS    F1/F2 ", " · F3 fight"),
        ("MINIBOSS F5/F6 ", " · F7 spawn"),
    ];
    for (i, ((prefix, suffix), name)) in rows.into_iter().zip(names).enumerate() {
        let y = panel.y + 1 + i as u16;
        let (x, _) = buf.set_stringn(x, y, prefix, room, label);
        let space = |x| usize::from(panel.right().saturating_sub(x + 1));
        let (x, _) = buf.set_stringn(x, y, name, space(x), value);
        buf.set_stringn(x, y, suffix, space(x), label);
    }
    let god = if world.player.god { "on" } else { "off" };
    let horde = if world.peace { "off" } else { "on" };
    let footer = format!("H god {god} · J horde {horde} · K kill · T sector");
    buf.set_stringn(x, panel.bottom() - 1, footer, room, label);
}
