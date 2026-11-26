# Changelog

## Unreleased

- Removed Hold Point and its Clamp evolution.

## 0.4.2 (2026-11-26)

- **Browser edition.** Play with a keyboard and mouse. The canvas renderer
  targets 60 frames per second. Scores, settings and Nest progress stay in
  browser storage. Music and effects use Web Audio.
- **Windows terminal.** Filling the last cell no longer scrolls the
  alternate screen in consoles that wrap immediately.
- **Balance tuning file.** Load balance overrides with `--tuning`, or use
  `--tuning-dump` to export the shipped values. See [TUNING.md](TUNING.md).
- **Surges.** Horde warnings precede surges at about 3:19 and 11:34 in
  each sector. Crowds grow and hits land harder. During recovery, the
  crowds thin and Medkits turn up more often.
- **Rebalance.** Subject R-47 starts with more HP, recovers faster and
  stays protected longer after a hit. Early upgrades cost less XP.
  Contact hits are softer for the first 2:48 in the Sewers. Horde size,
  enemy HP and attack warnings have been adjusted across the facility.

## 0.4.1 (2026-11-26)

- The Lantern swims over walls, stays opaque when it dives, and strikes more
  often. Its phase shifts are shorter.
- Growth Serum widens Super Dash and Ripcord hits. Dash effects grow with you.
- Foreground shadows can enter at the edge of the screen before the objects
  that cast them.
- The spectator wall can run at uncapped MAX speed. Use `--speed max` to start
  there and `--skill mixed` to rotate through Expert, Skilled and Casual.

## 0.4.0 (2026-11-26)

### New

- **The Outfall.** Take the storm gate after clearing the Sewers to reach a
  third Sector 2 route and unlock it for practice. Swim in a diving suit
  above a sandy seabed with aquatic plants. Blue-green light, rippling
  caustics, depth haze and drifting particles mark the flooded culvert.
- **Outfall hazards.** Avoid tidal drops and lure traps that release Congers.
  Wave crests streak downstream as a surge pushes you up to 64 px. Crab
  pots replace glue boards and mousetraps here. They can cage you or smaller
  enemies, and an empty snap can release a Shore Crab. Dash to break free.
- **Outfall encounters.** Eight enemies and four named minibosses join the
  Library. Fish swim above their shadows. Shore Crabs arrive in groups,
  scuttle along the seabed and pause before pinching.
- **The Lantern.** A 264 px anglerfish guards the Outfall. Seen from above
  at every turn, it sweeps a long lure, snuffs wall lamps and leaves a
  decoy light. It submerges between phases and can pull you toward its jaws.
  Aim for the head.
- **Return Current.** The Outfall's 40-bar loop runs at 100 BPM with walking
  bass, electric piano, flute, harp and soft strings.
- **Lava Breath.** Molten globs arc out and splash into pools that burn
  enemies. Pair it with Long Fuse to evolve Caldera.
- **Glass Bead.** Shots bounce off walls. Pair it with Lens Cap to evolve
  Last Bounce.
- **Plumb Bob.** A weight accelerates through its swing and its chain trails
  at each turn. Pair it with Counterweight to evolve Shed Bob.
- **Hold Point.** Stand still to charge a knockback pulse. Pair it with
  Thick Hide to evolve Clamp.
- **Shrink Draught.** A short-lived dose makes you smaller and quicker.
- **White Coat.** Blocks incoming damage and suspends your weapons for 5s.
- **The Collar.** Doubles gem XP while elites and the miniboss close in faster.
- **Blackouts.** Indoor lamps go out for 8s at 5:00 and 11:00. The Lab's
  second outage moves to 7:30 and lasts 18s. Enemy glints, shots and attack
  warnings stay visible.
- **Combat testbed.** Try weapons, Evolutions, pickups and encounters on any
  sector. Launch with `--testbed` or enter `testbed` in the console.

### Changed

- **Sewer opening.** Two curled Pillbugs arrive off-screen from 0:22. A Drip
  Shaft marks its splash tile, and a Bottle Cap appears outside the start
  room at 1:30. Corkscrew's trench stays wet for 25s.
- **Named minibosses.** The 24 returning minibosses each have two attacks
  with distinct shapes, movement and timing. Corkscrew signals a lunge,
  Bobbin circles, and Bell Diver cracks the ground where it will surface.
- **Five XP gem tiers.** Distant gems merge into larger drops and keep their
  combined XP. The Field Guide lists each tier's colour and value.
- **Longer dashes.** Ordinary and charged dashes travel twice as far. A
  fully charged Super Dash crosses about 119 px on open ground.
- **Growth Serum.** The giant fits through corridors but is too wide for
  rat-holes. More of its body can take a hit.
- **Tail Whip.** The swipe cuts a 160° arc behind your facing or aim.
  Ratchet Tail throws a struck foe forward into the crowd.
- **Laser Eyes.** Beams sweep to the edge of the widest view, stop at walls,
  light their path and reflect in puddles. They follow your eyes during the
  sweep. Damage falls off with distance, and higher ranks keep full damage
  farther out.
- **Hantavirus droppings.** Dark pellets cast a wider green glow. Higher
  ranks shine brighter, and the glow fades as the pellets dry.
- **The Laundry.** Washers and dryers tumble behind lit portholes. Clothes
  piles and overflowing baskets sit beside the machines.
- **Still water.** Puddle glints and reflections stay still at rest. Passing
  splash ripples brighten the water and bend reflections as they spread.
- **Map and Minimap.** Known walls have solid grey outlines and unexplored
  edges are dotted. The full Map labels both marks. Your position pulses
  on both.
- **Foreground and Graphics.** A few blurred bands cross each sector, with
  angled kelp and bubble trails in the Outfall. Foreground defaults to On
  and smoothing to Off. Older saves adopt those defaults and keep their
  other graphics preferences. The default key to open Graphics in play or
  pause is `G`. Press `S` in Graphics to cycle smoothing.

### Fixed

- **Cast shadows.** Diagonal edges follow the geometry at every lighting
  quality when Light Effects is on.
- **Windows console.** `exec` preserves backslashes in ordinary Windows
  paths.
- **Cross-platform runs.** Simulation maths gives the same results for
  the same inputs and seed on macOS, Linux and Windows.

### Removed

- The Daily run.

## 0.3.0 (2026-11-25)

Not signed or notarised on macOS: right-click the app and choose Open the
first time.

- **A Graphics page in Options.** Lighting quality, light effects, smoothing,
  the foreground layer, glyph particles and screen shake, all saved between
  sessions.
- **Smoother picture at 1x.** Sprite and prop edges are rounded with part-cell
  block glyphs (an xBR upscale fitted to sextants) where the terminal has
  them. `SMOOTHING` on the Graphics page picks between six levels, including
  an experimental wedge-glyph mode. Costs about 0.4 ms a frame at 120x40.
- **Standing water.** The Sewers are very wet, the Laundry and Kitchen have
  puddles, and the rat splashes through them. Lamps reflect in the water.
- **Round-glyph particles.** Dust motes in lit rooms, sparks off hits, and
  bubbles and steam from pipes and grates, drawn with small round glyphs.
- **Hantavirus is deployed by droppings.** The rat leaves one behind it every
  cycle; the first enemy to step in one takes the infection, which then jumps
  down the pack as before.
- **The Drake.** A big duck wanders into the Sewers from 2:30. It winds up,
  shows you the lane, and pecks three times. It is slow to recover.
- **The giant rat is drawn properly** at every size, instead of the small
  sprite with its pixels doubled.
- **Whip and laser reach stop at the screen edge.** Neither hits what you
  cannot see.
- **Gene splices last one sector.** Both sockets are empty again on the next
  floor, so the shop matters every time.
- **Loot markers shrink when the view is zoomed out.**
- **MIT licence.**
- **Foreground layer, rebuilt.** The overhead things are few, huge and
  attached: pipes, ducts, rails and clotheslines run wall to wall, hoods and
  arms reach in from a wall, lamps and hoists hang on cords that leave the top
  of the view. They are drawn magnified, with chunky scaled-up pixels, and
  thin out around the rat and near a boss. The Flatbed gets roadside branches
  sweeping past, power lines with passing poles, and cloud shadows.
  `FOREGROUND` on the Graphics page turns the layer off.

- **Lighting effects.** Light pools are smooth instead of blocky, floors and
  walls take a little relief, wet and metal things catch the lamp, bodies
  throw soft shadows (one straight down, one from each nearby lamp), lamps
  put faint shafts and dust in the air, and wet floors carry a streak of the
  lamp above them. Shadows, shafts, sheen and reflections were turned up after the first pass; bloom stays low. `RATRUN_FX=0` (or the
  console's `fx off`) restores the previous look exactly. Costs about a third
  of a millisecond a frame and a fifth to a third more terminal output.
- **Laser Eyes** is now two beams that close inward like scissors, at every
  rank; from rank four they cross and carry on to the far edges. It reaches
  1.6 times as far and sweeps in a third of the time. Damage per sweep is
  unchanged.
- **Growth Serum.** A rare pocket find: for twenty seconds the rat is five
  times its size, the view pulls back, and everything ordinary underfoot is
  crushed. Elites, minibosses and grounded bosses take a heavy stomp instead.
  The giant takes half damage.
- **Bosses can no longer be stun-locked or kited forever.** A stagger is
  rationed (0.3 s in any 4 s) and never interrupts an attack that has begun;
  shoves are capped; a boss that cannot reach the rat for three seconds
  sprints or leaps to close the gap. The jaguar pounces from range, and the
  mower locks on to a rat that backs out of its scan.

## 0.2.0 (2026-11-22)

Subject R-47's escape is now a campaign: four sectors, fifteen minutes each,
a boss at the end of every one, and a getaway on the back of a truck.

### New since the first notes were written

- **Four sectors.** The Sewers, then the Kitchen or the Laundry, then the Lab
  or the Printworks, then the Flatbed: the getaway is the finale.
- **No boss arenas.** At 15:00 the boss comes to wherever the rat is.
- **Forty-eight ordinary enemies and twenty-four named minibosses**, eight and
  four to a floor, none shared between floors.
- **Weapons reworked so each one looks and plays like itself.** Laser Eyes
  sweeps a beam, Squeak Nova is an expanding ring, Sonic Squeak a forward
  cone, Tail Whip a swipe in front, Gnaw a bite, and Spiked Tail a real whip
  that cracks on a target most of the way across the screen.
- **Slower, heavier levels.** A level takes about four times as long to earn
  and adds 4 % weapon damage. The first three come quickly.
- **Right click** fires the charged weapons in play and backs out of any menu.
- **New music.** Six floor songs in a 16-bit voice (`RATRUN_MUSIC=chip` or
  `classic` for the older sounds).
- **Lighting.** Darker floors, pools of light with soft edges, wall lamps.
- **Overhead things** slide past above the floor as the camera moves.
- **Sharper zoom.** Zoomed-out views use finer block glyphs where the terminal
  has them, and enemies never appear inside the widest view.
- **The Library is alive.** The selected card plays the thing it describes.
- **Half the CPU.** The frame is written to the terminal in one pass, and a
  still screen sends nothing.

### Known issues

- The macOS build is not signed: right-click the app and choose Open the
  first time.
- Boss and miniboss fights run shorter than intended, and level one has a hard
  minute around 6:00.
- Wall lamps are very bright and their light has a stepped edge.
- The overhead layer is faint and small; it is being redrawn larger.
- Some Library cards are small or dim (boss plates, Tail Thrash).
- Gamepad rumble has not been tried on a real controller.

### The run

- **A campaign.** The Sewers, then the Kitchen or the Laundry, then the Lab
  or the Printworks, then the Flatbed. You pick the next sector's floor
  between levels and carry up to three powerups through.
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
