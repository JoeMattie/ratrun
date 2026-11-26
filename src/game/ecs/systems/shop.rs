//! **Mouse-hole shops: the threshold touch, the re-arm and the exit grace.**
//!
//! A level carries two shop holes ([`crate::game::level::ShopHole`]) cut into
//! structural wall faces. They are *not* passable cells — the wall cell stays
//! solid, so neither the rat nor the horde can walk into one and nothing can
//! get stuck on one — so all this system does is notice the rat leaning on the
//! face:
//!
//! | edge | what happens |
//! |---|---|
//! | the rat's centre enters the threshold zone | `pending_shop = true` + the hole index + `Sfx::ShopEnter` |
//! | the rat stays there | nothing (the raise happens on the entry edge only) |
//! | the rat steps clear | the hole re-arms |
//! | the app closes the screen | [`World::leave_shop`] starts the 0.8 s grace |
//!
//! ## Determinism
//!
//! No rng, no entities, no particles: it reads the rat's position and writes
//! `RunCounters::pending_shop`/`pending_shop_hole` and the [`ShopRuntime`]
//! scratch. A run that never touches a shop therefore draws exactly the rng a
//! run without the feature draws.
//!
//! ## Slot
//!
//! Chained right after the interactive-object runtime (`gates::door_system`)
//! and before `enemy_move::flow_refresh_system`: it reads the rat's post-move
//! position, and nothing downstream of it is affected by the flag (the App acts
//! on it between ticks, and the world is frozen while the screen is open).

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::resources::{RunCounters, SfxQueue, TickInput};
use crate::game::level::{Level, ShopHole};
use crate::game::player::Player;
use crate::math::Vec2;

/// How long after leaving the shop the threshold is disarmed ("exit grants
/// 0.8 s grace").
pub const EXIT_GRACE_SECS: f32 = 0.8;

/// Half-width of the threshold zone along the wall face, in px. The hole is one
/// 16 px cell wide, so ±6 catches the middle two thirds of it and leaves the
/// corners to the wall.
pub const TOUCH_LATERAL: f32 = 6.0;

/// How far in front of the face the rat still counts as touching it, in px.
/// The rat's centre can get `PLAYER_RADIUS` (3 px) from the wall face, so 7 px
/// is "standing on the threshold and leaning in".
pub const TOUCH_REACH: f32 = 7.0;

/// How far *behind* the face the zone reaches. A rat can never stand inside the
/// wall, but the small tolerance keeps a dash that ends against the face inside
/// the zone instead of straddling it.
const TOUCH_BEHIND: f32 = 3.0;

/// Per-level scratch for the threshold edges.
///
/// Inserted permanently in [`World::new`](crate::game::world::World::new) like
/// `GateRuntime`: it never round-trips through the facade, sizes its buffers
/// once per level and allocates nothing per tick.
#[derive(Resource, Default)]
pub struct ShopRuntime {
    /// Level seed the buffers were sized for (`None` before the first tick).
    key: Option<u64>,
    /// Was the rat inside each hole's threshold zone on the previous tick?
    inside: Vec<bool>,
    /// Seconds of exit grace left.
    pub grace: f32,
    /// How many times a threshold fired (tests/diagnostics).
    pub triggers: u32,
}

impl ShopRuntime {
    fn sync(&mut self, level: &Level) {
        // The length is part of the key (as in `TrapRuntime::sync`): a level
        // whose hole list changed under the same seed must not index past
        // the old buffer.
        if self.key == Some(level.seed) && self.inside.len() == level.objects.shop_holes.len() {
            return;
        }
        self.key = Some(level.seed);
        self.inside.clear();
        self.inside.resize(level.objects.shop_holes.len(), false);
        self.grace = 0.0;
    }

    /// Is the threshold at `hole` still disarmed (the rat has not stepped
    /// clear since the last touch)?
    pub fn inside(&self, hole: usize) -> bool {
        self.inside.get(hole).copied().unwrap_or(false)
    }
}

/// Is `pos` inside `hole`'s threshold zone? The zone is a slab in front of the
/// wall face: ±[`TOUCH_LATERAL`] along the face, from [`TOUCH_BEHIND`] behind
/// it out to [`TOUCH_REACH`] in front.
pub fn threshold_contains(pos: Vec2, hole: &ShopHole) -> bool {
    let rel = pos - hole.face();
    let along = rel.x * hole.facing.x + rel.y * hole.facing.y;
    let lateral = (rel.x * hole.facing.y - rel.y * hole.facing.x).abs();
    (-TOUCH_BEHIND..=TOUCH_REACH).contains(&along) && lateral <= TOUCH_LATERAL
}

/// Raise `pending_shop` (with the hole's index) the first tick the rat leans on
/// a hole's threshold.
pub fn threshold_system(
    input: Res<TickInput>,
    level: Res<Level>,
    player: Res<Player>,
    mut counters: ResMut<RunCounters>,
    mut sfx: ResMut<SfxQueue>,
    mut rt: ResMut<ShopRuntime>,
) {
    rt.sync(&level);
    let dt = input.dt.max(0.0);
    if rt.grace > 0.0 {
        rt.grace = (rt.grace - dt).max(0.0);
    }
    for index in 0..level.objects.shop_holes.len() {
        let hole = level.objects.shop_holes[index];
        let inside = threshold_contains(player.pos, &hole);
        let was_inside = rt.inside[index];
        if inside
            && !was_inside
            && rt.grace <= 0.0
            && !counters.pending_shop
            && !counters.game_over
            && !counters.level_complete
        {
            counters.pending_shop = true;
            counters.pending_shop_hole = index;
            rt.triggers = rt.triggers.saturating_add(1);
            sfx.0.push(Sfx::ShopEnter);
        }
        rt.inside[index] = inside;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::world::World;

    /// A world with its rat standing in front of shop hole `index`.
    fn world_at_hole(index: usize) -> World {
        let mut w = World::new(Theme::Sewer, 7);
        let hole = w.level.objects.shop_holes[index];
        w.player.pos = hole.face() + hole.facing * 3.0;
        w
    }

    fn tick(w: &mut World) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(threshold_system).unwrap();
        w.end_tick();
    }

    #[test]
    fn every_level_carries_two_shop_holes() {
        for theme in Theme::all() {
            for seed in [0u64, 3, 11] {
                let w = World::new(theme, seed);
                assert_eq!(
                    w.level.objects.shop_holes.len(),
                    2,
                    "{theme:?} seed {seed} must carry two shop holes"
                );
                assert_eq!(w.level.objects.shop_holes[0].index, 0);
                assert_eq!(w.level.objects.shop_holes[1].index, 1);
            }
        }
    }

    #[test]
    fn a_touch_raises_the_shop_with_the_hole_index_and_step_clear_rearms() {
        let mut w = world_at_hole(1);
        assert!(!w.pending_shop);
        tick(&mut w);
        assert!(w.pending_shop, "leaning on the threshold opens the shop");
        assert_eq!(w.pending_shop_hole, 1);
        assert!(!w.sfx.is_empty(), "the opening has its own sting");

        // The app consumes the flag; standing still must not re-raise it.
        w.pending_shop = false;
        w.sfx.clear();
        tick(&mut w);
        assert!(!w.pending_shop, "a held touch does not retrigger");

        // Step clear, then come back: the hole is armed again.
        let hole = w.level.objects.shop_holes[1];
        w.player.pos = hole.face() + hole.facing * 40.0;
        tick(&mut w);
        w.player.pos = hole.face() + hole.facing * 3.0;
        tick(&mut w);
        assert!(w.pending_shop, "a fresh approach re-opens the shop");
        assert_eq!(w.pending_shop_hole, 1);
    }

    #[test]
    fn the_exit_grace_holds_the_threshold_shut_for_eight_tenths() {
        let mut w = world_at_hole(0);
        tick(&mut w);
        assert!(w.pending_shop);
        w.leave_shop();
        assert!(!w.pending_shop);
        // Step clear and back while the grace runs: still shut.
        let hole = w.level.objects.shop_holes[0];
        w.player.pos = hole.face() + hole.facing * 40.0;
        tick(&mut w);
        w.player.pos = hole.face() + hole.facing * 3.0;
        tick(&mut w);
        assert!(!w.pending_shop, "0.8 s of grace: no instant re-open");
        // Let the grace run out (0.8 s at 60 Hz is 48 ticks), then re-enter.
        for _ in 0..40 {
            tick(&mut w);
        }
        assert!(!w.pending_shop, "still inside the grace window");
        for _ in 0..12 {
            tick(&mut w);
        }
        // The rat is standing in the zone, so it has to step clear and back.
        w.player.pos = hole.face() + hole.facing * 40.0;
        tick(&mut w);
        w.player.pos = hole.face() + hole.facing * 3.0;
        tick(&mut w);
        assert!(w.pending_shop, "after the grace the threshold works again");
    }

    #[test]
    fn the_threshold_zone_is_the_face_not_the_whole_wall() {
        let w = World::new(Theme::Sewer, 7);
        let hole = w.level.objects.shop_holes[0];
        assert!(threshold_contains(hole.face() + hole.facing * 2.0, &hole));
        // Off the end of the hole's own cell: not this hole.
        let side = Vec2::new(-hole.facing.y, hole.facing.x);
        assert!(!threshold_contains(hole.face() + side * 14.0, &hole));
        // Far in front: walking past, not touching.
        assert!(!threshold_contains(hole.face() + hole.facing * 30.0, &hole));
    }

    /// A shop hole is a wall cell: the horde never walks into one and never
    /// gets stuck on one. Small bodies are pressed against the face and the
    /// tunnel stays shut to them.
    #[test]
    fn enemies_never_enter_a_shop_hole_or_stick_to_it() {
        use crate::game::collision::cell_of;
        use crate::game::enemy::EnemyKind;
        let mut w = World::new(Theme::Sewer, 8);
        let hole = w.level.objects.shop_holes[0];
        // Five small bodies piled against the threshold, and the rat well away
        // so nothing here is about the shop opening.
        for i in 0..5 {
            let a = i as f32 * 0.7;
            let at = hole.face()
                + Vec2::new(crate::detmath::cos(a), crate::detmath::sin(a)) * 6.0;
            w.spawn_enemy(EnemyKind::Skitterer, at, 1.0);
        }
        w.player.pos = hole.face() + hole.facing * 220.0;
        for _ in 0..180 {
            w.update(1.0 / 60.0, Vec2::ZERO, false);
        }
        let mut checked = 0;
        for entity in w.ecs.iter_entities() {
            if let Some(enemy) = entity.get::<crate::game::enemy::Enemy>() {
                if !enemy.alive() {
                    continue;
                }
                checked += 1;
                assert_ne!(
                    cell_of(enemy.pos),
                    hole.cell,
                    "an enemy is inside the shop hole"
                );
                assert!(
                    !w.level.tiles.passable(hole.cell.0, hole.cell.1, crate::game::collision::SizeClass::Small),
                    "the hole cell must stay solid to the small class"
                );
            }
        }
        assert!(checked > 0, "the bodies survived long enough to be checked");
    }

    #[test]
    fn a_finished_run_never_opens_the_shop() {
        let mut w = world_at_hole(0);
        w.game_over = true;
        tick(&mut w);
        assert!(!w.pending_shop, "a dead rat does not shop");
    }

    /// The system takes no rng at all, and a tick that touches a threshold
    /// leaves the run's stream exactly where a tick that does not touch one
    /// leaves it: a run that visits no shop draws the same numbers.
    #[test]
    fn the_threshold_draws_no_random_numbers() {
        let mut touching = world_at_hole(0);
        let mut clear = World::new(Theme::Sewer, 7);
        clear.player.pos = Vec2::new(2.0, 2.0);
        for _ in 0..8 {
            tick(&mut touching);
            tick(&mut clear);
        }
        assert!(touching.pending_shop);
        assert!(!clear.pending_shop);
        use rand::RngExt as _;
        assert_eq!(touching.rng.random::<u64>(), clear.rng.random::<u64>());
    }
}
