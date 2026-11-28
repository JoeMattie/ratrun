//! Overhead foreground bands and off-screen-anchored reaches, drawn after
//! lighting and under hostile tells. Placement depends only on the level.
//! Motion, fades and scratch compositing are render-only and leave the
//! simulation and level generation untouched.

use bevy_ecs::world::World as EcsWorld;

use crate::game::art_foreground;
use crate::game::art_foreground_reach::{
    self, AnchorSide, ReachShape, EMISSIVE_PEN,
};
use crate::game::collision::TILE_F;
use crate::game::ecs::systems::boss::Boss;
use crate::game::ecs::systems::miniboss::Miniboss;
use crate::game::enemy::Enemy;
use crate::game::level::{Level, RoomKind, Theme};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

pub use art_foreground_reach::ReachKind;

/// Unit vector pointing from an off-screen [`AnchorSide`] into the view.
const fn inward_axis(side: AnchorSide) -> Vec2 {
    match side {
        AnchorSide::Top => VERTICAL,
        AnchorSide::Bottom => Vec2 { x: 0.0, y: -1.0 },
        AnchorSide::Left => HORIZONTAL,
        AnchorSide::Right => Vec2 { x: -1.0, y: 0.0 },
    }
}

const fn is_vertical_edge(side: AnchorSide) -> bool {
    matches!(side, AnchorSide::Top | AnchorSide::Bottom)
}

/// Allowed anchor edges per [`ReachKind`], read directly from `art_foreground_reach`.
fn reach_allowed_sides(kind: ReachKind) -> &'static [AnchorSide] {
    art_foreground_reach::reach_spec(kind).sides
}

/// Whether `kind` is a 2D edge-anchored whole-tree canopy (`ReachShape::Canopy`).
fn is_canopy_reach(kind: ReachKind) -> bool {
    art_foreground_reach::reach_spec(kind).shape == ReachShape::Canopy
}

pub const ALPHA_MAX: u8 = 215;
const ALPHA_FLOOR: f32 = 28.0;
const ALPHA_FLOOR_RAT: f32 = 120.0;
const FADE_RAT_PX: f32 = 48.0;
const FADE_THREAT_PX: f32 = 48.0;
const FADE_RAMP_PX: f32 = 40.0;
const EDGE_FADE_PX: f32 = 16.0;
const SWAY_HZ: f32 = 3.0;
const REACH_SWAY_HZ: f32 = 8.0;
const COVERAGE_PCT: u32 = 45;
const COVERAGE_SKIP_PCT: u32 = 85;
/// Coverage cap on the Flatbed, which places no static items: its whole
/// foreground is the three analytic sky planes, each easing in and out at its
/// own screen edge, so they are exempt from the static budget (`draw`).
const FLATBED_SKY_COVERAGE_PCT: u32 = 200;
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
/// Slots in the frame plan. The two nearest static items keep
/// [`MAX_BANDS_IN_VIEW`]; the Flatbed's analytic sky track is planned beside
/// them (each sky item eases in and out at a screen edge of its own) instead
/// of contesting their slots.
const MAX_SKY_ITEMS: usize = 3;
const MAX_FRAME_ITEMS: usize = if MAX_SKY_ITEMS > MAX_BANDS_IN_VIEW {
    MAX_SKY_ITEMS
} else {
    MAX_BANDS_IN_VIEW
};
const MIN_SCREEN_SPACING: f32 = 48.0;
const PIXEL_SCALE: i32 = 2;
const PIXEL_SCALE_NEAR: i32 = 3;
const PIXEL_SCALE_NEAR_DEPTH: f32 = 2.6;
// Cache whole sections of the along-axis art lattice, with room for the blur.
const RASTER_PAGE: i32 = 32;
// Empty art columns past a reach's free tip so the tent blur feathers cleanly.
const REACH_TIP_PAD: i32 = 6;
// Floor shadow lead ahead of a reach's tip along its ray (world px), matching
// the cross-axis shadow offset so shadows enter first from every edge.
const REACH_SHADOW_LEAD: f32 = 14.0;

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
/// World-pixel margin past each end of a sky item's track: the whole crossing
/// plus margin is covered, so the body and the shadow's offset rect are both
/// completely outside the view when the track starts and when it ends. It also
/// covers the art pad the shadow's soft edge paints past `cross_samples`.
const SKY_TRACK_MARGIN: f32 = 12.0;
/// Screen-pixel edge ramp for a sky reach. `SKY_BRANCH_SPEED` screen pixels a
/// second would turn the default [`EDGE_FADE_PX`] ramp into a 0.18 s flick on
/// the body; this is 0.33 s on the body and 0.65–0.79 s on the shadow (which
/// travels `depth` times slower across).
const SKY_EDGE_FADE_PX: f32 = 72.0;

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

pub const fn all_reach_kinds() -> [ReachKind; 8] {
    ReachKind::all()
}

fn reach_kinds_for(theme: Theme) -> Vec<ReachKind> {
    all_reach_kinds()
        .into_iter()
        .filter(|&k| art_foreground_reach::reach_spec(k).floors.contains(&theme))
        .collect()
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

/// Placement shape: either a full-span band (`art_foreground`) or an
/// off-screen-anchored reach (`art_foreground_reach`, either a repeating-stem
/// strip ray or a 2D edge-anchored `TreeCanopy` half-disc).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ForeShape {
    Band(ForeKind),
    Reach {
        kind: ReachKind,
        /// World coordinate of the free tip/rim projected onto `axis` (`tip · axis`).
        tip_along: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForeEntry {
    pub shape: ForeShape,
    /// Unit tangent: for `Reach`, points from the off-screen anchor into the
    /// view toward the free tip/rim (`tip_along`).
    pub axis: Vec2,
    /// World line offset along `normal()`.
    pub cross: f32,
    pub depth: f32,
    pub scale: f32,
    pub seed: u32,
}

const HORIZONTAL: Vec2 = Vec2 { x: 1.0, y: 0.0 };
const VERTICAL: Vec2 = Vec2 { x: 0.0, y: 1.0 };

fn canonical_normal(axis: Vec2) -> Vec2 {
    // Canonical upright normal (`normal(-v) == normal(v)`, `normal.y >= 0`,
    // and `(1, 0)` for vertical axes) matching `art_foreground_reach`'s exact
    // `AnchorSide` screen mapping on all four viewport edges.
    if axis.x < -1e-6 || (axis.x.abs() <= 1e-6 && axis.y >= 0.0) {
        -axis.perp()
    } else {
        axis.perp()
    }
}

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

impl ForeEntry {
    pub fn from_band(item: ForeItem) -> Self {
        Self {
            shape: ForeShape::Band(item.kind),
            axis: item.axis,
            cross: item.cross,
            depth: item.depth,
            scale: item.scale,
            seed: item.seed,
        }
    }

    pub fn reach(kind: ReachKind, axis: Vec2, tip: Vec2, depth: f32, seed: u32) -> Self {
        let axis = axis.normalized();
        let normal = canonical_normal(axis);
        Self {
            shape: ForeShape::Reach { kind, tip_along: tip.dot(axis) },
            axis,
            cross: tip.dot(normal),
            depth,
            scale: 1.0,
            seed,
        }
    }

    /// Stage a [`ReachKind`] along one of its allowed sides for capture helpers.
    #[cfg(test)]
    pub(crate) fn stage_reach(kind: ReachKind, tip: Vec2, seed: u32) -> Self {
        let axis = reach_placement_axis(kind, None, seed);
        let depth = if is_canopy_reach(kind) { 2.2 } else { 2.35 };
        Self::reach(kind, axis, tip, depth, seed)
    }

    pub(crate) fn normal(self) -> Vec2 {
        match self.shape {
            ForeShape::Band(_) => {
                if self.axis == VERTICAL {
                    HORIZONTAL
                } else {
                    self.axis.perp()
                }
            }
            ForeShape::Reach { .. } => canonical_normal(self.axis),
        }
    }

    #[cfg(test)]
    pub(crate) fn tip_world(self) -> Option<Vec2> {
        match self.shape {
            ForeShape::Band(_) => None,
            ForeShape::Reach { tip_along, .. } => {
                Some(self.axis * tip_along + self.normal() * self.cross)
            }
        }
    }

    fn as_band(self) -> Option<ForeItem> {
        match self.shape {
            ForeShape::Band(kind) => Some(ForeItem {
                kind,
                axis: self.axis,
                cross: self.cross,
                depth: self.depth,
                scale: self.scale,
                seed: self.seed,
            }),
            ForeShape::Reach { .. } => None,
        }
    }

    pub(crate) fn organic(self) -> bool {
        match self.shape {
            ForeShape::Band(kind) => kind.organic(),
            ForeShape::Reach { kind, .. } => art_foreground_reach::reach_spec(kind).organic,
        }
    }

    pub(crate) fn is_canopy(self) -> bool {
        match self.shape {
            ForeShape::Band(ForeKind::BranchCanopy) => true,
            ForeShape::Reach { kind, .. } => is_canopy_reach(kind),
            ForeShape::Band(_) => false,
        }
    }

    fn is_cloud(self) -> bool {
        matches!(self.shape, ForeShape::Band(ForeKind::CloudBand))
    }

    fn is_wires(self) -> bool {
        matches!(self.shape, ForeShape::Band(ForeKind::PowerLines))
    }

    /// A roadside reach of the Flatbed's analytic sky track (`sky_items`).
    /// `TreeCanopy` is also placed statically on the Kitchen, where the wider
    /// ease is equally wanted.
    fn is_sky_reach(self) -> bool {
        matches!(
            self.shape,
            ForeShape::Reach {
                kind: ReachKind::LightPole | ReachKind::SignArm | ReachKind::TreeCanopy,
                ..
            }
        )
    }

    fn art_thickness(self) -> f32 {
        match self.shape {
            ForeShape::Band(_) => art_foreground::thickness(&self.as_band().unwrap()),
            ForeShape::Reach { kind, .. } => {
                art_foreground_reach::reach_spec(kind).thickness * self.scale.max(0.25)
            }
        }
    }

    fn cross_samples(self) -> std::ops::Range<i32> {
        match self.shape {
            ForeShape::Band(_) => art_foreground::cross_samples(&self.as_band().unwrap()),
            ForeShape::Reach { kind, .. } => {
                let spec = art_foreground_reach::reach_spec(kind);
                let th = spec.thickness * self.scale.max(0.25);
                let pad = if spec.shape == ReachShape::Canopy {
                    1.0
                } else if spec.organic {
                    3.0
                } else {
                    1.0
                };
                let ht = th * 0.5 + pad;
                let centre = if (th.round() as i32) & 1 == 0 { -0.5 } else { 0.0 };
                ((centre - ht).floor() as i32)..(centre + ht).ceil() as i32 + 1
            }
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

/// Choose a reach axis strictly from `reach_allowed_sides(kind)`.
/// `ReachShape::Canopy` stays flush on its straight viewport edge so the
/// half-disc's flat diameter never tilts inside the view; organic strips
/// (`VineTendril`, `KelpFrond`) lean diagonally off their chosen edge.
fn reach_placement_axis(kind: ReachKind, prefer_horizontal: Option<bool>, seed: u32) -> Vec2 {
    let spec = art_foreground_reach::reach_spec(kind);
    let sides = reach_allowed_sides(kind);
    let pick_from = |want_horiz: bool| -> Option<AnchorSide> {
        let mut buf = [AnchorSide::Top; 4];
        let mut n = 0;
        for &s in sides {
            if (!is_vertical_edge(s)) == want_horiz {
                buf[n] = s;
                n += 1;
            }
        }
        (n > 0).then(|| buf[(hash_u32(u64::from(seed), 0xD19C) as usize) % n])
    };
    let side = prefer_horizontal
        .and_then(pick_from)
        .unwrap_or_else(|| {
            if sides.is_empty() {
                AnchorSide::Top
            } else {
                sides[(hash_u32(u64::from(seed), 0xD19C) as usize) % sides.len()]
            }
        });
    let base = inward_axis(side);
    if spec.organic && spec.shape == ReachShape::Strip {
        let mag = 0.14 + hash01(u64::from(seed), 0xB7E1) * 0.34;
        let sign = if hash_u32(u64::from(seed), 0x7A31) & 1 == 0 { 1.0 } else { -1.0 };
        base.rotate(sign * mag).normalized()
    } else {
        base
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

fn band_pixel_scale(entry: &ForeEntry) -> i32 {
    if !entry.is_cloud() && entry.depth >= PIXEL_SCALE_NEAR_DEPTH {
        PIXEL_SCALE_NEAR
    } else {
        PIXEL_SCALE
    }
}

/// Art-to-world block scale for `entry` in `view`. Strip reaches and bands use
/// 2x/3x blocks by depth; `ReachShape::Canopy` (`TreeCanopy`, `R = 36` art px)
/// scales `k` so its radial reach covers ~0.8–1.0 of `view.size`'s short axis
/// and its lateral span covers most of the view at every resolution and zoom.
fn entry_pixel_scale(entry: ForeEntry, view: View) -> i32 {
    match entry.shape {
        ForeShape::Band(_) => band_pixel_scale(&entry),
        ForeShape::Reach { kind, .. } => {
            let spec = art_foreground_reach::reach_spec(kind);
            if spec.shape == ReachShape::Canopy {
                let short = view.size.x.min(view.size.y).max(spec.tip_len);
                let max_k = (short / spec.tip_len).floor().max(1.0) as i32;
                ((short * 0.92 / spec.tip_len).round() as i32).clamp(1, max_k)
            } else if entry.depth >= PIXEL_SCALE_NEAR_DEPTH {
                PIXEL_SCALE_NEAR
            } else {
                PIXEL_SCALE
            }
        }
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
    static WRITE_BUDGET: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
    static WRITES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static CLEAR_DISC: std::cell::Cell<(i32, i32, f32, bool)> =
        const { std::cell::Cell::new((0, 0, 1.0, false)) };
    static TARGET: std::cell::RefCell<Option<Scratch>> = const { std::cell::RefCell::new(None) };
}

/// One item's offscreen raster: a window `[x0, x0 + w) x [y0, y0 + h)` of art
/// coordinates holding premultiplied `[r, g, b, a, emissive]` and floor shadow
/// strength. Supports arbitrary 2D extents (including full-view `TreeCanopy`
/// half-discs) and only ever grows once warm.
#[derive(Default)]
struct Scratch {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    body: Vec<[u32; 5]>,
    filtered_rows: Vec<[u32; 5]>,
    shade: Vec<u8>,
}

impl Scratch {
    fn reset(&mut self, x0: i32, y0: i32, w: i32, h: i32) {
        let n = (w.max(0) * h.max(0)) as usize;
        self.x0 = x0;
        self.y0 = y0;
        self.w = w.max(0);
        self.h = h.max(0);
        self.body.clear();
        self.body.resize(n, [0; 5]);
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

    #[inline]
    fn put(&mut self, x: i32, y: i32, rgb: Rgb, alpha: u8) {
        self.put_with_emissive(x, y, rgb, alpha, false);
    }

    #[inline]
    fn put_with_emissive(&mut self, x: i32, y: i32, rgb: Rgb, alpha: u8, emissive: bool) {
        if let Some(i) = self.index(x, y) {
            let a = u32::from(alpha);
            let e = if emissive { a } else { 0 };
            self.body[i] = [
                u32::from(rgb.0) * a,
                u32::from(rgb.1) * a,
                u32::from(rgb.2) * a,
                a,
                e,
            ];
        }
    }

    #[inline]
    fn put_shade(&mut self, x: i32, y: i32, strength: f32) {
        if let Some(i) = self.index(x, y) {
            let v = (strength.clamp(0.0, 1.0) * 255.0).round() as u8;
            if v > self.shade[i] {
                self.shade[i] = v;
            }
        }
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.body.capacity() + self.filtered_rows.capacity() + self.shade.capacity()
    }
}

/// Reusable local buffers handed to `art_foreground_reach::draw_reach`, bundled
/// with `Scratch` so `Focus::draw` takes one scratch handle.
struct RenderBuffers {
    scratch: Scratch,
    reach_body: PixelBuffer,
    reach_shadow: PixelBuffer,
}

impl Default for RenderBuffers {
    fn default() -> Self {
        let mut reach_body = PixelBuffer::new(0, 0);
        let mut reach_shadow = PixelBuffer::new(0, 0);
        reach_body.set_materials(true);
        reach_shadow.set_materials(true);
        Self {
            scratch: Scratch::default(),
            reach_body,
            reach_shadow,
        }
    }
}

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
/// buffer. The **only** body painter the band art may use.
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

/// Minimum `s · axis` among all points `s` inside `[0, size.x] × [0, size.y]`
/// within the cross-axis strip `span`, or `None` when the strip misses the view.
fn min_strip_along(span: (f32, f32), axis: Vec2, normal: Vec2, size: Vec2) -> Option<f32> {
    let view_span = project(size, normal);
    let c_lo = span.0.max(view_span.0);
    let c_hi = span.1.min(view_span.1);
    if c_hi <= c_lo {
        return None;
    }
    let mut lo = f32::INFINITY;
    for c in [c_lo, c_hi] {
        let (a, b) = line_span(normal * c, axis, size);
        if a <= b {
            lo = lo.min(a);
        }
    }
    for corner in [
        Vec2::ZERO,
        Vec2::new(size.x, 0.0),
        Vec2::new(0.0, size.y),
        size,
    ] {
        let cn = corner.dot(normal);
        if cn >= c_lo - 1e-4 && cn <= c_hi + 1e-4 {
            lo = lo.min(corner.dot(axis));
        }
    }
    (lo < f32::INFINITY).then_some(lo)
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    cross: i32,
    span: (f32, f32),
    tip_max: f32,
    fade: f32,
}

impl Plane {
    fn new_band(cross: f32, extent: (f32, f32), view_span: (f32, f32), fade_px: f32) -> Self {
        let cross = cross.round() as i32;
        let span = (cross as f32 + extent.0, cross as f32 + extent.1);
        let penetration = (span.1 - view_span.0).min(view_span.1 - span.0);
        let fade = (penetration / fade_px.max(1.0)).clamp(0.0, 1.0);
        Self { cross, span, tip_max: f32::INFINITY, fade }
    }

    fn new_reach(
        cross: f32,
        extent: (f32, f32),
        tip_max: f32,
        basis: (Vec2, Vec2),
        view: View,
        view_span: (f32, f32),
        fade_px: f32,
    ) -> Self {
        let (axis, normal) = basis;
        let cross = cross.round() as i32;
        let span = (cross as f32 + extent.0, cross as f32 + extent.1);
        let cross_pen = (span.1 - view_span.0).min(view_span.1 - span.0);
        let fade = if cross_pen <= 0.0 {
            0.0
        } else if let Some(min_along) = min_strip_along(span, axis, normal, view.size) {
            let along_pen = tip_max - min_along;
            (cross_pen.min(along_pen) / fade_px.max(1.0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self { cross, span, tip_max, fade }
    }
}

#[derive(Clone, Copy, Debug)]
struct Plan {
    entry: ForeEntry,
    normal: Vec2,
    body: Plane,
    shadow: Plane,
    centre_cross: f32,
    centre_along: f32,
    along: (f32, f32),
    /// Screen along-coordinate of body's unpadded tip (`+∞` for bands).
    body_tip: f32,
    /// Screen along-coordinate of shadow's unpadded tip (`+∞` for bands).
    shadow_tip: f32,
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
            let (lo, hi) = line_span(origin, self.entry.axis, size);
            (hi.min(self.body.tip_max) - lo).max(0.0)
        };
        let box_area = (end - start)
            * (width_at(start) + 4.0 * width_at((start + end) * 0.5) + width_at(end))
            / 6.0;
        if self.entry.is_canopy() {
            // Semicircular half-disc occupies pi/4 of its bounding rectangle.
            box_area * std::f32::consts::FRAC_PI_4
        } else {
            box_area
        }
    }

    fn visible_lines(&self) -> impl Iterator<Item = i32> {
        [self.body, self.shadow].into_iter()
            .filter_map(|plane| (plane.fade > 0.0).then_some(plane.cross))
    }

    /// On-screen area of the shadow's offset rect, in screen pixels. Zero when
    /// the shadow has faded out. Mirrors [`Plan::body_area`].
    #[cfg(test)]
    fn shadow_area(&self, size: Vec2) -> f32 {
        if self.shadow.fade == 0.0 {
            return 0.0;
        }
        let (lo, hi) = project(size, self.normal);
        let start = self.shadow.span.0.max(lo);
        let end = self.shadow.span.1.min(hi);
        if end <= start {
            return 0.0;
        }
        let width_at = |v| {
            let origin = self.normal * v;
            let (lo, hi) = line_span(origin, self.entry.axis, size);
            (hi.min(self.shadow.tip_max) - lo).max(0.0)
        };
        let box_area = (end - start)
            * (width_at(start) + 4.0 * width_at((start + end) * 0.5) + width_at(end))
            / 6.0;
        if self.entry.is_canopy() {
            box_area * std::f32::consts::FRAC_PI_4
        } else {
            box_area
        }
    }

    fn centre_distance(&self) -> f32 {
        [self.body, self.shadow]
            .into_iter()
            .filter(|plane| plane.fade > 0.0)
            .map(|plane| {
                let dc = plane.cross as f32 - self.centre_cross;
                let da = if plane.tip_max.is_finite() {
                    (self.centre_along - plane.tip_max).max(0.0)
                } else {
                    0.0
                };
                (dc * dc + da * da).sqrt()
            })
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

struct PlacedItem {
    entry: ForeEntry,
    anchor: Vec2,
    #[cfg(test)]
    corridor: bool,
}

/// Render state owned by `World`. Placement is cached by level, and the
/// frame plan and raster allocation are reused.
pub struct ForeLayer {
    items: Vec<PlacedItem>,
    key: Option<(Theme, u64)>,
    frame: Vec<Plan>,
    /// Static entries drawn last frame. An incumbent keeps its slot until it
    /// has fully left the view, so a newcomer never evicts a visible item.
    prev: Vec<ForeEntry>,
    bufs: RenderBuffers,
    rasters: [Focus; MAX_FRAME_ITEMS],
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
            frame: Vec::with_capacity(MAX_FRAME_ITEMS),
            prev: Vec::with_capacity(MAX_FRAME_ITEMS),
            bufs: RenderBuffers::default(),
            rasters: std::array::from_fn(|_| Focus::default()),
        }
    }

    /// Build the static item list once per level.
    pub fn sync(&mut self, level: &Level) {
        let key = (level.theme, level.seed);
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.items.clear();
        self.frame.clear();
        self.prev.clear();
        generate(level, &mut self.items);
    }

    #[cfg(test)]
    pub(crate) fn capture_entries(&self) -> impl Iterator<Item = (ForeEntry, Vec2, bool)> + '_ {
        self.items.iter().map(|p| (p.entry, p.anchor, p.corridor))
    }

    #[cfg(test)]
    pub(crate) fn force_single_entry(&mut self, level: &Level, entry: ForeEntry, anchor: Vec2) {
        self.key = Some((level.theme, level.seed));
        self.items.clear();
        self.items.push(PlacedItem { entry, anchor, corridor: false });
    }

    #[cfg(test)]
    pub(crate) fn capture_shadow_only(
        &mut self,
        entry: ForeEntry,
        anchor: Vec2,
        size: Vec2,
        zoom: f32,
    ) -> Vec2 {
        let view = View { cam: Vec2::ZERO, size, zoom };
        let normal = entry.normal();
        let (near, _) = project(size, normal);
        let block = entry_pixel_scale(entry, view);
        let (_, shadow_end) = cross_extent(entry, block, block / 2);
        let world_cross = near - shadow_end - shadow_offset(entry) + 14.0 * zoom;
        let cam = anchor - size * 0.5;
        let cam = cam + normal * (entry.cross - world_cross - cam.dot(normal));
        let plan = plan_entry(View { cam, size, zoom }, entry, anchor, &[])
            .expect("the item shadow enters the view");
        assert_eq!(plan.body.fade, 0.0);
        assert!(plan.shadow.fade > 0.0);
        self.items.retain(|placed| placed.entry == entry);
        cam + size * 0.5
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

        // The Flatbed's analytic sky track is the level's foreground design:
        // wires, cloud shadow and the roadside reach each own a plane and ease
        // in and out at a screen edge, so they are planned every frame and
        // never contest a slot with each other or with a static placement.
        // Selecting among them (three candidates, two slots, nearest to the
        // view centre) is what switched a shadow on and off mid-screen.
        for entry in sky.into_iter().flatten() {
            if let Some(plan) = plan_entry(view, entry, rat, threats) {
                self.frame.push(plan);
            }
        }

        // Static placements keep the two closest visible items, but an item
        // that was drawn last frame holds its slot until it has fully left.
        // Parallel lines keep 48 screen px between visible planes, and at most
        // one whole-tree canopy (`TreeCanopy`) may appear on screen at a time.
        let mut chosen = 0;
        for &entry in &self.prev {
            if chosen >= MAX_BANDS_IN_VIEW || self.frame.len() >= MAX_FRAME_ITEMS {
                break;
            }
            if !self.items.iter().any(|p| p.entry == entry)
                || self.frame.iter().any(|p| p.entry == entry)
            {
                continue;
            }
            let Some(plan) = plan_entry(view, entry, rat, threats) else { continue };
            if !self.frame.iter().all(|other| frames_together(&plan, other, view)) {
                continue;
            }
            self.frame.push(plan);
            chosen += 1;
        }
        let items = self.items.iter().map(|p| p.entry);
        while chosen < MAX_BANDS_IN_VIEW && self.frame.len() < MAX_FRAME_ITEMS {
            let next = items.clone()
                .filter_map(|entry| plan_entry(view, entry, rat, threats))
                .filter(|p| !self.frame.iter().any(|other| other.entry == p.entry))
                .filter(|p| self.frame.iter().all(|other| frames_together(p, other, view)))
                .min_by(|a, b| a.centre_distance().total_cmp(&b.centre_distance()));
            let Some(next) = next else { break };
            self.frame.push(next);
            chosen += 1;
        }
        // Painter's order: the farthest plane first, so a nearer item covers a
        // farther one. A larger depth is nearer the lens, and equal depths keep
        // the selection order (`sort_by` is stable), which is the nearest-first
        // contest order.
        self.frame.sort_by(|a, b| a.entry.depth.total_cmp(&b.entry.depth));
        self.prev.clear();
        self.prev.extend(self.frame.iter().map(|p| p.entry));

        let fill = coverage_fill(level.theme);
        let covered: f32 = self.frame.iter()
            .filter(|p| !(p.entry.is_cloud() || p.entry.is_wires()))
            .map(|p| p.body_area(view.size) * fill)
            .sum();
        let cap = view.size.x * view.size.y * coverage_pct(level.theme) as f32 / 100.0;
        if covered > cap {
            let factor = (cap / covered).max(COVERAGE_ALPHA_MIN);
            for plan in &mut self.frame {
                if !plan.entry.is_cloud() {
                    plan.alpha = (plan.alpha as f32 * factor).round() as u8;
                }
            }
        }
        if let [back, front] = self.frame.as_mut_slice() {
            let near_rat = |p: &Plan| {
                p.body.fade > 0.0
                    && p.rat_distance <= p.half_thickness + CLEAR_DISC_IN_PX * view.zoom
            };
            front.crossing_rat = (back.normal - front.normal).len_sq() > 1e-4
                && !back.entry.is_cloud()
                && near_rat(back)
                && near_rat(front);
        }
    }
}

/// Whether `p` may share the frame with `other`: two visible planes of parallel
/// lines must keep [`MIN_SCREEN_SPACING`] on screen, and only one 2D canopy may
/// be on screen at a time.
fn frames_together(p: &Plan, other: &Plan, view: View) -> bool {
    if p.entry.is_canopy() && other.entry.is_canopy() {
        return false;
    }
    (p.normal - other.normal).len_sq() > 1e-4
        || p.visible_lines().all(|cross| {
            other.visible_lines().all(|other_cross| {
                (cross - other_cross).abs() as f32 >= MIN_SCREEN_SPACING * view.zoom
            })
        })
}

/// The scene is still composited at world resolution, before resampling.
/// Multiply screen-space distances by the eased zoom to get buffer distances.
#[derive(Clone, Copy)]
struct View {
    cam: Vec2,
    size: Vec2,
    zoom: f32,
}

fn blur_radius(entry: ForeEntry, zoom: f32) -> i32 {
    (((entry.depth - DEPTH_MIN) / (DEPTH_MAX - DEPTH_MIN) * 2.0 + 2.0)
        * zoom).round().max(1.0) as i32
}

fn cross_extent(entry: ForeEntry, block: i32, pad: i32) -> (f32, f32) {
    let samples = entry.cross_samples();
    ((samples.start * block - pad) as f32, (samples.end * block + pad) as f32)
}

fn shadow_offset(entry: ForeEntry) -> f32 {
    10.0 + 4.0 * entry.axis.x.abs()
}

/// Screen-pixel width of the ease at a plane's screen edge. The Flatbed's
/// roadside reaches sweep at [`SKY_BRANCH_SPEED`] screen pixels a second, where
/// the default [`EDGE_FADE_PX`] ramp would be a 0.18 s flick on the body, so
/// they (and the canopy kind elsewhere) ease over [`SKY_EDGE_FADE_PX`].
fn edge_fade_px(entry: ForeEntry, zoom: f32) -> f32 {
    let ramp = if entry.is_sky_reach() { SKY_EDGE_FADE_PX } else { EDGE_FADE_PX };
    ramp * zoom.max(0.25)
}

fn plan_entry(view: View, entry: ForeEntry, rat: Vec2, threats: &[(Vec2, f32)]) -> Option<Plan> {
    let normal = entry.normal();
    let view_span = project(view.size, normal);
    let along = project(view.size, entry.axis);
    let centre_cross = (view_span.0 + view_span.1) * 0.5;
    let centre_along = (along.0 + along.1) * 0.5;
    let world_cross = entry.cross - view.cam.dot(normal);
    let screen_cross = centre_cross + (world_cross - centre_cross) * entry.depth;
    let k = entry_pixel_scale(entry, view);
    let radius = blur_radius(entry, view.zoom);
    let fade_px = edge_fade_px(entry, view.zoom);
    let (mut body, shadow, body_tip, shadow_tip) = match entry.shape {
        ForeShape::Band(_) => {
            let body = Plane::new_band(
                screen_cross, cross_extent(entry, k, radius), view_span, fade_px,
            );
            let shadow = Plane::new_band(
                world_cross + shadow_offset(entry),
                cross_extent(entry, k, k / 2),
                view_span,
                fade_px,
            );
            (body, shadow, f32::INFINITY, f32::INFINITY)
        }
        ForeShape::Reach { kind, tip_along } => {
            let spec = art_foreground_reach::reach_spec(kind);
            let cam_along = view.cam.dot(entry.axis);
            let raw_body_tip = tip_along - cam_along;
            let raw_shadow_tip = raw_body_tip + REACH_SHADOW_LEAD;
            let (body_tip, shadow_tip) = if spec.shape == ReachShape::Canopy {
                // `TreeCanopy` draws nothing for `local_x <= 0` (trunk is off-screen);
                // clamp its tip at `along.0 + tip_len * k` so its flat diameter never
                // detaches from the anchor viewport edge.
                let max_tip = along.0 + (spec.tip_len.round() as i32 * k) as f32;
                (raw_body_tip.min(max_tip), raw_shadow_tip.min(max_tip))
            } else {
                (raw_body_tip, raw_shadow_tip)
            };
            let basis = (entry.axis, normal);
            let body = Plane::new_reach(
                screen_cross,
                cross_extent(entry, k, radius),
                body_tip + radius as f32,
                basis,
                view,
                view_span,
                fade_px,
            );
            let shadow = Plane::new_reach(
                world_cross + shadow_offset(entry),
                cross_extent(entry, k, k / 2),
                shadow_tip + (k / 2) as f32,
                basis,
                view,
                view_span,
                fade_px,
            );
            (body, shadow, body_tip, shadow_tip)
        }
    };
    if entry.is_cloud() {
        body.fade = 0.0;
    }
    if body.fade == 0.0 && shadow.fade == 0.0 {
        return None;
    }
    let half_thickness = entry.art_thickness() * 0.5 * k as f32 + radius as f32;
    let is_canopy_shape = matches!(
        entry.shape,
        ForeShape::Reach { kind, .. }
            if art_foreground_reach::reach_spec(kind).shape == ReachShape::Canopy
    );
    let shape_distance = |at: Vec2| -> f32 {
        let rel = at - view.cam;
        let dc = rel.dot(normal) - body.cross as f32;
        if is_canopy_shape {
            let spec_len = match entry.shape {
                ForeShape::Reach { kind, .. } => art_foreground_reach::reach_spec(kind).tip_len,
                ForeShape::Band(_) => 0.0,
            };
            let hub_along = body_tip - spec_len * k as f32;
            let du = (rel.dot(entry.axis) - hub_along).max(0.0);
            ((dc * dc + du * du).sqrt() - half_thickness).max(0.0)
        } else {
            let da = if body_tip.is_finite() {
                (rel.dot(entry.axis) - body_tip).max(0.0)
            } else {
                0.0
            };
            (dc * dc + da * da).sqrt()
        }
    };
    let rat_distance = shape_distance(rat);
    let mut alpha = proximity_alpha(
        ALPHA_MAX as f32, rat_distance / view.zoom, FADE_RAT_PX, ALPHA_FLOOR_RAT,
    );
    for &(body_pos, threat_r) in threats {
        let raw = shape_distance(body_pos);
        let surf = if is_canopy_shape { raw } else { (raw - half_thickness).max(0.0) };
        let distance = (surf - threat_r.max(0.0)).max(0.0);
        alpha = alpha.min(proximity_alpha(
            ALPHA_MAX as f32, distance / view.zoom, FADE_THREAT_PX, ALPHA_FLOOR,
        ));
    }
    if entry.is_cloud() {
        alpha = alpha.min(CLOUD_ALPHA_MAX as f32);
    }
    Some(Plan {
        entry,
        normal,
        body,
        shadow,
        centre_cross,
        centre_along,
        along,
        body_tip,
        shadow_tip,
        half_thickness,
        rat_distance,
        alpha: alpha.round() as u8,
        k,
        radius,
        crossing_rat: false,
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
    // The Flatbed has no static placements: its whole foreground is the three
    // analytic sky planes, each easing in and out at its own screen edge. They
    // are exempt from the static coverage budget (the cap sits above the whole
    // frame) because dimming or truncating one plane as another sweeps in reads
    // as a pop — the half-disc canopy alone fills most of a frame and used to
    // cut the wires' body off outright.
    let flatbed_sky = scene.level.theme == Theme::Trailer;
    let cap_skip = if flatbed_sky {
        total * FLATBED_SKY_COVERAGE_PCT / 100
    } else {
        total * coverage_skip_pct(scene.level.theme) / 100
    };
    budget_start(cap_skip);
    clear_disc_set(scene.rat - scene.cam, view.zoom);
    for (plan, raster) in layer.frame.iter().zip(&mut layer.rasters) {
        let painted = writes_so_far();
        let est_writes = if scene.level.theme == Theme::Outfall {
            plan.body_area(view.size) * OUTFALL_COVERAGE_FILL
        } else if plan.entry.is_canopy() {
            plan.body_area(view.size) * COVERAGE_FILL
        } else {
            plan.body_area(view.size)
        };
        let alpha = if flatbed_sky {
            plan.alpha
        } else if est_writes > cap_skip.saturating_sub(painted) as f32 {
            plan.alpha.min(CLOUD_ALPHA_MAX)
        } else {
            coverage_alpha(plan.alpha, painted, cap_full)
        };
        raster.draw(pb, &mut layer.bufs, view, plan, alpha, scene);
    }
    clear_disc_reset();
    budget_finish()
}

#[derive(Clone, Copy, PartialEq)]
enum RasterKind {
    Band(ForeKind),
    Reach(ReachKind),
}

#[derive(Clone, Copy, PartialEq)]
struct RasterKey {
    kind: RasterKind,
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
    /// For bands: world-fixed along-axis art lattice `[first, last)`.
    first: i32,
    last: i32,
    /// For reaches: cached local art length from anchor side to tip (`<= tip_len` for canopies).
    reach_len: i32,
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
        bufs: &mut RenderBuffers,
        view: View,
        plan: &Plan,
        alpha: u8,
        scene: &ForeScene,
    ) {
        let entry = plan.entry;
        let block = plan.k;
        let cam_along = view.cam.dot(entry.axis);
        let toward_centre = if plan.body.cross as f32 <= plan.centre_cross { 1 } else { -1 };

        let (body_along_origin, shadow_along_origin) = match entry.shape {
            ForeShape::Band(kind) => {
                let item = entry.as_band().unwrap();
                let pad = BAND_OVERHANG as f32 * view.zoom + plan.radius as f32 + 1.0;
                let (lo, hi) = plan.along;
                let first = ((cam_along + lo - pad) / block as f32).floor() as i32;
                let last = ((cam_along + hi + pad) / block as f32).ceil() as i32;
                let key = RasterKey {
                    kind: RasterKind::Band(kind),
                    seed: entry.seed,
                    scale: entry.scale,
                    theme: scene.level.theme,
                    sway: sway_of(&item, scene.elapsed),
                    toward_centre,
                    block,
                    radius: plan.radius,
                };
                if self.key != Some(key) || first < self.first || last > self.last {
                    self.first = first.div_euclid(RASTER_PAGE) * RASTER_PAGE;
                    self.last = last.div_euclid(RASTER_PAGE) * RASTER_PAGE + RASTER_PAGE;
                    self.reach_len = 0;
                    let width = self.last - self.first;
                    let art_half = (plan.half_thickness / block as f32).ceil() as i32 + 2;
                    bufs.scratch.reset(0, -art_half, width, 2 * art_half + 1);
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
                    TARGET.with(|t| *t.borrow_mut() = Some(std::mem::take(&mut bufs.scratch)));
                    art_foreground::draw(pb, &ForeItem { axis: HORIZONTAL, ..item }, &d);
                    bufs.scratch = TARGET.with(|t| t.borrow_mut().take()).unwrap_or_default();
                    self.filter(&mut bufs.scratch, key, false);
                    self.key = Some(key);
                    #[cfg(test)]
                    {
                        self.rebuilds += 1;
                    }
                }
                let origin = (self.first * block) as f32 - cam_along;
                (origin, origin)
            }
            ForeShape::Reach { kind, .. } => {
                let spec = art_foreground_reach::reach_spec(kind);
                let paged_len = if spec.shape == ReachShape::Canopy {
                    // Canopy contract: clamp `len` to `spec.tip_len` (`36`) so
                    // `local_x <= 0` (off-screen trunk) is never requested.
                    spec.tip_len.round().max(1.0) as i32
                } else {
                    let overhang = BAND_OVERHANG as f32 * view.zoom;
                    let (along_lo, _) = plan.along;
                    let needed_px = plan.body_tip - along_lo
                        + REACH_SHADOW_LEAD + overhang + plan.radius as f32 + 4.0;
                    let needed_art = (needed_px / block as f32).ceil() as i32;
                    let min_art = spec.tip_len.ceil() as i32 + 8;
                    let req_len = needed_art.max(min_art).max(RASTER_PAGE);
                    ((req_len + RASTER_PAGE - 1) / RASTER_PAGE) * RASTER_PAGE
                };
                let t = reach_time_of(spec.organic, entry.seed, scene.elapsed);
                let key = RasterKey {
                    kind: RasterKind::Reach(kind),
                    seed: entry.seed,
                    scale: entry.scale,
                    theme: scene.level.theme,
                    sway: t,
                    toward_centre,
                    block,
                    radius: plan.radius,
                };
                if self.key != Some(key) || self.reach_len != paged_len {
                    self.reach_len = paged_len;
                    self.first = 0;
                    self.last = paged_len;
                    let art_half = (plan.half_thickness / block as f32).ceil() as i32 + 6;
                    let art_h = (2 * art_half + 1) as usize;
                    let art_w = paged_len as usize;
                    bufs.reach_body.set_pen(0);
                    bufs.reach_body.resize(art_w, art_h);
                    bufs.reach_body.clear((0, 0, 0));
                    bufs.reach_shadow.set_pen(0);
                    bufs.reach_shadow.resize(art_w, art_h);
                    bufs.reach_shadow.clear((0, 0, 0));
                    art_foreground_reach::draw_reach(
                        kind,
                        &mut bufs.reach_body,
                        &mut bufs.reach_shadow,
                        paged_len,
                        t,
                        scene.level.theme,
                    );
                    unpack_reach_buffers(
                        &mut bufs.scratch,
                        &bufs.reach_body,
                        &bufs.reach_shadow,
                        paged_len,
                        art_half,
                    );
                    self.filter(&mut bufs.scratch, key, true);
                    self.key = Some(key);
                    #[cfg(test)]
                    {
                        self.rebuilds += 1;
                    }
                }
                let span_px = (self.reach_len * block) as f32;
                (plan.body_tip - span_px, plan.shadow_tip - span_px)
            }
        };

        let opacity = u32::from(alpha);
        let shadow_opacity = if entry.is_cloud() {
            opacity * 256 / u32::from(ALPHA_MAX)
        } else {
            141 + opacity * 115 / u32::from(ALPHA_MAX)
        };
        let shadow_opacity = (shadow_opacity as f32 * plan.shadow.fade).round() as u32;
        let body_opacity = (opacity as f32 * plan.body.fade).round() as u32;
        self.blit(pb, plan, shadow_along_origin, shadow_opacity, true);
        self.blit(pb, plan, body_along_origin, body_opacity, false);
    }

    /// Filter the expanded art with a tent kernel in each direction. Each
    /// pass carries premultiplied RGB + emissive weight so empty samples soften
    /// the silhouette without a dark fringe and emissive lamp lenses retain glow.
    fn filter(&mut self, scratch: &mut Scratch, key: RasterKey, clamp_anchor: bool) {
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
        if key.kind != RasterKind::Band(ForeKind::CloudBand) {
            scratch.filtered_rows.resize(width * rows, [0; 5]);
            for (source, row) in scratch.body.chunks_exact(scratch.w as usize)
                .zip(scratch.filtered_rows.chunks_exact_mut(width))
            {
                for (x, dst) in row.iter_mut().enumerate() {
                    *dst = tent(x as i32, block, radius, divisor, |i| {
                        let idx = if clamp_anchor && i < 0 { 0 } else { i };
                        if idx < 0 {
                            [0; 5]
                        } else {
                            source.get(idx as usize).copied().unwrap_or_default()
                        }
                    });
                }
            }
            for (y, row) in self.body.chunks_exact_mut(width).enumerate() {
                for (x, dst) in row.iter_mut().enumerate() {
                    let [r, g, b, a, e] = tent(y as i32, block, radius, divisor, |i| {
                        if (0..scratch.h).contains(&i) {
                            scratch.filtered_rows[i as usize * width + x]
                        } else {
                            [0; 5]
                        }
                    });
                    *dst = if a < 6 {
                        [0; 4]
                    } else {
                        let rgb = ((r / a) as u8, (g / a) as u8, (b / a) as u8);
                        let gray = crate::render::lighting::luminance(rgb);
                        let muted = palette::mix(palette::scale(rgb, DARKEN), (gray, gray, gray), 0.18);
                        let silhouette = palette::mix(muted, ambient, 0.16);
                        let tint = if e == 0 {
                            silhouette
                        } else {
                            palette::mix(silhouette, rgb, (e as f32 / a as f32).clamp(0.0, 1.0))
                        };
                        [stepped(tint.0 as u32, 8, 255), stepped(tint.1 as u32, 8, 255),
                            stepped(tint.2 as u32, 8, 255), stepped(a, 12, ALPHA_MAX)]
                    };
                }
            }
        }
        for (y, row) in self.shadow.chunks_exact_mut(width).enumerate() {
            let (v, fy) = shadow_phase(y as i32, block);
            for (x, dst) in row.iter_mut().enumerate() {
                let (u, fx) = shadow_phase(x as i32, block);
                let sample = |sx: i32, sy: i32| {
                    let sx = if clamp_anchor && sx < 0 { 0 } else { sx };
                    scratch.index(sx, sy + scratch.y0).map_or(0, |i| u32::from(scratch.shade[i]))
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
        let axis = plan.entry.axis;
        let normal = plan.normal;
        let cross = if shade { plan.shadow.cross } else { plan.body.cross } as f32;
        let (width, height) = (pb.w, pb.h);
        let (cx, cy, zoom, disc) = CLEAR_DISC.with(|c| c.get());
        let disc_active = disc && (!shade || plan.crossing_rat);
        let fade_width = 12.0;
        let limit = (CLEAR_DISC_OUT_PX + if plan.crossing_rat { fade_width } else { 0.0 }) * zoom;
        let limit2 = limit * limit;
        let rows = pb.pixels_mut().chunks_exact_mut(width);
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

/// Convert `draw_reach`'s local `body` (un-premultiplied RGB + material pens)
/// and `shadow` (`(s, s, s)` grayscale mask) buffers into `Scratch`, leaving
/// `REACH_TIP_PAD` empty columns past `len` so the tent filter feathers the
/// free tip or outer canopy rim cleanly into empty air.
fn unpack_reach_buffers(
    scratch: &mut Scratch,
    body: &PixelBuffer,
    shadow: &PixelBuffer,
    len: i32,
    art_half: i32,
) {
    let w = body.w.min(len.max(0) as usize);
    let h = body.h;
    scratch.reset(0, -art_half, len.max(0) + REACH_TIP_PAD, h as i32);
    let body_px = body.pixels();
    let body_mat = body.mat();
    let shadow_px = shadow.pixels();
    let cy = h as i32 / 2;
    for y in 0..h {
        let sy = y as i32 - cy;
        let row = y * body.w;
        let srow = y * shadow.w;
        for x in 0..w {
            let rgb = body_px[row + x];
            if rgb != (0, 0, 0) {
                let pen = body_mat.get(row + x).copied().unwrap_or(0);
                scratch.put_with_emissive(x as i32, sy, rgb, ALPHA_MAX, pen == EMISSIVE_PEN);
            }
            if x < shadow.w && y < shadow.h {
                let s = shadow_px[srow + x].0;
                if s > 0 {
                    scratch.put_shade(x as i32, sy, (f32::from(s) / 255.0) * 0.36);
                }
            }
        }
    }
}

#[inline]
fn tent(
    at: i32, block: i32, radius: i32, divisor: u32, mut get: impl FnMut(i32) -> [u32; 5],
) -> [u32; 5] {
    let mut sum = [0; 5];
    let mut p = at - radius;
    let last = at + radius;
    while p <= last {
        let sample = p.div_euclid(block);
        let end = ((sample + 1) * block - 1).min(last);
        let weight = (p..=end).map(|q| radius + 1 - (q - at).abs()).sum::<i32>() as u32;
        let rgba = get(sample);
        for i in 0..5 {
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

fn sway_of(item: &ForeItem, elapsed: f32) -> f32 {
    if !item.kind.sways() {
        return 0.0;
    }
    let step = (elapsed.max(0.0) * SWAY_HZ).floor() as u32;
    let h = mix32(step.wrapping_mul(0x9E37_79B1) ^ item.seed);
    let unit = (h & 0xFFFF) as f32 / 65535.0;
    let v = unit * 2.0 - 1.0;
    (v * 4.0).round() * 0.25
}

fn reach_time_of(organic: bool, seed: u32, elapsed: f32) -> f32 {
    if !organic {
        return 0.0;
    }
    let phase = (seed & 0xFF) as f32 * 0.0625;
    ((elapsed.max(0.0) + phase) * REACH_SWAY_HZ).floor() / REACH_SWAY_HZ
}

#[inline]
fn mix32(mut z: u32) -> u32 {
    z ^= z >> 16;
    z = z.wrapping_mul(0x7FEB_352D);
    z ^= z >> 15;
    z = z.wrapping_mul(0x846C_A68B);
    z ^ (z >> 16)
}

// ---- Generation ----------------------------------------------------------

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

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * unit
    }

    fn u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
}

/// Check that `entry` clears `level.start` by `START_CLEAR_PX` (widened for
/// `TreeCanopy` up to 2x zoom) and never crosses the start room.
fn clears_start(entry: ForeEntry, level: &Level, start: Option<&crate::game::level::Room>) -> bool {
    let normal = entry.normal();
    let k = if entry.is_canopy() { 4 } else { band_pixel_scale(&entry) };
    let half_world = entry.art_thickness() * 0.5 * k as f32;
    let min_clear = START_CLEAR_PX.max(half_world + 48.0);
    let dc_start = (entry.cross - level.start.dot(normal)).abs();
    let clear_of_start = match entry.shape {
        ForeShape::Band(_) => dc_start >= min_clear,
        ForeShape::Reach { tip_along, .. } => {
            let da = (level.start.dot(entry.axis) - tip_along).max(0.0);
            (dc_start * dc_start + da * da).sqrt() >= min_clear
        }
    };
    if !clear_of_start {
        return false;
    }
    if let Some(r) = start {
        let half_size = Vec2::new(r.rect.w as f32, r.rect.h as f32) * (TILE_F * 0.5);
        let reach_cross = half_size.x * normal.x.abs()
            + half_size.y * normal.y.abs()
            + (half_world - 24.0).max(0.0);
        let in_cross = (entry.cross - r.rect.center_world().dot(normal)).abs() <= reach_cross;
        if in_cross {
            match entry.shape {
                ForeShape::Band(_) => return false,
                ForeShape::Reach { tip_along, .. } => {
                    let reach_along =
                        half_size.x * entry.axis.x.abs() + half_size.y * entry.axis.y.abs();
                    let room_min_along = r.rect.center_world().dot(entry.axis) - reach_along;
                    if tip_along >= room_min_along {
                        return false;
                    }
                }
            }
        }
    }
    true
}

// Roll band and reach candidates from wide rooms and long corridor runs, then
// spread a small 2–4 item selection across the level.
fn generate(level: &Level, out: &mut Vec<PlacedItem>) {
    if level.theme == Theme::Trailer {
        return;
    }
    let mut rng = Gen::new(crate::game::level::level_seed(level.theme, level.seed, GEN_SALT));
    let count = (2 + rng.u32() % 3) as usize;
    let band_kinds = kinds_for(level.theme);
    let reach_kinds = reach_kinds_for(level.theme);
    let start = level.rooms.iter().find(|room| room.kind == RoomKind::Start);
    let mut candidates = Vec::new();
    for (room_idx, room) in level.rooms.iter().enumerate() {
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
        let mut room_has_canopy = false;
        for slot in 0..slots {
            for attempt in 0..PLACE_ATTEMPTS {
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

                let depth = if level.theme == Theme::Outfall {
                    rng.range(OUTFALL_DEPTH_MIN, OUTFALL_DEPTH_MAX)
                } else {
                    rng.range(DEPTH_MIN, DEPTH_MAX)
                };
                let scale = rng.range(SCALE_MIN, SCALE_MAX);
                let pick_roll = rng.u32();
                let seed = rng.u32();

                let want_reach = !reach_kinds.is_empty() && ((room_idx + slot as usize) & 1 == 0);
                let entry = if want_reach {
                    let rkind = reach_kinds[(pick_roll as usize) % reach_kinds.len()];
                    if is_canopy_reach(rkind) && (corridor || room_has_canopy) {
                        continue;
                    }
                    let sides = reach_allowed_sides(rkind);
                    if corridor
                        && !art_foreground_reach::reach_spec(rkind).organic
                        && !sides.iter().any(|&s| (!is_vertical_edge(s)) == horizontal)
                    {
                        continue;
                    }
                    let pref = if corridor { Some(horizontal) } else { None };
                    let axis = reach_placement_axis(rkind, pref, seed);
                    ForeEntry::reach(rkind, axis, at, depth, seed)
                } else {
                    let mut bkind = band_kinds[(pick_roll % band_kinds.len() as u32) as usize];
                    if !corridor && bkind == ForeKind::HullRib && room_has_rib {
                        bkind = if seed & 1 == 0 { ForeKind::Kelp } else { ForeKind::BubbleChain };
                    }
                    let axis = placement_axis(bkind, horizontal, seed);
                    let mut item = ForeItem { kind: bkind, axis, cross: 0.0, depth, scale, seed };
                    item.cross = at.dot(item.normal());
                    ForeEntry::from_band(fit_scale(item))
                };

                if !clears_start(entry, level, start) {
                    continue;
                }
                if matches!(entry.shape, ForeShape::Band(ForeKind::HullRib)) {
                    room_has_rib = true;
                }
                if entry.is_canopy() {
                    room_has_canopy = true;
                }
                candidates.push((PlacedItem {
                    entry,
                    anchor: at,
                    #[cfg(test)]
                    corridor,
                }, quality));
                break;
            }
        }
    }

    for _ in 0..count {
        let have_band = out.iter().any(|p| matches!(p.entry.shape, ForeShape::Band(_)));
        let have_reach = out.iter().any(|p| matches!(p.entry.shape, ForeShape::Reach { .. }));
        let have_canopy = out.iter().any(|p| p.entry.is_canopy());
        let best = candidates.iter().enumerate().filter_map(|(i, (cand, quality))| {
            if cand.entry.is_canopy() && have_canopy {
                return None;
            }
            let mut spacing = cand.anchor.dist_sq(level.start);
            let normal = cand.entry.normal();
            for placed in out.iter() {
                if (normal - placed.entry.normal()).len_sq() <= 1e-4
                    && (cand.entry.cross - placed.entry.cross).abs() < MIN_SPACING
                {
                    return None;
                }
                spacing = spacing.min(placed.anchor.dist_sq(cand.anchor));
            }
            let is_reach = matches!(cand.entry.shape, ForeShape::Reach { .. });
            let mixes_shape = !out.is_empty()
                && ((is_reach && !have_reach) || (!is_reach && !have_band && !reach_kinds.is_empty()));
            let fresh_organic = cand.entry.organic()
                && out.iter().all(|p| {
                    std::mem::discriminant(&p.entry.shape) != std::mem::discriminant(&cand.entry.shape)
                        || p.entry.shape != cand.entry.shape
                });
            Some((i, mixes_shape, fresh_organic, spacing * (1.0 + quality * 0.50)))
        }).max_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.total_cmp(&b.3))
        });
        let Some((index, _, _, _)) = best else { break };
        out.push(candidates.swap_remove(index).0);
    }
}

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

fn sky_track(view: View, entry: ForeEntry) -> (f32, f32) {
    let (lo, hi) = project(view.size, entry.normal());
    let k = entry_pixel_scale(entry, view);
    let (shadow_lo, shadow_hi) = cross_extent(entry, k, k / 2);
    let offset = shadow_offset(entry);
    let mut start = lo - shadow_hi - offset;
    let mut end = hi - shadow_lo - offset;
    if !entry.is_cloud() {
        let (body_lo, body_hi) = cross_extent(entry, k, blur_radius(entry, view.zoom));
        let centre = (lo + hi) * 0.5;
        start = start.min(centre + (lo - body_hi - centre) / entry.depth);
        end = end.max(centre + (hi - body_lo - centre) / entry.depth);
    }
    (start - SKY_TRACK_MARGIN, end + SKY_TRACK_MARGIN)
}

fn flatbed_reach_kind(seed: u64, event: u64) -> ReachKind {
    // The Flatbed draws no horizontal item, so only kinds that may enter from
    // the top or bottom edge (a vertical long axis) are eligible.
    let kinds: Vec<ReachKind> = reach_kinds_for(Theme::Trailer)
        .into_iter()
        .filter(|&k| reach_allowed_sides(k).iter().any(|&s| is_vertical_edge(s)))
        .collect();
    if kinds.is_empty() {
        return ReachKind::TreeCanopy;
    }
    if event.is_multiple_of(3) {
        if let Some(&tc) = kinds.iter().find(|&&k| is_canopy_reach(k)) {
            return tc;
        }
    }
    let others: Vec<ReachKind> = kinds
        .iter()
        .copied()
        .filter(|&k| !is_canopy_reach(k))
        .collect();
    if others.is_empty() {
        kinds[(event as usize) % kinds.len()]
    } else {
        let pick = ((event / 3) as usize + (event % 3) as usize - 1 + (seed as usize & 1))
            % others.len();
        others[pick]
    }
}

/// Analytic tracks on the Flatbed: centre-pinned vertical wires, drifting cloud
/// shadow, and off-screen-anchored roadside reaches (`TreeCanopy`, `LightPole`,
/// `SignArm`) entering from the top or bottom edge only, honouring each kind's
/// `reach_spec(kind).sides`. Nothing on the Flatbed lies horizontally: every
/// item's long axis is vertical, so it can only sweep in and out across a side
/// edge.
fn sky_items(level: &Level, view: View, elapsed: f32) -> [Option<ForeEntry>; 3] {
    if level.theme != Theme::Trailer {
        return [None; 3];
    }
    let seed = crate::game::level::level_seed(level.theme, level.seed, GEN_SALT);
    let t = (elapsed.max(0.0) * SKY_QUANT_HZ).floor() / SKY_QUANT_HZ;
    let band_entry = |kind, horizontal, depth, scale, seed| {
        let axis = placement_axis(kind, horizontal, seed);
        ForeEntry::from_band(fit_scale(ForeItem { kind, axis, cross: 0.0, depth, scale, seed }))
    };

    // Wires run up and down: a vertical band pinned to the view centre, so it
    // never leaves the frame and never fades.
    let mut wires = band_entry(ForeKind::PowerLines, false, 2.0, 1.15, hash_u32(seed, 0x12));
    wires.cross = (view.cam + view.size * 0.5).dot(wires.normal());

    let mut cloud = band_entry(ForeKind::CloudBand, false, 1.8, 1.25, hash_u32(seed, 0xC10D));
    let (start, end) = sky_track(view, cloud);
    let travel = end - start;
    let speed = SKY_CLOUD_SPEED * view.zoom / cloud.depth;
    let phase = (hash01(seed, 0xC10D) * travel + t * speed).rem_euclid(travel);
    cloud.cross = view.cam.dot(cloud.normal()) + start + phase;

    let jitter = |event| (hash01(seed ^ 0x5EED_0000, event) - 0.5) * 2.0;
    let nearest = (t / SKY_BRANCH_PERIOD).floor() as u64;
    let roadside = (nearest.saturating_sub(1)..=nearest + 1).rev().find_map(|event| {
        let age = t - (SKY_BRANCH_PERIOD * event as f32 + jitter(event));
        if age < 0.0 || age >= SKY_BRANCH_PERIOD + 2.0 {
            return None;
        }
        let rkind = flatbed_reach_kind(seed, event);
        let spec = art_foreground_reach::reach_spec(rkind);
        let ev_seed = hash_u32(seed ^ 0x1EAF_0000, event);
        let sides = reach_allowed_sides(rkind);

        // Intersect `sides` with the north/south highway shoulders (`Top`/`Bottom`):
        // `LightPole` (`[Bottom]`) always takes `Bottom`; `TreeCanopy` and `SignArm`
        // (`ALL`) take `Top` or `Bottom` along the roadside. A kind that may only
        // enter from a side edge would lie horizontally, so it is skipped.
        let mut vert_sides = [AnchorSide::Bottom; 4];
        let mut vn = 0;
        for &s in sides {
            if is_vertical_edge(s) && vn < vert_sides.len() {
                vert_sides[vn] = s;
                vn += 1;
            }
        }
        if vn == 0 {
            return None;
        }
        let chosen_side = vert_sides[(hash_u32(seed ^ 0x51DE_0000, event) as usize) % vn];

        // Every Flatbed reach kind is axis-aligned on its edge (no organic lean),
        // so the long axis is exactly the inward vertical.
        let axis = inward_axis(chosen_side);

        let depth = if spec.shape == ReachShape::Canopy { 2.2 } else if spec.organic { 2.6 } else { 2.4 };
        let mut entry = ForeEntry::reach(rkind, axis, Vec2::ZERO, depth, ev_seed);
        let normal = entry.normal();

        let from_north = chosen_side == AnchorSide::Top;
        let (start, end) = sky_track(view, entry);
        let speed = (SKY_BRANCH_SPEED * view.zoom / entry.depth)
            .max((end - start) / (SKY_BRANCH_PERIOD - 2.0));
        let travelled = age * speed;
        // The track is the whole crossing plus `SKY_TRACK_MARGIN`, so both the
        // body and the shadow's offset rect are clear of the view at each end;
        // the item is dropped only once it has fully left.
        (travelled < end - start).then(|| {
            let cross = if normal.x < 0.0 { start + travelled } else { end - travelled };
            entry.cross = view.cam.dot(normal) + cross;
            let tip_world_y = if spec.shape == ReachShape::Canopy {
                // Seat the full semicircular canopy flush on its anchor edge.
                let k = entry_pixel_scale(entry, view);
                let full_in = (spec.tip_len.round() as i32 * k) as f32;
                if from_north {
                    view.cam.y + full_in
                } else {
                    view.cam.y + view.size.y - full_in
                }
            } else {
                let mid_y = view.cam.y + view.size.y * 0.5;
                if (900.0..=1660.0).contains(&mid_y) {
                    let reach_in = 152.0 + hash01(seed ^ 0x7190_0000, event) * 48.0;
                    if from_north { 1072.0 + reach_in } else { 1488.0 - reach_in }
                } else {
                    let frac = 0.38 + hash01(seed ^ 0x7190_0000, event) * 0.20;
                    if from_north {
                        view.cam.y + view.size.y * frac
                    } else {
                        view.cam.y + view.size.y * (1.0 - frac)
                    }
                }
            };
            let tip_world_x = view.cam.x + view.size.x * 0.5
                + (entry.cross - (view.cam + view.size * 0.5).dot(normal)) * normal.x;
            let tip_world = Vec2::new(tip_world_x, tip_world_y);
            entry.shape = ForeShape::Reach { kind: rkind, tip_along: tip_world.dot(axis) };
            entry
        })
    });
    [Some(wires), Some(cloud), roadside]
}

/// Find a Flatbed clock at which the requested sky band or roadside reach is
/// prominently visible in view.
#[cfg(test)]
pub(crate) fn sky_capture_time(
    level: &Level,
    cam: Vec2,
    size: Vec2,
    zoom: f32,
    elapsed: f32,
    target: ForeShape,
) -> Option<f32> {
    if level.theme != Theme::Trailer {
        return None;
    }
    let view = View { cam, size, zoom };
    // `RATRUN_DUMP_SKY_AT=<seconds>` pins the Flatbed clock to `elapsed + n`
    // instead of searching for a prominent frame, so one capture can step a run
    // of frames across a whole sweep (0.0, 0.25, 0.5 …) and show the body and
    // its shadow travelling in from the edge and out the other side.
    if let Ok(at) = std::env::var("RATRUN_DUMP_SKY_AT") {
        if let Ok(at) = at.trim().parse::<f32>() {
            return Some(elapsed + at.max(0.0));
        }
    }
    let mut layer = ForeLayer::new();
    for step in 0..(60 * SKY_QUANT_HZ as i32) {
        let t = elapsed + step as f32 / SKY_QUANT_HZ;
        layer.plan_frame(view, level, cam + size * 0.5, t, &[]);
        if layer.frame.iter().any(|p| {
            let matches_kind = match (p.entry.shape, target) {
                (ForeShape::Band(a), ForeShape::Band(b)) => a == b,
                (ForeShape::Reach { kind: a, .. }, ForeShape::Reach { kind: b, .. }) => a == b,
                (ForeShape::Reach { kind: ReachKind::TreeCanopy, .. }, ForeShape::Band(ForeKind::BranchCanopy)) => true,
                _ => false,
            };
            if !matches_kind {
                return false;
            }
            let plane = if p.entry.is_cloud() { p.shadow } else { p.body };
            if plane.fade <= 0.75 {
                return false;
            }
            if p.entry.axis.x.abs() > 0.9 {
                let (along_lo, along_hi) = p.along;
                p.body_tip >= along_lo + (along_hi - along_lo) * 0.32
            } else {
                (plane.cross as f32 - p.centre_cross).abs() <= 16.0 * zoom
            }
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
        layer.items = items.iter().map(|&item| PlacedItem {
            entry: ForeEntry::from_band(item),
            anchor: Vec2::ZERO,
            corridor: false,
        }).collect();
    }

    fn force_entries(layer: &mut ForeLayer, level: &Level, entries: &[ForeEntry]) {
        layer.key = Some((level.theme, level.seed));
        layer.items = entries.iter().map(|&entry| PlacedItem {
            entry,
            anchor: entry.tip_world().unwrap_or(Vec2::ZERO),
            corridor: false,
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

    fn axis_matches_allowed_side(kind: ReachKind, axis: Vec2) -> bool {
        let spec = art_foreground_reach::reach_spec(kind);
        spec.sides.iter().any(|&side| {
            let inward = inward_axis(side);
            if spec.organic && spec.shape == ReachShape::Strip {
                axis.dot(inward) >= 0.70
            } else {
                (axis - inward).len_sq() <= 1e-5
            }
        })
    }

    #[test]
    fn placement_is_seeded_mixes_bands_and_reaches_and_respects_sides_budget_and_clearance() {
        for theme in Theme::all() {
            let mut saw_band = false;
            let mut saw_reach = false;
            for seed in [0, 1, 2, 7, 11, 21, 54, 809] {
                let world = World::new(theme, seed);
                let mut a = ForeLayer::new();
                let mut b = ForeLayer::new();
                a.sync(&world.level);
                b.sync(&world.level);
                let a: Vec<_> = a.capture_entries().collect();
                assert_eq!(a, b.capture_entries().collect::<Vec<_>>());
                assert!((2..=4).contains(&a.len()), "{theme:?} seed {seed}: {} items", a.len());
                let canopies = a.iter().filter(|(e, _, _)| e.is_canopy()).count();
                assert!(canopies <= 1, "{theme:?} seed {seed}: {canopies} canopies placed");
                let start_room = world.level.rooms.iter().find(|r| r.kind == RoomKind::Start);
                for (i, &(entry, anchor, corridor)) in a.iter().enumerate() {
                    let normal = entry.normal();
                    assert!((entry.cross - anchor.dot(normal)).abs() <= 1e-3);
                    assert!(clears_start(entry, &world.level, start_room));
                    assert!(world.level.tiles.get_at(anchor).is_open());
                    if !entry.organic() || entry.is_canopy() {
                        assert!(
                            entry.axis.x.abs() <= 1e-5 || entry.axis.y.abs() <= 1e-5,
                            "item must be axis-aligned: {entry:?}"
                        );
                    }
                    assert!((DEPTH_MIN..=DEPTH_MAX).contains(&entry.depth));
                    match entry.shape {
                        ForeShape::Band(kind) => {
                            saw_band = true;
                            assert!(kinds_for(theme).contains(&kind));
                            assert!((SCALE_MIN..=SCALE_MAX).contains(&entry.scale));
                            assert!((9.999..=30.001).contains(&entry.art_thickness()));
                        }
                        ForeShape::Reach { kind, tip_along } => {
                            saw_reach = true;
                            let spec = art_foreground_reach::reach_spec(kind);
                            assert!(spec.floors.contains(&theme), "{kind:?} not allowed on {theme:?}");
                            assert!((tip_along - anchor.dot(entry.axis)).abs() <= 1e-3);
                            assert!(
                                axis_matches_allowed_side(kind, entry.axis),
                                "{kind:?} axis {:?} violates allowed sides {:?}",
                                entry.axis,
                                spec.sides
                            );
                        }
                    }
                    assert!(a[..i].iter().all(|(other, _, _)| {
                        (other.normal() - normal).len_sq() > 1e-4
                            || (other.cross - entry.cross).abs() >= MIN_SPACING
                    }));
                    if corridor {
                        let (cx, cy) = ((anchor.x / TILE_F) as i32, (anchor.y / TILE_F) as i32);
                        assert!(world.level.rooms.iter().any(|r| {
                            r.kind == RoomKind::Corridor
                                && r.rect.w.max(r.rect.h) >= CORRIDOR_MIN_TILES
                                && (r.rect.x..r.rect.right()).contains(&cx)
                                && (r.rect.y..r.rect.bottom()).contains(&cy)
                        }));
                    }
                }
            }
            assert!(saw_band, "{theme:?} never placed a band");
            if !reach_kinds_for(theme).is_empty() {
                assert!(saw_reach, "{theme:?} never placed a reach");
            }
        }
    }

    #[test]
    fn allowed_sides_come_from_art_spec_and_govern_flatbed_and_rooms() {
        for seed in 0..32u32 {
            assert_eq!(
                reach_placement_axis(ReachKind::LightPole, None, seed),
                inward_axis(AnchorSide::Bottom)
            );
            assert_eq!(
                reach_placement_axis(ReachKind::HangingLamp, None, seed),
                inward_axis(AnchorSide::Top)
            );
            assert_eq!(
                reach_placement_axis(ReachKind::CraneHook, None, seed),
                inward_axis(AnchorSide::Top)
            );
        }
        let world = World::new(Theme::Trailer, 5);
        let view = View { cam: Vec2::new(600.0, 1240.0), size: Vec2::new(240.0, 160.0), zoom: 1.0 };
        let mut seen = [false; 3];
        for step in 0..(30 * SKY_QUANT_HZ as u32) {
            let [_, _, mover] = sky_items(&world.level, view, step as f32 / SKY_QUANT_HZ);
            if let Some(entry) = mover {
                if let ForeShape::Reach { kind, .. } = entry.shape {
                    assert!(
                        axis_matches_allowed_side(kind, entry.axis),
                        "Flatbed {kind:?} axis {:?} violates {:?}",
                        entry.axis,
                        reach_allowed_sides(kind)
                    );
                    match kind {
                        ReachKind::TreeCanopy => seen[0] = true,
                        ReachKind::LightPole => seen[1] = true,
                        ReachKind::SignArm => seen[2] = true,
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(seen, [true, true, true], "Flatbed sky track should cycle TreeCanopy, LightPole, SignArm");
    }

    #[test]
    fn tree_canopy_scales_to_cover_most_of_view_caps_at_one_and_fades_over_rat() {
        let mut world = World::new(Theme::Kitchen, 11);
        let mut layer = ForeLayer::new();
        for zoom in [0.5, 1.0, 2.0] {
            let size = Vec2::new(VIEW.0 as f32, VIEW.1 as f32) * zoom;
            let short = size.x.min(size.y);
            let view = View { cam: Vec2::ZERO, size, zoom };
            let canopy = ForeEntry::reach(
                ReachKind::TreeCanopy,
                inward_axis(AnchorSide::Top),
                Vec2::new(size.x * 0.5, short * 0.95),
                2.2,
                1,
            );
            let plan = plan_entry(view, canopy, Vec2::new(-500.0, -500.0), &[]).unwrap();
            let radial_px = art_foreground_reach::CANOPY_RADIUS * plan.k as f32;
            let ratio = radial_px / short;
            assert!(
                (0.80..=1.00).contains(&ratio),
                "zoom {zoom}: canopy radial extent {radial_px} / {short} = {ratio:.3} outside 0.80..=1.00"
            );
        }

        let view = View { cam: Vec2::ZERO, size: Vec2::new(120.0, 80.0), zoom: 1.0 };
        let c1 = ForeEntry::reach(ReachKind::TreeCanopy, inward_axis(AnchorSide::Top), Vec2::new(60.0, 72.0), 2.2, 1);
        let c2 = ForeEntry::reach(ReachKind::TreeCanopy, inward_axis(AnchorSide::Left), Vec2::new(72.0, 40.0), 2.4, 2);
        let lamp = ForeEntry::reach(ReachKind::HangingLamp, inward_axis(AnchorSide::Top), Vec2::new(108.0, 36.0), 2.0, 3);
        force_entries(&mut layer, &world.level, &[c1, c2, lamp]);

        // At most one TreeCanopy may appear on screen at a time.
        layer.plan_frame(view, &world.level, Vec2::new(-500.0, -500.0), 0.0, &[]);
        let canopy_count = layer.frame.iter().filter(|p| p.entry.is_canopy()).count();
        assert_eq!(canopy_count, 1, "at most one TreeCanopy may be on screen at a time");

        // Standing anywhere inside the half-disc fades the canopy; standing far outside keeps full alpha.
        let plan_under = plan_entry(view, c1, Vec2::new(95.0, 35.0), &[]).unwrap();
        assert_eq!(plan_under.alpha, ALPHA_FLOOR_RAT as u8);
        let plan_far = plan_entry(view, c1, Vec2::new(60.0, 220.0), &[]).unwrap();
        assert_eq!(plan_far.alpha, ALPHA_MAX);

        // Rendering the full-view canopy respects the hard coverage budget and clamps `len <= 36`.
        force_entries(&mut layer, &world.level, &[c1]);
        let (_, used) = render(&mut world, &mut layer, Vec2::ZERO, 1.0, 0.0);
        assert!(used > 0);
        assert_eq!(layer.rasters[0].reach_len, art_foreground_reach::CANOPY_RADIUS as i32);
        assert!(used <= (VIEW.0 * VIEW.1) as u32 * COVERAGE_SKIP_PCT / 100);
    }

    #[test]
    fn reach_anchor_end_is_never_inside_the_view_at_any_camera_or_zoom() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
        let tip = Vec2::new(600.0, 500.0);
        let cases = [
            (ReachKind::PipeElbow, HORIZONTAL),
            (ReachKind::SignArm, -HORIZONTAL),
            (ReachKind::HangingLamp, VERTICAL),
            (ReachKind::LightPole, -VERTICAL),
            (ReachKind::VineTendril, Vec2::new(-0.8, 0.6)),
            (ReachKind::KelpFrond, Vec2::new(-0.6, -0.8)),
            (ReachKind::TreeCanopy, inward_axis(AnchorSide::Top)),
            (ReachKind::TreeCanopy, inward_axis(AnchorSide::Bottom)),
            (ReachKind::TreeCanopy, inward_axis(AnchorSide::Left)),
            (ReachKind::TreeCanopy, inward_axis(AnchorSide::Right)),
        ];
        for (kind, axis) in cases {
            let spec = art_foreground_reach::reach_spec(kind);
            let entry = ForeEntry::reach(kind, axis, tip, 2.2, 91);
            force_entries(&mut layer, &world.level, &[entry]);
            for zoom in [0.5, 1.0, 1.5, 2.0] {
                let size = Vec2::new(VIEW.0 as f32, VIEW.1 as f32) * zoom;
                for gx in -4..=4 {
                    for gy in -4..=4 {
                        let cam = tip - size * 0.5 + Vec2::new(gx as f32 * 55.0, gy as f32 * 42.0);
                        let (mut pb, _) = render(&mut world, &mut layer, cam, zoom, 1.25);
                        let any_painted = pb.pixels().iter().any(|&px| px != FLOOR);
                        if !any_painted {
                            continue;
                        }
                        let plan = &layer.frame[0];
                        let raster = &layer.rasters[0];
                        let span_px = (raster.reach_len * plan.k) as f32;
                        let body_origin = plan.body_tip - span_px;
                        let shadow_origin = plan.shadow_tip - span_px;
                        if spec.shape == ReachShape::Canopy {
                            // Canopy's flat base (`u == 0`, trunk at `x = -11 < 0`) never
                            // detaches past the anchor viewport edge `along.0` into the view.
                            assert!(
                                raster.reach_len <= spec.tip_len.round() as i32,
                                "canopy reach_len {} exceeded tip_len {}",
                                raster.reach_len,
                                spec.tip_len
                            );
                            assert!(
                                shadow_origin <= plan.along.0 + 0.5,
                                "TreeCanopy axis {axis:?} zoom {zoom} cam {cam:?}: shadow detached ({shadow_origin} > {})",
                                plan.along.0
                            );
                            assert!(
                                body_origin <= plan.along.0 + 0.5,
                                "TreeCanopy axis {axis:?} zoom {zoom} cam {cam:?}: body detached ({body_origin} > {})",
                                plan.along.0
                            );
                        } else {
                            let overhang = BAND_OVERHANG as f32 * zoom;
                            assert!(
                                shadow_origin <= plan.along.0 - overhang + 0.5,
                                "{kind:?} axis {axis:?} zoom {zoom} cam {cam:?}: shadow_origin {shadow_origin} > {}",
                                plan.along.0 - overhang
                            );
                            assert!(
                                body_origin <= plan.along.0 - overhang + 0.5,
                                "{kind:?} axis {axis:?} zoom {zoom} cam {cam:?}: body_origin {body_origin} > {}",
                                plan.along.0 - overhang
                            );
                        }

                        if plan.body.fade > 0.0 {
                            pb.clear(FLOOR);
                            raster.blit(&mut pb, plan, body_origin, u32::from(ALPHA_MAX), false);
                            let origin = plan.normal * plan.body.cross as f32;
                            let (u_enter, u_exit) = line_span(origin, entry.axis, size);
                            if u_enter < u_exit && plan.body_tip > u_enter + 8.0 * zoom {
                                let search = if spec.shape == ReachShape::Canopy {
                                    (24.0 * zoom) as i32
                                } else {
                                    6
                                };
                                let edge = origin + entry.axis * (u_enter + 2.0);
                                let hit = (-search..=search).any(|off| {
                                    let p = edge + plan.normal * (off as f32);
                                    let ex = p.x.round();
                                    let ey = p.y.round();
                                    ex >= 0.0
                                        && ex < size.x
                                        && ey >= 0.0
                                        && ey < size.y
                                        && pb.pixel_at(ex as usize, ey as usize) != FLOOR
                                });
                                assert!(
                                    hit,
                                    "anchor gap at entry edge {edge:?} for {kind:?} axis {axis:?} zoom {zoom} cam {cam:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn reach_tip_is_fixed_in_world_space_and_parallax_acts_only_across_the_ray() {
        let mut world = World::new(Theme::Kitchen, 12);
        let mut layer = ForeLayer::new();
        let tip = Vec2::new(420.0, 340.0);
        let entry = ForeEntry::reach(ReachKind::HangingLamp, VERTICAL, tip, 2.5, 55);
        force_entries(&mut layer, &world.level, &[entry]);

        let view0 = View { cam: Vec2::new(360.0, 300.0), size: Vec2::new(120.0, 80.0), zoom: 1.0 };
        let rat_far = Vec2::ZERO;
        let p0 = plan_entry(view0, entry, rat_far, &[]).unwrap();
        assert!((p0.body_tip - (tip.y - view0.cam.y)).abs() <= 1e-4);

        let view_along = View { cam: view0.cam + Vec2::new(0.0, 14.0), ..view0 };
        let p_along = plan_entry(view_along, entry, rat_far, &[]).unwrap();
        assert!((p0.body_tip - p_along.body_tip - 14.0).abs() <= 1e-4);
        assert_eq!(p0.body.cross, p_along.body.cross);
        assert_eq!(p0.shadow.cross, p_along.shadow.cross);

        let view_across = View { cam: view0.cam + Vec2::new(8.0, 0.0), ..view0 };
        let p_across = plan_entry(view_across, entry, rat_far, &[]).unwrap();
        assert!((p0.body_tip - p_across.body_tip).abs() <= 1e-4);
        assert_eq!(p0.shadow.cross - p_across.shadow.cross, 8);
        assert_eq!(p0.body.cross - p_across.body.cross, 20);

        let (pb_a, used_a) = render(&mut world, &mut layer, view0.cam, 1.0, 0.0);
        let (pb_b, used_b) = render(&mut world, &mut layer, view0.cam + Vec2::new(0.0, 6.0), 1.0, 0.0);
        assert!(used_a > 0 && used_b > 0);
        let max_painted_y = |pb: &PixelBuffer| {
            (0..pb.h)
                .rev()
                .find(|&y| (0..pb.w).any(|x| pb.pixel_at(x, y) != FLOOR))
                .expect("painted row")
        };
        assert_eq!(max_painted_y(&pb_a) - max_painted_y(&pb_b), 6);
        assert!((0..pb_a.w).all(|x| pb_a.pixel_at(x, pb_a.h - 1) == FLOOR));
    }

    #[test]
    fn reach_shadow_leads_in_from_the_anchor_edge_before_the_body() {
        let mut world = World::new(Theme::Sewer, 7);
        let size = Vec2::new(VIEW.0 as f32, VIEW.1 as f32);
        for (kind, axis) in [
            (ReachKind::PipeElbow, HORIZONTAL),
            (ReachKind::SignArm, -HORIZONTAL),
            (ReachKind::CraneHook, VERTICAL),
            (ReachKind::LightPole, -VERTICAL),
            (ReachKind::VineTendril, Vec2::new(0.6, 0.8)),
            (ReachKind::TreeCanopy, VERTICAL),
        ] {
            let tip = Vec2::new(500.0, 500.0);
            let entry = ForeEntry::reach(kind, axis, tip, 2.2, 73);
            let mut layer = ForeLayer::new();
            force_entries(&mut layer, &world.level, &[entry]);
            let normal = entry.normal();
            let (along_lo, _) = project(size, axis);
            let base_cam = tip - size * 0.5;
            let base_cam = base_cam + axis * (tip.dot(axis) - along_lo - base_cam.dot(axis));
            let base_cam = base_cam + normal * (entry.cross - (size * 0.5).dot(normal) - base_cam.dot(normal));

            let mut saw_shadow_only = false;
            let mut saw_body_after = false;
            for step in -18..=28 {
                let cam = base_cam - axis * (step as f32);
                let (pb, used) = render(&mut world, &mut layer, cam, 1.0, 0.0);
                let any_dark = pb.pixels().iter().any(|&px| px != FLOOR);
                if let Some(plan) = layer.frame.first() {
                    if plan.shadow.fade > 0.0 && plan.body.fade == 0.0 && any_dark {
                        assert_eq!(used, 0, "shadow-only reach frame must spend zero body budget");
                        saw_shadow_only = true;
                        assert!(!saw_body_after, "shadow led after body for {kind:?} {axis:?}");
                    }
                    if plan.body.fade > 0.0 && used > 0 {
                        saw_body_after = true;
                    }
                }
            }
            assert!(saw_shadow_only, "shadow never led in tip-first for {kind:?} {axis:?}");
            assert!(saw_body_after, "body never entered after shadow for {kind:?} {axis:?}");
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
            let entry = ForeEntry::from_band(item);
            let normal = entry.normal();
            let (lo, hi) = project(size, normal);
            let k = band_pixel_scale(&entry);
            let (shadow_lo, shadow_hi) = cross_extent(entry, k, k / 2);
            for (edge, start, inward) in [(lo, lo - shadow_hi, 1.0), (hi, hi - shadow_lo, -1.0)] {
                let mut layer = ForeLayer::new();
                force_items(&mut layer, &world.level, &[item]);
                let camera = |step| normal * (shadow_offset(entry) - (start + inward * step as f32));
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
                assert!(first_step > 0 && first_depth <= 3.0 * zoom, "{axis:?}, edge {edge}: {depths:?}");
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

                let (still, _) = render(&mut world, &mut layer, camera(28), zoom, 0.0);
                assert_eq!(inside.pixels(), still.pixels());
                let centre_step = (((lo + hi) * 0.5 + shadow_offset(entry)) - start) * inward;
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
            kind: RasterKind::Band(ForeKind::MainPipe), seed: 0, scale: 1.0, theme: Theme::Sewer,
            sway: 0.0, toward_centre: 1, block: 3, radius: 4,
        }, false);
        let row = &focus.body[12 * focus.width as usize..13 * focus.width as usize];
        let alpha = |x: usize| row[x][3];
        assert!(alpha(5) > 0 && alpha(5) < alpha(6));
        assert!(alpha(6) < alpha(7) && alpha(7) < alpha(9));
    }

    #[test]
    fn organic_angles_come_from_level_seeds() {
        let mut angles = Vec::new();
        for theme in [Theme::Outfall, Theme::Sewer] {
            for seed in [1, 2, 7, 11] {
                let world = World::new(theme, seed);
                let mut layer = ForeLayer::new();
                layer.sync(&world.level);
                for (entry, _, _) in layer.capture_entries().filter(|(e, _, _)| e.organic() && !e.is_canopy()) {
                    assert!(entry.axis.x.abs() > 0.001 && entry.axis.y.abs() > 0.001);
                    if !angles.contains(&entry.axis) {
                        angles.push(entry.axis);
                    }
                }
            }
        }
        for seed in [1, 2, 7, 11] {
            let mut c = 0;
            let world = World::new(Theme::Trailer, seed);
            let view = View { cam: Vec2::ZERO, size: Vec2::new(120.0, 80.0), zoom: 1.0 };
            for step in 0..(SKY_QUANT_HZ as u32 * 10) {
                let sky = sky_items(&world.level, view, step as f32 / SKY_QUANT_HZ);
                let kinds: Vec<_> = sky.into_iter().flatten().collect();
                assert!((2..=4).contains(&kinds.len()));
                if let Some(mover) = sky[2] {
                    if mover.is_canopy() {
                        assert!(mover.axis.x.abs() <= 1e-5 && mover.axis.y.abs() > 0.99);
                        c += 1;
                    }
                }
            }
            assert!(c > 0, "no canopy passed for seed {seed}");
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
        assert_eq!(layer.frame[0].entry, ForeEntry::from_band(near));
        assert_eq!(layer.frame[1].entry, ForeEntry::from_band(spaced));

        let reach_left = ForeEntry::reach(ReachKind::PipeElbow, HORIZONTAL, Vec2::new(80.0, 90.0), 2.0, 1);
        let reach_right_close = ForeEntry::reach(ReachKind::PipeElbow, -HORIZONTAL, Vec2::new(100.0, 110.0), 2.0, 2);
        let reach_right_spaced = ForeEntry::reach(ReachKind::PipeElbow, -HORIZONTAL, Vec2::new(100.0, 150.0), 2.0, 3);
        force_entries(&mut layer, &world.level, &[reach_right_close, reach_right_spaced, reach_left]);
        layer.plan_frame(view, &world.level, Vec2::new(-500.0, -500.0), 0.0, &[]);
        assert_eq!(layer.frame.len(), 2);
        assert_eq!(layer.frame[0].entry, reach_left);
        assert_eq!(layer.frame[1].entry, reach_right_spaced);

        let crossing = ForeItem { axis: Vec2::new(0.8, -0.6), cross: 126.0, ..near };
        force_items(&mut layer, &world.level, &[near, crossing]);
        layer.plan_frame(view, &world.level, centre, 0.0, &[]);
        assert_eq!(layer.frame.len(), 2);
        assert!(layer.frame[1].crossing_rat);
        let along = near.axis * 14.0;
        let still = plan_entry(view, ForeEntry::from_band(near), centre, &[]).unwrap();
        let slid = plan_entry(View { cam: along, ..view }, ForeEntry::from_band(near), centre, &[]).unwrap();
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
            assert_eq!(layer.frame[0].entry, ForeEntry::from_band(near));
            assert_eq!(layer.frame[1].entry, ForeEntry::from_band(spaced));
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
            let plan = plan_entry(view, ForeEntry::from_band(item), rat, &[]).unwrap();
            assert_eq!(plan.body.fade, 0.0);
            assert!(plan.shadow.fade > 0.0);
        }
        force_items(&mut layer, &world.level, &items);
        layer.plan_frame(view, &world.level, rat, 0.0, &[]);
        assert_eq!(layer.frame.len(), MAX_BANDS_IN_VIEW);
        assert_eq!(layer.frame[0].entry, ForeEntry::from_band(nearest));
        assert_eq!(layer.frame[1].entry, ForeEntry::from_band(spaced));
    }

    #[test]
    fn filtered_detail_is_world_anchored_along_the_band() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut layer = ForeLayer::new();
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
                let warm = layer.bufs.scratch.capacity();
                let raster = layer.rasters[0].body.as_ptr();
                let (again, _) = render(&mut world, &mut layer, cam, 1.0, 0.0);
                assert_eq!(again.pixels(), a.pixels());
                assert_eq!(layer.bufs.scratch.capacity(), warm);
                assert_eq!(layer.rasters[0].body.as_ptr(), raster);
            }
        }
    }

    #[test]
    fn only_the_cross_axis_parallaxes_and_the_rat_fades_against_the_line() {
        let view = View { cam: Vec2::ZERO, size: Vec2::new(180.0, 180.0), zoom: 1.0 };
        let item = pipe(true, 90.0, 2.5);
        let rat = Vec2::new(1000.0, 90.0);
        let plan = plan_entry(view, ForeEntry::from_band(item), rat, &[]).unwrap();
        assert_eq!(plan.alpha, ALPHA_FLOOR_RAT as u8);
        let moved = plan_entry(View { cam: Vec2::new(6.0, 4.0), ..view }, ForeEntry::from_band(item), rat, &[]).unwrap();
        assert_eq!(plan.body.cross - moved.body.cross, 10);
        assert_eq!(plan.shadow.cross - moved.shadow.cross, 4);
        let far = plan_entry(view, ForeEntry::from_band(item), Vec2::new(1000.0, 0.0), &[]).unwrap();
        assert_eq!(far.alpha, ALPHA_MAX);

        let reach = ForeEntry::reach(ReachKind::PipeElbow, HORIZONTAL, Vec2::new(50.0, 90.0), 2.5, 9);
        let past_tip = plan_entry(view, reach, Vec2::new(170.0, 90.0), &[]).unwrap();
        assert_eq!(past_tip.alpha, ALPHA_MAX);
        let under_tip = plan_entry(view, reach, Vec2::new(40.0, 90.0), &[]).unwrap();
        assert_eq!(under_tip.alpha, ALPHA_FLOOR_RAT as u8);
    }

    #[test]
    fn a_still_scene_draws_identically_twice_for_bands_and_reaches() {
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
            let pct = if theme == Theme::Trailer {
                FLATBED_SKY_COVERAGE_PCT
            } else {
                coverage_skip_pct(theme)
            };
            assert!(a_used <= (VIEW.0 * VIEW.1) as u32 * pct / 100);
        }
        let mut world = World::new(Theme::Sewer, 9);
        for kind in all_reach_kinds() {
            let mut layer = ForeLayer::new();
            let entry = ForeEntry::stage_reach(kind, Vec2::new(60.0, 45.0), 17);
            force_entries(&mut layer, &world.level, &[entry]);
            let (a, a_used) = render(&mut world, &mut layer, Vec2::ZERO, 1.0, 4.375);
            let rebuilds = layer.rasters[0].rebuilds;
            let (b, b_used) = render(&mut world, &mut layer, Vec2::ZERO, 1.0, 4.375);
            assert_eq!(a.pixels(), b.pixels(), "{kind:?}");
            assert_eq!(a_used, b_used);
            assert_eq!(layer.rasters[0].rebuilds, rebuilds, "{kind:?} rebuilt on frozen frame");
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
                assert!(layer.frame.iter().any(|p| p.entry.is_wires()));
                assert!(layer.frame.len() <= MAX_FRAME_ITEMS);
            }
        }
    }

    /// The Flatbed draws no item whose long axis is horizontal, at any clock and
    /// camera: wires run up and down, the roadside reaches enter from the top or
    /// bottom edge, and the cloud is a band on the vertical axis.
    #[test]
    fn no_flatbed_item_lies_horizontally() {
        for seed in [5, 11] {
            let world = World::new(Theme::Trailer, seed);
            let mut layer = ForeLayer::new();
            for cam in [Vec2::new(654.5, 1241.0), Vec2::new(600.0, 380.0)] {
                let view = View { cam, size: Vec2::new(VIEW.0 as f32, VIEW.1 as f32), zoom: 1.0 };
                for step in 0..(60 * SKY_QUANT_HZ as u32) {
                    let elapsed = step as f32 / SKY_QUANT_HZ;
                    for entry in sky_items(&world.level, view, elapsed).into_iter().flatten() {
                        assert!(
                            entry.axis.x.abs() <= 1e-5 && entry.axis.y.abs() > 0.99,
                            "seed {seed}: Flatbed {entry:?} lies horizontally"
                        );
                    }
                    layer.plan_frame(view, &world.level, cam + view.size * 0.5, elapsed, &[]);
                    for plan in &layer.frame {
                        assert!(
                            plan.entry.axis.x.abs() <= 1e-5,
                            "seed {seed} t {elapsed:.2}: planned {:?} lies horizontally",
                            plan.entry.shape
                        );
                    }
                }
            }
        }
    }

    /// A sky item's shadow travels in and out continuously: across a sweep
    /// sampled every frame its drawn area never steps, it enters at the screen
    /// edge while the body is still off screen, and it is never dropped before
    /// it has fully left (no flip at a 5–9 s schedule boundary).
    #[test]
    fn flatbed_sky_shadows_cross_without_a_jump() {
        let size = Vec2::new(VIEW.0 as f32, VIEW.1 as f32);
        let cam = Vec2::new(654.5, 1241.0);
        // A tag per item, so a series follows one item rather than one slot.
        let tag = |entry: ForeEntry| -> (u8, u8) {
            match entry.shape {
                ForeShape::Band(ForeKind::PowerLines) => (1, 0),
                ForeShape::Band(ForeKind::CloudBand) => (2, 0),
                ForeShape::Band(_) => (3, 0),
                ForeShape::Reach { kind, .. } => (4, kind as u8),
            }
        };
        for seed in [5, 11] {
            let mut world = World::new(Theme::Trailer, seed);
            let mut layer = ForeLayer::new();
            let mut prev: Vec<((u8, u8), f32)> = Vec::new();
            let mut prev_dark = 0.0f32;
            let mut prev_canopy = false;
            let mut led_in = 0;
            for step in 0..(24 * 60) {
                let elapsed = step as f32 / 60.0;
                let (pb, _) = render(&mut world, &mut layer, cam, 1.0, elapsed);
                let drawn: Vec<((u8, u8), f32)> = layer.frame.iter()
                    .map(|p| {
                        let alpha = p.alpha as f32 / ALPHA_MAX as f32;
                        (tag(p.entry), p.shadow_area(size) * p.shadow.fade * alpha)
                    })
                    .collect();
                if step > 0 {
                    for &(key, cur) in &drawn {
                        if let Some(&(_, before)) = prev.iter().find(|(k, _)| *k == key) {
                            let jump = (cur - before).abs();
                            // The floor is for the entry itself: the area and
                            // the fade both ramp linearly, so the drawn figure
                            // grows quadratically in 30 Hz steps (1, 20, 51, 96,
                            // 179, 258, 351, 498 …). Measured over both seeds,
                            // the largest step still under the 30 % bound is
                            // 121 px² (230 -> 351); a pop is the whole shadow.
                            assert!(
                                jump <= (cur.max(before) * 0.30).max(128.0),
                                "seed {seed} t {elapsed:.3}: {key:?} shadow drew {before:.0} -> {cur:.0} px"
                            );
                            if before < 1.0 && cur > 0.0 {
                                let plan = layer.frame.iter()
                                    .find(|p| tag(p.entry) == key)
                                    .expect("the drawn item is planned");
                                assert_eq!(
                                    plan.body.fade, 0.0,
                                    "seed {seed} t {elapsed:.3}: {key:?} body led its shadow in"
                                );
                                let (lo, hi) = project(size, plan.normal);
                                let gap = (plan.shadow.span.1 - lo).min(hi - plan.shadow.span.0);
                                assert!(
                                    gap <= SKY_EDGE_FADE_PX + 8.0,
                                    "seed {seed} t {elapsed:.3}: {key:?} shadow appeared {gap:.0} px inside the edge"
                                );
                                led_in += 1;
                            }
                        }
                    }
                    // An item that was drawn must not vanish until it has left.
                    for &(key, before) in &prev {
                        assert!(
                            before <= 64.0 || drawn.iter().any(|(k, _)| *k == key),
                            "seed {seed} t {elapsed:.3}: {key:?} vanished with {before:.0} px drawn"
                        );
                    }
                }
                let dark = pb.pixels().iter()
                    .filter(|&&(r, g, b)| {
                        (r, g, b) != FLOOR && r <= FLOOR.0 && g <= FLOOR.1 && b <= FLOOR.2
                    })
                    .count() as f32;
                // The buffer is counted whole (a body's dark pixels cannot be
                // told from the shadow's), so the step bound is skipped while
                // the half-disc canopy is on screen: it covers most of the
                // frame by design and doubles that count. Its own shadow is
                // checked above through `shadow_area` like every other item.
                let canopy_on = layer.frame.iter()
                    .any(|p| p.entry.is_canopy() && p.body.fade > 0.0);
                if step > 0 && !canopy_on && !prev_canopy {
                    assert!(
                        (dark - prev_dark).abs() <= (dark.max(prev_dark) * 0.35).max(900.0),
                        "seed {seed} t {elapsed:.3}: {prev_dark:.0} -> {dark:.0} darkened px"
                    );
                }
                prev_canopy = canopy_on;
                prev_dark = dark;
                prev = drawn;
            }
            assert!(led_in >= 2, "seed {seed}: only {led_in} sky items led their shadow in");
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
            let cloud_fade = plan_entry(view, cloud, rat, &[]).map_or(0.0, |p| p.shadow.fade);
            if let Some((cross, fade)) = previous_cloud {
                if cloud.cross < cross {
                    assert_eq!((fade, cloud_fade), (0.0, 0.0));
                    cloud_wrapped = true;
                }
            }
            previous_cloud = Some((cloud.cross, cloud_fade));

            let (shadow, body) = branch.and_then(|entry| plan_entry(view, entry, rat, &[]))
                .map_or((0.0, 0.0), |p| (p.shadow.fade, p.body.fade));
            if step > 0 {
                if previous_branch.0 == 0.0 && shadow > 0.0 {
                    assert!(shadow <= 0.35, "roadside shadow appeared at {shadow}");
                }
                if previous_branch.0 > 0.0 && shadow == 0.0 {
                    assert!(previous_branch.0 <= 0.35, "roadside shadow left at {}", previous_branch.0);
                }
                if previous_branch.1 == 0.0 && body > 0.0 {
                    assert!(body <= 0.5, "roadside body appeared at {body}");
                }
                if previous_branch.1 > 0.0 && body == 0.0 {
                    assert!(previous_branch.1 <= 0.5, "roadside body left at {}", previous_branch.1);
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
    fn outfall_items_respect_depth_room_rib_cap_and_coverage_budget() {
        let mut world = World::new(Theme::Outfall, 7);
        let mut layer = ForeLayer::new();
        layer.sync(&world.level);
        let placed: Vec<_> = layer.capture_entries().collect();
        assert!(!placed.is_empty(), "Outfall should place foreground items");
        for &(entry, _, _) in &placed {
            assert!(
                (OUTFALL_DEPTH_MIN..=OUTFALL_DEPTH_MAX).contains(&entry.depth),
                "Outfall item depth {} outside 1.8..=2.4",
                entry.depth
            );
            match entry.shape {
                ForeShape::Band(k) => assert!(kinds_for(Theme::Outfall).contains(&k)),
                ForeShape::Reach { kind, .. } => {
                    assert!(art_foreground_reach::reach_spec(kind).floors.contains(&Theme::Outfall));
                }
            }
        }
        for room in &world.level.rooms {
            if room.kind == RoomKind::Corridor || room.kind == RoomKind::Maze {
                continue;
            }
            let ribs = placed
                .iter()
                .filter(|&&(entry, anchor, corridor)| {
                    !corridor
                        && matches!(entry.shape, ForeShape::Band(ForeKind::HullRib))
                        && room.rect.contains_world(anchor)
                })
                .count();
            assert!(ribs <= 1, "Outfall room had {ribs} HullRib bands (max 1)");
        }
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
        let kelp_reach = ForeEntry::reach(ReachKind::KelpFrond, HORIZONTAL, Vec2::new(80.0, 40.0), 2.1, 19);
        force_entries(&mut layer, &world.level, &[kelp_reach]);
        let (_, used) = render(&mut world, &mut layer, Vec2::ZERO, 1.0, 1.0);
        assert!(
            used <= max_writes,
            "KelpFrond wrote {used} px, exceeding Outfall 14% cap ({max_writes})"
        );
    }
}
