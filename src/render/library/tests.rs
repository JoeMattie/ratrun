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

/// The card's rectangle at a pinned terminal size, built the way [`draw`]
/// builds it: the panel is centred with a one-cell margin, the tabs and their
/// rule take the top two rows, the list takes two fifths of the width (26..40
/// columns) and above 70 columns the card sits beside it.
fn card_rect(w: u16, h: u16) -> Rect {
    let pw = w.saturating_sub(2).min(116);
    let ph = h.saturating_sub(2).min(42);
    let inner = Rect::new(
        (w - pw) / 2 + 1,
        (h - ph) / 2 + 1,
        pw.saturating_sub(2),
        ph.saturating_sub(2),
    );
    let body = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(4),
    );
    if inner.width < 70 {
        body
    } else {
        let list_w = (inner.width * 2 / 5).clamp(26, 40);
        Rect::new(
            body.x + list_w + 1,
            body.y,
            body.width.saturating_sub(list_w + 1),
            body.height,
        )
    }
}

/// The card's rows out of a rendered frame, right-trimmed.
fn card_rows(text: &str, rect: Rect) -> Vec<String> {
    text.lines()
        .skip(rect.y as usize)
        .take(rect.height as usize)
        .map(|line| {
            let chars: Vec<char> = line.chars().collect();
            let start = (rect.x as usize).min(chars.len());
            let end = (start + rect.width as usize).min(chars.len());
            chars[start..end].iter().collect::<String>().trim_end().to_string()
        })
        .collect()
}

/// Every run of whitespace folded to one space, so wrapped text can be matched.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The King Cobra's row: the longest note in the index, and the one the review
/// saw cut after three lines ("…leave the loop through its one gap, or").
fn cobra_row(rows: &[Row]) -> Row {
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::library::Key;
    let key = Key::Boss(BossKind::Cobra);
    rows.iter()
        .find(|r| r.key == key)
        .copied()
        .expect("the Cobra has a row")
}

/// The note rows the card actually drew: from the row that starts with the
/// note's first wrapped line down to the rule above the facts.
fn note_rows(text: &str, rect: Rect, dossier: &str) -> Vec<String> {
    let card = card_rows(text, rect);
    let first = wrap_words(dossier, usize::from(rect.width))[0].clone();
    let start = card
        .iter()
        .position(|row| row.starts_with(&first))
        .unwrap_or_else(|| panic!("the note is not on the card:\n{text}"));
    card[start..]
        .iter()
        .take_while(|row| !row.is_empty() && !row.contains('\u{2500}'))
        .cloned()
        .collect()
}

/// Selecting a card **by key** lands on that entry, whatever its row index
/// is: the row index is positional and moves when a group gains an entry (the
/// `library-boss` capture's hard-coded index opened a miniboss card instead).
#[test]
fn selecting_a_card_by_key_lands_on_that_entry() {
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::library::{Key, Tab};
    let rows = all_rows();
    let mut ui = LibraryUi::default();
    for kind in BossKind::all() {
        let key = Key::Boss(kind);
        ui.set_tab(Tab::Bestiary);
        assert!(
            ui.select_key(&rows, key),
            "{} is not in the Bestiary tab",
            kind.name()
        );
        assert_eq!(ui.selected_key(&rows), Some(key), "{}", kind.name());
        let expect = rows
            .iter()
            .filter(|r| r.tab == Tab::Bestiary)
            .position(|r| r.key == key)
            .expect("the row exists");
        assert_eq!(ui.cursor, expect, "{}", kind.name());
        assert!(ui.card_open, "a by-key selection opens the card");
    }
    // A key from another tab is refused and the cursor stays put.
    ui.set_tab(Tab::Arsenal);
    let before = ui.cursor;
    assert!(!ui.select_key(&rows, Key::Boss(BossKind::Cat)));
    assert_eq!(ui.cursor, before);
    // The card that opens is the named boss's, not whatever sits at an index.
    let mut lib = Library::default();
    for row in &rows {
        lib.mark_seen(row.key);
    }
    let mut ui = LibraryUi::default();
    ui.set_tab(Tab::Bestiary);
    assert!(ui.select_key(&rows, Key::Boss(BossKind::Cat)));
    let (text, _) = render(&rows, &lib, &mut ui, 120, 40);
    assert!(text.contains("APEX JAGUAR"), "the boss card is not open:\n{text}");
    assert!(text.contains("SHARES"), "the card lost its SHARES row:\n{text}");
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

/// The note is wrapped on whole words: no line is wider than the card and no
/// word is split across two of them.
#[test]
fn the_note_wraps_on_words() {
    let rows = all_rows();
    let dossier = cobra_row(&rows).dossier;
    for width in [73usize, 53, 45, 21] {
        let lines = wrap_words(dossier, width);
        assert!(lines.len() > 1, "{width}: the note did not wrap");
        for line in &lines {
            assert!(
                line.chars().count() <= width,
                "{width}: line {:?} overflows",
                line
            );
        }
        // Joining the lines gives back exactly the note's words, in order.
        assert_eq!(words(&lines.join(" ")), words(dossier), "{width}");
    }
    assert!(wrap_words("", 20).is_empty());
    assert!(wrap_words("anything", 0).is_empty());
}

/// A note that has to be cut ends a sentence, or fails that a whole word plus
/// the safe ellipsis glyph: never mid-word, never a dangling clause.
#[test]
fn a_cut_note_ends_on_a_sentence_or_the_ellipsis_glyph() {
    let sentences: Vec<String> = vec![
        "The first line ends a sentence.".into(),
        "the second runs on and".into(),
        "the third runs on and".into(),
        "the fourth ends it.".into(),
    ];
    // Everything fits: the note is returned untouched.
    assert_eq!(fit_lines(&sentences, 4, 40), sentences);
    // Two rows: the sentence at the top is kept and the rest is dropped, so the
    // cut lands on a full stop.
    assert_eq!(
        fit_lines(&sentences, 2, 40),
        vec!["The first line ends a sentence.".to_string()]
    );
    assert!(fit_lines(&sentences, 0, 40).is_empty());
    // No sentence inside the window: whole words and the ellipsis glyph.
    let running: Vec<String> = wrap_words(
        "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu",
        20,
    );
    let cut = fit_lines(&running, 2, 20);
    assert_eq!(cut.len(), 2);
    let last = cut.last().expect("two rows");
    assert!(last.ends_with('\u{2026}'), "{last:?} does not end in the glyph");
    assert!(last.chars().count() <= 20, "{last:?} overflows the card");
    // The retained text is whole words: every fragment is one of the note's.
    for word in last.split_whitespace() {
        if word == "\u{2026}" {
            continue;
        }
        assert!(running.join(" ").contains(word), "{word:?} is a fragment");
    }
}

/// A single (unwrapped) line that overflows is clipped on a word with the
/// glyph, not sliced through a letter.
#[test]
fn an_overlong_line_is_clipped_on_a_word() {
    assert_eq!(clip_line("short", 20), "short");
    assert_eq!(clip_line("exactly ten", 11), "exactly ten");
    let long = "\u{201c}Weaponized dairy. Don't ask.\u{201d} and then some more";
    let cut = clip_line(long, 24);
    assert!(cut.chars().count() <= 24, "{cut:?}");
    assert!(cut.ends_with('\u{2026}'), "{cut:?}");
    let kept = cut.trim_end_matches('\u{2026}').trim_end();
    assert!(!kept.is_empty());
    assert!(words(long).starts_with(&words(kept)), "{cut:?}");
    assert!(clip_line("anything", 0).is_empty());
}

/// At 120 × 40 the card is tall enough for the whole note: the art plate gives
/// up its rows rather than the dossier ending in the middle of a clause.
#[test]
fn a_tall_card_prints_the_whole_note() {
    use crate::game::library::{Key, Tab};
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    let rect = card_rect(120, 40);
    let row = cobra_row(&rows);
    let mut ui = LibraryUi::default();
    ui.set_tab(Tab::Bestiary);
    assert!(ui.select_key(&rows, Key::Boss(crate::game::ecs::systems::boss::BossKind::Cobra)));
    let (text, _) = render(&rows, &lib, &mut ui, 120, 40);
    let note = note_rows(&text, rect, row.dossier);
    assert_eq!(
        words(&note.join(" ")),
        words(row.dossier),
        "the whole note is on the card:\n{text}"
    );
    // ...and the facts still make it: the cut is the art's loss, not theirs.
    assert!(text.contains("SHARES"), "the facts lost a row:\n{text}");
}

/// Where the note cannot fit (57 × 20, 80 × 24) it takes every row it can and
/// ends on a sentence or the glyph instead of stopping mid-clause.
#[test]
fn a_short_card_ends_its_note_cleanly() {
    use crate::game::library::{Key, Tab};
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    let row = cobra_row(&rows);
    for (w, h, at_least) in [(57u16, 20u16, 3usize), (80, 24, 6)] {
        let rect = card_rect(w, h);
        let mut ui = LibraryUi::default();
        ui.set_tab(Tab::Bestiary);
        assert!(ui.select_key(
            &rows,
            Key::Boss(crate::game::ecs::systems::boss::BossKind::Cobra)
        ));
        ui.card_open = true;
        let (text, _) = render(&rows, &lib, &mut ui, w, h);
        let note = note_rows(&text, rect, row.dossier);
        assert!(
            note.len() >= at_least,
            "{w}x{h}: only {} note rows: {note:?}",
            note.len()
        );
        let last = note.last().expect("a note row");
        assert!(
            last.ends_with('.') || last.ends_with('\u{2026}'),
            "{w}x{h}: the note stops mid-clause: {last:?}\n{text}"
        );
    }
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

// ---- The animated card -----------------------------------------------------

#[test]
fn a_card_animates_only_after_the_cursor_rests() {
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    let mut ui = LibraryUi::default();
    ui.still = false;
    let key = ui.selected_key(&rows).expect("a selection");
    assert!(catalogued(&lib, key));
    // Nothing on the frame the selection arrives: a card scrolled past is a
    // still plate, never a simulation.
    ui.tick_card(0.0, Some(key), true);
    assert_eq!(ui.scene_key(), None, "the scene was built before the rest");
    // One frame is not a rest either (60 Hz).
    ui.tick_card(1.0 / 60.0, Some(key), true);
    assert_eq!(ui.scene_key(), None);
    // Past the rest window it runs.
    ui.tick_card(scene::REST_SECS, Some(key), true);
    assert_eq!(ui.scene_key(), Some(key), "the card never started");
    assert!(ui.frame_due(), "a running scene is a clock");

    // Moving the cursor drops it, and the next key gets its own rest window.
    let other = rows
        .iter()
        .filter(|r| r.tab == ui.tab)
        .nth(1)
        .map(|r| r.key)
        .expect("a second row");
    ui.tick_card(1.0 / 60.0, Some(other), true);
    assert_ne!(ui.scene_key(), Some(key), "the old card kept running");

    // Closing the screen tears it down.
    ui.tick_card(1.0, Some(other), true);
    ui.stop();
    assert_eq!(ui.scene_key(), None);
}

#[test]
fn a_locked_card_builds_nothing_however_long_it_rests() {
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    let mut ui = LibraryUi::default();
    ui.still = false;
    // An entry that is seen but not owned is still silhouetted: its plate must
    // not run a scene either (a weapon only offered on a card).
    let row = rows
        .iter()
        .find(|r| matches!(r.key, Key::Weapon(_)))
        .expect("a weapon row");
    assert!(lib.is_seen(row.key));
    assert!(!catalogued(&lib, row.key), "an unowned weapon is not catalogued");
    for _ in 0..40 {
        ui.tick_card(0.1, Some(row.key), true);
    }
    assert_eq!(ui.scene_key(), None, "a half-catalogued card ran a scene");
    // A locked entry (live = false) never builds, whatever it is.
    let unseen = rows
        .iter()
        .find(|r| !lib.is_seen(r.key))
        .map(|r| r.key);
    if let Some(unseen) = unseen {
        for _ in 0..40 {
            ui.tick_card(0.1, Some(unseen), false);
        }
        assert_eq!(ui.scene_key(), None, "a locked card ran a scene");
    }
}

#[test]
fn the_still_switch_freezes_every_card() {
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    let mut ui = LibraryUi::default();
    ui.still = true;
    let key = ui.selected_key(&rows).expect("a selection");
    for _ in 0..40 {
        ui.tick_card(0.1, Some(key), true);
    }
    assert_eq!(ui.scene_key(), None, "RATRUN_LIBRARY_STILL built a scene");
    assert!(!ui.frame_due());
    // The still pose's own clock never ran, so the posed art is frozen: two
    // frames of a still card are the same plate, cell for cell.
    assert_eq!(ui.anim, 0.0, "the still pose's clock ran");
    let mut a = PixelBuffer::new(44, 60);
    let mut b = PixelBuffer::new(44, 60);
    let mut scenes = Scenes::default();
    draw_art(&mut a, &mut scenes, key, (150, 190, 176), ui.anim);
    draw_art(&mut b, &mut scenes, key, (150, 190, 176), ui.anim);
    assert_eq!(a.pixels(), b.pixels(), "the still pose moved on its own");
}

#[test]
fn a_running_scene_is_what_the_plate_draws() {
    let rows = all_rows();
    let mut lib = Library::default();
    lib.reveal_all();
    let mut ui = LibraryUi::default();
    ui.still = false;
    let key = ui.selected_key(&rows).expect("a selection");
    for _ in 0..3 {
        ui.tick_card(0.1, Some(key), true);
    }
    assert_eq!(ui.scene_key(), Some(key));
    render(&rows, &lib, &mut ui, 80, 24);
    let first = ui.art.pixels().to_vec();
    assert!(first.iter().any(|p| *p != (0, 0, 0)), "an empty plate");
    // Ten ticks of the real simulation, then a second frame: the still pose is
    // frozen, so only the scene can have moved the pixels.
    {
        let running = ui.running_scene().expect("a running scene");
        for _ in 0..10 {
            running.step();
        }
    }
    render(&rows, &lib, &mut ui, 80, 24);
    assert_ne!(
        ui.art.pixels(),
        first.as_slice(),
        "the plate is the posed art, not the scene"
    );
}
