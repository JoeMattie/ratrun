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
        Theme::Outfall => outfall(&mut floor, &level.palette),
        Theme::Trailer => unreachable!("the flatbed draws its own floor"),
    }
    // Walkable hazard surfaces sit on the floor layer.
    hazards(&mut floor, &level.palette, &level.objects, elapsed);
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

    #[inline]
    fn px(&mut self, wx: i32, wy: i32, color: Rgb) {
        if wx >= self.x.start && wx < self.x.end && wy >= self.y.start && wy < self.y.end {
            self.pb.plot(wx - self.cam_x, wy - self.cam_y, color);
        }
    }

    #[inline]
    fn blend_world(&mut self, wx: i32, wy: i32, color: Rgb, alpha: f32) {
        if alpha <= 0.0 || wx < self.x.start || wx >= self.x.end || wy < self.y.start || wy >= self.y.end {
            return;
        }
        let (sx, sy) = ((wx - self.cam_x) as usize, (wy - self.cam_y) as usize);
        if sx < self.pb.w && sy < self.pb.h {
            let cur = self.pb.pixel_at(sx, sy);
            self.pb.plot(sx as i32, sy as i32, mix(cur, color, alpha.clamp(0.0, 1.0)));
        }
    }

    #[inline]
    fn darken_world(&mut self, wx: i32, wy: i32, factor: f32) {
        if wx < self.x.start || wx >= self.x.end || wy < self.y.start || wy >= self.y.end {
            return;
        }
        let (sx, sy) = ((wx - self.cam_x) as usize, (wy - self.cam_y) as usize);
        if sx < self.pb.w && sy < self.pb.h {
            let cur = self.pb.pixel_at(sx, sy);
            self.pb.plot(sx as i32, sy as i32, scale(cur, factor.clamp(0.0, 1.0)));
        }
    }
}

/// 64-entry integer sine LUT in `[-100, 100]` for zero-trig, crop-invariant
/// seabed sand ripples and scalloped tidal-drop shorelines.
const SIN64_LUT: [i32; 64] = [
    0, 10, 20, 29, 38, 47, 56, 63, 71, 77, 83, 88, 92, 96, 98, 100,
    100, 100, 98, 96, 92, 88, 83, 77, 71, 63, 56, 47, 38, 29, 20, 10,
    0, -10, -20, -29, -38, -47, -56, -63, -71, -77, -83, -88, -92, -96, -98, -100,
    -100, -100, -98, -96, -92, -88, -83, -77, -71, -63, -56, -47, -38, -29, -20, -10,
];

#[inline]
fn sin64(idx: i32) -> i32 {
    SIN64_LUT[(idx & 63) as usize]
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

/// The Laundry floor: indigo glazed tile laid over faded-denim grout, soapy
/// rinse puddles, recessed washhouse gutter troughs, and brass drain plates at
/// four-tile junctions. All colours stay well below the entity contrast cap so
/// the white rat, gems and telegraphs pop effortlessly.
fn laundry(floor: &mut Floor<'_>, pal: &Palette, elapsed: f32) {
    const TILE: i32 = 16;
    let grout = scale(pal.bg, 0.66);
    let denim_grout = mix(grout, pal.wall, 0.16);
    let glaze = [
        pal.bg,
        mix(pal.bg, pal.bg_alt, 0.62),
        mix(pal.bg, pal.wall, 0.15),
    ];
    let porcelain = mix(pal.bg_alt, pal.wall_edge, 0.24);
    let wash_pink = mix(pal.bg_alt, pal.blood, 0.22);
    let brass_rim = mix(pal.bg, (132, 102, 48), 0.56);
    let brass_hi = mix(pal.bg, (158, 126, 64), 0.56);
    let throat = scale(pal.bg, 0.42);

    floor.fill_visible(denim_grout);

    for row in floor.rows(TILE, 0) {
        let y = row * TILE;
        for col in floor.columns(TILE, 0) {
            let x = col * TILE;
            let n = hash(col + 73, row + 41);
            let checker = ((col + row) & 1) as usize;
            let tone_idx = if checker == 0 {
                0
            } else if (n & 7) == 0 {
                2
            } else {
                1
            };
            let body = mix(glaze[tone_idx], grout, (n & 3) as f32 * 0.055);
            let bevel_hi = mix(body, porcelain, 0.34);
            let bevel_lo = mix(body, grout, 0.48);

            // Tile body with 1 px wet-denim grout frame and crisp 3/4 bevel.
            floor.fill(x + 1, y + 1, TILE - 1, TILE - 1, body);
            floor.fill(x + 1, y + 1, TILE - 2, 1, bevel_hi);
            floor.fill(x + 1, y + 2, 1, TILE - 3, mix(body, porcelain, 0.20));
            floor.fill(x + 2, y + TILE - 1, TILE - 2, 1, bevel_lo);

            // Subtle porcelain scuff, lint wisp, or ruined-pink dye streak.
            match n & 15 {
                0 | 8 => {
                    // Trolley wheel scuff across the indigo glaze.
                    let sx = x + 3 + (n >> 6 & 5) as i32;
                    let sy = y + 3 + (n >> 9 & 5) as i32;
                    floor.fill(sx, sy, 5 + (n >> 12 & 3) as i32, 1, mix(body, porcelain, 0.34));
                    floor.fill(sx + 2, sy + 2, 4, 1, mix(body, porcelain, 0.18));
                }
                3 => {
                    // Hairline glaze crack near corner.
                    let cx = x + 4 + (n >> 7 & 5) as i32;
                    let cy = y + 4 + (n >> 10 & 4) as i32;
                    floor.fill(cx, cy, 4, 1, grout);
                    floor.fill(cx + 3, cy + 1, 3, 1, grout);
                    floor.fill(cx + 3, cy + 2, 2, 1, bevel_hi);
                }
                5 => {
                    // Pale cotton lint caught in the lower grout seam.
                    for k in 0..3 {
                        floor.fill(x + 2 + k * 4, y + TILE - 2, 2, 1, mix(body, porcelain, 0.26));
                    }
                }
                11 => {
                    // Faint pink rinse streak ("the whites come out pink down here").
                    let sx = x + 3 + (n >> 8 & 4) as i32;
                    let sy = y + 6 + (n >> 11 & 4) as i32;
                    floor.fill(sx, sy, 6, 1, wash_pink);
                    floor.fill(sx + 2, sy + 1, 4, 1, mix(body, wash_pink, 0.55));
                }
                _ => {}
            }
        }
    }

    // Flush washhouse drainage trough every 8 tile rows (128 px): dark wet
    // channel with porcelain lip and brass grate bars so 2x zoom reads as a
    // wet industrial laundry floor.
    let trough_water = mix(throat, pal.wall, 0.24);
    let trough_lip = mix(pal.bg_alt, pal.wall_edge, 0.22);
    for row in floor.rows(128, 48) {
        let y = row * 128 + 48;
        let x = floor.x.start;
        let w = floor.x.end - x;
        floor.fill(x, y, w, 1, trough_lip);
        floor.fill(x, y + 1, w, 4, throat);
        floor.fill(x, y + 2, w, 2, trough_water);
        floor.fill(x, y + 5, w, 1, denim_grout);
        for col in floor.columns(32, 0) {
            let gx = col * 32;
            floor.fill(gx + 3, y + 1, 1, 4, brass_rim);
            floor.fill(gx + 7, y + 1, 1, 4, brass_rim);
            floor.fill(gx + 11, y + 1, 1, 4, brass_rim);
            floor.fill(gx + 16, y + 2, 11, 1, mix(trough_water, porcelain, 0.26));
        }
    }

    // Low soapy rinse puddles scattered across the tiles with slow shimmer.
    let suds_water = mix(grout, pal.wall, 0.22);
    let suds_edge = mix(pal.bg_alt, pal.wall_edge, 0.25);
    for row in floor.rows(52, 12) {
        for col in floor.columns(64, 8) {
            let n = hash(col + 211, row + 59);
            if n & 3 != 1 {
                continue;
            }
            let x = col * 64 + 8 + (n >> 3 & 15) as i32;
            let y = row * 52 + 12 + (n >> 7 & 11) as i32;
            let stretch = (n >> 11 & 7) as i32;
            let pink_tint = (n >> 15 & 3) == 0;
            let pool_col = if pink_tint {
                mix(suds_water, pal.blood, 0.14)
            } else {
                suds_water
            };
            floor.fill(x + 4, y, 10 + stretch, 1, suds_edge);
            floor.fill(x + 1, y + 1, 16 + stretch, 2, pool_col);
            floor.fill(x, y + 3, 18 + stretch, 2, pool_col);
            floor.fill(x + 2, y + 5, 13 + stretch, 1, pool_col);
            floor.fill(x + 5, y + 6, 7 + stretch, 1, scale(pool_col, 0.84));
            let shimmer = ((elapsed * 0.7 + (n >> 18 & 31) as f32).sin() + 1.0) * 0.5;
            let glint = mix(pool_col, suds_edge, 0.45 + 0.45 * shimmer);
            floor.fill(x + 3, y + 2, 5 + stretch / 2, 1, glint);
            floor.fill(x + 10, y + 4, 4, 1, glint);
        }
    }

    // Recessed circular-beveled brass drain plate at every 4×4 tile junction.
    // Centred at +2*TILE inside each 4*TILE bucket so its 11×11 footprint never
    // crosses a bucket boundary (scroll-crop invariance).
    for row in floor.rows(TILE * 4, 0) {
        let cy = row * TILE * 4 + TILE * 2;
        for col in floor.columns(TILE * 4, 0) {
            let cx = col * TILE * 4 + TILE * 2;
            let n = hash(col + 17, row + 89);
            let shimmer = ((elapsed * 0.65 + (n & 31) as f32).sin() + 1.0) * 0.5;
            let lit_brass = mix(brass_rim, brass_hi, 0.35 + 0.55 * shimmer);
            // Outer octagonal brass collar centred on the 4-tile grout cross.
            floor.fill(cx - 3, cy - 5, 7, 11, brass_rim);
            floor.fill(cx - 5, cy - 3, 11, 7, brass_rim);
            floor.fill(cx - 4, cy - 4, 9, 9, brass_rim);
            // Top-left specular rim and recessed dark drain throat.
            floor.fill(cx - 3, cy - 5, 6, 1, lit_brass);
            floor.fill(cx - 5, cy - 3, 1, 5, lit_brass);
            floor.fill(cx - 3, cy - 3, 7, 7, throat);
            // Three slotted brass grate bars across the throat + center hub.
            floor.fill(cx - 2, cy - 3, 1, 7, lit_brass);
            floor.fill(cx, cy - 3, 1, 7, brass_rim);
            floor.fill(cx + 2, cy - 3, 1, 7, brass_rim);
            floor.fill(cx - 3, cy, 7, 1, brass_rim);
            floor.fill(cx - 1, cy - 1, 3, 2, lit_brass);
            // Dark bottom shadow lip.
            floor.fill(cx - 3, cy + 5, 7, 1, throat);
        }
    }
}

/// The Printworks floor: chamfered charcoal press-hall iron/stone slabs with
/// recessed oil-dark seams, corner rivets, dried-ink drags, fallen newsprint
/// broadsheets (low-contrast so entities pop), and cyan registration crosshairs.
fn printworks(floor: &mut Floor<'_>, pal: &Palette, elapsed: f32) {
    const W: i32 = 36;
    const H: i32 = 24;
    let seam = scale(pal.bg, 0.54);
    let slab_a = pal.bg;
    let slab_b = mix(pal.bg, pal.bg_alt, 0.72);
    let bevel_hi = mix(pal.bg_alt, pal.wall, 0.16);
    let bevel_lo = mix(pal.bg, seam, 0.62);
    let rivet_hi = mix(pal.bg_alt, pal.wall_edge, 0.24);
    // Keep fallen newsprint well under the floor contrast cap (~86 peak).
    let paper = mix(pal.bg, pal.wall_edge, 0.36);
    let paper_hi = mix(pal.bg, pal.wall_edge, 0.44);
    let paper_fold = mix(pal.bg, pal.wall_edge, 0.26);
    let ink = mix(pal.bg, pal.blood, 0.42);
    let ink_deep = mix(seam, pal.blood, 0.28);
    let cyan = pal.accent;

    floor.fill_visible(seam);

    for row in floor.rows(H, 0) {
        let y = row * H;
        for col in floor.columns(W, 0) {
            let x = col * W;
            let n = hash(col + 19, row + 61);
            let base = if ((col + row) & 1) == 0 { slab_a } else { slab_b };
            let body = mix(base, seam, (n & 3) as f32 * 0.05);

            // Chamfered press-hall floor plate.
            floor.fill(x + 2, y + 1, W - 4, H - 2, body);
            floor.fill(x + 1, y + 2, W - 2, H - 4, body);
            floor.fill(x + 2, y + 1, W - 4, 1, bevel_hi);
            floor.fill(x + 1, y + 2, 1, H - 4, mix(body, bevel_hi, 0.55));
            floor.fill(x + 2, y + H - 2, W - 4, 1, bevel_lo);

            // Subtle bolted plate corners on every other slab.
            if (n & 1) == 0 {
                floor.fill(x + 3, y + 3, 1, 1, rivet_hi);
                floor.fill(x + W - 4, y + 3, 1, 1, bevel_hi);
                floor.fill(x + 3, y + H - 4, 1, 1, bevel_hi);
                floor.fill(x + W - 4, y + H - 4, 1, 1, seam);
            }

            // Recessed paper-trolley rail groove every 4th row.
            if row.rem_euclid(4) == 1 {
                floor.fill(x + 1, y + H / 2 - 1, W - 2, 1, seam);
                floor.fill(x + 1, y + H / 2, W - 2, 1, mix(body, bevel_hi, 0.45));
            }

            // Dried-ink roller drag smears (low-contrast dried violet-charcoal).
            if n & 3 == 1 {
                let sx = x + 5 + (n >> 5 & 7) as i32;
                let sy = y + 4 + (n >> 9 & 7) as i32;
                let len = 8 + (n >> 13 & 7) as i32;
                floor.fill(sx, sy, len, 2, ink);
                floor.fill(sx + 2, sy + 2, len - 3, 1, ink_deep);
                floor.fill(sx + 4, sy + 4, (len / 2).max(3), 1, mix(body, ink, 0.55));
            }

            // Dropped newsprint broadsheet page (with masthead, two columns,
            // folded dog-ear corner, and cyan registration mark).
            if n & 7 == 2 && row.rem_euclid(4) != 1 {
                let sx = x + 6 + (n >> 12 & 3) as i32;
                let sy = y + 4 + (n >> 15 & 3) as i32;
                let sw = 16 + (n >> 17 & 3) as i32;
                let sh = 11 + (n >> 19 & 3) as i32;
                // Subtle drop shadow + broadsheet body.
                floor.fill(sx + 1, sy + sh, sw, 1, seam);
                floor.fill(sx + sw, sy + 1, 1, sh, seam);
                floor.fill(sx, sy, sw, sh, paper);
                floor.fill(sx, sy, sw - 2, 1, paper_hi);
                // Dog-eared top-right corner fold.
                floor.fill(sx + sw - 2, sy, 2, 2, body);
                floor.fill(sx + sw - 3, sy + 1, 2, 2, paper_fold);
                // Masthead bar + two-column miniature grey text lines.
                floor.fill(sx + 2, sy + 2, sw - 6, 1, ink_deep);
                let col_w = ((sw - 6) / 2).max(3);
                for line in 0..3 {
                    let ly = sy + 4 + line * 2;
                    if ly < sy + sh - 2 {
                        floor.fill(sx + 2, ly, col_w - (line & 1), 1, ink);
                        floor.fill(sx + 4 + col_w, ly, (col_w - 2).max(2), 1, ink);
                    }
                }
                // Cyan registration crop tick in bottom-right corner.
                floor.fill(sx + sw - 4, sy + sh - 2, 2, 1, mix(body, cyan, 0.48));
            }

            // Registration cyan crosshair reticle at press-hall grid nodes.
            if col.rem_euclid(4) == 2 && row.rem_euclid(3) == 2 {
                let pulse = 0.28 + 0.14 * ((elapsed * 0.85 + (n & 31) as f32).sin() * 0.5 + 0.5);
                let rc = mix(body, cyan, pulse);
                let rc_core = mix(body, cyan, pulse + 0.18);
                let mx = x + W / 2;
                let my = y + H / 2;
                // Target crosshair + tiny corner brackets.
                floor.fill(mx - 4, my, 9, 1, rc);
                floor.fill(mx, my - 4, 1, 9, rc);
                floor.fill(mx - 2, my - 2, 1, 1, rc);
                floor.fill(mx + 2, my - 2, 1, 1, rc);
                floor.fill(mx - 2, my + 2, 1, 1, rc);
                floor.fill(mx + 2, my + 2, 1, 1, rc);
                floor.fill(mx, my, 1, 1, rc_core);
            }
        }
    }
}

/// The Outfall floor: warped diagonal ocean-seabed sand ripples with fine silt
/// grain, scattered half-buried pebbles and shell flecks, dark silt hollows,
/// and flat recumbent seabed kelp fronds (visual only). Still and strictly
/// world-anchored so the lighting bump pass stays quiet and scroll crops match.
fn outfall(floor: &mut Floor<'_>, pal: &Palette) {
    let sand_deep = mix((17, 28, 38), pal.bg, 0.12);
    let sand_trough = mix((24, 38, 49), pal.bg, 0.10);
    let sand_mid = mix((35, 52, 62), pal.bg_alt, 0.08);
    let sand_crest: Rgb = (48, 67, 75);
    let sand_hi: Rgb = (58, 78, 85);
    let pebble_lo: Rgb = (24, 37, 46);
    let pebble_hi: Rgb = (68, 86, 92);
    let kelp_dark: Rgb = (14, 42, 36);
    let kelp_mid: Rgb = (22, 62, 50);
    let kelp_hi: Rgb = (32, 82, 64);

    for wy in floor.y.clone() {
        let wy_warp = sin64((wy * 2).div_euclid(3));
        let sy = wy - floor.cam_y;
        for wx in floor.x.clone() {
            let warp = (sin64((wx * 2).div_euclid(3) + 7) * 5 + wy_warp * 4) / 100;
            let phase = (wy * 5 + wx + warp * 2) & 63;
            let r = sin64(phase);
            let grain = ((hash(wx >> 1, wy >> 1) & 7) as i32 - 3) * 5;
            let val = r + grain;

            let mut col = if val < -48 {
                sand_deep
            } else if val < -10 {
                sand_trough
            } else if val < 34 {
                sand_mid
            } else if val < 72 {
                sand_crest
            } else {
                sand_hi
            };
            if val > 54 && (hash(wx, wy) & 15) == 0 {
                col = mix(col, pebble_hi, 0.36);
            }
            floor.pb.plot(wx - floor.cam_x, sy, col);
        }
    }

    // Sparse seabed storytelling details on a 48 × 40 world grid (crop-invariant).
    const STEP_X: i32 = 48;
    const STEP_Y: i32 = 40;
    let gx0 = (floor.x.start - 16).div_euclid(STEP_X);
    let gx1 = (floor.x.end + 16).div_euclid(STEP_X);
    let gy0 = (floor.y.start - 16).div_euclid(STEP_Y);
    let gy1 = (floor.y.end + 16).div_euclid(STEP_Y);
    for gy in gy0..=gy1 {
        for gx in gx0..=gx1 {
            let h = hash(gx + 137, gy + 59);
            let ox = gx * STEP_X + 8 + ((h >> 3) & 31) as i32;
            let oy = gy * STEP_Y + 8 + ((h >> 8) & 23) as i32;
            match h & 7 {
                0 | 1 => {
                    // Flat recumbent seabed kelp tuft anchored to a tiny silt pebble.
                    floor.fill(ox - 1, oy, 3, 2, pebble_lo);
                    floor.px(ox - 1, oy, pebble_hi);
                    let flen = 8 + ((h >> 13) & 5) as i32;
                    let flip = if (h & 16) != 0 { 1 } else { -1 };
                    let p0 = (h & 7) as i32 * 5;
                    for k in 0..flen {
                        let fx = ox + flip * (k + 1);
                        let fy = oy + (sin64(k * 6 + p0) * 14) / 1000 - (k / 4);
                        let c = if k % 3 == 0 {
                            kelp_hi
                        } else if (k & 1) == 0 {
                            kelp_mid
                        } else {
                            kelp_dark
                        };
                        floor.blend_world(fx, fy, c, 0.78);
                        if (k & 1) == 1 {
                            floor.blend_world(fx, fy + 1, kelp_dark, 0.55);
                        }
                    }
                }
                2 => {
                    // Half-buried pale shell & silt pebble cluster.
                    floor.blend_world(ox, oy, pebble_hi, 0.65);
                    floor.blend_world(ox + 1, oy, sand_hi, 0.55);
                    floor.blend_world(ox + 1, oy + 1, pebble_lo, 0.70);
                    floor.blend_world(ox + 4, oy - 1, pebble_hi, 0.50);
                    floor.blend_world(ox - 3, oy + 2, pebble_lo, 0.65);
                }
                3 => {
                    // Sunken dark silt hollow.
                    for dy in -2i32..=2 {
                        for dx in -5i32..=5 {
                            if dx * dx * 4 + dy * dy * 25 <= 100 {
                                floor.darken_world(ox + dx, oy + dy, 0.80);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Floor-layer art for steam grates, ink pads and the drip shaft.
///
/// The surfaces must read before their effects engage.
/// - **Ink pads** are glossy, velvety dried-ink pools with an organic scalloped
///   silhouette, deliberate halftone stipple clusters, a slow-shimmering wet
///   specular crescent, and crisp plum/cyan registration corner ticks marking
///   the exact slow footprint.
/// - **Steam grates** are recessed brass-bolted boiler vents with warm-lit
///   slots even while resting, three distinct rising billowy puffs + a pulsing
///   1-tile floor border during the 1 s warning, and a boiling white-hot steam
///   column during the 0.7 s active plume.
/// - **Drip shaft** leaves a small wet patch below the hostile-layer warning.
fn hazards(
    floor: &mut Floor<'_>,
    pal: &Palette,
    objects: &super::level::LevelObjects,
    elapsed: f32,
) {
    let on_screen = |f: &Floor<'_>, x: i32, y: i32, w: i32, h: i32| {
        x + w > f.x.start && x < f.x.end && y + h > f.y.start && y < f.y.end
    };

    for pad in &objects.ink_pads {
        let (x0, y0) = ((pad.pos.x - pad.half.x) as i32, (pad.pos.y - pad.half.y) as i32);
        let (w, h) = ((pad.half.x * 2.0) as i32, (pad.half.y * 2.0) as i32);
        if !on_screen(floor, x0 - 2, y0 - 2, w + 4, h + 4) {
            continue;
        }
        let ink_deep: Rgb = (24, 15, 34);
        let ink_body: Rgb = (54, 34, 72);
        let ink_mid: Rgb = (78, 52, 102);
        let ink_rim: Rgb = (108, 78, 134);
        let wet_cyan = mix(ink_rim, pal.accent, 0.62);
        let wet_glint = mix((206, 224, 232), pal.accent, 0.35);

        // 1. Subtle 1 px outer wet bleed shadow + organic rounded ink body
        //    covering the pad's exact cell rect.
        floor.fill(x0 + 2, y0, w - 4, h, ink_deep);
        floor.fill(x0, y0 + 2, w, h - 4, ink_deep);
        floor.fill(x0 + 1, y0 + 1, w - 2, h - 2, ink_body);
        floor.fill(x0 + 2, y0 + 2, w - 4, h - 4, ink_body);

        // 2. Crisp printer's halftone stipple pattern (deliberate rhythmic
        //    3×3 dot matrix + recessed dark ink heart so it unmistakably reads
        //    as permanent tacky printer's ink stipple).
        if w > 8 && h > 8 {
            floor.fill(x0 + 3, y0 + 3, w - 6, h - 6, mix(ink_body, ink_deep, 0.32));
        }
        let mut ry = 2;
        while ry < h - 2 {
            let stagger = if (ry / 3) & 1 == 0 { 0 } else { 1 };
            let mut rx = 2 + stagger;
            while rx < w - 2 {
                let hval = hash(x0 + rx + (pad.seed as i32 & 63), y0 + ry);
                if (hval & 3) < 2 {
                    // Deep dried-ink halftone stipple dot.
                    floor.fill(x0 + rx, y0 + ry, 2, 1, ink_deep);
                    if ry + 1 < h - 2 && (hval & 4) != 0 {
                        floor.fill(x0 + rx, y0 + ry + 1, 1, 1, ink_deep);
                    }
                } else if (hval & 7) == 5 {
                    // Raised tacky plum stipple bead.
                    floor.fill(x0 + rx, y0 + ry, 1, 1, ink_rim);
                } else if (hval & 15) == 14 {
                    // Faint cyan-sheen ink reflection fleck.
                    floor.fill(x0 + rx, y0 + ry, 1, 1, mix(ink_mid, wet_cyan, 0.50));
                }
                rx += 3;
            }
            ry += 3;
        }

        // 3. Curved glossy top-left wet specular sheen (slow shimmer so the
        //    pad clearly reads as sticky wet liquid, not dry tile paint).
        let sheen = 0.55 + 0.45 * (elapsed * 1.4 + (pad.seed & 31) as f32).sin();
        let sheen_col = mix(ink_rim, wet_glint, 0.28 + 0.32 * sheen);
        floor.fill(x0 + 3, y0 + 1, (w - 8).max(4), 1, mix(ink_rim, wet_cyan, 0.45));
        floor.fill(x0 + 2, y0 + 2, (w / 2).clamp(4, w - 4), 1, sheen_col);
        floor.fill(x0 + 1, y0 + 3, 1, (h - 7).max(3), mix(ink_rim, wet_cyan, 0.35));
        // Secondary glossy highlight bead inside the pool.
        let gx = x0 + 4 + ((pad.seed >> 4) % 5) as i32;
        let gy = y0 + 4 + ((pad.seed >> 8) % 3) as i32;
        if gx + 4 < x0 + w - 2 && gy + 1 < y0 + h - 2 {
            floor.fill(gx, gy, 3, 1, sheen_col);
            floor.fill(gx + 1, gy + 1, 2, 1, wet_cyan);
        }

        // 4. Tactile cyan registration corner ticks marking the exact slow box
        //    plus 2–3 tiny satellite ink droplets clinging along the bottom rim.
        let tick = mix(ink_mid, pal.accent, 0.72);
        floor.fill(x0, y0, 2, 1, tick);
        floor.fill(x0, y0, 1, 2, tick);
        floor.fill(x0 + w - 2, y0, 2, 1, tick);
        floor.fill(x0 + w - 1, y0, 1, 2, tick);
        floor.fill(x0, y0 + h - 1, 2, 1, tick);
        floor.fill(x0, y0 + h - 2, 1, 2, tick);
        floor.fill(x0 + w - 2, y0 + h - 1, 2, 1, tick);
        floor.fill(x0 + w - 1, y0 + h - 2, 1, 2, tick);

        // Organic scalloped edge drips hugging the lower border.
        let drips = 2 + (pad.seed as i32 & 1);
        for k in 0..drips {
            let dx = 3 + ((k * 7 + (pad.seed as i32 & 3)) % (w - 6).max(1));
            floor.fill(x0 + dx, y0 + h, 2, 1, ink_deep);
            if k == 0 && dx + 1 < w - 1 {
                floor.fill(x0 + dx, y0 + h + 1, 1, 1, ink_body);
            }
        }
    }

    if let Some(opening) = &objects.sewer_opening {
        let pos = opening.drip_shaft.pos;
        let (x, y) = (pos.x as i32, pos.y as i32);
        floor.fill(x - 6, y - 2, 12, 4, (16, 26, 31));
        floor.fill(x - 4, y - 3, 8, 6, (19, 31, 36));
        floor.fill(x - 3, y - 1, 5, 1, (42, 61, 62));
    }

    for grate in &objects.steam_grates {
        let (cx, cy) = (grate.pos.x as i32, grate.pos.y as i32);
        if !on_screen(floor, cx - 10, cy - 36, 20, 46) {
            continue;
        }

        let iron_dark: Rgb = (18, 22, 34);
        let iron_mid: Rgb = (52, 62, 78);
        let iron_hi: Rgb = (96, 114, 128);
        let brass_bolt: Rgb = (176, 142, 72);
        let brass_hi: Rgb = (218, 184, 108);
        let slot_rest: Rgb = (12, 16, 26);
        let slot_warm: Rgb = (118, 86, 52);

        // 1. Recessed boiler-vent grate plate (14 × 14 centered on the 16 px cell)
        //    so it reads as an industrial steam vent even while resting.
        floor.fill(cx - 7, cy - 7, 14, 14, iron_dark);
        floor.fill(cx - 6, cy - 6, 12, 12, iron_mid);
        floor.fill(cx - 6, cy - 6, 12, 1, iron_hi);
        floor.fill(cx - 6, cy - 6, 1, 12, mix(iron_mid, iron_hi, 0.6));
        floor.fill(cx - 6, cy + 5, 12, 1, iron_dark);
        floor.fill(cx + 5, cy - 6, 1, 12, iron_dark);
        // Four brass corner rivets (immediately separates steam grates from flat tiles).
        floor.fill(cx - 5, cy - 5, 1, 1, brass_hi);
        floor.fill(cx + 4, cy - 5, 1, 1, brass_bolt);
        floor.fill(cx - 5, cy + 4, 1, 1, brass_bolt);
        floor.fill(cx + 4, cy + 4, 1, 1, mix(brass_bolt, iron_dark, 0.4));

        // Four horizontal vent louver slots, glowing hotter as warning/plume engages.
        let slot_col = if grate.plume() {
            (238, 248, 255)
        } else if grate.warning() {
            let t = grate.phase.progress(grate.timer);
            mix((168, 144, 104), (242, 244, 248), t)
        } else {
            let breath = 0.5 + 0.5 * (elapsed * 1.8 + cx as f32 * 0.1).sin();
            mix(slot_rest, slot_warm, 0.22 + 0.14 * breath)
        };
        for k in 0..4 {
            let sy = cy - 4 + k * 2 + (k / 2);
            floor.fill(cx - 4, sy, 8, 1, slot_rest);
            floor.fill(cx - 3, sy, 6, 1, slot_col);
        }

        let steam_hi: Rgb = (242, 248, 250);
        let steam_mid: Rgb = (188, 212, 220);
        let steam_lo: Rgb = (114, 146, 166);
        let warn_rim: Rgb = (232, 198, 118);

        if grate.warning() {
            // 1 s tell: crisp 1-tile pulsing danger border on the floor +
            // THREE distinct billowy pale puffs staggered in sequence!
            let t = grate.phase.progress(grate.timer);
            let pulse = 0.55 + 0.45 * (t * std::f32::consts::TAU * 3.0).sin();
            let rim = mix(warn_rim, steam_hi, 0.25 + 0.45 * pulse);
            // Dashed 1-tile danger perimeter (exact 16 × 16 scalding cell).
            for d in (-8..=7).step_by(3) {
                floor.fill(cx + d, cy - 8, 2, 1, rim);
                floor.fill(cx + d, cy + 7, 2, 1, rim);
                floor.fill(cx - 8, cy + d, 1, 2, rim);
                floor.fill(cx + 7, cy + d, 1, 2, rim);
            }
            // Corner danger brackets.
            for (bx, by) in [(-8, -8), (6, -8), (-8, 6), (6, 6)] {
                floor.fill(cx + bx, cy + by, 2, 2, steam_hi);
            }

            // Three discrete rising puffs (puff 0 starts immediately, puff 1 at
            // t=0.22, puff 2 at t=0.46) separated horizontally so all three
            // pale puffs read individually at a glance!
            const PUFF_STARTS: [f32; 3] = [0.0, 0.22, 0.46];
            const PUFF_DX: [i32; 3] = [-4, 4, 0];
            for k in 0..3 {
                let age = (t - PUFF_STARTS[k]) / (1.0 - PUFF_STARTS[k]).max(0.2);
                if age <= 0.0 {
                    continue;
                }
                let age = age.clamp(0.0, 1.0);
                let rise = 5 + (age * 20.0) as i32;
                let sway = PUFF_DX[k]
                    + ((age * std::f32::consts::PI + k as f32).sin() * 1.2).round() as i32;
                let px = cx + sway;
                let py = cy - 2 - rise;
                let rx = 2 + (age * 2.0) as i32;
                let ry = 1 + (age * 1.4) as i32;
                let fade = 1.0 - age * 0.30;
                let c_lo = mix(pal.bg, steam_lo, 0.72 * fade);
                let c_mid = mix(pal.bg, steam_mid, 0.86 * fade);
                let c_hi = mix(pal.bg, steam_hi, 0.95 * fade);
                // Rounded billowy puff lobe (shadow underside + pale crown).
                floor.fill(px - rx, py - ry + 1, rx * 2 + 1, ry * 2 - 1, c_lo);
                floor.fill(px - rx + 1, py - ry, rx * 2 - 1, ry * 2, c_mid);
                floor.fill(px - rx + 1, py - ry, (rx * 2 - 2).max(1), 1, c_hi);
                if rx >= 3 {
                    floor.fill(px - 1, py - ry + 1, 2, 1, c_hi);
                }
            }
        } else if grate.plume() {
            // Active 0.7 s scalding plume: blazing 1-tile floor border +
            // billowing turbulent vertical steam jet with soft aqua-silver edges!
            let t = grate.phase.progress(grate.timer);
            let env = if t < 0.15 {
                t / 0.15
            } else if t > 0.85 {
                (1.0 - t) / 0.15
            } else {
                1.0
            };
            let height = (14.0 + env * 13.0) as i32;
            let hot_rim = mix((255, 224, 152), (255, 255, 255), 0.45);
            // Solid bright 1-tile scalding floor border.
            floor.fill(cx - 8, cy - 8, 16, 1, hot_rim);
            floor.fill(cx - 8, cy + 7, 16, 1, hot_rim);
            floor.fill(cx - 8, cy - 8, 1, 16, hot_rim);
            floor.fill(cx + 7, cy - 8, 1, 16, hot_rim);

            // Base pressurized steam cushion expanding out of the grate.
            floor.fill(cx - 6, cy - 3, 12, 5, steam_lo);
            floor.fill(cx - 5, cy - 4, 10, 5, steam_mid);
            floor.fill(cx - 4, cy - 3, 8, 3, steam_hi);

            // Stacked billowing rolling steam lobes rising up the column with
            // soft steel-aqua undersides and bright sunlit crowns.
            let scroll = elapsed * 16.0;
            let lobes = 6;
            for i in 0..lobes {
                let frac = i as f32 / (lobes - 1) as f32;
                let ly = cy - 3 - (frac * height as f32) as i32;
                let wave = ((scroll + i as f32 * 1.5 + cx as f32 * 0.2).sin() * 2.1).round() as i32;
                let w_half = (4 + ((frac * 2.6) as i32)).min(7);
                let lx = cx + wave;
                let edge_c = mix(pal.bg_alt, steam_lo, 0.72 * (1.0 - frac * 0.25));
                let body_c = mix(steam_lo, steam_mid, 0.55 + 0.35 * (1.0 - frac * 0.4));
                let core_c = mix(steam_mid, steam_hi, 0.78 * (1.0 - frac * 0.35));
                floor.fill(lx - w_half, ly - 1, w_half * 2, 4, edge_c);
                floor.fill(lx - w_half + 1, ly - 3, (w_half * 2 - 2).max(2), 5, body_c);
                floor.fill(lx - (w_half / 2).max(1), ly - 2, (w_half).max(2), 3, core_c);
            }
            // Incandescent narrow central jet throat straight up from the slots.
            floor.fill(cx - 1, cy - height + 4, 3, (height - 5).max(2), steam_hi);
            floor.fill(cx, cy - height + 2, 1, (height - 3).max(2), (255, 255, 255));
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
        Theme::Outfall => {
            let tide = mix(face, pal.accent, 0.36);
            (24, 8, seam, tide)
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

/// Draw **only** the walkable floor hazards of `objects` onto `pb`.
///
/// The Library's hazard cards use this with a hand-built [`LevelObjects`]
/// fixture (one grate, one ink pad), so the steam plume and the ink stipple on
/// a card are the same art the floor paints — no second illustration to drift.
/// `cam` is the world position of the buffer's top-left pixel.
pub fn draw_hazards(
    pb: &mut PixelBuffer,
    theme: Theme,
    objects: &super::level::LevelObjects,
    cam: Vec2,
    elapsed: f32,
) {
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let x = cam_x..cam_x + pb.w as i32;
    let y = cam_y..cam_y + pb.h as i32;
    let mut floor = Floor {
        pb,
        cam_x,
        cam_y,
        x,
        y,
    };
    hazards(&mut floor, &Palette::for_theme(theme), objects, elapsed);
}

/// Draw the ceiling drop and its exact floor footprint above the lights.
pub fn draw_drip_shaft(pb: &mut PixelBuffer, shaft: &super::hazards::DripShaft, cam: Vec2) {
    use super::hazards::{DripPhase, DRIP_HALF, DRIP_SPLASH_SECS, DRIP_WARN_SECS};
    let x = (shaft.pos.x - cam.x).round() as i32;
    let y = (shaft.pos.y - cam.y).round() as i32;
    match shaft.phase {
        DripPhase::Warning => {
            let rim = (240, 202, 124);
            let half = DRIP_HALF as i32;
            let (left, right, top, bottom) = (x - half, x + half - 1, y - half, y + half - 1);
            pb.line(left, top, right, top, rim);
            pb.line(left, bottom, right, bottom, rim);
            pb.line(left, top, left, bottom, rim);
            pb.line(right, top, right, bottom, rim);
            let drop_y = y - 20 + (shaft.timer / DRIP_WARN_SECS * 20.0) as i32;
            pb.rect_fill(x, drop_y - 3, 1, 4, (117, 202, 218));
            pb.rect_fill(x - 1, drop_y, 3, 2, (213, 247, 240));
        }
        DripPhase::Splash => {
            let spread = 2 + (shaft.timer / DRIP_SPLASH_SECS * 5.0) as i32;
            let color = (134, 214, 221);
            pb.line(x - spread, y, x - 1, y - 2, color);
            pb.line(x + 1, y - 2, x + spread, y, color);
            pb.line(x - spread + 1, y + 2, x + spread - 1, y + 2, color);
        }
        DripPhase::Idle | DripPhase::Resting => {}
    }
}

/// **Floor / Ground Hazard Layer (Outfall)**: draw lethal abyssal tidal-drop
/// pits (`level.objects.tidal_pits`) and lure-trap base stalks + bulbs
/// (`level.objects.lure_traps`).
///
/// ## Explicit Pit & Puddle Ordering
/// Call **after** `World::draw_puddles` (and before wall bodies, props and
/// entities) so almost-black tidal drops cut cleanly through any adjacent
/// shallow slate-blue puddles with their scalloped high-contrast surf rim
/// intact, and lure-trap stalks sit visibly on the seabed. Camera-clipped and
/// purely render-only (never advances state or draws RNG).
pub fn draw_outfall_hazards(pb: &mut PixelBuffer, level: &Level, cam: Vec2, elapsed: f32) {
    use super::hazards::LurePhase;

    if level.theme != Theme::Outfall || pb.w == 0 || pb.h == 0 {
        return;
    }
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let bw = pb.w as i32;
    let bh = pb.h as i32;

    // 1. Tidal drop pockets: almost-black abyssal water with a distinct
    //    scalloped undercut lip and high-contrast surf rim.
    const ABYSS: Rgb = (2, 5, 10);
    const DEEP: Rgb = (5, 11, 19);
    const SHELF: Rgb = (11, 24, 36);
    const RIM_HI: Rgb = (178, 228, 238);
    const RIM_MID: Rgb = (112, 174, 192);
    const RIM_LO: Rgb = (48, 88, 108);
    const SAND_LIP: Rgb = (66, 88, 96);
    const EDDY: Rgb = (8, 17, 27);
    let eddy_step = (elapsed.max(0.0) * 3.0).floor() as i32;

    for pit in &level.objects.tidal_pits {
        let rect = pit.rect.to_world();
        let x0 = rect.x.round() as i32;
        let y0 = rect.y.round() as i32;
        let w = rect.w.round() as i32;
        let h = rect.h.round() as i32;
        if w <= 0 || h <= 0 {
            continue;
        }
        let sx0 = x0 - cam_x;
        let sy0 = y0 - cam_y;
        if sx0 + w + 2 <= 0 || sx0 - 2 >= bw || sy0 + h + 2 <= 0 || sy0 - 2 >= bh {
            continue;
        }

        let ly_start = (-2).max(-sy0);
        let ly_end = (h + 2).min(bh - sy0);
        let lx_start = (-2).max(-sx0);
        let lx_end = (w + 2).min(bw - sx0);

        for ly in ly_start..ly_end {
            let py = sy0 + ly;
            let wy = y0 + ly;
            let in_t = ly;
            let in_b = h - 1 - ly;
            let min_v = in_t.min(in_b);
            for lx in lx_start..lx_end {
                let px = sx0 + lx;
                let wx = x0 + lx;
                let in_l = lx;
                let in_r = w - 1 - lx;
                let min_h = in_l.min(in_r);
                let edge_d = min_h.min(min_v);

                let axis_coord = if min_v <= min_h { wx } else { wy };
                let sc_wave = sin64(axis_coord * 6 + (x0 + y0) * 3) + sin64(axis_coord * 11) / 2;
                let scallop = if sc_wave > 55 {
                    2
                } else if sc_wave > -25 {
                    1
                } else {
                    0
                };
                let corner = ((3 - in_l).max(0) + (3 - in_r).max(0))
                    * ((3 - in_t).max(0) + (3 - in_b).max(0));
                let c_in = if corner >= 5 {
                    2
                } else if corner >= 2 {
                    1
                } else {
                    0
                };
                let d_eff = edge_d - scallop - c_in;

                if d_eff < -2 {
                    continue;
                } else if d_eff == -2 {
                    let cur = pb.pixel_at(px as usize, py as usize);
                    pb.plot(px, py, mix(cur, SAND_LIP, 0.38));
                } else if d_eff == -1 {
                    let cur = pb.pixel_at(px as usize, py as usize);
                    pb.plot(px, py, mix(cur, SAND_LIP, 0.65));
                } else if d_eff == 0 {
                    let top_left = (in_t <= in_b && in_t <= in_r) || (in_l <= in_r && in_l <= in_b);
                    let col = if top_left {
                        if (wx + wy).rem_euclid(4) != 0 {
                            RIM_HI
                        } else {
                            RIM_MID
                        }
                    } else if scallop == 2 && (wx + wy).rem_euclid(3) == 0 {
                        RIM_HI
                    } else if ((wx + wy) & 1) == 0 {
                        RIM_MID
                    } else {
                        RIM_LO
                    };
                    pb.plot(px, py, col);
                } else if d_eff == 1 {
                    let col = if in_t <= 3 || in_l <= 3 { RIM_LO } else { SHELF };
                    pb.plot(px, py, col);
                } else if d_eff == 2 {
                    pb.plot(px, py, DEEP);
                } else {
                    let eh = hash((wx + eddy_step).div_euclid(6), wy.div_euclid(3));
                    let col = if (eh & 31) == 0 && (wx + eddy_step).rem_euclid(6) < 3 {
                        EDDY
                    } else {
                        ABYSS
                    };
                    pb.plot(px, py, col);
                }
            }
        }
    }

    // 2. Lure trap seabed mounts & 6 px cyan bulbs (dark husk when spent).
    let bulb_step = (elapsed.max(0.0) * 3.0).floor();
    for trap in &level.objects.lure_traps {
        let cx = trap.pos.x.round() as i32 - cam_x;
        let cy = trap.pos.y.round() as i32 - cam_y;
        if cx < -12 || cy < -12 || cx >= bw + 12 || cy >= bh + 12 {
            continue;
        }
        // Grounded seabed silt mound + bent dark stalk.
        for dy in -2i32..=3 {
            for dx in -4i32..=4 {
                if dx * dx + dy * dy * 2 <= 14 {
                    shade_floor(pb, cx + dx, cy + 2 + dy, 0.42);
                }
            }
        }
        for &(dx, dy) in &[(-2, 3), (-1, 2), (0, 2), (1, 2), (2, 3)] {
            pb.plot(cx + dx, cy + dy, (26, 38, 46));
        }
        for &(dx, dy) in &[(-2, 2), (-3, 0), (-2, -2), (-1, -3), (0, -2)] {
            pb.plot(cx + dx, cy + dy, (18, 28, 34));
        }

        match trap.phase {
            LurePhase::Spent => {
                // Dark deflated husk with zero cyan emission.
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        if dx * dx + dy * dy <= 4 {
                            let c = if dx + dy < 0 {
                                (44, 54, 62)
                            } else {
                                (20, 28, 34)
                            };
                            pb.plot(cx + dx, cy + dy, c);
                        }
                    }
                }
            }
            LurePhase::Ready | LurePhase::Warning => {
                let warn = matches!(trap.phase, LurePhase::Warning);
                let pulse = if warn {
                    1.0
                } else {
                    0.88 + 0.12 * (bulb_step * 1.1 + trap.pos.x * 0.1).sin()
                };
                let outer = scale((36, 218, 206), pulse);
                let body = scale((82, 250, 236), pulse);
                let core = if warn { (242, 255, 252) } else { (216, 255, 248) };
                // Subtle 1 px aqua corona around the 6 px bulb.
                for dy in -4i32..=4 {
                    for dx in -4i32..=4 {
                        let d2 = dx * dx + dy * dy;
                        if d2 > 9 && d2 <= 16 {
                            let (px, py) = (cx + dx, cy + dy);
                            if px >= 0 && py >= 0 && px < bw && py < bh {
                                let cur = pb.pixel_at(px as usize, py as usize);
                                pb.plot(px, py, mix(cur, (28, 138, 136), 0.35 * pulse));
                            }
                        }
                    }
                }
                for dy in -3i32..=3 {
                    for dx in -3i32..=3 {
                        let d2 = dx * dx + dy * dy;
                        if d2 <= 9 {
                            let col = if d2 <= 2 {
                                core
                            } else if d2 <= 5 {
                                body
                            } else {
                                outer
                            };
                            pb.plot(cx + dx, cy + dy, col);
                        }
                    }
                }
            }
        }
    }
}

/// Self-lit 16 px lure-trap warning rings above lighting and foreground.
pub fn draw_outfall_tells(pb: &mut PixelBuffer, level: &Level, cam: Vec2, elapsed: f32) {
    use super::hazards::LurePhase;

    if level.theme != Theme::Outfall || pb.w == 0 || pb.h == 0 {
        return;
    }
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    const OUTLINE: Rgb = (8, 18, 28);

    // Armed lure-trap 16 px warning rings (0.7 s tell before Conger release).
    for trap in &level.objects.lure_traps {
        if !matches!(trap.phase, LurePhase::Warning) {
            continue;
        }
        let cx = trap.pos.x.round() as i32 - cam_x;
        let cy = trap.pos.y.round() as i32 - cam_y;
        if cx < -24 || cy < -24 || cx >= bw + 24 || cy >= bh + 24 {
            continue;
        }
        let pulse = 0.55 + 0.45 * (elapsed * 14.0).sin();
        let ring_col = mix((96, 244, 236), (242, 255, 255), pulse);
        const STEPS: i32 = 72;
        const R: f32 = 16.0;
        // Pass 1: continuous dark outline backing so the ring survives bright spills.
        for i in 0..STEPS {
            let a = i as f32 * std::f32::consts::TAU / STEPS as f32;
            let x = cx + (a.cos() * R).round() as i32;
            let y = cy + (a.sin() * R).round() as i32;
            for &(ox, oy) in &[(-1, 0), (1, 0), (0, -1), (0, 1)] {
                pb.plot(x + ox, y + oy, OUTLINE);
            }
        }
        // Pass 2: crisp dashed cyan-white 16 px warning ring.
        for i in 0..STEPS {
            if (i % 6) < 4 {
                let a = i as f32 * std::f32::consts::TAU / STEPS as f32;
                let x = cx + (a.cos() * R).round() as i32;
                let y = cy + (a.sin() * R).round() as i32;
                pb.plot(x, y, ring_col);
            }
        }
        // Pass 3: contracting inner pulse ring + blazing self-lit bulb core.
        let inner_r = 13.5 - (elapsed * 1.6).rem_euclid(1.0) * 9.5;
        for i in (0..36).step_by(2) {
            let a = i as f32 * std::f32::consts::TAU / 36.0;
            let x = cx + (a.cos() * inner_r).round() as i32;
            let y = cy + (a.sin() * inner_r).round() as i32;
            pb.plot(x, y, (186, 255, 250));
        }
        pb.filled_circle(cx, cy, 2, (236, 255, 252));
    }
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
        // The three legacy pickups used to be the same two-pixel disc in three
        // colours, which a colour-folded terminal (or a lit floor) could not
        // tell apart. Each keeps its colour and gets its own silhouette.
        PickupKind::Heal => draw_heal_cross(pb, s, scale(base, 0.7 + pulse * 0.3)),
        PickupKind::Magnet => draw_horseshoe(pb, s, scale(base, 0.7 + pulse * 0.3)),
        PickupKind::Nuke => draw_nuke_burst(pb, s, scale(base, 0.7 + pulse * 0.3), pulse),
        PickupKind::BottleCap => draw_bottle_cap(pb, s, pulse),
        PickupKind::WindupMouse => draw_windup_pickup(pb, s, bob),
        PickupKind::PocketWatch => draw_pocket_watch(pb, s, bob),
        PickupKind::CleanSocks => draw_clean_socks(pb, s, pulse),
        PickupKind::GrowthSerum => draw_growth_serum(pb, s, bob),
        PickupKind::ShrinkDraught | PickupKind::WhiteCoat | PickupKind::TheCollar => {
            super::art_pickups3::draw_pickup3(pb, s, kind, bob * 0.25);
        }
    }
}

/// Dark contour shared by the three legacy pickups, so each silhouette holds
/// against a bright floor or its own halo.
const PICKUP_OUTLINE: Rgb = (18, 22, 30);

/// Heal: a fat medical cross (7 px across) with a white core.
fn draw_heal_cross(pb: &mut PixelBuffer, s: (i32, i32), body: Rgb) {
    let (cx, cy) = s;
    // Contour: the cross grown by one pixel.
    pb.rect_fill(cx - 4, cy - 2, 9, 5, PICKUP_OUTLINE);
    pb.rect_fill(cx - 2, cy - 4, 5, 9, PICKUP_OUTLINE);
    // Body: two 7 × 3 bars.
    pb.rect_fill(cx - 3, cy - 1, 7, 3, body);
    pb.rect_fill(cx - 1, cy - 3, 3, 7, body);
    // White core cross.
    pb.rect_fill(cx - 2, cy, 5, 1, (255, 255, 255));
    pb.rect_fill(cx, cy - 2, 1, 5, (255, 255, 255));
}

/// Magnet: a horseshoe, open downward, with white pole tips.
fn draw_horseshoe(pb: &mut PixelBuffer, s: (i32, i32), body: Rgb) {
    let (cx, cy) = s;
    // Contour.
    pb.rect_fill(cx - 3, cy - 4, 7, 1, PICKUP_OUTLINE);
    pb.rect_fill(cx - 4, cy - 3, 9, 2, PICKUP_OUTLINE);
    pb.rect_fill(cx - 4, cy - 1, 4, 5, PICKUP_OUTLINE);
    pb.rect_fill(cx + 1, cy - 1, 4, 5, PICKUP_OUTLINE);
    // The bow and the two arms.
    pb.rect_fill(cx - 2, cy - 3, 5, 1, body);
    pb.rect_fill(cx - 3, cy - 2, 7, 1, body);
    pb.rect_fill(cx - 3, cy - 1, 2, 3, body);
    pb.rect_fill(cx + 2, cy - 1, 2, 3, body);
    // Top-left sheen on the bow.
    pb.plot(cx - 2, cy - 3, mix(body, (255, 255, 255), 0.55));
    pb.plot(cx - 1, cy - 3, mix(body, (255, 255, 255), 0.55));
    // White pole tips.
    pb.rect_fill(cx - 3, cy + 2, 2, 1, (255, 255, 255));
    pb.rect_fill(cx + 2, cy + 2, 2, 1, (255, 255, 255));
}

/// Nuke: an eight-point burst round a hot core; the diagonal rays breathe
/// with `pulse` so it is the one pickup that visibly throbs.
fn draw_nuke_burst(pb: &mut PixelBuffer, s: (i32, i32), body: Rgb, pulse: f32) {
    let (cx, cy) = s;
    pb.filled_circle(cx, cy, 3, PICKUP_OUTLINE);
    // Cardinal rays, outlined by the disc where they leave it.
    for d in 1..=4 {
        pb.plot(cx + d, cy, body);
        pb.plot(cx - d, cy, body);
        pb.plot(cx, cy + d, body);
        pb.plot(cx, cy - d, body);
    }
    // Diagonal rays: two pixels, three at the top of the pulse.
    let reach = if pulse > 0.8 { 3 } else { 2 };
    for d in 1..=reach {
        for (sx, sy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            pb.plot(cx + sx * d, cy + sy * d, body);
        }
    }
    // Hot core: yellow ring round a white centre.
    let hot = mix(body, (255, 240, 150), 0.7);
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        pb.plot(cx + dx, cy + dy, hot);
    }
    pb.plot(cx, cy, (255, 255, 255));
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

/// Growth Serum (batch 13): a squat lab flask of hot pink growth medium, corked,
/// with two bubbles rising through it.
///
/// It is the one find that is **emissive**: the meniscus and the liquid's core
/// are mixed toward white (and the vial gets a faint additive halo) so it glows
/// on the dark floors, exactly like the halos the caller paints round every
/// pickup. The alternating bubble height is `bob`, so a row of serums on the
/// floor never bubble in step.
fn draw_growth_serum(pb: &mut PixelBuffer, s: (i32, i32), bob: f32) {
    let (cx, cy) = s;
    for dx in -3..=4 {
        shade_floor(pb, cx + dx, cy + 5, 0.44);
    }

    let out: Rgb = (30, 24, 40);
    let glass_hi: Rgb = (238, 228, 248);
    let glass_lo: Rgb = (148, 128, 168);
    let liquid_hi: Rgb = (255, 168, 232);
    let liquid: Rgb = (232, 48, 168);
    let liquid_lo: Rgb = (146, 14, 102);
    let cork: Rgb = (198, 148, 96);
    let cork_lo: Rgb = (140, 96, 58);

    // 1. Dark contour: a narrow neck over a rounded body (the flask silhouette).
    pb.rect_fill(cx - 2, cy - 5, 5, 3, out);
    pb.rect_fill(cx - 3, cy - 2, 7, 7, out);
    pb.plot(cx - 4, cy + 2, out);
    pb.plot(cx + 4, cy + 2, out);
    pb.plot(cx - 4, cy + 1, out);
    pb.plot(cx + 4, cy + 1, out);

    // 2. Fluted cork plugging the neck, lit from the top-left like everything.
    pb.rect_fill(cx - 1, cy - 6, 3, 2, cork);
    pb.plot(cx - 1, cy - 6, mix(cork, (255, 255, 255), 0.4));
    pb.plot(cx + 1, cy - 5, cork_lo);

    // 3. Glass walls of the neck: bright on the lit side, steel on the far side.
    pb.plot(cx - 2, cy - 4, glass_hi);
    pb.plot(cx - 2, cy - 3, glass_hi);
    pb.plot(cx + 2, cy - 4, glass_lo);
    pb.plot(cx + 2, cy - 3, glass_lo);

    // 4. The medium: a bright pink body with a white meniscus and a dark
    //    meniscus line, so it reads as liquid rather than as a pink block.
    pb.rect_fill(cx - 3, cy - 1, 7, 5, liquid);
    pb.rect_fill(cx - 2, cy + 1, 5, 3, liquid_hi);
    pb.rect_fill(cx - 2, cy + 2, 3, 2, liquid_hi);
    pb.rect_fill(cx - 3, cy + 4, 7, 1, liquid_lo);
    pb.rect_fill(cx - 2, cy - 1, 5, 1, mix(liquid, (255, 255, 255), 0.62));
    // Glass rim highlights down the lit edge and the far wall's shadow.
    pb.plot(cx - 3, cy - 1, glass_hi);
    pb.plot(cx - 3, cy, glass_hi);
    pb.plot(cx - 3, cy + 1, glass_hi);
    pb.plot(cx + 3, cy, glass_lo);
    pb.plot(cx + 3, cy + 1, glass_lo);
    pb.plot(cx + 3, cy + 2, glass_lo);

    // 5. Two bubbles climbing the medium, out of phase with each other.
    let t = bob * 1.6;
    for (i, bx) in [-1i32, 1].into_iter().enumerate() {
        let phase = t + i as f32 * 1.9;
        let rise = ((phase.sin() * 0.5 + 0.5) * 3.0).round() as i32;
        pb.plot(cx + bx, cy + 3 - rise, (255, 238, 252));
    }

    // 6. Emissive core: one additive pixel so the find glows through the dark.
    pb.plot_add(cx, cy + 2, (86, 26, 66));
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
        // One additive plot per pixel: an expanding ring whose steps doubled up
        // on some pixels shimmered as it grew.
        let mut seen = crate::game::art_wave3a::RingDedup::new();
        for i in (0..steps).step_by(2) {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU + angle * 0.2;
            let rx = cx + (a.cos() * r as f32).round() as i32;
            let ry = cy + (a.sin() * r as f32).round() as i32;
            if seen.repeat((rx, ry)) {
                continue;
            }
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
    fn drip_shaft_warning_outlines_one_tile_and_the_drop_falls() {
        use crate::game::hazards::{DripShaft, DRIP_WARN_SECS};
        let mut shaft = DripShaft::new((1, 1));
        let mut pb = PixelBuffer::new(48, 48);
        let bg = (1, 2, 3);
        pb.clear(bg);
        draw_drip_shaft(&mut pb, &shaft, Vec2::ZERO);
        assert!(pb.pixels().iter().all(|&pixel| pixel == bg));

        shaft.tick(0.1, true);
        draw_drip_shaft(&mut pb, &shaft, Vec2::ZERO);
        for (x, y) in [(16, 16), (31, 16), (16, 31), (31, 31)] {
            assert_eq!(pb.pixel_at(x, y), (240, 202, 124));
        }
        assert_eq!(pb.pixel_at(24, 4), (213, 247, 240));
        assert_eq!(pb.pixel_at(15, 16), bg);

        shaft.tick(DRIP_WARN_SECS * 0.5, true);
        pb.clear(bg);
        draw_drip_shaft(&mut pb, &shaft, Vec2::ZERO);
        assert_eq!(pb.pixel_at(24, 4), bg);
        assert_eq!(pb.pixel_at(24, 14), (213, 247, 240));
    }

    /// Every pickup paints a different silhouette (the set of pixels it
    /// touches), not just a different colour, and stays inside a 13 px box.
    #[test]
    fn pickups_have_pairwise_distinct_silhouettes() {
        use crate::game::gem::PickupKind;
        const BG: Rgb = (1, 2, 3);
        let shape = |kind: PickupKind| -> Vec<bool> {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear(BG);
            draw_pickup(&mut pb, kind, (16, 16), 0.0);
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let inside = (10..=22).contains(&x) && (10..=22).contains(&y);
                    assert!(inside || pb.pixel_at(x, y) == BG, "{kind:?} paints ({x}, {y})");
                }
            }
            pb.pixels().iter().map(|&p| p != BG).collect()
        };
        let kinds = PickupKind::all();
        let shapes: Vec<Vec<bool>> = kinds.iter().map(|&k| shape(k)).collect();
        for i in 0..kinds.len() {
            assert!(shapes[i].iter().any(|&p| p), "{:?} drew nothing", kinds[i]);
            for j in i + 1..kinds.len() {
                assert!(shapes[i] != shapes[j], "{:?} and {:?} share a silhouette", kinds[i], kinds[j]);
            }
        }
    }

    /// Drawing at and past every buffer edge must clip, never panic.
    #[test]
    fn pickups_clip_at_the_buffer_edges() {
        use crate::game::gem::PickupKind;
        let mut pb = PixelBuffer::new(8, 8);
        for kind in PickupKind::all() {
            for s in [(0, 0), (7, 7), (-3, 4), (4, -3), (11, 4), (4, 11), (-40, -40)] {
                draw_pickup(&mut pb, kind, s, 1.3);
            }
        }
    }

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

    #[test]
    fn outfall_hazards_and_tells_distinguish_lure_phases_without_a_surge_arrow() {
        use crate::game::hazards::LurePhase;

        let mut level = Level::generate(Theme::Outfall, 7);
        assert!(
            !level.objects.lure_traps.is_empty(),
            "generated Outfall map should carry lure traps"
        );
        let focus = level.objects.lure_traps[0].pos;
        let cam = focus - Vec2::new(60.0, 40.0);

        // Ready vs Warning vs Spent lure bulbs render distinctly, and only
        // Warning paints on the hostile tell layer.
        let mut pb_ready = PixelBuffer::new(120, 80);
        let mut pb_warn = PixelBuffer::new(120, 80);
        let mut pb_spent = PixelBuffer::new(120, 80);
        draw_floor(&mut pb_ready, &level, cam, 1.0);
        draw_floor(&mut pb_warn, &level, cam, 1.0);
        draw_floor(&mut pb_spent, &level, cam, 1.0);

        level.objects.lure_traps[0].phase = LurePhase::Ready;
        draw_outfall_hazards(&mut pb_ready, &level, cam, 1.0);
        let mut tell_ready = pb_ready.pixels().to_vec();
        let mut pb_tr = PixelBuffer::new(120, 80);
        pb_tr.pixels_mut().copy_from_slice(&tell_ready);
        draw_outfall_tells(&mut pb_tr, &level, cam, 1.0);
        tell_ready = pb_tr.pixels().to_vec();
        assert_eq!(
            tell_ready,
            pb_ready.pixels(),
            "Ready lure must not paint on the hostile tell layer"
        );

        level.objects.lure_traps[0].phase = LurePhase::Warning;
        draw_outfall_hazards(&mut pb_warn, &level, cam, 1.0);
        let before_tell = pb_warn.pixels().to_vec();
        draw_outfall_tells(&mut pb_warn, &level, cam, 1.0);
        assert_ne!(
            before_tell,
            pb_warn.pixels(),
            "Warning lure must paint its 16 px hostile warning ring"
        );

        level.objects.lure_traps[0].phase = LurePhase::Spent;
        draw_outfall_hazards(&mut pb_spent, &level, cam, 1.0);
        assert_ne!(
            pb_ready.pixels(),
            pb_spent.pixels(),
            "Spent lure husk must look distinct from a lit Ready bulb"
        );

        // The hostile pass stays clear at both surge phases with no armed lures.
        level.objects.lure_traps.clear();
        let base_col = (18, 28, 38);
        let mut pb = PixelBuffer::new(120, 80);
        pb.clear(base_col);
        for t in [8.5, 9.55] {
            draw_outfall_tells(&mut pb, &level, cam, t);
            assert!(pb.pixels().iter().all(|&p| p == base_col), "t={t}");
        }
    }
}
