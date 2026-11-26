//! The Printworks' own enemy rules (`WAVE4_ROSTERS.md` "Printworks", batch 6b).
//!
//! | kind | build | rule here |
//! |---|---|---|
//! | Silverfish | R(Skitterer) | none |
//! | Erratum Slip | V(Bat) | weaves twice as wide for its first second |
//! | Type Slug | V(Cat) | keeps its last heading 0.6 s after losing sight |
//! | Staple Crawler | N | two warned feet astride the rat (below) |
//! | Paper Wasp | R(Spitter) | none |
//! | Paper Reel | R(Brute) | none |
//! | Ink Blot | V(Wick Moth) | the burst centre slides 12 px along its last velocity |
//!
//! ## Placement
//!
//! [`floor_printworks_system`] belongs **right after
//! `enemy_wave4::wave4_system`** in the tick chain (it takes `Commands`, so
//! `chain()` gives it its own `ApplyDeferred`). That slot is after steering,
//! separation and wall resolution, after `wave3_system` has turned a dead Ink
//! Blot into its `Wick`, and before `reap_enemies` removes the corpse — the one
//! tick on which the corpse and its wick both exist.
//!
//! ## Layered, not graduated
//!
//! No kind here changes its `EnemyKind::archetype()`. The Staple Crawler
//! still names the Bell Flea (`radius()`'s fallback `debug_assert`s that a
//! registration names a base, and its nav class, glue units, bot and library
//! rows key on it), so this file **layers** the crawler's machine on top: it
//! pins `w3.flea_cd` every tick so the borrowed hop can never start, and runs
//! its own phases in the otherwise-unused `w4` scratch (`phase`, `t`, `cd`,
//! `spent`, `warn_age`, `trail[0..2]` = the two feet, `trail[2]` = the planted
//! anchor). `W4` is zero for every non-wave-4 archetype and `is_wave4()` is
//! false for the crawler, so `wave4_system` never reads or writes it. Keeping
//! the state on `Enemy` is what lets the footprint be drawn from `&Enemy`
//! (`art_roster_printworks::draw_staple_tell`) with the same two points.
//!
//! ## Determinism
//!
//! Bodies are walked in `SpawnId` order. **No rng draw** decides anything:
//! the crawler's first cooldown is staggered from its `SpawnId`, the slide
//! direction is the corpse's own velocity. The only draws are inside
//! `vfx::emit_player_hurt` on a landed hit, the same particle-only draw every
//! other hit in the game makes. With none of this floor's four ruled kinds
//! alive and no sliding wick the system returns before its first sort, so the
//! six scripted cases (which never field them) cannot see it.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::components::{SpawnId, Wick};
#[cfg(test)]
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::ecs::systems::enemy_wave3::{FLEA_JUMP_PERIOD, WICK_FUSE_SECS};
use crate::game::ecs::systems::projectiles::los_clear;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

// ---- Erratum Slip ----------------------------------------------------------

/// How long after spawn the slip's weave is doubled.
pub const SLIP_WIDE_SECS: f32 = 1.0;
/// The Bat's weave weight in `Enemy::steer` (`chase + wander * 0.6`). Adding
/// one more of it is "twice as wide".
pub const SLIP_EXTRA_WEAVE: f32 = 0.6;

// ---- Type Slug -------------------------------------------------------------

/// How long the slug keeps skating its last heading once the rat is out of
/// sight.
pub const SLUG_HOLD_SECS: f32 = 0.6;

// ---- Staple Crawler --------------------------------------------------------

pub const STAPLE_PHASE_WALK: u8 = 0;
pub const STAPLE_PHASE_TELL: u8 = 1;
pub const STAPLE_PHASE_STRIKE: u8 = 2;
pub const STAPLE_PHASE_REST: u8 = 3;

/// The crawler marks its feet once the rat is this close.
pub const STAPLE_TRIGGER_RANGE: f32 = 26.0;
/// Radius of one foot's hole.
pub const STAPLE_FOOT_RADIUS: f32 = 4.0;
/// Distance between the two holes; they sit astride the locked rat.
pub const STAPLE_FOOT_SPREAD: f32 = 14.0;
/// The two holes are marked this long before the feet land.
pub const STAPLE_TELL_SECS: f32 = 0.8;
/// The feet stay live this long.
pub const STAPLE_STRIKE_SECS: f32 = 0.4;
/// Damage of a foot. Both share one latch: one hit per strike.
pub const STAPLE_FOOT_DAMAGE: f32 = 7.0;
/// Stuck in the floor, exposed.
pub const STAPLE_REST_SECS: f32 = 1.0;
/// From the end of the rest to the next mark.
pub const STAPLE_COOLDOWN: f32 = 4.0;
/// The first mark after spawn: a base wait plus a `SpawnId` stagger, so a
/// column of crawlers does not stamp in unison (and no rng is drawn).
pub const STAPLE_FIRST_CD: f32 = 1.5;
/// A grazing allowance on the foot test. The holes are 7 px either side of
/// the locked rat and the rat is 3 px wide, so without it a rat that stands
/// still would be clipped by both rims; with it the gap between the feet is
/// the safe lane the design asks for ("walk between the feet").
pub const STAPLE_GRAZE: f32 = 0.5;

// ---- Ink Blot --------------------------------------------------------------

/// How far the burst centre travels over the fuse.
pub const INK_SLIDE_DISTANCE: f32 = 12.0;

/// Per-body state for the two `V` rules that need a clock of their own.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PressBody {
    /// Seconds since this system first saw the body (Erratum Slip).
    pub age: f32,
    /// Type Slug: the heading it had while it last saw the rat.
    pub heading: Vec2,
    /// Type Slug: seconds of blind skating left.
    pub blind_t: f32,
}

/// Rides an Ink Blot's `Wick`: where the burst started and where it will be
/// when the fuse ends. The endpoint is fixed at death, so it can be marked
/// for the whole fuse.
#[derive(Component, Clone, Copy, Debug)]
pub struct InkSlide {
    pub from: Vec2,
    pub end: Vec2,
}

impl InkSlide {
    /// The burst centre with `fuse` seconds left.
    #[inline]
    pub fn at(&self, fuse: f32) -> Vec2 {
        let k = (1.0 - fuse / WICK_FUSE_SECS).clamp(0.0, 1.0);
        self.from + (self.end - self.from) * k
    }
}

/// The two feet for a crawler at `crawler` locking a rat at `rat`: 14 px apart
/// astride the rat, across the line between them.
#[inline]
pub fn staple_feet(crawler: Vec2, rat: Vec2) -> [Vec2; 2] {
    let to = (rat - crawler).normalized();
    let axis = if to.len_sq() > 0.001 { to.perp() } else { Vec2::new(1.0, 0.0) };
    let half = axis * (STAPLE_FOOT_SPREAD * 0.5);
    [rat + half, rat - half]
}

/// Does a rat at `rat` stand in either live hole? The one test the strike, the
/// tests and (once it is taught) the bot share.
#[inline]
pub fn staple_hits(feet: &[Vec2; 2], rat: Vec2, radius: f32) -> bool {
    let reach = STAPLE_FOOT_RADIUS + radius - STAPLE_GRAZE;
    feet.iter().any(|f| f.dist(rat) < reach)
}

/// Is this crawler's footprint dangerous right now or about to be? (Bot hook.)
#[inline]
pub fn staple_warned(e: &Enemy) -> bool {
    e.kind == EnemyKind::StapleCrawler
        && e.alive()
        && matches!(e.w4.phase, STAPLE_PHASE_TELL | STAPLE_PHASE_STRIKE)
}

/// Where an Ink Blot's burst will end: 12 px along `vel` (or `facing` when it
/// died standing), shortened in quarters until the path is clear of walls.
pub fn ink_slide_end(level: &Level, from: Vec2, vel: Vec2, facing: Vec2) -> Vec2 {
    let mut dir = vel.normalized();
    if dir.len_sq() < 0.001 || vel.len_sq() < 1.0 {
        dir = facing.normalized();
    }
    if dir.len_sq() < 0.001 {
        return from;
    }
    for k in [1.0f32, 0.75, 0.5, 0.25] {
        let cand = from + dir * (INK_SLIDE_DISTANCE * k);
        if los_clear(level, from, cand, 1.0) {
            return cand;
        }
    }
    from
}

#[inline]
fn ruled(kind: EnemyKind) -> bool {
    matches!(
        kind,
        EnemyKind::ErratumSlip | EnemyKind::TypeSlug | EnemyKind::StapleCrawler | EnemyKind::InkBlot
    )
}

/// Reused per-tick lists (grow-only, so a tick allocates nothing once warm).
#[derive(Default)]
pub struct PrintworksScratch {
    order: Vec<(u64, Entity)>,
    /// Ink Blots that died this tick: `(pos, end)`, in `SpawnId` order.
    corpses: Vec<(Vec2, Vec2, bool)>,
    /// Where every Ink Blot stood at the end of last tick's pass and where its
    /// burst would end, carried to the next tick: a blot killed **after**
    /// `wave3_system` (a wave-4 hit, the Nuke's re-reap) gets its `Wick` from
    /// the reap, at this exact position, with the corpse already gone.
    pending: Vec<(Vec2, Vec2, bool)>,
    wick_order: Vec<(u64, Entity)>,
}

/// The Printworks behaviour tick. See the module docs for placement and
/// determinism.
#[allow(clippy::too_many_arguments)]
pub fn floor_printworks_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut PressBody>)>,
    mut wicks: Query<(Entity, &SpawnId, &mut Wick, Option<&InkSlide>)>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    // The floor's hit softening (`[enemies] sector_damage_mult`): the staple
    // feet carry it.
    sector: crate::game::ecs::resources::SectorDamage,
    mut scratch: Local<PrintworksScratch>,
) {
    let dt = input.dt;
    let dmg_scale = sector.mult();
    let ppos = player.pos;
    let scratch = &mut *scratch;
    // The Wind-up Mouse: the live key (highest `SpawnId` wins), the same rule
    // the steering lure and waves 3 and 4 use, so the walk and the mark agree.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    // ---- Sliding wicks: move every marked burst centre along its line ------
    //
    // Before the corpse scan, so a wick marked this tick starts at `from`.
    for (_, _, mut wick, slide) in wicks.iter_mut() {
        if let Some(slide) = slide {
            // Each wick is written from its own component only, so the visit
            // order cannot matter.
            wick.pos = slide.at(wick.fuse);
        }
    }

    // ---- Late wicks: blots that died after last tick's pass ----------------
    //
    // Before the gate: the last blot on the map may be the one that died.
    if !scratch.pending.is_empty() {
        pair_wicks(&mut scratch.pending, &mut scratch.wick_order, &wicks, &mut commands);
        scratch.pending.clear();
    }

    // Cheap gate: none of this floor's ruled kinds on the map — no sort, no
    // rng, nothing.
    if !enemies.iter().any(|(_, _, e, _)| ruled(e.kind)) {
        return;
    }

    scratch.order.clear();
    scratch
        .order
        .extend(enemies.iter().filter(|(_, _, e, _)| ruled(e.kind)).map(|(en, id, _, _)| (id.0, en)));
    scratch.order.sort_unstable_by_key(|&(id, _)| id);
    scratch.corpses.clear();

    for i in 0..scratch.order.len() {
        let (spawn_id, entity) = scratch.order[i];
        let Ok((_, _, mut e, state)) = enemies.get_mut(entity) else {
            continue;
        };

        // A dead Ink Blot whose wick `wave3_system` lit this tick (`-1` =
        // spent). The corpse is reaped later this tick, so this is seen once.
        if e.kind == EnemyKind::InkBlot {
            let end = ink_slide_end(&level, e.pos, e.vel, e.facing);
            if !e.alive() && e.w3.wick_t < 0.0 {
                scratch.corpses.push((e.pos, end, false));
            } else if e.w3.wick_t >= 0.0 {
                // Still alive, or killed since `wave3_system` ran: if the reap
                // lights its wick later this tick, next tick's pass marks it.
                // Nothing after this slot moves a body, so `pos` is exact.
                scratch.pending.push((e.pos, end, false));
            }
            continue;
        }
        if !e.alive() {
            continue;
        }

        match e.kind {
            EnemyKind::ErratumSlip => {
                let Some(mut st) = state else {
                    commands.entity(entity).insert(PressBody { age: dt, ..PressBody::default() });
                    slip_weave(&mut e, dt);
                    continue;
                };
                if st.age < SLIP_WIDE_SECS {
                    slip_weave(&mut e, dt);
                    st.age += dt;
                }
            }
            EnemyKind::TypeSlug => {
                let Some(mut st) = state else {
                    commands.entity(entity).insert(PressBody::default());
                    continue;
                };
                slug_tick(&mut e, &mut st, &level, ppos, dt);
            }
            EnemyKind::StapleCrawler => {
                let mark = lure_target(&e, ppos, decoy, &level);
                if staple_tick(&mut e, spawn_id, mark, &level, &mut player, dt, dmg_scale) {
                    sfx.0.push(Sfx::Hurt);
                    vfx::emit_player_hurt(&mut particles, &mut rng.0, ppos);
                }
            }
            _ => {}
        }
    }

    // ---- Mark this tick's Ink Blot wicks -----------------------------------
    //
    // `wave3_system` spawned each at the corpse's exact position with a full
    // fuse; pair them in `SpawnId` order (both lists are sorted, so two blots
    // dying on one pixel still pair deterministically).
    if !scratch.corpses.is_empty() {
        pair_wicks(&mut scratch.corpses, &mut scratch.wick_order, &wicks, &mut commands);
    }
}

/// Give every unmarked wick that sits exactly on a recorded blot position its
/// [`InkSlide`]. Wicks are visited in `SpawnId` order and each record is used
/// once, so two blots dying on one pixel still pair deterministically.
fn pair_wicks(
    records: &mut [(Vec2, Vec2, bool)],
    wick_order: &mut Vec<(u64, Entity)>,
    wicks: &Query<(Entity, &SpawnId, &mut Wick, Option<&InkSlide>)>,
    commands: &mut Commands,
) {
    wick_order.clear();
    wick_order.extend(
        wicks
            .iter()
            .filter(|(_, _, _, slide)| slide.is_none())
            .map(|(entity, id, _, _)| (id.0, entity)),
    );
    wick_order.sort_unstable_by_key(|&(id, _)| id);
    for &(_, entity) in wick_order.iter() {
        let Ok((_, _, wick, _)) = wicks.get(entity) else {
            continue;
        };
        let pos = wick.pos;
        if let Some(c) = records.iter_mut().find(|c| !c.2 && c.0 == pos) {
            c.2 = true;
            commands.entity(entity).insert(InkSlide { from: c.0, end: c.1 });
        }
    }
}

/// The slip's extra weave: one more helping of the Bat's wander term, pushed
/// through the same velocity smoothing `Enemy::steer` uses. Velocity only —
/// next tick's steering integrates it and the wall pass resolves it, so a wide
/// weave cannot carry a slip through a wall.
#[inline]
fn slip_weave(e: &mut Enemy, dt: f32) {
    if e.stun > 0.0 {
        return;
    }
    let speed = e.speed() * e.speed_scale * e.slow_mult;
    e.vel += Vec2::from_angle(e.wander) * (SLIP_EXTRA_WEAVE * speed) * (6.0 * dt).min(1.0);
}

/// The slug's ruled line: while it sees the rat it remembers its heading; for
/// 0.6 s after losing sight it keeps skating that heading instead of turning
/// with the route. Velocity only (see [`slip_weave`]).
fn slug_tick(e: &mut Enemy, st: &mut PressBody, level: &Level, rat: Vec2, dt: f32) {
    if los_clear(level, e.pos, rat, 1.0) {
        if e.vel.len_sq() > 1.0 {
            st.heading = e.vel.normalized();
        }
        st.blind_t = SLUG_HOLD_SECS;
        return;
    }
    if st.blind_t <= 0.0 || st.heading.len_sq() < 0.001 || e.stun > 0.0 {
        return;
    }
    st.blind_t -= dt;
    let base = e.speed() * e.speed_scale * e.slow_mult;
    let pace = e.vel.len().max(base);
    e.vel = st.heading * pace;
    e.facing = st.heading;
}

/// What a body marks: the Wind-up Mouse's key while the key is luring this
/// body, the rat otherwise (the wave-3/wave-4 rule, verbatim: lurable body,
/// inside the key's radius, clear line to it).
#[inline]
fn lure_target(e: &Enemy, ppos: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    let Some(key) = decoy else {
        return ppos;
    };
    if !crate::game::ecs::systems::powerups::decoy_lures_body(e) {
        return ppos;
    }
    let decoy_r = crate::game::ecs::systems::powerups::WINDUP_MOUSE_RADIUS;
    if e.pos.dist_sq(key) > decoy_r * decoy_r {
        return ppos;
    }
    if !los_clear(level, e.pos, key, e.radius) {
        return ppos;
    }
    key
}

/// One Staple Crawler tick. Returns `true` when a foot hit the rat this tick.
///
/// `mark` is what the crawler walks at and marks astride: the live Wind-up
/// Mouse if there is one, else the rat. The feet only ever hurt the rat, so a
/// crawler lured onto the key stamps an empty patch of floor.
fn staple_tick(
    e: &mut Enemy,
    spawn_id: u64,
    mark: Vec2,
    level: &Level,
    player: &mut Player,
    dt: f32,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) -> bool {
    let rat = player.pos;
    // The borrowed Bell Flea hop never starts: its cooldown is held full.
    e.w3.flea_cd = FLEA_JUMP_PERIOD;
    if e.w4.armed == 0 {
        e.w4.armed = 1;
        e.w4.phase = STAPLE_PHASE_WALK;
        e.w4.cd = STAPLE_FIRST_CD + (spawn_id % 5) as f32 * 0.4;
    }
    let mut hit = false;
    match e.w4.phase {
        STAPLE_PHASE_TELL => {
            staple_hold(e);
            e.w4.warn_age += dt;
            e.w4.t -= dt;
            if e.w4.t <= 0.0 {
                e.w4.phase = STAPLE_PHASE_STRIKE;
                e.w4.t = STAPLE_STRIKE_SECS;
                e.w4.spent = 0;
            }
        }
        STAPLE_PHASE_STRIKE => {
            staple_hold(e);
            let feet = [e.w4.trail[0], e.w4.trail[1]];
            // One latch for both feet: one hit per strike.
            if e.w4.spent == 0
                && staple_hits(&feet, rat, player.buffs.hurt_radius())
                && player.take_damage(STAPLE_FOOT_DAMAGE * dmg_scale)
            {
                e.w4.spent = 1;
                hit = true;
            }
            e.w4.t -= dt;
            if e.w4.t <= 0.0 {
                e.w4.phase = STAPLE_PHASE_REST;
                e.w4.t = STAPLE_REST_SECS;
            }
        }
        STAPLE_PHASE_REST => {
            staple_hold(e);
            e.w4.t -= dt;
            if e.w4.t <= 0.0 {
                e.w4.phase = STAPLE_PHASE_WALK;
                e.w4.t = 0.0;
                e.w4.cd = STAPLE_COOLDOWN;
                e.w4.spent = 0;
                e.w4.warn_age = 0.0;
            }
        }
        _ => {
            e.w4.cd = (e.w4.cd - dt).max(0.0);
            // No mark through a wall: the crawler needs a clear line, a foot
            // wide, to what it stamps.
            if e.w4.cd <= 0.0
                && e.stun <= 0.0
                && e.pos.dist(mark) <= STAPLE_TRIGGER_RANGE
                && los_clear(level, e.pos, mark, STAPLE_FOOT_RADIUS)
            {
                let feet = staple_feet(e.pos, mark);
                e.w4.trail[0] = feet[0];
                e.w4.trail[1] = feet[1];
                e.w4.trail[2] = e.pos;
                e.w4.dir = (mark - e.pos).normalized();
                e.w4.phase = STAPLE_PHASE_TELL;
                e.w4.t = STAPLE_TELL_SECS;
                e.w4.warn_age = 0.0;
                e.w4.spent = 0;
            }
        }
    }
    hit
}

/// Planted: the crawler stays on the spot it marked from. Steering ran
/// earlier this tick, so its step is undone and its momentum cleared.
#[inline]
fn staple_hold(e: &mut Enemy) {
    e.pos = e.w4.trail[2];
    e.vel = Vec2::ZERO;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    fn open_world(seed: u64) -> (World, Vec2) {
        let mut w = World::new(Theme::Printworks, seed);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.clear_enemies();
        w.clear_bullets();
        let p = w.level.arena * 0.5;
        w.player.pos = p;
        (w, p)
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_printworks_system).unwrap();
        w.end_tick();
    }

    fn secs(s: f32) -> usize {
        (s / DT).ceil() as usize
    }

    fn state(w: &World, entity: Entity) -> Enemy {
        w.ecs.get::<Enemy>(entity).unwrap().clone()
    }

    fn put<F: FnOnce(&mut Enemy)>(w: &mut World, entity: Entity, f: F) {
        let mut e = w.ecs.get_mut::<Enemy>(entity).unwrap();
        f(&mut e);
    }

    // ---- geometry ---------------------------------------------------------

    #[test]
    fn the_feet_sit_fourteen_apart_astride_the_rat() {
        let rat = Vec2::new(100.0, 100.0);
        let feet = staple_feet(Vec2::new(80.0, 100.0), rat);
        assert!((feet[0].dist(feet[1]) - STAPLE_FOOT_SPREAD).abs() < 1e-3);
        let mid = (feet[0] + feet[1]) * 0.5;
        assert!(mid.dist(rat) < 1e-3);
        // Across the approach line, not along it.
        assert!((feet[0].x - rat.x).abs() < 1e-3);
    }

    #[test]
    fn the_lane_between_the_feet_is_safe_and_each_hole_is_not() {
        let rat = Vec2::new(100.0, 100.0);
        let feet = staple_feet(Vec2::new(80.0, 100.0), rat);
        assert!(!staple_hits(&feet, rat, PLAYER_RADIUS), "standing between the feet is the answer");
        assert!(!staple_hits(&feet, rat + Vec2::new(6.0, 0.0), PLAYER_RADIUS), "walking the lane stays safe");
        assert!(staple_hits(&feet, feet[0], PLAYER_RADIUS));
        assert!(staple_hits(&feet, feet[1], PLAYER_RADIUS));
        assert!(staple_hits(&feet, rat + Vec2::new(0.0, 2.0), PLAYER_RADIUS), "drifting sideways is clipped");
    }

    // ---- Staple Crawler ---------------------------------------------------

    #[test]
    fn the_crawler_tells_for_its_full_window_before_a_foot_can_hit() {
        let (mut w, p) = open_world(11);
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, p + Vec2::new(-20.0, 0.0), 1.0);
        put(&mut w, c, |e| {
            e.w4.armed = 1;
            e.w4.cd = 0.0;
        });
        let hp = w.player.hp;
        tick(&mut w);
        let e = state(&w, c);
        assert_eq!(e.w4.phase, STAPLE_PHASE_TELL);
        // Stand in a hole for the whole tell: nothing lands.
        w.player.pos = e.w4.trail[0];
        for _ in 0..secs(STAPLE_TELL_SECS) - 2 {
            tick(&mut w);
            assert_eq!(state(&w, c).w4.phase, STAPLE_PHASE_TELL);
            assert_eq!(w.player.hp, hp, "a tell never bites");
        }
        // Then the strike lands exactly once.
        for _ in 0..secs(STAPLE_STRIKE_SECS) + 4 {
            tick(&mut w);
            w.player.iframes = 0.0;
        }
        assert_eq!(w.player.hp, hp - STAPLE_FOOT_DAMAGE, "one hit per strike, both feet share the latch");
        assert_eq!(state(&w, c).w4.phase, STAPLE_PHASE_REST);
    }

    #[test]
    fn the_crawler_rests_then_waits_out_its_cooldown() {
        let (mut w, p) = open_world(12);
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, p + Vec2::new(-20.0, 0.0), 1.0);
        put(&mut w, c, |e| {
            e.w4.armed = 1;
            e.w4.cd = 0.0;
        });
        let anchor = state(&w, c).pos;
        for _ in 0..secs(STAPLE_TELL_SECS + STAPLE_STRIKE_SECS) + 6 {
            tick(&mut w);
        }
        assert_eq!(state(&w, c).w4.phase, STAPLE_PHASE_REST);
        assert_eq!(state(&w, c).pos, anchor, "planted feet do not walk");
        for _ in 0..secs(STAPLE_REST_SECS) + 1 {
            tick(&mut w);
        }
        let e = state(&w, c);
        assert_eq!(e.w4.phase, STAPLE_PHASE_WALK);
        assert!(e.w4.cd > STAPLE_COOLDOWN - 0.2);
        // It does not mark again until the cooldown is spent.
        for _ in 0..secs(STAPLE_COOLDOWN) - 30 {
            tick(&mut w);
            assert_eq!(state(&w, c).w4.phase, STAPLE_PHASE_WALK);
        }
    }

    #[test]
    fn a_far_crawler_never_marks_and_never_hops() {
        let (mut w, p) = open_world(13);
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, p + Vec2::new(-80.0, 0.0), 1.0);
        for _ in 0..secs(6.0) {
            tick(&mut w);
        }
        let e = state(&w, c);
        assert_eq!(e.w4.phase, STAPLE_PHASE_WALK);
        assert_eq!(e.w3.flea_phase, 0);
        assert_eq!(e.w3.flea_cd, FLEA_JUMP_PERIOD, "the borrowed hop stays pinned");
    }

    #[test]
    fn the_rat_between_the_feet_takes_nothing() {
        let (mut w, p) = open_world(14);
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, p + Vec2::new(-20.0, 0.0), 1.0);
        put(&mut w, c, |e| {
            e.w4.armed = 1;
            e.w4.cd = 0.0;
        });
        let hp = w.player.hp;
        for _ in 0..secs(STAPLE_TELL_SECS + STAPLE_STRIKE_SECS + STAPLE_REST_SECS) + 4 {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp);
    }

    #[test]
    fn the_wind_up_mouse_draws_the_feet_off_the_rat() {
        use crate::game::ecs::systems::powerups::Decoy;
        let (mut w, p) = open_world(15);
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, p + Vec2::new(-60.0, 0.0), 1.0);
        let key = p + Vec2::new(-80.0, 0.0);
        w.ecs.spawn((Decoy::new(key), SpawnId(u64::MAX - 1)));
        put(&mut w, c, |e| {
            e.w4.armed = 1;
            e.w4.cd = 0.0;
        });
        let hp = w.player.hp;
        tick(&mut w);
        let e = state(&w, c);
        assert_eq!(e.w4.phase, STAPLE_PHASE_TELL, "the key is in range though the rat is not");
        let mid = (e.w4.trail[0] + e.w4.trail[1]) * 0.5;
        assert!(mid.dist(key) < 1e-3, "the holes sit astride the key");
        for _ in 0..secs(STAPLE_TELL_SECS + STAPLE_STRIKE_SECS) + 6 {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp);
    }

    #[test]
    fn a_far_key_does_not_disarm_the_crawler() {
        use crate::game::ecs::systems::powerups::{Decoy, WINDUP_MOUSE_RADIUS};
        let (mut w, p) = open_world(16);
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, p + Vec2::new(-20.0, 0.0), 1.0);
        // A key ticking well outside its own lure radius.
        w.ecs.spawn((Decoy::new(p + Vec2::new(WINDUP_MOUSE_RADIUS * 4.0, 0.0)), SpawnId(u64::MAX - 1)));
        put(&mut w, c, |e| {
            e.w4.armed = 1;
            e.w4.cd = 0.0;
        });
        tick(&mut w);
        let e = state(&w, c);
        assert_eq!(e.w4.phase, STAPLE_PHASE_TELL, "the rat is in range and the key is not luring");
        let mid = (e.w4.trail[0] + e.w4.trail[1]) * 0.5;
        assert!(mid.dist(p) < 1e-3, "the holes sit astride the rat");
    }

    #[test]
    fn the_crawler_does_not_mark_through_a_wall() {
        let (mut w, p) = open_world(17);
        // Tile-aligned: the crawler one tile left of a solid column, the rat
        // just beyond it, 24 px apart (inside the 26 px trigger).
        let cell = crate::game::collision::cell_of(p);
        let wall_x = cell.0 as f32 * 16.0;
        let rat = Vec2::new(wall_x + 16.0 + 4.0, p.y);
        let crawler = Vec2::new(wall_x - 4.0, p.y);
        w.player.pos = rat;
        for dy in -3..=3 {
            w.level.tiles.set(cell.0, cell.1 + dy, TileKind::Wall(WallKind::Solid));
        }
        let c = w.spawn_enemy(EnemyKind::StapleCrawler, crawler, 1.0);
        put(&mut w, c, |e| {
            e.w4.armed = 1;
            e.w4.cd = 0.0;
        });
        assert!(crawler.dist(rat) <= STAPLE_TRIGGER_RANGE);
        for _ in 0..30 {
            tick(&mut w);
            assert_eq!(state(&w, c).w4.phase, STAPLE_PHASE_WALK, "no mark through a wall");
        }
        // Take the wall away and it marks at once.
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        tick(&mut w);
        assert_eq!(state(&w, c).w4.phase, STAPLE_PHASE_TELL);
    }

    // ---- Erratum Slip -----------------------------------------------------

    #[test]
    fn the_slip_weaves_wide_for_one_second_only() {
        let (mut w, p) = open_world(21);
        let s = w.spawn_enemy(EnemyKind::ErratumSlip, p + Vec2::new(-60.0, 0.0), 1.0);
        put(&mut w, s, |e| {
            e.vel = Vec2::ZERO;
            e.wander = 0.0;
        });
        tick(&mut w);
        assert!(state(&w, s).vel.len() > 0.0, "the extra weave pushes it on its first tick");
        for _ in 0..secs(SLIP_WIDE_SECS) + 2 {
            tick(&mut w);
        }
        put(&mut w, s, |e| e.vel = Vec2::ZERO);
        tick(&mut w);
        assert_eq!(state(&w, s).vel, Vec2::ZERO, "after a second it is an ordinary Bat");
        assert!(w.ecs.get::<PressBody>(s).unwrap().age >= SLIP_WIDE_SECS);
    }

    // ---- Type Slug --------------------------------------------------------

    #[test]
    fn the_slug_keeps_its_heading_for_point_six_after_losing_sight() {
        let (mut w, p) = open_world(31);
        let s = w.spawn_enemy(EnemyKind::TypeSlug, p + Vec2::new(-60.0, 0.0), 1.0);
        let heading = Vec2::new(1.0, 0.0);
        put(&mut w, s, |e| e.vel = heading * 30.0);
        tick(&mut w); // inserts the state
        tick(&mut w); // sees the rat, records the heading
        assert_eq!(w.ecs.get::<PressBody>(s).unwrap().heading, heading);
        // Break sight: a solid wall of tiles between them is simplest to fake
        // by moving the rat behind the map's own solid border.
        let mut blocked = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        let slug = state(&w, s).pos;
        let cell = crate::game::collision::cell_of(slug + Vec2::new(32.0, 0.0));
        // A full column of wall, so no sight line slips past a single tile.
        for dy in -3..=3 {
            blocked.set(cell.0, cell.1 + dy, TileKind::Wall(WallKind::Solid));
        }
        w.level.tiles = blocked;
        // The route now wants it to turn; it does not.
        put(&mut w, s, |e| e.vel = Vec2::new(0.0, 30.0));
        tick(&mut w);
        let e = state(&w, s);
        assert!(e.vel.normalized().dist(heading) < 1e-3, "it skates its ruled line");
        for _ in 0..secs(SLUG_HOLD_SECS) + 2 {
            tick(&mut w);
        }
        put(&mut w, s, |e| e.vel = Vec2::new(0.0, 30.0));
        tick(&mut w);
        assert!(state(&w, s).vel.normalized().dist(Vec2::new(0.0, 1.0)) < 1e-3, "then it is a Cat again");
    }

    // ---- Ink Blot ---------------------------------------------------------

    #[test]
    fn the_slide_ends_twelve_along_the_last_velocity() {
        let (w, p) = open_world(41);
        let end = ink_slide_end(&w.level, p, Vec2::new(20.0, 0.0), Vec2::new(0.0, 1.0));
        assert!(end.dist(p + Vec2::new(INK_SLIDE_DISTANCE, 0.0)) < 1e-3);
        // Standing still: along its facing.
        let end = ink_slide_end(&w.level, p, Vec2::ZERO, Vec2::new(0.0, 1.0));
        assert!(end.dist(p + Vec2::new(0.0, INK_SLIDE_DISTANCE)) < 1e-3);
    }

    #[test]
    fn the_slide_reaches_its_endpoint_as_the_fuse_ends() {
        let slide = InkSlide { from: Vec2::new(10.0, 10.0), end: Vec2::new(22.0, 10.0) };
        assert_eq!(slide.at(WICK_FUSE_SECS), slide.from);
        assert!(slide.at(WICK_FUSE_SECS * 0.5).dist(Vec2::new(16.0, 10.0)) < 1e-3);
        assert_eq!(slide.at(0.0), slide.end);
        assert_eq!(slide.at(-0.1), slide.end);
    }

    #[test]
    fn a_dead_blots_wick_is_marked_and_slides() {
        use crate::game::ecs::systems::enemy_wave3::wave3_system;
        let (mut w, p) = open_world(42);
        let b = w.spawn_enemy(EnemyKind::InkBlot, p + Vec2::new(-70.0, 0.0), 1.0);
        let from = state(&w, b).pos;
        // Let wave 3 arm the fuse, then kill it moving right.
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(wave3_system).unwrap();
        w.end_tick();
        put(&mut w, b, |e| {
            e.hurt(1.0e6);
            e.vel = Vec2::new(25.0, 0.0);
        });
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(wave3_system).unwrap();
        w.ecs.run_system_cached(floor_printworks_system).unwrap();
        w.end_tick();
        let mut q = w.ecs.query::<(&Wick, &InkSlide)>();
        let (wick, slide) = q.iter(&w.ecs).next().expect("the blot's wick carries a slide");
        assert_eq!(wick.pos, from);
        assert!(slide.end.dist(from + Vec2::new(INK_SLIDE_DISTANCE, 0.0)) < 1e-3);
        // Half the fuse later the centre is half way there.
        for _ in 0..secs(WICK_FUSE_SECS * 0.5) {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(wave3_system).unwrap();
            w.ecs.run_system_cached(floor_printworks_system).unwrap();
            w.end_tick();
        }
        let mut q = w.ecs.query::<(&Wick, &InkSlide)>();
        let (wick, _) = q.iter(&w.ecs).next().expect("still burning");
        assert!((wick.pos.x - (from.x + INK_SLIDE_DISTANCE * 0.5)).abs() < 1.0);
    }

    #[test]
    fn a_blot_killed_after_wave_three_still_slides() {
        use crate::game::ecs::systems::enemy_wave3::wave3_system;
        let (mut w, p) = open_world(44);
        let b = w.spawn_enemy(EnemyKind::InkBlot, p + Vec2::new(-70.0, 0.0), 1.0);
        let from = state(&w, b).pos;
        // Wave 3 has already run this tick when the blot dies (a wave-4 hit,
        // the Nuke): the reap, not wave 3, lights the wick.
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(wave3_system).unwrap();
        {
            let mut e = w.ecs.get_mut::<Enemy>(b).unwrap();
            e.vel = Vec2::new(25.0, 0.0);
            e.hurt(1.0e6);
        }
        w.ecs.run_system_cached(floor_printworks_system).unwrap();
        // What `loot`'s reap does with a corpse whose fuse is still unspent.
        w.ecs.despawn(b);
        w.ecs.spawn((Wick::new(from), SpawnId(u64::MAX - 2)));
        w.end_tick();
        tick(&mut w);
        let mut q = w.ecs.query::<(&Wick, &InkSlide)>();
        let (_, slide) = q.iter(&w.ecs).next().expect("the late wick is marked on the next tick");
        assert_eq!(slide.from, from);
        assert!(slide.end.dist(from + Vec2::new(INK_SLIDE_DISTANCE, 0.0)) < 1e-3);
    }

    #[test]
    fn a_plain_wick_moth_does_not_slide() {
        use crate::game::ecs::systems::enemy_wave3::wave3_system;
        let (mut w, p) = open_world(43);
        let m = w.spawn_enemy(EnemyKind::WickMoth, p + Vec2::new(-70.0, 0.0), 1.0);
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(wave3_system).unwrap();
        w.end_tick();
        put(&mut w, m, |e| {
            e.hurt(1.0e6);
            e.vel = Vec2::new(25.0, 0.0);
        });
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(wave3_system).unwrap();
        w.ecs.run_system_cached(floor_printworks_system).unwrap();
        w.end_tick();
        let mut q = w.ecs.query::<&InkSlide>();
        assert!(q.iter(&w.ecs).next().is_none());
    }

    #[test]
    fn an_empty_floor_costs_no_rng() {
        use rand::RngExt;
        let (mut a, _) = open_world(51);
        let (mut b, _) = open_world(51);
        for _ in 0..30 {
            tick(&mut a);
        }
        assert_eq!(a.rng.random::<u64>(), b.rng.random::<u64>());
    }
}
