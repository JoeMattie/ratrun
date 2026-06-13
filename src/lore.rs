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
             you fed — to something with a bigger mouth.",
        ),
        Theme::Lab => (
            "LEVEL 3 — THE LAB",
            "Where you were made. The lights hum a number that used to be your \
             name. End it here. Walk out the front door.",
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
    }
}

/// Flavor line for dying.
pub const DEFEAT: &str =
    "SUBJECT R-47 — REACQUIRED. The protocol logs another success. Somewhere, a fresh cage is prepared.";
