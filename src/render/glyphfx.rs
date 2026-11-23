//! Round-glyph particles: dust motes, hit sparks and rising bubbles/steam,
//! drawn as **whole cells** over the finished playfield.
//!
//! The owner's mock-up (`.review/glyphs/`) tested what a *round* glyph does
//! inside a half-block picture and found exactly three of its uses worth
//! building (the mock's own recommendation):
//!
//! * **A — dust motes** (`·` `∙` `◦`) in the lit pools. `·` and `◦` are smaller
//!   than a single half-block pixel, so they add air without competing with the
//!   chunky sprites. Only on low-contrast, *lit* floor cells.
//! * **B — hit sparks and cooling embers** (`✦` `✧` `•`), 0.35 s around an
//!   impact, three hot steps (white → gold → ember). The 4-point star reads as
//!   a spark immediately, and its shape changes each step, which hides the
//!   18 px vertical cell step a terminal cell has.
//! * **D — bubbles, drips and steam** (`°` `◦` `○` `◌`) rising from the level's
//!   own ambient emitters. Hollow rings are shapes half blocks cannot draw.
//!
//! The mock's losing ideas are deliberately **absent**: glyph pickups (`● ◉ ◎`:
//! too quiet at 1 cell, and they break plank/floor seams) and stippled
//! light-pool edges (`· ∙ • ●` turn into horizontal scanlines at the 1:2 cell
//! aspect).
//!
//! ## Where it sits in a frame
//!
//! `App::draw_game` blits the world picture into the ratatui `Buffer` first
//! (half-block pixels, or the sub-cell frame when one was made), then this layer
//! writes cells, then the HUD, the overlays, the minimap and the cursor. So a
//! particle can never cover the HUD, a plate, a callout or the cursor, and the
//! layer is **outside `PictureKey`**: it is regenerated every rendered frame
//! from `World` state the picture cache does not hold, and it never needs the
//! world drawn again.
//!
//! ## The cell contract (the mock's own findings, made mechanical)
//!
//! A glyph cell loses the `▀` split and gets **one** background colour, so the
//! layer may only touch a cell that can afford it:
//!
//! * the cell still shows the plain half block (`▀`) — never a sub-cell
//!   quadrant/sextant/eighth, never text, never the ink pass's marks;
//! * its two pixels are within [`EDGE_GUARD`] (summed RGB) of each other — a
//!   wall border, a plank seam or a grout line collapsed into one flat 9×18
//!   rectangle is the one thing the mock showed is worse than no glyph at all;
//! * no cell within [`RAT_CLEAR`] cells of the rat, and motes only where the
//!   cell's mean luma clears [`MOTE_LUMA_MIN`] (their "lit pool" test).
//!
//! The replacement background is the **average** of the two colours the cell
//! already had, and the glyph colour is that average mixed toward the particle's
//! tint — never a flat full-saturation RGB (the mock's colour rule). A spark's
//! tint is its hot colour at 100 %, so it stays white-hot on any floor.
//!
//! Hostile marks, damage digits and loot need no special case: the digits and
//! the ink pass are stamped into the pixel buffer before the blit (so those
//! cells are high contrast and fail the guard), and hostile pellets are high
//! contrast by design. `World` has no cheap read-only accessor for their
//! positions, so the guard is the contract (see the report).
//!
//! ## Motion
//!
//! All of it is **analytic**, in held steps of [`STEP_HZ`] (8 Hz) taken from
//! `World::elapsed`, which the simulation freezes while a plate is up: a paused
//! frame is bit-identical frame to frame and repaints nothing. There is no
//! particle pool, no per-frame allocation and no rng — a particle is a pure
//! function of `(level seed, step, its own index)`, so nothing here can touch
//! the simulation, its rng, a golden or the map pin.
//!
//! Motes are anchored to **world** cells (the camera offset is subtracted, so
//! they do not swim when the rat runs) and drift one cell sideways every 1.5 s
//! and at most one cell vertically every 4 s — the mock's finding was that slow
//! vertical cell steps are the one motion that reads as "ticking".
//!
//! Render-only. The `--ai` wall and the Library never reach `draw_game`, so
//! they cannot draw it; the safe glyph set does not get it at all (see
//! [`allowed`]).

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

use crate::game::particle::damage_number::DamageNumber;
use crate::game::hazards::{SteamGrate, SteamPhase};
use crate::game::level::{AmbientEmitter, EmitterKind};
use crate::game::world::World;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};
use crate::term::{self, ColorDepth, GlyphSet};

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Held steps a second, the rate every particle moves at. Taken from
/// `World::elapsed`, which stops while a plate is up.
pub const STEP_HZ: u32 = 8;

/// The most cells this layer may write in one frame. The three category caps
/// add up to exactly this, so a busy frame can never exceed it.
pub const MAX_GLYPHS: usize = 48;

/// Dust mote cap (restrained: the mock's 32-candidate pass, cut to what a
/// 120×40 view can carry without reading as stipple).
const MOTE_CAP: usize = 20;
/// Hit spark / ember cap.
const SPARK_CAP: usize = 16;
/// Bubble / steam cap.
const BUBBLE_CAP: usize = 12;

/// Every glyph the layer may write, for tests and for the safe-set decision.
pub const GLYPHS: [char; 9] = ['·', '∙', '◦', '✦', '✧', '•', '°', '○', '◌'];

// ---------------------------------------------------------------------------
// The cell contract
// ---------------------------------------------------------------------------

/// Summed-RGB difference between a cell's two pixels at which the cell is
/// treated as an edge (a wall border, a seam, a sprite outline) and left alone.
/// The mock used 48…62 for ambient glyphs; a glyph costs 18 px of split, and
/// below 40 the two halves were already reading as one flat patch.
pub const EDGE_GUARD: u16 = 60;

/// Cells within this Chebyshev distance of the rat's own cell are never
/// written: the silhouette, its dash arc and every digit over it stay intact.
const RAT_CLEAR: i32 = 2;

/// Rows above and below a damage number's own cell that belong to its digits
/// (seven to nine screen pixels tall, plus the pop's rise).
const DIGIT_CLEAR_ROWS: i32 = 3;

/// A dust mote only sits where the cell's mean luma says "lit pool". The
/// mock's simple version of the brightness test (the median comparison it also
/// tried was not worth the extra pass).
const MOTE_LUMA_MIN: u8 = 56;

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// Warm white a mote is lifted toward off the cell average.
const MOTE_TINT: Rgb = (255, 236, 200);
/// Lift off the cell average, in percent.
const MOTE_LIFT: u8 = 62;

/// Pale teal for the damp floors' drips and fumes.
const BUBBLE_TEAL: Rgb = (156, 232, 218);
/// Steam reads white, not tinted.
const BUBBLE_WHITE: Rgb = (232, 240, 246);
/// Lift off the cell average, in percent.
const BUBBLE_LIFT: u8 = 48;

/// Spark steps, hottest first: white-hot → gold → cooling ember.
const SPARK_HOT: [Rgb; 3] = [(255, 250, 235), (255, 214, 120), (255, 140, 60)];

// ---------------------------------------------------------------------------
// Glyph sets
// ---------------------------------------------------------------------------

const MOTE_GLYPHS: [char; 3] = ['·', '∙', '◦'];
const SPARK_GLYPHS: [char; 3] = ['✦', '✧', '•'];
const BUBBLE_GLYPHS: [char; 4] = ['°', '◦', '○', '◌'];

// ---------------------------------------------------------------------------
// Mote geometry and timing (steps of 1/8 s)
// ---------------------------------------------------------------------------

/// World cells between mote slots. A slot either carries a mote for the whole
/// run or never does, chosen by the seed alone, so the picture is stable.
const MOTE_LATTICE: i32 = 2;
/// One slot in `MOTE_ONE_IN` carries a mote. At a 120×40 view that is about
/// 32 candidates, inside the mock's 20–40 band; the cap then takes 20.
const MOTE_ONE_IN: u64 = 64;
const MOTE_GATE: u64 = 3;
/// A mote steps one cell sideways every 1.5 s…
const MOTE_DRIFT_STEPS: u64 = 12;
/// …and one cell vertically every 4 s at most.
const MOTE_LIFT_STEPS: u64 = 32;
/// The glyph morphs one step of `· → ∙ → ◦` every 3 s.
const MOTE_MORPH_STEPS: u64 = 24;

// ---------------------------------------------------------------------------
// Sparks
// ---------------------------------------------------------------------------

/// How long a hit keeps throwing sparks: the mock's flash window.
const SPARK_SECS: f32 = 0.35;
/// Spark offsets are hashed inside this radius around the hit.
const SPARK_REACH: i32 = 2;

// ---------------------------------------------------------------------------
// Bubbles
// ---------------------------------------------------------------------------

/// Steps a bubble lives: 1 cell up every 2 steps, four shape stages.
const BUBBLE_STEPS: u64 = 6;

// ---------------------------------------------------------------------------
// Seeds
// ---------------------------------------------------------------------------

const SALT_MOTE: u64 = 0x6D6F_7465_5F66_7801;
const SALT_SPARK: u64 = 0x7370_6172_6B5F_7802;
const SALT_BUBBLE: u64 = 0x6275_6262_6C65_7803;

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

/// The layer's own state: just the switch. Nothing is stored per frame.
#[derive(Clone, Copy, Debug)]
pub struct GlyphFx {
    enabled: bool,
}

impl Default for GlyphFx {
    fn default() -> Self {
        GlyphFx { enabled: true }
    }
}

impl GlyphFx {
    /// On unless `RATRUN_GLYPHFX=0` pins it off for the session.
    pub fn from_env() -> Self {
        GlyphFx { enabled: enabled_from_env() }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// May the layer draw this frame?
    pub fn draws(&self) -> bool {
        allowed(self.enabled, term::glyph_set())
    }

    /// Draw this frame's particles into `buf`, over the world picture the app
    /// has just blitted into `area`. `pixels` is the viewport's own pixel grid
    /// (the same picture, two pixels a cell row) and is the exact RGB source for
    /// the guard, the average background and the tint, whatever colour depth the
    /// terminal is folded to.
    pub fn draw(&self, buf: &mut Buffer, area: Rect, pixels: &PixelBuffer, w: &World) {
        if !self.draws() || area.is_empty() {
            return;
        }
        if pixels.w != area.width as usize || pixels.h < area.height as usize * 2 {
            return;
        }
        let scene = Scene::of(area, w);
        self.draw_scene(buf, area, pixels, &scene);
    }

    /// The world-free half of [`Self::draw`]: no `World`, so the tests can drive
    /// it from a hand-built [`Scene`].
    pub fn draw_scene(
        &self,
        buf: &mut Buffer,
        area: Rect,
        pixels: &PixelBuffer,
        scene: &Scene,
    ) {
        let depth = term::color_depth();
        // The picture's own say on a cell, beyond the per-cell guard in
        // `stamp`: its mean colour must also sit within `EDGE_GUARD` of each
        // of its four neighbours'. A pellet, a loot marker, a gem or a sprite
        // edge is a small bright feature, so every cell of one (and every cell
        // beside one) differs from a neighbour and is left alone; open floor
        // under a lamp's gradient does not.
        let mean = |lx: i32, ly: i32| -> Option<Rgb> {
            if lx < 0 || ly < 0 {
                return None;
            }
            let (lx, ly) = (lx as usize, ly as usize);
            if lx >= pixels.w || ly * 2 + 1 >= pixels.h {
                return None;
            }
            Some(average(pixels.pixel_at(lx, ly * 2), pixels.pixel_at(lx, ly * 2 + 1)))
        };
        let calm = |cx: i32, cy: i32| {
            let (lx, ly) = (cx - area.x as i32, cy - area.y as i32);
            let Some(here) = mean(lx, ly) else {
                return false;
            };
            [(-1, 0), (1, 0), (0, -1), (0, 1)].into_iter().all(|(dx, dy)| {
                mean(lx + dx, ly + dy).is_none_or(|n| contrast(here, n) < EDGE_GUARD)
            })
        };
        // A mote slot only counts toward the cap where its cell is a lit, flat
        // patch: without this the first twenty slots in scan order (the top
        // rows of the view) spent the whole cap whether or not they could show.
        let lit = |cx: i32, cy: i32| {
            let (lx, ly) = (cx - area.x as i32, cy - area.y as i32);
            if lx < 0 || ly < 0 || lx as usize >= pixels.w || ly as usize * 2 + 1 >= pixels.h {
                return false;
            }
            let top = pixels.pixel_at(lx as usize, ly as usize * 2);
            let bot = pixels.pixel_at(lx as usize, ly as usize * 2 + 1);
            contrast(top, bot) < EDGE_GUARD
                && luma(average(top, bot)) >= u32::from(MOTE_LUMA_MIN)
                && calm(cx, cy)
                && !scene.near_rat(cx, cy)
                && !scene.near_digit(cx, cy)
        };
        let batch = collect_lit(scene, lit);
        for g in batch.as_slice() {
            if !scene.inside(g.cx, g.cy)
                || scene.near_rat(g.cx, g.cy)
                || scene.near_digit(g.cx, g.cy)
                || !calm(g.cx, g.cy)
            {
                continue;
            }
            let lx = (g.cx - area.x as i32) as usize;
            let ly = (g.cy - area.y as i32) as usize;
            if lx >= pixels.w || ly * 2 + 1 >= pixels.h {
                continue;
            }
            let top = pixels.pixel_at(lx, ly * 2);
            let bot = pixels.pixel_at(lx, ly * 2 + 1);
            let Some(cell) = buf.cell_mut((g.cx as u16, g.cy as u16)) else {
                continue;
            };
            stamp(cell, top, bot, g, depth);
        }
    }
}

/// `RATRUN_GLYPHFX=0|off|false|no` pins the layer off; anything else (or no
/// variable) leaves it on, like every other render-only A/B.
pub fn enabled_from_env() -> bool {
    match std::env::var("RATRUN_GLYPHFX") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => true,
    }
}

/// Whether the layer may draw at all: the switch, **and** a terminal that can
/// show the glyphs. `GlyphSet::Safe` would rewrite `·`, `•` and `°` to `.`/`o`
/// (and leave `◦`, `◌`, `∙`, `✧` alone — a mix of dots and stray shapes), so the
/// safe set does not get the layer at all.
pub fn allowed(enabled: bool, glyphs: GlyphSet) -> bool {
    enabled && glyphs == GlyphSet::Unicode
}

// ---------------------------------------------------------------------------
// A particle
// ---------------------------------------------------------------------------

/// One glyph to write. A position is a screen cell of the frame (absolute, the
/// viewport's own offset included); everything else says what to draw there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub cx: i32,
    pub cy: i32,
    pub ch: char,
    /// The colour the glyph is mixed toward, off the cell average.
    pub tint: Rgb,
    /// The mix, in percent: 100 draws `tint` itself (sparks).
    pub lift: u8,
    /// The cell's mean luma must reach this for the glyph to be drawn.
    pub min_luma: u8,
}

/// The empty slot a [`Batch`] starts full of.
const EMPTY: Glyph = Glyph { cx: 0, cy: 0, ch: ' ', tint: (0, 0, 0), lift: 0, min_luma: 0 };

/// A fixed-size frame of particles. No heap, no growth: [`MAX_GLYPHS`] is the
/// hard ceiling and the categories each have their own cap under it.
pub struct Batch {
    items: [Glyph; MAX_GLYPHS],
    len: usize,
}

impl Default for Batch {
    fn default() -> Self {
        Batch { items: [EMPTY; MAX_GLYPHS], len: 0 }
    }
}

impl Batch {
    pub fn as_slice(&self) -> &[Glyph] {
        &self.items[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Add one particle. One particle a cell: two glyphs stacked in one cell
    /// would flicker as they change shape at different rates.
    fn push(&mut self, g: Glyph) {
        if self.len >= MAX_GLYPHS {
            return;
        }
        if self.items[..self.len].iter().any(|o| o.cx == g.cx && o.cy == g.cy) {
            return;
        }
        self.items[self.len] = g;
        self.len += 1;
    }
}

// ---------------------------------------------------------------------------
// The scene
// ---------------------------------------------------------------------------

/// The screen mapping plus the per-frame inputs the layer reads. Built from
/// `World` once a frame, or by hand in a test.
pub struct Scene<'a> {
    /// The game viewport in frame cells.
    pub area: Rect,
    /// `World::cam`: the world pixel the viewport's top-left shows.
    pub cam: Vec2,
    /// `World::zoom_scale`: world pixels per screen pixel (x and y).
    pub zoom: Vec2,
    /// The held 1/8 s step, from `World::elapsed`.
    pub step: u64,
    /// `Level::seed`: the only randomness this layer has.
    pub seed: u64,
    /// The rat, in world pixels.
    pub rat: Vec2,
    /// The live damage numbers (a fresh one throws sparks).
    pub hits: &'a [DamageNumber],
    /// The level's ambient emitters (steam, drips, fumes).
    pub emitters: &'a [AmbientEmitter],
    /// The Laundry's steam grates (a plume throws steam).
    pub grates: &'a [SteamGrate],
}

impl<'a> Scene<'a> {
    pub fn of(area: Rect, w: &'a World) -> Self {
        Scene {
            area,
            cam: w.cam,
            zoom: w.zoom_scale,
            step: step_of(w.elapsed),
            seed: w.level.seed,
            rat: w.player.pos,
            hits: &w.damage_numbers.items,
            emitters: &w.level.objects.emitters,
            grates: &w.level.objects.steam_grates,
        }
    }

    /// A world position's frame cell: one cell column is `zoom.x` world pixels,
    /// one cell row `2 · zoom.y` — the same transform `World::draw` and the
    /// mouse aim use, so the layer lines up at every zoom rung.
    pub fn cell(&self, p: Vec2) -> (i32, i32) {
        let zx = self.zoom.x.max(0.01);
        let zy = (self.zoom.y * 2.0).max(0.01);
        (
            self.area.x as i32 + ((p.x - self.cam.x) / zx).floor() as i32,
            self.area.y as i32 + ((p.y - self.cam.y) / zy).floor() as i32,
        )
    }

    pub fn inside(&self, cx: i32, cy: i32) -> bool {
        cx >= self.area.x as i32
            && cy >= self.area.y as i32
            && cx < self.area.x as i32 + self.area.width as i32
            && cy < self.area.y as i32 + self.area.height as i32
    }

    /// Within [`RAT_CLEAR`] cells of the rat's cell: the silhouette and every
    /// digit over it are left alone.
    pub fn near_rat(&self, cx: i32, cy: i32) -> bool {
        let (rx, ry) = self.cell(self.rat);
        (cx - rx).abs() <= RAT_CLEAR && (cy - ry).abs() <= RAT_CLEAR
    }

    /// The half-width, in cells, of the box a damage number's digits are
    /// printed in (a digit is four screen pixels wide with its gap, bold ones
    /// five, plus the dark plate's margin).
    fn digit_half_width(dn: &DamageNumber) -> i32 {
        let digits = dn.display_value().max(1).ilog10() as i32 + 1;
        let per = if dn.is_big() { 5 } else { 4 };
        (digits * per) / 2 + 3
    }

    /// Inside the box of a live damage number: the digits are text, and a
    /// particle on one makes it a different number.
    pub fn near_digit(&self, cx: i32, cy: i32) -> bool {
        self.hits.iter().any(|dn| {
            if !dn.visible() {
                return false;
            }
            let (ax, ay) = self.cell(dn.pos);
            (cx - ax).abs() <= Self::digit_half_width(dn) && (cy - ay).abs() <= DIGIT_CLEAR_ROWS
        })
    }
}

/// The held step [`STEP_HZ`] a given elapsed time is on. Saturating, so a
/// pathological clock cannot wrap the animation.
pub fn step_of(elapsed: f32) -> u64 {
    if !elapsed.is_finite() {
        return 0;
    }
    (elapsed.max(0.0) * STEP_HZ as f32) as u64
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// The three category caps, halved when the view is pulled back past 1.5×: at
/// that size a cell is several world pixels and particles crowd the picture
/// rather than adding air to it.
#[derive(Clone, Copy)]
struct Caps {
    motes: usize,
    sparks: usize,
    bubbles: usize,
}

const CAPS: Caps = Caps { motes: MOTE_CAP, sparks: SPARK_CAP, bubbles: BUBBLE_CAP };
const CAPS_ZOOMED_OUT: Caps = Caps {
    motes: MOTE_CAP / 2,
    sparks: SPARK_CAP / 2,
    bubbles: BUBBLE_CAP / 2,
};

/// Every particle for this frame, in draw order (motes, bubbles, sparks), each
/// category capped and no two on one cell.
pub fn collect(scene: &Scene) -> Batch {
    collect_lit(scene, |_, _| true)
}

/// [`collect`] with the picture's say on the motes: `lit(cx, cy)` is whether a
/// frame cell is a lit pool a mote may sit in, asked **before** the slot is
/// counted against the mote cap.
pub fn collect_lit(scene: &Scene, lit: impl Fn(i32, i32) -> bool) -> Batch {
    let caps = if scene.zoom.x > 1.5 { CAPS_ZOOMED_OUT } else { CAPS };
    let mut out = Batch::default();
    motes(scene, caps.motes, &lit, &mut out);
    bubbles(scene, caps.bubbles, &mut out);
    sparks(scene, caps.sparks, &mut out);
    out
}

/// A — dust motes. Candidates come from a **world-anchored** lattice: a slot
/// either is a mote (forever, for this seed) or is not, and the camera offset is
/// subtracted at draw time, so the field of motes holds still while the rat
/// runs and the motes do not swim.
fn motes(scene: &Scene, cap: usize, lit: &impl Fn(i32, i32) -> bool, out: &mut Batch) {
    if cap == 0 || scene.area.is_empty() {
        return;
    }
    // The viewport in world cells (one cell is zoom.x by 2·zoom.y world pixels).
    let zx = scene.zoom.x.max(0.01);
    let zy = (scene.zoom.y * 2.0).max(0.01);
    let c0x = (scene.cam.x / zx).floor() as i32;
    let c0y = (scene.cam.y / zy).floor() as i32;
    let cols = scene.area.width as i32;
    let rows = scene.area.height as i32;
    let (l0x, l0y) = (c0x.div_euclid(MOTE_LATTICE), c0y.div_euclid(MOTE_LATTICE));
    let (l1x, l1y) = (
        (c0x + cols).div_euclid(MOTE_LATTICE),
        (c0y + rows).div_euclid(MOTE_LATTICE),
    );
    let mut placed = 0;
    for sy in l0y..=l1y {
        for sx in l0x..=l1x {
            if placed >= cap {
                return;
            }
            // The slot's whole life comes from the seed and the slot, never from
            // the step: that is what keeps a mote in its own world cell.
            let slot = hash(scene.seed ^ SALT_MOTE, sx as i64 as u64, sy as i64 as u64);
            if (slot >> 40) % MOTE_ONE_IN >= MOTE_GATE {
                continue;
            }
            let jx = ((slot >> 8) % MOTE_LATTICE as u64) as i32;
            let jy = ((slot >> 13) % MOTE_LATTICE as u64) as i32;
            // Sideways: one cell every 1.5 s, and back.
            let drift_phase = (slot >> 17) % MOTE_DRIFT_STEPS;
            let drift_dir = if (slot >> 24) & 1 == 0 { 1 } else { -1 };
            let dx = ((((scene.step + drift_phase) / MOTE_DRIFT_STEPS) & 1) as i32) * drift_dir;
            // Vertically: at most one cell per 4 s (the mock's jumpy-mote rule).
            let lift_phase = (slot >> 28) % MOTE_LIFT_STEPS;
            let dy = -((((scene.step + lift_phase) / MOTE_LIFT_STEPS) & 1) as i32);
            // Each mote is at its own point of the `· → ∙ → ◦` cycle, so the
            // field shimmers instead of all morphing on the same step.
            let morph = (slot >> 36) % 3;
            let stage = (((scene.step / MOTE_MORPH_STEPS) + morph) % 3) as usize;

            let wcx = sx * MOTE_LATTICE + jx;
            let wcy = sy * MOTE_LATTICE + jy;
            let cx = scene.area.x as i32 + (wcx - c0x) + dx;
            let cy = scene.area.y as i32 + (wcy - c0y) + dy;
            if !scene.inside(cx, cy) || !lit(cx, cy) {
                continue;
            }
            out.push(Glyph {
                cx,
                cy,
                ch: MOTE_GLYPHS[stage],
                tint: MOTE_TINT,
                lift: MOTE_LIFT,
                min_luma: MOTE_LUMA_MIN,
            });
            placed += 1;
        }
    }
}

/// B — hit sparks and cooling embers. The renderer already has
/// `World::damage_numbers` at draw time; a number younger than [`SPARK_SECS`]
/// throws 2–3 sparks that move outward a cell a step and cool white → gold →
/// ember.
fn sparks(scene: &Scene, cap: usize, out: &mut Batch) {
    if cap == 0 {
        return;
    }
    let mut placed = 0;
    for (i, dn) in scene.hits.iter().enumerate() {
        if placed >= cap {
            return;
        }
        if !dn.visible() {
            continue;
        }
        let age = (dn.max_life - dn.life).max(0.0);
        if age > SPARK_SECS {
            continue;
        }
        let (ax, ay) = scene.cell(dn.pos);
        if !scene.inside(ax, ay) {
            continue;
        }
        // Whole steps since the hit, 0…2: the shape and the colour step with it.
        let t = ((age * STEP_HZ as f32) as i32).min(2);
        let h = hash(
            scene.seed ^ SALT_SPARK,
            i as u64,
            dn.total_dmg.to_bits() as u64,
        );
        let n = 2 + (h & 1) as usize;
        for k in 0..n {
            if placed >= cap {
                return;
            }
            let hk = hash(h, k as u64, 0x5350_4152_4B00);
            // Sparks fly from the *edge* of the number's own box, never from
            // inside it: mostly out of the sides (where the struck body is),
            // one in three from above.
            let half = Scene::digit_half_width(dn);
            let side = if hk & 1 == 0 { 1 } else { -1 };
            let (cx, cy) = if (hk >> 1).is_multiple_of(3) {
                let ox = ((hk >> 8) % (2 * SPARK_REACH + 1) as u64) as i32 - SPARK_REACH;
                (ax + ox + ox.signum() * t, ay - DIGIT_CLEAR_ROWS - 1 - t)
            } else {
                let oy = ((hk >> 8) % (2 * SPARK_REACH + 1) as u64) as i32 - SPARK_REACH;
                (ax + side * (half + 1 + t), ay + oy)
            };
            if !scene.inside(cx, cy) {
                continue;
            }
            let stage = t as usize;
            out.push(Glyph {
                cx,
                cy,
                ch: SPARK_GLYPHS[stage],
                tint: SPARK_HOT[stage],
                lift: 100,
                min_luma: 0,
            });
            placed += 1;
        }
    }
}

/// D — bubbles, drips and steam, from the level's own ambient emitters (and the
/// Laundry's grates mid-plume). A bubble rises a cell every 2 steps over
/// [`BUBBLE_STEPS`] and grows then dissolves: `° → ◦ → ○ → ◌`.
fn bubbles(scene: &Scene, cap: usize, out: &mut Batch) {
    if cap == 0 {
        return;
    }
    let mut placed = 0;
    for (i, e) in scene.emitters.iter().enumerate() {
        if placed >= cap {
            return;
        }
        let tint = match e.kind {
            EmitterKind::Steam => BUBBLE_WHITE,
            EmitterKind::Drip | EmitterKind::Fumes => BUBBLE_TEAL,
            _ => continue,
        };
        let (bx, by) = scene.cell(e.pos);
        if !scene.inside(bx, by) {
            continue;
        }
        let h = hash(scene.seed ^ SALT_BUBBLE, i as u64, u64::from(e.seed));
        let n = 1 + (h % 3) as usize;
        for k in 0..n {
            if placed >= cap {
                return;
            }
            let off = (h >> (8 + k * 8)) % BUBBLE_STEPS;
            let age = (scene.step + off) % BUBBLE_STEPS;
            let rise = (age / 2) as i32;
            let stage = ((age * 4) / BUBBLE_STEPS).min(3) as usize;
            let dx = ((h >> (20 + k * 6)) % 3) as i32 - 1;
            let cx = bx + dx;
            let cy = by - 1 - rise;
            if !scene.inside(cx, cy) {
                continue;
            }
            out.push(Glyph {
                cx,
                cy,
                ch: BUBBLE_GLYPHS[stage],
                tint,
                lift: BUBBLE_LIFT,
                min_luma: 0,
            });
            placed += 1;
        }
    }
    // The Laundry's grates: a plume is steam at full pressure.
    for (i, g) in scene.grates.iter().enumerate() {
        if placed >= cap {
            return;
        }
        if g.phase != SteamPhase::Plume {
            continue;
        }
        let (bx, by) = scene.cell(g.pos);
        if !scene.inside(bx, by) {
            continue;
        }
        let h = hash(scene.seed ^ SALT_BUBBLE, i as u64 ^ 0x4752_4154_4500, 3);
        let n = 1 + (h % 3) as usize;
        for k in 0..n {
            if placed >= cap {
                return;
            }
            let off = (h >> (8 + k * 8)) % BUBBLE_STEPS;
            let age = (scene.step + off) % BUBBLE_STEPS;
            let rise = (age / 2) as i32;
            let stage = ((age * 4) / BUBBLE_STEPS).min(3) as usize;
            let dx = ((h >> (20 + k * 6)) % 3) as i32 - 1;
            let cx = bx + dx;
            let cy = by - 1 - rise;
            if !scene.inside(cx, cy) {
                continue;
            }
            out.push(Glyph {
                cx,
                cy,
                ch: BUBBLE_GLYPHS[stage],
                tint: BUBBLE_WHITE,
                lift: BUBBLE_LIFT,
                min_luma: 0,
            });
            placed += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// Write one glyph over one cell, if the cell contract allows it. `top` and
/// `bot` are the cell's two pixel colours (the `▀` foreground and background).
///
/// Returns whether it wrote: a cell that is not a plain half block, or that is
/// an edge, or that is too dark for this particle, is left exactly as it was.
pub fn stamp(cell: &mut Cell, top: Rgb, bot: Rgb, g: &Glyph, depth: ColorDepth) -> bool {
    if cell.symbol() != "▀" {
        return false;
    }
    if contrast(top, bot) >= EDGE_GUARD {
        return false;
    }
    let bg = average(top, bot);
    if luma(bg) < u32::from(g.min_luma) {
        return false;
    }
    cell.set_char(g.ch);
    cell.fg = palette::to_term(palette::mix(bg, g.tint, f32::from(g.lift) / 100.0), depth);
    cell.bg = palette::to_term(bg, depth);
    true
}

/// Summed RGB distance.
fn contrast(a: Rgb, b: Rgb) -> u16 {
    u16::from(a.0.abs_diff(b.0)) + u16::from(a.1.abs_diff(b.1)) + u16::from(a.2.abs_diff(b.2))
}

/// The single background a glyph cell gets: the mean of the two colours it
/// already had, so a mote cannot punch a dark box into a lit pool.
fn average(a: Rgb, b: Rgb) -> Rgb {
    (
        ((u16::from(a.0) + u16::from(b.0)) / 2) as u8,
        ((u16::from(a.1) + u16::from(b.1)) / 2) as u8,
        ((u16::from(a.2) + u16::from(b.2)) / 2) as u8,
    )
}

/// Rec. 601 luma, 0–255.
fn luma(c: Rgb) -> u32 {
    (299 * u32::from(c.0) + 587 * u32::from(c.1) + 114 * u32::from(c.2)) / 1000
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// SplitMix64's finaliser over three mixed inputs: a particle's whole life is
/// this function of (seed, index, what), so there is no rng and no state.
#[inline]
pub fn hash(a: u64, b: u64, c: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ c.wrapping_mul(0x1656_67B1_9E37_79F9);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::particle::damage_number::DamageSource;
    use ratatui::style::Color;

    const W: u16 = 80;
    const H: u16 = 24;

    fn field(c: Rgb) -> (Buffer, PixelBuffer) {
        let mut pixels = PixelBuffer::new(W as usize, H as usize * 2);
        for p in pixels.pixels_mut() {
            *p = c;
        }
        let mut buf = Buffer::empty(Rect::new(0, 0, W, H));
        for y in 0..H {
            for x in 0..W {
                let cell = &mut buf[(x, y)];
                cell.set_symbol("▀");
                cell.fg = palette::to_term(c, ColorDepth::Truecolor);
                cell.bg = palette::to_term(c, ColorDepth::Truecolor);
            }
        }
        (buf, pixels)
    }

    fn scene(step: u64) -> Scene<'static> {
        Scene {
            area: Rect::new(0, 0, W, H),
            cam: Vec2::ZERO,
            zoom: Vec2::new(1.0, 1.0),
            step,
            seed: 7,
            rat: Vec2::new(-500.0, -500.0),
            hits: &[],
            emitters: &[],
            grates: &[],
        }
    }

    fn hit(pos: Vec2) -> DamageNumber {
        DamageNumber {
            enemy_id: 1,
            pos,
            vel: Vec2::ZERO,
            total_dmg: 12.0,
            life: 1.0,
            max_life: 1.0,
            total_age: 0.0,
            since_hit: 0.0,
            pop: 1.0,
            is_kill: false,
            source: DamageSource::Normal,
        }
    }

    #[test]
    fn the_writer_respects_the_edge_guard_and_the_average_bg() {
        let g = Glyph {
            cx: 0,
            cy: 0,
            ch: '·',
            tint: MOTE_TINT,
            lift: MOTE_LIFT,
            min_luma: MOTE_LUMA_MIN,
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        for x in 0..4 {
            buf[(x, 0)].set_symbol("▀");
        }

        // A flat, lit cell takes the glyph, and the background is the mean of
        // its two pixels — never one of them, never black.
        let (top, bot) = ((100u8, 90u8, 70u8), (110u8, 100u8, 80u8));
        assert!(stamp(&mut buf[(0, 0)], top, bot, &g, ColorDepth::Truecolor));
        assert_eq!(buf[(0, 0)].symbol(), "·");
        let bg = average(top, bot);
        assert_eq!(bg, (105, 95, 75));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(bg.0, bg.1, bg.2));
        let want = palette::mix(bg, MOTE_TINT, f32::from(MOTE_LIFT) / 100.0);
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(want.0, want.1, want.2));
        assert!(want.1 > bg.1, "the mote is lifted toward warm white");

        // An edge (a wall border, a plank seam) is never overwritten.
        assert!(!stamp(
            &mut buf[(1, 0)],
            (0, 0, 0),
            (20, 20, 20),
            &g,
            ColorDepth::Truecolor
        ));
        assert_eq!(buf[(1, 0)].symbol(), "▀");

        // Nor is anything that is not the plain half block.
        buf[(2, 0)].set_symbol("█");
        assert!(!stamp(
            &mut buf[(2, 0)],
            (100, 90, 70),
            (110, 100, 80),
            &g,
            ColorDepth::Truecolor
        ));
        assert_eq!(buf[(2, 0)].symbol(), "█");

        // A flat but dark cell fails a mote's lit-pool test.
        assert!(!stamp(
            &mut buf[(3, 0)],
            (20, 20, 20),
            (24, 22, 22),
            &g,
            ColorDepth::Truecolor
        ));
        assert_eq!(buf[(3, 0)].symbol(), "▀");
    }

    #[test]
    fn particles_are_a_pure_function_of_the_seed_and_the_step() {
        let a = collect(&scene(41));
        let b = collect(&scene(41));
        assert_eq!(a.as_slice(), b.as_slice(), "same seed and step, same particles");
        assert!(!a.is_empty(), "a 80×24 view gets some motes");
        assert!(
            (0..64).any(|step| collect(&scene(step)).as_slice() != a.as_slice()),
            "the layer animates"
        );
    }

    #[test]
    fn a_fixed_frame_is_drawn_identically_twice() {
        // Sparks need a fresh hit, so this also covers the damage-number path.
        let hits = [hit(Vec2::new(40.0, 20.0))];
        let mut s = scene(21);
        s.hits = &hits;
        let area = s.area;
        let (mut one, pixels) = field((60, 56, 48));
        let (mut two, _) = field((60, 56, 48));
        let fx = GlyphFx::default();
        fx.draw_scene(&mut one, area, &pixels, &s);
        fx.draw_scene(&mut two, area, &pixels, &s);
        assert_eq!(one, two, "one frozen frame, one picture");
        assert_ne!(one, field((60, 56, 48)).0, "the layer drew something");
    }

    #[test]
    fn the_caps_hold() {
        let hits: Vec<DamageNumber> = (0..40)
            .map(|i| hit(Vec2::new(4.0 + i as f32 * 1.7, 24.0)))
            .collect();
        let emitters: Vec<AmbientEmitter> = (0..40)
            .map(|i| AmbientEmitter {
                kind: EmitterKind::Steam,
                pos: Vec2::new(4.0 + i as f32 * 1.9, 40.0),
                rate: 1.0,
                intensity: 1.0,
                seed: i as u32,
            })
            .collect();
        let mut s = scene(3);
        s.hits = &hits;
        s.emitters = &emitters;

        let batch = collect(&s);
        assert!(batch.len() <= MAX_GLYPHS);
        let sparks = batch
            .as_slice()
            .iter()
            .filter(|g| SPARK_GLYPHS.contains(&g.ch))
            .count();
        assert!(sparks <= SPARK_CAP, "{sparks} sparks");
        let bubbles = batch
            .as_slice()
            .iter()
            .filter(|g| BUBBLE_GLYPHS.contains(&g.ch) && g.ch != '◦')
            .count();
        assert!(bubbles <= BUBBLE_CAP, "{bubbles} bubbles");

        // Pulled back past 1.5× the caps halve.
        s.zoom = Vec2::new(2.0, 2.0);
        assert!(collect(&s).len() <= MAX_GLYPHS / 2);
    }

    #[test]
    fn safe_glyph_mode_disables_the_layer() {
        assert!(allowed(true, GlyphSet::Unicode), "the shipped picture gets it");
        assert!(
            !allowed(true, GlyphSet::Safe),
            "the safe set would rewrite half of these glyphs to ASCII"
        );
        assert!(!allowed(false, GlyphSet::Unicode), "the switch off is off");
        assert!(!allowed(false, GlyphSet::Safe));
    }

    #[test]
    fn a_bubble_rises_above_its_emitter() {
        // A drip at world (40, 30) is the cell (40, 15) at 1×.
        let emitters = [AmbientEmitter {
            kind: EmitterKind::Drip,
            pos: Vec2::new(40.0, 30.0),
            rate: 1.7,
            intensity: 1.0,
            seed: 3,
        }];
        // `◦` is also a mote glyph, so look for the three only bubbles use.
        let only_bubbles = ['°', '○', '◌'];
        let mut seen = 0;
        for step in 0..BUBBLE_STEPS {
            let mut s = scene(step);
            s.emitters = &emitters;
            let batch = collect(&s);
            for g in batch.as_slice() {
                if !only_bubbles.contains(&g.ch) {
                    continue;
                }
                seen += 1;
                assert!(g.cy < 15, "a bubble rises above its emitter: {g:?}");
                assert!((g.cx - 40).abs() <= 1, "and stays in its column: {g:?}");
            }
        }
        assert!(seen > 0, "a drip throws bubbles over its cycle");
    }
}
