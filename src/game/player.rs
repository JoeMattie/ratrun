//! The player rat: movement, dash, health, XP/leveling, and loadout.

use crate::math::Vec2;

use super::loadout::Loadout;
use super::weapon::WeaponKind;

/// Timed buffs from the wave-3 pocket finds (WAVE3_DESIGN §10).
///
/// Every countdown is in **simulation seconds** and `0.0` means "inactive", so
/// a run that never finds one of these pickups carries four zeros and draws no
/// extra rng. A repeat find refreshes the countdown to full (and, for the
/// decoy, moves the key).
///
/// They live on the player rather than in a separate resource because the
/// facade already round-trips `World::player` every tick, so the HUD can read
/// them from `&World` with no new accessor and no twin-spare plumbing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Buffs {
    /// Bottle Cap: absorb the next damaging hit (and still grant the normal
    /// post-hit immunity window). 12 s.
    pub shield: f32,
    /// Pocket Watch: hostile pellets travel at 40 % speed. 4 s.
    pub slow_bullets: f32,
    /// Clean Socks: sticky boards cannot grab or slow the rat. 6 s.
    pub socks: f32,
    /// Wind-up Mouse: the brass key is on the floor (`powerups::Decoy` carries
    /// its position and timer; this mirror is what the HUD counts down). 5 s.
    pub decoy: f32,
}

impl Buffs {
    /// Every tick: age the countdowns. Returns the kinds that expired this
    /// tick, so the caller can play the release cue without a second pass.
    pub fn tick(&mut self, dt: f32) -> [bool; 4] {
        let mut expired = [false; 4];
        for (slot, was) in [
            (&mut self.shield, 0),
            (&mut self.slow_bullets, 1),
            (&mut self.socks, 2),
            (&mut self.decoy, 3),
        ] {
            if *slot > 0.0 {
                *slot = (*slot - dt).max(0.0);
                expired[was] = *slot == 0.0;
            }
        }
        expired
    }

    /// Is any timed buff running? (The HUD skips its strip when not.)
    pub fn any(&self) -> bool {
        self.shield > 0.0 || self.slow_bullets > 0.0 || self.socks > 0.0 || self.decoy > 0.0
    }
}

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
    /// Seconds left of the "dash is back" pop: set the tick the cooldown runs
    /// out, read by the on-rat gauge (`art_rat::draw_dash_gauge`) and the HUD's
    /// dash module. Purely visual, no rng, no sim effect.
    pub dash_ready_flash: f32,
    /// Seconds left of the "dash refused" shake: set when the player presses
    /// the dash key while the cooldown is still running, so a failed press is
    /// never silent (gauge shake + `Sfx::DashDenied`). Visual only.
    pub dash_denied: f32,
    /// **Super Dash**'s charge meter while the dash input is held, `0.0` to
    /// `1.0`. Written by `systems::player::player_system` from the held dash
    /// input (only while the weapon is owned) and read by the on-rat arc
    /// (`art_rat::draw_dash_gauge`), the lane preview and the HUD's dash
    /// module. A tap never fills it: the release fires an ordinary dash.
    pub dash_charge: f32,
    /// Is the dash input being held for a charge right now? Distinguishes "a
    /// fresh press, meter empty" from "no charge running at all", which is
    /// what an empty `dash_charge` alone cannot say.
    pub dash_charging: bool,
    /// Wave-3 pocket finds (Bottle Cap / Pocket Watch / Clean Socks / the
    /// Wind-up Mouse's HUD mirror). See [`Buffs`].
    pub buffs: Buffs,
    /// Where the Wind-up Mouse's key belongs: set by the collection pass and
    /// consumed by `powerups_system`, which owns the decoy entity's lifecycle
    /// (spawn, move on a repeat find, expire). `None` means "no request".
    pub decoy_at: Option<Vec2>,
    /// Console `god on`: every damage source is ignored. Set between ticks by
    /// the console; read in [`Player::take_damage`] so it is the single funnel.
    pub god: bool,
    /// Console `speed x`: a multiplier on the rat's move speed. Defaults to
    /// `1.0` (no change); `player_system` multiplies it into the throttle.
    pub speed_mult: f32,
}

/// First card's cost, in XP. Six XP is the design's "first card in 12-20 s"
/// anchor: the opening horde hands that over in about fifteen seconds.
/// Weapon damage gained per character level above the first: +1 %, so level
/// 31 hits ×1.30 and level 51 ×1.50. No cap.
pub const LEVEL_DAMAGE_PER_LEVEL: f32 = 0.01;

/// The character level's weapon-damage multiplier
/// (`1 + LEVEL_DAMAGE_PER_LEVEL × (level − 1)`). The level restarts at 1 on
/// every floor, and so does this.
pub fn level_damage_mult(level: u32) -> f32 {
    1.0 + LEVEL_DAMAGE_PER_LEVEL * level.saturating_sub(1) as f32
}

/// The same bonus as a whole percentage, for the screens (`+11` at level 12).
pub fn level_damage_percent(level: u32) -> u32 {
    level.saturating_sub(1)
}

pub const XP_BASE: f64 = 6.0;

/// Growth ratio of the geometric XP curve.
///
/// The wave-3 balance pass replaced the flat-plus-linear curve (`6, 8, 10, …,
/// capped at 100`) with a geometric one on the owner's direction: *"the game is
/// too linear, more XP needed to get higher level"*. With the flat cap the
/// measured economy (≈14 800 XP collected on a fifteen-minute level) reached
/// player level ~150, which left the build complete by minute six and made the
/// boss pools trivial. At 1.10 the same economy buys ~55 picks by 14:00 and
/// ~57 by the boss: the design's 50-56 band, with the pace visibly slowing
/// (level 20 costs 37 XP, level 40 costs 247, level 50 costs 640) and no flat
/// cap.
///
/// Fitted against the measured per-checkpoint XP curve of the pre-change batch
/// (skilled, 8 campaigns: 24 XP at 1:00, 100 at 2:00, 257 at 3:00, 1 613 at
/// 6:00, 4 556 at 9:00, 8 946 at 12:00, 12 307 at 14:00). See
/// `scratchpad/w3/balance/LOG.md`.
pub const XP_GROWTH: f64 = 1.10;

/// XP needed to go from `level` to `level + 1`.
///
/// `round(XP_BASE * XP_GROWTH^(level-1))`, with no cap. The curve is cheap in
/// the teaching band (6, 7, 7, 8, 9, 10 …) so the first cards land within a
/// minute, and grows geometrically from there, so each later level takes
/// noticeably longer than the one before it.
pub fn xp_for_level(level: u32) -> u32 {
    let n = level.max(1).saturating_sub(1) as i32;
    let cost = XP_BASE * XP_GROWTH.powi(n);
    cost.round().clamp(1.0, u32::MAX as f64) as u32
}

/// Cumulative XP banked to reach `level` from level 1 (the sum of every card's
/// cost up to but not including `level`), matching [`Player::add_xp`]'s integer
/// arithmetic exactly.
///
/// This is the balance pass's measuring stick: a run's total XP income is
/// `xp_to_reach(player.level) + player.xp`, so the batch report can plot the
/// economy against the cost curve without adding a counter to the simulation.
pub fn xp_to_reach(level: u32) -> u32 {
    (1..level.max(1)).map(xp_for_level).sum()
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
            dash_ready_flash: 0.0,
            dash_denied: 0.0,
            dash_charge: 0.0,
            dash_charging: false,
            buffs: Buffs::default(),
            decoy_at: None,
            god: false,
            speed_mult: 1.0,
        }
    }

    pub fn max_hp(&self) -> f32 {
        self.loadout.stats.max_hp
    }

    /// The multiplier on every weapon's damage: the build's
    /// (`Stats::damage_mult` — Sharp Teeth, the nest, splices) times the
    /// character level's ([`level_damage_mult`]).
    ///
    /// This is the **one** place the two are composed. Every weapon pass reads
    /// it when it fires (the auto-fire pass, the active weapons, Laser Eyes,
    /// Super Dash and its Ripcord), and a pool, a mark or an evolved effect
    /// carries the damage it was fired with, so nothing downstream multiplies
    /// again.
    pub fn damage_mult(&self) -> f32 {
        self.loadout.stats.damage_mult * level_damage_mult(self.level)
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
    /// This is the **single funnel** every damage source goes through (enemy
    /// bullets, contact, mousetraps, boss hazards and boss contact), which is
    /// why the Bottle Cap's shield is checked here rather than at each call
    /// site: one charge absorbs exactly one hit from any of them.
    ///
    /// A blocked hit still reports `true` (the shot is consumed, the hazard
    /// counts as spent) and still opens the normal post-hit immunity window, so
    /// the rat cannot be hit twice by the same boss beat: the cap buys one
    /// hit, not one tick of invulnerability.
    ///
    /// Two gene splices are read here, because they are about the *hit* rather
    /// than a stat: Hollow Bones adds +3 incoming damage **before** armor, and
    /// Double Lid adds 0.22 s to the post-hit immunity. A rat with no splices
    /// takes exactly the legacy numbers.
    ///
    /// Order: i-frames first, then the shield, then the splices. A hit the
    /// Bottle Cap blocks deals no damage at all, so Hollow Bones adds nothing
    /// to it; Double Lid lengthens the immunity window either way, because
    /// the window opens on a blocked hit too.
    pub fn take_damage(&mut self, raw: f32) -> bool {
        if self.god {
            return false;
        }
        if self.iframes > 0.0 {
            return false;
        }
        if self.buffs.shield > 0.0 {
            self.buffs.shield = 0.0;
            self.iframes = 0.9 + super::shop::iframe_extra(&self.loadout.splices);
            self.hurt_flash = 0.25;
            return true;
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

    /// The hit funnel's order with a Bottle Cap, Hollow Bones and Double Lid
    /// all in play: i-frames, then the shield, then the splices.
    #[test]
    fn shield_then_hollow_bones_with_double_lid_either_way() {
        use crate::game::shop::SpliceKind;
        let spliced = || {
            let mut p = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
            p.loadout.splices = vec![SpliceKind::HollowBones, SpliceKind::DoubleLid];
            p.loadout.rebuild_stats();
            p
        };
        let long = 0.9 + crate::game::shop::iframe_extra(&spliced().loadout.splices);
        assert!((long - 1.12).abs() < 1e-6);

        // A shielded hit: consumed, no damage at all (so no Hollow Bones +3),
        // and the Double Lid window opens all the same.
        let mut p = spliced();
        p.buffs.shield = 12.0;
        let hp = p.hp;
        assert!(p.take_damage(10.0), "a blocked hit is still consumed");
        assert_eq!(p.hp, hp, "the cap blocked it: Hollow Bones adds nothing");
        assert_eq!(p.buffs.shield, 0.0, "one charge, one hit");
        assert_eq!(p.iframes, long, "Double Lid lengthens a blocked hit's window");
        // Inside the window nothing lands and nothing is spent.
        assert!(!p.take_damage(10.0));
        assert_eq!(p.hp, hp);

        // The next, unshielded hit: +3 before armor, same long window.
        p.iframes = 0.0;
        assert!(p.take_damage(10.0));
        assert_eq!(p.hp, hp - 13.0, "Hollow Bones: +3 incoming before armor");
        assert_eq!(p.iframes, long);

        // No splices: the cap's window and the hit are the legacy numbers.
        let mut plain = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
        plain.buffs.shield = 12.0;
        assert!(plain.take_damage(10.0));
        assert_eq!(plain.iframes, 0.9);
        plain.iframes = 0.0;
        let hp = plain.hp;
        assert!(plain.take_damage(10.0));
        assert_eq!(plain.hp, hp - 10.0);
    }

    #[test]
    fn xp_curve_is_geometric_and_uncapped() {
        // First card 6 XP, then ×1.10 per card, rounded, with no flat cap.
        assert_eq!(xp_for_level(1), 6);
        assert_eq!(xp_for_level(2), 7);
        assert_eq!(xp_for_level(3), 7);
        assert_eq!(xp_for_level(10), 14);
        assert_eq!(xp_for_level(20), 37);
        assert_eq!(xp_for_level(30), 95);
        assert_eq!(xp_for_level(40), 247);
        assert_eq!(xp_for_level(50), 640);
        // Cumulative anchors: the design's 50-56 pick band by 14:00 needs
        // ≈3.9k-11.3k collected XP, which the measured economy supplies.
        assert_eq!(xp_to_reach(45), 3919);
        assert_eq!(xp_to_reach(50), 6346);
        assert_eq!(xp_to_reach(55), 10254);
        // No cap: the cost keeps growing past the old 100 ceiling.
        assert!(xp_for_level(60) > xp_for_level(50));
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
    #[test]
    fn the_level_bonus_is_one_percent_a_level_with_no_cap() {
        assert_eq!(level_damage_mult(0), 1.0);
        assert_eq!(level_damage_mult(1), 1.0);
        assert!((level_damage_mult(2) - 1.01).abs() < 1e-6);
        assert!((level_damage_mult(31) - 1.30).abs() < 1e-6);
        assert!((level_damage_mult(51) - 1.50).abs() < 1e-6);
        assert!((level_damage_mult(201) - 3.0).abs() < 1e-5);
        assert_eq!(level_damage_percent(1), 0);
        assert_eq!(level_damage_percent(12), 11);
        // A fresh rat (every floor starts one) is at level 1: no bonus.
        let p = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
        assert_eq!(p.damage_mult(), p.loadout.stats.damage_mult);
    }

}
