# Terminal support

Rat Run is developed against **Ghostty** and assumes its defaults: 24-bit
colour, the Kitty keyboard protocol, and a monospace font that carries every
glyph the art uses. This document records what the game does for terminals that
are less generous, what was measured, and what is still unknown.

Everything below is decided in one place, [`src/term.rs`](src/term.rs), and
exposed through three cheap lookups: `color_depth()`, `glyph_set()` and
`is_multiplexer()`. The renderer never guesses.

## Overrides

| Knob | Values | Effect |
| --- | --- | --- |
| `RATRUN_COLOR` | `truecolor`, `256`, `16` | Force the colour depth; beats all detection. |
| `RATRUN_GLYPHS` | `unicode`, `safe` | Force the glyph set. |
| `--color <v>` | same as `RATRUN_COLOR` | Command-line form; beats the environment. |
| `--glyphs <v>` | same as `RATRUN_GLYPHS` | Command-line form; beats the environment. |
| `--caps` | flag | Print the resolved capability set and exit (no TTY needed). |
| `--probe` | flag | Print the capability set **and** the terminal's real answer to the keyboard-protocol query, then exit. Needs a TTY. |

The resolution order for colour is: flag → `RATRUN_COLOR` → `COLORTERM=truecolor|24bit`
→ `TERM` suffix (`*truecolor`, `*direct`, `*256color`) → known `TERM_PROGRAM`
(24-bit) → `Apple_Terminal` (256) → `TERM` values that mean 16 colours (`dumb`,
`vt100`, `ansi`, `linux`, `screen`, plain `tmux`) → **default 256**.

The default is deliberately 256, not 24-bit: 256-colour SGR is understood
everywhere SGR is, while a 24-bit sequence sent to a terminal that does not
parse it renders as garbage.

Glyphs: flag → `RATRUN_GLYPHS` → CJK locale (`LC_ALL`/`LC_CTYPE`/`LANG` with a
`ja`/`ko`/`zh` language) → `TERM=linux` → `TERM_PROGRAM=Apple_Terminal` →
default **unicode**.

Measured resolutions (`--caps`):

```text
Ghostty          color=truecolor (COLORTERM) glyphs=unicode (default) multiplexer=no
Kitty            color=truecolor (COLORTERM) glyphs=unicode (default) multiplexer=no
WezTerm          color=truecolor (COLORTERM) glyphs=unicode (default) multiplexer=no
iTerm2           color=truecolor (COLORTERM) glyphs=unicode (default) multiplexer=no
Apple Terminal   color=256 (TERM)            glyphs=safe (TERM_PROGRAM)  multiplexer=no
tmux             color=256 (TERM)            glyphs=unicode (default)    multiplexer=yes
Linux console    color=16 (TERM)             glyphs=safe (TERM)          multiplexer=no
dumb             color=16 (TERM)             glyphs=unicode (default)    multiplexer=no
CJK locale       color=truecolor (COLORTERM) glyphs=safe (locale)        multiplexer=no
ssh, unknown     color=256 (TERM)            glyphs=unicode (default)    multiplexer=no
```

## Support table

| Terminal | Colour mode used | Keyboard protocol | Mouse | Known issues |
| --- | --- | --- | --- | --- |
| **Ghostty** (reference) | 24-bit | Kitty, verified (`--probe`: an answer is received; releases arrive under `REPORT_EVENT_TYPES`) | yes | none observed |
| **Kitty** | 24-bit | Kitty. Probe reply `1b 5b 3f 30 75` = `CSI ? 0 u` — an answer, flags 0, i.e. supported with no flags pushed | yes | Window capture with `screencapture -l` returns a black image (Metal surface); verify with the pane's own text instead |
| **WezTerm** | 24-bit | **no** — the probe gets only DA1 (`CSI ? 65;4;6;18;22c`), so held keys use the grace-window fallback | yes | none observed |
| **iTerm2** | 24-bit (`COLORTERM`) | not measured here (automation unavailable) — iTerm2 ≥3.5 supports the protocol | yes | — |
| **Apple Terminal** | **256**, never 24-bit: Terminal.app parses `38;2;r;g;b` wrongly | no (falls back to the grace window) | yes | Its default font `SFMonoTerminal-Regular` lacks the geometric shapes the art uses, so the safe glyph set is selected |
| **tmux** (screen-256color) | 256 by default; 24-bit only with `terminal-features` configured | forwarded to the outer terminal; depends on both | yes (passed through) | Reported as `multiplexer=yes`; colour defaults conservatively to 256 even on a 24-bit outer terminal |
| **Linux console** (`TERM=linux`) | 16 | no | no (no mouse reporting) | Kernel font has no geometric shapes; safe glyphs selected |
| **`TERM=dumb`** | 16 | no | no | No cursor addressing at all, so the TUI cannot run; colour/keys degrade gracefully but the screen does not |
| **SSH / unknown xterm-256color** | 256 | no | yes | Nothing to detect beyond `TERM`; default 256 is correct |

## Colour fallback

When 24-bit is unavailable, every RGB the game emits is folded into the xterm
palette **at the output edge**, so layout, hit boxes and simulation state never
see the change:

* **256 colours** — nearest entry of the xterm 6×6×6 cube plus the 24-step grey
  ramp (indices 16–255), by squared *redmean* distance (cheap perceptual metric:
  green dominates and the red/blue weights shift with the mean red).
* **16 colours** — nearest of the 16 standard base colours, emitted as the named
  ANSI colours (`Color::Red`, `Color::LightBlue`, …) so the terminal's own
  palette applies.

The half-block pixel renderer is the hot path (13 440 cells on a 160×168
viewport), so the fold happens there per pixel through a 4096-entry
direct-mapped cache (packed RGB + depth → palette index, atomics only because
the table is a `static`), and the HUD/menu cells are folded in the same pass
that rewrites glyphs. No allocation per frame.

Degradation, measured on a gameplay frame (fully saturated sprite colours are
the worst case — flat UI panels lose far less):

| Frame | Colours requested | Distinct colours after folding |
| --- | --- | --- |
| play, 80×24 | 108 | **38** (256) / **7** (16) |
| play, 120×40 | 143 | **46** (256) / **8** (16) |

Renders, left to right = 24-bit, the same 24-bit frame folded to 256, and the
game's own 16-colour output:
`scratchpad/terminals/colour_256_before_after_80x24.png` and
`..._120x40.png`.

## Glyph audit

`src/term.rs::safe_symbol` is the whole substitution table: the safe set keeps
ASCII, **box drawing** (U+2500–U+257F) and **block elements** (U+2580–U+259F),
and maps every other glyph in the UI to a single-cell ASCII stand-in. Every
replacement is exactly one cell wide, so a sanitised frame keeps its layout —
proved by the test matrix below.

What stays, and why:

* **Box drawing** — every frame, panel, rule and the minimap's stamped frame.
* **Block elements (eighth blocks)** — the HP/XP/boss bars keep their 8-step
  ramp (▁▂▃▄▅▆▇█). In safe mode they are **unchanged**: these are block
  elements, part of the safe set, and degrading them to ASCII would lose the
  sub-cell resolution the bars rely on. Only the glyphs *around* the bars
  change.
* **The half block `▀`** — the pixel renderer cannot work without it, so it is
  never substituted.

Everything else, with its East Asian Width class (A = "ambiguous", drawn
double-width in a CJK locale; N = neutral) and whether a terminal may render it
as emoji:

| Glyph | Code point | EAW | Emoji? | Where | Safe replacement |
| --- | --- | --- | --- | --- | --- |
| `§` | U+00A7 | A | no | level/theme lore | `S` |
| `«` `»` | U+00AB/BB | N | no | HUD weapon tags | `<` `>` |
| `°` | U+00B0 | A | no | compass | `o` |
| `±` | U+00B1 | A | no | damage modifiers | `+` |
| `²` | U+00B2 | A | no | descriptions | `2` |
| `µ` | U+00B5 | N | no | descriptions | `u` |
| `·` | U+00B7 | A | no | HUD separators | `.` |
| `×` | U+00D7 | A | no | multipliers, test names | `x` |
| `é` | U+00E9 | A | no | lore | `e` |
| `÷` | U+00F7 | A | no | descriptions | `/` |
| `τ` | U+03C4 | A | no | boss debug text | `t` |
| `–` `—` | U+2013/14 | A | no | menu copy | `-` |
| `•` | U+2022 | A | no | minimap legend | `.` |
| `…` | U+2026 | A | no | ellipsis | `.` |
| `‹` `›` | U+2039/3A | N | no | sector picker | `<` `>` |
| `←` `→` | U+2190/92 | A | no | controls legend, menu hints | `<` `>` |
| `↑` `↓` | U+2191/93 | A | no | controls legend | `^` `v` |
| `↔` | U+2194 | A | no | lore | `<` |
| `⇒` | U+21D2 | A | no | lore | `>` |
| `∈` | U+2208 | A | no | comments/art helpers | `e` |
| `−` | U+2212 | N | no | minus sign in layout text | `-` |
| `≈` | U+2248 | A | no | fog coverage text | `~` |
| `≤` `≥` | U+2264/65 | A | no | descriptions | `<` `>` |
| `▪` `▫` | U+25AA/AB | N | **yes, with VS16** | minimap legend | `.` |
| `▰` `▱` | U+25B0/31 | N | no | menu progress bar | `=` `.` |
| `▲` `▴` | U+25B2/34 | A/N | no | compass, arrows | `^` |
| `▼` `▾` | U+25BC/BE | A/N | no | compass, arrows | `v` |
| `▸` `►` `◂` `◄` | U+25B8/BA/C2/C4 | N | **yes, with VS16** | menu cursor, overlays | `>` `>` `<` `<` |
| `◆` | U+25C6 | A | **yes, with VS16** | weapons, boss markers, cache pips | `*` |
| `◇` | U+25C7 | A | no | empty cache pip | `o` |
| `◈` `◎` `●` `○` | U+25C8/CE/CF/CB | A | `●` yes with VS16 | weapon glyphs, legend | `*`/`o` |
| `◢` `◣` `◤` `◥` | U+25E2..E5 | A | no | 8-way minimap compass | `\` `/` `\` `/` |
| `★` `♥` `♦` | U+2605/65/66 | A/A/N | **yes, with VS16** | score stars, health, weapon | `*` `o` `*` |
| `✓` | U+2713 | N | no | check marks | `v` |
| `✦` | U+2726 | N | no | Squeak Nova weapon glyph | `*` |
| `─ ━ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ═ ╞ ╡ ╧ ╭ ╮ ╰ ╯ ╱ ╲` | U+2500–257F | A | no | every frame, panel, rule, minimap | **kept** |
| `▀ ▁ ▂ ▃ ▄ ▅ ▆ ▇ █ ▉ ▊ ▋ ▌ ▍ ▎ ▏ ▐ ░ ▒ ▓` | U+2580–259F | A/N | no | pixel renderer, bars, fill | **kept** |

**The default set** keeps the art as designed (the geometric shapes are
decorative and single-width in a Western locale), with two deliberate changes:
the one emoji in the source (🗡 U+1F5E1 + U+FE0F VARIATION SELECTOR-16, in the
old `push_short_name` test) is **gone** — no monospace font covers it and its
width is unpredictable everywhere — and there is now no variation selector,
emoji or East-Asian-Wide character anywhere in `src/`. Terminals that cannot
draw the rest are handled by detection instead of by changing the art:

* a CJK locale, where *ambiguous* characters are double-width, and
* Apple Terminal, where `SFMonoTerminal-Regular` covers 21 of the 23 non-ASCII
  characters the shipped UI uses but not `◆ ◇ ▪ ▫ ► ◄ ◂ ▸ ▲ ▼ ◤ ◥ ◢ ◣ ✦ ♥ ♦ ▰ ▱`
  (measured from the font's cmap; Menlo covers all of them, Monaco misses 59,
  Courier misses 75), and
* the Linux console, whose font has no geometric shapes at all.

## The startup query

At startup the game asks the terminal whether it speaks the Kitty keyboard
protocol by sending `CSI ? u` and `CSI c` (the recommended pair). crossterm's
`supports_keyboard_enhancement` waits its full **2 s** for an answer, which every
silent terminal — and every terminal that answers only DA1 — charges to
startup. `term::query_keyboard_enhancement` sends the same two queries and gives
up after **300 ms**, with two refinements:

* a protocol answer wins even when DA1 arrives first in the same read (Ghostty
  and Kitty batch both replies), and
* after a DA1-only reply it keeps listening for a further 60 ms, so a protocol
  answer that arrived second is not lost.

No environment sniffing: tmux and ssh forward the query, so asking is always
correct and "don't ask in tmux" would break exactly those setups.

Measured in a PTY whose master is never read (nothing ever answers), from exec
to the first presented frame (DEC 2026 begin): **385–424 ms**, of which the
first byte is written at ~6 ms and the query budget is 300 ms. With crossterm's
helper the same startup costs ~2.1 s.

A terminal that answers the query but reports zero flags (`CSI ? 0 u`, which is
what Kitty sends when no flags are pushed) is **supported**: the verdict is "an
answer arrived", not "flags > 0". An earlier revision got that wrong and
misdetected Kitty.

## Test matrix

Two independent engines, both automated, plus the real windows.

**Headless PTY** (`matrix.py`): the release binary in a PTY with a fixed
`TIOCSWINSZ`, 80×24 and 120×40 × {truecolor, 256, 16} × {unicode, safe} ×
{title, gameplay, expanded map} = **36 captures**. Each frame is reconstructed
with an independent VT parser and checked for: no overflow and every row exactly
`cols` wide; layout preserved (title screen character-identical across colour
modes; gameplay compared on its box-drawing skeleton and static label columns);
safe mode aligned (same skeleton, same label columns, no double-width glyph);
colour-mode consistency (no `38;2;` outside truecolor; no palette index above
15 in 16-colour mode); no unsupported CSI. **All 36 pass.**

**tmux** (`tmux_matrix.sh` + `tmux capture-pane -e`): the same 36 combinations
through tmux. **All 36 pass**, and the pane really stores 24-bit sequences in
truecolor mode, 256-colour indices in 256 mode and only indices 0–15 in 16 mode
(distinct colours on a gameplay frame: 143 / 46 / 8).

**xterm.js cross-check** (`xterm_check.js`, the VT engine VS Code uses): every
one of those 72 frames is replayed through xterm.js and compared row by row with
the other parser. **All agree.**

**Real windows** (one launch each, closed afterwards):
* **Ghostty** — rendered the title and gameplay correctly at the window's own
  size; contact sheet in `scratchpad/terminals/ghostty_contact.png`.
* **Kitty** — the capability probe ran and reported `TERM=xterm-kitty`,
  `COLORTERM=truecolor`, and a protocol answer (`CSI ? 0 u`). Its window could
  not be screenshotted: macOS `screencapture -l` returns a black image for
  kitty's Metal surface. `scratchpad/terminals/kitty_contact.png` shows that
  limitation next to the probe result.
* **WezTerm** — probe ran: `TERM=xterm-256color`, `TERM_PROGRAM=WezTerm`,
  no protocol (DA1 only).
* **iTerm2**, **Apple Terminal** — could not be driven: the screen locked part
  way through this pass, so new windows did not appear and AppleScript
  automation timed out. Their rows above come from `--caps` with the
  environment each app sets, not from a live run.

Per-terminal captures and the raw logs are in
`scratchpad/terminals/` (`frames/`, `frames_matrix.txt`,
`headless_matrix.log`, `tmux_matrix.log`).

## Not tested from this machine

* **Windows Terminal** (24-bit, Win32 input mode, no Kitty protocol) and the
  **legacy Windows console** (conhost — 16 colours, no UTF-8 by default,
  double-width under CJK code pages). The code paths exist (`ColorDepth::Ansi16`,
  `GlyphSet::Safe`, a `cfg(not(unix))` keyboard query that returns immediately)
  but nothing was executed there.
* **Linux terminals** — GNOME Terminal/VTE, Konsole, xterm, Alacritty, foot.
  The Linux *console* path was reasoned about and unit-tested but not run on a
  real VT.
* **A CJK-locale terminal.** This one matters: the pixel renderer's half block
  `▀` is East Asian Width "A", so a terminal that draws ambiguous characters
  double-width breaks the pixel grid itself. Measured on a gameplay frame, every
  row shifts by up to `cols` cells (total ≈ 1650 cells at 80×24) in both glyph
  sets — safe mode only shrinks the *text* damage, because the canvas is built
  from block elements. The game therefore needs a terminal that treats
  ambiguous-width characters as single-width; there is no glyph substitution
  that can fix the canvas without abandoning the half-block renderer.
* **`--ai` wall mode** is not sanitised: `spectate.rs` has its own composite
  render path (out of scope for this change), so the wall still emits 24-bit
  colour and unicode glyphs regardless of detection.
* **Audio** was not exercised anywhere (headless runs have no device).

## Files

* `src/term.rs` (new) — capability detection, colour folding, glyph
  sanitising, the bounded keyboard query, and their tests.
* `src/main.rs` — resolves capabilities before the first draw, adds `--caps` /
  `--probe`, uses the bounded query, sanitises each finished frame.
* `src/render/palette.rs` — `to_term()`: the output colour fold.
* `src/render/framebuffer.rs` — the half-block blit folds colours at the edge.
* `src/render/hud.rs` — the emoji-with-variation-selector removed from the test
  fixture (the only emoji in the source).
* `Cargo.toml` — `libc` (unix only, already in the dependency tree) for the
  bounded `poll`/`read` used by the startup query.
