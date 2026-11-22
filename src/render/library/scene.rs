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
//! discovery flags, the nest or the campaign. Draining the scene's `SfxQueue`
//! and its `pending_*` flags on every step is what keeps a card from opening a
//! level-up plate or a shelf inside a menu.
//!
//! ## Two plans ([`plan`])
//!
//! * [`Plan::Sandbox`] — a scene world: every bestiary body, every named
//!   fight, the weapons and evolutions, the pocket finds, the floor furniture
//!   (traps, hazards, the flatbed's features, the level's interactive objects)
//!   and the six floors. This is the whole index but the passives and the
//!   splices below.
//! * [`Plan::Still`] — today's posed art, with the reason the plate cannot
//!   move. Only what a single plate cannot say is still: a splash-less stat
//!   trade, furniture, and the legacy `Boss` body the game never spawns.
//!
//! ## Keeping the picture honest
//!
//! * The rat is `god`: a card may show the rat being hit, never killed (a
//!   death would end the loop and freeze the plate on `game_over`).
//! * The scene world keeps its own rng (`SEED`), so no card can move a golden
//!   or the map pin.
//! * `flat_lit` is on except for a floor's card, which is the one plate that
//!   promises the floor *lit*.

use bevy_ecs::entity::Entity;

use crate::game::art_traps::{MazeRewardView, MousetrapView, RatHoleView, StickyTrapView};
use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
use crate::game::bullet::{Bullet, Faction};
use crate::game::campaign::CampaignTotals;
use crate::game::collision::{SizeClass, TileKind, TileMap};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{spawn_with_id, DashHold};
use crate::game::ecs::systems::boss::{Boss, BossKind};
use crate::game::ecs::systems::miniboss::MinibossKind;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Gold, Pickup, PickupKind};
use crate::game::hazards::{InkPad, SteamGrate};
use crate::game::level::{
    BossArena, Level, LevelObjects, OneWayDoor, Palette, Pit, PitKind, RatHole, ShopHole,
    ShopHoleRole, SlidingCargo, Theme, TileRect, TrailerDeck,
};
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
    /// A point drifting on a slow figure of eight around a fixed point (a
    /// floor's pan).
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
    /// A fixed approach direction from the rat (a trap wants its bodies to
    /// walk over it). `None` alternates sides, a little above and below.
    from: Option<Vec2>,
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
    /// A per-tick hook for the one thing a script cannot say as input.
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
        // and never dies of them: `Player::god` is the guarantee, and the floor
        // below is the belt for anything that writes `hp` directly.
        let floor = self.world.player.max_hp() * 0.6;
        if self.world.player.hp < floor {
            self.world.player.hp = floor;
        }
        // No audio, and no plate: the card must never come back as a level-up,
        // a chest or the shelf. The queue is dropped on the floor.
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

/// A patch of open floor: `cols × rows` tiles of the theme's ground, a solid
/// border (so a scripted rat cannot walk off the map), no rooms, no props,
/// nothing placed.
fn patch(theme: Theme, cols: i32, rows: i32) -> Level {
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
    }
}

/// A patch with a solid wall down column `wall_x` and one cell of `gap` in it
/// — a rat-hole, a doorway or a one-way door, the only way through.
fn walled(theme: Theme, cols: i32, rows: i32, wall_x: i32, gap_y: i32, gap: TileKind) -> Level {
    assert!(wall_x > 1 && wall_x < cols - 2, "the wall must be on the patch");
    assert!(gap_y > 1 && gap_y < rows - 2, "the gap must be on the wall");
    let mut level = patch(theme, cols, rows);
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

/// A scene world over `level`: no director, no lighting pass, an unarmed rat
/// that cannot be hurt.
fn sandbox(level: Level) -> World {
    let mut loadout = Loadout::starting();
    loadout.weapons.clear();
    let mut world = World::with_level(0, level, SEED, loadout, CampaignTotals::default());
    world.peace = true;
    world.flat_lit = true;
    // `god` is what makes "the card rat never dies" a property of the game
    // rather than a hope: nothing writes `player.hp` directly.
    world.player.god = true;
    world.viewport = Vec2::new(88.0, 60.0);
    world
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
        Key::Enemy(kind) => Some(stage_enemy(theme, kind, false)),
        Key::Miniboss(kind) => Some(stage_miniboss(theme, kind)),
        Key::Boss(kind) => Some(stage_boss(theme, kind)),
        Key::Weapon(kind) => Some(stage_weapon(theme, kind, false)),
        Key::Evolution(kind) => Some(stage_weapon(theme, kind, true)),
        Key::Pickup(kind) => Some(stage_pickup(theme, kind)),
        Key::Floor(theme) => Some(stage_floor(theme)),
        Key::Passive(kind) => stage_passive(theme, kind),
        Key::Object(kind) => Some(stage_object(theme, kind)),
        // `plan` answers `Still` for everything below, so `build` never gets
        // here; a caller that skipped the plan gets nothing rather than a
        // wrong picture.
        Key::Splice(_) | Key::Prop(_) => None,
    }
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
    // The rat stands just outside the boss's body contact, on the side its
    // attacks are aimed at, so the plate frames the animal and the space it is
    // about to take.
    world.player.pos += Vec2::new(30.0, 6.0);
    world.force_boss(kind);
    let mut script = Script::new(10.0, 1.0);
    // Drawn 2× oversized and box-filtered down: the art modules draw at 1:1
    // only, and every boss sprite is 26–84 px long against a 44 px plate.
    script.shrink = 2;
    (world, script, None)
}

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
        weapon.level = 3.min(kind.max_level());
    }
    world.player.loadout.weapons.push(weapon);
    // The rat faces its dummies (a Laser Eyes card fires down `face`, and an
    // idle rat never turns on its own).
    world.player.face = Vec2::new(1.0, 0.0);
    let mut script = Script::new(5.0, 1.5);
    script.framing = Framing::Rat;
    script.feed = Some(Feed {
        kind: roster::fodder_kind(theme),
        first: 0.0,
        every: 0.7,
        ring: 34.0,
        hp: 40.0,
        speed: 0.6,
        from: None,
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
    if tick % 24 != 0 {
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

/// A floor's card: the real generated map, lit by its real fixtures, with the
/// floor's two fodder kinds walking through the start room while the plate
/// pans.
fn stage_floor(theme: Theme) -> Staged {
    let level = Level::generate(theme, SEED);
    let start = level.start;
    let mut world = sandbox(level);
    // The one card that promises the floor *lit*.
    world.flat_lit = false;
    for (i, slot) in [Slot::A, Slot::B].into_iter().enumerate() {
        let kind = roster::kind(theme, slot);
        let side = if i == 0 { -1.0 } else { 1.0 };
        let e = world.spawn_enemy(kind, start + Vec2::new(side * 30.0, 26.0), 1.0);
        if let Some(mut body) = world.ecs.get_mut::<Enemy>(e) {
            body.speed_scale = 0.7;
        }
    }
    let mut script = Script::new(10.0, 1.5);
    script.framing = Framing::Pan(start + Vec2::new(0.0, 14.0));
    (world, script, None)
}

/// What a flatbed card shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeckFeature {
    Pit,
    Cargo,
    Wind,
}

fn stage_object(theme: Theme, kind: ObjectKind) -> Staged {
    match kind {
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
    }
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
    if tick % 20 != 0 {
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
    let mut script = Script::new(6.0, 2.0);
    script.framing = Framing::Point(chest - Vec2::new(8.0, 0.0));
    (world, script, None)
}

/// A glue board takes a pack of Brutes one after another until its lifetime
/// budget is gone: `Idle` → `Stuck` → `Used`.
fn stage_sticky(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
    let rat = world.player.pos;
    let board = rat + Vec2::new(0.0, 26.0);
    world
        .level
        .objects
        .sticky_traps
        .push(StickyTrapView::new(board, Vec2::new(1.0, 0.0)));
    let mut script = Script::new(9.0, 1.5);
    script.framing = Framing::Point(board + Vec2::new(0.0, -8.0));
    // The rat stands still, above the board: every Brute is fed in from below
    // it, so each one has to cross the glue to reach the rat.
    script.feed = Some(Feed {
        kind: EnemyKind::Brute,
        first: 0.5,
        every: 0.7,
        ring: 66.0,
        hp: 120.0,
        speed: 0.5,
        from: Some(Vec2::new(0.0, 1.0)),
    });
    (world, script, None)
}

/// A baited mousetrap: `Armed` → `Triggered` → `Snapping` → `Sprung`, walked
/// into by the floor's fodder.
fn stage_mousetrap(theme: Theme) -> Staged {
    let mut world = sandbox(patch(theme, 40, 30));
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

/// A steam grate's whole cycle (resting 4 s, warning 1 s, plume 0.7 s) with a
/// body walking over it.
fn stage_steam() -> Staged {
    let mut world = sandbox(patch(Theme::Laundry, 40, 30));
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

/// An ink pad: the rat and the pack wade through a permanent 75 % stain.
fn stage_ink() -> Staged {
    let mut world = sandbox(patch(Theme::Printworks, 40, 30));
    let rat = world.player.pos;
    let cells = TileRect::new((rat.x as i32) / 16 + 2, (rat.y as i32) / 16 - 1, 3, 3);
    world
        .level
        .objects
        .ink_pads
        .push(InkPad::new(cells, 7));
    let mut script = Script::new(5.0, 1.5);
    script.framing = Framing::Point(cells.center_world() - Vec2::new(14.0, 0.0));
    script.input = |t| Input {
        // Wading in and back out again: the stain's 75 % is the picture.
        mv: if (t % 1.8) < 0.9 {
            Vec2::new(1.0, 0.0)
        } else {
            Vec2::new(-1.0, 0.0)
        },
        ..Input::default()
    };
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

/// One of the flatbed's own hazards on a hand-built deck.
fn stage_deck(feature: DeckFeature) -> Staged {
    let mut level = patch(Theme::Trailer, 32, 22);
    let rat = level.start;
    let mut deck = TrailerDeck {
        deck: TileRect::new(1, 1, 30, 20),
        tailgate: Vec2::new(20.0, rat.y),
        pits: Vec::new(),
        wind: Vec::new(),
        slides: Vec::new(),
    };
    let cells = |p: Vec2| crate::game::collision::cell_of(p);
    let mut script = Script::new(5.0, 1.5);
    match feature {
        DeckFeature::Pit => {
            let pit_at = Vec2::new(rat.x + 44.0, rat.y);
            let c = cells(pit_at);
            deck.pits.push(Pit::new(TileRect::new(c.0, c.1 - 1, 2, 3), PitKind::BrokenPlank));
            script.framing = Framing::Point(TileRect::new(c.0, c.1 - 1, 2, 3).center_world());
            // Bodies walk in from the far side and fall in: the deck's horde
            // rule is the loudest half of the hazard.
            script.feed = Some(Feed {
                kind: EnemyKind::Skitterer,
                first: 0.3,
                every: 1.1,
                ring: 78.0,
                hp: 40.0,
                speed: 0.8,
                from: Some(Vec2::new(1.0, 0.0)),
            });
        }
        DeckFeature::Cargo => {
            let rest = rat + Vec2::new(28.0, 0.0);
            deck.slides.push(SlidingCargo {
                rest,
                dir: Vec2::new(1.0, 0.0),
                travel: 14.0,
                phase: 0.0,
                half: Vec2::new(8.0, 8.0),
                warn: 0.6,
            });
            script.framing = Framing::Point(rest + Vec2::new(4.0, 0.0));
            script.loop_secs = 8.0;
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
            let c = cells(rat + Vec2::new(34.0, 0.0));
            let wind = TileRect::new(c.0, c.1 - 2, 6, 5);
            deck.wind.push(wind);
            script.framing = Framing::Point(wind.center_world());
        }
    }
    level.trailer = deck;
    let mut world = sandbox(level);
    if feature == DeckFeature::Wind {
        // Gems on the exposed stretch blow tailward (the slipstream's loot
        // half), so the plate shows the wind without a single sfx.
        let wind = world.level.trailer.wind[0].center_world();
        for i in 0..6 {
            let p = Vec2::new(
                wind.x + 10.0 + (i % 3) as f32 * 8.0,
                wind.y - 16.0 + (i / 3) as f32 * 26.0,
            );
            world.spawn_gem(Gem::new(p, 1));
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
        theme,
        40,
        30,
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
    let mut script = Script::new(5.0, 1.5);
    script.input = |t| Input {
        mv: if (0.5..2.4).contains(&t) {
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
        theme,
        40,
        30,
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
    let mut script = Script::new(4.5, 1.5);
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

/// A mouse-hole shop in a wall: the rat walks up to the threshold and the
/// mouth answers.
fn stage_shop_hole(theme: Theme) -> Staged {
    let wall_x = 20;
    let gap_y = 15;
    let cell = (wall_x, gap_y);
    let pos = cell_centre(cell);
    let mut level = walled(theme, 40, 30, wall_x, gap_y, TileKind::Wall(WallKind::Solid));
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
    let mut script = Script::new(5.0, 1.5);
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
    let mut script = Script::new(4.5, 1.5);
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

#[cfg(test)]
mod tests;
