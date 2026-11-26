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
| `director` | Pulse frequency, opening pulse, rate and live-budget tables, elite cadence, curve speed, HP corrections, sector ramps, boss arrival and cleanup pressure. Rate and HP knot tables interpolate, while the live budget holds each value until its next row. Keep curve times in ascending order. The pulse interval multiplier adjusts frequency without changing the number of bodies in each pulse. |
| `enemies`, `enemies.kinds.<Kind>` | Global HP, contact damage and speed multipliers, and per-kind base HP, speed, damage, XP, and gold. Base overrides apply before global multipliers. Attacks that use the body's damage stat also use its tuned damage. |
| `minibosses` | Four appointment times, HP by sector and appointment, signature damage, contact damage, and XP. Appointments at or after boss arrival are skipped. |
| `bosses`, `bosses.kinds.<Kind>` | Global and per-kind HP and contact multipliers. Sector-one HP scale covers bare practice. The Lantern has separate sector-one and later-sector defaults. |
| `loot` | Gold probability and value, named payouts, rare pickup odds and weights, healing, chest fallback and evolution eligibility, and Purse payout. Enemies and minibosses have their own XP tables. |
| `weapons`, `weapons.kinds.<Kind>` | Global and per-weapon damage and per-weapon recovery, including evolutions. Below `1.0` means faster recovery. Super Dash uses dash recovery. |

Use the kind keys printed by the dump. Keep the same number of entries when editing an array. Sector arrays follow campaign order. In a practice run using a bare build, the game uses sector one's difficulty values.

## Run a comparison

```sh
ratrun --bot-batch 24 --seed 1000 --skill expert --threads 8 --json > baseline.json
ratrun --bot-batch 24 --seed 1000 --skill expert --threads 8 --tuning harder.toml --json > harder.json
ratrun --bot-compare baseline.json harder.json
```

The text report shows the file path, the effective tuning hash and changed fields. The JSON carries the same information in `tuning.path`, `tuning.hash` and `tuning.diff`. Files with equivalent effective values share the hash even when their paths or formatting differ. `--bot-compare` also accepts reports written before the tuning fields existed. When checking exact replay, compare simulation fields separately from the file path, thread count and measured wall-clock fields.
