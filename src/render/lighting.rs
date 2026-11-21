//! Per-frame 2D lighting pass over the world-resolution [`PixelBuffer`].
//!
//! # Architecture
//!
//! Lighting is **100% render-only**: it reads the world's entities, props,
//! particles, boss state and tile grid during `World::draw`, builds a low-res
//! light grid (`LIGHT_CELL = 4` pixels per cell), splats a capped nearest-first
//! list of point/directional lights with optional cheap tile-grid wall
//! occlusion, and bilinearly upsamples the result onto the framebuffer using
//! pure integer fixed-point math (`Q8.8`, `256 = 1.0×`).
//!
//! No simulation state or RNG is ever touched, and all scratch buffers live
//! inside [`LightMap`] so steady-state frames allocate nothing.
//!
//! # Readability Guarantees
//!
//! 1. Every theme ambient (even after boss-fight shift and low-HP vignette) is
//!    strictly clamped to [`MIN_AMBIENT_Q8`] (`Full`) or [`MIN_AMBIENT_SOFT_Q8`]
//!    (`Soft`), so silhouettes never vanish into black.
//! 2. Hostile bullets, boss projectiles, and active boss telegraphs both emit
//!    high-priority local lights AND benefit from the per-pixel self-lit floor
//!    for high-luminance pixels (`peak > SELF_LIT_THRESHOLD`), ensuring danger
//!    remains unmistakable even at the 600-enemy cap.

use bevy_ecs::world::World as EcsWorld;

use crate::game::art_boss_broom::BroomBossState;
use crate::game::art_boss_cat::BossCatState;
use crate::game::art_boss_mower::MowerState;
use crate::game::art_traps::MazeRewardState;
use crate::game::art_walls::WallKind;
use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{TileKind, TILE_F};
use crate::game::ecs::components::Pool;
use crate::game::ecs::systems::boss::{Boss, BossProjectile, BossProjectileKind, BossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Pickup};
use crate::game::level::{Level, PropKind, Theme};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;

/// Light map grid stride in world/framebuffer pixels (1 light cell per 4×4 px).
pub const LIGHT_CELL: usize = 4;

/// Maximum active lights accumulated per frame in `Full` mode.
pub const MAX_LIGHTS_FULL: usize = 40;
/// Maximum active lights accumulated per frame in `Soft` mode.
pub const MAX_LIGHTS_SOFT: usize = 18;

/// Hard minimum multiplicative ambient per channel in `Full` mode (`Q8.8`: 104 / 256 ≈ 0.41×).
/// Guarantees enemies, walls and floor detail remain readable even in deep shadow.
pub const MIN_AMBIENT_Q8: u16 = 104;
/// Hard minimum multiplicative ambient per channel in `Soft` mode (`Q8.8`: 168 / 256 ≈ 0.66×).
pub const MIN_AMBIENT_SOFT_Q8: u16 = 168;
/// Upper clamp on multiplicative light (`Q8.8`: 308 / 256 ≈ 1.20×) so bright
/// sprites (rat coat, kitchen counter, lab console) never bleach to flat white.
pub const MAX_MUL_Q8: u16 = 308;
/// Upper clamp on additive colored scatter per channel (applied with soft
/// headroom compression so dark floors catch rich colored pools while bright
/// pixels keep crisp detail).
pub const MAX_ADD_U16: u16 = 112;

/// Pixels with `max(r, g, b)` above this threshold smoothly resist darkening so
/// hostile bullets, warning tells, sparks and bright eyes stay self-lit.
pub const SELF_LIT_THRESHOLD: u8 = 176;

/// Environment variable that overrides the persisted/auto lighting mode.
pub const LIGHTING_ENV: &str = "RATRUN_LIGHTING";

// ---------------------------------------------------------------------------
// LightingMode preference & overrides
// ---------------------------------------------------------------------------

/// Quality / intensity setting for the lighting pass (and paired smoke layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LightingMode {
    /// Full dramatic per-theme ambient, wall occlusion, vignette & rich smoke.
    /// Default on 24-bit truecolor terminals.
    #[default]
    Full,
    /// Gentler contrast, fewer lights, no hard shadow occlusion: avoids color
    /// banding on 256-colour terminals.
    Soft,
    /// Lighting pass disabled (instant no-op). Default on 16-colour terminals
    /// and the `--ai` spectate wall.
    Off,
}

impl LightingMode {
    /// Recommended default for a terminal's detected colour depth.
    #[inline]
    pub const fn default_for_depth(depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::Truecolor => Self::Full,
            ColorDepth::Ansi256 => Self::Soft,
            ColorDepth::Ansi16 => Self::Off,
        }
    }

    /// Canonical lowercase string persisted in `scores.json`.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Soft => "soft",
            Self::Off => "off",
        }
    }

    /// Parse a persisted setting or environment value (case-insensitive).
    pub fn parse_setting(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "full" | "on" | "high" | "1" | "true" => Some(Self::Full),
            "soft" | "low" | "med" | "medium" | "subtle" => Some(Self::Soft),
            "off" | "none" | "0" | "false" | "disabled" => Some(Self::Off),
            _ => None,
        }
    }

    /// Cycle through `Full → Soft → Off → Full` (`dir > 0`) or reverse (`dir < 0`).
    #[inline]
    pub const fn step(self, dir: i32) -> Self {
        if dir > 0 {
            match self {
                Self::Full => Self::Soft,
                Self::Soft => Self::Off,
                Self::Off => Self::Full,
            }
        } else if dir < 0 {
            match self {
                Self::Full => Self::Off,
                Self::Soft => Self::Full,
                Self::Off => Self::Soft,
            }
        } else {
            self
        }
    }

    /// Active light cap for this mode.
    #[inline]
    pub const fn light_cap(self) -> usize {
        match self {
            Self::Full => MAX_LIGHTS_FULL,
            Self::Soft => MAX_LIGHTS_SOFT,
            Self::Off => 0,
        }
    }

    /// Minimum ambient luminance floor (`Q8.8`) for this mode.
    #[inline]
    pub const fn min_ambient_q8(self) -> u16 {
        match self {
            Self::Full => MIN_AMBIENT_Q8,
            Self::Soft => MIN_AMBIENT_SOFT_Q8,
            Self::Off => 256,
        }
    }
}

/// Parse a `RATRUN_LIGHTING` environment override string.
pub fn parse_env_override(raw: &str) -> Option<LightingMode> {
    LightingMode::parse_setting(raw)
}

/// Read `RATRUN_LIGHTING` from the process environment once at app startup.
pub fn env_override() -> Option<LightingMode> {
    std::env::var(LIGHTING_ENV)
        .ok()
        .as_deref()
        .and_then(parse_env_override)
}

// ---------------------------------------------------------------------------
// Theme ambient profiles
// ---------------------------------------------------------------------------

/// Per-theme lighting character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeAmbient {
    /// Base multiplicative ambient light in `Q8.8` (`256 = 1.0×`).
    pub mul: [u16; 3],
    /// Whether fixed level lights use crisp/hard pool falloff (Lab).
    pub hard_pools: bool,
    /// Soft personal aura tint around the player rat.
    pub rat_tint: Rgb,
}

/// Ambient tint & lighting character for `theme`. Unknown/future themes receive
/// a balanced cool-neutral default so new themes compile and read cleanly.
#[inline]
pub fn theme_ambient(theme: Theme) -> ThemeAmbient {
    #[allow(unreachable_patterns)]
    match theme {
        // Dim, damp green-blue subterranean shadow; warm lantern rat contrasts nicely.
        Theme::Sewer => ThemeAmbient {
            mul: [110, 146, 152],
            hard_pools: false,
            rat_tint: (255, 240, 206),
        },
        // Warm amber kitchen lighting with cozy shaded corners.
        Theme::Kitchen => ThemeAmbient {
            mul: [172, 146, 118],
            hard_pools: false,
            rat_tint: (255, 244, 222),
        },
        // Cold clinical shadows with sharp, bright pools of light.
        Theme::Lab => ThemeAmbient {
            mul: [128, 152, 194],
            hard_pools: true,
            rat_tint: (224, 244, 255),
        },
        // Open daylight on the road: bright and slightly warm, so the flatbed
        // reads as outdoors and the light pass only shades it gently.
        Theme::Trailer => ThemeAmbient {
            mul: [236, 226, 204],
            hard_pools: false,
            rat_tint: (255, 248, 230),
        },
        // Sensible default for future themes (e.g. Laundry, Printworks).
        _ => ThemeAmbient {
            mul: [148, 152, 162],
            hard_pools: false,
            rat_tint: (248, 244, 232),
        },
    }
}

/// Compute the final ambient multiplier (`Q8.8`) for a theme, mode, and optional
/// active boss phase, strictly enforcing the mode's readability floor.
pub fn effective_ambient(theme: Theme, mode: LightingMode, boss_phase: Option<u8>) -> [u16; 3] {
    if mode == LightingMode::Off {
        return [256, 256, 256];
    }
    let base = theme_ambient(theme).mul;
    let floor = mode.min_ambient_q8();

    // Soft mode pulls ambient 60% toward neutral 256 to prevent 256-colour banding
    // and clearly differentiate Soft from dramatic Full lighting.
    let mut amb = if mode == LightingMode::Soft {
        [
            ((base[0] as u32 * 2 + 256 * 3) / 5) as u16,
            ((base[1] as u32 * 2 + 256 * 3) / 5) as u16,
            ((base[2] as u32 * 2 + 256 * 3) / 5) as u16,
        ]
    } else {
        base
    };

    // Subtle boss-fight ambient tension shift (stronger in phase 2+).
    if let Some(phase) = boss_phase {
        let shift: (i32, i32, i32) = if mode == LightingMode::Soft {
            if phase >= 2 {
                (6, -8, -6)
            } else {
                (3, -5, -4)
            }
        } else if phase >= 2 {
            (12, -16, -12)
        } else {
            (6, -10, -8)
        };
        amb[0] = (amb[0] as i32 + shift.0).clamp(floor as i32, 256) as u16;
        amb[1] = (amb[1] as i32 + shift.1).clamp(floor as i32, 256) as u16;
        amb[2] = (amb[2] as i32 + shift.2).clamp(floor as i32, 256) as u16;
    }

    [amb[0].max(floor), amb[1].max(floor), amb[2].max(floor)]
}

// ---------------------------------------------------------------------------
// Point lights & reusable LightMap scratch
// ---------------------------------------------------------------------------

/// One active light source in screen-pixel coordinates.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub color: Rgb,
    /// Multiplicative light gain at center in `Q8.8` (`256 = +1.0×`).
    pub intensity: u16,
    /// Additive colored scatter at center (`0..=255` added onto dark pixels).
    pub scatter: u16,
    /// Capping priority (`3` = hero/boss tell/nuke, `2` = combat/boss/enemy shot,
    /// `1` = props/pools/pickups/player shots, `0` = gem glints).
    pub priority: u8,
    /// Casts cheap tile-grid wall shadows in `Full` mode.
    pub occlude: bool,
    /// Sharper plateau-and-drop falloff (Lab hard pools).
    pub hard: bool,
}

/// Read-only view of the world state needed to gather lights for one frame.
pub struct LightScene<'a> {
    pub ecs: &'a EcsWorld,
    pub level: &'a Level,
    pub player: &'a Player,
    pub particles: &'a Particles,
    pub thrash: Option<(f32, f32, f32)>,
    pub squeak: Option<(Vec2, f32)>,
    pub cam: Vec2,
    pub elapsed: f32,
    pub mode: LightingMode,
}

/// Reusable per-world lighting scratch buffers (zero allocation after warm-up).
pub struct LightMap {
    lw: usize,
    lh: usize,
    mul: Vec<[u16; 3]>,
    add: Vec<[u16; 3]>,
    lights: Vec<PointLight>,
    occluders: Vec<u8>,
    occ_x0: i32,
    occ_y0: i32,
    occ_w: usize,
    occ_h: usize,
    bat_eyes: Vec<(i32, i32, Rgb)>,
}

impl Default for LightMap {
    fn default() -> Self {
        Self::new()
    }
}

impl LightMap {
    pub fn new() -> Self {
        Self {
            lw: 0,
            lh: 0,
            mul: Vec::new(),
            add: Vec::new(),
            lights: Vec::with_capacity(128),
            occluders: Vec::new(),
            occ_x0: 0,
            occ_y0: 0,
            occ_w: 0,
            occ_h: 0,
            bat_eyes: Vec::with_capacity(64),
        }
    }

    /// Number of lights kept in the most recent pass (for tests & diagnostics).
    #[inline]
    pub fn active_light_count(&self) -> usize {
        self.lights.len()
    }

    /// Light map grid dimensions `(lw, lh)`.
    #[inline]
    pub fn grid_size(&self) -> (usize, usize) {
        (self.lw, self.lh)
    }

    /// Sample the computed multiplicative light (`Q8.8`) at light-grid cell `(gx, gy)`.
    #[inline]
    pub fn mul_at(&self, gx: usize, gy: usize) -> [u16; 3] {
        if gx < self.lw && gy < self.lh {
            self.mul[gy * self.lw + gx]
        } else {
            [256, 256, 256]
        }
    }

    /// Sample the computed additive scatter at light-grid cell `(gx, gy)`.
    #[inline]
    pub fn add_at(&self, gx: usize, gy: usize) -> [u16; 3] {
        if gx < self.lw && gy < self.lh {
            self.add[gy * self.lw + gx]
        } else {
            [0, 0, 0]
        }
    }

    /// Resize the low-resolution light grid only when needed (no steady-state alloc).
    fn ensure_grid(&mut self, pb_w: usize, pb_h: usize) {
        let lw = pb_w.div_ceil(LIGHT_CELL) + 1;
        let lh = pb_h.div_ceil(LIGHT_CELL) + 1;
        let len = lw * lh;
        self.lw = lw;
        self.lh = lh;
        if self.mul.len() != len {
            self.mul.resize(len, [256, 256, 256]);
            self.add.resize(len, [0, 0, 0]);
        }
    }

    /// Run the complete lighting pass over `pb` for the given `scene`.
    pub fn apply_scene(&mut self, pb: &mut PixelBuffer, scene: &LightScene<'_>) {
        if scene.mode == LightingMode::Off || pb.w == 0 || pb.h == 0 {
            self.lights.clear();
            self.bat_eyes.clear();
            return;
        }

        self.ensure_grid(pb.w, pb.h);
        self.lights.clear();
        self.bat_eyes.clear();

        let boss_phase = self.gather_lights(pb.w, pb.h, scene);

        // Enforce hard light cap, keeping highest-priority & nearest-to-player lights.
        let px = scene.player.pos.x - scene.cam.x;
        let py = scene.player.pos.y - scene.cam.y;
        self.cap_lights(scene.mode.light_cap(), px, py);

        // Fill light grid with theme ambient (plus boss-fight shift).
        let ambient = effective_ambient(scene.level.theme, scene.mode, boss_phase);
        self.mul.fill(ambient);
        self.add.fill([0, 0, 0]);

        // Populate compact local tile opacity grid for wall occlusion (Full mode only).
        let use_occlusion = scene.mode == LightingMode::Full;
        if use_occlusion {
            self.build_occluders(scene.level, scene.cam, pb.w, pb.h);
        }

        // Splat all surviving lights onto the low-res grid.
        let (int_scale, scatter_scale): (u32, u32) = if scene.mode == LightingMode::Soft {
            (156, 112)
        } else {
            (256, 256)
        };
        for i in 0..self.lights.len() {
            let light = self.lights[i];
            self.splat_light(light, scene.cam, use_occlusion, int_scale, scatter_scale);
        }

        // Apply low-HP crimson heartbeat vignette directly on the low-res grid.
        let max_hp = scene.player.max_hp().max(1.0);
        let hp_frac = (scene.player.hp / max_hp).clamp(0.0, 1.0);
        if hp_frac < 0.35 {
            self.apply_low_hp_vignette(hp_frac, scene.elapsed, scene.mode);
        }

        // Bilinearly upsample and apply to framebuffer pixels.
        self.composite_onto(pb);

        // Paint eerie glowing eyes for bats lurking in dark corners.
        self.draw_dark_bat_eyes(pb);
    }

    /// Direct light-list entry point (useful for unit tests and benchmarks).
    pub fn apply_custom(
        &mut self,
        pb: &mut PixelBuffer,
        ambient: [u16; 3],
        mode: LightingMode,
        player_screen: (f32, f32),
        lights: &[PointLight],
    ) {
        if mode == LightingMode::Off || pb.w == 0 || pb.h == 0 {
            self.lights.clear();
            return;
        }
        self.ensure_grid(pb.w, pb.h);
        self.lights.clear();
        self.lights.extend_from_slice(lights);
        self.cap_lights(mode.light_cap(), player_screen.0, player_screen.1);

        let floor = mode.min_ambient_q8();
        let amb = [
            ambient[0].max(floor),
            ambient[1].max(floor),
            ambient[2].max(floor),
        ];
        self.mul.fill(amb);
        self.add.fill([0, 0, 0]);

        let (int_scale, scatter_scale): (u32, u32) = if mode == LightingMode::Soft {
            (156, 112)
        } else {
            (256, 256)
        };
        for i in 0..self.lights.len() {
            let light = self.lights[i];
            self.splat_light(light, Vec2::ZERO, false, int_scale, scatter_scale);
        }
        self.composite_onto(pb);
    }

    /// Cap `self.lights` to `cap`, keeping highest-priority and nearest lights.
    pub fn cap_lights(&mut self, cap: usize, ref_x: f32, ref_y: f32) {
        if self.lights.len() <= cap {
            return;
        }
        if cap == 0 {
            self.lights.clear();
            return;
        }
        self.lights.select_nth_unstable_by(cap, |a, b| {
            // Higher priority first; within the same priority, closer to player first.
            b.priority.cmp(&a.priority).then_with(|| {
                let da = (a.x - ref_x) * (a.x - ref_x) + (a.y - ref_y) * (a.y - ref_y);
                let db = (b.x - ref_x) * (b.x - ref_x) + (b.y - ref_y) * (b.y - ref_y);
                da.total_cmp(&db)
            })
        });
        self.lights.truncate(cap);
    }

    /// Gather all lights from world state; returns the active boss phase if any.
    fn gather_lights(&mut self, pb_w: usize, pb_h: usize, scene: &LightScene<'_>) -> Option<u8> {
        let vw = pb_w as f32;
        let vh = pb_h as f32;
        let cam = scene.cam;
        let elapsed = scene.elapsed;
        let theme_prof = theme_ambient(scene.level.theme);

        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };

        // 1. Player rat soft personal aura (always present, priority 3).
        let px = scene.player.pos.x - cam.x;
        let py = scene.player.pos.y - cam.y;
        let rat_radius = if scene.level.theme == Theme::Lab {
            58.0
        } else {
            66.0
        };
        let rat_flicker = 1.0 + 0.03 * (elapsed * 4.3).sin();
        self.lights.push(PointLight {
            x: px,
            y: py,
            radius: rat_radius * rat_flicker,
            color: theme_prof.rat_tint,
            intensity: 162,
            scatter: 22,
            priority: 3,
            occlude: true,
            hard: false,
        });

        // Dash cyan-white burst light.
        if scene.player.dash_active > 0.0 {
            let k = (scene.player.dash_active / 0.16).clamp(0.2, 1.0);
            self.lights.push(PointLight {
                x: px,
                y: py,
                radius: 36.0,
                color: (165, 224, 255),
                intensity: (170.0 * k) as u16,
                scatter: (42.0 * k) as u16,
                priority: 3,
                occlude: false,
                hard: false,
            });
        }

        // Player hurt flash.
        if scene.player.hurt_flash > 0.0 {
            let k = (scene.player.hurt_flash / 0.25).clamp(0.0, 1.0);
            self.lights.push(PointLight {
                x: px,
                y: py,
                radius: 42.0,
                color: (255, 76, 86),
                intensity: (180.0 * k) as u16,
                scatter: (48.0 * k) as u16,
                priority: 3,
                occlude: false,
                hard: false,
            });
        }

        // 2. Active weapons: Tail Thrash arc & Sonic Squeak ring.
        if let Some((angle, reach, fade)) = scene.thrash {
            let dir = Vec2::from_angle(angle);
            let tip = scene.player.pos + dir * (reach * 0.62);
            let sx = tip.x - cam.x;
            let sy = tip.y - cam.y;
            let r = (reach * 0.95 + 16.0).clamp(24.0, 72.0);
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (255, 178, 92),
                    intensity: (230.0 * fade) as u16,
                    scatter: (58.0 * fade) as u16,
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }
        if let Some((origin, ring_r)) = scene.squeak {
            let sx = origin.x - cam.x;
            let sy = origin.y - cam.y;
            let r = (ring_r + 22.0).clamp(24.0, 110.0);
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (140, 240, 255),
                    intensity: 205,
                    scatter: 50,
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }

        // 3. Shockwaves (explosions, nukes, brute/boss deaths, TailWhip, SqueakNova, acid splashes).
        for sw in &scene.particles.shockwaves {
            let age = 1.0 - (sw.life / sw.max_life.max(1e-4)).clamp(0.0, 1.0);
            let fade = (1.0 - age) * (1.0 - age * 0.55);
            if fade <= 0.04 {
                continue;
            }
            let sx = sw.pos.x - cam.x;
            let sy = sw.pos.y - cam.y;
            let r = (sw.end_r * 1.30 + 18.0).clamp(20.0, 145.0);
            if !in_range(sx, sy, r) {
                continue;
            }
            let is_nuke = sw.end_r >= 75.0;
            let is_big = sw.end_r >= 20.0;
            // Keep nuke flash warm golden-orange so it illuminates dramatically without washing out.
            let col = if is_nuke {
                blend_rgb((255, 198, 92), sw.color, age * 0.65)
            } else {
                blend_rgb(sw.core, sw.color, age)
            };
            self.lights.push(PointLight {
                x: sx,
                y: sy,
                radius: r,
                color: col,
                intensity: ((if is_nuke { 235.0 } else if is_big { 205.0 } else { 155.0 }) * fade)
                    as u16,
                scatter: ((if is_nuke { 92.0 } else if is_big { 64.0 } else { 36.0 }) * fade)
                    as u16,
                priority: if is_nuke { 3 } else { 2 },
                occlude: is_big && !is_nuke,
                hard: false,
            });
        }

        // 4. Fixed level lights from props, dock pad, and unopened maze rewards.
        self.gather_fixed_lights(scene, vw, vh, theme_prof.hard_pools);

        // 5. ECS entities: pools, pickups, gems, bullets, enemies (bats/boss), bosses, boss projectiles.
        let mut boss_phase: Option<u8> = None;
        for entity in scene.ecs.iter_entities() {
            // Acid pools (glowing toxic green).
            if let Some(p) = entity.get::<Pool>() {
                let sx = p.pos.x - cam.x;
                let sy = p.pos.y - cam.y;
                let r = p.radius * 2.1 + 12.0;
                if in_range(sx, sy, r) {
                    let k = (p.life / 2.6).clamp(0.25, 1.0);
                    let pulse = 0.90 + 0.10 * (elapsed * 4.5 + p.pos.x * 0.13).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: p.color,
                        intensity: (175.0 * k * pulse) as u16,
                        scatter: (44.0 * k * pulse) as u16,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Pickups (pulsing colored halos).
            if let Some(p) = entity.get::<Pickup>() {
                let sx = p.pos.x - cam.x;
                let sy = p.pos.y - cam.y;
                let pulse = 0.72 + 0.28 * (p.bob.sin() * 0.5 + 0.5);
                let r = 28.0 + pulse * 8.0;
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: p.kind.color(),
                        intensity: (185.0 * pulse) as u16,
                        scatter: (46.0 * pulse) as u16,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Gems (small glints; big gold gems glow slightly brighter).
            if let Some(g) = entity.get::<Gem>() {
                let sx = g.pos.x - cam.x;
                let sy = g.pos.y - cam.y;
                let big = g.value >= 25;
                let r = if big { 18.0 } else { 11.0 };
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: g.color,
                        intensity: if big { 130 } else { 80 },
                        scatter: if big { 26 } else { 14 },
                        priority: 0,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Bullets (hostile shots self-lit & high priority; player shots & spores colored).
            if let Some(b) = entity.get::<Bullet>() {
                let sx = b.pos.x - cam.x;
                let sy = b.pos.y - cam.y;
                if b.faction == Faction::Enemy {
                    let r = 22.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: brighten_rgb(b.color, 40),
                            intensity: 200,
                            scatter: 48,
                            priority: 2,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if b.orbit.is_some() {
                    let r = 22.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: (125, 255, 180),
                            intensity: 155,
                            scatter: 34,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else {
                    // Muzzle flash boost when freshly spawned near the player.
                    let near_muzzle = b.pos.dist_sq(scene.player.pos) < 160.0;
                    let r = if near_muzzle {
                        24.0
                    } else if b.lob.is_some() {
                        20.0
                    } else {
                        15.0
                    };
                    if in_range(sx, sy, r) {
                        let col = b.weapon.map(|w| w.color()).unwrap_or(b.color);
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: col,
                            intensity: if near_muzzle { 170 } else { 115 },
                            scatter: if near_muzzle { 38 } else { 22 },
                            priority: if near_muzzle { 2 } else { 1 },
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }

            // Boss-owned projectiles (high-priority warning glows).
            if let Some(bp) = entity.get::<BossProjectile>() {
                let sx = bp.pos.x - cam.x;
                let sy = bp.pos.y - cam.y;
                let r = 26.0;
                if in_range(sx, sy, r) {
                    #[allow(unreachable_patterns)]
                    let col = match bp.kind {
                        BossProjectileKind::BladeDisc => (255, 145, 70),
                        BossProjectileKind::GrassShrapnel => (170, 255, 110),
                        BossProjectileKind::SonicFang => (255, 115, 185),
                        BossProjectileKind::DustpanShard => (255, 210, 120),
                        _ => scene.level.palette.accent,
                    };
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: col,
                        intensity: 215,
                        scatter: 54,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Enemies: glowing eyes for Bats in the dark, legacy Boss glow, hit-flash glint.
            if let Some(e) = entity.get::<Enemy>() {
                if !e.alive() {
                    continue;
                }
                let sx = e.pos.x - cam.x;
                let sy = e.pos.y - cam.y;
                if e.kind == EnemyKind::Boss {
                    boss_phase = Some(boss_phase.unwrap_or(1));
                    let r = 64.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: (255, 90, 150),
                            intensity: 210,
                            scatter: 52,
                            priority: 2,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if e.kind == EnemyKind::Bat
                    && self.bat_eyes.len() < 60
                    && sx >= 2.0
                    && sy >= 2.0
                    && sx < vw - 2.0
                    && sy < vh - 2.0
                {
                    let face = if e.facing.len_sq() > 0.01 {
                        e.facing.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    let head = e.pos + face * 1.4;
                    let side = face.perp() * 0.85;
                    let eye_col = if e.elite {
                        (255, 225, 95)
                    } else {
                        (255, 110, 145)
                    };
                    let e1 = (
                        (head.x + side.x - cam.x).round() as i32,
                        (head.y + side.y - cam.y).round() as i32,
                    );
                    let e2 = (
                        (head.x - side.x - cam.x).round() as i32,
                        (head.y - side.y - cam.y).round() as i32,
                    );
                    self.bat_eyes.push((e1.0, e1.1, eye_col));
                    if e2 != e1 {
                        self.bat_eyes.push((e2.0, e2.1, eye_col));
                    }
                }
            }

            // Bosses (headlamps, lidar cones, attack tells, phase auras, hazards).
            if let Some(boss) = entity.get::<Boss>() {
                if !boss.dead {
                    boss_phase = Some(boss.phase.max(1));
                }
                self.gather_boss_lights(boss, cam, vw, vh, elapsed);
            }
        }

        boss_phase
    }

    /// Fixed lights derived from existing themed props, Lab charging dock, and
    /// unopened maze reward pedestals.
    fn gather_fixed_lights(
        &mut self,
        scene: &LightScene<'_>,
        vw: f32,
        vh: f32,
        hard_pools: bool,
    ) {
        let cam = scene.cam;
        let elapsed = scene.elapsed;
        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };

        for prop in &scene.level.props {
            let c = prop.rect.center();
            let sx = c.x - cam.x;
            let sy = c.y - cam.y;
            let h = pos_hash(prop.rect.x as i32, prop.rect.y as i32);

            #[allow(unreachable_patterns)]
            let spec: Option<(f32, Rgb, u16, u16, bool)> = match prop.kind {
                // Lab computer screens: crisp cyan pools of light with subtle CRT flicker.
                PropKind::Console => {
                    let flick = 0.94 + 0.06 * (elapsed * 7.0 + (h & 31) as f32).sin();
                    Some((
                        48.0,
                        (96, 220, 255),
                        (142.0 * flick) as u16,
                        (38.0 * flick) as u16,
                        true,
                    ))
                }
                // Lab bio-specimen tanks: bioluminescent emerald-cyan pools.
                PropKind::Tank => {
                    let pulse = 0.90 + 0.10 * (elapsed * 2.4 + (h & 63) as f32).sin();
                    Some((
                        44.0,
                        (84, 240, 192),
                        (136.0 * pulse) as u16,
                        (36.0 * pulse) as u16,
                        true,
                    ))
                }
                // Sewer grates: eerie sub-floor teal vent glow.
                PropKind::Grate => {
                    let pulse = 0.88 + 0.12 * (elapsed * 1.9 + (h & 31) as f32).sin();
                    Some((
                        42.0,
                        (84, 215, 185),
                        (132.0 * pulse) as u16,
                        (30.0 * pulse) as u16,
                        false,
                    ))
                }
                // Sewer valves: warm industrial gauge lamp.
                PropKind::Valve => {
                    let pulse = 0.92 + 0.08 * (elapsed * 3.1 + (h & 31) as f32).sin();
                    Some((
                        34.0,
                        (235, 175, 96),
                        (124.0 * pulse) as u16,
                        (28.0 * pulse) as u16,
                        false,
                    ))
                }
                // Kitchen counters: warm overhead range/hood light (keeps stainless steel crisp).
                PropKind::Counter => Some((52.0, (255, 202, 126), 118, 28, true)),
                // Kitchen tables: warm diner lamp on ~2/3 of tables for pools of light & shade.
                PropKind::Table if !h.is_multiple_of(3) => Some((44.0, (255, 192, 114), 108, 24, true)),
                // Kitchen barrels: occasional warm lantern glint.
                PropKind::Barrel if h & 1 == 0 => Some((30.0, (245, 174, 84), 96, 20, false)),
                // Barriers: small pulsing amber hazard beacon.
                PropKind::Barrier => {
                    let blink = 0.65 + 0.35 * (elapsed * 3.8 + (h & 15) as f32).sin().abs();
                    Some((
                        28.0,
                        (255, 182, 68),
                        (118.0 * blink) as u16,
                        (28.0 * blink) as u16,
                        false,
                    ))
                }
                _ => None,
            };

            if let Some((radius, color, intensity, scatter, occlude)) = spec {
                if in_range(sx, sy, radius) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius,
                        color,
                        intensity,
                        scatter,
                        priority: 1,
                        occlude,
                        hard: hard_pools,
                    });
                }
            }
        }

        // Lab boss charging dock pad glow.
        if let Some(dock) = scene.level.boss.dock {
            let sx = dock.x - cam.x;
            let sy = dock.y - cam.y;
            let r = 46.0;
            if in_range(sx, sy, r) {
                let pulse = 0.82 + 0.18 * (elapsed * 3.2).sin();
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (100, 242, 215),
                    intensity: (195.0 * pulse) as u16,
                    scatter: (44.0 * pulse) as u16,
                    priority: 1,
                    occlude: false,
                    hard: hard_pools,
                });
            }
        }

        // Unopened maze reward pedestals emit a warm golden treasure glint.
        for reward in &scene.level.objects.rewards {
            if reward.state == MazeRewardState::Opened {
                continue;
            }
            let sx = reward.pos.x - cam.x;
            let sy = reward.pos.y - cam.y;
            let r = 36.0;
            if in_range(sx, sy, r) {
                let pulse = 0.85 + 0.15 * (elapsed * 3.6).sin();
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (255, 215, 92),
                    intensity: (190.0 * pulse) as u16,
                    scatter: (42.0 * pulse) as u16,
                    priority: 1,
                    occlude: false,
                    hard: false,
                });
            }
        }
    }

    /// Lights for living/dying bosses: mower headlamp & lidar, cat eyes & pounce
    /// tell, broom slam/sweep tells, and all active `BossHazard` warnings.
    fn gather_boss_lights(&mut self, boss: &Boss, cam: Vec2, vw: f32, vh: f32, elapsed: f32) {
        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };
        let bsx = boss.pos.x - cam.x;
        let bsy = boss.pos.y - cam.y;
        let face = if boss.facing.len_sq() > 0.01 {
            boss.facing.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let progress = if boss.duration > 0.001 {
            (boss.elapsed / boss.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };

        #[allow(unreachable_patterns)]
        match boss.state {
            BossState::Mower(st) => {
                // Mower headlamp / lidar beam projecting ahead of the chassis.
                let (lamp_col, lamp_r, lamp_int, lamp_sc) = match st {
                    MowerState::Patrol => ((155, 232, 255), 54.0, 210u16, 48u16),
                    MowerState::Scan => {
                        if progress >= 0.70 {
                            ((255, 105, 58), 64.0, 255, 68)
                        } else {
                            ((92, 255, 148), 62.0, 225, 56)
                        }
                    }
                    MowerState::ChargeWindup | MowerState::Charge | MowerState::Ricochet => {
                        ((255, 96, 48), 66.0, 265, 72)
                    }
                    MowerState::FlingBlades => ((255, 168, 72), 54.0, 230, 58),
                    MowerState::Overheat | MowerState::Flipped => {
                        let flick = 0.72 + 0.28 * (elapsed * 16.0).sin().abs();
                        (
                            (255, 152, 58),
                            46.0,
                            (215.0 * flick) as u16,
                            (58.0 * flick) as u16,
                        )
                    }
                    MowerState::Docking => ((105, 245, 230), 58.0, 235, 60),
                    MowerState::Dying => ((255, 215, 120), 92.0, 320, 105),
                };

                // Chassis core glow.
                if in_range(bsx, bsy, 46.0) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 44.0,
                        color: lamp_col,
                        intensity: lamp_int / 2 + 70,
                        scatter: lamp_sc / 2 + 12,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }

                // Directional headlamp cone (2 stepped pools ahead of the nose).
                if !matches!(st, MowerState::Flipped | MowerState::Dying) {
                    for &(dist, scale_r, scale_i) in &[(24.0f32, 0.75f32, 1.0f32), (50.0, 1.0, 0.82)]
                    {
                        let p = boss.pos + face * dist;
                        let sx = p.x - cam.x;
                        let sy = p.y - cam.y;
                        let r = lamp_r * scale_r;
                        if in_range(sx, sy, r) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: r,
                                color: lamp_col,
                                intensity: (lamp_int as f32 * scale_i) as u16,
                                scatter: (lamp_sc as f32 * scale_i) as u16,
                                priority: 3,
                                occlude: true,
                                hard: true,
                            });
                        }
                    }
                }

                // Target lock / charge rail beacon.
                if matches!(st, MowerState::Scan | MowerState::ChargeWindup) {
                    let tsx = boss.target.x - cam.x;
                    let tsy = boss.target.y - cam.y;
                    if in_range(tsx, tsy, 40.0) {
                        self.lights.push(PointLight {
                            x: tsx,
                            y: tsy,
                            radius: 38.0,
                            color: (255, 92, 62),
                            intensity: 230,
                            scatter: 60,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
            BossState::Cat(st) => {
                // Predator amber-gold eyes & presence glow.
                let head = boss.pos + face * 10.0;
                let hsx = head.x - cam.x;
                let hsy = head.y - cam.y;
                if in_range(hsx, hsy, 42.0) {
                    let col = if boss.phase >= 2 {
                        (255, 130, 62)
                    } else {
                        (255, 205, 88)
                    };
                    self.lights.push(PointLight {
                        x: hsx,
                        y: hsy,
                        radius: 40.0,
                        color: col,
                        intensity: 195,
                        scatter: 42,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }

                // Pounce landing telegraph or swipe tell.
                match st {
                    BossCatState::Crouch | BossCatState::Pounce => {
                        let tsx = boss.target.x - cam.x;
                        let tsy = boss.target.y - cam.y;
                        let locked = st == BossCatState::Pounce || progress >= 0.75;
                        let r = if locked { 44.0 } else { 36.0 };
                        if in_range(tsx, tsy, r) {
                            self.lights.push(PointLight {
                                x: tsx,
                                y: tsy,
                                radius: r,
                                color: if locked {
                                    (255, 82, 62)
                                } else {
                                    (255, 186, 72)
                                },
                                intensity: if locked { 255 } else { 205 },
                                scatter: if locked { 66 } else { 44 },
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    BossCatState::SwipeWindup | BossCatState::Swipe => {
                        let p = boss.pos + face * 20.0;
                        let sx = p.x - cam.x;
                        let sy = p.y - cam.y;
                        if in_range(sx, sy, 46.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: 44.0,
                                color: (255, 140, 70),
                                intensity: 235,
                                scatter: 58,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    BossCatState::Roar | BossCatState::Death if in_range(bsx, bsy, 80.0) => {
                        self.lights.push(PointLight {
                            x: bsx,
                            y: bsy,
                            radius: 76.0,
                            color: (255, 165, 88),
                            intensity: 280,
                            scatter: 82,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                    _ => {}
                }
            }
            BossState::Broom(st) => {
                // Warm overhead spotlight following the Sweeper.
                if in_range(bsx, bsy, 54.0) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 52.0,
                        color: (255, 214, 150),
                        intensity: 175,
                        scatter: 34,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                match st {
                    BroomBossState::StompWalk { .. }
                    | BroomBossState::DustpanDrop
                    | BroomBossState::BroomJab
                    | BroomBossState::BroomSweep { .. } => {
                        let tsx = boss.target.x - cam.x;
                        let tsy = boss.target.y - cam.y;
                        if in_range(tsx, tsy, 42.0) {
                            self.lights.push(PointLight {
                                x: tsx,
                                y: tsy,
                                radius: 40.0,
                                color: (255, 118, 72),
                                intensity: 230,
                                scatter: 56,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    BroomBossState::PhaseChange | BroomBossState::Defeat
                        if in_range(bsx, bsy, 76.0) =>
                    {
                        self.lights.push(PointLight {
                            x: bsx,
                            y: bsy,
                            radius: 74.0,
                            color: (255, 195, 105),
                            intensity: 270,
                            scatter: 78,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                    _ => {}
                }
            }
            _ => {
                // Fallback aura for future wave-3 bosses.
                if in_range(bsx, bsy, 58.0) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 56.0,
                        color: (255, 145, 95),
                        intensity: 210,
                        scatter: 48,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }
        }

        // Every active boss hazard gets a high-priority self-lit warning pool.
        for hazard in &boss.hazards {
            if hazard.is_expired() {
                continue;
            }
            let c = hazard.shape.center();
            let sx = c.x - cam.x;
            let sy = c.y - cam.y;
            let active = hazard.is_active();
            let r = if active { 46.0 } else { 36.0 };
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: if active {
                        (255, 92, 66)
                    } else {
                        (255, 195, 76)
                    },
                    intensity: if active { 260 } else { 205 },
                    scatter: if active { 68 } else { 44 },
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }
    }

    /// Build a compact L1-friendly tile opacity grid covering the visible viewport
    /// plus a 4-tile margin (`0 = open/doorway`, `1 = rat-hole`, `2 = solid wall`).
    fn build_occluders(&mut self, level: &Level, cam: Vec2, pb_w: usize, pb_h: usize) {
        let margin = 4;
        let x0 = (cam.x / TILE_F).floor() as i32 - margin;
        let y0 = (cam.y / TILE_F).floor() as i32 - margin;
        let x1 = ((cam.x + pb_w as f32) / TILE_F).ceil() as i32 + margin;
        let y1 = ((cam.y + pb_h as f32) / TILE_F).ceil() as i32 + margin;
        let w = (x1 - x0 + 1).max(1) as usize;
        let h = (y1 - y0 + 1).max(1) as usize;
        self.occ_x0 = x0;
        self.occ_y0 = y0;
        self.occ_w = w;
        self.occ_h = h;
        let len = w * h;
        if self.occluders.len() != len {
            self.occluders.resize(len, 0);
        }

        let tiles = &level.tiles;
        for row in 0..h {
            let cy = y0 + row as i32;
            let base = row * w;
            for col in 0..w {
                let cx = x0 + col as i32;
                let occ = match tiles.get(cx, cy) {
                    TileKind::Open => 0,
                    TileKind::Wall(WallKind::Doorway { .. }) => 0,
                    TileKind::Wall(WallKind::RatHole { .. }) => 1,
                    TileKind::Wall(WallKind::OneWayDoor { .. }) => {
                        let open = level
                            .objects
                            .door_at(cx, cy)
                            .is_some_and(|d| d.open_progress() > 0.35);
                        if open { 0 } else { 2 }
                    }
                    TileKind::Wall(_) => 2,
                };
                self.occluders[base + col] = occ;
            }
        }
    }

    #[inline]
    fn sample_occluder(&self, tx: i32, ty: i32) -> u8 {
        let lx = tx - self.occ_x0;
        let ly = ty - self.occ_y0;
        if lx < 0 || ly < 0 {
            return 0;
        }
        let (ux, uy) = (lx as usize, ly as usize);
        if ux < self.occ_w && uy < self.occ_h {
            self.occluders[uy * self.occ_w + ux]
        } else {
            0
        }
    }

    /// Cheap low-resolution tile visibility test from light world position `(lx, ly)`
    /// to cell world position `(cx, cy)`. Returns `0..=256` (`256` = unoccluded).
    /// Excludes the light's own tile and the target cell's tile so facing wall
    /// faces stay illuminated while floor behind a wall falls into shadow.
    #[inline]
    fn wall_visibility(&self, lx: f32, ly: f32, cx: f32, cy: f32) -> u32 {
        let ltx = (lx / TILE_F).floor() as i32;
        let lty = (ly / TILE_F).floor() as i32;
        let ctx = (cx / TILE_F).floor() as i32;
        let cty = (cy / TILE_F).floor() as i32;
        if (ctx - ltx).abs() <= 1 && (cty - lty).abs() <= 1 {
            return 256;
        }
        let dx = cx - lx;
        let dy = cy - ly;
        let max_d = dx.abs().max(dy.abs());
        // Sample every ~12 world pixels (3/4 of a 16 px tile) so thin 1-tile walls are never skipped.
        let steps = ((max_d / 12.0).ceil() as i32).clamp(2, 10);
        let inv = 1.0 / steps as f32;
        let sx = dx * inv;
        let sy = dy * inv;
        let mut px = lx + sx;
        let mut py = ly + sy;
        let mut vis: u32 = 256;
        let mut prev = (ltx, lty);
        for _ in 1..steps {
            let tx = (px / TILE_F).floor() as i32;
            let ty = (py / TILE_F).floor() as i32;
            if (tx, ty) != prev && (tx, ty) != (ctx, cty) {
                match self.sample_occluder(tx, ty) {
                    2 => {
                        vis = (vis * 48) >> 8;
                        if vis <= 12 {
                            return 0;
                        }
                    }
                    1 => {
                        vis = (vis * 136) >> 8;
                    }
                    _ => {}
                }
                prev = (tx, ty);
            }
            px += sx;
            py += sy;
        }
        vis
    }

    /// Splat one point light onto the low-resolution `mul` + `add` grids.
    fn splat_light(
        &mut self,
        light: PointLight,
        cam: Vec2,
        use_occlusion: bool,
        int_scale: u32,
        scatter_scale: u32,
    ) {
        if light.radius <= 1.0 || (light.intensity == 0 && light.scatter == 0) {
            return;
        }
        let r = light.radius;
        let r_cells = (r / LIGHT_CELL as f32).ceil() as i32;
        let cx_cell = (light.x / LIGHT_CELL as f32).round() as i32;
        let cy_cell = (light.y / LIGHT_CELL as f32).round() as i32;
        let gx0 = (cx_cell - r_cells).max(0) as usize;
        let gx1 = (cx_cell + r_cells).min(self.lw as i32 - 1);
        let gy0 = (cy_cell - r_cells).max(0) as usize;
        let gy1 = (cy_cell + r_cells).min(self.lh as i32 - 1);
        if gx0 as i32 > gx1 || gy0 as i32 > gy1 {
            return;
        }

        let r2_i = (r * r).max(1.0) as i32;
        let lx_i = light.x.round() as i32;
        let ly_i = light.y.round() as i32;
        let lx_w = light.x + cam.x;
        let ly_w = light.y + cam.y;
        let do_occ = use_occlusion && light.occlude && r >= 26.0;

        // Pre-scale light RGB by intensity (Q8.8) and scatter (0..255).
        let lr = light.color.0 as u32;
        let lg = light.color.1 as u32;
        let lb = light.color.2 as u32;
        let eff_int = (light.intensity as u32 * int_scale) >> 8;
        let mr_base = (lr * eff_int) >> 8;
        let mg_base = (lg * eff_int) >> 8;
        let mb_base = (lb * eff_int) >> 8;
        let sc = (light.scatter as u32 * scatter_scale) >> 8;
        let ar_base = (lr * sc) >> 8;
        let ag_base = (lg * sc) >> 8;
        let ab_base = (lb * sc) >> 8;

        for gy in gy0..=(gy1 as usize) {
            let py = (gy * LIGHT_CELL) as i32;
            let dy = py - ly_i;
            let dy2 = dy * dy;
            if dy2 >= r2_i {
                continue;
            }
            let row = gy * self.lw;
            let wy = py as f32 + cam.y;
            for gx in gx0..=(gx1 as usize) {
                let px = (gx * LIGHT_CELL) as i32;
                let dx = px - lx_i;
                let d2 = dx * dx + dy2;
                if d2 >= r2_i {
                    continue;
                }
                // Pure integer falloff `t` in `0..=256`:
                // `lin = 1 - d^2 / r^2`. Smooth lights square it (`lin^2`), while
                // Lab hard pools keep a brighter plateau then drop sharply at the rim.
                let lin = (((r2_i - d2) as u32) << 8) / (r2_i as u32);
                let mut fall = if light.hard {
                    // Hard pool: bright flat inner core, crisp edge.
                    ((lin * 320) >> 8).min(256)
                } else {
                    (lin * lin) >> 8
                };

                if do_occ {
                    let vis = self.wall_visibility(lx_w, ly_w, px as f32 + cam.x, wy);
                    if vis == 0 {
                        continue;
                    }
                    fall = (fall * vis) >> 8;
                }
                if fall == 0 {
                    continue;
                }

                let idx = row + gx;
                let m = &mut self.mul[idx];
                m[0] = (m[0] + ((mr_base * fall) >> 8) as u16).min(MAX_MUL_Q8);
                m[1] = (m[1] + ((mg_base * fall) >> 8) as u16).min(MAX_MUL_Q8);
                m[2] = (m[2] + ((mb_base * fall) >> 8) as u16).min(MAX_MUL_Q8);

                let a = &mut self.add[idx];
                a[0] = (a[0] + ((ar_base * fall) >> 8) as u16).min(MAX_ADD_U16);
                a[1] = (a[1] + ((ag_base * fall) >> 8) as u16).min(MAX_ADD_U16);
                a[2] = (a[2] + ((ab_base * fall) >> 8) as u16).min(MAX_ADD_U16);
            }
        }
    }

    /// Heartbeat crimson border vignette at low HP, applied on the low-res grid.
    fn apply_low_hp_vignette(&mut self, hp_frac: f32, elapsed: f32, mode: LightingMode) {
        let urgency = (1.0 - hp_frac / 0.35).clamp(0.0, 1.0);
        let pulse = 0.68 + 0.32 * (elapsed * 5.5).sin().abs();
        let strength = urgency * pulse * if mode == LightingMode::Soft { 0.5 } else { 1.0 };
        let floor = mode.min_ambient_q8();
        let inv_w = 2.0 / (self.lw.saturating_sub(1).max(1) as f32);
        let inv_h = 2.0 / (self.lh.saturating_sub(1).max(1) as f32);

        for gy in 0..self.lh {
            let ny = gy as f32 * inv_h - 1.0;
            let ny2 = ny * ny;
            let row = gy * self.lw;
            for gx in 0..self.lw {
                let nx = gx as f32 * inv_w - 1.0;
                let d2 = nx * nx + ny2;
                if d2 <= 0.45 {
                    continue;
                }
                let edge = ((d2 - 0.45) / 1.35).clamp(0.0, 1.0);
                let k = edge * edge * strength;
                let idx = row + gx;
                let m = &mut self.mul[idx];
                let dim = (k * 46.0) as u16;
                m[1] = m[1].saturating_sub(dim).max(floor);
                m[2] = m[2].saturating_sub(dim).max(floor);
                let a = &mut self.add[idx];
                a[0] = (a[0] + (k * 54.0) as u16).min(MAX_ADD_U16);
            }
        }
    }

    /// Bilinearly upsample the low-res `mul` + `add` light grids onto `pb` using
    /// pure integer arithmetic, keeping high-luminance gameplay pixels self-lit.
    fn composite_onto(&self, pb: &mut PixelBuffer) {
        let bw = pb.w;
        let bh = pb.h;
        if bw == 0 || bh == 0 || self.lw < 2 || self.lh < 2 {
            return;
        }
        let lw = self.lw;
        let pixels = pb.pixels_mut();

        for gy in 0..(self.lh - 1) {
            let y0 = gy * LIGHT_CELL;
            if y0 >= bh {
                break;
            }
            let y_max = (y0 + LIGHT_CELL).min(bh);
            let row0 = gy * lw;
            let row1 = row0 + lw;

            for gx in 0..(lw - 1) {
                let x0 = gx * LIGHT_CELL;
                if x0 >= bw {
                    break;
                }
                let x_max = (x0 + LIGHT_CELL).min(bw);

                let m00 = self.mul[row0 + gx];
                let m10 = self.mul[row0 + gx + 1];
                let m01 = self.mul[row1 + gx];
                let m11 = self.mul[row1 + gx + 1];

                let a00 = self.add[row0 + gx];
                let a10 = self.add[row0 + gx + 1];
                let a01 = self.add[row1 + gx];
                let a11 = self.add[row1 + gx + 1];

                for y in y0..y_max {
                    let dy = (y - y0) as i32;
                    let wy0 = 4 - dy;

                    // Interpolate left and right edges vertically (still scaled by 4).
                    let ml_r = wy0 * m00[0] as i32 + dy * m01[0] as i32;
                    let ml_g = wy0 * m00[1] as i32 + dy * m01[1] as i32;
                    let ml_b = wy0 * m00[2] as i32 + dy * m01[2] as i32;
                    let mr_r = wy0 * m10[0] as i32 + dy * m11[0] as i32;
                    let mr_g = wy0 * m10[1] as i32 + dy * m11[1] as i32;
                    let mr_b = wy0 * m10[2] as i32 + dy * m11[2] as i32;

                    let al_r = wy0 * a00[0] as i32 + dy * a01[0] as i32;
                    let al_g = wy0 * a00[1] as i32 + dy * a01[1] as i32;
                    let al_b = wy0 * a00[2] as i32 + dy * a01[2] as i32;
                    let ar_r = wy0 * a10[0] as i32 + dy * a11[0] as i32;
                    let ar_g = wy0 * a10[1] as i32 + dy * a11[1] as i32;
                    let ar_b = wy0 * a10[2] as i32 + dy * a11[2] as i32;

                    let prow = y * bw;
                    for x in x0..x_max {
                        let dx = (x - x0) as i32;
                        let wx0 = 4 - dx;

                        let mut mr = ((wx0 * ml_r + dx * mr_r) >> 4) as u32;
                        let mut mg = ((wx0 * ml_g + dx * mr_g) >> 4) as u32;
                        let mut mb = ((wx0 * ml_b + dx * mr_b) >> 4) as u32;
                        let ar = ((wx0 * al_r + dx * ar_r) >> 4) as u32;
                        let ag = ((wx0 * al_g + dx * ar_g) >> 4) as u32;
                        let ab = ((wx0 * al_b + dx * ar_b) >> 4) as u32;

                        let px = pixels[prow + x];
                        let peak = px.0.max(px.1).max(px.2);
                        if peak > SELF_LIT_THRESHOLD {
                            // Self-lit boost for bullets, warning tells, sparks, bright highlights.
                            let lift = ((peak - SELF_LIT_THRESHOLD) as u32 * 256)
                                / (255 - SELF_LIT_THRESHOLD) as u32;
                            if mr < 256 {
                                mr += ((256 - mr) * lift) >> 8;
                            }
                            if mg < 256 {
                                mg += ((256 - mg) * lift) >> 8;
                            }
                            if mb < 256 {
                                mb += ((256 - mb) * lift) >> 8;
                            }
                        }

                        // Soft-shoulder light scatter: dark floors absorb the full colored
                        // pool while bright sprites (rat coat, steel, sparks) keep crisp detail.
                        let base_r = ((px.0 as u32 * mr) >> 8).min(255);
                        let base_g = ((px.1 as u32 * mg) >> 8).min(255);
                        let base_b = ((px.2 as u32 * mb) >> 8).min(255);
                        let nr = (base_r + ((ar * (255 - base_r)) >> 8)).min(255) as u8;
                        let ng = (base_g + ((ag * (255 - base_g)) >> 8)).min(255) as u8;
                        let nb = (base_b + ((ab * (255 - base_b)) >> 8)).min(255) as u8;
                        pixels[prow + x] = (nr, ng, nb);
                    }
                }
            }
        }
    }

    /// Paint tiny self-lit glowing eyes on bats lurking in dim shadows.
    fn draw_dark_bat_eyes(&self, pb: &mut PixelBuffer) {
        if self.bat_eyes.is_empty() || self.lw == 0 || self.lh == 0 {
            return;
        }
        for &(x, y, col) in &self.bat_eyes {
            if x < 0 || y < 0 {
                continue;
            }
            let (ux, uy) = (x as usize, y as usize);
            if ux >= pb.w || uy >= pb.h {
                continue;
            }
            let gx = (ux / LIGHT_CELL).min(self.lw - 1);
            let gy = (uy / LIGHT_CELL).min(self.lh - 1);
            let m = self.mul[gy * self.lw + gx];
            let avg = (m[0] as u32 + m[1] as u32 + m[2] as u32) / 3;
            // Only ignite eerie glowing eyes when the bat is in dim shadow.
            if avg < 215 {
                pb.plot(x, y, col);
            }
        }
    }
}

#[inline]
fn blend_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline]
fn brighten_rgb(c: Rgb, boost: u8) -> Rgb {
    (
        c.0.saturating_add(boost),
        c.1.saturating_add(boost),
        c.2.saturating_add(boost),
    )
}

#[inline]
fn pos_hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^ (h >> 15)
}

/// Fast pixel luminance approximation (0..=255) for readability assertions.
#[inline]
pub fn luminance(c: Rgb) -> u8 {
    ((c.0 as u32 * 77 + c.1 as u32 * 150 + c.2 as u32 * 29) >> 8) as u8
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallAxis;
    use crate::game::world::World;
    use std::time::Instant;

    #[test]
    fn mode_defaults_follow_terminal_color_depth() {
        assert_eq!(
            LightingMode::default_for_depth(ColorDepth::Truecolor),
            LightingMode::Full
        );
        assert_eq!(
            LightingMode::default_for_depth(ColorDepth::Ansi256),
            LightingMode::Soft
        );
        assert_eq!(
            LightingMode::default_for_depth(ColorDepth::Ansi16),
            LightingMode::Off
        );
    }

    #[test]
    fn mode_parsing_and_stepping_round_trip() {
        assert_eq!(LightingMode::parse_setting("full"), Some(LightingMode::Full));
        assert_eq!(LightingMode::parse_setting("SOFT"), Some(LightingMode::Soft));
        assert_eq!(LightingMode::parse_setting(" off "), Some(LightingMode::Off));
        assert_eq!(LightingMode::parse_setting("invalid"), None);
        assert_eq!(parse_env_override("soft"), Some(LightingMode::Soft));
        assert_eq!(parse_env_override(""), None);

        assert_eq!(LightingMode::Full.step(1), LightingMode::Soft);
        assert_eq!(LightingMode::Soft.step(1), LightingMode::Off);
        assert_eq!(LightingMode::Off.step(1), LightingMode::Full);
        assert_eq!(LightingMode::Full.step(-1), LightingMode::Off);
        assert_eq!(LightingMode::Off.step(-1), LightingMode::Soft);
        assert_eq!(LightingMode::Soft.step(-1), LightingMode::Full);
        assert_eq!(LightingMode::Full.step(0), LightingMode::Full);
    }

    #[test]
    fn ambient_floors_never_drop_below_readable_minimum() {
        for theme in Theme::playable() {
            for mode in [LightingMode::Full, LightingMode::Soft, LightingMode::Off] {
                let floor = mode.min_ambient_q8();
                for boss_phase in [None, Some(1), Some(2), Some(3)] {
                    let amb = effective_ambient(theme, mode, boss_phase);
                    for ch in amb {
                        assert!(
                            ch >= floor,
                            "{theme:?}/{mode:?}/phase={boss_phase:?}: channel {ch} < floor {floor}"
                        );
                    }
                }
            }
        }
        // Theme identities: Sewer cool green-blue, Kitchen warm, Lab cold blue.
        let sewer = theme_ambient(Theme::Sewer).mul;
        let kitchen = theme_ambient(Theme::Kitchen).mul;
        let lab = theme_ambient(Theme::Lab).mul;
        assert!(sewer[1] > sewer[0] && sewer[2] > sewer[0], "sewer is green-blue");
        assert!(kitchen[0] > kitchen[2], "kitchen is warm");
        assert!(lab[2] > lab[0], "lab is cold blue");
        assert!(theme_ambient(Theme::Lab).hard_pools);
        // The flatbed is outdoors: brighter than every indoor theme on every
        // channel, and warm (red above blue).
        let road = theme_ambient(Theme::Trailer).mul;
        for indoor in [sewer, kitchen, lab] {
            assert!((0..3).all(|c| road[c] > indoor[c]), "the road is daylight");
        }
        assert!(road[0] > road[2], "daylight is slightly warm");
    }

    #[test]
    fn light_map_cap_keeps_highest_priority_and_nearest_lights() {
        let mut lm = LightMap::new();
        let mut lights = Vec::new();
        // 50 low-priority distant gems.
        for i in 0..50 {
            lights.push(PointLight {
                x: 80.0 + (i as f32 + 10.0),
                y: 80.0,
                radius: 16.0,
                color: (100, 220, 255),
                intensity: 100,
                scatter: 20,
                priority: 0,
                occlude: false,
                hard: false,
            });
        }
        // 2 high-priority lights far away: must survive over priority-0 gems.
        lights.push(PointLight {
            x: 10.0,
            y: 10.0,
            radius: 40.0,
            color: (255, 80, 60),
            intensity: 240,
            scatter: 60,
            priority: 3,
            occlude: false,
            hard: false,
        });
        lights.push(PointLight {
            x: 150.0,
            y: 150.0,
            radius: 40.0,
            color: (255, 220, 80),
            intensity: 240,
            scatter: 60,
            priority: 3,
            occlude: false,
            hard: false,
        });

        let mut pb = PixelBuffer::new(160, 168);
        pb.clear((60, 60, 60));
        lm.apply_custom(&mut pb, [128, 128, 128], LightingMode::Full, (80.0, 80.0), &lights);
        assert_eq!(lm.active_light_count(), MAX_LIGHTS_FULL);
        // Both priority-3 lights survived.
        assert_eq!(lm.lights.iter().filter(|l| l.priority == 3).count(), 2);
        // The remaining 38 are the 38 nearest gems (x <= 80 + 47).
        for l in lm.lights.iter().filter(|l| l.priority == 0) {
            assert!(l.x <= 80.0 + 47.5, "distant gem x={} should have been culled", l.x);
        }

        // Soft mode enforces its tighter cap.
        lm.apply_custom(&mut pb, [128, 128, 128], LightingMode::Soft, (80.0, 80.0), &lights);
        assert_eq!(lm.active_light_count(), MAX_LIGHTS_SOFT);
    }

    #[test]
    fn light_brightens_center_and_self_lit_pixels_resist_darkening() {
        let mut lm = LightMap::new();
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear((80, 80, 80));
        // Place a bright hostile bullet pixel in the unlit corner (2, 2).
        pb.plot(2, 2, (255, 70, 90));

        let light = PointLight {
            x: 32.0,
            y: 32.0,
            radius: 24.0,
            color: (255, 220, 160),
            intensity: 240,
            scatter: 48,
            priority: 3,
            occlude: false,
            hard: false,
        };
        lm.apply_custom(&mut pb, [110, 110, 110], LightingMode::Full, (32.0, 32.0), &[light]);

        let lit_center = pb.pixel_at(32, 32);
        let dark_corner = pb.pixel_at(4, 4);
        let bullet_px = pb.pixel_at(2, 2);
        assert!(
            luminance(lit_center) > luminance(dark_corner) + 40,
            "center {lit_center:?} should be much brighter than dark corner {dark_corner:?}"
        );
        // Dark corner stays readable (>= ambient floor applied to 80 ≈ 34).
        assert!(dark_corner.0 >= 30, "ambient floor violated: {dark_corner:?}");
        // Self-lit bullet pixel keeps near-full red channel even in deep shadow.
        assert!(
            bullet_px.0 >= 245,
            "hostile bullet pixel should stay self-lit, got {bullet_px:?}"
        );
    }

    #[test]
    fn lighting_off_is_exact_noop() {
        let mut lm = LightMap::new();
        let mut pb = PixelBuffer::new(40, 40);
        pb.clear((42, 84, 126));
        let before = pb.pixels().to_vec();
        let light = PointLight {
            x: 20.0,
            y: 20.0,
            radius: 20.0,
            color: (255, 255, 255),
            intensity: 256,
            scatter: 60,
            priority: 3,
            occlude: false,
            hard: false,
        };
        lm.apply_custom(&mut pb, [110, 110, 110], LightingMode::Off, (20.0, 20.0), &[light]);
        assert_eq!(pb.pixels(), before.as_slice());
    }

    #[test]
    fn wall_occlusion_shadows_behind_solid_wall_and_spills_through_doorway() {
        let mut level = Level::empty(Theme::Sewer);
        // Vertical wall at col 102, rows 76..85, with a doorway at (102, 80).
        level.tiles.fill_cells(102, 76, 1, 9, TileKind::Wall(WallKind::Solid));
        level.tiles.set(
            102,
            80,
            TileKind::Wall(WallKind::Doorway {
                axis: WallAxis::Vertical,
            }),
        );
        level.tiles.rebuild_masks();

        let mut lm = LightMap::new();
        let cam = Vec2::new(100.0 * TILE_F - 80.0, 80.0 * TILE_F - 80.0);
        lm.build_occluders(&level, cam, 160, 160);

        // Light on the west side of the wall at cell (100.5, 80.5).
        let lx = 100.5 * TILE_F;
        let ly = 80.5 * TILE_F;
        // Through doorway at (102, 80) to (104.5, 80.5): unoccluded!
        let vis_door = lm.wall_visibility(lx, ly, 104.5 * TILE_F, 80.5 * TILE_F);
        // Through solid wall at (102, 78) to (104.5, 78.5): shadowed!
        let vis_wall = lm.wall_visibility(lx, ly, 104.5 * TILE_F, 78.5 * TILE_F);
        assert_eq!(vis_door, 256, "light should spill freely through doorway");
        assert!(
            vis_wall < 80,
            "solid wall should strongly occlude light behind it (got {vis_wall})"
        );
    }

    #[test]
    fn degenerate_viewports_do_not_panic() {
        for (w, h) in [(0usize, 0usize), (1, 1), (5, 3), (3, 5), (4, 4), (7, 9)] {
            let mut world = World::new(Theme::Sewer, 7);
            world.lighting = LightingMode::Full;
            let mut pb = PixelBuffer::new(w, h);
            world.draw(&mut pb);
            world.lighting = LightingMode::Soft;
            world.draw(&mut pb);
            world.lighting = LightingMode::Off;
            world.draw(&mut pb);
        }
    }

    #[test]
    fn lighting_pass_performance_within_budget() {
        let mut world = World::new(Theme::Lab, 42);
        // Populate a busy scene: 200 enemies, bullets, pools, gems, pickups.
        let center = world.player.pos;
        for i in 0..200 {
            let a = i as f32 * 0.37;
            let r = 12.0 + (i % 8) as f32 * 11.0;
            let kind = match i % 5 {
                0 => EnemyKind::Skitterer,
                1 => EnemyKind::Bat,
                2 => EnemyKind::Spitter,
                3 => EnemyKind::Cat,
                _ => EnemyKind::Brute,
            };
            world.spawn_enemy(kind, center + Vec2::from_angle(a) * r, 1.0);
        }
        for i in 0..40 {
            let a = i as f32 * 0.15;
            world.spawn_bullet(Bullet::new(
                center + Vec2::from_angle(a) * 28.0,
                Vec2::from_angle(a) * 80.0,
                5.0,
                if i % 3 == 0 { Faction::Enemy } else { Faction::Player },
                (255, 180, 90),
            ));
        }
        for i in 0..8 {
            let a = i as f32 * 0.78;
            world.spawn_pool(Pool {
                pos: center + Vec2::from_angle(a) * 35.0,
                radius: 11.0,
                dps: 6.0,
                life: 2.0,
                color: (150, 255, 90),
            });
        }

        // Benchmark 120x40 cells (120x80 px), 160x168 px, and 2x zoom (320x336 px).
        let mut lm = LightMap::new();
        for (label, w, h) in [
            ("120x40 cells (120x80 px)", 120usize, 80usize),
            ("160x84 cells (160x168 px)", 160, 168),
            ("2x zoom (320x336 px)", 320, 336),
        ] {
            let mut pb = PixelBuffer::new(w, h);
            pb.clear((40, 50, 60));
            let scene = LightScene {
                ecs: &world.ecs,
                level: &world.level,
                player: &world.player,
                particles: &world.particles,
                thrash: None,
                squeak: None,
                cam: center - Vec2::new(w as f32 * 0.5, h as f32 * 0.5),
                elapsed: 12.0,
                mode: LightingMode::Full,
            };
            // Warm up scratch buffers.
            lm.apply_scene(&mut pb, &scene);

            let iters = 30u32;
            let t0 = Instant::now();
            for _ in 0..iters {
                lm.apply_scene(&mut pb, &scene);
            }
            let us = t0.elapsed().as_secs_f64() * 1e6 / iters as f64;
            eprintln!("[lighting bench] {label}: {us:.1} us/frame (lights={})", lm.active_light_count());
            // Even in dev opt-level=1, 120x80 must be well under 3 ms (and release << 1 ms).
            assert!(us < 16_000.0, "{label} exceeded 60 FPS frame budget: {us:.1} us");
        }
    }

    fn write_ppm(path: &std::path::Path, pb: &PixelBuffer) {
        let mut data = Vec::with_capacity(32 + pb.w * pb.h * 3);
        use std::io::Write;
        writeln!(data, "P6\n{} {}\n255", pb.w, pb.h).unwrap();
        for &(r, g, b) in pb.pixels() {
            data.push(r);
            data.push(g);
            data.push(b);
        }
        std::fs::write(path, data).unwrap();
    }

    /// Gated behind `RATRUN_LIGHTING_CAPTURES=<dir>`: renders the visual review
    /// gallery (busy frame on Sewer/Kitchen/Lab at Off/Soft/Full, boss fights,
    /// nuke, 3-frame explosion smoke progression, low-HP vignette).
    #[test]
    fn capture_lighting_gallery() {
        let Some(dir) = std::env::var_os("RATRUN_LIGHTING_CAPTURES") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        use crate::game::bullet::Orbit;
        use crate::game::ecs::components::SpawnId;
        use crate::game::ecs::systems::boss::{Boss, BossKind};
        use crate::game::gem::{Gem, Pickup, PickupKind};
        use crate::game::level::{Prop, Rectf, TileRect};
        use crate::game::particle::vfx;
        use crate::game::weapon::WeaponKind;
        use rand::SeedableRng;

        let (vw, vh) = (160usize, 120usize);

        // Helper to build a rich, busy scene for a given theme with real props,
        // a doorway wall, enemies (including bats in dark corners), bullets,
        // orbiting spores, acid pool, pickup, gems, and live smoke.
        let build_busy = |theme: Theme| -> World {
            let mut w = World::new(theme, 17);
            let center = w.player.pos;
            // Place a partial wall with a doorway just east of the player so wall
            // occlusion and doorway light spill are clearly visible!
            let (pcx, pcy) = crate::game::collision::cell_of(center);
            w.level
                .tiles
                .fill_cells(pcx + 2, pcy - 3, 1, 7, TileKind::Wall(WallKind::Solid));
            w.level.tiles.set(
                pcx + 2,
                pcy,
                TileKind::Wall(WallKind::Doorway {
                    axis: WallAxis::Vertical,
                }),
            );
            w.level.tiles.rebuild_masks();

            // Place theme-appropriate illuminated props inside view.
            #[allow(unreachable_patterns)]
            let prop_kinds = match theme {
                Theme::Sewer => [PropKind::Grate, PropKind::Valve],
                Theme::Kitchen => [PropKind::Counter, PropKind::Table],
                Theme::Lab => [PropKind::Console, PropKind::Tank],
                _ => [PropKind::Grate, PropKind::Valve],
            };
            let (pw0, ph0) = prop_kinds[0].size();
            let (pw1, ph1) = prop_kinds[1].size();
            w.level.props.push(Prop {
                kind: prop_kinds[0],
                rect: Rectf::new(center.x - 58.0, center.y - 36.0, pw0, ph0),
            });
            w.level.props.push(Prop {
                kind: prop_kinds[1],
                rect: Rectf::new(center.x - 52.0, center.y + 18.0, pw1, ph1),
            });

            let mut rng = rand::rngs::StdRng::seed_from_u64(99);
            // Enemies including bats in the dark corners behind the wall.
            for (kind, ofs) in [
                (EnemyKind::Skitterer, Vec2::new(-20.0, -14.0)),
                (EnemyKind::Skitterer, Vec2::new(-28.0, 12.0)),
                (EnemyKind::Cat, Vec2::new(14.0, -24.0)),
                (EnemyKind::Spitter, Vec2::new(-10.0, 26.0)),
                (EnemyKind::Brute, Vec2::new(18.0, 22.0)),
                (EnemyKind::Bat, Vec2::new(56.0, -26.0)),
                (EnemyKind::Bat, Vec2::new(62.0, 24.0)),
                (EnemyKind::Bat, Vec2::new(-64.0, -2.0)),
                (EnemyKind::Skitterer, Vec2::new(52.0, 2.0)),
            ] {
                w.spawn_enemy_with_rng(kind, center + ofs, 2.0, &mut rng);
            }
            // Acid pool, pickup, gems, player bullets, orbiting spores, enemy bullets.
            w.spawn_pool(Pool {
                pos: center + Vec2::new(-22.0, 20.0),
                radius: 12.0,
                dps: 6.0,
                life: 2.4,
                color: (150, 255, 90),
            });
            w.spawn_pickup(Pickup {
                pos: center + Vec2::new(12.0, -34.0),
                kind: PickupKind::Heal,
                bob: 1.1,
            });
            for ofs in [
                Vec2::new(-12.0, -22.0),
                Vec2::new(8.0, 16.0),
                Vec2::new(-38.0, 6.0),
            ] {
                w.spawn_gem(Gem::new(center + ofs, 25));
            }
            for (ang, wkind, col) in [
                (0.2f32, WeaponKind::CheeseSpray, WeaponKind::CheeseSpray.color()),
                (-0.4, WeaponKind::SqueakNova, WeaponKind::SqueakNova.color()),
                (2.4, WeaponKind::Gnaw, WeaponKind::Gnaw.color()),
            ] {
                let d = Vec2::from_angle(ang);
                let mut b = Bullet::new(center + d * 14.0, d * 90.0, 6.0, Faction::Player, col);
                b.weapon = Some(wkind);
                b.size = 2;
                w.spawn_bullet(b);
            }
            // Orbiting spores.
            for k in 0..3 {
                let a = k as f32 * 2.09;
                let mut b = Bullet::new(
                    center + Vec2::from_angle(a) * 18.0,
                    Vec2::ZERO,
                    5.0,
                    Faction::Player,
                    WeaponKind::SporeOrbit.color(),
                );
                b.orbit = Some(Orbit {
                    angle: a,
                    radius: 18.0,
                    speed: 3.0,
                });
                b.size = 2;
                w.spawn_bullet(b);
            }
            // Hostile bullets in both lit and shadowed areas (must stay self-lit!).
            for ofs in [Vec2::new(54.0, -14.0), Vec2::new(-16.0, 32.0), Vec2::new(48.0, 18.0)] {
                let mut b = Bullet::new(center + ofs, Vec2::new(-40.0, 0.0), 6.0, Faction::Enemy, (255, 75, 110));
                b.size = 2;
                w.spawn_bullet(b);
            }
            // Pre-warm smoke emitters with a few ticks of continuous emission.
            w.smoke_mut().emit_dash_dust(center + Vec2::new(-6.0, 4.0), Vec2::new(1.0, 0.0), theme, 1.0);
            w.elapsed = 12.0;
            let mut pb = PixelBuffer::new(vw, vh);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            for step in 0..10 {
                w.elapsed = 12.0 + step as f32 * 0.08;
                w.draw(&mut pb);
            }
            w
        };

        // 1. Busy frame on Sewer, Kitchen, Lab at Off / Soft / Full.
        for (name, theme) in [
            ("sewer", Theme::Sewer),
            ("kitchen", Theme::Kitchen),
            ("lab", Theme::Lab),
        ] {
            let mut w = build_busy(theme);
            let mut pb = PixelBuffer::new(vw, vh);
            for (mname, mode, smoke_on) in [
                ("off", LightingMode::Off, false),
                ("soft", LightingMode::Soft, true),
                ("full", LightingMode::Full, true),
            ] {
                w.lighting = mode;
                w.smoke_enabled = smoke_on;
                w.draw(&mut pb);
                write_ppm(&dir.join(format!("{name}_{mname}.ppm")), &pb);
            }
        }

        // 2. Boss fights: Lab Mower (headlamp + lidar scan + exhaust) & Sewer Cat (pounce tell).
        {
            let mut w = World::new(Theme::Lab, 33);
            let c = w.player.pos;
            let dock = c + Vec2::new(48.0, -28.0);
            w.level.boss.dock = Some(dock);
            let mut boss = Boss::new(
                BossKind::Mower,
                c + Vec2::new(-38.0, -8.0),
                1600.0,
                TileRect::new(90, 70, 20, 20),
                dock,
                false,
            );
            boss.facing = Vec2::new(1.0, 0.15).normalized();
            boss.state = BossState::Mower(MowerState::Scan);
            boss.duration = 1.1;
            boss.elapsed = 0.88; // locked amber-red lidar!
            boss.target = c + Vec2::new(8.0, 2.0);
            boss.phase = 2;
            w.ecs.spawn((boss, SpawnId(9001)));
            let mut pb = PixelBuffer::new(vw, vh);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            for step in 0..8 {
                w.elapsed = 30.0 + step as f32 * 0.08;
                w.draw(&mut pb);
            }
            write_ppm(&dir.join("boss_mower_full.ppm"), &pb);
            w.lighting = LightingMode::Off;
            w.smoke_enabled = false;
            w.draw(&mut pb);
            write_ppm(&dir.join("boss_mower_off.ppm"), &pb);
        }
        {
            let mut w = World::new(Theme::Sewer, 44);
            let c = w.player.pos;
            let mut boss = Boss::new(
                BossKind::Cat,
                c + Vec2::new(-34.0, -14.0),
                2200.0,
                TileRect::new(90, 70, 20, 20),
                c,
                false,
            );
            boss.facing = Vec2::new(1.0, 0.35).normalized();
            boss.state = BossState::Cat(BossCatState::Crouch);
            boss.duration = 0.95;
            boss.elapsed = 0.78; // locked pounce warning ring on player!
            boss.target = c + Vec2::new(6.0, 4.0);
            boss.phase = 2;
            w.ecs.spawn((boss, SpawnId(9002)));
            let mut pb = PixelBuffer::new(vw, vh);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            for step in 0..8 {
                w.elapsed = 40.0 + step as f32 * 0.08;
                w.draw(&mut pb);
            }
            write_ppm(&dir.join("boss_cat_full.ppm"), &pb);
        }

        // 3. Nuke capture (Off vs Full).
        {
            let mut w = build_busy(Theme::Sewer);
            let c = w.player.pos;
            vfx::emit_nuke(&mut w.particles, &mut w.rng, c);
            w.particles.update(0.06);
            w.elapsed += 0.06;
            let mut pb = PixelBuffer::new(vw, vh);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            w.draw(&mut pb);
            write_ppm(&dir.join("nuke_full.ppm"), &pb);
            w.lighting = LightingMode::Off;
            w.smoke_enabled = false;
            w.draw(&mut pb);
            write_ppm(&dir.join("nuke_off.ppm"), &pb);
        }

        // 4. Explosion smoke progression over 3 frames (lit by fireball -> rising -> fading).
        {
            let mut w = World::new(Theme::Sewer, 55);
            let c = w.player.pos;
            let blast = c + Vec2::new(18.0, 2.0);
            vfx::emit_enemy_death(
                &mut w.particles,
                &mut w.rng,
                blast,
                6.5,
                EnemyKind::Brute,
                w.level.palette.blood,
            );
            let mut pb = PixelBuffer::new(vw, vh);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            w.elapsed = 10.0;
            w.draw(&mut pb);
            write_ppm(&dir.join("smoke_exp_f1.ppm"), &pb);

            for _ in 0..14 {
                w.particles.update(1.0 / 60.0);
                w.elapsed += 1.0 / 60.0;
                w.draw(&mut pb);
            }
            write_ppm(&dir.join("smoke_exp_f2.ppm"), &pb);

            for _ in 0..24 {
                w.particles.update(1.0 / 60.0);
                w.elapsed += 1.0 / 60.0;
                w.draw(&mut pb);
            }
            write_ppm(&dir.join("smoke_exp_f3.ppm"), &pb);
        }

        // 5. Low-HP vignette capture.
        {
            let mut w = build_busy(Theme::Sewer);
            w.player.hp = w.player.max_hp() * 0.16;
            let mut pb = PixelBuffer::new(vw, vh);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            w.draw(&mut pb);
            write_ppm(&dir.join("low_hp_full.ppm"), &pb);
        }
    }
}
