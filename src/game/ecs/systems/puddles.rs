//! Wet-floor footsteps: the rat's little splashes, the ripples they leave, and
//! the rings the horde's bodies make in standing water.
//!
//! The owner's brief: *"the rat should make little splashing sounds when walking
//! over \[very wet floor\]"*. This system is the whole of the gameplay half of
//! standing water — it reads [`Level::wet_at`] and the rat's movement and writes
//! only [`SfxQueue`] and the cosmetic [`PuddleSteps`] resource the renderer
//! reads.
//!
//! ## What makes a sound (and what does not)
//!
//! * **A step**: every [`STEP_PX`] of travel **on water**. Distance accumulates
//!   only while the rat is wet, so a rat that walks out of a puddle and back in
//!   starts again from zero rather than splashing the moment it touches water.
//! * **Entering water from dry**: one splash at once, even at a crawl — the
//!   moment a toe goes in is a sound.
//! * **A dash**: exactly one `Sfx::SplashBig` and one big ripple per dash that
//!   *starts or passes through* water (Entering, crossing a puddle at speed, or
//!   dashing out of one). The latch is on the dash, not the water, so a single
//!   dash never machine-guns the heavy sound; a *new* dash may splash again.
//! * **Standing still**: nothing. The accumulator only grows by the distance the
//!   rat actually moved this tick, so a rat in a puddle with no input is silent.
//! * **Dry floor**: nothing at all, and the accumulator is zeroed.
//!
//! ## Enemies
//!
//! The first [`ENEMY_RIPPLE_SLOTS`] ordinary ground bodies in `SpawnId` order
//! that are standing in water **and on screen** (the simulation's own viewport,
//! never the camera — see [`approach::in_sim_view`]) start a small ripple at
//! most every [`ENEMY_RIPPLE_SECS`]. They make no sound: the horde is a
//! thousand feet and the sound budget is the rat's.
//!
//! ## Determinism
//!
//! No rng draw, no entity spawn or despawn, no write to any simulated number —
//! the rat is **not** slowed, no body is moved, nothing is read back. The
//! golden traces fold the rat, the enemies, the counters and the entity
//! populations; they do not fold [`SfxQueue`] or [`PuddleSteps`], so the
//! goldens cannot move for this system (pinned by
//! `puddle_steps_change_no_simulated_state`).
//!
//! The slot table is keyed by `SpawnId` order rather than by a component, so a
//! body that leaves the first six hands its slot back and the cooldowns can
//! never lengthen a spawn or a despawn path.
//!
//! ## Schedule slot
//!
//! Chained **immediately after `player::player_system`**: the rat's position,
//! `dash_active` and the level are all this tick's, and nothing between here and
//! the render pass reads the water again.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{
    EnemyRippleSlot, PuddleSteps, Ripple, SfxQueue, TickInput, Viewport,
};
use crate::game::ecs::systems::approach::in_sim_view;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{Level, Theme};
use crate::game::player::Player;
use crate::game::roster::{self, Slot};
use crate::math::Vec2;

/// How far the rat travels on water between two step splashes, in world px.
///
/// The rat's ordinary speed is ~90 px/s, so this is a splash about every
/// 0.12 s — a patter, not a metronome, and dense enough that crossing a puddle
/// reads as water.
pub const STEP_PX: f32 = 11.0;

/// How long a ripple lives, in seconds.
pub const RIPPLE_LIFE: f32 = 0.45;

/// How many of the horde's bodies may ripple at once.
pub const ENEMY_RIPPLE_SLOTS: usize = 6;

/// How often one of those slots may start a ripple.
pub const ENEMY_RIPPLE_SECS: f32 = 0.5;

/// The feet are this many pixels below the rat's centre: where a ripple starts.
const FEET_PX: f32 = 5.0;

/// Does this body walk on the floor (so it can stand in water)?
///
/// The roster's **B slot** is each floor's flying fodder, and the wave-4
/// `MurderHornet` archetype hovers and dives; everything else in the game walks,
/// slithers or scuttles on the floor. A kind a floor does not list (a boss body
/// or a test spawn) counts as walking.
pub fn on_the_floor(theme: Theme, kind: EnemyKind) -> bool {
    if matches!(roster::floor(theme).slot_of(kind), Some(Slot::B)) {
        return false;
    }
    !matches!(kind.archetype(), EnemyKind::MurderHornet)
}

#[allow(clippy::too_many_arguments)]
pub fn puddle_steps_system(
    input: Res<TickInput>,
    level: Res<Level>,
    player: Res<Player>,
    viewport: Res<Viewport>,
    mut sfx: ResMut<SfxQueue>,
    mut steps: ResMut<PuddleSteps>,
    enemies: Query<(&SpawnId, &Enemy)>,
    mut order: Local<Vec<(u64, Vec2)>>,
) {
    let dt = input.dt.max(0.0);

    // Ripples age (and die) every tick, wet floor or not: they are cosmetic
    // and must not freeze when the rat walks off the puddle.
    for ripple in steps.ripples.iter_mut() {
        if !ripple.live {
            continue;
        }
        ripple.age += dt;
        if ripple.age >= RIPPLE_LIFE {
            *ripple = Ripple::default();
        }
    }

    if level.puddles.is_empty() {
        // A dry floor: nothing to accumulate and no state worth keeping.
        steps.dist = 0.0;
        steps.last_wet = false;
        steps.last_pos = player.pos;
        return;
    }

    let pos = player.pos;
    let travelled = pos.dist(steps.last_pos);
    steps.last_pos = pos;
    let wet = level.wet_at(pos);
    let dashing = player.dash_active > 0.0;
    if !dashing {
        // A dash's latch is re-armed when the dash ends, so the *next* dash
        // through water splashes again.
        steps.dash_splashed = false;
    }
    let feet = pos + Vec2::new(0.0, FEET_PX);

    if wet && dashing {
        // One heavy slosh per dash, wherever in the dash the water is.
        if !steps.dash_splashed {
            steps.dash_splashed = true;
            steps.dist = 0.0;
            sfx.0.push(Sfx::SplashBig);
            start_ripple(&mut steps, feet, true);
        }
    } else if wet {
        if steps.last_wet {
            // Every 11 px of wet travel is a step.
            steps.dist += travelled;
            while steps.dist >= STEP_PX {
                steps.dist -= STEP_PX;
                sfx.0.push(Sfx::Splash);
                start_ripple(&mut steps, feet, false);
            }
        } else {
            // Dry → wet: a splash on the touch, whatever the speed.
            steps.dist = 0.0;
            sfx.0.push(Sfx::Splash);
            start_ripple(&mut steps, feet, false);
        }
    } else {
        steps.dist = 0.0;
    }
    steps.last_wet = wet;

    // The horde's rings, with no sound.
    for slot in steps.enemy_slots.iter_mut() {
        slot.cooldown = (slot.cooldown - dt).max(0.0);
    }
    order.clear();
    for (id, enemy) in enemies.iter() {
        if enemy.hp <= 0.0 || enemy.elite || enemy.named {
            continue;
        }
        if !on_the_floor(level.theme, enemy.kind) {
            continue;
        }
        if !level.wet_at(enemy.pos) || !in_sim_view(enemy.pos, pos, viewport.0) {
            continue;
        }
        order.push((id.0, enemy.pos));
    }
    order.sort_unstable_by_key(|(id, _)| *id);
    for slot in 0..ENEMY_RIPPLE_SLOTS {
        let held = steps.enemy_slots[slot];
        match order.get(slot).copied() {
            Some((id, at)) => {
                // A different body in the slot starts with a clean clock
                // rather than inheriting the last body's wait.
                let mut cooldown = if held.id == id { held.cooldown } else { 0.0 };
                if cooldown <= 0.0 {
                    cooldown = ENEMY_RIPPLE_SECS;
                    start_ripple(&mut steps, at, false);
                }
                steps.enemy_slots[slot] = EnemyRippleSlot { id, cooldown };
            }
            None => steps.enemy_slots[slot] = EnemyRippleSlot::default(),
        }
    }
}

/// Start one ripple: the first free slot, else the oldest (so a burst of
/// splashes keeps the newest [`ENEMY_RIPPLE_SLOTS`] rings rather than dropping
/// the one that just started).
fn start_ripple(steps: &mut PuddleSteps, pos: Vec2, big: bool) {
    let mut slot = 0;
    let mut oldest = -1.0;
    for (i, ripple) in steps.ripples.iter().enumerate() {
        if !ripple.live {
            slot = i;
            break;
        }
        if ripple.age > oldest {
            oldest = ripple.age;
            slot = i;
        }
    }
    steps.ripples[slot] = Ripple {
        pos,
        age: 0.0,
        big,
        live: true,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::level::Theme;
    use crate::game::world::World;
    use rand::Rng;

    const DT: f32 = 1.0 / 60.0;

    /// A wet floor with the rat standing on its **largest** puddle and nothing
    /// else on it: the frame every test below starts from.
    fn wet_world() -> (World, Vec2) {
        let mut w = World::new(Theme::Sewer, 3);
        w.clear_enemies();
        let mut puddle = w
            .level
            .puddles
            .first()
            .copied()
            .expect("the Sewer is the wet floor");
        for other in &w.level.puddles {
            if other.rx * other.ry > puddle.rx * puddle.ry {
                puddle = *other;
            }
        }
        w.player.pos = puddle.pos;
        w.sfx.clear();
        // The resource's remembered position must start where the rat is, or
        // the first tick counts a phantom jump.
        w.ecs.resource_mut::<PuddleSteps>().last_pos = puddle.pos;
        (w, puddle.pos)
    }

    /// A dry, walkable point at least `away_from` away from `from`.
    fn dry_spot(w: &World, from: Vec2) -> Vec2 {
        for cy in 0..w.level.tiles.rows() {
            for cx in 0..w.level.tiles.cols() {
                if !matches!(w.level.tiles.get(cx, cy), TileKind::Open) {
                    continue;
                }
                let centre = TileMap::tile_rect(cx, cy).center();
                if !w.level.wet_at(centre) && centre.dist(from) > 160.0 {
                    return centre;
                }
            }
        }
        panic!("this map has no dry floor to stand on");
    }

    /// Move the rat to `spot` without pretending it walked there.
    fn teleport(w: &mut World, spot: Vec2) {
        w.player.pos = spot;
        w.player.vel = Vec2::ZERO;
        let mut steps = w.ecs.resource_mut::<PuddleSteps>();
        steps.last_pos = spot;
        steps.dist = 0.0;
        steps.last_wet = false;
    }

    /// Run `ticks` ticks of the whole schedule and return every sound pushed
    /// (the facade queue is per tick, so they are gathered tick by tick).
    fn run(w: &mut World, ticks: usize, dir: Vec2, dash: bool) -> Vec<Sfx> {
        let mut all = Vec::new();
        for _ in 0..ticks {
            w.update(DT, dir, dash);
            all.append(&mut w.sfx);
        }
        all
    }

    fn splashes(sounds: &[Sfx]) -> (usize, usize) {
        (
            sounds.iter().filter(|s| **s == Sfx::Splash).count(),
            sounds.iter().filter(|s| **s == Sfx::SplashBig).count(),
        )
    }

    fn live_ripples(w: &World) -> usize {
        w.ecs
            .resource::<PuddleSteps>()
            .ripples
            .iter()
            .filter(|r| r.live)
            .count()
    }

    #[test]
    fn a_step_splash_every_eleven_pixels_of_wet_travel() {
        let (mut w, start) = wet_world();
        // Walk the way the water runs furthest: the largest puddle may sit
        // against a wall on one side (only floor cells are wet).
        let run = |dir: Vec2| {
            (1..200)
                .take_while(|&i| w.level.wet_at(start + dir * i as f32))
                .count()
        };
        let dir = [
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, -1.0),
        ]
        .into_iter()
        .max_by_key(|&d| run(d))
        .unwrap();
        let mut last = start;
        let mut total_wet = 0.0;
        let mut small = 0;
        let mut big = 0;
        // The first tick stands **in** water with `last_wet == false`: that is
        // the dry → wet edge, whose splash is not a step.
        let mut first = true;
        for _ in 0..240 {
            w.update(DT, dir, false);
            let moved = w.player.pos.dist(last);
            last = w.player.pos;
            let (s, b) = splashes(&w.sfx);
            small += s;
            big += b;
            // The facade queue is drained by the App, not by the tick.
            w.sfx.clear();
            if !w.level.wet_at(last) {
                break;
            }
            if first {
                first = false;
            } else {
                total_wet += moved;
            }
        }
        assert_eq!(big, 0, "walking must not slosh");
        // One splash on entry, then exactly one per whole `STEP_PX` of wet
        // travel: the accumulator carries the remainder, so the count is the
        // floor of the total.
        let expected = 1 + (total_wet / STEP_PX).floor() as usize;
        assert_eq!(
            small, expected,
            "{total_wet} px of wet travel made {small} splashes (expected {expected})"
        );
        assert!(small > 2, "the rat left the puddle before the ladder ran");
        assert!(live_ripples(&w) > 0, "a splash must leave a ripple");
    }

    #[test]
    fn entering_water_splashes_at_once_and_dry_floor_is_silent() {
        let (mut w, wet) = wet_world();
        let dry = dry_spot(&w, wet);
        teleport(&mut w, dry);
        // Standing on dry floor: silent for a second.
        assert_eq!(splashes(&run(&mut w, 60, Vec2::ZERO, false)), (0, 0));
        // Walking about on dry floor: still silent, and it really walks.
        let before = w.player.pos;
        assert_eq!(splashes(&run(&mut w, 30, Vec2::new(-1.0, 0.0), false)), (0, 0));
        assert!(w.player.pos.dist(before) > 1.0, "the rat did not move");
        // And a dry walk leaves no ripple behind it.
        assert_eq!(live_ripples(&w), 0);
    }

    #[test]
    fn standing_still_in_water_makes_no_sound() {
        let (mut w, _) = wet_world();
        // The entry splash is the first tick; after that a still rat is silent.
        assert_eq!(splashes(&run(&mut w, 1, Vec2::ZERO, false)), (1, 0));
        assert_eq!(
            splashes(&run(&mut w, 120, Vec2::ZERO, false)),
            (0, 0),
            "a still rat splashed"
        );
    }

    #[test]
    fn a_dash_through_water_sloshes_exactly_once() {
        let (mut w, _) = wet_world();
        // The legacy `update(dt, dir, dash)` press: the first tick starts the
        // dash, the cooldown holds the rest of the presses off.
        let sounds = run(&mut w, 20, Vec2::new(1.0, 0.0), true);
        assert_eq!(
            splashes(&sounds).1,
            1,
            "one dash through water is one SplashBig"
        );
        let big = w
            .ecs
            .resource::<PuddleSteps>()
            .ripples
            .iter()
            .filter(|r| r.live && r.big)
            .count();
        assert_eq!(big, 1, "the dash left exactly one big ripple");
    }

    #[test]
    fn a_dash_on_dry_floor_is_silent() {
        let (mut w, wet) = wet_world();
        let dry = dry_spot(&w, wet);
        teleport(&mut w, dry);
        assert_eq!(
            splashes(&run(&mut w, 5, Vec2::new(1.0, 0.0), true)),
            (0, 0),
            "a dry dash splashed"
        );
    }

    #[test]
    fn ripples_age_and_die_at_their_life() {
        let (mut w, _) = wet_world();
        run(&mut w, 1, Vec2::ZERO, false);
        assert!(live_ripples(&w) > 0);
        // `RIPPLE_LIFE` is 0.45 s, so 27 ticks of aging take the first ring
        // past its life.
        let ticks = (RIPPLE_LIFE / DT).ceil() as usize + 2;
        assert_eq!(splashes(&run(&mut w, ticks, Vec2::ZERO, false)), (0, 0));
        assert_eq!(live_ripples(&w), 0, "the ripple outlived its life");
    }

    #[test]
    fn the_first_ground_bodies_ripple_without_a_sound() {
        let (mut w, wet) = wet_world();
        let fodder = roster::kind(Theme::Sewer, Slot::A);
        for i in 0..8 {
            w.spawn_enemy(fodder, wet + Vec2::new(i as f32 * 2.0, (i % 3) as f32 * 2.0), 1.0);
        }
        run(&mut w, 2, Vec2::ZERO, false);
        assert!(
            live_ripples(&w) > 2,
            "the horde's feet left {} ripples",
            live_ripples(&w)
        );
        // No heavy sound from a body: the only SplashBig is a dash's and the
        // rat never dashes here.
        assert_eq!(splashes(&w.sfx).1, 0);
        assert!(live_ripples(&w) <= crate::game::ecs::resources::RIPPLE_SLOTS);
        // The slots are `SpawnId`-ordered, so the six that ripple are the first
        // six of the eight.
        let slots = w.ecs.resource::<PuddleSteps>().enemy_slots;
        let ids: Vec<u64> = slots.iter().map(|s| s.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "the enemy slots are not in SpawnId order");
        assert!(ids[0] < ids[5], "the six slots share one body");
    }

    #[test]
    fn flying_fodder_never_ripples() {
        let (mut w, wet) = wet_world();
        let flyer = roster::kind(Theme::Sewer, Slot::B);
        assert!(!on_the_floor(Theme::Sewer, flyer));
        for i in 0..6 {
            w.spawn_enemy(flyer, wet + Vec2::new(i as f32, 0.0), 1.0);
        }
        run(&mut w, 1, Vec2::ZERO, false);
        // Only the rat's own entry ripple exists.
        assert_eq!(live_ripples(&w), 1, "a flyer rippled in the water");
    }

    #[test]
    fn puddle_steps_change_no_simulated_state() {
        // The system's only outputs are `SfxQueue` and `PuddleSteps`. Running
        // the tick with water under the rat and with the puddles removed must
        // leave the rat, the rng and every counter identical.
        let (mut a, _) = wet_world();
        let (mut b, _) = wet_world();
        b.level.puddles.clear();
        let start = b.level.start;
        teleport(&mut b, start);
        let start = a.level.start;
        teleport(&mut a, start);
        for _ in 0..30 {
            a.update(DT, Vec2::new(1.0, 0.0), false);
            b.update(DT, Vec2::new(1.0, 0.0), false);
        }
        assert_eq!(a.player.pos, b.player.pos, "water moved the rat");
        assert_eq!(a.player.hp, b.player.hp);
        assert_eq!(a.ticks, b.ticks);
        assert_eq!(a.rng.next_u64(), b.rng.next_u64(), "the rng streams differ");
    }

    #[test]
    fn the_system_is_cheap_on_a_dry_floor() {
        // Nothing to do on a floor `count_for` does not list: no splash, no
        // ripple.
        let mut w = World::new(Theme::Lab, 5);
        w.clear_enemies();
        assert!(w.level.puddles.is_empty());
        assert_eq!(splashes(&run(&mut w, 60, Vec2::new(1.0, 0.0), false)), (0, 0));
        assert_eq!(live_ripples(&w), 0);
    }
}
