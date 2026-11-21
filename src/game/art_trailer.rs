//! Art for the getaway ride: the flatbed trailer's deck and the road under it.
//!
//! The owner's brief: *"a level that is on the bed of a long trailer, with the
//! road flowing past you can see through cracks between the wood planks."*
//!
//! This module owns that floor, and it is deliberately **render-only**: the
//! road's scroll, the expansion-joint bump and the sliding crates are pure
//! functions of the run clock ([`Level::elapsed`] reaches the art as `elapsed`),
//! so they draw no rng, allocate nothing and cannot change a replay.
//!
//! ## Layers
//!
//! 1. **The road** ([`draw_floor`]): asphalt with lane markings, reflective
//!    studs, tar seams and skid marks, scrolling backwards at [`ROAD_SPEED`].
//!    It runs under the deck (seen through the plank gaps) and for four cells
//!    past each rail (seen over the side).
//! 2. **The deck**: oak boards running the length of the bed, with a 2 px gap
//!    between them. Grain, butt joints, knot holes, bolt heads, oil stains and
//!    scuffed ends are hashed from the plank row, so they never swim. Steel
//!    cross-members sit *under* the gaps (static: they are the trailer), and
//!    the road dashes pass behind them.
//! 3. **The holes** ([`Pit`](crate::game::level::Pit)): broken planks and
//!    missing rail sections, drawn as holes with jagged ends and the road
//!    visible below. The simulation's fall rule reads the same records, so the
//!    hole you see is the hole you fall down.
//! 4. **The load**: crates, pallets, tarped machinery, tyres, toolboxes, pipes
//!    and chains, drawn from [`PropKind`]; sliding cargo is drawn from
//!    `TrailerDeck::slides` at the offset its own cycle function gives.
//! 5. **The rails and the verge** ([`draw_wall_cell`]): steel side rails with
//!    stake pockets on the two bed edges, the tailgate at the left and the cab
//!    bulkhead at the right. Beyond the road strip the wall cells are drawn as
//!    verge: dust, weeds and guard-rail posts whipping past at [`VERGE_SPEED`].
//!
//! Everything is culled and clipped: the floor draws into the visible world
//! range only and the wall cells are already culled by the caller.

use std::ops::Range;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::level::{Level, Palette, PitKind, PropKind};
use super::levelgen::trailer::{DECK_LEFT, DECK_RIGHT, DECK_ROWS, DECK_TOP, RAIL_BOTTOM, RAIL_TOP};

/// World pixels per grid cell (the wall-art cell size).
const PX: i32 = 16;

/// How fast the road scrolls past, in world pixels per second. Fast enough to
/// read at a glance (about nine pixels a frame at 60 fps) without turning the
/// lane markings into a strobe.
pub const ROAD_SPEED: f32 = 540.0;
/// The verge's parallax: further away than the road, so slower.
pub const VERGE_SPEED: f32 = 300.0;
/// One expansion-joint bump every this many seconds (the deck's one-pixel dip).
pub const BUMP_PERIOD: f32 = 3.4;
/// Width of the exposed road strip past each side rail, in cells.
const ROAD_STRIP: i32 = 4;
/// Spacing between the road's lane bands, in pixels. One grid for the whole
/// road: the dashes seen through the cracks line up with the ones on the
/// exposed road beside the trailer.
const LANE_STEP: i32 = 96;
/// World y of the first lane line on the road (above the bed, so the grid is
/// independent of the camera).
const LANE_ZERO: i32 = DECK_TOP * PX - 128;
/// The bed's top edge in world pixels (const form of [`deck_top_px`]).
const DECK_TOP_PX: i32 = DECK_TOP * PX;

/// Height of one board + gap, in pixels: a 6 px board over a 3 px crack.
const PLANK: i32 = 9;
/// The board's own height (the rest of [`PLANK`] is the crack: one shadowed
/// lip and two rows of road).
const BOARD: i32 = 6;

// ---- Means of the bed (truecolor; the terminal's palette degrades these) ----

const ASPHALT: Rgb = (54, 54, 62);
const ASPHALT_DARK: Rgb = (31, 31, 36);
const ASPHALT_LIGHT: Rgb = (76, 77, 86);
const LANE_INK: Rgb = (198, 190, 156);
const STUD: Rgb = (232, 232, 240);
const RIP: Rgb = (24, 22, 20);

const WOOD: [Rgb; 5] = [
    (104, 80, 50),
    (94, 71, 44),
    (114, 88, 55),
    (86, 65, 40),
    (98, 74, 46),
];
const WOOD_DARK: Rgb = (62, 46, 28);
const WOOD_LIGHT: Rgb = (146, 116, 74);
const WOOD_END: Rgb = (128, 100, 62);
const KNOT: Rgb = (48, 34, 20);
const BOLT: Rgb = (150, 152, 158);
const STEEL: Rgb = (94, 98, 106);
const STEEL_HI: Rgb = (150, 156, 166);
const STEEL_LO: Rgb = (54, 58, 64);
const RUST: Rgb = (118, 74, 46);
const TARP: Rgb = (70, 80, 90);
const TARP_HI: Rgb = (96, 108, 120);

const VERGE: Rgb = (38, 46, 32);
const DUST: Rgb = (66, 58, 44);
const POST: Rgb = (116, 120, 128);
const OIL: Rgb = (22, 18, 14);

/// Deterministic 2D hash (the same one the other floor arts use).
#[inline]
fn hash(x: i32, y: i32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n
}

/// The one-pixel suspension dip on an expansion joint: `-1` for a fraction of
/// a frame after the joint, else `0`.
#[inline]
fn bump_offset(elapsed: f32) -> i32 {
    let phase = elapsed.rem_euclid(BUMP_PERIOD);
    if phase < 0.06 {
        -1
    } else {
        0
    }
}

/// A clipped painter over the visible world range.
struct Canvas<'a> {
    pb: &'a mut PixelBuffer,
    cam_x: i32,
    cam_y: i32,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
}

impl Canvas<'_> {
    #[inline]
    fn fill_visible(&mut self, c: Rgb) {
        self.fill(self.x0, self.y0, self.x1 - self.x0, self.y1 - self.y0, c);
    }

    /// Fill `w × h` world pixels from `(x, y)` (already bumped into screen
    /// space by the caller), clipped to the visible range.
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        let left = x.max(self.x0);
        let top = y.max(self.y0);
        let w = (x + w).min(self.x1) - left;
        let h = (y + h).min(self.y1) - top;
        if w <= 0 || h <= 0 {
            return;
        }
        self.pb.rect_fill(left - self.cam_x, top - self.cam_y, w, h, c);
    }

    /// One world pixel.
    fn px(&mut self, x: i32, y: i32, c: Rgb) {
        if x < self.x0 || y < self.y0 || x >= self.x1 || y >= self.y1 {
            return;
        }
        self.pb.plot(x - self.cam_x, y - self.cam_y, c);
    }
}

/// The bed's top edge in world pixels (the first deck row).
#[inline]
fn deck_top_px() -> i32 {
    DECK_TOP * PX
}

/// One past the bed's bottom edge in world pixels.
#[inline]
fn deck_bot_px() -> i32 {
    (DECK_TOP + DECK_ROWS) * PX
}

/// Paint the flatbed's floor: the road, the deck and the holes in it.
///
/// `cam` is the world position of the buffer's top-left pixel; `x`/`y` are the
/// visible world-pixel ranges (already clamped to the map by `art_floor`), and
/// `elapsed` is the run clock.
pub fn draw_floor(
    pb: &mut PixelBuffer,
    level: &Level,
    cam: Vec2,
    elapsed: f32,
    x: &Range<i32>,
    y: &Range<i32>,
) {
    let cam_x = cam.x.round() as i32;
    let cam_y = cam.y.round() as i32;
    let mut c = Canvas {
        pb,
        cam_x,
        cam_y,
        x0: x.start,
        x1: x.end,
        y0: y.start,
        y1: y.end,
    };
    let pal = &level.palette;
    let bump = bump_offset(elapsed);
    let scroll = elapsed * ROAD_SPEED;
    let verge_scroll = elapsed * VERGE_SPEED;

    let deck_top = DECK_TOP_PX;
    let deck_bot = deck_bot_px();
    let road_top = deck_top - ROAD_STRIP * PX;
    let road_bot = deck_bot + ROAD_STRIP * PX;

    // 1. the verge beyond the road strip, then the road itself.
    c.fill_visible(VERGE);
    c.fill(c.x0, road_top + bump, c.x1 - c.x0, road_bot - road_top, ASPHALT);
    road_texture(&mut c, road_top, road_bot, scroll, bump, pal);

    // 2. the deck: boards over the road.
    planks(&mut c, deck_top, deck_bot, scroll, bump, pal);

    // 3. the hole you fall down, drawn over the deck.
    for pit in &level.trailer.pits {
        let rect = pit.rect.to_world();
        let (px, py) = (rect.x.round() as i32, rect.y.round() as i32 + bump);
        let (pw, ph) = (rect.w.round() as i32, rect.h.round() as i32);
        match pit.kind {
            PitKind::BrokenPlank => broken_planks(&mut c, px, py, pw, ph, scroll, pal),
            PitKind::RailGap => rail_gap(&mut c, px, py, pw, ph, scroll, pal),
        }
    }

    // 4. the load that walks along its straps.
    for slide in &level.trailer.slides {
        let offset = crate::game::level::TrailerDeck::slide_offset(slide, elapsed);
        let pos = slide.rest + slide.dir * offset;
        let phase = (elapsed + slide.phase).rem_euclid(crate::game::level::SLIDE_PERIOD);
        let telling = phase < slide.warn;
        slide_crate(&mut c, pos, bump, telling, elapsed, pal);
    }

    // 5. the slipstream's tell: pale streaks drifting tailward.
    for zone in &level.trailer.wind {
        let rect = zone.to_world();
        wind_streaks(&mut c, rect, bump, verge_scroll, pal);
    }
}

/// The road: asphalt, tar seams, lane markings, studs and skid marks, all
/// scrolling at [`ROAD_SPEED`] (plus the odd patch that is static because it is
/// painted on the road, not passed over it — no: everything here moves).
fn road_texture(
    c: &mut Canvas<'_>,
    road_top: i32,
    road_bot: i32,
    scroll: f32,
    bump: i32,
    pal: &Palette,
) {
    let x0 = c.x0;
    let x1 = c.x1;
    let width = x1 - x0;
    if width <= 0 {
        return;
    }
    let span = (road_bot - road_top).max(1);
    // Lane lines on one fixed grid across the whole road (the trailer
    // straddles them, which is why the dashes show through the cracks).
    let mut lanes: [i32; 12] = [0; 12];
    let mut n_lanes = 0;
    let mut lane_y = LANE_ZERO;
    while lane_y <= road_bot && n_lanes < lanes.len() {
        if lane_y >= road_top {
            lanes[n_lanes] = lane_y;
            n_lanes += 1;
        }
        lane_y += LANE_STEP;
    }
    for lane in lanes[..n_lanes].iter().copied() {
        let ly = lane + bump;
        for x in x0..x1 {
            let wx = ((x as f32 + scroll) as i32).rem_euclid(128);
            if wx < 44 {
                // A dash: bright centre, worn ends.
                let ink = if !(4..=40).contains(&wx) {
                    mix(ASPHALT, LANE_INK, 0.45)
                } else {
                    LANE_INK
                };
                c.px(x, ly, ink);
                if wx == 22 {
                    // A reflective stud on the line, every dash.
                    c.px(x, ly, STUD);
                }
            } else if wx == 70 {
                // A worn tar seam between dashes.
                c.px(x, ly + 1, ASPHALT_DARK);
            }
        }
    }
    // Skid marks: two dark parallel streaks, scrolling with the road.
    for (i, off) in [(0i32, 22i32), (1, 30)] {
        let y = road_bot - 3 * PX + off + bump;
        for x in x0..x1 {
            let wx = ((x as f32 + scroll * 0.5) as i32 + i * 37).rem_euclid(320);
            if wx < 60 {
                c.px(x, y, mix(ASPHALT, RIP, 0.55));
            }
        }
    }
    // Asphalt grain and patches: short moving dabs, so every gap in the planks
    // carries motion even where no marking passes.
    let mut x = (x0 / 7) * 7;
    while x < x1 {
        let n = hash((((x as f32 + scroll) as i32) / 7).rem_euclid(4096), 0x4d);
        let y = road_top + (n >> 7) as i32 % span.max(1);
        let tone = match n & 3 {
            0 => ASPHALT_LIGHT,
            1 => ASPHALT_DARK,
            _ => mix(ASPHALT, ASPHALT_DARK, 0.5),
        };
        c.fill(x, y + bump, 2 + (n >> 11) as i32 % 3, 1, tone);
        x += 7;
    }
    // The road's edge lines mark where the asphalt meets the verge.
    for edge in [road_top + 2, road_bot - 3] {
        let y = edge + bump;
        for x in x0..x1 {
            let wx = ((x as f32 + scroll) as i32).rem_euclid(64);
            if wx < 40 {
                c.px(x, y, mix(ASPHALT, LANE_INK, 0.55));
            }
        }
    }
    let _ = pal;
}

/// The deck: long oak boards with cracks between them, and everything that has
/// happened to a plank in twenty years of freight.
fn planks(
    c: &mut Canvas<'_>,
    deck_top: i32,
    deck_bot: i32,
    scroll: f32,
    bump: i32,
    pal: &Palette,
) {
    let total = deck_bot - deck_top;
    if total <= 0 {
        return;
    }
    let x0 = c.x0;
    let x1 = c.x1;
    // Which plank rows can the view see?
    let first = ((c.y0 - deck_top - bump).div_euclid(PLANK)).max(0);
    let last = ((c.y1 - deck_top - bump).div_euclid(PLANK)).min(total / PLANK);
    for row in first..=last {
        let y = deck_top + row * PLANK + bump;
        let n = hash(row, 0x71);
        let mut body = WOOD[(n & 3) as usize];
        // Some boards have weathered to a pale grey: the deck is old.
        if n & 15 == 0 {
            body = mix(body, (128, 122, 110), 0.42);
        }
        let grain = mix(body, WOOD_DARK, 0.5);
        let light = mix(body, WOOD_LIGHT, 0.45);
        // The board.
        c.fill(x0, y, x1 - x0, BOARD, body);
        // Grain: two lengthwise lines, broken at the butt joints below.
        c.fill(x0, y + 2, x1 - x0, 1, grain);
        c.fill(x0, y + 4, x1 - x0, 1, mix(body, grain, 0.5));
        // The lit upper lip and the shadow it casts into the crack.
        c.fill(x0, y, x1 - x0, 1, light);
        c.fill(x0, y + BOARD - 1, x1 - x0, 1, mix(body, WOOD_DARK, 0.7));

        // Butt joints: every 48–104 px, staggered per plank so the deck does
        // not read as tiling.
        let step = 48 + (hash(row, 0x17) % 7) as i32 * 8;
        let off = (n >> 8) as i32 % step;
        let mut joint = (x0 - off).div_euclid(step) * step + off;
        while joint < x1 {
            let jx = joint;
            if jx > x0 - 2 {
                // Scuffed end: a pale worn lip and a dark seam.
                c.fill(jx, y, 1, BOARD, WOOD_DARK);
                c.fill(jx + 1, y + 1, 1, BOARD - 2, WOOD_END);
                c.fill(jx + 2, y + 2, 1, BOARD - 4, mix(body, WOOD_END, 0.5));
            }
            // A bolt head on the joint, on every other one.
            if hash(row, (jx / step) ^ 0x2f) & 1 == 0 {
                c.px(jx + 3, y + 3, BOLT);
            }
            joint += step;
        }

        // Knots: a dark eye, one per board every so often.
        if n & 7 == 1 {
            let kstep = 128;
            let koff = (hash(row, 0x33) as i32).rem_euclid(kstep);
            let mut kx = (x0 - koff).div_euclid(kstep) * kstep + koff;
            while kx < x1 {
                if kx > x0 - 2 {
                    c.px(kx, y + 2, KNOT);
                    c.px(kx + 1, y + 2, KNOT);
                    c.px(kx, y + 3, mix(body, KNOT, 0.5));
                }
                kx += kstep;
            }
        }
        // Oil: a dark run soaked along one board now and then.
        if n & 31 == 5 {
            let ostep = 192;
            let ooff = (hash(row, 0x51) as i32).rem_euclid(ostep);
            let mut ox = (x0 - ooff).div_euclid(ostep) * ostep + ooff;
            while ox < x1 {
                if ox > x0 - 16 {
                    c.fill(ox, y + 1, 14, 3, mix(body, OIL, 0.55));
                    c.fill(ox + 3, y + 4, 8, 1, mix(body, OIL, 0.35));
                }
                ox += ostep;
            }
        }
        // Bolt rows: a pair every 96 px, metal washers biting into the wood.
        let mut bx = x0.div_euclid(96) * 96 + 16;
        while bx < x1 {
            c.px(bx, y + 1, BOLT);
            c.px(bx, y + 4, mix(body, BOLT, 0.5));
            bx += 96;
        }

        // The crack: a dark shadowed lip, then the road underneath.
        let gy = y + BOARD;
        c.fill(x0, gy, x1 - x0, 1, RIP);
        road_in_gap(c, gy + 1, 0, scroll, pal);
        road_in_gap(c, gy + 2, 1, scroll, pal);
        let _ = pal;
    }
}

/// The road as seen through one 1-px crack between planks.
fn road_in_gap(c: &mut Canvas<'_>, y: i32, depth: i32, scroll: f32, pal: &Palette) {
    let x0 = c.x0;
    let x1 = c.x1;
    if y < c.y0 || y >= c.y1 {
        return;
    }
    // The row right under the lip is shaded by the board above it; the next
    // one catches the light. That vertical step is what makes the crack read
    // as a hole with a road far below rather than a painted line.
    let base = scale(ASPHALT, 0.86 + 0.20 * depth as f32);
    c.fill(x0, y, x1 - x0, 1, base);
    // A long sheen sweeping the asphalt, so the surface has depth. Anchored to
    // the road (x + scroll), never to the view, so a crop matches the whole
    // map pixel for pixel.
    let period = 221;
    let mut sx = x0;
    while sx < x1 {
        let wx = (sx + scroll as i32).rem_euclid(period);
        if wx < 46 {
            let len = (46 - wx).min(x1 - sx);
            c.fill(sx, y, len, 1, mix(base, ASPHALT_LIGHT, 0.35));
            sx += len;
        } else {
            sx += period - wx;
        }
    }
    // Lane dashes: a bright dashed line through every lane band, on the same
    // grid as the road outside the rails.
    let mut lane = LANE_ZERO;
    while lane <= deck_bot_px() + LANE_STEP {
        if (y - lane).abs() <= 1 {
            for x in x0..x1 {
                let wx = ((x as f32 + scroll) as i32).rem_euclid(144);
                if (6..54).contains(&wx) {
                    c.px(x, y, if !(10..=50).contains(&wx) {
                        mix(ASPHALT, LANE_INK, 0.5)
                    } else {
                        LANE_INK
                    });
                } else if wx == 72 {
                    // A reflective stud, every dash.
                    c.px(x, y, STUD);
                }
            }
        }
        lane += LANE_STEP;
    }
    // Static steel cross-members under the deck: the road blinks behind them.
    let mut member = (x0.div_euclid(96)) * 96 + 24;
    while member < x1 {
        c.fill(member, y, 3, 1, STEEL_LO);
        c.px(member + 1, y, STEEL);
        member += 96;
    }
    // Aggregate and grit: short bright streaks scrolling at road speed, so
    // every crack reads as fast even when no marking is passing.
    let mut x = (x0.div_euclid(4)) * 4;
    while x < x1 {
        let n = hash((((x as f32 + scroll) as i32).div_euclid(4)).rem_euclid(8192), y);
        if n & 3 == 0 {
            let len = 1 + (n >> 12) as i32 % 3;
            c.fill(x, y, len, 1, ASPHALT_LIGHT);
            if n & 7 == 0 {
                c.px(x + len - 1, y, mix(ASPHALT_LIGHT, STUD, 0.45));
            }
        }
        x += 4;
    }
    let _ = pal;
}

/// A broken-plank hole: the road below, a dark lip, and splintered ends.
#[allow(clippy::too_many_arguments)]
fn broken_planks(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scroll: f32,
    pal: &Palette,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    // The hole: the road below, lit from above (a hole is a window).
    c.fill(x, y, w, h, scale(ASPHALT, 1.08));
    for lane_y in 0..h {
        let yy = y + lane_y;
        c.fill(x, yy, w, 1, ASPHALT_DARK);
        let mut px = x;
        while px < x + w {
            let n = hash((((px as f32 + scroll) as i32) / 4).rem_euclid(4096), yy);
            if n & 3 == 0 {
                c.px(px, yy, ASPHALT_LIGHT);
            }
            px += 4;
        }
    }
    // The lip: a hard shadow on the far edge, a splintered board on the near.
    c.fill(x, y, w, 1, RIP);
    c.fill(x, y, 1, h, RIP);
    c.fill(x + w - 1, y, 1, h, RIP);
    // Splintered boards: pale teeth poking into the hole on each side, with
    // the dark seam where the plank was torn off.
    for k in 0..h {
        let tooth = (hash(x, y + k) % 3) as i32;
        if tooth > 0 {
            c.fill(x, y + k, 1 + tooth, 1, WOOD_END);
            c.px(x + 1 + tooth, y + k, WOOD[2]);
        }
        let tooth = (hash(x + w, y + k) % 3) as i32;
        if tooth > 0 {
            c.fill(x + w - 1 - tooth, y + k, 1 + tooth, 1, WOOD_END);
            c.px(x + w - 2 - tooth, y + k, WOOD[2]);
        }
    }
    for k in 0..w {
        let tooth = (hash(x + k, y + h) % 3) as i32;
        if tooth > 0 {
            c.fill(x + k, y + h - 1 - tooth, 1, 1 + tooth, WOOD_END);
        }
    }
    let _ = pal;
}

/// A missing rail section: the road four metres below, and two sheared stubs of
/// rail at the sides.
#[allow(clippy::too_many_arguments)]
fn rail_gap(c: &mut Canvas<'_>, x: i32, y: i32, w: i32, h: i32, scroll: f32, pal: &Palette) {
    if w <= 0 || h <= 0 {
        return;
    }
    c.fill(x, y, w, h, ASPHALT_DARK);
    // The road below, through the gap.
    for yy in y..y + h {
        let mut px = x;
        while px < x + w {
            let n = hash((((px as f32 + scroll) as i32) / 3).rem_euclid(4096), yy);
            if n & 7 == 0 {
                c.px(px, yy, ASPHALT);
            }
            px += 3;
        }
    }
    // Sheared rail stubs at both ends.
    for sx in [x - 2, x + w] {
        c.fill(sx, y + 1, 2, h - 2, STEEL_LO);
        c.px(sx, y + 2, STEEL_HI);
        c.px(sx + 1, y + h - 3, RUST);
    }
    // A dark shadow under the deck edge so the drop reads.
    c.fill(x, y, w, 1, RIP);
    let _ = pal;
}

/// The crate that walks along its straps. `telling` is the creak before the
/// shove: it rattles a pixel and shows the strap pulled up bright.
fn slide_crate(
    c: &mut Canvas<'_>,
    pos: Vec2,
    bump: i32,
    telling: bool,
    elapsed: f32,
    pal: &Palette,
) {
    let rattle = if telling {
        ((elapsed * 38.0).sin() * 0.9).round() as i32
    } else {
        0
    };
    let x = pos.x.round() as i32 - 10 + rattle;
    let y = pos.y.round() as i32 - 9 + bump;
    // Contact shadow.
    c.fill(x + 1, y + 2, 20, 17, mix(ASPHALT, RIP, 0.55));
    // Body: a strapped wooden crate, lit from the top-left.
    c.fill(x, y, 20, 17, WOOD_DARK);
    c.fill(x + 1, y + 1, 18, 14, WOOD[2]);
    c.fill(x + 1, y + 1, 18, 1, WOOD_LIGHT);
    c.fill(x + 2, y + 2, 16, 1, mix(WOOD[2], WOOD_LIGHT, 0.35));
    c.fill(x + 2, y + 13, 16, 1, mix(WOOD[2], WOOD_DARK, 0.6));
    // Plank seams.
    for k in [5, 9, 13] {
        c.fill(x + 1, y + k, 18, 1, mix(WOOD[2], WOOD_DARK, 0.45));
    }
    // Ratchet straps: two vertical bands with buckles, bright while creaking.
    let strap = if telling { STEEL_HI } else { mix(WOOD_DARK, STEEL, 0.45) };
    for sx in [x + 4, x + 14] {
        c.fill(sx, y, 2, 17, strap);
        c.fill(sx, y + 7, 2, 2, STEEL_LO);
        if telling {
            c.px(sx, y + 3 + (elapsed * 12.0) as i32 % 3, STUD);
        }
    }
    let _ = pal;
}

/// Pale streaks drifting toward the tailgate inside a wind stretch.
fn wind_streaks(c: &mut Canvas<'_>, rect: crate::game::level::Rectf, bump: i32, scroll: f32, pal: &Palette) {
    let y0 = rect.y.round() as i32 + bump;
    let h = rect.h.round() as i32;
    let x0 = rect.x.round() as i32;
    let w = rect.w.round() as i32;
    let ink = mix(mix(ASPHALT, LANE_INK, 0.2), WOOD_LIGHT, 0.35);
    let _ = pal;
    for k in 0..3 {
        let yy = y0 + 2 + k * (h.max(1) / 3);
        let mut x = x0;
        while x < x0 + w {
            let n = hash(x / 3, k);
            if (n >> 5) & 3 == 0 {
                let len = 4 + (n >> 9) as i32 % 5;
                c.fill(x + ((scroll as i32 + k * 41) % 23) - 11, yy, len, 1, ink);
            }
            x += 11;
        }
    }
}

// ---- Wall cells: cargo, rails, tailgate, cab and the verge -----------------

/// What a solid cell of the trailer maps is.
enum DeckPart {
    /// Cargo inside the bed (crates, pallets, a tarped machine).
    Cargo(u32),
    /// The left tailgate.
    Tailgate,
    /// The right cab bulkhead (steel plate).
    Bulkhead,
    /// A side rail row.
    Rail,
    /// The road strip just outside the rails: leave the floor's road visible.
    Road,
    /// The verge beyond the road.
    Verge,
}

#[inline]
fn deck_part(cell_x: i32, cell_y: i32, seed: u32) -> DeckPart {
    let deck_top = DECK_TOP;
    let deck_bot = DECK_TOP + DECK_ROWS;
    if (deck_top..deck_bot).contains(&cell_y) {
        if cell_x < DECK_LEFT - 1 {
            return DeckPart::Road;
        }
        if cell_x == DECK_LEFT - 1 {
            return DeckPart::Tailgate;
        }
        if cell_x >= DECK_RIGHT {
            return DeckPart::Bulkhead;
        }
        return DeckPart::Cargo(hash(cell_x, cell_y ^ seed as i32));
    }
    if cell_y == RAIL_TOP || cell_y == RAIL_BOTTOM {
        return DeckPart::Rail;
    }
    if (RAIL_TOP - ROAD_STRIP..RAIL_TOP).contains(&cell_y)
        || (RAIL_BOTTOM + 1..=RAIL_BOTTOM + ROAD_STRIP).contains(&cell_y)
    {
        return DeckPart::Road;
    }
    DeckPart::Verge
}

/// The ground layer: nothing. The deck's contact shadows are painted by the
/// floor pass, which knows where the boards are; a wall-art shadow across the
/// road would read as a floating stripe.
pub fn draw_wall_ground(_pb: &mut PixelBuffer, _view: &super::art_walls::WallCellView, _cam: Vec2, _accent: Rgb) {}

/// The body layer: cargo blocks, the rails, the tailgate and the cab bulkhead.
pub fn draw_wall_cell(
    pb: &mut PixelBuffer,
    view: &super::art_walls::WallCellView,
    cam: Vec2,
    _accent: Rgb,
) {
    let sx = (view.cell_x * PX) - cam.x.round() as i32;
    let sy = (view.cell_y * PX) - cam.y.round() as i32;
    if sx + PX < 0 || sy + PX < 0 || sx > pb.w as i32 || sy > pb.h as i32 {
        return;
    }
    match deck_part(view.cell_x, view.cell_y, view.seed) {
        // The floor's road shows through: nothing painted here.
        DeckPart::Road => {}
        DeckPart::Cargo(n) => cargo_cell(pb, sx, sy, n, view.anim_clock, view.mask),
        DeckPart::Tailgate => tailgate(pb, sx, sy),
        DeckPart::Bulkhead => bulkhead(pb, sx, sy, view.seed),
        DeckPart::Rail => rail(pb, sx, sy, view.cell_x, view.anim_clock),
        DeckPart::Verge => verge(pb, sx, sy, view.cell_x, view.cell_y, view.anim_clock),
    }
}

/// The overhang layer: nothing hangs over the deck. (Cargo is drawn in the body
/// pass; the rat never ducks under a crate on this floor.)
pub fn draw_wall_overhang(
    _pb: &mut PixelBuffer,
    _view: &super::art_walls::WallCellView,
    _cam: Vec2,
    _accent: Rgb,
) {
}

/// One cargo cell: a crate, a pallet stack, a tarped machine or a pipe bundle,
/// chosen from the cell hash so a stack is a mosaic of freight.
fn cargo_cell(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    n: u32,
    elapsed: f32,
    mask: super::art_walls::NeighborMask,
) {
    use super::art_walls::NeighborMask;
    let variant = (n >> 4) & 3;
    let body = WOOD[(n & 3) as usize];
    match variant {
        0 => {
            // A wooden crate: lid, slats, corner irons.
            pb.rect_fill(sx, sy, PX, PX, WOOD_DARK);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, PX - 2, body);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, 2, mix(body, WOOD_LIGHT, 0.5));
            pb.rect_fill(sx + 2, sy + 9, PX - 4, 1, mix(body, WOOD_DARK, 0.6));
            pb.rect_fill(sx + 2, sy + 4, PX - 4, 1, mix(body, WOOD_DARK, 0.4));
            // Corner irons.
            for (cx, cy) in [(2, 2), (PX - 3, 2), (2, PX - 3), (PX - 3, PX - 3)] {
                pb.plot(sx + cx, sy + cy, STEEL_LO);
            }
        }
        1 => {
            // A pallet: slat gaps show the shadow behind.
            pb.rect_fill(sx, sy, PX, PX, WOOD_DARK);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, PX - 2, mix(body, WOOD_DARK, 0.4));
            for k in 0..4 {
                let yy = sy + 1 + k * 4;
                pb.rect_fill(sx + 1, yy, PX - 2, 3, body);
                pb.rect_fill(sx + 1, yy, PX - 2, 1, mix(body, WOOD_LIGHT, 0.35));
            }
        }
        2 => {
            // A tarped machine: grey-blue canvas with a rope and a crease.
            let tarp = mix(TARP, TARP_HI, (n & 7) as f32 / 14.0);
            pb.rect_fill(sx, sy, PX, PX, scale(tarp, 0.5));
            pb.rect_fill(sx + 1, sy + 2, PX - 2, PX - 3, tarp);
            pb.rect_fill(sx + 1, sy + 2, PX - 2, 1, TARP_HI);
            pb.rect_fill(sx + 2, sy + 7, PX - 4, 1, mix(tarp, TARP, 0.7));
            pb.rect_fill(sx + 1, sy + 11, PX - 2, 1, mix(tarp, STEEL_LO, 0.4));
            // Rope across the load.
            pb.rect_fill(sx + 8, sy + 2, 1, PX - 3, mix(tarp, WOOD_LIGHT, 0.4));
        }
        _ => {
            // Pipe bundle: end circles and a bright rim.
            pb.rect_fill(sx, sy, PX, PX, STEEL_LO);
            for (dx, dy) in [(5, 6), (11, 5), (8, 11)] {
                pb.filled_circle(sx + dx, sy + dy, 3, mix(STEEL, STEEL_LO, 0.5));
                pb.ring(sx + dx, sy + dy, 2, STEEL_HI);
                pb.plot(sx + dx - 1, sy + dy - 1, mix(STEEL_HI, WOOD_LIGHT, 0.5));
            }
            pb.rect_fill(sx + 1, sy + 1, PX - 2, 1, STEEL);
            pb.rect_fill(sx + 1, sy + PX - 2, PX - 2, 1, STEEL_LO);
            // A strap binding the bundle.
            let strap = mix(STEEL, RUST, 0.35 + (elapsed * 0.0).sin());
            pb.rect_fill(sx + 2, sy + 13, PX - 4, 1, strap);
        }
    }
    // Height: a lit top edge on the north face of a stack, a shaded front face
    // and a contact shadow where the stack ends over open deck. Without this a
    // stack reads as floor paint rather than a wall of freight.
    if !mask.has(NeighborMask::N) {
        pb.rect_fill(sx, sy, PX, 1, mix(WOOD_LIGHT, WOOD[2], 0.4));
    }
    if !mask.has(NeighborMask::W) {
        pb.rect_fill(sx, sy, 1, PX, mix(WOOD_DARK, RIP, 0.2));
    }
    if !mask.has(NeighborMask::S) {
        // The south face: two rows darker, then the shadow it throws.
        pb.rect_fill(sx, sy + PX - 3, PX, 3, scale(WOOD_DARK, 0.70));
        pb.rect_fill(sx, sy + PX - 1, PX, 1, RIP);
        pb.rect_fill(sx + 1, sy + PX + 1, PX - 1, 3, mix(ASPHALT_DARK, RIP, 0.45));
    }
    if !mask.has(NeighborMask::E) {
        pb.rect_fill(sx + PX - 1, sy + 1, 1, PX - 1, mix(WOOD_DARK, RIP, 0.35));
    }
}

/// The tailgate: ribbed steel with a hinge and a latch.
fn tailgate(pb: &mut PixelBuffer, sx: i32, sy: i32) {
    pb.rect_fill(sx, sy, PX, PX, STEEL_LO);
    for k in 0..4 {
        pb.rect_fill(sx + 1, sy + 1 + k * 4, PX - 2, 3, STEEL);
        pb.rect_fill(sx + 1, sy + 1 + k * 4, PX - 2, 1, mix(STEEL, STEEL_HI, 0.5));
    }
    pb.rect_fill(sx, sy, 1, PX, STEEL_HI);
    pb.rect_fill(sx + 3, sy + 6, 2, 4, RUST);
}

/// The cab bulkhead: a riveted steel plate.
fn bulkhead(pb: &mut PixelBuffer, sx: i32, sy: i32, seed: u32) {
    pb.rect_fill(sx, sy, PX, PX, mix(STEEL_LO, STEEL, 0.35));
    pb.rect_fill(sx + 1, sy + 1, PX - 2, 1, STEEL_HI);
    pb.rect_fill(sx + 1, sy + PX - 2, PX - 2, 1, STEEL_LO);
    for k in 0..3 {
        pb.plot(sx + 2 + k % 2 * 10, sy + 3 + k * 5, STEEL_HI);
    }
    let _ = seed;
}

/// One side-rail cell: steel rail on the deck edge, with the stake pockets at
/// intervals and the road's glow under it.
fn rail(pb: &mut PixelBuffer, sx: i32, sy: i32, cell_x: i32, elapsed: f32) {
    pb.rect_fill(sx, sy, PX, PX, mix(STEEL_LO, VERGE, 0.35));
    // The rail: two bright lines with a shadow between them.
    pb.rect_fill(sx, sy + 3, PX, 2, STEEL);
    pb.rect_fill(sx, sy + 3, PX, 1, STEEL_HI);
    pb.rect_fill(sx, sy + 5, PX, 1, STEEL_LO);
    pb.rect_fill(sx, sy + 9, PX, 2, mix(STEEL, STEEL_LO, 0.4));
    // Stake pockets every four cells, with a bitter rust tear.
    if cell_x.rem_euclid(4) == 0 {
        pb.rect_fill(sx + 3, sy, 3, PX, mix(STEEL, STEEL_HI, 0.25));
        pb.rect_fill(sx + 3, sy, 1, PX, STEEL_HI);
        pb.rect_fill(sx + 3, sy + 7, 3, 3, RUST);
    }
    // The road's light catching the underside as it passes.
    let glint = ((elapsed * 6.0 + cell_x as f32 * 0.7).sin() + 1.0) * 0.5;
    if glint > 0.86 {
        pb.rect_fill(sx, sy + PX - 2, PX, 1, mix(ASPHALT, STEEL, 0.45));
    }
}

/// The verge beyond the road: dust, weeds and guard-rail posts whipping past.
fn verge(pb: &mut PixelBuffer, sx: i32, sy: i32, cell_x: i32, cell_y: i32, elapsed: f32) {
    let n = hash(cell_x, cell_y);
    pb.rect_fill(sx, sy, PX, PX, VERGE);
    // Dust and stones.
    for k in 0..4 {
        let dx = (n >> (k * 3)) as i32 % PX;
        let dy = (n >> (k * 5 + 2)) as i32 % PX;
        pb.plot(sx + dx, sy + dy, if n & 1 == 0 { DUST } else { mix(VERGE, DUST, 0.4) });
    }
    // Weeds: a hank of pale grass on some cells.
    if n & 3 == 0 {
        let gx = sx + 4 + (n >> 7) as i32 % 7;
        pb.rect_fill(gx, sy + 6, 1, 8, mix(VERGE, DUST, 0.55));
        pb.rect_fill(gx + 2, sy + 8, 1, 6, mix(VERGE, DUST, 0.35));
        pb.plot(gx + 1, sy + 7, mix(VERGE, WOOD_LIGHT, 0.35));
    }
    // Guard-rail posts: a post every four cells, scrolling with the verge's
    // parallax. The cell that "owns" a post is the one whose world x the
    // scrolling index lands on.
    let scroll = elapsed * VERGE_SPEED;
    let phase = ((cell_x as f32 * PX as f32 + scroll) as i32).rem_euclid(64);
    let post_row = if cell_y < RAIL_TOP { cell_y == RAIL_TOP - 2 } else { cell_y == RAIL_BOTTOM + 2 };
    if post_row && phase < 8 {
        let px = sx + phase;
        pb.rect_fill(px, sy - 2, 2, PX + 4, POST);
        pb.rect_fill(px, sy - 2, 2, 2, mix(POST, STEEL_HI, 0.5));
        pb.rect_fill(px, sy + PX + 1, 2, 1, STEEL_LO);
        // The rail line the posts carry.
        pb.rect_fill(sx, sy + 2, PX, 1, mix(POST, STEEL, 0.4));
    } else if post_row {
        pb.rect_fill(sx, sy + 2, PX, 1, mix(POST, STEEL, 0.35));
    }
}

// ---- Loose cargo props -----------------------------------------------------

/// Draw one cargo prop into its screen rectangle. Dispatched from
/// `art_props::draw_prop` for the flatbed's cargo kinds.
#[allow(clippy::too_many_arguments)]
pub fn draw_cargo(
    pb: &mut PixelBuffer,
    kind: PropKind,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    seed: u32,
    pal: &Palette,
) {
    let n = seed;
    // Contact shadow under everything.
    pb.rect_fill(x + 1, y + h - 2, w, 3, mix(ASPHALT, RIP, 0.6));
    match kind {
        PropKind::Pallet => {
            pb.rect_fill(x, y, w, h, WOOD_DARK);
            pb.rect_fill(x + 1, y + 1, w - 2, h - 2, mix(WOOD[1], WOOD_DARK, 0.35));
            let mut k = 1;
            while k < h - 1 {
                pb.rect_fill(x + 1, y + k, w - 2, 3, WOOD[1]);
                pb.rect_fill(x + 1, y + k, w - 2, 1, mix(WOOD[1], WOOD_LIGHT, 0.4));
                k += 4;
            }
            // Slats broken out of the middle.
            pb.rect_fill(x + w / 2, y + 5, 5, 3, mix(WOOD_DARK, RIP, 0.4));
        }
        PropKind::TarpLump => {
            let tarp = mix(TARP, TARP_HI, (n & 7) as f32 / 16.0);
            pb.rect_fill(x, y, w, h, scale(tarp, 0.45));
            pb.rect_fill(x + 1, y + 2, w - 2, h - 3, tarp);
            // A lumpy ridge line and the rope tied down over it.
            for k in 0..w - 2 {
                let hump = (k as f32 / w as f32 * 3.2).sin();
                let yy = y + 3 + (hump * 1.6) as i32 + 1;
                pb.plot(x + 1 + k, yy, TARP_HI);
            }
            pb.rect_fill(x + w / 2 - 1, y + 2, 2, h - 3, mix(tarp, WOOD_LIGHT, 0.45));
            pb.rect_fill(x + 1, y + h - 2, w - 2, 1, mix(tarp, STEEL_LO, 0.5));
        }
        PropKind::Tyre => {
            let cx = x + w / 2;
            let cy = y + h / 2;
            let r = (w.min(h) / 2 - 1).max(2);
            pb.filled_circle(cx, cy, r, (28, 28, 30));
            pb.ring(cx, cy, r, mix((28, 28, 30), (74, 76, 80), 0.6));
            pb.ring(cx, cy, r - 2, (20, 20, 22));
            pb.filled_circle(cx, cy, r - 3, mix(ASPHALT, RIP, 0.4));
            // Tread nubs.
            for k in 0..6 {
                let a = k as f32 / 6.0 * std::f32::consts::TAU;
                pb.plot(cx + (a.cos() * r as f32) as i32, cy + (a.sin() * r as f32) as i32, (60, 62, 66));
            }
        }
        PropKind::Toolbox => {
            pb.rect_fill(x, y, w, h, STEEL_LO);
            pb.rect_fill(x + 1, y + 1, w - 2, h - 2, mix(STEEL, STEEL_LO, 0.45));
            pb.rect_fill(x + 1, y + 1, w - 2, 1, STEEL_HI);
            pb.rect_fill(x + 2, y + h - 3, w - 4, 1, mix(STEEL, STEEL_HI, 0.3));
            // Latch and hinge.
            pb.rect_fill(x + w / 2, y + h - 4, 2, 2, RUST);
            pb.plot(x + 2, y + 3, STEEL_HI);
            pb.plot(x + w - 3, y + 3, STEEL_HI);
        }
        PropKind::PipeStack => {
            pb.rect_fill(x, y, w, h, mix(STEEL_LO, RIP, 0.3));
            let r = (h / 2 - 2).max(2);
            let mut cx = x + r + 1;
            while cx + r < x + w {
                pb.filled_circle(cx, y + h / 2, r, mix(STEEL, STEEL_LO, 0.4));
                pb.ring(cx, y + h / 2, r, STEEL_HI);
                pb.filled_circle(cx, y + h / 2, r - 2, mix(STEEL_LO, RIP, 0.45));
                cx += 2 * r;
            }
            // The strap binding the bundle, with a buckle.
            pb.rect_fill(x + w / 2 - 1, y, 2, h, mix(STEEL, RUST, 0.3));
            pb.rect_fill(x + w / 2 - 2, y + h / 2 - 1, 4, 3, STEEL_LO);
        }
        PropKind::ChainCoil => {
            let cx = x + w / 2;
            let cy = y + h / 2;
            for k in 0..4 {
                let r = 2 + k * 2;
                pb.ring(cx, cy, r, if k & 1 == 0 { STEEL } else { mix(STEEL, RUST, 0.4) });
            }
            pb.plot(cx, cy, STEEL_HI);
            // A short tail of chain trailing off the coil.
            pb.plot(cx + 4, cy + 5, STEEL_LO);
            pb.plot(cx + 6, cy + 6, STEEL);
        }
        _ => {}
    }
    let _ = pal;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::{Level, Theme};

    /// A small view of the deck around the tailgate.
    fn tailgate_frame(level: &Level, cam: Vec2) -> PixelBuffer {
        let mut pb = PixelBuffer::new(120, 80);
        draw_floor(
            &mut pb,
            level,
            cam,
            7.0,
            &(cam.x.round() as i32..cam.x.round() as i32 + 120),
            &(cam.y.round() as i32..cam.y.round() as i32 + 80),
        );
        pb
    }

    #[test]
    fn the_floor_is_a_crop_of_the_whole_deck() {
        // The road scrolls, the planks do not swim: the same world pixel must
        // paint the same colour whether it is drawn in a full-map pass or in a
        // small crop (the other themes pin this too).
        let level = Level::generate(Theme::Trailer, 5);
        let width = level.arena.x as usize;
        let height = level.arena.y as usize;
        let mut whole = PixelBuffer::new(width, height);
        draw_floor(
            &mut whole,
            &level,
            Vec2::new(0.0, 0.0),
            13.0,
            &(0..width as i32),
            &(0..height as i32),
        );
        let mut crop = PixelBuffer::new(101, 71);
        for (left, top) in [(92, 1060), (110, 1100), (700, 980), (2800, 1220)] {
            draw_floor(
                &mut crop,
                &level,
                Vec2::new(left as f32 + 0.4, top as f32 - 0.3),
                13.0,
                &(left..left + 101),
                &(top..top + 71),
            );
            for y in 0..crop.h {
                for x in 0..crop.w {
                    assert_eq!(
                        crop.pixel_at(x, y),
                        whole.pixel_at((left + x as i32) as usize, (top + y as i32) as usize),
                        "the deck moved between passes at ({}, {})",
                        left + x as i32,
                        top + y as i32
                    );
                }
            }
        }
    }

    #[test]
    fn the_road_moves_between_frames() {
        let level = Level::generate(Theme::Trailer, 5);
        let cam = level.start - Vec2::new(60.0, 40.0);
        let mut pb = PixelBuffer::new(120, 80);
        let visible = |pb: &mut PixelBuffer, elapsed: f32| {
            draw_floor(
                pb,
                &level,
                cam,
                elapsed,
                &(cam.x.round() as i32..cam.x.round() as i32 + 120),
                &(cam.y.round() as i32..cam.y.round() as i32 + 80),
            );
        };
        visible(&mut pb, 3.0);
        let a: Vec<Rgb> = (0..pb.w * pb.h).map(|i| pb.pixel_at(i % pb.w, i / pb.w)).collect();
        visible(&mut pb, 3.05);
        let b: Vec<Rgb> = (0..pb.w * pb.h).map(|i| pb.pixel_at(i % pb.w, i / pb.w)).collect();
        let changed = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert!(
            changed > 40,
            "the road barely moved in 0.05 s ({changed} pixels changed)"
        );
        assert!(
            changed < pb.w * pb.h / 2,
            "the whole floor re-rolled in 0.05 s ({changed} pixels changed)"
        );
    }

    #[test]
    fn the_deck_is_planks_over_a_road() {
        let level = Level::generate(Theme::Trailer, 11);
        let cam = Vec2::new(level.start.x - 60.0, level.start.y - 40.0);
        let pb = tailgate_frame(&level, cam);
        // Somewhere in the view there must be wood tones and asphalt-ish darks
        // and at least one bright bolt/splinter pixel.
        let mut wood = 0;
        let mut dark = 0;
        let mut bright = 0;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let (r, g, b) = pb.pixel_at(x, y);
                if r > 70 && r > b + 20 && g > b {
                    wood += 1;
                } else if r < 60 && g < 60 && b < 66 {
                    dark += 1;
                }
                if r > 140 && g > 140 && b > 130 {
                    bright += 1;
                }
            }
        }
        assert!(wood > 800, "only {wood} wood pixels on the deck");
        assert!(dark > 300, "only {dark} shadow/crack pixels");
        assert!(bright > 2, "no bolts, studs or splinters: {bright}");
    }

    #[test]
    fn the_bump_is_a_pure_function_of_the_clock() {
        assert_eq!(bump_offset(0.0), -1);
        assert_eq!(bump_offset(0.03), -1);
        assert_eq!(bump_offset(1.0), 0);
        assert_eq!(bump_offset(BUMP_PERIOD + 0.01), -1);
        assert_eq!(bump_offset(2.0 * BUMP_PERIOD), -1);
    }
}
