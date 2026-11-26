//! Named minibosses: four appointments per sector, with two strikes and a
//! temperament per body. The table lives in `miniboss_behaviour`. Tells last
//! 1 s and track the rat for their first half. The body stays planted through
//! the tell, active window and exposed recovery. The next tell can start once
//! the body's own gap has elapsed and its movement is ready.
//!
//! Return and Drip latch each pass or drop separately. Wake can strike from
//! the locked tell. A tell with no walkable escape is re-aimed or skipped.
//!
//! The movement and signature changes intentionally affect seeded play once
//! an appointment arrives. The 60 s harness ends before the first appointment.
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
//! A tell that leaves no walkable escape is re-aimed (up to eight tries),
//! then skipped if it still traps the rat. Tell, strike, Library framing and
//! bot danger read the same world-space shape.
//! On the Outfall, the lock samples the actual commitment instant and checks
//! the rat's whole escape path for walls, tidal drops and the planted body.
//! The Shade commits to its live lamp direction, and The False Bulb to the
//! nearest lure. If those fixed warnings leave no escape, the strike is skipped.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;

use super::miniboss_behaviour::{self as behaviour, Manoeuvre, Motion, MoveStyle, MovementParams};

use crate::audio::Sfx;
use crate::detmath::DetF32;
use crate::game::collision::SizeClass;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::director::MINIBOSS_TIMES;
use crate::game::ecs::resources::{
    NextEnemyId, NextSpawnId, RatTrack, RngRes, RunCounters, SfxQueue, TickInput, Viewport,
};
use super::miniboss_shapes::{self as shapes, SignatureTrack, WakeTrail};
use crate::game::ecs::systems::spawn::{in_maze, spawn_point_reachable, widest_view, SpawnScratch};
use crate::game::director::BOSS_TIME;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::Gold;
use crate::game::hazards::clamp_outfall_displacement;
use crate::game::level::{Level, Theme};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

/// Seconds between the appointment and the first signature.
pub const TELL_TIME: f32 = 1.0;
/// The planted body updates its aim until this point in the tell. The
/// outline then stays fixed so the rat can react to it.
///
/// A tell that locked on its first frame never had to be answered: the body
/// stood still for the whole second while a rat at walking pace covered 58 px,
/// further than any body-anchored shape reaches, so a rat that simply kept
/// moving was missed every time (about 2,800 signatures to 28 hits in the
/// batch that found it, nearly all of them rings). The warning is still the
/// full [`TELL_TIME`]; the last half of it is the committed, readable part.
pub const TELL_LOCK: f32 = 0.5;
/// The fastest rat the tracking aim leads (px/s): a walk with a few speed
/// stacks. A dash is not led, so it always clears the shape.
pub const TELL_LEAD_SPEED_MAX: f32 = 72.0;
/// Seconds the rat has to punish a signature.
pub const RECOVER_TIME: f32 = 1.0;
/// A named body's contact damage. The signature is the threat; the body is
/// what makes standing on it a bad idea. (At 8, with the faster walk, the body
/// alone was a fifth of all damage taken in a batch and the signatures under
/// one percent.)
pub const MINIBOSS_CONTACT_DAMAGE: f32 = 7.5;
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
/// The radius the Late Plate's closing ring finishes at. With its 2.5 px
/// half-width and the rat's 3 px body, anything at or under 5.5 bites a rat
/// that stood still on the locked centre; 5 leaves half a pixel of margin.
pub const LATE_PLATE_CLOSED: f32 = 5.0;
/// Half-size of a Foreman stamp (12 px square).
pub const STAMP_HALF: f32 = 6.0;
/// Spacing between the three stamps.
pub const STAMP_SPACING: f32 = 16.0;
/// Length of the Milk Carton's spill.
pub const SPILL_LEN: f32 = 32.0;
/// Half-width of the spill.
pub const SPILL_HALF: f32 = 10.0;
/// The furthest any miniboss starts its tell from: the range of the rows whose
/// signature is anchored on the rat (Cage Counter, Road Atlas, Late Plate) and
/// the cap for everything else. See [`MinibossKind::engage_range`].
pub const ENGAGE_RANGE: f32 = 70.0;
/// A body-anchored signature tells once the rat is within this fraction of its
/// reach, so a rat that simply stops when the tell sounds is still inside the
/// footprint and has to make the row's answer.
pub const ENGAGE_REACH_FRACTION: f32 = 0.7;

/// The named minibosses. The first twenty-four fill the original six floors;
/// the Outfall's appointments follow them in arrival order.
///
/// **Append-only.** `library::Key::Miniboss` uses the discriminant as the
/// inside-category index, so a new variant must go at the end and an old
/// variant must never move: a save's bestiary bits index this list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinibossKind {
    // ---- wave 3: the six that already existed ---------------------------
    Corkscrew,
    SisterSnip,
    Bobbin,
    BellDiver,
    Foreman,
    MilkCarton,
    // ---- wave 4: the eighteen that fill the rest of the table -----------
    ValveWidow,
    Mudlark,
    PastryWheel,
    SaltHen,
    PotBrush,
    DosePump,
    PetriMother,
    CageCounter,
    RotorTechnician,
    SudsBailiff,
    WringerBoy,
    QuoinKey,
    CarbonTwin,
    LatePlate,
    TarpaulinPup,
    AxleNut,
    ChainDog,
    RoadAtlas,
    Gasket,
    Swell,
    Shade,
    FalseBulb,
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
            MinibossKind::ValveWidow => "Valve Widow",
            MinibossKind::Mudlark => "The Mudlark",
            MinibossKind::PastryWheel => "Pastry Wheel",
            MinibossKind::SaltHen => "Salt Hen",
            MinibossKind::PotBrush => "Pot Brush",
            MinibossKind::DosePump => "Dose Pump",
            MinibossKind::PetriMother => "Petri Mother",
            MinibossKind::CageCounter => "Cage Counter",
            MinibossKind::RotorTechnician => "Rotor Technician",
            MinibossKind::SudsBailiff => "Suds Bailiff",
            MinibossKind::WringerBoy => "The Wringer Boy",
            MinibossKind::QuoinKey => "Quoin Key",
            MinibossKind::CarbonTwin => "Carbon Twin",
            MinibossKind::LatePlate => "Late Plate",
            MinibossKind::TarpaulinPup => "Tarpaulin Pup",
            MinibossKind::AxleNut => "Axle Nut",
            MinibossKind::ChainDog => "Chain Dog",
            MinibossKind::RoadAtlas => "The Road Atlas",
            MinibossKind::Gasket => "The Gasket",
            MinibossKind::Swell => "The Swell",
            MinibossKind::Shade => "The Shade",
            MinibossKind::FalseBulb => "The False Bulb",
        }
    }

    /// Every named miniboss, in enum order. The console's `miniboss` command,
    /// its Tab completion, the Library's Named Minibosses section and the
    /// roster test all read this table rather than a retyped list.
    pub fn all() -> [MinibossKind; 28] {
        [
            MinibossKind::Corkscrew,
            MinibossKind::SisterSnip,
            MinibossKind::Bobbin,
            MinibossKind::BellDiver,
            MinibossKind::Foreman,
            MinibossKind::MilkCarton,
            MinibossKind::ValveWidow,
            MinibossKind::Mudlark,
            MinibossKind::PastryWheel,
            MinibossKind::SaltHen,
            MinibossKind::PotBrush,
            MinibossKind::DosePump,
            MinibossKind::PetriMother,
            MinibossKind::CageCounter,
            MinibossKind::RotorTechnician,
            MinibossKind::SudsBailiff,
            MinibossKind::WringerBoy,
            MinibossKind::QuoinKey,
            MinibossKind::CarbonTwin,
            MinibossKind::LatePlate,
            MinibossKind::TarpaulinPup,
            MinibossKind::AxleNut,
            MinibossKind::ChainDog,
            MinibossKind::RoadAtlas,
            MinibossKind::Gasket,
            MinibossKind::Swell,
            MinibossKind::Shade,
            MinibossKind::FalseBulb,
        ]
    }

    pub(super) fn is_outfall(self) -> bool {
        matches!(self, Self::Gasket | Self::Swell | Self::Shade | Self::FalseBulb)
    }

    /// Seconds the active window lasts. One source of truth: the row's
    /// [`Signature`].
    #[inline]
    pub fn active_duration(self) -> f32 {
        self.signatures()[0].active
    }

    /// Collision radius (§8: every named body is 6.5–7.5 px).
    pub fn body_radius(self) -> f32 {
        match self {
            MinibossKind::Corkscrew => 7.0,
            MinibossKind::SisterSnip => 7.0,
            MinibossKind::Bobbin => 6.5,
            MinibossKind::BellDiver => 7.0,
            MinibossKind::Foreman => 7.5,
            MinibossKind::MilkCarton => 7.0,
            MinibossKind::ValveWidow => 7.0,
            MinibossKind::Mudlark => 7.5,
            MinibossKind::PastryWheel => 6.5,
            MinibossKind::SaltHen => 7.0,
            MinibossKind::PotBrush => 7.0,
            MinibossKind::DosePump => 6.5,
            MinibossKind::PetriMother => 7.5,
            MinibossKind::CageCounter => 7.0,
            MinibossKind::RotorTechnician => 7.5,
            MinibossKind::SudsBailiff => 7.0,
            MinibossKind::WringerBoy => 7.5,
            MinibossKind::QuoinKey => 6.5,
            MinibossKind::CarbonTwin => 7.0,
            MinibossKind::LatePlate => 7.5,
            MinibossKind::TarpaulinPup => 6.5,
            MinibossKind::AxleNut => 7.0,
            MinibossKind::ChainDog => 7.0,
            MinibossKind::RoadAtlas => 7.5,
            MinibossKind::Gasket => 7.0,
            MinibossKind::Swell => 7.5,
            MinibossKind::Shade => 7.0,
            MinibossKind::FalseBulb => 7.0,
        }
    }

    /// How close the body walks before it starts its tell.
    ///
    /// Every tell used to start at [`ENGAGE_RANGE`] (70 px), but a signature
    /// anchored on the body reaches 30–50 px: a rat that stood still when the
    /// tell sounded was outside nearly every shape. A body-anchored row now
    /// closes to [`ENGAGE_REACH_FRACTION`] of its own reach first; the rows
    /// anchored on the rat's locked position keep the full range.
    pub fn engage_range(self) -> f32 {
        self.engage_range_for(self.signatures()[0])
    }

    pub fn engage_range_for(self, sig: Signature) -> f32 {
        use SignatureShape as S;
        let reach = match sig.shape {
            S::Trench { len, .. }
            | S::Cone { len, .. }
            | S::Cord { len, .. }
            | S::Spill { len, .. }
            | S::Pincer { len, .. }
            | S::Lane { len, .. } => len,
            S::Ring {
                from,
                to,
                on_rat: false,
                ..
            } => from.max(to),
            S::Ring { on_rat: true, .. } => return ENGAGE_RANGE,
            S::Stamps { half, layout } => match layout {
                // Three pads out from the body: the far edge of the third.
                StampLayout::Along { spacing } => spacing * 3.0 + half,
                StampLayout::AroundRat { .. } | StampLayout::V { .. } => return ENGAGE_RANGE,
            },
            S::Annulus { outer, .. } if self == Self::Gasket => GASKET_OFFSET + outer,
            S::Annulus { .. } | S::Overlap { .. } | S::Comb { .. } | S::Wake { .. } | S::Drip { .. } => {
                return ENGAGE_RANGE;
            }
            S::Tide { travel, half, .. } => travel + half,
            S::Return { len, half, .. } => len + half,
            S::Shear { len, .. } | S::Umbra { len, .. } => len,
            S::False { forward, len, .. } => forward + len,
        };
        if self == Self::Shade && matches!(sig.shape, S::Umbra { .. }) {
            // The warned emergence is 40 px out. It must be able to cast
            // from there when the rat stays within the wedge's full reach.
            return reach;
        }
        (reach * ENGAGE_REACH_FRACTION).clamp(self.body_radius() + 10.0, ENGAGE_RANGE)
    }

    /// The tell's outline: the whole hazard area, drawn for the full
    /// [`TELL_TIME`] before the strike starts. It is derived from the same
    /// row data as [`Self::active_shape`], so the two can never disagree.
    pub fn telegraph(self, mb: &Miniboss) -> MinibossShape {
        match mb.signature().shape {
            SignatureShape::Trench { len, half } => MinibossShape::Trench {
                a: mb.origin,
                b: mb.origin + mb.aim * len,
                half,
            },
            SignatureShape::Cone {
                half_angle,
                len,
            } => MinibossShape::Cone {
                origin: mb.origin,
                aim: mb.aim,
                half_angle,
                len,
            },
            // A swinging cord's footprint is the whole sector it sweeps, not
            // the one line it lies on mid-swing: `sweep/2` either side of the
            // locked aim, out to the cord's far edge, widened by the angle its
            // half-thickness subtends at the tip. (The tell used to be the
            // single mid-swing cord, so a 150° blade struck 75° either side of
            // the line the player, the bot and `escape_exists` were shown.)
            // Close to the hub the cord's own width pokes up to `half` px past
            // this wedge — inside the body's contact disc, where the rat is
            // already being hurt.
            SignatureShape::Cord { len, half, sweep } => MinibossShape::Cone {
                origin: mb.origin,
                aim: mb.aim,
                half_angle: sweep * 0.5 + (half / len.max(1.0)).clamp(0.0, 1.0).dasin(),
                len: len + half,
            },
            SignatureShape::Ring {
                from,
                to,
                half,
                gap_half,
                on_rat,
            } => MinibossShape::RingGap {
                origin: if on_rat { mb.rat_lock() } else { mb.origin },
                // The widest the ring ever gets: the whole hazard area.
                radius: from.max(to),
                half,
                // A fixed gap a quarter turn off the locked aim, never aimed
                // at the rat (see `RING_GAP_TURN`).
                gap_dir: mb.aim.rotate(RING_GAP_TURN),
                gap_half,
            },
            SignatureShape::Stamps { half, .. } => MinibossShape::Stamps {
                centers: mb.stamp_centers,
                half,
            },
            SignatureShape::Spill { len, half_w } => MinibossShape::Spill {
                center: mb.origin + mb.aim * (len * 0.5),
                aim: mb.aim,
                half_len: len * 0.5,
                half_w,
            },
            SignatureShape::Pincer {
                len,
                half,
                open,
                closed,
            } => MinibossShape::Pincer {
                origin: mb.origin,
                aim: mb.aim,
                len,
                half,
                // The claws' whole travel: open at the tell, closed at the
                // end of the strike, and everything between.
                gap_lo: closed,
                gap_hi: open,
            },
            SignatureShape::Lane {
                len,
                seam_half,
                half,
                ..
            } => MinibossShape::Lane {
                origin: mb.origin,
                aim: mb.aim,
                len,
                seam_half,
                half,
                // The tell marks the whole lane: one bar as long as the lane,
                // running its full length, with the seam free.
                bar_len: len,
                covered: len,
            },
            SignatureShape::False { forward, across, len, half } => {
                false_shape(mb, forward, across, len, half, false)
            }
            SignatureShape::Annulus { inner, outer } if self == Self::Gasket => {
                MinibossShape::Annulus { origin: mb.origin + mb.aim * GASKET_OFFSET, inner, outer }
            }
            shape => shapes::resolve(shape, mb, None),
        }
    }

    /// The active hit shape at `t` seconds into the active window, or `None`
    /// when the window has expired.
    pub fn active_shape(self, mb: &Miniboss, t: f32) -> Option<MinibossShape> {
        let sig = mb.signature();
        let d = sig.active;
        if t < 0.0 || t > d {
            return None;
        }
        let p = (t / d).clamp(0.0, 1.0);
        Some(match sig.shape {
            SignatureShape::Trench { len, half } => MinibossShape::Trench {
                a: mb.origin,
                b: mb.origin + mb.aim * (len * p),
                half,
            },
            SignatureShape::Cone {
                half_angle,
                len,
            } => MinibossShape::Cone {
                origin: mb.origin,
                aim: mb.aim,
                half_angle,
                len,
            },
            SignatureShape::Cord { len, half, sweep } => {
                // The cord sweeps from `-sweep/2` to `+sweep/2` about the aim.
                let angle = -sweep * 0.5 + p * sweep;
                let dir = mb.aim.rotate(angle);
                MinibossShape::Cord {
                    spool: mb.origin,
                    tip: mb.origin + dir * len,
                    half,
                }
            }
            SignatureShape::Ring {
                from,
                to,
                half,
                gap_half,
                on_rat,
            } => MinibossShape::RingGap {
                origin: if on_rat { mb.rat_lock() } else { mb.origin },
                radius: from + (to - from) * p,
                half,
                gap_dir: mb.aim.rotate(RING_GAP_TURN),
                gap_half,
            },
            SignatureShape::Stamps { half, .. } => {
                // One stamp at a time, `STAMP_STEP` apart, in the order the
                // layout locked them.
                let idx = ((t / STAMP_STEP).floor() as usize).min(2);
                MinibossShape::Stamps {
                    centers: [mb.stamp_centers[idx], mb.stamp_centers[idx], mb.stamp_centers[idx]],
                    half,
                }
            }
            SignatureShape::Spill { len, half_w } => MinibossShape::Spill {
                center: mb.origin + mb.aim * (len * 0.5),
                aim: mb.aim,
                half_len: len * 0.5,
                half_w,
            },
            SignatureShape::Pincer {
                len,
                half,
                open,
                closed,
            } => {
                let gap = open + (closed - open) * p;
                MinibossShape::Pincer {
                    origin: mb.origin,
                    aim: mb.aim,
                    len,
                    half,
                    gap_lo: gap,
                    gap_hi: gap,
                }
            }
            SignatureShape::Lane {
                len,
                seam_half,
                half,
                bar_len,
            } => MinibossShape::Lane {
                origin: mb.origin,
                aim: mb.aim,
                len,
                seam_half,
                half,
                bar_len,
                // The pair rolls from the near end to clear the far end: the
                // leading edge travels the lane plus the bar's own length.
                covered: (len + bar_len) * p,
            },
            SignatureShape::Wake { delay, .. } if TELL_TIME - TELL_LOCK + t < delay => return None,
            SignatureShape::False { forward, across, len, half } => {
                false_shape(mb, forward, across, len, half, true)
            }
            SignatureShape::Annulus { inner, outer } if self == Self::Gasket => {
                MinibossShape::Annulus { origin: mb.origin + mb.aim * GASKET_OFFSET, inner, outer }
            }
            shape => shapes::resolve(shape, mb, Some((p, t))),
        })
    }
}

/// The shared evaluator one row's signature uses, with that row's own numbers.
///
/// Every row is *data*: [`MinibossKind::telegraph`] and
/// [`MinibossKind::active_shape`] are the only two places that turn these into
/// geometry, so the tell and the strike can never drift apart, and the bot,
/// the ink pass and the renderer read the same [`MinibossShape`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignatureShape {
    /// A trench from the body to its tip that fills over the active window.
    Trench { len: f32, half: f32 },
    /// A cone from the body along the locked aim.
    Cone { half_angle: f32, len: f32 },
    /// A cord from the body to its tip, swinging `sweep` radians through the
    /// locked aim (`-sweep/2 .. +sweep/2`). Behind the pivot is always safe.
    Cord { len: f32, half: f32, sweep: f32 },
    /// A ring with one gap, growing (or shrinking) from radius `from` to
    /// `to`. `on_rat` centres it on the rat's locked position instead of the
    /// body (the closing ring). The gap is a fixed quarter turn off the aim.
    Ring {
        from: f32,
        to: f32,
        half: f32,
        gap_half: f32,
        on_rat: bool,
    },
    /// Three stamps, marked together for the tell and landed one at a time,
    /// [`STAMP_STEP`] apart, in the layout's landing order.
    Stamps { half: f32, layout: StampLayout },
    /// A rectangle laid from the body along the locked aim.
    Spill { len: f32, half_w: f32 },
    /// Two claws astride the aim, `gap_lo..gap_hi` apart centre to centre and
    /// `len` long: an open tip along the aim is always safe.
    Pincer {
        len: f32,
        half: f32,
        open: f32,
        closed: f32,
    },
    /// Two `bar_len`-long bars down a `len`-long lane with a
    /// `2·seam_half`-wide seam down the middle. They roll from the body's end
    /// to clear the far end over the active window. The seam is always safe.
    Lane {
        len: f32,
        seam_half: f32,
        half: f32,
        bar_len: f32,
    },
    Annulus { inner: f32, outer: f32 },
    Tide { len: f32, half: f32, gap: f32, travel: f32 },
    Return { len: f32, half: f32, sweep: f32 },
    Overlap { radius: f32, sep: f32 },
    Comb { len: f32, half: f32, tooth_gap: f32, wide_gap: f32 },
    Shear { len: f32, half: f32, channel: f32 },
    Wake { delay: f32, window: f32, half: f32 },
    Drip { radius: f32 },
    Umbra { half_angle: f32, len: f32 },
    /// Two lateral bulbs with a rectangular jaw extending from either one.
    False { forward: f32, across: f32, len: f32, half: f32 },
}

/// Where a row's three stamps sit, relative to what the tell locked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StampLayout {
    /// Three pads `spacing` apart along the locked aim from the body.
    Along { spacing: f32 },
    /// The rat's locked position, then `across` either side of it. Landing
    /// order: left, right, centre.
    AroundRat { across: f32 },
    /// A V: the tip on the rat's locked position, the other two `forward`
    /// ahead and `across` either side. Landing order: left, tip, right.
    V { forward: f32, across: f32 },
}

/// One row's signature: the active window in seconds and the footprint it
/// uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signature {
    pub active: f32,
    pub shape: SignatureShape,
}

/// Seconds between one stamp landing and the next.
pub const STAMP_STEP: f32 = 0.3;
const GASKET_OFFSET: f32 = 16.0;
const SHADE_SCONCE_REACH: f32 = 140.0;

fn false_shape(mb: &Miniboss, forward: f32, across: f32, jaw_len: f32, jaw_half: f32, active: bool)
    -> MinibossShape
{
    let origin = mb.origin;
    let aim = mb.aim.normalized();
    let mid = origin + aim * forward;
    let offset = aim.perp() * across;
    let bulbs = [mid - offset, mid + offset];
    let committed = active || mb.state == MinibossState::Active || mb.timer >= TELL_LOCK;
    // The perpendicular bisector is the aim line. Its zero projects to
    // the left bulb, and stored `target` is the unled rat at lock.
    let selected = committed.then(|| usize::from((mb.target - origin).dot(aim.perp()) > 1e-4));
    MinibossShape::False { origin, aim, bulbs, selected, jaw_len, jaw_half }
}

/// A distinct signature footprint. All units are world pixels.
/// Wake keeps its fixed history inline so resolving a footprint never allocates.
#[allow(clippy::large_enum_variant)]
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
    /// The closing pincer: two `len × 2·half` claws astride `aim` whose
    /// centre-line separation sweeps `gap_lo..gap_hi`. The footprint is the
    /// whole travel, so the tell marks every place a claw can pass through.
    Pincer {
        origin: Vec2,
        aim: Vec2,
        len: f32,
        half: f32,
        gap_lo: f32,
        gap_hi: f32,
    },
    /// The rollers: two `bar_len`-long bars astride `aim`, each `2·half` wide,
    /// starting `seam_half` either side of the centre line, rolling down a
    /// `len`-long lane. `covered` is the leading edge's distance from the
    /// body. The seam between the bars is never covered.
    Lane {
        origin: Vec2,
        aim: Vec2,
        len: f32,
        seam_half: f32,
        half: f32,
        bar_len: f32,
        covered: f32,
    },
    Annulus { origin: Vec2, inner: f32, outer: f32 },
    /// The warned travel, or just the live crossbar, split by a fixed gap.
    Tide { origin: Vec2, aim: Vec2, len: f32, from: f32, to: f32, gap: (f32, f32) },
    /// A swept or instantaneous cord with an uncovered hub.
    Return { origin: Vec2, aim: Vec2, len: f32, half: f32, sweep: f32 },
    Overlap { centers: [Vec2; 2], radius: f32 },
    Comb { origin: Vec2, aim: Vec2, len: f32, half: f32, gaps: [(f32, f32); 3] },
    /// `blade` is the lateral offset of either live blade. `None` warns the lane.
    Shear { origin: Vec2, aim: Vec2, len: f32, half: f32, channel: f32, blade: Option<f32> },
    Wake { trail: WakeTrail, half: f32 },
    /// All three circles when warning, a single circle when striking.
    Drip { centers: [Vec2; 3], radius: f32, mask: u8 },
    Umbra { origin: Vec2, aim: Vec2, half_angle: f32, len: f32 },
    /// Both proposals while tracking; index zero is the left bulb. The
    /// committed tell and strike carry the same `selected` jaw and decoy.
    False {
        origin: Vec2,
        aim: Vec2,
        bulbs: [Vec2; 2],
        selected: Option<usize>,
        jaw_len: f32,
        jaw_half: f32,
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
            MinibossShape::Pincer {
                origin, aim, len, ..
            } => origin + aim.normalized() * (len * 0.5),
            MinibossShape::Lane {
                origin,
                aim,
                covered,
                bar_len,
                ..
            } => origin + aim.normalized() * ((covered - bar_len * 0.5).max(0.0)),
            MinibossShape::False { aim, bulbs, selected, jaw_len, .. } => selected.map_or(
                (bulbs[0] + bulbs[1]) * 0.5,
                |i| bulbs[i] + aim * (jaw_len * 0.5),
            ),
            shape => shape.signature_bounds().map_or(Vec2::ZERO, |(lo, hi)| (lo + hi) * 0.5),
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
                let ang = v.normalized().dot(aim.normalized()).clamp(-1.0, 1.0).dacos();
                let margin = (radius / d).clamp(0.0, 1.0).dasin();
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
                    // Dead centre has no bearing, so the break cannot shelter
                    // it: a body here that reaches the rim (the band test
                    // above) straddles it on every side. This is the rat that
                    // stands still inside the Late Plate's closing ring.
                    return true;
                }
                let ang = v
                    .normalized()
                    .dot(gap_dir.normalized())
                    .clamp(-1.0, 1.0)
                    .dacos();
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
            MinibossShape::Pincer {
                origin,
                aim,
                len,
                half,
                gap_lo,
                gap_hi,
            } => {
                // Work in the claws' own frame: `along` down the aim, `across`
                // sideways. A claw is a line from the body to its tip at
                // `across = ±separation/2`, so the nearest one to `p` uses the
                // separation that lands closest to `|across|`'s side, clamped
                // to the travel the claw actually makes.
                let dir = aim.normalized();
                let v = p - origin;
                let along = v.dot(dir);
                let across = v.dot(dir.perp()).abs();
                let sep = (across * 2.0).clamp(gap_lo.min(gap_hi), gap_lo.max(gap_hi));
                let along_off = along - along.clamp(0.0, len);
                let across_off = across - sep * 0.5;
                along_off * along_off + across_off * across_off <= (half + radius) * (half + radius)
            }
            MinibossShape::Lane {
                origin,
                aim,
                len,
                seam_half,
                half,
                bar_len,
                covered,
            } => {
                let dir = aim.normalized();
                let v = p - origin;
                let along = v.dot(dir);
                let across = v.dot(dir.perp()).abs();
                // The stretch the pair is pressing right now, clipped to the
                // lane: the bars lead with `covered` and trail by `bar_len`.
                let head = covered.min(len);
                let tail = (covered - bar_len).max(0.0);
                // The two bars: `seam_half .. seam_half + 2·half` either side
                // of the lane's centre line. The seam itself is never covered.
                head - tail > 0.0
                    && along + radius >= tail
                    && along - radius <= head
                    && across + radius >= seam_half
                    && across <= seam_half + half * 2.0 + radius
            }
            _ => shapes::contains(self, p, radius),
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
    /// The rat's position at tell start. The closing ring (Late Plate) is
    /// centred here; every other row ignores it.
    pub target: Vec2,
    /// The three stamp centres, locked at tell start, in landing order.
    pub stamp_centers: [Vec2; 3],
    pub(super) signature_track: SignatureTrack,
    /// Successful hits in the separately damaging passes or circles.
    multi_hit_mask: u8,
    /// Remaining i-frames granted by this strike's most recent hit.
    multi_hit_iframes: f32,
    /// One-shot arrival cue.
    pub announced: bool,
    /// One-shot tell cue for this signature.
    pub tell_cued: bool,
    /// One-shot active cue for this signature.
    pub active_cued: bool,
    /// The signature has already bitten this rat: one hit per strike, however
    /// many ticks the active window lasts and however long the shape lingers
    /// over the body. Cleared when the next strike starts.
    pub hit_this_strike: bool,
    /// Set once the chest has been queued.
    pub chest_dropped: bool,
    /// XP this miniboss leaves (§1 budget).
    pub xp: u32,
    /// Damage of one signature hit.
    pub signature_damage: f32,
    pub(crate) manoeuvre: Manoeuvre,
    pub(crate) strikes: u32,
    pub(crate) preview: bool,
    signature_index: usize,
    second_unlocked: bool,
    next_secondary: bool,
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
            target: Vec2::ZERO,
            stamp_centers: [Vec2::ZERO; 3],
            signature_track: SignatureTrack::default(),
            multi_hit_mask: 0,
            multi_hit_iframes: 0.0,
            announced: false,
            tell_cued: false,
            active_cued: false,
            hit_this_strike: false,
            chest_dropped: false,
            xp: appointment_xp(appointment),
            signature_damage: appointment_signature_damage(appointment),
            manoeuvre: Manoeuvre::default(),
            strikes: 0,
            preview: false,
            signature_index: 0,
            second_unlocked: kind.temperament().second_from_hp == 1.0,
            next_secondary: false,
        }
    }

    /// Where a signature centred on the rat locks to.
    ///
    /// A card with no tell (the Library's animated preview) never sets a
    /// target, so it falls back to the body: a ring centred on nowhere would
    /// draw off the card.
    #[inline]
    pub fn rat_lock(&self) -> Vec2 {
        if self.target == Vec2::ZERO {
            self.origin
        } else {
            self.target
        }
    }

    pub fn signature(&self) -> Signature {
        self.kind.signatures()[self.signature_index]
    }

    pub fn active_duration(&self) -> f32 {
        self.signature().active
    }

    pub(super) fn select_signature(&mut self, hp: f32, max_hp: f32) {
        if !self.second_unlocked && hp < max_hp * self.kind.temperament().second_from_hp {
            self.second_unlocked = true;
            self.next_secondary = true;
        }
        self.signature_index = usize::from(self.next_secondary && !self.preview);
    }

    pub(super) fn finish_signature(&mut self) {
        self.strikes = self.strikes.wrapping_add(1);
        self.next_secondary = self.second_unlocked && self.signature_index == 0;
    }

    /// The body is planted throughout the warning, strike and recovery.
    pub fn holds(&self) -> bool {
        matches!(self.state, MinibossState::Tell | MinibossState::Active | MinibossState::Recover)
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
        if self.state == MinibossState::Tell {
            return shapes::locked_tell_hit(self.signature().shape, self);
        }
        if self.state != MinibossState::Active {
            return None;
        }
        // Clamp to the window: the state machine leaves `Active` on the first
        // tick whose timer reaches the duration, and that tick must still
        // evaluate the strike's *final* pose (a ring fully closed, a trench
        // fully filled, a sweep at its far edge). Unclamped, the timer steps
        // past the window on that tick and the last pose is never tested —
        // at a long frame (dt up to 50 ms) that skipped the whole end of the
        // strike.
        self.kind
            .active_shape(self, self.timer.min(self.active_duration()))
    }
}

/// HP by appointment (3:00 / 6:00 / 9:00 / 12:00) on campaign level one:
/// 270 / 1498 / 2736 / 4163 since batch twelve ([`appointment_hp_for`] has the
/// later levels). The full-build pools they are scaled from are 4500 / 10700
/// / 14400 / 18100.
///
/// The wave-3 ladder (160 / 320 / 540 / 780) died in 2–6 s, and batch six's
/// (800 / 1600 / 2500 / 3400) in 5 / 7 / 10 / 14 s for the Skilled bot: one
/// signature at most. Batch seven sized it for a 20–45 s fight by slot
/// (3800 / 8000 / 10000 / 12000, measured 23 / 30 / 40 / 48 s over 96
/// campaigns). Batch eight gave the rat +1 % weapon damage per character level
/// and multiplied each pool by the bonus a typical rat carries at its
/// appointment (×1.19 / ×1.34 / ×1.44 / ×1.51, `director::level_bonus_hp_at`),
/// so the fights keep their length. Every reader (the spawn, the console
/// and test spawn in `World::spawn_miniboss`, the Library's HIT POINTS fact)
/// goes through this function.
#[inline]
pub fn appointment_hp(appointment: usize) -> f32 {
    appointment_hp_for(0, appointment)
}

/// [`appointment_hp`] on a campaign level (the difficulty level: a practice
/// run plays level one's row).
#[inline]
pub fn appointment_hp_for(level: usize, appointment: usize) -> f32 {
    let i = appointment.min(3);
    APPOINTMENT_HP_FULL[i] * APPOINTMENT_BUILD_SCALE[level.min(3)][i]
}

/// The pools sized against a full batch-eight build (see [`appointment_hp`]).
const APPOINTMENT_HP_FULL: [f32; 4] = [4500.0, 10700.0, 14400.0, 18100.0];
/// Batch twelve: what a quarter-as-many-cards build is worth against that
/// one, by campaign level (rows) and appointment (columns).
/// `director::BUILD_HP` is the horde's and the boss's version of the same
/// correction. Level one starts from the bare Gnaw, so its 3:00 pool is 270;
/// levels two to four arrive with a carried build. Measured medians over 48
/// mixed-skill campaigns: 24 / 32 / 40 / 43 s by slot (the batch-seven targets
/// were 23 / 32 / 42 / 44).
const APPOINTMENT_BUILD_SCALE: [[f32; 4]; 4] = [
    [0.06, 0.14, 0.19, 0.23],
    [0.58, 0.48, 0.64, 0.49],
    [0.52, 0.45, 0.47, 0.39],
    [0.48, 0.36, 0.46, 0.45],
];

/// Signature damage by appointment: 18 / 22 / 26 / 30 (the design's §8 row
/// was 14 / 16 / 18 / 20; batch seven raised it so the signature, not the
/// body, is what a named fight punishes).
#[inline]
pub fn appointment_signature_damage(appointment: usize) -> f32 {
    match appointment {
        0 => 18.0,
        1 => 22.0,
        2 => 26.0,
        _ => 30.0,
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

/// The per-theme schedule: four fixed appointments per floor, in order
/// (3:00, 6:00, 9:00, 12:00).
///
/// The table itself lives in the roster ([`crate::game::roster`]), beside the
/// floor's eight ordinary slots, so a per-floor content pass edits one file.
/// `WAVE4_ROSTERS.md` gives every floor four fixed appointments in increasing
/// complexity — the last one a rehearsal for the floor's boss — and each row
/// is now the real body, on its own floor: the six wave-3 names keep their
/// homes (Sewer Corkscrew/Bell Diver, Kitchen Milk Carton, Laundry Sister
/// Snip/Bobbin, Printworks The Foreman) and the eighteen wave-4 names fill the
/// other slots. See `roster::floor(theme).appointments`.
pub fn schedule_for(theme: Theme, appointment: usize) -> MinibossKind {
    let row = crate::game::roster::appointments(theme);
    row[appointment.min(row.len() - 1)]
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
    level: usize,
    pos: Vec2,
    next_id: &mut NextSpawnId,
    next_enemy_id: &mut NextEnemyId,
    rng: &mut StdRng,
) {
    // A Brute body carries the miniboss so every existing weapon, trap and
    // bullet path already targets it. The stats are overwritten below.
    let mut e = Enemy::spawn(EnemyKind::Brute, pos, 1.0, rng);
    e.hp = appointment_hp_for(level, appointment);
    e.max_hp = e.hp;
    e.radius = kind.body_radius();
    e.damage = MINIBOSS_CONTACT_DAMAGE;
    // Faster than an ordinary brute so the approach reads as a named arrival.
    e.speed_scale = kind.speed_scale();
    e.elite = false;
    e.named = true;
    e.id = next_enemy_id.take();
    commands.spawn((e, Miniboss::new(kind, appointment), next_id.take()));
}

/// The ordinary spawn ring needs only 4 px of clearance. Fit the larger
/// named body near that result, in the same reachable region and out of sight
/// if the ring was out of sight, without rolling the ring again.
fn fit_outfall_arrival(
    level: &Level, scratch: &SpawnScratch, pos: Vec2, rat: Vec2, viewport: Vec2, radius: f32,
) -> Option<Vec2> {
    use crate::game::collision::{cell_of, TileMap, TILE_F};

    if behaviour::clear(level, pos, radius, SizeClass::Large) {
        return Some(pos);
    }
    let half_view = widest_view(viewport) * 0.5 + Vec2::splat(radius);
    let off_screen = |p: Vec2| (p.x - rat.x).abs() > half_view.x || (p.y - rat.y).abs() > half_view.y;
    let keep_hidden = off_screen(pos);
    let min_dist = pos.dist(rat) - TILE_F;
    let (x, y) = cell_of(pos);
    for ring in 0..=3i32 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) != ring {
                    continue;
                }
                let at = TileMap::tile_rect(x + dx, y + dy).center();
                if (!keep_hidden || off_screen(at)) && at.dist(rat) >= min_dist
                    && scratch.is_reachable(level, at) && !in_maze(level, at)
                    && behaviour::clear(level, at, radius, SizeClass::Large)
                {
                    return Some(at);
                }
            }
        }
    }
    None
}

#[derive(Default)]
pub(crate) struct MinibossScratch {
    order: Vec<(u64, Entity)>,
    owners: Vec<u64>,
    summon_order: Vec<(u64, Entity)>,
}

/// Appointments, movement and strike states run before the grid build, so the
/// grid sees the named body and any fodder it called on their new cells.
pub fn miniboss_system(
    mut commands: Commands,
    (input, rat_track): (Res<TickInput>, Res<RatTrack>),
    counters: ResMut<RunCounters>,
    mut schedule: ResMut<MinibossSchedule>,
    level: Res<Level>,
    viewport: Res<Viewport>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut movement: MovementParams,
    mut sfx: ResMut<SfxQueue>,
    mut particles: ResMut<Particles>,
    difficulty: Res<crate::game::ecs::resources::DifficultyLevel>,
    mut scratch: Local<SpawnScratch>,
    mut behaviour_scratch: Local<MinibossScratch>,
    mut bosses: Query<(Entity, &mut Enemy, &mut Miniboss, &SpawnId), With<Miniboss>>,
) {
    let MinibossScratch { order, owners, summon_order } = &mut *behaviour_scratch;
    let dt = input.dt;
    let elapsed = counters.elapsed;
    let ppos = player.pos;
    let pvel = player.vel;
    let retiring = !counters.testbed
        && schedule.next_appointment().map_or(elapsed >= BOSS_TIME, |at| elapsed >= at);
    owners.clear();

    // 1. Tick the living minibosses.
    let mut alive = 0u32;
    // In spawn order: the loop draws rng (arrival and hurt particles), and the
    // console can put two named bodies on the floor at once.
    // (The same collect-sort-`get_mut` shape `enemy_move` and `combat` use;
    // the buffer is a `Local`, so a tick allocates nothing once it has grown.)
    order.clear();
    order.extend(bosses.iter().map(|(entity, _, _, id)| (id.0, entity)));
    order.sort_unstable_by_key(|&(id, _)| id);
    for &(id, entity) in order.iter() {
        let Ok((_, mut e, mut mb, _)) = bosses.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        alive += 1;
        if !retiring {
            owners.push(id);
        }
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
        let contact_cd = e.contact_cd;
        let flash = e.flash;
        let previous_timer = mb.timer;
        mb.timer += dt;
        mb.multi_hit_iframes = (mb.multi_hit_iframes - dt).max(0.0);
        match mb.state {
            MinibossState::Approach | MinibossState::Cooldown => {
                mb.select_signature(e.hp, e.max_hp);
                let until_ready = if mb.state == MinibossState::Cooldown {
                    mb.kind.temperament().gap - RECOVER_TIME - mb.timer
                } else {
                    0.0
                };
                let range = mb.kind.engage_range_for(mb.signature());
                if movement.advance(entity, id, &mut mb, &mut e, &level, ppos, range,
                    until_ready, dt, &mut particles)
                {
                    mb.manoeuvre.motion = Motion::Roam;
                    e.vel = Vec2::ZERO;
                    begin_tell(
                        &mut mb,
                        &mut e,
                        ppos,
                        pvel,
                        &level,
                        &mut sfx,
                        &mut particles,
                        &mut rng.0,
                        &rat_track,
                    );
                }
            }
            MinibossState::Tell => {
                let signature = mb.signature();
                let timer = mb.timer;
                mb.signature_track.update(signature.shape, timer, &rat_track);
                if !track_tell(&mut mb, previous_timer, e.pos, ppos, pvel, &level, &rat_track) {
                    skip_tell(&mut mb);
                } else {
                    if timer >= TELL_LOCK {
                        e.vel = Vec2::ZERO;
                        strike_rat(&mut mb, &mut player, &mut sfx, &mut particles, &mut rng.0);
                    }
                    if timer >= TELL_TIME {
                        mb.state = MinibossState::Active;
                        mb.timer = 0.0;
                        mb.active_cued = false;
                        if mb.kind.is_outfall() {
                            strike_rat(&mut mb, &mut player, &mut sfx, &mut particles, &mut rng.0);
                        }
                    }
                }
            }
            MinibossState::Active => {
                e.vel = Vec2::ZERO;
                if !mb.active_cued {
                    mb.active_cued = true;
                    sfx.0.push(Sfx::MinibossTell);
                }
                strike_rat(&mut mb, &mut player, &mut sfx, &mut particles, &mut rng.0);
                if mb.timer >= mb.active_duration() {
                    mb.state = MinibossState::Recover;
                    mb.timer = 0.0;
                    mb.finish_signature();
                    if !retiring && !mb.preview && mb.kind.temperament().style == MoveStyle::Summon {
                        movement.summon(
                            &mut commands, id, &mb, e.pos, ppos, &level, difficulty.0, elapsed,
                        );
                    }
                }
            }
            MinibossState::Recover => {
                // Exposed: the rat can hit it freely.
                if mb.timer >= RECOVER_TIME {
                    mb.state = MinibossState::Cooldown;
                    mb.timer = 0.0;
                }
            }
        }
        e.position_locked = mb.holds() || mb.manoeuvre.locks_position();
        e.contact_cd = (contact_cd - dt).max(0.0);
        e.flash = (flash - dt).max(0.0);
    }
    schedule.active = alive;
    movement.clean_summons(&mut commands, owners, summon_order);
    if counters.testbed {
        return;
    }

    // 2. Appointments. A surviving miniboss retreats without loot when the
    //    next one arrives, so only one is ever on the floor.
    if let Some(at) = schedule.next_appointment() {
        if elapsed >= at {
            // Retreat every living miniboss (no loot, no chest).
            for &(_, entity) in order.iter() {
                if bosses.get(entity).is_ok_and(|(_, e, _, _)| e.alive()) {
                    commands.entity(entity).despawn();
                    sfx.0.push(Sfx::MinibossRetreat);
                }
            }
            // Spawn the new one just outside the view.
            scratch.sync(&level);
            let kind = schedule_for(level.theme, schedule.next);
            let pos = spawn_point_reachable(&mut rng.0, &level, &player, viewport.0, &mut scratch);
            let pos = if level.theme == Theme::Outfall {
                pos.and_then(|at| fit_outfall_arrival(
                    &level, &scratch, at, player.pos, viewport.0, kind.body_radius(),
                ))
            } else {
                pos
            };
            if let Some(pos) = pos {
                spawn_miniboss(
                    &mut commands,
                    kind,
                    schedule.next,
                    difficulty.0,
                    pos,
                    &mut movement.next_id,
                    &mut movement.next_enemy_id,
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
        for &(_, entity) in order.iter() {
            if bosses.get(entity).is_ok_and(|(_, e, _, _)| e.alive()) {
                commands.entity(entity).despawn();
                sfx.0.push(Sfx::MinibossRetreat);
            }
        }
    }
}

fn strike_rat(
    mb: &mut Miniboss,
    player: &mut Player,
    sfx: &mut SfxQueue,
    particles: &mut Particles,
    rng: &mut StdRng,
) {
    let Some(shape) = mb.active_shape() else {
        return;
    };
    let bit = multi_hit_bit(mb.signature(), mb.timer);
    if apply_strike(mb, player, &shape, bit) {
        sfx.0.push(Sfx::Hurt);
        crate::game::particle::vfx::emit_player_hurt(particles, rng, player.pos);
    }
}

pub(super) fn apply_strike(
    mb: &mut Miniboss,
    player: &mut Player,
    shape: &MinibossShape,
    bit: Option<u8>,
) -> bool {
    let already_hit = if let Some(bit) = bit {
        mb.multi_hit_mask & bit != 0
    } else {
        mb.hit_this_strike
    };
    if already_hit || !shape.contains(player.pos, crate::game::ecs::constants::PLAYER_RADIUS) {
        return false;
    }
    // Consecutive passes and drops are separate hits. Ignore only the
    // immunity granted by this same strike, and honor an active dash.
    let immunity = player.iframes;
    if bit.is_some() && immunity == mb.multi_hit_iframes && player.dash_active <= 0.0 {
        player.iframes = 0.0;
    }
    if !player.take_damage(mb.signature_damage) {
        player.iframes = immunity;
        return false;
    }
    if let Some(bit) = bit {
        mb.multi_hit_mask |= bit;
        mb.multi_hit_iframes = player.iframes;
    }
    mb.hit_this_strike = true;
    true
}

pub(super) fn multi_hit_bit(signature: Signature, timer: f32) -> Option<u8> {
    match signature.shape {
        SignatureShape::Return { .. } => Some(1 << u8::from(timer > signature.active * 0.5)),
        SignatureShape::Drip { .. } => Some(1 << shapes::drip_index(timer)),
        _ => None,
    }
}

/// Keep tracking through the first half of the warning. Outfall signatures
/// evaluate the actual lock instant and can decline an unsafe commitment.
fn track_tell(
    mb: &mut Miniboss, previous_timer: f32, body: Vec2, rat: Vec2, rat_vel: Vec2,
    level: &Level, rat_track: &RatTrack,
) -> bool {
    if mb.timer < TELL_LOCK {
        // A bad tracking pose keeps the last legal warning until lock.
        let _ = aim_signature(mb, body, rat, rat_vel, level);
    } else if mb.kind.is_outfall() && previous_timer < TELL_LOCK {
        let at_lock = if rat_track.is_empty() {
            rat
        } else {
            rat_track.at_age(f64::from(mb.timer - TELL_LOCK))
        };
        return aim_signature(mb, body, at_lock, rat_vel, level);
    }
    true
}

/// Aim the signature from a body at `body` at a rat at `ppos` moving at
/// `pvel`: the origin, the aim, the rat lock and the stamp centres.
///
/// The aim **leads** the rat by the frozen part of the tell
/// ([`TELL_TIME`] − [`TELL_LOCK`]), so a rat that keeps its heading walks into
/// the strike and a rat that turns at the lock walks out of it. Returns
/// `false`, leaving `mb` untouched, when no turn of the aim leaves the rat a
/// walkable way out.
fn aim_signature(mb: &mut Miniboss, body: Vec2, ppos: Vec2, pvel: Vec2, level: &Level) -> bool {
    let speed = pvel.len();
    let led = if speed > TELL_LEAD_SPEED_MAX {
        // A dash (or a shove): not a heading the rat will keep.
        Vec2::ZERO
    } else {
        pvel * (TELL_TIME - TELL_LOCK)
    };
    let at = ppos + led;
    let aim = (at - body).normalized();
    let aim = if aim.len_sq() < 0.5 {
        Vec2::new(1.0, 0.0)
    } else {
        aim
    };
    // The rollers leave a safe seam down the middle of their lane. Aimed
    // straight at the rat, that seam is where the rat already stands and the
    // strike never has to be answered; turn the lane just far enough that the
    // rat is under one bar's centre line and has to step to the seam.
    let shape = mb.signature().shape;
    let aim = match shape {
        SignatureShape::Lane {
            seam_half, half, ..
        } => {
            let d = at.dist(body).max(1.0);
            aim.rotate(((seam_half + half) / d).clamp(0.0, 0.7).dasin())
        }
        SignatureShape::Umbra { .. } if mb.kind == MinibossKind::Shade => {
            let away_from_rat = (body - ppos).normalized();
            let fallback = if away_from_rat == Vec2::ZERO { -aim } else { away_from_rat };
            level.nearest_live_sconce(body, SHADE_SCONCE_REACH)
                .map(|(_, lamp)| (body - lamp).normalized())
                .filter(|&away| away != Vec2::ZERO)
                .unwrap_or(fallback)
        }
        SignatureShape::Umbra { .. } => shapes::umbra_aim(level, body, aim),
        _ => aim,
    };
    let before = (mb.origin, mb.aim, mb.target, mb.stamp_centers);
    mb.origin = body;
    mb.aim = aim;
    mb.target = if matches!(shape, SignatureShape::False { .. }) { ppos } else { at };
    lock_stamps(mb);
    // Reposition when the signature leaves no walkable escape: try rotating
    // the aim until a sample direction is clear.
    let attempts = if matches!(shape, SignatureShape::Umbra { .. } | SignatureShape::False { .. }) {
        1
    } else {
        8
    };
    for _ in 0..attempts {
        if escape_exists(mb, ppos, level) {
            return true;
        }
        mb.aim = mb.aim.rotate(std::f32::consts::FRAC_PI_4);
        lock_stamps(mb);
    }
    (mb.origin, mb.aim, mb.target, mb.stamp_centers) = before;
    false
}

/// Enter the tell: aim the signature and cue it. The aim keeps tracking until
/// [`TELL_LOCK`] (see `miniboss_system`).
fn begin_tell(
    mb: &mut Miniboss,
    e: &mut Enemy,
    ppos: Vec2,
    pvel: Vec2,
    level: &Level,
    sfx: &mut SfxQueue,
    particles: &mut Particles,
    rng: &mut StdRng,
    rat_track: &RatTrack,
) {
    mb.timer = 0.0;
    if mb.kind.is_outfall() {
        // Swell's gap begins at the tracking edge, including when the
        // previous state was cooldown.
        mb.state = MinibossState::Tell;
    }
    let shape = mb.signature().shape;
    if matches!(shape, SignatureShape::Wake { .. } | SignatureShape::Drip { .. }) {
        mb.signature_track = SignatureTrack::start(shape, ppos, rat_track);
    }
    if !aim_signature(mb, e.pos, ppos, pvel, level) {
        // Still trapped: skip this signature and go straight to cooldown.
        skip_tell(mb);
        return;
    }
    mb.state = MinibossState::Tell;
    mb.hit_this_strike = false;
    mb.multi_hit_mask = 0;
    mb.multi_hit_iframes = 0.0;
    // `tell_cued` stays set from the first tell on: the art reads it to know
    // the arrival pose is over.
    mb.tell_cued = true;
    sfx.0.push(Sfx::MinibossTell);
    crate::game::particle::vfx::emit_miniboss_tell(particles, rng, e.pos, (255, 226, 140));
}

fn skip_tell(mb: &mut Miniboss) {
    mb.state = MinibossState::Cooldown;
    mb.timer = 0.0;
}

/// Is there at least one walkable direction out of the rat's position that the
/// telegraph does not cover? Sampled at 8 compass points.
fn escape_exists(mb: &Miniboss, ppos: Vec2, level: &Level) -> bool {
    let shape = mb.kind.telegraph(mb);
    if mb.kind.is_outfall() {
        return outfall_escape_exists(&shape, mb.origin, mb.kind.body_radius(), ppos, level);
    }
    // A ring has exactly one answer — its break — and the 12 px samples below
    // sit inside its rim, so they cannot judge it. Require the break itself
    // to be floor the rat can walk; `begin_tell` turns the aim, and the break
    // with it, an eighth at a time until one is.
    //
    // * Closing on the rat (the Late Plate): the whole way out, from the lock
    //   past the rim ("a 65° break faces the nearest open aisle").
    // * Growing from the body (Bell Diver, Petri Mother, Suds Bailiff): the
    //   rat meets the ring where it stands, so the break must be open from
    //   the rat's own distance outward — somewhere to wait for the rim to
    //   pass — rather than pointing into a wall.
    if let (
        SignatureShape::Ring { on_rat, .. },
        MinibossShape::RingGap {
            origin,
            radius,
            half,
            gap_dir,
            ..
        },
    ) = (mb.signature().shape, shape)
    {
        let dir = gap_dir.normalized();
        let reach = radius + half + crate::game::ecs::constants::PLAYER_RADIUS * 2.0;
        let mut d = if on_rat {
            6.0
        } else {
            origin.dist(ppos).clamp(12.0, radius)
        };
        while d <= reach {
            let cell = crate::game::collision::cell_of(origin + dir * d);
            if !level.tiles.get(cell.0, cell.1).passable(SizeClass::Small) {
                return false;
            }
            d += 6.0;
        }
        return true;
    }
    if shape.signature_bounds().is_some()
        && !shape.contains(ppos, crate::game::ecs::constants::PLAYER_RADIUS)
    {
        return true;
    }
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::TAU / 8.0;
        let dir = Vec2::from_angle(ang);
        // Walk outward a step at a time, as far as the rat can get inside the
        // 1 s tell. A row that engages from inside its own reach
        // (`engage_range`) covers every 12 px sample around the rat, so one
        // ring of samples would call every such tell inescapable and turn the
        // aim off the rat; the way out only has to be open floor all the way
        // to the first sample the shape does not cover.
        for step in 1..=3 {
            let sample = ppos + dir * (12.0 * step as f32);
            let cell = crate::game::collision::cell_of(sample);
            if !level.tiles.get(cell.0, cell.1).passable(SizeClass::Small) {
                break;
            }
            if !shape.contains(sample, crate::game::ecs::constants::PLAYER_RADIUS) {
                return true;
            }
        }
    }
    false
}

fn outfall_escape_exists(
    shape: &MinibossShape, body: Vec2, body_radius: f32, rat: Vec2, level: &Level,
) -> bool {
    const MAX_WALK: f32 = 36.0;
    let contact = body_radius + PLAYER_RADIUS;
    if !behaviour::clear(level, rat, PLAYER_RADIUS, SizeClass::Small) {
        return false;
    }
    let safe = |at: Vec2| {
        if at.dist_sq(rat) > MAX_WALK * MAX_WALK
            || at.dist_sq(body) <= contact * contact
            || shape.contains(at, PLAYER_RADIUS)
            || !behaviour::clear(level, at, PLAYER_RADIUS, SizeClass::Small)
        {
            return false;
        }
        // If already inside contact, the rat can step out. A route starting
        // clear must never require walking through the planted body.
        if rat.dist_sq(body) > contact * contact && segment_dist(body, rat, at) <= contact {
            return false;
        }
        let end = clamp_outfall_displacement(level, rat, at - rat, PLAYER_RADIUS, SizeClass::Small);
        end.dist_sq(at) < 1e-4
    };
    if safe(rat) {
        return true;
    }
    let clear_margin = PLAYER_RADIUS + 1.0;
    match *shape {
        MinibossShape::Annulus { origin, outer, .. } => {
            let away = (rat - origin).normalized();
            if safe(origin) || safe(origin + away * (outer + clear_margin)) {
                return true;
            }
        }
        MinibossShape::Tide { origin, aim, len, from, to, gap } => {
            let cross = aim.perp();
            let along = (rat - origin).dot(aim).clamp(from, to);
            let edge = len * 0.5 + clear_margin;
            let gap_center = (gap.0 + gap.1) * 0.5;
            let gap_route = safe(origin + aim * along + cross * gap_center)
                && [from - clear_margin, to + clear_margin].into_iter().any(|end| {
                    let through = origin + cross * gap_center;
                    let delta = aim * (end - along);
                    let start = through + aim * along;
                    let reached = clamp_outfall_displacement(
                        level, start, delta, PLAYER_RADIUS, SizeClass::Small,
                    );
                    reached.dist_sq(start + delta) < 1e-4
                        && segment_dist(body, start, start + delta) > contact
                });
            if gap_route || [-edge, edge].into_iter().any(|side| {
                safe(origin + aim * along + cross * side)
            }) {
                return true;
            }
        }
        MinibossShape::Umbra { origin, aim, .. } => {
            let bank = origin - aim * (contact + 1.0);
            if safe(bank) {
                return true;
            }
        }
        _ => {}
    }
    (0..8).any(|i| {
        let dir = Vec2::from_angle(i as f32 * std::f32::consts::TAU / 8.0);
        (1..=3).any(|step| safe(rat + dir * (12.0 * step as f32)))
    })
}

/// Lock the three stamp centres for the aim the tell just fixed, in the row's
/// own landing order.
///
/// The layout is part of the row data ([`StampLayout`]); a row that does not
/// use stamps still gets three centres, so the field is never left stale.
pub(crate) fn lock_stamps(mb: &mut Miniboss) {
    let origin = mb.origin;
    let aim = mb.aim;
    let left = aim.perp();
    let layout = match mb.signature().shape {
        SignatureShape::Stamps { layout, .. } => layout,
        _ => StampLayout::Along {
            spacing: STAMP_SPACING,
        },
    };
    let centers = match layout {
        StampLayout::Along { spacing } => [
            origin + aim * spacing,
            origin + aim * (spacing * 2.0),
            origin + aim * (spacing * 3.0),
        ],
        StampLayout::AroundRat { across } => {
            let at = mb.rat_lock();
            [at - left * across, at + left * across, at]
        }
        StampLayout::V { forward, across } => {
            let at = mb.rat_lock();
            let ahead = at + aim * forward;
            [ahead - left * across, at, ahead + left * across]
        }
    };
    mb.stamp_centers = centers;
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
        // The tell locks all three of these from the row's own stamp layout.
        m.target = origin + aim * 24.0;
        lock_stamps(&mut m);
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
        // Tracking changes the aim and leaves the body planted.
        m.state = MinibossState::Tell;
        m.timer = 0.0;
        assert!(m.holds());
        m.timer = TELL_LOCK;
        assert!(m.holds());
        m.state = MinibossState::Active;
        assert!(m.holds());
        // The exposed second after the strike is a planted body too.
        m.state = MinibossState::Recover;
        assert!(m.holds());
        m.state = MinibossState::Cooldown;
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

    /// The Late Plate's ring closes on a rat that never left the lock: the
    /// last pose of the strike reaches it, and no earlier one does.
    #[test]
    fn the_closing_ring_bites_a_rat_that_stands_still_at_its_centre() {
        const R: f32 = crate::game::ecs::constants::PLAYER_RADIUS;
        let kind = MinibossKind::LatePlate;
        let mut m = mb(kind, Vec2::new(200.0, 160.0), Vec2::new(1.0, 0.0));
        let rat = m.rat_lock();
        let d = kind.active_duration();
        // Open: the rat in the middle is untouched for most of the strike.
        let open = kind.active_shape(&m, d * 0.5).expect("mid strike");
        assert!(!open.contains(rat, R), "the ring bit its centre while still wide");
        // Closed: the final pose bites dead centre, whatever the break faces.
        let closed = kind.active_shape(&m, d).expect("final pose");
        assert!(closed.contains(rat, R), "a rat standing still was never bitten");
        // The state machine's last active tick (timer just past the window)
        // still evaluates that final pose.
        m.state = MinibossState::Active;
        m.timer = d + 1.0 / 60.0;
        let last = m.active_shape().expect("the last tick still has a shape");
        assert_eq!(last, closed);
        // Out through the break is still the answer.
        let out = rat + m.aim.rotate(RING_GAP_TURN) * 20.0;
        for i in 0..=24 {
            let t = (d * i as f32 / 24.0).min(d);
            assert!(!kind.active_shape(&m, t).expect("active").contains(out, R));
        }
    }

    /// A rat that stops dead when the tell sounds is still inside every
    /// body-anchored footprint: the body closes to within its own reach first.
    #[test]
    fn every_body_anchored_row_engages_inside_its_own_reach() {
        for kind in legacy_primaries() {
            let range = kind.engage_range();
            assert!(range <= ENGAGE_RANGE, "{kind:?} engages from {range}");
            assert!(range > kind.body_radius() + 6.0, "{kind:?} engages inside contact");
            // The rat on the aim line at the engage range, as `begin_tell`
            // would lock it.
            let mut m = Miniboss::new(kind, 0);
            m.origin = Vec2::new(200.0, 160.0);
            m.aim = Vec2::new(1.0, 0.0);
            m.target = m.origin + m.aim * range;
            lock_stamps(&mut m);
            let rat = m.target;
            // The rollers aim their seam at the rat by design ("align with the
            // seam"): the row's reach is still checked, a pad's width aside.
            let probe = match kind.signatures()[0].shape {
                SignatureShape::Lane { seam_half, half, .. } => {
                    rat + m.aim.perp() * (seam_half + half)
                }
                _ => rat,
            };
            let tell = kind.telegraph(&m);
            // A travelling ring's tell is its outermost rim, which encloses
            // the strike rather than painting it: the rat must be inside it.
            if let MinibossShape::RingGap { origin, radius, .. } = tell {
                assert!(
                    rat.dist(origin) < radius,
                    "{kind:?}: a rat at the engage range ({range}) is outside the ring"
                );
                continue;
            }
            assert!(
                tell.contains(probe, crate::game::ecs::constants::PLAYER_RADIUS),
                "{kind:?}: a rat at the engage range ({range}) stands outside the tell"
            );
        }
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
        // WAVE4_ROSTERS.md "Named appointments and elites", row for row: four
        // fixed appointments per floor, in order (3:00, 6:00, 9:00, 12:00).
        // The six wave-3 names keep their homes (Sewer 3:00 Corkscrew / 6:00
        // Bell Diver, Kitchen 3:00 Milk Carton, Laundry 3:00 Sister Snip /
        // 6:00 Bobbin, Printworks 3:00 The Foreman) and every other slot is now
        // the design's own body.
        let rows = [
            (Theme::Sewer, [Corkscrew, BellDiver, ValveWidow, Mudlark]),
            (Theme::Kitchen, [MilkCarton, PastryWheel, SaltHen, PotBrush]),
            (Theme::Lab, [DosePump, PetriMother, CageCounter, RotorTechnician]),
            (Theme::Laundry, [SisterSnip, Bobbin, SudsBailiff, WringerBoy]),
            (Theme::Printworks, [Foreman, QuoinKey, CarbonTwin, LatePlate]),
            (Theme::Trailer, [
                TarpaulinPup,
                AxleNut,
                ChainDog,
                RoadAtlas,
            ]),
            (Theme::Outfall, [Gasket, Swell, Shade, FalseBulb]),
        ];
        for (theme, row) in rows {
            for (i, kind) in row.into_iter().enumerate() {
                assert_eq!(schedule_for(theme, i), kind, "{theme:?} appointment {i}");
            }
        }
        // Every named miniboss has exactly one home, so no
        // floor borrows another floor's body and none of the six's work is
        // lost. This is the "tie them to specific floors" rule.
        assert_eq!(MinibossKind::all().len(), 28);
        for (i, kind) in MinibossKind::all().into_iter().enumerate() {
            let homes: Vec<(Theme, usize)> = Theme::playable()
                .iter()
                .flat_map(|&t| (0..4).map(move |at| (t, at)))
                .filter(|&(t, at)| schedule_for(t, at) == kind)
                .collect();
            assert_eq!(homes.len(), 1, "{kind:?} does not have exactly one home: {homes:?}");
            // Every variant is in `all()`, in enum order.
            assert_eq!(kind as usize, i, "{kind:?} is out of order in all()");
        }
        // Every playable floor (the Flatbed included) has four distinct names,
        // and the four are unique across the whole campaign.
        let mut seen: Vec<MinibossKind> = Vec::new();
        for theme in Theme::playable() {
            let row: Vec<MinibossKind> = (0..4).map(|i| schedule_for(theme, i)).collect();
            for i in 0..4 {
                for j in 0..i {
                    assert_ne!(row[i], row[j], "{theme:?} repeats a miniboss");
                }
                assert!(
                    !seen.contains(&row[i]),
                    "{:?} appears on two floors",
                    row[i]
                );
                seen.push(row[i]);
            }
            // Out-of-range appointments clamp to the last row.
            assert_eq!(schedule_for(theme, 9), row[3]);
        }
        assert_eq!(seen.len(), 28, "the seven floors must field all 28 names");
    }

    #[test]
    fn appointment_stats_match_the_design() {
        // Batch seven's ladder: sized for 20-45 s fights by slot (measured
        // with the Skilled bot: 23 / 30 / 40 / 48 s).
        // Batch eight: the same fights against a rat whose level adds damage
        // (batch seven's 3800 / 8000 / 10000 / 12000 times the level bonus at
        // each appointment).
        // Batch twelve: each is scaled for the smaller builds, by level and
        // slot (`APPOINTMENT_BUILD_SCALE`); level one's row is the default.
        let base = [3800.0f32, 8000.0, 10000.0, 12000.0];
        let pools = APPOINTMENT_HP_FULL;
        assert_eq!(pools, [4500.0f32, 10700.0, 14400.0, 18100.0]);
        let level_one = [270.0f32, 1498.0, 2736.0, 4163.0];
        for (slot, time) in MINIBOSS_TIMES.iter().enumerate() {
            assert!((appointment_hp(slot) - level_one[slot]).abs() < 0.5, "slot {slot}");
            assert_eq!(appointment_hp(slot), appointment_hp_for(0, slot));
            for level in 1..4 {
                assert!(
                    appointment_hp_for(level, slot) > appointment_hp(slot),
                    "a carried build meets a larger pool (level {level}, slot {slot})"
                );
            }
            // The full-build pool is batch seven's times the level bonus
            // (without batch twelve's build correction).
            let want = base[slot] * crate::game::director::level_bonus_hp_at(*time);
            assert!(
                (pools[slot] - want).abs() < want * 0.02,
                "slot {slot}: {} is not the level bonus on {}",
                pools[slot],
                base[slot]
            );
        }
        assert_eq!(appointment_signature_damage(0), 18.0);
        assert_eq!(appointment_signature_damage(1), 22.0);
        assert_eq!(appointment_signature_damage(2), 26.0);
        assert_eq!(appointment_signature_damage(3), 30.0);
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
        // Use a surface body to exercise the normal Nuke path. Burrow's
        // independent immunity would absorb the Nuke on this tick.
        let e = w.spawn_miniboss(K::Corkscrew, 1, p + Vec2::new(60.0, 0.0));
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
        let unclaimed = chests(&mut w);
        assert_eq!(unclaimed + w.pending_chests as usize, 1, "a DOT kill yields one chest");
        assert_eq!(gold_values(&mut w).into_iter().sum::<u32>() + w.gold, MINIBOSS_GOLD);
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

    /// The first six bodies retain their radii and primary active windows.
    #[test]
    fn the_existing_six_keep_their_numbers() {
        let want = [
            (MinibossKind::Corkscrew, 0.5, 7.0),
            (MinibossKind::SisterSnip, 0.8, 7.0),
            (MinibossKind::Bobbin, 0.7, 6.5),
            (MinibossKind::BellDiver, 1.2, 7.0),
            (MinibossKind::Foreman, 0.9, 7.5),
            (MinibossKind::MilkCarton, 1.2, 7.0),
        ];
        for (kind, active, radius) in want {
            assert_eq!(kind.active_duration(), active, "{kind:?} active window");
            assert_eq!(kind.body_radius(), radius, "{kind:?} body radius");
        }
    }

    fn close(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() < 1e-3, "{what}: {a} != {b}");
    }

    /// Legacy primary attacks keep their dimensions except where longer reach
    /// is part of the new movement style.
    #[test]
    fn legacy_primaries_keep_their_assigned_dimensions() {
        use SignatureShape as S;
        // "a 46×8 cut fills down the counter over 0.6" / "injection trench
        // 40×8 fills in 0.65" / "rolls a 50×10 trench out … over 0.65".
        for (kind, len, width, active) in [
            (MinibossKind::PastryWheel, 46.0, 8.0, 0.6),
            (MinibossKind::DosePump, 40.0, 8.0, 0.65),
            (MinibossKind::AxleNut, 50.0, 10.0, 0.65),
        ] {
            match kind.signatures()[0] {
                Signature {
                    active: a,
                    shape: S::Trench { len: l, half },
                } => {
                    close(a, active, "active");
                    close(l, len, "length");
                    close(half * 2.0, width, "width");
                }
                other => panic!("{kind:?} is not a trench: {other:?}"),
            }
        }
        // "a 38-long, 55° wedge in 0.75".
        match MinibossKind::QuoinKey.signatures()[0] {
            Signature {
                active,
                shape: S::Cone { half_angle, len },
            } => {
                close(active, 0.75, "active");
                close(len, 38.0, "length");
                close(half_angle.to_degrees() * 2.0, 55.0, "wedge");
            }
            other => panic!("Quoin Key is not a cone: {other:?}"),
        }
        // The brush arm and the chain retain their original sweeps.
        for (kind, len, width, sweep, active) in [
            (MinibossKind::PotBrush, 42.0, 6.0, 110.0, 0.85),
            (MinibossKind::ChainDog, 36.0, 6.0, 115.0, 0.9),
        ] {
            match kind.signatures()[0] {
                Signature {
                    active: a,
                    shape: S::Cord {
                        len: l,
                        half,
                        sweep: sw,
                    },
                } => {
                    close(a, active, "active");
                    close(l, len, "length");
                    close(half * 2.0, width, "width");
                    close(sw.to_degrees(), sweep, "sweep");
                }
                other => panic!("{kind:?} is not a cord: {other:?}"),
            }
        }
        // The culture ring and the closing ring (32 → 5, 5 wide, 65°, on the rat:
        // see `LATE_PLATE_CLOSED` for why it is not the row's 6).
        for (kind, from, to, width, gap, on_rat, active) in [
            (MinibossKind::PetriMother, 0.0, 36.0, 6.0, 75.0, false, 1.2),
            (MinibossKind::LatePlate, 32.0, LATE_PLATE_CLOSED, 5.0, 65.0, true, 1.2),
        ] {
            match kind.signatures()[0] {
                Signature {
                    active: a,
                    shape:
                        S::Ring {
                            from: f0,
                            to: f1,
                            half,
                            gap_half,
                            on_rat: r,
                        },
                } => {
                    close(a, active, "active");
                    close(f0, from, "from radius");
                    close(f1, to, "to radius");
                    close(half * 2.0, width, "width");
                    close((gap_half * 2.0).to_degrees(), gap, "gap");
                    assert_eq!(r, on_rat, "{kind:?} ring centre");
                }
                other => panic!("{kind:?} is not a ring: {other:?}"),
            }
        }
        // The stamps: 12×12 piles 16 apart; 12×12 pads ±15 across the rat;
        // 14×14 talons 16 forward and ±14 sideways.
        match MinibossKind::SaltHen.signatures()[0] {
            Signature {
                active,
                shape: S::Stamps { half, layout },
            } => {
                close(active, 0.9, "active");
                close(half * 2.0, 12.0, "pad");
                assert_eq!(layout, StampLayout::Along { spacing: 16.0 });
            }
            other => panic!("Salt Hen is not three stamps: {other:?}"),
        }
        match MinibossKind::CageCounter.signatures()[0] {
            Signature {
                shape: S::Stamps { half, layout },
                ..
            } => {
                close(half * 2.0, 12.0, "pad");
                assert_eq!(layout, StampLayout::AroundRat { across: 15.0 });
            }
            other => panic!("Cage Counter is not three stamps: {other:?}"),
        }
        match MinibossKind::RoadAtlas.signatures()[0] {
            Signature {
                shape: S::Stamps { half, layout },
                ..
            } => {
                close(half * 2.0, 14.0, "talon");
                assert_eq!(
                    layout,
                    StampLayout::V {
                        forward: 16.0,
                        across: 14.0
                    }
                );
            }
            other => panic!("The Road Atlas is not three stamps: {other:?}"),
        }
        // The Pup's raised cloth reaches 48 px after surfacing 40 px out.
        for (kind, len, width, active) in [
            (MinibossKind::CarbonTwin, 38.0, 20.0, 1.4),
            (MinibossKind::TarpaulinPup, 48.0, 18.0, 1.1),
        ] {
            match kind.signatures()[0] {
                Signature {
                    active: a,
                    shape: S::Spill { len: l, half_w },
                } => {
                    close(a, active, "active");
                    close(l, len, "length");
                    close(half_w * 2.0, width, "width");
                }
                other => panic!("{kind:?} is not a spill: {other:?}"),
            }
        }
        // The pincer: two 30×5 claws closing from 28 apart to 4 in 0.65.
        match MinibossKind::Mudlark.signatures()[0] {
            Signature {
                active,
                shape:
                    S::Pincer {
                        len,
                        half,
                        open,
                        closed,
                    },
            } => {
                close(active, 0.65, "active");
                close(len, 30.0, "claw length");
                close(half * 2.0, 5.0, "claw width");
                close(open, 28.0, "open");
                close(closed, 4.0, "closed");
            }
            other => panic!("The Mudlark is not a pincer: {other:?}"),
        }
        // The anchored rollers cover an 82 px lane with a 12 px seam.
        match MinibossKind::WringerBoy.signatures()[0] {
            Signature {
                active,
                shape:
                    S::Lane {
                        len,
                        seam_half,
                        half,
                        bar_len,
                    },
            } => {
                close(active, 1.0, "active");
                close(len, 82.0, "lane");
                close(seam_half * 2.0, 12.0, "seam");
                close(half * 2.0, 6.0, "bar width");
                close(bar_len, 32.0, "bar length");
            }
            other => panic!("The Wringer Boy is not rollers: {other:?}"),
        }
    }

    /// `all()` is the enum, in order, with 28 unique display names.
    #[test]
    fn all_has_twenty_eight_unique_names() {
        let all = MinibossKind::all();
        assert_eq!(all.len(), 28);
        let mut names: Vec<&'static str> = Vec::new();
        for (i, kind) in all.into_iter().enumerate() {
            assert_eq!(kind as usize, i);
            assert!(!names.contains(&kind.name()), "{} appears twice", kind.name());
            names.push(kind.name());
            // The design's own capitalisation, never all-caps or lower case.
            assert_eq!(kind.name().trim(), kind.name());
            assert!(kind.name().chars().next().is_some_and(|c| c.is_uppercase()));
        }
        // The three the design names with an article keep it, and the ones it
        // does not name with one do not.
        assert_eq!(MinibossKind::Mudlark.name(), "The Mudlark");
        assert_eq!(MinibossKind::WringerBoy.name(), "The Wringer Boy");
        assert_eq!(MinibossKind::RoadAtlas.name(), "The Road Atlas");
        assert_eq!(MinibossKind::ValveWidow.name(), "Valve Widow");
    }

    fn legacy_primaries() -> impl Iterator<Item = MinibossKind> {
        MinibossKind::all().into_iter().filter(|kind| matches!(kind.signatures()[0].shape,
            SignatureShape::Trench { .. } | SignatureShape::Cone { .. }
                | SignatureShape::Cord { .. } | SignatureShape::Ring { .. }
                | SignatureShape::Stamps { .. } | SignatureShape::Spill { .. }
                | SignatureShape::Pincer { .. } | SignatureShape::Lane { .. }))
    }

    /// A locked miniboss at a fixed spot, for the geometry tests.
    fn locked(kind: MinibossKind) -> Miniboss {
        mb(kind, Vec2::new(200.0, 160.0), Vec2::new(1.0, 0.0))
    }

    #[test]
    fn outfall_appointments_keep_the_shared_schedule_and_stats() {
        let rows = [
            (MinibossKind::Gasket, 180.0, 18.0, 0.7, 7.0, 2610.0, 270.0),
            (MinibossKind::Swell, 360.0, 22.0, 1.0, 7.5, 5136.0, 1498.0),
            (MinibossKind::Shade, 540.0, 26.0, 0.55, 7.0, 9216.0, 2736.0),
            (MinibossKind::FalseBulb, 720.0, 30.0, 0.20, 7.0, 8869.0, 4163.0),
        ];
        for (i, (kind, at, damage, active, radius, stage_two, practice)) in
            rows.into_iter().enumerate()
        {
            assert_eq!(schedule_for(Theme::Outfall, i), kind);
            assert_eq!(MINIBOSS_TIMES[i], at);
            assert_eq!(appointment_signature_damage(i), damage);
            assert_eq!(kind.body_radius(), radius);
            assert_eq!(kind.active_duration(), active);
            assert_eq!(kind.temperament().speed, 48.0);
            assert_eq!(kind.temperament().gap, 2.6);
            assert!((appointment_hp_for(1, i) - stage_two).abs() < 0.01);
            assert!((appointment_hp_for(0, i) - practice).abs() < 0.01);
        }
    }

    #[test]
    fn gasket_hole_clears_both_contact_and_rim() {
        let m = locked(MinibossKind::Gasket);
        let center = m.origin + m.aim * 16.0;
        let tell = m.kind.telegraph(&m);
        assert_eq!(tell, MinibossShape::Annulus { origin: center, inner: 8.0, outer: 18.0 });
        assert_eq!(m.kind.active_shape(&m, 0.35), Some(tell));
        assert!(center.dist(m.origin) > m.kind.body_radius() + PLAYER_RADIUS);
        assert!(!tell.contains(center + m.aim * 4.99, PLAYER_RADIUS));
        assert!(tell.contains(center + m.aim * 5.0, PLAYER_RADIUS));
        assert!(tell.contains(center + m.aim * 21.0, PLAYER_RADIUS));
        assert!(!tell.contains(center + m.aim * 21.01, PLAYER_RADIUS));
    }

    #[test]
    fn swell_tracks_a_sixteen_pixel_gap_then_sweeps_with_it_fixed() {
        let mut m = locked(MinibossKind::Swell);
        m.state = MinibossState::Tell;
        for (t, expected_gap) in [(0.0, (-20.0, -4.0)), (0.25, (-8.0, 8.0)), (0.5, (4.0, 20.0))] {
            m.timer = t;
            let MinibossShape::Tide { gap, len, from, to, .. } = m.kind.telegraph(&m) else {
                panic!("expected the Tide warning");
            };
            assert_eq!((gap, len, from, to), (expected_gap, 64.0, -3.0, 51.0));
        }
        let told = m.kind.telegraph(&m);
        m.timer = 0.9;
        assert_eq!(told, m.kind.telegraph(&m));
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let live = m.kind.active_shape(&m, t).expect("live bar");
            let middle = m.origin + m.aim * (48.0 * t);
            let sample = |y| middle + m.aim.perp() * y;
            for shape in [told, live] {
                assert!(!shape.contains(sample(12.0), PLAYER_RADIUS));
                assert!(!shape.contains(sample(7.01), PLAYER_RADIUS));
                assert!(shape.contains(sample(7.0), PLAYER_RADIUS));
                assert!(shape.contains(sample(17.0), PLAYER_RADIUS));
                assert!(!shape.contains(sample(36.0), PLAYER_RADIUS));
            }
        }
    }

    #[test]
    fn shade_rereads_live_lamps_at_lock_then_keeps_the_lit_side_safe() {
        use crate::game::level::{LightFixture, LightFixtureKind};

        let mut level = Level::empty(Theme::Outfall);
        let mut m = locked(MinibossKind::Shade);
        let body = m.origin;
        let x = m.aim;
        let y = x.perp();
        let rat = body + x * 36.0;
        level.objects.lights = [body - x * 60.0, body - y * 36.0].map(|pos| LightFixture {
            pos, kind: LightFixtureKind::Sconce, radius: 80.0, seed: 1,
        }).to_vec();
        let track = RatTrack::default();
        m.state = MinibossState::Tell;
        m.timer = 0.25;
        assert!(track_tell(&mut m, 0.2, body, rat, Vec2::ZERO, &level, &track));
        assert_eq!(m.aim, y);
        level.objects.outfall_elite_snuff = Some(1);
        m.timer = 0.51;
        assert!(track_tell(&mut m, 0.49, body, rat, Vec2::ZERO, &level, &track));
        assert_eq!(m.aim, x);
        let warned = m.kind.telegraph(&m);
        let MinibossShape::Umbra { half_angle, len, .. } = warned else { panic!("expected Umbra") };
        assert_eq!(len, 46.0);
        assert!((half_angle.to_degrees() * 2.0 - 50.0).abs() < 1e-4);
        assert!(!warned.contains(body - x * 20.0, PLAYER_RADIUS));
        assert!(warned.contains(body + x * 46.0, PLAYER_RADIUS));
        assert!(!warned.contains(body + x * 50.0, PLAYER_RADIUS));
        assert!(warned.contains(body + x.rotate(24.5_f32.to_radians()) * 40.0, 0.0));
        assert!(!warned.contains(body + x.rotate(25.5_f32.to_radians()) * 40.0, 0.0));
        level.objects.lights.clear();
        m.timer = 0.75;
        assert!(track_tell(&mut m, 0.7, body, rat, Vec2::ZERO, &level, &track));
        assert_eq!(warned, m.kind.telegraph(&m));
        assert_eq!(m.kind.active_shape(&m, 0.4), Some(warned));
        assert!(aim_signature(&mut m, body, rat, Vec2::ZERO, &level));
        assert_eq!(m.aim, -x, "without a live fixture the wedge points away from the rat");
    }

    #[test]
    fn false_bulb_selects_at_lock_and_only_the_selected_jaw_can_hit() {
        let mut m = locked(MinibossKind::FalseBulb);
        let level = Level::empty(Theme::Outfall);
        let body = m.origin;
        let x = m.aim;
        let y = x.perp();
        let rat = body + x * 24.0 + y * 8.0;
        let vel = -y * 16.0;
        m.state = MinibossState::Tell;
        m.timer = 0.35;
        let mut track = RatTrack::default();
        track.record(0.0, rat);
        assert!(track_tell(&mut m, 0.3, body, rat, vel, &level, &track));
        let tracking = m.kind.telegraph(&m);
        let MinibossShape::False { bulbs, selected: None, jaw_len, jaw_half, .. } = tracking else {
            panic!("expected both proposed jaws");
        };
        assert_eq!(bulbs, [body + x * 22.0 - y * 14.0, body + x * 22.0 + y * 14.0]);
        assert_eq!((jaw_len, jaw_half), (18.0, 3.0));
        assert!(bulbs.into_iter().all(|bulb| tracking.contains(bulb + x * 9.0, 0.0)));
        assert!(m.active_shape().is_none());

        m.timer = TELL_LOCK;
        assert!(track_tell(&mut m, 0.48, body, rat, vel, &level, &track));
        let committed = m.kind.telegraph(&m);
        assert!(matches!(committed, MinibossShape::False { selected: Some(1), .. }));
        assert_eq!(tracking.signature_bounds(), committed.signature_bounds());
        m.timer = 0.85;
        assert!(track_tell(&mut m, 0.8, body, bulbs[0], Vec2::ZERO, &level, &track));
        assert_eq!(m.kind.telegraph(&m), committed);
        assert!(m.active_shape().is_none());
        let live = m.kind.active_shape(&m, 0.0).expect("jaw at start of strike");
        assert_eq!(live, committed);
        assert!(live.contains(bulbs[1] + x * 18.0 + y * 3.0, 0.0));
        assert!(!live.contains(bulbs[1] + x * 18.01, 0.0));
        assert!(!live.contains(bulbs[0] + x * 9.0, PLAYER_RADIUS));
        assert!(m.kind.active_shape(&m, 0.201).is_none());

        m.target = body + x * 24.0;
        assert!(matches!(m.kind.telegraph(&m), MinibossShape::False { selected: Some(0), .. }));
    }

    #[test]
    fn outfall_signatures_each_latch_one_hit() {
        use crate::game::weapon::WeaponKind;

        for (kind, t, position) in [
            (MinibossKind::Gasket, 0.35, Vec2::new(29.0, 0.0)),
            (MinibossKind::Swell, 0.5, Vec2::new(24.0, 0.0)),
            (MinibossKind::Shade, 0.3, Vec2::new(24.0, 0.0)),
            (MinibossKind::FalseBulb, 0.1, Vec2::new(31.0, -14.0)),
        ] {
            let mut m = locked(kind);
            m.state = MinibossState::Active;
            m.timer = t;
            let live = m.active_shape().expect("active strike");
            let mut player = Player::new(m.origin + position, WeaponKind::Gnaw);
            assert!(apply_strike(&mut m, &mut player, &live, None), "{kind:?}");
            player.iframes = 0.0;
            assert!(!apply_strike(&mut m, &mut player, &live, None), "{kind:?}");
        }
    }

    #[test]
    fn outfall_escape_cannot_end_in_a_pit_or_pass_through_a_wall() {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::TileKind;
        use crate::game::level::{Pit, PitKind, TileRect};

        let mut level = Level::empty(Theme::Outfall);
        for y in 8..=12 {
            for x in 11..=16 {
                level.tiles.set(x, y, TileKind::Wall(WallKind::Solid));
            }
        }
        level.tiles.set(13, 10, TileKind::Open);
        level.tiles.set(14, 10, TileKind::Open);
        let rat = Vec2::new(216.0, 168.0);
        let m = mb(MinibossKind::Gasket, rat - Vec2::new(28.0, 0.0), Vec2::new(1.0, 0.0));
        assert!(escape_exists(&m, rat, &level), "the open bank is an escape");
        level.objects.tidal_pits.push(Pit::new(TileRect::new(14, 10, 1, 1), PitKind::TidalDrop));
        assert!(!escape_exists(&m, rat, &level), "the pit is the only exit");
        level.objects.tidal_pits.clear();
        level.tiles.set(14, 10, TileKind::Wall(WallKind::Solid));
        level.tiles.set(15, 10, TileKind::Open);
        assert!(!escape_exists(&m, rat, &level), "the open tile behind the wall is unreachable");
    }

    #[test]
    fn outfall_arrival_fits_the_named_radius_at_an_ordinary_ring_point() {
        use crate::game::level::{Pit, PitKind, TileRect};

        let mut level = Level::empty(Theme::Outfall);
        level.objects.tidal_pits.push(Pit::new(TileRect::new(112, 80, 1, 1), PitKind::TidalDrop));
        let mut scratch = SpawnScratch::default();
        scratch.sync(&level);
        let at = Vec2::new(1787.0, 1288.0);
        let radius = MinibossKind::Swell.body_radius();
        assert!(behaviour::clear(&level, at, 4.0, SizeClass::Large));
        assert!(!behaviour::clear(&level, at, radius, SizeClass::Large));
        let viewport = Vec2::new(57.0, 40.0);
        let fitted = fit_outfall_arrival(&level, &scratch, at, level.start, viewport, radius)
            .expect("clear sand behind the lip near the chosen arrival");
        assert!(behaviour::clear(&level, fitted, radius, SizeClass::Large));
        assert!(fitted.dist(at) < 16.0);
        assert!((fitted.x - level.start.x).abs() > widest_view(viewport).x * 0.5 + radius);
    }

    /// §8/`WAVE4_ROSTERS.md`: outline the full footprint for the whole tell.
    /// Everywhere the strike can bite was already marked, a swinging cord's
    /// whole swept sector included.
    #[test]
    fn the_tell_outlines_the_whole_strike_footprint() {
        for kind in legacy_primaries() {
            let m = locked(kind);
            let tell = kind.telegraph(&m);
            let d = kind.active_duration();
            // A swinging cord's tell is the wedge it sweeps. Along the wedge's
            // two edges the cord's own half-width overhangs by up to `half` px,
            // and its hub end sits inside the body: so for those five rows
            // "marked" means within half a cord's width, outside the body.
            let (pad, hub) = match kind.signatures()[0].shape {
                SignatureShape::Cord { half, .. } => (half, kind.body_radius()),
                _ => (0.0, 0.0),
            };
            for i in 0..=8 {
                let t = d * i as f32 / 8.0;
                let strike = kind
                    .active_shape(&m, t)
                    .unwrap_or_else(|| panic!("{kind:?}: no strike at {t}"));
                // A travelling ring's tell is its outermost rim (the Bell
                // Diver's wave-3 tell, unchanged): it *encloses* the strike
                // rather than painting it, so the claim is that the live ring
                // never leaves that rim and keeps the same centre and break.
                if let (
                    MinibossShape::RingGap {
                        origin: o0,
                        radius: r0,
                        half: h0,
                        gap_dir: g0,
                        gap_half: gh0,
                    },
                    MinibossShape::RingGap {
                        origin: o1,
                        radius: r1,
                        half: h1,
                        gap_dir: g1,
                        gap_half: gh1,
                    },
                ) = (tell, strike)
                {
                    assert!(o0.dist(o1) < 1e-3, "{kind:?}: the ring's centre moved");
                    assert!(r1 <= r0 + 1e-3, "{kind:?}: the ring outgrew its tell");
                    assert!((h0 - h1).abs() < 1e-6, "{kind:?}: the rim's width changed");
                    assert!(g0.dist(g1) < 1e-3, "{kind:?}: the break moved");
                    assert!((gh0 - gh1).abs() < 1e-6, "{kind:?}: the break's width changed");
                    continue;
                }
                // A coarse world grid around the tell: anywhere the strike
                // bites, the tell already marked.
                for gy in 0..=64 {
                    for gx in 0..=96 {
                        let q = m.origin
                            + Vec2::new((gx as f32 - 48.0) * 2.0, (gy as f32 - 32.0) * 2.0);
                        if q.dist(m.origin) <= hub {
                            continue;
                        }
                        if strike.contains(q, crate::game::ecs::constants::PLAYER_RADIUS) {
                            assert!(
                                tell.contains(
                                    q,
                                    crate::game::ecs::constants::PLAYER_RADIUS + pad
                                ),
                                "{kind:?}: {q:?} is struck at t={t} but was not marked"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Where `WAVE4_ROSTERS.md` says the answer is, row by row: the gap, the
    /// seam, behind the pivot, the open tip, off the sheet.
    fn safe_spot(m: &Miniboss) -> Vec2 {
        use MinibossKind::*;
        let o = m.origin;
        let aim = m.aim;
        let side = aim.perp();
        // The swinging arms, the wedge and the cone never come back past their
        // own origin.
        let behind = o - aim * 12.0;
        // Off the axis: wider than any trench, lane or cut is deep.
        let aside = o + aim * 20.0 + side * 16.0;
        // The break in the ring, on the rim it reaches.
        let gap_at = |r: f32| o + aim.rotate(RING_GAP_TURN) * r;
        match m.kind {
            Corkscrew | PastryWheel | DosePump | AxleNut => aside,
            SisterSnip | QuoinKey | ValveWidow | PotBrush | RotorTechnician | ChainDog | Bobbin => {
                behind
            }
            BellDiver => gap_at(RING_RADIUS),
            PetriMother => gap_at(36.0),
            SudsBailiff => gap_at(42.0),
            // The closing ring is centred on the rat: its break is the way out.
            LatePlate => m.rat_lock() + aim.rotate(RING_GAP_TURN) * 20.0,
            // Out through the open tip of the claws.
            Mudlark => o + aim * 44.0,
            // Down the central seam between the rollers.
            WringerBoy => o + aim * 22.0,
            // Off the sheet by its short edge.
            MilkCarton | CarbonTwin | TarpaulinPup => o + aim * 16.0 + side * 16.0,
            // Off the marked line.
            Foreman | SaltHen => o + aim * 16.0 + side * 16.0,
            // Clear of the last reader pad.
            CageCounter => m.rat_lock() + aim * 40.0,
            // Clear of the talon marks.
            RoadAtlas => m.rat_lock() + aim * 16.0 + side * 30.0,
            Gasket => o + aim * GASKET_OFFSET,
            Swell => o + aim * 24.0 + side * 12.0,
            Shade => behind,
            FalseBulb => o + aim * 28.0,
        }
    }

    /// Every legacy primary's documented answer works: the rat standing there is never
    /// touched, on any tick of the strike.
    #[test]
    fn every_kind_misses_a_rat_in_the_documented_safe_spot() {
        const R: f32 = crate::game::ecs::constants::PLAYER_RADIUS;
        for kind in legacy_primaries() {
            let m = locked(kind);
            let safe = safe_spot(&m);
            let d = kind.active_duration();
            for i in 0..=24 {
                // `d * 24 / 24` can land an ulp past `d`, where the window
                // has (correctly) already closed.
                let t = (d * i as f32 / 24.0).min(d);
                let strike = kind.active_shape(&m, t).expect("active");
                assert!(
                    !strike.contains(safe, R),
                    "{kind:?} bites the documented safe spot {safe:?} at t={t}"
                );
            }
        }
    }

    /// The tell tracks and leads until [`TELL_LOCK`], then commits: a rat that
    /// keeps its heading walks into the strike, a rat that turns at the lock
    /// walks out of it. (With the aim frozen on the first frame, any rat that
    /// simply kept moving was missed: the batch that found it counted about
    /// 2,800 signatures and 28 hits.)
    #[test]
    fn the_tell_leads_a_walking_rat_and_a_turn_at_the_lock_escapes() {
        use crate::game::level::Theme;
        use crate::game::world::World;
        const DT: f32 = 1.0 / 60.0;
        let kind = MinibossKind::MilkCarton;
        let walk = Vec2::new(0.0, 20.0);
        for turns in [false, true] {
            let mut w = World::new(Theme::Sewer, 23);
            w.ecs.resource_mut::<MinibossSchedule>().next = MINIBOSS_TIMES.len();
            w.player.hp = 1.0e6;
            w.elapsed = 30.0;
            let start = w.player.pos;
            let e = w.spawn_miniboss(kind, 0, start + Vec2::new(20.0, 0.0));
            let hp = w.player.hp;
            let mut vel = walk;
            let mut locked_aim = None;
            let ticks = (((TELL_TIME + kind.active_duration()) / DT) as usize) + 8;
            for _ in 0..ticks {
                let (state, timer, aim) = {
                    let mb = w.ecs.get::<Miniboss>(e).expect("the miniboss");
                    (mb.state, mb.timer, mb.aim)
                };
                if state == MinibossState::Tell && timer >= TELL_LOCK {
                    // Frozen from the lock on, whatever the rat does.
                    let locked = *locked_aim.get_or_insert(aim);
                    assert!((aim - locked).len() < 1e-6, "the aim moved after the lock");
                    if turns {
                        vel = -walk;
                    }
                }
                w.player.vel = vel;
                w.player.pos += vel * DT;
                w.player.iframes = 0.0;
                w.begin_tick(DT, Vec2::ZERO, false);
                w.ecs
                    .run_system_cached(miniboss_system)
                    .expect("miniboss system");
                w.end_tick();
            }
            let locked = locked_aim.expect("the tell reached its lock");
            // The lock leads the rat: the aim is turned toward where it was
            // heading, not at where it stood when the tell began.
            assert!(locked.y > 0.2, "the aim did not lead the walk: {locked:?}");
            let drop = hp - w.player.hp;
            if turns {
                assert_eq!(drop, 0.0, "a rat that turned at the lock was still bitten");
            } else {
                let armor = w.player.loadout.stats.armor;
                let one_hit = (appointment_signature_damage(0) - armor).max(1.0);
                assert!(
                    (drop - one_hit).abs() < 0.01,
                    "a rat that kept walking took {drop}, expected one hit of {one_hit}"
                );
            }
        }
    }

    /// Each of the eighteen new kinds dropped into a world dies to damage and
    /// leaves exactly one chest plus the appointment's gold.
    #[test]
    fn every_new_kind_dies_and_leaves_one_chest_and_gold() {
        use crate::game::level::Theme;
        use crate::game::shop::MINIBOSS_GOLD;
        use crate::game::world::World;
        for kind in MinibossKind::all() {
            let mut w = World::new(Theme::Sewer, 31);
            w.ecs.resource_mut::<MinibossSchedule>().next = MINIBOSS_TIMES.len();
            w.elapsed = 300.0;
            let p = w.player.pos;
            let e = w.spawn_miniboss(kind, 2, p + Vec2::new(30.0, 0.0));
            assert_eq!(
                w.ecs.get::<Enemy>(e).map(|x| x.max_hp),
                Some(appointment_hp(2)),
                "{kind:?}: slot 2's HP"
            );
            w.ecs.get_mut::<Enemy>(e).expect("body").hp = 0.0;
            reap(&mut w);
            assert_eq!(chests(&mut w), 1, "{kind:?}: a kill must leave exactly one chest");
            assert_eq!(
                gold_values(&mut w),
                vec![MINIBOSS_GOLD],
                "{kind:?}: the appointment's gold"
            );
            // A second reap (the Nuke's) must not pay twice.
            reap(&mut w);
            assert_eq!(chests(&mut w), 1, "{kind:?}: the chest was queued twice");
            let mut gems = w.ecs.query::<&crate::game::gem::Gem>();
            assert!(
                gems.iter(&w.ecs).any(|g| g.value == appointment_xp(2)),
                "{kind:?}: no gem worth the appointment's XP"
            );
        }
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
