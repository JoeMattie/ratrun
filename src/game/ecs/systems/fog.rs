//! Fog-of-war reveal.
//!
//! Runs directly after `player::player_system` (schedule slot 2) and only
//! writes the [`Fog`] resource: it reads the player position and the level's
//! tile grid, floods the explored set and never touches the rng, an entity or a
//! simulated value. Adding it to the chain therefore leaves every golden trace
//! untouched (proved by `game::ecs::tests::fog_never_moves_the_simulation`).
//!
//! The flood itself lives in [`crate::game::fog`]; the rule and radius are
//! documented there.

use bevy_ecs::prelude::*;

use crate::game::fog::Fog;
use crate::game::level::Level;
use crate::game::player::Player;

/// Reveal what the rat can see from its current cell.
///
/// The fade clock advances every tick (so a "seen recently" glow dims while the
/// player stands still) but the flood only reruns when the player's cell
/// changes — the same cadence the nav flow field uses.
pub fn reveal_system(level: Res<Level>, player: Res<Player>, mut fog: ResMut<Fog>) {
    fog.tick();
    if !fog.reveal_enabled {
        return;
    }
    fog.update_from(&level, player.pos);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::level::Theme;
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Drive exactly one system through the facade harness.
    fn run_reveal(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(reveal_system).unwrap();
        w.end_tick();
    }

    #[test]
    fn the_reveal_tracks_the_player_and_reveals_the_start_room() {
        let mut w = World::new(Theme::Sewer, 7);
        // `World::new` has not ticked yet: the start room is still dark.
        assert_eq!(w.fog.seen_count(), 0);
        let start = w.player.pos;
        let (cx, cy) = crate::game::collision::cell_of(start);
        run_reveal(&mut w, 1.0 / 60.0);
        assert_eq!(w.fog.last_cell(), (cx, cy));
        assert!(w.fog.is_seen(cx, cy));
        assert!(
            w.fog.seen_count() > 20,
            "the start room should reveal a patch of floor, got {}",
            w.fog.seen_count()
        );
    }

    #[test]
    fn standing_still_does_not_rerun_the_flood() {
        let mut w = World::new(Theme::Sewer, 7);
        run_reveal(&mut w, 1.0 / 60.0);
        let seen = w.fog.seen_count();
        let clock = w.fog.clock();
        for _ in 0..10 {
            run_reveal(&mut w, 1.0 / 60.0);
        }
        assert_eq!(w.fog.seen_count(), seen);
        assert_eq!(w.fog.clock(), clock + 10, "the fade clock still advances");
    }

    #[test]
    fn walking_reveals_more_and_never_forgets() {
        let mut w = World::new(Theme::Kitchen, 3);
        run_reveal(&mut w, 1.0 / 60.0);
        let first = w.fog.seen_count();
        let (cx, cy) = w.fog.last_cell();
        // Teleport a screen to the right on open floor and reveal again.
        let tiles: &TileMap = &w.level.tiles;
        for step in 1..40 {
            let target = TileMap::tile_rect(cx + step, cy).center();
            if tiles.passable(cx + step, cy, crate::game::collision::SizeClass::Small) {
                w.player.pos = target;
                break;
            }
        }
        run_reveal(&mut w, 1.0 / 60.0);
        assert!(w.fog.seen_count() > first);
        assert!(w.fog.is_seen(cx, cy), "explored cells stay explored");
    }

    /// The reveal system must not move the simulation at all: the same seed
    /// with the reveal switched off has to agree on every sampled value, tick
    /// for tick, including the rng state (a `next_u64` draw compared on both
    /// sides).
    ///
    /// This is the in-test equivalent of "the golden traces do not move": the
    /// system reads `Player` + `Level` and writes only `Fog`.
    #[test]
    fn fog_never_moves_the_simulation() {
        use rand::Rng;
        const DT: f32 = 1.0 / 60.0;
        fn dir_at(tick: u64) -> Vec2 {
            let phase = tick as f32 * 0.013;
            Vec2::new(crate::detmath::sin(phase), crate::detmath::cos(phase * 0.7))
        }
        let mut on = World::new(Theme::Sewer, 11);
        let mut off = World::new(Theme::Sewer, 11);
        off.fog.reveal_enabled = false;
        on.viewport = Vec2::new(160.0, 168.0);
        off.viewport = on.viewport;
        for tick in 0..1_800u64 {
            let (dir, dash) = (dir_at(tick), tick % 157 == 0);
            on.update(DT, dir, dash);
            off.update(DT, dir, dash);
            if tick % 30 != 0 {
                continue;
            }
            let ctx = format!("tick {tick}");
            assert_eq!(on.elapsed.to_bits(), off.elapsed.to_bits(), "{ctx}");
            assert_eq!(on.score, off.score, "{ctx}");
            assert_eq!(on.kills, off.kills, "{ctx}");
            assert_eq!(
                on.player.pos.x.to_bits(),
                off.player.pos.x.to_bits(),
                "{ctx}"
            );
            assert_eq!(
                on.player.pos.y.to_bits(),
                off.player.pos.y.to_bits(),
                "{ctx}"
            );
            assert_eq!(on.player.hp.to_bits(), off.player.hp.to_bits(), "{ctx}");
            let (a, b) = (on.enemies(), off.enemies());
            assert_eq!(a.len(), b.len(), "{ctx}");
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(x.pos.x.to_bits(), y.pos.x.to_bits(), "{ctx}");
                assert_eq!(x.pos.y.to_bits(), y.pos.y.to_bits(), "{ctx}");
                assert_eq!(x.hp.to_bits(), y.hp.to_bits(), "{ctx}");
            }
            assert_eq!(on.bullets().len(), off.bullets().len(), "{ctx}");
            assert_eq!(on.gems().len(), off.gems().len(), "{ctx}");
            assert_eq!(on.pools().len(), off.pools().len(), "{ctx}");
            assert_eq!(on.particles.len(), off.particles.len(), "{ctx}");
            assert_eq!(on.damage_numbers.len(), off.damage_numbers.len(), "{ctx}");
            assert_eq!(on.rng.next_u64(), off.rng.next_u64(), "rng diverged at {ctx}");
        }
        assert!(on.fog().seen_count() > 0, "the reveal never ran");
        assert_eq!(off.fog().seen_count(), 0);
    }

    #[test]
    fn the_reveal_can_be_switched_off() {
        let mut w = World::new(Theme::Lab, 11);
        w.fog.reveal_enabled = false;
        for _ in 0..5 {
            run_reveal(&mut w, 1.0 / 60.0);
        }
        assert_eq!(w.fog.seen_count(), 0);
    }
}
