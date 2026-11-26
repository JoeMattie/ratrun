# Changelog

## Unreleased

- **Graphics shortcuts and defaults.** Press `G` in play or pause to open
  Graphics and `S` there to cycle smoothing. Smoothing and foreground default
  to Off. Older saves retain their other graphics settings.
- **Still water.** Puddle glints and reflections stay still at rest. Passing
  splash ripples brighten the water and bend reflections as they spread.
- **Laser Eyes reaches across the widest view and stops at walls.** The thinner beams light their path and reflect brightly in puddles.
  Damage falls off with distance, and higher ranks deal full damage farther
  from the muzzle.
- **Hantavirus droppings cast green light.** Higher ranks shine brighter, and the glow fades as they dry.
- **Lava Breath.** Spit molten globs in an arc. They splash on landing and leave
  glowing pools that burn foes. Pair it with Long Fuse to evolve it into Caldera.
- **Named minibosses move differently.** Corkscrew signals a lunge, Bobbin
  circles, and Bell Diver leaves a crack where it will surface. All 24 have
  two attacks, with their own pace and a Library entry for both.
- Added an interactive testbed for trying every weapon, evolution, pickup,
  enemy, miniboss, and boss on any sector. Launch it with `--testbed` or the
  console's `testbed` command.
- **Pulsing rat marker.** A bright halo helps locate you on the minimap and Tab map.
- **Clearer map boundaries.** Known walls are solid warm grey and unexplored
  edges are dotted. The Tab map footer explains both marks.
- **Foreground bands.** Switch on Foreground to reveal oversized pipes,
  catwalks and other overhead objects that cross the view.
- **Five XP gem tiers.** Distant gems combine into larger drops that carry
  their combined XP. The Field Guide lists each tier's colour and value.

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
