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

/// How wide a wall lamp's fan is, half-angle in radians: about 117° of the
/// half-disc in front of the face. Wide enough to wash the floor beside it,
/// narrow enough that a corridor lamp does not light the passage back the way
/// it came.
pub(super) const SCONCE_HALF_ANGLE: f32 = 1.02;
/// How high a wall lamp sits, in world px. Low on the wall, so a crate or a
/// counter beside it still throws a shadow.
pub(super) const SCONCE_HEIGHT: f32 = 9.0;
/// How many times a second a wall lamp's flicker takes a new value.
const SCONCE_FLICKER_HZ: f32 = 3.0;

/// The lens colour of a theme's wall lamp (batch 9): the light's tint and the
/// drawn lens are one value, so the pixels and the pool can never disagree.
///
/// Warm on most floors — an amber caged bulkhead lamp in the Sewer, a warm
/// white under-shelf strip in the Kitchen, a soft yellow enamel wall lamp in
/// the Laundry, an orange rail marker on the Flatbed. The Lab reads cold
/// cyan-green (a recessed indicator) with the odd red one, and the Printworks'
/// darkroom safelight is deep red with the odd green register lamp.
pub(super) fn sconce_color(theme: Theme, seed: u32) -> Rgb {
    match theme {
        Theme::Sewer => (255, 176, 92),
        Theme::Kitchen => (255, 233, 196),
        Theme::Lab => {
            if seed.is_multiple_of(5) {
                (255, 92, 78)
            } else {
                (132, 255, 214)
            }
        }
        Theme::Laundry => (255, 238, 168),
        Theme::Printworks => {
            if seed.is_multiple_of(4) {
                (126, 255, 132)
            } else {
                (222, 56, 48)
            }
        }
        Theme::Trailer => (255, 152, 62),
    }
}

/// One wall lamp in eight is a spent one: dead, or sputtering now and then.
/// The light and the drawn lens read this same rule (and [`sconce_sputter`]),
/// so a dead lamp never shows a lit pixel over a black pool.
pub(super) fn sconce_spent(seed: u32) -> bool {
    seed.is_multiple_of(8)
}

/// Whether a spent wall lamp's dying lens is lit at `elapsed`. The rate is the
/// one the [`Kind::Broken`] fixture and its art share (7.7 rad/s).
pub(super) fn sconce_sputter(seed: u32, elapsed: f32) -> bool {
    (elapsed * 7.7 + (seed & 63) as f32 * 0.43).sin() > 0.86
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
            // A wall lamp is drawn at its face, not on the hanging jig.
            Kind::Sconce => {
                draw_sconce(map, pb, theme, fixture, x, y, elapsed);
                continue;
            }
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

/// One wall-mounted lamp: a few pixels of housing on the face plus a bright
/// lens, marked emissive so the lamp itself stays bright however dark the room
/// is.
///
/// The fixture is placed on its face, half a pixel inside the floor cell in
/// front of it, so the housing straddles wall and floor and reads as a lamp on
/// that wall. The art is deliberately orientation-free — the shape comes from
/// the theme (a cage, a strip, an indicator, a bracket, a safelight, a rail
/// marker) and the colour from [`sconce_color`].
fn draw_sconce(
    map: &mut LightMap,
    pb: &mut PixelBuffer,
    theme: Theme,
    fixture: &LightFixture,
    x: i32,
    y: i32,
    elapsed: f32,
) {
    let casing = (39, 43, 49);
    let lens = sconce_color(theme, fixture.seed);
    let lit = !sconce_spent(fixture.seed) || sconce_sputter(fixture.seed, elapsed);
    let lens = if lit { lens } else { (71, 52, 46) };
    // How wide the lit part of the housing is, so the emissive mark covers
    // exactly the pixels that carry the lens.
    let emissive_half = match theme {
        // Sewer: a caged bulkhead lamp — a dark cage over an amber lens.
        Theme::Sewer => {
            for dx in -1..=1 {
                pb.plot(x + dx, y - 1, casing);
                pb.plot(x + dx, y + 1, casing);
            }
            pb.plot(x - 1, y, casing);
            pb.plot(x + 1, y, casing);
            pb.plot(x, y, lens);
            0
        }
        // Kitchen: an under-shelf strip, warm white.
        Theme::Kitchen => {
            for dx in -1..=1 {
                pb.plot(x + dx, y - 1, casing);
                pb.plot(x + dx, y, lens);
            }
            1
        }
        // Lab: a recessed indicator, one cold pixel in a dark recess.
        Theme::Lab => {
            pb.plot(x, y - 1, casing);
            pb.plot(x - 1, y, casing);
            pb.plot(x + 1, y, casing);
            pb.plot(x, y, lens);
            0
        }
        // Laundry: an enamel wall lamp on a little bracket.
        Theme::Laundry => {
            pb.plot(x, y - 1, casing);
            for dx in -1..=1 {
                pb.plot(x + dx, y + 1, casing);
            }
            pb.plot(x - 1, y, casing);
            pb.plot(x + 1, y, casing);
            pb.plot(x, y, lens);
            0
        }
        // Printworks: a darkroom safelight — a hooded bar, deep red.
        Theme::Printworks => {
            for dx in -1..=1 {
                pb.plot(x + dx, y, lens);
                pb.plot(x + dx, y + 1, casing);
            }
            1
        }
        // Trailer: a marker lamp on the side rail, one orange pixel.
        Theme::Trailer => {
            pb.plot(x, y - 1, casing);
            pb.plot(x, y, lens);
            0
        }
    };
    if lit {
        map.mark_emissive_rect(pb, x - emissive_half, y, 2 * emissive_half + 1, 1);
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
        Kind::Sconce => {
            // A lamp on a wall face: the fan runs away from the wall along the
            // face normal, and the wall's own occluder (baked from the pixel
            // next to it) keeps the back half dark, so it never lights through
            // its own wall.
            light.shape = LightShape::Cone {
                radius,
                dir: fixture.wall_facing(&scene.level.tiles),
                half_angle: SCONCE_HALF_ANGLE,
            };
            light.color = sconce_color(theme, fixture.seed);
            light.height = SCONCE_HEIGHT;
            light.energy = 0.78;
            light.scatter = 0.13;
            if sconce_spent(fixture.seed) {
                // One in eight is spent: dead, with an occasional sputter. The
                // drawn lens follows the same rule (`sconce_sputter`).
                light.energy = if sconce_sputter(fixture.seed, scene.elapsed) {
                    0.26
                } else {
                    0.010
                };
                light.scatter = light.energy * 0.30;
            } else {
                // A wall lamp breathes in steps, not every frame: the same
                // gentle curve a hanging lamp uses, sampled three times a
                // second and held. A continuous flicker repainted its whole
                // pool every frame (measured: +46–80 % changed cells on a still
                // screen with one lamp in view); held steps repaint it three
                // times a second.
                let held = (scene.elapsed * SCONCE_FLICKER_HZ).floor() / SCONCE_FLICKER_HZ;
                light.energy *= Flicker {
                    amount: 0.09,
                    speed: 7.0,
                    phase: phase_offset,
                }
                .gain(held);
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

#[cfg(test)]
mod tests {
    use super::*;
    // `Geometry` and `ShadowProfile` live in the private `shadow` module, so the
    // test names it from the `lighting` root rather than from `fixtures`.
    use crate::render::lighting::shadow::{Geometry, ShadowProfile};
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap, TILE_F};
    use crate::game::level::{Level, LightFixture, LightFixtureKind};

    /// The wall lamp's light, built the way [`light`]'s `Sconce` arm builds it.
    fn sconce_light(fixture: LightFixture, dir: Vec2) -> Light {
        let mut light = Light::radial(fixture.pos, fixture.radius, sconce_color(Theme::Sewer, 0), 0.78);
        light.shape = LightShape::Cone {
            radius: fixture.radius,
            dir,
            half_angle: SCONCE_HALF_ANGLE,
        };
        light.height = SCONCE_HEIGHT;
        light
    }

    /// A wall lamp's pool stops at the wall it hangs on: with floor on **both**
    /// sides of a one-cell wall, the tile straight behind the lamp stays dark
    /// while the tile in front of it is lit.
    #[test]
    fn a_wall_lamp_does_not_light_the_far_side_of_its_wall() {
        let mut level = Level::empty(Theme::Sewer);
        let (wx, wy) = (10, 50);
        level
            .tiles
            .fill_cells(wx, wy - 10, 1, 20, TileKind::Wall(WallKind::Solid));
        let dir = Vec2::new(1.0, 0.0);
        let fixture = LightFixture {
            pos: TileMap::tile_rect(wx, wy).center() + dir * (TILE_F * 0.5 + 0.5),
            kind: LightFixtureKind::Sconce,
            radius: 46.0,
            seed: 3,
        };
        level.objects.lights.push(fixture);
        // The renderer reads the mount back off the grid: the generator's own
        // rule, so the fan can only run into the room.
        assert_eq!(fixture.wall_facing(&level.tiles), dir);

        let mut geometry = Geometry::default();
        geometry.sync(&level);
        let light = sconce_light(fixture, dir);
        let mut baked = ShadowProfile::default();
        baked.bake(&geometry, light);

        let near = TileMap::tile_rect(wx + 2, wy).center();
        let far = TileMap::tile_rect(wx - 1, wy).center();
        assert_eq!(level.tiles.get_at(far), TileKind::Open);
        assert_eq!(level.tiles.get_at(near), TileKind::Open);
        assert_eq!(
            baked.visibility(
                near - fixture.pos,
                geometry.surface(near),
                ShadowSoftness::None
            ),
            1.0,
            "the floor in front of the lamp is lit"
        );
        assert_eq!(
            baked.visibility(
                far - fixture.pos,
                geometry.surface(far),
                ShadowSoftness::None
            ),
            0.0,
            "the lamp's own wall stops its light"
        );
    }

    /// The look: warm on the warm floors, cold in the Lab, a darkroom safelight
    /// in the Printworks — with the odd red indicator and green register lamp.
    #[test]
    fn wall_lamp_colours_follow_the_floor() {
        for seed in 0..64u32 {
            for theme in [
                Theme::Sewer,
                Theme::Kitchen,
                Theme::Laundry,
                Theme::Trailer,
            ] {
                let (r, g, b) = sconce_color(theme, seed);
                assert!(r > 190 && r > g && g > b, "{theme:?} {seed}: {:?}", (r, g, b));
            }
            let (r, g, b) = sconce_color(Theme::Lab, seed);
            if seed % 5 == 0 {
                assert!(r > 240 && g < 120, "lab red indicator: {:?}", (r, g, b));
            } else {
                assert!(b > 190 && g > 240 && r < 160, "lab lamp: {:?}", (r, g, b));
            }
            let (r, g, b) = sconce_color(Theme::Printworks, seed);
            if seed % 4 == 0 {
                assert!(g > 240 && r < 160, "register lamp: {:?}", (r, g, b));
            } else {
                assert!(r > 200 && g < 80 && b < 80, "safelight: {:?}", (r, g, b));
            }
            // One lamp in eight is spent.
            assert_eq!(sconce_spent(seed), seed.is_multiple_of(8));
        }
    }
}
