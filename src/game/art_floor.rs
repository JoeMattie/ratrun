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

// ---- Floor finds: pickups and the Wind-up Mouse's key --------------------
//
// Placeholder two-tone sprites for the wave-3 pocket finds (WAVE3_DESIGN §10):
// a tin dome with a white rim (Bottle Cap), a brass key that visibly turns
// (Wind-up Mouse), a pocket-watch face with a stem and a moving hand (Pocket
// Watch) and a little white sock with a grey sole (Clean Socks). Each is five
// to eight pixels across, so it reads at half-block resolution. The Neon art
// pass replaces these; the layer order and the call site do not change.
//
// The three legacy finds keep exactly the sprite they had in `World::draw` (a
// 5 px disc with a white core and the pulsing halo drawn by the caller).

/// Draw one floor pickup at screen position `s` (already `w2s`-mapped).
///
/// `bob` is the pickup's animation phase (`Pickup::bob`); it drives the halo
/// pulse for the legacy disc and the key's spin for the Wind-up Mouse.
pub fn draw_pickup(pb: &mut PixelBuffer, kind: crate::game::gem::PickupKind, s: (i32, i32), bob: f32) {
    use crate::game::gem::PickupKind;
    let pulse = 0.5 + 0.5 * (bob.sin() * 0.5 + 0.5);
    let base = kind.color();
    match kind {
        PickupKind::Heal | PickupKind::Magnet | PickupKind::Nuke => {
            pb.filled_circle(s.0, s.1, 2, scale(base, 0.7 + pulse * 0.3));
            pb.plot(s.0, s.1, (255, 255, 255));
        }
        PickupKind::BottleCap => {
            // Tin dome, white rim, two dark crimps at the base.
            let tin = scale(base, 0.75 + pulse * 0.25);
            pb.filled_circle(s.0, s.1 - 1, 2, tin);
            pb.ring(s.0, s.1 - 1, 3, (250, 248, 240));
            pb.plot(s.0 - 2, s.1 + 2, (128, 124, 116));
            pb.plot(s.0 + 2, s.1 + 2, (128, 124, 116));
            pb.plot(s.0 - 1, s.1 - 2, (255, 255, 250));
        }
        PickupKind::WindupMouse => draw_windup_key(pb, s, bob, base),
        PickupKind::PocketWatch => {
            // Cool clock face: a rim, a white dial and a stem at 12 o'clock.
            pb.ring(s.0, s.1, 2, scale(base, 0.55));
            pb.filled_circle(s.0, s.1, 1, (236, 246, 250));
            pb.plot(s.0, s.1 - 4, (208, 200, 176));
            pb.plot(s.0, s.1 - 3, (208, 200, 176));
            // The hand turns once per bob cycle.
            let a = bob * 0.5 - std::f32::consts::FRAC_PI_2;
            let hand = Vec2::from_angle(a) * 1.6;
            pb.plot(s.0 + hand.x.round() as i32, s.1 + hand.y.round() as i32, (48, 62, 74));
        }
        PickupKind::CleanSocks => {
            // A little white sock: leg up the left, foot to the right, grey sole.
            let white = scale(base, 0.85 + pulse * 0.15);
            for dy in -3..=0 {
                pb.plot(s.0 - 1, s.1 + dy, white);
                pb.plot(s.0, s.1 + dy, scale(white, 0.82));
            }
            for dx in -1..=2 {
                pb.plot(s.0 + dx, s.1 + 1, white);
            }
            for dx in -1..=2 {
                pb.plot(s.0 + dx, s.1 + 2, scale(white, 0.6));
            }
            pb.plot(s.0 + 2, s.1, scale(white, 0.82));
        }
    }
}

/// The Wind-up Mouse's brass key on the floor: a bow, a shaft and two teeth,
/// turning about the shaft's end (`angle` = the pickup's `bob`).
fn draw_windup_key(pb: &mut PixelBuffer, s: (i32, i32), angle: f32, brass: Rgb) {
    let d = Vec2::from_angle(angle);
    let side = d.perp();
    let bright = scale(brass, 1.0);
    let dark = scale(brass, 0.55);
    for k in 0..4 {
        let at = d * k as f32;
        pb.plot(s.0 + at.x.round() as i32, s.1 + at.y.round() as i32, bright);
        // A shaded underside reads as brass rather than a flat line.
        pb.plot(
            s.0 + (at.x + side.x * 0.6).round() as i32,
            s.1 + (at.y + side.y * 0.6).round() as i32,
            dark,
        );
    }
    // Bow (the loop you hold).
    let bow = d * -2.0;
    let bc = (s.0 + bow.x.round() as i32, s.1 + bow.y.round() as i32);
    pb.plot(bc.0, bc.1, bright);
    pb.plot(bc.0 + 1, bc.1, dark);
    pb.plot(bc.0 - 1, bc.1, dark);
    pb.plot(bc.0, bc.1 + 1, dark);
    // Teeth at the tip.
    for k in [3.0f32, 4.0] {
        let at = d * k + side * 1.0;
        pb.plot(s.0 + at.x.round() as i32, s.1 + at.y.round() as i32, bright);
    }
}

/// The stationary decoy key a Wind-up Mouse drops (`systems::powerups::Decoy`).
///
/// Bigger than a floor pickup's key (it is the thing the horde is walking at)
/// and drawn with a soft shadow so it reads against the floor.
pub fn draw_decoy(pb: &mut PixelBuffer, s: (i32, i32), angle: f32) {
    pb.filled_circle(s.0, s.1 + 3, 3, scale((10, 10, 12), 0.5));
    draw_windup_key(pb, s, angle, (240, 200, 110));
    pb.plot(s.0, s.1 - 1, (255, 246, 210));
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
