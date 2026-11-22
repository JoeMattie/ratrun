//! Profiling stage accumulators for `--prof-bench` (see `profbench.rs`).
//!
//! Off by default: a scope is one relaxed atomic load when disabled. Stages
//! are *inclusive*; `profbench` subtracts children where it reports a parent.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

macro_rules! stages {
    ($($name:ident),* $(,)?) => {
        #[allow(dead_code)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum Stage { $($name),* }
        pub const STAGE_NAMES: &[&str] = &[$(stringify!($name)),*];
    };
}

stages! {
    // simulation
    SimTick,
    // World::draw
    Draw,
    SceneFloor,
    SceneWallsProps,
    SceneFixturesAmbience,
    SceneGroundLoot,
    SceneEnemies,
    SceneActors,
    SceneOverhang,
    SceneSmoke,
    SceneParticles,
    Lighting,
    Hostile,
    Resample,
    DamageNumbers,
    Ink,
    // inside the lighting pass
    LSync,
    LPrepare,
    LGather,
    LStaticLights,
    LStaticBake,
    LStaticSplat,
    LSun,
    LRank,
    LMovingBake,
    LMovingSplat,
    LVignette,
    LComposite,
    LMarks,
    // App level
    AppUpdate,
    AppRender,
    Blit,
    Overlays,
    Hud,
    Minimap,
    Sanitize,
    Survey,
    // counters only
    CStaticLit,
    CStaticBakeMiss,
    CMovingBakeMiss,
    CShaped,
    CShapedLit,
    CPoints,
    CMarks,
    CSamples,
    CSplatSamples,
    CSunSamples,
}

const N: usize = 64;
static ON: AtomicBool = AtomicBool::new(false);
/// `--prof-bench` tier sweeps pin the tier: the cost controller observes but
/// never moves.
static FREEZE: AtomicBool = AtomicBool::new(false);
static SKIP_LIGHT: AtomicBool = AtomicBool::new(false);
static NS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static CNT: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

pub fn enable(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn freeze_controller(on: bool) {
    FREEZE.store(on, Ordering::Relaxed);
}

#[inline]
pub fn controller_frozen() -> bool {
    FREEZE.load(Ordering::Relaxed)
}

pub fn skip_lighting(on: bool) {
    SKIP_LIGHT.store(on, Ordering::Relaxed);
}

#[inline]
pub fn lighting_skipped() -> bool {
    SKIP_LIGHT.load(Ordering::Relaxed)
}

pub fn reset() {
    for i in 0..N {
        NS[i].store(0, Ordering::Relaxed);
        CNT[i].store(0, Ordering::Relaxed);
    }
}

pub fn ns(stage: Stage) -> u64 {
    NS[stage as usize].load(Ordering::Relaxed)
}

pub fn cnt(stage: Stage) -> u64 {
    CNT[stage as usize].load(Ordering::Relaxed)
}

#[inline]
pub fn count(stage: Stage, n: u64) {
    if ON.load(Ordering::Relaxed) {
        CNT[stage as usize].fetch_add(n, Ordering::Relaxed);
    }
}

pub struct Scope(Stage, Option<Instant>);

#[inline]
pub fn scope(stage: Stage) -> Scope {
    Scope(
        stage,
        if ON.load(Ordering::Relaxed) {
            Some(Instant::now())
        } else {
            None
        },
    )
}

impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        if let Some(t) = self.1 {
            NS[self.0 as usize].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            CNT[self.0 as usize].fetch_add(1, Ordering::Relaxed);
        }
    }
}
