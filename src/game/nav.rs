//! Flow-field pathfinding.
//!
//! For a horde all chasing one target, per-agent A* is wasteful. Instead we
//! build one **flow field**: a single BFS over a coarse nav grid from the
//! player's cell produces a direction vector per cell that routes around
//! walls. Every enemy then samples its cell in O(1). The BFS is only
//! recomputed when the player crosses into a new cell, so it's near-free.

use std::collections::VecDeque;

use crate::math::Vec2;

use super::level::Rectf;

/// 8-connected neighborhood (orthogonal + diagonal).
const OFFS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

pub struct NavGrid {
    pub cols: i32,
    pub rows: i32,
    pub cell: f32,
    blocked: Vec<bool>,
}

impl NavGrid {
    pub fn build(arena: Vec2, walls: &[Rectf], cell: f32) -> NavGrid {
        let cols = (arena.x / cell).ceil() as i32;
        let rows = (arena.y / cell).ceil() as i32;
        let mut blocked = vec![false; (cols * rows) as usize];
        // Inflate walls slightly so flow lines keep a little clearance.
        let m = 5.0;
        for cy in 0..rows {
            for cx in 0..cols {
                let x = cx as f32 * cell;
                let y = cy as f32 * cell;
                let b = walls.iter().any(|w| {
                    x + cell > w.x - m
                        && x < w.right() + m
                        && y + cell > w.y - m
                        && y < w.bottom() + m
                });
                blocked[(cy * cols + cx) as usize] = b;
            }
        }
        NavGrid {
            cols,
            rows,
            cell,
            blocked,
        }
    }

    #[inline]
    fn idx(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    #[inline]
    pub fn is_blocked(&self, cx: i32, cy: i32) -> bool {
        cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows || self.blocked[self.idx(cx, cy)]
    }

    #[inline]
    pub fn cell_of(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell) as i32, (p.y / self.cell) as i32)
    }

    /// Nearest non-blocked cell to `c` (the goal may sit in/near a wall).
    fn nearest_free(&self, c: (i32, i32)) -> (i32, i32) {
        if !self.is_blocked(c.0, c.1) {
            return c;
        }
        for r in 1..12i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    if !self.is_blocked(c.0 + dx, c.1 + dy) {
                        return (c.0 + dx, c.1 + dy);
                    }
                }
            }
        }
        c
    }
}

pub struct FlowField {
    cols: i32,
    rows: i32,
    cell: f32,
    goal: Option<(i32, i32)>,
    cost: Vec<u32>,
    dir: Vec<Vec2>,
    queue: VecDeque<(i32, i32)>,
}

impl FlowField {
    pub fn new(g: &NavGrid) -> FlowField {
        let n = (g.cols * g.rows) as usize;
        FlowField {
            cols: g.cols,
            rows: g.rows,
            cell: g.cell,
            goal: None,
            cost: vec![u32::MAX; n],
            dir: vec![Vec2::ZERO; n],
            queue: VecDeque::new(),
        }
    }

    /// Recompute the field toward `goal_pos`, but only if the goal moved to a
    /// different cell (the field depends solely on goal cell + static walls).
    pub fn update(&mut self, g: &NavGrid, goal_pos: Vec2) {
        let gc = g.nearest_free(g.cell_of(goal_pos));
        if self.goal == Some(gc) {
            return;
        }
        self.goal = Some(gc);
        self.compute(g, gc);
    }

    fn compute(&mut self, g: &NavGrid, goal: (i32, i32)) {
        for c in self.cost.iter_mut() {
            *c = u32::MAX;
        }
        self.queue.clear();
        let gi = self.idx(goal.0, goal.1);
        self.cost[gi] = 0;
        self.queue.push_back(goal);

        // BFS integration field (cost = steps to goal).
        while let Some((cx, cy)) = self.queue.pop_front() {
            let cc = self.cost[self.idx(cx, cy)];
            for &(dx, dy) in OFFS.iter() {
                let (nx, ny) = (cx + dx, cy + dy);
                if g.is_blocked(nx, ny) {
                    continue;
                }
                // No diagonal corner-cutting through wall corners.
                if dx != 0 && dy != 0 && (g.is_blocked(cx + dx, cy) || g.is_blocked(cx, cy + dy)) {
                    continue;
                }
                let ni = self.idx(nx, ny);
                if self.cost[ni] == u32::MAX {
                    self.cost[ni] = cc + 1;
                    self.queue.push_back((nx, ny));
                }
            }
        }

        // Derive a smooth direction per cell: blend toward all lower-cost
        // neighbors weighted by how much closer they are.
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let i = self.idx(cx, cy);
                if self.cost[i] == u32::MAX {
                    self.dir[i] = Vec2::ZERO;
                    continue;
                }
                let cc = self.cost[i];
                let mut d = Vec2::ZERO;
                for &(dx, dy) in OFFS.iter() {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if g.is_blocked(nx, ny) {
                        continue;
                    }
                    let nc = self.cost[self.idx(nx, ny)];
                    if nc < cc {
                        let w = (cc - nc) as f32;
                        d += Vec2::new(dx as f32, dy as f32).normalized() * w;
                    }
                }
                self.dir[i] = d.normalized();
            }
        }
    }

    #[inline]
    fn idx(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    /// Routing direction at a world position (ZERO at/inside the goal cell or
    /// off-grid → caller falls back to steering straight at the player).
    pub fn dir_at(&self, p: Vec2) -> Vec2 {
        let cx = (p.x / self.cell) as i32;
        let cy = (p.y / self.cell) as i32;
        if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
            return Vec2::ZERO;
        }
        self.dir[self.idx(cx, cy)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_points_toward_goal_in_open_field() {
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[], 16.0);
        let mut f = FlowField::new(&g);
        let goal = Vec2::new(160.0, 160.0);
        f.update(&g, goal);
        // From a corner, the flow should head roughly toward the center.
        let from = Vec2::new(24.0, 24.0);
        let d = f.dir_at(from);
        let to_goal = (goal - from).normalized();
        assert!(d.dot(to_goal) > 0.5, "flow should aim at the goal: {d:?}");
    }

    #[test]
    fn flow_routes_around_a_wall() {
        // A vertical wall splits the arena; goal on the right, sample on left.
        let wall = Rectf::new(150.0, 0.0, 20.0, 240.0); // gap at the bottom
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[wall], 16.0);
        let mut f = FlowField::new(&g);
        let goal = Vec2::new(260.0, 120.0);
        f.update(&g, goal);
        // A cell on the left, level with the wall, must have a finite route
        // (nonzero direction) — i.e. it found a path around the gap.
        let from = Vec2::new(60.0, 120.0);
        let d = f.dir_at(from);
        assert!(d.len() > 0.1, "left-side cell should have a route around the wall");
        // And it should not point straight through the wall (pure +x).
        assert!(d.x < 0.98, "flow should bend around, not push into the wall");
    }

    #[test]
    fn recompute_only_on_cell_change() {
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[], 16.0);
        let mut f = FlowField::new(&g);
        f.update(&g, Vec2::new(100.0, 100.0));
        let g0 = f.goal;
        f.update(&g, Vec2::new(104.0, 102.0)); // same cell
        assert_eq!(f.goal, g0);
        f.update(&g, Vec2::new(200.0, 200.0)); // different cell
        assert_ne!(f.goal, g0);
    }
}
