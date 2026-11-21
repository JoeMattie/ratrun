# Rat Run 🐀

A visually rich, **bullet-hell horde survivor** for your terminal — built in Rust with
[`ratatui`](https://ratatui.rs). Think *Vampire Survivors*, rendered with half-block
"pixels" and a lot of particle juice, running entirely in a text terminal.

You are a rat. The horde wants you dead. Your weapons fire on their own — you just
*move*, dodge, scoop up XP, and choose how to grow.

## Gameplay

- **You only move.** `WASD` / arrow keys. Weapons auto-fire and auto-target.
- **Level up** by collecting the XP gems dropped by slain foes. Each level pauses the
  action and offers **1 of 3 upgrades**: new weapons, weapon level-ups, or passive stats.
- **Survive the clock** (5 minutes) against an ever-escalating swarm — skitterers, bats,
  cats, ranged spitters, hulking brutes, and a timed **boss**.
- **Dash** (`Space`) through danger with a brief window of invulnerability.
- Grab rare pickups: **heal**, **magnet** (vacuum all gems), and **nuke** (clear the screen).
- Three themed maps — **Sewer**, **Kitchen**, **Lab** — each with its own palette,
  obstacles, and floor hazards.
- Persistent **high-score table**.

## Weapons

| Weapon | Behavior |
| --- | --- |
| **Gnaw** | Fires a dart at the nearest foe. |
| **Cheese Spray** | Scatters darts in a forward arc. |
| **Squeak Nova** | Bursts shots in every direction. |
| **Spore Orbit** | Spores circle the rat and shred anything close. |
| **Tail Whip** | Pulses melee damage to everything nearby. |
| **Acid Spit** | Lobs acid that pools and burns over time. |

## Controls

Three schemes, chosen in **Options → SCHEME** (saved beside the other
preferences) and remappable in **Options → CONTROLS**:

| Scheme | Aim | Move | Dash | Fire actives |
| --- | --- | --- | --- | --- |
| **Classic** | nearest enemy (auto) | `WASD` / arrows, or the pointer steers | `Ctrl` or left click | `Space` or left click |
| **Mouse aim** | the pointer | `WASD` / arrows | right click (`Ctrl` too) | left click (`Space` too) |
| **Gamepad** | right stick when deflected | left stick / d-pad | `RB` | `X` |

A left click in Classic is the whole "do the thing" gesture (dash + actives);
in Mouse aim the clicks split so a left click never spends the dash. On a
terminal without lone-modifier reporting, the `Ctrl` dash falls back to `Space`
and the legends say so.

Shared keys in every scheme:

| Key | Action |
| --- | --- |
| `Tab` | Toggle the expanded map |
| `N` | Cycle the minimap size |
| `Z` / `X` | Zoom in / out (the wheel works too) |
| Middle click / `R3` | Reset the zoom to 1.0× |
| `C` | Toggle the pointer |
| `M` | Mute |
| `O` | Options |
| `Esc` / `P` | Pause |
| `1` / `2` / `3`, `←/→`, `Enter` | Pick an upgrade on level-up |

Gamepad: the d-pad or left stick moves, the right stick aims when deflected,
`RB` dashes, `X` fires the charged actives, `Start` pauses, `View` opens the
map, `Y` flips the pointer, `LB` mutes, `RT`/`LT` zoom, `R3` resets the zoom
and `L3` cycles the minimap. The pad is read only while the terminal has focus.

Every binding can be changed on the **Controls** screen: pick a row, press a
key, pad button or mouse button, and a conflict swaps the two actions. `R`
resets the active scheme to its defaults.

## Rendering

The world is drawn into a pixel framebuffer where each terminal cell shows **two stacked
pixels** via the `▀` glyph (foreground = top pixel, background = bottom pixel) with
truecolor. This doubles vertical resolution and makes the particle effects — sparks,
blood bursts, explosions, dash trails, screen shake — possible in a terminal.

> **Terminal note:** held-key movement is smoothest in terminals that support the
> [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) (kitty,
> WezTerm, foot, Ghostty, recent xterm), which report key-release events. Elsewhere the
> game falls back to a short auto-repeat grace window, so it still plays fine. Truecolor
> support is recommended.

## Build & run

```sh
cargo run --release
```

Run the tests:

```sh
cargo test
```

## License

MIT
