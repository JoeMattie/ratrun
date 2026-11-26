//! Brief, alpha-blended smoke from combat and machinery (render-only).
//!
//! Distinct from [`crate::game::particle::Particles`] (crisp pixel sparks,
//! streaks and shockwaves): every [`SmokePuff`] is a soft radial-falloff cloud
//! that expands, rises/drifts with gentle turbulence, fades smoothly, and is
//! illuminated by nearby lights during the subsequent lighting pass.
//!
//! # Determinism & Zero-Allocation Contract
//!
//! - Lives on `World` as purely render-side state; `World::update` never reads
//!   or writes it, and it **never** draws from `World::rng`.
//! - Uses a fixed-capacity inline ring buffer (`[SmokePuff; SMOKE_CAP]`) and
//!   its own tiny deterministic hash/PRNG (`u64` state seeded from level seed
//!   + theme), so zero heap allocations occur per frame.
//! - Smoke and lighting quality never alter simulation state.
//! - **Render-only, so its maths stays on `std`.** Checked: the only callers of
//!   [`SmokeLayer::sync_and_draw`] are `World::draw_smoke` (the draw half) and
//!   the lighting gallery capture, and [`SmokeLayer::step`] is reached only
//!   from there — no tick system touches it. Its `sin` therefore never has to
//!   be bit-identical across platforms, and the `disallowed_methods` lint is
//!   silenced for the file (see `src/detmath.rs`).

// Render-only: the platform maths library is fine here (see the module docs).
#![allow(clippy::disallowed_methods)]

use bevy_ecs::world::World as EcsWorld;

use crate::game::art_boss_broom::BroomBossState;
use crate::game::art_boss_mower::MowerState;
use crate::game::ecs::components::Pool;
use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
use crate::game::level::{Level, Theme};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::LightingMode;
use crate::render::palette::Rgb;

/// Fixed capacity of the inline smoke ring buffer.
pub const SMOKE_CAP: usize = 144;

/// Number of recent shockwave signatures tracked to avoid double-emitting.
const SEEN_SW_CAP: usize = 32;

/// Visual behavior class of a smoke puff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmokeKind {
    /// Thick expanding ash/blast cloud from explosions, nukes, and big deaths.
    Blast,
    /// Thin toxic green fumes rising from acid pools.
    AcidFume,
    /// Chugging engine exhaust or dark oil smoke from the mower boss.
    Exhaust,
    /// Brief pale steam from active machinery.
    Steam,
    /// Ground-tinted dust kick from dashing or Tail Thrash sweeps.
    Dust,
}

/// One soft alpha-blended cloud puff in world-pixel space.
#[derive(Clone, Copy, Debug)]
pub struct SmokePuff {
    pub pos: Vec2,
    pub vel: Vec2,
    pub r0: f32,
    pub r1: f32,
    pub life: f32,
    pub max_life: f32,
    /// Peak opacity (`0.0..=1.0`).
    pub alpha: f32,
    pub color: Rgb,
    pub kind: SmokeKind,
    pub seed: u16,
    pub alive: bool,
}

impl SmokePuff {
    const DEAD: Self = Self {
        pos: Vec2::ZERO,
        vel: Vec2::ZERO,
        r0: 1.0,
        r1: 1.0,
        life: 0.0,
        max_life: 1.0,
        alpha: 0.0,
        color: (0, 0, 0),
        kind: SmokeKind::Blast,
        seed: 0,
        alive: false,
    };

    /// Normalised age in `0.0..=1.0` (`0` = freshly emitted, `1` = expired).
    #[inline]
    pub fn age(&self) -> f32 {
        if self.max_life <= 1e-4 {
            1.0
        } else {
            (1.0 - self.life / self.max_life).clamp(0.0, 1.0)
        }
    }

    /// Current expanding cloud radius in world pixels.
    #[inline]
    pub fn current_radius(&self) -> f32 {
        let t = self.age();
        // Ease-out expansion: billows out rapidly then drifts wide.
        let grow = 1.0 - (1.0 - t) * (1.0 - t);
        self.r0 + (self.r1 - self.r0) * grow
    }

    /// Current opacity (`0.0..=1.0`): rapid eruption for blasts/dust, gentle
    /// fade-in for mist/steam, followed by a smooth billowy fade-out.
    #[inline]
    pub fn current_alpha(&self) -> f32 {
        let t = self.age();
        let fade_in = match self.kind {
            SmokeKind::Blast => 0.55 + 0.45 * (t / 0.10).min(1.0),
            SmokeKind::Dust | SmokeKind::Exhaust => 0.35 + 0.65 * (t / 0.12).min(1.0),
            SmokeKind::AcidFume | SmokeKind::Steam => (t / 0.18).min(1.0),
        };
        let fade_out = (1.0 - t) * (1.0 - t * 0.42);
        (self.alpha * fade_in * fade_out).clamp(0.0, 1.0)
    }
}

/// Read-only view of the world state needed to emit and draw smoke.
pub struct SmokeScene<'a> {
    pub ecs: &'a EcsWorld,
    pub level: &'a Level,
    pub player: &'a Player,
    pub particles: &'a Particles,
    pub thrash: Option<(f32, f32, f32)>,
    pub cam: Vec2,
    pub elapsed: f32,
    pub mode: LightingMode,
}

/// Fixed-capacity, allocation-free render-only smoke system.
pub struct SmokeLayer {
    puffs: [SmokePuff; SMOKE_CAP],
    head: usize,
    rng: u64,
    initialized: bool,
    last_elapsed: f32,
    prev_dash_active: f32,
    prev_thrash_active: bool,
    prev_boss_phase: u8,
    prev_boss_phase_change: bool,
    emit_accum: f32,
    seen_sw: [u64; SEEN_SW_CAP],
    seen_sw_head: usize,
}

impl SmokeLayer {
    pub fn new(seed: u64, theme: Theme) -> Self {
        #[allow(unreachable_patterns)]
        let salt: u64 = match theme {
            Theme::Sewer => 0x517C_C1B7_2722_0A95,
            Theme::Kitchen => 0x94D0_49BB_1331_11EB,
            Theme::Lab => 0xBF58_476D_1CE4_E5B9,
            _ => 0x9E37_79B9_7F4A_7C15,
        };
        let mut s = seed ^ salt ^ 0x5A0C_E123_4567_89AB;
        if s == 0 {
            s = 0xDEAD_BEEF_CAFE_BABE;
        }
        Self {
            puffs: [SmokePuff::DEAD; SMOKE_CAP],
            head: 0,
            rng: s,
            initialized: false,
            last_elapsed: 0.0,
            prev_dash_active: 0.0,
            prev_thrash_active: false,
            prev_boss_phase: 0,
            prev_boss_phase_change: false,
            emit_accum: 0.0,
            seen_sw: [0; SEEN_SW_CAP],
            seen_sw_head: 0,
        }
    }

    /// Count currently alive smoke puffs.
    pub fn active_count(&self) -> usize {
        self.puffs.iter().filter(|p| p.alive).count()
    }

    /// Iterate live puffs (for tests & inspection).
    pub fn active_puffs(&self) -> impl Iterator<Item = &SmokePuff> {
        self.puffs.iter().filter(|p| p.alive)
    }

    /// Push a puff into the O(1) fixed-capacity ring buffer (evicts oldest on wrap).
    #[inline]
    pub fn push(&mut self, puff: SmokePuff) {
        self.puffs[self.head] = puff;
        self.head = (self.head + 1) % SMOKE_CAP;
    }

    /// Advance all active puffs by `dt` seconds (expand, drift, gentle turbulence, fade).
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.1);
        for p in self.puffs.iter_mut() {
            if !p.alive {
                continue;
            }
            p.life -= dt;
            if p.life <= 0.0 {
                p.alive = false;
                continue;
            }
            let drag = match p.kind {
                SmokeKind::Blast => 2.4,
                SmokeKind::Dust => 3.6,
                SmokeKind::Exhaust => 1.8,
                SmokeKind::AcidFume | SmokeKind::Steam => 1.1,
            };
            let damp = (1.0 - drag * dt).max(0.0);
            p.vel = p.vel * damp;
            // Gentle upward buoyancy + deterministic wispy horizontal curl.
            let sway = ((p.life * 3.8 + (p.seed as f32) * 0.17).sin()) * match p.kind {
                SmokeKind::Steam | SmokeKind::AcidFume => 5.5,
                _ => 3.5,
            };
            let rise = match p.kind {
                SmokeKind::Dust => -2.0,
                SmokeKind::Blast => -5.5,
                SmokeKind::Exhaust => -7.0,
                SmokeKind::AcidFume => -8.5,
                SmokeKind::Steam => -10.0,
            };
            p.pos.x += (p.vel.x + sway) * dt;
            p.pos.y += (p.vel.y + rise) * dt;
        }
    }

    /// Observe world state changes, advance puffs, and alpha-blend onto `pb`.
    pub fn sync_and_draw(&mut self, pb: &mut PixelBuffer, scene: &SmokeScene<'_>) {
        if pb.w == 0 || pb.h == 0 {
            return;
        }
        let dt = if !self.initialized {
            self.initialized = true;
            self.last_elapsed = scene.elapsed;
            self.prev_dash_active = scene.player.dash_active;
            self.prev_thrash_active = scene.thrash.is_some();
            0.0
        } else {
            let d = (scene.elapsed - self.last_elapsed).clamp(0.0, 0.1);
            self.last_elapsed = scene.elapsed;
            d
        };

        // Advance existing puffs first.
        if dt > 0.0 {
            self.step(dt);
        }

        let density = if scene.mode == LightingMode::Compact { 0.43 } else { 0.65 };

        // 1. Observe fresh shockwaves (explosions, big deaths, nukes).
        self.observe_shockwaves(scene, pb.w as f32, pb.h as f32, density);

        // 2. Dash dust kick (rising edge on `dash_active`).
        if scene.player.dash_active > self.prev_dash_active + 0.01 {
            self.emit_dash_dust(scene.player.pos, scene.player.face, scene.level.theme, density);
        }
        self.prev_dash_active = scene.player.dash_active;

        // 3. Tail Thrash swirling dust/smoke along the arc.
        if let Some((angle, reach, fade)) = scene.thrash {
            if !self.prev_thrash_active || (dt > 0.0 && fade > 0.25) {
                self.emit_thrash_smoke(scene.player.pos, angle, reach, !self.prev_thrash_active, density);
            }
            self.prev_thrash_active = true;
        } else {
            self.prev_thrash_active = false;
        }

        // 4. Continuous emitters (rate-governed off `dt > 0`, so static draws stay clean).
        if dt > 0.0 {
            self.emit_accum += dt;
            if self.emit_accum >= 0.09 {
                let steps = (self.emit_accum / 0.09).floor().min(3.0) as u32;
                self.emit_accum -= steps as f32 * 0.09;
                for _ in 0..steps {
                    self.emit_continuous(scene, pb.w as f32, pb.h as f32, density);
                }
            }
        }

        // Draw all visible smoke puffs onto `pb`.
        self.draw(pb, scene.cam);
    }

    /// Public emitter for explosions / big deaths (also callable directly by tests).
    pub fn emit_explosion(&mut self, pos: Vec2, blast_radius: f32, is_nuke: bool, density: f32) {
        let base_count = if is_nuke {
            20
        } else if blast_radius >= 40.0 {
            14
        } else {
            10
        };
        let count = ((base_count as f32 * density).round() as usize).max(4);
        let spread = (blast_radius * 0.24).clamp(3.0, 16.0);
        for i in 0..count {
            let a = (i as f32 / count as f32) * std::f32::consts::TAU + self.rand_range(-0.28, 0.28);
            let dir = Vec2::from_angle(a);
            let speed = if is_nuke {
                self.rand_range(18.0, 56.0)
            } else if blast_radius >= 40.0 {
                self.rand_range(14.0, 42.0)
            } else {
                self.rand_range(12.0, 32.0)
            };
            let r0 = if is_nuke {
                self.rand_range(4.0, 5.8)
            } else {
                self.rand_range(3.0, 4.4)
            };
            let r1 = if is_nuke {
                self.rand_range(12.5, 18.0)
            } else if blast_radius >= 40.0 {
                self.rand_range(9.5, 14.5)
            } else {
                self.rand_range(8.0, 12.2)
            };
            let life = if is_nuke {
                self.rand_range(1.25, 2.15)
            } else {
                self.rand_range(0.85, 1.45)
            };
            // Warm ash/smoke tint — catches orange-gold firelight on birth and cools to ash!
            let shade = self.rand_range(132.0, 174.0) as u8;
            let color = (shade.saturating_add(16), shade.saturating_add(6), shade.saturating_sub(8));
            let seed = (self.next_u64() & 0xFFFF) as u16;
            let puff_pos = pos + dir * self.rand_range(0.0, spread);
            let puff = SmokePuff {
                pos: puff_pos,
                vel: dir * speed + Vec2::new(0.0, -5.0),
                r0,
                r1,
                life,
                max_life: life,
                alpha: if is_nuke { 0.38 } else { 0.34 },
                color,
                kind: SmokeKind::Blast,
                seed,
                alive: true,
            };
            self.push(puff);
        }
    }

    /// Emit ground dust kick behind a dashing rat.
    pub fn emit_dash_dust(&mut self, pos: Vec2, face: Vec2, theme: Theme, density: f32) {
        let back = if face.len_sq() > 0.01 {
            -face.normalized()
        } else {
            Vec2::new(-1.0, 0.0)
        };
        let perp = back.perp();
        let col = dust_color(theme);
        let count = ((5.0 * density).round() as usize).max(2);
        for _ in 0..count {
            let lat = self.rand_range(-3.0, 3.0);
            let sp = self.rand_range(14.0, 32.0);
            let life = self.rand_range(0.38, 0.68);
            let seed = (self.next_u64() & 0xFFFF) as u16;
            let vel = back * sp + perp * self.rand_range(-8.0, 8.0);
            let r0 = self.rand_range(1.8, 2.6);
            let r1 = self.rand_range(4.8, 6.8);
            let puff = SmokePuff {
                pos: pos + back * 2.0 + perp * lat,
                vel,
                r0,
                r1,
                life,
                max_life: life,
                alpha: 0.34,
                color: col,
                kind: SmokeKind::Dust,
                seed,
                alive: true,
            };
            self.push(puff);
        }
    }

    fn emit_thrash_smoke(
        &mut self,
        player_pos: Vec2,
        angle: f32,
        reach: f32,
        burst_start: bool,
        density: f32,
    ) {
        let count = if burst_start {
            ((6.0 * density).round() as usize).max(2)
        } else if self.rand_f32() < 0.55 * density {
            1
        } else {
            0
        };
        for k in 0..count {
            let a = angle + if burst_start {
                (k as f32 / count.max(1) as f32 - 0.5) * 2.2
            } else {
                self.rand_range(-0.45, 0.45)
            };
            let out = Vec2::from_angle(a);
            let tang = Vec2::from_angle(a + std::f32::consts::FRAC_PI_2);
            let dist = reach * self.rand_range(0.45, 0.95);
            let life = self.rand_range(0.40, 0.72);
            let seed = (self.next_u64() & 0xFFFF) as u16;
            let vel = tang * self.rand_range(12.0, 26.0) + out * self.rand_range(4.0, 12.0);
            let r0 = self.rand_range(2.0, 2.8);
            let r1 = self.rand_range(5.2, 7.6);
            let puff = SmokePuff {
                pos: player_pos + out * dist,
                vel,
                r0,
                r1,
                life,
                max_life: life,
                alpha: 0.30,
                color: (182, 148, 138),
                kind: SmokeKind::Dust,
                seed,
                alive: true,
            };
            self.push(puff);
        }
    }

    /// Emit radial billowing puffs when a boss changes phase.
    fn emit_boss_phase_plume(&mut self, pos: Vec2, theme: Theme, density: f32) {
        let count = ((12.0 * density).round() as usize).max(5);
        #[allow(unreachable_patterns)]
        let base = match theme {
            Theme::Sewer => (124, 152, 146),
            Theme::Kitchen => (176, 156, 136),
            Theme::Lab => (136, 166, 188),
            _ => (150, 152, 160),
        };
        for i in 0..count {
            let a = (i as f32 / count as f32) * std::f32::consts::TAU + self.rand_range(-0.2, 0.2);
            let dir = Vec2::from_angle(a);
            let life = self.rand_range(0.85, 1.45);
            let seed = (self.next_u64() & 0xFFFF) as u16;
            let puff_pos = pos + dir * self.rand_range(4.0, 10.0);
            let vel = dir * self.rand_range(22.0, 44.0);
            let r0 = self.rand_range(3.0, 4.4);
            let r1 = self.rand_range(9.0, 13.5);
            let puff = SmokePuff {
                pos: puff_pos,
                vel,
                r0,
                r1,
                life,
                max_life: life,
                alpha: 0.28,
                color: base,
                kind: SmokeKind::Blast,
                seed,
                alive: true,
            };
            self.push(puff);
        }
    }

    /// Detect freshly born explosion/death/nuke shockwaves and emit blast smoke once.
    fn observe_shockwaves(&mut self, scene: &SmokeScene<'_>, vw: f32, vh: f32, density: f32) {
        let cam = scene.cam;
        for sw in &scene.particles.shockwaves {
            if sw.end_r < 18.0 {
                continue;
            }
            let age = 1.0 - (sw.life / sw.max_life.max(1e-4)).clamp(0.0, 1.0);
            if age > 0.35 {
                continue;
            }
            let sx = sw.pos.x - cam.x;
            let sy = sw.pos.y - cam.y;
            if sx < -40.0 || sy < -40.0 || sx >= vw + 40.0 || sy >= vh + 40.0 {
                continue;
            }
            let sig = shockwave_sig(sw.pos, sw.end_r, sw.max_life);
            if self.seen_sw.contains(&sig) {
                continue;
            }
            self.seen_sw[self.seen_sw_head] = sig;
            self.seen_sw_head = (self.seen_sw_head + 1) % SEEN_SW_CAP;

            let is_nuke = sw.end_r >= 75.0;
            self.emit_explosion(sw.pos, sw.end_r, is_nuke, density);
        }
    }

    /// Continuous emitters: acid pool fumes, mower exhaust/steam, boss phase
    /// transitions, and exhaust.
    fn emit_continuous(&mut self, scene: &SmokeScene<'_>, vw: f32, vh: f32, density: f32) {
        let cam = scene.cam;
        let visible = |p: Vec2, m: f32| -> bool {
            let sx = p.x - cam.x;
            let sy = p.y - cam.y;
            sx + m >= 0.0 && sy + m >= 0.0 && sx - m < vw && sy - m < vh
        };

        // Acid pools: thin rising toxic fumes.
        for entity in scene.ecs.iter_entities() {
            if let Some(pool) = entity.get::<Pool>() {
                if !visible(pool.pos, pool.radius + 12.0) {
                    continue;
                }
                if self.rand_f32() < 0.38 * density {
                    let a = self.rand_range(0.0, std::f32::consts::TAU);
                    let d = self.rand_range(0.0, pool.radius * 0.75);
                    let life = self.rand_range(0.75, 1.30);
                    let seed = (self.next_u64() & 0xFFFF) as u16;
                    let vel = Vec2::new(self.rand_range(-3.0, 3.0), self.rand_range(-8.0, -3.0));
                    let r0 = self.rand_range(1.6, 2.4);
                    let r1 = self.rand_range(4.4, 6.2);
                    let puff = SmokePuff {
                        pos: pool.pos + Vec2::from_angle(a) * d,
                        vel,
                        r0,
                        r1,
                        life,
                        max_life: life,
                        alpha: 0.36,
                        color: (154, 234, 96),
                        kind: SmokeKind::AcidFume,
                        seed,
                        alive: true,
                    };
                    self.push(puff);
                }
            }

            // Boss phase change plumes + Mower exhaust / overheat steam / flipped smoke.
            if let Some(boss) = entity.get::<Boss>() {
                let phase_change_state =
                    matches!(boss.state, BossState::Broom(BroomBossState::PhaseChange));
                if ((self.prev_boss_phase > 0 && boss.phase > self.prev_boss_phase)
                    || (phase_change_state && !self.prev_boss_phase_change))
                    && visible(boss.pos, 72.0)
                {
                    self.emit_boss_phase_plume(boss.pos, scene.level.theme, density);
                }
                self.prev_boss_phase = boss.phase;
                self.prev_boss_phase_change = phase_change_state;

                if boss.kind == BossKind::Mower && visible(boss.pos, 48.0) {
                    let face = if boss.facing.len_sq() > 0.01 {
                        boss.facing.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    let rear = boss.pos - face * 12.5;
                    if let BossState::Mower(st) = boss.state {
                        match st {
                            MowerState::Overheat => {
                                // Thick white venting steam.
                                if self.rand_f32() < 0.90 * density {
                                    let life = self.rand_range(0.75, 1.25);
                                    let seed = (self.next_u64() & 0xFFFF) as u16;
                                    let puff_pos = boss.pos
                                        + Vec2::new(
                                            self.rand_range(-5.0, 5.0),
                                            self.rand_range(-4.0, 4.0),
                                        );
                                    let vel = Vec2::new(
                                        self.rand_range(-10.0, 10.0),
                                        self.rand_range(-18.0, -9.0),
                                    );
                                    let r0 = self.rand_range(2.4, 3.4);
                                    let r1 = self.rand_range(7.0, 10.5);
                                    let puff = SmokePuff {
                                        pos: puff_pos,
                                        vel,
                                        r0,
                                        r1,
                                        life,
                                        max_life: life,
                                        alpha: 0.28,
                                        color: (206, 218, 224),
                                        kind: SmokeKind::Steam,
                                        seed,
                                        alive: true,
                                    };
                                    self.push(puff);
                                }
                            }
                            MowerState::Flipped | MowerState::Dying => {
                                // Dark oil/motor smoke rising from the damaged deck.
                                if self.rand_f32() < 0.85 * density {
                                    let life = self.rand_range(0.85, 1.45);
                                    let seed = (self.next_u64() & 0xFFFF) as u16;
                                    let puff_pos = boss.pos
                                        + Vec2::new(
                                            self.rand_range(-4.0, 4.0),
                                            self.rand_range(-4.0, 4.0),
                                        );
                                    let vel = Vec2::new(
                                        self.rand_range(-7.0, 7.0),
                                        self.rand_range(-14.0, -7.0),
                                    );
                                    let r0 = self.rand_range(2.6, 3.8);
                                    let r1 = self.rand_range(8.0, 12.0);
                                    let puff = SmokePuff {
                                        pos: puff_pos,
                                        vel,
                                        r0,
                                        r1,
                                        life,
                                        max_life: life,
                                        alpha: 0.26,
                                        color: (94, 96, 104),
                                        kind: SmokeKind::Exhaust,
                                        seed,
                                        alive: true,
                                    };
                                    self.push(puff);
                                }
                            }
                            _ => {
                                // Normal chugging mower exhaust from the rear louver.
                                let chance = if boss.phase >= 2 { 0.82 } else { 0.58 };
                                if self.rand_f32() < chance * density {
                                    let life = self.rand_range(0.75, 1.20);
                                    let seed = (self.next_u64() & 0xFFFF) as u16;
                                    let puff_pos =
                                        rear + face.perp() * self.rand_range(-3.0, 3.0);
                                    let vel = -face * self.rand_range(12.0, 24.0)
                                        + Vec2::new(0.0, -4.5);
                                    let r0 = self.rand_range(2.2, 3.2);
                                    let r1 = self.rand_range(6.4, 9.6);
                                    let puff = SmokePuff {
                                        pos: puff_pos,
                                        vel,
                                        r0,
                                        r1,
                                        life,
                                        max_life: life,
                                        alpha: 0.28,
                                        color: (146, 156, 168),
                                        kind: SmokeKind::Exhaust,
                                        seed,
                                        alive: true,
                                    };
                                    self.push(puff);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Alpha-blend all active visible puffs onto `pb` using fast integer math.
    pub fn draw(&self, pb: &mut PixelBuffer, cam: Vec2) {
        if pb.w == 0 || pb.h == 0 {
            return;
        }
        let bw = pb.w as i32;
        let bh = pb.h as i32;
        let stride = pb.w;
        let pixels = pb.pixels_mut();

        for p in &self.puffs {
            if !p.alive {
                continue;
            }
            let alpha = p.current_alpha();
            if alpha <= 0.015 {
                continue;
            }
            let r = p.current_radius();
            if r < 0.8 {
                continue;
            }
            let sx = (p.pos.x - cam.x).round() as i32;
            let sy = (p.pos.y - cam.y).round() as i32;
            let ri = r.ceil() as i32;
            if sx + ri < 0 || sy + ri < 0 || sx - ri >= bw || sy - ri >= bh {
                continue;
            }

            // Organic cloud wobble: slight secondary lobe offset derived from `seed`.
            let ox = ((p.seed & 3) as i32 - 1).clamp(-1, 1);
            let oy = (((p.seed >> 2) & 3) as i32 - 1).clamp(-1, 1);
            let r2 = (r * r).max(1.0) as i32;
            let base_a = (alpha * 256.0) as u32;

            let x0 = (sx - ri).max(0);
            let x1 = (sx + ri).min(bw - 1);
            let y0 = (sy - ri).max(0);
            let y1 = (sy + ri).min(bh - 1);

            let cr = p.color.0 as u32;
            let cg = p.color.1 as u32;
            let cb = p.color.2 as u32;
            // Subtle top-left highlight & bottom-right shadow for plump cloud volume.
            let hi_r = (cr + ((255 - cr) >> 3)).min(255);
            let hi_g = (cg + ((255 - cg) >> 3)).min(255);
            let hi_b = (cb + ((255 - cb) >> 3)).min(255);
            let lo_r = (cr * 218) >> 8;
            let lo_g = (cg * 218) >> 8;
            let lo_b = (cb * 218) >> 8;

            for py in y0..=y1 {
                let dy = py - sy;
                let dy2 = dy * dy;
                let ldy = dy - oy;
                let ldy2 = ldy * ldy;
                let row = py as usize * stride;
                for px in x0..=x1 {
                    let dx = px - sx;
                    let d2_main = dx * dx + dy2;
                    let ldx = dx - ox;
                    let d2_lobe = ldx * ldx + ldy2;
                    // Blend primary circle with slightly shifted lobe for billowy silhouette.
                    let d2 = d2_main.min(d2_lobe + (r2 >> 3));
                    if d2 >= r2 {
                        continue;
                    }
                    // Smooth quadratic falloff: soft wispy edges, denser center.
                    let lin = (((r2 - d2) as u32) << 8) / (r2 as u32);
                    let fall = (lin * lin) >> 8;
                    let a = (base_a * fall) >> 8;
                    if a < 3 {
                        continue;
                    }
                    let (sr, sg, sb) = if dx + dy < -1 {
                        (hi_r, hi_g, hi_b)
                    } else if dx + dy > 1 {
                        (lo_r, lo_g, lo_b)
                    } else {
                        (cr, cg, cb)
                    };
                    let inv = 256 - a;
                    let bg = pixels[row + px as usize];
                    let nr = ((bg.0 as u32 * inv + sr * a) >> 8) as u8;
                    let ng = ((bg.1 as u32 * inv + sg * a) >> 8) as u8;
                    let nb = ((bg.2 as u32 * inv + sb * a) >> 8) as u8;
                    pixels[row + px as usize] = (nr, ng, nb);
                }
            }
        }
    }

    // ---- Private deterministic PRNG (never touches simulation RNG) ---------

    #[inline]
    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    #[inline]
    fn rand_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) * (1.0 / 16_777_216.0)
    }

    #[inline]
    fn rand_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand_f32()
    }
}

#[inline]
fn dust_color(theme: Theme) -> Rgb {
    #[allow(unreachable_patterns)]
    match theme {
        Theme::Sewer => (116, 136, 128),
        Theme::Kitchen => (176, 152, 124),
        Theme::Lab => (136, 148, 172),
        _ => (148, 150, 156),
    }
}

#[inline]
fn shockwave_sig(pos: Vec2, end_r: f32, max_life: f32) -> u64 {
    let px = (pos.x.round() as i32) as u64;
    let py = (pos.y.round() as i32) as u64;
    let er = (end_r.round() as u32) as u64;
    let ml = ((max_life * 100.0).round() as u32) as u64;
    let mut h = (px & 0xFFFF)
        | ((py & 0xFFFF) << 16)
        | ((er & 0xFFFF) << 32)
        | ((ml & 0xFFFF) << 48);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h.max(1)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::world::World;
    use std::time::Instant;

    #[test]
    fn ring_buffer_enforces_fixed_capacity_without_allocating() {
        let mut layer = SmokeLayer::new(1, Theme::Sewer);
        for i in 0..(SMOKE_CAP * 3) {
            layer.push(SmokePuff {
                pos: Vec2::new(i as f32, 0.0),
                vel: Vec2::ZERO,
                r0: 2.0,
                r1: 6.0,
                life: 1.0,
                max_life: 1.0,
                alpha: 0.4,
                color: (140, 140, 140),
                kind: SmokeKind::Blast,
                seed: i as u16,
                alive: true,
            });
        }
        assert_eq!(layer.active_count(), SMOKE_CAP);
    }

    #[test]
    fn explosion_puffs_expand_rise_and_fade_over_time() {
        let mut layer = SmokeLayer::new(42, Theme::Sewer);
        let origin = Vec2::new(100.0, 100.0);
        layer.emit_explosion(origin, 28.0, false, 1.0);
        assert!(layer.active_count() >= 5);

        let mean_r0: f32 = layer
            .active_puffs()
            .map(SmokePuff::current_radius)
            .sum::<f32>()
            / layer.active_count() as f32;
        let mean_y0: f32 = layer.active_puffs().map(|p| p.pos.y).sum::<f32>()
            / layer.active_count() as f32;

        // Advance 0.35s: puffs must expand and drift upward (-y).
        for _ in 0..21 {
            layer.step(1.0 / 60.0);
        }
        let alive_mid = layer.active_count();
        assert!(alive_mid > 0);
        let mean_r1: f32 = layer
            .active_puffs()
            .map(SmokePuff::current_radius)
            .sum::<f32>()
            / alive_mid as f32;
        let mean_y1: f32 = layer.active_puffs().map(|p| p.pos.y).sum::<f32>() / alive_mid as f32;
        assert!(mean_r1 > mean_r0 + 1.0, "smoke should expand: {mean_r0} -> {mean_r1}");
        assert!(mean_y1 < mean_y0, "smoke should rise: {mean_y0} -> {mean_y1}");

        // After 3 seconds all blast puffs have faded out.
        for _ in 0..180 {
            layer.step(1.0 / 60.0);
        }
        assert_eq!(layer.active_count(), 0);
    }

    #[test]
    fn smoke_draw_handles_tiny_and_empty_buffers() {
        let mut layer = SmokeLayer::new(9, Theme::Kitchen);
        layer.emit_explosion(Vec2::new(2.0, 2.0), 40.0, true, 1.0);
        layer.step(0.1);
        for (w, h) in [(0usize, 0usize), (1, 1), (5, 3), (3, 5)] {
            let mut pb = PixelBuffer::new(w, h);
            pb.clear((30, 30, 30));
            layer.draw(&mut pb, Vec2::ZERO);
        }
    }

    #[test]
    fn lighting_and_smoke_on_vs_off_produce_identical_world_state() {
        // Run two twin worlds with identical seeds and inputs: one draws every
        // frame with Full lighting + smoke, the other with both Off.
        let mut a = World::new(Theme::Sewer, 777);
        let mut b = World::new(Theme::Sewer, 777);
        a.lighting = LightingMode::Full;
        a.smoke_enabled = true;
        b.lighting = LightingMode::Off;
        b.smoke_enabled = false;

        let mut pb_a = PixelBuffer::new(120, 80);
        let mut pb_b = PixelBuffer::new(120, 80);

        for tick in 0..240u32 {
            let dir = match (tick / 30) % 4 {
                0 => Vec2::new(1.0, 0.0),
                1 => Vec2::new(0.0, 1.0),
                2 => Vec2::new(-1.0, 0.0),
                _ => Vec2::new(0.0, -1.0),
            };
            let dash = tick == 20 || tick == 140;
            a.update(1.0 / 60.0, dir, dash);
            b.update(1.0 / 60.0, dir, dash);
            a.draw(&mut pb_a);
            b.draw(&mut pb_b);
        }

        // Simulation state must be bit-for-bit identical.
        assert_eq!(a.elapsed, b.elapsed);
        assert_eq!(a.player.pos, b.player.pos);
        assert_eq!(a.player.vel, b.player.vel);
        assert_eq!(a.player.hp, b.player.hp);
        assert_eq!(a.player.xp, b.player.xp);
        assert_eq!(a.kills, b.kills);
        assert_eq!(a.score, b.score);
        assert_eq!(a.enemies().len(), b.enemies().len());
        assert_eq!(a.bullets().len(), b.bullets().len());
        assert_eq!(a.gems().len(), b.gems().len());
        assert_eq!(a.particles.len(), b.particles.len());
        for (ea, eb) in a.enemies().iter().zip(b.enemies().iter()) {
            assert_eq!(ea.id, eb.id);
            assert_eq!(ea.pos, eb.pos);
            assert_eq!(ea.hp, eb.hp);
        }
        // Rendered pixels differ because lighting + smoke actually ran on `a`.
        assert_ne!(pb_a.pixels(), pb_b.pixels());
    }

    #[test]
    fn combined_smoke_and_lighting_timings() {
        // Optional wall-clock measurement. The lighting suite measures CPU
        // time and checks the draw budget without scheduler contention.
        if std::env::var_os("RATRUN_SMOKE_TIMINGS").is_none() {
            return;
        }
        let mut world = World::new(Theme::Sewer, 123);
        world.lighting = LightingMode::Full;
        world.smoke_enabled = true;
        // Advance 2 seconds so enemies, bullets, acid, and ambient smoke are active.
        for tick in 0..120u32 {
            world.update(1.0 / 60.0, Vec2::new(0.6, 0.2), tick == 10);
        }
        // Trigger a nuke + dash for worst-case smoke + bright flash load.
        let p = world.player.pos;
        crate::game::particle::vfx::emit_nuke(&mut world.particles, &mut world.rng, p);

        for (label, w, h) in [
            ("120x40 cells (120x80 px)", 120usize, 80usize),
            ("160x84 cells (160x168 px)", 160, 168),
            ("2x zoom (320x336 px)", 320, 336),
        ] {
            let mut pb = PixelBuffer::new(w, h);
            // Warm up buffers.
            world.draw(&mut pb);

            world.lighting = LightingMode::Off;
            world.smoke_enabled = false;
            let iters = 25u32;
            let t0 = Instant::now();
            for _ in 0..iters {
                world.draw(&mut pb);
            }
            let base_us = t0.elapsed().as_secs_f64() * 1e6 / iters as f64;

            world.lighting = LightingMode::Full;
            world.smoke_enabled = true;
            let t1 = Instant::now();
            for _ in 0..iters {
                world.draw(&mut pb);
            }
            let full_us = t1.elapsed().as_secs_f64() * 1e6 / iters as f64;
            let delta_us = full_us - base_us;
            eprintln!(
                "[world draw bench] {label}: base={base_us:.1} us, full={full_us:.1} us, lighting+smoke delta={delta_us:.1} us"
            );
            assert!(full_us < 16_000.0, "{label} exceeded 60 FPS frame budget: {full_us:.1} us");
        }
    }
}
