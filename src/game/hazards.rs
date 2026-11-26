//! Walkable floor hazards: steam grates, ink pads and the Sewer's drip shaft.
//!
//! These are **walkable** floor hazards (they never paint a solid cell), placed
//! by `levelgen::hazards` into the Sewer, Laundry and Printworks and driven by
//! `ecs::systems::hazards`. The structs here are the one copy of the hazard
//! state: the generator builds them, the system mutates them and the floor art
//! reads them, so the drawn plume and the damaging plume cannot drift.
//!
//! ## Steam grate (`WAVE3_DESIGN.md` §11)
//!
//! Three pale puffs warn for 1 s, then a one-tile plume deals 8 once per
//! occupant over 0.7 s, then the grate rests for 4 s. It hurts the horde as
//! well as the rat: pursuers path over the grate and get scalded, which is what
//! makes a side lane worth using even when it is not the short way round.
//!
//! ```text
//! Resting (4.0 s) --> Warning (1.0 s) --> Plume (0.7 s) --> Resting ...
//! ```
//!
//! Each grate runs its own clock, started from a per-grate phase offset the
//! generator draws, so a washer aisle does not vent in unison.
//!
//! ## Ink pad
//!
//! A permanent stipple: ground movement crossing it runs at
//! [`INK_SLOW`] of normal speed, for the rat and for pursuers equally. Flying
//! (`EnemyKind::Bat`) and a dash ignore it entirely — that is the whole
//! counter-play.

use crate::detmath::DetF32;
use crate::game::collision::{cell_of, segment_rect_hit, SizeClass, TileMap, TILE_F};
use crate::game::level::Level;
use crate::math::Vec2;

// ---- Outfall ---------------------------------------------------------------

pub const LURE_ARM_RADIUS: f32 = 16.0;
pub const LURE_WARN_SECS: f32 = 0.7;
pub const LURE_BULB_RADIUS: f32 = 6.0;
pub const LURE_LIGHT_RANGE: f32 = 26.0;
pub const OUTFALL_LURE_COUNT: usize = 6;
pub const SURGE_PERIOD: f32 = 9.0;
pub const SURGE_WARN_SECS: f32 = 1.0;
pub const SURGE_ACTIVE_SECS: f32 = 1.1;
pub const SURGE_DISTANCE: f32 = 64.0;

/// The angular step from one surge's push direction to the next: the golden
/// angle. With the jitter below, consecutive bursts are always at least
/// 77.5° apart and the sequence still looks arbitrary (it is not a compass
/// direction and not a repeat).
const SURGE_STEP: f32 = 2.399_963_2;
/// Jitter added to one surge's step, in radians (±30°).
const SURGE_JITTER: f32 = std::f32::consts::FRAC_PI_6;
/// How far a surge's push may wander from its starting direction over the
/// 1.1 s it runs, in radians (±35°).
pub const SURGE_WANDER: f32 = 0.610_865_4;

/// One 64-bit mix, used to draw a surge's angle from `(seed, index)`.
#[inline]
fn surge_hash(seed: u64, index: u64) -> u64 {
    let mut h = seed ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// A `0.0..1.0` draw from `(seed, index)`, salted so one surge has several
/// independent ones (its phase, its jitter and the phase of its wander).
#[inline]
fn surge_unit(seed: u64, index: u64, salt: u64) -> f32 {
    // The top 24 bits, so the value stays under 1.0.
    (surge_hash(seed ^ salt, index) >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// The push direction a surge starts with: the level's own hashed phase, the
/// surge's index on the golden-angle step, and a hashed ±30° jitter. The
/// phase belongs to the seed alone: hashed per surge it would undo the step,
/// and two bursts in a row could push nearly the same way.
fn surge_start_angle(seed: u64, index: u64) -> f32 {
    let phase = surge_unit(seed, 0, 0x51ed_2b1f_00c3_a5d9) * std::f32::consts::TAU;
    let jitter = (surge_unit(seed, index, 0x2f9e_4a77_1b6d_c03b) * 2.0 - 1.0) * SURGE_JITTER;
    phase + index as f32 * SURGE_STEP + jitter
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LurePhase {
    Ready,
    Warning,
    Spent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LureTrap {
    pub pos: Vec2,
    pub phase: LurePhase,
    pub warning_left: f32,
    /// Age of the spent pulse. A failed release uses the same clock.
    pub spent_age: f32,
    pub failed: bool,
    /// Generator-ranked positions, checked again at release.
    pub landings: Vec<Vec2>,
}

impl LureTrap {
    pub fn new(pos: Vec2, landings: Vec<Vec2>) -> Self {
        Self {
            pos,
            phase: LurePhase::Ready,
            warning_left: 0.0,
            spent_age: 0.0,
            failed: false,
            landings,
        }
    }

    pub fn is_lit(&self) -> bool {
        self.phase != LurePhase::Spent
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurgePhase {
    Quiet,
    Warning,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurgeView {
    pub dir: Vec2,
    pub phase: SurgePhase,
}

/// One shared clock for simulation, warnings and the visible current.
///
/// The push direction belongs to the **surge**, not the map: it is hashed
/// from `(seed, surge index = floor(elapsed / SURGE_PERIOD))` to any angle, so
/// two bursts of one level never push the same way. Within a burst it wanders
/// smoothly by up to ±[`SURGE_WANDER`] (a sine of the surge's own hashed
/// phase), and the one-second warning before it shows exactly the direction
/// the burst will start with. Pure in `(seed, elapsed)`: no rng draw, no
/// state, so the crests, the bot and the lighting cannot disagree.
pub fn outfall_surge(seed: u64, elapsed: f32) -> SurgeView {
    let clock = elapsed % SURGE_PERIOD;
    let cycle = (elapsed / SURGE_PERIOD).floor().max(0.0);
    // The burst that opens at this cycle's start (there is none before the
    // first period), or the one the warning at the end of this cycle is for.
    let (index, progress, phase) = if elapsed >= SURGE_PERIOD
        && elapsed < cycle * SURGE_PERIOD + SURGE_ACTIVE_SECS
    {
        (
            (cycle as u64).max(1),
            ((elapsed - cycle * SURGE_PERIOD) / SURGE_ACTIVE_SECS).clamp(0.0, 1.0),
            SurgePhase::Active,
        )
    } else if clock >= SURGE_PERIOD - SURGE_WARN_SECS {
        (cycle as u64 + 1, 0.0, SurgePhase::Warning)
    } else {
        (cycle as u64 + 1, 0.0, SurgePhase::Quiet)
    };
    // The wander starts and ends at zero, so a burst's opening direction is
    // exactly its hashed start angle (and consecutive bursts stay >= 60°
    // apart); the hashed bit only decides which way it bends.
    let bend = if surge_unit(seed, index, 0xa5d9_51ed_c3b0_2f9e) < 0.5 {
        -1.0
    } else {
        1.0
    };
    let angle = surge_start_angle(seed, index)
        + bend * SURGE_WANDER * (progress * std::f32::consts::TAU).dsin();
    SurgeView {
        dir: Vec2::from_angle(angle),
        phase,
    }
}

/// Distance covered by the sine-bell current over this elapsed-time slice.
/// Subtract the integrated curve at each endpoint so even a tick crossing
/// either edge of the surge gets just its share of the 64 px.
pub fn outfall_surge_distance(previous_elapsed: f32, elapsed: f32) -> f32 {
    let period = f64::from(SURGE_PERIOD);
    let duration = f64::from(SURGE_ACTIVE_SECS);
    let cumulative = |time: f32| {
        let time = f64::from(time);
        let cycle = (time / period).floor();
        if cycle < 1.0 {
            return 0.0;
        }
        let progress = (time - cycle * period) / duration;
        if progress >= 1.0 {
            return cycle;
        }
        // sin²(πt/2) integrates the normalized half-sine speed and is
        // numerically stable at the very start of the surge.
        let s = libm::sin(progress * std::f64::consts::FRAC_PI_2);
        cycle - 1.0 + s * s
    };
    ((cumulative(elapsed) - cumulative(previous_elapsed)).max(0.0)
        * f64::from(SURGE_DISTANCE)) as f32
}

/// Also call this for the visible current. The caller combines the director
/// handoff, a due arrival and a Lantern present on the floor.
pub fn outfall_surge_enabled(arriving_or_present: bool, testbed: bool, elapsed: f32) -> bool {
    !arriving_or_present && (testbed || elapsed < crate::game::director::BOSS_TIME)
}

/// True for a legal Outfall footprint, including the dry side of pit lips.
pub fn outfall_sand_clear(level: &Level, pos: Vec2, radius: f32, class: SizeClass) -> bool {
    let (cx, cy) = cell_of(pos);
    level.tiles.in_bounds(cx, cy)
        && level.tiles.one_way_dir(cx, cy).is_none()
        && level.tiles.circle_clear(pos, radius.max(0.001), class)
        && !level.touches_tidal_pit(pos, radius)
}

/// Stop at the first obstacle on a current or boss pull. The swept pit and
/// wall bounds are conservatively expanded by the full body radius. Gate
/// leaves need a separate check because they let projectiles through.
pub fn clamp_outfall_displacement(
    level: &Level,
    from: Vec2,
    step: Vec2,
    radius: f32,
    class: SizeClass,
) -> Vec2 {
    let length = step.len();
    if length <= 0.0 || !outfall_sand_clear(level, from, radius, class) {
        return from;
    }
    let to = from + step;
    let mut first = level.tiles.segment_hit(from, to, radius).unwrap_or(1.0);
    for pit in &level.objects.tidal_pits {
        if let Some(hit) = segment_rect_hit(from, to, radius, &pit.rect.to_world()) {
            first = first.min(hit);
        }
    }
    let (x0, y0) = cell_of(Vec2::new(from.x.min(to.x) - radius, from.y.min(to.y) - radius));
    let (x1, y1) = cell_of(Vec2::new(from.x.max(to.x) + radius, from.y.max(to.y) + radius));
    for y in y0..=y1 {
        for x in x0..=x1 {
            if level.tiles.one_way_dir(x, y).is_some() {
                if let Some(hit) = segment_rect_hit(from, to, radius, &TileMap::tile_rect(x, y)) {
                    first = first.min(hit);
                }
            }
        }
    }

    let end = if first < 1.0 {
        from + step * (first - 0.01 / length).max(0.0)
    } else {
        to
    };
    if outfall_sand_clear(level, end, radius, class) {
        end
    } else {
        from
    }
}

/// A Conger uses the same radius and ray at placement and at release.
pub fn lure_landing_clear(level: &Level, bulb: Vec2, at: Vec2) -> bool {
    let step = at - bulb;
    (20.0 * 20.0..=32.0 * 32.0).contains(&step.len_sq())
        && clamp_outfall_displacement(level, bulb, step, 4.0, SizeClass::Large).dist_sq(at)
            < 0.000001
}


// ---- Steam ----------------------------------------------------------------

/// Seconds of pale puffs before the plume (design: 1 s).
pub const STEAM_WARN_SECS: f32 = 1.0;
/// Seconds the plume is up (design: 0.7 s).
pub const STEAM_PLUME_SECS: f32 = 0.7;
/// Seconds the grate rests between plumes (design: 4 s).
pub const STEAM_REST_SECS: f32 = 4.0;
/// The whole vent cycle: warn + plume + rest.
pub const STEAM_CYCLE_SECS: f32 = STEAM_WARN_SECS + STEAM_PLUME_SECS + STEAM_REST_SECS;
/// Damage one plume deals to an occupant, once per occupant (design: 8).
pub const STEAM_DAMAGE: f32 = 8.0;
/// The plume covers exactly the grate's own 16 px cell.
pub const STEAM_HALF: f32 = TILE_F * 0.5;

/// Which beat of the vent cycle a grate is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SteamPhase {
    /// Shut, with [`STEAM_REST_SECS`] between plumes.
    #[default]
    Resting,
    /// Three puffs: the 1 s tell.
    Warning,
    /// The scalding one-tile plume.
    Plume,
}

impl SteamPhase {
    /// `0.0..=1.0` progress through the current beat (the floor art's driver).
    pub fn progress(self, timer: f32) -> f32 {
        let span = match self {
            SteamPhase::Resting => STEAM_REST_SECS,
            SteamPhase::Warning => STEAM_WARN_SECS,
            SteamPhase::Plume => STEAM_PLUME_SECS,
        };
        (timer / span).clamp(0.0, 1.0)
    }
}

/// One steam grate: its cell, its phase clock, and who it has already scalded
/// in the current plume.
#[derive(Clone, Copy, Debug)]
pub struct SteamGrate {
    /// Centre of the grate's cell in world pixels.
    pub pos: Vec2,
    /// The grate's cell.
    pub cell: (i32, i32),
    /// Which beat the grate is on.
    pub phase: SteamPhase,
    /// Seconds elapsed in the current beat.
    pub timer: f32,
    /// The rat was already scalded by this plume.
    pub player_struck: bool,
    /// Run-clock seconds (for the art's puff animation).
    pub anim: f32,
}

impl SteamGrate {
    /// A grate starting `phase_offset` seconds into its vent cycle.
    pub fn new(cell: (i32, i32), phase_offset: f32) -> SteamGrate {
        let pos = Vec2::new(
            (cell.0 as f32 + 0.5) * TILE_F,
            (cell.1 as f32 + 0.5) * TILE_F,
        );
        let mut grate = SteamGrate {
            pos,
            cell,
            phase: SteamPhase::Resting,
            timer: 0.0,
            player_struck: false,
            anim: 0.0,
        };
        grate.seek(phase_offset);
        grate
    }

    /// Advance the phase clock to `offset` seconds into the cycle (used by the
    /// generator so neighbouring grates are out of step, and by tests).
    pub fn seek(&mut self, offset: f32) {
        let mut t = offset.rem_euclid(STEAM_CYCLE_SECS);
        self.phase = if t < STEAM_REST_SECS {
            SteamPhase::Resting
        } else {
            t -= STEAM_REST_SECS;
            if t < STEAM_WARN_SECS {
                SteamPhase::Warning
            } else {
                t -= STEAM_WARN_SECS;
                SteamPhase::Plume
            }
        };
        self.timer = t;
        self.player_struck = false;
    }

    /// Advance one tick, crossing phase edges. Returns `true` when a plume
    /// *starts* this tick (the system's cue to reset the struck lists).
    pub fn tick(&mut self, dt: f32) -> bool {
        self.anim += dt;
        if dt <= 0.0 {
            return false;
        }
        let mut started = false;
        self.timer += dt;
        loop {
            let span = match self.phase {
                SteamPhase::Resting => STEAM_REST_SECS,
                SteamPhase::Warning => STEAM_WARN_SECS,
                SteamPhase::Plume => STEAM_PLUME_SECS,
            };
            if self.timer < span {
                break;
            }
            self.timer -= span;
            self.phase = match self.phase {
                SteamPhase::Resting => SteamPhase::Warning,
                SteamPhase::Warning => SteamPhase::Plume,
                SteamPhase::Plume => SteamPhase::Resting,
            };
            if self.phase == SteamPhase::Plume {
                started = true;
                self.player_struck = false;
            }
        }
        started
    }

    /// Is the plume up?
    pub fn plume(&self) -> bool {
        self.phase == SteamPhase::Plume
    }

    /// Is the 1 s tell showing?
    pub fn warning(&self) -> bool {
        self.phase == SteamPhase::Warning
    }

    /// Does `p` (a circle of `radius`) overlap the grate's cell?
    pub fn covers(&self, p: Vec2, radius: f32) -> bool {
        (p.x - self.pos.x).abs() <= STEAM_HALF + radius
            && (p.y - self.pos.y).abs() <= STEAM_HALF + radius
    }
}

// ---- Ink ------------------------------------------------------------------

/// Ground movement multiplier while crossing ink (design: 75 %).
pub const INK_SLOW: f32 = 0.75;

/// One permanent ink pad: a flat, walkable stipple on the press floor.
#[derive(Clone, Copy, Debug)]
pub struct InkPad {
    /// Centre of the pad in world pixels.
    pub pos: Vec2,
    /// Cells the stain covers (always a rectangle of whole cells).
    pub cells: crate::game::level::TileRect,
    /// Half-extent in world pixels.
    pub half: Vec2,
    /// The generator's stain seed, so the stipple reads differently per pad
    /// without any per-frame randomness.
    pub seed: u32,
}

impl InkPad {
    /// The pad covering `cells`.
    pub fn new(cells: crate::game::level::TileRect, seed: u32) -> InkPad {
        InkPad {
            pos: cells.center_world(),
            cells,
            half: Vec2::new(
                cells.w as f32 * TILE_F * 0.5,
                cells.h as f32 * TILE_F * 0.5,
            ),
            seed,
        }
    }

    /// Is the point `p` (a circle of `radius`) on the ink?
    pub fn covers(&self, p: Vec2, radius: f32) -> bool {
        (p.x - self.pos.x).abs() <= self.half.x + radius
            && (p.y - self.pos.y).abs() <= self.half.y + radius
    }
}

/// The authored encounters in the first Sewer sector. Candidate spawn points
/// are ordered by distance to the second room's centre.
#[derive(Clone, Debug)]
pub struct SewerOpening {
    pub pillbug_cells: Vec<Vec2>,
    pub bottle_cap: Vec2,
    pub drip_shaft: DripShaft,
}

pub const DRIP_ARM_PX: f32 = 48.0;
pub const DRIP_WARN_SECS: f32 = 0.5;
pub const DRIP_SPLASH_SECS: f32 = 0.3;
pub const DRIP_REST_SECS: f32 = 2.5;
pub const DRIP_DAMAGE: f32 = 6.0;
pub const DRIP_HALF: f32 = TILE_F * 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DripPhase {
    Idle,
    Warning,
    Splash,
    Resting,
}

#[derive(Clone, Copy, Debug)]
pub struct DripShaft {
    pub cell: (i32, i32),
    pub pos: Vec2,
    pub phase: DripPhase,
    /// Seconds elapsed in the current phase.
    pub timer: f32,
    pub player_struck: bool,
}

impl DripShaft {
    pub fn new(cell: (i32, i32)) -> Self {
        Self {
            cell,
            pos: crate::game::collision::TileMap::tile_rect(cell.0, cell.1).center(),
            phase: DripPhase::Idle,
            timer: 0.0,
            player_struck: false,
        }
    }

    /// Finish an armed cycle even if the rat leaves. Start a fresh one only
    /// when the rat is in range, with a full warning on the first arming tick.
    pub fn tick(&mut self, dt: f32, rat_near: bool) {
        if dt <= 0.0 {
            return;
        }
        if self.phase == DripPhase::Idle {
            if rat_near {
                self.phase = DripPhase::Warning;
            }
            return;
        }
        self.timer += dt;
        loop {
            let span = match self.phase {
                DripPhase::Idle => return,
                DripPhase::Warning => DRIP_WARN_SECS,
                DripPhase::Splash => DRIP_SPLASH_SECS,
                DripPhase::Resting => DRIP_REST_SECS,
            };
            if self.timer < span {
                return;
            }
            self.timer -= span;
            self.phase = match self.phase {
                DripPhase::Idle => unreachable!(),
                DripPhase::Warning => {
                    self.player_struck = false;
                    DripPhase::Splash
                }
                DripPhase::Splash => DripPhase::Resting,
                DripPhase::Resting if rat_near => DripPhase::Warning,
                DripPhase::Resting => {
                    self.timer = 0.0;
                    DripPhase::Idle
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_grate_rests_then_warns_then_plumes_then_rests() {
        let mut g = SteamGrate::new((10, 10), 0.0);
        assert_eq!(g.phase, SteamPhase::Resting);
        // Walk the clock forward a frame at a time and record the phase order.
        let mut order = Vec::new();
        let dt = 1.0 / 60.0;
        for _ in 0..((STEAM_CYCLE_SECS / dt) as i32 + 4) {
            if g.tick(dt) {
                order.push(SteamPhase::Plume);
            }
            if order.last() != Some(&g.phase) {
                order.push(g.phase);
            }
        }
        assert_eq!(
            order,
            vec![
                SteamPhase::Resting,
                SteamPhase::Warning,
                SteamPhase::Plume,
                SteamPhase::Resting
            ]
        );
    }

    #[test]
    fn phase_offsets_stagger_neighbouring_grates() {
        let a = SteamGrate::new((0, 0), 0.0);
        let b = SteamGrate::new((1, 0), STEAM_REST_SECS + 0.5);
        assert_eq!(a.phase, SteamPhase::Resting);
        assert_eq!(b.phase, SteamPhase::Warning);
        // A plume is exactly one cell wide plus the occupant's radius.
        assert!(a.covers(a.pos, 0.0));
        assert!(!a.covers(a.pos + Vec2::new(STEAM_HALF + 1.0, 0.0), 0.0));
        assert!(a.covers(a.pos + Vec2::new(STEAM_HALF, 0.0), 2.0));
    }

    #[test]
    fn ink_pads_cover_their_cells_and_nothing_else() {
        use crate::game::level::TileRect;
        let pad = InkPad::new(TileRect::new(4, 4, 2, 1), 7);
        assert!(pad.covers(pad.pos, 0.0));
        // Just inside the far edge (plus a small body radius) still counts.
        assert!(pad.covers(pad.pos + Vec2::new(pad.half.x, 0.0), 1.0));
        // A cell past the pad does not.
        assert!(!pad.covers(pad.pos + Vec2::new(pad.half.x + TILE_F, 0.0), 0.0));
    }
}
