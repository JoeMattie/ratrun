//! Parallax **foreground**: the few, huge overhead things nearest the viewer.
//!
//! The owner asked for "a foreground layer in some cases with parallax"; v1's
//! small, floating props did not read, so v2 is **huge and few**: an item is
//! 3–6× v1's draw size, at most ~2 are in view at once, and no item floats —
//! every one is visibly attached to something (a wall, the map edge, or the
//! ceiling above the frame). [`crate::game::art_foreground`] is the art, one
//! item at a time. Nothing here is simulation state: the layer is generated
//! from the finished level, drawn into the world-resolution picture, and read
//! by no system — so it cannot move a golden or the map pin (*Determinism*).
//!
//! ## Attachments (never floating)
//!
//! [`Attach::Span`] runs wall to wall: the code walks the tile grid along the
//! axis through the anchor to the nearest **solid** wall both ways, so
//! [`ForeItem::len`] is the real gap plus 6 px bitten into each wall, and the
//! art draws collars/brackets at both ends. [`Attach::Edge`] enters from beyond
//! the map/room on one side ([`Side`]): its anchor is a point on a solid wall
//! face and its reach goes into the open. [`Attach::Drop`] hangs from above the
//! frame: the art runs the cord/chain/trunk up the screen for [`DROP_TAIL`] px,
//! so it always leaves the top of the view. [`Attach::Sky`] is the outdoor
//! floor's own movers (see *Sky movers*).
//!
//! ## Placement
//!
//! From `(theme, level seed, the level's rooms/corridors)` once per level
//! ([`ForeLayer::sync`]), at most [`MAX_ITEMS`] 60 items and never two anchors
//! closer than [`MIN_SPACING`] 220 world px:
//!
//! * **corridors** at least 5 tiles long get one Span *across* the corridor
//!   (perpendicular to its long axis) per ~14 tiles, in one corridor of two.
//!   A corridor is 1–3 cells across, so a cross-span is measured against
//!   [`CORRIDOR_SPAN_MIN`] 28 px, not a room's [`SPAN_MIN_PX`] 64: the look is
//!   a fat pipe or beam crossing the passage overhead;
//! * **rooms/halls** at least 12 × 9 tiles get a Span across the short
//!   dimension, an Edge reaching 80–160 px in from a wall face, or a Drop over
//!   open floor at least 4 tiles from any wall, one per ~120 tiles² and at most
//!   three a room, the kind chosen among the theme's three whose attach class
//!   fits ([`kinds_for`]).
//!
//! Keep-outs: the start room's centre (96 px), a mouse-hole shop's face
//! (5 tiles), and 48 px around any rat-hole, one-way door or maze cache.
//!
//! **No dependence on a boss arena.** The layer reads rooms, tile kinds and the
//! object lists only — never `Level::boss` — so a former arena is just a large
//! room here (batch 12 removed the arenas; this module cannot care).
//!
//! ## Where in the frame
//!
//! `World::draw_scene` calls [`draw`] **after the lighting pass and before**
//! `World::draw_hostile_layer`: pellets, telegraphs, tells, miniboss shapes,
//! damage digits and the ink pass are always stamped on top, so the layer can
//! never hide the fight. It is drawn in the world picture (before the zoom
//! resample), so it works at every zoom rung including sub-cell detail.
//!
//! ## Parallax, size and depth
//!
//! An item at world position `P` is drawn at
//! `screen = view_centre + (P - cam_centre) * depth`, `depth` in `1.8..=3.0`
//! (near the eye: it slides fast as the camera moves, which is intended). Draw
//! size is *separate*: `scale` in `2.0..=3.4` multiplies the art's thickness and
//! detail. The floor shadow stays at the item's **un-parallaxed** world
//! position (`w2s(P)` plus (+10, +14) px) with the same footprint, so the body
//! visibly slides over its own shadow.
//!
//! ## The upscale (chunky pixels)
//!
//! The owner: "the pixels themselves need to be made bigger (actually scaled
//! up, not redrawn with regular sized pixels)". The art draws each item at its
//! own `scale` into a kept offscreen [`Scratch`] (rgb + alpha, and a shadow
//! strength plane), and the layer blits it **nearest-neighbour** at the integer
//! [`pixel_scale`]: [`PIXEL_SCALE`] 2, or [`PIXEL_SCALE_NEAR`] 3 from depth
//! [`PIXEL_SCALE_NEAR_DEPTH`] 2.6 (a Cloud is always 2). Every art pixel is a
//! `k x k` block, a hole in a grating is a block of alpha 0, and the shadow is
//! chunky too. The art does not know: [`blend`] and [`shadow`] route to the
//! lent scratch while it runs ([`target_rect`] tells it the window). Net size
//! on screen is `scale x k`: a pipe is ~35-85 px thick in a 120 x 84 px view
//! and most items are bigger than the view, on purpose. `len` is world-measured,
//! so the art is given `len / k` ([`art_item`]) and a Span still ends in its
//! walls (to within `k` px). The scratch window is the item's art box clipped
//! to what can reach the view, so a long span or a Drop's 400 px cord never
//! rasterises off-screen pixels; the blocks are rigid with the item (fixed
//! phase to its anchor), so nothing crawls as the camera moves.
//!
//! ## Never hide the fight
//!
//! * Every body pixel goes through [`blend`], which **clamps alpha to
//!   [`ALPHA_MAX`]** (215/255): no pixel of this layer is ever opaque.
//! * The item fades to [`ALPHA_FLOOR_RAT`] (120) within [`FADE_RAT_PX`] 48 px
//!   of the rat and to [`ALPHA_FLOOR`] (28) within [`FADE_THREAT_PX`] of a
//!   miniboss/boss body, ramping back over [`FADE_RAMP_PX`]. The distance is
//!   measured to the item's on-screen **body rect** (0 inside, after the
//!   upscale), not to its anchor, so a huge pipe over the rat fades.
//! * A **clear disc** around the rat, per screen pixel: nothing at all is
//!   written within [`RAT_SKIP_PX`] 10 px, alpha is quartered inside
//!   [`CLEAR_DISC_IN_PX`] 14 px, ramping back to full at [`CLEAR_DISC_OUT_PX`]
//!   30 px. It is a thread-local set by [`draw`] for the frame and cleared
//!   afterwards (a direct `blend` call, as in a test, sees no disc).
//! * **Coverage**: when the items' body rects would cover more than
//!   [`COVERAGE_PCT`] 45 % of the view, every alpha is scaled down (to
//!   [`COVERAGE_ALPHA_MIN`] at most), and [`blend`] refuses writes past
//!   [`COVERAGE_SKIP_PCT`] 85 % of the view (shadows are darkens and are not
//!   counted), so no art can break the cap.
//! * The colour is **darkened by [`DARKEN`]** (0.88) inside [`blend`]. The art's
//!   bodies are a mid-dark per-floor colour clearly lighter than the floor
//!   (batch 13c: the near-black v2 bodies were murky on the dark floors), so
//!   the layer reads at a glance and still looks like a dim overhead thing.
//!
//! ## Sky movers (the outdoor floor)
//!
//! The Flatbed level (`Theme::Trailer`, a moving truck bed) scrolls on its own,
//! so its [`Attach::Sky`] items are not placed by the room logic: [`sky_items`]
//! computes three analytic tracks from `(level seed, elapsed)` alone, with the
//! clock quantised to 1/30 s — two drifting clouds, three power wires always in
//! view (drawn by [`draw_wires`]: one art pixel thick, [`WIRE_RGB`], mean y at
//! [`WIRE_BANDS`] 18/24/31 % of the view, sagging between the poles) with a
//! pole passing every ~4 s, and a branch sweeping across every 5–9 s. Paused, `elapsed` does not advance, so nothing moves. The art needs
//! no clock: it gets ordinary [`ForeDraw`] positions.
//!
//! ## Cost and stillness
//!
//! No allocation per frame: the item list is built once per level, the planned
//! list and the frame plan are `Vec`s that are cleared and refilled (their
//! capacity is reached on the first frame a level draws), and the threat list is
//! a fixed eight-entry array on the stack. With a still camera and no sway step
//! the layer writes the same pixels, so the presenter sees an unchanged frame;
//! sway is stepped at [`SWAY_HZ`] (3 Hz) for the kinds that sway, so a still
//! scene repaints the layer's cells at most three times a second.
//!
//! ## Determinism
//!
//! [`generate`] is a pure function of the finished [`Level`] (theme, seed,
//! start, rooms, tile kinds and the object lists) with its own splitmix stream
//! salted away ([`GEN_SALT`]) from the map's, so it draws no simulation rng and
//! touches no simulation state. The item list is kept on `World` as **render
//! state** (like `light_map`/`detail`), not as a resource, and `World::draw`
//! never writes anything a system reads. The goldens (`game/ecs/harness.rs`)
//! fold simulation state only and `levelgen`'s `TRACE_HASHES` pins
//! `generate_once`, which this module only reads — so neither can move.

use bevy_ecs::world::World as EcsWorld;

use crate::game::art_foreground;
use crate::game::art_walls::WallKind;
use crate::game::collision::{TileKind, TileMap, TILE_F};
use crate::game::ecs::systems::boss::Boss;
use crate::game::ecs::systems::miniboss::Miniboss;
use crate::game::enemy::Enemy;
use crate::game::level::{Level, RoomKind, Theme};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

// ---- Public tuning constants ---------------------------------------------

/// The largest alpha any body pixel of this layer can reach (215/255).
/// [`blend`] clamps to it, so nothing in the layer is ever opaque.
pub const ALPHA_MAX: u8 = 215;
/// The dimmest the **threat** fade takes an item to (out of 255): an item over
/// a boss or a miniboss all but disappears.
pub const ALPHA_FLOOR: f32 = 28.0;
/// The dimmest the **rat** fade takes a whole item to. Higher than
/// [`ALPHA_FLOOR`] on purpose: a 1x view is only 120 x 84 px, so every item on
/// screen is within [`FADE_RAT_PX`] of the rat and a floor of 28 made the layer
/// invisible in play. 120 (it was 150 before the upscale made every item 2-3x
/// bigger). The rat's own surroundings are cleared per pixel instead (the clear
/// disc, and nothing at all within [`RAT_SKIP_PX`]).
pub const ALPHA_FLOOR_RAT: f32 = 120.0;
/// Fade to [`ALPHA_FLOOR_RAT`] within this many pixels of the rat.
pub const FADE_RAT_PX: f32 = 48.0;
/// … and within this many pixels of a miniboss/boss body.
pub const FADE_THREAT_PX: f32 = 48.0;
/// … ramping back to full over this many pixels.
pub const FADE_RAMP_PX: f32 = 40.0;
/// Sway is stepped at this rate (Hz), for the kinds that sway only.
pub const SWAY_HZ: f32 = 3.0;
/// How much of the view the layer may cover at full alpha, in percent. A frame
/// whose items would cover more (estimated from their on-screen body rects) is
/// drawn proportionally more transparent, down to [`COVERAGE_ALPHA_MIN`] of its
/// alpha, so a huge item is seen through rather than cut off.
pub const COVERAGE_PCT: u32 = 45;
/// The hard writer cap: past this percentage of the view [`blend`] drops the
/// write and the remaining items are skipped outright (a safety net; the alpha
/// scaling above is what normally holds the layer back).
pub const COVERAGE_SKIP_PCT: u32 = 85;
/// The least the coverage scaling multiplies an item's alpha by.
pub const COVERAGE_ALPHA_MIN: f32 = 0.5;
/// The share of an item's body rect its silhouette is taken to fill.
const COVERAGE_FILL: f32 = 0.7;
/// The alpha multiplier for items drawn inside the coverage grace band.
pub const COVERAGE_HALF: f32 = 0.5;
/// The fixed silhouette darkening [`blend`] applies to every colour. 0.88 (it
/// was 0.72, which with the v2 near-black bodies crushed them into the dark
/// floors; the bodies now carry their own mid-dark colour).
pub const DARKEN: f32 = 0.88;
/// How far [`shadow`] may darken a floor pixel at its strongest.
pub const SHADOW_MAX: f32 = 1.0;
/// How far a [`Attach::Drop`] item's cord/chain/trunk runs up the screen, so it
/// always leaves the top of the view.
pub const DROP_TAIL: i32 = 400;

/// Inside this radius of the rat every body alpha is multiplied by
/// [`CLEAR_DISC_MIN`] (the per-pixel guard, see [`blend`]).
pub const CLEAR_DISC_IN_PX: f32 = 14.0;
/// … ramping back to 1.0 at this radius.
pub const CLEAR_DISC_OUT_PX: f32 = 30.0;
/// The alpha multiplier at the centre of the clear disc.
pub const CLEAR_DISC_MIN: f32 = 0.25;
/// Inside this radius of the rat the layer writes **nothing** at all (the clear
/// disc's hard core): with items 2-3x bigger, the rat's own body and the few
/// pixels around it are never under the layer.
pub const RAT_SKIP_PX: f32 = 10.0;

/// **The upscale.** Every item is rasterised by the art at its own `scale` into
/// a small offscreen scratch and blitted nearest-neighbour at this integer
/// factor, so each art pixel is a `k x k` block on screen: the layer reads as
/// magnified, nearer than the floor. `PIXEL_SCALE` for items nearer than
/// [`PIXEL_SCALE_NEAR_DEPTH`], [`PIXEL_SCALE_NEAR`] from it (a Cloud is always
/// `PIXEL_SCALE`). See [`pixel_scale`].
pub const PIXEL_SCALE: i32 = 2;
/// The upscale factor of the nearest items (`depth >=` [`PIXEL_SCALE_NEAR_DEPTH`]).
pub const PIXEL_SCALE_NEAR: i32 = 3;
/// From this parallax depth an item is drawn at [`PIXEL_SCALE_NEAR`].
pub const PIXEL_SCALE_NEAR_DEPTH: f32 = 2.6;
/// Art pixels of slack around an item's [`art_foreground::extent`] in the
/// scratch (feathers, sway, the shadow's own soft edge).
const SCRATCH_MARGIN: i32 = 12;
/// Where the art is told the item's anchor is, in scratch (art) coordinates.
/// Fixed, so an art pixel's coordinate never depends on how much of the item
/// is visible this frame (hash-textured art cannot crawl), and positive, so no
/// art arithmetic ever sees a negative coordinate.
const ART_ORIGIN: i32 = 2048;
/// The most pixels the scratch is ever asked to hold (a safety net: the window
/// is already clipped to the view, so a real frame is far below it).
const SCRATCH_MAX_PX: i32 = 1 << 20;

/// The Flatbed's power wires: three lines, one **art** pixel thick (so `k`
/// screen px), pinned inside the view at these fractions of its height (their
/// mean y), sagging [`WIRE_SAG_ART`] art px between the passing poles and
/// sliding horizontally with them. Drawn by the layer itself ([`draw_wires`]).
pub const WIRE_BANDS: [f32; 3] = [0.18, 0.24, 0.31];
/// How far a wire sags between two poles, in art px.
pub const WIRE_SAG_ART: f32 = 6.0;
/// The wires' colour and alpha (before [`DARKEN`] and the fades): a light
/// grey-blue that reads against the dark bed.
pub const WIRE_RGB: Rgb = (150, 160, 175);
pub const WIRE_ALPHA: u8 = 200;
/// The wires' floor shadow strength (0..=1).
const WIRE_SHADOW: f32 = 0.18;

// ---- Internal tuning ------------------------------------------------------

/// Placement keep-out around a shop hole's face (5 tiles).
const CLEAR_PX: f32 = 5.0 * TILE_F;
/// Placement keep-out around the start room's centre (6 tiles).
const START_CLEAR_PX: f32 = 6.0 * TILE_F;
/// Placement keep-out around a rat-hole, one-way door or maze cache (3 tiles).
const FIXTURE_CLEAR_PX: f32 = 3.0 * TILE_F;
/// No two anchors closer than this (world px): at most ~2 items in a view.
const MIN_SPACING: f32 = 220.0;
/// The most items a level can hold.
const MAX_ITEMS: usize = 60;
/// The layer's own splitmix salt, so its stream is not the map's.
const GEN_SALT: u32 = 0x0F0E_6000;
/// The radius given to a boss body for the fade (the art's bodies are ~27 px
/// across; this is deliberately generous).
const BOSS_BODY_RADIUS: f32 = 20.0;

/// The parallax range for a placed item ([`ForeItem::depth`]).
const DEPTH_MIN: f32 = 1.8;
const DEPTH_MAX: f32 = 3.0;
/// The draw-size range for a placed item ([`ForeItem::scale`]).
/// 2.0–3.4 (the contract's first 3.0–6.0 was sized on a 240 x 148 px preview;
/// in a 120 x 84 px view a 50 px pipe is the whole picture and reads as
/// nothing): a pipe is 17–29 px thick, a fifth to a third of the view's height.
const SCALE_MIN: f32 = 2.0;
const SCALE_MAX: f32 = 3.4;

/// A room Span shorter than this (or longer than [`SPAN_MAX_PX`]) is not
/// placed.
const SPAN_MIN_PX: f32 = 64.0;
/// A **corridor** cross-span's minimum. A 1–3-cell corridor is only 16–48 px
/// across, and a fat pipe or beam crossing it overhead (28 px of span, 30–50 px
/// of body) is exactly the look the owner asked for, so the corridor rule has
/// its own, smaller floor.
const CORRIDOR_SPAN_MIN: f32 = 28.0;
const SPAN_MAX_PX: f32 = 420.0;
/// How far a Span's `len` bites into each wall.
const SPAN_BITE_PX: f32 = 6.0;
/// An Edge's reach bounds, from its wall face into the open.
const EDGE_LEN_MIN_PX: f32 = 44.0;
const EDGE_LEN_MAX_PX: f32 = 90.0;
/// A Drop must hang over floor at least this many tiles from any wall.
const DROP_CLEAR_TILES: i32 = 4;
/// Corridors at least this long get cross-spans, one per
/// [`CORRIDOR_TILES_PER_SPAN`] tiles.
const CORRIDOR_MIN_TILES: i32 = 5;
const CORRIDOR_TILES_PER_SPAN: i32 = 14;
/// Rooms: at least [`ROOM_MIN_SHORT`] on the short side and
/// [`ROOM_MIN_CELLS`] cells, one item per [`ROOM_TILES2_PER_ITEM`] cells up to
/// [`ROOM_MAX_ITEMS`].
const ROOM_MIN_SHORT: i32 = 9;
const ROOM_MIN_LONG: i32 = 12;
const ROOM_MIN_CELLS: i32 = 108;
const ROOM_TILES2_PER_ITEM: i32 = 120;
const ROOM_MAX_ITEMS: i32 = 3;
/// How many anchor draws one room slot may spend before it is given up.
const PLACE_ATTEMPTS: u32 = 4;
/// The longest wall walk (cells) before a span or edge gives up.
const MAX_WALL_WALK: i32 = 48;

/// The sky clock is quantised to this rate, so a frame is a function of a held
/// step and a paused frame is still.
const SKY_QUANT_HZ: f32 = 30.0;
/// One branch event every [`SKY_BRANCH_PERIOD`] s ± jitter (5–9 s), sweeping at
/// [`SKY_BRANCH_SPEED`] px/s.
const SKY_BRANCH_PERIOD: f32 = 7.0;
const SKY_BRANCH_SPEED: f32 = 220.0;
/// A pole passes every [`SKY_POLE_PERIOD`] s at [`SKY_POLE_SPEED`] px/s.
const SKY_POLE_PERIOD: f32 = 4.0;
const SKY_POLE_SPEED: f32 = 300.0;
/// Clouds drift at [`SKY_CLOUD_SPEED`] px/s over a [`SKY_CLOUD_LOOP`] px loop.
const SKY_CLOUD_SPEED: f32 = 12.0;
const SKY_CLOUD_LOOP: f32 = 1200.0;
/// How far outside the view a sky mover starts and ends.
const SKY_MARGIN: f32 = 200.0;
/// The sky items' parallax. The wires and the branch are *pinned to the view*
/// (`depth` 1.0 with their world position taken from the camera), because they
/// are painted across the whole frame; clouds are the far horizon, so they sit
/// in bed space with a very low `depth`: they barely slide as the camera moves
/// and their floor shadow falls well away from the haze body, which is the
/// depth cue. A low depth is also what keeps them on screen while the camera
/// pans across the bed.
const SKY_PINNED_DEPTH: f32 = 1.0;
const SKY_CLOUD_DEPTH: f32 = 0.15;
const SKY_WIRE_SCALE: f32 = 1.5;
const SKY_POLE_SCALE: f32 = 2.0;
/// A cloud's body is a haze: never above this alpha, whatever the fades say.
const CLOUD_ALPHA_MAX: u8 = 60;
/// The alpha of a pinned sky mover (wires, pole, branch) before the threat fade.
const SKY_ALPHA: f32 = 185.0;

// ---- Attachment, side and kind (the art's contract) -----------------------

/// The side of the frame an [`Attach::Edge`] item enters from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    N,
    E,
    S,
    W,
}

impl Side {
    /// The grid/world delta of a side (`+y` is south, as everywhere else).
    pub const fn dir(self) -> (i32, i32) {
        match self {
            Side::N => (0, -1),
            Side::E => (1, 0),
            Side::S => (0, 1),
            Side::W => (-1, 0),
        }
    }

    /// Is this side's inward axis horizontal (so an Edge from it is a
    /// horizontal item)?
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Side::E | Side::W)
    }
}

/// Attach-class codes, for [`ForeKind::attach_class`] and the placement rule
/// (a candidate asks for a class, the theme's fitting kinds answer).
pub const ATTACH_SPAN: u8 = 0;
pub const ATTACH_EDGE: u8 = 1;
pub const ATTACH_DROP: u8 = 2;
pub const ATTACH_SKY: u8 = 3;

/// How an item is attached to the world — never floating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    /// Wall to wall; `len` reaches into both walls.
    Span,
    /// In from beyond the map/room on `side`; `pos` is on a solid wall face.
    Edge(Side),
    /// Hangs from above the frame; the art draws [`DROP_TAIL`] px up the screen.
    Drop,
    /// The outdoor floor's own movers (see [`sky_items`]).
    Sky,
}

impl Attach {
    /// The attach-class code of this attachment ([`ATTACH_SPAN`] and friends).
    pub const fn class(self) -> u8 {
        match self {
            Attach::Span => ATTACH_SPAN,
            Attach::Edge(_) => ATTACH_EDGE,
            Attach::Drop => ATTACH_DROP,
            Attach::Sky => ATTACH_SKY,
        }
    }
}

/// One overhead kind. Each floor uses three (see [`kinds_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeKind {
    // Sewer
    /// Span: a fat riveted pipe with flanges and a valve wheel.
    MainPipe,
    /// Span: a walkway of open grating, light shows through the holes.
    CatwalkGrating,
    /// Drop: a hook block on a heavy chain.
    ChainHoist,
    // Kitchen
    /// Edge: a canopy hood corner with a filter grid.
    ExtractorHood,
    /// Span: a rail with big pans/ladles hanging from it (sway).
    PotRail,
    /// Drop: a wide enamel shade on a cord (sway).
    HangingLamp,
    // Lab
    /// Span: a ladder tray carrying bundled cables, some drooping.
    CableTray,
    /// Span: a square duct with seams and a louvre.
    VentDuct,
    /// Edge: a jointed equipment arm reaching in.
    ArticulatedArm,
    // Laundry
    /// Span: a line tied at both walls with big sheets and shirts (sway).
    Clothesline,
    /// Span: a lagged pipe with a leaking joint.
    SteamPipe,
    /// Drop: a ceiling airer (slatted frame) on four cords.
    DryingRack,
    // Printworks
    /// Span: a wide paper ribbon over rollers.
    PaperWeb,
    /// Span: a gantry beam with a roller carriage.
    RollerBeam,
    /// Drop: a rack of big printed sheets on clips (sway).
    HangingProofs,
    // Flatbed (OUTDOORS)
    /// Edge: a roadside branch with twigs and leaf clusters, sweeping past.
    TreeBranch,
    /// Sky: a soft cloud; only its shadow falls on the bed plus a faint haze.
    Cloud,
    /// Span across the whole view: 2–3 sagging wires, with a pole passing.
    PowerLine,
}

impl ForeKind {
    /// Every kind, in declaration order (the art worker's checklist).
    pub const fn all() -> [ForeKind; 18] {
        [
            ForeKind::MainPipe,
            ForeKind::CatwalkGrating,
            ForeKind::ChainHoist,
            ForeKind::ExtractorHood,
            ForeKind::PotRail,
            ForeKind::HangingLamp,
            ForeKind::CableTray,
            ForeKind::VentDuct,
            ForeKind::ArticulatedArm,
            ForeKind::Clothesline,
            ForeKind::SteamPipe,
            ForeKind::DryingRack,
            ForeKind::PaperWeb,
            ForeKind::RollerBeam,
            ForeKind::HangingProofs,
            ForeKind::TreeBranch,
            ForeKind::Cloud,
            ForeKind::PowerLine,
        ]
    }

    /// The [`Attach`] class this kind is drawn for ([`ATTACH_SPAN`] and
    /// friends), which is what placement asks for.
    pub const fn attach_class(self) -> u8 {
        match self {
            ForeKind::MainPipe
            | ForeKind::CatwalkGrating
            | ForeKind::PotRail
            | ForeKind::CableTray
            | ForeKind::VentDuct
            | ForeKind::Clothesline
            | ForeKind::SteamPipe
            | ForeKind::PaperWeb
            | ForeKind::RollerBeam
            | ForeKind::PowerLine => ATTACH_SPAN,
            ForeKind::ExtractorHood | ForeKind::ArticulatedArm | ForeKind::TreeBranch => ATTACH_EDGE,
            ForeKind::ChainHoist
            | ForeKind::HangingLamp
            | ForeKind::DryingRack
            | ForeKind::HangingProofs => ATTACH_DROP,
            ForeKind::Cloud => ATTACH_SKY,
        }
    }

    /// Does this kind swing, and so take a non-zero [`ForeDraw::sway`]? Lines,
    /// rails, racks and branches only — everything else is nailed up, and a
    /// still scene with nothing swaying in view makes the terminal repaint
    /// nothing.
    pub const fn sways(self) -> bool {
        matches!(
            self,
            ForeKind::PotRail
                | ForeKind::HangingLamp
                | ForeKind::Clothesline
                | ForeKind::HangingProofs
                | ForeKind::TreeBranch
        )
    }
}

/// One placed overhead object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForeItem {
    pub kind: ForeKind,
    /// Anchor in **world** pixels: the centre of a [`Attach::Span`], the root
    /// of an [`Attach::Edge`] (on a solid wall face), the object of an
    /// [`Attach::Drop`].
    pub pos: Vec2,
    /// Span: the wall-to-wall length, +6 px into each wall. Edge: the reach
    /// into the open from the wall face. Drop/Sky: 0.
    pub len: f32,
    /// Orientation of the long axis, for a Span or an Edge.
    pub horizontal: bool,
    /// How it is attached — what the art must show.
    pub attach: Attach,
    /// Parallax: screen pixels per camera pixel, `1.8..=3.0` for placed items.
    pub depth: f32,
    /// Art draw size, `2.0..=3.4` for placed items (Cloud `1.0..=1.6`); the
    /// on-screen size is this times [`pixel_scale`].
    pub scale: f32,
    /// Per-item variation (art detail, sway phase).
    pub seed: u32,
}

/// What the art function is handed for one item, one frame.
#[derive(Clone, Copy, Debug)]
pub struct ForeDraw {
    /// The anchor in the buffer after the parallax transform.
    pub screen: (i32, i32),
    /// Where the same anchor's **floor shadow** lands: world 1:1 plus the
    /// (10, 14) px offset. The art draws the shadow there (not at `screen`).
    pub shadow: (i32, i32),
    /// Draw size: equal to the item's [`ForeItem::scale`].
    pub scale: f32,
    /// Overall opacity after the fades, `0..=`[`ALPHA_MAX`].
    pub alpha: u8,
    /// `-1.0..=1.0`, stepped at [`SWAY_HZ`]; `0.0` for a kind that never sways.
    pub sway: f32,
    /// The floor, for palette and rim tint ([`rim_tint`]).
    pub theme: Theme,
    /// Sign of `(view centre - anchor)` per axis: which side of the body faces
    /// the middle of the screen, for the rim highlight.
    pub toward_centre: (i8, i8),
}

/// The rim tint the brief asks for: the floor's ambient character colour, dim
/// enough to stay a rim on a dark silhouette.
pub fn rim_tint(theme: Theme) -> Rgb {
    palette::scale(crate::render::lighting::theme_ambient(theme).rat_tint, 0.30)
}

/// The three kinds a floor's foreground uses.
pub const fn kinds_for(theme: Theme) -> [ForeKind; 3] {
    match theme {
        Theme::Sewer => [
            ForeKind::MainPipe,
            ForeKind::CatwalkGrating,
            ForeKind::ChainHoist,
        ],
        Theme::Kitchen => [
            ForeKind::ExtractorHood,
            ForeKind::PotRail,
            ForeKind::HangingLamp,
        ],
        Theme::Lab => [
            ForeKind::CableTray,
            ForeKind::VentDuct,
            ForeKind::ArticulatedArm,
        ],
        Theme::Laundry => [
            ForeKind::Clothesline,
            ForeKind::SteamPipe,
            ForeKind::DryingRack,
        ],
        Theme::Printworks => [
            ForeKind::PaperWeb,
            ForeKind::RollerBeam,
            ForeKind::HangingProofs,
        ],
        Theme::Trailer => [ForeKind::TreeBranch, ForeKind::Cloud, ForeKind::PowerLine],
    }
}

/// The integer upscale an item is blitted at ([`PIXEL_SCALE`] or
/// [`PIXEL_SCALE_NEAR`]): nearer items (higher parallax) are magnified more.
/// A cloud is far haze and always takes [`PIXEL_SCALE`].
pub fn pixel_scale(item: &ForeItem) -> i32 {
    if item.kind != ForeKind::Cloud && item.depth >= PIXEL_SCALE_NEAR_DEPTH {
        PIXEL_SCALE_NEAR
    } else {
        PIXEL_SCALE
    }
}

/// The item as the **art** sees it at upscale `k`: `len` is world-measured
/// (a Span must still end in its walls, an Edge reach exactly as far), so the
/// art is handed `len / k` and the upscale brings it back to `len`. Every other
/// field is the item's own (`scale` stays 2.0-3.4: the upscale multiplies it).
pub fn art_item(item: &ForeItem, k: i32) -> ForeItem {
    let mut art = *item;
    if art.len > 0.0 {
        art.len /= k.max(1) as f32;
    }
    art
}

/// The item's on-screen half-extent `(x, y)` after the upscale (the art's
/// [`art_foreground::extent`] of [`art_item`], times `k`, oriented to screen
/// axes as the plan reads it).
pub fn screen_extent(item: &ForeItem) -> (f32, f32) {
    let k = pixel_scale(item);
    let (ex, ey) = art_foreground::extent(&art_item(item, k));
    let kf = k as f32;
    if item.horizontal {
        (ex.max(0.0) * kf, ey.max(0.0) * kf)
    } else {
        (ey.max(0.0) * kf, ex.max(0.0) * kf)
    }
}

/// Is this item the Flatbed's pinned wire span (drawn by [`draw_wires`], not
/// the art)?
#[inline]
fn is_wire_span(item: &ForeItem) -> bool {
    item.kind == ForeKind::PowerLine && item.attach == Attach::Sky && item.len > 0.0
}

/// Is the layer on by default? `RATRUN_FOREGROUND=0` (or `off`/`false`/`no`)
/// pins it off for a session; the console's read-only `foreground on|off` is
/// the in-run switch.
pub fn enabled_from_env() -> bool {
    match std::env::var("RATRUN_FOREGROUND") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => true,
    }
}

// ---- The only pixel writers the art may use -------------------------------

thread_local! {
    /// Pixels this frame's layer may still paint (`u32::MAX` = unlimited). See
    /// [`COVERAGE_PCT`] and [`draw`].
    static WRITE_BUDGET: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
    /// Pixels painted since [`budget_start`].
    static WRITES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// The rat's **clear disc** for this frame: `(x, y, active)`. Set by [`draw`]
    /// at frame start and cleared at the end; a direct [`blend`] call (a test,
    /// a one-off) sees no disc.
    static CLEAR_DISC: std::cell::Cell<(i32, i32, bool)> =
        const { std::cell::Cell::new((0, 0, false)) };
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
pub(crate) struct Scratch {
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

/// The rectangle the art may draw into, `(x0, y0, w, h)`: the lent scratch's
/// window while an item is being rasterised, else the whole of `pb`. The art
/// clips its primitives to it, so it never rasterises a pixel nobody keeps.
pub fn target_rect(pb: &PixelBuffer) -> (i32, i32, i32, i32) {
    TARGET
        .with(|t| t.borrow().as_ref().map(|s| (s.x0, s.y0, s.w, s.h)))
        .unwrap_or((0, 0, pb.w as i32, pb.h as i32))
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

fn clear_disc_set(centre: (i32, i32)) {
    CLEAR_DISC.with(|c| c.set((centre.0, centre.1, true)));
}

fn clear_disc_reset() {
    CLEAR_DISC.with(|c| c.set((0, 0, false)));
}

/// The alpha multiplier the rat's clear disc asks for at `(x, y)`: 1.0 with no
/// disc, [`CLEAR_DISC_MIN`] inside [`CLEAR_DISC_IN_PX`], ramping back to 1.0 at
/// [`CLEAR_DISC_OUT_PX`].
#[inline]
fn clear_disc_factor(x: i32, y: i32) -> f32 {
    CLEAR_DISC.with(|c| {
        let (cx, cy, on) = c.get();
        if !on {
            return 1.0;
        }
        let dx = (x - cx) as f32;
        let dy = (y - cy) as f32;
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
        let (cx, cy, on) = c.get();
        if !on {
            return false;
        }
        let dx = (x - cx) as f32;
        let dy = (y - cy) as f32;
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

// ---- The layer, and its per-frame plan ------------------------------------

/// One item's placement in this frame (the plan [`draw`] executes).
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// Index into the layer's planned list (the placed items, then any sky
    /// movers), far to near.
    pub item: usize,
    /// The anchor's screen position after the parallax transform.
    pub screen: (i32, i32),
    /// Where the anchor's **un-parallaxed** floor shadow lands (world 1:1 plus
    /// the (+10, +14) px offset).
    pub shadow: (i32, i32),
    /// Draw size: the item's [`ForeItem::scale`].
    pub scale: f32,
    /// Opacity after the fades, `0..=ALPHA_MAX`.
    pub alpha: u8,
    /// `-1.0..=1.0` sway for this frame (stepped at [`SWAY_HZ`]).
    pub sway: f32,
    /// Which way the body's rim highlight faces (see [`ForeDraw`]).
    pub toward_centre: (i8, i8),
    /// The integer upscale this item is blitted at ([`pixel_scale`]).
    pub k: i32,
}

/// The layer's item list and its per-frame scratch. **Render state**: built
/// once per level, held on `World` next to `light_map`/`detail`, never a
/// simulation resource.
pub struct ForeLayer {
    items: Vec<ForeItem>,
    /// This frame's items in draw order: any sky movers, then `items`.
    planned: Vec<ForeItem>,
    /// The `(theme, level seed)` the items were built for.
    key: Option<(Theme, u64)>,
    frame: Vec<Plan>,
    /// The kept item raster the upscale blits from (see [`Scratch`]).
    scratch: Scratch,
    /// Test-only: keep a hand-placed list through [`ForeLayer::sync`].
    #[cfg(test)]
    forced: bool,
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
            planned: Vec::new(),
            key: None,
            frame: Vec::new(),
            scratch: Scratch::default(),
            #[cfg(test)]
            forced: false,
        }
    }

    /// Build the item list for `level` when it is a level this layer has not
    /// seen yet. Once per level: no other call allocates.
    pub fn sync(&mut self, level: &Level) {
        #[cfg(test)]
        if self.forced {
            return;
        }
        let key = (level.theme, level.seed);
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.items.clear();
        generate(level, &mut self.items);
        // Draw order, far to near, decided once per level: a stable sort keeps
        // the generation order among equal depths, so the order is total and
        // deterministic and the frame loop needs no sort of its own.
        self.items.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    }

    /// The placed items, in generation order (depths ascending).
    pub fn items(&self) -> &[ForeItem] {
        &self.items
    }

    /// This frame's plan, in draw order (far to near).
    pub fn frame(&self) -> &[Plan] {
        &self.frame
    }

    /// The scratch's buffer capacity (tests: no growth once warm).
    #[cfg(test)]
    pub(crate) fn scratch_capacity(&self) -> usize {
        self.scratch.capacity()
    }

    /// Replace the item list with a hand-built one and stop rebuilding from the
    /// level (tests: parallax, fade and ordering need known items).
    #[cfg(test)]
    pub(crate) fn force_items(&mut self, items: Vec<ForeItem>) {
        self.forced = true;
        self.items = items;
        self.frame.clear();
    }

    /// Plan one frame: assemble the draw list (sky movers first, they are the
    /// farthest thing in the layer), cull with the art's own extent, apply the
    /// parallax transform, work out the fade and the sway step.
    ///
    /// Pure in its arguments (`cam`, `size`, `rat`, `elapsed` and the threat
    /// list only decide *where* and *how opaque*, never *what* is placed).
    fn plan_frame(
        &mut self,
        level: &Level,
        cam: Vec2,
        size: (f32, f32),
        rat: Vec2,
        elapsed: f32,
        threats: &[(Vec2, f32)],
    ) {
        self.frame.clear();
        self.planned.clear();
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return;
        }
        sky_items(level, cam, size, elapsed, &mut self.planned);
        self.planned.extend_from_slice(&self.items);
        if self.planned.is_empty() {
            return;
        }
        let centre = Vec2::new(size.0 * 0.5, size.1 * 0.5);
        let cam_centre = cam + centre;
        let rat_screen = rat - cam;
        // The frame's estimated body coverage, in pixels (clouds are a haze).
        let mut covered = 0.0f32;
        for (i, item) in self.planned.iter().enumerate() {
            let item = *item;
            // The art's own half-extent (it owns its box), of the item as the
            // art draws it, times the upscale: what the item covers on screen.
            let k = pixel_scale(&item);
            let (hx, hy) = screen_extent(&item);
            let depth = item.depth.clamp(0.2, 4.0);
            // Parallax, per attachment, so an attached item stays attached:
            // a Span slides only ACROSS its long axis (along it the item rides
            // the world 1:1 and its two ends stay in their walls), an Edge only
            // along its wall (its root stays on the wall face), a Drop and the
            // sky movers on both axes.
            let (dx, dy) = match item.attach {
                Attach::Span if item.horizontal => (1.0, depth),
                Attach::Span => (depth, 1.0),
                Attach::Edge(side) if side.is_horizontal() => (1.0, depth),
                Attach::Edge(_) => (depth, 1.0),
                Attach::Drop | Attach::Sky => (depth, depth),
            };
            let off = item.pos - cam_centre;
            let p = centre + Vec2::new(off.x * dx, off.y * dy);
            // Cull: the art's box plus the sway reach (art px, so times the
            // upscale), and a Drop's tail above.
            let sway_reach = 6.0 * k as f32;
            let tail = if item.attach == Attach::Drop {
                DROP_TAIL as f32
            } else {
                0.0
            };
            if p.x + hx + sway_reach < 0.0
                || p.x - hx - sway_reach >= size.0
                || p.y + hy + sway_reach < 0.0
                || p.y - hy - tail >= size.1
            {
                continue;
            }
            // Fade: from the rat/threat to the item's on-screen BODY RECT (0
            // inside), so a huge pipe over the rat fades even though its anchor
            // is far away.
            // The sky movers are pinned to the view, so the rat is always
            // "under" them: they take a fixed alpha instead of the rat fade
            // (they pass in a second or two, and the clear disc still opens
            // the rat's own surroundings).
            let mut alpha = if item.attach == Attach::Sky || item.kind == ForeKind::TreeBranch {
                SKY_ALPHA
            } else {
                proximity_alpha(
                    ALPHA_MAX as f32,
                    box_dist(p, hx, hy, rat_screen),
                    FADE_RAT_PX,
                    ALPHA_FLOOR_RAT,
                )
            };
            for &(body, radius) in threats {
                let d = (box_dist(p, hx, hy, body - cam) - radius.max(0.0)).max(0.0);
                alpha = alpha.min(proximity_alpha(
                    ALPHA_MAX as f32,
                    d,
                    FADE_THREAT_PX,
                    ALPHA_FLOOR,
                ));
            }
            if item.kind == ForeKind::Cloud {
                // A cloud is a haze, never a silhouette.
                alpha = alpha.min(CLOUD_ALPHA_MAX as f32);
            }
            let shadow_at = item.pos - cam + Vec2::new(10.0, 14.0);
            let sign = |v: f32| -> i8 {
                if v > 0.5 {
                    1
                } else if v < -0.5 {
                    -1
                } else {
                    0
                }
            };
            // Clouds are a haze and the wires are three thin lines: neither is
            // counted as cover (the wires' box is the whole view's width).
            if item.kind != ForeKind::Cloud && !is_wire_span(&item) {
                let w = (p.x + hx).min(size.0) - (p.x - hx).max(0.0);
                let h = (p.y + hy).min(size.1) - (p.y - hy).max(0.0);
                covered += w.max(0.0) * h.max(0.0) * COVERAGE_FILL;
            }
            self.frame.push(Plan {
                item: i,
                screen: (p.x.round() as i32, p.y.round() as i32),
                shadow: (shadow_at.x.round() as i32, shadow_at.y.round() as i32),
                scale: item.scale,
                alpha: alpha.round().clamp(0.0, ALPHA_MAX as f32) as u8,
                sway: sway_of(&item, elapsed),
                toward_centre: (sign(centre.x - p.x), sign(centre.y - p.y)),
                k,
            });
        }
        // Too much of the view under the layer: see through all of it rather
        // than cut an item off.
        let cap = size.0 * size.1 * COVERAGE_PCT as f32 / 100.0;
        if covered > cap {
            let k = (cap / covered).max(COVERAGE_ALPHA_MIN);
            for plan in self.frame.iter_mut() {
                if self.planned[plan.item].kind != ForeKind::Cloud {
                    plan.alpha = (plan.alpha as f32 * k).round() as u8;
                }
            }
        }
    }
}

/// The scene [`draw`] paints into: the camera the scene drew with (shake
/// included), the buffer size, the rat, the frame's clock, and a **read-only**
/// handle to the ECS world for the bodies the fade keeps clear of.
pub struct ForeScene<'a> {
    pub ecs: &'a mut EcsWorld,
    pub level: &'a Level,
    pub cam: Vec2,
    /// The world-picture size in pixels (the frame being drawn).
    pub size: (f32, f32),
    pub rat: Vec2,
    pub elapsed: f32,
}

/// Draw one frame of the layer. Returns the pixels painted (the coverage the
/// frame used, for tests and the profiler).
///
/// Call it **after** the lighting pass and **before** the hostile layer.
pub fn draw(pb: &mut PixelBuffer, layer: &mut ForeLayer, scene: &mut ForeScene) -> u32 {
    layer.sync(scene.level);
    if pb.w == 0 || pb.h == 0 {
        return 0;
    }
    // The bodies the fade must keep clear of: every live miniboss's body and
    // the boss's. Capped at eight, which is more than a floor can hold (one
    // miniboss and one boss at a time).
    let mut threats = [(Vec2::ZERO, 0.0f32); 8];
    let mut n = 0usize;
    {
        let mut q = scene.ecs.query::<(&Enemy, Option<&Miniboss>)>();
        for (e, mb) in q.iter(scene.ecs) {
            if mb.is_none() || n == threats.len() {
                continue;
            }
            threats[n] = (e.pos, e.radius.max(8.0));
            n += 1;
        }
    }
    {
        let mut q = scene.ecs.query::<&Boss>();
        for boss in q.iter(scene.ecs) {
            if n == threats.len() {
                break;
            }
            threats[n] = (boss.pos, BOSS_BODY_RADIUS);
            n += 1;
        }
    }
    layer.plan_frame(
        scene.level,
        scene.cam,
        scene.size,
        scene.rat,
        scene.elapsed,
        &threats[..n],
    );
    if layer.frame.is_empty() {
        return 0;
    }
    let total = (pb.w * pb.h) as u32;
    let cap_full = total * COVERAGE_PCT / 100;
    let cap_skip = total * COVERAGE_SKIP_PCT / 100;
    budget_start(cap_skip);
    // The per-pixel guard, per SCREEN pixel (so after the upscale): nothing
    // within RAT_SKIP_PX of the rat, the alpha quartered within 14 px, ramping
    // back to full at 30 px, whatever the art paints.
    let rat_screen = scene.rat - scene.cam;
    clear_disc_set((rat_screen.x.round() as i32, rat_screen.y.round() as i32));
    // Far to near. The plan is already in that order (sky movers first, then
    // the depth-sorted placed items), and each item draws its own floor shadow
    // first, so a nearer body covers a farther shadow.
    for i in 0..layer.frame.len() {
        let plan = layer.frame[i];
        let Some(alpha) = coverage_alpha(plan.alpha, writes_so_far(), cap_full, cap_skip) else {
            break;
        };
        let item = layer.planned[plan.item];
        if is_wire_span(&item) {
            draw_wires(pb, plan.k, alpha, scene.elapsed);
            continue;
        }
        draw_item(pb, &mut layer.scratch, &item, &plan, alpha, scene.level.theme);
    }
    clear_disc_reset();
    budget_finish()
}

/// Rasterise one item into `scratch` at its art scale and blit it
/// nearest-neighbour at `plan.k`: the floor shadow plane at `plan.shadow`, then
/// the body at `plan.screen`, every art pixel a `k x k` block written through
/// [`shadow_screen`] / [`blend_screen`] (so the alpha clamp, [`DARKEN`], the
/// clear disc per screen pixel and the coverage budget all hold).
///
/// The art is told the anchor is at [`ART_ORIGIN`] and may draw into the
/// scratch window only ([`target_rect`]): the item's art box
/// ([`art_foreground::extent`] + [`SCRATCH_MARGIN`], plus a Drop's
/// [`DROP_TAIL`] above) clipped to the art pixels that can land in the view
/// from either anchor. A 420 px span is therefore never rasterised past the
/// view, and a Drop's cord is rasterised (chunky, like the rest) up to the
/// view's top edge and no further.
///
/// Block grid: art pixel `X` lands at `plan.screen.x + k * (X - ART_ORIGIN)`,
/// so the blocks are rigid with the item — their edges sit at a fixed phase
/// relative to its anchor whatever the camera does, and the art (rasterised at
/// a fixed integer anchor) is identical frame to frame. Nothing crawls.
fn draw_item(
    pb: &mut PixelBuffer,
    scratch: &mut Scratch,
    item: &ForeItem,
    plan: &Plan,
    alpha: u8,
    theme: Theme,
) {
    if alpha == 0 {
        return;
    }
    let k = plan.k.max(1);
    let art = art_item(item, k);
    let (ex, ey) = art_foreground::extent(&art);
    let (hx, hy) = if art.horizontal {
        (ex.max(0.0), ey.max(0.0))
    } else {
        (ey.max(0.0), ex.max(0.0))
    };
    let (hx, hy) = (hx.ceil() as i32 + SCRATCH_MARGIN, hy.ceil() as i32 + SCRATCH_MARGIN);
    let tail = if art.attach == Attach::Drop { DROP_TAIL } else { 0 };
    // The art box, in art coordinates around ART_ORIGIN.
    let (bx0, bx1) = (ART_ORIGIN - hx, ART_ORIGIN + hx);
    let (by0, by1) = (ART_ORIGIN - hy - tail, ART_ORIGIN + hy);
    // The art pixels that can land in the view from an anchor at `a` (screen).
    let (w, h) = (pb.w as i32, pb.h as i32);
    let vis = |a: i32, len: i32| -> (i32, i32) {
        (
            ART_ORIGIN + (-a).div_euclid(k) - 1,
            ART_ORIGIN + (len - 1 - a).div_euclid(k) + 1,
        )
    };
    let (bvx0, bvx1) = vis(plan.screen.0, w);
    let (svx0, svx1) = vis(plan.shadow.0, w);
    let (bvy0, bvy1) = vis(plan.screen.1, h);
    let (svy0, svy1) = vis(plan.shadow.1, h);
    let x0 = bx0.max(bvx0.min(svx0));
    let x1 = bx1.min(bvx1.max(svx1));
    let y0 = by0.max(bvy0.min(svy0));
    let y1 = by1.min(bvy1.max(svy1));
    if x0 > x1 || y0 > y1 {
        return;
    }
    let sw = (x1 - x0 + 1).min(SCRATCH_MAX_PX / (y1 - y0 + 1).max(1));
    let sh = y1 - y0 + 1;
    scratch.reset(x0, y0, sw, sh);

    let d = ForeDraw {
        screen: (ART_ORIGIN, ART_ORIGIN),
        shadow: (ART_ORIGIN, ART_ORIGIN),
        scale: plan.scale,
        alpha,
        sway: plan.sway,
        theme,
        toward_centre: plan.toward_centre,
    };
    // Lend the scratch to the two writers for the art call, then take it back.
    let lent = std::mem::take(scratch);
    TARGET.with(|t| *t.borrow_mut() = Some(lent));
    art_foreground::draw(pb, &art, &d);
    *scratch = TARGET.with(|t| t.borrow_mut().take()).unwrap_or_default();

    blit(pb, scratch, plan.shadow, k, true);
    blit(pb, scratch, plan.screen, k, false);
}

/// Blit one plane of `sc` with the art origin at screen `anchor`, each art
/// pixel a `k x k` block: the shadow plane (`shade`) through [`shadow_screen`],
/// or the body through [`blend_screen`].
fn blit(pb: &mut PixelBuffer, sc: &Scratch, anchor: (i32, i32), k: i32, shade: bool) {
    let (bw, bh) = (pb.w as i32, pb.h as i32);
    let ox = anchor.0 + k * (sc.x0 - ART_ORIGIN);
    let oy = anchor.1 + k * (sc.y0 - ART_ORIGIN);
    for j in 0..sc.h {
        let sy = oy + j * k;
        if sy + k <= 0 {
            continue;
        }
        if sy >= bh {
            break;
        }
        let (ya, yb) = (sy.max(0), (sy + k).min(bh));
        for i in 0..sc.w {
            let sx = ox + i * k;
            if sx + k <= 0 {
                continue;
            }
            if sx >= bw {
                break;
            }
            let idx = (j * sc.w + i) as usize;
            let (xa, xb) = (sx.max(0), (sx + k).min(bw));
            if shade {
                let v = sc.shade[idx];
                if v == 0 {
                    continue;
                }
                let st = v as f32 / 255.0;
                for y in ya..yb {
                    for x in xa..xb {
                        shadow_screen(pb, x, y, st);
                    }
                }
            } else {
                let a = sc.alpha[idx];
                if a == 0 {
                    continue;
                }
                let rgb = sc.rgb[idx];
                for y in ya..yb {
                    for x in xa..xb {
                        blend_screen(pb, x, y, rgb, a);
                    }
                }
            }
        }
    }
}

/// The Flatbed's three power wires, drawn by the layer (not the art) so they are
/// in every Flatbed frame: each is one art pixel thick (`k` screen px, chunky
/// like the rest of the layer) in [`WIRE_RGB`] at [`WIRE_ALPHA`] (scaled by
/// the item's faded `alpha` against [`SKY_ALPHA`]), its mean y pinned to
/// [`WIRE_BANDS`] of the view's height on the `k` grid, sagging
/// [`WIRE_SAG_ART`] art px between the passing poles and sliding horizontally
/// with them (the poles' own track: [`SKY_POLE_SPEED`] px/s, one every
/// [`SKY_POLE_PERIOD`] s). A thin floor shadow is pinned 10, 14 px below.
fn draw_wires(pb: &mut PixelBuffer, k: i32, alpha: u8, elapsed: f32) {
    let k = k.max(1);
    let (w, h) = (pb.w as i32, pb.h as i32);
    if w == 0 || h == 0 || alpha == 0 {
        return;
    }
    let a = (WIRE_ALPHA as f32 * (alpha as f32 / SKY_ALPHA).min(1.0)).round() as u8;
    if a == 0 {
        return;
    }
    let t = (elapsed.max(0.0) * SKY_QUANT_HZ).floor() / SKY_QUANT_HZ;
    let span = SKY_POLE_SPEED * SKY_POLE_PERIOD;
    let phase = t - (t / SKY_POLE_PERIOD).floor() * SKY_POLE_PERIOD;
    // A pole's screen x: the same track `sky_items` puts the pole on.
    let pole_x = w as f32 + SKY_MARGIN - SKY_POLE_SPEED * phase;
    for pass_shadow in [true, false] {
        for &band in &WIRE_BANDS {
            let base = wire_base_y(h as f32, band, k);
            let mut x0 = 0;
            while x0 < w {
                let off = wire_offset_art(x0 as f32 + k as f32 * 0.5, pole_x, span);
                let y0 = base + k * off;
                for y in y0..y0 + k {
                    for x in x0..(x0 + k).min(w) {
                        if pass_shadow {
                            shadow_screen(pb, x + 10, y + 14, WIRE_SHADOW);
                        } else {
                            blend_screen(pb, x, y, WIRE_RGB, a);
                        }
                    }
                }
                x0 += k;
            }
        }
    }
}

/// A wire's mean y on screen for `band` (a fraction of the view height),
/// snapped to the `k` grid.
#[inline]
fn wire_base_y(view_h: f32, band: f32, k: i32) -> i32 {
    ((view_h * band) / k as f32).round() as i32 * k
}

/// A wire's offset from its mean, in whole art px, at screen x `x` with a pole
/// at `pole_x` and poles every `span` px: highest at a pole, [`WIRE_SAG_ART`]
/// lower midway, mean zero over a span.
#[inline]
fn wire_offset_art(x: f32, pole_x: f32, span: f32) -> i32 {
    let u = (x - pole_x).rem_euclid(span) / span; // 0..1 between two poles
    let sag = WIRE_SAG_ART * 4.0 * u * (1.0 - u); // 0 at a pole, SAG midway
    (sag - WIRE_SAG_ART * 2.0 / 3.0).round() as i32
}

/// What alpha an item draws at, given the pixels already painted this frame:
/// full below [`COVERAGE_PCT`], half up to [`COVERAGE_SKIP_PCT`], then `None`
/// (the item is skipped).
#[inline]
fn coverage_alpha(alpha: u8, painted: u32, cap_full: u32, cap_skip: u32) -> Option<u8> {
    if painted >= cap_skip {
        return None;
    }
    if painted < cap_full {
        return Some(alpha);
    }
    Some((alpha as f32 * COVERAGE_HALF).round().clamp(0.0, 255.0) as u8)
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

/// Distance from `p` to the outside of the box `(bx, by)` around `p` (0 inside).
fn box_dist(p: Vec2, bx: f32, by: f32, q: Vec2) -> f32 {
    let dx = ((q.x - p.x).abs() - bx).max(0.0);
    let dy = ((q.y - p.y).abs() - by).max(0.0);
    (dx * dx + dy * dy).sqrt()
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

    /// A coin flip.
    fn flip(&mut self) -> bool {
        self.next_u64() & 1 == 1
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

/// The cell a world position falls in.
#[inline]
fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / TILE_F).floor() as i32, (p.y / TILE_F).floor() as i32)
}

/// The world centre of a cell.
#[inline]
fn cell_centre(c: (i32, i32)) -> Vec2 {
    Vec2::new((c.0 as f32 + 0.5) * TILE_F, (c.1 as f32 + 0.5) * TILE_F)
}

/// Distance in px from `origin` to the near face of the first **solid** wall
/// along `dir`, or `None` when the walk leaves the map or ends on a wall that is
/// not solid (a doorway, a rat-hole, a maze bar) — a span or a root there would
/// be a lie about where the thing is attached.
fn reach_to_wall(tiles: &TileMap, origin: Vec2, dir: (i32, i32)) -> Option<f32> {
    let (cx, cy) = cell_of(origin);
    let mut k = 0i32;
    loop {
        k += 1;
        if k > MAX_WALL_WALK {
            return None;
        }
        let (nx, ny) = (cx + dir.0 * k, cy + dir.1 * k);
        if !tiles.in_bounds(nx, ny) {
            return None;
        }
        let kind = tiles.get(nx, ny);
        if kind.is_open() {
            continue;
        }
        if !matches!(kind, TileKind::Wall(WallKind::Solid)) {
            return None;
        }
        let face = match (dir.0, dir.1) {
            (1, 0) => Vec2::new(nx as f32 * TILE_F, origin.y),
            (-1, 0) => Vec2::new((nx + 1) as f32 * TILE_F, origin.y),
            (0, 1) => Vec2::new(origin.x, ny as f32 * TILE_F),
            (0, -1) => Vec2::new(origin.x, (ny + 1) as f32 * TILE_F),
            _ => return None,
        };
        return Some((face - origin).len());
    }
}

/// Measure the span through `at` along its long axis: walk the tile grid both
/// ways to the solid walls, then return the recentred anchor (the midpoint of
/// the two wall faces) and `len` = the gap + 6 px bitten into each wall.
///
/// `None` when either end leaves the map without a wall, or when the span is
/// shorter than `min_len` (rooms: [`SPAN_MIN_PX`], corridors:
/// [`CORRIDOR_SPAN_MIN`]) or longer than [`SPAN_MAX_PX`].
fn span_across(
    tiles: &TileMap,
    at: Vec2,
    horizontal: bool,
    min_len: f32,
) -> Option<(Vec2, f32)> {
    if !tiles.get_at(at).is_open() {
        return None;
    }
    let (a, b) = if horizontal {
        (
            reach_to_wall(tiles, at, (1, 0))?,
            reach_to_wall(tiles, at, (-1, 0))?,
        )
    } else {
        (
            reach_to_wall(tiles, at, (0, 1))?,
            reach_to_wall(tiles, at, (0, -1))?,
        )
    };
    let len = a + b + 2.0 * SPAN_BITE_PX;
    if len < min_len || len > SPAN_MAX_PX {
        return None;
    }
    let mid = if horizontal {
        Vec2::new(at.x + (a - b) * 0.5, at.y)
    } else {
        Vec2::new(at.x, at.y + (a - b) * 0.5)
    };
    Some((mid, len))
}

/// Put an Edge's root on the solid wall face that `at` faces along `side`, and
/// return it with its reach: `want` (80–160 px) clamped so the tip stops a cell
/// short of the opposite wall. `None` when there is no solid wall that way or no
/// room to reach into.
fn edge_from_wall(tiles: &TileMap, at: Vec2, side: Side, want: f32) -> Option<(Vec2, f32)> {
    if !tiles.get_at(at).is_open() {
        return None;
    }
    let (dx, dy) = side.dir();
    let dist = reach_to_wall(tiles, at, (dx, dy))?;
    let room = dist - TILE_F;
    if room < EDGE_LEN_MIN_PX {
        return None;
    }
    let pos = Vec2::new(at.x + dx as f32 * dist, at.y + dy as f32 * dist);
    Some((pos, want.min(room)))
}

/// Is every cell within `r` of `at` an open floor cell? (A Drop needs open floor
/// around it: it hangs over the middle of a room, never over a wall.)
fn open_around(tiles: &TileMap, at: Vec2, r: i32) -> bool {
    let (cx, cy) = cell_of(at);
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = (cx + dx, cy + dy);
            if !tiles.in_bounds(x, y) || !tiles.get(x, y).is_open() {
                return false;
            }
        }
    }
    true
}

/// The theme's kind of `class`, chosen by `rng` when the theme has more than
/// one. `None` when no kind of that floor fits the attachment.
fn kind_of_class(theme: Theme, class: u8, rng: &mut Gen) -> Option<ForeKind> {
    let kinds = kinds_for(theme);
    let matching = kinds.iter().filter(|k| k.attach_class() == class).count();
    // The draw is taken whatever the answer, so the stream order is stable.
    let roll = rng.next_u64() as usize;
    if matching == 0 {
        return None;
    }
    kinds
        .iter()
        .filter(|k| k.attach_class() == class)
        .nth(roll % matching)
        .copied()
}

/// The side a `0..4` roll names, in declaration order.
fn side_from_roll(v: f32) -> Side {
    match v as i32 {
        0 => Side::N,
        1 => Side::E,
        2 => Side::S,
        _ => Side::W,
    }
}

/// Build the layer's items for `level`: a pure function of the finished level
/// (theme, seed, start, rooms, tile kinds and the object lists) and its own rng.
/// Draws no simulation rng, so it cannot move a golden.
fn generate(level: &Level, out: &mut Vec<ForeItem>) {
    let theme = level.theme;
    let tiles = &level.tiles;
    let mut rng = Gen::new(crate::game::level::level_seed(theme, level.seed, GEN_SALT));
    let start = level.start;
    let shops = &level.objects.shop_holes;
    let holes = &level.objects.rat_holes;
    let doors = &level.objects.one_way_doors;
    let caches = &level.mazes;

    // Keep-outs: the start room's centre, a shop hole's threshold, and the
    // fixtures that must stay legible under the layer. Deliberately **nothing**
    // about a boss arena: the layer reads rooms, tile kinds and the object lists
    // only, so removing arenas from the levels (batch 12) cannot break it.
    let clear_of_fixtures = |p: Vec2| -> bool {
        p.dist(start) >= START_CLEAR_PX
            && shops.iter().all(|h| h.pos.dist(p) >= CLEAR_PX)
            && holes
                .iter()
                .all(|h| cell_centre(h.cell).dist(p) >= FIXTURE_CLEAR_PX)
            && doors
                .iter()
                .all(|d| cell_centre(d.cell).dist(p) >= FIXTURE_CLEAR_PX)
            && caches.iter().all(|m| m.reward.dist(p) >= FIXTURE_CLEAR_PX)
    };
    let spaced = |out: &[ForeItem], p: Vec2| -> bool {
        out.iter().all(|i| i.pos.dist(p) >= MIN_SPACING)
    };
    let can_place = |out: &[ForeItem], p: Vec2| -> bool {
        out.len() < MAX_ITEMS && clear_of_fixtures(p) && spaced(out, p)
    };

    // 1. Corridors: a Span ACROSS the corridor (perpendicular to its long
    // axis), one corridor in two, one per ~14 tiles of length.
    for room in level.rooms.iter().filter(|r| r.kind == RoomKind::Corridor) {
        let long_axis_horizontal = room.rect.w >= room.rect.h;
        let long_cells = if long_axis_horizontal {
            room.rect.w
        } else {
            room.rect.h
        };
        if long_cells < CORRIDOR_MIN_TILES || !rng.flip() {
            continue;
        }
        let slots = (long_cells / CORRIDOR_TILES_PER_SPAN).max(1);
        let centre = room.rect.center_world();
        for _ in 0..slots {
            let along = rng.range(1.0, (long_cells - 1).max(1) as f32);
            let depth = rng.range(DEPTH_MIN, DEPTH_MAX);
            let scale = rng.range(SCALE_MIN, SCALE_MAX);
            let seed = rng.u32();
            let Some(kind) = kind_of_class(theme, ATTACH_SPAN, &mut rng) else {
                continue;
            };
            let at = if long_axis_horizontal {
                Vec2::new(
                    (room.rect.x as f32 + along + 0.5) * TILE_F,
                    centre.y,
                )
            } else {
                Vec2::new(
                    centre.x,
                    (room.rect.y as f32 + along + 0.5) * TILE_F,
                )
            };
            // Across the corridor: the item's long axis is the short one. A
            // corridor is narrow, so this rule has its own, smaller minimum.
            let span_horizontal = !long_axis_horizontal;
            let Some((pos, len)) =
                span_across(tiles, at, span_horizontal, CORRIDOR_SPAN_MIN)
            else {
                continue;
            };
            if !can_place(out.as_slice(), pos) {
                continue;
            }
            out.push(ForeItem {
                kind,
                pos,
                len,
                horizontal: span_horizontal,
                attach: Attach::Span,
                depth,
                scale,
                seed,
            });
        }
    }

    // 2. Rooms/halls: a Span across the short dimension, an Edge from a wall
    // face, or a Drop over open floor; one per ~120 tiles², three a room at
    // most. A room's kind is not read: a former boss arena is just a large room.
    for room in &level.rooms {
        if matches!(room.kind, RoomKind::Corridor | RoomKind::Maze) {
            continue;
        }
        let (w, h) = (room.rect.w, room.rect.h);
        if w.min(h) < ROOM_MIN_SHORT || w.max(h) < ROOM_MIN_LONG || w * h < ROOM_MIN_CELLS {
            continue;
        }
        let slots = (w * h / ROOM_TILES2_PER_ITEM).clamp(1, ROOM_MAX_ITEMS);
        // One cell in from the rect's edge: the edge itself is the wall row.
        let x0 = (room.rect.x + 1) as f32 * TILE_F;
        let y0 = (room.rect.y + 1) as f32 * TILE_F;
        let x1 = (room.rect.right() - 1) as f32 * TILE_F;
        let y1 = (room.rect.bottom() - 1) as f32 * TILE_F;
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        for _ in 0..slots {
            for _attempt in 0..PLACE_ATTEMPTS {
                let at = Vec2::new(rng.range(x0, x1), rng.range(y0, y1));
                let class_roll = rng.range(0.0, 3.0);
                let side_roll = rng.range(0.0, 4.0);
                let want = rng.range(EDGE_LEN_MIN_PX, EDGE_LEN_MAX_PX);
                let depth = rng.range(DEPTH_MIN, DEPTH_MAX);
                let scale = rng.range(SCALE_MIN, SCALE_MAX);
                let seed = rng.u32();
                // A Span lies across the room's short dimension.
                let class = if class_roll < 1.0 {
                    ATTACH_SPAN
                } else if class_roll < 2.0 {
                    ATTACH_EDGE
                } else {
                    ATTACH_DROP
                };
                let Some(kind) = kind_of_class(theme, class, &mut rng) else {
                    continue;
                };
                let placed: Option<(Vec2, f32, bool, Attach)> = match class {
                    ATTACH_SPAN => {
                        let span_horizontal = w < h;
                        span_across(tiles, at, span_horizontal, SPAN_MIN_PX)
                            .map(|(pos, len)| (pos, len, span_horizontal, Attach::Span))
                    }
                    ATTACH_EDGE => {
                        let side = side_from_roll(side_roll);
                        edge_from_wall(tiles, at, side, want).map(|(pos, len)| {
                            (pos, len, side.is_horizontal(), Attach::Edge(side))
                        })
                    }
                    _ => {
                        if open_around(tiles, at, DROP_CLEAR_TILES) {
                            Some((at, 0.0, true, Attach::Drop))
                        } else {
                            None
                        }
                    }
                };
                let Some((pos, len, horizontal, attach)) = placed else {
                    continue;
                };
                if !can_place(out.as_slice(), pos) {
                    continue;
                }
                out.push(ForeItem {
                    kind,
                    pos,
                    len,
                    horizontal,
                    attach,
                    depth,
                    scale,
                    seed,
                });
                break;
            }
        }
    }
}

// ---- Sky movers (the outdoor floor) ---------------------------------------

/// A 0..=1 hash of `(seed, k)`.
fn hash01(seed: u64, k: u64) -> f32 {
    let h = mix32((seed as u32) ^ mix32(k as u32 ^ 0x5BD1_E995));
    (h & 0x00FF_FFFF) as f32 / 16_777_215.0
}

/// A per-item `u32` seed from `(seed, k)`.
fn hash_u32(seed: u64, k: u64) -> u32 {
    mix32((seed as u32).wrapping_add(mix32(k as u32)))
}

/// The outdoor floor's own movers: a pure function of `(level seed, elapsed)`
/// (plus the camera and the view, which only say where "the view" is), with the
/// clock quantised to 1/30 s, so a paused frame is still and two calls at the
/// same instant agree exactly.
///
/// * **Cloud** — two, drifting at 12 px/s over a 1200 px loop in bed space,
///   distant (`depth` below 1, so they slide slower than the floor and their
///   shadow falls away from the body).
/// * **PowerLine** — one Span pinned across the whole view at a seed-chosen
///   band, plus a pole crossing every ~4 s at 300 px/s (a second PowerLine item,
///   `len` 0, vertical).
/// * **TreeBranch** — one event every 5–9 s (the period is the jittered gap
///   between consecutive event starts, from a hash of the index), sweeping
///   tail-ward at 220 px/s, anchored on the N or S edge of the view.
///
/// The sky list is far to near: clouds, wires, pole, branch.
fn sky_items(level: &Level, cam: Vec2, size: (f32, f32), elapsed: f32, out: &mut Vec<ForeItem>) {
    if level.theme != Theme::Trailer {
        return;
    }
    let (vw, vh) = (size.0.max(1.0), size.1.max(1.0));
    let t = (elapsed.max(0.0) * SKY_QUANT_HZ).floor() / SKY_QUANT_HZ;
    let seed = level.seed;

    // ---- Clouds: distant, in bed space, drifting over a 1200 px loop.
    for i in 0..2u64 {
        let base = hash01(seed ^ 0xC10D_0000, i);
        let drift = base * SKY_CLOUD_LOOP + SKY_CLOUD_SPEED * t;
        let wrapped = drift % SKY_CLOUD_LOOP;
        let x = level.arena.x * 0.5 - SKY_CLOUD_LOOP * 0.5 + wrapped;
        let y = level.arena.y * (0.15 + hash01(seed ^ 0x7E57_0000, i) * 0.7);
        out.push(ForeItem {
            kind: ForeKind::Cloud,
            pos: Vec2::new(x, y),
            len: 0.0,
            horizontal: true,
            attach: Attach::Sky,
            depth: SKY_CLOUD_DEPTH,
            scale: 1.0 + hash01(seed ^ 0x0D0D_0000, i) * 0.6,
            seed: hash_u32(seed ^ 0xC10D_0000, i),
        });
    }

    // ---- The wires: one Span across the whole view, pinned to the frame and
    // always inside it (the v2 wires slid with the camera and wrapped, and in
    // the captures they were never on screen). The item only marks them: the
    // layer draws the three lines itself ([`draw_wires`]) at [`WIRE_BANDS`] of
    // the view height, sliding horizontally with the poles.
    let wire_y = cam.y + vh * WIRE_BANDS[1];
    out.push(ForeItem {
        kind: ForeKind::PowerLine,
        pos: Vec2::new(cam.x + vw * 0.5, wire_y),
        len: vw + 160.0,
        horizontal: true,
        attach: Attach::Sky,
        depth: SKY_PINNED_DEPTH,
        scale: SKY_WIRE_SCALE,
        seed: hash_u32(seed, 0x12),
    });

    // ---- A pole passing every ~4 s at 300 px/s.
    let pole_span = vw + 2.0 * SKY_MARGIN;
    let pole_sweep = pole_span / SKY_POLE_SPEED;
    let pole_phase = t - (t / SKY_POLE_PERIOD).floor() * SKY_POLE_PERIOD;
    if pole_phase < pole_sweep {
        let x = cam.x + vw + SKY_MARGIN - SKY_POLE_SPEED * pole_phase;
        // The crossarm sits on the middle wire where it meets the pole: the
        // art's arm is 6.5 * scale art px above the anchor and a wire is
        // `2/3 * WIRE_SAG_ART` above its mean at a pole, both times the upscale.
        let arm_drop = (6.5 * SKY_POLE_SCALE - WIRE_SAG_ART * 2.0 / 3.0) * PIXEL_SCALE as f32;
        out.push(ForeItem {
            kind: ForeKind::PowerLine,
            pos: Vec2::new(x, wire_y + arm_drop),
            len: 0.0,
            horizontal: false,
            attach: Attach::Sky,
            depth: SKY_PINNED_DEPTH,
            scale: SKY_POLE_SCALE,
            seed: hash_u32(seed, 0x13),
        });
    }

    // ---- Branches: one event per jittered period, sweeping the view.
    let jitter = |k: u64| (hash01(seed ^ 0x5EED_0000, k) - 0.5) * 2.0;
    let branch_sweep = (vw + 2.0 * SKY_MARGIN) / SKY_BRANCH_SPEED;
    let candidate = (t / SKY_BRANCH_PERIOD).floor().max(0.0) as u64;
    for k in candidate.saturating_sub(1)..=candidate + 1 {
        let start = SKY_BRANCH_PERIOD * k as f32 + jitter(k);
        let end = SKY_BRANCH_PERIOD * (k + 1) as f32 + jitter(k + 1);
        if t < start || t >= end || (t - start) >= branch_sweep {
            continue;
        }
        let from_north = hash01(seed ^ 0x9E37_0000, k) < 0.5;
        let x = cam.x + vw + SKY_MARGIN - SKY_BRANCH_SPEED * (t - start);
        // Rooted just outside the view's N or S edge, reaching 55–80 % of the
        // way across it.
        let y = if from_north { cam.y - 6.0 } else { cam.y + vh + 6.0 };
        let reach = vh * (0.55 + 0.25 * hash01(seed ^ 0xB4A2_0000, k));
        out.push(ForeItem {
            kind: ForeKind::TreeBranch,
            pos: Vec2::new(x, y),
            len: reach,
            horizontal: false,
            attach: if from_north {
                Attach::Edge(Side::N)
            } else {
                Attach::Edge(Side::S)
            },
            depth: SKY_PINNED_DEPTH,
            scale: 3.0 + hash01(seed ^ 0x1EAF_0000, k) * 1.2,
            seed: hash_u32(seed ^ 0x1EAF_0000, k),
        });
        break;
    }
}

/// The sky movers at `elapsed` for a view at `cam` (captures: finding the
/// moment a branch or a pole is mid-view).
#[cfg(test)]
pub(crate) fn sky_probe(level: &Level, cam: Vec2, size: (f32, f32), elapsed: f32) -> Vec<ForeItem> {
    let mut out = Vec::new();
    sky_items(level, cam, size, elapsed, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::bullet::{Bullet, Faction};
    use crate::game::world::World;

    const VIEW: (usize, usize) = (120, 80); // a 120 × 40-cell view at 1x

    fn layer_for(theme: Theme, seed: u64) -> (World, ForeLayer) {
        let w = World::new(theme, seed);
        let mut layer = ForeLayer::new();
        layer.sync(&w.level);
        (w, layer)
    }

    /// A hand-built item, so a test never repeats all eight fields.
    #[allow(clippy::too_many_arguments)]
    fn mk(
        kind: ForeKind,
        pos: Vec2,
        len: f32,
        horizontal: bool,
        attach: Attach,
        depth: f32,
        scale: f32,
        seed: u32,
    ) -> ForeItem {
        ForeItem {
            kind,
            pos,
            len,
            horizontal,
            attach,
            depth,
            scale,
            seed,
        }
    }

    /// Draw one frame over a mid-grey floor and return the buffer plus the
    /// pixels painted.
    fn render(
        w: &mut World,
        layer: &mut ForeLayer,
        cam: Vec2,
        rat: Vec2,
        elapsed: f32,
    ) -> (PixelBuffer, u32) {
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        pb.clear((70, 66, 60));
        let mut scene = ForeScene {
            ecs: &mut w.ecs,
            level: &w.level,
            cam,
            size: (VIEW.0 as f32, VIEW.1 as f32),
            rat,
            elapsed,
        };
        let used = draw(&mut pb, layer, &mut scene);
        (pb, used)
    }

    fn changed(a: &PixelBuffer, b: &PixelBuffer) -> usize {
        a.pixels().iter().zip(b.pixels()).filter(|(x, y)| x != y).count()
    }

    fn lum(c: Rgb) -> u32 {
        u32::from(c.0) + u32::from(c.1) + u32::from(c.2)
    }

    fn kind_code(kind: TileKind) -> u64 {
        match kind {
            TileKind::Open => 0,
            TileKind::Wall(WallKind::Solid) => 1,
            TileKind::Wall(WallKind::Maze) => 2,
            TileKind::Wall(WallKind::RatHole { .. }) => 3,
            TileKind::Wall(WallKind::Doorway { .. }) => 4,
            TileKind::Wall(WallKind::OneWayDoor { .. }) => 5,
        }
    }

    fn fold(h: &mut u64, v: u64) {
        *h ^= v;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3).rotate_left(7);
    }

    /// Everything about a level a render-only layer must never write.
    fn level_fingerprint(level: &Level) -> u64 {
        let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
        for cy in 0..level.tiles.rows() {
            for cx in 0..level.tiles.cols() {
                fold(&mut h, kind_code(level.tiles.get(cx, cy)));
            }
        }
        fold(&mut h, level.seed);
        fold(&mut h, level.rooms.len() as u64);
        fold(&mut h, level.objects.shop_holes.len() as u64);
        fold(&mut h, level.objects.rat_holes.len() as u64);
        fold(&mut h, level.objects.one_way_doors.len() as u64);
        fold(&mut h, level.mazes.len() as u64);
        fold(&mut h, level.start.x.to_bits() as u64);
        fold(&mut h, level.start.y.to_bits() as u64);
        h
    }

    /// Generation is a pure function of the level: the same `(theme, seed)`
    /// twice gives the same list, in the same order, and every item honours its
    /// kind's attachment and the v2 size bands.
    #[test]
    fn generation_is_deterministic_for_every_theme_and_seed() {
        for theme in Theme::playable() {
            for seed in [1u64, 2, 7, 1234] {
                let (_, a) = layer_for(theme, seed);
                let (_, b) = layer_for(theme, seed);
                assert_eq!(a.items(), b.items(), "{theme:?} seed {seed}");
                for item in a.items() {
                    assert!(
                        kinds_for(theme).contains(&item.kind),
                        "{theme:?}: a foreign kind"
                    );
                    assert_eq!(
                        item.kind.attach_class(),
                        item.attach.class(),
                        "{theme:?}: {item:?} is not drawn for its attachment"
                    );
                    assert!(
                        (DEPTH_MIN..=DEPTH_MAX).contains(&item.depth),
                        "depth {}",
                        item.depth
                    );
                    assert!(
                        (SCALE_MIN..=SCALE_MAX).contains(&item.scale),
                        "scale {}",
                        item.scale
                    );
                    assert!(item.len >= 0.0, "a negative length");
                    match item.attach {
                        Attach::Span => assert!(
                            (CORRIDOR_SPAN_MIN..=SPAN_MAX_PX).contains(&item.len),
                            "span len {}",
                            item.len
                        ),
                        Attach::Edge(_) => assert!(
                            (EDGE_LEN_MIN_PX..=EDGE_LEN_MAX_PX).contains(&item.len),
                            "edge len {}",
                            item.len
                        ),
                        Attach::Drop => assert_eq!(item.len, 0.0, "a hanging item has no length"),
                        Attach::Sky => {}
                    }
                }
            }
        }
    }

    /// Every floor places items over a handful of seeds (at most two of the
    /// three kinds of an outdoor floor ever fit a room, so Trailer is the
    /// thinnest), and a different seed on the same floor is a different layer.
    #[test]
    fn every_floor_places_items_and_seeds_change_them() {
        for theme in Theme::playable() {
            let mut total = 0;
            for seed in 0..8u64 {
                let (_, a) = layer_for(theme, seed);
                total += a.items().len();
            }
            assert!(total > 0, "{theme:?} placed nothing over 8 seeds");
            let (_, a) = layer_for(theme, 11);
            let (_, b) = layer_for(theme, 12);
            if !a.items().is_empty() && !b.items().is_empty() {
                assert_ne!(a.items(), b.items(), "{theme:?}: the seed does nothing");
            }
        }
    }

    /// The item list is independent of the camera, the zoom and the frame
    /// clock: generation reads the level only.
    #[test]
    fn the_items_are_identical_at_every_zoom_and_camera() {
        let (mut w, mut layer) = layer_for(Theme::Sewer, 5);
        let base = layer.items().to_vec();
        for z in [0.5f32, 0.75, 1.0, 1.25, 1.5, 2.0] {
            w.set_zoom_instant(z);
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut pb);
            layer.sync(&w.level);
            assert_eq!(layer.items(), base.as_slice(), "zoom {z} changed the list");
        }
        let _ = render(&mut w, &mut layer, Vec2::new(400.0, 300.0), Vec2::ZERO, 9.0);
        assert_eq!(
            layer.items(),
            base.as_slice(),
            "a camera move changed the list"
        );
    }

    /// Nothing sits over the start room's centre, within 5 tiles of a shop hole,
    /// or within 48 px of a rat-hole, one-way door or maze cache.
    #[test]
    fn nothing_covers_the_start_a_shop_hole_or_a_fixture() {
        for theme in Theme::playable() {
            for seed in 0..20u64 {
                let (w, layer) = layer_for(theme, seed);
                for item in layer.items() {
                    assert!(
                        item.pos.dist(w.level.start) >= START_CLEAR_PX,
                        "{theme:?} {seed}: an item hangs over the start centre"
                    );
                    for hole in &w.level.objects.shop_holes {
                        assert!(
                            item.pos.dist(hole.pos) >= CLEAR_PX,
                            "{theme:?} {seed}: an item crowds a shop hole"
                        );
                    }
                    for hole in &w.level.objects.rat_holes {
                        assert!(
                            item.pos.dist(cell_centre(hole.cell)) >= FIXTURE_CLEAR_PX,
                            "{theme:?} {seed}: an item crowds a rat-hole"
                        );
                    }
                    for door in &w.level.objects.one_way_doors {
                        assert!(
                            item.pos.dist(cell_centre(door.cell)) >= FIXTURE_CLEAR_PX,
                            "{theme:?} {seed}: an item crowds a one-way door"
                        );
                    }
                    for maze in &w.level.mazes {
                        assert!(
                            item.pos.dist(maze.reward) >= FIXTURE_CLEAR_PX,
                            "{theme:?} {seed}: an item crowds a maze cache"
                        );
                    }
                }
            }
        }
    }

    /// HUGE and FEW: no two anchors closer than [`MIN_SPACING`], and no level
    /// holds more than sixty items.
    #[test]
    fn no_two_anchors_are_closer_than_the_spacing_and_no_level_exceeds_the_cap() {
        for theme in Theme::playable() {
            for seed in 0..20u64 {
                let (_, layer) = layer_for(theme, seed);
                let items = layer.items();
                assert!(
                    items.len() <= MAX_ITEMS,
                    "{theme:?} {seed}: {} items",
                    items.len()
                );
                assert!(items.len() <= 60, "{theme:?} {seed}: the per-level cap is 60");
                for i in 0..items.len() {
                    for j in i + 1..items.len() {
                        assert!(
                            items[i].pos.dist(items[j].pos) >= MIN_SPACING,
                            "{theme:?} {seed}: anchors {} px apart",
                            items[i].pos.dist(items[j].pos)
                        );
                    }
                }
            }
        }
    }

    /// Never floating: on a generated Sewer level, the two ends of every Span
    /// sit **inside solid wall tiles**, so `len` really reaches into both walls,
    /// and the anchor hangs over open floor.
    #[test]
    fn every_span_ends_inside_a_solid_wall() {
        let mut spans = 0;
        for seed in [1u64, 2, 5, 9, 21] {
            let (w, layer) = layer_for(Theme::Sewer, seed);
            let tiles = &w.level.tiles;
            for item in layer.items() {
                if item.attach != Attach::Span {
                    continue;
                }
                spans += 1;
                let half = item.len * 0.5;
                let (ax, ay) = if item.horizontal {
                    (half, 0.0)
                } else {
                    (0.0, half)
                };
                for s in [1.0f32, -1.0] {
                    let end = Vec2::new(item.pos.x + ax * s, item.pos.y + ay * s);
                    assert!(
                        matches!(tiles.get_at(end), TileKind::Wall(WallKind::Solid)),
                        "seed {seed}: a span end at {end:?} is not inside a solid wall"
                    );
                }
                assert!(
                    tiles.get_at(item.pos).is_open(),
                    "seed {seed}: a span's anchor is inside a wall"
                );
            }
        }
        assert!(spans > 0, "no span was placed, so the test proves nothing");
    }

    /// A corridor cross-span is measured against the corridor's own, smaller
    /// minimum ([`CORRIDOR_SPAN_MIN`] 28 px), so a fat pipe really does cross a
    /// 1–3-cell corridor overhead; a room span still has to reach
    /// [`SPAN_MIN_PX`].
    #[test]
    fn corridor_cross_spans_use_the_smaller_minimum() {
        let mut short_corridor_spans = 0;
        for seed in 0..12u64 {
            let (w, layer) = layer_for(Theme::Sewer, seed);
            for item in layer.items() {
                if item.attach != Attach::Span {
                    continue;
                }
                // A span's anchor is recentred onto the middle of what it
                // crosses, so the room rect says exactly which rule placed it.
                let in_corridor = w.level.rooms.iter().any(|r| {
                    r.kind == RoomKind::Corridor && r.rect.contains_world(item.pos)
                });
                let min = if in_corridor {
                    CORRIDOR_SPAN_MIN
                } else {
                    SPAN_MIN_PX
                };
                assert!(
                    item.len >= min && item.len <= SPAN_MAX_PX,
                    "seed {seed}: a {} span of {} px is outside {min}..={SPAN_MAX_PX}",
                    if in_corridor { "corridor" } else { "room" },
                    item.len
                );
                if in_corridor && item.len < SPAN_MIN_PX {
                    short_corridor_spans += 1;
                }
            }
        }
        assert!(
            short_corridor_spans > 0,
            "no corridor cross-span used the 28 px minimum over 12 Sewer seeds"
        );
    }

    /// Generation is render-only: it never writes the level (whose tile kinds
    /// the map pin folds).
    #[test]
    fn generation_never_touches_the_level() {
        for theme in Theme::playable() {
            for seed in [0u64, 3, 17] {
                let w = World::new(theme, seed);
                let before = level_fingerprint(&w.level);
                let mut items = Vec::new();
                generate(&w.level, &mut items);
                assert_eq!(
                    before,
                    level_fingerprint(&w.level),
                    "{theme:?} {seed}: generation wrote to the level"
                );
                assert!(items.len() <= MAX_ITEMS);
            }
        }
    }

    /// The coverage cap holds over 20 seeds × 6 themes at several cameras: the
    /// enforced budget (pixels `blend` actually painted) never exceeds
    /// [`COVERAGE_SKIP_PCT`] of the view, and the layer does paint.
    #[test]
    fn the_coverage_cap_holds_everywhere() {
        let cap = (VIEW.0 * VIEW.1) as u32 * COVERAGE_SKIP_PCT / 100;
        let mut painted_any = false;
        for theme in Theme::playable() {
            for seed in 0..20u64 {
                let (mut w, mut layer) = layer_for(theme, seed);
                let arena = w.level.arena;
                let cams = [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(arena.x * 0.5, arena.y * 0.5),
                    Vec2::new(arena.x - VIEW.0 as f32, arena.y - VIEW.1 as f32),
                    Vec2::new(arena.x * 0.25, arena.y * 0.7),
                ];
                for cam in cams {
                    for elapsed in [0.0f32, 1.0, 7.5] {
                        let (_, used) = render(&mut w, &mut layer, cam, cam, elapsed);
                        painted_any |= used > 0;
                        assert!(
                            used <= cap,
                            "{theme:?} {seed} cam {cam:?}: {used} px painted of a {cap} px cap"
                        );
                    }
                }
            }
        }
        assert!(
            painted_any,
            "the layer never painted at all: the cap is vacuous"
        );
    }

    /// Parallax: an item at depth 2.5 slides 2.5 px per camera px, and its
    /// floor shadow stays on the world (1:1 plus the (+10, +14) offset).
    #[test]
    fn an_item_at_depth_two_point_five_moves_two_and_a_half_pixels_per_camera_pixel() {
        let mut w = World::new(Theme::Sewer, 5);
        let mut layer = ForeLayer::new();
        layer.force_items(vec![mk(
            ForeKind::MainPipe,
            Vec2::new(600.0, 400.0),
            200.0,
            true,
            Attach::Span,
            2.5,
            5.0,
            3,
        )]);
        let plan_at = |layer: &mut ForeLayer, w: &mut World, cam: Vec2| -> Plan {
            let mut scene = ForeScene {
                ecs: &mut w.ecs,
                level: &w.level,
                cam,
                size: (VIEW.0 as f32, VIEW.1 as f32),
                rat: Vec2::new(-4000.0, -4000.0), // far away: no fade
                elapsed: 0.0,
            };
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            pb.clear((70, 66, 60));
            draw(&mut pb, layer, &mut scene);
            *layer.frame().first().expect("a plan")
        };
        let a = plan_at(&mut layer, &mut w, Vec2::new(540.0, 360.0));
        let b = plan_at(&mut layer, &mut w, Vec2::new(542.0, 362.0));
        assert_eq!(
            (a.screen.0 - b.screen.0, a.screen.1 - b.screen.1),
            (2, 5),
            "2 px of camera at depth 2.5 must be 5 px of item across a Span and 2 along it"
        );
        assert_eq!(
            (a.shadow.0 - b.shadow.0, a.shadow.1 - b.shadow.1),
            (2, 2),
            "the shadow rides the world, not the parallax"
        );
    }

    /// The fade is measured to the item's on-screen **body rect**, not to its
    /// anchor: the rat standing under a huge pipe fades it even though the
    /// anchor is far away.
    #[test]
    fn the_fade_measures_the_body_rect_not_the_anchor() {
        let mut w = World::new(Theme::Sewer, 5);
        let mut layer = ForeLayer::new();
        let item = mk(
            ForeKind::MainPipe,
            Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5),
            400.0,
            true,
            Attach::Span,
            2.0,
            1.0,
            7,
        );
        layer.force_items(vec![item]);
        // The on-screen box, after the upscale (horizontal: x is the long axis).
        let (ex, _ey) = screen_extent(&item);
        assert!(
            ex * 0.5 > FADE_RAT_PX + FADE_RAMP_PX,
            "the body rect ({ex} px half-length) is too small to tell the rect from the anchor"
        );
        let alpha_with_rat = |layer: &mut ForeLayer, w: &mut World, rat: Vec2| -> u8 {
            let mut scene = ForeScene {
                ecs: &mut w.ecs,
                level: &w.level,
                cam: Vec2::ZERO,
                size: (VIEW.0 as f32, VIEW.1 as f32),
                rat,
                elapsed: 0.0,
            };
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            pb.clear((70, 66, 60));
            draw(&mut pb, layer, &mut scene);
            layer.frame().first().expect("a plan").alpha
        };
        let anchor = Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5);
        // Under the body, far from the anchor along it: the floor alpha.
        let under = alpha_with_rat(
            &mut layer,
            &mut w,
            Vec2::new(anchor.x + ex * 0.5, anchor.y),
        );
        assert_eq!(under, ALPHA_FLOOR_RAT as u8, "the body rect did not fade");
        // Clear of the body: full alpha.
        let clear = alpha_with_rat(
            &mut layer,
            &mut w,
            Vec2::new(anchor.x + ex + FADE_RAT_PX + FADE_RAMP_PX + 4.0, anchor.y),
        );
        assert_eq!(clear, ALPHA_MAX, "the fade never returned to full");
    }

    /// **The layer can never paint an opaque pixel**, whatever the art asks
    /// for: `blend` clamps to [`ALPHA_MAX`]. That is the structural half of
    /// "never hide the fight".
    #[test]
    fn blend_clamps_the_alpha_so_nothing_is_ever_opaque() {
        let mut pb = PixelBuffer::new(2, 1);
        pb.clear((200, 100, 50));
        blend(&mut pb, 0, 0, (0, 0, 0), 255);
        let got = pb.pixel_at(0, 0);
        let floor: (u8, u8, u8) = (200, 100, 50);
        for (g, f) in [(got.0, floor.0), (got.1, floor.1), (got.2, floor.2)] {
            let moved = f32::from(f) - f32::from(g);
            assert!(
                moved <= f32::from(f) * (ALPHA_MAX as f32 / 255.0) + 1.0,
                "the floor pixel was more than {ALPHA_MAX}/255 covered"
            );
        }
        // And the write counted against the budget.
        budget_start(0);
        blend(&mut pb, 1, 0, (255, 255, 255), 128);
        assert_eq!(budget_finish(), 0, "a spent budget must refuse the write");
        assert_eq!(pb.pixel_at(1, 0), (200, 100, 50), "the refused write landed");
    }

    /// The per-pixel guard: the frame's clear disc quarters the alpha inside
    /// [`CLEAR_DISC_IN_PX`] of the rat, ramps back out, and touches nothing past
    /// [`CLEAR_DISC_OUT_PX`]. It is inert outside [`draw`].
    #[test]
    fn the_clear_disc_cuts_alpha_around_the_rat() {
        let floor: Rgb = (200, 200, 200);
        let paint = |disc: Option<(i32, i32)>, x: i32, y: i32| -> Rgb {
            let mut pb = PixelBuffer::new(96, 96);
            pb.clear(floor);
            if let Some(c) = disc {
                clear_disc_set(c);
            }
            blend(&mut pb, x, y, (0, 0, 0), ALPHA_MAX);
            clear_disc_reset();
            pb.pixel_at(x as usize, y as usize)
        };
        let plain = paint(None, 40, 40);
        let inside = paint(Some((40, 40)), 40, 40);
        let ramp = paint(Some((40, 40)), 40 + 22, 40);
        let outside = paint(Some((40, 40)), 40 + 34, 40);
        assert!(lum(inside) > lum(plain), "the disc did not cut the alpha");
        assert!(
            lum(ramp) > lum(plain) && lum(ramp) < lum(inside),
            "the disc ramp is not between the centre and the untouched pixel"
        );
        assert_eq!(outside, plain, "outside the disc the alpha must be untouched");
        assert_eq!(paint(None, 40, 40), plain, "the disc was left set");
    }

    /// The coverage tiers: full alpha below the cap, half in the grace band,
    /// skipped past it.
    #[test]
    fn the_coverage_tiers_drop_the_layer_to_half_then_nothing() {
        let (full, skip) = (220u32, 300u32);
        assert_eq!(coverage_alpha(200, 0, full, skip), Some(200));
        assert_eq!(coverage_alpha(200, full - 1, full, skip), Some(200));
        assert_eq!(coverage_alpha(200, full, full, skip), Some(100));
        assert_eq!(coverage_alpha(200, skip - 1, full, skip), Some(100));
        assert_eq!(coverage_alpha(200, skip, full, skip), None);
    }

    /// A hostile pellet under an item survives the frame: the layer is drawn
    /// **before** the hostile layer, so the pellet still marks its pixel on top
    /// of the layer's paint.
    #[test]
    fn a_pellet_under_an_item_is_never_overwritten() {
        let mut w = World::new(Theme::Sewer, 7);
        w.player.pos = Vec2::new(90.0, 60.0);
        let mut layer = ForeLayer::new();
        layer.force_items(vec![mk(
            ForeKind::MainPipe,
            w.player.pos,
            200.0,
            true,
            Attach::Span,
            2.0,
            5.0,
            1,
        )]);
        w.foreground_layer = layer;
        w.set_zoom_instant(1.0);

        // (1) the frame with the layer on, (2) the same frame with it off.
        w.foreground = true;
        let mut with_layer = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut with_layer);
        w.foreground = false;
        let mut bare = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut bare);
        // (3) the layer on again, with a hostile pellet under it.
        w.foreground = true;
        w.spawn_bullet(Bullet::new(
            Vec2::new(96.0, 60.0),
            Vec2::new(0.0, 0.0),
            4.0,
            Faction::Enemy,
            (240, 120, 160),
        ));
        let mut with_pellet = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut with_pellet);

        let cam = w.cam;
        let px = (
            (96.0 - cam.x).round() as usize,
            (60.0 - cam.y).round() as usize,
        );
        assert!(
            changed(&with_layer, &bare) > 0,
            "the layer painted nothing, so the test proves nothing"
        );
        let before = with_layer.pixel_at(px.0, px.1);
        let after = with_pellet.pixel_at(px.0, px.1);
        assert_ne!(
            before, after,
            "the pellet's pixel did not change: the layer hid it"
        );
        assert!(
            lum(after) > lum(before),
            "the pellet was dimmed ({before:?} → {after:?}); the hostile layer must be on top"
        );
    }

    /// A still frame changes nothing: two draws with the same camera and the
    /// same sway step produce the same pixels, and a new sway step (which is
    /// what the 3 Hz clock does) is what moves them.
    #[test]
    fn a_still_camera_changes_no_pixel_until_the_sway_step() {
        let (mut w, mut layer) = layer_for(Theme::Laundry, 3);
        let cam = Vec2::new(300.0, 240.0);
        let rat = cam + Vec2::new(200.0, 200.0);
        // One swaying line in the middle of the view, so the test does not
        // depend on where this seed happened to hang one.
        let item = mk(
            ForeKind::Clothesline,
            cam + Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5),
            240.0,
            true,
            Attach::Span,
            2.0,
            4.0,
            5,
        );
        layer.force_items(vec![item]);

        let t0 = 1.0f32;
        let s0 = sway_of(&item, t0);
        let mut t1 = t0;
        for k in 1..24 {
            let cand = t0 + k as f32 / SWAY_HZ + 0.01;
            if (sway_of(&item, cand) - s0).abs() >= 0.4 {
                t1 = cand;
                break;
            }
        }
        assert!(t1 > t0, "no sway step moved this item in 8 s");

        let (a, used_a) = render(&mut w, &mut layer, cam, rat, t0);
        let (b, used_b) = render(&mut w, &mut layer, cam, rat, t0 + 0.9 / SWAY_HZ);
        assert_eq!(used_a, used_b);
        assert_eq!(changed(&a, &b), 0, "a still frame repainted");
        let (c, _used_c) = render(&mut w, &mut layer, cam, rat, t1);
        assert!(changed(&a, &c) > 0, "the sway step moved nothing");
    }

    /// The outdoor floor's sky is a pure function of `(seed, elapsed)` — and of
    /// nothing else — so a held frame is still and two calls agree.
    #[test]
    fn the_sky_tracks_are_pure_and_still() {
        let w = World::new(Theme::Trailer, 5);
        let cam = Vec2::new(300.0, 200.0);
        let size = (640.0f32, 360.0f32);
        let mut a = Vec::new();
        let mut b = Vec::new();
        sky_items(&w.level, cam, size, 12.5, &mut a);
        sky_items(&w.level, cam, size, 12.5, &mut b);
        assert_eq!(a, b, "the sky is not a function of (seed, elapsed)");
        assert!(!a.is_empty(), "the outdoor floor has no sky");
        assert!(
            a.iter().any(|i| i.kind == ForeKind::PowerLine),
            "no power line"
        );
        assert!(
            a.iter().any(|i| i.kind == ForeKind::Cloud),
            "no cloud"
        );
        for item in &a {
            assert_eq!(item.attach, Attach::Sky, "a sky mover is not Sky");
            if item.kind == ForeKind::Cloud {
                assert!(
                    (1.0..=1.6).contains(&item.scale),
                    "a cloud's scale {} is outside 1.0..=1.6",
                    item.scale
                );
            }
        }
        // A frozen clock is a frozen sky …
        let mut c = Vec::new();
        sky_items(&w.level, cam, size, 12.5, &mut c);
        assert_eq!(a, c, "the sky moved with a frozen clock");
        // … and later it has moved.
        let mut moved = false;
        for step in 1..31 {
            let mut d = Vec::new();
            sky_items(&w.level, cam, size, 12.5 + step as f32 / SKY_QUANT_HZ, &mut d);
            if d != a {
                moved = true;
                break;
            }
        }
        assert!(moved, "nothing in the sky ever moves");
        // An indoor floor has no sky movers at all.
        let indoor = World::new(Theme::Sewer, 5);
        let mut none = Vec::new();
        sky_items(&indoor.level, cam, size, 12.5, &mut none);
        assert!(none.is_empty(), "an indoor floor has sky movers");
    }

    /// A cloud is a haze, never a silhouette: its planned alpha is capped at
    /// [`CLOUD_ALPHA_MAX`] whatever the fades say.
    #[test]
    fn a_cloud_is_only_ever_a_haze() {
        let mut w = World::new(Theme::Trailer, 5);
        let mut layer = ForeLayer::new();
        layer.force_items(vec![mk(
            ForeKind::Cloud,
            Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5),
            0.0,
            true,
            Attach::Sky,
            0.55,
            1.3,
            4,
        )]);
        let (_, used) = render(
            &mut w,
            &mut layer,
            Vec2::ZERO,
            Vec2::new(-9000.0, -9000.0),
            0.0,
        );
        assert!(used > 0, "the cloud painted nothing");
        // The outdoor floor also emits its own sky, so check every cloud in the
        // plan (the forced one and the two the sky track adds).
        let mut clouds = 0;
        for plan in layer.frame() {
            if layer.planned[plan.item].kind != ForeKind::Cloud {
                continue;
            }
            clouds += 1;
            assert!(
                plan.alpha <= CLOUD_ALPHA_MAX,
                "a cloud at alpha {} is not a haze",
                plan.alpha
            );
        }
        assert!(clouds > 0, "no cloud reached the plan");
    }

    /// Every kind is reachable and has a positive extent: the art's `extent` is
    /// what the cull and the plan rely on.
    #[test]
    fn every_kind_has_an_extent_and_is_used_by_a_floor() {
        for kind in ForeKind::all() {
            // A span or an edge needs a length; a drop and a cloud do not.
            let len = if kind.attach_class() == ATTACH_SPAN
                || kind.attach_class() == ATTACH_EDGE
            {
                200.0
            } else {
                0.0
            };
            let attach = match kind.attach_class() {
                ATTACH_SPAN => Attach::Span,
                ATTACH_EDGE => Attach::Edge(Side::E),
                ATTACH_DROP => Attach::Drop,
                _ => Attach::Sky,
            };
            let item = mk(kind, Vec2::ZERO, len, true, attach, 2.0, 4.0, 0);
            let (ex, ey) = art_foreground::extent(&item);
            assert!(ex > 0.0 && ey > 0.0, "{kind:?} has no extent");
        }
        // Every theme's three kinds are real kinds, and no kind is left over.
        let mut used = 0;
        for theme in Theme::playable() {
            for kind in kinds_for(theme) {
                assert!(ForeKind::all().contains(&kind), "{theme:?}: {kind:?}");
                used += 1;
            }
        }
        assert_eq!(used, ForeKind::all().len(), "a kind no floor uses");
        // Every kind sways or does not, and every kind names one attach class.
        for kind in ForeKind::all() {
            assert!(kind.attach_class() <= ATTACH_SKY, "{kind:?}");
            let _ = kind.sways();
        }
    }

    /// Draw one frame into a buffer of `size` over `floor`, the rat far away.
    fn render_sized(
        w: &mut World,
        layer: &mut ForeLayer,
        cam: Vec2,
        size: (usize, usize),
        floor: Rgb,
        elapsed: f32,
    ) -> PixelBuffer {
        let mut pb = PixelBuffer::new(size.0, size.1);
        pb.clear(floor);
        let mut scene = ForeScene {
            ecs: &mut w.ecs,
            level: &w.level,
            cam,
            size: (size.0 as f32, size.1 as f32),
            rat: Vec2::new(-9000.0, -9000.0),
            elapsed,
        };
        draw(&mut pb, layer, &mut scene);
        pb
    }

    /// Every pixel transition along a row or a column falls on the item's block
    /// grid: `(x - plan.screen.x) % k == 0`. Returns how many transitions it saw.
    fn assert_on_block_grid(pb: &PixelBuffer, origin: (i32, i32), k: i32, what: &str) -> usize {
        let mut seen = 0;
        for y in 0..pb.h {
            for x in 1..pb.w {
                if pb.pixel_at(x, y) != pb.pixel_at(x - 1, y) {
                    seen += 1;
                    assert_eq!(
                        (x as i32 - origin.0).rem_euclid(k),
                        0,
                        "{what}: a column edge at x {x} is off the {k}-px grid of the item at {origin:?}"
                    );
                }
            }
        }
        for x in 0..pb.w {
            for y in 1..pb.h {
                if pb.pixel_at(x, y) != pb.pixel_at(x, y - 1) {
                    seen += 1;
                    assert_eq!(
                        (y as i32 - origin.1).rem_euclid(k),
                        0,
                        "{what}: a row edge at y {y} is off the {k}-px grid of the item at {origin:?}"
                    );
                }
            }
        }
        seen
    }

    /// The upscale factor: 2 below depth 2.6, 3 from it, a cloud always 2.
    #[test]
    fn the_pixel_scale_is_two_or_three_by_depth() {
        let at = |kind, depth| mk(kind, Vec2::ZERO, 100.0, true, Attach::Span, depth, 2.5, 0);
        assert_eq!(pixel_scale(&at(ForeKind::MainPipe, 1.8)), 2);
        assert_eq!(pixel_scale(&at(ForeKind::MainPipe, 2.59)), 2);
        assert_eq!(pixel_scale(&at(ForeKind::MainPipe, 2.6)), 3);
        assert_eq!(pixel_scale(&at(ForeKind::MainPipe, 3.0)), 3);
        assert_eq!(pixel_scale(&at(ForeKind::Cloud, 3.0)), 2);
        // The art gets len / k, and the screen box is the art's box times k.
        let item = at(ForeKind::VentDuct, 2.8);
        let art = art_item(&item, 3);
        assert!((art.len - 100.0 / 3.0).abs() < 1e-4);
        let (ax, ay) = art_foreground::extent(&art);
        let (sx, sy) = screen_extent(&item);
        assert!((sx - ax * 3.0).abs() < 1e-3 && (sy - ay * 3.0).abs() < 1e-3);
        // A span's on-screen length is still its world length.
        assert!((sx - 50.0).abs() < 1.0, "a 100 px span is {sx} px half-long on screen");
    }

    /// **Chunky pixels**: a drawn item is made of uniform `k x k` blocks. Every
    /// colour edge of the body lies on the item's own `k` grid, and there are
    /// edges to check, for both factors. (A black floor: the shadow plane is a
    /// darken, so it cannot show, and its own grid is phased to `plan.shadow`.)
    #[test]
    fn a_drawn_item_is_made_of_uniform_k_by_k_blocks() {
        for (depth, k) in [(2.0f32, 2), (2.8f32, 3)] {
            let mut w = World::new(Theme::Lab, 5);
            let mut layer = ForeLayer::new();
            let size = (240usize, 200usize);
            let item = mk(
                ForeKind::VentDuct,
                Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5),
                200.0,
                true,
                Attach::Span,
                depth,
                2.0,
                9,
            );
            layer.force_items(vec![item]);
            let pb = render_sized(&mut w, &mut layer, Vec2::ZERO, size, (0, 0, 0), 0.0);
            let plan = *layer.frame().first().expect("a plan");
            assert_eq!(plan.k, k);
            let edges = assert_on_block_grid(&pb, plan.screen, k, "vent duct");
            assert!(edges > 40, "only {edges} edges: the item barely drew");
            // And every block really is uniform.
            let (ox, oy) = (plan.screen.0.rem_euclid(k), plan.screen.1.rem_euclid(k));
            let mut blocks = 0;
            let mut by = oy;
            while by + k <= size.1 as i32 {
                let mut bx = ox;
                while bx + k <= size.0 as i32 {
                    let c = pb.pixel_at(bx as usize, by as usize);
                    for dy in 0..k {
                        for dx in 0..k {
                            assert_eq!(
                                pb.pixel_at((bx + dx) as usize, (by + dy) as usize),
                                c,
                                "the block at ({bx}, {by}) is not uniform (k {k})"
                            );
                        }
                    }
                    if c != (0, 0, 0) {
                        blocks += 1;
                    }
                    bx += k;
                }
                by += k;
            }
            assert!(blocks > 100, "only {blocks} painted blocks");
        }
    }

    /// See-through structures keep real holes after the upscale: a catwalk
    /// grating shows the floor through it, as whole blocks.
    #[test]
    fn a_grating_keeps_its_holes_after_the_upscale() {
        let mut w = World::new(Theme::Sewer, 5);
        let mut layer = ForeLayer::new();
        let size = (240usize, 200usize);
        let floor: Rgb = (0, 0, 0);
        let item = mk(
            ForeKind::CatwalkGrating,
            Vec2::new(120.0, 100.0),
            200.0,
            true,
            Attach::Span,
            2.0,
            3.0,
            4,
        );
        layer.force_items(vec![item]);
        let pb = render_sized(&mut w, &mut layer, Vec2::ZERO, size, floor, 0.0);
        let plan = *layer.frame().first().expect("a plan");
        // On the body's rows, between its first and last lit pixel, some pixels
        // are untouched floor: holes the floor shows through (a black floor, so
        // the shadow, a darken, cannot fill them).
        let (_, hy) = screen_extent(&item);
        let mut holes = 0;
        let y0 = (plan.screen.1 - (hy * 0.6) as i32).max(0);
        let y1 = (plan.screen.1 + (hy * 0.6) as i32).min(size.1 as i32 - 1);
        for y in y0..=y1 {
            let lit: Vec<usize> = (0..size.0).filter(|&x| pb.pixel_at(x, y as usize) != floor).collect();
            if let (Some(&a), Some(&b)) = (lit.first(), lit.last()) {
                holes += (a..=b).filter(|&x| pb.pixel_at(x, y as usize) == floor).count();
            }
        }
        assert!(holes > 0, "the upscaled grating has no holes");
        assert_on_block_grid(&pb, plan.screen, plan.k, "grating");
    }

    /// Block grid stability: two camera positions 1 px apart put the block
    /// edges at the same phase relative to the item (they move with it, never
    /// crawl across it).
    #[test]
    fn block_edges_keep_their_phase_when_the_camera_moves_a_pixel() {
        let mut w = World::new(Theme::Kitchen, 5);
        let mut layer = ForeLayer::new();
        let size = (240usize, 200usize);
        let item = mk(
            ForeKind::HangingLamp,
            Vec2::new(120.0, 110.0),
            0.0,
            true,
            Attach::Drop,
            2.7,
            2.4,
            3,
        );
        layer.force_items(vec![item]);
        let mut origins = Vec::new();
        for cam in [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)] {
            let pb = render_sized(&mut w, &mut layer, cam, size, (0, 0, 0), 0.0);
            let plan = *layer.frame().first().expect("a plan");
            let edges = assert_on_block_grid(&pb, plan.screen, plan.k, "lamp");
            assert!(edges > 20, "the lamp barely drew");
            origins.push(plan.screen);
        }
        assert_ne!(origins[0], origins[1], "the camera move did not move the item");
    }

    /// A span still ends **in its walls** after the upscale: on generated Sewer
    /// levels, the outermost body pixel at each end lands inside a solid wall
    /// tile, within `k + 1` px of the span's true end.
    #[test]
    fn a_span_blitted_end_lands_inside_its_wall() {
        let size = (160usize, 120usize);
        let mut checked = 0;
        for seed in [1u64, 2, 5, 9] {
            let (mut w, generated) = layer_for(Theme::Sewer, seed);
            let spans: Vec<ForeItem> = generated
                .items()
                .iter()
                .filter(|i| i.attach == Attach::Span)
                .copied()
                .take(4)
                .collect();
            for item in spans {
                let k = pixel_scale(&item);
                for dir in [1.0f32, -1.0] {
                    let along = if item.horizontal {
                        Vec2::new(1.0, 0.0)
                    } else {
                        Vec2::new(0.0, 1.0)
                    };
                    let end = item.pos + along * (item.len * 0.5 * dir);
                    // The end at the middle of the view; along the span the item
                    // rides the world 1:1, across it the camera centre sits on it.
                    let cam = end - Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5);
                    let mut layer = ForeLayer::new();
                    layer.force_items(vec![item]);
                    // A black floor: the shadow (a darken) cannot show, so every
                    // lit pixel is the body.
                    let pb = render_sized(&mut w, &mut layer, cam, size, (0, 0, 0), 0.0);
                    let Some(plan) = layer.frame().first().copied() else {
                        continue;
                    };
                    // Scan the rows (columns) through the anchor line and keep
                    // the outermost lit pixel on the `dir` side.
                    let mut outer: Option<i32> = None;
                    let run = if item.horizontal { size.0 } else { size.1 } as i32;
                    for cross in -6i32..=6 {
                        for t in 0..run {
                            let (x, y) = if item.horizontal {
                                (t, plan.screen.1 + cross)
                            } else {
                                (plan.screen.0 + cross, t)
                            };
                            if x < 0 || y < 0 || x >= size.0 as i32 || y >= size.1 as i32 {
                                continue;
                            }
                            if pb.pixel_at(x as usize, y as usize) != (0, 0, 0) {
                                let c = if item.horizontal { x } else { y };
                                outer = Some(match outer {
                                    Some(o) if dir > 0.0 => o.max(c),
                                    Some(o) => o.min(c),
                                    None => c,
                                });
                            }
                        }
                    }
                    let Some(edge) = outer else { continue };
                    let world_c = if item.horizontal {
                        cam.x + edge as f32 + 0.5
                    } else {
                        cam.y + edge as f32 + 0.5
                    };
                    let true_c = if item.horizontal { end.x } else { end.y };
                    assert!(
                        (world_c - true_c).abs() <= k as f32 + 1.0,
                        "seed {seed}: the blitted end is {} px from the span's end (k {k})",
                        (world_c - true_c).abs()
                    );
                    let probe = if item.horizontal {
                        Vec2::new(world_c, item.pos.y)
                    } else {
                        Vec2::new(item.pos.x, world_c)
                    };
                    assert!(
                        matches!(w.level.tiles.get_at(probe), TileKind::Wall(WallKind::Solid)),
                        "seed {seed}: the blitted end at {probe:?} is not inside a solid wall"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no span end was checked, so the test proves nothing");
    }

    /// No allocation growth once warm: the scratch reaches its capacity on the
    /// first frame and keeps it over ten more (a still camera, a running clock).
    #[test]
    fn the_scratch_does_not_grow_after_the_first_frame() {
        for theme in [Theme::Sewer, Theme::Laundry, Theme::Trailer] {
            let (mut w, mut layer) = layer_for(theme, 3);
            // Stand on an item so something draws.
            let at = layer.items().first().map(|i| i.pos).unwrap_or(Vec2::new(400.0, 300.0));
            let cam = at - Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5);
            let _ = render(&mut w, &mut layer, cam, cam - Vec2::new(500.0, 500.0), 1.0);
            let warm = layer.scratch_capacity();
            let frames = layer.frame.capacity();
            for i in 0..10 {
                // Indoors the clock only steps the sway (no window changes);
                // the Flatbed's movers change their windows, so hold its clock.
                let t = if theme == Theme::Trailer { 1.0 } else { 1.0 + i as f32 * 0.4 };
                let _ = render(&mut w, &mut layer, cam, cam - Vec2::new(500.0, 500.0), t);
                assert_eq!(layer.scratch_capacity(), warm, "{theme:?}: the scratch grew on frame {i}");
                assert_eq!(layer.frame.capacity(), frames, "{theme:?}: the plan grew on frame {i}");
            }
        }
    }

    /// With the layer off nothing is drawn: a world holding a huge item over the
    /// rat draws exactly the picture it draws with no items at all.
    #[test]
    fn with_the_layer_off_nothing_is_drawn() {
        let mut w = World::new(Theme::Sewer, 7);
        w.set_zoom_instant(1.0);
        let mut layer = ForeLayer::new();
        layer.force_items(vec![mk(
            ForeKind::MainPipe,
            w.player.pos,
            300.0,
            true,
            Attach::Span,
            2.8,
            3.0,
            1,
        )]);
        w.foreground_layer = layer;
        w.foreground = false;
        let mut off = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut off);
        let mut empty = ForeLayer::new();
        empty.force_items(Vec::new());
        w.foreground_layer = empty;
        w.foreground = true;
        let mut none = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut none);
        assert_eq!(changed(&off, &none), 0, "the switched-off layer drew something");
    }

    /// A still scene (fixed clock, fixed camera) draws identically twice, on
    /// every floor including the Flatbed's sky.
    #[test]
    fn a_still_scene_draws_identically_twice() {
        for theme in Theme::playable() {
            let (mut w, mut layer) = layer_for(theme, 4);
            let at = layer.items().first().map(|i| i.pos).unwrap_or(Vec2::new(400.0, 300.0));
            let cam = at - Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5);
            let rat = at + Vec2::new(0.0, 30.0);
            let (a, ua) = render(&mut w, &mut layer, cam, rat, 6.25);
            let (b, ub) = render(&mut w, &mut layer, cam, rat, 6.25);
            assert_eq!(ua, ub, "{theme:?}");
            assert_eq!(changed(&a, &b), 0, "{theme:?}: a still scene repainted");
        }
    }

    /// The rat's hard core: nothing at all is written within [`RAT_SKIP_PX`].
    #[test]
    fn nothing_is_written_within_ten_pixels_of_the_rat() {
        let floor: Rgb = (90, 90, 90);
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(floor);
        clear_disc_set((32, 32));
        for (x, y) in [(32, 32), (32 + 9, 32), (32, 32 - 7), (32 + 6, 32 + 6)] {
            blend(&mut pb, x, y, (0, 0, 0), ALPHA_MAX);
            assert_eq!(pb.pixel_at(x as usize, y as usize), floor, "({x}, {y}) was painted");
        }
        blend(&mut pb, 32 + 12, 32, (0, 0, 0), ALPHA_MAX);
        clear_disc_reset();
        assert_ne!(pb.pixel_at(44, 32), floor, "the disc ramp past the core painted nothing");
    }

    /// The Flatbed's wires are in **every** frame: the wire item is always in
    /// the plan (pinned inside the view), and the drawn lines cross the whole
    /// view's width in `k`-px blocks around their pinned bands.
    #[test]
    fn the_power_wires_are_in_every_flatbed_frame() {
        let w = World::new(Theme::Trailer, 5);
        let size = (VIEW.0 as f32, VIEW.1 as f32);
        for t in [0.0f32, 0.7, 1.9, 3.3, 4.0, 7.1, 13.37] {
            for cam in [Vec2::ZERO, Vec2::new(500.0, 300.0), Vec2::new(1234.5, 77.7)] {
                let sky = sky_probe(&w.level, cam, size, t);
                let wire = sky.iter().find(|i| is_wire_span(i)).expect("no wire item");
                let p = wire.pos - cam;
                assert!(
                    p.x >= 0.0 && p.x <= size.0 && p.y >= 0.0 && p.y <= size.1,
                    "t {t} cam {cam:?}: the wires are pinned at {p:?}, outside the view"
                );
            }
            let floor: Rgb = (40, 36, 30);
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            pb.clear(floor);
            draw_wires(&mut pb, PIXEL_SCALE, SKY_ALPHA as u8, t);
            for &band in &WIRE_BANDS {
                let base = wire_base_y(VIEW.1 as f32, band, PIXEL_SCALE);
                let lo = base - (WIRE_SAG_ART as i32 + 1) * PIXEL_SCALE;
                let hi = base + (WIRE_SAG_ART as i32 + 1) * PIXEL_SCALE;
                for x in 0..VIEW.0 {
                    let hit = (lo.max(0)..hi.min(VIEW.1 as i32)).any(|y| {
                        let c = pb.pixel_at(x, y as usize);
                        lum(c) > lum(floor) + 120
                    });
                    assert!(hit, "t {t}: column {x} has no wire near the {band} band");
                }
            }
        }
    }
}
