//! Laundry boss — The Mangle (cast-iron wringer, two striped rollers).
//!
//! The art module [`crate::game::art_boss_mangle`] is the gameplay spec: its
//! state enum, every per-state duration, damage number, lane geometry and the
//! view struct are binding. This file only drives the transitions and queues
//! the art's telegraphed attacks as shared [`BossHazard`]s /
//! [`BossProjectile`]s; everything the rat sees is drawn from the art's pose
//! through [`Boss::view`].
//!
//! ## The fight (`WAVE3_DESIGN.md` §12)
//!
//! Three phases, one machine:
//!
//! | phase | rotation | the read |
//! |-------|----------|----------|
//! | 1 (> 60 %) | `Intake → Jam → Idle` | the lane outlines, the rollers feed, one 16 bite; the towel jams the nip for 1.5 s and the crank takes ×1.5 |
//! | 2 (60–25 %) | `Intake` alternating with `WringOut`, each into `Jam` | the crank clicks twice, the towel rolls down a fixed 56 × 10 lane; peel aside, then follow it back to the crank |
//! | 3 (< 25 %) | `WringOut` alternating with a **two-part** `Intake`, each into `FinalJam` | a fresh 0.9 s tell per lane and a 0.5 s break between bites; the final jam is 2.2 s and the slack rollers are unhittable, so the dropped crank is the target |
//!
//! The **pull is fair**: the drag is [`art_boss_mangle::PULL_SPEED`] (22 px/s)
//! against the rat's 58 px/s walk, and it only exists inside the painted lane.
//! The rat can always walk out along the lane or step across it.
//!
//! The **missed attack is the punish**: every attack ends in a jam (1.5 s, or
//! 2.2 s in phase 3) with body contact off and the crank exposed. A move whose
//! lane hits a wall or whose target cannot see the tell is suppressed: the
//! machine walks to a fresh angle and tries again in its next neutral beat.
//!
//! ## Determinism and allocation
//!
//! No rng draws at all: the phase-2/3 alternation is a flag, the phase-3 pair
//! is a flag, and every lane is aimed at the rat's position on the state-entry
//! tick. Nothing allocates — hazards go into the boss's pre-reserved ring and
//! the towel into the reused [`BossOutput`].
//!
//! ## Per-controller memory
//!
//! * `flags` — see the bits below (one-shot latches, the two-part pair and the
//!   intake/wring alternation). No `scratch` slot is needed.
//!
//! ## Feedback
//!
//! [`BossCue`]s: the backwards roller whirr on the intake tell, the nip's
//! metal bite, the towel jam, each ratchet click and the towel launch, the
//! phase-shift shudder and the spring-pop death.

use super::*;
use crate::game::art_boss_mangle as art;

/// The Mangle controller (zero-sized; the state lives on the [`Boss`]).
pub struct MangleController;

// ============================================================================
// Movement and pacing
// ============================================================================

/// The machine's waddle speed in its neutral beat (it is a heavy mangle).
const WALK_SPEED: f32 = 26.0;
/// Inside this distance the machine holds and attacks; outside it it walks.
const ATTACK_RANGE: f32 = 34.0;

/// The two ratchet clicks of the wring-out (seconds into the 0.90 s tell).
const WRING_CLICK_1: f32 = 0.45;
const WRING_CLICK_2: f32 = 0.90;

/// The death cue's beat (the springs pop at 20 % of the 2.40 s death).
const DEATH_CUE_PROGRESS: f32 = 0.20;

// ---- `Boss::flags` layout --------------------------------------------------

/// This beat already queued its hazards.
const ARMED_BIT: u32 = 1 << 0;
/// A phase-3 two-part intake has fired its first lane (the second is next).
const SECOND_BIT: u32 = 1 << 1;
/// This beat already played its first cue.
const CUE_BIT: u32 = 1 << 2;
/// The wring-out's second click already played.
const CUE2_BIT: u32 = 1 << 3;
/// The next phase-2/3 attack is the wring-out.
const WRING_NEXT_BIT: u32 = 1 << 4;
/// The death beat already cleared hazards and contact.
const DEATH_CLEARED_BIT: u32 = 1 << 8;
/// The death beat already played its spring-pop cue.
const DEATH_CUE_BIT: u32 = 1 << 9;

// ============================================================================
// Controller
// ============================================================================

impl BossController for MangleController {
    fn kind(&self) -> BossKind {
        BossKind::Mangle
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let aim = Aim::new(ctx.player_pos, boss);
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // A phase change is a harmless pose change, whatever the machine was
        // doing: the telegraphed attack is cancelled (a tell never fires from a
        // pose the rat can no longer see).
        if out.phase_changed
            && !matches!(
                boss.state,
                BossState::Mangle(art::MangleState::PhaseShift)
                    | BossState::Mangle(art::MangleState::Death)
            )
        {
            boss.hazards.clear();
            enter_harmless(boss, art::MangleState::PhaseShift, art::PHASE_SHIFT);
            out.cue(BossCue::MangleShift);
            return;
        }

        match boss.state {
            BossState::Mangle(art::MangleState::Idle) => {
                boss.facing = aim.toward;
                // Walk in from a fresh angle when the rat is out of reach (or
                // has left the arena entirely: the machine hunts on reachable
                // floor like the other two bosses).
                if boss.hunting || aim.dist > ATTACK_RANGE {
                    move_boss(ctx, boss, ctx.seek(boss), WALK_SPEED);
                }
                if boss.progress() >= 1.0 {
                    choose_and_begin(ctx, boss, &aim);
                }
            }

            BossState::Mangle(art::MangleState::Intake { second }) => {
                // The frame's front is an intake, not a body: during the feed
                // only the single bite deals damage. (Contact 8 is the
                // machine's walk and its wring-out pump, not the nip.)
                boss.no_contact();
                // The lane is locked on entry: facing and target stay put so
                // the paint, the hazards and the art agree.
                if !armed(boss) {
                    arm(boss);
                    push_intake_hazards(boss, art::intake_windows(boss.phase));
                    out.cue(BossCue::MangleRollers);
                }
                // The single bite crosses the nip at the end of the pull.
                let (tell, pull) = art::intake_windows(boss.phase);
                let bite_at = tell + pull;
                if previous_elapsed(boss, ctx.dt) < bite_at && boss.elapsed >= bite_at {
                    out.cue(BossCue::MangleBite);
                }
                if boss.progress() >= 1.0 {
                    if boss.phase >= 3 && !second {
                        // The second lane of the two-part intake: a fresh tell
                        // after the design's 0.5 s break, then the final jam.
                        boss.flags |= SECOND_BIT;
                        enter_harmless(boss, art::MangleState::Idle, art::INTAKE_BREAK);
                    } else if boss.phase >= 3 {
                        enter_harmless(boss, art::MangleState::FinalJam, art::FINAL_JAM);
                        boss.damage_mult = art::JAM_DAMAGE_MULT;
                        out.cue(BossCue::MangleJam);
                    } else {
                        enter_harmless(boss, art::MangleState::Jam, art::JAM);
                        boss.damage_mult = art::JAM_DAMAGE_MULT;
                        out.cue(BossCue::MangleJam);
                    }
                }
            }

            BossState::Mangle(art::MangleState::WringOut) => {
                if !armed(boss) {
                    arm(boss);
                    out.cue(BossCue::MangleCrank);
                }
                let e = boss.elapsed;
                if e >= WRING_CLICK_1 && boss.flags & CUE_BIT == 0 {
                    boss.flags |= CUE_BIT;
                    out.cue(BossCue::MangleCrank);
                }
                if e >= WRING_CLICK_2 && boss.flags & CUE2_BIT == 0 {
                    boss.flags |= CUE2_BIT;
                    out.cue(BossCue::MangleCrank);
                    spawn_towel(boss, out);
                }
                if boss.progress() >= 1.0 {
                    if boss.phase >= 3 {
                        enter_harmless(boss, art::MangleState::FinalJam, art::FINAL_JAM);
                    } else {
                        enter_harmless(boss, art::MangleState::Jam, art::JAM);
                    }
                    boss.damage_mult = art::JAM_DAMAGE_MULT;
                    out.cue(BossCue::MangleJam);
                }
            }

            BossState::Mangle(art::MangleState::Jam)
            | BossState::Mangle(art::MangleState::FinalJam) => {
                // The jam is the missed attack: no body contact, the loose
                // crank takes ×1.5 damage.
                boss.no_contact();
                boss.damage_mult = art::JAM_DAMAGE_MULT;
                if boss.progress() >= 1.0 {
                    go_idle(boss);
                }
            }

            BossState::Mangle(art::MangleState::PhaseShift) => {
                boss.no_contact();
                boss.damage_mult = 1.0;
                if boss.progress() >= 1.0 {
                    go_idle(boss);
                }
            }

            // The framework owns the death state.
            BossState::Mangle(art::MangleState::Death) => {}
            // A boss of another kind never runs this controller.
            _ => {}
        }
    }

    /// The death beat: "For the first time in thirty years, the laundry stops."
    ///
    /// The Mangle's hazards all belong to the boss, so they stop on its death
    /// (the shared hazard resolver would otherwise finish a bite that was
    /// already in flight). The springs pop once, halfway through the sag.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & DEATH_CLEARED_BIT == 0 {
            boss.flags |= DEATH_CLEARED_BIT;
            boss.hazards.clear();
            boss.no_contact();
            boss.damage_mult = 1.0;
        }
        if boss.progress() >= DEATH_CUE_PROGRESS && boss.flags & DEATH_CUE_BIT == 0 {
            boss.flags |= DEATH_CUE_BIT;
            out.cue(BossCue::MangleDeath);
        }
    }
}

// ============================================================================
// Per-tick inputs
// ============================================================================

/// The geometry one controller tick needs.
struct Aim {
    toward: Vec2,
    dist: f32,
}

impl Aim {
    fn new(player: Vec2, boss: &Boss) -> Self {
        let to = player - boss.pos;
        let toward = unit_or(to, boss.facing);
        Self {
            toward,
            dist: to.len(),
        }
    }
}

// ============================================================================
// Scratch helpers
// ============================================================================

#[inline]
fn armed(boss: &Boss) -> bool {
    boss.flags & ARMED_BIT != 0
}

#[inline]
fn arm(boss: &mut Boss) {
    boss.flags |= ARMED_BIT;
}

/// Seconds since the state clock was last at zero (the framework has already
/// advanced `elapsed` this tick).
#[inline]
fn previous_elapsed(boss: &Boss, dt: f32) -> f32 {
    (boss.elapsed - dt).max(0.0)
}

/// Enter a beat: reset its clock, its one-shots and the incoming multiplier.
fn enter(boss: &mut Boss, state: art::MangleState, duration: f32) {
    boss.set_state(BossState::Mangle(state), duration);
    boss.flags &= !(ARMED_BIT | CUE_BIT | CUE2_BIT);
    boss.damage_mult = 1.0;
}

/// Enter a recoverable beat: harmless from its very first tick (no body
/// contact, no incoming-damage cut).
fn enter_harmless(boss: &mut Boss, state: art::MangleState, duration: f32) {
    enter(boss, state, duration);
    boss.no_contact();
    boss.damage_mult = 1.0;
}

/// Neutral recovery beat for the current phase.
fn go_idle(boss: &mut Boss) {
    let phase = boss.phase;
    enter(boss, art::MangleState::Idle, art::idle_duration(phase));
}

// ============================================================================
// Attack selection
// ============================================================================

/// End of a neutral beat: pick the next attack, or walk if it does not fit.
fn choose_and_begin(ctx: &mut BossContext<'_>, boss: &mut Boss, aim: &Aim) {
    // The second lane of a phase-3 pair always starts first.
    if boss.flags & SECOND_BIT != 0 {
        boss.flags &= !SECOND_BIT;
        if intake_fits(ctx, boss, aim) {
            begin_intake(boss, aim, true);
        } else {
            go_idle(boss);
        }
        return;
    }

    let wring_next = boss.flags & WRING_NEXT_BIT != 0;
    let wants_wring = boss.phase >= 2 && wring_next;
    if wants_wring && wring_fits(ctx, boss, aim) {
        begin_wring(boss, aim);
        boss.flags ^= WRING_NEXT_BIT;
        return;
    }
    if intake_fits(ctx, boss, aim) {
        begin_intake(boss, aim, false);
        boss.flags ^= WRING_NEXT_BIT;
        return;
    }
    // Neither move is legal where the machine stands: walk to a fresh angle.
    // The neutral beat is also the walk, so this simply retries next beat.
    go_idle(boss);
}

// ============================================================================
// Fit tests — never start a move whose lane hits a wall / cannot be seen
// ============================================================================

/// A rat this far outside the lane can still step into the tell (the same
/// "walk into the telegraph" slack the broom's rake allows).
const INTAKE_FIT_SLACK: f32 = 10.0;

/// The intake lane has to be clear of walls for the rat's whole escape, the
/// machine has to fit where it stands, and the rat has to be able to see the
/// tell (no wall between the nip and the rat).
fn intake_fits(ctx: &BossContext<'_>, boss: &Boss, aim: &Aim) -> bool {
    let radius = boss.kind.body_radius();
    let view = boss.mangle_view();
    let lane = art::intake_lane(&view);
    let nip = art::nip_pos(&view);
    let end = nip + aim.toward * art::INTAKE_LANE_LEN;
    ctx.footprint_fits(boss.pos, radius)
        && ctx.los_clear(nip, end, art::ROLLER_HALF_WIDTH)
        && ctx.los_clear(nip, ctx.player_pos, ctx.player_radius)
        && lane.contains(ctx.player_pos, ctx.player_radius + INTAKE_FIT_SLACK)
}

/// The wring-out rail has to be clear of walls for its whole 56 px.
fn wring_fits(ctx: &BossContext<'_>, boss: &Boss, aim: &Aim) -> bool {
    let radius = boss.kind.body_radius();
    let view = boss.mangle_view();
    let nip = art::nip_pos(&view);
    let end = nip + aim.toward * art::TOWEL_LANE_LEN;
    ctx.footprint_fits(boss.pos, radius)
        && ctx.los_clear(nip, end, art::TOWEL_LANE_HALF_WIDTH)
        && ctx.los_clear(nip, ctx.player_pos, ctx.player_radius)
}

// ============================================================================
// Attack starts
// ============================================================================

/// `Intake`: lock the lane on the rat and outline it. The hazards carry the
/// tell, the pull and the bite, so the whole beat is one queue at entry.
fn begin_intake(boss: &mut Boss, aim: &Aim, second: bool) {
    boss.facing = aim.toward;
    let phase = boss.phase;
    let view = boss.mangle_view();
    boss.target = art::nip_pos(&view) + aim.toward * art::INTAKE_LANE_LEN;
    enter(
        boss,
        art::MangleState::Intake { second },
        art::intake_duration(phase),
    );
}

/// `WringOut`: the crank clicks twice and the towel is fired down the rail.
fn begin_wring(boss: &mut Boss, aim: &Aim) {
    boss.facing = aim.toward;
    let view = boss.mangle_view();
    boss.target = art::nip_pos(&view) + aim.toward * art::TOWEL_LANE_LEN;
    enter(
        boss,
        art::MangleState::WringOut,
        art::WRING_TELL + art::WRING_TRAVEL + art::WRING_RECOVER,
    );
}

// ============================================================================
// Hazards and projectiles
// ============================================================================

/// The pull corridor and the single bite, both on the locked lane.
fn push_intake_hazards(boss: &mut Boss, windows: (f32, f32)) {
    let (tell, pull) = windows;
    let view = boss.mangle_view();
    let lane = art::intake_lane(&view);
    let bite = art::bite_strip(&view);
    let f = lane.facing;

    // The conveyor: no damage, only the 22 px/s drag toward the nip.
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Rect {
                center: lane.center,
                facing: f,
                half: lane.half,
                half_perp: lane.half_perp,
            },
            tell,
            pull,
            0.0,
        )
        .with_drag(-f * art::PULL_SPEED),
    );
    // The bite: one hit, the last 8 px of the lane, at the end of the pull.
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Rect {
                center: bite.center,
                facing: f,
                half: bite.half,
                half_perp: bite.half_perp,
            },
            tell + pull,
            art::BITE_WINDOW,
            art::BITE_DAMAGE,
        )
        .with_enemies(),
    );
}

/// The rolled towel: fired from the nip down the locked rail.
fn spawn_towel(boss: &Boss, out: &mut BossOutput) {
    let view = boss.mangle_view();
    let f = art::unit_facing(&view);
    let mut spawn = BossProjectileSpawn::new(
        BossProjectileKind::RolledTowel,
        art::nip_pos(&view),
        f * art::TOWEL_SPEED,
    );
    // A rail is 56 px; whatever the wall does, the towel is spent by then.
    spawn.life = (art::TOWEL_LANE_LEN / art::TOWEL_SPEED) + 0.25;
    out.projectiles.push(spawn);
    out.cue(BossCue::MangleTowel);
}

// ============================================================================
// Small helpers
// ============================================================================

#[inline]
fn unit_or(v: Vec2, fallback: Vec2) -> Vec2 {
    let u = v.normalized();
    if u.len_sq() > 0.0 {
        u
    } else {
        fallback
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::bot::{Bot, Skill};
    use crate::game::collision::TileKind;
    use crate::game::director;
    use super::BossProjectile;
    use crate::game::ecs::systems::player::player_system;
    use crate::game::level::{Level, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::nav::{FlowField, NavBudget};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    const TICKS_PER_SEC: usize = 60;
    /// The empty grid's middle in world pixels (200 × 160 cells).
    const CENTER: Vec2 = Vec2 { x: 1600.0, y: 1280.0 };

    struct Rig {
        w: World,
        arena: TileRect,
    }

    impl Rig {
        fn new(seed: u64) -> Self {
            let mut w = World::new(Theme::Laundry, seed);
            w.level = Level::empty(Theme::Laundry);
            w.flow = FlowField::new(&w.level.tiles);
            w.flow.update(&w.level.tiles, CENTER, NavBudget::FULL);
            w.ecs.resource_mut::<BossFight>().rng = BossRng::for_run(seed, BossKind::Mangle);
            let arena = TileRect::new(0, 0, MAP_COLS, MAP_ROWS);
            let mut boss = Boss::new(
                BossKind::Mangle,
                CENTER,
                art::BOSS_BASE_HP,
                arena,
                arena.center_world(),
                false,
            );
            boss.id = 1;
            boss.facing = Vec2::new(1.0, 0.0);
            w.ecs.spawn((boss, SpawnId(0)));
            w.player.pos = CENTER + Vec2::new(24.0, 0.0);
            w.player.vel = Vec2::ZERO;
            Self { w, arena }
        }

        fn tick_with(&mut self, move_dir: Vec2) {
            self.w.begin_tick(DT, move_dir, false);
            self.w
                .ecs
                .run_system_cached(player_system)
                .expect("player system");
            self.w
                .ecs
                .run_system_cached(boss_update_system)
                .expect("boss update system");
            self.w.end_tick();
        }

        fn tick(&mut self) {
            self.tick_with(Vec2::ZERO);
        }

        fn boss(&mut self) -> Boss {
            let mut query = self.w.ecs.query::<&Boss>();
            query
                .iter(&self.w.ecs)
                .next()
                .cloned()
                .expect("the Mangle is in the world")
        }

        fn has_boss(&mut self) -> bool {
            let mut query = self.w.ecs.query::<&Boss>();
            query.iter(&self.w.ecs).next().is_some()
        }

        fn edit<R>(&mut self, f: impl FnOnce(&mut Boss) -> R) -> R {
            let mut query = self.w.ecs.query::<&mut Boss>();
            let mut iter = query.iter_mut(&mut self.w.ecs);
            let mut boss = iter.next().expect("the Mangle is in the world");
            f(&mut boss)
        }

        fn state(&mut self) -> art::MangleState {
            match self.boss().state {
                BossState::Mangle(s) => s,
                other => panic!("the Mangle is in a {other:?} state"),
            }
        }

        fn set_player(&mut self, pos: Vec2) {
            self.w.player.pos = pos;
            self.w.player.vel = Vec2::ZERO;
        }

        fn player_pos(&self) -> Vec2 {
            self.w.player.pos
        }

        fn hp(&self) -> f32 {
            self.w.player.hp
        }

        /// Paint solid rock over a block of cells (and refresh the masks).
        fn paint(&mut self, cx: i32, cy: i32, cw: i32, ch: i32) {
            self.w
                .level
                .tiles
                .fill_cells(cx, cy, cw, ch, TileKind::Wall(WallKind::Solid));
            self.w.level.tiles.rebuild_masks();
            let center = self.player_pos();
            self.w.flow = FlowField::new(&self.w.level.tiles);
            self.w.flow.update(&self.w.level.tiles, center, NavBudget::FULL);
        }

        fn clear(&mut self, cx: i32, cy: i32, cw: i32, ch: i32) {
            self.w.level.tiles.fill_cells(cx, cy, cw, ch, TileKind::Open);
            self.w.level.tiles.rebuild_masks();
            let center = self.player_pos();
            self.w.flow = FlowField::new(&self.w.level.tiles);
            self.w.flow.update(&self.w.level.tiles, center, NavBudget::FULL);
        }

        /// Run ticks until `done`; returns how many it took.
        fn run_until(&mut self, max: usize, mut done: impl FnMut(&mut Self) -> bool) -> usize {
            for i in 0..max {
                if done(self) {
                    return i;
                }
                self.tick();
            }
            max
        }

        /// Force a state with the art's duration (a test hook, like the broom's).
        fn force(&mut self, state: art::MangleState, duration: f32) {
            self.edit(|b| {
                b.facing = Vec2::new(1.0, 0.0);
                b.target = b.pos + Vec2::new(art::NIP_OFFSET + art::INTAKE_LANE_LEN, 0.0);
                enter(b, state, duration);
            });
        }

        fn projectiles(&mut self) -> usize {
            let mut query = self.w.ecs.query::<&BossProjectile>();
            query.iter(&self.w.ecs).count()
        }
    }

    fn hp_drop(before: f32, after: f32) -> f32 {
        (before - after).max(0.0)
    }

    // ---- the art contract ---------------------------------------------------

    #[test]
    fn the_move_set_is_the_designs() {
        assert_eq!(art::BITE_DAMAGE, 16.0);
        assert_eq!(art::TOWEL_DAMAGE, 18.0);
        assert_eq!(art::CONTACT_DAMAGE, 8.0);
        assert_eq!(art::PULL_SPEED, 22.0);
        assert_eq!(art::JAM_DAMAGE_MULT, 1.5);
        assert_eq!(art::INTAKE_LANE_LEN, 40.0);
        assert_eq!(art::ROLLER_HALF_WIDTH * 2.0, 22.0);
        assert_eq!(art::INTAKE_BITE_DEPTH, 8.0);
        assert_eq!(art::TOWEL_LANE_LEN, 56.0);
        assert_eq!(art::TOWEL_LANE_HALF_WIDTH * 2.0, 10.0);
        assert_eq!(BossKind::Mangle.contact_damage(), 8.0);
        assert_eq!(BossKind::Mangle.death_duration(), 2.40);
        // Phase thresholds: > 60 % / 60-25 % / < 25 %.
        assert_eq!(BossKind::Mangle.phase_for(0.95), 1);
        assert_eq!(BossKind::Mangle.phase_for(0.60), 2);
        assert_eq!(BossKind::Mangle.phase_for(0.26), 2);
        assert_eq!(BossKind::Mangle.phase_for(0.25), 3);
        // The Laundry fields its own boss, not the borrowed broom.
        assert_eq!(BossKind::for_theme(Theme::Laundry), BossKind::Mangle);
    }

    #[test]
    fn every_state_runs_for_its_art_duration() {
        let mut rig = Rig::new(7);
        rig.set_player(CENTER + Vec2::new(300.0, 0.0));
        let cases = [
            (art::MangleState::Idle, art::IDLE_P1),
            (
                art::MangleState::Intake { second: false },
                art::intake_duration(1),
            ),
            (art::MangleState::WringOut, art::WRING_TELL + art::WRING_TRAVEL + art::WRING_RECOVER),
            (art::MangleState::Jam, art::JAM),
            (art::MangleState::FinalJam, art::FINAL_JAM),
            (art::MangleState::PhaseShift, art::PHASE_SHIFT),
        ];
        for (state, duration) in cases {
            // The Idle case only ends when an attack can start: put the rat in
            // the lane for it. The rest are forced directly.
            if matches!(state, art::MangleState::Idle) {
                rig.set_player(CENTER + Vec2::new(22.0, 0.0));
            } else {
                rig.set_player(CENTER + Vec2::new(300.0, 0.0));
            }
            rig.force(state, duration);
            let start = rig.state();
            let ticks = rig.run_until(400, |r| r.state() != start);
            let ran = ticks as f32 * DT;
            assert!(
                (ran - duration).abs() <= 2.0 * DT + 1e-4,
                "{state:?} ran {ran:.3} s, the art says {duration:.2} s"
            );
        }
    }

    // ---- intake ------------------------------------------------------------

    #[test]
    fn the_intake_telegraphs_then_pulls_then_bites_once() {
        let mut rig = Rig::new(11);
        rig.set_player(CENTER + Vec2::new(30.0, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(1),
        );
        // Tick once so the controller queues its hazards.
        rig.tick();
        let boss = rig.boss();
        let pull = boss
            .hazards
            .iter()
            .find(|h| h.damage == 0.0)
            .copied()
            .expect("the intake queues the conveyor");
        assert!((pull.warn - art::INTAKE_TELL_P1).abs() < 1e-4);
        assert!((pull.active - art::INTAKE_PULL_P1).abs() < 1e-4);
        assert!((pull.drag.len() - art::PULL_SPEED).abs() < 1e-3);
        // The drag is toward the nip (-x for a +x facing).
        assert!(pull.drag.x < 0.0);
        let bite = boss
            .hazards
            .iter()
            .find(|h| h.damage > 0.0)
            .copied()
            .expect("the intake queues the bite");
        assert_eq!(bite.damage, art::BITE_DAMAGE);
        assert!((bite.warn - (art::INTAKE_TELL_P1 + art::INTAKE_PULL_P1)).abs() < 1e-4);
        assert!((bite.active - art::BITE_WINDOW).abs() < 1e-4);
        match bite.shape {
            HazardShape::Rect {
                half, half_perp, facing, ..
            } => {
                assert!((half * 2.0 - art::INTAKE_BITE_DEPTH).abs() < 1e-4);
                assert!((half_perp * 2.0 - 22.0).abs() < 1e-4);
                assert_eq!(facing, Vec2::new(1.0, 0.0));
            }
            other => panic!("the bite is a {other:?}, not a rect"),
        }

        // The tell: no damage and no pull (the hazard is queued on the
        // controller's first tick, so allow that one tick of framework drift).
        let start = rig.player_pos();
        let tick0 = 2usize;
        let mut hits = 0;
        let mut total = 0.0;
        let mut pulled_at = None;
        let mut x_at_pull_start = 0.0;
        for i in 0..((art::intake_duration(1) / DT) as usize + 4) {
            let before = rig.hp();
            let px = rig.player_pos().x;
            rig.tick();
            let drop = hp_drop(before, rig.hp());
            if drop > 0.0 {
                hits += 1;
                total += drop;
            }
            if i == tick0 {
                x_at_pull_start = px;
            }
            if i > tick0 && i < ((art::INTAKE_TELL_P1 - 3.0 * DT) / DT) as usize {
                assert!(
                    (rig.player_pos().x - px).abs() < 1e-4,
                    "the tell pulled the rat at {:.2} s",
                    i as f32 * DT
                );
            } else if i == ((art::INTAKE_TELL_P1 + 0.5) / DT) as usize {
                pulled_at = Some(x_at_pull_start - rig.player_pos().x);
            }
        }
        assert_eq!(hits, 1, "the bite must land exactly once, not {hits}");
        assert!((total - art::BITE_DAMAGE).abs() < 1e-3, "bite damage {total}");
        assert!(start.x > rig.player_pos().x, "the rat was never fed in");
        let pulled = pulled_at.expect("the pull started");
        assert!(
            (0.5 * art::PULL_SPEED * 0.5..0.5 * art::PULL_SPEED * 1.0 + 2.0).contains(&pulled),
            "half a second of pull moved {pulled:.1} px, expected ~{:.1}",
            0.5 * art::PULL_SPEED
        );
        // The bite is the missed attack's opening: the nip jams next.
        assert!(matches!(rig.state(), art::MangleState::Jam));
    }

    #[test]
    fn the_pull_can_always_be_out_walked_or_sidestepped() {
        // (a) Straight up the lane, away from the nip.
        let mut rig = Rig::new(13);
        rig.set_player(CENTER + Vec2::new(24.0, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(1),
        );
        let mut total = 0.0;
        for _ in 0..((art::intake_duration(1) / DT) as usize + 4) {
            let before = rig.hp();
            rig.tick_with(Vec2::new(1.0, 0.0));
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "walking up the lane still took {total}");

        // (b) Across the lane.
        let mut rig = Rig::new(17);
        rig.set_player(CENTER + Vec2::new(24.0, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(1),
        );
        let mut total = 0.0;
        for _ in 0..((art::intake_duration(1) / DT) as usize + 4) {
            let before = rig.hp();
            rig.tick_with(Vec2::new(0.0, 1.0));
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "stepping across the lane still took {total}");
        // The player really did leave the lane.
        assert!(
            (rig.player_pos().y - CENTER.y).abs() > art::ROLLER_HALF_WIDTH,
            "the rat did not clear the lane (y offset {:.1})",
            rig.player_pos().y - CENTER.y
        );
    }

    #[test]
    fn the_intake_body_is_harmless_so_the_bite_is_the_only_damage() {
        let mut rig = Rig::new(19);
        // Standing on the nip: the frame itself does not bump during the feed.
        rig.set_player(CENTER + Vec2::new(6.5, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(1),
        );
        let mut total = 0.0;
        for _ in 0..((art::INTAKE_TELL_P1 / DT) as usize) {
            let before = rig.hp();
            rig.tick();
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "the intake frame bumped for {total}");
    }

    // ---- wring-out ---------------------------------------------------------

    #[test]
    fn the_wring_out_clicks_twice_then_fires_one_towel() {
        let mut rig = Rig::new(23);
        rig.set_player(CENTER + Vec2::new(30.0, 0.0));
        rig.force(
            art::MangleState::WringOut,
            art::WRING_TELL + art::WRING_TRAVEL + art::WRING_RECOVER,
        );
        for _ in 0..((art::WRING_TELL - 0.1) / DT) as usize {
            rig.tick();
        }
        assert_eq!(rig.projectiles(), 0, "the towel left before the clicks");
        let mut seen_towel = false;
        let mut hits = 0;
        let mut total = 0.0;
        for _ in 0..((art::WRING_TRAVEL + art::WRING_RECOVER + 0.3) / DT) as usize {
            if !seen_towel {
                let mut query = rig.w.ecs.query::<&BossProjectile>();
                if let Some(p) = query.iter(&rig.w.ecs).next() {
                    assert_eq!(p.kind, BossProjectileKind::RolledTowel);
                    assert_eq!(p.damage, art::TOWEL_DAMAGE);
                    assert!(p.pos.dist(CENTER + Vec2::new(art::NIP_OFFSET, 0.0)) < 1.0);
                    seen_towel = true;
                }
            }
            let before = rig.hp();
            rig.tick();
            let drop = hp_drop(before, rig.hp());
            if drop > 0.0 {
                hits += 1;
                total += drop;
            }
        }
        assert!(seen_towel, "the towel never fired");
        assert_eq!(hits, 1, "the towel hit {hits} times");
        assert!((total - art::TOWEL_DAMAGE).abs() < 1e-3, "towel damage {total}");
        // Body contact only bumps the body; the rat was on the rail, not the frame.
        assert!(matches!(rig.state(), art::MangleState::Jam));
    }

    #[test]
    fn a_wall_stops_the_rolled_towel() {
        let mut rig = Rig::new(29);
        rig.set_player(CENTER + Vec2::new(30.0, 0.0));
        // A wall across the rail, 24 px past the nip.
        rig.paint(101, 77, 1, 7);
        rig.set_player(CENTER + Vec2::new(30.0, 0.0));
        rig.force(
            art::MangleState::WringOut,
            art::WRING_TELL + art::WRING_TRAVEL + art::WRING_RECOVER,
        );
        for _ in 0..(1.6 / DT) as usize {
            rig.tick();
        }
        assert_eq!(
            rig.projectiles(),
            0,
            "the towel must be spent on the wall it cannot reach past"
        );
    }

    // ---- the jam / the punish ----------------------------------------------

    #[test]
    fn the_jam_is_the_punish_window_and_the_crank_takes_extra() {
        let mut rig = Rig::new(31);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(1),
        );
        rig.run_until(400, |r| matches!(r.state(), art::MangleState::Jam));
        let boss = rig.boss();
        assert_eq!(boss.damage_mult, art::JAM_DAMAGE_MULT);
        assert_eq!(boss.contact_damage, 0.0, "the jam is harmless");
        assert!(boss.hazards.is_empty(), "no hazard survives the bite");
        // The crank is hittable and the frame is hittable (phase 1/2 jam).
        let shapes: Vec<_> = boss.hit_shapes().iter().copied().collect();
        assert!(shapes.iter().all(|s| s.grounded()));
        // Body contact ignores a rat standing on the machine during the jam.
        rig.set_player(CENTER);
        let mut total = 0.0;
        for _ in 0..((art::JAM - 0.2) / DT) as usize {
            let before = rig.hp();
            rig.tick();
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "the jam dealt {total}");
    }

    #[test]
    fn the_phase_three_pair_and_the_final_jam() {
        let mut rig = Rig::new(37);
        rig.edit(|b| {
            b.phase = 3;
            b.hp = b.max_hp * 0.2;
        });
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(3),
        );
        // The first lane hands over to a 0.5 s break, not a jam.
        rig.run_until(400, |r| matches!(r.state(), art::MangleState::Idle));
        let boss = rig.boss();
        assert_eq!(boss.flags & SECOND_BIT, SECOND_BIT, "the second lane is next");
        assert!((boss.duration - art::INTAKE_BREAK).abs() < 1e-4);
        // The break ends in the second lane (a fresh 0.9 s tell).
        rig.run_until(200, |r| matches!(r.state(), art::MangleState::Intake { .. }));
        // The arm tick queues the lane's hazards.
        rig.tick();
        let boss = rig.boss();
        assert_eq!(
            boss.state,
            BossState::Mangle(art::MangleState::Intake { second: true })
        );
        assert!((boss.duration - art::intake_duration(3)).abs() < 1e-4);
        let view = boss.mangle_view();
        let pull = boss
            .hazards
            .iter()
            .find(|h| h.damage == 0.0)
            .copied()
            .expect("the second lane queues its own conveyor");
        assert!((pull.warn - art::INTAKE_TELL_P3).abs() < 1e-4);
        assert!(
            pull.shape.center().dist(art::intake_lane(&view).center) < 1e-3,
            "the second lane is a fresh lock, not the first lane's"
        );
        // Then the long final jam, with the rollers slack and the crank out.
        rig.run_until(400, |r| matches!(r.state(), art::MangleState::FinalJam));
        let boss = rig.boss();
        assert!((boss.duration - art::FINAL_JAM).abs() < 1e-4);
        assert_eq!(boss.damage_mult, art::JAM_DAMAGE_MULT);
        assert_eq!(boss.contact_damage, 0.0);
        let shapes: Vec<_> = boss.hit_shapes().iter().copied().collect();
        assert!(!shapes[0].grounded(), "the slack rollers slip past ground weapons");
        assert!(shapes[1].grounded(), "the dropped crank is the target");
        let targets: Vec<Vec2> = boss.target_points().iter().collect();
        assert_eq!(targets.len(), 1, "only the crank is aimable");
        assert!(
            targets[0].dist(shapes[1].center()) < 1e-3,
            "auto-aim must find the crank's hit circle"
        );
        assert!(
            targets[0].dist(art::crank_pos(&boss.mangle_view())) < 1e-3,
            "the art's crank point is the hit point"
        );
    }

    // ---- phases -------------------------------------------------------------

    #[test]
    fn a_phase_change_is_a_harmless_pose_then_the_next_phase_rotation() {
        let mut rig = Rig::new(41);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.edit(|b| b.hp = b.max_hp * 0.55);
        rig.tick();
        assert_eq!(rig.boss().phase, 2);
        assert!(matches!(rig.state(), art::MangleState::PhaseShift));
        let boss = rig.boss();
        assert_eq!(boss.damage_mult, 1.0);
        assert_eq!(boss.contact_damage, 0.0);
        assert!(boss.hazards.is_empty());
        let ticks = rig.run_until(200, |r| !matches!(r.state(), art::MangleState::PhaseShift));
        assert!((ticks as f32 * DT - art::PHASE_SHIFT).abs() <= 2.0 * DT + 1e-4);
        // Phase 2 alternates the intake with the wring-out.
        let mut seen = Vec::new();
        let mut last = rig.state();
        for _ in 0..(14.0 * TICKS_PER_SEC as f32) as usize {
            rig.tick();
            let now = rig.state();
            if now != last
                && !matches!(now, art::MangleState::Idle | art::MangleState::Jam)
                && !seen.contains(&now)
            {
                seen.push(now);
            }
            last = now;
        }
        assert!(
            seen.contains(&art::MangleState::Intake { second: false }),
            "the intake never ran in phase 2: {seen:?}"
        );
        assert!(
            seen.contains(&art::MangleState::WringOut),
            "the wring-out never ran in phase 2: {seen:?}"
        );
        // And the low phase threshold.
        rig.edit(|b| b.hp = b.max_hp * 0.20);
        rig.tick();
        assert_eq!(rig.boss().phase, 3);
        assert!(matches!(rig.state(), art::MangleState::PhaseShift));
    }

    // ---- suppression --------------------------------------------------------

    #[test]
    fn a_lane_that_hits_a_wall_is_suppressed_and_the_machine_walks() {
        let mut rig = Rig::new(43);
        // A solid block right in front of the machine, with the rat beyond.
        rig.paint(103, 76, 2, 9);
        rig.set_player(CENTER + Vec2::new(90.0, 0.0));
        let start = rig.boss().pos;
        let mut saw_intake = false;
        for _ in 0..(3.0 * TICKS_PER_SEC as f32) as usize {
            rig.tick();
            if matches!(rig.state(), art::MangleState::Intake { .. }) {
                saw_intake = true;
            }
        }
        assert!(!saw_intake, "the machine fed a lane into a wall");
        let boss = rig.boss();
        assert!(
            boss.pos.x >= start.x - 0.5,
            "the machine must walk toward the rat, not stall on the wall"
        );
        // Clear the block and put the rat in the open: the intake starts.
        rig.clear(103, 76, 2, 9);
        let anchor = rig.boss().pos;
        rig.set_player(anchor + Vec2::new(24.0, 0.0));
        let mut ran = false;
        for _ in 0..(2.5 * TICKS_PER_SEC as f32) as usize {
            rig.tick();
            if matches!(rig.state(), art::MangleState::Intake { .. }) {
                ran = true;
                break;
            }
        }
        assert!(ran, "the intake never started on open floor");
    }

    #[test]
    fn a_walled_off_target_is_never_fed() {
        let mut rig = Rig::new(47);
        // The rat behind a full wall: no lane can be seen or reached.
        rig.paint(101, 72, 1, 17);
        rig.set_player(CENTER + Vec2::new(60.0, 0.0));
        for _ in 0..(4.0 * TICKS_PER_SEC as f32) as usize {
            rig.tick();
            assert!(
                !matches!(rig.state(), art::MangleState::Intake { .. }),
                "the machine fed a lane through a wall"
            );
        }
    }

    // ---- the hunt -----------------------------------------------------------

    #[test]
    fn the_mangle_hunts_on_reachable_floor_outside_its_arena() {
        let mut rig = Rig::new(53);
        // A small arena at the centre: the rat leaves it.
        let arena = TileRect::new(96, 74, 8, 8);
        rig.edit(|b| b.arena = arena);
        let outside = CENTER + Vec2::new(150.0, 0.0);
        rig.set_player(outside);
        rig.tick();
        assert!(rig.boss().hunting, "the rat is outside the arena");
        let start = rig.boss().pos;
        for _ in 0..TICKS_PER_SEC {
            rig.tick();
        }
        let boss = rig.boss();
        assert!(
            boss.pos.dist(outside) < start.dist(outside) - 6.0,
            "the machine did not travel toward the rat ({} -> {})",
            start.dist(outside),
            boss.pos.dist(outside)
        );
        assert!(
            rig.w
                .level
                .tiles
                .circle_clear(boss.pos, boss.kind.body_radius(), SizeClass::Large),
            "the hunt walked the machine into a wall"
        );
        // Back inside the arena: the fight resumes.
        rig.set_player(arena.center_world());
        rig.tick();
        assert!(!rig.boss().hunting);
    }

    // ---- death --------------------------------------------------------------

    #[test]
    fn hazards_stop_on_death_and_the_machine_stops_once() {
        let mut rig = Rig::new(59);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.force(
            art::MangleState::Intake { second: false },
            art::intake_duration(1),
        );
        rig.tick();
        assert!(!rig.boss().hazards.is_empty());
        // Kill it mid-tell: the queued lane must die with it.
        rig.edit(|b| b.hp = 0.0);
        rig.set_player(CENTER + Vec2::new(10.0, 0.0));
        let before = rig.hp();
        rig.tick();
        let boss = rig.boss();
        assert!(boss.dead);
        assert!(matches!(boss.state, BossState::Mangle(art::MangleState::Death)));
        assert!(boss.hazards.is_empty(), "the banked bite must not fire");
        assert!(boss.hit_shapes().iter().all(|s| !s.grounded()));
        let mut total = 0.0;
        let mut completions = 0;
        let mut was_complete = false;
        let mut death_ticks = 0;
        for i in 0..((art::DEATH_DURATION + 0.2) / DT) as usize {
            let drop = hp_drop(before, rig.hp());
            total += drop;
            let _ = drop;
            rig.tick();
            if rig.w.level_complete && !was_complete {
                completions += 1;
            }
            was_complete = rig.w.level_complete;
            if rig.has_boss() {
                death_ticks = i + 1;
            }
        }
        assert!(total == 0.0, "the death beat dealt {total}");
        assert_eq!(completions, 1, "the level completed {completions} times");
        assert!((death_ticks as f32 * DT - art::DEATH_DURATION).abs() <= 2.0 * DT + 1e-4);
        assert!(!rig.has_boss(), "the death beat reaps the machine");
        assert!(rig.w.ecs.resource::<BossFight>().level_complete);
    }

    // ---- determinism --------------------------------------------------------

    #[test]
    fn twin_runs_with_the_mangle_active_are_identical() {
        fn trace(seed: u64) -> Vec<(u8, u32, u32, u32, u32, u32, usize)> {
            let mut rig = Rig::new(seed);
            let mut out = Vec::new();
            for i in 0..(20 * TICKS_PER_SEC) {
                // A scripted rat: back away, then strafe across the lane.
                let boss = rig.boss();
                let to = rig.player_pos() - boss.pos;
                let dir = if i % 240 < 120 {
                    to.normalized()
                } else {
                    to.perp().normalized()
                };
                rig.tick_with(dir);
                let boss = rig.boss();
                let tag = match boss.state {
                    BossState::Mangle(art::MangleState::Idle) => 0,
                    BossState::Mangle(art::MangleState::Intake { second: false }) => 1,
                    BossState::Mangle(art::MangleState::Intake { second: true }) => 2,
                    BossState::Mangle(art::MangleState::WringOut) => 3,
                    BossState::Mangle(art::MangleState::Jam) => 4,
                    BossState::Mangle(art::MangleState::FinalJam) => 5,
                    BossState::Mangle(art::MangleState::PhaseShift) => 6,
                    BossState::Mangle(art::MangleState::Death) => 7,
                    _ => 8,
                };
                out.push((
                    tag,
                    boss.pos.x.to_bits(),
                    boss.pos.y.to_bits(),
                    boss.hp.to_bits(),
                    rig.w.player.hp.to_bits(),
                    boss.flags,
                    boss.hazards.len(),
                ));
            }
            out
        }
        for seed in [3, 91, 4242] {
            assert_eq!(trace(seed), trace(seed), "seed {seed} diverged");
        }
        // The fight is rng-free: different seeds play the same scripted fight.
        assert_eq!(trace(3), trace(91));
    }

    // ---- the bot wins -------------------------------------------------------

    /// A near-complete Laundry build, as the campaign carries one to 15:00.
    fn give_a_typical_build(w: &mut World) {
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::game::weapon::WeaponKind;
        {
            let lo = &mut w.player.loadout;
            for kind in [
                WeaponKind::Gnaw,
                WeaponKind::CheeseSpray,
                WeaponKind::SqueakNova,
                WeaponKind::TailWhip,
            ] {
                if !lo.has(kind) {
                    lo.apply(&Upgrade::NewWeapon(kind));
                }
            }
            for _ in 0..8 {
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
        }
        w.player.hp = w.player.max_hp();
    }

    #[test]
    fn the_bot_fights_the_mangle_to_the_death_on_its_real_arena() {
        let mut lengths = Vec::new();
        for seed in [11u64, 29, 1234, 77, 505] {
            let mut w = World::new(Theme::Laundry, seed);
            give_a_typical_build(&mut w);
            // The fight, not the 15-minute build-up: spawn the boss now (the
            // director's own hand-off and the horde break are the real path).
            w.elapsed = director::BOSS_TIME;
            w.update(DT, Vec2::ZERO, false);
            assert!(
                w.boss_hp().is_some(),
                "seed {seed}: the director never handed over the Mangle"
            );
            let kind = {
                let mut query = w.ecs.query::<&Boss>();
                query.iter(&w.ecs).next().map(|b| b.kind)
            };
            assert_eq!(kind, Some(BossKind::Mangle), "seed {seed}");
            // Start in the fight: batch 12 removed the arena, so the boss
            // arrives where the rat is and the fight measures nothing about a
            // walk. Put the rat on the boss's own ground.
            if let Some(pos) = w.boss_marker() {
                if let Some(open) = w
                    .level
                    .tiles
                    .nearest_open_center(pos, SizeClass::Small, 16)
                {
                    w.player.pos = open;
                }
            }
            let mut bot = Bot::new(Skill::Expert, seed);
            bot.on_new_level(0);
            // 300 s. This loop never drains `w.sfx`, and until the whip's boss
            // hit moved into its cast (`weapons::fire_one`) an undrained
            // `Sfx::ShootWhip` re-applied the whip to the boss on **every tick**,
            // so the old 120 s limit was measured on a 60x whip. Honest fights
            // with this build run 176-193 s.
            let limit = 60 * 300;
            let mut ticks = 0u32;
            let mut min_hp = w.player.hp;
            while ticks < limit && !w.level_complete && w.player.alive() {
                let input = bot.decide(&w, DT);
                w.update(DT, input.move_dir, input.dash);
                min_hp = min_hp.min(w.player.hp);
                ticks += 1;
            }
            assert!(
                w.player.alive(),
                "seed {seed}: the bot died to the Mangle (min hp {min_hp:.0}) after {:.0} s",
                ticks as f32 * DT
            );
            assert!(
                w.level_complete,
                "seed {seed}: the bot did not kill the Mangle in {} ticks ({:.0} s, boss hp {:?})",
                ticks,
                ticks as f32 * DT,
                w.boss_hp()
            );
            let seconds = ticks as f32 * DT;
            lengths.push(seconds);
            println!("mangle seed {seed}: killed in {seconds:.1} s");
        }
        let total: f32 = lengths.iter().sum();
        println!("mangle bot fights: {lengths:?} mean {:.1} s", total / lengths.len() as f32);
    }
}

