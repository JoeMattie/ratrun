//! The thirteen weapons (`WeaponKind::all`): names, per-level stats, cooldowns
//! and evolutions. This module is data and rules only; the firing patterns
//! live in the weapon systems (`ecs/systems/weapons.rs`, `weapons_wave3a.rs`,
//! `weapons_wave3b.rs`, `weapon_spiketail.rs`, `evolutions.rs`), which can see
//! every entity.

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

/// Half-angle of the Sonic Squeak **cone** (batch 12: it is a forward wedge, not
/// a ring), in radians (38°). A body is inside the cone when its centre lies
/// within this angle of the cone's axis, with its own radius as slack.
pub const SQUEAK_CONE_HALF: f32 = 0.663_225_1;

/// Seconds the Sonic Squeak cone's chevron front takes to push out to its
/// reach (the damage lands on the press tick; this is the drawn push).
pub const SQUEAK_EXPAND: f32 = 0.30;
/// Stun applied to regular enemies the ring passes.
pub const SQUEAK_STUN: f32 = 1.5;
/// Elites are slowed instead of stunned (fraction of their speed).
pub const SQUEAK_ELITE_SLOW: f32 = 0.35;
/// The boss is slowed for a shorter window and less severely.
pub const SQUEAK_BOSS_SLOW: f32 = 0.5;
pub const SQUEAK_BOSS_SLOW_SECS: f32 = 0.8;
/// Outward shove (px/s of velocity) the cone gives everything it covers.
pub const SQUEAK_SHOVE: f32 = 70.0;

// ---- Squeak Nova: the expanding ring (batch 12) --------------------------
//
// The nova used to fire a ring of radial darts; it is now a true shockwave: a
// single ring that damages every body its front passes, once. The old spokes'
// flight was `life` 1.3 s at 95 px/s, so the ring's reach is that distance and
// a body the spokes could reach still takes one hit per cast — the same
// per-body dps.

/// Seconds the ring takes to reach [`NOVA_REACH`] (and, with **Echo Chamber**,
/// the same again to come back in).
pub const NOVA_EXPAND: f32 = 0.32;
/// The ring's final radius in px before Lens Cap (`Stats::ranged_reach`).
pub const NOVA_REACH: f32 = 123.5;

// ---- Tail Whip: the rear swipe -------------------------------------------

/// Half of the 160° swipe centred opposite the aim or facing.
pub const WHIP_ARC_HALF: f32 = 80.0 * std::f32::consts::PI / 180.0;

/// Reach of the swipe in px, L1 → L5, before Lens Cap (`Stats::melee_reach`).
/// Short on purpose: the swipe is the rat's near guard, and the Spiked Tail is
/// the tail that reaches.
pub const WHIP_REACH: [f32; 5] = [30.0, 33.0, 36.0, 39.0, 42.0];

/// Seconds the drawn crescent lives (`components::WhipCrack`).
pub const WHIP_SWIPE_SECS: f32 = 0.12;

// ---- Gnaw: the frontal bite (batch 12) ----------------------------------
//
// Gnaw was the starter dart; it is now a **melee bite** — no projectile, a
// snapping jaw in front of the rat's nose, very short reach, fast. It keeps the
// dart's single-target identity (it bites the nearest body in its cone) and its
// whole burst's damage: at L4+ the old two darts' worth lands as one bite.

/// Reach of the bite in px, L1 → L5 (before Lens Cap's melee reach hook).
///
/// The release pass: it was 20-24 px, and a rat that keeps moving was rarely
/// that close to what chased it (the Skilled bot landed 8 bites in the first
/// minute of level one, of 109 the cooldown allows). The damage per bite and
/// the cooldown are unchanged, so dps on an adjacent body is what it was.
pub const GNAW_BITE_REACH: [f32; 5] = [34.0, 35.5, 37.0, 38.5, 40.0];
/// Half-angle of the bite's jaw (48°), in radians.
pub const GNAW_BITE_HALF: f32 = 0.837_758;
/// Seconds the drawn snap lives (`components::BiteSnap`).
pub const BITE_SECS: f32 = 0.11;

// ---- Cheese Spray: the slowing wedges (batch 12) ------------------------
//
// The spray is a shotgun of **chunky wedges**: they fly a short way, slow down
// and drop, and everything the fan covers is slowed. The per-body damage per
// cycle is the darts' own (one wedge = one dart's damage); the reach is what
// shrank.

/// Wedge speed in px/s (the old dart flew at 110).
pub const CHEESE_WEDGE_SPEED: f32 = 78.0;
/// Wedge flight in seconds, before Lens Cap's ranged-reach hook. 78 x 0.55 is
/// about 43 px: a shotgun, not a rifle.
pub const CHEESE_WEDGE_LIFE: f32 = 0.55;
/// A body the fan covers is slowed to this fraction of its speed.
pub const CHEESE_SLOW_MULT: f32 = 0.55;
/// …for this many seconds.
pub const CHEESE_SLOW_SECS: f32 = 0.9;
/// Extra half-angle (10°) on either side of the wedge fan the slowing field
/// covers, so a body at the fan's edge — and one a wedge's fat body would have
/// clipped — is slowed with the rest.
pub const CHEESE_FAN_SLACK: f32 = 0.174_532_9;

// ---- Laser Eyes tuning (batch 13: the inward scissor) --------------------
//
// A burst is **two continuous beams closing on the aim line**: one starts at
// each outer edge of the arc (`aim - half`, `aim + half`) and both sweep
// inward, meeting on the aim direction when the sweep ends. From L4 they do not
// stop there: they cross and carry on to the opposite edges over the same
// sweep, so they move twice as fast and end swapped (an X-shaped scissor).
// Both beams share one per-sweep hit set, so a body the pair covers takes the
// whole sweep's damage exactly once — one old pulse's worth (`laser_hit_damage`).

/// Seconds one sweep lasts, L1 → L5 (batch 13: the sweep is faster, the cycle
/// is unchanged, so a cycle now spends more of itself not firing).
pub const LASER_SWEEP_SECS: [f32; 5] = [0.26, 0.28, 0.30, 0.32, 0.34];
/// Full arc one sweep pivots through, L1 → L5, in **degrees** (60° → 110°).
/// Stored in degrees because that is how the design row reads; `Weapon::laser_half_arc`
/// converts on the way out (radians), so the arc is never a hand-rounded literal.
pub const LASER_ARC_DEG: [f32; 5] = [60.0, 70.0, 80.0, 95.0, 110.0];
/// From this level the two beams **cross** the aim line and run on to the
/// opposite edges (the X). At every level they share one hit set, so a body is
/// still cut once per sweep.
pub const LASER_TWIN_LEVEL: u8 = 4;

/// The old per-pulse cadence, kept only because two render readers still scale
/// a light's brightness by it (`world::laser_visual`, `lighting::sources`).
/// Nothing in the simulation pulses any more: `LaserFx::since_pulse` stays 0, so
/// those readers see a steady (unblinking) beam.
pub const LASER_PULSE_INTERVAL: f32 = 0.10;
/// Reference half-arc for the render side's beam light pools (the sweep's real
/// half-arc per level is `LASER_ARC_DEG` / 2, carried live on `LaserFx::half`).
pub const LASER_SWEEP_HALF: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// The beam's half-width in px (the design's "3 px wide" bar).
pub const LASER_HALF_WIDTH: f32 = 1.5;

/// Number of weapon slots every per-weapon bookkeeping array needs
/// (`ecs::resources::DamageLedger`, the balance reports).
///
/// Must equal `WeaponKind::all().len()` (a test asserts it): the arrays are
/// indexed by [`WeaponKind::index`], so a new kind has to raise this and the
/// arrays in one place. Wave 3 added Laser Eyes, Super Dash, Acid Rain,
/// Hantavirus and the Spiked Tail.
pub const WEAPON_SLOTS: usize = 13;

// ---- Wave 3 weapons A: Acid Rain and Hantavirus --------------------------
//
// Per-level numbers are tables instead of the generic `1 + 0.25 * (level - 1)`
// scaling because `WAVE3_DESIGN.md` §4 fixes every one of them exactly (the
// generic arithmetic would land on 2.63 s / 3.38 s cycles and 3.75 damage per
// virus tick, not the designed 2.6 s / 3.4 s / 4). `Weapon::damage` and
// `Weapon::cooldown` read these tables when a kind has one and keep the old
// arithmetic for every other weapon.

/// Acid Rain: how far from the rat a foe may be marked, in px.
pub const RAIN_RANGE: f32 = 72.0;
/// Seconds from the shot to the first drop landing.
pub const RAIN_WINDUP: f32 = 0.35;
/// Seconds between successive drops of one volley.
pub const RAIN_STAGGER: f32 = 0.12;
/// Seconds a drop is drawn *falling* before it lands. The mark ring is on the
/// floor for the whole wind-up; inside this last window the three lime strokes
/// descend.
pub const RAIN_FALL: f32 = 0.16;
/// Per-splash damage, L1 → L5 (design §4).
pub const RAIN_DAMAGE: [f32; 5] = [8.0, 10.0, 12.0, 14.0, 16.0];
/// Cycle length, L1 → L5.
pub const RAIN_CYCLE: [f32; 5] = [2.8, 2.6, 2.4, 2.2, 2.0];
/// Drops per volley, L1 → L5.
pub const RAIN_DROPS: [u8; 5] = [2, 2, 3, 3, 4];
/// Splash radius (px), L1 → L5.
pub const RAIN_RADIUS: [f32; 5] = [7.0, 8.0, 8.0, 9.0, 10.0];

/// Hantavirus: how far from the rat a foe may be marked, in px.
///
/// **Historical** (the weapon no longer tags at range): the drop's trigger
/// radius is [`DROPPING_RADIUS`], and the Lens Cap scaling that used to stretch
/// this reach now stretches that radius. Kept as the documented size of the
/// old ranged tag so the wave-3 design doc still resolves.
#[allow(dead_code)]
pub const VIRUS_RANGE: f32 = 56.0;

/// Hantavirus: seconds a dropping stays live on the floor.
pub const DROPPING_LIFE: f32 = 14.0;
/// Hantavirus: live droppings the weapon keeps, L1 → L5 (dropping one more
/// removes the oldest, i.e. the lowest `SpawnId`, first).
pub const DROPPING_CAP: [u8; 5] = [4, 5, 6, 7, 8];
/// Hantavirus: a dropping's trigger radius in px. Lens Cap stretches it like
/// any other ranged reach (`Stats::ranged_reach`, relative to this).
pub const DROPPING_RADIUS: f32 = 7.0;
/// Hantavirus: seconds a fresh dropping cannot be stepped in, so the rat's own
/// footprint cannot trigger the drop it just left.
pub const DROPPING_ARM: f32 = 0.25;
/// Hantavirus: how far behind the rat's facing a dropping lands, in px (the
/// rat's centre, less its back). Inside a wall it lands under the rat instead.
pub const DROPPING_BACK: f32 = 5.0;
/// Damage ticks per host (three pips, one lost per tick).
pub const VIRUS_TICKS: u8 = 3;
/// Seconds between ticks.
pub const VIRUS_TICK: f32 = 0.35;
/// Per-tick damage, L1 → L5 (design §4).
pub const VIRUS_DAMAGE: [f32; 5] = [3.0, 4.0, 5.0, 6.0, 7.0];
/// Cycle length, L1 → L5.
pub const VIRUS_CYCLE: [f32; 5] = [3.6, 3.4, 3.2, 3.0, 2.8];
/// Transfers a chain may still make, L1 → L5 (a chain marks `jumps + 1`
/// bodies at most).
pub const VIRUS_JUMPS: [u8; 5] = [1, 2, 2, 3, 4];
/// Reach of a transfer, L1 → L5.
pub const VIRUS_REACH: [f32; 5] = [14.0, 14.0, 18.0, 18.0, 22.0];

// ---- The Spiked Tail Whip (wave 3, w3-spiketail) -------------------------
//
// The same table rule as the other wave-3 weapons: the design fixes the five
// numbers per row, so `Weapon::damage`/`cooldown` read these instead of the
// generic 25 %-per-level curve.
//
// The lash is a two-beat animation, and the sim runs on the same clock the art
// does (one set of constants, so the crack and the damage cannot drift apart):
//
// ```text
// fire ── LASH_WINDUP ── LASH_RECOIL_AT ────────── LASH_TOTAL ──> (despawn)
//        bristle         CRACK + burst   Barbed Wire drag, then recoil
// ```
/// Seconds from the shot to the crack: the tail thickens and sprouts spikes.
/// `Sfx::Lash` spends exactly this long on its rising swish before the crack.
pub const LASH_WINDUP: f32 = 0.12;
/// Seconds from the shot to the start of the recoil — where an evolved
/// **Barbed Wire** lash drags its catch home.
pub const LASH_RECOIL_AT: f32 = 0.20;
/// How long a lash entity lives (bristle, crack, a beat of extension, recoil).
pub const LASH_TOTAL: f32 = 0.34;
/// Per-crack damage, L1 → L5 (the primary body takes all of it).
pub const LASH_DAMAGE: [f32; 5] = [22.0, 27.0, 33.0, 38.0, 44.0];
/// Cycle length, L1 → L5.
pub const LASH_CYCLE: [f32; 5] = [1.3, 1.19, 1.08, 0.96, 0.85];
/// Reach of the lash (rat to the target's near edge), L1 → L5. Lens Cap
/// stretches it.
pub const LASH_REACH: [f32; 5] = [80.0, 90.0, 100.0, 110.0, 120.0];
/// Radius of the burst at the impact point, L1 → L5. Long Fuse widens it.
pub const LASH_RADIUS: [f32; 5] = [10.0, 11.5, 13.0, 14.5, 16.0];
/// Spare spike shards that break off the tail and fly on, L1 → L5 (design:
/// an optional flourish from level 4, two or three shards).
pub const LASH_SHARDS: [u8; 5] = [0, 0, 0, 2, 3];
/// What one shard is worth, as a fraction of the level's lash damage.
pub const LASH_SHARD_FRAC: f32 = 0.22;
/// The burst at the impact point is worth this much of the lash hit.
pub const LASH_BURST_FRAC: f32 = 0.4;
/// Seconds the primary body is staggered by the crack.
pub const LASH_STAGGER: f32 = 0.16;
/// Knockback velocity (px/s) the crack hands the primary body, before
/// Counterweight's `Stats::shove` multiplier (the lash has a native shove, so
/// Counterweight's +2 px nudge does not apply — its card says so).
pub const LASH_SHOVE: f32 = 120.0;
/// Heavy bodies are dragged and shoved this much less (the thrash's boss rule).
pub const LASH_HEAVY_SCALE: f32 = 0.15;
/// Screen shake on the crack frame.
pub const LASH_SHAKE: f32 = 2.4;
/// Where an evolved Barbed Wire drag drops its catch: this far from the rat,
/// so a dragged body lands at its feet instead of inside it.
pub const LASH_DRAG_GAP: f32 = 9.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WeaponKind {
    Gnaw,        // homing dart at nearest enemy
    CheeseSpray, // forward spread of darts
    SqueakNova,  // radial burst in all directions
    SporeOrbit,  // orbiting projectiles around the player
    TailWhip,    // rear melee swipe
    AcidSpit,    // lobbed shot that leaves a damaging pool
    TailThrash,  // ACTIVE: charged burst of sweeping tail sweeps
    SonicSqueak, // ACTIVE: charged expanding stunning ring
    LaserEyes,   // sweeping piercing beam, fired in a burst of pulses
    SuperDash,   // DASH: the dash itself is the attack (passive-triggered)
    AcidRain,    // marks floor spots, staggered drops, splash damage
    Hantavirus,  // leaves droppings; a body that steps in one is infected
    SpikedTail,  // single-target lash: spikes out, small burst at the hit
}

impl WeaponKind {
    pub fn all() -> [WeaponKind; WEAPON_SLOTS] {
        [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::SqueakNova,
            WeaponKind::SporeOrbit,
            WeaponKind::TailWhip,
            WeaponKind::AcidSpit,
            WeaponKind::TailThrash,
            WeaponKind::SonicSqueak,
            WeaponKind::LaserEyes,
            WeaponKind::SuperDash,
            WeaponKind::AcidRain,
            WeaponKind::Hantavirus,
            WeaponKind::SpikedTail,
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

    /// Is this weapon **not** fired by `weapons::fire_weapons_system`?
    ///
    /// Two wave-3 kinds drive themselves and are skipped by the auto-fire
    /// pass, which keeps their state (a burst, a dash) in
    /// `weapons_wave3b`:
    ///
    /// - [`WeaponKind::LaserEyes`] charges its own timer and resolves a burst
    ///   of pulses over the following ticks.
    /// - [`WeaponKind::SuperDash`] never fires on a timer at all: its "fire"
    ///   is a successful player dash (`systems::player::player_system` records
    ///   the travelled segment; `weapons_wave3b` damages what it crossed).
    ///
    /// Neither is [`WeaponKind::is_active`]: both are passive-triggered, so
    /// they have no charge gauge and the Space press does not spend them.
    pub fn fires_outside_fire_one(self) -> bool {
        matches!(self, WeaponKind::LaserEyes | WeaponKind::SuperDash)
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
            WeaponKind::LaserEyes => 8,
            WeaponKind::SuperDash => 9,
            WeaponKind::AcidRain => 10,
            WeaponKind::Hantavirus => 11,
            WeaponKind::SpikedTail => 12,
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
            WeaponKind::LaserEyes => "Laser Eyes",
            WeaponKind::SuperDash => "Super Dash",
            WeaponKind::AcidRain => "Acid Rain",
            WeaponKind::Hantavirus => "Hantavirus",
            WeaponKind::SpikedTail => "Spiked Tail",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Bites the nearest foe in front of you.",
            WeaponKind::CheeseSpray => "Blasts a short, slowing fan of wedges.",
            WeaponKind::SqueakNova => "A ring of force bursts out, cutting all it passes.",
            WeaponKind::SporeOrbit => "Orbiting spores shred nearby foes.",
            WeaponKind::TailWhip => "Swipes through foes behind you.",
            WeaponKind::AcidSpit => "Lobs acid that pools and burns.",
            // The "ACTIVE" prefix is what the level-up card shows, so the
            // player learns which weapons the Space bar fires (task: the first
            // two player-triggered weapons).
            WeaponKind::TailThrash => "ACTIVE: Space sweeps a shot-clearing arc.",
            WeaponKind::SonicSqueak => "ACTIVE: Space blasts a stunning cone ahead.",
            // The dash weapon's card teaches the trigger the same way: the
            // dash key, and (since the hold-to-charge pass) the hold itself.
            WeaponKind::LaserEyes => "Two beams close inward like scissors.",
            WeaponKind::SuperDash => "DASH: hold to charge, release to dash through foes.",
            WeaponKind::AcidRain => "Drops staggered acid splashes nearby.",
            WeaponKind::Hantavirus => "Leaves droppings. A step tags 3 hits, then jumps.",
            WeaponKind::SpikedTail => "Lashes one foe, then bursts at the hit.",
        }
    }

    pub fn flavor(self) -> &'static str {
        match self {
            WeaponKind::Gnaw => "Teeth are the original weapon.",
            WeaponKind::CheeseSpray => "Weaponized dairy. Don't ask.",
            WeaponKind::SqueakNova => "Pitched past hearing. It still hurts.",
            WeaponKind::SporeOrbit => "Fungal hitchhikers from Vat 9, now loyal.",
            WeaponKind::TailWhip => "Forty grams of pure indignation.",
            WeaponKind::AcidSpit => "Stomach contents, redirected.",
            WeaponKind::TailThrash => "The tail remembered every boot.",
            WeaponKind::SonicSqueak => "Past hearing, straight into the skull.",
            WeaponKind::LaserEyes => "Please look directly at the sensor.",
            WeaponKind::SuperDash => "Exit wound, entrance wound, exit.",
            WeaponKind::AcidRain => "There was a ceiling here once.",
            WeaponKind::Hantavirus => "Somebody passed a note in Block C.",
            WeaponKind::SpikedTail => "The tail grew a point of view.",
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
            WeaponKind::LaserEyes => (255, 196, 74),
            WeaponKind::SuperDash => (198, 230, 255),
            // Lime with more yellow in it than Acid Spit's bile green, so the
            // two acids read apart; plum is unused by any other weapon.
            WeaponKind::AcidRain => (198, 255, 72),
            WeaponKind::Hantavirus => (206, 106, 190),
            // Steel: the one weapon colour in the game with no hue in it, so a
            // barbed tail reads apart from the two rose/orange tail weapons.
            WeaponKind::SpikedTail => (206, 214, 226),
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
            // The burst cycle (design §4: 2.0 → 1.6 s across the levels).
            WeaponKind::LaserEyes => 2.0,
            // Super Dash has no timer of its own: its cycle *is* the dash's
            // recharge. The number is the nominal 2.0 s for the ledger/HUD.
            WeaponKind::SuperDash => 2.0,
            WeaponKind::AcidRain => RAIN_CYCLE[0],
            WeaponKind::Hantavirus => VIRUS_CYCLE[0],
            WeaponKind::SpikedTail => LASH_CYCLE[0],
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
            // Per pulse (design §4); see [`Weapon::damage`] for the table.
            WeaponKind::LaserEyes => LASER_DAMAGE[0],
            // Per crossed body, per dash.
            WeaponKind::SuperDash => DASH_DAMAGE[0],
            // Both wave-3 weapons carry their own per-level tables (design §4)
            // instead of the generic 25 %-per-level curve; these are the L1
            // numbers the tables start from.
            WeaponKind::AcidRain => RAIN_DAMAGE[0],
            WeaponKind::Hantavirus => VIRUS_DAMAGE[0],
            WeaponKind::SpikedTail => LASH_DAMAGE[0],
        }
    }

    /// Per-level damage table, for the wave-3 weapons whose design numbers
    /// already include level scaling ("Values in every five-entry row run
    /// L1 → L5"). `None` keeps the classic `base x (1 + 0.25 x level)` curve.
    pub fn damage_by_level(self) -> Option<[f32; 5]> {
        match self {
            WeaponKind::LaserEyes => Some(LASER_DAMAGE),
            WeaponKind::SuperDash => Some(DASH_DAMAGE),
            WeaponKind::AcidRain => Some(RAIN_DAMAGE),
            WeaponKind::Hantavirus => Some(VIRUS_DAMAGE),
            WeaponKind::SpikedTail => Some(LASH_DAMAGE),
            _ => None,
        }
    }

    /// Per-level cycle table (seconds). `None` uses the classic
    /// 6%-per-level cooldown shrink.
    pub fn cooldown_by_level(self) -> Option<[f32; 5]> {
        match self {
            WeaponKind::LaserEyes => Some(LASER_CYCLE),
            WeaponKind::AcidRain => Some(RAIN_CYCLE),
            WeaponKind::Hantavirus => Some(VIRUS_CYCLE),
            WeaponKind::SpikedTail => Some(LASH_CYCLE),
            _ => None,
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
    /// All twelve design rows are live (`WAVE3_DESIGN.md` §3): the eight the
    /// first evolution pass shipped, plus the four the wave-3 weapons brought
    /// with them (Downspout, Dead Letter, Carbon Copy, Ripcord), whose
    /// behaviours live in `game::ecs::systems::evolutions`.
    pub fn evolution(self) -> Option<Evolution> {
        let e = |passive, name, flavor, attack| Some(Evolution { passive, name, flavor, attack });
        match self {
            // Milk Teeth — the lead dart lodges, then comes home biting.
            WeaponKind::Gnaw => e(
                super::loadout::PassiveKind::Damage,
                "Milk Teeth",
                "They always come back in.",
                "Lead dart lodges 1s, then yanks back to you through every foe in line.",
            ),
            // Fondue Fork — the outer two darts plant tines and string a wire.
            WeaponKind::CheeseSpray => e(
                super::loadout::PassiveKind::MaxHp,
                "Fondue Fork",
                "Dinner is a tripwire.",
                "Outer darts plant tines strung with a 1.5s wire that cuts crossers.",
            ),
            // Echo Chamber — spokes bounce off walls and come back.
            WeaponKind::SqueakNova => e(
                super::loadout::PassiveKind::FireRate,
                "Echo Chamber",
                "The walls know the chorus.",
                "The ring strikes the walls and comes back through for a second hit.",
            ),
            // Fairy Ring — every second cast leaves a still, burning ring.
            WeaponKind::SporeOrbit => e(
                super::loadout::PassiveKind::Regen,
                "Fairy Ring",
                "Something grew where you stood.",
                "Every second cast leaves a fixed ring of spores that burns for 3s.",
            ),
            // Ratchet Tail — the whip hooks a body and throws it forward.
            WeaponKind::TailWhip => e(
                super::loadout::PassiveKind::MoveSpeed,
                "Ratchet Tail",
                "Redirected for further testing.",
                "Hooks the nearest light foe it hits and throws it forward.",
            ),
            // Drain Cleaner — the splash becomes a directed corrosive channel.
            WeaponKind::AcidSpit => e(
                super::loadout::PassiveKind::Armor,
                "Drain Cleaner",
                "Clears hair, grease and witnesses.",
                "Splash surges 36px forward as a corrosive channel that burns for 2s.",
            ),
            // Flywheel — the thrash catches pellets and flings them back.
            WeaponKind::TailThrash => e(
                // `loadout::FLYWHEEL_PARTNER` is Counterweight (design §3).
                super::loadout::FLYWHEEL_PARTNER,
                "Flywheel",
                "Save that for the return trip.",
                "Sweeps catch up to 2 enemy shots each; the final sweep flings them back.",
            ),
            // Roll Call — the squeak's tags answer with their own pulses.
            WeaponKind::SonicSqueak => e(
                super::loadout::PassiveKind::Magnet,
                "Roll Call",
                "Every cage answers.",
                "Stunned or slowed foes echo 0.8s later with a damaging pulse of their own.",
            ),
            // Downspout — the volley becomes one marching gutter of drops.
            WeaponKind::AcidRain => e(
                super::loadout::PassiveKind::LongFuse,
                "Downspout",
                "The leak has found the stairs.",
                "Drops march 10px per splash back toward you, leaving a 1s acid gutter.",
            ),
            // Dead Letter — a dying host seals its last jumps into a packet.
            WeaponKind::Hantavirus => e(
                super::loadout::PassiveKind::SecondScent,
                "Dead Letter",
                "Return to sender.",
                "Isolated hosts die leaving a 2s packet; the next foe near it restarts it.",
            ),
            // Carbon Copy — the previous sweep repeats itself backwards.
            WeaponKind::LaserEyes => e(
                super::loadout::PassiveKind::LensCap,
                "Carbon Copy",
                "The camera kept a negative.",
                "Records each beam sweep and replays it backward next burst at 50% damage.",
            ),
            // Ripcord — the dash's route stays taut and then snaps.
            WeaponKind::SuperDash => e(
                super::loadout::PassiveKind::SpringHeel,
                "Ripcord",
                "The tail was the fuse.",
                "Your dash trail stays taut for 0.6s, striking foes and yanking them 8px.",
            ),
            // Barbed Wire — the lash hooks, and the recoil reels the catch in.
            // Partner: Counterweight, the passive that is literally a weight
            // tied to the tip of a weapon. It is the one passive with **two**
            // rows (Flywheel is the other), so one stack opens both; a chest
            // still evolves one weapon at a time, earliest in rack order.
            WeaponKind::SpikedTail => e(
                super::loadout::PassiveKind::Counterweight,
                "Barbed Wire",
                "Whatever it catches comes home.",
                "The lash hooks; the recoil drags its catch home, bursting every body it crosses.",
            ),
        }
    }
}
/// Laser Eyes: damage per pulse, L1 → L5 (design §4).
const LASER_DAMAGE: [f32; 5] = [6.0, 7.0, 8.0, 9.0, 10.0];
/// Laser Eyes: seconds per burst cycle, L1 → L5.
const LASER_CYCLE: [f32; 5] = [2.0, 1.9, 1.8, 1.7, 1.6];
/// Laser Eyes: pulses per burst, L1 → L5.
const LASER_PULSES: [u32; 5] = [3, 3, 4, 4, 5];
/// Laser Eyes: beam length in px before the wall stop, L1 → L5, as the design
/// row reads it. Batch 13 stretches every rank by [`LASER_REACH_SCALE`];
/// `Weapon::laser_reach` is the only place that applies it.
const LASER_REACH: [f32; 5] = [34.0, 40.0, 40.0, 48.0, 54.0];
/// Laser Eyes: how much longer every rank's beam is than the original table
/// ("longer"): 54.4 / 64 / 64 / 76.8 / 86.4 px, before Lens Cap.
pub const LASER_REACH_SCALE: f32 = 1.6;
/// Super Dash: damage per crossed body, L1 → L5.
const DASH_DAMAGE: [f32; 5] = [14.0, 20.0, 26.0, 32.0, 38.0];
/// Super Dash: swept radius around the travelled segment, L1 → L5.
const DASH_RADIUS: [f32; 5] = [4.0, 4.0, 5.0, 5.0, 6.0];

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

    /// Index into a per-level table (`level` is 1-based and capped at 5).
    #[inline]
    fn tier(&self) -> usize {
        (self.level.clamp(1, 5) - 1) as usize
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

    /// Damage scales ~25% per level, except for the wave-3 weapons that ship
    /// an explicit per-level table (their design numbers already include the
    /// scaling).
    pub fn damage(&self) -> f32 {
        match self.kind.damage_by_level() {
            Some(table) => table[self.tier()],
            None => self.kind.base_damage() * (1.0 + 0.25 * self.level.saturating_sub(1) as f32),
        }
    }

    /// Cooldown shrinks slightly with level (6% faster per level).
    ///
    /// No floor is needed: the factor only reaches 0.76 at `max_level()`, and
    /// `level` is capped there by the upgrade loop, so a clamp could never
    /// bind. (Clamping the *factor* as before looked like a safety net but was
    /// dead code — 1.0 - 0.06 * 4 = 0.76 never drops under 0.5.)
    pub fn cooldown(&self) -> f32 {
        match self.kind.cooldown_by_level() {
            Some(table) => table[self.tier()],
            None => self.kind.base_cooldown() * (1.0 - 0.06 * self.level.saturating_sub(1) as f32),
        }
    }

    /// Laser Eyes: the old burst's pulses (3 / 3 / 4 / 4 / 5). A sweep no
    /// longer pulses; the count is kept as the size of the burst the sweep
    /// replaces (the Carbon Copy afterimage's spark count). The per-body hit is
    /// [`Weapon::laser_hit_damage`], so a body the beams cross takes one old
    /// pulse's damage per sweep and its dps is unchanged.
    pub fn laser_pulses(&self) -> u32 {
        match self.kind {
            WeaponKind::LaserEyes => LASER_PULSES[self.tier()],
            _ => 0,
        }
    }

    /// Laser Eyes: what one body the beam crosses takes per sweep: one old
    /// pulse's damage (`Weapon::damage()` is the per-pulse table). The old
    /// burst fanned its pulses at distinct angles, so a body normally took
    /// one; the sweep's gain is its coverage, not its per-body damage.
    #[inline]
    pub fn laser_hit_damage(&self) -> f32 {
        match self.kind {
            WeaponKind::LaserEyes => self.damage(),
            _ => 0.0,
        }
    }

    /// Laser Eyes: **half** the sweep's arc in radians (L1 30° → L5 55°).
    #[inline]
    pub fn laser_half_arc(&self) -> f32 {
        match self.kind {
            WeaponKind::LaserEyes => (LASER_ARC_DEG[self.tier()] * 0.5).to_radians(),
            _ => 0.0,
        }
    }

    /// Laser Eyes: seconds one sweep lasts (L1 0.26 → L5 0.34).
    #[inline]
    pub fn laser_sweep_secs(&self) -> f32 {
        match self.kind {
            WeaponKind::LaserEyes => LASER_SWEEP_SECS[self.tier()],
            _ => 0.0,
        }
    }

    /// Laser Eyes: do the two beams **cross** the aim line and swap edges at
    /// this level (L4+)? At L1–L3 they stop where they meet, on the aim line.
    #[inline]
    pub fn laser_twin(&self) -> bool {
        self.kind == WeaponKind::LaserEyes && self.level >= LASER_TWIN_LEVEL
    }

    /// Laser Eyes: beam length in px, before the wall stop and Lens Cap
    /// (`Stats::ranged_reach`). Batch 13's ×1.6 is applied here, so the design
    /// row in `LASER_REACH` stays the number it always was.
    pub fn laser_reach(&self) -> f32 {
        match self.kind {
            WeaponKind::LaserEyes => LASER_REACH[self.tier()] * LASER_REACH_SCALE,
            _ => 0.0,
        }
    }

    /// Super Dash: the swept radius around the travelled dash segment.
    pub fn dash_radius(&self) -> f32 {
        match self.kind {
            WeaponKind::SuperDash => DASH_RADIUS[self.tier()],
            _ => 0.0,
        }
    }

    /// Acid Rain drops per volley (design §4's "2 / 2 / 3 / 3 / 4").
    #[inline]
    pub fn rain_drops(&self) -> u8 {
        RAIN_DROPS[self.tier()]
    }

    /// Acid Rain splash radius in px.
    #[inline]
    pub fn rain_radius(&self) -> f32 {
        RAIN_RADIUS[self.tier()]
    }

    /// Hantavirus transfers the chain may still make from its first host.
    #[inline]
    pub fn virus_jumps(&self) -> u8 {
        VIRUS_JUMPS[self.tier()]
    }

    /// Hantavirus transfer reach in px.
    #[inline]
    pub fn virus_reach(&self) -> f32 {
        VIRUS_REACH[self.tier()]
    }

    /// Spiked Tail: reach of the lash in px, before Lens Cap
    /// (`Stats::melee_reach` — a tail is a melee weapon).
    #[inline]
    pub fn lash_reach(&self) -> f32 {
        match self.kind {
            WeaponKind::SpikedTail => LASH_REACH[self.tier()],
            _ => 0.0,
        }
    }

    /// Spiked Tail: radius of the burst at the impact point, before Long Fuse
    /// (`Stats::splash_radius`).
    #[inline]
    pub fn lash_radius(&self) -> f32 {
        match self.kind {
            WeaponKind::SpikedTail => LASH_RADIUS[self.tier()],
            _ => 0.0,
        }
    }

    /// Spiked Tail: spike shards that break off at the impact (0 until L4).
    #[inline]
    pub fn lash_shards(&self) -> u8 {
        match self.kind {
            WeaponKind::SpikedTail => LASH_SHARDS[self.tier()],
            _ => 0,
        }
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
            WeaponKind::TailThrash => 60.0 + 6.0 * self.level.saturating_sub(1) as f32,
            WeaponKind::SonicSqueak => 90.0 + 10.0 * self.level.saturating_sub(1) as f32,
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

    /// `WEAPON_SLOTS` is what every per-weapon bookkeeping array is sized to,
    /// and `index()` must agree with `all()`: a new kind has to update both.
    #[test]
    fn weapon_slots_match_the_kind_list() {
        assert_eq!(WeaponKind::all().len(), WEAPON_SLOTS);
        for (i, kind) in WeaponKind::all().iter().enumerate() {
            assert_eq!(kind.index(), i, "{kind:?} is out of index order");
        }
    }

    #[test]
    fn cooldown_shrinks_6_percent_per_level() {
        for kind in WeaponKind::all() {
            // Wave-3 weapons with an explicit per-level table are covered by
            // `wave3_weapons_use_their_design_tables` instead.
            if kind.cooldown_by_level().is_some() {
                continue;
            }
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
        assert_eq!(WeaponKind::all().len(), WEAPON_SLOTS);
        for kind in WeaponKind::all() {
            assert_eq!(
                kind.is_active(),
                matches!(kind, WeaponKind::TailThrash | WeaponKind::SonicSqueak),
                "{kind:?} is misclassified"
            );
        }
        for kind in [WeaponKind::TailThrash, WeaponKind::SonicSqueak] {
            assert!(
                kind.desc().starts_with("ACTIVE: Space"),
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

    /// The four numbers of every `WAVE3_DESIGN.md` §4 row this worker owns,
    /// per level, straight from the table in the doc.
    #[test]
    fn wave3_weapons_match_the_design_tables() {
        // Damage, cycle, count and reach per rank, L1 → L5.
        let expect = [
            (
                WeaponKind::AcidRain,
                [8.0, 10.0, 12.0, 14.0, 16.0],
                [2.8, 2.6, 2.4, 2.2, 2.0],
                [2.0, 2.0, 3.0, 3.0, 4.0],
                [7.0, 8.0, 8.0, 9.0, 10.0],
            ),
            (
                WeaponKind::Hantavirus,
                [3.0, 4.0, 5.0, 6.0, 7.0],
                [3.6, 3.4, 3.2, 3.0, 2.8],
                [1.0, 2.0, 2.0, 3.0, 4.0],
                [14.0, 14.0, 18.0, 18.0, 22.0],
            ),
        ];
        for (kind, dmg, cycle, count, reach) in expect {
            for level in 1..=5u8 {
                let mut w = Weapon::new(kind);
                w.level = level;
                let i = (level - 1) as usize;
                assert_eq!(w.damage(), dmg[i], "{kind:?} L{level} damage");
                assert_eq!(w.cooldown(), cycle[i], "{kind:?} L{level} cycle");
                let (got_count, got_reach) = match kind {
                    WeaponKind::AcidRain => (w.rain_drops() as f32, w.rain_radius()),
                    _ => (w.virus_jumps() as f32, w.virus_reach()),
                };
                assert_eq!(got_count, count[i], "{kind:?} L{level} count");
                assert_eq!(got_reach, reach[i], "{kind:?} L{level} reach");
            }
            // Level 1 is the base, so the generic `Weapon::new` path is intact.
            let w = Weapon::new(kind);
            assert_eq!(w.damage(), kind.base_damage());
            assert_eq!(w.cooldown(), kind.base_cooldown());
            // Neither is a player-triggered weapon, and neither has an active
            // reach: they auto-fire off a cooldown.
            assert!(!kind.is_active());
            assert_eq!(w.active_reach(), 0.0);
        }
        // A level past the cap must not index out of the table (the upgrade
        // loop caps levels, but a corrupt save should not panic the sim).
        let mut w = Weapon::new(WeaponKind::AcidRain);
        w.level = 9;
        assert_eq!(w.damage(), 16.0);
        assert_eq!(w.cooldown(), 2.0);
    }

    /// Every weapon with a live evolution has one chest row, with the partner the
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
            // The four wave-3 weapons' rows (their behaviours shipped with the
            // weapons; see `systems::evolutions`).
            (WeaponKind::AcidRain, PassiveKind::LongFuse, "Downspout"),
            (WeaponKind::Hantavirus, PassiveKind::SecondScent, "Dead Letter"),
            (WeaponKind::LaserEyes, PassiveKind::LensCap, "Carbon Copy"),
            (WeaponKind::SuperDash, PassiveKind::SpringHeel, "Ripcord"),
            // The spiked tail hooks with Counterweight (the weight on the tip).
            (WeaponKind::SpikedTail, PassiveKind::Counterweight, "Barbed Wire"),
        ];
        assert_eq!(rows.len(), WeaponKind::all().len(), "every kind needs a row");
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

    /// Flywheel's partner is Counterweight (design §3), through the one
    /// constant; Drain Cleaner keeps Thick Hide.
    #[test]
    fn flywheel_partner_is_counterweight() {
        use crate::game::loadout::PassiveKind;
        assert_eq!(crate::game::loadout::FLYWHEEL_PARTNER, PassiveKind::Counterweight);
        assert_eq!(
            WeaponKind::TailThrash.evolution().unwrap().passive,
            PassiveKind::Counterweight
        );
        assert_eq!(WeaponKind::AcidSpit.evolution().unwrap().passive, PassiveKind::Armor);
    }

    #[test]
    fn level_one_cooldown_is_the_base() {
        for kind in WeaponKind::all() {
            let w = Weapon::new(kind);
            assert_eq!(w.cooldown(), kind.base_cooldown());
            assert_eq!(w.damage(), kind.base_damage());
        }
    }

    /// Laser Eyes and Super Dash quote their levels directly in the design
    /// ("Values in every five-entry row run L1 → L5"), so they ship tables
    /// instead of the classic 25%-per-level damage curve.
    #[test]
    fn wave3_weapons_use_their_design_tables() {
        let at = |kind: WeaponKind, level: u8| {
            let mut w = Weapon::new(kind);
            w.level = level;
            w
        };
        for (level, (dmg, cycle, pulses, reach)) in [
            (1u8, (6.0f32, 2.0f32, 3u32, 54.4f32)),
            (2, (7.0, 1.9, 3, 64.0)),
            (3, (8.0, 1.8, 4, 64.0)),
            (4, (9.0, 1.7, 4, 76.8)),
            (5, (10.0, 1.6, 5, 86.4)),
        ] {
            let w = at(WeaponKind::LaserEyes, level);
            assert_eq!(w.damage(), dmg, "laser L{level} damage");
            assert_eq!(w.cooldown(), cycle, "laser L{level} cycle");
            assert_eq!(w.laser_pulses(), pulses, "laser L{level} pulses");
            // Batch 13: every rank's beam is ×1.6 the design row's length.
            assert!(
                (w.laser_reach() - reach).abs() < 1e-4,
                "laser L{level} reach"
            );
            assert!((w.laser_reach() - LASER_REACH[(level - 1) as usize] * 1.6).abs() < 1e-4);
        }
        for (level, (dmg, radius)) in [
            (1u8, (14.0f32, 4.0f32)),
            (2, (20.0, 4.0)),
            (3, (26.0, 5.0)),
            (4, (32.0, 5.0)),
            (5, (38.0, 6.0)),
        ] {
            let w = at(WeaponKind::SuperDash, level);
            assert_eq!(w.damage(), dmg, "dash L{level} damage");
            assert_eq!(w.dash_radius(), radius, "dash L{level} radius");
        }
    }

    /// The Spiked Tail's five-entry rows, straight from the design brief: it
    /// auto-fires (no charge gauge), its reach grows 80 → 120 px, its burst
    /// 10 → 16 px, its cycle 1.3 → 0.85 s, and the spike shards only exist
    /// from level 4.
    #[test]
    fn spiked_tail_matches_its_design_tables() {
        assert_eq!(WEAPON_SLOTS, 13);
        let expect = [
            (1u8, 22.0f32, 1.3f32, 80.0f32, 10.0f32, 0u8),
            (2, 27.0, 1.19, 90.0, 11.5, 0),
            (3, 33.0, 1.08, 100.0, 13.0, 0),
            (4, 38.0, 0.96, 110.0, 14.5, 2),
            (5, 44.0, 0.85, 120.0, 16.0, 3),
        ];
        for (level, dmg, cycle, reach, radius, shards) in expect {
            let mut w = Weapon::new(WeaponKind::SpikedTail);
            w.level = level;
            assert_eq!(w.damage(), dmg, "lash L{level} damage");
            assert_eq!(w.cooldown(), cycle, "lash L{level} cycle");
            assert_eq!(w.lash_reach(), reach, "lash L{level} reach");
            assert_eq!(w.lash_radius(), radius, "lash L{level} burst radius");
            assert_eq!(w.lash_shards(), shards, "lash L{level} shards");
        }
        // Level 1 is the base, and the classic generic paths still agree.
        let w = Weapon::new(WeaponKind::SpikedTail);
        assert_eq!(w.damage(), WeaponKind::SpikedTail.base_damage());
        assert_eq!(w.cooldown(), WeaponKind::SpikedTail.base_cooldown());
        // It is an ordinary auto-firing weapon: no charge gauge, no Space.
        assert!(!WeaponKind::SpikedTail.is_active());
        assert!(!WeaponKind::SpikedTail.fires_outside_fire_one());
        assert_eq!(w.active_reach(), 0.0);
        // A corrupt save past the cap must not index out of the table.
        let mut w = Weapon::new(WeaponKind::SpikedTail);
        w.level = 9;
        assert_eq!(w.damage(), 44.0);
        assert_eq!(w.lash_reach(), 120.0);
    }

    /// **Dps parity (batch 12)**: every weapon whose hit *shape* changed keeps
    /// the damage one body took per cycle. The numbers in `old` are what the
    /// pre-batch-12 build dealt a body standing in the weapon's reach, read off
    /// that build's tables:
    ///
    /// | weapon | per cycle, per body |
    /// |---|---|
    /// | Gnaw | one dart, or two from L4 (`level x (1 + 0.25 (level-1))`) |
    /// | Cheese Spray | one dart of the fan |
    /// | Squeak Nova | one spoke |
    /// | Tail Whip | one aura hit |
    /// | Sonic Squeak | one ring hit |
    /// | Laser Eyes | one pulse of the fanned burst |
    /// | Tail Thrash | three sweeps |
    ///
    /// The new build has to stay within 10 % of that, per level. What *did*
    /// change is the **area** a cycle covers (a bite instead of a dart, a line
    /// instead of an aura, a cone instead of a ring): that is the point of
    /// batch 12, and the dps a body standing in the region takes is untouched.
    #[test]
    fn the_reworked_weapons_keep_their_damage_per_cycle() {
        let at = |kind: WeaponKind, level: u8| {
            let mut w = Weapon::new(kind);
            w.level = level;
            w
        };
        // What one body took per cycle before batch 12, L1 → L5.
        let old: [(WeaponKind, [f32; 5]); 7] = [
            (WeaponKind::Gnaw, [9.0, 11.25, 13.5, 31.5, 36.0]),
            (WeaponKind::CheeseSpray, [7.0, 8.75, 10.5, 12.25, 14.0]),
            (WeaponKind::SqueakNova, [6.5, 8.125, 9.75, 11.375, 13.0]),
            (WeaponKind::TailWhip, [10.0, 12.5, 15.0, 17.5, 20.0]),
            (WeaponKind::SonicSqueak, [9.0, 11.25, 13.5, 15.75, 18.0]),
            (WeaponKind::LaserEyes, [6.0, 7.0, 8.0, 9.0, 10.0]),
            (WeaponKind::TailThrash, [78.0, 97.5, 117.0, 136.5, 156.0]),
        ];
        for (kind, want) in old {
            for level in 1..=5u8 {
                let w = at(kind, level);
                let now = match kind {
                    // A bite lands the whole volley (one dart, two from L4) on
                    // the one body it takes.
                    WeaponKind::Gnaw => w.damage() * (1 + (level / 4) as u32) as f32,
                    // One wedge, one ring pass, one crack, one cone hit, one
                    // sweep hit (which is the whole old burst).
                    WeaponKind::CheeseSpray
                    | WeaponKind::SqueakNova
                    | WeaponKind::TailWhip
                    | WeaponKind::SonicSqueak => w.damage(),
                    WeaponKind::LaserEyes => w.laser_hit_damage(),
                    // The thrash still sweeps three full discs.
                    WeaponKind::TailThrash => w.damage() * w.thrash_sweeps() as f32,
                    _ => unreachable!(),
                };
                let want = want[(level - 1) as usize];
                let ratio = now / want;
                assert!(
                    (ratio - 1.0).abs() <= 0.10,
                    "{kind:?} L{level}: {now} per cycle vs {want} before batch 12 \
                     ({}% off)",
                    (ratio - 1.0) * 100.0
                );
            }
        }
    }

    /// The laser's per-sweep hit is the whole old burst, and its arc/sweep table
    /// is batch 13's: the arc per rank is unchanged (60°..110°), the sweep is
    /// faster (0.26..0.34 s), both beams exist at every rank, and from L4 they
    /// cross to the opposite edges.
    #[test]
    fn laser_sweeps_match_the_design_row() {
        for (level, (arc_deg, secs, twin)) in [
            (1u8, (60.0f32, 0.26f32, false)),
            (2, (70.0, 0.28, false)),
            (3, (80.0, 0.30, false)),
            (4, (95.0, 0.32, true)),
            (5, (110.0, 0.34, true)),
        ] {
            let mut w = Weapon::new(WeaponKind::LaserEyes);
            w.level = level;
            assert!(
                (w.laser_half_arc() * 2.0 - arc_deg.to_radians()).abs() < 1e-4,
                "L{level} arc"
            );
            assert!((w.laser_sweep_secs() - secs).abs() < 1e-6, "L{level} sweep");
            assert_eq!(w.laser_twin(), twin, "L{level} crossing beams");
            // One body takes one old pulse's damage per sweep.
            assert!((w.laser_hit_damage() - w.damage()).abs() < 1e-6);
            // Both beams exist at every rank; the flag only says whether they
            // cross, which is what makes the pair move twice as fast.
            assert!((w.laser_reach() - LASER_REACH[(level - 1) as usize] * 1.6).abs() < 1e-4);
        }
    }

    /// Neither wave-3 weapon is a Space-triggered *active*: Super Dash must
    /// never read as one (it has no charge gauge), and both keep the
    /// auto-fire pass out of their state.
    #[test]
    fn wave3_weapons_are_not_active() {
        for kind in WeaponKind::all() {
            assert_eq!(
                kind.is_active(),
                matches!(kind, WeaponKind::TailThrash | WeaponKind::SonicSqueak),
                "{kind:?} is misclassified as active"
            );
            assert_eq!(
                kind.fires_outside_fire_one(),
                matches!(kind, WeaponKind::LaserEyes | WeaponKind::SuperDash),
                "{kind:?} is driven by the wrong system"
            );
        }
        assert!(!WeaponKind::SuperDash.is_active());
        assert!(!WeaponKind::LaserEyes.is_active());
    }
}
