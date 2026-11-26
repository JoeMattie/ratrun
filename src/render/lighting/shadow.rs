//! Compact, world-aligned occluders and finite visibility queries.

use super::engine::Blend;
use super::{LayerMask, Light, ShadowSoftness};
use crate::game::art_walls::{one_way_gate_rect, wall_colliders, WallKind};
use crate::game::collision::{TileKind, TileMap};
use crate::game::level::{Level, LightZoneKind, PropKind, Rectf, Theme};
use crate::math::Vec2;

const SUB: f32 = 4.0;
pub(super) const RAYS: usize = 256;

/// A light's unoccluded weight and the portion that the grid received.
#[derive(Clone, Copy, Default)]
struct EdgeTap {
    weight: f32,
    visible: f32,
    coverage: f32,
    shaft: f32,
    shaft_visible: f32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct EdgeDelta {
    pub mul: [i16; 3],
    pub add: [i16; 3],
    pub air: [i16; 3],
}

/// Hard, Coarse and Compact keep their coarse light grids. Only quads where
/// one caster straddles the shadow edge get a pixel coverage correction.
/// The correction replaces the interpolated occlusion for that light while
/// the other light, ambient, bounce and vignette still come from the grid.
#[derive(Default)]
pub(super) struct HardShadows {
    size: (usize, usize),
    grid: (usize, usize),
    stride: usize,
    start: Vec2,
    offset: (usize, usize),
    compact: bool,
    taps: Vec<EdgeTap>,
    quads: Vec<bool>,
    shaft_quads: Vec<bool>,
    pixels: Vec<EdgeDelta>,
    blocked: bool,
    lit: bool,
    pub(super) active: bool,
    pub(super) shafts_active: bool,
}

impl HardShadows {
    pub(super) fn reset(
        &mut self,
        size: (usize, usize),
        grid: (usize, usize),
        stride: usize,
        start: Vec2,
        offset: (usize, usize),
        compact: bool,
    ) {
        self.size = size;
        self.grid = grid;
        self.stride = stride;
        self.start = start;
        self.offset = offset;
        self.compact = compact;
        self.active = false;
        self.shafts_active = false;
        self.quads.clear();
        self.shaft_quads.clear();
        if self.enabled() {
            self.quads.resize((grid.0 + 1) * (grid.1 + 1), false);
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.size.0 != 0 && self.size.1 != 0 && self.grid.0 > 1 && self.grid.1 > 1
    }

    pub(super) fn begin_light(&mut self) {
        self.taps.clear();
        self.taps.resize(self.grid.0 * self.grid.1, EdgeTap::default());
        self.blocked = false;
        self.lit = false;
    }

    #[inline]
    pub(super) fn record(&mut self, i: usize, weight: f32, vis: f32, shaft: f32) {
        let visible = weight * vis;
        let blocked = weight - visible;
        self.taps[i] = EdgeTap {
            weight,
            visible,
            coverage: vis,
            shaft,
            shaft_visible: shaft * vis,
        };
        self.blocked |= blocked > 0.0;
        self.lit |= visible > 0.0;
    }

    /// Rerasterise only quads with both lit and occluded taps. Reusing the
    /// incoming light's four weights keeps falloff continuous at their joins.
    pub(super) fn refine(
        &mut self,
        surfaces: &[Surface],
        light: Light,
        xs: std::ops::Range<usize>,
        ys: std::ops::Range<usize>,
        visibility: impl Fn(Vec2) -> f32,
    ) {
        if !self.blocked || !self.lit {
            return;
        }
        let (gw, gh) = self.grid;
        let (w, h) = self.size;
        let step = self.stride as f32;
        let sign = if matches!(light.blend, Blend::Subtract) {
            -1.0
        } else {
            1.0
        };
        let color = [light.color.0, light.color.1, light.color.2].map(f32::from);
        let (r, g, b) = light.shadow_color.rgb;
        let shade = [r, g, b].map(f32::from);
        let mul = color.map(|c| c * light.energy * sign * light.shadow_color.opacity);
        let add = std::array::from_fn::<_, 3, _>(|c| {
            (color[c] * light.scatter * sign - shade[c] * 0.20) * light.shadow_color.opacity
        });
        let peak = mul.into_iter().chain(add).map(f32::abs).fold(1.0, f32::max);
        let enough = 2.0 / peak;
        // Quad (0, 0) starts half a stride before the first sample centre.
        // Clamp each tap just as the composite does at the viewport edges.
        for y in ys.start..(ys.end + 1).min(gh + 1) {
            for x in xs.start..(xs.end + 1).min(gw + 1) {
                let quad = y * (gw + 1) + x;
                let (left, right) = (x.saturating_sub(1), x.min(gw - 1));
                let (top, bottom) = (y.saturating_sub(1) * gw, y.min(gh - 1) * gw);
                let ids = [top + left, top + right, bottom + left, bottom + right];
                let zone = surfaces[ids[0]].zone;
                if ids.into_iter().any(|j| surfaces[j].height > 0.0 || surfaces[j].zone != zone) {
                    continue; // The coarse shade still owns faces and zone boundaries.
                }
                let taps = ids.map(|j| self.taps[j]);
                if !taps.iter().any(|p| p.visible >= enough)
                    || !taps.iter().any(|p| p.weight - p.visible >= enough)
                {
                    continue;
                }
                // If every tap has the same coverage, this quad only varies
                // in light strength. Its ordinary falloff needs no repair.
                let lit = taps.map(|p| p.visible);
                let weights = taps.map(|p| p.weight);
                let shaft = taps.map(|p| p.shaft);
                let shaft_visible = taps.map(|p| p.shaft_visible);
                let has_shaft = shaft.iter().any(|s| *s > 0.0);
                let coverage = taps.map(|p| p.coverage);
                let (min_vis, max_vis) = coverage.into_iter().fold((1.0f32, 0.0f32), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                });
                if max_vis - min_vis < 0.10 {
                    continue;
                }
                let sx = (x * self.stride) as i32 - (self.stride / 2 + self.offset.0) as i32;
                let sy = (y * self.stride) as i32 - (self.stride / 2 + self.offset.1) as i32;
                let base = self.start + Vec2::new((x as f32 - 1.0) * step, (y as f32 - 1.0) * step);
                let first = !self.quads[quad];
                if !self.active {
                    self.pixels.resize(w * h, EdgeDelta::default());
                    self.active = true;
                }
                self.quads[quad] = true;
                if has_shaft {
                    if !self.shafts_active {
                        self.shaft_quads.resize((gw + 1) * (gh + 1), false);
                        self.shafts_active = true;
                    }
                    self.shaft_quads[quad] = true;
                }
                for py in sy.max(0)..(sy + self.stride as i32).min(h as i32) {
                    let dy = (py - sy) as f32;
                    let v = dy / step;
                    for px in sx.max(0)..(sx + self.stride as i32).min(w as i32) {
                        let dx = (px - sx) as f32;
                        let u = dx / step;
                        let blend = |t: [f32; 4]| {
                            let top = t[0] + (t[1] - t[0]) * u;
                            let bottom = t[2] + (t[3] - t[2]) * u;
                            top + (bottom - top) * v
                        };
                        let vis = visibility(base + Vec2::new(dx, dy));
                        let change = if self.compact {
                            let own = usize::from(u >= 0.5) + 2 * usize::from(v >= 0.5);
                            vis * weights[own] - lit[own]
                        } else {
                            vis * blend(weights) - blend(lit)
                        };
                        let air = if has_shaft {
                            vis * blend(shaft) - blend(shaft_visible)
                        } else {
                            0.0
                        };
                        let pixel = &mut self.pixels[py as usize * w + px as usize];
                        if first {
                            *pixel = EdgeDelta::default();
                        }
                        for channel in 0..3 {
                            let mul = (change * mul[channel]).round() as i16;
                            let add = (change * add[channel]).round() as i16;
                            pixel.mul[channel] = pixel.mul[channel].saturating_add(mul);
                            pixel.add[channel] = pixel.add[channel].saturating_add(add);
                            if has_shaft {
                                let shaft = (air * color[channel]).round() as i16;
                                pixel.air[channel] = pixel.air[channel].saturating_add(shaft);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Pixel corrections for one span between sample centres.
    #[inline]
    pub(super) fn row(
        &self,
        col: i32,
        row: i32,
        x: usize,
        y: usize,
        end: usize,
    ) -> Option<&[EdgeDelta]> {
        if !self.active || !self.quads[(row + 1) as usize * (self.grid.0 + 1) + (col + 1) as usize] {
            return None;
        }
        let base = y * self.size.0;
        Some(&self.pixels[base + x..base + end])
    }

    #[inline]
    pub(super) fn shaft(&self, col: i32, row: i32, x: usize, y: usize) -> Option<[i16; 3]> {
        let quad = (row + 1) as usize * (self.grid.0 + 1) + (col + 1) as usize;
        self.shaft_quads[quad].then(|| self.pixels[y * self.size.0 + x].air)
    }
}

#[derive(Clone, Copy, Default)]
struct Tile {
    footprint: u16,
    height: u8,
    layer: u8,
    zone: Option<LightZoneKind>,
    door: bool,
}

#[derive(Default)]
pub(super) struct Geometry {
    cols: i32,
    rows: i32,
    tiles: Vec<Tile>,
    pub(super) revision: u32,
    key: Option<(Theme, u64, u32, usize, usize, i32, i32)>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Surface {
    pub height: f32,
    pub layer: u8,
    pub facing: Vec2,
    pub zone: Option<LightZoneKind>,
}

/// Height and 4×4 outline of a prop within its rectangular tile footprint.
/// Keep height zero for floor art and holes in the mask for thin silhouettes.
fn prop_occluder(kind: PropKind) -> (u8, u16) {
    match kind {
        PropKind::Grate => (3, 0x9999),
        PropKind::ChainCoil => (0, 0),
        PropKind::PipeV => (10, 0x6666),
        PropKind::PipeH => (10, 0x0ff0),
        PropKind::Pallet | PropKind::Toolbox | PropKind::Valve | PropKind::PipeStack => {
            (10, 0xffff)
        }
        PropKind::Tyre | PropKind::TowelRoll | PropKind::TypeBlock => (13, 0xffff),
        PropKind::Table | PropKind::Crate | PropKind::Barrel | PropKind::Barrier => (19, 0xffff),
        PropKind::Counter | PropKind::Console | PropKind::Washer | PropKind::TarpLump => {
            (21, 0xffff)
        }
        PropKind::Tank | PropKind::Clothespin | PropKind::PressIsland | PropKind::PaperReel => {
            (25, 0xffff)
        }
        // Outfall: curved shipwreck rib, heavy ballast crate, tall rounded piling.
        PropKind::OutfallRib => (15, 0x6666),
        PropKind::OutfallBallast => (18, 0xffff),
        PropKind::OutfallPiling => (22, 0x6ff6),
    }
}

impl Geometry {
    pub(super) fn sync(&mut self, level: &Level) {
        let cols = level.tiles.cols();
        let rows = level.tiles.rows();
        let key = (
            level.theme,
            level.seed,
            level.tiles.seed(),
            level.props.len(),
            level.objects.light_zones.len(),
            cols,
            rows,
        );
        if self.key != Some(key) {
            self.key = Some(key);
            self.cols = cols;
            self.rows = rows;
            self.tiles.clear();
            self.tiles.resize((cols * rows) as usize, Tile::default());
            self.revision = self.revision.wrapping_add(1);

            // Passages may run into rooms, and the maze can overlap a public
            // room's bounding rect. Paint smaller-priority zones first.
            for kind in [
                LightZoneKind::Corridor,
                LightZoneKind::Room,
                LightZoneKind::Start,
                LightZoneKind::Arena,
                LightZoneKind::Blackout,
                LightZoneKind::Maze,
            ] {
                for zone in level.objects.light_zones.iter().filter(|z| z.kind == kind) {
                    for (x, y) in zone.rect.cells() {
                        if let Some(i) = self.index(x, y) {
                            self.tiles[i].zone = Some(kind);
                        }
                    }
                }
            }

            let deck = level.trailer.deck;
            for (i, tile) in self.tiles.iter_mut().enumerate() {
                let x = i as i32 % cols;
                let y = i as i32 / cols;
                let rail = level.theme == Theme::Trailer
                    && (y == deck.y - 1 || y == deck.bottom())
                    && (deck.x..deck.right()).contains(&x);
                // Outside the flatbed, collision marks the surrounding air
                // solid. Only the deck, rails and warren actually cast shadows.
                if level.theme == Theme::Trailer
                    && !rail
                    && !deck.contains(x, y)
                    && tile.zone != Some(LightZoneKind::Maze)
                    && !level.trailer.is_empty()
                {
                    continue;
                }
                let TileKind::Wall(kind) = level.tiles.get(x, y) else {
                    continue;
                };
                tile.layer = LayerMask::WALLS.0;
                tile.height = if rail { 9 } else { 24 };
                tile.footprint = match kind {
                    WallKind::Solid => u16::MAX,
                    WallKind::Doorway { .. } => 0, // pixel-wide jambs are below the raster stride
                    WallKind::OneWayDoor { .. } => {
                        tile.door = true;
                        0
                    }
                    WallKind::Maze | WallKind::RatHole { .. } => {
                        let bounds = TileMap::tile_rect(x, y);
                        let mut rects = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
                        let n = wall_colliders(&level.tiles.wall_view(x, y), &mut rects);
                        let mut mask = 0;
                        for rect in &rects[..n] {
                            mask |= footprint(bounds, *rect);
                        }
                        mask
                    }
                };
            }
            for prop in &level.props {
                let (x0, y0, x1, y1) = level.tiles.cell_range(prop.rect);
                let (height, outline) = prop_occluder(prop.kind);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let tile = &mut self.tiles[(y * cols + x) as usize];
                        // A prop that laps onto a wall cell must not replace the
                        // wall's full-height occluder with its own partial, lower
                        // one: light would leak through that cell.
                        if tile.layer == LayerMask::WALLS.0 {
                            continue;
                        }
                        let bounds = TileMap::tile_rect(x, y);
                        tile.footprint = footprint(bounds, prop.rect) & outline;
                        tile.layer = LayerMask::PROPS.0;
                        tile.height = height;
                    }
                }
            }
        }
        // Door changes are sparse. A transition invalidates just the fixture
        // profiles, which are regenerated on demand as they come into view.
        for door in &level.objects.one_way_doors {
            let (x, y) = door.cell;
            let Some(i) = self.index(x, y) else { continue };
            let tile = &mut self.tiles[i];
            if !tile.door {
                continue;
            }
            let footprint = if let Some(dir) = level
                .tiles
                .one_way_dir(x, y)
                .filter(|_| door.open_progress() < 0.5)
            {
                let bounds = TileMap::tile_rect(x, y);
                footprint(bounds, one_way_gate_rect(x, y, dir))
            } else {
                0
            };
            if footprint != tile.footprint {
                tile.footprint = footprint;
                self.revision = self.revision.wrapping_add(1);
            }
        }
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x >= 0 && y >= 0 && x < self.cols && y < self.rows {
            Some((y * self.cols + x) as usize)
        } else {
            None
        }
    }

    #[inline]
    fn lookup(&self, x: i32, y: i32) -> Tile {
        self.index(x >> 2, y >> 2)
            .map_or(Tile::default(), |i| self.tiles[i])
    }

    #[inline]
    pub(super) fn height_at(&self, x: i32, y: i32, layers: LayerMask) -> f32 {
        let tile = self.lookup(x, y);
        let bit = 1 << ((x & 3) + 4 * (y & 3));
        if tile.footprint & bit != 0 && tile.layer & layers.0 != 0 {
            f32::from(tile.height)
        } else {
            0.0
        }
    }

    pub(super) fn surface(&self, p: Vec2) -> Surface {
        let (x, y) = (subcell(p.x), subcell(p.y));
        let tile = self.lookup(x, y);
        let height = self.height_at(x, y, LayerMask::ALL);
        let mut facing = Vec2::ZERO;
        if height > 0.0 {
            // Compare the edges of the tight footprint, with a slight bias
            // towards the south-facing wall bevel present in the sprite.
            facing.x = (self.height_at(x - 1, y, LayerMask::ALL) > 0.0) as i32 as f32
                - (self.height_at(x + 1, y, LayerMask::ALL) > 0.0) as i32 as f32;
            facing.y = (self.height_at(x, y - 1, LayerMask::ALL) > 0.0) as i32 as f32
                - (self.height_at(x, y + 1, LayerMask::ALL) > 0.0) as i32 as f32;
        }
        Surface {
            height,
            layer: if height > 0.0 {
                tile.layer
            } else {
                LayerMask::FLOOR.0
            },
            facing,
            zone: tile.zone,
        }
    }

    /// Trace a 4px grid with DDA, visiting each crossed subcell exactly once.
    /// The straight line can skim a thin jamb or the corner of a prop.
    pub(super) fn trace(
        &self,
        origin: Vec2,
        ray: Vec2,
        limit: f32,
        blocks: impl Fn(f32, f32) -> bool,
        mask: LayerMask,
    ) -> f32 {
        let (mut x, mut y) = (subcell(origin.x), subcell(origin.y));
        let step_x = if ray.x < 0.0 { -1 } else { 1 };
        let step_y = if ray.y < 0.0 { -1 } else { 1 };
        let delta_x = (SUB / ray.x).abs();
        let delta_y = (SUB / ray.y).abs();
        let mut tx = if ray.x == 0.0 {
            f32::INFINITY
        } else {
            ((x + i32::from(step_x > 0)) as f32 * SUB - origin.x) / ray.x
        };
        let mut ty = if ray.y == 0.0 {
            f32::INFINITY
        } else {
            ((y + i32::from(step_y > 0)) as f32 * SUB - origin.y) / ray.y
        };
        let source_tile = (x >> 2, y >> 2);
        let mut left_source = self.height_at(x, y, mask) == 0.0;
        loop {
            let t = tx.min(ty);
            if t >= limit {
                return f32::INFINITY;
            }
            if tx <= ty {
                x += step_x;
                tx += delta_x;
            }
            if ty <= t {
                y += step_y;
                ty += delta_y;
            }
            let height = self.height_at(x, y, mask);
            if !left_source {
                left_source = height == 0.0 || (x >> 2, y >> 2) != source_tile;
            }
            if left_source && height > 0.0 && blocks(t, height) {
                return t;
            }
        }
    }

    /// Reverse a parallel ray from the receiving surface, rising by the sun's
    /// slope. A blocker of height h stops affecting the ground beyond h/slope.
    pub(super) fn sun_visibility(
        &self,
        p: Vec2,
        surface: Surface,
        light: Light,
        ray: Vec2,
        bodies: &[Body],
    ) -> f32 {
        let slope = light.height.max(0.35);
        let max_distance = (32.0 / slope).min(70.0);
        let hit = self.trace(
            p,
            ray,
            max_distance,
            |t, h| h > surface.height + t * slope,
            light.occluders,
        );
        if hit.is_finite() {
            return 0.0;
        }
        if surface.height == 0.0 && light.occluders.0 & LayerMask::BODIES.0 != 0 {
            for body in bodies {
                let delta = body.pos - p;
                let along = delta.dot(ray);
                let across = delta.dot(ray.perp());
                if along > body.radius * 0.60
                    && along < body.height / slope
                    && across.abs() < body.radius
                    && delta.len_sq() > body.radius * body.radius
                {
                    return 0.0;
                }
            }
        }
        1.0
    }
}

#[inline]
fn subcell(p: f32) -> i32 {
    (p * 0.25).floor() as i32
}

fn footprint(bounds: Rectf, rect: Rectf) -> u16 {
    let mut mask = 0;
    for y in 0..4 {
        for x in 0..4 {
            let sx = bounds.x + x as f32 * SUB;
            let sy = bounds.y + y as f32 * SUB;
            if sx < rect.right() && sx + SUB > rect.x && sy < rect.bottom() && sy + SUB > rect.y {
                mask |= 1 << (y * 4 + x);
            }
        }
    }
    mask
}

#[derive(Clone, Copy)]
pub(super) struct Body {
    pub pos: Vec2,
    pub radius: f32,
    pub height: f32,
}

#[derive(Clone, Copy)]
pub(super) struct ShadowProfile {
    depth: [f32; RAYS],
    key: Option<(u32, Vec2, u32, LayerMask, u32)>,
}

impl Default for ShadowProfile {
    fn default() -> Self {
        Self {
            depth: [f32::INFINITY; RAYS],
            key: None,
        }
    }
}

impl ShadowProfile {
    pub(super) fn bake(&mut self, geometry: &Geometry, light: Light) -> bool {
        // The full reach, so a small pool's soft tail is shadowed as well.
        let radius = light.extent();
        let key = (
            geometry.revision,
            light.pos,
            radius.to_bits(),
            light.occluders,
            light.height.to_bits(),
        );
        if self.key == Some(key) {
            return false;
        }
        self.key = Some(key);
        let origin = light.pos;
        for (i, depth) in self.depth.iter_mut().enumerate() {
            let n = (i & 63) as f32 / 32.0 - 1.0;
            let ray = match i >> 6 {
                0 => Vec2::new(1.0, n),
                1 => Vec2::new(-n, 1.0),
                2 => Vec2::new(-1.0, -n),
                _ => Vec2::new(n, -1.0),
            };
            // Distances here are in the L∞ metric (the square angular atlas).
            // A point fixture's height softens its relationship with low props.
            *depth = geometry.trace(
                origin,
                ray,
                radius,
                |_, h| h + 5.0 >= light.height,
                light.occluders,
            );
        }
        true
    }

    /// The baked blocker distance per ray (L∞ metric; infinite = clear).
    pub(super) fn depth(&self) -> &[f32; RAYS] {
        &self.depth
    }

    /// The fractional ray of the angular atlas that `delta` falls on.
    /// `None` at the light itself.
    pub(super) fn ray_position(delta: Vec2) -> Option<f32> {
        let (x, y) = (delta.x, delta.y);
        let dist = x.abs().max(y.abs());
        if dist < 0.5 {
            return None;
        }
        let (quadrant, n) = if x >= y.abs() {
            (0, y)
        } else if y >= x.abs() {
            (64, -x)
        } else if -x >= y.abs() {
            (128, -y)
        } else {
            (192, x)
        };
        Some(quadrant as f32 + (32.0 + 32.0 * n / dist))
    }

    /// Two cached rays give the hard edge pixel coverage without a new trace.
    /// Blend their evaluated visibility because a ray may have no blocker.
    pub(super) fn hard_coverage(&self, delta: Vec2) -> f32 {
        let Some(ray) = Self::ray_position(delta) else {
            return 1.0;
        };
        let dist = delta.x.abs().max(delta.y.abs());
        let lo = ray as usize;
        let tap = |i: usize| {
            ((self.depth[i & (RAYS - 1)] + 0.9 - dist) * 0.35 + 1.0).clamp(0.0, 1.0)
        };
        let a = tap(lo);
        a + (tap(lo + 1) - a) * (ray - lo as f32)
    }

    pub(super) fn visibility(
        &self,
        delta: Vec2,
        surface: Surface,
        softness: ShadowSoftness,
    ) -> f32 {
        let (x, y) = (delta.x, delta.y);
        let dist = x.abs().max(y.abs());
        if dist < 0.5 {
            return 1.0;
        }
        let (quadrant, n) = if x >= y.abs() {
            (0, y)
        } else if y >= x.abs() {
            (64, -x)
        } else if -x >= y.abs() {
            (128, -y)
        } else {
            (192, x)
        };
        let index = quadrant + (32.0 + 32.0 * n / dist).round() as usize;
        // The receiver's own leading edge and raised cap catch light. For
        // floor receivers a cast shadow begins right at the blocker.
        let reach = if surface.height > 0.0 { 8.0 } else { 0.9 };
        let tap =
            |i: usize| ((self.depth[i & (RAYS - 1)] + reach - dist) * 0.35 + 1.0).clamp(0.0, 1.0);
        match softness {
            ShadowSoftness::None => tap(index),
            ShadowSoftness::Soft | ShadowSoftness::Softer => {
                let step = if softness == ShadowSoftness::Soft {
                    1
                } else {
                    3
                };
                (tap(index) * 2.0
                    + tap(index.wrapping_sub(step))
                    + tap(index + step)
                    + tap(index.wrapping_sub(2 * step))
                    + tap(index + 2 * step))
                    / 6.0
            }
        }
    }
}
