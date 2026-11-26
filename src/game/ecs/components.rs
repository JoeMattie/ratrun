//! Coarse ECS components for the first port.
//!
//! Contract: each legacy entity struct stays **one component as a whole** —
//! `Enemy`, `Bullet`, `Gem`, `Pickup`, `Pool` — plus a [`SpawnId`] ordering
//! component. Do not split them into Position/Health/etc.; the art functions
//! (`art_enemy::draw_enemy`, `art_rat::draw_rat`, ...) take `&Enemy` and
//! friends, and Phase 1 is a behaviour-preserving port.
//!
//! The leaf modules (`enemy.rs`, `bullet.rs`, `gem.rs`) are frozen except for
//! derives, so the `Component` impls for those types are manual impls here
//! instead of touching the leaf files. `Pool` moving out of `world.rs` is
//! re-exported from the facade for source compatibility.

use bevy_ecs::component::{Component, Mutable, StorageType};

use crate::game::bullet::Bullet;
use crate::game::enemy::Enemy;
use crate::game::gem::{Chest, Gem, Pickup};
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;
use crate::render::palette::Rgb;

pub use bevy_ecs::entity::Entity;

/// Monotonically increasing spawn order.
///
/// **Every** system that today depends on `Vec` index order (nearest-enemy
/// ties, separation nudges, per-enemy rng draws, reaping/drop order, bullet
/// spawn order, draw order) must iterate its query sorted by this component:
///
/// ```ignore
/// for (id, mut e) in enemies
///     .iter_mut()
///     .sort_by_key::<&SpawnId, _>(|id| id.0)
/// { /* legacy Vec order */ }
/// ```
///
/// `SpawnId` is assigned from [`super::resources::NextSpawnId`] at spawn time,
/// so it is stable across ticks (entities are NOT respawned between ticks).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpawnId(pub u64);

/// A ballistic lava glob's world-space footprint and out-of-plane height.
#[derive(Component, Clone, Copy, Debug)]
pub struct LavaGlob {
    pub ground: Vec2,
    pub from: Vec2,
    pub to: Vec2,
    pub t: f32,
    pub flight: f32,
    pub apex: f32,
    pub damage: f32,
    pub seed: u32,
}

impl LavaGlob {
    pub fn height(&self) -> f32 {
        let u = (self.t / self.flight).clamp(0.0, 1.0);
        4.0 * self.apex * u * (1.0 - u)
    }
}

/// The floor residue from a landed glob or a Caldera volley.
#[derive(Component, Clone, Copy, Debug)]
pub struct LavaPool {
    pub pos: Vec2,
    pub radius: f32,
    pub age: f32,
    pub life: f32,
    pub dps: f32,
    pub seed: u32,
}

/// Damaging ground pool left by an Acid Spit lob.
///
/// Moved here from `world.rs` (it was never an entity outside the world
/// module). The damage + decay + despawning all happen in one pass in
/// `systems::projectiles::pools_system` so the same-tick ordering matches the
/// legacy `update_pools`.
#[derive(Component, Clone, Debug)]
pub struct Pool {
    pub pos: Vec2,
    pub radius: f32,
    pub dps: f32,
    pub life: f32,
    pub color: Rgb,
}

// ---- Wave 3 weapons A: Acid Rain drops and Hantavirus marks -------------

/// One locked Acid Rain drop: a floor position a volley marked, with the
/// splash that lands there when its countdown runs out.
///
/// Spawned by `weapons::fire_one` (the whole volley at once, in drop order) at
/// the moment the weapon fires, so the mark ring is on the floor for the
/// entire wind-up and the drops are visibly staggered. `systems::
/// weapons_wave3a::acid_rain_system` counts `delay` down, applies the splash
/// once and despawns the entity. It never leaves a lingering pool (design §4:
/// "No lingering base pool").
#[derive(Component, Clone, Copy, Debug)]
pub struct AcidDrop {
    /// Where the drop lands (world px, clamped into the arena at fire time).
    pub pos: Vec2,
    /// Seconds until it lands; the art reads this for the falling strokes.
    pub delay: f32,
    /// Splash radius and per-splash damage, snapshotted when the volley fired.
    pub radius: f32,
    pub dmg: f32,
    /// Position in the volley (0-based), for deterministic art variety.
    pub index: u8,
    /// Fired by an evolved **Downspout** volley: the deck of drops is one
    /// marching gutter rather than one splash per marked foe (design §3).
    pub evolved: bool,
    /// Unit direction from the rat to the marked target (the gutter's march
    /// line); zero for a base volley. The runoff stripe is laid along it.
    pub dir: Vec2,
    /// Drops in the volley this drop belongs to, so the last landing knows the
    /// gutter has finished marching and can leave its runoff.
    pub volley: u8,
}

/// One **dropping** Hantavirus left on the floor: the weapon's whole delivery
/// now (owner's rule: "make the hantavirus weapon deployed by poops").
///
/// The rat leaves one every weapon cycle where it stands (5 px behind its
/// facing); the first unmarked body that steps inside `radius` takes the
/// [`Infection`] the old ranged tag used to plant, with `damage`/`jumps`/`reach`
/// snapshotted from the level **at drop time** (exactly as the old tag
/// snapshotted them at tag time). A drop is silent and harmless until it has
/// aged `DROPPING_ARM` seconds, lives `DROPPING_LIFE` seconds, and the weapon
/// keeps at most `DROPPING_CAP[level]` of them, oldest (`SpawnId`) first out.
///
/// Spawned by `weapons_wave3a::fire_hantavirus`, aged and spent by
/// `weapons_wave3a::droppings_system`, drawn by `art_droppings::draw_dropping`.
#[derive(Component, Clone, Copy, Debug)]
pub struct Dropping {
    /// Where it lies (world px).
    pub pos: Vec2,
    /// Seconds since it was dropped (the art's squash/fly age, and the arm).
    pub age: f32,
    /// Per-tick damage it was dropped with (the level's table times the rat's
    /// damage multiplier at drop time).
    pub damage: f32,
    /// Transfers the chain it starts may still make.
    pub jumps: u8,
    /// Transfer reach the chain it starts will use.
    pub reach: f32,
    /// Trigger radius in px (Lens Cap stretches it at drop time).
    pub radius: f32,
    /// Art variation seed. The drop's own `SpawnId` truncated — a drop draws
    /// no rng, so the picture cannot move a golden.
    pub seed: u32,
    /// Hantavirus weapon rank at drop time (1–5), used by art and lighting.
    pub level: u8,
}

/// Longest Hantavirus chain the visit history can record. The design's largest
/// chain is `1 + VIRUS_JUMPS[4]` = 5 bodies; the array is fixed-size so a chain
/// never allocates, with slack for future tuning.
pub const MAX_CHAIN: usize = 8;

/// An active Hantavirus mark on one body (an [`crate::game::enemy::Enemy`] or
/// the boss — a boss is not an `Enemy`, so its mark lives in
/// `resources::BossVirus` instead of on an entity).
///
/// Three ticks 0.35 s apart, then the remaining jumps pass to one fresh
/// neighbour. The whole chain shares one visit history (copied on every jump),
/// so a chain visits each body at most once and stops when it is isolated.
#[derive(Component, Clone, Copy, Debug)]
pub struct Infection {
    /// Ticks left (3 → 2 → 1 → 0). The pips drawn over the body show this.
    pub ticks_left: u8,
    /// Seconds until the next tick.
    pub timer: f32,
    /// Jumps the chain may still make from this host.
    pub jumps_left: u8,
    /// Reach of the next jump and the damage per tick, snapshotted from the
    /// weapon level when the chain was started.
    pub reach: f32,
    pub dmg: f32,
    /// `SpawnId`s this chain has already visited, in visit order.
    pub visits: [u64; MAX_CHAIN],
    pub visited: u8,
    /// Was this chain tagged by an evolved **Dead Letter** weapon? Only such a
    /// chain may seal its last jumps into a packet when its host dies.
    pub can_seal: bool,
    /// Has this chain already sealed a packet? Design §3: "one packet per
    /// chain", so a chain that reopens and dies again simply ends.
    pub sealed: bool,
}

impl Infection {
    /// A fresh mark on the body with `id` (three ticks, `jumps` transfers
    /// still available).
    pub fn new(id: u64, jumps: u8, reach: f32, dmg: f32) -> Self {
        let mut visits = [0u64; MAX_CHAIN];
        visits[0] = id;
        Self {
            ticks_left: crate::game::weapon::VIRUS_TICKS,
            timer: crate::game::weapon::VIRUS_TICK,
            jumps_left: jumps,
            reach,
            dmg,
            visits,
            visited: 1,
            can_seal: false,
            sealed: false,
        }
    }

    /// Is this mark still ticking? (A spent mark no longer blocks a new tag.)
    #[inline]
    pub fn active(&self) -> bool {
        self.ticks_left > 0
    }

    /// Has this chain already visited the body with `id`?
    #[inline]
    pub fn has_visited(&self, id: u64) -> bool {
        self.visits[..self.visited as usize].contains(&id)
    }

    /// The next generation of this mark: a fresh three-tick infection on a new
    /// host, one jump shorter and with `id` appended to the shared history.
    /// `None` when the fixed history is full (which caps a chain, it never
    /// allocates).
    #[inline]
    pub fn jumped(&self, id: u64) -> Option<Self> {
        if self.jumps_left == 0 || self.visited as usize >= MAX_CHAIN {
            return None;
        }
        let mut next = *self;
        next.visits[next.visited as usize] = id;
        next.visited += 1;
        next.ticks_left = crate::game::weapon::VIRUS_TICKS;
        next.timer = crate::game::weapon::VIRUS_TICK;
        next.jumps_left -= 1;
        Some(next)
    }
}

/// A dead Wick Moth's burning fuse (`WAVE3_DESIGN.md` §9).
///
/// The moth's body is reaped with every other corpse, so the fuse is its own
/// entity: [`crate::game::ecs::systems::enemy_wave3`] spawns one on the tick it
/// sees a dead moth, burns `fuse` down, and then bursts for 9 to the rat and 20
/// to every enemy inside 9 px. A moth killed by a burst leaves its own fuse, so
/// the chain only ends when the moths do.
#[derive(Component, Clone, Debug)]
pub struct Wick {
    pub pos: Vec2,
    /// Seconds left before the burst.
    pub fuse: f32,
}

impl Wick {
    /// A fresh, fully-fused wick at `pos`.
    pub fn new(pos: Vec2) -> Self {
        Wick {
            pos,
            fuse: crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS,
        }
    }
}

// ---- Component impls for the frozen leaf entity structs ------------------

impl Component for Enemy {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

impl Component for Bullet {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

impl Component for Gem {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

impl Component for Pickup {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

// ---- Evolution entities (WAVE3_DESIGN.md §3) -----------------------------
//
// One component per evolved weapon that needs state between ticks. They are
// owned by `systems::evolutions` (except the two markers, which
// `projectiles::bullets_system` reads at the moment the evolved shot lands or
// hits a wall). Every history below is a fixed-size array: nothing here
// allocates per tick, and every walk over them is `SpawnId`-ordered.

/// Bodies a single Milk Teeth return thread may bite (fixed history).
pub const MILK_BITE_CAP: usize = 16;
/// Bodies one Fondue Fork string may cut (fixed history).
pub const STRING_CUT_CAP: usize = 24;
/// Bodies one Fairy Ring may track for its "one hit per target per second".
pub const FAIRY_TRACK_CAP: usize = 24;
/// Bodies one Drain Cleaner channel may damage on its advancing front.
pub const CHANNEL_HIT_CAP: usize = 24;

/// Marker on a Squeak Nova spoke fired by an **Echo Chamber** evolution: its
/// first wall contact reflects the spoke along its incoming line instead of
/// consuming it (one reflection per spoke).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct EchoSpoke {
    pub reflected: bool,
}

/// Marker on an evolved projectile whose landing behaviour differs from its
/// base weapon (Acid Spit → **Drain Cleaner**'s directed channel).
#[derive(Component, Clone, Copy, Debug)]
pub struct EvolvedShot {
    pub kind: WeaponKind,
    /// Firing direction (unit vector); the corrosive channel follows it.
    pub dir: Vec2,
}

/// Phase of a lodged **Milk Teeth** dart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MilkPhase {
    /// Flying out toward the aim point, biting the first body it crosses.
    Out,
    /// Lodged: hanging still for [`crate::game::ecs::systems::evolutions::MILK_LODGE_SECS`].
    Lodged,
    /// Pulling back to the rat's current position, biting each crossed body.
    Back,
}

/// A **Milk Teeth** dart: out, lodge 1 s, then home back to the rat.
#[derive(Component, Clone, Debug)]
pub struct MilkTooth {
    pub pos: Vec2,
    pub vel: Vec2,
    pub phase: MilkPhase,
    /// Seconds in the current phase.
    pub age: f32,
    /// Per-hit damage (the weapon's L5 hit after stats).
    pub damage: f32,
    /// How far the dart flies before it lodges (Lens Cap stretches it, exactly
    /// as it stretches the base dart's projectile life).
    pub reach: f32,
    /// Bodies bitten on the way back, in bite order.
    pub bitten: [Option<u64>; MILK_BITE_CAP],
    pub bitten_n: u8,
    /// Has the return thread already bitten the boss (`Boss` is not an
    /// `Enemy`, so it needs its own flag: one boss hit per return).
    pub boss_bitten: bool,
}

impl MilkTooth {
    pub fn new(pos: Vec2, vel: Vec2, damage: f32, reach: f32) -> Self {
        Self {
            pos,
            vel,
            phase: MilkPhase::Out,
            age: 0.0,
            damage,
            reach,
            bitten: [None; MILK_BITE_CAP],
            bitten_n: 0,
            boss_bitten: false,
        }
    }

    #[inline]
    pub fn has_bitten(&self, id: u64) -> bool {
        self.bitten[..self.bitten_n as usize].contains(&Some(id))
    }

    /// Record a bite. Returns `false` when this body must **not** be hit: it
    /// was already bitten, or the fixed history is full.
    ///
    /// The cap bounds the memory, so a successful record is a precondition of
    /// the damage: otherwise a 17th body in the same thread would be bitten on
    /// every tick it stayed inside the sweep (a once-per-target effect turning
    /// into damage over time).
    pub fn remember_bite(&mut self, id: u64) -> bool {
        if self.has_bitten(id) {
            return false;
        }
        if (self.bitten_n as usize) >= MILK_BITE_CAP {
            return false;
        }
        self.bitten[self.bitten_n as usize] = Some(id);
        self.bitten_n += 1;
        true
    }
}

/// A **Fondue Fork** cheese string between two planted tines: cuts each crosser
/// once while it lasts.
#[derive(Component, Clone, Debug)]
pub struct CheeseString {
    pub a: Vec2,
    pub b: Vec2,
    pub life: f32,
    pub total: f32,
    pub damage: f32,
    pub cut: [Option<u64>; STRING_CUT_CAP],
    pub cut_n: u8,
}

impl CheeseString {
    pub fn new(a: Vec2, b: Vec2, life: f32, damage: f32) -> Self {
        Self { a, b, life, total: life, damage, cut: [None; STRING_CUT_CAP], cut_n: 0 }
    }

    pub fn has_cut(&self, id: u64) -> bool {
        self.cut[..self.cut_n as usize].contains(&Some(id))
    }

    /// Record a cut. Returns `false` when this body must **not** be hit (it
    /// was already cut, or the fixed history is full): a successful record is
    /// a precondition of the damage, so a 25th body standing in the same string
    /// cannot be cut once per tick for the string's whole life.
    pub fn remember_cut(&mut self, id: u64) -> bool {
        if self.has_cut(id) || (self.cut_n as usize) >= STRING_CUT_CAP {
            return false;
        }
        self.cut[self.cut_n as usize] = Some(id);
        self.cut_n += 1;
        true
    }
}

/// A **Fairy Ring**: satellites left fixed around the casting point. Crossing
/// the band costs one hit per body per second.
#[derive(Component, Clone, Debug)]
pub struct FairyRing {
    pub pos: Vec2,
    pub radius: f32,
    pub life: f32,
    pub total: f32,
    pub spin: f32,
    pub satellites: u8,
    pub damage: f32,
    /// Per-body hit cooldowns (SpawnId, seconds left), a fixed history.
    pub cooled: [Option<(u64, f32)>; FAIRY_TRACK_CAP],
}

impl FairyRing {
    pub fn new(pos: Vec2, radius: f32, life: f32, satellites: u8, damage: f32) -> Self {
        Self {
            pos,
            radius,
            life,
            total: life,
            spin: 0.0,
            satellites,
            damage,
            cooled: [None; FAIRY_TRACK_CAP],
        }
    }

    /// Seconds of cooldown left for `id` (0 when it may be hit again).
    pub fn cooldown(&self, id: u64) -> f32 {
        for entry in self.cooled.iter().flatten() {
            if entry.0 == id {
                return entry.1;
            }
        }
        0.0
    }

    /// Start (or restart) `id`'s cooldown. Returns `false` when the fixed table
    /// is full of **other** live cooldowns, i.e. this body cannot be tracked:
    /// the caller then skips the hit, because evicting a live cooldown would let
    /// the evicted body be hit again on the next tick (the same body, twice per
    /// second, for a `FAIRY_TRACK_CAP`+1-binding ring).
    pub fn cool(&mut self, id: u64, secs: f32) -> bool {
        for (i, slot) in self.cooled.iter().enumerate() {
            match slot {
                Some((held, left)) if *held == id => {
                    self.cooled[i] = Some((id, secs.max(*left)));
                    return true;
                }
                Some(_) => {}
                None => {
                    self.cooled[i] = Some((id, secs));
                    return true;
                }
            }
        }
        false
    }

    pub fn tick_cooldowns(&mut self, dt: f32) {
        for slot in self.cooled.iter_mut() {
            if let Some((id, left)) = *slot {
                let left = left - dt;
                *slot = if left <= 0.0 { None } else { Some((id, left)) };
            }
        }
    }
}

/// A **Drain Cleaner** channel: the splash runs `CHANNEL_LEN` px along the
/// firing direction over 0.6 s, then the settled stripe ticks for 2 s.
#[derive(Component, Clone, Debug)]
pub struct CorrosiveChannel {
    pub start: Vec2,
    /// Unit firing direction.
    pub dir: Vec2,
    /// Length still open to walk down (truncated at the first wall).
    pub length: f32,
    /// Seconds since the splash landed.
    pub age: f32,
    /// Pool damage per second for the settled phase (base pool dps).
    pub dps: f32,
    /// Per-body damage of the advancing front (one hit each).
    pub front_damage: f32,
    pub hit: [Option<u64>; CHANNEL_HIT_CAP],
    pub hit_n: u8,
    /// Seconds the settled stripe keeps ticking after the front has run
    /// (the design's 2 s; **Downspout**'s runoff is a shorter one). Long Fuse
    /// stretches this at tick time, never at spawn time.
    pub settle: f32,
    /// Is this stripe a **Downspout** runoff? A runoff has no advancing front
    /// (it is already settled when it lands) and draws as a stepped gutter.
    pub runoff: bool,
    /// Has the advancing front already hit the full boss? (The boss is not an
    /// `Enemy`, so it needs its own once-per-channel flag.)
    pub boss_hit: bool,
}

/// Seconds a settled corrosive stripe ticks for with [`CorrosiveChannel::new`]
/// (the Drain Cleaner's 2 s; `evolutions::CHANNEL_SETTLE` is this value).
pub const CHANNEL_DEFAULT_SETTLE: f32 = 2.0;

impl CorrosiveChannel {
    pub fn new(start: Vec2, dir: Vec2, length: f32, dps: f32, front_damage: f32) -> Self {
        Self {
            start,
            dir,
            length,
            age: 0.0,
            dps,
            front_damage,
            hit: [None; CHANNEL_HIT_CAP],
            hit_n: 0,
            settle: CHANNEL_DEFAULT_SETTLE,
            runoff: false,
            boss_hit: false,
        }
    }

    /// A settled **Downspout** runoff stripe: it burns from the first tick (no
    /// advancing front) for `settle` seconds.
    pub fn gutter(start: Vec2, dir: Vec2, length: f32, dps: f32, settle: f32) -> Self {
        Self {
            runoff: true,
            settle,
            ..Self::new(start, dir, length, dps, 0.0)
        }
    }

    pub fn has_hit(&self, id: u64) -> bool {
        self.hit[..self.hit_n as usize].contains(&Some(id))
    }

    /// Record a front hit. Returns `false` when this body must **not** be hit
    /// (already hit, or the fixed history is full): a successful record is a
    /// precondition of the damage, so a body past the cap cannot be hit again
    /// every tick while the front is still advancing.
    pub fn remember_hit(&mut self, id: u64) -> bool {
        if self.has_hit(id) || (self.hit_n as usize) >= CHANNEL_HIT_CAP {
            return false;
        }
        self.hit[self.hit_n as usize] = Some(id);
        self.hit_n += 1;
        true
    }

    /// Far end of the stripe, clamped to the wall it stopped at.
    pub fn end(&self) -> Vec2 {
        self.start + self.dir * self.length
    }

    /// The stripe centre (for the settled tick test).
    pub fn centre(&self) -> Vec2 {
        self.start + self.dir * (self.length * 0.5)
    }
}

/// A sealed **Dead Letter** packet: a Hantavirus chain that ran out of
/// neighbours when its host died, folded into an envelope and left on the
/// floor for two seconds.
///
/// `evolutions::dead_letter_system` ticks the fuse and hands the chain to the
/// first eligible foe that steps within reach; when the fuse runs out the
/// chain is lost. A packet carries the whole chain (its visit history, its
/// remaining jumps), so reopening it resumes the same walk rather than
/// starting a new one.
#[derive(Component, Clone, Copy, Debug)]
pub struct DeadLetterPacket {
    pub pos: Vec2,
    /// Seconds left before the envelope is lost.
    pub life: f32,
    /// Life it started with (the art's fade).
    pub total: f32,
    /// The chain sealed inside.
    pub chain: Infection,
}

/// Queue with `commands.queue(ChestSpawn { pos })` to drop a chest from a system.
pub struct ChestSpawn {
    pub pos: Vec2,
}

impl ChestSpawn {
    pub fn spawn(self, ecs: &mut bevy_ecs::world::World) -> Entity {
        super::resources::spawn_with_id(ecs, Chest { pos: self.pos })
    }
}

impl bevy_ecs::system::Command for ChestSpawn {
    type Out = ();

    fn apply(self, ecs: &mut bevy_ecs::world::World) {
        self.spawn(ecs);
    }
}

/// One **Fondue Fork** tine in flight: the evolved cheese spray's two outer
/// darts are real projectiles (`evolutions::fondue_tines_system`), not a
/// cast-time ray, so the string plants where they actually land.
///
/// The base spray's darts are `Bullet`s; a tine is deliberately its own
/// component because its life ends at a *plant point* rather than in a despawn,
/// and because two tines of one cast have to agree on where their string goes.
#[derive(Component, Clone, Copy, Debug)]
pub struct FondueTine {
    pub pos: Vec2,
    pub prev: Vec2,
    pub vel: Vec2,
    /// One dart's damage (the tine hits the first body it crosses).
    pub damage: f32,
    /// Flight left in px (Lens Cap stretches it).
    pub reach: f32,
    /// Which end of the string this tine plants (0 = the first outer dart,
    /// 1 = the second).
    pub slot: u8,
    /// `SpawnId` of the [`FondueCast`] this tine belongs to.
    pub cast: u64,
}

/// A **Fondue Fork** cast: two tines in flight and the string they plant when
/// both have settled. Spawned by `weapons::fire_one`, resolved here.
#[derive(Component, Clone, Copy, Debug)]
pub struct FondueCast {
    /// Plant points, one per tine, filled as they land.
    pub ends: [Option<Vec2>; 2],
    /// What the finished string cuts for.
    pub damage: f32,
    /// Seconds the cast may wait for its tines (they always settle by reach).
    pub wait: f32,
}

// ---- The Spiked Tail Whip (wave 3, w3-spiketail) -------------------------

/// Which body a lash is aimed at. A boss is not an `Enemy`, so it cannot be
/// named by an `Entity` here: it is found again at the crack through its hit
/// shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LashTarget {
    /// A regular enemy body (named minibosses included; they are `Enemy`s).
    Enemy(Entity),
    /// One of the boss's grounded hit points, locked when the lash fired.
    Boss,
}

/// One Spiked Tail lash in flight: the crack the tail is winding up to.
///
/// Spawned by `weapons::fire_one` at the moment the weapon fires, so the tail
/// is already bristling on that frame. `systems::weapon_spiketail` follows the
/// body for the [`crate::game::weapon::LASH_WINDUP`] seconds of wind-up, lands
/// the crack (primary hit + small burst), and — for an evolved **Barbed Wire**
/// lash — drags its catch home on the recoil. Pure data: every number that
/// matters is snapshotted here, so the strike reads nothing from the loadout
/// (a level-up card opened mid-lash cannot change the hit that was paid for).
#[derive(Component, Clone, Copy, Debug)]
pub struct Lash {
    /// The tail's root: the rat's position, followed every tick until the
    /// crack, so the whip starts at the rump it is actually attached to.
    pub origin: Vec2,
    /// The impact point. Follows the target body while it lives (a lash that
    /// misses a walking body reads as broken), and stays put once it is gone.
    pub target: Vec2,
    /// The body the full lash damage lands on, if it was an enemy.
    pub primary: Option<LashTarget>,
    /// Was the locked impact point a boss hit shape? Then the boss takes the
    /// full lash damage there (and can still take the burst once, below).
    pub boss: bool,
    /// Reach (rat to impact) and burst radius, stats applied at fire time.
    pub reach: f32,
    pub radius: f32,
    /// Full lash damage and the primary's shove velocity (Counterweight
    /// applied); the burst is `LASH_BURST_FRAC` of `dmg`.
    pub dmg: f32,
    pub shove: f32,
    /// Spike shards that break off at the crack (0 below level 4).
    pub shards: u8,
    /// Fired by an evolved **Barbed Wire** lash: the recoil drags the catch.
    pub evolved: bool,
    /// Seconds since the fire (0 → `LASH_TOTAL`).
    pub timer: f32,
    /// The crack has landed.
    pub struck: bool,
    /// The recoil drag (Barbed Wire) has resolved.
    pub dragged: bool,
}

/// Which of the lash's three beats it is in right now (`Lash::phase`).
///
/// The boundaries are the simulation's own clock
/// (`weapon::LASH_WINDUP` / `LASH_RECOIL_AT` / `LASH_TOTAL`), so an animation
/// driven off this can never drift from the damage it is drawing:
///
/// ```text
/// spawn ── WindUp ── 0.12 ── Crack ── 0.20 ── Recoil ── 0.34 ──> despawn
///         bristle         the crack lands   hold + Barbed Wire drag, settle
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LashPhase {
    /// Bristling: the tail has not landed yet.
    WindUp,
    /// The crack landed. The tail holds extended (and Barbed Wire starts its
    /// drag at `LASH_RECOIL_AT`).
    Crack,
    /// The tail is reeling back and settling.
    Recoil,
    /// Past `LASH_TOTAL`: only exists for the tick it is despawned on.
    Over,
}

impl LashPhase {
    /// `(start, length)` of this phase in seconds since the fire. The last
    /// phase has zero length.
    pub fn span(self) -> (f32, f32) {
        use crate::game::weapon::{LASH_RECOIL_AT, LASH_TOTAL, LASH_WINDUP};
        match self {
            LashPhase::WindUp => (0.0, LASH_WINDUP),
            LashPhase::Crack => (LASH_WINDUP, LASH_RECOIL_AT - LASH_WINDUP),
            LashPhase::Recoil => (LASH_RECOIL_AT, (LASH_TOTAL - LASH_RECOIL_AT).max(0.0)),
            LashPhase::Over => (LASH_TOTAL, 0.0),
        }
    }
}

impl Lash {
    /// The tail's root: where the whip is actually anchored (the rat's rump,
    /// re-read every tick of the wind-up).
    #[inline]
    pub fn base(&self) -> Vec2 {
        self.origin
    }

    /// The locked impact point: follows the target body while it lives, then
    /// stays where it died.
    #[inline]
    pub fn tip(&self) -> Vec2 {
        self.target
    }

    /// Seconds since the fire (0 → `LASH_TOTAL`).
    #[inline]
    pub fn elapsed(&self) -> f32 {
        self.timer
    }

    /// Which beat the lash is in.
    pub fn phase(&self) -> LashPhase {
        use crate::game::weapon::{LASH_RECOIL_AT, LASH_TOTAL, LASH_WINDUP};
        if self.timer < LASH_WINDUP {
            LashPhase::WindUp
        } else if self.timer < LASH_RECOIL_AT {
            LashPhase::Crack
        } else if self.timer < LASH_TOTAL {
            LashPhase::Recoil
        } else {
            LashPhase::Over
        }
    }

    /// Progress through the whole lash, `0..=1` across `LASH_TOTAL`.
    pub fn progress(&self) -> f32 {
        (self.timer / crate::game::weapon::LASH_TOTAL.max(1e-4)).clamp(0.0, 1.0)
    }

    /// Progress through the current beat, `0..=1` (`1.0` once the lash is over).
    pub fn phase_progress(&self) -> f32 {
        let (start, len) = self.phase().span();
        if len <= 0.0 {
            return 1.0;
        }
        ((self.timer - start) / len).clamp(0.0, 1.0)
    }

    /// Seconds from the fire to the crack (`weapon::LASH_WINDUP`).
    #[inline]
    pub fn crack_time(&self) -> f32 {
        crate::game::weapon::LASH_WINDUP
    }

    /// The tick (1 = the tick the lash was spawned on) the crack lands on at
    /// this frame time. `spiked_tail_system` advances `timer` by one `dt` on the
    /// spawn tick, so the crack is resolved on the first tick where
    /// `n * dt >= crack_time()`.
    pub fn crack_tick(&self, dt: f32) -> u32 {
        if dt <= 0.0 {
            return 1;
        }
        (crate::game::weapon::LASH_WINDUP / dt).ceil().max(1.0) as u32
    }

    /// Ticks left until the crack (0 once it has landed).
    pub fn ticks_to_crack(&self, dt: f32) -> u32 {
        if dt <= 0.0 || self.struck {
            return 0;
        }
        let done = (self.timer / dt).round() as u32;
        self.crack_tick(dt).saturating_sub(done)
    }

    /// Has the crack landed?
    #[inline]
    pub fn cracked(&self) -> bool {
        self.struck
    }

    /// Did the crack land **during the last tick**? True on exactly one frame
    /// (the one the animation's white flash belongs on).
    pub fn cracked_now(&self, dt: f32) -> bool {
        self.struck && self.timer - dt < crate::game::weapon::LASH_WINDUP
    }

    /// Is this an evolved **Barbed Wire** lash (the recoil drags its catch)?
    #[inline]
    pub fn barbed_wire(&self) -> bool {
        self.evolved
    }

    /// Seconds from the fire to the start of the Barbed Wire drag
    /// (`weapon::LASH_RECOIL_AT`).
    #[inline]
    pub fn drag_time(&self) -> f32 {
        crate::game::weapon::LASH_RECOIL_AT
    }

    /// The tick the Barbed Wire drag resolves on, if this lash has one.
    pub fn drag_tick(&self, dt: f32) -> Option<u32> {
        if !self.evolved || dt <= 0.0 {
            return None;
        }
        Some((crate::game::weapon::LASH_RECOIL_AT / dt).ceil().max(1.0) as u32)
    }

    /// Has the Barbed Wire drag resolved?
    #[inline]
    pub fn dragged(&self) -> bool {
        self.dragged
    }

    /// Did the drag resolve during the last tick?
    pub fn dragged_now(&self, dt: f32) -> bool {
        self.evolved
            && self.dragged
            && self.timer - dt < crate::game::weapon::LASH_RECOIL_AT
    }
}

// ---- The melee weapons' snapshots (batch 12) -----------------------------
//
// Gnaw's bite and Tail Whip's crack are instant: they damage, and then the only
// thing left is the picture. Rather than a new schedule slot, each is a
// short-lived component that `weapons::fire_weapons_system` ages and despawns
// on its own pass (that system runs every tick regardless of whether either
// weapon fires). Art reads them; nothing in the simulation does.

/// One **Gnaw** bite: a snapping jaw in front of the rat's nose.
#[derive(Component, Clone, Copy, Debug)]
pub struct BiteSnap {
    /// The bite's aim point on the rat's face plane.
    pub pos: Vec2,
    /// Unit vector the jaw closed along (the reticle, else the rat's facing).
    pub dir: Vec2,
    /// Seconds left to draw.
    pub t: f32,
    /// The bite's whole life in seconds.
    pub total: f32,
}

/// One **Tail Whip** swipe: a 160° crescent opposite the aim or facing.
///
/// * `from` — the rat on the swipe tick (the arc's centre),
/// * `to` — the **centre of the arc**: `from + dir * reach`, so
///   `dir = (to - from).normalized()` and `reach = (to - from).len()`,
/// * the drawn arc is `dir ± WHIP_ARC_HALF` out to `reach`,
/// * `t` counts down from `total` = `WHIP_SWIPE_SECS`.
#[derive(Component, Clone, Copy, Debug)]
pub struct WhipCrack {
    /// The rat (the arc's centre).
    pub from: Vec2,
    /// The arc's centre line at the swipe's reach, opposite the aim or facing.
    pub to: Vec2,
    /// Seconds left to draw.
    pub t: f32,
    /// The swipe's whole life in seconds.
    pub total: f32,
}

// ---- The guest: the Drake (Sewer only) ------------------------------------
//
// One per body, inserted at spawn (`systems::drake::spawn_guest`) and, for a
// body that reached the floor another way (`spawn drake`, a test), on the
// first tick `drake_system` sees it — the Sump Leech's own rule. The component
// is authoritative; the three numbers an `&Enemy` reader needs (the phase, the
// phase clock and the aim) are mirrored into the body's otherwise-unused `w4`
// scratch, exactly as `floor_sewer` mirrors a leech's tongue.

/// Which of the Drake's four states the body is in.
///
/// The discriminant **is** the `u8` the wave-4 scratch mirrors
/// (`w4.phase`), so the bot, the damage attribution and
/// `Enemy::damage_mult_from` read one number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrakeState {
    /// Walking toward the rat on the ordinary flow field (the only state it
    /// moves itself in).
    #[default]
    Waddle,
    /// Planted, neck drawn back, the peck lane outlined on the floor.
    Tell,
    /// Planted, the bill going out and back: three pecks.
    Peck,
    /// Planted, exposed: x1.5 incoming damage for 1.3 s.
    Recover,
}

/// One Drake's state machine (`ecs/systems/drake.rs` owns the timings).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Drake {
    pub state: DrakeState,
    /// Seconds left in the current state (for `Peck`, in the current peck).
    pub t: f32,
    /// Pecks still to throw in this volley (0 outside `Peck`).
    pub pecks_left: u8,
    /// Unit vector the bill travels along: it tracks the rat until
    /// `drake::TELL_LOCK_SECS` into the tell, then locks.
    pub aim: Vec2,
    /// Seconds spent walking, for the waddle cycle (never reset by a tell).
    pub walk: f32,
    /// Where the body planted itself for this tell → peck → recover cycle. It
    /// is pinned here every tick, so a shove, a separation nudge or a wall
    /// resolution cannot drag a committed bird off its mark.
    pub anchor: Vec2,
    /// Standing in a puddle this tick: the drawn lower body is hidden by a
    /// ripple line (`art_duck::DuckView::on_water`). Cached on the component
    /// because the view is built from the body and its state alone — art never
    /// asks the map a question.
    pub on_water: bool,
}
