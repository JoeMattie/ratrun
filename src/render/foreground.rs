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
const SCRATCH_MARGIN: i32 = 12;
const ART_ORIGIN: i32 = 2048;

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
const SKY_MARGIN: f32 = 64.0;

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
    pub horizontal: bool,
    /// World y for a horizontal band, world x for a vertical band.
    pub cross: f32,
    pub depth: f32,
    pub scale: f32,
    pub seed: u32,
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

/// Return the along and cross components in that order. Applying this twice
/// converts back to x and y for a vertical band.
fn axes<T>(horizontal: bool, x: T, y: T) -> (T, T) {
    if horizontal { (x, y) } else { (y, x) }
}

/// The foreground layer defaults to off. `RATRUN_FOREGROUND=1` enables it
/// for a session, and `0` (or `off`/`false`/`no`) pins it off.
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
    rgb: Vec<Rgb>,
    alpha: Vec<u8>,
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
        self.rgb.clear();
        self.rgb.resize(n, (0, 0, 0));
        self.alpha.clear();
        self.alpha.resize(n, 0);
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
            self.rgb[i] = rgb;
            self.alpha[i] = alpha;
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
        self.rgb.capacity() + self.alpha.capacity() + self.shade.capacity()
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
/// instead (see [`TARGET`]); the scratch is then blitted as `k x k` blocks
/// through [`blend_screen`], which applies all of the above per screen pixel.
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
struct Plan {
    item: ForeItem,
    screen_cross: i32,
    shadow_cross: i32,
    cross_size: f32,
    along_size: f32,
    half_thickness: f32,
    alpha: u8,
    k: i32,
}

impl Plan {
    fn body_area(&self) -> f32 {
        let cross = self.screen_cross as f32;
        let start = (cross - self.half_thickness).floor().max(0.0);
        let end = (cross + self.half_thickness).ceil().min(self.cross_size);
        (end - start).max(0.0) * self.along_size
    }
}

struct PlacedBand {
    item: ForeItem,
    #[cfg(test)]
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

        // Select nearest first, comparing in screen pixels across either axis.
        // A second pass is enough to find the next candidate with room beside
        // the first. Candidate order breaks ties (the pinned wires come first).
        for _ in 0..MAX_BANDS_IN_VIEW {
            let next = items.clone()
                .filter_map(|item| plan_band(view, item, rat, threats))
                .filter(|p| {
                    self.frame.iter().all(|other| {
                        p.item.horizontal != other.item.horizontal
                            || (p.screen_cross - other.screen_cross).abs() as f32
                                >= MIN_SCREEN_SPACING * view.zoom
                    })
                })
                .min_by(|a, b| {
                    let distance = |p: &Plan| (p.screen_cross as f32 - p.cross_size * 0.5).abs();
                    distance(a).total_cmp(&distance(b))
                });
            let Some(next) = next else { break };
            self.frame.push(next);
        }
        let fill = coverage_fill(level.theme);
        let covered: f32 = self.frame.iter()
            .filter(|p| !matches!(p.item.kind, ForeKind::CloudBand | ForeKind::PowerLines))
            .map(|p| p.body_area() * fill)
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

fn plan_band(view: View, item: ForeItem, rat: Vec2, threats: &[(Vec2, f32)]) -> Option<Plan> {
    let (along_size, cross_size) = axes(item.horizontal, view.size.x, view.size.y);
    let (_, cam_cross) = axes(item.horizontal, view.cam.x, view.cam.y);
    let centre = cross_size * 0.5;
    let world_cross = item.cross - cam_cross;
    let screen_cross = (centre + (world_cross - centre) * item.depth).round() as i32;
    let k = pixel_scale(&item);
    let half_thickness = art_foreground::thickness(&item) * 0.5 * k as f32;
    let cross = screen_cross as f32;
    if cross + half_thickness < 0.0 || cross - half_thickness >= cross_size {
        return None;
    }

    let line_distance = |body: Vec2| {
        let (_, body_cross) = axes(item.horizontal, body.x, body.y);
        (body_cross - cam_cross - cross).abs()
    };
    let mut alpha = proximity_alpha(
        ALPHA_MAX as f32, line_distance(rat) / view.zoom, FADE_RAT_PX, ALPHA_FLOOR_RAT,
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
    let shadow_cross = world_cross + if item.horizontal { 14.0 } else { 10.0 };
    Some(Plan {
        item, screen_cross, shadow_cross: shadow_cross.round() as i32,
        cross_size, along_size, half_thickness, alpha: alpha.round() as u8, k,
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
    for plan in &layer.frame {
        let painted = writes_so_far();
        let est_writes = if scene.level.theme == Theme::Outfall {
            plan.body_area() * OUTFALL_COVERAGE_FILL
        } else {
            plan.body_area()
        };
        let alpha = if est_writes > cap_skip.saturating_sub(painted) as f32 {
            // Keep the whole run visible as haze when it might exceed the
            // hard budget. Stopping midway would leave a visible band end.
            plan.alpha.min(CLOUD_ALPHA_MAX)
        } else {
            coverage_alpha(plan.alpha, painted, cap_full)
        };
        draw_band(pb, &mut layer.scratch, view, plan, alpha, scene);
    }
    clear_disc_reset();
    budget_finish()
}

/// The art grid has a world-fixed phase along the band. Split the camera
/// coordinate into a whole art-pixel offset and the residual translation of
/// the whole raster. Even a one-pixel camera step moves the chunky blocks
/// with the floor, including when the origin passes a block boundary.
fn draw_band(
    pb: &mut PixelBuffer,
    scratch: &mut Scratch,
    view: View,
    plan: &Plan,
    alpha: u8,
    scene: &ForeScene,
) {
    if alpha == 0 {
        return;
    }
    let item = &plan.item;
    let block = plan.k;
    let (cam_along, _) = axes(item.horizontal, view.cam.x, view.cam.y);
    let cam_along = cam_along.round() as i32;
    let world_art_offset = cam_along.div_euclid(block);
    let origin = -cam_along.rem_euclid(block);
    let overhang = (BAND_OVERHANG as f32 * view.zoom).ceil() as i32;
    let from = (-overhang - origin).div_euclid(block);
    let to = (plan.along_size as i32 - 1 + overhang - origin).div_euclid(block);
    let half = (plan.half_thickness / block as f32).ceil() as i32 + SCRATCH_MARGIN;
    let (x0, y0) = axes(item.horizontal, from, ART_ORIGIN - half);
    let (w, h) = axes(item.horizontal, to - from + 1, 2 * half + 1);
    scratch.reset(x0, y0, w, h);
    let d = ForeDraw {
        screen_cross: ART_ORIGIN,
        shadow_cross: ART_ORIGIN,
        from,
        to,
        scale: item.scale,
        alpha,
        sway: sway_of(item, scene.elapsed),
        theme: scene.level.theme,
        along_offset: world_art_offset,
        elapsed_step: (scene.elapsed.max(0.0) * SWAY_HZ).floor() as u32,
        toward_centre: if plan.screen_cross as f32 <= plan.cross_size * 0.5 { 1 } else { -1 },
    };
    TARGET.with(|t| *t.borrow_mut() = Some(std::mem::take(scratch)));
    art_foreground::draw(pb, item, &d);
    *scratch = TARGET.with(|t| t.borrow_mut().take()).unwrap_or_default();

    for (cross, shade) in [(plan.shadow_cross, true), (plan.screen_cross, false)] {
        let (x, y) = axes(item.horizontal, origin, cross - ART_ORIGIN * block);
        blit(pb, scratch, (x, y), block, shade);
    }
}

/// Project an art plane into the world buffer. Its integer lattice stays
/// fixed in the world when the zoom changes.
fn blit(pb: &mut PixelBuffer, sc: &Scratch, origin: (i32, i32), block: i32, shade: bool) {
    let (bw, bh) = (pb.w as i32, pb.h as i32);
    for j in 0..sc.h {
        let y = origin.1 + (sc.y0 + j) * block;
        let (ya, yb) = (y.max(0), (y + block).min(bh));
        if ya >= yb {
            continue;
        }
        for i in 0..sc.w {
            let x = origin.0 + (sc.x0 + i) * block;
            let (xa, xb) = (x.max(0), (x + block).min(bw));
            if xa >= xb {
                continue;
            }
            let idx = (j * sc.w + i) as usize;
            if shade {
                let strength = sc.shade[idx];
                if strength != 0 {
                    for y in ya..yb {
                        for x in xa..xb {
                            shadow_screen(pb, x, y, strength as f32 / 255.0);
                        }
                    }
                }
            } else {
                let alpha = sc.alpha[idx];
                if alpha != 0 {
                    for y in ya..yb {
                        for x in xa..xb {
                            blend_screen(pb, x, y, sc.rgb[idx], alpha);
                        }
                    }
                }
            }
        }
    }
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


// Placement rolls each band's cross position independently of the simulation.
// Large rooms go first so they can claim space before corridor lines. Each
// eligible run gets one slot, with retries for blocked or crowded positions.
fn generate(level: &Level, out: &mut Vec<PlacedBand>) {
    if level.theme == Theme::Trailer {
        return;
    }
    let mut rng = Gen::new(crate::game::level::level_seed(level.theme, level.seed, GEN_SALT));
    let kinds = kinds_for(level.theme);
    for corridor in [false, true] {
        for room in &level.rooms {
            if (room.kind == RoomKind::Corridor) != corridor || room.kind == RoomKind::Maze {
                continue;
            }
            let rect = room.rect;
            let short = rect.w.min(rect.h);
            let long = rect.w.max(rect.h);
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
            let mut room_has_rib = false;
            for _ in 0..slots {
                for attempt in 0..PLACE_ATTEMPTS {
                    // Cross the short axis. If the room is crowded, try a
                    // perpendicular band; a corridor always stays transverse.
                    let mut horizontal = rect.w < rect.h;
                    if !corridor && attempt >= PLACE_ATTEMPTS / 2 {
                        horizontal = !horizontal;
                    }
                    // Corridors may be only one tile wide, so sample on their
                    // centre line. Rooms sample over their open interior.
                    let (x_margin, y_margin) = if corridor {
                        if horizontal { (rect.w as f32 * 0.5, 0.5) }
                        else { (0.5, rect.h as f32 * 0.5) }
                    } else {
                        (1.0, 1.0)
                    };
                    let at = centre + Vec2::new(
                        rng.range(x_margin - rect.w as f32 * 0.5, rect.w as f32 * 0.5 - x_margin),
                        rng.range(y_margin - rect.h as f32 * 0.5, rect.h as f32 * 0.5 - y_margin),
                    ) * TILE_F;
                    let (_, cross) = axes(horizontal, at.x, at.y);
                    let (_, start_cross) = axes(horizontal, level.start.x, level.start.y);
                    if (cross - start_cross).abs() < START_CLEAR_PX
                        || !level.tiles.get_at(at).is_open()
                        || out.iter().any(|p| {
                            p.item.horizontal == horizontal && (p.item.cross - cross).abs() < MIN_SPACING
                        })
                    {
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
                    if level.theme == Theme::Outfall && !corridor && kind == ForeKind::HullRib {
                        if room_has_rib {
                            kind = if (seed & 1) == 0 {
                                ForeKind::Kelp
                            } else {
                                ForeKind::BubbleChain
                            };
                        } else {
                            room_has_rib = true;
                        }
                    }
                    let item = ForeItem {
                        kind, horizontal, cross,
                        depth,
                        scale,
                        seed,
                    };
                    out.push(PlacedBand {
                        item: fit_scale(item),
                        #[cfg(test)]
                        anchor: at,
                        #[cfg(test)]
                        corridor,
                    });
                    break;
                }
            }
        }
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

/// Analytic tracks in screen space, expressed as world cross positions for
/// the same planner as static bands. Only the track needs the view to set its
/// crossing distance. Along-axis detail still follows the world camera.
fn sky_items(level: &Level, view: View, elapsed: f32) -> [Option<ForeItem>; 3] {
    if level.theme != Theme::Trailer {
        return [None; 3];
    }
    let seed = crate::game::level::level_seed(level.theme, level.seed, GEN_SALT);
    let t = (elapsed.max(0.0) * SKY_QUANT_HZ).floor() / SKY_QUANT_HZ;
    let at = |kind, horizontal, cross, depth, scale, seed| {
        let (_, cam_cross) = axes(horizontal, view.cam.x, view.cam.y);
        let (_, span) = axes(horizontal, view.size.x, view.size.y);
        let centre = span * 0.5;
        fit_scale(ForeItem {
            kind, horizontal, depth, scale, seed,
            cross: cam_cross + centre + (cross * view.zoom - centre) / depth,
        })
    };

    // The centre line keeps the wires present even when both movers qualify.
    let wires = at(ForeKind::PowerLines, true, view.size.y * 0.5 / view.zoom,
        2.0, 1.15, hash_u32(seed, 0x12));

    let width = view.size.x / view.zoom;
    let travel = width + 2.0 * SKY_MARGIN;
    let cloud_phase = (hash01(seed, 0xC10D) * travel + t * SKY_CLOUD_SPEED).rem_euclid(travel);
    let cloud = at(ForeKind::CloudBand, false, cloud_phase - SKY_MARGIN,
        1.8, 1.25, hash_u32(seed, 0xC10D));

    // Successive starts differ by 5–9 s. The branch travels from the cab
    // side of the frame to the tail (right to left).
    let jitter = |event| (hash01(seed ^ 0x5EED_0000, event) - 0.5) * 2.0;
    let nearest = (t / SKY_BRANCH_PERIOD).floor() as u64;
    let branch = (nearest.saturating_sub(1)..=nearest + 1).rev().find_map(|event| {
        let age = t - (SKY_BRANCH_PERIOD * event as f32 + jitter(event));
        (age >= 0.0 && age * SKY_BRANCH_SPEED < travel).then(|| {
            at(ForeKind::BranchCanopy, false, width + SKY_MARGIN - age * SKY_BRANCH_SPEED,
                2.8, 1.4, hash_u32(seed ^ 0x1EAF_0000, event))
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
            p.item.kind == kind && (p.screen_cross as f32 - p.cross_size * 0.5).abs() <= 12.0 * zoom
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
        ForeItem { kind: ForeKind::MainPipe, horizontal, cross, depth, scale: 1.0, seed: 37 }
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
            for seed in [1, 2, 7] {
                let world = World::new(theme, seed);
                let mut a = ForeLayer::new();
                let mut b = ForeLayer::new();
                a.sync(&world.level);
                b.sync(&world.level);
                let a: Vec<_> = a.capture_items().collect();
                assert_eq!(a, b.capture_items().collect::<Vec<_>>());
                assert!(!a.is_empty(), "{theme:?} seed {seed}");
                for (i, &(item, anchor, corridor)) in a.iter().enumerate() {
                    let (_, start_cross) = axes(item.horizontal, world.level.start.x, world.level.start.y);
                    let (_, anchor_cross) = axes(item.horizontal, anchor.x, anchor.y);
                    assert_eq!(item.cross, anchor_cross);
                    assert!((item.cross - start_cross).abs() >= START_CLEAR_PX);
                    assert!(world.level.tiles.get_at(anchor).is_open());
                    assert!(kinds_for(theme).contains(&item.kind));
                    assert!((DEPTH_MIN..=DEPTH_MAX).contains(&item.depth));
                    assert!((SCALE_MIN..=SCALE_MAX).contains(&item.scale));
                    assert!((9.999..=30.001).contains(&art_foreground::thickness(&item)));
                    assert!(a[..i].iter().all(|(other, _, _)| {
                        other.horizontal != item.horizontal || (other.cross - item.cross).abs() >= MIN_SPACING
                    }));
                    if corridor {
                        let (cx, cy) = ((anchor.x / TILE_F) as i32, (anchor.y / TILE_F) as i32);
                        assert!(world.level.rooms.iter().any(|r| {
                            r.kind == RoomKind::Corridor
                                && (r.rect.w < r.rect.h) == item.horizontal
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
            for horizontal in [true, false] {
                let (_, cam_cross) = axes(horizontal, cam.x, cam.y);
                let (_, span) = axes(horizontal, VIEW.0, VIEW.1);
                let item = pipe(horizontal, cam_cross + span as f32 * zoom * 0.5, 2.0);
                force_items(&mut layer, &world.level, &[item]);
                let (mut raster, used) = render(&mut world, &mut layer, cam, zoom, 0.0);
                assert!(used > 0, "no body drawn");
                let scratch = &layer.scratch;
                let plan = &layer.frame[0];
                let (cam_along, _) = axes(horizontal, cam.x, cam.y);
                let block = plan.k;
                let phase = -(cam_along.round() as i32).rem_euclid(block);
                let cross_origin = plan.screen_cross - ART_ORIGIN * block;
                // Verify the body itself. Its separate shadow may reach both edges too.
                raster.clear(FLOOR);
                blit(&mut raster, scratch, axes(horizontal, phase, cross_origin), block, false);
                let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
                raster.resample_to(&mut pb);
                let (length, width) = axes(horizontal, pb.w, pb.h);
                for edge in [0, length - 1] {
                    assert!((0..width).any(|cross| {
                        let (x, y) = axes(horizontal, edge, cross);
                        pb.pixel_at(x, y) != FLOOR
                    }), "missing edge {edge} at {zoom}x, horizontal = {horizontal}");
                }
                let (from, _) = axes(horizontal, scratch.x0, scratch.y0);
                let (len, _) = axes(horizontal, scratch.w, scratch.h);
                assert!((phase + from * block) as f32 <= -(BAND_OVERHANG as f32) * zoom);
                assert!((phase + (from + len) * block) as f32 >= plan.along_size + BAND_OVERHANG as f32 * zoom);
            }
        }
    }

    #[test]
    fn only_the_two_nearest_well_spaced_bands_are_selected() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
        for zoom in [1.0, 2.0] {
            let side = (120.0 * zoom) as usize;
            let mut pb = PixelBuffer::new(side, side);
            let centre = pb.h as f32 * 0.5;
            let near = pipe(true, centre, 2.0);
            let too_close = pipe(true, centre + zoom * 47.0 / 2.0, 2.0);
            let spaced = pipe(true, centre - zoom * 48.0 / 2.0, 2.0);
            let farther = pipe(true, centre + zoom * 56.0 / 2.0, 2.0);
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
            assert_eq!((layer.frame[0].screen_cross - layer.frame[1].screen_cross).abs() as f32, 48.0 * zoom);
        }
    }

    #[test]
    fn detail_and_chunky_pixels_are_world_anchored_along_the_band() {
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
                render(&mut world, &mut layer, cam, 1.0, 0.0);
                assert_eq!(layer.scratch.capacity(), warm);
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
        assert_eq!(plan.screen_cross - moved.screen_cross, 10);
        assert_eq!(plan.shadow_cross - moved.shadow_cross, 4);
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
                horizontal: true,
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
