//! Art for the getaway ride: the flatbed trailer's deck and the rushing
//! highway beneath it.
//!
//! The owner's brief: *"a level that is on the bed of a long trailer, with the
//! road flowing past you can see through cracks between the wood planks."*
//!
//! This module owns that floor, and it is deliberately **render-only**: the
//! road's scroll, the expansion-joint bump, the gap-light flicker and the
//! sliding crates are pure functions of the run clock ([`Level::elapsed`]
//! reaches the art as `elapsed`), so they draw no rng, allocate nothing and
//! cannot change a replay.
//!
//! ## Layers
//!
//! 1. **The road** ([`draw_floor`]): sunlit asphalt with streaking highway
//!    dashes (white lane lines and warm yellow center dividers), reflective
//!    cat's-eye studs, glossy black tar seams, tyre wear ruts and occasional
//!    pothole flickers scrolling past at [`ROAD_SPEED`], seen through wide
//!    irregular plank gaps and for four cells past each side rail.
//! 2. **The undercarriage & deck**: weathered oak/pine boards of varied width
//!    with generous 4–6 px irregular gaps and warped chipped edges. Static
//!    steel I-beam cross-members, chassis frame rails and tandem-axle shadow
//!    bands sit *under* the gaps so the rushing road visibly streams behind
//!    the trailer's own steel skeleton.
//! 3. **The holes** ([`Pit`](crate::game::level::Pit)): torn, irregular blobs
//!    in the planks — not rectangles — showing the full sunlit rushing highway
//!    below through ragged snapped timber ends and a dark lip, plus sheared
//!    rail gaps where the highway streams right past the deck lip. The drawn
//!    hole is the sim's own outline ([`hole_pixel`] is [`Pit::contains`]), so a
//!    hole looks exactly as wide as it plays.
//! 4. **The load**: heavy crates, pallets, tarped machinery, brushed-steel
//!    pipe bundles, tyres, toolboxes and chain coils, plus sliding crates with
//!    high-visibility ratchet straps that blaze taut before each shove.
//! 5. **The rails and verge** ([`draw_wall_cell`]): riveted steel C-channel
//!    side rails with stake pockets and DOT reflectors, striped tailgate,
//!    headache-rack cab bulkhead, and a parallax gravel shoulder + W-beam
//!    highway guardrail whipping past at [`VERGE_SPEED`].

use std::ops::Range;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::level::{Level, Palette, Pit, PitKind, PropKind};
use super::levelgen::trailer::{DECK_LEFT, DECK_RIGHT, DECK_ROWS, DECK_TOP, RAIL_BOTTOM, RAIL_TOP};

/// World pixels per grid cell (the wall-art cell size).
const PX: i32 = 16;

/// How fast the road scrolls past, in world pixels per second.
pub const ROAD_SPEED: f32 = 540.0;
/// The verge's parallax: further away than the road, so slower.
pub const VERGE_SPEED: f32 = 300.0;
/// One expansion-joint bump every this many seconds (the deck's one-pixel dip).
pub const BUMP_PERIOD: f32 = 3.4;
/// Width of the exposed road strip past each side rail, in cells.
const ROAD_STRIP: i32 = 4;
/// Spacing between highway lane bands, in pixels (tight enough that 1–2 live
/// lane lines are always visibly streaking through the deck cracks).
const LANE_STEP: i32 = 48;
/// World y of the first lane line on the road (above the bed, anchored to
/// world coordinates so crops always match the whole map).
const LANE_ZERO: i32 = DECK_TOP * PX - 96;
/// The bed's top edge in world pixels.
const DECK_TOP_PX: i32 = DECK_TOP * PX;
/// The bed's left/right edges in world pixels.
const DECK_LEFT_PX: i32 = DECK_LEFT * PX;
const DECK_RIGHT_PX: i32 = DECK_RIGHT * PX;
/// Total deck height in pixels (`DECK_ROWS * 16 = 416`).
const DECK_H_PX: usize = (DECK_ROWS * PX) as usize;

// ---- Palette (truecolor; degrades cleanly to 256/16) -----------------------

const ASPHALT: Rgb = (58, 60, 68);
const ASPHALT_DARK: Rgb = (34, 35, 42);
const ASPHALT_LIGHT: Rgb = (88, 90, 100);
const ASPHALT_STREAK: Rgb = (106, 108, 118);
const TAR_SEAM: Rgb = (18, 18, 22);
const LANE_WHITE: Rgb = (218, 214, 194);
const LANE_YELLOW: Rgb = (224, 178, 68);
const STUD: Rgb = (244, 246, 252);
const STUD_AMBER: Rgb = (255, 208, 96);
const RIP: Rgb = (18, 16, 16);

const WOOD: [Rgb; 6] = [
    (112, 84, 52),  // warm weathered oak
    (96, 72, 45),   // aged mid timber
    (124, 95, 60),  // sun-warmed plank
    (84, 62, 39),   // dark oil-worn board
    (106, 86, 62),  // dry faded plank
    (114, 104, 88), // sun-bleached driftwood grey-oak
];
const WOOD_DARK: Rgb = (58, 42, 25);
const WOOD_LIGHT: Rgb = (158, 126, 82);
const WOOD_END: Rgb = (142, 112, 70);
const SPLINTER: Rgb = (186, 152, 98);
const KNOT: Rgb = (44, 30, 18);
const BOLT: Rgb = (162, 166, 174);
const BOLT_RING: Rgb = (74, 70, 68);
const STEEL: Rgb = (96, 102, 112);
const STEEL_HI: Rgb = (162, 170, 182);
const STEEL_LO: Rgb = (50, 54, 62);
const RUST: Rgb = (132, 74, 42);
const STRAP_ORANGE: Rgb = (214, 112, 36);
const STRAP_HOT: Rgb = (255, 216, 84);
const REFLECTOR_RED: Rgb = (212, 56, 48);
const REFLECTOR_WHT: Rgb = (232, 232, 224);
const TARP: Rgb = (68, 82, 94);
const TARP_HI: Rgb = (104, 120, 134);
const TARP_LO: Rgb = (40, 48, 56);
const TOOLBOX_RED: Rgb = (166, 52, 44);
const TOOLBOX_HI: Rgb = (216, 94, 78);

const VERGE: Rgb = (42, 52, 34);
const VERGE_DARK: Rgb = (28, 36, 22);
const GRAVEL: Rgb = (82, 78, 70);
const DUST: Rgb = (92, 82, 62);
const GRASS_HI: Rgb = (74, 86, 50);
const POST: Rgb = (122, 126, 136);
const OIL: Rgb = (22, 18, 14);

/// Deterministic 2D hash (fast, zero-alloc, crop-invariant).
#[inline(always)]
fn hash(x: i32, y: i32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n
}

/// One weathered deck plank and the wide irregular road gap below it.
#[derive(Clone, Copy)]
struct PlankSpec {
    /// Local offset from `DECK_TOP_PX`.
    y_off: i32,
    /// Board thickness in pixels (6..9 px).
    board_h: i32,
    /// Road gap thickness below this board in pixels (4..6 px).
    gap_h: i32,
    /// Stable row index for grain/joints/wear hashing.
    row_id: i32,
}

const MAX_PLANKS: usize = 44;

/// Compile-time irregular plank layout across the 416 px deck height.
/// Varied board widths (6–9 px) + wide irregular gaps (4–6 px) give the rushing
/// highway real visual presence while staying O(1) and 100% crop-invariant.
const fn build_plank_layout() -> ([PlankSpec; MAX_PLANKS], usize) {
    const BOARD_PAT: [i32; 11] = [8, 7, 6, 8, 7, 9, 6, 7, 8, 6, 7];
    const GAP_PAT: [i32; 9] = [5, 4, 6, 4, 5, 6, 4, 5, 5];
    let mut list = [PlankSpec {
        y_off: 0,
        board_h: 7,
        gap_h: 5,
        row_id: 0,
    }; MAX_PLANKS];
    let mut y = 0i32;
    let mut i = 0usize;
    while y < DECK_H_PX as i32 && i < MAX_PLANKS {
        let mut bh = BOARD_PAT[i % BOARD_PAT.len()];
        let mut gh = GAP_PAT[(i * 3 + 1) % GAP_PAT.len()];
        let rem = DECK_H_PX as i32 - y;
        if bh > rem {
            bh = rem;
            gh = 0;
        } else if bh + gh > rem {
            gh = rem - bh;
        }
        list[i] = PlankSpec {
            y_off: y,
            board_h: bh,
            gap_h: gh,
            row_id: i as i32,
        };
        y += bh + gh;
        i += 1;
    }
    (list, i)
}

static PLANK_LAYOUT: ([PlankSpec; MAX_PLANKS], usize) = build_plank_layout();

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

    /// Fill `w × h` world pixels from `(x, y)`, clipped to the visible range.
    #[inline]
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
    #[inline]
    fn px(&mut self, x: i32, y: i32, c: Rgb) {
        if x < self.x0 || y < self.y0 || x >= self.x1 || y >= self.y1 {
            return;
        }
        self.pb.plot(x - self.cam_x, y - self.cam_y, c);
    }

    /// Darken a world rectangle in place (used for shadows over the rushing road).
    fn darken_rect(&mut self, x: i32, y: i32, w: i32, h: i32, factor: f32) {
        let left = x.max(self.x0);
        let top = y.max(self.y0);
        let right = (x + w).min(self.x1);
        let bot = (y + h).min(self.y1);
        if left >= right || top >= bot {
            return;
        }
        for wy in top..bot {
            let sy = (wy - self.cam_y) as usize;
            for wx in left..right {
                let sx = (wx - self.cam_x) as usize;
                let cur = self.pb.pixel_at(sx, sy);
                self.pb.plot(sx as i32, sy as i32, scale(cur, factor));
            }
        }
    }
}

/// One past the bed's bottom edge in world pixels.
#[inline]
fn deck_bot_px() -> i32 {
    (DECK_TOP + DECK_ROWS) * PX
}

/// Paint the flatbed's floor: the rushing highway, the weathered plank deck,
/// the broken-plank pits, sliding crates, and slipstream streaks.
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
    let scroll_i = (elapsed * ROAD_SPEED).floor() as i32;
    let verge_scroll = elapsed * VERGE_SPEED;

    let deck_top = DECK_TOP_PX;
    let deck_bot = deck_bot_px();
    let road_top = deck_top - ROAD_STRIP * PX;
    let road_bot = deck_bot + ROAD_STRIP * PX;

    // 1. The verge beyond the road strip, then the exposed highway outside the deck.
    c.fill_visible(VERGE);
    // Exposed north and south highway strips beside the trailer rails.
    draw_road_band(&mut c, road_top, deck_top, scroll_i, bump, false);
    draw_road_band(&mut c, deck_bot, road_bot, scroll_i, bump, false);
    // Exposed road behind the tailgate (west of DECK_LEFT_PX).
    if c.x0 < DECK_LEFT_PX {
        let save_x1 = c.x1;
        c.x1 = c.x1.min(DECK_LEFT_PX);
        draw_road_band(&mut c, deck_top, deck_bot, scroll_i, bump, false);
        c.x1 = save_x1;
    }

    // 2. The deck: weathered oak/pine planks with wide irregular gaps revealing
    //    the rushing highway + static steel undercarriage underneath.
    planks(&mut c, deck_top, deck_bot, scroll_i, bump, pal);

    // 3. The pits you can fall through: torn holes in the planks (the outline
    //    is the sim's own [`Pit::contains`]) and sheared rail gaps.
    for pit in &level.trailer.pits {
        let rect = pit.bounds().to_world();
        let (px, py) = (rect.x.round() as i32, rect.y.round() as i32 + bump);
        let (pw, ph) = (rect.w.round() as i32, rect.h.round() as i32);
        if px + pw <= c.x0 || px >= c.x1 || py + ph <= c.y0 || py >= c.y1 {
            continue;
        }
        match pit.kind {
            PitKind::BrokenPlank if !pit.lobes().is_empty() => {
                torn_hole(&mut c, pit, scroll_i, bump)
            }
            PitKind::BrokenPlank => broken_planks(&mut c, px, py, pw, ph, scroll_i, bump),
            PitKind::RailGap => rail_gap(&mut c, px, py, pw, ph, scroll_i, bump),
            PitKind::TidalDrop => {}
        }
    }

    // 4. Sliding crates along their strapped scuff tracks.
    for slide in &level.trailer.slides {
        let offset = crate::game::level::TrailerDeck::slide_offset(slide, elapsed);
        let pos = slide.rest + slide.dir * offset;
        let phase = (elapsed + slide.phase).rem_euclid(crate::game::level::SLIDE_PERIOD);
        let telling = phase < slide.warn;
        let warn_p = if telling && slide.warn > 0.0 {
            (phase / slide.warn).clamp(0.0, 1.0)
        } else {
            0.0
        };
        slide_track(&mut c, slide.rest, slide.dir, slide.travel, bump);
        slide_crate(&mut c, pos, bump, telling, warn_p, elapsed);
    }

    // 5. The slipstream's tell: pale wind streaks drifting tailward.
    for zone in &level.trailer.wind {
        let rect = zone.to_world();
        if rect.x + rect.w < c.x0 as f32
            || rect.x > c.x1 as f32
            || rect.y + rect.h < c.y0 as f32
            || rect.y > c.y1 as f32
        {
            continue;
        }
        wind_streaks(&mut c, rect, bump, verge_scroll);
    }
}

/// Paint one horizontal band of rushing sunlit highway `[y_start..y_end]`
/// (in unbumped world y). Used for the exposed highway outside the rails AND
/// inside open pits. 100% anchored to world coordinates (`wx + scroll_i`).
fn draw_road_band(
    c: &mut Canvas<'_>,
    y_start: i32,
    y_end: i32,
    scroll_i: i32,
    bump: i32,
    under_deck: bool,
) {
    let vis_y0 = (y_start + bump).max(c.y0);
    let vis_y1 = (y_end + bump).min(c.y1);
    if vis_y0 >= vis_y1 || c.x0 >= c.x1 {
        return;
    }
    let road_top = DECK_TOP_PX - ROAD_STRIP * PX;
    let road_bot = deck_bot_px() + ROAD_STRIP * PX;
    let base_asphalt = if under_deck {
        scale(ASPHALT, 0.88)
    } else {
        ASPHALT
    };
    c.fill(c.x0, vis_y0, c.x1 - c.x0, vis_y1 - vis_y0, base_asphalt);

    for sy in vis_y0..vis_y1 {
        let wy = sy - bump;
        draw_road_scanline(c, c.x0, c.x1, sy, wy, scroll_i, road_top, road_bot, under_deck, 1.0);
    }
}

/// Paint a single rushing highway scanline across `[rx0..rx1]` at screen-world row `sy`
/// (whose unbumped road coordinate is `wy`), shaded by `shade`.
///
/// Fast horizontal-run design (zero per-pixel floats or trig) so wide gaps and
/// pits stay well under ~80 µs for a full 120×40 view while carrying real road speed:
/// * streaking asphalt aggregate & tyre wear ruts
/// * bold white lane dashes + warm yellow center divider dashes + cat's-eye studs
/// * diagonal glossy-black tar repair seams
/// * occasional recessed potholes with crumbled rims
/// * solid highway shoulder lines along the road edges
#[allow(clippy::too_many_arguments)]
fn draw_road_scanline(
    c: &mut Canvas<'_>,
    rx0: i32,
    rx1: i32,
    sy: i32,
    wy: i32,
    scroll_i: i32,
    road_top: i32,
    road_bot: i32,
    under_deck: bool,
    shade: f32,
) {
    if sy < c.y0 || sy >= c.y1 || rx0 >= rx1 {
        return;
    }
    let base = scale(ASPHALT, shade);
    let dark = scale(ASPHALT_DARK, shade);
    let light = scale(ASPHALT_LIGHT, shade);
    let streak = scale(ASPHALT_STREAK, shade);

    // 1) Tyre-track wear bands (subtle darker longitudinal ruts on the highway).
    let lane_local = (wy - LANE_ZERO).rem_euclid(LANE_STEP);
    let in_rut = (11..=16).contains(&lane_local) || (30..=35).contains(&lane_local);
    let row_base = if in_rut {
        mix(base, dark, 0.34)
    } else {
        base
    };
    c.fill(rx0, sy, rx1 - rx0, 1, row_base);

    // 2) Long sunlit/shaded road sheen waves anchored to `(wx + scroll_i)`.
    let sheen_period = 192;
    let mut sx = rx0;
    while sx < rx1 {
        let wx = (sx + scroll_i).rem_euclid(sheen_period);
        if wx < 42 {
            let len = (42 - wx).min(rx1 - sx);
            c.fill(sx, sy, len, 1, mix(row_base, light, 0.28));
            sx += len;
        } else {
            sx += sheen_period - wx;
        }
    }

    // 3) High-speed horizontal aggregate & grit streaks (4–11 px long motion blurs!).
    //    Anchored to world cell `((wx + scroll_i).div_euclid(16))` so crop == whole!
    let step = 16;
    let mut cell = (rx0 + scroll_i).div_euclid(step);
    let end_cell = (rx1 - 1 + scroll_i).div_euclid(step);
    while cell <= end_cell {
        let h = hash(cell, wy.div_euclid(2) ^ 0x51a7);
        if (h & 3) == 0 {
            let sub = ((h >> 4) & 7) as i32;
            let len = 4 + ((h >> 8) & 7) as i32; // 4..11 px motion-blurred streak!
            let x_start = cell * step + sub - scroll_i;
            let x_clamped = x_start.max(rx0);
            let x_end = (x_start + len).min(rx1);
            if x_clamped < x_end {
                let tone = match (h >> 12) & 3 {
                    0 => streak,
                    1 => dark,
                    _ => light,
                };
                c.fill(x_clamped, sy, x_end - x_clamped, 1, tone);
            }
        }
        cell += 1;
    }

    // 4) Diagonal glossy-black tar repair seams (every ~176 px, crooked by row).
    let tar_period = 176;
    let tar_skew = (wy * 3) / 2 + ((hash(wy / 5, 0x3b) & 3) as i32);
    let mut tx = rx0;
    while tx < rx1 {
        let phase = (tx + scroll_i + tar_skew).rem_euclid(tar_period);
        if phase < 2 {
            let len = (2 - phase).min(rx1 - tx);
            c.fill(tx, sy, len, 1, scale(TAR_SEAM, shade));
            tx += len;
        } else {
            tx += tar_period - phase;
        }
    }

    // 5) Occasional recessed pothole / road-patch flicker (every ~320 px).
    let pot_period = 320;
    let mut pcell = (rx0 + scroll_i).div_euclid(pot_period);
    let pend = (rx1 - 1 + scroll_i).div_euclid(pot_period);
    while pcell <= pend {
        let ph = hash(pcell, 0x993);
        let py = road_top + 14 + (ph as i32).rem_euclid((road_bot - road_top - 28).max(1));
        let dy = (wy - py).abs();
        if dy <= 3 {
            let half_w = if dy == 0 {
                7
            } else if dy == 1 {
                6
            } else if dy == 2 {
                4
            } else {
                2
            };
            let px_center = pcell * pot_period + 64 + ((ph >> 8) & 63) as i32 - scroll_i;
            let lx = (px_center - half_w).max(rx0);
            let rx = (px_center + half_w).min(rx1);
            if lx < rx {
                if dy == 3 {
                    // Pale crumbled asphalt rim catching sunlight.
                    c.fill(lx, sy, rx - lx, 1, light);
                } else {
                    c.fill(lx, sy, rx - lx, 1, scale(TAR_SEAM, shade));
                    if dy <= 1 && px_center + half_w > rx0 && px_center + half_w - 1 < rx1 {
                        c.px(px_center + half_w - 1, sy, light);
                    }
                }
            }
        }
        pcell += 1;
    }

    // 6) Bold highway lane dashes + cat's-eye road studs!
    //    Center of the trailer straddles a double-yellow highway center line;
    //    flanking bands carry bright white highway lane dashes.
    let center_y = DECK_TOP_PX + (DECK_H_PX as i32) / 2;
    let is_center_band = (wy - center_y).abs() <= 4;
    if is_center_band && ((wy - center_y).abs() == 1 || (wy - center_y).abs() == 2) {
        let dash_col = scale(LANE_YELLOW, shade);
        let dash_fade = mix(row_base, dash_col, 0.50);
        let stud_col = scale(STUD_AMBER, (shade * 1.08).min(1.0));
        let mut lx = rx0;
        while lx < rx1 {
            let wx = (lx + scroll_i).rem_euclid(96);
            if wx < 42 {
                let len = (42 - wx).min(rx1 - lx);
                c.fill(lx, sy, len, 1, dash_col);
                let fade_end = (3 - wx).max(0).min(len);
                if fade_end > 0 {
                    c.fill(lx, sy, fade_end, 1, dash_fade);
                }
                lx += len;
            } else if (66..=67).contains(&wx) {
                c.px(lx, sy, stud_col);
                lx += 1;
            } else if wx < 66 {
                lx += 66 - wx;
            } else {
                lx += 96 - wx;
            }
        }
    } else if lane_local <= 2 {
        // White highway lane dash (2–3 px thick with faded speed-blurred ends + cat's-eye stud).
        let dash_col = scale(LANE_WHITE, shade * if lane_local == 1 { 1.0 } else { 0.82 });
        let dash_fade = mix(row_base, dash_col, 0.48);
        let stud_col = scale(STUD, (shade * 1.12).min(1.0));
        let mut lx = rx0;
        while lx < rx1 {
            let wx = (lx + scroll_i).rem_euclid(112);
            if wx < 48 {
                let len = (48 - wx).min(rx1 - lx);
                c.fill(lx, sy, len, 1, dash_col);
                let fade_end = (4 - wx).max(0).min(len);
                if fade_end > 0 {
                    c.fill(lx, sy, fade_end, 1, dash_fade);
                }
                lx += len;
            } else if (78..=79).contains(&wx) && lane_local == 1 {
                c.px(lx, sy, stud_col);
                lx += 1;
            } else if wx < 78 {
                lx += 78 - wx;
            } else {
                lx += 112 - wx;
            }
        }
    }

    // 7) Solid highway shoulder lines framing the asphalt strips outside the rails.
    if !under_deck {
        if (wy - (road_top + 3)).abs() <= 1 || (wy - (road_bot - 4)).abs() <= 1 {
            let edge_col = mix(row_base, scale(LANE_WHITE, shade), 0.72);
            c.fill(rx0, sy, rx1 - rx0, 1, edge_col);
        }
        // Gravel shoulder dust transition right at the very outer edge of the road.
        if wy <= road_top + 1 || wy >= road_bot - 2 {
            c.fill(rx0, sy, rx1 - rx0, 1, mix(GRAVEL, dark, 0.45));
        }
    }
}

/// Overlay static trailer steel undercarriage (I-beam cross-members, chassis
/// rails and tandem-axle shadow bands) inside a gap scanline so the rushing
/// road visibly streams behind the trailer's skeleton!
fn undercarriage_in_gap(c: &mut Canvas<'_>, sy: i32, wy: i32, depth: i32, gap_h: i32) {
    let x0 = c.x0.max(DECK_LEFT_PX);
    let x1 = c.x1.min(DECK_RIGHT_PX);
    if sy < c.y0 || sy >= c.y1 || x0 >= x1 {
        return;
    }

    // 1) Two broad tandem-axle & wheel-bogie undercarriage shadow zones (static on the trailer).
    let bed_len = DECK_RIGHT_PX - DECK_LEFT_PX;
    let axle1 = DECK_LEFT_PX + bed_len / 5;
    let axle2 = DECK_LEFT_PX + (bed_len * 3) / 4;
    for ax in [axle1, axle2] {
        // Broad soft undercarriage shadow + dense twin-axle housing bars.
        c.darken_rect(ax - 34, sy, 68, 1, 0.62);
        c.darken_rect(ax - 16, sy, 32, 1, 0.68);
        c.fill(ax - 12, sy, 5, 1, STEEL_LO);
        c.fill(ax + 7, sy, 5, 1, STEEL_LO);
        if depth == gap_h / 2 {
            c.px(ax - 10, sy, STEEL);
            c.px(ax + 9, sy, STEEL);
        }
    }

    // 2) Twin longitudinal steel chassis I-beam rails running the length of the bed.
    let rail_n = DECK_TOP_PX + (DECK_H_PX as i32) / 3;
    let rail_s = DECK_TOP_PX + ((DECK_H_PX as i32) * 2) / 3;
    for rail_y in [rail_n, rail_s] {
        let d = (wy - rail_y).abs();
        if d <= 2 {
            c.fill(x0, sy, x1 - x0, 1, if d == 0 { STEEL } else { STEEL_LO });
        } else if d <= 5 {
            c.darken_rect(x0, sy, x1 - x0, 1, 0.65);
        }
    }

    // 3) Transverse steel cross-members every 80 px (the road dashes blink behind them!).
    let member_step = 80;
    let mut m = (x0 - 20).div_euclid(member_step) * member_step + 20;
    while m < x1 + 8 {
        if m + 8 > x0 && m - 2 < x1 {
            // Cast shadow on the asphalt right beside the cross-member.
            c.darken_rect(m - 2, sy, 9, 1, 0.56);
            // Steel flange: shaded edge, bright spine, bolted centre.
            c.fill(m, sy, 4, 1, STEEL_LO);
            c.px(m + 1, sy, STEEL);
            if depth == 1 || depth == gap_h - 1 {
                c.px(m + 2, sy, mix(STEEL, STEEL_HI, 0.4));
            }
        }
        m += member_step;
    }
}

/// The deck: weathered oak/pine boards with wide irregular gaps, warped edges,
/// rich timber grain, butt joints, knots, oil stains, carriage bolts, and
/// sunlit road-glare flicker along the gap lips.
fn planks(
    c: &mut Canvas<'_>,
    deck_top: i32,
    deck_bot: i32,
    scroll_i: i32,
    bump: i32,
    pal: &Palette,
) {
    let total = deck_bot - deck_top;
    if total <= 0 {
        return;
    }
    let x0 = c.x0.max(DECK_LEFT_PX);
    let x1 = c.x1.min(DECK_RIGHT_PX);
    if x0 >= x1 {
        return;
    }
    let road_top = deck_top - ROAD_STRIP * PX;
    let road_bot = deck_bot + ROAD_STRIP * PX;
    let (layout, count) = &PLANK_LAYOUT;

    for spec in &layout[..*count] {
        let wy_board = deck_top + spec.y_off;
        let y = wy_board + bump;
        let bh = spec.board_h;
        let gh = spec.gap_h;
        if y + bh + gh <= c.y0 || y >= c.y1 {
            continue;
        }
        let row = spec.row_id;
        let n = hash(row, 0x71);
        let body = WOOD[(n as usize) % WOOD.len()];
        let grain_lo = mix(body, WOOD_DARK, 0.46);
        let grain_hi = mix(body, WOOD_LIGHT, 0.30);
        let top_lip = mix(body, WOOD_LIGHT, 0.50);
        let bot_lip = mix(body, WOOD_DARK, 0.65);

        // 1) Base weathered plank body + directional bevel.
        c.fill(x0, y, x1 - x0, bh, body);
        c.fill(x0, y, x1 - x0, 1, top_lip);
        if bh >= 3 {
            c.fill(x0, y + bh - 1, x1 - x0, 1, bot_lip);
        }

        // 2) Flowing broken wood-grain fibers (segmented along world x so grain feels organic!).
        let gstep = 24;
        let mut gcell = x0.div_euclid(gstep);
        let gend = (x1 - 1).div_euclid(gstep);
        while gcell <= gend {
            let gh_hash = hash(gcell, row * 13 + 7);
            let gx = gcell * gstep;
            let seg_w = 12 + (gh_hash & 11) as i32;
            let gy1 = y + 1 + ((gh_hash >> 4) as i32 % (bh - 2).max(1));
            c.fill(gx, gy1, seg_w, 1, grain_lo);
            if bh >= 6 {
                let gy2 = y + 2 + ((gh_hash >> 9) as i32 % (bh - 3).max(1));
                c.fill(gx + 5, gy2, (seg_w - 3).max(4), 1, grain_hi);
            }
            // Subtle sunlit highway glare flickering up onto the bottom edge of the plank
            // whenever a bright lane dash or light sheen passes right underneath!
            let road_glare = ((gx + scroll_i) as u32).wrapping_mul(0x045d_9f3b) >> 24;
            if road_glare > 218 && bh >= 4 {
                c.fill(gx + 2, y + bh - 1, 7, 1, mix(bot_lip, WOOD_LIGHT, 0.45));
            }
            gcell += 1;
        }

        // 3) Butt joints: staggered per board, with pale splintered end-grain & recessed seam.
        let step = 52 + (hash(row, 0x17) % 7) as i32 * 8;
        let off = (n >> 8) as i32 % step;
        let mut joint = (x0 - off).div_euclid(step) * step + off;
        while joint < x1 + 4 {
            if joint + 4 > x0 && joint < x1 {
                c.fill(joint, y, 1, bh, RIP);
                c.fill(joint + 1, y + 1, 1, (bh - 2).max(1), WOOD_END);
                if bh >= 5 {
                    c.fill(joint + 2, y + 2, 1, bh - 4, mix(body, WOOD_END, 0.55));
                }
                // Paired carriage bolts flanking the butt joint.
                if bh >= 5 {
                    c.px(joint - 2, y + 1, BOLT_RING);
                    c.px(joint - 2, y + bh - 2, BOLT_RING);
                    c.px(joint + 3, y + 1, BOLT);
                    c.px(joint + 3, y + bh - 2, BOLT);
                }
            }
            joint += step;
        }

        // 4) Knots with concentric dark ring.
        if n & 5 == 1 && bh >= 5 {
            let kstep = 112;
            let koff = (hash(row, 0x33) as i32).rem_euclid(kstep);
            let mut kx = (x0 - koff).div_euclid(kstep) * kstep + koff;
            while kx < x1 + 4 {
                if kx + 4 > x0 {
                    let ky = y + 2 + ((n >> 12) as i32 % (bh - 4).max(1));
                    c.fill(kx - 1, ky, 4, 1, grain_lo);
                    c.fill(kx, ky, 2, 2, KNOT);
                    c.px(kx + 2, ky + 1, mix(body, KNOT, 0.55));
                }
                kx += kstep;
            }
        }

        // 5) Oil slicks & forklift tyre scuffs across weathered planks.
        if n & 15 == 5 && bh >= 5 {
            let ostep = 176;
            let ooff = (hash(row, 0x51) as i32).rem_euclid(ostep);
            let mut ox = (x0 - ooff).div_euclid(ostep) * ostep + ooff;
            while ox < x1 + 18 {
                if ox + 18 > x0 {
                    c.fill(ox, y + 1, 16, (bh - 3).max(2), mix(body, OIL, 0.52));
                    c.fill(ox + 3, y + bh - 2, 10, 1, mix(body, OIL, 0.36));
                }
                ox += ostep;
            }
        }

        // 6) Structural carriage-bolt rows aligned with the steel cross-members every 80 px!
        let mut bx = (x0 - 20).div_euclid(80) * 80 + 21;
        while bx < x1 {
            if bx >= x0 {
                c.px(bx, y + 1, BOLT);
                if bh >= 5 {
                    c.px(bx, y + bh - 2, mix(body, BOLT, 0.65));
                }
                c.px(bx + 1, y + 1, BOLT_RING);
            }
            bx += 80;
        }

        // 7) The wide irregular road gap beneath this plank!
        if gh > 0 {
            let gy_top = y + bh;
            let wy_gap = wy_board + bh;

            // Row 0 of gap: deep plank overhang shadow lip.
            c.fill(x0, gy_top, x1 - x0, 1, RIP);

            // Middle rows of gap: LIVE rushing highway with depth-stepped shading!
            for d in 1..gh {
                let sy = gy_top + d;
                let wy = wy_gap + d;
                // Shaded near top lip, catching full sunlit highway light in the center!
                let shade = if d == 1 {
                    0.76
                } else if d == gh - 1 && gh >= 5 {
                    0.94
                } else {
                    1.04
                };
                draw_road_scanline(c, x0, x1, sy, wy, scroll_i, road_top, road_bot, true, shade);
                undercarriage_in_gap(c, sy, wy, d, gh);
            }

            // Organic irregular warped/chipped plank edges along the gap!
            // Occasional 1 px splinter nibs poking slightly into the top/bottom of the gap,
            // or a 1 px chipped bite out of the plank edge so cracks look naturally aged.
            let wstep = 18;
            let mut wc = x0.div_euclid(wstep);
            let wend = (x1 - 1).div_euclid(wstep);
            while wc <= wend {
                let wh = hash(wc, row ^ 0x4b1);
                let wx = wc * wstep + ((wh >> 3) & 7) as i32;
                let wlen = 3 + ((wh >> 7) & 5) as i32;
                if (wh & 7) == 0 && bh >= 5 {
                    // Chipped gouge widening the crack by 1 px into the plank bottom!
                    c.fill(wx, gy_top - 1, wlen, 1, RIP);
                    c.fill(wx + 1, gy_top - 2, (wlen - 2).max(1), 1, WOOD_END);
                } else if (wh & 7) == 3 {
                    // Warped plank edge bulging 1 px down into the shadow lip.
                    c.fill(wx, gy_top, wlen, 1, bot_lip);
                }
                wc += 1;
            }
        }
        let _ = pal;
    }
}

/// Does the hole's art cover the world pixel `(wx, wy)`? The drawing fills
/// exactly these pixels, and the predicate is [`Pit::contains`] on the pixel's
/// centre — so the sim's fall test, the flood's cell test and the picture can
/// never disagree. `bump` is the deck's one-pixel expansion-joint dip.
pub fn hole_pixel(pit: &Pit, wx: i32, wy: i32, bump: i32) -> bool {
    pit.contains(Vec2::new(wx as f32 + 0.5, (wy - bump) as f32 + 0.5))
}

/// A torn hole: the rushing highway seen through the pit's own irregular
/// outline, exactly as the sim reads it ([`hole_pixel`] decides every pixel),
/// a dark lip hugging the rim, splintered plank ends overhanging it, and the
/// exposed undercarriage bar deep in the void.
///
/// The outline is the single source of truth: the deck planks are painted
/// first, then the hole is punched out of them pixel by pixel, so the sim's
/// fall test, the flood's cell test and the picture can never disagree.
fn torn_hole(c: &mut Canvas<'_>, pit: &Pit, scroll_i: i32, bump: i32) {
    let b = pit.bounds();
    let px = b.x * PX;
    let py = b.y * PX + bump;
    let pw = b.w * PX;
    let ph = b.h * PX;
    if pw <= 0 || ph <= 0 {
        return;
    }
    let road_top = DECK_TOP_PX - ROAD_STRIP * PX;
    let road_bot = deck_bot_px() + ROAD_STRIP * PX;
    let inside = |wx: i32, sy: i32| hole_pixel(pit, wx, sy, bump);

    // 1) The rushing highway through the hole, row by row, run by run.
    let y0 = py.max(c.y0);
    let y1 = (py + ph).min(c.y1);
    let x0 = px.max(c.x0);
    let x1 = (px + pw).min(c.x1);
    for sy in y0..y1 {
        let wy = sy - bump;
        let mut wx = x0;
        while wx < x1 {
            if !inside(wx, sy) {
                wx += 1;
                continue;
            }
            let run_start = wx;
            while wx < x1 && inside(wx, sy) {
                wx += 1;
            }
            // Shade by depth from the hole's north rim: the near lip is in
            // shadow, the open middle catches full daylight.
            let depth = sy - py;
            let shade = match depth {
                0 => 0.48,
                1 => 0.72,
                2 => 0.92,
                _ => 1.14,
            };
            draw_road_scanline(
                c, run_start, wx, sy, wy, scroll_i, road_top, road_bot, false, shade,
            );
        }
    }

    // 2) A rusted steel undercarriage rod running down the void, for scale.
    //    Only the pixels the outline covers, so the rod never paints planks.
    if pw >= 12 {
        let bar_x = px + pw / 2 - 1;
        for sy in y0..y1 {
            for k in 0..2 {
                let wx = bar_x + k;
                if inside(wx, sy) {
                    c.px(wx, sy, mix(STEEL_LO, RUST, 0.45));
                }
            }
        }
    }

    // 3) The dark lip: one pixel just outside the outline all round, so the
    //    edge reads as a drop at 1x as well as 2x.
    for sy in y0.saturating_sub(1)..(y1 + 1).min(c.y1) {
        for wx in x0.saturating_sub(1)..(x1 + 1).min(c.x1) {
            if inside(wx, sy) {
                continue;
            }
            let neighbour = inside(wx - 1, sy)
                || inside(wx + 1, sy)
                || inside(wx, sy - 1)
                || inside(wx, sy + 1);
            if neighbour {
                c.px(wx, sy, RIP);
            }
        }
    }

    // 4) Splintered, broken plank ends overhanging the rim: a few jagged stubs
    //    of fresh wood where the boards tore, stable frame to frame.
    for (cx, cy) in pit.rim_cells() {
        let mid = cy * PX + PX / 2 + bump;
        if mid < c.y0 || mid >= c.y1 {
            continue;
        }
        let h = hash(cx, cy);
        if h % 3 == 0 {
            continue;
        }
        // Walk out from the cell's centre to the rim, then stub outward.
        let centre = cx * PX + PX / 2;
        let mut edge = centre;
        let mut dir = 1;
        while edge > cx * PX && edge < cx * PX + PX && inside(edge, mid) {
            edge += dir;
        }
        if edge >= cx * PX + PX || edge < cx * PX {
            dir = -1;
            edge = centre;
            while edge > cx * PX && edge < cx * PX + PX && inside(edge, mid) {
                edge += dir;
            }
        }
        let stub = 1 + (h % 3) as i32;
        for k in 0..stub {
            let wx = edge + dir * k;
            if wx < cx * PX || wx >= cx * PX + PX {
                break;
            }
            c.px(wx, mid, WOOD_END);
            c.px(wx, mid + 1, SPLINTER);
        }
    }
}

/// A broken-plank pit: unmistakably reads as "YOU CAN FALL THROUGH HERE" —
/// bright sunlit rushing highway below, deep 3 px cast interior drop shadow,
/// jagged freshly-snapped timber ends matching the deck planks, and an exposed
/// rusted undercarriage bar deep in the void!
fn broken_planks(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scroll_i: i32,
    bump: i32,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let road_top = DECK_TOP_PX - ROAD_STRIP * PX;
    let road_bot = deck_bot_px() + ROAD_STRIP * PX;

    // 1) Full bright sunlit rushing highway right through the broken opening!
    let rx0 = x.max(c.x0);
    let rx1 = (x + w).min(c.x1);
    let ry0 = y.max(c.y0);
    let ry1 = (y + h).min(c.y1);
    for sy in ry0..ry1 {
        let wy = sy - bump;
        let depth_top = sy - y;
        let shade = if depth_top == 0 {
            0.48
        } else if depth_top == 1 {
            0.72
        } else if depth_top == 2 {
            0.92
        } else {
            1.14 // bright open daylight hitting the rushing road below!
        };
        draw_road_scanline(c, rx0, rx1, sy, wy, scroll_i, road_top, road_bot, false, shade);
    }

    // 2) A single rusted steel undercarriage rod visible deep across the pit for scale/depth.
    if w >= 12 {
        let bar_x = x + w / 2 - 1;
        c.darken_rect(bar_x - 1, y + 1, 4, h - 1, 0.58);
        c.fill(bar_x, y + 1, 2, h - 2, mix(STEEL_LO, RUST, 0.45));
    }

    // 3) Deep drop-off cast shadow along the north & west interior walls of the hole.
    c.fill(x, y, w, 1, RIP);
    c.darken_rect(x, y + 1, w, 2, 0.55);
    c.fill(x, y, 1, h, RIP);
    c.darken_rect(x + 1, y, 2, h, 0.62);
    c.fill(x + w - 1, y, 1, h, RIP);
    c.fill(x, y + h - 1, w, 1, RIP);

    // 4) Jagged snapped wooden plank stubs jutting raggedly into the left & right edges,
    //    with bright freshly-splintered inner wood and dark tear shadows!
    for k in 1..(h - 1) {
        let wy = y + k - bump;
        let on_plank = ((wy - DECK_TOP_PX).rem_euclid(11)) < 7;
        if !on_plank {
            continue;
        }
        let tl = (hash(x, wy) % 4) as i32;
        if tl > 0 {
            c.fill(x, y + k, tl, 1, WOOD_END);
            c.px(x + tl, y + k, SPLINTER);
            c.px(x + tl, y + k + 1, RIP);
        }
        let tr = (hash(x + w, wy) % 4) as i32;
        if tr > 0 {
            c.fill(x + w - tr, y + k, tr, 1, WOOD_END);
            c.px(x + w - 1 - tr, y + k, SPLINTER);
            c.px(x + w - 1 - tr, y + k + 1, RIP);
        }
    }
    // Splintered front lip catching top-left sunlight so the hole perimeter pops.
    for k in 1..(w - 1) {
        let tooth = (hash(x + k, y + h) % 3) as i32;
        if tooth > 0 {
            c.fill(x + k, y + h - tooth, 1, tooth, WOOD_END);
            c.px(x + k, y + h - tooth - 1, SPLINTER);
        }
    }
    // Subtle exposed bent nail glints along the broken lip.
    if w >= 10 {
        c.px(x + 2, y + h - 1, BOLT);
        c.px(x + w - 3, y + 1, BOLT);
    }
}

/// A sheared rail gap: open highway streaming past the deck edge, drop shadow,
/// and twisted rusty steel rail stubs at both ends.
fn rail_gap(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scroll_i: i32,
    bump: i32,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let road_top = DECK_TOP_PX - ROAD_STRIP * PX;
    let road_bot = deck_bot_px() + ROAD_STRIP * PX;
    let rx0 = x.max(c.x0);
    let rx1 = (x + w).min(c.x1);
    let ry0 = y.max(c.y0);
    let ry1 = (y + h).min(c.y1);
    for sy in ry0..ry1 {
        let wy = sy - bump;
        let shade = if sy - y <= 1 { 0.62 } else { 1.06 };
        draw_road_scanline(c, rx0, rx1, sy, wy, scroll_i, road_top, road_bot, false, shade);
    }
    // Deep drop-off shadow under the exposed timber edge.
    c.fill(x, y, w, 2, RIP);
    c.darken_rect(x, y + 2, w, 2, 0.58);
    // Sheared, twisted rusty steel rail stubs + dangling chain links at both sides.
    for (sx, dir) in [(x - 3, 1i32), (x + w, -1)] {
        c.fill(sx, y + 1, 3, h - 2, STEEL_LO);
        c.fill(sx + if dir > 0 { 0 } else { 1 }, y + 2, 2, 2, STEEL_HI);
        c.fill(sx, y + h - 4, 3, 2, RUST);
        // Jagged sheared steel end & dangling link.
        c.px(sx + if dir > 0 { 3 } else { -1 }, y + 3, STEEL_HI);
        c.px(sx + if dir > 0 { 3 } else { -1 }, y + 6, RUST);
        c.px(sx + if dir > 0 { 4 } else { -2 }, y + 7, STEEL);
    }
}

/// Pale scuffed skid marks scratched into the deck planks along a sliding crate's lane.
fn slide_track(c: &mut Canvas<'_>, rest: Vec2, dir: Vec2, span: f32, bump: i32) {
    let steps = (span.abs() / 3.0).ceil() as i32;
    let perp = Vec2::new(-dir.y, dir.x);
    let scuff = mix(WOOD[2], WOOD_LIGHT, 0.35);
    for k in 0..=steps {
        let p = rest + dir * (k as f32 * 3.0).min(span);
        for side in [-5.0f32, 5.0] {
            let q = p + perp * side;
            let qx = q.x.round() as i32;
            let qy = q.y.round() as i32 + bump;
            c.px(qx, qy, scuff);
        }
    }
}

/// Heavy strapped freight crate that slides along its ratchet straps.
/// During `telling`, the crate shudders and both orange ratchet straps snap
/// blazing hazard-yellow/white with flashing sparks at the steel buckles!
fn slide_crate(
    c: &mut Canvas<'_>,
    pos: Vec2,
    bump: i32,
    telling: bool,
    warn_p: f32,
    elapsed: f32,
) {
    let rattle = if telling {
        ((elapsed * 44.0).sin() * 1.1).round() as i32
    } else {
        0
    };
    let x = pos.x.round() as i32 - 10 + rattle;
    let y = pos.y.round() as i32 - 9 + bump;
    if x + 24 < c.x0 || x - 4 > c.x1 || y + 22 < c.y0 || y - 4 > c.y1 {
        return;
    }

    // Grounded cast shadow down-right.
    c.darken_rect(x + 2, y + 3, 20, 17, 0.48);
    c.fill(x + 1, y + 17, 19, 2, RIP);

    // Outer beveled timber frame.
    c.fill(x, y, 20, 17, RIP);
    c.fill(x + 1, y + 1, 18, 15, WOOD_DARK);
    c.fill(x + 2, y + 2, 16, 13, WOOD[2]);
    // Top-left lit frame rails + recessed dark inner border.
    c.fill(x + 1, y + 1, 18, 1, WOOD_LIGHT);
    c.fill(x + 1, y + 1, 1, 15, mix(WOOD[2], WOOD_LIGHT, 0.45));
    c.fill(x + 1, y + 15, 18, 1, WOOD_DARK);
    // Horizontal plank grooves.
    for ky in [5, 8, 11] {
        c.fill(x + 2, y + ky, 16, 1, mix(WOOD[2], WOOD_DARK, 0.55));
    }
    // Diagonal timber cross-brace across the crate face.
    for i in 0..11 {
        let bx = x + 3 + (i * 13) / 11;
        let by = y + 3 + i;
        c.px(bx, by, WOOD_END);
        c.px(bx + 1, by, mix(WOOD[2], WOOD_DARK, 0.4));
    }
    // Corner iron brackets.
    for (cx, cy) in [(2, 2), (17, 2), (2, 14), (17, 14)] {
        c.px(x + cx, y + cy, STEEL_HI);
    }

    // Two high-visibility ratchet straps tied across the crate!
    // Calm: safety orange. Telling: blazing pulsing yellow-white with travelling tension spark!
    let pulse = 0.5 + 0.5 * (elapsed * 18.0).sin();
    let strap_col = if telling {
        mix(STRAP_ORANGE, STRAP_HOT, 0.45 + 0.55 * pulse.max(warn_p))
    } else {
        STRAP_ORANGE
    };
    let strap_edge = if telling {
        (255, 252, 224)
    } else {
        mix(STRAP_ORANGE, WOOD_DARK, 0.45)
    };
    let travel_y = ((elapsed * 26.0) as i32).rem_euclid(15);

    for sx in [x + 4, x + 14] {
        // Strap anchors biting past top/bottom edges of the crate.
        c.fill(sx, y - 1, 2, 19, strap_col);
        c.fill(sx + 1, y, 1, 17, strap_edge);
        // Heavy steel ratchet buckle in the middle.
        c.fill(sx - 1, y + 7, 4, 3, STEEL_LO);
        c.fill(sx, y + 8, 2, 1, if telling { STUD } else { STEEL_HI });
        if telling {
            // Travelling tension glint + buckle warning spark!
            c.px(sx, y + 1 + travel_y, STUD);
            c.px(sx + 1, y + 1 + travel_y, STRAP_HOT);
            if pulse > 0.45 {
                c.px(sx - 1, y + 6, STRAP_HOT);
                c.px(sx + 2, y + 10, STUD);
            }
        }
    }
}

/// Pale slipstream streaks drifting toward the tailgate inside a wind stretch.
fn wind_streaks(
    c: &mut Canvas<'_>,
    rect: crate::game::level::Rectf,
    bump: i32,
    scroll: f32,
) {
    let y0 = rect.y.round() as i32 + bump;
    let h = rect.h.round() as i32;
    let x0 = rect.x.round() as i32;
    let w = rect.w.round() as i32;
    let ink = mix(ASPHALT_STREAK, LANE_WHITE, 0.42);
    let scroll_i = scroll.floor() as i32;
    for k in 0..4 {
        let yy = y0 + 2 + k * (h.max(1) / 4);
        let step = 28;
        let mut cell = (x0 + scroll_i).div_euclid(step);
        let end_cell = (x0 + w + scroll_i).div_euclid(step);
        while cell <= end_cell {
            let n = hash(cell, k * 19 + 3);
            if (n & 3) == 0 {
                let len = 5 + ((n >> 4) & 7) as i32;
                let sx = cell * step + ((n >> 8) & 7) as i32 - scroll_i;
                let lx = sx.max(x0);
                let rx = (sx + len).min(x0 + w);
                if lx < rx {
                    c.fill(lx, yy, rx - lx, 1, ink);
                }
            }
            cell += 1;
        }
    }
}

// ---- Wall cells: cargo, rails, tailgate, cab bulkhead and the verge --------

/// What a solid cell of the trailer maps is.
enum DeckPart {
    Cargo(u32),
    Tailgate,
    Bulkhead,
    Rail { south: bool },
    Road,
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
    if cell_y == RAIL_TOP {
        return DeckPart::Rail { south: false };
    }
    if cell_y == RAIL_BOTTOM {
        return DeckPart::Rail { south: true };
    }
    if (RAIL_TOP - ROAD_STRIP..RAIL_TOP).contains(&cell_y)
        || (RAIL_BOTTOM + 1..=RAIL_BOTTOM + ROAD_STRIP).contains(&cell_y)
    {
        return DeckPart::Road;
    }
    DeckPart::Verge
}

/// The ground layer: nothing (contact shadows live on cargo/deck).
pub fn draw_wall_ground(
    _pb: &mut PixelBuffer,
    _view: &super::art_walls::WallCellView,
    _cam: Vec2,
    _accent: Rgb,
) {
}

/// The body layer: cargo blocks, steel rails, striped tailgate, cab bulkhead and verge.
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
        DeckPart::Road => {}
        DeckPart::Cargo(n) => cargo_cell(pb, sx, sy, view.cell_x, n, view.anim_clock, view.mask),
        DeckPart::Tailgate => tailgate(pb, sx, sy, view.cell_y),
        DeckPart::Bulkhead => bulkhead(pb, sx, sy, view.cell_y),
        DeckPart::Rail { south } => rail(pb, sx, sy, view.cell_x, south, view.anim_clock),
        DeckPart::Verge => verge(pb, sx, sy, view.cell_x, view.cell_y, view.anim_clock),
    }
}

/// The overhang layer: nothing hangs over the flatbed deck.
pub fn draw_wall_overhang(
    _pb: &mut PixelBuffer,
    _view: &super::art_walls::WallCellView,
    _cam: Vec2,
    _accent: Rgb,
) {
}

/// One solid cargo stack cell: heavy wooden crate with cross-brace, stacked
/// pallet, strapped tarped machinery, or shaded gunmetal steel pipe bundle —
/// with realistic top-left lighting, south face height, cast shadow, and
/// subtle highway light flicker grazing the underside from the deck gaps!
fn cargo_cell(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    cell_x: i32,
    n: u32,
    elapsed: f32,
    mask: super::art_walls::NeighborMask,
) {
    use super::art_walls::NeighborMask;
    let variant = (n >> 4) & 3;
    let body = WOOD[(n as usize) % 5];
    match variant {
        0 => {
            // Heavy framed wooden freight crate with diagonal brace, corner irons & stencil.
            pb.rect_fill(sx, sy, PX, PX, WOOD_DARK);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, PX - 2, body);
            // Raised perimeter frame rails.
            pb.rect_fill(sx + 1, sy + 1, PX - 2, 2, mix(body, WOOD_LIGHT, 0.52));
            pb.rect_fill(sx + 1, sy + 1, 2, PX - 2, mix(body, WOOD_LIGHT, 0.30));
            pb.rect_fill(sx + 3, sy + 3, PX - 6, PX - 6, mix(body, WOOD_DARK, 0.18));
            // Recessed plank seams inside the crate face.
            pb.rect_fill(sx + 3, sy + 6, PX - 6, 1, mix(body, WOOD_DARK, 0.55));
            pb.rect_fill(sx + 3, sy + 10, PX - 6, 1, mix(body, WOOD_DARK, 0.55));
            // Diagonal cross-brace timber.
            for k in 0..9 {
                pb.plot(sx + 3 + k, sy + 3 + k, WOOD_END);
                if k < 8 {
                    pb.plot(sx + 4 + k, sy + 3 + k, mix(body, WOOD_DARK, 0.45));
                }
            }
            // Corner iron brackets + optional ratchet strap.
            for (cx, cy) in [(2, 2), (PX - 3, 2), (2, PX - 3), (PX - 3, PX - 3)] {
                pb.plot(sx + cx, sy + cy, STEEL_HI);
            }
            if (n & 7) == 0 {
                pb.rect_fill(sx + 7, sy, 2, PX, STRAP_ORANGE);
                pb.rect_fill(sx + 7, sy + 7, 2, 2, STEEL_LO);
            }
        }
        1 => {
            // Stacked timber pallet with dark forklift channels & nail heads.
            pb.rect_fill(sx, sy, PX, PX, RIP);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, PX - 2, mix(WOOD_DARK, RIP, 0.55));
            for k in 0..4 {
                let yy = sy + 1 + k * 4;
                let slat = WOOD[((n as usize) + k as usize) % 5];
                pb.rect_fill(sx + 1, yy, PX - 2, 3, slat);
                pb.rect_fill(sx + 1, yy, PX - 2, 1, mix(slat, WOOD_LIGHT, 0.42));
                pb.plot(sx + 3, yy + 1, BOLT);
                pb.plot(sx + PX - 4, yy + 1, BOLT);
            }
        }
        2 => {
            // Heavy draped canvas tarp over machinery, with wind creases, grommets & ratchet strap.
            let tarp = mix(TARP, TARP_HI, (n & 7) as f32 / 14.0);
            pb.rect_fill(sx, sy, PX, PX, TARP_LO);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, PX - 2, tarp);
            pb.rect_fill(sx + 1, sy + 1, PX - 2, 2, TARP_HI);
            // Diagonal fabric folds & tension ridges.
            for k in 0..10 {
                pb.plot(sx + 2 + k, sy + 4 + k / 3, TARP_HI);
                pb.plot(sx + 3 + k, sy + 9 - k / 4, TARP_LO);
            }
            // High-visibility tie-down strap + brass grommets.
            let strap_x = sx + 6 + ((n >> 7) & 3) as i32;
            pb.rect_fill(strap_x, sy + 1, 2, PX - 2, STRAP_ORANGE);
            pb.plot(strap_x, sy + 7, STEEL_HI);
            pb.plot(sx + 2, sy + PX - 3, BOLT);
            pb.plot(sx + PX - 3, sy + PX - 3, BOLT);
        }
        _ => {
            // Lengthwise stacked steel pipes (3 horizontal cylinders strapped down
            // across the bed, with brushed-metal highlights, shadow troughs, and
            // hollow dark pipe mouths at exposed ends).
            pb.rect_fill(sx, sy, PX, PX, RIP);
            for (k, py) in [1i32, 6, 11].iter().enumerate() {
                let pipe_mid = if k == 1 {
                    mix(STEEL, STEEL_HI, 0.14)
                } else {
                    STEEL
                };
                let pipe_hi = mix(STEEL, STEEL_HI, 0.72);
                let pipe_sh = mix(STEEL_LO, RIP, 0.25);
                pb.rect_fill(sx + 1, sy + py, PX - 2, 4, pipe_mid);
                pb.rect_fill(sx + 1, sy + py, PX - 2, 1, pipe_hi);
                pb.rect_fill(sx + 1, sy + py + 3, PX - 2, 1, pipe_sh);
                // Recessed dark pipe mouths / welded collars at the ends.
                pb.rect_fill(sx + 1, sy + py + 1, 1, 2, RIP);
                pb.rect_fill(sx + PX - 2, sy + py + 1, 1, 2, pipe_sh);
                // Subtle rust spot on weathered pipes.
                if ((n >> (k * 2)) & 3) == 0 {
                    let rx = sx + 3 + ((n >> (k * 3 + 2)) & 7) as i32;
                    pb.rect_fill(rx, sy + py + 1, 2, 2, mix(STEEL, RUST, 0.55));
                }
            }
            // High-visibility orange ratchet strap + steel buckle cinching the bundle!
            let strap_x = sx + 6 + ((n >> 6) & 3) as i32;
            pb.rect_fill(strap_x, sy, 2, PX, STRAP_ORANGE);
            pb.rect_fill(strap_x, sy + 6, 2, 3, STEEL_LO);
            pb.plot(strap_x, sy + 7, STEEL_HI);
        }
    }

    // 3D height rim + south drop shadow + subtle highway gap-light under-flicker!
    if !mask.has(NeighborMask::N) {
        pb.rect_fill(sx, sy, PX, 1, mix(WOOD_LIGHT, STEEL_HI, 0.25));
    }
    if !mask.has(NeighborMask::W) {
        pb.rect_fill(sx, sy, 1, PX, mix(WOOD_DARK, RIP, 0.30));
    }
    if !mask.has(NeighborMask::S) {
        // Shaded vertical south cargo skirt.
        pb.rect_fill(sx, sy + PX - 3, PX, 2, scale(WOOD_DARK, 0.65));
        pb.rect_fill(sx, sy + PX - 1, PX, 1, RIP);
        // Subtle sunlit highway glare flickering up through the deck cracks onto the cargo skirt!
        let flicker = ((elapsed * 16.0 + cell_x as f32 * 1.9).sin() + 1.0) * 0.5;
        if flicker > 0.62 {
            let glow = mix(WOOD_DARK, LANE_WHITE, (flicker - 0.62) * 0.85);
            pb.rect_fill(sx + 2, sy + PX - 2, PX - 4, 1, glow);
        }
        // Soft cast shadow onto the planks below.
        for dy in 0..3 {
            let f = if dy == 0 { 0.48 } else if dy == 1 { 0.68 } else { 0.84 };
            let py = sy + PX + dy;
            if py >= 0 && (py as usize) < pb.h {
                for lx in 1..PX {
                    let px = sx + lx;
                    if px >= 0 && (px as usize) < pb.w {
                        let cur = pb.pixel_at(px as usize, py as usize);
                        pb.plot(px, py, scale(cur, f));
                    }
                }
            }
        }
    }
    if !mask.has(NeighborMask::E) {
        pb.rect_fill(sx + PX - 1, sy + 1, 1, PX - 1, mix(WOOD_DARK, RIP, 0.40));
    }
}

/// The left tailgate: heavy ribbed steel gate with red/white DOT reflective
/// hazard chevrons, iron hinge knuckles, and rusty safety chain links.
fn tailgate(pb: &mut PixelBuffer, sx: i32, sy: i32, cell_y: i32) {
    pb.rect_fill(sx, sy, PX, PX, STEEL_LO);
    // Ribbed steel plate.
    for k in 0..4 {
        pb.rect_fill(sx + 2, sy + 1 + k * 4, PX - 4, 3, STEEL);
        pb.rect_fill(sx + 2, sy + 1 + k * 4, PX - 4, 1, mix(STEEL, STEEL_HI, 0.5));
    }
    // Red/white DOT conspicuity tape along the outer rear lip.
    let stripe_red = cell_y.rem_euclid(2) == 0;
    pb.rect_fill(sx, sy, 2, PX / 2, if stripe_red { REFLECTOR_RED } else { REFLECTOR_WHT });
    pb.rect_fill(sx, sy + PX / 2, 2, PX / 2, if stripe_red { REFLECTOR_WHT } else { REFLECTOR_RED });
    // Deck-side lit steel rim + heavy hinge knuckles every 3 cells.
    pb.rect_fill(sx + PX - 2, sy, 2, PX, STEEL_HI);
    if cell_y.rem_euclid(3) == 0 {
        pb.rect_fill(sx + 4, sy + 4, 4, 6, RUST);
        pb.rect_fill(sx + 5, sy + 5, 2, 4, STEEL_HI);
    }
}

/// The right cab bulkhead: tall riveted steel headache-rack with vertical
/// stiffening ribs and protective louvers.
fn bulkhead(pb: &mut PixelBuffer, sx: i32, sy: i32, cell_y: i32) {
    pb.rect_fill(sx, sy, PX, PX, STEEL_LO);
    pb.rect_fill(sx + 1, sy, PX - 2, PX, mix(STEEL_LO, STEEL, 0.48));
    // Vertical steel reinforcement ribs.
    pb.rect_fill(sx, sy, 2, PX, STEEL_HI);
    pb.rect_fill(sx + 7, sy, 2, PX, STEEL);
    pb.rect_fill(sx + 9, sy, 1, PX, RIP);
    // Rivet heads & horizontal headache-rack louvers.
    for k in 0..4 {
        let yy = sy + 2 + k * 4;
        pb.plot(sx + 3, yy, STEEL_HI);
        pb.plot(sx + 13, yy, STEEL_HI);
        if cell_y.rem_euclid(2) == 0 {
            pb.rect_fill(sx + 10, yy, 4, 1, RIP);
        }
    }
}

/// One side-rail cell: heavy riveted C-channel steel rub-rail along the deck
/// edge, with welded stake pockets every 4 cells, DOT reflectors, and dynamic
/// highway glare flickering underneath!
fn rail(pb: &mut PixelBuffer, sx: i32, sy: i32, cell_x: i32, south: bool, elapsed: f32) {
    // Let the exposed road strip show slightly at the outer half, steel rub-rail on deck edge.
    let deck_y = if south { sy } else { sy + 4 };
    pb.rect_fill(sx, deck_y, PX, 12, STEEL_LO);
    // Heavy C-channel steel rub-rail (top bevel, recessed web, lower flange).
    pb.rect_fill(sx, deck_y + 1, PX, 3, STEEL);
    pb.rect_fill(sx, deck_y + 1, PX, 1, STEEL_HI);
    pb.rect_fill(sx, deck_y + 4, PX, 3, mix(STEEL_LO, RIP, 0.35));
    pb.rect_fill(sx, deck_y + 7, PX, 3, STEEL);
    pb.rect_fill(sx, deck_y + 7, PX, 1, mix(STEEL, STEEL_HI, 0.45));
    pb.rect_fill(sx, deck_y + 10, PX, 2, RIP);

    // Welded steel stake pockets every 4 cells with rust drip & bolts.
    if cell_x.rem_euclid(4) == 0 {
        pb.rect_fill(sx + 2, sy, 6, PX, STEEL_LO);
        pb.rect_fill(sx + 3, sy + 1, 4, PX - 2, STEEL);
        pb.rect_fill(sx + 3, sy + 1, 1, PX - 2, STEEL_HI);
        pb.rect_fill(sx + 4, sy + 5, 3, 4, RUST);
        pb.plot(sx + 3, sy + 2, BOLT);
        pb.plot(sx + 6, sy + 2, BOLT);
    } else if cell_x.rem_euclid(4) == 2 {
        // DOT amber/red reflective marker riveted into the rub-rail channel.
        let marker = if cell_x.rem_euclid(8) == 2 {
            STRAP_HOT
        } else {
            REFLECTOR_RED
        };
        pb.rect_fill(sx + 5, deck_y + 4, 6, 2, marker);
        pb.plot(sx + 6, deck_y + 4, STUD);
    }

    // Highway sunlight & passing-dash glare flickering off the rail edge.
    let glint = ((elapsed * 10.0 + cell_x as f32 * 0.85).sin() + 1.0) * 0.5;
    if glint > 0.72 {
        let gcol = mix(STEEL, LANE_WHITE, (glint - 0.72) * 1.4);
        let gy = if south { deck_y + 9 } else { deck_y + 2 };
        pb.rect_fill(sx + 1, gy, PX - 2, 1, gcol);
    }
}

/// The verge beyond the road: scrolling gravel shoulder, metallic W-beam
/// highway guardrail with parallax posts + elongated shadows, and blurred
/// weeds/brush streaking past at [`VERGE_SPEED`].
fn verge(pb: &mut PixelBuffer, sx: i32, sy: i32, cell_x: i32, cell_y: i32, elapsed: f32) {
    let scroll_i = (elapsed * VERGE_SPEED).floor() as i32;
    let is_north = cell_y < RAIL_TOP;
    let dist_from_road = if is_north {
        (RAIL_TOP - ROAD_STRIP) - cell_y
    } else {
        cell_y - (RAIL_BOTTOM + ROAD_STRIP)
    };

    // 1) Base verge ground: gravel shoulder right next to the road, grassy embankment beyond.
    if dist_from_road == 1 {
        pb.rect_fill(sx, sy, PX, PX, mix(VERGE, GRAVEL, 0.65));
        // Road-edge gravel strip.
        let edge_y = if is_north { sy + PX - 4 } else { sy };
        pb.rect_fill(sx, edge_y, PX, 4, GRAVEL);
    } else {
        pb.rect_fill(sx, sy, PX, PX, if (cell_y & 1) == 0 { VERGE } else { VERGE_DARK });
    }

    // 2) Parallax-scrolling blurred scrub grass, dust & pebbles streaking past!
    let wx0 = cell_x * PX;
    for row in 0..4 {
        let yy = sy + 2 + row * 4;
        let p_cell = (wx0 + scroll_i).div_euclid(16);
        let h = hash(p_cell, cell_y * 7 + row);
        let off = (wx0 + scroll_i).rem_euclid(16);
        let gx = sx + ((h & 7) as i32 - off).rem_euclid(PX);
        let len = 4 + ((h >> 4) & 5) as i32;
        let col = match (h >> 8) & 3 {
            0 => DUST,
            1 => GRASS_HI,
            2 => GRAVEL,
            _ => VERGE_DARK,
        };
        let clipped_len = len.min(sx + PX - gx);
        if clipped_len > 0 {
            pb.rect_fill(gx, yy, clipped_len, 1, col);
        }
    }

    // 3) Metallic W-beam highway guardrail + scrolling posts along the verge row nearest the road!
    if dist_from_road == 1 {
        let rail_y = if is_north { sy + 7 } else { sy + 5 };
        // Cast shadow under the guardrail.
        pb.rect_fill(sx, rail_y + 3, PX, 2, RIP);
        // W-beam steel ribbon.
        pb.rect_fill(sx, rail_y, PX, 3, STEEL);
        pb.rect_fill(sx, rail_y, PX, 1, STEEL_HI);
        pb.rect_fill(sx, rail_y + 2, PX, 1, STEEL_LO);

        // Guardrail posts whipping past every 48 px at VERGE_SPEED.
        let post_period = 48;
        let phase = (wx0 + scroll_i).rem_euclid(post_period);
        if phase < PX {
            let px = sx + (PX - 1 - phase);
            pb.rect_fill(px, rail_y - 2, 3, 7, POST);
            pb.rect_fill(px, rail_y - 2, 1, 7, STEEL_HI);
            pb.rect_fill(px + 2, rail_y - 1, 2, 6, RIP);
            // Tiny amber highway reflector tab on every other post.
            if ((wx0 + scroll_i).div_euclid(post_period) & 1) == 0 {
                pb.plot(px + 1, rail_y, STUD_AMBER);
            }
        }
    }
}

// ---- Loose cargo props -----------------------------------------------------

/// Draw one loose cargo prop into its screen rectangle. Dispatched from
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
    // Grounded down-right contact shadow.
    pb.rect_fill(x + 2, y + h - 2, w, 3, mix(ASPHALT_DARK, RIP, 0.65));
    match kind {
        PropKind::Pallet => {
            pb.rect_fill(x, y, w, h, RIP);
            pb.rect_fill(x + 1, y + 1, w - 2, h - 2, mix(WOOD_DARK, RIP, 0.5));
            let mut k = 1;
            while k < h - 1 {
                let slat = WOOD[((n as usize) + (k as usize / 4)) % 5];
                pb.rect_fill(x + 1, y + k, w - 2, 3, slat);
                pb.rect_fill(x + 1, y + k, w - 2, 1, mix(slat, WOOD_LIGHT, 0.42));
                pb.plot(x + 3, y + k + 1, BOLT);
                if w > 8 {
                    pb.plot(x + w - 4, y + k + 1, BOLT);
                }
                k += 4;
            }
            // Broken weathered slat notch in the middle.
            if w >= 12 && h >= 10 {
                pb.rect_fill(x + w / 2 - 2, y + 5, 5, 3, RIP);
                pb.plot(x + w / 2 - 3, y + 6, SPLINTER);
                pb.plot(x + w / 2 + 3, y + 6, SPLINTER);
            }
        }
        PropKind::TarpLump => {
            let tarp = mix(TARP, TARP_HI, (n & 7) as f32 / 14.0);
            pb.rect_fill(x, y, w, h, TARP_LO);
            pb.rect_fill(x + 1, y + 2, w - 2, h - 3, tarp);
            // Draped lumpy ridge line + shadow crease.
            for k in 0..(w - 2) {
                let hump = (k as f32 / w.max(1) as f32 * 3.4).sin();
                let yy = y + 3 + (hump * 1.8) as i32;
                pb.plot(x + 1 + k, yy, TARP_HI);
                pb.plot(x + 1 + k, yy + 2, TARP_LO);
            }
            // High-vis orange ratchet strap cinched across the tarp + steel buckle.
            pb.rect_fill(x + w / 2 - 1, y + 1, 2, h - 2, STRAP_ORANGE);
            pb.rect_fill(x + w / 2 - 1, y + h / 2, 2, 2, STEEL_HI);
            pb.rect_fill(x + 1, y + h - 2, w - 2, 1, TARP_LO);
        }
        PropKind::Tyre => {
            let cx = x + w / 2;
            let cy = y + h / 2;
            let r = (w.min(h) / 2 - 1).max(2);
            // Heavy truck tyre rubber + metallic steel wheel hub inside.
            pb.filled_circle(cx, cy, r, (28, 30, 34));
            pb.ring(cx, cy, r, (64, 68, 76));
            pb.ring(cx, cy, (r - 1).max(1), (42, 44, 50));
            if r >= 4 {
                pb.filled_circle(cx, cy, r - 3, STEEL_LO);
                pb.ring(cx, cy, r - 3, STEEL_HI);
                pb.filled_circle(cx, cy, (r - 4).max(1), RIP);
            }
            // Bold tread blocks around the circumference.
            for k in 0..8 {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                pb.plot(
                    cx + (a.cos() * r as f32).round() as i32,
                    cy + (a.sin() * r as f32).round() as i32,
                    (86, 90, 98),
                );
            }
        }
        PropKind::Toolbox => {
            // Red mechanic's steel toolbox with top handle, silver latches & corner scuffs!
            pb.rect_fill(x, y, w, h, RIP);
            pb.rect_fill(x + 1, y + 1, w - 2, h - 2, TOOLBOX_RED);
            pb.rect_fill(x + 1, y + 1, w - 2, 2, TOOLBOX_HI);
            pb.rect_fill(x + 1, y + h / 2, w - 2, 1, RIP);
            // Chrome top handle.
            let hx = x + w / 2 - 2;
            pb.rect_fill(hx, y, 5, 1, STEEL_HI);
            pb.plot(hx, y + 1, STEEL);
            pb.plot(hx + 4, y + 1, STEEL);
            // Twin silver clasp latches.
            pb.rect_fill(x + 3, y + h / 2 - 1, 2, 2, STEEL_HI);
            if w >= 10 {
                pb.rect_fill(x + w - 5, y + h / 2 - 1, 2, 2, STEEL_HI);
            }
        }
        PropKind::PipeStack => {
            // Bundled shaded steel pipes with hollow dark bores + orange ratchet strap!
            pb.rect_fill(x, y, w, h, RIP);
            pb.rect_fill(x + 1, y + 1, w - 2, h - 2, STEEL_LO);
            let r = (h / 2 - 2).max(2);
            let mut cx = x + r + 1;
            while cx + r < x + w {
                pb.filled_circle(cx, y + h / 2, r, STEEL);
                pb.ring(cx, y + h / 2, r, STEEL_LO);
                pb.plot(cx - 1, y + h / 2 - r + 1, STEEL_HI);
                pb.filled_circle(cx, y + h / 2, (r - 2).max(1), RIP);
                cx += 2 * r;
            }
            pb.rect_fill(x + w / 2 - 1, y, 2, h, STRAP_ORANGE);
            pb.rect_fill(x + w / 2 - 2, y + h / 2 - 1, 4, 3, STEEL_HI);
        }
        PropKind::ChainCoil => {
            let cx = x + w / 2;
            let cy = y + h / 2;
            for k in 0..4 {
                let r = 2 + k * 2;
                let col = if k & 1 == 0 { STEEL_HI } else { mix(STEEL, RUST, 0.5) };
                pb.ring(cx, cy, r, col);
                pb.ring(cx + 1, cy + 1, r, STEEL_LO);
            }
            pb.plot(cx, cy, STEEL_HI);
            // Heavy hook & trailing links off the coil.
            pb.plot(cx + 4, cy + 4, STEEL_LO);
            pb.plot(cx + 5, cy + 5, STEEL_HI);
            pb.plot(cx + 6, cy + 6, RUST);
            pb.plot(cx + 7, cy + 5, STEEL_HI);
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

    /// Flatbed visual matrix: 1x lighting full, 1x lighting off, 2x zoom-out,
    /// and a sliding crate mid-tell + loose cargo props.
    /// `RATRUN_DUMP=flatbed_matrix cargo test dump_flatbed_matrix`.
    #[test]
    fn dump_flatbed_matrix() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("flatbed_matrix") {
            return;
        }
        use crate::game::world::World;
        use crate::render::lighting::LightingMode;
        let tw = 160usize;
        let th = 96usize;
        let mut grid = vec![(0u8, 0u8, 0u8); tw * 2 * th * 2];
        let mut paste = |slot: usize, pb: &PixelBuffer| {
            let ox = (slot % 2) * tw;
            let oy = (slot / 2) * th;
            for y in 0..th {
                for x in 0..tw {
                    grid[(oy + y) * (tw * 2) + (ox + x)] = pb.pixel_at(x, y);
                }
            }
        };

        // 0) 1x zoom, lighting Full, overlooking deck + pit.
        let mut w = World::new(Theme::Trailer, 5);
        w.elapsed = 7.4;
        if let Some(pit) = w.level.trailer.pits.first() {
            w.player.pos = pit.center_world() + Vec2::new(-32.0, -8.0);
        }
        let mut pb = PixelBuffer::new(tw, th);
        w.lighting = LightingMode::Full;
        w.draw(&mut pb);
        paste(0, &pb);

        // 1) Same view with lighting Off.
        w.lighting = LightingMode::Off;
        w.draw(&mut pb);
        paste(1, &pb);

        // 2) 2x zoom-out (320x192 world -> resampled into 160x96).
        w.lighting = LightingMode::Full;
        w.zoom = 2.0;
        w.zoom_view = 2.0;
        w.draw(&mut pb);
        paste(2, &pb);

        // 3) Sliding crate caught mid-tell (glowing straps!) + loose cargo props nearby.
        w.zoom = 1.0;
        w.zoom_view = 1.0;
        if let Some(sl) = w.level.trailer.slides.first() {
            w.player.pos = sl.rest + Vec2::new(-28.0, 12.0);
            // Put elapsed squarely inside the crate's warn window!
            let period = crate::game::level::SLIDE_PERIOD;
            w.elapsed = (period - sl.phase).rem_euclid(period) + sl.warn * 0.55;
        }
        w.draw(&mut pb);
        paste(3, &pb);

        let mut out = format!("P6\n{} {}\n255\n", tw * 2, th * 2).into_bytes();
        for (r, g, b) in grid {
            out.extend_from_slice(&[r, g, b]);
        }
        let dir = std::env::var_os("RATRUN_DUMP_DIR")
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        std::fs::write(dir.join("ratrun_flatbed_matrix.ppm"), out).unwrap();
    }

    #[test]
    fn benchmark_on_deck_floor_draw() {
        let level = Level::generate(Theme::Trailer, 5);
        let cam = level.start - Vec2::new(60.0, 40.0);
        let x = cam.x.round() as i32..cam.x.round() as i32 + 120;
        let y = cam.y.round() as i32..cam.y.round() as i32 + 80;
        let mut pb = PixelBuffer::new(120, 80);
        for _ in 0..8 {
            draw_floor(&mut pb, &level, cam, 7.0, &x, &y);
        }
        let mut best = u128::MAX;
        for _ in 0..12 {
            let t0 = std::time::Instant::now();
            for _ in 0..20 {
                draw_floor(&mut pb, &level, cam, 7.0, &x, &y);
            }
            best = best.min(t0.elapsed().as_micros() / 20);
        }
        println!("flatbed on-deck draw_floor (120x80 px): {best} µs");
        assert!(best < 2_000, "on-deck draw_floor too slow: {best} µs");
    }
}
