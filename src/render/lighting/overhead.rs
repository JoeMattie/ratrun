//! W5 — the global overhead light: one light high above the map's centre, so
//! every body casts a soft ground shadow displaced away from the centre by
//! `(pos - centre) * height / H` (contract §4; owner decision in
//! `.review/handoff/reports-b12/lighting-decisions.md`). Drawn into the floor
//! before the walls, props and bodies, so they cover it and walls clip it.
//!
//! A body at the map's centre has its shadow directly under it; toward the
//! edges the shadow slides away from the centre and stretches along that
//! slide, so the frame reads as one soft light over the whole map rather than
//! a per-body drop shadow. Shadows depend on **world** positions only: the
//! same world draws the same pixels at any camera offset.
//!
//! * The shadow is a *multiply*, never an add: the floor keeps its hue and
//!   loses [`strength::OVERHEAD_DARKEN`] at the core, fading smoothly to 0 at
//!   the rim, and never goes black.
//! * Overlapping shadows take the darker value, not the product: the frame is
//!   rasterised into a `u8` darkness buffer with `max` and applied once, so a
//!   crowd cannot become a black hole and the composite is order-free.
//! * Wall and pit pixels are skipped (the tile map and the flatbed's deck),
//!   so nothing darkens a wall face or the road under a broken plank.
//! * Coarse/Ansi16 draw one flat ellipse of the core darkness (no gradient in
//!   faint terms); Compact draws nothing at all.
//!
//! Integer maths per pixel; the only `f32` is per body (its displacement) and
//! once per frame (the core darkness). Scratch lives in a `thread_local!` and
//! is reused, so a frame allocates nothing.
//!
//! Render-only: nothing here reads or writes simulation state, no rng, no
//! clock (the light does not move, so the shadows are still).

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{strength, Canvas};
use super::material::{self, MaterialKind};
use crate::game::art_roster_outfall;
use crate::game::collision::{TileMap, TILE_F};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{Level, Pit, Theme};
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use bevy_ecs::world::World as EcsWorld;
use std::cell::RefCell;

/// The light's height above the floor, in world pixels (the owner's value:
/// a body's shadow slides `body_height / H` of the way from it to the map's
/// edge).
const LIGHT_HEIGHT: f32 = 900.0;
/// Longest slide, world px: a ground body's shadow never smears off its own
/// neighbourhood.
const DISP_MAX: f32 = 22.0;
/// Flyers (and airborne bosses) sit high, so their shadow slides further and
/// visibly separates from the body.
const DISP_MAX_FLYER: f32 = 20.0;
/// How much longer the shadow gets along its slide, per pixel of slide
/// (`a = b + slide * ELONGATION`): round under a body at the map's centre,
/// clearly oblong toward the edges.
const ELONGATION: f32 = 0.5;
/// A ground body's longest slide, as a share of its footprint half-width.
const GROUND_SLIDE: f32 = 0.8;
/// The share of a footprint table entry the shadow's half-width is.
const FOOTPRINT_NUM: i32 = 2;
const FOOTPRINT_DEN: i32 = 3;
/// Bodies per frame, nearest the rat first.
const MAX_BODIES: usize = 160;

/// Body heights by size class, world px (the owner's table).
const HEIGHT_SMALL: i32 = 6;
const HEIGHT_MEDIUM: i32 = 9;
const HEIGHT_LARGE: i32 = 14;
const HEIGHT_MINIBOSS: i32 = 18;
const HEIGHT_BOSS: i32 = 26;
const HEIGHT_FLYER: i32 = 28;

/// Footprint half-width of the rat (its sprite is about 14 px wide).
const RAT_HALF: i32 = 7;
/// A boss's body is drawn far bigger than any ordinary body.
const BOSS_HALF: i32 = 16;

/// `collision::TILE == 16`, so a world pixel's cell is a shift. Arithmetic
/// shift on `i32` floors negative world positions the way `cell_of`'s float
/// `floor` does.
const CELL_SHIFT: u32 = 4;

/// What the overhead pass reads. Built by `World::draw_scene` each frame the
/// effects are on (and the plate is not flat-lit).
pub(crate) struct OverheadScene<'a> {
    pub ecs: &'a EcsWorld,
    pub level: &'a Level,
    pub player: &'a Player,
    /// World position of pixel (0, 0).
    pub cam: Vec2,
    /// Unread: the overhead light does not move, so nothing here keys on time.
    #[allow(dead_code)]
    pub elapsed: f32,
    /// The tier this frame's lighting runs at.
    pub mode: LightingMode,
    /// A Library plate with no rat: cast no rat shadow.
    pub hide_rat: bool,
}

/// One body's ground shadow, in **screen** pixels: an ellipse centred at
/// `(x, y)`, semi-major `a` along the Q8 unit vector `(ux, uy)` (the slide
/// direction), semi-minor `b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cast {
    x: i32,
    y: i32,
    a: i32,
    b: i32,
    ux: i32,
    uy: i32,
}

/// A body waiting to cast, with the keys the cap and the ordering need.
#[derive(Clone, Copy)]
struct Item {
    /// `SpawnId` (0 for the rat): the stable iteration order.
    spid: u64,
    /// Distance to the rat, squared: the cap keeps the nearest bodies.
    d2: f32,
    cast: Cast,
}

/// The union of every pixel a frame's shadows touched, so the shading pass
/// costs what the shadows cover and nothing more. Kept per row (`x0 > x1` is
/// an untouched row), which is tight for a convex shape and pays for no scan
/// of the empty frame between two distant shadows.
struct Rows<'a> {
    /// One byte of accumulated darkness per world pixel, `max`-composited.
    dark: &'a mut [u8],
    x0: &'a mut [i32],
    x1: &'a mut [i32],
}

impl Rows<'_> {
    #[inline]
    fn put(&mut self, bw: i32, x: i32, y: i32, d: u8) {
        let i = y as usize * bw as usize + x as usize;
        if d > self.dark[i] {
            self.dark[i] = d;
        }
        let row = y as usize;
        if x < self.x0[row] {
            self.x0[row] = x;
        }
        if x > self.x1[row] {
            self.x1[row] = x;
        }
    }
}

/// Reused per-frame scratch (no allocation once warm).
#[derive(Default)]
struct Scratch {
    items: Vec<Item>,
    /// One byte of accumulated darkness per world pixel, `max`-composited.
    dark: Vec<u8>,
    /// The touched span of each screen row (see [`Rows`]).
    row_x0: Vec<i32>,
    row_x1: Vec<i32>,
    /// The size of the frame `dark` was rasterised for and not yet multiplied
    /// into ([`apply_lit`] takes it); `None` when nothing is pending.
    applied: Option<(usize, usize)>,
}

/// The lit-space half of a deferred cast ([`draw_deferred`]): multiply this
/// frame's shadows into the **lit** picture, once, right after the body
/// shadows. Floor pixels only — no wall, prop or raised sample, no creature
/// or emissive pixel — by the same test `softshadow::entity_shadows` uses.
///
/// Why lit space: on a dark floor (the Sewer) most of what the eye sees is
/// additive (scatter, the rat's aura, bounce), so a multiply of the albedo
/// under it moved the pixel by a few levels however deep the shadow was; and
/// the shadow's edge, sitting in the albedo, was embossed by the bump stage
/// like a crack in the floor.
pub(super) fn apply_lit(grid: &Grid, canvas: &mut Canvas<'_>) {
    SCRATCH.with(|cell| {
        let mut guard = cell.borrow_mut();
        let s: &mut Scratch = &mut guard;
        let (w, h) = (canvas.w, canvas.h);
        if s.applied.take() != Some((w, h)) || s.dark.len() < w * h || s.row_x0.len() < h {
            return;
        }
        for y in 0..h {
            let (xa, xb) = (s.row_x0[y], s.row_x1[y]);
            if xa > xb {
                continue;
            }
            for x in xa.max(0) as usize..=(xb.min(w as i32 - 1)) as usize {
                let i = y * w + x;
                let d = s.dark[i];
                if d == 0 {
                    continue;
                }
                let m = canvas.mat_at(i);
                if material::is_creature(m) || material::kind(m) != MaterialKind::Default {
                    continue;
                }
                let si = grid.sample_index(x, y);
                let Some(surface) = grid.surfaces.get(si) else {
                    continue;
                };
                if surface.height > 0.0 || surface.layer & LayerMask::FLOOR.0 == 0 {
                    continue;
                }
                let keep = u32::from(255 - d);
                let p = &mut canvas.px[i];
                p.0 = ((u32::from(p.0) * keep + 127) / 255) as u8;
                p.1 = ((u32::from(p.1) * keep + 127) / 255) as u8;
                p.2 = ((u32::from(p.2) * keep + 127) / 255) as u8;
            }
        }
    });
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// What the rasteriser must not darken: wall cells, deck pits and Outfall tidal drops.
struct Clip<'a> {
    tiles: &'a TileMap,
    /// `cam` rounded once (the same snap `art_floor` uses).
    cam: (i32, i32),
    pits: &'a [Pit],
    tidal_pits: &'a [Pit],
}

impl Clip<'_> {
    /// Is the world pixel `(x, y)` open floor? Out-of-bounds cells read as
    /// solid, which also clips the arena border.
    #[inline]
    fn open(&self, x: i32, y: i32) -> bool {
        let cx = (self.cam.0 + x) >> CELL_SHIFT;
        let cy = (self.cam.1 + y) >> CELL_SHIFT;
        if self.tiles.get(cx, cy).is_wall() {
            return false;
        }
        for pit in self.pits.iter().chain(self.tidal_pits.iter()) {
            if pit.rect.contains(cx, cy) {
                return false;
            }
        }
        true
    }
}

/// Darken the floor under every body (`fx::strength::OVERHEAD_DARKEN`),
/// straight into `pb` (the unlit picture).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn draw(pb: &mut PixelBuffer, scene: &OverheadScene<'_>) {
    cast(pb, scene, false);
}

/// What the scene draw calls: on the tiers that run the lit-space pixel
/// stages (Full, Hard) only rasterise the shadows and leave the multiply to
/// [`apply_lit`]; on Coarse there is no such stage, so darken `pb` here.
pub(crate) fn draw_deferred(pb: &mut PixelBuffer, scene: &OverheadScene<'_>) {
    let lit = matches!(scene.mode, LightingMode::Full | LightingMode::Hard);
    cast(pb, scene, lit);
}

fn cast(pb: &mut PixelBuffer, scene: &OverheadScene<'_>, defer: bool) {
    // Whatever an earlier frame left pending is stale now.
    SCRATCH.with(|cell| cell.borrow_mut().applied = None);
    if pb.w == 0 || pb.h == 0 || super::fx::skip("overhead") {
        return;
    }
    // The tier table (contract §5): on at Full/Hard/Coarse, off at Compact.
    // A positive list, so the test-only `Off` tier draws nothing either.
    if !matches!(
        scene.mode,
        LightingMode::Full | LightingMode::Hard | LightingMode::Coarse
    ) {
        return;
    }
    let soft = matches!(scene.mode, LightingMode::Full | LightingMode::Hard);
    let core = (strength::OVERHEAD_DARKEN * 255.0).round().clamp(0.0, 200.0) as u8;
    if core == 0 {
        return;
    }

    let (bw, bh) = (pb.w as i32, pb.h as i32);
    let centre = Vec2::new(
        scene.level.tiles.cols() as f32 * TILE_F * 0.5,
        scene.level.tiles.rows() as f32 * TILE_F * 0.5,
    );
    let clip = Clip {
        tiles: &scene.level.tiles,
        cam: (scene.cam.x.round() as i32, scene.cam.y.round() as i32),
        pits: &scene.level.trailer.pits,
        tidal_pits: &scene.level.objects.tidal_pits,
    };

    SCRATCH.with(|cell| {
        let mut guard = cell.borrow_mut();
        // A plain `&mut` so the two scratch fields can be borrowed at once
        // (a `RefMut` would borrow the whole struct through `Deref`).
        let s: &mut Scratch = &mut guard;
        s.items.clear();

        // The rat first (a Library plate casts none). On the Outfall the
        // swimming diver floats above the seabed, so its shadow uses the flyer
        // slide rule and a slightly smaller footprint under `player.pos`.
        if !scene.hide_rat {
            let (rat_half, rat_height, rat_flyer) = if scene.level.theme == Theme::Outfall {
                (RAT_HALF - 1, HEIGHT_FLYER, true)
            } else {
                (RAT_HALF, HEIGHT_MEDIUM, false)
            };
            push_body(
                &mut s.items,
                scene.player.pos,
                rat_half,
                rat_height,
                rat_flyer,
                0,
                scene,
                centre,
                (bw, bh),
            );
        }

        // Ordinary bodies and bosses. The composite is a `max`, so the order
        // cannot change a pixel; sorting by `SpawnId` makes the cap's
        // tie-break deterministic too.
        for entity in scene.ecs.iter_entities() {
            let spid = entity.get::<SpawnId>().map_or(u64::MAX, |id| id.0);
            if let Some(boss) = entity.get::<Boss>() {
                if boss.dead || boss.kind == BossKind::Lantern {
                    continue;
                }
                let flying = boss.is_airborne()
                    || boss.overhead()
                    || matches!(boss.state, BossState::Magpie(st) if st.flying());
                let height = if flying { HEIGHT_FLYER } else { HEIGHT_BOSS };
                push_body(
                    &mut s.items,
                    boss.pos,
                    BOSS_HALF,
                    height,
                    flying,
                    spid,
                    scene,
                    centre,
                    (bw, bh),
                );
            } else if let Some(e) = entity.get::<Enemy>() {
                if !e.alive() {
                    continue;
                }
                let (half, height, flying) = enemy_body(e);
                push_body(
                    &mut s.items,
                    e.pos,
                    half,
                    height,
                    flying,
                    spid,
                    scene,
                    centre,
                    (bw, bh),
                );
            }
        }

        s.items.sort_unstable_by_key(|it| it.spid);
        if s.items.len() > MAX_BODIES {
            s.items
                .sort_unstable_by(|a, b| a.d2.total_cmp(&b.d2).then(a.spid.cmp(&b.spid)));
            s.items.truncate(MAX_BODIES);
        }
        if s.items.is_empty() {
            return;
        }

        s.dark.clear();
        s.dark.resize(pb.w * pb.h, 0);
        s.row_x0.clear();
        s.row_x0.resize(bh as usize, i32::MAX);
        s.row_x1.clear();
        s.row_x1.resize(bh as usize, i32::MIN);
        let mut rows = Rows {
            dark: &mut s.dark[..],
            x0: &mut s.row_x0[..],
            x1: &mut s.row_x1[..],
        };
        let mut written = 0usize;
        for it in &s.items {
            written += splat(&mut rows, (bw, bh), it.cast, core, soft, &clip);
        }
        if written == 0 {
            return;
        }
        if defer {
            s.applied = Some((pb.w, pb.h));
            return;
        }

        // One multiply per shadowed pixel: the shadows never stack and never
        // reach black. Each row only visits the span its shadows touched.
        let stride = pb.w;
        let px = pb.pixels_mut();
        for y in 0..bh as usize {
            let (xa, xb) = (rows.x0[y], rows.x1[y]);
            if xa > xb {
                continue;
            }
            let base = y * stride;
            for x in xa.max(0)..=xb.min(bw - 1) {
                let i = base + x as usize;
                let d = rows.dark[i];
                if d == 0 {
                    continue;
                }
                let m = u32::from(255 - d);
                let p = &mut px[i];
                p.0 = ((u32::from(p.0) * m + 127) / 255) as u8;
                p.1 = ((u32::from(p.1) * m + 127) / 255) as u8;
                p.2 = ((u32::from(p.2) * m + 127) / 255) as u8;
            }
        }
    });
}

/// Push one body's cast, if its ellipse can reach the frame.
#[allow(clippy::too_many_arguments)]
fn push_body(
    items: &mut Vec<Item>,
    pos: Vec2,
    half: i32,
    height: i32,
    flyer: bool,
    spid: u64,
    scene: &OverheadScene<'_>,
    centre: Vec2,
    (bw, bh): (i32, i32),
) {
    let Some(cast) = cast_of(
        pos,
        half,
        height,
        flyer,
        centre,
        scene.cam,
        (bw, bh),
    ) else {
        return;
    };
    items.push(Item {
        spid,
        d2: pos.dist_sq(scene.player.pos),
        cast,
    });
}

/// The shadow one body of `height` casts at world position `pos`: the body's
/// footprint slid away from the map centre and stretched along the slide.
/// `None` when the ellipse cannot touch the frame.
fn cast_of(
    pos: Vec2,
    half: i32,
    height: i32,
    flyer: bool,
    centre: Vec2,
    cam: Vec2,
    (bw, bh): (i32, i32),
) -> Option<Cast> {
    // The footprint tables are generous (they were sized for a cone that
    // faded from its centre line); the plateau profile wants the body's own
    // width, or a crowd's shadows merge into one dim floor.
    let b = (half * FOOTPRINT_NUM / FOOTPRINT_DEN).max(2);
    let slide = Vec2::new(pos.x - centre.x, pos.y - centre.y)
        * (height as f32 / LIGHT_HEIGHT);
    // A ground body's shadow stays attached: it slides at most about its own
    // half-width, so it peeks out on the side away from the map's centre
    // instead of lying loose beside the body (a 22 px slide with a 16 px
    // stretch covered the floor between bodies and read as nothing at all).
    let slide = slide.clamp_len(if flyer {
        DISP_MAX_FLYER
    } else {
        DISP_MAX.min(b as f32 * GROUND_SLIDE + 2.0)
    });
    let d = slide.len();
    let x = (pos.x + slide.x - cam.x).round() as i32;
    let y = (pos.y + slide.y - cam.y).round() as i32;
    let a = b + (d * ELONGATION).round() as i32;
    let (ux, uy) = if d > 0.5 {
        (
            (slide.x / d * 256.0).round() as i32,
            (slide.y / d * 256.0).round() as i32,
        )
    } else {
        (256, 0)
    };
    let cast = Cast { x, y, a, b, ux, uy };
    if x + a < 0 || y + a < 0 || x - a >= bw || y - a >= bh {
        return None;
    }
    Some(cast)
}

/// The footprint and height class of an ordinary body, and whether it flies.
fn enemy_body(e: &Enemy) -> (i32, i32, bool) {
    let r = e.radius.max(1.0);
    let archetype = e.kind.archetype();
    // Flyers: the Bat's whole family (every registered moth, fly and wasp is a
    // Bat), the hornet, the moth that carries a fuse, and the Outfall's
    // pelagic swimmers (Conger, Glass Shrimp, Lampfish, Grouper, Fry School).
    let flyer = art_roster_outfall::is_swimmer(e.kind)
        || matches!(
            archetype,
            EnemyKind::Bat | EnemyKind::MurderHornet | EnemyKind::WickMoth
        );
    if e.named {
        // A named miniboss is an `Enemy` with the Brute's radius; its art is
        // two or three times that wide.
        return (14, HEIGHT_MINIBOSS, false);
    }
    if flyer {
        return (((r * 3.0 + 3.0) as i32).clamp(5, 16), HEIGHT_FLYER, true);
    }
    if r < 3.5 {
        // Small: skitterers, fleas, ants, ticks.
        return (((r * 2.4 + 2.0) as i32).clamp(4, 10), HEIGHT_SMALL, false);
    }
    if r < 5.8 {
        // Medium: spitters, pillbugs, porters, cats.
        return (((r * 2.2 + 2.0) as i32).clamp(6, 15), HEIGHT_MEDIUM, false);
    }
    // Large: brutes, the snake, the horde, the carts.
    (((r * 1.9 + 2.0) as i32).clamp(10, 18), HEIGHT_LARGE, false)
}

/// Rasterise one elliptical shadow into the darkness buffer, keeping the
/// darker value per pixel.
///
/// Inverse-mapped over the bounding box: every pixel asks which row of the
/// ellipse (`u`, along the slide direction) it lies on and how far across it
/// (`v`). A forward-mapped rotated lattice is not onto, and left pinholes in
/// every shadow whose slide was not axis-aligned. The per-row profile is two
/// small stack tables, so the pixel loop is integer multiplies only.
fn splat(
    rows: &mut Rows<'_>,
    (bw, bh): (i32, i32),
    cast: Cast,
    core: u8,
    soft: bool,
    clip: &Clip<'_>,
) -> usize {
    const MAX_A: i32 = 63;
    let a = cast.a.clamp(1, MAX_A);
    let b = cast.b.max(1);
    let a2 = a * a;
    let mut span_of = [0i32; (2 * MAX_A + 1) as usize];
    let mut step_of = [0i32; (2 * MAX_A + 1) as usize];
    let mut core_of = [0i32; (2 * MAX_A + 1) as usize];
    for u in -a..=a {
        let t2 = a2 - u * u;
        // Q8 half-width factor: 256 * sqrt(1 - u²/a²).
        let f8 = ((t2 << 16) / a2).isqrt();
        let span = (b * f8) >> 8;
        // The core thins toward the ellipse's tips (half strength at `u = a`),
        // so the shadow reads as a soft blob rather than a dark bar. Faint
        // tiers get one flat step instead: the core, with no gradient at all.
        let core_row = if soft {
            i32::from(core) - (i32::from(core) * u * u) / (2 * a2)
        } else {
            i32::from(core)
        };
        if span < 1 || core_row < 4 {
            continue;
        }
        let k = (u + a) as usize;
        span_of[k] = span;
        core_of[k] = core_row;
        // Darkness per pixel across the row; `v = 0` is the darkest line, and
        // its value is the core exactly (the ceiling keeps the rounding from
        // eating a level at the core).
        step_of[k] = if soft {
            (core_row * 256 + span - 1) / span
        } else {
            core_row
        };
    }
    let reach = a.max(b);
    let (x0, x1) = ((cast.x - reach).max(0), (cast.x + reach).min(bw - 1));
    let (y0, y1) = ((cast.y - reach).max(0), (cast.y + reach).min(bh - 1));
    let mut written = 0;
    for y in y0..=y1 {
        let dy = y - cast.y;
        for x in x0..=x1 {
            let dx = x - cast.x;
            let uq = dx * cast.ux + dy * cast.uy; // u, Q8
            let vq = (dy * cast.ux - dx * cast.uy).abs(); // |v|, Q8
            let ui = (uq + 128) >> 8;
            if ui < -a || ui > a {
                continue;
            }
            let k = (ui + a) as usize;
            let span = span_of[k];
            if span == 0 {
                continue;
            }
            let d = if soft {
                let left = (span << 8) - vq;
                if left <= 0 {
                    continue;
                }
                // A plateau, then a ramp: the core holds out to two thirds of
                // the row's width and fades over the rest, so the part that
                // shows past the body's own sprite is still the core (a
                // linear cone left only its faint skirt in view).
                ((step_of[k] * left * 3) >> 16).min(core_of[k])
            } else {
                if vq * 5 > span * 3 * 256 {
                    continue;
                }
                step_of[k]
            };
            if d <= 0 || !clip.open(x, y) {
                continue;
            }
            rows.put(bw, x, y, d.min(255) as u8);
            written += 1;
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{cell_of, TileKind};
    use crate::game::level::{PitKind, Theme, TileRect};
    use crate::game::world::World;
    use crate::render::lighting::fx::testkit;
    use crate::render::palette::Rgb;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const FLOOR: Rgb = (128, 128, 128);
    const SIZE: usize = 96;

    fn world(theme: Theme) -> World {
        testkit::fx_world(theme)
    }

    fn scene<'a>(w: &'a World, cam: Vec2) -> OverheadScene<'a> {
        OverheadScene {
            ecs: &w.ecs,
            level: &w.level,
            player: &w.player,
            cam,
            elapsed: 0.0,
            mode: LightingMode::Full,
            hide_rat: false,
        }
    }

    fn canvas() -> PixelBuffer {
        let mut pb = PixelBuffer::new(SIZE, SIZE);
        pb.clear(FLOOR);
        pb
    }

    /// The darkest pixel of a drawn frame: `(x, y, red loss)`.
    fn darkest(pb: &PixelBuffer) -> Option<(i32, i32, i32)> {
        let mut best: Option<(i32, i32, i32)> = None;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                let loss = i32::from(FLOOR.0) - i32::from(c.0);
                if loss > 0 && best.is_none_or(|b| loss > b.2) {
                    best = Some((x as i32, y as i32, loss));
                }
            }
        }
        best
    }

    /// The core darkness in output levels, i.e. what `FLOOR` becomes.
    fn core() -> u8 {
        (strength::OVERHEAD_DARKEN * 255.0).round() as u8
    }

    fn darkened(level: u8) -> u8 {
        ((u32::from(FLOOR.0) * u32::from(255 - level) + 127) / 255) as u8
    }

    fn enemy_at(kind: EnemyKind, pos: Vec2) -> Enemy {
        let mut rng = StdRng::seed_from_u64(11);
        Enemy::spawn(kind, pos, 1.0, &mut rng)
    }

    fn spawn(w: &mut World, e: Enemy, id: u64) {
        w.ecs.spawn((e, SpawnId(id)));
    }

    fn centre(w: &World) -> Vec2 {
        Vec2::new(w.level.arena.x * 0.5, w.level.arena.y * 0.5)
    }

    #[test]
    fn a_body_at_the_map_centre_casts_under_itself() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        w.player.pos = c;
        // The rat lands on the buffer's centre pixel.
        let cam = c - Vec2::splat(SIZE as f32 * 0.5);
        let mut pb = canvas();
        draw(&mut pb, &scene(&w, cam));
        let (_, _, loss) = darkest(&pb).expect("a shadow at the centre");
        // The core is a plateau now, so "the darkest pixel" is a patch: it
        // must hold the centre, and be symmetric about it.
        let mid = SIZE / 2;
        assert_eq!(
            i32::from(FLOOR.0) - i32::from(pb.pixel_at(mid, mid).0),
            loss,
            "the rat's own pixel is in the core"
        );
        for d in 1..6 {
            assert_eq!(
                pb.pixel_at(mid - d, mid).0,
                pb.pixel_at(mid + d, mid).0,
                "shadow centred under the rat ({d} px either side)"
            );
        }
        assert_eq!(
            loss,
            i32::from(FLOOR.0) - i32::from(darkened(core())),
            "the core is the contract's overhead darkness"
        );
    }

    #[test]
    fn the_shadow_slides_further_toward_the_map_edge() {
        let w = world(Theme::Sewer);
        let c = centre(&w);
        let cam = Vec2::ZERO;
        let far = cast_of(
            c - Vec2::new(400.0, 400.0),
            7,
            HEIGHT_MEDIUM,
            false,
            c,
            cam,
            (4000, 4000),
        )
        .expect("on screen");
        let mid = cast_of(
            c - Vec2::new(100.0, 100.0),
            7,
            HEIGHT_MEDIUM,
            false,
            c,
            cam,
            (4000, 4000),
        )
        .expect("on screen");
        let far_slide = (i64::from(far.x) - (c.x - 400.0) as i64)
            * (i64::from(far.x) - (c.x - 400.0) as i64)
            + (i64::from(far.y) - (c.y - 400.0) as i64)
                * (i64::from(far.y) - (c.y - 400.0) as i64);
        let mid_slide = (i64::from(mid.x) - (c.x - 100.0) as i64)
            * (i64::from(mid.x) - (c.x - 100.0) as i64)
            + (i64::from(mid.y) - (c.y - 100.0) as i64)
                * (i64::from(mid.y) - (c.y - 100.0) as i64);
        assert!(far.x < (c.x - 400.0) as i32, "up and to the left");
        assert!(far.y < (c.y - 400.0) as i32, "up and to the left");
        assert!(
            far_slide > mid_slide,
            "the corner slides farther than mid-distance ({far_slide} vs {mid_slide})"
        );
        assert!(far.a > far.b, "and stretches along that slide");
    }

    #[test]
    fn a_flyer_slides_further_than_a_ground_body() {
        let w = world(Theme::Sewer);
        let c = centre(&w);
        let at = c + Vec2::new(300.0, 150.0);
        let ground = cast_of(at, 7, HEIGHT_MEDIUM, false, c, Vec2::ZERO, (4000, 4000)).unwrap();
        let flyer = cast_of(at, 7, HEIGHT_FLYER, true, c, Vec2::ZERO, (4000, 4000)).unwrap();
        let slide = |cast: &Cast| {
            ((cast.x as f32 - at.x).powi(2) + (cast.y as f32 - at.y).powi(2)).sqrt()
        };
        assert!(
            slide(&flyer) > slide(&ground) + 2.0,
            "flyer {} px vs ground {} px",
            slide(&flyer),
            slide(&ground)
        );
        assert!(flyer.a > ground.a, "and its shadow is the longer one");
    }

    #[test]
    fn walls_and_pits_are_never_darkened() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        let (cx, cy) = cell_of(c);
        // A wide body at the centre of an open cell, a wall east of it and a
        // hole south of it: both sit inside the shadow's reach.
        w.level.tiles.set(cx + 1, cy, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        w.level
            .trailer
            .pits
            .push(Pit::new(TileRect::new(cx, cy + 1, 1, 1), PitKind::BrokenPlank));
        let body = Vec2::new(cx as f32 * 16.0 + 8.0, cy as f32 * 16.0 + 8.0);
        spawn(&mut w, enemy_at(EnemyKind::Brute, body), 1);
        // The rat is the only other caster, and it is off this frame.
        w.player.pos = c + Vec2::new(600.0, 600.0);
        let cam = Vec2::new(cx as f32 * 16.0 - 40.0, cy as f32 * 16.0 - 40.0);
        let mut pb = canvas();
        draw(&mut pb, &scene(&w, cam));

        let mut darkened_floor = 0;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let wx = cam.x.round() as i32 + x as i32;
                let wy = cam.y.round() as i32 + y as i32;
                let (px_cx, px_cy) = cell_of(Vec2::new(wx as f32, wy as f32));
                let wall = w.level.tiles.get(px_cx, px_cy).is_wall();
                let pit = w.level.trailer.pit_at(Vec2::new(wx as f32, wy as f32)).is_some();
                let col = pb.pixel_at(x, y);
                if wall || pit {
                    assert_eq!(col, FLOOR, "({x}, {y}) is a wall/pit and must stay clear");
                } else if col != FLOOR {
                    darkened_floor += 1;
                }
            }
        }
        assert!(darkened_floor > 0, "the test must actually shadow the floor");
        // The shadow reaches both faces: the last floor pixel before the wall
        // is dark, the wall pixel itself is not.
        let at = |wx: i32, wy: i32| {
            pb.pixel_at((wx - cam.x as i32) as usize, (wy - cam.y as i32) as usize)
        };
        let (wall_x, body_y) = ((cx + 1) * 16, cy * 16 + 8);
        let (body_x, pit_y) = (cx * 16 + 8, (cy + 1) * 16);
        assert_ne!(at(wall_x - 1, body_y), FLOOR, "floor before the wall face is dark");
        assert_eq!(at(wall_x, body_y), FLOOR, "the wall face itself is not");
        assert_ne!(at(body_x, pit_y - 1), FLOOR, "floor before the pit is dark");
        assert_eq!(at(body_x, pit_y), FLOOR, "the pit is not");
    }

    #[test]
    fn outfall_tidal_pits_are_never_darkened() {
        let mut w = world(Theme::Outfall);
        let c = centre(&w);
        let (cx, cy) = cell_of(c);
        w.level
            .objects
            .tidal_pits
            .push(Pit::new(TileRect::new(cx, cy + 1, 1, 1), PitKind::TidalDrop));
        let body = Vec2::new(cx as f32 * 16.0 + 8.0, cy as f32 * 16.0 + 8.0);
        spawn(&mut w, enemy_at(EnemyKind::Brute, body), 1);
        w.player.pos = c + Vec2::new(600.0, 600.0);
        let cam = Vec2::new(cx as f32 * 16.0 - 40.0, cy as f32 * 16.0 - 40.0);
        let mut pb = canvas();
        draw(&mut pb, &scene(&w, cam));
        let at = |wx: i32, wy: i32| {
            pb.pixel_at((wx - cam.x as i32) as usize, (wy - cam.y as i32) as usize)
        };
        let (body_x, pit_y) = (cx * 16 + 8, (cy + 1) * 16);
        assert_ne!(at(body_x, pit_y - 1), FLOOR, "sand before tidal drop is shadowed");
        assert_eq!(at(body_x, pit_y), FLOOR, "tidal drop pit stays clear");
    }

    #[test]
    fn overlapping_shadows_take_the_darker_value_not_the_product() {
        let c = centre(&world(Theme::Sewer));
        let cam = c - Vec2::splat(SIZE as f32 * 0.5);

        let single = {
            let mut w = world(Theme::Sewer);
            w.player.pos = c;
            spawn(&mut w, enemy_at(EnemyKind::Skitterer, c), 1);
            let mut pb = canvas();
            let mut sc = scene(&w, cam);
            sc.hide_rat = true;
            draw(&mut pb, &sc);
            darkest(&pb).expect("one shadow")
        };
        let pair = {
            let mut w = world(Theme::Sewer);
            w.player.pos = c;
            spawn(&mut w, enemy_at(EnemyKind::Skitterer, c), 1);
            spawn(&mut w, enemy_at(EnemyKind::Skitterer, c), 2);
            let mut pb = canvas();
            let mut sc = scene(&w, cam);
            sc.hide_rat = true;
            draw(&mut pb, &sc);
            darkest(&pb).expect("two shadows")
        };
        assert_eq!(pair.2, single.2, "the second shadow does not stack");
        assert!(pair.2 < 2 * single.2, "never the product");
    }

    #[test]
    fn the_same_world_draws_the_same_pixels_at_two_camera_offsets() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        w.player.pos = c;
        spawn(&mut w, enemy_at(EnemyKind::Brute, c + Vec2::new(20.0, -14.0)), 1);
        let cam_a = c - Vec2::splat(SIZE as f32 * 0.5);
        let shift = (11, 7);
        let cam_b = cam_a + Vec2::new(shift.0 as f32, shift.1 as f32);
        let mut a = canvas();
        let mut b = canvas();
        draw(&mut a, &scene(&w, cam_a));
        draw(&mut b, &scene(&w, cam_b));
        let mut differing = 0;
        for y in shift.1..SIZE {
            for x in shift.0..SIZE {
                assert_eq!(
                    a.pixel_at(x, y),
                    b.pixel_at(x - shift.0, y - shift.1),
                    "({x}, {y}) must be the same world pixel, shifted"
                );
                if a.pixel_at(x, y) != FLOOR {
                    differing += 1;
                }
            }
        }
        assert!(differing > 0, "the frame must carry a shadow to compare");
    }

    #[test]
    fn hide_rat_casts_no_rat_shadow() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        w.player.pos = c;
        let cam = c - Vec2::splat(SIZE as f32 * 0.5);
        let mut shown = canvas();
        draw(&mut shown, &scene(&w, cam));
        assert!(darkest(&shown).is_some(), "the rat casts one normally");
        let mut hidden = canvas();
        let mut sc = scene(&w, cam);
        sc.hide_rat = true;
        draw(&mut hidden, &sc);
        assert_eq!(darkest(&hidden), None, "a plate with no rat is untouched");
    }

    #[test]
    fn full_defers_to_the_lit_picture_and_spares_bodies_and_coarse_does_not() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        w.player.pos = c;
        let cam = c - Vec2::splat(SIZE as f32 * 0.5);
        let mid = SIZE / 2;

        // Full: the scene draw only rasterises; the unlit picture is untouched.
        let mut pb = canvas();
        pb.set_materials(true);
        // A creature pixel in the core and a floor pixel beside it.
        let prev = pb.set_pen(material::pen(MaterialKind::Fur, true, 2));
        pb.plot(mid as i32, mid as i32, FLOOR);
        pb.set_pen(prev);
        draw_deferred(&mut pb, &scene(&w, cam));
        assert_eq!(darkest(&pb), None, "Full leaves the albedo alone");
        let grid = testkit::flat_grid(SIZE, SIZE, 1, [256, 256, 256]);
        apply_lit(&grid, &mut testkit::canvas(&mut pb));
        assert_eq!(pb.pixel_at(mid, mid), FLOOR, "a creature pixel is never shadowed");
        assert_eq!(
            pb.pixel_at(mid, mid + 1).0,
            darkened(core()),
            "the floor beside it takes the core"
        );
        // Taken once: a second call (a frame that cast nothing) is a no-op.
        let once = pb.pixels().to_vec();
        apply_lit(&grid, &mut testkit::canvas(&mut pb));
        assert_eq!(pb.pixels(), &once[..], "the pending shadows are applied once");

        // Coarse has no lit-space stage: the scene draw darkens the albedo.
        let mut pb = canvas();
        let mut sc = scene(&w, cam);
        sc.mode = LightingMode::Coarse;
        draw_deferred(&mut pb, &sc);
        assert!(darkest(&pb).is_some(), "Coarse darkens in place");
    }

    #[test]
    fn compact_draws_nothing() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        w.player.pos = c;
        spawn(&mut w, enemy_at(EnemyKind::Brute, c + Vec2::new(24.0, 0.0)), 1);
        let cam = c - Vec2::splat(SIZE as f32 * 0.5);
        let mut pb = canvas();
        let mut sc = scene(&w, cam);
        sc.mode = LightingMode::Compact;
        draw(&mut pb, &sc);
        assert_eq!(darkest(&pb), None, "Compact has no overhead pass");
    }

    #[test]
    fn coarse_shadows_are_one_flat_step() {
        let mut w = world(Theme::Sewer);
        let c = centre(&w);
        w.player.pos = c;
        let cam = c - Vec2::splat(SIZE as f32 * 0.5);
        let mut pb = canvas();
        let mut sc = scene(&w, cam);
        sc.mode = LightingMode::Coarse;
        draw(&mut pb, &sc);
        let mut levels = std::collections::BTreeSet::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let col = pb.pixel_at(x, y);
                if col != FLOOR {
                    levels.insert(col.0);
                }
            }
        }
        assert_eq!(levels.len(), 1, "one flat darkness at Coarse: {levels:?}");
        assert_eq!(levels.into_iter().next(), Some(darkened(core())));
    }

    #[test]
    fn outfall_diver_rat_and_swimmers_use_flyer_slide_while_sewer_is_unchanged() {
        let w = world(Theme::Outfall);
        let c = centre(&w);
        let at = c + Vec2::new(320.0, 180.0);

        // Swimmers use the flyer height/slide rule; bottom dwellers stay grounded.
        for swimmer in [
            EnemyKind::GlassShrimp,
            EnemyKind::Conger,
            EnemyKind::Lampfish,
            EnemyKind::Grouper,
            EnemyKind::FrySchool,
        ] {
            let e = enemy_at(swimmer, at);
            let (_, h, fly) = enemy_body(&e);
            assert!(fly && h == HEIGHT_FLYER, "{swimmer:?} must use flyer overhead rule");
        }
        for grounded in [
            EnemyKind::SandHopper,
            EnemyKind::PistolShrimp,
            EnemyKind::Skater,
            EnemyKind::Skitterer,
            EnemyKind::Brute,
        ] {
            let e = enemy_at(grounded, at);
            let (_, _, fly) = enemy_body(&e);
            assert!(!fly, "{grounded:?} must stay grounded");
        }

        // Outfall rat slides further away from map centre than a Sewer rat at the same spot.
        let mut sewer_w = world(Theme::Sewer);
        let mut outfall_w = world(Theme::Outfall);
        sewer_w.player.pos = at;
        outfall_w.player.pos = at;
        let cam = at - Vec2::splat(SIZE as f32 * 0.5);
        let mut pb_s = canvas();
        let mut pb_o = canvas();
        draw(&mut pb_s, &scene(&sewer_w, cam));
        draw(&mut pb_o, &scene(&outfall_w, cam));
        let (sx, sy, _) = darkest(&pb_s).expect("sewer rat shadow");
        let (ox, oy, _) = darkest(&pb_o).expect("outfall rat shadow");
        let mid = (SIZE / 2) as f32;
        let d_sewer = (sx as f32 - mid).hypot(sy as f32 - mid);
        let d_outfall = (ox as f32 - mid).hypot(oy as f32 - mid);
        assert!(
            d_outfall > d_sewer + 1.5,
            "Outfall floating rat should slide further ({d_outfall:.1}) than Sewer rat ({d_sewer:.1})"
        );
    }
}
