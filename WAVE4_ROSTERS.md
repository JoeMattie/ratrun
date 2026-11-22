# The things underfoot

> **Status (in the tree now).** This is the design contract; `src/game/roster.rs` is its implementation.
> - **Registered:** all 48 ordinary kinds (`EnemyKind::all()` is 49 with the legacy `Boss` body) and all 24 minibosses (`MinibossKind`). Every floor's eight slots and four appointments hold their designed kind; no slot carries a stand-in.
> - **Director:** the slot unlocks, caps, pool shares, single weighted draw and the 9:45 featured rotation below are live (`roster::pick_kind`).
> - **Still pending:** the per-kind rules of the 33 wave-four ordinary kinds. Each behaves as its `EnemyKind::archetype()` base today: the `V(X)` rows lack their one new rule and the six `N` rows (Sump Leech, Egg Timer, Walking Syringe, Peg Snapper, Staple Crawler, Loose Can) borrow a neighbour's behaviour. Bodies without bespoke art draw their archetype recoloured. `roster.rs`'s module docs carry the exact list.

R-47 should know which floor he woke on before he finds a wall. In the sewer the first kill is a copper roach that runs straight at his teeth. In the laundry it is a tuft of wet wool that advances one stitch at a time. Give each floor exclusive ownership of **eight ordinary kinds**, including both one-XP bodies. Keep the dodge vocabulary, collision rules and timings recognizable across the campaign. Spend the art budget on the bodies that fill the screen, and the behaviour budget on the encounters that change where the rat wants to stand.

## Reading a room

| Slot | First eligible | What the player learns |
|---|---|---|
| A, ground fodder | 0:00 | Opens alone. The first six approach from one direction. The rat can double back for the gems. |
| B, flying fodder | 0:45 | Smaller HP pool, faster, weaving approach. Flights follow legal routes as the Bat does. Cut across the weave. |
| C, chaser | 1:15 | A longer stride, nose or jaws pointed at the rat. Give up a little ground to regain a clear approach. |
| I, first trick | 2:00 | Read one warning under light pressure before the first chest. |
| R, shooter | 3:35 | Side-on firing body, charged muzzle, small bright shot. Break sight or step across the shot. |
| II, second trick | 4:30 | A second decision about terrain, spacing or target priority. |
| H, heavy | 6:35 | Wide shoulders and a slow gait. Pull it away from a choke. Its elite can preview this silhouette after 3:20. |
| III, lieutenant's ordinary support | 9:45 | Third trick. At 7:15 feature II and at 8:15 feature I to rehearse both before III arrives. |

**Sheet key.** Each table row is one bestiary entry. Stats are unscaled HP / px per second / contact damage / XP. Gold is chance in percent × pieces on success. A slot carries the unlock above. E = existing, F = in flight (numbers and tell timings are proposed tuning targets). R(X) = reskin of X's behaviour with new numbers, V(X) = variant of X with one new rule, N = new behaviour. The fifteen E/F rows appear only on their listed floor.

For R and V, inherit the named base's steering, navigation size and attack unless a change is given. Skitterer follows flow, Bat weaves, Cat bears straight down and cannot fit a rat hole, Brute plugs the wide route. “Shot” inherits Spitter's single aimed pellet, 75 px/s, 4 s life, 0.6 × contact, range under 170, clear line of sight, 2.2 s cooldown, 0.75 s muzzle swell and 80–110 px stand-off. Other ordinary attacks deal contact only unless damage is specified. All lengths below are world px, attack clocks are seconds. New signature periods restart after recovery.

Draw ordinary cores 6–12 px across. Keep a two-pixel mass on either side of a defining notch where possible. Give each body a light and a dark edge against its own tiles. Preserve the pose change and the hazard outline at 2× zoom-out. Long hazards and snake segments may extend beyond the core. Telegraph with shape first, colour second. Give each floor a procedural sound pair for its two fodder kinds, played on approach in view, coalesced by kind with a 0.8 s floor-wide cooldown. Reserve louder accents for tells. A silent player gets the same information.

## Sewer

Copper and bone against wet masonry. The first two sounds are claws on a pipe and wing leather under a grate.

| Kind | Slot · build | Silhouette and motion | Stats | Behaviour / player's answer | Gold | Field note |
|---|---|---|---|---|---|---|
| Skitterer | A · E | Copper almond, long feelers, six scraping feet | 6/30/5/1 | Small direct pursuer. Turn through the thin edge of its line for gems. | 8×1 | The pipes have tenants. |
| Bat | B · E | Notched M wings folding around a pin head | 4/42/4/1 | Weaves through small passages. Cross its approach before its next wingbeat. | 10×1 | The grate lets the squeak through first. |
| Pipe Eel | C · V(Cat) | Bone hook with a thick neck, tail scribbles | 16/39/8/3 | Steers at half turning rate. Reverse around a pipe end to shed it. | 18×2 | Knows every bend but the last. |
| Pillbug | I · E | Ribbed silver oval folds into a shield | 18/23/5/3 | Within 24 curls toward rat for 1.4, cooldown 2.6. Front 120° takes quarter damage, rear takes double. Circle its shoulder. | 18×2 | It brought its own door. |
| Grate Archer | R · R(Spitter) | Barnacle cone on two feet, lip puckers | 12/22/6/4 | Shot through its puckered lip. Put a pipe between lip and rat. | 20×2 | Something spits back up the drain. |
| Sump Leech | II · N | Black comma opens into a pale sucker | 11/25/5/3 | At ≤24 extends a straight 24×4 tongue after 0.7, holds 0.4, then rests 4. Touch slows 30% for 1, once per extension. Bait the tongue and cross behind its head. | 20×2 | Drainage works both ways. |
| Sluice Toad | H · R(Brute) | Squat slab, twin eye turrets, slow belly haul | 58/16/15/10 | Heavy continuous contact. Draw its broad belly out of the valve throat. | 45×3 | The valve is open. The toad isn't. |
| Big Snake | III · F | Pearl head, four linked dark S segments sliding in its wake | 36/23/7/6 | Moving wall, one health pool. Head coils 0.9, then sweeps a locked 32×6 lane in 0.4, rests 1.2, period 5. Round the tail while the head is committed. | 32×3 | The longest pipe down here breathes. |

## Kitchen

Grease-dark bodies against pale counter tiles. Ant feet tap in pairs. The fly makes a short, rising whine.

| Kind | Slot · build | Silhouette and motion | Stats | Behaviour / player's answer | Gold | Field note |
|---|---|---|---|---|---|---|
| Crumb Ant | A · V(Skitterer) | Three beads and a square crumb, marching | 5/31/5/1 | Holds heading for 0.25 between course corrections. Cut across the file. | 8×1 | Someone counted the crumbs. |
| Fruit Fly | B · R(Bat) | Two upright seed wings over a round red eye, tight bob | 4/41/4/1 | Weaving flyer catches a rat trailing the ant file. Sweep across its nose. | 10×1 | Yesterday's peach has company. |
| Cat | C · E | Two ears, white bib, long level tail, low prowl | 18/38/9/3 | Direct large-class pursuit. Slip through the pantry rat hole. | 18×2 | Kitchen staff. Paid in mice. |
| Egg Timer | I · N | Round clock, key ears, waddles until the dial stops | 13/21/4/3 | Within 26 winds for 1, ticks a stationary 5-wide ring from radius 4 to 22 in 0.8 for 7, then sleeps 2, cooldown 4. Stay close inside its departing ring or clear the dial early. | 18×2 | Lunch is on time. Run. |
| Pepper Mill | R · V(Spitter) | Waisted wooden pawn twists its cap | 13/20/6/4 | Shot splits into two rays at ±10° on release. Walk the seam as the cap twists. | 20×2 | Two turns for a small rat. |
| Porter | II · E | Green scarab with a square sugar cube overhead | 26/20/6/5 | Raises cube 0.9 within 24 of wounded ordinary ally, heals 8 every 5. Hit during lift to interrupt. | 40×3 | One lump for medicinal purposes. |
| Pie Weight | H · R(Brute) | Crimped tin skirt under a ceramic ball, rolls ponderously | 60/16/15/10 | Keeps advancing under fire. Lead it along the counter's long side to open the aisle. | 45×3 | The recipe calls for a blind bake. |
| Sink Crab | III · V(Pillbug) | Broad flat shell, one pot-lid claw, scuttles sideways | 22/23/6/4 | Pillbug curl and arcs. Shield faces 90° left of approach throughout curl. Let the shield pass and cut right behind the lid. | 25×2 | Left the plug in. Took the lid. |

## Lab

White ceramic feet, black rubber and specimen pink against console glow. A swab goes *shff* against tile. A loose sample lid chatters in the air.

| Kind | Slot · build | Silhouette and motion | Stats | Behaviour / player's answer | Gold | Field note |
|---|---|---|---|---|---|---|
| Swab Foot | A · R(Skitterer) | Cotton club on a bent black stem, paddles tip first | 6/29/5/1 | Small direct cleaner. Shave the head off the approaching column. | 8×1 | The floor tested positive for you. |
| Spinner Lid | B · V(Bat) | Floating slanted disk, bright rim alternating face and edge | 4/40/4/1 | Weave flips handedness every 1.2. Cross while it shows its broad face. | 10×1 | Nothing in this room stayed sealed. |
| Glass Runner | C · R(Cat) | Clear ampoule on two long rubber feet, vaulting gait | 17/39/8/3 | Runs a direct large-class route. Turn through the narrow service chase. | 18×2 | Return empties to the lab. |
| Walking Syringe | I · N | Needle on stilt legs, plunger draws back | 13/25/5/3 | Within 30 plants tip, draws a 26×3 dotted ray for 0.75. Thrust fills ray for 0.2, 8 damage. Retracts over 0.8, cooldown 4. Step across the needle and approach the barrel. | 18×2 | Hold very still, said the machine. |
| Spitter | R · E | Veined acid sac, wings pitched back as it strafes | 12/22/6/4 | Shot. Duck behind a tank when the sac fills, leave at a different corner. | 20×2 | The control group learned to aim. |
| Lilliputians | II · F | Three tiny upright coats on one base, spear / bow / rope overhead | 24/22/5/5 | One squad entity cycles tools every 4. Spear marks 18×4 for 0.8 then jabs for 0.2 (7). Bow draws 0.8 then fires one shot. Rope marks a radius-6 disk at rat for 1, snaps once, roots for 0.45, dash breaks it. Flank the line, strafe the bow, leave the loop. | 30×2 | Their paperwork is actual size. |
| Autoclave Cart | H · R(Brute) | Box door on two big wheels, latch oscillates | 60/15/16/10 | Wide slow blocker. Tow it past the tanks before circling for gems. | 45×3 | Sterile inside. Very busy outside. |
| Turnkey | III · E | Brass key drone dragging its teeth across tile | 22/24/6/4 | Every 6 lays a 22×2 crossbar across its old route. Dots for 0.8, live for 2, 8 on crossing once. Spend the bar in an empty aisle. | 32×3 | Nobody asked who locked the hallway. |

## Laundry

Ivory yarn and copper fittings against indigo grout. Wet wool slaps. The clothes moth makes a dry brush sound between the slaps.

| Kind | Slot · build | Silhouette and motion | Stats | Behaviour / player's answer | Gold | Field note |
|---|---|---|---|---|---|---|
| Wool Pill | A · V(Skitterer) | Pale round tuft on a trailing stitch, gathers and stretches | 5/34/5/1 | Pauses for 0.15 every 1.1 to pull its strand free. Double back through the stalled rank. | 8×1 | Hot wash. Cold feet. |
| Clothes Moth | B · R(Bat) | Broad triangular shawl opens and shuts over a blunt head | 4/40/4/1 | Weaves through the break left by the pills. Give the moving shawls room. | 10×1 | Holes sold separately. |
| House Centipede | C · F | Two long feelers over a striped comb, legs ripple | 16/41/8/3 | Tracks nearest wall within 12, pursuing around its edge. Runs away for 0.8 on hit. Fight in the aisle centre or hit it before turning a corner. | 18×2 | Enough feet to fill the missing socks. |
| Peg Snapper | I · N | Forked wooden jaw on a bright hinge, walks open | 14/24/5/3 | Within 20 shows two 16×3 jaws 14 apart for 0.8. Jaws slide together in 0.3 for 7 on touch, stay closed 1, cooldown 4. Walk out through an open end. | 18×2 | Mind the business end of the line. |
| Starch Bulb | R · V(Spitter) | Rubber pear on a tripod, thin crooked nozzle rocks | 12/21/6/4 | Shot travels at 45 for its first 0.5 then 95. Cross near the bulb before the jet picks up. | 20×2 | Keeps a fine crease in the air. |
| Bell Flea | II · E | Copper tuning fork doubles over then springs | 9/32/4/2 | Marks two radius-6 crosses, hops up to 26 at 0.35, home 0.30 later, 7 per impact, cooldown 4. Leave both crosses free. | 14×2 | Even the echo has feet. |
| Lint Golem | H · R(Brute) | Broad uneven shoulders and a dark dryer-button eye, shambles | 58/16/16/10 | Slow solid plug between washers. Lead it into the sorting bay. | 45×3 | Clean the filter after every shift. |
| Wick Moth | III · E | Narrow hanging flame, bright tail bobs below short wings | 12/28/4/3 | Death leaves a wick for 1, then radius-9 burst for 9 to rat and 20 to enemies. Pass a golem over the wick. | 22×2 | It dies with the light on. |

## Printworks

Bone paper and metal edges catch the register lamps above charcoal slabs. Silverfish rasp. Slips of paper make a double flap.

| Kind | Slot · build | Silhouette and motion | Stats | Behaviour / player's answer | Gold | Field note |
|---|---|---|---|---|---|---|
| Silverfish | A · R(Skitterer) | Narrow spindle with a three-prong tail, rippling sides | 5/32/5/1 | Streams through gaps in the press beds. Meet its nose to collect the close gems. | 8×1 | Eats yesterday's news first. |
| Erratum Slip | B · V(Bat) | Torn L of paper tumbles edge over edge | 4/38/4/1 | Weaves twice as wide for the first 1 after spawn. Wait for the loose sheets to fold in before turning. | 10×1 | A small correction is on its way. |
| Type Slug | C · V(Cat) | Lead block with raised T face, skates in a straight ruled line | 18/39/9/3 | Keeps its last heading for 0.6 after losing sight. Break sight at a reel, then reverse. | 18×2 | Set in twelve-point pursuit. |
| Staple Crawler | I · N | Inverted U, two needle feet alternately planted | 14/23/5/3 | Within 26, marks two radius-4 holes 14 apart astride rat for 0.8. Both feet land for 0.4, 7 each, shared one-hit latch. Rests 1, cooldown 4. Walk between the feet. | 18×2 | Leave room in the margin. |
| Paper Wasp | R · R(Spitter) | Folded arrow wings, rolled newsprint abdomen coils | 12/23/6/4 | Shot from rolled tip. Move across the press lane as the tube opens. | 20×2 | Delivers the morning sting. |
| Termite Horde | II · F | Moving scalloped patch with three pale heads, contracts when struck | 28/22/6/5 | One many-bodied enemy. Direct projectile hits deal half, area hits full. At ≤20, bunches 0.8 then spreads to radius 8 for 0.7, contact once, reforms for 1.5, cooldown 4. Lead it across a pool or keep it at the edge of sustained fire. | 30×2 | The paper bill keeps getting smaller. |
| Paper Reel | H · R(Brute) | Wide spool, dark axle hole, loose end flaps as it rolls | 62/15/15/10 | Wide continuous contact. Pull the roll off the aisle before entering. | 45×3 | Stop press means stop press. |
| Ink Blot | III · V(Wick Moth) | Ragged black star with a porcelain outline, flows then contracts | 12/25/4/3 | Wick fuse, radius and damage. Burst centre slides 12 along last velocity over the 1 s fuse, marked endpoint throughout. Lead the travelling spot into the swarm. | 22×2 | Allow one second to dry. |

## Flatbed trailer

Road-yellow, bare aluminium and white ties over dark oak. Gravel clicks on the deck. A June bug's wing case rattles like a loose washer. Orient all deck marks in world space as the camera moves.

| Kind | Slot · build | Silhouette and motion | Stats | Behaviour / player's answer | Gold | Field note |
|---|---|---|---|---|---|---|
| Gravel Tick | A · V(Skitterer) | Squared pebble with splayed feet, crab-walks between planks | 6/30/5/1 | Drifts 5 px/s toward the tailgate while chasing. Collect on the cab side of its line. | 8×1 | That chip in the paint just moved. |
| June Bug | B · R(Bat) | Split wing cases like an open book above a dangling belly | 4/41/4/1 | Wobbles into the rat's line around cargo. Step across its belly shadow. | 10×1 | Caught the late truck. |
| Tar Rat | C · R(Cat) | Black pennant tail stiff with tar, nose low, gallops | 17/38/8/3 | Straight large-class chaser. Put a wide crate corner in its path. | 18×2 | Didn't pay the toll either. |
| Loose Can | I · N | Dented horizontal cylinder, ring pull beats against the deck | 13/22/5/3 | Within 32 rocks for 0.8, outlines a 48×6 roll with one specular bounce off the first solid face. Rolls at 60 px/s along that path, stops at a second obstacle, then lies still 1, cooldown 4. Cross behind the rebound. | 18×2 | Still has a deposit on it. |
| Toll Gull | R · V(Spitter) | White elbow wing, long flat beak, hovers broadside | 12/22/6/4 | Shot aims 0.35 ahead along current rat velocity. Reverse as the beak opens. | 20×2 | Exact change, dropped from above. |
| Murder Hornet | II · F | Black waist, yellow chevron, wings stop at the top of its hover | 16/32/6/4 | Marks locked 30×5 dive lane for 0.85, traverses in 0.3 for 8, grounded for 1.2, cooldown 4. Cross the stripe, turn on the grounded hornet. | 22×2 | It signals before changing lanes. |
| Brute | H · E | Rhino beetle, single horn over massive plated shoulders | 60/16/16/10 | Heavy hitchhiker. Draw it around cargo and reclaim the narrow deck. | 45×3 | Manifest says one passenger. |
| Loose Bungee | III · V(Turnkey) | J hook dragging a loop of striped rubber | 20/25/6/4 | Crossbar tell, length, life and damage. On first crossing pulls rat 6 toward its hook, swept against solids and stopped before pits. Route around the free end. | 32×3 | Rated for twice your weight. |

## Named appointments and elites

All 24 minibosses have exactly one home. Fixed order gives every player the same four lessons in increasing complexity, including a repeatable 12:00 rehearsal for the boss. One signature per kind. For **every** row lock aim and outline the full footprint for 1 s, strike as specified, expose the body for at least 1 s, leave 4 s between the strike ending and the next tell. Draw 16–40 px along the longest axis, collision radius 6.5–7.5. Use slot HP 800/1600/2500/3400, contact 8 and signature damage 14/16/18/20. Each signature hits once per rat. Existing encounter rewards, retreat, arena checks and chest rules apply.

Shape dimensions are length × full width unless noted. Stamp patterns mark every centre during the tell and pulse in listed order. Ring gaps stay fixed, a quarter turn off locked aim. **Data** means an existing signature evaluator with different dimensions, centres or angles and new art. **New** means new collision and matching tell geometry. Existing rows retain their own evaluator.

| Floor / time | Miniboss · implementation | Silhouette | Signature after tell / answer | Field note |
|---|---|---|---|---|
| Sewer 3:00 | Corkscrew · existing | 20 px worm riding a brass auger | Trench fills 48×10 tail to tip in 0.5. Step off the axis, return to the head. | The drain has grown a tooth. |
| Sewer 6:00 | Bell Diver · existing | 22 px diving beetle under a cracked bronze bell | 10-wide ring expands to 44 at 36/s for 1.2, 60° gap. Walk toward the chip. | Listen for the missing note. |
| Sewer 9:00 | Valve Widow · data, Bobbin | 26 px wheel spider with a long wrench foreleg | Wrench sweeps a 36×5 radius arm through 100° over 0.85. Cross behind the trailing edge. | Someone left the water running. |
| Sewer 12:00 | The Mudlark · new, pincer | 32 px mud crab wearing two broken mousetraps | Two 30×5 straight claws start 28 apart and close to 4 apart in 0.65. Leave via an open tip and take its flank. Rehearses the crab's claws and the cat's committed paw. | Both traps are occupied. |
| Kitchen 3:00 | Milk Carton · existing | 18 px peaked roof and four square feet | Spills a 32×20 rectangle from its spout for 1.2. Circle the folded seam. | Three weeks past instructions. |
| Kitchen 6:00 | Pastry Wheel · data, Corkscrew | 22 px fluted coin on a fork handle | A 46×8 cut fills down the counter over 0.6. Step sideways off the pastry seam. | Makes six equal portions. |
| Kitchen 9:00 | Salt Hen · data, Foreman | 24 px china hen, three holes in its high tail | Three 12×12 salt piles fall at 0/0.3/0.6, live 0.3 each, centres 16 apart along rat heading. Turn across the row. | Season to taste. Hers. |
| Kitchen 12:00 | Pot Brush · data, Bobbin | 34 px bristle fan at the end of a jointed handle | A 42×6 arm sweeps 110° in 0.85. Cut in behind the handle. Its long reach previews the broom and the Megarat's body sweep. | Scrub until the stain runs away. |
| Lab 3:00 | Dose Pump · data, Corkscrew | 19 px glass cylinder with a plunger foot | Injection trench 40×8 fills in 0.65. Sidestep the tube. | Read the label twice. |
| Lab 6:00 | Petri Mother · data, Bell Diver | 24 px shallow dish on cilia with a missing rim | Culture front 6 wide expands to 36 over 1.2 with a 75° sterile gap. Meet the gap. | The blank sample had children. |
| Lab 9:00 | Cage Counter · data, Foreman | 26 px clicking tally wheel on a tripod | Three 12×12 reader pads pulse for 0.3 each, left, right, centre, at rat's locked position and ±15 across aim. Leave the centre before the last count. | One rat short again. |
| Lab 12:00 | Rotor Technician · data, Bobbin | 32 px triangular rotor head over a snaking cable | A 40×5 blade turns 150° in 1.1. Follow the blade near its hub. Rehearses the mower's turning radius and a cobra sweeping out of coil. | Switch off before removing tail. |
| Laundry 3:00 | Sister Snip · existing | 20 px upright shears, two finger holes apart | 36-long, 45° cone, blades close over 0.8. Walk behind the pivot. | She trims loose ends. |
| Laundry 6:00 | Bobbin · existing | 22 px spindle rat anchored to a spool | Cord 44×6 sweeps 90° in 0.7. Cross behind the moving cord. | It learned to sew in circles. |
| Laundry 9:00 | Suds Bailiff · data, Bell Diver | 27 px soap dome with one flat broken edge | Foam rim 8 wide expands to 42 in 1.2, 65° gap. Match the missing wedge while moving between washers. | All delicates report to rinse. |
| Laundry 12:00 | The Wringer Boy · new, rollers | 36 px twin rollers with a limp glove between them | Two 32×6 bars roll in parallel down a 44-long lane in 1, end to end with a 12-wide central seam. Align with the seam before they pass. Teaches the Mangle's feed line. | Please remove hands before pressing. |
| Printworks 3:00 | The Foreman · existing | 20 px rubber stamp on two heavy boots | Three 12×12 squares 16 apart along rat heading, live 0.3 each in sequence. Turn off the marked line. | Denied in triplicate. |
| Printworks 6:00 | Quoin Key · data, Sister Snip | 23 px fork of steel driven by a walking wedge | Chase frame closes over a 38-long, 55° wedge in 0.75. Step around the heel. | Locks the story in place. |
| Printworks 9:00 | Carbon Twin · data, Milk Carton | 26 px two offset sheets peeling apart | Lays a 38×20 transfer rectangle from the folded edge for 1.4. Skirt the sheet by its short edge. | Your duplicate stayed behind. |
| Printworks 12:00 | Late Plate · new, closing ring | 32 px curved printing plate with a torn registration notch | 5-wide circle centred on rat at lock shrinks from radius 32 to 5 in 1.2. A 65° break faces the nearest open aisle. Escape through the break. Teaches the Night Edition’s blank margin. | That's all the room we have tonight. |
| Flatbed 3:00 | Tarpaulin Pup · data, Milk Carton | 20 px folded tarp with four grommet feet | Unfurls a 30×18 rectangle for 1.1. Pass round the hem on solid deck. | Contents may have shifted. |
| Flatbed 6:00 | Axle Nut · data, Corkscrew | 24 px walking hex nut, black hole in its face | Rolls a 50×10 trench out along locked aim over 0.65. Move one plank aside. | Last seen under the truck. |
| Flatbed 9:00 | Chain Dog · data, Bobbin | 28 px chain loop dragging a triangular binder | Chain 36×6 swings 115° in 0.9. Slip behind the latch instead of retreating toward a pit. | Stays where it's tied. Mostly. |
| Flatbed 12:00 | The Road Atlas · data, Foreman | 34 px gull with a road map folded over its wings | Three 14×14 talon marks in a V, tip at locked rat, others 16 forward and ±14 sideways, land left/tip/right for 0.3 each. Cross a spent print, facing the next. Teaches the magpie and hawk landing marks. | Every exit has a bird on it. |

Elite uses the local H body with its original gait, a thick double shoulder stripe, and a distinct attachment visible from either side. Sewer toad wears a split valve collar, Kitchen weight a doubled pie rim, Lab cart an open crown of safety latches, Laundry golem two brass coat buttons, Printworks reel a pair of metal flanges, Flatbed Brute its existing plated crest. Keep the elite flag, ×1.6 spawn HP, triple base XP and guaranteed 5 gold. Its damage, reach and speed stay readable from the base body. Cap three elites, hold due elites within 20 s either side of every appointment.

## Director and campaign

Look up the eight rows by **theme** (Flatbed is `Theme::Trailer`). Use campaign difficulty index only for scalar curves. Replace `LEVEL_MIX`, `KIND_ORDER`, the global special list and the hardcoded Brute elite / Skitterer cleanup paths. Every spawn source, including room waves and fallback spawns, reads the same floor table. Keep reachable spawn checks, protected mazes, boss reservation and trailer pit rules.

| Clock | Attempted ordinary bodies/s | Live budget | Fodder A+B minimum | Special I+II+III pool | Elite curve |
|---|---:|---:|---:|---:|---:|
| 0–1 | 0.8→1.5 | 18 | 100% | 0 | 0 |
| 1–3 | 1.5→3 | 42 | 70% | 0 until 2:00, then 8% | 0 |
| 3–6 | 3→5 | 72 | 60% | 12% | 0→1% |
| 6–9 | 5→7 | 105 | 55% | 16% | 1→2% |
| 9–12 | 7→9 | 135 | 55% | 20% | 2→3% |
| 12–15 | 9→11 | 165 | 55% | 20% | 3→4% |
| 15+ | 1 | 35 | 100% | 0 | 0 |

Within fodder use weights A:B = 3:1. Remaining probability goes to C:R:H at 2:1:1. Special weights I:II:III = 1:1:1. In each pool normalize among unlocked members, then return each capped member's mass to fodder. Empty pools also return their mass to fodder. Count queued bodies toward all caps. Caps by slot are A/B/C up to live budget, R 4, H 8 ordinary, I 5, II 4, III 3. Big Snake caps at 2. Count a squad, segment chain or swarm as one body with one payout. At most two nearby ordinary warned attacks may start within 64 px. Cosmetic tells cannot hide an active hazard. Reject a ground signature that blocks every walkable exit, counts pit as safe floor, or begins offscreen without a visible target-side tell.

From 7:15 to 8:15 double II's special weight, then double I's until 9:45. At 9:45 every floor starts a repeating 30 s I → II → III rotation, doubling only the featured weight inside its pool. Thus Sewer cycles Pillbug / Sump Leech / Big Snake, Kitchen Egg Timer / Porter / Sink Crab, Lab Walking Syringe / Lilliputians / Turnkey, Laundry Peg Snapper / Bell Flea / Wick Moth, Printworks Staple Crawler / Termite Horde / Ink Blot, Flatbed Loose Can / Murder Hornet / Loose Bungee. Combined special selection probability never exceeds 20%. From 12:00 double C and B within their respective pools for 30 s, then double H for 30 s, repeating. At 15:00 existing enemies stay, and new spawns use only local A/B at cleanup pace. Halve attempted rate for 20 s after each appointment, suppress fresh spawns for 8 s after its death.

Keep each stage's opening clock at zero, the 25 s boss warning, ordinary HP ×1→2 by 15:00 and the existing movement ramp topping at ×1.75 at 5:00. Keep current difficulty-slot speed/damage ramps and additional ordinary HP (0/5/10% by 15:00, starting after 3:00). Retain the current difficulty slot for the trailer. Comparing routes, inspect XP collected, net shop gold, incoming damage and how long the rat spends cut off from gems. Use the same seed, input and kept loadout for sibling floors. Target first choice by 12–20 s, 9–12 picks by 3:00, 2,300–3,400 collected XP at 12–14 and 50–56 picks by 14:00 on a surviving first-stage build. Tune individual weights, XP and gold before touching the common clock. Compare a starter Gnaw run and a carried build so a resistant swarm or body wall still has a movement-only answer.

## Work packages and cuts

| Package | Deliverable and merge boundary |
|---|---|
| Registry and director first | One integrator owns shared enum appends, stable save IDs, floor modules and their registration hooks. A static kind definition binds stats, radius/nav class, role, unlock, caps, weights, gold, steering/attack, art and LOD, procedural SFX/death cue, bestiary text and bot hazard/response. Existing IDs remain readable in old saves. Validate exactly eight unique ordinary IDs and four unique appointments per theme. |
| Shared encounter geometry first | Parameterize trench, cone, cord, ring, stamps and spill. Share the same geometry between render, collision and bot, including target lock, walkable escape, once-per-strike hit latch and recovery. Give floor modules a signature hook for the three new shapes. Allocate timer and geometry storage at setup. Retain one ordered ECS chain and resolve competing starts by SpawnId. |
| Six ordinary packages | One floor owner per table, in its own module. Wire its eight definitions and floor-specific rules through the registry. Consume the five in-flight kinds through their owning APIs. Store formation poses on their owner, with fixed-size segment/crew storage. Capped picks use a single weighted draw from stable order, no retries or extra RNG for absent kinds. |
| Six art/sound packages | One art owner per floor, separate module from simulation. Deliver normal and far poses, elite attachment, approach sounds, charge/impact cues and named silhouettes from that floor's table. Compare idle, movement, locked tell and live hit shape over actual floor and props at normal and far zoom, including grayscale with sound muted. |
| Six miniboss packages | Each owns only its floor's four appointment definitions, signature data and narrow tests. Sewer supplies pincer, Laundry paired rollers, Printworks closing ring. Kitchen, Lab and Flatbed use the shared shape evaluators. Keep slot scaling, reachable approach, chest guarantee and retreat in shared encounter code. |
| Integration and balance | Exercise the normal campaign start, branch choice and trailer entry. Check first-six approach, all unlock boundaries, caps including queued spawns, named order, protected exits, rewards and boss cleanup by theme. Check seed twins, bot knowledge for lock/recovery, old-save IDs and injected-store isolation. Measure full routes together after integration. |

**Inventory.** 48 exclusive ordinary kinds, eight per floor: ten existing, five in flight and 33 wave-four additions. Of those 33, 14 reuse a behaviour intact, 13 add one rule and six need a new behaviour (Leech, Timer, Syringe, Peg, Staple, Can). The five in flight are additional behaviour work already assigned. Of 24 exclusive minibosses, six exist, 15 need new shape data and art, three need new evaluators. New art scope is **51 body designs** (33 ordinary + 18 named), each needing a far-zoom pass, plus five new elite attachments beside the existing Brute treatment. Budget the five incoming ordinary bodies and their far art separately if they miss their current delivery. Procedural animation and tell poses are work within each design. No new final bosses in this budget.

**Cut order.** First simplify pose counts, decorative movement and custom impact sounds. Keep body silhouettes, attack cues and both opening sounds. Next replace new miniboss evaluators with unused shapes on the same floor (Mudlark cone, Wringer Boy spill, Late Plate cord), retaining 24 distinct names and bodies. Then defer III per floor and rotate I/II at 9:45 during rollout. If deliveries must be staged, ship the six fodder pairs as two batches, Sewer/Kitchen/Lab first, then Laundry/Printworks/Flatbed. Until a floor has all four local appointments and its core roster ready, keep its old director behind the previous-content switch. Mark the exclusive-roster rollout complete only when all six tables are live.
