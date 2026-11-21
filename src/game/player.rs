//! The player rat: movement, dash, health, XP/leveling, and loadout.

use crate::math::Vec2;

use super::loadout::Loadout;
use super::weapon::WeaponKind;

#[derive(bevy_ecs::resource::Resource)]
pub struct Player {
    pub pos: Vec2,
    pub vel: Vec2,
    pub hp: f32,
    pub loadout: Loadout,
    pub xp: u32,
    /// Fractional gem XP banked by the Blackboard Belly splice (see
    /// [`crate::game::shop::xp_with_bank`]); `0.0` without it.
    pub xp_bank: f32,
    pub level: u32,
    pub xp_to_next: u32,
    pub face: Vec2,
    pub dash_cd: f32,
    pub dash_active: f32,
    pub iframes: f32,
    pub hurt_flash: f32,
}

/// XP needed to go from `level` to `level + 1`.
///
/// Linear (+5 per level) for the first ten levels — that is the teaching
/// stretch and it should stay quick — then a quadratic drag
/// (`2/3 * (level-10)^2` extra). Without the drag a build that is killing 400
/// bodies a minute reached player level 90-140 by the boss, which made the
/// boss pool arbitrary and the last two minutes a formality. With it, a strong
/// run reaches the level-1 boss around level 25-30 and the levels it does earn
/// are all meaningful.
///
/// b4-upgrades: the drag was `(level-10)^2 / 2`, which put the measured level
/// at the first boss at 29-30 — the very top of the "15 to 30 at the first
/// boss" target, with no margin for the batch's other workers making level 1
/// (and therefore the kill rate, and therefore the XP) easier. `2/3` costs
/// about 4% more XP by level 20 and 12% by level 30. Measured on the 30-run
/// seed-900 batches, the level at the first boss became 28/29/29 (expert),
/// 25/26/27 (skilled) and 26/27/28 (casual) p25/median/p75 — inside the band
/// with margin. Nothing changes before level 11, so the teaching stretch and
/// the first level-up (the tutorial beat) are untouched.
///
/// A full `(level-10)^2` was measured too and is **too steep**: it cost 23
/// percentage points of level-1 clear rate (expert 83 %→60 %, skilled 67 %→40 %,
/// casual 33 %→20 %) for one player level at the boss. The XP curve is a
/// sensitive difficulty lever; `2/3` is the point that meets the level band
/// without moving the clear rate.
///
/// Totals (integer arithmetic, as the code computes them): level 10 = 270 XP
/// (unchanged), level 20 = 1 231 XP, level 30 = 3 958 XP.
pub fn xp_for_level(level: u32) -> u32 {
    let drag = level.saturating_sub(10);
    5 + level * 5 + drag * drag * 2 / 3
}

impl Player {
    pub fn new(pos: Vec2, start: WeaponKind) -> Self {
        let loadout = Loadout::new(start);
        Player {
            pos,
            vel: Vec2::ZERO,
            hp: loadout.stats.max_hp,
            loadout,
            xp: 0,
            xp_bank: 0.0,
            level: 1,
            xp_to_next: xp_for_level(1),
            face: Vec2::new(0.0, -1.0),
            dash_cd: 0.0,
            dash_active: 0.0,
            iframes: 0.0,
            hurt_flash: 0.0,
        }
    }

    pub fn max_hp(&self) -> f32 {
        self.loadout.stats.max_hp
    }

    /// Add XP; returns how many level-ups were triggered.
    pub fn add_xp(&mut self, amount: u32) -> u32 {
        self.xp += amount;
        let mut levels = 0;
        while self.xp >= self.xp_to_next {
            self.xp -= self.xp_to_next;
            self.level += 1;
            self.xp_to_next = xp_for_level(self.level);
            levels += 1;
        }
        levels
    }

    pub fn heal(&mut self, amount: f32) {
        self.hp = (self.hp + amount).min(self.max_hp());
    }

    /// Apply incoming damage through armor + i-frames. Returns true if it landed.
    ///
    /// Two gene splices are read here, because they are about the *hit* rather
    /// than a stat: Hollow Bones adds +3 incoming damage **before** armor, and
    /// Double Lid adds 0.22 s to the post-hit immunity. A rat with no splices
    /// takes exactly the legacy numbers.
    pub fn take_damage(&mut self, raw: f32) -> bool {
        if self.iframes > 0.0 {
            return false;
        }
        let raw = raw + super::shop::incoming_extra(&self.loadout.splices);
        let dmg = (raw - self.loadout.stats.armor).max(1.0);
        self.hp -= dmg;
        self.iframes = 0.9 + super::shop::iframe_extra(&self.loadout.splices);
        self.hurt_flash = 0.25;
        true
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_up_on_threshold() {
        let mut p = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
        let need = p.xp_to_next;
        let ups = p.add_xp(need);
        assert_eq!(ups, 1);
        assert_eq!(p.level, 2);
    }

    #[test]
    fn multi_level_in_one_gain() {
        let mut p = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
        let ups = p.add_xp(1000);
        assert!(ups >= 2);
    }

    #[test]
    fn iframes_block_damage() {
        let mut p = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
        assert!(p.take_damage(10.0));
        assert!(!p.take_damage(10.0)); // still invulnerable
    }

    /// Cumulative XP to reach a level, matching [`Player::add_xp`]'s integer
    /// arithmetic exactly (so the documented totals cannot drift).
    fn xp_to_reach(level: u32) -> u32 {
        (1..level).map(xp_for_level).sum()
    }

    #[test]
    fn xp_curve_totals_match_the_documented_band() {
        // The teaching stretch is linear and unchanged: 270 XP to level 10.
        assert_eq!(xp_to_reach(10), 270);
        // The drag is what bounds the snowball. These are the numbers the doc
        // comment quotes; a curve change that moves them must update both.
        assert_eq!(xp_to_reach(20), 1231);
        assert_eq!(xp_to_reach(30), 3958);
    }

    #[test]
    fn xp_curve_is_monotonic_and_never_cheapens() {
        for level in 2..=60 {
            let prev = xp_for_level(level - 1);
            let next = xp_for_level(level);
            assert!(
                next > prev,
                "level {level} costs {next} XP, not more than level {}'s {prev}",
                level - 1
            );
        }
    }

    #[test]
    fn reaching_level_30_costs_more_than_the_linear_curve_would() {
        // The drag has to bite: 30 levels of the flat 5+5*level curve would be
        // 2 325 XP, and the boss pool would be meaningless again.
        let linear: u32 = (1..30).map(|l| 5 + l * 5).sum();
        assert!(xp_to_reach(30) > linear * 3 / 2);
    }
}
