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
/// **Wave-3 §1 XP budget.** The first card costs 6 XP, then +2 per card, capped
/// at 100. The curve is deliberately cheap in the teaching stretch and flat
/// late: `WAVE3_DESIGN.md` §1 anchors the build milestones at 2,250 cumulative
/// XP through pick 45 and 2,944 through pick 52 (both matched exactly by this
/// integer curve). A surviving player should see 50-56 picks by 14:00.
pub fn xp_for_level(level: u32) -> u32 {
    (6 + level.saturating_sub(1) * 2).min(100)
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
    pub fn take_damage(&mut self, raw: f32) -> bool {
        if self.iframes > 0.0 {
            return false;
        }
        let dmg = (raw - self.loadout.stats.armor).max(1.0);
        self.hp -= dmg;
        self.iframes = 0.9;
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
    fn xp_curve_matches_the_wave3_budget() {
        // §1: first card 6 XP, +2 per card, cap 100.
        assert_eq!(xp_for_level(1), 6);
        assert_eq!(xp_for_level(2), 8);
        assert_eq!(xp_for_level(48), 100);
        assert_eq!(xp_for_level(80), 100);
        // Cumulative anchors: pick 45 costs 2,250 and pick 52 costs 2,944.
        assert_eq!(xp_to_reach(46), 2250);
        assert_eq!(xp_to_reach(53), 2944);
    }

    #[test]
    fn xp_curve_is_monotonic_and_never_cheapens() {
        for level in 2..=70 {
            let prev = xp_for_level(level - 1);
            let next = xp_for_level(level);
            assert!(
                next >= prev,
                "level {level} costs {next} XP, less than level {}'s {prev}",
                level - 1
            );
        }
    }
}
