//! Printworks boss — THE NIGHT EDITION (the walking offset press).
//!
//! [`crate::game::art_boss_night_edition`] is the gameplay spec: its state
//! enum, every per-state duration, damage number, footprint helper
//! (`inked_rect`, `margin_rect`, `misprint_rects`, `rule_band`, `body_box`,
//! `weak_point`, `incoming_damage_mult`, `collision_radius`) and the view struct
//! are binding. This file only drives the transitions, chooses the page and the
//! carriage rule, and queues the art's telegraphed attacks as shared
//! [`BossHazard`]s; everything the rat sees is drawn from the art view through
//! [`Boss::view`].
//!
//! ## The fight (`WAVE3_DESIGN.md` §12)
//!
//! Phase 1 (`hp > 65 %`) is **page stamps**: the press outlines a 36 × 30 px
//! page at the rat with a 12 px blank margin, prints the inked 24 × 30 px for
//! `18`, then rests on its lowered type bed for `1.4 s` at ×1.5 damage.
//!
//! Phase 2 (`65..30 %`) alternates page stamps with **carriage returns**: a bell
//! and a dotted 54 × 8 px rule for `1 s`, then the whole press runs the rule at
//! `108 px/s` for `20`, and jams at its far end with the ink ribbon lifted for
//! `1.6 s` at ×1.5.
//!
//! Phase 3 (`≤ 30 %`) is the **misprint**: two sheets offset `12 px` under one
//! shared 12 px clear stripe, stamped `0.55 s` apart, which then chains into the
//! solo carriage and a `2 s` ribbon window.
//!
//! ## State machine (art timings; phases at 65 % / 30 %)
//!
//! | state | duration | telegraph → active | damage |
//! |-------|----------|--------------------|--------|
//! | `Roll` | 1.8 / 1.5 / 1.2 s | — | body contact 8 |
//! | `PageTell` | 1.1 s | page outline + cyan blank margin | 0 |
//! | `PageStamp` | 0.54 s | (tell) → inked 24 × 30 px for 0.30 s | 18 |
//! | `BedLowered` | 1.4 s | open bed, harmless | 0 (takes ×1.5) |
//! | `MisprintTell` | 1.2 s | two sheets, shared stripe | 0 |
//! | `MisprintStamp` | 1.11 s | (tell) → each sheet for 0.26 s, 0.55 s apart | 18 each |
//! | `CarriageTell` | 1.0 s | bell + dotted rule | 0 |
//! | `CarriageSweep` | 0.50 s | (tell) → the 8 px rule band | 20 |
//! | `Ribbon` | 1.6 / 2.0 s | the lifted ribbon at the stop, harmless | 0 (takes ×1.5) |
//! | `PhaseTurn` | 1.2 s | machinery only, harmless | 0 |
//! | `Wreck` | 2.8 s | the sheets tear loose | 0 |
//!
//! ## Suppression (the design's rule, and the arena's reality)
//!
//! A move is only started when its **stated safe strip is walkable** and the
//! target can see the tell:
//!
//! * a page needs the 12 px blank margin clear for the rat (`page_fits`) and the
//!   printed area free of walls, and the press has to have line of sight to the
//!   sheet;
//! * a misprint needs both sheets *and* the shared stripe clear;
//! * a carriage return needs a full [`art::RULE_LEN`] run of body-clear floor
//!   with the rat inside the 8 px band, and a side lane to cross out of it.
//!
//! When the wanted move does not fit, the press walks (`Roll`) — one rng draw
//! per decision, from the boss's own stream — and tries again from a fresh
//! angle. Nothing allocates: the plan structs are stack values and hazards go
//! into the boss's pre-reserved ring.
//!
//! ## Determinism
//!
//! Exactly two kinds of draw: one `f32` per `Roll` entry (which way it circles)
//! and one per attack decision (which side of the page stays blank). Both come
//! from `ctx.rng` (the boss's salted stream), so the run cannot perturb the main
//! rng order.
//!
//! ## Per-controller memory
//!
//! The art view reads `pos/facing/state/progress/anim/hit_flash/hp_frac/phase/
//! page/margin_side/rule_from/rule_to`; the shared `Boss` fields carry exactly
//! those (`target` = the page or the rule's far end, `pounce_origin` = the
//! rule's start, `swipe_side` = the blank margin's side). Decisions live in
//! `flags`:
//!
//! * bit 0 `WANT_CARRIAGE` — the phase-2 rotation's next move is the carriage;
//! * bit 1 `LONG_RIBBON` — the next ribbon window is the misprint's `2.0 s`;
//! * bit 2 `STAMP_A` / bit 3 `STAMP_B` — the misprint's two impressions played;
//! * bit 4 `CUED` — this beat already played its one-shot cue.
//!
//! `scratch[0]` is the circling direction.

use super::*;
use crate::game::art_boss_night_edition as art;

// ============================================================================
// Controller-side timings (the art owns the numbers)
// ============================================================================

/// The walking beat per phase (`1.8 / 1.5 / 1.2 s`).
const ROLL_DURATION: [f32; 3] = [1.8, 1.5, 1.2];
/// The mid range the press holds between attacks (`30..56 px`). The upper end
/// matters: the carriage rule is 54 px, so the rat has to be inside it.
const ROLL_BAND: [f32; 2] = [30.0, 56.0];
/// A page is only stamped on a rat at least this far away (a sheet under the
/// press is not a fight, it is a coin flip).
const PAGE_MIN_RANGE: f32 = 18.0;
/// The carriage rule needs the rat inside the lane, so it is only started in
/// this band.
const RULE_RANGE: [f32; 2] = [14.0, 46.0];
/// A crossing lane this wide has to be open beside the rat.
const CROSS_LANE: f32 = 16.0;

/// `PageStamp`'s state length (the impression plus its settle).
const STAMP_STATE: f32 = art::STAMP_ACTIVE + art::STAMP_SETTLE;
/// `MisprintStamp`'s state length: the gap to the second impression, that
/// impression's own window, then the settle.
const MISPRINT_STATE: f32 = art::MISPRINT_GAP + art::MISPRINT_ACTIVE + MISPRINT_SETTLE;
/// Space after the misprint's second impression before the carriage moves.
const MISPRINT_SETTLE: f32 = 0.30;
/// The carriage's run time over its own rule.
const SWEEP_TIME: f32 = art::RULE_LEN / art::SWEEP_SPEED;
/// The first impression's offset into `MisprintStamp` (the state starts a beat
/// before the type comes down).
const MISPRINT_FIRST: f32 = art::MISPRINT_GAP * 0.18;
/// Aim steering tried around the bearing to the rat, centre-out.
const AIM_OFFSETS: [f32; 5] = [0.0, 0.16, -0.16, 0.32, -0.32];

/// [`Boss::flags`] layout.
const F_WANT_CARRIAGE: u32 = 1 << 0;
const F_LONG_RIBBON: u32 = 1 << 1;
const F_STAMP_A: u32 = 1 << 2;
const F_STAMP_B: u32 = 1 << 3;
const F_CUED: u32 = 1 << 4;
/// Bits [`enter`] clears (everything state-local).
const F_STATE_LOCAL: u32 = F_STAMP_A | F_STAMP_B | F_CUED;

/// [`Boss::scratch`] layout.
const S_CIRCLE: usize = 0;

/// The Night Edition controller (zero-sized; state lives on [`Boss`]).
pub struct NightEditionController;

impl BossController for NightEditionController {
    fn kind(&self) -> BossKind {
        BossKind::NightEdition
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = state_of(boss);
        // Outside the arena the press walks the large-class flow field to the
        // rat (the framework's hunt); inside it, it fights.
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // A phase threshold is a set piece: the frame unlatches, the sheet is
        // stripped and fed again, and nothing can hurt the rat for 1.2 s. Any
        // telegraph it was in the middle of is cancelled — the rat never sees
        // an impression come down from a pose the press has left.
        if out.phase_changed && !matches!(boss.state, BossState::NightEdition(NightEditionState::Wreck))
        {
            boss.hazards.clear();
            enter(boss, NightEditionState::PhaseTurn, art::PHASE_TURN);
        }

        match boss.state {
            BossState::NightEdition(NightEditionState::Roll) => tick_roll(ctx, boss),
            BossState::NightEdition(NightEditionState::PageTell) => tick_page_tell(ctx, boss),
            BossState::NightEdition(NightEditionState::PageStamp) => {
                if boss.progress() >= 1.0 {
                    enter(boss, NightEditionState::BedLowered, art::BED_LOWERED);
                }
            }
            BossState::NightEdition(NightEditionState::BedLowered) => {
                if boss.progress() >= 1.0 {
                    enter_roll(ctx, boss);
                }
            }
            BossState::NightEdition(NightEditionState::MisprintTell) => {
                tick_misprint_tell(ctx, boss)
            }
            BossState::NightEdition(NightEditionState::MisprintStamp) => {
                tick_misprint_stamp(ctx, boss, out)
            }
            BossState::NightEdition(NightEditionState::CarriageTell) => {
                tick_carriage_tell(ctx, boss)
            }
            BossState::NightEdition(NightEditionState::CarriageSweep) => {
                tick_carriage_sweep(ctx, boss)
            }
            BossState::NightEdition(NightEditionState::Ribbon) => {
                if boss.progress() >= 1.0 {
                    enter_roll(ctx, boss);
                }
            }
            BossState::NightEdition(NightEditionState::PhaseTurn) => {
                if boss.progress() >= 1.0 {
                    enter_roll(ctx, boss);
                }
            }
            BossState::NightEdition(NightEditionState::Wreck)
            | BossState::Cat(_)
            | BossState::Broom(_)
            | BossState::Mower(_)
            | BossState::Mangle(_)
            | BossState::HorseshoeCrab(_)
            | BossState::Magpie(_)
            | BossState::MegaRat(_)
            | BossState::Hawk(_)
            | BossState::Cobra(_)
            | BossState::Lantern(_) => {}
        }

        // The art owns the multiplier and the recovery states' contact rule;
        // both are applied after the transitions so a window is live on the
        // very tick the press opens it.
        let settled = state_of(boss);
        boss.damage_mult = art::incoming_damage_mult(settled);
        if !art::state_contacts(settled) {
            boss.no_contact();
        }

        // One cue per state entry, on the tick of the transition.
        let after = state_of(boss);
        let cue = if after != before {
            state_cue(after)
        } else {
            None
        };
        if let Some(cue) = cue {
            out.cue(cue);
        }
    }

    /// The wreck: the press stops printing the moment its frame goes down.
    /// Every hazard it had queued dies with it (the design's "all hazards
    /// belong to the boss and stop on its death"), then the sheets tear loose.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        boss.hazards.clear();
        if boss.flags & F_CUED == 0 {
            boss.flags |= F_CUED;
            out.cue(BossCue::NightWreck);
        }
    }
}

/// The sound/effect cue for entering a state.
#[inline]
fn state_cue(state: NightEditionState) -> Option<BossCue> {
    match state {
        NightEditionState::PageTell => Some(BossCue::NightPage),
        NightEditionState::PageStamp => Some(BossCue::NightStamp),
        NightEditionState::BedLowered => Some(BossCue::NightBed),
        NightEditionState::MisprintTell => Some(BossCue::NightMisprint),
        NightEditionState::CarriageTell => Some(BossCue::NightBell),
        NightEditionState::CarriageSweep => Some(BossCue::NightCarriage),
        NightEditionState::Ribbon => Some(BossCue::NightRibbon),
        NightEditionState::PhaseTurn => Some(BossCue::NightTurn),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

#[inline]
fn phase_slot(boss: &Boss) -> usize {
    (boss.phase.clamp(1, 3) - 1) as usize
}

#[inline]
fn state_of(boss: &Boss) -> NightEditionState {
    match boss.state {
        BossState::NightEdition(s) => s,
        _ => NightEditionState::Roll,
    }
}

/// Enter an art state, clearing the per-state latches.
#[inline]
fn enter(boss: &mut Boss, state: NightEditionState, duration: f32) {
    boss.set_state(BossState::NightEdition(state), duration);
    boss.flags &= !F_STATE_LOCAL;
}

/// The neutral beat: hold mid range, circling one way for the whole beat.
fn tick_roll(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    let dir = if dist < ROLL_BAND[0] {
        -boss.facing
    } else if dist > ROLL_BAND[1] {
        // The straight bearing when the rat is in the open, the large-class
        // flow field when a wall is between (the framework's shared steering).
        ctx.seek(boss)
    } else {
        boss.facing.perp() * circle_sign(boss)
    };
    move_boss(ctx, boss, dir, art::WALK_SPEED[slot]);
    // While walking, the art's aim point is the rat (the press leans toward
    // what it is about to print).
    boss.target = ctx.player_pos;
    if boss.progress() >= 1.0 {
        choose_next(ctx, boss);
    }
}

/// Creep toward the sheet while the tell runs. The page is already locked: the
/// whole point of the telegraph is that it does not follow the rat.
fn tick_page_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    move_boss(ctx, boss, boss.facing, art::TELL_CREEP);
    if boss.progress() >= 1.0 {
        enter(boss, NightEditionState::PageStamp, STAMP_STATE);
    }
}

fn tick_misprint_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    move_boss(ctx, boss, boss.facing, art::TELL_CREEP * 0.6);
    if boss.progress() >= 1.0 {
        enter(boss, NightEditionState::MisprintStamp, MISPRINT_STATE);
        boss.flags |= F_LONG_RIBBON;
    }
}

fn tick_misprint_stamp(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let t = boss.progress() * boss.duration;
    if boss.flags & F_STAMP_A == 0 && t >= MISPRINT_FIRST {
        boss.flags |= F_STAMP_A;
        out.cue(BossCue::NightStamp);
    }
    if boss.flags & F_STAMP_B == 0 && t >= MISPRINT_FIRST + art::MISPRINT_GAP {
        boss.flags |= F_STAMP_B;
        out.cue(BossCue::NightStamp);
    }
    if boss.progress() >= 1.0 {
        // The misprint always chains into the solo carriage: the sheet is
        // ruined and the carriage has to re-register.
        if begin_carriage(ctx, boss) {
            return;
        }
        // No rule fits from here, so there is no carriage for the long ribbon
        // to follow: drop the request rather than leave it for a later sweep.
        boss.flags &= !F_LONG_RIBBON;
        enter_roll(ctx, boss);
    }
}

fn tick_carriage_tell(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The rule is locked the moment the bell rings: the press keeps the *lane
    // it ruled*, it does not re-aim at the rat. Re-aiming here would walk the
    // body off the dotted line the player is reading (and onto a rat that had
    // crossed the rule early, which is the design's whole dodge).
    if boss.progress() >= 1.0 {
        enter(boss, NightEditionState::CarriageSweep, SWEEP_TIME);
    }
}

fn tick_carriage_sweep(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let moved = move_boss(ctx, boss, boss.facing, art::SWEEP_SPEED);
    if !moved || boss.progress() >= 1.0 {
        // Jammed at the end of the rule (or against a wall): the ribbon comes
        // up, and that is the opening.
        // A carriage that jams early stops being a ram: the rule's corridor was
        // queued live for the whole `SWEEP_TIME`, and without this it kept
        // hurting along its full length while the press sat jammed with its
        // ribbon up — the punish window the rat is supposed to walk into.
        boss.hazards
            .retain(|h| !matches!(h.shape, HazardShape::Corridor { .. }));
        let long = boss.flags & F_LONG_RIBBON != 0;
        boss.flags &= !F_LONG_RIBBON;
        let duration = if long { art::RIBBON_LONG } else { art::RIBBON };
        enter(boss, NightEditionState::Ribbon, duration);
    }
}

// ============================================================================
// Decisions
// ============================================================================

fn enter_roll(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    enter(boss, NightEditionState::Roll, ROLL_DURATION[slot]);
    boss.scratch[S_CIRCLE] = if ctx.rng.random::<f32>() < 0.5 {
        1.0
    } else {
        -1.0
    };
}

#[inline]
fn circle_sign(boss: &Boss) -> f32 {
    if boss.scratch[S_CIRCLE] >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Pick the next move for the current phase (with the design's suppression).
fn choose_next(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    match boss.phase.clamp(1, 3) {
        1 => {
            if begin_page(ctx, boss) {
                return;
            }
        }
        2 => {
            // Alternation: the move that is due first, then the other one as
            // the fallback. Whichever commits flips the flag for next time.
            let want_carriage = boss.flags & F_WANT_CARRIAGE != 0;
            let mut took;
            if want_carriage {
                took = begin_carriage(ctx, boss);
                if !took {
                    took = begin_page(ctx, boss);
                }
            } else {
                took = begin_page(ctx, boss);
                if !took {
                    took = begin_carriage(ctx, boss);
                }
            }
            if took {
                return;
            }
        }
        _ => {
            if begin_misprint(ctx, boss) || begin_page(ctx, boss) {
                return;
            }
        }
    }
    // Nothing fits from here: walk to a fresh angle and ask again.
    enter_roll(ctx, boss);
}

/// A chosen sheet: where the page lies and which side stays blank.
#[derive(Clone, Copy, Debug)]
struct PagePlan {
    page: Vec2,
    side: f32,
}

/// The chosen carriage rule.
#[derive(Clone, Copy, Debug)]
struct RulePlan {
    from: Vec2,
    to: Vec2,
    dir: Vec2,
}

fn begin_page(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(plan) = plan_page(ctx, boss) else {
        return false;
    };
    boss.target = plan.page;
    boss.swipe_side = plan.side;
    // The rotation's next phase-2 move is the carriage.
    boss.flags |= F_WANT_CARRIAGE;
    enter(boss, NightEditionState::PageTell, art::STAMP_TELL);
    let (ink, half) = art::inked_rect(plan.page, plan.side);
    let impression = BossHazard::new(
        HazardShape::Rect {
            center: ink,
            facing: Vec2::new(1.0, 0.0),
            half: half.x,
            half_perp: half.y,
        },
        art::STAMP_TELL,
        art::STAMP_ACTIVE,
        art::STAMP_DAMAGE,
    )
    .with_knockback(90.0)
    .with_enemies();
    let _ = boss.push_hazard(impression);
    true
}

fn begin_misprint(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(plan) = plan_misprint(ctx, boss) else {
        return false;
    };
    boss.target = plan.page;
    boss.swipe_side = plan.side;
    boss.flags |= F_WANT_CARRIAGE;
    enter(boss, NightEditionState::MisprintTell, art::MISPRINT_TELL);
    let rects = art::misprint_rects(plan.page, plan.side);
    for (i, (ink, half)) in rects.iter().enumerate() {
        let warn = art::MISPRINT_TELL
            + if i == 0 {
                MISPRINT_FIRST
            } else {
                MISPRINT_FIRST + art::MISPRINT_GAP
            };
        let impression = BossHazard::new(
            HazardShape::Rect {
                center: *ink,
                facing: Vec2::new(1.0, 0.0),
                half: half.x,
                half_perp: half.y,
            },
            warn,
            art::MISPRINT_ACTIVE,
            art::STAMP_DAMAGE,
        )
        .with_knockback(90.0)
        .with_enemies();
        let _ = boss.push_hazard(impression);
    }
    true
}

fn begin_carriage(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(plan) = plan_rule(ctx, boss) else {
        return false;
    };
    boss.pounce_origin = plan.from;
    boss.target = plan.to;
    boss.facing = plan.dir;
    boss.flags &= !F_WANT_CARRIAGE;
    enter(boss, NightEditionState::CarriageTell, art::CARRIAGE_TELL);
    let ram = BossHazard::new(
        HazardShape::Corridor {
            from: plan.from,
            to: plan.to,
            half_width: art::RULE_HALF_WIDTH,
        },
        art::CARRIAGE_TELL,
        SWEEP_TIME,
        art::SWEEP_DAMAGE,
    )
    .with_knockback(140.0)
    .with_enemies();
    let _ = boss.push_hazard(ram);
    true
}

// ============================================================================
// Fit tests (the design's suppression rules)
// ============================================================================

/// The page plan: one draw picks the blank side, then the walkable side wins.
///
/// The move is suppressed unless the press can see the sheet, the rat is at
/// range, the 12 px blank margin is walkable **and** the printed area is free
/// of walls — a page whose safe strip is inside masonry is not a fair tell.
fn plan_page(ctx: &mut BossContext<'_>, boss: &Boss) -> Option<PagePlan> {
    let to = ctx.player_pos - boss.pos;
    if to.len() < PAGE_MIN_RANGE {
        return None;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, ctx.player_radius) {
        return None;
    }
    let page = ctx.player_pos;
    let first = if ctx.rng.random::<f32>() < 0.5 {
        1.0
    } else {
        -1.0
    };
    for side in [first, -first] {
        if page_fits(ctx.level, page, side, ctx.player_radius) {
            return Some(PagePlan { page, side });
        }
    }
    None
}

fn plan_misprint(ctx: &mut BossContext<'_>, boss: &Boss) -> Option<PagePlan> {
    let plan = plan_page(ctx, boss)?;
    // The printed union is a whole page wide, shifted against the margin: it
    // has to be floor too, or the failure reads as the press printing into a
    // wall.
    let union_centre = plan.page - Vec2::new(plan.side * art::MISPRINT_OFFSET * 0.5, 0.0);
    let union = Rectf::new(
        union_centre.x - art::PAGE_HALF.x,
        union_centre.y - art::PAGE_HALF.y,
        art::PAGE_HALF.x * 2.0,
        art::PAGE_HALF.y * 2.0,
    );
    if !ctx.level.tiles.rect_clear(union, SizeClass::Small) {
        return None;
    }
    Some(plan)
}

/// Is a page at `page` with its blank margin on `side` a fair stamp?
fn page_fits(level: &Level, page: Vec2, side: f32, radius: f32) -> bool {
    // The printed area: no wall inside it.
    let (ink, ink_half) = art::inked_rect(page, side);
    let ink_rect = Rectf::new(
        ink.x - ink_half.x,
        ink.y - ink_half.y,
        ink_half.x * 2.0,
        ink_half.y * 2.0,
    );
    if !level.tiles.rect_clear(ink_rect, SizeClass::Small) {
        return false;
    }
    // The blank margin: the rat has to be able to stand anywhere on it, so the
    // whole strip is walked with the player's own radius.
    let (margin, margin_half) = art::margin_rect(page, side);
    let strip = Rectf::new(
        margin.x - margin_half.x,
        margin.y - margin_half.y,
        margin_half.x * 2.0,
        margin_half.y * 2.0,
    );
    if !level.tiles.rect_clear(strip, SizeClass::Small) {
        return false;
    }
    let top = Vec2::new(margin.x, margin.y - margin_half.y + 1.0);
    let bottom = Vec2::new(margin.x, margin.y + margin_half.y - 1.0);
    level.tiles.segment_hit(top, bottom, radius).is_none()
}

/// The carriage plan: a full [`art::RULE_LEN`] run from the press through the
/// rat, ending on floor it can stand on, with a lane the rat can cross out of.
fn plan_rule(ctx: &BossContext<'_>, boss: &Boss) -> Option<RulePlan> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if !(RULE_RANGE[0]..=RULE_RANGE[1]).contains(&dist) {
        return None;
    }
    let base = to.normalized();
    if base.len_sq() <= 0.0 {
        return None;
    }
    for offset in AIM_OFFSETS {
        let dir = base.rotate(offset);
        let from = boss.pos;
        let to_end = from + dir * art::RULE_LEN;
        // The run has to fit: floor for the whole lane and a stand at the end.
        if !ctx.los_clear(from, to_end, boss.kind.body_radius()) {
            continue;
        }
        if !ctx.footprint_fits(to_end, boss.kind.body_radius()) {
            continue;
        }
        // The rat has to be *in* the band (the tell is aimed where it stands).
        let rel = ctx.player_pos - from;
        let along = rel.dot(dir);
        let across = rel.dot(dir.perp()).abs();
        if along <= 0.0 || across > art::RULE_HALF_WIDTH + ctx.player_radius {
            continue;
        }
        // ... and it has to be able to cross the 8 px band early.
        let proj = from + dir * along;
        let side = dir.perp();
        let cross = side * CROSS_LANE;
        if !ctx.los_clear(proj, proj + cross, ctx.player_radius)
            && !ctx.los_clear(proj, proj - cross, ctx.player_radius)
        {
            continue;
        }
        return Some(RulePlan {
            from,
            to: to_end,
            dir,
        });
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    //! Narrow tests for THE NIGHT EDITION.
    //!
    //! Every test drives the shared boss system through `begin_tick` →
    //! `run_system_cached` → `end_tick`, so hazards, body contact and the death
    //! hand-off are the real ones. The only shim is the rat's i-frame clock
    //! (the player system does not run here).

    use super::*;
    use crate::game::collision::{TileKind, TILE_F};
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    /// A controlled press room (cells) carved inside the real boss arena.
    const ROOM: (i32, i32) = (12, 10);

    // ---- harness ---------------------------------------------------------

    fn printworks(seed: u64) -> World {
        let mut w = World::new(Theme::Printworks, seed);
        // Campaign slot 2 is the Printworks, i.e. the Night Edition.
        w.level_index = 2;
        w
    }

    fn boss_of(w: &mut World) -> Option<Boss> {
        let mut q = w.ecs.query::<&Boss>();
        q.iter(&w.ecs).next().cloned()
    }

    fn with_boss(w: &mut World, mut f: impl FnMut(&mut Boss)) {
        let mut q = w.ecs.query::<&mut Boss>();
        for mut b in q.iter_mut(&mut w.ecs) {
            f(&mut b);
        }
    }

    fn spawn_press(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the Night Edition");
        assert_eq!(boss.kind, BossKind::NightEdition);
        boss
    }

    /// One tick of the shared boss system plus the rat's i-frame decay.
    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_update_system)
            .expect("boss update system must run");
        w.end_tick();
        w.player.iframes = (w.player.iframes - DT).max(0.0);
    }

    fn tick_n(w: &mut World, n: usize) {
        for _ in 0..n {
            tick(w);
        }
    }

    fn boss_state(w: &mut World) -> NightEditionState {
        state_of(&boss_of(w).expect("boss alive"))
    }

    fn boss_hp(w: &mut World) -> f32 {
        boss_of(w).expect("boss alive").hp
    }

    fn set_state(w: &mut World, state: NightEditionState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::NightEdition(state), duration);
            b.flags = 0;
            b.hurt = 0.0;
        });
    }

    /// Carve a sealed room into the boss arena; returns the interior rect.
    fn carve_room(w: &mut World, cells: (i32, i32)) -> TileRect {
        let arena = w.level.fight_rect_around(w.player.pos);
        let x = arena.x + 2;
        let y = arena.y + 2;
        w.level.tiles.fill_cells(
            x - 1,
            y - 1,
            cells.0 + 2,
            cells.1 + 2,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.fill_cells(x, y, cells.0, cells.1, TileKind::Open);
        w.level.tiles.rebuild_masks();
        TileRect::new(x, y, cells.0, cells.1)
    }

    /// A press in a carved room, facing east, with the rat `dist` px east.
    fn press_room(seed: u64, phase: u8, dist: f32) -> (World, TileRect) {
        let mut w = printworks(seed);
        spawn_press(&mut w);
        let room = carve_room(&mut w, ROOM);
        let pos = room.center_world();
        with_boss(&mut w, |b| {
            b.pos = pos;
            b.phase = phase;
            b.facing = Vec2::new(1.0, 0.0);
            b.target = pos;
            b.pounce_origin = pos;
            b.state = BossState::NightEdition(NightEditionState::Roll);
            b.duration = 1.0;
            b.elapsed = 0.99; // the walk beat ends on the next tick
        });
        w.player.pos = pos + Vec2::new(dist, 0.0);
        (w, room)
    }

    /// Drive the boss until its state changes; returns the seconds it lasted.
    fn drive_until_change(w: &mut World, limit: f32) -> (f32, NightEditionState) {
        let start = boss_state(w);
        let mut t = 0.0;
        while t < limit {
            tick(w);
            t += DT;
            let now = boss_state(w);
            if now != start {
                return (t, now);
            }
        }
        panic!("{start:?} never changed within {limit:.2} s");
    }

    /// Tick until the boss reaches `want` (with a wall-clock budget).
    fn drive_until(w: &mut World, want: NightEditionState, limit: f32) -> f32 {
        let mut t = 0.0;
        while t < limit {
            tick(w);
            t += DT;
            if boss_state(w) == want {
                return t;
            }
        }
        panic!("never reached {want:?} within {limit:.2} s");
    }

    // ---- geometry and timing --------------------------------------------

    #[test]
    fn the_page_stamp_marks_the_inked_rect_and_hits_on_the_beat() {
        let (mut w, _room) = press_room(11, 1, 34.0);
        let (_, state) = drive_until_change(&mut w, 1.0);
        assert_eq!(state, NightEditionState::PageTell);

        let boss = boss_of(&mut w).expect("boss alive");
        assert!(
            (boss.duration - art::STAMP_TELL).abs() < 1e-6,
            "the page tell is the art's 1.1 s, got {}",
            boss.duration
        );
        assert_eq!(boss.hazards.len(), 1, "one impression per sheet");
        let hz = boss.hazards[0];
        let (ink, ink_half) = art::inked_rect(boss.target, boss.swipe_side);
        match hz.shape {
            HazardShape::Rect {
                center,
                facing,
                half,
                half_perp,
            } => {
                assert!(center.dist(ink) < 1e-4, "the hazard is the inked part");
                assert!((half - ink_half.x).abs() < 1e-4);
                assert!((half_perp - ink_half.y).abs() < 1e-4);
                assert!((facing - Vec2::new(1.0, 0.0)).len() < 1e-4);
            }
            other => panic!("the impression must be a rect, got {other:?}"),
        }
        assert!((hz.warn - art::STAMP_TELL).abs() < 1e-6);
        assert!((hz.active - art::STAMP_ACTIVE).abs() < 1e-6);
        assert!((hz.damage - art::STAMP_DAMAGE).abs() < 1e-6);

        // The rat stands on the printed part: it is hit exactly once, and not
        // before the ink comes down.
        w.player.pos = ink;
        let before = w.player.hp;
        tick_n(&mut w, (art::STAMP_TELL / DT) as usize - 4);
        assert!(
            (w.player.hp - before).abs() < 1e-4,
            "the tell must never hurt ({} -> {})",
            before,
            w.player.hp
        );
        tick_n(&mut w, 12);
        let lost = before - w.player.hp;
        assert!(
            (lost - art::STAMP_DAMAGE).abs() < 1e-3,
            "the impression deals {} exactly, lost {lost}",
            art::STAMP_DAMAGE
        );
        // ... and only once inside the same window.
        tick_n(&mut w, 6);
        assert!((before - w.player.hp - art::STAMP_DAMAGE).abs() < 1e-3);
    }

    #[test]
    fn the_blank_margin_is_the_safe_strip() {
        // The margin has to face away from the press: the press creeps toward
        // the rat for the whole tell, and a margin *under* it is a body bump
        // (8 contact) rather than an unfair impression. The blank side is the
        // decision's rng draw, so pick a seed whose first sheet is that way
        // round — deterministic, and it keeps the test about the stamp.
        let mut w = None;
        for seed in 12..24u64 {
            let (mut candidate, _room) = press_room(seed, 1, 34.0);
            let (_, state) = drive_until_change(&mut candidate, 1.0);
            assert_eq!(state, NightEditionState::PageTell);
            if boss_of(&mut candidate).expect("boss alive").swipe_side > 0.0 {
                w = Some(candidate);
                break;
            }
        }
        let mut w = w.expect("one of the seeds leaves the margin away from the press");
        let boss = boss_of(&mut w).expect("boss alive");
        let (margin, _) = art::margin_rect(boss.target, boss.swipe_side);
        w.player.pos = margin;
        let before = w.player.hp;
        // The whole tell, the impression and the recovery.
        tick_n(&mut w, ((art::STAMP_TELL + art::STAMP_ACTIVE + 0.2) / DT) as usize);
        assert!(
            (w.player.hp - before).abs() < 1e-3,
            "the blank margin must be safe ({} -> {})",
            before,
            w.player.hp
        );
    }

    #[test]
    fn the_bed_lowered_recovery_is_the_punish_window() {
        let (mut w, _room) = press_room(13, 1, 34.0);
        drive_until(&mut w, NightEditionState::PageTell, 2.0);
        drive_until(&mut w, NightEditionState::PageStamp, 2.0);
        let t = drive_until(&mut w, NightEditionState::BedLowered, 2.0);
        assert!(
            (t - art::STAMP_ACTIVE - art::STAMP_SETTLE).abs() < 2.0 * DT,
            "the impression lasts {:.2} s, ended after {t:.2}",
            art::STAMP_ACTIVE + art::STAMP_SETTLE
        );
        let boss = boss_of(&mut w).expect("boss alive");
        assert!((boss.duration - art::BED_LOWERED).abs() < 1e-6);
        assert!(
            (boss.damage_mult - art::WEAK_MULT).abs() < 1e-6,
            "the lowered bed takes x1.5"
        );
        assert_eq!(boss.contact_damage, 0.0, "recovery is harmless to touch");
        assert!(art::weak_point(&boss.night_edition_view()).is_some());
    }

    #[test]
    fn the_carriage_rule_runs_fifty_four_px_through_the_rat_for_twenty() {
        let (mut w, _room) = press_room(21, 2, 30.0);
        // Phase 2 alternates: walk until the carriage is chosen.
        let t = drive_until(&mut w, NightEditionState::CarriageTell, 12.0);
        assert!(t > 0.0);
        let boss = boss_of(&mut w).expect("boss alive");
        assert!((boss.duration - art::CARRIAGE_TELL).abs() < 1e-6);
        assert_eq!(boss.hazards.len(), 1);
        let hz = boss.hazards[0];
        let (from, to, half_width) = match hz.shape {
            HazardShape::Corridor {
                from,
                to,
                half_width,
            } => (from, to, half_width),
            other => panic!("the carriage must be a corridor, got {other:?}"),
        };
        assert!((half_width - art::RULE_HALF_WIDTH).abs() < 1e-6);
        assert!(
            ((to - from).len() - art::RULE_LEN).abs() < 0.5,
            "the rule is 54 px, got {}",
            (to - from).len()
        );
        assert!((hz.damage - art::SWEEP_DAMAGE).abs() < 1e-6);
        assert!((hz.warn - art::CARRIAGE_TELL).abs() < 1e-6);
        assert!((hz.active - SWEEP_TIME).abs() < 1e-6);

        // The rat stands in the middle of the band when the carriage runs.
        let mid = from + (to - from) * 0.4;
        w.player.pos = mid;
        let before = w.player.hp;
        // Hold it there through the tell and the sweep.
        let steps = ((art::CARRIAGE_TELL + SWEEP_TIME + 0.1) / DT) as usize;
        for _ in 0..steps {
            w.player.pos = mid;
            tick(&mut w);
        }
        let lost = before - w.player.hp;
        assert!(
            (lost - art::SWEEP_DAMAGE).abs() < 1e-3,
            "the carriage deals {} once, lost {lost}",
            art::SWEEP_DAMAGE
        );
    }

    #[test]
    fn a_crossed_rule_is_safe_and_the_stop_opens_the_ribbon() {
        let (mut w, _room) = press_room(22, 2, 30.0);
        drive_until(&mut w, NightEditionState::CarriageTell, 12.0);
        let boss = boss_of(&mut w).expect("boss alive");
        let (from, to, _) = match boss.hazards[0].shape {
            HazardShape::Corridor {
                from,
                to,
                half_width,
            } => (from, to, half_width),
            other => panic!("expected a corridor, got {other:?}"),
        };
        let dir = (to - from).normalized();
        // Crossed early: well clear of the band *and* the press's own body, which
        // follows the rule at a 13 px half-width.
        let safe = from + dir * 20.0 + dir.perp() * 26.0;
        let before = w.player.hp;
        let steps = ((art::CARRIAGE_TELL + SWEEP_TIME + 0.1) / DT) as usize;
        for _ in 0..steps {
            w.player.pos = safe;
            tick(&mut w);
        }
        assert!((w.player.hp - before).abs() < 1e-3, "off the rule is safe");

        // The press jams at the stop: the ribbon comes up, harmless and soft.
        let t = drive_until(&mut w, NightEditionState::Ribbon, 2.0);
        assert!(t > 0.0);
        let boss = boss_of(&mut w).expect("boss alive");
        assert!((boss.duration - art::RIBBON).abs() < 1e-6);
        assert!((boss.damage_mult - art::WEAK_MULT).abs() < 1e-6);
        assert_eq!(boss.contact_damage, 0.0);
        let ribbon = art::ribbon_pos(boss.pos, boss.facing);
        assert!((ribbon.dist(boss.pos) - art::RIBBON_OFFSET).abs() < 1e-3);
    }

    #[test]
    fn the_misprint_stamps_two_sheets_apart_with_one_shared_clear_stripe() {
        let (mut w, _room) = press_room(31, 3, 34.0);
        drive_until(&mut w, NightEditionState::MisprintTell, 3.0);
        let boss = boss_of(&mut w).expect("boss alive");
        assert!((boss.duration - art::MISPRINT_TELL).abs() < 1e-6);
        assert_eq!(boss.hazards.len(), 2, "two impressions, one per sheet");
        let a = boss.hazards[0];
        let b = boss.hazards[1];
        assert!(
            (b.warn - a.warn - art::MISPRINT_GAP).abs() < 1e-6,
            "the sheets land 0.55 s apart ({} vs {})",
            a.warn,
            b.warn
        );
        assert!((a.damage - art::STAMP_DAMAGE).abs() < 1e-6);
        assert!((a.active - art::MISPRINT_ACTIVE).abs() < 1e-6);

        // The shared stripe is clear in both, and it is where the rat survives.
        let (margin, margin_half) = art::margin_rect(boss.target, boss.swipe_side);
        for hz in [a, b] {
            assert!(!hz.shape.contains(margin, 0.0), "the stripe is clear in both");
            assert!(!hz.shape.contains(margin, PLAYER_RADIUS));
        }
        assert!(margin_half.x > 0.0);
        w.player.pos = margin;
        let before = w.player.hp;
        tick_n(&mut w, ((art::MISPRINT_TELL + art::MISPRINT_GAP + 0.6) / DT) as usize);
        assert!(
            (w.player.hp - before).abs() < 1e-3,
            "the unprinted stripe survives both beats"
        );

        // ... then the solo carriage, then the long ribbon window.
        let boss = boss_of(&mut w).expect("boss alive");
        let mut state = state_of(&boss);
        let mut t = 0.0;
        while state != NightEditionState::Ribbon && t < 6.0 {
            tick(&mut w);
            t += DT;
            state = boss_state(&mut w);
        }
        assert_eq!(state, NightEditionState::Ribbon);
        let boss = boss_of(&mut w).expect("boss alive");
        assert!(
            (boss.duration - art::RIBBON_LONG).abs() < 1e-6,
            "a misprint's carriage leaves the ribbon open for 2 s, got {}",
            boss.duration
        );
    }

    #[test]
    fn the_pool_is_the_designs_three_thousand_scaled_like_its_siblings() {
        let mut w = printworks(7);
        let boss = spawn_press(&mut w);
        // `WAVE3_DESIGN.md` §12: 3,000 HP for the Printworks, times the same
        // framework scale the others use (`BossKind::hp_scale` × BOSS_POOL_SCALE
        // 4.0 × the frozen hp curve at BOSS_TIME, 2.0).
        let expected = BossKind::NightEdition.hp_pool(2);
        assert!(
            (boss.max_hp - expected).abs() < 1.0,
            "pool {:.0} vs {:.0}",
            boss.max_hp,
            expected
        );
        let mut legacy = crate::tuning::Tuning::default();
        legacy.campaign.carry = true;
        let carried_pool = BossKind::NightEdition.encounter_hp(2, &[], &legacy);
        assert!(carried_pool > 6_000.0, "legacy pool {carried_pool}");
        assert!(boss.max_hp < carried_pool, "fresh pool {}", boss.max_hp);
    }

    // ---- phases ----------------------------------------------------------

    #[test]
    fn the_phase_thresholds_are_sixty_five_and_thirty_percent() {
        assert_eq!(BossKind::NightEdition.phase_for(1.0), 1);
        assert_eq!(BossKind::NightEdition.phase_for(0.66), 1);
        assert_eq!(BossKind::NightEdition.phase_for(0.65), 2);
        assert_eq!(BossKind::NightEdition.phase_for(0.31), 2);
        assert_eq!(BossKind::NightEdition.phase_for(0.30), 3);
        assert_eq!(BossKind::NightEdition.phase_for(0.05), 3);
    }

    #[test]
    fn a_phase_change_is_a_harmless_pose_change() {
        let (mut w, _room) = press_room(41, 1, 34.0);
        drive_until(&mut w, NightEditionState::PageTell, 2.0);
        // Cross the 65 % threshold mid-tell: the telegraph is cancelled.
        with_boss(&mut w, |b| b.hp = b.max_hp * 0.5);
        tick(&mut w);
        let boss = boss_of(&mut w).expect("boss alive");
        assert_eq!(boss.phase, 2);
        assert_eq!(state_of(&boss), NightEditionState::PhaseTurn);
        assert!((boss.duration - art::PHASE_TURN).abs() < 1e-6);
        assert_eq!(boss.contact_damage, 0.0, "the pose change cannot hurt");
        assert!(boss.hazards.is_empty(), "the interrupted tell is cancelled");
        assert!((boss.damage_mult - 1.0).abs() < 1e-6);

        // It lasts exactly the art's beat and then the press walks again.
        let t = drive_until(&mut w, NightEditionState::Roll, 2.0);
        assert!((t - art::PHASE_TURN).abs() < 3.0 * DT);
    }

    // ---- death -----------------------------------------------------------

    #[test]
    fn hazards_stop_when_the_press_dies() {
        let (mut w, _room) = press_room(51, 1, 34.0);
        drive_until(&mut w, NightEditionState::PageTell, 2.0);
        assert_eq!(boss_of(&mut w).expect("alive").hazards.len(), 1);
        with_boss(&mut w, |b| b.hp = 0.0);
        tick(&mut w);
        let boss = boss_of(&mut w).expect("the death beat plays");
        assert!(boss.dead);
        assert_eq!(state_of(&boss), NightEditionState::Wreck);
        assert!(
            boss.hazards.is_empty(),
            "the press's hazards die with it ({} left)",
            boss.hazards.len()
        );
        assert_eq!(boss.contact_damage, 0.0);

        // Stand on the marked page for the whole death beat: nothing lands.
        let before = w.player.hp;
        let page = boss.target;
        let mut completed = false;
        for _ in 0..((art::DEATH_DURATION + 0.2) / DT) as usize {
            w.player.pos = page;
            tick(&mut w);
            completed |= w.level_complete;
        }
        assert!((w.player.hp - before).abs() < 1e-3, "a dead press prints nothing");
        assert!(completed, "the death beat completes the level");
        assert!(boss_of(&mut w).is_none(), "the wreck is reaped at the end");
    }

    // ---- the hunt --------------------------------------------------------

    #[test]
    fn a_rat_outside_the_arena_is_hunted_on_reachable_floor() {
        let mut w = printworks(61);
        spawn_press(&mut w);
        let arena = w.level.fight_rect_around(w.player.pos);
        let radius = BossKind::NightEdition.body_radius();
        let boss = boss_of(&mut w).expect("boss alive");
        // Park the rat far outside the arena, on open floor.
        let start_boss = boss.pos;
        let outside = if arena.center_world().x > w.level.arena.x * 0.5 {
            Vec2::new(w.level.arena.x + 40.0, w.level.arena.y + 40.0)
        } else {
            Vec2::new(w.level.arena.x + w.level.arena.x * 0.5, w.level.arena.y + 40.0)
        };
        w.player.pos = if w
            .level
            .tiles
            .circle_clear(outside, PLAYER_RADIUS, SizeClass::Small)
        {
            outside
        } else {
            w.level
                .tiles
                .nearest_open_center(outside, SizeClass::Small, 40)
                .unwrap_or(outside)
        };
        assert!(
            !arena.contains_world(w.player.pos),
            "the rat has to be outside the arena for this test"
        );
        let start_dist = boss_of(&mut w).expect("alive").pos.dist(w.player.pos);
        for _ in 0..(6.0 / DT) as usize {
            tick(&mut w);
            let boss = boss_of(&mut w).expect("boss alive");
            assert!(
                w.level.tiles.circle_clear(boss.pos, radius, SizeClass::Large),
                "the hunting press left the passable floor at {:?}",
                boss.pos
            );
        }
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            boss.pos.dist(w.player.pos) < start_dist || boss.hunting,
            "the press must walk toward a rat outside its arena (was {start_dist:.0} px, now {:.0})",
            boss.pos.dist(w.player.pos)
        );
        let _ = start_boss;
    }

    // ---- suppression -----------------------------------------------------

    #[test]
    fn a_page_is_suppressed_when_its_blank_margin_is_a_wall() {
        let mut w = printworks(71);
        spawn_press(&mut w);
        // A one-cell-wide slot: no 36 x 30 px page fits in it at all.
        let arena = w.level.fight_rect_around(w.player.pos);
        let x = arena.x + 3;
        let y = arena.y + 3;
        w.level.tiles.fill_cells(
            x - 2,
            y - 2,
            7,
            7,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.fill_cells(x, y, 1, 3, TileKind::Open);
        w.level.tiles.rebuild_masks();
        let slot = Vec2::new((x as f32 + 0.5) * TILE_F, (y as f32 + 1.5) * TILE_F);
        w.player.pos = slot;
        with_boss(&mut w, |b| {
            b.pos = Vec2::new((x as f32 - 3.0) * TILE_F, slot.y);
            b.phase = 1;
        });
        // Give it a long time to try: it must never print into a slot whose
        // safe strip is masonry.
        let mut seen_page = false;
        for _ in 0..(8.0 / DT) as usize {
            tick(&mut w);
            if let Some(boss) = boss_of(&mut w) {
                if matches!(
                    state_of(&boss),
                    NightEditionState::PageTell | NightEditionState::PageStamp
                ) {
                    seen_page = true;
                    break;
                }
            } else {
                break;
            }
            w.player.pos = slot;
        }
        assert!(
            !seen_page,
            "the press must not stamp a sheet it cannot give a safe margin"
        );
    }

    #[test]
    fn a_carriage_is_suppressed_when_the_rule_cannot_run() {
        let mut w = printworks(72);
        spawn_press(&mut w);
        let arena = w.level.fight_rect_around(w.player.pos);
        let x = arena.x + 3;
        let y = arena.y + 3;
        // A three-cell-wide pocket: 48 px, less than the rule's 54 px run.
        w.level.tiles.fill_cells(
            x - 2,
            y - 2,
            7,
            7,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.fill_cells(x, y, 3, 3, TileKind::Open);
        w.level.tiles.rebuild_masks();
        let centre = Vec2::new((x as f32 + 1.5) * TILE_F, (y as f32 + 1.5) * TILE_F);
        w.player.pos = centre + Vec2::new(20.0, 0.0);
        with_boss(&mut w, |b| {
            b.pos = centre;
            b.phase = 2;
            b.flags = F_WANT_CARRIAGE;
        });
        let mut seen_carriage = false;
        for _ in 0..(10.0 / DT) as usize {
            tick(&mut w);
            let Some(boss) = boss_of(&mut w) else { break };
            if matches!(
                state_of(&boss),
                NightEditionState::CarriageTell | NightEditionState::CarriageSweep
            ) {
                seen_carriage = true;
                break;
            }
            w.player.pos = centre + Vec2::new(20.0, 0.0);
        }
        assert!(
            !seen_carriage,
            "a 54 px rule cannot run inside a 48 px pocket"
        );
    }

    // ---- determinism -----------------------------------------------------

    #[test]
    fn twin_press_fights_match() {
        let run = |seed: u64| {
            let mut w = printworks(seed);
            spawn_press(&mut w);
            let mut q = w.ecs.query::<&Boss>();
            let pos = q.iter(&w.ecs).next().expect("boss").pos;
            w.player.pos = pos + Vec2::new(36.0, 0.0);
            // The real schedule: director, horde, the boss and the combat
            // systems all run, so any rng draw outside the boss's stream shows.
            for _ in 0..(25.0 / DT) as usize {
                w.update(DT, Vec2::new(1.0, 0.0), false);
                if !w.player.alive() {
                    break;
                }
            }
            let boss = {
                let mut q = w.ecs.query::<&Boss>();
                q.iter(&w.ecs).next().cloned()
            };
            let boss = boss.expect("the press survives 25 s");
            (
                boss.pos,
                boss.hp,
                boss.state,
                boss.phase,
                boss.hazards.len(),
                w.player.hp,
                w.enemy_count(),
            )
        };
        let a = run(seed_of(9001));
        let b = run(seed_of(9001));
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
        assert_eq!(a.2, b.2);
        assert_eq!(a.3, b.3);
        assert_eq!(a.4, b.4);
        assert_eq!(a.5, b.5);
        assert_eq!(a.6, b.6);
    }

    #[inline]
    fn seed_of(seed: u64) -> u64 {
        seed
    }

    // ---- the bot fights it ----------------------------------------------

    /// A near-complete build (the campaign's usual end state).
    fn give_typical_build(w: &mut World) {
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::game::weapon::WeaponKind;
        let lo = &mut w.player.loadout;
        for kind in [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::SqueakNova,
            WeaponKind::SporeOrbit,
            WeaponKind::TailWhip,
        ] {
            if !lo.has(kind) {
                lo.apply(&Upgrade::NewWeapon(kind));
            }
        }
        for _ in 0..5 {
            for i in 1..lo.weapons.len() {
                lo.apply(&Upgrade::LevelWeapon(i));
            }
        }
        for passive in [
            PassiveKind::MaxHp,
            PassiveKind::Armor,
            PassiveKind::Damage,
            PassiveKind::Regen,
        ] {
            for _ in 0..4 {
                lo.apply(&Upgrade::Passive(passive));
            }
        }
        w.player.hp = w.player.max_hp();
    }

    /// Fight length and read quality on a near-complete build (env-gated; the
    /// balance pass's own harness, the sibling of `measure_campaign_boss_dps`).
    ///
    /// `RATRUN_PRESS_MEASURE=1 cargo test measure_press_fight -- --nocapture`
    /// prints, per seed: seconds from the press arriving to its death, the
    /// rat's HP, the seconds it spent inside a punish window, and how many
    /// times each attack actually landed (the tells a competent bot dodges
    /// should be 0–1 per fight).
    #[test]
    fn measure_press_fight() {
        use crate::game::bot::{Bot, Skill};
        if std::env::var_os("RATRUN_PRESS_MEASURE").is_none() {
            return;
        }
        for seed in [5u64, 41, 77, 1234, 9091, 13, 99] {
            let mut w = printworks(seed);
            give_typical_build(&mut w);
            w.elapsed = crate::game::director::BOSS_TIME - 4.0;
            let mut bot = Bot::new(Skill::Skilled, seed ^ 0x5eed);
            bot.on_new_level(w.level_index);
            let mut ticks = 0u32;
            let limit = (170.0 / DT) as u32;
            let mut weak_secs = 0.0f32;
            // [page, carriage, misprint, body/other]
            let mut hits = [0u32; 4];
            while ticks < limit && !w.level_complete && w.player.alive() {
                let input = bot.decide(&w, DT);
                let hp_before = w.player.hp;
                w.update(DT, input.move_dir, input.dash);
                ticks += 1;
                if let Some(b) = boss_of(&mut w) {
                    let st = state_of(&b);
                    if matches!(st, NightEditionState::BedLowered | NightEditionState::Ribbon) {
                        weak_secs += DT;
                    }
                    if w.player.hp < hp_before - 1e-4 {
                        hits[match st {
                            NightEditionState::PageStamp => 0,
                            NightEditionState::CarriageSweep => 1,
                            NightEditionState::MisprintStamp => 2,
                            _ => 3,
                        }] += 1;
                    }
                }
            }
            let boss_left = boss_of(&mut w).map(|b| b.hp).unwrap_or(0.0);
            eprintln!(
                "PRESS seed={seed} secs={:.1} boss_left={:.0} php={:.0} weak_secs={:.1} hits(page,carriage,misprint,other)={hits:?}",
                ticks as f32 * DT,
                boss_left,
                w.player.hp,
                weak_secs,
            );
        }
    }

    #[test]
    fn the_bot_kills_the_night_edition_over_five_seeds() {
        use crate::game::bot::{Bot, Skill};
        for seed in [5u64, 41, 77, 1234, 9091] {
            let mut w = printworks(seed);
            give_typical_build(&mut w);
            // Wind the clock to the boss so the test measures the fight.
            w.elapsed = crate::game::director::BOSS_TIME - 4.0;
            let mut bot = Bot::new(Skill::Skilled, seed ^ 0x5eed);
            bot.on_new_level(w.level_index);
            let mut ticks = 0u32;
            let limit = (170.0 / DT) as u32;
            while ticks < limit && !w.level_complete && w.player.alive() {
                let input = bot.decide(&w, DT);
                w.update(DT, input.move_dir, input.dash);
                ticks += 1;
            }
            assert!(
                w.player.alive(),
                "seed {seed}: the bot died to the press after {:.0} s",
                ticks as f32 * DT
            );
            assert!(
                w.level_complete,
                "seed {seed}: the bot never killed the press in {:.0} s",
                ticks as f32 * DT
            );
            let sans_boss = {
                let mut q = w.ecs.query::<&Boss>();
                q.iter(&w.ecs).count()
            };
            assert_eq!(sans_boss, 0, "seed {seed}: the press must be dead");
        }
    }

    // ---- captures (env-gated) -------------------------------------------

    /// Render every tell, strike, phase and the death through the **real**
    /// `World::draw`, so the reviewer looks at what the game draws.
    /// `RATRUN_NIGHT_DUMP=/tmp/dir cargo test night_edition_captures`.
    #[test]
    fn night_edition_captures() {
        let Ok(dir) = std::env::var("RATRUN_NIGHT_DUMP") else {
            return;
        };
        let mut w = printworks(2024);
        w.lighting = crate::render::lighting::LightingMode::Off;
        spawn_press(&mut w);
        let vw = 140usize;
        let vh = 100usize;
        w.viewport = Vec2::new(vw as f32, vh as f32);
        let poses = [
            ("roll", NightEditionState::Roll, 0.4),
            ("page_tell", NightEditionState::PageTell, 0.55),
            ("page_stamp", NightEditionState::PageStamp, 0.30),
            ("bed_lowered", NightEditionState::BedLowered, 0.5),
            ("misprint_tell", NightEditionState::MisprintTell, 0.6),
            ("misprint_stamp", NightEditionState::MisprintStamp, 0.4),
            ("carriage_tell", NightEditionState::CarriageTell, 0.6),
            ("carriage_sweep", NightEditionState::CarriageSweep, 0.5),
            ("ribbon", NightEditionState::Ribbon, 0.5),
            ("phase_turn", NightEditionState::PhaseTurn, 0.5),
            ("wreck", NightEditionState::Wreck, 0.55),
        ];
        for (name, state, progress) in poses {
            for phase in [1u8, 2, 3] {
                let anchor = {
                    let boss = boss_of(&mut w).expect("boss alive");
                    boss.pos
                };
                w.player.pos = anchor + Vec2::new(30.0, 4.0);
                with_boss(&mut w, |b| {
                    b.state = BossState::NightEdition(state);
                    b.duration = if state == NightEditionState::Wreck {
                        art::DEATH_DURATION
                    } else {
                        1.0
                    };
                    b.elapsed = b.duration * progress;
                    b.phase = phase;
                    b.hp = b.max_hp * if phase == 1 { 0.8 } else { 0.2 };
                    b.max_hp = b.max_hp.max(1.0);
                    b.target = b.pos + Vec2::new(26.0, -3.0);
                    b.swipe_side = 1.0;
                    b.pounce_origin = b.pos - Vec2::new(6.0, 0.0);
                    b.anim = 1.7;
                    b.hit_flash = 0.0;
                });
                // The spawn burst would sit frozen over the platen in a
                // single-frame capture: clear it so the reviewer sees the art.
                w.particles.items.clear();
                w.particles.shockwaves.clear();
                let mut pb = crate::render::framebuffer::PixelBuffer::new(vw, vh);
                w.elapsed += 0.02;
                w.draw(&mut pb);
                let path = std::path::Path::new(&dir).join(format!("{name}_p{phase}.ppm"));
                let _ = std::fs::write(path, to_ppm(&pb));
            }
        }
    }

    fn to_ppm(pb: &crate::render::framebuffer::PixelBuffer) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for px in pb.pixels() {
            out.push(px.0);
            out.push(px.1);
            out.push(px.2);
        }
        out
    }
}
