//! The Outfall's anglerfish. The head is the only targetable part. Lures,
//! strike geometry and the pull share the controller's world-space locks.

use super::{
    move_boss, Boss, BossContext, BossController, BossCue, BossHazard, BossKind, BossOutput,
    BossState, HazardShape,
};
use crate::game::art_boss_lantern::{
    self as art, LanternBulbView, LanternState, LanternView, HEAD_RADIUS,
};
use crate::game::bullet::Bullet;
use crate::game::collision::{self, SizeClass};
use crate::game::gem::Gem;
use crate::game::hazards::{clamp_outfall_displacement, outfall_sand_clear};
use crate::game::level::{Level, Theme};
use crate::game::player::Player;
use crate::math::Vec2;
use bevy_ecs::prelude::*;
use std::f32::consts::FRAC_PI_2;

const FEINT_APPROACH_SECS: f32 = 1.0;
const FEINT_LURE_MIN: f32 = 20.0 * art::BODY_SCALE * art::LURE_PROPORTION;
const COOLDOWN_STANDOFF: f32 = art::MOUTH_OFFSET + 8.0;
const COOLDOWN_STEER_BAND: f32 = 16.0;
const FEINT_START_GAP: f32 = HEAD_RADIUS + 68.0;
const GAPE_START_GAP: f32 = art::MOUTH_OFFSET + 54.0;
const ATTACK_APPROACH_SPEED: f32 = 85.0;

#[derive(Clone, Copy, Debug)]
struct AttackLock {
    shape: HazardShape,
    facing: Vec2,
    origin: Vec2,
    end: Vec2,
    stamp: Option<HazardShape>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Decoy {
    pub(super) pos: Vec2,
    end: Vec2,
    life: f32,
}

#[derive(Clone, Copy, Debug)]
struct PullSlice {
    cone: HazardShape,
    secs: f32,
}

#[derive(Clone, Debug)]
pub(super) struct LanternRuntime {
    // The framework's HP phase advances immediately. This phase waits for
    // a committed move and its recovery to finish before resetting its locks.
    phase: u8,
    bulb: Option<Vec2>,
    planted: bool,
    attack: Option<AttackLock>,
    locked: bool,
    next_feint: f32,
    next_gape: f32,
    next_snuff: f32,
    next_dark: f32,
    pub(super) snuff: Option<usize>,
    pub(super) decoy: Option<Decoy>,
    retaliation: Option<Vec2>,
    pull: Option<PullSlice>,
}

impl Default for LanternRuntime {
    fn default() -> Self {
        Self {
            phase: 1,
            bulb: None,
            planted: false,
            attack: None,
            locked: false,
            next_feint: 0.0,
            next_gape: 0.0,
            next_snuff: 0.0,
            next_dark: 0.0,
            snuff: None,
            decoy: None,
            retaliation: None,
            pull: None,
        }
    }
}

pub(super) fn feint_armor(boss: &Boss, source_dir: Vec2) -> f32 {
    if boss.state == BossState::Lantern(LanternState::FeintRecover)
        && source_dir.normalized().dot(boss.facing) >= 0.5
    {
        art::FEINT_FRONT_DAMAGE_MULT
    } else {
        1.0
    }
}

fn neutral_state(phase: u8) -> LanternState {
    match phase {
        1 => LanternState::Swim,
        2 => LanternState::DarkSwim,
        _ => LanternState::GapeSwim,
    }
}

fn enter(boss: &mut Boss, state: LanternState, duration: f32) {
    boss.set_state(BossState::Lantern(state), duration);
}

fn neutral(boss: &mut Boss) {
    boss.lantern.attack = None;
    boss.lantern.locked = false;
    enter(boss, neutral_state(boss.lantern.phase), 1.0);
}

fn face(from: Vec2, to: Vec2, fallback: Vec2) -> Vec2 {
    let dir = (to - from).normalized();
    if dir == Vec2::ZERO {
        fallback
    } else {
        dir
    }
}

/// Keep the hittable head and the floating bulbs inside the map.
pub(super) fn clamp_to_map(level: &Level, pos: Vec2, radius: f32) -> Vec2 {
    let inset = Vec2::new(radius.min(level.arena.x * 0.5), radius.min(level.arena.y * 0.5));
    pos.clamp_box(inset, level.arena - inset)
}

pub(super) fn arrival_body_fits(level: &Level, head: Vec2, facing: Vec2) -> bool {
    let (lo, hi) = art::body_bounds_at(head, facing);
    lo.x >= 0.0 && lo.y >= 0.0 && hi.x <= level.arena.x && hi.y <= level.arena.y
        && clamp_to_map(level, head, HEAD_RADIUS) == head
}

/// Floor-bound displacement for the rat and gems caught by Gape.
fn point_fits(level: &Level, pos: Vec2, radius: f32, class: SizeClass) -> bool {
    let (cx, cy) = collision::cell_of(pos);
    if level.theme == Theme::Outfall {
        return level.tiles.passable(cx, cy, class) && outfall_sand_clear(level, pos, radius, class);
    }
    level.tiles.passable(cx, cy, class)
        && level.tiles.one_way_dir(cx, cy).is_none()
        && level.tiles.circle_clear(pos, radius, class)
        && level.pit_at(pos).is_none()
        && !level
            .trailer
            .pits
            .iter()
            .any(|pit| pit.touches(pos, radius))
}

fn pull_path_end(level: &Level, from: Vec2, step: Vec2, radius: f32, class: SizeClass) -> Vec2 {
    if level.theme == Theme::Outfall {
        return clamp_outfall_displacement(level, from, step, radius, class);
    }
    // A console can force this controller on any floor.
    let steps = (step.len() / 2.0).ceil().max(1.0) as u32;
    let mut end = from;
    for i in 1..=steps {
        let next = from + step * (i as f32 / steps as f32);
        if !point_fits(level, next, radius, class) {
            break;
        }
        end = next;
    }
    end
}

fn stow_lure(boss: &mut Boss, level: &Level, reach: f32) {
    let target = boss.pos + boss.facing * reach;
    boss.lantern.bulb = Some(clamp_to_map(level, target, art::BULB_RADIUS));
}

fn move_lure(boss: &mut Boss, ctx: &BossContext<'_>, reach: f32, speed: f32) {
    let pos = boss.lantern.bulb.unwrap_or(boss.pos);
    let target = clamp_to_map(ctx.level, boss.pos + boss.facing * reach, art::BULB_RADIUS);
    let next = pos + (target - pos).clamp_len(speed * ctx.dt);
    boss.lantern.bulb = Some(
        boss.pos + (next - boss.pos).clamp_len(art::LURE_TETHER_MAX),
    );
}

fn rect(center: Vec2, facing: Vec2, half: f32, half_perp: f32) -> HazardShape {
    HazardShape::Rect { center, facing, half, half_perp }
}

fn attack(origin: Vec2, facing: Vec2, shape: HazardShape) -> AttackLock {
    AttackLock { shape, facing, origin, end: origin, stamp: None }
}

fn jaw(boss: &Boss) -> AttackLock {
    let bulb = boss.lantern.bulb.unwrap_or(boss.pos);
    let facing = face(boss.pos, bulb, boss.facing);
    let center = art::jaw_center(boss.pos, facing);
    let shape = rect(center, facing, art::JAW_CRESCENT_WIDTH * 0.5, art::JAW_CRESCENT_LEN * 0.5);
    attack(center, facing, shape)
}

fn gape(boss: &Boss, facing: Vec2) -> AttackLock {
    let mouth = boss.pos + facing * art::MOUTH_OFFSET;
    let cone = HazardShape::Arc {
        center: mouth,
        inner: 0.0,
        outer: art::GAPE_CONE_RANGE,
        facing,
        half_angle: art::GAPE_CONE_HALF_ANGLE,
    };
    let mut lock = attack(mouth, facing, cone);
    let half_stamp = art::GAPE_STAMP_SIZE * 0.5;
    lock.stamp = Some(rect(mouth, facing, half_stamp, half_stamp));
    lock
}

fn trench(origin: Vec2, end: Vec2, facing: Vec2, width: f32) -> AttackLock {
    let mouth = facing * art::MOUTH_OFFSET;
    let shape = rect((origin + end) * 0.5 + mouth, facing, origin.dist(end) * 0.5, width * 0.5);
    let mut lock = attack(origin, facing, shape);
    lock.end = end;
    lock
}

fn body_path(ctx: &BossContext<'_>, boss: &Boss, facing: Vec2, reach: f32) -> Vec2 {
    let step = facing * reach;
    let allowed = clamp_to_map(ctx.level, boss.pos + step, HEAD_RADIUS) - boss.pos;
    let fraction = |d: f32, clipped: f32| if d == 0.0 { 1.0 } else { clipped / d };
    let t = fraction(step.x, allowed.x).min(fraction(step.y, allowed.y)).clamp(0.0, 1.0);
    boss.pos + step * t
}

fn close_to(ctx: &BossContext<'_>, boss: &mut Boss, facing: Vec2, gap: f32, reach: f32) {
    if gap > reach {
        let speed = ATTACK_APPROACH_SPEED.min((gap - reach) / ctx.dt);
        move_boss(ctx, boss, facing, speed);
    }
}

fn feint_reach(gap: f32) -> f32 {
    ((gap - art::FEINT_STOP_SHORT) * art::LURE_PROPORTION)
        .clamp(FEINT_LURE_MIN, art::LURE_TETHER_MAX)
}

fn swim(ctx: &BossContext<'_>, boss: &mut Boss, speed: f32) {
    if boss.anim < boss.lantern.next_feint {
        cooldown_swim(ctx, boss, speed);
        stow_lure(boss, ctx.level, art::STALK_LENGTH);
        return;
    }
    let dir = ctx.seek(boss);
    if dir != Vec2::ZERO {
        boss.facing = dir;
        // The pursuit system already moved a leaping boss this tick.
        if boss.leap_t <= 0.0 {
            move_boss(ctx, boss, dir, speed);
        }
    }
    stow_lure(boss, ctx.level, art::STALK_LENGTH);
}

fn cooldown_swim(ctx: &BossContext<'_>, boss: &mut Boss, speed: f32) {
    let facing = face(boss.pos, ctx.player_pos, boss.facing);
    let gap = boss.pos.dist(ctx.player_pos);
    let dir = if gap <= COOLDOWN_STANDOFF + COOLDOWN_STEER_BAND {
        let radial = ((gap - COOLDOWN_STANDOFF) / COOLDOWN_STEER_BAND).clamp(-2.0, 1.0);
        facing.perp() + facing * radial
    } else {
        ctx.seek(boss)
    };
    if boss.leap_t <= 0.0 {
        move_boss(ctx, boss, dir, speed);
    }
    boss.facing = facing;
}

fn snuff(ctx: &BossContext<'_>, boss: &mut Boss) {
    boss.lantern.next_snuff = boss.anim + art::SNUFF_INTERVAL;
    let mount = ctx.level.nearest_live_sconce(boss.pos, art::SNUFF_REACH);
    boss.lantern.snuff = mount.map(|(index, _)| index);
    let pos = mount.map_or_else(
        || clamp_to_map(
            ctx.level,
            boss.pos + boss.facing * (HEAD_RADIUS + 28.0),
            art::BULB_RADIUS,
        ),
        |(_, pos)| pos,
    );
    let toward_rat = face(pos, ctx.player_pos, boss.facing);
    let end = clamp_to_map(
        ctx.level,
        pos + toward_rat * (art::DECOY_SPEED * art::DECOY_LIFE),
        art::BULB_RADIUS,
    );
    boss.lantern.decoy = Some(Decoy { pos, end, life: art::DECOY_LIFE });
}

fn advance_decoy(boss: &mut Boss, dt: f32, rat: Vec2) {
    if let Some(decoy) = &mut boss.lantern.decoy {
        let step = (decoy.end - decoy.pos).clamp_len(art::DECOY_SPEED * dt.min(decoy.life));
        decoy.pos += step;
        decoy.life -= dt;
        if decoy.life <= 0.0 {
            boss.lantern.decoy = None;
            if boss.pos.dist(rat) <= boss.kind.engage_range() {
                boss.lantern.retaliation = Some(rat);
            }
        }
    }
}

/// First entrance into a circle along a projectile sweep. A projectile that
/// starts in a target has a contact time of zero. Head wins an exact tie.
fn circle_entry(from: Vec2, to: Vec2, center: Vec2, radius: f32) -> Option<f32> {
    let offset = from - center;
    let c = offset.len_sq() - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let step = to - from;
    let a = step.len_sq();
    let b = offset.dot(step);
    let discriminant = b * b - a * c;
    if a <= 1e-9 || b >= 0.0 || discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / a;
    (0.0..=1.0).contains(&t).then_some(t)
}

pub(super) fn try_pop_decoy(boss: &mut Boss, bullet: &Bullet, rat: Vec2) -> bool {
    let Some(decoy) = boss.lantern.decoy else {
        return false;
    };
    let Some(bulb_t) = circle_entry(bullet.prev_pos, bullet.pos, decoy.pos, art::DECOY_HIT_RADIUS + bullet.radius) else {
        return false;
    };
    let head_t = circle_entry(bullet.prev_pos, bullet.pos, boss.pos, HEAD_RADIUS + bullet.radius);
    if head_t.is_some_and(|time| time <= bulb_t) {
        return false;
    }
    boss.lantern.decoy = None;
    boss.lantern.retaliation = Some(rat);
    true
}

fn commit_phase(ctx: &BossContext<'_>, boss: &mut Boss) {
    boss.lantern.phase = boss.phase;
    boss.lantern.snuff = None;
    boss.lantern.decoy = None;
    boss.lantern.retaliation = None;
    boss.lantern.next_snuff = boss.anim + 1.2;
    boss.lantern.next_dark = boss.anim;
    boss.lantern.next_feint = boss.anim;
    boss.lantern.attack = None;
    boss.leap_t = 0.0;
    if boss.phase == 3 {
        // Plant the resting lure even if the extended one was interrupted.
        stow_lure(boss, ctx.level, art::STALK_LENGTH);
        boss.lantern.planted = true;
        boss.target = boss.lantern.bulb.unwrap_or(boss.pos);
        boss.lantern.next_gape = boss.anim + art::DROP_LAMP_DURATION + 0.65;
        enter(boss, LanternState::DropLamp, art::DROP_LAMP_DURATION);
    } else {
        neutral(boss);
    }
}

fn start_strike(
    boss: &mut Boss,
    out: &mut BossOutput,
    state: LanternState,
    secs: f32,
    damage: f32,
    dt: f32,
) {
    let Some(lock) = boss.lantern.attack else {
        neutral(boss);
        return;
    };
    enter(boss, state, secs);
    let shape = lock.stamp.unwrap_or(lock.shape);
    let mut hazard = BossHazard::new(shape, 0.0, secs, damage);
    if state == LanternState::GapeStamp {
        // Resolve on the next boss tick, after the schedule has applied the
        // last partial pull. Only the rat's centre can enter this stamp.
        hazard.elapsed = -dt;
        hazard.player_radius = 0.0;
    }
    boss.push_hazard(hazard);
    out.cue(BossCue::LanternStrike);
}

fn move_on_lock(boss: &mut Boss) {
    if let Some(lock) = boss.lantern.attack {
        boss.pos = lock.origin + (lock.end - lock.origin) * boss.progress();
    }
}

pub(super) struct LanternController;

impl BossController for LanternController {
    fn kind(&self) -> BossKind {
        BossKind::Lantern
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        use LanternState::*;
        let BossState::Lantern(state) = boss.state else {
            return;
        };
        boss.lantern.pull = None;
        if boss.phase > boss.lantern.phase
            && (state.is_neutral()
                || (matches!(state, FeintApproach | FeintTell | DarkTell | GapeTell)
                    && !boss.lantern.locked))
        {
            boss.lantern.attack = None;
            boss.lantern.decoy = None;
            boss.lantern.snuff = None;
            boss.leap_t = 0.0;
            boss.invuln = art::PHASE_SHIFT_DURATION;
            enter(boss, PhaseShift, art::PHASE_SHIFT_DURATION);
            out.cue(BossCue::LanternTell);
            return;
        }

        if state == PhaseShift {
            if boss.elapsed >= boss.duration {
                commit_phase(ctx, boss);
            } else {
                cooldown_swim(ctx, boss, art::SWIM_SPEED);
                stow_lure(boss, ctx.level, art::STALK_LENGTH);
            }
            return;
        }

        advance_decoy(boss, ctx.dt, ctx.player_pos);
        if boss.lantern.phase == 2 && boss.anim >= boss.lantern.next_snuff {
            snuff(ctx, boss);
        }

        let facing = face(boss.pos, ctx.player_pos, boss.facing);
        let gap = boss.pos.dist(ctx.player_pos);
        let done = boss.elapsed >= boss.duration;
        match state {
            ArrivalLure => {
                boss.facing = facing;
                move_lure(boss, ctx, art::ARRIVAL_LURE_LEAD, art::ARRIVAL_LURE_LEAD / art::ARRIVAL_DURATION);
                if done {
                    if boss.phase > boss.lantern.phase {
                        commit_phase(ctx, boss);
                    } else {
                        neutral(boss);
                    }
                }
            }
            Swim | DarkSwim => {
                if state == DarkSwim {
                    if let Some(target) = boss.lantern.retaliation.take() {
                        // The pop or expiry fixes the target. The current
                        // neutral head supplies the start of the warned leap.
                        let dir = face(boss.pos, target, facing);
                        let reach = boss.pos.dist(target).clamp(24.0, art::DECOY_LEAP_MAX_DIST);
                        let end = body_path(ctx, boss, dir, reach);
                        let lock = trench(boss.pos, end, dir, art::DECOY_LEAP_WIDTH);
                        boss.leap_t = 0.0;
                        if boss.pos.dist(end) >= 24.0 {
                            boss.facing = dir;
                            stow_lure(boss, ctx.level, art::STALK_LENGTH);
                            boss.lantern.attack = Some(lock);
                            boss.lantern.locked = true;
                            enter(boss, DecoyLeapTell, art::DECOY_LEAP_TELL_DURATION);
                            out.cue(BossCue::LanternTell);
                        } else {
                            enter(boss, DecoyRecover, art::DECOY_RECOVER_DURATION);
                        }
                        return;
                    }
                }
                if state == DarkSwim && boss.anim >= boss.lantern.next_dark
                    && gap <= 70.0 * art::BODY_SCALE
                {
                    let end = body_path(ctx, boss, facing, art::DARK_LUNGE_LEN);
                    if boss.pos.dist(end) >= 24.0 {
                        boss.lantern.next_dark = boss.anim + art::DARK_COOLDOWN;
                        boss.lantern.attack = Some(trench(boss.pos, end, facing, art::DARK_LUNGE_WIDTH));
                        boss.leap_t = 0.0;
                        boss.facing = facing;
                        enter(boss, DarkTell, art::DARK_TELL_DURATION);
                        out.cue(BossCue::LanternTell);
                        return;
                    }
                }
                if boss.anim >= boss.lantern.next_feint && gap <= FEINT_START_GAP {
                    boss.facing = facing;
                    boss.leap_t = 0.0;
                    stow_lure(boss, ctx.level, art::STALK_LENGTH.min(feint_reach(gap)));
                    enter(boss, FeintApproach, FEINT_APPROACH_SECS);
                } else {
                    swim(ctx, boss, art::SWIM_SPEED);
                }
            }
            FeintApproach | FeintTell => {
                if !boss.lantern.locked {
                    close_to(ctx, boss, facing, gap, art::MOUTH_OFFSET);
                    boss.facing = facing;
                    let reach = feint_reach(boss.pos.dist(ctx.player_pos));
                    move_lure(boss, ctx, reach, art::FEINT_LURE_SPEED);
                    let lock = jaw(boss);
                    boss.lantern.attack = Some(lock);
                    if state == FeintApproach {
                        if done || gap <= art::MOUTH_OFFSET + 18.0 {
                            enter(boss, FeintTell, art::FEINT_TELL_DURATION);
                            out.cue(BossCue::LanternTell);
                        }
                    } else if boss.elapsed >= art::FEINT_LOCK_TIME {
                        boss.facing = lock.facing;
                        boss.lantern.locked = true;
                    }
                }
                if state == FeintTell && done {
                    start_strike(boss, out, JawSnap, art::JAW_SNAP_DURATION, art::JAW_SNAP_DAMAGE, ctx.dt);
                }
            }
            JawSnap if done => {
                boss.lantern.next_feint = boss.anim + art::FEINT_COOLDOWN;
                enter(boss, FeintRecover, art::FEINT_RECOVER_DURATION);
            }
            FeintRecover | DecoyRecover | DarkRecover | GapeRecover if done => neutral(boss),
            DecoyLeapTell if done => {
                if let Some(lock) = boss.lantern.attack {
                    let secs = lock.origin.dist(lock.end) / art::DECOY_LEAP_SPEED;
                    start_strike(boss, out, DecoyLeap, secs, art::DECOY_LEAP_DAMAGE, ctx.dt);
                }
            }
            DecoyLeap => {
                move_on_lock(boss);
                stow_lure(boss, ctx.level, art::STALK_LENGTH);
                if done {
                    enter(boss, DecoyRecover, art::DECOY_RECOVER_DURATION);
                }
            }
            DarkTell => {
                if !boss.lantern.locked {
                    let end = body_path(ctx, boss, facing, art::DARK_LUNGE_LEN);
                    if boss.pos.dist(end) >= 24.0 {
                        boss.lantern.attack = Some(trench(boss.pos, end, facing, art::DARK_LUNGE_WIDTH));
                        boss.facing = facing;
                    }
                    if boss.elapsed >= art::DARK_TELL_DURATION * art::DARK_LOCK_PROGRESS {
                        boss.lantern.locked = true;
                    }
                }
                if done {
                    start_strike(boss, out, DarkLunge, art::DARK_LUNGE_DURATION, art::DARK_LUNGE_DAMAGE, ctx.dt);
                }
            }
            DarkLunge => {
                move_on_lock(boss);
                if done {
                    enter(boss, DarkRecover, art::DARK_RECOVER_DURATION);
                    stow_lure(boss, ctx.level, art::STALK_LENGTH);
                }
            }
            DropLamp if done => neutral(boss),
            GapeSwim => {
                if boss.anim >= boss.lantern.next_gape && gap <= GAPE_START_GAP {
                    boss.lantern.next_gape = boss.anim + art::GAPE_COOLDOWN;
                    boss.lantern.attack = Some(gape(boss, facing));
                    boss.leap_t = 0.0;
                    boss.facing = facing;
                    enter(boss, GapeTell, art::GAPE_TELL_DURATION);
                    out.cue(BossCue::LanternTell);
                } else if boss.anim < boss.lantern.next_gape {
                    cooldown_swim(ctx, boss, art::GAPE_SWIM_SPEED);
                } else {
                    if boss.leap_t <= 0.0 {
                        move_boss(ctx, boss, ctx.seek(boss), art::GAPE_SWIM_SPEED);
                    }
                    boss.facing = facing;
                }
            }
            GapeTell => {
                if !boss.lantern.locked {
                    close_to(ctx, boss, facing, gap, art::MOUTH_OFFSET + 8.0);
                    boss.facing = facing;
                    boss.lantern.attack = Some(gape(boss, facing));
                    if boss.elapsed >= art::GAPE_LOCK_TIME {
                        boss.lantern.locked = true;
                    }
                }
                if done {
                    // Carry only the part of the tick after the tell completed.
                    let overflow = (boss.elapsed - boss.duration).clamp(0.0, ctx.dt);
                    enter(boss, GapePull, art::GAPE_PULL_DURATION);
                    boss.elapsed = overflow;
                    record_pull(boss, overflow);
                }
            }
            GapePull => {
                let previous = (boss.elapsed - ctx.dt).max(0.0);
                record_pull(boss, (boss.elapsed.min(art::GAPE_PULL_DURATION) - previous).max(0.0));
                if done {
                    start_strike(boss, out, GapeStamp, art::GAPE_STAMP_DURATION, art::GAPE_STAMP_DAMAGE, ctx.dt);
                }
            }
            GapeStamp if done => enter(boss, GapeRecover, art::GAPE_RECOVER_DURATION),
            GapeRecover => {
                if gap < COOLDOWN_STANDOFF {
                    move_boss(ctx, boss, -facing, art::GAPE_RECOVER_SPEED);
                }
                boss.facing = face(boss.pos, ctx.player_pos, boss.facing);
            }
            _ => {}
        }
    }

    fn death_tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, _out: &mut BossOutput) {
        boss.lantern.snuff = None;
        boss.lantern.decoy = None;
        boss.lantern.retaliation = None;
        boss.lantern.pull = None;
        boss.lantern.attack = None;
        boss.hazards.clear();
        if boss.lantern.bulb.is_none() {
            stow_lure(boss, ctx.level, art::STALK_LENGTH);
        }
    }
}

fn record_pull(boss: &mut Boss, secs: f32) {
    if let Some(lock) = boss.lantern.attack {
        boss.lantern.pull = Some(PullSlice { cone: lock.shape, secs });
    }
}

fn pull_point(
    level: &Level,
    cone: HazardShape,
    pos: Vec2,
    reach: f32,
    radius: f32,
    class: SizeClass,
) -> Vec2 {
    let mouth = cone.center();
    if !cone.contains(pos, 0.0) {
        return pos;
    }
    let step = (mouth - pos).clamp_len(reach);
    pull_path_end(level, pos, step, radius, class)
}

/// Runs immediately after the boss controller. It consumes the frozen cone's
/// exact overlap with this tick and never mutates a grid-indexed enemy.
pub fn lantern_gape_system(
    level: Res<Level>,
    mut player: ResMut<Player>,
    mut gems: Query<&mut Gem>,
    mut bosses: Query<&mut Boss>,
) {
    for mut boss in &mut bosses {
        if boss.kind != BossKind::Lantern || boss.dead {
            continue;
        }
        let Some(pull) = boss.lantern.pull.take() else {
            continue;
        };
        let reach = art::GAPE_PULL_SPEED * pull.secs;
        if player.dash_active <= 0.0 && !level.objects.rat_in_crab_pot() {
            let scale = if player.buffs.is_giant() { 0.5 } else { 1.0 };
            let (radius, class) = player.movement_body(&level.tiles);
            player.pos = pull_point(&level, pull.cone, player.pos, reach * scale, radius, class);
        }
        for mut gem in &mut gems {
            if !gem.magnetized {
                gem.pos = pull_point(&level, pull.cone, gem.pos, reach, 1.0, SizeClass::Small);
            }
        }
    }
}

pub(super) fn view(boss: &Boss, state: LanternState) -> LanternView {
    use LanternState::*;
    let real_bulb = if matches!(state, DarkTell | DarkLunge | DarkRecover) {
        None
    } else {
        let radius = if boss.lantern.planted {
            art::BULB_PLANTED_LIGHT_RADIUS
        } else if matches!(state, FeintRecover | DecoyRecover) {
            art::BULB_RECOVER_LIGHT_RADIUS
        } else {
            art::BULB_LIGHT_RADIUS
        };
        let flicker = if state == FeintTell {
            let step = (boss.elapsed * art::LURE_FLICKER_HZ).floor();
            (1.0 + step % 2.0) * FRAC_PI_2
        } else {
            0.0
        };
        Some(LanternBulbView {
            pos: boss.lantern.bulb.unwrap_or(boss.pos),
            radius,
            gain: 1.0,
            flicker,
            is_decoy: false,
            planted: boss.lantern.planted,
        })
    };
    let decoy = boss.lantern.decoy.map(|d| LanternBulbView {
        pos: d.pos,
        radius: art::DECOY_LIGHT_RADIUS,
        gain: 1.0,
        flicker: 0.0,
        is_decoy: true,
        planted: false,
    });
    let visible = state.is_tell() || state.is_active_strike();
    let attack = boss.lantern.attack.filter(|_| visible);
    LanternView {
        pos: boss.pos,
        facing: boss.facing,
        anim: boss.anim,
        progress: boss.progress(),
        state,
        hit_flash: boss.hit_flash,
        hp_frac: boss.hp_frac(),
        phase: boss.lantern.phase,
        attack_origin: attack.map_or(boss.pos, |lock| lock.origin),
        attack_target: attack.map_or(boss.pos, |lock| lock.end),
        attack_facing: attack.map_or(boss.facing, |lock| lock.facing),
        attack_locked: attack.is_some() && boss.lantern.locked,
        attack: attack.filter(|_| state != GapeStamp).map(|lock| lock.shape),
        stamp: attack.and_then(|lock| lock.stamp),
        real_bulb,
        decoy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;
    use crate::game::ecs::constants::PLAYER_RADIUS;
    use crate::game::level::{Pit, PitKind, TileRect};
    use crate::game::nav::FlowField;
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn lantern_opens_distance_between_attacks() {
        let level = Level::empty(Theme::Outfall);
        let head = Vec2::new(800.0, 800.0);
        let rat = head + Vec2::new(COOLDOWN_STANDOFF - 12.0, 0.0);
        let arena = level.fight_rect_around(head);
        let mut boss = Boss::new(BossKind::Lantern, head, 1600.0, arena, head, false);
        let flow = FlowField::new(&level.tiles);
        let mut rng = StdRng::seed_from_u64(1);
        let ctx = BossContext {
            dt: 1.0 / 60.0,
            time: 0.0,
            player_pos: rat,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena,
            level: &level,
            flow: &flow,
            rng: &mut rng,
        };
        cooldown_swim(&ctx, &mut boss, art::SWIM_SPEED);
        assert!(boss.pos.dist(rat) > head.dist(rat));
        assert_eq!(clamp_to_map(&level, boss.pos, HEAD_RADIUS), boss.pos);
    }

    #[test]
    fn lantern_lure_extends_twice_as_far_in_proportion_to_its_head() {
        let level = Level::empty(Theme::Outfall);
        let head = Vec2::new(900.0, 900.0);
        let arena = level.fight_rect_around(head);
        let mut boss = Boss::new(BossKind::Lantern, head, 1600.0, arena, head, false);
        boss.facing = Vec2::new(1.0, 0.0);
        for reach in [art::STALK_LENGTH, art::ARRIVAL_LURE_LEAD] {
            stow_lure(&mut boss, &level, reach);
            assert_eq!(boss.lantern.bulb, Some(head + boss.facing * reach));
        }
        assert_eq!(art::STALK_LENGTH / art::BODY_SCALE, 56.0);
        assert_eq!(art::ARRIVAL_LURE_LEAD / art::BODY_SCALE, 72.0);
    }

    #[test]
    fn lantern_body_crosses_the_wall_and_pit_that_stop_a_gape_pull() {
        let mut level = Level::empty(Theme::Outfall);
        level.objects.tidal_pits.push(Pit::new(TileRect::new(50, 50, 1, 1), PitKind::TidalDrop));
        level.tiles.fill_cells(48, 48, 1, 5, TileKind::Wall(WallKind::Solid));
        level.tiles.rebuild_masks();
        let from = Vec2::new(832.0, 808.0);
        let facing = Vec2::new(-1.0, 0.0);
        let rat = Vec2::new(720.0, 808.0);
        let arena = level.fight_rect_around(from);
        let mut boss = Boss::new(BossKind::Lantern, from, 1600.0, arena, from, false);
        let flow = FlowField::new(&level.tiles);
        let mut rng = StdRng::seed_from_u64(1);
        let ctx = BossContext {
            dt: 1.0,
            time: 0.0,
            player_pos: rat,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena,
            level: &level,
            flow: &flow,
            rng: &mut rng,
        };
        assert_eq!(ctx.seek(&boss), facing);
        assert!(move_boss(&ctx, &mut boss, facing, 112.0));
        assert_eq!(boss.pos, rat);
        let cone = gape(&boss, -facing).shape;
        let to = pull_point(&level, cone, from, art::GAPE_PULL_SPEED, PLAYER_RADIUS, SizeClass::Small);
        assert!(from.x > to.x && to.x > 816.0);
        assert!(outfall_sand_clear(&level, to, PLAYER_RADIUS, SizeClass::Small));
    }
}
