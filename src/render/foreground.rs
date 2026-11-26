//! Screen-spanning foreground bands, drawn after lighting and under hostile tells.
//! Placement depends only on the level. Motion, fades and scratch compositing
//! are render-only and leave the simulation and level generation untouched.

use bevy_ecs::world::World as EcsWorld;

use crate::game::art_foreground;
use crate::game::collision::TILE_F;
use crate::game::ecs::systems::boss::Boss;
use crate::game::ecs::systems::miniboss::Miniboss;
use crate::game::enemy::Enemy;
use crate::game::level::{Level, RoomKind, Theme};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

pub const ALPHA_MAX: u8 = 215;
const ALPHA_FLOOR: f32 = 28.0;
const ALPHA_FLOOR_RAT: f32 = 120.0;
const FADE_RAT_PX: f32 = 48.0;
const FADE_THREAT_PX: f32 = 48.0;
const FADE_RAMP_PX: f32 = 40.0;
const EDGE_FADE_PX: f32 = 16.0;
const SWAY_HZ: f32 = 3.0;
const COVERAGE_PCT: u32 = 45;
const COVERAGE_SKIP_PCT: u32 = 85;
const OUTFALL_COVERAGE_PCT: u32 = 14;
const OUTFALL_COVERAGE_FILL: f32 = 0.36;
const COVERAGE_ALPHA_MIN: f32 = 0.5;
const COVERAGE_FILL: f32 = 0.7;
const COVERAGE_HALF: f32 = 0.5;
const DARKEN: f32 = 0.88;
const SHADOW_MAX: f32 = 1.0;
const CLEAR_DISC_IN_PX: f32 = 14.0;
const CLEAR_DISC_OUT_PX: f32 = 30.0;
const CLEAR_DISC_MIN: f32 = 0.25;
const RAT_SKIP_PX: f32 = 10.0;
const CLOUD_ALPHA_MAX: u8 = 60;

pub const BAND_OVERHANG: i32 = 24;
pub const MAX_BANDS_IN_VIEW: usize = 2;
const MIN_SCREEN_SPACING: f32 = 48.0;
const PIXEL_SCALE: i32 = 2;
const PIXEL_SCALE_NEAR: i32 = 3;
const PIXEL_SCALE_NEAR_DEPTH: f32 = 2.6;
// Cache whole sections of the along-axis art lattice, with room for the blur.
const RASTER_PAGE: i32 = 32;

const START_CLEAR_PX: f32 = 96.0;
const MIN_SPACING: f32 = 160.0;
const GEN_SALT: u32 = 0x0F0E_6000;
const BOSS_BODY_RADIUS: f32 = 20.0;
const DEPTH_MIN: f32 = 1.8;
const DEPTH_MAX: f32 = 3.0;
const OUTFALL_DEPTH_MIN: f32 = 1.8;
const OUTFALL_DEPTH_MAX: f32 = 2.4;
const SCALE_MIN: f32 = 1.0;
const SCALE_MAX: f32 = 1.6;
const CORRIDOR_MIN_TILES: i32 = 5;
const ROOM_MIN_SHORT: i32 = 9;
const ROOM_MIN_LONG: i32 = 12;
const ROOM_TILES2_PER_BAND: i32 = 120;
const PLACE_ATTEMPTS: u32 = 8;

const SKY_QUANT_HZ: f32 = 30.0;
const SKY_BRANCH_PERIOD: f32 = 7.0;
const SKY_BRANCH_SPEED: f32 = 220.0;
const SKY_CLOUD_SPEED: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeKind {
    MainPipe,
    CatwalkGrating,
    ChainRun,
    ExtractorDuct,
    PotRail,
    CableRun,
    CableTray,
    VentDuct,
    Gantry,
    Clothesline,
    SteamPipe,
    RackBeam,
    PaperWeb,
    RollerBeam,
    ProofLine,
    BranchCanopy,
    PowerLines,
    CloudBand,
    /// Outfall swaying kelp curtain band (depth 1.8–2.4, 3 Hz held sway, 12.5 art px base).
    Kelp,
    /// Outfall sunken shipwreck hull-rib span (depth 1.8–2.4, rigid, max 1/room, 13.5 art px base).
    HullRib,
    /// Outfall rising deep-sea bubble-chain band (depth 1.8–2.4, 3 Hz held sway, 12.0 art px base).
    BubbleChain,
}

impl ForeKind {
    pub const fn all() -> [Self; 21] {
        [
            Self::MainPipe,
            Self::CatwalkGrating,
            Self::ChainRun,
            Self::ExtractorDuct,
            Self::PotRail,
            Self::CableRun,
            Self::CableTray,
            Self::VentDuct,
            Self::Gantry,
            Self::Clothesline,
            Self::SteamPipe,
            Self::RackBeam,
            Self::PaperWeb,
            Self::RollerBeam,
            Self::ProofLine,
            Self::BranchCanopy,
            Self::PowerLines,
            Self::CloudBand,
            Self::Kelp,
            Self::HullRib,
            Self::BubbleChain,
        ]
    }

    fn organic(self) -> bool {
        matches!(self, Self::Kelp | Self::BubbleChain | Self::BranchCanopy)
    }

    fn sways(self) -> bool {
        matches!(
            self,
            Self::ChainRun
                | Self::PotRail
                | Self::CableRun
                | Self::Clothesline
                | Self::ProofLine
                | Self::BranchCanopy
                | Self::Kelp
                | Self::BubbleChain
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForeItem {
    pub kind: ForeKind,
    /// Unit tangent of the band, pointing in the direction of the art.
    pub axis: Vec2,
    /// World line offset along the unit normal.
    pub cross: f32,
    pub depth: f32,
    pub scale: f32,
    pub seed: u32,
}

const HORIZONTAL: Vec2 = Vec2 { x: 1.0, y: 0.0 };
const VERTICAL: Vec2 = Vec2 { x: 0.0, y: 1.0 };

impl ForeItem {
    pub(crate) fn normal(self) -> Vec2 {
        // Keep the original positive cross direction of the vertical art.
        if self.axis == VERTICAL {
            HORIZONTAL
        } else {
            self.axis.perp()
        }
    }
}

fn placement_axis(kind: ForeKind, horizontal: bool, seed: u32) -> Vec2 {
    if kind.organic() {
        let turn = hash01(u64::from(seed), 0xA961) - 0.5;
        Vec2::from_angle(turn * std::f32::consts::PI).normalized()
    } else if horizontal {
        HORIZONTAL
    } else {
        VERTICAL
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ForeDraw {
    /// Both planes are drawn at the same art-space cross coordinate. The
    /// layer places each plane at its own cross coordinate during the blit.
    pub screen_cross: i32,
    pub shadow_cross: i32,
    /// Inclusive along-axis extent in scratch art pixels.
    pub from: i32,
    pub to: i32,
    pub scale: f32,
    pub alpha: u8,
    pub sway: f32,
    pub theme: Theme,
    /// Add to an along-axis art coordinate to get its world-aligned art pixel.
    pub along_offset: i32,
    /// Held animation step, in 3 Hz steps.
    pub elapsed_step: u32,
    /// Direction along the cross axis toward the view centre, either -1 or 1.
    pub toward_centre: i8,
}

pub fn rim_tint(theme: Theme) -> Rgb {
    palette::scale(crate::render::lighting::theme_ambient(theme).rat_tint, 0.30)
}

const fn kinds_for(theme: Theme) -> [ForeKind; 3] {
    let all = ForeKind::all();
    let first = match theme {
        Theme::Sewer => 0,
        Theme::Kitchen => 3,
        Theme::Lab => 6,
        Theme::Laundry => 9,
        Theme::Printworks => 12,
        Theme::Trailer => 15,
        Theme::Outfall => 18,
    };
    [all[first], all[first + 1], all[first + 2]]
}

#[inline]
const fn coverage_pct(theme: Theme) -> u32 {
    if matches!(theme, Theme::Outfall) {
        OUTFALL_COVERAGE_PCT
    } else {
        COVERAGE_PCT
    }
}

#[inline]
const fn coverage_skip_pct(theme: Theme) -> u32 {
    if matches!(theme, Theme::Outfall) {
        OUTFALL_COVERAGE_PCT
    } else {
        COVERAGE_SKIP_PCT
    }
}

#[inline]
const fn coverage_fill(theme: Theme) -> f32 {
    if matches!(theme, Theme::Outfall) {
        OUTFALL_COVERAGE_FILL
    } else {
        COVERAGE_FILL
    }
}

fn pixel_scale(item: &ForeItem) -> i32 {
    if item.kind != ForeKind::CloudBand && item.depth >= PIXEL_SCALE_NEAR_DEPTH {
        PIXEL_SCALE_NEAR
    } else {
        PIXEL_SCALE
    }
}

/// Standalone worlds can enable this layer with `RATRUN_FOREGROUND=1`.
/// The App uses the saved graphics preference and honours either env pin.
pub fn enabled_from_env() -> bool {
    match std::env::var("RATRUN_FOREGROUND") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => false,
    }
}

// ---- The only pixel writers the art may use -------------------------------

thread_local! {
    /// Pixels this frame's layer may still paint (`u32::MAX` = unlimited). See
    /// [`COVERAGE_PCT`] and [`draw`].
    static WRITE_BUDGET: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
    /// Pixels painted since [`budget_start`].
    static WRITES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// The rat's clear disc: centre in buffer pixels, zoom and active flag.
    /// Direct art calls outside [`draw`] have no disc.
    static CLEAR_DISC: std::cell::Cell<(i32, i32, f32, bool)> =
        const { std::cell::Cell::new((0, 0, 1.0, false)) };
    /// Where [`blend`] and [`shadow`] write. `None` (the default, and what a
    /// test calling the art directly sees) is the frame buffer they are handed;
    /// `Some` is the item scratch [`draw`] lends out around one art call, so the
    /// art keeps calling the same two functions and is upscaled afterwards.
    static TARGET: std::cell::RefCell<Option<Scratch>> = const { std::cell::RefCell::new(None) };
}

/// One item's offscreen raster: a window `[x0, x0 + w) x [y0, y0 + h)` of art
/// coordinates holding the body's colour and alpha and the floor shadow's
/// strength. Kept on [`ForeLayer`] and only ever grown, so once a level has
/// drawn its largest item no frame allocates.
#[derive(Default)]
struct Scratch {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    body: Vec<[u32; 4]>,
    filtered_rows: Vec<[u32; 4]>,
    shade: Vec<u8>,
}

impl Scratch {
    /// Make the scratch the window `(x0, y0, w, h)`, cleared. Reuses the
    /// buffers: `clear` + `resize` only allocates past the capacity reached.
    fn reset(&mut self, x0: i32, y0: i32, w: i32, h: i32) {
        let n = (w.max(0) * h.max(0)) as usize;
        self.x0 = x0;
        self.y0 = y0;
        self.w = w.max(0);
        self.h = h.max(0);
        self.body.clear();
        self.body.resize(n, [0; 4]);
        self.shade.clear();
        self.shade.resize(n, 0);
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (lx, ly) = (x - self.x0, y - self.y0);
        if lx < 0 || ly < 0 || lx >= self.w || ly >= self.h {
            return None;
        }
        Some((ly * self.w + lx) as usize)
    }

    /// The art wrote a body pixel: keep its colour and alpha (the art writes
    /// each pixel once a pass, so the last write is the pixel).
    #[inline]
    fn put(&mut self, x: i32, y: i32, rgb: Rgb, alpha: u8) {
        if let Some(i) = self.index(x, y) {
            let a = u32::from(alpha);
            self.body[i] = [u32::from(rgb.0) * a, u32::from(rgb.1) * a,
                u32::from(rgb.2) * a, a];
        }
    }

    /// The art wrote a shadow pixel: keep the strongest.
    #[inline]
    fn put_shade(&mut self, x: i32, y: i32, strength: f32) {
        if let Some(i) = self.index(x, y) {
            let v = (strength.clamp(0.0, 1.0) * 255.0).round() as u8;
            if v > self.shade[i] {
                self.shade[i] = v;
            }
        }
    }

    /// The buffers' combined capacity (tests: nothing grows once warm).
    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.body.capacity() + self.filtered_rows.capacity() + self.shade.capacity()
    }
}

/// Take one pixel of the frame's budget; `false` when it is spent.
#[inline]
fn take_budget() -> bool {
    WRITE_BUDGET.with(|b| {
        let left = b.get();
        if left == 0 {
            return false;
        }
        if left != u32::MAX {
            b.set(left - 1);
        }
        WRITES.with(|w| w.set(w.get().saturating_add(1)));
        true
    })
}

fn budget_start(pixels: u32) {
    WRITE_BUDGET.with(|b| b.set(pixels));
    WRITES.with(|w| w.set(0));
}

fn budget_finish() -> u32 {
    WRITE_BUDGET.with(|b| b.set(u32::MAX));
    WRITES.with(|w| w.get())
}

/// Pixels painted so far this frame (0 outside a [`draw`]).
#[inline]
fn writes_so_far() -> u32 {
    WRITES.with(|w| w.get())
}

fn clear_disc_set(centre: Vec2, zoom: f32) {
    CLEAR_DISC.with(|c| c.set((centre.x.round() as i32, centre.y.round() as i32, zoom, true)));
}

fn clear_disc_reset() {
    CLEAR_DISC.with(|c| c.set((0, 0, 1.0, false)));
}

/// The alpha multiplier the rat's clear disc asks for at `(x, y)`: 1.0 with no
/// disc, [`CLEAR_DISC_MIN`] inside [`CLEAR_DISC_IN_PX`], ramping back to 1.0 at
/// [`CLEAR_DISC_OUT_PX`].
#[inline]
fn clear_disc_factor(x: i32, y: i32) -> f32 {
    CLEAR_DISC.with(|c| {
        let (cx, cy, zoom, on) = c.get();
        if !on {
            return 1.0;
        }
        let dx = (x - cx) as f32 / zoom;
        let dy = (y - cy) as f32 / zoom;
        let d = (dx * dx + dy * dy).sqrt();
        if d <= CLEAR_DISC_IN_PX {
            CLEAR_DISC_MIN
        } else if d >= CLEAR_DISC_OUT_PX {
            1.0
        } else {
            let t = (d - CLEAR_DISC_IN_PX) / (CLEAR_DISC_OUT_PX - CLEAR_DISC_IN_PX);
            CLEAR_DISC_MIN + (1.0 - CLEAR_DISC_MIN) * t
        }
    })
}

/// Is `(x, y)` inside the clear disc's hard core ([`RAT_SKIP_PX`])? Never true
/// outside a [`draw`].
#[inline]
fn in_rat_core(x: i32, y: i32) -> bool {
    CLEAR_DISC.with(|c| {
        let (cx, cy, zoom, on) = c.get();
        if !on {
            return false;
        }
        let dx = (x - cx) as f32 / zoom;
        let dy = (y - cy) as f32 / zoom;
        dx * dx + dy * dy <= RAT_SKIP_PX * RAT_SKIP_PX
    })
}

/// Alpha-blend `rgb` over the existing pixel at `(x, y)`, clipped to the
/// buffer. The **only** body painter the layer's art may use.
///
/// * The alpha is clamped to [`ALPHA_MAX`], so no pixel of this layer is ever
///   opaque (the fight can never be hidden), and quartered near the rat by the
///   frame's clear disc.
/// * The colour is darkened by [`DARKEN`] first, so the layer reads as a
///   silhouette against the lit floor without a per-pixel lighting pass. Ask
///   [`rim_tint`] for a rim colour on top.
/// * It counts against the frame's coverage budget ([`COVERAGE_SKIP_PCT`]);
///   past it the write is dropped, so the cap holds whatever the art paints.
///
/// While [`draw`] is rasterising an item the write goes to the item's scratch
/// instead (see [`TARGET`]); its filtered raster is then composited per pixel.
#[inline]
pub fn blend(pb: &mut PixelBuffer, x: i32, y: i32, rgb: Rgb, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let to_scratch = TARGET.with(|t| match t.borrow_mut().as_mut() {
        Some(s) => {
            s.put(x, y, rgb, alpha);
            true
        }
        None => false,
    });
    if !to_scratch {
        blend_screen(pb, x, y, rgb, alpha);
    }
}

/// The screen half of [`blend`]: clip, clamp, clear disc, budget, darken, mix.
#[inline]
fn blend_screen(pb: &mut PixelBuffer, x: i32, y: i32, rgb: Rgb, alpha: u8) {
    if alpha == 0 || x < 0 || y < 0 {
        return;
    }
    let (xu, yu) = (x as usize, y as usize);
    if xu >= pb.w || yu >= pb.h {
        return;
    }
    if in_rat_core(x, y) {
        return;
    }
    let a = (alpha.min(ALPHA_MAX) as f32 * clear_disc_factor(x, y)).round() as u8;
    if a == 0 {
        return;
    }
    // A haze-level write (a cloud's body, an item faded right down) hides
    // nothing, so it is not charged to the coverage budget.
    if a > CLOUD_ALPHA_MAX && !take_budget() {
        return;
    }
    let i = yu * pb.w + xu;
    let px = pb.pixels_mut();
    let cur = px[i];
    px[i] = palette::mix(cur, palette::scale(rgb, DARKEN), a as f32 / 255.0);
}

/// Darken the pixel at `(x, y)` by `strength` (clamped to `0..=1`, scaled by
/// [`SHADOW_MAX`]), clipped to the buffer: the layer's floor-shadow writer.
/// It is a darken, so it can never brighten anything, and it is **not** counted
/// against the body coverage budget (the cap counts item pixels only).
///
/// Like [`blend`], it writes the lent scratch while an item is rasterised.
#[inline]
pub fn shadow(pb: &mut PixelBuffer, x: i32, y: i32, strength: f32) {
    if strength <= 0.0 {
        return;
    }
    let to_scratch = TARGET.with(|t| match t.borrow_mut().as_mut() {
        Some(s) => {
            s.put_shade(x, y, strength);
            true
        }
        None => false,
    });
    if !to_scratch {
        shadow_screen(pb, x, y, strength);
    }
}

/// The screen half of [`shadow`].
#[inline]
fn shadow_screen(pb: &mut PixelBuffer, x: i32, y: i32, strength: f32) {
    let s = strength.clamp(0.0, 1.0) * SHADOW_MAX;
    if s <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (xu, yu) = (x as usize, y as usize);
    if xu >= pb.w || yu >= pb.h {
        return;
    }
    let i = yu * pb.w + xu;
    let px = pb.pixels_mut();
    px[i] = palette::scale(px[i], 1.0 - s);
}


#[derive(Clone, Copy, Debug)]
struct Plane {
    cross: i32,
    span: (f32, f32),
    fade: f32,
}

impl Plane {
    fn new(cross: f32, extent: (f32, f32), view_span: (f32, f32), zoom: f32) -> Self {
        let cross = cross.round() as i32;
        let span = (cross as f32 + extent.0, cross as f32 + extent.1);
        // Measure how far the leading side has passed the nearer view edge.
        let penetration = (span.1 - view_span.0).min(view_span.1 - span.0);
        let fade = (penetration / (EDGE_FADE_PX * zoom)).clamp(0.0, 1.0);
        Self { cross, span, fade }
    }
}

#[derive(Clone, Copy, Debug)]
struct Plan {
    item: ForeItem,
    normal: Vec2,
    body: Plane,
    shadow: Plane,
    centre_cross: f32,
    along: (f32, f32),
    half_thickness: f32,
    rat_distance: f32,
    alpha: u8,
    k: i32,
    radius: i32,
    crossing_rat: bool,
}

impl Plan {
    fn body_area(&self, size: Vec2) -> f32 {
        if self.body.fade == 0.0 {
            return 0.0;
        }
        let (lo, hi) = project(size, self.normal);
        let start = self.body.span.0.max(lo);
        let end = self.body.span.1.min(hi);
        if end <= start {
            return 0.0;
        }
        let width_at = |v| {
            let origin = self.normal * v;
            let (lo, hi) = line_span(origin, self.item.axis, size);
            (hi - lo).max(0.0)
        };
        // Three slices estimate the visible strip area, including bands
        // passing through a corner of the view.
        (end - start) * (width_at(start) + 4.0 * width_at((start + end) * 0.5)
            + width_at(end)) / 6.0
    }

    fn visible_lines(&self) -> impl Iterator<Item = i32> {
        [self.body, self.shadow].into_iter()
            .filter_map(|plane| (plane.fade > 0.0).then_some(plane.cross))
    }

    fn centre_distance(&self) -> f32 {
        self.visible_lines()
            .map(|cross| (cross as f32 - self.centre_cross).abs())
            .fold(f32::INFINITY, f32::min)
    }
}

/// The projection of the viewport corners onto one of the band's axes.
fn project(size: Vec2, axis: Vec2) -> (f32, f32) {
    let x = size.x * axis.x;
    let y = size.y * axis.y;
    (x.min(0.0) + y.min(0.0), x.max(0.0) + y.max(0.0))
}

/// Inclusive parameter range for a line intersecting the view rectangle.
fn line_span(origin: Vec2, axis: Vec2, size: Vec2) -> (f32, f32) {
    let slab = |at: f32, dir: f32, edge: f32| {
        if dir == 0.0 {
            return if (0.0..=edge).contains(&at) {
                (f32::NEG_INFINITY, f32::INFINITY)
            } else {
                (f32::INFINITY, f32::NEG_INFINITY)
            };
        }
        let a = -at / dir;
        let b = (edge - at) / dir;
        (a.min(b), a.max(b))
    };
    let x = slab(origin.x, axis.x, size.x);
    let y = slab(origin.y, axis.y, size.y);
    (x.0.max(y.0), x.1.min(y.1))
}

struct PlacedBand {
    item: ForeItem,
    anchor: Vec2,
    #[cfg(test)]
    corridor: bool,
}

/// Render state owned by `World`. Placement is cached by level, and the
/// frame plan and raster allocation are reused.
pub struct ForeLayer {
    items: Vec<PlacedBand>,
    key: Option<(Theme, u64)>,
    frame: Vec<Plan>,
    scratch: Scratch,
    rasters: [Focus; MAX_BANDS_IN_VIEW],
}

impl Default for ForeLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl ForeLayer {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            key: None,
            frame: Vec::with_capacity(MAX_BANDS_IN_VIEW),
            scratch: Scratch::default(),
            rasters: std::array::from_fn(|_| Focus::default()),
        }
    }

    /// Build the static band list once per level.
    pub fn sync(&mut self, level: &Level) {
        let key = (level.theme, level.seed);
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.items.clear();
        generate(level, &mut self.items);
    }

    #[cfg(test)]
    pub(crate) fn capture_items(&self) -> impl Iterator<Item = (ForeItem, Vec2, bool)> + '_ {
        self.items.iter().map(|p| (p.item, p.anchor, p.corridor))
    }

    fn plan_frame(
        &mut self,
        view: View,
        level: &Level,
        rat: Vec2,
        elapsed: f32,
        threats: &[(Vec2, f32)],
    ) {
        self.frame.clear();
        let sky = sky_items(level, view, elapsed);
        let items = sky.into_iter().flatten().chain(self.items.iter().map(|p| p.item));

        // Select by the nearest visible plane of each band, in screen pixels.
        // A second pass is enough to find the next candidate with room beside
        // the first. Parallel bands need space between all their visible lines.
        for _ in 0..MAX_BANDS_IN_VIEW {
            let next = items.clone()
                .filter_map(|item| plan_band(view, item, rat, threats))
                .filter(|p| {
                    self.frame.iter().all(|other| {
                        p.item.axis != other.item.axis
                            || p.visible_lines().all(|cross| {
                                other.visible_lines().all(|other_cross| {
                                    (cross - other_cross).abs() as f32 >= MIN_SCREEN_SPACING * view.zoom
                                })
                            })
                    })
                })
                .min_by(|a, b| a.centre_distance().total_cmp(&b.centre_distance()));
            let Some(next) = next else { break };
            self.frame.push(next);
        }
        let fill = coverage_fill(level.theme);
        let covered: f32 = self.frame.iter()
            .filter(|p| !matches!(p.item.kind, ForeKind::CloudBand | ForeKind::PowerLines))
            .map(|p| p.body_area(view.size) * fill)
            .sum();
        let cap = view.size.x * view.size.y * coverage_pct(level.theme) as f32 / 100.0;
        if covered > cap {
            let factor = (cap / covered).max(COVERAGE_ALPHA_MIN);
            for plan in &mut self.frame {
                if plan.item.kind != ForeKind::CloudBand {
                    plan.alpha = (plan.alpha as f32 * factor).round() as u8;
                }
            }
        }
        // Two entries at most. Draw the far one first, retaining selection
        // order when their depths tie.
        if self.frame.len() == 2 && self.frame[0].item.depth > self.frame[1].item.depth {
            self.frame.swap(0, 1);
        }
        if let [back, front] = self.frame.as_mut_slice() {
            let near_rat = |p: &Plan| {
                p.body.fade > 0.0
                    && p.rat_distance <= p.half_thickness + CLEAR_DISC_IN_PX * view.zoom
            };
            front.crossing_rat = back.item.axis != front.item.axis
                && back.item.kind != ForeKind::CloudBand
                && near_rat(back)
                && near_rat(front);
        }
    }
}

/// The scene is still composited at world resolution, before resampling.
/// Multiply screen-space distances by the eased zoom to get buffer distances.
#[derive(Clone, Copy)]
struct View {
    cam: Vec2,
    size: Vec2,
    zoom: f32,
}

fn blur_radius(item: ForeItem, zoom: f32) -> i32 {
    // Depth is 1.8–3.0. Keep the radius in screen pixels as the view eases.
    (((item.depth - DEPTH_MIN) / (DEPTH_MAX - DEPTH_MIN) * 2.0 + 2.0)
        * zoom).round().max(1.0) as i32
}

fn cross_extent(item: ForeItem, block: i32, pad: i32) -> (f32, f32) {
    let samples = art_foreground::cross_samples(&item);
    ((samples.start * block - pad) as f32, (samples.end * block + pad) as f32)
}

fn shadow_offset(item: ForeItem) -> f32 {
    10.0 + 4.0 * item.axis.x.abs()
}

fn plan_band(view: View, item: ForeItem, rat: Vec2, threats: &[(Vec2, f32)]) -> Option<Plan> {
    let normal = item.normal();
    let view_span = project(view.size, normal);
    let centre_cross = (view_span.0 + view_span.1) * 0.5;
    let world_cross = item.cross - view.cam.dot(normal);
    let screen_cross = centre_cross + (world_cross - centre_cross) * item.depth;
    let k = pixel_scale(&item);
    let radius = blur_radius(item, view.zoom);
    // Account for the tent on the body and the half-sample filter on the shadow.
    let mut body = Plane::new(screen_cross, cross_extent(item, k, radius), view_span, view.zoom);
    let shadow = Plane::new(
        world_cross + shadow_offset(item), cross_extent(item, k, k / 2), view_span, view.zoom,
    );
    if item.kind == ForeKind::CloudBand {
        body.fade = 0.0;
    }
    if body.fade == 0.0 && shadow.fade == 0.0 {
        return None;
    }
    let half_thickness = art_foreground::thickness(&item) * 0.5 * k as f32 + radius as f32;
    let line_distance = |at: Vec2| ((at - view.cam).dot(normal) - body.cross as f32).abs();
    let rat_distance = line_distance(rat);
    let mut alpha = proximity_alpha(
        ALPHA_MAX as f32, rat_distance / view.zoom, FADE_RAT_PX, ALPHA_FLOOR_RAT,
    );
    for &(body, radius) in threats {
        let distance = (line_distance(body) - half_thickness - radius.max(0.0)).max(0.0);
        alpha = alpha.min(proximity_alpha(
            ALPHA_MAX as f32, distance / view.zoom, FADE_THREAT_PX, ALPHA_FLOOR,
        ));
    }
    if item.kind == ForeKind::CloudBand {
        alpha = alpha.min(CLOUD_ALPHA_MAX as f32);
    }
    Some(Plan {
        item, normal, body, shadow,
        centre_cross, along: project(view.size, item.axis), half_thickness, rat_distance,
        alpha: alpha.round() as u8, k, radius, crossing_rat: false,
    })
}

pub struct ForeScene<'a> {
    pub ecs: &'a mut EcsWorld,
    pub level: &'a Level,
    pub cam: Vec2,
    /// Actual eased world pixels per screen pixel.
    pub zoom: f32,
    pub rat: Vec2,
    pub elapsed: f32,
}

/// Draw after lighting and before the hostile layer. Returns the number of
/// body writes charged against the coverage guard.
pub fn draw(pb: &mut PixelBuffer, layer: &mut ForeLayer, scene: &mut ForeScene) -> u32 {
    if pb.w == 0 || pb.h == 0 {
        return 0;
    }
    layer.sync(scene.level);
    let view = View { cam: scene.cam, size: Vec2::new(pb.w as f32, pb.h as f32), zoom: scene.zoom };
    let mut threats = [(Vec2::ZERO, 0.0f32); 8];
    let mut n = 0;
    {
        let mut q = scene.ecs.query::<(&Enemy, &Miniboss)>();
        for (enemy, _) in q.iter(scene.ecs).take(threats.len()) {
            threats[n] = (enemy.pos, enemy.radius.max(8.0));
            n += 1;
        }
    }
    {
        let mut q = scene.ecs.query::<&Boss>();
        for boss in q.iter(scene.ecs).take(threats.len() - n) {
            threats[n] = (boss.pos, BOSS_BODY_RADIUS);
            n += 1;
        }
    }
    layer.plan_frame(view, scene.level, scene.rat, scene.elapsed, &threats[..n]);
    if layer.frame.is_empty() {
        return 0;
    }

    let total = (pb.w * pb.h) as u32;
    let cap_full = total * coverage_pct(scene.level.theme) / 100;
    let cap_skip = total * coverage_skip_pct(scene.level.theme) / 100;
    budget_start(cap_skip);
    clear_disc_set(scene.rat - scene.cam, view.zoom);
    for (plan, raster) in layer.frame.iter().zip(&mut layer.rasters) {
        let painted = writes_so_far();
        let est_writes = if scene.level.theme == Theme::Outfall {
            plan.body_area(view.size) * OUTFALL_COVERAGE_FILL
        } else {
            plan.body_area(view.size)
        };
        let alpha = if est_writes > cap_skip.saturating_sub(painted) as f32 {
            // Keep the whole run visible as haze when it might exceed the
            // hard budget. Stopping midway would leave a visible band end.
            plan.alpha.min(CLOUD_ALPHA_MAX)
        } else {
            coverage_alpha(plan.alpha, painted, cap_full)
        };
        raster.draw(pb, &mut layer.scratch, view, plan, alpha, scene);
    }
    clear_disc_reset();
    budget_finish()
}

#[derive(Clone, Copy, PartialEq)]
struct RasterKey {
    kind: ForeKind,
    seed: u32,
    scale: f32,
    theme: Theme,
    sway: f32,
    toward_centre: i8,
    block: i32,
    radius: i32,
}

#[derive(Default)]
struct Focus {
    key: Option<RasterKey>,
    /// The extent is on the world-fixed, along-axis art lattice.
    first: i32,
    last: i32,
    width: i32,
    top: i32,
    bottom: i32,
    body: Vec<[u8; 4]>,
    shadow: Vec<u8>,
    #[cfg(test)]
    rebuilds: usize,
}

impl Focus {
    fn draw(
        &mut self,
        pb: &mut PixelBuffer,
        scratch: &mut Scratch,
        view: View,
        plan: &Plan,
        alpha: u8,
        scene: &ForeScene,
    ) {
        let item = plan.item;
        let block = plan.k;
        let cam_along = view.cam.dot(item.axis);
        let pad = BAND_OVERHANG as f32 * view.zoom + plan.radius as f32 + 1.0;
        let (lo, hi) = plan.along;
        let first = ((cam_along + lo - pad) / block as f32).floor() as i32;
        let last = ((cam_along + hi + pad) / block as f32).ceil() as i32;
        let key = RasterKey {
            kind: item.kind,
            seed: item.seed,
            scale: item.scale,
            theme: scene.level.theme,
            sway: sway_of(&item, scene.elapsed),
            toward_centre: if plan.body.cross as f32 <= plan.centre_cross { 1 } else { -1 },
            block,
            radius: plan.radius,
        };
        // Keep both planes warm on the same world-aligned page as either
        // one enters or leaves the view. Edge opacity is only used at blit.
        if self.key != Some(key) || first < self.first || last > self.last {
            self.first = first.div_euclid(RASTER_PAGE) * RASTER_PAGE;
            self.last = last.div_euclid(RASTER_PAGE) * RASTER_PAGE + RASTER_PAGE;
            let width = self.last - self.first;
            let art_half = (plan.half_thickness / block as f32).ceil() as i32 + 2;
            scratch.reset(0, -art_half, width, 2 * art_half + 1);
            let d = ForeDraw {
                screen_cross: 0,
                shadow_cross: 0,
                from: 0,
                to: width - 1,
                scale: key.scale,
                alpha: ALPHA_MAX,
                sway: key.sway,
                theme: key.theme,
                along_offset: self.first,
                elapsed_step: 0,
                toward_centre: key.toward_centre,
            };
            TARGET.with(|t| *t.borrow_mut() = Some(std::mem::take(scratch)));
            art_foreground::draw(pb, &ForeItem { axis: HORIZONTAL, ..item }, &d);
            *scratch = TARGET.with(|t| t.borrow_mut().take()).unwrap_or_default();
            self.filter(scratch, key);
            self.key = Some(key);
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }

        let along_origin = (self.first * block) as f32 - cam_along;
        let opacity = u32::from(alpha);
        let shadow_opacity = if item.kind == ForeKind::CloudBand {
            opacity * 256 / u32::from(ALPHA_MAX)
        } else {
            141 + opacity * 115 / u32::from(ALPHA_MAX)
        };
        let shadow_opacity = (shadow_opacity as f32 * plan.shadow.fade).round() as u32;
        let body_opacity = (opacity as f32 * plan.body.fade).round() as u32;
        self.blit(pb, plan, along_origin, shadow_opacity, true);
        self.blit(pb, plan, along_origin, body_opacity, false);
    }

    /// Filter the expanded art with a tent kernel in each direction. Each
    /// pass carries premultiplied RGB so empty samples soften the silhouette
    /// without adding a dark fringe. Normalize and quantize once per rebuild.
    fn filter(&mut self, scratch: &mut Scratch, key: RasterKey) {
        let block = key.block;
        let radius = key.radius;
        let divisor = ((radius + 1) * (radius + 1)) as u32;
        self.width = scratch.w * block;
        self.top = scratch.y0 * block;
        self.bottom = self.top + scratch.h * block;
        let width = self.width as usize;
        let rows = scratch.h as usize;
        let len = width * (self.bottom - self.top) as usize;
        self.body.resize(len, [0; 4]);
        self.shadow.resize(len, 0);

        let ambient = crate::render::lighting::theme_ambient(key.theme).mul;
        let ambient = (ambient[0] as u8 / 2, ambient[1] as u8 / 2, ambient[2] as u8 / 2);
        if key.kind != ForeKind::CloudBand {
            scratch.filtered_rows.resize(width * rows, [0; 4]);
            for (source, row) in scratch.body.chunks_exact(scratch.w as usize)
                .zip(scratch.filtered_rows.chunks_exact_mut(width))
            {
                for (x, dst) in row.iter_mut().enumerate() {
                    *dst = tent(x as i32, block, radius, divisor, |i| {
                        source.get(i as usize).copied().unwrap_or_default()
                    });
                }
            }
            for (y, row) in self.body.chunks_exact_mut(width).enumerate() {
                for (x, dst) in row.iter_mut().enumerate() {
                    let [r, g, b, a] = tent(y as i32, block, radius, divisor, |i| {
                        if (0..scratch.h).contains(&i) {
                            scratch.filtered_rows[i as usize * width + x]
                        } else {
                            [0; 4]
                        }
                    });
                    *dst = if a < 6 {
                        [0; 4]
                    } else {
                        let rgb = ((r / a) as u8, (g / a) as u8, (b / a) as u8);
                        let gray = crate::render::lighting::luminance(rgb);
                        let muted = palette::mix(palette::scale(rgb, DARKEN), (gray, gray, gray), 0.18);
                        let tint = palette::mix(muted, ambient, 0.16);
                        [stepped(tint.0 as u32, 8, 255), stepped(tint.1 as u32, 8, 255),
                            stepped(tint.2 as u32, 8, 255), stepped(a, 12, ALPHA_MAX)]
                    };
                }
            }
        }
        // The floor shadow is sharper. Interpolate between the art sample
        // centres, giving it only half an art pixel of softness per side.
        for (y, row) in self.shadow.chunks_exact_mut(width).enumerate() {
            let (v, fy) = shadow_phase(y as i32, block);
            for (x, dst) in row.iter_mut().enumerate() {
                let (u, fx) = shadow_phase(x as i32, block);
                let sample = |x, y| {
                    scratch.index(x, y + scratch.y0).map_or(0, |i| u32::from(scratch.shade[i]))
                };
                let a = sample(u, v) * (256 - fx) + sample(u + 1, v) * fx;
                let b = sample(u, v + 1) * (256 - fx) + sample(u + 1, v + 1) * fx;
                *dst = stepped((a * (256 - fy) + b * fy + 32768) / 65536, 8, 255);
            }
        }
    }

    fn blit(&self, pb: &mut PixelBuffer, plan: &Plan, along_origin: f32, opacity: u32, shade: bool) {
        if opacity == 0 {
            return;
        }
        let axis = plan.item.axis;
        let normal = plan.normal;
        let cross = if shade { plan.shadow.cross } else { plan.body.cross } as f32;
        let (width, height) = (pb.w, pb.h);
        let (cx, cy, zoom, disc) = CLEAR_DISC.with(|c| c.get());
        let disc_active = disc && (!shade || plan.crossing_rat);
        let fade_width = 12.0;
        let limit = (CLEAR_DISC_OUT_PX + if plan.crossing_rat { fade_width } else { 0.0 }) * zoom;
        let limit2 = limit * limit;
        let rows = pb.pixels_mut().chunks_exact_mut(width);
        // Invert the band's basis per scanline. Only visit the strip, even if
        // its angled bounding box covers the whole framebuffer.
        for (y, row) in rows.enumerate().take(height) {
            let cross_at_x0 = y as f32 * normal.y - cross;
            let top = self.top as f32;
            let bottom = self.bottom as f32;
            let (from, to) = if normal.x == 0.0 {
                if cross_at_x0 < top || cross_at_x0 >= bottom {
                    continue;
                }
                (0, width)
            } else {
                let lo = (top - cross_at_x0) / normal.x;
                let hi = (bottom - cross_at_x0) / normal.x;
                ((lo.min(hi).floor().max(0.0) as usize).min(width),
                 (lo.max(hi).ceil().max(0.0) as usize).min(width))
            };
            let along_at_x0 = y as f32 * axis.y - along_origin;
            let dy = y as f32 - cy as f32;
            for (x, pixel) in row.iter_mut().enumerate().take(to).skip(from) {
                let u = along_at_x0 + x as f32 * axis.x;
                let v = cross_at_x0 + x as f32 * normal.x - top;
                if u < 0.0 || u >= self.width as f32 || v < 0.0 || v >= bottom - top {
                    continue;
                }
                let i = v as usize * self.width as usize + u as usize;
                let [r, g, b, a] = if shade { [0, 0, 0, self.shadow[i]] } else { self.body[i] };
                if a == 0 {
                    continue;
                }
                let mut a = u32::from(a) * opacity;
                if disc_active {
                    let dx = x as f32 - cx as f32;
                    let d2 = dx * dx + dy * dy;
                    if d2 < limit2 {
                        let d = d2.sqrt() / zoom;
                        let factor = if plan.crossing_rat {
                            ((d - CLEAR_DISC_OUT_PX) / fade_width).clamp(0.0, 1.0)
                        } else if d <= RAT_SKIP_PX {
                            0.0
                        } else {
                            let t = ((d - CLEAR_DISC_IN_PX) / (CLEAR_DISC_OUT_PX - CLEAR_DISC_IN_PX))
                                .clamp(0.0, 1.0);
                            CLEAR_DISC_MIN + (1.0 - CLEAR_DISC_MIN) * t
                        };
                        a = (a as f32 * factor) as u32;
                    }
                }
                let a = if shade { a / 256 } else { a / u32::from(ALPHA_MAX) };
                if a > u32::from(CLOUD_ALPHA_MAX) && !shade && !take_budget() {
                    continue;
                }
                if a != 0 {
                    let mix = |src: u8, dst: u8| {
                        ((u32::from(src) * a + u32::from(dst) * (255 - a) + 127) / 255) as u8
                    };
                    *pixel = (mix(r, pixel.0), mix(g, pixel.1), mix(b, pixel.2));
                }
            }
        }
    }
}

/// Collapse the tent taps that land in the same art sample.
#[inline]
fn tent(
    at: i32, block: i32, radius: i32, divisor: u32, mut get: impl FnMut(i32) -> [u32; 4],
) -> [u32; 4] {
    let mut sum = [0; 4];
    let mut p = at - radius;
    let last = at + radius;
    while p <= last {
        let sample = p.div_euclid(block);
        let end = ((sample + 1) * block - 1).min(last);
        let weight = (p..=end).map(|q| radius + 1 - (q - at).abs()).sum::<i32>() as u32;
        let rgba = get(sample);
        for i in 0..4 {
            sum[i] += weight * rgba[i];
        }
        p = end + 1;
    }
    sum.map(|v| (v + divisor / 2) / divisor)
}

fn shadow_phase(at: i32, block: i32) -> (i32, u32) {
    let shifted = 2 * at - (block - 1);
    (shifted.div_euclid(2 * block), (shifted.rem_euclid(2 * block) * 128 / block) as u32)
}

fn stepped(value: u32, step: u32, max: u8) -> u8 {
    ((value + step / 2) / step * step).min(u32::from(max)) as u8
}

#[inline]
fn coverage_alpha(alpha: u8, painted: u32, cap_full: u32) -> u8 {
    if painted < cap_full {
        return alpha;
    }
    (alpha as f32 * COVERAGE_HALF).round() as u8
}

/// Alpha for an item whose nearest pixel is `d` px from the rat/a body:
/// [`ALPHA_FLOOR`] inside `near`, ramping back to `base` over
/// [`FADE_RAMP_PX`].
fn proximity_alpha(base: f32, d: f32, near: f32, floor: f32) -> f32 {
    if d <= near {
        floor.min(base)
    } else if d >= near + FADE_RAMP_PX {
        base
    } else {
        let t = (d - near) / FADE_RAMP_PX;
        (floor + (base - floor) * t).min(base)
    }
}

/// Sway for `item` at `elapsed`: `0.0` for a kind that never sways, else a
/// value in `-1.0..=1.0` quantised to quarters and **stepped at [`SWAY_HZ`]** —
/// within one step the value is constant, so a still camera does not repaint
/// between steps.
fn sway_of(item: &ForeItem, elapsed: f32) -> f32 {
    if !item.kind.sways() {
        return 0.0;
    }
    let step = (elapsed.max(0.0) * SWAY_HZ).floor() as u32;
    let h = mix32(step.wrapping_mul(0x9E37_79B1) ^ item.seed);
    let unit = (h & 0xFFFF) as f32 / 65535.0; // 0..=1
    let v = unit * 2.0 - 1.0;
    (v * 4.0).round() * 0.25
}

/// One 32-bit splitmix-ish mix (cheap, deterministic, no rng dependency).
#[inline]
fn mix32(mut z: u32) -> u32 {
    z ^= z >> 16;
    z = z.wrapping_mul(0x7FEB_352D);
    z ^= z >> 15;
    z = z.wrapping_mul(0x846C_A68B);
    z ^ (z >> 16)
}

// ---- Generation ----------------------------------------------------------

/// A tiny deterministic stream: splitmix64 over a counter. The layer's own
/// stream, salted away from the level generator's.
struct Gen {
    state: u64,
}

impl Gen {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform value in `lo..hi`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * unit
    }

    fn u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
}


// Roll candidates from the usual wide rooms and long corridor runs, then
// spread a small, seeded selection across the level.
fn generate(level: &Level, out: &mut Vec<PlacedBand>) {
    if level.theme == Theme::Trailer {
        return;
    }
    let mut rng = Gen::new(crate::game::level::level_seed(level.theme, level.seed, GEN_SALT));
    let count = 2 + rng.u32() % 3;
    let kinds = kinds_for(level.theme);
    let start = level.rooms.iter().find(|room| room.kind == RoomKind::Start);
    let mut candidates = Vec::new();
    for room in &level.rooms {
        if matches!(room.kind, RoomKind::Start | RoomKind::Maze) {
            continue;
        }
        let rect = room.rect;
        let short = rect.w.min(rect.h);
        let long = rect.w.max(rect.h);
        let corridor = room.kind == RoomKind::Corridor;
        let slots = if corridor {
            if long < CORRIDOR_MIN_TILES {
                continue;
            }
            1
        } else {
            if short < ROOM_MIN_SHORT || long < ROOM_MIN_LONG {
                continue;
            }
            (rect.w * rect.h / ROOM_TILES2_PER_BAND).clamp(1, 2)
        };
        let centre = rect.center_world();
        let size = Vec2::new(rect.w as f32, rect.h as f32);
        let quality = if corridor {
            (long as f32 / 30.0).min(1.0)
        } else {
            (size.x * size.y / 320.0).min(1.0)
        };
        let mut room_has_rib = false;
        for _ in 0..slots {
            for attempt in 0..PLACE_ATTEMPTS {
                // Corridors always sample their centre line. Later room
                // attempts try a perpendicular axis for the rigid kinds.
                let horizontal = (rect.w < rect.h) ^ (!corridor && attempt >= PLACE_ATTEMPTS / 2);
                let extent = if !corridor {
                    size * 0.5 - Vec2::splat(1.0)
                } else if horizontal {
                    Vec2::new(0.0, (size.y - 1.0) * 0.5)
                } else {
                    Vec2::new((size.x - 1.0) * 0.5, 0.0)
                };
                let at = centre + Vec2::new(
                    rng.range(-extent.x, extent.x),
                    rng.range(-extent.y, extent.y),
                ) * TILE_F;
                if !level.tiles.get_at(at).is_open() {
                    continue;
                }

                let mut kind = kinds[(rng.u32() % kinds.len() as u32) as usize];
                let depth = if level.theme == Theme::Outfall {
                    rng.range(OUTFALL_DEPTH_MIN, OUTFALL_DEPTH_MAX)
                } else {
                    rng.range(DEPTH_MIN, DEPTH_MAX)
                };
                let scale = rng.range(SCALE_MIN, SCALE_MAX);
                let seed = rng.u32();
                if !corridor && kind == ForeKind::HullRib && room_has_rib {
                    kind = if seed & 1 == 0 { ForeKind::Kelp } else { ForeKind::BubbleChain };
                }
                let axis = placement_axis(kind, horizontal, seed);
                let mut item = ForeItem { kind, axis, cross: 0.0, depth, scale, seed };
                let normal = item.normal();
                item.cross = at.dot(normal);
                if (item.cross - level.start.dot(normal)).abs() < START_CLEAR_PX
                    || start.is_some_and(|r| {
                        let half_size = Vec2::new(r.rect.w as f32, r.rect.h as f32) * (TILE_F * 0.5);
                        let reach = half_size.x * normal.x.abs() + half_size.y * normal.y.abs();
                        (item.cross - r.rect.center_world().dot(normal)).abs() <= reach
                    })
                {
                    continue;
                }
                room_has_rib |= kind == ForeKind::HullRib;
                candidates.push((PlacedBand {
                    item: fit_scale(item),
                    anchor: at,
                    #[cfg(test)]
                    corridor,
                }, quality));
                break;
            }
        }
    }

    for _ in 0..count {
        let best = candidates.iter().enumerate().filter_map(|(i, (band, quality))| {
            let mut spacing = band.anchor.dist_sq(level.start);
            for placed in out.iter() {
                if band.item.axis == placed.item.axis
                    && (band.item.cross - placed.item.cross).abs() < MIN_SPACING
                {
                    return None;
                }
                spacing = spacing.min(placed.anchor.dist_sq(band.anchor));
            }
            let fresh_organic = band.item.kind.organic()
                && out.iter().all(|p| p.item.kind != band.item.kind);
            Some((i, fresh_organic, spacing * (1.0 + quality * 0.50)))
        }).max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)));
        let Some((index, _, _)) = best else { break };
        out.push(candidates.swap_remove(index).0);
    }
}

/// Keep each kind inside the art thickness range even at the ends of the
/// scale roll (for example, a narrow wire requires slightly more scale).
fn fit_scale(mut item: ForeItem) -> ForeItem {
    let unit = ForeItem { scale: 1.0, ..item };
    let base = art_foreground::thickness(&unit);
    item.scale = item.scale.clamp((10.0 / base).max(SCALE_MIN), (30.0 / base).min(SCALE_MAX));
    item
}

fn hash01(seed: u64, index: u64) -> f32 {
    (hash_u32(seed, index) & 0x00FF_FFFF) as f32 / 16_777_215.0
}

fn hash_u32(seed: u64, index: u64) -> u32 {
    mix32(seed as u32 ^ (seed >> 32) as u32 ^ mix32(index as u32 ^ 0x5BD1_E995))
}

/// Cross coordinates, relative to the camera, where a sky track can wrap or
/// stop with both planes outside the viewport.
fn sky_track(view: View, item: ForeItem) -> (f32, f32) {
    let (lo, hi) = project(view.size, item.normal());
    let k = pixel_scale(&item);
    let (shadow_lo, shadow_hi) = cross_extent(item, k, k / 2);
    let offset = shadow_offset(item);
    let mut start = lo - shadow_hi - offset;
    let mut end = hi - shadow_lo - offset;
    if item.kind != ForeKind::CloudBand {
        let (body_lo, body_hi) = cross_extent(item, k, blur_radius(item, view.zoom));
        let centre = (lo + hi) * 0.5;
        start = start.min(centre + (lo - body_hi - centre) / item.depth);
        end = end.max(centre + (hi - body_lo - centre) / item.depth);
    }
    // Allow for rounding either plane's centre to buffer pixels.
    (start - 1.0, end + 1.0)
}

/// Analytic tracks measured across each band's normal. Along-axis detail
/// still follows the world camera.
fn sky_items(level: &Level, view: View, elapsed: f32) -> [Option<ForeItem>; 3] {
    if level.theme != Theme::Trailer {
        return [None; 3];
    }
    let seed = crate::game::level::level_seed(level.theme, level.seed, GEN_SALT);
    let t = (elapsed.max(0.0) * SKY_QUANT_HZ).floor() / SKY_QUANT_HZ;
    let item = |kind, horizontal, depth, scale, seed| {
        let axis = placement_axis(kind, horizontal, seed);
        fit_scale(ForeItem { kind, axis, cross: 0.0, depth, scale, seed })
    };

    // The centre line keeps the wires present even when both movers qualify.
    let mut wires = item(ForeKind::PowerLines, true, 2.0, 1.15, hash_u32(seed, 0x12));
    wires.cross = (view.cam + view.size * 0.5).dot(wires.normal());

    let mut cloud = item(ForeKind::CloudBand, false, 1.8, 1.25, hash_u32(seed, 0xC10D));
    let (start, end) = sky_track(view, cloud);
    let travel = end - start;
    let speed = SKY_CLOUD_SPEED * view.zoom / cloud.depth;
    let phase = (hash01(seed, 0xC10D) * travel + t * speed).rem_euclid(travel);
    cloud.cross = view.cam.dot(cloud.normal()) + start + phase;

    // Successive starts differ by 5–9 s. The branch travels from the cab
    // side of the frame to the tail (right to left).
    let jitter = |event| (hash01(seed ^ 0x5EED_0000, event) - 0.5) * 2.0;
    let nearest = (t / SKY_BRANCH_PERIOD).floor() as u64;
    let branch = (nearest.saturating_sub(1)..=nearest + 1).rev().find_map(|event| {
        let age = t - (SKY_BRANCH_PERIOD * event as f32 + jitter(event));
        if age < 0.0 || age >= SKY_BRANCH_PERIOD + 2.0 {
            return None;
        }
        let mut branch = item(ForeKind::BranchCanopy, false, 2.8, 1.4,
            hash_u32(seed ^ 0x1EAF_0000, event));
        let normal = branch.normal();
        let (start, end) = sky_track(view, branch);
        // Fit the crossing before the next event even on very wide views.
        let speed = (SKY_BRANCH_SPEED * view.zoom / branch.depth)
            .max((end - start) / (SKY_BRANCH_PERIOD - 2.0));
        let travelled = age * speed;
        (travelled < end - start).then(|| {
            let cross = if normal.x < 0.0 { start + travelled } else { end - travelled };
            branch.cross = view.cam.dot(normal) + cross;
            branch
        })
    });
    [Some(wires), Some(cloud), branch]
}

/// The count and spacing rules also apply to capture staging. Use the same
/// plan to find a frame in which the requested band is visible.
#[cfg(test)]
pub(crate) fn sky_capture_time(
    level: &Level,
    cam: Vec2,
    size: Vec2,
    zoom: f32,
    elapsed: f32,
    kind: ForeKind,
) -> Option<f32> {
    if level.theme != Theme::Trailer {
        return None;
    }
    let view = View { cam, size, zoom };
    let mut layer = ForeLayer::new();
    for step in 0..(40 * SKY_QUANT_HZ as i32) {
        let t = elapsed + step as f32 / SKY_QUANT_HZ;
        layer.plan_frame(view, level, cam + size * 0.5, t, &[]);
        if layer.frame.iter().any(|p| {
            let plane = if kind == ForeKind::CloudBand { p.shadow } else { p.body };
            p.item.kind == kind && plane.fade > 0.0
                && (plane.cross as f32 - p.centre_cross).abs() <= 12.0 * zoom
        }) {
            return Some(t);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::world::World;

    const FLOOR: Rgb = (70, 66, 60);
    const VIEW: (usize, usize) = (120, 80);

    fn pipe(horizontal: bool, cross: f32, depth: f32) -> ForeItem {
        let axis = if horizontal { HORIZONTAL } else { VERTICAL };
        ForeItem { kind: ForeKind::MainPipe, axis, cross, depth, scale: 1.0, seed: 37 }
    }

    fn axes<T>(horizontal: bool, x: T, y: T) -> (T, T) {
        if horizontal { (x, y) } else { (y, x) }
    }

    fn force_items(layer: &mut ForeLayer, level: &Level, items: &[ForeItem]) {
        layer.key = Some((level.theme, level.seed));
        layer.items = items.iter().map(|&item| PlacedBand {
            item, anchor: Vec2::ZERO, corridor: false,
        }).collect();
    }

    fn render(
        world: &mut World,
        layer: &mut ForeLayer,
        cam: Vec2,
        zoom: f32,
        elapsed: f32,
    ) -> (PixelBuffer, u32) {
        let mut pb = PixelBuffer::new((VIEW.0 as f32 * zoom) as usize, (VIEW.1 as f32 * zoom) as usize);
        pb.clear(FLOOR);
        let mut scene = ForeScene {
            ecs: &mut world.ecs,
            level: &world.level,
            cam,
            zoom,
            rat: cam - Vec2::new(200.0, 200.0),
            elapsed,
        };
        let coverage = draw(&mut pb, layer, &mut scene);
        (pb, coverage)
    }

    #[test]
    fn placement_is_seeded_and_every_item_is_a_clear_spaced_band() {
        for theme in Theme::all() {
            for seed in [0, 1, 2, 7, 11, 21, 54, 809] {
                let world = World::new(theme, seed);
                let mut a = ForeLayer::new();
                let mut b = ForeLayer::new();
                a.sync(&world.level);
                b.sync(&world.level);
                let a: Vec<_> = a.capture_items().collect();
                assert_eq!(a, b.capture_items().collect::<Vec<_>>());
                assert!((2..=4).contains(&a.len()), "{theme:?} seed {seed}: {} bands", a.len());
                for (i, &(item, anchor, corridor)) in a.iter().enumerate() {
                    let normal = item.normal();
                    assert_eq!(item.cross, anchor.dot(normal));
                    assert!((item.cross - world.level.start.dot(normal)).abs() >= START_CLEAR_PX);
                    assert!(world.level.tiles.get_at(anchor).is_open());
                    if let Some(start) = world.level.rooms.iter().find(|r| r.kind == RoomKind::Start) {
                        let half = Vec2::new(start.rect.w as f32, start.rect.h as f32) * (TILE_F * 0.5);
                        let projected = half.x * normal.x.abs() + half.y * normal.y.abs();
                        assert!((item.cross - start.rect.center_world().dot(normal)).abs() > projected);
                    }
                    if !item.kind.organic() {
                        assert!(item.axis == HORIZONTAL || item.axis == VERTICAL);
                    }
                    assert!(kinds_for(theme).contains(&item.kind));
                    assert!((DEPTH_MIN..=DEPTH_MAX).contains(&item.depth));
                    assert!((SCALE_MIN..=SCALE_MAX).contains(&item.scale));
                    assert!((9.999..=30.001).contains(&art_foreground::thickness(&item)));
                    assert!(a[..i].iter().all(|(other, _, _)| {
                        other.axis != item.axis || (other.cross - item.cross).abs() >= MIN_SPACING
                    }));
                    if corridor {
                        let (cx, cy) = ((anchor.x / TILE_F) as i32, (anchor.y / TILE_F) as i32);
                        assert!(world.level.rooms.iter().any(|r| {
                            r.kind == RoomKind::Corridor
                                && (item.kind.organic() || (r.rect.w < r.rect.h) == (item.axis == HORIZONTAL))
                                && r.rect.w.max(r.rect.h) >= CORRIDOR_MIN_TILES
                                && (r.rect.x..r.rect.right()).contains(&cx)
                                && (r.rect.y..r.rect.bottom()).contains(&cy)
                        }));
                    }
                }
            }
        }
    }

    #[test]
    fn the_body_crosses_both_edges_at_every_zoom_and_orientation() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
        let cam = Vec2::new(321.0, 401.0);
        for zoom in [0.5, 1.0, 1.35, 2.0] {
            let size = Vec2::new(VIEW.0 as f32, VIEW.1 as f32) * zoom;
            let centre = size * 0.5;
            for axis in [HORIZONTAL, VERTICAL, Vec2::new(0.6, 0.8), Vec2::new(0.8, -0.6)] {
                let mut item = ForeItem { axis, ..pipe(true, 0.0, 2.0) };
                item.cross = (cam + centre).dot(item.normal());
                force_items(&mut layer, &world.level, &[item]);
                let (mut pb, used) = render(&mut world, &mut layer, cam, zoom, 0.0);
                assert!(used > 0, "no body drawn");
                let raster = &layer.rasters[0];
                let plan = &layer.frame[0];
                let cam_along = cam.dot(axis);
                let extent = |p| (p * plan.k) as f32 - cam_along;
                let overhang = BAND_OVERHANG as f32 * zoom;
                assert!(extent(raster.first) <= plan.along.0 - overhang);
                assert!(extent(raster.last) >= plan.along.1 + overhang);

                // The shadow is separately offset. Verify the body against
                // each actual viewport edge that its centreline crosses.
                pb.clear(FLOOR);
                raster.blit(&mut pb, plan, extent(raster.first), u32::from(ALPHA_MAX), false);
                let (first, last) = line_span(centre, axis, size);
                for t in [first, last] {
                    let edge = centre + axis * t;
                    let x = edge.x.round().clamp(0.0, size.x - 1.0) as usize;
                    let y = edge.y.round().clamp(0.0, size.y - 1.0) as usize;
                    assert_ne!(pb.pixel_at(x, y), FLOOR, "missing edge {edge:?} at {zoom}x, axis {axis:?}");
                }
            }
        }
    }

    #[test]
    fn shadows_enter_from_both_edges_while_the_body_is_offscreen() {
        let mut world = World::new(Theme::Sewer, 7);
        let zoom = 2.0;
        let size = Vec2::new(VIEW.0 as f32, VIEW.1 as f32) * zoom;
        for axis in [HORIZONTAL, VERTICAL, Vec2::new(0.6, 0.8)] {
            let item = ForeItem { axis, ..pipe(true, 0.0, 3.0) };
            let normal = item.normal();
            let (lo, hi) = project(size, normal);
            let k = pixel_scale(&item);
            let (shadow_lo, shadow_hi) = cross_extent(item, k, k / 2);
            for (edge, start, inward) in [(lo, lo - shadow_hi, 1.0), (hi, hi - shadow_lo, -1.0)] {
                let mut layer = ForeLayer::new();
                force_items(&mut layer, &world.level, &[item]);
                let camera = |step| normal * (shadow_offset(item) - (start + inward * step as f32));
                let mut depths = Vec::new();
                for step in -2..=28 {
                    let cam = camera(step);
                    let (pb, used) = render(&mut world, &mut layer, cam, zoom, 0.0);
                    assert_eq!(used, 0);
                    if let Some(plan) = layer.frame.first() {
                        assert_eq!(plan.body.fade, 0.0);
                        assert_eq!(plan.body_area(size), 0.0);
                    }
                    let deepest = pb.pixels().iter().enumerate().filter(|(_, px)| **px != FLOOR)
                        .map(|(i, &(r, g, b))| {
                            assert!(r <= FLOOR.0 && g <= FLOOR.1 && b <= FLOOR.2);
                            let xy = Vec2::new((i % pb.w) as f32, (i / pb.w) as f32);
                            (xy.dot(normal) - edge) * inward
                        })
                        .reduce(f32::max);
                    if let Some(depth) = deepest {
                        depths.push((step, depth));
                    }
                }
                let &(first_step, first_depth) = depths.first().expect("shadow never reached the viewport");
                assert!(first_step > 0 && first_depth <= 4.0, "{axis:?}, edge {edge}: {depths:?}");
                let growth = depths.windows(2).filter(|w| w[1].0 == w[0].0 + 1 && w[1].1 > w[0].1).count();
                assert!(growth >= 5, "{axis:?}, edge {edge}: {depths:?}");
                let (inside, _) = render(&mut world, &mut layer, camera(28), zoom, 0.0);
                let plan = &layer.frame[0];
                let raster = &layer.rasters[0];
                let along = (raster.first * k) as f32 - camera(28).dot(axis);
                let mut body = PixelBuffer::new(inside.w, inside.h);
                body.clear(FLOOR);
                raster.blit(&mut body, plan, along, u32::from(ALPHA_MAX), false);
                assert!(body.pixels().iter().all(|&px| px == FLOOR));
                assert_eq!(raster.rebuilds, 1, "cross-axis motion should reuse the same raster");

                // The last body-free frame is identical when frozen and when
                // returning to it after the body has crossed the viewport.
                let (still, _) = render(&mut world, &mut layer, camera(28), zoom, 0.0);
                assert_eq!(inside.pixels(), still.pixels());
                let centre_step = (((lo + hi) * 0.5 + shadow_offset(item)) - start) * inward;
                let (_, used) = render(&mut world, &mut layer, camera(centre_step.round() as i32), zoom, 0.0);
                assert!(used > 0);
                let (leaving, used) = render(&mut world, &mut layer, camera(28), zoom, 0.0);
                assert_eq!(used, 0);
                assert_eq!(inside.pixels(), leaving.pixels());
                let (outside, _) = render(&mut world, &mut layer, camera(-2), zoom, 0.0);
                assert!(outside.pixels().iter().all(|&px| px == FLOOR));
            }
        }
    }

    #[test]
    fn soft_focus_replaces_hard_blocks_with_intermediate_coverage() {
        let mut art = Scratch::default();
        art.reset(0, -4, 8, 8);
        for y in -2..2 {
            for x in 2..6 {
                art.put(x, y, (100, 60, 30), ALPHA_MAX);
            }
        }
        let mut focus = Focus::default();
        focus.filter(&mut art, RasterKey {
            kind: ForeKind::MainPipe, seed: 0, scale: 1.0, theme: Theme::Sewer,
            sway: 0.0, toward_centre: 1, block: 3, radius: 4,
        });
        let row = &focus.body[12 * focus.width as usize..13 * focus.width as usize];
        let alpha = |x: usize| row[x][3];
        // The left body edge was a hard 3x3 boundary at x=6. The sample
        // immediately outside now has coverage and the inner ramp varies
        // within the same original art pixel.
        assert!(alpha(5) > 0 && alpha(5) < alpha(6));
        assert!(alpha(6) < alpha(7) && alpha(7) < alpha(9));
    }

    #[test]
    fn organic_angles_come_from_level_seeds() {
        let mut angles = Vec::new();
        for seed in [1, 2, 7, 11] {
            let world = World::new(Theme::Outfall, seed);
            let mut layer = ForeLayer::new();
            layer.sync(&world.level);
            for (item, _, _) in layer.capture_items().filter(|(i, _, _)| i.kind.organic()) {
                assert!(item.axis.x.abs() > 0.001 && item.axis.y.abs() > 0.001);
                if !angles.contains(&item.axis) {
                    angles.push(item.axis);
                }
            }
            let mut c = 0;
            let world = World::new(Theme::Trailer, seed);
            let view = View { cam: Vec2::ZERO, size: Vec2::new(120.0, 80.0), zoom: 1.0 };
            for step in 0..(SKY_QUANT_HZ as u32 * 10) {
                let sky = sky_items(&world.level, view, step as f32 / SKY_QUANT_HZ);
                let kinds: Vec<_> = sky.into_iter().flatten().collect();
                assert!((2..=4).contains(&kinds.len()));
                if let Some(branch) = sky[2] {
                    assert!(branch.axis.x.abs() > 0.001 && branch.axis.y.abs() > 0.001);
                    c += 1;
                }
            }
            assert!(c > 0, "no branch passed for seed {seed}");
        }
        assert!(angles.len() >= 4);
    }

    #[test]
    fn angled_parallel_spacing_and_crossing_use_projected_distance() {
        let world = World::new(Theme::Outfall, 7);
        let mut layer = ForeLayer::new();
        let view = View { cam: Vec2::ZERO, size: Vec2::new(180.0, 180.0), zoom: 1.0 };
        let centre = view.size * 0.5;
        let mut near = ForeItem { kind: ForeKind::Kelp, axis: Vec2::new(0.6, 0.8), ..pipe(true, 0.0, 2.0) };
        near.cross = centre.dot(near.normal());
        let close = ForeItem { cross: near.cross + 20.0, ..near };
        let spaced = ForeItem { cross: near.cross - 62.0, ..near };
        force_items(&mut layer, &world.level, &[close, spaced, near]);
        layer.plan_frame(view, &world.level, centre, 0.0, &[]);
        assert_eq!(layer.frame[0].item, near);
        assert_eq!(layer.frame[1].item, spaced);

        let crossing = ForeItem { axis: Vec2::new(0.8, -0.6), cross: 126.0, ..near };
        force_items(&mut layer, &world.level, &[near, crossing]);
        layer.plan_frame(view, &world.level, centre, 0.0, &[]);
        assert_eq!(layer.frame.len(), 2);
        assert!(layer.frame[1].crossing_rat);
        let along = near.axis * 14.0;
        let still = plan_band(view, near, centre, &[]).unwrap();
        let slid = plan_band(View { cam: along, ..view }, near, centre, &[]).unwrap();
        assert_eq!(still.body.cross, slid.body.cross);
    }

    #[test]
    fn only_the_two_nearest_well_spaced_bands_are_selected() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
        for zoom in [1.0, 2.0] {
            let side = (240.0 * zoom) as usize;
            let mut pb = PixelBuffer::new(side, side);
            let centre = pb.h as f32 * 0.5;
            let near = pipe(true, centre, 2.0);
            let too_close = pipe(true, centre + zoom * 47.0, 2.0);
            let spaced = pipe(true, centre + zoom * 48.0, 2.0);
            let farther = pipe(true, centre - zoom * 70.0 - 14.0, 2.0);
            force_items(&mut layer, &world.level, &[farther, too_close, spaced, near]);
            let mut scene = ForeScene {
                ecs: &mut world.ecs,
                level: &world.level,
                cam: Vec2::ZERO,
                zoom,
                rat: Vec2::ZERO,
                elapsed: 0.0,
            };
            draw(&mut pb, &mut layer, &mut scene);
            assert_eq!(layer.frame.len(), MAX_BANDS_IN_VIEW);
            assert_eq!(layer.frame[0].item, near);
            assert_eq!(layer.frame[1].item, spaced);
            assert_eq!(layer.frame[1].shadow.cross - layer.frame[0].shadow.cross, (48.0 * zoom) as i32);
        }
    }

    #[test]
    fn shadow_only_bands_participate_in_the_two_line_spacing_rule() {
        let world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
        let view = View { cam: Vec2::ZERO, size: Vec2::splat(360.0), zoom: 1.0 };
        let rat = view.size * 0.5;
        let nearest = pipe(true, 112.0 - 14.0, 3.0);
        let too_close = pipe(true, 112.0 - 14.0 - 47.0, 3.0);
        let spaced = pipe(true, 310.0 - 14.0, 3.0);
        let items = [spaced, too_close, nearest];
        for item in items {
            let plan = plan_band(view, item, rat, &[]).unwrap();
            assert_eq!(plan.body.fade, 0.0);
            assert!(plan.shadow.fade > 0.0);
        }
        force_items(&mut layer, &world.level, &items);
        layer.plan_frame(view, &world.level, rat, 0.0, &[]);
        assert_eq!(layer.frame.len(), MAX_BANDS_IN_VIEW);
        assert_eq!(layer.frame[0].item, nearest);
        assert_eq!(layer.frame[1].item, spaced);
    }

    #[test]
    fn filtered_detail_is_world_anchored_along_the_band() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
        // At 1x both origins cross an art block boundary, for k = 2 and 3.
        let cam = Vec2::new(323.0, 401.0);
        for horizontal in [true, false] {
            for (depth, k) in [(2.0, PIXEL_SCALE), (2.8, PIXEL_SCALE_NEAR)] {
                let (_, cam_cross) = axes(horizontal, cam.x, cam.y);
                let (along, across) = axes(horizontal, VIEW.0, VIEW.1);
                let item = pipe(horizontal, cam_cross + across as f32 * 0.5, depth);
                force_items(&mut layer, &world.level, &[item]);
                let (a, _) = render(&mut world, &mut layer, cam, 1.0, 0.0);
                for step in 1..=k {
                    let (dx, dy) = axes(horizontal, step as f32, 0.0);
                    let (b, _) = render(&mut world, &mut layer, cam + Vec2::new(dx, dy), 1.0, 0.0);
                    let mut varied = false;
                    for v in 0..across {
                        for u in 0..along - step as usize {
                            let (x, y) = axes(horizontal, u, v);
                            let (ax, ay) = axes(horizontal, u + step as usize, v);
                            assert_eq!(b.pixel_at(x, y), a.pixel_at(ax, ay), "along {u}, cross {v}, step {step}");
                            varied |= a.pixel_at(x, y) != a.pixel_at(ax, ay);
                        }
                    }
                    assert!(varied, "art had no along-axis detail");
                }
                // Once both phases have been drawn, identical frames don't grow the scratch.
                let warm = layer.scratch.capacity();
                let raster = layer.rasters[0].body.as_ptr();
                let (again, _) = render(&mut world, &mut layer, cam, 1.0, 0.0);
                assert_eq!(again.pixels(), a.pixels());
                assert_eq!(layer.scratch.capacity(), warm);
                assert_eq!(layer.rasters[0].body.as_ptr(), raster);
            }
        }
    }

    #[test]
    fn only_the_cross_axis_parallaxes_and_the_rat_fades_against_the_line() {
        let view = View { cam: Vec2::ZERO, size: Vec2::new(180.0, 180.0), zoom: 1.0 };
        let item = pipe(true, 90.0, 2.5);
        let rat = Vec2::new(1000.0, 90.0);
        let plan = plan_band(view, item, rat, &[]).unwrap();
        assert_eq!(plan.alpha, ALPHA_FLOOR_RAT as u8);
        let moved = plan_band(View { cam: Vec2::new(6.0, 4.0), ..view }, item, rat, &[]).unwrap();
        assert_eq!(plan.body.cross - moved.body.cross, 10);
        assert_eq!(plan.shadow.cross - moved.shadow.cross, 4);
        let far = plan_band(view, item, Vec2::new(1000.0, 0.0), &[]).unwrap();
        assert_eq!(far.alpha, ALPHA_MAX);
    }

    #[test]
    fn a_still_scene_draws_identically_twice() {
        for theme in Theme::playable() {
            let mut world = World::new(theme, 4);
            let mut layer = ForeLayer::new();
            layer.sync(&world.level);
            let at = layer.items.first().map(|p| p.anchor).unwrap_or(world.level.start);
            let cam = at - Vec2::new(60.0, 40.0);
            let (a, a_used) = render(&mut world, &mut layer, cam, 1.0, 6.25);
            let (b, b_used) = render(&mut world, &mut layer, cam, 1.0, 6.25);
            assert_eq!(a.pixels(), b.pixels(), "{theme:?}");
            assert_eq!(a_used, b_used);
            assert!(a_used <= (VIEW.0 * VIEW.1) as u32 * COVERAGE_SKIP_PCT / 100);
        }
    }

    #[test]
    fn with_the_layer_off_nothing_is_drawn() {
        let mut world = World::new(Theme::Sewer, 7);
        world.set_zoom_instant(1.0);
        let at = world.player.pos;
        force_items(&mut world.foreground_layer, &world.level, &[pipe(true, at.y, 2.8)]);
        let mut off = PixelBuffer::new(VIEW.0, VIEW.1);
        let mut on = PixelBuffer::new(VIEW.0, VIEW.1);
        let mut none = PixelBuffer::new(VIEW.0, VIEW.1);
        world.foreground = false;
        world.draw(&mut off);
        world.foreground = true;
        world.draw(&mut on);
        force_items(&mut world.foreground_layer, &world.level, &[]);
        world.draw(&mut none);
        assert_ne!(on.pixels(), none.pixels());
        assert_eq!(off.pixels(), none.pixels());
    }

    #[test]
    fn the_clear_disc_stays_the_same_size_at_zoom() {
        let mut pb = PixelBuffer::new(100, 100);
        pb.clear(FLOOR);
        clear_disc_set(Vec2::new(40.0, 40.0), 2.0);
        blend(&mut pb, 60, 40, (0, 0, 0), ALPHA_MAX);
        blend(&mut pb, 62, 40, (0, 0, 0), ALPHA_MAX);
        clear_disc_reset();
        assert_eq!(pb.pixel_at(60, 40), FLOOR);
        assert_ne!(pb.pixel_at(62, 40), FLOOR);
    }

    #[test]
    fn the_wires_are_planned_on_every_flatbed_frame() {
        let mut world = World::new(Theme::Trailer, 5);
        let mut layer = ForeLayer::new();
        for zoom in [1.0, 2.0] {
            for elapsed in [0.0, 0.7, 1.9, 3.3, 4.0, 7.1, 13.37] {
                let cam = Vec2::new(654.5, 1241.0);
                let _ = render(&mut world, &mut layer, cam, zoom, elapsed);
                assert!(layer.frame.iter().any(|p| p.item.kind == ForeKind::PowerLines));
                assert!(layer.frame.len() <= MAX_BANDS_IN_VIEW);
            }
        }
    }

    #[test]
    fn flatbed_movers_fade_at_their_track_ends() {
        let world = World::new(Theme::Trailer, 5);
        let view = View { cam: Vec2::ZERO, size: Vec2::new(240.0, 160.0), zoom: 1.0 };
        let rat = view.size * 0.5;
        let mut previous_cloud = None;
        let mut previous_branch: (f32, f32) = (0.0, 0.0);
        let mut branch_body_seen = false;
        let mut cloud_wrapped = false;
        let mut branch_led = false;
        let mut branch_trailed = false;
        for step in 0..(50 * SKY_QUANT_HZ as u32) {
            let elapsed = (step as f32 + 0.25) / SKY_QUANT_HZ;
            let [_, cloud, branch] = sky_items(&world.level, view, elapsed);
            let cloud = cloud.unwrap();
            let cloud_fade = plan_band(view, cloud, rat, &[]).map_or(0.0, |p| p.shadow.fade);
            if let Some((cross, fade)) = previous_cloud {
                if cloud.cross < cross {
                    assert_eq!((fade, cloud_fade), (0.0, 0.0));
                    cloud_wrapped = true;
                }
            }
            previous_cloud = Some((cloud.cross, cloud_fade));

            let (shadow, body) = branch.and_then(|item| plan_band(view, item, rat, &[]))
                .map_or((0.0, 0.0), |p| (p.shadow.fade, p.body.fade));
            if step > 0 {
                if previous_branch.0 == 0.0 && shadow > 0.0 {
                    assert!(shadow <= 0.25, "canopy shadow appeared at {shadow}");
                }
                if previous_branch.0 > 0.0 && shadow == 0.0 {
                    assert!(previous_branch.0 <= 0.25, "canopy shadow left at {}", previous_branch.0);
                }
                if previous_branch.1 == 0.0 && body > 0.0 {
                    assert!(body <= 0.5, "canopy body appeared at {body}");
                }
                if previous_branch.1 > 0.0 && body == 0.0 {
                    assert!(previous_branch.1 <= 0.5, "canopy body left at {}", previous_branch.1);
                }
            }
            if branch.is_none() {
                branch_body_seen = false;
            }
            if shadow > 0.0 && body == 0.0 && !branch_body_seen {
                branch_led = true;
            }
            if shadow > 0.0 && body == 0.0 && branch_body_seen {
                branch_trailed = true;
            }
            branch_body_seen |= body > 0.0;
            previous_branch = (shadow, body);
        }
        assert!(cloud_wrapped && branch_led && branch_trailed);
    }

    #[test]
    fn outfall_bands_respect_depth_room_rib_cap_and_coverage_budget() {
        let mut world = World::new(Theme::Outfall, 7);
        let mut layer = ForeLayer::new();
        layer.sync(&world.level);
        let placed: Vec<_> = layer.capture_items().collect();
        assert!(!placed.is_empty(), "Outfall should place foreground bands");
        for &(item, _, _) in &placed {
            assert!(
                (OUTFALL_DEPTH_MIN..=OUTFALL_DEPTH_MAX).contains(&item.depth),
                "Outfall band depth {} outside 1.8..=2.4",
                item.depth
            );
            assert!(kinds_for(Theme::Outfall).contains(&item.kind));
        }
        // At most one HullRib per non-corridor room.
        for room in &world.level.rooms {
            if room.kind == RoomKind::Corridor || room.kind == RoomKind::Maze {
                continue;
            }
            let ribs = placed
                .iter()
                .filter(|&&(item, anchor, corridor)| {
                    !corridor && item.kind == ForeKind::HullRib && room.rect.contains_world(anchor)
                })
                .count();
            assert!(ribs <= 1, "Outfall room had {ribs} HullRib bands (max 1)");
        }
        // Coverage stays strictly within the 14% Outfall overlay budget.
        let max_writes = (VIEW.0 * VIEW.1) as u32 * OUTFALL_COVERAGE_PCT / 100;
        for kind in kinds_for(Theme::Outfall) {
            let item = ForeItem {
                kind,
                axis: HORIZONTAL,
                cross: 40.0,
                depth: 2.1,
                scale: 1.2,
                seed: 19,
            };
            force_items(&mut layer, &world.level, &[item]);
            let (_, used) = render(&mut world, &mut layer, Vec2::ZERO, 1.0, 1.0);
            assert!(
                used <= max_writes,
                "{kind:?} wrote {used} px, exceeding Outfall 14% cap ({max_writes})"
            );
        }
    }
}
