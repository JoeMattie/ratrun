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
    ///
    /// The cell is first clamped into the grid so a goal outside the arena
    /// (e.g. a player pushed past the edge) can never produce an out-of-range
    /// index; `None` means no free cell was found near it, in which case the
    /// caller must leave the field alone instead of computing toward it.
    fn nearest_free(&self, c: (i32, i32)) -> Option<(i32, i32)> {
        let cx = c.0.clamp(0, self.cols - 1);
        let cy = c.1.clamp(0, self.rows - 1);
        if !self.is_blocked(cx, cy) {
            return Some((cx, cy));
        }
        for r in 1..12i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    if !self.is_blocked(cx + dx, cy + dy) {
                        return Some((cx + dx, cy + dy));
                    }
                }
            }
        }
        None
    }
}

/// A diagonal step `(dx, dy)` is only legal when it does not squeeze between
/// two blocked orthogonal neighbours (no cutting a wall corner). Orthogonal
/// steps are always legal. BFS and direction derivation share this rule so
/// their notion of "reachable neighbour" cannot disagree.
#[inline]
fn diagonal_ok(g: &NavGrid, cx: i32, cy: i32, dx: i32, dy: i32) -> bool {
    dx == 0 || dy == 0 || (!g.is_blocked(cx + dx, cy) && !g.is_blocked(cx, cy + dy))
}

#[derive(bevy_ecs::resource::Resource)]
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
        let Some(gc) = g.nearest_free(g.cell_of(goal_pos)) else {
            // No free goal cell anywhere near the target: keep the previous
            // field (routing toward a stale goal beats clearing it to ZERO).
            return;
        };
        if self.goal == Some(gc) {
            return;
        }
        self.goal = Some(gc);
        self.compute(g, gc);
    }

    fn compute(&mut self, g: &NavGrid, goal: (i32, i32)) {
        // Defensive: an out-of-grid or blocked goal has no valid cost field.
        if goal.0 < 0 || goal.1 < 0 || goal.0 >= self.cols || goal.1 >= self.rows {
            return;
        }
        if g.is_blocked(goal.0, goal.1) {
            return;
        }
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
                if g.is_blocked(nx, ny) || !diagonal_ok(g, cx, cy, dx, dy) {
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
        // neighbors weighted by how much closer they are. Blocked cells get an
        // escape direction instead, so an enemy that wall resolution left in
        // one steers back onto the field rather than straight at the player.
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let i = self.idx(cx, cy);
                if g.is_blocked(cx, cy) {
                    let esc = self.escape_dir(g, cx, cy);
                    self.dir[i] = esc;
                    continue;
                }
                let cc = self.cost[i];
                if cc == u32::MAX {
                    self.dir[i] = Vec2::ZERO;
                    continue;
                }
                let mut d = Vec2::ZERO;
                // Cheapest lower-cost neighbour, used when the weighted blend
                // cancels out (a symmetric corridor sums to exactly ZERO —
                // that cell would otherwise fall back to charging the wall).
                let mut best: Option<(u32, Vec2)> = None;
                for &(dx, dy) in OFFS.iter() {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if g.is_blocked(nx, ny) || !diagonal_ok(g, cx, cy, dx, dy) {
                        continue;
                    }
                    let nc = self.cost[self.idx(nx, ny)];
                    if nc < cc {
                        let step = Vec2::new(dx as f32, dy as f32).normalized();
                        d += step * (cc - nc) as f32;
                        if best.is_none_or(|(bc, _)| nc < bc) {
                            best = Some((nc, step));
                        }
                    }
                }
                let blended = d.normalized();
                // Only trust the blend when it actually steps downhill. Between
                // two props, opposing neighbours cancel one axis, so a cell can
                // point sideways at an equal-cost cell and the pair ping-pongs
                // an enemy forever. The cheapest-neighbour fallback always
                // lowers the cost, so progress is monotone.
                let tx = cx + blended.x.round() as i32;
                let ty = cy + blended.y.round() as i32;
                let downhill = blended.len_sq() > 0.0
                    && !g.is_blocked(tx, ty)
                    && self.cost[self.idx(tx, ty)] < cc;
                self.dir[i] = if downhill {
                    blended
                } else {
                    best.map_or(Vec2::ZERO, |(_, step)| step)
                };
            }
        }
    }

    /// Direction out of a blocked cell: toward its lowest-cost free
    /// 8-neighbour (any free neighbour at all if none is reachable from the
    /// goal). Corner-cutting rules do not apply here — the cell is already
    /// inside an inflated obstacle, so any free neighbour is an improvement.
    ///
    /// Inflated obstacles are up to two cells thick (props are walled with a
    /// 5 px margin, the grid is 16 px), so "stuck" cells can be marooned from
    /// free space entirely; those search a wider ring for the nearest one.
    fn escape_dir(&self, g: &NavGrid, cx: i32, cy: i32) -> Vec2 {
        let mut best: Option<(u32, Vec2)> = None;
        let mut any_free: Option<Vec2> = None;
        for &(dx, dy) in OFFS.iter() {
            let (nx, ny) = (cx + dx, cy + dy);
            if g.is_blocked(nx, ny) {
                continue;
            }
            let v = Vec2::new(dx as f32, dy as f32).normalized();
            let nc = self.cost[self.idx(nx, ny)];
            if nc == u32::MAX {
                any_free.get_or_insert(v);
                continue;
            }
            if best.is_none_or(|(bc, _)| nc < bc) {
                best = Some((nc, v));
            }
        }
        if let Some((_, v)) = best {
            return v;
        }
        if let Some(v) = any_free {
            return v;
        }
        for r in 2..=6i32 {
            let mut ring: Option<(u32, Vec2)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // ring only
                    }
                    if g.is_blocked(cx + dx, cy + dy) {
                        continue;
                    }
                    let v = Vec2::new(dx as f32, dy as f32).normalized();
                    let nc = self.cost[self.idx(cx + dx, cy + dy)];
                    if ring.is_none_or(|(bc, _)| nc < bc) {
                        ring = Some((nc, v));
                    }
                }
            }
            if let Some((_, v)) = ring {
                return v;
            }
        }
        Vec2::ZERO
    }

    #[inline]
    fn idx(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    /// Routing direction at a world position. Non-zero for every reachable
    /// free cell and for blocked cells near open ground (escape direction).
    /// ZERO only inside the goal cell or off-grid — the caller then falls back
    /// to steering straight at the player.
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
    use crate::game::level::{Level, Theme};

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
        assert!(
            d.len() > 0.1,
            "left-side cell should have a route around the wall"
        );
        // And it should not point straight through the wall (pure +x).
        assert!(
            d.x < 0.98,
            "flow should bend around, not push into the wall"
        );
    }

    /// Any in-bounds free cell in the 8-neighbourhood?
    fn free_8_neighbour(g: &NavGrid, cx: i32, cy: i32) -> bool {
        OFFS.iter().any(|&(dx, dy)| !g.is_blocked(cx + dx, cy + dy))
    }

    #[test]
    fn blocked_cells_escape_toward_the_cheapest_free_neighbour() {
        let wall = Rectf::new(100.0, 40.0, 16.0, 66.0); // a Barrier prop
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[wall], 16.0);
        let mut f = FlowField::new(&g);
        f.update(&g, Vec2::new(200.0, 73.0));
        let mut checked = 0;
        for cy in 0..g.rows {
            for cx in 0..g.cols {
                if !g.is_blocked(cx, cy) || !free_8_neighbour(&g, cx, cy) {
                    continue;
                }
                let d = f.dir_at(Vec2::new(
                    cx as f32 * g.cell + 1.0,
                    cy as f32 * g.cell + 1.0,
                ));
                assert!(
                    d.len() > 0.5,
                    "blocked cell ({cx},{cy}) has no escape direction"
                );
                // The escape direction must land on a free neighbour and that
                // neighbour must be one of the cheapest free ones.
                let (ex, ey) = (d.x.round() as i32, d.y.round() as i32);
                assert!(
                    !g.is_blocked(cx + ex, cy + ey),
                    "({cx},{cy}) escapes into another blocked cell"
                );
                let best = OFFS
                    .iter()
                    .filter(|&&(dx, dy)| !g.is_blocked(cx + dx, cy + dy))
                    .fold(u32::MAX, |m, &(dx, dy)| {
                        m.min(f.cost[f.idx(cx + dx, cy + dy)])
                    });
                assert_eq!(f.cost[f.idx(cx + ex, cy + ey)], best);
                checked += 1;
            }
        }
        assert!(checked > 0, "the barrier should block some cells");
    }

    #[test]
    fn an_agent_parked_against_a_prop_walks_back_onto_the_field() {
        // A tall barrier with the player on its right and an enemy parked a
        // few px off its left face — the exact spot that used to report ZERO
        // and therefore sent the enemy straight into the prop.
        let wall = Rectf::new(100.0, 40.0, 16.0, 66.0);
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[wall], 16.0);
        let mut f = FlowField::new(&g);
        let player = Vec2::new(200.0, 73.0);
        f.update(&g, player);
        let start = Vec2::new(96.0, 73.0);
        let (cx, cy) = g.cell_of(start);
        assert!(g.is_blocked(cx, cy), "probe should sit in an inflated cell");
        assert!(
            f.dir_at(start).len() > 0.5,
            "a blocked cell with open ground nearby must have a direction"
        );

        // Walk a point agent (wall resolution is the world's job): it must
        // leave the blocked halo and make progress toward the player.
        let mut p = start;
        let mut escaped_at = None;
        let goal_cell = g.cell_of(player);
        for step in 0..600 {
            let d = f.dir_at(p);
            if d.len() <= 0.5 {
                // ZERO is only legal in the goal cell itself (the caller then
                // steers straight at the player over ground it can see).
                assert_eq!(g.cell_of(p), goal_cell, "no flow at {p:?} on step {step}");
                break;
            }
            p += d * 2.0;
            let (cx, cy) = g.cell_of(p);
            if escaped_at.is_none() && !g.is_blocked(cx, cy) {
                escaped_at = Some(step);
            }
        }
        assert!(escaped_at.is_some(), "agent never left the prop halo");
        assert!(
            p.dist(player) < start.dist(player),
            "agent made no progress toward the player: {p:?}"
        );
    }

    #[test]
    fn every_reachable_free_cell_has_a_direction() {
        // BFS and direction derivation must agree on the corner rule: with the
        // same diagonal eligibility, any cell the BFS reached has a legal
        // lower-cost neighbour to point at.
        for theme in Theme::all() {
            let level = Level::new(theme);
            let g = &level.nav;
            let mut f = FlowField::new(g);
            f.update(g, level.arena * 0.5);
            for cy in 0..g.rows {
                for cx in 0..g.cols {
                    let i = f.idx(cx, cy);
                    if g.is_blocked(cx, cy) || f.cost[i] == u32::MAX || f.cost[i] == 0 {
                        continue;
                    }
                    assert!(
                        f.dir[i].len() > 0.1,
                        "{theme:?}: cell ({cx},{cy}) has cost {} but no direction",
                        f.cost[i]
                    );
                }
            }
        }
    }

    #[test]
    fn free_cell_directions_always_step_downhill() {
        // Monotone progress is what stops an enemy ping-ponging between two
        // equal-cost cells: the blend is only kept when the cell it steers
        // into is strictly cheaper, otherwise the cheapest neighbour wins.
        for theme in Theme::all() {
            let level = Level::new(theme);
            let g = &level.nav;
            let mut f = FlowField::new(g);
            f.update(g, level.arena * 0.5);
            for cy in 0..g.rows {
                for cx in 0..g.cols {
                    let i = f.idx(cx, cy);
                    if g.is_blocked(cx, cy) || f.cost[i] == u32::MAX || f.cost[i] == 0 {
                        continue;
                    }
                    let d = f.dir[i];
                    let (tx, ty) = (cx + d.x.round() as i32, cy + d.y.round() as i32);
                    assert!(
                        (tx - cx).abs() <= 1 && (ty - cy).abs() <= 1 && (tx, ty) != (cx, cy),
                        "{theme:?}: ({cx},{cy}) points outside its neighbourhood: {d:?}"
                    );
                    assert!(
                        !g.is_blocked(tx, ty),
                        "{theme:?}: ({cx},{cy}) points into a blocked cell"
                    );
                    assert!(
                        f.cost[f.idx(tx, ty)] < f.cost[i],
                        "{theme:?}: ({cx},{cy}) cost {} does not step down into ({tx},{ty}) cost {}",
                        f.cost[i],
                        f.cost[f.idx(tx, ty)]
                    );
                }
            }
        }
    }

    #[test]
    fn points_just_outside_walls_always_have_a_route() {
        for theme in Theme::all() {
            let level = Level::new(theme);
            let g = &level.nav;
            let mut f = FlowField::new(g);
            f.update(g, level.arena * 0.5);
            for w in &level.walls {
                let steps = 8;
                for k in 0..=steps {
                    let t = k as f32 / steps as f32;
                    let px = w.x + w.w * t;
                    let py = w.y + w.h * t;
                    for off in [2.0f32, 5.0, 9.0, 13.0] {
                        let probes = [
                            Vec2::new(px, w.y - off),
                            Vec2::new(px, w.bottom() + off),
                            Vec2::new(w.x - off, py),
                            Vec2::new(w.right() + off, py),
                        ];
                        for p in probes {
                            let (cx, cy) = g.cell_of(p);
                            if cx < 0 || cy < 0 || cx >= g.cols || cy >= g.rows {
                                continue;
                            }
                            if !free_8_neighbour(g, cx, cy) {
                                continue; // no free neighbour to steer toward
                            }
                            let d = f.dir_at(p);
                            assert!(
                                d.len() > 0.1,
                                "{theme:?}: ZERO flow at {p:?} (cell {cx},{cy}) beside {w:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn goal_outside_the_arena_is_clamped_not_indexed() {
        let g = NavGrid::build(Vec2::new(320.0, 320.0), &[], 16.0);
        let mut f = FlowField::new(&g);
        // Used to index `cost` out of range and panic.
        f.update(&g, Vec2::new(-5000.0, -5000.0));
        f.update(&g, Vec2::new(9000.0, 9000.0));
        f.update(&g, Vec2::new(160.0, -9000.0));
        assert!(f.goal.is_some());
        assert!(f.dir_at(Vec2::new(60.0, 120.0)).len() > 0.1);
    }

    #[test]
    fn no_free_goal_cell_leaves_the_field_alone() {
        // Fully walled arena: nearest_free finds nothing, so the field must
        // stay empty instead of computing toward an invalid goal cell.
        let g = NavGrid::build(
            Vec2::new(64.0, 64.0),
            &[Rectf::new(0.0, 0.0, 64.0, 64.0)],
            16.0,
        );
        let mut f = FlowField::new(&g);
        f.update(&g, Vec2::new(32.0, 32.0));
        f.update(&g, Vec2::new(-999.0, 4321.0));
        assert_eq!(f.goal, None);
        assert!(f.cost.iter().all(|&c| c == u32::MAX));
        assert_eq!(f.dir_at(Vec2::new(32.0, 32.0)), Vec2::ZERO);
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
