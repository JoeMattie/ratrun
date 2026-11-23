//! W3 — bump-mapped floors and walls from the albedo's own luminance
//! gradient (contract §4, technique 08). Writes the per-pixel normals the
//! specular and reflection stages read.
//!
//! The pre-lit albedo already *is* a height field: brick faces against dark
//! mortar, tiles against grout, planks against seams, rivets against steel,
//! lead type against a recessed tray. One pass over the picture takes
//! `L = (77·r + 150·g + 29·b) >> 8` at `(x±1, y±1)` and turns the 4-neighbour
//! gradient into a tangent normal, the way a normal map would, with no texture
//! assets and no allocation:
//!
//! ```text
//! gx = L[x+1] - L[x-1]                (luminance of the 2 px stencil)
//! gy = L[y+1] - L[y-1]
//! n  = (-gx, -gy) · NORMAL_ONE / SLOPE_FULL   → Q0.7, capped to unit length
//! ```
//!
//! The negated gradient is the usual height-field normal (`h` rising to the
//! right makes the surface face left), so a ridge catches light on the flank
//! that faces the lamp and deepens on the far flank. The normal is written for
//! every floor and wall pixel of a sample that has a dominant light; the
//! diffuse modulation is `1 + clamp(N·L, -0.35, +0.55) · BUMP · bump/255`,
//! applied to the albedo (the shade multiplies it afterwards, so scaling the
//! albedo scales the *lit* brightness by the same factor).
//!
//! Deliberate limits, per the contract and §5's tier table:
//!
//! * **Full/Hard only.** Coarse, Compact and Ansi16 return immediately; the
//!   substep tiers have no materials buffer to read relief from.
//! * **No dominant light, no bump.** `info.dir == [0, 0, 0]` (ambient only)
//!   leaves the pixels and the normals alone, so the stage cannot invent
//!   relief in the dark.
//! * **Never CREATURE, never Emissive.** A creature's normal is the rim
//!   stage's business (it reads its own silhouette mask); an emissive pixel is
//!   self-lit and every stage leaves it alone. Both are `0` in the material
//!   table below.
//! * **No noise.** Gradients within [`NOISE_FLOOR`] levels in both axes are
//!   picture dither, not relief: the pixel stays flat.
//! * **Restrained.** [`strength::BUMP`] (0.5) halves the bold plate's swing:
//!   the strongest wet-brick relief moves a pixel by about 10 % at a full
//!   tilt, which is the owner's brief, not a bevel.
//! * **Still.** Every term is integer arithmetic over the picture and the
//!   frame's grid; nothing reads `elapsed`/`step8`, so a still scene is
//!   byte-identical frame to frame and the goldens cannot move.
//!
//! Cost: one luminance byte per pixel into a three-row rolling buffer (the
//! `thread_local!` scratch grows once and is reused), then ~20 integer ops per
//! pixel — well inside the stage's 60 µs at 120×80.

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{strength, Canvas, FxFrame, Normal, NORMAL_ONE};
use super::material::{self, MaterialKind};
use crate::game::level::Theme;

/// Gradients this small in *both* axes are picture noise (terminal dither,
/// anti-aliasing): no relief, no normal. Luminance levels, 0..=255.
const NOISE_FLOOR: i32 = 6;

/// The gradient (luminance levels across the 2 px stencil) that tilts a normal
/// fully sideways. Brick/mortar steps are 40-90, so they land at 15-30°.
const SLOPE_FULL: i32 = 192;

/// `N·L` is clamped to `[-0.35, +0.55]` before the material gain (contract
/// §4), in Q8 (`256` = 1.0).
const NDOTL_MIN_Q8: i32 = -90;
const NDOTL_MAX_Q8: i32 = 141;

/// Three neighbouring rows of the pre-lit luminance (`y-1`, `y`, `y+1`, slot
/// `y % 3`), so each pixel's luminance is computed once and its gradient reads
/// neighbours out of L1. Render-only scratch: grown on the first frame of a
/// size, never allocated again.
thread_local! {
    static LUM_ROWS: std::cell::RefCell<Vec<u8>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Pre-composite pixel stage over the **unlit** albedo: write a tangent
/// normal per pixel into `normals` (`canvas.w * canvas.h`, zeroed this frame)
/// and modulate the albedo by `N·L` against the covering sample's dominant
/// light (`fx::strength::BUMP`). Never touches CREATURE or Emissive pixels.
pub(super) fn apply(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    normals: &mut [Normal],
    frame: &FxFrame<'_>,
) {
    if !matches!(frame.mode, LightingMode::Full | LightingMode::Hard) {
        return;
    }
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 || grid.w == 0 || grid.h == 0 || grid.stride == 0 {
        return;
    }
    if canvas.px.len() < w * h || normals.len() < w * h {
        return;
    }
    // `strength::BUMP` (0.5) in Q8: the one owner-set scale this stage obeys.
    let bump_gain = (strength::BUMP * 256.0).round() as i32;
    if bump_gain <= 0 {
        return;
    }
    let mat = canvas.mat;
    let px = &mut *canvas.px;
    let normals = &mut *normals;
    let stride = grid.stride;
    let floor_gain = gain_table(frame.theme, false, bump_gain);
    let wall_gain = gain_table(frame.theme, true, bump_gain);
    // The first run of a row ends at the next sample boundary, exactly like
    // `Grid::composite_px`, so a pixel reads the sample that covers it.
    let first = (stride - grid.offset.0 % stride).min(w);
    let first_sample = grid.offset.0 / stride;

    LUM_ROWS.with(|cell| {
        let mut rows = cell.borrow_mut();
        if rows.len() < 3 * w {
            rows.resize(3 * w, 0);
        }
        let lum = &mut rows[..];
        for (x, slot) in lum[..w].iter_mut().enumerate() {
            *slot = luminance(px[x]);
        }
        for y in 0..h {
            let line = y * w;
            let down_row = (y + 1).min(h - 1);
            let own = (y % 3) * w;
            let up = (y.saturating_sub(1) % 3) * w;
            let down = (down_row % 3) * w;
            if down_row > y {
                for (x, slot) in lum[down..down + w].iter_mut().enumerate() {
                    *slot = luminance(px[(y + 1) * w + x]);
                }
            }
            let grid_row = ((y + grid.offset.1) / stride) * grid.w;
            let (mut x, mut sx) = (0usize, first_sample);
            let mut end = first;
            while x < w && end > x {
                let sample = grid_row + sx;
                if let Some(info) = grid.info.get(sample) {
                    let (lx, ly) = (i32::from(info.dir[0]), i32::from(info.dir[1]));
                    // Ambient only, or a lamp exactly overhead: nothing to bump.
                    let live = info.dom > 0 && (lx != 0 || ly != 0);
                    if live {
                        let wall = grid
                            .surfaces
                            .get(sample)
                            .is_some_and(|s| s.layer == LayerMask::WALLS.0);
                        let gain = if wall { &wall_gain } else { &floor_gain };
                        for xx in x..end {
                            let i = line + xx;
                            let k = i32::from(gain[usize::from(mat.get(i).copied().unwrap_or(0))]);
                            if k == 0 {
                                continue;
                            }
                            let centre = i32::from(lum[own + xx]);
                            let left = if xx == 0 {
                                centre
                            } else {
                                i32::from(lum[own + xx - 1])
                            };
                            let right = if xx + 1 >= w {
                                centre
                            } else {
                                i32::from(lum[own + xx + 1])
                            };
                            let gx = right - left;
                            let gy = i32::from(lum[down + xx]) - i32::from(lum[up + xx]);
                            if gx.abs() <= NOISE_FLOOR && gy.abs() <= NOISE_FLOOR {
                                continue; // flat: normal stays [0, 0], pixel untouched
                            }
                            let one = i32::from(NORMAL_ONE);
                            let mut nx = (-gx * one) / SLOPE_FULL;
                            let mut ny = (-gy * one) / SLOPE_FULL;
                            nx = nx.clamp(-one, one);
                            ny = ny.clamp(-one, one);
                            let n2 = nx * nx + ny * ny;
                            if n2 > one * one {
                                // Two axes at full tilt: keep the direction, cap
                                // the tangent at unit length (`nz` then reads 0).
                                let m = n2.isqrt().max(1);
                                nx = nx * one / m;
                                ny = ny * one / m;
                            }
                            normals[i] = [nx as i8, ny as i8];
                            // Q8 `N·L`: the tangent dot the light (a light
                            // straight overhead shades no relief at all).
                            let d = ((nx * lx + ny * ly) * 2 >> 12)
                                .clamp(NDOTL_MIN_Q8, NDOTL_MAX_Q8);
                            let e = d * k / 256;
                            if e == 0 {
                                continue;
                            }
                            let (r, g, b) = px[i];
                            px[i] = (shade(r, e), shade(g, e), shade(b, e));
                        }
                    }
                }
                x = end;
                if x >= w {
                    break;
                }
                sx += 1;
                end = (end + stride).min(w);
            }
        }
    });
}

/// The per-material bump gain indexed by the material byte, in Q8 (`0` = leave
/// the pixel's albedo alone): the resolved material's `bump` depth, scaled by
/// the pixel's height class and by [`strength::BUMP`].
fn gain_table(theme: Theme, wall: bool, bump_gain: i32) -> [u8; 256] {
    let mut table = [0u8; 256];
    for (byte, slot) in table.iter_mut().enumerate() {
        let b = byte as u8;
        if material::is_creature(b) {
            continue; // a creature's normal is the rim stage's own business
        }
        let kind = material::resolve(b, theme, wall);
        if kind == MaterialKind::Emissive {
            continue; // self-lit: every stage leaves it alone
        }
        let depth = i32::from(material::params(kind).bump);
        if depth == 0 {
            continue; // glass, shell, slime, fur: no relief to catch light with
        }
        // A raised object reads deeper relief: height class 0..=7 → 0.75x..=1.19x.
        let height_gain = 192 + i32::from(material::height(b)) * 16;
        let k = (depth * height_gain / 256) * bump_gain / 255;
        *slot = k.clamp(0, 255) as u8;
    }
    table
}

/// Scale one channel by `1 + e/256` (`e` is Q8, negative = deepen).
#[inline]
fn shade(c: u8, e: i32) -> u8 {
    ((i32::from(c) * (256 + e) + 128) >> 8).clamp(0, 255) as u8
}

/// The pre-lit luminance the height field is read from: `(77r + 150g + 29b) >> 8`.
#[inline]
fn luminance((r, g, b): crate::render::palette::Rgb) -> u8 {
    ((i32::from(r) * 77 + i32::from(g) * 150 + i32::from(b) * 29) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::fx::{testkit, LightInfo, DIR_ONE};
    use crate::game::level::Level;
    use crate::render::framebuffer::PixelBuffer;
    use crate::render::palette::Rgb;

    const W: usize = 16;
    const H: usize = 8;
    /// The column of the bright 1 px ridge (a grout lip, a rivet row, a plank edge).
    const RIDGE: usize = 8;
    const FLOOR: Rgb = (85, 71, 78);
    const BRIGHT: Rgb = (150, 140, 130);
    const DARK: Rgb = (52, 44, 48);
    /// A row well away from the top and bottom edges.
    const ROW: usize = 4;

    fn lum(c: Rgb) -> i32 {
        i32::from(super::luminance(c))
    }

    /// A `W × H` buffer of `FLOOR`, cleared with the untagged pen, plus a 1 px
    /// vertical line of `BRIGHT` in column `ridge` stamped with `pen`.
    fn ridge_picture(ridge: Option<usize>, pen: u8) -> PixelBuffer {
        let mut pb = PixelBuffer::new(W, H);
        pb.set_materials(true);
        pb.set_pen(0);
        pb.clear(FLOOR);
        if let Some(ridge) = ridge {
            pb.set_pen(pen);
            for y in 0..H as i32 {
                pb.plot(ridge as i32, y, BRIGHT);
            }
        }
        pb.set_pen(0);
        pb
    }

    /// A `W × H` buffer split into two flat bands at column `edge`.
    fn step_picture(left: Rgb, right: Rgb, edge: usize) -> PixelBuffer {
        let mut pb = PixelBuffer::new(W, H);
        pb.set_materials(true);
        pb.set_pen(0);
        pb.clear(left);
        pb.rect_fill(edge as i32, 0, W as i32, H as i32, right);
        pb
    }

    /// Every sample of a `w × h` grid at stride 1, so a pixel reads its own
    /// sample, with the given dominant light direction (Q12).
    fn grid_with_dir(w: usize, h: usize, dir: [i16; 3]) -> Grid {
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in grid.info.iter_mut() {
            *info = LightInfo {
                dir,
                dom: 200,
                ..LightInfo::default()
            };
        }
        grid
    }

    /// Run the stage over `pb`; returns the pixels and the normals.
    fn run(mut pb: PixelBuffer, grid: &Grid, frame: &FxFrame<'_>) -> (Vec<Rgb>, Vec<Normal>) {
        let (w, h) = (pb.w, pb.h);
        let mut normals = vec![[0i8; 2]; w * h];
        {
            let mut canvas = testkit::canvas(&mut pb);
            apply(grid, &mut canvas, &mut normals, frame);
        }
        (pb.pixels().to_vec(), normals)
    }

    fn sewer_level() -> Level {
        Level::empty(Theme::Sewer)
    }

    #[test]
    fn an_edge_brightens_its_lamp_side_and_deepens_the_far_side() {
        let level = sewer_level();
        let frame = testkit::frame(&level, LightingMode::Full);
        let before = ridge_picture(Some(RIDGE), 0).pixels().to_vec();

        // Light from the left: `dir` points from the surface toward the lamp,
        // so -x is a lamp on the left. The ridge's left flank faces it.
        let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);
        let (after, normals) = run(ridge_picture(Some(RIDGE), 0), &grid, &frame);
        let lamp_side = ROW * W + RIDGE - 1;
        let far_side = ROW * W + RIDGE + 1;
        let top = ROW * W + RIDGE;
        assert!(
            lum(after[lamp_side]) > lum(before[lamp_side]),
            "lamp-side flank should brighten: {:?} -> {:?}",
            before[lamp_side],
            after[lamp_side]
        );
        assert!(
            lum(after[far_side]) < lum(before[far_side]),
            "far flank should deepen: {:?} -> {:?}",
            before[far_side],
            after[far_side]
        );
        assert_eq!(after[top], before[top], "the ridge top has a zero gradient");
        assert!(normals[lamp_side][0] < 0, "the lamp-side flank faces -x");
        assert!(normals[far_side][0] > 0, "the far flank faces +x");
        // Restrained: never more than a fifth on this picture.
        let gain = lum(after[lamp_side]) - lum(before[lamp_side]);
        let base = lum(before[lamp_side]).max(1);
        assert!(gain * 5 < base, "bump must stay subtle, got +{gain} on {base}");

        // Move the lamp to the right: the two flanks swap roles.
        let grid = grid_with_dir(W, H, [DIR_ONE, 0, 0]);
        let (after, normals) = run(ridge_picture(Some(RIDGE), 0), &grid, &frame);
        assert!(
            lum(after[lamp_side]) < lum(before[lamp_side]),
            "with the lamp on the right the left flank deepens"
        );
        assert!(
            lum(after[far_side]) > lum(before[far_side]),
            "with the lamp on the right the right flank brightens"
        );
        assert!(normals[lamp_side][0] < 0 && normals[far_side][0] > 0);
    }

    #[test]
    fn a_flat_patch_is_left_exactly_alone() {
        let level = sewer_level();
        let frame = testkit::frame(&level, LightingMode::Full);
        let pb = ridge_picture(None, 0);
        let before = pb.pixels().to_vec();
        let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);
        let (after, normals) = run(pb, &grid, &frame);
        assert_eq!(after, before, "a flat floor has no relief to shade");
        assert!(normals.iter().all(|n| *n == [0, 0]));
    }

    #[test]
    fn no_dominant_light_leaves_the_picture_alone() {
        let level = sewer_level();
        let frame = testkit::frame(&level, LightingMode::Full);
        let pb = ridge_picture(Some(RIDGE), 0);
        let before = pb.pixels().to_vec();
        // Ambient only: the foundation writes `[0, 0, 0]` and `dom == 0`.
        let mut grid = grid_with_dir(W, H, [0, 0, 0]);
        for info in grid.info.iter_mut() {
            info.dom = 0;
        }
        let (after, normals) = run(pb, &grid, &frame);
        assert_eq!(after, before);
        assert!(normals.iter().all(|n| *n == [0, 0]));
    }

    #[test]
    fn normals_follow_the_gradient_by_sign() {
        let level = sewer_level();
        let frame = testkit::frame(&level, LightingMode::Full);
        let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);

        // Bright on the left, dark on the right: the height falls to the
        // right, so the surface faces +x.
        let pb = step_picture(BRIGHT, DARK, RIDGE);
        let (_, normals) = run(pb, &grid, &frame);
        let edge = ROW * W + RIDGE - 1;
        assert!(normals[edge][0] > 0, "falling to the right faces +x");
        assert_eq!(normals[edge][1], 0, "a vertical edge has no y gradient");
        assert!(material::nz(normals[edge]) > 0, "a slope is not a wall");

        // The mirror picture faces the other way.
        let pb = step_picture(DARK, BRIGHT, RIDGE);
        let (_, normals) = run(pb, &grid, &frame);
        assert!(normals[edge][0] < 0, "rising to the right faces -x");
    }

    #[test]
    fn emissive_and_creature_pixels_are_untouched() {
        let level = sewer_level();
        let frame = testkit::frame(&level, LightingMode::Full);
        let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);
        let pens = [
            material::pen(MaterialKind::Emissive, false, 0),
            material::pen(MaterialKind::WetBrick, true, 4),
        ];
        for pen in pens {
            let pb = ridge_picture(Some(RIDGE), pen);
            let before = pb.pixels().to_vec();
            let (after, normals) = run(pb, &grid, &frame);
            // The tagged line itself stays flat and keeps its colour; the floor
            // either side of it still reads its silhouette gradient (a contact
            // edge, which is wanted).
            for y in 0..H {
                let i = y * W + RIDGE;
                assert_eq!(after[i], before[i], "pen {pen} must not be bumped");
                assert_eq!(normals[i], [0, 0], "pen {pen} stays flat");
            }
        }
    }

    #[test]
    fn a_still_scene_is_identical_within_one_eighth_second_step() {
        let level = sewer_level();
        let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);
        let mut first = testkit::frame(&level, LightingMode::Full);
        first.elapsed = 0.0;
        first.step8 = 0;
        let mut second = testkit::frame(&level, LightingMode::Full);
        second.elapsed = 0.08;
        second.step8 = 0;
        assert_eq!(first.step8, second.step8);
        let (a, an) = run(ridge_picture(Some(RIDGE), 0), &grid, &first);
        let (b, bn) = run(ridge_picture(Some(RIDGE), 0), &grid, &second);
        assert_eq!(a, b, "a still scene draws the same pixels");
        assert_eq!(an, bn);
    }

    #[test]
    fn the_stage_is_off_below_full_and_hard() {
        let level = sewer_level();
        for mode in [LightingMode::Coarse, LightingMode::Compact] {
            let frame = testkit::frame(&level, mode);
            let pb = ridge_picture(Some(RIDGE), 0);
            let before = pb.pixels().to_vec();
            let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);
            let (after, normals) = run(pb, &grid, &frame);
            assert_eq!(after, before, "{mode:?} is not a bump tier");
            assert!(normals.iter().all(|n| *n == [0, 0]));
        }
    }

    #[test]
    fn hard_tier_bumps_too() {
        let level = sewer_level();
        let frame = testkit::frame(&level, LightingMode::Hard);
        let pb = ridge_picture(Some(RIDGE), 0);
        let before = pb.pixels().to_vec();
        let grid = grid_with_dir(W, H, [-DIR_ONE, 0, 0]);
        let (after, _) = run(pb, &grid, &frame);
        assert_ne!(after, before, "Hard is a bump tier");
    }
}
