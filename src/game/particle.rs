//! Particle system — the visual "juice".
//!
//! Owns three O(1)-eviction capped ring buffers (all pre-allocated up front,
//! zero per-frame heap allocation):
//! - `items`: multi-shape particles (dots, velocity streaks, sparkle crosses,
//!   cooling blobs, flickering embers).
//! - `shockwaves`: crisp expanding rings / directional slash arcs.
//! - `stains`: subtle alpha-blended floor decals (wet ichor splatters and
//!   explosion scorch marks) drawn onto the floor before props and entities.

#[path = "damage_number.rs"]
pub mod damage_number;
#[path = "vfx.rs"]
pub mod vfx;

use std::collections::VecDeque;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};
use rand::{Rng, RngExt};

/// Cap on simultaneous shockwave rings (O(1) eviction ring).
pub const SHOCKWAVE_CAP: usize = 48;
/// Cap on simultaneous floor stains (O(1) eviction ring).
pub const STAIN_CAP: usize = 220;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Crisp 1×1 pixel.
    Dot,
    /// Velocity-aligned 2..4 px streak with bright head and fading tail.
    Streak,
    /// 3×3 `+` sparkle with bright core that collapses to a dot as it ages.
    Cross,
    /// Chunky 2×2 cluster with top-left highlight that shrinks to 1×1 as it cools.
    Blob,
    /// Flickering ember dot that pulses gently as it drifts.
    Ember,
    /// Soft alpha-blended billowing dust, smoke, or steam cloud (3×3 -> 2×2 -> 1×1).
    Puff,
    /// Crisp velocity-aligned forward-pointing `>` chevron (Sonic Squeak cone).
    Chevron,
}

#[derive(Clone, Copy)]
pub enum Ramp {
    Fire,
    Spark,
    Smoke,
    Gem,
    /// Two-color custom fade (start → end), e.g. enemy-tinted blood.
    Custom(Rgb, Rgb),
    /// Toxic lime-chartreuse acid fade.
    Acid,
    /// Electric violet sonic nova fade.
    Nova,
    /// Bioluminescent mint spore fade.
    Spore,
    /// Rat-tail pink / rose-white melee slash fade.
    Tail,
    /// Warm level-up / big-gem gold fade.
    Gold,
    /// Pale ghost-cyan / coat-white dash wind fade.
    Dash,
    /// Emerald heal sparkle fade.
    Heal,
    /// Electric cyan-blue magnet pulse fade.
    Magnet,
    /// Wet ichor/blood droplets: rich core fading into dark stain tint.
    Blood(Rgb),
}

impl Ramp {
    /// Color at age fraction `t` (0 = freshly born, 1 = about to die).
    pub fn color_at(self, t: f32) -> Rgb {
        match self {
            Ramp::Fire => palette::ramp(
                &[
                    (0.0, (255, 255, 220)),
                    (0.22, (255, 205, 78)),
                    (0.55, (238, 88, 28)),
                    (1.0, (64, 18, 10)),
                ],
                t,
            ),
            Ramp::Spark => palette::ramp(
                &[
                    (0.0, (255, 255, 255)),
                    (0.35, (255, 236, 145)),
                    (1.0, (125, 82, 32)),
                ],
                t,
            ),
            Ramp::Smoke => palette::ramp(
                &[
                    (0.0, (130, 134, 144)),
                    (0.5, (72, 76, 84)),
                    (1.0, (22, 24, 28)),
                ],
                t,
            ),
            Ramp::Gem => palette::ramp(
                &[
                    (0.0, (235, 255, 255)),
                    (0.45, (105, 235, 255)),
                    (1.0, (32, 95, 168)),
                ],
                t,
            ),
            Ramp::Custom(a, b) => palette::scale(palette::mix(a, b, t), 1.0 - t * 0.6),
            Ramp::Acid => palette::ramp(
                &[
                    (0.0, (230, 255, 155)),
                    (0.35, (150, 242, 72)),
                    (0.75, (74, 158, 36)),
                    (1.0, (24, 58, 16)),
                ],
                t,
            ),
            Ramp::Nova => palette::ramp(
                &[
                    (0.0, (255, 248, 255)),
                    (0.35, (206, 155, 255)),
                    (1.0, (72, 36, 135)),
                ],
                t,
            ),
            Ramp::Spore => palette::ramp(
                &[
                    (0.0, (225, 255, 235)),
                    (0.45, (120, 245, 175)),
                    (1.0, (30, 96, 68)),
                ],
                t,
            ),
            Ramp::Tail => palette::ramp(
                &[
                    (0.0, (255, 240, 248)),
                    (0.35, (255, 142, 196)),
                    (1.0, (112, 34, 82)),
                ],
                t,
            ),
            Ramp::Gold => palette::ramp(
                &[
                    (0.0, (255, 255, 230)),
                    (0.35, (255, 224, 108)),
                    (0.75, (230, 148, 42)),
                    (1.0, (92, 46, 14)),
                ],
                t,
            ),
            Ramp::Dash => palette::ramp(
                &[
                    (0.0, (245, 248, 255)),
                    (0.40, (155, 212, 255)),
                    (1.0, (44, 72, 112)),
                ],
                t,
            ),
            Ramp::Heal => palette::ramp(
                &[
                    (0.0, (235, 255, 240)),
                    (0.40, (102, 255, 138)),
                    (1.0, (24, 98, 48)),
                ],
                t,
            ),
            Ramp::Magnet => palette::ramp(
                &[
                    (0.0, (235, 248, 255)),
                    (0.40, (118, 192, 255)),
                    (1.0, (32, 68, 148)),
                ],
                t,
            ),
            Ramp::Blood(c) => {
                let bright = palette::mix(c, (255, 255, 255), 0.18);
                let dark = palette::scale(c, 0.28);
                palette::scale(palette::mix(bright, dark, t), 1.0 - t * 0.45)
            }
        }
    }
}

pub struct Particle {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    pub max_life: f32,
    pub drag: f32,
    pub gravity: f32,
    pub ramp: Ramp,
    pub additive: bool,
    pub shape: Shape,
}

/// An expanding circular shockwave or directional slash arc.
#[derive(Clone, Copy)]
pub struct Shockwave {
    pub pos: Vec2,
    pub start_r: f32,
    pub end_r: f32,
    pub life: f32,
    pub max_life: f32,
    pub color: Rgb,
    pub core: Rgb,
    pub thick: f32,
    pub additive: bool,
    /// Optional directional arc center; `None` = full 360° ring.
    pub arc_dir: Option<Vec2>,
    pub arc_half_angle: f32,
}

/// Visual decal style for floor [`Stain`]s.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StainKind {
    /// Wet organic splatter (ichor, acid, honey glue) with top-left specular glint.
    #[default]
    Splat,
    /// Charred radial explosion scorch smudge.
    Scorch,
    /// Jagged radial floor fracture web with subtle top-left chipped-stone bevels.
    Crack,
    /// Concave wall/floor impact dent with shadowed top-left bowl and lit outer bevel.
    Dent,
}

/// A subtle alpha-blended floor decal (ichor splatter, scorch mark, floor cracks, or impact dent).
#[derive(Clone, Copy)]
pub struct Stain {
    pub pos: Vec2,
    pub radius: f32,
    pub color: Rgb,
    pub life: f32,
    pub max_life: f32,
    pub seed: u16,
    pub kind: StainKind,
}

#[derive(Default)]
#[derive(bevy_ecs::resource::Resource)]
pub struct Particles {
    /// Insertion order: front = oldest, back = newest.
    pub items: VecDeque<Particle>,
    cap: usize,
    pub shockwaves: VecDeque<Shockwave>,
    shockwave_cap: usize,
    pub stains: VecDeque<Stain>,
    stain_cap: usize,
}

impl Particles {
    pub fn new(cap: usize) -> Self {
        let shockwave_cap = if cap == 0 { 0 } else { SHOCKWAVE_CAP };
        let stain_cap = if cap == 0 { 0 } else { STAIN_CAP };
        Self {
            // Size the buffers up front: a Nuke on a full horde pushes the
            // whole cap in a single tick and must not reallocate.
            items: VecDeque::with_capacity(cap),
            cap,
            shockwaves: VecDeque::with_capacity(shockwave_cap),
            shockwave_cap,
            stains: VecDeque::with_capacity(stain_cap),
            stain_cap,
        }
    }

    pub fn push(&mut self, p: Particle) {
        // `Particles::default()` has no capacity to store anything in (and
        // `pop_front` on the empty deque below would be wrong either way).
        if self.cap == 0 {
            return;
        }
        if self.items.len() >= self.cap {
            // Oldest-first recycling keeps the framerate bounded, in O(1):
            // `remove(0)` here was O(cap) per push, i.e. quadratic once a Nuke
            // tops the buffer up.
            self.items.pop_front();
        }
        self.items.push_back(p);
    }

    pub fn push_shockwave(&mut self, sw: Shockwave) {
        if self.shockwave_cap == 0 {
            return;
        }
        if self.shockwaves.len() >= self.shockwave_cap {
            self.shockwaves.pop_front();
        }
        self.shockwaves.push_back(sw);
    }

    pub fn push_stain(&mut self, stain: Stain) {
        if self.stain_cap == 0 {
            return;
        }
        if self.stains.len() >= self.stain_cap {
            self.stains.pop_front();
        }
        self.stains.push_back(stain);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Graceful degradation factor under high particle load.
    /// Minor/ambient effects scale down as `items` fills past 50% of `cap`,
    /// preserving headroom so hero moments (nukes, boss deaths, level-ups)
    /// never get immediately evicted by background sparks.
    pub fn budget_count(&self, desired: usize, hero: bool) -> usize {
        if self.cap == 0 || desired == 0 {
            return 0;
        }
        let load = self.items.len() as f32 / self.cap as f32;
        if hero {
            if load > 0.90 {
                ((desired as f32) * 0.7).round().max(1.0) as usize
            } else {
                desired
            }
        } else if load > 0.80 {
            ((desired as f32) * 0.3).round().max(1.0) as usize
        } else if load > 0.50 {
            let factor = 1.0 - (load - 0.50) * 1.8;
            ((desired as f32) * factor.clamp(0.4, 1.0))
                .round()
                .max(1.0) as usize
        } else {
            desired
        }
    }

    /// Radial burst of `n` particles.
    #[allow(clippy::too_many_arguments)]
    pub fn burst(
        &mut self,
        rng: &mut impl Rng,
        pos: Vec2,
        n: usize,
        speed: f32,
        spread: f32,
        ramp: Ramp,
        life: f32,
        additive: bool,
    ) {
        self.burst_shaped(rng, pos, n, speed, spread, ramp, life, additive, Shape::Dot, 3.0);
    }

    /// Radial burst with explicit shape and drag.
    #[allow(clippy::too_many_arguments)]
    pub fn burst_shaped(
        &mut self,
        rng: &mut impl Rng,
        pos: Vec2,
        n: usize,
        speed: f32,
        spread: f32,
        ramp: Ramp,
        life: f32,
        additive: bool,
        shape: Shape,
        drag: f32,
    ) {
        for _ in 0..n {
            let a = rng.random_range(0.0..std::f32::consts::TAU);
            let s = speed * rng.random_range(0.3..1.0);
            let l = life * rng.random_range(0.6..1.2);
            let jitter = if spread > 0.0 {
                Vec2::new(
                    rng.random_range(-spread..spread),
                    rng.random_range(-spread..spread),
                )
            } else {
                Vec2::ZERO
            };
            self.push(Particle {
                pos: pos + jitter,
                vel: Vec2::from_angle(a) * s,
                life: l,
                max_life: l,
                drag,
                gravity: 0.0,
                ramp,
                additive,
                shape,
            });
        }
    }

    /// Directional cone (muzzle flash, dash trail).
    #[allow(clippy::too_many_arguments)]
    pub fn cone(
        &mut self,
        rng: &mut impl Rng,
        pos: Vec2,
        dir: Vec2,
        n: usize,
        speed: f32,
        ramp: Ramp,
        life: f32,
    ) {
        self.cone_shaped(rng, pos, dir, n, speed, 0.5, ramp, life, true, Shape::Dot, 5.0);
    }

    /// Directional cone with explicit spread angle, shape, blend mode, and drag.
    #[allow(clippy::too_many_arguments)]
    pub fn cone_shaped(
        &mut self,
        rng: &mut impl Rng,
        pos: Vec2,
        dir: Vec2,
        n: usize,
        speed: f32,
        half_angle: f32,
        ramp: Ramp,
        life: f32,
        additive: bool,
        shape: Shape,
        drag: f32,
    ) {
        let base = dir.angle();
        for _ in 0..n {
            let a = base + rng.random_range(-half_angle..half_angle);
            let s = speed * rng.random_range(0.4..1.1);
            let l = life * rng.random_range(0.55..1.05);
            self.push(Particle {
                pos,
                vel: Vec2::from_angle(a) * s,
                life: l,
                max_life: l,
                drag,
                gravity: 0.0,
                ramp,
                additive,
                shape,
            });
        }
    }

    pub fn update(&mut self, dt: f32) {
        for p in self.items.iter_mut() {
            p.life -= dt;
            let drag = (1.0 - p.drag * dt).max(0.0);
            p.vel = p.vel * drag;
            p.vel.y += p.gravity * dt;
            p.pos += p.vel * dt;
        }
        self.items.retain(|p| p.life > 0.0);

        for sw in self.shockwaves.iter_mut() {
            sw.life -= dt;
        }
        self.shockwaves.retain(|sw| sw.life > 0.0);

        for st in self.stains.iter_mut() {
            st.life -= dt;
        }
        self.stains.retain(|st| st.life > 0.0);
    }

    /// Draw subtle floor stains (ichor splatters and explosion scorch marks).
    /// Call after `art_floor::draw_floor` and before props/entities.
    pub fn draw_stains(&self, pb: &mut PixelBuffer, cam: Vec2) {
        if pb.w == 0 || pb.h == 0 || self.stains.is_empty() {
            return;
        }
        let bw = pb.w as i32;
        let bh = pb.h as i32;

        for st in &self.stains {
            if !st.pos.x.is_finite() || !st.pos.y.is_finite() || !st.radius.is_finite() || st.radius <= 0.2 {
                continue;
            }
            let sx = (st.pos.x - cam.x).round() as i32;
            let sy = (st.pos.y - cam.y).round() as i32;
            let reach = (st.radius * 2.2).ceil() as i32 + 2;
            if sx + reach < 0 || sy + reach < 0 || sx - reach >= bw || sy - reach >= bh {
                continue;
            }

            let age = 1.0 - (st.life / st.max_life.max(0.01)).clamp(0.0, 1.0);
            let fade = if age < 0.62 {
                1.0
            } else {
                (1.0 - (age - 0.62) / 0.38).clamp(0.0, 1.0)
            };
            if fade <= 0.02 {
                continue;
            }

            match st.kind {
                StainKind::Crack => {
                    draw_crack_stain(pb, bw, bh, sx, sy, st, fade);
                    continue;
                }
                StainKind::Dent => {
                    draw_dent_stain(pb, bw, bh, sx, sy, st, fade);
                    continue;
                }
                StainKind::Splat | StainKind::Scorch => {}
            }

            let scorch = st.kind == StainKind::Scorch;
            let base_alpha = if scorch { 0.54 * fade } else { 0.66 * fade };
            let r = st.radius;
            let ri = r.ceil() as i32;
            let r2 = r * r;

            // Core organic splatter: perturb boundary with deterministic hash of
            // angle/coords + stain seed so splatters look natural, never circular.
            for dy in -ri..=ri {
                let py = sy + dy;
                if py < 0 || py >= bh {
                    continue;
                }
                for dx in -ri..=ri {
                    let px = sx + dx;
                    if px < 0 || px >= bw {
                        continue;
                    }
                    let d2 = (dx * dx + dy * dy) as f32;
                    let h = stain_hash(st.seed, dx, dy);
                    let jitter = 0.68 + (h as f32 / 255.0) * 0.58;
                    if d2 <= r2 * jitter {
                        let fall = (1.0 - (d2 / (r2 * jitter).max(0.5)).sqrt()).clamp(0.25, 1.0);
                        let local_alpha = base_alpha * (0.55 + 0.45 * fall);
                        let bg = pb.pixel_at(px as usize, py as usize);
                        let tint = if scorch {
                            palette::scale(st.color, 0.5 + 0.5 * (1.0 - fall))
                        } else {
                            // Slightly darker core, wet rim.
                            palette::scale(st.color, 0.72 + 0.28 * fall)
                        };
                        pb.plot(px, py, palette::mix(bg, tint, local_alpha));
                    }
                }
            }

            // 2..3 deterministic satellite droplets outside the core splatter.
            let sat_count = if r >= 4.0 { 3 } else { 2 };
            for k in 0..sat_count {
                let h1 = stain_hash(st.seed.wrapping_add(k as u16 * 37), k + 3, 11);
                let h2 = stain_hash(st.seed.wrapping_add(k as u16 * 71), 17, k + 5);
                let ang = (h1 as f32 / 255.0) * std::f32::consts::TAU;
                let dist = r * (1.15 + (h2 as f32 / 255.0) * 0.85);
                let px = sx + (ang.cos() * dist).round() as i32;
                let py = sy + (ang.sin() * dist).round() as i32;
                if px >= 0 && px < bw && py >= 0 && py < bh {
                    let bg = pb.pixel_at(px as usize, py as usize);
                    pb.plot(px, py, palette::mix(bg, st.color, base_alpha * 0.75));
                }
            }

            // Fresh wet top-left specular glint (matches the game's top-left lighting).
            if !scorch && age < 0.45 && r >= 2.0 {
                let gx = sx - (r * 0.30).round() as i32;
                let gy = sy - (r * 0.30).round() as i32;
                if gx >= 0 && gx < bw && gy >= 0 && gy < bh {
                    let wet = (1.0 - age / 0.45) * 0.32;
                    let glint = palette::mix(st.color, (255, 255, 255), 0.55);
                    let bg = pb.pixel_at(gx as usize, gy as usize);
                    pb.plot(gx, gy, palette::mix(bg, glint, wet));
                }
            }
        }
    }

    pub fn draw(&self, pb: &mut PixelBuffer, cam: Vec2) {
        if pb.w == 0 || pb.h == 0 {
            return;
        }
        let bw = pb.w as i32;
        let bh = pb.h as i32;

        // Expanding shockwaves sit underneath the brightest foreground sparks.
        for sw in &self.shockwaves {
            draw_shockwave(pb, cam, bw, bh, sw);
        }

        // Particles.
        for p in &self.items {
            let x = (p.pos.x - cam.x).round() as i32;
            let y = (p.pos.y - cam.y).round() as i32;
            if x < -6 || y < -6 || x >= bw + 6 || y >= bh + 6 {
                continue;
            }
            let t = 1.0 - (p.life / p.max_life.max(1e-4)).clamp(0.0, 1.0);
            let mut c = p.ramp.color_at(t);

            match p.shape {
                Shape::Dot => {
                    plot_mode(pb, x, y, c, p.additive);
                }
                Shape::Streak => {
                    plot_mode(pb, x, y, c, p.additive);
                    let sp = p.vel.len();
                    if sp > 14.0 {
                        let back = p.vel * (-1.0 / sp);
                        let steps = if sp > 75.0 { 3 } else if sp > 38.0 { 2 } else { 1 };
                        let mut prev = (x, y);
                        for s in 1..=steps {
                            let tx = (p.pos.x + back.x * s as f32 - cam.x).round() as i32;
                            let ty = (p.pos.y + back.y * s as f32 - cam.y).round() as i32;
                            if (tx, ty) != prev {
                                let atten = (1.0 - s as f32 / (steps as f32 + 1.0)).powi(2);
                                plot_mode(pb, tx, ty, palette::scale(c, atten * 0.85), p.additive);
                                prev = (tx, ty);
                            }
                        }
                    }
                }
                Shape::Cross => {
                    let core = palette::mix(c, (255, 255, 255), (1.0 - t) * 0.65);
                    plot_mode(pb, x, y, core, p.additive);
                    if t < 0.62 {
                        let arm = palette::scale(c, 0.72 * (1.0 - t * 0.5));
                        plot_mode(pb, x - 1, y, arm, p.additive);
                        plot_mode(pb, x + 1, y, arm, p.additive);
                        plot_mode(pb, x, y - 1, arm, p.additive);
                        plot_mode(pb, x, y + 1, arm, p.additive);
                    }
                }
                Shape::Blob => {
                    if t < 0.55 {
                        // Top-left pixel slightly brighter for cohesive pixel lighting.
                        let hi = palette::mix(c, (255, 255, 255), 0.2);
                        let lo = palette::scale(c, 0.76);
                        plot_mode(pb, x, y, hi, p.additive);
                        plot_mode(pb, x + 1, y, c, p.additive);
                        plot_mode(pb, x, y + 1, c, p.additive);
                        plot_mode(pb, x + 1, y + 1, lo, p.additive);
                    } else {
                        plot_mode(pb, x, y, c, p.additive);
                    }
                }
                Shape::Ember => {
                    // Subtle phase flicker tied deterministically to remaining life + pos.
                    let phase = (p.life * 26.0 + p.pos.x * 0.37 + p.pos.y * 0.23).sin();
                    let flicker = 0.62 + 0.38 * (phase * 0.5 + 0.5);
                    c = palette::scale(c, flicker);
                    plot_mode(pb, x, y, c, p.additive);
                }
                Shape::Puff => {
                    // Soft alpha-blended billowing dust/smoke/steam cloud with top-left cel highlight.
                    let alpha = (1.0 - t).powf(1.20) * 0.88;
                    if alpha > 0.02 {
                        let hi = palette::mix(c, (255, 255, 255), 0.26);
                        let lo = palette::scale(c, 0.70);
                        if t < 0.40 {
                            // Plump 5×5 rounded cloud puff (center + cardinal + soft corners).
                            plot_alpha(pb, x, y, c, alpha, p.additive);
                            plot_alpha(pb, x - 1, y, hi, alpha * 0.88, p.additive);
                            plot_alpha(pb, x, y - 1, hi, alpha * 0.88, p.additive);
                            plot_alpha(pb, x - 1, y - 1, hi, alpha * 0.68, p.additive);
                            plot_alpha(pb, x + 1, y, lo, alpha * 0.80, p.additive);
                            plot_alpha(pb, x, y + 1, lo, alpha * 0.80, p.additive);
                            plot_alpha(pb, x + 1, y + 1, lo, alpha * 0.48, p.additive);
                            plot_alpha(pb, x - 1, y + 1, c, alpha * 0.56, p.additive);
                            plot_alpha(pb, x + 1, y - 1, c, alpha * 0.56, p.additive);
                            // Soft outer wisps on cardinal tips.
                            plot_alpha(pb, x - 2, y, hi, alpha * 0.34, p.additive);
                            plot_alpha(pb, x, y - 2, hi, alpha * 0.34, p.additive);
                            plot_alpha(pb, x + 2, y, lo, alpha * 0.28, p.additive);
                            plot_alpha(pb, x, y + 2, lo, alpha * 0.28, p.additive);
                        } else if t < 0.72 {
                            // Soft 3×3 thinning puff.
                            plot_alpha(pb, x, y, c, alpha * 0.88, p.additive);
                            plot_alpha(pb, x - 1, y, hi, alpha * 0.66, p.additive);
                            plot_alpha(pb, x, y - 1, hi, alpha * 0.66, p.additive);
                            plot_alpha(pb, x + 1, y, lo, alpha * 0.56, p.additive);
                            plot_alpha(pb, x, y + 1, lo, alpha * 0.56, p.additive);
                        } else {
                            // Fading wisp.
                            plot_alpha(pb, x, y, c, alpha * 0.70, p.additive);
                            plot_alpha(pb, x - 1, y, hi, alpha * 0.35, p.additive);
                        }
                    }
                }
                Shape::Chevron => {
                    // Crisp velocity-aligned `>` wedge pointing along travel direction.
                    let dir = if p.vel.len_sq() > 1.0 {
                        p.vel.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    let perp = dir.perp();
                    let tip_col = palette::mix(c, (255, 255, 255), (1.0 - t) * 0.55);
                    plot_mode(pb, x, y, tip_col, p.additive);
                    for side in [-1.0f32, 1.0] {
                        let w1x = (p.pos.x - dir.x * 1.1 + perp.x * side * 1.2 - cam.x).round() as i32;
                        let w1y = (p.pos.y - dir.y * 1.1 + perp.y * side * 1.2 - cam.y).round() as i32;
                        plot_mode(pb, w1x, w1y, c, p.additive);
                        if t < 0.62 {
                            let w2x = (p.pos.x - dir.x * 2.1 + perp.x * side * 2.2 - cam.x).round() as i32;
                            let w2y = (p.pos.y - dir.y * 2.1 + perp.y * side * 2.2 - cam.y).round() as i32;
                            plot_mode(pb, w2x, w2y, palette::scale(c, 0.70), p.additive);
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn plot_mode(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, additive: bool) {
    if additive {
        pb.plot_add(x, y, c);
    } else {
        pb.plot(x, y, c);
    }
}

#[inline]
fn plot_alpha(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32, additive: bool) {
    if a <= 0.01 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if additive {
        pb.plot_add(x, y, palette::scale(c, a));
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, palette::mix(bg, c, a.clamp(0.0, 1.0)));
    }
}

/// Jagged radial floor fracture lines with subtle chipped-stone top-left bevels.
fn draw_crack_stain(pb: &mut PixelBuffer, bw: i32, bh: i32, sx: i32, sy: i32, st: &Stain, fade: f32) {
    let r = st.radius.clamp(2.0, 24.0);
    let crack_col = palette::scale(st.color, 0.28);
    let chip_hi = palette::mix(st.color, (228, 222, 206), 0.68);

    // Subtle darkened central impact dimple.
    let cr = (r * 0.30).clamp(1.0, 4.5);
    let cri = cr.ceil() as i32;
    let cr2 = cr * cr;
    for dy in -cri..=cri {
        let py = sy + dy;
        if py < 0 || py >= bh {
            continue;
        }
        for dx in -cri..=cri {
            let px = sx + dx;
            if px < 0 || px >= bw {
                continue;
            }
            let d2 = (dx * dx + dy * dy) as f32;
            if d2 <= cr2 {
                let fall = 1.0 - (d2 / cr2.max(0.5)).sqrt();
                let bg = pb.pixel_at(px as usize, py as usize);
                pb.plot(px, py, palette::mix(bg, crack_col, (0.36 + 0.44 * fall) * fade));
            }
        }
    }

    // 5..7 deterministic jagged radial fracture rays + secondary fork branches.
    let rays: i32 = if r >= 8.0 { 7 } else if r >= 5.0 { 6 } else { 5 };
    for k in 0..rays {
        let jitter_a = (stain_hash(st.seed, k * 13 + 5, 7) as f32 / 255.0 - 0.5) * 0.48;
        let base_a = (k as f32 / rays as f32) * std::f32::consts::TAU + jitter_a;
        let ray_len = r * (0.82 + (stain_hash(st.seed, k * 19 + 3, 29) as f32 / 255.0) * 0.36);
        let steps = (ray_len * 1.35).ceil().clamp(3.0, 26.0) as i32;
        let mut ang = base_a;
        let mut cx = sx as f32;
        let mut cy = sy as f32;

        for s in 1..=steps {
            let frac = s as f32 / steps as f32;
            // Periodic zigzag kink for natural masonry/concrete cracks.
            if s % 3 == 0 {
                let kink = (stain_hash(st.seed.wrapping_add(k as u16 * 41), s, k) as f32 / 255.0
                    - 0.5)
                    * 0.62;
                ang += kink;
            }
            let step_len = ray_len / steps as f32;
            cx += ang.cos() * step_len;
            cy += ang.sin() * step_len;
            let px = cx.round() as i32;
            let py = cy.round() as i32;
            let w_alpha = (1.0 - frac * 0.48) * 0.90 * fade;
            // Pale chipped-stone top-left bevel lip beside the dark fracture line so
            // cracks read crisply even on dark Sewer/Lab floor tiles.
            if frac < 0.84 {
                plot_alpha(pb, px - 1, py - 1, chip_hi, w_alpha * 0.46, false);
            }
            plot_alpha(pb, px, py, crack_col, w_alpha, false);

            // Secondary fork halfway down alternating cracks.
            if k % 2 == 0 && s == steps / 2 && r >= 5.5 {
                let fork_dir = if k % 4 == 0 { 0.58 } else { -0.58 };
                let fa = ang + fork_dir;
                let mut fx = cx;
                let mut fy = cy;
                let fsteps = (steps / 2).clamp(2, 8);
                for fs in 1..=fsteps {
                    fx += fa.cos() * step_len * 0.92;
                    fy += fa.sin() * step_len * 0.92;
                    let fpx = fx.round() as i32;
                    let fpy = fy.round() as i32;
                    let falpha = (1.0 - fs as f32 / (fsteps as f32 + 1.0)) * 0.74 * fade;
                    plot_alpha(pb, fpx - 1, fpy - 1, chip_hi, falpha * 0.36, false);
                    plot_alpha(pb, fpx, fpy, crack_col, falpha, false);
                }
            }
        }
    }
}

/// Concave floor/wall-edge impact dent with shadowed top-left inner wall and lit bottom-right rim.
fn draw_dent_stain(pb: &mut PixelBuffer, bw: i32, bh: i32, sx: i32, sy: i32, st: &Stain, fade: f32) {
    let r = st.radius.clamp(2.0, 14.0);
    let ri = (r + 1.5).ceil() as i32;
    let r2 = r * r;
    let deep_col = palette::scale(st.color, 0.25);
    let rim_hi = palette::mix(st.color, (234, 232, 224), 0.74);

    for dy in -ri..=ri {
        let py = sy + dy;
        if py < 0 || py >= bh {
            continue;
        }
        for dx in -ri..=ri {
            let px = sx + dx;
            if px < 0 || px >= bw {
                continue;
            }
            let d2 = (dx * dx + dy * dy) as f32;
            let h = stain_hash(st.seed, dx * 2, dy * 2);
            let jitter = 0.80 + (h as f32 / 255.0) * 0.34;
            let eff_r2 = r2 * jitter;
            if d2 <= eff_r2 {
                let norm_d = (d2 / eff_r2.max(0.5)).sqrt();
                // Top-left light = (-0.55, -0.83). Inside a concave bowl, the top-left
                // inner slope is in deep shadow and the bottom-right slope catches light!
                let light_dot = if d2 > 0.25 {
                    let inv = 1.0 / d2.sqrt();
                    (dx as f32 * inv) * (-0.55) + (dy as f32 * inv) * (-0.83)
                } else {
                    0.0
                };
                let bg = pb.pixel_at(px as usize, py as usize);
                if norm_d > 0.58 && light_dot < -0.14 {
                    // Lit bottom-right concave bevel rim!
                    let a = ((-light_dot - 0.14) * 0.86).clamp(0.16, 0.72) * fade;
                    pb.plot(px, py, palette::mix(bg, rim_hi, a));
                } else {
                    // Dark recessed dent shadow (deepest on top-left inner wall).
                    let bowl = (0.62 + 0.38 * light_dot.max(0.0)) * (1.0 - norm_d * 0.28);
                    let col = palette::mix(st.color, deep_col, 0.75);
                    pb.plot(px, py, palette::mix(bg, col, (0.82 * bowl).clamp(0.22, 0.88) * fade));
                }
            }
        }
    }

    // 4 short radial stress ticks around the dent rim.
    for k in 0..4 {
        let a = (stain_hash(st.seed, k * 23 + 7, 13) as f32 / 255.0) * std::f32::consts::TAU;
        let (sa, ca) = a.sin_cos();
        for step in 0..3 {
            let dist = r * 0.90 + step as f32 * 0.95;
            let px = sx + (ca * dist).round() as i32;
            let py = sy + (sa * dist).round() as i32;
            plot_alpha(pb, px - 1, py - 1, rim_hi, (0.32 - step as f32 * 0.08) * fade, false);
            plot_alpha(pb, px, py, deep_col, (0.72 - step as f32 * 0.18) * fade, false);
        }
    }
}

#[inline]
fn stain_hash(seed: u16, dx: i32, dy: i32) -> u8 {
    let mut x = (seed as u32)
        ^ (dx.wrapping_mul(374761393) as u32)
        ^ (dy.wrapping_mul(668265263) as u32);
    x = (x ^ (x >> 13)).wrapping_mul(1274126177);
    (x ^ (x >> 16)) as u8
}

fn draw_shockwave(pb: &mut PixelBuffer, cam: Vec2, bw: i32, bh: i32, sw: &Shockwave) {
    let t = 1.0 - (sw.life / sw.max_life.max(1e-4)).clamp(0.0, 1.0);
    // Ease-out cubic so shockwaves snap outward with punch and decelerate smoothly.
    let ease = 1.0 - (1.0 - t).powi(3);
    let r = sw.start_r + (sw.end_r - sw.start_r) * ease;
    if r < 0.8 {
        return;
    }

    let cx = (sw.pos.x - cam.x).round() as i32;
    let cy = (sw.pos.y - cam.y).round() as i32;
    let reach = r.ceil() as i32 + 3;
    if cx + reach < 0 || cy + reach < 0 || cx - reach >= bw || cy - reach >= bh {
        return;
    }

    let fade = (1.0 - t).powf(1.45);
    if fade <= 0.02 {
        return;
    }

    let base_col = palette::mix(sw.core, sw.color, (t * 2.2).min(1.0));
    let ring_col = palette::scale(base_col, fade);
    let wake_col = palette::scale(sw.color, fade * 0.42);
    let layers = if sw.thick >= 1.8 { 2 } else { 1 };

    let (start_a, span_a) = if let Some(dir) = sw.arc_dir {
        let mid = dir.angle();
        (mid - sw.arc_half_angle, sw.arc_half_angle * 2.0)
    } else {
        (0.0, std::f32::consts::TAU)
    };

    // Sample enough points around the arc/circle to avoid gaps, capped for safety.
    let arc_len = r * span_a;
    let steps = (arc_len * 1.45).ceil().clamp(12.0, 480.0) as i32;
    let mut last_px = (i32::MIN, i32::MIN);

    for i in 0..=steps {
        let frac = i as f32 / steps as f32;
        let a = start_a + frac * span_a;
        let (sa, ca) = a.sin_cos();
        let px = cx + (ca * r).round() as i32;
        let py = cy + (sa * r).round() as i32;
        if (px, py) == last_px {
            continue;
        }
        last_px = (px, py);
        if px >= 0 && px < bw && py >= 0 && py < bh {
            // Taper directional arc ends softly.
            let end_atten = if sw.arc_dir.is_some() {
                let edge = (frac * 2.0).min((1.0 - frac) * 2.0).clamp(0.0, 1.0);
                0.35 + 0.65 * edge
            } else {
                1.0
            };
            if sw.additive {
                pb.plot_add(px, py, palette::scale(ring_col, end_atten));
            } else {
                plot_alpha(pb, px, py, base_col, fade * end_atten * 0.85, false);
            }
        }
        if layers == 2 && r > 2.5 {
            let ix = cx + (ca * (r - 1.25)).round() as i32;
            let iy = cy + (sa * (r - 1.25)).round() as i32;
            if (ix, iy) != (px, py) && ix >= 0 && ix < bw && iy >= 0 && iy < bh {
                if sw.additive {
                    pb.plot_add(ix, iy, wake_col);
                } else {
                    plot_alpha(pb, ix, iy, sw.color, fade * 0.36, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn cap_is_respected() {
        let mut ps = Particles::new(50);
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..20 {
            ps.burst(&mut rng, Vec2::ZERO, 20, 40.0, 1.0, Ramp::Spark, 1.0, true);
        }
        assert!(ps.len() <= 50);
    }

    fn particle(life: f32) -> Particle {
        Particle {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            life,
            max_life: life,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Spark,
            additive: true,
            shape: Shape::Dot,
        }
    }

    #[test]
    fn oldest_particles_are_evicted_first() {
        let mut ps = Particles::new(3);
        for i in 0..6 {
            ps.push(particle(1.0 + i as f32));
        }
        assert_eq!(ps.len(), 3);
        let lives: Vec<f32> = ps.items.iter().map(|p| p.life).collect();
        assert_eq!(lives, vec![4.0, 5.0, 6.0], "only the newest survive");
    }

    #[test]
    fn zero_cap_drops_pushes_instead_of_panicking() {
        let mut ps = Particles::default();
        assert_eq!(ps.len(), 0);
        ps.push(particle(1.0));
        ps.push_shockwave(Shockwave {
            pos: Vec2::ZERO,
            start_r: 1.0,
            end_r: 20.0,
            life: 0.5,
            max_life: 0.5,
            color: (255, 255, 255),
            core: (255, 255, 255),
            thick: 1.0,
            additive: true,
            arc_dir: None,
            arc_half_angle: 0.0,
        });
        ps.push_stain(Stain {
            pos: Vec2::ZERO,
            radius: 3.0,
            color: (100, 20, 20),
            life: 5.0,
            max_life: 5.0,
            seed: 1,
            kind: StainKind::Splat,
        });
        assert_eq!(ps.len(), 0);
        assert!(ps.shockwaves.is_empty());
        assert!(ps.stains.is_empty());
        assert_eq!(ps.cap, 0);
    }

    #[test]
    fn cap_is_allocated_up_front() {
        let ps = Particles::new(500);
        assert!(ps.items.capacity() >= 500);
        assert!(ps.shockwaves.capacity() >= SHOCKWAVE_CAP);
        assert!(ps.stains.capacity() >= STAIN_CAP);
    }

    #[test]
    fn particles_expire() {
        let mut ps = Particles::new(100);
        let mut rng = StdRng::seed_from_u64(2);
        ps.burst(&mut rng, Vec2::ZERO, 10, 10.0, 0.0, Ramp::Fire, 0.5, false);
        assert_eq!(ps.len(), 10);
        ps.update(1.0); // exceeds max life
        assert_eq!(ps.len(), 0);
    }

    #[test]
    fn budget_count_throttles_under_load() {
        let mut ps = Particles::new(100);
        assert_eq!(ps.budget_count(20, false), 20);
        for _ in 0..85 {
            ps.push(particle(1.0));
        }
        assert!(
            ps.budget_count(20, false) < 10,
            "minor particles should throttle when buffer is >80% full"
        );
        assert_eq!(
            ps.budget_count(20, true),
            20,
            "hero bursts stay full until >90% load"
        );
    }

    #[test]
    fn shockwaves_and_stains_respect_caps_and_render() {
        let mut ps = Particles::new(200);
        for i in 0..(SHOCKWAVE_CAP + 20) {
            ps.push_shockwave(Shockwave {
                pos: Vec2::new(30.0, 30.0),
                start_r: 2.0,
                end_r: 18.0,
                life: 0.4,
                max_life: 0.4,
                color: (255, 160, 80),
                core: (255, 255, 220),
                thick: 2.0,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            ps.push_stain(Stain {
                pos: Vec2::new(20.0 + (i % 5) as f32, 20.0),
                radius: 3.5,
                color: (120, 200, 90),
                life: 6.0,
                max_life: 6.0,
                seed: i as u16,
                kind: StainKind::Splat,
            });
        }
        assert_eq!(ps.shockwaves.len(), SHOCKWAVE_CAP);
        assert!(ps.stains.len() <= STAIN_CAP);

        let mut pb = PixelBuffer::new(64, 64);
        pb.clear((16, 22, 20));
        ps.draw_stains(&mut pb, Vec2::ZERO);
        let stained = (0..pb.h)
            .flat_map(|y| (0..pb.w).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != (16, 22, 20))
            .count();
        assert!(stained > 10, "stains should tint floor pixels");

        ps.update(0.1);
        ps.draw(&mut pb, Vec2::ZERO);

        // Zero-size buffer must never panic.
        let mut zero = PixelBuffer::new(0, 0);
        ps.draw_stains(&mut zero, Vec2::ZERO);
        ps.draw(&mut zero, Vec2::ZERO);
    }

    #[test]
    fn puff_shape_and_crack_dent_stains_render_cleanly() {
        let mut ps = Particles::new(64);
        // Young, mid-life, and fading Puff particles (both alpha-blended and additive).
        for (i, life) in [0.95, 0.40, 0.10].iter().enumerate() {
            ps.push(Particle {
                pos: Vec2::new(12.0 + i as f32 * 14.0, 20.0),
                vel: Vec2::new(6.0, -4.0),
                life: *life,
                max_life: 1.0,
                drag: 2.0,
                gravity: 0.0,
                ramp: Ramp::Custom((210, 204, 190), (110, 104, 96)),
                additive: i == 2,
                shape: Shape::Puff,
            });
        }
        // Non-additive translucent dust shockwave.
        ps.push_shockwave(Shockwave {
            pos: Vec2::new(32.0, 32.0),
            start_r: 3.0,
            end_r: 18.0,
            life: 0.25,
            max_life: 0.40,
            color: (204, 196, 180),
            core: (238, 232, 218),
            thick: 2.0,
            additive: false,
            arc_dir: None,
            arc_half_angle: 0.0,
        });
        // Crack and Dent floor stains.
        ps.push_stain(Stain {
            pos: Vec2::new(20.0, 44.0),
            radius: 10.0,
            color: (48, 44, 40),
            life: 10.0,
            max_life: 10.0,
            seed: 42,
            kind: StainKind::Crack,
        });
        ps.push_stain(Stain {
            pos: Vec2::new(46.0, 44.0),
            radius: 6.0,
            color: (44, 48, 54),
            life: 10.0,
            max_life: 10.0,
            seed: 99,
            kind: StainKind::Dent,
        });

        let mut pb = PixelBuffer::new(64, 64);
        let bg = (28, 32, 30);
        pb.clear(bg);
        ps.draw_stains(&mut pb, Vec2::ZERO);
        let stained = (0..pb.h)
            .flat_map(|y| (0..pb.w).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != bg)
            .count();
        assert!(stained > 25, "crack and dent stains should paint visible floor pixels");

        ps.draw(&mut pb, Vec2::ZERO);
        let drawn = (0..pb.h)
            .flat_map(|y| (0..pb.w).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != bg)
            .count();
        assert!(drawn > stained + 15, "puffs and non-additive shockwave should paint over floor");

        // Zero-size buffer must never panic.
        let mut zero = PixelBuffer::new(0, 0);
        ps.draw_stains(&mut zero, Vec2::ZERO);
        ps.draw(&mut zero, Vec2::ZERO);
    }

    #[test]
    fn chevron_shape_renders_directional_wedge() {
        let mut ps = Particles::new(16);
        ps.push(Particle {
            pos: Vec2::new(20.0, 20.0),
            vel: Vec2::new(80.0, 0.0),
            life: 0.25,
            max_life: 0.30,
            drag: 2.0,
            gravity: 0.0,
            ramp: Ramp::Custom((255, 255, 255), (180, 224, 255)),
            additive: true,
            shape: Shape::Chevron,
        });
        ps.push(Particle {
            pos: Vec2::new(36.0, 20.0),
            vel: Vec2::ZERO,
            life: 0.10,
            max_life: 0.30,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Custom((255, 255, 255), (180, 224, 255)),
            additive: false,
            shape: Shape::Chevron,
        });
        let mut pb = PixelBuffer::new(48, 40);
        let bg = (16, 20, 22);
        pb.clear(bg);
        ps.draw(&mut pb, Vec2::ZERO);
        let lit = (0..pb.h)
            .flat_map(|y| (0..pb.w).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != bg)
            .count();
        assert!(lit >= 6, "chevron particles should paint tip + wings, got {lit}");
        let mut zero = PixelBuffer::new(0, 0);
        ps.draw(&mut zero, Vec2::ZERO);
    }
}
