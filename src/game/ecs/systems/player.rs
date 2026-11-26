//! (a) Player movement, dash, sub-stepping and timers.
//!
//! Based on the legacy `World::update_player`. Ordinary dashes now last 0.32 s
//! at the same boost, with charge scaling that duration up to 0.64 s. The
//! scripted dash traces deliberately move through player positions, immunity
//! and the additional trail emissions. Level generation is unaffected.
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
//! incremented by `World::update` before `begin_tick`, exactly where the
//! legacy `update` did it.
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
//!   draws. Start still precedes movement and trail still follows it.
//! - Performance: this system allocates nothing per tick — no query, no
//!   sorting, no `Vec` scratch. `stats` is a plain 8-`f32` stack copy (exactly
//!   legacy's `clone()`), and `Particles` pushes into pre-allocated
//!   `VecDeque`s with O(1) eviction.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::SizeClass;
use crate::game::ecs::constants::PLAYER_SUBSTEP;
use crate::game::ecs::resources::{DashTrack, DebugFlags, RngRes, Shake, SfxQueue, TickInput};
use crate::game::level::Level;
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;

/// How long the on-rat gauge and the HUD's dash module stay lit once the dash
/// is back. (`loot::pickups_system` uses it for the Clean Socks' refill too.)
pub const DASH_READY_SECS: f32 = 0.35;
/// How long a refused dash shakes the gauge and turns the HUD module red. Also
/// the throttle on the refused blip (see `player_system`).
pub const DASH_DENIED_SECS: f32 = 0.16;
/// How long an ordinary dash runs (seconds). A charged Super Dash scales this
/// by up to [`SUPER_DASH_DISTANCE`], which is the distance.
pub const DASH_SECS: f32 = 0.32;
/// Dash speed as a multiple of the rat's current move speed.
pub const DASH_BOOST: f32 = 3.2;
/// Seconds of holding the dash input to fill the Super Dash meter at weapon
/// level 1. Each weapon level takes [`SUPER_DASH_CHARGE_STEP`] less.
pub const SUPER_DASH_CHARGE_SECS: f32 = 0.90;
/// How much faster each Super Dash level fills the meter (seconds off the
/// level-1 fill).
pub const SUPER_DASH_CHARGE_STEP: f32 = 0.04;
/// A hold shorter than this is today's ordinary dash: same distance, same
/// cooldown, same crossing damage.
pub const SUPER_DASH_TAP_SECS: f32 = 0.15;
/// How far a full charge reaches, as a multiple of the ordinary dash's travel
/// (the dash's own duration is scaled, so the boost stays the same).
pub const SUPER_DASH_DISTANCE: f32 = 2.0;
/// Immunity granted past the end of the dash's travel, so the whole route
/// is covered and nothing can land on the frame it stops.
const DASH_IFRAME_PAD: f32 = 0.02;
/// Screen shake on a charged release, in px, at full charge (the ordinary dash
/// has none).
const SUPER_DASH_SHAKE: f32 = 7.0;

/// Travel time for a dash with a charge between zero and one.
pub fn dash_duration(charge: f32) -> f32 {
    dash_duration_from(DASH_SECS, charge)
}

pub fn dash_duration_from(base: f32, charge: f32) -> f32 {
    base * (1.0 + (SUPER_DASH_DISTANCE - 1.0) * charge)
}

/// Seconds of holding that fill the Super Dash meter at weapon `level`
/// (1..=5): 0.90 s at L1 down to 0.74 s at L5 ("a little faster per level").
pub fn super_dash_charge_secs(level: u32) -> f32 {
    (SUPER_DASH_CHARGE_SECS - SUPER_DASH_CHARGE_STEP * level.saturating_sub(1).min(4) as f32)
        .max(0.55)
}

#[allow(clippy::too_many_arguments)]
pub fn player_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    mut level: ResMut<Level>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
    // w3-weapons-b: the dash owner's record of what the dash actually did, so
    // the Super Dash weapon never has to guess (see the hooks below).
    mut dash_track: ResMut<DashTrack>,
    // A charged release lands heavier than an ordinary dash.
    mut shake: ResMut<Shake>,
    // Console `noclip on`: skip tile collision. Default false = normal.
    debug: Res<DebugFlags>,
    // A dash press the cooldown refused whose input is still held (Super Dash
    // only): the one hold that may start a charge without a fresh press edge.
    // Input bookkeeping, not simulation state: it is only ever set from live
    // hold edges, which the bot and the goldens never pass.
    mut hold_buffered: Local<bool>,
    mut traps: ResMut<super::traps::TrapRuntime>,
) {
    let dt = input.dt;
    let move_dir = input.move_dir;
    let dash = input.dash;
    let mut pot_hold = level.objects.crab_pots.iter().position(|pot| pot.caught_rat);

    // Legacy 184: a snapshot, not a borrow, exactly like the legacy clone.
    let stats = player.loadout.stats;
    if move_dir.len_sq() > 0.01 {
        player.face = move_dir.normalized();
    }
    // Wave 3 controls: while aiming, the rat faces the reticle (its dash and
    // every facing-based effect follow the pointer, not the walk). With no
    // aim (`World::update`, the bot, the goldens) this changes nothing.
    if let Some(d) = input.aim.map(|p| p - player.pos).filter(|v| v.len_sq() > 4.0) {
        player.face = d.normalized();
    }

    // ---- Super Dash: hold to charge, release to fire -----------------------
    // Only while the weapon is owned. A *tap* (released inside
    // `SUPER_DASH_TAP_SECS`) is today's ordinary dash: same distance, same
    // cooldown, same crossing damage. A longer hold fills the meter at
    // `super_dash_charge_secs(level)` while the rat keeps moving; releasing
    // fires the charged dash. A full meter can be held indefinitely (no
    // auto-fire) and grants no immunity until it is released.
    //
    // `release` is the meter to fire with on this tick (`Some(0.0)` = an
    // ordinary dash); `None` = no dash requested.
    let super_dash_level = player
        .loadout
        .weapons
        .iter()
        .find(|w| w.kind == WeaponKind::SuperDash && !player.buffs.weapons_suspended())
        .map(|w| u32::from(w.level.max(1)));
    let mut release: Option<f32> = None;
    match super_dash_level {
        None => {
            // No weapon: drop any stale meter. The press-to-dash path below is
            // exactly the pre-charge one, with no release latency.
            player.dash_charging = false;
            player.dash_charge = 0.0;
            *hold_buffered = false;
        }
        Some(level) => {
            if !input.dash_held || input.dash_released {
                // The buffered hold ends with the hold itself.
                *hold_buffered = false;
            }
            if input.charge > 0.0 && (dash || input.dash_held || input.dash_released) {
                // A scripted charged release (`World::update_with_charge`: the
                // bot and the single-system tests hand the meter over whole).
                release = Some(input.charge.clamp(0.0, 1.0));
                player.dash_charging = false;
                player.dash_charge = 0.0;
            } else if player.dash_charging {
                if input.dash_released {
                    let frac = player.dash_charge.clamp(0.0, 1.0);
                    // A hold shorter than the tap window is an ordinary dash.
                    let held = frac * super_dash_charge_secs(level);
                    release = Some(if held < SUPER_DASH_TAP_SECS { 0.0 } else { frac });
                    player.dash_charging = false;
                    player.dash_charge = 0.0;
                } else if !input.dash_held {
                    // The hold vanished without a release edge (the terminal
                    // lost focus, the button's owner went away): drop the
                    // charge, never fire it.
                    player.dash_charging = false;
                    player.dash_charge = 0.0;
                } else {
                    let before = player.dash_charge;
                    player.dash_charge =
                        (before + dt / super_dash_charge_secs(level)).min(1.0);
                    if before < 1.0 && player.dash_charge >= 1.0 {
                        // The click at a full meter; the whine started on the
                        // press (`Sfx::DashCharge`).
                        sfx.0.push(Sfx::DashCharged);
                    }
                }
            } else if input.dash_down && (input.dash_released || !input.dash_held) {
                // A press and a release inside one tick is a click, not a hold
                // — and on a terminal that cannot report a key release the key
                // can only ever tap (the mouse and the pad still charge).
                release = Some(0.0);
            } else if input.dash_down && player.dash_cd <= 0.0 {
                // A fresh press with the dash ready starts the charge. While
                // the cooldown runs it is the refused press below instead: a
                // charge never starts from a press that could not dash.
                player.dash_charging = true;
                player.dash_charge = 0.0;
                sfx.0.push(Sfx::DashCharge);
            } else if input.dash_down {
                // Pressed while the dash is cooling and kept down: the refused
                // press below, and the hold is remembered.
                *hold_buffered = true;
            } else if *hold_buffered
                && input.dash_held
                && player.dash_cd <= 0.0
                && player.dash_active <= 0.0
            {
                // A press the cooldown refused, still held when the dash comes
                // back: the charge starts now and the release fires it (a
                // short one as a tap, as usual) instead of the input being
                // dropped. Only a hold whose own press was refused *in play,
                // with the weapon owned* counts: a hold that began in a menu,
                // or before the weapon was picked up, never set the flag. A tap
                // during the cooldown stays the refused press.
                *hold_buffered = false;
                player.dash_charging = true;
                player.dash_charge = 0.0;
                sfx.0.push(Sfx::DashCharge);
            }
        }
    }

    // Dash (legacy 189-199), plus the charged release above. `was_cooling` is
    // read here, before the trigger, so the ready tick below is the tick the
    // *old* cooldown ran out (a dash that starts now must not also announce
    // itself as ready).
    let was_cooling = player.dash_cd > 0.0;
    // The App reports a live held dash input as edges; on such a tick the plain
    // `dash` flag must not also fire an instant dash (the release does). Without
    // the weapon, or without hold information (the bot, the harness, `update`),
    // `dash` keeps its press-to-dash meaning exactly as before.
    let live_hold = super_dash_level.is_some()
        && (input.dash_down || input.dash_held || input.dash_released);
    let request: Option<f32> = if let Some(charge) = release {
        Some(charge)
    } else if dash && !live_hold {
        Some(0.0)
    } else if super_dash_level.is_some() && input.dash_down && !player.dash_charging {
        // A press that could not start a charge (the cooldown is still
        // running): the existing refusal feedback, never a free dash.
        Some(0.0)
    } else {
        None
    };
    dash_track.started_this_tick = false;
    if let Some(charge) = request.filter(|_| pot_hold.is_none_or(|i| traps.pot_dash_ready(i))) {
        if player.dash_cd <= 0.0 {
            if let Some(i) = pot_hold.take() {
                traps.dash_out_of_pot(i, &mut level.objects.crab_pots[i]);
            }
            // Up to `SUPER_DASH_DISTANCE` times the ordinary travel: the boost
            // is unchanged, the dash simply lasts longer.
            player.dash_active = dash_duration_from(player.dash_base_duration(), charge);
            player.dash_cd = player.dash_cooldown();
            player.iframes = player.iframes.max(player.dash_active + DASH_IFRAME_PAD);
            let pos = player.pos;
            let face = player.face;
            vfx::emit_dash_start(&mut particles, &mut rng.0, pos, face);
            sfx.0.push(Sfx::Dash);
            if charge > 0.0 {
                // The release kicks: the weapon's own `Sfx::SuperDash` lands on
                // the dash's first leg, and the shake scales with the meter
                // (the longer dash already draws the bigger streak).
                shake.amount = shake.amount.max(SUPER_DASH_SHAKE * (0.4 + 0.6 * charge));
            }
            // ---- w3-weapons-b hook 1/2: a *successful* dash just began ----
            // Only this branch (a live press or release with the cooldown
            // ready) reaches here, so a blocked or cooling dash buys no Super
            // Dash hit by construction.
            dash_track.begin();
            // How hard this dash was charged: the weapon's swept radius and
            // per-body damage scale with it (`weapons_wave3b`), and so does
            // Ripcord's cord, which outlives the dash.
            dash_track.charge = charge;
        } else if player.dash_denied <= 0.0 {
            // Asked for a dash that is not ready: never silent. The timer
            // throttles the feedback so a held (auto-repeating) key gives one
            // refused blip and one gauge shake per 0.16 s instead of
            // machine-gunning them.
            player.dash_denied = DASH_DENIED_SECS;
            sfx.0.push(Sfx::DashDenied);
        }
    }
    // ---- w3-weapons-b hook 2/2: record the segment this tick travels -------
    // `dashing` is the pre-decay flag, so the dash's last tick still counts and
    // the tick after it records nothing.
    let w3b_from = player.pos;

    let dashing = pot_hold.is_none() && player.dash_active > 0.0;
    let dash_boost = if dashing { DASH_BOOST } else { 1.0 };
    // A partial final tick travels only the remaining dash time.
    let move_dt = if dashing { dt.min(player.dash_active) } else { dt };
    // `move_dir` is a throttle vector: unit length = full speed (keyboard),
    // shorter = proportionally slower (mouse easing near the cursor).
    // A dash with no held direction launches along the facing instead of
    // spending the cooldown in place.
    let throttle = if dashing && move_dir.len_sq() <= 0.01 {
        player.face
    } else {
        move_dir.clamp_len(1.0)
    };
    // Console speed and the current size buff scale the throttle.
    let target_vel = throttle
        * stats.move_speed
        * player.speed_mult.max(0.0)
        * player.buffs.move_speed_scale()
        * dash_boost;
    // Copy out `vel` first (the legacy "disjoint borrow" convention) so the
    // arithmetic reads the pre-lerp velocity exactly once.
    let vel = player.vel;
    player.vel = if pot_hold.is_some() {
        Vec2::ZERO
    } else {
        vel + (target_vel - vel) * (12.0 * move_dt).min(1.0)
    };

    // Move in sub-steps shorter than the player radius. A dash at upgraded
    // speed can cover ~12 px per tick, which would otherwise skip straight
    // over a thin pipe and let nearest-face ejection pop the player out the
    // far side.
    let delta = player.vel * move_dt;
    let steps = ((delta.len() / PLAYER_SUBSTEP).ceil() as i32).clamp(1, 64);
    let step = delta * (1.0 / steps as f32);
    let arena = level.arena;
    let giant = player.buffs.is_giant();
    let (mut radius, mut class) = player.movement_body(&level.tiles);
    // `DashTrack::segs` is *this tick's* legs. Start it empty every tick: a
    // live dash tick does not clear it in `record`, so without this the legs of
    // every earlier tick of the same dash piled up (the weapon re-tested the
    // whole path so far each tick, and Ripcord's route was filled with
    // duplicates until its cap cut the cord short).
    dash_track.seg_len = 0;
    let move_steps = if pot_hold.is_some() { 0 } else { steps };
    for _ in 0..move_steps {
        // Where this sub-step started, so the dash records the legs that
        // really happened (a sub-step a wall pushes back on is no movement and
        // no leg).
        let leg_from = player.pos;
        // While leaving an occupied hole, check each leg so a dash can grow
        // clear of this opening without keeping its permission for the next.
        if giant && class == SizeClass::Small {
            (radius, class) = player.movement_body(&level.tiles);
        }
        let lo = Vec2::splat(radius);
        let hi = Vec2::new(arena.x - radius, arena.y - radius);
        player.pos += step;
        player.pos = player.pos.clamp_box(lo, hi);
        // One tile-grid query per sub-step: the cells around the rat, never
        // the level's walls. `Some(player.vel)` is the one-way-door
        // permission — it is the only entity that may push a gate leaf open,
        // every enemy passes `None`.
        let vel = player.vel;
        if !debug.noclip {
            player.pos = if giant && class == SizeClass::Large {
                level.tiles.resolve_circle_step(leg_from, player.pos, radius, class)
            } else {
                level.tiles.resolve_circle(player.pos, radius, class, Some(vel))
            };
        }
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
        vfx::emit_dash_trail(&mut particles, &mut rng.0, pos, vel, move_dt);
    }

    // Timers (legacy 233-240). Decay order matters: dash_cd, dash_active,
    // iframes, hurt_flash, then regen.
    player.dash_cd = (player.dash_cd - dt).max(0.0);
    player.dash_active = (player.dash_active - dt).max(0.0);
    player.iframes = (player.iframes - dt).max(0.0);
    if dashing && player.dash_active == 0.0 {
        // Hit systems see the post-move state. Keep the pad when a long frame
        // uses up the remaining dash time and would otherwise consume it too.
        player.iframes = player.iframes.max(DASH_IFRAME_PAD);
    }
    player.hurt_flash = (player.hurt_flash - dt).max(0.0);
    // Pure-visual dash feedback timers (no sim reads them, no rng involved).
    player.dash_ready_flash = (player.dash_ready_flash - dt).max(0.0);
    player.dash_denied = (player.dash_denied - dt).max(0.0);
    if was_cooling && player.dash_cd <= 0.0 {
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
    use super::{
        super_dash_charge_secs, DASH_DENIED_SECS, DASH_IFRAME_PAD, DASH_SECS, SUPER_DASH_CHARGE_SECS,
        SUPER_DASH_DISTANCE,
    };
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::constants::PLAYER_RADIUS;
    use crate::game::ecs::resources::{DashHold, SfxQueue};
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::particle::{Particles, Ramp, Shape};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;
    use crate::game::world::World;
    use crate::math::Vec2;

    const DT: f32 = 1.0 / 60.0;

    /// Give the rat the Super Dash weapon, level 1.
    fn own_super_dash(w: &mut World) {
        use crate::game::loadout::Upgrade;
        w.player
            .loadout
            .apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
    }

    fn run_player(w: &mut World) {
        w.ecs
            .run_system_cached(player_system)
            .expect("player system must run");
    }

    /// Strip the generated layout's walls so the test measures the movement
    /// system instead of the map (what `Level::walls.clear()` used to do
    /// before the tile grid owned collision).
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

    #[test]
    fn dash_travel_doubles_at_every_charge_and_immunity_covers_the_pad() {
        let east = Vec2::new(1.0, 0.0);
        for dt in [DT, 0.05] {
            for (charge, factor) in [(0.0, 2.0), (0.5, 3.0), (1.0, 4.0)] {
                let mut w = World::new(Theme::Sewer, 19);
                open_map(&mut w);
                if charge > 0.0 {
                    own_super_dash(&mut w);
                }
                let start = w.level.arena * 0.5;
                w.player.pos = start;
                let speed = w.player.loadout.stats.move_speed * 3.2;
                // Start at steady boost speed to measure the duration alone.
                w.player.vel = east * speed;
                let cooldown = w.player.loadout.stats.dash_cd;

                w.begin_tick_charged(dt, east, true, false, None, charge);
                run_player(&mut w);
                w.end_tick();
                assert!((w.player.dash_cd - (cooldown - dt)).abs() < 1e-5);
                let expected_iframes = factor * 0.16 + DASH_IFRAME_PAD - dt;
                assert!((w.player.iframes - expected_iframes).abs() < 1e-5);
                assert!(!w.player.take_damage(1.0));

                for _ in 0..40 {
                    if w.player.dash_active == 0.0 {
                        break;
                    }
                    w.begin_tick(dt, east, false);
                    run_player(&mut w);
                    w.end_tick();
                    assert!(
                        !w.player.take_damage(1.0),
                        "charge {charge} lost immunity mid-dash"
                    );
                }
                assert_eq!(w.player.dash_active, 0.0);
                let travel = w.player.pos.x - start.x;
                assert!(
                    (travel - factor * speed * 0.16).abs() < 0.02,
                    "charge {charge}: {travel} px"
                );
                assert!((w.player.vel.x - speed).abs() < 1e-5);
                assert!((w.player.iframes - 0.02).abs() < 1e-5);

                w.begin_tick(0.01, Vec2::ZERO, false);
                run_player(&mut w);
                w.end_tick();
                assert!(!w.player.take_damage(1.0), "the pad expired early");
                w.begin_tick(0.011, Vec2::ZERO, false);
                run_player(&mut w);
                w.end_tick();
                assert!(w.player.take_damage(1.0), "the pad failed to expire");
            }
        }
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

        // The same closed leaf stops a full charge, including at a long frame.
        own_super_dash(&mut w);
        w.player.pos = Vec2::new(centre.x + 36.0, centre.y);
        let west = Vec2::new(-1.0, 0.0);
        w.player.vel = west * (w.player.loadout.stats.move_speed * 3.2);
        let stop = gate.right() + PLAYER_RADIUS;
        for tick in 0..13 {
            if tick == 0 {
                w.begin_tick_charged(0.05, west, true, false, None, 1.0);
            } else {
                w.begin_tick(0.05, west, false);
            }
            run_player(&mut w);
            w.end_tick();
            assert!(
                w.player.pos.x >= stop - 0.01,
                "dash tunnelled through the closed leaf"
            );
        }
        assert_eq!(w.player.dash_active, 0.0);
        assert!(
            (w.player.pos.x - stop).abs() < 0.01,
            "dash did not stop at the leaf"
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

    /// `DashTrack::legs_now` is one tick's legs, not the dash so far: on the
    /// second tick of a dash the first recorded leg starts where the rat stood
    /// at the top of *that* tick, and the last one ends where it stands now.
    #[test]
    fn dash_legs_are_recorded_per_tick_not_accumulated() {
        use crate::game::ecs::resources::DashTrack;

        let mut w = World::new(Theme::Sewer, 47);
        open_map(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 0.0;

        let mut first_tick_legs = 0;
        for i in 0..4 {
            let start = w.player.pos;
            w.begin_tick(DT, Vec2::new(1.0, 0.0), i == 0);
            run_player(&mut w);
            w.end_tick();
            let track = w.ecs.resource::<DashTrack>();
            let legs = track.legs_now();
            assert!(!legs.is_empty(), "tick {i}: a moving dash recorded no leg");
            assert!(
                legs[0].0.dist(start) < 1e-3,
                "tick {i}: the first leg starts at {:?}, not at this tick's start {start:?}",
                legs[0].0
            );
            assert!(
                legs[legs.len() - 1].1.dist(w.player.pos) < 1e-3,
                "tick {i}: the last leg does not end at the rat"
            );
            if i == 0 {
                first_tick_legs = legs.len();
            } else {
                // The velocity only eases up to dash speed, so a later tick may
                // take a sub-step more than the first, never the running total.
                assert!(
                    legs.len() <= first_tick_legs + 2,
                    "tick {i}: {} legs after {first_tick_legs} on the first tick (accumulating)",
                    legs.len()
                );
            }
        }
    }

    /// A dash press the cooldown refuses, held until the dash is back, is not
    /// dropped: the charge starts on the tick the cooldown ends and the release
    /// fires a dash.
    #[test]
    fn a_hold_through_the_cooldown_is_buffered_and_fires_on_release() {
        let mut w = World::new(Theme::Sewer, 53);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 3.0 * DT;
        w.player.dash_active = 0.0;

        let step = |w: &mut World, hold: DashHold| {
            w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), false, false, hold);
            run_player(w);
            w.end_tick();
        };

        // The press lands while the dash is cooling: refused, no charge.
        step(&mut w, PRESS);
        assert!(!w.player.dash_charging, "a refused press started a charge");
        assert_eq!(w.player.dash_active, 0.0, "a refused press dashed");

        // Still held when the cooldown runs out: the charge starts.
        for _ in 0..6 {
            step(&mut w, HELD);
        }
        assert!(w.player.dash_charging, "the buffered hold never started a charge");
        assert_eq!(w.player.dash_active, 0.0, "the hold fired without a release");

        // The release fires the dash.
        step(&mut w, RELEASE);
        assert!(!w.player.dash_charging);
        assert!(w.player.dash_cd > 0.0, "the release did not dash");

        // A hold with no refused press behind it (begun in a menu, or before
        // the weapon was owned) never starts a charge on its own.
        let mut w = World::new(Theme::Sewer, 59);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 0.0;
        for _ in 0..6 {
            step(&mut w, HELD);
        }
        assert!(!w.player.dash_charging, "an unpressed hold started a charge");
        assert_eq!(w.player.dash_active, 0.0);
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

    // ---- Super Dash: hold to charge, release to fire ----------------------

    const HELD: DashHold = DashHold {
        down: false,
        held: true,
        up: false,
    };
    const PRESS: DashHold = DashHold {
        down: true,
        held: true,
        up: false,
    };
    const RELEASE: DashHold = DashHold {
        down: false,
        held: false,
        up: true,
    };

    /// A tap (released inside the tap window) is today's ordinary dash — and a
    /// press with the weapon owned never dashes on its own.
    #[test]
    fn a_quick_tap_with_super_dash_is_the_ordinary_dash() {
        let mut w = World::new(Theme::Sewer, 71);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 0.0;

        w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), false, false, PRESS);
        run_player(&mut w);
        w.end_tick();
        assert!(w.player.dash_charging, "the press must start a charge");
        assert_eq!(w.player.dash_active, 0.0, "the press must not dash");
        assert_eq!(w.player.dash_cd, 0.0, "the cooldown starts on the release");

        let before = w.player.pos.x;
        w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), false, false, RELEASE);
        run_player(&mut w);
        w.end_tick();
        assert!(!w.player.dash_charging);
        assert!(w.player.dash_active > DASH_SECS * 0.5, "a tap must dash");
        assert!(
            w.player.dash_active <= DASH_SECS + 1e-4,
            "a tap is not charged: {}",
            w.player.dash_active
        );
        assert!(w.player.dash_cd > 0.0, "the cooldown starts on the release");
        assert!(w.player.pos.x > before, "the tap dash must move the rat");
    }

    /// Holding fills the meter at the weapon's rate and the release fires a
    /// longer dash; holding itself grants nothing (no dash, no immunity).
    #[test]
    fn holding_charges_and_the_release_fires_a_longer_dash() {
        let mut w = World::new(Theme::Sewer, 73);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 0.0;

        w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), false, false, PRESS);
        run_player(&mut w);
        w.end_tick();
        let ticks = (SUPER_DASH_CHARGE_SECS / DT).ceil() as usize + 2;
        for _ in 0..ticks {
            w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), false, false, HELD);
            run_player(&mut w);
            w.end_tick();
            assert_eq!(w.player.dash_active, 0.0, "holding must not dash");
            assert_eq!(w.player.iframes, 0.0, "holding grants no immunity");
        }
        assert!(
            w.player.dash_charge >= 1.0,
            "the meter must fill: {}",
            w.player.dash_charge
        );
        assert!(w.player.dash_charging, "a full meter is held, not fired");

        w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), false, false, RELEASE);
        run_player(&mut w);
        w.end_tick();
        assert!(
            w.player.dash_active > DASH_SECS + 0.01,
            "a charged dash lasts longer: {}",
            w.player.dash_active
        );
        assert!(
            w.player.dash_active <= SUPER_DASH_DISTANCE * DASH_SECS + 1e-4,
            "the charged dash must not exceed the cap: {}",
            w.player.dash_active
        );
        assert!(
            w.player.iframes >= w.player.dash_active,
            "a charged dash is immune for the whole travel"
        );
    }

    /// A full meter can be held indefinitely: it caps at 1.0 and never fires on
    /// its own.
    #[test]
    fn a_full_charge_caps_and_never_auto_fires() {
        let mut w = World::new(Theme::Sewer, 75);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 0.0;
        w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, PRESS);
        run_player(&mut w);
        w.end_tick();
        for _ in 0..600 {
            w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, HELD);
            run_player(&mut w);
            w.end_tick();
        }
        assert_eq!(w.player.dash_charge, 1.0, "the meter caps at a full charge");
        assert_eq!(w.player.dash_active, 0.0, "a full charge must not auto-fire");
        assert!(w.player.dash_charging);
    }

    /// The fill rate is per weapon level: a higher level reaches a given meter
    /// sooner ("a little faster per level"), and the curve is monotone.
    #[test]
    fn a_higher_level_fills_the_meter_faster() {
        assert!((super_dash_charge_secs(1) - SUPER_DASH_CHARGE_SECS).abs() < 1e-6);
        for level in 2..=5u32 {
            assert!(
                super_dash_charge_secs(level) < super_dash_charge_secs(level - 1),
                "level {level} must fill faster than {}",
                level - 1
            );
        }
        assert!(super_dash_charge_secs(99) >= 0.5, "the rate has a floor");

        let meter_after = |level: u8| -> f32 {
            let mut w = World::new(Theme::Sewer, 79);
            open_map(&mut w);
            own_super_dash(&mut w);
            for weapon in w.player.loadout.weapons.iter_mut() {
                if weapon.kind == WeaponKind::SuperDash {
                    weapon.level = level;
                }
            }
            w.player.dash_cd = 0.0;
            w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, PRESS);
            run_player(&mut w);
            w.end_tick();
            for _ in 0..20 {
                w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, HELD);
                run_player(&mut w);
                w.end_tick();
            }
            w.player.dash_charge
        };
        assert!(
            meter_after(5) > meter_after(1),
            "L5 must fill faster than L1"
        );
    }

    /// A press while the cooldown is still running is the existing refused
    /// press: no charge starts, and the release that follows fires nothing.
    #[test]
    fn a_press_during_the_cooldown_is_refused_and_starts_no_charge() {
        let mut w = World::new(Theme::Sewer, 83);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 1.0;
        w.player.dash_denied = 0.0;

        w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, PRESS);
        run_player(&mut w);
        w.end_tick();
        assert!(!w.player.dash_charging, "a refused press must not charge");
        assert_eq!(w.player.dash_charge, 0.0);
        assert_eq!(w.player.dash_active, 0.0);
        assert!(w.player.dash_denied > 0.0, "the refusal must be felt");

        w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, RELEASE);
        run_player(&mut w);
        w.end_tick();
        assert_eq!(
            w.player.dash_active, 0.0,
            "a refused press must not dash on the release"
        );
        assert!(w.player.dash_cd > 0.0);
    }

    /// A hold dropped without a release edge (the terminal lost focus) cancels
    /// the charge: it is never fired.
    #[test]
    fn losing_the_hold_without_a_release_cancels_the_charge() {
        let mut w = World::new(Theme::Sewer, 87);
        open_map(&mut w);
        own_super_dash(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 0.0;
        w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, PRESS);
        run_player(&mut w);
        w.end_tick();
        for _ in 0..20 {
            w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, HELD);
            run_player(&mut w);
            w.end_tick();
        }
        assert!(w.player.dash_charge > 0.0);
        w.begin_tick_with_dash_hold(DT, Vec2::ZERO, false, false, DashHold::NONE);
        run_player(&mut w);
        w.end_tick();
        assert!(!w.player.dash_charging, "the charge is cancelled");
        assert_eq!(w.player.dash_charge, 0.0);
        assert_eq!(w.player.dash_active, 0.0, "a cancelled charge never fires");
    }

    /// A charged dash is still stopped by a wall: the longer route buys no
    /// phasing.
    #[test]
    fn a_charged_dash_is_stopped_by_a_wall() {
        let east = Vec2::new(1.0, 0.0);
        // Solid cells fill their tile. Maze spines are inset by 4 px.
        for (wall, inset) in [(WallKind::Solid, 0.0), (WallKind::Maze, 4.0)] {
            let mut w = World::new(Theme::Sewer, 89);
            open_map(&mut w);
            own_super_dash(&mut w);
            let (cx, cy) = (40, 40);
            w.level
                .tiles
                .fill_cells(cx + 2, cy - 4, 1, 9, TileKind::Wall(wall));
            w.level.tiles.rebuild_masks();
            w.player.pos = TileMap::tile_rect(cx, cy).center();
            w.player.vel = east * (w.player.loadout.stats.move_speed * 3.2);
            w.player.dash_cd = 0.0;
            let stop = TileMap::tile_rect(cx + 2, cy).x + inset - PLAYER_RADIUS;

            w.begin_tick_charged(0.05, east, true, false, None, 1.0);
            run_player(&mut w);
            w.end_tick();
            for _ in 0..12 {
                w.begin_tick(0.05, east, false);
                run_player(&mut w);
                w.end_tick();
                assert!(w.player.pos.x <= stop + 0.01, "dash tunnelled through the wall");
            }
            assert_eq!(w.player.dash_active, 0.0);
            assert!(
                (w.player.pos.x - stop).abs() < 0.01,
                "the charged dash did not stop at the wall face: {:?}",
                w.player.pos
            );
        }
    }

    /// Without the weapon there is no release latency: the press is the dash,
    /// exactly as before, and no charge state is ever written.
    #[test]
    fn without_the_weapon_the_press_dashes_immediately() {
        let mut w = World::new(Theme::Sewer, 91);
        open_map(&mut w);
        w.player.pos = w.level.arena * 0.5;
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.dash_cd = 0.0;
        w.begin_tick_with_dash_hold(DT, Vec2::new(1.0, 0.0), true, false, PRESS);
        run_player(&mut w);
        w.end_tick();
        assert!(w.player.dash_active > 0.0, "the press must dash at once");
        assert!(!w.player.dash_charging);
        assert_eq!(w.player.dash_charge, 0.0);
    }

    /// A scripted hold-and-release run is deterministic: two worlds fed the same
    /// ticks land on the same state, bit for bit.
    #[test]
    fn a_scripted_hold_and_release_is_deterministic() {
        let mut a = World::new(Theme::Sewer, 93);
        let mut b = World::new(Theme::Sewer, 93);
        for w in [&mut a, &mut b] {
            own_super_dash(w);
            w.player.pos = w.level.arena * 0.5;
            w.player.dash_cd = 0.0;
        }
        for i in 0..150u32 {
            let hold = match i {
                0 => PRESS,
                1..=40 => HELD,
                41 => RELEASE,
                _ => DashHold::NONE,
            };
            let mv = Vec2::new(((i % 3) as f32) - 1.0, 1.0).normalized();
            for w in [&mut a, &mut b] {
                w.update_with_dash_hold(DT, mv, false, false, None, hold);
            }
        }
        assert_eq!(a.elapsed.to_bits(), b.elapsed.to_bits());
        assert_eq!(a.player.pos.x.to_bits(), b.player.pos.x.to_bits());
        assert_eq!(a.player.pos.y.to_bits(), b.player.pos.y.to_bits());
        assert_eq!(a.player.hp.to_bits(), b.player.hp.to_bits());
        assert_eq!(a.player.dash_charge.to_bits(), b.player.dash_charge.to_bits());
        assert_eq!(a.kills, b.kills);
        assert_eq!(a.score, b.score);
        assert_eq!(a.enemies().len(), b.enemies().len());
    }
}
