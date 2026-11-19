# Rat Run → `bevy_ecs` port record

Short record of what the port is, what it deliberately does differently, and the
verification that it reproduces the Vec-based reference exactly.

## What was ported

The whole simulation moved out of `game/world.rs` into a `bevy_ecs` 0.20 world
(`src/game/ecs/`), with `game::world::World` kept as a source-compatible facade.

- **Components are coarse**: one component per legacy entity struct — `Enemy`,
  `Bullet`, `Gem`, `Pickup`, `Pool` — plus `SpawnId(u64)`, a monotonic
  spawn-order tag. Entities persist for the whole run (not respawned per tick),
  so `Entity`/`SpawnId` are stable and no query order is needed to reconstruct
  the old `Vec` semantics.
- **Singletons are resources**: `Player`, `Level`, `Director`, `Particles`,
  `DamageNumbers`, `FlowField`, `SpatialGrid`, `RngRes`, `SfxQueue`,
  `TickInput`, `Viewport`, `RunCounters`, `Shake`, `NextEnemyId`, `LastPickups`,
  plus permanent scratch (`NextSpawnId`, `NeighborScratch`, `PickupScratch`,
  `NukeTick`, `GridScratch`, `EnemyIndex`).
- **The tick is one explicitly chained, single-threaded schedule** (`schedule.rs`)
  mirroring the legacy `update()` order:

  `player → flow_refresh → director → steering → enemy_fire → grid_build →
  separation → wall_resolve → grid_refresh → fire_weapons → bullets → pools →
  player_bullet_hits → enemy_bullet_hits → contact → reap → gems → pickups →
  nuke_reap → particles → damage_numbers → shake → outcome`

  `chain()` also inserts `ApplyDeferred`, so `Commands` from one system are
  visible to the next; that is what replaced `Vec::push`. The two chained tuples
  exist only because `IntoScheduleConfigs` covers ≤ 20 entries. The
  `settle_schedule` (particles + damage numbers) is the legacy `finished()`
  path.
- **Rendering is unchanged**: `World::draw` keeps the legacy camera, culling,
  palette and draw order (floor → stains → props → pools → gems → pickups →
  enemies → bullets → player → particles → damage numbers), reading the ECS
  world through `SpawnId`-sorted queries.
- **Tests**: every legacy `world.rs` test was ported to `ecs/tests.rs`, each
  `systems/*.rs` file carries narrow single-system tests
  (`begin_tick` → `ecs.run_system_cached(system)` → `end_tick`), and
  `ecs/harness.rs` is a scripted seeded-run determinism regression with golden
  hashes taken from the reference.
- **Build**: `bevy_ecs = "0.20"` with `default-features = false`,
  `features = ["std"]` (no `multi_threaded`), and `rust-toolchain.toml` pinning
  1.100.0 in this worktree because bevy_ecs 0.20 requires rustc ≥ 1.95.

## Facade surface (`game::world::World`)

Fields stay real fields on the facade and are authoritative between ticks; each
`update` moves them into the ECS world (`begin_tick` + `swap_in`, using
pre-built spare twins so there is no per-tick allocation), runs the schedule,
then swaps them back (`end_tick` + `swap_out`).

| Surface | Users |
| --- | --- |
| `World::new(theme, seed)`, `update(dt, move_dir, dash)`, `draw(&mut PixelBuffer)`, `finished()`, `time_left()` | `app.rs`, hud tests |
| `begin_tick` / `end_tick` / `ecs` (narrow single-system tests) | `ecs/tests.rs`, `systems/*.rs` tests |
| `player` (+ `loadout`), `level`, `rng`, `sfx`, `pending_levelups`, `game_over`, `elapsed`, `kills`, `score`, `won`, `cam`, `viewport`, `director`, `particles`, `damage_numbers` | `app.rs`, `render/*`, app/hud tests |
| `boss_hp()`, `boss_offset()`, `enemy_count()`, `last_pickups()` | hud/overlay telemetry (merged from the feedback work) |
| `enemies()`, `bullets()`, `gems()`, `pickups()`, `pools()` — allocating `SpawnId`-ordered snapshots | tests + the determinism harness only |
| `spawn_enemy`/`spawn_bullet`/`spawn_gem`/`spawn_pickup`/`spawn_pool`, `clear_*`, `despawn_entity`, `spawn_point`, `nearest_enemy` | tests and setup only |

**No per-tick entity caches.** The port briefly kept `pub enemies/bullets/gems/
pickups/pools` `Vec`s rebuilt every tick so those fields kept working. Nothing
outside `src/game` read them (grep over `app.rs`, `render/`, `main.rs`,
`input.rs`, `audio.rs` finds no use), so they and `refresh_entity_cache` are
gone. `boss_hp`/`boss_offset`/`enemy_count` read the Bevy world directly through
`World::iter_entities` (`&self`, no allocation), which is what keeps the
`&World` call site in `app.rs`'s determinism test compiling.

## Sanctioned behavioural differences

1. **`SpawnId` order replaces the `swap_remove`-shuffled `Vec` order.** Every
   order-sensitive walk (nearest-enemy ties, per-enemy rng draws, drop order,
   bullet spawn order, draw order) now iterates in spawn order. After the first
   reap the old code's order was an artefact of `swap_remove`, so tie-breaking
   and the enemies that receive a given rng draw could differ from the pre-port
   build. Same seed still reproduces the same run, tick for tick.
2. **Carried Spore Orbit angle.** Re-firing the ring carries the oldest orbit
   bullet's angle forward (`SpawnId` order) and drops the previous ring through
   deferred `Commands` despawns, which the chain's `ApplyDeferred` applies
   before the next system — so the ring is still replaced within the same tick,
   exactly once.
3. **Reap and separation order.** `reap_enemies`/`nuke_reap` walk enemies in
   `SpawnId` order (no `swap_remove` shuffle), and separation runs the same
   in-place sequential nudges (earlier pushes visible to later neighbour reads)
   in `SpawnId` order instead of `Vec` order.
4. **Enemy cap counts queued spawns.** The director cannot see its own deferred
   `Commands`, so it counts live enemies **plus** the spawns it has already
   queued this tick. The legacy `self.enemies.len()` grew on each push, so the
   cap (`ENEMY_CAP`, one slot reserved for the boss) holds identically.

## Merge-hook audit (the neon combat-feedback merge)

The other team's work landed as commit 07e26c9 on `review-fixes-art-mouse`; its
`world.rs` changes were captured hunk-by-hunk in a diff and re-applied by hand
into the ECS systems. This is the completeness audit of that diff: **34 hunks,
34 present, 0 missing.** Every added/changed line of every hunk was located in
the ported code; the table below lists where. Line numbers are the current ones
in this tree.

| # | hunk (legacy function → change) | ECS home |
| --- | --- | --- |
| 1 | imports: `DamageNumbers`, `DamageSource`, `DAMAGE_NUMBER_CAP`, `vfx` | per-system imports — `combat.rs:121`, `loot.rs:53`, `projectiles.rs:69-70`, `weapons.rs:63-64`, `player.rs:59`, `spawn.rs:56`, `world.rs:61-62` |
| 2 | `World` fields `damage_numbers`, `next_enemy_id` | `world.rs:83,86` + resources `NextEnemyId` (`resources.rs:208`) |
| 3 | `World` field `last_pickups` | `world.rs:89` + resource `LastPickups` (`resources.rs:230`) |
| 4 | `World::new` init `last_pickups` | `world.rs:143` |
| 5 | `World::new` init `damage_numbers`, `next_enemy_id` | `world.rs:141-142` |
| 6 | accessors `boss_hp`, `boss_offset`, `enemy_count`, `last_pickups` | `world.rs:198,203,208,216` (query-backed, see below) |
| 7 | `update`: `damage_numbers.update(dt)` in tick + `finished()` path | `schedule.rs:61` `damage_numbers_system`, chained at `schedule.rs:103` and in `build_settle_schedule` (`schedule.rs:118`) |
| 8 | `update_player`: dash start via `vfx::emit_dash_start(…, face)` | `player.rs:88` |
| 9 | `update_player`: `vfx::emit_dash_trail` while dashing, after the sub-step move | `player.rs:133` |
| 10 | `run_director`: regular spawns get `e.id` from `next_enemy_id` | `spawn.rs:114` (+ `NextEnemyId::take`, `resources.rs:216`) |
| 11 | `run_director`: elite Brute gets `e.id` + `e.elite = true` | `spawn.rs:128,131` |
| 12 | `run_director`: boss gets `e.id` + `vfx::emit_boss_spawn` | `spawn.rs:141,146` (shake `14.0` assignment and `Sfx::Boss` at `147-148`) |
| 13 | `fire_one` Gnaw: `b.weapon = Some(kind)` + `emit_weapon_fire` | `weapons.rs:246,248` |
| 14 | `fire_one` Cheese Spray: same | `weapons.rs:262,265` |
| 15 | `fire_one` Squeak Nova: same | `weapons.rs:274,277` |
| 16 | `fire_one` Spore Orbit: bullet tagged | `weapons.rs:308` |
| 17 | `fire_one` Spore Orbit: `emit_weapon_fire(…, radius)` after the ring loop | `weapons.rs:316` |
| 18 | `fire_one` Tail Whip: `e.alive()` guard, `killed`, `emit_tail_whip_hit`, `damage_numbers.record(TailWhip)` | `weapons.rs:325,327,331,332-339`; pulse via `emit_weapon_fire` at `weapons.rs:342` |
| 19 | `fire_one` Acid Spit: bullet tagged + `emit_weapon_fire(…, to)` | `weapons.rs:358,367` |
| 20 | `fire_one`: per-weapon `Sfx` (`Shoot`…`ShootAcid`) behind `fired` | `weapons.rs:374-382` |
| 21 | `update_bullets`: orbit trail | `projectiles.rs:113` |
| 22 | `update_bullets`: lob landing → `emit_acid_splash`, else in-flight `emit_acid_drip` | `projectiles.rs:155,158` |
| 23 | `update_pools`: `emit_pool_bubbles`, `tick_dmg`, `!e.alive()` guard, `damage_numbers.record(Acid)` | `projectiles.rs:216,217,226,232` |
| 24 | `resolve_player_bullets`: `killed`, `emit_weapon_hit`, `record(from_weapon)` | `combat.rs:201,210` |
| 25 | `resolve_enemy_hits`: `emit_player_hurt` | `combat.rs:265` |
| 26 | `resolve_contact`: `emit_player_hurt` | `combat.rs:354` |
| 27 | `reap_enemies`: `emit_enemy_death`, elite accent, boss shake 16 + `Sfx::BossDeath`, big shake 6 + `Explosion` | `loot.rs:110,112,115,117,119` |
| 28 | `update_gems`: borrow split (`particles`/`rng` locals) | ECS resources (`ResMut<Particles>` + `ResMut<RngRes>`), no split needed — `loot.rs:216-257` |
| 29 | `update_gems`: `emit_gem_trail` on magnetized gems | `loot.rs:257` |
| 30 | `update_gems`: `emit_gem_collect` + `emit_level_up` | `loot.rs:272,277` |
| 31 | `update_pickups`: borrow split | as #28 — `loot.rs:289-305`, the `particles`/`rng`/`gems` resources |
| 32 | `update_pickups`: `emit_pickup_ambient`, `last_pickups.push`, heal/magnet VFX + `Sfx::Heal`/`Magnet`, boss Nuke `record`, `emit_nuke`, `Sfx::Nuke` | `loot.rs:328,333,340-341,349-350,374-383,387-388` |
| 33 | `draw`: `particles.draw_stains` after the floor | `world.rs:503` |
| 34 | `draw`: `damage_numbers.draw` after the particles | `world.rs:616` |
| — | the four tests added in `mod tests` (`world_accessors_…`, `enemy_hits_spawn_damage_numbers_…`, `weapons_emit_specialized_sfx`, `dump_vfx_showcase`) | `tests.rs:922,955,997,1038` |

Verification of the audit was mechanical as well as manual: every distinctive
token added by the diff (`vfx::emit_*`, `Sfx::*`, `DamageSource::*`,
`e.id`/`e.elite`, `b.weapon = Some(kind)`, `last_pickups`, `draw_stains`,
`damage_numbers.draw`) was grepped in `src/game/ecs/` and `src/game/world.rs`;
all are present. The order-sensitive ones were read against the legacy body
(e.g. pool bubbles use the post-decay `p.life`, the Whip hit runs
`e.hurt` → `killed` → vfx → `record`, the boss death shake replaced the old
`big` branch rather than adding to it).

## Behavioural equivalence against the reference (07e26c9)

Reference: commit 07e26c9 exported with `git archive` (no git state touched),
built in a scratch directory with the same harness. Because the port iterates
enemies in `SpawnId` (spawn) order where the reference's `Vec` is permuted by
`swap_remove`, the reference copy was patched with the single-line
`reap_enemies` change `self.enemies.swap_remove(i)` → `self.enemies.remove(i)`;
that makes its `Vec` order equal spawn order and changes nothing else. (The
patch is legitimate: PORT_PLAN §Sanctioned difference 1 declares that ordering
as the one sanctioned divergence.)

Cases: 3 themes × 3 seeds × 2 scripts (`wander`, `evade`) × god-mode on/off —
36 cases, 300 simulated seconds each (30 ticks past the win), a state line every
60 ticks with player pos/vel/hp/iframes/dash cooldown, level/xp, kills, score,
entity counts, particle count, damage-number count and two enemy hashes
(position hash and full per-enemy pos/hp/anim/vel hash).

**Result: byte-identical.** All 36 cases matched the patched reference exactly
(`cmp` of the two output files reports no difference), both before and after the
facade cache removal in §"No per-tick entity caches".

Reproduce (paths are the development scratchpad; the harness is now the trimmed
golden-hash test in `src/game/ecs/harness.rs`, the full comparison harness is
kept there as `harness_port_final.rs` / `harness_legacy07.rs`):

```sh
# reference (07e26c9 + swap_remove→remove patch), harness in src/game/harness.rs
RATRUN_HARNESS_MODE=compare RATRUN_HARNESS_OUT=/tmp/ref07.txt \
  cargo test --release behavior_harness -- --nocapture
# port (this tree, full harness copied into src/game/ecs/harness.rs)
RATRUN_HARNESS_MODE=compare RATRUN_HARNESS_OUT=/tmp/port.txt \
  cargo test --release behavior_harness -- --nocapture
cmp /tmp/ref07.txt /tmp/port.txt && echo identical
```

The goldens in `src/game/ecs/harness.rs` were generated by running that same
hashing code in the reference tree, so `cargo test harness` now re-checks the
equivalence without needing the second tree.

## `EnemyIndex` validity proof

`EnemyIndex` caches the `SpawnId`-ordered enemy list that the spatial grid's
bucket values index into (legacy `self.enemies[j]`). It is only valid while no
enemy has been spawned or despawned since it was built:

| built by | valid for |
| --- | --- |
| `enemy_move::grid_build_system` | `separation_system`, `wall_resolve_system`, `grid_refresh_system` |
| `enemy_move::grid_refresh_system` | `fire_weapons_system`, `bullets`/`pools`, `player_bullet_hits`, `enemy_bullet_hits`, `contact` — everything up to `reap_enemies_system` |

Between each build point and its readers, the chain only mutates components
(`Enemy` positions/velocity/cooldown, `Bullet`, `Pool`, `Gem`, `Pickup`), pushes
`Sfx`, and queues **non-enemy** spawns (bullets, pools, gems, pickups) plus
bullet/pool/gem despawns; the only enemy spawn is `spawn::director_system`
(before both builds) and the only enemy despawns are `reap_enemies_system` and
`nuke_reap_system` (after all readers). `ApplyDeferred` therefore never changes
the enemy set inside a validity window.

That reasoning is enforced, not just claimed: `resources::debug_assert_enemy_index`
compares the cache against a freshly sorted live `(SpawnId, Entity)` list
(contents, not just length — a despawn+spawn pair keeps the length) and is
called at the top of **every** reader: `separation_system` (`enemy_move.rs:260`),
`wall_resolve_system` (`:329`), `grid_refresh_system` (`:367`),
`fire_weapons_system` (`weapons.rs:105`), `player_bullet_hits_system`
(`combat.rs:161`), `contact_system` (`:337`, when it uses the index rather than
the narrow-test fallback) and `pools_system` (`projectiles.rs:197`). The guard
compiles to nothing in release (`#[cfg(debug_assertions)]`) and its two
`#[should_panic]` tests prove it actually rejects a reordering and a length
mismatch. `ecs/tests.rs::enemy_index_is_fresh_at_every_reader_for_a_full_run`
drives 18 100 ticks (301 s at 60 Hz, god mode, through the cap, elites and the
boss window) so the guard runs at every reader on every tick; the other
full-run tests (`headless_run_does_not_panic`,
`enemy_cap_holds_through_a_full_run`) run it too.

The guard was itself tested by mutation: inserting a throwaway system between
`grid_refresh_system` and `fire_weapons_system` that despawns one enemy makes
`enemy_index_is_fresh_at_every_reader_for_a_full_run` fail immediately with
`stale EnemyIndex: 2 entries but 1 live enemies`; with the real schedule it
passes. Holes found: none. One false lead is documented in `loot.rs`: the Nuke
pass looks like a natural `EnemyIndex` reader but runs *after* the reap, so it
sorts the live enemies itself (Nukes are rare).

## Facade cache removal (step 4) — before/after

Before: `end_tick` cloned every entity into five facade `Vec`s in `SpawnId`
order (`collect_sorted_into` with reused scratch), so `world.enemies` and
friends were valid between ticks.

| measurement (600 enemies, release) | before | after |
| --- | --- | --- |
| cache rebuild that step 4 removed (5 × the `collect_sorted_into` body: clone + `SpawnId` sort, reused scratch) | **20.7 µs/tick** (`cachecost`, the deleted code body) | 0 (no caches) |
| `end_tick` alone (resource swaps; previously swap + cache rebuild) | — (9.0 µs/tick observed on a loaded machine before the change) | **0.47 µs/tick** |
| `begin_tick` alone | 0.74 µs/tick (loaded) | 0.57 µs/tick |
| entity fields read outside `src/game` | none | none — no field had to be kept |

So the removed caches cost a measured 20.7 µs/tick of clone+sort at the enemy
cap (≈13 % of the 160 µs tick measured in the same conditions) and were pure
overhead for a facade nobody read. Snapshot accessors (`world.enemies()` etc.)
allocate only when a test or the harness asks for a whole list.

## Drawing cost at the cap (step 5)

`World::draw` at 600 live enemies, real camera (160×168 px viewport), 111
enemies visible, 600-draw minimum-of-chunks timings from the harness
`drawcost` mode (same body as `World::draw`, per-phase timers):

| phase | µs/draw |
| --- | --- |
| **total `World::draw`** | **385** |
| enemies (111 visible of 600) | 354 |
| floor + border | 16.6 |
| rat | 13.2 |
| `pb.clear` | 6.8 |
| query construction + sorted walks (5 queries) | 1.3 |
| props (culled) | 0.7 |
| bullets / pools / gems / pickups | 0.4–0.6 each |
| particles / damage numbers / floor stains | < 0.2 each |

So the draw is 92 % enemy art. In-situ per-kind cost of the visible mix
(Skitterer 38, Spitter 31, Brute 42 visible in that view):

| kind | base radius | ns/draw (in situ) | ns/draw (isolated) |
| --- | --- | --- | --- |
| Skitterer | 2.5 | 1 178 | 1 656 |
| Bat | 2.0 | — | 2 840 |
| Spitter | 3.0 | 2 079 | 3 060 |
| Cat | 4.0 | — | 7 195 |
| Brute | 6.5 | 5 702 | 10 879 |
| Boss | 14.0 | — | 66 420 |

Unculled worst case (all 600 drawn, no cull): **379 µs**, i.e. only ~25 µs more
than the culled path — the off-screen sprites early-out in
`art_enemy::Frame::bbox_pb` at ~50 ns each. The cull in `World::draw`
(`on_screen(s, e.radius * 3.0 + 8.0, viewport)`) is therefore worth ~6 % of the
frame, and it is already in place.

For `art_enemy.rs`'s owner, with numbers: cost is pixel-bound and grows
superlinearly with radius (Boss ≈ 66 µs vs Brute ≈ 11 µs for 2.15× the radius),
i.e. the per-pixel SDF rasterisation loops in `draw_brute`/`draw_boss`/
`draw_cat` dominate, not the health ring/flash colour passes (hp-ring and
flash variants are within noise of the full-health draw) and not allocations
(there are none). The 1 ms / 865 µs figures previously quoted came from an
unculled measurement that drew all 600 sprites on a loaded machine; the
consistent numbers are above (385 µs real view, 379 µs unculled worst case,
92 % of it enemy art).

No avoidable work was found in `World::draw` beyond the (already removed)
per-tick cache clones: off-screen entities are culled, the sorted walks are a
`pdqsort` over already-ascending `SpawnId`s plus a reused-by-`malloc` buffer
(1.3 µs for all five queries including construction), and nothing is cloned per
frame.

## Cleanup status

Done: the verbatim legacy test copy, the reference snapshots/diffs and the
line-number citations that pointed at them are deleted, so no doc comment
references a file that is not in the tree. The scratch comparison harness is
gone; `src/game/ecs/harness.rs` is a permanent `#[cfg(test)]` determinism
regression with baked-in golden hashes and no environment variables.
`cargo build`, `cargo test` (296 tests) and `cargo clippy --all-targets` are
clean. Nothing is committed.
