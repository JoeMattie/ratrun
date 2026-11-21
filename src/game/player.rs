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
            dash_ready_flash: 0.0,
            dash_denied: 0.0,
            buffs: Buffs::default(),
            decoy_at: None,
            god: false,
            speed_mult: 1.0,
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
