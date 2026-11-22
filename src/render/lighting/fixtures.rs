//! Art direction for generated, non-blocking level fixtures.

use super::{
    Falloff, Flicker, Light, LightMap, LightScene, LightShape, ShadowColor, ShadowSoftness,
    ThemeAmbient,
};
use crate::game::level::{LightFixture, LightFixtureKind as Kind, Theme};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

pub(super) fn colors(theme: Theme) -> (Rgb, Rgb) {
    match theme {
        Theme::Sewer => ((87, 216, 184), (244, 165, 78)),
        Theme::Kitchen => ((255, 206, 132), (255, 116, 72)),
        Theme::Lab => ((108, 203, 255), (137, 255, 213)),
        Theme::Laundry => ((161, 184, 255), (218, 163, 247)),
        Theme::Printworks => ((108, 230, 225), (246, 125, 157)),
        Theme::Trailer => ((255, 223, 159), (138, 168, 226)),
    }
}

pub(crate) fn draw_fixtures(
    map: &mut LightMap,
    pb: &mut PixelBuffer,
    fixtures: &[LightFixture],
    theme: Theme,
    cam: Vec2,
    elapsed: f32,
) {
    let (primary, alternate) = colors(theme);
    for fixture in fixtures {
        let p = fixture.pos - cam;
        if p.x < -6.0 || p.y < -4.0 || p.x >= pb.w as f32 + 6.0 || p.y >= pb.h as f32 + 4.0 {
            continue;
        }
        let (x, y) = (p.x.round() as i32, p.y.round() as i32);
        let (half, color) = match fixture.kind {
            Kind::Pendant => (
                4,
                if fixture.seed.is_multiple_of(4) {
                    alternate
                } else {
                    primary
                },
            ),
            Kind::Arena => (
                5,
                if fixture.seed & 1 == 0 {
                    primary
                } else {
                    alternate
                },
            ),
            Kind::Passage => (2, alternate),
            Kind::Broken => (3, alternate),
            Kind::Sun => continue,
        };
        let casing = (39, 43, 49);
        for dx in -half..=half {
            pb.plot(x + dx, y - 1, casing);
            pb.plot(x + dx, y + 1, casing);
        }
        pb.plot(x - half, y, (82, 85, 92));
        pb.plot(x + half, y, (82, 85, 92));
        let lit = fixture.kind != Kind::Broken
            || (elapsed * 7.7 + (fixture.seed & 63) as f32 * 0.43).sin() > 0.84;
        for dx in (-half + 1)..half {
            pb.plot(x + dx, y, if lit { color } else { (71, 52, 46) });
        }
        if lit {
            map.mark_emissive_rect(pb, x - half + 1, y, 2 * half - 1, 1);
        }
    }
}

pub(super) fn light(
    fixture: LightFixture,
    theme: Theme,
    ambient: ThemeAmbient,
    phase: Option<u8>,
    scene: &LightScene<'_>,
) -> Light {
    let (primary, secondary) = colors(theme);
    let phase_offset = (fixture.seed & 63) as f32 * 0.43;
    let radius = fixture.radius;
    let mut light = Light::radial(fixture.pos, radius, primary, 0.83);
    light.falloff = if ambient.hard_pools {
        Falloff::Hard
    } else {
        Falloff::Smooth
    };
    match fixture.kind {
        Kind::Pendant => {
            light.height = 23.0;
            light.scatter = 0.15;
            if fixture.seed.is_multiple_of(4) {
                light.color = secondary;
                light.flicker = Some(Flicker {
                    amount: 0.17,
                    speed: 8.0,
                    phase: phase_offset,
                });
            }
        }
        Kind::Passage => {
            light.color = secondary;
            light.energy = 0.67;
            light.height = 15.0;
            // A corridor lamp is a small source: the soft, long tail carries
            // it down the passage without a hard pool edge.
            if light.falloff == Falloff::Smooth {
                light.falloff = Falloff::Small;
            }
        }
        Kind::Arena => {
            light.color = if fixture.seed & 1 == 0 {
                primary
            } else {
                secondary
            };
            let centre = scene
                .level
                .boss_room()
                .map_or(fixture.pos, |room| room.center());
            let dir = (centre - fixture.pos).normalized();
            light.height = 20.0;
            light.energy = 0.62;
            light.falloff = Falloff::Linear;
            light.scatter = 0.13;
            light.shadow_softness = ShadowSoftness::Softer;
            if let Some(phase) = phase {
                let slew =
                    (scene.elapsed * (0.8 + 0.32 * f32::from(phase)) + phase_offset).sin() * 0.32;
                light.shape = LightShape::Cone {
                    radius,
                    dir: dir.rotate(slew),
                    half_angle: 0.60,
                };
                light.color = match phase {
                    0 | 1 => light.color,
                    2 => (255, 98, 89),
                    _ => (223, 87, 231),
                };
                light.energy = 0.93;
            } else {
                light.shape = LightShape::Cone {
                    radius,
                    dir,
                    half_angle: 0.96,
                };
            }
        }
        Kind::Broken => {
            light.energy = if (scene.elapsed * 7.7 + phase_offset).sin() > 0.84 {
                0.31
            } else {
                0.015
            };
            light.scatter = light.energy * 0.30;
            light.color = secondary;
            // A dying bulb is a small source too, so its infrequent flash has
            // a soft skirt rather than a ring.
            if light.falloff == Falloff::Smooth {
                light.falloff = Falloff::Small;
            }
        }
        Kind::Sun => {
            let angle = 0.85 + (scene.elapsed * 0.043).sin() * 0.27;
            light.shape = LightShape::Directional {
                dir: Vec2::from_angle(angle),
            };
            light.energy = 0.77;
            light.scatter = 0.145;
            light.height = 0.83;
            light.shadow_color = ShadowColor {
                rgb: (70, 97, 166),
                opacity: 0.93,
            };
            // Short sun shadows suit the deck's chunky sprites.
            light.shadow_softness = ShadowSoftness::None;
        }
    }
    light
}
