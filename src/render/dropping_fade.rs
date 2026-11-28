//! Hantavirus droppings that leave the floor early keep their glow for a
//! moment and fade it out, instead of cutting it off.
//!
//! A dropping normally dries out over the last [`DROPPING_DRY_SECS`] of its
//! [`DROPPING_LIFE`], so its green ground pool, its floor bloom and its GI
//! emitter are already down to nothing when the entity despawns. Two other
//! paths remove one **while it is still bright**: the weapon's `DROPPING_CAP`
//! eviction (the oldest `SpawnId` goes first) and an unmarked body stepping in
//! it. Before this module both of those took the light away in one frame — a
//! visible pop, and the owner's bug ("when hantavirus poops go away, the light
//! should fade out smoothly").
//!
//! The fix stays entirely on the render side. The simulation still despawns the
//! entity on exactly the same tick and draws no rng; this module keeps a small
//! fixed-size list of **fading ghosts** — the removed dropping, the `elapsed`
//! it was removed at — and the lighting (`sources.rs`), the GI pass (`gi.rs`)
//! and the floor-bloom art (`art_droppings.rs`) keep drawing that drop with a
//! gain that falls on a smoothstep to exactly zero over [`DROPPING_FADE_SECS`].
//!
//! The list is filled by **diffing** this frame's live droppings against the
//! ones the last frame saw, so every removal path is covered without the
//! simulation having to write a record (it may not) and without any of them
//! being special-cased. A deliberately cleared floor — a new level, which is a
//! new `World` and so a new [`crate::render::lighting::LightMap`] — can never
//! reach it.
//!
//! Rules it keeps:
//!
//! * **No step.** A ghost starts at exactly the energy its live light had
//!   (`fade(now) == 1.0` on the removal frame) and smoothsteps to exactly `0.0`
//!   at `start + DROPPING_FADE_SECS`: zero slope at both ends, so neither the
//!   removal nor the end of the fade can pop.
//! * **Bounded, allocation-free.** [`MAX_DROPPING_GHOSTS`] ghosts, a fixed
//!   array of the last [`MAX_TRACKED`] live droppings and a fixed array of
//!   ghosts: `update` allocates nothing, and once the list is full the ghost
//!   that has been fading longest makes room.
//! * **Stable while frozen.** The fade is computed from `World::elapsed`, so a
//!   frame that does not tick (a pause, a plate over the field) draws exactly
//!   the same pixels. `PictureKey` needs nothing: it is keyed on `World::ticks`.
//!
//! Render-only: nothing here reads or writes simulation state, nothing here is
//! round-tripped, and a frame in which this list changes simulates identically.

use bevy_ecs::world::World as EcsWorld;

use crate::game::ecs::components::{Dropping, SpawnId};
use crate::game::weapon::{
    DROPPING_ARM, DROPPING_LIFE, DROPPING_LIGHT_ENERGY, DROPPING_LIGHT_RADIUS,
};

/// Seconds a dropping that leaves the floor early takes to fade its light, its
/// floor bloom and its GI contribution to exactly zero.
pub const DROPPING_FADE_SECS: f32 = 0.6;

/// The most fading ghosts held at once. The weapon keeps at most
/// `DROPPING_CAP[4]` = 8 droppings live, and a whole capped set can be spent on
/// one tick (a crowd walking over them), so 16 leaves room for two clear-outs
/// without ever growing.
pub const MAX_DROPPING_GHOSTS: usize = 16;

/// Seconds at the end of a dropping's life over which it dries out (the last
/// two seconds of [`DROPPING_LIFE`]). The live light, the GI emitter and the
/// art share this one figure.
pub const DROPPING_DRY_SECS: f32 = 2.0;

/// Droppings remembered for the removal diff: `DROPPING_CAP[4]`, the most the
/// weapon keeps live (`weapons_wave3a::fire_hantavirus`'s cap). A stack array,
/// so a frame allocates nothing.
const MAX_TRACKED: usize = 8;

/// A dropping's natural dry-out: `1.0` while it is fresh, falling over its
/// final [`DROPPING_DRY_SECS`] and **exactly** `0.0` at (and past)
/// [`DROPPING_LIFE`], so the natural expiry has no last-frame step.
#[inline]
pub fn natural_fade(age: f32) -> f32 {
    ((DROPPING_LIFE - age) / DROPPING_DRY_SECS).clamp(0.0, 1.0)
}

/// A dropping's arming factor: half strength until it is armed
/// ([`DROPPING_ARM`]), full after — the same step the weapon's contract has.
#[inline]
pub fn arm_fade(age: f32) -> f32 {
    if age < DROPPING_ARM {
        0.5
    } else {
        1.0
    }
}

/// Smoothstep on the share of a fade still to run: exactly `1.0` at `since ==
/// 0.0`, exactly `0.0` from [`DROPPING_FADE_SECS`] on, with a zero slope at
/// both ends (so the removal frame cannot step down and the last frame cannot
/// pop out). A negative or NaN `since` reads as the start, so a nonsense clock
/// cannot put NaN into a light.
#[inline]
pub fn smooth_fade(since: f32) -> f32 {
    let t = if since > 0.0 {
        (since / DROPPING_FADE_SECS).min(1.0)
    } else {
        0.0
    };
    let u = 1.0 - t;
    u * u * (3.0 - 2.0 * u)
}

/// `dropping.level` (1..=5) as a table index, clamped like every other reader.
#[inline]
fn rank_of(level: u8) -> usize {
    usize::from(level.clamp(1, 5) - 1)
}

/// One removed dropping, still lighting the floor for [`DROPPING_FADE_SECS`].
#[derive(Clone, Copy, Debug)]
pub struct DroppingGhost {
    /// The dropping as it was last drawn: position, rank, art seed and the age
    /// it had, which is what its dry-out and arming gains are taken from.
    drop: Dropping,
    /// `World::elapsed` when the removal was seen.
    start: f32,
}

impl DroppingGhost {
    /// The dropping this ghost is the fading remains of.
    #[inline]
    pub fn drop(&self) -> &Dropping {
        &self.drop
    }

    /// `World::elapsed` at which the ghost was made.
    #[inline]
    pub fn start(&self) -> f32 {
        self.start
    }

    /// The time fade at `now`: `1.0` on the removal frame, exactly `0.0` once
    /// [`DROPPING_FADE_SECS`] have passed.
    #[inline]
    pub fn fade(&self, now: f32) -> f32 {
        smooth_fade(now - self.start)
    }

    /// Everything except the time fade: the arming step times the dry-out. This
    /// is the gain the live light had when the drop went, so a ghost starts at
    /// exactly the brightness the floor saw on the previous frame.
    #[inline]
    pub fn peak_strength(&self) -> f32 {
        arm_fade(self.drop.age) * natural_fade(self.drop.age)
    }

    /// Arming and dry-out times the time fade, without the rank's energy: the
    /// scatter share, the GI emitter and the sprite's bloom use this.
    #[inline]
    pub fn strength(&self, now: f32) -> f32 {
        self.peak_strength() * self.fade(now)
    }

    /// The rank's ground-light energy still left at `now`, in
    /// [`DROPPING_LIGHT_ENERGY`] units (the light and GI gains are applied by
    /// their own passes).
    #[inline]
    pub fn energy(&self, now: f32) -> f32 {
        DROPPING_LIGHT_ENERGY[rank_of(self.drop.level)] * self.strength(now)
    }

    /// The rank's ground-light radius in world px ([`DROPPING_LIGHT_RADIUS`]).
    #[inline]
    pub fn radius(&self) -> f32 {
        DROPPING_LIGHT_RADIUS[rank_of(self.drop.level)]
    }
}

/// The render-only list of droppings that left the floor early.
///
/// Held by [`crate::render::lighting::LightMap`] for the life of a `World`, so
/// a new floor (a new `World`) starts with an empty list and a cleared floor
/// never fades.
pub struct DroppingFade {
    ghosts: [Option<DroppingGhost>; MAX_DROPPING_GHOSTS],
    /// The live droppings the previous `update` saw, `SpawnId`-ordered: the
    /// left-hand side of the removal diff. Fixed size, never allocated.
    live: [Option<(u64, Dropping)>; MAX_TRACKED],
    /// More live droppings than the window holds (`DROPPING_CAP[4]` is the
    /// design's most, so only a hand-built test field can do it): the window
    /// cannot be trusted for a diff that frame, so it is reseeded without
    /// making a ghost. That can only ever *miss* a fade, never invent one.
    overflowed: bool,
}

impl Default for DroppingFade {
    fn default() -> Self {
        Self::new()
    }
}

impl DroppingFade {
    pub const fn new() -> Self {
        Self {
            ghosts: [None; MAX_DROPPING_GHOSTS],
            live: [None; MAX_TRACKED],
            overflowed: false,
        }
    }

    /// Note this frame's live droppings, turn the ones that are gone into
    /// fading ghosts, and drop the ghosts whose fade has finished.
    ///
    /// Called once per drawn frame (before the floor art is painted, so the
    /// sprite and the light of a ghost appear on the very frame its dropping
    /// goes). Calling it twice at the same `now` is idempotent: the second call
    /// sees the same live list and makes no ghost.
    pub fn update(&mut self, ecs: &EcsWorld, now: f32) {
        // 1. A ghost whose fade has run out is done: exactly zero energy, so it
        //    leaves no light and paints nothing.
        for slot in self.ghosts.iter_mut() {
            if slot.is_some_and(|ghost| ghost.fade(now) <= 0.0) {
                *slot = None;
            }
        }

        // 2. This frame's live droppings, lowest `SpawnId` first (entity
        //    iteration order decides nothing), kept in a fixed window.
        let mut fresh: [Option<(u64, Dropping)>; MAX_TRACKED] = [None; MAX_TRACKED];
        let mut total = 0usize;
        for entity in ecs.iter_entities() {
            let Some(drop) = entity.get::<Dropping>() else {
                continue;
            };
            let id = entity.get::<SpawnId>().map_or(0, |id| id.0);
            total += 1;
            let mut at = 0usize;
            while at < MAX_TRACKED && fresh[at].is_some_and(|(prior, _)| prior < id) {
                at += 1;
            }
            if at < MAX_TRACKED {
                fresh[at..].rotate_right(1);
                fresh[at] = Some((id, *drop));
            }
        }

        // 3. The diff: a dropping the last frame drew and this frame does not is
        //    a removal — the cap's eviction, the body that stepped in it and the
        //    natural expiry alike. The ghost starts at the brightness the floor
        //    saw last frame, so the light cannot step.
        let previous = self.live;
        if !self.overflowed {
            for &(id, drop) in previous.iter().flatten() {
                let gone = !fresh
                    .iter()
                    .flatten()
                    .any(|&(live_id, _)| live_id == id);
                if gone {
                    self.push_ghost(DroppingGhost { drop, start: now });
                }
            }
        }
        self.live = fresh;
        self.overflowed = total > MAX_TRACKED;
    }

    /// Take a slot for `ghost`: the first free one, else the ghost that has
    /// been fading longest (so a crowded list loses its faintest member).
    fn push_ghost(&mut self, ghost: DroppingGhost) {
        let slot = self
            .ghosts
            .iter()
            .position(Option::is_none)
            .or_else(|| {
                let mut oldest = 0usize;
                for (i, this) in self.ghosts.iter().enumerate() {
                    if let (Some(held), Some(this)) = (self.ghosts[oldest], *this) {
                        if this.start < held.start {
                            oldest = i;
                        }
                    }
                }
                Some(oldest)
            });
        if let Some(slot) = slot {
            self.ghosts[slot] = Some(ghost);
        }
    }

    /// Every ghost still fading, for the lighting and GI passes.
    pub fn ghosts(&self) -> impl Iterator<Item = &DroppingGhost> {
        self.ghosts.iter().flatten()
    }

    /// How many ghosts are held (bounded by [`MAX_DROPPING_GHOSTS`]).
    pub fn ghost_count(&self) -> usize {
        self.ghosts.iter().flatten().count()
    }

    /// The total ground-light energy the ghosts still throw at `now`, in
    /// [`DROPPING_LIGHT_ENERGY`] units. The curve the tests pin: monotone down
    /// to exactly zero over [`DROPPING_FADE_SECS`].
    pub fn total_energy(&self, now: f32) -> f32 {
        self.ghosts().map(|ghost| ghost.energy(now)).sum()
    }

    /// A stack copy of the ghosts' sprites and their fade, for the floor-art
    /// pass — which marks emissive pixels while it paints and so cannot hold a
    /// borrow of the list.
    pub fn snapshot(&self, now: f32) -> [Option<(Dropping, f32)>; MAX_DROPPING_GHOSTS] {
        let mut out = [None; MAX_DROPPING_GHOSTS];
        for (slot, ghost) in out.iter_mut().zip(self.ghosts()) {
            let fade = ghost.fade(now);
            if fade > 0.0 {
                *slot = Some((ghost.drop, fade));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::ecs::components::{Dropping, Entity};
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::ecs::systems::weapons_wave3a::droppings_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::weapon::{Weapon, WeaponKind, DROPPING_CAP, DROPPING_LIFE};
    use crate::game::world::World;
    use crate::math::Vec2;

    const DT: f32 = 1.0 / 60.0;

    /// A fresh, wall-free arena with Hantavirus armed and ready, facing +x.
    fn arena(seed: u64, level: u8) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.level.tiles.rebuild_masks();
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.clear_droppings();
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons = vec![Weapon {
            kind: WeaponKind::Hantavirus,
            level,
            timer: 0.0,
            evolved: false,
        }];
        w
    }

    /// One fire cycle on a zero-length tick: the timer only moves on a real
    /// shot, so a dud leaves the weapon ready.
    fn run_fire(w: &mut World) -> bool {
        let before = w.player.loadout.weapons[0].timer;
        w.begin_tick(0.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        w.player.loadout.weapons[0].timer > before
    }

    /// Age, expire and infect: the dropping pass only.
    fn run_droppings(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(droppings_system).unwrap();
        w.end_tick();
    }

    /// Every live dropping's energy, in the units the ground light uses.
    fn live_energy(w: &mut World) -> f32 {
        let mut q = w.ecs.query::<&Dropping>();
        q.iter(&w.ecs)
            .map(|d| DROPPING_LIGHT_ENERGY[rank_of(d.level)] * arm_fade(d.age) * natural_fade(d.age))
            .sum()
    }

    fn live_count(w: &mut World) -> usize {
        let mut q = w.ecs.query::<&Dropping>();
        q.iter(&w.ecs).count()
    }

    /// Step the clock in one-frame steps over a whole fade, asserting the curve
    /// every frame: never up, never a step, and exactly zero at the end.
    fn assert_fades_to_zero(fade: &DroppingFade, start: f32, peak: f32, what: &str) {
        let mut prev = fade.total_energy(start);
        assert!(
            (prev - peak).abs() < 1e-5,
            "{what}: the ghost starts at the energy the live light had ({prev} vs {peak})"
        );
        assert!(prev > 0.0, "{what}: there is something to fade");
        let frames = (DROPPING_FADE_SECS / DT).round() as u32;
        for i in 1..=frames {
            let now = start + i as f32 * DT;
            let energy = fade.total_energy(now);
            assert!(
                energy <= prev + 1e-6,
                "{what}: frame {i} went back up ({prev} -> {energy})"
            );
            assert!(
                prev - energy <= 0.08 * peak,
                "{what}: frame {i} dropped {:.1}% of the light in one frame",
                100.0 * (prev - energy) / peak
            );
            prev = energy;
        }
        let end = start + DROPPING_FADE_SECS;
        assert_eq!(
            fade.total_energy(end),
            0.0,
            "{what}: the fade must land on exactly zero"
        );
    }

    #[test]
    fn the_fade_is_a_smoothstep_that_lands_on_exactly_zero() {
        assert_eq!(smooth_fade(0.0), 1.0);
        assert_eq!(smooth_fade(DROPPING_FADE_SECS), 0.0);
        assert_eq!(smooth_fade(DROPPING_FADE_SECS * 4.0), 0.0);
        assert_eq!(smooth_fade(-3.0), 1.0, "a nonsense clock reads as the start");
        // No step at either end: the first frame barely moves and the last
        // frame has already arrived.
        assert!(smooth_fade(DT) > 0.99);
        assert!(smooth_fade(DROPPING_FADE_SECS - DT) < 0.01);
        let mut prev = 1.0;
        for i in 0..=72u32 {
            let f = smooth_fade(i as f32 / 72.0 * DROPPING_FADE_SECS);
            assert!(f <= prev + 1e-6, "smoothstep must never rise");
            assert!((0.0..=1.0).contains(&f), "smoothstep stays in 0..=1");
            prev = f;
        }
    }

    #[test]
    fn the_natural_expiry_ends_on_exactly_zero() {
        assert_eq!(natural_fade(DROPPING_LIFE), 0.0);
        assert_eq!(natural_fade(DROPPING_LIFE + 30.0), 0.0);
        assert_eq!(natural_fade(0.0), 1.0);
        assert_eq!(natural_fade(DROPPING_LIFE - DROPPING_DRY_SECS), 1.0);
        // The last frame a drop is still live is already dark.
        assert!(natural_fade(DROPPING_LIFE - DT) < 0.01);
    }

    /// The cap evicts the oldest dropping while it is still bright. Its light
    /// must fall smoothly to zero instead of cutting out.
    #[test]
    fn an_evicted_dropping_fades_its_light_to_zero() {
        let mut w = arena(60, 1);
        let cap = DROPPING_CAP[0] as usize;
        let mut fade = DroppingFade::new();
        let now = 0.0;
        for _ in 0..cap {
            w.player.loadout.weapons[0].timer = 0.0;
            assert!(run_fire(&mut w), "a dropping needs no target");
            fade.update(&w.ecs, now);
            assert_eq!(fade.ghost_count(), 0, "live droppings are not ghosts");
        }
        // The light the evicted drop was throwing: ungated (age 0 is still
        // arming, so half strength) and freshly dropped, so never drying.
        let evicted = DROPPING_LIGHT_ENERGY[0] * arm_fade(0.0) * natural_fade(0.0);

        // One more drop than the cap allows evicts the oldest one.
        w.player.loadout.weapons[0].timer = 0.0;
        assert!(run_fire(&mut w));
        assert_eq!(live_count(&mut w), cap, "the cap still holds {cap}");
        fade.update(&w.ecs, now);
        assert_eq!(fade.ghost_count(), 1, "the evicted drop became one ghost");
        assert!(
            (fade.total_energy(now) - evicted).abs() < 1e-5,
            "the ghost starts at exactly the evicted drop's light"
        );
        assert_fades_to_zero(&fade, now, evicted, "evicted");

        // …and it is gone from the list once the fade is done.
        fade.update(&w.ecs, DROPPING_FADE_SECS + DT);
        assert_eq!(fade.ghost_count(), 0, "a finished ghost is dropped");
    }

    /// A body stepping in an armed dropping spends it. Same fade, same curve.
    #[test]
    fn a_consumed_dropping_fades_its_light_to_zero() {
        let mut w = arena(61, 3);
        assert!(run_fire(&mut w));
        // Put the drop under a body and arm it: the next dropping pass infects
        // the body and despawns the drop.
        let at = w.player.pos + Vec2::new(-5.0, 0.0);
        let entity = {
            let mut q = w.ecs.query::<(Entity, &Dropping)>();
            q.iter(&w.ecs).map(|(e, _)| e).next().expect("the drop")
        };
        {
            let mut d = w.ecs.get_mut::<Dropping>(entity).expect("dropping");
            d.pos = at;
            d.age = DROPPING_ARM;
        }
        w.spawn_enemy(EnemyKind::Brute, at, 1.0);

        let mut fade = DroppingFade::new();
        let now = 0.0;
        fade.update(&w.ecs, now);
        assert_eq!(fade.ghost_count(), 0);
        let peak = live_energy(&mut w);
        assert!(peak > 0.0);

        run_droppings(&mut w, DT);
        assert_eq!(live_count(&mut w), 0, "the body consumed the drop");
        fade.update(&w.ecs, now);
        assert_eq!(fade.ghost_count(), 1, "the consumed drop became one ghost");
        assert_fades_to_zero(&fade, now, peak, "consumed");
    }

    /// Natural expiry keeps its two-second dry-out and ends at zero: the ghost
    /// it briefly leaves behind starts (and stays) invisible.
    #[test]
    fn a_naturally_expired_dropping_leaves_an_invisible_ghost() {
        let mut w = arena(62, 2);
        assert!(run_fire(&mut w));
        let mut fade = DroppingFade::new();
        let now = 0.0;
        fade.update(&w.ecs, now);

        // Age it to a hair short of its life (the dry-out is nearly done).
        run_droppings(&mut w, DROPPING_LIFE - 0.05);
        assert_eq!(live_count(&mut w), 1, "still inside its life");
        fade.update(&w.ecs, now);
        assert_eq!(fade.ghost_count(), 0);
        let last_live = live_energy(&mut w);
        assert!(
            last_live > 0.0 && last_live < 0.05 * DROPPING_LIGHT_ENERGY[1],
            "the dry-out must have almost finished before the drop goes ({last_live})"
        );

        // Past its life the entity despawns, as it always did.
        run_droppings(&mut w, 0.06);
        assert_eq!(live_count(&mut w), 0, "past its life it is gone");
        fade.update(&w.ecs, now);
        assert_eq!(fade.ghost_count(), 1);
        assert!(
            fade.total_energy(now) <= last_live + 1e-6,
            "an expired drop's ghost cannot be brighter than its last live frame"
        );
        // What it still throws is a few hundredths of a rank's armed light: the
        // natural expiry carries no pop, so the ghost has nothing to hide.
        assert!(fade.total_energy(now) < 0.05);
        for i in 1..=36u32 {
            let t = now + i as f32 * DT;
            assert!(fade.total_energy(t) <= last_live + 1e-6);
        }
        assert_eq!(fade.total_energy(now + DROPPING_FADE_SECS), 0.0);
    }

    /// The list is bounded: a crowded floor loses its oldest ghosts, never
    /// grows past [`MAX_DROPPING_GHOSTS`], and allocates nothing to do it.
    #[test]
    fn ghosts_are_capped() {
        let mut w = arena(63, 5);
        let mut fade = DroppingFade::new();
        let now = 0.0;
        for wave in 0..3u64 {
            // A full cap of fresh drops, registered as live…
            let mut spawned = [None; MAX_TRACKED];
            for (i, slot) in spawned.iter_mut().enumerate() {
                let id = wave * 16 + i as u64;
                *slot = Some(w.ecs.spawn((
                    Dropping {
                        pos: Vec2::new(40.0 + i as f32 * 4.0, 40.0),
                        age: DROPPING_ARM,
                        damage: 3.0,
                        jumps: 1,
                        reach: 14.0,
                        radius: 7.0,
                        seed: id as u32,
                        level: 3,
                    },
                    SpawnId(id),
                )).id());
            }
            // The earlier waves' ghosts are still held (the clock is not
            // moved), so a live dropping must add none to them.
            let held = fade.ghost_count();
            fade.update(&w.ecs, now);
            assert_eq!(fade.ghost_count(), held, "live droppings are not ghosts");
            // …then all removed on one tick (a crowd walking over them).
            for entity in spawned.into_iter().flatten() {
                let _ = w.ecs.despawn(entity);
            }
            fade.update(&w.ecs, now);
            assert_eq!(
                fade.ghost_count(),
                ((wave as usize + 1) * MAX_TRACKED).min(MAX_DROPPING_GHOSTS),
                "every removed drop leaves a ghost until the list is full"
            );
        }
        assert_eq!(
            fade.ghost_count(),
            MAX_DROPPING_GHOSTS,
            "the ghost list is capped"
        );
        // Oldest ghosts made room for the newest ones: the survivors are the
        // last wave's (they all start at `now`, so they are the newest eight).
        assert!(fade.total_energy(now) > 0.0);
        // Freeing them is bounded too: one full fade empties the list.
        fade.update(&w.ecs, now + DROPPING_FADE_SECS);
        assert_eq!(fade.ghost_count(), 0);
    }

    /// Every ghost's sprite position, rank and fade bits: `Dropping` carries no
    /// `PartialEq` (the simulation does not compare droppings), so a frame's
    /// ghost picture is compared by these values instead.
    fn ghost_picture(fade: &DroppingFade, now: f32) -> Vec<(i32, i32, u8, u32)> {
        fade.snapshot(now)
            .iter()
            .flatten()
            .map(|(drop, f)| {
                (
                    drop.pos.x.to_bits() as i32,
                    drop.pos.y.to_bits() as i32,
                    drop.level,
                    f.to_bits(),
                )
            })
            .collect()
    }

    /// A frozen frame draws identically: the fade reads `elapsed`, not the
    /// frame count, so nothing changes until the simulation ticks.
    #[test]
    fn a_frozen_clock_draws_the_same_ghosts() {
        let mut w = arena(64, 2);
        assert!(run_fire(&mut w));
        let mut fade = DroppingFade::new();
        fade.update(&w.ecs, 0.0);
        let entity = {
            let mut q = w.ecs.query::<(Entity, &Dropping)>();
            q.iter(&w.ecs).map(|(e, _)| e).next().expect("the drop")
        };
        let _ = w.ecs.despawn(entity);
        fade.update(&w.ecs, 1.0);
        let base = ghost_picture(&fade, 1.0);
        assert_eq!(base.len(), 1, "there is a ghost");
        for _ in 0..10 {
            fade.update(&w.ecs, 1.0);
            assert_eq!(
                base,
                ghost_picture(&fade, 1.0),
                "a frozen frame must not move a ghost"
            );
        }
        // One tick of the same clock length does move it.
        assert_ne!(base, ghost_picture(&fade, 1.0 + DT));
    }
}
