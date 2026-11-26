//! Geometry for the newer signature families.
//! Telegraphs, damage, bot steering and Library fitting use the same geometry.

use bevy_ecs::prelude::*;

use super::miniboss::{Miniboss, MinibossShape, MinibossState, SignatureShape, STAMP_STEP, TELL_LOCK};
use super::projectiles::los_clear;
use crate::detmath::DetF32;
use crate::game::art_walls::{wall_colliders, WallCellView};
use crate::game::collision::{cell_of, segment_rect_hit, TileCell, TileMap};
use crate::game::ecs::resources::{RatTrack, TickInput, RAT_TRACK_CAPACITY};
use crate::game::level::{Level, Rectf, Theme};
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

const RETURN_HUB: f32 = 8.0;
const SCONCE_REACH: f32 = 140.0;
const DRIP_MIDDLE: f32 = 0.25;
const FALSE_POP_RADIUS: f32 = 5.0;

pub const MILK_REACH: f32 = 92.0;
pub const MILK_HALF_ANGLE: f32 = std::f32::consts::PI / 10.0;
const MILK_SPOUT: f32 = 12.0;
pub const MILK_SPEED: f32 = 390.0;
const SPLASH_LIFE: f32 = 1.25;
// Along each axis the 36° sector spans no more than its 92 px reach.
// Eight 16 px tiles cover that span, even with padding for the hit radius.
const SPRAY_CELLS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MilkSpray {
    pub origin: Vec2,
    pub aim: Vec2,
    pub reach: f32,
    first: (i32, i32),
    obstructed: bool,
    walls: [[TileCell; SPRAY_CELLS]; SPRAY_CELLS],
}

impl MilkSpray {
    pub(super) fn open(body: Vec2, aim: Vec2) -> Self {
        Self {
            origin: body + aim * MILK_SPOUT,
            aim,
            reach: MILK_REACH,
            first: (0, 0),
            obstructed: false,
            walls: [[TileCell::OPEN; SPRAY_CELLS]; SPRAY_CELLS],
        }
    }

    pub(super) fn capture(tiles: &TileMap, body: Vec2, aim: Vec2) -> Self {
        let mut spray = Self::open(body, aim);
        if let Some(hit) = tiles.segment_hit(body, spray.origin, 0.0) {
            spray.origin = body + aim * (hit * MILK_SPOUT - 0.2).max(0.0);
        }
        let (lo, hi) = spray.bounds();
        spray.first = cell_of(lo - Vec2::splat(4.0));
        let last = cell_of(hi + Vec2::splat(4.0));
        debug_assert!(last.0 - spray.first.0 < SPRAY_CELLS as i32);
        debug_assert!(last.1 - spray.first.1 < SPRAY_CELLS as i32);
        for (y, row) in spray.walls.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                *cell = tiles.cell(spray.first.0 + x as i32, spray.first.1 + y as i32);
                spray.obstructed |= !cell.kind.is_open();
            }
        }
        spray
    }

    pub fn bounds(&self) -> (Vec2, Vec2) {
        sector_bounds(self.origin, self.aim, self.reach, MILK_HALF_ANGLE, 0.0)
    }

    /// First contact with the same permanent colliders that stop projectiles.
    fn wall_hit(&self, end: Vec2) -> Option<f32> {
        if !self.obstructed {
            return None;
        }
        let from = self.origin;
        // A ray exactly on a cell boundary can graze the wall on either side.
        let lower = Vec2::new(from.x.min(end.x), from.y.min(end.y)) - Vec2::splat(0.001);
        let (lo_x, lo_y) = cell_of(lower);
        let (hi_x, hi_y) = cell_of(Vec2::new(from.x.max(end.x), from.y.max(end.y)));
        let mut first: Option<f32> = None;
        for cy in lo_y.max(self.first.1)..=hi_y.min(self.first.1 + SPRAY_CELLS as i32 - 1) {
            let row = &self.walls[(cy - self.first.1) as usize];
            for cx in lo_x.max(self.first.0)..=hi_x.min(self.first.0 + SPRAY_CELLS as i32 - 1) {
                let cell = row[(cx - self.first.0) as usize];
                let Some(kind) = cell.kind.wall_kind() else { continue };
                // Collision uses only the cell coordinates, kind and mask.
                let view = WallCellView::new(cx, cy, 0, Theme::Kitchen, kind, cell.mask);
                let mut rects = [Rectf::new(0.0, 0.0, 0.0, 0.0); 2];
                let count = wall_colliders(&view, &mut rects);
                for rect in &rects[..count] {
                    if let Some(t) = segment_rect_hit(from, end, 0.0, rect) {
                        first = Some(first.map_or(t, |previous| previous.min(t)));
                    }
                }
            }
        }
        first
    }

    pub fn contains(&self, p: Vec2, radius: f32) -> bool {
        let offset = p - self.origin;
        let padded_reach = self.reach + radius;
        if offset.len_sq() > padded_reach * padded_reach {
            return false;
        }
        let along = offset.dot(self.aim);
        let across = offset.dot(self.aim.perp());
        let inside = along >= 0.0 && across.abs() <= along * MILK_HALF_ANGLE.dtan();
        if radius == 0.0 {
            return inside && self.wall_hit(p).is_none();
        }
        let nearest = if inside {
            if offset.len_sq() <= self.reach * self.reach {
                p
            } else {
                self.origin + offset.normalized() * self.reach
            }
        } else {
            let edge = self.aim.rotate(MILK_HALF_ANGLE.copysign(across));
            self.origin + edge * offset.dot(edge).clamp(0.0, self.reach)
        };
        p.dist_sq(nearest) <= radius * radius
            && self.wall_hit(nearest).is_none()
            && (nearest == p || self.wall_hit(p).is_none())
    }

    pub(super) fn deposit(
        &self, previous: f32, now: f32, elapsed: f32, splashes: &mut Vec<MilkSplash>,
    ) {
        if previous >= MILK_REACH / MILK_SPEED {
            return;
        }
        // Seven narrow streams break up at staggered distances. When a wall
        // intercepts one, deposit its first splash just short of the face.
        for i in 0..7 {
            let dir = self.aim.rotate((i as f32 - 3.0) * MILK_HALF_ANGLE * 0.28);
            let limit = self.wall_hit(self.origin + dir * self.reach)
                .map_or(self.reach, |hit| hit * self.reach - 2.0);
            let mut last = 0.0;
            for j in 0..4 {
                let dist = (23.0 + 19.0 * j as f32 + ((i * 17 + j * 7) % 13) as f32).min(limit);
                if dist < 5.0 || dist <= last {
                    break;
                }
                last = dist;
                let arrival = dist / MILK_SPEED;
                if previous < arrival && now >= arrival {
                    splashes.push(MilkSplash {
                        pos: self.origin + dir * dist,
                        size: 1.5 + ((i * 3 + j * 11) % 7) as f32 * 0.23,
                        born: elapsed - (now - arrival),
                    });
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MilkSplash {
    pub pos: Vec2,
    pub size: f32,
    born: f32,
}

impl MilkSplash {
    pub(super) fn expired(&self, elapsed: f32) -> bool {
        elapsed - self.born >= SPLASH_LIFE
    }

    pub fn opacity(&self, elapsed: f32) -> f32 {
        let age = elapsed - self.born;
        (age / 0.08).clamp(0.0, 1.0) * ((SPLASH_LIFE - age) / 0.55).clamp(0.0, 1.0)
    }
}

pub fn record_rat_track(input: Res<TickInput>, rat: Res<Player>, mut track: ResMut<RatTrack>) {
    track.record(input.dt, rat.pos);
}

/// The frozen polyline for Wake. It retains the intervening samples, plus
/// the interpolated endpoints of the requested time window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WakeTrail {
    points: [Vec2; RAT_TRACK_CAPACITY + 2],
    count: usize,
    lo: Vec2,
    hi: Vec2,
}

impl Default for WakeTrail {
    fn default() -> Self {
        Self { points: [Vec2::ZERO; RAT_TRACK_CAPACITY + 2], count: 0, lo: Vec2::ZERO, hi: Vec2::ZERO }
    }
}

impl WakeTrail {
    fn capture(track: &RatTrack, age: f32, window: f32) -> Self {
        let start = f64::from(age);
        let end = start + f64::from(window.clamp(0.0, 2.0));
        let mut trail = Self::default();
        trail.push(track.at_age(start));
        for (at, pos) in track.samples() {
            if at >= end {
                break;
            }
            if at > start {
                trail.push(pos);
            }
        }
        trail.push(track.at_age(end));
        trail
    }

    fn push(&mut self, pos: Vec2) {
        if self.count > 0 && self.points[self.count - 1] == pos {
            return;
        }
        if self.count == 0 {
            self.lo = pos;
            self.hi = pos;
        } else {
            self.lo = Vec2::new(self.lo.x.min(pos.x), self.lo.y.min(pos.y));
            self.hi = Vec2::new(self.hi.x.max(pos.x), self.hi.y.max(pos.y));
        }
        self.points[self.count] = pos;
        self.count += 1;
    }

    fn contains(&self, p: Vec2, reach: f32) -> bool {
        if self.count == 0
            || p.x < self.lo.x - reach || p.x > self.hi.x + reach
            || p.y < self.lo.y - reach || p.y > self.hi.y + reach
        {
            return false;
        }
        let reach_sq = reach * reach;
        if self.count == 1 {
            return p.dist_sq(self.points[0]) <= reach_sq;
        }
        self.points[..self.count].windows(2).any(|pair| {
            let (a, b) = (pair[0], pair[1]);
            p.x >= a.x.min(b.x) - reach && p.x <= a.x.max(b.x) + reach
                && p.y >= a.y.min(b.y) - reach && p.y <= a.y.max(b.y) + reach
                && segment_dist_sq(p, a, b) <= reach_sq
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SignatureTrack {
    trail: WakeTrail,
    drips: [Vec2; 3],
    locked: bool,
}

impl SignatureTrack {
    pub(super) fn start(shape: SignatureShape, pos: Vec2, track: &RatTrack) -> Self {
        let mut shot = Self { drips: [pos; 3], ..Self::default() };
        if track.is_empty() {
            shot.trail.push(pos);
        }
        shot.update(shape, 0.0, track);
        shot
    }

    /// At a frame crossing the lock, sample the lock instant inside that
    /// frame. Once captured, the track cannot follow later rat movement.
    pub(super) fn update(&mut self, shape: SignatureShape, timer: f32, track: &RatTrack) {
        if self.locked || track.is_empty() {
            return;
        }
        let locked_age = (timer - TELL_LOCK).max(0.0);
        match shape {
            SignatureShape::Wake { window, .. } => self.trail = WakeTrail::capture(track, locked_age, window),
            SignatureShape::Drip { .. } => {
                self.drips[1] = track.at_age(f64::from((timer - DRIP_MIDDLE).max(0.0)));
                self.drips[2] = track.at_age(f64::from(locked_age));
            }
            _ => {}
        }
        self.locked = timer >= TELL_LOCK;
    }
}

pub(super) fn drip_index(t: f32) -> u8 {
    ((t / STAMP_STEP).floor() as u8).min(2)
}

pub(super) fn umbra_aim(level: &Level, origin: Vec2, fallback: Vec2) -> Vec2 {
    let mut nearest = SCONCE_REACH * SCONCE_REACH;
    let mut aim = None;
    // Stable placement order breaks ties. Fixture art and lighting settings
    // do not participate in the gameplay decision.
    for lamp in &level.objects.lights {
        let away = origin - lamp.pos;
        let d = away.len_sq();
        if lamp.is_live_sconce()
            && d > 1e-6
            && (d < nearest || (aim.is_none() && d == nearest))
            && los_clear(level, origin, lamp.pos, 0.0)
        {
            nearest = d;
            aim = Some(away.normalized());
        }
    }
    aim.unwrap_or(fallback)
}

pub(super) fn locked_tell_hit(shape: SignatureShape, mb: &Miniboss) -> Option<MinibossShape> {
    if let SignatureShape::Wake { delay, .. } = shape {
        if mb.timer >= TELL_LOCK && mb.timer - TELL_LOCK >= delay {
            return Some(resolve(shape, mb, None));
        }
    }
    None
}

/// With no active time, resolve the whole warned footprint. Otherwise the
/// pair is (fraction of the active window, seconds into that window).
pub(super) fn resolve(
    shape: SignatureShape,
    mb: &Miniboss,
    active: Option<(f32, f32)>,
) -> MinibossShape {
    let origin = mb.origin;
    let aim = mb.aim.normalized();
    let tracking = if mb.state == MinibossState::Tell {
        (mb.timer / TELL_LOCK).clamp(0.0, 1.0)
    } else if mb.state == MinibossState::Approach {
        0.0
    } else {
        1.0
    };
    match shape {
        SignatureShape::Annulus { inner, outer } => MinibossShape::Annulus {
            origin: mb.rat_lock(), inner, outer,
        },
        SignatureShape::Tide { len, half, gap, travel } => {
            let drift = (len - gap).max(0.0) * 0.25;
            let center = drift * (2.0 * tracking - 1.0);
            let (from, to) = active.map_or((-half, travel + half), |(p, _)| {
                (travel * p - half, travel * p + half)
            });
            MinibossShape::Tide {
                origin, aim, len, from, to, gap: (center - gap * 0.5, center + gap * 0.5),
            }
        }
        SignatureShape::Return { len, half, sweep } => {
            let (aim, sweep) = active.map_or((aim, sweep), |(p, _)| {
                let pass = 1.0 - (p * 2.0 - 1.0).abs();
                (aim.rotate(sweep * (pass - 0.5)), 0.0)
            });
            MinibossShape::Return { origin, aim, len, half, sweep }
        }
        SignatureShape::Overlap { radius, sep } => {
            let offset = aim.perp() * (sep * 0.5);
            let target = mb.rat_lock();
            MinibossShape::Overlap { centers: [target - offset, target + offset], radius }
        }
        SignatureShape::Comb { len, half, tooth_gap, wide_gap } => {
            let wide_slot = if tracking >= 1.0 { 1 } else { 0 };
            let gaps = [-1, 0, 1].map(|slot| {
                let center = slot as f32 * len * 0.25;
                let width = if slot == wide_slot { wide_gap } else { tooth_gap };
                (center - width * 0.5, center + width * 0.5)
            });
            MinibossShape::Comb { origin: mb.rat_lock(), aim, len, half, gaps }
        }
        SignatureShape::Shear { len, half, channel } => {
            let blade = active.map(|(p, _)| ((channel * 0.5 - half).max(0.0) * (1.0 - 2.0 * p)).abs());
            MinibossShape::Shear { origin, aim, len, half, channel, blade }
        }
        SignatureShape::Wake { half, .. } => MinibossShape::Wake { trail: mb.signature_track.trail, half },
        SignatureShape::Drip { radius } => MinibossShape::Drip {
            centers: mb.signature_track.drips,
            radius,
            mask: active.map_or(0b111, |(_, t)| 1 << drip_index(t)),
        },
        SignatureShape::Umbra { half_angle, len } => MinibossShape::Umbra { origin, aim, half_angle, len },
        SignatureShape::MilkSpray => {
            let mut spray = mb.milk_spray();
            if let Some((_, time)) = active {
                spray.reach = (time * MILK_SPEED).min(MILK_REACH);
            }
            MinibossShape::MilkSpray(spray)
        }
        _ => unreachable!("legacy signature shape"),
    }
}

fn segment_dist_sq(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.len_sq().max(1e-8)).clamp(0.0, 1.0);
    p.dist_sq(a + ab * t)
}

fn rect_dist_sq(along: f32, across: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let x = along - along.clamp(a.0, a.1);
    let y = across - across.clamp(b.0, b.1);
    x * x + y * y
}

/// A bar with holes is a set of rectangles. Testing each solid interval also
/// accounts for a rat whose centre fits in a gap but whose body grazes a tooth.
fn slotted_bar(
    p: Vec2, aim: Vec2, len: f32, along: (f32, f32), slots: &[(f32, f32)], r: f32,
) -> bool {
    let x = p.dot(aim);
    let y = p.dot(aim.perp());
    let lo = -len * 0.5;
    let hi = len * 0.5;
    let mut start = lo;
    for &(a, b) in slots {
        let end = a.min(hi);
        if end > start && rect_dist_sq(x, y, along, (start, end)) <= r * r {
            return true;
        }
        start = start.max(b.clamp(lo, hi));
    }
    hi > start && rect_dist_sq(x, y, along, (start, hi)) <= r * r
}

pub(super) fn contains(shape: &MinibossShape, p: Vec2, r: f32) -> bool {
    match *shape {
        MinibossShape::Annulus { origin, inner, outer } => {
            let d = p.dist(origin);
            d + r >= inner && d <= outer + r
        }
        MinibossShape::Tide { origin, aim, len, from, to, gap } => {
            slotted_bar(p - origin, aim, len, (from, to), &[gap], r)
        }
        MinibossShape::Return { origin, aim, len, half, sweep } => {
            let radial = p - origin;
            let d = radial.len();
            if d + r < RETURN_HUB || d > len + half + r {
                return false;
            }
            let dir = if sweep > 0.0 {
                // At the bearing nearest this point, a cord in the swept
                // sector passes closest to it, including either rounded tip.
                let ray = Vec2::new(radial.dot(aim), radial.dot(aim.perp()));
                aim.rotate(ray.angle().clamp(-sweep * 0.5, sweep * 0.5))
            } else {
                aim
            };
            segment_dist_sq(p, origin, origin + dir * len) <= (half + r) * (half + r)
        }
        MinibossShape::Overlap { centers, radius } => {
            let [a, b] = centers.map(|c| p.dist(c));
            a.min(b) <= radius + r && a.max(b) + r > radius
        }
        MinibossShape::Comb { origin, aim, len, half, gaps } => {
            slotted_bar(p - origin, aim, len, (-half, half), &gaps, r)
        }
        MinibossShape::Shear { origin, aim, len, half, channel, blade } => {
            let x = (p - origin).dot(aim);
            let y = (p - origin).dot(aim.perp()).abs();
            let edge = channel * 0.5;
            let (lo, hi) = blade.map_or((0.0, edge), |b| ((b - half).max(0.0), (b + half).min(edge)));
            rect_dist_sq(x, y, (0.0, len), (lo, hi)) <= r * r
        }
        MinibossShape::Wake { ref trail, half } => trail.contains(p, half + r),
        MinibossShape::Drip { centers, radius, mask } => centers.iter().enumerate().any(|(i, c)| {
            mask & (1 << i) != 0 && p.dist_sq(*c) <= (radius + r) * (radius + r)
        }),
        MinibossShape::Umbra { origin, aim, half_angle, len } => {
            let v = p - origin;
            let d = v.len();
            if d > len + r {
                return false;
            }
            let dir = Vec2::new(v.dot(aim), v.dot(aim.perp()));
            let angle = dir.angle();
            if angle.abs() <= half_angle {
                return true;
            }
            let edge = aim.rotate(angle.clamp(-half_angle, half_angle));
            segment_dist_sq(p, origin, origin + edge * len) <= r * r
        }
        MinibossShape::False { aim, bulbs, selected, jaw_len, jaw_half, .. } => {
            bulbs.into_iter().enumerate().any(|(i, bulb)| {
                let offset = p - bulb;
                selected.is_none_or(|index| index == i)
                    && rect_dist_sq(
                        offset.dot(aim), offset.dot(aim.perp()),
                        (0.0, jaw_len), (-jaw_half, jaw_half),
                    ) <= r * r
            })
        }
        MinibossShape::MilkSpray(spray) => spray.contains(p, r),
        _ => unreachable!("legacy miniboss footprint"),
    }
}

impl MinibossShape {
    /// Bounds of the added shapes, used both to clip the rasterizer and to fit
    /// the Library plate. A half-open gap can only shrink these bounds.
    pub fn signature_bounds(&self) -> Option<(Vec2, Vec2)> {
        let (origin, aim, along, across) = match *self {
            Self::Annulus { origin, outer, .. } => return Some(disk_bounds(origin, outer)),
            Self::Tide { origin, aim, len, from, to, .. } => (origin, aim, (from, to), len * 0.5),
            Self::Return { origin, aim, len, half, sweep } => {
                return Some(sector_bounds(origin, aim, len, sweep * 0.5, half));
            }
            Self::Overlap { centers, radius } => return Some(disks_bounds(&centers, radius, 0b11)),
            Self::Comb { origin, aim, len, half, .. } => (origin, aim, (-half, half), len * 0.5),
            Self::Shear { origin, aim, len, channel, .. } => (origin, aim, (0.0, len), channel * 0.5),
            Self::Wake { ref trail, half } => return Some((trail.lo - Vec2::splat(half), trail.hi + Vec2::splat(half))),
            Self::Drip { centers, radius, mask } => return Some(disks_bounds(&centers, radius, mask)),
            Self::Umbra { origin, aim, len, half_angle } => {
                return Some(sector_bounds(origin, aim, len, half_angle, 0.0));
            }
            Self::False { origin, aim, bulbs, jaw_len, jaw_half, .. } => {
                let (mut lo, mut hi) = disks_bounds(&bulbs, FALSE_POP_RADIUS, 0b11);
                let cross = aim.perp() * jaw_half;
                // Keep both possible jaws in frame through commitment. The
                // dangerous subset comes from `contains`, independent of fit.
                for point in std::iter::once(origin).chain(bulbs.into_iter().flat_map(|bulb| {
                    let tip = bulb + aim * jaw_len;
                    [bulb - cross, bulb + cross, tip - cross, tip + cross]
                })) {
                    lo = Vec2::new(lo.x.min(point.x), lo.y.min(point.y));
                    hi = Vec2::new(hi.x.max(point.x), hi.y.max(point.y));
                }
                return Some((lo, hi));
            }
            Self::MilkSpray(spray) => return Some(spray.bounds()),
            _ => return None,
        };
        let center = origin + aim * ((along.0 + along.1) * 0.5);
        let len = (along.1 - along.0) * 0.5;
        let span = Vec2::new(aim.x.abs() * len + aim.y.abs() * across, aim.y.abs() * len + aim.x.abs() * across);
        Some((center - span, center + span))
    }
}

fn disk_bounds(center: Vec2, radius: f32) -> (Vec2, Vec2) {
    (center - Vec2::splat(radius), center + Vec2::splat(radius))
}

fn disks_bounds(centers: &[Vec2], radius: f32, mask: u8) -> (Vec2, Vec2) {
    let mut selected = centers.iter().enumerate().filter(|(i, _)| mask & (1 << *i) != 0);
    let (_, first) = selected.next().expect("a signature must have a circle");
    let (mut lo, mut hi) = disk_bounds(*first, radius);
    for (_, center) in selected {
        let (a, b) = disk_bounds(*center, radius);
        lo = Vec2::new(lo.x.min(a.x), lo.y.min(a.y));
        hi = Vec2::new(hi.x.max(b.x), hi.y.max(b.y));
    }
    (lo, hi)
}

fn sector_bounds(origin: Vec2, aim: Vec2, len: f32, half_angle: f32, pad: f32) -> (Vec2, Vec2) {
    let mut lo = origin;
    let mut hi = origin;
    let mut grow = |dir| {
        let p = origin + dir * len;
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    };
    grow(aim.rotate(-half_angle));
    grow(aim.rotate(half_angle));
    for dir in [Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)] {
        if Vec2::new(dir.dot(aim), dir.dot(aim.perp())).angle().abs() <= half_angle {
            grow(dir);
        }
    }
    (lo - Vec2::splat(pad), hi + Vec2::splat(pad))
}

/// Rasterize the boundary on its dangerous side. Holes, the lens, slots and
/// the lamp side stay open because this samples the same predicate as damage.
/// The ink pass uses the same procedure at its own pixel scale.
pub fn draw_outline(
    pb: &mut PixelBuffer, shape: &MinibossShape, cam: Vec2, scale: Vec2, colour: Rgb,
) {
    if let MinibossShape::Drip { centers, radius, mask: 0b111 } = *shape {
        for (i, color) in [colour, (254, 184, 119), (225, 135, 98)].into_iter().enumerate() {
            let circle = MinibossShape::Drip { centers, radius, mask: 1 << i };
            draw_outline(pb, &circle, cam, scale, color);
        }
        return;
    }
    rasterize(pb, shape, cam, scale, colour, false);
}

fn rasterize(
    pb: &mut PixelBuffer, shape: &MinibossShape, cam: Vec2, scale: Vec2, colour: Rgb, fill: bool,
) {
    let Some((lo, hi)) = shape.signature_bounds() else { return };
    let left = (((lo.x - cam.x) / scale.x).floor() as i32).max(0);
    let right = (((hi.x - cam.x) / scale.x).ceil() as i32).min(pb.w as i32 - 1);
    let top = (((lo.y - cam.y) / scale.y).floor() as i32).max(0);
    let bottom = (((hi.y - cam.y) / scale.y).ceil() as i32).min(pb.h as i32 - 1);
    let dx = Vec2::new(scale.x, 0.0);
    let dy = Vec2::new(0.0, scale.y);
    for y in top..=bottom {
        for x in left..=right {
            let p = cam + Vec2::new(x as f32 * scale.x, y as f32 * scale.y);
            if shape.contains(p, 0.0)
                && (fill || [p - dx, p + dx, p - dy, p + dy].iter().any(|&q| !shape.contains(q, 0.0)))
            {
                pb.plot(x, y, colour);
            }
        }
    }
}

pub fn milk_warning_colour(timer: f32) -> Rgb {
    if timer < TELL_LOCK { (242, 139, 81) } else { (255, 199, 103) }
}

pub fn draw_telegraph(
    pb: &mut PixelBuffer, mb: &Miniboss, footprint: &MinibossShape, cam: Vec2, scale: Vec2,
) {
    if matches!(footprint, MinibossShape::MilkSpray(_)) {
        if mb.state == MinibossState::Tell {
            draw_outline(pb, footprint, cam, scale, milk_warning_colour(mb.timer));
        }
        return;
    }
    if let MinibossShape::False { origin, bulbs, selected, .. } = *footprint {
        let screen = |p: Vec2| {
            let p = p - cam;
            ((p.x / scale.x).round() as i32, (p.y / scale.y).round() as i32)
        };
        let bulb_scale = scale.x.max(scale.y).max(0.01);
        let active = mb.state == MinibossState::Active;
        let (ox, oy) = screen(origin);
        for (i, bulb) in bulbs.into_iter().enumerate() {
            let (x, y) = screen(bulb);
            if selected.is_none_or(|index| index == i) {
                pb.line(ox, oy, x, y, (100, 151, 162));
            } else if active {
                let progress = (mb.timer / mb.active_duration()).clamp(0.0, 1.0);
                let pop = ((2.0 + (FALSE_POP_RADIUS - 2.0) * progress) / bulb_scale).round() as i32;
                // A hollow cool pulse, with neither a line nor a jaw fill.
                pb.ring(x, y, pop.max(1), (98, 178, 195));
                continue;
            }
            let colour = if selected.is_none_or(|index| index == i) {
                (199, 250, 251)
            } else {
                (77, 127, 143)
            };
            pb.filled_circle(x, y, (2.0 / bulb_scale).round().max(1.0) as i32, colour);
        }
        if active {
            rasterize(pb, footprint, cam, scale, (182, 74, 50), true);
            draw_outline(pb, footprint, cam, scale, (255, 236, 177));
        } else {
            draw_outline(pb, footprint, cam, scale, (242, 139, 81));
        }
        return;
    }
    draw_outline(pb, footprint, cam, scale, (242, 139, 81));
    if let Some(live) = mb.active_shape() {
        draw_outline(pb, &live, cam, scale, (255, 236, 177));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{cell_of, TileKind};
    use crate::game::ecs::constants::PLAYER_RADIUS as R;
    use crate::game::ecs::systems::miniboss::{apply_strike, multi_hit_bit, MinibossKind, Signature};
    use crate::game::level::{LightFixture, LightFixtureKind, Theme};
    use crate::game::weapon::WeaponKind;
    use crate::game::world::World;
    use rand::RngExt;

    const ORIGIN: Vec2 = Vec2 { x: 80.0, y: 80.0 };
    const AIM: Vec2 = Vec2 { x: 1.0, y: 0.0 };
    const TARGET: Vec2 = Vec2 { x: 112.0, y: 80.0 };
    const TIDE: SignatureShape = SignatureShape::Tide { len: 56.0, half: 4.0, gap: 12.0, travel: 48.0 };
    const RETURN: SignatureShape = SignatureShape::Return { len: 42.0, half: 3.0, sweep: 1.6 };
    const WAKE: SignatureShape = SignatureShape::Wake { delay: 0.25, window: 0.5, half: 4.0 };
    const DRIP: SignatureShape = SignatureShape::Drip { radius: 5.0 };
    const COMB: SignatureShape = SignatureShape::Comb { len: 64.0, half: 4.0, tooth_gap: 2.0, wide_gap: 12.0 };
    const DT: f32 = 1.0 / 60.0;

    fn point(x: f32, y: f32) -> Vec2 {
        ORIGIN + Vec2::new(x, y)
    }

    fn staged(shape: SignatureShape) -> Miniboss {
        let mut mb = Miniboss::new(MinibossKind::Corkscrew, 0);
        mb.origin = ORIGIN;
        mb.aim = AIM;
        mb.target = TARGET;
        mb.state = MinibossState::Tell;
        mb.timer = TELL_LOCK;
        let mut track = RatTrack::default();
        track.record(0.0, ORIGIN);
        mb.signature_track = SignatureTrack::start(shape, ORIGIN, &track);
        for frame in 1..=30 {
            track.record(DT, point(frame as f32, 0.0));
        }
        mb.signature_track.update(shape, TELL_LOCK, &track);
        mb
    }

    #[test]
    fn each_new_signature_has_a_safe_and_dangerous_spot() {
        let cases = [
            (SignatureShape::Annulus { inner: 10.0, outer: 20.0 }, TARGET, point(47.0, 0.0)),
            (TIDE, point(24.0, 11.0), point(24.0, -12.0)),
            (RETURN, point(4.0, 0.0), point(22.0, 0.0)),
            (SignatureShape::Overlap { radius: 16.0, sep: 14.0 }, TARGET, point(32.0, 18.0)),
            (COMB, point(32.0, 16.0), TARGET),
            (SignatureShape::Shear { len: 50.0, half: 4.0, channel: 24.0 }, point(-8.0, 0.0), point(24.0, 0.0)),
            (WAKE, point(18.0, 12.0), point(18.0, 0.0)),
            (DRIP, point(15.0, 12.0), point(15.0, 0.0)),
            (SignatureShape::Umbra { half_angle: 0.6, len: 46.0 }, point(-18.0, 0.0), point(24.0, 0.0)),
        ];
        for (shape, safe, danger) in cases {
            let mb = staged(shape);
            let footprint = resolve(shape, &mb, None);
            let progress = if shape == RETURN { 0.25 } else { 0.5 };
            let strike = resolve(shape, &mb, Some((progress, STAMP_STEP)));
            for geometry in [footprint, strike] {
                assert!(!geometry.contains(safe, R), "{shape:?}: {safe:?} should be safe");
                assert!(geometry.contains(danger, R), "{shape:?}: {danger:?} should be hit");
            }
        }
    }

    #[test]
    fn milk_spray_follows_the_locked_cone_and_stops_at_wall_faces() {
        let mut level = Level::empty(Theme::Kitchen);
        let mut mb = staged(SignatureShape::MilkSpray);
        mb.kind = MinibossKind::MilkCarton;
        let full = resolve(SignatureShape::MilkSpray, &mb, None);
        assert!(full.contains(point(51.0, 11.0), 0.0));
        assert!(full.contains(point(104.0, 0.0), 0.0));
        assert!(!full.contains(point(106.0, 0.0), 0.0));
        assert!(!full.contains(point(48.0, 20.0), R));
        assert!(!full.contains(point(4.0, 0.0), R));

        let (x, y) = cell_of(point(32.0, 0.0));
        level.tiles.set(x, y, TileKind::Wall(WallKind::Solid));
        mb.spray = Some(MilkSpray::capture(&level.tiles, ORIGIN, AIM));
        let clipped = resolve(SignatureShape::MilkSpray, &mb, None);
        assert!(clipped.contains(point(31.0, 3.0), 0.0));
        assert!(!clipped.contains(point(32.0, 3.0), 0.0));
        assert!(!clipped.contains(point(49.0, 0.0), R));
        assert!(clipped.contains(point(49.0, -10.0), 0.0));

        mb.state = MinibossState::Active;
        mb.timer = 0.02;
        assert!(!mb.active_shape().unwrap().contains(point(26.0, 0.0), 0.0));
        mb.timer = 0.06;
        assert!(mb.active_shape().unwrap().contains(point(26.0, 0.0), 0.0));
        mb.timer = mb.active_duration();
        assert_eq!(mb.active_shape(), Some(clipped));
    }

    #[test]
    fn milk_spray_hits_once_at_the_appointment_damage() {
        for (appointment, damage) in [18.0, 22.0, 26.0, 30.0].into_iter().enumerate() {
            let mut mb = Miniboss::new(MinibossKind::MilkCarton, appointment);
            mb.origin = ORIGIN;
            mb.aim = AIM;
            mb.timer = TELL_LOCK;
            mb.state = MinibossState::Tell;
            assert!(mb.active_shape().is_none());
            mb.state = MinibossState::Active;
            let mut rat = Player::new(point(48.0, 0.0), WeaponKind::Gnaw);
            let hp = rat.hp;
            let armor = rat.loadout.stats.armor;
            let front = mb.active_shape().unwrap();
            assert!(apply_strike(&mut mb, &mut rat, &front, None, 1.0));
            assert_eq!(hp - rat.hp, damage - armor);
            mb.timer += 0.1;
            rat.iframes = 0.0;
            let front = mb.active_shape().unwrap();
            assert!(!apply_strike(&mut mb, &mut rat, &front, None, 1.0));
        }
    }

    #[test]
    fn milk_splashes_arrive_with_the_jet_and_expire() {
        let mut level = Level::empty(Theme::Kitchen);
        let (x, y) = cell_of(point(32.0, 0.0));
        level.tiles.fill_cells(x, y - 2, 1, 5, TileKind::Wall(WallKind::Solid));
        let spray = MilkSpray::capture(&level.tiles, ORIGIN, AIM);
        let mut splashes = Vec::new();
        spray.deposit(0.0, 0.01, 0.01, &mut splashes);
        assert!(splashes.is_empty());
        spray.deposit(0.01, 0.30, 0.30, &mut splashes);
        assert!(!splashes.is_empty());
        assert!(splashes.iter().all(|s| s.pos.x < point(32.0, 0.0).x));
        spray.deposit(0.30, 0.50, 0.50, &mut splashes);
        let splash = splashes[0];
        assert!(splash.opacity(splash.born + 0.74) > splash.opacity(splash.born + 1.08));
        assert!(!splash.expired(splash.born + 1.24));
        assert!(splash.expired(splash.born + 1.26));
        assert_eq!(splash.opacity(splash.born + 1.26), 0.0);
    }

    #[test]
    fn tide_gap_drifts_until_lock_and_comb_changes_slots_at_lock() {
        let mut mb = staged(TIDE);
        mb.timer = 0.0;
        let MinibossShape::Tide { gap: start, .. } = resolve(TIDE, &mb, None) else { panic!() };
        mb.timer = DRIP_MIDDLE;
        let MinibossShape::Tide { gap: during, .. } = resolve(TIDE, &mb, None) else { panic!() };
        assert!(start.0 < during.0);
        assert!(!resolve(COMB, &mb, None).contains(TARGET, R));
        mb.timer = TELL_LOCK;
        let tide_lock = resolve(TIDE, &mb, None);
        let MinibossShape::Tide { gap, .. } = tide_lock else { panic!() };
        assert!(during.0 < gap.0);
        assert!(resolve(COMB, &mb, None).contains(TARGET, R));
        mb.timer = TELL_LOCK + 0.2;
        assert_eq!(resolve(TIDE, &mb, None), tide_lock);
        let MinibossShape::Tide { gap: live, .. } = resolve(TIDE, &mb, Some((0.9, 0.6))) else { panic!() };
        assert_eq!(live, gap);
    }

    #[test]
    fn drip_samples_at_start_middle_and_lock_and_strikes_in_order() {
        let mut mb = staged(DRIP);
        let lock = resolve(DRIP, &mb, None);
        let MinibossShape::Drip { centers, .. } = lock else { panic!() };
        for (actual, x) in centers.into_iter().zip([0.0, 15.0, 30.0]) {
            assert!(actual.dist(point(x, 0.0)) < 1e-4, "sample was {actual:?}");
        }
        mb.timer += 0.2;
        let mut later = RatTrack::default();
        later.record(0.0, point(50.0, 30.0));
        mb.signature_track.update(DRIP, mb.timer, &later);
        assert_eq!(resolve(DRIP, &mb, None), lock);
        let sig = Signature { shape: DRIP, active: 0.9 };
        let mut rat = Player::new(centers[0], WeaponKind::Gnaw);
        for (i, center) in centers.into_iter().enumerate() {
            let time = STAMP_STEP * i as f32;
            let disk = resolve(DRIP, &mb, Some((time / sig.active, time)));
            assert!(disk.contains(center, R));
            for (other, &pos) in centers.iter().enumerate() {
                if i != other {
                    assert!(!disk.contains(pos, R));
                }
            }
            rat.pos = center;
            let bit = multi_hit_bit(sig, time);
            assert!(apply_strike(&mut mb, &mut rat, &disk, bit, 1.0));
            assert!(!apply_strike(&mut mb, &mut rat, &disk, bit, 1.0));
        }
    }

    #[test]
    fn return_hits_once_on_each_pass_and_dash_preserves_the_second_hit() {
        let sig = Signature { shape: RETURN, active: 1.0 };
        let mut mb = staged(RETURN);
        let mut rat = Player::new(point(22.0, 0.0), WeaponKind::Gnaw);
        let outward = resolve(RETURN, &mb, Some((0.25, 0.25)));
        assert!(apply_strike(&mut mb, &mut rat, &outward, multi_hit_bit(sig, 0.25), 1.0));
        assert!(!apply_strike(&mut mb, &mut rat, &outward, multi_hit_bit(sig, 0.25), 1.0));
        let returning = resolve(RETURN, &mb, Some((0.75, 0.75)));
        let second = multi_hit_bit(sig, 0.75);
        assert_eq!(outward, returning);
        rat.dash_active = 0.1;
        assert!(!apply_strike(&mut mb, &mut rat, &returning, second, 1.0));
        rat.dash_active = 0.0;
        assert!(apply_strike(&mut mb, &mut rat, &returning, second, 1.0));
        assert!(!apply_strike(&mut mb, &mut rat, &returning, second, 1.0));
    }

    #[test]
    fn wake_waits_for_its_delay_and_hits_the_earlier_path_once() {
        let mut mb = staged(WAKE);
        let mut rat = Player::new(point(8.0, 12.0), WeaponKind::Gnaw);
        assert!(locked_tell_hit(WAKE, &mb).is_none());
        mb.timer = TELL_LOCK + 0.25;
        let live = locked_tell_hit(WAKE, &mb).expect("delay elapsed");
        assert!(!apply_strike(&mut mb, &mut rat, &live, None, 1.0));
        rat.pos = point(8.0, 0.0);
        assert!(apply_strike(&mut mb, &mut rat, &live, None, 1.0));
        rat.iframes = 0.0;
        assert!(!apply_strike(&mut mb, &mut rat, &live, None, 1.0));
    }

    #[test]
    fn umbra_uses_the_nearest_clear_live_sconce_or_keeps_the_rat_aim() {
        let mut level = Level::empty(Theme::Sewer);
        let lamp = |pos, seed| LightFixture { pos, kind: LightFixtureKind::Sconce, radius: 80.0, seed };
        assert_eq!(umbra_aim(&level, ORIGIN, AIM), AIM);
        level.objects.lights.extend([lamp(point(0.0, 16.0), 8), lamp(point(33.0, 0.0), 1)]);
        assert_eq!(umbra_aim(&level, ORIGIN, AIM), -AIM);
        let (x, y) = cell_of(point(16.0, 0.0));
        level.tiles.set(x, y, TileKind::Wall(WallKind::Solid));
        assert_eq!(umbra_aim(&level, ORIGIN, AIM), AIM);
    }

    #[test]
    fn outlines_have_pixels_and_fit_the_library_bounds() {
        let shapes = [
            SignatureShape::Annulus { inner: 10.0, outer: 20.0 }, TIDE, RETURN,
            SignatureShape::Overlap { radius: 16.0, sep: 14.0 }, COMB,
            SignatureShape::Shear { len: 50.0, half: 4.0, channel: 24.0 }, WAKE, DRIP,
            SignatureShape::Umbra { half_angle: 0.6, len: 46.0 },
        ];
        for shape in shapes {
            let mb = staged(shape);
            let mut pb = PixelBuffer::new(180, 160);
            let footprint = resolve(shape, &mb, None);
            let (lo, hi) = footprint.signature_bounds().expect("Library bounds");
            draw_outline(&mut pb, &footprint, Vec2::ZERO, Vec2::splat(1.0), (255, 160, 96));
            let mut marked = 0;
            for (i, &color) in pb.pixels().iter().enumerate() {
                if color == (0, 0, 0) {
                    continue;
                }
                marked += 1;
                let (x, y) = ((i % pb.w) as f32, (i / pb.w) as f32);
                assert!(x >= lo.x && x <= hi.x && y >= lo.y && y <= hi.y, "{shape:?}: ({x}, {y})");
            }
            assert!(marked > 0, "{shape:?} did not draw");
        }
    }

    #[test]
    fn outfall_warnings_draw_without_lighting_on_a_narrow_frame() {
        for zoom in [1.0, 2.0] {
            let scale = Vec2::splat(zoom);
            let cam = ORIGIN - Vec2::new(44.0, 19.0) * zoom;
            for (kind, safe) in [
                (MinibossKind::Gasket, point(16.0, 0.0)),
                (MinibossKind::Swell, point(24.0, 12.0)),
                (MinibossKind::Shade, point(-16.0, 0.0)),
                (MinibossKind::FalseBulb, point(31.0, 14.0)),
            ] {
                let mut mb = Miniboss::new(kind, 0);
                mb.origin = ORIGIN;
                mb.aim = AIM;
                mb.target = TARGET;
                mb.state = MinibossState::Tell;
                mb.timer = TELL_LOCK;
                let mut pb = PixelBuffer::new(114, 40);
                let warned = mb.telegraph().expect("fixed warning");
                draw_telegraph(&mut pb, &mb, &warned, cam, scale);
                assert!(pb.pixels().contains(&(242, 139, 81)), "{kind:?} at {zoom}x");
                let x = ((safe.x - cam.x) / zoom).round() as usize;
                let y = ((safe.y - cam.y) / zoom).round() as usize;
                assert_eq!(pb.pixel_at(x, y), (0, 0, 0), "{kind:?} covers its safe space");

                if kind == MinibossKind::FalseBulb {
                    mb.state = MinibossState::Active;
                    mb.timer = 0.1;
                    pb.clear((0, 0, 0));
                    let warned = mb.telegraph().expect("live jaw and decoy");
                    draw_telegraph(&mut pb, &mb, &warned, cam, scale);
                    assert!(pb.pixels().contains(&(182, 74, 50)), "selected jaw fills");
                    assert!(pb.pixels().contains(&(98, 178, 195)), "decoy has a hollow pop");
                    assert_eq!(pb.pixel_at(x, y), (0, 0, 0), "the decoy has no jaw fill");
                }
            }
        }
    }

    #[test]
    fn recorder_keeps_the_latest_two_seconds_without_drawing_rng() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut expected = World::new(Theme::Sewer, 7).rng;
        for i in 0..=180 {
            world.player.pos = point(i as f32, 0.0);
            world.begin_tick(DT, Vec2::ZERO, false);
            world.ecs.run_system_cached(record_rat_track).expect("record the post-move position");
            world.end_tick();
        }
        let track = world.ecs.resource::<RatTrack>();
        assert_eq!(track.samples().count(), RAT_TRACK_CAPACITY);
        assert!(track.at_age(0.5).dist(point(150.0, 0.0)) < 1e-3);
        assert!(track.at_age(2.0).dist(point(60.0, 0.0)) < 1e-3);
        assert_eq!(world.rng.random::<u64>(), expected.random::<u64>());
    }
}
