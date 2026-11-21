//! Pixel art for the two wave-3 weapons that live on the floor and on bodies:
//! **Acid Rain**'s marked, falling drops and **Hantavirus**'s infection pips.
//!
//! Both are deliberately small, flat and legible at two half-block pixels: a
//! floor mark that never moves (so the player learns *where* the splash will
//! land) and a three-pip counter over an infected body (so the player learns
//! *when* the chain jumps). A Neon art pass replaces the shapes, not the
//! contract: `draw_acid_drop` gets the drop entity + its screen position, and
//! `draw_infection_pips` gets a body position, radius and pips left.

use crate::game::ecs::components::AcidDrop;
use crate::game::weapon::RAIN_FALL;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Bright lime core of a drop (the weapon colour, lifted towards white).
const CORE: Rgb = (240, 255, 190);
/// The descending strokes' body.
const STROKE: Rgb = (198, 255, 72);
/// Plum infection pip, and the dimmer "already spent" socket.
const PIP: Rgb = (236, 156, 224);
const PIP_DIM: Rgb = (86, 46, 82);

/// Draw one locked Acid Rain drop at screen position `s`.
///
/// Three reads, in the design's order:
///
/// 1. **The mark** — a dashed lime ring on the floor for the whole wind-up, so
///    the drop's landing spot is visible before anything falls.
/// 2. **The fall** — in the last [`RAIN_FALL`] seconds, three short vertical
///    strokes descend above the mark, the lowest one brightest.
/// 3. **The hit** — the last couple of frames brighten the ring, which reads
///    with the splash shockwave as one motion.
pub fn draw_acid_drop(pb: &mut PixelBuffer, d: &AcidDrop, s: (i32, i32)) {
    let ring_r = (d.radius.round() as i32).clamp(3, 12);
    // `1 - delay/RAIN_FALL`: 0 while only the mark shows, 1 at impact.
    let fall = ((RAIN_FALL - d.delay) / RAIN_FALL).clamp(0.0, 1.0);
    let dim = 0.55 + 0.45 * fall;
    // Run order gives each drop in the volley its own dash phase, so four drops
    // over one spot still read as four drops.
    let phase = d.index as i32;

    // 1. The dashed floor ring: stronger as the drop gets close.
    for dy in -ring_r..=ring_r {
        for dx in -ring_r..=ring_r {
            let d2 = dx * dx + dy * dy;
            if d2 > ring_r * ring_r || d2 < (ring_r - 1) * (ring_r - 1) {
                continue;
            }
            // Every other cell on the ring: dense enough to read as a ring at
            // 120 x 92, open enough not to read as a solid pool.
            if (dx + dy + phase).rem_euclid(2) != 0 {
                continue;
            }
            pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(STROKE, 0.42 + 0.40 * dim));
        }
    }
    // The mark's centre: a flat lime cross that only shows once the fall is
    // under way (before that the ring alone is the tell).
    if fall > 0.0 {
        let c = palette::scale(CORE, 0.45 * fall);
        pb.plot_add(s.0, s.1, c);
        pb.plot_add(s.0 - 1, s.1, palette::scale(c, 0.6));
        pb.plot_add(s.0 + 1, s.1, palette::scale(c, 0.6));
        pb.plot_add(s.0, s.1 - 1, palette::scale(c, 0.6));
        pb.plot_add(s.0, s.1 + 1, palette::scale(c, 0.6));
    }

    // 2. The three descending strokes. They hang as a group above the mark
    // and close the gap as `fall` grows; each is a 3 px vertical streak, the
    // middle one a step lower (the volley's own stagger), with a bright head
    // on the leading drop.
    if fall > 0.0 {
        let drop_y = s.1 - (3.0 + (1.0 - fall) * 12.0).round() as i32;
        for k in 0..3i32 {
            let x = s.0 + (k - 1) * 2;
            // The stagger follows the volley order, so a stack of drops reads
            // as several staggered falls instead of one thick column.
            let y = drop_y + (((k + phase) % 3) - 1);
            let bright = (0.35 + 0.65 * fall).clamp(0.1, 1.0);
            let c = palette::scale(STROKE, bright);
            pb.plot_add(x, y, c);
            pb.plot_add(x, y - 1, palette::scale(c, 0.75));
            pb.plot_add(x, y - 2, palette::scale(c, 0.5));
            if k == 1 {
                // The head: one hot pixel at the bottom of the group.
                pb.plot(x, y + 1, palette::scale(CORE, bright));
            }
        }
    }

    // 3. Impact frames: the ring flares white-hot.
    if fall >= 0.92 {
        pb.ring(s.0, s.1, ring_r, palette::scale(CORE, 0.7));
    }
}

/// Draw the three Hantavirus pips above a body at screen position `s`.
///
/// `ticks_left` pips are lit and the rest are empty sockets, so the countdown
/// is readable at a glance: 3 at the tag, then one gone per tick, and the pips
/// vanish with the mark on the third.
pub fn draw_infection_pips(pb: &mut PixelBuffer, s: (i32, i32), radius: f32, ticks_left: u8) {
    if ticks_left == 0 {
        return;
    }
    let lit = ticks_left.min(3) as i32;
    // Sit just over the body's own art (legs/wings overhang the hit radius),
    // as three 2-px pips with a 1-px gap: a wide-enough bar to read over a
    // bright bug or a churning horde.
    let y = s.1 - (radius.round() as i32 + 8);
    let x0 = s.0 - 3;
    // A one-row dark plate under the pips: a status pip has to survive being
    // drawn over a bright bug, a flash or a particle burst.
    for x in (x0 - 1)..=(x0 + 7) {
        pb.plot(x, y, (20, 10, 18));
    }
    for i in 0..3i32 {
        let x = x0 + i * 3;
        if i < 3 - lit {
            // Already spent: a hollow socket, dim enough not to read as a body.
            pb.plot(x, y, PIP_DIM);
            pb.plot(x + 1, y, PIP_DIM);
        } else {
            // Dark backing first, so the pip survives a bright body or a
            // particle burst right under it.
            pb.plot(x, y + 1, (26, 12, 24));
            pb.plot(x + 1, y + 1, (26, 12, 24));
            pb.plot(x, y, PIP);
            pb.plot(x + 1, y, PIP);
            pb.plot_add(x, y - 1, palette::scale(PIP, 0.35));
        }
    }
    // A one-pixel thread down to the body so the pips belong to it, not to
    // the floor.
    pb.plot_add(s.0, y + 2, palette::scale(PIP, 0.45));
}
