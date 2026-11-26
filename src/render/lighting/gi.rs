//! Optional Global Illumination (one-bounce coloured indirect light) and
//! Ambient Occlusion pass (`GlobalLight`: `Off`, `AoOnly`, `AoGi`; default
//! `Off`).
//!
//! ## Tier policy
//!
//! * **`GlobalLight::Off`** — complete no-op; the lighting pipeline is
//!   bit-for-bit identical to the baseline pass.
//! * **`LightingMode::Full`** — runs **AO** in `AoOnly` and `AoGi`, and
//!   one-bounce **GI** in `AoGi`.
//! * **`LightingMode::Hard`** (the 256-colour default tier) — runs **AO only**
//!   in both `AoOnly` and `AoGi`, skipping subtle coloured GI gradients that
//!   would band or churn palette cells while keeping crisp contact depth.
//! * **`LightingMode::Coarse` / `Compact`** — neither stage runs.
//!
//! ## Ambient occlusion (`capture_crevices` + `apply_ao`)
//!
//! 1. **Static level field (`LevelCache::ao`, 2 px world grid, built once per
//!    level)** — 8-direction horizon occlusion against nearby wall colliders
//!    (`Solid`, `Maze`, `RatHole` piers, `Doorway` jambs, closed one-way gates,
//!    respecting the Flatbed's open-air boundary and skipping open pits) with a
//!    10.5 px soft falloff and a quadratic concave-corner synergy term so
//!    inside room corners darken ~1.8× more than straight wall faces; static
//!    prop skirts from `Level::props` scaled in radius and depth by each prop's
//!    material height class; and inner concave creases on wall faces.
//! 2. **Pixel-exact prop base & wall crevices** — `capture_crevices` reads the
//!    pre-lit albedo luminance Laplacian (`mean_4 - centre`) on wall and raised
//!    prop pixels before bump mapping embosses the art, isolating mortar lines,
//!    cabinet seams and masonry cracks; `apply_ao` adds a 1–2 px silhouette
//!    skirt around any pen-tagged raised prop (`material::height > 0`).
//! 3. **Grounded body contact AO** — a tight foot-contact halo and 1–2 px
//!    lower-body contact tuck on grounded actors (skipping flyers, airborne
//!    bosses and Outfall swimmers), weighted away from `overhead`'s displaced
//!    ellipse so it grounds exposed feet without doubling the overhead core.
//! 4. **Anti-mud & byte budget** — direct lamp irradiance (`LightInfo::dom`)
//!    partially relieves AO inside hot pools so spotlights punch into corners;
//!    darkness is quantised to flat steps of 6/255 and open-floor lookups share
//!    horizontal pixel pairs so terminal SGR runs stay compact.
//!
//! ## Global illumination (`apply_indirect`)
//!
//! Why **wall-occluded 2D radiosity diffusion on the world-snapped 4 px grid**
//! rather than virtual point lights (VPLs):
//!
//! * VPLs selected from "brightest patches" jump or trade slots when the camera
//!   pans or two pools cross in brightness, and each VPL needs its own angular
//!   shadow trace. Because `Grid` is already snapped to 4 px world coordinates,
//!   every lit surface patch contributes proportionally with **zero VPL
//!   popping**, and wall occlusion is enforced directly on the 4 px lattice
//!   (both midpoint-checked stride-2 coarse steps and corner-checked stride-1
//!   fine steps refuse to cross `WALLS` samples).
//! * **Wall participation without leaks**: lit wall faces inject their
//!   albedo-tinted radiosity outward into adjacent front-facing floor cells
//!   (`dot(delta, wall.facing) >= 0`), floor diffusion spreads coloured bounce
//!   ~38 px across open floor, and exposed wall faces gather indirect light
//!   back from the floor cells in front of them — so a red lamp-lit wall tints
//!   nearby floor, a lava pool warms nearby walls, green droppings tint the
//!   floor around them, sunlit Flatbed oak planks bounce warm amber into cool
//!   cargo shadows, and Outfall's water bounces marine cyan onto sand and
//!   pilings, while a wall blocks bounce from reaching the room behind it.
//! * **Flicker-free & blackout-aware**: static fixtures are evaluated at unit
//!   steady power (`elapsed = 0`, `blackout = 0`, `flicker = None`), propagated
//!   once per world-snapped view key (`StaticGiCache`), and scaled linearly by
//!   `1.0 - scene.blackout` (`Sun` stays unattenuated). Moving and ground
//!   emitters drop time-based breath/pulse terms, so GI changes **only** when a
//!   light moves, spawns, fades or blacks out. Layered additively on top of
//!   `upsample::bounce` (which remains the tight 12 px local floor skirt).

use std::cell::RefCell;

use super::engine::{
    ambient_level_scaled, zone_gain, Curve, Grid, LayerMask, LightingMode, MUL_CLAMP,
};
use super::fixtures;
use super::fx::{Canvas, FxFrame};
use super::material::{self, MaterialKind};
use super::shadow::{Geometry, ShadowProfile};
use super::{ specular, theme_ambient, Falloff, Light, LightScene, LightShape };
use crate::game::art_roster_outfall;
use crate::game::art_walls::{one_way_gate_rect, wall_colliders, WallAxis, WallKind, DOORWAY_JAMB_VISUAL};
use crate::game::collision::{TileKind, TileMap, TILE, TILE_F};
use crate::game::ecs::components::{Dropping, LavaGlob, LavaPool, Pool, SpawnId};
use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::Pickup;
use crate::game::level::{Level, LightFixtureKind, LightZoneKind, PropKind, Rectf, Theme};
use crate::game::weapon::{DROPPING_ARM, DROPPING_LIFE, DROPPING_LIGHT_ENERGY, DROPPING_LIGHT_RADIUS};
use crate::math::Vec2;
use crate::render::palette::{self, Rgb};

/// Graphics setting for the Ambient Occlusion + Global Illumination stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlobalLight {
    /// Off (default): lighting pass is byte-identical to baseline.
    #[default]
    Off,
    /// Ambient occlusion only (wall/corner horizon AO, prop height skirts,
    /// wall crevices, grounded foot contact).
    AoOnly,
    /// Ambient occlusion + one-bounce coloured global illumination (`Full`
    /// runs both; `Hard` runs AO only).
    AoGi,
}

impl GlobalLight {
    /// Cycle order for the Graphics page and bare `gi` console command.
    pub const ALL: [Self; 3] = [Self::Off, Self::AoOnly, Self::AoGi];

    /// Parse a config, console or `RATRUN_GI` token.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "0" | "off" | "false" | "no" | "none" => Some(Self::Off),
            "1" | "ao" | "ao-only" | "ao_only" | "aoonly" => Some(Self::AoOnly),
            "2" | "on" | "true" | "yes" | "gi" | "ao+gi" | "ao-gi" | "ao_gi" | "aogi"
            | "full" => Some(Self::AoGi),
            _ => None,
        }
    }

    /// Canonical token persisted in `scores.json` and printed by the console.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::AoOnly => "ao",
            Self::AoGi => "gi",
        }
    }

    /// Player-facing value text on the Graphics page (`STYLE.md`: ALL CAPS).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::AoOnly => "AO ONLY",
            Self::AoGi => "AO + GI",
        }
    }

    /// Whether ambient occlusion runs at `mode`. `Full` and `Hard` both run AO;
    /// `Coarse` and `Compact` skip it.
    #[inline]
    pub const fn ao_enabled(self, mode: LightingMode) -> bool {
        !matches!(self, Self::Off) && matches!(mode, LightingMode::Full | LightingMode::Hard)
    }

    /// Whether indirect coloured bounce runs at `mode` (`Full` only).
    #[inline]
    pub const fn gi_enabled(self, mode: LightingMode) -> bool {
        matches!(self, Self::AoGi) && matches!(mode, LightingMode::Full)
    }
}

/// Session pin from `RATRUN_GI`, if set to a recognised value.
pub fn env_pin() -> Option<GlobalLight> {
    std::env::var("RATRUN_GI").ok().and_then(|v| GlobalLight::parse(&v))
}

/// Resolved startup value (`RATRUN_GI` if set, else `GlobalLight::Off`).
pub fn from_env() -> GlobalLight {
    env_pin().unwrap_or_default()
}

// ============================================================================
// Tunables
// ============================================================================

/// Wall-to-floor horizon AO reach in world pixels (soft 6–12 px falloff).
const WALL_AO_RADIUS: f32 = 10.5;
/// Peak static AO darkness (`0..=255`, ~43 % ceiling in deep concave corners).
const STATIC_AO_CAP: u8 = 110;
/// Maximum combined AO darkness (`0..=255`, ~42 %).
const TOTAL_AO_CAP: i32 = 108;
/// Flat quantisation step for AO darkness (`0..=255`) to preserve SGR runs.
const AO_QUANT_STEP: i32 = 6;

/// Crevice Laplacian noise floor (unlit luminance levels, `0..=255`).
const CREVICE_NOISE: i32 = 7;
/// Maximum crevice AO on mortar seams and recessed wall/prop joints (`~22 %`).
const CREVICE_MAX: u8 = 56;

/// Grounded bodies considered for foot-contact AO per frame (nearest rat first).
const MAX_CONTACT_BODIES: usize = 64;
/// Peak floor contact AO on an exposed foot edge (`~16 %`).
const BODY_FLOOR_AO_MAX: i32 = 40;
/// Subtle underside contact self-occlusion on the bottom 1–2 rows of grounded
/// creature sprites (`~12 %`).
const BODY_SPRITE_TUCK: i32 = 30;

/// Padding around the visible 4 px light grid (in 4 px samples = `24 px`) so
/// lit walls and pools just outside the viewport bounce smoothly into view.
const GI_PAD: usize = 6;
/// Coarse (`step = 2` samples = 8 px, midpoint-checked) flood iterations.
const GI_COARSE_ITERS: usize = 2;
/// Fine (`step = 1` sample = 4 px, corner-checked) flood iterations.
const GI_FINE_ITERS: usize = 5;
/// Per-step energy retention (`Q8`: `214 / 256 ≈ 0.836`).
const GI_DECAY_Q8: i32 = 214;
/// Master indirect gain applied to `Sample::mul` (`Q8`).
const GI_MUL_GAIN_Q8: i32 = 124;
/// Share of indirect bounce carried into `Sample::add` (`Q8` of `mul` add) so
/// dark-base floors and walls visibly shift hue toward bounced colour without
/// washing out texture contrast.
const GI_ADD_SHARE_Q8: i32 = 44;
/// Hard ceiling on GI's additive lift (`0..=255` scale) to prevent milky haze.
const GI_ADD_MAX: i16 = 24;
/// Minimum quantised `mul` increment (`Q8.8`); smaller tails are dropped.
const GI_MIN_MUL: i32 = 6;
/// Minimum quantised `add` increment (`Q8.8`); smaller tails are dropped.
const GI_MIN_ADD: i32 = 2;

/// 8 unit directions for the static 2D horizon AO integral.
const DIRS_8: [(f32, f32); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (0.707_106_77, 0.707_106_77),
    (-0.707_106_77, 0.707_106_77),
    (0.707_106_77, -0.707_106_77),
    (-0.707_106_77, -0.707_106_77),
];

// ============================================================================
// Static per-level AO & Albedo cache
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
struct LevelKey {
    theme: Theme,
    seed: u64,
    tile_seed: u32,
    props_len: usize,
    puddles_len: usize,
    clothes_len: usize,
    cols: i32,
    rows: i32,
    wall: Rgb,
    wall_edge: Rgb,
    bg: Rgb,
    bg_alt: Rgb,
    accent: Rgb,
}

impl LevelKey {
    fn of(level: &Level) -> Self {
        Self {
            theme: level.theme,
            seed: level.seed,
            tile_seed: level.tiles.seed(),
            props_len: level.props.len(),
            puddles_len: level.puddles.len(),
            clothes_len: level.clothes_piles.len(),
            cols: level.tiles.cols(),
            rows: level.tiles.rows(),
            wall: level.palette.wall,
            wall_edge: level.palette.wall_edge,
            bg: level.palette.bg,
            bg_alt: level.palette.bg_alt,
            accent: level.palette.accent,
        }
    }
}

#[derive(Default)]
struct LevelCache {
    key: Option<LevelKey>,
    last_used: u64,
    /// 2 px world-space grid dimensions (`cols * 8`, `rows * 8`).
    ao_w: usize,
    ao_h: usize,
    /// Static AO darkness (`0..=STATIC_AO_CAP`) at 2 px resolution.
    ao: Vec<u8>,
    /// 4 px world-space subcell dimensions (`cols * 4`, `rows * 4`).
    alb_w: usize,
    alb_h: usize,
    /// Diffuse bounce reflectance per 4 px subcell.
    albedo: Vec<Rgb>,
    /// Bit/byte flag: 1 where an Outfall 4 px subcell is standing water.
    water_sub: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct StaticGiKey {
    level: LevelKey,
    geo_rev: u32,
    snuffed_len: usize,
    snuffed_hash: u64,
    px0: u32,
    py0: u32,
    pw: u16,
    ph: u16,
    cap_q8: u16,
}

#[derive(Default)]
struct StaticGiCache {
    key: Option<StaticGiKey>,
    /// Propagated indoor fixture GI (`[u16; 3]` Q8.8 per padded cell), before
    /// `1.0 - blackout`.
    indoor: Vec<[u16; 3]>,
    /// Propagated sun / ambient-water GI (`[u16; 3]` Q8.8 per padded cell).
    sky: Vec<[u16; 3]>,
    any_indoor: bool,
    any_sky: bool,
}

#[derive(Clone, Copy)]
struct GroundedBody {
    spid: u64,
    d2: f32,
    /// Screen-space grounded foot centre `(x, y)`.
    sx: i32,
    sy: i32,
    /// Footprint half-width in pixels.
    half: i32,
    /// Unit `overhead` slide direction in Q8 `(ux, uy)` and slide ratio `0..=256`.
    slide_ux: i32,
    slide_uy: i32,
    slide_q8: i32,
}

#[derive(Default)]
struct GiScratch {
    clock: u64,
    /// Two slots so an open Library card preview never evicts the active run's
    /// level field.
    levels: [LevelCache; 2],
    static_gi: StaticGiCache,
    /// Per-pixel art crevice darkness (`w * h`), captured before bump mapping.
    crevice: Vec<u8>,
    crevice_size: Option<(usize, usize)>,
    /// Per-pixel dynamic body contact floor AO (`w * h`) + active row spans.
    body_ao: Vec<u8>,
    row_x0: Vec<i32>,
    row_x1: Vec<i32>,
    /// Grounded creature foot boxes for subtle 1–2 px sprite-base tuck.
    bodies: Vec<GroundedBody>,
    /// GI padded-grid working buffers (reused across frames, zero steady alloc).
    is_wall: Vec<bool>,
    facing: Vec<Vec2>,
    albedo: Vec<Rgb>,
    water: Vec<u8>,
    emit_indoor: Vec<[i32; 3]>,
    emit_sky: Vec<[i32; 3]>,
    emit_dyn: Vec<[i32; 3]>,
    ping: Vec<[i32; 3]>,
    pong: Vec<[i32; 3]>,
    dyn_out: Vec<[u16; 3]>,
    temp_shadow: ShadowProfile,
    rat_shadow: ShadowProfile,
}

thread_local! {
    static STATE: RefCell<GiScratch> = RefCell::new(GiScratch::default());
}

#[inline]
fn soft_union(a: u8, b: u8) -> u8 {
    let (a, b) = (i32::from(a), i32::from(b));
    (a + b - ((a * b) >> 8)).clamp(0, 255) as u8
}

/// Matches `shadow.rs`: on the Flatbed, collision marks the sky outside the
/// deck as `Wall(Solid)` to confine movement, but only the deck, side rails
/// and warren actually exist as geometry.
#[inline]
fn is_real_wall(level: &Level, cx: i32, cy: i32, maze_cell: bool) -> bool {
    if !level.tiles.get(cx, cy).is_wall() {
        return false;
    }
    if level.theme == Theme::Trailer && !level.trailer.is_empty() {
        let deck = level.trailer.deck;
        let rail = (cy == deck.y - 1 || cy == deck.bottom())
            && (deck.x..deck.right()).contains(&cx);
        if !rail && !deck.contains(cx, cy) && !maze_cell {
            return false;
        }
    }
    true
}

/// Normalise a dark pixel-art palette colour into a diffuse bounce reflectance:
/// keeps ~36 % broadband neutral reflectance (so red/green/orange lamps bounce
/// their hue off any wall or floor) and ~64 % of the surface's normalised
/// pigment chromaticity (boosted gently around luma so warm wood and coloured
/// masonry read distinctly).
fn bounce_reflectance(raw: Rgb, chroma_boost: f32, target_peak: i32, neutral_mix_q8: i32) -> Rgb {
    let (r, g, b) = (i32::from(raw.0), i32::from(raw.1), i32::from(raw.2));
    let peak = r.max(g).max(b).max(1);
    let nr = r * target_peak / peak;
    let ng = g * target_peak / peak;
    let nb = b * target_peak / peak;
    let mean = (nr + ng + nb) / 3;
    let boost = (chroma_boost * 256.0) as i32;
    let br = (mean + (((nr - mean) * boost) >> 8)).clamp(10, 255);
    let bg = (mean + (((ng - mean) * boost) >> 8)).clamp(10, 255);
    let bb = (mean + (((nb - mean) * boost) >> 8)).clamp(10, 255);
    let neu = target_peak * 9 / 10;
    let keep = 256 - neutral_mix_q8;
    (
        (((br * keep + neu * neutral_mix_q8) >> 8).clamp(10, 255)) as u8,
        (((bg * keep + neu * neutral_mix_q8) >> 8).clamp(10, 255)) as u8,
        (((bb * keep + neu * neutral_mix_q8) >> 8).clamp(10, 255)) as u8,
    )
}

fn prop_bounce_rgb(kind: PropKind, level: &Level) -> Rgb {
    match kind {
        PropKind::Pallet | PropKind::Table | PropKind::Crate | PropKind::Clothespin => {
            (226, 172, 108)
        }
        PropKind::Counter | PropKind::Washer | PropKind::TowelRoll | PropKind::PaperReel => {
            (222, 224, 228)
        }
        PropKind::Tank => bounce_reflectance(level.palette.accent, 1.25, 224, 64),
        PropKind::Barrel | PropKind::TarpLump => (196, 148, 104),
        PropKind::Tyre | PropKind::PressIsland | PropKind::TypeBlock => (148, 152, 162),
        PropKind::OutfallPiling | PropKind::OutfallBallast | PropKind::OutfallRib => {
            (184, 154, 116)
        }
        _ => bounce_reflectance(level.palette.wall_edge, 1.1, 196, 96),
    }
}

/// Shortest positive ray distance `t in (0, max_t)` from `p` along unit `dir`
/// to axis-aligned `rect` (`0.0` if `p` is inside `rect`, `max_t` if miss).
#[inline]
fn ray_rect_hit(p: Vec2, dir: (f32, f32), rect: Rectf, max_t: f32) -> f32 {
    let ( mut tmin, mut tmax ) = (0.0f32, max_t);
    if dir.0.abs() < 1e-6 {
        if p.x < rect.x || p.x > rect.right() {
            return max_t;
        }
    } else {
        let inv = 1.0 / dir.0;
        let mut t1 = (rect.x - p.x) * inv;
        let mut t2 = (rect.right() - p.x) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return max_t;
        }
    }
    if dir.1.abs() < 1e-6 {
        if p.y < rect.y || p.y > rect.bottom() {
            return max_t;
        }
    } else {
        let inv = 1.0 / dir.1;
        let mut t1 = (rect.y - p.y) * inv;
        let mut t2 = (rect.bottom() - p.y) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return max_t;
        }
    }
    tmin.max(0.0)
}

#[inline]
fn point_rect_dist(p: Vec2, r: Rectf) -> f32 {
    let dx = (r.x - p.x).max(0.0).max(p.x - r.right());
    let dy = (r.y - p.y).max(0.0).max(p.y - r.bottom());
    (dx * dx + dy * dy).sqrt()
}

/// Append the visual/physical wall occluder rectangles of cell `(cx, cy)`.
fn push_cell_wall_rects(
    level: &Level,
    cx: i32,
    cy: i32,
    maze_cells: &[u8],
    cols: i32,
    out: &mut [Rectf; 32],
    len: &mut usize,
) {
    if cx < 0 || cy < 0 || cx >= cols || cy >= level.tiles.rows() || *len >= out.len() {
        return;
    }
    let idx = (cy * cols + cx) as usize;
    if !is_real_wall(level, cx, cy, maze_cells[idx] != 0) {
        return;
    }
    let TileKind::Wall(kind) = level.tiles.get(cx, cy) else {
        return;
    };
    let bounds = TileMap::tile_rect(cx, cy);
    match kind {
        WallKind::Solid => {
            out[*len] = bounds;
            *len += 1;
        }
        WallKind::Maze | WallKind::RatHole { .. } => {
            let mut rects = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
            let n = wall_colliders(&level.tiles.wall_view(cx, cy), &mut rects);
            for r in &rects[..n] {
                if *len < out.len() {
                    out[*len] = *r;
                    *len += 1;
                }
            }
        }
        WallKind::Doorway { axis } => {
            let jb = DOORWAY_JAMB_VISUAL as f32;
            match axis {
                WallAxis::Horizontal => {
                    if *len + 2 <= out.len() {
                        out[*len] = Rectf::new(bounds.x, bounds.y, jb, bounds.h);
                        out[*len + 1] = Rectf::new(bounds.right() - jb, bounds.y, jb, bounds.h);
                        *len += 2;
                    }
                }
                WallAxis::Vertical => {
                    if *len + 2 <= out.len() {
                        out[*len] = Rectf::new(bounds.x, bounds.y, bounds.w, jb);
                        out[*len + 1] = Rectf::new(bounds.x, bounds.bottom() - jb, bounds.w, jb);
                        *len += 2;
                    }
                }
            }
        }
        WallKind::OneWayDoor { .. } => {
            if let Some(dir) = level.tiles.one_way_dir(cx, cy) {
                let closed = level
                    .objects
                    .one_way_doors
                    .iter()
                    .find(|d| d.cell == (cx, cy))
                    .is_none_or(|d| d.open_progress() < 0.5);
                if closed && *len < out.len() {
                    out[*len] = one_way_gate_rect(cx, cy, dir);
                    *len += 1;
                }
            }
        }
    }
}

impl LevelCache {
    fn sync(&mut self, level: &Level, stamp: u64) {
        let key = LevelKey::of(level);
        self.last_used = stamp;
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        let cols = level.tiles.cols().max(0);
        let rows = level.tiles.rows().max(0);
        let n_tiles = (cols * rows) as usize;

        // Mark Maze zones + pits on the tile grid for fast O(1) lookup.
        let mut maze_cells = vec![0u8; n_tiles];
        for z in &level.objects.light_zones {
            if z.kind == LightZoneKind::Maze {
                for (x, y) in z.rect.cells() {
                    if x >= 0 && y >= 0 && x < cols && y < rows {
                        maze_cells[(y * cols + x) as usize] = 1;
                    }
                }
            }
        }
        let mut pit_cells = vec![0u8; n_tiles];
        for pit in level.trailer.pits.iter().chain(level.objects.tidal_pits.iter()) {
            for (x, y) in pit.rect.cells() {
                if x >= 0 && y >= 0 && x < cols && y < rows {
                    pit_cells[(y * cols + x) as usize] = 1;
                }
            }
        }
        let mut wall_tile = vec![0u8; n_tiles];
        for cy in 0..rows {
            for cx in 0..cols {
                let i = (cy * cols + cx) as usize;
                if is_real_wall(level, cx, cy, maze_cells[i] != 0) {
                    wall_tile[i] = 1;
                }
            }
        }

        // ---- 1. Static AO at 2 px world resolution -------------------------
        self.ao_w = (cols as usize) * 8;
        self.ao_h = (rows as usize) * 8;
        self.ao.clear();
        self.ao.resize(self.ao_w * self.ao_h, 0);

        if self.ao_w > 0 && self.ao_h > 0 {
            let mut rects = [Rectf::new(0.0, 0.0, 0.0, 0.0); 32];
            for cy in 0..rows {
                for cx in 0..cols {
                    let ti = (cy * cols + cx) as usize;
                    if pit_cells[ti] != 0 {
                        continue;
                    }
                    let own_solid = wall_tile[ti] != 0
                        && matches!(level.tiles.get(cx, cy), TileKind::Wall(WallKind::Solid));
                    // Check 3×3 neighbourhood for both wall presence and open floor.
                    let mut near_wall = false;
                    let mut near_open = !own_solid;
                    for dy in -1..=1 {
                        let ny = cy + dy;
                        if ny < 0 || ny >= rows {
                            continue;
                        }
                        for dx in -1..=1 {
                            let nx = cx + dx;
                            if nx < 0 || nx >= cols {
                                continue;
                            }
                            if wall_tile[(ny * cols + nx) as usize] != 0 {
                                near_wall = true;
                            } else {
                                near_open = true;
                            }
                        }
                    }
                    if !near_wall || !near_open {
                        continue;
                    }

                    // For solid wall tiles adjacent to open floor, darken only
                    // concave inner room-corner creases on the wall face.
                    if own_solid {
                        let open = |ox: i32, oy: i32| -> bool {
                            let (nx, ny) = (cx + ox, cy + oy);
                            nx >= 0
                                && ny >= 0
                                && nx < cols
                                && ny < rows
                                && wall_tile[(ny * cols + nx) as usize] == 0
                        };
                        // An inner wall corner crease occurs where a wall cell
                        // sits diagonally behind two meeting wall faces, or
                        // along the vertical corner edge of an exposed face.
                        let crease_se = wall_tile.get(((cy + 1).min(rows - 1) * cols + cx) as usize).copied().unwrap_or(1) != 0
                            && wall_tile.get((cy * cols + (cx + 1).min(cols - 1)) as usize).copied().unwrap_or(1) != 0
                            && open(1, 1);
                        let crease_sw = wall_tile.get(((cy + 1).min(rows - 1) * cols + cx) as usize).copied().unwrap_or(1) != 0
                            && wall_tile.get((cy * cols + (cx - 1).max(0)) as usize).copied().unwrap_or(1) != 0
                            && open(-1, 1);
                        // Also check exposed south face adjacent to a perpendicular east/west wall.
                        let south_open = open(0, 1);
                        let inner_w = south_open
                            && cx > 0
                            && wall_tile[(cy * cols + cx - 1) as usize] != 0
                            && cy + 1 < rows
                            && wall_tile[((cy + 1) * cols + cx - 1) as usize] != 0;
                        let inner_e = south_open
                            && cx + 1 < cols
                            && wall_tile[(cy * cols + cx + 1) as usize] != 0
                            && cy + 1 < rows
                            && wall_tile[((cy + 1) * cols + cx + 1) as usize] != 0;
                        if crease_se || crease_sw || inner_w || inner_e {
                            for sy in 0..8usize {
                                let ly = sy as f32 * 2.0 + 1.0;
                                for sx in 0..8usize {
                                    let lx = sx as f32 * 2.0 + 1.0;
                                    let mut d_min = 9.0f32;
                                    if inner_w {
                                        d_min = d_min.min(lx);
                                    }
                                    if inner_e {
                                        d_min = d_min.min(16.0 - lx);
                                    }
                                    if crease_se {
                                        d_min = d_min.min(((16.0 - lx).hypot(16.0 - ly)).min(9.0));
                                    }
                                    if crease_sw {
                                        d_min = d_min.min((lx.hypot(16.0 - ly)).min(9.0));
                                    }
                                    if d_min < 7.0 {
                                        let u = 1.0 - d_min / 7.0;
                                        let v = (u * u * 38.0).round() as u8;
                                        let idx = (cy as usize * 8 + sy) * self.ao_w
                                            + (cx as usize * 8 + sx);
                                        self.ao[idx] = self.ao[idx].max(v);
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    // Open floor (or partial wall cell): gather 3×3 wall rects.
                    let mut n_rects = 0usize;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            push_cell_wall_rects(
                                level,
                                cx + dx,
                                cy + dy,
                                &maze_cells,
                                cols,
                                &mut rects,
                                &mut n_rects,
                            );
                        }
                    }
                    if n_rects == 0 {
                        continue;
                    }
                    let slice = &rects[..n_rects];
                    for sy in 0..8usize {
                        let wy = (cy * TILE) as f32 + sy as f32 * 2.0 + 1.0;
                        let ay = cy as usize * 8 + sy;
                        for sx in 0..8usize {
                            let wx = (cx * TILE) as f32 + sx as f32 * 2.0 + 1.0;
                            let p = Vec2::new(wx, wy);
                            if slice.iter().any(|r| r.contains(p)) {
                                continue;
                            }
                            let mut occ = 0.0f32;
                            for &dir in &DIRS_8 {
                                let mut best = WALL_AO_RADIUS;
                                for &r in slice {
                                    let t = ray_rect_hit(p, dir, r, best);
                                    if t < best {
                                        best = t;
                                    }
                                }
                                if best < WALL_AO_RADIUS {
                                    let u = 1.0 - best / WALL_AO_RADIUS;
                                    let mut w = u * u;
                                    // `art_walls` already paints a 4 px south & 3 px east
                                    // contact shadow: soften the inner 4 px on N/W rays so
                                    // AO extends that shadow smoothly instead of doubling it.
                                    if (dir.1 < -0.5 || dir.0 < -0.5) && best < 4.5 {
                                        w *= 0.66;
                                    }
                                    occ += w;
                                }
                            }
                            occ *= 1.0 / 8.0;
                            if occ > 0.0 {
                                // Quadratic synergy makes concave L-corners and narrow
                                // alcoves ~1.8× deeper than straight wall faces.
                                let shaped = occ * 0.96 + occ * occ * 1.05;
                                let val = (shaped * 132.0).round().clamp(0.0, f32::from(STATIC_AO_CAP)) as u8;
                                let idx = ay * self.ao_w + (cx as usize * 8 + sx);
                                self.ao[idx] = self.ao[idx].max(val);
                            }
                        }
                    }
                }
            }

            // Static prop skirts scaled by material height class.
            for prop in &level.props {
                if prop.kind == PropKind::ChainCoil {
                    continue;
                }
                let h_class = material::height(specular::prop_pen(prop.kind)).max(1);
                let grate_scale: f32 = if prop.kind == PropKind::Grate { 0.35 } else { 1.0 };
                let radius = (4.0 + f32::from(h_class) * 1.2) * grate_scale.max(0.6);
                let peak = ((16.0 + f32::from(h_class) * 9.5) * grate_scale).round();
                let ax0 = (((prop.rect.x - radius) * 0.5).floor() as i32).clamp(0, self.ao_w as i32 - 1) as usize;
                let ay0 = (((prop.rect.y - radius) * 0.5).floor() as i32).clamp(0, self.ao_h as i32 - 1) as usize;
                let ax1 = (((prop.rect.right() + radius) * 0.5).ceil() as i32).clamp(0, self.ao_w as i32 - 1) as usize;
                let ay1 = (((prop.rect.bottom() + radius) * 0.5).ceil() as i32).clamp(0, self.ao_h as i32 - 1) as usize;
                for ay in ay0..=ay1 {
                    let wy = ay as f32 * 2.0 + 1.0;
                    for ax in ax0..=ax1 {
                        let wx = ax as f32 * 2.0 + 1.0;
                        let p = Vec2::new(wx, wy);
                        let cx = (wx as i32) >> 4;
                        let cy = (wy as i32) >> 4;
                        if cx >= 0
                            && cy >= 0
                            && cx < cols
                            && cy < rows
                            && pit_cells[(cy * cols + cx) as usize] != 0
                        {
                            continue;
                        }
                        let d = point_rect_dist(p, prop.rect);
                        if d >= radius {
                            continue;
                        }
                        // Soft skirt outside the footprint + a gentle rim inside
                        // the prop's outer 3 px base edge.
                        let v = if d > 0.0 {
                            let u = 1.0 - d / radius;
                            (peak * u * u).round() as u8
                        } else {
                            let edge_d = (wx - prop.rect.x)
                                .min(prop.rect.right() - wx)
                                .min(prop.rect.bottom() - wy);
                            if edge_d < 3.0 {
                                ((peak * 0.45) * (1.0 - edge_d / 3.0)).round() as u8
                            } else {
                                0
                            }
                        };
                        if v > 0 {
                            let idx = ay * self.ao_w + ax;
                            self.ao[idx] = soft_union(self.ao[idx], v).min(STATIC_AO_CAP);
                        }
                    }
                }
            }
        }

        // ---- 2. Static Albedo at 4 px subcell resolution -------------------
        self.alb_w = (cols as usize) * 4;
        self.alb_h = (rows as usize) * 4;
        let n_alb = self.alb_w * self.alb_h;
        self.albedo.clear();
        self.water_sub.clear();

        let floor_raw = palette::mix(level.palette.bg, level.palette.bg_alt, 0.55);
        let floor_alb = match level.theme {
            // Sunlit weathered oak planks bounce rich warm amber-ochre.
            Theme::Trailer => (240, 174, 100),
            // Dry seabed sand is warm-neutral stone before underwater grading.
            Theme::Outfall => (186, 194, 184),
            _ => bounce_reflectance(floor_raw, 1.40, 206, 28),
        };
        let wall_raw = palette::mix(level.palette.wall, level.palette.wall_edge, 0.5);
        let wall_alb = bounce_reflectance(wall_raw, 1.45, 222, 24);

        self.albedo.resize(n_alb, floor_alb);
        self.water_sub.resize(n_alb, 0);

        for cy in 0..rows {
            for cx in 0..cols {
                let ti = (cy * cols + cx) as usize;
                if wall_tile[ti] != 0 {
                    for sy in 0..4usize {
                        let gy = cy as usize * 4 + sy;
                        let row_off = gy * self.alb_w + cx as usize * 4;
                        for sx in 0..4usize {
                            self.albedo[row_off + sx] = wall_alb;
                        }
                    }
                } else if pit_cells[ti] != 0 {
                    for sy in 0..4usize {
                        let gy = cy as usize * 4 + sy;
                        let row_off = gy * self.alb_w + cx as usize * 4;
                        for sx in 0..4usize {
                            self.albedo[row_off + sx] = (0, 0, 0);
                        }
                    }
                }
            }
        }

        // Standing water puddles: Outfall water bounces vivid marine cyan;
        // indoor wet floors shift toward the floor's wet accent.
        let water_alb = match level.theme {
            Theme::Outfall => (42, 240, 255),
            _ => palette::mix(
                floor_alb,
                bounce_reflectance(level.palette.accent, 1.3, 222, 24),
                0.45,
            ),
        };
        for puddle in &level.puddles {
            let gx0 = (((puddle.pos.x - puddle.rx) * 0.25).floor() as i32)
                .clamp(0, self.alb_w as i32 - 1) as usize;
            let gy0 = (((puddle.pos.y - puddle.ry) * 0.25).floor() as i32)
                .clamp(0, self.alb_h as i32 - 1) as usize;
            let gx1 = (((puddle.pos.x + puddle.rx) * 0.25).ceil() as i32)
                .clamp(0, self.alb_w as i32 - 1) as usize;
            let gy1 = (((puddle.pos.y + puddle.ry) * 0.25).ceil() as i32)
                .clamp(0, self.alb_h as i32 - 1) as usize;
            for gy in gy0..=gy1 {
                let wy = gy as f32 * 4.0 + 2.0;
                for gx in gx0..=gx1 {
                    let wx = gx as f32 * 4.0 + 2.0;
                    let cx = (wx as i32) >> 4;
                    let cy = (wy as i32) >> 4;
                    if cx >= 0
                        && cy >= 0
                        && cx < cols
                        && cy < rows
                        && wall_tile[(cy * cols + cx) as usize] == 0
                        && puddle.contains(Vec2::new(wx, wy))
                    {
                        let idx = gy * self.alb_w + gx;
                        self.albedo[idx] = water_alb;
                        if level.theme == Theme::Outfall {
                            self.water_sub[idx] = 1;
                        }
                    }
                }
            }
        }

        // Clothes piles & props.
        for pile in &level.clothes_piles {
            let cloth_rgb = bounce_reflectance(level.palette.accent, 1.15, 214, 90);
            let gx0 = ((pile.pos.x - 10.0) * 0.25).floor().max(0.0) as usize;
            let gy0 = ((pile.pos.y - 8.0) * 0.25).floor().max(0.0) as usize;
            let gx1 = (((pile.pos.x + 10.0) * 0.25).ceil() as usize).min(self.alb_w.saturating_sub(1));
            let gy1 = (((pile.pos.y + 8.0) * 0.25).ceil() as usize).min(self.alb_h.saturating_sub(1));
            for gy in gy0..=gy1 {
                for gx in gx0..=gx1 {
                    self.albedo[gy * self.alb_w + gx] = cloth_rgb;
                }
            }
        }
        for prop in &level.props {
            let rgb = prop_bounce_rgb(prop.kind, level);
            let gx0 = ((prop.rect.x * 0.25).floor() as i32).clamp(0, self.alb_w as i32 - 1) as usize;
            let gy0 = ((prop.rect.y * 0.25).floor() as i32).clamp(0, self.alb_h as i32 - 1) as usize;
            let gx1 = ((prop.rect.right() * 0.25).ceil() as i32).clamp(0, self.alb_w as i32 - 1) as usize;
            let gy1 = ((prop.rect.bottom() * 0.25).ceil() as i32).clamp(0, self.alb_h as i32 - 1) as usize;
            for gy in gy0..=gy1 {
                for gx in gx0..=gx1 {
                    self.albedo[gy * self.alb_w + gx] = rgb;
                }
            }
        }
    }

    /// Bilinear lookup of static AO at integer world pixel `(wx, wy)`.
    #[inline]
    fn sample_ao(&self, wx: i32, wy: i32) -> u8 {
        if self.ao_w == 0 || self.ao_h == 0 {
            return 0;
        }
        // Sample centres sit at `2 * a + 1`. Shift by -1 so `t = wx - 1`
        // aligns cleanly with 2 px intervals.
        let tx = wx - 1;
        let ty = wy - 1;
        let ax0 = (tx >> 1).clamp(0, self.ao_w as i32 - 1) as usize;
        let ay0 = (ty >> 1).clamp(0, self.ao_h as i32 - 1) as usize;
        let ax1 = (ax0 + 1).min(self.ao_w - 1);
        let ay1 = (ay0 + 1).min(self.ao_h - 1);
        let fx = (tx & 1) as u32;
        let fy = (ty & 1) as u32;
        let r0 = ay0 * self.ao_w;
        let r1 = ay1 * self.ao_w;
        let top = if fx == 0 {
            u32::from(self.ao[r0 + ax0]) * 2
        } else {
            u32::from(self.ao[r0 + ax0]) + u32::from(self.ao[r0 + ax1])
        };
        let bot = if fx == 0 {
            u32::from(self.ao[r1 + ax0]) * 2
        } else {
            u32::from(self.ao[r1 + ax0]) + u32::from(self.ao[r1 + ax1])
        };
        let v = if fy == 0 { top * 2 } else { top + bot };
        ((v + 2) >> 2) as u8
    }

    #[inline]
    fn sample_albedo(&self, p: Vec2) -> (Rgb, bool) {
        if self.alb_w == 0 || self.alb_h == 0 {
            return ((180, 180, 180), false);
        }
        let gx = ((p.x * 0.25).floor() as i32).clamp(0, self.alb_w as i32 - 1) as usize;
        let gy = ((p.y * 0.25).floor() as i32).clamp(0, self.alb_h as i32 - 1) as usize;
        let idx = gy * self.alb_w + gx;
        (self.albedo[idx], self.water_sub[idx] != 0)
    }
}

fn select_level_slot(levels: &mut [LevelCache; 2], level: &Level, clock: &mut u64) -> usize {
    *clock = clock.wrapping_add(1);
    let stamp = *clock;
    let key = LevelKey::of(level);
    if levels[0].key == Some(key) {
        levels[0].last_used = stamp;
        return 0;
    }
    if levels[1].key == Some(key) {
        levels[1].last_used = stamp;
        return 1;
    }
    let victim = usize::from(levels[1].last_used < levels[0].last_used);
    levels[victim].sync(level, stamp);
    victim
}

// ============================================================================
// Stage A: Pre-bump unlit art crevice capture + Post-overhead AO application
// ============================================================================

#[inline]
fn luma((r, g, b): Rgb) -> i32 {
    (i32::from(r) * 77 + i32::from(g) * 150 + i32::from(b) * 29) >> 8
}

/// Read mortar lines, masonry joints and recessed panel seams from the
/// **unlit** art before `surface::apply` embosses luminance flanks.
pub(super) fn capture_crevices(grid: &Grid, canvas: &Canvas<'_>, frame: &FxFrame<'_>, gi: GlobalLight) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.crevice_size = None;
        if !gi.ao_enabled(frame.mode) {
            return;
        }
        let (w, h) = (canvas.w, canvas.h);
        if w < 3 || h < 3 || grid.w == 0 || grid.h == 0 || grid.stride == 0 || canvas.px.len() < w * h {
            return;
        }
        s.crevice.clear();
        s.crevice.resize(w * h, 0);
        let px = &*canvas.px;
        let mat = canvas.mat;
        let stride = grid.stride;
        let (ox, oy) = grid.offset;

        for y in 0..h {
            let ym1 = y.saturating_sub(1) * w;
            let y0 = y * w;
            let yp1 = (y + 1).min(h - 1) * w;
            let srow = ((y + oy) / stride).min(grid.h - 1) * grid.w;
            for x in 0..w {
                let i = y0 + x;
                let m = mat.get(i).copied().unwrap_or(0);
                if material::is_creature(m) || material::kind(m) == MaterialKind::Emissive {
                    continue;
                }
                let si = srow + ((x + ox) / stride).min(grid.w - 1);
                let is_wall = grid
                    .surfaces
                    .get(si)
                    .is_some_and(|surf| surf.layer & LayerMask::WALLS.0 != 0);
                let prop_h = material::height(m);
                if !is_wall && prop_h == 0 {
                    continue;
                }
                let c = luma(px[i]);
                let xm1 = x.saturating_sub(1);
                let xp1 = (x + 1).min(w - 1);
                let mean4 = (luma(px[y0 + xm1]) + luma(px[y0 + xp1]) + luma(px[ym1 + x]) + luma(px[yp1 + x])) >> 2;
                let cavity = mean4 - c;
                if cavity <= CREVICE_NOISE {
                    continue;
                }
                let scale = if is_wall { 22 } else { 12 + i32::from(prop_h) * 2 };
                let ao = (((cavity - CREVICE_NOISE) * scale) >> 3).clamp(0, i32::from(CREVICE_MAX)) as u8;
                s.crevice[i] = ao;
            }
        }
        s.crevice_size = Some((w, h));
    });
}

fn gather_grounded_bodies(
    out: &mut Vec<GroundedBody>,
    scene: &LightScene<'_>,
    cam: (i32, i32),
    (w, h): (i32, i32),
) {
    out.clear();
    let centre = Vec2::new(
        scene.level.tiles.cols() as f32 * TILE_F * 0.5,
        scene.level.tiles.rows() as f32 * TILE_F * 0.5,
    );
    let push = |out: &mut Vec<GroundedBody>, pos: Vec2, half: i32, height: f32, spid: u64| {
        let sx = pos.x.round() as i32 - cam.0;
        let sy = pos.y.round() as i32 - cam.1;
        let r = half + 5;
        if sx + r < 0 || sy + r < 0 || sx - r >= w || sy - r >= h {
            return;
        }
        let raw_slide = (pos - centre) * (height / 900.0);
        let max_slide = (half as f32 * 0.55 + 2.0).min(22.0);
        let slide = raw_slide.clamp_len(max_slide);
        let len = slide.len();
        let (ux, uy) = if len > 0.25 {
            ((slide.x / len * 256.0).round() as i32, (slide.y / len * 256.0).round() as i32)
        } else {
            (0, 0)
        };
        let slide_q8 = ((len / max_slide.max(1.0)) * 256.0).round().clamp(0.0, 256.0) as i32;
        out.push(GroundedBody {
            spid,
            d2: pos.dist_sq(scene.player.pos),
            sx,
            sy,
            half: half.max(3),
            slide_ux: ux,
            slide_uy: uy,
            slide_q8,
        });
    };

    // Player rat (on Outfall only when grounded inside a crab pot; swimming
    // diver floats above the seabed).
    let rat_swims = scene.level.theme == Theme::Outfall && !scene.level.objects.rat_in_crab_pot();
    if !rat_swims {
        let scale = scene.player.buffs.draw_scale();
        let half = ((5.0 * scale).round() as i32).clamp(3, 22);
        push(out, scene.player.pos, half, 9.0 * scale, 0);
    }

    for entity in scene.ecs.iter_entities() {
        let spid = entity.get::<SpawnId>().map_or(u64::MAX, |id| id.0);
        if let Some(boss) = entity.get::<Boss>() {
            if boss.dead || boss.kind == BossKind::Lantern {
                continue;
            }
            let flying = boss.is_airborne()
                || boss.overhead()
                || matches!(boss.state, BossState::Magpie(st) if st.flying());
            if !flying {
                push(out, boss.pos, 13, 26.0, spid);
            }
        } else if let Some(e) = entity.get::<Enemy>() {
            if !e.alive() {
                continue;
            }
            let arch = e.kind.archetype();
            let flyer = art_roster_outfall::is_swimmer(e.kind)
                || matches!(arch, EnemyKind::Bat | EnemyKind::MurderHornet | EnemyKind::WickMoth);
            if flyer {
                continue;
            }
            let half = if e.named {
                10
            } else {
                ((e.radius * 1.35 + 1.5) as i32).clamp(3, 13)
            };
            let height = if e.named { 18.0 } else { (e.radius * 2.0).clamp(6.0, 14.0) };
            push(out, e.pos, half, height, spid);
        }
    }

    out.sort_unstable_by_key(|b| b.spid);
    if out.len() > MAX_CONTACT_BODIES {
        out.sort_unstable_by(|a, b| a.d2.total_cmp(&b.d2).then(a.spid.cmp(&b.spid)));
        out.truncate(MAX_CONTACT_BODIES);
    }
}

/// Apply static wall/corner/prop AO, art crevices, pixel-exact prop skirts,
/// and grounded body contact AO to the lit canvas.
pub(super) fn apply_ao(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    scene: &LightScene<'_>,
    frame: &FxFrame<'_>,
) {
    if !scene.gi.ao_enabled(frame.mode) {
        return;
    }
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 || grid.w == 0 || grid.h == 0 || grid.stride == 0 || canvas.px.len() < w * h {
        return;
    }

    STATE.with(|cell| {
        let mut guard = cell.borrow_mut();
        let s = &mut *guard;
        let slot = select_level_slot(&mut s.levels, scene.level, &mut s.clock);
        let cam = (frame.cam.x as i32, frame.cam.y as i32);
        let (bw, bh) = (w as i32, h as i32);

        // Rasterise tight foot-contact halos for grounded bodies.
        gather_grounded_bodies(&mut s.bodies, scene, cam, (bw, bh));
        s.body_ao.clear();
        s.body_ao.resize(w * h, 0);
        s.row_x0.clear();
        s.row_x0.resize(h, i32::MAX);
        s.row_x1.clear();
        s.row_x1.resize(h, i32::MIN);

        for b in &s.bodies {
            let rx = b.half + 3;
            let ry = (b.half * 3 / 4 + 2).max(3);
            // Foot line sits slightly towards the bottom of the sprite.
            let cy = b.sy + (b.half / 3).max(1);
            let x0 = (b.sx - rx).max(0);
            let x1 = (b.sx + rx).min(bw - 1);
            let y0 = (cy - ry).max(0);
            let y1 = (cy + ry).min(bh - 1);
            let rx2 = (rx * rx).max(1);
            let ry2 = (ry * ry).max(1);
            for py in y0..=y1 {
                let dy = py - cy;
                for px in x0..=x1 {
                    let dx = px - b.sx;
                    // Normalized squared ellipse distance in Q8 (`256` = edge).
                    let q8 = (dx * dx * 256) / rx2 + (dy * dy * 256) / ry2;
                    if q8 >= 256 {
                        continue;
                    }
                    let fall = ((256 - q8) * (256 - q8)) >> 8; // 0..=256
                    // Complement `overhead`: where `overhead` slid away, keep
                    // full contact AO on the exposed foot side; where `overhead`
                    // already sits (`dot > 0` or `slide_q8 == 0`), attenuate so
                    // the two never double up.
                    let dot_slide = if b.slide_q8 > 16 {
                        let dlen = ((dx * dx + dy * dy).max(1) as u32).isqrt() as i32;
                        ((dx * b.slide_ux + dy * b.slide_uy) / dlen.max(1)).clamp(-256, 256)
                    } else {
                        128
                    };
                    // `anti_overlap` in 72..=256: ~28 % at map-centre / shadow
                    // side, rising to 100 % on the exposed contact rim.
                    let base_expose = 72 + ((b.slide_q8 * 112) >> 8);
                    let side_expose = ((-dot_slide) * b.slide_q8) >> 9;
                    let expose = (base_expose + side_expose).clamp(64, 256);
                    let ao = (((fall * BODY_FLOOR_AO_MAX) >> 8) * expose) >> 8;
                    if ao > 0 {
                        let idx = py as usize * w + px as usize;
                        let v = ao.min(255) as u8;
                        if v > s.body_ao[idx] {
                            s.body_ao[idx] = v;
                            let row = py as usize;
                            if px < s.row_x0[row] {
                                s.row_x0[row] = px;
                            }
                            if px > s.row_x1[row] {
                                s.row_x1[row] = px;
                            }
                        }
                    }
                }
            }
        }

        let has_crevice = s.crevice_size == Some((w, h)) && s.crevice.len() >= w * h;
        let has_mat = canvas.mat.len() >= w * h;
        let has_info = grid.info.len() >= grid.w * grid.h;
        let stride = grid.stride;
        let (ox, oy) = grid.offset;
        let lvl = &s.levels[slot];

        for y in 0..h {
            let wy = cam.1 + y as i32;
            let srow = ((y + oy) / stride).min(grid.h - 1) * grid.w;
            let row_off = y * w;
            let body_active = y < s.row_x0.len() && s.row_x0[y] <= s.row_x1[y];

            for x in 0..w {
                let i = row_off + x;
                let m = if has_mat { canvas.mat[i] } else { 0 };
                if material::kind(m) == MaterialKind::Emissive {
                    continue;
                }
                let si = srow + ((x + ox) / stride).min(grid.w - 1);
                let is_wall = grid
                    .surfaces
                    .get(si)
                    .is_some_and(|surf| surf.layer & LayerMask::WALLS.0 != 0);

                let raw_ao: i32 = if material::is_creature(m) {
                    // Grounded creature foot tuck: only on the lowest 1–2 rows
                    // of a grounded body where the pixel below is open floor.
                    if !has_mat || y + 1 >= h || !body_active {
                        continue;
                    }
                    let xx = x as i32;
                    if xx < s.row_x0[y] || xx > s.row_x1[y] || s.body_ao[i] < 8 {
                        continue;
                    }
                    let below1 = canvas.mat[(y + 1) * w + x];
                    let edge = if !material::is_creature(below1) {
                        BODY_SPRITE_TUCK
                    } else if y + 2 < h && !material::is_creature(canvas.mat[(y + 2) * w + x]) {
                        BODY_SPRITE_TUCK / 2
                    } else {
                        0
                    };
                    if edge == 0 {
                        continue;
                    }
                    edge
                } else {
                    let wx = cam.0 + x as i32;
                    // Share horizontal pairs on open untagged floor so shallow
                    // wall-distance ramps compress cleanly in the SGR encoder.
                    let prop_h = material::height(m);
                    let sample_wx = if !is_wall && prop_h == 0 { wx & !1 } else { wx };
                    let s_ao = lvl.sample_ao(sample_wx, wy);

                    let c_ao = if has_crevice { s.crevice[i] } else { 0 };

                    // Pixel-exact contact skirt around any pen-tagged raised prop.
                    let mut p_skirt = 0u8;
                    if has_mat && !is_wall && prop_h == 0 {
                        let mut max_h = 0u8;
                        if x > 0 {
                            let nb = canvas.mat[i - 1];
                            if !material::is_creature(nb) {
                                max_h = max_h.max(material::height(nb));
                            }
                        }
                        if x + 1 < w {
                            let nb = canvas.mat[i + 1];
                            if !material::is_creature(nb) {
                                max_h = max_h.max(material::height(nb));
                            }
                        }
                        if y > 0 {
                            let nb = canvas.mat[i - w];
                            if !material::is_creature(nb) {
                                max_h = max_h.max(material::height(nb));
                            }
                        }
                        if y + 1 < h {
                            let nb = canvas.mat[i + w];
                            if !material::is_creature(nb) {
                                max_h = max_h.max(material::height(nb));
                            }
                        }
                        if max_h > 0 {
                            p_skirt = (max_h * 7 + 8).min(56);
                        }
                    }

                    let b_ao = if !is_wall && prop_h == 0 && body_active {
                        s.body_ao[i]
                    } else {
                        0
                    };

                    let local = c_ao.max(p_skirt).max(b_ao);
                    i32::from(soft_union(s_ao, local)).min(TOTAL_AO_CAP)
                };

                if raw_ao < AO_QUANT_STEP {
                    continue;
                }

                // Direct-light relief: strong direct illumination partially
                // fills micro/corner occlusion so lamps read crisp rather than
                // muddy, while ambient shadow keeps full AO depth.
                let dom = if has_info { i32::from(grid.info[si].dom) } else { 0 };
                let relief_q8 = 256 - ((dom.min(320) * 100) / 320);
                let d = (((raw_ao * relief_q8) >> 8) / AO_QUANT_STEP) * AO_QUANT_STEP;
                if d < AO_QUANT_STEP {
                    continue;
                }

                let p = canvas.px[i];
                if (p.0 | p.1 | p.2) == 0 {
                    continue;
                }
                let keep = (255 - d) as u32;
                canvas.px[i] = (
                    ((u32::from(p.0) * keep + 127) / 255) as u8,
                    ((u32::from(p.1) * keep + 127) / 255) as u8,
                    ((u32::from(p.2) * keep + 127) / 255) as u8,
                );
            }
        }
    });
}

// ============================================================================
// Stage B: One-bounce coloured Global Illumination on the 4 px light grid
// ============================================================================

#[inline]
fn snuffed_signature(level: &Level) -> (usize, u64) {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut count = 0usize;
    for (i, f) in level.objects.lights.iter().enumerate() {
        if f.kind == LightFixtureKind::Sconce && level.sconce_snuffed(i) {
            count += 1;
            h = (h ^ (i as u64)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    (count, h)
}

/// Inject steady incident RGB energy `(ir, ig, ib)` (`Q8.8`) arriving at padded
/// cell `(x, y)` from direction `from_pos`: floor cells re-emit in place; lit
/// wall faces inject into adjacent front-facing open floor cells.
#[allow(clippy::too_many_arguments)]
#[inline]
fn inject_surface_bounce(
    emit: &mut [[i32; 3]],
    is_wall: &[bool],
    facing: &[Vec2],
    albedo: &[Rgb],
    (pw, ph): (usize, usize),
    (x, y): (usize, usize),
    inc: [i32; 3],
) {
    let i = y * pw + x;
    let alb = albedo[i];
    if alb == (0, 0, 0) {
        return;
    }
    let bounced = [
        (inc[0] * i32::from(alb.0)) >> 8,
        (inc[1] * i32::from(alb.1)) >> 8,
        (inc[2] * i32::from(alb.2)) >> 8,
    ];
    if (bounced[0] | bounced[1] | bounced[2]) == 0 {
        return;
    }
    if !is_wall[i] {
        emit[i][0] += bounced[0];
        emit[i][1] += bounced[1];
        emit[i][2] += bounced[2];
        return;
    }

    // Wall face -> adjacent front-facing open floor cells.
    let wf = facing[i];
    let mut targets = [0usize; 8];
    let mut count = 0usize;
    for dy in -1i32..=1 {
        let ny = y as i32 + dy;
        if ny < 0 || ny >= ph as i32 {
            continue;
        }
        for dx in -1i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = x as i32 + dx;
            if nx < 0 || nx >= pw as i32 {
                continue;
            }
            let nb = ny as usize * pw + nx as usize;
            if is_wall[nb] {
                continue;
            }
            if wf != Vec2::ZERO && (dx as f32 * wf.x + dy as f32 * wf.y) < -0.05 {
                continue;
            }
            targets[count] = nb;
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    // Boost wall-face re-emission slightly because a vertical wall face
    // intercepts horizontal lamp rays squarely and reflects onto its foot.
    let share = (320 / count as i32).max(1);
    for &nb in &targets[..count] {
        emit[nb][0] += (bounced[0] * share) >> 8;
        emit[nb][1] += (bounced[1] * share) >> 8;
        emit[nb][2] += (bounced[2] * share) >> 8;
    }
}

/// Splat one steady (unflickered) point/cone/beam light onto the padded 4 px
/// GI emission buffer. When `trace_walls_without_profile` is true (ground
/// sources whose direct pass has `casts_shadow = false`), traces wall
/// occlusion on `geometry` so GI never seeds across a wall!
#[allow(clippy::too_many_arguments)]
fn splat_steady_light(
    light: Light,
    shadow: Option<&ShadowProfile>,
    trace_walls_without_profile: bool,
    geometry: &Geometry,
    pstart: Vec2,
    (pw, ph): (usize, usize),
    is_wall: &[bool],
    facing: &[Vec2],
    albedo: &[Rgb],
    emit: &mut [[i32; 3]],
) {
    let radius = light.extent();
    if !radius.is_finite() || radius <= 1.0 || light.energy <= 0.001 {
        return;
    }
    let rel = light.pos - pstart;
    let lo = |v: f32| ((v - radius) * 0.25).ceil().max(0.0) as usize;
    let hi = |v: f32, lim: usize| ((((v + radius) * 0.25).floor() as i32 + 1).clamp(0, lim as i32)) as usize;
    let xs = lo(rel.x)..hi(rel.x, pw);
    let ys = lo(rel.y)..hi(rel.y, ph);
    if xs.is_empty() || ys.is_empty() {
        return;
    }
    let cone_limit = match light.shape {
        LightShape::Cone { half_angle, .. } => half_angle.cos(),
        _ => 0.0,
    };
    let curve = Curve::new(&light);
    let min_amount = curve.step();
    if min_amount > 1.0 {
        return;
    }
    let energy_total = light.energy + light.scatter * 0.65;
    let col = [i32::from(light.color.0), i32::from(light.color.1), i32::from(light.color.2)];

    for y in ys {
        let wy = pstart.y + (y * 4) as f32;
        for x in xs.clone() {
            let p = Vec2::new(pstart.x + (x * 4) as f32, wy);
            let delta = p - light.pos;
            let (core, tail) = curve.parts(delta, delta.len_sq(), cone_limit);
            let tail = curve.quantise(tail);
            let w = core + tail;
            if w < min_amount {
                continue;
            }
            let idx = y * pw + x;
            let wall = is_wall[idx];
            let surf = geometry.surface(p);
            let vis = if let Some(prof) = shadow {
                prof.visibility(delta, surf, super::ShadowSoftness::Soft)
            } else if trace_walls_without_profile {
                let linf = delta.x.abs().max(delta.y.abs());
                if linf <= 2.5 {
                    1.0
                } else {
                    let limit = if wall { (linf - 4.5).max(0.5) } else { linf - 1.0 };
                    let hit = geometry.trace(
                        light.pos,
                        delta * (1.0 / linf),
                        limit,
                        |_, h| h >= 8.0,
                        LayerMask::WALLS,
                    );
                    if hit.is_finite() { 0.0 } else { 1.0 }
                }
            } else {
                1.0
            };
            if vis <= 0.02 {
                continue;
            }
            let face_gain = if wall && facing[idx] != Vec2::ZERO {
                let toward = -delta;
                let denom = toward.x.abs().max(toward.y.abs()).max(0.01);
                let f = (facing[idx].dot(toward) / denom).clamp(-1.0, 1.0);
                if f < -0.25 {
                    0.0
                } else {
                    0.75 + 0.25 * f
                }
            } else {
                1.0
            };
            let strength = w * vis * face_gain * energy_total;
            if strength <= 0.002 {
                continue;
            }
            let s_q8 = (strength * 256.0) as i32;
            let inc = [
                (col[0] * s_q8) >> 8,
                (col[1] * s_q8) >> 8,
                (col[2] * s_q8) >> 8,
            ];
            inject_surface_bounce(emit, is_wall, facing, albedo, (pw, ph), (x, y), inc);
        }
    }
}

/// Propagate floor radiosity `emit` across `!is_wall` cells (stopping at all
/// walls), then gather indirect floor light onto adjacent front-facing wall
/// cells. Writes `Q8.8` indirect bounce into `out`.
#[allow(clippy::too_many_arguments)]
fn propagate_gi(
    emit: &[[i32; 3]],
    is_wall: &[bool],
    facing: &[Vec2],
    albedo: &[Rgb],
    water: Option<&[u8]>,
    (pw, ph): (usize, usize),
    bounds: (usize, usize, usize, usize),
    ping: &mut Vec<[i32; 3]>,
    pong: &mut Vec<[i32; 3]>,
    out: &mut Vec<[u16; 3]>,
) -> bool {
    let n = pw * ph;
    let (bx0, by0, bx1, by1) = bounds;
    if bx0 > bx1 || by0 > by1 {
        out.clear();
        out.resize(n, [0; 3]);
        return false;
    }

    ping.clear();
    ping.extend_from_slice(&emit[..n]);
    pong.clear();
    pong.resize(n, [0; 3]);

    // Expand the active box as iterations spread energy outward.
    let margin = GI_COARSE_ITERS * 2 + GI_FINE_ITERS + 1;
    let x0 = bx0.saturating_sub(margin);
    let y0 = by0.saturating_sub(margin);
    let x1 = (bx1 + margin).min(pw - 1);
    let y1 = (by1 + margin).min(ph - 1);

    // Coarse midpoint-checked steps (`step = 2`) + fine corner-checked steps (`step = 1`).
    for pass in 0..(GI_COARSE_ITERS + GI_FINE_ITERS) {
        let step = if pass < GI_COARSE_ITERS { 2usize } else { 1usize };
        let (src, dst) = if pass % 2 == 0 {
            (&ping[..], &mut pong[..])
        } else {
            (&pong[..], &mut ping[..])
        };

        for y in y0..=y1 {
            let row = y * pw;
            for x in x0..=x1 {
                let i = row + x;
                if is_wall[i] {
                    dst[i] = [0; 3];
                    continue;
                }
                let mut acc = [0i32; 3];
                // 4 cardinal taps (weight 3 each = 12).
                if x >= step {
                    let ok = (step == 1 || !is_wall[i - 1]) && !is_wall[i - step];
                    if ok {
                        let v = src[i - step];
                        acc[0] += v[0] * 3;
                        acc[1] += v[1] * 3;
                        acc[2] += v[2] * 3;
                    }
                }
                if x + step < pw {
                    let ok = (step == 1 || !is_wall[i + 1]) && !is_wall[i + step];
                    if ok {
                        let v = src[i + step];
                        acc[0] += v[0] * 3;
                        acc[1] += v[1] * 3;
                        acc[2] += v[2] * 3;
                    }
                }
                if y >= step {
                    let ok = (step == 1 || !is_wall[i - pw]) && !is_wall[i - step * pw];
                    if ok {
                        let v = src[i - step * pw];
                        acc[0] += v[0] * 3;
                        acc[1] += v[1] * 3;
                        acc[2] += v[2] * 3;
                    }
                }
                if y + step < ph {
                    let ok = (step == 1 || !is_wall[i + pw]) && !is_wall[i + step * pw];
                    if ok {
                        let v = src[i + step * pw];
                        acc[0] += v[0] * 3;
                        acc[1] += v[1] * 3;
                        acc[2] += v[2] * 3;
                    }
                }
                // 4 diagonal taps (weight 1 each = 4; total stencil sum = 16).
                // Require clean cardinal shoulders so diagonals never leak past wall corners.
                for (dx, dy) in [(-1isize, -1isize), (1, -1), (-1, 1), (1, 1)] {
                    let nx = x as isize + dx * step as isize;
                    let ny = y as isize + dy * step as isize;
                    if nx < 0 || ny < 0 || nx >= pw as isize || ny >= ph as isize {
                        continue;
                    }
                    let shoulder_a = (y as isize * pw as isize + (x as isize + dx)) as usize;
                    let shoulder_b = ((y as isize + dy) * pw as isize + x as isize) as usize;
                    if is_wall[shoulder_a] || is_wall[shoulder_b] {
                        continue;
                    }
                    if step == 2 {
                        let mid = ((y as isize + dy) * pw as isize + (x as isize + dx)) as usize;
                        if is_wall[mid] {
                            continue;
                        }
                    }
                    let nb = ny as usize * pw + nx as usize;
                    if !is_wall[nb] {
                        let v = src[nb];
                        acc[0] += v[0];
                        acc[1] += v[1];
                        acc[2] += v[2];
                    }
                }
                let seed = emit[i];
                dst[i] = [
                    (seed[0] >> 2) + (((acc[0] >> 4) * GI_DECAY_Q8) >> 8),
                    (seed[1] >> 2) + (((acc[1] >> 4) * GI_DECAY_Q8) >> 8),
                    (seed[2] >> 2) + (((acc[2] >> 4) * GI_DECAY_Q8) >> 8),
                ];
            }
        }
    }

    let field = if (GI_COARSE_ITERS + GI_FINE_ITERS) % 2 == 1 {
        &pong[..]
    } else {
        &ping[..]
    };

    out.clear();
    out.resize(n, [0; 3]);
    let mut any = false;

    for y in y0..=y1 {
        let row = y * pw;
        for x in x0..=x1 {
            let i = row + x;
            if !is_wall[i] {
                // Remove the unpropagated self-seed so indirect light represents
                // transported neighbour bounce rather than re-scaling the core.
                let self_seed = emit[i];
                let mut ind = [
                    (field[i][0] - (self_seed[0] >> 2)).max(0),
                    (field[i][1] - (self_seed[1] >> 2)).max(0),
                    (field[i][2] - (self_seed[2] >> 2)).max(0),
                ];
                if water.is_some_and(|w| w.get(i).copied().unwrap_or(0) != 0) {
                    ind = [ind[0] >> 2, ind[1] >> 2, ind[2] >> 2];
                }
                let m = [
                    ((ind[0] * GI_MUL_GAIN_Q8) >> 8).clamp(0, 260) as u16,
                    ((ind[1] * GI_MUL_GAIN_Q8) >> 8).clamp(0, 260) as u16,
                    ((ind[2] * GI_MUL_GAIN_Q8) >> 8).clamp(0, 260) as u16,
                ];
                if (m[0] | m[1] | m[2]) != 0 {
                    out[i] = m;
                    any = true;
                }
            } else {
                // Exposed wall face: gather indirect light from open floor
                // cells in front of this face (`facing`), tinted gently by the
                // wall's own reflectance so lava pools and droppings warm/tint
                // nearby walls without crossing through to the room behind!
                let wf = facing[i];
                let mut sum = [0i32; 3];
                let mut wsum = 0i32;
                for dy in -1i32..=1 {
                    let ny = y as i32 + dy;
                    if ny < 0 || ny >= ph as i32 {
                        continue;
                    }
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = x as i32 + dx;
                        if nx < 0 || nx >= pw as i32 {
                            continue;
                        }
                        let nb = ny as usize * pw + nx as usize;
                        if is_wall[nb] {
                            continue;
                        }
                        if wf != Vec2::ZERO && (dx as f32 * wf.x + dy as f32 * wf.y) < -0.05 {
                            continue;
                        }
                        let w = if dx == 0 || dy == 0 { 2 } else { 1 };
                        let f = field[nb];
                        sum[0] += f[0] * w;
                        sum[1] += f[1] * w;
                        sum[2] += f[2] * w;
                        wsum += w;
                    }
                }
                if wsum > 0 {
                    let alb = albedo[i];
                    let wall_gain = (GI_MUL_GAIN_Q8 * 5) / 4;
                    let calc = |s: i32, a: u8| -> u16 {
                        let avg = s / wsum;
                        let tinted = (avg * (96 + ((i32::from(a) * 160) >> 8))) >> 8;
                        ((tinted * wall_gain) >> 8).clamp(0, 260) as u16
                    };
                    let m = [calc(sum[0], alb.0), calc(sum[1], alb.1), calc(sum[2], alb.2)];
                    if (m[0] | m[1] | m[2]) != 0 {
                        out[i] = m;
                        any = true;
                    }
                }
            }
        }
    }
    any
}

fn non_zero_bounds(buf: &[[i32; 3]], pw: usize, ph: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (pw, ph, 0usize, 0usize);
    let mut any = false;
    for y in 0..ph {
        let row = y * pw;
        for x in 0..pw {
            let v = buf[row + x];
            if (v[0] | v[1] | v[2]) != 0 {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    any.then_some((x0, y0, x1, y1))
}

/// Propagate one-bounce coloured indirect light across the 4 px grid and add
/// it onto `grid.samples` (after `upsample::bounce`, before `upsample::composite`).
pub(super) fn apply_indirect(
    grid: &mut Grid,
    geometry: &Geometry,
    static_shadows: &[ShadowProfile],
    scene: &LightScene<'_>,
    frame: &FxFrame<'_>,
    cap: f32,
) {
    if !scene.gi.gi_enabled(frame.mode) {
        return;
    }
    let (gw, gh) = (grid.w, grid.h);
    if gw == 0 || gh == 0 || grid.stride != 4 || grid.samples.len() < gw * gh {
        return;
    }

    let pw = gw + 2 * GI_PAD;
    let ph = gh + 2 * GI_PAD;
    let p_len = pw * ph;
    let pstart = grid.start - Vec2::splat((GI_PAD * 4) as f32);

    STATE.with(|cell| {
        let mut guard = cell.borrow_mut();
        let s = &mut *guard;
        let slot = select_level_slot(&mut s.levels, scene.level, &mut s.clock);

        // Prepare padded geometry/albedo slices (needed by dynamic pass and
        // static cache rebuilds).
        s.is_wall.clear();
        s.is_wall.resize(p_len, false);
        s.facing.clear();
        s.facing.resize(p_len, Vec2::ZERO);
        s.albedo.clear();
        s.albedo.resize(p_len, (0, 0, 0));
        s.water.clear();
        s.water.resize(p_len, 0);

        {
            let lvl = &s.levels[slot];
            for y in 0..ph {
                let wy = pstart.y + (y * 4) as f32;
                let row = y * pw;
                for x in 0..pw {
                    let p = Vec2::new(pstart.x + (x * 4) as f32, wy);
                    let surf = geometry.surface(p);
                    let idx = row + x;
                    s.is_wall[idx] = surf.layer & LayerMask::WALLS.0 != 0;
                    s.facing[idx] = surf.facing;
                    let (alb, wet) = lvl.sample_albedo(p);
                    s.albedo[idx] = alb;
                    s.water[idx] = u8::from(wet);
                }
            }
        }

        // ---- 1. Static fixture + sun + marine water GI (cached) ------------
        let (snuffed_len, snuffed_hash) = snuffed_signature(scene.level);
        let gi_key = StaticGiKey {
            level: LevelKey::of(scene.level),
            geo_rev: geometry.revision,
            snuffed_len,
            snuffed_hash,
            px0: pstart.x.to_bits(),
            py0: pstart.y.to_bits(),
            pw: pw as u16,
            ph: ph as u16,
            cap_q8: (cap.clamp(0.0, 2.0) * 256.0).round() as u16,
        };

        if s.static_gi.key != Some(gi_key) {
            s.static_gi.key = Some(gi_key);
            s.emit_indoor.clear();
            s.emit_indoor.resize(p_len, [0; 3]);
            s.emit_sky.clear();
            s.emit_sky.resize(p_len, [0; 3]);

            let theme = scene.level.theme;
            let profile = theme_ambient(theme);
            // Canonical steady scene (`elapsed = 0, blackout = 0`) so neither
            // sconce 3 Hz steps nor outage fades enter the cached field.
            let steady_scene = LightScene {
                ecs: scene.ecs,
                level: scene.level,
                player: scene.player,
                particles: scene.particles,
                thrash: None,
                squeak: None,
                cam: scene.cam,
                elapsed: 0.0,
                blackout: 0.0,
                mode: LightingMode::Full,
                depth: scene.depth,
                fx: scene.fx,
                gi: scene.gi,
            };

            let end = pstart + Vec2::new((pw * 4) as f32, (ph * 4) as f32);
            for (i, fixture) in scene.level.objects.lights.iter().enumerate() {
                if fixtures::sconce_snuffed(scene.level, i, *fixture) {
                    continue;
                }
                // Broken bulbs and spent sconces only sputter intermittently;
                // excluding them keeps static GI 100 % flicker-free.
                if fixture.kind == LightFixtureKind::Broken
                    || (fixture.kind == LightFixtureKind::Sconce && fixtures::sconce_spent(fixture.seed))
                {
                    continue;
                }
                let mut light = fixtures::light(i, *fixture, theme, profile, None, &steady_scene);
                light.flicker = None;
                if fixture.kind == LightFixtureKind::Sun {
                    let LightShape::Directional { dir } = light.shape else {
                        continue;
                    };
                    let ray = -dir.normalized();
                    let sun_e = ((light.energy + light.scatter * 0.65) * 256.0) as i32;
                    let col = [
                        (i32::from(light.color.0) * sun_e) >> 8,
                        (i32::from(light.color.1) * sun_e) >> 8,
                        (i32::from(light.color.2) * sun_e) >> 8,
                    ];
                    for y in 0..ph {
                        let wy = pstart.y + (y * 4) as f32;
                        for x in 0..pw {
                            let p = Vec2::new(pstart.x + (x * 4) as f32, wy);
                            let surf = geometry.surface(p);
                            if surf.zone == Some(LightZoneKind::Maze)
                                || surf.layer & light.layers.0 == 0
                            {
                                continue;
                            }
                            let vis = if light.casts_shadow {
                                geometry.sun_visibility(p, surf, light, ray, &[])
                            } else {
                                1.0
                            };
                            if vis <= 0.02 {
                                continue;
                            }
                            let v_q8 = (vis * 256.0) as i32;
                            let inc = [
                                (col[0] * v_q8) >> 8,
                                (col[1] * v_q8) >> 8,
                                (col[2] * v_q8) >> 8,
                            ];
                            inject_surface_bounce(
                                &mut s.emit_sky,
                                &s.is_wall,
                                &s.facing,
                                &s.albedo,
                                (pw, ph),
                                (x, y),
                                inc,
                            );
                        }
                    }
                    continue;
                }

                let r = light.extent();
                if light.pos.x + r < pstart.x
                    || light.pos.x - r > end.x
                    || light.pos.y + r < pstart.y
                    || light.pos.y - r > end.y
                    || light.energy <= 0.001
                {
                    continue;
                }
                let own = zone_gain(geometry.surface(fixture.pos).zone);
                if own > cap {
                    light.energy *= cap / own;
                    light.scatter *= cap / own;
                }
                // Reuse `LightMap`'s baked profile when valid, else bake into
                // scratch for fixtures sitting in the 24 px off-screen pad.
                let shadow_ref = if let Some(existing) = static_shadows.get(i) {
                    s.temp_shadow = *existing;
                    s.temp_shadow.bake(geometry, light);
                    Some(&s.temp_shadow)
                } else {
                    s.temp_shadow.bake(geometry, light);
                    Some(&s.temp_shadow)
                };
                splat_steady_light(
                    light,
                    shadow_ref,
                    false,
                    geometry,
                    pstart,
                    (pw, ph),
                    &s.is_wall,
                    &s.facing,
                    &s.albedo,
                    &mut s.emit_indoor,
                );
                if fixture.kind == LightFixtureKind::Sconce {
                    let mut wash = fixtures::sconce_wash(&light, fixture.seed, 0.0);
                    // Keep the lamp's true spectral tint for indirect bounce
                    // rather than `lens_color`'s white-hot eye-glare core.
                    wash.color = light.color;
                    wash.flicker = None;
                    splat_steady_light(
                        wash,
                        shadow_ref,
                        false,
                        geometry,
                        pstart,
                        (pw, ph),
                        &s.is_wall,
                        &s.facing,
                        &s.albedo,
                        &mut s.emit_indoor,
                    );
                }
            }

            // Outfall marine water column: standing water puddles scatter cool
            // cyan sky-water light onto surrounding sand and timber pilings.
            if theme == Theme::Outfall {
                let scale = super::upsample::ambient_scale(theme, frame.mode);
                for y in 0..ph {
                    let wy = pstart.y + (y * 4) as f32;
                    for x in 0..pw {
                        let idx = y * pw + x;
                        if s.water[idx] == 0 || s.is_wall[idx] {
                            continue;
                        }
                        let p = Vec2::new(pstart.x + (x * 4) as f32, wy);
                        let z_gain = zone_gain(geometry.surface(p).zone).min(cap);
                        let amb_b = i32::from(ambient_level_scaled(frame.ambient[2], z_gain, scale));
                        // Cyan water emission proportional to marine ambient.
                        s.emit_sky[idx][0] += (amb_b * 88) >> 8;
                        s.emit_sky[idx][1] += (amb_b * 1840) >> 8;
                        s.emit_sky[idx][2] += (amb_b * 2280) >> 8;
                    }
                }
            }

            s.static_gi.any_indoor = if let Some(b) = non_zero_bounds(&s.emit_indoor, pw, ph) {
                propagate_gi(
                    &s.emit_indoor,
                    &s.is_wall,
                    &s.facing,
                    &s.albedo,
                    None,
                    (pw, ph),
                    b,
                    &mut s.ping,
                    &mut s.pong,
                    &mut s.static_gi.indoor,
                )
            } else {
                s.static_gi.indoor.clear();
                s.static_gi.indoor.resize(p_len, [0; 3]);
                false
            };

            s.static_gi.any_sky = if let Some(b) = non_zero_bounds(&s.emit_sky, pw, ph) {
                propagate_gi(
                    &s.emit_sky,
                    &s.is_wall,
                    &s.facing,
                    &s.albedo,
                    Some(&s.water),
                    (pw, ph),
                    b,
                    &mut s.ping,
                    &mut s.pong,
                    &mut s.static_gi.sky,
                )
            } else {
                s.static_gi.sky.clear();
                s.static_gi.sky.resize(p_len, [0; 3]);
                false
            };
        }

        // ---- 2. Steady dynamic & ground emitters (no flicker/pulse) --------
        s.emit_dyn.clear();
        s.emit_dyn.resize(p_len, [0; 3]);

        let end = pstart + Vec2::new((pw * 4) as f32, (ph * 4) as f32);
        let overlaps_pad = |pos: Vec2, r: f32| -> bool {
            pos.x + r >= pstart.x && pos.x - r <= end.x && pos.y + r >= pstart.y && pos.y - r <= end.y
        };

        // 2a. Player's personal aura (steady: `rat_flicker = 1.0`).
        {
            let theme_prof = theme_ambient(scene.level.theme);
            let rat_r = match scene.level.theme {
                Theme::Lab => 58.0,
                Theme::Outfall => 54.0,
                _ => 66.0,
            };
            let energy = match scene.level.theme {
                Theme::Trailer => 62.0 / 256.0,
                Theme::Outfall => 184.0 / 256.0,
                _ => 196.0 / 256.0,
            };
            let mut rat_light = Light::radial(
                 Vec2::new(scene.player.pos.x.round(), scene.player.pos.y.round()),
                rat_r,
                theme_prof.rat_tint,
                energy,
            );
            rat_light.scatter = match scene.level.theme {
                Theme::Trailer => 7.0 / 256.0,
                Theme::Outfall => 20.0 / 256.0,
                _ => 22.0 / 256.0,
            };
            if overlaps_pad(rat_light.pos, rat_light.extent()) {
                s.rat_shadow.bake(geometry, rat_light);
                splat_steady_light(
                    rat_light,
                    Some(&s.rat_shadow),
                    false,
                    geometry,
                    pstart,
                    (pw, ph),
                    &s.is_wall,
                    &s.facing,
                    &s.albedo,
                    &mut s.emit_dyn,
                );
            }
        }

        // 2b. Custom shaped lights pushed directly onto `LightMap` in tests or
        // boss fights (`frame.lights` carries `steady`, excluding fixtures/sun).
        for fxl in frame.lights {
            if matches!(fxl.tag, super::fx::LightTag::Fixture(_) | super::fx::LightTag::SconceWash | super::fx::LightTag::Sun) {
                continue;
            }
            if !fxl.extent.is_finite() || fxl.steady < 18.0 {
                continue;
            }
            // Skip the rat's own aura (already splatted above with its exact
            // `ShadowProfile`).
            if fxl.pos.dist_sq(scene.player.pos) <= 2.5 && (fxl.extent - 66.0).abs() < 14.0 {
                continue;
            }
            let peak = f32::from(fxl.color.0.max(fxl.color.1).max(fxl.color.2)).max(1.0);
            let total_e = fxl.steady / peak;
            let mut l = Light::radial(fxl.pos, fxl.extent, fxl.color, total_e * 0.85);
            l.scatter = total_e * 0.15;
            l.height = fxl.height;
            l.falloff = Falloff::Smooth;
            if overlaps_pad(l.pos, l.extent()) {
                splat_steady_light(
                    l,
                    None,
                    true,
                    geometry,
                    pstart,
                    (pw, ph),
                    &s.is_wall,
                    &s.facing,
                    &s.albedo,
                    &mut s.emit_dyn,
                );
            }
        }

        // 2c. Ground emitters: LavaPool, LavaGlob, Hantavirus Dropping, Acid Pool, Pickup.
        // Iterated in deterministic order where capped; wall-occluded so ground
        // pools warm nearby walls on their own side and never through a wall.
        let mut drops: Vec<(u64, &Dropping)> = Vec::new();
        for entity in scene.ecs.iter_entities() {
            if let Some(pool) = entity.get::<LavaPool>() {
                let fade = (pool.life - pool.age).clamp(0.0, 1.0);
                if fade > 0.01 {
                    let r = 22.0 * fade.min(1.0);
                    if overlaps_pad(pool.pos, r * super::TAIL_REACH) {
                        let mut l = Light::radial(pool.pos, r, (255, 136, 36), 1.35 * fade);
                        l.scatter = 0.28 * fade;
                        l.falloff = Falloff::Small;
                        l.height = 4.0;
                        splat_steady_light(
                            l,
                            None,
                            true,
                            geometry,
                            pstart,
                            (pw, ph),
                            &s.is_wall,
                            &s.facing,
                            &s.albedo,
                            &mut s.emit_dyn,
                        );
                    }
                }
            } else if let Some(glob) = entity.get::<LavaGlob>() {
                let h = glob.height();
                let pos = glob.ground - Vec2::new(0.0, h);
                if overlaps_pad(pos, 18.0 * super::TAIL_REACH) {
                    let mut l = Light::radial(pos, 18.0, (255, 140, 40), 0.72);
                    l.scatter = 0.16;
                    l.falloff = Falloff::Small;
                    l.height = h.max(3.0);
                    splat_steady_light(
                        l,
                        None,
                        true,
                        geometry,
                        pstart,
                        (pw, ph),
                        &s.is_wall,
                        &s.facing,
                        &s.albedo,
                        &mut s.emit_dyn,
                    );
                }
            } else if let Some(drop) = entity.get::<Dropping>() {
                let spid = entity.get::<SpawnId>().map_or(0, |id| id.0);
                drops.push((spid, drop));
            } else if let Some(p) = entity.get::<Pool>() {
                let r = p.radius * 2.1 + 12.0;
                if overlaps_pad(p.pos, r * super::TAIL_REACH) {
                    let k = (p.life / 2.6).clamp(0.25, 1.0);
                    // Steady (no `elapsed` pulse) so still pools never flicker GI.
                    let mut l = Light::radial(p.pos, r, p.color, (175.0 / 256.0) * k);
                    l.scatter = (44.0 / 256.0) * k;
                    l.falloff = Falloff::Small;
                    splat_steady_light(
                        l,
                        None,
                        true,
                        geometry,
                        pstart,
                        (pw, ph),
                        &s.is_wall,
                        &s.facing,
                        &s.albedo,
                        &mut s.emit_dyn,
                    );
                }
            } else if let Some(p) = entity.get::<Pickup>() {
                if overlaps_pad(p.pos, 32.0 * super::TAIL_REACH) {
                    let mut l = Light::radial(p.pos, 30.0, p.kind.color(), 155.0 / 256.0);
                    l.scatter = 36.0 / 256.0;
                    l.falloff = Falloff::Small;
                    splat_steady_light(
                        l,
                        None,
                        true,
                        geometry,
                        pstart,
                        (pw, ph),
                        &s.is_wall,
                        &s.facing,
                        &s.albedo,
                        &mut s.emit_dyn,
                    );
                }
            }
        }

        if !drops.is_empty() {
            drops.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
            for &(_, drop) in drops.iter().take(12) {
                let left = DROPPING_LIFE - drop.age;
                if left <= 0.0 {
                    continue;
                }
                let rank = usize::from(drop.level.clamp(1, 5) - 1);
                let arm = if drop.age < DROPPING_ARM { 0.5 } else { 1.0 };
                let fade = (left / 2.0).min(1.0);
                let r = DROPPING_LIGHT_RADIUS[rank];
                if overlaps_pad(drop.pos, r * super::TAIL_REACH) {
                    let mut l = Light::radial(
                        drop.pos,
                        r,
                        (110, 255, 120),
                        DROPPING_LIGHT_ENERGY[rank] * arm * fade * 1.15,
                    );
                    l.scatter = 0.18 * arm * fade;
                    l.falloff = Falloff::Small;
                    l.height = 3.0;
                    splat_steady_light(
                        l,
                        None,
                        true,
                        geometry,
                        pstart,
                        (pw, ph),
                        &s.is_wall,
                        &s.facing,
                        &s.albedo,
                        &mut s.emit_dyn,
                    );
                }
            }
        }

        let any_dyn = if let Some(b) = non_zero_bounds(&s.emit_dyn, pw, ph) {
            propagate_gi(
                &s.emit_dyn,
                &s.is_wall,
                &s.facing,
                &s.albedo,
                None,
                (pw, ph),
                b,
                &mut s.ping,
                &mut s.pong,
                &mut s.dyn_out,
            )
        } else {
            false
        };

        // ---- 3. Composite cached static (× power) + dynamic onto `grid` ----
        let indoor_power_q8 = ((1.0 - scene.blackout).clamp(0.0, 1.0) * 256.0).round() as i32;
        // Outfall ambient water bounce dims substantially during a blackout,
        // while the Flatbed sun is unaffected.
        let sky_power_q8 = if scene.level.theme == Theme::Outfall {
            ((1.0 - scene.blackout * 0.80).clamp(0.0, 1.0) * 256.0).round() as i32
        } else {
            256
        };
        let ceiling = if scene.fx {
            super::upsample::MUL_CEILING
        } else {
            MUL_CLAMP
        };

        let use_indoor = s.static_gi.any_indoor && indoor_power_q8 > 0;
        let use_sky = s.static_gi.any_sky && sky_power_q8 > 0;
        if !use_indoor && !use_sky && !any_dyn {
            return;
        }

        for gy in 0..gh {
            let p_row = (gy + GI_PAD) * pw + GI_PAD;
            let g_row = gy * gw;
            for gx in 0..gw {
                let pi = p_row + gx;
                let sample = &mut grid.samples[g_row + gx];
                for c in 0..3 {
                    let mut total = 0i32;
                    if use_indoor {
                        total += (i32::from(s.static_gi.indoor[pi][c]) * indoor_power_q8) >> 8;
                    }
                    if use_sky {
                        total += (i32::from(s.static_gi.sky[pi][c]) * sky_power_q8) >> 8;
                    }
                    if any_dyn {
                        total += i32::from(s.dyn_out[pi][c]);
                    }
                    let mul_inc = total & !3;
                    if mul_inc >= GI_MIN_MUL {
                        sample.mul[c] = (i32::from(sample.mul[c]) + mul_inc).min(ceiling) as u16;
                    }
                    let add_inc = (((total * GI_ADD_SHARE_Q8) >> 8) & !1).min(i32::from(GI_ADD_MAX));
                    if add_inc >= GI_MIN_ADD {
                        sample.add[c] = (i32::from(sample.add[c]) + add_inc).clamp(-128, 142) as i16;
                    }
                }
            }
        }
    });
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::components::{Dropping, LavaPool, SpawnId};
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::{LightFixture, LightFixtureKind, Prop, Puddle};
    use crate::game::world::World;
    use crate::render::framebuffer::PixelBuffer;
    use crate::render::lighting::{luminance, LightMap};
    use crate::term::ColorDepth;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn render_with(w: &mut World, gi: GlobalLight, size: (usize, usize)) -> PixelBuffer {
        let mut pb = PixelBuffer::new(size.0, size.1);
        w.global_light = gi;
        w.draw(&mut pb);
        pb
    }

    #[test]
    fn off_is_byte_identical_across_themes_and_zooms() {
        for theme in Theme::playable() {
            for zoom in [1.0f32, 2.0] {
                let mut w_a = World::new(theme, 42);
                w_a.set_zoom_instant(zoom);
                w_a.lighting = LightingMode::Full;
                w_a.lighting_depth = ColorDepth::Truecolor;
                w_a.lighting_fx = true;
                for _ in 0..12 {
                    w_a.update(1.0 / 60.0, Vec2::new(1.0, 0.2).normalized(), false);
                }
                let mut w_b = World::new(theme, 42);
                w_b.set_zoom_instant(zoom);
                w_b.lighting = LightingMode::Full;
                w_b.lighting_depth = ColorDepth::Truecolor;
                w_b.lighting_fx = true;
                for _ in 0..12 {
                    w_b.update(1.0 / 60.0, Vec2::new(1.0, 0.2).normalized(), false);
                }

                // Exercise AO+GI first so thread-local scratch is warm, then
                // verify switching back to Off matches a fresh Off world byte for byte.
                let _warm = render_with(&mut w_b, GlobalLight::AoGi, (120, 72));
                let off_after = render_with(&mut w_b, GlobalLight::Off, (120, 72));
                let off_fresh = render_with(&mut w_a, GlobalLight::Off, (120, 72));
                assert_eq!(
                    off_after.pixels(),
                    off_fresh.pixels(),
                    "{theme:?} at {zoom}x: Off must be byte-identical"
                );
            }
        }
    }

    #[test]
    fn ao_darkens_inside_corner_more_than_straight_wall_and_open_floor() {
        let mut w = World::new(Theme::Kitchen, 101);
        w.level = Level::empty(Theme::Kitchen);
        // Carve an L-shaped wall: top horizontal wall at row 10 (`x = 10..24`),
        // left vertical wall at col 10 (`y = 10..24`). Inside corner sits at
        // cell (11, 11) -> world (176, 176).
        w.level
            .tiles
            .fill_cells(10, 10, 14, 1, TileKind::Wall(WallKind::Solid));
        w.level
            .tiles
            .fill_cells(10, 10, 1, 14, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        // Light the room evenly with an overhead Kitchen pendant so floor pixels
        // have clean 8-bit dynamic range while keeping the rat far off-screen.
        w.level.objects.lights.push(LightFixture {
            pos: Vec2::new(240.0, 224.0),
            kind: LightFixtureKind::Pendant,
            radius: 190.0,
            seed: 4,
        });
        w.player.pos = Vec2::new(900.0, 900.0);
        w.cam_focus = Some(Vec2::new(224.0, 210.0));
        w.lighting = LightingMode::Full;
        w.lighting_depth = ColorDepth::Truecolor;
        w.lighting_fx = true;
        w.smoke_enabled = false;

        let off = render_with(&mut w, GlobalLight::Off, (160, 160));
        let ao = render_with(&mut w, GlobalLight::AoOnly, (160, 160));
        let cam = (w.cam.x.round() as i32, w.cam.y.round() as i32);

        let drop_at = |wx: i32, wy: i32| -> i32 {
            let (sx, sy) = ((wx - cam.0) as usize, (wy - cam.1) as usize);
            i32::from(luminance(off.pixel_at(sx, sy))) - i32::from(luminance(ao.pixel_at(sx, sy)))
        };

        // Inside corner (2 px diagonally out from both walls: world 178, 178).
        let corner_drop = drop_at(178, 178);
        // Straight north wall far from the vertical leg (world 272, 178).
        let wall_drop = drop_at(272, 178);
        // Open floor well clear of walls and rat (world 240, 224).
        let open_drop = drop_at(240, 224);

        assert_eq!(open_drop, 0, "open floor away from geometry takes zero AO");
        assert!(wall_drop > 0, "floor beside a wall face must darken ({wall_drop})");
        assert!(
            corner_drop > wall_drop,
            "inside corner ({corner_drop}) must darken more than a straight wall ({wall_drop}) and open floor ({open_drop})"
        );
    }

    #[test]
    fn ao_prop_skirt_scales_with_height_class_and_crevices_darken() {
        let mut lvl_low = Level::empty(Theme::Lab);
        lvl_low.props.push(Prop {
            kind: PropKind::Toolbox, // small height class
            rect: Rectf::new(160.0, 160.0, 20.0, 12.0),
        });
        let mut lvl_tall = Level::empty(Theme::Lab);
        lvl_tall.props.push(Prop {
            kind: PropKind::PressIsland, // tall height class
            rect: Rectf::new(160.0, 160.0, 48.0, 32.0),
        });

        let mut c_low = LevelCache::default();
        c_low.sync(&lvl_low, 1);
        let mut c_tall = LevelCache::default();
        c_tall.sync(&lvl_tall, 2);

        // 3 px west of both props (`x = 157`, mid-height).
        let low_ao = c_low.sample_ao(157, 166);
        let tall_ao = c_tall.sample_ao(157, 176);
        assert!(low_ao > 0, "low prop casts contact AO");
        assert!(
            tall_ao > low_ao,
            "tall prop AO ({tall_ao}) must exceed low prop AO ({low_ao})"
        );

        // Crevice test on wall art: a dark mortar groove surrounded by bright
        // brick takes crevice AO while flat brick stays 0.
        let (w, h) = (16usize, 16usize);
        let mut grid = super::super::fx::testkit::flat_grid(4, 4, 4, [180, 180, 180]);
        for surf in &mut grid.surfaces {
            surf.layer = LayerMask::WALLS.0;
        }
        let frame = super::super::fx::testkit::frame(&lvl_low, LightingMode::Full);
        let mut pb = PixelBuffer::new(w, h);
        pb.set_materials(true);
        pb.clear((130, 120, 115));
        // 1 px recessed mortar line at y = 8.
        for x in 0..w as i32 {
            pb.plot(x, 8, (55, 50, 48));
        }
        let canvas = super::super::fx::testkit::canvas(&mut pb);
        capture_crevices(&grid, &canvas, &frame, GlobalLight::AoOnly);
        STATE.with(|cell| {
            let s = cell.borrow();
            assert!(s.crevice[8 * w + 8] > 15, "mortar groove detected");
            assert_eq!(s.crevice[3 * w + 8], 0, "flat wall brick untouched");
        });
    }

    #[test]
    fn ao_grounds_landed_bodies_and_skips_flyers() {
        let mut w_ground = World::new(Theme::Kitchen, 55);
        w_ground.level = Level::empty(Theme::Kitchen);
        w_ground.player.pos = Vec2::new(400.0, 400.0);
        w_ground.cam_focus = Some(Vec2::new(220.0, 220.0));
        w_ground.smoke_enabled = false;
        let mut rng = StdRng::seed_from_u64(7);
        w_ground
            .ecs
            .spawn((Enemy::spawn(EnemyKind::Cat, Vec2::new(220.0, 220.0), 1.0, &mut rng), SpawnId(1)));

        let mut w_flyer = World::new(Theme::Kitchen, 55);
        w_flyer.level = Level::empty(Theme::Kitchen);
        w_flyer.player.pos = Vec2::new(400.0, 400.0);
        w_flyer.cam_focus = Some(Vec2::new(220.0, 220.0));
        w_flyer.smoke_enabled = false;
        let mut rng2 = StdRng::seed_from_u64(7);
        w_flyer
            .ecs
            .spawn((Enemy::spawn(EnemyKind::Bat, Vec2::new(220.0, 220.0), 1.0, &mut rng2), SpawnId(1)));

        let mut bodies = Vec::new();
        let scene_g = LightScene {
            ecs: &w_ground.ecs,
            level: &w_ground.level,
            player: &w_ground.player,
            particles: &w_ground.particles,
            thrash: None,
            squeak: None,
            cam: Vec2::new(160.0, 160.0),
            elapsed: 0.0,
            blackout: 0.0,
            mode: LightingMode::Full,
            depth: ColorDepth::Truecolor,
            fx: true,
            gi: GlobalLight::AoOnly,
        };
        gather_grounded_bodies(&mut bodies, &scene_g, (160, 160), (120, 120));
        assert_eq!(bodies.len(), 1, "grounded Cat is gathered");

        let scene_f = LightScene {
            ecs: &w_flyer.ecs,
            level: &w_flyer.level,
            player: &w_flyer.player,
            particles: &w_flyer.particles,
            ..scene_g
        };
        gather_grounded_bodies(&mut bodies, &scene_f, (160, 160), (120, 120));
        assert!(bodies.is_empty(), "flying Bat casts no ground-contact AO");
    }

    #[test]
    fn hard_runs_ao_only_and_coarse_compact_run_nothing() {
        assert!(GlobalLight::AoOnly.ao_enabled(LightingMode::Full));
        assert!(GlobalLight::AoOnly.ao_enabled(LightingMode::Hard));
        assert!(GlobalLight::AoGi.ao_enabled(LightingMode::Hard));
        assert!(!GlobalLight::AoGi.gi_enabled(LightingMode::Hard));
        for tier in [LightingMode::Coarse, LightingMode::Compact] {
            assert!(!GlobalLight::AoGi.ao_enabled(tier));
            assert!(!GlobalLight::AoGi.gi_enabled(tier));
        }
    }

    #[test]
    fn gi_tints_floor_near_coloured_lit_wall_and_never_through_wall() {
        // 1. Neutral white lamp shining on a vivid red wall (`wx = 240..256`):
        //    surface albedo alone tints the nearby east floor red, and nothing
        //    leaks through the wall to the west room (`wx < 240`).
        {
            let mut w = World::new(Theme::Printworks, 87);
            w.level = Level::empty(Theme::Printworks);
            w.level.palette.wall = (245, 32, 32);
            w.level.palette.wall_edge = (255, 48, 48);
            w.level
                .tiles
                .fill_cells(15, 8, 1, 16, TileKind::Wall(WallKind::Solid));
            w.level.tiles.rebuild_masks();
            // Neutral white overhead pendant (`seed = 0`) 20 px east of the wall face.
            w.level.objects.lights.push(LightFixture {
                pos: Vec2::new(276.0, 248.0),
                kind: LightFixtureKind::Pendant,
                radius: 96.0,
                seed: 0,
            });
            w.player.pos = Vec2::new(1200.0, 1200.0);
            w.cam_focus = Some(Vec2::new(256.0, 248.0));
            w.lighting = LightingMode::Full;
            w.lighting_depth = ColorDepth::Truecolor;
            w.lighting_fx = true;
            w.smoke_enabled = false;

            let ao = render_with(&mut w, GlobalLight::AoOnly, (160, 120));
            let gi = render_with(&mut w, GlobalLight::AoGi, (160, 120));
            let cam = (w.cam.x.round() as i32, w.cam.y.round() as i32);

            let (ex, ey) = ((266 - cam.0) as usize, (248 - cam.1) as usize);
            let before = ao.pixel_at(ex, ey);
            let after = gi.pixel_at(ex, ey);
            let dr = i32::from(after.0) - i32::from(before.0);
            let dg = i32::from(after.1) - i32::from(before.1);
            let db = i32::from(after.2) - i32::from(before.2);
            assert!(
                dr >= 4 && dr > dg + 2 && dr > db + 2,
                "floor near white-lit red wall must shift toward red: {before:?} -> {after:?}"
            );

            let (wx_s, wy_s) = ((228 - cam.0) as usize, (248 - cam.1) as usize);
            assert_eq!(
                gi.pixel_at(wx_s, wy_s),
                ao.pixel_at(wx_s, wy_s),
                "GI must not leak through a solid wall"
            );
        }

        // 2. Printworks red safelight (`seed = 1`) mounted on a wall: tints
        //    nearby floor red and never crosses through the wall.
        {
            let mut w = World::new(Theme::Printworks, 88);
            w.level = Level::empty(Theme::Printworks);
            w.level
                .tiles
                .fill_cells(15, 8, 1, 16, TileKind::Wall(WallKind::Solid));
            w.level.tiles.rebuild_masks();
            let mount = TileMap::tile_rect(15, 15).center() + Vec2::new(TILE_F * 0.5 + 0.5, 0.0);
            w.level.objects.lights.push(LightFixture {
                pos: mount,
                kind: LightFixtureKind::Sconce,
                radius: 110.0,
                seed: 1,
            });
            w.player.pos = Vec2::new(1200.0, 1200.0);
            w.cam_focus = Some(Vec2::new(256.0, 248.0));
            w.lighting = LightingMode::Full;
            w.lighting_depth = ColorDepth::Truecolor;
            w.lighting_fx = true;
            w.smoke_enabled = false;

            let ao = render_with(&mut w, GlobalLight::AoOnly, (160, 120));
            let gi = render_with(&mut w, GlobalLight::AoGi, (160, 120));
            let cam = (w.cam.x.round() as i32, w.cam.y.round() as i32);

            // Sample floor beside the wall just outside the direct cone (`wy = 276`).
            let (ex, ey) = ((266 - cam.0) as usize, (276 - cam.1) as usize);
            let before = ao.pixel_at(ex, ey);
            let after = gi.pixel_at(ex, ey);
            let dr = i32::from(after.0) - i32::from(before.0);
            let dg = i32::from(after.1) - i32::from(before.1);
            let db = i32::from(after.2) - i32::from(before.2);
            assert!(
                dr >= 4 && dr > dg + 2 && dr > db + 2,
                "floor near red lamp-lit wall must shift toward red: {before:?} -> {after:?}"
            );

            let (wx_s, wy_s) = ((228 - cam.0) as usize, (248 - cam.1) as usize);
            assert_eq!(
                gi.pixel_at(wx_s, wy_s),
                ao.pixel_at(wx_s, wy_s),
                "GI must not leak through a solid wall"
            );
        }
    }

    #[test]
    fn gi_lava_warms_nearby_wall_and_droppings_tint_floor_and_water_bounces_cyan() {
        // 1. Lava pool near a wall warms the facing wall face and not the room behind.
        {
            let mut w = World::new(Theme::Lab, 91);
            w.level = Level::empty(Theme::Lab);
            w.level
                .tiles
                .fill_cells(12, 10, 8, 1, TileKind::Wall(WallKind::Solid));
            w.level.tiles.rebuild_masks();
            // Lava pool 18 px south of the south wall face (`wy = 176 + 18 = 194`).
            w.ecs.spawn((
                LavaPool {
                    pos: Vec2::new(240.0, 194.0),
                    radius: 14.0,
                    age: 0.2,
                    life: 4.0,
                    dps: 10.0,
                    seed: 3,
                },
                SpawnId(1),
            ));
            w.player.pos = Vec2::new(1200.0, 1200.0);
            w.cam_focus = Some(Vec2::new(240.0, 184.0));
            w.lighting = LightingMode::Full;
            w.lighting_fx = true;
            w.smoke_enabled = false;

            let ao = render_with(&mut w, GlobalLight::AoOnly, (120, 96));
            let gi = render_with(&mut w, GlobalLight::AoGi, (120, 96));
            let cam = (w.cam.x.round() as i32, w.cam.y.round() as i32);
            // South front face of wall cell (15, 10): world `(242, 173)`.
            let (sx, sy) = ((242 - cam.0) as usize, (173 - cam.1) as usize);
            let b_px = ao.pixel_at(sx, sy);
            let g_px = gi.pixel_at(sx, sy);
            let dr = i32::from(g_px.0) - i32::from(b_px.0);
            let db = i32::from(g_px.2) - i32::from(b_px.2);
            assert!(
                dr > 0 && dr > db,
                "lava pool must warm the nearby wall face: {b_px:?} -> {g_px:?}"
            );
            // Floor north of the wall (`wy = 148`) stays untouched.
            let (nx, ny) = ((242 - cam.0) as usize, (148 - cam.1) as usize);
            assert_eq!(gi.pixel_at(nx, ny), ao.pixel_at(nx, ny));
        }

        // 2. Green Hantavirus dropping tints surrounding floor green.
        {
            let mut w = World::new(Theme::Kitchen, 92);
            w.level = Level::empty(Theme::Kitchen);
            w.ecs.spawn((
                Dropping {
                    pos: Vec2::new(240.0, 240.0),
                    age: 1.0,
                    damage: 4.0,
                    jumps: 1,
                    reach: 14.0,
                    radius: 7.0,
                    seed: 2,
                    level: 5,
                },
                SpawnId(1),
            ));
            w.player.pos = Vec2::new(1200.0, 1200.0);
            w.cam_focus = Some(Vec2::new(240.0, 240.0));
            w.lighting = LightingMode::Full;
            w.lighting_fx = true;
            w.smoke_enabled = false;

            let ao = render_with(&mut w, GlobalLight::AoOnly, (120, 96));
            let gi = render_with(&mut w, GlobalLight::AoGi, (120, 96));
            let cam = (w.cam.x.round() as i32, w.cam.y.round() as i32);
            let (sx, sy) = ((262 - cam.0) as usize, (240 - cam.1) as usize);
            let b_px = ao.pixel_at(sx, sy);
            let g_px = gi.pixel_at(sx, sy);
            let dg = i32::from(g_px.1) - i32::from(b_px.1);
            let dr = i32::from(g_px.0) - i32::from(b_px.0);
            assert!(
                dg >= 4 && dg > dr,
                "dropping GI must tint surrounding floor green: {b_px:?} -> {g_px:?}"
            );
        }

        // 3. Outfall blue water puddle bounces marine cyan onto adjacent dry sand.
        {
            let mut w = World::new(Theme::Outfall, 93);
            w.level = Level::empty(Theme::Outfall);
            w.level.puddles.push(Puddle {
                pos: Vec2::new(240.0, 240.0),
                rx: 24.0,
                ry: 16.0,
                seed: 5,
            });
            w.player.pos = Vec2::new(1200.0, 1200.0);
            w.cam_focus = Some(Vec2::new(240.0, 240.0));
            w.lighting = LightingMode::Full;
            w.lighting_fx = true;
            w.smoke_enabled = false;

            let ao = render_with(&mut w, GlobalLight::AoOnly, (120, 96));
            let gi = render_with(&mut w, GlobalLight::AoGi, (120, 96));
            let cam = (w.cam.x.round() as i32, w.cam.y.round() as i32);
            // Dry sand 8 px outside the puddle's right edge (`wx = 272`).
            let (sx, sy) = ((272 - cam.0) as usize, (240 - cam.1) as usize);
            assert!(!w.level.wet_at(Vec2::new(272.0, 240.0)));
            let b_px = ao.pixel_at(sx, sy);
            let g_px = gi.pixel_at(sx, sy);
            let dg = i32::from(g_px.1) - i32::from(b_px.1);
            let db = i32::from(g_px.2) - i32::from(b_px.2);
            let dr = i32::from(g_px.0) - i32::from(b_px.0);
            assert!(
                (dg > 0 || db > 0) && dg + db > dr * 2,
                "Outfall water must bounce cyan onto nearby sand: {b_px:?} -> {g_px:?}"
            );
        }
    }

    #[test]
    fn gi_sunlit_flatbed_planks_bounce_warm_light_into_shadow() {
        let mut w = World::new(Theme::Trailer, 64);
        w.clear_enemies();
        // Place a cargo crate on the deck and stand the camera beside its sun shadow.
        let deck_c = w.level.trailer.deck.center_world();
        w.level.props.push(Prop {
            kind: PropKind::Crate,
            rect: Rectf::new(deck_c.x - 12.0, deck_c.y - 12.0, 24.0, 24.0),
        });
        w.player.pos = deck_c + Vec2::new(300.0, 0.0);
        w.cam_focus = Some(deck_c);
        w.lighting = LightingMode::Full;
        w.lighting_fx = true;
        w.smoke_enabled = false;

        let ao = render_with(&mut w, GlobalLight::AoOnly, (120, 80));
        let gi = render_with(&mut w, GlobalLight::AoGi, (120, 80));
        // Across the shaded plank pixels around the crate, warm red+green gain
        // exceeds blue gain.
        let mut warm_gain = 0i64;
        let mut blue_gain = 0i64;
        for (a, g) in ao.pixels().iter().zip(gi.pixels().iter()) {
            warm_gain += i64::from(g.0) - i64::from(a.0);
            blue_gain += i64::from(g.2) - i64::from(a.2);
        }
        assert!(
            warm_gain > blue_gain && warm_gain > 200,
            "Flatbed sunlit planks must bounce warm amber light (warm +{warm_gain} vs blue +{blue_gain})"
        );
    }

    #[test]
    fn frozen_frames_are_identical_and_blackouts_dim_gi() {
        let mut w = World::new(Theme::Sewer, 77);
        w.clear_enemies();
        // Stand in front of the nearest working sconce.
        if let Some(lamp) = w
            .level
            .objects
            .lights
            .iter()
            .find(|f| f.kind == LightFixtureKind::Sconce && !fixtures::sconce_spent(f.seed))
            .copied()
        {
            let dir = lamp.wall_facing(&w.level.tiles);
            w.player.pos = lamp.pos + dir * 90.0;
            w.cam_focus = Some(lamp.pos + dir * 24.0);
        }
        w.lighting = LightingMode::Full;
        w.lighting_depth = ColorDepth::Truecolor;
        w.lighting_fx = true;
        w.smoke_enabled = false;
        w.elapsed = 12.0;

        // Two frozen renders at the same clock are bit-for-bit identical.
        let f1 = render_with(&mut w, GlobalLight::AoGi, (120, 80));
        let f2 = render_with(&mut w, GlobalLight::AoGi, (120, 80));
        assert_eq!(f1.pixels(), f2.pixels(), "frozen AoGi frames must be identical");

        // Move the rat's personal aura off-map so `gi_energy_at` isolates room
        // fixture GI across sconce flicker steps and blackout levels.
        w.player.pos = Vec2::new(1200.0, 1200.0);

        // Verify GI's own grid contribution is invariant across sconce flicker
        // steps (`t = 12.00` vs `t = 12.37`) and dims during a blackout (`t = 304.0`).
        let gi_energy_at = |world: &World, blackout: f32, elapsed: f32| -> u64 {
            let mut map_ao = LightMap::new();
            let mut map_gi = LightMap::new();
            let mut pb_ao = PixelBuffer::new(120, 80);
            let mut pb_gi = PixelBuffer::new(120, 80);
            pb_ao.set_materials(true);
            pb_gi.set_materials(true);
            pb_ao.clear((40, 44, 46));
            pb_gi.clear((40, 44, 46));
            let cam = world.cam;
            let base = LightScene {
                ecs: &world.ecs,
                level: &world.level,
                player: &world.player,
                particles: &world.particles,
                thrash: None,
                squeak: None,
                cam,
                elapsed,
                blackout,
                mode: LightingMode::Full,
                depth: ColorDepth::Truecolor,
                fx: true,
                gi: GlobalLight::AoOnly,
            };
            map_ao.apply_scene(&mut pb_ao, &base);
            map_gi.apply_scene(
                &mut pb_gi,
                &LightScene {
                    gi: GlobalLight::AoGi,
                    ..base
                },
            );
            pb_ao
                .pixels()
                .iter()
                .zip(pb_gi.pixels().iter())
                .map(|(a, g)| {
                    (i32::from(g.0) - i32::from(a.0)).max(0) as u64
                        + (i32::from(g.1) - i32::from(a.1)).max(0) as u64
                        + (i32::from(g.2) - i32::from(a.2)).max(0) as u64
                })
                .sum()
        };

        let powered_a = gi_energy_at(&w, 0.0, 12.00);
        let indoor_step_a = STATE.with(|c| c.borrow().static_gi.indoor.clone());
        let powered_b = gi_energy_at(&w, 0.0, 12.04); // within same step
        assert_eq!(powered_a, powered_b, "steady scene produces identical GI");
        assert!(powered_a > 100, "fixture GI must add measurable indirect light ({powered_a})");

        let _ = gi_energy_at(&w, 0.0, 12.37); // across a 3 Hz sconce step
        let indoor_step_b = STATE.with(|c| c.borrow().static_gi.indoor.clone());
        assert_eq!(
            indoor_step_a, indoor_step_b,
            "static GI field must not flicker across sconce 3 Hz steps"
        );

        let half_outage = gi_energy_at(&w, 0.5, 300.3);
        let full_outage = gi_energy_at(&w, 1.0, 304.0);
        assert!(
            half_outage < powered_a,
            "half blackout ({half_outage}) must dim GI below powered ({powered_a})"
        );
        assert!(
            full_outage < half_outage,
            "full blackout ({full_outage}) must dim GI further than half blackout ({half_outage})"
        );
        assert_eq!(full_outage, 0, "full blackout turns room-fixture GI off");
    }
}
