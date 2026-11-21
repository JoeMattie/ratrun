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
//! `Res<Level>` (arena + tile grid), `ResMut<Particles>` + `ResMut<RngRes>` +
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
//!   clamped to the arena then pushed out of the walls it overlaps with
//!   `TileMap::resolve_circle` (a lookup in the cells around the rat, with the
//!   player's velocity as the one-way-door permission). Do not move the whole
//!   delta at once.
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
use crate::game::collision::SizeClass;
use crate::game::ecs::constants::{PLAYER_RADIUS, PLAYER_SUBSTEP};
use crate::game::ecs::resources::{DashTrack, RngRes, SfxQueue, TickInput};
use crate::game::level::Level;
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

/// How long the on-rat gauge and the HUD's dash module stay lit once the dash
/// is back.
const DASH_READY_SECS: f32 = 0.35;
/// How long a refused dash shakes the gauge and turns the HUD module red. Also
/// the throttle on the refused blip (see `player_system`).
pub const DASH_DENIED_SECS: f32 = 0.16;

pub fn player_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
    // w3-weapons-b: the dash owner's record of what the dash actually did, so
    // the Super Dash weapon never has to guess (see the hooks below).
    mut dash_track: ResMut<DashTrack>,
) {
    let dt = input.dt;
    let move_dir = input.move_dir;
    let dash = input.dash;

    // Legacy 184: a snapshot, not a borrow, exactly like the legacy clone.
    let stats = player.loadout.stats;
    if move_dir.len_sq() > 0.01 {
        player.face = move_dir.normalized();
    }

    // Dash (legacy 189-199). `was_charging` is read here, before the trigger,
    // so the ready tick below is the tick the *old* cooldown ran out (a dash
    // that starts now must not also announce itself as ready).
    let was_charging = player.dash_cd > 0.0;
    if dash && player.dash_cd <= 0.0 {
        player.dash_active = 0.16;
        player.dash_cd = stats.dash_cd;
        player.iframes = player.iframes.max(0.18);
        let pos = player.pos;
        let face = player.face;
        vfx::emit_dash_start(&mut particles, &mut rng.0, pos, face);
        sfx.0.push(Sfx::Dash);
        // ---- w3-weapons-b hook 1/2: a *successful* dash just began --------
        // Only this branch (a live press with the cooldown ready) reaches here,
        // so a blocked or cooling dash buys no Super Dash hit by construction.
        dash_track.begin();
    } else if dash && player.dash_denied <= 0.0 {
        // Asked for a dash that is not ready: never silent. The timer throttles
        // the feedback so a held (auto-repeating) key gives one refused blip
        // and one gauge shake per 0.16 s instead of machine-gunning them.
        player.dash_denied = DASH_DENIED_SECS;
        sfx.0.push(Sfx::DashDenied);
    }
    // ---- w3-weapons-b hook 2/2: record the segment this tick travels -------
    // `dashing` is the pre-decay flag, so the dash's last tick still counts and
    // the tick after it records nothing.
    let w3b_from = player.pos;

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
        // Where this sub-step started, so the dash records the legs that
        // really happened (a sub-step a wall pushes back on is no movement and
        // no leg).
        let leg_from = player.pos;
        player.pos += step;
        player.pos = player.pos.clamp_box(lo, hi);
        // One tile-grid query per sub-step: the cells around the rat, never
        // the level's walls. `Some(player.vel)` is the one-way-door
        // permission — it is the only entity that may push a gate leaf open,
        // every enemy passes `None`.
        let vel = player.vel;
        player.pos = level
            .tiles
            .resolve_circle(player.pos, PLAYER_RADIUS, SizeClass::Small, Some(vel));
        if dashing {
            dash_track.push_leg(leg_from, player.pos);
        }
    }

    // ---- w3-weapons-b hook 2/2 (see above): the travelled segment, exactly
    // as the sub-step move actually moved the rat (a dash stopped by a wall
    // records a short segment, which is why a blocked dash hits nothing).
    dash_track.record(w3b_from, player.pos, dashing);

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
    // Pure-visual dash feedback timers (no sim reads them, no rng involved).
    player.dash_ready_flash = (player.dash_ready_flash - dt).max(0.0);
    player.dash_denied = (player.dash_denied - dt).max(0.0);
    if was_charging && player.dash_cd <= 0.0 {
        // The dash is back: the gauge pops bright and a quiet tick lands.
        player.dash_ready_flash = DASH_READY_SECS;
        sfx.0.push(Sfx::DashReady);
    }
    if stats.regen > 0.0 {
        player.heal(stats.regen * dt);
    }
}

#[cfg(test)]
mod tests {
    use super::player_system;
    use super::DASH_DENIED_SECS;
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::constants::PLAYER_RADIUS;
    use crate::game::ecs::resources::SfxQueue;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::particle::{Particles, Ramp, Shape};
    use crate::game::player::Player;
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Strip the generated layout's walls so the test measures the movement
    /// system instead of the map (what `Level::walls.clear()` used to do
    /// before the tile grid owned collision).
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

    /// Legacy `fast_player_cannot_tunnel_a_thin_wall`: a dash-speed step must
    /// not jump a one-cell (`WallKind::Maze`, 8 px spine) partition.
    #[test]
    fn fast_player_cannot_tunnel_a_thin_wall() {
        let mut w = World::new(Theme::Sewer, 23);
        open_map(&mut w);
        // Cell column 32 (world x 512..528) is an 8 px spine at 516..524.
        w.level.tiles.fill_cells(
            32,
            18,
            1,
            14,
            TileKind::Wall(WallKind::Maze),
        );
        w.level.tiles.rebuild_masks();
        w.player.pos = Vec2::new(500.0, 400.0);
        w.player.vel = Vec2::new(900.0, 0.0);

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        w.end_tick();

        assert!(
            w.player.pos.x <= 516.0 - PLAYER_RADIUS + 0.01,
            "player tunnelled to {:?}",
            w.player.pos
        );
    }

    /// The player is the only entity that may push a one-way gate leaf open:
    /// walking along its allowed direction it gets through, walking back
    /// against it — or an enemy, which passes no velocity — does not.
    #[test]
    fn one_way_gate_opens_for_the_player_only() {
        use crate::game::art_walls::Cardinal;

        let gate_cell = (40, 40);
        let lift = |w: &mut World| {
            open_map(w);
            w.level.tiles.set(
                gate_cell.0,
                gate_cell.1,
                TileKind::Wall(WallKind::OneWayDoor {
                    dir: Cardinal::East,
                }),
            );
            w.level.tiles.rebuild_masks();
        };
        let centre = TileMap::tile_rect(gate_cell.0, gate_cell.1).center();

        // East (the allowed direction): through the cell.
        let mut w = World::new(Theme::Sewer, 41);
        lift(&mut w);
        w.player.pos = Vec2::new(centre.x - 20.0, centre.y);
        w.player.vel = Vec2::new(120.0, 0.0);
        for _ in 0..40 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
            w.ecs
                .run_system_cached(player_system)
                .expect("player system must run");
            w.end_tick();
        }
        assert!(
            w.player.pos.x > centre.x + 8.0,
            "the rat was refused its own gate at {:?}",
            w.player.pos
        );

        // West (against the gate): refused, never inside the leaf.
        let mut w = World::new(Theme::Sewer, 43);
        lift(&mut w);
        w.player.pos = Vec2::new(centre.x + 20.0, centre.y);
        w.player.vel = Vec2::new(-120.0, 0.0);
        for _ in 0..40 {
            w.begin_tick(1.0 / 60.0, Vec2::new(-1.0, 0.0), false);
            w.ecs
                .run_system_cached(player_system)
                .expect("player system must run");
            w.end_tick();
        }
        // The leaf is 4 px wide, 6 px into the cell: a radius-3 rat pushed
        // west stops on its east face.
        let gate = crate::game::art_walls::one_way_gate_rect(
            gate_cell.0,
            gate_cell.1,
            Cardinal::East,
        );
        assert!(
            w.player.pos.x >= gate.right() + PLAYER_RADIUS - 0.01,
            "the rat walked a gate against its direction to {:?} (leaf ends at {})",
            w.player.pos,
            gate.right()
        );
    }

    /// Legacy `dash_without_direction_uses_facing`.
    #[test]
    fn dash_without_direction_uses_facing() {
        let mut w = World::new(Theme::Sewer, 37);
        open_map(&mut w);
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
        open_map(&mut w);
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
        open_map(&mut w);
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

    /// Run one tick of the player system with the given dash press.
    fn tick(w: &mut World, dash: bool) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, dash);
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
        w.end_tick();
    }

    /// The dash coming back is announced exactly once: a quiet `Sfx::DashReady`
    /// on the tick the cooldown reaches zero, plus the gauge's pop timer. The
    /// ticks after it stay silent.
    #[test]
    fn the_dash_ready_tick_is_announced_once_and_quietly() {
        let mut w = World::new(Theme::Sewer, 29);
        open_map(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 0.01;
        w.player.dash_ready_flash = 0.0;
        w.sfx.clear();

        tick(&mut w, false);
        assert_eq!(
            w.sfx.iter().filter(|s| **s == Sfx::DashReady).count(),
            1,
            "the ready tick must tick once: {:?}",
            w.sfx
        );
        assert!(w.player.dash_ready_flash > 0.0, "no gauge pop on the ready tick");
        // The cue is wired into the synth cache (and a refused dash too).
        assert!(Sfx::ALL.contains(&Sfx::DashReady) && Sfx::ALL.contains(&Sfx::DashDenied));
        assert_eq!(Sfx::DashReady.variant_count(), 1);

        // Nothing re-announces the same cooldown.
        w.sfx.clear();
        for _ in 0..6 {
            tick(&mut w, false);
        }
        assert!(!w.sfx.contains(&Sfx::DashReady), "re-announced: {:?}", w.sfx);
    }

    /// A dash press the cooldown refuses is never silent, never dashes, and the
    /// feedback is throttled so a held key cannot machine-gun it.
    #[test]
    fn a_refused_dash_shakes_blips_and_does_not_dash() {
        let mut w = World::new(Theme::Sewer, 31);
        open_map(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 1.0;
        w.player.dash_active = 0.0;
        w.player.dash_denied = 0.0;
        w.sfx.clear();

        tick(&mut w, true);
        assert!(w.player.dash_denied > 0.0, "no shake for the refused press");
        assert_eq!(w.player.dash_active, 0.0, "a refused press dashed anyway");
        assert!(w.player.dash_cd > 1.0 - 0.02, "the cooldown was reset");
        assert_eq!(
            w.sfx.iter().filter(|s| **s == Sfx::DashDenied).count(),
            1,
            "no refused blip: {:?}",
            w.sfx
        );

        // Held key (auto-repeat): silent while the shake is still running.
        w.sfx.clear();
        tick(&mut w, true);
        assert!(!w.sfx.contains(&Sfx::DashDenied), "blip machine-gunned");
        // ...and it speaks again once the throttle window has passed.
        w.sfx.clear();
        for _ in 0..(DASH_DENIED_SECS * 60.0) as u32 + 2 {
            tick(&mut w, true);
        }
        assert!(
            w.sfx.contains(&Sfx::DashDenied),
            "the throttle never re-opened: {:?}",
            w.sfx
        );
    }

    /// The dash feedback (gauge pop, refused shake, their sounds) is pure visual
    /// state: setting it draws no rng and moves nothing the sim reads.
    #[test]
    fn dash_feedback_draws_no_rng_and_moves_nothing() {
        fn run(poke: bool) -> (u64, u64, u64, usize, u64) {
            let mut w = World::new(Theme::Sewer, 77);
            open_map(&mut w);
            w.player.pos = w.level.arena * 0.5;
            for tick in 0..240u32 {
                if poke {
                    w.player.dash_ready_flash = 0.35;
                    w.player.dash_denied = DASH_DENIED_SECS;
                }
                let dash = tick % 37 == 0;
                w.update(1.0 / 60.0, Vec2::new(1.0, 0.0), dash);
            }
            let mut enemies = 0u64;
            for e in w.enemies() {
                enemies = enemies
                    .wrapping_mul(31)
                    .wrapping_add(e.pos.x.to_bits() as u64)
                    .wrapping_add(e.pos.y.to_bits() as u64);
            }
            (
                w.player.pos.x.to_bits() as u64,
                w.player.pos.y.to_bits() as u64,
                w.player.dash_cd.to_bits() as u64,
                w.particles.len(),
                enemies,
            )
        }
        assert_eq!(
            run(false),
            run(true),
            "the dash feedback timers moved the simulation"
        );
    }

    /// Neon hook #1: a successful dash runs `vfx::emit_dash_start`, which
    /// pushes a shockwave (the legacy inline cone never did) plus streak/dust
    /// particles, and exactly one `Sfx::Dash`. A dash on cooldown emits
    /// neither.
    #[test]
    fn dash_start_emits_the_vfx_burst_and_one_dash_sfx() {
        let mut w = World::new(Theme::Sewer, 11);
        open_map(&mut w);
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
        open_map(&mut w);
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
        open_map(&mut w);
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
