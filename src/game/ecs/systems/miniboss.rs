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
//! one is ever on the floor (the last one also retreats when the level's boss
//! arrives). On death it leaves exactly one chest (plus the shop table's
//! [`MINIBOSS_GOLD`]) at a reachable, non-pit floor position. The drop is made
//! by [`drop_loot`], which `loot::reap_dead` calls for a named corpse: **every**
//! kill path ends in that reap (weapons, traps, pools and other damage over
//! time, Wick bursts, and the Nuke's same-tick re-reap), so no source can skip
//! the chest and none can drop two.
//!
//! ## Rules against the rest of wave 3
//!
//! The body sets `Enemy::named`, which is how the `&Enemy` readers tell it
//! from a Brute: the Wind-up Mouse never lures it, a Porter never heals it, a
//! flatbed pit never drops it (it cannot be killed or stranded by a hole), and
//! Ratchet Tail never hooks it. It is a `Brute` body, so `EnemyKind::heavy()`
//! already holds for every other "heavy target" rule, directional armour
//! (the Pillbug's shell) never applies, and Hantavirus ticks it like any body.
//! The director keeps elites away from an appointment
//! (`director::ELITE_APPOINTMENT_GAP`).
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
use crate::game::director::BOSS_TIME;
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
/// Gold dropped with the chest (§6): the shop's gold table owns the number.
pub use crate::game::shop::MINIBOSS_GOLD;
/// Half-angle of the cone signature (a 45° cone).
pub const CONE_HALF_ANGLE: f32 = std::f32::consts::FRAC_PI_8; // 22.5°
/// Length of the cone.
pub const CONE_LEN: f32 = 36.0;
/// Length of the trench: the design's "dotted 48 × 10 px trench" (§8).
pub const TRENCH_LEN: f32 = 48.0;
/// Half-width of the trench (10 px across).
pub const TRENCH_HALF: f32 = 5.0;
/// Length of the swinging cord: the design's "44 px cord to a fixed spool".
pub const CORD_LEN: f32 = 44.0;
/// Half-thickness of the cord.
pub const CORD_HALF: f32 = 3.0;
/// Radius of the Bell Diver's ring: the design's "ring expands to 44 px at
/// 36 px/s" (§8).
pub const RING_RADIUS: f32 = 44.0;
/// Half-thickness of the ring.
pub const RING_HALF: f32 = 5.0;
/// Half-angle of the ring's gap (a 60° gap).
pub const RING_GAP_HALF: f32 = std::f32::consts::FRAC_PI_6; // 30°

/// How far the Bell Diver's gap sits from its locked aim, as a quarter turn.
///
/// §8 gives the ring a **fixed** 60° gap and tells the rat to "line up with its
/// chipped rim". The gap used to be centred on `mb.aim`, i.e. aimed at the rat:
/// the one safe sector was always the one the rat was standing in, so the
/// signature never had to be dodged. Offsetting it a quarter turn keeps it a
/// fixed, readable feature of the ring (never the rat's own line) while staying
/// derivable from the aim that was locked at tell start.
pub const RING_GAP_TURN: f32 = std::f32::consts::FRAC_PI_2;
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
                // A fixed gap a quarter turn off the locked aim, never aimed at
                // the rat (see `RING_GAP_TURN`).
                gap_dir: mb.aim.rotate(RING_GAP_TURN),
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
                gap_dir: mb.aim.rotate(RING_GAP_TURN),
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
    /// A representative centre of the footprint (the lighting pass puts the
    /// telegraph's warning pool here).
    pub fn center(&self) -> Vec2 {
        match *self {
            MinibossShape::Trench { a, b, .. } => (a + b) * 0.5,
            MinibossShape::Cone { origin, aim, len, .. } => origin + aim.normalized() * (len * 0.5),
            MinibossShape::Cord { spool, tip, .. } => (spool + tip) * 0.5,
            MinibossShape::RingGap { origin, .. } => origin,
            MinibossShape::Stamps { centers, .. } => centers[1],
            MinibossShape::Spill { center, .. } => center,
        }
    }

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

    /// Is the signature holding the body in place this tick? (`enemy_steering`
    /// skips the walk for a body whose own state machine owns its position,
    /// which lets the miniboss be *slowed* — an elite-grade body — instead of
    /// stunned, so no crowd control is cancelled by the appointment.)
    pub fn holds(&self) -> bool {
        matches!(self.state, MinibossState::Tell | MinibossState::Active)
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

/// The per-theme schedule: the `WAVE3_DESIGN.md` §8 table, row for row. The
/// match is exhaustive over `Theme`, so a new floor cannot ship without a row.
///
/// The **Flatbed** is not in the design table (it was added after it). It is a
/// last-stage getaway, so it takes the four signatures that read best on an
/// open deck with holes in it — the two straight lanes first (Corkscrew's
/// trench, the Foreman's stamps), then the two that sweep (Bobbin's cord, the
/// Bell Diver's ring) — and never the Milk Carton, whose wide spill would
/// cover a whole plank lane.
pub fn schedule_for(theme: Theme, appointment: usize) -> MinibossKind {
    use MinibossKind::*;
    let row = match theme {
        Theme::Sewer => [Corkscrew, BellDiver, MilkCarton, SisterSnip],
        Theme::Kitchen => [MilkCarton, SisterSnip, BellDiver, Foreman],
        Theme::Lab => [BellDiver, Corkscrew, Foreman, Bobbin],
        Theme::Laundry => [Bobbin, SisterSnip, Corkscrew, MilkCarton],
        Theme::Printworks => [Foreman, Bobbin, SisterSnip, BellDiver],
        Theme::Trailer => [Corkscrew, Foreman, Bobbin, BellDiver],
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
    e.named = true;
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
                // Nothing here touches `stun`/`slow_mult`: crowd control (the
                // Sonic Squeak's ring) is the rat's, not the appointment's.
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
                // Held in place by `enemy_steering_system` (`Miniboss::holds`),
                // not by a stun: a miniboss is an elite-grade body and takes
                // **slows**, so a Sonic Squeak ring must slow it exactly like
                // it slows an elite instead of being cancelled here.
                e.vel = Vec2::ZERO;
                if mb.timer >= TELL_TIME {
                    mb.state = MinibossState::Active;
                    mb.timer = 0.0;
                    mb.active_cued = false;
                }
            }
            MinibossState::Active => {
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
                if mb.timer >= RECOVER_TIME {
                    mb.state = MinibossState::Cooldown;
                    mb.timer = 0.0;
                }
            }
            MinibossState::Cooldown => {
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
    } else if elapsed >= BOSS_TIME {
        // The level's boss is the fifth appointment: a miniboss that outlived
        // the 12:00 slot retreats without loot when the boss is due, by the
        // same "one named enemy at a time" rule. No rng, no spawn.
        for (entity, e, _) in bosses.iter() {
            if e.alive() {
                commands.entity(entity).despawn();
                sfx.0.push(Sfx::MinibossRetreat);
            }
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

/// Drop a dead miniboss's loot: exactly one chest plus [`MINIBOSS_GOLD`], and
/// the 8 s of quiet after its death.
///
/// Called by `loot::reap_dead` for a named corpse, on the tick it is reaped.
/// Every kill path ends there — weapons, traps, acid pools, runoff and
/// Hantavirus ticks, a Wick burst, and the Nuke's same-tick re-reap — so the
/// chest is guaranteed and `chest_dropped` keeps it to one. A retreat despawns
/// the body without a reap and therefore pays nothing.
pub fn drop_loot(
    commands: &mut Commands,
    level: &Level,
    schedule: &mut MinibossSchedule,
    next_id: &mut NextSpawnId,
    elapsed: f32,
    corpse: Vec2,
    mb: &mut Miniboss,
) {
    if mb.chest_dropped {
        return;
    }
    mb.chest_dropped = true;
    schedule.grace_suppress_until = elapsed + SPAWN_GRACE_SUPPRESS;
    let pos = chest_floor_near(level, corpse);
    commands.queue(crate::game::ecs::components::ChestSpawn { pos });
    commands.spawn((Gold::new(pos, MINIBOSS_GOLD), next_id.take()));
}

/// Where a miniboss's chest lands: the corpse's own spot when the rat can walk
/// there, otherwise the nearest cell centre that it can.
///
/// "Can walk there" is three things: open floor for the rat's size class, part
/// of the map connected to the level's start (one flood fill, paid at most
/// four times a level), and not over a flatbed pit — a chest in a hole could
/// never be touched.
pub fn chest_floor_near(level: &Level, pos: Vec2) -> Vec2 {
    use crate::game::collision::{cell_of, TileMap};
    use crate::game::levelgen::validate::{is_reached, reachable};
    let tiles = &level.tiles;
    let seen = reachable(tiles, cell_of(level.start), SizeClass::Small);
    // ... and not on a floor hazard, a trap or a shop threshold: a chest is a
    // reward, so touching it must not cost a scald, a glue hold or a shop stop.
    let objects = &level.objects;
    let clear = |c: (i32, i32), at: Vec2| {
        const KEEP: f32 = 22.0;
        !objects.steam_grates.iter().any(|g| g.cell == c)
            && !objects.ink_pads.iter().any(|p| p.cells.contains(c.0, c.1))
            && !objects.sticky_traps.iter().any(|t| t.pos.dist_sq(at) < KEEP * KEEP)
            && !objects.mousetraps.iter().any(|t| t.pos.dist_sq(at) < KEEP * KEEP)
            && !objects.shop_holes.iter().any(|h| h.mouth == c || h.cell == c)
    };
    let ok = |c: (i32, i32), at: Vec2| {
        is_reached(&seen, tiles, c) && level.pit_at(at).is_none() && clear(c, at)
    };
    let cell = cell_of(pos);
    if ok(cell, pos) {
        return pos;
    }
    // Rings of growing Chebyshev radius; within a ring, the cell whose centre
    // is nearest the corpse (ties in scan order, so the result is a pure
    // function of the map and the position).
    for r in 0..=12i32 {
        let mut best: Option<(f32, Vec2)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue;
                }
                let c = (cell.0 + dx, cell.1 + dy);
                let centre = TileMap::tile_rect(c.0, c.1).center();
                if !ok(c, centre) {
                    continue;
                }
                let d = centre.dist_sq(pos);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, centre));
                }
            }
        }
        if let Some((_, centre)) = best {
            return centre;
        }
    }
    // Nothing walkable within twelve cells (not reachable on a generated map):
    // the start is always a legal chest spot.
    level.start
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

    /// §8's four geometry rows: "a dotted 48 × 10 px trench", "a 44 px cord",
    /// "a ring expands to 44 px", "a fixed 60° gap".
    #[test]
    fn signatures_match_the_design_geometry() {
        assert_eq!((TRENCH_LEN, TRENCH_HALF * 2.0), (48.0, 10.0), "trench 48 x 10");
        assert_eq!(CORD_LEN, 44.0, "cord 44 px");
        assert_eq!(RING_RADIUS, 44.0, "ring radius 44 px");
        assert!((RING_GAP_HALF * 2.0).to_degrees() - 60.0 < 1e-3, "60 degree gap");
    }

    /// The Bell Diver's gap is **fixed** (a quarter turn off the locked aim),
    /// not aimed at the rat: a rat standing on the tell's own line is not
    /// automatically in the safe sector.
    #[test]
    fn the_ring_gap_is_not_aimed_at_the_rat() {
        let m = mb(MinibossKind::BellDiver, Vec2::ZERO, Vec2::new(1.0, 0.0));
        let shape = m.kind.telegraph(&m);
        // On the rim, on the locked aim line (where a rat standing on the
        // tell's own line would be): outside the gap, so the ring bites.
        let aim_rim = Vec2::new(1.0, 0.0) * RING_RADIUS;
        assert!(
            shape.contains(aim_rim, 0.0),
            "the rat's own line must not be the bell's gap"
        );
        // The fixed gap (a quarter turn off the aim) is the escape.
        let gap_rim = Vec2::from_angle(RING_GAP_TURN) * RING_RADIUS;
        assert!(
            !shape.contains(gap_rim, 0.0),
            "the fixed gap sector is the escape"
        );
    }

    /// A signature holds the body without a `stun`, so the Sonic Squeak's
    /// control is not cancelled: the state machine sets `vel = 0` and reports
    /// `holds()`, and the crowd-control timers are the rat's business.
    #[test]
    fn a_signature_holds_the_body_without_stunning_it() {
        use crate::game::ecs::systems::miniboss::MinibossState;
        let mut m = Miniboss::new(MinibossKind::Corkscrew, 0);
        m.state = MinibossState::Tell;
        assert!(m.holds());
        m.state = MinibossState::Active;
        assert!(m.holds());
        m.state = MinibossState::Recover;
        assert!(!m.holds());
        m.state = MinibossState::Approach;
        assert!(!m.holds());
        m.state = MinibossState::Cooldown;
        assert!(!m.holds());
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
    fn schedule_is_the_design_table_and_total_over_every_playable_theme() {
        use MinibossKind::*;
        // WAVE3_DESIGN.md §8, row for row.
        let rows = [
            (Theme::Sewer, [Corkscrew, BellDiver, MilkCarton, SisterSnip]),
            (Theme::Kitchen, [MilkCarton, SisterSnip, BellDiver, Foreman]),
            (Theme::Lab, [BellDiver, Corkscrew, Foreman, Bobbin]),
            (Theme::Laundry, [Bobbin, SisterSnip, Corkscrew, MilkCarton]),
            (Theme::Printworks, [Foreman, Bobbin, SisterSnip, BellDiver]),
        ];
        for (theme, row) in rows {
            for (i, kind) in row.into_iter().enumerate() {
                assert_eq!(schedule_for(theme, i), kind, "{theme:?} appointment {i}");
            }
        }
        // Every playable floor (the Flatbed included) has four distinct names.
        for theme in Theme::playable() {
            let row: Vec<MinibossKind> = (0..4).map(|i| schedule_for(theme, i)).collect();
            for i in 0..4 {
                for j in 0..i {
                    assert_ne!(row[i], row[j], "{theme:?} repeats a miniboss");
                }
            }
            // Out-of-range appointments clamp to the last row.
            assert_eq!(schedule_for(theme, 9), row[3]);
        }
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

    /// Run the reap (and the Nuke's re-reap) once, as the schedule does.
    fn reap(w: &mut crate::game::world::World) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::loot::reap_enemies_system)
            .expect("reap");
        w.end_tick();
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
        w.elapsed = 200.0;
        reap(&mut w);
        assert_eq!(chests(&mut w), 1, "a miniboss kill must leave exactly one chest");
        assert_eq!(gold_values(&mut w), vec![crate::game::shop::MINIBOSS_GOLD]);
        assert_eq!(MINIBOSS_GOLD, 12, "§6: a miniboss pays 12 gold");
        assert!(w.ecs.get::<Enemy>(e).is_none(), "the corpse was not reaped");
        // The 8 s of quiet after the death.
        let s = w.ecs.resource::<MinibossSchedule>();
        assert!(s.suppress(200.0 + SPAWN_GRACE_SUPPRESS - 0.5));
        // A second reap must not produce a second chest.
        reap(&mut w);
        assert_eq!(chests(&mut w), 1, "the chest was queued twice");
        // Its gem carries the appointment's XP, not a Brute's.
        let mut gems = w.ecs.query::<&crate::game::gem::Gem>();
        assert!(gems.iter(&w.ecs).any(|g| g.value == appointment_xp(0)));
    }

    /// A Nuke **chunks** a named miniboss (35 % of its pool) instead of
    /// killing it — and when the rat then finishes it, the chest still drops
    /// exactly once, with the gold.
    #[test]
    fn a_nuked_miniboss_chunks_and_then_leaves_one_chest() {
        use crate::game::ecs::systems::miniboss::MinibossKind as K;
        use crate::game::gem::{Pickup, PickupKind};
        use crate::game::level::Theme;
        use crate::game::world::World;
        let mut w = World::new(Theme::Sewer, 11);
        // Past the opening so the director has nothing special queued; the
        // appointment table is parked so no second miniboss arrives.
        w.ecs.resource_mut::<MinibossSchedule>().next = MINIBOSS_TIMES.len();
        w.director.spawn_timer = 1.0e6;
        w.director.elite_timer = 1.0e6;
        let p = w.player.pos;
        let e = w.spawn_miniboss(K::BellDiver, 1, p + Vec2::new(60.0, 0.0));
        let hp0 = w.ecs.get::<Enemy>(e).unwrap().hp;
        let max = w.ecs.get::<Enemy>(e).unwrap().max_hp;
        w.spawn_pickup(Pickup { pos: p, kind: PickupKind::Nuke, bob: 0.0 });
        w.update(1.0 / 60.0, Vec2::ZERO, false);
        let hp1 = w.ecs.get::<Enemy>(e).map(|x| x.hp).unwrap_or(-1.0);
        assert!(hp1 > 0.0, "a Nuke must not one-shot an appointment ({hp0} -> {hp1})");
        assert!(
            (hp0 - hp1 - max * 0.35).abs() < 0.5,
            "the chunk should be 35 % of the pool"
        );
        assert!(chests(&mut w) == 0, "a surviving miniboss pays nothing yet");
        // Finish the fight: the chest and its gold land once.
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = 0.0;
        for _ in 0..2 {
            w.update(1.0 / 60.0, Vec2::ZERO, false);
        }
        assert_eq!(chests(&mut w), 1, "the finished appointment leaves exactly one chest");
        assert_eq!(gold_values(&mut w), vec![MINIBOSS_GOLD]);
        for _ in 0..30 {
            w.update(1.0 / 60.0, Vec2::ZERO, false);
        }
        assert!(chests(&mut w) <= 1, "a second chest appeared");
    }

    /// Damage over time (a Hantavirus mark's ticks, here) kills through the
    /// same reap: one chest, twelve gold.
    #[test]
    fn a_miniboss_killed_by_damage_over_time_leaves_one_chest() {
        use crate::game::ecs::systems::miniboss::MinibossKind as K;
        use crate::game::level::Theme;
        use crate::game::weapon::{Weapon, WeaponKind};
        use crate::game::world::World;
        let mut w = World::new(Theme::Sewer, 13);
        w.ecs.resource_mut::<MinibossSchedule>().next = MINIBOSS_TIMES.len();
        w.director.spawn_timer = 1.0e6;
        w.director.elite_timer = 1.0e6;
        // Hantavirus only: every point of damage below is a virus tick.
        w.player.loadout.weapons.clear();
        let mut virus = Weapon::new(WeaponKind::Hantavirus);
        virus.level = 5;
        w.player.loadout.weapons.push(virus);
        w.player.hp = 1.0e6;
        w.player.iframes = 1.0e9;
        let p = w.player.pos;
        let e = w.spawn_miniboss(K::Corkscrew, 0, p + Vec2::new(30.0, 0.0));
        // One tick from dead, so the first virus tick is the killing blow.
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = 1.0;
        let mut killed = false;
        for _ in 0..(20 * 60) {
            w.update(1.0 / 60.0, Vec2::ZERO, false);
            if w.ecs.get::<Enemy>(e).is_none() {
                killed = true;
                break;
            }
        }
        assert!(killed, "Hantavirus never ticked the miniboss down");
        assert_eq!(chests(&mut w), 1, "a DOT kill must leave exactly one chest");
        assert_eq!(gold_values(&mut w), vec![MINIBOSS_GOLD]);
    }

    /// The chest never lands in a wall, an unreachable pocket or a flatbed pit.
    #[test]
    fn the_chest_lands_on_reachable_non_pit_floor() {
        use crate::game::collision::cell_of;
        use crate::game::level::Theme;
        use crate::game::levelgen::validate::{is_reached, reachable};
        use crate::game::world::World;
        for theme in Theme::playable() {
            for seed in [3u64, 19, 404] {
                let w = World::new(theme, seed);
                let level = &w.level;
                let seen = reachable(&level.tiles, cell_of(level.start), SizeClass::Small);
                let cols = level.tiles.cols();
                let rows = level.tiles.rows();
                // Corpses everywhere: in walls, in pits, on open floor.
                for cy in (0..rows).step_by(7) {
                    for cx in (0..cols).step_by(7) {
                        let corpse = crate::game::collision::TileMap::tile_rect(cx, cy).center();
                        let at = chest_floor_near(level, corpse);
                        assert!(
                            is_reached(&seen, &level.tiles, cell_of(at)),
                            "{theme:?}/{seed}: chest at {at:?} is not reachable"
                        );
                        assert!(level.pit_at(at).is_none(), "{theme:?}/{seed}: chest in a pit");
                        let c = cell_of(at);
                        let o = &level.objects;
                        assert!(
                            !o.steam_grates.iter().any(|g| g.cell == c)
                                && !o.ink_pads.iter().any(|p| p.cells.contains(c.0, c.1)),
                            "{theme:?}/{seed}: chest on a floor hazard"
                        );
                        assert!(
                            !o.sticky_traps.iter().any(|t| t.pos.dist(at) < 22.0)
                                && !o.mousetraps.iter().any(|t| t.pos.dist(at) < 22.0),
                            "{theme:?}/{seed}: chest on a trap"
                        );
                        assert!(
                            !o.shop_holes.iter().any(|h| h.mouth == c),
                            "{theme:?}/{seed}: chest on a shop threshold"
                        );
                    }
                }
                // Every hazard, trap and shop threshold in particular.
                let spots: Vec<Vec2> = level
                    .objects
                    .steam_grates
                    .iter()
                    .map(|g| g.pos)
                    .chain(level.objects.ink_pads.iter().map(|p| p.pos))
                    .chain(level.objects.sticky_traps.iter().map(|t| t.pos))
                    .chain(level.objects.shop_holes.iter().map(|h| h.face()))
                    .collect();
                for spot in spots {
                    let at = chest_floor_near(level, spot);
                    let c = cell_of(at);
                    let o = &level.objects;
                    assert!(
                        !o.steam_grates.iter().any(|g| g.cell == c)
                            && !o.ink_pads.iter().any(|p| p.cells.contains(c.0, c.1))
                            && !o.sticky_traps.iter().any(|t| t.pos.dist(at) < 22.0)
                            && !o.shop_holes.iter().any(|h| h.mouth == c),
                        "{theme:?}/{seed}: a corpse on a hazard left its chest on one"
                    );
                }
                // Every pit centre in particular.
                for pit in &level.trailer.pits {
                    let at = chest_floor_near(level, pit.center_world());
                    assert!(level.pit_at(at).is_none(), "{theme:?}/{seed}: chest left in a pit");
                }
            }
        }
    }

    /// The merged wave-3 rules a named body must be exempt from.
    #[test]
    fn a_miniboss_is_heavy_unlured_unhealed_and_never_falls() {
        use crate::game::ecs::systems::miniboss::MinibossKind as K;
        use crate::game::ecs::systems::powerups::decoy_lures_body;
        use crate::game::level::Theme;
        use crate::game::world::World;
        let mut w = World::new(Theme::Trailer, 5);
        let p = w.player.pos;
        let e = w.spawn_miniboss(K::Foreman, 0, p + Vec2::new(40.0, 0.0));
        {
            let body = w.ecs.get::<Enemy>(e).unwrap();
            assert!(body.named);
            assert!(body.kind.heavy(), "a miniboss must count as a heavy target");
            assert!(!decoy_lures_body(body), "the Wind-up Mouse lured a miniboss");
            // Directional armour is the Pillbug's alone.
            assert_eq!(body.damage_mult_from(p), 1.0);
        }
        // Stand it on a pit: it neither takes fall damage nor is moved.
        if let Some(pit) = w.level.trailer.pits.first() {
            let centre = pit.center_world();
            {
                let mut body = w.ecs.get_mut::<Enemy>(e).unwrap();
                body.pos = centre;
            }
            let hp = w.ecs.get::<Enemy>(e).unwrap().hp;
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::trailer::trailer_system)
                .expect("trailer system");
            w.end_tick();
            let body = w.ecs.get::<Enemy>(e).unwrap();
            assert_eq!(body.hp, hp, "a pit hurt a miniboss");
            assert_eq!(body.pos, centre, "a pit moved a miniboss");
        } else {
            panic!("the flatbed generated no pit to test against");
        }
    }

    /// The level's boss is the fifth appointment: the 12:00 miniboss retreats
    /// without loot when it is due.
    #[test]
    fn a_surviving_miniboss_retreats_when_the_boss_is_due() {
        use crate::game::ecs::systems::miniboss::MinibossKind as K;
        use crate::game::level::Theme;
        use crate::game::world::World;
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = BOSS_TIME + 0.1;
        w.ecs.resource_mut::<MinibossSchedule>().next = MINIBOSS_TIMES.len();
        let p = w.player.pos;
        let old = w.spawn_miniboss(K::SisterSnip, 3, p + Vec2::new(30.0, 0.0));
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(miniboss_system).expect("miniboss system");
        w.end_tick();
        assert!(w.ecs.get::<Enemy>(old).is_none(), "the miniboss stayed for the boss");
        assert_eq!(chests(&mut w), 0);
        assert!(gold_values(&mut w).is_empty());
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
