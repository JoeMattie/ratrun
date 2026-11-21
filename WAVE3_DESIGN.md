# Rat Run · Subject R-47's route to the surface

## 1. Fifteen minutes on the floor

At 0:00 the rat has Gnaw and enough empty floor to turn back for its first gem. At 3:00 it should recognize a particular enemy by its silhouette. At 12:00 it should be steering a build it chose. At 15:00 the clock reads BOSS. Killing the themed boss clears the floor. Keep the current boss hunt and death sequence. A rat miles from the arena still gets a fight.

Initial tuning targets for an ordinary first stage follow. Use `director::RUN_SECONDS = BOSS_TIME = 900` and `MINIBOSS_TIMES = [180, 360, 540, 720]`. Keep the existing 25 s boss warning. Rate means attempted ordinary bodies per second, including room waves. Live budgets count the whole map. Interpolate rates between row endpoints. Skip overspent pulses. Reserve named encounter slots separately.

| Clock | First appearances and mix | Bodies/s | Live budget | Elite chance per ordinary spawn | Build milestone by end |
|---|---|---:|---:|---:|---|
| 0–1 | Skitterers. First Bat at 0:45. Send the first six on one approach. | 0.8→1.5 | 18 | 0 | First card in 12–20 s, 3–4 picks by 1:00 |
| 1–3 | Cats at 1:15, Pillbugs at 2:00. At least 70% Skitterer/Bat. | 1.5→3 | 42 | 0 | 9–12 picks, usually three weapons |
| 3–6 | Spitters at 3:35, Bell Fleas at 4:30. At least 60% Skitterer/Bat. | 3→5 | 72 | 0→1% | 21–25 picks, first evolution usually ready |
| 6–9 | Brutes at 6:35, Wick Moths at 7:15, Porters at 8:15. At least 55% Skitterer/Bat. | 5→7 | 105 | 1→2% | 32–37 picks, both racks nearly full |
| 9–12 | Add Turnkeys at 9:45. Cycle the three newest kinds as featured threats. | 7→9 | 135 | 2→3% | 43–48 picks, two or three evolved weapons possible |
| 12–15 | All unlocked. Every 30 s alternate a Cat/Bat chase and a heavier Brute-led pack. | 9→11 | 165 | 3→4% | 50–56 picks by 14:00, usually five weapons and five passives |
| 15+ | Boss enters, ordinary reinforcements fall to Skitterer/Bat cleanup pace. Existing enemies keep moving. | 1 | 35 | 0 | Victory on boss death, no time-out |

Split the fodder share 3:1 Skitterer to Bat once Bats unlock. Split the unassigned share evenly among other unlocked kinds (55% fodder after 9:00). If a category is empty or capped, give its spare share to fodder. Limit all five new kinds combined to 20% of spawns. From 9:45 rotate Wick Moth, Porter and Turnkey every 30 s, giving the featured kind twice its usual weight within that share. Cap simultaneous Spitters at four and elites at three. Keep the existing room-wave path and protected maze rules. Give the player 20 s at half spawn rate after each miniboss arrives and 8 s without fresh spawns after its death. A cleared doorway is a chance to fetch gems. Keep ordinary HP scaling shallow (×1 at start to ×2 at 15:00), speed and contact damage at base on stage one. Stages two/three add 5/10% ordinary HP after 3:00, with the same opening grace.

**XP budget.** Charge 6 XP for the first card, then add 2 XP to the cost of each successive card up to 100. Keep ordinary XP values (1/1/3/4/10 for Skitterer/Bat/Cat/Spitter/Brute). Use the new-kind values in §9. Elites give three times their kind's XP, minibosses give 25/40/55/70 XP. Pick 45 costs 2,250 cumulative XP, pick 52 costs 2,944. Aim for 2,300–3,400 collected XP at 12–14 minutes without a maze, shop or XP splice. Five L5 weapons cost 24 picks from starting Gnaw. Five typical passives need another 22–32. Count chest upgrades toward build progress separately from XP. Tune gem yield first if a surviving player's builds lag these milestones.

**Campaign.** Three stages, roughly 45 minutes plus fights. Stage one is Sewer. At the keep screen choose Kitchen or Laundry for stage two, then Lab or Printworks for stage three. Seed each choice from run seed, stage and theme. Keep the original route as the first menu choice. The final boss guards a street-level exit on both routes. After each clear keep up to three owned kinds at current ranks, including any earned evolution. A kept evolution operates even if its partner stays behind. Grant L1 Gnaw if no weapon was kept. Reset XP and player level, refill HP and dash, carry gold and splices outside the racks, discard field pickups and timed buffs. A player who built a favorite weapon can spend the next floor discovering a different partner.

## 2. Five paws' worth of gear

Cap distinct kinds at five weapons and five passives independently. Build three distinct offers from legal cards. When possible, deal one owned rank-up, one new kind, then one other legal card. Prioritize an unowned evolution partner for an owned L4/L5 weapon on the new-kind card. Break competing candidates with the seeded choice draw. The first level-up includes a second weapon. Show the partner's name and owned mark on weapon cards.

At five of a category, offer only ranks in owned kinds from that category. Both full still yields rank-ups. Exhaust live upgrades first, then fill spare places with at most one Bandage (+25% max HP) and one Purse (+8 gold). Hide Bandage at full health. Show fewer cards if needed. At full ranks and full HP offer just the Purse. Both consumables leave slots untouched.

## 3. Chest evolutions

Every row takes its L5 weapon plus **one stack** of the listed passive at chest opening. The weapon changes in its own slot, retains its timing family and credits damage to the original weapon. Lock the evolved flag once. Base damage below means the weapon's L5 hit after stats. Keep attack silhouettes legible in two rows of half-blocks and draw hostile warnings above friendly effects.

| Weapon + passive | Chest name | New behavior | Half-block read | Chest inscription |
|---|---|---|---|---|
| Gnaw + Sharp Teeth | Milk Teeth | The lead dart lodges for 1 s, then pulls back to the rat's current position, biting each crossed enemy once at base damage. | Ivory hook and dotted return thread | They always come back in. |
| Cheese Spray + Fat Reserves | Fondue Fork | The outer two darts plant tines on hit or at 40 px. A cheese string between them lasts 1.5 s and cuts each crosser once. | Yellow posts, single sagging line | Dinner is a tripwire. |
| Squeak Nova + Frenzy | Echo Chamber | Each spoke that hits a wall returns along its incoming line, damaging once on the way back. One reflection per spoke. | Violet outward dots become hollow white inward ticks | The walls know the chorus. |
| Spore Orbit + Resilience | Fairy Ring | Every second orbit cycle leaves its satellites fixed around the casting point for 3 s. Crossings of the ring deal one hit per target per second. | Mint mushrooms outlining an empty circle | Something grew where you stood. |
| Tail Whip + Quick Paws | Ratchet Tail | A hit hooks the nearest ordinary target, swinging it 90° around the rat into others for one hit each, then releases it. Heavy targets take the whip hit only. | Pink hook, one brown body on a bright quarter arc | Give the room a turn. |
| Acid Spit + Thick Hide | Drain Cleaner | The splash runs along the firing direction in a 36 × 6 px corrosive channel over 0.6 s. Pool ticks follow the stripe for 2 s. Stop at masonry. | Bile-green advancing chevrons | Clears hair, grease and witnesses. |
| Tail Thrash + Counterweight | Flywheel | Each manual sweep catches up to two hostile pellets. The final sweep flings the captured pellets forward at one base hit apiece. | Copper pips on the tail, then a copper fan | Save that for the return trip. |
| Sonic Squeak + Whiskers | Roll Call | The active ring tags stunned or slowed targets. After 0.8 s each answers with one 9 px damaging pulse at half base damage, capped at three answers hitting the same body. | Cyan quotation marks wink shut in sequence | Every cage answers. |
| Acid Rain + Long Fuse | Downspout | Replace separate splashes with a marching gutter. Strike the first marked target, then step each remaining drop 10 px toward the rat along the same line. Leave 1 s of burning runoff. | Slanted drops become a stepped green staircase | The leak has found the stairs. |
| Hantavirus + Second Scent | Dead Letter | A host dying with jumps left leaves a sealed packet for 2 s. The first eligible foe within 10 px opens it and resumes the chain. One packet per chain. | Magenta envelope shuts, then tears open | Return to sender. |
| Laser Eyes + Lens Cap | Carbon Copy | Record the facing and origin of a completed sweep. On the next sweep a fixed afterimage at that spot repeats it backwards for half damage. Keep one copy. | Amber eye pair stays behind, beams cross | The camera kept a negative. |
| Super Dash + Spring Heel | Ripcord | After the dash, its actual route stays taut for 0.6 s, then snaps. Enemies on it take one dash hit and slide 8 px toward the departure point. Bosses take damage only. | White knots pull down a blue cord | The tail was the fuse. |

## 4. Four new weapons

Values in every five-entry row run L1 → L5, before passive modifiers. Hit damage is per listed hit. The numbers already include level scaling. Target by nearest eligible visible body, break ties by SpawnId, stop hostile and friendly attack geometry at solid cover. Bosses use their damageable hit shapes. Use fixed-size hit histories. These four fill ground targeting, pack transfer, aimed sweep and escape damage.

| Weapon | Damage | Cycle in seconds | Size / count by rank | Rule, tell and flavor |
|---|---|---|---|---|
| Acid Rain | 8 / 10 / 12 / 14 / 16 per splash | 2.8 / 2.6 / 2.4 / 2.2 / 2.0 | 2 / 2 / 3 / 3 / 4 drops, radius 7 / 8 / 8 / 9 / 10 px | Mark separate foes within 72 px, nearest first. If only one exists, spend the spare drops at its marked spot. Lock floor positions, drizzle after 0.35 s at 0.12 s intervals. No lingering base pool. Three descending lime strokes. “There was a ceiling here once.” |
| Hantavirus | 3 / 4 / 5 / 6 / 7 per tick | 3.6 / 3.4 / 3.2 / 3.0 / 2.8 | 1 / 2 / 2 / 3 / 4 transfers, reach 14 / 14 / 18 / 18 / 22 px | A fictional arcade status. Tag one foe within 56 px for three ticks 0.35 s apart. On third tick or death pass remaining jumps to one fresh neighbor. A chain visits each target once, skips active marks and ends if isolated. A boss can take ticks. Three plum pips disappear in order. “Somebody passed a note in Block C.” |
| Laser Eyes | 6 / 7 / 8 / 9 / 10 per pulse | 2.0 / 1.9 / 1.8 / 1.7 / 1.6 | 3 / 3 / 4 / 4 / 5 pulses, length 34 / 40 / 40 / 48 / 54 px | Lock last facing at burst start. Space pulses evenly across −35° to +35°, 0.10 s apart, 3 px wide. Pierce every exposed body along the ray once per pulse. Twin amber muzzle dots and a blinking bar leave dark intervals. “Please look directly at the sensor.” |
| Super Dash | 14 / 20 / 26 / 32 / 38 per crossed body | On each successful normal dash, base 2.0 s recharge | Swept radius 4 / 4 / 5 / 5 / 6 px | Only traveled segments deal damage, each target once per dash, including Ctrl and click-dash. A blocked or cooling dash buys no hit. Keep dash motion and immunity. Frenzy affects weapon timers only. Blue-white comb through the crossed bodies. “Exit wound, entrance wound, exit.” |

## 5. Passive rack

Keep existing stack arithmetic. New percentage bonuses add per stack against base unless a rule gives a multiplier. All five new passives work without their partner.

| Passive | Cap | Each stack | Card line |
|---|---:|---|---|
| Fat Reserves | 8 | +20 max HP and heal 20 | Winter is always coming. |
| Quick Paws | 6 | Move speed ×1.12 | Twitchier than the cameras. |
| Sharp Teeth | 8 | +15% base damage | Filed sharp on rebar. |
| Frenzy | 8 | +12% base fire rate | More is, itself, a strategy. |
| Whiskers | 4 | Pickup radius ×1.30 | Feel the room before you enter it. |
| Resilience | 6 | +0.5 HP/s | Bred to survive worse than this. |
| Thick Hide | 6 | +1 flat armor | The first coat was laboratory issue. |
| Spring Heel | 5 | Subtract 0.18 s from base dash cooldown (2.0 → 1.1 s) | Smuggled out one spring at a time. |
| Counterweight | 4 | +15% weapon shove, +2 px outward shove on ordinary melee hits with zero native shove (once per target per 0.4 s) | A washer tied to the tip. |
| Long Fuse | 4 | +12% lifetime to friendly lingering zones and orbiters, +4% splash/radial radius | Let it simmer. |
| Second Scent | 4 | +10% gem XP, bank fractional XP across collections | There's a second crumb under the first. |
| Lens Cap | 4 | +8% reach of ranged weapons, +4% reach of melee and ring weapons | Cut from the bottom of a bottle. |

## 6. Gold and the shop in the wall

Coins use gem attraction and a distinct square glint. Use a gold-only seeded roll keyed by floor seed and victim SpawnId to preserve existing drop rolls. One roll per eligible kill, one coin entity carrying its whole value. Elite payout replaces its kind's roll. Gold belongs to the current campaign.

| Enemy | Chance × gold on success | Enemy | Chance × gold on success |
|---|---|---|---|
| Skitterer | 8% × 1 | Bat | 10% × 1 |
| Cat | 18% × 2 | Spitter | 20% × 2 |
| Brute | 45% × 3 | Pillbug | 18% × 2 |
| Bell Flea | 14% × 2 | Wick Moth | 22% × 2 |
| Porter | 40% × 3 | Turnkey | 32% × 3 |
| Elite | 100% × 5 | Miniboss | 100% × 12, alongside chest |
| Full boss | 100% × 30, credit at clear | | |

Place **two** shop holes per level in structural wall faces on public, reachable floor. First one is 16–40 walking cells from spawn, facing a fork on the way to an ordinary room. Put the second along a different approach toward the boss arena. Search nearest valid face by walking distance if the ideal band fails. Leave regular shortcut holes intact. A bent spoon over the opening and three white whisker scratches at the fork identify the shop. Reveal its minimap spoon only on sight, then remember it. Touch its threshold to pause the simulation, step clear to re-arm. Exit grants 0.8 s grace. “Under the spoon, the cameras have a blind spot.”

Each hole stocks three distinct seeded splices and the services below. Generate stock from floor seed, hole index and reroll count when the shelf first opens. Inventory persists on revisit. Splices last until campaign end, occupy neither rack and can each be bought once. Two splice sockets total. Show both stat changes before payment. Apply splice multipliers after passive stats. Always keep at least 1 max HP, cap current HP at the new max, and keep dash cooldown ≥0.7 s. Expected first-floor purse at 3:00 is 20–45 gold including the miniboss. The affordable heal gets an injured rat through the next fight. A saved purse buys a build change.

| Splice | Gold | Permanent bargain | Label on the vial |
|---|---:|---|---|
| Glass Incisors | 35 | ×1.22 weapon damage, ×0.82 max HP | Handle by the roots. |
| Boiler Heart | 32 | +1 HP/s regen, ×0.90 move speed | Keeps warm in cold pipes. |
| Hollow Bones | 28 | ×1.15 move speed, +3 incoming damage before armor | Mind the landing. |
| Double Lid | 30 | +0.22 s post-hit immunity, ×0.88 fire rate | Blink once for each world. |
| Hoard Gland | 30 | +50% coin value (bank halves), ×0.90 damage | The left cheek is for rent. |
| Copper Nerves | 38 | ×0.80 dash cooldown, ×0.75 pickup radius | Ground before handling. |
| Night Shift | 34 | ×1.18 fire rate, halve total passive HP/s | Sleep was the control group. |
| Blackboard Belly | 36 | +25% gem XP, ×0.75 healing from items and shop | Everything goes in the notebook. |
| Ballast Sac | 28 | +3 armor, ×1.15 dash cooldown | A little sand steadies the hand. |
| Long Fingers | 24 | ×1.60 pickup radius, ×0.90 max HP | Someone has to reach the shelf. |

| Service | Gold | Purchase |
|---|---:|---|
| Stitch | 12 | Heal 35% max HP, once per hole. Disable at full HP. “Hold still for six stitches.” |
| Turn the shelf | 8, then 16 | Replace unsold splices with eligible unseen stock, at most twice per hole. Preview price. “Look behind the tins.” |
| Molt | 22 | Once per hole, nominate one owned kind and choose a different unowned kind in the same category at rank 1. Display forfeited ranks and evolution. Recompute stats and clamp HP. Cancel freely before selection. “Leave the old skin here.” |

## 7. Miniboss chests

| At touch | Give immediately | Chest line |
|---|---|---|
| At least one eligible unevolved weapon | Evolve the earliest held eligible weapon in rack order. Preview partner and new attack on the paused chest card. | The lock fits your teeth. |
| No evolution, an owned weapon or passive can gain a rank | One free rank, first preferring the highest-rank unfinished weapon, then highest-stack unfinished passive. Resolve ties in rack order. | Filed under spare parts. |
| No eligible owned rank | 24 gold and heal 25% max HP. At full health credit all gold. | Someone left lunch money. |

Exactly one chest per miniboss kill, including trap or damage-over-time kills. It stays on reachable floor until touched or the stage ends. Mark a seen chest on the minimap. Resolve accumulated level-up selections before evaluating a touched chest, then process chests one at a time. Multiple eligible weapons still cost one chest each. Cache doors keep their separate reward.

## 8. The four appointments

Named minibosses arrive at 3:00, 6:00, 9:00 and 12:00. Use distinct 12–20 px silhouettes and a short nameplate. Spawn on connected floor just outside view, approach visibly before attacking. HP by appointment is 160 / 320 / 540 / 780, regardless of species. Contact 8, signature hit 14 / 16 / 18 / 20. Start with a 1 s tell, then 4 s between signatures and at least 1 s exposed recovery. Lock aim at tell start. Telegraph on the rat's side of the screen. If the proposed shape leaves no walkable escape, reposition. A surviving miniboss retreats at the next appointment without dropping loot, so a slow build faces one named enemy at a time.

| Name | Silhouette and one signature | Player opening | Field note |
|---|---|---|---|
| Corkscrew | Brass auger under a pink worm. A dotted 48 × 10 px trench fills from tail to tip in 0.5 s. | Step sideways, bite the exposed head. | The drain has grown a tooth. |
| Sister Snip | Tall shears open over a 45° cone out to 36 px. Two blades close once across the outlined wedge. | Walk behind the pivot. | She trims loose ends. |
| Bobbin | Spindle rat reels out a 44 px cord to a fixed spool, then swings it through 90° over 0.7 s. | Cross behind the moving cord. | It learned to sew in circles. |
| Bell Diver | Bell-shaped beetle taps the floor. A ring expands to 44 px at 36 px/s with a fixed 60° gap. | Line up with its chipped rim. | Listen for the missing note. |
| The Foreman | Walking rubber stamp marks three 12 × 12 px squares along the rat's last heading, 16 px apart, then stamps each at 0.3 s intervals. | Turn the corner after the first mark. | Denied in triplicate. |
| Milk Carton | Four square feet tilt and spill a single 20 × 32 px rectangle from the printed spout for 1.2 s. | Circle to the folded seam. | Three weeks past instructions. |

| Level | 3:00 | 6:00 | 9:00 | 12:00 |
|---|---|---|---|---|
| Sewer | Corkscrew | Bell Diver | Milk Carton | Sister Snip |
| Kitchen | Milk Carton | Sister Snip | Bell Diver | The Foreman |
| Lab | Bell Diver | Corkscrew | The Foreman | Bobbin |
| Laundry | Bobbin | Sister Snip | Corkscrew | Milk Carton |
| Printworks | The Foreman | Bobbin | Sister Snip | Bell Diver |

## 9. Five ordinary enemies with tells

Stats are base HP / speed in px per second / contact damage / gem XP. New kinds share the director budget. Limit each special behavior by its own timer. At most two nearby ordinary enemies may start a warned attack together. A body waiting its turn can still move and take damage.

| Kind | Stats | Read and response | Field note |
|---|---|---|---|
| Pillbug | 18 / 23 / 5 / 3 | Ribbed silver oval. At 24 px range it curls for 1.4 s, immobile. Frontal 120° shell takes quarter damage, exposed rear takes double. Loop past its shoulder. | It brought its own door. |
| Bell Flea | 9 / 32 / 4 / 2 | Copper tuning fork. Once per 4 s, two jumps, first toward a locked point within 26 px, second back to its starting spot. Both landing crosses show for 0.65 s. Each impact deals 7 within 6 px. Step off the first cross and leave the second empty. | Even the echo has feet. |
| Wick Moth | 12 / 28 / 4 / 3 | Teardrop with a lit tail. Death leaves a wick flashing for 1 s, then a single 9 px burst for 9 to rat and 20 to enemies. Lure the pack onto the wick. Secondary moth deaths get their own full fuse. | It dies with the light on. |
| Porter | 26 / 20 / 6 / 5 | Beetle carrying a white sugar cube. Within 24 px of a wounded ordinary ally, raises the cube for 0.9 s then heals it for 8 once per 5 s. Any hit on the Porter interrupts. Step toward the carrier to redirect auto-fire. | One lump for medicinal purposes. |
| Turnkey | 22 / 24 / 6 / 4 | Key-shaped drone. Every 6 s lays one 22 px crossbar across its own previous path for 2 s after a 0.8 s dotted tell. Rat crossing takes 8 once, enemies pass freely. Bait the bar into spent floor, then change lanes. Reject placements filling an entire exit. | Nobody asked who locked the hallway. |

## 10. Pocket finds

Keep the existing ordinary/big rare-pickup drop chances (1.2%/12%). On a successful roll use weights Heal 32, Magnet 18, Nuke 8, and the four rows below. Timers use simulation time. Repeat finds refresh duration or replace the spent charge. Give each a solid two-tone silhouette, distinct from the square gold glint.

| Pickup | Weight | Effect and visual | Label |
|---|---:|---|---|
| Bottle Cap | 14 | Block the next damaging hit within 12 s, then use normal hit immunity. Tin dome pops off, a small white rim marks the charge. | Good for one boot. |
| Wind-up Mouse | 10 | A brass key spins on the floor for 5 s. Ordinary enemies within 48 px chase the stationary decoy. Elites and bosses keep their targets. | The cheapest member of the team. |
| Pocket Watch | 9 | Slow enemy bullets to 40% travel speed for 4 s. A pair of hands rotates once, slowed bullets grow a short dark tail. Keep named attack telegraph timers intact. | Borrow four seconds. |
| Clean Socks | 9 | Immediately ready the dash and ignore sticky-board drag for 6 s. Little white heel prints fade behind the rat. | Stolen from the lost and found. |

## 11. Two side routes

Use the current 16 px tile lattice, opened room sizes, connected-room validation and rare maze reservation. Choose props with solid footprints along the edges. Every hazard has a visible dry route at least two tiles wide. Teach each theme's hazard once beside the first public fork. Keep arena floors and shop thresholds clear of ordinary hazards.

| Theme | Palette and landmarks | Procedural floor and hazard | Music and arrival |
|---|---|---|---|
| Laundry | Indigo tile #22263B, faded denim #426779, porcelain #B3C7C3. Ribbed washboard walls, a brass drain at each junction, rolled towels and giant clothespins. | Pair long washer aisles around folding bays. Connect aisle ends into loops, with a cross-aisle every 6–9 tiles. Place 3–5 steam grates in optional side lanes. Three pale puffs warn for 1 s, then a 1-tile plume deals 8 once per occupant over 0.7 s, resting 4 s. Hits pursuers too. Boss floor has two open side lanes. | A brushed 6/8 drum, two low pump notes, a missed beat before steam. “The whites come out pink down here.” Boss is the Mangle. |
| Printworks | Newsprint grey #B6B4A5 against charcoal #22232A, dried ink #554064, registration cyan #4E9498. Type-block walls, reversed letters, paper reels and a clock over the press bed. | Build offset press islands with 3-tile paper lanes winding around them. Cut at least two diagonal stair-step shortcuts through each long pair of lanes. Place 3–5 ink pads at inside corners. Their permanent stipple reduces ground movement to 75% while crossed, equally for pursuers. Flying and dash traversal ignore it. | A platen clack on beats two and four, bass pauses for the bell. “Tomorrow's front page says you were caught.” Boss is the Night Edition. |

## 12. Bosses that belong on these floors

Keep the existing bosses on Sewer, Kitchen and Lab. The jaguar teaches the landing tell, the broom teaches a safe pocket, the mower teaches the value of a miss. Both additions follow that grammar. Use target-end cues, single-hit attack windows and a missed attack the rat can punish. Explicit vulnerable shapes guide auto-aim. Body contact costs 8 outside recovery. Start with 2,400 HP for the Laundry boss and 3,000 for Printworks as final encounter pools including stage scaling. Aim for 45–80 s with a near-complete build. At phase transitions spend 1.2 s on a harmless pose change. All hazards below belong to the boss and stop on its death.

| Boss / phase | Tell → attack | Safe response and payoff |
|---|---|---|
| The Mangle, above 60% | Two striped rollers on little iron feet waddle into view. The rollers turn backwards for 1.1 s, outlining a 22 × 40 px intake lane. Pull toward the rollers at 22 px/s for 1 s. A single 16-damage bite crosses the last 8 px. | Walk across the lane or outrun the pull. A towel jams the nip for 1.5 s. The loose crank at its side takes ×1.5 damage. |
| The Mangle, 60–25% | Alternate intake with a wring-out. The crank clicks twice over 0.9 s, then ejects a rolled towel along a fixed 56 × 10 px lane for 18 damage. | Peel aside and follow the towel back to the crank. One clear flank beside every attack. |
| The Mangle, below 25% | A torn belt hangs loose. Alternate towel and a two-part intake. Lock each lane separately with a fresh 0.9 s tell and 0.5 s break. Final jam lasts 2.2 s. | Change sides between bites. Strike the dropped crank until the springs pop. “For the first time in thirty years, the laundry stops.” |
| The Night Edition, above 65% | Walking platen with a square type bed and two bright keys for feet. Outline a 36 × 30 px page at the rat's current position for 1.1 s, leaving a 12 px blank margin on one side. Stamp the inked part once for 18. | Step into the blank margin or off the sheet. The lowered type bed takes ×1.5 damage for 1.4 s. |
| The Night Edition, 65–30% | Alternate page stamps with carriage return. A bell and a dotted 54 × 8 px rule warn for 1 s, then the carriage sweeps along it for 20 damage. | Cross the rule early. Follow the carriage to its stopped end for a 1.6 s exposed ribbon window. |
| The Night Edition, below 30% | Misprint. Show two pages offset 12 px for 1.2 s. Stamp them 0.55 s apart with a shared 12 px clear margin. Follow with the solo carriage move, then 2 s exposed. | Stand on the unprinted stripe for both beats. The last sheet tears loose and rolls out under the street door. “Correction: subject still at large.” |

Both bosses travel on reachable floor when the rat leaves their arena. Suppress any move whose stated safe strip hits a wall or whose target cannot see the tell. Walk to a fresh angle before retrying. Shape and timing carry the warning even with sound muted.

## 13. Cut list, in order

1. Ship the four requested weapons. Two extra guns would compete for the same ten slots and their evolution art.
2. Cut the second shop reroll and defer Hoard Gland and Blackboard Belly. Eight splices and all three services still cover distinct reasons to visit.
3. Cut Milk Carton. Use Corkscrew in its Kitchen slot, Sister Snip in its Laundry slot (swap Laundry's 6:00 to Bell Diver), and The Foreman in its Sewer slot. Five minibosses still fill all four appointments.
4. Reuse sticky-board physics and steam-tell art for theme hazards. Keep both route geometries, both bosses and their safe lanes.
5. Simplify the final boss phases to a repeat of phase two with the longer recovery. Preserve every opening tell, the 3-minute appointment schedule, chest guarantees and all twelve evolution silhouettes.
