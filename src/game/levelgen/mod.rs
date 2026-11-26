//! Procedural level generation.
//!
//! Layout and placement stages run in this order (the file-ownership
//! table with signatures is in `CONTENT_PLAN.md` §2):
//!
//! 1. [`layout::build_layout`] — rooms, corridors, open halls, the far hall and
//!    the start room; carves them into the tile grid and returns the named
//!    regions. *(generator worker A)*
//! 2. [`layout::furnish`] — the theme's props inside the rooms, painted into
//!    the grid as solid cells. *(worker A)*
//! 3. [`maze::carve_mazes`] — **at most one** maze section in the reserved
//!    slot (a maze is a find: about 1 map in 8 carries one, see
//!    [`layout::MAZE_ONE_IN`]), with a reward at its end and a one-way door as
//!    its exit. *(worker B)*
//! 4. [`placement::place_rat_holes`] — rat-holes in structural walls, two of
//!    them chosen for the route they serve. *(worker P2)*
//! 5. [`placement::place_traps`] — sticky traps and mousetraps in corridors
//!    and rooms. *(worker P2)*
//! 6. [`placement::place_shop_holes`] — the two shop holes in structural wall
//!    faces.
//! 7. `lights::place` — light fixtures and ambient emitters, on their own
//!    salted rng over the finished tiles.
//!
//! Then [`validate`] proves the map is playable; [`generate`] retries with a
//! salted seed when it is not. The retry is a **safety net, not a routine
//! path**: the first attempt validates on every map in the sweep
//! (`layout::tests::every_map_validates_on_its_first_attempt`, and thousands of
//! seeds per theme under `RATRUN_LEVEL_STATS`), so a retry means a generator
//! bug rather than bad luck.
//!
//! ## Determinism
//!
//! One [`StdRng`] seeded from `(theme, run seed, attempt)` drives gameplay
//! placements in order. Fixtures and ambient emitters read the finished tiles
//! and use a separate, salted RNG. There are no clocks or hash-map iteration
//! in generation.
//!
//! ## Cost
//!
//! One map costs about 3 ms in the dev profile (median over 300 seeds, best of
//! three samples; p95 ≈ 7 ms), down from ≈ 60 ms before placement's searches
//! were bounded. `levelgen::tests::level_stage_timings` is the measurement:
//!
//! ```text
//! RATRUN_LEVEL_TIMINGS=100 cargo test level_stage_timings -- --nocapture
//! ```
//!
//! and `levelgen::tests::map_trace_pin` pins the exact output (tile grid and
//! every placed object) of 20 seeds on each of the five campaign themes (100
//! maps), so a cost change cannot quietly change
//! which holes and traps a seed grows.
//!
//! ## Measuring it
//!
//! ```text
//! RATRUN_LEVEL_STATS=2500 cargo test level_statistics -- --nocapture
//! ```
//!
//! prints the designer-facing distributions (rooms, halls, corridors, largest
//! hall size and distance, mazes per map and their distance, open-floor share)
//! over that many seeds per theme, and fails if any first attempt does not
//! validate.

pub mod hazards;
pub mod layout;
mod lights;
pub mod maze;
/// The flatbed trailer's own generator (see the module docs).
pub mod trailer;
/// Measurement only (`cfg(test)`): the open-world numbers, the map dump and
/// the survival probe. See the module docs for the environment variables.
#[cfg(test)]
pub mod metrics;
pub mod placement;
/// Standing water (the wet floors' puddles); its own salted rng, run last.
pub mod puddles;
/// Decorative clothes piles and overflowing baskets (Laundry only); its own salted rng, run last.
pub mod laundry_piles;
pub mod validate;

use crate::math::Vec2;

use super::art_traps::{MazeRewardView, MousetrapView, RatHoleView, StickyTrapView};
use super::collision::{cardinal_vec, SizeClass, TileMap};
use super::level::{
    level_rng, level_seed, Level, LevelObjects, MazeSection, OneWayDoor, Palette, RatHole,
    RatHoleRecord, Theme, TrapRecord, MAP_COLS, MAP_ROWS,
};
use super::nav::NavGrid;

/// How many differently-salted maps to try before giving up on validation.
///
/// Retries are part of the deterministic contract: the same seed always
/// produces the same map *and* the same retry outcome.
pub const MAX_ATTEMPTS: u32 = 4;

/// Generate the level for `(theme, seed)` and prove it is playable.
///
/// A map that fails [`validate`] is thrown away and regenerated from a salted
/// seed (up to [`MAX_ATTEMPTS`]). In debug builds a map that never validates
/// panics with the validator's report — that is a generator bug, and every
/// test that generates a map will show it. In a release build the last
/// attempt is used so the player still gets a runnable map.
pub fn generate(theme: Theme, seed: u64) -> Level {
    let mut last: Option<(Level, Vec<String>)> = None;
    let mut outfall_attempts = Vec::new();
    for attempt in 0..MAX_ATTEMPTS {
        let level = generate_once(theme, seed, attempt);
        match validate::validate(&level) {
            Ok(()) => return level,
            Err(errors) if theme == Theme::Outfall => outfall_attempts.push((attempt, level, errors)),
            Err(errors) => last = Some((level, errors)),
        }
    }
    for (attempt, mut level, errors) in outfall_attempts {
        if repair_outfall_lures(&mut level, attempt) {
            return level;
        }
        last = Some((level, errors));
    }
    let (level, errors) = last.expect("MAX_ATTEMPTS is at least one");
    if cfg!(debug_assertions) {
        panic!("level generation failed validation for {theme:?} seed {seed}: {errors:#?}");
    }
    level
}

fn repair_outfall_lures(level: &mut Level, attempt: u32) -> bool {
    let rank_seed = level_seed(level.theme, level.seed, attempt);
    hazards::place_lure_traps(level, rank_seed, true);
    if validate::validate(level).is_err() {
        return false;
    }
    puddles::place(level, attempt);
    true
}

/// One generation attempt: the five stages, then the derived data (masks, the
/// legacy wall mirror, the navigation mask).
pub fn generate_once(theme: Theme, seed: u64, attempt: u32) -> Level {
    generate_stages(theme, seed, attempt, None)
}

/// Wall-clock cost of one [`generate_once`], in **microseconds**, per stage.
///
/// Only [`generate_once_timed`] fills this in: production generation never
/// reads the clock, so the hot path stays a plain branch on `None`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    pub layout: u64,
    pub furnish: u64,
    pub mazes: u64,
    pub holes: u64,
    pub traps: u64,
    /// Mouse-hole shops (the two per level; no rng, a graph query only).
    pub shop: u64,
    pub hazards: u64,
    pub masks: u64,
    pub objects: u64,
    pub nav: u64,
    /// Render-only fixtures and ambient emitter placement.
    pub lights: u64,
    /// Standing water (`puddles::place`).
    pub puddles: u64,
    /// Laundry clothes piles (`laundry_piles::place`): the last stage of all.
    pub laundry_piles: u64,
}

impl StageTimings {
    /// Element-wise minimum, so repeated samples of one map report the cost
    /// without scheduler noise.
    pub fn min(self, other: StageTimings) -> StageTimings {
        StageTimings {
            layout: self.layout.min(other.layout),
            furnish: self.furnish.min(other.furnish),
            mazes: self.mazes.min(other.mazes),
            holes: self.holes.min(other.holes),
            traps: self.traps.min(other.traps),
            shop: self.shop.min(other.shop),
            hazards: self.hazards.min(other.hazards),
            masks: self.masks.min(other.masks),
            objects: self.objects.min(other.objects),
            nav: self.nav.min(other.nav),
            lights: self.lights.min(other.lights),
            puddles: self.puddles.min(other.puddles),
            laundry_piles: self.laundry_piles.min(other.laundry_piles),
        }
    }

    pub fn total(&self) -> u64 {
        self.layout
            + self.furnish
            + self.mazes
            + self.holes
            + self.traps
            + self.shop
            + self.hazards
            + self.masks
            + self.objects
            + self.nav
            + self.lights
            + self.puddles
            + self.laundry_piles
    }
}

/// [`generate_once`], with the cost of each stage measured.
///
/// Used by `levelgen::tests::level_stage_timings`
/// (`RATRUN_LEVEL_TIMINGS=100 cargo test level_stage_timings -- --nocapture`)
/// and by the placement tests that profile one stage on its own.
pub fn generate_once_timed(theme: Theme, seed: u64, attempt: u32) -> (Level, StageTimings) {
    let mut timings = StageTimings::default();
    let level = generate_stages(theme, seed, attempt, Some(&mut timings));
    (level, timings)
}

/// The pipeline itself. `timings` is `None` for every production call.
fn generate_stages(
    theme: Theme,
    seed: u64,
    attempt: u32,
    mut timings: Option<&mut StageTimings>,
) -> Level {
    /// Run one stage, recording its cost when profiling.
    macro_rules! stage {
        ($field:ident, $body:expr) => {{
            let started = timings.is_some().then(std::time::Instant::now);
            let value = $body;
            if let (Some(slot), Some(started)) = (timings.as_deref_mut(), started) {
                slot.$field = started.elapsed().as_micros() as u64;
            }
            value
        }};
    }

    let mut rng = level_rng(theme, seed, attempt);
    let art_seed = level_seed(theme, seed, attempt) as u32;
    let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, art_seed);

    // The flatbed is its own world: a long, narrow bed of planks rather than a
    // sprawl of rooms, so it owns the whole pipeline (see `trailer.rs`).
    if theme == Theme::Trailer {
        let mut level = trailer::generate(tiles, &mut rng, seed);
        stage!(lights, lights::place(&mut level, attempt));
        // No puddles on the Flatbed: `puddles::count_for` has no row for it and
        // the stage is not run here at all, so the ride is dry.
        return strip_boss_arena(level);
    }

    let design = stage!(layout, layout::build_layout(&mut tiles, &mut rng, theme));
    let props = stage!(furnish, layout::furnish(&mut tiles, &design, &mut rng, theme));
    let mazes = stage!(mazes, maze::carve_mazes(&mut tiles, &design, &mut rng, theme));
    let holes = stage!(
        holes,
        placement::place_rat_holes(&mut tiles, &design, &mut rng)
    );
    let mut traps = stage!(
        traps,
        placement::place_traps(&tiles, &design, &mazes, &mut rng)
    );
    // The two mouse-hole shops. No rng: the walls are chosen by walking
    // distance, so this stage cannot move an existing drop roll or hole.
    let shop_holes = stage!(
        shop,
        placement::place_shop_holes(&tiles, &design, &mazes)
    );
    // A `40 × 24` snap trap (or a glue board) must never crowd a shop
    // threshold. The shops are placed after the traps and draw no rng, so the
    // prune can only ever remove a trap.
    placement::drop_traps_crowding_shops(&mut traps, &shop_holes);
    // The floor hazards (steam grates, ink pads and the Sewer drip shaft).
    // Kitchen and Lab return immediately without drawing. After the shop
    // holes, so a hazard is never placed on (or within two cells of) a shop
    // threshold: the shops draw no rng, so their order against this stage
    // cannot move a map.
    let floor_hazards = stage!(
        hazards,
        hazards::place_hazards(&tiles, &design, &mazes, &traps, &shop_holes, &mut rng, theme)
    );

    stage!(masks, tiles.rebuild_masks());

    let objects = stage!(
        objects,
        objects_from(
            &tiles,
            &mazes,
            &holes,
            &traps,
            &shop_holes,
            &floor_hazards,
            level_seed(theme, seed, attempt),
        )
    );
    let nav = stage!(nav, NavGrid::from_tiles(&tiles, SizeClass::Large));
    let mut level = Level {
        theme,
        seed,
        arena: tiles.world_size(),
        tiles,
        props,
        palette: Palette::for_theme(theme),
        nav,
        rooms: design.rooms.clone(),
        start: design.start,
        mazes: mazes.clone(),
        objects,
        boss: crate::game::level::BossArena::default(),
        trailer: crate::game::level::TrailerDeck::default(),
        puddles: Vec::new(),
        clothes_piles: Vec::new(),
    };
    if theme == Theme::Outfall {
        stage!(hazards, {
            let rank_seed = level_seed(theme, seed, attempt);
            hazards::place_tidal_pits(&mut level, rank_seed);
            hazards::place_lure_traps(&mut level, rank_seed, false);
        });
    }
    stage!(lights, lights::place(&mut level, attempt));
    let mut level = strip_boss_arena(level);
    // Standing water: on its own salted rng over the finished grid (`puddles`'s
    // module docs). It writes only `Level::puddles`, which the map pin does not
    // fold, and it draws nothing another stage reads, so no earlier stage can
    // move.
    stage!(puddles, puddles::place(&mut level, attempt));
    // Laundry clothes piles and overflowing baskets: the last stage of all,
    // Laundry only, on its own salted rng, writing only `Level::clothes_piles`.
    stage!(laundry_piles, laundry_piles::place(&mut level, attempt));
    level
}

/// Make a generated level arena-free (batch 12).
///
/// The generator no longer builds a boss arena, and the boss arrives where the
/// rat is (`Level::fight_rect_around`). The themes' own generators still
/// describe the room that used to be one — the Flatbed's cab hold is the last
/// one — so this is the single place the last of it is taken away: a
/// [`crate::game::level::RoomKind::Boss`] room becomes an ordinary
/// [`crate::game::level::RoomKind::Hall`] and the level's
/// [`crate::game::level::BossArena`] furniture is cleared. Nothing else about
/// the map changes: the tile grid, the props, the objects and the lights are
/// already painted.
fn strip_boss_arena(mut level: Level) -> Level {
    use crate::game::level::{BossArena, LightFixtureKind, LightZoneKind, RoomKind};

    for room in &mut level.rooms {
        if room.kind == RoomKind::Boss {
            room.kind = RoomKind::Hall;
        }
    }
    // `lights::place` runs before this, so a theme's own generator can have
    // zoned or lit its former arena: those become ordinary room lights.
    for zone in &mut level.objects.light_zones {
        if zone.kind == LightZoneKind::Arena {
            zone.kind = LightZoneKind::Room;
        }
    }
    for fixture in &mut level.objects.lights {
        if fixture.kind == LightFixtureKind::Arena {
            fixture.kind = LightFixtureKind::Pendant;
        }
    }
    level.boss = BossArena::default();
    level
}


/// Build the runtime [`LevelObjects`] from the generator's placements: the
/// art modules' own view structs, initialised to their idle states.
pub(super) fn objects_from(
    tiles: &TileMap,
    mazes: &[MazeSection],
    holes: &[placement::RatHolePlacement],
    traps: &[placement::TrapPlacement],
    shop_holes: &[placement::ShopHolePlacement],
    floor_hazards: &hazards::HazardPlacement,
    // The level's seed, keying the independent per-trap bait/kind stream (it
    // draws nothing from the generation rng, so it cannot move the map).
    spec_seed: u64,
) -> LevelObjects {
    // The wave 3 floor hazards: the generator's own view structs, straight in.
    let mut objects = LevelObjects {
        steam_grates: floor_hazards.steam.clone(),
        ink_pads: floor_hazards.ink.clone(),
        sewer_opening: floor_hazards.sewer_opening.clone(),
        ..LevelObjects::default()
    };

    // Rat-holes: one view per hole cell, facing along the passage toward the
    // public approach side (the placement's sign, not just the axis), plus the
    // generator's record of why the hole exists and which side is public.
    for hole in holes {
        let cell = hole.cell;
        let facing = cardinal_vec(hole.approach);
        let center = TileMap::tile_rect(cell.0, cell.1).center();
        objects.rat_holes.push(RatHole {
            cell,
            view: RatHoleView::new(center, facing),
        });
        objects.rat_hole_records.push(RatHoleRecord {
            role: hole.role,
            approach: hole.approach,
        });
    }

    // The two mouse-hole shops (stage B's placement, dressed as the level's
    // own record: the shelf and the threshold system read this).
    for (index, hole) in shop_holes.iter().enumerate() {
        objects.shop_holes.push(crate::game::level::ShopHole {
            index,
            cell: hole.cell,
            pos: TileMap::tile_rect(hole.cell.0, hole.cell.1).center(),
            facing: cardinal_vec(hole.approach),
            approach: hole.approach,
            mouth: hole.mouth,
            role: hole.role,
        });
    }

    // One-way doors and the reward that ends each maze.
    for section in mazes {
        let dir = cardinal_vec(section.exit_dir);
        objects
            .one_way_doors
            .push(OneWayDoor::new(section.exit, dir));
        let exit_center = TileMap::tile_rect(section.exit.0, section.exit.1).center();
        let facing = (exit_center - section.reward).normalized();
        let facing = if facing.len_sq() > 0.5 {
            facing
        } else {
            Vec2::new(1.0, 0.0)
        };
        objects
            .rewards
            .push(MazeRewardView::new(section.reward, facing));
    }

    // Traps (floor hazards: they never paint a cell). Each view carries the
    // generator's record (encounter or incidental, and the arriving side) in
    // the parallel vector at the same index.
    for t in traps {
        let record = TrapRecord {
            encounter: t.encounter,
            approach: t.approach,
        };
        match t.kind {
            placement::TrapKind::Sticky => {
                objects
                    .sticky_traps
                    .push(StickyTrapView::new(t.pos, t.facing));
                objects.sticky_trap_records.push(record);
            }
            placement::TrapKind::Snap => {
                // Wave 3: every set trap carries a bar kind and a bait, chosen
                // from its own deterministic stream at generation time.
                let (kind, bait) = placement::snap_spec(spec_seed, t.pos);
                objects
                    .mousetraps
                    .push(MousetrapView::baited(t.pos, t.facing, kind, bait));
                objects.mousetrap_records.push(record);
            }
        }
    }

    // Doors are authored by `carve_mazes`, so their cells must exist.
    debug_assert!(
        objects
            .one_way_doors
            .iter()
            .all(|d| tiles.one_way_dir(d.cell.0, d.cell.1).is_some()),
        "one-way door placed on a cell that is not a OneWayDoor wall"
    );

    objects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
    use crate::game::collision::TileKind;
    use crate::game::level::{RatHoleRole, RoomKind};

    // ---- The map trace pin -------------------------------------------------
    //
    // Optimising the generator must not change *what* it generates. This pin
    // hashes the whole tile grid (every cell's kind and neighbour mask) and
    // every placed object (cell, world position, facing) for 20 seeds per campaign theme (100 maps), so a
    // refactor that alters which holes or traps are chosen, or moves one by a
    // cell, fails here instead of silently in a player's run.
    //
    // The values were recorded from the generator **before** the placement
    // cost work (the same stage output, just slower). If a *layout* or *maze*
    // stage change makes this fail, the pin needs regenerating (integrator) —
    // print it with `RATRUN_LEVEL_TRACE=1 cargo test map_trace_pin -- --nocapture`.

    /// Seeds per theme (`0..TRACE_SEEDS`): 5 × 20 = 100 pinned maps.
    const TRACE_SEEDS: u64 = 20;

    /// Pinned trace, `[theme][seed]`, in the original five-theme order.
    /// The Sewer row includes the authored opening layout (shaft, cap and Pillbug cells).
    const TRACE_HASHES: [[u64; TRACE_SEEDS as usize]; 5] = [
        [
            10832721958084521709,
            11449579157051971039,
            7177082262695515155,
            1937950407477618921,
            10090667170217652087,
            2802282806743630607,
            12581247162618467954,
            9760912417913437243,
            3811232889367773881,
            9083087300871067692,
            13956924747722307213,
            4069275993194223053,
            15873320110997350559,
            17920790026558822162,
            7586902183204826288,
            5387443381940272038,
            18352361035439511791,
            10577491350045340411,
            9186312933502842503,
            11825709167235093016,
        ],
        [
            2022908987977853186,
            10458584545441928764,
            10968261466279748939,
            5448941178267367717,
            11713479263895906942,
            3840157771133177466,
            5145912062345644220,
            9505348330936205212,
            10307118091017312214,
            7057743618538330296,
            411125678842618689,
            8551705656137253867,
            9362434510014654182,
            8755110305313840974,
            1903672601652543086,
            14441073405275002535,
            17951629672828530648,
            11776904961408709160,
            7095830473849507578,
            2595039895547006177,
        ],
        [
            15128152747255210787,
            9183533470009517399,
            2608389098307680429,
            4437052308364505605,
            4944379373348968975,
            6229043342073822714,
            12935309819400149299,
            13450478728939217872,
            17410487776348564377,
            2316938695199510427,
            9408564482974240055,
            15190270023416182127,
            17312556243128149571,
            2018345762220046634,
            9939655680731750461,
            16552488554363124947,
            13169709758328033871,
            2588604818994507737,
            9596587971999436786,
            16564480185206401729,
        ],
        [
            14773806120003153490,
            6766351403375990969,
            18296347923801463894,
            11163207921832330729,
            214656540143688640,
            17469156078929724265,
            10589934380225918874,
            16844547639973693246,
            13038016783502788048,
            15934680715917080087,
            15214859026079533469,
            12423987704747220611,
            13369567648541251145,
            16448850914901540918,
            17529272602156802592,
            15081162102766116796,
            15246478572752053259,
            15869551028044234051,
            12291988103901469390,
            3048400843203054115,
        ],
        [
            16765315462351781438,
            7418216091588320109,
            6430218183988654622,
            13397238647140415233,
            17924033443761018082,
            7369394754283053889,
            8242955930999033563,
            8932332441222053281,
            7343148119434242293,
            6624961117964325445,
            3421381815066593856,
            441888048213918023,
            8719359398798657643,
            4527943986620740905,
            2533252422578643402,
            5187108554049276502,
            17730480211229268387,
            17901311143199154906,
            9695934112510945482,
            16897342032463284409,
        ],
    ];

    /// FNV-1a over the bytes fed to it: stable across platforms and runs.
    struct Trace(u64);

    impl Trace {
        fn new() -> Trace {
            Trace(0xcbf2_9ce4_8422_2325)
        }

        fn byte(&mut self, b: u8) {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }

        fn u32(&mut self, v: u32) {
            for b in v.to_le_bytes() {
                self.byte(b);
            }
        }

        fn u64(&mut self, v: u64) {
            for b in v.to_le_bytes() {
                self.byte(b);
            }
        }

        fn f32(&mut self, v: f32) {
            self.u32(v.to_bits());
        }

        fn vec2(&mut self, v: Vec2) {
            self.f32(v.x);
            self.f32(v.y);
        }

        fn cell(&mut self, cell: (i32, i32)) {
            self.u32(cell.0 as u32);
            self.u32(cell.1 as u32);
        }
    }

    fn axis_code(axis: WallAxis) -> u32 {
        match axis {
            WallAxis::Horizontal => 0,
            WallAxis::Vertical => 1,
        }
    }

    fn role_code(role: RatHoleRole) -> u32 {
        match role {
            RatHoleRole::Excursion => 0,
            RatHoleRole::Gathering => 1,
            RatHoleRole::Incidental => 2,
        }
    }

    fn cardinal_code(dir: Cardinal) -> u32 {
        match dir {
            Cardinal::North => 0,
            Cardinal::East => 1,
            Cardinal::South => 2,
            Cardinal::West => 3,
        }
    }

    fn kind_code(kind: TileKind) -> u32 {
        match kind {
            TileKind::Open => 0,
            TileKind::Wall(WallKind::Solid) => 1,
            TileKind::Wall(WallKind::Maze) => 2,
            TileKind::Wall(WallKind::RatHole { axis }) => 3 + axis_code(axis),
            TileKind::Wall(WallKind::Doorway { axis }) => 5 + axis_code(axis),
            TileKind::Wall(WallKind::OneWayDoor { dir }) => 7 + cardinal_code(dir),
        }
    }

    /// The whole map's identity: tile grid + placed objects.
    fn map_hash(level: &Level) -> u64 {
        let mut trace = Trace::new();
        let (cols, rows) = (level.tiles.cols(), level.tiles.rows());
        trace.u32(cols as u32);
        trace.u32(rows as u32);
        for cy in 0..rows {
            for cx in 0..cols {
                let cell = level.tiles.cell(cx, cy);
                trace.u32(kind_code(cell.kind));
                trace.byte(cell.mask.0);
            }
        }
        let objects = &level.objects;
        trace.u32(objects.rat_holes.len() as u32);
        for (hole, record) in objects.rat_holes.iter().zip(&objects.rat_hole_records) {
            trace.cell(hole.cell);
            trace.vec2(hole.view.pos);
            trace.vec2(hole.view.facing);
            trace.u32(role_code(record.role));
            trace.u32(cardinal_code(record.approach));
        }
        trace.u32(objects.sticky_traps.len() as u32);
        for (trap, record) in objects
            .sticky_traps
            .iter()
            .zip(&objects.sticky_trap_records)
        {
            trace.vec2(trap.pos);
            trace.vec2(trap.facing);
            trace.byte(u8::from(record.encounter));
            trace.u32(cardinal_code(record.approach));
        }
        trace.u32(objects.mousetraps.len() as u32);
        for (trap, record) in objects
            .mousetraps
            .iter()
            .zip(&objects.mousetrap_records)
        {
            trace.vec2(trap.pos);
            trace.vec2(trap.facing);
            trace.byte(u8::from(record.encounter));
            trace.u32(cardinal_code(record.approach));
        }
        trace.u32(objects.one_way_doors.len() as u32);
        for door in &objects.one_way_doors {
            trace.cell(door.cell);
            trace.vec2(door.view.pos);
            trace.vec2(door.view.facing);
        }
        trace.u32(objects.rewards.len() as u32);
        for reward in &objects.rewards {
            trace.vec2(reward.pos);
            trace.vec2(reward.facing);
        }
        // The Sewer opening intentionally extends the pinned map output.
        if let Some(opening) = &objects.sewer_opening {
            trace.cell(opening.drip_shaft.cell);
            trace.vec2(opening.bottle_cap);
            trace.u32(opening.pillbug_cells.len() as u32);
            for &pos in &opening.pillbug_cells {
                trace.vec2(pos);
            }
        }
        trace.0
    }

    // ---- Per-stage cost ----------------------------------------------------
    //
    // Whole-map generation must stay cheap: `World::new` runs it on the frame
    // the run starts, and the tests build hundreds of worlds. The budget is
    // 8 ms in the dev/test profile (`opt-level = 1`).
    //
    //   RATRUN_LEVEL_TIMINGS=100 cargo test level_stage_timings -- --nocapture
    //
    // prints the median cost of every stage over that many seeds per theme and
    // fails when a map's median total exceeds [`GENERATION_BUDGET_US`]. It is a
    // no-op (one env lookup) in a normal `cargo test` run, so it never makes
    // the suite slow or flaky on a loaded machine.

    /// Per-map budget for one generation attempt, in microseconds (8 ms).
    const GENERATION_BUDGET_US: u64 = 8_000;

    fn median(values: &mut [u64]) -> u64 {
        values.sort_unstable();
        values.get(values.len() / 2).copied().unwrap_or(0)
    }

    fn percentile(values: &mut [u64], percent: usize) -> u64 {
        values.sort_unstable();
        if values.is_empty() {
            return 0;
        }
        let last = values.len() - 1;
        values[(last * percent / 100).min(last)]
    }

    #[test]
    fn level_stage_timings() {
        let Ok(per_theme) = std::env::var("RATRUN_LEVEL_TIMINGS") else {
            return;
        };
        let per_theme: u64 = per_theme.parse().unwrap_or(100);
        let mut grand: Vec<u64> = Vec::new();
        for theme in Theme::all() {
            let mut layout = Vec::new();
            let mut furnish = Vec::new();
            let mut mazes = Vec::new();
            let mut holes = Vec::new();
            let mut traps = Vec::new();
            let mut hazards_v = Vec::new();
            let mut masks = Vec::new();
            let mut objects = Vec::new();
            let mut nav = Vec::new();
            let mut totals = Vec::new();
            for seed in 0..per_theme {
                // Best of three: this machine runs a dozen agents at once, and a
                // single sample can be preempted for a hundred milliseconds.
                let (_, mut t) = generate_once_timed(theme, seed, 0);
                for _ in 1..3 {
                    let (_, sample) = generate_once_timed(theme, seed, 0);
                    t = t.min(sample);
                }
                layout.push(t.layout);
                furnish.push(t.furnish);
                mazes.push(t.mazes);
                holes.push(t.holes);
                traps.push(t.traps);
                hazards_v.push(t.hazards);
                masks.push(t.masks);
                objects.push(t.objects);
                nav.push(t.nav);
                totals.push(t.total());
            }
            println!(
                "{theme:?} ({per_theme} seeds, median µs): layout {}, furnish {}, \
                 mazes {}, rat-holes {}, traps {}, floor-hazards {}, masks {}, \
                 objects {}, nav {} | total {} ({:.2} ms, p95 {} µs, max {} µs)",
                median(&mut layout),
                median(&mut furnish),
                median(&mut mazes),
                median(&mut holes),
                median(&mut traps),
                median(&mut hazards_v),
                median(&mut masks),
                median(&mut objects),
                median(&mut nav),
                median(&mut totals),
                median(&mut totals) as f64 / 1000.0,
                percentile(&mut totals, 95),
                totals.last().copied().unwrap_or(0),
            );
            grand.extend(totals);
        }
        let total = median(&mut grand);
        println!(
            "all themes: median whole map {} µs ({:.2} ms), p95 {} µs, max {} µs, \
             budget {} µs",
            total,
            total as f64 / 1000.0,
            percentile(&mut grand, 95),
            grand.last().copied().unwrap_or(0),
            GENERATION_BUDGET_US
        );
        assert!(
            total <= GENERATION_BUDGET_US,
            "generating one map costs {} µs, over the {} µs budget",
            total,
            GENERATION_BUDGET_US
        );
    }

    #[test]
    fn map_trace_pin() {
        let mut actual = [[0u64; TRACE_SEEDS as usize]; 5];
        let legacy = [Theme::Sewer, Theme::Kitchen, Theme::Lab, Theme::Laundry, Theme::Printworks];
        for (t, theme) in legacy.into_iter().enumerate() {
            for seed in 0..TRACE_SEEDS {
                actual[t][seed as usize] = map_hash(&generate_once(theme, seed, 0));
            }
        }
        if std::env::var("RATRUN_LEVEL_TRACE").is_ok() {
            println!("TRACE_HASHES = {actual:#?}");
            return;
        }
        assert_eq!(
            actual, TRACE_HASHES,
            "the generator's output changed: every hole, trap, door and wall \
             cell must stay put (regenerate the pin only for an intended \
             layout/maze/placement change)"
        );
    }

    #[test]
    fn outfall_seed_sweep_keeps_dry_routes_lures_sconces_and_wet_banks() {
        use crate::game::hazards::OUTFALL_LURE_COUNT;
        use crate::game::level::LightFixtureKind;

        let mut valid_by_attempt = [0usize; 2];
        for seed in 0..20 {
            let accepted = generate(Theme::Outfall, seed);
            validate::validate(&accepted).unwrap();
            for (attempt, count) in valid_by_attempt.iter_mut().enumerate() {
                let level = generate_once(Theme::Outfall, seed, attempt as u32);
                if validate::validate(&level).is_ok() {
                    *count += 1;
                }
            }
            let objects = &accepted.objects;
            assert_eq!(objects.lure_traps.len(), OUTFALL_LURE_COUNT, "seed {seed}");
            assert!((4..=8).contains(&objects.tidal_pits.len()), "seed {seed}");
            assert!((240..=265).contains(&accepted.puddles.len()), "seed {seed}");
            assert!((8..=lights::OUTFALL_SCONCE_CAP).contains(&objects.lights.len()), "seed {seed}");
            let entry = accepted.start_room().unwrap().rect;
            assert_eq!(objects.lights.iter().filter(|f| entry.contains_world(f.pos)
                && f.is_live_sconce()).count(), 4, "seed {seed}");
            assert!(objects.lights.iter().all(|f| f.kind == LightFixtureKind::Sconce));

            let (mut walkable, mut wet) = (0, 0);
            for y in (0..accepted.tiles.rows()).step_by(2) {
                for x in (0..accepted.tiles.cols()).step_by(2) {
                    let at = TileMap::tile_rect(x, y).center();
                    if accepted.tiles.get(x, y) == TileKind::Open && accepted.pit_at(at).is_none() {
                        walkable += 1;
                        wet += accepted.wet_at(at) as usize;
                    }
                }
            }
            let coverage = wet as f32 / walkable as f32;
            println!("Outfall seed {seed}: wet {coverage:.3}, pits {}, fixtures {}",
                objects.tidal_pits.len(), objects.lights.len());
            assert!((0.30..=0.42).contains(&coverage), "seed {seed}: wet {coverage:.3}");
        }
        assert!(valid_by_attempt.into_iter().sum::<usize>() >= 20);
        assert!(valid_by_attempt.into_iter().all(|count| count > 0));
    }

    #[test]
    fn outfall_exhausted_attempt_repair_rescans_lure_banks() {
        use crate::game::hazards::OUTFALL_LURE_COUNT;

        let attempt = MAX_ATTEMPTS - 1;
        let mut level = (0..20).find_map(|seed| {
            let level = generate_once(Theme::Outfall, seed, attempt);
            validate::validate(&level).is_ok().then_some(level)
        }).expect("one of the final-attempt layouts should validate");
        level.objects.lure_traps.clear();
        assert!(validate::validate(&level).is_err());
        assert!(repair_outfall_lures(&mut level, attempt));
        assert_eq!(level.objects.lure_traps.len(), OUTFALL_LURE_COUNT);
        let lures = level.objects.lure_traps.clone();
        assert!(repair_outfall_lures(&mut level, attempt));
        assert_eq!(level.objects.lure_traps, lures);
    }

    // ---- The persisted placement records -----------------------------------
    //
    // `LevelObjects` carries, next to each trap and rat-hole, the generator's
    // record of what it is for (`TrapRecord`, `RatHoleRecord`). Later systems
    // read them through the accessors, so these tests check both the plumbing
    // (one record per object, same index) and the meaning (role, public side,
    // encounter-ness) on 100 maps per run (20 seeds per campaign theme).

    /// Small-class cell distances from `from` (the generator's own flood is
    /// private to `placement`, so the test keeps its own).
    fn cell_distances(tiles: &TileMap, from: (i32, i32)) -> Vec<u32> {
        let mut dist = vec![u32::MAX; (tiles.cols() * tiles.rows()) as usize];
        let mut queue = std::collections::VecDeque::new();
        if !tiles.passable(from.0, from.1, SizeClass::Small) {
            return dist;
        }
        let index = |cx: i32, cy: i32| (cy * tiles.cols() + cx) as usize;
        dist[index(from.0, from.1)] = 0;
        queue.push_back(from);
        while let Some((cx, cy)) = queue.pop_front() {
            let d = dist[index(cx, cy)] + 1;
            let gate = tiles.one_way_dir(cx, cy);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if !tiles.passable(nx, ny, SizeClass::Small) {
                    continue;
                }
                if let Some(allowed) = gate {
                    let dir = match (dx, dy) {
                        (1, 0) => Cardinal::East,
                        (-1, 0) => Cardinal::West,
                        (0, 1) => Cardinal::South,
                        _ => Cardinal::North,
                    };
                    if allowed != dir {
                        continue;
                    }
                }
                let i = index(nx, ny);
                if dist[i] == u32::MAX {
                    dist[i] = d;
                    queue.push_back((nx, ny));
                }
            }
        }
        dist
    }

    /// The two open mouth cells of the tunnel that carries `hole`, in
    /// `(north/west side, south/east side)` order.
    fn hole_mouths(tiles: &TileMap, cell: (i32, i32), along_y: bool) -> ((i32, i32), (i32, i32)) {
        let step = if along_y { (0, 1) } else { (1, 0) };
        let mut last = cell;
        for _ in 0..3 {
            let next = (last.0 + step.0, last.1 + step.1);
            if !matches!(
                tiles.get(next.0, next.1),
                TileKind::Wall(WallKind::RatHole { .. })
            ) {
                break;
            }
            last = next;
        }
        (
            (cell.0 - step.0, cell.1 - step.1),
            (last.0 + step.0, last.1 + step.1),
        )
    }

    #[test]
    fn every_hole_carries_its_role_and_public_side() {
        for theme in Theme::all() {
            for seed in 0..TRACE_SEEDS {
                let level = generate_once(theme, seed, 0);
                let objects = &level.objects;
                assert_eq!(
                    objects.rat_hole_records.len(),
                    objects.rat_holes.len(),
                    "{theme:?} seed {seed}: one record per rat-hole"
                );
                let dist = cell_distances(&level.tiles, validate::cell_of(level.start));
                let (mut excursion, mut gathering) = (0, 0);
                for (hole, record) in objects.rat_holes.iter().zip(&objects.rat_hole_records) {
                    assert_eq!(
                        objects.rat_hole_record(hole.cell),
                        Some(*record),
                        "{theme:?} seed {seed}: the record is reachable by cell"
                    );
                    // The art faces along the passage, toward the public side.
                    assert_eq!(
                        cardinal_vec(record.approach),
                        hole.view.facing,
                        "{theme:?} seed {seed}: hole {:?} faces away from its record",
                        hole.cell
                    );
                    // The public side really is the mouth nearer the start.
                    let along_y = hole.view.facing.y.abs() >= hole.view.facing.x.abs();
                    let (north_west, south_east) = hole_mouths(&level.tiles, hole.cell, along_y);
                    let approach = cardinal_vec(record.approach);
                    let public_is_north_west = if along_y {
                        approach.y < 0.0
                    } else {
                        approach.x < 0.0
                    };
                    let (public, other) = if public_is_north_west {
                        (north_west, south_east)
                    } else {
                        (south_east, north_west)
                    };
                    let at = |cell: (i32, i32)| (cell.1 * level.tiles.cols() + cell.0) as usize;
                    assert!(
                        dist[at(public)] <= dist[at(other)],
                        "{theme:?} seed {seed}: hole {:?} calls {public:?} the public side \
                         but {:?} is nearer the start",
                        hole.cell,
                        other
                    );
                    match record.role {
                        RatHoleRole::Excursion => excursion += 1,
                        RatHoleRole::Gathering => gathering += 1,
                        RatHoleRole::Incidental => {}
                    }
                }
                // One anchor per role: at most one hole is the first-excursion
                // hole and at most one is the mid-run gathering hole.
                assert!(excursion <= 1, "{theme:?} seed {seed}: {excursion} excursion holes");
                assert!(gathering <= 1, "{theme:?} seed {seed}: {gathering} gathering holes");
                assert_eq!(
                    objects.rat_holes_with_role(RatHoleRole::Excursion).count(),
                    excursion
                );
                assert_eq!(
                    objects.rat_holes_with_role(RatHoleRole::Incidental).count(),
                    objects.rat_holes.len() - excursion - gathering
                );
            }
        }
    }

    #[test]
    fn every_trap_carries_its_encounter_flag_and_approach() {
        let mut encounters = 0;
        for theme in Theme::all() {
            for seed in 0..TRACE_SEEDS {
                let level = generate_once(theme, seed, 0);
                let objects = &level.objects;
                assert_eq!(
                    objects.sticky_trap_records.len(),
                    objects.sticky_traps.len(),
                    "{theme:?} seed {seed}: one record per sticky trap"
                );
                assert_eq!(
                    objects.mousetrap_records.len(),
                    objects.mousetraps.len(),
                    "{theme:?} seed {seed}: one record per mousetrap"
                );
                let traps = objects
                    .sticky_traps
                    .iter()
                    .map(|t| (t.pos, t.facing, objects.sticky_trap_record(t.pos)))
                    .chain(
                        objects
                            .mousetraps
                            .iter()
                            .map(|t| (t.pos, t.facing, objects.mousetrap_record(t.pos))),
                    );
                for (pos, facing, record) in traps {
                    let record = record.unwrap_or_else(|| {
                        panic!("{theme:?} seed {seed}: trap at {pos:?} has no record")
                    });
                    assert_eq!(objects.trap_record(pos), Some(record));
                    assert_eq!(objects.is_encounter_trap(pos), record.encounter);
                    // Both trap kinds point their long axis along the approach.
                    assert_eq!(
                        cardinal_vec(record.approach),
                        facing,
                        "{theme:?} seed {seed}: trap at {pos:?} faces away from its record"
                    );
                    if !record.encounter {
                        continue;
                    }
                    encounters += 1;
                    // An encounter is one or two cells inside a hall or room,
                    // facing back at the mouth, on a clear 3 x 3 of floor so
                    // there are two ways past it.
                    let cell = validate::cell_of(pos);
                    let step = cardinal_vec(record.approach);
                    let back = (cell.0 + step.x as i32, cell.1 + step.y as i32);
                    // Rooms merge on an open map, so a cell can be inside two
                    // of them: the encounter is fair if *any* hall or room it
                    // belongs to still holds the mouth it faces.
                    let anchored = level.rooms.iter().any(|room| {
                        matches!(room.kind, RoomKind::Hall | RoomKind::Room)
                            && room.rect.contains(cell.0, cell.1)
                            && room.rect.contains(back.0, back.1)
                    });
                    assert!(
                        anchored,
                        "{theme:?} seed {seed}: encounter at {cell:?} does not face its mouth"
                    );
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (cx, cy) = (cell.0 + dx, cell.1 + dy);
                            assert!(
                                level.tiles.get(cx, cy).is_open()
                                    && level.tiles.passable(cx, cy, SizeClass::Large),
                                "{theme:?} seed {seed}: encounter at {cell:?} has no clear \
                                 floor at ({cx}, {cy})"
                            );
                        }
                    }
                }
            }
        }
        assert!(encounters > 0, "no map grew a single encounter trap");
    }

    #[test]
    fn the_records_answer_by_position_and_cell() {
        let level = generate_once(Theme::Sewer, 0, 0);
        let objects = &level.objects;
        let hole = objects.rat_holes[0];
        assert!(objects.rat_hole_record(hole.cell).is_some());
        assert_eq!(objects.rat_hole_record((0, 0)), None, "no hole in the corner");
        assert_eq!(
            objects.trap_record(Vec2::new(-1000.0, -1000.0)),
            None,
            "no trap off the map"
        );
        assert!(!objects.is_encounter_trap(Vec2::new(-1000.0, -1000.0)));

        // A blank level carries no records at all (the facade's spare).
        let empty = Level::empty(Theme::Sewer);
        assert_eq!(empty.objects.rat_hole_record((5, 5)), None);
        assert_eq!(empty.objects.trap_record(Vec2::new(0.0, 0.0)), None);
        assert_eq!(
            empty
                .objects
                .rat_holes_with_role(RatHoleRole::Incidental)
                .count(),
            0
        );
    }
}
