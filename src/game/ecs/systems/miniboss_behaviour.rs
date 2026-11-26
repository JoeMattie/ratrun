//! Named miniboss movement and their two attacks, in appointment order.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;

use super::hazards::InkSlow;
use super::miniboss::{
    Miniboss, MinibossKind, MinibossShape, Signature, SignatureShape, StampLayout, CONE_HALF_ANGLE,
    CONE_LEN, CORD_HALF, CORD_LEN, LATE_PLATE_CLOSED, RING_GAP_HALF, RING_HALF, RING_RADIUS,
    SPILL_HALF, SPILL_LEN, STAMP_HALF, STAMP_SPACING, TRENCH_HALF, TRENCH_LEN,
};
use super::projectiles::los_clear;
use super::traps::TrapSlow;
use crate::game::collision::{cell_of, SizeClass};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{DebugFlags, NextEnemyId, NextSpawnId};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::hazards::{clamp_outfall_displacement, outfall_sand_clear};
use crate::game::level::{Level, Theme};
use crate::game::levelgen::validate::{is_reached, reachable};
use crate::game::nav::FlowField;
use crate::game::particle::{Particle, Particles, Ramp, Shape, Stain, StainKind};
use crate::math::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStyle {
    Stalk,
    Charge,
    Orbit,
    Anchor,
    Burrow,
    Summon,
}

#[derive(Clone, Copy, Debug)]
pub struct Temperament {
    pub style: MoveStyle,
    pub speed: f32,
    pub gap: f32,
    pub second_from_hp: f32,
}

fn attack(active: f32, shape: SignatureShape) -> Signature {
    Signature { active, shape }
}

fn ring(to: f32, half: f32, gap_degrees: f32) -> SignatureShape {
    SignatureShape::Ring {
        from: 0.0, to, half, gap_half: gap_degrees.to_radians() * 0.5, on_rat: false,
    }
}

fn stamps(half: f32, layout: StampLayout) -> SignatureShape {
    SignatureShape::Stamps { half, layout }
}

fn cord(len: f32, half: f32, degrees: f32) -> SignatureShape {
    SignatureShape::Cord { len, half, sweep: degrees.to_radians() }
}

fn backswing(len: f32, half: f32, degrees: f32) -> SignatureShape {
    SignatureShape::Return { len, half, sweep: degrees.to_radians() }
}

fn wake(half: f32) -> SignatureShape {
    SignatureShape::Wake { delay: 0.18, window: 1.2, half }
}

fn comb(len: f32, half: f32, wide_gap: f32) -> SignatureShape {
    SignatureShape::Comb { len, half, tooth_gap: 3.0, wide_gap }
}

fn umbra(len: f32, degrees: f32) -> SignatureShape {
    SignatureShape::Umbra { len, half_angle: degrees.to_radians() * 0.5 }
}

impl MinibossKind {
    pub fn temperament(self) -> Temperament {
        self.behaviour().0
    }

    pub fn signatures(self) -> [Signature; 2] {
        self.behaviour().1
    }

    fn behaviour(self) -> (Temperament, [Signature; 2]) {
        use MinibossKind::*;
        use MoveStyle::*;
        use SignatureShape as S;
        let (style, speed, gap, second_from_hp, primary, secondary) = match self {
            // Sewers
            Corkscrew => (Charge, 68.0, 2.1, 1.0,
                attack(0.5, S::Trench { len: TRENCH_LEN, half: TRENCH_HALF }),
                attack(0.8, wake(4.0))),
            BellDiver => (Burrow, 43.0, 3.2, 0.5,
                attack(1.2, S::Ring {
                    from: 0.0, to: RING_RADIUS, half: RING_HALF, gap_half: RING_GAP_HALF, on_rat: false,
                }),
                attack(0.95, S::Tide { len: 68.0, half: 4.0, gap: 14.0, travel: 58.0 })),
            ValveWidow => (Anchor, 35.0, 2.8, 0.5,
                attack(0.85, backswing(78.0, 2.5, 100.0)),
                attack(1.0, umbra(84.0, 60.0))),
            Mudlark => (Stalk, 52.0, 1.8, 1.0,
                attack(0.65, S::Pincer { len: 30.0, half: 2.5, open: 28.0, closed: 4.0 }),
                attack(0.7, S::Overlap { radius: 19.0, sep: 22.0 })),
            // Kitchen
            MilkCarton => (Stalk, 38.0, 2.9, 0.5,
                attack(1.2, S::Spill { len: SPILL_LEN, half_w: SPILL_HALF }),
                attack(0.9, S::Drip { radius: 8.0 })),
            PastryWheel => (Charge, 66.0, 2.2, 1.0,
                attack(0.6, S::Trench { len: 46.0, half: 4.0 }),
                attack(0.72, comb(62.0, 4.0, 15.0))),
            SaltHen => (Summon, 44.0, 3.3, 0.6,
                attack(0.9, stamps(6.0, StampLayout::Along { spacing: 16.0 })),
                attack(0.8, S::Annulus { inner: 9.0, outer: 27.0 })),
            PotBrush => (Orbit, 50.0, 1.9, 1.0,
                attack(0.85, cord(42.0, 3.0, 110.0)),
                attack(0.85, S::Shear { len: 42.0, half: 3.0, channel: 18.0 })),
            // Lab
            DosePump => (Charge, 64.0, 2.05, 0.5,
                attack(0.65, S::Trench { len: 40.0, half: 4.0 }),
                attack(0.9, S::Drip { radius: 7.0 })),
            PetriMother => (Burrow, 40.0, 3.4, 1.0,
                attack(1.2, ring(36.0, 3.0, 75.0)),
                attack(0.9, S::Overlap { radius: 23.0, sep: 25.0 })),
            CageCounter => (Anchor, 30.0, 2.7, 0.5,
                attack(0.9, stamps(6.0, StampLayout::AroundRat { across: 15.0 })),
                attack(0.8, comb(96.0, 5.0, 16.0))),
            RotorTechnician => (Orbit, 56.0, 1.7, 1.0,
                attack(1.1, backswing(43.0, 2.5, 150.0)),
                attack(0.95, S::Shear { len: 50.0, half: 3.0, channel: 22.0 })),
            // Laundry
            SisterSnip => (Stalk, 54.0, 1.6, 1.0,
                attack(0.8, S::Shear { len: 38.0, half: 3.0, channel: 17.0 }),
                attack(0.8, S::Cone { half_angle: CONE_HALF_ANGLE, len: CONE_LEN })),
            Bobbin => (Orbit, 48.0, 2.35, 1.0,
                attack(0.7, cord(CORD_LEN, CORD_HALF, 90.0)),
                attack(0.85, comb(62.0, 3.0, 15.0))),
            SudsBailiff => (Burrow, 39.0, 3.3, 0.5,
                attack(1.2, S::Tide { len: 74.0, half: 5.0, gap: 15.0, travel: 58.0 }),
                attack(0.9, S::Drip { radius: 10.0 })),
            WringerBoy => (Anchor, 31.0, 2.9, 0.5,
                attack(1.0, S::Lane { len: 82.0, seam_half: 6.0, half: 3.0, bar_len: 32.0 }),
                attack(0.9, backswing(86.0, 3.0, 95.0))),
            // Printworks
            Foreman => (Summon, 41.0, 3.2, 1.0,
                attack(0.9, stamps(STAMP_HALF, StampLayout::Along { spacing: STAMP_SPACING })),
                attack(0.9, umbra(58.0, 55.0))),
            QuoinKey => (Charge, 63.0, 2.0, 0.5,
                attack(0.75, S::Cone { half_angle: 27.5_f32.to_radians(), len: 38.0 }),
                attack(0.78, S::Annulus { inner: 9.0, outer: 29.0 })),
            CarbonTwin => (Orbit, 46.0, 2.45, 1.0,
                attack(1.4, S::Spill { len: 38.0, half_w: 10.0 }),
                attack(0.85, wake(5.0))),
            LatePlate => (Anchor, 33.0, 2.85, 0.5,
                attack(1.2, S::Ring {
                    from: 32.0, to: LATE_PLATE_CLOSED, half: 2.5,
                    gap_half: 32.5_f32.to_radians(), on_rat: true,
                }),
                attack(0.95, S::Overlap { radius: 23.0, sep: 28.0 })),
            // Flatbed
            TarpaulinPup => (Burrow, 45.0, 2.6, 1.0,
                attack(1.1, S::Spill { len: 48.0, half_w: 9.0 }),
                attack(0.9, S::Tide { len: 66.0, half: 4.0, gap: 14.0, travel: 56.0 })),
            AxleNut => (Charge, 70.0, 1.85, 1.0,
                attack(0.65, S::Trench { len: 50.0, half: 5.0 }),
                attack(0.75, S::Annulus { inner: 8.0, outer: 28.0 })),
            ChainDog => (Stalk, 58.0, 2.3, 0.5,
                attack(0.9, cord(36.0, 3.0, 115.0)),
                attack(0.8, wake(4.5))),
            RoadAtlas => (Anchor, 32.0, 3.15, 0.5,
                attack(0.9, stamps(7.0, StampLayout::V { forward: 16.0, across: 14.0 })),
                attack(1.05, umbra(92.0, 64.0))),
            // Outfall uses the same walk speed and strike-end gap for each
            // appointment. Distinct movement styles set up the four warnings.
            Gasket => (Stalk, 48.0, 2.6, 0.5,
                attack(0.7, S::Annulus { inner: 8.0, outer: 18.0 }),
                attack(0.8, S::Spill { len: 36.0, half_w: 8.0 })),
            Swell => (Orbit, 48.0, 2.6, 0.5,
                attack(1.0, S::Tide { len: 64.0, half: 3.0, gap: 16.0, travel: 48.0 }),
                attack(0.85, S::Overlap { radius: 20.0, sep: 20.0 })),
            Shade => (Burrow, 48.0, 2.6, 0.65,
                attack(0.55, umbra(46.0, 50.0)),
                attack(0.9, stamps(6.0, StampLayout::AroundRat { across: 14.0 }))),
            FalseBulb => (Charge, 48.0, 2.6, 0.5,
                attack(0.20, S::False { forward: 22.0, across: 14.0, len: 18.0, half: 3.0 }),
                attack(0.85, S::Shear { len: 40.0, half: 2.0, channel: 22.0 })),
        };
        (Temperament { style, speed, gap, second_from_hp }, [primary, secondary])
    }

    pub(crate) fn speed_scale(self) -> f32 {
        self.temperament().speed / EnemyKind::Brute.speed()
    }
}

impl SignatureShape {
    pub fn name(self) -> &'static str {
        use SignatureShape::*;
        match self {
            Trench { .. } => "Trench",
            Cone { .. } => "Cone",
            Cord { .. } => "Sweeping cord",
            Ring { .. } => "Broken ring",
            Stamps { .. } => "Stamps",
            Spill { .. } => "Spill",
            Pincer { .. } => "Closing claws",
            Lane { .. } => "Rollers",
            Annulus { .. } => "Hollow ring",
            Tide { .. } => "Sweeping bar",
            Return { .. } => "Returning cord",
            Overlap { .. } => "Overlapping disks",
            Comb { .. } => "Slotted bar",
            Shear { .. } => "Crossing blades",
            Wake { .. } => "Live trail",
            Drip { .. } => "Falling drops",
            Umbra { .. } => "Shadow cone",
            False { .. } => "False jaws",
        }
    }
}

const STANDOFF: f32 = 70.0;
const CHARGE_TELL: f32 = 0.24;
const LUNGE_TIME: f32 = 0.35;
const SINK_TIME: f32 = 0.6;
const CRACK_TIME: f32 = 0.5;
const SURFACE_RANGE: f32 = 40.0;
const SUMMON_RANGE: f32 = 50.0;
const SUMMON_CAP: usize = 9;
const LARGE: SizeClass = SizeClass::Large;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Motion {
    #[default]
    Roam,
    ChargeTell { from: Vec2, end: Vec2, left: f32 },
    Lunge { from: Vec2, end: Vec2, left: f32 },
    Charged,
    Sunk { left: f32 },
    Crack { at: Vec2, left: f32 },
    Surfaced,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Manoeuvre {
    pub motion: Motion,
    pub anchored: bool,
    orbit_step: u32,
    orbit_flip: f32,
}

#[derive(Component)]
pub(crate) struct MinibossSummon(pub u64);

#[derive(SystemParam)]
pub(crate) struct MovementParams<'w, 's> {
    pub next_id: ResMut<'w, NextSpawnId>,
    pub next_enemy_id: ResMut<'w, NextEnemyId>,
    flow: Res<'w, FlowField>,
    debug: Res<'w, DebugFlags>,
    glue: Query<'w, 's, &'static TrapSlow>,
    ink: Query<'w, 's, &'static InkSlow>,
    summons: Query<'w, 's, (Entity, &'static SpawnId, &'static Enemy, &'static MinibossSummon), Without<Miniboss>>,
}

// A fixed integer mix keyed by spawn identity and a local step. No run RNG
// is used for steering, placement, summons or movement effects.
fn phase(id: u64, step: u32) -> u32 {
    let mut n = id.wrapping_add(u64::from(step).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (n ^ (n >> 31)) as u32
}

fn unit(n: u32) -> f32 {
    (n >> 8) as f32 / (1 << 24) as f32
}

fn orbit_direction(body: Vec2, rat: Vec2, range: f32, side: f32) -> Vec2 {
    let delta = rat - body;
    let inward = delta.normalized();
    let correction = ((delta.len() - range) / 10.0).clamp(-1.0, 1.0);
    (inward.perp() * side + inward * correction).normalized()
}

pub(super) fn clear(level: &Level, at: Vec2, radius: f32, class: SizeClass) -> bool {
    if level.theme == Theme::Outfall {
        outfall_sand_clear(level, at, radius, class)
    } else {
        level.tiles.circle_clear(at, radius, class) && level.trailer.pit_avoid(at, radius).is_none()
    }
}

fn open_step(level: &Level, from: Vec2, to: Vec2, radius: f32) -> bool {
    clear(level, to, radius, LARGE) && if level.theme == Theme::Outfall {
        outfall_step(level, from, to, radius).dist_sq(to) < 1e-4
    } else {
        los_clear(level, from, to, radius)
    }
}

fn outfall_step(level: &Level, from: Vec2, to: Vec2, radius: f32) -> Vec2 {
    clamp_outfall_displacement(level, from, to - from, radius, LARGE)
}

fn landing(
    level: &Level, seen: &[bool], rat: Vec2, radius: f32, class: SizeClass,
    turn: f32, range: f32,
) -> Option<Vec2> {
    (0..16).find_map(|i| {
        let at = rat + Vec2::from_angle(turn + i as f32 * std::f32::consts::TAU / 16.0) * range;
        (is_reached(seen, &level.tiles, cell_of(at)) && clear(level, at, radius, class))
            .then_some(at)
    })
}

fn dust_mound(particles: &mut Particles, at: Vec2) {
    for (x, y) in [(-4.0, 0.0), (4.0, 0.0), (-2.0, -2.0), (2.0, -2.0), (0.0, -3.0)] {
        particles.push(Particle {
            pos: at + Vec2::new(x, y),
            vel: Vec2::ZERO,
            life: SINK_TIME + CRACK_TIME,
            max_life: SINK_TIME + CRACK_TIME,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Blood((138, 101, 68)),
            additive: false,
            shape: Shape::Puff,
        });
    }
}

fn crack(particles: &mut Particles, at: Vec2, id: u64) {
    particles.push_stain(Stain {
        pos: at,
        radius: 9.0,
        color: (188, 134, 83),
        life: CRACK_TIME + 0.3,
        max_life: CRACK_TIME + 0.3,
        seed: phase(id, 0) as u16,
        kind: StainKind::Crack,
    });
}

impl Manoeuvre {
    pub fn charge_lane(self, pos: Vec2, radius: f32) -> Option<(MinibossShape, f32, bool)> {
        let (a, b, left, live) = match self.motion {
            Motion::ChargeTell { from, end, left } => (from, end, left, false),
            Motion::Lunge { end, left, .. } => (pos, end, left, true),
            _ => return None,
        };
        Some((MinibossShape::Trench { a, b, half: radius }, left, live))
    }

    pub fn locks_position(self) -> bool {
        self.anchored || matches!(self.motion,
            Motion::ChargeTell { .. } | Motion::Lunge { .. } | Motion::Charged
                | Motion::Sunk { .. } | Motion::Crack { .. } | Motion::Surfaced)
    }
}

impl MovementParams<'_, '_> {
    /// Move between signatures. The return value permits a tell this tick.
    pub(super) fn advance(
        &self, entity: Entity, id: u64, mb: &mut Miniboss, body: &mut Enemy,
        level: &Level, rat: Vec2, range: f32, until_ready: f32, dt: f32,
        particles: &mut Particles,
    ) -> bool {
        let ready = until_ready <= 0.0;
        if mb.preview {
            return ready;
        }
        if self.debug.freeze {
            return ready && !body.buried && body.pos.dist(rat) <= range;
        }
        let t = mb.kind.temperament();
        let distance = body.pos.dist(rat);
        let chase = self.flow.dir_at(LARGE, body.pos);
        let direct = (rat - body.pos).normalized();
        let stand = |body: &mut Enemy| body.vel = Vec2::ZERO;
        let steer = |body: &mut Enemy, dir: Vec2| {
            let glue = self.glue.get(entity).map_or(1.0, |s| s.0);
            let ink = self.ink.get(entity).map_or(1.0, |s| s.0);
            let dir = if let Some(away) = level.trailer.pit_avoid(body.pos, body.radius) {
                (dir + away * 1.6).normalized()
            } else {
                dir
            };
            let before = body.pos;
            body.steer(before + dir * 32.0, dir, dt, glue * ink);
            body.pos = level.tiles.resolve_circle(body.pos, body.radius, LARGE, None);
            if level.theme == Theme::Outfall && clear(level, before, body.radius, LARGE) {
                body.pos = outfall_step(level, before, body.pos, body.radius);
            }
        };
        let toward = if open_step(level, body.pos, rat, body.radius) || chase == Vec2::ZERO {
            direct
        } else {
            chase
        };
        match t.style {
            MoveStyle::Stalk | MoveStyle::Summon => {
                if ready && distance <= range {
                    return true;
                }
                steer(body, toward);
            }
            MoveStyle::Anchor => {
                let anchor_range = mb.kind.signatures().into_iter()
                    .map(|sig| mb.kind.engage_range_for(sig))
                    .fold(range, f32::min);
                let in_sight = los_clear(level, body.pos, rat, 2.0);
                if distance <= anchor_range && in_sight {
                    mb.manoeuvre.anchored = true;
                }
                if mb.manoeuvre.anchored {
                    stand(body);
                    return ready && distance <= range && in_sight;
                }
                steer(body, toward);
            }
            MoveStyle::Orbit => {
                let m = &mut mb.manoeuvre;
                m.orbit_flip -= dt;
                if m.orbit_flip <= 0.0 {
                    m.orbit_step = m.orbit_step.wrapping_add(1);
                    m.orbit_flip += 2.0 + 2.0 * unit(phase(id, m.orbit_step));
                }
                if ready && distance <= range + 2.0 && los_clear(level, body.pos, rat, 2.0) {
                    return true;
                }
                let side = if (m.orbit_step ^ phase(id, 0)) & 1 == 0 { 1.0 } else { -1.0 };
                let orbit = orbit_direction(body.pos, rat, range, side);
                let dir = if distance > range + 16.0
                    || !open_step(level, body.pos, body.pos + orbit * 12.0, body.radius)
                {
                    toward
                } else {
                    orbit
                };
                steer(body, dir);
            }
            MoveStyle::Charge => {
                let m = &mut mb.manoeuvre.motion;
                match *m {
                    Motion::ChargeTell { from, end, left } => {
                        stand(body);
                        *m = if left <= dt {
                            Motion::Lunge { from, end, left: LUNGE_TIME }
                        } else {
                            Motion::ChargeTell { from, end, left: left - dt }
                        };
                    }
                    Motion::Lunge { from, end, left } => {
                        let dir = (end - from).normalized();
                        let step = t.speed * 3.0 * dt.min(left);
                        let remaining = end.dist(body.pos);
                        let left = if step >= remaining { 0.0 } else { (left - dt).max(0.0) };
                        let next = if step >= remaining { end } else { body.pos + dir * step };
                        if !open_step(level, body.pos, next, body.radius) {
                            stand(body);
                            *m = Motion::Charged;
                        } else {
                            let delta = next - body.pos;
                            body.pos = next;
                            body.vel = if left > 0.0 && dt > 0.0 {
                                delta * dt.recip()
                            } else {
                                Vec2::ZERO
                            };
                            body.facing = dir;
                            body.anim += dt * (4.0 + t.speed * 1.5);
                            *m = if left > 0.0 {
                                Motion::Lunge { from, end, left }
                            } else {
                                Motion::Charged
                            };
                        }
                    }
                    Motion::Charged if distance <= range => {
                        stand(body);
                        return ready;
                    }
                    _ => {
                        *m = Motion::Roam;
                        let back = open_step(level, body.pos, body.pos - direct * 12.0, body.radius);
                        if distance > STANDOFF + 2.0 {
                            steer(body, toward);
                        } else if distance < STANDOFF - 4.0 && back {
                            steer(body, -direct);
                        } else if until_ready <= CHARGE_TELL + LUNGE_TIME && direct != Vec2::ZERO {
                            stand(body);
                            let from = body.pos;
                            let travel = t.speed * 3.0 * LUNGE_TIME;
                            // Tell only as far as the lunge can actually go.
                            let mut length = travel;
                            while length > 8.0
                                && !open_step(level, from, from + direct * length, body.radius)
                            {
                                length *= 0.75;
                            }
                            if length > 8.0 {
                                let end = from + direct * length;
                                *m = Motion::ChargeTell { from, end, left: CHARGE_TELL };
                            } else {
                                steer(body, toward);
                            }
                        } else {
                            stand(body);
                        }
                    }
                }
            }
            MoveStyle::Burrow => {
                let m = &mut mb.manoeuvre.motion;
                match *m {
                    Motion::Sunk { left } => {
                        if left > dt {
                            *m = Motion::Sunk { left: left - dt };
                        } else {
                            let seen = reachable(&level.tiles, cell_of(rat), SizeClass::Small);
                            let turn = unit(phase(id, mb.strikes)) * std::f32::consts::TAU;
                            if let Some(at) = [SURFACE_RANGE, 32.0, 48.0].into_iter()
                                .find_map(|r| landing(level, &seen, rat, body.radius, LARGE, turn, r))
                            {
                                crack(particles, at, id);
                                *m = Motion::Crack { at, left: CRACK_TIME };
                            } else {
                                body.buried = false;
                                *m = Motion::Surfaced;
                            }
                        }
                    }
                    Motion::Crack { at, left } => {
                        if left > dt {
                            *m = Motion::Crack { at, left: left - dt };
                        } else {
                            if clear(level, at, body.radius, LARGE) {
                                body.pos = at;
                            }
                            body.buried = false;
                            *m = Motion::Surfaced;
                        }
                    }
                    Motion::Surfaced => {
                        if distance <= range.max(SURFACE_RANGE + 2.0) {
                            return ready;
                        }
                        steer(body, toward);
                    }
                    _ => {
                        if distance <= STANDOFF && until_ready <= SINK_TIME + CRACK_TIME
                            && body.slow_t <= 0.0 && self.glue.get(entity).is_err()
                        {
                            stand(body);
                            body.buried = true;
                            dust_mound(particles, body.pos);
                            *m = Motion::Sunk { left: SINK_TIME };
                        } else {
                            steer(body, toward);
                        }
                    }
                }
            }
        }
        false
    }

    pub(super) fn summon(
        &mut self, commands: &mut Commands, owner: u64, mb: &Miniboss, pos: Vec2,
        rat: Vec2, level: &Level, difficulty: usize, elapsed: f32,
    ) {
        let live = self.summons.iter().filter(|(_, _, e, add)| add.0 == owner && e.alive()).count();
        let count = SUMMON_CAP.saturating_sub(live).min(3);
        if count == 0 {
            return;
        }
        let kind = crate::game::roster::kind(level.theme, crate::game::roster::Slot::A);
        let class = SizeClass::of_enemy(kind);
        let seen = reachable(&level.tiles, cell_of(pos), class);
        let turn = unit(phase(owner, mb.strikes)) * std::f32::consts::TAU;
        let mut placed = [pos; 3];
        let mut spawned = 0;
        for i in 0..3 {
            let angle = turn + i as f32 * std::f32::consts::TAU / 3.0;
            let mut at = None;
            for offset in [0.0, 0.24, -0.24, 0.48, -0.48, 0.72, -0.72, 0.96, -0.96] {
                let p = pos + Vec2::from_angle(angle + offset) * SUMMON_RANGE;
                if is_reached(&seen, &level.tiles, cell_of(p))
                    && clear(level, p, kind.radius(), class)
                    && p.dist(rat) > kind.radius() + crate::game::ecs::constants::PLAYER_RADIUS + 6.0
                    && placed[..spawned].iter().all(|q| q.dist(p) > kind.radius() * 2.0)
                {
                    at = Some(p);
                    break;
                }
            }
            let Some(at) = at else { continue };
            let mut enemy = Enemy::spawn_with_phases(
                kind, at, crate::game::director::horde_hp_mult_for(difficulty, elapsed),
                1.0 + 1.5 * unit(phase(owner, mb.strikes.wrapping_mul(9).wrapping_add(i * 3))),
                angle, angle,
            );
            let mods = crate::game::director::enemy_mods_for(difficulty, elapsed);
            enemy.damage *= mods.damage;
            enemy.speed_scale = mods.speed;
            enemy.id = self.next_enemy_id.take();
            enemy.summoned = true;
            commands.spawn((enemy, MinibossSummon(owner), self.next_id.take()));
            placed[spawned] = at;
            spawned += 1;
            if spawned == count {
                break;
            }
        }
    }

    pub(super) fn clean_summons(
        &self, commands: &mut Commands, live_owners: &[u64],
        order: &mut Vec<(u64, Entity)>,
    ) {
        order.clear();
        order.extend(self.summons.iter()
            .filter(|(_, _, _, add)| live_owners.binary_search(&add.0).is_err())
            .map(|(entity, id, _, _)| (id.0, entity)));
        order.sort_unstable_by_key(|&(id, _)| id);
        for &(_, entity) in order.iter() {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rand::RngExt;

    use super::*;
    use crate::game::campaign::CampaignTotals;
    use crate::game::director::MINIBOSS_TIMES;
    use crate::game::ecs::systems::miniboss::{
        miniboss_system, MinibossSchedule, MinibossState, RECOVER_TIME, TELL_TIME,
    };
    use crate::game::level::Theme;
    use crate::game::loadout::Loadout;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    const X: Vec2 = Vec2 { x: 1.0, y: 0.0 };

    #[test]
    fn personalities_and_attacks_are_distinct_on_every_floor() {
        let mut totals = BTreeMap::new();
        for theme in Theme::playable() {
            let mut families = Vec::new();
            let mut appearances: Vec<Temperament> = Vec::new();
            for kind in crate::game::roster::appointments(theme) {
                let t = kind.temperament();
                assert!((30.0..=70.0).contains(&t.speed), "{kind:?}");
                assert!((1.6..=3.4).contains(&t.gap), "{kind:?}");
                for other in &appearances {
                    assert_ne!(other.style, t.style, "{theme:?}");
                    if theme != Theme::Outfall {
                        assert_ne!(other.speed, t.speed, "{theme:?}");
                        assert_ne!(other.gap, t.gap, "{theme:?}");
                    }
                }
                appearances.push(t);
                let [a, b] = kind.signatures().map(|sig| sig.shape.name());
                assert_ne!(a, b, "{kind:?}");
                if t.style == MoveStyle::Anchor {
                    assert!(kind.signatures().into_iter()
                        .all(|sig| kind.engage_range_for(sig) >= 50.0), "{kind:?}");
                }
                for family in [a, b] {
                    assert!(!families.contains(&family), "{theme:?}: repeats {family}");
                    families.push(family);
                    *totals.entry(family).or_insert(0) += 1;
                }
            }
        }
        assert_eq!(totals.values().sum::<usize>(), 56);
        assert!(totals.values().all(|&count| count <= 6));
        for shape in [
            "Hollow ring", "Sweeping bar", "Returning cord", "Overlapping disks", "Slotted bar",
            "Crossing blades", "Live trail", "Falling drops", "Shadow cone",
        ] {
            assert!(totals.get(shape).copied().unwrap_or(0) >= 2, "{shape}");
        }
    }

    fn setup(theme: Theme, kind: MinibossKind, range: f32) -> (World, Entity) {
        let mut world = World::with_level(
            0, Level::empty(theme), 46, Loadout::starting(), CampaignTotals::default(),
        );
        world.ecs.resource_mut::<MinibossSchedule>().next = MINIBOSS_TIMES.len();
        world.player.god = true;
        let at = world.player.pos + X * range;
        let entity = world.spawn_miniboss(kind, 0, at);
        world.ecs.get_mut::<Miniboss>(entity).unwrap().announced = true;
        (world, entity)
    }

    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(miniboss_system).expect("miniboss system");
        w.end_tick();
    }

    fn motion(w: &World, entity: Entity) -> Motion {
        w.ecs.get::<Miniboss>(entity).unwrap().manoeuvre.motion
    }

    #[test]
    fn charge_traces_the_locked_lane_and_stops_at_the_cap() {
        let kind = MinibossKind::Corkscrew;
        let (mut w, entity) = setup(Theme::Sewer, kind, STANDOFF);
        tick(&mut w);
        let Motion::ChargeTell { from, end, .. } = motion(&w, entity) else {
            panic!("charge did not announce its line");
        };
        assert!((end.dist(from) - kind.temperament().speed * 3.0 * LUNGE_TIME).abs() < 0.01);
        w.player.pos += Vec2::new(0.0, 15.0);
        let mut lunge_ticks = 0;
        for _ in 0..60 {
            tick(&mut w);
            let body = w.ecs.get::<Enemy>(entity).unwrap();
            assert!((body.pos.y - from.y).abs() < 0.001);
            if motion(&w, entity) == Motion::Charged {
                assert!(body.pos.dist(end) < 0.01);
                assert_eq!(body.vel, Vec2::ZERO);
                assert!((lunge_ticks as f32 * DT - LUNGE_TIME).abs() <= DT);
                return;
            }
            if matches!(motion(&w, entity), Motion::Lunge { .. }) {
                lunge_ticks += 1;
            }
        }
        panic!("charge never stopped");
    }

    #[test]
    fn orbit_keeps_its_reach_and_changes_direction() {
        let kind = MinibossKind::Bobbin;
        let range = kind.engage_range_for(kind.signatures()[0]);
        let (mut w, entity) = setup(Theme::Laundry, kind, range);
        {
            let mut mb = w.ecs.get_mut::<Miniboss>(entity).unwrap();
            mb.state = MinibossState::Cooldown;
            mb.timer = -10.0;
        }
        let start = w.ecs.get::<Enemy>(entity).unwrap().pos;
        let mut sides = [false; 2];
        let mut previous = start;
        for _ in 0..420 {
            tick(&mut w);
            let body = w.ecs.get::<Enemy>(entity).unwrap();
            assert!((body.pos.dist(w.player.pos) - range).abs() < 6.0);
            let tangent = (w.player.pos - previous).normalized().perp();
            let step = (body.pos - previous).dot(tangent);
            if step.abs() > 0.15 {
                sides[usize::from(step > 0.0)] = true;
            }
            previous = body.pos;
        }
        assert_eq!(sides, [true, true]);
    }

    #[test]
    fn anchor_does_not_follow_a_rat_that_leaves_its_reach() {
        let (mut w, entity) = setup(Theme::Printworks, MinibossKind::LatePlate, 30.0);
        let mark = w.ecs.get::<Enemy>(entity).unwrap().pos;
        tick(&mut w);
        assert!(w.ecs.get::<Miniboss>(entity).unwrap().manoeuvre.anchored);
        w.player.pos -= X * 140.0;
        for _ in 0..400 {
            tick(&mut w);
            let body = w.ecs.get::<Enemy>(entity).unwrap();
            assert_eq!(body.pos, mark);
            assert_eq!(body.vel, Vec2::ZERO);
        }
    }

    #[test]
    fn burrow_is_immune_until_the_reachable_crack_resolves() {
        let (mut w, entity) = setup(Theme::Sewer, MinibossKind::BellDiver, 48.0);
        let mut crack_ticks = 0;
        let mut sunk_ticks = 0;
        let mut landing_at = None;
        for _ in 0..100 {
            tick(&mut w);
            match motion(&w, entity) {
                Motion::Sunk { .. } => sunk_ticks += 1,
                Motion::Crack { at, .. } => {
                    crack_ticks += 1;
                    landing_at = Some(at);
                }
                Motion::Surfaced => break,
                other => panic!("expected to be underground, got {other:?}"),
            }
            let mut body = w.ecs.get_mut::<Enemy>(entity).unwrap();
            let hp = body.hp;
            assert!(!body.targetable());
            body.hurt(hp * 2.0);
            assert_eq!(body.hurt_from(hp, Vec2::ZERO), 0.0);
            assert_eq!(body.hp, hp);
        }
        assert!((sunk_ticks as f32 * DT - SINK_TIME).abs() <= DT * 1.01);
        assert!((crack_ticks as f32 * DT - CRACK_TIME).abs() <= DT * 1.01);
        let body = w.ecs.get::<Enemy>(entity).unwrap();
        assert!(body.targetable());
        assert_eq!(Some(body.pos), landing_at);
        assert!((body.pos.dist(w.player.pos) - SURFACE_RANGE).abs() < 0.01);
        let seen = reachable(&w.level.tiles, cell_of(w.player.pos), SizeClass::Small);
        assert!(is_reached(&seen, &w.level.tiles, cell_of(body.pos)));
        tick(&mut w);
        assert_eq!(w.ecs.get::<Miniboss>(entity).unwrap().state, MinibossState::Tell);
    }

    fn summon_wave(w: &mut World, owner: Entity) {
        let mut mb = w.ecs.get_mut::<Miniboss>(owner).unwrap();
        mb.state = MinibossState::Active;
        mb.timer = mb.active_duration();
        mb.active_cued = true;
        mb.hit_this_strike = true;
        tick(w);
    }

    fn fodder(w: &mut World) -> Vec<(u64, Entity, Vec2)> {
        let mut query = w.ecs.query::<(Entity, &SpawnId, &Enemy, &MinibossSummon)>();
        let mut adds: Vec<_> = query.iter(&w.ecs).map(|(e, id, body, _)| (id.0, e, body.pos)).collect();
        adds.sort_unstable_by_key(|row| row.0);
        adds
    }

    #[test]
    fn summons_respect_the_owner_cap_and_pay_no_kill_credit() {
        let (mut w, owner) = setup(Theme::Printworks, MinibossKind::Foreman, 25.0);
        let center = w.ecs.get::<Enemy>(owner).unwrap().pos;
        for n in [3, 6, 9, 9] {
            summon_wave(&mut w, owner);
            let adds = fodder(&mut w);
            assert_eq!(adds.len(), n);
            assert_eq!(w.enemy_count(), 1);
            for (_, e, pos) in adds {
                assert_eq!(w.ecs.get::<Enemy>(e).unwrap().kind, EnemyKind::Silverfish);
                assert!((pos.dist(center) - SUMMON_RANGE).abs() < 0.01);
            }
        }
        let dead = fodder(&mut w)[0].1;
        w.ecs.get_mut::<Enemy>(dead).unwrap().hp = 0.0;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(crate::game::ecs::systems::loot::reap_enemies_system).unwrap();
        w.end_tick();
        assert_eq!((w.kills, w.score, w.gold), (0, 0, 0));
        assert!(w.gems().is_empty());
        summon_wave(&mut w, owner);
        assert_eq!(fodder(&mut w).len(), SUMMON_CAP);
        w.ecs.get_mut::<Enemy>(owner).unwrap().hp = 0.0;
        tick(&mut w);
        assert!(fodder(&mut w).is_empty());
    }

    #[test]
    fn attacks_alternate_immediately_or_start_with_secondary_below_the_threshold() {
        for (kind, hp, expected) in [
            (MinibossKind::Corkscrew, [100.0; 5], [0, 1, 0, 1, 0]),
            (MinibossKind::MilkCarton, [100.0, 50.0, 49.0, 70.0, 30.0], [0, 0, 1, 0, 1]),
        ] {
            let mut mb = Miniboss::new(kind, 0);
            for (hp, expected) in hp.into_iter().zip(expected) {
                mb.select_signature(hp, 100.0);
                assert_eq!(mb.signature(), kind.signatures()[expected]);
                mb.finish_signature();
            }
        }
    }

    #[test]
    fn planted_cycle_retains_damage_warning_and_recovery_and_uses_its_own_gap() {
        let kind = MinibossKind::MilkCarton;
        let (mut w, entity) = setup(Theme::Kitchen, kind, 20.0);
        w.player.god = false;
        w.player.hp = 1_000.0;
        let hp = w.player.hp;
        let mut counts = [0; 4];
        let mut began = false;
        let origin = w.ecs.get::<Enemy>(entity).unwrap().pos;
        for _ in 0..400 {
            w.player.iframes = 0.0;
            tick(&mut w);
            let mb = w.ecs.get::<Miniboss>(entity).unwrap();
            if counts[3] > 0 && mb.state == MinibossState::Tell {
                break;
            }
            began |= mb.state == MinibossState::Tell;
            if !began {
                continue;
            }
            if mb.holds() {
                assert_eq!(w.ecs.get::<Enemy>(entity).unwrap().pos, origin);
            }
            let slot = match mb.state {
                MinibossState::Tell => 0,
                MinibossState::Active => 1,
                MinibossState::Recover => 2,
                MinibossState::Cooldown => 3,
                MinibossState::Approach => unreachable!(),
            };
            counts[slot] += 1;
        }
        let seconds = counts.map(|count| count as f32 * DT);
        assert!((seconds[0] - TELL_TIME).abs() <= 2.0 * DT);
        assert!((seconds[1] - kind.signatures()[0].active).abs() <= 2.0 * DT);
        assert!((seconds[2] - RECOVER_TIME).abs() <= 2.0 * DT);
        assert!((seconds[2] + seconds[3] - kind.temperament().gap).abs() <= 3.0 * DT);
        let armor = w.player.loadout.stats.armor;
        assert!((hp - w.player.hp - (18.0 - armor).max(1.0)).abs() < 0.01);
    }

    #[test]
    fn twin_movement_and_summon_traces_leave_the_run_rng_untouched() {
        fn scenario() -> (World, Entity, Entity, Entity) {
            let (mut w, orbit) = setup(Theme::Printworks, MinibossKind::CarbonTwin, 28.0);
            {
                let mut mb = w.ecs.get_mut::<Miniboss>(orbit).unwrap();
                mb.state = MinibossState::Cooldown;
                mb.timer = -12.0;
            }
            let at = w.player.pos - X * 68.0;
            let charge = w.spawn_miniboss(MinibossKind::QuoinKey, 1, at);
            let mut mb = w.ecs.get_mut::<Miniboss>(charge).unwrap();
            mb.announced = true;
            mb.state = MinibossState::Cooldown;
            // A lunge can be sampled without entering the subsequent tell.
            mb.timer = mb.kind.temperament().gap - RECOVER_TIME - 0.8;
            let at = w.player.pos + Vec2::new(0.0, 56.0);
            let summoner = w.spawn_miniboss(MinibossKind::Foreman, 0, at);
            w.ecs.get_mut::<Miniboss>(summoner).unwrap().announced = true;
            (w, orbit, charge, summoner)
        }

        type MovementTrace = (Vec<(Vec2, Motion)>, Vec<(u64, Vec2)>, [u64; 3]);
        fn trace() -> MovementTrace {
            let (mut w, orbit, charge, summoner) = scenario();
            let mut untouched = scenario().0.rng;
            summon_wave(&mut w, summoner);
            let mut steps = Vec::new();
            for _ in 0..40 {
                tick(&mut w);
                for entity in [orbit, charge] {
                    steps.push((w.ecs.get::<Enemy>(entity).unwrap().pos, motion(&w, entity)));
                }
            }
            let rng = std::array::from_fn(|_| w.rng.random::<u64>());
            assert_eq!(rng, std::array::from_fn(|_| untouched.random::<u64>()));
            let adds = fodder(&mut w).into_iter().map(|(id, _, at)| (id, at)).collect();
            (steps, adds, rng)
        }
        assert_eq!(trace(), trace());
    }
}
