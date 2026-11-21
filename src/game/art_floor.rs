//! Art for the arena floor and its border. Pure drawing.

use std::ops::Range;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::level::{Level, Palette, Theme};

#[allow(unused_imports)]
pub use super::art_marks::{
    draw_boss_arena_floor, draw_gnaw_mark, draw_gold_fleck, draw_hall_landmark,
    BOSS_ARENA_MOTIF_CELLS, GNAW_MARK_OFFSETS, GNAW_MARK_SPACING, GOLD_FLECK_SPACING,
    HALL_LANDMARK_HALF_EXTENT, HALL_LANDMARK_VARIANTS,
};

/// Paint the floor for the visible region and the arena border. The buffer has
/// already been cleared to `level.palette.bg`. `cam` is the world position of
/// the buffer's top-left pixel; `elapsed` is the run clock in seconds.
pub fn draw_floor(pb: &mut PixelBuffer, level: &Level, cam: Vec2, elapsed: f32) {
    // Snap once so bricks, highlights and seams scroll as one rigid surface.
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let (width, height) = (level.arena.x.round() as i32, level.arena.y.round() as i32);
    let x = cam_x.max(0)..(cam_x + pb.w as i32).min(width);
    let y = cam_y.max(0)..(cam_y + pb.h as i32).min(height);
    if x.is_empty() || y.is_empty() {
        pb.clear(scale(level.palette.bg, 0.44));
        return;
    }
    if x.end - x.start != pb.w as i32 || y.end - y.start != pb.h as i32 {
        pb.clear(scale(level.palette.bg, 0.44));
    }
    // The flatbed's deck is its own layer: planks, the road through their
    // gaps, the rails and the verge. It owns its clipping so the bike-lane
    // `kerb` below never paints over the road.
    if level.theme == Theme::Trailer {
        crate::game::art_trailer::draw_floor(pb, level, cam, elapsed, &x, &y);
        return;
    }
    let mut floor = Floor {
        pb,
        cam_x,
        cam_y,
        x,
        y,
    };
    match level.theme {
        Theme::Sewer => sewer(&mut floor, &level.palette, elapsed),
        Theme::Kitchen => kitchen(&mut floor, &level.palette),
        Theme::Lab => lab(&mut floor, &level.palette),
        Theme::Laundry => laundry(&mut floor, &level.palette, elapsed),
        Theme::Printworks => printworks(&mut floor, &level.palette, elapsed),
        Theme::Trailer => unreachable!("the flatbed draws its own floor"),
    }
    // The wave 3 floor hazards sit on the floor layer: they are walkable, so
    // they must read as paint and grate plates, never as obstacles.
    hazards(&mut floor, level, elapsed);
    kerb(&mut floor, level, width, height);
}

struct Floor<'a> {
    pb: &'a mut PixelBuffer,
    cam_x: i32,
    cam_y: i32,
    x: Range<i32>,
    y: Range<i32>,
}

impl Floor<'_> {
    fn fill_visible(&mut self, color: Rgb) {
        self.fill(
            self.x.start,
            self.y.start,
            self.x.end - self.x.start,
            self.y.end - self.y.start,
            color,
        );
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
        let left = x.max(self.x.start);
        let top = y.max(self.y.start);
        self.pb.rect_fill(
            left - self.cam_x,
            top - self.cam_y,
            (x + w).min(self.x.end) - left,
            (y + h).min(self.y.end) - top,
            color,
        );
    }

    fn columns(&self, step: i32, offset: i32) -> Range<i32> {
        grid(&self.x, step, offset)
    }

    fn rows(&self, step: i32, offset: i32) -> Range<i32> {
        grid(&self.y, step, offset)
    }
}

// Include partial tiles at either side, including a staggered tile at x < 0.
fn grid(span: &Range<i32>, step: i32, offset: i32) -> Range<i32> {
    (span.start - offset).div_euclid(step)..(span.end - 1 - offset).div_euclid(step) + 1
}

fn hash(x: i32, y: i32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n
}

fn sewer(floor: &mut Floor<'_>, pal: &Palette, elapsed: f32) {
    const BRICK_W: i32 = 27;
    const BRICK_H: i32 = 13;
    let mortar = scale(pal.bg, 0.76);
    let shoulder = mix(pal.bg_alt, pal.wall, 0.12);
    let stone = [
        pal.bg,
        mix(pal.bg, pal.bg_alt, 0.35),
        mix(pal.bg, pal.bg_alt, 0.70),
        mix(pal.bg, (41, 38, 25), 0.23),
    ];
    floor.fill_visible(mortar);
    for row in floor.rows(BRICK_H, 0) {
        let offset = (row & 1) * (BRICK_W / 2);
        let y = row * BRICK_H;
        for col in floor.columns(BRICK_W, offset) {
            let x = col * BRICK_W + offset;
            let n = hash(col, row);
            let body = stone[(n & 3) as usize];
            floor.fill(x + 1, y + 1, BRICK_W - 2, BRICK_H - 1, body);
            let wear = (n >> 3 & 7) as i32;
            floor.fill(
                x + 2 + wear,
                y + 1,
                BRICK_W - 4 - wear,
                1,
                mix(body, shoulder, 0.48),
            );
            floor.fill(
                x + BRICK_W - 3,
                y + 2,
                2,
                BRICK_H - 3,
                mix(body, mortar, 0.38),
            );
            if n & 7 < 3 {
                // Cracked corners and old mineral deposits interrupt the bond.
                let scar_x = x + 4 + (n >> 6 & 7) as i32;
                let scar_y = y + 4 + (n >> 9 & 3) as i32;
                let unlit = mix(body, mortar, 0.84);
                let mineral = mix(body, shoulder, 0.37);
                floor.fill(scar_x, scar_y, 3 + (n >> 11 & 3) as i32, 1, unlit);
                floor.fill(scar_x + 4, scar_y + 1, 2, 1, unlit);
                floor.fill(scar_x + 5, scar_y + 2, 4, 1, mineral);
            } else if n & 7 == 4 {
                let moss = mix(body, (35, 45, 26), 0.22);
                floor.fill(x + 1, y + BRICK_H - 3, 8 + wear, 3, moss);
                floor.fill(x + 2 + wear, y + BRICK_H - 4, 4, 1, moss);
            }
        }
    }

    // Low spots collect water across the mortar. Only the reflections change.
    let water = mix(mortar, pal.accent, 0.060);
    let wet_edge = mix(pal.bg, pal.wall, 0.24);
    for row in floor.rows(45, 0) {
        for col in floor.columns(59, 0) {
            let n = hash(col + 231, row);
            if n & 3 == 0 {
                continue;
            }
            let x = col * 59 + 7 + (n >> 3 & 15) as i32;
            let y = row * 45 + 5 + (n >> 7 & 15) as i32;
            let stretch = (n >> 11 & 7) as i32;
            let bend = (n >> 14 & 3) as i32;
            floor.fill(x + 4 + bend, y, 12 + stretch - bend, 1, wet_edge);
            floor.fill(x + 1, y + 1, 21 + stretch, 2, water);
            floor.fill(x, y + 3, 25 + stretch, 2, water);
            floor.fill(x + 3 + bend, y + 5, 23 + stretch - bend, 2, water);
            floor.fill(x + 9 + bend, y + 7, 11 + stretch - bend, 1, water);
            floor.fill(x + 13, y + 8, 6 + stretch, 1, scale(water, 0.83));
            floor.fill(x + 22 + stretch, y + 7, 5, 1, mix(water, wet_edge, 0.33));
            floor.fill(x + 9, y + 1, 11 + stretch, 3, mix(water, wet_edge, 0.19));
            let shimmer = ((elapsed * 0.75 + (n >> 16 & 63) as f32).sin() + 1.0) * 0.5;
            let reflected = mix(water, wet_edge, 0.46 + 0.54 * shimmer);
            floor.fill(x + 3, y + 2, 6 + stretch, 1, reflected);
            floor.fill(x + 15, y + 5, 5 + stretch, 1, reflected);
        }
    }

    // The trough sits flush with the pavement, with iron bars at intervals.
    let iron = mix(pal.bg_alt, pal.wall, 0.17);
    for row in floor.rows(143, 56) {
        let y = row * 143 + 56;
        let x = floor.x.start;
        let w = floor.x.end - x;
        floor.fill(x, y, w, 1, shoulder);
        floor.fill(x, y + 1, w, 5, mortar);
        floor.fill(x, y + 2, w, 3, water);
        floor.fill(x, y + 6, w, 1, wet_edge);
        for col in floor.columns(34, 0) {
            let x = col * 34;
            floor.fill(x + 2, y + 1, 2, 5, iron);
            floor.fill(x + 7, y + 1, 2, 5, iron);
            floor.fill(x + 12, y + 1, 2, 5, iron);
            floor.fill(x + 17, y + 3, 15, 1, wet_edge);
        }
    }
}

fn kitchen(floor: &mut Floor<'_>, pal: &Palette) {
    const TILE: i32 = 20;
    let grout = scale(pal.bg, 0.63);
    let glaze = [
        mix(pal.bg, (57, 32, 37), 0.25),
        mix(pal.bg_alt, (61, 55, 32), 0.23),
    ];
    for row in floor.rows(TILE, 0) {
        for col in floor.columns(TILE, 0) {
            let (x, y) = (col * TILE, row * TILE);
            let n = hash(col + 16, row + 96);
            let body = mix(
                glaze[((col + row) & 1) as usize],
                grout,
                (n & 3) as f32 * 0.07,
            );
            let bevel = mix(body, pal.wall, 0.13);
            floor.fill(x, y, TILE, TILE, grout);
            floor.fill(x + 1, y + 1, TILE - 1, TILE - 1, body);
            floor.fill(x + 2, y + 1, TILE - 3, 1, bevel);
            floor.fill(x + 1, y + 2, 1, TILE - 3, bevel);
            floor.fill(x + 3, y + TILE - 1, TILE - 3, 1, mix(body, grout, 0.35));

            // Scuffed glaze has two broken streaks in the same direction.
            if n & 16 != 0 {
                let sx = x + 3 + (n >> 7 & 7) as i32;
                let sy = y + 4 + (n >> 10 & 7) as i32;
                floor.fill(sx, sy, 6, 1, mix(body, bevel, 0.61));
                floor.fill(sx + 3, sy + 2, 4, 1, mix(body, bevel, 0.35));
            }

            if n & 7 == 0 {
                let stain = mix(body, (19, 9, 7), 0.43);
                let smear = mix(body, stain, 0.65);
                let sx = x + 2 + (n >> 13 & 3) as i32;
                let sy = y + 7 + (n >> 15 & 3) as i32;
                let spill = (n >> 17 & 3) as i32;
                floor.fill(sx + 3, sy, 4 + spill, 1, smear);
                floor.fill(sx + 1, sy + 1, 8 + spill, 2, stain);
                floor.fill(sx, sy + 3, 6, 1, stain);
                floor.fill(sx + 2, sy + 4, 3 + spill, 1, smear);
                floor.fill(sx + 10, sy + 4, 2, 1, bevel);
                floor.fill(sx + 12, sy + 5, 1, 1, bevel);
            } else if n & 7 == 1 {
                floor.fill(x + 14, y + 1, 5, 1, grout);
                floor.fill(x + 17, y + 2, 2, 1, mix(body, grout, 0.70));
                floor.fill(x + 9 + (n >> 13 & 3) as i32, y + 14, 2, 1, bevel);
            } else if n & 15 == 3 {
                let crumb = mix(body, (82, 65, 38), 0.23);
                floor.fill(x + 5, y + 14, 2, 1, crumb);
                floor.fill(x + 8, y + 12, 1, 1, mix(crumb, body, 0.35));
                floor.fill(x + 7, y + 16, 1, 1, bevel);
            }
        }
    }
}

fn lab(floor: &mut Floor<'_>, pal: &Palette) {
    const W: i32 = 46;
    const H: i32 = 28;
    let seam = scale(pal.bg, 0.65);
    let flange = mix(pal.bg_alt, pal.wall, 0.16);
    let stencil = mix(pal.bg_alt, (112, 86, 40), 0.26);
    let socket = mix(pal.bg_alt, pal.accent, 0.10);
    for row in floor.rows(H, 0) {
        for col in floor.columns(W, 0) {
            let (x, y) = (col * W, row * H);
            let n = hash(col + 87, row + 13);
            let deck = mix(pal.bg, pal.bg_alt, 0.25 + (n & 3) as f32 * 0.14);
            floor.fill(x, y, W, H, seam);
            // Broad slabs with chamfered corners and a recessed service seam.
            floor.fill(x + 3, y + 1, W - 6, H - 2, deck);
            floor.fill(x + 1, y + 3, W - 2, H - 6, deck);
            floor.fill(x + 3, y + 1, W - 6, 1, flange);
            floor.fill(x + 1, y + 3, 1, H - 6, mix(deck, flange, 0.55));
            floor.fill(x + 3, y + H - 2, W - 6, 1, mix(deck, seam, 0.48));
            floor.fill(x + W - 9, y + 4, 1, H - 8, mix(deck, flange, 0.35));
            floor.fill(x + W - 6, y + 5, 2, 1, seam);
            floor.fill(x + W - 6, y + H - 6, 2, 1, seam);

            let route = (row + 1) % 5 == 0;
            if n & 7 == 0 && !route {
                // Flush access hatch, stencilled corners and a short vent grille.
                floor.fill(x + 10, y + 8, 21, 12, mix(deck, seam, 0.68));
                floor.fill(x + 11, y + 9, 19, 10, deck);
                floor.fill(x + 10, y + 8, 4, 1, stencil);
                floor.fill(x + 27, y + 19, 4, 1, stencil);
                for vane in 0..4 {
                    floor.fill(x + 15 + vane * 3, y + 11, 1, 5, seam);
                }
                floor.fill(x + 12, y + 17, 3, 1, socket);
            } else if n & 3 == 1 {
                floor.fill(x + W - 6, y + 10, 2, 7, seam);
                floor.fill(x + W - 6, y + 11 + (n >> 8 & 3) as i32, 2, 2, socket);
            }
            if route {
                // Stencilled chevrons and parallel stripes stop at the panel joints.
                let lane = mix(deck, pal.accent, 0.072);
                let cy = y + 15;
                let direction = if row % 10 == 4 { 1 } else { -1 };
                for dy in -3i32..=3 {
                    floor.fill(x + 12 + direction * (3 - dy.abs()), cy + dy, 2, 1, lane);
                    floor.fill(x + 18 + direction * (3 - dy.abs()), cy + dy, 2, 1, lane);
                }
                let stripes_x = x + if direction == 1 { 25 } else { 24 };
                let length = 8 + (n >> 16 & 3) as i32;
                floor.fill(stripes_x, cy - 1, length, 1, lane);
                floor.fill(stripes_x, cy + 1, length + 2, 1, lane);
            }
        }
    }
}

/// The Laundry floor: indigo square tile laid over faded denim grout, with a
/// porcelain scuff in most tiles and a brass drain plate where four tiles meet.
/// The drains glint slowly so the floor reads as wet without any per-frame
/// randomness.
fn laundry(floor: &mut Floor<'_>, pal: &Palette, elapsed: f32) {
    const TILE: i32 = 16;
    let grout = mix(pal.bg, pal.wall, 0.30);
    let glaze = [
        mix(pal.bg, pal.bg_alt, 0.55),
        mix(pal.bg, pal.bg_alt, 0.22),
    ];
    let porcelain = mix(pal.bg_alt, pal.wall_edge, 0.38);
    let brass = mix(pal.bg, (122, 96, 46), 0.60);
    for row in floor.rows(TILE, 0) {
        for col in floor.columns(TILE, 0) {
            let (x, y) = (col * TILE, row * TILE);
            let n = hash(col + 73, row + 41);
            let body = mix(glaze[((col + row) & 1) as usize], grout, (n & 3) as f32 * 0.06);
            floor.fill(x, y, TILE, TILE, grout);
            floor.fill(x + 1, y + 1, TILE - 2, TILE - 2, body);
            // Bevel: porcelain catches the light on the top-left, denim in the
            // shadowed bottom-right.
            floor.fill(x + 1, y + 1, TILE - 2, 1, mix(body, pal.wall_edge, 0.22));
            floor.fill(x + 1, y + TILE - 2, TILE - 2, 1, mix(body, grout, 0.45));
            if n & 7 == 0 {
                // A scuff of porcelain where a trolley wheel has ground the glaze.
                let sx = x + 3 + (n >> 6 & 7) as i32;
                let sy = y + 3 + (n >> 10 & 7) as i32;
                floor.fill(sx, sy, 4 + (n >> 13 & 3) as i32, 1, mix(body, porcelain, 0.42));
                floor.fill(sx + 2, sy + 2, 3, 1, mix(body, porcelain, 0.24));
            } else if n & 15 == 5 {
                // Lint in the seam: a pale dashed line.
                for k in 0..4 {
                    floor.fill(x + 2 + k * 3, y + TILE - 3, 2, 1, mix(body, porcelain, 0.30));
                }
            }
            // Brass drain plate at every fourth tile junction.
            if col.rem_euclid(4) == 0 && row.rem_euclid(4) == 0 {
                // Brass drain plate. Every colour here stays under the floor
                // layer's contrast cap (110): the entities above it have to pop.
                let shimmer = ((elapsed * 0.6 + (n >> 16 & 31) as f32).sin() + 1.0) * 0.5;
                let glow = mix(brass, (104, 92, 62), 0.30 + 0.25 * shimmer);
                floor.fill(x + 4, y + 4, 8, 8, mix(brass, grout, 0.35));
                for bx in 0..3 {
                    floor.fill(x + 5 + bx * 2, y + 6, 1, 5, glow);
                }
                floor.fill(x + 5, y + 5, 6, 1, mix(glow, pal.wall_edge, 0.16));
            }
        }
    }
}

/// The Printworks floor: charcoal slab with the paler seams of a press hall,
/// sheets of newsprint that slid off the belt (pale, with a printed rule), and
/// registration marks in cyan at every press bed.
fn printworks(floor: &mut Floor<'_>, pal: &Palette, elapsed: f32) {
    const W: i32 = 38;
    const H: i32 = 26;
    let seam = mix(pal.bg, (0, 0, 0), 0.45);
    let slab = mix(pal.bg, pal.bg_alt, 0.55);
    let newsprint = mix(pal.bg, pal.wall_edge, 0.68);
    let ink = pal.blood;
    let cyan = pal.accent;
    for row in floor.rows(H, 0) {
        for col in floor.columns(W, 0) {
            let (x, y) = (col * W, row * H);
            let n = hash(col + 19, row + 61);
            let body = mix(slab, seam, (n & 3) as f32 * 0.05);
            floor.fill(x, y, W, H, seam);
            floor.fill(x + 1, y + 1, W - 2, H - 2, body);
            floor.fill(x + 1, y + 1, W - 2, 1, mix(body, pal.wall, 0.16));
            floor.fill(x + 1, y + H - 2, W - 2, 1, mix(body, seam, 0.55));
            // Ink smears: two dark drags in one direction, always low contrast.
            if n & 3 == 1 {
                let sx = x + 4 + (n >> 5 & 7) as i32;
                let sy = y + 5 + (n >> 9 & 7) as i32;
                floor.fill(sx, sy, 9, 2, mix(body, ink, 0.24));
                floor.fill(sx + 3, sy + 3, 5, 1, mix(body, ink, 0.16));
            }
            // A sheet of newsprint slid off the belt: pale, with a printed rule
            // and a registration tick in the corner.
            if n & 7 == 2 {
                let sx = x + 6 + (n >> 12 & 3) as i32;
                let sy = y + 4 + (n >> 15 & 3) as i32;
                let sw = 14 + (n >> 17 & 3) as i32;
                let sh = 9 + (n >> 19 & 3) as i32;
                floor.fill(sx, sy, sw, sh, mix(body, newsprint, 0.80));
                floor.fill(sx, sy, sw, 1, mix(newsprint, body, 0.30));
                // Two grey text rules and the masthead band.
                floor.fill(sx + 2, sy + 2, sw - 5, 1, mix(body, ink, 0.35));
                floor.fill(sx + 2, sy + 4, sw - 8, 1, mix(body, ink, 0.24));
                floor.fill(sx + 2, sy + 6, sw - 6, 1, mix(body, ink, 0.20));
                floor.fill(sx + sw - 4, sy + sh - 3, 2, 1, mix(cyan, body, 0.35));
            }
            // Registration cross at the press-bed corners of the hall.
            if col.rem_euclid(5) == 2 && row.rem_euclid(4) == 2 {
                let pulse = 0.45 + 0.15 * ((elapsed * 0.8 + n as f32).sin());
                floor.fill(x + W / 2 - 3, y + H / 2, 6, 1, mix(body, cyan, pulse));
                floor.fill(x + W / 2, y + H / 2 - 3, 1, 6, mix(body, cyan, pulse));
                floor.fill(x + W / 2, y + H / 2, 1, 1, mix(cyan, pal.wall_edge, 0.45));
            }
        }
    }
}

/// Steam grates and ink pads: the two floors' own hazards.
///
/// The grate plate is always visible (so the lane reads before it vents), the
/// three pale puffs are the 1 s tell and the plume is a pale column over the
/// plate. Ink pads are a dark stipple with a wet cyan rim — permanent, because
/// the slow is permanent.
fn hazards(floor: &mut Floor<'_>, level: &Level, elapsed: f32) {
    let pal = &level.palette;
    let on_screen = |f: &Floor<'_>, x: i32, y: i32, w: i32, h: i32| {
        x + w > f.x.start && x < f.x.end && y + h > f.y.start && y < f.y.end
    };

    for pad in &level.objects.ink_pads {
        let (x0, y0) = ((pad.pos.x - pad.half.x) as i32, (pad.pos.y - pad.half.y) as i32);
        let (w, h) = ((pad.half.x * 2.0) as i32, (pad.half.y * 2.0) as i32);
        if !on_screen(floor, x0, y0, w, h) {
            continue;
        }
        let stain = mix(pal.bg, pal.blood, 0.46);
        let deep = mix(stain, (0, 0, 0), 0.42);
        floor.fill(x0 + 1, y0 + 1, w - 2, h - 2, stain);
        // Stipple: short seeded dashes, so two pads never look alike.
        let mut seed = pad.seed;
        for ry in 1..h - 1 {
            for rx in 1..w - 1 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let bit = (seed >> 24) as u8;
                if bit.is_multiple_of(5) {
                    floor.fill(x0 + rx, y0 + ry, 1, 1, deep);
                } else if bit.is_multiple_of(11) {
                    floor.fill(x0 + rx, y0 + ry, 2, 1, mix(stain, pal.accent, 0.22));
                }
            }
        }
        // The wet rim: a cyan-lit top edge and a couple of drips.
        floor.fill(x0 + 1, y0, w - 2, 1, mix(stain, pal.accent, 0.42));
        floor.fill(x0, y0 + 2, 1, h - 4, mix(stain, pal.accent, 0.24));
        let drip = 1 + (pad.seed as i32 & 3);
        for k in 0..drip {
            floor.fill(x0 + 2 + k * 3, y0 + h - 1, 1, 1 + k % 2, deep);
        }
    }

    for grate in &level.objects.steam_grates {
        let (cx, cy) = (grate.pos.x as i32, grate.pos.y as i32);
        if !on_screen(floor, cx - 8, cy - 8, 16, 16) {
            continue;
        }
        // Plate: a recessed iron square with three slots.
        let plate = mix(pal.bg, pal.wall, 0.30);
        let slot = mix(plate, (0, 0, 0), 0.55);
        floor.fill(cx - 6, cy - 6, 12, 12, plate);
        floor.fill(cx - 6, cy - 6, 12, 1, mix(plate, pal.wall_edge, 0.28));
        floor.fill(cx - 6, cy + 5, 12, 1, slot);
        for k in 0..3 {
            floor.fill(cx - 4, cy - 3 + k * 3, 8, 1, slot);
        }
        let puff = mix(pal.bg_alt, pal.wall_edge, 0.72);
        if grate.warning() {
            // Three pale puffs: the 1 s tell.
            let t = grate.phase.progress(grate.timer);
            let rise = (t * 18.0) as i32;
            for k in 0..3 {
                let r = 2 + k;
                let px = cx - r;
                let py = cy - 4 - rise + k * 3;
                floor.fill(px, py, r * 2, r, mix(puff, pal.bg, 0.18 + 0.2 * k as f32));
            }
        } else if grate.plume() {
            // The scalding column: pale body, bright core, a rim at the base.
            let t = grate.phase.progress(grate.timer);
            let height = 18 + (t * 8.0) as i32;
            let body = mix(pal.wall_edge, pal.bg_alt, 0.24);
            let core = mix(body, (255, 255, 255), 0.30);
            floor.fill(cx - 6, cy - height, 12, height, body);
            floor.fill(cx - 4, cy - height + 2, 8, height - 4, mix(core, body, 0.35));
            floor.fill(cx - 2, cy - height + 3, 4, height - 8, core);
            floor.fill(cx - 7, cy - 1, 14, 1, mix(body, pal.accent, 0.40));
            // The fast shimmer of escaping steam.
            let s = ((elapsed * 9.0 + cx as f32).sin() * 0.5 + 0.5) * 4.0;
            floor.fill(cx - 5 + s as i32, cy - height + 4, 2, height - 8, mix(core, body, 0.25));
        }
    }
}

fn kerb(floor: &mut Floor<'_>, level: &Level, width: i32, height: i32) {
    let pal = &level.palette;
    let shade = scale(pal.bg, 0.54);
    let face = mix(pal.bg, pal.wall, 0.27);
    let lip = mix(pal.bg, pal.wall_edge, 0.23);
    let bevel = mix(face, lip, 0.43);

    // Mitered corners. The bevel and its contact shadow are inside the arena.
    for (depth, color) in [shade, bevel, face, face, lip, bevel, shade]
        .into_iter()
        .enumerate()
    {
        let d = depth as i32;
        floor.fill(d, d, width - 2 * d, 1, color);
        floor.fill(d, height - d - 1, width - 2 * d, 1, color);
        floor.fill(d, d + 1, 1, height - 2 * d - 2, color);
        floor.fill(width - d - 1, d + 1, 1, height - 2 * d - 2, color);
    }

    let seam = mix(shade, face, 0.4);
    let (spacing, offset, joint, joint_edge) = match level.theme {
        Theme::Sewer => (24, 10, seam, bevel),
        Theme::Kitchen => (20, 0, seam, face),
        Theme::Lab => {
            let warning = mix(face, (103, 80, 37), 0.48);
            (32, 13, warning, mix(warning, bevel, 0.45))
        }
        Theme::Laundry => {
            let porcelain = mix(face, pal.wall_edge, 0.46);
            (16, 0, porcelain, mix(porcelain, bevel, 0.45))
        }
        Theme::Printworks => {
            let cyan = mix(face, pal.accent, 0.52);
            (38, 19, cyan, mix(cyan, bevel, 0.40))
        }
        Theme::Trailer => unreachable!("the flatbed draws its own border"),
    };
    for edge in [2, height - 4] {
        if edge < floor.y.start - 1 || edge >= floor.y.end {
            continue;
        }
        for col in floor.columns(spacing, 0) {
            let x = col * spacing + offset;
            if (7..width - 7).contains(&x) {
                floor.fill(x, edge, 1, 2, joint);
                floor.fill(x + 1, edge, 1, 1, joint_edge);
            }
        }
    }
    for edge in [2, width - 4] {
        if edge < floor.x.start - 1 || edge >= floor.x.end {
            continue;
        }
        for row in floor.rows(spacing, 0) {
            let y = row * spacing + offset;
            if (7..height - 7).contains(&y) {
                floor.fill(edge, y, 2, 1, joint);
                floor.fill(edge, y + 1, 1, 1, joint_edge);
            }
        }
    }
}

// ---- Floor finds: pickups, the Wind-up Mouse decoy, gold & chests --------
//
// Solid two-tone pixel sprites with top-left directional lighting and dark
// contour rims so every floor find reads instantly at half-block resolution,
// with lighting Full, Soft, or Off, and in 256 / 16 colours.
//
// The three legacy finds keep exactly the sprite they had in `World::draw` (a
// 5 px disc with a white core and the pulsing halo drawn by the caller).

/// Subtle floor shadow helper (darkens existing pixel in bounds).
#[inline]
fn shade_floor(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let cur = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale(cur, 1.0 - amount));
}

/// Draw one floor pickup at screen position `s` (already `w2s`-mapped).
///
/// `bob` is the pickup's animation phase (`Pickup::bob`); it drives the halo
/// pulse for the legacy disc, the clock hands on the Pocket Watch, and the
/// turning butterfly key on the Wind-up Mouse.
pub fn draw_pickup(pb: &mut PixelBuffer, kind: crate::game::gem::PickupKind, s: (i32, i32), bob: f32) {
    use crate::game::gem::PickupKind;
    let pulse = 0.5 + 0.5 * (bob.sin() * 0.5 + 0.5);
    let base = kind.color();
    match kind {
        PickupKind::Heal | PickupKind::Magnet | PickupKind::Nuke => {
            pb.filled_circle(s.0, s.1, 2, scale(base, 0.7 + pulse * 0.3));
            pb.plot(s.0, s.1, (255, 255, 255));
        }
        PickupKind::BottleCap => draw_bottle_cap(pb, s, pulse),
        PickupKind::WindupMouse => draw_windup_pickup(pb, s, bob),
        PickupKind::PocketWatch => draw_pocket_watch(pb, s, bob),
        PickupKind::CleanSocks => draw_clean_socks(pb, s, pulse),
    }
}

/// Fluted tin bottle cap with a visibly corrugated toothed rim, metallic
/// blue-steel dome shading and a bold vintage crimson cap stripe.
fn draw_bottle_cap(pb: &mut PixelBuffer, s: (i32, i32), pulse: f32) {
    let (cx, cy) = s;
    // Soft ground contact shadow shifted down-right.
    for dx in -2..=3 {
        shade_floor(pb, cx + dx, cy + 4, 0.44);
    }
    shade_floor(pb, cx + 4, cy + 2, 0.32);
    shade_floor(pb, cx + 4, cy + 3, 0.36);

    let rim_dark: Rgb = (28, 34, 46);
    let crimp_hi: Rgb = (252, 250, 242);
    let crimp_lo: Rgb = (108, 120, 138);
    let dome_hi: Rgb = (242, 246, 252);
    let dome_mid: Rgb = (178, 190, 208);
    let dome_lo: Rgb = (114, 128, 148);
    let crest: Rgb = (224, 58, 72);
    let crest_hi: Rgb = (255, 126, 132);
    let crest_lo: Rgb = (162, 34, 48);

    // 1. Dark contour rim around the inner dome + projecting crimp notches.
    pb.filled_circle(cx, cy, 3, rim_dark);
    for &(tx, ty) in &[
        (0, -4), (2, -3), (4, -1), (4, 1), (2, 3), (0, 4),
        (-2, 3), (-4, 1), (-4, -1), (-2, -3),
    ] {
        pb.plot(cx + tx, cy + ty, rim_dark);
    }

    // 2. Serrated outer crimp teeth (alternating protruding tin flanges & dark gaps).
    for &(tx, ty, lit) in &[
        (0, -3, true), (2, -3, true), (3, -2, true), (3, 0, false),
        (3, 2, false), (2, 3, false), (0, 3, false), (-2, 3, false),
        (-3, 2, false), (-3, 0, true), (-3, -2, true), (-2, -3, true),
    ] {
        pb.plot(cx + tx, cy + ty, if lit { crimp_hi } else { crimp_lo });
    }
    // Crisp white inner rim ring highlight.
    for &(rx, ry) in &[(-2, -2), (-1, -3), (1, -3), (-3, -1), (2, -2)] {
        pb.plot(cx + rx, cy + ry, crimp_hi);
    }

    // 3. Domed brushed-steel tin crown with top-left shading.
    for dy in -2..=2i32 {
        for dx in -2..=2i32 {
            if dx * dx + dy * dy <= 5 {
                let col = if dx + dy <= -2 {
                    dome_hi
                } else if dx + dy >= 2 {
                    dome_lo
                } else {
                    dome_mid
                };
                pb.plot(cx + dx, cy + dy, col);
            }
        }
    }

    // 4. Bold vintage crimson soda-cap stripe across the dome + specular glint.
    pb.plot(cx - 2, cy, crest_hi);
    pb.plot(cx - 1, cy, crest_hi);
    pb.plot(cx, cy, crest);
    pb.plot(cx + 1, cy, crest);
    pb.plot(cx + 2, cy, crest_lo);
    pb.plot(cx - 1, cy - 1, dome_hi);
    pb.plot(cx - 1, cy - 2, crimp_hi);
    if pulse > 0.72 {
        pb.plot_add(cx - 2, cy - 2, (70, 74, 80));
    }
}

/// Spinning brass butterfly wind-up key: two symmetric oval finger-wings on a
/// vertical shaft that foreshorten smoothly as `angle` turns (`cos(angle)`).
fn draw_butterfly_key(pb: &mut PixelBuffer, s: (i32, i32), angle: f32, big: bool) {
    let (cx, cy) = s;
    let out: Rgb = (46, 30, 14);
    let hi: Rgb = (255, 246, 194);
    let brass: Rgb = (242, 194, 78);
    let bronze: Rgb = (164, 106, 28);

    let c = angle.cos();
    let span_f = if big { 4.2 } else { 3.3 };
    let span = (c.abs() * span_f).round() as i32;
    let top_y = cy - if big { 4 } else { 3 };
    let mid_y = cy - if big { 2 } else { 1 };

    // Vertical brass pin & collar.
    for y in (top_y - 1)..=(cy + 1) {
        pb.plot(cx - 1, y, out);
        pb.plot(cx + 1, y, out);
    }
    for y in top_y..=cy {
        pb.plot(cx, y, if y <= mid_y { hi } else { brass });
    }
    pb.plot(cx, cy + 1, bronze);

    if span <= 0 {
        // Edge-on flash: bright specular glinting spine.
        for y in top_y..=mid_y {
            pb.plot(cx, y, hi);
        }
        pb.plot_add(cx, top_y - 1, (140, 120, 60));
        return;
    }

    // Twin butterfly wing loops on left and right of the stem.
    for side in [-1i32, 1] {
        let lit_side = (side < 0) == (c >= 0.0);
        let top_col = if lit_side { hi } else { brass };
        let bot_col = if lit_side { brass } else { bronze };
        // Dark contour around each wing so it reads cleanly against bright halos.
        for dx in 1..=(span + 1) {
            pb.plot(cx + side * dx, top_y - 1, out);
            pb.plot(cx + side * dx, mid_y + 1, out);
        }
        for y in top_y..=mid_y {
            pb.plot(cx + side * (span + 1), y, out);
        }
        // Top & bottom curved brass rims of the bow loop.
        for dx in 1..=span {
            pb.plot(cx + side * dx, top_y, top_col);
            pb.plot(cx + side * dx, mid_y, bot_col);
        }
        // Outer tip connecting top and bottom.
        for y in top_y..=mid_y {
            pb.plot(cx + side * span, y, if y == top_y { top_col } else { bot_col });
        }
        // Hollow eyelet inside the loop when wide enough.
        if span >= 2 && mid_y - top_y >= 2 {
            for dx in 1..span {
                pb.plot(cx + side * dx, top_y + 1, out);
            }
        }
    }
}

/// Floor pickup for the Wind-up Mouse: a small tin clockwork cog base topped
/// by a gleaming spinning brass butterfly key.
fn draw_windup_pickup(pb: &mut PixelBuffer, s: (i32, i32), bob: f32) {
    let (cx, cy) = s;
    for dx in -2..=3 {
        shade_floor(pb, cx + dx, cy + 3, 0.42);
    }
    // Small tin clockwork base under the brass key so it has grounded weight.
    let out: Rgb = (34, 34, 42);
    let tin_hi: Rgb = (206, 212, 224);
    let tin_lo: Rgb = (122, 130, 144);
    pb.rect_fill(cx - 2, cy, 5, 3, out);
    pb.plot(cx - 1, cy + 1, tin_hi);
    pb.plot(cx, cy + 1, tin_hi);
    pb.plot(cx + 1, cy + 1, tin_lo);
    // Little pink nose pip on the clockwork mouse base.
    pb.plot(cx + 2, cy + 1, (248, 142, 168));
    draw_butterfly_key(pb, (cx, cy), bob * 1.4, false);
}

/// Ornate gold-rimmed Pocket Watch with bail loop, crisp pale dial, and twin
/// dark hands rotating smoothly around a brass pin.
fn draw_pocket_watch(pb: &mut PixelBuffer, s: (i32, i32), bob: f32) {
    let (cx, cy) = s;
    for dx in -2..=3 {
        shade_floor(pb, cx + dx, cy + 4, 0.44);
    }

    let out: Rgb = (30, 36, 48);
    let gold_hi: Rgb = (255, 232, 138);
    let gold_mid: Rgb = (218, 168, 62);
    let gold_lo: Rgb = (126, 90, 34);
    let dial: Rgb = (232, 246, 255);
    let tick: Rgb = (138, 164, 186);
    let hand_col: Rgb = (28, 40, 58);

    // 1. Crown stem & open chain bail loop at 12 o'clock.
    pb.plot(cx - 1, cy - 5, gold_hi);
    pb.plot(cx, cy - 5, gold_hi);
    pb.plot(cx + 1, cy - 5, gold_mid);
    pb.plot(cx - 1, cy - 4, gold_mid);
    pb.plot(cx + 1, cy - 4, gold_lo);
    pb.plot(cx, cy - 4, out);
    pb.plot(cx, cy - 3, gold_hi);

    // 2. Dark outer casing contour.
    pb.filled_circle(cx, cy + 1, 3, out);
    pb.filled_circle(cx, cy, 3, out);
    // 3. Beveled golden outer ring.
    for dy in -3..=3i32 {
        for dx in -3..=3i32 {
            let d2 = dx * dx + dy * dy;
            if (6..=9).contains(&d2) {
                let col = if dx + dy <= -2 {
                    gold_hi
                } else if dx + dy >= 2 {
                    gold_lo
                } else {
                    gold_mid
                };
                pb.plot(cx + dx, cy + dy, col);
            }
        }
    }

    // 4. Crisp pale ice-white dial face with subtle cardinal hour ticks.
    pb.filled_circle(cx, cy, 2, dial);
    pb.plot(cx, cy - 2, tick);
    pb.plot(cx, cy + 2, tick);
    pb.plot(cx - 2, cy, tick);
    pb.plot(cx + 2, cy, tick);

    // 5. Twin hands: minute hand sweeps briskly, hour hand drifts behind it.
    let a_min = bob * 0.9 - std::f32::consts::FRAC_PI_2;
    let a_hr = bob * 0.22 - std::f32::consts::FRAC_PI_2 + 1.1;
    let m1 = Vec2::from_angle(a_min);
    let h1 = Vec2::from_angle(a_hr);
    pb.plot(cx + (m1.x * 1.1).round() as i32, cy + (m1.y * 1.1).round() as i32, hand_col);
    pb.plot(cx + (m1.x * 2.0).round() as i32, cy + (m1.y * 2.0).round() as i32, hand_col);
    pb.plot(cx + (h1.x * 1.2).round() as i32, cy + (h1.y * 1.2).round() as i32, hand_col);
    // Center brass pivot pin.
    pb.plot(cx, cy, gold_mid);
}

/// Crisp white athletic tube sock with red & sky-blue varsity cuff stripes,
/// reinforced grey heel and toe cap, and an unmistakable L-shaped foot profile.
fn draw_clean_socks(pb: &mut PixelBuffer, s: (i32, i32), pulse: f32) {
    let (cx, cy) = s;
    for dx in -3..=4 {
        shade_floor(pb, cx + dx, cy + 4, 0.44);
    }

    let out: Rgb = (30, 36, 48);
    let hi: Rgb = (254, 254, 250);
    let mid: Rgb = (218, 224, 234);
    let grey_hi: Rgb = (156, 166, 182);
    let grey_lo: Rgb = (108, 118, 134);
    let red: Rgb = (232, 66, 86);
    let blue: Rgb = (72, 174, 246);

    // 1. Clean L-shaped contour outline (vertical leg left, rounded foot right).
    pb.rect_fill(cx - 3, cy - 5, 5, 9, out);
    pb.rect_fill(cx - 3, cy, 7, 4, out);
    pb.plot(cx + 4, cy + 1, out);
    pb.plot(cx + 4, cy + 2, out);
    if pulse > 0.78 {
        pb.plot_add(cx - 2, cy - 4, (36, 36, 40));
    }
    pb.plot(cx - 2, cy - 4, hi);
    pb.plot(cx - 1, cy - 4, hi);
    pb.plot(cx, cy - 4, mid);
    // Twin varsity stripes near the cuff.
    pb.plot(cx - 2, cy - 3, red);
    pb.plot(cx - 1, cy - 3, red);
    pb.plot(cx, cy - 3, scale(red, 0.78));
    pb.plot(cx - 2, cy - 2, blue);
    pb.plot(cx - 1, cy - 2, blue);
    pb.plot(cx, cy - 2, scale(blue, 0.78));
    // Lower tube ankle.
    pb.plot(cx - 2, cy - 1, hi);
    pb.plot(cx - 1, cy - 1, hi);
    pb.plot(cx, cy - 1, mid);
    // Arch & upper foot extending cleanly to the right.
    pb.plot(cx - 2, cy, grey_hi);
    pb.plot(cx - 1, cy, hi);
    pb.plot(cx, cy, hi);
    pb.plot(cx + 1, cy, mid);
    // Main foot & reinforced grey heel + toe cap.
    pb.plot(cx - 2, cy + 1, grey_hi);
    pb.plot(cx - 1, cy + 1, grey_lo);
    pb.plot(cx, cy + 1, hi);
    pb.plot(cx + 1, cy + 1, hi);
    pb.plot(cx + 2, cy + 1, grey_hi);
    pb.plot(cx + 3, cy + 1, grey_hi);
    // Grey sole along the bottom.
    pb.plot(cx - 2, cy + 2, grey_lo);
    pb.plot(cx - 1, cy + 2, grey_lo);
    pb.plot(cx, cy + 2, grey_hi);
    pb.plot(cx + 1, cy + 2, grey_hi);
    pb.plot(cx + 2, cy + 2, grey_lo);
    pb.plot(cx + 3, cy + 2, grey_lo);
}

/// The stationary decoy key a Wind-up Mouse drops (`systems::powerups::Decoy`).
///
/// Drawn as a chunky tin wind-up toy mouse with a prominent spinning brass
/// butterfly key on its back and soft pulsing golden lure ripples on the floor.
pub fn draw_decoy(pb: &mut PixelBuffer, s: (i32, i32), angle: f32) {
    let (cx, cy) = s;
    // 1. Expanding pulsing brass lure ripples on the floor (two staggered rings).
    for k in 0..2 {
        let phase = (angle * 0.28 + k as f32 * 0.5).fract();
        let r = (4.0 + phase * 10.0).round() as i32;
        let fade = (1.0 - phase) * 0.55;
        let col = scale((244, 196, 86), fade);
        let steps = (r * 6).max(12);
        for i in (0..steps).step_by(2) {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU + angle * 0.2;
            let rx = cx + (a.cos() * r as f32).round() as i32;
            let ry = cy + (a.sin() * r as f32).round() as i32;
            pb.plot_add(rx, ry, col);
        }
    }

    // 2. Grounded contact shadow.
    for dx in -3..=4 {
        shade_floor(pb, cx + dx, cy + 3, 0.48);
    }

    // 3. Tin clockwork toy mouse body (facing right with pink nose & wire tail).
    let out: Rgb = (32, 34, 42);
    let tin_hi: Rgb = (224, 230, 238);
    let tin_mid: Rgb = (164, 174, 188);
    let tin_lo: Rgb = (102, 112, 126);
    let ear: Rgb = (240, 152, 176);

    // Curled wire tail on left.
    pb.plot(cx - 4, cy + 1, tin_lo);
    pb.plot(cx - 5, cy, tin_mid);
    // Body contour + tin shell.
    pb.rect_fill(cx - 3, cy - 1, 7, 4, out);
    pb.rect_fill(cx - 2, cy, 5, 2, tin_mid);
    pb.plot(cx - 2, cy, tin_hi);
    pb.plot(cx - 1, cy, tin_hi);
    pb.plot(cx, cy, tin_hi);
    pb.plot(cx + 1, cy + 1, tin_lo);
    pb.plot(cx + 2, cy + 1, tin_lo);
    // Ears + eye + pink nose.
    pb.plot(cx + 1, cy - 1, ear);
    pb.plot(cx + 2, cy, (24, 20, 28));
    pb.plot(cx + 3, cy + 1, (255, 132, 162));

    // 4. Big spinning brass butterfly key rising off the mouse's back.
    draw_butterfly_key(pb, (cx - 1, cy - 1), angle * 1.4, true);
}

/// Draw a floor Gold coin with minted bevel shading and a distinct animated
/// square glint (design §6: "distinct square glint", never confused with round
/// XP gems or pickups).
pub fn draw_gold_coin(pb: &mut PixelBuffer, s: (i32, i32), value: u32, elapsed: f32, phase: f32) {
    let (cx, cy) = s;
    let out: Rgb = (88, 48, 14);
    let bronze: Rgb = (176, 106, 22);
    let gold: Rgb = (248, 186, 40);
    let hi: Rgb = (255, 234, 118);
    let glint_col: Rgb = (255, 254, 222);

    // Subtle ground contact shadow.
    shade_floor(pb, cx, cy + 3, 0.36);
    shade_floor(pb, cx + 1, cy + 3, 0.36);

    // High-value coins (elite / miniboss drops) show a second coin peeking behind.
    if value >= 5 {
        pb.rect_fill(cx, cy - 1, 4, 4, out);
        pb.plot(cx + 1, cy, gold);
        pb.plot(cx + 2, cy, bronze);
        pb.plot(cx + 1, cy + 1, bronze);
        pb.plot(cx + 2, cy + 1, bronze);
    }

    // Crisp minted coin silhouette (4x4 beveled disc).
    pb.plot(cx - 1, cy - 2, out);
    pb.plot(cx, cy - 2, out);
    pb.plot(cx + 1, cy - 2, out);
    pb.plot(cx - 2, cy - 1, out);
    pb.plot(cx + 2, cy - 1, out);
    pb.plot(cx - 2, cy, out);
    pb.plot(cx + 2, cy, out);
    pb.plot(cx - 2, cy + 1, out);
    pb.plot(cx + 2, cy + 1, out);
    pb.plot(cx - 1, cy + 2, out);
    pb.plot(cx, cy + 2, out);
    pb.plot(cx + 1, cy + 2, out);

    // Gold body with top-left specular bevel & inner stamped sheen.
    pb.plot(cx - 1, cy - 1, hi);
    pb.plot(cx, cy - 1, hi);
    pb.plot(cx + 1, cy - 1, gold);
    pb.plot(cx - 1, cy, hi);
    pb.plot(cx, cy, gold);
    pb.plot(cx + 1, cy, bronze);
    pb.plot(cx - 1, cy + 1, gold);
    pb.plot(cx, cy + 1, bronze);
    pb.plot(cx + 1, cy + 1, bronze);

    // Distinct square glint: sharp crisp 2x2 / framed square flash.
    let wave = ((elapsed * 3.6 + phase).sin() * 0.5 + 0.5).powi(3);
    if wave > 0.38 {
        let k = ((wave - 0.38) / 0.62).clamp(0.0, 1.0);
        // Crisp 2x2 square glint on the top-left corner of the coin.
        pb.plot(cx - 1, cy - 1, mix(hi, glint_col, k));
        pb.plot(cx, cy - 1, mix(hi, glint_col, k * 0.85));
        pb.plot(cx - 1, cy, mix(hi, glint_col, k * 0.85));
        pb.plot(cx, cy, mix(gold, glint_col, k * 0.65));
        // Subtle outer square corner glints when peaking.
        if k > 0.55 {
            let g = scale(glint_col, (k - 0.55) * 1.3);
            pb.plot_add(cx - 2, cy - 2, g);
            pb.plot_add(cx + 1, cy - 2, g);
            pb.plot_add(cx - 2, cy + 1, g);
        }
    }
}

/// Draw a floor Miniboss Chest: an inviting mahogany-and-brass treasure chest
/// with a pulsing warm golden aura, arched lid, glowing seam, lockplate clasp,
/// and periodic corner glint stars.
pub fn draw_chest(pb: &mut PixelBuffer, s: (i32, i32), elapsed: f32) {
    let (cx, cy) = s;
    let pulse = 0.72 + 0.28 * (elapsed * 3.4).sin();

    // 1. Inviting warm golden floor aura so the chest draws the eye across the room.
    let gr = 10i32;
    let grf = gr as f32;
    for dy in -gr..=gr {
        for dx in -gr..=gr {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d <= grf {
                let fall = (1.0 - d / grf).powi(2);
                pb.plot_add(cx + dx, cy + dy, scale((255, 204, 72), fall * pulse * 0.42));
            }
        }
    }

    // 2. Grounded contact shadow shifted down-right.
    for dx in -5..=6 {
        shade_floor(pb, cx + dx, cy + 4, 0.52);
    }
    for dy in -2..=3 {
        shade_floor(pb, cx + 6, cy + dy, 0.34);
    }

    let out: Rgb = (34, 18, 14);
    let wood_hi: Rgb = (206, 118, 50);
    let wood_mid: Rgb = (158, 78, 34);
    let wood_lo: Rgb = (104, 46, 22);
    let brass_hi: Rgb = (255, 226, 102);
    let brass_mid: Rgb = (218, 150, 34);
    let brass_lo: Rgb = (138, 84, 18);
    let glow_seam: Rgb = scale((255, 232, 128), 0.75 + 0.25 * pulse);
    let clasp_hi: Rgb = (255, 252, 206);

    // 3. Arched outer silhouette (11x9 px).
    pb.rect_fill(cx - 4, cy - 4, 9, 1, out);
    pb.rect_fill(cx - 5, cy - 3, 11, 7, out);

    // 4. Mahogany domed lid (y = -3..-1) & lower chest body (y = 1..2).
    pb.rect_fill(cx - 4, cy - 3, 9, 1, wood_hi);
    pb.rect_fill(cx - 4, cy - 2, 9, 2, wood_mid);
    pb.rect_fill(cx - 4, cy + 1, 9, 2, wood_lo);
    // Warm top-left light on the front planks.
    pb.rect_fill(cx - 4, cy + 1, 4, 1, wood_mid);

    // 5. Glowing golden seam between lid and base (treasure waiting inside!).
    for dx in -4..=4 {
        pb.plot(cx + dx, cy, if dx.abs() <= 2 { glow_seam } else { scale(glow_seam, 0.55) });
    }

    // 6. Twin vertical beveled brass strapping bands.
    for bx in [cx - 3, cx + 3] {
        pb.plot(bx, cy - 3, brass_hi);
        pb.plot(bx, cy - 2, brass_mid);
        pb.plot(bx, cy - 1, brass_mid);
        pb.plot(bx, cy, brass_hi);
        pb.plot(bx, cy + 1, brass_mid);
        pb.plot(bx, cy + 2, brass_lo);
    }
    // Corner brass studs on the lid corners.
    pb.plot(cx - 4, cy - 3, brass_hi);
    pb.plot(cx + 4, cy - 3, brass_mid);

    // 7. Central gleaming golden lockplate / clasp with dark keyhole.
    pb.rect_fill(cx - 1, cy - 1, 3, 3, brass_mid);
    pb.plot(cx - 1, cy - 1, clasp_hi);
    pb.plot(cx, cy - 1, clasp_hi);
    pb.plot(cx + 1, cy - 1, brass_hi);
    pb.plot(cx, cy, out); // keyhole
    pb.plot(cx, cy + 1, brass_hi);
    pb.plot(cx + 1, cy + 1, brass_lo);

    // 8. Periodic corner sparkle star (`+`) dancing between top-left & top-right.
    let star_phase = (elapsed * 2.1).rem_euclid(2.0);
    let (sx, sy, local) = if star_phase < 1.0 {
        (cx - 4, cy - 4, star_phase)
    } else {
        (cx + 4, cy - 4, star_phase - 1.0)
    };
    let spark = (1.0 - (local * 2.0 - 1.0).abs()).powi(2);
    if spark > 0.25 {
        let c = scale(clasp_hi, spark);
        pb.plot_add(sx, sy, c);
        pb.plot_add(sx - 1, sy, scale(c, 0.6));
        pb.plot_add(sx + 1, sy, scale(c, 0.6));
        pb.plot_add(sx, sy - 1, scale(c, 0.6));
        pb.plot_add(sx, sy + 1, scale(c, 0.6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_scrolls_as_a_crop_of_the_arena() {
        for theme in Theme::all() {
            let level = Level::new(theme);
            let (width, height) = (level.arena.x as usize, level.arena.y as usize);
            let mut arena = PixelBuffer::new(width, height);
            draw_floor(&mut arena, &level, Vec2::new(0.0, 0.0), 13.0);

            let mut crop = PixelBuffer::new(123, 89);
            for (left, top) in [(-3, -4), (23, 61), (472, 366), (912, 742)] {
                draw_floor(
                    &mut crop,
                    &level,
                    Vec2::new(left as f32 + 0.2, top as f32 - 0.2),
                    13.0,
                );
                for y in 0..crop.h {
                    for x in 0..crop.w {
                        let wx = (left + x as i32) as usize;
                        let wy = (top + y as i32) as usize;
                        let expected = if wx < width && wy < height {
                            arena.pixel_at(wx, wy)
                        } else {
                            scale(level.palette.bg, 0.44)
                        };
                        assert_eq!(crop.pixel_at(x, y), expected, "{theme:?} at ({wx}, {wy})");
                    }
                }
            }
        }
    }
}
