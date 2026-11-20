//! Floor-layer environmental storytelling markings:
//!
//! 1. **Gnaw Marks** ([`draw_gnaw_mark`]) — organic rat-incisor V-gouges,
//!    paw-scuffs, and kicked floor shavings leading from a public corridor fork
//!    into a maze entrance breach across the final 4 cells (64 px). No glow,
//!    no hazard palette, no gold, and no geometric chevrons (so they never read
//!    as pickups, traps, or Lab's decorative floor lanes).
//! 2. **Gold-Fleck Trail** ([`draw_gold_fleck`]) — fainter, sparse tarnished
//!    gold dust rubbed along an interior maze route toward the gilded cache.
//!    Recommended *sparingly*: place 1 fleck 1–2 cells inside the entrance to
//!    link the gnaw trail to the treasure, plus 2–3 flecks along the final 3–4
//!    cells approaching the cache so mid-maze forks stay genuinely exploratory.
//! 3. **Boss Arena Floor** ([`draw_boss_arena_floor`]) — low-contrast thematic
//!    motifs repeated every 4 cells (`64 × 64 px`) across the boss arena
//!    rectangle so the player immediately knows they have entered the boss's
//!    territory:
//!    - **Sewer**: deep 4-claw jaguar rake gouges and damp weathered bones.
//!    - **Kitchen**: scuffed diner-checkerboard glaze overlaid with enormous
//!      rubber shoe-sole treads, heel skids, and faint broom-bristle arcs.
//!    - **Lab**: alternating 2-cell mown turf-bay stripes with wheel-track
//!      seams and low-contrast painted charging-dock bay outlines.
//! 4. **Hall Landmarks** ([`draw_hall_landmark`]) — flush, no-collision floor
//!    landmarks with [`HALL_LANDMARK_VARIANTS`] (6) distinct variants per theme
//!    so major halls are instantly tellable apart:
//!    - **Sewer**: flush recessed cast-iron drain grates & sub-floor pipe bends.
//!    - **Kitchen**: flat counter-end floor mats / threshold aprons & fallen
//!      rat-scale bread crusts with distinct spills.
//!    - **Lab**: worn stencilled industrial bay numbers (`01`..`06`, scaling up
//!      to `99`) with distinct sector-colored frames and badges.
//!
//! # Render Pass Order (for implementers)
//!
//! Every function here paints strictly into the **floor layer** (no collision,
//! zero heap allocations, early-culled to `PixelBuffer` bounds). Call them
//! immediately after [`super::art_floor::draw_floor`] and before floor stains,
//! wall contact shadows, traps, props, and entities:
//!
//! 1. `art_floor::draw_floor`
//! 2. `art_marks::draw_boss_arena_floor` (for `level.boss_room()`)
//! 3. `art_marks::draw_hall_landmark` (for each major hall)
//! 4. `art_marks::draw_gnaw_mark` & `art_marks::draw_gold_fleck`
//! 5. Floor stains, wall ground layer, traps, props, entities, player rat.

#![allow(dead_code)]

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::collision::TILE;
use super::level::{Theme, TileRect};

// ============================================================================
// Public Placement Constants
// ============================================================================

/// Recommended centre-to-centre spacing (world px) between consecutive gnaw
/// marks along the 4-cell (64 px) approach corridor to a maze entrance.
/// At `19 px` (~1.2 cells), at least two marks sit simultaneously on a
/// `120 × 90 px` camera view with ~9 px of clean floor between them.
pub const GNAW_MARK_SPACING: f32 = 19.0;

/// Recommended distances (world px) back from the maze entrance cell centre
/// along `-approach_dir` for the three gnaw marks spanning the final 4 cells:
/// `1.0`, `2.2`, and `3.4` cells out (so the outermost mark sits right at the
/// public corridor fork).
pub const GNAW_MARK_OFFSETS: [f32; 3] = [16.0, 35.0, 54.0];

/// Recommended spacing (world px) for optional interior [`draw_gold_fleck`]
/// hints placed sparsely near the entrance (1–2 cells in) and along the final
/// 3–4 cells approaching the maze cache.
pub const GOLD_FLECK_SPACING: f32 = 22.0;

/// Grid period (in 16 px cells) of the repeating boss arena floor motif
/// (`4 × 4 cells = 64 × 64 world px`).
pub const BOSS_ARENA_MOTIF_CELLS: i32 = 4;

/// Number of distinct visual hall landmark variants per theme (`0..6`).
pub const HALL_LANDMARK_VARIANTS: u8 = 6;

/// World-space bounding half-size (px) of a hall landmark (for culling).
pub const HALL_LANDMARK_HALF_EXTENT: f32 = 24.0;

// ============================================================================
// Shared Low-Level Drawing Helpers (Zero Allocation)
// ============================================================================

/// Consistent top-left directional light shared across `art_rat`, `art_walls`,
/// `art_props`, and `art_traps`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

#[derive(Clone, Copy)]
struct ClipRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl ClipRect {
    #[inline]
    fn from_pb(pb: &PixelBuffer) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: pb.w as i32,
            y1: pb.h as i32,
        }
    }

    #[inline]
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

#[inline]
fn hash2(x: i32, y: i32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846c_a68b);
    n ^= n >> 16;
    n
}

#[inline]
fn safe_dir(d: Vec2) -> Vec2 {
    if !d.x.is_finite() || !d.y.is_finite() {
        return Vec2::new(1.0, 0.0);
    }
    let l2 = d.len_sq();
    if l2 > 1e-6 {
        d * (1.0 / l2.sqrt())
    } else {
        Vec2::new(1.0, 0.0)
    }
}

#[inline]
fn cam_snap(cam: Vec2) -> (i32, i32) {
    let cx = if cam.x.is_finite() {
        cam.x.round() as i32
    } else {
        0
    };
    let cy = if cam.y.is_finite() {
        cam.y.round() as i32
    } else {
        0
    };
    (cx, cy)
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, mix(cur, c, alpha.clamp(0.0, 1.0)));
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, factor: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale(cur, factor.clamp(0.0, 1.0)));
}

#[inline]
fn blend_clip(pb: &mut PixelBuffer, clip: ClipRect, x: i32, y: i32, c: Rgb, alpha: f32) {
    if clip.contains(x, y) {
        blend_px(pb, x, y, c, alpha);
    }
}

#[inline]
fn darken_clip(pb: &mut PixelBuffer, clip: ClipRect, x: i32, y: i32, factor: f32) {
    if clip.contains(x, y) {
        darken_px(pb, x, y, factor);
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn rect_blend_clip(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    c: Rgb,
    alpha: f32,
) {
    if w <= 0 || h <= 0 || alpha <= 0.0 {
        return;
    }
    let xa = x0.max(clip.x0);
    let xb = x0.saturating_add(w).min(clip.x1);
    let ya = y0.max(clip.y0);
    let yb = y0.saturating_add(h).min(clip.y1);
    if xa >= xb || ya >= yb {
        return;
    }
    let a = alpha.clamp(0.0, 1.0);
    for y in ya..yb {
        for x in xa..xb {
            let cur = pb.pixel_at(x as usize, y as usize);
            pb.plot(x, y, mix(cur, c, a));
        }
    }
}

#[inline]
fn sd_rbox(dx: f32, dy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = dx.abs() - hw + r;
    let qy = dy.abs() - hh + r;
    let out = Vec2::new(qx.max(0.0), qy.max(0.0)).len();
    let inside = qx.max(qy).min(0.0);
    out + inside - r
}

/// Draw a slightly curved tapered floor gouge/scratch from `a` to `b` with
/// transverse `bend`, painting both a recessed dark groove and a chipped
/// top-left shoulder highlight. Zero allocations.
#[allow(clippy::too_many_arguments)]
fn draw_floor_gouge(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    a: Vec2,
    b: Vec2,
    bend: f32,
    trough_col: Rgb,
    trough_alpha: f32,
    lip_col: Rgb,
    lip_alpha: f32,
) {
    let delta = b - a;
    let len = delta.len();
    if len < 0.5 {
        return;
    }
    let perp = Vec2::new(-delta.y / len, delta.x / len);
    // Offset the chipped highlight lip toward the top-left light source.
    let lip_ofs = if perp.dot(LIGHT) >= 0.0 {
        perp * 0.9
    } else {
        perp * -0.9
    };
    let steps = ((len * 2.2).ceil() as i32).clamp(3, 64);
    let mut last_t = (i32::MIN, i32::MIN);
    let mut last_l = (i32::MIN, i32::MIN);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let arch = 4.0 * t * (1.0 - t); // 0 at tips, 1 at middle
        let taper = (0.35 + 0.65 * arch).clamp(0.0, 1.0);
        let p = a + delta * t + perp * (bend * arch);

        let lx = (p.x + lip_ofs.x).round() as i32;
        let ly = (p.y + lip_ofs.y).round() as i32;
        if (lx, ly) != last_l {
            last_l = (lx, ly);
            blend_clip(pb, clip, lx, ly, lip_col, lip_alpha * taper);
        }

        let tx = p.x.round() as i32;
        let ty = p.y.round() as i32;
        if (tx, ty) != last_t {
            last_t = (tx, ty);
            blend_clip(pb, clip, tx, ty, trough_col, trough_alpha * taper);
        }
    }
}

// ============================================================================
// 1. Gnaw Marks & Optional Interior Gold-Fleck Trail
// ============================================================================

/// **Floor Layer**: Draw one directional rat gnaw-mark trail sign at
/// `world_pos`, pointing along `direction` toward a maze entrance breach.
///
/// Designed to be stamped three times across the final 4 cells (64 px) before
/// a maze entrance at [`GNAW_MARK_OFFSETS`] (`16`, `35`, `54 px` back from the
/// entrance centre, ~[`GNAW_MARK_SPACING`] = `19 px` apart). Each mark draws
/// paired converging rat-incisor V-gouges with chipped mineral-dust burrs, a
/// trailing claw-scuff tick, and kicked floor shavings — readable at playing
/// scale on all three floors without glow, gold, hazard hues, or geometric
/// chevrons.
pub fn draw_gnaw_mark(
    pb: &mut PixelBuffer,
    world_pos: Vec2,
    direction: Vec2,
    cam: Vec2,
    theme: Theme,
) {
    if !world_pos.x.is_finite() || !world_pos.y.is_finite() {
        return;
    }
    let (cam_x, cam_y) = cam_snap(cam);
    let sx = world_pos.x - cam_x as f32;
    let sy = world_pos.y - cam_y as f32;
    if sx < -10.0 || sy < -10.0 || sx > pb.w as f32 + 10.0 || sy > pb.h as f32 + 10.0 {
        return;
    }

    let clip = ClipRect::from_pb(pb);
    let f = safe_dir(direction);
    let p = f.perp();
    let origin = Vec2::new(sx, sy);

    // Deterministic subtle per-mark variation so 3 marks in a row feel
    // hand-gnawed rather than mechanically rubber-stamped.
    let h = hash2(world_pos.x.round() as i32, world_pos.y.round() as i32);
    let jitter_u = ((h & 7) as f32 - 3.5) * 0.12;
    let jitter_v = (((h >> 3) & 7) as f32 - 3.5) * 0.14;
    let flip = if (h >> 6) & 1 == 0 { 1.0_f32 } else { -1.0_f32 };

    // Theme-matched scraped mineral / bisque / brushed-primer tones.
    // Deliberately non-gold, non-hazard, non-glowing.
    let (trough, dust, chip) = match theme {
        Theme::Sewer => ((5, 9, 8), (112, 126, 114), (164, 176, 162)),
        Theme::Kitchen => ((10, 7, 5), (144, 120, 96), (186, 162, 134)),
        Theme::Lab => ((5, 6, 12), (116, 128, 152), (170, 182, 204)),
    };

    let place = |u: f32, v: f32| origin + f * u + p * v;

    // 1. Soft floor scuff pad beneath the gouge so it separates cleanly from
    //    brick mortar, grout, or Lab's faint decorative chevrons.
    let bx0 = (sx - 6.0).floor() as i32;
    let bx1 = (sx + 6.0).ceil() as i32;
    let by0 = (sy - 6.0).floor() as i32;
    let by1 = (sy + 6.0).ceil() as i32;
    for y in by0.max(0)..=by1.min(pb.h as i32 - 1) {
        for x in bx0.max(0)..=bx1.min(pb.w as i32 - 1) {
            let dx = x as f32 - sx;
            let dy = y as f32 - sy;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let e = (u / 5.2).powi(2) + (v / 3.4).powi(2);
            if e < 1.0 {
                darken_px(pb, x, y, 0.84 + 0.16 * e);
            }
        }
    }

    // 2. Paired converging rat-incisor V-gouges pointing dead-true along
    //    +direction. Two clean, slightly outward-bowed tooth furrows converge
    //    to a crisp chisel apex with twin incisor ridges — immediately
    //    directional at every angle without looking like a geometric chevron.
    let tip_u = 4.3 + jitter_u * 0.5;
    let tail_u = -2.2 + jitter_u * 0.35;
    let upper_a = place(tail_u, -3.2 + jitter_v * 0.2);
    let upper_b = place(tip_u, -0.45);
    let lower_a = place(tail_u - 0.2, 3.2 + jitter_v * 0.2);
    let lower_b = place(tip_u, 0.45);

    draw_floor_gouge(pb, clip, upper_a, upper_b, -0.48, trough, 0.86, dust, 0.82);
    draw_floor_gouge(pb, clip, lower_a, lower_b, 0.48, trough, 0.86, dust, 0.82);

    // Crisp chipped mineral edge burrs reinforcing the forward V apex and twin
    // incisor ridges (clean silhouette on all 8 compass headings).
    for &(u, v, a) in &[
        (-1.2, -2.9, 0.55_f32),
        (0.4, -2.3, 0.78),
        (2.0, -1.5, 0.86),
        (3.4, -0.8, 0.90),
        (-1.3, 2.9, 0.52),
        (0.3, 2.3, 0.74),
        (2.0, 1.5, 0.84),
        (3.4, 0.8, 0.90),
        (4.5, 0.0, 0.92),
    ] {
        let q = place(u + jitter_u * 0.25, v);
        blend_clip(pb, clip, q.x.round() as i32, q.y.round() as i32, chip, a);
    }

    // Subtle inner secondary tooth-scrape notch tucked inside the V (gives the
    // paired-rodent-incisor double groove look without muddying the tip).
    let in_u = place(-0.4, -1.25);
    let in_b = place(1.9, -0.25);
    let in_l = place(-0.4, 1.25);
    let in_lb = place(1.9, 0.25);
    draw_floor_gouge(pb, clip, in_u, in_b, -0.15, trough, 0.68, dust, 0.48);
    draw_floor_gouge(pb, clip, in_l, in_lb, 0.15, trough, 0.68, dust, 0.48);

    // 3. Two clean staggered trailing paw-scuffs behind the wedge (-u) and
    //    2 tiny kicked shavings ahead of the tip (+u).
    for &(u, v) in &[(-4.0, -1.25 * flip), (-3.1, 1.35 * flip)] {
        let q = place(u, v);
        let (qx, qy) = (q.x.round() as i32, q.y.round() as i32);
        darken_clip(pb, clip, qx + 1, qy + 1, 0.74);
        blend_clip(pb, clip, qx, qy, dust, 0.72);
    }
    for &(u, v, a) in &[
        (5.4, -flip, 0.66_f32),
        (5.0, 1.4 * flip, 0.52),
    ] {
        let q = place(u + jitter_u * 0.5, v + jitter_v * 0.4);
        blend_clip(pb, clip, q.x.round() as i32, q.y.round() as i32, chip, a);
    }
}

/// **Floor Layer**: Draw a faint, sparse trail of rubbed gold-leaf flecks
/// inside a maze corridor, loosely aligned along `direction` toward the cache.
///
/// Gold is strictly reserved for the cache and this interior hint. Unlike
/// pickups and XP gems, gold flecks have **no halo and no solid core** — just
/// 5–6 dusty 1 px grains with one slow-breathing specular mote (`elapsed`).
///
/// **Recommendation**: Use sparingly — place 1 mark just inside the maze
/// entrance (1–2 cells in) to confirm the treasure link, then 2–3 marks along
/// the final 3–4 cells approaching the cache (~[`GOLD_FLECK_SPACING`] = `22 px`
/// apart), leaving mid-maze branches and loops unspoiled.
pub fn draw_gold_fleck(
    pb: &mut PixelBuffer,
    world_pos: Vec2,
    direction: Vec2,
    cam: Vec2,
    elapsed: f32,
) {
    if !world_pos.x.is_finite() || !world_pos.y.is_finite() {
        return;
    }
    let (cam_x, cam_y) = cam_snap(cam);
    let sx = world_pos.x - cam_x as f32;
    let sy = world_pos.y - cam_y as f32;
    if sx < -8.0 || sy < -8.0 || sx > pb.w as f32 + 8.0 || sy > pb.h as f32 + 8.0 {
        return;
    }

    let clip = ClipRect::from_pb(pb);
    let f = safe_dir(direction);
    let p = f.perp();
    let origin = Vec2::new(sx, sy);
    let h = hash2(
        world_pos.x.round() as i32 + 41,
        world_pos.y.round() as i32 - 19,
    );
    let flip = if h & 1 == 0 { 1.0_f32 } else { -1.0_f32 };
    let t = if elapsed.is_finite() { elapsed } else { 0.0 };
    let phase = ((h >> 4) & 63) as f32 * 0.1;

    // Antique rubbed gold dust palette (matches MazeReward brass/gold family,
    // kept low-alpha so it never competes with glowing gems or pickups).
    let gold_dim: Rgb = (164, 126, 48);
    let gold_mid: Rgb = (218, 178, 76);
    let gold_glint: Rgb = (250, 222, 128);

    // 6 scattered dust grains tapering cleanly forward along `direction`.
    let grains: [(f32, f32, Rgb, f32); 6] = [
        (-4.0, 0.8 * flip, gold_dim, 0.34),
        (-2.0, -1.3 * flip, gold_dim, 0.46),
        (0.2, 1.0 * flip, gold_mid, 0.54),
        (1.9, -0.9 * flip, gold_mid, 0.64),
        (3.2, 0.4 * flip, gold_mid, 0.66),
        (4.3, -0.1 * flip, gold_dim, 0.48),
    ];
    for (i, &(u, v, col, a)) in grains.iter().enumerate() {
        let ju = (((h >> (8 + i * 2)) & 3) as f32 - 1.5) * 0.22;
        let jv = (((h >> (10 + i * 2)) & 3) as f32 - 1.5) * 0.22;
        let q = origin + f * (u + ju) + p * (v + jv);
        blend_clip(pb, clip, q.x.round() as i32, q.y.round() as i32, col, a);
    }

    // One gentle slow-breathing specular flake near the leading tip.
    let shimmer = 0.5 + 0.5 * (t * 2.2 + phase).sin();
    let glint_pos = origin + f * 2.6 + p * (-0.2 * flip);
    blend_clip(
        pb,
        clip,
        glint_pos.x.round() as i32,
        glint_pos.y.round() as i32,
        gold_glint,
        0.34 + 0.48 * shimmer,
    );
}

// ============================================================================
// 2. Boss Arena Floor (4x4-Cell Repeating Thematic Motif)
// ============================================================================

/// **Floor Layer**: Paint the repeating 4×4-cell (`64 × 64 px`) boss-arena
/// floor motif strictly inside `arena`.
///
/// Call immediately after [`super::art_floor::draw_floor`] for
/// `level.boss_room()` so it sits beneath floor stains, traps, boss telegraphs,
/// and entities. Stays strictly low-contrast like the rest of the floor while
/// making arrival in the boss chamber unmistakable on all three themes.
pub fn draw_boss_arena_floor(pb: &mut PixelBuffer, arena: TileRect, theme: Theme, cam: Vec2) {
    if arena.w <= 0 || arena.h <= 0 || pb.w == 0 || pb.h == 0 {
        return;
    }
    let (cam_x, cam_y) = cam_snap(cam);
    let ax0 = arena.x * TILE;
    let ay0 = arena.y * TILE;
    let ax1 = arena.right() * TILE;
    let ay1 = arena.bottom() * TILE;

    // Intersect arena world bounds with the visible pixel buffer.
    let clip = ClipRect {
        x0: (ax0 - cam_x).max(0),
        y0: (ay0 - cam_y).max(0),
        x1: (ax1 - cam_x).min(pb.w as i32),
        y1: (ay1 - cam_y).min(pb.h as i32),
    };
    if clip.is_empty() {
        return;
    }

    const BLOCK: i32 = BOSS_ARENA_MOTIF_CELLS * TILE; // 64 world px

    // First-pass continuous surface treatments (Kitchen checker wash, Lab mown
    // stripes) and low-contrast arena perimeter framing.
    match theme {
        Theme::Sewer => {
            boss_arena_perimeter(pb, clip, ax0, ay0, ax1, ay1, cam_x, cam_y, (46, 66, 58), 0.28);
        }
        Theme::Kitchen => {
            boss_kitchen_checker(pb, clip, ax0, ay0, cam_x, cam_y);
            boss_arena_perimeter(pb, clip, ax0, ay0, ax1, ay1, cam_x, cam_y, (86, 68, 48), 0.30);
        }
        Theme::Lab => {
            boss_lab_mown_stripes(pb, clip, ax0, cam_x);
            boss_arena_perimeter(pb, clip, ax0, ay0, ax1, ay1, cam_x, cam_y, (72, 86, 62), 0.32);
        }
    }

    // Iterate every 4×4-cell block overlapping the visible arena slice.
    let vis_wx0 = clip.x0 + cam_x;
    let vis_wx1 = clip.x1 + cam_x;
    let vis_wy0 = clip.y0 + cam_y;
    let vis_wy1 = clip.y1 + cam_y;

    let bx_start = (vis_wx0 - ax0).div_euclid(BLOCK);
    let bx_end = (vis_wx1 - 1 - ax0).div_euclid(BLOCK);
    let by_start = (vis_wy0 - ay0).div_euclid(BLOCK);
    let by_end = (vis_wy1 - 1 - ay0).div_euclid(BLOCK);

    for by in by_start..=by_end {
        for bx in bx_start..=bx_end {
            let sx = ax0 + bx * BLOCK - cam_x;
            let sy = ay0 + by * BLOCK - cam_y;
            match theme {
                Theme::Sewer => boss_sewer_block(pb, clip, sx, sy, bx, by),
                Theme::Kitchen => boss_kitchen_block(pb, clip, sx, sy, bx, by),
                Theme::Lab => boss_lab_block(pb, clip, sx, sy, bx, by),
            }
        }
    }
}

/// Subtle 2 px inset border just inside the boss arena boundary so stepping
/// across the boss doorway reads as crossing a threshold.
#[allow(clippy::too_many_arguments)]
fn boss_arena_perimeter(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    ax0: i32,
    ay0: i32,
    ax1: i32,
    ay1: i32,
    cam_x: i32,
    cam_y: i32,
    col: Rgb,
    alpha: f32,
) {
    let sx0 = ax0 - cam_x + 2;
    let sy0 = ay0 - cam_y + 2;
    let sw = (ax1 - ax0) - 4;
    let sh = (ay1 - ay0) - 4;
    if sw <= 4 || sh <= 4 {
        return;
    }
    rect_blend_clip(pb, clip, sx0, sy0, sw, 1, col, alpha);
    rect_blend_clip(pb, clip, sx0, sy0 + sh - 1, sw, 1, col, alpha);
    rect_blend_clip(pb, clip, sx0, sy0 + 1, 1, sh - 2, col, alpha);
    rect_blend_clip(pb, clip, sx0 + sw - 1, sy0 + 1, 1, sh - 2, col, alpha);
}

/// Sewer Boss Den (4×4 cell = 64×64 px block): deep 4-claw jaguar rake gouges
/// and damp weathered bone fragments, strictly low contrast.
fn boss_sewer_block(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    sx: i32,
    sy: i32,
    bx: i32,
    by: i32,
) {
    let h = hash2(bx + 31, by + 73);
    let trough: Rgb = (5, 9, 8);
    let chip: Rgb = (62, 82, 74);

    // Subtle damp den-grime wash beneath the claw rake.
    let cx = sx + 28 + ((h & 7) as i32 - 3);
    let cy = sy + 28 + (((h >> 3) & 7) as i32 - 3);
    for dy in -11..=11 {
        for dx in -13..=13 {
            let e = (dx as f32 / 13.0).powi(2) + (dy as f32 / 11.0).powi(2);
            if e < 1.0 {
                darken_clip(pb, clip, cx + dx, cy + dy, 0.88 + 0.12 * e);
            }
        }
    }

    // Sweeping 4-claw jaguar rake across the heart of the 64×64 block.
    let ang_idx = ((h >> 6) & 3) as usize;
    let angles = [0.48_f32, -0.58, 0.88, -0.95];
    let ang = angles[ang_idx] + (((h >> 8) & 7) as f32 - 3.5) * 0.04;
    let dir = Vec2::from_angle(ang);
    let perp = dir.perp();
    let rake_center = Vec2::new(cx as f32, cy as f32);
    let bend_sign = if (h >> 11) & 1 == 0 { 1.0_f32 } else { -1.0_f32 };

    for claw in 0..4 {
        let offset_v = (claw as f32 - 1.5) * 3.4;
        // Outer claws are slightly shorter than the middle two, like a real paw!
        let half_len = if claw == 0 || claw == 3 { 9.5 } else { 12.5 };
        let stagger = (claw as f32 - 1.5) * 1.4;
        let a = rake_center + dir * (-half_len + stagger) + perp * offset_v;
        let b = rake_center + dir * (half_len + stagger) + perp * offset_v;
        draw_floor_gouge(
            pb,
            clip,
            a,
            b,
            1.6 * bend_sign,
            trough,
            0.84,
            chip,
            0.44,
        );
    }

    // Secondary short 3-claw swipe in the diagonal corner on alternating blocks.
    if (bx + by) & 1 == 0 {
        let sc = Vec2::new((sx + 50) as f32, (sy + 14) as f32);
        let sd = Vec2::from_angle(-ang + 0.35);
        let sp = sd.perp();
        for claw in 0..3 {
            let ov = (claw as f32 - 1.0) * 2.8;
            let a = sc - sd * 5.5 + sp * ov;
            let b = sc + sd * 5.5 + sp * ov;
            draw_floor_gouge(pb, clip, a, b, 0.9, trough, 0.68, chip, 0.32);
        }
    }

    // Scattered weathered bones tucked in the opposite quadrant (muted damp
    // bone grey — low contrast so it never looks like the pale rat or a pickup).
    // Slight per-block offset breaks any grid-row alignment across a big den.
    let bone_x = sx
        + if (h >> 12) & 1 == 0 { 10 } else { 46 }
        + ((h >> 17) & 7) as i32;
    let bone_y = sy
        + if (h >> 13) & 1 == 0 { 44 } else { 10 }
        + ((h >> 20) & 7) as i32;
    let bone_kind = (h >> 14) % 3;
    draw_sewer_bone_cluster(pb, clip, bone_x, bone_y, bone_kind, (h >> 16) & 1 != 0);
}

/// Draw a small low-contrast weathered bone cluster (femur, rib arc, or fish-spine/skull).
fn draw_sewer_bone_cluster(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    cx: i32,
    cy: i32,
    kind: u32,
    flip_x: bool,
) {
    let bone_sh: Rgb = (46, 54, 48);
    let bone_mid: Rgb = (74, 84, 72);
    let bone_hi: Rgb = (106, 114, 98);
    let sx = if flip_x { -1 } else { 1 };

    let plot_bone = |pb: &mut PixelBuffer, dx: i32, dy: i32, col: Rgb, a: f32| {
        let x = cx + dx * sx;
        let y = cy + dy;
        darken_clip(pb, clip, x + 1, y + 1, 0.66);
        blend_clip(pb, clip, x, y, col, a);
    };

    match kind {
        0 => {
            // Classic knobby femur lying slightly diagonally (~10×5 px).
            for &(dx, dy, c) in &[
                (-4, -2, bone_hi),
                (-5, -1, bone_mid),
                (-4, 0, bone_sh),
                (-3, -1, bone_hi),
                (-2, -1, bone_mid),
                (-1, 0, bone_hi),
                (0, 0, bone_mid),
                (1, 0, bone_mid),
                (2, 1, bone_mid),
                (3, 1, bone_sh),
                (4, 0, bone_hi),
                (5, 1, bone_mid),
                (4, 2, bone_sh),
                (3, 2, bone_mid),
            ] {
                plot_bone(pb, dx, dy, c, 0.78);
            }
        }
        1 => {
            // Curved 3-rib cage fragment with spine.
            for &(dx, dy, c) in &[
                (-4, -2, bone_mid),
                (-3, -1, bone_hi),
                (-2, 0, bone_mid),
                (-1, 1, bone_mid),
                (0, 2, bone_sh),
                // Rib 1
                (-2, -3, bone_hi),
                (-1, -3, bone_hi),
                (0, -2, bone_mid),
                (1, -2, bone_sh),
                // Rib 2
                (-1, -1, bone_hi),
                (0, -1, bone_hi),
                (1, 0, bone_mid),
                (2, 0, bone_sh),
                // Rib 3
                (0, 1, bone_hi),
                (1, 1, bone_mid),
                (2, 2, bone_sh),
            ] {
                plot_bone(pb, dx, dy, c, 0.78);
            }
        }
        _ => {
            // Small weathered jaw/skull + loose femur shard.
            for &(dx, dy, c) in &[
                (-3, -2, bone_hi),
                (-2, -2, bone_hi),
                (-1, -2, bone_mid),
                (-4, -1, bone_mid),
                (-3, -1, (20, 28, 25)), // dark eye socket
                (-2, -1, bone_mid),
                (-1, -1, (20, 28, 25)), // dark eye socket
                (0, -1, bone_mid),
                (-3, 0, bone_mid),
                (-2, 0, bone_sh),
                (-1, 0, bone_mid),
                // Loose bone shard nearby
                (2, 2, bone_hi),
                (3, 2, bone_hi),
                (4, 3, bone_mid),
                (5, 3, bone_sh),
            ] {
                plot_bone(pb, dx, dy, c, 0.80);
            }
        }
    }
}

/// Kitchen Boss Arena continuous surface: clean inlaid 16×16-px (1-cell) worn
/// diner-checkerboard tile floor replacing the base 20 px grid cleanly so there
/// is zero moiré, while keeping the same dark espresso low-contrast luminance.
fn boss_kitchen_checker(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    ax0: i32,
    ay0: i32,
    cam_x: i32,
    cam_y: i32,
) {
    let grout: Rgb = (13, 10, 8);
    let dark_tile: Rgb = (19, 15, 13);
    let light_tile: Rgb = (42, 34, 27);
    let light_bevel: Rgb = (54, 44, 35);
    let dark_bevel: Rgb = (27, 22, 19);

    for sy in clip.y0..clip.y1 {
        let wy = sy + cam_y - ay0;
        let cy = wy.div_euclid(TILE);
        let ly = wy.rem_euclid(TILE);
        for sx in clip.x0..clip.x1 {
            let wx = sx + cam_x - ax0;
            let cx = wx.div_euclid(TILE);
            let lx = wx.rem_euclid(TILE);
            let cur = pb.pixel_at(sx as usize, sy as usize);
            let h = hash2(cx + 11, cy + 29);
            let wear = (h & 3) as f32 * 0.04;
            let is_light = ((cx + cy) & 1) == 0;
            let tile_col = if lx == 0 || ly == 0 {
                grout
            } else if lx == 1 || ly == 1 {
                if is_light { light_bevel } else { dark_bevel }
            } else {
                let base = if is_light { light_tile } else { dark_tile };
                mix(base, grout, wear)
            };
            // High enough alpha (0.86) to cleanly replace the 20 px base grid
            // with zero cross-grid beating while retaining subtle floor warmth.
            pb.plot(sx, sy, mix(cur, tile_col, 0.86));
        }
    }
}

/// Kitchen Boss Arena (4×4 cell = 64×64 px block): enormous scuffed rubber-sole
/// shoe prints (paired stepping impressions), curved heel-skid streaks, and
/// faint broom-bristle sweep arcs.
fn boss_kitchen_block(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    sx: i32,
    sy: i32,
    bx: i32,
    by: i32,
) {
    let h = hash2(bx + 19, by + 53);
    let rubber: Rgb = (9, 6, 6);
    let scuff_wax: Rgb = (96, 82, 64);
    let dust_arc: Rgb = (78, 64, 48);

    // 1. Primary giant scuffed shoe-sole tread impression (~20 × 10 px — huge
    //    compared to the 9 px rat!). Angle varies per 4×4 block.
    let angles = [0.34_f32, -0.46, 1.12, -1.24];
    let ang = angles[((h >> 2) & 3) as usize] + (((h >> 5) & 7) as f32 - 3.5) * 0.05;
    let f = Vec2::from_angle(ang);
    let p = f.perp();
    let shoe_c = Vec2::new(
        (sx + 21 + ((h >> 8) & 15) as i32) as f32,
        (sy + 23 + ((h >> 12) & 15) as i32) as f32,
    );
    draw_giant_shoe_scuff(pb, clip, shoe_c, f, p, rubber, scuff_wax);

    // 2. Curved rubber heel-skid streak showing a stomping pivot!
    let skid_a = Vec2::new((sx + 9) as f32, (sy + 14 + ((h >> 14) & 7) as i32) as f32);
    let skid_b = Vec2::new((sx + 25) as f32, (sy + 7 + ((h >> 17) & 7) as i32) as f32);
    draw_floor_gouge(pb, clip, skid_a, skid_b, 2.4, rubber, 0.74, scuff_wax, 0.40);
    draw_floor_gouge(
        pb,
        clip,
        skid_a + Vec2::new(0.0, 2.2),
        skid_b + Vec2::new(0.0, 2.2),
        2.4,
        rubber,
        0.58,
        scuff_wax,
        0.28,
    );

    // 3. Faint concentric broom-bristle dust sweep arcs across the far corner.
    let arc_c = Vec2::new((sx + 58) as f32, (sy + 56) as f32);
    for ring_i in 0..3 {
        let r = 11.0 + ring_i as f32 * 3.5;
        let steps = 22;
        for i in 0..=steps {
            let a = std::f32::consts::PI + 0.15 + (i as f32 / steps as f32) * 1.25;
            let px = (arc_c.x + a.cos() * r).round() as i32;
            let py = (arc_c.y + a.sin() * r).round() as i32;
            blend_clip(pb, clip, px, py, dust_arc, 0.28);
        }
    }
}

/// Draw one enormous scuffed shoe-sole impression (heel + treaded forefoot +
/// pale wax-scuff rim so it reads cleanly over both light and dark tiles).
fn draw_giant_shoe_scuff(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    center: Vec2,
    f: Vec2,
    p: Vec2,
    rubber: Rgb,
    wax: Rgb,
) {
    let x0 = (center.x - 14.0).floor() as i32;
    let x1 = (center.x + 14.0).ceil() as i32;
    let y0 = (center.y - 14.0).floor() as i32;
    let y1 = (center.y + 14.0).ceil() as i32;

    for y in y0.max(clip.y0)..=y1.min(clip.y1 - 1) {
        for x in x0.max(clip.x0)..=x1.min(clip.x1 - 1) {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let u = dx * f.x + dy * f.y; // -10..+10 (heel at -u, toe at +u)
            let v = dx * p.x + dy * p.y; // -5..+5

            // Heel pad: u in [-9.2, -3.4], |v| <= 3.5
            let d_heel = sd_rbox(u + 6.3, v, 2.9, 3.5, 1.5);
            // Forefoot sole: u in [-0.8, +9.6], wider ball tapering at toe
            let toe_w = 4.5 - ((u - 3.6).max(0.0) * 0.24);
            let d_fore = sd_rbox(u - 4.3, v, 5.4, toe_w.max(1.9), 2.5);

            let d = d_heel.min(d_fore);
            if d > 1.4 {
                continue;
            }
            if d > 0.0 {
                // Dusty scuffed-wax outline halo makes the huge silhouette read
                // unmistakably even across dark checkerboard squares!
                let rim_a = (1.0 - d / 1.4) * 0.36;
                blend_px(pb, x, y, wax, rim_a);
            } else {
                // Crisp alternating dark rubber tread ribs + faint scuffed grooves.
                let on_rib = ((u * 1.15).sin() > -0.1) || d > -0.9;
                if on_rib {
                    blend_px(pb, x, y, rubber, 0.72);
                } else {
                    blend_px(pb, x, y, wax, 0.16);
                }
            }
        }
    }
}

/// Lab Boss Arena continuous surface: alternating 2-cell (32 px) mown turf-bay
/// stripes completing one full light+dark cycle every 4 cells (64 px), offset
/// by half a stripe so each 4×4-cell charging-dock outline sits cleanly
/// centered inside a mown lane!
fn boss_lab_mown_stripes(pb: &mut PixelBuffer, clip: ClipRect, ax0: i32, cam_x: i32) {
    const STRIPE: i32 = 2 * TILE; // 32 px per band -> 64 px (4 cells) full cycle
    let dark_nap: Rgb = (12, 26, 24);
    let light_nap: Rgb = (24, 46, 39);
    let nap_hi: Rgb = (34, 62, 52);
    let rut: Rgb = (6, 10, 14);

    for sy in clip.y0..clip.y1 {
        for sx in clip.x0..clip.x1 {
            // Shift by +16 px so `sx_block + 32` is dead-center of an odd light lane.
            let wx = sx + cam_x - ax0 + 16;
            let band = wx.div_euclid(STRIPE);
            let lx = wx.rem_euclid(STRIPE);
            let cur = pb.pixel_at(sx as usize, sy as usize);

            let out = if lx == 0 || lx == STRIPE - 1 {
                // Mower wheel-track seam between mown lanes.
                mix(cur, rut, 0.56)
            } else if (band & 1) == 0 {
                let grain = if (lx + (sy & 3)) % 6 == 0 { 0.48 } else { 0.56 };
                mix(cur, dark_nap, grain)
            } else {
                let base = mix(cur, light_nap, 0.54);
                if (lx - (sy & 3)).rem_euclid(7) == 0 {
                    mix(base, nap_hi, 0.24)
                } else {
                    base
                }
            };
            pb.plot(sx, sy, out);
        }
    }
}

/// Lab Boss Arena (4×4 cell = 64×64 px block): low-contrast stencilled
/// charging-dock bay outline centered cleanly inside a 32 px mown lane, with
/// corner brackets, recessed contact-pad marks, and subtle guide arrows.
fn boss_lab_block(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    sx: i32,
    sy: i32,
    bx: i32,
    by: i32,
) {
    let line_col: Rgb = (58, 86, 92);
    let bracket_col: Rgb = (114, 100, 50);
    let contact_col: Rgb = (104, 92, 50);
    let pad_dark: Rgb = (8, 12, 18);

    // Charging-dock floor outline centered dead-center inside the 32 px light
    // mown lane (`24 × 24 px`, leaving clean 4 px margins inside the wheel ruts).
    let cx = sx + 32;
    let cy = sy + 32;
    let hw = 12;
    let hh = 12;

    // Subtle recessed floor pad shadow inside the outline.
    rect_blend_clip(
        pb,
        clip,
        cx - hw + 2,
        cy - hh + 2,
        hw * 2 - 4,
        hh * 2 - 4,
        pad_dark,
        0.26,
    );

    // Dashed perimeter outline.
    for dx in -hw..=hw {
        if (dx + hw) % 4 < 2 {
            blend_clip(pb, clip, cx + dx, cy - hh, line_col, 0.44);
            blend_clip(pb, clip, cx + dx, cy + hh, line_col, 0.44);
        }
    }
    for dy in -hh..=hh {
        if (dy + hh) % 4 < 2 {
            blend_clip(pb, clip, cx - hw, cy + dy, line_col, 0.44);
            blend_clip(pb, clip, cx + hw, cy + dy, line_col, 0.44);
        }
    }

    // Solid muted-ochre corner L-brackets (4 px arms) framing the dock bay.
    for &(sx_sign, sy_sign) in &[(-1, -1), (1, -1), (-1, 1), (1, 1)] {
        let kx = cx + sx_sign * hw;
        let ky = cy + sy_sign * hh;
        for step in 0..4 {
            blend_clip(pb, clip, kx - sx_sign * step, ky, bracket_col, 0.50);
            blend_clip(pb, clip, kx, ky - sy_sign * step, bracket_col, 0.50);
        }
    }

    // Two contact-pad outlines along the rear edge (alternating top/bottom).
    let rear_y = if (bx + by) & 1 == 0 {
        cy - hh + 3
    } else {
        cy + hh - 4
    };
    for &pad_x in &[cx - 5, cx + 1] {
        rect_blend_clip(pb, clip, pad_x, rear_y, 4, 2, contact_col, 0.46);
    }

    // Subtle center lightning-bolt / charging stencil glyph.
    for &(dx, dy) in &[
        (1, -3),
        (0, -2),
        (-1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (1, 1),
        (0, 2),
        (-1, 3),
    ] {
        blend_clip(pb, clip, cx + dx, cy + dy, bracket_col, 0.40);
    }

    // Subtle lane-turnaround alignment tick centered in the adjacent dark lane.
    let dark_cx = sx;
    for &(dx, dy) in &[(-2, -1), (-1, 0), (0, 1), (1, 0), (2, -1)] {
        blend_clip(pb, clip, dark_cx + dx, cy + dy, line_col, 0.28);
    }
}

// ============================================================================
// 3. Hall Landmarks (6 Distinct Floor-Layer Variants Per Theme)
// ============================================================================

/// **Floor Layer (no collision)**: Draw a themed hall landmark centered at
/// `world_pos` so major halls are immediately distinguishable at playing scale.
///
/// - `variant`: chooses one of [`HALL_LANDMARK_VARIANTS`] (6) distinct motifs
///   per theme (`variant % 6`). For `Theme::Lab`, the stencilled bay number
///   renders `variant + 1` (`01`..`99`) while cycling the 6 frame/badge styles.
pub fn draw_hall_landmark(
    pb: &mut PixelBuffer,
    world_pos: Vec2,
    variant: u8,
    cam: Vec2,
    theme: Theme,
) {
    if !world_pos.x.is_finite() || !world_pos.y.is_finite() {
        return;
    }
    let (cam_x, cam_y) = cam_snap(cam);
    let cx = world_pos.x.round() as i32 - cam_x;
    let cy = world_pos.y.round() as i32 - cam_y;
    let pad = HALL_LANDMARK_HALF_EXTENT as i32 + 4;
    if cx < -pad || cy < -pad || cx > pb.w as i32 + pad || cy > pb.h as i32 + pad {
        return;
    }
    let clip = ClipRect::from_pb(pb);
    match theme {
        Theme::Sewer => landmark_sewer(pb, clip, cx, cy, variant % HALL_LANDMARK_VARIANTS),
        Theme::Kitchen => landmark_kitchen(pb, clip, cx, cy, variant % HALL_LANDMARK_VARIANTS),
        Theme::Lab => landmark_lab(pb, clip, cx, cy, variant),
    }
}

// ---- Sewer Hall Landmarks: Flush Recessed Drain Grates & Pipe Bends --------

/// Draw a flush recessed cylindrical sub-floor pipe segment along `(a -> b)`
/// inside a dark recessed stone trough (zero outward floor drop shadow).
#[allow(clippy::too_many_arguments)]
fn draw_recessed_pipe_seg(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    a: Vec2,
    b: Vec2,
    radius: f32,
    pipe_sh: Rgb,
    pipe_mid: Rgb,
    pipe_hi: Rgb,
) {
    let delta = b - a;
    let len = delta.len();
    if len < 0.5 {
        return;
    }
    let dir = delta * (1.0 / len);
    let perp = dir.perp();
    let pit: Rgb = (5, 9, 8);
    let rim: Rgb = (42, 62, 54);

    let min_x = (a.x.min(b.x) - radius - 2.5).floor() as i32;
    let max_x = (a.x.max(b.x) + radius + 2.5).ceil() as i32;
    let min_y = (a.y.min(b.y) - radius - 2.5).floor() as i32;
    let max_y = (a.y.max(b.y) + radius + 2.5).ceil() as i32;

    for y in min_y.max(clip.y0)..=max_y.min(clip.y1 - 1) {
        for x in min_x.max(clip.x0)..=max_x.min(clip.x1 - 1) {
            let rx = x as f32 - a.x;
            let ry = y as f32 - a.y;
            let t = (rx * dir.x + ry * dir.y).clamp(0.0, len);
            let qx = a.x + dir.x * t;
            let qy = a.y + dir.y * t;
            let d = ((x as f32 - qx).powi(2) + (y as f32 - qy).powi(2)).sqrt();
            if d > radius + 2.0 {
                continue;
            }
            if d > radius + 0.9 {
                // Flush stone trough lip.
                blend_px(pb, x, y, rim, 0.52);
            } else if d > radius - 0.1 {
                // Dark sub-floor recess surrounding the pipe.
                blend_px(pb, x, y, pit, 0.85);
            } else {
                // Cylindrical shading lit from top-left, kept muted for floor layer.
                let nx = (x as f32 - qx) / radius.max(1.0);
                let ny = (y as f32 - qy) / radius.max(1.0);
                let lit = (0.52 + 0.48 * (nx * LIGHT.x + ny * LIGHT.y)).clamp(0.0, 1.0);
                let col = if lit < 0.5 {
                    mix(pipe_sh, pipe_mid, lit * 2.0)
                } else {
                    mix(pipe_mid, pipe_hi, (lit - 0.5) * 2.0)
                };
                // Subtle seam highlight along pipe crown.
                let cross = ((x as f32 - qx) * perp.x + (y as f32 - qy) * perp.y).abs();
                let col = if cross < 0.85 && lit > 0.58 {
                    mix(col, pipe_hi, 0.40)
                } else {
                    col
                };
                blend_px(pb, x, y, col, 0.84);
            }
        }
    }
}

/// Draw a riveted pipe flange collar across `center` perpendicular to `dir`.
fn draw_pipe_flange(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    center: Vec2,
    dir: Vec2,
    half_w: f32,
    col: Rgb,
    hi: Rgb,
) {
    let p = dir.perp();
    let steps = (half_w * 2.5).ceil() as i32;
    for i in -steps..=steps {
        let s = (i as f32 / steps.max(1) as f32) * half_w;
        for &u in &[-0.6_f32, 0.6] {
            let q = center + dir * u + p * s;
            let c = if u < 0.0 { hi } else { col };
            blend_clip(pb, clip, q.x.round() as i32, q.y.round() as i32, c, 0.82);
        }
    }
}

/// Draw a flush recessed rectangular drain grate with vertical/horizontal/grid bars.
#[allow(clippy::too_many_arguments)]
fn draw_flush_rect_grate(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    bar_mode: u8, // 0 = vertical, 1 = horizontal, 2 = crosshatch, 3 = diagonal diamond
    iron: Rgb,
    iron_hi: Rgb,
) {
    let pit: Rgb = (5, 9, 8);
    let water: Rgb = (14, 30, 27);
    let stone_rim: Rgb = (44, 66, 58);

    // Flush stone curb collar (1 px).
    rect_blend_clip(pb, clip, x0 - 1, y0 - 1, w + 2, 1, stone_rim, 0.62);
    rect_blend_clip(pb, clip, x0 - 1, y0 + h, w + 2, 1, pit, 0.62);
    rect_blend_clip(pb, clip, x0 - 1, y0, 1, h, stone_rim, 0.56);
    rect_blend_clip(pb, clip, x0 + w, y0, 1, h, pit, 0.56);

    // Recessed pit + subtle stagnant water sheen at bottom of sump.
    rect_blend_clip(pb, clip, x0, y0, w, h, pit, 0.88);
    rect_blend_clip(pb, clip, x0 + 2, y0 + 2, (w - 4).max(0), (h - 4).max(0), water, 0.35);

    // Outer cast-iron frame + recessed bars.
    for dy in 0..h {
        for dx in 0..w {
            let border = dx == 0 || dy == 0 || dx == w - 1 || dy == h - 1;
            let bar = match bar_mode {
                0 => dx % 3 == 0,
                1 => dy % 3 == 0,
                2 => dx % 4 == 0 || dy % 4 == 0,
                _ => (dx + dy) % 4 == 0 || (dx - dy).rem_euclid(4) == 0,
            };
            if border || bar {
                let top_left = dx == 0 || dy == 0 || (bar && (dx + dy) % 5 == 0);
                let c = if top_left { iron_hi } else { iron };
                blend_clip(pb, clip, x0 + dx, y0 + dy, c, 0.86);
            }
        }
    }
}

/// Draw a flush circular spoked sewer manhole drain grate.
#[allow(clippy::too_many_arguments)]
fn draw_flush_round_grate(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    cx: i32,
    cy: i32,
    r: i32,
    iron: Rgb,
    iron_hi: Rgb,
    rust: Rgb,
) {
    let pit: Rgb = (5, 9, 8);
    let stone_rim: Rgb = (44, 66, 58);
    let rf = r as f32;
    for dy in -(r + 1)..=(r + 1) {
        for dx in -(r + 1)..=(r + 1) {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d > rf + 1.1 {
                continue;
            }
            let x = cx + dx;
            let y = cy + dy;
            if d > rf {
                let c = if dx + dy < 0 { stone_rim } else { pit };
                blend_clip(pb, clip, x, y, c, 0.62);
            } else if d > rf - 1.5 {
                // Cast-iron rim with occasional rust rivets.
                let rivet = (dx.abs() == 0 || dy.abs() == 0) && d > rf - 1.2;
                let c = if rivet {
                    rust
                } else if dx + dy < 0 {
                    iron_hi
                } else {
                    iron
                };
                blend_clip(pb, clip, x, y, c, 0.86);
            } else {
                // Recessed pit with 4 radial spokes + inner hub ring.
                let on_spoke = dx.abs() <= 0 || dy.abs() <= 0 || (dx.abs() - dy.abs()).abs() <= 0;
                let on_hub = d < 2.3;
                if on_hub || on_spoke {
                    let c = if dx + dy < 0 { iron_hi } else { iron };
                    blend_clip(pb, clip, x, y, c, 0.82);
                } else {
                    blend_clip(pb, clip, x, y, pit, 0.88);
                }
            }
        }
    }
}

fn landmark_sewer(pb: &mut PixelBuffer, clip: ClipRect, cx: i32, cy: i32, variant: u8) {
    let iron: Rgb = (46, 62, 58);
    let iron_hi: Rgb = (78, 98, 92);
    let pipe_sh: Rgb = (22, 35, 33);
    let pipe_mid: Rgb = (44, 66, 60);
    let pipe_hi: Rgb = (74, 100, 92);
    let copper_sh: Rgb = (24, 44, 39);
    let copper_mid: Rgb = (44, 82, 70);
    let copper_hi: Rgb = (74, 120, 104);
    let rust: Rgb = (88, 58, 38);
    let rust_hi: Rgb = (122, 82, 52);
    let moss: Rgb = (36, 56, 34);
    let pit: Rgb = (5, 9, 8);
    let stone_lip: Rgb = (44, 64, 56);
    let c = Vec2::new(cx as f32, cy as f32);

    // Draw a flush stone floor-pocket collar directly capping where a sub-floor
    // pipe dives under the pavement so it never shows a floating pill endcap.
    let pipe_socket = |pb: &mut PixelBuffer, end: Vec2, dir: Vec2| {
        let p = dir.perp();
        for s in -4..=4 {
            let sf = s as f32 * 0.85;
            let hole = end + p * (sf * 0.85) + dir * 0.2;
            let hole2 = end + p * (sf * 0.85) + dir * 1.1;
            let rim = end + p * sf + dir * 2.1;
            blend_clip(pb, clip, hole.x.round() as i32, hole.y.round() as i32, pit, 0.90);
            blend_clip(pb, clip, hole2.x.round() as i32, hole2.y.round() as i32, pit, 0.86);
            blend_clip(pb, clip, rim.x.round() as i32, rim.y.round() as i32, stone_lip, 0.78);
        }
    };

    // Draw a rounded cast-iron/copper elbow sleeve at pipe bends so angled
    // segments join smoothly with zero corner step.
    let pipe_elbow = |pb: &mut PixelBuffer, joint: Vec2, r: f32, sh: Rgb, mid: Rgb, hi: Rgb| {
        let ri = (r + 1.0).ceil() as i32;
        let jx = joint.x.round() as i32;
        let jy = joint.y.round() as i32;
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= r + 0.3 {
                    let lit = (0.52 + 0.48 * ((dx as f32 * LIGHT.x + dy as f32 * LIGHT.y) / r.max(1.0))).clamp(0.0, 1.0);
                    let col = if lit < 0.5 {
                        mix(sh, mid, lit * 2.0)
                    } else {
                        mix(mid, hi, (lit - 0.5) * 2.0)
                    };
                    blend_clip(pb, clip, jx + dx, jy + dy, col, 0.88);
                }
            }
        }
    };

    match variant {
        0 => {
            // Variant 0: Square vertical-bar drain grate (left) + NE riveted
            // iron pipe elbow bending up-right into a floor socket.
            let p0 = c + Vec2::new(-2.0, 4.0);
            let p1 = c + Vec2::new(11.0, 4.0);
            let p2 = c + Vec2::new(11.0, -10.0);
            draw_recessed_pipe_seg(pb, clip, p0, p1, 3.2, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p1, p2, 3.2, pipe_sh, pipe_mid, pipe_hi);
            pipe_elbow(pb, p1, 3.3, pipe_sh, pipe_mid, pipe_hi);
            draw_pipe_flange(pb, clip, c + Vec2::new(4.0, 4.0), Vec2::new(1.0, 0.0), 4.0, iron, iron_hi);
            draw_pipe_flange(pb, clip, c + Vec2::new(11.0, -4.0), Vec2::new(0.0, 1.0), 4.0, iron, iron_hi);
            pipe_socket(pb, p2, Vec2::new(0.0, -1.0));
            draw_flush_rect_grate(pb, clip, cx - 17, cy - 5, 14, 13, 0, iron, iron_hi);
            rect_blend_clip(pb, clip, cx - 14, cy + 9, 10, 2, (26, 54, 48), 0.36);
        }
        1 => {
            // Variant 1: Circular spoked manhole grate + NW rust-flanged elbow.
            let p0 = c + Vec2::new(14.0, 6.0);
            let p1 = c + Vec2::new(4.0, 6.0);
            let p2 = c + Vec2::new(-4.0, -9.0);
            draw_recessed_pipe_seg(pb, clip, p0, p1, 3.0, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p1, p2, 3.0, pipe_sh, pipe_mid, pipe_hi);
            pipe_elbow(pb, p1, 3.2, pipe_sh, pipe_mid, pipe_hi);
            draw_pipe_flange(pb, clip, p1, Vec2::new(0.7, 0.7), 3.8, rust, rust_hi);
            pipe_socket(pb, p0, Vec2::new(1.0, 0.0));
            pipe_socket(pb, p2, Vec2::new(-0.47, -0.88));
            draw_flush_round_grate(pb, clip, cx - 10, cy + 3, 8, iron, iron_hi, rust_hi);
        }
        2 => {
            // Variant 2: Wide horizontal trench grate (bottom) + SE verdigris-copper
            // pipe elbow above it.
            let p0 = c + Vec2::new(-14.0, -8.0);
            let p1 = c + Vec2::new(7.0, -8.0);
            let p2 = c + Vec2::new(14.0, 3.0);
            draw_recessed_pipe_seg(pb, clip, p0, p1, 3.0, copper_sh, copper_mid, copper_hi);
            draw_recessed_pipe_seg(pb, clip, p1, p2, 3.0, copper_sh, copper_mid, copper_hi);
            pipe_elbow(pb, p1, 3.2, copper_sh, copper_mid, copper_hi);
            draw_pipe_flange(pb, clip, c + Vec2::new(-4.0, -8.0), Vec2::new(1.0, 0.0), 3.8, copper_mid, copper_hi);
            pipe_socket(pb, p0, Vec2::new(-1.0, 0.0));
            pipe_socket(pb, p2, Vec2::new(0.54, 0.84));
            draw_flush_rect_grate(pb, clip, cx - 14, cy + 2, 24, 9, 0, iron, iron_hi);
        }
        3 => {
            // Variant 3: Twin small square grates + recessed iron U-bend below.
            let p0 = c + Vec2::new(-11.0, -2.0);
            let p1 = c + Vec2::new(-11.0, 10.0);
            let p2 = c + Vec2::new(11.0, 10.0);
            let p3 = c + Vec2::new(11.0, -2.0);
            draw_recessed_pipe_seg(pb, clip, p0, p1, 2.8, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p1, p2, 2.8, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p2, p3, 2.8, pipe_sh, pipe_mid, pipe_hi);
            draw_pipe_flange(pb, clip, c + Vec2::new(0.0, 10.0), Vec2::new(1.0, 0.0), 3.6, rust, rust_hi);
            draw_flush_rect_grate(pb, clip, cx - 16, cy - 11, 11, 10, 1, iron, iron_hi);
            draw_flush_rect_grate(pb, clip, cx + 5, cy - 11, 11, 10, 1, iron, iron_hi);
            rect_blend_clip(pb, clip, cx - 3, cy - 9, 6, 4, moss, 0.40);
        }
        4 => {
            // Variant 4: Central crosshatch sump grate + S-curve dogleg pipe.
            let p0 = c + Vec2::new(-17.0, -9.0);
            let p1 = c + Vec2::new(-5.0, -9.0);
            let p2 = c + Vec2::new(5.0, 9.0);
            let p3 = c + Vec2::new(17.0, 9.0);
            draw_recessed_pipe_seg(pb, clip, p0, p1, 2.8, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p1, p2, 2.8, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p2, p3, 2.8, pipe_sh, pipe_mid, pipe_hi);
            pipe_socket(pb, p0, Vec2::new(-1.0, 0.0));
            pipe_socket(pb, p3, Vec2::new(1.0, 0.0));
            draw_flush_rect_grate(pb, clip, cx - 8, cy - 7, 16, 14, 2, iron, iron_hi);
            rect_blend_clip(pb, clip, cx - 16, cy + 4, 7, 3, (36, 48, 62), 0.36);
        }
        _ => {
            // Variant 5: Diamond-mesh grate (right) + T-junction pipe bend with
            // flush brass inspection cap (left).
            let p0 = c + Vec2::new(-14.0, 0.0);
            let p1 = c + Vec2::new(0.0, 0.0);
            let p2 = c + Vec2::new(-8.0, -10.0);
            let p3 = c + Vec2::new(-8.0, 10.0);
            draw_recessed_pipe_seg(pb, clip, p0, p1, 3.0, pipe_sh, pipe_mid, pipe_hi);
            draw_recessed_pipe_seg(pb, clip, p2, p3, 3.0, pipe_sh, pipe_mid, pipe_hi);
            pipe_socket(pb, p0, Vec2::new(-1.0, 0.0));
            pipe_socket(pb, p2, Vec2::new(0.0, -1.0));
            pipe_socket(pb, p3, Vec2::new(0.0, 1.0));
            // Flush brass inspection plate at the T-junction center.
            rect_blend_clip(pb, clip, cx - 11, cy - 3, 6, 6, (94, 76, 40), 0.82);
            rect_blend_clip(pb, clip, cx - 10, cy - 2, 4, 2, (128, 106, 56), 0.74);
            draw_flush_rect_grate(pb, clip, cx + 2, cy - 8, 14, 16, 3, iron, iron_hi);
        }
    }
}

// ---- Kitchen Hall Landmarks: Flat Counter-End Floor Mats & Bread Crusts ----

/// Draw a rat-scale fallen toasted bread crust heel (`~11 × 7 px`, strictly
/// floor debris — warm crust + pale crumb interior + tiny crumbs, no glow).
#[allow(clippy::too_many_arguments)]
fn draw_bread_crust(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    center: Vec2,
    ang: f32,
    style: u8, // 0 = loaf heel, 1 = bitten crescent, 2 = baguette end, 3 = toast half
) {
    let f = Vec2::from_angle(ang);
    let p = f.perp();
    let crust_dk: Rgb = (74, 42, 20);
    let crust_mid: Rgb = (132, 82, 38);
    let crust_hi: Rgb = (176, 122, 62);
    let crumb_sh: Rgb = (138, 118, 86);
    let crumb_hi: Rgb = (182, 162, 126);

    let x0 = (center.x - 8.0).floor() as i32;
    let x1 = (center.x + 8.0).ceil() as i32;
    let y0 = (center.y - 8.0).floor() as i32;
    let y1 = (center.y + 8.0).ceil() as i32;

    for y in y0.max(clip.y0)..=y1.min(clip.y1 - 1) {
        for x in x0.max(clip.x0)..=x1.min(clip.x1 - 1) {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;

            let (d, is_crust) = match style {
                0 => {
                    // Classic rounded loaf-end heel: flat cut on -v, domed crust on +v.
                    let dome = ((u / 5.2).powi(2) + ((v + 0.5) / 3.6).powi(2)).sqrt() - 1.0;
                    let flat = -v - 2.3;
                    let d = dome.max(flat);
                    (d, dome > -0.34 || v > 1.3)
                }
                1 => {
                    // Crescent sourdough crust with a rat-bite notch taken out!
                    let outer = ((u / 5.4).powi(2) + (v / 3.8).powi(2)).sqrt() - 1.0;
                    let bite = ((u - 1.5).powi(2) + (v + 2.6).powi(2)).sqrt() - 2.6;
                    let d = outer.max(-bite);
                    (d, outer > -0.32)
                }
                2 => {
                    // Long crusty baguette heel with diagonal slash scores.
                    let d = sd_rbox(u, v, 6.0, 2.3, 1.9);
                    let score = ((u * 0.8 + v * 0.5).sin()).abs() > 0.85;
                    (d, d > -1.0 || u > -2.5 || score)
                }
                _ => {
                    // Crisp halved triangular toast slice with crust along both
                    // outer straight edges and pale crumb along the diagonal cut.
                    let d = (u - 3.6)
                        .max(-u - 3.4)
                        .max(v - 3.3)
                        .max(-v - 3.3)
                        .max((u + v) * 0.72 - 1.5);
                    (d, u < -2.1 || v < -2.0)
                }
            };

            if d > 0.85 {
                continue;
            }
            if d > 0.0 {
                // Soft 1 px floor grounding shadow on bottom-right side.
                if dx + dy > 0.0 {
                    darken_px(pb, x, y, 0.62);
                }
            } else {
                let lit = (0.55 + 0.45 * ((dx * LIGHT.x + dy * LIGHT.y) / 5.0)).clamp(0.0, 1.0);
                let col = if is_crust {
                    if lit < 0.45 {
                        mix(crust_dk, crust_mid, lit / 0.45)
                    } else {
                        mix(crust_mid, crust_hi, (lit - 0.45) / 0.55)
                    }
                } else {
                    mix(crumb_sh, crumb_hi, lit)
                };
                blend_px(pb, x, y, col, 0.88);
            }
        }
    }

    // Scatter 4 tiny bread crumbs around the crust.
    for &(cu, cv) in &[(4.8, -2.5), (-4.6, 2.2), (2.2, 3.6), (-1.8, -3.5)] {
        let q = center + f * cu + p * cv;
        blend_clip(pb, clip, q.x.round() as i32, q.y.round() as i32, crumb_hi, 0.68);
    }
}

fn landmark_kitchen(pb: &mut PixelBuffer, clip: ClipRect, cx: i32, cy: i32, variant: u8) {
    let mat_dark: Rgb = (17, 13, 12);
    let mat_rib: Rgb = (50, 41, 36);
    let mat_edge: Rgb = (68, 56, 46);
    let brass: Rgb = (118, 94, 52);
    let brass_hi: Rgb = (152, 124, 72);
    let terra: Rgb = (78, 44, 32);
    let terra_hi: Rgb = (112, 68, 48);
    let wood_dk: Rgb = (44, 28, 18);
    let wood_lt: Rgb = (82, 56, 34);
    let wood_hi: Rgb = (104, 72, 44);
    let steel: Rgb = (58, 62, 66);
    let steel_hi: Rgb = (92, 96, 102);
    let c = Vec2::new(cx as f32, cy as f32);

    match variant {
        0 => {
            // Variant 0: Ribbed rubber counter-end prep mat with beveled rim +
            // curved rye bread heel + muted flour dusting.
            rect_blend_clip(pb, clip, cx - 19, cy - 10, 18, 20, mat_dark, 0.78);
            rect_blend_clip(pb, clip, cx - 19, cy - 10, 18, 1, mat_edge, 0.72);
            rect_blend_clip(pb, clip, cx - 19, cy - 10, 1, 20, mat_edge, 0.68);
            rect_blend_clip(pb, clip, cx - 19, cy + 9, 18, 1, (10, 7, 6), 0.72);
            for ry in (cy - 7..cy + 8).step_by(3) {
                rect_blend_clip(pb, clip, cx - 17, ry, 14, 1, mat_rib, 0.78);
            }
            // Brass edge trim on the counter-end toe-kick edge.
            rect_blend_clip(pb, clip, cx - 2, cy - 10, 2, 20, brass, 0.66);
            rect_blend_clip(pb, clip, cx - 2, cy - 10, 1, 20, brass_hi, 0.48);
            // Soft flour dust smear under the crust.
            for dy in -5..=5 {
                for dx in -6..=6 {
                    let e = (dx as f32 / 6.0).powi(2) + (dy as f32 / 5.0).powi(2);
                    if e < 1.0 && hash2(cx + dx, cy + dy) & 3 != 0 {
                        blend_clip(pb, clip, cx + 8 + dx, cy + 2 + dy, (136, 126, 108), 0.28 * (1.0 - e));
                    }
                }
            }
            draw_bread_crust(pb, clip, c + Vec2::new(9.0, 0.0), 0.25, 0);
        }
        1 => {
            // Variant 1: Inlaid terracotta counter-bay tile apron (top-left L
            // with tile mortar joints) + bitten sourdough crescent + wax paper.
            rect_blend_clip(pb, clip, cx - 18, cy - 11, 26, 7, terra, 0.70);
            rect_blend_clip(pb, clip, cx - 18, cy - 4, 8, 15, terra, 0.70);
            rect_blend_clip(pb, clip, cx - 18, cy - 11, 26, 1, terra_hi, 0.64);
            rect_blend_clip(pb, clip, cx - 18, cy - 11, 1, 22, terra_hi, 0.64);
            // Inlaid tile grout joints along the terracotta L-border.
            for gx in [-10, -2, 6] {
                rect_blend_clip(pb, clip, cx + gx, cy - 11, 1, 7, mat_dark, 0.68);
            }
            for gy in [-4, 4] {
                rect_blend_clip(pb, clip, cx - 18, cy + gy, 8, 1, mat_dark, 0.68);
            }
            // Warm parchment wax-paper corner on floor.
            rect_blend_clip(pb, clip, cx - 5, cy + 1, 10, 8, (132, 118, 92), 0.42);
            rect_blend_clip(pb, clip, cx - 4, cy + 2, 8, 6, (156, 140, 110), 0.36);
            draw_bread_crust(pb, clip, c + Vec2::new(6.0, 2.0), -0.6, 1);
        }
        2 => {
            // Variant 2: Brass-trimmed counter-end threshold strip (top) +
            // torn baguette crust + muted plum-jam floor smear.
            rect_blend_clip(pb, clip, cx - 18, cy - 11, 36, 7, (38, 30, 26), 0.74);
            rect_blend_clip(pb, clip, cx - 18, cy - 11, 36, 1, mat_edge, 0.62);
            rect_blend_clip(pb, clip, cx - 18, cy - 4, 36, 2, brass, 0.74);
            rect_blend_clip(pb, clip, cx - 18, cy - 4, 36, 1, brass_hi, 0.52);
            // Small brass countersunk screw heads along strip.
            for sx in [-14, -4, 6, 15] {
                blend_clip(pb, clip, cx + sx, cy - 8, brass_hi, 0.66);
            }
            // Muted dark berry-jam smear beside crust.
            for dy in -4..=4 {
                for dx in -6..=6 {
                    let e = (dx as f32 / 6.0).powi(2) + (dy as f32 / 4.0).powi(2);
                    if e < 1.0 {
                        blend_clip(pb, clip, cx - 8 + dx, cy + 5 + dy, (88, 28, 38), 0.58 * (1.0 - e * 0.45));
                    }
                }
            }
            draw_bread_crust(pb, clip, c + Vec2::new(6.0, 4.0), -0.35, 2);
        }
        3 => {
            // Variant 3: Perforated chef's drainage floor mat with beveled edge
            // + pair of halved toast crusts + coffee-ring stain.
            rect_blend_clip(pb, clip, cx - 18, cy - 10, 25, 19, mat_dark, 0.78);
            rect_blend_clip(pb, clip, cx - 18, cy - 10, 25, 1, mat_edge, 0.70);
            rect_blend_clip(pb, clip, cx - 18, cy - 10, 1, 19, mat_edge, 0.68);
            rect_blend_clip(pb, clip, cx - 18, cy + 8, 25, 1, (10, 7, 6), 0.70);
            for hy in (cy - 7..cy + 6).step_by(4) {
                for hx in (cx - 15..cx + 5).step_by(4) {
                    rect_blend_clip(pb, clip, hx, hy, 2, 2, (52, 42, 36), 0.64);
                }
            }
            // Coffee ring stain at right.
            for i in 0..24 {
                let a = i as f32 / 24.0 * std::f32::consts::TAU;
                let rx = cx + 13 + (a.cos() * 4.4).round() as i32;
                let ry = cy - 4 + (a.sin() * 4.4).round() as i32;
                blend_clip(pb, clip, rx, ry, (72, 42, 22), 0.68);
            }
            draw_bread_crust(pb, clip, c + Vec2::new(-5.0, 1.0), 0.35, 3);
            draw_bread_crust(pb, clip, c + Vec2::new(10.0, 6.0), -2.45, 3);
        }
        4 => {
            // Variant 4: Slatted wooden duckboard floor mat (bottom) + loaf heel
            // + warm paprika/spice dust scatter.
            for sy_i in 0..4 {
                let y = cy + 1 + sy_i * 3;
                rect_blend_clip(pb, clip, cx - 18, y, 35, 1, wood_hi, 0.64);
                rect_blend_clip(pb, clip, cx - 18, y + 1, 35, 1, wood_lt, 0.74);
                rect_blend_clip(pb, clip, cx - 18, y + 2, 35, 1, wood_dk, 0.70);
            }
            // Cross-battens & brass pins under slats.
            rect_blend_clip(pb, clip, cx - 13, cy + 1, 2, 11, wood_dk, 0.68);
            rect_blend_clip(pb, clip, cx + 10, cy + 1, 2, 11, wood_dk, 0.68);
            // Warm paprika spice scatter.
            for i in 0..10 {
                let h = hash2(cx + i * 7, cy - i * 5);
                let px = cx + 6 + (h % 11) as i32 - 5;
                let py = cy - 6 + ((h >> 4) % 7) as i32 - 3;
                blend_clip(pb, clip, px, py, (132, 54, 28), 0.52);
            }
            draw_bread_crust(pb, clip, c + Vec2::new(-6.0, -5.0), 2.8, 0);
        }
        _ => {
            // Variant 5: Flush brushed-steel counter-end kick-plate & recessed
            // floor drip-strip (left) + rat-bitten crust + muted cream puddle.
            rect_blend_clip(pb, clip, cx - 19, cy - 11, 10, 22, steel, 0.72);
            rect_blend_clip(pb, clip, cx - 18, cy - 10, 3, 20, steel_hi, 0.46);
            rect_blend_clip(pb, clip, cx - 13, cy - 9, 2, 18, mat_dark, 0.66); // drip slot
            rect_blend_clip(pb, clip, cx - 9, cy - 11, 1, 22, mat_dark, 0.70);
            // Corner bolt heads on the steel floor plate.
            for &(bx, by) in &[(-17, -9), (-11, -9), (-17, 8), (-11, 8)] {
                blend_clip(pb, clip, cx + bx, cy + by, (118, 122, 128), 0.72);
            }
            // Low-contrast warm cream puddle.
            for dy in -5..=5 {
                for dx in -7..=7 {
                    let e = (dx as f32 / 7.0).powi(2) + (dy as f32 / 5.0).powi(2);
                    if e < 1.0 {
                        blend_clip(pb, clip, cx + 4 + dx, cy + 4 + dy, (124, 114, 98), 0.38 * (1.0 - e * 0.45));
                    }
                }
            }
            draw_bread_crust(pb, clip, c + Vec2::new(6.0, -4.0), -0.9, 1);
        }
    }
}

// ---- Lab Hall Landmarks: Stencilled Industrial Bay Markings (01..99) -------

/// Crisp 5×7 industrial stencil bitmaps (`0..=9`) with authentic 1 px stencil
/// bridge breaks so digits look spray-painted directly onto the Lab floor.
const STENCIL_DIGITS: [[u8; 7]; 10] = [
    // 0 (crisp industrial oval with single top-left stencil bridge)
    [0b01110, 0b10001, 0b10001, 0b00001, 0b10001, 0b10001, 0b01110],
    // 1
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    // 2
    [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
    // 3
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    // 4 (bold crossbar)
    [0b10010, 0b10010, 0b10010, 0b11111, 0b00010, 0b00010, 0b00010],
    // 5
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    // 6
    [0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    // 7
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    // 8
    [0b01110, 0b10001, 0b10001, 0b01010, 0b10001, 0b10001, 0b01110],
    // 9
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// Paint one 5×7 stencil digit with subtle floor-wear scuffing.
#[allow(clippy::too_many_arguments)]
fn draw_stencil_digit(
    pb: &mut PixelBuffer,
    clip: ClipRect,
    x0: i32,
    y0: i32,
    digit: u8,
    col: Rgb,
    alpha: f32,
    wear_seed: i32,
) {
    let rows = STENCIL_DIGITS[(digit % 10) as usize];
    for (ry, &bits) in rows.iter().enumerate() {
        for rx in 0..5 {
            if (bits >> (4 - rx)) & 1 != 0 {
                // Subtle paint-wear chip on ~1 in 16 pixels so it feels painted
                // onto worn deck plates rather than floating HUD text.
                let w = hash2(x0 + rx + wear_seed, y0 + ry as i32);
                let a = if (w & 15) == 0 { alpha * 0.45 } else { alpha };
                blend_clip(pb, clip, x0 + rx, y0 + ry as i32, col, a);
            }
        }
    }
}

fn landmark_lab(pb: &mut PixelBuffer, clip: ClipRect, cx: i32, cy: i32, variant: u8) {
    let style = variant % HALL_LANDMARK_VARIANTS;
    let bay_num = ((variant as u16) % 99) + 1; // 01..99
    let tens = (bay_num / 10) as u8;
    let ones = (bay_num % 10) as u8;

    // 6 distinct muted sector-paint colors (low-to-mid contrast over dark navy).
    let (accent, trim): (Rgb, Rgb) = match style {
        0 => ((74, 146, 166), (52, 86, 108)),   // Bay 01: Cyan-slate
        1 => ((156, 122, 54), (96, 80, 44)),    // Bay 02: Industrial amber
        2 => ((70, 142, 102), (46, 86, 68)),    // Bay 03: Bio-sage
        3 => ((112, 114, 174), (66, 70, 112)),  // Bay 04: Steel-indigo
        4 => ((156, 82, 68), (94, 54, 48)),     // Bay 05: Rust-coral
        _ => ((130, 146, 162), (76, 88, 104)),  // Bay 06: Cool silver
    };
    let stencil_ink: Rgb = mix((148, 160, 178), accent, 0.38);
    let pad_dark: Rgb = (7, 9, 17);

    // Matte dark floor-coating pad (`38 × 22 px`) cleanly grounds the stencil
    // over Lab deck seams or decorative chevrons.
    rect_blend_clip(pb, clip, cx - 19, cy - 11, 38, 22, pad_dark, 0.54);

    // Distinct frame geometry & sector badge per variant (0..6).
    match style {
        0 => {
            // Solid corner L-brackets + diagonal top-left hatch + reticle badge.
            for &(sx, sy) in &[(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let kx = cx + sx * 19;
                let ky = cy + sy * 11;
                for s in 0..6 {
                    blend_clip(pb, clip, kx - sx * s, ky, accent, 0.56);
                    blend_clip(pb, clip, kx, ky - sy * s, accent, 0.56);
                }
            }
            for i in 0..4 {
                blend_clip(pb, clip, cx - 17 + i * 2, cy - 8, trim, 0.52);
                blend_clip(pb, clip, cx - 16 + i * 2, cy - 9, trim, 0.52);
            }
            // Right-side circle-cross reticle badge.
            for &(dx, dy) in &[
                (0, -3), (1, -3), (2, -2), (3, -1), (3, 0), (2, 1), (1, 2), (0, 2),
                (-1, 2), (-2, 1), (-3, 0), (-3, -1), (-2, -2), (-1, -3),
                (0, -1), (0, 0), (-1, -1), (1, -1),
            ] {
                blend_clip(pb, clip, cx + 11 + dx, cy + dy, accent, 0.54);
            }
        }
        1 => {
            // Double top/bottom horizontal rails + 2×2 test-grid square badge.
            rect_blend_clip(pb, clip, cx - 19, cy - 11, 38, 1, accent, 0.56);
            rect_blend_clip(pb, clip, cx - 19, cy - 9, 14, 1, trim, 0.48);
            rect_blend_clip(pb, clip, cx - 19, cy + 10, 38, 1, accent, 0.56);
            rect_blend_clip(pb, clip, cx + 5, cy + 8, 14, 1, trim, 0.48);
            // 2×2 grid badge on right.
            rect_blend_clip(pb, clip, cx + 8, cy - 4, 3, 3, accent, 0.54);
            rect_blend_clip(pb, clip, cx + 12, cy - 4, 3, 3, trim, 0.52);
            rect_blend_clip(pb, clip, cx + 8, cy, 3, 3, trim, 0.52);
            rect_blend_clip(pb, clip, cx + 12, cy, 3, 3, accent, 0.54);
        }
        2 => {
            // Dashed outer perimeter box + vertical alignment bars + hollow triangle badge.
            for dx in -19..=19 {
                if (dx + 19) % 4 < 2 {
                    blend_clip(pb, clip, cx + dx, cy - 11, accent, 0.54);
                    blend_clip(pb, clip, cx + dx, cy + 11, accent, 0.54);
                }
            }
            for dy in -11..=11 {
                if (dy + 11) % 4 < 2 {
                    blend_clip(pb, clip, cx - 19, cy + dy, accent, 0.54);
                    blend_clip(pb, clip, cx + 19, cy + dy, accent, 0.54);
                }
            }
            rect_blend_clip(pb, clip, cx - 16, cy - 7, 1, 14, trim, 0.54);
            // Hollow triangle badge on right.
            for &(dx, dy) in &[
                (0, -3), (-1, -2), (1, -2), (-2, -1), (2, -1), (-3, 0), (3, 0),
                (-3, 1), (3, 1), (-2, 2), (-1, 2), (0, 2), (1, 2), (2, 2), (0, 0),
            ] {
                blend_clip(pb, clip, cx + 11 + dx, cy + dy, accent, 0.56);
            }
        }
        3 => {
            // Chamfered octagonal border + bottom ruler ticks + diamond badge.
            rect_blend_clip(pb, clip, cx - 15, cy - 11, 30, 1, accent, 0.54);
            rect_blend_clip(pb, clip, cx - 15, cy + 11, 30, 1, accent, 0.54);
            rect_blend_clip(pb, clip, cx - 19, cy - 7, 1, 14, accent, 0.54);
            rect_blend_clip(pb, clip, cx + 19, cy - 7, 1, 14, accent, 0.54);
            for s in 1..4 {
                blend_clip(pb, clip, cx - 19 + s, cy - 7 - s, accent, 0.54);
                blend_clip(pb, clip, cx + 19 - s, cy - 7 - s, accent, 0.54);
                blend_clip(pb, clip, cx - 19 + s, cy + 7 + s, accent, 0.54);
                blend_clip(pb, clip, cx + 19 - s, cy + 7 + s, accent, 0.54);
            }
            for tx in (cx - 12..=cx + 12).step_by(4) {
                rect_blend_clip(pb, clip, tx, cy + 8, 1, 2, trim, 0.54);
            }
            // Diamond sensor badge on right.
            for &(dx, dy) in &[
                (0, -3), (-1, -2), (1, -2), (-2, -1), (2, -1), (-3, 0), (3, 0),
                (-2, 1), (2, 1), (-1, 2), (1, 2), (0, 3), (0, 0),
            ] {
                blend_clip(pb, clip, cx + 11 + dx, cy + dy, accent, 0.56);
            }
        }
        4 => {
            // Heavy side vertical brackets + angled corner hazard ticks + concentric ring badge.
            rect_blend_clip(pb, clip, cx - 19, cy - 10, 2, 20, accent, 0.56);
            rect_blend_clip(pb, clip, cx + 18, cy - 10, 2, 20, accent, 0.56);
            for i in 0..3 {
                blend_clip(pb, clip, cx - 15 + i * 3, cy - 10, trim, 0.54);
                blend_clip(pb, clip, cx - 14 + i * 3, cy - 9, trim, 0.54);
                blend_clip(pb, clip, cx + 14 - i * 3, cy + 10, trim, 0.54);
                blend_clip(pb, clip, cx + 13 - i * 3, cy + 9, trim, 0.54);
            }
            // Concentric turbine ring badge on right.
            for dy in -3..=3 {
                for dx in -3..=3 {
                    let r2 = dx * dx + dy * dy;
                    if (7..=11).contains(&r2) || r2 <= 1 {
                        blend_clip(pb, clip, cx + 11 + dx, cy + dy, accent, 0.56);
                    }
                }
            }
        }
        _ => {
            // Solid hairline frame + center divider + hexagonal core badge.
            rect_blend_clip(pb, clip, cx - 19, cy - 11, 38, 1, trim, 0.54);
            rect_blend_clip(pb, clip, cx - 19, cy + 10, 38, 1, trim, 0.54);
            rect_blend_clip(pb, clip, cx - 19, cy - 10, 1, 20, trim, 0.54);
            rect_blend_clip(pb, clip, cx + 18, cy - 10, 1, 20, trim, 0.54);
            rect_blend_clip(pb, clip, cx + 5, cy - 8, 1, 16, accent, 0.44);
            // Hexagonal badge on right.
            for &(dx, dy) in &[
                (-1, -3), (0, -3), (1, -3), (-2, -2), (2, -2), (-3, -1), (3, -1),
                (-3, 0), (3, 0), (-2, 1), (2, 1), (-1, 2), (0, 2), (1, 2),
            ] {
                blend_clip(pb, clip, cx + 11 + dx, cy + dy, accent, 0.58);
            }
        }
    }

    // Tiny painted "BAY" header micro-bar above the numeral (`y = cy - 7`).
    rect_blend_clip(pb, clip, cx - 13, cy - 7, 13, 1, trim, 0.50);

    // Bold 2-digit stencilled bay number (`01`..`06`...) in left-center zone,
    // with a subtle 1 px dark bevel offset so it sits etched into the floor coat.
    draw_stencil_digit(pb, clip, cx - 12, cy - 3, tens, pad_dark, 0.35, variant as i32);
    draw_stencil_digit(pb, clip, cx - 5, cy - 3, ones, pad_dark, 0.35, variant as i32 + 7);
    draw_stencil_digit(pb, clip, cx - 13, cy - 4, tens, stencil_ink, 0.68, variant as i32);
    draw_stencil_digit(pb, clip, cx - 6, cy - 4, ones, stencil_ink, 0.68, variant as i32 + 7);
}

// ============================================================================
// Tests & Visual Reference Sheet
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor::draw_floor;
    use crate::game::art_rat::draw_rat;
    use crate::game::art_traps::{draw_maze_reward, draw_maze_reward_ground, MazeRewardView};
    use crate::game::art_walls::{draw_wall_cell, draw_wall_ground, NeighborMask, WallCellView, WallKind};
    use crate::game::level::{Level, Theme};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;

    #[test]
    fn marks_draw_safely_everywhere_without_panic() {
        let mut pb = PixelBuffer::new(48, 36);
        let positions = [
            Vec2::new(24.0, 18.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(47.5, 35.5),
            Vec2::new(-80.0, 15.0),
            Vec2::new(5000.0, -3000.0),
            Vec2::new(f32::NAN, 12.0),
        ];
        let dirs = [
            Vec2::ZERO,
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.6, -0.8),
            Vec2::new(f32::NAN, 1.0),
        ];
        for theme in Theme::all() {
            for &pos in &positions {
                for &dir in &dirs {
                    draw_gnaw_mark(&mut pb, pos, dir, Vec2::ZERO, theme);
                    draw_gold_fleck(&mut pb, pos, dir, Vec2::ZERO, 1.25);
                }
                for v in 0..10u8 {
                    draw_hall_landmark(&mut pb, pos, v, Vec2::ZERO, theme);
                }
            }
            for rect in [
                TileRect::new(0, 0, 0, 0),
                TileRect::new(-5, -5, 4, 4),
                TileRect::new(0, 0, 6, 6),
                TileRect::new(100, 100, 12, 12),
            ] {
                draw_boss_arena_floor(&mut pb, rect, theme, Vec2::ZERO);
            }
        }
    }

    #[test]
    fn gnaw_mark_is_visible_and_non_gold_on_all_themes() {
        for theme in Theme::all() {
            let level = Level::empty(theme);
            let cam = Vec2::new(320.0, 320.0);
            let mut base = PixelBuffer::new(48, 48);
            draw_floor(&mut base, &level, cam, 0.0);

            let mut with_mark = PixelBuffer::new(48, 48);
            draw_floor(&mut with_mark, &level, cam, 0.0);
            draw_gnaw_mark(
                &mut with_mark,
                cam + Vec2::new(24.0, 24.0),
                Vec2::new(1.0, 0.0),
                cam,
                theme,
            );

            let mut changed = 0usize;
            for y in 0..base.h {
                for x in 0..base.w {
                    if base.pixel_at(x, y) != with_mark.pixel_at(x, y) {
                        changed += 1;
                    }
                }
            }
            assert!(
                changed >= 20,
                "{theme:?}: gnaw mark only changed {changed} pixels"
            );
        }
    }

    #[test]
    fn hall_landmarks_produce_six_distinct_variants_per_theme() {
        for theme in Theme::all() {
            let level = Level::empty(theme);
            let cam = Vec2::new(400.0, 400.0);
            let mut buffers = Vec::new();
            for v in 0..HALL_LANDMARK_VARIANTS {
                let mut pb = PixelBuffer::new(56, 44);
                draw_floor(&mut pb, &level, cam, 0.0);
                draw_hall_landmark(&mut pb, cam + Vec2::new(28.0, 22.0), v, cam, theme);
                buffers.push(pb);
            }
            for i in 0..buffers.len() {
                for j in (i + 1)..buffers.len() {
                    let mut diff = 0usize;
                    for y in 0..44 {
                        for x in 0..56 {
                            if buffers[i].pixel_at(x, y) != buffers[j].pixel_at(x, y) {
                                diff += 1;
                            }
                        }
                    }
                    assert!(
                        diff >= 30,
                        "{theme:?}: landmark variants {i} and {j} only differ by {diff} px"
                    );
                }
            }
        }
    }

    #[test]
    fn boss_arena_floor_clips_strictly_to_arena_rect() {
        for theme in Theme::all() {
            let level = Level::empty(theme);
            let cam = Vec2::new(160.0, 160.0);
            let arena = TileRect::new(12, 12, 6, 5); // world x: 192..288, y: 192..272
            let mut base = PixelBuffer::new(160, 144);
            draw_floor(&mut base, &level, cam, 0.0);
            let mut with_arena = PixelBuffer::new(160, 144);
            draw_floor(&mut with_arena, &level, cam, 0.0);
            draw_boss_arena_floor(&mut with_arena, arena, theme, cam);

            let mut inside_changed = 0usize;
            for y in 0..base.h {
                for x in 0..base.w {
                    let wx = cam.x as i32 + x as i32;
                    let wy = cam.y as i32 + y as i32;
                    let in_arena = wx >= arena.x * TILE
                        && wx < arena.right() * TILE
                        && wy >= arena.y * TILE
                        && wy < arena.bottom() * TILE;
                    if in_arena {
                        if base.pixel_at(x, y) != with_arena.pixel_at(x, y) {
                            inside_changed += 1;
                        }
                    } else {
                        assert_eq!(
                            base.pixel_at(x, y),
                            with_arena.pixel_at(x, y),
                            "{theme:?}: boss arena bled outside rect at world ({wx}, {wy})"
                        );
                    }
                }
            }
            assert!(
                inside_changed > 500,
                "{theme:?}: boss arena floor barely drew ({inside_changed} px)"
            );
        }
    }

    #[test]
    fn boss_arena_floor_stays_low_contrast_under_entities() {
        for theme in Theme::all() {
            let level = Level::empty(theme);
            let cam = Vec2::new(320.0, 320.0);
            let arena = TileRect::new(20, 20, 8, 8);
            let mut pb = PixelBuffer::new(128, 128);
            draw_floor(&mut pb, &level, cam, 0.0);
            draw_boss_arena_floor(&mut pb, arena, theme, cam);

            let mut max_ch = 0u8;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let (r, g, b) = pb.pixel_at(x, y);
                    max_ch = max_ch.max(r).max(g).max(b);
                }
            }
            // Rat fur starts at 212+; floor must stay well below 120 so
            // entities, bullets, and boss telegraphs pop effortlessly.
            assert!(
                max_ch <= 110,
                "{theme:?}: boss arena peak channel {max_ch} is too bright for floor layer"
            );
        }
    }

    #[test]
    fn gold_fleck_is_visible_and_distinct_from_gnaw_mark() {
        for theme in Theme::all() {
            let level = Level::empty(theme);
            let cam = Vec2::new(320.0, 320.0);
            let mut base = PixelBuffer::new(32, 32);
            draw_floor(&mut base, &level, cam, 0.0);

            let mut fleck = PixelBuffer::new(32, 32);
            draw_floor(&mut fleck, &level, cam, 0.0);
            draw_gold_fleck(
                &mut fleck,
                cam + Vec2::new(16.0, 16.0),
                Vec2::new(1.0, 0.0),
                cam,
                1.0,
            );

            let mut changed = 0usize;
            for y in 0..base.h {
                for x in 0..base.w {
                    if base.pixel_at(x, y) != fleck.pixel_at(x, y) {
                        changed += 1;
                    }
                }
            }
            assert!(
                (4..=12).contains(&changed),
                "{theme:?}: gold fleck should be sparse & faint (4..=12 px), got {changed}"
            );
        }
    }

    fn make_rat(pos: Vec2, face: Vec2) -> Player {
        let mut p = Player::new(pos, WeaponKind::Gnaw);
        p.face = face.normalized();
        p.vel = p.face * 58.0;
        p
    }

    /// Render a single theme panel showing:
    /// - Top band (`h = 84`): Gnaw marks leading along 4 cells into a maze
    ///   entrance + gold flecks toward the cache + rat for scale + 4-dir strip.
    /// - Middle band (`h = 112`): Boss arena floor (4×4-cell repeating motif)
    ///   with a normal-floor comparison strip on the left + rat for scale.
    /// - Bottom band (`h = 104`): All 6 hall landmark variants (3×2 grid) with
    ///   the rat for scale.
    fn render_theme_sheet(theme: Theme) -> PixelBuffer {
        const W: usize = 240;
        const H_TOP: usize = 84;
        const H_MID: usize = 112;
        const H_BOT: usize = 104;
        const H: usize = H_TOP + H_MID + H_BOT;

        let level = Level::empty(theme);
        let cam = Vec2::new(320.0, 320.0); // cell (20, 20)
        let mut pb = PixelBuffer::new(W, H);
        draw_floor(&mut pb, &level, cam, 1.5);

        // ---- Band 1: Gnaw marks + maze breach + gold flecks + compass ------
        // Maze approach corridor runs East along cell row 22 (screen y = 32..48)
        // from cell col 21..25 into a maze breach at col 25, with cache at col 29.
        let wall_cells: &[(i32, i32, WallKind, NeighborMask)] = &[
            // Top & bottom walls framing the maze mouth at col 25 (x = 80..96)
            (25, 20, WallKind::Solid, NeighborMask(NeighborMask::S)),
            (25, 21, WallKind::Solid, NeighborMask(NeighborMask::N)),
            (25, 23, WallKind::Solid, NeighborMask(NeighborMask::S)),
            (25, 24, WallKind::Solid, NeighborMask(NeighborMask::N)),
            // Thin maze partitions inside the breach (cols 26..29)
            (26, 21, WallKind::Maze, NeighborMask(NeighborMask::W | NeighborMask::E)),
            (27, 21, WallKind::Maze, NeighborMask(NeighborMask::W | NeighborMask::E)),
            (28, 21, WallKind::Maze, NeighborMask(NeighborMask::W | NeighborMask::E)),
            (26, 23, WallKind::Maze, NeighborMask(NeighborMask::W | NeighborMask::E)),
            (27, 23, WallKind::Maze, NeighborMask(NeighborMask::W | NeighborMask::E)),
            (28, 23, WallKind::Maze, NeighborMask(NeighborMask::W | NeighborMask::E)),
        ];

        // 3 gnaw marks along the final 4 cells leading into the breach at x = 88.
        let breach_center = cam + Vec2::new(88.0, 40.0);
        let approach_dir = Vec2::new(1.0, 0.0);
        for (i, &dist) in GNAW_MARK_OFFSETS.iter().enumerate() {
            let stagger = if i == 1 { -1.5 } else { 1.0 };
            let pos = breach_center - approach_dir * dist + Vec2::new(0.0, stagger);
            draw_gnaw_mark(&mut pb, pos, approach_dir, cam, theme);
        }

        // Gold flecks inside the maze leading from the entrance toward the cache.
        for &offset in &[16.0_f32, 38.0] {
            let pos = breach_center + approach_dir * offset;
            draw_gold_fleck(&mut pb, pos, approach_dir, cam, 0.8);
        }

        // 4-direction + diagonal gnaw mark & gold fleck compass strip on the right.
        for (i, dir) in [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.707, -0.707),
        ]
        .iter()
        .enumerate()
        {
            let gx = 166.0 + (i as f32) * 16.0;
            draw_gnaw_mark(&mut pb, cam + Vec2::new(gx, 24.0), *dir, cam, theme);
            draw_gold_fleck(&mut pb, cam + Vec2::new(gx, 52.0), *dir, cam, 0.8 + i as f32 * 0.4);
        }

        // Draw walls + cache pedestal + rat for scale in Band 1.
        for &(cx, cy, kind, mask) in wall_cells {
            let v = WallCellView::new(cx, cy, hash2(cx, cy), theme, kind, mask);
            draw_wall_ground(&mut pb, &v, cam, level.palette.accent);
        }
        let cache = MazeRewardView::new(cam + Vec2::new(146.0, 40.0), Vec2::new(-1.0, 0.0));
        draw_maze_reward_ground(&mut pb, &cache, cam, level.palette.accent);
        for &(cx, cy, kind, mask) in wall_cells {
            let v = WallCellView::new(cx, cy, hash2(cx, cy), theme, kind, mask);
            draw_wall_cell(&mut pb, &v, cam, level.palette.accent);
        }
        draw_maze_reward(&mut pb, &cache, cam, level.palette.accent);

        let rat_trail = make_rat(cam + Vec2::new(22.0, 40.0), Vec2::new(1.0, 0.0));
        draw_rat(&mut pb, &rat_trail, 0.5, cam);

        // Subtle 1 px divider between Band 1 and Band 2.
        pb.rect_fill(0, H_TOP as i32 - 1, W as i32, 1, (48, 52, 60));

        // ---- Band 2: Boss Arena Floor (left 2 cells = normal floor, right 13
        //      cells = boss arena showing 4×4-cell repeating motif) -----------
        // Band 2 spans screen y = 84..196 -> cells cy = 26..32 (world y = 416..512).
        let arena_rect = TileRect::new(22, 26, 13, 6);
        draw_boss_arena_floor(&mut pb, arena_rect, theme, cam);
        let rat_arena = make_rat(cam + Vec2::new(98.0, 144.0), Vec2::new(0.85, -0.5));
        draw_rat(&mut pb, &rat_arena, 1.1, cam);

        pb.rect_fill(0, (H_TOP + H_MID) as i32 - 1, W as i32, 1, (48, 52, 60));

        // ---- Band 3: All 6 Hall Landmark Variants (3×2 grid) + rat scale ---
        let base_y = (H_TOP + H_MID) as f32;
        for v in 0..HALL_LANDMARK_VARIANTS {
            let col = (v % 3) as f32;
            let row = (v / 3) as f32;
            let pos = cam + Vec2::new(42.0 + col * 78.0, base_y + 26.0 + row * 50.0);
            draw_hall_landmark(&mut pb, pos, v, cam, theme);
        }
        // Place two rats for scale right next to landmarks 0 and 4.
        let rat_hall1 = make_rat(cam + Vec2::new(66.0, base_y + 26.0), Vec2::new(-0.9, 0.3));
        let rat_hall2 = make_rat(cam + Vec2::new(146.0, base_y + 76.0), Vec2::new(0.9, -0.3));
        draw_rat(&mut pb, &rat_hall1, 0.9, cam);
        draw_rat(&mut pb, &rat_hall2, 1.7, cam);

        pb
    }

    fn write_ppm(path: &str, pb: &PixelBuffer) {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                out.extend_from_slice(&[c.0, c.1, c.2]);
            }
        }
        std::fs::write(path, out).unwrap();
    }

    /// Visual reference sheet: `RATRUN_DUMP=marks cargo test dump_marks_sheet`
    /// writes `/tmp/ratrun_marks.ppm` (all 3 themes side-by-side) plus
    /// `/tmp/ratrun_marks_{sewer,kitchen,lab}.ppm`.
    #[test]
    fn dump_marks_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("marks") {
            return;
        }
        let sewer = render_theme_sheet(Theme::Sewer);
        let kitchen = render_theme_sheet(Theme::Kitchen);
        let lab = render_theme_sheet(Theme::Lab);

        write_ppm("/tmp/ratrun_marks_sewer.ppm", &sewer);
        write_ppm("/tmp/ratrun_marks_kitchen.ppm", &kitchen);
        write_ppm("/tmp/ratrun_marks_lab.ppm", &lab);

        let gap = 4usize;
        let total_w = sewer.w * 3 + gap * 2;
        let total_h = sewer.h;
        let mut combined = PixelBuffer::new(total_w, total_h);
        combined.clear((16, 18, 22));
        for (idx, sheet) in [&sewer, &kitchen, &lab].iter().enumerate() {
            let ox = idx * (sheet.w + gap);
            for y in 0..sheet.h {
                for x in 0..sheet.w {
                    combined.plot((ox + x) as i32, y as i32, sheet.pixel_at(x, y));
                }
            }
        }
        write_ppm("/tmp/ratrun_marks.ppm", &combined);
    }
}
