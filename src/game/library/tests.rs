//! Narrow tests for the Library registry, its discovery bookkeeping and the
//! persisted shape. The screen's own tests live in `render::library`.

use super::*;

/// Every kind of every category has exactly one row, and every row is usable.
#[test]
fn registry_is_complete() {
    let rows = all_rows();
    let mut keys: Vec<(usize, usize)> = rows
        .iter()
        .map(|r| (r.key.category() as usize, r.key.kind_index()))
        .collect();
    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(before, keys.len(), "duplicate rows in the registry");

    // Every rebuildable key must be present, and nothing else.
    for cat in 0..CATEGORY_COUNT {
        for kind in 0..MAX_KINDS {
            let Some(key) = key_at(cat, kind) else {
                continue;
            };
            assert!(
                rows.iter().any(|r| r.key == key),
                "missing registry row for {:?}",
                key
            );
        }
    }
    for row in &rows {
        assert!(!row.name.is_empty(), "{:?} has no name", row.key);
        assert!(!row.hint.is_empty(), "{} has no locked hint", row.name);
        assert!(!row.dossier.is_empty(), "{} has an empty dossier", row.name);
        // The design target is ~320 characters; the test allows headroom for
        // the long-form entries (a boss card) and still catches a paragraph
        // that has grown into an essay.
        assert!(
            row.dossier.chars().count() <= 400,
            "{}'s dossier is {} chars (limit 400)",
            row.name,
            row.dossier.chars().count()
        );
        assert!(!row.group.is_empty(), "{} has no group", row.name);
    }
}

/// A category cannot silently hold more kinds than its bitset.
#[test]
fn no_category_overflows_its_bitset() {
    let rows = all_rows();
    for row in &rows {
        assert!(
            row.key.kind_index() < MAX_KINDS,
            "{:?} indexes past MAX_KINDS",
            row.key
        );
    }
    assert_eq!(WORDS * 64, 64);
}

/// Every tab has rows, and the counts bookkeeping agrees with the rows.
#[test]
fn tabs_have_rows_and_counts_start_locked() {
    let rows = all_rows();
    let lib = Library::default();
    for tab in Tab::ALL {
        let (seen, total) = lib.counts(&rows, tab);
        assert!(total > 0, "{:?} has no rows", tab);
        assert_eq!(seen, 0, "{:?} starts with discoveries", tab);
        assert!(
            rows.iter().filter(|r| r.tab == tab).count() == total,
            "count total disagrees with the tab's rows"
        );
    }
    // Tab stepping wraps both ways.
    assert_eq!(Tab::Bestiary.step(-1), Tab::Floors);
    assert_eq!(Tab::Floors.step(1), Tab::Bestiary);
    assert_eq!(Tab::Arsenal.step(1), Tab::FieldGuide);
}

/// The facts a card prints come from the game's tables, not from the text.
#[test]
fn facts_match_the_constants() {
    let pillbug = facts(Key::Enemy(EnemyKind::Pillbug));
    let hp = pillbug
        .rows()
        .iter()
        .find(|f| f.label == "HIT POINTS")
        .expect("an HP fact");
    assert_eq!(hp.value, format!("{:.0}", EnemyKind::Pillbug.base_hp()));

    let porter = facts(Key::Enemy(EnemyKind::Porter));
    let coin = porter
        .rows()
        .iter()
        .find(|f| f.label == "COIN DROP")
        .expect("a coin fact");
    let (chance, gold) = shop::gold_row(EnemyKind::Porter);
    assert_eq!(
        coin.value,
        format!("{:.0}% × {} gold", chance * 100.0, gold)
    );

    // Per-level weapon damage is the real `Weapon::damage` table.
    let gnaw = facts(Key::Weapon(WeaponKind::Gnaw));
    let damage = gnaw
        .rows()
        .iter()
        .find(|f| f.label == "DAMAGE 1→5")
        .expect("a damage table");
    for level in 1..=5u8 {
        let w = Weapon {
            kind: WeaponKind::Gnaw,
            level,
            timer: 0.0,
            evolved: false,
        };
        assert!(
            damage.value.contains(&format!("{:.0}", w.damage())),
            "Gnaw L{} damage missing from {:?}",
            level,
            damage.value
        );
    }

    // Prices and caps are the shop's and the loadout's own numbers.
    let splice = facts(Key::Splice(SpliceKind::CopperNerves));
    let price = splice
        .rows()
        .iter()
        .find(|f| f.label == "PRICE")
        .expect("a price");
    assert!(price
        .value
        .contains(&SpliceKind::CopperNerves.price().to_string()));

    let heel = facts(Key::Passive(PassiveKind::SpringHeel));
    let cap = heel
        .rows()
        .iter()
        .find(|f| f.label == "CAP")
        .expect("a cap");
    assert!(cap
        .value
        .contains(&PassiveKind::SpringHeel.max_stacks().to_string()));

    // A floor names every boss its pool may field (a row each) and its four
    // named minibosses.
    let kitchen = facts(Key::Floor(Theme::Kitchen));
    let bosses: Vec<&str> = kitchen
        .rows()
        .iter()
        .filter(|f| f.label == "BOSS" || f.label == "OR")
        .map(|f| f.value.as_str())
        .collect();
    let pool: Vec<&str> = BossKind::pool_for_theme(Theme::Kitchen)
        .iter()
        .map(|kind| name_of(Key::Boss(*kind)))
        .collect();
    assert_eq!(bosses, pool);
    let named = kitchen
        .rows()
        .iter()
        .filter(|f| f.label == "MINIBOSS")
        .count();
    assert_eq!(named, crate::game::director::MINIBOSS_TIMES.len());
}

/// A boss entry is named the same thing the HUD's boss bar calls it.
#[test]
fn bosses_share_the_huds_names() {
    for theme in Theme::playable() {
        let kind = BossKind::for_theme(theme);
        assert_eq!(
            name_of(Key::Boss(kind)),
            crate::render::hud::boss_codename(theme).0
        );
    }
}

/// Every boss in the pool table has a real card: a name, a hint, a dossier, a
/// pose (checked by the renderer's own match), and the mechanical rows —
/// including `SHARES` (said out loud even when the floor has one boss, so a
/// reader is never left to infer it) and `ANSWER` (the row a pooled boss used
/// to push off the eight-row card).
#[test]
fn every_boss_has_a_whole_card() {
    use crate::game::ecs::systems::boss::BossKind;
    let rows = all_rows();
    for kind in BossKind::all() {
        let key = Key::Boss(kind);
        let row = rows
            .iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("{} has no card", kind.name()));
        assert!(!row.name.is_empty() && !row.hint.is_empty(), "{:?}", kind);
        assert!(
            !row.dossier.is_empty() && row.dossier.chars().count() > 80,
            "{} has no dossier",
            row.name
        );
        let facts = facts(key);
        for label in ["HIT POINTS", "FLOOR", "SHARES", "DROPS", "ANSWER"] {
            let fact = facts
                .rows()
                .iter()
                .find(|f| f.label == label)
                .unwrap_or_else(|| panic!("{} has no {label} row", row.name));
            assert!(!fact.value.trim().is_empty(), "{}: empty {label}", row.name);
        }
        // A pooled floor names its mate; a single-boss floor says so.
        let pool = BossKind::pool_for_theme(boss_theme(kind));
        let shares = facts
            .rows()
            .iter()
            .find(|f| f.label == "SHARES")
            .map(|f| f.value.clone())
            .unwrap_or_default();
        if pool.len() > 1 {
            for other in pool.iter().filter(|o| **o != kind) {
                assert!(
                    shares.contains(boss_name(*other)),
                    "{}: SHARES does not name {}: {shares}",
                    row.name,
                    boss_name(*other)
                );
            }
        } else {
            assert!(
                shares.contains("only boss"),
                "{}: SHARES should say the floor has one fight: {shares}",
                row.name
            );
        }
    }
    // The row budget covers the widest card, or `Facts::push` would silently
    // drop the last line of it.
    assert!(FACT_ROWS >= facts(Key::Boss(BossKind::Cat)).rows().len());
    assert!(FACT_ROWS >= facts(Key::Boss(BossKind::Mangle)).rows().len());
}

/// An evolution entry exists exactly for the weapons that have one.
#[test]
fn evolution_rows_track_the_evolution_table() {
    let rows = all_rows();
    for kind in WeaponKind::all() {
        let listed = rows.iter().any(|r| r.key == Key::Evolution(kind));
        assert_eq!(
            listed,
            kind.evolution().is_some(),
            "{}'s evolution row disagrees with the table",
            kind.name()
        );
        if let Some(evo) = kind.evolution() {
            assert_eq!(name_of(Key::Evolution(kind)), evo.name);
        }
    }
}

/// Discovery: first sight marks NEW, viewing clears it, counters are per floor.
#[test]
fn discovery_marks_news_and_counts_floors() {
    let mut lib = Library::default();
    let key = Key::Enemy(EnemyKind::Pillbug);
    assert!(!lib.is_seen(key));
    assert!(lib.mark_seen(key), "the first sighting is a discovery");
    assert!(lib.is_seen(key));
    assert!(lib.is_new(key), "a fresh discovery is marked NEW");
    lib.view(key);
    assert!(!lib.is_new(key), "viewing clears the NEW mark");

    // A second frame on the same floor does not count another sighting…
    assert!(!lib.mark_seen(key));
    assert_eq!(lib.record(key).seen, 1);
    // …but a new floor does.
    lib.begin_floor();
    assert!(!lib.mark_seen(key), "already discovered");
    assert_eq!(lib.record(key).seen, 2);

    // The news buffer carries each discovery exactly once.
    let mut out = Vec::new();
    lib.drain_news(&mut out);
    assert_eq!(out, vec![key]);
    out.clear();
    lib.drain_news(&mut out);
    assert!(out.is_empty(), "the discovery was drained twice");
    lib.begin_floor();
    lib.mark_seen(Key::Enemy(EnemyKind::WickMoth));
    lib.mark_seen(Key::Enemy(EnemyKind::WickMoth));
    lib.drain_news(&mut out);
    assert_eq!(out, vec![Key::Enemy(EnemyKind::WickMoth)]);
    out.clear();
    lib.drain_news(&mut out);
    assert!(out.is_empty());
}

/// Timed counters are independent per kind and saturating.
#[test]
fn counters_are_per_kind_and_saturating() {
    let mut lib = Library::default();
    let a = Key::Enemy(EnemyKind::Bat);
    let b = Key::Enemy(EnemyKind::Cat);
    lib.bump(a, Counter::Killed, 3);
    lib.bump(a, Counter::KilledBy, 1);
    lib.bump(b, Counter::Killed, 7);
    assert_eq!(lib.record(a).killed, 3);
    assert_eq!(lib.record(a).killed_by, 1);
    assert_eq!(lib.record(b).killed, 7);
    assert_eq!(lib.record(b).killed_by, 0);
    lib.bump(a, Counter::Killed, u32::MAX);
    assert_eq!(lib.record(a).killed, u32::MAX);
}

/// The persisted shape round-trips and a missing/empty file loads.
#[test]
fn save_round_trips_and_old_files_load() {
    let mut lib = Library::default();
    lib.mark_seen(Key::Enemy(EnemyKind::Turnkey));
    lib.mark_seen(Key::Floor(Theme::Printworks));
    lib.bump(Key::Enemy(EnemyKind::Turnkey), Counter::Killed, 4);
    lib.bump(Key::Weapon(WeaponKind::LaserEyes), Counter::Owned, 1);

    let save: LibrarySave = lib.clone().into();
    let json = serde_json::to_string(&save).unwrap();
    let back: LibrarySave = serde_json::from_str(&json).unwrap();
    let loaded = Library::from(back.clone());
    assert!(loaded.is_seen(Key::Enemy(EnemyKind::Turnkey)));
    assert!(loaded.is_seen(Key::Floor(Theme::Printworks)));
    assert!(!loaded.is_seen(Key::Enemy(EnemyKind::Bat)));
    assert_eq!(loaded.record(Key::Enemy(EnemyKind::Turnkey)).killed, 4);
    assert_eq!(loaded.record(Key::Weapon(WeaponKind::LaserEyes)).owned, 1);
    // NEW marks are session state, never persisted.
    assert!(!loaded.is_new(Key::Enemy(EnemyKind::Turnkey)));

    // A fresh save is tiny: only the seen bitset, no counter rows.
    let fresh: LibrarySave = Library::default().into();
    assert!(fresh.stats.is_empty());
    // And an old (empty) object loads as an empty library.
    let old: LibrarySave = serde_json::from_str("{}").unwrap();
    let old = Library::from(old);
    assert!(!old.is_seen(Key::Enemy(EnemyKind::Skitterer)));
    // Out-of-range rows in a hand-edited file are ignored, not fatal.
    let junk: LibrarySave =
        serde_json::from_str(r#"{"stats":[{"category":250,"kind":250,"seen":1}]}"#).unwrap();
    let _ = Library::from(junk);
}

/// `RATRUN_LIBRARY_ALL` reveals everything for the session.
#[test]
fn reveal_all_unlocks_every_row() {
    let mut lib = Library::default();
    lib.reveal_all();
    let rows = all_rows();
    for row in &rows {
        assert!(lib.is_seen(row.key), "{:?} stayed locked", row.key);
    }
    for tab in Tab::ALL {
        let (seen, total) = lib.counts(&rows, tab);
        assert_eq!(seen, total, "{:?} is not fully revealed", tab);
    }
}

/// Splits and hint text never name the entry while it is locked: the screen
/// asserts the same thing, and this pins the registry half of it (a locked row
/// renders `???` and `Row::hint`, so the hint must not contain the name).
#[test]
fn locked_hints_do_not_leak_the_name() {
    for row in all_rows() {
        let lower = row.hint.to_lowercase();
        let name = row.name.to_lowercase();
        assert!(
            !lower.contains(&name),
            "{}'s locked hint contains its own name: {:?}",
            row.name,
            row.hint
        );
    }
}

/// The survey is read-only: it consumes no simulation rng, and a second sweep
/// over an unchanged frame discovers nothing new.
#[test]
fn survey_draws_no_rng_and_is_idempotent() {
    use crate::game::world::World;
    use rand::RngExt;

    let script = |w: &mut World| {
        for tick in 0..180 {
            let dir = if tick % 40 < 20 {
                Vec2::new(1.0, 0.0)
            } else {
                Vec2::new(0.0, 1.0)
            };
            w.update_with(1.0 / 60.0, dir, false, false);
        }
    };
    let mut watched = World::new(Theme::Sewer, 7);
    let mut twin = World::new(Theme::Sewer, 7);
    script(&mut watched);
    script(&mut twin);

    let mut lib = Library::default();
    lib.begin_floor();
    lib.survey(&watched);
    let after_first = serde_json::to_string(&LibrarySave::from(lib.clone())).unwrap();
    lib.survey(&watched);
    assert_eq!(
        serde_json::to_string(&LibrarySave::from(lib.clone())).unwrap(),
        after_first,
        "a second sweep over the same frame changed the library"
    );

    // Same seed, same script, one survey: the rng streams still agree.
    let a = watched.rng.random_range(0..1_000_000u32);
    let b = twin.rng.random_range(0..1_000_000u32);
    assert_eq!(a, b, "the survey consumed simulation rng");
}

/// A discovered enemy is one that was on screen inside the rat's sight; a body
/// left far away stays locked.
#[test]
fn enemies_are_discovered_only_in_sight() {
    use crate::game::world::World;
    use rand::SeedableRng;

    let mut w = World::new(Theme::Sewer, 3);
    // A body beside the rat: discovered on the first survey.
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let p = w.player.pos;
    w.spawn_enemy_with_rng(
        EnemyKind::Skitterer,
        p + Vec2::new(12.0, 0.0),
        1.0,
        &mut rng,
    );
    // A body on the far side of the map: not discovered.
    w.spawn_enemy_with_rng(EnemyKind::Brute, p + Vec2::new(900.0, 900.0), 1.0, &mut rng);
    for _ in 0..4 {
        w.update_with(1.0 / 60.0, Vec2::ZERO, false, false);
    }
    let mut lib = Library::default();
    lib.begin_floor();
    lib.survey(&w);
    assert!(lib.is_seen(Key::Enemy(EnemyKind::Skitterer)));
    assert!(
        !lib.is_seen(Key::Enemy(EnemyKind::Brute)),
        "an off-screen body was discovered"
    );
}

/// Every wave-four registration has a whole Bestiary card: one row, a locked
/// hint that does not leak its own name, an answer line, and fact rows read
/// from its own stat/gold tables (never its archetype's).
#[test]
fn every_wave_four_registration_has_a_card() {
    let rows = all_rows();
    let mut registered = 0;
    for kind in EnemyKind::all().into_iter().filter(|k| k.is_registration()) {
        registered += 1;
        let row = rows
            .iter()
            .find(|r| r.key == Key::Enemy(kind))
            .unwrap_or_else(|| panic!("{kind:?} has no Bestiary row"));
        assert!(!row.name.is_empty(), "{kind:?} has no display name");
        assert!(!row.hint.is_empty(), "{} has no locked hint", row.name);
        assert!(!row.dossier.is_empty(), "{} has an empty dossier", row.name);
        assert!(
            !row.hint.to_lowercase().contains(&row.name.to_lowercase()),
            "{}'s locked hint leaks its name: {:?}",
            row.name,
            row.hint
        );
        assert!(
            !enemy_answer(kind).trim().is_empty(),
            "{kind:?} has no answer line"
        );
        // The roster carries it and it is on exactly one floor.
        assert!(
            enemy_floors(kind).is_some(),
            "{kind:?} is on no floor's roster"
        );

        let facts = facts(Key::Enemy(kind));
        let hp = facts
            .rows()
            .iter()
            .find(|f| f.label == "HIT POINTS")
            .expect("an HP fact");
        assert_eq!(hp.value, format!("{:.0}", kind.base_hp()), "{kind:?}");
        let coin = facts
            .rows()
            .iter()
            .find(|f| f.label == "COIN DROP")
            .expect("a coin fact");
        let (chance, gold) = shop::gold_row(kind);
        assert_eq!(
            coin.value,
            format!("{:.0}% × {} gold", chance * 100.0, gold),
            "{kind:?}"
        );
        let tell = facts
            .rows()
            .iter()
            .find(|f| f.label == "TELL")
            .expect("a tell fact");
        assert_eq!(tell.value, enemy_answer(kind), "{kind:?}");
    }
    assert_eq!(registered, 33, "the batch registers 33 ordinary kinds");
}

/// Each category's kinds index `0..category_len` exactly once. The survey's
/// "everything here is known" early-outs mask the seen bitset with that
/// length, so a gap or a short list would leave an early-out dead (or fire it
/// with an entry still locked).
#[test]
fn every_category_is_a_contiguous_range() {
    for (c, cat) in Category::ALL.into_iter().enumerate() {
        let mut indexes: Vec<usize> = (0..MAX_KINDS)
            .filter_map(|kind| key_at(c, kind))
            .map(|key| {
                assert_eq!(key.category(), cat);
                key.kind_index()
            })
            .collect();
        indexes.sort_unstable();
        let expected: Vec<usize> = (0..category_len(cat)).collect();
        assert_eq!(indexes, expected, "{cat:?} is not 0..len");
    }
}

/// The early-outs can actually fire: a fully revealed library reports no
/// category as holding a locked entry, and one missing kind reopens its own.
#[test]
fn a_complete_category_reports_nothing_undiscovered() {
    let mut lib = Library::default();
    for cat in Category::ALL {
        assert!(lib.any_undiscovered(cat), "{cat:?} starts complete");
    }
    lib.reveal_all();
    for cat in Category::ALL {
        assert!(!lib.any_undiscovered(cat), "{cat:?} never completes");
    }
    let mut partial = Library::default();
    for kind in PICKUP_KINDS.into_iter().skip(1) {
        partial.mark_seen(Key::Pickup(kind));
    }
    assert!(partial.any_undiscovered(Category::Pickup));
    partial.mark_seen(Key::Pickup(PICKUP_KINDS[0]));
    assert!(!partial.any_undiscovered(Category::Pickup));
}

/// A weapon offered on a card and only then taken still becomes a full entry:
/// `OWNED` is booked once per floor the build carries it, not on the first
/// sighting (which the offer already used up). The same for an evolution,
/// whose own card reads its own record.
#[test]
fn gear_offered_first_is_still_booked_as_owned() {
    use crate::game::loadout::Upgrade;
    use crate::game::world::World;

    let kind = WeaponKind::CheeseSpray;
    let mut w = World::new(Theme::Sewer, 5);
    let mut lib = Library::default();
    lib.begin_floor();
    lib.note_offered(&Upgrade::NewWeapon(kind));
    assert!(lib.is_seen(Key::Weapon(kind)));
    assert_eq!(lib.record(Key::Weapon(kind)).owned, 0, "offered is not owned");

    w.player.loadout.weapons.retain(|held| held.kind != kind);
    w.player.loadout.weapons.push(Weapon::new(kind));
    lib.survey(&w);
    lib.survey(&w);
    assert_eq!(lib.record(Key::Weapon(kind)).owned, 1, "once per floor");

    // Evolved: booked once, on the weapon's card and on the evolution's own.
    for held in w.player.loadout.weapons.iter_mut() {
        if held.kind == kind {
            held.evolved = true;
        }
    }
    for _ in 0..3 {
        lib.survey(&w);
    }
    assert!(lib.is_seen(Key::Evolution(kind)));
    assert_eq!(lib.record(Key::Weapon(kind)).evolved, 1, "not once per frame");
    assert_eq!(lib.record(Key::Evolution(kind)).evolved, 1);

    // The next floor carries both: one more of each.
    lib.begin_floor();
    lib.survey(&w);
    lib.survey(&w);
    assert_eq!(lib.record(Key::Weapon(kind)).owned, 2);
    assert_eq!(lib.record(Key::Evolution(kind)).evolved, 2);
}

/// An ordinary enemy's locked hint names the floor and the clock its roster
/// slot really keeps (the hints are authored strings; this pins them to
/// `roster` so a schedule change cannot leave them behind).
#[test]
fn enemy_hints_agree_with_the_roster() {
    for row in all_rows() {
        let Key::Enemy(kind) = row.key else {
            continue;
        };
        let Some(home) = enemy_floors(kind) else {
            continue;
        };
        let (floor, clock) = home.split_once(" · ").expect("floor · clock");
        assert!(
            row.hint.starts_with(floor),
            "{}: hint {:?} is not on {floor}",
            row.name,
            row.hint
        );
        assert!(
            row.hint.ends_with(clock),
            "{}: hint {:?} does not say {clock:?}",
            row.name,
            row.hint
        );
    }
}

/// The weapon card does not claim the classic "+25%, 6% faster" curve for a
/// weapon that ships its own per-level table.
#[test]
fn each_level_row_matches_the_weapons_curve() {
    for kind in WeaponKind::all() {
        let facts = facts(Key::Weapon(kind));
        let each = facts
            .rows()
            .iter()
            .find(|f| f.label == "EACH LEVEL")
            .expect("an EACH LEVEL row");
        assert_eq!(
            each.value.contains("+25% damage"),
            kind.damage_by_level().is_none(),
            "{}: {}",
            kind.name(),
            each.value
        );
        assert_eq!(
            each.value.contains("6% faster"),
            kind.cooldown_by_level().is_none(),
            "{}: {}",
            kind.name(),
            each.value
        );
    }
}

/// No fact value is wider than the card gives it at 80×24
/// ([`FACT_VALUE_COLS`], less the bar where the row draws one): the screen
/// clips a longer value mid-word. This walks every row of the registry —
/// enemies' `TELL`, the bosses' `ANSWER` and `SHARES`, the minibosses, the
/// gear and the floors — so a new entry cannot ship a line that does not fit.
#[test]
fn every_fact_value_fits_its_row() {
    for row in all_rows() {
        let facts = facts(row.key);
        assert!(
            !facts.rows().is_empty(),
            "{} has no facts at all",
            row.name
        );
        for fact in facts.rows() {
            let room = if fact.bar >= 0.0 {
                FACT_VALUE_COLS - FACT_BAR_COLS
            } else {
                FACT_VALUE_COLS
            };
            let cols = fact.value.chars().count();
            assert!(
                cols <= room,
                "{} · {}: {:?} is {cols} columns (room for {room})",
                row.name,
                fact.label,
                fact.value
            );
        }
    }
    // The floor card is the fullest: its rows must all be on the card.
    for theme in Theme::playable() {
        let floor = facts(Key::Floor(theme));
        assert!(
            floor.rows().iter().any(|f| f.label == "MAZES"),
            "{}'s card ran out of rows before its last fact",
            theme.name()
        );
    }
}

/// The tell line was cut to one row, so the dossier is where the designed
/// rule lives: every wave-four kind's note is longer than its tell and no
/// tell is a whole sentence carried over from the design table.
#[test]
fn wave_four_tells_are_short_and_the_dossier_keeps_the_rule() {
    let rows = all_rows();
    for kind in EnemyKind::all().into_iter().filter(|k| k.is_registration()) {
        let tell = enemy_answer(kind);
        assert!(tell.chars().count() <= FACT_VALUE_COLS, "{kind:?}: {tell:?}");
        let row = rows
            .iter()
            .find(|r| r.key == Key::Enemy(kind))
            .expect("a Bestiary row");
        assert!(
            row.dossier.chars().count() > tell.chars().count() * 2,
            "{}'s dossier does not carry the rule its tell was cut from",
            row.name
        );
    }
}

/// Thirteen evolutions, not twelve: Spiked Tail evolves into Barbed Wire, and
/// the Arsenal carries a row and a full card for it.
#[test]
fn barbed_wire_is_the_thirteenth_evolution() {
    let rows = all_rows();
    let evolutions: Vec<&Row> = rows
        .iter()
        .filter(|r| matches!(r.key, Key::Evolution(_)))
        .collect();
    assert_eq!(evolutions.len(), 13);
    assert_eq!(
        evolutions.len(),
        WeaponKind::all()
            .into_iter()
            .filter(|k| k.evolution().is_some())
            .count()
    );
    let wire = evolutions
        .iter()
        .find(|r| r.key == Key::Evolution(WeaponKind::SpikedTail))
        .expect("Spiked Tail's evolution row");
    assert_eq!(wire.name, "Barbed Wire");
    assert!(wire.hint.contains(WeaponKind::SpikedTail.name()));
    let needs = facts(wire.key);
    let needs = needs
        .rows()
        .iter()
        .find(|f| f.label == "NEEDS")
        .expect("a NEEDS row");
    assert!(needs.value.contains(PassiveKind::Counterweight.name()));
    // Counterweight partners two weapons, and its card names both.
    let partner = facts(Key::Passive(PassiveKind::Counterweight));
    let evolves = partner
        .rows()
        .iter()
        .find(|f| f.label == "EVOLVES")
        .expect("an EVOLVES row");
    assert!(evolves.value.contains(WeaponKind::TailThrash.name()));
    assert!(evolves.value.contains(WeaponKind::SpikedTail.name()));
}
