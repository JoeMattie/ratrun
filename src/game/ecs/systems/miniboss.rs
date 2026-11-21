//! Named minibosses (`WAVE3_DESIGN.md` §8): four appointments per level, each
//! with one distinct signature shape, a 1 s tell, an exposed recovery and a
//! chest on death.
//!
//! A miniboss is an ordinary [`Enemy`] body (kind [`EnemyKind::Brute`] so the
//! weapons, traps, bullets and contact rules already target it) carrying a
//! [`Miniboss`] component. The component owns the signature state machine; the
//! ordinary steering still walks the body toward the rat during the approach
//! and the cooldown. The signature is a *shape* ([`MinibossShape`]) resolved
//! against the rat directly — no boss-framework hazards, so the determinism
//! contract stays one ordered chain.
//!
//! Timings: `TELL_TIME` 1 s → active window (per kind) → `RECOVER_TIME` 1 s
//! exposed → cooldown so the next tell is 4 s after the last signature ended.
//! Aim is locked at the tell's first tick. A signature whose shape leaves no
//! walkable escape is re-aimed (up to 8 tries) and skipped if still trapped.
//!
//! Appointments: `MINIBOSS_TIMES` = 3:00, 6:00, 9:00, 12:00. A surviving
//! miniboss retreats without loot when the next appointment arrives, so only
//! one is ever on the floor. On death it leaves exactly one chest (plus 12
//! gold) at a reachable floor position — the chest is queued by
//! [`miniboss_chest_system`], which runs just before the reap, so a kill by a
//! trap or damage over time still drops it.
//!
//! Spawn grace: 20 s at half spawn rate after an arrival and 8 s with no fresh
//! ordinary spawns after a death ([`MinibossSchedule`]).
//!
//! ## The six signatures (§8)
//!
//! | Name | Shape |
//! |---|---|
//! | Corkscrew | a trench that extends from the tail over 0.5 s |
//! | Sister Snip | a 45° cone |
//! | Bobbin | a cord that swings through an arc |
//! | Bell Diver | a ring with a gap |
//! | The Foreman | three 12 px stamps |
//! | Milk Carton | a spill rectangle |
//!
//! Every shape is exact: see [`MinibossShape::contains`] and its tests.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;

use crate::audio::Sfx;
use crate::game::collision::SizeClass;
use crate::game::director::MINIBOSS_TIMES;
use crate::game::ecs::resources::{
    NextEnemyId, NextSpawnId, RngRes, RunCounters, SfxQueue, TickInput, Viewport,
};
use crate::game::ecs::systems::spawn::{spawn_point_reachable, SpawnScratch};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::Gold;
use crate::game::level::{Level, Theme};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

/// Seconds between the appointment and the first signature.
pub const TELL_TIME: f32 = 1.0;
/// Seconds the rat has to punish a signature.
pub const RECOVER_TIME: f32 = 1.0;
/// Seconds between one signature ending and the next tell starting.
pub const SIGNATURE_GAP: f32 = 4.0;
/// Seconds of half-rate ordinary spawning after a miniboss arrives.
pub const SPAWN_GRACE_HALF: f32 = 20.0;
/// Seconds with no fresh ordinary spawns after a miniboss dies.
pub const SPAWN_GRACE_SUPPRESS: f32 = 8.0;
/// Gold dropped with the chest (§6).
pub const MINIBOSS_GOLD: u32 = 12;
/// Half-angle of the cone signature (a 45° cone).
pub const CONE_HALF_ANGLE: f32 = std::f32::consts::FRAC_PI_8; // 22.5°
/// Length of the cone.
pub const CONE_LEN: f32 = 36.0;
/// Length of the trench.
pub const TRENCH_LEN: f32 = 40.0;
/// Half-width of the trench.
pub const TRENCH_HALF: f32 = 4.0;
/// Length of the swinging cord.
pub const CORD_LEN: f32 = 48.0;
/// Half-thickness of the cord.
pub const CORD_HALF: f32 = 3.0;
/// Radius of the Bell Diver's ring.
pub const RING_RADIUS: f32 = 32.0;
/// Half-thickness of the ring.
pub const RING_HALF: f32 = 5.0;
/// Half-angle of the ring's gap (a 60° gap).
pub const RING_GAP_HALF: f32 = std::f32::consts::FRAC_PI_6; // 30°
/// Half-size of a Foreman stamp (12 px square).
pub const STAMP_HALF: f32 = 6.0;
/// Spacing between the three stamps.
pub const STAMP_SPACING: f32 = 16.0;
/// Length of the Milk Carton's spill.
pub const SPILL_LEN: f32 = 32.0;
/// Half-width of the spill.
pub const SPILL_HALF: f32 = 10.0;
/// How close the rat must be before the miniboss starts its tell.
pub const ENGAGE_RANGE: f32 = 70.0;

/// The six named minibosses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinibossKind {
    Corkscrew,
    SisterSnip,
    Bobbin,
    BellDiver,
    Foreman,
    MilkCarton,
}

impl MinibossKind {
    pub fn name(self) -> &'static str {
        match self {
            MinibossKind::Corkscrew => "Corkscrew",
            MinibossKind::SisterSnip => "Sister Snip",
            MinibossKind::Bobbin => "Bobbin",
            MinibossKind::BellDiver => "Bell Diver",
            MinibossKind::Foreman => "The Foreman",
            MinibossKind::MilkCarton => "Milk Carton",
        }
    }

    /// Seconds the active window lasts.
    pub fn active_duration(self) -> f32 {
        match self {
            MinibossKind::Corkscrew => 0.5,
            MinibossKind::SisterSnip => 0.8,
            MinibossKind::Bobbin => 0.7,
            MinibossKind::BellDiver => 1.2,
            MinibossKind::Foreman => 0.9,
            MinibossKind::MilkCarton => 1.2,
        }
    }

    /// Silhouette radius for the placeholder art.
    pub fn body_radius(self) -> f32 {
        match self {
            MinibossKind::Corkscrew => 7.0,
            MinibossKind::SisterSnip => 7.0,
            MinibossKind::Bobbin => 6.5,
            MinibossKind::BellDiver => 7.0,
            MinibossKind::Foreman => 7.5,
            MinibossKind::MilkCarton => 7.0,
        }
    }

    /// The telegraph outline drawn during the tell.
    pub fn telegraph(self, mb: &Miniboss) -> MinibossShape {
        match self {
            MinibossKind::Corkscrew => MinibossShape::Trench {
                a: mb.origin,
                b: mb.origin + mb.aim * TRENCH_LEN,
                half: TRENCH_HALF,
            },
            MinibossKind::SisterSnip => MinibossShape::Cone {
                origin: mb.origin,
                aim: mb.aim,
                half_angle: CONE_HALF_ANGLE,
                len: CONE_LEN,
            },
            MinibossKind::Bobbin => MinibossShape::Cord {
                spool: mb.origin,
                tip: mb.origin + mb.aim * CORD_LEN,
                half: CORD_HALF,
            },
            MinibossKind::BellDiver => MinibossShape::RingGap {
                origin: mb.origin,
                radius: RING_RADIUS,
                half: RING_HALF,
                gap_dir: mb.aim,
                gap_half: RING_GAP_HALF,
            },
            MinibossKind::Foreman => MinibossShape::Stamps {
                centers: mb.stamp_centers,
                half: STAMP_HALF,
            },
            MinibossKind::MilkCarton => MinibossShape::Spill {
                center: mb.origin + mb.aim * (SPILL_LEN * 0.5),
                aim: mb.aim,
                half_len: SPILL_LEN * 0.5,
                half_w: SPILL_HALF,
            },
        }
    }

    /// The active hit shape at `t` seconds into the active window, or `None`
    /// when the window has expired.
    pub fn active_shape(self, mb: &Miniboss, t: f32) -> Option<MinibossShape> {
        let d = self.active_duration();
        if t < 0.0 || t > d {
            return None;
        }
        let p = (t / d).clamp(0.0, 1.0);
        Some(match self {
            MinibossKind::Corkscrew => MinibossShape::Trench {
                a: mb.origin,
                b: mb.origin + mb.aim * (TRENCH_LEN * p),
                half: TRENCH_HALF,
            },
            MinibossKind::SisterSnip => MinibossShape::Cone {
                origin: mb.origin,
                aim: mb.aim,
                half_angle: CONE_HALF_ANGLE,
                len: CONE_LEN,
            },
            MinibossKind::Bobbin => {
                // The cord swings from straight ahead through a 90° arc.
                let angle = -std::f32::consts::FRAC_PI_4 + p * std::f32::consts::FRAC_PI_2;
                let dir = mb.aim.rotate(angle);
                MinibossShape::Cord {
                    spool: mb.origin,
                    tip: mb.origin + dir * CORD_LEN,
                    half: CORD_HALF,
                }
            }
            MinibossKind::BellDiver => MinibossShape::RingGap {
                origin: mb.origin,
                radius: RING_RADIUS * p,
                half: RING_HALF,
                gap_dir: mb.aim,
                gap_half: RING_GAP_HALF,
            },
            MinibossKind::Foreman => {
                // One stamp at a time, 0.3 s apart.
                let idx = ((t / 0.3).floor() as usize).min(2);
                MinibossShape::Stamps {
                    centers: [mb.stamp_centers[idx], mb.stamp_centers[idx], mb.stamp_centers[idx]],
                    half: STAMP_HALF,
                }
            }
            MinibossKind::MilkCarton => MinibossShape::Spill {
                center: mb.origin + mb.aim * (SPILL_LEN * 0.5),
                aim: mb.aim,
                half_len: SPILL_LEN * 0.5,
                half_w: SPILL_HALF,
            },
        })
    }
}

/// A distinct signature footprint. All units are world pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MinibossShape {
    /// A straight trench from `a` to `b` with half-width `half`.
    Trench { a: Vec2, b: Vec2, half: f32 },
    /// A cone from `origin` in `aim`, `half_angle` either side, `len` long.
    Cone {
        origin: Vec2,
        aim: Vec2,
        half_angle: f32,
        len: f32,
    },
    /// A swinging cord from `spool` to `tip` with half-thickness `half`.
    Cord { spool: Vec2, tip: Vec2, half: f32 },
    /// A ring with a gap: centre `origin`, radius `radius`, thickness `half`,
    /// gap centred on `gap_dir` with `gap_half` half-angle.
    RingGap {
        origin: Vec2,
        radius: f32,
        half: f32,
        gap_dir: Vec2,
        gap_half: f32,
    },
    /// Three stamps, each a square of half-size `half`.
    Stamps { centers: [Vec2; 3], half: f32 },
    /// A spill rectangle: centre `center`, long axis `aim`, half-length
    /// `half_len`, half-width `half_w`.
    Spill {
        center: Vec2,
        aim: Vec2,
        half_len: f32,
        half_w: f32,
    },
}

impl MinibossShape {
    /// Does a circle of `radius` at `p` overlap this shape?
    pub fn contains(&self, p: Vec2, radius: f32) -> bool {
        match *self {
            MinibossShape::Trench { a, b, half } => segment_dist(p, a, b) <= half + radius,
            MinibossShape::Cone {
                origin,
                aim,
                half_angle,
                len,
            } => {
                let v = p - origin;
                let d = v.len();
                if d > len + radius {
                    return false;
                }
                if d <= 1e-4 {
                    return true;
                }
                // Widen the angular test by the circle's angular radius so a
                // rat-sized body is hit when its edge grazes the cone.
                let ang = v.normalized().dot(aim.normalized()).clamp(-1.0, 1.0).acos();
                let margin = (radius / d).clamp(0.0, 1.0).asin();
                ang <= half_angle + margin
            }
            MinibossShape::Cord { spool, tip, half } => segment_dist(p, spool, tip) <= half + radius,
            MinibossShape::RingGap {
                origin,
                radius: r,
                half,
                gap_dir,
                gap_half,
            } => {
                let v = p - origin;
                let d = v.len();
                if (d - r).abs() > half + radius {
                    return false;
                }
                if d <= 1e-4 {
                    return false;
                }
                let ang = v.normalized().dot(gap_dir.normalized()).clamp(-1.0, 1.0).acos();
                ang > gap_half
            }
            MinibossShape::Stamps { centers, half } => centers.iter().any(|&c| {
                let v = p - c;
                v.x.abs() <= half + radius && v.y.abs() <= half + radius
            }),
            MinibossShape::Spill {
                center,
                aim,
                half_len,
                half_w,
            } => {
                let v = p - center;
                let along = v.dot(aim.normalized());
                let across = v.dot(aim.normalized().perp());
                along.abs() <= half_len + radius && across.abs() <= half_w + radius
            }
        }
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.len_sq();
    if len2 <= 1e-8 {
        return p.dist(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.dist(a + ab * t)
}

/// The miniboss's signature state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinibossState {
    Approach,
    Tell,
    Active,
    Recover,
    Cooldown,
}

/// The miniboss component on an ordinary `Enemy` body.
#[derive(Component, Clone, Debug)]
pub struct Miniboss {
    pub kind: MinibossKind,
    pub appointment: usize,
    pub state: MinibossState,
    pub timer: f32,
    /// The locked origin of the signature (the body's position at tell start).
    pub origin: Vec2,
    /// The locked aim direction at tell start.
    pub aim: Vec2,
    /// The three stamp centres, locked at tell start.
    pub stamp_centers: [Vec2; 3],
    /// One-shot arrival cue.
    pub announced: bool,
    /// One-shot tell cue for this signature.
    pub tell_cued: bool,
    /// One-shot active cue for this signature.
    pub active_cued: bool,
    /// Set once the chest has been queued.
    pub chest_dropped: bool,
    /// XP this miniboss leaves (§1 budget).
    pub xp: u32,
    /// Damage of one signature hit.
    pub signature_damage: f32,
}

impl Miniboss {
    pub fn new(kind: MinibossKind, appointment: usize) -> Self {
        Miniboss {
            kind,
            appointment,
            state: MinibossState::Approach,
            timer: 0.0,
            origin: Vec2::ZERO,
            aim: Vec2::new(1.0, 0.0),
            stamp_centers: [Vec2::ZERO; 3],
            announced: false,
            tell_cued: false,
            active_cued: false,
            chest_dropped: false,
            xp: appointment_xp(appointment),
            signature_damage: appointment_signature_damage(appointment),
        }
    }

    /// The telegraph outline currently drawn, if any.
    pub fn telegraph(&self) -> Option<MinibossShape> {
        match self.state {
            MinibossState::Tell | MinibossState::Active => Some(self.kind.telegraph(self)),
            _ => None,
        }
    }

    /// The active hit shape, if the signature is live.
    pub fn active_shape(&self) -> Option<MinibossShape> {
        if self.state != MinibossState::Active {
            return None;
        }
        self.kind.active_shape(self, self.timer)
    }
}

/// HP by appointment (§8): 160 / 320 / 540 / 780.
#[inline]
pub fn appointment_hp(appointment: usize) -> f32 {
    match appointment {
        0 => 160.0,
        1 => 320.0,
        2 => 540.0,
        _ => 780.0,
    }
}

/// Signature damage by appointment (§8): 14 / 16 / 18 / 20.
#[inline]
pub fn appointment_signature_damage(appointment: usize) -> f32 {
    match appointment {
        0 => 14.0,
        1 => 16.0,
        2 => 18.0,
        _ => 20.0,
    }
}

/// XP by appointment (§1): 25 / 40 / 55 / 70.
#[inline]
pub fn appointment_xp(appointment: usize) -> u32 {
    match appointment {
        0 => 25,
        1 => 40,
        2 => 55,
        _ => 70,
    }
}

/// The per-theme schedule (§8 table). Laundry and Printworks rows are keyed so
/// they compile when those themes land; anything else gets the default row.
pub fn schedule_for(theme: Theme, appointment: usize) -> MinibossKind {
    let row = match theme {
        Theme::Sewer => [
            MinibossKind::Corkscrew,
            MinibossKind::SisterSnip,
            MinibossKind::Bobbin,
            MinibossKind::BellDiver,
        ],
        Theme::Kitchen => [
            MinibossKind::BellDiver,
            MinibossKind::Bobbin,
            MinibossKind::Foreman,
            MinibossKind::MilkCarton,
        ],
        Theme::Lab => [
            MinibossKind::MilkCarton,
            MinibossKind::Foreman,
            MinibossKind::Corkscrew,
            MinibossKind::SisterSnip,
        ],
    };
    row[appointment.min(3)]
}

/// Appointment schedule + spawn grace state. Lives on the ECS world for the
/// whole run and is read by the director and the HUD.
#[derive(Resource, Default, Clone, Debug)]
pub struct MinibossSchedule {
    /// Index into [`MINIBOSS_TIMES`] of the next appointment.
    pub next: usize,
    /// Ordinary spawn interval is doubled until this clock.
    pub grace_half_until: f32,
    /// No fresh ordinary spawns until this clock.
    pub grace_suppress_until: f32,
    /// How many minibosses are alive right now.
    pub active: u32,
}

impl MinibossSchedule {
    /// The spawn-interval multiplier the director should apply this tick.
    #[inline]
    pub fn spawn_interval_mult(&self, elapsed: f32) -> f32 {
        if elapsed < self.grace_half_until {
            2.0
        } else {
            1.0
        }
    }

    /// Should the director hold all fresh ordinary spawns this tick?
    #[inline]
    pub fn suppress(&self, elapsed: f32) -> bool {
        elapsed < self.grace_suppress_until
    }

    /// The next appointment's clock, or `None` after the last one.
    #[inline]
    pub fn next_appointment(&self) -> Option<f32> {
        MINIBOSS_TIMES.get(self.next).copied()
    }
}

/// Spawn a miniboss on connected floor just outside the view, then let the
/// ordinary steering walk it in. Returns the spawned entity (via Commands).
fn spawn_miniboss(
    commands: &mut Commands,
    kind: MinibossKind,
    appointment: usize,
    pos: Vec2,
    next_id: &mut NextSpawnId,
    next_enemy_id: &mut NextEnemyId,
    rng: &mut StdRng,
) {
    // A Brute body carries the miniboss so every existing weapon, trap and
    // bullet path already targets it. The stats are overwritten below.
    let mut e = Enemy::spawn(EnemyKind::Brute, pos, 1.0, rng);
    e.hp = appointment_hp(appointment);
    e.max_hp = e.hp;
    e.radius = kind.body_radius();
    e.damage = 8.0;
    // Faster than an ordinary brute so the approach reads as a named arrival.
    e.speed_scale = 2.4;
    e.elite = false;
    e.id = next_enemy_id.take();
    commands.spawn((e, Miniboss::new(kind, appointment), next_id.take()));
}

/// The miniboss tick: appointments, retreats, the signature state machine and
/// the damage it deals. Runs early in the chain so the stun it sets stops this
/// tick's steering, and so a spawn lands before the grid build.
pub fn miniboss_system(
    mut commands: Commands,
    input: Res<TickInput>,
    counters: ResMut<RunCounters>,
    mut schedule: ResMut<MinibossSchedule>,
    level: Res<Level>,
    viewport: Res<Viewport>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut next_id: ResMut<NextSpawnId>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut sfx: ResMut<SfxQueue>,
    mut particles: ResMut<Particles>,
    mut scratch: Local<SpawnScratch>,
    mut bosses: Query<(Entity, &mut Enemy, &mut Miniboss)>,
) {
    let dt = input.dt;
    let elapsed = counters.elapsed;
    let ppos = player.pos;

    // 1. Tick the living minibosses.
    let mut alive = 0u32;
    for (_, mut e, mut mb) in bosses.iter_mut() {
        if !e.alive() {
            continue;
        }
        alive += 1;
        // The arrival cue fires once, on the first tick the body exists.
        if !mb.announced {
            mb.announced = true;
            sfx.0.push(Sfx::MinibossArrive);
            crate::game::particle::vfx::emit_miniboss_arrive(
                &mut particles,
                &mut rng.0,
                e.pos,
                level.palette.accent,
            );
        }
        mb.timer += dt;
        match mb.state {
            MinibossState::Approach => {
                // Let the ordinary steering walk it in; just watch the range.
                e.stun = 0.0;
                if e.pos.dist(ppos) <= ENGAGE_RANGE {
                    begin_tell(
                        &mut mb,
                        &mut e,
                        ppos,
                        &level,
                        &mut sfx,
                        &mut particles,
                        &mut rng.0,
                    );
                }
            }
            MinibossState::Tell => {
                e.stun = 1.0; // hold position
                e.vel = Vec2::ZERO;
                if mb.timer >= TELL_TIME {
                    mb.state = MinibossState::Active;
                    mb.timer = 0.0;
                    mb.active_cued = false;
                }
            }
            MinibossState::Active => {
                e.stun = 1.0;
                e.vel = Vec2::ZERO;
                if !mb.active_cued {
                    mb.active_cued = true;
                    sfx.0.push(Sfx::MinibossTell);
                }
                if let Some(shape) = mb.active_shape() {
                    // The hit lands once per activation: the rat's i-frames
                    // would block a repeat anyway, but the flag keeps the
                    // semantics explicit.
                    if shape.contains(ppos, crate::game::ecs::constants::PLAYER_RADIUS)
                        && player.take_damage(mb.signature_damage)
                    {
                        sfx.0.push(Sfx::Hurt);
                        crate::game::particle::vfx::emit_player_hurt(
                            &mut particles,
                            &mut rng.0,
                            ppos,
                        );
                    }
                }
                if mb.timer >= mb.kind.active_duration() {
                    mb.state = MinibossState::Recover;
                    mb.timer = 0.0;
                }
            }
            MinibossState::Recover => {
                // Exposed: the rat can hit it freely.
                e.stun = 0.0;
                if mb.timer >= RECOVER_TIME {
                    mb.state = MinibossState::Cooldown;
                    mb.timer = 0.0;
                }
            }
            MinibossState::Cooldown => {
                e.stun = 0.0;
                if mb.timer >= SIGNATURE_GAP - RECOVER_TIME {
                    mb.state = MinibossState::Approach;
                    mb.timer = 0.0;
                }
            }
        }
    }
    schedule.active = alive;

    // 2. Appointments. A surviving miniboss retreats without loot when the
    //    next one arrives, so only one is ever on the floor.
    if let Some(at) = schedule.next_appointment() {
        if elapsed >= at {
            // Retreat every living miniboss (no loot, no chest).
            let mut retreat = Vec::new();
            for (entity, e, _) in bosses.iter() {
                if e.alive() {
                    retreat.push(entity);
                }
            }
            for entity in retreat {
                commands.entity(entity).despawn();
                sfx.0.push(Sfx::MinibossRetreat);
            }
            // Spawn the new one just outside the view.
            scratch.sync(&level);
            if let Some(pos) = spawn_point_reachable(&mut rng.0, &level, &player, viewport.0, &mut scratch) {
                let kind = schedule_for(level.theme, schedule.next);
                spawn_miniboss(
                    &mut commands,
                    kind,
                    schedule.next,
                    pos,
                    &mut next_id,
                    &mut next_enemy_id,
                    &mut rng.0,
                );
                schedule.grace_half_until = elapsed + SPAWN_GRACE_HALF;
            }
            schedule.next += 1;
        }
    }
}

/// Enter the tell: lock the aim, stamp centres and telegraph, and cue the tell.
fn begin_tell(
    mb: &mut Miniboss,
    e: &mut Enemy,
    ppos: Vec2,
    level: &Level,
    sfx: &mut SfxQueue,
    particles: &mut Particles,
    rng: &mut StdRng,
) {
    let aim = (ppos - e.pos).normalized();
    let aim = if aim.len_sq() < 0.5 {
        Vec2::new(1.0, 0.0)
    } else {
        aim
    };
    mb.origin = e.pos;
    mb.aim = aim;
    // Three stamps in a line from the miniboss toward the rat.
    mb.stamp_centers = [
        e.pos + aim * STAMP_SPACING,
        e.pos + aim * (STAMP_SPACING * 2.0),
        e.pos + aim * (STAMP_SPACING * 3.0),
    ];
    // Reposition when the signature leaves no walkable escape: try rotating
    // the aim until a sample direction is clear.
    let mut tries = 0;
    while tries < 8 && !escape_exists(mb, ppos, level) {
        mb.aim = mb.aim.rotate(std::f32::consts::FRAC_PI_4);
        mb.stamp_centers = [
            e.pos + mb.aim * STAMP_SPACING,
            e.pos + mb.aim * (STAMP_SPACING * 2.0),
            e.pos + mb.aim * (STAMP_SPACING * 3.0),
        ];
        tries += 1;
    }
    if tries >= 8 {
        // Still trapped: skip this signature and go straight to cooldown.
        mb.state = MinibossState::Cooldown;
        mb.timer = 0.0;
        return;
    }
    mb.state = MinibossState::Tell;
    mb.timer = 0.0;
    mb.tell_cued = false;
    if !mb.tell_cued {
        mb.tell_cued = true;
        sfx.0.push(Sfx::MinibossTell);
        crate::game::particle::vfx::emit_miniboss_tell(
            particles,
            rng,
            e.pos,
            (255, 226, 140),
        );
    }
}

/// Is there at least one walkable direction out of the rat's position that the
/// telegraph does not cover? Sampled at 8 compass points.
fn escape_exists(mb: &Miniboss, ppos: Vec2, level: &Level) -> bool {
    let shape = mb.kind.telegraph(mb);
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::TAU / 8.0;
        let dir = Vec2::from_angle(ang);
        let sample = ppos + dir * 12.0;
        if !shape.contains(sample, crate::game::ecs::constants::PLAYER_RADIUS) {
            let cell = crate::game::collision::cell_of(sample);
            if level.tiles.get(cell.0, cell.1).passable(SizeClass::Small) {
                return true;
            }
        }
    }
    false
}

/// Drops exactly one chest (plus 12 gold) on the tick a miniboss dies. Runs
/// just before the reap, so a kill by a trap or damage over time still counts.
pub fn miniboss_chest_system(
    mut commands: Commands,
    mut schedule: ResMut<MinibossSchedule>,
    counters: ResMut<RunCounters>,
    level: Res<Level>,
    mut next_id: ResMut<NextSpawnId>,
    mut bosses: Query<(&Enemy, &mut Miniboss)>,
) {
    for (e, mut mb) in bosses.iter_mut() {
        if e.alive() || mb.chest_dropped {
            continue;
        }
        mb.chest_dropped = true;
        schedule.grace_suppress_until = counters.elapsed + SPAWN_GRACE_SUPPRESS;
        // Snap to a reachable floor position near the corpse.
        let pos = reachable_floor_near(&level, e.pos);
        commands.queue(crate::game::ecs::components::ChestSpawn { pos });
        commands.spawn((Gold::new(pos, MINIBOSS_GOLD), next_id.take()));
    }
}

/// The nearest walkable cell centre to `pos` (falls back to `pos`).
fn reachable_floor_near(level: &Level, pos: Vec2) -> Vec2 {
    let cell = crate::game::collision::cell_of(pos);
    if level.tiles.get(cell.0, cell.1).passable(SizeClass::Small) {
        return pos;
    }
    for r in 1..=4i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                let c = (cell.0 + dx, cell.1 + dy);
                if level.tiles.get(c.0, c.1).passable(SizeClass::Small) {
                    return crate::game::collision::TileMap::tile_rect(c.0, c.1).center();
                }
            }
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(kind: MinibossKind, origin: Vec2, aim: Vec2) -> Miniboss {
        let mut m = Miniboss::new(kind, 0);
        m.origin = origin;
        m.aim = aim;
        m.stamp_centers = [
            origin + aim * STAMP_SPACING,
            origin + aim * (STAMP_SPACING * 2.0),
            origin + aim * (STAMP_SPACING * 3.0),
        ];
        m
    }

    #[test]
    fn trench_hit_shape_is_exact() {
        let m = mb(MinibossKind::Corkscrew, Vec2::ZERO, Vec2::new(1.0, 0.0));
        let s = MinibossShape::Trench {
            a: Vec2::ZERO,
            b: Vec2::new(TRENCH_LEN, 0.0),
            half: TRENCH_HALF,
        };
        assert!(s.contains(Vec2::new(20.0, 0.0), 0.0));
        assert!(s.contains(Vec2::new(20.0, TRENCH_HALF - 0.1), 0.0));
        assert!(!s.contains(Vec2::new(20.0, TRENCH_HALF + 1.0), 0.0));
        assert!(!s.contains(Vec2::new(TRENCH_LEN + TRENCH_HALF + 1.0, 0.0), 0.0));
        let _ = m;
    }

    #[test]
    fn cone_hit_shape_is_exact() {
        let s = MinibossShape::Cone {
            origin: Vec2::ZERO,
            aim: Vec2::new(1.0, 0.0),
            half_angle: CONE_HALF_ANGLE,
            len: CONE_LEN,
        };
        assert!(s.contains(Vec2::new(20.0, 0.0), 0.0));
        // A point just inside the 22.5° edge at 20 px.
        let edge = Vec2::from_angle(CONE_HALF_ANGLE - 0.01) * 20.0;
        assert!(s.contains(edge, 0.0));
        // A point just outside.
        let outside = Vec2::from_angle(CONE_HALF_ANGLE + 0.05) * 20.0;
        assert!(!s.contains(outside, 0.0));
        assert!(!s.contains(Vec2::new(CONE_LEN + 2.0, 0.0), 0.0));
    }

    #[test]
    fn cord_hit_shape_is_exact() {
        let s = MinibossShape::Cord {
            spool: Vec2::ZERO,
            tip: Vec2::new(CORD_LEN, 0.0),
            half: CORD_HALF,
        };
        assert!(s.contains(Vec2::new(20.0, 0.0), 0.0));
        assert!(s.contains(Vec2::new(20.0, CORD_HALF - 0.1), 0.0));
        assert!(!s.contains(Vec2::new(20.0, CORD_HALF + 1.0), 0.0));
        assert!(!s.contains(Vec2::new(CORD_LEN + CORD_HALF + 1.0, 0.0), 0.0));
    }

    #[test]
    fn ring_with_a_gap_hit_shape_is_exact() {
        let s = MinibossShape::RingGap {
            origin: Vec2::ZERO,
            radius: RING_RADIUS,
            half: RING_HALF,
            gap_dir: Vec2::new(1.0, 0.0),
            gap_half: RING_GAP_HALF,
        };
        // On the ring, away from the gap.
        assert!(s.contains(Vec2::new(0.0, RING_RADIUS), 0.0));
        // Inside the gap.
        assert!(!s.contains(Vec2::new(RING_RADIUS, 0.0), 0.0));
        // Too close to the centre.
        assert!(!s.contains(Vec2::new(0.0, RING_RADIUS - RING_HALF - 2.0), 0.0));
        // Too far out.
        assert!(!s.contains(Vec2::new(0.0, RING_RADIUS + RING_HALF + 2.0), 0.0));
    }

    #[test]
    fn three_stamps_hit_shape_is_exact() {
        let m = mb(MinibossKind::Foreman, Vec2::ZERO, Vec2::new(1.0, 0.0));
        let s = MinibossShape::Stamps {
            centers: m.stamp_centers,
            half: STAMP_HALF,
        };
        assert!(s.contains(m.stamp_centers[0], 0.0));
        assert!(s.contains(m.stamp_centers[1], 0.0));
        assert!(s.contains(m.stamp_centers[2], 0.0));
        // Between stamps is not covered.
        let between = m.stamp_centers[0] + Vec2::new(STAMP_SPACING * 0.5, 0.0);
        assert!(!s.contains(between, 0.0));
    }

    #[test]
    fn spill_hit_shape_is_exact() {
        let s = MinibossShape::Spill {
            center: Vec2::new(SPILL_LEN * 0.5, 0.0),
            aim: Vec2::new(1.0, 0.0),
            half_len: SPILL_LEN * 0.5,
            half_w: SPILL_HALF,
        };
        assert!(s.contains(Vec2::new(SPILL_LEN * 0.5, 0.0), 0.0));
        assert!(s.contains(Vec2::new(SPILL_LEN * 0.5, SPILL_HALF - 0.1), 0.0));
        assert!(!s.contains(Vec2::new(SPILL_LEN * 0.5, SPILL_HALF + 1.0), 0.0));
        assert!(!s.contains(Vec2::new(SPILL_LEN + 2.0, 0.0), 0.0));
    }

    #[test]
    fn schedule_is_per_theme() {
        assert_eq!(schedule_for(Theme::Sewer, 0), MinibossKind::Corkscrew);
        assert_eq!(schedule_for(Theme::Sewer, 3), MinibossKind::BellDiver);
        assert_eq!(schedule_for(Theme::Kitchen, 0), MinibossKind::BellDiver);
        assert_eq!(schedule_for(Theme::Lab, 0), MinibossKind::MilkCarton);
        // Out-of-range appointments clamp to the last row.
        assert_eq!(schedule_for(Theme::Sewer, 9), MinibossKind::BellDiver);
    }

    #[test]
    fn appointment_stats_match_the_design() {
        assert_eq!(appointment_hp(0), 160.0);
        assert_eq!(appointment_hp(3), 780.0);
        assert_eq!(appointment_signature_damage(0), 14.0);
        assert_eq!(appointment_signature_damage(3), 20.0);
        assert_eq!(appointment_xp(0), 25);
        assert_eq!(appointment_xp(3), 70);
    }

    #[test]
    fn tell_is_before_damage() {
        // The active shape only exists in the Active state; the telegraph is
        // drawn in Tell and Active. A miniboss in Tell cannot damage.
        let mut m = mb(MinibossKind::Corkscrew, Vec2::ZERO, Vec2::new(1.0, 0.0));
        m.state = MinibossState::Tell;
        assert!(m.telegraph().is_some());
        assert!(m.active_shape().is_none());
        m.state = MinibossState::Active;
        m.timer = 0.25;
        assert!(m.active_shape().is_some());
    }

    #[test]
    fn a_miniboss_death_drops_one_chest_and_twelve_gold() {
        use crate::game::ecs::systems::miniboss::MinibossKind as K;
        use crate::game::level::Theme;
        use crate::game::world::World;
        let mut w = World::new(Theme::Sewer, 7);
        let p = w.player.pos;
        let e = w.spawn_miniboss(K::Corkscrew, 0, p + Vec2::new(40.0, 0.0));
        // Any damage source — trap, pool, bullet — only has to drive hp to zero.
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = 0.0;
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(miniboss_chest_system).expect("chest system");
        w.end_tick();
        assert_eq!(chests(&mut w), 1, "a miniboss kill must leave exactly one chest");
        assert_eq!(gold_values(&mut w), vec![MINIBOSS_GOLD]);
        // A second tick must not queue a second chest.
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(miniboss_chest_system).expect("chest system");
        w.end_tick();
        assert_eq!(chests(&mut w), 1, "the chest was queued twice");
    }

    #[test]
    fn a_surviving_miniboss_retreats_without_loot_at_the_next_appointment() {
        use crate::game::ecs::systems::miniboss::MinibossKind as K;
        use crate::game::level::Theme;
        use crate::game::world::World;
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = crate::game::director::MINIBOSS_TIMES[0] + 0.5;
        w.ecs.resource_mut::<MinibossSchedule>().next = 0;
        let p = w.player.pos;
        let old = w.spawn_miniboss(K::Corkscrew, 0, p + Vec2::new(30.0, 0.0));
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(miniboss_system).expect("miniboss system");
        w.end_tick();
        assert!(w.ecs.get::<Enemy>(old).is_none(), "the old miniboss did not retreat");
        assert!(w.miniboss_marker().is_some(), "no new miniboss spawned");
        assert_eq!(chests(&mut w), 0, "a retreat dropped a chest");
        assert!(gold_values(&mut w).is_empty(), "a retreat dropped gold");
    }

    fn chests(w: &mut crate::game::world::World) -> usize {
        let mut q = w.ecs.query::<&crate::game::gem::Chest>();
        q.iter(&w.ecs).count()
    }

    fn gold_values(w: &mut crate::game::world::World) -> Vec<u32> {
        let mut q = w.ecs.query::<&Gold>();
        q.iter(&w.ecs).map(|g| g.value).collect()
    }

    #[test]
    fn grace_windows() {
        let s = MinibossSchedule {
            grace_half_until: 20.0,
            grace_suppress_until: 8.0,
            ..MinibossSchedule::default()
        };
        assert_eq!(s.spawn_interval_mult(10.0), 2.0);
        assert_eq!(s.spawn_interval_mult(20.0), 1.0);
        assert!(s.suppress(4.0));
        assert!(!s.suppress(8.0));
    }
}
