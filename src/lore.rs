//! Narrative content for Rat Run.
//!
//! The thread: you are **Subject R-47**, a lab rat that chewed out of its cage
//! in a sprawling research facility. The building's automated extermination
//! protocol has triggered. Claw up from the Sewers, through the Kitchens, to
//! the Lab core — and get out.

use crate::game::level::Theme;

pub const TITLE_BLURB: &str = "Subject R-47 has escaped. The facility wants it back.";

/// Intro crawl shown from the title via the Story screen.
pub const INTRO: &[&str] = &[
    "SUBJECT R-47",
    "STATUS: ESCAPED",
    "",
    "You gnawed through the bars of Cell Block C.",
    "Somewhere above this place: the surface. Air. Freedom.",
    "",
    "But the facility woke up when you did.",
    "EXTERMINATION PROTOCOL — ENGAGED.",
    "",
    "Every drone, trap, and failed experiment is hunting you now.",
    "You have teeth. You have nerve. You have nothing left to lose.",
    "",
    "Claws out, rat. Run.",
];

/// (heading, flavor) shown on the pre-run intro card for each map.
pub fn map_intro(theme: Theme) -> (&'static str, &'static str) {
    match theme {
        Theme::Sewer => (
            "SECTOR 01 — THE SEWERS",
            "Cold pipes and colder things. The drainage threads under the whole \
             facility. Follow the current up and out.",
        ),
        Theme::Kitchen => (
            "SECTOR 02 — THE KITCHENS",
            "Grease, snap-traps, and the stink of bait. Everything here wants \
             you fed — to something with a bigger mouth. Nobody agrees who \
             keeps this kitchen: the caretaker with the broom, or the thing in \
             the cold room that was the first of us.",
        ),
        Theme::Lab => (
            "SECTOR 03 — THE LAB",
            "Where you were made. The lights hum a number that used to be your \
             name. End it here. Walk out the front door.",
        ),
        Theme::Laundry => (
            "SECTOR 02B — THE LAUNDRY",
            "Indigo tile, steam between the machines, and a hundred doors that \
             will not open. The whites come out pink down here.",
        ),
        Theme::Printworks => (
            "SECTOR 03B — THE PRINTWORKS",
            "Ink in the seams of your paws and the presses still running \
             overnight. Tomorrow's front page says you were caught.",
        ),
        Theme::Trailer => (
            "THE GETAWAY — THE FLATBED",
            "Forty feet of wet deck doing eighty into the dark. The cab is \
             forward. So is the magpie, and it thinks the rig is its nest.",
        ),
    }
}

/// Payoff line for surviving the run on a given map.
pub fn victory(theme: Theme) -> &'static str {
    match theme {
        Theme::Sewer => {
            "The pumps shudder and die. Daylight leaks down a storm drain. \
             You climb toward it, whiskers first."
        }
        Theme::Kitchen => {
            "The pilot lights gutter out one by one. A back door propped on a \
             mop handle opens onto the night air."
        }
        Theme::Lab => {
            "Every alarm flatlines at once. The blast door you were born behind \
             swings outward — and you are already gone."
        }
        Theme::Laundry => {
            "After thirty years, the laundry stops. The iron drum ticks as it \
             cools; the service stair stands unlatched."
        }
        Theme::Printworks => {
            "Correction: subject still at large. The final edition curls under \
             the street door, and you are out with it."
        }
        Theme::Trailer => {
            "The magpie drops astern in the road roar. At the next shoulder \
             you hop the tailgate onto warm, open asphalt."
        }
    }
}

/// The Kitchen's second boss, **SUBJECT R-01**: the bar's name and class tag.
///
/// The first success: the rat the facility made before it made R-47, kept in
/// the kitchens ever since, collared and tagged like a trophy. The player is
/// R-47. The mirror is the whole fight.
pub const MEGARAT_NAME: &str = "SUBJECT R-01";
pub const MEGARAT_CLASS: &str = "CLASS-V PROTOTYPE — FIRST SUCCESS";

/// The arrival banner's flavour line for SUBJECT R-01 (the plate's second row
/// when the terminal has room, and the fight's own epitaph at the clear).
pub const MEGARAT_ARRIVAL: &str =
    "It has been in the walls since before your cage. It has won this fight before.";

/// Flavor line for dying.
pub const DEFEAT: &str =
    "SUBJECT R-47 — REACQUIRED. The protocol logs another success. Somewhere, a fresh cage is prepared.";

// ============================================================================
// The Sewer's pool mate: THE HORSESHOE
// ============================================================================

/// Name plate for the horseshoe crab (the Sewer's second possible boss).
///
/// The other bosses are named by their campaign floor
/// (`render::campaign::CampaignLevel::boss_name`); the crab shares the Sewer
/// with the Apex Jaguar, so its plate is its own.
pub const HORSESHOE_NAME: &str = "THE HORSESHOE";

/// Classification tag beside the name plate, in the same voice as the campaign
/// plates ("CLASS-V APEX PREDATOR", "MK.IV AUTONOMOUS UNIT").
pub const HORSESHOE_CLASS: &str = "PRE-PIPE — STILL HERE";

/// The field note the drain rats pass around. It is the boss's lore: something
/// older than the facility crawled up the outflow and has not left.
pub const HORSESHOE_FIELD_NOTE: &str = "It was here before the pipes were. It will be here after.";

// ============================================================================
// The Flatbed's pool mate: THE REDTAIL
// ============================================================================

/// Name plate for the hawk (the getaway ride's second possible boss).
///
/// The magpie is the trailer's thief; this is the thing that has been following
/// the truck. Its plate carries its own class tag like the crab's and the
/// megabat's, because the flatbed is not a campaign floor.
pub const HAWK_NAME: &str = "THE REDTAIL";

/// Classification tag beside the name plate. While the bird is high the plate
/// swaps this for `art_boss_hawk::OVERHEAD_TAG`, so the rat is told why its
/// weapons are finding nothing.
pub const HAWK_CLASS: &str = "CLASS-V STOOPING PREDATOR";

/// The field note on the deck's wind-blasted noticeboard.
pub const HAWK_FIELD_NOTE: &str = "Nothing on this deck is bigger than its shadow.";

// ============================================================================
// The Lab's other specimen: KING COBRA
// ============================================================================

/// Name plate for the Lab's second boss.
///
/// The Lab's original fight is the Turf-Reaper Mk.IV, so the king's plate is
/// its own: a specimen number the facility stopped saying out loud.
pub const COBRA_NAME: &str = "KING COBRA";

/// Classification tag beside the name plate, in the same voice as the campaign
/// plates ("CLASS-V APEX PREDATOR", "PRE-PIPE — STILL HERE").
pub const COBRA_CLASS: &str = "CLASS-VII SPECIMEN — LOOSE";

/// The arrival banner's flavour line (the plate's second row, and the fight's
/// own epitaph at the clear).
pub const COBRA_ARRIVAL: &str =
    "The lab grew it long. The lab grew it longer than the room it was kept in.";

/// The field note on the specimen card.
pub const COBRA_FIELD_NOTE: &str =
    "Twenty-two feet of it, and the room it was kept in was eleven. That is how it got out.";
