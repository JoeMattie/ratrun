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
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::gem::Gem;
use crate::game::hazards::{clamp_outfall_displacement, outfall_sand_clear};
use crate::game::level::{Level, Theme};
use crate::game::player::Player;
use crate::math::Vec2;
use bevy_ecs::prelude::*;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

const FEINT_APPROACH_SECS: f32 = 1.0;
const MOUTH_OFFSET: f32 = 14.0;

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
    dark_used: bool,
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
            dark_used: false,
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

pub(super) fn relocate(boss: &mut Boss, level: &Level) {
    boss.lantern.pull = None;
    boss.hazards.clear();
    neutral(boss);
    if boss.lantern.planted {
        boss.target = boss.lantern.bulb.unwrap_or(boss.pos);
    } else {
        stow_lure(boss, level, art::STALK_LENGTH);
    }
}

fn face(from: Vec2, to: Vec2, fallback: Vec2) -> Vec2 {
    let dir = (to - from).normalized();
    if dir == Vec2::ZERO {
        fallback
    } else {
        dir
    }
}

/// Legal standing footprint for launch, rejoin, shoves and attack locks.
/// Pit rectangles are small, and the exact circle/rectangle test also checks
/// the head at the edge of a pit when its centre is still over sand.
pub fn head_fits(level: &Level, pos: Vec2) -> bool {
    point_fits(level, pos, HEAD_RADIUS, SizeClass::Large)
}

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
            .any(|pit| collision::circle_rect_overlap(pos, radius, &pit.rect.to_world()))
}

fn path_end(level: &Level, from: Vec2, step: Vec2, radius: f32, class: SizeClass) -> Vec2 {
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

pub(super) fn head_path_fits(level: &Level, from: Vec2, to: Vec2) -> bool {
    head_fits(level, to)
        && path_end(level, from, to - from, HEAD_RADIUS, SizeClass::Large).dist_sq(to) < 0.0001
}

fn stow_lure(boss: &mut Boss, level: &Level, reach: f32) {
    boss.lantern.bulb = Some(path_end(
        level,
        boss.pos,
        boss.facing * reach,
        art::BULB_RADIUS,
        SizeClass::Small,
    ));
}

fn move_lure(boss: &mut Boss, ctx: &BossContext<'_>, reach: f32, speed: f32) {
    let pos = boss.lantern.bulb.unwrap_or(boss.pos);
    let target = boss.pos + boss.facing * reach;
    let limit = (target - pos).clamp_len(speed * ctx.dt);
    let end = path_end(ctx.level, pos, limit, art::BULB_RADIUS, SizeClass::Small);
    // The stalk never crosses a wall or stretches beyond its tether as the
    // rat circles. Hold the last legal point when the bulb cannot follow.
    if end.dist_sq(boss.pos) <= art::LURE_TETHER_MAX * art::LURE_TETHER_MAX + 0.01
        && path_end(ctx.level, boss.pos, end - boss.pos, art::BULB_RADIUS, SizeClass::Small)
            .dist_sq(end)
            < 0.0001
    {
        boss.lantern.bulb = Some(end);
    }
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
    let center = bulb - facing * art::JAW_CRESCENT_BACK;
    let shape = rect(center, facing, art::JAW_CRESCENT_WIDTH * 0.5, art::JAW_CRESCENT_LEN * 0.5);
    attack(center, facing, shape)
}

fn gape(boss: &Boss, facing: Vec2) -> AttackLock {
    let mouth = boss.pos + facing * MOUTH_OFFSET;
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
    let shape = rect((origin + end) * 0.5, facing, origin.dist(end) * 0.5, width * 0.5);
    let mut lock = attack(origin, facing, shape);
    lock.end = end;
    lock
}

fn fits_rect(level: &Level, shape: HazardShape) -> bool {
    let HazardShape::Rect { center, facing, half, half_perp } = shape else {
        return false;
    };
    [-half_perp, 0.0, half_perp].into_iter().all(|across| {
        let edge = center + facing.perp() * across;
        let from = edge - facing * half;
        let to = edge + facing * half;
        point_fits(level, from, 0.25, SizeClass::Small)
            && path_end(level, from, to - from, 0.25, SizeClass::Small).dist_sq(to) < 0.0001
    })
}

fn leaves_escape(ctx: &BossContext<'_>, boss: &Boss, shape: HazardShape, pad: f32) -> bool {
    let rat = ctx.player_pos;
    if !shape.contains(rat, pad) {
        return true;
    }
    let separation = HEAD_RADIUS + ctx.player_radius;
    let separation_sq = separation * separation;
    [16.0, 28.0, 44.0, 60.0].into_iter().any(|distance| {
        (0..16).any(|i| {
            let delta = Vec2::from_angle(i as f32 * TAU / 16.0) * distance;
            let end = path_end(ctx.level, rat, delta, ctx.player_radius, SizeClass::Small);
            !shape.contains(end, pad)
                && (end.dist_sq(boss.pos) > separation_sq
                    || rat.dist_sq(boss.pos) <= separation_sq)
        })
    })
}

fn valid_gape(ctx: &BossContext<'_>, boss: &Boss, lock: AttackLock) -> bool {
    lock.stamp.is_some_and(|stamp| {
        fits_rect(ctx.level, stamp)
            && leaves_escape(ctx, boss, lock.shape, 0.0)
            && leaves_escape(ctx, boss, stamp, 0.0)
    })
}

fn body_path(ctx: &BossContext<'_>, boss: &Boss, facing: Vec2, reach: f32) -> Vec2 {
    path_end(ctx.level, boss.pos, facing * reach, HEAD_RADIUS, SizeClass::Large)
}

fn swim(ctx: &BossContext<'_>, boss: &mut Boss, speed: f32) {
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

fn snuff(ctx: &BossContext<'_>, boss: &mut Boss) {
    boss.lantern.next_snuff = boss.anim + art::SNUFF_INTERVAL;
    let mount = ctx.level.nearest_live_sconce(boss.pos, art::SNUFF_REACH);
    boss.lantern.snuff = mount.map(|(index, _)| index);
    let mount_pos = mount.map(|(_, pos)| pos).filter(|&pos| {
        point_fits(ctx.level, pos, 0.25, SizeClass::Small)
    });
    let Some(pos) = mount_pos.or_else(|| {
        [-1.0, 1.0].into_iter().find_map(|side| {
            let point = boss.pos + boss.facing.perp() * (side * 28.0);
            point_fits(ctx.level, point, art::BULB_RADIUS, SizeClass::Small).then_some(point)
        })
    }) else {
        return;
    };
    let away = mount_pos.map_or(boss.facing, |p| face(p, boss.pos, boss.facing));
    // A wall sconce is half a pixel inside its mounting face. Its visual
    // centre can leave that face even though the 4 px projectile target
    // overlaps it at birth.
    let reach = art::DECOY_SPEED * art::DECOY_LIFE;
    let mut end = pos;
    for angle in [0.0, -FRAC_PI_4, FRAC_PI_4, -2.0 * FRAC_PI_4, 2.0 * FRAC_PI_4] {
        let p = path_end(ctx.level, pos, away.rotate(angle) * reach, 0.25, SizeClass::Small);
        if p.dist_sq(pos) > end.dist_sq(pos) {
            end = p;
        }
        if end.dist_sq(pos) >= (reach - 0.01) * (reach - 0.01) {
            break;
        }
    }
    boss.lantern.decoy = Some(Decoy { pos, end, life: art::DECOY_LIFE });
}

fn advance_decoy(level: &Level, boss: &mut Boss, dt: f32) {
    if let Some(decoy) = &mut boss.lantern.decoy {
        let step = (decoy.end - decoy.pos).clamp_len(art::DECOY_SPEED * dt.min(decoy.life));
        decoy.pos = path_end(level, decoy.pos, step, 0.25, SizeClass::Small);
        decoy.life -= dt;
        if decoy.life <= 0.0 {
            boss.lantern.decoy = None;
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
    boss.lantern.next_snuff = boss.anim + art::SNUFF_INTERVAL;
    boss.lantern.next_feint = boss.anim;
    boss.lantern.attack = None;
    boss.leap_t = 0.0;
    if boss.phase == 3 {
        // A planted bulb is safe sand near the head even if the extended
        // lure was interrupted or had just gone dark.
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

fn move_on_lock(ctx: &BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(lock) = boss.lantern.attack else {
        return false;
    };
    let next = lock.origin + (lock.end - lock.origin) * boss.progress();
    if !head_path_fits(ctx.level, boss.pos, next) {
        boss.hazards.clear();
        return false;
    }
    boss.pos = next;
    true
}

fn stalk_lamp(ctx: &BossContext<'_>, boss: &mut Boss) {
    let gap = boss.pos.dist(ctx.player_pos);
    let lamp = boss.lantern.bulb.unwrap_or(boss.pos);
    let radial = face(lamp, boss.pos, boss.facing);
    let orbit = lamp + radial.rotate(0.28) * art::GAPE_ORBIT_DIST;
    let dir = if gap > art::GAPE_CONE_RANGE - 8.0 || !head_path_fits(ctx.level, boss.pos, orbit) {
        ctx.seek(boss)
    } else {
        (orbit - boss.pos).normalized()
    };
    if boss.leap_t <= 0.0 {
        move_boss(ctx, boss, dir, art::GAPE_SWIM_SPEED);
    }
    boss.facing = face(boss.pos, ctx.player_pos, boss.facing);
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
            commit_phase(ctx, boss);
            return;
        }

        advance_decoy(ctx.level, boss, ctx.dt);
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
                let dark_end = boss.pos + facing * art::DARK_LUNGE_LEN;
                if state == DarkSwim
                    && !boss.lantern.dark_used
                    && gap <= 70.0
                    && head_path_fits(ctx.level, boss.pos, dark_end)
                {
                    let lock = trench(boss.pos, dark_end, facing, art::DARK_LUNGE_WIDTH);
                    if leaves_escape(ctx, boss, lock.shape, ctx.player_radius) {
                        boss.lantern.dark_used = true;
                        boss.lantern.attack = Some(lock);
                        boss.leap_t = 0.0;
                        boss.facing = facing;
                        enter(boss, DarkTell, art::DARK_TELL_DURATION);
                        out.cue(BossCue::LanternTell);
                        return;
                    }
                }
                if state == DarkSwim {
                    if let Some(target) = boss.lantern.retaliation.take() {
                        // The target is captured by the projectile pass;
                        // the current neutral head supplies the start point.
                        let dir = face(boss.pos, target, facing);
                        let end = body_path(ctx, boss, dir, art::DECOY_LEAP_MAX_DIST);
                        let lock = trench(boss.pos, end, dir, art::DECOY_LEAP_WIDTH);
                        boss.leap_t = 0.0;
                        if boss.pos.dist(end) >= 24.0 && leaves_escape(ctx, boss, lock.shape, ctx.player_radius) {
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
                if boss.anim >= boss.lantern.next_feint
                    && gap <= art::LURE_TETHER_MAX + art::FEINT_STOP_SHORT
                    && ctx.los_clear(boss.pos, ctx.player_pos, art::BULB_RADIUS)
                {
                    boss.facing = facing;
                    boss.leap_t = 0.0;
                    stow_lure(boss, ctx.level, art::STALK_LENGTH.min((gap - art::FEINT_STOP_SHORT).max(20.0)));
                    enter(boss, FeintApproach, FEINT_APPROACH_SECS);
                } else {
                    swim(ctx, boss, art::SWIM_SPEED);
                }
            }
            FeintApproach => {
                boss.facing = facing;
                let reach = (gap - art::FEINT_STOP_SHORT).clamp(20.0, art::LURE_TETHER_MAX);
                move_lure(boss, ctx, reach, art::FEINT_LURE_SPEED);
                let bulb = boss.lantern.bulb.unwrap_or(boss.pos);
                if done || bulb.dist_sq(boss.pos + facing * reach) <= 0.25 {
                    let lock = jaw(boss);
                    if fits_rect(ctx.level, lock.shape) {
                        boss.lantern.attack = Some(lock);
                        enter(boss, FeintTell, art::FEINT_TELL_DURATION);
                        out.cue(BossCue::LanternTell);
                    } else {
                        boss.lantern.next_feint = boss.anim + 0.4;
                        neutral(boss);
                    }
                }
            }
            FeintTell => {
                if !boss.lantern.locked {
                    boss.facing = facing;
                    let reach = (gap - art::FEINT_STOP_SHORT).clamp(20.0, art::LURE_TETHER_MAX);
                    move_lure(boss, ctx, reach, art::FEINT_LURE_SPEED);
                    let lock = jaw(boss);
                    boss.lantern.attack = Some(lock);
                    if boss.elapsed >= art::FEINT_LOCK_TIME {
                        if !fits_rect(ctx.level, lock.shape)
                            || !leaves_escape(ctx, boss, lock.shape, ctx.player_radius)
                        {
                            boss.lantern.next_feint = boss.anim + 0.4;
                            neutral(boss);
                            return;
                        }
                        boss.facing = lock.facing;
                        boss.lantern.locked = true;
                    }
                }
                if done {
                    start_strike(boss, out, JawSnap, art::JAW_SNAP_DURATION, art::JAW_SNAP_DAMAGE, ctx.dt);
                }
            }
            JawSnap if done => {
                boss.lantern.next_feint = boss.anim + art::FEINT_COOLDOWN;
                enter(boss, FeintRecover, art::FEINT_RECOVER_DURATION);
            }
            FeintRecover | DecoyRecover | DarkRecover | GapeRecover if done => neutral(boss),
            DecoyLeapTell if done => {
                if let Some(lock) = boss.lantern.attack.filter(|lock| {
                    head_path_fits(ctx.level, boss.pos, lock.end)
                }) {
                    let secs = lock.origin.dist(lock.end) / art::DECOY_LEAP_SPEED;
                    start_strike(boss, out, DecoyLeap, secs, art::DECOY_LEAP_DAMAGE, ctx.dt);
                } else {
                    enter(boss, DecoyRecover, art::DECOY_RECOVER_DURATION);
                }
            }
            DecoyLeap => {
                let moved = move_on_lock(ctx, boss);
                stow_lure(boss, ctx.level, art::STALK_LENGTH);
                if !moved || done {
                    enter(boss, DecoyRecover, art::DECOY_RECOVER_DURATION);
                }
            }
            DarkTell => {
                if !boss.lantern.locked {
                    let end = boss.pos + facing * art::DARK_LUNGE_LEN;
                    if head_path_fits(ctx.level, boss.pos, end) {
                        let lock = trench(boss.pos, end, facing, art::DARK_LUNGE_WIDTH);
                        if leaves_escape(ctx, boss, lock.shape, ctx.player_radius) {
                            boss.lantern.attack = Some(lock);
                            boss.facing = facing;
                        }
                    }
                    if boss.elapsed >= art::DARK_TELL_DURATION * art::DARK_LOCK_PROGRESS {
                        boss.lantern.locked = true;
                    }
                }
                if done {
                    if boss.lantern.attack.is_some_and(|lock| head_path_fits(ctx.level, boss.pos, lock.end)) {
                        start_strike(boss, out, DarkLunge, art::DARK_LUNGE_DURATION, art::DARK_LUNGE_DAMAGE, ctx.dt);
                    } else {
                        enter(boss, DarkRecover, art::DARK_RECOVER_DURATION);
                    }
                }
            }
            DarkLunge => {
                if !move_on_lock(ctx, boss) || done {
                    enter(boss, DarkRecover, art::DARK_RECOVER_DURATION);
                    stow_lure(boss, ctx.level, art::STALK_LENGTH);
                }
            }
            DropLamp if done => neutral(boss),
            GapeSwim => {
                let lock = gape(boss, facing);
                if boss.anim >= boss.lantern.next_gape
                    && gap <= art::GAPE_CONE_RANGE + HEAD_RADIUS
                    && ctx.los_clear(lock.shape.center(), ctx.player_pos, 0.0)
                    && valid_gape(ctx, boss, lock)
                {
                    boss.lantern.next_gape = boss.anim + art::GAPE_COOLDOWN;
                    boss.lantern.attack = Some(lock);
                    boss.leap_t = 0.0;
                    boss.facing = facing;
                    enter(boss, GapeTell, art::GAPE_TELL_DURATION);
                    out.cue(BossCue::LanternTell);
                } else {
                    stalk_lamp(ctx, boss);
                }
            }
            GapeTell => {
                if !boss.lantern.locked {
                    let lock = gape(boss, facing);
                    if valid_gape(ctx, boss, lock) {
                        boss.facing = facing;
                        boss.lantern.attack = Some(lock);
                    }
                    if boss.elapsed >= art::GAPE_LOCK_TIME {
                        if !boss.lantern.attack.is_some_and(|lock| valid_gape(ctx, boss, lock)) {
                            neutral(boss);
                            return;
                        }
                        boss.lantern.locked = true;
                    }
                }
                if done {
                    if boss.lantern.attack.and_then(|lock| lock.stamp)
                        .is_some_and(|stamp| fits_rect(ctx.level, stamp))
                    {
                        // Carry only the part of the tick after the tell
                        // completed into the first pull.
                        let overflow = (boss.elapsed - boss.duration).clamp(0.0, ctx.dt);
                        enter(boss, GapePull, art::GAPE_PULL_DURATION);
                        boss.elapsed = overflow;
                        record_pull(boss, overflow);
                    } else {
                        neutral(boss);
                    }
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
                let away = face(ctx.player_pos, boss.pos, -boss.facing);
                move_boss(ctx, boss, away, art::GAPE_RECOVER_SPEED);
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

fn pull_point(level: &Level, cone: HazardShape, pos: Vec2, reach: f32, radius: f32) -> Vec2 {
    let mouth = cone.center();
    if !cone.contains(pos, 0.0) {
        return pos;
    }
    let step = (mouth - pos).clamp_len(reach);
    path_end(level, pos, step, radius, SizeClass::Small)
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
        if player.dash_active <= 0.0 {
            let scale = if player.buffs.is_giant() { 0.5 } else { 1.0 };
            player.pos = pull_point(&level, pull.cone, player.pos, reach * scale, PLAYER_RADIUS);
        }
        for mut gem in &mut gems {
            if !gem.magnetized {
                gem.pos = pull_point(&level, pull.cone, gem.pos, reach, 1.0);
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
    use crate::game::level::{Pit, PitKind, TileRect};
    use crate::game::nav::FlowField;
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn lantern_pull_and_head_travel_stop_on_the_dry_side_of_a_pit() {
        let mut level = Level::empty(Theme::Outfall);
        level.objects.tidal_pits.push(Pit::new(TileRect::new(50, 50, 1, 1), PitKind::TidalDrop));
        let from = Vec2::new(832.0, 808.0);
        let cone = HazardShape::Arc {
            center: Vec2::new(780.0, 808.0),
            facing: Vec2::new(1.0, 0.0),
            inner: 0.0,
            outer: art::GAPE_CONE_RANGE,
            half_angle: art::GAPE_CONE_HALF_ANGLE,
        };
        let to = pull_point(&level, cone, from, art::GAPE_PULL_SPEED, PLAYER_RADIUS);
        assert!(from.x > to.x && to.x > 816.0);
        assert!(outfall_sand_clear(&level, to, PLAYER_RADIUS, SizeClass::Small));
        assert!(!head_fits(&level, from), "the head overlaps the pit edge");
        assert!(!head_path_fits(&level, Vec2::new(836.0, 808.0), Vec2::new(780.0, 808.0)));
    }

    #[test]
    fn lantern_gape_fits_both_bearings_in_a_corridor_but_rejects_a_closed_end() {
        let mut level = Level::empty(Theme::Outfall);
        for y in [47, 51] {
            level.tiles.fill_cells(44, y, 15, 1, TileKind::Wall(WallKind::Solid));
        }
        level.tiles.rebuild_masks();
        let head = Vec2::new(800.0, 792.0);
        let arena = level.fight_rect_around(head);
        let boss = Boss::new(BossKind::Lantern, head, 1600.0, arena, head, false);
        let dir = Vec2::new(1.0, 0.0);
        let flow = FlowField::new(&level.tiles);
        let mut rng = StdRng::seed_from_u64(1);
        for facing in [dir, -dir] {
            let ctx = BossContext {
                dt: 1.0 / 60.0,
                time: 0.0,
                player_pos: head + facing * 60.0,
                player_vel: Vec2::ZERO,
                player_radius: PLAYER_RADIUS,
                arena,
                level: &level,
                flow: &flow,
                rng: &mut rng,
            };
            assert!(valid_gape(&ctx, &boss, gape(&boss, facing)));
        }
        level.tiles.fill_cells(51, 48, 1, 3, TileKind::Wall(WallKind::Solid));
        level.tiles.rebuild_masks();
        assert!(!fits_rect(&level, gape(&boss, dir).stamp.unwrap()));
    }
}
