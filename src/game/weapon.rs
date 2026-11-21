//! Auto-firing weapons. Each weapon has a cooldown and a firing pattern;
//! the actual spawning lives in `world.rs` so it can see all entities.

use crate::render::palette::Rgb;

// ---- Active-weapon tuning ------------------------------------------------
//
// These are the *feel* numbers of the two player-triggered weapons (per-level
// scaling lives on `Weapon::damage`/`cooldown`/`active_reach`). They are
// documented here, next to the weapon stats, so a tuning pass has one place to
// look.

/// Seconds one Tail Thrash burst lasts (a 1 s storm of three full sweeps).
pub const THRASH_BURST: f32 = 1.0;
/// How many sweeps one Tail Thrash burst makes. The burst's sweep interval is
/// `THRASH_BURST / THRASH_SWEEPS`, so the damage count and the art's three
/// revolutions cannot drift apart.
pub const THRASH_SWEEPS: u32 = 3;
/// Radians per second the thrash arc sweeps at: three full turns per burst, so
/// each drawn revolution is one damaging sweep.
pub const THRASH_SWEEP_RATE: f32 = std::f32::consts::TAU * 3.0;
/// Knockback velocity (px/s) handed to a regular enemy by a sweep.
pub const THRASH_KNOCKBACK: f32 = 260.0;
/// Bosses are heavy: a sweep only nudges them by this fraction of the above
/// (the thrash's answer to a boss is damage, not crowd control).
pub const THRASH_BOSS_KNOCKBACK: f32 = 0.15;
/// Screen shake (the `Shake` resource) on the trigger frame.
pub const THRASH_SHAKE: f32 = 5.0;

/// Seconds the Sonic Squeak ring takes to expand to its final radius.
pub const SQUEAK_EXPAND: f32 = 0.30;
/// Stun applied to regular enemies the ring passes.
pub const SQUEAK_STUN: f32 = 1.5;
/// Elites are slowed instead of stunned (fraction of their speed).
pub const SQUEAK_ELITE_SLOW: f32 = 0.35;
/// The boss is slowed for a shorter window and less severely.
pub const SQUEAK_BOSS_SLOW: f32 = 0.5;
pub const SQUEAK_BOSS_SLOW_SECS: f32 = 0.8;
/// Outward shove (px/s of velocity) the ring gives everything it passes.
pub const SQUEAK_SHOVE: f32 = 70.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WeaponKind {
    Gnaw,        // homing dart at nearest enemy
    CheeseSpray, // forward spread of darts
    SqueakNova,  // radial burst in all directions
    SporeOrbit,  // orbiting projectiles around the player
    TailWhip,    // melee aura pulse around the player
    AcidSpit,    // lobbed shot that leaves a damaging pool
    TailThrash,  // ACTIVE: charged burst of sweeping tail sweeps
    SonicSqueak, // ACTIVE: charged expanding stunning ring
}

impl WeaponKind {
    pub fn all() -> [WeaponKind; 8] {
        [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::SqueakNova,
            WeaponKind::SporeOrbit,
            WeaponKind::TailWhip,
            WeaponKind::AcidSpit,
            WeaponKind::TailThrash,
            WeaponKind::SonicSqueak,
        ]
    }

    /// Is this weapon **player-triggered** rather than auto-firing?
    ///
    /// An active weapon never fires on its own: its `Weapon::timer` counts down
    /// as a charge meter and only a Space press (or a click-dash) spends it.
    /// See `game::ecs::systems::weapons::active_weapons_system`.
    pub fn is_active(self) -> bool {
        matches!(self, WeaponKind::TailThrash | WeaponKind::SonicSqueak)
    }

    /// Position in [`WeaponKind::all()`]; the index of this weapon in the
    /// balance ledger (`ecs::resources::DamageLedger`) and in batch reports.
    pub fn index(self) -> usize {
        match self {
            WeaponKind::Gnaw => 0,
            WeaponKind::CheeseSpray => 1,
            WeaponKind::SqueakNova => 2,
            WeaponKind::SporeOrbit => 3,
            WeaponKind::TailWhip => 4,
            WeaponKind::AcidSpit => 5,
            WeaponKind::TailThrash => 6,
            WeaponKind::SonicSqueak => 7,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Gnaw",
            WeaponKind::CheeseSpray => "Cheese Spray",
            WeaponKind::SqueakNova => "Squeak Nova",
            WeaponKind::SporeOrbit => "Spore Orbit",
            WeaponKind::TailWhip => "Tail Whip",
            WeaponKind::AcidSpit => "Acid Spit",
            WeaponKind::TailThrash => "Tail Thrash",
            WeaponKind::SonicSqueak => "Sonic Squeak",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Fires a dart at the nearest foe.",
            WeaponKind::CheeseSpray => "Scatters darts in a forward arc.",
            WeaponKind::SqueakNova => "Bursts shots in every direction.",
            WeaponKind::SporeOrbit => "Spores circle and shred nearby foes.",
            WeaponKind::TailWhip => "Pulses damage to everything close.",
            WeaponKind::AcidSpit => "Lobs acid that pools and burns.",
            // The "ACTIVE" prefix is what the level-up card shows, so the
            // player learns which weapons the Space bar fires (task: the first
            // two player-triggered weapons).
            WeaponKind::TailThrash => "ACTIVE: press SPACE to thrash a long sweeping arc.",
            WeaponKind::SonicSqueak => "ACTIVE: press SPACE to squeak a stunning ring.",
        }
    }

    pub fn flavor(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Teeth are the original weapon.",
            WeaponKind::CheeseSpray => "Weaponized dairy. Don't ask.",
            WeaponKind::SqueakNova => "A scream pitched past hearing. It still hurts.",
            WeaponKind::SporeOrbit => "Fungal hitchhikers from Vat 9, now loyal.",
            WeaponKind::TailWhip => "Forty grams of pure indignation.",
            WeaponKind::AcidSpit => "Stomach contents, redirected.",
            WeaponKind::TailThrash => "The tail remembered every boot.",
            WeaponKind::SonicSqueak => "Past hearing, straight into the skull.",
        }
    }

    pub fn color(self) -> Rgb {
        match self {
            WeaponKind::Gnaw => (255, 240, 180),
            WeaponKind::CheeseSpray => (255, 210, 90),
            WeaponKind::SqueakNova => (200, 160, 255),
            WeaponKind::SporeOrbit => (140, 255, 180),
            WeaponKind::TailWhip => (255, 150, 200),
            WeaponKind::AcidSpit => (160, 255, 90),
            WeaponKind::TailThrash => (255, 178, 92),
            WeaponKind::SonicSqueak => (140, 240, 255),
        }
    }

    pub fn max_level(self) -> u8 {
        5
    }

    /// Base cooldown (seconds) before fire-rate stats are applied.
    pub fn base_cooldown(self) -> f32 {
        match self {
            WeaponKind::Gnaw => 0.55,
            WeaponKind::CheeseSpray => 0.95,
            WeaponKind::SqueakNova => 1.8,
            WeaponKind::SporeOrbit => 4.0,
            WeaponKind::TailWhip => 1.1,
            WeaponKind::AcidSpit => 1.5,
            // Active weapons: the cooldown *is* the charge time, so it is the
            // panic-button cost. Level and the Frenzy passive shorten it like
            // every other weapon's (`Weapon::cooldown`).
            WeaponKind::TailThrash => 10.0,
            WeaponKind::SonicSqueak => 6.0,
        }
    }

    /// Base damage per hit before weapon levels, damage passives and the
    /// player's fire-rate stats.
    ///
    /// Raised by ~60% across the board (b3-balance): the old numbers did
    /// ~9 dps into a 6-14 hp skitterer, i.e. ~0.5 bodies/s, against a spawn
    /// curve that reaches 3.5/s by 60 s. The XP engine could not start, so a
    /// level-1 build fell behind from the first minute and only the runs that
    /// survived that cliff snowballed. These values let a starting Gnaw keep
    /// its screen clear at a modest trickle, which is what makes the first
    /// minute teach instead of execute.
    pub fn base_damage(self) -> f32 {
        match self {
            // b4-upgrades: the first measurement of *dealt* damage by weapon
            // (the batch report's per-weapon table) showed a 3-to-1 spread
            // that was not the old "Tail Whip is half the boss damage" story:
            // Tail Whip led total damage (27%) but was the worst boss weapon
            // (9%), while Acid Spit was best at both (25% / 28%) and Spore
            // Orbit was a dud everywhere (7% / 7%). These numbers lift the
            // two weak weapons and shave the two best; the measured band
            // became 7-29% of total damage and 6-27% of boss damage, with no
            // weapon under 8% and none over 29%.
            WeaponKind::Gnaw => 9.0,
            WeaponKind::CheeseSpray => 7.0,
            WeaponKind::SqueakNova => 6.5,
            WeaponKind::SporeOrbit => 9.0,
            WeaponKind::TailWhip => 10.0,
            WeaponKind::AcidSpit => 5.0,
            // Tail Thrash books this once **per sweep** (three sweeps per
            // burst), so a L1 burst is 3 x 26 = 78 damage to everything in a
            // 60 px reach for 10 s. The first measurement (100 seed-900
            // campaigns per skill) put these two at 48 % of all enemy damage
            // between them; Squeak paid 12 and was 23 % on its own, which is a
            // crowd-clear weapon wearing a utility weapon's card. 9 keeps the
            // stun as the reason to press it (a L1 ring is 27 damage to the
            // horde for the price of a 1.5 s stun, not a third of a burst) and
            // leaves the thrash as the damagedealer of the pair.
            WeaponKind::TailThrash => 26.0,
            WeaponKind::SonicSqueak => 9.0,
        }
    }
}

/// One row of the chest-evolution table (`WAVE3_DESIGN.md` §3).
///
/// A weapon evolves when a chest is opened while it is at its max level and
/// the loadout owns **at least one stack** of `passive`. The evolution lives on
/// the weapon slot (`Weapon::evolved`): it keeps the weapon's kind, timing
/// family and damage ledger column, and locks once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evolution {
    /// The passive whose first stack opens this evolution.
    pub passive: super::loadout::PassiveKind,
    /// The chest name (`"Milk Teeth"`), shown wherever the weapon is.
    pub name: &'static str,
    /// The chest inscription — the flavour line on the chest card.
    pub flavor: &'static str,
    /// Card text for the evolved attack (chest card and level-up cards).
    pub attack: &'static str,
}

impl WeaponKind {
    /// The evolution this weapon can wear, if any.
    ///
    /// Eight rows are live. The four other rows in the design table belong to
    /// weapons that are not in this branch yet (Acid Rain, Hantavirus, Laser
    /// Eyes, Super Dash); their spec text and the hooks they need are stubbed
    /// in `game::ecs::systems::evolutions` under "the four rows that arrive
    /// with their weapons".
    pub fn evolution(self) -> Option<Evolution> {
        let e = |passive, name, flavor, attack| Some(Evolution { passive, name, flavor, attack });
        match self {
            // Milk Teeth — the lead dart lodges, then comes home biting.
            WeaponKind::Gnaw => e(
                super::loadout::PassiveKind::Damage,
                "Milk Teeth",
                "They always come back in.",
                "The lead dart lodges for 1 s, then pulls back to you, biting each crossed foe once.",
            ),
            // Fondue Fork — the outer two darts plant tines and string a wire.
            WeaponKind::CheeseSpray => e(
                super::loadout::PassiveKind::MaxHp,
                "Fondue Fork",
                "Dinner is a tripwire.",
                "The outer two darts plant tines; a cheese string between them cuts crossers for 1.5 s.",
            ),
            // Echo Chamber — spokes bounce off walls and come back.
            WeaponKind::SqueakNova => e(
                super::loadout::PassiveKind::FireRate,
                "Echo Chamber",
                "The walls know the chorus.",
                "Each spoke that hits a wall returns along its incoming line, hitting once more.",
            ),
            // Fairy Ring — every second cast leaves a still, burning ring.
            WeaponKind::SporeOrbit => e(
                super::loadout::PassiveKind::Regen,
                "Fairy Ring",
                "Something grew where you stood.",
                "Every second cast leaves its satellites fixed for 3 s; crossing the ring burns.",
            ),
            // Ratchet Tail — the whip hooks a body and swings it around.
            WeaponKind::TailWhip => e(
                super::loadout::PassiveKind::MoveSpeed,
                "Ratchet Tail",
                "Give the room a turn.",
                "A whip hit hooks the nearest body and swings it 90° into everything it crosses.",
            ),
            // Drain Cleaner — the splash becomes a directed corrosive channel.
            WeaponKind::AcidSpit => e(
                super::loadout::PassiveKind::Armor,
                "Drain Cleaner",
                "Clears hair, grease and witnesses.",
                "The splash runs 36 px forward as a corrosive channel, then lingers for 2 s.",
            ),
            // Flywheel — the thrash catches pellets and flings them back.
            WeaponKind::TailThrash => e(
                // TEMPORARY partner: `Counterweight` lands on a sibling branch.
                // See `loadout::FLYWHEEL_PARTNER` — flipping that one constant
                // finishes this row; the behaviour below never reads it.
                super::loadout::FLYWHEEL_PARTNER,
                "Flywheel",
                "Save that for the return trip.",
                "Each sweep catches up to two hostile pellets; the last sweep flings them forward.",
            ),
            // Roll Call — the squeak's tags answer with their own pulses.
            WeaponKind::SonicSqueak => e(
                super::loadout::PassiveKind::Magnet,
                "Roll Call",
                "Every cage answers.",
                "Foes the ring stuns or slows answer 0.8 s later with a 9 px pulse of their own.",
            ),
        }
    }
}

#[derive(Clone)]
pub struct Weapon {
    pub kind: WeaponKind,
    pub level: u8,
    pub timer: f32,
    pub evolved: bool,
}

impl Weapon {
    pub fn new(kind: WeaponKind) -> Self {
        Self {
            kind,
            level: 1,
            timer: 0.3,
            evolved: false,
        }
    }

    pub fn is_max(&self) -> bool {
        self.level >= self.kind.max_level()
    }

    /// The evolution this weapon wears, if it has one and is evolved.
    #[inline]
    pub fn evolution(&self) -> Option<Evolution> {
        if self.evolved {
            self.kind.evolution()
        } else {
            None
        }
    }

    /// Name to show for this weapon: the evolution's chest name once evolved
    /// (`"Milk Teeth"`), else [`WeaponKind::name`].
    pub fn name(&self) -> &'static str {
        self.evolution().map(|e| e.name).unwrap_or(self.kind.name())
    }

    /// Short mechanical line: the evolved attack's card text once evolved.
    pub fn desc(&self) -> &'static str {
        self.evolution().map(|e| e.attack).unwrap_or(self.kind.desc())
    }

    /// Flavour line: the chest inscription once evolved.
    pub fn flavor(&self) -> &'static str {
        self.evolution().map(|e| e.flavor).unwrap_or(self.kind.flavor())
    }

    /// Damage scales ~25% per level.
    pub fn damage(&self) -> f32 {
        self.kind.base_damage() * (1.0 + 0.25 * (self.level - 1) as f32)
    }

    /// Cooldown shrinks slightly with level (6% faster per level).
    ///
    /// No floor is needed: the factor only reaches 0.76 at `max_level()`, and
    /// `level` is capped there by the upgrade loop, so a clamp could never
    /// bind. (Clamping the *factor* as before looked like a safety net but was
    /// dead code — 1.0 - 0.06 * 4 = 0.76 never drops under 0.5.)
    pub fn cooldown(&self) -> f32 {
        self.kind.base_cooldown() * (1.0 - 0.06 * (self.level - 1) as f32)
    }

    /// Reach in px of an active weapon's effect: the thrash arc's radius or the
    /// squeak ring's final radius. Grows steeply with level, because reach is
    /// what an active weapon is bought for.
    ///
    /// Level 1 is already a screen-wide panic button (the viewport is about
    /// 160 x 168 px): 60 px of thrash is more than the drawn tail and a bit
    /// over a third of the screen, and 90 px of squeak rings most of it.
    pub fn active_reach(&self) -> f32 {
        match self.kind {
            WeaponKind::TailThrash => 60.0 + 6.0 * (self.level - 1) as f32,
            WeaponKind::SonicSqueak => 90.0 + 10.0 * (self.level - 1) as f32,
            _ => 0.0,
        }
    }

    /// How many times a Tail Thrash burst sweeps across its arc (see
    /// [`THRASH_SWEEPS`]).
    pub fn thrash_sweeps(&self) -> u32 {
        THRASH_SWEEPS
    }

    /// Extra projectiles granted by level (for multi-shot weapons).
    ///
    /// Gnaw is the one single-target weapon, so its level scaling was only raw
    /// damage and it fell out of every late build. From level 4 it fires two
    /// darts: keeping the starter weapon is now a real single-target choice
    /// (`weapons::fire_one` fires `1 + projs / 4` darts for it).
    pub fn level_projectiles(&self) -> i32 {
        match self.kind {
            WeaponKind::CheeseSpray => 2 + self.level as i32,
            WeaponKind::SqueakNova => 6 + 2 * self.level as i32,
            WeaponKind::SporeOrbit => 2 + self.level as i32,
            _ => self.level as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_shrinks_6_percent_per_level() {
        for kind in WeaponKind::all() {
            let base = kind.base_cooldown();
            let mut prev = f32::INFINITY;
            for level in 1..=kind.max_level() {
                let mut w = Weapon::new(kind);
                w.level = level;
                let expect = base * (1.0 - 0.06 * (level - 1) as f32);
                assert!(
                    (w.cooldown() - expect).abs() < 1e-6,
                    "{kind:?} L{level}: {} != {expect}",
                    w.cooldown()
                );
                assert!(w.cooldown() < prev, "{kind:?} L{level} must be faster");
                prev = w.cooldown();
            }
            // The removed clamp was unreachable: the factor bottoms out at
            // 0.76, well above the old 0.5 floor.
            let mut w = Weapon::new(kind);
            w.level = kind.max_level();
            assert!((w.cooldown() / base - 0.76).abs() < 1e-6);
            assert!(w.cooldown() > base * 0.5);
        }
    }

    /// The two active weapons are offered by the level-up pool like any other
    /// weapon, never auto-fire, and their reach/charge scale with level.
    #[test]
    fn active_weapons_are_marked_and_scale_with_level() {
        assert_eq!(WeaponKind::all().len(), 8);
        for kind in WeaponKind::all() {
            assert_eq!(
                kind.is_active(),
                matches!(kind, WeaponKind::TailThrash | WeaponKind::SonicSqueak),
                "{kind:?} is misclassified"
            );
        }
        for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
            assert!(
                kind.desc().starts_with("ACTIVE: press SPACE"),
                "{kind:?}'s card must teach the Space trigger: {}",
                kind.desc()
            );
            let mut prev_reach = 0.0;
            let mut prev_cd = f32::INFINITY;
            for level in 1..=kind.max_level() {
                let mut w = Weapon::new(kind);
                w.level = level;
                assert!(w.active_reach() > prev_reach, "{kind:?} L{level} reach");
                assert!(w.cooldown() < prev_cd, "{kind:?} L{level} charge time");
                prev_reach = w.active_reach();
                prev_cd = w.cooldown();
            }
            // A passive weapon has no reach at all.
            for passive in WeaponKind::all() {
                if !passive.is_active() {
                    let mut w = Weapon::new(passive);
                    w.level = 2;
                    assert_eq!(w.active_reach(), 0.0, "{passive:?} must have no reach");
                }
            }
        }
        // Level 1 numbers, quoted in the balance report.
        let mut thrash = Weapon::new(WeaponKind::TailThrash);
        assert_eq!(thrash.active_reach(), 60.0);
        assert_eq!(thrash.cooldown(), 10.0);
        assert_eq!(thrash.damage(), 26.0);
        assert_eq!(thrash.thrash_sweeps(), 3);
        let mut squeak = Weapon::new(WeaponKind::SonicSqueak);
        assert_eq!(squeak.active_reach(), 90.0);
        assert_eq!(squeak.cooldown(), 6.0);
        assert_eq!(squeak.damage(), 9.0);
        // …and at max level.
        thrash.level = WeaponKind::TailThrash.max_level();
        assert_eq!(thrash.active_reach(), 84.0);
        assert!((thrash.damage() - 52.0).abs() < 1e-4);
        squeak.level = WeaponKind::SonicSqueak.max_level();
        assert_eq!(squeak.active_reach(), 130.0);
        assert!((squeak.damage() - 18.0).abs() < 1e-4);
    }

    /// Every weapon in this branch has one chest row, with the partner the
    /// design table names, and `Weapon::name`/`desc`/`flavor` switch to it once
    /// the evolved flag is set (and only then).
    #[test]
    fn every_weapon_has_its_design_row() {
        use crate::game::loadout::PassiveKind;
        let rows = [
            (WeaponKind::Gnaw, PassiveKind::Damage, "Milk Teeth"),
            (WeaponKind::CheeseSpray, PassiveKind::MaxHp, "Fondue Fork"),
            (WeaponKind::SqueakNova, PassiveKind::FireRate, "Echo Chamber"),
            (WeaponKind::SporeOrbit, PassiveKind::Regen, "Fairy Ring"),
            (WeaponKind::TailWhip, PassiveKind::MoveSpeed, "Ratchet Tail"),
            (WeaponKind::AcidSpit, PassiveKind::Armor, "Drain Cleaner"),
            (WeaponKind::TailThrash, crate::game::loadout::FLYWHEEL_PARTNER, "Flywheel"),
            (WeaponKind::SonicSqueak, PassiveKind::Magnet, "Roll Call"),
        ];
        assert_eq!(rows.len(), WeaponKind::all().len());
        for (kind, passive, name) in rows {
            let evo = kind.evolution().unwrap_or_else(|| panic!("{kind:?} has no row"));
            assert_eq!(evo.passive, passive, "{kind:?} partner");
            assert_eq!(evo.name, name, "{kind:?} chest name");
            assert!(evo.flavor.ends_with('.'), "{kind:?} inscription: {}", evo.flavor);
            assert!(!evo.attack.is_empty());

            let mut w = Weapon::new(kind);
            assert_eq!(w.name(), kind.name());
            assert_eq!(w.desc(), kind.desc());
            assert_eq!(w.flavor(), kind.flavor());
            w.evolved = true;
            assert_eq!(w.name(), name);
            assert_eq!(w.desc(), evo.attack);
            assert_eq!(w.flavor(), evo.flavor);
            // The evolution changes behaviour, never the timing family.
            assert_eq!(w.cooldown(), Weapon::new(kind).cooldown());
            assert_eq!(w.is_max(), Weapon::new(kind).is_max());
        }
    }

    /// The temporary Flywheel partner is documented as temporary in one place,
    /// so the coordinator's flip cannot be missed.
    #[test]
    fn flywheel_partner_is_one_marked_constant() {
        assert_eq!(
            WeaponKind::TailThrash.evolution().unwrap().passive,
            crate::game::loadout::FLYWHEEL_PARTNER
        );
    }

    #[test]
    fn level_one_cooldown_is_the_base() {
        for kind in WeaponKind::all() {
            let w = Weapon::new(kind);
            assert_eq!(w.cooldown(), kind.base_cooldown());
            assert_eq!(w.damage(), kind.base_damage());
        }
    }
}
