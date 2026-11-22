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

const OUTLINE_INK: Rgb = (8, 8, 12);
const LEAD_SH: Rgb = (46, 48, 58);
const LEAD_MID: Rgb = (106, 112, 128);
const LEAD_HI: Rgb = (182, 190, 206);
const BRASS_SHADOW: Rgb = (96, 66, 20);

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
    damage_tier: u8,
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
        let phase = view.phase.max(1);
        let damage_tier = if phase >= 3 || hp <= 0.30 {
            3
        } else if phase >= 2 || hp <= 0.65 {
            2
        } else {
            1
        };
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
            damage_tier,
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
fn smoothstep(t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
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
    if a <= 0.004 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.995 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.004 || x < 0 || y < 0 {
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
    if l < 0.56 {
        mix(shadow, mid, l / 0.56)
    } else {
        mix(mid, hi, (l - 0.56) / 0.59)
    }
}

#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.01 {
        return c;
    }
    let flash = if hit > 0.55 {
        (255, 248, 224)
    } else {
        (255, 212, 158)
    };
    mix(c, flash, (0.24 + hit * 0.46).min(0.70))
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
    if !a.x.is_finite()
        || !a.y.is_finite()
        || !b.x.is_finite()
        || !b.y.is_finite()
        || pb.w == 0
        || pb.h == 0
        || alpha <= 0.004
    {
        return;
    }
    let min_x = a.x.min(b.x);
    let max_x = a.x.max(b.x);
    let min_y = a.y.min(b.y);
    let max_y = a.y.max(b.y);
    if max_x < -1.0 || min_x > pb.w as f32 + 1.0 || max_y < -1.0 || min_y > pb.h as f32 + 1.0 {
        return;
    }
    let ax = a.x.clamp(-1e6, 1e6).round() as i64;
    let ay = a.y.clamp(-1e6, 1e6).round() as i64;
    let bx = b.x.clamp(-1e6, 1e6).round() as i64;
    let by = b.y.clamp(-1e6, 1e6).round() as i64;
    let dx = (bx - ax).abs();
    let dy = -(by - ay).abs();
    let sx: i64 = if ax < bx { 1 } else { -1 };
    let sy: i64 = if ay < by { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (ax, ay);
    let mut step = 0i32;
    let max_steps = (dx - dy + 2).min(360) as i32;
    while step <= max_steps {
        let on = dash_period <= 0 || ((step + phase).rem_euclid(dash_period) < dash_on);
        if on && x >= 0 && y >= 0 && x < pb.w as i64 && y < pb.h as i64 {
            blend_px(pb, x as i32, y as i32, c, alpha);
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
    if radius < 1.0 || pb.w == 0 || pb.h == 0 || alpha <= 0.004 {
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

/// The outline of an axis-aligned rectangle (clipped to `pb` so off-screen/huge
/// coordinates never loop unnecessarily).
fn outline_rect(pb: &mut PixelBuffer, center: Vec2, half: Vec2, c: Rgb, alpha: f32, dashed: bool) {
    if !center.x.is_finite()
        || !center.y.is_finite()
        || !half.x.is_finite()
        || !half.y.is_finite()
        || bbox_rect(pb, center, half, 1.0).is_none()
    {
        return;
    }
    let x0 = (center.x - half.x).clamp(-1e6, 1e6).round() as i32;
    let x1 = (center.x + half.x).clamp(-1e6, 1e6).round() as i32;
    let y0 = (center.y - half.y).clamp(-1e6, 1e6).round() as i32;
    let y1 = (center.y + half.y).clamp(-1e6, 1e6).round() as i32;
    let cx0 = x0.max(0);
    let cx1 = x1.min(pb.w as i32 - 1);
    let cy0 = y0.max(0);
    let cy1 = y1.min(pb.h as i32 - 1);
    let dash = |i: i32| !dashed || (i / 2) % 2 == 0;
    for x in cx0..=cx1 {
        if dash(x - x0) {
            blend_px(pb, x, y0, c, alpha);
            blend_px(pb, x, y1, c, alpha);
        }
    }
    for y in cy0..=cy1 {
        if dash(y - y0) {
            blend_px(pb, x0, y, c, alpha);
            blend_px(pb, x1, y, c, alpha);
        }
    }
}

/// Draw an oriented newspaper broadsheet page (for fluttering/tumbling sheets).
fn draw_oriented_sheet(
    pb: &mut PixelBuffer,
    center: Vec2,
    ang: f32,
    hu: f32,
    hv: f32,
    alpha: f32,
) {
    if alpha <= 0.02 {
        return;
    }
    let f = Vec2::from_angle(ang);
    let p = f.perp();
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, hu.max(hv) + 2.5) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = sd_rbox(u, v, hu, hv, 0.4);
            if d > 0.9 {
                continue;
            }
            if d > 0.0 {
                blend_px(pb, x, y, OUTLINE_INK, 0.55 * alpha * (1.0 - d / 0.9));
                continue;
            }
            let lit = (u / hu.max(1.0)) * f.dot(LIGHT) + (v / hv.max(1.0)) * p.dot(LIGHT);
            let mut col = shade3(PAPER_SHADOW, PAPER_MID, PAPER_SPEC, 0.62 + 0.35 * lit);
            // Tiny dark headline & column stripes so it reads as a printed newspaper!
            if u < -hu * 0.35 && u > -hu * 0.72 && v.abs() < hv * 0.75 {
                col = INK_MID;
            } else if u > -hu * 0.15 && ((u * 1.4).floor() as i32) % 2 == 0 && v.abs() < hv * 0.72 {
                col = mix(col, INK_MID, 0.48);
            }
            blend_px(pb, x, y, col, alpha);
        }
    }
}

/// Cel-shaded smoke puff for damaged press bearings / wreck.
fn draw_smoke_puff(pb: &mut PixelBuffer, center: Vec2, r: f32, alpha: f32) {
    if r <= 0.6 || alpha <= 0.02 {
        return;
    }
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, r + 1.2) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d > r {
                continue;
            }
            let lit = (dx / r) * LIGHT.x + (dy / r) * LIGHT.y;
            let col = shade3((26, 24, 34), (66, 62, 78), (126, 122, 140), 0.45 + 0.55 * lit);
            blend_px(pb, x, y, col, alpha * (1.0 - (d / r).powi(2) * 0.4));
        }
    }
}

// ============================================================================
// Ground layer
// ============================================================================

/// Draw the Night Edition's floor layer: press bed plinth, shadow, page and
/// rule telegraphs, recovery markers, pose-change sheets and wreck ink flood.
pub fn draw_ground(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fr = Frame::from_view(view, cam, accent);
    draw_press_bed(pb, view.bed - cam, accent);

    match view.state {
        NightEditionState::PageTell | NightEditionState::PageStamp => {
            draw_page(pb, view, cam, fr.prog, fr.anim);
        }
        NightEditionState::MisprintTell | NightEditionState::MisprintStamp => {
            draw_misprint(pb, view, cam, fr.prog, fr.anim);
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
            let r = (14.0 + 28.0 * t).clamp(0.0, 44.0);
            draw_ring_styled(
                pb,
                fr.pos,
                r,
                mix(PAPER_SPEC, CYAN_HI, 0.45),
                (1.0 - t).clamp(0.0, 1.0) * 0.85,
                3,
                fr.anim * 2.2,
            );
            draw_ring_styled(
                pb,
                fr.pos,
                r * 0.65,
                RIBBON_HI,
                (1.0 - t).clamp(0.0, 1.0) * 0.55,
                0,
                0.0,
            );
            // Stripped printed newsprint pages flutter outward as the new roll loads!
            for i in 0..4 {
                let a = i as f32 * TAU / 4.0 + 0.45 + t * 0.8;
                let p = fr.pos + Vec2::from_angle(a) * (10.0 + 24.0 * t);
                draw_oriented_sheet(pb, p, a + t * 2.5, 4.4, 3.2, 0.82 * (1.0 - t));
            }
        }
        NightEditionState::Wreck => {
            draw_wreck_ground(pb, &fr);
        }
        _ => {}
    }

    draw_shadow(pb, &fr, view.state);
}

/// The arena's press bed: recessed stone plinth with a type-block border,
/// cyan registration crosses and dried-ink press ghosting.
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
                darken_px(pb, x, y, 0.34 + 0.10 * (edge + 2.5).clamp(0.0, 1.0));
                blend_px(pb, x, y, mix(IRON_SHADOW, accent, 0.25), 0.20);
                continue;
            }
            if edge < 3.0 {
                let block = ((dx / 5.0) as i32 + (dy / 5.0) as i32) % 2 == 0;
                let col = if block { IRON_MID } else { IRON_SHADOW };
                blend_px(pb, x, y, col, 0.46);
                if block && hash01(x / 2, y / 2) > 0.72 {
                    blend_px(pb, x, y, PAPER_SHADOW, 0.22);
                }
                continue;
            }
            let grit = hash01(x, y);
            let col = mix(IRON_SHADOW, IRON_MID, 0.32 + 0.22 * grit);
            blend_px(pb, x, y, col, 0.32);
            let in_print = dx < half.x * 0.65 && dy < half.y * 0.65;
            if in_print {
                let glyph = hash01(x / 3 + 17, y / 3 + 5);
                if glyph > 0.66 {
                    blend_px(pb, x, y, INK_MID, 0.24);
                } else if glyph > 0.58 {
                    blend_px(pb, x, y, PAPER_SHADOW, 0.14);
                }
            }
        }
    }
    // Crisp registration target crosses at the four corners of the press bed.
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let p = c + Vec2::new(sx * (half.x - 5.5), sy * (half.y - 5.5));
        let px = p.x.round() as i32;
        let py = p.y.round() as i32;
        blend_px(pb, px, py, CYAN_HI, 0.65);
        for k in 1..=2 {
            blend_px(pb, px + k, py, CYAN_MID, 0.45);
            blend_px(pb, px - k, py, CYAN_MID, 0.45);
            blend_px(pb, px, py + k, CYAN_MID, 0.45);
            blend_px(pb, px, py - k, CYAN_MID, 0.45);
        }
    }
}

fn draw_shadow(pb: &mut PixelBuffer, fr: &Frame, state: NightEditionState) {
    let alpha_scale = if state == NightEditionState::Wreck {
        (1.0 - fr.prog * 0.45).clamp(0.35, 1.0)
    } else {
        1.0
    };
    let c = fr.pos + Vec2::new(2.6, 3.6);
    let Some((x0, x1, y0, y1)) = bbox_around(pb, c, BODY_RADIUS + 5.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * fr.f.x + dy * fr.f.y;
            let v = dx * fr.p.x + dy * fr.p.y;
            let d = sd_rbox(u, v, BODY_HALF_LEN + 0.8, BODY_HALF_WIDTH + 0.8, 2.8);
            if d <= 1.8 {
                darken_px(pb, x, y, 0.48 * (1.0 - (d.max(0.0) / 1.8)) * alpha_scale);
            }
        }
    }
}

/// The page tell and the impression: unmistakable inked danger zone vs bright cyan safe margin.
fn draw_page(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, prog: f32, anim: f32) {
    let page = view.page - cam;
    let side = view.side();
    let stamping = view.state == NightEditionState::PageStamp;
    let (ink, ink_half) = inked_rect(page, side);
    let (blank, blank_half) = margin_rect(page, side);
    let pulse = 0.5 + 0.5 * (anim * 11.0).sin();

    // Newsprint sheet stock underneath.
    if bbox_rect(pb, page, PAGE_HALF, 3.0).is_some() {
        let alpha = if stamping { 0.42 } else { 0.24 + 0.12 * pulse };
        fill_rect(pb, page, PAGE_HALF, PAPER_MID, alpha);
    }

    // 1) INKED DANGER AREA (24 x 30 px): floods dark purple-crimson as the stamp nears!
    let flood = if stamping { 1.0 } else { smoothstep(prog) * 0.90 };
    let danger_fill = mix(INK_SHADOW, (124, 24, 38), if stamping { 0.55 } else { flood * 0.45 });
    fill_rect(pb, ink, ink_half, danger_fill, 0.34 + 0.48 * flood);

    // Rising vertical ink-charge bar sweeping across the inked zone during PageTell!
    if !stamping {
        let fill_h = ink_half.y * 2.0 * prog;
        let bar_y = (ink.y - ink_half.y) + fill_h;
        draw_line_styled(
            pb,
            Vec2::new(ink.x - ink_half.x, bar_y),
            Vec2::new(ink.x + ink_half.x, bar_y),
            mix(TELL_AMBER, DANGER_RED, prog),
            0.85,
            0,
            0,
            0,
        );
    }

    // Crisp newspaper headline banner + reversed type columns inside the inked zone!
    // Bold headline bar across the top of the page.
    let head_col = mix(PAPER_SPEC, DANGER_RED, if stamping { 0.45 } else { prog * 0.35 });
    fill_rect(
        pb,
        Vec2::new(ink.x, ink.y - ink_half.y + 4.0),
        Vec2::new(ink_half.x - 2.5, 1.5),
        head_col,
        0.48 + 0.38 * flood,
    );
    // Newspaper text lines below the headline (clipped to viewport).
    if let Some((bx0, bx1, by0, by1)) = bbox_rect(pb, ink, ink_half, 0.0) {
        let raw_x0 = (ink.x - ink_half.x).round() as i32;
        let raw_y0 = (ink.y - ink_half.y).round() as i32;
        let start_y = (raw_y0 + 8).max(by0);
        let end_y = ((ink.y + ink_half.y).round() as i32 - 2).min(by1);
        let start_x = (raw_x0 + 2).max(bx0);
        let end_x = ((ink.x + ink_half.x).round() as i32 - 2).min(bx1);
        let mut ty = start_y;
        while ty < end_y {
            let mut tx = start_x;
            while tx < end_x {
                let glyph = hash01(tx / 3, ty / 3);
                if glyph > 0.38 {
                    let lit = mix(INK_HI, PAPER_HI, 0.35 + 0.35 * flood);
                    blend_px(pb, tx, ty, lit, 0.40 + 0.45 * flood);
                    if glyph > 0.68 {
                        blend_px(pb, tx + 1, ty, lit, 0.35 + 0.40 * flood);
                    }
                }
                tx += 3;
            }
            ty += 3;
        }
    }

    // Crisp danger border around the inked area.
    let ink_border = if stamping {
        (255, 236, 206)
    } else {
        mix(TELL_AMBER, DANGER_RED, prog)
    };
    outline_rect(pb, ink, ink_half, ink_border, 0.72 + 0.24 * pulse, false);

    // 2) BLANK SAFE MARGIN (12 x 30 px): clean crisp newsprint-white + bright registration cyan!
    let safe_alpha = if stamping { 0.92 } else { 0.65 + 0.30 * pulse };
    fill_rect(pb, blank, blank_half, mix(PAPER_HI, SAFE_CYAN, 0.42), 0.24 + 0.12 * pulse);
    outline_rect(pb, blank, blank_half, SAFE_CYAN, safe_alpha, false);
    // Animated safe-zone chevron pointing cleanly INTO the blank margin!
    let bob = (anim * 8.0).sin() * 1.0;
    let tip = blank + Vec2::new(side * (2.5 + bob), 0.0);
    let tail = tip - Vec2::new(side * 4.5, 0.0);
    for &dy in &[-4.0f32, 4.0] {
        draw_line_styled(pb, tail + Vec2::new(0.0, dy), tip, PAPER_SPEC, safe_alpha, 0, 0, 0);
        draw_line_styled(pb, tail + Vec2::new(0.0, dy) - Vec2::new(side, 0.0), tip - Vec2::new(side, 0.0), SAFE_CYAN, safe_alpha, 0, 0, 0);
    }

    // Printer's corner crop marks around the full 36 x 30 sheet.
    let edge = if stamping { PAPER_SPEC } else { PAPER_HI };
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = page + Vec2::new(sx * PAGE_HALF.x, sy * PAGE_HALF.y);
        draw_line_styled(pb, corner - Vec2::new(sx * 3.5, 0.0), corner + Vec2::new(sx * 3.5, 0.0), edge, 0.75, 0, 0, 0);
        draw_line_styled(pb, corner - Vec2::new(0.0, sy * 3.5), corner + Vec2::new(0.0, sy * 3.5), edge, 0.75, 0, 0, 0);
    }

    // Dramatic impression slam flash + ink-splat burst!
    if stamping {
        let flash = (1.0 - prog).powi(2).clamp(0.0, 1.0);
        fill_rect(pb, ink, ink_half, DANGER_RED, 0.45 * flash);
        outline_rect(pb, ink, ink_half, (255, 246, 224), 0.95 * flash, false);
        for k in 0..8 {
            let a = k as f32 / 8.0 * TAU + 0.2;
            let dir = Vec2::from_angle(a);
            let p0 = ink + Vec2::new(dir.x * ink_half.x, dir.y * ink_half.y);
            let p1 = p0 + dir * (4.0 + 7.0 * prog);
            draw_line_styled(pb, p0, p1, mix(INK_FRESH, DANGER_RED, 0.5), 0.75 * flash, 0, 0, 0);
        }
    }
}

/// The phase-3 misprint: two offset sheets (Beat 1 crimson-white, Beat 2 amber) and the
/// unmistakable shared 12 px clear registration-cyan margin on top.
fn draw_misprint(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, prog: f32, anim: f32) {
    let page = view.page - cam;
    let side = view.side();
    let stamping = view.state == NightEditionState::MisprintStamp;
    let pulse = 0.5 + 0.5 * (anim * 11.0).sin();
    let rects = misprint_rects(page, side);

    for (i, (ink, ink_half)) in rects.iter().enumerate() {
        let second = i == 1;
        fill_rect(pb, *ink, *ink_half, PAPER_SHADOW, 0.16 + 0.08 * pulse);
        let flood = if stamping {
            1.0
        } else {
            prog * 0.60 + if second { 0.0 } else { 0.18 }
        };
        let fill_col = if second {
            mix(INK_SHADOW, (120, 62, 22), 0.35)
        } else {
            mix(INK_SHADOW, (124, 24, 42), 0.42)
        };
        fill_rect(pb, *ink, *ink_half, fill_col, 0.38 + 0.46 * flood);

        if let Some((bx0, bx1, by0, by1)) = bbox_rect(pb, *ink, *ink_half, 0.0) {
            let start_y = ((ink.y - ink_half.y).round() as i32 + 3).max(by0);
            let end_y = ((ink.y + ink_half.y).round() as i32 - 2).min(by1);
            let start_x = ((ink.x - ink_half.x).round() as i32 + 2).max(bx0);
            let end_x = ((ink.x + ink_half.x).round() as i32 - 2).min(bx1);
            let mut ty = start_y;
            while ty < end_y {
                let mut tx = start_x;
                while tx < end_x {
                    if hash01(tx / 3 + i as i32 * 5, ty / 3) > 0.42 {
                        blend_px(pb, tx, ty, mix(INK_HI, PAPER_SPEC, 0.35 + 0.30 * flood), 0.46);
                    }
                    tx += 3;
                }
                ty += 3;
            }
        }
        let edge = if second { TELL_AMBER } else { mix(PAPER_SPEC, DANGER_RED, prog * 0.65) };
        outline_rect(pb, *ink, *ink_half, edge, 0.72 + 0.20 * pulse, second);
    }

    // Shared 12 px clear margin drawn BOLDLY on top so the safe lane is unmistakable!
    let (blank, blank_half) = margin_rect(page, side);
    fill_rect(pb, blank, blank_half, mix(PAPER_SPEC, SAFE_CYAN, 0.45), 0.26 + 0.12 * pulse);
    let strip_alpha = 0.78 + 0.20 * pulse;
    outline_rect(pb, blank, blank_half, SAFE_CYAN, strip_alpha, false);
    // Double safe chevrons vertically stacked inside the shared safe stripe.
    for &cy in &[-6.0f32, 6.0] {
        let tip = blank + Vec2::new(side * 2.5, cy);
        let tail = tip - Vec2::new(side * 4.5, 0.0);
        draw_line_styled(pb, tail + Vec2::new(0.0, -3.0), tip, PAPER_SPEC, strip_alpha, 0, 0, 0);
        draw_line_styled(pb, tail + Vec2::new(0.0, 3.0), tip, PAPER_SPEC, strip_alpha, 0, 0, 0);
    }

    if stamping {
        // Beat 1 stamps immediately (`prog 0.0..0.55`); beat 2 lands at `prog ≈ 0.55..1.0`
        // and must not flash before it lands!
        let f0 = (1.0 - (prog * 1.9).clamp(0.0, 1.0)).powi(2);
        let f1 = if prog >= 0.55 {
            let second = ((prog - 0.55) / 0.45).clamp(0.0, 1.0);
            (1.0 - second).powi(2)
        } else {
            0.0
        };
        for (i, (ink, ink_half)) in rects.iter().enumerate() {
            let f = if i == 0 { f0 } else { f1 };
            if f > 0.01 {
                fill_rect(pb, *ink, *ink_half, DANGER_RED, 0.44 * f);
                outline_rect(pb, *ink, *ink_half, (255, 244, 218), 0.92 * f, false);
            }
        }
    }
}

/// The carriage rule (`54 × 8`), ringing brass bell, wet ink smear, and exposed ribbon bracket.
fn draw_rule(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, fr: &Frame) {
    let from = view.rule_from - cam;
    let to = view.rule_to - cam;
    if !from.x.is_finite() || !from.y.is_finite() || !to.x.is_finite() || !to.y.is_finite() {
        return;
    }
    let dir = to - from;
    let len = dir.len();
    let u = if len > 1e-4 {
        dir * (1.0 / len)
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = Vec2::new(-u.y, u.x);
    let sweeping = view.state == NightEditionState::CarriageSweep;
    let telling = view.state == NightEditionState::CarriageTell;
    let ribbon = view.state == NightEditionState::Ribbon;
    let prog = fr.prog;
    let pulse = 0.5 + 0.5 * (fr.anim * 12.0).sin();

    // Rasterize the exact `±RULE_HALF_WIDTH` corridor over a clipped screen AABB (zero diagonal
    // gaps, no 120 px truncation). During `Ribbon` recovery the rule is harmless, so show only a
    // subdued spent ink track instead of an amber danger carpet!
    let pad = RULE_HALF_WIDTH + 1.5;
    let x0 = (from.x.min(to.x) - pad).floor().max(0.0) as i32;
    let x1 = (from.x.max(to.x) + pad).ceil().min((pb.w as i32 - 1) as f32) as i32;
    let y0 = (from.y.min(to.y) - pad).floor().max(0.0) as i32;
    let y1 = (from.y.max(to.y) + pad).ceil().min((pb.h as i32 - 1) as f32) as i32;
    if x0 <= x1 && y0 <= y1 {
        let (band_col, band_a) = if sweeping {
            (DANGER_RED, 0.22)
        } else if ribbon {
            (INK_SHADOW, 0.16)
        } else {
            (TELL_AMBER, 0.11 + 0.06 * pulse)
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let rx = x as f32 - from.x;
                let ry = y as f32 - from.y;
                let along = rx * u.x + ry * u.y;
                if along < 0.0 || along > len {
                    continue;
                }
                let across = (rx * perp.x + ry * perp.y).abs();
                if across <= RULE_HALF_WIDTH {
                    blend_px(pb, x, y, band_col, band_a);
                }
            }
        }
    }

    // Crisp outer dotted rails (muted cool iron during harmless Ribbon recovery).
    let rail = if sweeping {
        mix(DANGER_RED, PAPER_SPEC, 0.25)
    } else if ribbon {
        IRON_MID
    } else {
        mix(PAPER_HI, TELL_AMBER, 0.45 + 0.35 * prog)
    };
    let rail_a = if ribbon { 0.42 } else { 0.85 };
    for side in [-1.0f32, 1.0] {
        let off = perp * (RULE_HALF_WIDTH - 0.3) * side;
        draw_line_styled(pb, from + off, to + off, rail, rail_a, 2, 4, -(fr.anim * 18.0) as i32);
    }
    // Centre printer's dotted rule.
    draw_line_styled(
        pb,
        from,
        to,
        if sweeping {
            (255, 232, 198)
        } else if ribbon {
            IRON_SHADOW
        } else {
            TELL_AMBER
        },
        if ribbon { 0.38 } else { 0.72 },
        1,
        3,
        0,
    );

    // Marching directional chevrons along the rule only while active (`CarriageTell` / `CarriageSweep`).
    if !ribbon {
        let march = (fr.anim * (if sweeping { 36.0 } else { 22.0 })) % 11.0;
        let mut d = march;
        let max_d = (len - 4.0).min(400.0);
        while d < max_d {
            let p = from + u * d;
            let col = if sweeping { (255, 236, 200) } else { TELL_AMBER };
            draw_line_styled(pb, p - perp * 2.5 - u * 1.2, p + u * 2.2, col, 0.80, 0, 0, 0);
            draw_line_styled(pb, p + perp * 2.5 - u * 1.2, p + u * 2.2, col, 0.80, 0, 0, 0);
            d += 11.0;
        }
    }

    // Chunky brass stop bracket at the far end (`rule_to`).
    let bracket = if sweeping { DANGER_RED } else { BRASS_HI };
    draw_line_styled(pb, to - perp * (RULE_HALF_WIDTH + 2.5), to + perp * (RULE_HALF_WIDTH + 2.5), OUTLINE_INK, 0.8, 0, 0, 0);
    draw_line_styled(pb, to - perp * (RULE_HALF_WIDTH + 2.0), to + perp * (RULE_HALF_WIDTH + 2.0), bracket, 0.95, 0, 0, 0);
    for side in [-1.0f32, 1.0] {
        let p = to + perp * (RULE_HALF_WIDTH + 2.0) * side;
        draw_line_styled(pb, p - u * 3.5, p + u * 1.0, bracket, 0.92, 0, 0, 0);
    }

    // Gleaming brass bell at the near end + ringing acoustic arcs during CarriageTell!
    let bell = from - u * 3.5 + perp * (RULE_HALF_WIDTH + 7.5);
    fill_rect(pb, bell, Vec2::splat(2.4), OUTLINE_INK, 0.82);
    fill_rect(pb, bell, Vec2::splat(1.8), BRASS_MID, 0.96);
    blend_px(pb, (bell.x - 0.7).round() as i32, (bell.y - 0.7).round() as i32, BRASS_HI, 0.98);
    draw_ring_styled(pb, bell, 3.6, BRASS_HI, 0.85, 0, fr.anim * 3.0);
    if telling {
        // Expanding golden bell chime rings!
        let chime = (fr.anim * 3.2).fract();
        draw_ring_styled(pb, bell, 4.5 + chime * 7.5, BRASS_HI, (1.0 - chime) * 0.85, 0, 0.0);
    }

    if sweeping {
        // Wet glossy purple-black ink smear laid down behind the carriage's actual swept position!
        let swept_d = ((fr.pos - from).dot(u)).clamp(0.0, len);
        let swept = from + u * swept_d;
        for side in [-1.0f32, 1.0] {
            let off = perp * (RULE_HALF_WIDTH - 1.6) * side;
            draw_line_styled(pb, from + off, swept + off, INK_FRESH, 0.65, 3, 5, 0);
        }
        draw_line_styled(pb, from, swept, INK_SHADOW, 0.60, 0, 0, 0);
    }

    // Exposed Ribbon weak-point ground reticle at the stopped carriage end!
    if ribbon {
        let rib = fr.place(RIBBON_OFFSET, 0.0);
        let r = RIBBON_WINDOW_RADIUS + 2.2 + 1.2 * pulse;
        draw_ring_styled(pb, rib, r, SAFE_CYAN, 0.65 + 0.30 * pulse, 2, fr.anim * 1.8);
        for i in 0..4 {
            let a = fr.anim * 1.4 + i as f32 * std::f32::consts::FRAC_PI_2;
            let d_v = Vec2::from_angle(a);
            draw_line_styled(pb, rib + d_v * (r + 2.8), rib + d_v * (r - 0.6), PAPER_SPEC, 0.88, 0, 0, 0);
        }
    }
}

/// The lowered type bed's glowing fresh-ink punish ring!
fn draw_bed_glow(pb: &mut PixelBuffer, fr: &Frame) {
    let pulse = 0.5 + 0.5 * (fr.anim * 8.0).sin();
    let c = fr.pos;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, c, BED_GLOW_RADIUS + 4.0) else {
        return;
    };
    let inv = 1.0 / (BED_GLOW_RADIUS + 2.0);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = (u * u + v * v).sqrt() * inv;
            if d > 1.0 {
                continue;
            }
            let a = (1.0 - d).powi(2) * (0.28 + 0.18 * pulse);
            blend_px(pb, x, y, mix(RIBBON_HI, CYAN_HI, 0.45), a);
        }
    }
    let r = BED_GLOW_RADIUS + 1.8 + 1.4 * pulse;
    draw_ring_styled(pb, c, r, SAFE_CYAN, 0.60 + 0.32 * pulse, 2, fr.anim * 1.6);
}

/// Wreck ground layer: spreading glossy ink flood + the last broadsheet rolling out under the street door!
fn draw_wreck_ground(pb: &mut PixelBuffer, fr: &Frame) {
    let t = fr.prog;
    let r = 9.0 + 27.0 * smoothstep((t / 0.85).min(1.0));
    let c = fr.pos + Vec2::new(0.0, 2.0);
    if let Some((x0, x1, y0, y1)) = bbox_around(pb, c, r + 3.0) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - c.x;
                let dy = (y as f32 - c.y) * 1.25;
                let wobble = (dx.atan2(dy) * 5.0).sin() * 2.2;
                let d = ((dx * dx + dy * dy).sqrt() + wobble) / r.max(1.0);
                if d > 1.0 {
                    continue;
                }
                let a = (1.0 - d.powi(3)) * 0.72;
                let col = if d > 0.80 {
                    INK_MID
                } else if d < 0.35 && dx < 0.0 && dy < 0.0 {
                    mix(INK_SHADOW, INK_FRESH, 0.35)
                } else {
                    INK_SHADOW
                };
                blend_px(pb, x, y, col, a);
            }
        }
    }
    // THE LAST SHEET: tears loose and unrolls dramatically forward out from under the wrecked press!
    if t > 0.28 {
        let k = smoothstep(((t - 0.28) / 0.72).clamp(0.0, 1.0));
        let start = fr.place(6.0, 1.5);
        let tip = fr.place(12.0 + 38.0 * k, 3.0 + (k * 4.0).sin() * 2.5);
        // Unrolled paper trail behind the rolling scroll cylinder!
        let mid = (start + tip) * 0.5;
        let len = (tip - start).len() * 0.5;
        draw_oriented_sheet(pb, mid, fr.f.angle(), len.max(4.0), 4.6, 0.90);
        // Curled white paper roll leading edge!
        let perp = fr.p;
        draw_line_styled(pb, tip - perp * 4.8, tip + perp * 4.8, OUTLINE_INK, 0.85, 0, 0, 0);
        draw_line_styled(pb, tip - fr.f - perp * 4.6, tip - fr.f + perp * 4.6, PAPER_SPEC, 0.96, 0, 0, 0);
        draw_line_styled(pb, tip - fr.f * 2.0 - perp * 4.6, tip - fr.f * 2.0 + perp * 4.6, PAPER_SHADOW, 0.75, 0, 0, 0);
    }
}

// ============================================================================
// Entity layer
// ============================================================================

/// Draw the Night Edition's body (and its multi-stage wreck collapse).
pub fn draw(pb: &mut PixelBuffer, view: &NightEditionView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let mut fr = Frame::from_view(view, cam, accent);
    let state = view.state;

    // Expressive anticipation, squash-and-stretch, and recoil per state!
    let (squash, shift_u, shake) = match state {
        NightEditionState::Roll => {
            let sway = (fr.anim * 5.2).sin() * 0.7;
            (1.0, (fr.anim * 5.2).cos().abs() * 0.4, fr.p * sway)
        }
        NightEditionState::PageTell | NightEditionState::MisprintTell => {
            // Platen rears up slightly in anticipation, trembling before the slam!
            let lift = smoothstep(fr.prog);
            let vib = (fr.anim * 36.0).sin() * 0.55 * fr.prog;
            (1.0 + 0.06 * lift, -1.6 * lift, fr.p * vib)
        }
        NightEditionState::PageStamp | NightEditionState::MisprintStamp => {
            // Heavy downward slam squash + impact rebound!
            let slam = (1.0 - fr.prog).powi(2);
            (1.0 - 0.14 * slam, 1.8 * slam, fr.f * (0.9 * slam))
        }
        NightEditionState::CarriageTell => {
            let vib = (fr.anim * 44.0).sin() * 0.7 * fr.prog;
            (1.0, -1.2 * fr.prog, fr.p * vib)
        }
        NightEditionState::CarriageSweep => (1.05, 1.0, Vec2::ZERO),
        NightEditionState::BedLowered => (0.96, 0.0, Vec2::ZERO),
        // Keep zero longitudinal shift in Ribbon so the lifted silk ribbon aligns with `weak_point`.
        NightEditionState::Ribbon => (1.0, 0.0, Vec2::ZERO),
        NightEditionState::PhaseTurn => {
            let sh = (fr.anim * 48.0).sin() * 1.4 * (1.0 - fr.prog);
            (1.0, 0.0, fr.p * sh)
        }
        NightEditionState::Wreck => {
            let sh = if fr.prog < 0.35 {
                (fr.anim * 60.0).sin() * 1.8 * (1.0 - fr.prog / 0.35)
            } else {
                0.0
            };
            (1.0 - 0.22 * smoothstep(fr.prog), 0.0, fr.p * sh)
        }
    };
    fr.pos += fr.f * shift_u + shake;

    // 1) Two bright vintage typewriter keys striding out from the flanks!
    draw_keys(pb, &fr, view);

    // 2) Heavy cast-iron platen slab with crisp dark outline, bevels, corner bolts & damage cracks.
    draw_slab(pb, &fr, view, squash);

    // 3) Recessed square movable type bed (oriented cleanly in local space!).
    draw_type_bed(pb, &fr, view);

    // 4) Clamped newsprint broadsheet (oriented cleanly in local space!).
    draw_sheet(pb, &fr, view);

    // 5) Brushed-steel carriage rail & sliding clamp (always subtly visible as hardware;
    //    highlighted & animated during CarriageTell / CarriageSweep / Ribbon!).
    draw_carriage(pb, &fr, view);

    // 6) Spoked brass-and-crimson ink-ribbon spools + lifted silk ribbon weak point!
    draw_ribbon(pb, &fr, view);

    // 6b) Unmistakable golden weak-point reticle & ticks when BedLowered or Ribbon exposes ×1.5!
    if matches!(state, NightEditionState::BedLowered | NightEditionState::Ribbon) {
        if let Some(wp) = weak_point(view) {
            let wc = wp - cam;
            let pulse = 0.5 + 0.5 * (fr.anim * 12.0).sin();
            let rad = if state == NightEditionState::BedLowered { 8.5 } else { 7.5 } + pulse * 1.2;
            draw_ring_styled(pb, wc, rad + 0.9, OUTLINE_INK, 0.55, 0, fr.anim);
            draw_ring_styled(pb, wc, rad, mix(BRASS_HI, TELL_AMBER, 0.45), 0.78 + 0.20 * pulse, 2, fr.anim * 1.4);
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + fr.anim * 1.5;
                let dir = Vec2::from_angle(a);
                let p0 = wc + dir * (rad + 2.8);
                let p1 = wc + dir * (rad - 1.0);
                draw_line_styled(pb, p0, p1, PAPER_SPEC, 0.92, 0, 0, 0);
            }
        }
    }

    // 7) Damage smoke wisps (Phase 2/3) & hit-flash sparks.
    if fr.damage_tier >= 2 && state != NightEditionState::Wreck {
        draw_damage_smoke(pb, &fr);
    }
    if fr.hit > 0.10 {
        draw_hit_sparks(pb, &fr);
    }

    // 8) Wreck debris: exploding newspaper sheets, lead type slugs & ink burst!
    if state == NightEditionState::Wreck {
        draw_wreck_body(pb, &fr);
    } else {
        draw_hp_arc(pb, &fr);
    }
}

/// The cast-iron platen: dark outline, chamfered rim, recessed chase, brass corner bolts & phase cracks.
fn draw_slab(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView, squash: f32) {
    let hu = BODY_HALF_LEN * squash;
    let hv = BODY_HALF_WIDTH;
    let wreck_split = if view.state == NightEditionState::Wreck {
        smoothstep(((fr.prog - 0.25) / 0.55).clamp(0.0, 1.0))
    } else {
        0.0
    };
    let alpha = if view.state == NightEditionState::Wreck && fr.prog > 0.72 {
        1.0 - smoothstep(((fr.prog - 0.72) / 0.28).clamp(0.0, 1.0)) * 0.45
    } else {
        1.0
    };
    let Some((x0, x1, y0, y1)) = bbox_around(pb, fr.pos, BODY_RADIUS + 6.0) else {
        return;
    };
    let pulse = 0.5 + 0.5 * (fr.anim * 10.0).sin();
    let phase2 = fr.damage_tier >= 2;
    let phase3 = fr.damage_tier >= 3;
    let rim_col = if fr.hit > 0.45 { (54, 34, 24) } else { OUTLINE_INK };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u_raw, v_raw) = fr.local(x, y);
            // In Wreck, the platen splits diagonally into two buckling halves!
            let split_gap = wreck_split * 3.2;
            let diag = u_raw * 0.7 + v_raw * 0.7;
            if split_gap > 0.4 && diag.abs() < split_gap * 0.55 {
                continue;
            }
            let shift = if diag >= 0.0 { split_gap * 0.45 } else { -split_gap * 0.45 };
            let u = u_raw - shift;
            let v = v_raw - shift;

            let d = sd_rbox(u, v, hu, hv, 2.2);
            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.25 {
                    blend_px(pb, x, y, rim_col, 0.94 * alpha);
                }
                continue;
            }

            let nu = (u / hu.max(1.0)).clamp(-1.0, 1.0);
            let nv = (v / hv).clamp(-1.0, 1.0);
            let lit = fr.lit(nu, nv);
            let outer_rim = d > -2.8;
            let bevel = if outer_rim {
                if lit > 0.12 { 0.30 } else { -0.22 }
            } else {
                -0.14
            };
            let mut col = shade3(IRON_SHADOW, IRON_MID, IRON_HI, 0.44 + 0.54 * lit + bevel);
            // Crisp machined top-left specular sheen on the cast-iron bevel so it survives card downscale!
            if outer_rim && d > -1.45 && lit > 0.25 {
                col = mix(col, IRON_SPEC, 0.72);
            }
            // Embossed side grip ribs on the iron rim.
            if outer_rim && (u.abs() < hu - 4.0) && (v.abs() > hv - 2.3) && ((u * 1.6).sin() > 0.4) {
                col = mix(col, IRON_SHADOW, 0.42);
            }
            // Recessed inner chase seam line.
            if (d + 2.8).abs() < 0.48 {
                col = mix(col, OUTLINE_INK, 0.72);
            }
            // Gleaming brass hex bolts at all four platen corners!
            if (u.abs() - (hu - 2.4)).abs() < 1.1 && (v.abs() - (hv - 2.4)).abs() < 1.1 {
                let b_d = ((u.abs() - (hu - 2.4)).powi(2) + (v.abs() - (hv - 2.4)).powi(2)).sqrt();
                if b_d < 1.05 {
                    col = shade3(BRASS_SHADOW, BRASS_MID, BRASS_HI, 0.65 + 0.40 * lit);
                }
            }
            // Tiny registration-cyan alignment ticks centered on each edge of the platen!
            if (u.abs() > hu - 2.4 && v.abs() < 0.7) || (v.abs() > hv - 2.4 && u.abs() < 0.7) {
                col = mix(col, CYAN_HI, 0.72);
            }

            // Phase 2/3 jagged cast-iron cracks & leaking ink!
            if phase2 {
                let c1 = ((u + 4.5) - (v + 3.0) * 0.7 + (v * 1.5).sin() * 0.4).abs();
                if u < -1.0 && v < 4.0 && c1 < 0.65 {
                    col = INK_SHADOW;
                }
            }
            if phase3 {
                let c2 = ((u - 2.5) + (v - 1.0) * 0.85 + (u * 1.3).cos() * 0.45).abs();
                if c2 < 0.75 {
                    col = mix(INK_SHADOW, mix(INK_FRESH, CYAN_HI, pulse), 0.70);
                } else if c2 < 1.5 {
                    col = mix(col, INK_MID, 0.52);
                }
            }

            let final_col = apply_hit(col, fr.hit * 0.45);
            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, rim_col, 0.84 * alpha);
                pb.plot(x, y, mix(rimmed, final_col, cover * alpha));
            } else {
                blend_px(pb, x, y, final_col, alpha);
            }
        }
    }
}

/// The two bright vintage typewriter keys it strides on (brass bezel, glossy ivory cap, key legend)!
fn draw_keys(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let walking = matches!(
        view.state,
        NightEditionState::Roll | NightEditionState::PageTell | NightEditionState::MisprintTell
    );
    let wreck = view.state == NightEditionState::Wreck;
    let alpha = if wreck && fr.prog > 0.75 {
        1.0 - smoothstep(((fr.prog - 0.75) / 0.25).clamp(0.0, 1.0)) * 0.5
    } else {
        1.0
    };

    for (i, &side) in [-1.0f32, 1.0].iter().enumerate() {
        let phase = fr.anim * 5.2 + i as f32 * std::f32::consts::PI;
        let stride = if walking { phase.sin() * 2.8 } else { 0.0 };
        let lift = if walking { phase.cos().max(0.0) * 1.6 } else { 0.0 };
        let splay = if matches!(view.state, NightEditionState::BedLowered | NightEditionState::Ribbon) {
            1.6
        } else if wreck {
            2.4 * smoothstep(fr.prog)
        } else {
            0.0
        };

        let c = fr.place(-1.2 + stride, side * (BODY_HALF_WIDTH + 3.0 + splay))
            + Vec2::new(0.0, -lift);

        // Angled brass key-lever arm connecting the platen underbody to the keycap!
        let anchor = fr.place(-2.2, side * (BODY_HALF_WIDTH - 1.5));
        draw_line_styled(pb, anchor + Vec2::new(0.0, 1.0), c + Vec2::new(0.0, 1.0), OUTLINE_INK, 0.8 * alpha, 0, 0, 0);
        draw_line_styled(pb, anchor, c, BRASS_MID, 0.95 * alpha, 0, 0, 0);

        // Contact shadow beneath each stepping keycap.
        let sh = c + Vec2::new(1.0, 2.2 + lift * 0.7);
        if let Some((x0, x1, y0, y1)) = bbox_around(pb, sh, 4.5) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = ((x as f32 - sh.x).powi(2) + (y as f32 - sh.y).powi(2)).sqrt();
                    if d < 4.0 {
                        darken_px(pb, x, y, 0.34 * (1.0 - d / 4.0) * alpha);
                    }
                }
            }
        }

        // Vintage round typewriter key: dark outer rim -> metallic brass bezel -> glossy ivory cap -> stamped legend!
        let Some((x0, x1, y0, y1)) = bbox_around(pb, c, 5.2) else {
            continue;
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - c.x;
                let dy = y as f32 - c.y;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 4.6 {
                    continue;
                }
                if d > 3.85 {
                    blend_px(pb, x, y, OUTLINE_INK, 0.85 * alpha * (1.0 - (d - 3.85) / 0.75));
                    continue;
                }
                let lit = (-dx * 0.55 - dy * 0.83) / 3.8;
                let col = if d > 2.95 {
                    // Gleaming metallic brass/chrome typewriter key bezel ring!
                    shade3(BRASS_SHADOW, BRASS_MID, BRASS_HI, 0.55 + 0.55 * lit)
                } else {
                    // Concave bright ivory keycap with crisp top-left glint!
                    let dome = (1.0 - (d / 2.95).powi(2) * 0.45).clamp(0.0, 1.0);
                    let mut kcol = shade3(IVORY_SHADOW, IVORY_MID, IVORY_HI, 0.50 + 0.30 * dome + 0.45 * lit);
                    if d < 2.2 && lit > 0.35 {
                        kcol = (255, 254, 246);
                    }
                    kcol
                };
                blend_px(pb, x, y, apply_hit(col, fr.hit * 0.45), alpha);
            }
        }
        // Crisp dark stamped typewriter key legend right on the center of each ivory key!
        let cx = c.x.round() as i32;
        let cy = c.y.round() as i32;
        if i == 0 {
            // 'N' / bold letter glyph
            blend_px(pb, cx - 1, cy - 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx - 1, cy, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx - 1, cy + 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx, cy, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx + 1, cy - 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx + 1, cy, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx + 1, cy + 1, INK_SHADOW, 0.88 * alpha);
        } else {
            // 'E' / bold letter glyph
            blend_px(pb, cx - 1, cy - 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx, cy - 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx + 1, cy - 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx - 1, cy, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx, cy, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx - 1, cy + 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx, cy + 1, INK_SHADOW, 0.88 * alpha);
            blend_px(pb, cx + 1, cy + 1, INK_SHADOW, 0.88 * alpha);
        }
    }
}

/// The recessed square movable type bed — rendered completely in oriented local `(u, v)` space!
fn draw_type_bed(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let bed_exposed = view.state == NightEditionState::BedLowered;
    let any_exposed = matches!(
        view.state,
        NightEditionState::BedLowered | NightEditionState::Ribbon
    );
    let pulse = 0.5 + 0.5 * (fr.anim * (if any_exposed { 9.0 } else { 2.6 })).sin();
    let half = BED_HALF;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, fr.pos, half.x.max(half.y) + 4.0) else {
        return;
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = sd_rbox(u, v, half.x, half.y, 1.0);
            if d > 1.0 {
                continue;
            }
            if d > 0.0 {
                // Machined inner chase lip; glows hot cyan-magenta when BedLowered!
                let lip_col = if bed_exposed {
                    mix(CYAN_HI, PAPER_SPEC, 0.35 + 0.35 * pulse)
                } else {
                    IRON_HI
                };
                blend_px(pb, x, y, lip_col, (if bed_exposed { 0.92 } else { 0.55 }) * (1.0 - d));
                continue;
            }
            // Dark recessed ink-stained chase well.
            let lit = fr.lit(u / half.x, v / half.y);
            let mut col = if bed_exposed {
                mix(INK_MID, mix(INK_FRESH, CYAN_MID, 0.35), 0.35 + 0.30 * pulse)
            } else {
                mix(INK_SHADOW, INK_MID, 0.22 + 0.14 * lit)
            };

            // Movable lead-alloy type slugs laid in newspaper columns (oriented in `(u, v)`!).
            let in_margin = u.abs() < half.x - 1.2 && v.abs() < half.y - 1.2;
            if in_margin {
                // Brass column divider rule right down the middle (`v ≈ 0`)!
                if v.abs() < 0.45 && u > -half.x + 4.0 {
                    col = shade3(BRASS_SHADOW, BRASS_MID, BRASS_HI, 0.65 + 0.30 * lit);
                } else if u < -half.x + 3.4 && u > -half.x + 1.4 {
                    // Bold raised lead headline slug bar across the top of the bed!
                    let seg = ((v * 1.4).floor() as i32) & 1 == 0;
                    if seg {
                        col = if bed_exposed {
                            mix(CYAN_HI, PAPER_SPEC, 0.45 + 0.35 * pulse)
                        } else {
                            shade3(LEAD_SH, LEAD_MID, LEAD_HI, 0.60 + 0.35 * lit)
                        };
                    }
                } else {
                    // Rows of lead type blocks!
                    let row_i = ((u + half.x) / 2.6).floor() as i32;
                    let col_i = ((v + half.y) / 2.2).floor() as i32;
                    let row_f = ((u + half.x) / 2.6).fract();
                    let col_f = ((v + half.y) / 2.2).fract();
                    if row_f > 0.22 && row_f < 0.80 && col_f > 0.18 && col_f < 0.84
                        && hash01(col_i + 3, row_i + 7) > 0.22
                    {
                        col = if bed_exposed {
                            mix(RIBBON_HI, CYAN_HI, 0.35 + 0.45 * pulse)
                        } else {
                            shade3(LEAD_SH, LEAD_MID, LEAD_HI, 0.50 + 0.40 * lit)
                        };
                    }
                }
            }

            blend_px(pb, x, y, apply_hit(col, fr.hit * 0.45), 1.0);
        }
    }

    // Properly oriented weak-point reticle & corner brackets when the bed is lowered (×1.5)!
    if bed_exposed {
        let rim = mix(SAFE_CYAN, PAPER_SPEC, 0.35 + 0.35 * pulse);
        for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = fr.place(su * (half.x + 2.2), sv * (half.y + 2.2));
            let along_u = corner - fr.f * (su * 4.2);
            let along_v = corner - fr.p * (sv * 4.2);
            draw_line_styled(pb, along_u, corner, rim, 0.95, 0, 0, 0);
            draw_line_styled(pb, along_v, corner, rim, 0.95, 0, 0, 0);
        }
        draw_ring_styled(
            pb,
            fr.pos,
            BED_GLOW_RADIUS * 0.72,
            rim,
            0.48 + 0.35 * pulse,
            3,
            fr.anim * 2.4,
        );
    } else if fr.damage_tier >= 3 {
        // Phase-3 fracture across the lead type bed glowing with fresh ink!
        let a = fr.place(-7.2, -5.2);
        let b = fr.place(7.2, 5.2);
        draw_line_styled(pb, a, b, INK_SHADOW, 0.90, 0, 0, 0);
        draw_line_styled(pb, a + fr.f, b + fr.f, mix(INK_FRESH, CYAN_HI, pulse), 0.75, 1, 3, 0);
    }
}

/// The clamped newsprint broadsheet — rendered cleanly in oriented local `(u, v)` space!
fn draw_sheet(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let (present, slide, flutter) = match view.state {
        NightEditionState::BedLowered | NightEditionState::Ribbon | NightEditionState::Wreck => {
            (false, 0.0, 0.0)
        }
        NightEditionState::PhaseTurn => (true, -7.5 * (1.0 - smoothstep(fr.prog)), 0.0),
        NightEditionState::PageTell | NightEditionState::MisprintTell => {
            (true, 1.2 * fr.prog, (fr.anim * 22.0).sin() * 0.6)
        }
        _ => (true, 0.0, 0.0),
    };
    if !present {
        return;
    }

    let su_c = -0.8 + slide;
    let hu: f32 = 6.2;
    let hv: f32 = 7.4;
    let center = fr.place(su_c, 0.0);
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, hu.max(hv) + 3.0) else {
        return;
    };
    let tattered = fr.damage_tier >= 3;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u_all, v) = fr.local(x, y);
            let u = u_all - su_c;
            // Serrated torn-off leading edge at `+hu` + slight anticipation flutter.
            let tooth = ((v * 1.8).sin() * 0.45) + flutter * ((u + hu) / (2.0 * hu)).clamp(0.0, 1.0);
            if u < -hu || u > hu + tooth || v.abs() > hv {
                continue;
            }
            // Phase 3: torn-off corner on the broadsheet!
            if tattered && u > 2.2 && v > 3.2 && (u + v) > 7.8 {
                continue;
            }
            let edge_d = (hu - u).min(u + hu).min(hv - v.abs());
            if edge_d < 0.55 {
                blend_px(pb, x, y, OUTLINE_INK, 0.65);
                continue;
            }
            let lit = fr.lit(u / hu, v / hv);
            let mut col = shade3(PAPER_SHADOW, PAPER_MID, PAPER_SPEC, 0.64 + 0.38 * lit);

            // Bold newspaper masthead banner near the clamped rear (`u ≈ -4.2`).
            if u > -4.8 && u < -3.3 && v.abs() < 5.6 {
                col = INK_SHADOW;
            } else if u > -2.5 && u < 4.5 && v.abs() < 5.6 {
                // Two tidy columns of printed newsprint text lines!
                let in_gutter = v.abs() < 0.75;
                let line_row = ((u + 2.5) * 1.15).fract() < 0.50;
                if !in_gutter && line_row && hash01((v * 2.0) as i32, (u * 2.0) as i32) > 0.18 {
                    col = mix(col, INK_MID, 0.68);
                }
            }
            // Tiny registration-cyan printer's mark in the bottom-right corner of the sheet!
            if (u - 4.8).abs() < 0.8 && (v + 5.2).abs() < 0.8 {
                col = CYAN_HI;
            }
            blend_px(pb, x, y, apply_hit(col, fr.hit * 0.35), 0.96);
        }
    }

    // Two gleaming brass paper-bail clamps holding the sheet down!
    for &side in &[-1.0f32, 1.0] {
        let clamp = fr.place(su_c - hu + 0.6, side * 5.2);
        fill_rect(pb, clamp, Vec2::splat(1.8), OUTLINE_INK, 0.82);
        fill_rect(pb, clamp, Vec2::splat(1.3), BRASS_MID, 0.96);
        blend_px(pb, clamp.x.round() as i32, clamp.y.round() as i32, BRASS_HI, 0.95);
    }
}

/// The brushed-steel carriage rail, brass end-caps, and sliding carriage clamp.
fn draw_carriage(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let rail_u = -BODY_HALF_LEN + 2.4;
    let rail_half = BODY_HALF_WIDTH + 2.8;
    let a = fr.place(rail_u, -rail_half);
    let b = fr.place(rail_u, rail_half);

    // Steel rail with dark outline & bright specular top edge!
    draw_line_styled(pb, a - fr.f, b - fr.f, OUTLINE_INK, 0.88, 0, 0, 0);
    draw_line_styled(pb, a, b, IRON_SPEC, 0.96, 0, 0, 0);
    draw_line_styled(pb, a + fr.f, b + fr.f, IRON_SHADOW, 0.82, 0, 0, 0);

    // Brass end-stop knobs at both ends of the rail.
    for &side in &[-1.0f32, 1.0] {
        let end = fr.place(rail_u, side * rail_half);
        fill_rect(pb, end, Vec2::splat(2.1), OUTLINE_INK, 0.85);
        fill_rect(pb, end, Vec2::splat(1.5), BRASS_MID, 0.96);
        blend_px(pb, (end.x - 0.5).round() as i32, (end.y - 0.5).round() as i32, BRASS_HI, 0.95);
    }

    let sweep = view.state == NightEditionState::CarriageSweep;
    let telling = view.state == NightEditionState::CarriageTell;
    let ribbon = view.state == NightEditionState::Ribbon;
    let slide = if sweep {
        0.0
    } else if ribbon {
        rail_half - 3.4
    } else if telling {
        (fr.prog * 8.0).sin() * (rail_half - 4.0)
    } else {
        (fr.anim * 1.4).sin() * (rail_half - 6.0) * 0.45
    };

    let clamp = fr.place(rail_u, slide);
    fill_rect(pb, clamp, Vec2::new(2.6, 3.0), OUTLINE_INK, 0.85);
    fill_rect(pb, clamp, Vec2::new(1.9, 2.3), IRON_HI, 0.98);
    let lamp = if sweep {
        DANGER_RED
    } else if telling {
        TELL_AMBER
    } else if ribbon {
        SAFE_CYAN
    } else {
        BRASS_HI
    };
    fill_rect(pb, clamp + fr.f * 1.1, Vec2::new(0.9, 1.6), lamp, 0.96);

    if sweep {
        for &side in &[-1.0f32, 1.0] {
            let p = fr.place(rail_u, side * rail_half);
            draw_ring_styled(pb, p, 3.4, DANGER_RED, 0.78, 0, fr.anim * 8.0);
        }
    }
}

/// Spoked brass-and-crimson ink-ribbon spools + the lifted silk ribbon weak point!
fn draw_ribbon(pb: &mut PixelBuffer, fr: &Frame, view: &NightEditionView) {
    let exposed = view.state == NightEditionState::Ribbon;
    let pulse = 0.5 + 0.5 * (fr.anim * 9.5).sin();

    // Two detailed spoked ink-ribbon spools at the rear corners of the platen!
    for &side in &[-1.0f32, 1.0] {
        let c = fr.place(-7.2, side * (BODY_HALF_WIDTH - 3.4));
        let Some((x0, x1, y0, y1)) = bbox_around(pb, c, 4.4) else {
            continue;
        };
        let spin = fr.anim * if exposed { 6.0 } else { 1.4 } * side;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - c.x;
                let dy = y as f32 - c.y;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 3.9 {
                    continue;
                }
                if d > 3.2 {
                    blend_px(pb, x, y, OUTLINE_INK, 0.82 * (1.0 - (d - 3.2) / 0.7));
                    continue;
                }
                let lit = (-dx * 0.55 - dy * 0.83) / 3.2;
                let col = if d > 2.45 {
                    // Metallic brass spool flange rim!
                    shade3(BRASS_SHADOW, BRASS_MID, BRASS_HI, 0.55 + 0.50 * lit)
                } else if d < 1.05 {
                    // Bright central brass spindle pin!
                    BRASS_HI
                } else {
                    // Wound crimson-magenta ink ribbon + rotating metallic spokes!
                    let ang = dy.atan2(dx) + spin;
                    if (ang * 3.0).sin().abs() > 0.84 {
                        IRON_SPEC
                    } else {
                        shade3(RIBBON_SHADOW, RIBBON_MID, RIBBON_HI, 0.52 + 0.50 * lit)
                    }
                };
                blend_px(pb, x, y, apply_hit(col, fr.hit * 0.35), 1.0);
            }
        }
    }

    // The ink ribbon band across the front face (`u = RIBBON_OFFSET` when exposed!).
    let u = if exposed { RIBBON_OFFSET } else { BODY_HALF_LEN - 3.6 };
    let half_w = BODY_HALF_WIDTH - 4.4;
    let a = fr.place(u, -half_w);
    let b = fr.place(u, half_w);
    if exposed {
        // Lifted wide billowing crimson-magenta silk ribbon with cyan-white sheen & dark outline!
        let col = mix(RIBBON_HI, SAFE_CYAN, 0.16 + 0.20 * pulse);
        draw_line_styled(pb, a - fr.f * 2.0, b - fr.f * 2.0, OUTLINE_INK, 0.85, 0, 0, 0);
        draw_line_styled(pb, a + fr.f * 2.0, b + fr.f * 2.0, OUTLINE_INK, 0.85, 0, 0, 0);
        draw_line_styled(pb, a - fr.f, b - fr.f, RIBBON_MID, 0.96, 0, 0, 0);
        draw_line_styled(pb, a, b, col, 0.98, 0, 0, 0);
        draw_line_styled(pb, a + fr.f, b + fr.f, mix(col, PAPER_SPEC, 0.35), 0.94, 0, 0, 0);
        // Traveling bright silk specular highlight running along the exposed ribbon!
        let glint = fr.place(u, (fr.anim * 4.5).sin() * (half_w - 2.0));
        fill_rect(pb, glint, Vec2::new(1.5, 2.2), (255, 246, 250), 0.96);
        // Crisp cyan weak-point side arrows pointing at the ribbon!
        for &side in &[-1.0f32, 1.0] {
            let p = fr.place(u, side * (half_w + 3.2));
            draw_line_styled(pb, p, p - fr.p * (side * 2.5), SAFE_CYAN, 0.92, 0, 0, 0);
        }
    } else {
        draw_line_styled(pb, a, b, RIBBON_SHADOW, 0.72, 0, 0, 0);
        draw_line_styled(pb, a + fr.f * 0.6, b + fr.f * 0.6, RIBBON_MID, 0.75, 0, 0, 0);
    }
}

/// Dark smoky wisps from overheated carriage bearings in Phase 2/3.
fn draw_damage_smoke(pb: &mut PixelBuffer, fr: &Frame) {
    let count = if fr.damage_tier >= 3 { 4 } else { 2 };
    let origin = fr.place(-8.0, 6.0);
    for i in 0..count {
        let t = (fr.anim * 0.9 + i as f32 / count as f32).fract();
        let p = origin + Vec2::new((t * 5.0 + i as f32).sin() * 2.2 + t * 3.0, -2.0 - t * 11.0);
        draw_smoke_puff(pb, p, 1.5 + t * 2.0, (1.0 - t) * 0.72);
    }
}

/// Crisp metallic & cyan registration sparks when struck!
fn draw_hit_sparks(pb: &mut PixelBuffer, fr: &Frame) {
    let seed = (fr.anim * 24.0) as i32;
    for k in 0..6 {
        let a = k as f32 / 6.0 * TAU + hash01(seed + k * 7, 19) * 0.5;
        let dir = Vec2::from_angle(a);
        let p0 = fr.pos + dir * 11.5;
        let p1 = fr.pos + dir * (15.0 + fr.hit * 4.5);
        let col = if k % 2 == 0 { PAPER_SPEC } else { CYAN_HI };
        draw_line_styled(pb, p0, p1, col, 0.90 * fr.hit, 0, 0, 0);
    }
}

/// Multi-stage death wreck: exploding broadsheets, scattered lead type blocks & smoke!
fn draw_wreck_body(pb: &mut PixelBuffer, fr: &Frame) {
    let t = fr.prog;
    // Initial crimson-magenta ink shockwave burst!
    if t < 0.36 {
        let k = 1.0 - t / 0.36;
        draw_ring_styled(pb, fr.pos, 9.0 + (1.0 - k) * 20.0, RIBBON_HI, 0.85 * k, 0, fr.anim);
        draw_ring_styled(pb, fr.pos, 6.0 + (1.0 - k) * 14.0, PAPER_SPEC, 0.65 * k, 2, fr.anim);
    }
    // Six printed newspaper broadsheets tearing loose and tumbling outward!
    for i in 0..6 {
        let a = i as f32 * TAU / 6.0 + 0.35;
        let k = smoothstep(((t - i as f32 * 0.04) / 0.78).clamp(0.0, 1.0));
        if k <= 0.01 {
            continue;
        }
        let lift = (k * std::f32::consts::PI).sin() * 6.0;
        let p = fr.pos + Vec2::from_angle(a) * (9.0 + 28.0 * k) + Vec2::new(0.0, -lift);
        draw_oriented_sheet(pb, p, a + k * 4.5, 4.2, 3.0, 0.90 * (1.0 - 0.45 * k));
    }
    // Gleaming metallic lead type blocks & brass springs scattering out of the split bed!
    for i in 0..10 {
        let a = i as f32 * 0.65 + 0.2;
        let k = ((t * 1.25) - i as f32 * 0.03).clamp(0.0, 1.0);
        if k <= 0.01 {
            continue;
        }
        let p = fr.pos + Vec2::from_angle(a) * (7.0 + 22.0 * k);
        let col = if i % 3 == 0 { BRASS_HI } else { LEAD_HI };
        fill_rect(pb, p, Vec2::splat(1.0), OUTLINE_INK, 0.75 * (1.0 - 0.4 * k));
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, col, 0.95 * (1.0 - 0.35 * k));
    }
    // Rising smoke column from the wrecked press!
    for i in 0..4 {
        let st = (t * 0.8 + i as f32 * 0.24).min(1.0);
        let p = fr.pos + Vec2::new(((i as f32) - 1.5) * 3.0 + st * 2.5, -2.0 - st * 12.0);
        draw_smoke_puff(pb, p, 2.2 + st * 2.4, (1.0 - st * 0.65) * 0.82);
    }
}

/// Clean radial segmented Boss HP arc around the platen (3-phase markers at 30% and 65%).
fn draw_hp_arc(pb: &mut PixelBuffer, fr: &Frame) {
    if fr.hp <= 0.0 {
        return;
    }
    let r = BODY_RADIUS + 6.5;
    let steps = 48;
    let lit = (fr.hp * steps as f32).round() as i32;
    let col = if fr.hp > 0.65 {
        CYAN_HI
    } else if fr.hp > 0.30 {
        TELL_AMBER
    } else {
        DANGER_RED
    };
    for i in 0..steps {
        let frac = i as f32 / steps as f32;
        let a = -std::f32::consts::FRAC_PI_2 + frac * TAU;
        let x = (fr.pos.x + a.cos() * r).round() as i32;
        let y = (fr.pos.y + a.sin() * r).round() as i32;
        if (frac - 0.30).abs() < 0.015 || (frac - 0.65).abs() < 0.015 {
            blend_px(pb, x, y, OUTLINE_INK, 0.90);
            continue;
        }
        darken_px(pb, x, y + 1, 0.38);
        if i < lit {
            let lead = i + 1 == lit;
            blend_px(pb, x, y, if lead { PAPER_SPEC } else { col }, if lead { 0.95 } else { 0.78 });
        } else {
            blend_px(pb, x, y, IRON_SHADOW, 0.45);
        }
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

    /// Gated behind `RATRUN_NIGHT_WORLD=<dir>`: renders every Night Edition
    /// state through the real `World::draw(Theme::Printworks)` stack with both
    /// `LightingMode::Full` and `LightingMode::Off`. Inert unless set.
    #[test]
    fn night_edition_world_gallery() {
        use crate::game::ecs::components::SpawnId;
        use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
        use crate::game::level::{MAP_COLS, MAP_ROWS, Theme, TileRect};
        use crate::game::world::World;
        use crate::render::lighting::LightingMode;

        let Ok(dir) = std::env::var("RATRUN_NIGHT_WORLD") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);

        let shots: &[(&str, NightEditionState, f32, u8, f32)] = &[
            ("w_01_roll", NightEditionState::Roll, 0.35, 1, 0.95),
            ("w_02_page_tell", NightEditionState::PageTell, 0.65, 1, 0.88),
            ("w_03_page_stamp", NightEditionState::PageStamp, 0.25, 1, 0.85),
            ("w_04_bed_lowered", NightEditionState::BedLowered, 0.45, 1, 0.78),
            ("w_05_carriage_tell", NightEditionState::CarriageTell, 0.60, 2, 0.52),
            ("w_06_carriage_sweep", NightEditionState::CarriageSweep, 0.55, 2, 0.48),
            ("w_07_ribbon", NightEditionState::Ribbon, 0.45, 2, 0.45),
            ("w_08_misprint_tell", NightEditionState::MisprintTell, 0.65, 3, 0.24),
            ("w_09_misprint_stamp", NightEditionState::MisprintStamp, 0.35, 3, 0.22),
            ("w_10_phase_turn", NightEditionState::PhaseTurn, 0.45, 2, 0.60),
            ("w_11_wreck", NightEditionState::Wreck, 0.58, 3, 0.0),
        ];

        for &(name, state, prog, phase, hp) in shots {
            let mut w = World::new(Theme::Printworks, 7);
            w.viewport = Vec2::new(176.0, 108.0);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            let anchor = w.player.pos;
            let pos = anchor - Vec2::new(26.0, 4.0);
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .unwrap_or(TileRect::new(0, 0, MAP_COLS, MAP_ROWS));
            let mut boss = Boss::new(BossKind::NightEdition, pos, 3000.0, arena, pos, false);
            boss.facing = Vec2::new(1.0, 0.15).normalized();
            boss.phase = phase;
            boss.hp = boss.max_hp * hp;
            boss.state = BossState::NightEdition(state);
            boss.duration = 1.4;
            boss.elapsed = 1.4 * prog;
            boss.anim = 2.1;
            boss.swipe_side = 1.0;
            boss.pounce_origin = pos - Vec2::new(6.0, 0.0);
            boss.target = match state {
                NightEditionState::CarriageTell
                | NightEditionState::CarriageSweep
                | NightEditionState::Ribbon => pos + Vec2::new(RULE_LEN - 6.0, 0.0),
                _ => anchor,
            };
            w.ecs.spawn((boss, SpawnId(9001)));

            let mut pb = PixelBuffer::new(176, 108);
            w.draw(&mut pb);
            w.elapsed = 30.08;
            w.draw(&mut pb);
            let _ = std::fs::write(dir.join(format!("{name}.ppm")), to_ppm(&pb));

            w.lighting = LightingMode::Off;
            w.smoke_enabled = false;
            w.draw(&mut pb);
            let _ = std::fs::write(dir.join(format!("{name}_off.ppm")), to_ppm(&pb));
        }
    }

    #[test]
    fn night_edition_audit_fixes_misprint_ribbon_and_nonfinite() {
        // 1) Non-finite / extreme coordinates and empty buffers must never panic or hang.
        let mut empty = PixelBuffer::new(0, 0);
        let mut bad = NightEditionView::roll(Vec2::new(60.0, 60.0), Vec2::new(1.0, 0.0), f32::NAN);
        bad.state = NightEditionState::CarriageTell;
        bad.rule_from = Vec2::new(f32::NAN, 0.0);
        bad.rule_to = Vec2::new(1e12, -1e12);
        bad.page = Vec2::new(1e9, 1e9);
        draw_ground(&mut empty, &bad, Vec2::ZERO, (120, 120, 140));
        draw(&mut empty, &bad, Vec2::ZERO, (120, 120, 140));
        let mut pb = PixelBuffer::new(96, 96);
        draw_ground(&mut pb, &bad, Vec2::ZERO, (120, 120, 140));
        draw(&mut pb, &bad, Vec2::ZERO, (120, 120, 140));

        // 2) MisprintStamp at prog=0.20 must NOT flash sheet 2 before beat 2 lands (`prog >= 0.55`).
        let mut early = PixelBuffer::new(120, 100);
        let mut late = PixelBuffer::new(120, 100);
        let mut mv = NightEditionView::roll(Vec2::new(40.0, 50.0), Vec2::new(1.0, 0.0), 1.0);
        mv.state = NightEditionState::MisprintStamp;
        mv.page = Vec2::new(64.0, 50.0);
        mv.margin_side = 1.0;
        mv.progress = 0.20;
        draw_misprint(&mut early, &mv, Vec2::ZERO, mv.progress, mv.anim);
        mv.progress = 0.58;
        draw_misprint(&mut late, &mv, Vec2::ZERO, mv.progress, mv.anim);
        // Sample inside sheet 2's exclusive band (right of sheet 1, left of safe margin).
        let rects = misprint_rects(mv.page, mv.side());
        let s2_only_x = (rects[1].0.x + rects[1].1.x - 2.0).round() as usize;
        let s2_y = rects[1].0.y.round() as usize;
        // At prog=0.58 sheet 2 flashes bright red; at prog=0.20 it should be much dimmer.
        assert!(
            late.pixel_at(s2_only_x, s2_y).0 > early.pixel_at(s2_only_x, s2_y).0 + 25,
            "sheet 2 should flash at beat 2 (prog=0.58), not early (prog=0.20): early={:?}, late={:?}",
            early.pixel_at(s2_only_x, s2_y),
            late.pixel_at(s2_only_x, s2_y),
        );

        // 3) Ribbon recovery must not paint TELL_AMBER danger carpet on the rule.
        let mut tell_pb = PixelBuffer::new(120, 80);
        let mut rib_pb = PixelBuffer::new(120, 80);
        let mut rv = NightEditionView::roll(Vec2::new(30.0, 40.0), Vec2::new(1.0, 0.0), 1.0);
        rv.rule_from = Vec2::new(20.0, 40.0);
        rv.rule_to = Vec2::new(74.0, 40.0);
        rv.state = NightEditionState::CarriageTell;
        let fr_t = Frame::from_view(&rv, Vec2::ZERO, (120, 120, 140));
        draw_rule(&mut tell_pb, &rv, Vec2::ZERO, &fr_t);
        rv.state = NightEditionState::Ribbon;
        let fr_r = Frame::from_view(&rv, Vec2::ZERO, (120, 120, 140));
        draw_rule(&mut rib_pb, &rv, Vec2::ZERO, &fr_r);
        // Midpoint of rule band: amber tell has noticeably higher red+green than subdued Ribbon track.
        let tp = tell_pb.pixel_at(47, 42);
        let rp = rib_pb.pixel_at(47, 42);
        assert!(
            (tp.0 as u16 + tp.1 as u16) > (rp.0 as u16 + rp.1 as u16) + 10,
            "Ribbon recovery should not draw bright amber danger fill: tell={tp:?}, ribbon={rp:?}"
        );
    }
}
