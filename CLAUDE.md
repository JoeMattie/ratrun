# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Rat Run is a terminal **bullet-hell horde survivor** (Vampire-Survivors-style) written in Rust with `ratatui`. The player only moves; weapons auto-fire; kills drop XP that drives a 1-of-3 upgrade loop; survive a 5-minute escalating horde across three themed maps.

## Commands

```sh
cargo run --release      # play (needs a real TTY — see below)
cargo build              # debug build
cargo test               # full suite (unit + headless sim + TestBackend e2e)
cargo test <name>        # single test, e.g. cargo test headless_run_does_not_panic
cargo clippy             # lint (keep clean)
```

**Build prerequisite:** `rodio`/`cpal` link ALSA, so building requires the dev headers: `sudo apt-get install libasound2-dev` (the runtime `libasound.so.2` alone is not enough). Without them the crate won't compile.

**Screenshots:** `RATRUN_DUMP=play|story|levelup cargo test dump_frame_ppm` writes `/tmp/ratrun_<target>.ppm` (a rendered frame, each cell → fg-top/bg-bottom pixels). That test is a no-op unless `RATRUN_DUMP` is set.

## Running / testing without a TTY

`cargo run` needs an attached terminal; piped/headless environments make `enable_raw_mode` return `os error 6`, which the app handles by erroring out cleanly (no panic, terminal restored). To drive the **real binary** headlessly, allocate a PTY *with a window size set* (`script` does not set winsize → 0×0 → empty render); use Python's `pty` + `TIOCSWINSZ` (see the harness pattern used during development). For deterministic rendering tests, prefer ratatui's `TestBackend` instead (see `app.rs` tests).

## Architecture

Flow: `main.rs` (terminal lifecycle + 60 FPS loop) → `app.rs` (`App` state machine) → `game/world.rs` (`World`, the whole simulation).

- **`main.rs`** owns terminal setup/teardown (raw mode, alt screen, Kitty keyboard flags, panic-safe restore via a `KITTY_PUSHED` static + panic hook), polls crossterm events into `InputState`, computes `dt`, and calls `app.update(dt)` + `terminal.draw(app.render)`.
- **`app.rs`** is the only place that knows about screens (`Title / Story / MapIntro / Playing / LevelUp / Paused / End`). It drives the world during `Playing`, freezes it during overlays, owns menu/level-up selection state, finalizes scores, and bridges to audio.
- **`game/world.rs`** owns every entity in plain `Vec`s (no ECS) and a single `update(dt, move_dir, dash)` that runs the whole tick in order: player → director spawns → enemy steering → spatial-grid build → separation → weapon fire → bullets/pools → collisions → death reaping → gems/pickups → particles → shake. `draw(&mut PixelBuffer)` renders the world; the App composites HUD/menus around it.

### Rendering: half-block pixels (`render/framebuffer.rs`)

`PixelBuffer` is a `width × (height*2)` RGB grid. `render_to` blits it into a ratatui `Buffer` using the `▀` glyph per cell: **foreground = top pixel, background = bottom pixel**. This doubles vertical resolution and is what makes particles/sprites possible. The buffer is sized to the game viewport each frame; the camera follows the player clamped to the arena (plus a decaying shake offset). All world coordinates are in **world-pixel space** (`Vec2`, f32); `world.cam` + `w2s()` map to screen. HUD (`render/hud.rs`) and menus (`render/menu.rs`) are normal ratatui widgets drawn *over/around* the pixel viewport.

### Borrow-checker conventions in the tick

`World::update` reaches into many fields at once. The code relies on **disjoint field borrows**: it iterates with `for i in 0..vec.len()` (not iterators) and copies `Copy` values (e.g. `player.pos`, stats) into locals before mutating sibling fields, so `self.player.loadout.weapons[i]`, `self.enemies`, `self.bullets`, and `self.rng` can be touched in the same scope. Preserve this pattern — don't introduce `&mut self` helper methods inside these loops, or the borrows will conflict.

### Decoupled subsystems

- **Audio (`audio.rs`)** is fully procedural (no asset files) and optional: `AudioEngine::new()` returns `None` if there's no device, and everything no-ops silently. The sim never calls audio directly — `World` pushes `Sfx` enum values into `world.sfx`; the App **drains and de-duplicates them per frame** (so a 16-shot nova is one sound) and plays them, and sets music intensity. Music is a custom infinite `rodio::Source` (`MusicSynth`) with per-theme key/scale/pattern; an `Arc<AtomicU32>` intensity gates the lead + hat layers.
- **Spawning (`game/director.rs`)** is time-curve based, not fixed waves: `spawn_interval`/`pulse_count`/`hp_mult`/`pick_kind` all read `elapsed`; a boss spawns near the end.
- **Progression (`game/loadout.rs`)** holds owned weapons + passive `Stats`. On level-up the World sets `pending_levelups`; the App opens the `LevelUp` screen, calls `generate_choices` (1-of-3) and `apply`. Weapon damage/cooldown/projectile-count scale with level inside `game/weapon.rs`; firing patterns live in `World::fire_one`.
- **Input (`input.rs`)** tracks held movement via the Kitty keyboard protocol (real press/release) when available, else a short auto-repeat **grace window** fallback. Discrete presses (menus, dash, mute) come from a per-frame `pressed` list.
- **Lore (`lore.rs`)** is pure content (intro crawl, per-`Theme` map-intro + victory lines, defeat line); weapon/passive flavor strings live on their enums and surface on level-up cards.

### Where to add content

- New weapon: add to `WeaponKind` (`game/weapon.rs`) with stats/flavor, then a firing pattern in `World::fire_one`.
- New enemy: add to `EnemyKind` (`game/enemy.rs`) with per-kind stats + steering, and weight it into `director::pick_kind`.
- New upgrade/passive: extend `PassiveKind`/`Upgrade` in `game/loadout.rs`.
- New map/theme: add a `Theme` variant in `game/level.rs` (palette + walls) and content in `lore.rs`; `Theme::all()` drives the menu and music.

## Testing strategy

Three layers, all runnable without audio/TTY: pure-logic unit tests (math, collision, director, loadout, scores, audio synth), a **headless world sim** (`world.rs` `headless_run_does_not_panic` drives thousands of ticks across all themes at multiple viewport sizes to catch render/sim panics), and **`TestBackend` end-to-end** tests (`app.rs`) that drive the full App+render stack with simulated keystrokes. Tests construct `App::new(false, None)` to stay silent.
