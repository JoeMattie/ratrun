# Run tuning

Use a TOML file to test balance in a normal run, the spectator wall, the testbed, or a headless bot batch. The game reads the file at launch and uses the same effective values for every floor of the run. Unspecified fields take the shipped values. Tables and kind names are case-sensitive. An unknown key reports its full path before the game enters raw terminal mode.

```sh
ratrun --tuning-dump defaults.toml
ratrun --tuning harder.toml
ratrun --ai 16 --tuning harder.toml
ratrun --testbed --floor outfall --tuning harder.toml
```

Use `RATRUN_TUNING=harder.toml` to set a launch default. An explicit `--tuning` takes precedence. The dump shows every key, its unit and its current default. Either edit a copy or write a file containing just the changes.

```toml
[enemies]
hp_mult = 1.5

[enemies.kinds.Pillbug]
xp = 18

[weapons.kinds.Gnaw]
cooldown_mult = 0.85
```

Any effective change marks the run **TUNED** and voids Nest banking, high scores, campaign and sector records, and Library discoveries from the run. A file containing only shipped values remains eligible for progression.

## Tables

| Table | Controls |
| --- | --- |
| `player` | Base health, regeneration, movement, armor, magnet radius, dash and hit immunity. The Nest, passives and splices apply to the player base as usual. |
| `xp` | Geometric step cost, steps per level, the three-card opening ramp, weapon damage per level, drops, gem tier thresholds and merge rules. Enemy-kind XP is in `enemies.kinds`. |
| `director`, `director.sectors.<1–4>` | Pulse frequency, opening pulse, rate and live-budget tables, elite cadence, curve speed, HP corrections, sector ramps, boss arrival and cleanup pressure. Rate and HP knot tables interpolate, while the live budget holds each value until its next row. Keep curve times in ascending order. The pulse interval multiplier adjusts frequency without changing the number of bodies in each pulse. |
| `enemies`, `enemies.kinds.<Kind>` | Global HP, contact damage and speed multipliers, and per-kind base HP, speed, damage, XP, and gold. Base overrides apply before global multipliers. Attacks that use the body's damage stat also use its tuned damage. |
| `minibosses` | Four appointment times, HP by sector and appointment, signature damage, contact damage, and XP. Appointments at or after boss arrival are skipped. |
| `bosses`, `bosses.kinds.<Kind>` | Global and per-kind HP and contact multipliers, per-kind hazard damage and warning multipliers, and separate swipe controls for the Jungle Cat. Sector-one HP scale covers bare practice. The Lantern has separate sector-one and later-sector defaults. |
| `loot` | Gold probability and value, named payouts, rare pickup odds and weights, healing, chest fallback and evolution eligibility, and Purse payout. Enemies and minibosses have their own XP tables. |
| `weapons`, `weapons.kinds.<Kind>` | Global and per-weapon damage and per-weapon recovery, including evolutions. Below `1.0` means faster recovery. Super Dash uses dash recovery. |

Use the kind keys printed by the dump. Keep the same number of entries when editing an array. Sector arrays follow campaign order. In a practice run using a bare build, the game uses sector one's difficulty values.

## Sector buildup

Each sector ships with its own live budget, pulse interval and elite interval multiplier. Other sector settings inherit the shared director values unless overridden in `[director.sectors.1]` through `[director.sectors.4]`. The tuning dump prints the shipped rows. Each curve uses seven pairs, with times in curve seconds. Practice uses sector 1 on every floor.

- `live_budget` sets the ordinary-body budget and holds each count until the next step.
- `spawn_interval` interpolates seconds between pulses. Use strictly increasing times and positive intervals. The shared opening and spawn interval multipliers still apply.
- `bodies_per_second` interpolates the rate used to size each pulse and compute the elite interval. Pulse size uses the shared base interval, so changing `spawn_interval` changes frequency alone.
- `elite_start`, `elite_chance` and `elite_interval_mult` replace the shared first-arrival gate, cadence knots and repeat interval multiplier. The quiet period around a miniboss appointment still applies.

The boss cleanup pulses and budget use the shared cleanup settings.

```toml
[director.sectors.1]
live_budget = [[0, 18], [60, 42], [180, 72], [255, 55], [360, 70], [540, 100], [900, 150]]
```

## Pressure beats

Two surges ship enabled. At about 3:19 and 11:34 on each floor, the horde
surges for about 104 s and 101 s, followed by about 268 s and 69 s of
recovery. The floor-specific `HORDE WARNING` callout and sound lead each
surge by `pressure_warning_lead` (5 s). Set the lead to zero for silent
starts. Set `pressure_beats = false` in `[director]` to disable the
surges, recovery and Sewer opening grace.

- Set the two times with `pressure_starts`, the surge lengths with
  `pressure_lengths` and the following recovery lengths with
  `pressure_recovery_lengths`. These use simulation seconds. The windows
  include their warnings and must fit before boss arrival without overlap.
- `pressure_sector_scale` scales the surge and recovery strengths by campaign
  sector. A zero scale skips both beats for that sector.
- `pressure_rate_bonus` borrows pulse cadence from the following recovery.
  The director aims to preserve the ordinary body supply over the two windows.
  The live cap, safe placements and appointment grace still govern actual
  arrivals. Enemy kinds retain their existing XP values.
- `pressure_budget_bonus`, `pressure_contact_bonus` and
  `pressure_recovery_budget_cut` set the live cap uplift, contact damage
  uplift and recovery live cap reduction as fractions of their normal values.
  `pressure_recovery_heal` biases the existing rare finds toward Medkits
  without changing the total chance of a rare find.
- On the Sewer, `pressure_sewer_opening_secs` and
  `pressure_sewer_opening_cut` reduce ordinary contact damage by about
  26.2% for the first 167.6 s while the rat picks up its first cards.

The shipped balance also adjusts the rat's starting HP, regeneration, hit
immunity and XP costs, plus the horde's speed, sector HP and contact ramps,
and boss HP. The effective defaults qualify for progression with or without
a tuning file.

## Boss hazards

Every boss kind accepts `attack_damage_mult` for timed hazard damage and `tell_time_mult` for the warning portion of a queued hazard. The Jungle Cat, Horseshoe Crab, Broom, MegaRat, Mower, Cobra, Magpie and Hawk ship with about ×1.18 hazard damage and ×1.33 warning time. Other bosses default to ×1.0 on both. Body contact has its own `contact_damage_mult`. A hazard keeps its footprint and the length of its active window.

`[bosses.kinds.Cat]` also accepts `swipe_damage` in HP before the damage multiplier and armor (about 17.51 by default), `swipe_windup_mult` for the raised-paw tell in both phases (about 0.98 by default, compounded with `tell_time_mult`), and `swipe_chain_gap` in seconds after a completed first swipe and before the second windup (default 0). The damage and time multipliers and the base swipe damage must be positive. The chain gap may be zero.

```toml
[bosses.kinds.Cat]
swipe_damage = 16.0
swipe_windup_mult = 1.4
swipe_chain_gap = 0.25
```

## Run a comparison

```sh
ratrun --bot-batch 24 --seed 1000 --skill expert --threads 8 --json > baseline.json
ratrun --bot-batch 24 --seed 1000 --skill expert --threads 8 --tuning harder.toml --json > harder.json
ratrun --bot-compare baseline.json harder.json
```

The text report shows the file path, the effective tuning hash and changed fields. The JSON carries the same information in `tuning.path`, `tuning.hash` and `tuning.diff`. Files with equivalent effective values share the hash even when their paths or formatting differ. `--bot-compare` also accepts reports written before the tuning fields existed. When checking exact replay, compare simulation fields separately from the file path, thread count and measured wall-clock fields.
