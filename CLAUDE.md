# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Rat Run is a terminal **bullet-hell horde survivor** (Vampire-Survivors-style) written in Rust with `ratatui`. The player only moves; weapons auto-fire; kills drop XP that drives a 1-of-3 upgrade loop; survive a 5-minute escalating horde across three themed maps.

## Commands

```sh
cargo run --release      # play (needs a real TTY — see below)
cargo build              # debug build
cargo test               # full suite (unit + per-system + headless sim + TestBackend e2e)
cargo test <name>        # single test, e.g. cargo test headless_run_does_not_panic
cargo clippy --all-targets   # lint (keep clean)
```

**Toolchain:** `rust-toolchain.toml` pins `1.100.0` for this repo — `bevy_ecs` 0.20 needs rustc ≥ 1.95 and the machine default is 1.94. rustup installs it on demand; don't touch the global default.

**Build prerequisite:** `rodio`/`cpal` link ALSA, so building requires the dev headers: `sudo apt-get install libasound2-dev` (the runtime `libasound.so.2` alone is not enough). Without them the crate won't compile.

**Screenshots:** `RATRUN_DUMP=play|story|levelup cargo test dump_frame_ppm` writes `/tmp/ratrun_<target>.ppm` (a rendered frame, each cell → fg-top/bg-bottom pixels). That test is a no-op unless `RATRUN_DUMP` is set.

## Running / testing without a TTY

`cargo run` needs an attached terminal; piped/headless environments make `enable_raw_mode` return `os error 6`, which the app handles by erroring out cleanly (no panic, terminal restored). To drive the **real binary** headlessly, allocate a PTY *with a window size set* (`script` does not set winsize → 0×0 → empty render); use Python's `pty` + `TIOCSWINSZ` (see the harness pattern used during development). For deterministic rendering tests, prefer ratatui's `TestBackend` instead (see `app.rs` tests).

## Architecture

Flow: `main.rs` (terminal lifecycle + 60 FPS loop) → `app.rs` (`App` state machine) → `game/world.rs` (the `World` facade) → `game/ecs/` (a `bevy_ecs` world + one ordered schedule) → `render/` (half-block pixels, HUD/menus).

**Levels are procedural**: a 200 × 160 cell (16 px) tile grid (`game/collision.rs::TileMap`) generated from `(theme, run seed)` by `game/levelgen/`, with `game/nav.rs` flow fields over it and `game/level.rs` as the data model. `CONTENT_PLAN.md` is the contract for that subsystem (data model, generator stages, navigation, tile collision, rendering, spawn pacing, file ownership) — read it before touching levels, walls, mazes, traps or pathfinding.

- **`main.rs`** owns terminal setup/teardown (raw mode, alt screen, Kitty keyboard flags, focus reporting, restore via a drop guard + `KITTY_PUSHED` static + panic hook), reads crossterm events on a joined input thread into `InputState`, wraps each frame in a synchronized update (DEC 2026), computes `dt`, and calls `app.update(dt)` + `terminal.draw(app.render)`.
- **`app.rs`** is the only place that knows about screens (`Title / Story / MapIntro / Playing / LevelUp / Paused / End`). It drives the world during `Playing`, freezes it during overlays, owns menu/level-up selection state, finalizes scores, and bridges to audio.
- **`game/world.rs`** is the facade and the only type outside code names: it keeps the legacy public fields (`player`, `level`, `elapsed`, `kills`, `score`, `pending_levelups`, `game_over`, `won`, `sfx`, `rng`, `cam`, `viewport`, `particles`, `damage_numbers`, …) plus read-only HUD accessors (`boss_hp`, `boss_offset`, `enemy_count`, `last_pickups`) and allocating `SpawnId`-ordered snapshots for tests (`enemies()`, `bullets()`, …), drives one tick, and keeps the camera/culling/draw order for `draw(&mut PixelBuffer)`.
- **`game/ecs/`** is the simulation: `components.rs`, `resources.rs`, `schedule.rs`, `systems/` (one file per concern), `tests.rs`, plus `harness.rs` (a golden-hash determinism regression).

### The facade and the resource round-trip (`game/ecs/resources.rs`)

Singleton state is authoritative on `World` *between* ticks and in the ECS world *during* one:

1. `update` calls `begin_tick`, which inserts the per-tick resources (`TickInput`, `Viewport`, `NukeTick`, `RunCounters`, `Shake`, `SfxQueue`, `NextEnemyId`, `LastPickups`) and moves `rng` (via the `RngRes` newtype), `level`, `player`, `particles`, `damage_numbers`, `director`, `flow` into the ECS world with `swap_in`. Each type has a pre-built spare twin from `World::new`, so the round-trip allocates nothing.
2. `schedule.run(&mut world.ecs)` — one thread, one chain.
3. `end_tick` swaps the resources back with `swap_out`. Nothing is copied out of the ECS world per tick: entity components live there for the whole run, the HUD accessors read them directly, and `World::enemies()`/`bullets()`/`gems()`/`pickups()`/`pools()` clone a `SpawnId`-ordered snapshot only when a test or the determinism harness asks for a whole list.

So writes made between ticks (`world.player.hp = …`, `world.pending_levelups = 1`, app tests) reach the next tick and reads right after `update()` see its results. `begin_tick`/`end_tick` are public — that is the per-system harness. Never touch facade singletons while a tick is open; use `world.ecs.resource_mut::<T>()`. `boss_hp()`/`boss_offset()`/`enemy_count()` take `&self` and walk the ECS world with `iter_entities`, so they allocate nothing and work for the `&World` call sites in `app.rs`.

### Components, resources, systems

- Components are **coarse**: each legacy entity struct stays one component (`Enemy`, `Bullet`, `Gem`, `Pickup`, `Pool`) plus `SpawnId(u64)`, a monotonic spawn-order tag. Do not split them into Position/Health — art and behaviour take `&Enemy` and friends. Entities live in the ECS world for a whole run, so `Entity`/`SpawnId` are stable.
- Resources are the singletons above plus permanent scratch (`NextSpawnId`, `NextEnemyId`, `NeighborScratch`, `PickupScratch`, `NukeTick`, `GridScratch`, `EnemyIndex`, `SpatialGrid`). `EnemyIndex` caches the `SpawnId`-ordered enemy list that the spatial grid's bucket indices address (legacy `self.enemies[j]`); it is only valid while no enemy has spawned or despawned since the grid was built, which is why every reader calls `resources::debug_assert_enemy_index` in debug builds (see *Determinism rules*). `Enemy::id` (damage-number identity) comes from `NextEnemyId`, never from a `Vec` index. Systems push `Sfx` into `SfxQueue` where the old code pushed into `self.sfx`; there is no audio system.
- Systems live in `game/ecs/systems/{player,spawn,enemy_move,weapons,projectiles,combat,loot,fx}.rs`; they spawn with `commands.spawn((component, next_id.take()))` and despawn with `commands.entity(e).despawn()`. The `World::spawn_*`/`clear_*` helpers are for tests and setup — systems must not call them.

### The tick schedule — order is behaviour (`game/ecs/schedule.rs`)

ONE `Schedule`, every system `.chain()`ed, always single-threaded (`multi_threaded` is off in `Cargo.toml`):

`player → flow_refresh → director → enemy_steering → enemy_fire → grid_build → separation → wall_resolve → grid_refresh → fire_weapons → bullets → pools → player_bullet_hits → enemy_bullet_hits → contact → reap_enemies → gems → pickups → nuke_reap → particles → damage_numbers → shake → outcome`

- `chain()` makes Bevy run `ApplyDeferred` between consecutive systems whenever the earlier one took `Commands`, so entities spawned by one system (director enemies, weapon projectiles, death drops) exist for the next — that replaces the old `Vec::push` visibility.
- Load-bearing adjacencies: `reap_enemies` right before `gems` (drops collect the same tick), `pickups` right before `nuke_reap` (a Nuke re-reaps the same tick), `outcome` last (loss beats win).
- The list is two chained tuples only because `IntoScheduleConfigs` covers ≤ 20 entries; the second is `.after(combat::contact_system)`. Still one ordered chain — no parallel ordering, and keep the order map in the module doc in sync.
- `build_settle_schedule()` is the legacy `finished()` path: particles and damage numbers only. `damage_numbers_system` lives in `schedule.rs` (not `fx.rs`) because that file owns the tick order.

### Determinism rules

Single-threaded, and query iteration order must never decide anything. Whenever the old code relied on "first / next / in-`Vec`-order", iterate sorted by `SpawnId`:

```rust
for (id, mut e) in enemies.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) { /* legacy order */ }
```

That covers nearest-enemy ties, separation nudges, per-enemy rng draws, drop order, bullet spawn order and draw order. The number *and* order of rng draws are part of the spec — draw only where the old code drew (e.g. `fx::shake_system` jitters only when `shake > 0.05`). Same seed + same inputs must reproduce the same run.

**Cached order (`EnemyIndex`).** `enemy_move::grid_build_system` and `grid_refresh_system` build `EnemyIndex` once per tick in `SpawnId` order, together with the `SpatialGrid` whose bucket values index it. It may be read instead of sorting **only** while no enemy has spawned or despawned since the build: usable by `separation`, `wall_resolve`, `grid_refresh`, then (after the refresh) `fire_weapons`, `bullets`/`pools`, `player_bullet_hits`, `enemy_bullet_hits` and `contact` — nothing up to `reap_enemies_system`. It is stale in `enemy_steering`/`enemy_fire` (the director may already have queued spawns) and after `reap_enemies`/`nuke_reap`/`clear_enemies`. Every reader calls `resources::debug_assert_enemy_index` (compiled out in release), which compares the cache with a fresh `SpawnId` sort *including contents*, so `cargo test` verifies the window on every tick of a full run.

### Rendering: half-block pixels (`render/framebuffer.rs`)

`PixelBuffer` is a `width × (height*2)` RGB grid. `render_to` blits it into a ratatui `Buffer` using the `▀` glyph per cell: **foreground = top pixel, background = bottom pixel**. This doubles vertical resolution and is what makes particles/sprites possible. The buffer is sized to the game viewport each frame; the camera follows the player clamped to the arena (plus a decaying shake offset). All world coordinates are in **world-pixel space** (`Vec2`, f32); `world.cam` + `w2s()` map to screen.

Sprites live in `game/art_floor.rs` (floor + arena border), `art_props.rs` (themed obstacles), `art_enemy.rs` (bugs, elites, health rings, boss arc) and `art_rat.rs` (player). `World::draw` culls props, pools, gems, pickups, bullets and enemies with `on_screen` before calling the art, and iterates entities in `SpawnId` order. At the 600-enemy cap a 160×168-px view costs ~385 µs, of which ~92 % is visible enemy art (~25 µs more if every enemy were drawn unculled: off-screen sprites early-out inside `art_enemy`); see `PORT_PLAN.md` for the measured breakdown. `game/particle.rs` owns the capped ring buffers (particles, shockwaves, floor stains) and hosts `particle::vfx` (feedback emitters the systems call instead of inlining bursts) and `particle::damage_number` (aggregating floating damage digits). Draw order: floor → floor stains → props → pools → gems → pickups → enemies → bullets → player → particles → damage numbers. HUD (`render/hud.rs`), viewport overlays (`render/overlay.rs`: boss bar/warning/compass, low-health cue, callouts, from `HudState`) and menus (`render/menu.rs`) are ratatui widgets drawn *over/around* the pixel viewport.

### Decoupled subsystems

- **Audio (`audio.rs`)** is fully procedural (no asset files) and optional: `AudioEngine::new()` returns `None` if there's no device, and everything no-ops silently. The sim never calls audio directly — systems push `Sfx` variants (one per weapon family, kill, boss, pickup, …) into `SfxQueue`; the App **drains and de-duplicates them per frame** (so a 16-shot nova is one sound), plays them, and sets music intensity. Music is a custom infinite `rodio::Source` (`MusicSynth`) with per-theme key/scale/pattern; an `Arc<AtomicU32>` intensity gates the lead + hat layers.
- **Spawning (`game/director.rs` + `systems/spawn.rs`)** is time-curve based, not fixed waves: `spawn_interval`/`pulse_count`/`hp_mult`/`pick_kind` read `elapsed`; elites and the near-end boss come from the same system under `ENEMY_CAP` (one slot reserved for the boss).
- **Progression (`game/loadout.rs`)** holds owned weapons + passive `Stats`. On level-up `gems_system` sets `pending_levelups`; the App opens the `LevelUp` screen, calls `generate_choices` (1-of-3) and `apply`. Weapon damage/cooldown/projectile-count scale with level inside `game/weapon.rs`; firing patterns live in `systems/weapons.rs::fire_one`.
- **Input (`input.rs`)** tracks held movement via the Kitty keyboard protocol (real press/release; Ghostty is the primary target terminal and implements it, as does Kitty) when available, else a short auto-repeat **grace window** fallback. Discrete presses (menus, dash, mute) come from a per-frame `pressed` list.
- **Lore (`lore.rs`)** is pure content (intro crawl, per-`Theme` map-intro + victory lines, defeat line); weapon/passive flavor strings live on their enums and surface on level-up cards.

### Where to add content

- New weapon: add to `WeaponKind` (`game/weapon.rs`) with stats/flavor, then a firing pattern in `systems/weapons.rs::fire_one` (tag the bullet's `weapon`, add `vfx::emit_weapon_fire`/`emit_weapon_hit` feedback and its `Sfx`).
- New enemy: add to `EnemyKind` (`game/enemy.rs`) with per-kind stats + `steer`, weight it into `director::pick_kind`, draw it in `game/art_enemy.rs`, hook its death feedback in `particle::vfx`.
- New system: add `fn x_system(…)` to the matching file in `game/ecs/systems/`, chain it in `game/ecs/schedule.rs` at the exact slot the old code ran in, and add a narrow test in that file.
- New resource: define it in `game/ecs/resources.rs`; insert it permanently in `World::new` (scratch), or — if the facade exposes it — give it a spare twin and round-trip it in `begin_tick`/`end_tick`.
- New upgrade/passive: extend `PassiveKind`/`Upgrade` in `game/loadout.rs`.
- New map/theme: add a `Theme` variant in `game/level.rs` (palette + walls) and content in `lore.rs`; `Theme::all()` drives the menu and music.

## Testing strategy

Four layers, all runnable without audio/TTY: pure-logic unit tests (math, collision, director, loadout, scores, audio synth, damage numbers); **per-system narrow tests** in each `systems/*.rs`, driving one system through `begin_tick` → `ecs.run_system_cached(…)` → `end_tick` and asserting on facade caches; the **ported suite** in `game/ecs/tests.rs` (thousands of headless ticks across themes/viewports, plus spawn/wall/cap/nuke/tie-order regressions); and **`TestBackend` end-to-end** tests (`app.rs`, `render/*`) driving the full App + render stack with simulated keystrokes. Construct `App::new(false, None)` / `World::new(theme, seed)` to stay silent and deterministic. `ecs/harness.rs` is the determinism regression: six scripted seeded games (both scripts, all three themes) whose full state trace is hashed against goldens generated from the pre-port reference, so any change in numbers, rng draws or ordering fails `cargo test`.

The default profile is `[profile.dev] opt-level = 1`, so `cargo test` (no `--release`) is fast enough for the full-run tests.
