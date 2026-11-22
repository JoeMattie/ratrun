//! The card scenes' own tests: every registry row answers (a scene or a stated
//! reason), every scene survives two loops with a living rat, and a sample of
//! plates really moves — the subject's attack included.
//!
//! The animating plate is deliberately sampled, not proven per entry: the
//! registry is content, and a new kind's *behaviour* is the simulation's to
//! test. What is exhaustive here is the plan (a row cannot ship without an
//! answer), the two-loop survival of everything that runs, and the card's two
//! promises: **a bestiary plate shows no rat** (the render flag is what takes
//! it out), and **the subject stays in frame** for most of the loop.

use super::*;
use crate::game::bullet::{Bullet, Faction};
use crate::game::ecs::systems::boss::BossState;
use crate::game::ecs::systems::miniboss::{Miniboss, MinibossState};
use crate::game::hazards::SteamPhase;
use crate::game::library::all_rows;
use crate::game::weapon::WeaponKind;

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

// ---- Shape ----------------------------------------------------------------

/// Every registry row resolves to a scene or to a named still plate, and the
/// two never disagree: a `Sandbox` entry builds, a `Still` one does not.
#[test]
fn every_registry_row_has_a_scene_or_a_reason() {
    for row in all_rows() {
        match plan(row.key) {
            Plan::Sandbox => assert!(
                LibraryScene::build(row.key).is_some(),
                "{} is planned as a scene and builds nothing",
                row.name
            ),
            Plan::Still(why) => {
                assert!(!why.is_empty(), "{} has a still plate with no reason", row.name);
                assert!(
                    LibraryScene::build(row.key).is_none(),
                    "{} claims a still plate and runs a scene",
                    row.name
                );
            }
        }
    }
}

/// Every scene builds, then runs two whole loops: no panic, no game over, and
/// the card rat still alive at every step (hidden or not: it is the real rat).
#[test]
fn every_scene_runs_two_loops_and_the_rat_survives() {
    for row in all_rows() {
        if plan(row.key) != Plan::Sandbox {
            continue;
        }
        let mut scene = LibraryScene::build(row.key)
            .unwrap_or_else(|| panic!("{} has no scene", row.name));
        let ticks = (scene.loop_secs() / TICK).ceil() as u32 * 3 + 8;
        for _ in 0..ticks {
            scene.step();
            let w = scene.world();
            assert!(!w.game_over, "{} ended the scene's run", row.name);
            assert!(w.player.hp > 0.0, "{} killed the card rat", row.name);
            assert!(w.player.alive(), "{} killed the card rat", row.name);
        }
        assert!(
            scene.loops() >= 2,
            "{} did not loop twice in {ticks} ticks",
            row.name
        );
    }
}

// ---- Sampling helpers -----------------------------------------------------

/// Every bestiary entry: the ordinary bodies, the named fights, the bosses and
/// the Elite card. The one still bestiary row (the legacy `Boss` body) is not
/// a scene and is skipped by the plan, not by a special case here.
fn bestiary_keys() -> Vec<Key> {
    let mut keys = Vec::new();
    for row in all_rows() {
        if matches!(
            row.key,
            Key::Enemy(_) | Key::Miniboss(_) | Key::Boss(_) | Key::Object(ObjectKind::Elite)
        ) && plan(row.key) == Plan::Sandbox
        {
            keys.push(row.key);
        }
    }
    keys
}

/// Step one whole loop, calling `f` after every tick (before the rebuild).
fn one_loop(scene: &mut LibraryScene, mut f: impl FnMut(&mut LibraryScene)) {
    let ticks = (scene.loop_secs() / TICK).ceil() as u32 + 3;
    let loops = scene.loops();
    for _ in 0..ticks {
        scene.step();
        if scene.loops() != loops {
            return;
        }
        f(scene);
    }
}

/// The plate as it is drawn now (the real `World::draw` at the card's size).
fn plate(scene: &mut LibraryScene, art: &mut PixelBuffer) {
    art.clear((0, 0, 0));
    scene.draw(art);
}

/// The plate after 20 ticks is not the plate before them.
fn plate_moves(key: Key) -> bool {
    let (w, h) = (44, 60);
    let mut scene = LibraryScene::build(key).expect("a scene for the key");
    let mut a = PixelBuffer::new(w, h);
    let mut b = PixelBuffer::new(w, h);
    plate(&mut scene, &mut a);
    for _ in 0..20 {
        scene.step();
    }
    plate(&mut scene, &mut b);
    a.pixels() != b.pixels()
}

/// Hostile shots in flight right now.
fn hostile_shots(scene: &mut LibraryScene) -> usize {
    let w = scene.world();
    let mut q = w.ecs.query::<&Bullet>();
    q.iter(&w.ecs)
        .filter(|b| b.faction == Faction::Enemy)
        .count()
}

/// Where the hostile shots in flight are right now.
fn hostile_shot_points(scene: &mut LibraryScene) -> Vec<Vec2> {
    let w = scene.world();
    let mut q = w.ecs.query::<&Bullet>();
    q.iter(&w.ecs)
        .filter(|b| b.faction == Faction::Enemy)
        .map(|b| b.pos)
        .collect()
}

/// Is any body in the scene hurt (a weapon's own proof that it connected)?
fn a_body_is_hurt(scene: &mut LibraryScene) -> bool {
    let w = scene.world();
    let mut q = w.ecs.query::<&Enemy>();
    // A max-level evolution kills a dummy outright, so a kill counts too.
    w.kills > 0 || q.iter(&w.ecs).any(|e| e.hp < e.max_hp - 0.01)
}

/// Was `p` on the plate the last draw covered?
fn on_plate(scene: &LibraryScene, p: Vec2) -> bool {
    let cam = scene.world.cam;
    let v = scene.view;
    p.x >= cam.x - 0.5
        && p.x <= cam.x + v.x + 0.5
        && p.y >= cam.y - 0.5
        && p.y <= cam.y + v.y + 0.5
}

/// Does the plate hold this exact colour?
fn has_colour(pb: &PixelBuffer, c: Rgb) -> bool {
    pb.pixels().contains(&c)
}

/// The ink pass's rat core, and only the ink pass's rat core: two exact
/// colours no other drawing code in the game produces.
const RAT_INK: [Rgb; 2] = [(255, 240, 244), (255, 196, 210)];

fn has_rat_ink(pb: &PixelBuffer) -> bool {
    pb.pixels().iter().any(|p| RAT_INK.contains(p))
}

// ---- A portrait of its subject --------------------------------------------

/// No bestiary plate draws a rat. The render flag is the mechanism (the
/// simulation still needs the target), and the same plate with the flag off
/// *does* differ, so this is a real assertion and not a plate that happens to
/// hide the rat behind its own dummy.
#[test]
fn bestiary_plates_show_no_rat() {
    for key in bestiary_keys() {
        let mut scene = LibraryScene::build(key).unwrap_or_else(|| panic!("{key:?} has no scene"));
        assert!(scene.world.hide_rat, "{key:?} would draw the card's rat");
        let mut art = PixelBuffer::new(44, 30);
        plate(&mut scene, &mut art);
        assert!(!has_rat_ink(&art), "{key:?} drew the ink pass's rat mark");
        // The comparison is made without the card's dummy: at a shooter's zoom
        // the cork covers the whole rat, and the proof is about the world pass.
        // (Half a second in, so the camera has eased onto the plate's lane.)
        for _ in 0..30 {
            scene.step();
        }
        let dummy = scene.script.dummy;
        scene.script.dummy = false;
        plate(&mut scene, &mut art);
        let hidden = art.pixels().to_vec();
        scene.world.hide_rat = false;
        plate(&mut scene, &mut art);
        assert!(
            art.pixels() != hidden.as_slice(),
            "{key:?}: hide_rat changes nothing, so the plate is not the proof"
        );
        scene.world.hide_rat = true;
        scene.script.dummy = dummy;
        for _ in 0..20 {
            plate(&mut scene, &mut art);
            assert!(!has_rat_ink(&art), "{key:?} drew the ink pass's rat mark");
            scene.step();
        }
    }
}

/// The subject is on the plate for at least four fifths of the loop: a card
/// that frames a body must not lose it at the edge.
#[test]
fn every_bestiary_subject_stays_on_its_plate() {
    for key in bestiary_keys() {
        let mut scene = LibraryScene::build(key).unwrap_or_else(|| panic!("{key:?} has no scene"));
        let mut art = PixelBuffer::new(44, 30);
        let ticks = (scene.loop_secs() / TICK) as u32;
        let (mut seen, mut on) = (0u32, 0u32);
        for i in 0..ticks {
            if i % 6 == 0 {
                plate(&mut scene, &mut art);
                seen += 1;
                if scene.subject_on_plate() {
                    on += 1;
                }
            }
            scene.step();
        }
        assert!(
            on * 100 >= seen * 80,
            "{key:?}: the subject was on the plate {on} of {seen} samples"
        );
    }
}

/// Every family whose plate is about an attack draws the card's own practice
/// target where the hidden rat stands — and it is the card's own pixels, not
/// an entity: no body of that kind of colour can be among the enemies.
#[test]
fn the_aimed_families_draw_a_practice_dummy() {
    for key in bestiary_keys() {
        let mut scene = LibraryScene::build(key).unwrap_or_else(|| panic!("{key:?} has no scene"));
        if !scene.script.dummy {
            continue;
        }
        let mut art = PixelBuffer::new(44, 30);
        let mut seen = false;
        for _ in 0..40 {
            plate(&mut scene, &mut art);
            seen |= has_colour(&art, CORK) && has_colour(&art, RIND);
            scene.step();
        }
        assert!(seen, "{key:?} draws no practice dummy");
    }
}

/// The four bestiary families do not share one script: a pack is not a
/// shooter, a shooter is not a trick, and a trick is not a heavy.
#[test]
fn the_bestiary_families_run_different_scripts() {
    let mut shapes: Vec<(f32, f32, bool)> = Vec::new();
    for slot in [Slot::A, Slot::C, Slot::R, Slot::I, Slot::H] {
        let key = Key::Enemy(roster::kind(Theme::Sewer, slot));
        let scene = LibraryScene::build(key).expect("a bestiary scene");
        let shape = (scene.script.loop_secs, scene.script.zoom, scene.script.dummy);
        assert!(
            !shapes.contains(&shape),
            "{slot:?} restages another family's script: {shape:?}"
        );
        shapes.push(shape);
    }
    let theme = Theme::Sewer;
    let pack = LibraryScene::build(Key::Enemy(roster::kind(theme, Slot::A))).expect("a pack");
    let shock = LibraryScene::build(Key::Enemy(roster::kind(theme, Slot::I))).expect("a trick");
    assert!(!pack.script.dummy, "a pack attacks nothing, so it has no dummy");
    assert!(shock.script.dummy, "a trick is aimed at a dummy");
    assert!(
        pack.script.cue.is_some(),
        "a pack needs its crossing waypoint"
    );
    assert!(
        pack.subject.is_some() && shock.subject.is_some(),
        "both families are about a body"
    );
}

// ---- The samples ----------------------------------------------------------

/// One `R` (shooter), one `C` (chaser) and one `I` (trick) body per floor: all
/// three plates move, the shooter's shot really leaves inside a loop, and the
/// chaser really runs its lap. (A trick's attack is a warned footprint, so its
/// walk into range *is* the tell; the shot is the one attack a plate can prove
/// from the world.)
#[test]
fn a_shooter_a_chaser_and_a_trick_per_floor_animate() {
    for theme in Theme::playable() {
        for slot in [Slot::R, Slot::C, Slot::I] {
            let key = Key::Enemy(roster::kind(theme, slot));
            assert!(plate_moves(key), "{key:?} never changed its plate");
        }
        let key = Key::Enemy(roster::kind(theme, Slot::R));
        let mut scene = LibraryScene::build(key).expect("a shooter scene");
        let mut peak = 0;
        one_loop(&mut scene, |s| peak = peak.max(hostile_shots(s)));
        assert!(peak > 0, "{key:?} never fired inside one loop");

        let key = Key::Enemy(roster::kind(theme, Slot::C));
        let mut scene = LibraryScene::build(key).expect("a chaser scene");
        let mut first: Option<Vec2> = None;
        let mut far = 0.0f32;
        one_loop(&mut scene, |s| {
            if let Some(p) = s.subject_pos() {
                let start = *first.get_or_insert(p);
                far = far.max(p.dist(start));
            }
        });
        assert!(far > 8.0, "{key:?} never left its spot: {far} px");
    }
}

/// A shooter's shot stays on the plate for its whole flight: the pellet is
/// never alone on empty floor, because the lane is framed with both ends.
#[test]
fn every_shooter_keeps_its_shot_on_the_plate() {
    for theme in Theme::playable() {
        let key = Key::Enemy(roster::kind(theme, Slot::R));
        let mut scene = LibraryScene::build(key).expect("a shooter scene");
        let mut art = PixelBuffer::new(44, 30);
        let (mut shots, mut off) = (0u32, 0u32);
        one_loop(&mut scene, |s| {
            plate(s, &mut art);
            for p in hostile_shot_points(s) {
                shots += 1;
                if !on_plate(s, p) {
                    off += 1;
                }
            }
        });
        assert!(shots > 0, "{key:?} never fired inside one loop");
        assert_eq!(off, 0, "{key:?}: {off} of {shots} shots left the plate");
    }
}

/// The named fights, including the two new shapes (the Mudlark's closing
/// pincer and the Wringer Boy's paired rollers): each reaches its strike.
#[test]
fn the_named_fights_reach_their_strike() {
    for kind in [
        MinibossKind::Corkscrew,
        MinibossKind::Mudlark,
        MinibossKind::WringerBoy,
    ] {
        let key = Key::Miniboss(kind);
        assert!(plate_moves(key), "{kind:?} never changed its plate");
        let mut scene = LibraryScene::build(key).expect("a miniboss scene");
        let subject = scene.subject().expect("a miniboss body");
        let mut struck = false;
        one_loop(&mut scene, |s| {
            let w = s.world();
            if let Some(mb) = w.ecs.get::<Miniboss>(subject) {
                struck |= mb.state == MinibossState::Active;
            }
        });
        assert!(struck, "{kind:?} never reached its signature");
    }
}

/// Three bosses run their real attack states (the controller's own `BossState`,
/// so a plate cannot show a pose the fight never takes).
#[test]
fn the_bosses_run_their_attack_states() {
    for kind in [BossKind::Cat, BossKind::Mangle, BossKind::Cobra] {
        let key = Key::Boss(kind);
        assert!(plate_moves(key), "{kind:?} never changed its plate");
        let mut scene = LibraryScene::build(key).expect("a boss scene");
        let mut states: Vec<BossState> = Vec::new();
        one_loop(&mut scene, |s| {
            let w = s.world();
            let mut q = w.ecs.query::<&Boss>();
            if let Some(b) = q.iter(&w.ecs).next() {
                if states.last() != Some(&b.state) {
                    states.push(b.state);
                }
            }
        });
        assert!(
            states.len() >= 2,
            "{kind:?} held one boss state for the whole loop: {states:?}"
        );
    }
}

/// Six weapons hit the standing dummies the scene keeps in front of the rat.
#[test]
fn the_weapons_fire_at_the_dummies() {
    for kind in [
        WeaponKind::Gnaw,
        WeaponKind::CheeseSpray,
        WeaponKind::SqueakNova,
        WeaponKind::TailWhip,
        WeaponKind::AcidSpit,
        WeaponKind::LaserEyes,
    ] {
        let key = Key::Weapon(kind);
        assert!(plate_moves(key), "{key:?} never changed its plate");
        let mut scene = LibraryScene::build(key).expect("a weapon scene");
        let mut hit = false;
        one_loop(&mut scene, |s| hit |= a_body_is_hurt(s));
        assert!(hit, "{kind:?} never connected inside one loop");
    }
}

/// Three evolutions behave, not just draw.
#[test]
fn the_evolutions_run() {
    for kind in [
        WeaponKind::Gnaw,
        WeaponKind::CheeseSpray,
        WeaponKind::SqueakNova,
    ] {
        let key = Key::Evolution(kind);
        assert!(plate_moves(key), "{key:?} never changed its plate");
        let mut scene = LibraryScene::build(key).expect("an evolution scene");
        let mut hit = false;
        one_loop(&mut scene, |s| hit |= a_body_is_hurt(s));
        assert!(hit, "{kind:?}'s evolution never connected inside one loop");
    }
}

/// The field guide separates what a body springs from what the rat is the
/// point of: a glue board, a mousetrap, a steam grate and an ink pad are shown
/// being sprung by fodder (the rat is not drawn); a pit, a rat-hole and the
/// sliding cargo are about the rat, so it stays.
#[test]
fn the_traps_are_sprung_by_fodder_not_the_rat() {
    for kind in [
        ObjectKind::StickyBoard,
        ObjectKind::Mousetrap,
        ObjectKind::SteamGrate,
        ObjectKind::InkPad,
        ObjectKind::Slipstream,
    ] {
        let scene = LibraryScene::build(Key::Object(kind)).expect("a hazard scene");
        assert!(scene.world.hide_rat, "{kind:?} shows the rat as its subject");
    }
    for kind in [
        ObjectKind::Pit,
        ObjectKind::RatHole,
        ObjectKind::SlidingCargo,
        ObjectKind::Gold,
        ObjectKind::Chest,
        ObjectKind::MazeCache,
        ObjectKind::ShopHole,
        ObjectKind::OneWayDoor,
    ] {
        let scene = LibraryScene::build(Key::Object(kind)).expect("a rat scene");
        assert!(!scene.world.hide_rat, "{kind:?} hid the rat it is about");
    }
}

/// Both traps run their real state machines: the glue board catches a body and
/// wears out to `Used`, the mousetrap walks Armed → Triggered → Snapping →
/// Sprung.
#[test]
fn both_traps_spring() {
    use crate::game::art_traps::{MousetrapState, StickyTrapState};

    let mut scene = LibraryScene::build(Key::Object(ObjectKind::StickyBoard)).expect("a board");
    let mut caught = false;
    let mut used = false;
    for _ in 0..(scene.loop_secs() / TICK) as u32 {
        scene.step();
        let w = scene.world();
        for trap in &w.level.objects.sticky_traps {
            caught |= trap.state == StickyTrapState::Stuck;
            used |= trap.state == StickyTrapState::Used;
        }
    }
    assert!(caught, "the glue board never caught a body");
    assert!(used, "the glue board never wore out");

    let mut scene = LibraryScene::build(Key::Object(ObjectKind::Mousetrap)).expect("a trap");
    let mut seen: Vec<MousetrapState> = Vec::new();
    for _ in 0..(scene.loop_secs() / TICK) as u32 {
        scene.step();
        let w = scene.world();
        if let Some(trap) = w.level.objects.mousetraps.first() {
            if seen.last() != Some(&trap.state) {
                seen.push(trap.state);
            }
        }
    }
    assert_eq!(
        seen,
        vec![
            MousetrapState::Armed,
            MousetrapState::Triggered,
            MousetrapState::Snapping,
            MousetrapState::Sprung,
        ],
        "the mousetrap's cycle is not the art's cycle"
    );
}

/// The steam grate vents inside one loop (resting → warning → plume).
#[test]
fn the_steam_grate_cycles() {
    let mut scene = LibraryScene::build(Key::Object(ObjectKind::SteamGrate)).expect("a grate");
    let mut phases: Vec<SteamPhase> = Vec::new();
    for _ in 0..(scene.loop_secs() / TICK) as u32 {
        scene.step();
        let w = scene.world();
        if let Some(grate) = w.level.objects.steam_grates.first() {
            if phases.last() != Some(&grate.phase) {
                phases.push(grate.phase);
            }
        }
    }
    assert!(
        phases.contains(&SteamPhase::Plume),
        "the grate never vented: {phases:?}"
    );
}

/// Four pocket finds are walked over and collected (the plate then shows what
/// the pickup did).
#[test]
fn the_pocket_finds_are_collected() {
    for kind in [
        PickupKind::Heal,
        PickupKind::Nuke,
        PickupKind::BottleCap,
        PickupKind::CleanSocks,
    ] {
        let key = Key::Pickup(kind);
        let mut scene = LibraryScene::build(key).expect("a pickup scene");
        let mut present = true;
        one_loop(&mut scene, |s| {
            let w = s.world();
            let mut q = w.ecs.query::<&Pickup>();
            present &= q.iter(&w.ecs).next().is_some();
        });
        assert!(!present, "{kind:?} was never collected");
    }
}

// ---- Filmstrips (captures) ------------------------------------------------

/// One strip: `frames` plates across one loop, side by side.
fn strip(key: Key, w: usize, h: usize, frames: usize) -> Option<PixelBuffer> {
    let mut scene = LibraryScene::build(key)?;
    let gap = 2;
    let mut out = PixelBuffer::new(frames * (w + gap), h);
    out.clear((0, 0, 0));
    let mut art = PixelBuffer::new(w, h);
    let total = (scene.loop_secs() / TICK) as usize;
    let mut next = 0;
    for i in 0..total {
        // Sample the middle of each of `frames` equal spans.
        if next < frames && i == (next * 2 + 1) * total / (frames * 2) {
            scene.draw(&mut art);
            for y in 0..h {
                for x in 0..w {
                    out.plot((next * (w + gap) + x) as i32, y as i32, art.pixels()[y * w + x]);
                }
            }
            next += 1;
        }
        scene.step();
    }
    Some(out)
}

fn write_ansi(dir: &std::path::Path, name: &str, pb: &PixelBuffer) {
    let mut s = String::new();
    let px = pb.pixels();
    for cy in 0..pb.h / 2 {
        for x in 0..pb.w {
            let t = px[cy * 2 * pb.w + x];
            let b = px[(cy * 2 + 1) * pb.w + x];
            s.push_str(&format!(
                "\x1b[38;2;{};{};{};48;2;{};{};{}m▀",
                t.0, t.1, t.2, b.0, b.1, b.2
            ));
        }
        s.push_str("\x1b[0m\n");
    }
    let path = dir.join(format!("{}x{}-{}.ansi", pb.w, pb.h / 2, name));
    std::fs::write(path, s).expect("write the strip");
}

/// `RATRUN_LIBANIM_DIR=<dir> RATRUN_LIBANIM=<names|all|tab> cargo test
/// capture_library_filmstrips`: one `.ansi` strip per card (or, with
/// `RATRUN_LIBANIM_SHEET=n`, sheets of `n` cards stacked) for
/// `scratchpad/cheese/view.py`. A no-op without the directory.
#[test]
fn capture_library_filmstrips() {
    let Ok(dir) = std::env::var("RATRUN_LIBANIM_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("create the capture directory");
    let want = std::env::var("RATRUN_LIBANIM").unwrap_or_else(|_| "all".into());
    let want: Vec<String> = want.split(',').map(slug).collect();
    let frames: usize = std::env::var("RATRUN_LIBANIM_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let sheet: usize = std::env::var("RATRUN_LIBANIM_SHEET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let (w, h) = (44, 30);
    let mut strips: Vec<(String, PixelBuffer)> = Vec::new();
    for row in all_rows() {
        let name = slug(row.name);
        let tab = slug(row.tab.title());
        let group = slug(row.group);
        if !want.iter().any(|x| x == "all" || *x == name || *x == tab || *x == group) {
            continue;
        }
        if let Some(pb) = strip(row.key, w, h, frames) {
            strips.push((name, pb));
        }
    }
    if sheet == 0 {
        for (name, pb) in &strips {
            write_ansi(&dir, name, pb);
        }
        return;
    }
    for (n, chunk) in strips.chunks(sheet).enumerate() {
        let mut out = PixelBuffer::new(chunk[0].1.w, chunk.len() * (h + 2));
        out.clear((0, 0, 0));
        for (i, (_, pb)) in chunk.iter().enumerate() {
            for y in 0..h {
                for x in 0..pb.w {
                    out.plot(x as i32, (i * (h + 2) + y) as i32, pb.pixels()[y * pb.w + x]);
                }
            }
        }
        let names: Vec<&str> = chunk.iter().map(|(n, _)| n.as_str()).collect();
        eprintln!("sheet {n}: {}", names.join(", "));
        write_ansi(&dir, &format!("sheet{n:02}-{}", want[0]), &out);
    }
}

/// `RATRUN_LIBANIM_TIMINGS=1 cargo test library_scene_timings -- --nocapture`:
/// the cost of one card frame (one fixed step plus one plate draw) for a
/// spread of cards, in the profile the test is built with. A no-op otherwise.
#[test]
fn library_scene_timings() {
    if std::env::var("RATRUN_LIBANIM_TIMINGS").is_err() {
        return;
    }
    let keys = [
        Key::Enemy(EnemyKind::Skitterer),
        Key::Miniboss(MinibossKind::Corkscrew),
        Key::Boss(BossKind::Cat),
        Key::Weapon(WeaponKind::AcidSpit),
        Key::Evolution(WeaponKind::SqueakNova),
        Key::Object(ObjectKind::StickyBoard),
    ];
    for key in keys {
        let built = std::time::Instant::now();
        let mut scene = LibraryScene::build(key).expect("a scene");
        let build_us = built.elapsed().as_micros();
        let mut art = PixelBuffer::new(44, 60);
        let frames = 600u32;
        let (mut step_us, mut draw_us) = (0u128, 0u128);
        for _ in 0..frames {
            let t = std::time::Instant::now();
            scene.step();
            step_us += t.elapsed().as_micros();
            let t = std::time::Instant::now();
            plate(&mut scene, &mut art);
            draw_us += t.elapsed().as_micros();
        }
        println!(
            "LIBANIM {key:?}: build {build_us} us, step {} us, draw {} us a frame",
            step_us / u128::from(frames),
            draw_us / u128::from(frames)
        );
    }
}
