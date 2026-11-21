# Wave 3 plan: the big content update

Coordination contract for the parallel workers building this update. Content
design (what each weapon, enemy, evolution and shop item *is*) lives in
`WAVE3_DESIGN.md`; this file fixes scope, ownership, shared interfaces and the
testing rules. Delete both when the wave has landed and the docs pass has
folded what matters into `CLAUDE.md`.

## What the owner asked for

- Ctrl dashes; the dash cooldown is much more obvious.
- Icons for weapons and passives.
- Weapon + passive evolutions, each visibly distinct.
- At most 5 weapons and 5 passives at a time.
- Minibosses every 3 minutes that leave a chest; the full boss at 15 minutes.
- Enemies drop gold; mouse holes in walls lead to a gene-splice shop that takes it.
- New enemies, levels, bosses, powerups, passives (including dash cooldown reduction).
- New weapons: acid rain, hantavirus, laser eyes (sweep in front of the rat,
  fire in pulses), a super dash that damages what it passes through.
- Mouse wheel zooms the view: out to 2x the current view, in to 0.5x.
- Docs pass and README with screenshots and video.

## Waves

| Wave | Worker (branch `JoeMattie/<name>`) | Agent | Owns |
|---|---|---|---|
| 0 | `w3-design` | Astra | `WAVE3_DESIGN.md` only |
| 0 | `w3-foundation` | Sol | slot caps, gold, chests, the shared interfaces below |
| 0 | `w3-dash` | Pi | Ctrl dash, dash cooldown read-out |
| 0 | `w3-zoom` | Pi | wheel zoom |
| 0 | `w3-icons` | Neon | `src/game/art_icons.rs`, icons on cards and the HUD rack |
| 1 | `w3-timeline` | Pi | 15-minute level, minibosses, chest drops |
| 1 | `w3-evolutions` | Pi | evolution table, chest rewards |
| 1 | `w3-shop` | Pi | mouse holes, gene-splice shop |
| 1 | `w3-weapons-a` | Pi | Acid Rain, Hantavirus |
| 1 | `w3-weapons-b` | Pi | Laser Eyes, Super Dash, new passives |
| 1 | `w3-enemies` | Pi | new enemy kinds |
| 1 | `w3-powerups` | Pi | new pickups |
| 1 | `w3-levels` | Pi | new themes and their generators |
| 2 | `w3-bosses` | Pi | bosses for the new levels |
| 2 | `w3-art-*` | Neon | art for everything wave 1 added |
| 2 | `w3-review-*` | Sol, Astra | code review of the merged wave; Pi fixes |
| 3 | `w3-balance` | Pi | one bot-measured balance pass |
| 3 | `w3-docs` | Pi | CLAUDE.md, README, screenshots, video |

## Shared interfaces (wave 0 `w3-foundation` lands these; wave 1 builds on them)

Names are binding so that parallel branches meet. Details are the owner's call.

- `loadout::MAX_WEAPONS = 5`, `loadout::MAX_PASSIVES = 5` (distinct kinds).
  `generate_choices` never offers a new kind past a cap; carried campaign
  loadouts respect it.
- Gold: `World.gold: u32` (facade field, round-tripped like `score`), a `Gold`
  drop entity collected like gems, `Sfx::Gold`, a HUD count, carried between
  campaign levels. `World::spend_gold(n) -> bool`.
- Chests: a `Chest` entity (`World::spawn_chest(pos)` for tests, a
  `ChestSpawn` command path for systems), collected by touch, raising
  `World.pending_chests: u32`. The App opens `Screen::Chest` and calls
  `loadout::open_chest(&mut Loadout, &mut rng) -> ChestReward`.
- Evolutions: `Weapon.evolved: bool`; `WeaponKind::evolution() ->
  Option<Evolution>` where `Evolution { passive: PassiveKind, name, flavor }`;
  `Loadout::evolvable() -> Option<usize>` (weapon index at max level whose
  passive is owned) and `Loadout::evolve(idx)`. Foundation ships the plumbing
  with an empty table; `w3-evolutions` fills it and makes `open_chest` prefer an
  evolution.
- Shop: `World.pending_shop: bool` raised by a system when the rat enters a
  shop hole; the App opens `Screen::Shop`. `w3-shop` owns everything behind it.
- Timeline: `director::RUN_SECONDS`, `director::BOSS_TIME`,
  `director::MINIBOSS_TIMES`. `w3-timeline` owns their values.
- Icons: `art_icons::weapon(kind) -> &'static Icon`, `art_icons::passive(kind)`,
  `art_icons::draw(buf, area, icon)`; an `Icon` is a small fixed half-block
  sprite. New kinds fall back to a placeholder until their art lands.

## Testing rules for this wave

- Each worker has its own worktree and build directory. Run name-filtered tests
  while working. Run the full `cargo test` and `cargo clippy --all-targets`
  once, before the final commit.
- Do not edit the golden hashes in `src/game/ecs/harness.rs`
  (`scripted_runs_match_the_reference_trace`) or the map pin
  (`levelgen::tests::map_trace_pin`). If your change moves them, that failure
  is expected: say so in the report. The coordinator regenerates both once per
  merge round. `same_seed_same_run` and every other twin-run test must pass.
- No bot batches (`--bot-batch`) in waves 0 to 2: they saturate the machine.
  Balance is measured once, in wave 3. Pick sensible numbers and move on.
- A run in which a new thing never appears must draw no extra rng.
