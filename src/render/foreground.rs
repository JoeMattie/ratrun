//! Parallax **foreground**: overhead things nearer the viewer than the floor.
//!
//! The owner asked for "a foreground layer in some cases with parallax". This
//! module is the layer; [`crate::game::art_foreground`] is the art, one item at
//! a time. Nothing here is simulation state: the layer is generated from the
//! finished level, drawn into the world-resolution picture, and read by no
//! system — so it cannot move a golden or the map pin (see *Determinism*).
//!
//! ## What it is
//!
//! A run of overhead objects placed from `(theme, level seed, the level's
//! rooms/corridors)` once per level ([`ForeLayer::sync`]), in three roles:
//!
//! * **corridor runs** — a pipe / cable / line following the corridor's long
//!   axis, one corridor in two;
//! * **doorway hangers or beams** — one across the middle of a doorway, one
//!   doorway in two;
//! * **point items** in large rooms (up to three a room).
//!
//! Never over the start room's centre (96 px), never within 5 tiles (80 px) of
//! a mouse-hole shop. The per-theme kinds are [`kinds_for`].
//!
//! **No dependence on a boss arena.** The layer reads rooms, tile kinds and shop
//! holes only — never `Level::boss` and never a `RoomKind` the like of the boss
//! arena's — so removing arenas from the levels cannot break it (batch 12: they
//! were removed; a former arena is just a large room here, and it gets the same
//! at most three point items as any other).
//!
//! ## Where in the frame
//!
//! `World::draw_scene` calls [`draw`] **after the lighting pass and before
//! `World::draw_hostile_layer`**: pellets, telegraphs, tells, miniboss shapes,
//! damage digits and the ink pass are always stamped on top, so the layer can
//! never hide the fight. It is drawn in the world picture (before the zoom
//! resample), so it works at every zoom rung including sub-cell detail.
//!
//! ## Parallax
//!
//! An item at world position `P` is drawn at
//! `screen = centre + (P - cam_centre) * depth`, with `depth` in `1.15..=1.6`:
//! it slides faster than the floor and is drawn larger by `scale = depth`. The
//! camera is the one the scene drew with, shake offset included, so the shake
//! is amplified slightly on the overhead layer (which reads as depth). The
//! shadow stays at the item's **un-parallaxed** floor position — the depth cue.
//!
//! ## Never hide the fight
//!
//! * Every pixel goes through [`blend`], which **clamps the alpha to
//!   [`ALPHA_MAX`]** (150/255): no pixel of this layer is ever opaque.
//! * The whole item fades to [`ALPHA_FLOOR`] (40) within [`FADE_RAT_PX`] of the
//!   rat and within [`FADE_THREAT_PX`] of a miniboss/boss body, ramping back
//!   over [`FADE_RAMP_PX`]. The distance is measured from the rat to the item's
//!   nearest on-screen pixel (the art takes one alpha for the whole item).
//! * The layer is **darkened by a fixed factor** ([`DARKEN`]) inside [`blend`]
//!   rather than lit per pixel: overhead things read as dark silhouettes
//!   against the lit floor whatever colours the art hands over. [`rim_tint`]
//!   gives the art the floor's ambient tint for a rim (§ the brief).
//! * The floor shadow is a *darken* ([`shadow`]), drawn after the lighting
//!   pass, so it costs no extra pass and can never brighten anything.
//!
//! ## Cost and stillness
//!
//! * **Coverage cap.** A frame may paint at most [`COVERAGE_PCT`] percent of
//!   the view's pixels. This is enforced for real, not estimated: [`blend`]
//!   counts every pixel it writes and refuses to write past the frame's budget
//!   (a thread-local counter, so a test or a one-off call outside [`draw`] is
//!   unlimited). No art can break the cap.
//! * **No allocation per frame.** The item list is built once per level; the
//!   per-frame plan is a `Vec` that is cleared and refilled (its capacity is
//!   reached on the first frame a level draws), and the threat list is a fixed
//!   eight-entry array on the stack.
//! * **Cost estimate** (unmeasured: this batch does not build) at the 120 × 40
//!   reference view (120 × 80 = 9 600 world pixels): a typical frame paints
//!   400–2 500 pixels through two thread-local touches each, and plans ~130
//!   items with one box test each — about 20–60 µs, well under the 0.15 ms
//!   budget. The three per-level sorts and the generation run once.
//! * **Stillness.** With the camera still and no sway step, the layer writes
//!   the same pixels with the same colours, so the presenter sees an unchanged
//!   frame and sends nothing. Sway is stepped at [`SWAY_HZ`] (3 Hz) and only
//!   for the kinds that sway, so a still scene repaints the layer's cells at
//!   most three times a second; the palette's own still-frame rule then keeps
//!   the rest of the terminal untouched.
//!
//! ## Determinism
//!
//! [`generate`] is a pure function of the finished [`Level`] (its theme, seed,
//! start, rooms, tile kinds and shop holes) with its own splitmix stream
//! salted away from the map's, so it draws no simulation rng and touches no
//! simulation state. The item list is kept on `World` as **render state**
//! (like `light_map`/`detail`), not as a resource, and [`World::draw`] never
//! writes anything a system reads. The goldens (`game/ecs/harness.rs`) fold
//! simulation state only and `levelgen`'s `TRACE_HASHES` pins `generate_once`,
//! which this module only reads — so neither can move.

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

// ---- Public tuning constants ---------------------------------------------

/// The largest alpha any pixel of this layer can reach (150/255). [`blend`]
/// clamps to it, so nothing in the layer is ever opaque.
pub const ALPHA_MAX: u8 = 150;
/// The dimmest the rat/threat fade takes an item to (out of 255).
pub const ALPHA_FLOOR: f32 = 40.0;
/// Fade to [`ALPHA_FLOOR`] within this many pixels of the rat.
pub const FADE_RAT_PX: f32 = 36.0;
/// … and within this many pixels of a miniboss/boss body.
pub const FADE_THREAT_PX: f32 = 28.0;
/// … ramping back to full over this many pixels.
pub const FADE_RAMP_PX: f32 = 24.0;
/// Sway is stepped at this rate (Hz), for the kinds that sway only.
pub const SWAY_HZ: f32 = 3.0;
/// How far the whole layer may paint per frame, as a percentage of the view.
pub const COVERAGE_PCT: u32 = 8;
/// The fixed silhouette darkening [`blend`] applies to every colour.
pub const DARKEN: f32 = 0.72;
/// How far [`shadow`] may darken a floor pixel at its strongest.
pub const SHADOW_MAX: f32 = 0.55;

/// Placement keep-out: never within this many world px of a shop hole's face
/// (5 tiles).
const CLEAR_PX: f32 = 5.0 * TILE_F;
/// Placement keep-out around the start room's centre (6 tiles).
const START_CLEAR_PX: f32 = 6.0 * TILE_F;
/// The layer's own splitmix salt, so its stream is not the map's.
const GEN_SALT: u32 = 0x0F0E_6000;
/// The most items a level can hold (a safety net; a real level has a few dozen).
const MAX_ITEMS: usize = 240;
/// The radius given to a boss body for the fade (the art's bodies are ~27 px
/// across; this is deliberately generous).
const BOSS_BODY_RADIUS: f32 = 20.0;

// ---- The item types (the art's contract) ----------------------------------

/// One overhead kind. Each floor uses three (see [`kinds_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeKind {
    // Sewer
    PipeRun,
    Grate,
    DripChain,
    // Kitchen
    PotRack,
    ShelfEdge,
    HangingUtensils,
    // Lab
    CableTray,
    VentDuct,
    HangingLampCord,
    // Laundry
    Clothesline,
    SheetOnLine,
    SteamBank,
    // Printworks
    PaperWeb,
    HangingProofs,
    RollerBeam,
    // Flatbed
    TarpStrap,
    SignGantryShadow,
    CargoNetEdge,
}

impl ForeKind {
    /// Every kind, in declaration order (the art worker's checklist).
    pub const fn all() -> [ForeKind; 18] {
        [
            ForeKind::PipeRun,
            ForeKind::Grate,
            ForeKind::DripChain,
            ForeKind::PotRack,
            ForeKind::ShelfEdge,
            ForeKind::HangingUtensils,
            ForeKind::CableTray,
            ForeKind::VentDuct,
            ForeKind::HangingLampCord,
            ForeKind::Clothesline,
            ForeKind::SheetOnLine,
            ForeKind::SteamBank,
            ForeKind::PaperWeb,
            ForeKind::HangingProofs,
            ForeKind::RollerBeam,
            ForeKind::TarpStrap,
            ForeKind::SignGantryShadow,
            ForeKind::CargoNetEdge,
        ]
    }

    /// Does this kind swing, and so take a non-zero [`ForeDraw::sway`]?
    /// Lines, chains, sheets and steam only — everything else is nailed up, and
    /// a still scene with nothing swaying in view makes the terminal repaint
    /// nothing.
    pub const fn sways(self) -> bool {
        matches!(
            self,
            ForeKind::DripChain
                | ForeKind::HangingLampCord
                | ForeKind::Clothesline
                | ForeKind::SheetOnLine
                | ForeKind::SteamBank
                | ForeKind::HangingProofs
                | ForeKind::PaperWeb
        )
    }

    /// Hangs from above (its long axis is vertical when it hangs in a doorway)
    /// rather than spanning the opening (a beam, long axis horizontal).
    pub const fn hangs(self) -> bool {
        !matches!(
            self,
            ForeKind::PotRack
                | ForeKind::RollerBeam
                | ForeKind::SignGantryShadow
                | ForeKind::ShelfEdge
                | ForeKind::Grate
        )
    }
}

/// One placed overhead object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForeItem {
    pub kind: ForeKind,
    /// Anchor in **world** pixels: where it hangs over.
    pub pos: Vec2,
    /// Long axis in world px (runs, lines, beams); `0` for point items.
    pub len: f32,
    /// Orientation of the long axis.
    pub horizontal: bool,
    /// Parallax factor: `1.15` (just overhead) ..= `1.6` (near the eye). Also
    /// the draw scale.
    pub depth: f32,
    /// Per-item variation (art detail, sway phase).
    pub seed: u32,
}

/// What the art function is handed for one item, one frame.
#[derive(Clone, Copy, Debug)]
pub struct ForeDraw {
    /// Where the anchor lands in the buffer after the parallax transform.
    pub screen: (i32, i32),
    /// Draw scale: equal to the item's depth (nearer things are larger).
    pub scale: f32,
    /// Overall opacity after the rat/threat fade, `0..=`[`ALPHA_MAX`].
    pub alpha: u8,
    /// `-1.0..=1.0`, stepped at [`SWAY_HZ`]; `0.0` for a kind that never sways.
    pub sway: f32,
    /// The floor, for palette and rim tint ([`rim_tint`]).
    pub theme: Theme,
}

/// The rim tint the brief asks for: the floor's ambient character colour, dim
/// enough to stay a rim on a dark silhouette.
pub fn rim_tint(theme: Theme) -> Rgb {
    palette::scale(crate::render::lighting::theme_ambient(theme).rat_tint, 0.30)
}

/// The three kinds a floor's foreground uses, in role order:
/// `[corridor run, doorway hanger/beam, room point item]`.
pub const fn kinds_for(theme: Theme) -> [ForeKind; 3] {
    match theme {
        Theme::Sewer => [ForeKind::PipeRun, ForeKind::DripChain, ForeKind::Grate],
        Theme::Kitchen => [ForeKind::ShelfEdge, ForeKind::PotRack, ForeKind::HangingUtensils],
        Theme::Lab => [ForeKind::CableTray, ForeKind::HangingLampCord, ForeKind::VentDuct],
        Theme::Laundry => [ForeKind::Clothesline, ForeKind::SheetOnLine, ForeKind::SteamBank],
        Theme::Printworks => [ForeKind::PaperWeb, ForeKind::HangingProofs, ForeKind::RollerBeam],
        Theme::Trailer => [
            ForeKind::CargoNetEdge,
            ForeKind::TarpStrap,
            ForeKind::SignGantryShadow,
        ],
    }
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

/// Alpha-blend `rgb` over the existing pixel at `(x, y)`, clipped to the
/// buffer. The **only** painter the layer's art may use.
///
/// * The alpha is clamped to [`ALPHA_MAX`], so no pixel of this layer is ever
///   opaque (the fight can never be hidden).
/// * The colour is darkened by [`DARKEN`] first, so the layer reads as a
///   silhouette against the lit floor without a per-pixel lighting pass. Ask
///   [`rim_tint`] for a rim colour on top.
/// * It counts against the frame's coverage budget ([`COVERAGE_PCT`]); past it
///   the write is dropped, so the cap holds whatever the art paints.
#[inline]
pub fn blend(pb: &mut PixelBuffer, x: i32, y: i32, rgb: Rgb, alpha: u8) {
    let a = alpha.min(ALPHA_MAX);
    if a == 0 || x < 0 || y < 0 {
        return;
    }
    let (xu, yu) = (x as usize, y as usize);
    if xu >= pb.w || yu >= pb.h {
        return;
    }
    if !take_budget() {
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
    /// Index into [`ForeLayer::items`].
    pub item: usize,
    /// The anchor's screen position after the parallax transform.
    pub screen: (i32, i32),
    /// The anchor's screen position **without** parallax: the floor shadow.
    pub shadow: (i32, i32),
    /// Opacity after the rat/threat fade.
    pub alpha: u8,
    /// `-1.0..=1.0` sway for this frame (stepped at [`SWAY_HZ`]).
    pub sway: f32,
}

/// The layer's item list and its per-frame scratch. **Render state**: built
/// once per level, held on `World` next to `light_map`/`detail`, never a
/// simulation resource.
pub struct ForeLayer {
    items: Vec<ForeItem>,
    /// The `(theme, level seed)` the items were built for.
    key: Option<(Theme, u64)>,
    frame: Vec<Plan>,
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
            key: None,
            frame: Vec::new(),
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
        // deterministic and the frame loop needs no scratch list.
        self.items.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    }

    /// The placed items, in generation order.
    pub fn items(&self) -> &[ForeItem] {
        &self.items
    }

    /// This frame's plan, in draw order (far to near).
    pub fn frame(&self) -> &[Plan] {
        &self.frame
    }

    /// Replace the item list with a hand-built one and stop rebuilding from the
    /// level (tests: parallax, fade and ordering need known items).
    #[cfg(test)]
    pub(crate) fn force_items(&mut self, items: Vec<ForeItem>) {
        self.forced = true;
        self.items = items;
        self.frame.clear();
    }

    /// Plan one frame: cull with the art's own extent, apply the parallax
    /// transform, work out the fade and the sway step. Pure in its arguments
    /// (`cam`, `size`, `rat`, `elapsed` and the threat list only decide *where*
    /// and *how opaque*, never *what* is placed).
    ///
    /// Draw order is far to near (`depth` ascending), which is the only order
    /// in which two overlapping items look right, with the item's list index as
    /// the tie-break so the order is total and deterministic.
    fn plan_frame(
        &mut self,
        cam: Vec2,
        size: (f32, f32),
        rat: Vec2,
        elapsed: f32,
        threats: &[(Vec2, f32)],
    ) {
        self.frame.clear();
        if self.items.is_empty() || size.0 <= 0.0 || size.1 <= 0.0 {
            return;
        }
        let centre = Vec2::new(size.0 * 0.5, size.1 * 0.5);
        let cam_centre = cam + centre;
        let rat_screen = rat - cam;
        for (i, item) in self.items.iter().enumerate() {
            let item = *item;
            let (ex, ey) = art_foreground::extent(&item);
            let (hx, hy) = if item.horizontal {
                (ex.max(0.0), ey.max(0.0))
            } else {
                (ey.max(0.0), ex.max(0.0))
            };
            let s = item.depth.clamp(1.0, 2.0);
            let p = centre + (item.pos - cam_centre) * s;
            let (bx, by) = ((hx * s).max(1.0), (hy * s).max(1.0));
            // Cull: the art's box at the draw scale, plus the sway reach.
            const SWAY_REACH: f32 = 4.0;
            if p.x + bx + SWAY_REACH < 0.0
                || p.y + by + SWAY_REACH < 0.0
                || p.x - bx - SWAY_REACH >= size.0
                || p.y - by - SWAY_REACH >= size.1
            {
                continue;
            }
            // Fade: from the item's nearest on-screen pixel, because the art
            // takes one alpha for the whole item.
            let mut alpha = proximity_alpha(ALPHA_MAX as f32, box_dist(p, bx, by, rat_screen), FADE_RAT_PX);
            for &(body, radius) in threats {
                let d = (box_dist(p, bx, by, body - cam) - radius.max(0.0)).max(0.0);
                alpha = alpha.min(proximity_alpha(ALPHA_MAX as f32, d, FADE_THREAT_PX));
            }
            let shadow_at = item.pos - cam;
            self.frame.push(Plan {
                item: i,
                screen: (p.x.round() as i32, p.y.round() as i32),
                shadow: (shadow_at.x.round() as i32, shadow_at.y.round() as i32),
                alpha: alpha.round().clamp(0.0, ALPHA_MAX as f32) as u8,
                sway: sway_of(&item, elapsed),
            });
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
    if layer.items.is_empty() || pb.w == 0 || pb.h == 0 {
        return 0;
    }
    // The bodies the fade must keep clear of: every live miniboss's body and
    // the boss's. Capped at eight, which is more than a floor can hold (one
    // miniboss and one boss at a time).
    let mut threats = [(Vec2::ZERO, 0.0f32); 8];
    let mut n = 0usize;
    {
        let mut q = scene.ecs.query::<(&Enemy, Option<&Miniboss>)>();
        for (e, mb) in q.iter(&scene.ecs) {
            if mb.is_none() || n == threats.len() {
                continue;
            }
            threats[n] = (e.pos, e.radius.max(8.0));
            n += 1;
        }
    }
    {
        let mut q = scene.ecs.query::<&Boss>();
        for boss in q.iter(&scene.ecs) {
            if n == threats.len() {
                break;
            }
            threats[n] = (boss.pos, BOSS_BODY_RADIUS);
            n += 1;
        }
    }
    layer.plan_frame(
        scene.cam,
        scene.size,
        scene.rat,
        scene.elapsed,
        &threats[..n],
    );
    let budget = ((pb.w * pb.h) as u32) * COVERAGE_PCT / 100;
    budget_start(budget);
    // Shadows first — every one of them is on the *floor*, under every item —
    // then the items themselves, far to near. Both loops walk the same plan,
    // so the order is the sorted item order and nothing allocates.
    for plan in layer.frame.iter().copied() {
        let item = layer.items[plan.item];
        draw_shadow(pb, &item, plan.shadow);
    }
    for plan in layer.frame.iter().copied() {
        let item = layer.items[plan.item];
        let d = ForeDraw {
            screen: plan.screen,
            scale: item.depth.clamp(1.0, 2.0),
            alpha: plan.alpha,
            sway: plan.sway,
            theme: scene.level.theme,
        };
        art_foreground::draw(pb, &item, &d);
    }
    budget_finish()
}

/// The item's floor shadow: a soft ellipse at its **un-parallaxed** world
/// position, thin across its long axis, so the object visibly slides over the
/// floor while the shadow stays with the world.
fn draw_shadow(pb: &mut PixelBuffer, item: &ForeItem, at: (i32, i32)) {
    let (ex, ey) = art_foreground::extent(item);
    let (hx, hy) = if item.horizontal {
        (ex.max(0.0), ey.max(0.0))
    } else {
        (ey.max(0.0), ex.max(0.0))
    };
    let rx = hx.max(2.0);
    let ry = (hy * 0.45).max(1.0);
    let (irx, iry) = (rx.round() as i32, ry.round() as i32);
    for dy in -iry..=iry {
        for dx in -irx..=irx {
            let nx = dx as f32 / rx;
            let ny = dy as f32 / ry;
            let d2 = nx * nx + ny * ny;
            if d2 > 1.0 {
                continue;
            }
            shadow(pb, at.0 + dx, at.1 + dy, 0.65 * (1.0 - d2));
        }
    }
}

/// Alpha for an item whose nearest pixel is `d` px from the rat/a body:
/// [`ALPHA_FLOOR`] inside `near`, ramping back to `base` over
/// [`FADE_RAMP_PX`].
fn proximity_alpha(base: f32, d: f32, near: f32) -> f32 {
    if d <= near {
        ALPHA_FLOOR.min(base)
    } else if d >= near + FADE_RAMP_PX {
        base
    } else {
        let t = (d - near) / FADE_RAMP_PX;
        (ALPHA_FLOOR + (base - ALPHA_FLOOR) * t).min(base)
    }
}

/// Distance from `p` to the outside of the box `(bx, by)` around `p` (0 inside).
fn box_dist(p: Vec2, bx: f32, by: f32, q: Vec2) -> f32 {
    let dx = ((q.x - p.x).abs() - bx).max(0.0);
    let dy = ((q.y - p.y).abs() - by).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// Sway for `item` at `elapsed`: `0.0` for a kind that never sways, else a
/// value in `-1.0..=1.0` quantised to eighths and **stepped at
/// [`SWAY_HZ`]** — within one step the value is constant, so a still camera
/// does not repaint between steps.
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

/// Build the layer's items for `level`: a pure function of the finished level
/// (theme, seed, rooms, tile kinds, shop holes, boss floor) and its own rng.
/// Draws no simulation rng, so it cannot move a golden.
fn generate(level: &Level, out: &mut Vec<ForeItem>) {
    let theme = level.theme;
    let [run_kind, door_kind, point_kind] = kinds_for(theme);
    let mut rng = Gen::new(crate::game::level::level_seed(theme, level.seed, GEN_SALT));
    let start = level.start;
    let shops = &level.objects.shop_holes;

    // Keep-outs: the start room's centre and the mouse holes (a shelf needs a
    // clear threshold). Deliberately **nothing** about a boss arena: the layer
    // reads rooms, tile kinds and shop holes only, so removing arenas from the
    // levels (batch 12) cannot break it.
    let blocked = |p: Vec2| -> bool {
        p.dist(start) < START_CLEAR_PX || shops.iter().any(|h| h.pos.dist(p) < CLEAR_PX)
    };

    let push = |out: &mut Vec<ForeItem>, item: ForeItem| {
        if out.len() < MAX_ITEMS {
            out.push(item);
        }
    };

    // 1. Corridor runs: one corridor in two, along its long axis.
    for room in level.rooms.iter().filter(|r| r.kind == RoomKind::Corridor) {
        if !rng.flip() {
            continue;
        }
        let horizontal = room.rect.w >= room.rect.h;
        let long_cells = if horizontal { room.rect.w } else { room.rect.h };
        let long_px = long_cells as f32 * TILE_F;
        // Shorter than the corridor (the ends are the doorways' business).
        let len = (long_px * 0.85).clamp(TILE_F * 1.5, 128.0);
        let centre = room.rect.center_world();
        if blocked(centre) {
            continue;
        }
        let depth = rng.range(1.15, 1.40);
        let seed = rng.u32();
        push(
            out,
            ForeItem {
                kind: run_kind,
                pos: centre,
                len,
                horizontal,
                depth,
                seed,
            },
        );
    }

    // 2. Doorway hangers/beams: one doorway in two, at the opening's middle.
    // Doorways are wall cells (`WallKind::Doorway`), so they come from the tile
    // grid rather than the room list; a two-cell opening is one doorway.
    {
        use crate::game::art_walls::{WallAxis, WallKind};
        let (cols, rows) = (level.tiles.cols(), level.tiles.rows());
        for cy in 1..rows - 1 {
            for cx in 1..cols - 1 {
                let Some(WallKind::Doorway { axis }) = level.tiles.get(cx, cy).wall_kind() else {
                    continue;
                };
                // The second cell of a two-cell opening shares the first's run.
                let prev = match axis {
                    WallAxis::Horizontal => (cx - 1, cy),
                    WallAxis::Vertical => (cx, cy - 1),
                };
                if matches!(
                    level.tiles.get(prev.0, prev.1).wall_kind(),
                    Some(WallKind::Doorway { axis: a }) if a == axis
                ) {
                    continue;
                }
                if !rng.flip() {
                    continue;
                }
                let pos = Vec2::new(
                    (cx as f32 + 0.5) * TILE_F,
                    (cy as f32 + 0.5) * TILE_F,
                );
                if blocked(pos) {
                    continue;
                }
                let depth = rng.range(1.20, 1.50);
                let seed = rng.u32();
                push(
                    out,
                    ForeItem {
                        kind: door_kind,
                        pos,
                        len: TILE_F * 2.2,
                        horizontal: !door_kind.hangs(),
                        depth,
                        seed,
                    },
                );
            }
        }
    }

    // 3. Point items in the large rooms (three at most a room). A room's kind
    // is not read: a former boss arena is just a large room and gets the same
    // treatment as a hall.
    for room in &level.rooms {
        let cells = room.rect.area();
        if matches!(room.kind, RoomKind::Corridor | RoomKind::Maze) || cells < 36 {
            continue;
        }
        let n = (cells / 120).clamp(1, 3) as usize;
        if room.rect.w < 6 || room.rect.h < 6 {
            continue;
        }
        let (x0, y0) = (
            (room.rect.x + 2) as f32 * TILE_F,
            (room.rect.y + 2) as f32 * TILE_F,
        );
        let (x1, y1) = (
            (room.rect.right() - 2) as f32 * TILE_F,
            (room.rect.bottom() - 2) as f32 * TILE_F,
        );
        for _ in 0..n {
            let pos = Vec2::new(rng.range(x0, x1), rng.range(y0, y1));
            let depth = rng.range(1.25, 1.60);
            let seed = rng.u32();
            let horizontal = rng.flip();
            if blocked(pos) {
                continue;
            }
            push(
                out,
                ForeItem {
                    kind: point_kind,
                    pos,
                    len: 0.0,
                    horizontal,
                    depth,
                    seed,
                },
            );
        }
    }
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

    /// Draw one frame over a mid-grey floor and return the buffer plus the
    /// pixels painted.
    fn render(w: &mut World, layer: &mut ForeLayer, cam: Vec2, rat: Vec2, elapsed: f32) -> (PixelBuffer, u32) {
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

    /// Generation is a pure function of the level: the same `(theme, seed)`
    /// twice gives the same list, in the same order.
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
                    assert!((1.15..=1.6).contains(&item.depth), "depth {}", item.depth);
                    assert!(item.len >= 0.0, "a negative length");
                }
            }
        }
    }

    /// A different seed on the same floor is a different layer (the stream is
    /// live), and every floor actually places something.
    #[test]
    fn every_floor_places_items_and_seeds_change_them() {
        for theme in Theme::playable() {
            let (_, a) = layer_for(theme, 11);
            let (_, b) = layer_for(theme, 12);
            assert!(!a.items().is_empty(), "{theme:?} placed nothing");
            assert_ne!(a.items(), b.items(), "{theme:?}: the seed does nothing");
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
        assert_eq!(layer.items(), base.as_slice(), "a camera move changed the list");
    }

    /// Nothing sits over the start room's centre or within 5 tiles of a shop
    /// hole. (There is deliberately no arena rule: batch 12 removed the boss
    /// arenas and this module must not care either way.)
    #[test]
    fn nothing_covers_the_start_or_a_shop_hole() {
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
                }
            }
        }
    }

    /// The coverage cap holds over 20 seeds × 6 themes at several cameras: the
    /// enforced budget (pixels `blend` actually painted) never exceeds
    /// [`COVERAGE_PCT`] of the view.
    #[test]
    fn the_coverage_cap_holds_everywhere() {
        let cap = (VIEW.0 * VIEW.1) as u32 * COVERAGE_PCT / 100;
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
        assert!(painted_any, "the layer never painted at all: the cap is vacuous");
    }

    /// Parallax: an item at depth 1.5 slides 1.5 px for every camera pixel (and
    /// its shadow stays on the world).
    #[test]
    fn an_item_at_depth_one_point_five_moves_one_and_a_half_pixels_per_camera_pixel() {
        let mut w = World::new(Theme::Sewer, 5);
        let mut layer = ForeLayer::new();
        let item = ForeItem {
            kind: ForeKind::PipeRun,
            pos: Vec2::new(600.0, 400.0),
            len: 40.0,
            horizontal: true,
            depth: 1.5,
            seed: 3,
        };
        layer.force_items(vec![item]);
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
            let mut it = layer.frame().iter();
            let plan = *it.next().expect("a plan");
            plan
        };
        let a = plan_at(&mut layer, &mut w, Vec2::new(500.0, 300.0));
        let b = plan_at(&mut layer, &mut w, Vec2::new(502.0, 302.0));
        // The camera moved +2 px, so the item's screen position moved −3 px:
        // exactly `depth` (1.5) screen pixels for every camera pixel.
        assert_eq!(
            (a.screen.0 - b.screen.0, a.screen.1 - b.screen.1),
            (3, 3),
            "2 px of camera at depth 1.5 must be 3 px of item"
        );
        // The shadow is the world position: it moves exactly with the camera.
        assert_eq!(
            (a.shadow.0 - b.shadow.0, a.shadow.1 - b.shadow.1),
            (2, 2),
            "the shadow rides the world, not the parallax"
        );
    }

    /// The fade: an item over the rat is at the alpha floor, an item well away
    /// is at max, and the ramp in between is monotone.
    #[test]
    fn the_fade_near_the_rat_reaches_the_floor_and_back() {
        // Pure maths first, then through the plan.
        assert_eq!(proximity_alpha(ALPHA_MAX as f32, 0.0, FADE_RAT_PX), ALPHA_FLOOR);
        assert_eq!(
            proximity_alpha(ALPHA_MAX as f32, FADE_RAT_PX + FADE_RAMP_PX + 1.0, FADE_RAT_PX),
            ALPHA_MAX as f32
        );
        let mut last = 0.0;
        for d in 0..120 {
            let a = proximity_alpha(ALPHA_MAX as f32, d as f32, FADE_RAT_PX);
            assert!(a >= last, "the ramp must not go backwards at {d}");
            last = a;
        }

        let mut w = World::new(Theme::Sewer, 5);
        let mut layer = ForeLayer::new();
        let item = ForeItem {
            kind: ForeKind::Grate,
            pos: Vec2::new(600.0, 400.0),
            len: 0.0,
            horizontal: true,
            depth: 1.2,
            seed: 9,
        };
        layer.force_items(vec![item]);
        // The rat is placed in *screen* space: the anchor's own screen position
        // (planned with the rat far away) plus `dx`. The camera is at the
        // origin, so a screen position is also a world position here.
        let alpha_at = |layer: &mut ForeLayer, w: &mut World, dx: f32| -> u8 {
            let mut plan = |layer: &mut ForeLayer, w: &mut World, rat: Vec2| -> u8 {
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
                let mut it = layer.frame().iter();
                it.next().expect("a plan").alpha
            };
            let away = Vec2::new(-9000.0, -9000.0);
            let anchor = {
                // Learn the anchor's screen position from a frame with the rat
                // far away, so the test never has to repeat the transform.
                let mut scene = ForeScene {
                    ecs: &mut w.ecs,
                    level: &w.level,
                    cam: Vec2::ZERO,
                    size: (VIEW.0 as f32, VIEW.1 as f32),
                    rat: away,
                    elapsed: 0.0,
                };
                let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
                pb.clear((70, 66, 60));
                draw(&mut pb, layer, &mut scene);
                let mut it = layer.frame().iter();
                it.next().expect("a plan").screen
            };
            // On the anchor: the floor. Far away: full. Both through the plan.
            let _ = plan(&mut layer, &mut w, away);
            plan(
                layer,
                w,
                Vec2::new(anchor.0 as f32 + dx, anchor.1 as f32),
            )
        };
        // Right on the item (its box contains the rat): the alpha floor.
        let on_rat = alpha_at(&mut layer, &mut w, 0.0);
        assert_eq!(on_rat, ALPHA_FLOOR as u8);
        // Well away: full.
        let far = alpha_at(&mut layer, &mut w, 200.0);
        assert_eq!(far, ALPHA_MAX);
        // In the ramp: between the two. The box is 6 px wide each way, so the
        // nearest pixel is `dx - 6` away.
        let mid = alpha_at(&mut layer, &mut w, FADE_RAT_PX + FADE_RAMP_PX * 0.5 + 6.0);
        assert!(mid > on_rat && mid < far, "mid-ramp alpha {mid}");
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
        // At most ALPHA_MAX of the way to the (darkened) art colour.
        let floor = (200, 100, 50);
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

    /// A hostile pellet under an item survives the frame: the layer is drawn
    /// **before** the hostile layer, so the pellet still marks its pixel on top
    /// of the layer's paint.
    #[test]
    fn a_pellet_under_an_item_is_never_overwritten() {
        // The rat sits on the camera centre, so an item placed on the rat lands
        // on the rat's screen pixel (the parallax transform cancels there); the
        // pellet sits 6 px to its right, inside the item's box too.
        let mut w = World::new(Theme::Sewer, 7);
        w.player.pos = Vec2::new(90.0, 60.0);
        let mut layer = ForeLayer::new();
        layer.force_items(vec![ForeItem {
            kind: ForeKind::PipeRun,
            pos: w.player.pos,
            len: 60.0,
            horizontal: true,
            depth: 1.6,
            seed: 1,
        }]);
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

    fn lum(c: Rgb) -> u32 {
        u32::from(c.0) + u32::from(c.1) + u32::from(c.2)
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
        let item = ForeItem {
            kind: ForeKind::SheetOnLine,
            pos: cam + Vec2::new(VIEW.0 as f32 * 0.5, VIEW.1 as f32 * 0.5),
            len: 24.0,
            horizontal: false,
            depth: 1.3,
            seed: 5,
        };
        layer.force_items(vec![item]);

        // A time inside one sway step, and a time in a later step whose sway
        // value is genuinely different (the values are quantised, so the test
        // keeps looking rather than trusting the first step).
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
        let (c, used_c) = render(&mut w, &mut layer, cam, rat, t1);
        assert_eq!(used_c, used_a, "the sway must not change what is drawn");
        assert!(changed(&a, &c) > 0, "the sway step moved nothing");
    }

    /// Every kind is reachable and has a positive extent: the art's `extent`
    /// is what the cull and the plan rely on.
    #[test]
    fn every_kind_has_an_extent_and_is_used_by_a_floor() {
        for kind in ForeKind::all() {
            let item = ForeItem {
                kind,
                pos: Vec2::ZERO,
                len: 24.0,
                horizontal: true,
                depth: 1.3,
                seed: 0,
            };
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
    }
}
