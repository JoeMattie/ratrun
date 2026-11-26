//! The single-threaded, explicitly ordered tick schedule.
//!
//! `bevy_ecs` is built with `default-features = false, features = ["std"]`, so
//! `multi_threaded` is off and the schedule runs on `SingleThreadedExecutor`.
//! Every system is chained, so the execution order is exactly the legacy
//! `World::update` order and no query iteration may choose an order that
//! affects the sim (components carry `SpawnId`; see [`super::components`]).
//!
//! Legacy `update()` order map:
//!
//! | # | system | legacy step |
//! |---|--------|-------------|
//! | -1 | `powerups::powerups_system` | wave-3 pocket-find timers (new) |
//! | 0 | `traps::traps_system` | floor traps (sticky boards, mousetraps); before the player |
//! | 0b | `hazards::hazards_system` | drip shaft, steam grates and ink pads; explicitly after 0 and before 1 |
//! | 0c | `floor_sewer::floor_sewer_slow_system` | the Sump Leech's tongue slow folded into this tick's throttle (Sewer); explicitly after 0b and before 1 |
//! | 1 | `player::player_system` | `update_player` |
//! | 1-outfall | `outfall::outfall_motion_system` | recover the rat, apply the surge and release Congers before the footsteps, fog and director |
//! | 1a | `miniboss_shapes::record_rat_track` | sample the post-move rat into the Wake/Drip history; no rng |
//! | 1b | `puddles::puddle_steps_system` | trench puddle expiry, wet-floor footsteps and ripples; explicitly after 1a and before 2 |
//! | 2 | `fog::reveal_system` | fog of war (minimap; new, writes only `Fog`) |
//! | 2a | `gates::{passage,cache,door}_system` | rat-holes, the maze cache, one-way doors (new); explicitly after 2 and before 3 |
//! | 2b | `shop::threshold_system` | the mouse-hole shops' thresholds (new); chained right after the doors |
//! | 3 | `enemy_move::flow_refresh_system` | flow refresh |
//! | 4 | `spawn::director_system` | `run_director` |
//! | 4 | `floor_sewer::sewer_opening_system` | fixed Sewer encounters; after the director and before 4a |
//! | 4-outfall | `floor_outfall::outfall_snuff_system` | resolve live fixtures before a named attack locks and before grid build |
//! | 4a | `miniboss::miniboss_system` | named miniboss appointments, retreats and signatures (wave 3); before 4b |
//! | 4b | `boss::boss_spawn_system` | boss hand-off (reads `BossDue`) |
//! | 4c | `boss::megarat::boss_summon_system` | a boss's `Call` spawns its rats here, outside the `EnemyIndex` window |
//! | 5 | `enemy_move::enemy_steering_system` | `update_enemies` steer |
//! | 6 | `enemy_move::enemy_fire_system` | `update_enemies` fire |
//! | 7 | `enemy_move::grid_build_system` | first grid build + `max_radius` |
//! | 8 | `enemy_move::separation_system` | `separate_enemies` |
//! | 9 | `enemy_move::wall_resolve_system` | `resolve_enemy_walls` |
//! | 10 | `enemy_move::grid_refresh_system` | second grid build |
//! | 11 | `weapons::fire_weapons_system` | `fire_weapons` |
//! | 11b | `weapons::active_weapons_system` | the two Space-triggered weapons (new) |
//! | 11b2 | `weapon_spiketail::spiked_tail_system` | the Spiked Tail's wind-up, crack and recoil (wave 3) |
//! | 11c | `weapons_wave3b::{laser_eyes,super_dash,melee_nudge}_system` | wave 3's self-driving weapons (new) |
//! | 11d | `weapons_wave3a::acid_rain_system` | Acid Rain's locked drops landing (wave 3) |
//! | 11d1 | `weapon_lava::lava_system` | Lava globs land and splash, pools merge and burn |
//! | 11d1a | `weapons_wave5::{glass_bead,plumb_bob,hold_point}_system` | ricochet, bob sweep and stillness pulse |
//! | 11d2 | `weapons_wave3a::droppings_system` | Hantavirus droppings age, expire and infect (`SpawnId` order; wave 3) |
//! | 11e | `weapons_wave3a::hantavirus_system` | Hantavirus ticks + third-tick jumps (wave 3) |
//! | 12 | `projectiles::bullets_system` | `update_bullets` |
//! | 13 | `projectiles::pools_system` | `update_pools` |
//! | 13b | `boss::boss_damage_system` | player damage → boss hit shapes |
//! | 13c | `boss::boss_update_system` | boss state machine + hazards |
//! | 13c-outfall | `boss::lantern::lantern_gape_system` | apply the current from the boss update to the rat and gems |
//! | 13d | `boss::cobra::coil_block_system` | the King Cobra's coils push a walking rat out of the body (a dashing one passes over); explicitly after 13c and before 14 |
//! | 14 | `combat::player_bullet_hits_system` | `resolve_player_bullets` |
//! | 15 | `combat::enemy_bullet_hits_system` | `resolve_enemy_hits` |
//! | 16 | `combat::contact_system` | `resolve_contact`; the last reader of `EnemyIndex` by index |
//! | 16a | `powerups::giant_stomp_system` | the Growth Serum's stomp disc (batch 13); right after contact (the head of 16b's nested chain) and before 16b-16e and the reap, so a stomp kill is seen by the death jump, reaped and looted on the same tick |
//! | 16b | `evolutions::{milk_teeth,fondue_tines,fondue_string}_system`, `fairy_ring_system`, `drain_cleaner_system`, `roll_call_system`, `dead_letter_system`, `ripcord_system` | evolved weapons (new), in this order |
//! | 16c | `enemy_wave3::wave3_system` | wave-3 kinds' tells, hazards, wick fuse (new); after every weapon damage pass so a moth any of them killed lights its wick |
//! | 16c2 | `enemy_wave4::wave4_system` | wave-4 kinds' tells and hazards (new); right after 16c |
//! | 16c3 | `floor_sewer::floor_sewer_system`, `floor_kitchen::floor_kitchen_system`, `floor_lab::floor_lab_system`, `floor_laundry::floor_laundry_system`, `floor_printworks::floor_printworks_system`, `floor_flatbed::floor_flatbed_system`, `floor_outfall::floor_outfall_system` | the per-floor roster rules (new), in this order; right after 16c2 |
//! | 16c3a | `drake::drake_system` | the guest's tell → peck → recover (Sewer only, a no-op without a Drake); in the same slot, right after `floor_sewer_system` |
//! | 16c4 | `approach::approach_sfx_system` | fodder approach sounds (new; writes only `SfxQueue`); right after 16c3 |
//! | 16d | `trailer::trailer_system` | the flatbed's holes, cargo and wind (new; `Theme::Trailer` only); a fall can kill, so it runs before 16e |
//! | 16d-outfall | `outfall::outfall_falls_system` | recover bodies and loose drops after grid readers and before death jumps and loot |
//! | 16e | `weapons_wave3a::hantavirus_death_system` | death-triggered jumps (sees evolution, wick-burst and fall kills), before the reap |
//! | 17 | `loot::reap_enemies_system` | `reap_enemies`; a named miniboss's chest + gold drop here (`miniboss::drop_loot`) |
//! | 18 | `loot::gems_system` | `update_gems` |
//! | 18a | `loot::gem_merge_system` | merge distant, unmagnetized gems after collection |
//! | 18b | `loot::treasure_system` | gold and chests |
//! | 19 | `loot::pickups_system` | `update_pickups` |
//! | 20 | `loot::nuke_reap_system` | the nested re-reap after a Nuke (same miniboss drop, so a nuked miniboss still leaves its chest) |
//! | 21 | `fx::particles_system` | `particles.update(dt)` |
//! | 22 | `damage_numbers_system` | `damage_numbers.update(dt)` (neon merge) |
//! | 23 | `fx::shake_system` | shake decay + jitter |
//! | 24 | `loot::outcome_system` | the win/loss block |
//!
//! `fog::reveal_system` is the only system with no legacy step: it reads the
//! player position and the tile grid, writes the `Fog` resource and nothing
//! else (no rng draw, no entity, no simulated value), so the golden traces are
//! identical with it enabled or disabled.
//!
//! `gates::passage_system`, `gates::cache_system` and `gates::door_system`
//! (the interactive-object runtime: rat-hole states and centring, the maze
//! cache heist, the one-way door leaves and the exit slam) are new in the same
//! sense. They are chained after `fog::reveal_system` and before
//! `enemy_move::flow_refresh_system` so they read the rat's post-move position
//! and write only `Level::objects`, `player.iframes`/`pending_levelups`,
//! `Fog::maze_claimed` and the `GateRuntime` scratch.
//! `shop::threshold_system` (the mouse-hole shops) is chained right after
//! `gates::door_system` in that same group: it reads the rat's post-move
//! position, draws no rng, and only raises `pending_shop` + the hole index.
//!
//! `elapsed += dt` is applied by `World::update` before `begin_tick`; the
//! per-tick `NukeTick` reset happens in `World::begin_tick`.
//!
//! The chain is split into several tuples because `IntoScheduleConfigs` is
//! implemented for tuples up to 20 elements. Each tuple is `.chain()`ed, so
//! Bevy inserts `ApplyDeferred` between consecutive systems whose predecessor
//! uses `Commands`. The later groups are explicitly anchored (`.after(...)` /
//! `.before(...)`); the last one is `.after(contact_system)`:
//! it starts with reaping, so the reaped→gem ordering is itself chained and
//! deferred spawns are visible to `gems_system`.
//!
//! [`build_settle_schedule`] is the legacy `finished()` early-return path:
//! only particles and damage numbers advance, nothing else in the sim runs.

use bevy_ecs::prelude::*;

use super::resources::TickInput;
use super::systems::{
    approach, boss, combat, drake, enemy_move, enemy_wave3, enemy_wave4, evolutions, floor_flatbed,
    floor_kitchen, floor_lab, floor_laundry, floor_printworks, floor_sewer, fog, fx, gates, hazards,
    loot, miniboss, player, powerups, projectiles, puddles, shop, spawn, trailer, traps,
    weapon_spiketail, weapons, weapons_wave3a, weapons_wave3b,
    weapon_lava, weapons_wave5, floor_outfall, outfall,
};

/// Legacy `self.damage_numbers.update(dt)` (neon merge, `World::update`).
///
/// Runs immediately after `fx::particles_system`, exactly where the legacy
/// `update` advanced the floating numbers next to the particles. It lives here
/// (rather than in `fx.rs`) because the schedule is the file that owns the
/// tick order; the body is a one-line delegation.
fn damage_numbers_system(
    input: Res<TickInput>,
    mut numbers: ResMut<crate::game::particle::damage_number::DamageNumbers>,
) {
    numbers.update(input.dt);
}

/// Build the full per-tick schedule (order documented above).
pub fn build_tick_schedule() -> Schedule {
    let mut schedule = Schedule::default();

    // 0: floor traps. First in the tick, because the sticky board's slow is
    // applied to this tick's `TickInput` throttle (before `player_system`
    // derives its movement from it) and to the enemies' `TrapSlow` component
    // (which `enemy_steering_system` multiplies in); damage dealt here also
    // lands before `loot::reap_enemies_system`, so trap kills take the normal
    // loot path. Trap state lives in `level.objects` (see `traps.rs`).
    schedule.add_systems(traps::traps_system.before(player::player_system));

    // -1: the wave-3 pocket finds' timers. Before the traps (Clean Socks must
    // already be aged when a glue board looks for a victim), before the player
    // (a Pocket Watch has to cover the whole tick) and before everything that
    // reads a buff. A run without one of these pickups returns immediately.
    schedule.add_systems(powerups::powerups_system.before(traps::traps_system));
    // 0b: the wave 3 floor hazards (steam grates, ink pads). Next to the traps
    // for the same reasons: the rat's ink slow is folded into this tick's
    // `TickInput` before `player_system` derives its movement, the pursuers'
    // `InkSlow` component is multiplied in by `enemy_steering_system`, and a
    // plume's damage lands before `reap_enemies`, so a scalded kill takes the
    // normal loot path.
    //
    // The whole pre-player order is therefore fixed: `powerups` (-1, so Clean
    // Socks are already aged) → `traps` (0) → `hazards` (0b) → the Sewer's
    // tongue slow (0c, added below) → `player` (1).
    // Relative to the contact→reap chain the hazards are simply the earliest
    // damage pass of the tick: a plume kill is a corpse through every weapon
    // pass (nothing targets it), lights a Wick Moth's fuse in
    // `enemy_wave3::wave3_system`, passes a Hantavirus host's jumps on in
    // `hantavirus_death_system`, and is reaped at row 17 like any other —
    // a named miniboss's chest included.
    schedule.add_systems(
        hazards::hazards_system
            .after(traps::traps_system)
            .before(player::player_system),
    );
    // 0c: the Sump Leech's tongue slow (Sewer). Same reason and same shape as
    // the ink pad: it shortens this tick's `TickInput` throttle, so it has to
    // sit after the hazards and before the player. It returns at once unless a
    // tongue has touched the rat (the `TongueSlow` resource does not exist
    // until then), draws no rng and touches no entity.
    schedule.add_systems(
        floor_sewer::floor_sewer_slow_system
            .after(hazards::hazards_system)
            .before(player::player_system),
    );

    // 1b: expire temporary puddles, then use this tick's rat position and
    // `dash_active` after the rat recorder for wet-floor footsteps. Before the fog reveal.
    schedule.add_systems(
        puddles::puddle_steps_system
            .after(super::systems::miniboss_shapes::record_rat_track)
            .before(fog::reveal_system),
    );

    // 1-11b: the player through the active weapons (the first tuple of the
    // legacy chain; contact damage is in the tuple after it).
    schedule.add_systems(
        (
            player::player_system,
            outfall::outfall_motion_system,
            super::systems::miniboss_shapes::record_rat_track,
            fog::reveal_system,
            enemy_move::flow_refresh_system,
            spawn::director_system,
            boss::boss_spawn_system,
            // 4c: the called rats. A boss may only add `Enemy` entities at the
            // head of the tick (its controller has no `Commands`, and the
            // update slot is inside the `EnemyIndex` window), so the summon a
            // `Call` queued last tick lands here — before `grid_build_system`
            // sees the field.
            boss::megarat::boss_summon_system,
            enemy_move::enemy_steering_system,
            enemy_move::enemy_fire_system,
            enemy_move::grid_build_system,
            enemy_move::separation_system,
            enemy_move::wall_resolve_system,
            enemy_move::grid_refresh_system,
            weapons::fire_weapons_system,
            // 11b: the player-triggered weapons. Right after the auto-fire pass
            // and before the projectile sweep, so it sees the same cached
            // `EnemyIndex` order (nothing spawned or reaped since
            // `grid_refresh`), its bullet deletions are applied before
            // `bullets_system` moves them, and a sweep's damage lands before
            // `player_bullet_hits`/`reap_enemies` in the same tick.
            weapons::active_weapons_system,
        )
            .chain(),
    );

    // 11d-16: wave 3's Acid Rain and Hantavirus, then the projectile sweep
    // through contact. A separate tuple only because `IntoScheduleConfigs` stops
    // at 20 entries; it is explicitly chained after `active_weapons_system`, so
    // the order is still the single chain documented above. The drops' splash
    // and the virus ticks land before `player_bullet_hits`/`reap_enemies`, and
    // both read the cached `EnemyIndex` order, which nothing has invalidated
    // since `grid_refresh`.
    //
    // 11d2: `droppings_system` sits immediately **before** the virus tick pass
    // it feeds. It ages and expires the rat's droppings (11e's `SpawnId` order
    // is the live one there: nothing has spawned or despawned an enemy since
    // `grid_refresh`), and the infection it plants through `Commands` is
    // applied by the `ApplyDeferred` that `chain()` inserts before
    // `hantavirus_system` — so a body that steps on a dropping on this tick is
    // ticked on the next, exactly as the old ranged tag's deferred insert was.
    schedule.add_systems(
        (
            weapons_wave3a::acid_rain_system,
            weapon_lava::lava_system,
            weapons_wave5::glass_bead_system,
            weapons_wave5::plumb_bob_system,
            weapons_wave5::hold_point_system,
            weapons_wave3a::droppings_system,
            weapons_wave3a::hantavirus_system,
            projectiles::bullets_system,
            projectiles::pools_system,
            boss::boss_damage_system,
            boss::boss_update_system,
            boss::lantern::lantern_gape_system,
            combat::player_bullet_hits_system,
            combat::enemy_bullet_hits_system,
            combat::contact_system,
        )
            .chain()
            .after(weapons::active_weapons_system),
    );

    // 11c: wave 3's self-driving weapons (Laser Eyes' burst, Super Dash's dash
    // hit) and Counterweight's nudge clock. Chained right after the active
    // weapons and before `bullets_system`: the cached `EnemyIndex` order is
    // still valid there (nothing spawned or reaped since `grid_refresh`), the
    // damage lands before `player_bullet_hits`/`reap_enemies` in the same tick,
    // and both systems read this tick's player position. Explicitly before
    // 11d (`acid_rain_system`, the head of the next tuple) so the order of the
    // wave-3 weapon passes is fixed: 3b's three systems, then rain, then virus.
    schedule.add_systems(
        (
            weapons_wave3b::laser_eyes_system,
            weapons_wave3b::super_dash_system,
            weapons_wave3b::melee_nudge_system,
        )
            .chain()
            .after(weapons::active_weapons_system)
            .before(weapons_wave3a::acid_rain_system),
    );

    // 11b2: the Spiked Tail's lash (wave 3, w3-spiketail). Chained right after
    // the auto-fire pass that spawns the lash and before `bullets_system`: the
    // cached `EnemyIndex` order is still the live one there (nothing spawned or
    // despawned since `grid_refresh`), so the crack's burst walks the same
    // order every other weapon does, its damage lands before
    // `player_bullet_hits`/`reap_enemies` in the same tick, and a spike shard a
    // crack breaks off is moved by `bullets_system` on the tick it was born.
    // An unevolved run with no lash in flight returns immediately (no rng).
    schedule.add_systems(
        weapon_spiketail::spiked_tail_system
            .after(weapons::active_weapons_system)
            .before(weapons_wave3b::laser_eyes_system),
    );

    // 4a: named miniboss appointments: spawn/retreat and the signature state
    // machine. It runs right after the director (so its spawn lands before the
    // grid build) and before the boss hand-off, in its own explicitly ordered
    // step because the first tuple is kept for the legacy chain.
    //
    // There is no separate miniboss system in the contact→reap chain: a
    // miniboss is an ordinary `Enemy` body, so every damage pass between
    // `contact_system` and `reap_enemies_system` (evolutions, the wave-3
    // enemies, the flatbed, the Hantavirus death jump) treats it as one, and
    // its chest is dropped by the reap itself (rows 17 and 20). That is what
    // makes the chest path-independent: whatever killed it, it is reaped.
    schedule.add_systems(
        (floor_sewer::sewer_opening_system, floor_outfall::outfall_snuff_system, miniboss::miniboss_system)
            .chain()
            .after(spawn::director_system)
            .before(boss::boss_spawn_system),
    );

    // The interactive-object runtime (gates.rs): rat-holes + centring, the
    // maze-cache heist, then the one-way doors and the exit slam. Chained
    // between the fog reveal and the flow refresh so all three see this tick's
    // player position and nothing downstream sees a stale cell.
    schedule.add_systems(
        (
            gates::passage_system,
            gates::cache_system,
            gates::door_system,
            // 2b: the mouse-hole shops' thresholds. Same slot and the same
            // reasons as the gates: it reads the rat's post-move position and
            // draws no rng (it only raises the pending flag + the hole index).
            shop::threshold_system,
        )
            .chain()
            .after(fog::reveal_system)
            .before(enemy_move::flow_refresh_system),
    );

    // 16b-24: the evolved weapons, reaping, loot and the outcome. Ordered
    // after contact; internally
    // chained so the Nuke re-reap and every deferred drop stay in order.
    //
    // `trailer::trailer_system` is the flatbed's hazard pass (holes, sliding
    // cargo, slipstream). It runs after the evolved weapons and before the
    // Hantavirus death jump and the reaper: fall damage lands
    // in the same tick as contact damage (so a fatal fall takes the normal
    // loot path), and it only writes positions/stun — never spawning or
    // despawning — so the `EnemyIndex` that `contact_system` just read stays
    // valid up to the reap. It early-outs on every other theme.
    //
    // `enemy_wave3::wave3_system` (row 16c) sits after the evolved weapons and
    // before the flatbed pass: every *weapon* damage source has run (bullets,
    // pools, melee, actives, traps, boss hazards and the eight evolutions), so
    // a Wick Moth any of them killed lights its fuse before the reap takes the
    // corpse, a Wick burst resolves into this tick's reaping and loot path,
    // and every warn it starts is visible for a full tick before it can bite.
    // It spawns and despawns only `Wick`s, never an enemy, so the
    // `EnemyIndex` window is untouched. The flatbed pass follows it so a Bell
    // Flea that lands on a hole falls in the same tick (Wick Moths fly and
    // never fall, so no fall can orphan a wick), and the Hantavirus death
    // jump is last so it sees burst and fall kills too.
    schedule.add_systems(
        (
            // 16b: evolved weapons (`WAVE3_DESIGN.md` §3). They damage bodies, so
            // they run after contact and before the reap, which is also the
            // window where `EnemyIndex` is still the live `SpawnId` order (no
            // enemy has spawned or despawned since the grid build). An
            // unevolved run walks empty queries here and draws no rng.
            // The three Fondue/Milk systems are one nested chain so the outer
            // tuple stays inside `IntoScheduleConfigs`' 20-element limit: the
            // tines fly before their string is ticked, so a cast that settles
            // this tick already has a string this tick.
            // 16a: Growth Serum's stomp (batch 13) heads the nested chain: right
            // after `contact_system` (a body touching the giant is being
            // stomped, so the two passes agree on the tick) and before every
            // 16b-16e system and the reap, so a crushed body is seen by the
            // Hantavirus death jump, reaped and looted this tick. It spawns and
            // despawns nothing and draws no rng, so the `EnemyIndex` window is
            // untouched; a run without the find returns on its first line.
            (
                powerups::giant_stomp_system,
                evolutions::milk_teeth_system,
                evolutions::fondue_tines_system,
                evolutions::fondue_string_system,
            )
                .chain(),
            evolutions::fairy_ring_system,
            evolutions::drain_cleaner_system,
            evolutions::roll_call_system,
            // Dead Letter's sealed packets (2 s fuse, opens on the first
            // eligible foe) and Ripcord's taut dash route. Both read the same
            // live `EnemyIndex` order and are before `hantavirus_death_system`,
            // so a packet opened this tick can hand its chain on this tick.
            evolutions::dead_letter_system,
            evolutions::ripcord_system,
            // 16c: the wave-3 enemies' behaviour tick (see the comment above
            // this tuple).
            enemy_wave3::wave3_system,
            // 16c2: the wave-4 kinds' behaviour tick. After wave 3 so its
            // warned-attack limiter counts every tell that started this tick;
            // before the flatbed pass so a horde that splits (the one place a
            // system in this window spawns an enemy) is placed by it in the
            // same tick, and before the reap so a lunge, jab or split kill takes
            // the normal loot path. `EnemyIndex` is already stale here, which
            // is legal: nothing after `contact` reads it.
            enemy_wave4::wave4_system,
            // 16c3: the per-floor roster rules, one system per floor, in route
            // order. Each returns before its sort on a floor that has none of
            // its kinds. They take `Commands` only to attach a state component
            // (or, the Sewer, one resource) to a body on its first tick — never
            // to spawn or despawn an enemy — and none reads `EnemyIndex`. After
            // wave 3 and 4 so a wick `wave3_system` lit this tick exists for the
            // Printworks' slide; before the flatbed pass so a body they moved
            // can still fall this tick, and before the reap so a hit they land
            // on the rat is this tick's. One nested chain: the outer tuple is
            // at `IntoScheduleConfigs`' 20-entry limit.
            (
                floor_sewer::floor_sewer_system,
                // 16c3a: the guest (the Drake). Its own narrow system in the
                // same slot: it returns on its first line without a Drake on
                // the floor (no sort, no rng), it never spawns or despawns
                // (it only attaches its state component to a body that reached
                // the floor another way), and it draws no rng of its own. Its
                // body is an ordinary `Enemy`, so every damage pass above has
                // already seen it, and the peck's hit on the rat lands before
                // the reap like every other floor strike.
                drake::drake_system,
                floor_kitchen::floor_kitchen_system,
                floor_lab::floor_lab_system,
                floor_laundry::floor_laundry_system,
                floor_printworks::floor_printworks_system,
                floor_flatbed::floor_flatbed_system,
                floor_outfall::floor_outfall_system,
                // 16c4: the fodder approach sounds. Reads the bodies where
                // they stand after every behaviour pass; writes only the
                // `SfxQueue` (no rng, no entity, no simulated value).
                approach::approach_sfx_system,
            )
                .chain(),
            // 16d: the flatbed's hazard pass (see the comment above this
            // tuple). A pit fall takes 35 % of an enemy's max HP and can kill,
            // so it is a damage pass and must precede the Hantavirus death
            // jump below; it never spawns or despawns.
            (trailer::trailer_system, outfall::outfall_falls_system).chain(),
            // 16e: a Hantavirus host that died this tick (any source, the
            // evolved weapons, a Wick burst and a flatbed fall above included) passes
            // its remaining jumps on while the corpse and the `EnemyIndex`
            // order still exist — i.e. after every damage pass and before the
            // reap despawns it.
            weapons_wave3a::hantavirus_death_system,
            loot::reap_enemies_system,
            // Keep collection and consolidation adjacent without adding a
            // 21st entry to the outer chain.
            (loot::gems_system, loot::gem_merge_system).chain(),
            loot::treasure_system,
            loot::pickups_system,
            loot::nuke_reap_system,
            fx::particles_system,
            damage_numbers_system,
            fx::shake_system,
            loot::outcome_system,
        )
            .chain()
            .after(combat::contact_system),
    );

    // 13d: the King Cobra's laid body is a **moving wall**. A boss controller
    // has no player access (`BossContext` hands it a `&Level` and a
    // `&mut StdRng` and nothing else) and the coil must not be a tile edit —
    // the map pin is a golden — so the separation pass lives here, right after
    // the update slot that laid the loop this tick and before the damage
    // passes. A walking rat is pushed out of the nearest body point, a dashing
    // one passes over it; a floor with no cobra on it (or a cobra that is not
    // coiled) returns before touching anything, so every other fight is
    // byte-identical. It spawns and despawns nothing, so the `EnemyIndex`
    // window is untouched.
    schedule.add_systems(
        boss::cobra::coil_block_system
            .after(boss::lantern::lantern_gape_system)
            .before(combat::player_bullet_hits_system),
    );

    schedule
}

/// Legacy `finished()` early-return: `self.particles.update(dt)` plus the
/// damage-number settle the neon merge added next to it.
pub fn build_settle_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems((fx::particles_system, damage_numbers_system).chain());
    schedule
}
