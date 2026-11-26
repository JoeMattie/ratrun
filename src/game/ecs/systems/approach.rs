//! Fodder approach sounds (`WAVE4_ROSTERS.md`: "Give each floor a procedural
//! sound pair for its two fodder kinds, played on approach in view, coalesced
//! by kind with a 0.8 s floor-wide cooldown").
//!
//! While any slot-A (ground fodder) or slot-B (flying fodder) body of the
//! floor's roster is inside the simulation viewport and that slot's cooldown
//! has run out, the slot's sound ([`roster::approach_sfx`]) is pushed into
//! [`SfxQueue`] and the cooldown restarts at [`APPROACH_COOLDOWN_SECS`].
//!
//! ## Determinism
//!
//! * **No rng**, no spawn, no despawn, no write to any enemy.
//! * **Order-independent**: the test is "is *any* such body in view", so query
//!   iteration order cannot decide anything and no `SpawnId` sort is needed.
//! * **Render-independent**: "in view" is the simulation's own rectangle — the
//!   [`Viewport`] resource centred on the rat — not the camera, the zoom or
//!   the fog. (The drawn camera is clamped to the arena and shaken, so near a
//!   map edge this rectangle and the screen differ by up to half a viewport;
//!   that is the price of a run sounding the same at every zoom.)
//! * The only output is an extra [`SfxQueue`] entry. Nothing reads the queue
//!   back into the simulation.
//!
//! The cooldowns are system-local state ([`ApproachCooldowns`] in a `Local`),
//! so `World::new` needs no insert; they count simulated time (`TickInput.dt`).
//!
//! ## Schedule slot
//!
//! Anywhere after the enemies have moved this tick; the intended slot is right
//! after the per-floor enemy systems (`wave4`, the `floor_*` passes) and before
//! `reap_enemies`. It takes no `Commands`.

// Two parallel two-element arrays (seen / cooldown) read by one index.
#![allow(clippy::needless_range_loop)]

use bevy_ecs::prelude::*;

use crate::game::ecs::resources::{SfxQueue, TickInput, Viewport};
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::player::Player;
use crate::game::roster::{self, Slot};
use crate::math::Vec2;

/// Floor-wide cooldown per fodder slot, in simulated seconds.
pub const APPROACH_COOLDOWN_SECS: f32 = 0.8;

/// Seconds left before each fodder slot may sound again: `[slot A, slot B]`.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct ApproachCooldowns(pub [f32; 2]);

/// The two slots that have an approach sound, in cooldown-array order.
const FODDER_SLOTS: [Slot; 2] = [Slot::A, Slot::B];

/// Is `pos` inside the simulation's view rectangle (the viewport centred on
/// the rat)? Edges count as inside.
#[inline]
pub fn in_sim_view(pos: Vec2, player: Vec2, viewport: Vec2) -> bool {
    (pos.x - player.x).abs() <= viewport.x * 0.5 && (pos.y - player.y).abs() <= viewport.y * 0.5
}

/// [`in_sim_view`] with a margin: the same rectangle, shrunk by `inset` px on
/// every side (a weapon that draws its own burst wants its centre a little
/// inside the edge, not on it). `inset` is clamped so the rectangle cannot
/// invert.
///
/// This rectangle is ordinarily the [`Viewport`] resource at zoom 1.0,
/// centred on the point the caller passes. Laser Eyes passes
/// `spawn::widest_view(viewport)` to the ray helper below.
#[inline]
pub fn in_sim_view_inset(pos: Vec2, center: Vec2, viewport: Vec2, inset: f32) -> bool {
    let hw = (viewport.x * 0.5 - inset).max(0.0);
    let hh = (viewport.y * 0.5 - inset).max(0.0);
    (pos.x - center.x).abs() <= hw && (pos.y - center.y).abs() <= hh
}

/// How far a ray from `from` along `dir` may travel before it leaves the
/// simulation view rectangle: the same rectangle [`in_sim_view_inset`] tests,
/// centred on `center`, `viewport` px across, inset by `inset` px a side.
///
/// Tests the ray against the rectangle's forward faces. Callers choose the
/// view size without consulting the renderer. `dir` must be a unit vector
/// (every caller passes an angle). A zero direction returns `0.0`.
#[inline]
pub fn sim_view_ray_exit(from: Vec2, dir: Vec2, center: Vec2, viewport: Vec2, inset: f32) -> f32 {
    let hw = (viewport.x * 0.5 - inset).max(0.0);
    let hh = (viewport.y * 0.5 - inset).max(0.0);
    let (dx, dy) = (from.x - center.x, from.y - center.y);
    let mut t = f32::INFINITY;
    if dir.x > 1e-6 {
        t = t.min((hw - dx) / dir.x);
    } else if dir.x < -1e-6 {
        t = t.min((-hw - dx) / dir.x);
    }
    if dir.y > 1e-6 {
        t = t.min((hh - dy) / dir.y);
    } else if dir.y < -1e-6 {
        t = t.min((-hh - dy) / dir.y);
    }
    if !t.is_finite() {
        return 0.0;
    }
    t.max(0.0)
}

pub fn approach_sfx_system(
    input: Res<TickInput>,
    viewport: Res<Viewport>,
    level: Res<Level>,
    player: Res<Player>,
    mut sfx: ResMut<SfxQueue>,
    enemies: Query<&Enemy>,
    mut cooldowns: Local<ApproachCooldowns>,
) {
    for t in cooldowns.0.iter_mut() {
        *t = (*t - input.dt).max(0.0);
    }
    if cooldowns.0[0] > 0.0 && cooldowns.0[1] > 0.0 {
        return;
    }
    let theme = level.theme;
    let kinds = [roster::kind(theme, Slot::A), roster::kind(theme, Slot::B)];
    let mut seen = [false; 2];
    for e in enemies.iter() {
        // Elites and named minibosses announce themselves; they are not fodder
        // even when they wear a fodder body.
        if e.elite || e.named || e.hp <= 0.0 {
            continue;
        }
        for i in 0..2 {
            if !seen[i] && e.kind == kinds[i] && in_sim_view(e.pos, player.pos, viewport.0) {
                seen[i] = true;
            }
        }
        if seen[0] && seen[1] {
            break;
        }
    }
    for i in 0..2 {
        if seen[i] && cooldowns.0[i] <= 0.0 {
            if let Some(sound) = roster::approach_sfx(theme, FODDER_SLOTS[i]) {
                sfx.0.push(sound);
                cooldowns.0[i] = APPROACH_COOLDOWN_SECS;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sfx;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    const VIEW: Vec2 = Vec2 { x: 240.0, y: 92.0 };

    fn floor(theme: Theme) -> World {
        let mut w = World::new(theme, 11);
        w.clear_enemies();
        w.viewport = VIEW;
        w.sfx.clear();
        w
    }

    /// One tick of the system alone; returns the approach sounds it pushed.
    fn tick(w: &mut World) -> Vec<Sfx> {
        w.sfx.clear();
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(approach_sfx_system).unwrap();
        w.end_tick();
        std::mem::take(&mut w.sfx)
    }

    fn count(w: &mut World, ticks: usize, sound: Sfx) -> usize {
        (0..ticks).map(|_| tick(w).iter().filter(|&&s| s == sound).count()).sum()
    }

    #[test]
    fn every_floor_maps_its_two_fodder_slots_and_nothing_else() {
        let mut all = Vec::new();
        for theme in Theme::playable() {
            for slot in Slot::ALL {
                let sound = roster::approach_sfx(theme, slot);
                assert_eq!(sound.is_some(), matches!(slot, Slot::A | Slot::B), "{theme:?} {slot:?}");
                all.extend(sound);
            }
        }
        assert_eq!(all.len(), 12);
        for (i, a) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(a), "{a:?} is used twice");
        }
    }

    #[test]
    fn a_fodder_body_in_view_sounds_once_per_cooldown() {
        for theme in Theme::playable() {
            let mut w = floor(theme);
            let p = w.player.pos;
            w.spawn_enemy(roster::kind(theme, Slot::A), p + Vec2::new(30.0, 0.0), 1.0);
            // A crowd is still one sound.
            w.spawn_enemy(roster::kind(theme, Slot::A), p + Vec2::new(-30.0, 10.0), 1.0);
            let a = roster::approach_sfx(theme, Slot::A).unwrap();
            let b = roster::approach_sfx(theme, Slot::B).unwrap();
            assert_eq!(tick(&mut w), vec![a], "{theme:?}: the first tick in view sounds");
            // 0.8 s is 48 ticks: silent for the next 46, at most one more by 50.
            assert_eq!(count(&mut w, 46, a), 0, "{theme:?}: silent inside the cooldown");
            assert_eq!(count(&mut w, 4, a), 1, "{theme:?}: sounds again after 0.8 s");
            assert_eq!(count(&mut w, 40, b), 0, "{theme:?}: slot B never sounded");
        }
    }

    #[test]
    fn the_two_slots_keep_separate_cooldowns() {
        let mut w = floor(Theme::Sewer);
        let p = w.player.pos;
        w.spawn_enemy(roster::kind(Theme::Sewer, Slot::A), p + Vec2::new(30.0, 0.0), 1.0);
        assert_eq!(tick(&mut w), vec![Sfx::ApproachPipeClaws]);
        for _ in 0..10 {
            assert!(tick(&mut w).is_empty());
        }
        // The flyer arrives mid-cooldown of slot A and sounds at once.
        w.spawn_enemy(roster::kind(Theme::Sewer, Slot::B), p + Vec2::new(0.0, -20.0), 1.0);
        assert_eq!(tick(&mut w), vec![Sfx::ApproachWingLeather]);
    }

    #[test]
    fn bodies_out_of_view_and_other_slots_are_silent() {
        let mut w = floor(Theme::Sewer);
        let p = w.player.pos;
        // Just past the right edge, and far below.
        w.spawn_enemy(roster::kind(Theme::Sewer, Slot::A), p + Vec2::new(VIEW.x * 0.5 + 4.0, 0.0), 1.0);
        w.spawn_enemy(roster::kind(Theme::Sewer, Slot::B), p + Vec2::new(0.0, VIEW.y * 0.5 + 4.0), 1.0);
        // A chaser in plain view has no approach sound.
        w.spawn_enemy(roster::kind(Theme::Sewer, Slot::C), p + Vec2::new(20.0, 0.0), 1.0);
        for _ in 0..120 {
            assert!(tick(&mut w).is_empty());
        }
    }

    #[test]
    fn elites_and_named_bodies_are_not_fodder() {
        let mut w = floor(Theme::Sewer);
        let p = w.player.pos;
        let e = w.spawn_enemy(roster::kind(Theme::Sewer, Slot::A), p + Vec2::new(30.0, 0.0), 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().elite = true;
        let n = w.spawn_enemy(roster::kind(Theme::Sewer, Slot::B), p + Vec2::new(-30.0, 0.0), 1.0);
        w.ecs.get_mut::<Enemy>(n).unwrap().named = true;
        for _ in 0..60 {
            assert!(tick(&mut w).is_empty());
        }
    }

    #[test]
    fn the_view_rectangle_is_the_sim_viewport_centred_on_the_rat() {
        let c = Vec2::new(100.0, 100.0);
        assert!(in_sim_view(c, c, VIEW));
        assert!(in_sim_view(c + Vec2::new(120.0, -46.0), c, VIEW), "edges are inside");
        assert!(!in_sim_view(c + Vec2::new(120.5, 0.0), c, VIEW));
        assert!(!in_sim_view(c + Vec2::new(0.0, 46.5), c, VIEW));
        assert!(!in_sim_view(c, c + Vec2::new(500.0, 0.0), Vec2::ZERO));
    }

    /// The inset rectangle is the same rectangle with a margin, and a margin
    /// wider than the view cannot invert it into "everything is outside".
    #[test]
    fn the_inset_view_shrinks_every_side() {
        let c = Vec2::new(0.0, 0.0);
        assert!(in_sim_view_inset(Vec2::new(116.0, 0.0), c, VIEW, 4.0));
        assert!(!in_sim_view_inset(Vec2::new(116.5, 0.0), c, VIEW, 4.0));
        assert!(!in_sim_view_inset(Vec2::new(0.0, 42.5), c, VIEW, 4.0));
        // A degenerate inset collapses the rectangle onto its centre only.
        assert!(in_sim_view_inset(Vec2::ZERO, c, VIEW, 400.0));
        assert!(!in_sim_view_inset(Vec2::new(1.0, 0.0), c, VIEW, 400.0));
    }

    /// The beam exit is the slab test against the same rectangle: a beam along
    /// an axis is cut by the near wall, and a diagonal one by whichever wall it
    /// meets first.
    #[test]
    fn a_beam_leaves_the_view_where_the_rectangle_ends() {
        let c = Vec2::new(0.0, 0.0);
        // Straight out: the x wall.
        let t = sim_view_ray_exit(c, Vec2::new(1.0, 0.0), c, VIEW, 0.0);
        assert!((t - 120.0).abs() < 1e-3, "{t}");
        let t = sim_view_ray_exit(c, Vec2::new(0.0, -1.0), c, VIEW, 0.0);
        assert!((t - 46.0).abs() < 1e-3, "{t}");
        // 45 degrees up-right: the y wall (46) comes first.
        let d = Vec2::from_angle(std::f32::consts::FRAC_PI_4);
        let t = sim_view_ray_exit(c, d, c, VIEW, 0.0);
        assert!((t - 46.0 / d.y).abs() < 1e-3, "{t}");
        // The inset pulls the exit in by that much on an axis.
        let t = sim_view_ray_exit(c, Vec2::new(1.0, 0.0), c, VIEW, 4.0);
        assert!((t - 116.0).abs() < 1e-3, "{t}");
        // A ray that starts off-centre leaves earlier on that side.
        let t = sim_view_ray_exit(Vec2::new(100.0, 0.0), Vec2::new(1.0, 0.0), c, VIEW, 0.0);
        assert!((t - 20.0).abs() < 1e-3, "{t}");
        // Degenerate: no direction, no beam.
        assert_eq!(sim_view_ray_exit(c, Vec2::ZERO, c, VIEW, 0.0), 0.0);
        assert_eq!(sim_view_ray_exit(c, Vec2::new(1.0, 0.0), c, Vec2::ZERO, 0.0), 0.0);
    }
}
