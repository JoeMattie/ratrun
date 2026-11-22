# Changelog

## 0.2.0 (unreleased)

Subject R-47's escape is now a campaign: three sectors, fifteen minutes each,
a boss at the end of every one.

### The run

- **A three-sector campaign.** The Sewers, then the Kitchen or the Laundry,
  then the Lab or the Printworks. You pick the next sector's floor between
  levels and carry up to three powerups through.
- **Fifteen minutes a sector.** The horde builds on a clock. Four named
  minibosses arrive at 3:00, 6:00, 9:00 and 12:00, and the sector's boss at
  15:00. A sector ends when the boss dies or you do.
- **Practice runs.** Sector select has one card per floor. Starting a later
  floor plays it alone, on the first sector's difficulty curve.
- **The Flatbed.** A practice-only getaway ride: pits in the bed, sliding
  cargo and a slipstream that drags small things tailward.

### The facility

- **Procedural levels, ten times larger.** Rooms, halls, corridors and a boss
  arena on a tile grid, new for every seed.
- **Rat-holes.** Gaps in the walls that you fit through and big enemies do not.
- **Mazes.** About one map in eight hides one, with a Cache at the end and a
  one-way door out.
- **Traps and hazards.** Glue boards, baited mousetraps, steam grates in the
  Laundry and ink pads in the Printworks.
- **Fog of war.** Ground you have not seen stays under a dim red veil. The
  Minimap has three sizes, and `Tab` opens the full Map and pauses the game.

### Everything that wants you back

- **48 enemy kinds, eight to a sector, none shared.** Each sector unlocks its
  cast on the clock, from the first fodder to a third trick at 9:45. The 33
  newest are named, statted and in the Library; most still behave like the
  body they are built on.
- **24 named minibosses.** Each has one signature attack with a one-second
  tell, an exposed recovery, and exactly one Chest on death.
- **Ten bosses.** The Jungle Cat, The Horseshoe, The Sweeper, SUBJECT R-01,
  TURF-REAPER MK.IV, KING COBRA, The Mangle, THE NIGHT EDITION, The Magpie and
  The Redtail. A sector draws its boss from its own pool.

### Your build

- **13 weapons.** New: Tail Thrash and Sonic Squeak (ACTIVE weapons, fired with
  `Space` or left-click when charged), Laser Eyes, Super Dash, Acid Rain,
  Hantavirus and Spiked Tail.
- **Super Dash is hold-to-charge.** Hold the dash to fill the meter, release
  for a longer, wider, harder dash. A quick tap is still the ordinary dash.
- **12 passives and 12 Evolutions.** A Chest evolves a max-level weapon when
  you own its partner passive.
- **Racks grow.** You start with four weapon slots and four passive slots and
  can reach eight of each.
- **Auto-pick.** `U` takes level-up cards for you and shows a toast for each.
- **Pocket finds.** Bottle Cap, Wind-Up Mouse, Pocket Watch and Clean Socks
  join the Medkit, Magnet and Purge. The Magnet now pulls far gems and gold in
  fast.

### Gold

- **Gold drops from kills**, with more from elites, minibosses and bosses.
- **The Gene Splice Shelf.** Mouse holes in the walls hide a shop: three
  Splices on the shelf, two sockets to fill, plus Stitch, Turn the Shelf and
  Molt.
- **The Nest.** Unspent gold is banked when a run ends and buys 15 permanent
  upgrades, including more slots, a bigger carry-over and one revive.

### Controls

- **Dash on `Ctrl`**, with a cooldown you cannot miss. On a terminal that
  cannot report a lone `Ctrl`, `Space` dashes and the legends say so.
- **Three control schemes:** Classic, Mouse aim and Gamepad.
- **Every binding can be changed** on the Controls screen, and every legend
  prints the key you actually bound.
- **Mouse everywhere.** Menus are clickable, and an optional cheese cursor
  marks the pointer.

### Looks and sound

- **2D lighting** with cast shadows, level-placed fixtures, smoke and ambient
  effects.
- **Zoom** from 0.5× to 2× with the wheel or `Z/X`. Zoomed out, pellets and
  loot stay crisp.
- **A redesigned HUD**, damage numbers, pixel icons for every weapon and
  passive, and new art for enemies, minibosses and bosses.
- **Separate music and effects volume**, on a new Options screen.

### The rest

- **The Library.** A four-tab dossier (BESTIARY, ARSENAL, FIELD GUIDE, FLOORS)
  that fills in as you meet things.
- **More terminals.** The game detects what your terminal can do and folds
  colour and glyphs to fit: truecolor, 256 colours or 16.
- **Watch it play itself.** `ratrun --ai` shows a wall of self-playing games.
- **An admin console** on the backtick key. A run that uses it sets no scores
  and banks no gold.

### Balance

- A calmer opening, with the first level-up inside about 15 seconds.
- XP costs grow geometrically, so late levels are earned.
- Weapons measured against each other and evened out.
- Later sectors are a different fight, not the first one with bigger numbers.
