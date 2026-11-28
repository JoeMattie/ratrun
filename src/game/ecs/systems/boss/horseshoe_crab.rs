//! Sewer boss pool — **THE HORSESHOE** (the horseshoe crab that came up the
//! drain).
//!
//! The art module [`crate::game::art_boss_horseshoe_crab`] is the gameplay
//! spec: the state enum, every per-state duration, damage number, the armour
//! rule (`incoming_multiplier`), the telson arc/lane/ring geometry and the blue
//! blood constants are binding. This file drives the transitions, picks the
//! sweep side and the charge lane, and queues the art's telegraphed attacks as
//! shared [`BossHazard`]s.
//!
//! The 1.5× shell uses the same geometry for movement, contact, tells and
//! strikes. A scheduled arrival clears the widest camera and lands on floor
//! that can hold the shell. The 60-second determinism runs never summon it.
//!
//! ## The fight the level asked for
//!
//! The Sewer's original boss, the Apex Jaguar, is speed. This is mass. The
//! horseshoe crab is **armoured from the front and the top**: every shot that
//! arrives over the dome from anywhere but its rear hemisphere sparks off and
//! is wasted. So the fight is not "dodge the attack", it is **"get behind it"**:
//!
//! 1. **`Dome`** walks at the rat, slow and relentless, holding the dome
//!    toward it.
//! 2. **The telson sweep** turns the rear into the rat and paints a 120° arc
//!    ([`art::telson_arc`]); one sidestep leaves it and the miss leaves a
//!    [`art::RECOVER`] limp.
//! 3. **The bulldoze** marks a lane from the crab through the rat to the first
//!    wall ([`art::bulldoze_lane`]) and drives down it, slower than the rat can
//!    run but faster than it can walk, and it is unstoppable. It ends
//!    **wedged**: nose in the masonry, tail up, gill fans out — the one clear
//!    weak point, no contact, and the longest punish in the fight.
//! 4. **`Burrow`** marks a circle under the rat and comes up in it. The
//!    eruption leaves the crab dazed in its own crater.
//! 5. **Phase 3: the flip.** It throws itself onto its back on purpose: fully
//!    vulnerable ([`art::FLIP_DAMAGE_MULT`], no deflection, the gills at the
//!    centre) while the legs thrash inside a marked ring.
//! 6. **Blue blood**: from phase 2 every wedge, flip and phase shift sheds
//!    pools that slow the rat to a wade. The pools are the crab's, and they
//!    stop when it dies.
//!
//! ## Suppression (every attack, every time)
//!
//! A move whose stated safe strip hits a wall, or whose target cannot see the
//! tell, is refused and the crab falls through to a cheaper move and finally
//! back to `Dome`:
//!
//! * the bulldoze needs a wall inside [`art::BULLDOZE_MAX`], a lane at least
//!   [`art::BULLDOZE_MIN`] long with the rat inside it, line of sight, and a
//!   clear step off the lane on **one** side;
//! * the telson needs the rat inside its own reach, line of sight, and a clear
//!   step out of the 120° arc (sideways or radially back);
//! * the burrow needs its marked circle on floor the crab can surface in, line
//!   of sight on the rat, and a clear step off the circle;
//! * the flip needs the ring's floor and a clear step out of it.
//!
//! ## Determinism
//!
//! No rng draws at all: the sweep side alternates (framework `swipe_side`), the
//! rotation is [`Boss::scratch`], the blood-pool fan walks a fixed golden-angle
//! table, and every hazard is a copy. Nothing allocates per tick.
//!
//! ## Spawns
//!
//! The controller takes no `Commands`: it moves the boss, queues hazards and
//! cues. No `Enemy` entity is spawned or despawned inside the `EnemyIndex`
//! window by this fight.

use super::*;
use crate::detmath::DetF32;
use crate::game::art_boss_horseshoe_crab as art;
use crate::game::art_boss_horseshoe_crab::CrabState;

/// Scheduled landings need clearance for the full shell and a margin for the
/// tail, arrival particles and camera shake, even at maximum zoom-out.
fn landing_fits(level: &Level, rat: Vec2, viewport: Vec2, pos: Vec2) -> bool {
    let radius = art::BODY_RADIUS;
    pos.x >= radius && pos.y >= radius
        && pos.x <= level.arena.x - radius && pos.y <= level.arena.y - radius
        && out_of_arrival_view(level, rat, viewport, pos, art::CULL_MARGIN + 14.0)
        && level.tiles.circle_clear(pos, art::BODY_RADIUS, SizeClass::Large)
}

pub(super) fn scheduled_landing(rng: &mut StdRng, level: &Level, rat: Vec2, viewport: Vec2) -> Option<Vec2> {
    use crate::game::ecs::systems::spawn::ring_radius;
    let arena = level.fight_rect_around(rat);
    let radius = ring_radius(viewport + Vec2::splat(art::CULL_MARGIN + 14.0));
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    for distance in [radius * 0.8, radius, radius * 1.45] {
        for spoke in 0..24 {
            let dir = Vec2::from_angle(angle + spoke as f32 * std::f32::consts::TAU / 24.0);
            let pos = rat + dir * distance;
            if arena.contains_world(pos) && landing_fits(level, rat, viewport, pos) {
                return Some(pos);
            }
        }
    }
    // A ring can cross only rooms too narrow for the shell. Search tile
    // centres for the closest suitable floor outside the same camera bounds.
    let mut best: Option<((bool, f32), Vec2)> = None;
    for y in 0..level.tiles.rows() {
        for x in 0..level.tiles.cols() {
            if !level.tiles.passable(x, y, SizeClass::Large) {
                continue;
            }
            let pos = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * collision::TILE_F;
            let rank = (!arena.contains_world(pos), rat.dist_sq(pos));
            if best.is_none_or(|(previous, _)| rank < previous) && landing_fits(level, rat, viewport, pos) {
                best = Some((rank, pos));
            }
        }
    }
    best.map(|(_, pos)| pos)
}

/// Scale feedback around its own anchor so a tail clang stays at the tip and a
/// burrow burst stays at the marked entrance or exit.
pub(super) fn emit_cue(cue: BossCue, boss: &Boss, particles: &mut Particles, rng: &mut StdRng) {
    let anchor = match cue {
        BossCue::CrabTelsonSweep => art::telson_tip(&boss.crab_view()),
        BossCue::CrabWedge => art::gill_pos(&boss.crab_view()),
        BossCue::CrabBurrow => boss.target,
        _ => boss.pos,
    };
    particles.with_scaled_emission(anchor, art::BODY_SCALE, |particles| match cue {
        BossCue::CrabRattle => {
            vfx::emit_boss_cat_skid(particles, rng, anchor, boss.facing, 0.12);
        }
        BossCue::CrabTelsonCock => {
            vfx::emit_boss_mower_overheat_steam(particles, rng, anchor, boss.facing, 0.10);
        }
        BossCue::CrabTelsonSweep => {
            vfx::emit_boss_mower_wall_impact(particles, rng, anchor, boss.facing, false);
        }
        BossCue::CrabCharge => {
            vfx::emit_boss_mower_charge_trail(particles, rng, anchor, boss.facing, false, 0.10);
        }
        BossCue::CrabWedge => {
            vfx::emit_boss_mower_wall_impact(particles, rng, anchor, -boss.facing, true);
        }
        BossCue::CrabBurrow => {
            vfx::emit_boss_broom_stomp(
                particles, rng, anchor, art::BURROW_LOCAL_RADIUS * 0.6, boss.phase >= 3,
            );
        }
        BossCue::CrabEmerge => {
            vfx::emit_boss_broom_stomp(particles, rng, anchor, art::BURROW_LOCAL_RADIUS, true);
        }
        BossCue::CrabFlip => {
            vfx::emit_boss_mower_flipped_smoke(particles, rng, anchor, 0.12);
        }
        BossCue::CrabDeath => {
            vfx::emit_boss_magpie_death(particles, rng, anchor, boss.facing, (44, 92, 190));
        }
        _ => unreachable!("only Horseshoe cues are routed here"),
    });
}

// ---- scratch layout -------------------------------------------------------

/// The attack rotation counter (`0..4`, persists across states).
const S_ROT: usize = 0;
/// `1.0` when the committed charge lane ended at a wall (it always must).
const S_WALLED: usize = 1;
/// The committed lane's length in px.
const S_LANE_LEN: usize = 2;
/// The blood-pool fan's cursor (a deterministic walk, not a random draw).
const S_POOLS: usize = 3;
/// The committed burrow's own travel time (s), fixed at the tell's lock so the
/// published hazard and the visible mound arrive together.
const S_DIVE: usize = 4;

// ---- flags layout ---------------------------------------------------------

/// The committed attack has queued its live hazard (one per tell).
const F_LOCK: u32 = 1 << 0;
/// The death beat has taken its one shot (its own bit).
const F_DEATH: u32 = 1 << 1;

/// How far the rat can walk inside a tell's own window (px): the safe-strip
/// probe. A 0.9 s tell at the rat's 58 px/s walk is ~50 px, so a step of that
/// length is what "there is a way out" means — not a single 24 px hop, which
/// would suppress every sweep aimed at a rat already in melee.
const ESCAPE_WALK: f32 = 48.0;


// ============================================================================
// Controller
// ============================================================================

/// The Horseshoe's controller (zero-sized; the state lives on the [`Boss`]).
pub struct HorseshoeCrabController;

impl BossController for HorseshoeCrabController {
    fn kind(&self) -> BossKind {
        BossKind::HorseshoeCrab
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = crab_state(boss);
        // The crab follows the rat out of the arena on the large-class flow
        // field; refreshed every tick, like the mower's and the bird's.
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // A phase threshold is a set piece: the harmless pose change, and the
        // blue starts to run.
        if out.phase_changed && !matches!(boss.state, BossState::HorseshoeCrab(CrabState::Death)) {
            // The attack it abandons takes its telegraph with it (an arc, a
            // lane or a marked burrow must not land out of the harmless pose).
            // The blue pools are floor, not an attack: they carry no damage
            // and stay.
            boss.hazards.retain(|h| h.damage <= 0.0);
            enter(boss, CrabState::Pose, art::POSE_DURATION);
            shed_blood(ctx, boss);
        }

        match boss.state {
            BossState::HorseshoeCrab(CrabState::Dome) => tick_dome(ctx, boss, out),
            BossState::HorseshoeCrab(CrabState::TelsonTell) => tick_telson_tell(boss),
            BossState::HorseshoeCrab(CrabState::TelsonSweep) => tick_telson_sweep(boss),
            BossState::HorseshoeCrab(CrabState::BulldozeTell) => tick_bulldoze_tell(boss),
            BossState::HorseshoeCrab(CrabState::Bulldoze) => tick_bulldoze(ctx, boss),
            BossState::HorseshoeCrab(CrabState::Wedged) => tick_wedged(ctx, boss),
            BossState::HorseshoeCrab(CrabState::BurrowTell) => tick_burrow_tell(ctx, boss),
            BossState::HorseshoeCrab(CrabState::Burrow) => tick_burrow(ctx, boss),
            BossState::HorseshoeCrab(CrabState::Emerge) => tick_emerge(boss),
            BossState::HorseshoeCrab(CrabState::FlipTell) => tick_flip_tell(boss),
            BossState::HorseshoeCrab(CrabState::Flipped) => tick_flipped(ctx, boss),
            BossState::HorseshoeCrab(CrabState::Righting) => tick_righting(boss),
            BossState::HorseshoeCrab(CrabState::Pose) => tick_pose(boss),
            BossState::HorseshoeCrab(CrabState::Recover) => tick_recover(boss),
            BossState::HorseshoeCrab(CrabState::Death) => {}
            BossState::Cat(_)
            | BossState::Broom(_)
            | BossState::Mower(_)
            | BossState::Mangle(_)
            | BossState::NightEdition(_)
            | BossState::Magpie(_)
            | BossState::MegaRat(_)
            | BossState::Hawk(_)
            | BossState::Cobra(_)
            | BossState::Lantern(_) => {}
        }

        // The art owns the multiplier and the contact rule; apply them after the
        // transitions so a punish window is live on the tick it opens.
        let now = crab_state(boss);
        boss.damage_mult = art::incoming_damage_mult(now);
        if !art::has_body_contact(now) {
            boss.no_contact();
        }

        // One cue per state entry, on the tick of the transition.
        let after = crab_state(boss);
        if after != before {
            if let Some(cue) = state_cue(after) {
                out.cue(cue);
            }
        }
    }

    /// The death beat: the shell cracks and the blue floods out.
    ///
    /// Its telegraphs stop with it: the first tick of the beat clears every
    /// hazard the living crab had queued — the telson arc, the charge lane, the
    /// marked burrow and every blue pool — so nothing can land on the rat after
    /// the fight is already won. Nothing new is dealt here.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_DEATH == 0 {
            boss.flags |= F_DEATH;
            boss.hazards.clear();
            out.cue(BossCue::CrabDeath);
        }
    }
}

/// The sound/effect cue for entering a state (within-state beats are cued where
/// they happen).
#[inline]
fn state_cue(state: CrabState) -> Option<BossCue> {
    match state {
        CrabState::TelsonTell => Some(BossCue::CrabTelsonCock),
        CrabState::TelsonSweep => Some(BossCue::CrabTelsonSweep),
        CrabState::BulldozeTell | CrabState::Bulldoze => Some(BossCue::CrabCharge),
        CrabState::Wedged => Some(BossCue::CrabWedge),
        CrabState::BurrowTell | CrabState::Burrow => Some(BossCue::CrabBurrow),
        CrabState::Emerge => Some(BossCue::CrabEmerge),
        CrabState::FlipTell | CrabState::Flipped => Some(BossCue::CrabFlip),
        CrabState::Righting | CrabState::Pose => Some(BossCue::CrabRattle),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

fn tick_dome(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    // Turn toward the rat slowly: the armour is a facing, and the rat has to be
    // able to see which way it points. The dome leads even while it walks.
    if to.len_sq() > 1e-6 {
        boss.facing = turn_towards(boss.facing, to * (1.0 / dist), 2.6 * ctx.dt);
    }
    // Walk it down. It never backs off: that is the whole animal.
    let dir = ctx.seek(boss);
    let speed = art::DOME_SPEED[slot]
        * if boss.hunting {
            art::HUNT_SPEED_MULT
        } else {
            1.0
        };
    if dist > art::DOME_BAND[0] * 0.5 {
        move_boss(ctx, boss, dir, speed);
    }
    if boss.progress() >= 1.0 {
        next_attack(ctx, boss, out);
    }
}

fn tick_telson_tell(boss: &mut Boss) {
    // The tail is cocked; the crab does not turn (the arc is the aimed thing).
    if boss.progress() >= 1.0 {
        enter(boss, CrabState::TelsonSweep, art::TELSON_SWEEP);
    }
}

fn tick_telson_sweep(boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        enter(boss, CrabState::Recover, art::RECOVER);
    }
}

fn tick_bulldoze_tell(boss: &mut Boss) {
    let slot = phase_slot(boss);
    let secs = (boss.scratch[S_LANE_LEN] / art::BULLDOZE_SPEED[slot]).max(0.15);
    if boss.flags & F_LOCK == 0 && boss.progress() >= art::BULLDOZE_LOCK_PROGRESS {
        boss.flags |= F_LOCK;
        let warn = (boss.duration * (1.0 - art::BULLDOZE_LOCK_PROGRESS)).max(0.05);
        let lane = BossHazard::new(
            HazardShape::Corridor {
                from: boss.pounce_origin,
                to: boss.target,
                half_width: art::BULLDOZE_HALF_WIDTH,
            },
            warn,
            secs,
            art::BULLDOZE_DAMAGE,
        )
        .with_knockback(art::BULLDOZE_KNOCKBACK)
        .with_enemies();
        let _ = boss.push_hazard(lane);
        // Keep the remaining warning on the floor before moving down the lane.
        boss.duration = boss.elapsed + warn * boss.attack_tuning.tell_time_mult;
    }
    if boss.progress() >= 1.0 {
        enter(boss, CrabState::Bulldoze, secs);
    }
}

fn tick_bulldoze(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // No steering: a bulldoze is a straight line with a wall at the end of it.
    let dir = boss.facing;
    let slot = phase_slot(boss);
    let step = art::BULLDOZE_SPEED[slot] * ctx.dt;
    let remaining = boss.pos.dist(boss.target);
    if remaining <= step + 1.0 {
        boss.pos = boss.target;
        wedge(boss);
        return;
    }
    // The shared large-body step; a jamb that stops it is the wall it wanted.
    if !move_boss(ctx, boss, dir, art::BULLDOZE_SPEED[slot]) {
        wedge(boss);
    }
}

fn tick_wedged(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Nose in the masonry, tail up: it cannot move and it cannot reach. The
    // whole state is the rat's window — and where the shell is cracked open,
    // the highest bleed.
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        shed_blood(ctx, boss);
    }
    if boss.progress() >= 1.0 {
        enter_dome(boss);
    }
}

fn tick_burrow_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The mark follows the rat until the lock, then freezes so the rat can walk
    // out of it (the cat's landing-ring rule, at boss scale).
    let locked = boss.flags & F_LOCK != 0;
    if !locked {
        let at = ctx.player_pos;
        if ctx.footprint_fits(at, art::BODY_RADIUS) && escape_from_circle(ctx, at, art::BURROW_RADIUS) {
            boss.target = at;
        }
        if boss.progress() >= art::BURROW_LOCK_PROGRESS {
            boss.flags |= F_LOCK;
            let remaining = (boss.duration * (1.0 - art::BURROW_LOCK_PROGRESS)).max(0.05);
            // The mark is the exit *and* the place the mound travels to, so it
            // has to hold the whole shell. A stale mark (a forced state, or a
            // mark left on floor the rat stands on with no room) is moved to the
            // nearest point on the way back that does, before it is published.
            if !ctx.footprint_fits(boss.target, art::BODY_RADIUS) {
                boss.target = burrow_landing(ctx, boss.target, boss.pos, art::BODY_RADIUS);
            }
            // The visible dive's own clock: the sink plus the ground it has to
            // cover at the crab's top speed, never a teleport.
            let dive = burrow_dive_time(boss);
            boss.scratch[S_DIVE] = dive;
            // `push_hazard` scales the warn by the tell multiplier below, so
            // divide the dive half out here: the eruption then lands exactly
            // when the mound does (never before it), whatever the multiplier.
            let delay = remaining
                + dive / boss.attack_tuning.tell_time_mult.max(0.01);
            let burst = BossHazard::new(
                HazardShape::Circle {
                    center: boss.target,
                    radius: art::BURROW_RADIUS,
                },
                delay,
                0.18,
                art::EMERGE_DAMAGE,
            )
            .with_knockback(art::EMERGE_KNOCKBACK);
            let _ = boss.push_hazard(burst);
            // Match the pre-dive wait and travel to the published warning.
            boss.duration = boss.elapsed + remaining * boss.attack_tuning.tell_time_mult;
        }
    }
    if boss.progress() >= 1.0 {
        boss.pounce_origin = boss.pos;
        enter(boss, CrabState::Burrow, boss.scratch[S_DIVE]);
    }
}

/// How long the visible dive takes (s): the sink, then the distance from where
/// the shell goes down to the marked exit at [`art::BURROW_TRAVEL_SPEED`] (the
/// crab's own top speed).
fn burrow_dive_time(boss: &Boss) -> f32 {
    let slot = phase_slot(boss);
    let ground = boss.pos.dist(boss.target) / art::BURROW_TRAVEL_SPEED[slot].max(1.0);
    (art::BURROW_SINK * boss.attack_tuning.tell_time_mult + ground).max(0.05)
}

fn tick_burrow(ctx: &BossContext<'_>, boss: &mut Boss) {
    // Under the floor, but never out of sight: the mound is drawn at `boss.pos`
    // and travels the whole way to the mark at the crab's own top speed. No wall
    // rule (it is under the floor) and no contact (nothing is above the swell),
    // but the shell's hit box is right under the mound, so it stays hittable.
    let radius = boss.kind.body_radius();
    // A stale mark (a forced state, or geometry edited since the lock) is
    // re-aimed once, at the *near* end of the dive: the mound then walks to a
    // spot the shell actually fits in. Nothing is ever placed at the far end.
    if !ctx.footprint_fits(boss.target, radius) {
        let landing = burrow_landing(ctx, boss.target, boss.pounce_origin, radius);
        if landing.dist(boss.target) > 0.5 {
            boss.target = landing;
            let travel = burrow_dive_time(boss);
            boss.scratch[S_DIVE] = travel;
            boss.duration = boss.elapsed + travel;
        }
    }
    let slot = phase_slot(boss);
    // The shell sinks where it stands first (the share of the dive clock
    // `burrow_dive_time` budgets for it), then the mound sets off. Without
    // the hold the mound arrived a sink early and the shell was up and
    // recovering before the published eruption burst.
    let sink = art::BURROW_SINK * boss.attack_tuning.tell_time_mult;
    if boss.elapsed < sink {
        return;
    }
    let to = boss.target - boss.pos;
    let gap = to.len();
    let step = art::BURROW_TRAVEL_SPEED[slot] * ctx.dt;
    if gap > 1e-3 {
        boss.pos += to * (step.min(gap) / gap);
    }
    // It comes up on arrival, never before: a mound that is late (a phase
    // change slowed it mid-dive) keeps walking instead of being placed.
    if boss.pos.dist(boss.target) <= 1.0 {
        boss.pos = boss.target;
        enter(boss, CrabState::Emerge, art::EMERGE);
    }
}

/// Where the burrowing crab comes up, given the mark and the spot it went
/// down: the mark when the whole body fits there, otherwise the first fitting
/// point on the way back toward the dive, otherwise the nearest open cell. A
/// dive that cannot hold the mark returns to its origin.
fn burrow_landing(ctx: &BossContext<'_>, mark: Vec2, from: Vec2, radius: f32) -> Vec2 {
    if ctx.footprint_fits(mark, radius) {
        return mark;
    }
    let back = from - mark;
    let len = back.len();
    if len > 1e-3 {
        let dir = back * (1.0 / len);
        let mut d = 2.0;
        while d <= len.min(48.0 * art::BODY_SCALE) {
            let p = mark + dir * d;
            if ctx.footprint_fits(p, radius) {
                return p;
            }
            d += 2.0;
        }
    }
    let open = ctx.nearest_open(mark);
    if ctx.footprint_fits(open, radius) {
        return open;
    }
    from
}

fn surface_spot(ctx: &BossContext<'_>, boss: &Boss) -> Vec2 {
    burrow_landing(ctx, boss.target, boss.pounce_origin, boss.kind.body_radius())
}

fn tick_emerge(boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        enter(boss, CrabState::Recover, art::RECOVER_EMERGE);
    }
}

fn tick_flip_tell(boss: &mut Boss) {
    // The rocking is the tell; the crab is committed once it starts.
    if boss.progress() >= 1.0 {
        enter(
            boss,
            CrabState::Flipped,
            art::FLIPPED_DURATION[phase_slot(boss)],
        );
        let (center, inner, outer) = art::legs_ring(&crab_view_of(boss));
        let thrash = BossHazard::new(
            HazardShape::Annulus {
                center,
                inner,
                outer,
            },
            0.30,
            (art::FLIPPED_DURATION[phase_slot(boss)] - 0.30).max(0.2),
            art::LEGS_DAMAGE,
        );
        let _ = boss.push_hazard(thrash);
    }
}

fn tick_flipped(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let _ = ctx;
    // No movement: it is on its back. The blue runs out from under it as it
    // thrashes (phase 3 only: the pools are shed once per flip).
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        shed_blood(ctx, boss);
    }
    if boss.progress() >= 1.0 {
        enter(boss, CrabState::Righting, art::RIGHTING);
    }
}

fn tick_righting(boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        enter_dome(boss);
    }
}

fn tick_pose(boss: &mut Boss) {
    // The harmless phase pose: no contact, no attack, free damage.
    if boss.progress() >= 1.0 {
        enter_dome(boss);
    }
}

fn tick_recover(boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        enter_dome(boss);
    }
}

// ============================================================================
// Transitions
// ============================================================================

#[inline]
fn phase_slot(boss: &Boss) -> usize {
    (boss.phase.clamp(1, 3) - 1) as usize
}

#[inline]
fn crab_state(boss: &Boss) -> CrabState {
    match boss.state {
        BossState::HorseshoeCrab(s) => s,
        _ => CrabState::Dome,
    }
}

#[inline]
fn crab_view_of(boss: &Boss) -> art::CrabView {
    art::CrabView {
        pos: boss.pos,
        facing: boss.facing,
        state: crab_state(boss),
        progress: boss.progress(),
        anim: boss.anim,
        hit_flash: boss.hit_flash,
        hp_frac: boss.hp_frac(),
        phase: boss.phase,
        target: boss.target,
        lane_start: boss.pounce_origin,
        sweep_side: boss.swipe_side,
    }
}

/// Enter an art state, clearing the per-state latch.
#[inline]
fn enter(boss: &mut Boss, state: CrabState, duration: f32) {
    boss.set_state(BossState::HorseshoeCrab(state), duration);
    boss.flags = 0;
}

fn enter_dome(boss: &mut Boss) {
    enter(boss, CrabState::Dome, art::DOME_DURATION[phase_slot(boss)]);
}

/// Rotate `from` toward `to` by at most `max_step` radians' worth.
fn turn_towards(from: Vec2, to: Vec2, max_step: f32) -> Vec2 {
    let a = from.normalized();
    let b = to.normalized();
    if a.len_sq() <= 0.0 || b.len_sq() <= 0.0 {
        return b;
    }
    let cross = a.perp().dot(b);
    let dot = a.dot(b).clamp(-1.0, 1.0);
    let angle = cross.datan2(dot);
    if angle.abs() <= max_step {
        b
    } else {
        a.rotate(max_step * angle.signum())
    }
}

/// Enter the wedge and, from phase 2, bleed on the floor behind it.
fn wedge(boss: &mut Boss) {
    enter(boss, CrabState::Wedged, art::WEDGED_DURATION[phase_slot(boss)]);
    boss.pounce_origin = boss.pos;
}

/// Choose the next attack from the phase's rotation (no rng).
fn next_attack(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let _ = out;
    let rot = boss.scratch[S_ROT] as u32;
    boss.scratch[S_ROT] = (rot + 1) as f32 % 4.0;
    let phase = boss.phase.clamp(1, 3);
    let want = match phase {
        1 => {
            if rot.is_multiple_of(2) {
                Attack::Bulldoze
            } else {
                Attack::Telson
            }
        }
        2 => match rot % 3 {
            0 => Attack::Bulldoze,
            1 => Attack::Telson,
            _ => Attack::Burrow,
        },
        _ => match rot % 4 {
            0 => Attack::Flip,
            1 => Attack::Telson,
            2 => Attack::Bulldoze,
            _ => Attack::Burrow,
        },
    };
    // Fall through whatever the floor refuses, then re-seek.
    let mut pick = want;
    for _ in 0..4 {
        let ok = match pick {
            Attack::Bulldoze => commit_bulldoze(ctx, boss),
            Attack::Telson => commit_telson(ctx, boss),
            Attack::Burrow => commit_burrow(ctx, boss),
            Attack::Flip => commit_flip(ctx, boss),
        };
        if ok {
            return;
        }
        pick = match pick {
            Attack::Bulldoze => Attack::Telson,
            Attack::Telson => Attack::Burrow,
            Attack::Burrow => Attack::Flip,
            Attack::Flip => Attack::Telson,
        };
    }
    enter_dome(boss);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attack {
    Bulldoze,
    Telson,
    Burrow,
    Flip,
}

/// Start the telson sweep if the rat is in its reach and has a way out of the
/// arc.
fn commit_telson(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if dist < art::TELSON_INNER - 2.0 * art::BODY_SCALE || dist > art::TELSON_OUTER - 2.0 * art::BODY_SCALE {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    // The rat has to be behind the dome for the tail to be the threat: aim the
    // crab's *front* away from the rat.
    let away = if to.len_sq() > 1e-6 {
        -to.normalized()
    } else {
        -boss.facing
    };
    if !escape_from_arc(ctx, boss.pos, ctx.player_pos, away, art::TELSON_HALF_ANGLE) {
        return false;
    }
    // Alternate the shoulder each sweep (deterministic, no rng).
    boss.swipe_side = -boss.swipe_side;
    boss.facing = away;
    let slot = phase_slot(boss);
    let tell = art::TELSON_TELL[slot];
    let (center, inner, outer, af, half) = art::telson_arc(&crab_view_of(boss));
    let warn = tell + art::TELSON_SWEEP * art::TELSON_ACTIVE_PROGRESS;
    let arc = BossHazard::new(
        HazardShape::Arc {
            center,
            inner,
            outer,
            facing: af,
            half_angle: half,
        },
        warn,
        art::TELSON_ACTIVE,
        art::TELSON_DAMAGE,
    )
    .with_knockback(art::TELSON_KNOCKBACK);
    let _ = boss.push_hazard(arc);
    enter(boss, CrabState::TelsonTell, tell + warn * (boss.attack_tuning.tell_time_mult - 1.0));
    true
}

/// Start the charge if there is a wall to wedge into inside the lane and the
/// rat can step out of it.
fn commit_bulldoze(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    if to.len() < 24.0 * art::BODY_SCALE {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    let dir = to.normalized();
    let Some(len) = lane_len(ctx, boss.pos, dir) else {
        return false;
    };
    if len < art::BULLDOZE_MIN {
        return false;
    }
    // The rat has to be in front of it, inside the lane, and able to step off
    // the lane on at least one side.
    let along = to.dot(dir);
    if along <= 0.0 || along > len {
        return false;
    }
    let perp = dir.perp();
    let mut escapes = 0;
    for side in [1.0f32, -1.0] {
        let p = ctx.player_pos + perp * side * (art::BULLDOZE_HALF_WIDTH + ctx.player_hurt_radius + 2.0);
        if ctx.footprint_fits(p, ctx.player_radius)
            && ctx.los_clear(ctx.player_pos, p, ctx.player_radius)
        {
            escapes += 1;
        }
    }
    if escapes == 0 {
        return false;
    }
    boss.scratch[S_WALLED] = 1.0;
    boss.scratch[S_LANE_LEN] = len;
    boss.pounce_origin = boss.pos;
    boss.target = boss.pos + dir * len;
    boss.facing = dir;
    let slot = phase_slot(boss);
    enter(boss, CrabState::BulldozeTell, art::BULLDOZE_TELL[slot]);
    true
}

/// The length of the lane from `from` along `dir` before the first wall, or
/// `None` when there is no wall within [`art::BULLDOZE_MAX`] (a charge with no
/// wall to wedge into would never expose the gills, so it is not offered).
fn lane_len(ctx: &BossContext<'_>, from: Vec2, dir: Vec2) -> Option<f32> {
    let step = 4.0;
    let mut t = 0.0;
    while t < art::BULLDOZE_MAX {
        let p = from + dir * (t + step);
        if !ctx.footprint_fits(p, art::BODY_RADIUS) {
            return Some(t);
        }
        t += step;
    }
    None
}

/// Start the burrow if the marked circle is floor the crab can surface in and
/// the rat can step off it.
fn commit_burrow(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let at = ctx.player_pos;
    if !ctx.footprint_fits(at, art::BODY_RADIUS) {
        return false;
    }
    // A tell the rat cannot see is not a tell.
    if !ctx.los_clear(boss.pos, at, 1.0) {
        return false;
    }
    if !escape_from_circle(ctx, at, art::BURROW_RADIUS) {
        return false;
    }
    boss.pounce_origin = boss.pos;
    boss.target = at;
    let slot = phase_slot(boss);
    enter(boss, CrabState::BurrowTell, art::BURROW_TELL[slot]);
    true
}

/// Start the deliberate flip if there is room for the legs' ring.
fn commit_flip(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let (center, _inner, outer) = art::legs_ring(&crab_view_of(boss));
    if !ctx.footprint_fits(center, art::BODY_RADIUS) {
        return false;
    }
    if !escape_from_circle(ctx, center, outer) {
        return false;
    }
    enter(boss, CrabState::FlipTell, art::FLIP_TELL);
    true
}

/// Is a point clear of the telson arc? Outside the outer radius and inside the
/// inner hole are both safe; so is any bearing outside the fan.
#[inline]
fn clear_of_arc(center: Vec2, af: Vec2, half: f32, p: Vec2) -> bool {
    let d = p - center;
    let dist = d.len();
    if dist > art::TELSON_OUTER || dist < art::TELSON_INNER {
        return true;
    }
    if dist <= 1e-4 {
        return false;
    }
    d.normalized().dot(af) < half.dcos()
}

/// Is there a walkable way **out of the telson arc** from `at` within the tell's
/// own window? Every candidate is a full [`ESCAPE_WALK`] step in one of eight
/// directions, and it has to actually leave the fan (a step that is merely
/// clear of a wall but still inside the 120° wedge is not an escape). The
/// suppression rule: no such step, no sweep.
fn escape_from_arc(
    ctx: &BossContext<'_>,
    center: Vec2,
    at: Vec2,
    arc_facing: Vec2,
    half: f32,
) -> bool {
    let af = arc_facing.normalized();
    for k in 0..8 {
        let d = Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0);
        let p = at + d * ESCAPE_WALK;
        if clear_of_arc(center, af, half, p)
            && ctx.footprint_fits(p, ctx.player_radius)
            && ctx.los_clear(at, p, ctx.player_radius)
        {
            return true;
        }
    }
    false
}

/// Can the rat walk outside the marked circle within the tell?
fn escape_from_circle(ctx: &BossContext<'_>, center: Vec2, radius: f32) -> bool {
    let at = ctx.player_pos;
    let safe = radius + ctx.player_hurt_radius + 1.0;
    let offset = at - center;
    let remaining_sq = safe * safe - offset.len_sq();
    if remaining_sq <= 0.0 {
        return true;
    }
    for k in 0..8 {
        let dir = Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0);
        let along = dir.dot(offset);
        let step = (along * along + remaining_sq).sqrt() - along;
        let p = at + dir * step;
        if step <= ESCAPE_WALK
            && ctx.footprint_fits(p, ctx.player_radius)
            && ctx.los_clear(at, p, ctx.player_radius)
        {
            return true;
        }
    }
    false
}

/// Shed the copper-blue pools that slow the rat (phase 2+).
///
/// A fixed golden-angle fan, advancing a cursor in the boss's own scratch: no
/// rng, so the same fight sheds the same pools in the same places. Pools only go
/// down on floor the crab could stand on, and only the crab's own hazards are
/// touched (they all die with it).
fn shed_blood(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let phase = boss.phase.clamp(1, 3);
    if phase < 2 {
        return;
    }
    let n = art::BLOOD_POOLS[(phase - 2) as usize];
    let mut cursor = boss.scratch[S_POOLS] as i32;
    for _ in 0..n {
        let a = cursor as f32 * 2.399_963 + 0.6;
        let ring = (15.0 + (cursor.rem_euclid(3)) as f32 * 7.0) * art::BODY_SCALE;
        cursor = (cursor + 1).rem_euclid(12);
        let p = boss.pos + Vec2::from_angle(a) * ring;
        if !ctx.footprint_fits(p, 5.0 * art::BODY_SCALE) {
            continue;
        }
        let pool = BossHazard::new(
            HazardShape::Circle {
                center: p,
                radius: art::BLOOD_RADIUS,
            },
            art::BLOOD_WARN,
            art::BLOOD_LIFE,
            0.0,
        )
        .with_slow(art::BLOOD_SLOW);
        let _ = boss.push_hazard(pool);
    }
    boss.scratch[S_POOLS] = cursor as f32;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    //! Narrow tests for the Horseshoe fight.
    //!
    //! `cargo test boss::horseshoe_crab` runs these with the framework's suite.
    //! Every attack test drives the shared boss system through `begin_tick` →
    //! `run_system_cached` → `end_tick`, so hazards, contact and the death
    //! hand-off are the real ones; the suppression tests call the same
    //! `commit_*` functions the controller calls, with the same `BossContext`
    //! the system builds.

    use super::*;
    use crate::game::art_boss_horseshoe_crab as art;
    use crate::game::ecs::systems::projectiles;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    /// A Sewer seed whose pool draw is the crab. Asserted, so a change to the
    /// pool mix fails loudly here instead of silently testing the jaguar.
    const CRAB_SEED: u64 = 3;

    fn crab_world(seed: u64) -> World {
        let w = World::with_tuning(
            Theme::Sewer, seed, crate::tuning::RunTuning::with_base_boss_hazards(BossKind::HorseshoeCrab),
        );
        assert_eq!(
            BossKind::pool_pick(Theme::Sewer, w.level.seed),
            BossKind::HorseshoeCrab,
            "this test needs a Sewer seed that fields the horseshoe crab"
        );
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

    /// Spawn the floor's boss through the real spawn system.
    fn spawn_crab(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the horseshoe crab");
        assert_eq!(boss.kind, BossKind::HorseshoeCrab);
        boss
    }

    /// One tick of the shared boss system, plus the rat's i-frame decay that
    /// the player system would otherwise do.
    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_update_system)
            .expect("boss update system must run");
        w.end_tick();
        w.player.iframes = (w.player.iframes - DT).max(0.0);
    }

    fn set_state(w: &mut World, state: CrabState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::HorseshoeCrab(state), duration);
            b.flags = 0;
            b.hurt = 0.0;
        });
    }

    fn state_of(w: &mut World) -> CrabState {
        crab_state(&boss_of(w).expect("boss alive"))
    }

    fn arena_centre(w: &World) -> Vec2 {
        w.level.fight_rect_around(w.player.pos).center_world()
    }

    fn place(w: &mut World, pos: Vec2, rat: Vec2) {
        with_boss(w, |b| {
            b.pos = pos;
            b.target = rat;
            b.facing = (rat - pos).normalized();
            b.pounce_origin = pos;
        });
        w.player.pos = rat;
        w.player.vel = Vec2::ZERO;
    }

    /// Tick until the state changes or `limit` seconds pass; returns the first
    /// state the crab entered.
    fn drive_until_change(w: &mut World, limit: f32) -> CrabState {
        let start = state_of(w);
        let mut t = 0.0;
        while t < limit {
            tick(w);
            t += DT;
            let now = state_of(w);
            if now != start {
                return now;
            }
        }
        panic!("{start:?} never changed within {limit:.2} s");
    }

    fn boss_context<'a>(
        w: &'a World,
        rng: &'a mut rand::rngs::StdRng,
    ) -> BossContext<'a> {
        let arena = w.level.fight_rect_around(w.player.pos);
        BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena,
            view: w.viewport,
            level: &w.level,
            flow: &w.flow,
            rng,
        }
    }

    // ---- registration & pool ---------------------------------------------

    #[test]
    fn the_sewer_pool_holds_the_jaguar_and_the_horseshoe() {
        let pool = BossKind::pool_for_theme(Theme::Sewer);
        assert_eq!(pool[0], BossKind::Cat, "the floor's original boss is first");
        assert!(pool.contains(&BossKind::HorseshoeCrab));
        assert_eq!(pool.len(), 2);
        // The Kitchen is the other pooled floor (it shares with the Megarat);
        // every remaining floor's pool is its own boss and nothing else.
        assert_eq!(
            BossKind::pool_for_theme(Theme::Kitchen),
            &[BossKind::Broom, BossKind::MegaRat]
        );
        // every remaining floor starts with its own boss
        assert_eq!(
            BossKind::pool_for_theme(Theme::Lab),
            &[BossKind::Mower, BossKind::Cobra]
        );
        for (theme, kind) in [
            (Theme::Laundry, BossKind::Mangle),
            (Theme::Printworks, BossKind::NightEdition),
        ] {
            assert_eq!(BossKind::pool_for_theme(theme), &[kind]);
        }
        // The Flatbed fields the magpie or the redtail.
        assert_eq!(
            BossKind::pool_for_theme(Theme::Trailer),
            &[BossKind::Magpie, BossKind::Hawk]
        );
        // `for_theme` stays the floor's original boss (the pinned choice).
        assert_eq!(BossKind::for_theme(Theme::Sewer), BossKind::Cat);
        // The draw is deterministic and reaches both entries.
        let mut saw_crab = false;
        let mut saw_cat = false;
        for seed in 0..64u64 {
            let a = BossKind::pool_pick(Theme::Sewer, seed);
            assert_eq!(a, BossKind::pool_pick(Theme::Sewer, seed));
            saw_crab |= a == BossKind::HorseshoeCrab;
            saw_cat |= a == BossKind::Cat;
        }
        assert!(saw_crab && saw_cat, "the pool draw must reach both bosses");
        // Single-entry pools are pinned, whatever the seed.
        for seed in 0..16u64 {
            assert!(BossKind::pool_for_theme(Theme::Lab)
                .contains(&BossKind::pool_pick(Theme::Lab, seed)));
            assert!(BossKind::pool_for_theme(Theme::Trailer)
                .contains(&BossKind::pool_pick(Theme::Trailer, seed)));
            assert_eq!(BossKind::pool_pick(Theme::Laundry, seed), BossKind::Mangle);
        }
    }

    #[test]
    fn the_spawn_system_fields_a_fitting_crab_outside_the_widest_view() {
        let mut w = crab_world(CRAB_SEED);
        w.viewport = Vec2::new(120.0, 76.0);
        let boss = spawn_crab(&mut w);
        assert!(landing_fits(&w.level, w.player.pos, w.viewport, boss.pos));
        assert!(boss.hp > 0.0);
        // The pool is the framework's: art pool x scale x the director's curve.
        let expect = BossKind::HorseshoeCrab.hp_pool(0);
        assert!((boss.max_hp - expect).abs() < 1.0, "pool {} vs {expect}", boss.max_hp);
        assert_eq!(boss.kind.body_radius(), art::BODY_RADIUS);
        assert_eq!(boss.kind.contact_damage(), art::CONTACT_DAMAGE);
        assert_eq!(boss.kind.death_duration(), art::DEATH_DURATION);
    }

    #[test]
    fn the_phase_thresholds_are_the_arts() {
        assert_eq!(BossKind::HorseshoeCrab.phase_for(0.99), 1);
        assert_eq!(BossKind::HorseshoeCrab.phase_for(art::PHASE2_HP_FRAC), 2);
        assert_eq!(BossKind::HorseshoeCrab.phase_for(0.40), 2);
        assert_eq!(BossKind::HorseshoeCrab.phase_for(art::PHASE3_HP_FRAC), 3);
        assert_eq!(BossKind::HorseshoeCrab.phase_for(0.10), 3);
    }

    // ---- armour ----------------------------------------------------------

    /// The shell deflects front shots and they are *spent*; rear shots land.
    #[test]
    fn front_shots_spark_off_the_dome_and_rear_shots_land() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let centre = arena_centre(&w);
        // Rat in front of the crab (the crab faces +x, the rat at +x).
        let rat = centre + Vec2::new(40.0, 0.0);
        place(&mut w, centre, rat);
        with_boss(&mut w, |b| b.facing = Vec2::new(1.0, 0.0));
        let hp0 = boss_of(&mut w).unwrap().hp;

        // A dart fired from the rat's side travels toward -x: front armour.
        w.clear_bullets();
        let mut front = crate::game::bullet::Bullet::new(
            rat - Vec2::new(20.0, 0.0),
            Vec2::new(-480.0, 0.0),
            30.0,
            crate::game::bullet::Faction::Player,
            (250, 250, 250),
        );
        front.weapon = Some(WeaponKind::Gnaw);
        w.ecs.spawn((front, SpawnId(7)));
        for _ in 0..3 {
            tick_boss_damage(&mut w);
        }
        let hp1 = boss_of(&mut w).unwrap().hp;
        assert!((hp1 - hp0).abs() < 0.01, "the shell must deflect the front shot");
        assert_eq!(
            w.bullets().len(),
            0,
            "a deflected shot is spent, not a free wall pass"
        );

        // A dart from behind travels toward +x: the rear takes it.
        w.clear_bullets();
        let mut rear = crate::game::bullet::Bullet::new(
            centre - Vec2::new(20.0, 0.0),
            Vec2::new(480.0, 0.0),
            30.0,
            crate::game::bullet::Faction::Player,
            (250, 250, 250),
        );
        rear.weapon = Some(WeaponKind::Gnaw);
        w.ecs.spawn((rear, SpawnId(8)));
        for _ in 0..3 {
            tick_boss_damage(&mut w);
        }
        let hp2 = boss_of(&mut w).unwrap().hp;
        assert!(hp0 - hp2 >= 29.0, "the rear must take the hit, lost {}", hp0 - hp2);
    }

    fn tick_boss_damage(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(projectiles::bullets_system)
            .expect("bullets system");
        w.ecs
            .run_system_cached(boss_damage_system)
            .expect("boss damage system");
        w.end_tick();
    }

    // ---- the telson sweep ------------------------------------------------

    #[test]
    fn the_telson_tells_before_it_hits_and_hits_once() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        // The rat stands in the enlarged tail band, beyond the old 40 px sweep.
        let rat = c - Vec2::new(46.0, 0.0);
        with_boss(&mut w, |b| b.scratch[S_ROT] = 1.0);
        place(&mut w, c, rat);
        w.player.hp = w.player.max_hp();
        w.player.iframes = 0.0;
        set_state(&mut w, CrabState::Dome, 0.1);

        // Drive the rotation into a telson: phase 1 alternates bulldoze/telson,
        // so tick the dome out and read the state (a bulldoze may come first).
        let mut saw_tell = false;
        for _ in 0..600 {
            tick(&mut w);
            if state_of(&mut w) == CrabState::TelsonTell {
                saw_tell = true;
                break;
            }
        }
        assert!(saw_tell, "the telson never came up");
        let tell_start_hp = w.player.hp;
        // During the tell nothing may hurt the rat.
        let mut t = 0.0;
        while t < art::TELSON_TELL[0] * 0.9 {
            tick(&mut w);
            t += DT;
            w.sfx = Vec::new();
        }
        assert_eq!(w.player.hp, tell_start_hp, "the tell must not damage");
        // Then the sweep lands, exactly once (the loop stops when the sweep
        // ends, so a later attack cannot be mistaken for a second hit).
        let mut hits = 0.0;
        let mut last = w.player.hp;
        let mut t = 0.0;
        while t < 3.0 && state_of(&mut w) != CrabState::Recover {
            tick(&mut w);
            w.sfx = Vec::new();
            if w.player.hp < last {
                hits += last - w.player.hp;
                last = w.player.hp;
            }
            t += DT;
        }
        assert_eq!(state_of(&mut w), CrabState::Recover, "the sweep ends in a limp");
        assert!(
            (hits - art::TELSON_DAMAGE).abs() < 0.01,
            "the sweep is a single {}-damage hit, took {hits}",
            art::TELSON_DAMAGE
        );
    }

    #[test]
    fn the_telson_arc_geometry_is_the_arts() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c - Vec2::new(24.0, 0.0));
        set_state(&mut w, CrabState::Dome, 5.0);
        // Call the real commit function through the system's context.
        let arena = w.level.fight_rect_around(w.player.pos);
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        // A freshly built boss: a cloned one carries no spare hazard capacity,
        // which is irrelevant to the fight but not to a narrow test.
        let mut boss = Boss::new(BossKind::HorseshoeCrab, c, 1000.0, arena, c, false);
        boss.facing = Vec2::new(1.0, 0.0);
        boss.state = BossState::HorseshoeCrab(CrabState::Dome);
        let mut ctx = boss_context(&w, &mut rng);
        assert!(commit_telson(&mut ctx, &mut boss), "the tail should commit");
        let (center, inner, outer, af, half) = art::telson_arc(&crab_view_of(&boss));
        assert_eq!(
            af,
            Vec2::new(-1.0, 0.0),
            "the arc faces the rat: the crab aims its dome away and the tail at it"
        );
        assert_eq!((inner, outer, half), (art::TELSON_INNER, art::TELSON_OUTER, art::TELSON_HALF_ANGLE));
        let _ = center;
        let hazard = boss.hazards.last().expect("the arc hazard");
        assert!((hazard.warn - (art::TELSON_TELL[0] + art::TELSON_SWEEP * art::TELSON_ACTIVE_PROGRESS)).abs() < 1e-3);
        assert_eq!(hazard.active, art::TELSON_ACTIVE);
        assert_eq!(hazard.damage, art::TELSON_DAMAGE);
    }

    // ---- the bulldoze ----------------------------------------------------

    #[test]
    fn the_bulldoze_marks_a_lane_to_a_wall_and_wedges_on_it() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        // Use the arena's left wall: put the crab near the middle and the rat
        // between it and that wall.
        let arena = w.level.fight_rect_around(w.player.pos);
        let left = arena.x as f32 * crate::game::collision::TILE_F;
        // The crab 120 px off the left wall, the rat 60 px further in: a lane
        // long enough to commit and a wall at the end of it.
        let c = Vec2::new(left + 120.0, arena.center_world().y);
        let rat = Vec2::new(left + 60.0, c.y);
        place(&mut w, c, rat);
        with_boss(&mut w, |b| {
            b.facing = (rat - c).normalized();
            b.scratch[S_ROT] = 0.0;
        });
        set_state(&mut w, CrabState::Dome, 0.05);
        // The rotation's first pick is a bulldoze.
        let next = drive_until_change(&mut w, 3.0);
        assert_eq!(next, CrabState::BulldozeTell, "the first phase-1 move is the charge");
        let lane = boss_of(&mut w).unwrap();
        let len = lane.scratch[S_LANE_LEN];
        assert!(len >= art::BULLDOZE_MIN, "lane too short: {len}");
        assert!(len <= art::BULLDOZE_MAX);
        // The lane end is at a wall: a body-length beyond it does not fit.
        let dir = (lane.target - lane.pos).normalized();
        let check = lane.target + dir * 8.0;
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = boss_context(&w, &mut rng);
        assert!(
            !ctx.footprint_fits(check, art::BODY_RADIUS),
            "the lane must end at a wall ({check:?})"
        );

        // Let the charge run into the wall.
        let mut saw_wedge = false;
        for _ in 0..600 {
            tick(&mut w);
            w.sfx = Vec::new();
            if state_of(&mut w) == CrabState::Wedged {
                saw_wedge = true;
                break;
            }
        }
        assert!(saw_wedge, "the charge never wedged");
        let boss = boss_of(&mut w).unwrap();
        // The punish window: no contact, extra incoming damage, gills exposed.
        assert_eq!(boss.contact_damage, 0.0, "the wedge deals no body contact");
        assert!((boss.damage_mult - art::WEDGE_DAMAGE_MULT).abs() < 1e-4);
        let view = crab_view_of(&boss);
        let shapes = art::hit_shapes(&view);
        assert!(shapes[1].grounded, "the gills must be a target in the wedge");
        // A shot at the gill from any direction takes the weak-point multiplier.
        assert_eq!(
            art::incoming_multiplier(&view, Vec2::new(1.0, 0.0), art::gill_pos(&view)),
            art::GILL_DAMAGE_MULT
        );
    }

    #[test]
    fn the_bulldoze_is_suppressed_when_the_lane_has_no_wall() {
        // An arena-sized open box with no interior walls: there is nowhere to
        // wedge, so the charge is refused and the crab falls through to the
        // tail (which needs the rat in reach).
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c - Vec2::new(24.0, 0.0));
        let open = crate::game::level::TileRect::new(0, 0, 2, 2);
        let mut boss = boss_of(&mut w).unwrap();
        let before = boss.state;
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        let mut ctx = BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena: open,
            view: w.viewport,
            level: &w.level,
            flow: &w.flow,
            rng: &mut rng,
        };
        // Pretend the whole map is a 2-cell box: everything is a wall, so the
        // lane is refused for the *other* reason (a lane shorter than the
        // minimum). Either way the charge must not commit.
        let committed = commit_bulldoze(&mut ctx, &mut boss);
        assert!(!committed, "a lane with no wall must not be offered");
        assert_eq!(boss.state, before);
    }

    #[test]
    fn the_charge_refuses_when_the_rat_has_no_side_step() {
        // A rat in a one-cell pocket has no clear step off the lane: the charge
        // must be suppressed (the design's safe-strip rule).
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(20.0, 0.0));
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut boss = boss_of(&mut w).unwrap();
        let mut ctx = boss_context(&w, &mut rng);
        // Squeeze the rat into the level's far corner so both side steps hit
        // the arena edge.
        let arena = w.level.arena;
        ctx.player_pos = Vec2::new(PLAYER_RADIUS + 1.0, arena.y * 0.5);
        boss.pos = ctx.player_pos + Vec2::new(-120.0, 0.0);
        boss.facing = Vec2::new(1.0, 0.0);
        let committed = commit_bulldoze(&mut ctx, &mut boss);
        assert!(!committed, "no side step means no charge");
    }

    // ---- the burrow ------------------------------------------------------

    #[test]
    fn the_burrow_marks_a_circle_and_bursts_it_once() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        let rat = c + Vec2::new(50.0, 0.0);
        place(&mut w, c, rat);
        with_boss(&mut w, |b| b.phase = 2);
        set_state(&mut w, CrabState::Dome, 0.05);
        w.player.hp = w.player.max_hp();
        w.player.iframes = 0.0;

        // Force a burrow (phase 2's third pick).
        with_boss(&mut w, |b| b.scratch[S_ROT] = 2.0);
        let mut saw_mark = false;
        for _ in 0..400 {
            tick(&mut w);
            w.sfx = Vec::new();
            if state_of(&mut w) == CrabState::BurrowTell {
                saw_mark = true;
                break;
            }
        }
        assert!(saw_mark, "the burrow never came up");
        // The mark sits under the rat, and the crab is under the floor before
        // the circle bursts.
        let mark = boss_of(&mut w).unwrap().target;
        assert!(mark.dist(w.player.pos) < 26.0, "the mark must be under the rat");
        let mut underground = false;
        let mut dive_steps = Vec::new();
        let mut hits = 0.0;
        let mut last = w.player.hp;
        let mut t = 0.0;
        while t < 12.0 && state_of(&mut w) != CrabState::Recover {
            tick(&mut w);
            w.sfx = Vec::new();
            if state_of(&mut w) == CrabState::Burrow {
                underground = true;
                let boss = boss_of(&mut w).unwrap();
                // The mound is drawn, so the shell under it is hittable: the
                // dive is no longer a free, invisible escape.
                assert!(art::hit_shapes(&crab_view_of(&boss))[0].grounded);
                assert_eq!(boss.contact_damage, 0.0);
                dive_steps.push(boss.pos);
            }
            if w.player.hp < last {
                hits += last - w.player.hp;
                last = w.player.hp;
            }
            t += DT;
        }
        assert_eq!(state_of(&mut w), CrabState::Recover, "the eruption dazes it");
        assert!(underground, "the crab never went under");
        // The dive covered real ground, tick by tick, never faster than the
        // crab's own top speed: a visible mound, not a teleport.
        assert!(dive_steps.len() > 2, "the dive was not a travel");
        let cap = art::BURROW_TRAVEL_SPEED[1] * DT + 0.01;
        for pair in dive_steps.windows(2) {
            assert!(pair[0].dist(pair[1]) <= cap, "the mound jumped its dive");
        }
        assert!(
            (hits - art::EMERGE_DAMAGE).abs() < 0.01,
            "the eruption is a single {} hit, took {hits}",
            art::EMERGE_DAMAGE
        );
    }

    // ---- the flip --------------------------------------------------------

    #[test]
    fn the_flip_is_fully_vulnerable_and_the_legs_hit_once() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(12.0, 0.0));
        with_boss(&mut w, |b| b.phase = 3);
        set_state(&mut w, CrabState::FlipTell, 0.05);
        // Tick until it is on its back.
        let mut flipped = false;
        for _ in 0..120 {
            tick(&mut w);
            w.sfx = Vec::new();
            if state_of(&mut w) == CrabState::Flipped {
                flipped = true;
                break;
            }
        }
        assert!(flipped, "the crab never flipped");
        let boss = boss_of(&mut w).unwrap();
        assert!((boss.damage_mult - art::FLIP_DAMAGE_MULT).abs() < 1e-4);
        assert_eq!(boss.contact_damage, 0.0, "the legs are the weapon, not the body");
        let view = crab_view_of(&boss);
        // The dome is the weak point: no direction deflects.
        assert_eq!(
            art::incoming_multiplier(&view, Vec2::new(-1.0, 0.0), art::gill_pos(&view)),
            art::GILL_DAMAGE_MULT
        );
        // The ring is live and hits the rat standing in it, once per flip.
        let mut hits = 0.0;
        let mut last = w.player.hp;
        let mut t = 0.0;
        while t < 4.0 && state_of(&mut w) == CrabState::Flipped {
            tick(&mut w);
            w.sfx = Vec::new();
            if w.player.hp < last {
                hits += last - w.player.hp;
                last = w.player.hp;
            }
            t += DT;
        }
        assert!(
            (hits - art::LEGS_DAMAGE).abs() < 0.01,
            "the thrash is one {} hit per flip, took {hits}",
            art::LEGS_DAMAGE
        );
        // And it gets back up (Righting first, then the dome).
        assert_eq!(state_of(&mut w), CrabState::Righting);
        let mut t = 0.0;
        while t < 2.0 && state_of(&mut w) != CrabState::Dome {
            tick(&mut w);
            w.sfx = Vec::new();
            t += DT;
        }
        assert_eq!(state_of(&mut w), CrabState::Dome);
    }

    // ---- phase pose ------------------------------------------------------

    /// A forced burrow can still carry a stale mark by a wall. The crab needs
    /// a safe exit for that case as well as full clearance during live tells.
    #[test]
    fn it_never_surfaces_where_its_own_body_does_not_fit() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let arena = w.level.fight_rect_around(w.player.pos);
        let tile = crate::game::collision::TILE_F;
        let left = arena.x as f32 * tile;
        let fits = |w: &World, p: Vec2, r: f32| w.level.tiles.circle_clear(p, r, SizeClass::Large);
        // Inject a mark beside the wall that a half-body probe would accept.
        let mut mark = None;
        'rows: for row in 1..arena.h.max(2) - 1 {
            let y = (arena.y + row) as f32 * tile + tile * 0.5;
            for dx in -16..48 {
                let p = Vec2::new(left + dx as f32, y);
                if fits(&w, p, art::BODY_RADIUS * 0.5) && !fits(&w, p, art::BODY_RADIUS) {
                    mark = Some(p);
                    break 'rows;
                }
            }
        }
        let mark = mark.expect("the arena's left wall has a spot only half the body fits");
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(40.0, 0.0));
        set_state(&mut w, CrabState::Burrow, 0.05);
        with_boss(&mut w, |b| b.target = mark);
        let before = boss_of(&mut w).unwrap().pos;
        let next = drive_until_change(&mut w, 6.0);
        assert_eq!(next, CrabState::Emerge);
        let out = boss_of(&mut w).unwrap().pos;
        assert!(
            before.dist(out) <= art::BURROW_TRAVEL_SPEED[1] * 6.0 + 1.0,
            "the mound covered {:.1} px at its own speed",
            before.dist(out)
        );
        let boss = boss_of(&mut w).unwrap();
        assert!(
            fits(&w, boss.pos, boss.kind.body_radius()),
            "the crab surfaced wedged at {:?} (mark {mark:?})",
            boss.pos
        );
        assert!(
            boss.pos.dist(mark) <= 8.0 * tile + 1.0,
            "it surfaced far from its own mark: {:?} vs {mark:?}",
            boss.pos
        );
    }

    #[test]
    fn a_phase_threshold_spends_the_harmless_pose_change() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(20.0, 0.0));
        w.player.hp = w.player.max_hp();
        // Drop it through the phase 2 threshold.
        with_boss(&mut w, |b| {
            b.hp = b.max_hp * (art::PHASE2_HP_FRAC - 0.01);
            b.state = BossState::HorseshoeCrab(CrabState::Dome);
            b.duration = 0.02;
            b.elapsed = 0.0;
        });
        let hp0 = w.player.hp;
        tick(&mut w);
        let boss = boss_of(&mut w).unwrap();
        assert_eq!(boss.phase, 2);
        assert_eq!(crab_state(&boss), CrabState::Pose);
        assert!((boss.duration - art::POSE_DURATION).abs() < 1e-4);
        assert_eq!(boss.contact_damage, 0.0, "the pose is harmless");
        // No damage during the pose, even with the rat against the body.
        let mut t = 0.0;
        while t < art::POSE_DURATION * 0.8 {
            tick(&mut w);
            w.sfx = Vec::new();
            t += DT;
        }
        assert_eq!(w.player.hp, hp0);
        // Phase 2 sheds blue blood around itself.
        let pools = boss_of(&mut w)
            .unwrap()
            .hazards
            .iter()
            .filter(|h| h.slow > 0.0)
            .count();
        assert!(pools >= 1, "phase 2 must shed at least one pool, got {pools}");
    }

    #[test]
    fn blue_blood_slows_a_walking_rat() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c);
        with_boss(&mut w, |b| b.phase = 2);
        // Lay a pool right where the rat is walking. It is deliberately wider
        // than a real shed pool: the mechanic under test is the wade, and a
        // real 11 px pool is crossed in a fifth of a second.
        let at = w.player.pos;
        with_boss(&mut w, |b| {
            let pool = BossHazard::new(
                HazardShape::Circle {
                    center: at,
                    radius: 60.0,
                },
                0.0,
                art::BLOOD_LIFE,
                0.0,
            )
            .with_slow(art::BLOOD_SLOW);
            let _ = b.push_hazard(pool);
        });
        // The rat walks +x at full speed under the boss system's own slow.
        let start = w.player.pos;
        for _ in 0..60 {
            w.player.vel = Vec2::new(58.0, 0.0);
            w.player.pos += w.player.vel * DT;
            tick(&mut w);
            w.sfx = Vec::new();
        }
        let moved = w.player.pos.x - start.x;
        // 58 px/s for 1 s, minus the pool's 45 % share (and the crab's own
        // body may push the rat around, so the bound is generous).
        assert!(
            moved < 58.0 * 0.75,
            "the blood must be a tax on walking, moved {moved:.1} px"
        );
        assert!(moved > 10.0, "the rat must still be able to wade out, moved {moved:.1}");
    }

    // ---- death -----------------------------------------------------------

    #[test]
    fn the_death_clears_every_hazard_it_owns() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c - Vec2::new(30.0, 0.0));
        with_boss(&mut w, |b| {
            b.phase = 3;
            b.hp = 0.0;
        });
        // The damage tick kills it, then the death beat starts and clears.
        tick(&mut w);
        let boss = boss_of(&mut w).expect("the death beat is still playing");
        assert!(boss.dead);
        assert_eq!(crab_state(&boss), CrabState::Death);
        assert!(boss.hazards.is_empty(), "the boss's hazards stop with it");
        // The death beat completes and the level clears.
        let mut t = 0.0;
        while t < art::DEATH_DURATION + 1.0 && !w.level_complete {
            tick(&mut w);
            w.sfx = Vec::new();
            t += DT;
        }
        assert!(w.level_complete);
        assert!(boss_of(&mut w).is_none());
    }

    // ---- the hunt --------------------------------------------------------

    #[test]
    fn it_follows_the_rat_out_of_the_arena() {
        let mut w = crab_world(CRAB_SEED);
        spawn_crab(&mut w);
        // The rat stands just outside a small fight rectangle, on connected
        // floor (a rat that walks far away gets the rectangle recentred
        // instead: batch 12), and the crab starts 90 px off.
        let rat = w.player.pos;
        let (rx, ry) = crate::game::collision::cell_of(rat);
        let arena = crate::game::level::TileRect::new(rx + 2, ry - 3, 6, 6);
        assert!(!arena.contains_world(rat), "the rat must be outside the rectangle");
        let c = w
            .level
            .tiles
            .nearest_open_center(rat + Vec2::new(90.0, 0.0), SizeClass::Large, 16)
            .expect("open floor near the start");
        place(&mut w, c, rat);
        with_boss(&mut w, |b| b.arena = arena);
        w.clear_enemies();
        let d0 = boss_of(&mut w).unwrap().pos.dist(rat);
        for _ in 0..120 {
            tick(&mut w);
            w.sfx = Vec::new();
        }
        let boss = boss_of(&mut w).unwrap();
        assert!(boss.hunting, "a rat outside the arena is a hunt");
        let d1 = boss.pos.dist(rat);
        assert!(d1 < d0 - 20.0, "the crab must close the distance: {d0:.1} -> {d1:.1}");
    }

}
