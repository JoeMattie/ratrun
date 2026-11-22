//! The flatbed's hazards: holes in the bed, the slipstream, sliding cargo and
//! the expansion joints.
//!
//! The layout data lives in [`crate::game::level::TrailerDeck`] (filled by
//! `levelgen::trailer`) and is drawn by `art_trailer`, so what the player sees
//! is what this system reads. Nothing here runs on any other theme: the first
//! line is a `Theme::Trailer` check, so the three campaign floors keep their
//! exact tick (and their golden traces).
//!
//! ## Ordering
//!
//! `trailer_system` runs in the slot between contact and the reap, after the
//! rat and the horde have moved and contact damage has landed. Within that
//! slot the order is: evolved weapons, `enemy_wave3::wave3_system` (so a Bell
//! Flea that lands on a hole this tick falls this tick), **this system**,
//! `weapons_wave3a::hantavirus_death_system` (a fatal fall passes its host's
//! jumps on), then `loot::reap_enemies_system`:
//!
//! * a body that ends the tick over a [`Pit`] has already spent its movement,
//!   so the fall is measured from the position the player saw;
//! * fall damage lands before the reaper, so a fall that kills drops loot and
//!   scores exactly like any other kill;
//! * it is after the last reader of the tick's `EnemyIndex` (it writes
//!   positions and stun only — never spawns or despawns), so the index stays
//!   valid for `contact_system` before it and the Hantavirus death jump
//!   after it. Reaping is what retires the index.
//! * a Wick Moth flies and never falls; every other kind (a Bell Flea
//!   teleports to its locked landing point, so it is always on the ground)
//!   falls by the same rule. A fall has no direction: directional armour does
//!   not apply to it.
//!
//! ## Determinism
//!
//! The enemy walk is sorted by [`SpawnId`] (the legacy `Vec` order). The only
//! rng draw is `vfx::emit_player_hurt` on a falling rat, which is at most one
//! draw per player fall, in a fixed place. Nothing allocates per tick: the
//! order scratch is a `Local`, and the bump index is a `Local<u32>`.
//!
//! ## Rules (the designer's calls for this floor)
//!
//! | hazard | rat | horde |
//! |---|---|---|
//! | broken plank / missing rail ([`Pit`]) | 8 damage, put back on the nearest solid plank, 1.1 s of i-frames so a second hole cannot chain | 35 % of max HP, put back on the nearest plank it fits, 0.6 s stunned — and the plank it lands on is at least two cells from any hole, so it cannot flicker |
//! | sliding cargo | 10 damage and a shove along the strapping | not touched (the crate is at rat height) |
//! | slipstream | — | small bodies (radius ≤ 3.5 px: the bat, skitterer, spitter) are dragged tailward at 9 px/s |
//! | expansion joint | one `Sfx::Bump` every [`crate::game::art_trailer::BUMP_PERIOD`] s (the art's own one-pixel dip) | — |
//! | the Magpie's gust (phase 3) | the bird's own push-only hazard shoves the rat; this pass adds the *wind* half of it | gems and small bodies outside a `wind` stretch are dragged tailward too, at [`GUST_WIND_GEMS`] / [`GUST_WIND_ENEMIES`] px/s |
//!
//! The slipstream moves gems too, and stops at the deck edge: a gem must never
//! be blown onto the road (it would be unreachable).

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::art_trailer::BUMP_PERIOD;
use crate::game::collision::SizeClass;
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, RunCounters, SfxQueue, TickInput};
use crate::game::art_boss_magpie;
use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Gold, Pickup};
use crate::game::level::{Level, Theme, TrailerDeck};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

/// Damage a falling rat takes (before armor).
pub const FALL_PLAYER_DAMAGE: f32 = 8.0;
/// The rat's i-frames after a fall: long enough that two holes a step apart
/// cannot chain into a death, short enough to stay a threat.
pub const FALL_PLAYER_IFRAMES: f32 = 1.1;
/// Share of an ordinary enemy's max HP a fall takes.
pub const FALL_ENEMY_FRACTION: f32 = 0.35;
/// How long a fallen enemy lies stunned on the plank it lands on.
pub const FALL_ENEMY_STUN: f32 = 0.6;
/// Damage the sliding crate deals to the rat.
pub const SLIDE_DAMAGE: f32 = 10.0;
/// How far the slipstream drags a light body, in pixels per second.
pub const WIND_PULL_ENEMIES: f32 = 9.0;
/// How far the slipstream drags a gem, in pixels per second.
pub const WIND_PULL_GEMS: f32 = 14.0;
/// Small bodies this light are dragged by the slipstream.
const WIND_MAX_RADIUS: f32 = 3.5;
/// How many cells out the "nearest solid plank" search looks.
const RECOVERY_RADIUS: i32 = 7;
/// The fall search keeps this many cells from any hole for the horde, so an
/// ejected body is not standing on the lip of the hole it just fell into.
const ENEMY_RECOVERY_MARGIN: i32 = 2;

/// How far the Magpie's wing-beat drags a gem that the deck's own slipstream
/// was not already moving, in px/s.
pub const GUST_WIND_GEMS: f32 = 44.0;
/// The same for light bodies (a bird's wing-beat is nothing to a brute).
pub const GUST_WIND_ENEMIES: f32 = 22.0;
/// How hard the Redtail's hover downdraft shoves a loose gem off the planks
/// under it (px/s, radial). The light bodies get half of it: a wing-beat is
/// nothing to a brute.
pub const HAWK_DRAFT_PUSH: f32 = 52.0;

/// The flatbed's per-tick hazard pass. See the module docs for the order.
#[allow(clippy::too_many_arguments)]
pub fn trailer_system(
    level: Res<Level>,
    counters: Res<RunCounters>,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut gems: Query<&mut Gem>,
    // Loot on the deck: a drop that lands in a hole is nudged onto the nearest
    // plank (see the "loot on the deck" pass), and the slipstream drags coins
    // exactly as it drags gems.
    mut gold: Query<&mut Gold>,
    mut pickups: Query<&mut Pickup>,
    mut sfx: ResMut<SfxQueue>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    bosses: Query<&Boss>,
    mut order: Local<Vec<(u64, Entity)>>,
    mut bump: Local<u32>,
) {
    if level.theme != Theme::Trailer || level.trailer.is_empty() {
        return;
    }
    let dt = input.dt;
    let deck = &level.trailer;

    // ---- the expansion joints -------------------------------------------
    //
    // The art dips the deck by a pixel on the same clock; this is its thud.
    let joint = (counters.elapsed / BUMP_PERIOD) as u32;
    if joint != *bump {
        *bump = joint;
        sfx.0.push(Sfx::Bump);
    }

    // ---- the holes -------------------------------------------------------
    if player.alive() && deck.pit_at(player.pos).is_some() {
        if let Some(safe) = nearest_plank(&level, player.pos, SizeClass::Small, 0) {
            let fell = player.take_damage(FALL_PLAYER_DAMAGE);
            player.pos = safe;
            player.vel = Vec2::ZERO;
            // The fall's own grace replaces (never shortens) any i-frames.
            player.iframes = player.iframes.max(FALL_PLAYER_IFRAMES);
            if fell {
                vfx::emit_player_hurt(&mut particles, &mut rng.0, safe);
                sfx.0.push(Sfx::Hurt);
            }
        }
    }

    // The horde, in `SpawnId` order (the legacy `Vec` order). A fallen body is
    // ejected two cells clear of every hole and stunned, so it stumbles once
    // instead of flickering in and out of the crack.
    order.clear();
    order.extend(enemies.iter().map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|&(id, _)| id);
    let mut any_fell = false;
    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        // A Wick Moth flies: it crosses the holes and never falls. (A Bell
        // Flea is on the ground between hops and teleports to its locked
        // landing point, so it falls exactly when a landing point is a hole,
        // by the ordinary rule below.)
        //
        // A named miniboss does not fall either: a pit must never kill it (no
        // rat earned that chest) or strand it across the deck, so it treads
        // the holes like the moth flies them.
        if !e.alive()
            || e.named
            || matches!(e.kind, EnemyKind::WickMoth | EnemyKind::MurderHornet)
            || deck.pit_at(e.pos).is_none()
        {
            continue;
        }
        let class = SizeClass::of_radius(e.radius);
        if let Some(safe) = nearest_plank(&level, e.pos, class, ENEMY_RECOVERY_MARGIN) {
            // A fall has no direction (the unmodified multiplier, whatever a
            // Pillbug's shell is doing); `note_damage` books the side effects
            // (a Porter drops its raise).
            e.hp -= e.max_hp * FALL_ENEMY_FRACTION;
            e.note_damage();
            e.pos = safe;
            e.vel = Vec2::ZERO;
            e.stun = e.stun.max(FALL_ENEMY_STUN);
            e.contact_cd = e.contact_cd.max(FALL_ENEMY_STUN);
            any_fell = true;
        }
    }
    if any_fell {
        sfx.0.push(Sfx::TrapTrigger);
    }

    // ---- loot on the deck ------------------------------------------------
    //
    // A gem, coin or pocket find dropped over a hole would sit where the rat
    // cannot stand (its touch radius never reaches the middle of a pit) until
    // the level ends. Nudge each onto the nearest plank instead: the same
    // "nearest solid plank" walk the fallen bodies use, and no rng.
    for mut gem in gems.iter_mut() {
        if deck.pit_at(gem.pos).is_some() {
            if let Some(safe) = nearest_plank(&level, gem.pos, SizeClass::Small, 0) {
                gem.pos = safe;
            }
        }
    }
    for mut coin in gold.iter_mut() {
        if deck.pit_at(coin.pos).is_some() {
            if let Some(safe) = nearest_plank(&level, coin.pos, SizeClass::Small, 0) {
                coin.pos = safe;
            }
        }
    }
    for mut find in pickups.iter_mut() {
        if deck.pit_at(find.pos).is_some() {
            if let Some(safe) = nearest_plank(&level, find.pos, SizeClass::Small, 0) {
                find.pos = safe;
            }
        }
    }

    // ---- sliding cargo ---------------------------------------------------
    //
    // The crate's position is a pure function of the run clock (the art uses
    // the same function), so there is no state to keep here.
    if player.alive() {
        for slide in &deck.slides {
            let offset = TrailerDeck::slide_offset(slide, counters.elapsed);
            let centre = slide.rest + slide.dir * offset;
            let d = player.pos - centre;
            if d.x.abs() <= slide.half.x + PLAYER_RADIUS && d.y.abs() <= slide.half.y + PLAYER_RADIUS {
                if player.take_damage(SLIDE_DAMAGE) {
                    let push = slide.dir * (slide.half.x + PLAYER_RADIUS + 2.0);
                    player.pos = level.tiles.resolve_circle(
                        centre + push,
                        PLAYER_RADIUS,
                        SizeClass::Small,
                        None,
                    );
                    player.vel = Vec2::ZERO;
                    vfx::emit_player_hurt(&mut particles, &mut rng.0, player.pos);
                    sfx.0.push(Sfx::Hurt);
                }
                break;
            }
        }
    }

    // ---- the magpie's wing-beat -----------------------------------------
    //
    // Phase 3's gusts are the deck's own slipstream, stirred: the rat is shoved
    // by the bird's push-only hazard (which the boss owns), and this is what the
    // same wing-beat does to everything lying loose on the planks. Read from the
    // boss component — a pure function of its state and progress — so there is
    // no new resource, no rng and no allocation, and nothing here runs before
    // the rat's own hazard has already been resolved.
    if dt > 0.0 && !deck.is_empty() {
        let gusting = bosses.iter().any(|boss| {
            !boss.dead
                && boss.kind == BossKind::Magpie
                && matches!(boss.state, BossState::Magpie(state)
                    if art_boss_magpie::gust_pulse_active(state, boss.progress()))
        });
        if gusting {
            for mut gem in gems.iter_mut() {
                let to = gem.pos + Vec2::new(-GUST_WIND_GEMS * dt, 0.0);
                if slipstream_floor(&level, to) {
                    gem.pos = to;
                }
            }
            for &(_, entity) in order.iter() {
                let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                    continue;
                };
                if !e.alive() || e.radius > WIND_MAX_RADIUS || e.stun > 0.0 {
                    continue;
                }
                let to = e.pos + Vec2::new(-GUST_WIND_ENEMIES * dt, 0.0);
                if slipstream_floor(&level, to) {
                    e.pos = to;
                }
            }
        }
    }

    // ---- the redtail's downdraft ----------------------------------------
    //
    // The hawk's hover beats damage-free rings at the *rat* (they are its own
    // hazards, queued by the controller). This is what the same wing-beat does
    // to everything lying loose on the planks: a radial shove away from the
    // bird, so a hovering raptor scatters a gem pile and shoves light bodies
    // out from under itself. Read from the boss component — a pure function of
    // its state and progress — so there is no new resource and no rng.
    if dt > 0.0 && !deck.is_empty() {
        let mut hover: Option<(Vec2, f32)> = None;
        for boss in bosses.iter() {
            if boss.dead || boss.kind != BossKind::Hawk {
                continue;
            }
            let BossState::Hawk(state) = boss.state else {
                continue;
            };
            if crate::game::art_boss_hawk::downdraft_active(state, boss.progress()) {
                hover = Some((boss.pos, HAWK_DRAFT_PUSH));
            }
        }
        if let Some((from, push)) = hover {
            for mut gem in gems.iter_mut() {
                let away = (gem.pos - from).normalized();
                let to = gem.pos + away * (push * dt);
                if slipstream_floor(&level, to) {
                    gem.pos = to;
                }
            }
            for &(_, entity) in order.iter() {
                let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                    continue;
                };
                if !e.alive() || e.radius > WIND_MAX_RADIUS || e.stun > 0.0 {
                    continue;
                }
                let away = (e.pos - from).normalized();
                let to = e.pos + away * (push * 0.5 * dt);
                if slipstream_floor(&level, to) {
                    e.pos = to;
                }
            }
        }
    }

    // ---- the slipstream --------------------------------------------------
    if deck.wind.is_empty() || dt <= 0.0 {
        return;
    }
    // Gems are light: the wind herds them tailward, but never off the deck.
    for mut gem in gems.iter_mut() {
        if !deck.windy_at(gem.pos) {
            continue;
        }
        let to = gem.pos + Vec2::new(-WIND_PULL_GEMS * dt, 0.0);
        if slipstream_floor(&level, to) {
            gem.pos = to;
        }
    }
    // Coins are just as light (a coin is a gem with a square glint), and a
    // purse left up-wind of the rat is the same loss as an XP gem.
    for mut coin in gold.iter_mut() {
        if !deck.windy_at(coin.pos) {
            continue;
        }
        let to = coin.pos + Vec2::new(-WIND_PULL_GEMS * dt, 0.0);
        if slipstream_floor(&level, to) {
            coin.pos = to;
        }
    }
    // The smallest bodies are dragged too.
    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive()
            || e.radius > WIND_MAX_RADIUS
            || e.stun > 0.0
            || !deck.windy_at(e.pos)
        {
            continue;
        }
        let to = e.pos + Vec2::new(-WIND_PULL_ENEMIES * dt, 0.0);
        if slipstream_floor(&level, to) {
            e.pos = to;
        }
    }
}

/// Is `p` on walkable deck that is not a hole? (The wind must never blow a gem
/// onto the road, under a crate or into a crack.)
#[inline]
fn slipstream_floor(level: &Level, p: Vec2) -> bool {
    if level.trailer.pit_at(p).is_some() {
        return false;
    }
    let (cx, cy) = (
        (p.x / crate::game::collision::TILE_F).floor() as i32,
        (p.y / crate::game::collision::TILE_F).floor() as i32,
    );
    level.tiles.passable(cx, cy, SizeClass::Small)
}

/// The nearest plank a body of `class` can stand on, preferring cells at least
/// `margin` cells clear of every hole.
///
/// The search is a fixed outward ring walk (radius-first), so it is
/// deterministic and allocation-free. `None` means the body is in a hole with
/// no plank within [`RECOVERY_RADIUS`] — a generator invariant violation, not a
/// play state; the caller then leaves the body where it is.
pub fn nearest_plank(
    level: &Level,
    from: Vec2,
    class: SizeClass,
    margin: i32,
) -> Option<Vec2> {
    let (cx, cy) = (
        (from.x / crate::game::collision::TILE_F).floor() as i32,
        (from.y / crate::game::collision::TILE_F).floor() as i32,
    );
    let deck = level.trailer.deck;
    let mut best: Option<(i32, Vec2)> = None;
    for r in 1..=RECOVERY_RADIUS {
        for dy in -r..=r {
            for dx in -r..=r {
                // Ring only: the square shell at exactly `r`.
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let (nx, ny) = (cx + dx, cy + dy);
                if !deck.contains(nx, ny) {
                    continue;
                }
                let centre = crate::game::collision::TileMap::tile_rect(nx, ny).center();
                if level.trailer.pit_at(centre).is_some() {
                    continue;
                }
                if !level.tiles.circle_clear(centre, PLAYER_RADIUS, class) {
                    continue;
                }
                if margin > 0 && near_hole(level, nx, ny, margin) {
                    continue;
                }
                let d = (from - centre).len_sq();
                if best.is_none_or(|(bd, _)| d < bd as f32) {
                    best = Some((d as i32, centre));
                }
            }
        }
        if best.is_some() {
            // The first ring that offers a plank is the closest one; the
            // distance test inside it just picks the nearest cell of the ring.
            return best.map(|(_, p)| p);
        }
    }
    None
}

/// Is any cell within `margin` of `(cx, cy)` a hole?
fn near_hole(level: &Level, cx: i32, cy: i32, margin: i32) -> bool {
    level
        .trailer
        .pits
        .iter()
        .any(|pit| pit.rect.inflate(margin).contains(cx, cy))
}

#[cfg(test)]
mod tests {
    //! Narrow single-system tests (`begin_tick` → `run_system_cached` →
    //! `end_tick`), the same harness the other systems use.

    use super::*;
    use crate::game::gem::Gem;
    use crate::game::level::{Pit, PitKind, SlidingCargo, TileRect};
    use crate::game::world::World;

    /// A trailer world with the hazard list replaced by a hand-built one: one
    /// hole in the middle of a wide open bed.
    fn bed_world() -> World {
        let mut w = World::new(Theme::Trailer, 3);
        // Flatten a test deck: a 20 × 8 bed of open floor.
        let deck = w.level.trailer.deck;
        w.level
            .tiles
            .fill_cells(deck.x, deck.y, deck.w, deck.h, crate::game::collision::TileKind::Open);
        w.level.trailer.pits.clear();
        w.level.trailer.wind.clear();
        w.level.trailer.slides.clear();
        w
    }

    /// One real tick through the persistent schedule (so each system's
    /// `Local` state, including the bump counter, behaves as in a run).
    fn tick(w: &mut World, dt: f32) {
        w.update(dt, Vec2::ZERO, false);
    }

    /// Loot dropped over a hole is nudged onto the nearest plank, and the
    /// slipstream drags coins exactly as it drags gems.
    #[test]
    fn loot_never_stays_in_a_hole_and_the_wind_moves_coins() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let hole = TileRect::new(deck.x + 5, deck.y + 3, 1, 1);
        w.level
            .trailer
            .pits
            .push(Pit::new(hole, PitKind::BrokenPlank));
        let centre = hole.center_world();
        let gem = w.spawn_gem(Gem::new(centre, 1));
        let coin = crate::game::ecs::resources::spawn_with_id(&mut w.ecs, Gold::new(centre, 2));
        tick(&mut w, 1.0 / 60.0);
        let gem_pos = w.ecs.get::<Gem>(gem).unwrap().pos;
        let coin_pos = w.ecs.get::<Gold>(coin).unwrap().pos;
        assert!(w.level.trailer.pit_at(gem_pos).is_none(), "the gem sat in the hole");
        assert!(w.level.trailer.pit_at(coin_pos).is_none(), "the coin sat in the hole");

        // A windy stretch: a coin is blown tailward like a gem.
        w.level.trailer.wind.push(TileRect::new(
            deck.x + 8,
            deck.y + 4,
            4,
            2,
        ));
        let windy = TileRect::new(deck.x + 8, deck.y + 4, 4, 2).center_world();
        let coin = crate::game::ecs::resources::spawn_with_id(&mut w.ecs, Gold::new(windy, 2));
        let x0 = w.ecs.get::<Gold>(coin).unwrap().pos.x;
        for _ in 0..10 {
            tick(&mut w, 1.0 / 60.0);
        }
        let x1 = w.ecs.get::<Gold>(coin).unwrap().pos.x;
        assert!(x1 < x0, "the slipstream must herd coins too ({x0} -> {x1})");
    }

    /// A Bell Flea never picks a landing on a hole: it waits for open planks.
    #[test]
    fn a_bell_flea_refuses_a_pit_landing() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        // A wall of holes straight between the flea and the rat.
        for dx in 3..7 {
            w.level
                .trailer
                .pits
                .push(Pit::new(TileRect::new(deck.x + dx, deck.y + 4, 1, 1), PitKind::BrokenPlank));
        }
        let from = deck.center_world() + Vec2::new(-40.0, 0.0);
        let flea = w.spawn_enemy(EnemyKind::BellFlea, from, 1.0);
        {
            let mut e = w.ecs.get_mut::<Enemy>(flea).unwrap();
            e.w3.armed = 1;
            e.w3.flea_cd = 0.0;
        }
        for _ in 0..120 {
            w.update(1.0 / 60.0, Vec2::ZERO, false);
            let e = w.ecs.get::<Enemy>(flea).unwrap();
            if e.w3.flea_phase == 1 {
                assert!(
                    w.level.trailer.pit_at(e.w3.hop_to).is_none(),
                    "the flea chose a landing in a hole"
                );
            }
        }
    }

    #[test]
    fn the_rat_falls_into_a_hole_and_is_put_back_on_a_plank() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let hole = TileRect::new(deck.x + 5, deck.y + 3, 1, 1);
        w.level
            .trailer
            .pits
            .push(Pit::new(hole, PitKind::BrokenPlank));
        let hole_centre = hole.center_world();
        w.player.pos = hole_centre;
        let hp = w.player.hp;
        w.player.iframes = 0.0;
        tick(&mut w, 1.0 / 60.0);
        assert!(w.player.hp < hp, "the fall did not hurt");
        assert!(
            w.level.trailer.pit_at(w.player.pos).is_none(),
            "the rat was left in the hole"
        );
        assert!(w.player.iframes > 0.5, "no recovery i-frames");
        assert!(
            w.level.tiles.circle_clear(w.player.pos, PLAYER_RADIUS, SizeClass::Small),
            "the rat was put back inside a wall"
        );
    }

    #[test]
    fn a_missing_rail_section_is_the_same_fall() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let gap = TileRect::new(deck.x + 2, deck.y - 1, 2, 1);
        // In a generated map the missing rail section is open deck; mirror that.
        w.level
            .tiles
            .fill_cells(gap.x, gap.y, gap.w, gap.h, crate::game::collision::TileKind::Open);
        w.level.trailer.pits.push(Pit::new(gap, PitKind::RailGap));
        w.player.pos = gap.center_world();
        w.player.iframes = 0.0;
        let hp = w.player.hp;
        tick(&mut w, 1.0 / 60.0);
        assert!(w.player.hp < hp);
        assert!(w.level.trailer.pit_at(w.player.pos).is_none());
    }

    /// The wave-3 kinds on the flatbed: a Wick Moth flies over a hole, every
    /// grounded kind (a Bell Flea standing on its landing point included)
    /// falls like any other body.
    #[test]
    fn a_moth_flies_over_a_hole_and_a_flea_on_one_falls() {
        use crate::game::enemy::EnemyKind;
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let hole = TileRect::new(deck.x + 8, deck.y + 4, 1, 1);
        w.level
            .trailer
            .pits
            .push(Pit::new(hole, PitKind::BrokenPlank));
        let moth = w.spawn_enemy(EnemyKind::WickMoth, hole.center_world(), 1.0);
        let flea = w.spawn_enemy(EnemyKind::BellFlea, hole.center_world(), 1.0);
        let (moth_hp, flea_hp) = (
            w.ecs.get::<Enemy>(moth).unwrap().hp,
            w.ecs.get::<Enemy>(flea).unwrap().hp,
        );
        tick(&mut w, 1.0 / 60.0);
        let m = w.ecs.get::<Enemy>(moth).unwrap();
        assert_eq!(m.hp, moth_hp, "the moth fell");
        assert_eq!(m.stun, 0.0, "the moth was stunned by a fall");
        let f = w.ecs.get::<Enemy>(flea).unwrap();
        assert!(f.hp < flea_hp, "the flea did not fall");
        assert!(w.level.trailer.pit_at(f.pos).is_none(), "the flea was left in the hole");
    }

    #[test]
    fn an_enemy_falls_stunned_and_clear_of_the_hole() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let hole = TileRect::new(deck.x + 8, deck.y + 4, 1, 1);
        w.level
            .trailer
            .pits
            .push(Pit::new(hole, PitKind::BrokenPlank));
        let e = w.spawn_enemy(crate::game::enemy::EnemyKind::Skitterer, hole.center_world(), 1.0);
        let hp = w.ecs.get::<Enemy>(e).unwrap().hp;
        tick(&mut w, 1.0 / 60.0);
        let enemy = w.ecs.get::<Enemy>(e).expect("alive");
        assert!(enemy.hp < hp, "the fall did not hurt the skitterer");
        assert!(enemy.stun > 0.0, "the fallen enemy was not stunned");
        assert!(
            w.level.trailer.pit_at(enemy.pos).is_none(),
            "the enemy was left in the hole"
        );
        // At least two cells from the hole, so it cannot fall straight back in.
        let gap = (
            (enemy.pos.x / 16.0) as i32 - hole.x,
            (enemy.pos.y / 16.0) as i32 - hole.y,
        );
        assert!(
            gap.0.abs() >= ENEMY_RECOVERY_MARGIN - 1 || gap.1.abs() >= ENEMY_RECOVERY_MARGIN - 1,
            "the enemy was ejected onto the lip of the hole: {gap:?}"
        );
    }

    #[test]
    fn the_slipstream_drags_a_gem_tailward_but_not_off_the_bed() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        w.level
            .trailer
            .wind
            .push(TileRect::new(deck.x + 6, deck.y + 2, 8, 6));
        let wind_centre = TileRect::new(deck.x + 6, deck.y + 2, 8, 6).center_world();
        let gem = w.spawn_gem(Gem::new(wind_centre, 1));
        let before = w.ecs.get::<Gem>(gem).unwrap().pos.x;
        tick(&mut w, 0.1);
        let after = w.ecs.get::<Gem>(gem).unwrap().pos.x;
        assert!(after < before, "the wind did not push the gem tailward ({before} -> {after})");
        // The deck's west edge stops it: the wind never pushes a gem into the
        // road.
        w.ecs.get_mut::<Gem>(gem).unwrap().pos = Vec2::new((deck.x as f32 + 0.4) * 16.0, wind_centre.y);
        tick(&mut w, 0.5);
        let x = w.ecs.get::<Gem>(gem).unwrap().pos.x;
        assert!(
            x >= deck.x as f32 * 16.0,
            "the wind blew the gem off the bed: {x}"
        );
    }

    #[test]
    fn a_sliding_crate_hurts_the_rat_it_sweeps() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let rest = Vec2::new((deck.x as f32 + 6.0) * 16.0, (deck.y as f32 + 4.0) * 16.0);
        w.level.trailer.slides.push(SlidingCargo {
            rest,
            dir: Vec2::new(1.0, 0.0),
            travel: 40.0,
            phase: 0.0,
            half: Vec2::new(10.0, 9.0),
            warn: 1.2,
        });
        // Put the rat where the crate is once it has shifted out (2 s in).
        let offset = TrailerDeck::slide_offset(&w.level.trailer.slides[0], 2.0);
        assert!(offset > 10.0, "the crate did not move in the window");
        w.elapsed = 2.0;
        w.player.pos = rest + Vec2::new(offset, 0.0);
        w.player.iframes = 0.0;
        let hp = w.player.hp;
        tick(&mut w, 1.0 / 60.0);
        assert!(w.player.hp < hp, "the crate swept through the rat without a hit");
    }

    #[test]
    fn nothing_runs_off_the_flatbed() {
        let mut w = World::new(Theme::Sewer, 3);
        w.player.pos = w.level.start;
        let hp = w.player.hp;
        w.player.iframes = 0.0;
        tick(&mut w, 1.0 / 60.0);
        assert_eq!(w.player.hp, hp, "a campaign floor took fall damage");
    }

    #[test]
    fn a_fall_kills_through_the_normal_loot_path() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let hole = TileRect::new(deck.x + 8, deck.y + 4, 1, 1);
        w.level
            .trailer
            .pits
            .push(Pit::new(hole, PitKind::BrokenPlank));
        let e = w.spawn_enemy(crate::game::enemy::EnemyKind::Skitterer, hole.center_world(), 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = 0.2;
        tick(&mut w, 1.0 / 60.0);
        assert!(
            w.ecs.get::<Enemy>(e).is_none(),
            "a fatal fall did not reap the body (and so dropped no loot)"
        );
    }

    /// Twin runs on the flatbed: same seed, same bot, same inputs — the same
    /// hazards fire, the same falls land and the same bodies end up in the same
    /// places. (The golden harness pins the three campaign themes; this pins
    /// the trailer's own hazards without touching those hashes.)
    #[test]
    fn twin_runs_on_the_flatbed_match() {
        use crate::game::bot::{Bot, Skill};

        let mut a = World::new(Theme::Trailer, 77);
        let mut b = World::new(Theme::Trailer, 77);
        let mut bot_a = Bot::new(Skill::Skilled, 3);
        let mut bot_b = Bot::new(Skill::Skilled, 3);
        bot_a.on_new_level(0);
        bot_b.on_new_level(0);
        let dt = 1.0 / 60.0;
        for tick in 0..1800u32 {
            let ia = bot_a.decide(&a, dt);
            let ib = bot_b.decide(&b, dt);
            a.update(dt, ia.move_dir, ia.dash);
            b.update(dt, ib.move_dir, ib.dash);
            assert_eq!(
                a.player.pos.x.to_bits(),
                b.player.pos.x.to_bits(),
                "player x diverged at tick {tick}"
            );
            assert_eq!(a.player.pos.y.to_bits(), b.player.pos.y.to_bits());
            assert_eq!(a.player.hp.to_bits(), b.player.hp.to_bits());
            assert_eq!(a.kills, b.kills);
            assert_eq!(a.score, b.score);
            let ea = a.enemies();
            let eb = b.enemies();
            assert_eq!(ea.len(), eb.len(), "enemy count diverged at tick {tick}");
            for (x, y) in ea.iter().zip(eb.iter()) {
                assert_eq!(x.pos.x.to_bits(), y.pos.x.to_bits(), "tick {tick}");
                assert_eq!(x.pos.y.to_bits(), y.pos.y.to_bits(), "tick {tick}");
                assert_eq!(x.hp.to_bits(), y.hp.to_bits(), "tick {tick}");
            }
        }
    }

    /// The owner's floor, measured: a full 120 × 40-cell view of the flatbed is
    /// 120 × 80 pixels of planks, road, cargo and rails. Print the cost and
    /// fail if it stops fitting comfortably inside a 60 FPS frame.
    #[test]
    fn the_flatbed_floor_and_world_draw_fit_a_frame() {
        use crate::render::framebuffer::PixelBuffer;

        let mut w = World::new(Theme::Trailer, 5);
        w.elapsed = 7.0;
        let mut pb = PixelBuffer::new(120, 80);
        let x = w.cam.x.round() as i32..w.cam.x.round() as i32 + 120;
        let y = w.cam.y.round() as i32..w.cam.y.round() as i32 + 80;
        // Warm the caches and the buffer allocation.
        for _ in 0..8 {
            crate::game::art_trailer::draw_floor(&mut pb, &w.level, w.cam, 7.0, &x, &y);
            w.draw(&mut pb);
        }
        let best_us = |mut f: Box<dyn FnMut() + '_>| -> u128 {
            let mut best = u128::MAX;
            for _ in 0..12 {
                let start = std::time::Instant::now();
                for _ in 0..10 {
                    f();
                }
                best = best.min(start.elapsed().as_micros() / 10);
            }
            best
        };
        let floor = best_us(Box::new(|| {
            crate::game::art_trailer::draw_floor(&mut pb, &w.level, w.cam, 7.0, &x, &y)
        }));
        let whole = best_us(Box::new(|| w.draw(&mut pb)));
        println!("flatbed draw, 120 x 40 view: floor {floor} µs, whole world {whole} µs");
        // 60 FPS is 16 600 µs. Leave most of it for the HUD, the terminal
        // blit and the sim: the measured numbers are well under a tenth of it.
        assert!(
            floor < 2_000,
            "the flatbed floor costs {floor} µs for a 120 x 40 view"
        );
        assert!(
            whole < 8_000,
            "the flatbed world draw costs {whole} µs for a 120 x 40 view"
        );
    }

    #[test]
    fn the_expansion_joint_thuds_once_per_period() {
        let mut w = bed_world();
        tick(&mut w, 0.1);
        let bumps = |w: &World| w.sfx.iter().filter(|s| **s == Sfx::Bump).count();
        assert_eq!(bumps(&w), 0, "a bump at t=0.1");
        // Cross the joint: the counter advances exactly once.
        w.elapsed = BUMP_PERIOD - 0.05;
        tick(&mut w, 0.1);
        assert_eq!(bumps(&w), 1, "the joint did not thud");
        tick(&mut w, 0.1);
        assert_eq!(bumps(&w), 1, "the joint thudded twice in one period");
    }
}
