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

Any omitted field in `[director.sectors.1]` through `[director.sectors.4]` inherits the shared director value or curve. Each curve uses seven pairs. Times are curve seconds. Practice uses sector 1 on every floor.

- `live_budget` sets the ordinary-body budget and holds each count until the next step.
- `spawn_interval` interpolates seconds between pulses. Use strictly increasing times and positive intervals. The shared opening and spawn interval multipliers still apply. With no override, the base interval is `max(0.35, 1.6 − curve_seconds / 900)` seconds.
- `bodies_per_second` interpolates the rate used to size each pulse and compute the elite interval. Pulse size uses the shared base interval, so changing `spawn_interval` changes frequency alone.
- `elite_start`, `elite_chance` and `elite_interval_mult` replace the shared first-arrival gate, cadence knots and repeat interval multiplier. The quiet period around a miniboss appointment still applies.

The boss cleanup pulses and budget use the shared cleanup settings.

```toml
[director.sectors.1]
live_budget = [[0, 18], [60, 42], [180, 72], [255, 55], [360, 70], [540, 100], [900, 150]]
```

## Pressure beats

Set `pressure_beats = true` in `[director]` to try two surge and recovery
windows per floor. The shipped setting is off. The warning leads each surge
by `pressure_warning_lead` seconds. Set it to zero for silent starts.

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
  `pressure_sewer_opening_cut` give the rat a timed reprieve from ordinary
  contact damage while it picks up its first cards.

## Horde power scaling

Set `power_scaling = true` in `[director]` to size the next wave against
recent effective weapon damage. Each ordinary pulse and timed elite takes
one HP multiplier at spawn. The scripted opening pair, guests, minibosses,
and floor bosses keep their encounter HP. Spawn cadence and payouts stay
on their existing curves. The shipped power switch is off.

The director divides smoothed damage per second by a smoothed reference.
The reference combines the nominal horde rate and HP curves with
`power_reference_hp`, a four-sector calibration in HP per scheduled body at
unit horde HP. `power_memory_secs` is the averaging time constant. The sample
starts neutral on each floor and holds its last buildup value during boss
cleanup. The director raises the ratio to `power_hp_exponent` and clamps it
between `power_hp_floor` and `power_hp_ceiling`.

## Boss hazards

Every boss kind accepts `attack_damage_mult` for timed hazard damage and `tell_time_mult` for the warning portion of a queued hazard. Both default to 1.0. Body contact has its own `contact_damage_mult`. A hazard keeps its footprint and the length of its active window.

`[bosses.kinds.Cat]` also accepts `swipe_damage` in HP before the damage multiplier and armor (default 22), `swipe_windup_mult` for the raised-paw tell in both phases (default 1.0, compounded with `tell_time_mult`), and `swipe_chain_gap` in seconds after a completed first swipe and before the second windup (default 0). The damage and time multipliers and the base swipe damage must be positive. The chain gap may be zero.

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
