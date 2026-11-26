//! The player rat: movement, dash, health, XP/leveling, and loadout.

use crate::detmath::DetF64;
use crate::math::Vec2;

use super::collision::{SizeClass, TileMap};
use super::ecs::{constants::PLAYER_RADIUS, systems::powerups};
use super::loadout::Loadout;
use super::weapon::WeaponKind;

/// Brute clearance. A single 16 px doorway loses 1 px to each jamb, so
/// a 13 px diameter still leaves 1 px of steering room.
pub const GIANT_BODY_MAX_RADIUS: f32 = 6.5;

/// Timed pickup effects.
///
/// Every countdown is in **simulation seconds** and `0.0` means "inactive", so
/// a run that never finds one of these pickups draws no extra rng. A repeat
/// find refreshes the countdown to full (and, for the
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
    /// Growth Serum (batch 13): the rat is drawn five times its size, stomps
    /// the ordinary horde and shrugs off ordinary contact. 20 s.
    pub giant: f32,
    /// Shrink Draught: smaller hurt disc and faster feet. 12 s.
    pub tiny: f32,
    /// White Coat: damage immunity and suspended weapons. 5 s.
    pub white_coat: f32,
    /// The Collar: double gem XP and faster heavy pursuers. 16 s.
    pub collar: f32,
    /// Headlamp: a forward beam that reveals ground and steers the horde. 18 s.
    pub headlamp: f32,
}

impl Buffs {
    /// Every tick: age the countdowns. Returns the kinds that expired this
    /// tick, so the caller can play the release cue without a second pass.
    pub fn tick(&mut self, dt: f32) -> [bool; 9] {
        let mut expired = [false; 9];
        for (slot, was) in [
            (&mut self.shield, 0),
            (&mut self.slow_bullets, 1),
            (&mut self.socks, 2),
            (&mut self.decoy, 3),
            (&mut self.giant, 4),
            (&mut self.tiny, 5),
            (&mut self.white_coat, 6),
            (&mut self.collar, 7),
            (&mut self.headlamp, 8),
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
        self.shield > 0.0
            || self.slow_bullets > 0.0
            || self.socks > 0.0
            || self.decoy > 0.0
            || self.giant > 0.0
            || self.tiny > 0.0
            || self.white_coat > 0.0
            || self.collar > 0.0
            || self.headlamp > 0.0
    }

    /// Is the rat giant right now? (`Growth Serum`.) One reader-facing name for
    /// the state, so the draw pass, the collision rules and the stomp pass all
    /// ask the same question.
    #[inline]
    pub fn is_giant(&self) -> bool {
        self.giant > 0.0
    }

    /// Eased body and dash scale, including the size pickup warning blink.
    pub fn draw_scale(&self) -> f32 {
        if let Some(scale) = powerups::giant_draw_scale(self.giant) {
            return scale;
        }
        if self.tiny <= 0.0 {
            return 1.0;
        }
        let elapsed = (powerups::SHRINK_SECS - self.tiny).max(0.0);
        let ramp = elapsed.min(self.tiny).min(powerups::SHRINK_RAMP_SECS)
            / powerups::SHRINK_RAMP_SECS;
        let blink = self.tiny <= powerups::SHRINK_BLINK_SECS && (self.tiny * 6.0) as i32 % 2 == 0;
        let target = if blink {
            powerups::SHRINK_BLINK_SCALE
        } else {
            powerups::SHRINK_SCALE
        };
        1.0 - (1.0 - target) * ramp
    }

    /// The radius for incoming hit tests. Movement still uses the body radius.
    pub fn hurt_radius(&self) -> f32 {
        if let Some(scale) = powerups::giant_draw_scale(self.giant) {
            PLAYER_RADIUS * scale
        } else if self.tiny > 0.0 {
            powerups::SHRINK_HURT_RADIUS
        } else {
            PLAYER_RADIUS
        }
    }

    /// Add growth to a hit test with a fixed baseline clearance.
    pub fn giant_hit_padding(&self) -> f32 {
        (self.hurt_radius() - PLAYER_RADIUS).max(0.0)
    }

    /// Legacy centre-only tests keep their usual footprint outside the serum.
    pub fn point_hit_radius(&self) -> f32 {
        if self.is_giant() {
            self.hurt_radius()
        } else {
            0.0
        }
    }

    /// Routing uses the giant class for the whole serum, even at either end
    /// of the size animation.
    pub fn size_class(&self) -> SizeClass {
        if self.is_giant() {
            SizeClass::Large
        } else {
            SizeClass::Small
        }
    }

    pub fn desired_body_radius(&self) -> f32 {
        if self.is_giant() {
            self.hurt_radius().min(GIANT_BODY_MAX_RADIUS)
        } else {
            PLAYER_RADIUS
        }
    }

    pub fn weapons_suspended(&self) -> bool {
        self.white_coat > 0.0
    }

    pub fn move_speed_scale(&self) -> f32 {
        if self.is_giant() {
            powerups::GIANT_MOVE_MULT
        } else if self.tiny > 0.0 {
            powerups::SHRINK_MOVE_MULT
        } else {
            1.0
        }
    }

    /// How much of an incoming hit the rat takes while giant.
    ///
    /// Growth Serum halves everything that still reaches it: hostile pellets,
    /// traps, hazards, miniboss signatures and boss attacks. Ordinary enemy
    /// **contact** never gets here at all (`combat::contact_system` drops it
    /// while the rat is giant: those bodies are being stomped).
    #[inline]
    pub fn damage_taken_scale(&self) -> f32 {
        if self.is_giant() {
            powerups::GIANT_DAMAGE_TAKEN
        } else {
            1.0
        }
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
    /// Enabled for one tick by autoplay. No game systems read these values.
    pub(crate) observed_damage: Option<ObservedDamage>,
}

pub(crate) struct ObservedDamage {
    pub hp_lost: f32,
    pub min_hp_fraction: f32,
}

/// Weapon damage gained per character level above the first: +4 %, so level 14
/// hits ×1.52 and level 30 ×2.16. No cap.
///
/// Batch twelve quartered the level pace (one level now costs the XP of four
/// old levels, see [`xp_for_level`]) and multiplied this by four, so the bonus
/// a surviving rat carries at any given minute is the one it always did
/// (×1.19 by 3:00, ×1.55 by the boss) and no enemy pool had to move for it.
pub const LEVEL_DAMAGE_PER_LEVEL: f32 = 0.04;

/// The character level's weapon-damage multiplier
/// (`1 + LEVEL_DAMAGE_PER_LEVEL × (level − 1)`). The level restarts at 1 on
/// every floor, and so does this.
pub fn level_damage_mult(level: u32) -> f32 {
    1.0 + LEVEL_DAMAGE_PER_LEVEL * level.saturating_sub(1) as f32
}

/// The same bonus as a whole percentage, for the screens (`+44` at level 12,
/// `+152` at level 39). Read from the constant, never from the level alone.
pub fn level_damage_percent(level: u32) -> u32 {
    level_damage_percent_with(level, LEVEL_DAMAGE_PER_LEVEL)
}

pub(crate) fn level_damage_percent_with(level: u32, per_level: f32) -> u32 {
    let mult = 1.0 + per_level * level.saturating_sub(1) as f32;
    ((mult * 100.0).round() as u32).saturating_sub(100)
}

pub const XP_BASE: f64 = 4.84805895;

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
/// Those are **old** levels: batch twelve charges four of them per card
/// ([`OLD_LEVELS_PER_LEVEL`]), so the same economy now buys ~14 cards, not
/// ~55. The fine curve itself — and every measurement below — is unchanged.
///
/// Fitted against the measured per-checkpoint XP curve of the pre-change batch
/// (skilled, 8 campaigns: 24 XP at 1:00, 100 at 2:00, 257 at 3:00, 1 613 at
/// 6:00, 4 556 at 9:00, 8 946 at 12:00, 12 307 at 14:00). See
/// `scratchpad/w3/balance/LOG.md`.
pub const XP_GROWTH: f64 = 1.10;

/// Old levels whose XP one character level now costs ([`xp_for_level`]).
///
/// The owner: *"it feels like we level up way too fast. Maybe level up 1/4 as
/// often but increase DPS by 4x the current amount."* So a level costs what
/// four levels used to, and [`LEVEL_DAMAGE_PER_LEVEL`] went 0.01 → 0.04: the
/// same bonus lands on the same minute, with four times fewer cards.
pub const OLD_LEVELS_PER_LEVEL: u32 = 4;

/// **One old level's cost**, in XP: `round(XP_BASE * XP_GROWTH^(level-1))`, with
/// no cap. The round-three balance lowers the cost of a step, and
/// [`xp_for_level`] charges multiple steps per card after the first.
///
/// It is also the measuring stick the batch reports use through
/// [`xp_to_reach`], so the old level a run would have reached stays readable.
pub fn base_xp_for_level(level: u32) -> u32 {
    base_xp_for_level_with(level, &crate::tuning::Xp::default())
}

fn base_xp_for_level_with(level: u32, tuning: &crate::tuning::Xp) -> u32 {
    let n = level.max(1).saturating_sub(1) as i32;
    let cost = tuning.base * tuning.growth.dpowi(n);
    cost.round().clamp(1.0, u32::MAX as f64) as u32
}

/// Old levels the first three cards cost: one, two, three. From the fourth
/// card on every level costs [`OLD_LEVELS_PER_LEVEL`] of them.
///
/// The release pass measured level one with the flat four-old-level price: the
/// first card came at 77 s there and the rat met the 3:00 appointment with two
/// low ranks. The ramp puts the second weapon in the first half minute and the
/// third and fourth cards inside the first two, without changing what a level
/// costs for the rest of the floor.
pub const OPENING_OLD_LEVELS: [u32; 3] = [1, 2, 3];

/// The first old level `level`'s card is charged from, and how many it costs.
fn old_level_block(level: u32, tuning: &crate::tuning::Xp) -> (u32, u32) {
    let level = level.max(1);
    let ramp = tuning.opening_steps.len() as u32;
    if level <= ramp {
        let before: u32 = tuning.opening_steps[..(level - 1) as usize].iter().sum();
        (before + 1, tuning.opening_steps[(level - 1) as usize])
    } else {
        let before: u32 = tuning.opening_steps.iter().sum();
        (
            tuning.steps_per_level
                .saturating_mul(level - ramp - 1)
                .saturating_add(before + 1),
            tuning.steps_per_level,
        )
    }
}

/// XP needed to go from `level` to `level + 1`.
///
/// The first three cards cost 5, 11 and 21 XP at shipped values. Later
/// cards each charge the next four geometric steps, giving 39, 59, 85, …
/// No step is skipped when the number of steps per card increases.
pub fn xp_for_level(level: u32) -> u32 {
    xp_for_level_with(level, &crate::tuning::Xp::default())
}

fn xp_for_level_with(level: u32, tuning: &crate::tuning::Xp) -> u32 {
    let (first, count) = old_level_block(level, tuning);
    (first..first.saturating_add(count))
        .map(|level| base_xp_for_level_with(level, tuning))
        .fold(0u32, |acc, cost| acc.saturating_add(cost))
}

/// Cumulative XP banked to reach `level` from level 1 (the sum of every card's
/// cost up to but not including `level`), matching [`Player::add_xp`]'s integer
/// arithmetic exactly.
///
/// This is the balance pass's measuring stick: a run's total XP income is
/// `xp_to_reach(player.level) + player.xp`, so the batch report can plot the
/// economy against the cost curve without adding a counter to the simulation.
/// Saturating, because the four-old-level curve passes `u32::MAX` cumulative
/// at level 49 and a report must not panic on a level nobody reaches.
pub fn xp_to_reach(level: u32) -> u32 {
    (1..level.max(1)).map(xp_for_level).fold(0u32, |acc, cost| {
        acc.saturating_add(cost)
    })
}

impl Player {
    /// Hold growth at the widest circle supported by the current position.
    /// A rat already straddling a hole or gate can leave with ordinary
    /// clearance. The large class takes over once its minimum body fits.
    pub fn movement_body(&self, tiles: &TileMap) -> (f32, SizeClass) {
        self.movement_body_at(self.pos, tiles)
    }

    fn movement_body_at(&self, pos: Vec2, tiles: &TileMap) -> (f32, SizeClass) {
        if !self.buffs.is_giant() {
            return (PLAYER_RADIUS, SizeClass::Small);
        }
        let class = SizeClass::Large;
        let desired = self.buffs.desired_body_radius();
        if tiles.circle_clear(pos, desired, class) {
            return (desired, class);
        }
        if !tiles.circle_clear(pos, PLAYER_RADIUS, class) {
            return (PLAYER_RADIUS, SizeClass::Small);
        }
        let mut lo = PLAYER_RADIUS;
        let mut hi = desired;
        for _ in 0..8 {
            let mid = (lo + hi) * 0.5;
            if tiles.circle_clear(pos, mid, class) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo, class)
    }

    /// Keep a forced movement on the same side of walls and gates as the
    /// giant's ordinary movement. Callers keep their legacy small-rat motion.
    pub fn clip_giant_displacement(&self, from: Vec2, to: Vec2, tiles: &TileMap) -> Vec2 {
        if !self.buffs.is_giant() || from == to {
            return to;
        }
        let delta = to - from;
        let steps = (delta.len() / super::ecs::constants::PLAYER_SUBSTEP)
            .ceil()
            .max(1.0) as i32;
        let step = delta * (1.0 / steps as f32);
        let (mut radius, mut class) = self.movement_body_at(from, tiles);
        let mut pos = from;
        for _ in 0..steps {
            if class == SizeClass::Small {
                (radius, class) = self.movement_body_at(pos, tiles);
            }
            let next = pos + step;
            if !tiles.circle_clear(next, radius, class) {
                break;
            }
            pos = next;
        }
        pos
    }

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
            observed_damage: None,
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
        self.loadout.stats.damage_mult * self.level_damage_mult()
            * self.loadout.tuning.as_ref().map_or(1.0, |t| t.weapons.damage_mult)
    }

    pub fn level_damage_mult(&self) -> f32 {
        let per_level = self.loadout.tuning.as_ref().map_or(LEVEL_DAMAGE_PER_LEVEL, |t| t.xp.damage_per_level);
        1.0 + per_level * self.level.saturating_sub(1) as f32
    }

    pub fn level_damage_percent(&self) -> u32 {
        ((self.level_damage_mult() * 100.0).round() as u32).saturating_sub(100)
    }

    pub(crate) fn next_xp_cost(&self) -> u32 {
        self.loadout.tuning.as_ref().map_or_else(|| xp_for_level(self.level),
            |t| xp_for_level_with(self.level, &t.xp))
    }

    pub(crate) fn total_xp(&self) -> u32 {
        let tuning = self.loadout.tuning.as_ref();
        (1..self.level).fold(self.xp, |total, level| {
            total.saturating_add(tuning.map_or_else(|| xp_for_level(level), |t| xp_for_level_with(level, &t.xp)))
        })
    }

    pub(crate) fn dash_cooldown(&self) -> f32 {
        let mult = if self.loadout.has(WeaponKind::SuperDash) && !self.buffs.weapons_suspended() {
            self.loadout.weapon_cooldown_mult(WeaponKind::SuperDash)
        } else {
            1.0
        };
        self.loadout.stats.dash_cd * mult
    }

    pub(crate) fn dash_base_duration(&self) -> f32 {
        self.loadout.tuning.as_ref().map_or(super::ecs::systems::player::DASH_SECS, |t| t.player.dash_duration)
    }

    fn hit_iframes(&self) -> f32 {
        self.loadout.tuning.as_ref().map_or_else(
            || crate::tuning::Player::default().hit_iframes,
            |t| t.player.hit_iframes,
        )
    }

    /// Add XP; returns how many level-ups were triggered.
    pub fn add_xp(&mut self, amount: u32) -> u32 {
        self.xp = self.xp.saturating_add(amount);
        let mut levels = 0;
        while self.xp >= self.xp_to_next {
            self.xp -= self.xp_to_next;
            self.level += 1;
            self.xp_to_next = self.next_xp_cost();
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
        if self.god || self.buffs.white_coat > 0.0 {
            return false;
        }
        if self.iframes > 0.0 {
            return false;
        }
        if self.buffs.shield > 0.0 {
            self.buffs.shield = 0.0;
            self.iframes = self.hit_iframes() + super::shop::iframe_extra(&self.loadout.splices);
            self.hurt_flash = 0.25;
            return true;
        }
        let raw = raw + super::shop::incoming_extra(&self.loadout.splices);
        // Growth Serum: the giant takes half from everything but ordinary
        // contact, which `contact_system` never lands while the serum runs.
        let raw = raw * self.buffs.damage_taken_scale();
        let dmg = (raw - self.loadout.stats.armor).max(1.0);
        let hp_before = self.hp;
        self.hp -= dmg;
        if self.observed_damage.is_some() {
            let fraction = self.health_fraction();
            if let Some(observation) = &mut self.observed_damage {
                observation.hp_lost += hp_before.max(0.0) - self.hp.max(0.0);
                observation.min_hp_fraction = observation.min_hp_fraction.min(fraction);
            }
        }
        self.iframes = self.hit_iframes() + super::shop::iframe_extra(&self.loadout.splices);
        self.hurt_flash = 0.25;
        true
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }

    pub(crate) fn health_fraction(&self) -> f32 {
        (self.hp / self.max_hp().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0)
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
        let base_iframes = crate::tuning::Player::default().hit_iframes;
        let long = base_iframes + 0.22;
        assert_eq!(crate::game::shop::iframe_extra(&spliced().loadout.splices), 0.22);

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
        assert_eq!(plain.iframes, base_iframes);
        plain.iframes = 0.0;
        let hp = plain.hp;
        assert!(plain.take_damage(10.0));
        assert_eq!(plain.hp, hp - 10.0);
    }

    /// Sample the shipped geometric curve at the opening and later steps.
    #[test]
    fn the_old_fine_curve_is_geometric_and_uncapped() {
        assert_eq!(base_xp_for_level(0), 5);
        assert_eq!(base_xp_for_level(1), 5);
        assert_eq!(base_xp_for_level(2), 5);
        assert_eq!(base_xp_for_level(3), 6);
        assert_eq!(base_xp_for_level(10), 11);
        assert_eq!(base_xp_for_level(20), 30);
        assert_eq!(base_xp_for_level(30), 77);
        assert_eq!(base_xp_for_level(40), 199);
        assert_eq!(base_xp_for_level(50), 517);
        assert!(base_xp_for_level(60) > base_xp_for_level(50));
    }

    /// Batch twelve: one level costs four old levels' XP, after the opening
    /// ramp of one, two and three (the release pass), so the ~56 old-level
    /// cards by the boss become ~16 and the long-run pace is a quarter.
    #[test]
    fn a_level_costs_four_old_levels_after_the_opening_ramp() {
        // The opening charges one, two, then three geometric steps.
        assert_eq!(OPENING_OLD_LEVELS, [1, 2, 3]);
        assert_eq!(xp_for_level(0), 5);
        assert_eq!(xp_for_level(1), 5);
        assert_eq!(xp_for_level(2), 11);
        assert_eq!(xp_for_level(3), 21);

        // Every later level is exactly the next block of four old levels, from
        // 4·level − 9.
        for level in 4..=60 {
            let first = OLD_LEVELS_PER_LEVEL * level - 9;
            let block: u32 = (first..first + OLD_LEVELS_PER_LEVEL)
                .map(base_xp_for_level)
                .fold(0, |a, c| a.saturating_add(c));
            assert_eq!(xp_for_level(level), block, "level {level}");
        }
        assert_eq!(xp_for_level(4), 39);
        assert_eq!(xp_for_level(5), 59);
        assert_eq!(xp_for_level(10), 393);
        assert_eq!(xp_for_level(12), 840);
        assert_eq!(xp_for_level(20), 17770);
    }

    /// Cumulative anchors: no old level is skipped, so reaching new level
    /// `n ≥ 4` costs exactly what old level `4n − 9` did.
    #[test]
    fn xp_to_reach_is_the_old_cumulative_curve_at_a_quarter_pace() {
        assert_eq!(xp_to_reach(0), 0);
        assert_eq!(xp_to_reach(1), 0);
        assert_eq!(xp_to_reach(2), 5, "the first card is one old level");
        assert_eq!(xp_to_reach(3), 16);
        assert_eq!(xp_to_reach(4), 37);
        assert_eq!(xp_to_reach(5), 76);
        assert_eq!(xp_to_reach(9), 529);
        assert_eq!(xp_to_reach(11), 1191);
        assert_eq!(xp_to_reach(16), 8282);
        assert_eq!(xp_to_reach(17), 12149);
        let old_reach = |old_level: u32| -> u32 {
            (1..old_level).map(base_xp_for_level).fold(0, |a, c| a.saturating_add(c))
        };
        for level in 4..=20u32 {
            assert_eq!(
                xp_to_reach(level),
                old_reach(OLD_LEVELS_PER_LEVEL * level - 9),
                "new level {level}"
            );
        }
        // Saturating, not wrapping.
        assert_eq!(xp_to_reach(80), u32::MAX);
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
    fn the_level_bonus_is_four_percent_a_level_with_no_cap() {
        assert_eq!(level_damage_mult(0), 1.0);
        assert_eq!(level_damage_mult(1), 1.0);
        assert!((level_damage_mult(2) - 1.04).abs() < 1e-6);
        assert!((level_damage_mult(14) - 1.52).abs() < 1e-6);
        assert!((level_damage_mult(31) - 2.20).abs() < 1e-5);
        assert!((level_damage_mult(51) - 3.0).abs() < 1e-5);
        assert_eq!(level_damage_percent(1), 0);
        assert_eq!(level_damage_percent(2), 4);
        assert_eq!(level_damage_percent(14), 52);
        assert_eq!(level_damage_percent(40), 156);
        // The bonus a typical rat carries does not move: the mapping old level
        // -> new level is (old + 3) / 4, and 0.04 × (new − 1) = 0.01 × (old −
        // 1), which is exactly how `director`'s `LEVEL_BONUS_HP` knots were
        // built — so those knots stay where they are.
        for (old_level, typical_new, knot) in [
            (5.0f32, 2.0f32, 1.04f32),
            (13.0, 4.0, 1.12),
            (20.0, 5.75, 1.19),
            (35.0, 9.5, 1.34),
            (45.0, 12.0, 1.44),
            (52.0, 13.75, 1.51),
            (56.0, 14.75, 1.55),
        ] {
            let new = (old_level + 3.0) / 4.0;
            assert!((new - typical_new).abs() < 1e-6, "old {old_level} -> {new}");
            let bonus = 1.0 + LEVEL_DAMAGE_PER_LEVEL * (new - 1.0);
            assert!((bonus - knot).abs() < 1e-4, "old {old_level}: {bonus} vs {knot}");
        }
        assert_eq!(level_damage_percent(12), 44);
        // A fresh rat (every floor starts one) is at level 1: no bonus.
        let p = Player::new(Vec2::ZERO, WeaponKind::Gnaw);
        assert_eq!(p.damage_mult(), p.loadout.stats.damage_mult);
    }

}
