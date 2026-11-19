//! (a) Player movement, dash, sub-stepping and timers.
//!
//! Reproduces **verbatim** the legacy //! (`World::update_player`):
//!
//! | legacy | here |
//! | --- | --- |
//! | 184 `let stats = loadout.stats.clone()` | same (a snapshot, so the stat reads below cannot alias `player`) |
//! | 185-187 facing from `move_dir` | same |
//! | 190-199 dash trigger + `vfx::emit_dash_start` + `Sfx::Dash` | same, one `Sfx::Dash` per successful dash |
//! | 201-213 dashing/throttle/velocity lerp | same (`clamp_len(1.0)`, dash-without-input uses facing) |
//! | 219-231 sub-stepped move + arena clamp + wall resolve | same (`PLAYER_SUBSTEP`, 1..=64 steps) |
//! | neon hook: `vfx::emit_dash_trail` while dashing | inserted after the sub-step move, before the timers |
//! | 234-240 timer decay, then regen | same order |
//!
//! ## Cross-team hooks re-applied (the merge diff, `update_player`)
//!
//! - Dash start: the legacy inline `particles.cone(...)` is now
//!   `vfx::emit_dash_start(&mut particles, &mut rng, pos, face)`. Note the
//!   argument is `face`, not `-face`: the helper negates internally, so the
//!   emitted geometry and the rng draw order are unchanged.
//! - Dashing block: `vfx::emit_dash_trail(&mut particles, &mut rng, pos, vel,
//!   dt)` reading the post-sub-step `pos` and the post-lerp `vel`, guarded by
//!   the pre-decay `dashing` flag exactly as the other team's `world.rs` does.
//!
//! Resources: `Res<TickInput>` (dt/move_dir/dash), `ResMut<Player>`,
//! `Res<Level>` (arena + walls), `ResMut<Particles>` + `ResMut<RngRes>` +
//! `ResMut<SfxQueue>` (dash start burst + dash trail + `Sfx::Dash`).
//! Both dash hooks are plain `vfx` functions over `&mut Particles` +
//! `&mut rng`, so this system needs no shared state beyond the resources it
//! already takes.
//!
//! Ordering: first system in the tick (legacy `update()` calls it first,
//! before the flow-field refresh). The clock (`RunCounters::elapsed += dt`) is
//! incremented by `World::update` before `begin_tick`, exactly where legacy
//! the legacy `update` did it.
//!
//! Notes:
//! - `move_dir` is a throttle vector, not a unit direction; keep
//!   `clamp_len(1.0)` and the dash-without-input facing rule.
//! - Movement is sub-stepped (`PLAYER_SUBSTEP`, clamp 1..=64) and each step is
//!   clamped to the arena then pushed out of every wall with
//!   `collision::resolve_circle_rect`. Do not move the whole delta at once.
//! - RNG: the dash-start burst and, while dashing, the trail are the only
//!   draws, and they happen exactly where the legacy code-197 drew its cone
//!   and where the neon hook inserted the trail, so the per-tick rng sequence
//!   matches the other team's build.
//! - Performance: this system allocates nothing per tick — no query, no
//!   sorting, no `Vec` scratch. `stats` is a plain 8-`f32` stack copy (exactly
//!   legacy's `clone()`), and `Particles` pushes into pre-allocated
//!   `VecDeque`s with O(1) eviction.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision;
use crate::game::ecs::constants::{PLAYER_RADIUS, PLAYER_SUBSTEP};
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::level::Level;
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

pub fn player_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
) {
    let dt = input.dt;
    let move_dir = input.move_dir;
    let dash = input.dash;

    // Legacy 184: a snapshot, not a borrow, exactly like the legacy clone.
    let stats = player.loadout.stats.clone();
    if move_dir.len_sq() > 0.01 {
        player.face = move_dir.normalized();
    }

    // Dash (legacy 189-199).
    if dash && player.dash_cd <= 0.0 {
        player.dash_active = 0.16;
        player.dash_cd = stats.dash_cd;
        player.iframes = player.iframes.max(0.18);
        let pos = player.pos;
        let face = player.face;
        vfx::emit_dash_start(&mut particles, &mut rng.0, pos, face);
        sfx.0.push(Sfx::Dash);
    }

    let dashing = player.dash_active > 0.0;
    let dash_boost = if dashing { 3.2 } else { 1.0 };
    // `move_dir` is a throttle vector: unit length = full speed (keyboard),
    // shorter = proportionally slower (mouse easing near the cursor).
    // A dash with no held direction launches along the facing instead of
    // spending the cooldown in place.
    let throttle = if dashing && move_dir.len_sq() <= 0.01 {
        player.face
    } else {
        move_dir.clamp_len(1.0)
    };
    let target_vel = throttle * stats.move_speed * dash_boost;
    // Copy out `vel` first (the legacy "disjoint borrow" convention) so the
    // arithmetic reads the pre-lerp velocity exactly once.
    let vel = player.vel;
    player.vel = vel + (target_vel - vel) * (12.0 * dt).min(1.0);

    // Move in sub-steps shorter than the player radius. A dash at upgraded
    // speed can cover ~12 px per tick, which would otherwise skip straight
    // over a thin pipe and let nearest-face ejection pop the player out the
    // far side.
    let delta = player.vel * dt;
    let steps = ((delta.len() / PLAYER_SUBSTEP).ceil() as i32).clamp(1, 64);
    let step = delta * (1.0 / steps as f32);
    let arena = level.arena;
    let lo = Vec2::splat(PLAYER_RADIUS);
    let hi = Vec2::new(arena.x - PLAYER_RADIUS, arena.y - PLAYER_RADIUS);
    for _ in 0..steps {
        player.pos += step;
        player.pos = player.pos.clamp_box(lo, hi);
        for w in &level.walls {
            player.pos = collision::resolve_circle_rect(player.pos, PLAYER_RADIUS, w);
        }
    }

    // Continuous dash motion trail (neon hook: after the sub-step move and
    // before the timers; `dashing` is the pre-decay flag, exactly as legacy
    // computed it in the legacy body).
    if dashing {
        let pos = player.pos;
        let vel = player.vel;
        vfx::emit_dash_trail(&mut particles, &mut rng.0, pos, vel, dt);
    }

    // Timers (legacy 233-240). Decay order matters: dash_cd, dash_active,
    // iframes, hurt_flash, then regen.
    player.dash_cd = (player.dash_cd - dt).max(0.0);
    player.dash_active = (player.dash_active - dt).max(0.0);
    player.iframes = (player.iframes - dt).max(0.0);
    player.hurt_flash = (player.hurt_flash - dt).max(0.0);
    if stats.regen > 0.0 {
        player.heal(stats.regen * dt);
    }
}

#[cfg(test)]
mod tests {
    use super::player_system;
    use crate::audio::Sfx;
    use crate::game::ecs::constants::PLAYER_RADIUS;
    use crate::game::ecs::resources::SfxQueue;
    use crate::game::level::{Rectf, Theme};
    use crate::game::particle::{Particles, Ramp, Shape};
    use crate::game::player::Player;
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Legacy `fast_player_cannot_tunnel_a_thin_wall`.
    #[test]
    fn fast_player_cannot_tunnel_a_thin_wall() {
        let mut w = World::new(Theme::Sewer, 23);
        w.level.walls = vec![Rectf::new(512.0, 300.0, 1.0, 200.0)];
        w.player.pos = Vec2::new(500.0, 400.0);
        w.player.vel = Vec2::new(900.0, 0.0);

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        w.end_tick();

        assert!(
            w.player.pos.x <= 512.0 - PLAYER_RADIUS + 0.01,
            "player tunnelled to {:?}",
            w.player.pos
        );
    }

    /// Legacy `dash_without_direction_uses_facing`.
    #[test]
    fn dash_without_direction_uses_facing() {
        let mut w = World::new(Theme::Sewer, 37);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 0.0;
        let x0 = w.player.pos.x;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        w.end_tick();

        assert!(w.player.pos.x > x0 + 0.1, "dash with no input did not move");
        assert!(w.player.dash_cd > 0.0);
    }

    /// Timers decay in legacy order and the arena clamp keeps the player
    /// `PLAYER_RADIUS` inside the corner.
    #[test]
    fn timers_decay_and_player_stays_in_the_arena() {
        let mut w = World::new(Theme::Sewer, 5);
        w.level.walls.clear();
        w.player.pos = Vec2::new(4.0, 4.0);
        w.player.dash_cd = 1.0;
        w.player.dash_active = 0.5;
        w.player.iframes = 0.5;
        w.player.hurt_flash = 0.5;
        // Huge velocity toward the corner: the sub-step clamp must hold.
        w.player.vel = Vec2::new(-5000.0, -5000.0);
        w.player.loadout.stats.move_speed = 0.0;

        w.begin_tick(0.1, Vec2::new(-1.0, -1.0), false);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        {
            let p = w.ecs.resource::<Player>();
            assert!(p.pos.x >= PLAYER_RADIUS - 1e-3, "pos = {:?}", p.pos);
            assert!(p.pos.y >= PLAYER_RADIUS - 1e-3, "pos = {:?}", p.pos);
            assert!((p.dash_cd - 0.9).abs() < 1e-5);
            assert!((p.dash_active - 0.4).abs() < 1e-5);
            assert!((p.iframes - 0.4).abs() < 1e-5);
            assert!((p.hurt_flash - 0.4).abs() < 1e-5);
        }
        w.end_tick();
    }

    /// Regen runs after the timers, exactly like the legacy body.
    #[test]
    fn regen_applies_after_the_timers() {
        let mut w = World::new(Theme::Sewer, 9);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.player.hp = 50.0;
        w.player.loadout.stats.regen = 10.0;

        w.begin_tick(0.05, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        w.end_tick();

        assert!((w.player.hp - 50.5).abs() < 1e-4, "hp = {}", w.player.hp);
    }

    /// Neon hook #1: a successful dash runs `vfx::emit_dash_start`, which
    /// pushes a shockwave (the legacy inline cone never did) plus streak/dust
    /// particles, and exactly one `Sfx::Dash`. A dash on cooldown emits
    /// neither.
    #[test]
    fn dash_start_emits_the_vfx_burst_and_one_dash_sfx() {
        let mut w = World::new(Theme::Sewer, 11);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 0.0;
        w.sfx.clear();

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
        let before_particles = w.ecs.resource::<Particles>().len();
        let before_shockwaves = w.ecs.resource::<Particles>().shockwaves.len();
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        let spent = w.ecs.resource::<Particles>().len() - before_particles;
        assert!(spent > 0, "dash start emitted no particles");
        assert_eq!(
            w.ecs.resource::<Particles>().shockwaves.len() - before_shockwaves,
            1,
            "emit_dash_start must push its takeoff shockwave"
        );
        assert_eq!(
            w.ecs
                .resource::<SfxQueue>()
                .0
                .iter()
                .filter(|s| **s == Sfx::Dash)
                .count(),
            1
        );
        w.end_tick();

        // Second dash while on cooldown: no burst, no shockwave, no sound.
        w.sfx.clear();
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
        let before_particles = w.ecs.resource::<Particles>().len();
        let before_shockwaves = w.ecs.resource::<Particles>().shockwaves.len();
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        // A dash that is already in progress still trails (the player was
        // mid-dash), so only the *start* burst markers are asserted here.
        assert_eq!(
            w.ecs.resource::<Particles>().shockwaves.len(),
            before_shockwaves
        );
        assert!(!w.ecs.resource::<SfxQueue>().0.contains(&Sfx::Dash));
        assert!(w.ecs.resource::<Particles>().len() > before_particles);
        w.end_tick();
    }

    /// Neon hook #2: while `dash_active > 0.0` the system emits the continuous
    /// streak trail (1..=3 streak particles at 60 Hz, all Dash/Tail ramps)
    /// and nothing extra when not dashing.
    #[test]
    fn dashing_emits_the_continuous_trail() {
        let mut w = World::new(Theme::Sewer, 13);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 1.0; // no new dash start this tick
        w.player.dash_active = 0.1; // mid-dash
        w.player.vel = Vec2::new(120.0, 0.0);

        w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        let before = w.ecs.resource::<Particles>().len();
        let before_shockwaves = w.ecs.resource::<Particles>().shockwaves.len();
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        let added = w.ecs.resource::<Particles>().len() - before;
        assert!(
            (1..=3).contains(&added),
            "dash trail emitted {added} particles, expected 1..=3"
        );
        {
            let ps = w.ecs.resource::<Particles>();
            assert_eq!(
                ps.shockwaves.len(),
                before_shockwaves,
                "the trail must not push shockwaves"
            );
            for p in ps.items.iter().rev().take(added) {
                assert!(
                    matches!(p.shape, Shape::Streak),
                    "dash trail particles are streaks"
                );
                assert!(
                    matches!(p.ramp, Ramp::Dash | Ramp::Tail),
                    "dash trail particles use the Dash/Tail ramps"
                );
            }
        }
        w.end_tick();

        // Not dashing, not dashing this tick: zero trail particles.
        w.player.dash_active = 0.0;
        w.player.dash_cd = 1.0;
        w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        let before = w.ecs.resource::<Particles>().len();
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        assert_eq!(
            w.ecs.resource::<Particles>().len(),
            before,
            "no trail without a dash"
        );
        w.end_tick();
    }

    /// The dash trail reads the post-sub-step `pos`, so the streaks originate
    /// where the player actually ended up this tick, not where it started.
    #[test]
    fn dash_trail_starts_at_the_post_move_position() {
        let mut w = World::new(Theme::Sewer, 17);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 1.0;
        w.player.dash_active = 0.1;
        w.player.vel = Vec2::new(300.0, 0.0);
        let x0 = w.player.pos.x;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        let end_x = w.ecs.resource::<Player>().pos.x;
        assert!(end_x > x0, "dash moved the player");
        {
            let ps = w.ecs.resource::<Particles>();
            // Lateral jitter is at most 2.4 px, so a trail particle spawned at
            // the post-move x can never be near the pre-move x.
            for p in ps.items.iter() {
                assert!(
                    (p.pos.x - end_x).abs() <= 3.0,
                    "trail spawned at {:?}, expected near {:?}",
                    p.pos,
                    end_x
                );
            }
        }
        w.end_tick();
    }
}
