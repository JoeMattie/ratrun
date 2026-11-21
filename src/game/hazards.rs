//! The wave 3 side routes' floor hazards: steam grates and ink pads.
//!
//! Both are **walkable** floor hazards (they never paint a solid cell), placed
//! by `levelgen::hazards` into the Laundry and Printworks maps and driven by
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

use crate::game::collision::TILE_F;
use crate::math::Vec2;

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
