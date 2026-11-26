//! Short, self-lit streaks with their bright tips pointing downstream.
//! The grid drifts in world space through the warning and surge, and is
//! reseeded during the invisible part of the cycle.

use crate::game::hazards::{
    outfall_surge, outfall_surge_distance, SurgePhase, SURGE_ACTIVE_SECS, SURGE_PERIOD,
    SURGE_WARN_SECS,
};
use crate::game::level::{Level, Theme};
use crate::math::Vec2;

use super::framebuffer::PixelBuffer;
use super::glyphfx::hash;
use super::palette::{mix, Rgb};

const CYAN: Rgb = (170, 225, 245);
const MAX_CRESTS: usize = 60;
const DRIFT_PX_PER_SEC: f32 = 9.0;
// Two terminal rows in the half-block picture.
const RAT_CLEAR_PX: f32 = 4.0;
const TAIL_ALPHA: f32 = 0.35;
const DIAG: f32 = std::f32::consts::FRAC_1_SQRT_2;
// One sine period in eighths, with exact zero offsets at both ends.
const WAVE: [f32; 9] = [0.0, DIAG, 1.0, DIAG, 0.0, -DIAG, -1.0, -DIAG, 0.0];

struct Current {
    dir: Vec2,
    travel: f32,
    index: u64,
    alpha: f32,
    strength: f32,
}

impl Current {
    fn sample(seed: u64, elapsed: f32) -> Option<Self> {
        let surge = outfall_surge(seed, elapsed);
        let clock = elapsed % SURGE_PERIOD;
        let index = (elapsed / SURGE_PERIOD).floor() as u64;
        let (travel, index, alpha, strength) = match surge.phase {
            SurgePhase::Quiet => return None,
            SurgePhase::Warning => {
                let age = clock - (SURGE_PERIOD - SURGE_WARN_SECS);
                let alpha = 0.24 * smoothstep(age / 0.85);
                (DRIFT_PX_PER_SEC * age, index + 1, alpha, 0.0)
            }
            SurgePhase::Active => {
                let progress = (clock / SURGE_ACTIVE_SECS).clamp(0.0, 1.0);
                let strength = (std::f32::consts::PI * progress).sin().max(0.0);
                let fade = 1.0 - smoothstep((progress - 0.55) / 0.45);
                let travel = DRIFT_PX_PER_SEC * (SURGE_WARN_SECS + clock)
                    + outfall_surge_distance(elapsed - clock, elapsed);
                (travel, index, (0.24 + 0.04 * strength) * fade, strength)
            }
        };
        Some(Self {
            dir: surge.dir,
            travel,
            index,
            alpha,
            strength,
        })
    }

    fn visit_crests(
        &self,
        seed: u64,
        cam: Vec2,
        size: Vec2,
        zoom: f32,
        mut visit: impl FnMut(Vec2),
    ) -> usize {
        // Scale world spacing with zoom to hold screen density. Leave some
        // room under the hard cap for partial cells at the view edges.
        let density = 1.0 - 0.5 * smoothstep((zoom - 1.5) / 0.5);
        let cap = (MAX_CRESTS as f32 * density) as usize;
        let gap = (14.0 * zoom).max((size.x * size.y / 48.0).sqrt()) / density.sqrt();
        let origin = cam - self.dir * self.travel;
        let first = origin * (1.0 / gap);
        let last = (origin + size) * (1.0 / gap);
        let mut count = 0;
        for y in first.y.floor() as i32..=last.y.floor() as i32 {
            for x in first.x.floor() as i32..=last.x.floor() as i32 {
                let cell = (u64::from(x as u32) << 32) | u64::from(y as u32);
                let bits = hash(seed, cell, self.index);
                let jitter = |sample: u8| 0.15 + f32::from(sample) * (0.7 / 255.0);
                let anchor = Vec2::new(
                    x as f32 + jitter(bits as u8),
                    y as f32 + jitter((bits >> 8) as u8),
                ) * gap;
                let p = anchor - origin;
                if p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y {
                    continue;
                }
                visit(p);
                count += 1;
                if count == cap {
                    return count;
                }
            }
        }
        count
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

struct Stroke {
    dir: Vec2,
    offsets: [Vec2; WAVE.len()],
}

impl Stroke {
    fn new(dir: Vec2, strength: f32, zoom: f32) -> Self {
        let length = (4.4 + 1.1 * strength) * zoom;
        let across = dir.perp() * (0.85 * zoom);
        let offsets = std::array::from_fn(|i| {
            let t = i as f32 / (WAVE.len() - 1) as f32;
            dir * ((t - 0.5) * length) + across * WAVE[i]
        });
        Self { dir, offsets }
    }

    fn visit(&self, centre: Vec2, mut visit: impl FnMut(i32, i32, f32)) {
        let points = self.offsets.map(|offset| {
            let p = centre + offset;
            (p.x.round() as i32, p.y.round() as i32)
        });
        let tail = points[0];
        let head = points[WAVE.len() - 1];
        let along = |(x, y): (i32, i32)| {
            Vec2::new((x - tail.0) as f32, (y - tail.1) as f32).dot(self.dir)
        };
        let span = along(head).max(1.0);
        // At up to 2× each segment spans at most two raster steps.
        // Blend each pixel once, even where rounded segments overlap.
        let mut painted = [tail; 2 * (WAVE.len() - 1) + 1];
        let mut count = 1;
        visit(tail.0, tail.1, TAIL_ALPHA);
        for segment in points.windows(2) {
            let (x, y) = segment[0];
            let (dx, dy) = (segment[1].0 - x, segment[1].1 - y);
            let steps = dx.abs().max(dy.abs());
            // Integer DDA along the segment's major axis, rounding the minor axis.
            let advance = |delta: i32, step: i32| {
                (delta * step + delta.signum() * (steps / 2)) / steps
            };
            for step in 1..=steps {
                let p = (x + advance(dx, step), y + advance(dy, step));
                if painted[..count].contains(&p) {
                    continue;
                }
                let t = (along(p) / span).clamp(0.0, 1.0);
                visit(p.0, p.1, TAIL_ALPHA + (1.0 - TAIL_ALPHA) * t);
                painted[count] = p;
                count += 1;
            }
        }
    }
}

/// Draw in the lit world-resolution picture, before hostile information.
pub fn draw(pb: &mut PixelBuffer, level: &Level, cam: Vec2, rat: Vec2, zoom: f32, elapsed: f32) {
    if level.theme != Theme::Outfall || pb.w == 0 || pb.h == 0 {
        return;
    }
    let Some(current) = Current::sample(level.seed, elapsed) else {
        return;
    };
    if current.alpha < 1.0 / 255.0 {
        return;
    }

    let (w, h) = (pb.w, pb.h);
    let rat = rat - cam;
    let clear_sq = (RAT_CLEAR_PX * zoom) * (RAT_CLEAR_PX * zoom);
    let stroke = Stroke::new(current.dir, current.strength, zoom);
    let pixels = pb.pixels_mut();
    current.visit_crests(level.seed, cam, Vec2::new(w as f32, h as f32), zoom, |centre| {
        stroke.visit(centre, |x, y, weight| {
            if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                return;
            }
            let pixel_pos = Vec2::new(x as f32, y as f32);
            if pixel_pos.dist_sq(rat) <= clear_sq
                || !level.tiles.get_at(cam + pixel_pos).is_open()
            {
                return;
            }
            let pixel = &mut pixels[y as usize * w + x as usize];
            *pixel = mix(*pixel, CYAN, current.alpha * weight);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Rgb = (18, 28, 38);
    const CAM: Vec2 = Vec2 { x: 256.0, y: 256.0 };
    const RAT: Vec2 = Vec2 { x: 316.0, y: 296.0 };

    fn frame(level: &Level, elapsed: f32, zoom: f32) -> PixelBuffer {
        let mut pb = PixelBuffer::new((120.0 * zoom) as usize, (80.0 * zoom) as usize);
        pb.clear(BASE);
        draw(&mut pb, level, CAM, RAT, zoom, elapsed);
        pb
    }

    #[test]
    fn outfall_current_paints_warning_and_active_and_leaves_the_rat_clear() {
        let outfall = Level::empty(Theme::Outfall);
        let sewer = Level::empty(Theme::Sewer);
        for elapsed in [8.5, 9.55] {
            assert!(frame(&sewer, elapsed, 1.0).pixels().iter().all(|&c| c == BASE));
            for zoom in [0.5, 1.0, 2.0] {
                let pb = frame(&outfall, elapsed, zoom);
                let mut painted = 0;
                for (i, &pixel) in pb.pixels().iter().enumerate() {
                    if pixel != BASE {
                        painted += 1;
                        let at = CAM + Vec2::new((i % pb.w) as f32, (i / pb.w) as f32);
                        assert!(at.dist(RAT) > RAT_CLEAR_PX * zoom);
                    }
                }
                assert!(painted > 0, "{elapsed} at {zoom}×");
                assert_eq!(pb.pixels(), frame(&outfall, elapsed, zoom).pixels());
            }
        }
        for elapsed in [7.0, 10.1] {
            assert!(frame(&outfall, elapsed, 1.0).pixels().iter().all(|&c| c == BASE));
        }
    }

    #[test]
    fn outfall_crests_keep_their_anchors_and_accelerate_along_the_flow() {
        fn centres(current: &Current) -> Vec<Vec2> {
            let mut points = Vec::new();
            current.visit_crests(23, CAM, Vec2::new(120.0, 80.0), 1.0, |p| points.push(p));
            points
        }
        let mut distances = [0.0; 3];
        for ((start, end), travel) in [(8.4, 8.5), (8.95, 9.05), (9.5, 9.6)]
            .into_iter().zip(&mut distances)
        {
            let first = Current::sample(23, start).unwrap();
            let next = Current::sample(23, end).unwrap();
            *travel = next.travel - first.travel;
            assert!(*travel > 0.0);
            let shift = first.dir * *travel;
            let after = centres(&next);
            let before = centres(&first);
            assert!(before.iter().filter(|&&p| {
                after.iter().any(|&q| q.dist(p + shift) < 0.001)
            }).count() > before.len() / 2);
        }
        assert!(distances[2] > 4.0 * distances[0]);
    }

    #[test]
    fn outfall_crest_count_is_bounded_and_thins_at_long_zoom() {
        let current = Current::sample(23, 9.55).unwrap();
        for w in [57.0, 120.0, 300.0] {
            for pan in [Vec2::ZERO, Vec2::new(13.0, 8.0), Vec2::new(-7.0, -11.0)] {
                for zoom in [0.5, 1.0, 1.5, 2.0] {
                    let count = current.visit_crests(
                        23, CAM + pan, Vec2::new(w, 80.0) * zoom, zoom, |_| {},
                    );
                    let cap = if zoom == 2.0 { 30 } else { MAX_CRESTS };
                    assert!((1..=cap).contains(&count));
                }
            }
        }
    }

    #[test]
    fn outfall_strokes_point_downstream_with_bright_heads_in_all_eight_directions() {
        let mut seen = Vec::new();
        for seed in 0..64 {
            let warning = Current::sample(seed, 8.9).unwrap();
            if seen.contains(&warning.dir) {
                continue;
            }
            seen.push(warning.dir);
            let active = Current::sample(seed, 9.55).unwrap();
            for current in [warning, active] {
                for zoom in [0.5, 1.0, 2.0] {
                    let stroke = Stroke::new(current.dir, current.strength, zoom);
                    for centre in [Vec2::new(16.0, 16.0), Vec2::new(16.25, 16.5)] {
                        let mut pixels: Vec<(Vec2, Rgb)> = Vec::new();
                        stroke.visit(centre, |x, y, weight| {
                            let point = Vec2::new(x as f32, y as f32);
                            assert!(
                                pixels.is_empty() || pixels.iter().any(|(p, _)| {
                                    (p.x - point.x).abs().max((p.y - point.y).abs()) == 1.0
                                }),
                                "gap in the raster for {:?} at {zoom}×",
                                current.dir,
                            );
                            pixels.push((point, mix(BASE, CYAN, current.alpha * weight)));
                        });
                        let (tail, tail_color) = pixels[0];
                        let tip = centre + stroke.offsets[WAVE.len() - 1];
                        let head = Vec2::new(tip.x.round(), tip.y.round());
                        let (_, head_color) = pixels.iter().copied().find(|&(p, _)| p == head).unwrap();
                        let heading = head - tail;
                        assert!(heading.dot(current.dir) > 0.9 * heading.len());
                        assert!(head_color.1 > tail_color.1);
                    }
                }
            }
        }
        assert_eq!(seen.len(), 8);
    }
}
