//! Collision helpers: circle/rect resolution and a uniform spatial grid so
//! the many-bullets-vs-many-enemies test stays near O(n).

use crate::math::Vec2;

use super::level::Rectf;

pub fn circles_overlap(a: Vec2, ar: f32, b: Vec2, br: f32) -> bool {
    let r = ar + br;
    a.dist_sq(b) <= r * r
}

/// Swept circle test: does the segment `a -> b` (a bullet of radius `ar`)
/// come within `ar + br` of `c`? Catches fast projectiles that jump over a
/// target between two ticks.
pub fn segment_circle_hit(a: Vec2, b: Vec2, ar: f32, c: Vec2, br: f32) -> bool {
    let ab = b - a;
    let len_sq = ab.len_sq();
    let t = if len_sq > 1e-9 {
        ((c - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = a + ab * t;
    let r = ar + br;
    closest.dist_sq(c) <= r * r
}

/// Push a circle out of an axis-aligned rectangle (if overlapping).
pub fn resolve_circle_rect(pos: Vec2, r: f32, rect: &Rectf) -> Vec2 {
    let cx = pos.x.clamp(rect.x, rect.right());
    let cy = pos.y.clamp(rect.y, rect.bottom());
    let closest = Vec2::new(cx, cy);
    let delta = pos - closest;
    let d2 = delta.len_sq();
    if d2 >= r * r {
        return pos;
    }
    if d2 > 1e-6 {
        let d = d2.sqrt();
        pos + delta * ((r - d) / d)
    } else {
        // Center is inside the rect: push out along the nearest edge.
        let left = (pos.x - rect.x).abs();
        let right = (rect.right() - pos.x).abs();
        let top = (pos.y - rect.y).abs();
        let bottom = (rect.bottom() - pos.y).abs();
        let m = left.min(right).min(top).min(bottom);
        if m == left {
            Vec2::new(rect.x - r, pos.y)
        } else if m == right {
            Vec2::new(rect.right() + r, pos.y)
        } else if m == top {
            Vec2::new(pos.x, rect.y - r)
        } else {
            Vec2::new(pos.x, rect.bottom() + r)
        }
    }
}

/// First intersection of the segment `a -> b` with `rect` expanded by `r`,
/// as a parameter in `[0, 1]` (0 = already touching/overlapping at `a`).
/// Slab/Liang-Barsky clipping so a fast projectile cannot skip a thin wall.
pub fn segment_rect_hit(a: Vec2, b: Vec2, r: f32, rect: &Rectf) -> Option<f32> {
    let min = Vec2::new(rect.x - r, rect.y - r);
    let max = Vec2::new(rect.right() + r, rect.bottom() + r);
    let d = b - a;
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    let planes = [
        (-d.x, a.x - min.x),
        (d.x, max.x - a.x),
        (-d.y, a.y - min.y),
        (d.y, max.y - a.y),
    ];
    for (p, q) in planes {
        if p.abs() < 1e-9 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return None;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    Some(t0)
}

/// Uniform grid over enemy positions. The world keeps one and reuses it:
/// `reset` clears buckets in place (keeping their capacity) and `fill`
/// re-populates them, so a frame does no bucket allocation.
#[derive(bevy_ecs::resource::Resource)]
pub struct SpatialGrid {
    cell: f32,
    cols: i32,
    rows: i32,
    buckets: Vec<Vec<u32>>,
}

impl Default for SpatialGrid {
    fn default() -> Self {
        SpatialGrid {
            cell: 16.0,
            cols: 0,
            rows: 0,
            buckets: Vec::new(),
        }
    }
}

impl SpatialGrid {
    pub fn new(arena: Vec2, cell: f32) -> Self {
        let mut g = SpatialGrid {
            cell,
            cols: 0,
            rows: 0,
            buckets: Vec::new(),
        };
        g.reset(arena, cell);
        g
    }

    /// Convenience constructor: allocate a fresh grid and fill it.
    pub fn build(positions: impl Iterator<Item = Vec2>, arena: Vec2, cell: f32) -> Self {
        let mut g = SpatialGrid::new(arena, cell);
        g.fill(positions);
        g
    }

    /// Resize to an arena and clear every bucket, keeping bucket allocations
    /// when the arena/cell size is unchanged (the common case each frame).
    pub fn reset(&mut self, arena: Vec2, cell: f32) {
        self.cell = cell;
        self.cols = (arena.x / cell).ceil() as i32 + 1;
        self.rows = (arena.y / cell).ceil() as i32 + 1;
        let n = (self.cols * self.rows) as usize;
        if self.buckets.len() == n {
            for b in &mut self.buckets {
                b.clear();
            }
        } else {
            self.buckets.clear();
            self.buckets.resize(n, Vec::new());
        }
    }

    /// Insert entity indices by position. Call after `reset`.
    pub fn fill(&mut self, positions: impl Iterator<Item = Vec2>) {
        for (i, p) in positions.enumerate() {
            let cx = (p.x / self.cell) as i32;
            let cy = (p.y / self.cell) as i32;
            if cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows {
                self.buckets[(cy * self.cols + cx) as usize].push(i as u32);
            }
        }
    }

    /// Indices of entities in the 3×3 block of cells around `p`.
    pub fn neighbors(&self, p: Vec2, out: &mut Vec<u32>) {
        self.neighbors_within(p, self.cell, out);
    }

    /// Indices of entities in every cell within `reach` pixels of `p`.
    /// `reach` must cover the query circle plus the largest target radius,
    /// otherwise an overlap across a cell boundary is missed.
    pub fn neighbors_within(&self, p: Vec2, reach: f32, out: &mut Vec<u32>) {
        out.clear();
        let span = (reach / self.cell).ceil().max(1.0) as i32;
        let cx = (p.x / self.cell) as i32;
        let cy = (p.y / self.cell) as i32;
        for ny in (cy - span)..=(cy + span) {
            for nx in (cx - span)..=(cx + span) {
                if nx >= 0 && ny >= 0 && nx < self.cols && ny < self.rows {
                    out.extend_from_slice(&self.buckets[(ny * self.cols + nx) as usize]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_basic() {
        assert!(circles_overlap(Vec2::ZERO, 2.0, Vec2::new(3.0, 0.0), 2.0));
        assert!(!circles_overlap(Vec2::ZERO, 1.0, Vec2::new(5.0, 0.0), 1.0));
    }

    #[test]
    fn swept_circle_catches_a_jumped_target() {
        let a = Vec2::new(80.0, 100.0);
        let b = Vec2::new(92.0, 100.0);
        let target = Vec2::new(86.0, 100.0);
        // Both endpoints are outside the 4.5 px hit radius, but the path hits.
        assert!(!circles_overlap(a, 2.0, target, 2.5));
        assert!(!circles_overlap(b, 2.0, target, 2.5));
        assert!(segment_circle_hit(a, b, 2.0, target, 2.5));
        assert!(!segment_circle_hit(a, b, 2.0, Vec2::new(86.0, 120.0), 2.5));
    }

    #[test]
    fn segment_rect_hit_catches_a_thin_wall() {
        let wall = Rectf::new(20.0, -10.0, 1.0, 20.0);
        // A 12 px step that lands beyond the thin wall still reports a hit on
        // the near face (expanded by the bullet radius).
        let t = segment_rect_hit(Vec2::new(10.0, 0.0), Vec2::new(22.0, 0.0), 1.0, &wall)
            .expect("segment should hit the wall");
        assert!((t - 0.75).abs() < 1e-3, "hit t was {t}");
        assert!(segment_rect_hit(Vec2::new(10.0, 50.0), Vec2::new(22.0, 50.0), 1.0, &wall).is_none());
    }

    #[test]
    fn resolve_pushes_out() {
        let rect = Rectf::new(0.0, 0.0, 10.0, 10.0);
        let p = Vec2::new(-1.0, 5.0); // just left of the rect, radius overlaps
        let fixed = resolve_circle_rect(p, 3.0, &rect);
        assert!(fixed.x <= -3.0 + 0.01);
    }

    #[test]
    fn grid_finds_neighbors() {
        let pts = [
            Vec2::new(5.0, 5.0),
            Vec2::new(7.0, 6.0),
            Vec2::new(200.0, 200.0),
        ];
        let grid = SpatialGrid::build(pts.iter().copied(), Vec2::new(256.0, 256.0), 16.0);
        let mut out = Vec::new();
        grid.neighbors(Vec2::new(6.0, 6.0), &mut out);
        assert!(out.contains(&0) && out.contains(&1));
        assert!(!out.contains(&2));
    }

    #[test]
    fn reset_clears_but_keeps_bucket_capacity() {
        let arena = Vec2::new(256.0, 256.0);
        let mut grid = SpatialGrid::new(arena, 16.0);
        grid.fill(std::iter::once(Vec2::new(5.0, 5.0)));
        let cap = grid.buckets[0].capacity();
        assert!(cap > 0);
        grid.reset(arena, 16.0);
        assert_eq!(grid.buckets.len(), 17 * 17);
        assert!(grid.buckets.iter().all(|b| b.is_empty()));
        assert_eq!(grid.buckets[0].capacity(), cap, "bucket allocation was dropped");
    }

    #[test]
    fn wide_query_covers_more_than_one_cell() {
        let pts = [Vec2::new(15.9, 20.0), Vec2::new(32.5, 20.0)];
        let grid = SpatialGrid::build(pts.iter().copied(), Vec2::new(256.0, 256.0), 16.0);
        let mut out = Vec::new();
        grid.neighbors(Vec2::new(15.9, 20.0), &mut out);
        assert!(!out.contains(&1), "3x3 query should not reach two cells away");
        grid.neighbors_within(Vec2::new(15.9, 20.0), 17.0, &mut out);
        assert!(out.contains(&1), "wide query should reach two cells away");
    }
}
