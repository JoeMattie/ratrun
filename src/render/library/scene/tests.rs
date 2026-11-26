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
        // The comparison is made without the card's dummy: the cork is drawn at
        // the rat's own point, so taking it out is what makes the proof about
        // the world pass alone. (Half a second in, so the camera has eased onto
        // the plate's lane.)
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

/// The card's dummy is painted **under** the picture. On every frame of every
/// aimed plate the cork changes only pixels the world's own pass left as bare
/// floor, so a body standing on the dummy keeps its own pixels: the Bell Flea's
/// landing crosses, the Wick Moth's burst and the Loose Bungee's hook all end
/// their loop standing on the target, and the plate must still show them.
#[test]
fn the_practice_dummy_never_paints_over_a_body() {
    const W: usize = 44;
    const H: usize = 30;
    for key in bestiary_keys() {
        let mut scene = LibraryScene::build(key).unwrap_or_else(|| panic!("{key:?} has no scene"));
        if !scene.script.dummy {
            continue;
        }
        let mut art = PixelBuffer::new(W, H);
        let mut bare = PixelBuffer::new(W, H);
        // The proof compares two draws of the same frame, so the one layer that
        // is stateful across draws (smoke advances its own particles) is out.
        scene.world.smoke_enabled = false;
        let ticks = (scene.loop_secs() / TICK) as u32;
        let (mut drawn, mut subject_drawn) = (false, 0u32);
        for _ in 0..ticks {
            plate(&mut scene, &mut art);
            let subject = scene.subject_pos();
            // The same frame with the card's own target switched off, and the
            // bare floor the world's pass leaves where nothing stands.
            scene.script.dummy = false;
            plate(&mut scene, &mut bare);
            scene.script.dummy = true;
            let floor = scene.floor_plate(W, H).pixels().to_vec();
            assert_eq!(floor.len(), W * H);
            // The dummy's own disc on the plate, from the same placement rule
            // `draw` uses.
            let scale = scene.world.zoom.max(0.01) * scene.script.shrink.max(1) as f32;
            let cam = scene.world.cam;
            let p = scene.world.player.pos;
            let sx = ((p.x - cam.x) / scale).round() as i32;
            let sy = ((p.y - cam.y) / scale).round() as i32;
            let r = ((DUMMY_R / scale).round() as i32).max(2) + 1;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (x, y) = (sx + dx, sy + dy);
                    if !(0..W as i32).contains(&x) || !(0..H as i32).contains(&y) {
                        continue;
                    }
                    let i = y as usize * W + x as usize;
                    if art.pixels()[i] == bare.pixels()[i] {
                        continue;
                    }
                    drawn = true;
                    assert_eq!(
                        bare.pixels()[i], floor[i],
                        "{key:?}: the dummy painted over a drawn pixel at ({x}, {y})"
                    );
                    assert!(
                        DUMMY_INK.contains(&art.pixels()[i]),
                        "{key:?}: the dummy painted a colour it does not own at ({x}, {y})"
                    );
                }
            }
            // The subject's own point: whenever the world drew something there,
            // the cork left it alone.
            if let Some(p) = subject {
                let scale = scene.view.x / W as f32;
                let x = ((p.x - scene.world.cam.x) / scale).round() as i32;
                let y = ((p.y - scene.world.cam.y) / scale).round() as i32;
                if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
                    let i = y as usize * W + x as usize;
                    if bare.pixels()[i] != floor[i] {
                        subject_drawn += 1;
                        assert_eq!(
                            art.pixels()[i], bare.pixels()[i],
                            "{key:?}: the dummy covered the subject at ({x}, {y})"
                        );
                    }
                }
            }
            // Every third frame: the bodies move a pixel or two between
            // samples, so a body standing on the dummy is caught on the frames
            // that matter without paying two plate draws for all 60.
            for _ in 0..3 {
                scene.step();
            }
        }
        assert!(drawn, "{key:?} never drew its practice dummy");
        assert!(
            subject_drawn > 0,
            "{key:?}: the subject was never drawn at its own point"
        );
    }
}

/// Every named fight's plate holds its whole footprint during the tell and the
/// strike: the body, the telegraph's own shape (a cone, a cord's swept sector,
/// the Salt Hen's three pads) and the target are all inside the drawn view with
/// the fit's margin. The stage centres on body + footprint and takes its zoom
/// from those bounds, so a wide row can no longer clip the right edge.
#[test]
fn every_miniboss_footprint_fits_its_plate() {
    for kind in MinibossKind::all() {
        let key = Key::Miniboss(kind);
        let mut scene = LibraryScene::build(key).expect("a miniboss scene");
        let subject = scene.subject().expect("a miniboss body");
        let mut art = PixelBuffer::new(44, 30);
        let ticks = (scene.loop_secs() / TICK) as u32;
        let mut showed = false;
        for i in 0..ticks {
            let live = scene
                .world
                .ecs
                .get::<Miniboss>(subject)
                .map(|mb| (mb.state, mb.telegraph()));
            let strike = matches!(
                live,
                Some((MinibossState::Tell | MinibossState::Active, Some(_)))
            );
            if i == 0 || strike {
                plate(&mut scene, &mut art);
            }
            if strike {
                showed = true;
                let (lo, hi) = scene.fit_bounds().expect("a fitted footprint");
                for corner in [
                    Vec2::new(lo.x, lo.y),
                    Vec2::new(hi.x, lo.y),
                    Vec2::new(lo.x, hi.y),
                    Vec2::new(hi.x, hi.y),
                ] {
                    assert!(
                        on_plate(&scene, corner),
                        "{kind:?}: footprint corner {corner:?} left the plate (bounds {lo:?}..{hi:?}, zoom {}, cam {:?}, view {:?})",
                        scene.world.zoom,
                        scene.world.cam,
                        scene.view
                    );
                }
            }
            scene.step();
        }
        assert!(showed, "{kind:?} never showed a tell or a strike");
    }
}

/// Every flatbed card stands on the trailer's own bed. The deck art, the
/// slipstream and the tailgate drift all read the map's own deck rectangle, so
/// a card built on a patch at the origin showed plain green instead of planks,
/// joints and the rushing road in the gaps.
#[test]
fn flatbed_cards_stand_on_the_deck() {
    let mut keys: Vec<Key> = Vec::new();
    for slot in [
        Slot::A,
        Slot::B,
        Slot::C,
        Slot::I,
        Slot::II,
        Slot::III,
        Slot::R,
        Slot::H,
    ] {
        let kind = roster::kind(Theme::Trailer, slot);
        // A body is staged on the floor that fields it (`home_floor`), so only
        // the kinds this floor owns are flatbed cards.
        if home_floor(kind) == Theme::Trailer {
            keys.push(Key::Enemy(kind));
        }
    }
    for kind in roster::appointments(Theme::Trailer) {
        if miniboss_floor(kind) == Theme::Trailer {
            keys.push(Key::Miniboss(kind));
        }
    }
    for kind in [
        ObjectKind::Pit,
        ObjectKind::SlidingCargo,
        ObjectKind::Slipstream,
    ] {
        keys.push(Key::Object(kind));
    }
    for key in keys {
        let mut scene = LibraryScene::build(key).unwrap_or_else(|| panic!("{key:?} has no scene"));
        {
            let world = scene.world();
            let level = &world.level;
            assert_eq!(level.theme, Theme::Trailer, "{key:?}");
            assert!(
                !level.trailer.is_empty(),
                "{key:?}: the plate has no deck for the floor art to draw"
            );
            let (cx, cy) = crate::game::collision::cell_of(level.start);
            assert!(
                level.trailer.deck.contains(cx, cy),
                "{key:?}: the card starts off the bed at {cx},{cy}"
            );
        }
    }
}

/// Every weapon and every evolution has a runnable scene, whatever the save
/// has owned. The Library's animation gate is deliberately the caller's rule
/// (`render::library::catalogued`: a weapon or passive must be *owned*, an
/// evolution owned or evolved), so a locked card stays `???`, a merely-offered
/// weapon keeps its static silhouette pose, and the scene layer itself never
/// blanks a plate.
#[test]
fn every_weapon_has_a_scene_and_the_gate_is_the_callers() {
    for kind in WeaponKind::all() {
        assert!(
            matches!(plan(Key::Weapon(kind)), Plan::Sandbox),
            "{kind:?} is planned as a still plate"
        );
        assert!(
            LibraryScene::build(Key::Weapon(kind)).is_some(),
            "{kind:?} has no arsenal scene"
        );
        if kind.evolution().is_some() {
            assert!(
                LibraryScene::build(Key::Evolution(kind)).is_some(),
                "{kind:?}'s evolution has no scene"
            );
        }
    }
}

/// The arsenal's three targets stand at the effect's own scale: outside the
/// radius the renderer clears hit digits from (26 px), and inside the reach of
/// the shortest melee weapon at max rank (Tail Whip's aura is 26 + 3 × level).
#[test]
fn the_arsenal_targets_stand_at_the_effects_range() {
    let rat = Vec2::new(100.0, 100.0);
    for home in dummy_homes(rat) {
        let d = home.dist(rat);
        assert!(d > 26.0, "a target at {d} px has its hit digits suppressed");
        assert!(d <= 34.0, "a target at {d} px is beyond a max-rank whip's aura");
    }
}

/// The standing targets (and room for the digits above them) are inside the
/// frame: centred on the rat, a 66 px plate cut the targets at its right edge.
#[test]
fn the_arsenal_targets_stand_inside_the_plate() {
    for kind in WeaponKind::all() {
        if kind == WeaponKind::SuperDash {
            // The one arsenal plate whose rat travels.
            continue;
        }
        let mut scene = LibraryScene::build(Key::Weapon(kind)).expect("a weapon scene");
        let mut art = PixelBuffer::new(44, 30);
        for _ in 0..30 {
            scene.step();
        }
        plate(&mut scene, &mut art);
        let (cam, view) = (scene.world.cam, scene.view);
        let enemies = scene.world.enemies();
        assert!(!enemies.is_empty(), "{kind:?}: no targets were staged");
        for home in enemies.iter().map(|e| e.pos) {
            let margin = if kind == WeaponKind::TailWhip {
                home.x - cam.x
            } else {
                cam.x + view.x - home.x
            };
            assert!(margin >= 12.0, "{kind:?}: a target stands {margin} px from the edge");
            assert!(on_plate(&scene, home), "{kind:?}: a target is off the plate");
        }
    }
}

/// A brief active weapon fires several times inside one loop: the plate keeps
/// the charge off (the cost is on the card's facts, not in its picture) while
/// the effect, the hit flash and the digits are the weapon's own.
#[test]
fn the_active_weapons_fire_several_times_a_loop() {
    use crate::game::ecs::resources::ActiveFx;
    for kind in [WeaponKind::SonicSqueak, WeaponKind::TailThrash] {
        let key = Key::Weapon(kind);
        let mut scene = LibraryScene::build(key).expect("a weapon scene");
        let mut fired = 0u32;
        let mut was = false;
        one_loop(&mut scene, |s| {
            let fx = *s.world().ecs.resource::<ActiveFx>();
            let now = match kind {
                WeaponKind::SonicSqueak => fx.squeak_left > 0.0,
                _ => fx.thrash_left > 0.0,
            };
            if now && !was {
                fired += 1;
            }
            was = now;
        });
        assert!(fired >= 3, "{kind:?} fired {fired} times inside one loop");
    }
}

/// The Toll Gull's plate runs its target: the hidden rat walks away from the
/// gull in bursts, and its shot is aimed off the line the dummy is standing on
/// (the 0.35 s lead), which is what the card exists to show.
#[test]
fn the_toll_gull_leads_a_running_target() {
    let key = Key::Enemy(EnemyKind::TollGull);
    let mut scene = LibraryScene::build(key).expect("a gull scene");
    let mut fastest = 0.0f32;
    let mut off_axis = false;
    let mut shots = 0u32;
    one_loop(&mut scene, |s| {
        let w = s.world();
        let aim = w.player.pos;
        fastest = fastest.max(w.player.vel.len());
        let mut q = w.ecs.query::<&Bullet>();
        for b in q.iter(&w.ecs) {
            if b.faction != Faction::Enemy {
                continue;
            }
            shots += 1;
            let dir = b.vel.normalized();
            let direct = (aim - b.pos).normalized();
            if dir.dot(direct) < 0.9995 {
                off_axis = true;
            }
        }
    });
    assert!(shots > 0, "the gull never fired");
    assert!(fastest > 20.0, "the card's target never ran ({fastest} px/s)");
    assert!(off_axis, "every shot was aimed straight at the standing dummy");
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

/// Six weapons hit the standing dummies on their plates.
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

/// The sampled evolutions hit the standing dummies on their plates.
#[test]
fn the_evolutions_run() {
    for kind in [
        WeaponKind::Gnaw,
        WeaponKind::CheeseSpray,
        WeaponKind::SqueakNova,
        WeaponKind::TailWhip,
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
        ObjectKind::CrabPot,
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

#[test]
fn crab_pot_plate_is_sprung_by_fodder() {
    use crate::game::art_crabpot::CrabPotState;
    let mut scene = LibraryScene::build(Key::Object(ObjectKind::CrabPot)).expect("a crab pot");
    let mut warned = false;
    let mut held = false;
    let mut opened = false;
    for _ in 0..(scene.loop_secs() / TICK) as u32 {
        scene.step();
        let pot = &scene.world().level.objects.crab_pots[0];
        assert!(!pot.caught_rat);
        warned |= pot.state == CrabPotState::Closing;
        held |= pot.state == CrabPotState::Shut && pot.caught > 0;
        opened |= pot.state == CrabPotState::Open;
    }
    assert!(warned && held && opened);
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

/// Pickup scenes are walked over and collected (the plate then shows what
/// the pickup did).
#[test]
fn the_pocket_finds_are_collected() {
    for kind in [
        PickupKind::Heal,
        PickupKind::Nuke,
        PickupKind::BottleCap,
        PickupKind::CleanSocks,
        PickupKind::GrowthSerum,
        PickupKind::ShrinkDraught,
        PickupKind::WhiteCoat,
        PickupKind::TheCollar,
    ] {
        let key = Key::Pickup(kind);
        let mut scene = LibraryScene::build(key).expect("a pickup scene");
        let mut present = true;
        one_loop(&mut scene, |s| {
            let w = s.world();
            let mut q = w.ecs.query::<&Pickup>();
            // The staged find itself: a body the Nuke kills may drop a find of
            // its own, and that one is not what the card is about.
            present &= q.iter(&w.ecs).any(|p| p.kind == kind);
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

// ---- Batch 12: the cards the owner flagged --------------------------------

/// The body bounds a boss card must hold, straight from the scene's own rule.
/// The union framing (`Script::boss_fit`) is measured from exactly this, so the
/// test and the picture cannot drift apart.
fn boss_bounds_now(scene: &mut LibraryScene) -> Option<(Vec2, Vec2)> {
    let w = scene.world();
    let mut q = w.ecs.query::<&Boss>();
    q.iter(&w.ecs).next().map(boss_body_bounds)
}

/// Is the Redtail down where the art draws a bird (rather than only its shadow)?
///
/// The art owns the rule: while the hawk is overhead `Boss::hit_shapes` is
/// empty — no weapon can touch it and the deck shows a shadow — so "a hit shape
/// exists" is exactly "there are non-shadow pixels of the bird to draw".
fn redtail_on_the_deck(scene: &mut LibraryScene) -> bool {
    let w = scene.world();
    let mut q = w.ecs.query::<&Boss>();
    for boss in q.iter(&w.ecs) {
        if boss.kind == BossKind::Hawk {
            return boss.hit_shapes().iter().next().is_some();
        }
    }
    false
}

/// Every boss's drawn body — the union the card is framed on — is on its plate
/// for at least 90 % of the loop, and the Redtail is a bird, not only a shadow,
/// for at least 70 % of its.
///
/// The Sweeper and SUBJECT R-01 were clipped at the top, the Night Edition at
/// the bottom, the King Cobra was mostly off the plate and the Redtail was an
/// empty deck; this is the property that says none of them is.
#[test]
fn every_boss_body_stays_on_its_plate() {
    for kind in BossKind::all() {
        let key = Key::Boss(kind);
        let mut scene = LibraryScene::build(key).expect("a boss scene");
        let mut art = PixelBuffer::new(44, 30);
        let ticks = (scene.loop_secs() / TICK) as u32;
        let (mut seen, mut on) = (0u32, 0u32);
        let (mut samples, mut visible) = (0u32, 0u32);
        for i in 0..ticks {
            if i % 4 == 0 {
                plate(&mut scene, &mut art);
                if let Some((lo, hi)) = boss_bounds_now(&mut scene) {
                    seen += 1;
                    let corners = [
                        Vec2::new(lo.x, lo.y),
                        Vec2::new(hi.x, lo.y),
                        Vec2::new(lo.x, hi.y),
                        Vec2::new(hi.x, hi.y),
                    ];
                    if corners.iter().all(|c| on_plate(&scene, *c)) {
                        on += 1;
                    }
                }
                if kind == BossKind::Hawk {
                    samples += 1;
                    if redtail_on_the_deck(&mut scene) {
                        visible += 1;
                    }
                }
            }
            scene.step();
        }
        assert!(seen > 0, "{kind:?} never put a body on its plate");
        assert!(
            on * 100 >= seen * 90,
            "{kind:?}: the body was fully on the plate {on} of {seen} samples (union {:?}, view {:?})",
            scene.boss_union,
            scene.view
        );
        if kind == BossKind::Hawk {
            assert!(samples > 0, "the Redtail never ran");
            assert!(
                visible * 100 >= samples * 70,
                "the Redtail was on the deck for only {visible} of {samples} samples"
            );
        }
    }
}

#[test]
fn the_sweeper_plate_shows_the_lane_sprint_and_skid() {
    use crate::game::art_boss_broom::BroomBossState::{Run, RunRecover, RunWindup};
    let mut scene = LibraryScene::build(Key::Boss(BossKind::Broom)).expect("Sweeper scene");
    let mut phases = [false; 3];
    for _ in 0..(scene.loop_secs() / TICK) as u32 {
        scene.step();
        for entity in scene.world().ecs.iter_entities() {
            if let Some(boss) = entity.get::<Boss>() {
                match boss.state {
                    BossState::Broom(RunWindup) => {
                        phases[0] |= boss.hazards.iter().any(|h| {
                            matches!(h.shape, HazardShape::Rect { .. }) && !h.is_expired()
                        });
                    }
                    BossState::Broom(Run) => phases[1] = true,
                    BossState::Broom(RunRecover) => phases[2] = true,
                    _ => {}
                }
            }
        }
    }
    assert_eq!(phases, [true; 3]);
}

/// A floor's card is a real furnished, **lit** room cut from the floor's own
/// generated map: walls, a prop, a light fixture and a wall sconce in the
/// 12 × 8-tile window, the floor's real lighting (not the cards' flat light),
/// no rat, and the pan really travels across the window.
#[test]
fn every_floor_card_shows_a_furnished_lit_room() {
    for theme in Theme::playable() {
        let key = Key::Floor(theme);
        let mut scene = LibraryScene::build(key).expect("a floor scene");
        assert!(scene.world.hide_rat, "{theme:?} drew the rat");
        assert!(!scene.world.flat_lit, "{theme:?} is flat-lit");
        let window = scene.script.window.expect("a crop");
        let (walls, props, lights, sconces) = {
            let level = &scene.world.level;
            let walls = window
                .cells()
                .filter(|(x, y)| level.tiles.get(*x, *y).is_wall())
                .count();
            let props = level
                .props
                .iter()
                .filter(|p| window.contains_world(p.rect.center()))
                .count();
            let lights = level
                .objects
                .lights
                .iter()
                .filter(|l| window.contains_world(l.pos))
                .count();
            let sconces = level
                .objects
                .lights
                .iter()
                .filter(|l| {
                    window.contains_world(l.pos) && l.kind == LightFixtureKind::Sconce
                })
                .count();
            (walls, props, lights, sconces)
        };
        assert!(walls >= 2, "{theme:?}: a window with {walls} wall cells");
        assert!(props >= 1, "{theme:?}: the window has no prop");
        assert!(lights >= 1, "{theme:?}: the window has no light fixture");
        assert!(sconces >= 1, "{theme:?}: the window has no wall sconce");

        // The picture itself: unlit for the measurement, so every pixel that
        // differs from the bare floor is real art (a wall, a prop, a fixture or
        // a body) and not the lighting pass.
        let mut art = PixelBuffer::new(44, 30);
        let (mut wall_px, mut prop_px, mut light_px, mut rat_ink) = (0usize, 0usize, 0usize, false);
        let (mut cam_lo, mut cam_hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let ticks = (scene.loop_secs() / TICK) as u32;
        for i in 0..ticks {
            if i % 10 == 0 {
                scene.world.flat_lit = true;
                plate(&mut scene, &mut art);
                rat_ink |= has_rat_ink(&art);
                let (cam, view) = (scene.world.cam, scene.view);
                cam_lo = cam_lo.min(cam.x);
                cam_hi = cam_hi.max(cam.x);
                let scale = view.x / art.w as f32;
                let to_plate = |p: Vec2| {
                    (
                        ((p.x - cam.x) / scale).round() as i32,
                        ((p.y - cam.y) / scale).round() as i32,
                    )
                };
                let floor = scene.floor_plate(art.w, art.h).pixels().to_vec();
                let changed = |x: i32, y: i32| {
                    if !(0..art.w as i32).contains(&x) || !(0..art.h as i32).contains(&y) {
                        return false;
                    }
                    let i = y as usize * art.w + x as usize;
                    art.pixels()[i] != floor[i]
                };
                let level = &scene.world.level;
                for (cx, cy) in window.cells() {
                    if !level.tiles.get(cx, cy).is_wall() {
                        continue;
                    }
                    let (x0, y0) = to_plate(Vec2::new(
                        cx as f32 * crate::game::collision::TILE_F,
                        cy as f32 * crate::game::collision::TILE_F,
                    ));
                    let (x1, y1) = to_plate(Vec2::new(
                        (cx + 1) as f32 * crate::game::collision::TILE_F,
                        (cy + 1) as f32 * crate::game::collision::TILE_F,
                    ));
                    let mut hit = 0usize;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            hit += usize::from(changed(x, y));
                        }
                    }
                    if hit >= 8 {
                        wall_px += 1;
                    }
                }
                for p in &level.props {
                    let (x0, y0) = to_plate(Vec2::new(p.rect.x, p.rect.y));
                    let (x1, y1) = to_plate(Vec2::new(p.rect.right(), p.rect.bottom()));
                    let mut hit = 0usize;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            hit += usize::from(changed(x, y));
                        }
                    }
                    prop_px = prop_px.max(hit);
                }
                for l in &level.objects.lights {
                    // The fixture's own footprint is a few pixels of housing
                    // and housing glow around its mount point.
                    let (x, y) = to_plate(l.pos);
                    let mut hit = 0usize;
                    for dy in -4..=4 {
                        for dx in -4..=4 {
                            hit += usize::from(changed(x + dx, y + dy));
                        }
                    }
                    light_px = light_px.max(hit);
                }
                scene.world.flat_lit = false;
            }
            scene.step();
        }
        assert!(!rat_ink, "{theme:?}: the floor card drew the rat");
        assert!(wall_px > 0, "{theme:?}: no wall pixels on the plate");
        assert!(prop_px >= 6, "{theme:?}: no prop on the plate ({prop_px} px)");
        assert!(light_px > 0, "{theme:?}: no light fixture on the plate");
        assert!(
            cam_hi - cam_lo > 10.0,
            "{theme:?}: the camera did not pan ({cam_lo}..{cam_hi})"
        );
    }
}

/// The tiny subjects are drawn at 2 × (zoom 0.5), with the practice dummy at
/// least 14 px off the subject's rest position: at 1:1 they were specks.
#[test]
fn tiny_subjects_are_drawn_at_twice_the_plate_zoom() {
    for kind in [
        EnemyKind::BellFlea,
        EnemyKind::WickMoth,
        EnemyKind::Lilliputian,
        EnemyKind::InkBlot,
    ] {
        assert!(
            kind.radius() < TINY_BODY_RADIUS,
            "{kind:?} is not a tiny body"
        );
        let key = Key::Enemy(kind);
        let mut scene = LibraryScene::build(key).expect("a scene");
        assert_eq!(scene.script.zoom, TINY_ZOOM, "{kind:?} is not at 2x");
        let rest = scene.subject_pos().expect("a body");
        let dummy = scene.world.player.pos;
        assert!(
            rest.dist(dummy) >= 14.0,
            "{kind:?}: the dummy is {} px from the body",
            rest.dist(dummy)
        );
        let mut art = PixelBuffer::new(44, 30);
        plate(&mut scene, &mut art);
        assert!(
            (scene.view.x - 44.0 * TINY_ZOOM).abs() < 0.01,
            "{kind:?}: the plate shows {} px, not a 2x view",
            scene.view.x
        );
    }
}

/// Every named fight opens on its **idle** frame with the whole body on the
/// plate: the loop starts at the body's mark, not on a walk-in from off frame.
#[test]
fn every_miniboss_first_frame_holds_its_body() {
    for kind in MinibossKind::all() {
        let key = Key::Miniboss(kind);
        let mut scene = LibraryScene::build(key).expect("a miniboss scene");
        let subject = scene.subject().expect("a body");
        assert_eq!(
            scene.world.ecs.get::<Miniboss>(subject).map(|mb| mb.state),
            Some(MinibossState::Approach),
            "{kind:?} does not open on its idle approach"
        );
        let mut art = PixelBuffer::new(44, 30);
        plate(&mut scene, &mut art);
        let (lo, hi) = scene.fit_bounds().expect("a fitted body");
        for corner in [
            Vec2::new(lo.x, lo.y),
            Vec2::new(hi.x, lo.y),
            Vec2::new(lo.x, hi.y),
            Vec2::new(hi.x, hi.y),
        ] {
            assert!(
                on_plate(&scene, corner),
                "{kind:?}: the first frame cuts its body (corner {corner:?}, view {:?}, cam {:?})",
                scene.view,
                scene.world.cam
            );
        }
    }
}

/// The Tail Thrash's and the Flywheel's targets survive a whole loop (a
/// max-rank sweep killed a 4,000 hp body on the first pass) and the plate shows
/// the whole sweep radius.
#[test]
fn the_thrash_and_flywheel_targets_survive_the_loop() {
    for key in [
        Key::Weapon(WeaponKind::TailThrash),
        Key::Evolution(WeaponKind::TailThrash),
    ] {
        let mut scene = LibraryScene::build(key).expect("an arsenal scene");
        let mut art = PixelBuffer::new(44, 30);
        plate(&mut scene, &mut art);
        let reach = scene.world.player.loadout.weapons[0].active_reach();
        assert!(
            scene.view.x >= 2.0 * reach && scene.view.y >= 2.0 * reach,
            "{key:?}: a {reach} px sweep in a {:?} view",
            scene.view
        );
        let mut fewest = usize::MAX;
        one_loop(&mut scene, |s| {
            let w = s.world();
            let mut q = w.ecs.query::<&Enemy>();
            let standing = q.iter(&w.ecs).filter(|b| b.speed_scale == 0.0).count();
            fewest = fewest.min(standing);
        });
        assert!(
            fewest >= 3,
            "{key:?}: only {fewest} targets survived its own sweep"
        );
    }
}

/// The Super Dash and the Ripcord lay their targets along the dash lane and
/// keep the whole lane — the rat, the charge and every target — in frame.
#[test]
fn the_dash_lane_stays_in_frame() {
    for key in [
        Key::Weapon(WeaponKind::SuperDash),
        Key::Evolution(WeaponKind::SuperDash),
    ] {
        let mut scene = LibraryScene::build(key).expect("an arsenal scene");
        let mut art = PixelBuffer::new(44, 30);
        let mut hit = false;
        one_loop(&mut scene, |s| {
            plate(s, &mut art);
            for home in dash_lane_homes(s.world.level.start) {
                assert!(on_plate(s, home), "{key:?}: a lane target left the plate");
            }
            assert!(
                on_plate(s, s.world.player.pos),
                "{key:?}: the rat left its own dash lane"
            );
            hit |= a_body_is_hurt(s);
        });
        assert!(hit, "{key:?} never hit its lane");
    }
}

/// The chest card shows the rat walking up, the chest opening and a reward
/// burst — and the real collection never opens a plate (the card drops it).
#[test]
fn the_chest_opens_and_pays_out() {
    let key = Key::Object(ObjectKind::Chest);
    let mut scene = LibraryScene::build(key).expect("a chest scene");
    let mut art = PixelBuffer::new(44, 30);
    let chest = scene.script.chest.expect("a chest point");
    let ticks = (scene.loop_secs() / TICK) as u32;
    let mut collected_at = None;
    let mut burst = false;
    for i in 0..ticks {
        if i % 3 == 0 {
            plate(&mut scene, &mut art);
            // Only after the lid opened: the closed chest's own brass and seam
            // use these colours, so the claim is about the burst, not the box.
            if scene.opened.is_some() {
                burst |= has_colour(&art, CHEST_BRASS) && has_colour(&art, CHEST_GLOW);
            }
        }
        scene.step();
        assert_eq!(scene.world.pending_chests, 0, "a chest plate was left open");
        if collected_at.is_none() && scene.opened.is_some() {
            collected_at = Some(scene.clock());
        }
    }
    let at = collected_at.expect("the chest was never collected");
    assert!(at > 0.4, "the chest was collected before the rat walked up");
    assert!(burst, "the chest never burst open");
    assert!(chest.dist(scene.world.level.start) > 10.0);
}

/// A shooter's pellet leaves a self-lit trail (a 1 px dot is not a shot on a
/// card), and the trail is the card's own: no bullet is changed by it.
#[test]
fn shooter_pellets_leave_a_trail() {
    for theme in Theme::playable() {
        let key = Key::Enemy(roster::kind(theme, Slot::R));
        let mut scene = LibraryScene::build(key).expect("a shooter scene");
        assert!(scene.script.trail, "{key:?} draws no pellet trail");
        let mut art = PixelBuffer::new(44, 30);
        let mut seen = false;
        one_loop(&mut scene, |s| {
            plate(s, &mut art);
            seen |= PELLET_TRAIL.iter().any(|c| has_colour(&art, *c));
        });
        assert!(seen, "{key:?} never drew a pellet trail");
    }
}

#[test]
fn outfall_trick_cards_run_their_real_rules() {
    use crate::game::ecs::systems::floor_outfall::{
        OutfallTellPhase, SCHOOL_BUNCHED_RADIUS, SCHOOL_LIGHT_REACH, SCHOOL_RAT_REACH,
    };

    for kind in [EnemyKind::Skater, EnemyKind::Lampfish, EnemyKind::FrySchool] {
        let mut scene = LibraryScene::build(Key::Enemy(kind)).unwrap();
        let subject = scene.subject().unwrap();
        assert_eq!(scene.world.level.theme, Theme::Outfall);
        let (mut dry, mut wet, mut dark, mut bunched, mut tell, mut strike) =
            (false, false, false, false, false, false);
        one_loop(&mut scene, |scene| {
            let world = &scene.world;
            let body = world.ecs.get::<Enemy>(subject).unwrap();
            if kind == EnemyKind::Skater {
                wet |= world.level.wet_at(body.pos);
                dry |= !world.level.wet_at(body.pos);
            }
            if kind == EnemyKind::FrySchool {
                dark |= body.radius == kind.radius();
                bunched |= body.radius == SCHOOL_BUNCHED_RADIUS
                    && body.vel == Vec2::ZERO
                    && body.pos.dist(world.player.pos) > SCHOOL_RAT_REACH
                    && world.level.nearest_live_sconce(body.pos, SCHOOL_LIGHT_REACH).is_some();
            }
            if let Some(attack) = outfall_enemy_tell(body) {
                tell |= attack.phase == OutfallTellPhase::Tell;
                strike |= attack.phase == OutfallTellPhase::Strike;
            }
        });
        match kind {
            EnemyKind::Skater => assert!(dry && wet && tell && strike, "Skater missed a beat"),
            EnemyKind::Lampfish => assert!(tell && strike, "Lampfish missed its locked lunge"),
            EnemyKind::FrySchool => assert!(dark && bunched, "the sconce never bunched the school"),
            _ => unreachable!(),
        }
    }
}

#[test]
fn the_shore_crab_card_pinches_at_the_dummy() {
    use crate::game::art_crab::CrabState;
    use crate::game::ecs::systems::crab::{claw_warning, crab_view};

    let mut scene = LibraryScene::build(Key::Enemy(EnemyKind::ShoreCrab)).unwrap();
    let subject = scene.subject().unwrap();
    assert_eq!(scene.world.level.theme, Theme::Outfall);
    let (mut tell, mut pinch, mut recover) = (false, false, false);
    one_loop(&mut scene, |scene| {
        let body = scene.world.ecs.get::<Enemy>(subject).unwrap();
        match crab_view(body).state {
            CrabState::Tell => tell = true,
            CrabState::Pinch => {
                let (shape, _, _, _) = claw_warning(body).unwrap();
                pinch |= shape.contains(scene.world.player.pos, 0.0);
            }
            CrabState::Recover => recover = true,
            CrabState::Scuttle => {}
        }
    });
    assert!(tell && pinch && recover);
}

#[test]
fn outfall_named_cards_show_both_halves_of_the_tell_and_the_strike() {
    for kind in roster::appointments(Theme::Outfall) {
        let mut scene = LibraryScene::build(Key::Miniboss(kind)).unwrap();
        let subject = scene.subject().unwrap();
        let (mut tracking, mut locked, mut struck, mut signature) = (false, false, false, false);
        one_loop(&mut scene, |scene| {
            let world = &scene.world;
            let mb = world.ecs.get::<Miniboss>(subject).unwrap();
            struck |= mb.state == MinibossState::Active;
            if mb.state != MinibossState::Tell {
                return;
            }
            tracking |= mb.timer < crate::game::ecs::systems::miniboss::TELL_LOCK;
            locked |= mb.timer >= crate::game::ecs::systems::miniboss::TELL_LOCK;
            signature |= match (kind, mb.telegraph()) {
                (MinibossKind::Gasket, Some(MinibossShape::Annulus { inner, .. })) => inner > 0.0,
                (MinibossKind::Swell, Some(MinibossShape::Tide { from, to, .. })) => to - from > 40.0,
                (MinibossKind::Shade, Some(MinibossShape::Umbra { origin, aim, .. })) => {
                    let lamp = scene.script.fixture.unwrap();
                    aim.dot(origin - lamp) > 0.0
                }
                (MinibossKind::FalseBulb, Some(MinibossShape::False { selected, .. })) => {
                    selected.is_some()
                }
                _ => false,
            };
        });
        assert!(tracking && locked && struck && signature, "{kind:?} missed a beat");
    }
}

#[test]
fn outfall_lantern_card_runs_arrival_feint_snuff_and_gape() {
    let mut scene = LibraryScene::build(Key::Boss(BossKind::Lantern)).unwrap();
    let subject = scene.subject().unwrap();
    let (mut arrival, mut snap, mut snuff, mut paired, mut pull, mut stamp, mut restored) =
        (false, false, false, false, false, false, false);
    let mut art = PixelBuffer::new(44, 30);
    one_loop(&mut scene, |scene| {
        let view = scene.world.ecs.get::<Boss>(subject).unwrap().lantern_view().unwrap();
        let dark = scene.world.level.objects.outfall_boss_snuff == Some(0);
        let sample = match view.state {
            LanternState::ArrivalLure => {
                let showed = view.real_bulb.is_some_and(|bulb| bulb.pos.dist(view.pos) > 8.0);
                let first = showed && !arrival;
                arrival |= showed;
                first
            }
            LanternState::JawSnap => {
                let first = !snap;
                snap = true;
                first
            }
            LanternState::GapePull => {
                let first = !pull;
                pull |= view.attack.is_some() && view.stamp.is_some();
                restored |= !dark;
                first
            }
            LanternState::GapeStamp => {
                stamp = true;
                false
            }
            _ => {
                snuff |= dark;
                let both = dark && view.decoy.is_some() && view.real_bulb.is_some();
                let first = both && !paired;
                paired |= both;
                first
            }
        };
        if sample {
            plate(scene, &mut art);
            let (lo, hi) = scene.fit_bounds().unwrap();
            assert!(on_plate(scene, lo) && on_plate(scene, hi), "{:?} left the plate", view.state);
        }
    });
    assert!(arrival && snap && snuff && paired && pull && stamp && restored);
}

#[test]
fn outfall_floor_card_runs_a_lure_on_the_wet_bank() {
    use crate::game::hazards::{lure_landing_clear, LurePhase};

    let mut scene = LibraryScene::build(Key::Floor(Theme::Outfall)).unwrap();
    let level = &scene.world.level;
    let window = scene.script.window.unwrap();
    assert!(level.puddles.iter().any(|p| window.contains_world(p.pos)));
    assert!(level.props.iter().any(|p| p.kind == PropKind::OutfallRib
        && window.contains_world(p.rect.center())));
    let trap = &level.objects.lure_traps[0];
    assert_eq!(trap.phase, LurePhase::Ready);
    assert!(trap.landings.iter().any(|&p| lure_landing_clear(level, trap.pos, p)));
    let (mut warned, mut released) = (false, false);
    one_loop(&mut scene, |scene| {
        let trap = &scene.world.level.objects.lure_traps[0];
        warned |= trap.phase == LurePhase::Warning;
        released |= trap.phase == LurePhase::Spent && !trap.failed;
    });
    assert!(warned && released);
}
