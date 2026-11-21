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
            "LEVEL 1 — THE SEWERS",
            "Cold pipes and colder things. The drainage threads under the whole \
             facility. Follow the current up and out.",
        ),
        Theme::Kitchen => (
            "LEVEL 2 — THE KITCHENS",
            "Grease, snap-traps, and the stink of bait. Everything here wants \
             you fed — to something with a bigger mouth. Nobody agrees who \
             keeps this kitchen: the caretaker with the broom, or the thing in \
             the cold room that was the first of us.",
        ),
        Theme::Lab => (
            "LEVEL 3 — THE LAB",
            "Where you were made. The lights hum a number that used to be your \
             name. End it here. Walk out the front door.",
        ),
        Theme::Laundry => (
            "LEVEL 2 — THE LAUNDRY",
            "Indigo tile, steam between the machines, and a hundred doors that \
             will not open. The whites come out pink down here.",
        ),
        Theme::Printworks => (
            "LEVEL 3 — THE PRINTWORKS",
            "Ink in the seams of your paws and the presses still running \
             overnight. Tomorrow's front page says you were caught.",
        ),
        Theme::Trailer => (
            "THE GETAWAY — FLATBED",
            "Forty feet of oak deck and a hundred klicks of night, and the road \
             roaring underneath your paws. The cab is forward. So is the magpie: \
             it has been riding the slipstream over the cab since the last weigh \
             station, and it has already decided the cab is its nest.",
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
            "The pilot lights gutter out one by one. A back door, propped on a \
             mop handle, opens onto a sliver of night. You take it."
        }
        Theme::Lab => {
            "Every alarm flatlines at once. The blast door you were born behind \
             finally swings outward — and you are already gone."
        }
        Theme::Laundry => {
            "For the first time in thirty years, the laundry stops. The drum \
             ticks as it cools, and the service stair beyond it is unlatched."
        }
        Theme::Printworks => {
            "Correction: subject still at large. The last edition rolls off the \
             belt, curls under the street door, and you are out with it."
        }
        Theme::Trailer => {
            "The magpie's last shriek is carried off down the deck with a \
             hundredweight of road noise. The cab's brake lines pop and the rig \
             starts to drift. You leave it on the hard shoulder, drop off the \
             tailgate, and the asphalt is still warm. The facility is behind you \
             — all of it, at last."
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
