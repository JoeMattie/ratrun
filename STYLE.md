# Rat Run Copy & UI Text Style Guide

This guide governs every player-visible string in Rat Run across all supported terminal sizes (`57×20`, `80×24`, and `120×40`). Use it when writing or merging text for new screens (Library, Controls, Permanent Shop, Admin Console, Auto-Pick Toasts, new enemies, and new bosses).

---

## 1. Voice & Tone

Rat Run is a clinical facility dossier about **Subject R-47**, an instrumented lab rat fighting out of a multi-floor research complex.

- **Dry, institutional, deadpan**: Write like a terse lab technician logging an incident nobody wants to sign for (`"Weaponized dairy. Don't ask."`, `"Scar tissue, all the way down."`).
- **Short**: Prefer one crisp clause over two. Cut filler adverbs and throat-clearing.
- **No exclamation marks (`!`)**: Never use `!` in menus, HUD banners, callouts, cards, shop lines, or lore. (Permitted only inside an in-world boss battle cry if one is ever added.)
- **No emoji**: Use only terminal-safe geometric and box symbols (`◆`, `▸`, `✦`, `★`, `·`, `—`, `♥`, `¢`, `×`, `✓`) that already map cleanly through `src/term.rs` (`GlyphSet::Safe`).
- **Zero internal jargon**: Players have not read the design doc or the ECS source. Never expose internal terms on screen:
  - Do **not** write `"appointment"`, `"pocket find"`, `"splice socket"`, `"kind"`, `"rack"`, `"signature"`, `"threat tier"`, `"curve"`, or `"telemetry"`.
  - Name the concrete thing or state what happens (`"Splices 1/2"`, `"choose an upgrade to replace"`, `"BLOCKS NEXT HIT · 12s"`).

---

## 2. Item & Upgrade Copy Formula

1. **Line 1 — Mechanics first**: State **what the item does in one plain clause** before any secondary detail.
   - Prefix manual-trigger weapons with `ACTIVE: Space ...` and dash-triggered weapons with `DASH: ...`.
   - Never hide required trigger keys, durations, or drawbacks inside flavour text.
2. **Line 2 — Flavour**: Muted italic facility note (`<= 42` chars on level-up cards). Sharpens the joke or image; never repeats the stat line.
3. **Truthfulness**: Describe what the simulation actually executes. Read live constants where code formats numbers dynamically; avoid hard-coding tunables in prose when a balance pass could make them untrue.

---

## 3. Settled Glossary

Use **one name per concept** everywhere:

| Concept | Player-Facing Term | Avoid |
|---|---|---|
| Player character | **Subject R-47** (dossiers/lore), **you** (cards/hints) | the rat (in system stats), player |
| Campaign stages | **Sector** (`SECTOR 1/3`, `Sector`) | floor (in headers), stage, map (on sector picker) |
| Sector names | **The Sewers**, **The Kitchen**, **The Laundry**, **The Lab**, **The Printworks**; bonus ride: **The Flatbed** | Sewer Level, Stage 1 |
| Short sector names | **Sewer**, **Kitchen**, **Laundry**, **Lab**, **Printworks**, **Flatbed** | — |
| Currency | **Gold** (`47¢` in HUD/shop badges; `gold` in prose) | coins (as currency name), credits, cash |
| Experience | **XP** (`gem XP`, `XP` bar) | exp, points (for XP) |
| Health & defense | **HP**, **max HP**, **HP/s regen**, **armor** | health points, flat dmg cut |
| Knockback | **knockback**, **melee shove** | weapon shove |
| Triggered weapons | **ACTIVE** weapons (fired with `Space` or right-click when charged) | actives (alone without context), abilities, skills |
| Evolved weapons | **Evolution** (`✦EVO` / `EVOLVED`) | signature, ultimate, awakened |
| Shop | **The Blind Spot / Gene Splice Shelf** (marked on floor by spoon + mouse hole) | vendor, store |
| Shop upgrades | **Splices** (`SPLICES 0/2`, max 2 per run) | splice sockets, mutations, perks |
| Shop services | **Stitch** (heal 35%), **Turn the Shelf** (reroll stock), **Molt** (replace an upgrade with a new Lv1 upgrade of the same type) | swap a kind, rack |
| Floor pickups | **Medkit**, **Magnet**, **Purge**, **Bottle Cap**, **Wind-Up Mouse**, **Pocket Watch**, **Clean Socks**, **Growth Serum**, **Chest**, **Cache**, **Shrink Draught**, **White Coat**, **The Collar** | pocket finds |
| Map views | **Map** (`Tab`, pauses the game) and **Minimap** (`N`, cycles `Normal` / `Small` / `Hidden`) | radar (except as secondary noun for the corner widget) |
| Defeated enemies | **Kills** (`KILLS` in tables; `KO` only on the 1-row HUD top rail) | frags, takedowns |

---

## 4. Canonical Weapon, Evolution & Passive Tags

Never slice `.name().chars().take(4)` for compact strips (`Tail Whip` and `Tail Thrash` collide as `Tail`). Always call `crate::render::hud::weapon_tag(kind, evolved)` so every 2–5 char rack uses one consistent abbreviation whose **first two letters are unique across all 26 weapons and evolutions**:

| Base Weapon | Tag (`2-char`) | Evolution | Tag (`2-char`) | Partner Passive |
|---|---|---|---|---|
| Gnaw | `GN` (`GN`) | Milk Teeth | `MILK` (`MI`) | Sharp Teeth |
| Cheese Spray | `CH` (`CH`) | Fondue Fork | `FOND` (`FO`) | Fat Reserves |
| Squeak Nova | `NOVA` (`NO`) | Echo Chamber | `ECHO` (`EC`) | Frenzy |
| Spore Orbit | `ORB` (`OR`) | Fairy Ring | `FAIRY` (`FA`) | Resilience |
| Tail Whip | `WHIP` (`WH`) | Ratchet Tail | `RTC` (`RT`) | Quick Paws |
| Acid Spit | `SPIT` (`SP`) | Drain Cleaner | `DRAIN` (`DR`) | Thick Hide |
| Tail Thrash | `THRSH` (`TH`) | Flywheel | `FLY` (`FL`) | Counterweight |
| Sonic Squeak | `SQK` (`SQ`) | Roll Call | `ROLL` (`RO`) | Whiskers |
| Laser Eyes | `EYE` (`EY`) | Carbon Copy | `CARB` (`CA`) | Lens Cap |
| Super Dash | `DASH` (`DA`) | Ripcord | `RIP` (`RI`) | Spring Heel |
| Acid Rain | `RAIN` (`RA`) | Downspout | `ACID` (`AC`) | Long Fuse |
| Hantavirus | `VIR` (`VI`) | Dead Letter | `DEAD` (`DE`) | Second Scent |
| Spiked Tail | `BARB` (`BA`) | Barbed Wire | `WIRE` (`WI`) | Counterweight |

**Passive names & 3-char HUD tags**:
`Fat Reserves` (`HP`), `Quick Paws` (`SPD`), `Sharp Teeth` (`DMG`), `Frenzy` (`FIR`), `Whiskers` (`MAG`), `Resilience` (`REG`), `Thick Hide` (`ARM`), `Spring Heel` (`DSH`), `Counterweight` (`SHV`), `Long Fuse` (`FUS`), `Second Scent` (`XP+`), `Lens Cap` (`RCH`).

**Timed pickup buff chips** (row above bottom rail):
`B` = Bottle Cap, `M` = Wind-Up Mouse, `P` = Pocket Watch, `S` = Clean Socks, `G` = Growth Serum, `T` = Shrink Draught, `W` = White Coat, `K` = The Collar. On a wide terminal the strip names the find (`GIANT 18`, `TINY 12`, `WHITE COAT 5`, `COLLAR 16`).

---

## 5. Keys, Numbers & Stat Grammar

### Key Names
- Named keys in legends/prose: **`Space`**, **`Ctrl`**, **`Enter`**, **`Esc`**, **`Tab`** (ALL CAPS only inside all-caps HUD badges like `CTRL DASH`, `SPACE`, `[TAB]`).
- Single-letter keys: **`WASD`**, **`O`**, **`M`**, **`N`**, **`C`**, **`L`**, **`Q`**, **`P`**, **`U`**, **`B`**, **`R`**, **`Z/X`**. These are the defaults: a legend that names a remappable action prints the live binding (`Controls::action_label`), never a hard-coded letter.
- Arrows & symbols: **`↑↓`**, **`←→`**, **`[ ]`**, **`- =`**, **`wheel`**, **`click`**.
- Separators in legends: ` · ` (spaced middle dot) between bindings.

### Effect & Stat Grammar
Always lead with a signed delta followed by the plain stat noun:
- Good: `+15% weapon damage.`, `+12% move speed.`, `+0.5 HP/s regen.`, `-0.18s dash cooldown.`, `-50% passive HP/s regen`, `+1 armor (-1 damage per hit).`
- Avoid: `12% faster`, `halve passive HP/s`, `flat dmg cut`.
- Compact durations: no space before `s` (`4s`, `0.18s`, `12s`).

### Capitalisation & Punctuation
- **Panel headers, badges, buttons, HUD tags & callouts**: `ALL CAPS`, no trailing period (`SECTOR CLEARED`, `PATCH UP`, `BLOCKS NEXT HIT · 12s`).
- **Item names (weapons, passives, evolutions, splices, pickups, enemies)**: `Title Case` (`Cheese Spray`, `Glass Incisors`, `Clean Socks`).
- **Card descriptions & flavour lines**: Sentence case ending with a period (`.`).
- **Shop `plus` / `minus` clauses & denial reasons**: Compact lower-case stat/reason phrases without trailing period (`+22% weapon damage`, `not enough gold`).
- **Control legends**: Key token + lower-case verb/noun (`Enter confirm · Esc back`).

---

## 6. Measured Character Budgets by Surface

Design every string for **`57×20` first** unless the surface explicitly switches strings by width:

| Surface | `57×20` Budget | `80×24` Budget | `120×40` Budget | Notes |
|---|---|---|---|---|
| Level-up card `title` | `<= 28` chars | `<= 38` chars | `<= 38` chars | Leaves room for `◆  01  ` prefix and 9-col level pips |
| Level-up card `desc` (core clause) | `<= 42` chars | `<= 52` chars | `<= 52` chars | `text_rect` is `44×2` (`57×20`) and `54×3` (`80×24+`), **unwrapped** |
| Level-up card `flavor` | `<= 42` chars | `<= 52` chars | `<= 52` chars | Single unwrapped italic line |
| Evolution `attack` | `<= 78` chars | `<= 120` chars | `<= 120` chars | Compact Chest card wraps inside `44×2` (`88` chars max) |
| Evolution `flavor` | `<= 44` chars | `<= 56` chars | `<= 56` chars | Single italic line on Chest card (`44` cols at `57×20`) |
| HUD callout `tag` + `msg` | `tag <= 14`, `msg <= 28` | `tag <= 14`, `msg <= 36` | `tag <= 14`, `msg <= 44` | Pill width `4 + tag + msg <= 55` at `57×20` |
| Boss / miniboss plate | `name <= 22`, `sub <= 32` | `name <= 28`, `sub <= 44` | `name <= 28`, `sub <= 48` | Centred banner inside `game.width - 4` |
| Shop splice `plus` | `<= 38` chars | `<= 42` chars | `<= 48` chars | Single row beside 4-col (`57×20`) or 8-col (`120×40`) vial |
| Shop splice `minus` | `<= 26` chars | `<= 38` chars | `<= 48` chars | Shares row with refusal status (`44 - status.len()` at `57×20`) |
| Shop `Deny::reason` | `<= 17` chars | `<= 20` chars | `<= 24` chars | Right-aligned beside `minus` |
| Shop service blurb | `<= 21` chars (short) | `<= 38` chars (long) | `<= 48` chars (long) | `shop.rs` picks `(long, short)` tuple by `room >= long.len()` |
| Lore `map_intro` | `<= 145` chars | `<= 150` chars | `<= 150` chars | Wraps in `49×3` (`57×20`) / `52×3` (`80×24+`) |
| Lore `victory` / `DEFEAT` | `<= 112` chars | `<= 112` chars | `<= 112` chars | `EndSummary` gives lore `62×2` (`124` chars word-wrapped) |
| Narrow control legend line | `<= 52` chars | `<= 54` chars | `<= 92` (wide) | Title inner width `55` at `57×20`; Pause card inner width `54` |
