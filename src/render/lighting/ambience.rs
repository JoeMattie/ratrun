//! Analytic particles. Reconstruct a bounded window of births from the
//! emitter seed instead of advancing a particle pool or consuming game RNG.

use super::{Flicker, Light, LightMap, ShadowSoftness};
use crate::game::level::{AmbientEmitter, EmitterKind};
use crate::math::Vec2;
use crate::render::{framebuffer::PixelBuffer, palette::Rgb};

/// Maximum total opacity from one local column, including overlapping puffs.
pub(super) const MAX_SMOKE_OPACITY: f32 = 0.24;
const MAX_BIRTHS: usize = 12;

#[derive(Clone, Copy)]
enum Motion {
    Rise,
    Jet,
    Drip,
    Spark,
    Drift,
    Orbit,
}

#[derive(Clone, Copy)]
struct Recipe {
    core: Rgb,
    vapour: Rgb,
    rate: f32,
    life: f32,
    size: f32,
    travel: Vec2,
    radius: f32,
    energy: f32,
    motion: Motion,
}

fn recipe(kind: EmitterKind) -> Recipe {
    use EmitterKind::*;
    use Motion::{Drift, Drip as Fall, Jet, Orbit, Rise, Spark};
    let (core, vapour, rate, life, size, travel, radius, energy, motion) = match kind {
        Fire => (
            (255, 157, 67),
            (106, 88, 93),
            4.4,
            2.60,
            7.4,
            (8.0, -29.0),
            109.0,
            0.92,
            Rise,
        ),
        Steam => (
            (137, 166, 181),
            (187, 213, 226),
            6.3,
            1.35,
            7.5,
            (28.0, -13.0),
            28.0,
            0.10,
            Jet,
        ),
        Drip => (
            (145, 221, 244),
            (91, 157, 188),
            1.7,
            1.40,
            6.0,
            (0.0, 15.0),
            28.0,
            0.14,
            Fall,
        ),
        Cable => (
            (151, 208, 255),
            (89, 143, 216),
            13.0,
            0.72,
            7.0,
            (13.0, 5.0),
            57.0,
            0.62,
            Spark,
        ),
        Dust => (
            (186, 168, 133),
            (118, 120, 128),
            4.8,
            2.40,
            10.0,
            (17.0, -9.0),
            38.0,
            0.10,
            Drift,
        ),
        Fog => (
            (135, 143, 172),
            (141, 154, 180),
            3.0,
            2.75,
            10.0,
            (25.0, -4.0),
            33.0,
            0.13,
            Jet,
        ),
        Exhaust => (
            (194, 127, 84),
            (140, 122, 118),
            5.2,
            1.65,
            7.8,
            (-24.0, -10.0),
            33.0,
            0.15,
            Jet,
        ),
        Pilot => (
            (246, 159, 69),
            (122, 78, 59),
            3.0,
            0.80,
            3.0,
            (4.0, -7.0),
            41.0,
            0.38,
            Rise,
        ),
        Fumes => (
            (168, 190, 109),
            (129, 153, 121),
            4.0,
            2.20,
            8.0,
            (10.0, -18.0),
            37.0,
            0.16,
            Jet,
        ),
        Insects => (
            (227, 208, 135),
            (141, 121, 95),
            2.8,
            2.60,
            8.0,
            (14.0, -9.0),
            39.0,
            0.20,
            Orbit,
        ),
    };
    Recipe {
        core,
        vapour,
        rate,
        life,
        size,
        travel: Vec2::new(travel.0, travel.1),
        radius,
        energy,
        motion,
    }
}

fn flicker(emit: AmbientEmitter) -> Flicker {
    Flicker {
        amount: if emit.kind == EmitterKind::Fire {
            0.40
        } else {
            0.27
        },
        speed: if emit.kind == EmitterKind::Fire {
            8.9
        } else {
            6.4
        },
        phase: (emit.seed & 63) as f32 * 0.17,
    }
}

fn paired_light(emit: AmbientEmitter, spec: Recipe, time: f32) -> Light {
    let phase = flicker(emit);
    let stutter = (time * 7.4 + phase.phase).sin() * (time * 17.1 + 0.7).cos();
    let energy = if emit.kind == EmitterKind::Cable {
        if stutter > -0.06 {
            spec.energy
        } else {
            spec.energy * 0.06
        }
    } else {
        spec.energy
    };
    let mut pos = emit.pos;
    if emit.kind == EmitterKind::Fire {
        let t = time * 7.0 + phase.phase;
        pos += Vec2::new(t.sin() * 3.0, (t * 1.71).cos() * 2.0);
    }
    let mut light = Light::radial(
        pos,
        spec.radius * emit.intensity.sqrt(),
        spec.core,
        energy * emit.intensity,
    );
    light.scatter = energy * emit.intensity * 0.26;
    light.height = if emit.kind == EmitterKind::Fire {
        6.0
    } else {
        12.0
    };
    light.casts_shadow = matches!(emit.kind, EmitterKind::Fire | EmitterKind::Cable);
    light.shadow_softness = ShadowSoftness::Soft;
    light.flicker = Some(phase);
    light
}

pub(crate) fn draw_ambience(
    map: &mut LightMap,
    pb: &mut PixelBuffer,
    emitters: &[AmbientEmitter],
    cam: Vec2,
    elapsed: f32,
    rat: Vec2,
) {
    for &emitter in emitters {
        let spec = recipe(emitter.kind);
        let pos = emitter.pos - cam;
        let range = spec.radius.max(spec.travel.len() + spec.size + 12.0);
        if pos.x + range < 0.0
            || pos.y + range < 0.0
            || pos.x - range >= pb.w as f32
            || pos.y - range >= pb.h as f32
        {
            continue;
        }
        let light = paired_light(emitter, spec, elapsed);
        if light.casts_shadow {
            map.push_shaped(
                light,
                if emitter.kind == EmitterKind::Fire {
                    3
                } else {
                    1
                },
            );
        } else {
            let energy = light.energy * light.flicker.map_or(1.0, |f| f.gain(elapsed));
            map.emit(light.pos, light.color, light.shape.extent(), energy);
        }
        paint_emitter(map, pb, emitter, spec, pos, elapsed, rat);
    }
}

fn paint_emitter(
    map: &mut LightMap,
    pb: &mut PixelBuffer,
    emitter: AmbientEmitter,
    spec: Recipe,
    pos: Vec2,
    elapsed: f32,
    rat: Vec2,
) {
    let x = pos.x.round() as i32;
    let y = pos.y.round() as i32;
    let flame = emitter.kind == EmitterKind::Fire;
    let core = spec.core;
    let light = flicker(emitter).gain(elapsed);
    let scaled = |c: Rgb, amount: f32| crate::render::palette::scale(c, amount);
    // Housing silhouettes communicate floor decoration, without tell rings
    // or the straight red strokes reserved for attacks.
    match emitter.kind {
        EmitterKind::Fire | EmitterKind::Pilot => {
            let size = if flame && emitter.intensity > 0.7 {
                4
            } else {
                2
            };
            pb.filled_circle(x, y + 1, size, (43, 30, 31));
            for dx in -size..=size {
                let gain = (0.47 + 0.42 * (elapsed * 7.0 + dx as f32 * 1.9).sin().abs()) * light;
                pb.plot(x + dx, y, scaled(core, gain));
            }
            pb.plot(x, y - 1, (255, 194, 100));
            map.mark_emissive_rect(pb, x - size, y, 2 * size + 1, 1);
            map.mark_emissive_rect(pb, x, y - 1, 1, 1);
        }
        EmitterKind::Drip => {
            pb.filled_circle(x, y + 12, 4, (48, 78, 89));
            pb.plot(x - 1, y + 12, scaled(core, 0.57 + light * 0.22));
        }
        EmitterKind::Cable => {
            for n in 0..6 {
                pb.plot(x - n, y + n / 3, (62, 66, 84));
            }
            pb.plot(x, y, core);
            map.mark_emissive_rect(pb, x, y, 1, 1);
        }
        EmitterKind::Steam | EmitterKind::Fog | EmitterKind::Exhaust => {
            pb.filled_circle(x - 1, y, 3, scaled(core, 0.43));
            pb.plot(x + 1, y, scaled(core, 0.68));
        }
        EmitterKind::Fumes => pb.filled_circle(x, y, 3, (71, 87, 55)),
        EmitterKind::Dust | EmitterKind::Insects => {}
    }

    let births_per_sec = emitter.rate * spec.rate;
    let phase = (emitter.seed & 255) as f32 / 256.0;
    let birth = elapsed * births_per_sec + phase;
    let newest = birth.floor() as i32;
    let count = (births_per_sec * spec.life)
        .ceil()
        .clamp(1.0, MAX_BIRTHS as f32) as usize;
    let mut plume = Plume::new(x, y);
    let opacity = MAX_SMOKE_OPACITY * 0.56 * emitter.intensity.min(1.0);
    for i in 0..count {
        let index = newest - i as i32;
        let progress = ((birth - index as f32) / (births_per_sec * spec.life)).clamp(0.0, 1.0);
        let seed = hash(emitter.seed ^ (index as u32).wrapping_mul(0x9e37_79b9));
        let noise = ((seed & 255) as f32 - 127.0) / 127.0;
        let scatter = ((seed >> 8 & 255) as f32 - 127.0) / 127.0;
        let fade = ((1.0 - progress) * (progress * 8.0).min(1.0)).max(0.0);
        let swirl = (1.0 - rat.dist_sq(emitter.pos) / 144.0).max(0.0)
            * (elapsed * 7.0 + progress * 13.0).sin()
            * 5.0
            * progress;
        let drift = Vec2::new(
            noise * spec.size * (0.25 + progress) + swirl,
            scatter * spec.size * progress,
        );
        let at = pos + spec.travel * progress + drift;
        match spec.motion {
            Motion::Rise | Motion::Jet => {
                if emitter.kind != EmitterKind::Pilot {
                    plume.puff(
                        at,
                        (2.0 + progress * spec.size) * emitter.intensity.sqrt(),
                        opacity * fade.sqrt(),
                    );
                }
                if (flame || emitter.kind == EmitterKind::Pilot)
                    && seed.is_multiple_of(3)
                    && progress < 0.64
                {
                    let spark = pos
                        + Vec2::new(
                            noise * progress * 26.0,
                            -28.0 * progress + 19.0 * progress * progress,
                        );
                    dot(map, pb, spark, scaled(core, fade * 0.90));
                }
            }
            Motion::Drip => {
                if progress < 0.69 {
                    dot(
                        map,
                        pb,
                        pos + Vec2::new(0.0, progress * progress * 24.0),
                        scaled(core, 0.68),
                    );
                } else {
                    let radius = ((progress - 0.69) * 12.0) as i32;
                    for side in [-1, 1] {
                        pb.plot(x + radius * side, y + 12, scaled(core, fade));
                    }
                }
            }
            Motion::Spark => {
                if seed.is_multiple_of(4) && progress < 0.55 {
                    let spark = pos
                        + Vec2::new(
                            noise * 29.0 * progress,
                            -19.0 * progress + 34.0 * progress.powi(2),
                        );
                    dot(map, pb, spark, scaled(core, 0.50 + fade * light * 0.48));
                }
            }
            Motion::Drift => dot(map, pb, at, scaled(core, 0.31 + fade * 0.30)),
            Motion::Orbit => {
                let offset = Vec2::from_angle(elapsed * (2.0 + noise) + (seed & 63) as f32)
                    * (4.0 + progress * 9.0);
                dot(map, pb, pos + offset, scaled(core, 0.32 + fade * 0.42));
            }
        }
    }
    plume.paint(pb, spec.vapour);
}

fn dot(map: &mut LightMap, pb: &mut PixelBuffer, p: Vec2, color: Rgb) {
    if super::luminance(color) < 20 {
        return;
    }
    let (x, y) = (p.x.round() as i32, p.y.round() as i32);
    pb.plot(x, y, color);
    map.mark_emissive_rect(pb, x, y, 1, 1);
}

const PLUME_SIDE: usize = 96;

/// Merge overlapping puffs into one bounded-opacity stamp per emitter.
struct Plume {
    origin: (i32, i32),
    bounds: (i32, i32, i32, i32),
    coverage: [u8; PLUME_SIDE * PLUME_SIDE],
}

impl Plume {
    fn new(x: i32, y: i32) -> Self {
        let half = PLUME_SIDE as i32 / 2;
        Self {
            origin: (x - half, y - half),
            bounds: (PLUME_SIDE as i32, PLUME_SIDE as i32, 0, 0),
            coverage: [0; PLUME_SIDE * PLUME_SIDE],
        }
    }

    fn puff(&mut self, p: Vec2, radius: f32, alpha: f32) {
        let p = p - Vec2::new(self.origin.0 as f32, self.origin.1 as f32);
        let x0 = ((p.x - radius).floor() as i32).max(0);
        let x1 = ((p.x + radius).ceil() as i32).min(PLUME_SIDE as i32);
        let y0 = ((p.y - radius * 0.7).floor() as i32).max(0);
        let y1 = ((p.y + radius * 0.7).ceil() as i32).min(PLUME_SIDE as i32);
        let inv = 1.0 / (radius * radius).max(0.1);
        let cap = (MAX_SMOKE_OPACITY * 256.0) as u8;
        for y in y0..y1 {
            let dy2 = ((y as f32 - p.y) * 1.42).powi(2);
            for x in x0..x1 {
                let dx = x as f32 - p.x;
                let falloff = (1.0 - (dx * dx + dy2) * inv).max(0.0);
                let weight = (falloff * alpha * 256.0) as u8;
                let coverage = &mut self.coverage[y as usize * PLUME_SIDE + x as usize];
                *coverage = coverage.saturating_add(weight).min(cap);
            }
        }
        self.bounds.0 = self.bounds.0.min(x0);
        self.bounds.1 = self.bounds.1.min(y0);
        self.bounds.2 = self.bounds.2.max(x1);
        self.bounds.3 = self.bounds.3.max(y1);
    }

    fn paint(&self, pb: &mut PixelBuffer, tint: Rgb) {
        let (ox, oy) = self.origin;
        let (x0, y0, x1, y1) = self.bounds;
        let (w, h) = (pb.w, pb.h);
        let pixels = pb.pixels_mut();
        for y in (y0 + oy).max(0)..(y1 + oy).min(h as i32) {
            for x in (x0 + ox).max(0)..(x1 + ox).min(w as i32) {
                let weight =
                    u32::from(self.coverage[(y - oy) as usize * PLUME_SIDE + (x - ox) as usize]);
                if weight == 0 {
                    continue;
                }
                let pixel = &mut pixels[y as usize * w + x as usize];
                let mix = |a: u8, b: u8| {
                    ((u32::from(a) * (256 - weight) + u32::from(b) * weight) >> 8) as u8
                };
                *pixel = (
                    mix(pixel.0, tint.0),
                    mix(pixel.1, tint.1),
                    mix(pixel.2, tint.2),
                );
            }
        }
    }
}

fn hash(mut seed: u32) -> u32 {
    seed ^= seed >> 16;
    seed = seed.wrapping_mul(0x7feb_352d);
    seed ^ (seed >> 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const KINDS: [EmitterKind; 10] = [
        EmitterKind::Fire,
        EmitterKind::Steam,
        EmitterKind::Drip,
        EmitterKind::Cable,
        EmitterKind::Dust,
        EmitterKind::Fog,
        EmitterKind::Exhaust,
        EmitterKind::Pilot,
        EmitterKind::Fumes,
        EmitterKind::Insects,
    ];

    #[test]
    fn each_kind_emits_a_light_and_particles_culled_by_view() {
        for kind in KINDS {
            let emitter = AmbientEmitter {
                kind,
                pos: Vec2::new(42.0, 50.0),
                intensity: 1.0,
                rate: 1.0,
                seed: 24,
            };
            let spec = recipe(kind);
            let light = paired_light(emitter, spec, 12.0);
            assert!(light.energy > 0.0 && light.shape.extent() > 4.0, "{kind:?}");
            let mut map = LightMap::new();
            let mut pb = PixelBuffer::new(96, 80);
            draw_ambience(
                &mut map,
                &mut pb,
                &[emitter],
                Vec2::new(900.0, 900.0),
                12.0,
                Vec2::ZERO,
            );
            assert!(pb.pixels().iter().all(|&c| c == (0, 0, 0)));
            draw_ambience(&mut map, &mut pb, &[emitter], Vec2::ZERO, 12.0, Vec2::ZERO);
            let first = pb.pixels().to_vec();
            assert!(first.iter().any(|&c| c != (0, 0, 0)), "{kind:?}");
            assert!(
                (1..=8).any(|i| {
                    pb.clear((0, 0, 0));
                    map.begin_frame();
                    draw_ambience(
                        &mut map,
                        &mut pb,
                        &[emitter],
                        Vec2::ZERO,
                        12.0 + i as f32 / 13.0,
                        Vec2::ZERO,
                    );
                    first != pb.pixels()
                }),
                "static {kind:?}"
            );
        }
    }

    #[test]
    fn worst_case_overlapping_smoke_cannot_cover_more_than_a_quarter_of_a_pixel() {
        let mut pb = PixelBuffer::new(20, 20);
        for count in [1, 4, 12] {
            pb.clear((255, 255, 255));
            let mut plume = Plume::new(10, 10);
            for _ in 0..count {
                plume.puff(Vec2::new(10.0, 10.0), 8.0, 0.19);
            }
            plume.paint(&mut pb, (0, 0, 0));
            assert!(
                pb.pixel_at(10, 10).0 >= 191,
                "too much smoke at {count} overlaps"
            );
        }
    }
}
