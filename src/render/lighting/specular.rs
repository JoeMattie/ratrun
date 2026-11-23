//! W4 — per-material specular (Blinn-Phong LUTs, the slime double lobe,
//! steel/glass/grease/ink/water) and the directional rim light on creatures
//! (contract §4, techniques 07 + 09), plus the material **tagging tables**
//! the scene draw stamps with (`World::draw_scene` sets these pens around
//! each sprite's draw call).
//!
//! `enemy_pen`/`boss_pen`/`rat_pen`/`prop_pen`/`pool_pen` give each kind its
//! [`material::MaterialKind`] and a height class (body height / 4 px), so the
//! pixel stage knows what a pixel is made of without touching the simulation.
//! Every kind is matched exhaustively: a new kind cannot be added without
//! choosing its material here.
//!
//! [`apply`] runs after the shade (`upsample::composite`), so it adds light to
//! the already-lit picture. For every pixel whose material has gloss the
//! highlight is `lobe[N·H] * spec energy * dominant colour`: the lobe is a
//! 256-entry table per material, built once per process, so the hot loop has
//! no `powf`. Creature pixels use a dome normal from the CREATURE mask and
//! their height class (the shared `normals` buffer is never written by W3 on a
//! creature pixel); every other pixel uses the bump normal W3 left there.
//! Highlights move with the lights and are exactly still when nothing moves —
//! nothing here reads a clock.
//!
//! On the perimeter of a body (a CREATURE pixel with a non-CREATURE
//! 4-neighbour) the same stage adds the rim: the outward silhouette normal
//! from a 16-entry mask LUT, `max(0, r·L)`, in the dominant light's colour,
//! reaching 1.4× a light's radius so a body emerges at the edge of the dark.
//! Where that rim runs (Full/Hard) [`keep_glints`] drops the old 1 px glint.

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{strength, Canvas, FxFrame, LightInfo, Normal, DIR_ONE};
use super::material::{self, pen, MaterialKind};
use crate::game::ecs::systems::boss::BossKind;
use crate::game::ecs::systems::miniboss::MinibossKind;
use crate::game::enemy::EnemyKind;
use crate::game::level::PropKind;
use crate::math::Vec2;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;
use std::sync::OnceLock;

// ============================================================================
// The pens: what each kind is made of
// ============================================================================

/// A miniboss body is 16–40 px long at a collision radius of 6.5–7.5
/// (`WAVE4_ROSTERS.md`): 20 px of body height is the middle of that range.
const MINIBOSS_HEIGHT: u8 = 5;
/// The rat's body is about 12 px tall.
const RAT_HEIGHT: u8 = 3;

/// Height class for a body `px` pixels tall: bits 5..=7 of the material byte,
/// body height / 4 px, never 0 (0 means "unknown"), never above 7.
fn height_class(body_height_px: f32) -> u8 {
    let class = (body_height_px / 4.0).round();
    if class < 1.0 {
        1
    } else if class > 7.0 {
        7
    } else {
        class as u8
    }
}

/// The pen for an ordinary enemy or (with `miniboss`) a named miniboss body.
pub(crate) fn enemy_pen(kind: EnemyKind, miniboss: Option<MinibossKind>) -> u8 {
    match miniboss {
        Some(body) => pen(miniboss_material(body), true, MINIBOSS_HEIGHT),
        None => {
            // The collision radius is the body's half-width; a body is about
            // two of those tall.
            let height = height_class(kind.radius() * 2.0);
            pen(enemy_material(kind), true, height)
        }
    }
}

/// The pen for a boss body (both the ground and the airborne pass).
pub(crate) fn boss_pen(kind: BossKind) -> u8 {
    let (material, height) = match kind {
        BossKind::Cat => (MaterialKind::Fur, 3),
        BossKind::Broom => (MaterialKind::Wood, 4),
        BossKind::Mower => (MaterialKind::Metal, 3),
        BossKind::Mangle => (MaterialKind::Metal, 4),
        BossKind::NightEdition => (MaterialKind::Metal, 4),
        BossKind::Magpie => (MaterialKind::Fur, 3),
        BossKind::HorseshoeCrab => (MaterialKind::Shell, 3),
        BossKind::Hawk => (MaterialKind::Fur, 4),
        BossKind::MegaRat => (MaterialKind::Fur, 6),
        BossKind::Cobra => (MaterialKind::Slime, 5),
    };
    pen(material, true, height)
}

/// The pen for the rat's body (tail and whip included).
pub(crate) fn rat_pen() -> u8 {
    pen(MaterialKind::Fur, true, RAT_HEIGHT)
}

/// The pen for the rat under the Growth Serum: the same fur on a body five
/// times the size, so its dome curves slowly instead of like the small rat's.
pub(crate) fn giant_rat_pen() -> u8 {
    pen(MaterialKind::Fur, true, 7)
}

/// The pen for a prop (tanks and lenses: Glass; pipes and housings: Metal; …).
pub(crate) fn prop_pen(kind: PropKind) -> u8 {
    pen(prop_material(kind), false, prop_height_class(kind))
}

/// A prop's height class, from its smaller footprint half (a squat crate is
/// low, a tank is tall): `surface` reads deeper relief on a raised prop.
fn prop_height_class(kind: PropKind) -> u8 {
    let (w, h) = kind.size();
    height_class(w.min(h) * 0.5)
}

/// The pen for an acid pool (Acid Spit's ground pool).
pub(crate) fn pool_pen() -> u8 {
    // There is no `Acid` kind: a pool is standing liquid, and `Water` is the
    // wet surface that catches the lamp.
    pen(MaterialKind::Water, false, 0)
}

/// What an ordinary enemy is made of. The materials are the contract's
/// reading of `WAVE4_ROSTERS.md`: wet and shiny bodies are Slime, chitin and
/// carapace are Shell, matte coats and wings are Fur (rim light only), keys,
/// cans, carts and clocks are Metal, glassware is Glass, paper, wool and
/// cloth are Cloth.
fn enemy_material(kind: EnemyKind) -> MaterialKind {
    match kind {
        // ---- legacy and wave 1–2 bodies -----------------------------------
        EnemyKind::Skitterer => MaterialKind::Shell,
        EnemyKind::Bat => MaterialKind::Fur,
        EnemyKind::Cat => MaterialKind::Fur,
        // A veined acid sac: wet.
        EnemyKind::Spitter => MaterialKind::Slime,
        // Plated rhino beetle.
        EnemyKind::Brute => MaterialKind::Shell,
        // The sculpted queen: mandibles, horns, stinger.
        EnemyKind::Boss => MaterialKind::Shell,
        EnemyKind::Pillbug => MaterialKind::Shell,
        // A copper tuning fork.
        EnemyKind::BellFlea => MaterialKind::Shell,
        // A moth: matte, rim only.
        EnemyKind::WickMoth => MaterialKind::Fur,
        // A green scarab.
        EnemyKind::Porter => MaterialKind::Shell,
        // A brass key drone.
        EnemyKind::Turnkey => MaterialKind::Metal,
        EnemyKind::BigSnake => MaterialKind::Slime,
        // A striped chitin comb.
        EnemyKind::HouseCentipede => MaterialKind::Shell,
        EnemyKind::MurderHornet => MaterialKind::Shell,
        EnemyKind::TermiteHorde => MaterialKind::Shell,
        // Three tiny upright coats.
        EnemyKind::Lilliputian => MaterialKind::Cloth,
        // ---- Sewer --------------------------------------------------------
        EnemyKind::PipeEel => MaterialKind::Slime,
        // A barnacle cone.
        EnemyKind::GrateArcher => MaterialKind::Shell,
        EnemyKind::SumpLeech => MaterialKind::Slime,
        EnemyKind::SluiceToad => MaterialKind::Slime,
        // ---- Kitchen ------------------------------------------------------
        EnemyKind::CrumbAnt => MaterialKind::Shell,
        EnemyKind::FruitFly => MaterialKind::Shell,
        // A tin clock.
        EnemyKind::EggTimer => MaterialKind::Metal,
        // A wooden pawn.
        EnemyKind::PepperMill => MaterialKind::Wood,
        // A crimped tin skirt.
        EnemyKind::PieWeight => MaterialKind::Metal,
        EnemyKind::SinkCrab => MaterialKind::Shell,
        // ---- Lab ----------------------------------------------------------
        // A cotton club.
        EnemyKind::SwabFoot => MaterialKind::Cloth,
        // A specimen lid.
        EnemyKind::SpinnerLid => MaterialKind::Glass,
        // A clear ampoule.
        EnemyKind::GlassRunner => MaterialKind::Glass,
        EnemyKind::WalkingSyringe => MaterialKind::Glass,
        EnemyKind::AutoclaveCart => MaterialKind::Metal,
        // ---- Laundry ------------------------------------------------------
        // Pale wool.
        EnemyKind::WoolPill => MaterialKind::Cloth,
        // A moth: matte, rim only.
        EnemyKind::ClothesMoth => MaterialKind::Fur,
        // A forked wooden jaw.
        EnemyKind::PegSnapper => MaterialKind::Wood,
        // A rubber pear.
        EnemyKind::StarchBulb => MaterialKind::Cloth,
        EnemyKind::LintGolem => MaterialKind::Cloth,
        // ---- Printworks ---------------------------------------------------
        EnemyKind::Silverfish => MaterialKind::Shell,
        // A torn slip of paper.
        EnemyKind::ErratumSlip => MaterialKind::Cloth,
        // A lead type block.
        EnemyKind::TypeSlug => MaterialKind::Metal,
        EnemyKind::StapleCrawler => MaterialKind::Metal,
        // A rolled newsprint abdomen.
        EnemyKind::PaperWasp => MaterialKind::Cloth,
        EnemyKind::PaperReel => MaterialKind::Cloth,
        // Ragged wet ink (the contract names it with the slimes).
        EnemyKind::InkBlot => MaterialKind::Slime,
        // ---- Flatbed ------------------------------------------------------
        // A squared pebble.
        EnemyKind::GravelTick => MaterialKind::Stone,
        EnemyKind::JuneBug => MaterialKind::Shell,
        EnemyKind::TarRat => MaterialKind::Fur,
        // A dented can.
        EnemyKind::LooseCan => MaterialKind::Metal,
        // A feathered gull.
        EnemyKind::TollGull => MaterialKind::Fur,
        // A J hook dragging a rubber loop.
        EnemyKind::LooseBungee => MaterialKind::Cloth,
        // ---- The guest ----------------------------------------------------
        // Feathers: matte (rim light only, no highlight), the one place a
        // white body must not blow out under a lamp.
        EnemyKind::Drake => MaterialKind::Fur,
    }
}

fn miniboss_material(kind: MinibossKind) -> MaterialKind {
    match kind {
        // A brass auger.
        MinibossKind::Corkscrew => MaterialKind::Metal,
        // Steel shears.
        MinibossKind::SisterSnip => MaterialKind::Metal,
        // A spindle rat.
        MinibossKind::Bobbin => MaterialKind::Fur,
        // Under a cracked bronze bell.
        MinibossKind::BellDiver => MaterialKind::Metal,
        // A rubber stamp on heavy boots: matte.
        MinibossKind::Foreman => MaterialKind::Cloth,
        // A waxed carton.
        MinibossKind::MilkCarton => MaterialKind::Cloth,
        // A wheel spider with a wrench foreleg.
        MinibossKind::ValveWidow => MaterialKind::Metal,
        // Wet mud and a mousetrap chassis.
        MinibossKind::Mudlark => MaterialKind::Slime,
        // A fluted tin coin.
        MinibossKind::PastryWheel => MaterialKind::Metal,
        // China: glazed.
        MinibossKind::SaltHen => MaterialKind::Tile,
        // A bristle fan on a jointed wooden handle.
        MinibossKind::PotBrush => MaterialKind::Wood,
        // A glass cylinder.
        MinibossKind::DosePump => MaterialKind::Glass,
        MinibossKind::PetriMother => MaterialKind::Glass,
        MinibossKind::CageCounter => MaterialKind::Metal,
        MinibossKind::RotorTechnician => MaterialKind::Metal,
        // A soap dome: wet.
        MinibossKind::SudsBailiff => MaterialKind::Slime,
        MinibossKind::WringerBoy => MaterialKind::Metal,
        MinibossKind::QuoinKey => MaterialKind::Metal,
        // Two offset sheets of paper.
        MinibossKind::CarbonTwin => MaterialKind::Cloth,
        MinibossKind::LatePlate => MaterialKind::Metal,
        // A folded tarp.
        MinibossKind::TarpaulinPup => MaterialKind::Cloth,
        MinibossKind::AxleNut => MaterialKind::Metal,
        MinibossKind::ChainDog => MaterialKind::Metal,
        // A gull with a map over its wings.
        MinibossKind::RoadAtlas => MaterialKind::Fur,
    }
}

fn prop_material(kind: PropKind) -> MaterialKind {
    match kind {
        PropKind::PipeV | PropKind::PipeH | PropKind::Valve | PropKind::Grate => {
            MaterialKind::Metal
        }
        PropKind::Table | PropKind::Crate | PropKind::Barrel | PropKind::Counter => {
            MaterialKind::Wood
        }
        // A console is a steel housing with a glass face: the housing is what
        // most of the silhouette is.
        PropKind::Console => MaterialKind::Metal,
        PropKind::Tank => MaterialKind::Glass,
        PropKind::Barrier => MaterialKind::Metal,
        PropKind::Washer => MaterialKind::Metal,
        PropKind::TowelRoll => MaterialKind::Cloth,
        PropKind::Clothespin => MaterialKind::Wood,
        PropKind::PressIsland => MaterialKind::Metal,
        PropKind::PaperReel => MaterialKind::Cloth,
        // Lead type.
        PropKind::TypeBlock => MaterialKind::Metal,
        PropKind::Pallet => MaterialKind::Wood,
        PropKind::TarpLump => MaterialKind::Cloth,
        // Rubber.
        PropKind::Tyre => MaterialKind::Cloth,
        PropKind::Toolbox => MaterialKind::Metal,
        PropKind::PipeStack => MaterialKind::Metal,
        PropKind::ChainCoil => MaterialKind::Metal,
    }
}

/// Keep the old 1 px enemy rim glints this frame? False where [`apply`]'s
/// directional rim replaces them (Full/Hard).
pub(super) fn keep_glints(mode: LightingMode) -> bool {
    !matches!(mode, LightingMode::Full | LightingMode::Hard)
}

// ============================================================================
// The lobe tables: one 256-entry highlight curve per material
// ============================================================================

/// `MaterialKind` values, 0..=15.
const KINDS: usize = 16;
/// Entries per lobe table: `N·H` from 0 to 1.
const LUT: usize = 256;
/// Below this gloss a material is matte: it gets no highlight at all (fur 12,
/// stone 10, cloth 4), so a matte body shows only the rim light.
const GLOSS_FLOOR: u8 = 16;
/// The slime's double lobe: a broad wet sheen (`shininess / 8`, so the 80 of
/// the table becomes 10 — a wide wash) plus a sharp pinpoint glint
/// (`shininess * 2`). Both are baked into the one 256-entry curve, which is
/// why it stays monotonic.
const SLIME_SHEEN_DIV: f32 = 8.0;
const SLIME_SHEEN: f32 = 0.55;
const SLIME_GLINT_MUL: f32 = 2.0;
const SLIME_GLINT: f32 = 0.75;
/// The half vector's view direction (`relight_core.py`; slightly off axis so a
/// flat floor still catches a grazing lamp).
const VIEW: [f32; 3] = [0.0, 0.24, 0.97];
/// The largest specular-eligible energy one sample can hand a highlight, in
/// `Q8.8` (4 levels above the brightest lamp at its core).
const SPEC_ENERGY_CAP: i32 = 384;
/// `lobe * energy * colour >> SPEC_SHIFT` — the highlight's amount in pixel
/// levels. Chosen so a lit slime reads about +25, a lamp's core about +75.
const SPEC_SHIFT: u32 = 15;
/// No single pixel of highlight may add more than this, in levels toward
/// white: glass still gets its hard dot and nothing blows out.
const MAX_ADD: i32 = 132;
/// Creepy emergence: the rim reads out to this multiple of a light's radius.
const RIM_REACH: f32 = 1.4;
/// The rim just outside a pool is this share of the light's own gain (the pool
/// itself, inside the radius, is brighter: `info.dom`).
const RIM_REACH_GAIN: f32 = 0.35;
/// The largest rim energy one sample can hand the rim light, in `Q8.8`.
const RIM_ENERGY_CAP: i32 = 200;
/// `facing * energy * colour >> RIM_SHIFT`, clamped to [`MAX_RIM`].
const RIM_SHIFT: u32 = 21;
const MAX_RIM: i32 = 84;
/// No rim contribution below this many `Q8.8` units is worth painting.
const RIM_MIN_ENERGY: i32 = 2;
/// How many pixels either side of a creature pixel the dome normal reads the
/// CREATURE mask at (see [`dome_normal`]).
const DOME_TAP: i32 = 3;
/// A creature pixel with no height class still gets the smallest dome.
const DOME_MIN_HEIGHT_PX: i32 = 8;

/// `fx::strength::SPECULAR` / `RIM` in `Q8.8`.
const SPEC_Q8: i32 = (strength::SPECULAR * 256.0) as i32;
const RIM_Q8: i32 = (strength::RIM * 256.0) as i32;

/// The lobe tables, built once per process (never per frame).
struct Tables {
    /// Highlight curve per `MaterialKind`, indexed by `N·H` in 0..=255.
    lobe: [[u8; LUT]; KINDS],
    /// `lobe[kind][255]`: 0 for every matte material (and `Emissive`).
    peak: [u8; KINDS],
    /// [`Tables::peak`] with only the materials `Hard` draws (Slime, Ink,
    /// Metal) left in, so the tier test costs one array read per pixel.
    hard_peak: [u8; KINDS],
    /// The up component of a Q0.7 tangent normal, indexed by
    /// `(nx² + ny²) >> 7` (exact at the steep end, where a parabola is not).
    nz: [u8; 256],
}

static TABLES: OnceLock<Tables> = OnceLock::new();

fn tables() -> &'static Tables {
    TABLES.get_or_init(build_tables)
}

fn build_tables() -> Tables {
    let mut tables = Tables {
        lobe: [[0u8; LUT]; KINDS],
        peak: [0u8; KINDS],
        hard_peak: [0u8; KINDS],
        nz: [0u8; 256],
    };
    for (i, v) in tables.nz.iter_mut().enumerate() {
        let t = (i as f32) * 128.0;
        *v = (16129.0 - t).max(0.0).sqrt().round() as u8;
    }
    for (k, row) in tables.lobe.iter_mut().enumerate() {
        let params = material::params(MaterialKind::from_bits(k as u8));
        if params.gloss < GLOSS_FLOOR {
            continue;
        }
        let gloss = f32::from(params.gloss) / 255.0;
        let sharp = f32::from(params.shininess).max(1.0);
        for (i, value) in row.iter_mut().enumerate() {
            let cos = i as f32 / (LUT - 1) as f32;
            let lobe = if params.double_lobe {
                cos.powf(sharp / SLIME_SHEEN_DIV) * SLIME_SHEEN * gloss
                    + cos.powf(sharp * SLIME_GLINT_MUL) * SLIME_GLINT * gloss
            } else {
                cos.powf(sharp) * gloss
            };
            *value = (lobe * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    for k in 0..KINDS {
        let peak = tables.lobe[k][LUT - 1];
        tables.peak[k] = peak;
        tables.hard_peak[k] = if matches!(
            MaterialKind::from_bits(k as u8),
            MaterialKind::Slime | MaterialKind::Ink | MaterialKind::Metal
        ) {
            peak
        } else {
            0
        };
    }
    tables
}

/// Energy for a highlight or a rim, in `Q8.8` times the tier's strength.
fn scaled(energy_q8: i32, strength_q8: i32, cap: i32) -> i32 {
    if energy_q8 <= 0 || strength_q8 <= 0 {
        return 0;
    }
    (energy_q8.min(cap) * strength_q8) >> 8
}

/// Additive light toward white, the same blend the light samples use: a
/// highlight lifts a dark pixel far more than a bright one, so detail survives.
#[inline]
fn lift(pixel: u8, add: i32) -> u8 {
    let add = add.clamp(0, 255);
    (i32::from(pixel) + (((255 - i32::from(pixel)) * add) >> 8)) as u8
}

/// One material's highlight for a sample, in pixel levels per channel.
#[inline]
fn highlight_add(lobe: i32, energy: i32, colour: Rgb) -> [i32; 3] {
    if lobe <= 0 || energy <= 0 {
        return [0; 3];
    }
    let channels = [colour.0, colour.1, colour.2];
    let mut out = [0; 3];
    for (n, channel) in channels.into_iter().enumerate() {
        out[n] = ((lobe * energy * i32::from(channel)) >> SPEC_SHIFT).min(MAX_ADD);
    }
    out
}

/// The lamp's colour pulled [`WET_WHITE`]/256 of the way to white.
const WET_WHITE: i32 = 160;

#[inline]
fn wet_white(c: Rgb) -> Rgb {
    let mix = |v: u8| (i32::from(v) + (((255 - i32::from(v)) * WET_WHITE) >> 8)) as u8;
    (mix(c.0), mix(c.1), mix(c.2))
}

/// Post-composite pixel stage: specular highlights by material and the rim
/// light on CREATURE perimeter pixels (`fx::strength::SPECULAR`, `RIM`).
pub(super) fn apply(grid: &Grid, canvas: &mut Canvas<'_>, normals: &[Normal], frame: &FxFrame<'_>) {
    run(grid, canvas, normals, frame, SPEC_Q8, RIM_Q8);
}

/// [`apply`] with the two strength factors passed in, so the tests can run the
/// stage at zero strength and prove the picture does not move.
fn run(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    normals: &[Normal],
    frame: &FxFrame<'_>,
    spec_q8: i32,
    rim_q8: i32,
) {
    let mode = frame.mode;
    // §5: specular on Full (all materials) and Hard (Slime, Ink, Metal only),
    // off below; the rim only where it replaces the 1 px glint.
    if !matches!(mode, LightingMode::Full | LightingMode::Hard) {
        return;
    }
    let (w, h) = (canvas.w, canvas.h);
    let stride = grid.stride;
    if w == 0 || h == 0 || stride == 0 || grid.w == 0 {
        return;
    }
    // At Ansi256 the shade has already been dithered, so the stage's own
    // output is kept in flat steps rather than a gradient of single levels.
    let low_colour = frame.depth != ColorDepth::Truecolor;
    let hard = mode == LightingMode::Hard;
    let tables = tables();
    let theme = frame.theme;
    let mat: &[u8] = canvas.mat;
    let pixels: &mut [Rgb] = &mut *canvas.px;
    // The ink pads in view, as screen rectangles (the Printworks only).
    const MAX_PADS: usize = 24;
    let mut pads = [(0i32, 0i32, 0i32, 0i32); MAX_PADS];
    let mut npads = 0;
    for pad in &frame.level.objects.ink_pads {
        if npads >= MAX_PADS {
            break;
        }
        let p = pad.pos - frame.cam;
        let r = (
            (p.x - pad.half.x).floor() as i32,
            (p.y - pad.half.y).floor() as i32,
            (p.x + pad.half.x).ceil() as i32,
            (p.y + pad.half.y).ceil() as i32,
        );
        if r.0 < w as i32 && r.2 > 0 && r.1 < h as i32 && r.3 > 0 {
            pads[npads] = r;
            npads += 1;
        }
    }
    // The first run of a row ends at the next sample boundary.
    let first_run = (stride - grid.offset.0 % stride).min(w);
    let first_sample = grid.offset.0 / stride;
    for y in 0..h {
        let sy = (y + grid.offset.1) / stride;
        if sy >= grid.h {
            break;
        }
        let row = y * w;
        let mut x = 0usize;
        let mut sx = first_sample;
        let mut run_len = first_run;
        while x < w && run_len > 0 {
            let end = (x + run_len).min(w);
            let si = sy * grid.w + sx;
            let info = grid.info.get(si).copied().unwrap_or_default();
            let wall = grid
                .surfaces
                .get(si)
                .is_some_and(|s| s.layer & LayerMask::WALLS.0 != 0);
            // Once per run: the half vector and the energy the whole run's
            // highlights share.
            let (half, energy) = match half_vector(&info) {
                Some(half) => (half, scaled(i32::from(info.spec), spec_q8, SPEC_ENERGY_CAP)),
                None => ([0; 3], 0),
            };
            // §2.3: an ink pad is untagged floor, read from the level.
            let on_ink = !wall && npads > 0 && {
                let cx = (sx * stride) as i32 - grid.offset.0 as i32 + stride as i32 / 2;
                let cy = y as i32;
                pads[..npads]
                    .iter()
                    .any(|r| cx >= r.0 && cx < r.2 && cy >= r.1 && cy < r.3)
            };
            let floor_kind = if on_ink {
                MaterialKind::Ink as usize
            } else {
                material::resolve(0, theme, wall) as usize
            };
            // Looked up on the run's first creature pixel: most runs have none.
            let mut rim: Option<Option<Rim>> = None;
            for px_index in x..end {
                let i = row + px_index;
                let byte = mat.get(i).copied().unwrap_or(0);
                let creature = material::is_creature(byte);
                let mut add = [0i32; 3];
                let mut rim_add = [0i32; 3];
                // ---- the per-material highlight --------------------------
                if energy > 0 {
                    let own = material::kind(byte);
                    let slot = if own == MaterialKind::Default {
                        floor_kind
                    } else {
                        own as usize
                    };
                    // §5: Full draws every material, Hard only the glossy three.
                    let peak = if hard {
                        tables.hard_peak[slot]
                    } else {
                        tables.peak[slot]
                    };
                    if peak > 0 {
                        let normal = if creature {
                            dome_normal(mat, w, h, px_index, y, byte)
                        } else {
                            normals.get(i).copied().unwrap_or([0, 0])
                        };
                        // A flat, untagged face (a slab of floor or wall) takes
                        // no sheen: its highlight would be one value for the
                        // whole 4 px sample, and near a small light that
                        // painted glowing slab-shaped boxes. The relief (grout,
                        // cracks, edges) still catches the light.
                        let flat_face =
                            !creature && own == MaterialKind::Default && normal == [0, 0];
                        if !flat_face {
                            let (nx, ny) = (i32::from(normal[0]), i32::from(normal[1]));
                            let nz = i32::from(
                                tables.nz[((nx * nx + ny * ny) >> 7).min(255) as usize],
                            );
                            let dot = nx * half[0] + ny * half[1] + nz * half[2];
                            let index = (dot >> 7).clamp(0, (LUT - 1) as i32) as usize;
                            // A wet body's glint is the lamp's image, not its
                            // wash: pulled most of the way to white, so it
                            // shows on a body the same lamp already tints.
                            let colour = if slot == MaterialKind::Slime as usize {
                                wet_white(info.dom_rgb)
                            } else {
                                info.dom_rgb
                            };
                            add = highlight_add(
                                i32::from(tables.lobe[slot][index]),
                                energy,
                                colour,
                            );
                        }
                    }
                }
                // ---- the rim on a creature's silhouette ------------------
                if creature {
                    let rim =
                        rim.get_or_insert_with(|| rim_light(grid, frame, &info, sx, y, rim_q8));
                    if let Some(rim) = rim {
                        let free = mask_free(mat, w, h, px_index, y);
                        if free != 0 {
                            let r = RIM_NORMAL[free as usize];
                            let facing = (i32::from(r[0]) * rim.dir[0]
                                + i32::from(r[1]) * rim.dir[1])
                                >> 7;
                            if facing > 0 {
                                let channels = [rim.rgb.0, rim.rgb.1, rim.rgb.2];
                                for (n, channel) in channels.into_iter().enumerate() {
                                    rim_add[n] = ((facing * rim.energy * i32::from(channel))
                                        >> RIM_SHIFT)
                                        .min(MAX_RIM);
                                }
                            }
                        }
                    }
                }
                let mut any = false;
                for (slot, rim) in add.iter_mut().zip(rim_add) {
                    let mut a = (*slot).max(0) + rim.max(0);
                    if low_colour {
                        a &= !7;
                    }
                    *slot = a.min(MAX_ADD);
                    any |= *slot > 0;
                }
                if any {
                    let pixel = pixels[i];
                    pixels[i] = (
                        lift(pixel.0, add[0]),
                        lift(pixel.1, add[1]),
                        lift(pixel.2, add[2]),
                    );
                }
            }
            x = end;
            sx += 1;
            run_len = stride.min(w - x);
        }
    }
}

/// The Blinn half vector `normalize(L + V)` of a sample's dominant light, in
/// `Q8` (256 = 1.0), or `None` when the sample has no dominant light.
fn half_vector(info: &LightInfo) -> Option<[i32; 3]> {
    if info.dir == [0, 0, 0] || info.dom == 0 {
        return None;
    }
    let scale = 1.0 / f32::from(DIR_ONE);
    let light = [
        f32::from(info.dir[0]) * scale,
        f32::from(info.dir[1]) * scale,
        f32::from(info.dir[2]) * scale,
    ];
    let half = [
        light[0] + VIEW[0],
        light[1] + VIEW[1],
        light[2] + VIEW[2],
    ];
    let len = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    if len <= 0.001 || len.is_nan() {
        return None;
    }
    let k = 256.0 / len;
    Some([
        (half[0] * k) as i32,
        (half[1] * k) as i32,
        (half[2] * k) as i32,
    ])
}

/// A creature pixel's dome normal (Q0.7): the CREATURE mask's own gradient at
/// [`DOME_TAP`] pixels, scaled by the body's height class. A small body curves
/// sharply over 6 px and a big one is nearly flat there, so the same tap gives
/// `127 * 2 · tap / height` of tilt — a wet sheen that slides to the lit side
/// as the light moves. A CREATURE pixel is never written by W3, so this is the
/// only normal a body's highlight has.
#[inline]
fn dome_normal(mat: &[u8], w: usize, h: usize, x: usize, y: usize, byte: u8) -> Normal {
    let height = i32::from(material::height(byte)) * 4;
    let height = if height == 0 {
        DOME_MIN_HEIGHT_PX
    } else {
        height
    };
    let tilt = ((127 * 2 * DOME_TAP) / height).clamp(0, 127);
    let gx = mask_at(mat, w, h, x as i32 + DOME_TAP, y as i32)
        - mask_at(mat, w, h, x as i32 - DOME_TAP, y as i32);
    let gy = mask_at(mat, w, h, x as i32, y as i32 + DOME_TAP)
        - mask_at(mat, w, h, x as i32, y as i32 - DOME_TAP);
    let nx = (-gx * tilt).clamp(-127, 127);
    let ny = (-gy * tilt).clamp(-127, 127);
    [nx as i8, ny as i8]
}

/// Is this pixel a creature? Off the canvas counts as floor, so a body at the
/// screen edge still has a silhouette there.
#[inline]
fn mask_at(mat: &[u8], w: usize, h: usize, x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return 0;
    }
    i32::from(material::is_creature(
        mat.get(y as usize * w + x as usize).copied().unwrap_or(0),
    ))
}

/// Which of the four neighbours are **not** creatures, as the bit mask
/// [`RIM_NORMAL`] indexes: bit 0 west, 1 north, 2 east, 3 south.
#[inline]
fn mask_free(mat: &[u8], w: usize, h: usize, x: usize, y: usize) -> u8 {
    let (xi, yi) = (x as i32, y as i32);
    let mut free = 0u8;
    if mask_at(mat, w, h, xi - 1, yi) == 0 {
        free |= 1;
    }
    if mask_at(mat, w, h, xi, yi - 1) == 0 {
        free |= 2;
    }
    if mask_at(mat, w, h, xi + 1, yi) == 0 {
        free |= 4;
    }
    if mask_at(mat, w, h, xi, yi + 1) == 0 {
        free |= 8;
    }
    free
}

/// The outward silhouette normal per free-neighbour mask (Q0.7): the sum of
/// the free directions, normalised by how many there are, so a corner gives a
/// 45° normal of 90 and a lone pixel has none.
const fn rim_normal_lut() -> [[i8; 2]; 16] {
    let mut out = [[0i8; 2]; 16];
    let mut m = 0usize;
    while m < 16 {
        let mask = m as u8;
        let (mut x, mut y, mut n) = (0i32, 0i32, 0i32);
        if mask & 1 != 0 {
            x -= 1;
            n += 1;
        }
        if mask & 2 != 0 {
            y -= 1;
            n += 1;
        }
        if mask & 4 != 0 {
            x += 1;
            n += 1;
        }
        if mask & 8 != 0 {
            y += 1;
            n += 1;
        }
        if n > 0 {
            out[m] = [((x * 127) / n) as i8, ((y * 127) / n) as i8];
        }
        m += 1;
    }
    out
}

static RIM_NORMAL: [[i8; 2]; 16] = rim_normal_lut();

/// The light a run's rim uses: its 2D direction in `Q12`, its colour, and its
/// energy in `Q8.8`.
struct Rim {
    dir: [i32; 2],
    rgb: Rgb,
    energy: i32,
}

/// The rim light of one run. Inside a pool it is the sample's own dominant
/// light, already shadowed by the grid, so the rim never lights through a
/// wall. Just outside every pool it is the nearest strong light, faded in over
/// the outer 40 % of [`RIM_REACH`] times its radius, so a body emerges at the
/// edge of the dark.
fn rim_light(
    grid: &Grid,
    frame: &FxFrame<'_>,
    info: &LightInfo,
    sx: usize,
    y: usize,
    rim_q8: i32,
) -> Option<Rim> {
    if rim_q8 <= 0 {
        return None;
    }
    if info.dir != [0, 0, 0] && info.dom > 0 {
        let energy = scaled(i32::from(info.dom), rim_q8, RIM_ENERGY_CAP);
        if energy <= 0 {
            return None;
        }
        return Some(Rim {
            dir: [i32::from(info.dir[0]), i32::from(info.dir[1])],
            rgb: info.dom_rgb,
            energy,
        });
    }
    // The run's block, in world pixels.
    let point = frame.cam
        + Vec2::new(
            (sx * grid.stride) as f32 - grid.offset.0 as f32 + grid.stride as f32 * 0.5,
            y as f32 + 0.5,
        );
    let mut best: Option<Rim> = None;
    let mut best_energy = RIM_MIN_ENERGY;
    for light in frame.lights {
        let reach = light.extent * RIM_REACH;
        if !reach.is_finite() || reach <= 0.0 {
            continue;
        }
        let offset = light.pos - point;
        let distance = offset.len();
        // Inside the light's own reach the grid's `info` decides (and it knows
        // the shadows); only the band outside it is this stage's own guess.
        if distance <= light.extent || distance > reach {
            continue;
        }
        let q = distance / reach;
        let energy = light.gain * (1.0 - q * q) * RIM_REACH_GAIN;
        if energy <= 1.0 || energy.is_nan() {
            continue;
        }
        let energy = scaled(energy as i32, rim_q8, RIM_ENERGY_CAP);
        if energy <= best_energy {
            continue;
        }
        // A cheap wall guard for the fallback: a light whose midpoint with the
        // body is a wall sample does not reach it.
        if wall_between(grid, frame, point, light.pos) {
            continue;
        }
        let k = 4096.0 / distance;
        best = Some(Rim {
            dir: [(offset.x * k) as i32, (offset.y * k) as i32],
            rgb: light.color,
            energy,
        });
        best_energy = energy;
    }
    best
}

/// Is the grid sample halfway between two world points a wall? The one-tap
/// test the rim's outside-the-pool fallback uses (the pool's own light is
/// already shadowed by the grid).
fn wall_between(grid: &Grid, frame: &FxFrame<'_>, a: Vec2, b: Vec2) -> bool {
    let mid = (a + b) * 0.5 - frame.cam;
    let (px, py) = (mid.x.round(), mid.y.round());
    if px < 0.0 || py < 0.0 {
        return false;
    }
    let (px, py) = (px as usize, py as usize);
    if px >= (grid.w * grid.stride).max(1) || py >= (grid.h * grid.stride).max(1) {
        return false;
    }
    let i = grid.sample_index(px, py);
    grid.surfaces
        .get(i)
        .is_some_and(|s| s.layer & LayerMask::WALLS.0 != 0)
}

#[cfg(test)]
mod tests {
    use super::super::engine::{Grid, LightingMode};
    use super::super::fx::{testkit, DIR_ONE};
    use super::super::material::{self, pen, MaterialKind};
    use super::{
        boss_pen, enemy_pen, keep_glints, pool_pen, prop_pen, rat_pen, run, tables, KINDS, LUT,
        RAT_HEIGHT, RIM_Q8, SPEC_Q8,
    };
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::ecs::systems::miniboss::MinibossKind;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Level, PropKind, Theme};
    use crate::math::Vec2;
    use crate::render::framebuffer::PixelBuffer;
    use crate::render::palette::Rgb;
    use std::ops::Range;

    /// Every prop kind (there is no `PropKind::all()`).
    const PROPS: [PropKind; 23] = [
        PropKind::PipeV,
        PropKind::PipeH,
        PropKind::Valve,
        PropKind::Grate,
        PropKind::Table,
        PropKind::Crate,
        PropKind::Barrel,
        PropKind::Counter,
        PropKind::Console,
        PropKind::Tank,
        PropKind::Barrier,
        PropKind::Washer,
        PropKind::TowelRoll,
        PropKind::Clothespin,
        PropKind::PressIsland,
        PropKind::PaperReel,
        PropKind::TypeBlock,
        PropKind::Pallet,
        PropKind::TarpLump,
        PropKind::Tyre,
        PropKind::Toolbox,
        PropKind::PipeStack,
        PropKind::ChainCoil,
    ];

    /// The sum of every brightening this stage caused over a rectangle, in
    /// pixel levels.
    fn gain(off: &PixelBuffer, on: &PixelBuffer, x: Range<usize>, y: Range<usize>) -> i64 {
        let mut total = 0i64;
        for py in y {
            for px in x.clone() {
                let (a, b) = (off.pixel_at(px, py), on.pixel_at(px, py));
                for (from, to) in [(a.0, b.0), (a.1, b.1), (a.2, b.2)] {
                    total += i64::from(i32::from(to) - i32::from(from)).max(0);
                }
            }
        }
        total
    }

    #[test]
    fn every_kind_has_a_pen() {
        for kind in EnemyKind::all() {
            let b = enemy_pen(kind, None);
            assert!(material::is_creature(b), "{kind:?} is a body");
            assert_ne!(material::kind(b), MaterialKind::Default, "{kind:?}");
            assert!(material::height(b) >= 1, "{kind:?} has a height class");
        }
        for kind in MinibossKind::all() {
            let b = enemy_pen(EnemyKind::Skitterer, Some(kind));
            assert!(material::is_creature(b), "{kind:?} is a body");
            assert_ne!(material::kind(b), MaterialKind::Default, "{kind:?}");
            assert!(material::height(b) >= 1, "{kind:?} has a height class");
        }
        for kind in BossKind::all() {
            let b = boss_pen(kind);
            assert!(material::is_creature(b), "{kind:?} is a body");
            assert_ne!(material::kind(b), MaterialKind::Default, "{kind:?}");
            assert!(material::height(b) >= 1, "{kind:?} has a height class");
        }
        for kind in PROPS {
            let b = prop_pen(kind);
            assert!(!material::is_creature(b), "a prop is scenery, not a body");
            assert_ne!(material::kind(b), MaterialKind::Default, "every prop has a material");
        }
        let pool = pool_pen();
        assert!(!material::is_creature(pool));
        assert_eq!(material::kind(pool), MaterialKind::Water);
        let rat = rat_pen();
        assert!(material::is_creature(rat));
        assert_eq!(material::kind(rat), MaterialKind::Fur);
        assert_eq!(material::height(rat), RAT_HEIGHT);
    }

    #[test]
    fn the_contract_materials_land_on_their_kinds() {
        let slimy = [
            EnemyKind::SumpLeech,
            EnemyKind::SluiceToad,
            EnemyKind::PipeEel,
            EnemyKind::InkBlot,
            EnemyKind::BigSnake,
        ];
        for kind in slimy {
            assert_eq!(
                material::kind(enemy_pen(kind, None)),
                MaterialKind::Slime,
                "{kind:?}"
            );
        }
        assert_eq!(
            material::kind(boss_pen(BossKind::Cobra)),
            MaterialKind::Slime
        );
        for kind in [
            EnemyKind::Skitterer,
            EnemyKind::Brute,
            EnemyKind::Pillbug,
            EnemyKind::SinkCrab,
            EnemyKind::HouseCentipede,
        ] {
            assert_eq!(
                material::kind(enemy_pen(kind, None)),
                MaterialKind::Shell,
                "{kind:?}"
            );
        }
        for kind in [
            EnemyKind::Cat,
            EnemyKind::WickMoth,
            EnemyKind::ClothesMoth,
            EnemyKind::TarRat,
        ] {
            assert_eq!(
                material::kind(enemy_pen(kind, None)),
                MaterialKind::Fur,
                "{kind:?}"
            );
        }
        for kind in [EnemyKind::Turnkey, EnemyKind::LooseCan, EnemyKind::AutoclaveCart] {
            assert_eq!(
                material::kind(enemy_pen(kind, None)),
                MaterialKind::Metal,
                "{kind:?}"
            );
        }
        for kind in [BossKind::Mower, BossKind::Mangle, BossKind::NightEdition] {
            assert_eq!(material::kind(boss_pen(kind)), MaterialKind::Metal, "{kind:?}");
        }
        for kind in [
            EnemyKind::GlassRunner,
            EnemyKind::WalkingSyringe,
            EnemyKind::SpinnerLid,
        ] {
            assert_eq!(
                material::kind(enemy_pen(kind, None)),
                MaterialKind::Glass,
                "{kind:?}"
            );
        }
        for kind in [PropKind::Tank, PropKind::PipeV, PropKind::Toolbox] {
            assert!(!material::is_creature(prop_pen(kind)), "a prop is scenery");
        }
        assert_eq!(material::kind(prop_pen(PropKind::Tank)), MaterialKind::Glass);
        assert_eq!(material::kind(prop_pen(PropKind::PipeV)), MaterialKind::Metal);
        for kind in [
            EnemyKind::WoolPill,
            EnemyKind::ErratumSlip,
            EnemyKind::PaperReel,
            EnemyKind::Lilliputian,
        ] {
            assert_eq!(
                material::kind(enemy_pen(kind, None)),
                MaterialKind::Cloth,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn keep_glints_where_the_rim_does_not_run() {
        assert!(!keep_glints(LightingMode::Full));
        assert!(!keep_glints(LightingMode::Hard));
        assert!(keep_glints(LightingMode::Coarse));
        assert!(keep_glints(LightingMode::Compact));
    }

    #[test]
    fn lobe_tables_are_monotonic_and_bounded() {
        let tables = tables();
        for k in 0..KINDS {
            assert_eq!(tables.lobe[k][0], 0, "a grazing normal has no highlight");
            for i in 1..LUT {
                assert!(
                    tables.lobe[k][i] >= tables.lobe[k][i - 1],
                    "material {k} is monotonic in N·H"
                );
            }
            assert_eq!(tables.peak[k], tables.lobe[k][LUT - 1]);
        }
        // Matte materials are skipped outright, so a fur body gets no highlight.
        assert_eq!(tables.peak[MaterialKind::Fur as usize], 0);
        assert_eq!(tables.peak[MaterialKind::Stone as usize], 0);
        assert_eq!(tables.peak[MaterialKind::Cloth as usize], 0);
        assert_eq!(tables.peak[MaterialKind::Emissive as usize], 0);
        assert!(tables.peak[MaterialKind::Slime as usize] > 0);
        assert!(tables.peak[MaterialKind::Glass as usize] > 0);
        // A slime's sheen is broad: it holds up well away from the mirror angle.
        assert!(tables.lobe[MaterialKind::Slime as usize][200] > 0);
        // Glass is a tiny hard dot.
        assert_eq!(tables.lobe[MaterialKind::Glass as usize][200], 0);
    }

    /// The state a real `Grid::splat` leaves in `info` for one light, written by
    /// hand so these tests measure this stage **alone**: the rest of the pass
    /// (tone, bounce, bump normals, shafts, reflections) belongs to the other
    /// workers' files and moves pixels of its own. Mirrors `fx::note_light`:
    /// `dir` is surface → light in Q12, `dom`/`spec` the contribution in Q8.8.
    fn splat_light(grid: &mut Grid, pos: Vec2, height: f32, radius: f32, rgb: Rgb, energy: f32) {
        let stride = grid.stride;
        let half = stride as f32 * 0.5;
        let peak = f32::from(rgb.0.max(rgb.1).max(rgb.2));
        for sy in 0..grid.h {
            for sx in 0..grid.w {
                let point = Vec2::new(
                    (sx * stride) as f32 + half,
                    (sy * stride) as f32 + half,
                );
                let d2 = (point.x - pos.x) * (point.x - pos.x)
                    + (point.y - pos.y) * (point.y - pos.y);
                if d2 >= radius * radius {
                    continue;
                }
                let w = 1.0 - d2 / (radius * radius);
                let contribution = w * w * peak * energy;
                if contribution < 1.0 {
                    continue;
                }
                let toward = pos - point;
                let len = (d2 + height * height).sqrt().max(0.001);
                let k = f32::from(DIR_ONE) / len;
                let info = &mut grid.info[sy * grid.w + sx];
                info.dir = [
                    (toward.x * k) as i16,
                    (toward.y * k) as i16,
                    (height * k) as i16,
                ];
                info.dom = contribution.min(f32::from(u16::MAX)) as u16;
                info.spec = info.dom;
                info.dom_rgb = rgb;
            }
        }
    }

    /// A cleared world buffer with the material bytes on, like a frame's.
    fn blank(size: (usize, usize)) -> PixelBuffer {
        let mut pb = PixelBuffer::new(size.0, size.1);
        pb.set_materials(true);
        pb.set_pen(0);
        pb.clear((84, 72, 78));
        pb
    }

    /// Paint a body with its pen, so the material byte says what it is.
    fn paint_body(pb: &mut PixelBuffer, kind: MaterialKind, rect: (i32, i32, i32, i32), colour: Rgb) {
        let prev = pb.set_pen(pen(kind, true, 2));
        pb.rect_fill(rect.0, rect.1, rect.2, rect.3, colour);
        pb.set_pen(prev);
    }

    /// Run this stage alone over `pb`, at `mode` and `elapsed`.
    fn stage(pb: &mut PixelBuffer, grid: &Grid, level: &Level, mode: LightingMode, elapsed: f32) {
        let mut frame = testkit::frame(level, mode);
        frame.elapsed = elapsed;
        frame.step8 = (elapsed * 8.0) as u32;
        let normals = vec![[0i8, 0i8]; pb.w * pb.h];
        let mut canvas = testkit::canvas(pb);
        run(grid, &mut canvas, &normals, &frame, SPEC_Q8, RIM_Q8);
    }

    /// Is every pixel of a rectangle the same in both buffers?
    fn same(a: &PixelBuffer, b: &PixelBuffer, x: Range<usize>, y: Range<usize>) -> bool {
        for py in y {
            for px in x.clone() {
                if a.pixel_at(px, py) != b.pixel_at(px, py) {
                    return false;
                }
            }
        }
        true
    }

    /// The two lights these tests use, as a hand-written grid each: one to the
    /// west of the body, one to the east. Same colour and gain.
    fn west_light() -> Grid {
        let mut grid = testkit::flat_grid(16, 12, 4, [140, 140, 140]);
        splat_light(&mut grid, Vec2::new(6.0, 24.0), 20.0, 70.0, (238, 228, 208), 1.5);
        grid
    }

    fn east_light() -> Grid {
        let mut grid = testkit::flat_grid(16, 12, 4, [140, 140, 140]);
        splat_light(&mut grid, Vec2::new(58.0, 24.0), 20.0, 70.0, (238, 228, 208), 1.5);
        grid
    }

    fn no_light() -> Grid {
        testkit::flat_grid(16, 12, 4, [140, 140, 140])
    }

    /// The body these pixel tests paint: x 24..40, y 18..32.
    const BODY: (i32, i32, i32, i32) = (24, 18, 16, 14);

    fn body_left() -> Range<usize> {
        24..32
    }

    fn body_right() -> Range<usize> {
        32..40
    }

    fn body_rows() -> Range<usize> {
        18..32
    }

    #[test]
    fn a_slime_highlight_moves_with_the_light_and_needs_one() {
        let level = Level::empty(Theme::Sewer);
        let shoot = |grid: &Grid| {
            let mut pb = blank((64, 48));
            paint_body(&mut pb, MaterialKind::Slime, BODY, (52, 78, 52));
            stage(&mut pb, grid, &level, LightingMode::Full, 1.0);
            pb
        };
        let dark = shoot(&no_light());
        let west = shoot(&west_light());
        let east = shoot(&east_light());
        let west_left = gain(&dark, &west, body_left(), body_rows());
        let west_right = gain(&dark, &west, body_right(), body_rows());
        let east_left = gain(&dark, &east, body_left(), body_rows());
        let east_right = gain(&dark, &east, body_right(), body_rows());
        assert!(west_left + west_right > 0, "a lit slime has a sheen");
        assert!(
            west_left > west_right,
            "the sheen sits toward a light to the west ({west_left} vs {west_right})"
        );
        assert!(
            east_right > east_left,
            "and follows it to the east ({east_right} vs {east_left})"
        );
        // No light anywhere: the stage changes not one pixel.
        let mut lit = blank((64, 48));
        paint_body(&mut lit, MaterialKind::Slime, BODY, (52, 78, 52));
        let before = lit.pixels().to_vec();
        stage(
            &mut lit,
            &no_light(),
            &level,
            LightingMode::Full,
            1.0,
        );
        assert_eq!(lit.pixels(), before.as_slice(), "no light, no highlight");
    }

    #[test]
    fn a_matte_fur_body_gets_no_highlight() {
        let level = Level::empty(Theme::Sewer);
        let shoot = |grid: &Grid, kind: MaterialKind| {
            let mut pb = blank((64, 48));
            paint_body(&mut pb, kind, BODY, (62, 58, 54));
            stage(&mut pb, grid, &level, LightingMode::Full, 1.0);
            pb
        };
        let inside = 26..38;
        let rows = 20..28;
        let dark_fur = shoot(&no_light(), MaterialKind::Fur);
        let lit_fur = shoot(&west_light(), MaterialKind::Fur);
        assert!(
            same(&dark_fur, &lit_fur, inside.clone(), rows.clone()),
            "fur is matte: no highlight at all"
        );
        let dark_slime = shoot(&no_light(), MaterialKind::Slime);
        let lit_slime = shoot(&west_light(), MaterialKind::Slime);
        assert!(
            gain(&dark_slime, &lit_slime, inside, rows) > 0,
            "the same body in slime catches a sheen"
        );
    }

    #[test]
    fn the_rim_lights_only_the_lit_side() {
        let level = Level::empty(Theme::Sewer);
        let shoot = |grid: &Grid| {
            let mut pb = blank((64, 48));
            paint_body(&mut pb, MaterialKind::Fur, BODY, (60, 56, 52));
            stage(&mut pb, grid, &level, LightingMode::Full, 1.0);
            pb
        };
        let dark = shoot(&no_light());
        // The body is x 24..=39: its left limb faces west, its right limb east.
        let (left_limb, right_limb) = (24..25, 39..40);
        let east = shoot(&east_light());
        assert!(
            gain(&dark, &east, right_limb.clone(), body_rows()) > 0,
            "the limb facing the light gets the rim"
        );
        assert_eq!(
            gain(&dark, &east, left_limb.clone(), body_rows()),
            0,
            "the far limb stays dark"
        );
        let west = shoot(&west_light());
        assert!(
            gain(&dark, &west, left_limb.clone(), body_rows()) > 0,
            "and it flips with the light"
        );
        assert_eq!(
            gain(&dark, &west, right_limb, body_rows()),
            0,
            "the far limb stays dark"
        );
    }

    #[test]
    fn the_lower_tiers_draw_nothing_and_hard_only_the_glossy_materials() {
        let level = Level::empty(Theme::Sewer);
        let shoot = |grid: &Grid, kind: MaterialKind, mode: LightingMode| {
            let mut pb = blank((64, 48));
            paint_body(&mut pb, kind, BODY, (74, 58, 40));
            stage(&mut pb, grid, &level, mode, 1.0);
            pb
        };
        let inside = 26..38;
        let rows = 20..28;
        for mode in [LightingMode::Coarse, LightingMode::Compact] {
            let dark = shoot(&no_light(), MaterialKind::Slime, mode);
            let lit = shoot(&west_light(), MaterialKind::Slime, mode);
            assert!(
                same(&dark, &lit, 0..64, 0..48),
                "{mode:?} draws no specular at all"
            );
        }
        // Hard: Slime, Ink and Metal only — wood is out, a slime is not.
        let dark = shoot(&no_light(), MaterialKind::Wood, LightingMode::Hard);
        let lit = shoot(&west_light(), MaterialKind::Wood, LightingMode::Hard);
        assert!(
            same(&dark, &lit, inside.clone(), rows.clone()),
            "wood is not one of Hard's materials"
        );
        let dark = shoot(&no_light(), MaterialKind::Slime, LightingMode::Hard);
        let lit = shoot(&west_light(), MaterialKind::Slime, LightingMode::Hard);
        assert!(
            gain(&dark, &lit, inside, rows) > 0,
            "slime is one of Hard's materials"
        );
    }

    #[test]
    fn a_still_frame_draws_the_same_pixels_twice() {
        let level = Level::empty(Theme::Sewer);
        let shoot = |elapsed: f32| {
            let mut pb = blank((64, 48));
            paint_body(&mut pb, MaterialKind::Glass, (24, 20, 10, 8), (90, 120, 130));
            stage(&mut pb, &east_light(), &level, LightingMode::Full, elapsed);
            pb
        };
        // Two instants inside the same 8 Hz step of a scene that is not moving.
        let a = shoot(1.00);
        let b = shoot(1.10);
        assert!(
            a.pixels() == b.pixels(),
            "nothing in this stage reads a clock"
        );
    }
}
