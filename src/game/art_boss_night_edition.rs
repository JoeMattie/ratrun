//! Art and gameplay specification for the Printworks boss ("THE NIGHT EDITION").
//!
//! A walking offset press: a square cast-iron platen carried on two chunky
//! typewriter keys, a recessed type bed full of reversed letters, a clamped
//! newsprint sheet, a steel carriage rail with its clamp and two ink-ribbon
//! spools. Seen from above it reads as a slab of iron that prints where it
//! stands — charcoal frame, newsprint grey, dried-ink purple and registration
//! cyan, the palette of the Printworks floor (`WAVE3_DESIGN.md` §11/§12).
//!
//! # Layering contract for implementers
//! Call the draw functions in this order inside `World::draw`:
//! 1. **Ground layer** (after `art_floor::draw_floor`, stains and props; before
//!    gems, bullets, enemies and the player): [`draw_ground`] — the page
//!    outline and its blank margin, the misprint pair, the dotted carriage
//!    rule and its moving smear, the ribbon-window marker, the pose-change
//!    ring, the wreck's ink pool, and the drop shadow.
//! 2. **Entity layer** (alongside `art_enemy::draw_enemy` and `art_rat::draw_rat`):
//!    [`draw`] — the platen, its type bed (the exposed weak point), the keys,
//!    the carriage rail, the ribbon, the phase damage, the death sheets and the
//!    radial HP arc.
//!
//! No projectiles: every Night Edition attack is a floor footprint, so the
//! framework's [`crate::game::ecs::systems::boss::BossHazard`] carries it.
//!
//! # The fight the design asks for (`WAVE3_DESIGN.md` §12)
//!
//! | phase | tell → attack | safe response and payoff |
//! |-------|---------------|--------------------------|
//! | above 65 % | [`NightEditionState::PageTell`] outlines a 36 × 30 px page at the rat for `1.1 s`, leaving a `12 px` blank margin on one side, then [`NightEditionState::PageStamp`] prints the inked part for `18` | stand in the blank margin (or off the sheet); the press then rests on its lowered type bed ([`NightEditionState::BedLowered`], ×1.5 damage for `1.4 s`) |
//! | 65–30 % | alternate the page stamp with [`NightEditionState::CarriageTell`] (bell + dotted 54 × 8 px rule for `1 s`) → [`NightEditionState::CarriageSweep`] (`20`) | cross the 8 px rule early; follow the carriage to its stopped end for the exposed ribbon ([`NightEditionState::Ribbon`], ×1.5 for `1.6 s`) |
//! | below 30 % | [`NightEditionState::MisprintTell`]: two pages offset `12 px` for `1.2 s`, stamped `0.55 s` apart with a **shared 12 px clear margin**, then the solo carriage, then `2 s` exposed | stand on the unprinted stripe for both beats |
//!
//! Body contact is `8` outside recovery ([`CONTACT_DAMAGE`]); every recovery
//! state (`BedLowered`, `Ribbon`, `PhaseTurn`, `Wreck`) is harmless to touch.
//! A phase change spends [`PHASE_TURN`] (`1.2 s`) on a harmless pose change.
//! Every hazard belongs to the boss and dies with it (the framework owns that).
//!
//! # Zero allocation
//! Every draw function uses stack storage only and culls to the
//! [`PixelBuffer`] bounds first.

#![allow(dead_code)]

use std::f32::consts::TAU;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

// ============================================================================
// Geometry and gameplay constants (the art ↔ hitbox contract)
// ============================================================================

/// Primary collision radius of the platen slab.
pub const BODY_RADIUS: f32 = 13.5;

/// Half-length of the platen slab along `facing` (`+u` = the carriage side).
pub const BODY_HALF_LEN: f32 = 14.0;

/// Half-width of the platen slab across `facing`.
pub const BODY_HALF_WIDTH: f32 = 13.0;

/// Half-extents of the recessed type bed — the exposed weak point.
pub const BED_HALF: Vec2 = Vec2 { x: 9.0, y: 8.0 };

/// Where the exposed ink ribbon sits, along `facing` (a punish target).
pub const RIBBON_OFFSET: f32 = 9.5;

/// Radius of the ribbon-window target point (auto-aim only).
pub const RIBBON_WINDOW_RADIUS: f32 = 6.0;

/// Culling margin for the art's largest ground footprint (page 36 px wide, the
/// carriage rule 54 px long).
pub const CULL_MARGIN: f32 = 64.0;

/// The page's half-extents: a 36 × 30 px sheet.
pub const PAGE_HALF: Vec2 = Vec2 { x: 18.0, y: 15.0 };

/// The blank margin left on one side of the page (12 px of 36 px).
pub const PAGE_MARGIN: f32 = 12.0;

/// Page tell (`1.1 s`): the sheet is outlined at the rat.
pub const STAMP_TELL: f32 = 1.1;

/// Length of the inked part coming down.
pub const STAMP_ACTIVE: f32 = 0.30;

/// Settle after the impression before the bed drops to its recovery pose.
pub const STAMP_SETTLE: f32 = 0.24;

/// The stamp's damage.
pub const STAMP_DAMAGE: f32 = 18.0;

/// The lowered type bed: `1.4 s` of ×1.5 recovery.
pub const BED_LOWERED: f32 = 1.4;

/// Incoming-damage multiplier while the bed or the ribbon is exposed.
pub const WEAK_MULT: f32 = 1.5;

/// The carriage rule's length (54 px).
pub const RULE_LEN: f32 = 54.0;

/// The carriage rule's half-width (an 8 px band).
pub const RULE_HALF_WIDTH: f32 = 4.0;

/// Bell + rule tell (`1.0 s`).
pub const CARRIAGE_TELL: f32 = 1.0;

/// The carriage's sweep speed along the rule.
pub const SWEEP_SPEED: f32 = 108.0;

/// The carriage sweep's damage.
pub const SWEEP_DAMAGE: f32 = 20.0;

/// The exposed ribbon window after a carriage return (`1.6 s`).
pub const RIBBON: f32 = 1.6;

/// The exposed ribbon window after a misprint's carriage (`2.0 s`).
pub const RIBBON_LONG: f32 = 2.0;

/// The misprint's tell (`1.2 s`): two sheets, offset by [`MISPRINT_OFFSET`].
pub const MISPRINT_TELL: f32 = 1.2;

/// Offset between the misprint's two sheets (and the shared clear margin).
pub const MISPRINT_OFFSET: f32 = 12.0;

/// The two misprint impressions are this far apart (`0.55 s`).
pub const MISPRINT_GAP: f32 = 0.55;

/// Each misprint impression's active window.
pub const MISPRINT_ACTIVE: f32 = 0.26;

/// The harmless pose change at a phase threshold (`1.2 s`).
pub const PHASE_TURN: f32 = 1.2;

/// Body contact outside recovery.
pub const CONTACT_DAMAGE: f32 = 8.0;

/// The wreck's death beat.
pub const DEATH_DURATION: f32 = 2.8;

/// The art's recommended base pool. `WAVE3_DESIGN.md` §12 fixes the final
/// encounter pool at 3,000 HP *including stage scaling*, so this is the art
/// number the framework scales exactly like the other three bosses.
pub const BOSS_BASE_HP: f32 = 3000.0;

/// Walk speed per phase (`26 / 30 / 34 px/s`).
pub const WALK_SPEED: [f32; 3] = [26.0, 30.0, 34.0];

/// The platen creeps forward while a page is being outlined (`9 px/s`).
pub const TELL_CREEP: f32 = 9.0;

/// The bed-lowering flinch: the exposed bed's glow radius on the ground.
pub const BED_GLOW_RADIUS: f32 = 13.0;

/// Half-extents of the arena's press bed — the boss's own arena dressing,
/// laid on the encounter floor for the whole fight.
pub const PRESS_BED_HALF: Vec2 = Vec2 { x: 30.0, y: 23.0 };

// ---- palette ---------------------------------------------------------------

const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// Cast iron frame, carriage steel and the type bed's shadow.
const IRON_SHADOW: Rgb = (20, 20, 26);
const IRON_MID: Rgb = (58, 58, 70);
const IRON_HI: Rgb = (118, 118, 132);
const IRON_SPEC: Rgb = (188, 188, 200);

// Newsprint stock.
const PAPER_SHADOW: Rgb = (146, 144, 132);
const PAPER_MID: Rgb = (188, 186, 172);
const PAPER_HI: Rgb = (224, 222, 210);
const PAPER_SPEC: Rgb = (250, 248, 238);

// Dried ink, the bed's recess and fresh impressions.
const INK_SHADOW: Rgb = (26, 18, 32);
const INK_MID: Rgb = (68, 50, 82);
const INK_HI: Rgb = (104, 78, 124);
const INK_FRESH: Rgb = (128, 78, 126);

// Registration cyan (the press's own colour), bell brass, ivory keys.
const CYAN_DARK: Rgb = (34, 78, 82);
const CYAN_MID: Rgb = (78, 148, 152);
const CYAN_HI: Rgb = (150, 226, 226);
const BRASS_MID: Rgb = (186, 142, 56);
const BRASS_HI: Rgb = (255, 226, 142);
const IVORY_SHADOW: Rgb = (186, 176, 142);
const IVORY_MID: Rgb = (238, 230, 202);
const IVORY_HI: Rgb = (255, 251, 232);

// The ink ribbon.
const RIBBON_SHADOW: Rgb = (84, 22, 40);
const RIBBON_MID: Rgb = (168, 42, 70);
const RIBBON_HI: Rgb = (242, 112, 132);

// Tells.
const TELL_AMBER: Rgb = (255, 196, 84);
const DANGER_RED: Rgb = (255, 74, 58);
const SAFE_CYAN: Rgb = (128, 236, 226);

// ============================================================================
// State
// ============================================================================

/// The Night Edition's move set.
///
/// Every variant carries its art timing, its telegraph, its floor footprint and
/// its damage; the controller in
/// [`crate::game::ecs::systems::boss::night_edition`] only drives transitions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NightEditionState {
    /// **Walking platen (neutral).**
    ///
    /// - **Behaviour**: strides on its two ivory keys at [`WALK_SPEED`]
    ///   (`26 / 30 / 34 px/s` by phase), holding mid range (`32..56 px`) and
    ///   circling one way per beat.
    /// - **Duration**: `1.8 s` / `1.5 s` / `1.2 s` (feedback: `ROLL_DURATION`).
    /// - **Warning**: none (the keys clack).
    /// - **Dangerous area**: the body slab ([`BODY_HALF_LEN`] ×
    ///   [`BODY_HALF_WIDTH`]).
    /// - **Damage**: [`CONTACT_DAMAGE`] (`8`) body contact.
    /// - **Counter**: chip it while it walks; it always telegraphs next.
    Roll,

    /// **Page tell: the sheet is outlined where the rat stands.**
    ///
    /// - **Behaviour**: creeps forward at [`TELL_CREEP`] (`9 px/s`) and casts a
    ///   36 × 30 px page centred on the rat's position when the tell began,
    ///   leaving a [`PAGE_MARGIN`] (`12 px`) blank margin on one side (the side
    ///   with walkable floor).
    /// - **Duration**: [`STAMP_TELL`] (`1.1 s`).
    /// - **Warning**: the whole tell — newsprint outline, crop marks, the blank
    ///   margin picked out in registration cyan, the inked area filling with
    ///   ink as the press comes down.
    /// - **Dangerous area**: the **inked** part only (24 × 30 px).
    /// - **Damage**: `0` during the tell.
    /// - **Counter**: step into the blank margin or off the sheet entirely;
    ///   then punish the lowered bed.
    PageTell,

    /// **The impression: the inked part comes down.**
    ///
    /// - **Behaviour**: the platen drops onto the marked ink for
    ///   [`STAMP_ACTIVE`] (`0.30 s`) and settles.
    /// - **Duration**: [`STAMP_ACTIVE`] + [`STAMP_SETTLE`] (`0.54 s`).
    /// - **Warning**: the tell's own; the impression flashes ink and throws grit.
    /// - **Dangerous area**: the inked 24 × 30 px rectangle.
    /// - **Damage**: [`STAMP_DAMAGE`] (`18`).
    PageStamp,

    /// **The lowered type bed (recovery / weak point).**
    ///
    /// - **Behaviour**: the form drops away from the platen; the bed sits open
    ///   and the whole press stands still.
    /// - **Duration**: [`BED_LOWERED`] (`1.4 s`).
    /// - **Warning**: the bed glows hot ink and the frame is clearly open.
    /// - **Dangerous area**: none — the press is harmless to touch.
    /// - **Damage**: `0` (it *takes* ×1.5 — [`WEAK_MULT`] — for the whole beat).
    /// - **Counter**: this is the opening. Everything the rat has, now.
    BedLowered,

    /// **Misprint tell (phase 3): two sheets, one clear stripe.**
    ///
    /// - **Behaviour**: shows two sheets offset by [`MISPRINT_OFFSET`]
    ///   (`12 px`), both leaving the **same** 12 px stripe blank. The two
    ///   impressions land [`MISPRINT_GAP`] (`0.55 s`) apart, each
    ///   [`MISPRINT_ACTIVE`] long.
    /// - **Duration**: [`MISPRINT_TELL`] (`1.2 s`), then `MisprintStamp`.
    /// - **Warning**: both outlines, both inked areas, the shared stripe in cyan.
    /// - **Dangerous area**: each page's inked part (24 × 30 px), 12 px apart.
    /// - **Damage**: `0` during the tell.
    /// - **Counter**: stand on the shared stripe for both beats, then leave
    ///   before the carriage.
    MisprintTell,

    /// **The double impression.**
    ///
    /// - **Duration**: [`MISPRINT_GAP`] + [`MISPRINT_ACTIVE`] + settle
    ///   (`1.1 s`).
    /// - **Damage**: [`STAMP_DAMAGE`] on each impression (one hit each).
    MisprintStamp,

    /// **Carriage return tell: the bell and the dotted rule.**
    ///
    /// - **Behaviour**: the carriage clamps onto its rail and the press rings
    ///   its bell; a dotted 54 × 8 px rule is ruled through the rat's position.
    /// - **Duration**: [`CARRIAGE_TELL`] (`1.0 s`).
    /// - **Warning**: the bell, the dotted rule, the brass stop bracket at the
    ///   far end and a marching carriage glyph at the near end.
    /// - **Dangerous area**: the rule's band ([`RULE_HALF_WIDTH`] `4 px` each
    ///   side of the line), which is where the carriage will run.
    /// - **Damage**: `0` during the tell.
    /// - **Counter**: cross the 8 px band early — being off the rule at all is
    ///   the whole dodge — then follow the carriage to its stop.
    CarriageTell,

    /// **The carriage sweeps the rule.**
    ///
    /// - **Behaviour**: the whole press runs [`RULE_LEN`] (`54 px`) along the
    ///   rule at [`SWEEP_SPEED`] (`108 px/s`), leaving a wet ink smear.
    /// - **Duration**: the run's own time (`54 / 108 ≈ 0.5 s`), capped by the
    ///   rule's end or a wall.
    /// - **Warning**: the tell's; the rails glow red while it runs.
    /// - **Dangerous area**: the rule band, for the whole run.
    /// - **Damage**: [`SWEEP_DAMAGE`] (`20`) once.
    CarriageSweep,

    /// **The exposed ribbon (recovery / weak point) at the carriage's stop.**
    ///
    /// - **Behaviour**: the carriage jams at the end of its rule, the ribbon
    ///   lifts out of the spools and the press cannot print until it re-feeds.
    /// - **Duration**: [`RIBBON`] (`1.6 s`), or [`RIBBON_LONG`] (`2.0 s`) after
    ///   a misprint's carriage.
    /// - **Warning**: the ribbon is a bright magenta band across the front of
    ///   the slab with a glint and a cyan bracket around it.
    /// - **Dangerous area**: none (harmless to touch).
    /// - **Damage**: `0` (it *takes* ×1.5 for the whole beat).
    /// - **Counter**: follow it to the stop and hit the ribbon.
    Ribbon,

    /// **Phase-turn pose change (harmless).**
    ///
    /// - **Behaviour**: the frame unlatches, the sheet is stripped, a fresh one
    ///   feeds in, and the type bed re-seats — [`PHASE_TURN`] (`1.2 s`) of
    ///   machinery with no attack at all.
    /// - **Damage**: `0`, harmless to touch.
    PhaseTurn,

    /// **Wreck: the press tears itself apart.**
    ///
    /// - **Behaviour**: the sheets tear loose, the type bed scatters, the frame
    ///   buckles and the ink floods out — [`DEATH_DURATION`] (`2.8 s`). The last
    ///   sheet rolls out under the street door.
    /// - **Damage**: `0` (a pure death beat; the framework reaps it at the end).
    Wreck,
}

/// A `Copy` snapshot of the Night Edition for zero-allocation drawing.
#[derive(Clone, Copy, Debug)]
pub struct NightEditionView {
    /// World-space centre of the platen.
    pub pos: Vec2,
    /// Unit forward vector (`+u` = the carriage side).
    pub facing: Vec2,
    pub state: NightEditionState,
    /// Progress through the current state (`0..=1`).
    pub progress: f32,
    /// Continuous animation clock (key bobbing, ink shimmer, sheet flutter).
    pub anim: f32,
    /// Hit-flash intensity (`0..=1`).
    pub hit_flash: f32,
    /// Remaining health fraction (drives the three damage phases).
    pub hp_frac: f32,
    /// Fight phase (`1..=3`).
    pub phase: u8,
    /// The page's centre (the tell's target end).
    pub page: Vec2,
    /// Which side of the page is blank: `+1` = the `+x` margin, `-1` = `-x`.
    pub margin_side: f32,
    /// The carriage rule's start (behind the press).
    pub rule_from: Vec2,
    /// The carriage rule's end (the brass stop bracket).
    pub rule_to: Vec2,
    /// The arena's press bed (its centre). The Night Edition's own arena
    /// dressing: the recessed stone plinth the press works on, drawn on the
    /// ground layer for the whole fight.
    pub bed: Vec2,
}

impl NightEditionView {
    /// A healthy walking press facing `facing`.
    pub fn roll(pos: Vec2, facing: Vec2, anim: f32) -> Self {
        Self {
            pos,
            facing,
            state: NightEditionState::Roll,
            progress: 0.0,
            anim,
            hit_flash: 0.0,
            hp_frac: 1.0,
            phase: 1,
            page: pos + facing * 30.0,
            margin_side: 1.0,
            rule_from: pos,
            rule_to: pos + facing * RULE_LEN,
            bed: pos,
        }
    }

    /// The unit forward direction (safe for a zero facing).
    #[inline]
    pub fn forward(&self) -> Vec2 {
        if self.facing.len_sq() > 1e-4 {
            self.facing.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        }
    }

    /// The page's side sign as `±1`.
    #[inline]
    pub fn side(&self) -> f32 {
        if self.margin_side >= 0.0 {
            1.0
        } else {
            -1.0
        }
    }
}

// ============================================================================
// Footprints (shared by the controller and the art)
// ============================================================================

/// The centre and half-extents of the **inked** part of a page.
///
/// The page is [`PAGE_HALF`] × 2; the blank [`PAGE_MARGIN`] strip is on the
/// `side` of the sheet (`+1` = `+x`), so the ink is the remaining 24 × 30 px.
#[inline]
pub fn inked_rect(page: Vec2, side: f32) -> (Vec2, Vec2) {
    let s = if side >= 0.0 { 1.0 } else { -1.0 };
    (
        Vec2::new(page.x - s * PAGE_MARGIN * 0.5, page.y),
        Vec2::new(PAGE_HALF.x - PAGE_MARGIN * 0.5, PAGE_HALF.y),
    )
}

/// The 12 px wide clear margin strip of a page: `(centre, half)`.
#[inline]
pub fn margin_rect(page: Vec2, side: f32) -> (Vec2, Vec2) {
    let s = if side >= 0.0 { 1.0 } else { -1.0 };
    (
        Vec2::new(page.x + s * (PAGE_HALF.x - PAGE_MARGIN * 0.5), page.y),
        Vec2::new(PAGE_MARGIN * 0.5, PAGE_HALF.y),
    )
}

/// The two misprint impressions: the second sheet is offset by
/// [`MISPRINT_OFFSET`] *away* from the blank margin, so the two inked areas
/// overlap over the offset and leave one shared 12 px stripe clear.
#[inline]
pub fn misprint_rects(page: Vec2, side: f32) -> [(Vec2, Vec2); 2] {
    let s = if side >= 0.0 { 1.0 } else { -1.0 };
    let second = page + Vec2::new(-s * MISPRINT_OFFSET, 0.0);
    [inked_rect(page, side), inked_rect(second, side)]
}

/// The carriage rule's midpoint and half-extents (an 8 px band).
#[inline]
pub fn rule_band(from: Vec2, to: Vec2) -> (Vec2, Vec2) {
    let mid = (from + to) * 0.5;
    let half = ((to - from).len() * 0.5).max(0.0);
    (mid, Vec2::new(half, RULE_HALF_WIDTH))
}

/// Where the exposed ribbon is when the press stands at `pos`.
#[inline]
pub fn ribbon_pos(pos: Vec2, facing: Vec2) -> Vec2 {
    let f = if facing.len_sq() > 1e-4 {
        facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    pos + f * RIBBON_OFFSET
}

/// The platen slab's oriented hit box: `(centre, facing, half, half_perp)`.
#[inline]
pub fn body_box(view: &NightEditionView) -> (Vec2, Vec2, f32, f32) {
    (
        view.pos,
        view.forward(),
        BODY_HALF_LEN,
        BODY_HALF_WIDTH,
    )
}

/// The exposed weak point for the current state, if it has one: the lowered
/// type bed, or the ribbon lifted out at the carriage's stop.
#[inline]
pub fn weak_point(view: &NightEditionView) -> Option<Vec2> {
    match view.state {
        NightEditionState::BedLowered => Some(view.pos),
        NightEditionState::Ribbon => Some(ribbon_pos(view.pos, view.facing)),
        _ => None,
    }
}

/// Incoming-damage multiplier for a state (the two recovery windows).
#[inline]
pub fn incoming_damage_mult(state: NightEditionState) -> f32 {
    match state {
        NightEditionState::BedLowered | NightEditionState::Ribbon => WEAK_MULT,
        NightEditionState::Wreck => 0.0,
        _ => 1.0,
    }
}

/// Active collision radius for a state. The wreck stops being a body once the
/// frame has gone down.
#[inline]
pub fn collision_radius(state: NightEditionState, progress: f32) -> f32 {
    match state {
        NightEditionState::Wreck if progress >= 0.72 => 0.0,
        _ => BODY_RADIUS,
    }
}

/// Does this state deal body-contact damage at all?
#[inline]
pub fn state_contacts(state: NightEditionState) -> bool {
    matches!(
        state,
        NightEditionState::Roll
            | NightEditionState::PageTell
            | NightEditionState::PageStamp
            | NightEditionState::MisprintTell
            | NightEditionState::MisprintStamp
            | NightEditionState::CarriageTell
            | NightEditionState::CarriageSweep
    )
}

// ============================================================================
// Pixel helpers (zero allocation)
// ============================================================================

#[derive(Clone, Copy)]
struct Frame {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    hit: f32,
    hp: f32,
    prog: f32,
    anim: f32,
    accent: Rgb,
}

impl Frame {
    fn from_view(view: &NightEditionView, cam: Vec2, accent: Rgb) -> Self {
        let f = view.forward();
        let p = f.perp();
        let prog = if view.progress.is_finite() {
            view.progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let hit = if view.hit_flash.is_finite() {
            view.hit_flash.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let hp = if view.hp_frac.is_finite() {
            view.hp_frac.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let anim = if view.anim.is_finite() { view.anim } else { 0.0 };
        Self {
            pos: view.pos - cam,
            f,
            p,
            fl: f.dot(LIGHT),
            pl: p.dot(LIGHT),
            hit,
            hp,
            prog,
            anim,
            accent,
        }
    }

    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.pos + self.f * u + self.p * v
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }
}

#[inline]
fn bbox_around(pb: &PixelBuffer, center: Vec2, reach: f32) -> Option<(i32, i32, i32, i32)> {
    if pb.w == 0 || pb.h == 0 || !center.x.is_finite() || !center.y.is_finite() {
        return None;
    }
    let rr = reach.ceil() as i32;
    let cx = center.x.round() as i32;
    let cy = center.y.round() as i32;
    let x0 = (cx - rr).max(0);
    let x1 = (cx + rr).min(pb.w as i32 - 1);
    let y0 = (cy - rr).max(0);
    let y1 = (cy + rr).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        None
    } else {
        Some((x0, x1, y0, y1))
    }
}

#[inline]
fn bbox_rect(pb: &PixelBuffer, center: Vec2, half: Vec2, pad: f32) -> Option<(i32, i32, i32, i32)> {
    if pb.w == 0 || pb.h == 0 || !center.x.is_finite() || !center.y.is_finite() {
        return None;
    }
    let x0 = (center.x - half.x - pad).floor() as i32;
    let x1 = (center.x + half.x + pad).ceil() as i32;
    let y0 = (center.y - half.y - pad).floor() as i32;
    let y1 = (center.y + half.y + pad).ceil() as i32;
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(pb.w as i32 - 1);
    let y1 = y1.min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        None
    } else {
        Some((x0, x1, y0, y1))
    }
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

#[inline]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(shadow, mid, l / 0.62)
    } else {
        mix(mid, hi, (l - 0.62) / 0.53)
    }
}

#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.0 {
        return c;
    }
    let flash = if hit > 0.55 {
        (255, 246, 222)
    } else {
        (255, 214, 168)
    };
    mix(c, flash, (0.18 + hit * 0.40).min(0.60))
}

/// Deterministic grit hash in `0..1`.
#[inline]
fn hash01(ix: i32, iy: i32) -> f32 {
    let mut n = (ix.wrapping_mul(374_761_393)).wrapping_add(iy.wrapping_mul(668_265_263)) as u32;
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    ((n ^ (n >> 16)) & 0x00ff_ffff) as f32 / 16_777_215.0
}

#[inline]
fn sd_rbox(u: f32, v: f32, hu: f32, hv: f32, r: f32) -> f32 {
    let qu = u.abs() - (hu - r);
    let qv = v.abs() - (hv - r);
    let out = (qu.max(0.0).powi(2) + qv.max(0.0).powi(2)).sqrt();
    let inn = qu.max(qv).min(0.0);
    out + inn - r
}

/// A dashed / dotted line clipped by [`blend_px`].
#[allow(clippy::too_many_arguments)]
fn draw_line_styled(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    c: Rgb,
    alpha: f32,
    dash_on: i32,
    dash_period: i32,
    phase: i32,
) {
    if !a.x.is_finite() || !b.x.is_finite() || pb.w == 0 || pb.h == 0 {
        return;
    }
    let ax = a.x.round() as i32;
    let ay = a.y.round() as i32;
    let bx = b.x.round() as i32;
    let by = b.y.round() as i32;
    let dx = (bx - ax).abs();
    let dy = -(by - ay).abs();
    let sx = if ax < bx { 1 } else { -1 };
    let sy = if ay < by { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (ax, ay);
    let mut step = 0i32;
    let max_steps = (dx - dy + 2).min(320);
    while step <= max_steps {
        let on = dash_period <= 0 || ((step + phase).rem_euclid(dash_period) < dash_on);
        if on {
            blend_px(pb, x, y, c, alpha);
        }
        if x == bx && y == by {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        step += 1;
    }
}

fn draw_ring_styled(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    col: Rgb,
    alpha: f32,
    dash_mod: i32,
    phase: f32,
) {
    if radius < 1.0 || pb.w == 0 || pb.h == 0 {
        return;
    }
    if bbox_around(pb, center, radius + 2.0).is_none() {
        return;
    }
    let steps = ((radius * 6.5).round() as i32).clamp(16, 240);
    let mut last = (i32::MIN, i32::MIN);
    for i in 0..steps {
        if dash_mod > 0 && ((i / dash_mod) % 2 == 1) {
            continue;
        }
        let a = phase + i as f32 / steps as f32 * TAU;
        let x = (center.x + a.cos() * radius).round() as i32;
        let y = (center.y + a.sin() * radius).round() as i32;
        if (x, y) == last {
            continue;
        }
        last = (x, y);
        blend_px(pb, x, y, col, alpha);
    }
}

/// A filled axis-aligned rectangle, blended.
fn fill_rect(pb: &mut PixelBuffer, center: Vec2, half: Vec2, c: Rgb, alpha: f32) {
    let Some((x0, x1, y0, y1)) = bbox_rect(pb, center, half, 0.0) else {
        return;
    };
    let hx = half.x.max(0.0);
    let hy = half.y.max(0.0);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 - center.x).abs();
            let dy = (y as f32 - center.y).abs();
            if dx > hx + 0.49 || dy > hy + 0.49 {
                continue;
            }
            blend_px(pb, x, y, c, alpha);
        }
    }
}

/// The outline of an axis-aligned rectangle, with optional corner ticks.
fn outline_rect(pb: &mut PixelBuffer, center: Vec2, half: Vec2, c: Rgb, alpha: f32, dashed: bool) {
    let x0 = (center.x - half.x).round() as i32;
    let x1 = (center.x + half.x).round() as i32;
    let y0 = (center.y - half.y).round() as i32;
    let y1 = (center.y + half.y).round() as i32;
    let dash = |i: i32| !dashed || (i / 2) % 2 == 0;
    for x in x0..=x1 {
        if dash(x - x0) {
            blend_px(pb, x, y0, c, alpha);
            blend_px(pb, x, y1, c, alpha);
        }
    }
    for y in y0..=y1 {
        if dash(y - y0) {
            blend_px(pb, x0, y, c, alpha);
            blend_px(pb, x1, y, c, alpha);
        }
    }
}

// ============================================================================
// Ground layer
// ============================================================================

/// Draw the Night Edition's floor layer: shadow, page and rule telegraphs,
/// recovery markers, the pose-change ring and the wreck's ink pool.
pub fn draw_ground(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, accent: Rgb) {
    let fr = Frame::from_view(view, cam, accent);
    draw_press_bed(pb, view.bed - cam, accent);
    draw_shadow(pb, &fr);

    match view.state {
        NightEditionState::PageTell | NightEditionState::PageStamp => {
            draw_page(pb, view, cam, fr.prog);
        }
        NightEditionState::MisprintTell | NightEditionState::MisprintStamp => {
            draw_misprint(pb, view, cam, fr.prog);
        }
        NightEditionState::CarriageTell
        | NightEditionState::CarriageSweep
        | NightEditionState::Ribbon => {
            draw_rule(pb, view, cam, &fr);
        }
        NightEditionState::BedLowered => {
            draw_bed_glow(pb, &fr);
        }
        NightEditionState::PhaseTurn => {
            let t = fr.prog;
            let r = (14.0 + 26.0 * t).clamp(0.0, 40.0);
            draw_ring_styled(
                pb,
                fr.pos,
                r,
                mix(PAPER_SPEC, CYAN_HI, 0.4),
                (1.0 - t).clamp(0.0, 1.0) * 0.75,
                3,
                fr.anim * 2.0,
            );
            // Stripped sheets flutter out of the frame.
            for i in 0..4 {
                let a = i as f32 * TAU / 4.0 + 0.4;
                let p = fr.pos + Vec2::from_angle(a) * (10.0 + 22.0 * t);
                fill_rect(pb, p, Vec2::new(4.0, 3.0), PAPER_MID, 0.55 * (1.0 - t));
            }
        }
        NightEditionState::Wreck => {
            draw_wreck_ground(pb, &fr);
        }
        _ => {}
    }
}

/// The arena's press bed: a recessed stone plinth with a type-block border,
/// registration crosses and its own dried-ink stains. This is the Night
/// Edition's arena dressing (the Printworks no longer borrows the Lab's steel
/// bumpers and charging dock); it is ground layer only and collides with
/// nothing.
fn draw_press_bed(pb: &mut PixelBuffer, c: Vec2, accent: Rgb) {
    let half = PRESS_BED_HALF;
    let Some((x0, x1, y0, y1)) = bbox_rect(pb, c, half, 3.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 - c.x).abs();
            let dy = (y as f32 - c.y).abs();
            if dx > half.x + 2.5 || dy > half.y + 2.5 {
                continue;
            }
            let edge = (half.x - dx).min(half.y - dy);
            if edge < 0.0 {
                // The plinth's lip: a dark bevel catching the accent colour.
                darken_px(pb, x, y, 0.30 + 0.10 * (edge + 2.5).clamp(0.0, 1.0));
                blend_px(pb, x, y, mix(IRON_SHADOW, accent, 0.30), 0.22);
                continue;
            }
            if edge < 3.0 {
                // A border of type blocks: alternating newsprint and charcoal
                // ticks, the printworks' own masonry.
                let block = ((dx / 6.0) as i32 + (dy / 6.0) as i32) % 2 == 0;
                let col = if block { IRON_MID } else { IRON_SHADOW };
                blend_px(pb, x, y, col, 0.42);
                if block && hash01(x / 2, y / 2) > 0.72 {
                    blend_px(pb, x, y, PAPER_SHADOW, 0.22);
                }
                continue;
            }
            // The bed surface: stone, with an old impression still legible.
            let grit = hash01(x, y);
            let col = mix(IRON_SHADOW, IRON_MID, 0.35 + 0.25 * grit);
            blend_px(pb, x, y, col, 0.30);
            let in_print = dx < half.x * 0.62 && dy < half.y * 0.62;
            if in_print {
                let glyph = hash01(x / 3 + 17, y / 3 + 5);
                if glyph > 0.66 {
                    blend_px(pb, x, y, INK_MID, 0.20);
                } else if glyph > 0.58 {
                    blend_px(pb, x, y, PAPER_SHADOW, 0.12);
                }
            }
        }
    }
    // Registration crosses at the four corners of the print area.
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let p = c + Vec2::new(sx * (half.x - 6.0), sy * (half.y - 6.0));
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, CYAN_MID, 0.55);
        for k in 1..=2 {
            blend_px(pb, p.x.round() as i32 + k, p.y.round() as i32, CYAN_MID, 0.35);
            blend_px(pb, p.x.round() as i32 - k, p.y.round() as i32, CYAN_MID, 0.35);
            blend_px(pb, p.x.round() as i32, p.y.round() as i32 + k, CYAN_MID, 0.35);
            blend_px(pb, p.x.round() as i32, p.y.round() as i32 - k, CYAN_MID, 0.35);
        }
    }
}

fn draw_shadow(pb: &mut PixelBuffer, fr: &Frame) {
    let c = fr.pos + Vec2::new(2.5, 3.5);
    let Some((x0, x1, y0, y1)) = bbox_around(pb, c, BODY_RADIUS + 3.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = sd_rbox(u - 2.5, v - 3.5, BODY_HALF_LEN + 1.0, BODY_HALF_WIDTH + 1.0, 3.0);
            if d <= 1.5 {
                darken_px(pb, x, y, 0.42 * (1.0 - (d.max(0.0) / 1.5)));
            }
        }
    }
}

/// The page tell and the impression.
fn draw_page(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, prog: f32) {
    let page = view.page - cam;
    let side = view.side();
    let stamping = view.state == NightEditionState::PageStamp;
    // The page is drawn while the tell runs and for the first beat of the
    // impression (the sheet is still on the bed when the type comes down).
    let (ink, ink_half) = inked_rect(page, side);
    let (blank, blank_half) = margin_rect(page, side);
    let pulse = 0.5 + 0.5 * (prog * 9.0).sin();

    // Sheet stock under the ink: a light newsprint wash so the page reads on a
    // dark floor.
    if bbox_rect(pb, page, PAGE_HALF, 2.0).is_some() {
        let alpha = if stamping { 0.32 } else { 0.20 + 0.10 * pulse };
        fill_rect(pb, page, PAGE_HALF, PAPER_MID, alpha);
    }
    // The blank margin, marked as the safe side in registration cyan.
    let safe_alpha = if stamping { 0.85 } else { 0.45 + 0.35 * pulse };
    fill_rect(pb, blank, blank_half, SAFE_CYAN, 0.14 + 0.10 * pulse);
    outline_rect(pb, blank, blank_half, SAFE_CYAN, safe_alpha, false);
    // A chevron on the safe side, pointing out of the ink.
    let tip = blank - Vec2::new(side * (blank_half.x - 1.0), 0.0);
    let tail = tip + Vec2::new(side * 6.0, 0.0);
    draw_line_styled(pb, tail + Vec2::new(0.0, -4.0), tip, SAFE_CYAN, safe_alpha, 0, 0, 0);
    draw_line_styled(pb, tail + Vec2::new(0.0, 4.0), tip, SAFE_CYAN, safe_alpha, 0, 0, 0);

    // The inked area: hatched type that floods dark as the impression nears.
    let flood = if stamping { 1.0 } else { prog * 0.85 };
    fill_rect(pb, ink, ink_half, INK_SHADOW, 0.24 + 0.55 * flood);
    // Reversed letters: a blocky type grid, deterministic.
    let x0 = (ink.x - ink_half.x).round() as i32;
    let x1 = (ink.x + ink_half.x).round() as i32;
    let y0 = (ink.y - ink_half.y).round() as i32;
    let y1 = (ink.y + ink_half.y).round() as i32;
    let mut ty = y0 + 2;
    while ty < y1 - 1 {
        let mut tx = x0 + 2;
        while tx < x1 - 1 {
            let glyph = hash01(tx / 3, ty / 3);
            let lit = mix(INK_HI, PAPER_SPEC, 0.25 + 0.30 * flood);
            if glyph > 0.42 {
                blend_px(pb, tx, ty, lit, 0.35 + 0.45 * flood);
                if glyph > 0.72 {
                    blend_px(pb, tx + 1, ty, lit, 0.30 + 0.40 * flood);
                }
                if glyph > 0.88 {
                    blend_px(pb, tx, ty + 1, lit, 0.28 + 0.36 * flood);
                }
            }
            tx += 3;
        }
        ty += 3;
    }
    // Page edge + crop marks.
    let edge = if stamping { PAPER_SPEC } else { PAPER_HI };
    outline_rect(pb, page, PAGE_HALF, edge, 0.55 + 0.35 * pulse, false);
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = page + Vec2::new(sx * PAGE_HALF.x, sy * PAGE_HALF.y);
        let out_x = Vec2::new(sx * 4.0, 0.0);
        let out_y = Vec2::new(0.0, sy * 4.0);
        draw_line_styled(pb, corner - out_x, corner + out_x, edge, 0.6, 0, 0, 0);
        draw_line_styled(pb, corner - out_y, corner + out_y, edge, 0.6, 0, 0, 0);
    }
    // Impression flash.
    if stamping {
        let flash = (1.0 - prog).clamp(0.0, 1.0);
        fill_rect(pb, ink, ink_half, DANGER_RED, 0.30 * flash);
        outline_rect(pb, ink, ink_half, (255, 236, 210), 0.85 * flash, false);
    }
}

/// The phase-3 misprint: two sheets and the stripe that stays clear.
fn draw_misprint(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, prog: f32) {
    let page = view.page - cam;
    let side = view.side();
    let stamping = view.state == NightEditionState::MisprintStamp;
    let pulse = 0.5 + 0.5 * (prog * 9.0).sin();
    let rects = misprint_rects(page, side);

    // Both sheets first (sheet stock + a flooded ink area each), then the one
    // stripe that stays clear on top of them: the safe lane is the read that
    // matters, so nothing may cover it.
    for (i, (ink, ink_half)) in rects.iter().enumerate() {
        let second = i == 1;
        fill_rect(pb, *ink, *ink_half, PAPER_SHADOW, 0.14 + 0.10 * pulse);
        let flood = if stamping {
            1.0
        } else {
            prog * 0.55 + if second { 0.0 } else { 0.12 }
        };
        fill_rect(pb, *ink, *ink_half, INK_SHADOW, 0.40 + 0.5 * flood);
        // A blocky type grid, so the printed half reads as print.
        let x0 = (ink.x - ink_half.x).round() as i32;
        let x1 = (ink.x + ink_half.x).round() as i32;
        let y0 = (ink.y - ink_half.y).round() as i32;
        let y1 = (ink.y + ink_half.y).round() as i32;
        let mut ty = y0 + 2;
        while ty < y1 - 1 {
            let mut tx = x0 + 2;
            while tx < x1 - 1 {
                if hash01(tx / 3, ty / 3) > 0.45 {
                    blend_px(pb, tx, ty, mix(INK_HI, PAPER_SPEC, 0.3 + 0.3 * flood), 0.4);
                }
                tx += 3;
            }
            ty += 3;
        }
        // The first sheet's border is white, the second's amber: the order is
        // authored, not a coin flip.
        let edge = if second { TELL_AMBER } else { PAPER_SPEC };
        outline_rect(pb, *ink, *ink_half, edge, 0.55 + 0.2 * pulse, true);
    }

    // The shared clear stripe: solid cyan edge to edge, with a chevron pointing
    // into it from the press's side.
    let (blank, blank_half) = margin_rect(page, side);
    fill_rect(pb, blank, blank_half, SAFE_CYAN, 0.12 + 0.06 * pulse);
    let strip_alpha = 0.65 + 0.30 * pulse;
    outline_rect(pb, blank, blank_half, SAFE_CYAN, strip_alpha, false);
    let tip = blank - Vec2::new(side * (blank_half.x - 0.5), 0.0);
    let tail = tip + Vec2::new(side * 5.0, 0.0);
    for dy in [-2.0f32, 2.0] {
        draw_line_styled(
            pb,
            tail + Vec2::new(0.0, dy),
            tip,
            SAFE_CYAN,
            strip_alpha,
            0,
            0,
            0,
        );
    }
    let stripe_tick = mix(SAFE_CYAN, PAPER_SPEC, 0.35);
    fill_rect(pb, blank, Vec2::new(blank_half.x, 1.0), stripe_tick, 0.5);
    if stamping {
        // Each impression flashes on its own beat.
        let first = (prog * 1.9).min(1.0);
        let second = ((prog - 0.45) * 1.9).clamp(0.0, 1.0);
        for (i, (ink, ink_half)) in rects.iter().enumerate() {
            let f = if i == 0 { 1.0 - first } else { 1.0 - second };
            if f > 0.0 {
                fill_rect(pb, *ink, *ink_half, DANGER_RED, 0.28 * f);
                outline_rect(pb, *ink, *ink_half, (255, 236, 210), 0.8 * f, false);
            }
        }
    }
}

/// The carriage rule, its running smear and the exposed-ribbon window.
fn draw_rule(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, fr: &Frame) {
    let from = view.rule_from - cam;
    let to = view.rule_to - cam;
    let dir = to - from;
    let len = dir.len();
    let u = if len > 1e-4 {
        dir * (1.0 / len)
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = Vec2::new(-u.y, u.x);
    let sweeping = view.state == NightEditionState::CarriageSweep;
    let prog = fr.prog;

    // The 8 px band: a rule of dotted type on the floor.
    let band_alpha = if sweeping { 0.42 } else { 0.30 };
    let rail = if sweeping { DANGER_RED } else { PAPER_HI };
    for side in [-1.0f32, 1.0] {
        let off = perp * (RULE_HALF_WIDTH - 0.5) * side;
        draw_line_styled(pb, from + off, to + off, rail, band_alpha + 0.25, 1, 3, 0);
    }
    // The dotted centre rule.
    draw_line_styled(
        pb,
        from,
        to,
        if sweeping { DANGER_RED } else { TELL_AMBER },
        0.55,
        1,
        2,
        0,
    );
    // Marching chevrons toward the stop end (the cue that it runs *away* from
    // where the rat stands).
    let march = (fr.anim * 22.0) % 12.0;
    let mut d = march;
    while d < len - 4.0 {
        let p = from + u * d;
        draw_line_styled(pb, p - perp * 2.0 + u, p + u * 2.5, TELL_AMBER, 0.55, 0, 0, 0);
        draw_line_styled(pb, p + perp * 2.0 + u, p + u * 2.5, TELL_AMBER, 0.55, 0, 0, 0);
        d += 12.0;
    }
    // Brass stop bracket at the far end.
    let bracket = if sweeping { DANGER_RED } else { BRASS_HI };
    for side in [-1.0f32, 1.0] {
        let p = to + perp * (RULE_HALF_WIDTH + 2.0) * side;
        draw_line_styled(pb, p - u * 3.0, p + u * 3.0, bracket, 0.85, 0, 0, 0);
        draw_line_styled(pb, p + u * 3.0, to + u * 3.0, bracket, 0.45, 1, 2, 0);
    }
    // The bell at the near end of the rule, offset clear of the platen so the
    // player can see where the rule starts.
    let bell = from - u * 4.0 + perp * (RULE_HALF_WIDTH + 7.0);
    draw_ring_styled(pb, bell, 3.5, BRASS_HI, 0.75, 0, fr.anim * 3.0);
    fill_rect(pb, bell, Vec2::splat(1.5), BRASS_MID, 0.9);
    draw_line_styled(pb, bell, from + perp * RULE_HALF_WIDTH, BRASS_MID, 0.5, 1, 2, 0);

    if sweeping {
        // Wet ink smear along the swept part of the rule.
        let swept = from + u * (len * prog);
        for side in [-1.0f32, 1.0] {
            let off = perp * (RULE_HALF_WIDTH - 2.0) * side;
            draw_line_styled(
                pb,
                from + off,
                swept + off,
                mix(INK_MID, INK_FRESH, 0.5),
                0.42,
                2,
                5,
                0,
            );
        }
        // The carriage's own pressure smear right under it.
        fill_rect(pb, swept, Vec2::new(3.0, RULE_HALF_WIDTH), INK_SHADOW, 0.35);
    }

    // The ribbon window: a cyan bracket around the exposed ribbon.
    if view.state == NightEditionState::Ribbon {
        let rib = fr.place(RIBBON_OFFSET, 0.0);
        let pulse = 0.5 + 0.5 * (fr.anim * 7.0).sin();
        draw_ring_styled(pb, rib, RIBBON_WINDOW_RADIUS + 2.0, SAFE_CYAN, 0.35 + 0.3 * pulse, 2, 0.0);
        for side in [-1.0f32, 1.0] {
            let p = rib + fr.p * (RIBBON_WINDOW_RADIUS + 1.0) * side;
            draw_line_styled(pb, p, p + fr.f * 4.0, SAFE_CYAN, 0.5 + 0.3 * pulse, 0, 0, 0);
        }
    }
}

/// The lowered bed's hot-ink pool (the punish marker).
fn draw_bed_glow(pb: &mut PixelBuffer, fr: &Frame) {
    let pulse = 0.5 + 0.5 * (fr.anim * 6.0).sin();
    let c = fr.pos;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, c, BED_GLOW_RADIUS + 2.0) else {
        return;
    };
    let inv = 1.0 / BED_GLOW_RADIUS.max(1.0);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = (u * u + v * v).sqrt() * inv;
            if d > 1.0 {
                continue;
            }
            let a = (1.0 - d).powi(2) * (0.24 + 0.16 * pulse);
            blend_px(pb, x, y, mix(INK_FRESH, CYAN_HI, 0.35), a);
        }
    }
    draw_ring_styled(pb, c, BED_GLOW_RADIUS, SAFE_CYAN, 0.25 + 0.25 * pulse, 2, fr.anim);
}

fn draw_wreck_ground(pb: &mut PixelBuffer, fr: &Frame) {
    let t = fr.prog;
    // The ink flood.
    let r = 8.0 + 26.0 * t.min(0.85);
    let c = fr.pos + Vec2::new(0.0, 2.0);
    let Some((x0, x1, y0, y1)) = bbox_around(pb, c, r + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = (y as f32 - c.y) * 1.35;
            let d = (dx * dx + dy * dy).sqrt() / r.max(1.0);
            if d > 1.0 {
                continue;
            }
            let a = (1.0 - d).powi(2) * 0.55;
            blend_px(pb, x, y, INK_SHADOW, a);
            if d > 0.82 {
                blend_px(pb, x, y, INK_MID, a * 0.6);
            }
        }
    }
    // The last sheet rolling out under the street door.
    if t > 0.35 {
        let k = ((t - 0.35) / 0.65).clamp(0.0, 1.0);
        let roll = fr.pos + fr.f * (8.0 + 34.0 * k) + Vec2::new(0.0, 4.0);
        let half = Vec2::new(5.0 + 2.0 * k, 3.0);
        fill_rect(pb, roll, half, PAPER_HI, 0.85);
        outline_rect(pb, roll, half, PAPER_SHADOW, 0.6, false);
        draw_line_styled(pb, roll - half, roll + half, PAPER_SHADOW, 0.4, 0, 0, 0);
    }
}

// ============================================================================
// Entity layer
// ============================================================================

/// Draw the Night Edition's body (and its death beat).
pub fn draw(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, accent: Rgb) {
    let fr = Frame::from_view(view, cam, accent);
    let state = view.state;
    // The press squashes onto the page during the impression and the wreck
    // sinks into the floor.
    let (squash, lift) = match state {
        NightEditionState::PageStamp => (1.0 - 0.10 * fr.prog, 0.0),
        NightEditionState::MisprintStamp => (1.0 - 0.08 * fr.prog, 0.0),
        NightEditionState::Wreck => (1.0 - 0.35 * fr.prog, 6.0 * fr.prog),
        NightEditionState::Ribbon => (1.0, -1.0),
        _ => (1.0, 0.0),
    };
    let _ = lift;

    draw_slab(pb, &fr, view, squash);
    draw_keys(pb, &fr, view);
    draw_type_bed(pb, &fr, view);
    draw_sheet(pb, &fr, view);
    if matches!(
        state,
        NightEditionState::CarriageTell | NightEditionState::CarriageSweep | NightEditionState::Ribbon
    ) {
        draw_carriage(pb, &fr, view);
    }
    draw_ribbon(pb, &fr, view);
    if state == NightEditionState::Wreck {
        draw_wreck_body(pb, &fr);
    }
    draw_hp_arc(pb, &fr);
}

fn slab_colour(fr: &Frame, depth: f32) -> Rgb {
    let lum = 0.42 + 0.46 * fr.lit(0.0, 0.0) + 0.10 * (1.0 - depth);
    apply_hit(shade3(IRON_SHADOW, IRON_MID, IRON_HI, lum), fr.hit)
}

/// The cast-iron platen: rim, recessed face, rivets and phase damage.
fn draw_slab(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView, squash: f32) {
    let hu = BODY_HALF_LEN * squash;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, fr.pos, BODY_RADIUS + 3.0) else {
        return;
    };
    let phase = view.phase.max(1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = sd_rbox(u, v, hu, BODY_HALF_WIDTH, 2.4);
            if d > 1.4 {
                continue;
            }
            if d > 0.0 {
                // Dark rim + the light from the top-left.
                let lit = fr.lit(u / hu.max(1.0), v / BODY_HALF_WIDTH);
                let col = if lit > 0.25 { IRON_HI } else { IRON_SHADOW };
                blend_px(pb, x, y, apply_hit(col, fr.hit), 0.55 * (1.0 - d / 1.4));
                continue;
            }
            // The rim band, then the recessed top face.
            let rim = d > -2.6;
            let lum = if rim {
                0.58 + 0.44 * fr.lit(u.signum() * 0.6, v.signum() * 0.6)
            } else {
                0.38 + 0.36 * fr.lit(u / hu.max(1.0), v / BODY_HALF_WIDTH) - 0.10
            };
            let mut col = apply_hit(shade3(IRON_SHADOW, IRON_MID, IRON_HI, lum), fr.hit);
            if rim && hash01(x / 3, y / 3) > 0.86 {
                col = mix(col, IRON_SPEC, 0.5);
            }
            blend_px(pb, x, y, col, 1.0);
        }
    }
    // Face plate seam and rivets.
    let seam = mix(IRON_SHADOW, IRON_HI, 0.25);
    for side in [-1.0f32, 1.0] {
        let a = fr.place(-hu + 2.0, side * (BODY_HALF_WIDTH - 2.0));
        let b = fr.place(hu - 2.0, side * (BODY_HALF_WIDTH - 2.0));
        draw_line_styled(pb, a, b, seam, 0.55, 0, 0, 0);
        for k in [-1.0f32, 1.0] {
            let rivet = fr.place(k * (hu - 2.5), side * (BODY_HALF_WIDTH - 2.5));
            fill_rect(pb, rivet, Vec2::splat(1.2), IRON_SPEC, 0.75);
        }
    }
    // Phase damage: phase 2 cracks the rim and spills ink, phase 3 splits the
    // frame open.
    if phase >= 2 {
        for i in 0..5 {
            let a = fr.place(-hu + 1.0 + i as f32 * 4.0, -BODY_HALF_WIDTH + 3.0);
            let b = a + fr.f * (2.5 + hash01(i, 7) * 3.0) + fr.p * (2.0 + hash01(i, 11) * 2.0);
            draw_line_styled(pb, a, b, IRON_SHADOW, 0.8, 0, 0, 0);
        }
        // Ink runs down the front face.
        for i in 0..3 {
            let a = fr.place(hu - 3.0, -6.0 + i as f32 * 6.0);
            draw_line_styled(pb, a, a + fr.f * (3.0 + hash01(i, 3) * 3.0), INK_MID, 0.6, 2, 4, 0);
        }
    }
    if phase >= 3 {
        // A gash across the frame, glowing with fresh ink.
        let a = fr.place(-6.0, -BODY_HALF_WIDTH + 2.0);
        let b = fr.place(9.0, BODY_HALF_WIDTH - 3.0);
        draw_line_styled(pb, a, b, INK_SHADOW, 0.9, 0, 0, 0);
        draw_line_styled(pb, a + fr.f, b + fr.f, INK_FRESH, 0.6, 1, 3, 0);
        let sparkle = fr.place(6.0, 4.0);
        let pulse = 0.5 + 0.5 * (fr.anim * 9.0).sin();
        fill_rect(pb, sparkle, Vec2::splat(1.2), mix(INK_FRESH, CYAN_HI, pulse), 0.8);
    }
}

/// The two ivory typewriter keys it walks on, bobbing out of phase.
fn draw_keys(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let walking = !matches!(
        view.state,
        NightEditionState::Ribbon
            | NightEditionState::PhaseTurn
            | NightEditionState::Wreck
            | NightEditionState::BedLowered
    );
    for (i, side) in [-1.0f32, 1.0].iter().enumerate() {
        let bob = if walking {
            (fr.anim * 6.0 + i as f32 * std::f32::consts::PI).sin() * 1.1
        } else {
            0.0
        };
        let c = fr.place(-2.0 + bob * 0.4, side * (BODY_HALF_WIDTH + 1.6));
        // The key's brass stem and its ivory cap.
        let stem = c - fr.p * (*side) * 2.5;
        draw_line_styled(pb, stem, c, BRASS_MID, 0.85, 0, 0, 0);
        let Some((x0, x1, y0, y1)) = bbox_around(pb, c, 4.0) else {
            continue;
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - c.x;
                let dy = y as f32 - c.y;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 3.4 {
                    continue;
                }
                let lum = 0.55 + 0.45 * (-dx * 0.55 - dy * 0.83);
                let col = apply_hit(shade3(IVORY_SHADOW, IVORY_MID, IVORY_HI, lum), fr.hit);
                blend_px(pb, x, y, col, 1.0);
                if d > 2.8 {
                    blend_px(pb, x, y, IRON_SHADOW, 0.35);
                }
            }
        }
        fill_rect(pb, c + Vec2::new(-0.8, -0.8), Vec2::splat(0.9), IVORY_HI, 0.9);
    }
}

/// The recessed type bed, its reversed letters, and the exposed weak point.
fn draw_type_bed(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let exposed = matches!(
        view.state,
        NightEditionState::BedLowered | NightEditionState::Ribbon
    );
    let pulse = 0.5 + 0.5 * (fr.anim * (if exposed { 7.0 } else { 2.4 })).sin();
    let half = BED_HALF;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, fr.pos, half.x.max(half.y) + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = sd_rbox(u, v, half.x, half.y, 1.2);
            if d > 0.8 {
                continue;
            }
            if d > 0.0 {
                // The bed's inner lip catches the light.
                blend_px(pb, x, y, IRON_HI, 0.5 * (1.0 - d / 0.8));
                continue;
            }
            let deep = if exposed { 0.0 } else { 0.22 };
            let base = mix(INK_SHADOW, INK_MID, 0.20 + 0.10 * pulse - deep);
            blend_px(pb, x, y, apply_hit(base, fr.hit * 0.5), 1.0);
        }
    }
    // Reversed letters: a 5 × 4 grid of type blocks.
    for gy in 0..4 {
        for gx in 0..5 {
            let u = -6.0 + gx as f32 * 3.0;
            let v = -5.4 + gy as f32 * 3.6;
            let c = fr.place(u, v);
            let on = hash01(gx, gy) > 0.28;
            if !on {
                continue;
            }
            let col = if exposed {
                mix(INK_FRESH, CYAN_HI, 0.25 + 0.35 * pulse)
            } else {
                mix(INK_MID, INK_HI, 0.45)
            };
            fill_rect(pb, c, Vec2::new(1.0, 1.2), col, if exposed { 0.95 } else { 0.8 });
            if hash01(gx + 9, gy + 4) > 0.6 {
                fill_rect(pb, c + Vec2::new(1.0, 0.0), Vec2::new(0.5, 1.2), col, 0.7);
            }
        }
    }
    // The lowered bed's hot-ink rim: the weak point reads at a glance, with a
    // reticle on its four corners so "hit this" is unmistakable.
    if exposed {
        let rim = if view.state == NightEditionState::Ribbon {
            SAFE_CYAN
        } else {
            mix(INK_FRESH, CYAN_HI, 0.4 + 0.35 * pulse)
        };
        outline_rect(
            pb,
            fr.pos,
            half + Vec2::splat(0.5),
            mix(rim, PAPER_SPEC, 0.35),
            0.7 + 0.3 * pulse,
            false,
        );
        for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = fr.place(su * (half.x + 2.5), sv * (half.y + 2.5));
            let along_u = corner - fr.f * (su * 3.0);
            let along_v = corner - fr.p * (sv * 3.0);
            draw_line_styled(pb, along_u, corner, PAPER_SPEC, 0.85, 0, 0, 0);
            draw_line_styled(pb, along_v, corner, PAPER_SPEC, 0.85, 0, 0, 0);
        }
        draw_ring_styled(
            pb,
            fr.pos,
            BED_GLOW_RADIUS * 0.72,
            rim,
            0.30 + 0.25 * pulse,
            3,
            fr.anim * 2.0,
        );
    } else if view.phase >= 3 {
        // The split bed: a bright fracture through the type.
        let a = fr.place(-7.0, -5.0);
        let b = fr.place(7.0, 5.0);
        draw_line_styled(pb, a, b, INK_FRESH, 0.55, 1, 4, 0);
    }
}

/// The newsprint sheet clamped in the press: this is what makes the silhouette
/// read as a printing press rather than a slab. Stripped while the bed is open
/// (the recovery windows), and fed in during the phase-turn pose change.
fn draw_sheet(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    // The sheet is off the bed whenever the bed is the target, and sliding back
    // in during the phase-turn re-feed.
    let (present, slide) = match view.state {
        NightEditionState::BedLowered | NightEditionState::Ribbon | NightEditionState::Wreck => {
            (false, 0.0)
        }
        NightEditionState::PhaseTurn => (true, -6.0 * (1.0 - fr.prog)),
        _ => (true, 0.0),
    };
    if !present {
        return;
    }
    let centre = fr.place(-1.8 + slide, 0.0);
    let half = Vec2::new(5.2, 8.2);
    let Some((x0, x1, y0, y1)) = bbox_rect(pb, centre, half, 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let du = (x as f32 - centre.x).abs();
            let dv = (y as f32 - centre.y).abs();
            if du > half.x || dv > half.y {
                continue;
            }
            // Serrated leading edge (the sheet is torn from a roll).
            if du > half.x - 1.0 && (y % 3 == 0) {
                continue;
            }
            let lum = 0.48 + 0.42 * fr.lit(-du, -dv);
            let col = apply_hit(shade3(PAPER_SHADOW, PAPER_MID, PAPER_HI, lum), fr.hit * 0.4);
            blend_px(pb, x, y, col, 0.92);
        }
    }
    // The impression already on it, and the two brass clamps.
    let line = mix(INK_MID, PAPER_SHADOW, 0.4);
    for i in 0..4 {
        let a = fr.place(-5.0 + slide, -5.4 + i as f32 * 3.2);
        draw_line_styled(pb, a, a + fr.p * 4.5, line, 0.4, 1, 2, 0);
    }
    for side in [-1.0f32, 1.0] {
        let clamp = fr.place(-6.0 + slide, side * 6.0);
        fill_rect(pb, clamp, Vec2::splat(1.4), BRASS_MID, 0.95);
        fill_rect(pb, clamp + fr.f, Vec2::splat(0.8), BRASS_HI, 0.8);
    }
}

/// The steel carriage rail, its clamp, and the running carriage block.
fn draw_carriage(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let rail_u = -BODY_HALF_LEN + 3.0;
    let rail_half = BODY_HALF_WIDTH + 2.5;
    let a = fr.place(rail_u, -rail_half);
    let b = fr.place(rail_u, rail_half);
    draw_line_styled(pb, a, b, IRON_SPEC, 0.9, 0, 0, 0);
    draw_line_styled(pb, a + fr.f, b + fr.f, IRON_SHADOW, 0.7, 0, 0, 0);
    // End blocks.
    for side in [-1.0f32, 1.0] {
        fill_rect(
            pb,
            fr.place(rail_u, side * rail_half),
            Vec2::splat(1.8),
            BRASS_MID,
            0.9,
        );
    }
    // The clamp: slides along the rail through the tell, locks for the sweep.
    let sweep = view.state == NightEditionState::CarriageSweep;
    let ribbon = view.state == NightEditionState::Ribbon;
    let slide = if sweep {
        0.0
    } else if ribbon {
        rail_half - 3.0
    } else {
        // Sweeping across the rail as the bell rings.
        (fr.prog * 5.0).sin() * (rail_half - 4.0)
    };
    let clamp = fr.place(rail_u, slide);
    fill_rect(pb, clamp, Vec2::new(2.2, 3.0), IRON_HI, 1.0);
    fill_rect(pb, clamp + fr.f * 1.5, Vec2::new(1.0, 2.2), if sweep { DANGER_RED } else { BRASS_HI }, 0.95);
    // Rails glow red while it runs.
    if sweep {
        for side in [-1.0f32, 1.0] {
            let p = fr.place(rail_u, side * rail_half);
            draw_ring_styled(pb, p, 3.0, DANGER_RED, 0.6, 0, fr.anim * 6.0);
        }
    }
}

/// The ink ribbon: two spools, the band, and the lifted window.
fn draw_ribbon(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let exposed = view.state == NightEditionState::Ribbon;
    let pulse = 0.5 + 0.5 * (fr.anim * 8.0).sin();
    // Spools at the rear corners of the slab.
    for side in [-1.0f32, 1.0] {
        let c = fr.place(-6.5, side * (BODY_HALF_WIDTH - 3.5));
        let Some((x0, x1, y0, y1)) = bbox_around(pb, c, 3.5) else {
            continue;
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - c.x;
                let dy = y as f32 - c.y;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 3.0 {
                    continue;
                }
                let lum = 0.5 + 0.5 * (-dx * 0.55 - dy * 0.83);
                blend_px(
                    pb,
                    x,
                    y,
                    shade3(RIBBON_SHADOW, RIBBON_MID, RIBBON_HI, lum),
                    1.0,
                );
                if d > 2.3 {
                    blend_px(pb, x, y, IRON_SHADOW, 0.5);
                }
            }
        }
    }
    // The ribbon band across the front face.
    let u = if exposed { RIBBON_OFFSET } else { BODY_HALF_LEN - 4.0 };
    let a = fr.place(u, -(BODY_HALF_WIDTH - 5.0));
    let b = fr.place(u, BODY_HALF_WIDTH - 5.0);
    if exposed {
        // Lifted out of the spools: a wide, bright magenta band with its two
        // curled ends and a glint that runs its length.
        let col = mix(RIBBON_HI, SAFE_CYAN, 0.20 + 0.22 * pulse);
        for k in [-1.0f32, 1.0] {
            let off = fr.f * k;
            draw_line_styled(pb, a + off, b + off, col, 0.9, 0, 0, 0);
            let end = fr.place(u + k * 2.0, 0.0);
            let left = fr.place(u + k * 2.0, -(BODY_HALF_WIDTH - 5.0));
            let right = fr.place(u + k * 2.0, BODY_HALF_WIDTH - 5.0);
            draw_line_styled(pb, left, end - fr.p * 2.0, RIBBON_SHADOW, 0.6, 1, 2, 0);
            draw_line_styled(pb, right, end + fr.p * 2.0, RIBBON_SHADOW, 0.6, 1, 2, 0);
        }
        let glint = fr.place(u, (pulse * 2.0 - 1.0) * (BODY_HALF_WIDTH - 6.0));
        fill_rect(pb, glint, Vec2::new(1.4, 2.6), (255, 236, 240), 0.95);
        // Spool hubs wind down as the ribbon pays out.
        draw_line_styled(pb, a, b, mix(RIBBON_HI, PAPER_SPEC, 0.4), 0.35 + 0.3 * pulse, 1, 3, 0);
    } else {
        draw_line_styled(pb, a, b, RIBBON_MID, 0.6, 0, 0, 0);
    }
}

fn draw_wreck_body(pb: &mut PixelBuffer, fr: &Frame) {
    let t = fr.prog;
    // Sheets tearing loose and tumbling away from the frame.
    for i in 0..6 {
        let a = i as f32 * TAU / 6.0 + 0.3;
        let k = (t - i as f32 * 0.05).clamp(0.0, 1.0);
        if k <= 0.0 {
            continue;
        }
        let p = fr.pos + Vec2::from_angle(a) * (10.0 + 30.0 * k);
        let spin = k * 6.0;
        let half = Vec2::new(4.0, 3.0);
        let c = p + Vec2::new(spin.cos() * 2.0, spin.sin() * 2.0);
        fill_rect(pb, c, half, PAPER_MID, 0.85 * (1.0 - 0.5 * k));
        outline_rect(pb, c, half, PAPER_SHADOW, 0.55 * (1.0 - 0.5 * k), false);
    }
    // Type blocks scattering from the split bed.
    for i in 0..8 {
        let a = i as f32 * 0.9 + 0.15;
        let k = (t * 1.3 - i as f32 * 0.04).clamp(0.0, 1.0);
        if k <= 0.0 {
            continue;
        }
        let p = fr.pos + Vec2::from_angle(a) * (8.0 + 20.0 * k);
        fill_rect(pb, p, Vec2::splat(1.0), INK_HI, 0.8 * (1.0 - 0.4 * k));
    }
    // A last ink burst at the frame.
    if t < 0.35 {
        let k = 1.0 - t / 0.35;
        draw_ring_styled(pb, fr.pos, 10.0 + 16.0 * (1.0 - k), RIBBON_HI, 0.5 * k, 2, fr.anim);
    }
}

/// The radial HP arc under the body (the HUD bar's own read, in the world).
fn draw_hp_arc(pb: &mut PixelBuffer, fr: &Frame) {
    if fr.hp <= 0.0 {
        return;
    }
    let r = BODY_RADIUS + 5.0;
    let steps = 48;
    let lit = (fr.hp * steps as f32).round() as i32;
    let col = if fr.hp > 0.6 {
        mix(CYAN_MID, CYAN_HI, 0.6)
    } else if fr.hp > 0.3 {
        TELL_AMBER
    } else {
        DANGER_RED
    };
    for i in 0..lit {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 / steps as f32 * TAU;
        let x = (fr.pos.x + a.cos() * r).round() as i32;
        let y = (fr.pos.y + a.sin() * r).round() as i32;
        blend_px(pb, x, y, col, 0.55);
    }
}

// ============================================================================
// Tests (pure geometry only)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inked_page_leaves_the_designed_margin() {
        let page = Vec2::new(100.0, 100.0);
        let (ink, half) = inked_rect(page, 1.0);
        // 24 px of ink, 12 px of margin, on opposite sides.
        assert!((half.x - 12.0).abs() < 1e-6);
        assert!((half.y - PAGE_HALF.y).abs() < 1e-6);
        assert!((ink.x - 94.0).abs() < 1e-6, "ink sits away from the margin");
        let (margin, mh) = margin_rect(page, 1.0);
        assert!((mh.x - 6.0).abs() < 1e-6);
        assert!((margin.x - 112.0).abs() < 1e-6);
        // The two rectangles tile the page exactly.
        assert!((ink.x + half.x).abs() - (margin.x - mh.x).abs() < 1e-6);
        assert!(ink.x + half.x <= margin.x - mh.x + 1e-6);
    }

    #[test]
    fn the_inked_rect_lies_inside_the_page() {
        for side in [1.0f32, -1.0] {
            let page = Vec2::new(-20.0, 8.0);
            let (ink, half) = inked_rect(page, side);
            assert!(ink.x - half.x >= page.x - PAGE_HALF.x - 1e-6);
            assert!(ink.x + half.x <= page.x + PAGE_HALF.x + 1e-6);
            assert!((ink.y - page.y).abs() < 1e-6);
        }
    }

    #[test]
    fn the_misprint_pair_shares_one_clear_stripe() {
        let page = Vec2::new(0.0, 0.0);
        let (margin, mh) = margin_rect(page, 1.0);
        let rects = misprint_rects(page, 1.0);
        // The shared stripe is blank in both impressions.
        for (ink, half) in rects {
            // Strict overlap: the ink may *touch* the stripe's edge (the two
            // rectangles tile the page), but must never cover any of it.
            let overlap = (ink.x + half.x) - (margin.x - mh.x);
            let overlap_rev = (margin.x + mh.x) - (ink.x - half.x);
            assert!(
                overlap <= 1e-4 || overlap_rev <= 1e-4,
                "the clear stripe must be clear in both sheets (overlap {overlap})"
            );
        }
        // ... and the two impressions are exactly MISPRINT_OFFSET apart.
        let dx = (rects[1].0.x - rects[0].0.x).abs();
        assert!((dx - MISPRINT_OFFSET).abs() < 1e-6);
        // The printed union is a full page width.
        let left = (rects[0].0.x - rects[0].1.x).min(rects[1].0.x - rects[1].1.x);
        let right = (rects[0].0.x + rects[0].1.x).max(rects[1].0.x + rects[1].1.x);
        assert!((right - left - PAGE_HALF.x * 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_rule_band_and_ribbon_are_where_the_design_says() {
        let from = Vec2::new(10.0, 20.0);
        let to = Vec2::new(10.0 + RULE_LEN, 20.0);
        let (mid, half) = rule_band(from, to);
        assert!((mid.x - (10.0 + RULE_LEN * 0.5)).abs() < 1e-6);
        assert!((half.x - RULE_LEN * 0.5).abs() < 1e-6);
        assert!((half.y - 4.0).abs() < 1e-6);
        assert!((ribbon_pos(Vec2::ZERO, Vec2::new(1.0, 0.0)).x - RIBBON_OFFSET).abs() < 1e-6);
    }

    #[test]
    fn the_weak_point_and_multiplier_follow_the_recovery_states() {
        let mut v = NightEditionView::roll(Vec2::ZERO, Vec2::new(1.0, 0.0), 0.0);
        assert!(weak_point(&v).is_none());
        assert!((incoming_damage_mult(v.state) - 1.0).abs() < 1e-6);
        v.state = NightEditionState::BedLowered;
        assert_eq!(weak_point(&v), Some(Vec2::ZERO));
        assert!((incoming_damage_mult(v.state) - 1.5).abs() < 1e-6);
        v.state = NightEditionState::Ribbon;
        assert_eq!(weak_point(&v), Some(ribbon_pos(Vec2::ZERO, Vec2::new(1.0, 0.0))));
        assert!((incoming_damage_mult(v.state) - 1.5).abs() < 1e-6);
        assert!(!state_contacts(NightEditionState::Ribbon));
        assert!(state_contacts(NightEditionState::PageTell));
    }

    /// Render a gallery of the fight's poses (env-gated; never part of the
    /// pass/fail suite). `RATRUN_NIGHT_DUMP=/tmp/dir cargo test night_edition_dump`
    /// writes a PPM per pose for the reviewer's rasteriser.
    #[test]
    fn night_edition_dump() {
        let Ok(dir) = std::env::var("RATRUN_NIGHT_DUMP") else {
            return;
        };
        let mut pb = PixelBuffer::new(160, 120);
        let poses = [
            ("roll", NightEditionState::Roll, 0.4),
            ("page_tell", NightEditionState::PageTell, 0.55),
            ("page_stamp", NightEditionState::PageStamp, 0.3),
            ("bed_lowered", NightEditionState::BedLowered, 0.5),
            ("misprint_tell", NightEditionState::MisprintTell, 0.6),
            ("misprint_stamp", NightEditionState::MisprintStamp, 0.4),
            ("carriage_tell", NightEditionState::CarriageTell, 0.6),
            ("carriage_sweep", NightEditionState::CarriageSweep, 0.5),
            ("ribbon", NightEditionState::Ribbon, 0.5),
            ("phase_turn", NightEditionState::PhaseTurn, 0.5),
            ("wreck", NightEditionState::Wreck, 0.6),
        ];
        for (name, state, progress) in poses {
            for phase in [1u8, 3] {
                let mut v = NightEditionView::roll(Vec2::new(70.0, 62.0), Vec2::new(1.0, 0.25), 1.7);
                v.state = state;
                v.progress = progress;
                v.phase = phase;
                v.hp_frac = if phase == 1 { 0.8 } else { 0.22 };
                v.page = v.pos + Vec2::new(16.0, -2.0);
                v.rule_from = v.pos - Vec2::new(8.0, 0.0);
                v.rule_to = v.pos + Vec2::new(46.0, 0.0);
                pb.clear((22, 23, 26));
                // A press-floor grid so the ground layer reads.
                for y in (0..pb.h).step_by(16) {
                    for x in 0..pb.w {
                        pb.plot(x as i32, y as i32, (34, 34, 40));
                    }
                }
                for x in (0..pb.w).step_by(16) {
                    for y in 0..pb.h {
                        pb.plot(x as i32, y as i32, (34, 34, 40));
                    }
                }
                draw_ground(&mut pb, &v, Vec2::ZERO, (120, 120, 140));
                draw(&mut pb, &v, Vec2::ZERO, (120, 120, 140));
                let path = std::path::Path::new(&dir)
                    .join(format!("{name}_p{phase}.ppm"));
                let _ = std::fs::write(path, to_ppm(&pb));
            }
        }
    }

    fn to_ppm(pb: &PixelBuffer) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for px in pb.pixels() {
            out.push(px.0);
            out.push(px.1);
            out.push(px.2);
        }
        out
    }
}
