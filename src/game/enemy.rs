//! Enemies: kinds, per-kind stats, and steering behavior toward the player.

use crate::math::Vec2;
use crate::render::palette::Rgb;
use rand::{Rng, RngExt};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnemyKind {
    Skitterer,
    Bat,
    Cat,
    Spitter,
    Brute,
    Boss,
    /// Wave 3: an armoured oval that curls to present its shell (`WAVE3_DESIGN.md` §9).
    Pillbug,
    /// Wave 3: a tuning fork that jumps out and back, landing twice.
    BellFlea,
    /// Wave 3: a moth whose death lights a fuse, then bursts.
    WickMoth,
    /// Wave 3: a beetle carrying a sugar cube it raises to heal the horde.
    Porter,
    /// Wave 3: a key-shaped drone that lays crossbars across its own path.
    Turnkey,
    /// Wave 4: a long multi-segment body that cuts the room in half and lunges.
    BigSnake,
    /// Wave 4: a very fast wall-hugging scuttler that darts and flees when hit.
    HouseCentipede,
    /// Wave 4: a flyer that hovers on a rising buzz, dives in a locked line and
    /// must recover on the ground.
    MurderHornet,
    /// Wave 4: a seething swarm with one HP pool, a per-hit damage cap, splits
    /// and regrowth.
    TermiteHorde,
    /// Wave 4: a tiny human squad member (spear, archer, rope crew, leader).
    Lilliputian,
}

impl EnemyKind {
    pub fn base_hp(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 6.0,
            EnemyKind::Bat => 4.0,
            EnemyKind::Cat => 18.0,
            EnemyKind::Spitter => 12.0,
            EnemyKind::Brute => 60.0,
            EnemyKind::Boss => 1400.0,
            // Wave 3 (`WAVE3_DESIGN.md` §9: base HP / speed / contact / XP).
            EnemyKind::Pillbug => 18.0,
            EnemyKind::BellFlea => 9.0,
            EnemyKind::WickMoth => 12.0,
            EnemyKind::Porter => 26.0,
            EnemyKind::Turnkey => 22.0,
            // Wave 4.
            EnemyKind::BigSnake => 80.0,
            EnemyKind::HouseCentipede => 7.0,
            EnemyKind::MurderHornet => 14.0,
            EnemyKind::TermiteHorde => 64.0,
            EnemyKind::Lilliputian => 3.0,
        }
    }
    pub fn speed(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 30.0,
            EnemyKind::Bat => 42.0,
            EnemyKind::Cat => 38.0,
            EnemyKind::Spitter => 22.0,
            EnemyKind::Brute => 16.0,
            EnemyKind::Boss => 20.0,
            EnemyKind::Pillbug => 23.0,
            EnemyKind::BellFlea => 32.0,
            EnemyKind::WickMoth => 28.0,
            EnemyKind::Porter => 20.0,
            EnemyKind::Turnkey => 24.0,
            // Wave 4. The centipede is the fastest body in the game; the thing
            // that keeps it fair is that it always stops for its flutter.
            EnemyKind::BigSnake => 26.0,
            EnemyKind::HouseCentipede => 54.0,
            EnemyKind::MurderHornet => 32.0,
            EnemyKind::TermiteHorde => 26.0,
            EnemyKind::Lilliputian => 22.0,
        }
    }
    /// Collision radius, which also picks the navigation size class
    /// ([`crate::game::collision::SizeClass::of_enemy`]): a body at or under
    /// `RAT_HOLE_MAX_RADIUS` (3.5 px) squeezes through a rat-hole, a larger one
    /// must take the long way round.
    ///
    /// Wave 3: the Bell Flea (2.0) and the Wick Moth (2.5) are small and follow
    /// the rat through holes; the Pillbug (5.0), Porter (4.5) and Turnkey (4.0)
    /// are large and are held to corridors and doorways.
    pub fn radius(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 2.5,
            EnemyKind::Bat => 2.0,
            EnemyKind::Cat => 4.0,
            EnemyKind::Spitter => 3.0,
            EnemyKind::Brute => 6.5,
            EnemyKind::Boss => 14.0,
            EnemyKind::Pillbug => 5.0,
            EnemyKind::BellFlea => 2.0,
            EnemyKind::WickMoth => 2.5,
            EnemyKind::Porter => 4.5,
            EnemyKind::Turnkey => 4.0,
            // Wave 4. The snake is the only Large one (it cannot use a
            // rat-hole); the centipede, hornet and horde are rat-sized.
            EnemyKind::BigSnake => 6.0,
            EnemyKind::HouseCentipede => 2.4,
            EnemyKind::MurderHornet => 2.6,
            EnemyKind::TermiteHorde => 3.2,
            EnemyKind::Lilliputian => 2.0,
        }
    }
    pub fn damage(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 5.0,
            EnemyKind::Bat => 4.0,
            EnemyKind::Cat => 9.0,
            EnemyKind::Spitter => 6.0,
            EnemyKind::Brute => 16.0,
            EnemyKind::Boss => 30.0,
            EnemyKind::Pillbug => 5.0,
            EnemyKind::BellFlea => 4.0,
            EnemyKind::WickMoth => 4.0,
            EnemyKind::Porter => 6.0,
            EnemyKind::Turnkey => 6.0,
            // Wave 4. The horde's contact is deliberately light: it is a
            // damage check, not a brawler.
            EnemyKind::BigSnake => 8.0,
            EnemyKind::HouseCentipede => 5.0,
            EnemyKind::MurderHornet => 6.0,
            EnemyKind::TermiteHorde => 3.0,
            EnemyKind::Lilliputian => 3.0,
        }
    }
    pub fn xp(self) -> u32 {
        match self {
            EnemyKind::Skitterer => 1,
            EnemyKind::Bat => 1,
            EnemyKind::Cat => 3,
            EnemyKind::Spitter => 4,
            EnemyKind::Brute => 10,
            EnemyKind::Boss => 200,
            EnemyKind::Pillbug => 3,
            EnemyKind::BellFlea => 2,
            EnemyKind::WickMoth => 3,
            EnemyKind::Porter => 5,
            EnemyKind::Turnkey => 4,
            // Wave 4.
            EnemyKind::BigSnake => 12,
            EnemyKind::HouseCentipede => 2,
            EnemyKind::MurderHornet => 4,
            EnemyKind::TermiteHorde => 3,
            EnemyKind::Lilliputian => 1,
        }
    }
    pub fn color(self) -> Rgb {
        match self {
            EnemyKind::Skitterer => (150, 120, 90),
            EnemyKind::Bat => (120, 90, 160),
            EnemyKind::Cat => (90, 90, 100),
            EnemyKind::Spitter => (120, 200, 90),
            EnemyKind::Brute => (170, 70, 60),
            EnemyKind::Boss => (220, 60, 120),
            // The wave-3 silhouettes read by colour as well as shape: silver
            // shell, copper fork, warm wick, pale sugar, brass key.
            EnemyKind::Pillbug => (186, 192, 206),
            EnemyKind::BellFlea => (214, 138, 78),
            EnemyKind::WickMoth => (232, 168, 104),
            EnemyKind::Porter => (140, 176, 128),
            EnemyKind::Turnkey => (206, 186, 102),
            // Wave 4: olive scales, pale chitin, a hot hazard stripe and a
            // termite cream.
            EnemyKind::BigSnake => (118, 146, 78),
            EnemyKind::HouseCentipede => (198, 176, 138),
            EnemyKind::MurderHornet => (214, 148, 40),
            EnemyKind::TermiteHorde => (206, 190, 150),
            // Facility staff, shrunk: the pale lab-coat grey-blue of the walls.
            EnemyKind::Lilliputian => (198, 206, 216),
        }
    }
    /// Spitters fire enemy bullets; others are melee.
    ///
    /// A wave-3 kind is never `ranged`: its damage arrives as a floor hazard the
    /// behaviour system resolves (`enemy_wave3`), not as a bullet.
    pub fn ranged(self) -> bool {
        matches!(self, EnemyKind::Spitter | EnemyKind::Boss)
    }

    /// One of the five kinds `WAVE3_DESIGN.md` §9 added.
    #[inline]
    pub fn is_wave3(self) -> bool {
        matches!(
            self,
            EnemyKind::Pillbug
                | EnemyKind::BellFlea
                | EnemyKind::WickMoth
                | EnemyKind::Porter
                | EnemyKind::Turnkey
        )
    }

    /// One of the four kinds the wave-4 batch added (`enemy_wave4`).
    #[inline]
    pub fn is_wave4(self) -> bool {
        matches!(
            self,
            EnemyKind::BigSnake
                | EnemyKind::HouseCentipede
                | EnemyKind::MurderHornet
                | EnemyKind::TermiteHorde
                | EnemyKind::Lilliputian
        )
    }

    /// A body that cannot be shoved around by an ordinary melee hit.
    ///
    /// Wave 4: the snake's mass and the horde's thousands of mouths are
    /// immovable for the same reason the Pillbug is — a `heavy()` body takes
    /// the whip hit but is never swung.
    #[inline]
    pub fn heavy(self) -> bool {
        matches!(
            self,
            EnemyKind::Brute
                | EnemyKind::Boss
                | EnemyKind::Pillbug
                | EnemyKind::BigSnake
                | EnemyKind::TermiteHorde
        )
    }

    /// Every kind, in enum order. The console's `spawn` command and its Tab
    /// completion read this table rather than a retyped list.
    pub fn all() -> [EnemyKind; 16] {
        [
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
            EnemyKind::Boss,
            EnemyKind::Pillbug,
            EnemyKind::BellFlea,
            EnemyKind::WickMoth,
            EnemyKind::Porter,
            EnemyKind::Turnkey,
            EnemyKind::BigSnake,
            EnemyKind::HouseCentipede,
            EnemyKind::MurderHornet,
            EnemyKind::TermiteHorde,
            EnemyKind::Lilliputian,
        ]
    }

    /// Lowercase display name used by the console and diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            EnemyKind::Skitterer => "skitterer",
            EnemyKind::Bat => "bat",
            EnemyKind::Cat => "cat",
            EnemyKind::Spitter => "spitter",
            EnemyKind::Brute => "brute",
            EnemyKind::Boss => "boss",
            EnemyKind::Pillbug => "pillbug",
            EnemyKind::BellFlea => "bellflea",
            EnemyKind::WickMoth => "wickmoth",
            EnemyKind::Porter => "porter",
            EnemyKind::Turnkey => "turnkey",
            EnemyKind::BigSnake => "bigsnake",
            EnemyKind::HouseCentipede => "housecentipede",
            EnemyKind::MurderHornet => "murderhornet",
            EnemyKind::TermiteHorde => "termitehorde",
            EnemyKind::Lilliputian => "lilliputian",
        }
    }
}

// ---- Wave-3 behaviour state ----------------------------------------------

/// Per-body scratch for the five wave-3 behaviours (`systems::enemy_wave3`).
///
/// The timers, ranges and damages themselves live in that module; this struct
/// is only the storage, so the state round-trips with the body (a test can
/// inspect or force it between ticks) and the fast path stays allocation-free.
/// Every field is zero/`ZERO` for the base six kinds, so their tick is
/// byte-for-byte unchanged and [`W3::holds`] is never true for them.
#[derive(Clone, Copy, Debug)]
pub struct W3 {
    /// 1 once the behaviour system has seeded this body's timers (a body can be
    /// spawned by the facade or a test as well as by the director, so the
    /// seeding happens on the first tick it is simulated).
    pub armed: u8,
    /// Seconds since the current warned tell started (0 = this tick).
    pub warn_age: f32,
    /// Pillbug: seconds left curled (0 = walking), and the cooldown after one.
    pub curl_t: f32,
    pub curl_cd: f32,
    /// Bell Flea: 0 walking, 1 winding up, 2 landed (about to spring back).
    pub flea_phase: u8,
    /// Seconds left in the current flea phase.
    pub flea_t: f32,
    /// The two locked landing points: where the hop goes and where it started.
    pub hop_to: Vec2,
    pub home: Vec2,
    /// Seconds until the next hop.
    pub flea_cd: f32,
    /// Wick Moth: > 0 the fuse is burning on the corpse (the behaviour system
    /// spawns the `Wick` entity on the tick it sees it), -1 = spent.
    pub wick_t: f32,
    /// Porter: seconds left raising the cube (0 = not raising).
    pub raise_t: f32,
    /// Seconds until the next raise.
    pub heal_cd: f32,
    /// `SpawnId` of the ally being raised for (`u64::MAX` = none).
    pub heal_target: u64,
    /// Turnkey: seconds until the next crossbar.
    pub bar_cd: f32,
    /// Seconds left of the current crossbar (tell first, then live).
    pub bar_t: f32,
    /// Where the crossbar was laid and which way it runs (world space).
    pub bar_pos: Vec2,
    pub bar_dir: Vec2,
    /// The crossbar has already taken its one bite out of the rat.
    pub bar_spent: bool,
}

impl Default for W3 {
    fn default() -> Self {
        W3 {
            armed: 0,
            warn_age: 0.0,
            curl_t: 0.0,
            curl_cd: 0.0,
            flea_phase: 0,
            flea_t: 0.0,
            hop_to: Vec2::ZERO,
            home: Vec2::ZERO,
            flea_cd: 0.0,
            wick_t: 0.0,
            raise_t: 0.0,
            heal_cd: 0.0,
            heal_target: u64::MAX,
            bar_cd: 0.0,
            bar_t: 0.0,
            bar_pos: Vec2::ZERO,
            bar_dir: Vec2::ZERO,
            bar_spent: false,
        }
    }
}

impl W3 {
    /// The turnkey's 0.8 s dotted tell (before the bar goes live).
    #[inline]
    pub fn bar_telling(&self) -> bool {
        self.bar_t > crate::game::ecs::systems::enemy_wave3::CROSSBAR_LIVE_SECS
    }

    /// The body is held in place this tick (its behaviour owns its position).
    #[inline]
    pub fn holds(&self) -> bool {
        self.curl_t > 0.0 || self.flea_phase != 0 || self.raise_t > 0.0
    }
}

// ---- Wave-4 behaviour state ----------------------------------------------

/// Body points the big snake draws and collides with, head first.
pub const SNAKE_MAX_SEGMENTS: usize = 7;

/// Per-body scratch for the four wave-4 behaviours (`systems::enemy_wave4`).
///
/// The same contract as [`W3`]: only storage. Every field is zero/`ZERO` for
/// every other kind, so a body's tick is unchanged and [`W4::holds`] is never
/// true for the ten older kinds. `phase`, `t`, `cd` and `dir` are shared
/// between the three state machines because only one of them ever runs on a
/// given body.
#[derive(Clone, Copy, Debug)]
pub struct W4 {
    /// 1 once `enemy_wave4` has seeded this body's timers.
    pub armed: u8,
    /// Seconds since the current warned tell started (0 = this tick).
    pub warn_age: f32,
    /// Kind-specific phase (see the constants in `enemy_wave4`).
    pub phase: u8,
    /// Seconds left in the current phase.
    pub t: f32,
    /// Seconds until the next warned attack.
    pub cd: f32,
    /// The locked attack direction (lunge, dart, dive).
    pub dir: Vec2,
    /// The snake's head-first body points (world space, `trail[0]` = head).
    pub trail: [Vec2; SNAKE_MAX_SEGMENTS],
    /// How far the current lunge has travelled (px).
    pub traveled: f32,
    /// One-bit latch for the current phase (the hornet's dive jab).
    pub spent: u8,
    /// How many of [`W4::trail`] are live (0 until the first simulated tick).
    pub trail_len: u8,
    /// Centipede: seconds of fleeing left after a hit.
    pub flee_t: f32,
    /// Horde: size tier (2 large, 1 medium, 0 small).
    pub tier: u8,
    /// Horde: seconds since the last damage (regrowth clock).
    pub regrow_t: f32,
    /// Horde: set by `Enemy::apply_damage` when a hard hit should split it.
    pub split_pending: u8,
    /// Horde: 1 once this body has already split (a body splits at most once).
    pub splits: u8,
    /// Horde: seconds until the next chewing crumb.
    pub chew_t: f32,
    /// Lilliputian: role (0 leader, 1 spear, 2 archer, 3 rope).
    pub role: u8,
    /// Lilliputian: place in the squad, so the spear line forms up in order.
    pub slot: u8,
    /// Lilliputian: the squad's id (its leader's `SpawnId`), so a squad can
    /// count itself for the panic rule and a panicked straggler can join
    /// another squad.
    pub squad: u32,
    /// Lilliputian: the largest this body's squad has ever been, so a squad
    /// that *started* small (a test, or a half squad) does not instantly break.
    pub squad_peak: u16,
}

impl Default for W4 {
    fn default() -> Self {
        W4 {
            armed: 0,
            warn_age: 0.0,
            phase: 0,
            t: 0.0,
            cd: 0.0,
            dir: Vec2::ZERO,
            trail: [Vec2::ZERO; SNAKE_MAX_SEGMENTS],
            traveled: 0.0,
            spent: 0,
            trail_len: 0,
            flee_t: 0.0,
            tier: 0,
            regrow_t: 0.0,
            split_pending: 0,
            splits: 0,
            chew_t: 0.0,
            role: 0,
            slot: 0,
            squad: 0,
            squad_peak: 0,
        }
    }
}

impl W4 {
    /// The body is held in place this tick (its behaviour owns its position).
    ///
    /// The snake and the hornet always move themselves (a slither or a hover
    /// is not a straight chase); the centipede only while it flutters or
    /// darts. The horde is an ordinary flow-field chaser.
    #[inline]
    pub fn holds(&self, kind: EnemyKind) -> bool {
        match kind {
            // Every wave-4 body with a state machine owns its own position for
            // the whole tick: the snake slithers S-curves, the hornet orbits,
            // the centipede scuttles along walls and the lilliputian walks in
            // formation. `enemy_wave4` reads the glue/ink components itself and
            // folds them into the speed (see `speed_mult` in that module), so
            // holding here does not make a body immune to a trap.
            EnemyKind::BigSnake
            | EnemyKind::HouseCentipede
            | EnemyKind::MurderHornet
            | EnemyKind::Lilliputian => true,
            _ => false,
        }
    }
}

/// How a campaign level's horde differs from the base kinds.
///
/// The base kinds ([`EnemyKind::damage`], [`EnemyKind::speed`]) are the Sewer's
/// and are owned by the enemy-stats tuning pass; a later level *multiplies*
/// them here instead of retuning them, so the two cannot fight each other.
/// [`crate::game::director::enemy_mods_for`] builds these per spawn and ramps
/// them over the level's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyMods {
    /// Multiplies [`EnemyKind::damage`] (what contact/bullets take off the rat).
    pub damage: f32,
    /// Multiplies [`EnemyKind::speed`] (the Lab's swarm is quicker than the
    /// Sewer's).
    pub speed: f32,
}

impl EnemyMods {
    /// The base kinds, unchanged: campaign level 1.
    pub const NONE: EnemyMods = EnemyMods {
        damage: 1.0,
        speed: 1.0,
    };
}

impl Default for EnemyMods {
    fn default() -> Self {
        EnemyMods::NONE
    }
}

#[derive(Clone)]
pub struct Enemy {
    /// Monotonic runtime identifier assigned on spawn (0 if constructed outside `World`).
    pub id: u32,
    pub kind: EnemyKind,
    /// True for periodic elite Brutes spawned by the director.
    pub elite: bool,
    /// A named miniboss body (`systems::miniboss`): it also carries the
    /// `Miniboss` component, but the flag lets the many `&Enemy` readers (the
    /// decoy lure, the Porter's casebook, the flatbed's pits) recognise it
    /// without another query. False for every director-spawned body.
    pub named: bool,
    pub pos: Vec2,
    pub vel: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub radius: f32,
    pub damage: f32,
    pub flash: f32,
    pub fire_cd: f32,
    pub wander: f32,
    pub contact_cd: f32,
    /// Campaign-level speed multiplier ([`EnemyMods::speed`]); 1.0 for the base
    /// kinds. Stored on the body because [`Enemy::steer`] reads it every tick.
    pub speed_scale: f32,
    /// Gait/wing animation phase, advanced by movement.
    pub anim: f32,
    /// Smoothed facing direction for sprite orientation.
    pub facing: Vec2,
    /// Seconds of hard stun left (0 = free). A stunned body does not steer, its
    /// velocity bleeds off and it cannot fire (the Sonic Squeak ring).
    pub stun: f32,
    /// Movement multiplier while `slow_t > 0` (1.0 = unaffected). Elites and the
    /// boss are slowed instead of stunned.
    pub slow_mult: f32,
    /// Seconds of slow left.
    pub slow_t: f32,
    /// Wave-3 behaviour state (all zero for the six base kinds).
    pub w3: W3,
    /// Wave-4 behaviour state (all zero for the ten older kinds).
    pub w4: W4,
}

impl Enemy {
    pub fn spawn(kind: EnemyKind, pos: Vec2, hp_mult: f32, rng: &mut impl Rng) -> Self {
        let hp = kind.base_hp() * hp_mult;
        Enemy {
            id: 0,
            kind,
            elite: false,
            named: false,
            pos,
            vel: Vec2::ZERO,
            hp,
            max_hp: hp,
            radius: kind.radius(),
            damage: kind.damage(),
            flash: 0.0,
            fire_cd: rng.random_range(1.0..2.5),
            wander: rng.random_range(0.0..std::f32::consts::TAU),
            contact_cd: 0.0,
            speed_scale: 1.0,
            anim: rng.random_range(0.0..std::f32::consts::TAU),
            facing: Vec2::new(1.0, 0.0),
            // No extra rng draws: a freshly spawned body is neither stunned nor
            // slowed, so adding these fields cannot move the rng stream.
            stun: 0.0,
            slow_mult: 1.0,
            slow_t: 0.0,
            // Likewise zero-initialised: the wave-3 timers are seeded by the
            // behaviour system on the body's first simulated tick, never from
            // the rng.
            w3: W3::default(),
            // Same rule for wave 4 (the snake's trail is seeded from its spawn
            // point on the first simulated tick).
            w4: W4::default(),
        }
    }

    /// Decay the crowd-control timers and stop a stunned body dead.
    ///
    /// Returns `true` while the body is stunned (the caller must skip
    /// steering). At the defaults (`stun == 0`, `slow_t == 0`) this only
    /// performs the two timer comparisons, so an ordinary enemy's tick is
    /// unchanged.
    fn tick_control(&mut self, dt: f32) -> bool {
        if self.stun > 0.0 {
            self.stun = (self.stun - dt).max(0.0);
            self.vel = self.vel * (1.0 - (12.0 * dt).min(1.0));
            self.pos += self.vel * dt;
            if self.flash > 0.0 {
                self.flash -= dt;
            }
            if self.contact_cd > 0.0 {
                self.contact_cd -= dt;
            }
            return true;
        }
        if self.slow_t > 0.0 {
            self.slow_t = (self.slow_t - dt).max(0.0);
            if self.slow_t <= 0.0 {
                self.slow_mult = 1.0;
            }
        }
        false
    }

    /// Stun the body for `secs` (keeps the longer of the two).
    #[inline]
    pub fn stun_for(&mut self, secs: f32) {
        if secs > 0.0 {
            self.stun = self.stun.max(secs);
        }
    }

    /// Slow the body to `mult` of its speed for `secs` (keeps the stronger of
    /// the two effects; the slow lifts when its timer runs out).
    #[inline]
    pub fn slow_for(&mut self, mult: f32, secs: f32) {
        if secs > 0.0 {
            let mult = mult.clamp(0.05, 1.0);
            if self.slow_t <= 0.0 || mult < self.slow_mult {
                self.slow_mult = mult;
            }
            self.slow_t = self.slow_t.max(secs);
        }
    }

    /// Clear every crowd-control effect (used by tests and by a boss's death
    /// break, so nothing is left frozen after the horde flees).
    #[inline]
    pub fn clear_control(&mut self) {
        self.stun = 0.0;
        self.slow_mult = 1.0;
        self.slow_t = 0.0;
    }

    /// [`Enemy::spawn`] with a campaign level's [`EnemyMods`] applied.
    ///
    /// Draws exactly the same rng values as [`Enemy::spawn`] (it delegates to
    /// it), so a later level changes numbers, never the rng stream.
    pub fn spawn_with_mods(
        kind: EnemyKind,
        pos: Vec2,
        hp_mult: f32,
        mods: EnemyMods,
        rng: &mut impl Rng,
    ) -> Self {
        let mut e = Enemy::spawn(kind, pos, hp_mult, rng);
        e.damage = kind.damage() * mods.damage;
        e.speed_scale = mods.speed;
        e
    }

    /// Steer toward the player, routing around walls via the flow field.
    /// `flow_dir` is the pathfinding direction at this enemy's cell (ZERO
    /// inside the goal cell / off-grid, in which case we head straight at the
    /// player).
    pub fn steer(&mut self, target: Vec2, flow_dir: Vec2, dt: f32, speed_mult: f32) {
        // A wave-3 body in the middle of its own tell or hop is held in place:
        // its behaviour owns the position this tick (a curled Pillbug, a Bell
        // Flea between its two crosses, a Porter raising the cube). Wave 4's
        // snake, hornet and darting centipede are the same. The timers, the
        // flash and the contact grace still advance, so the body keeps taking
        // damage and its own state machine keeps running. Dead cheap for the
        // older kinds: both `holds()` are false.
        if self.w3.holds() || self.w4.holds(self.kind) {
            self.vel = self.vel * (1.0 - (12.0 * dt).min(1.0));
            if self.flash > 0.0 {
                self.flash -= dt;
            }
            if self.contact_cd > 0.0 {
                self.contact_cd -= dt;
            }
            return;
        }

        // Crowd control (the Sonic Squeak's ring): a stunned body does not steer
        // and its momentum bleeds off fast, so it stops dead instead of sliding
        // past the rat. The slow is a straight multiplier on the per-kind speed
        // below. Both are no-ops at their defaults (0 / 1.0), so the existing
        // simulation is byte-for-byte unchanged.
        if self.tick_control(dt) {
            return;
        }

        let to = target - self.pos;
        let dist = to.len();
        let direct = to.normalized();
        // Follow the route whenever the field has an opinion. The field is
        // only ZERO inside the player's own cell (and off-grid), i.e. exactly
        // where the straight line cannot cross a wall, so the un-gridded last
        // stretch costs nothing in look: the velocity smoothing below hides
        // the grid. Taking the direct line within 26 px instead used to drag
        // enemies into props, where wall resolution parked them for good.
        let chase = if flow_dir.len_sq() > 0.001 {
            flow_dir
        } else {
            direct
        };
        let speed = self.kind.speed() * self.speed_scale * speed_mult * self.slow_mult;
        let desired = match self.kind {
            EnemyKind::Bat => {
                self.wander += dt * 4.0;
                (chase + Vec2::from_angle(self.wander) * 0.6).normalized() * speed
            }
            EnemyKind::Spitter => {
                // Keep a stand-off range: approach via the route, retreat directly.
                if dist > 110.0 {
                    chase * speed
                } else if dist < 80.0 {
                    direct * (-speed)
                } else {
                    chase.perp() * speed * 0.5
                }
            }
            _ => chase * speed,
        };
        // Smooth toward desired velocity.
        self.vel += (desired - self.vel) * (6.0 * dt).min(1.0);
        self.pos += self.vel * dt;

        // Animation + facing.
        let sp = self.vel.len();
        // Flyers buzz constantly; crawlers scuttle in proportion to speed.
        let cadence = if matches!(self.kind, EnemyKind::Bat) {
            22.0
        } else {
            4.0 + sp * 0.5
        };
        self.anim += dt * cadence;
        if sp > 1.0 {
            self.facing += (self.vel.normalized() - self.facing) * (8.0 * dt).min(1.0);
        }

        if self.flash > 0.0 {
            self.flash -= dt;
        }
        if self.contact_cd > 0.0 {
            self.contact_cd -= dt;
        }
    }

    pub fn hurt(&mut self, dmg: f32) {
        self.apply_damage(dmg);
    }

    /// Land `dmg` after the kind's own incoming-damage rule and return what was
    /// actually taken (so `hurt_from`'s caller can draw and record the truth).
    ///
    /// The Termite Horde is the one body with a rule of its own
    /// ([`crate::game::ecs::systems::enemy_wave4::HORDE_HIT_CAP`]): one hit can
    /// remove at most a capped fraction of its pool, so a single-target dart
    /// only chips the swarm while a many-hit area volley (`rain`, a nova, a
    /// thrash sweep, a pool's continuous ticks) still tears through it. The
    /// same pass spots a hard enough hit and raises the split flag the wave-4
    /// system turns into a second, smaller horde.
    ///
    /// Every other kind is a plain `hp -= dmg` with the historical flash and
    /// side effects, so nothing about the older bodies changes.
    fn apply_damage(&mut self, dmg: f32) -> f32 {
        use crate::game::ecs::systems::enemy_wave4 as w4;
        let mut dmg = dmg.max(0.0);
        if self.kind == EnemyKind::TermiteHorde {
            if self.hp > 0.0
                && self.w4.tier >= w4::HORDE_SPLIT_TIER
                && dmg >= self.max_hp * w4::HORDE_SPLIT_FRACTION
            {
                self.w4.split_pending = 1;
            }
            dmg = dmg.min(self.max_hp * w4::HORDE_HIT_CAP);
        }
        self.hp -= dmg;
        self.flash = 0.12;
        self.note_damage();
        dmg
    }

    /// `hurt` for a hit that arrives **from** a point in world space, with the
    /// kind's directional armour applied ([`Enemy::damage_mult_from`]). Returns
    /// the damage actually taken, which is what a caller should draw and record.
    pub fn hurt_from(&mut self, dmg: f32, from: Vec2) -> f32 {
        self.apply_damage(dmg * self.damage_mult_from(from))
    }

    /// The Pillbug's shell: while it is curled, a hit arriving from its frontal
    /// 120° arc is quartered and anything else (the exposed rear) doubles.
    ///
    /// Every damage source in the game routes through here — bullets, pools, the
    /// melee arc, the actives, traps, boss hazards and the nuke — so the shell
    /// cannot be bypassed by picking a different weapon. The arc is measured off
    /// [`Enemy::facing`], which the curl locks when the tell starts.
    #[inline]
    pub fn damage_mult_from(&self, from: Vec2) -> f32 {
        use crate::game::ecs::systems::enemy_wave4 as w4;
        match self.kind {
            EnemyKind::Pillbug => {
                if self.w3.curl_t <= 0.0 {
                    return 1.0;
                }
                let to_hit = from - self.pos;
                if to_hit.len_sq() <= 1e-6 {
                    return 1.0;
                }
                let f = self.facing;
                if f.len_sq() <= 1e-6 {
                    return 1.0;
                }
                // cos(half of 120°) = 0.5, so `dot >= 0.5` is the shell's cone.
                if to_hit.normalized().dot(f.normalized()) >= 0.5 {
                    crate::game::ecs::systems::enemy_wave3::PILLBUG_FRONT_MULT
                } else {
                    crate::game::ecs::systems::enemy_wave3::PILLBUG_REAR_MULT
                }
            }
            // The snake's head is the soft part: every hit on it lands harder
            // than the same hit on the scale-armoured body.
            EnemyKind::BigSnake => {
                if from.dist_sq(self.pos) <= w4::SNAKE_HEAD_SWEET_SPOT * w4::SNAKE_HEAD_SWEET_SPOT {
                    w4::SNAKE_HEAD_MULT
                } else {
                    1.0
                }
            }
            // The spear line: a wall of tiny shields and levelled points in
            // front, nothing at all behind.
            EnemyKind::Lilliputian => {
                let to_hit = from - self.pos;
                if to_hit.len_sq() <= 1e-6 || self.facing.len_sq() <= 1e-6 {
                    return 1.0;
                }
                if to_hit.normalized().dot(self.facing.normalized()) >= 0.4 {
                    w4::LILLI_FRONT_MULT
                } else {
                    w4::LILLI_REAR_MULT
                }
            }
            // The hornet on the ground after a missed dive is the punish
            // window, and it says so with a number.
            EnemyKind::MurderHornet
                if self.w4.phase == w4::HORNET_PHASE_RECOVER => {
                    w4::HORNET_RECOVER_MULT
                }
            _ => 1.0,
        }
    }

    /// Book a damaging hit's side effects without touching the flash tint.
    ///
    /// Used by the damage-over-time path (acid pools keep their own softer
    /// flash) and by the nuke, so a Porter still flinches and a Wick Moth still
    /// arms its fuse however it died. Draws no rng.
    pub fn note_damage(&mut self) {
        // The Porter drops its raise and has to try again (a flinch window, so
        // one stray shot does not disable it for its whole 5 s cycle).
        if self.kind == EnemyKind::Porter && self.w3.raise_t > 0.0 {
            self.w3.raise_t = 0.0;
            self.w3.heal_target = u64::MAX;
            self.w3.heal_cd = self.w3.heal_cd.max(
                crate::game::ecs::systems::enemy_wave3::PORTER_INTERRUPT_COOLDOWN,
            );
        }
        // A Wick Moth that dies lights its own fuse, whatever killed it.
        if self.kind == EnemyKind::WickMoth && self.hp <= 0.0 && self.w3.wick_t == 0.0 {
            self.w3.wick_t = crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS;
        }
        // A centipede bolts for the nearest wall the moment anything touches
        // it: that is the whole "hard to pin down" clause, and it is booked
        // here so every damage source (a pool tick, a boss hazard, a trap) makes
        // it run, not just bullets.
        if self.kind == EnemyKind::HouseCentipede && self.hp > 0.0 {
            self.w4.flee_t = self.w4.flee_t.max(
                crate::game::ecs::systems::enemy_wave4::CENTIPEDE_FLEE_SECS,
            );
        }
        // Any hit on the horde restarts its regrowth clock.
        if self.kind == EnemyKind::TermiteHorde {
            self.w4.regrow_t = 0.0;
        }
        // A hit on a rope carrier drops its rope: killing the crew is one of
        // the two ways out of a knot (the other is the dash).
        if self.kind == EnemyKind::Lilliputian
            && self.w4.role == crate::game::ecs::systems::enemy_wave4::LILLI_ROLE_ROPE
            && (self.w4.phase == crate::game::ecs::systems::enemy_wave4::LILLI_PHASE_TELL
                || self.w4.phase == crate::game::ecs::systems::enemy_wave4::LILLI_PHASE_ACTION)
        {
            self.w4.phase = crate::game::ecs::systems::enemy_wave4::LILLI_PHASE_WALK;
            self.w4.t = 0.0;
        }
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision;
    use crate::game::level::{Level, Rectf, Theme};
    use crate::game::nav::{test_tiles, FlowField, NavBudget};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn direct_chase_needs_an_empty_flow() {
        let mut rng = StdRng::seed_from_u64(7);
        let target = Vec2::new(20.0, 0.0); // well inside the old 26 px shortcut
        let route = Vec2::new(0.0, 1.0); // the field says "go around"
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        e.steer(target, route, 0.2, 1.0);
        assert!(
            e.vel.y > 5.0 && e.vel.x.abs() < 1.0,
            "must keep following the route on the last stretch: {:?}",
            e.vel
        );

        // With no flow (inside the goal cell) it heads straight in.
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        e.steer(target, Vec2::ZERO, 0.2, 1.0);
        assert!(
            e.vel.x > 5.0 && e.vel.y.abs() < 1.0,
            "an empty flow means the straight line is clear: {:?}",
            e.vel
        );
    }

    #[test]
    fn enemy_behind_a_prop_is_not_dragged_into_it() {
        // Player just right of a barrier, enemy just left of it: the straight
        // line crosses the prop, the flow must bend around it.
        let wall = Rectf::new(96.0, 32.0, 16.0, 80.0);
        let tiles = test_tiles(20, 20, &[wall]);
        let mut f = FlowField::new(&tiles);
        let player = Vec2::new(122.0, 73.0);
        f.update(&tiles, player, NavBudget::FULL);
        let mut rng = StdRng::seed_from_u64(3);
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::new(94.0, 73.0), 1.0, &mut rng);
        assert!(
            e.pos.dist(player) <= 30.0,
            "the case under test is a short hop"
        );
        let flow = f.dir_at(collision::SizeClass::Small, e.pos);
        assert!(flow.len() > 0.5, "the flow should route around the prop");
        let before = e.pos.dist(player);
        // The wall is five cells tall, so the detour around it is longer than
        // half a second of a skitterer's life: give it five.
        for _ in 0..300 {
            let flow = f.dir_at(collision::SizeClass::Small, e.pos);
            e.steer(player, flow, 1.0 / 60.0, 1.0);
            e.pos = collision::resolve_circle_rect(e.pos, e.radius, &wall);
        }
        // It must not have been pushed into the prop (which would be the
        // straight-line tell) and must not have lost ground.
        assert!(
            !wall.contains(e.pos),
            "enemy ended inside the prop at {:?}",
            e.pos
        );
        assert!(e.pos.dist(player) <= before, "enemy stalled at {:?}", e.pos);
    }

    #[test]
    fn skitterers_parked_in_a_prop_halo_reach_the_player() {
        // The reported stall: 20 skitterers tucked behind a 16x66 barrier.
        let wall = Rectf::new(96.0, 32.0, 16.0, 80.0);
        let walls = [wall];
        let tiles = test_tiles(20, 20, &walls);
        let mut f = FlowField::new(&tiles);
        let player = Vec2::new(200.0, 73.0);
        f.update(&tiles, player, NavBudget::FULL);
        let mut rng = StdRng::seed_from_u64(11);
        let mut enemies: Vec<Enemy> = (0..20)
            .map(|i| {
                let y = 46.0 + i as f32 * 3.0;
                Enemy::spawn(EnemyKind::Skitterer, Vec2::new(94.0, y), 1.0, &mut rng)
            })
            .collect();
        let dt = 1.0 / 60.0;
        for _ in 0..20 * 60 {
            for e in enemies.iter_mut() {
                let flow = f.dir_at(collision::SizeClass::Small, e.pos);
                e.steer(player, flow, dt, 1.0);
                e.pos = e.pos.clamp_box(Vec2::splat(2.0), Vec2::new(318.0, 318.0));
                for w in &walls {
                    e.pos = collision::resolve_circle_rect(e.pos, e.radius, w);
                }
            }
        }
        let stuck: Vec<Vec2> = enemies
            .iter()
            .filter(|e| e.pos.dist(player) > 24.0)
            .map(|e| e.pos)
            .collect();
        assert!(stuck.is_empty(), "still stuck after 20 s: {stuck:?}");
    }

    #[test]
    fn enemies_reach_the_player_on_every_theme() {
        // Same promise on the real maps: nobody is left in a blocked cell.
        //
        // The map is 3200 × 2560 px now, so a skitterer (30 px/s) cannot cross
        // the whole arena in 60 s of simulated time: the enemies are scattered
        // inside a ring around the rat — the director spawns them a few dozen
        // pixels out, and the flow field's fine window covers 384 px — which
        // still exercises walls, corners and doorways. (Beyond the fine window
        // the coarse block field is best-effort: a point whose block-graph exit
        // needs a detour *inside* its own 8 × 8-cell block can stall.)
        for theme in Theme::all() {
            let level = Level::new(theme);
            let mut f = FlowField::new(&level.tiles);
            let player = level.arena * 0.5;
            f.update(&level.tiles, player, NavBudget::FULL);
            let mut rng = StdRng::seed_from_u64(5);
            let mut enemies: Vec<Enemy> = (0..12)
                .map(|_| {
                    let angle = rng.random_range(0.0..std::f32::consts::TAU);
                    let radius = rng.random_range(60.0..350.0);
                    let p = player + Vec2::from_angle(angle) * radius;
                    Enemy::spawn(EnemyKind::Skitterer, p, 1.0, &mut rng)
                })
                .collect();
            let dt = 1.0 / 60.0;
            for _ in 0..60 * 60 {
                for e in enemies.iter_mut() {
                    let flow = f.dir_at(collision::SizeClass::Small, e.pos);
                    e.steer(player, flow, dt, 1.0);
                    e.pos = e
                        .pos
                        .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
                    e.pos = level.tiles.resolve_circle(
                        e.pos,
                        e.radius,
                        collision::SizeClass::of_enemy(e.kind),
                        None,
                    );
                }
            }
            let stuck: Vec<Vec2> = enemies
                .iter()
                .filter(|e| e.pos.dist(player) > 40.0)
                .map(|e| e.pos)
                .collect();
            assert!(stuck.is_empty(), "{theme:?}: stuck enemies at {stuck:?}");
        }
    }
}
