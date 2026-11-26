//! **Card scenes**: the selected Library card plays the real game.
//!
//! A [`LibraryScene`] owns a small [`World`] of its own — a hand-built patch
//! of open floor (or, for a floor's card, that floor's generated map), a
//! fixed seed, a rat that cannot be hurt, no director — and a short script:
//! spawn the subject, place a few bodies, feed scripted input to
//! [`World::update`] at a fixed step, and draw the world into the card's art
//! plate. When the script ends the scene is rebuilt from scratch, so every
//! loop is the same loop.
//!
//! Because the card runs the simulation's own systems and the renderer's own
//! draw pass, it cannot show a tell, a footprint or a state the game does not
//! have. Nothing here is read by a run: the scene world is separate from the
//! run's, has its own rng, and never touches the save, the Library's
//! discovery flags, the nest or the campaign. Draining the scene's `SfxQueue`
//! and its `pending_*` flags on every step is what keeps a card from opening a
//! level-up plate or a shelf inside a menu.
//!
//! ## A plate is a portrait of its subject
//!
//! The owner's note on the first cut: *"the animations in library shouldn't all
//! chase after a rat"*. They did, because the rat is what every hostile
//! behaviour steers at, and the rat is the biggest, brightest sprite on the
//! plate. So a bestiary card now shows **no rat at all**:
//!
//! * The simulation keeps one — invulnerable, invisible, stationary or on a
//!   scripted waypoint — because attacks, steering and ranges all need a
//!   target. [`World::hide_rat`] is the render-only flag that takes it out of
//!   the picture (the sprite *and* the ink pass's minimum-size core).
//! * Where an attack needs something to hit, the card draws its own neutral
//!   practice target at that point ([`draw_practice_dummy`]): a small cork with
//!   a cheese-rind band, painted by this module after the world's pass. It is
//!   never a simulation entity, so no system can see it, hurt it or count it.
//! * The camera frames the subject and its aim point, centred, at a zoom that
//!   keeps the subject's own footprint on the plate ([`Framing`]).
//!
//! ## The six bestiary families ([`stage_enemy`])
//!
//! What a card shows follows the body's **roster slot** on its own floor, so
//! the plate demonstrates the kind's real rule instead of restaging one
//! generic duel:
//!
//! | slot | plate |
//! |---|---|
//! | A, B (fodder) | a pack of three to five crossing the plate in its own formation, chasing a hidden waypoint that runs edge to edge and back; no dummy |
//! | C (chaser) | one body running a lap round an obstacle — a pipe stub, or a rat-hole wall a Cat-lineage body cannot pass |
//! | R (shooter) | a stand-off body strafing and firing at the dummy down a **short lane** (34 px), so the body, the muzzle and the pellet are drawn large enough to read; the Toll Gull's lane also frames the 22 px its leading shot travels *past* the dummy |
//! | I, II, III (tricks) | the body's own tell → strike → recovery on the dummy, with room for its footprint |
//! | H (heavy) | the body lumbering in at its real 16 px/s past the dummy, a size reference |
//! | Elite | the same pass with the floor's heavy wearing its gild |
//!
//! Named fights and bosses have their own staging ([`stage_miniboss`],
//! [`stage_boss`]); a named fight's plate **fits the footprint it is about**
//! (body + the live telegraph), because a cone, a cord sweep, the Salt Hen's
//! three pads and the Mudlark's claws are all wider than a fixed zoom allowed.
//! The arsenal and the field guide keep the rat only where the rat *is* the
//! point (firing, collecting, squeezing through a hole).
//!
//! ## Keeping the picture honest
//!
//! * The rat is `god`: a card may show the rat being hit, never killed (a
//!   death would end the loop and freeze the plate on `game_over`). A hidden
//!   rat is still the real one, so its invulnerability is the real guarantee.
//! * Bestiary plates are `flat_lit`: a card is a portrait, and the pitch-dark
//!   ambient of a floor hides the very thing it is about. The hostile layer and
//!   the card's dummy are self-lit on top, as always.
//! * The scene world keeps its own rng (`SEED`), so no card can move a golden
//!   or the map pin.
//! * The card's dummy is painted **under** the picture. There is no mid-scene
//!   draw hook on [`World::draw`], so the plate renders the floor layer a
//!   second time as a reference and the cork only takes pixels the world's own
//!   pass left as bare floor: no body, tell, pellet or strand is ever covered
//!   by the target it is standing on (see [`draw_practice_dummy`]).
//! * A named fight's plate **fits its footprint** ([`Script::fit`]): the stage
//!   centres on the live union of body and telegraph and takes its zoom from
//!   the shape's own bounds (with a margin), so a cone, a cord sweep or the
//!   Salt Hen's three stamp pads cannot leave the frame.
//! * The flatbed's cards stand on the trailer's real bed ([`deck_patch`]): that
//!   floor's planks, joints, rails and slipstream are anchored to the map's own
//!   deck rectangle, not to wherever a small patch happens to sit, so a Flatbed
//!   body is shown on its own floor and the Gravel Tick's tailgate drift reads.
//! * A boss's plate frames the **union of its drawn body over a whole loop**
//!   ([`LibraryScene::probe_boss_union`]): every state of the controller is
//!   measured once when the card is built, the camera is pinned on the middle
//!   of that union and the downscale is chosen so the union keeps a 10 % margin.
//!   The walk-in a boss spends its first second on clipped the top of the
//!   Sweeper and SUBJECT R-01; a moving camera on a body that lunges, stomps and
//!   coils could not be right for every state, and a union can.
//! * **KING COBRA** is framed on the head, the hood and the **nearest coil**
//!   ([`boss_body_bounds`]) instead of the whole laid ring: the ring is the
//!   arena's breadth and the animal's face is the card.
//! * **THE REDTAIL** is fought at hover height (a scripted state cue), not
//!   overhead: while it circles, the art gives it no hit shape at all and the
//!   plate showed a shadow and nothing else.
//! * A **floor's** card is lit by its real lighting and framed on a furnished
//!   room ([`floor_window`]), with its fodder wandering through it and no rat.
//!   The Outfall's hand-staged bank also keeps a wet patch and a working lure
//!   in the pan.
//! * A **tiny body** (a radius under 4 px: the Bell Flea, the Wick Moth, the
//!   Lilliputians, the Ink Blot) is drawn at 2 ×, which is what makes it a body
//!   and not a speck; its practice dummy stands at least 14 px off.
//! * The animation gate is the **caller's** rule, never this module's:
//!   `render::library::catalogued` (seen, and for a weapon, passive or
//!   evolution *owned*) decides whether a card runs at all, so a locked card
//!   stays `???` and a weapon the player was merely offered keeps the static
//!   silhouette pose `draw_art` gives it. [`plan`] is unconditional, which is
//!   what guarantees that a plate the caller *does* run is never blank.

use bevy_ecs::entity::Entity;

use crate::game::art_boss_broom;
use crate::game::art_boss_cat;
use crate::game::art_boss_cobra;
use crate::game::art_boss_hawk;
use crate::game::art_boss_horseshoe_crab;
use crate::game::art_boss_lantern::{self, LanternState};
use crate::game::art_boss_magpie;
use crate::game::art_boss_megarat;
use crate::game::art_boss_mower;
use crate::game::art_boss_night_edition;
use crate::game::art_traps::{MazeRewardView, MousetrapView, RatHoleView, StickyTrapView};
use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
use crate::game::bullet::{Bullet, Faction};
use crate::game::campaign::CampaignTotals;
use crate::game::collision::{SizeClass, TileKind, TileMap};
use crate::game::ecs::resources::{spawn_with_id, DashHold};
use crate::game::ecs::systems::boss::{
    Boss, BossArtView, BossHitShape, BossKind, BossState, HazardShape,
};
use crate::game::ecs::systems::floor_outfall::{
    lampfish_bulb, outfall_enemy_tell, LAMPFISH_TRIGGER_REACH,
};
use crate::game::ecs::systems::miniboss::{
    Miniboss, MinibossKind, MinibossShape, RECOVER_TIME, TELL_TIME,
};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Gold, Pickup, PickupKind};
use crate::game::hazards::{InkPad, LureTrap, SteamGrate};
use crate::game::level::{
    BossArena, Level, LevelObjects, LightFixture, LightFixtureKind, OneWayDoor, Palette, Pit,
    PitKind, Prop, PropKind, Puddle, RatHole, RoomKind, ShopHole, ShopHoleRole, SlidingCargo, Theme,
    TileRect, TrailerDeck,
};
use crate::game::library::{self, Key, ObjectKind};
use crate::game::loadout::{Loadout, PassiveKind};
use crate::game::nav::NavGrid;
use crate::game::roster::{self, Slot};
use crate::game::weapon::{Weapon, WeaponKind};
use crate::game::world::World;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// The scene's fixed simulation step.
pub const TICK: f32 = 1.0 / 60.0;
/// How long the cursor must rest on a card before its scene is built: fast
/// scrolling shows the still pose and builds nothing.
pub const REST_SECS: f32 = 0.15;
/// The most simulation steps one [`LibraryScene::advance`] runs (a stalled
/// frame must not turn into a burst of ticks).
const MAX_STEPS: u32 = 4;
/// Every scene world's seed. A scene is a fixture, not a roll.
const SEED: u64 = 0x011B_2A27;

/// Seconds in one pack loop: a crossing of the plate, a turn and a crossing
/// back.
const PACK_LOOP: f32 = 5.0;
/// How far either side of the plate's centre the pack's hidden waypoint runs.
/// Just outside the bodies' cluster, so they stream across and turn there.
const PACK_REACH: f32 = 20.0;
/// Seconds in one chaser lap (one full turn of the hidden waypoint).
const LAP_LOOP: f32 = 6.0;
/// The practice dummy's radius in world pixels: rat-sized, so it stands in for
/// the target the subject is really attacking.
const DUMMY_R: f32 = 6.0;
/// The body-to-dummy lane a shooter's plate is staged on, in world pixels.
///
/// The Spitter archetype holds a real 80-110 px stand-off, which is wider than
/// a plate; the card therefore stages its own: the body is held on a slow arc
/// at this distance, so the whole flight, the muzzle and the dummy share one
/// frame (and the scene's own strafe is the movement the card is about).
/// 34 px leaves both ends a body's width inside the 55 px plate a 1.25× zoom
/// shows, which is what makes the body, the muzzle swing and the pellet
/// themselves readable instead of three small marks on a wide floor.
const SHOT_LANE: f32 = 34.0;
/// How far past the dummy the Toll Gull's plate frames the lead point (the
/// pellet of a leading shot lands there when the target is running away).
const SHOT_LEAD: f32 = 22.0;
/// The ring the arsenal plate stands its three targets on, in world pixels:
/// inside every weapon's own reach (the whip's aura is 26 + 3 × level, the
/// squeak ring 90+), and outside the `< 26 px` radius where the renderer
/// suppresses hit digits — so the flash *and* the number both show.
const ARSENAL_RING: f32 = 30.0;
/// How far Gnaw's targets stand from the rat: inside the level-5 jaw (24 px).
const BITE_RING: f32 = 20.0;
/// The Hantavirus plate: how fast (rad/s) the rat **turns on the spot**, so the
/// dropping it leaves each cycle lands on a different point of a small ring
/// around it instead of stacking on one spot.
const DROP_TURN: f32 = 0.9;
/// The Hantavirus plate: how often (seconds) the card lets the weapon drop. A
/// max-rank weapon's own cycle is 2.8 s, which would put a single dropping in a
/// five-second loop; the card is about the sequence (drop → step → infection →
/// jump), not about the charge a card's facts state.
const DROP_EVERY: f32 = 1.3;
/// The Hantavirus plate: how fast (px/s) its three fodder walk in on the
/// droppings, and how close to the rat they stop (a step away from the ring of
/// drops the rat is laying, so one of them is inside a 7 px dropping).
const DROP_WALK: f32 = 9.0;
const DROP_STOP: f32 = 12.0;

/// How much of a fitted plate the subject's footprint is allowed to take: the
/// rest is the margin that keeps a clipped edge impossible.
const FIT_MARGIN: f32 = 0.88;
/// The narrowest span a fitted stage assumes, so the plate does not start tight
/// on a body and then pull back when the tell opens.
const FIT_MIN_SPAN: Vec2 = Vec2 { x: 54.0, y: 38.0 };
/// The closest a fitted stage may zoom in (world pixels per plate pixel): the
/// travelling rings (a 42 px rim either side of the body is a 92 px footprint
/// on a 30 px plate) pin how much floor a plate may show, and a plate that
/// zoomed in past this would lose the body it is a portrait of.
///
/// There is no upper bound: a footprint larger than `zoom::MAX × plate` is drawn
/// oversized into a scratch buffer and box-filtered down (`Script::shrink`), so
/// a wide tell — or a boss's whole loop — can always be held.
const FIT_ZOOM_MIN: f32 = 0.55;

/// How much of a **boss** plate the body's loop-long union is allowed to take:
/// the task's 10 % margin (a boss is framed from the union of its drawn bounds
/// over a whole loop, so no state of the fight can leave the plate).
const BOSS_MARGIN: f32 = 0.90;
/// How far behind the head KING COBRA's **nearest coil** is framed. The laid
/// ring is the arena's breadth (74 px radius); the card is the hood, the neck
/// and the coil closest to them, not the whole loop.
const COBRA_COIL_VIEW: f32 = 58.0;

/// A body under this collision radius is a **tiny subject** on its card: at the
/// plate's 1:1 zoom it is a two- or three-pixel speck, so its plate is drawn at
/// 2 × ([`TINY_ZOOM`]) and its practice dummy stands [`TINY_DUMMY_GAP`] px off.
const TINY_BODY_RADIUS: f32 = 4.0;
/// The zoom a tiny body's card is drawn at: `0.5` shows half the world the
/// plate would at 1:1, i.e. every world pixel is 2 × 2 plate pixels.
const TINY_ZOOM: f32 = 0.5;
/// How far a tiny body's practice dummy stands from the subject's rest
/// position (px): the smallest gap the task allows, which leaves the pair
/// inside a 2 × plate.
const TINY_DUMMY_GAP: f32 = 15.0;
/// The margin a subject's lap is clamped inside by (px): the Type Slug's lap
/// used to run off the plate's edge.
const LAP_MARGIN: f32 = 6.0;

/// The arsenal targets' sandbox hp: high enough that no weapon kills one inside
/// a loop (a body a card re-spawns would read as a different target).
const ARSENAL_HP: f32 = 4000.0;
/// The Tail Thrash's and the Flywheel's targets: ×50 the ordinary sandbox hp.
/// A max-rank thrash kills a 4,000 hp fodder body on its **first** sweep, so
/// the card showed the arc and an empty floor for the rest of the loop.
const SWEEP_TARGET_HP: f32 = ARSENAL_HP * 50.0;
/// The charged Super Dash's travel in world px (58 px/s × 3.2 × 0.32 s), with a
/// little slack: the lane the dash card lays its targets along and frames.
const DASH_LANE: f32 = 62.0;
/// The window a floor card is framed on: 12 × 8 tiles around a furnished room.
const FLOOR_WINDOW: (i32, i32) = (12, 8);
/// How much world a floor card shows (world px per plate pixel). Two thirds of
/// the window, so the camera has room to pan across it and back.
const FLOOR_ZOOM: f32 = 2.0;

/// The Redtail's card loop: hover, one dive, the landing, hover again.
const HAWK_LOOP: f32 = 6.0;
/// When inside its loop the Redtail's card sends the bird down the one dive.
const HAWK_DIVE_AT: f32 = 3.7;

/// A shooter pellet's self-lit trail, nearest the pellet first: a 3 px streak
/// in a colour no floor and no pellet art draws, so a 1 px shot reads as a shot.
const PELLET_TRAIL: [Rgb; 3] = [(255, 246, 214), (255, 186, 120), (196, 104, 62)];

/// The chest card's open lid and its reward burst.
const CHEST_WOOD: Rgb = (158, 78, 34);
const CHEST_WOOD_HI: Rgb = (206, 118, 50);
const CHEST_MOUTH: Rgb = (40, 20, 12);
const CHEST_BRASS: Rgb = (255, 226, 102);
const CHEST_GLOW: Rgb = (255, 232, 128);
/// How long the chest's opening burst runs (seconds).
const BURST_SECS: f32 = 0.85;

/// How an entry's card moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A scene world running the real schedule.
    Sandbox,
    /// The still pose; the string says why the plate cannot move.
    Still(&'static str),
}

/// The plan for one registry row. Exhaustive: a new key cannot ship without
/// an answer here, and `every_registry_row_has_a_scene_or_a_reason` proves the
/// answer is real (a `Sandbox` row builds, a `Still` row does not).
pub fn plan(key: Key) -> Plan {
    match key {
        // The legacy boss body is the one kind the game never spawns: there is
        // nothing to run.
        Key::Enemy(EnemyKind::Boss) => Plan::Still("the legacy boss body never spawns"),
        Key::Enemy(_) | Key::Miniboss(_) | Key::Boss(_) => Plan::Sandbox,
        Key::Weapon(_) | Key::Evolution(_) => Plan::Sandbox,
        Key::Object(ObjectKind::Gem) => Plan::Still("a plate comparing the five gem tiers"),
        Key::Pickup(_) | Key::Object(_) | Key::Floor(_) => Plan::Sandbox,
        // A passive moves the plate only where its whole effect is legible
        // from one picture: the rat outrunning its pack, and loot crossing the
        // floor to reach it. Everything else is a number, and a number with no
        // reference on the plate is a lie dressed as a demonstration.
        Key::Passive(PassiveKind::MoveSpeed | PassiveKind::Magnet) => Plan::Sandbox,
        Key::Passive(PassiveKind::Damage | PassiveKind::FireRate | PassiveKind::LensCap) => {
            Plan::Still("a multiplier with no reference on the plate")
        }
        Key::Passive(PassiveKind::LongFuse) => {
            Plan::Still("a pool's lifetime: one plate shows the pool either way")
        }
        Key::Passive(PassiveKind::SpringHeel | PassiveKind::SecondScent) => {
            Plan::Still("a clock or a counter the plate cannot show")
        }
        Key::Passive(PassiveKind::Counterweight) => {
            Plan::Still("a shove: the body it moves looks the same being shoved")
        }
        Key::Passive(PassiveKind::MaxHp | PassiveKind::Regen | PassiveKind::Armor) => {
            Plan::Still("a health number: nothing on the floor changes")
        }
        Key::Splice(_) => Plan::Still("a stat trade: the shelf icon is the whole picture"),
        Key::Prop(_) => Plan::Still("furniture"),
    }
}

/// `RATRUN_LIBRARY_STILL=1` freezes every card to its still pose (captures and
/// tests that pin a plate).
pub fn still_from_env() -> bool {
    std::env::var("RATRUN_LIBRARY_STILL").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// One tick of scripted input.
#[derive(Clone, Copy, Default)]
struct Input {
    mv: Vec2,
    /// A dash press (an edge the script raises for one tick).
    dash: bool,
    /// The active-weapon press.
    active: bool,
    /// The dash input is held (Super Dash charge).
    hold: bool,
}

/// What the plate frames.
#[derive(Clone, Copy, Debug)]
enum Framing {
    /// Halfway between the subject and its aim point: the creature and the
    /// thing it is attacking, both on the plate.
    Pair,
    /// The rat (an arsenal plate, a pickup being collected).
    Rat,
    /// A fixed world point (a pack's crossing, a lap round an obstacle, a trap).
    Point(Vec2),
    /// A point drifting on a slow figure of eight around a fixed point (a
    /// floor's pan).
    Pan(Vec2),
}

/// A trickle of bodies walking in on the hidden rat.
#[derive(Clone, Copy)]
struct Feed {
    kind: EnemyKind,
    /// Seconds before the first body.
    first: f32,
    /// Seconds between bodies.
    every: f32,
    /// Spawn distance from the rat.
    ring: f32,
    hp: f32,
    speed: f32,
    /// A fixed approach direction from the rat (a trap wants its bodies to
    /// walk over it). `None` alternates sides, a little above and below.
    from: Option<Vec2>,
}

/// A scene's script: everything but the world.
#[derive(Clone, Copy)]
struct Script {
    loop_secs: f32,
    /// World pixels per plate pixel: how much floor the plate shows.
    zoom: f32,
    /// Draw `shrink`× oversized and box-filter down (the boss plates' fit).
    shrink: usize,
    framing: Framing,
    input: fn(f32) -> Input,
    feed: Option<Feed>,
    /// A per-tick hook for the one thing a script cannot say as input.
    cue: Option<fn(&mut World, Option<Entity>, u32)>,
    /// Draw the card's own practice target where the hidden rat stands.
    dummy: bool,
    /// Frame the plate on the subject's **live footprint** (body + tell) and
    /// take the zoom from its bounds, instead of the fixed zoom above.
    fit: bool,
    /// A fixture whose position explains the subject's behavior.
    fixture: Option<Vec2>,
    /// How far past the dummy the aim point is framed: a leading shot's pellet
    /// travels past its target, so the plate has to hold that far too.
    lead: f32,
    /// A [`Framing::Rat`] plate's offset from the rat: an arsenal plate leans
    /// towards its standing targets, so the rat, the targets and the hit
    /// digits above them all sit inside the frame instead of at its edge.
    lean: Vec2,
    /// A [`Framing::Pan`] plate's travel either side of its point (world px):
    /// a floor card pans the whole window it was cut from.
    pan: Vec2,
    /// Frame the plate on the **union of the boss's drawn body over one loop**
    /// ([`LibraryScene::probe_boss_union`]) instead of following it.
    boss_fit: bool,
    /// Draw the shooter pellet's short self-lit trail (a card-only overlay).
    trail: bool,
    /// Clamp the subject inside the plate by this margin (the Type Slug's lap).
    clamp_subject: Option<f32>,
    /// Hold the hidden rat at this point every tick, so a signature's knockback
    /// cannot walk the practice dummy off its mark (the Murder Hornet's card).
    pin: Option<Vec2>,
    /// The chest card's point: the lid opens, and the reward bursts, there.
    chest: Option<Vec2>,
    /// A wandering waypoint for the hidden rat (a floor card's fodder follows
    /// it through the window): `(centre, amplitude)` in world px.
    wander: Option<(Vec2, Vec2)>,
    /// The 12 × 8-tile window a floor card was cut from (for the pan and the
    /// card's own tests).
    window: Option<TileRect>,
}

fn idle(_: f32) -> Input {
    Input::default()
}

impl Script {
    fn new(loop_secs: f32, zoom: f32) -> Script {
        Script {
            loop_secs,
            zoom,
            shrink: 1,
            framing: Framing::Pair,
            input: idle,
            feed: None,
            cue: None,
            dummy: false,
            lean: Vec2::ZERO,
            pan: Vec2::new(30.0, 10.0),
            fit: false,
            fixture: None,
            lead: 0.0,
            boss_fit: false,
            trail: false,
            clamp_subject: None,
            pin: None,
            chest: None,
            wander: None,
            window: None,
        }
    }
}

/// A looping card scene. See the module docs.
pub struct LibraryScene {
    key: Key,
    world: World,
    script: Script,
    subject: Option<Entity>,
    /// Ticks into the current loop.
    tick: u32,
    /// Completed loops.
    loops: u32,
    acc: f32,
    held: bool,
    focus: Vec2,
    /// The world rectangle the last draw covered (for the framing rule).
    view: Vec2,
    /// Ticks stepped since the last draw.
    pending: u32,
    big: PixelBuffer,
    /// The bare floor of the plate, rendered the same way the world's own pass
    /// lays it down. Scratch, so a card draw allocates nothing after the first
    /// frame (see [`LibraryScene::floor_plate`]).
    floor_buf: PixelBuffer,
    /// The reference above at plate resolution (or its `shrink ×` intermediate).
    floor_mid: PixelBuffer,
    floor_plate: PixelBuffer,
    /// The widest subject footprint a fitted stage has had to hold this loop.
    fit_span: Vec2,
    /// The world rectangle a boss card must hold: the union of the boss's drawn
    /// body over one whole loop, measured once when the card is built.
    boss_union: Option<(Vec2, Vec2)>,
    /// Seconds since the chest card's chest opened (`None` before it does).
    opened: Option<f32>,
}

impl LibraryScene {
    /// Build the scene for `key`, or `None` when its card does not run one.
    pub fn build(key: Key) -> Option<LibraryScene> {
        if plan(key) != Plan::Sandbox {
            return None;
        }
        let (world, script, subject) = stage(key)?;
        let mut scene = LibraryScene {
            key,
            world,
            script,
            subject,
            tick: 0,
            loops: 0,
            acc: 0.0,
            held: false,
            focus: Vec2::ZERO,
            // The card's art plate is 44 × 30 until the first draw says
            // otherwise, so the opening frame is already framed for the lane.
            view: Vec2::new(44.0, 30.0) * (script.shrink.max(1) as f32 * script.zoom),
            pending: 1,
            big: PixelBuffer::new(1, 1),
            floor_buf: PixelBuffer::new(1, 1),
            floor_mid: PixelBuffer::new(1, 1),
            floor_plate: PixelBuffer::new(1, 1),
            fit_span: FIT_MIN_SPAN,
            boss_union: None,
            opened: None,
        };
        if scene.script.boss_fit {
            scene.boss_union = match key {
                Key::Boss(kind) => cached_boss_union(&mut scene, kind),
                _ => scene.probe_boss_union(),
            };
        }
        scene.focus = scene.focus_target();
        Some(scene)
    }

    pub fn key(&self) -> Key {
        self.key
    }

    /// Completed loops since the scene was built.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    /// Seconds into the current loop.
    pub fn clock(&self) -> f32 {
        self.tick as f32 * TICK
    }

    pub fn loop_secs(&self) -> f32 {
        self.script.loop_secs
    }

    /// The scene's world, for tests and captures.
    pub fn world(&mut self) -> &mut World {
        &mut self.world
    }

    /// The subject's entity, when the scene has one body it is about.
    pub fn subject(&self) -> Option<Entity> {
        self.subject
    }

    /// Has the picture moved on since the last [`draw`](Self::draw)? Two
    /// ticks are one 30 Hz menu frame.
    pub fn due(&self) -> bool {
        self.pending >= 2
    }

    /// Advance by wall time `dt`, in whole fixed steps.
    pub fn advance(&mut self, dt: f32) {
        self.acc += dt.clamp(0.0, 0.1);
        let mut steps = 0;
        while self.acc >= TICK && steps < MAX_STEPS {
            self.acc -= TICK;
            self.step();
            steps += 1;
        }
        if steps == MAX_STEPS {
            self.acc = 0.0;
        }
    }

    /// One fixed simulation step (and the loop's rebuild when it ends).
    pub fn step(&mut self) {
        let t = self.clock();
        if t >= self.script.loop_secs {
            self.restart();
            return;
        }
        if let Some(feed) = self.script.feed {
            self.feed(feed, t);
        }
        if let Some((centre, amp)) = self.script.wander {
            // A floor card's hidden rat drifts a slow figure of eight inside
            // the window it was cut from: the floor's two fodder kinds follow
            // it, which is what walks them through the picture.
            let a = t / self.script.loop_secs * std::f32::consts::TAU;
            self.world.player.pos =
                centre + Vec2::new(a.sin() * amp.x, (a * 2.0).sin() * amp.y);
        }
        if let Some(cue) = self.script.cue {
            cue(&mut self.world, self.subject, self.tick);
        }
        let input = (self.script.input)(t);
        let hold = DashHold {
            down: input.hold && !self.held,
            held: input.hold,
            up: !input.hold && self.held,
        };
        self.held = input.hold;
        self.world
            .update_with_dash_hold(TICK, input.mv, input.dash, input.active, None, hold);
        // The rat of a card takes its hits (the flash is part of the picture)
        // and never dies of them: `Player::god` is the guarantee, and the floor
        // below is the belt for anything that writes `hp` directly.
        let floor = self.world.player.max_hp() * 0.6;
        if self.world.player.hp < floor {
            self.world.player.hp = floor;
        }
        if let Some(pin) = self.script.pin {
            // A pinned dummy: the Murder Hornet's signature shoves a body, and
            // a card's target is a mark on the floor, not a thing to wind up
            // somewhere else.
            self.world.player.pos = pin;
            self.world.player.vel = Vec2::ZERO;
        }
        // No audio, and no plate: the card must never come back as a level-up,
        // a chest or the shelf. The queue is dropped on the floor.
        self.world.sfx.clear();
        self.world.pending_levelups = 0;
        if self.script.chest.is_some() && self.opened.is_none() && self.world.pending_chests > 0 {
            // The real collection happened on this tick (the chest is gone and
            // the plate was raised): the card starts its own open-and-burst,
            // and the pending flag is dropped below like every other plate.
            self.opened = Some(0.0);
        }
        if let Some(age) = self.opened.as_mut() {
            *age += TICK;
        }
        self.world.pending_chests = 0;
        self.world.pending_shop = false;
        self.tick += 1;
        self.pending = self.pending.saturating_add(1);
        // The camera eases toward what the plate frames. A fitted stage snaps
        // its own centre on every draw (the footprint it must hold can grow at
        // the tell), so easing here would fight it; so does a 2 x tiny-subject
        // plate, whose 22 px view is narrow enough that a lunge outran the ease.
        if !self.script.fit && !self.script.boss_fit {
            let target = self.focus_target();
            if self.script.zoom <= TINY_ZOOM {
                self.focus = target;
            } else {
                self.focus += (target - self.focus) * 0.12;
            }
        }
    }

    fn restart(&mut self) {
        let loops = self.loops + 1;
        let view = self.view;
        if let Some((world, script, subject)) = stage(self.key) {
            self.world = world;
            self.script = script;
            self.subject = subject;
        }
        self.tick = 0;
        self.loops = loops;
        self.held = false;
        self.view = view;
        self.focus = self.focus_target();
        self.fit_span = FIT_MIN_SPAN;
        // The measured boss union survives the rebuild (it is a property of the
        // loop, not of this instance of it); the chest's burst does not.
        self.opened = None;
        self.pending = self.pending.saturating_add(2);
    }

    fn feed(&mut self, feed: Feed, t: f32) {
        let since = t - feed.first;
        if since < 0.0 {
            return;
        }
        let n = (since / feed.every).floor();
        // Spawn on the tick that crosses each multiple of `every`.
        if since - n * feed.every >= TICK {
            return;
        }
        let n = n as i32;
        let pos = match feed.from {
            // A fixed approach with a little lateral jitter, so the bodies do
            // not walk a single line.
            Some(from) => {
                let perp = Vec2::new(-from.y, from.x);
                let jitter = [0.0, 7.0, -7.0][n.rem_euclid(3) as usize];
                self.world.player.pos + from * feed.ring + perp * jitter
            }
            // Alternate sides, a little above and below the rat's line.
            None => {
                let side = if n % 2 == 0 { 1.0 } else { -1.0 };
                let lift = [0.0, 0.35, -0.35][n.rem_euclid(3) as usize];
                let dir = Vec2::new(side, lift).normalized();
                self.world.player.pos + dir * feed.ring
            }
        };
        let e = self.world.spawn_enemy(feed.kind, pos, 1.0);
        if let Some(mut body) = self.world.ecs.get_mut::<Enemy>(e) {
            body.hp = feed.hp;
            body.max_hp = feed.hp;
            body.speed_scale = feed.speed;
        }
    }

    /// Where the subject stands, if the scene has one.
    pub fn subject_pos(&mut self) -> Option<Vec2> {
        if let Some(e) = self.subject {
            if let Some(body) = self.world.ecs.get::<Enemy>(e) {
                return Some(body.pos);
            }
        }
        let mut bosses = self.world.ecs.query::<&Boss>();
        bosses.iter(&self.world.ecs).next().map(|b| b.pos)
    }

    /// Is the subject on the plate the last [`draw`](Self::draw) covered? The
    /// card's contract is a portrait: the thing it is about must be in frame.
    pub fn subject_on_plate(&mut self) -> bool {
        let Some(p) = self.subject_pos() else {
            return true;
        };
        let cam = self.world.cam;
        let v = self.view;
        p.x >= cam.x && p.x <= cam.x + v.x && p.y >= cam.y && p.y <= cam.y + v.y
    }

    fn focus_target(&mut self) -> Vec2 {
        let rat = self.world.player.pos;
        match self.script.framing {
            Framing::Rat => rat + self.script.lean,
            Framing::Pair => {
                // A leading shot lands past its target, so a card that stages
                // one frames that far beyond the dummy as well (`Script::lead`).
                let lead = self.script.lead;
                let subject = self.subject_pos();
                let aim = if !self.script.dummy {
                    None
                } else {
                    Some(match subject {
                        Some(s) if lead > 0.0 && s.dist_sq(rat) > 1e-4 => {
                            rat + (rat - s).normalized() * lead
                        }
                        _ => rat,
                    })
                };
                match (subject, aim) {
                    (Some(s), Some(a)) => {
                        // Both ends of the lane, centred, whenever the plate is
                        // wide enough to hold them with a margin; otherwise the
                        // subject wins (it is what the card is about) and the
                        // aim point is allowed to slide off the edge. The
                        // margin is 4 % of the plate a side, which is what a
                        // shooter's 80 px stand-off needs from an 88 px field.
                        let d = a - s;
                        let fits =
                            d.x.abs() <= self.view.x * 0.92 && d.y.abs() <= self.view.y * 0.92;
                        if fits {
                            (s + a) * 0.5
                        } else {
                            let lean = Vec2::new(
                                d.x.clamp(-self.view.x * 0.22, self.view.x * 0.22),
                                d.y.clamp(-self.view.y * 0.2, self.view.y * 0.2),
                            );
                            s + lean
                        }
                    }
                    (Some(s), None) => s,
                    (None, Some(a)) => a,
                    (None, None) => rat,
                }
            }
            Framing::Point(p) => p,
            Framing::Pan(p) => {
                // A slow sweep across the plate's window and back: the x leg is
                // one full period a loop, the y leg two (a shallow figure of
                // eight, so a pan never retraces a straight line).
                let t = self.clock() / self.script.loop_secs * std::f32::consts::TAU;
                p + Vec2::new(
                    t.sin() * self.script.pan.x,
                    (t * 2.0).sin() * self.script.pan.y,
                )
            }
        }
    }

    /// Draw the scene into the card's art plate.
    pub fn draw(&mut self, art: &mut PixelBuffer) {
        self.pending = 0;
        let (w, h) = (art.w, art.h);
        if w == 0 || h == 0 {
            return;
        }
        let mut k = self.script.shrink.max(1);
        let mut zoom = self.script.zoom;
        if self.script.fit {
            // A named fight: hold the body and its live footprint, centred,
            // with the zoom coming from the shape's own bounds. The span only
            // ever grows inside a loop (the tell is the widest the footprint
            // gets), so the camera pulls back once and never pumps.
            if let Some((lo, hi)) = self.fit_bounds() {
                let span = hi - lo;
                self.fit_span = Vec2::new(
                    self.fit_span.x.max(span.x),
                    self.fit_span.y.max(span.y),
                );
                let (nk, nz) = fit_zoom(self.fit_span, w, h, FIT_MARGIN);
                k = nk;
                zoom = nz;
                self.script.shrink = k;
                // Centred on the midpoint of body + footprint (`fit_bounds`).
                self.focus = (lo + hi) * 0.5;
            }
        } else if let Some((lo, hi)) = self.boss_union {
            // A boss: the union of its drawn body over a whole loop, measured
            // when the card was built ([`LibraryScene::probe_boss_union`]). The
            // camera sits on the middle of that union and the downscale holds
            // it with a 10 % margin, so every state of the fight is in frame.
            let (nk, nz) = fit_zoom(hi - lo, w, h, BOSS_MARGIN);
            k = nk;
            zoom = nz;
            self.script.shrink = k;
            self.focus = (lo + hi) * 0.5;
        }
        self.view = Vec2::new((w * k) as f32 * zoom, (h * k) as f32 * zoom);
        if let Some(margin) = self.script.clamp_subject {
            // A subject whose own lap runs wide (the Type Slug) is held inside
            // the plate it is drawn on: the clamp is a property of the card's
            // frame, so it is applied where the frame is known.
            self.clamp_subject(margin);
        }
        self.world.zoom = zoom;
        self.world.zoom_view = zoom;
        self.world.cam_focus = Some(self.focus);
        if k == 1 {
            self.world.draw(art);
        } else {
            self.big.resize(w * k, h * k);
            self.world.draw(&mut self.big);
            box_down(&self.big, art, k);
        }
        if self.script.dummy {
            // The practice target stands exactly where the hidden rat stands:
            // that is the point every attack on the plate is aimed at. Painted
            // by the card and not by the simulation — no entity, no hit shape,
            // nothing to kill — and only on the pixels the world's pass left as
            // bare floor, so it sits *under* every body, tell and pellet.
            let scale = (self.script.shrink.max(1) as f32) * zoom;
            let cam = self.world.cam;
            let p = self.world.player.pos;
            let sx = ((p.x - cam.x) / scale).round() as i32;
            let sy = ((p.y - cam.y) / scale).round() as i32;
            let r = ((DUMMY_R / scale).round() as i32).max(2);
            // The floor reference is only needed where the cork can land.
            let bare = self.floor_plate(w, h);
            draw_practice_dummy(art, bare, (sx, sy), r);
        }
        if self.script.trail {
            self.pellet_trails(art);
        }
        if let (Some(age), Some(chest)) = (self.opened, self.script.chest) {
            let scale = (self.script.shrink.max(1) as f32) * zoom;
            let cam = self.world.cam;
            let s = (
                ((chest.x - cam.x) / scale).round() as i32,
                ((chest.y - cam.y) / scale).round() as i32,
            );
            draw_open_chest(art, s, age);
        }
    }

    /// The plate's bare floor, laid down exactly the way [`World::draw`] lays
    /// it (the same world buffer size, the same resample, the same box filter
    /// at a boss plate's `shrink`), so a pixel that matches it is a pixel no
    /// object, body, tell or pellet touched.
    ///
    /// This is what lets the card's dummy be painted *under* the picture
    /// without a hook into the mid-scene layers of `World::draw` (which this
    /// module does not own): the cork only replaces floor.
    fn floor_plate(&mut self, w: usize, h: usize) -> &PixelBuffer {
        let k = self.script.shrink.max(1);
        let zoom = self.view.x / (w * k) as f32;
        let ww = ((w * k) as f32 * zoom).round().max(1.0) as usize;
        let wh = ((h * k) as f32 * zoom).round().max(1.0) as usize;
        self.floor_buf.resize(ww, wh);
        // The first thing `World::draw_scene` does: clear to the theme's bg and
        // paint the floor (the scene is flat-lit, so no light touches it).
        let bg = self.world.level.palette.bg;
        self.floor_buf.clear(bg);
        crate::game::art_floor::draw_floor(
            &mut self.floor_buf,
            &self.world.level,
            self.world.cam,
            self.world.elapsed,
        );
        let level = &self.world.level;
        if level.theme == Theme::Outfall && self.world.flat_lit {
            for puddle in &level.puddles {
                crate::game::art_puddle::draw_puddle_among_with(
                    &mut self.floor_buf,
                    puddle,
                    &level.puddles,
                    self.world.cam,
                    self.world.elapsed,
                    level.theme,
                    false,
                );
            }
        }
        self.floor_plate.resize(w, h);
        if k == 1 {
            self.floor_buf.resample_to(&mut self.floor_plate);
        } else {
            self.floor_mid.resize(w * k, h * k);
            self.floor_buf.resample_to(&mut self.floor_mid);
            box_down(&self.floor_mid, &mut self.floor_plate, k);
        }
        &self.floor_plate
    }

    /// The world rectangle a fitted plate must hold: the subject's drawn body,
    /// its live telegraph (the widest the footprint gets) and the aim point.
    fn fit_bounds(&self) -> Option<(Vec2, Vec2)> {
        let e = self.subject?;
        if let Some(boss) = self.world.ecs.get::<Boss>(e) {
            let (lo, hi) = boss_body_bounds(boss);
            let mut bounds = Bounds::new(lo);
            bounds.grow(lo, hi);
            bounds.point(self.world.player.pos, DUMMY_R + 2.0);
            if let Some(fixture) = self.script.fixture {
                bounds.point(fixture, 5.0);
            }
            return Some(bounds.get());
        }
        let body = self.world.ecs.get::<Enemy>(e)?;
        let (pos, radius) = (body.pos, body.radius);
        // A named body's sprite is 2.5-3 × its collision radius to the sides and
        // **taller than that upwards**: the art draws a 30-34 px animal from a
        // 7 px hit disc (a stamp hoisted on its plunger, a carton on its feet),
        // and a symmetric 3 × disc cut the top of the first frame.
        let mut bounds = Bounds::new(pos);
        if let Some(mb) = self.world.ecs.get::<Miniboss>(e) {
            let side = (radius * 3.0).max(6.0);
            let up = (radius * 4.6).max(10.0);
            let down = (radius * 1.8).max(6.0);
            bounds.grow(pos - Vec2::new(side, up), pos + Vec2::new(side, down));
            if let Some(shape) = mb.telegraph() {
                let (lo, hi) = shape_bounds(shape);
                bounds.grow(lo, hi);
            }
        } else {
            bounds.point(pos, radius + 5.0);
            if let Some(tell) = outfall_enemy_tell(body) {
                bounds.hazard(tell.shape);
            }
            if let Some(bulb) = lampfish_bulb(body) {
                bounds.point(bulb, 5.0);
            }
        }
        if let Some(fixture) = self.script.fixture {
            bounds.point(fixture, 5.0);
        }
        // The target of the attack is part of the picture.
        bounds.point(self.world.player.pos, 0.0);
        Some(bounds.get())
    }

    /// Measure the union of the boss's drawn body over **one whole loop**, by
    /// running that loop once when the card is built.
    ///
    /// A boss plate used to follow the fight with an easing camera, which is
    /// what clipped the Sweeper's and SUBJECT R-01's tops, cut the Night
    /// Edition's plinth off the bottom and lost the King Cobra almost entirely:
    /// every controller's body leaves a fixed frame sooner or later. The union
    /// of the frames it *does* occupy cannot. The probe ends on the loop's
    /// boundary (`step` restarts the scene there), so the card's first drawn
    /// loop starts from a fresh world and is the same loop that was measured.
    ///
    /// The scene has its own seed and no director, so the probe is a fixture:
    /// it cannot reach a run, a golden or the map pin.
    pub(super) fn probe_boss_union(&mut self) -> Option<(Vec2, Vec2)> {
        let ticks = (self.script.loop_secs / TICK).ceil() as u32 + 2;
        let loops = self.loops;
        let mut union: Option<Bounds> = None;
        for _ in 0..ticks {
            self.record_boss_bounds(&mut union);
            self.step();
            if self.loops != loops {
                break;
            }
        }
        union.map(Bounds::get)
    }

    /// Fold the boss's drawn body (and the dummy it is attacking) into
    /// `union`, if the fight has started.
    fn record_boss_bounds(&mut self, union: &mut Option<Bounds>) {
        // The dummy is on the other end of every attack: a plate that framed
        // the body alone could lose the mark it is aimed at. Copy it out before
        // the ECS query borrows the world's ECS field.
        let aim = self.world.player.pos;
        let mut q = self.world.ecs.query::<&Boss>();
        for boss in q.iter(&self.world.ecs) {
            let (lo, hi) = boss_body_bounds(boss);
            union.get_or_insert_with(|| Bounds::new(lo)).grow(lo, hi);
            union
                .get_or_insert_with(|| Bounds::new(aim))
                .point(aim, DUMMY_R + 2.0);
        }
    }

    /// Hold the subject inside the plate the last [`draw`](Self::draw) set up
    /// (the Type Slug's lap), `margin` px in from every edge.
    fn clamp_subject(&mut self, margin: f32) {
        let Some(e) = self.subject else {
            return;
        };
        let half = self.view * 0.5 - Vec2::splat(margin);
        if half.x <= 0.0 || half.y <= 0.0 {
            return;
        }
        let lo = self.focus - half;
        let hi = self.focus + half;
        if let Some(mut body) = self.world.ecs.get_mut::<Enemy>(e) {
            body.pos = body.pos.clamp_box(lo, hi);
        }
    }

    /// A shooter pellet's short self-lit trail: the ink pass keeps a hostile
    /// mark at a minimum size, but a 1-2 px pellet still read as a dot. Drawn
    /// by the card, over the picture, and never by the simulation.
    fn pellet_trails(&mut self, art: &mut PixelBuffer) {
        if art.w == 0 || art.h == 0 {
            return;
        }
        let scale = self.view.x / art.w as f32;
        if scale <= 0.0 {
            return;
        }
        let cam = self.world.cam;
        let mut q = self.world.ecs.query::<&Bullet>();
        for b in q.iter(&self.world.ecs) {
            if b.faction != Faction::Enemy {
                continue;
            }
            let dir = b.vel.normalized();
            if dir.len_sq() < 0.5 {
                continue;
            }
            for (i, c) in PELLET_TRAIL.iter().enumerate() {
                let p = b.pos - dir * (i as f32 + 1.0);
                let x = ((p.x - cam.x) / scale).round() as i32;
                let y = ((p.y - cam.y) / scale).round() as i32;
                if x >= 0 && y >= 0 && (x as usize) < art.w && (y as usize) < art.h {
                    art.plot(x, y, *c);
                }
            }
        }
    }
}

/// `(k, zoom)` for a plate that must hold `span` world px: the downscale
/// `World::draw` honours (its zoom ladder stops at 2.0) and the zoom that goes
/// with it, keeping `margin` of the plate free.
fn fit_zoom(span: Vec2, w: usize, h: usize, margin: f32) -> (usize, f32) {
    let total = (span.x / (w as f32 * margin))
        .max(span.y / (h as f32 * margin))
        .max(FIT_ZOOM_MIN);
    // `World::draw` honours a zoom only inside the ladder (`zoom::clamp`), so a
    // plate that must show more than the ladder's top shows is drawn oversized
    // and box-filtered down by `k`.
    let k = ((total / crate::render::zoom::MAX).ceil() as usize).max(1);
    (k, total / k as f32)
}

/// A growing world rectangle (see [`boss_body_bounds`]).
#[derive(Clone, Copy)]
struct Bounds {
    lo: Vec2,
    hi: Vec2,
    any: bool,
}

impl Bounds {
    fn new(at: Vec2) -> Bounds {
        Bounds {
            lo: at,
            hi: at,
            any: false,
        }
    }

    fn grow(&mut self, lo: Vec2, hi: Vec2) {
        self.lo = Vec2::new(self.lo.x.min(lo.x), self.lo.y.min(lo.y));
        self.hi = Vec2::new(self.hi.x.max(hi.x), self.hi.y.max(hi.y));
        self.any = true;
    }

    fn point(&mut self, at: Vec2, r: f32) {
        self.grow(at - Vec2::splat(r), at + Vec2::splat(r));
    }

    fn hazard(&mut self, shape: HazardShape) {
        match shape {
            HazardShape::Circle { center, radius } => self.point(center, radius),
            HazardShape::Annulus { center, outer, .. } => self.point(center, outer),
            HazardShape::Arc { center, outer, facing, half_angle, .. } => {
                let dir = facing_of(facing);
                self.point(center, 0.0);
                for a in [-half_angle, 0.0, half_angle] {
                    self.point(center + dir.rotate(a) * outer, 0.0);
                }
                for axis in [Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)] {
                    for cardinal in [axis, -axis] {
                        if cardinal.dot(dir) >= half_angle.cos() {
                            self.point(center + cardinal * outer, 0.0);
                        }
                    }
                }
            }
            HazardShape::Rect { center, facing, half, half_perp } => {
                let dir = facing_of(facing);
                for along in [-half, half] {
                    for across in [-half_perp, half_perp] {
                        self.point(center + dir * along + dir.perp() * across, 0.0);
                    }
                }
            }
            HazardShape::Corridor { from, to, half_width } => {
                self.point(from, half_width);
                self.point(to, half_width);
            }
        }
    }

    fn get(self) -> (Vec2, Vec2) {
        (self.lo, self.hi)
    }
}

/// The world rectangle a boss's **drawn body** occupies right now: its art's
/// own hit shapes, plus the overshoot each art module documents past them (the
/// cat's tail, the broom's rising trousers, the crab's telson, the hawk's
/// wings and its hover lift, the cobra's nearest coil).
///
/// A hit-shape bound is all a plate needs to keep the body *on* the plate; the
/// overshoot is what makes it a portrait rather than a crop.
fn boss_body_bounds(boss: &Boss) -> (Vec2, Vec2) {
    let mut b = Bounds::new(boss.pos);
    for shape in boss.hit_shapes().iter() {
        match *shape {
            BossHitShape::Circle { center, radius, .. } => b.point(center, radius),
            BossHitShape::Box {
                center,
                facing,
                half,
                half_perp,
                ..
            } => {
                let f = if facing.len_sq() > 1e-6 {
                    facing.normalized()
                } else {
                    Vec2::new(1.0, 0.0)
                };
                let p = f.perp();
                let ex = half * f.x.abs() + half_perp * p.x.abs();
                let ey = half * f.y.abs() + half_perp * p.y.abs();
                b.grow(
                    center - Vec2::new(ex, ey),
                    center + Vec2::new(ex, ey),
                );
            }
        }
    }
    match boss.view() {
        BossArtView::Cat(v) => {
            // The banded tail trails a third of a body behind the rump.
            let f = facing_of(v.facing);
            b.point(
                v.pos - f * art_boss_cat::TAIL_LENGTH * 0.6,
                art_boss_cat::TAIL_LENGTH * 0.22,
            );
        }
        BossArtView::Broom(v) => {
            // The Sweeper is legs: the trouser legs rise ~30 px above the
            // slippers (`draw_sock_and_trouser`), and the broom head is out at
            // its own jab reach. Both were cut off the top of the plate.
            b.point(
                v.pos - Vec2::new(0.0, 30.0),
                art_boss_broom::STANCE_HALF_WIDTH + 4.0,
            );
            b.point(
                art_boss_broom::broom_head_pos(&v),
                art_boss_broom::JAB_HEAD_RADIUS,
            );
        }
        BossArtView::Mower(v) => {
            // Deck, bumper and the hazard-striped wrap.
            b.point(v.pos, art_boss_mower::BODY_RADIUS + 12.0);
        }
        BossArtView::Mangle(v) => {
            // The frame box plus the loose crank (the hit shapes) and the
            // roller cheeks' own thickness.
            b.point(
                v.pos,
                crate::game::art_boss_mangle::ROLLER_HALF_WIDTH
                    + crate::game::art_boss_mangle::BODY_HALF_LEN,
            );
        }
        BossArtView::NightEdition(v) => {
            // The slab is a hit box; the ribbon spool stands off its flank.
            b.point(
                art_boss_night_edition::ribbon_pos(v.pos, v.facing),
                art_boss_night_edition::RIBBON_WINDOW_RADIUS,
            );
        }
        BossArtView::Magpie(v) => {
            b.point(art_boss_magpie::beak_tip(&v), 4.0);
        }
        BossArtView::HorseshoeCrab(v) => {
            // The telson is longer than the dome's hit box.
            b.point(art_boss_horseshoe_crab::telson_tip(&v), 5.0);
        }
        BossArtView::MegaRat(v) => {
            let f = facing_of(v.facing);
            b.point(
                v.pos - f * (art_boss_megarat::TAIL_OFFSET + art_boss_megarat::TAIL_RADIUS),
                8.0,
            );
        }
        BossArtView::Hawk(v) => {
            // The bird is drawn above its ground point by its altitude; the
            // overhead lift is allowed to leave the plate (the shadow is still
            // the boss there), but the hover height is part of the body.
            let lift = art_boss_hawk::altitude(v.state, v.phase, v.progress)
                .min(art_boss_hawk::HOVER_LIFT + 6.0);
            b.point(
                v.pos - Vec2::new(0.0, lift),
                art_boss_hawk::WING_SPAN * 0.5,
            );
        }
        BossArtView::Cobra(v) => {
            // KING COBRA: the head, the hood (the hit circles) and the
            // **nearest coil** — never the whole laid ring, whose radius is the
            // arena's breadth.
            let head = v.pos;
            let mut chain = [Vec2::ZERO; 96];
            let n = art_boss_cobra::draw_chain(&v, &mut chain);
            for p in chain.iter().take(n) {
                if p.dist(head) <= COBRA_COIL_VIEW {
                    b.point(*p, art_boss_cobra::COIL_BODY_HALF + 3.0);
                }
            }
        }
        BossArtView::Lantern(v) => {
            b.point(v.pos, art_boss_lantern::HEAD_RADIUS);
            if v.state != LanternState::ArrivalLure {
                let (lo, hi) = art_boss_lantern::body_bounds(&v);
                b.grow(lo, hi);
            }
            for bulb in [v.real_bulb, v.decoy].into_iter().flatten() {
                b.point(bulb.pos, 6.0 * art_boss_lantern::BODY_SCALE);
            }
            for shape in [v.attack, v.stamp].into_iter().flatten() {
                b.hazard(shape);
            }
        }
    }
    if !b.any {
        // The overhead hawk has no hit shape at all: frame its ground point
        // with the body's own size so a shadow still has a body over it.
        b.point(boss.pos, boss.kind.body_radius() * 2.5);
    }
    b.get()
}

/// A unit facing vector, defensively (a view's facing may be unset).
fn facing_of(facing: Vec2) -> Vec2 {
    if facing.len_sq() > 1e-6 {
        facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// Box-filter `src` down by `k` into `dst` (a boss plate's oversized draw).
fn box_down(src: &PixelBuffer, dst: &mut PixelBuffer, k: usize) {
    let (w, h) = (dst.w, dst.h);
    let n = (k * k) as u32;
    let (bw, px) = (src.w, src.pixels());
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for yy in 0..k {
                let row = (y * k + yy) * bw + x * k;
                for p in &px[row..row + k] {
                    r += p.0 as u32;
                    g += p.1 as u32;
                    b += p.2 as u32;
                }
            }
            dst.plot(x as i32, y as i32, ((r / n) as u8, (g / n) as u8, (b / n) as u8));
        }
    }
}

// ---- Stages ----------------------------------------------------------------

/// A patch of open floor: `cols × rows` tiles of the theme's ground, a solid
/// border (so a scripted rat cannot walk off the map), no rooms, no props,
/// nothing placed.
///
/// The flatbed is the exception: its floor art (and its slipstream and its
/// tailgate) is anchored to the *map's* own deck rectangle, so a card about a
/// Flatbed body has to stand on that bed — see [`deck_patch`].
fn patch(theme: Theme, cols: i32, rows: i32) -> Level {
    if theme == Theme::Trailer {
        return deck_patch();
    }
    let mut tiles = TileMap::new(cols, rows, theme, 0);
    for x in 0..cols {
        tiles.set(x, 0, TileKind::Wall(WallKind::Solid));
        tiles.set(x, rows - 1, TileKind::Wall(WallKind::Solid));
    }
    for y in 0..rows {
        tiles.set(0, y, TileKind::Wall(WallKind::Solid));
        tiles.set(cols - 1, y, TileKind::Wall(WallKind::Solid));
    }
    tiles.rebuild_masks();
    let nav = NavGrid::from_tiles(&tiles, SizeClass::Large);
    let arena = Vec2::new(cols as f32 * 16.0, rows as f32 * 16.0);
    Level {
        theme,
        seed: 0,
        arena,
        tiles,
        props: Vec::new(),
        palette: Palette::for_theme(theme),
        nav,
        rooms: Vec::new(),
        start: arena * 0.5,
        mazes: Vec::new(),
        objects: LevelObjects::default(),
        boss: BossArena::default(),
        trailer: TrailerDeck::default(),
        puddles: Vec::new(),
        plants: Vec::new(),
    }
}

/// The flatbed's own bed, at the deck's real place on the map (the deck art,
/// the slipstream and the tailgate all read the map's deck rectangle rather
/// than the camera, so a card on a patch at the origin showed the verge and a
/// Flatbed body stood on plain green).
///
/// The bed is the only open floor; the rails and the road are solid, as they
/// are for the simulation on the real generator's map. The card's start sits a
/// short way in from the tailgate, so the Gravel Tick's drift (`tick_drift`,
/// tailward along `-x`) reads on the plate.
fn deck_patch() -> Level {
    use crate::game::levelgen::trailer::{
        DECK_LEFT, DECK_RIGHT, DECK_ROWS, DECK_TOP, TAILGATE_CELL,
    };
    let (cols, rows) = (200, 160);
    let mut tiles = TileMap::new(cols, rows, Theme::Trailer, 0);
    for y in 0..rows {
        for x in 0..cols {
            let on_deck = (DECK_LEFT..DECK_RIGHT).contains(&x)
                && (DECK_TOP..DECK_TOP + DECK_ROWS).contains(&y);
            if !on_deck {
                tiles.set(x, y, TileKind::Wall(WallKind::Solid));
            }
        }
    }
    tiles.rebuild_masks();
    let nav = NavGrid::from_tiles(&tiles, SizeClass::Large);
    let arena = Vec2::new(cols as f32 * 16.0, rows as f32 * 16.0);
    let start = cell_centre((DECK_LEFT + 24, DECK_TOP + DECK_ROWS / 2));
    Level {
        theme: Theme::Trailer,
        seed: 0,
        arena,
        tiles,
        props: Vec::new(),
        palette: Palette::for_theme(Theme::Trailer),
        nav,
        rooms: Vec::new(),
        start,
        mazes: Vec::new(),
        objects: LevelObjects::default(),
        boss: BossArena::default(),
        // The bed the floor art draws and `systems::trailer` reads: the full
        // deck strip, its tailgate (the slipstream's tailward direction) and
        // no hazards of the generator's own (a card places its feature).
        trailer: TrailerDeck {
            deck: TileRect::new(DECK_LEFT, DECK_TOP, DECK_RIGHT - DECK_LEFT, DECK_ROWS),
            tailgate: cell_centre(TAILGATE_CELL),
            pits: Vec::new(),
            wind: Vec::new(),
            slides: Vec::new(),
        },
        puddles: Vec::new(),
        plants: Vec::new(),
    }
}

/// A patch with a solid `w × h`-cell block of wall at cell `(cx, cy)`: the
/// obstacle a chaser laps round (a pipe end).
fn blocked(mut level: Level, cx: i32, cy: i32, w: i32, h: i32) -> Level {
    for y in cy..cy + h {
        for x in cx..cx + w {
            level.tiles.set(x, y, TileKind::Wall(WallKind::Solid));
        }
    }
    level.tiles.rebuild_masks();
    level.nav = NavGrid::from_tiles(&level.tiles, SizeClass::Large);
    level
}

/// A patch with a solid wall down column `wall_x` and one cell of `gap` in it
/// — a rat-hole, a doorway or a one-way door, the only way through.
fn walled(mut level: Level, wall_x: i32, gap_y: i32, gap: TileKind) -> Level {
    let (cols, rows) = (level.tiles.cols(), level.tiles.rows());
    assert!(wall_x > 1 && wall_x < cols - 2, "the wall must be on the patch");
    assert!(gap_y > 1 && gap_y < rows - 2, "the gap must be on the wall");
    for y in 1..rows - 1 {
        level.tiles.set(wall_x, y, TileKind::Wall(WallKind::Solid));
    }
    level.tiles.set(wall_x, gap_y, gap);
    level.tiles.rebuild_masks();
    level.nav = NavGrid::from_tiles(&level.tiles, SizeClass::Large);
    level
}

/// The centre of a cell, in world pixels.
fn cell_centre(cell: (i32, i32)) -> Vec2 {
    TileMap::tile_rect(cell.0, cell.1).center()
}

/// A working sconce mounted on the south face of a solid tile.
fn outfall_card_sconce(level: &mut Level, wall: (i32, i32)) -> Vec2 {
    let pos = cell_centre(wall) + Vec2::new(0.0, 8.5);
    level.objects.lights.push(LightFixture {
        pos,
        kind: LightFixtureKind::Sconce,
        radius: 110.0,
        seed: 1,
    });
    pos
}

/// A scene world over `level`: no director, an evenly lit plate, an unarmed
/// rat that cannot be hurt.
fn sandbox(level: Level) -> World {
    let mut loadout = Loadout::starting();
    loadout.weapons.clear();
    let mut world = World::with_level(0, level, SEED, loadout, CampaignTotals::default());
    world.peace = true;
    // A card is a portrait: the pitch-dark ambient of a real floor hides the
    // very thing the card is about. Every plate is flat-lit, and the hostile
    // layer (tells, footprints) stays self-lit above it as always.
    world.flat_lit = true;
    // And nothing hangs between the camera and the subject: the foreground
    // layer's big overhead silhouettes (and the Flatbed's sky movers, which
    // need no placement) would cross the plate and the practice dummy.
    world.foreground = false;
    // `god` is what makes "the card rat never dies" a property of the game
    // rather than a hope: nothing writes `player.hp` directly.
    world.player.god = true;
    world.viewport = Vec2::new(88.0, 60.0);
    world
}

/// Hide the card's rat (render-only).
///
/// The simulation keeps it — invulnerable, and stationary or on a scripted
/// waypoint — because steering, ranges and every attack need a target; the
/// plate shows the subject and the card's own practice dummy instead.
fn hide_rat(world: &mut World) {
    world.hide_rat = true;
}

/// Stack a passive on the card rat (the real `apply_passive_stack`, through
/// the loadout's own rebuild).
fn stack_passive(world: &mut World, kind: PassiveKind, stacks: u32) {
    let stacks = stacks.min(kind.max_stacks()).max(1);
    match world
        .player
        .loadout
        .passives
        .iter_mut()
        .find(|(p, _)| *p == kind)
    {
        Some((_, held)) => *held = (*held + stacks).min(kind.max_stacks()),
        None => world.player.loadout.passives.push((kind, stacks)),
    }
    world.player.loadout.rebuild_stats();
}

type Staged = (World, Script, Option<Entity>);

fn stage(key: Key) -> Option<Staged> {
    let theme = library::theme_of(key);
    match key {
        // A body is staged on the floor that fields it: the roster slot (which
        // picks the family) and the per-floor behaviour systems (each a no-op
        // off its own floor) are both keyed on that floor, not on the card's
        // index theme.
        Key::Enemy(kind) => Some(stage_enemy(home_floor(kind), kind, false)),
        Key::Miniboss(kind) => Some(stage_miniboss(miniboss_floor(kind), kind)),
        Key::Boss(kind) => Some(stage_boss(theme, kind)),
        Key::Weapon(kind) => Some(stage_weapon(theme, kind, false)),
        Key::Evolution(kind) => Some(stage_weapon(theme, kind, true)),
        Key::Pickup(kind) => Some(stage_pickup(theme, kind)),
        Key::Floor(theme) => Some(stage_floor(theme)),
        Key::Passive(kind) => stage_passive(theme, kind),
        Key::Object(kind) => stage_object(theme, kind),
        // `plan` answers `Still` for everything below, so `build` never gets
        // here; a caller that skipped the plan gets nothing rather than a
        // wrong picture.
        Key::Splice(_) | Key::Prop(_) => None,
    }
}

// ---- Bestiary: the six families -------------------------------------------

/// The floor whose roster fields `kind` (no kind is on two floors). A kind on
/// no floor (the legacy boss body, which has no scene) answers the Sewer.
fn home_floor(kind: EnemyKind) -> Theme {
    Theme::playable()
        .into_iter()
        .find(|t| roster::floor(*t).slot_of(kind).is_some())
        .unwrap_or(Theme::Sewer)
}

/// The floor whose appointments name `kind`.
fn miniboss_floor(kind: MinibossKind) -> Theme {
    Theme::playable()
        .into_iter()
        .find(|t| roster::appointments(*t).contains(&kind))
        .unwrap_or(Theme::Sewer)
}

/// A bestiary plate, staged by the body's roster slot on its own floor: what
/// the kind *is* decides what the card shows.
fn stage_enemy(theme: Theme, kind: EnemyKind, elite: bool) -> Staged {
    if elite {
        return stage_heavy(theme, kind, true);
    }
    let Some(slot) = roster::floor(theme).slot_of(kind) else {
        // A **guest** is in no roster slot (`EnemyKind::Drake`): it is a
        // tell-and-strike body, so it takes the trick staging — its own lane
        // on the floor and a peck volley at the card's dummy. The smallest
        // change: no new family, and nothing else can reach this arm.
        return stage_trick(theme, kind);
    };
    match slot {
        // Fodder: a pack, crossing the plate.
        Slot::A | Slot::B => stage_pack(theme, kind),
        // Chasers: one body, one lap, one obstacle.
        Slot::C => stage_chaser(theme, kind),
        // Shooters: a stand-off body and its whole flight.
        Slot::R => stage_shooter(theme, kind),
        // Tricks: the body's own tell → strike → recovery.
        Slot::I | Slot::II | Slot::III => stage_trick(theme, kind),
        // Heavies: the body lumbering in past a size reference.
        Slot::H => stage_heavy(theme, kind, false),
    }
}

/// Ground and flying fodder: three to five bodies crossing the plate in their
/// own formation, chasing a hidden waypoint that runs from one edge to the
/// other and back once a loop. There is no dummy: a pack has nothing to
/// attack, and the plate is about the gait (a file that holds its heading, a
/// weave that flips, a rank that stalls).
fn stage_pack(theme: Theme, kind: EnemyKind) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    hide_rat(&mut world);
    let centre = world.level.start;
    let count = pack_size(kind);
    let mut lead = None;
    for i in 0..count {
        let e = world.spawn_enemy(kind, centre + pack_slot(kind, i, count), 2.0);
        if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
            // A card is not a fight: nothing aboard a card's plate can kill the
            // subject, and a pack that thinned out would change the picture.
            body.hp = 400.0;
            body.max_hp = 400.0;
        }
        if lead.is_none() {
            lead = Some(e);
        }
    }
    let mut script = Script::new(PACK_LOOP, 1.25);
    script.framing = Framing::Point(centre);
    script.cue = Some(pack_waypoint);
    (world, script, lead)
}

/// How many bodies a pack card puts on the plate (three to five).
fn pack_size(kind: EnemyKind) -> usize {
    match kind.archetype() {
        // A weave needs room to weave; four flies read as a swarm.
        EnemyKind::Bat => 4,
        _ => 5,
    }
}

/// One body's place in the pack's opening formation.
///
/// The formation is the kind's own: walkers start in a file (a Crumb Ant holds
/// its heading, so a file arrives as a file) and flyers in a loose V (their
/// weave then crosses the plate twice per body).
fn pack_slot(kind: EnemyKind, i: usize, n: usize) -> Vec2 {
    let k = i as f32 - (n as f32 - 1.0) * 0.5;
    let t = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.5 };
    match kind.archetype() {
        EnemyKind::Bat => Vec2::new(k * 8.0, if i.is_multiple_of(2) { -6.0 } else { 6.0 }),
        _ => Vec2::new(-14.0 + k * 2.0, (t - 0.5) * 22.0),
    }
}

/// The pack's hidden rat: a waypoint that runs the plate edge to edge and back,
/// once a loop, so the pack streams across, turns and comes back. Nothing is
/// drawn at it.
fn pack_waypoint(world: &mut World, _subject: Option<Entity>, tick: u32) {
    let t = tick as f32 * TICK;
    let c = world.level.start;
    let phase = (t / (PACK_LOOP * 0.5)).rem_euclid(2.0);
    let tri = if phase <= 1.0 { phase } else { 2.0 - phase };
    let x = -PACK_REACH + 2.0 * PACK_REACH * tri;
    let y = (t * 1.7).sin() * 3.5;
    world.player.pos = c + Vec2::new(x, y);
}

/// Which obstacle a chaser laps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lap {
    /// A pipe end: the body has to swing round the stub, which is where a
    /// turning-rate rule shows (the Pipe Eel's wide swing, the Type Slug's
    /// held line, the centipede's wall hug).
    Pipe,
    /// A rat-hole wall: the rat crosses through the hole and a Cat-lineage
    /// body presses at a gap it cannot fit.
    Hole,
}

fn lap_of(kind: EnemyKind) -> Lap {
    match kind {
        // The eel's own rule is its half turning rate round a pipe end, and the
        // slug's is a ruled line past an obstacle: both need the stub.
        EnemyKind::PipeEel | EnemyKind::TypeSlug | EnemyKind::Conger => Lap::Pipe,
        // Every other Cat-lineage chaser is the one stopped by a rat-hole.
        kind if kind.archetype() == EnemyKind::Cat => Lap::Hole,
        _ => Lap::Pipe,
    }
}

/// One chaser body running a lap round an obstacle that shows its rule. No
/// dummy: the body never reaches the rat, and the obstacle is the subject.
fn stage_chaser(theme: Theme, kind: EnemyKind) -> Staged {
    let lap = lap_of(kind);
    let base = patch(theme, 40, 30);
    // The flatbed's bed is only 26 rows tall and sits at its own place on the
    // map, so the card's obstacle is carved relative to the card's start
    // rather than at a fixed cell of a hand-sized patch.
    let start_cell = crate::game::collision::cell_of(base.start);
    let (pipe_cell, gap_cell) = if theme == Theme::Trailer {
        (
            (start_cell.0 - 4, start_cell.1),
            (start_cell.0 + 4, start_cell.1),
        )
    } else {
        ((19, 14), (20, 15))
    };
    let gap = cell_centre(gap_cell);
    let pipe = cell_centre(pipe_cell) + Vec2::new(0.0, 8.0);
    let centre = match lap {
        Lap::Pipe => pipe,
        Lap::Hole => gap,
    };
    let mut level = match lap {
        Lap::Pipe => blocked(base, pipe_cell.0, pipe_cell.1, 1, 2),
        Lap::Hole => walled(
            base,
            gap_cell.0,
            gap_cell.1,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        ),
    };
    level.start = centre;
    if lap == Lap::Hole {
        level.objects.rat_holes.push(RatHole {
            cell: gap_cell,
            view: RatHoleView::new(gap, Vec2::new(1.0, 0.0)),
        });
    }
    let mut world = sandbox(level);
    hide_rat(&mut world);
    // Start the rat on its waypoint, not at the level's spawn point (which is
    // the pipe's own cell for a pipe lap).
    world.player.pos = match lap {
        Lap::Pipe => pipe + Vec2::new(24.0, 0.0),
        Lap::Hole => gap - Vec2::new(12.0, 0.0),
    };
    let e = world.spawn_enemy(kind, centre + Vec2::new(-26.0, 0.0), 2.0);
    if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
        body.hp = 6000.0;
        body.max_hp = 6000.0;
    }
    // A pipe lap is wide (the eel swings out at half its turning rate), so its
    // plate shows more floor than the rat-hole's.
    let zoom = match lap {
        Lap::Pipe => 1.75,
        Lap::Hole => 1.25,
    };
    let mut script = Script::new(LAP_LOOP, zoom);
    script.framing = Framing::Point(centre);
    let cue: fn(&mut World, Option<Entity>, u32) = match lap {
        Lap::Pipe => pipe_lap_cue,
        Lap::Hole => hole_lap_cue,
    };
    script.cue = Some(cue);
    if kind == EnemyKind::TypeSlug {
        // The slug's own rule is a ruled line: it holds its heading at a pipe
        // end instead of turning, so the far leg of its lap ran off the plate.
        // The lap is clamped inside the frame the card actually drew, 6 px in
        // from every edge.
        script.clamp_subject = Some(LAP_MARGIN);
    }
    (world, script, Some(e))
}

/// The pipe lap: one turn per loop round the stub, so the lap is closed.
fn pipe_lap_cue(world: &mut World, _subject: Option<Entity>, tick: u32) {
    let t = tick as f32 * TICK;
    let c = world.level.start;
    let a = t * std::f32::consts::TAU / LAP_LOOP;
    world.player.pos = c + Vec2::new(a.cos() * 24.0, a.sin() * 13.0);
}

/// The rat-hole lap: the rat crosses through the hole and back, four legs a
/// loop, while the Cat is stopped at the wall.
fn hole_lap_cue(world: &mut World, _subject: Option<Entity>, tick: u32) {
    let t = tick as f32 * TICK;
    let c = world.level.start;
    let phase = (t / (LAP_LOOP * 0.25)).rem_euclid(2.0);
    let tri = if phase <= 1.0 { phase } else { 2.0 - phase };
    world.player.pos = c + Vec2::new(-12.0 + 24.0 * tri, 0.0);
}

/// A shooter: one stand-off body strafing and firing at a dummy, framed so the
/// whole flight and both bodies share the picture. The body's gait is held at
/// zero and the card scripts the strafe itself, because the Spitter archetype's
/// real stand-off (80–110 px) is wider than a plate and a card must show the
/// shot land.
///
/// The lane is **shortened** rather than the view widened: at 34 px the body
/// and the pellet are drawn at a 1.25× zoom (twice the plate size they had at
/// the old 1.75×), and the hostile layer's self-lit pellet plus the ink pass's
/// minimum-size stamp stay readable over the floor.
fn stage_shooter(theme: Theme, kind: EnemyKind) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    hide_rat(&mut world);
    let aim = world.player.pos;
    let e = world.spawn_enemy(kind, aim + Vec2::new(-SHOT_LANE, 0.0), 1.0);
    if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
        body.speed_scale = 0.0;
        body.facing = Vec2::new(1.0, 0.0);
        // The first shot has to leave inside the loop's opening second: three
        // shots a loop show the muzzle tell, the flight and the hit.
        body.fire_cd = 0.35;
        body.hp = 4000.0;
        body.max_hp = 4000.0;
    }
    // The Toll Gull's shot leads the rat, so its plate has a running target: the
    // hidden rat (and the dummy with it) steps away in short bursts, which is
    // exactly the motion the lead aims ahead of.
    let leads = kind == EnemyKind::TollGull;
    let mut script = Script::new(5.0, if leads { 1.55 } else { 1.25 });
    script.framing = Framing::Pair;
    let strafe: fn(&mut World, Option<Entity>, u32) = if leads {
        gull_strafe
    } else {
        shooter_strafe
    };
    script.cue = Some(strafe);
    script.dummy = true;
    // A shooter's pellet is a 1-2 px dot on the hostile layer: the card draws a
    // short self-lit trail behind each one so the flight reads (card-only).
    script.trail = true;
    if leads {
        // The pellet crosses 22 px past the dummy before it is retired.
        script.lead = SHOT_LEAD;
        script.input = |t| Input {
            // Bursts of walking away from the gull, then a stand: the rat has
            // a velocity when the shot leaves (the three shots of a loop, at
            // 0.35 / 2.55 / 4.75 s, all fall inside a burst), so
            // `shot_target` leads it.
            mv: if (0.2..0.75).contains(&(t % 1.1)) {
                Vec2::new(1.0, 0.0)
            } else {
                Vec2::ZERO
            },
            ..Input::default()
        };
    }
    (world, script, Some(e))
}

/// The shooter's strafe: a slow arc at a fixed distance from the dummy, so the
/// lane keeps its length and its floor for the whole loop.
fn shooter_strafe(world: &mut World, subject: Option<Entity>, tick: u32) {
    let Some(e) = subject else {
        return;
    };
    let aim = world.player.pos;
    let t = tick as f32 * TICK;
    // ±17° about the dummy, one sweep a loop.
    let a = std::f32::consts::PI + (t / 5.0 * std::f32::consts::TAU).sin() * 0.30;
    let at = aim + Vec2::from_angle(a) * SHOT_LANE;
    if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
        body.pos = at;
    }
    // The hidden rat is invulnerable, so a pellet would fly on through it and
    // off the plate. The shot ends at the dummy: that is the card's "it lands".
    let mut spent = Vec::new();
    let mut shots = world.ecs.query::<(Entity, &Bullet)>();
    for (shot, b) in shots.iter(&world.ecs) {
        if b.faction == Faction::Enemy && (b.pos.dist(aim) <= DUMMY_R || b.pos.x > aim.x) {
            spent.push(shot);
        }
    }
    for shot in spent {
        world.ecs.despawn(shot);
    }
}

/// The Toll Gull's plate: the same strafe, with the aim point walking away in
/// bursts (see [`stage_shooter`]'s input) so the 0.35 s lead draws a shot that
/// crosses *beside* the dummy instead of dead on it. The pellet lives a little
/// past the dummy — the lead is the whole point — and is retired before it can
/// leave the framed lane.
fn gull_strafe(world: &mut World, subject: Option<Entity>, tick: u32) {
    let Some(e) = subject else {
        return;
    };
    let aim = world.player.pos;
    let t = tick as f32 * TICK;
    let a = std::f32::consts::PI + (t / 5.0 * std::f32::consts::TAU).sin() * 0.30;
    let at = aim + Vec2::from_angle(a) * SHOT_LANE;
    if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
        body.pos = at;
    }
    let mut spent = Vec::new();
    let mut shots = world.ecs.query::<(Entity, &Bullet)>();
    for (shot, b) in shots.iter(&world.ecs) {
        // A leading shot can pass the dummy on either side of the lane: retire
        // it once it is past the framed lead (or has come back almost to the
        // gull's own body).
        if b.faction == Faction::Enemy
            && (b.pos.dist(aim) > SHOT_LANE + SHOT_LEAD + 2.0
                || b.pos.x > aim.x + 24.0
                || b.pos.dist(at) <= DUMMY_R)
        {
            spent.push(shot);
        }
    }
    for shot in spent {
        world.ecs.despawn(shot);
    }
}

/// A trick: the body's own tell → strike → recovery on the dummy, at a zoom
/// that leaves room for its real footprint (a curl, a hop's crosses, a
/// crossbar, a burst, a dive lane, a slide, a needle, jaws, feet, a can's roll
/// and its rebound).
fn stage_trick(theme: Theme, kind: EnemyKind) -> Staged {
    if theme == Theme::Outfall {
        return stage_outfall_trick(kind);
    }
    let mut world = sandbox(patch(theme, 40, 30));
    hide_rat(&mut world);
    let aim = world.player.pos;
    // A body under 4 px is a speck at the plate's 1:1 zoom: its card is drawn at
    // 2 × and its dummy stands the minimum gap off the body's rest position, so
    // the pair fills the tighter frame instead of leaving the body lost in it.
    // Only the four named specks: other small trick bodies have tells far
    // wider than a 2 × view.
    let tiny = matches!(
        kind,
        EnemyKind::BellFlea | EnemyKind::WickMoth | EnemyKind::Lilliputian | EnemyKind::InkBlot
    );
    let gap = if tiny { TINY_DUMMY_GAP } else { 20.0 };
    // Close enough that the body's own rule starts inside the loop, far enough
    // that the dummy reads as a target and not as a hat.
    let e = world.spawn_enemy(kind, aim + Vec2::new(-gap, -gap * 0.15), 1.0);
    if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
        body.hp = 6000.0;
        body.max_hp = 6000.0;
    }
    let mut script = Script::new(5.0, if tiny { TINY_ZOOM } else { 1.5 });
    script.framing = Framing::Pair;
    script.dummy = true;
    if kind == EnemyKind::MurderHornet {
        // Its lane shoves a body: the card's target is a mark, so the rat (and
        // with it the dummy) is held where it was staged.
        script.pin = Some(aim);
    }
    (world, script, Some(e))
}

fn stage_outfall_trick(kind: EnemyKind) -> Staged {
    let mut level = patch(Theme::Outfall, 40, 30);
    let aim = level.start;
    let (cx, cy) = crate::game::collision::cell_of(aim);
    let mut script = Script::new(5.0, 1.5);
    script.dummy = true;
    script.fit = true;
    let at = match kind {
        EnemyKind::Skater => {
            // Start on dry sand within reach of the pool. The lit edge is
            // wide enough to give the splash room before the far bank.
            level.puddles.push(Puddle {
                pos: aim - Vec2::new(18.0, 0.0),
                rx: 25.0,
                ry: 18.0,
                seed: 2,
            });
            script.cue = Some(skater_bank_cue);
            aim - Vec2::new(54.0, 0.0)
        }
        EnemyKind::Lampfish => {
            // The dummy sits inside the lunge trigger with clear sand past it.
            aim - Vec2::new(LAMPFISH_TRIGGER_REACH - 1.0, 0.0)
        }
        EnemyKind::FrySchool => {
            let wall = (cx - 3, cy - 2);
            level = blocked(level, wall.0, wall.1, 1, 1);
            script.fixture = Some(outfall_card_sconce(&mut level, wall));
            // The school reaches the sconce's range before the dummy's.
            aim - Vec2::new(92.0, 0.0)
        }
        _ => unreachable!("the Outfall trick slots"),
    };
    let mut world = sandbox(level);
    hide_rat(&mut world);
    let e = world.spawn_enemy(kind, at, 1.0);
    (world, script, Some(e))
}

/// First let the Skater find the water while the dummy is on the bank, then
/// bring the mark across the shallows to invite its splash.
fn skater_bank_cue(world: &mut World, _subject: Option<Entity>, tick: u32) {
    let progress = ((tick as f32 * TICK - 1.4) / 0.8).clamp(0.0, 1.0);
    world.player.pos = world.level.start - Vec2::new(progress * 24.0, 0.0);
}

/// A heavy (and the Elite card, which is the floor's heavy wearing its gild):
/// the body lumbering in at its real 16 px/s past the dummy, a size reference,
/// on a camera that holds still so the crossing is the movement.
///
/// The body starts inside the plate (a heavy that starts off-frame spends the
/// first second of every loop walking into the picture) and the loop is only as
/// long as the walk plus the moment it presses at the dummy.
fn stage_heavy(theme: Theme, kind: EnemyKind, elite: bool) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    hide_rat(&mut world);
    let aim = world.player.pos;
    let e = world.spawn_enemy(kind, aim + Vec2::new(-24.0, -2.0), 1.0);
    if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
        body.elite = elite;
        body.hp = 9000.0;
        body.max_hp = 9000.0;
    }
    let mut script = Script::new(3.9, 1.35);
    script.framing = Framing::Point(aim + Vec2::new(-4.0, 0.0));
    script.dummy = true;
    (world, script, Some(e))
}

fn stage_miniboss(theme: Theme, kind: MinibossKind) -> Staged {
    let mut level = patch(theme, 40, 30);
    let aim = level.start;
    // Stage the primary attack on its own mark, with room for its footprint.
    // The preview skips the movement interlude and loops after one recovery.
    let primary = kind.signatures()[0];
    let mark = aim + Vec2::new(-kind.engage_range_for(primary).min(40.0), 0.0);
    let fixture = if kind == MinibossKind::Shade {
        let (cx, cy) = crate::game::collision::cell_of(mark);
        // Light behind the Shade, so the cone faces across the open sand.
        let wall = (cx - 2, cy - 1);
        level = blocked(level, wall.0, wall.1, 1, 1);
        Some(outfall_card_sconce(&mut level, wall))
    } else {
        None
    };
    let mut world = sandbox(level);
    hide_rat(&mut world);
    let e = world.spawn_miniboss(kind, 0, mark);
    world.ecs.get_mut::<Miniboss>(e).expect("staged miniboss").preview = true;
    let cycle = TELL_TIME + primary.active + RECOVER_TIME;
    let mut script = Script::new(cycle, 2.0);
    script.framing = Framing::Pair;
    script.dummy = true;
    // The stage fits the body + the tell's own footprint (a cone, a sweep, the
    // Salt Hen's three pads), centre on their midpoint, and takes its zoom from
    // those bounds: what clipped the right edge of the wide rows.
    script.fit = true;
    script.fixture = fixture;
    (world, script, Some(e))
}

fn stage_boss(theme: Theme, kind: BossKind) -> Staged {
    if kind == BossKind::Lantern {
        return stage_lantern();
    }
    let mut level = patch(theme, 44, 34);
    if theme == Theme::Trailer {
        // The scheduled arrival spawns on the map's boss arena, and a map
        // without one means "the whole map": on the bed's own 200-cell map that
        // is 80 cells from the card. Give the bed an arena around the card's
        // start, the size of the other floors' boss patch.
        let (cx, _) = crate::game::collision::cell_of(level.start);
        let deck = level.trailer.deck;
        let arena = TileRect::new(cx - 22, deck.y, 44, deck.h);
        level.boss.floor = arena;
        level.rooms.push(crate::game::level::Room::new(arena, crate::game::level::RoomKind::Boss));
    }
    let mut world = sandbox(level);
    hide_rat(&mut world);
    // The dummy stands just outside the boss's body contact, on the side its
    // attacks are aimed at, so the plate frames the animal, the space it is
    // about to take, and the thing its telegraphs land on.
    world.player.pos += Vec2::new(30.0, 6.0);
    world.force_boss(kind);
    let mut script = Script::new(10.0, 1.0);
    // Drawn 2× oversized and box-filtered down: the art modules draw at 1:1
    // only, and every boss sprite is 26–84 px long against a 44 px plate.
    script.shrink = 2;
    script.dummy = true;
    // The plate frames the union of the body's drawn bounds over one loop
    // (`LibraryScene::probe_boss_union`), not a camera that follows the fight:
    // a stomp, a pounce, a coil and a hoisted stamp all left a moving frame.
    script.boss_fit = true;
    if kind == BossKind::Hawk {
        // The Redtail circles overhead, and while it is up there the art gives
        // it no hit shape at all — the card was a shadow on empty planks. Its
        // loop is the part of the fight that is on the deck: a hover, one dive
        // and the landing, then the hover again.
        script.loop_secs = HAWK_LOOP;
        script.cue = Some(hawk_hover_cue);
    }
    (world, script, None)
}

fn stage_lantern() -> Staged {
    let mut level = patch(Theme::Outfall, 68, 52);
    let aim = level.start;
    let (cx, cy) = crate::game::collision::cell_of(aim);
    let wall = (cx, cy - 2);
    level = blocked(level, wall.0, wall.1, 1, 1);
    let fixture = outfall_card_sconce(&mut level, wall);
    level.puddles.push(Puddle {
        pos: aim + Vec2::new(24.0, 27.0),
        rx: 34.0,
        ry: 18.0,
        seed: 2,
    });
    let mut world = sandbox(level);
    hide_rat(&mut world);
    world.shake_scale = 0.0;
    // The close arrival uses the game's own fight setup and a legal head
    // landing on this open patch. The sconce is the phase-two snuff target.
    world.force_boss_here(BossKind::Lantern);
    let subject = world
        .ecs
        .iter_entities()
        .find(|entity| entity.contains::<Boss>())
        .map(|entity| entity.id());
    let mut script = Script::new(26.0 + 2.0 * art_boss_lantern::PHASE_SHIFT_DURATION, 2.0);
    script.fit = true;
    script.dummy = true;
    script.fixture = Some(fixture);
    script.cue = Some(lantern_phase_cue);
    (world, script, subject)
}

/// Advance only the health thresholds. The controller finishes its locked
/// move, snuffs the fixture, releases its decoy and runs Gape on its own.
fn lantern_phase_cue(world: &mut World, subject: Option<Entity>, tick: u32) {
    let gape_tick = ((18.0 + art_boss_lantern::PHASE_SHIFT_DURATION) / TICK).round() as u32;
    let hp_fraction = match tick {
        300 => 0.60,
        tick if tick == gape_tick => 0.32,
        _ => return,
    };
    if let Some(mut boss) = subject.and_then(|e| world.ecs.get_mut::<Boss>(e)) {
        boss.hp = boss.max_hp * hp_fraction;
    }
}

/// The Redtail's card script: keep the bird in the low fight (the hover and its
/// grounded attacks), and send it down **one** dive a loop so the landing — the
/// only frame that is unarguably the bird and not the shadow — is shown.
///
/// A state pinned here is the art's own state with the art's own duration, so
/// the controller's transitions, hazards and poses stay its own; the card only
/// chooses which of them the loop spends its time in.
fn hawk_hover_cue(world: &mut World, _subject: Option<Entity>, tick: u32) {
    let dive_tick = (HAWK_DIVE_AT / TICK).round() as u32;
    let rat = world.player.pos;
    let mut q = world.ecs.query::<&mut Boss>();
    for mut boss in q.iter_mut(&mut world.ecs) {
        if boss.kind != BossKind::Hawk {
            continue;
        }
        let state = match boss.state {
            BossState::Hawk(s) => s,
            _ => return,
        };
        let slot = (boss.phase.clamp(1, 3) - 1) as usize;
        match state {
            // The states the plate cannot show: the bird is high (or coming
            // back up), so only a shadow is on the deck. Take the low fight.
            art_boss_hawk::HawkState::Circle
            | art_boss_hawk::HawkState::Wheel
            | art_boss_hawk::HawkState::Climb => {
                boss.flags = 0;
                boss.pounce_origin = boss.pos;
                boss.set_state(
                    BossState::Hawk(art_boss_hawk::HawkState::Hover),
                    art_boss_hawk::HOVER_DURATION[slot],
                );
            }
            // One dive a loop, on its own tick so the loop always has exactly
            // one (a card that waited for a settled hover could spend the loop
            // inside a windup and never show the landing). A windup it
            // interrupts is cancelled, exactly as a phase change cancels one.
            _ if tick == dive_tick => {
                boss.flags = 0;
                boss.pounce_origin = boss.pos;
                boss.target = rat;
                boss.set_state(
                    BossState::Hawk(art_boss_hawk::HawkState::Dive),
                    art_boss_hawk::DIVE_DURATION[slot],
                );
            }
            _ => {}
        }
    }
}

// ---- Arsenal, passives, pickups, floors -----------------------------------

fn stage_weapon(theme: Theme, kind: WeaponKind, evolved: bool) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let mut weapon = Weapon::new(kind);
    if evolved {
        weapon.level = kind.max_level();
        if let Some(evo) = kind.evolution() {
            stack_passive(&mut world, evo.passive, 1);
        }
        weapon.evolved = true;
    } else {
        // Top rank: the effect at its most obvious (a level-5 Gnaw's second
        // dart, a full ring of nova spokes, the whip's widest aura), and the
        // active weapons' reach at its longest.
        weapon.level = kind.max_level();
    }
    world.player.loadout.weapons.push(weapon);
    // The rat faces its dummies (a Laser Eyes card fires down `face`, and an
    // idle rat never turns on its own).
    world.player.face = Vec2::new(1.0, 0.0);
    if kind == WeaponKind::LaserEyes {
        // The burst is a fan of thin beams either side of the facing, and at
        // most levels none of them runs dead ahead: turn the rat so the fan's
        // first beam crosses the target in front of it.
        world.player.face = Vec2::from_angle(crate::game::weapon::LASER_SWEEP_HALF);
    }
    let mut script = Script::new(5.0, 1.5);
    script.framing = Framing::Rat;
    // The targets stand `ARSENAL_RING` in front of the rat and the plate is 66 px
    // wide: centred on the rat they were cut by its right edge, digits and all.
    script.lean = Vec2::new(ARSENAL_RING * 0.5, 0.0);
    // Standing targets, not a chase: the arsenal plate is about what the
    // weapon does to a body, and a brief effect (a squeak ring, a thrash arc)
    // needs something on the plate to land on between its own flashes. The cue
    // keeps the active weapons charged so a charge of 6-10 s can still fire
    // several times inside a five-second loop.
    script.cue = Some(arsenal_cue);
    let sweep = matches!(kind, WeaponKind::TailThrash);
    if sweep {
        // A max-rank Tail Thrash sweeps an 84 px arc round the rat: the card
        // showed a white streak leaving the top of the plate and targets that
        // died on the first sweep. Its frame is the whole sweep radius, its
        // targets are x50 hp (`SWEEP_TARGET_HP`) so all three survive the loop.
        let reach = world
            .player
            .loadout
            .weapons
            .last()
            .map_or(84.0, |w| w.active_reach());
        let total = (2.0 * (reach + 6.0) * 1.05) / 30.0;
        script.shrink = ((total / crate::render::zoom::MAX).ceil() as usize).max(1);
        script.zoom = total / script.shrink as f32;
        script.lean = Vec2::ZERO;
        script.cue = Some(arsenal_sweep_cue);
    }
    if kind == WeaponKind::TailWhip {
        script.cue = Some(arsenal_rear_cue);
        script.lean = Vec2::new(-ARSENAL_RING * 0.5, 0.0);
        if evolved {
            // Keep the rear dummies and the front landing area in frame.
            script.zoom = crate::render::zoom::MAX;
            script.lean = Vec2::new(-6.0, 0.0);
        }
    }
    if kind == WeaponKind::Gnaw {
        // A bite reaches 24 px at most: its targets stand in the jaw.
        script.cue = Some(arsenal_bite_cue);
        script.lean = Vec2::new(BITE_RING * 0.5, 0.0);
    }
    if kind == WeaponKind::Hantavirus {
        // The weapon is deployed by **droppings**: the rat leaves one where it
        // stands and the first body to step in it is infected. Targets standing
        // still 30 px away would never step in one, so the rat turns slowly on
        // the spot (the drops ring it) and the three fodder walk slowly in on
        // the trail: one loop shows drop → step → infection → jump.
        script.cue = Some(arsenal_droppings_cue);
        script.loop_secs = 7.0;
    }
    if kind == WeaponKind::SuperDash {
        // The dash leaves the fan of targets behind it. Its targets are laid
        // along the lane the charged dash runs, and the camera holds the lane's
        // midpoint so the whole dash and every hit on it are in frame.
        let total = (DASH_LANE + 16.0) / 44.0;
        script.shrink = ((total / crate::render::zoom::MAX).ceil() as usize).max(1);
        script.zoom = total / script.shrink as f32;
        script.framing = Framing::Point(world.level.start + Vec2::new(DASH_LANE * 0.5, 0.0));
        script.lean = Vec2::ZERO;
        script.cue = Some(arsenal_lane_cue);
    }
    if kind == WeaponKind::LavaBreath {
        // Fit the lifted apex and both outer landing spots in the same frame.
        // Keep the standard targets and let the lava light the plate.
        script.shrink = 2;
        script.zoom = 1.4;
        script.lean = Vec2::new(20.0, -5.0);
        world.flat_lit = false;
    }
    if kind == WeaponKind::GlassBead {
        use crate::game::collision::cell_of;
        let (x, y) = cell_of(world.player.pos);
        let wall = TileKind::Wall(WallKind::Solid);
        world.level.tiles.fill_cells(x + 3, y - 2, 1, 5, wall);
        world.level.tiles.fill_cells(x - 3, y + 2, 6, 1, wall);
        world.level.tiles.fill_cells(x - 3, y - 2, 1, 4, wall);
        world.level.tiles.rebuild_masks();
        world.player.face = Vec2::new(1.0, 0.37).normalized();
        script.shrink = 2;
        script.zoom = 1.4;
        script.lean = Vec2::new(8.0, 4.0);
    }
    if kind == WeaponKind::PlumbBob {
        script.cue = Some(arsenal_bob_cue);
        script.shrink = 2;
        script.zoom = if evolved { 2.0 } else { 1.6 };
        script.lean = Vec2::new(12.0, 0.0);
    }
    if kind == WeaponKind::HoldPoint {
        script.cue = Some(arsenal_hold_cue);
        script.shrink = 2;
        script.zoom = 1.7;
        script.lean = Vec2::ZERO;
    }
    script.input = match kind {
        // A thrash burst lasts a second, so its presses are 1.4 s apart (three
        // bursts a loop, each with its own sweep and hit flashes); a squeak ring
        // expands in 0.3 s and can be fired every 0.8 s (six rings a loop).
        WeaponKind::TailThrash => |t| Input {
            active: (t % 1.4) < TICK * 1.5 && t > 0.4,
            ..Input::default()
        },
        WeaponKind::SonicSqueak => |t| Input {
            active: (t % 0.8) < TICK * 1.5 && t > 0.4,
            ..Input::default()
        },
        WeaponKind::SuperDash => |t| Input {
            // Charge in place, then one charged dash down the lane. The rat
            // does not walk back: the camera is fixed on the lane's midpoint
            // and a rat strolling home across it would leave the frame.
            hold: (0.05..0.9).contains(&t),
            ..Input::default()
        },
        WeaponKind::HoldPoint => |t| Input {
            mv: if (0.22..0.42).contains(&t) { Vec2::new(1.0, 0.0) } else { Vec2::ZERO },
            ..Input::default()
        },
        _ => idle,
    };
    if kind == WeaponKind::SuperDash {
        // The lane is a fixed picture: a shorter loop than the other weapons'
        // (charge, dash, and the sway of what was hit).
        script.loop_secs = 3.0;
    }
    (world, script, None)
}

/// The arsenal plate's per-tick hook: keep the active weapons ready, and keep
/// the three targets standing.
///
/// A Sonic Squeak is charged by a 6 s cooldown and a Tail Thrash by 10 s; a
/// five-second card could only ever show one of them. The charge is the cost a
/// card's facts state, not the thing the card demonstrates, so the plate holds
/// the timer at zero and the effect itself is the weapon's own.
fn arsenal_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    keep_actives_ready(world);
    arsenal_dummies(world, &dummy_homes(world.player.pos), ARSENAL_HP);
}

/// Tail Whip and Ratchet Tail start with all dummies behind the rat.
fn arsenal_rear_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    let rat = world.player.pos;
    let homes = dummy_homes(rat).map(|home| rat - (home - rat));
    arsenal_dummies(world, &homes, ARSENAL_HP);
}

fn arsenal_bob_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    let rat = world.player.pos;
    let homes = [-0.8, 0.0, 0.8].map(|a| rat + Vec2::from_angle(a) * 42.0);
    arsenal_dummies(world, &homes, ARSENAL_HP);
}

fn arsenal_hold_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    let rat = world.level.start;
    let homes = [-0.9, 0.0, 0.9].map(|a| rat + Vec2::from_angle(a) * 22.0);
    arsenal_dummies(world, &homes, ARSENAL_HP);
}

/// Gnaw's and Milk Teeth's plate: the targets stand inside the bite's reach
/// (closer than the radius that hides hit digits: a jaw cannot be shown at
/// arm's length).
fn arsenal_bite_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    keep_actives_ready(world);
    let rat = world.player.pos;
    let mut homes = [rat; 3];
    for (i, home) in homes.iter_mut().enumerate() {
        let a = (i as f32 - 1.0) * 0.66;
        *home = rat + Vec2::from_angle(a) * BITE_RING;
    }
    arsenal_dummies(world, &homes, ARSENAL_HP);
}

/// The Tail Thrash's and the Flywheel's plate: the same standing targets, with
/// enough sandbox hp to survive the loop (a first sweep used to kill them).
fn arsenal_sweep_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    keep_actives_ready(world);
    arsenal_dummies(world, &dummy_homes(world.player.pos), SWEEP_TARGET_HP);
}

/// The Hantavirus's and the Dead Letter's plate: the rat turns slowly on the
/// spot (so each cycle's dropping lands on a different point of a small ring
/// around it) and its three targets walk slowly in until one stands in a
/// dropping. The drop clock is the card's (see [`DROP_EVERY`]); the drop, its
/// arm, the trigger and the chain are the weapon's own.
fn arsenal_droppings_cue(world: &mut World, _subject: Option<Entity>, tick: u32) {
    arsenal_dummies(world, &dummy_homes(world.player.pos), ARSENAL_HP);
    let every = (DROP_EVERY / TICK).round() as u32;
    if let Some(weapon) = world.player.loadout.weapons.first_mut() {
        weapon.timer = if every > 0 && tick.is_multiple_of(every) {
            0.0
        } else {
            999.0
        };
    }
    world.player.face = Vec2::from_angle(tick as f32 * TICK * DROP_TURN);
    let rat = world.player.pos;
    let mut q = world.ecs.query::<&mut Enemy>();
    for mut body in q.iter_mut(&mut world.ecs) {
        // Only the plate's own targets walk: they are the bodies the arsenal
        // staged, and the zero gait is how they are found.
        if body.speed_scale != 0.0 {
            continue;
        }
        let to = rat - body.pos;
        let d = to.len();
        if d <= DROP_STOP || d <= 1e-3 {
            continue;
        }
        let step = (DROP_WALK * TICK).min(d - DROP_STOP);
        body.pos += to * (step / d);
    }
}

/// The Super Dash's and the Ripcord's plate: the targets lie along the dash
/// lane, so the charge, the dash and its hits all happen on the picture.
fn arsenal_lane_cue(world: &mut World, _subject: Option<Entity>, _tick: u32) {
    keep_actives_ready(world);
    arsenal_dummies(world, &dash_lane_homes(world.level.start), ARSENAL_HP);
}

/// Hold every active weapon's charge at zero: the cost a card's facts state,
/// not the thing the card demonstrates.
fn keep_actives_ready(world: &mut World) {
    for w in world.player.loadout.weapons.iter_mut() {
        if w.kind.is_active() {
            w.timer = 0.0;
        }
    }
}

/// Where the arsenal plate's three targets stand: a fan in front of the rat, so
/// a forward weapon's fire and a radial effect both cross them, at the effect's
/// own scale and outside the radius where hit digits are suppressed.
fn dummy_homes(rat: Vec2) -> [Vec2; 3] {
    let mut homes = [rat; 3];
    for (i, home) in homes.iter_mut().enumerate() {
        let a = (i as f32 - 1.0) * 0.66;
        *home = rat + Vec2::from_angle(a) * ARSENAL_RING;
    }
    homes
}

/// Where the dash card's three targets lie: along the charged dash's lane, at
/// three points the sweep crosses.
fn dash_lane_homes(start: Vec2) -> [Vec2; 3] {
    let mut homes = [start; 3];
    for (i, home) in homes.iter_mut().enumerate() {
        *home = start + Vec2::new(DASH_LANE * (0.3 + 0.3 * i as f32), 0.0);
    }
    homes
}

/// The arsenal plate's targets: three real bodies (a weapon can only hit an
/// entity) stood still at arm's length, refreshed if a max-level evolution
/// finishes one off inside the loop. The standing bodies are found by their
/// zero gait, so a knocked-about dummy is not replaced by a duplicate.
fn arsenal_dummies(world: &mut World, homes: &[Vec2; 3], hp: f32) {
    let kind = roster::fodder_kind(world.level.theme);
    let mut standing = 0usize;
    {
        let mut q = world.ecs.query::<&Enemy>();
        for body in q.iter(&world.ecs) {
            if body.kind == kind && body.speed_scale == 0.0 {
                standing += 1;
            }
        }
    }
    for home in homes.iter().skip(standing) {
        let e = world.spawn_enemy(kind, *home, 2.0);
        if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
            body.speed_scale = 0.0;
            body.hp = hp;
            body.max_hp = hp;
        }
    }
}

fn stage_passive(theme: Theme, kind: PassiveKind) -> Option<Staged> {
    Some(match kind {
        // The rat outruns the pack: four fodder chase it the length of the
        // plate and fall behind.
        PassiveKind::MoveSpeed => {
            let mut world = sandbox(patch(theme, 40, 30));
            stack_passive(&mut world, PassiveKind::MoveSpeed, kind.max_stacks());
            let mut script = Script::new(5.0, 1.5);
            script.framing = Framing::Rat;
            script.input = |t| Input {
                mv: Vec2::new(if (t % 2.0) < 1.0 { 1.0 } else { -1.0 }, 0.0),
                ..Input::default()
            };
            script.feed = Some(Feed {
                kind: roster::fodder_kind(theme),
                first: 0.0,
                every: 0.4,
                ring: 34.0,
                hp: 80.0,
                speed: 0.8,
                from: None,
            });
            (world, script, None)
        }
        // Loot at the edge of the plate crosses the floor to the rat: the
        // pickup radius is the passive, and a fresh pair of gems arrives while
        // the last is still in the air.
        PassiveKind::Magnet => {
            let mut world = sandbox(patch(theme, 40, 30));
            stack_passive(&mut world, PassiveKind::Magnet, kind.max_stacks());
            let mut script = Script::new(4.5, 2.0);
            script.framing = Framing::Rat;
            script.cue = Some(magnet_gems);
            (world, script, None)
        }
        _ => return None,
    })
}

/// The Magnet card's loot: a fresh pair of gems at the plate's edge every
/// 24 ticks, at the far side of the widened pickup radius.
fn magnet_gems(world: &mut World, _subject: Option<Entity>, tick: u32) {
    if !tick.is_multiple_of(24) {
        return;
    }
    let rat = world.player.pos;
    let a = tick as f32 * 0.031;
    for k in 0..2 {
        let dir = Vec2::from_angle(a + k as f32 * std::f32::consts::PI);
        world.spawn_gem(Gem::new(rat + dir * 34.0, 1));
    }
}

fn stage_pickup(theme: Theme, kind: PickupKind) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    world.spawn_pickup(Pickup {
        pos: rat + Vec2::new(22.0, 0.0),
        kind,
        bob: 0.0,
    });
    // The Growth Serum (batch 13) is the one find whose effect is a size
    // change plus a stomp: the rat walks onto the flask and then stands on the
    // fodder, so the card shows the real thing — the real
    // `powerups::giant_stomp_system` crushing real bodies — and not a staged
    // effect. Three bodies start inside the 30 px disc and three walk in from
    // beyond it, so a loop keeps stomping something.
    if kind == PickupKind::GrowthSerum {
        let fodder = roster::kind(theme, Slot::A);
        for (dx, dy) in [
            (26.0, -10.0),
            (28.0, 9.0),
            (-12.0, 11.0),
            (62.0, -22.0),
            (68.0, 16.0),
            (-48.0, 20.0),
        ] {
            let e = world.spawn_enemy(fodder, rat + Vec2::new(dx, dy), 1.0);
            if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
                // The rats are the subject here, not the fodder's speed.
                body.speed_scale = 0.55;
            }
        }
        let mut script = Script::new(4.6, 1.2);
        script.framing = Framing::Point(rat + Vec2::new(10.0, 0.0));
        script.input = |t| Input {
            mv: if (0.2..1.0).contains(&t) {
                Vec2::new(1.0, 0.0)
            } else {
                Vec2::ZERO
            },
            ..Input::default()
        };
        return (world, script, None);
    }
    for i in 0..6 {
        let a = i as f32 * 1.05 + 0.4;
        world.spawn_gem(Gem::new(rat + Vec2::new(a.cos() * 30.0 + 22.0, a.sin() * 18.0), 1));
    }
    let fodder = roster::kind(theme, Slot::A);
    for (dx, dy) in [(58.0, -16.0), (62.0, 12.0), (-40.0, 10.0)] {
        let e = world.spawn_enemy(fodder, rat + Vec2::new(dx, dy), 1.0);
        if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
            body.speed_scale = 0.45;
        }
    }
    let mut script = Script::new(2.8, 1.5);
    script.framing = Framing::Point(rat + Vec2::new(16.0, 0.0));
    script.input = |t| Input {
        mv: if (0.4..1.0).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    (world, script, None)
}

/// A floor's card: the real generated map, lit by the floor's own lighting,
/// cropped (by camera) on a furnished room and panned across it, with the
/// floor's two fodder kinds wandering through. No rat: the sector is the
/// subject, and a rat pinned at the edge of an empty floor was neither.
fn stage_floor(theme: Theme) -> Staged {
    let (level, window) = if theme == Theme::Outfall {
        let level = outfall_floor_patch();
        let (cx, cy) = crate::game::collision::cell_of(level.start);
        let (w, h) = FLOOR_WINDOW;
        (level, TileRect::new(cx - w / 2, cy - h / 2, w, h))
    } else {
        let level = Level::generate(theme, SEED);
        let window = floor_window(&level);
        (level, window)
    };
    let centre = window.center_world();
    // The crop's own middle may be the room's wall face (the window was shifted
    // to hold its sconce): the card stands its bodies and its pan on the
    // nearest open floor instead.
    let open = level
        .tiles
        .nearest_open_center(centre, SizeClass::Small, 16)
        .unwrap_or(centre);
    let mut world = sandbox(level);
    // A floor's card is not a portrait of one body: the lighting is the
    // floor's real lighting (`sandbox` force-flat-lights every other card) and
    // the rat is taken out of the picture (it still wanders, hidden, so the
    // fodder has something to follow).
    world.flat_lit = false;
    hide_rat(&mut world);
    world.player.pos = open;
    for (i, slot) in [Slot::A, Slot::B].into_iter().enumerate() {
        let kind = roster::kind(theme, slot);
        for k in 0..3 {
            let a = (i * 3 + k) as f32 * 1.9;
            let at = open + Vec2::from_angle(a) * 10.0 + Vec2::new(0.0, (k as f32 - 1.0) * 8.0);
            let e = world.spawn_enemy(kind, at, 1.0);
            if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
                // A slow wander, not a chase, and bodies that survive the loop:
                // the card is about the floor, and a thinned-out pack is not.
                body.speed_scale = 0.55;
                body.hp = 800.0;
                body.max_hp = 800.0;
            }
        }
    }
    let mut script = Script::new(10.0, FLOOR_ZOOM);
    script.framing = Framing::Pan(open);
    // The window is 12 × 8 tiles (192 × 128 px); at `FLOOR_ZOOM` the plate
    // shows 88 × 60 of them, so the pan travels the rest of the window and back.
    let view = Vec2::new(44.0 * FLOOR_ZOOM, 30.0 * FLOOR_ZOOM);
    script.pan = Vec2::new(
        ((window.w as f32 * crate::game::collision::TILE_F - view.x) * 0.5).max(0.0),
        ((window.h as f32 * crate::game::collision::TILE_F - view.y) * 0.5).max(0.0),
    );
    script.window = Some(window);
    script.wander = Some((open, Vec2::new(20.0, 10.0)));
    (world, script, None)
}

/// A cross-section of an Outfall bank. The fixtures share the same level data
/// and systems as the generated floor, so entering the bulb's range plays its
/// warning and release as the card pans.
pub(super) fn outfall_floor_patch() -> Level {
    let mut level = patch(Theme::Outfall, 24, 18);
    level.start += Vec2::new(0.0, 8.0);
    let (cx, cy) = crate::game::collision::cell_of(level.start);
    level = blocked(level, cx - 5, cy - 2, 11, 1);
    outfall_card_sconce(&mut level, (cx - 1, cy - 2));
    for (kind, dx, dy) in [
        (PropKind::OutfallPiling, -2, 0),
        (PropKind::OutfallRib, 2, 1),
    ] {
        let x = cx + dx;
        let y = cy + dy;
        let (w, h) = kind.size();
        let tile = crate::game::collision::TILE_F;
        level = blocked(level, x, y, (w / tile).ceil() as i32, (h / tile).ceil() as i32);
        level.props.push(Prop {
            kind,
            rect: crate::game::level::Rectf::new(x as f32 * tile, y as f32 * tile, w, h),
        });
    }
    let aim = level.start;
    level.puddles.push(Puddle {
        pos: aim + Vec2::new(-9.0, 8.0),
        rx: 25.0,
        ry: 15.0,
        seed: 2,
    });
    let bulb = aim + Vec2::new(25.0, 0.0);
    level.objects.lure_traps.push(LureTrap::new(
        bulb,
        vec![bulb + Vec2::new(0.0, 24.0)],
    ));
    let wall_y = (cy - 1) as f32 * crate::game::collision::TILE_F + 5.5;
    level.plants.extend([
        crate::game::level::SeabedPlant {
            pos: Vec2::new(aim.x - 36.0, wall_y),
            kind: crate::game::level::PlantKind::Kelp,
            phase: 0.4,
            size: 2,
            seed: 10,
        },
        crate::game::level::SeabedPlant {
            pos: Vec2::new(aim.x - 31.0, wall_y + 1.0),
            kind: crate::game::level::PlantKind::Kelp,
            phase: 1.5,
            size: 1,
            seed: 11,
        },
        crate::game::level::SeabedPlant {
            pos: aim + Vec2::new(8.0, 14.0),
            kind: crate::game::level::PlantKind::Seagrass,
            phase: 0.9,
            size: 1,
            seed: 21,
        },
        crate::game::level::SeabedPlant {
            pos: Vec2::new(aim.x + 12.0, wall_y),
            kind: crate::game::level::PlantKind::Anemone,
            phase: 2.1,
            size: 1,
            seed: 30,
        },
    ]);
    level
}

/// The 12 × 8-tile window a floor's card is cut from: the most furnished room
/// on the floor — wall faces, props, a hanging light and a wall sconce — with
/// the window shifted off the room's centre as little as possible.
///
/// The candidate is scored on what the card must show, so the choice is data
/// (the generator's own room and furniture lists), never a retyped guess. A
/// sconce is the anchor: the generator mounts one on the first wall face of the
/// start room and of every other furnished room (`levelgen::lights`), and the
/// room a sconce stands in is furnished by `levelgen::layout::furnish`.
fn floor_window(level: &Level) -> TileRect {
    let (w, h) = FLOOR_WINDOW;
    let (cols, rows) = (level.tiles.cols(), level.tiles.rows());
    let clamp = |x: i32, y: i32| {
        TileRect::new(
            x.clamp(1, (cols - w - 1).max(1)),
            y.clamp(1, (rows - h - 1).max(1)),
            w,
            h,
        )
    };
    let mut best: Option<(i32, TileRect)> = None;
    let mut consider = |window: TileRect, bonus: i32| {
        let score = bonus + floor_crop_score(level, window);
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, window));
        }
    };
    // Anchor on every sconce: the window holds the sconce and as much of its
    // room's middle (where the props are) as it can.
    for light in &level.objects.lights {
        if light.kind != LightFixtureKind::Sconce {
            continue;
        }
        let (sx, sy) = crate::game::collision::cell_of(light.pos);
        let room = level.rooms.iter().find(|r| r.rect.contains(sx, sy));
        let (mut wx, mut wy) = match room {
            Some(room) => (
                room.rect.x + room.rect.w / 2 - w / 2,
                room.rect.y + room.rect.h / 2 - h / 2,
            ),
            None => (sx - w / 2, sy - h / 2),
        };
        // Shift only as far as the sconce has to be inside the window.
        wx = wx.min(sx).max(sx - (w - 1));
        wy = wy.min(sy).max(sy - (h - 1));
        let bonus = match room.map(|r| r.kind) {
            Some(RoomKind::Room | RoomKind::Hall) => 6,
            Some(RoomKind::Start) => 4,
            Some(RoomKind::Boss) => 3,
            Some(RoomKind::Corridor) => 1,
            _ => 0,
        };
        consider(clamp(wx, wy), bonus);
    }
    // And on every room's centre, in case a floor's sconces all landed in
    // corridors: a furnished room with no lamp of its own still reads.
    for room in &level.rooms {
        if matches!(room.kind, RoomKind::Maze | RoomKind::Corridor) {
            continue;
        }
        consider(
            clamp(
                room.rect.x + room.rect.w / 2 - w / 2,
                room.rect.y + room.rect.h / 2 - h / 2,
            ),
            0,
        );
    }
    match best {
        Some((_, window)) => window,
        // A map with no rooms at all (never a generated one): the start room.
        None => {
            let (cx, cy) = crate::game::collision::cell_of(level.start);
            clamp(cx - w / 2, cy - h / 2)
        }
    }
}

/// What a floor's 12 × 8-tile window holds, as one number: walls are the room's
/// frame, props are the furniture, lights and especially sconces are the
/// floor's own lighting, and a window with no wall has nothing to show.
fn floor_crop_score(level: &Level, window: TileRect) -> i32 {
    let mut walls = 0;
    for (cx, cy) in window.cells() {
        if level.tiles.get(cx, cy).is_wall() {
            walls += 1;
        }
    }
    if walls < 2 {
        return -100;
    }
    let props = level
        .props
        .iter()
        .filter(|p| window.contains_world(p.rect.center()))
        .count();
    let mut lights = 0;
    let mut sconces = 0;
    for light in &level.objects.lights {
        if !window.contains_world(light.pos) {
            continue;
        }
        lights += 1;
        if light.kind == LightFixtureKind::Sconce {
            sconces += 1;
        }
    }
    // A window with **both** a prop and a wall sconce is the card the task
    // describes, so it outranks any window with only one of them (the boss
    // arena's ring of six sconces is a wall with no furniture in it); among
    // those, more furniture and more lamps win.
    let mut score = walls.min(24) + lights.min(6);
    if sconces > 0 {
        score += 40;
    }
    if props > 0 {
        score += 30;
    }
    if props > 0 && sconces > 0 {
        score += 200;
    }
    score + props.min(6) as i32 * 2 + sconces.min(3) * 2
}

// ---- Field guide ----------------------------------------------------------

/// What a flatbed card shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeckFeature {
    Pit,
    Cargo,
    Wind,
}

fn stage_object(theme: Theme, kind: ObjectKind) -> Option<Staged> {
    Some(match kind {
        ObjectKind::Elite => stage_enemy(theme, roster::elite_kind(theme), true),
        ObjectKind::Gold => stage_gold(theme),
        ObjectKind::Chest => stage_chest(theme),
        ObjectKind::StickyBoard => stage_sticky(theme),
        ObjectKind::Mousetrap => stage_mousetrap(theme),
        ObjectKind::SteamGrate => stage_steam(),
        ObjectKind::InkPad => stage_ink(),
        ObjectKind::Pit => stage_deck(DeckFeature::Pit),
        ObjectKind::SlidingCargo => stage_deck(DeckFeature::Cargo),
        ObjectKind::Slipstream => stage_deck(DeckFeature::Wind),
        ObjectKind::RatHole => stage_rat_hole(theme),
        ObjectKind::ShopHole => stage_shop_hole(theme),
        ObjectKind::MazeCache => stage_maze_cache(theme),
        ObjectKind::OneWayDoor => stage_one_way_door(theme),
        ObjectKind::Gem => return None,
        ObjectKind::CrabPot => stage_crab_pot(),
    })
}

/// Coins and gems on the floor, swept up by the rat's own pickup pass.
fn stage_gold(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    for i in 0..8 {
        let p = rat + Vec2::new(12.0 + i as f32 * 7.0, (i as f32 * 0.9).sin() * 7.0);
        spawn_with_id(&mut world.ecs, Gold::new(p, 1 + i % 3));
    }
    for i in 0..4 {
        let p = rat + Vec2::new(16.0 + i as f32 * 13.0, -9.0 + i as f32 * 6.0);
        world.spawn_gem(Gem::new(p, 1));
    }
    let mut script = Script::new(5.0, 2.0);
    script.framing = Framing::Rat;
    script.input = |t| Input {
        mv: if (0.4..1.7).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    // The floor keeps paying out: a coin lands within the magnet's reach every
    // 20 ticks and flies in, so the loop never runs dry.
    script.cue = Some(gold_trail);
    (world, script, None)
}

/// A fresh coin at the plate's edge, every 20 ticks (the Gold card's cue).
fn gold_trail(world: &mut World, _subject: Option<Entity>, tick: u32) {
    if !tick.is_multiple_of(20) {
        return;
    }
    let rat = world.player.pos;
    let at = rat + Vec2::from_angle(tick as f32 * 0.047) * 26.0;
    let value = 1 + (tick / 20) % 3;
    spawn_with_id(&mut world.ecs, Gold::new(at, value));
}

/// A chest on the floor with its glint. The rat stands off it: touching one
/// hands the run a chest screen, and a card must never open a plate.
fn stage_chest(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    let chest = rat + Vec2::new(24.0, 0.0);
    world.spawn_chest(chest);
    for (dx, dy) in [(-10.0, 12.0), (14.0, -14.0), (26.0, 16.0)] {
        world.spawn_gem(Gem::new(chest + Vec2::new(dx, dy), 1));
    }
    // The rat walks up and **takes** the chest (the real collection: the chest
    // despawns and `pending_chests` rises, then the card drops the plate on the
    // floor like every other pending screen). What the run shows next is its
    // chest screen; what the card shows is its own open lid and the payout
    // burst, painted over the spot.
    let mut script = Script::new(3.6, 2.0);
    script.framing = Framing::Point(chest - Vec2::new(4.0, 0.0));
    script.input = |t| Input {
        mv: if (0.4..0.95).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    script.chest = Some(chest);
    (world, script, None)
}

/// A glue board takes Brutes one after another until its lifetime budget is
/// gone: `Idle` → `Stuck` → `Used`. Sprung by the floor's own bodies, not by the
/// rat: the rat is the thing they are walking toward, and it is not drawn.
///
/// One body at a time, on a slow cadence: a Brute spends 4 of the board's 20
/// glue units, so a 13 s loop walks five of them across it (one caught, one
/// approaching — the plate is about the board, not about a crowd standing on
/// it), and the last catch leaves the board `Used` for the closing seconds. The
/// hidden rat stands well above the plate, so a body that has torn free walks
/// on out of the frame rather than piling up on the dummy.
fn stage_sticky(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    hide_rat(&mut world);
    let rat = world.player.pos;
    let board = rat + Vec2::new(0.0, 20.0);
    world
        .level
        .objects
        .sticky_traps
        .push(StickyTrapView::new(board, Vec2::new(1.0, 0.0)));
    // The rat (the bodies' goal) is 40 px above the board: past the top edge of
    // the plate, so the far side of the board stays clear.
    world.player.pos = board - Vec2::new(0.0, 40.0);
    let mut script = Script::new(13.0, 1.5);
    script.framing = Framing::Point(board);
    // Each Brute enters 30 px below the board and walks its own 16 px/s up over
    // the glue: caught, held 1.5 s, freed, and away off the top of the plate.
    script.feed = Some(Feed {
        kind: EnemyKind::Brute,
        first: 0.5,
        every: 1.9,
        ring: 70.0,
        hp: 6000.0,
        speed: 1.0,
        from: Some(Vec2::new(0.0, 1.0)),
    });
    (world, script, None)
}

/// A baited mousetrap: `Armed` → `Triggered` → `Snapping` → `Sprung`, walked
/// into by the floor's fodder.
fn stage_mousetrap(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    hide_rat(&mut world);
    let rat = world.player.pos;
    let trap = rat + Vec2::new(0.0, 22.0);
    // The bait end faces the body walking in from below.
    world
        .level
        .objects
        .mousetraps
        .push(MousetrapView::new(trap, Vec2::new(0.0, 1.0)));
    let mut script = Script::new(4.2, 1.5);
    script.framing = Framing::Point(trap);
    script.feed = Some(Feed {
        kind: roster::fodder_kind(theme),
        first: 0.2,
        every: 3.6,
        ring: 54.0,
        hp: 40.0,
        speed: 0.7,
        from: Some(Vec2::new(0.0, 1.0)),
    });
    (world, script, None)
}

fn stage_crab_pot() -> Staged {
    let mut world = sandbox(patch(Theme::Outfall, 40, 30));
    hide_rat(&mut world);
    let facing = Vec2::new(0.0, 1.0);
    let trap = world.player.pos + facing * 22.0;
    world.level.objects.crab_pots.push(
        crate::game::ecs::systems::traps::set_crab_pot(trap, facing),
    );
    let mut script = Script::new(5.5, 1.9);
    script.framing = Framing::Point(trap);
    script.feed = Some(Feed {
        kind: roster::fodder_kind(Theme::Outfall),
        first: 0.2,
        every: 6.0,
        ring: 48.0,
        hp: 40.0,
        speed: 0.9,
        from: Some(facing),
    });
    (world, script, None)
}

/// A steam grate's whole cycle (resting 4 s, warning 1 s, plume 0.7 s) with a
/// body walking over it.
fn stage_steam() -> Staged {
    let mut world = sandbox(patch(Theme::Laundry, 40, 30));
    hide_rat(&mut world);
    let rat = world.player.pos;
    let cell = crate::game::collision::cell_of(Vec2::new(rat.x, rat.y + 24.0));
    world
        .level
        .objects
        .steam_grates
        .push(SteamGrate::new(cell, 0.0));
    let mut script = Script::new(6.0, 1.5);
    script.framing = Framing::Point(cell_centre(cell));
    script.feed = Some(Feed {
        kind: roster::kind(Theme::Laundry, Slot::A),
        first: 0.6,
        every: 2.4,
        ring: 46.0,
        hp: 60.0,
        speed: 0.5,
        from: Some(Vec2::new(0.0, 1.0)),
    });
    (world, script, None)
}

/// An ink pad: the floor's pack wades through a permanent 75 % stain on its
/// way to the (hidden) rat.
fn stage_ink() -> Staged {
    let mut world = sandbox(patch(Theme::Printworks, 40, 30));
    hide_rat(&mut world);
    let rat = world.player.pos;
    let cells = TileRect::new((rat.x as i32) / 16 + 2, (rat.y as i32) / 16 - 1, 3, 3);
    world
        .level
        .objects
        .ink_pads
        .push(InkPad::new(cells, 7));
    let mut script = Script::new(5.0, 1.5);
    script.framing = Framing::Point(cells.center_world() - Vec2::new(14.0, 0.0));
    script.feed = Some(Feed {
        kind: roster::kind(Theme::Printworks, Slot::A),
        first: 0.3,
        every: 1.2,
        ring: 54.0,
        hp: 60.0,
        speed: 0.7,
        from: Some(Vec2::new(1.0, 0.0)),
    });
    (world, script, None)
}

/// One of the flatbed's own hazards, on the trailer's real bed.
fn stage_deck(feature: DeckFeature) -> Staged {
    // The flatbed's own bed (planks, joints, rails, the road in the gaps): the
    // deck art is anchored to the map's deck rectangle, so a hand-sized patch
    // at the origin only ever showed the verge.
    let mut level = patch(Theme::Trailer, 32, 22);
    let rat = level.start;
    let cells = |p: Vec2| crate::game::collision::cell_of(p);
    let mut script = Script::new(5.0, 1.5);
    match feature {
        DeckFeature::Pit => {
            let pit_at = Vec2::new(rat.x + 24.0, rat.y);
            let c = cells(pit_at);
            level
                .trailer
                .pits
                .push(Pit::new(TileRect::new(c.0, c.1 - 1, 2, 3), PitKind::BrokenPlank));
            // The rat is the subject of this card, so the plate holds it *and*
            // the hole it is about to fall into.
            script.framing = Framing::Point(Vec2::new(rat.x + 14.0, rat.y));
            script.loop_secs = 4.0;
            // The pit's own point is the rat: 8 damage, i-frames and a
            // replacement on the nearest plank. It walks in, falls, is put
            // back, and walks in again — two falls a loop, so the card never
            // stands still for the second half.
            script.input = |t| Input {
                mv: if (0.3..1.5).contains(&t) || (2.2..3.4).contains(&t) {
                    Vec2::new(1.0, 0.0)
                } else {
                    Vec2::ZERO
                },
                ..Input::default()
            };
        }
        DeckFeature::Cargo => {
            let rest = rat + Vec2::new(28.0, 0.0);
            level.trailer.slides.push(SlidingCargo {
                rest,
                dir: Vec2::new(1.0, 0.0),
                travel: 14.0,
                phase: 0.0,
                half: Vec2::new(8.0, 8.0),
                warn: 0.6,
            });
            script.framing = Framing::Point(rest + Vec2::new(4.0, 0.0));
            script.loop_secs = 6.4;
            script.input = |t| Input {
                mv: if (1.0..3.0).contains(&t) {
                    Vec2::new(1.0, 0.0)
                } else {
                    Vec2::ZERO
                },
                ..Input::default()
            };
        }
        DeckFeature::Wind => {
            let c = cells(rat + Vec2::new(24.0, 0.0));
            let wind = TileRect::new(c.0, c.1 - 2, 6, 5);
            level.trailer.wind.push(wind);
            script.framing = Framing::Point(wind.center_world());
        }
    }
    let mut world = sandbox(level);
    if feature == DeckFeature::Wind {
        // The slipstream is a hazard the floor's own bodies are dragged by, so
        // the rat has no part in this plate.
        hide_rat(&mut world);
        let wind = world.level.trailer.wind[0].center_world();
        // Gems on the exposed stretch blow tailward (the slipstream's loot
        // half), so the plate shows the wind without a single sfx.
        for i in 0..6 {
            let p = Vec2::new(
                wind.x + 10.0 + (i % 3) as f32 * 8.0,
                wind.y - 16.0 + (i / 3) as f32 * 26.0,
            );
            world.spawn_gem(Gem::new(p, 1));
        }
        let kind = roster::kind(Theme::Trailer, Slot::A);
        for (i, dx) in [4.0f32, 16.0].into_iter().enumerate() {
            let e = world.spawn_enemy(kind, wind + Vec2::new(dx, 10.0 - i as f32 * 20.0), 1.0);
            if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
                body.hp = 400.0;
                body.max_hp = 400.0;
            }
        }
    }
    (world, script, None)
}

/// A rat-hole in a wall, with the rat squeezing through and a Cat that cannot
/// follow it.
fn stage_rat_hole(theme: Theme) -> Staged {
    let wall_x = 20;
    let gap_y = 15;
    let gap = cell_centre((wall_x, gap_y));
    let mut level = walled(
        patch(theme, 40, 30),
        wall_x,
        gap_y,
        TileKind::Wall(WallKind::RatHole {
            axis: WallAxis::Vertical,
        }),
    );
    level.start = gap - Vec2::new(40.0, 0.0);
    level.objects.rat_holes.push(RatHole {
        cell: (wall_x, gap_y),
        view: RatHoleView::new(gap, Vec2::new(1.0, 0.0)),
    });
    let mut world = sandbox(level);
    // The hole's whole point: a Cat is bigger than the gap.
    world.spawn_enemy(EnemyKind::Cat, gap + Vec2::new(30.0, 0.0), 1.0);
    let mut script = Script::new(3.6, 1.5);
    script.input = |t| Input {
        mv: if (0.4..2.0).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    (world, script, None)
}

/// The one-way door at the end of a maze: the rat walks through, the leaf
/// swings open and shuts behind it.
fn stage_one_way_door(theme: Theme) -> Staged {
    let wall_x = 20;
    let gap_y = 15;
    let gap = cell_centre((wall_x, gap_y));
    let mut level = walled(
        patch(theme, 40, 30),
        wall_x,
        gap_y,
        TileKind::Wall(WallKind::OneWayDoor {
            dir: Cardinal::East,
        }),
    );
    level.start = gap - Vec2::new(40.0, 0.0);
    level
        .objects
        .one_way_doors
        .push(OneWayDoor::new((wall_x, gap_y), Vec2::new(1.0, 0.0)));
    let world = sandbox(level);
    let mut script = Script::new(3.4, 1.5);
    script.input = |t| Input {
        mv: if (0.3..1.3).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    (world, script, None)
}

/// A mouse-hole shop in a wall: the rat walks up to the threshold and the
/// mouth answers.
fn stage_shop_hole(theme: Theme) -> Staged {
    let wall_x = 20;
    let gap_y = 15;
    let cell = (wall_x, gap_y);
    let pos = cell_centre(cell);
    let mut level = walled(
        patch(theme, 40, 30),
        wall_x,
        gap_y,
        TileKind::Wall(WallKind::Solid),
    );
    level.start = pos - Vec2::new(44.0, 0.0);
    level.objects.shop_holes.push(ShopHole {
        index: 0,
        cell,
        pos,
        facing: Vec2::new(-1.0, 0.0),
        approach: Cardinal::West,
        mouth: (wall_x - 1, gap_y),
        role: ShopHoleRole::Fork,
    });
    let world = sandbox(level);
    let mut script = Script::new(4.0, 1.5);
    script.framing = Framing::Point(pos - Vec2::new(16.0, 0.0));
    script.input = |t| Input {
        mv: if (0.3..1.7).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    (world, script, None)
}

/// The maze cache: claimed, its lid hinges open over the pedestal.
fn stage_maze_cache(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    let pos = rat + Vec2::new(26.0, 0.0);
    world
        .level
        .objects
        .rewards
        .push(MazeRewardView::new(pos, Vec2::new(-1.0, 0.0)));
    let mut script = Script::new(3.0, 1.5);
    script.framing = Framing::Pair;
    script.input = |t| Input {
        mv: if (0.3..0.7).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    (world, script, None)
}

// ---- The practice dummy ---------------------------------------------------

/// Cork, its cut face and the cheese-rind band: a neutral target in no floor's
/// palette, so it can never be mistaken for the subject.
const CORK: Rgb = (198, 152, 98);
const CORK_LIT: Rgb = (232, 196, 148);
const CORK_RIND: Rgb = (96, 62, 36);
const RIND: Rgb = (240, 196, 92);
const CHALK: Rgb = (255, 248, 232);
/// The cork's own shadow (a value nothing else in the game draws).
const CORK_SHADOW: Rgb = (25, 21, 30);

/// The plate pixels the world's pass left as bare floor, taken **before** the
/// dummy paints anything: the cork may only land on these.
///
/// A mask and not a colour test on the live plate: the dummy lays its rim, band
/// and cross over its own cork, and "this pixel is already cork-coloured" cannot
/// say whose cork it is — The Mudlark's shell is exactly [`CORK`], and a dark
/// outline can match the shadow.
struct BareFloor {
    w: usize,
    h: usize,
    bare: Vec<bool>,
}

impl BareFloor {
    fn of(pb: &PixelBuffer, floor: &PixelBuffer) -> BareFloor {
        let (w, h) = (pb.w.min(floor.w), pb.h.min(floor.h));
        let mut bare = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                bare.push(pb.pixel_at(x, y) == floor.pixel_at(x, y));
            }
        }
        BareFloor { w, h, bare }
    }

    fn at(&self, x: i32, y: i32) -> bool {
        (0..self.w as i32).contains(&x)
            && (0..self.h as i32).contains(&y)
            && self.bare[y as usize * self.w + x as usize]
    }
}

/// One horizontal chord of a circle of radius `r`, `dy` rows from its centre,
/// painted only where the world's pass left bare floor — the card's dummy
/// never covers anything the world drew.
fn chord(pb: &mut PixelBuffer, floor: &BareFloor, s: (i32, i32), r: i32, dy: i32, c: Rgb) {
    let h2 = r * r - dy * dy;
    if h2 <= 0 {
        return;
    }
    let half = (h2 as f32).sqrt() as i32;
    for dx in -half..=half {
        plot_on_floor(pb, floor, s.0 + dx, s.1 + dy, c);
    }
}

/// Paint one pixel of the card's dummy, and only if the world's pass left that
/// pixel as bare floor: the check is what puts the cork *under* the bodies (see
/// [`draw_practice_dummy`]).
fn plot_on_floor(pb: &mut PixelBuffer, floor: &BareFloor, x: i32, y: i32, c: Rgb) {
    if floor.at(x, y) {
        pb.plot(x, y, c);
    }
}

/// The card's practice target: a small cork with a cheese-rind band, standing
/// where the hidden rat stands. Painted by the card itself: it is never an
/// entity, so no system can see it, hurt it or count it, and the plate can
/// never become a bigger fight.
///
/// `floor` is the plate's bare floor ([`LibraryScene::floor_plate`]), and every
/// pixel is painted only where the world's own pass left that floor alone. So
/// the dummy sits **under** the picture: a body that lands on it, a tell that
/// sweeps over it, a pellet crossing it and a glue strand across it all keep
/// their own pixels, which is what the card's "the attack lands here" mark
/// needs to stay honest (a body the cork had covered would leave the plate
/// with no subject at all — the Bell Flea, the Wick Moth and the Loose Bungee
/// all end their loop standing on the dummy).
fn draw_practice_dummy(pb: &mut PixelBuffer, floor: &PixelBuffer, s: (i32, i32), r: i32) {
    let r = r.max(2);
    let floor = &BareFloor::of(pb, floor);
    // Its own shadow, so it stands on the floor rather than in it.
    chord(pb, floor, (s.0, s.1 + r), r, 0, CORK_SHADOW);
    for dy in -r..=r {
        let h2 = r * r - dy * dy;
        if h2 <= 0 {
            continue;
        }
        let half = (h2 as f32).sqrt() as i32;
        for dx in -half..=half {
            plot_on_floor(pb, floor, s.0 + dx, s.1 + dy, CORK);
        }
    }
    // A hard cork rim, plotted (not the additive `PixelBuffer::ring`): the
    // dummy has to read at full contrast on a bright plate.
    for dy in -r..=r {
        let h2 = r * r - dy * dy;
        if h2 <= 0 {
            continue;
        }
        let half = (h2 as f32).sqrt() as i32;
        plot_on_floor(pb, floor, s.0 - half, s.1 + dy, CORK_RIND);
        plot_on_floor(pb, floor, s.0 + half, s.1 + dy, CORK_RIND);
    }
    // The cut face, lit from the game's top-left.
    chord(pb, floor, s, r - 1, -2, CORK_LIT);
    chord(pb, floor, s, r - 1, -1, CORK_LIT);
    // The rind band: the one bright mark that says "the attack lands here".
    chord(pb, floor, s, r, r / 2, RIND);
    chord(pb, floor, s, r, (r / 2).max(1), RIND);
    // A chalk cross on the face.
    plot_on_floor(pb, floor, s.0, s.1 - 1, CHALK);
    plot_on_floor(pb, floor, s.0 - 1, s.1 - 1, CHALK);
    plot_on_floor(pb, floor, s.0 + 1, s.1 - 1, CHALK);
}

/// Every colour the card's dummy paints (its shadow included).
/// The boss's loop union, probed once per kind per process: the probe is a
/// whole loop of world ticks, and the scene is a fixture (own seed, same stage
/// every time), so the answer never changes.
fn cached_boss_union(scene: &mut LibraryScene, kind: BossKind) -> Option<(Vec2, Vec2)> {
    use std::sync::{Mutex, OnceLock};
    type Slot = Option<Option<(Vec2, Vec2)>>;
    static CACHE: OnceLock<Mutex<[Slot; 16]>> = OnceLock::new();
    let i = BossKind::all()
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0)
        .min(15);
    let cache = CACHE.get_or_init(|| Mutex::new([None; 16]));
    if let Some(hit) = cache.lock().ok().and_then(|c| c[i]) {
        return hit;
    }
    let union = scene.probe_boss_union();
    if let Ok(mut c) = cache.lock() {
        c[i] = Some(union);
    }
    union
}

#[cfg(test)]
const DUMMY_INK: [Rgb; 6] = [CORK, CORK_LIT, CORK_RIND, RIND, CHALK, CORK_SHADOW];

/// The chest card's reward: the lid thrown open, the golden mouth and the burst
/// the collection pays out, painted by the card itself.
///
/// The simulation's chest is consumed on contact (`treasure_system` despawns it
/// and raises `pending_chests`), so the run has no open frame to show: the card
/// keeps the moment and paints it, the way it paints its practice target.
fn draw_open_chest(pb: &mut PixelBuffer, s: (i32, i32), age: f32) {
    let (cx, cy) = s;
    let burst = (age / BURST_SECS).clamp(0.0, 1.0);
    // The box, four rows tall so the open lid reads above it.
    for dy in 0..4 {
        for dx in -5..=5 {
            let c = if dy == 0 { CHEST_WOOD_HI } else { CHEST_WOOD };
            pb.plot(cx + dx, cy + 1 + dy, c);
        }
    }
    // The mouth, and the gold inside it.
    for dx in -3..=3 {
        pb.plot(cx + dx, cy, CHEST_MOUTH);
    }
    for x in [cx - 2, cx + 2] {
        pb.plot(x, cy, CHEST_GLOW);
    }
    // The lid, thrown up and back, with its brass studs.
    for dx in -5..=5 {
        pb.plot(cx + dx, cy - 2, CHEST_WOOD_HI);
        pb.plot(cx + dx, cy - 3, CHEST_WOOD);
    }
    for x in [cx - 4, cx + 4] {
        pb.plot(x, cy - 3, CHEST_BRASS);
        pb.plot(x, cy, CHEST_BRASS);
    }
    // The reward burst: a ring opening out of the mouth.
    let r = 3.0 + burst * 9.0;
    let steps = 22;
    for i in 0..steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        if a.sin() <= -0.05 {
            continue;
        }
        let x = cx + (a.cos() * r).round() as i32;
        let y = cy - 2 + (a.sin() * r * 0.7).round() as i32;
        pb.plot(x, y, if burst < 0.6 { CHEST_GLOW } else { CHEST_BRASS });
    }
    // Coins riding the burst up and out.
    if burst < 1.0 {
        for i in 0..3 {
            let a = -1.9 + i as f32 * 0.6;
            let d = 4.0 + burst * (10.0 + i as f32 * 3.0);
            let x = cx + (a.cos() * d).round() as i32;
            let y = cy - 3 + (a.sin() * d * 0.9).round() as i32;
            pb.plot(x, y, CHEST_BRASS);
        }
    }
}

/// The world rectangle one signature footprint covers, straight from the
/// shape's own numbers. A fitted plate asks the shape rather than guessing from
/// the kind's name.
fn shape_bounds(shape: MinibossShape) -> (Vec2, Vec2) {
    let mut lo = shape.center();
    let mut hi = lo;
    let mut grow = |p: Vec2, pad: f32| {
        lo = Vec2::new(lo.x.min(p.x - pad), lo.y.min(p.y - pad));
        hi = Vec2::new(hi.x.max(p.x + pad), hi.y.max(p.y + pad));
    };
    match shape {
        MinibossShape::Trench { a, b, half } => {
            grow(a, half);
            grow(b, half);
        }
        MinibossShape::Cone {
            origin,
            aim,
            half_angle,
            len,
        } => {
            let base = aim.normalized().angle();
            grow(origin, 0.0);
            for a in [base - half_angle, base, base + half_angle] {
                grow(origin + Vec2::from_angle(a) * len, 2.0);
            }
        }
        MinibossShape::Cord { spool, tip, half } => {
            grow(spool, half);
            grow(tip, half);
        }
        MinibossShape::RingGap {
            origin,
            radius,
            half,
            ..
        } => {
            grow(origin, radius + half);
        }
        MinibossShape::Stamps { centers, half } => {
            for c in centers {
                grow(c, half);
            }
        }
        MinibossShape::Spill {
            center,
            aim,
            half_len,
            half_w,
        } => {
            let dir = aim.normalized();
            let side = dir.perp();
            for along in [-half_len, half_len] {
                for across in [-half_w, half_w] {
                    grow(center + dir * along + side * across, 0.0);
                }
            }
        }
        MinibossShape::Pincer {
            origin,
            aim,
            len,
            half,
            gap_lo,
            gap_hi,
        } => {
            let dir = aim.normalized();
            let side = dir.perp();
            let sep = gap_lo.max(gap_hi) * 0.5 + half;
            for along in [0.0, len] {
                for across in [-sep, sep] {
                    grow(origin + dir * along + side * across, 0.0);
                }
            }
        }
        MinibossShape::Lane {
            origin,
            aim,
            len,
            seam_half,
            half,
            bar_len,
            covered,
        } => {
            let dir = aim.normalized();
            let side = dir.perp();
            let across = seam_half + half * 2.0;
            let head = covered.min(len);
            let tail = (covered - bar_len).max(0.0);
            for along in [tail, head] {
                for a in [-across, across] {
                    grow(origin + dir * along + side * a, 0.0);
                }
            }
        }
        shape => return shape.signature_bounds().expect("new signature bounds"),
    }
    (lo, hi)
}

#[cfg(test)]
mod tests;

