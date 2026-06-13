//! Collision helpers: circle/rect resolution and a uniform spatial grid so
//! the many-bullets-vs-many-enemies test stays near O(n).

use crate::math::Vec2;

use super::level::Rectf;

pub fn circles_overlap(a: Vec2, ar: f32, b: Vec2, br: f32) -> bool {
    let r = ar + br;
    a.dist_sq(b) <= r * r
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

/// Uniform grid over enemy positions, rebuilt each frame.
pub struct SpatialGrid {
    cell: f32,
    cols: i32,
    rows: i32,
    buckets: Vec<Vec<u32>>,
}

impl SpatialGrid {
    pub fn build(positions: impl Iterator<Item = Vec2>, arena: Vec2, cell: f32) -> Self {
        let cols = (arena.x / cell).ceil() as i32 + 1;
        let rows = (arena.y / cell).ceil() as i32 + 1;
        let mut buckets = vec![Vec::new(); (cols * rows) as usize];
        for (i, p) in positions.enumerate() {
            let cx = (p.x / cell) as i32;
            let cy = (p.y / cell) as i32;
            if cx >= 0 && cy >= 0 && cx < cols && cy < rows {
                buckets[(cy * cols + cx) as usize].push(i as u32);
            }
        }
        SpatialGrid {
            cell,
            cols,
            rows,
            buckets,
        }
    }

    /// Indices of entities in the 3×3 block of cells around `p`.
    pub fn neighbors(&self, p: Vec2, out: &mut Vec<u32>) {
        out.clear();
        let cx = (p.x / self.cell) as i32;
        let cy = (p.y / self.cell) as i32;
        for ny in (cy - 1)..=(cy + 1) {
            for nx in (cx - 1)..=(cx + 1) {
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
    fn resolve_pushes_out() {
        let rect = Rectf::new(0.0, 0.0, 10.0, 10.0);
        let p = Vec2::new(-1.0, 5.0); // just left of the rect, radius overlaps
        let fixed = resolve_circle_rect(p, 3.0, &rect);
        assert!(fixed.x <= -3.0 + 0.01);
    }

    #[test]
    fn grid_finds_neighbors() {
        let pts = vec![Vec2::new(5.0, 5.0), Vec2::new(7.0, 6.0), Vec2::new(200.0, 200.0)];
        let grid = SpatialGrid::build(pts.iter().copied(), Vec2::new(256.0, 256.0), 16.0);
        let mut out = Vec::new();
        grid.neighbors(Vec2::new(6.0, 6.0), &mut out);
        assert!(out.contains(&0) && out.contains(&1));
        assert!(!out.contains(&2));
    }
}
