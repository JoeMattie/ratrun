//! Short, self-lit wave crests. The grid drifts in world space for the whole
//! warning and surge, and is reseeded during the invisible part of the cycle.

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
    let across = current.dir.perp();
    let length = (4.4 + 1.1 * current.strength) * zoom;
    let amplitude = 0.85 * zoom;
    // Sample the shared sine once per frame so consecutive points are at most
    // one world pixel apart, even at 2×. Duplicate raster points blend once.
    const SAMPLES: usize = 24;
    let offsets: [Vec2; SAMPLES] = std::array::from_fn(|i| {
        let t = i as f32 / (SAMPLES - 1) as f32;
        across * ((t - 0.5) * length)
            + current.dir * ((t * std::f32::consts::TAU).sin() * amplitude)
    });
    let pixels = pb.pixels_mut();
    current.visit_crests(level.seed, cam, Vec2::new(w as f32, h as f32), zoom, |centre| {
        let mut painted = [(0, 0); SAMPLES];
        let mut count = 0;
        for offset in offsets {
            let p = centre + offset;
            let (x, y) = (p.x.round() as i32, p.y.round() as i32);
            if x < 0 || y < 0 || x as usize >= w || y as usize >= h
                || painted[..count].contains(&(x, y))
            {
                continue;
            }
            let pixel_pos = Vec2::new(x as f32, y as f32);
            if pixel_pos.dist_sq(rat) <= clear_sq
                || !level.tiles.get_at(cam + pixel_pos).is_open()
            {
                continue;
            }
            let pixel = &mut pixels[y as usize * w + x as usize];
            *pixel = mix(*pixel, CYAN, current.alpha);
            painted[count] = (x, y);
            count += 1;
        }
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
}
