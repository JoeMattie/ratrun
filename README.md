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

| Key | Action |
| --- | --- |
| `WASD` / arrows | Move |
| `Space` | Dash |
| `1` / `2` / `3`, `←/→`, `Enter` | Pick an upgrade on level-up |
| `Esc` / `P` | Pause |
| `Q` | Quit to menu |

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
