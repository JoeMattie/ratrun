# Terminal support

Rat Run is developed against **Ghostty** and assumes its defaults: 24-bit
colour, the Kitty keyboard protocol, and a monospace font that carries every
glyph the art uses. This document records what the game does for terminals that
are less generous, what was measured, and what is still unknown.

Detection is decided in one place, [`src/term.rs`](src/term.rs), and exposed
through three cheap lookups: `color_depth()`, `glyph_set()` and
`is_multiplexer()`. The renderer never guesses. The **keyboard** half is decided
by asking the terminal (see *The startup query*), not by sniffing `TERM`.

## Overrides

| Knob | Values | Effect |
| --- | --- | --- |
| `RATRUN_COLOR` | `truecolor`, `256`, `16` | Force the colour depth; beats all detection. |
| `RATRUN_GLYPHS` | `unicode`, `safe` | Force the glyph set. |
| `--color <v>` | same as `RATRUN_COLOR` | Command-line form; beats the environment. |
| `--glyphs <v>` | same as `RATRUN_GLYPHS` | Command-line form; beats the environment. |
| `--caps` | flag | Print the resolved capability set and exit (no TTY needed). |
| `--probe` | flag | Print the capability set **and** the terminal's real answer to the keyboard-protocol query, then exit. Needs a TTY. |

`--caps` on its own is the diagnostic above. `--bot-batch … --caps W:P` is a
different flag (distinct-kind slot caps for a headless batch): the batch runner
parses its command line *before* capability resolution, so the two never
collide.

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

## The keyboard protocol: what is pushed

At startup the game asks the terminal whether it speaks the Kitty keyboard
protocol, and if it answers it pushes **two flags**:

```text
REPORT_EVENT_TYPES | REPORT_ALL_KEYS_AS_ESCAPE_CODES
```

* **`REPORT_EVENT_TYPES`** is what buys **key-release events**. Held movement
  keys stay down until the terminal reports their release, so there is no
  auto-repeat guesswork and no stuck rat.
* **`REPORT_ALL_KEYS_AS_ESCAPE_CODES`** is what makes a **lone modifier press**
  report at all. It is the only way a bare `Ctrl` can be the dash key, which is
  the shipped mapping (`input::DashKey`).

The push is then **verified by asking again**: the protocol's query answers with
the flags currently in effect, so a second `CSI ? u` proves the enhancement
stuck. Only then does `KeyboardMode { lone_ctrl: true }` reach the app. A
terminal that answers the first query but ignores the push (or never answers the
second) keeps `lone_ctrl: false`, and **Space dashes *and* fires the charged
weapons** — the pre-protocol mapping. That fallback is deliberate: losing the
dash key entirely would make the game unplayable, and every legend in the game
names whichever key is live.

### What a release event is required for

| key | release needed? | why |
| --- | --- | --- |
| movement (`WASD`, pad direction) | **yes**, when available | a held key is the movement input; without releases the game uses a short auto-repeat **grace window** and the key lapses unless it repeats |
| `Ctrl` dash | yes (press *and* release) | the lone-modifier press is the dash; release ends the press |
| holding the dash key to charge a **Super Dash** | **yes** | the charge fires on the release edge; without release reporting a key press is always a tap (an ordinary dash) and only the mouse button or the gamepad can charge — the legends say which |
| letters, `Space`, `Q`, digits | no | used as discrete presses from the per-frame `pressed` list |
| `Enter`, `Tab`, `Backspace` | **not relied on** | under `REPORT_EVENT_TYPES` alone these do not report releases; the game only ever treats them as discrete presses, so it never needs one |

Under `report-all-keys` *every* key reports press/repeat/release as escape codes,
which is why the push is a pair rather than just `REPORT_EVENT_TYPES`.

**Focus loss.** A key held when the terminal loses focus never reports its
release, so focus reporting is enabled and `FocusLost` calls `input.release_all()`
— movement cannot stick after an `Alt-Tab`.

## Support table

| Terminal | Colour mode used | Keyboard protocol | Mouse | Known issues |
| --- | --- | --- | --- | --- |
| **Ghostty** (reference) | 24-bit | Kitty, verified (`--probe`: an answer is received; releases arrive under `REPORT_EVENT_TYPES`; the pushed flags read back, so lone Ctrl dashes) | yes | none observed |
| **Kitty** | 24-bit | Kitty. Probe reply `1b 5b 3f 30 75` = `CSI ? 0 u` — an answer, flags 0, i.e. supported with no flags pushed (so the first query is positive and the push is what is verified) | yes | Window capture with `screencapture -l` returns a black image (Metal surface); verify with the pane's own text instead |
| **WezTerm** | 24-bit | **no** — the probe gets only DA1 (`CSI ? 65;4;6;18;22c`), so held keys use the grace-window fallback and **Space** dashes | yes | none observed |
| **iTerm2** | 24-bit (`COLORTERM`) | not measured here (automation unavailable) — iTerm2 ≥3.5 supports the protocol | yes | — |
| **Apple Terminal** | **256**, never 24-bit: Terminal.app parses `38;2;r;g;b` wrongly | no (falls back to the grace window; Space dashes) | yes | Its default font `SFMonoTerminal-Regular` lacks the geometric shapes the art uses, so the safe glyph set is selected |
| **tmux** (screen-256color) | 256 by default; 24-bit only with `terminal-features` configured | forwarded to the outer terminal; depends on both | yes (passed through) | Reported as `multiplexer=yes`; colour defaults conservatively to 256 even on a 24-bit outer terminal |
| **Linux console** (`TERM=linux`) | 16 | no | no (no mouse reporting) | Kernel font has no geometric shapes; safe glyphs selected; no OSC 22, so the OS pointer is left alone |
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
game's own 16-colour output: `colour_256_before_after_80x24.png` and
`..._120x40.png`. These captures live with the review scratchpad and are **not
checked into the repository**; regenerate them with the dump harness
(`RATRUN_DUMP`, `RATRUN_DUMP_DIR`, `RATRUN_COLOR`) if you need them again.

### Lighting tiers follow the colour depth

The 2D lighting pass (`src/render/lighting/`) is the one effect whose *look*
changes with the terminal. `LightingMode::default_for_depth` picks the base
tier; a cost controller can drop it further:

| depth | `LightingMode` | what differs |
| --- | --- | --- |
| truecolor | **Full** | 4-pixel sampling stride, 28 dynamic lights (3 shadowed), **soft** shadows (each light's own softness knob), per-surface colour, no palette dithering |
| 256 | **Hard** | same stride and light budget, but **hard** shadow edges (softness is off outside Full), and the composite dithers into the 256-colour palette to fake the gradients the cube cannot hold |
| 16 | **Coarse** | same geometry as Hard, dithered into the 16 base colours; the minimum brightness it lifts a body to is higher (118 vs 78 of 255), because a 16-colour terminal has less room between "dark" and "unreadable" |
| any | **Compact** | the automatic fallback: 8-pixel stride, 14 lights (1 shadowed), and no Brute/boss **shadow bodies**. Entered when the pass exceeds its own cost budget (160 µs + 0.035 µs/pixel) for 8 consecutive frames, stepped back up when it drops below 56 % of budget |

So a 256-colour terminal loses shadow *softness*, not shadows; a 16-colour one
loses the tint gradient, not the lighting. Separately, a **theme** can ask for
hard pools regardless of mode (the Lab does, via `ThemeAmbient::hard_pools`),
because a clinical floor should have crisp-edged light. None of this changes
what the lights *illuminate*, so no tier can reveal or hide a route.

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

When the first query is positive the game **pushes**
`REPORT_EVENT_TYPES | REPORT_ALL_KEYS_AS_ESCAPE_CODES` and then asks a **second
time**: the protocol's reply carries the flags currently in effect, so the
second answer is the proof that the push stuck. That second answer, not the
first, is what sets `KeyboardMode::lone_ctrl` and therefore whether `Ctrl` or
`Space` is the dash key. Both queries share the same 300 ms budget; the second
one is only issued on a terminal that already answered once.

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
  size.
* **Kitty** — the capability probe ran and reported `TERM=xterm-kitty`,
  `COLORTERM=truecolor`, and a protocol answer (`CSI ? 0 u`). Its window could
  not be screenshotted: macOS `screencapture -l` returns a black image for
  kitty's Metal surface.
* **WezTerm** — probe ran: `TERM=xterm-256color`, `TERM_PROGRAM=WezTerm`,
  no protocol (DA1 only).
* **iTerm2**, **Apple Terminal** — could not be driven: the screen locked part
  way through this pass, so new windows did not appear and AppleScript
  automation timed out. Their rows above come from `--caps` with the
  environment each app sets, not from a live run.

Per-terminal captures, the contact sheets and the raw logs live with the review
scratchpad and are **not part of this repository**. The harnesses named above
(`matrix.py`, `tmux_matrix.sh`, `xterm_check.js`) are likewise scratchpad-only;
`--caps` and `--probe` are the in-tree way to re-measure a terminal.

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
  render path, so the wall still emits 24-bit colour and unicode glyphs
  regardless of detection. The normal game sanitises every finished frame.
* **Audio** was not exercised anywhere (headless runs have no device).
* **A real gamepad** was not exercised in this pass: the whole pad path
  (movement, aim, dash, actives, menu navigation, hot-plug, dead zone) is
  covered by injected `PadEvent`s in tests, and the `gilrs` poller itself was
  only verified to compile and start.

## Gamepad focus caveat

A terminal application has no window of its own, so a controller is readable
whether or not the terminal is in the foreground. The game therefore feeds
terminal **focus reporting** (the same `CSI ? 1004 h` it already enables to fix
stuck keys) into the pad layer and **drops pad input while unfocused**: a pad
resting in someone's lap must not drive a game nobody is watching. `Alt-Tab`
away and back therefore costs one frame of pad input, never a stuck stick.

The pad is also read only when the `gamepad` cargo feature is compiled in (the
default) and a device is connected; with `--no-default-features` or no pad,
every pad function is a no-op costing one branch per frame. On Linux, `gilrs`
links `libudev` at build time (`libudev-dev`).

## Pointer shape (OSC 22)

While the in-game cheese cursor is visible the game asks the terminal to switch
the OS pointer to `crosshair` with OSC 22, and resets it on the way out
(Ghostty, Kitty and iTerm2 ≥ 3.5 honour it; WezTerm and Apple Terminal ignore
unknown OSC). It is skipped on 16-colour consoles, and it never runs in `--ai`
mode, which has its own pointer-free input path.

## Files

* `src/term.rs` — capability detection, colour folding, glyph sanitising, the
  bounded keyboard query, and their tests.
* `src/main.rs` — resolves capabilities before the first draw; owns the
  terminal lifecycle (raw mode, alternate screen, mouse, focus reporting,
  synchronized updates); pushes and pops the keyboard flags; runs the startup
  query and the verifying second query; adds `--caps` / `--probe`; sanitises
  each finished frame; switches the OSC 22 pointer.
* `src/input.rs` — `InputState`, the per-frame `pressed` list, the held-key
  grace window and `DashKey` (which key the HUD and legends name);
  `release_all()` on focus loss.
* `src/controls.rs` — the remappable layer over `input.rs` (three schemes, 23
  actions, the legends), including the lone-modifier rule in
  `KeyBind::from_event`.
* `src/gamepad.rs` — the `gilrs` poller behind the `gamepad` feature, gated on
  terminal focus; `PadState`/`PadEvent` are the test seam.
* `src/render/palette.rs` — `to_term()`: the output colour fold.
* `src/render/framebuffer.rs` — the half-block blit folds colours at the edge.
* `src/render/lighting/engine.rs` — `LightingMode::default_for_depth` (the tier
  per colour depth) and the cost-driven quality controller.
* `Cargo.toml` — `libc` (unix only) for the bounded `poll`/`read` used by the
  startup query; `gilrs` as an optional dependency behind `gamepad`.
