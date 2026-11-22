//! The Library screen's drawing tests: every entry draws, the three layouts
//! hold, and a locked card never prints an entry's name.

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::game::library::{all_rows, Library};
use crate::render::menu::{HitMap, Target};

/// Draw one frame into a `TestBackend` and return the screen as text.
fn render(rows: &[Row], lib: &Library, ui: &mut LibraryUi, w: u16, h: u16) -> (String, DrawInfo) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut hits = HitMap::default();
    let mut info = DrawInfo::default();
    terminal
        .draw(|f| {
            info = draw(f, f.area(), rows, lib, (150, 190, 176), ui, &mut hits);
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for y in 0..h {
        for x in 0..w {
            text.push_str(buf[(x, y)].symbol());
        }
        text.push('\n');
    }
    (text, info)
}

/// A library where every third row has been met (locked rows stay locked).
fn mixed(rows: &[Row]) -> Library {
    let mut lib = Library::default();
    for (i, row) in rows.iter().enumerate() {
        if i % 3 != 2 {
            lib.mark_seen(row.key);
        }
    }
    lib
}

#[test]
fn draws_at_the_three_pinned_sizes() {
    let rows = all_rows();
    let lib = mixed(&rows);
    for (w, h) in [(57, 20), (80, 24), (120, 40)] {
        let mut ui = LibraryUi::default();
        let (text, info) = render(&rows, &lib, &mut ui, w, h);
        assert!(text.contains("LIBRARY"), "{w}x{h} has no title");
        assert!(text.contains("BESTIARY"), "{w}x{h} has no tab strip");
        assert!(info.lines > 0, "{w}x{h} listed no entries");
        assert!(info.visible > 0, "{w}x{h} showed no rows");
    }
}

#[test]
fn the_narrow_layout_opens_the_card_over_the_list() {
    let rows = all_rows();
    let lib = mixed(&rows);
    let mut ui = LibraryUi::default();
    let (list, info) = render(&rows, &lib, &mut ui, 57, 20);
    assert!(info.narrow, "57x20 must use the stacked layout");
    ui.card_open = true;
    let (card, _) = render(&rows, &lib, &mut ui, 57, 20);
    assert_ne!(list, card, "Enter must change the narrow screen");
}

#[test]
fn a_locked_row_and_card_never_print_the_name() {
    let rows = all_rows();
    let lib = Library::default();
    let mut ui = LibraryUi::default();
    ui.set_tab(Tab::Bestiary);
    ui.cursor = 0;
    let (text, _) = render(&rows, &lib, &mut ui, 80, 24);
    let first = rows
        .iter()
        .find(|r| r.tab == Tab::Bestiary)
        .expect("a bestiary row");
    assert!(text.contains("???"), "a locked row must read ???");
    assert!(
        !text.contains(first.name),
        "a locked card leaked the name {:?}",
        first.name
    );
    assert!(
        text.contains("NOT YET CATALOGUED"),
        "a locked card must say it is not catalogued"
    );
    // The locked hint is shown instead of the dossier.
    assert!(
        text.contains("found:"),
        "a locked card must hint where to look"
    );
}

#[test]
fn reveal_all_draws_every_entry_on_every_tab() {
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    for tab in Tab::ALL {
        let entries = rows.iter().filter(|r| r.tab == tab).count();
        for cursor in 0..entries {
            let mut ui = LibraryUi::default();
            ui.set_tab(tab);
            ui.cursor = cursor;
            let (text, _) = render(&rows, &lib, &mut ui, 120, 40);
            // A revealed card always prints its name (the row's own label).
            let row = rows
                .iter()
                .filter(|r| r.tab == tab)
                .nth(cursor)
                .expect("row");
            assert!(
                text.contains(row.name),
                "{:?} card {:?} did not print its name",
                tab,
                row.name
            );
        }
    }
}

#[test]
fn the_tab_strip_registers_targets_and_counts() {
    let rows = all_rows();
    let lib = mixed(&rows);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut hits = HitMap::default();
    let mut ui = LibraryUi::default();
    terminal
        .draw(|f| {
            draw(
                f,
                f.area(),
                &rows,
                &lib,
                (150, 190, 176),
                &mut ui,
                &mut hits,
            );
        })
        .unwrap();
    for tab in Tab::ALL {
        assert!(
            hits.rect_for(Target::LibraryTab(tab.index())).is_some(),
            "{:?} tab is not clickable",
            tab
        );
    }
    assert!(
        hits.rect_for(Target::LibraryRow(0)).is_some(),
        "no row target"
    );
    let buf = terminal.backend().buffer().clone();
    // The panel is centred with a one-cell margin: row 1 is its border, row 2
    // the tab strip.
    let row: String = (0..80).map(|x| buf[(x, 2)].symbol()).collect();
    let (seen, total) = lib.counts(&rows, Tab::Bestiary);
    assert!(
        row.contains(&format!("{seen}/{total}")),
        "the bestiary count {seen}/{total} is missing from {row:?}\n{}",
        (0..24)
            .map(|y| (0..80).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
