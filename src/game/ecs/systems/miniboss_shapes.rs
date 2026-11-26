//! Geometry for the nine signature families added alongside per-kind movement.
//! Telegraphs, damage, bot steering and Library fitting use the same geometry.

use bevy_ecs::prelude::*;

use super::miniboss::{Miniboss, MinibossShape, MinibossState, SignatureShape, STAMP_STEP, TELL_LOCK};
use super::projectiles::los_clear;
use crate::game::ecs::resources::{RatTrack, TickInput, RAT_TRACK_CAPACITY};
use crate::game::level::Level;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

const RETURN_HUB: f32 = 8.0;
const SCONCE_REACH: f32 = 140.0;
const DRIP_MIDDLE: f32 = 0.25;

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
    let Some((lo, hi)) = shape.signature_bounds() else { return };
    if let MinibossShape::Drip { centers, radius, mask: 0b111 } = *shape {
        for (i, color) in [colour, (254, 184, 119), (225, 135, 98)].into_iter().enumerate() {
            let circle = MinibossShape::Drip { centers, radius, mask: 1 << i };
            draw_outline(pb, &circle, cam, scale, color);
        }
        return;
    }
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
                && [p - dx, p + dx, p - dy, p + dy].iter().any(|&q| !shape.contains(q, 0.0))
            {
                pb.plot(x, y, colour);
            }
        }
    }
}

pub fn draw_telegraph(
    pb: &mut PixelBuffer, mb: &Miniboss, footprint: &MinibossShape, cam: Vec2, scale: Vec2,
) {
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
            assert!(apply_strike(&mut mb, &mut rat, &disk, bit));
            assert!(!apply_strike(&mut mb, &mut rat, &disk, bit));
        }
    }

    #[test]
    fn return_hits_once_on_each_pass_and_dash_preserves_the_second_hit() {
        let sig = Signature { shape: RETURN, active: 1.0 };
        let mut mb = staged(RETURN);
        let mut rat = Player::new(point(22.0, 0.0), WeaponKind::Gnaw);
        let outward = resolve(RETURN, &mb, Some((0.25, 0.25)));
        assert!(apply_strike(&mut mb, &mut rat, &outward, multi_hit_bit(sig, 0.25)));
        assert!(!apply_strike(&mut mb, &mut rat, &outward, multi_hit_bit(sig, 0.25)));
        let returning = resolve(RETURN, &mb, Some((0.75, 0.75)));
        let second = multi_hit_bit(sig, 0.75);
        assert_eq!(outward, returning);
        rat.dash_active = 0.1;
        assert!(!apply_strike(&mut mb, &mut rat, &returning, second));
        rat.dash_active = 0.0;
        assert!(apply_strike(&mut mb, &mut rat, &returning, second));
        assert!(!apply_strike(&mut mb, &mut rat, &returning, second));
    }

    #[test]
    fn wake_waits_for_its_delay_and_hits_the_earlier_path_once() {
        let mut mb = staged(WAKE);
        let mut rat = Player::new(point(8.0, 12.0), WeaponKind::Gnaw);
        assert!(locked_tell_hit(WAKE, &mb).is_none());
        mb.timer = TELL_LOCK + 0.25;
        let live = locked_tell_hit(WAKE, &mb).expect("delay elapsed");
        assert!(!apply_strike(&mut mb, &mut rat, &live, None));
        rat.pos = point(8.0, 0.0);
        assert!(apply_strike(&mut mb, &mut rat, &live, None));
        rat.iframes = 0.0;
        assert!(!apply_strike(&mut mb, &mut rat, &live, None));
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
    fn recorder_keeps_the_latest_two_seconds_without_drawing_rng() {
        let mut world = World::new(Theme::Sewer, 7);
        let mut expected = world.rng.clone();
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
