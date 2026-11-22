//! **Card scenes**: the selected Library card plays the real game.
//!
//! A [`LibraryScene`] owns a small [`World`] of its own — a hand-built patch
//! of open floor (or, for a floor's card, that floor's generated map), a
//! fixed seed, a rat that cannot be hurt, no director — and a short script:
//! spawn the subject, place the rat and a few bodies, feed scripted input to
//! [`World::update`] at a fixed step, and draw the world into the card's art
//! plate. When the script ends the scene is rebuilt from scratch, so every
//! loop is the same loop.
//!
//! Because the card runs the simulation's own systems and the renderer's own
//! draw pass, it cannot show a tell, a footprint or a state the game does not
//! have. Nothing here is read by a run: the scene world is separate from the
//! run's, has its own rng, and never touches the save, the Library's
//! discovery flags, the nest or the campaign.
//!
//! Three plans ([`plan`]):
//!
//! * [`Plan::Sandbox`] — a scene world (bestiary, arsenal, pickups, traps,
//!   hazards, the flatbed's features, floors).
//! * [`Plan::Views`] — the art module's own view struct driven through its
//!   state enum on the card's clock (the rat-hole, the mouse hole, the maze
//!   cache, the one-way door: features whose behaviour is a wall the rat
//!   passes, which a 44 px plate cannot stage).
//! * [`Plan::Still`] — today's pose, with the reason.

use bevy_ecs::entity::Entity;

use crate::game::bullet::{Bullet, Faction};
use crate::game::campaign::CampaignTotals;
use crate::game::collision::{SizeClass, TileMap};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::DashHold;
use crate::game::ecs::systems::boss::{Boss, BossKind};
use crate::game::ecs::systems::miniboss::MinibossKind;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Gold, Pickup, PickupKind};
use crate::game::level::{BossArena, Level, LevelObjects, Palette, Theme, TrailerDeck};
use crate::game::library::{self, Key, ObjectKind};
use crate::game::loadout::{Loadout, PassiveKind};
use crate::game::nav::NavGrid;
use crate::game::roster::{self, Slot};
use crate::game::weapon::{Weapon, WeaponKind};
use crate::game::world::World;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;

/// The scene's fixed simulation step.
pub const TICK: f32 = 1.0 / 60.0;
/// How long the cursor must rest on a card before its scene is built: fast
/// scrolling shows the still pose and builds nothing.
pub const REST_SECS: f32 = 0.15;
/// The most simulation steps one [`LibraryScene::advance`] runs (a stalled
/// frame must not turn into a burst of ticks).
const MAX_STEPS: u32 = 4;
/// Every scene world's seed. A scene is a fixture, not a roll.
const SEED: u64 = 0x11B2_A27;
/// The card rat's health never falls below this share of its maximum.
const HP_FLOOR: f32 = 0.6;

/// How an entry's card moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A scene world running the real schedule.
    Sandbox,
    /// The art module's view struct driven through its real states.
    Views,
    /// The still pose; the string says why.
    Still(&'static str),
}

/// The plan for one registry row. Exhaustive: a new key cannot ship without
/// an answer here.
pub fn plan(key: Key) -> Plan {
    match key {
        Key::Enemy(EnemyKind::Boss) => Plan::Still("the legacy boss body never spawns"),
        Key::Enemy(_) | Key::Miniboss(_) | Key::Boss(_) => Plan::Sandbox,
        Key::Weapon(_) | Key::Evolution(_) => Plan::Sandbox,
        Key::Passive(kind) => match kind {
            PassiveKind::MoveSpeed | PassiveKind::Magnet | PassiveKind::Damage => Plan::Sandbox,
            PassiveKind::FireRate | PassiveKind::LensCap | PassiveKind::LongFuse => Plan::Sandbox,
            PassiveKind::MaxHp | PassiveKind::Regen | PassiveKind::Armor => {
                Plan::Still("a health number: nothing on the floor changes")
            }
            PassiveKind::SpringHeel | PassiveKind::Counterweight | PassiveKind::SecondScent => {
                Plan::Still("a timing or bookkeeping change too small to read on a plate")
            }
        },
        Key::Splice(_) => Plan::Still("a stat trade: the shelf icon is the whole picture"),
        Key::Pickup(_) => Plan::Sandbox,
        Key::Prop(_) => Plan::Still("furniture"),
        Key::Floor(_) => Plan::Sandbox,
        Key::Object(kind) => match kind {
            ObjectKind::Elite
            | ObjectKind::Gold
            | ObjectKind::Chest
            | ObjectKind::StickyBoard
            | ObjectKind::Mousetrap
            | ObjectKind::SteamGrate
            | ObjectKind::InkPad
            | ObjectKind::Pit
            | ObjectKind::SlidingCargo
            | ObjectKind::Slipstream => Plan::Sandbox,
            ObjectKind::RatHole
            | ObjectKind::ShopHole
            | ObjectKind::MazeCache
            | ObjectKind::OneWayDoor => Plan::Views,
        },
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
#[derive(Clone, Copy)]
enum Framing {
    /// The subject and the rat when both fit, else the subject, leaning
    /// toward the rat.
    Pair,
    /// The subject, then its shot in flight all the way to the rat (the
    /// shooters stand off farther than a plate is wide).
    Shot,
    /// The rat.
    Rat,
    /// A fixed world point.
    Point(Vec2),
    /// A point drifting on a slow figure of eight around `0` (a floor's pan).
    Pan(Vec2),
}

/// A trickle of bodies walking in on the rat.
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
}

/// A scene's script: everything but the world.
#[derive(Clone, Copy)]
struct Script {
    loop_secs: f32,
    zoom: f32,
    /// Draw `shrink`× oversized and box-filter down (the boss plates' fit).
    shrink: usize,
    framing: Framing,
    input: fn(f32) -> Input,
    feed: Option<Feed>,
    /// A per-tick hook for the one thing a script cannot say as input (the
    /// moth's death, the rat's wound before a heal).
    cue: Option<fn(&mut World, Option<Entity>, u32)>,
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
            view: Vec2::new(66.0, 45.0),
            pending: 1,
            big: PixelBuffer::new(1, 1),
        };
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
        // and never dies of them.
        let floor = self.world.player.max_hp() * HP_FLOOR;
        if self.world.player.hp < floor {
            self.world.player.hp = floor;
        }
        // No audio: the queue is dropped on the floor.
        self.world.sfx.clear();
        self.world.pending_levelups = 0;
        self.world.pending_chests = 0;
        self.world.pending_shop = false;
        self.tick += 1;
        self.pending = self.pending.saturating_add(1);
        // The camera eases toward what the plate frames.
        let target = self.focus_target();
        self.focus += (target - self.focus) * 0.12;
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
        // Alternate sides, a little above and below the rat's line.
        let n = n as i32;
        let side = if n % 2 == 0 { 1.0 } else { -1.0 };
        let lift = [0.0, 0.35, -0.35][(n.rem_euclid(3)) as usize];
        let dir = Vec2::new(side, lift).normalized();
        let pos = self.world.player.pos + dir * feed.ring;
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

    fn focus_target(&mut self) -> Vec2 {
        let rat = self.world.player.pos;
        match self.script.framing {
            Framing::Rat => rat,
            Framing::Shot => {
                let mut shots = self.world.ecs.query::<(&SpawnId, &Bullet)>();
                let shot = shots
                    .iter(&self.world.ecs)
                    .filter(|(_, b)| b.faction == Faction::Enemy)
                    .min_by_key(|(id, _)| id.0)
                    .map(|(_, b)| b.pos);
                match (shot, self.subject_pos()) {
                    (Some(p), _) => p,
                    (None, Some(p)) => p + (rat - p).normalized() * (self.view.x * 0.25),
                    (None, None) => rat,
                }
            }
            Framing::Point(p) => p,
            Framing::Pan(p) => {
                let t = self.clock() / self.script.loop_secs * std::f32::consts::TAU;
                p + Vec2::new(t.sin() * 30.0, (t * 2.0).sin() * 10.0)
            }
            Framing::Pair => {
                let Some(subject) = self.subject_pos() else {
                    return rat;
                };
                let d = rat - subject;
                let fits = d.x.abs() <= self.view.x * 0.72 && d.y.abs() <= self.view.y * 0.62;
                if fits {
                    subject + d * 0.5
                } else {
                    // Keep the subject a third of the way in from the edge it
                    // is attacking toward.
                    let lean = Vec2::new(
                        d.x.clamp(-self.view.x * 0.22, self.view.x * 0.22),
                        d.y.clamp(-self.view.y * 0.2, self.view.y * 0.2),
                    );
                    subject + lean
                }
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
        let k = self.script.shrink.max(1);
        let zoom = self.script.zoom;
        self.view = Vec2::new((w * k) as f32 * zoom, (h * k) as f32 * zoom);
        self.world.zoom = zoom;
        self.world.zoom_view = zoom;
        self.world.cam_focus = Some(self.focus);
        if k == 1 {
            self.world.draw(art);
            return;
        }
        self.big.resize(w * k, h * k);
        self.world.draw(&mut self.big);
        let n = (k * k) as u32;
        let (bw, src) = (self.big.w, self.big.pixels());
        for y in 0..h {
            for x in 0..w {
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for yy in 0..k {
                    let row = (y * k + yy) * bw + x * k;
                    for px in &src[row..row + k] {
                        r += px.0 as u32;
                        g += px.1 as u32;
                        b += px.2 as u32;
                    }
                }
                art.plot(x as i32, y as i32, ((r / n) as u8, (g / n) as u8, (b / n) as u8));
            }
        }
    }
}

// ---- Stages ----------------------------------------------------------------

/// A patch of open floor: `cols × rows` tiles of the theme's ground, no rooms,
/// no props, nothing placed.
fn patch(theme: Theme, cols: i32, rows: i32) -> Level {
    let tiles = TileMap::new(cols, rows, theme, 0);
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
    }
}

/// A scene world over `level`: no director, no lighting pass, an unarmed rat
/// that cannot be hurt.
fn sandbox(level: Level) -> World {
    let mut loadout = Loadout::starting();
    loadout.weapons.clear();
    let mut world = World::with_level(0, level, SEED, loadout, CampaignTotals::default());
    world.peace = true;
    world.flat_lit = true;
    world.viewport = Vec2::new(88.0, 60.0);
    world
}

type Staged = (World, Script, Option<Entity>);

fn stage(key: Key) -> Option<Staged> {
    let theme = library::theme_of(key);
    Some(match key {
        Key::Enemy(kind) => stage_enemy(theme, kind, false),
        Key::Object(ObjectKind::Elite) => stage_enemy(theme, roster::elite_kind(theme), true),
        Key::Miniboss(kind) => stage_miniboss(theme, kind),
        Key::Boss(kind) => stage_boss(theme, kind),
        Key::Weapon(kind) => stage_weapon(theme, kind, false),
        Key::Evolution(kind) => stage_weapon(theme, kind, true),
        Key::Pickup(kind) => stage_pickup(theme, kind),
        _ => return None,
    })
}

fn stage_enemy(theme: Theme, kind: EnemyKind, elite: bool) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    let shooter = kind.archetype() == EnemyKind::Spitter;
    let at = rat + Vec2::new(if shooter { -84.0 } else { -36.0 }, -5.0);
    let e = world.spawn_enemy_near(kind, at, 0.0, elite);
    let mut script = Script::new(5.0, 1.0);
    if shooter {
        script.framing = Framing::Shot;
    }
    (world, script, Some(e))
}

fn stage_miniboss(theme: Theme, kind: MinibossKind) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    let e = world.spawn_miniboss(kind, 0, rat + Vec2::new(-52.0, -4.0));
    let script = Script::new(6.0, 2.0);
    (world, script, Some(e))
}

fn stage_boss(theme: Theme, kind: BossKind) -> Staged {
    let mut world = sandbox(patch(theme, 44, 34));
    world.player.pos += Vec2::new(54.0, 14.0);
    world.force_boss(kind);
    let mut script = Script::new(10.0, 1.0);
    script.shrink = 3;
    (world, script, None)
}

fn stage_weapon(theme: Theme, kind: WeaponKind, evolved: bool) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let mut weapon = Weapon::new(kind);
    if evolved {
        weapon.level = kind.max_level();
        if let Some(evo) = kind.evolution() {
            world.player.loadout.passives.push((evo.passive, 1));
            world.player.loadout.rebuild_stats();
        }
        weapon.evolved = true;
    } else {
        weapon.level = 3.min(kind.max_level());
    }
    world.player.loadout.weapons.push(weapon);
    let mut script = Script::new(5.0, 1.5);
    script.framing = Framing::Rat;
    script.feed = Some(Feed {
        kind: roster::fodder_kind(theme),
        first: 0.0,
        every: 0.7,
        ring: 34.0,
        hp: 40.0,
        speed: 0.6,
    });
    script.input = match kind {
        WeaponKind::TailThrash | WeaponKind::SonicSqueak => |t| Input {
            active: (t % 1.6) < TICK * 1.5 && t > 0.8,
            ..Input::default()
        },
        WeaponKind::SuperDash => |t| {
            let c = t % 2.5;
            Input {
                mv: Vec2::new(if t < 2.5 { 1.0 } else { -1.0 }, 0.0),
                hold: (0.5..1.5).contains(&c),
                ..Input::default()
            }
        },
        _ => idle,
    };
    (world, script, None)
}

fn stage_pickup(theme: Theme, kind: PickupKind) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    world.spawn_pickup(Pickup {
        pos: rat + Vec2::new(22.0, 0.0),
        kind,
        bob: 0.0,
    });
    for i in 0..6 {
        let a = i as f32 * 1.05 + 0.4;
        world.spawn_gem(Gem::new(rat + Vec2::new(a.cos() * 30.0 + 22.0, a.sin() * 18.0), 1));
    }
    let _ = Gold::new(rat, 1);
    let fodder = roster::kind(theme, Slot::A);
    for (dx, dy) in [(58.0, -16.0), (62.0, 12.0), (-40.0, 10.0)] {
        let e = world.spawn_enemy(fodder, rat + Vec2::new(dx, dy), 1.0);
        if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
            body.speed_scale = 0.45;
        }
    }
    let mut script = Script::new(5.0, 1.5);
    script.framing = Framing::Point(rat + Vec2::new(16.0, 0.0));
    script.input = |t| Input {
        mv: if (1.2..1.75).contains(&t) {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::ZERO
        },
        ..Input::default()
    };
    (world, script, None)
}

#[cfg(test)]
mod tests;
