//! Art direction for generated, non-blocking level fixtures.

use super::{
    Falloff, Flicker, Light, LightMap, LightScene, LightShape, ShadowColor, ShadowSoftness,
    ThemeAmbient,
};
use crate::game::level::{Level, LightFixture, LightFixtureKind as Kind, Theme};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

pub(super) fn colors(theme: Theme) -> (Rgb, Rgb) {
    match theme {
        Theme::Sewer => ((87, 216, 184), (244, 165, 78)),
        Theme::Kitchen => ((255, 206, 132), (255, 116, 72)),
        Theme::Lab => ((108, 203, 255), (137, 255, 213)),
        Theme::Laundry => ((161, 184, 255), (218, 163, 247)),
        Theme::Printworks => ((108, 230, 225), (246, 125, 157)),
        Theme::Trailer => ((255, 223, 159), (138, 168, 226)),
        Theme::Outfall => ((92, 232, 236), (214, 168, 114)),
    }
}

/// How wide a wall lamp's fan is, half-angle in radians: 150° of the half-disc
/// in front of the face (batch 12 made it wide: it is a lamp lighting a room,
/// not a spotlight). Still narrow enough that a corridor lamp does not light
/// the passage behind its own wall.
pub(super) const SCONCE_HALF_ANGLE: f32 = 1.31;
/// How high a wall lamp sits, in world px. Low on the wall, so a crate or a
/// counter beside it still throws a shadow.
pub(super) const SCONCE_HEIGHT: f32 = 9.0;
/// A wall lamp's intensity (batch 12): three times batch 9's 0.78, so its core
/// is brighter than a hanging Pendant's (0.83 at the radial default) and the
/// floor at its foot is a hot pool rather than a tint. `energy` and `scatter`
/// both scale the contribution, and the multiplier is clamped at `350` (`Q8.8`)
/// per channel, so the saturated white-hot core reaches about 60 px and what
/// the eye reads past that is the smooth cone falling off to the 110 px radius.
pub(super) const SCONCE_ENERGY: f32 = 2.35;
/// How much a wall lamp lifts its tint towards white (0.13 in batch 9): the hot
/// core reads as a lamp rather than as a coloured haze.
pub(super) const SCONCE_SCATTER: f32 = 0.34;
/// The radius of the small, very bright light that washes the face a lamp hangs
/// on and the floor at its foot, in world px. It shares the fan's baked
/// occlusion, so it cannot leak through the wall, and its `Falloff::Small` tail
/// runs the glow about 44 px along the face.
pub(super) const SCONCE_WASH_RADIUS: f32 = 26.0;
/// How many times a second a wall lamp's flicker takes a new value.
const SCONCE_FLICKER_HZ: f32 = 3.0;
/// A wall lamp's flicker amplitude: a 3 % dip, held in steps. The *absolute*
/// breath is what repaints cells, and a share of the new 2.35 energy worth 3 %
/// is about the share of batch 9's 0.78 worth 9 % (`0.09 × 0.78 / 2.35`), so the
/// lamp breathes as far as it always did while being three times as bright.
/// The saturated core (about 60 px) does not move at all under a 3 % step; what
/// changes is the ring outside it. If a still frame's bytes do rise, this is the
/// knob (with `SCONCE_FLICKER_HZ`), not the brightness.
const SCONCE_FLICKER_AMOUNT: f32 = 0.03;

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
        Theme::Sewer => (255, 190, 96),
        Theme::Kitchen => (255, 233, 196),
        Theme::Lab => {
            if seed.is_multiple_of(5) {
                (255, 84, 70)
            } else {
                (110, 255, 205)
            }
        }
        Theme::Laundry => (255, 244, 190),
        Theme::Printworks => {
            if seed.is_multiple_of(4) {
                (130, 255, 140)
            } else {
                // The safelight is the only light in its room: a bright,
                // saturated red rather than the near-black maroon a darkroom
                // would really use, because it has to be seen *by*.
                (255, 74, 62)
            }
        }
        Theme::Trailer => (255, 162, 60),
        Theme::Outfall => {
            if seed.is_multiple_of(5) {
                // Subdued warm sodium-brass fallback on occasional pilings.
                (226, 178, 116)
            } else {
                // Cold marine cyan-aqua storm-gate bulkhead lens.
                (96, 236, 232)
            }
        }
    }
}

/// The colour of a lamp's lit lens: [`sconce_color`] lifted towards white, so
/// the core reads as a lamp and the pool it throws keeps the theme's tint.
pub(super) fn lens_color(tint: Rgb) -> Rgb {
    palette::mix(tint, (255, 255, 255), 0.55)
}

/// The 1 px rim around a lit housing: the theme colour at a little under half,
/// bright enough to read as glow on a dark wall and never as a second lens.
fn rim_color(tint: Rgb) -> Rgb {
    palette::scale(tint, 0.42)
}

/// One theme's lamp housing in pixels: half its drawn width, half the width of
/// its lit lens, and whether a cage crosses the lens.
#[derive(Clone, Copy)]
struct Housing {
    half: i32,
    lens: i32,
    cage: bool,
}

impl Housing {
    /// Total width in pixels, the 1 px glow rim included.
    fn width(self) -> i32 {
        2 * self.half + 1
    }
}

/// Batch 12: every lamp is five or seven pixels wide on its wall face — wide
/// enough to read as a fitting at 1× and to survive the 2× resample — with a
/// bright lens, a casing frame and a 1 px rim of its own light. The motif is
/// the theme's (a caged bulkhead, a strip, an indicator, an enamel lamp, a
/// hooded safelight bar, a rail marker); only the size grew.
fn housing(theme: Theme) -> Housing {
    match theme {
        // A caged amber bulkhead: two bars over the lens.
        Theme::Sewer => Housing {
            half: 3,
            lens: 2,
            cage: true,
        },
        // A warm-white under-shelf strip.
        Theme::Kitchen => Housing {
            half: 3,
            lens: 2,
            cage: false,
        },
        // A recessed cold indicator.
        Theme::Lab => Housing {
            half: 2,
            lens: 1,
            cage: false,
        },
        // A soft-yellow enamel wall lamp on its bracket.
        Theme::Laundry => Housing {
            half: 2,
            lens: 1,
            cage: false,
        },
        // A deep-red safelight under a hood.
        Theme::Printworks => Housing {
            half: 3,
            lens: 2,
            cage: false,
        },
        // An orange marker on the side rail.
        Theme::Trailer => Housing {
            half: 2,
            lens: 1,
            cage: false,
        },
        // A heavy caged marine bulkhead lamp on a pier piling.
        Theme::Outfall => Housing {
            half: 3,
            lens: 2,
            cage: true,
        },
    }
}

/// One wall lamp in eight is a spent one: dead, or sputtering now and then.
/// The light and the drawn lens read this same rule (and [`sconce_sputter`]),
/// so a dead lamp never shows a lit pixel over a black pool.
pub(super) fn sconce_spent(seed: u32) -> bool {
    LightFixture::sconce_spent(seed)
}

/// Whether a spent wall lamp's dying lens is lit at `elapsed`. The rate is the
/// one the [`Kind::Broken`] fixture and its art share (7.7 rad/s).
pub(super) fn sconce_sputter(seed: u32, elapsed: f32) -> bool {
    (elapsed * 7.7 + (seed & 63) as f32 * 0.43).sin() > 0.86
}

/// Whether `fixture` at `index` is a wall lamp currently snuffed by an Outfall
/// elite or The Lantern. Snuffed sconces reject housing emission, fan and wash.
#[inline]
pub(super) fn sconce_snuffed(level: &Level, index: usize, fixture: LightFixture) -> bool {
    fixture.kind == Kind::Sconce && level.sconce_snuffed(index)
}

pub(crate) fn draw_fixtures(
    map: &mut LightMap,
    pb: &mut PixelBuffer,
    level: &Level,
    cam: Vec2,
    elapsed: f32,
    blackout: f32,
) {
    let theme = level.theme;
    let (primary, alternate) = colors(theme);
    let power = 1.0 - blackout;
    for (index, fixture) in level.objects.lights.iter().enumerate() {
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
                let power = if level.sconce_snuffed(index) { None } else { Some(power) };
                draw_sconce(map, pb, theme, fixture, (x, y), elapsed, power);
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
        let lit = power > 0.0
            && (fixture.kind != Kind::Broken
                || (elapsed * 7.7 + (fixture.seed & 63) as f32 * 0.43).sin() > 0.84);
        let glass = (71, 52, 46);
        let lens = if lit { palette::mix(glass, color, power) } else { glass };
        for dx in (-half + 1)..half {
            pb.plot(x + dx, y, lens);
        }
        if lit {
            map.mark_emissive_rect(pb, x - half + 1, y, 2 * half - 1, 1);
        }
    }
}

/// One wall-mounted lamp: a five- or seven-pixel housing on the face, a bright
/// near-white lens and a 1 px rim of the theme's colour, marked emissive so the
/// lamp itself stays bright however dark the room is.
///
/// The fixture is placed on its face, half a pixel inside the floor cell in
/// front of it, so the housing straddles wall and floor and reads as a lamp on
/// that wall. The art is deliberately orientation-free — the shape comes from
/// the theme (a cage, a strip, an indicator, a bracket, a safelight, a rail
/// marker) and the colours from [`sconce_color`].
#[allow(clippy::too_many_arguments)]
fn draw_sconce(
    map: &mut LightMap,
    pb: &mut PixelBuffer,
    theme: Theme,
    fixture: &LightFixture,
    (x, y): (i32, i32),
    elapsed: f32,
    power: Option<f32>,
) {
    let casing = (39, 43, 49);
    let tint = sconce_color(theme, fixture.seed);
    let housing = housing(theme);
    let (lens, rim, lit) = if let Some(power) = power {
        let glass = (71, 52, 46);
        let cold_rim = (48, 42, 40);
        let lit = power > 0.0 && (!sconce_spent(fixture.seed) || sconce_sputter(fixture.seed, elapsed));
        let lens = if lit { palette::mix(glass, lens_color(tint), power) } else { glass };
        let rim = if lit { palette::mix(cold_rim, rim_color(tint), power) } else { cold_rim };
        (lens, rim, lit)
    } else {
        ((54, 48, 46), (38, 40, 44), false)
    };
    // The cage's bars sit at the ends of the lens (a bulkhead's wire cage), in
    // the lens colour at a little under half, so the bulkhead reads as a lit
    // pane behind bars rather than one wide light.
    let bar = palette::scale(lens, 0.45);
    for dx in -housing.half..=housing.half {
        let edge = dx.abs() == housing.half;
        let core = dx.abs() <= housing.lens;
        let top = if edge { rim } else { casing };
        let bottom = top;
        let middle = if edge {
            rim
        } else if core && housing.cage && dx.abs() == housing.lens {
            bar
        } else if core {
            lens
        } else {
            casing
        };
        pb.plot(x + dx, y - 1, top);
        pb.plot(x + dx, y, middle);
        pb.plot(x + dx, y + 1, bottom);
    }
    if lit {
        map.mark_emissive_rect(pb, x - housing.half, y - 1, housing.width(), 3);
    }
}

/// The bright wash a wall lamp throws on the face it hangs on and on the floor
/// at its foot (batch 12): a second, very small and very bright radial source at
/// the mount, so the lamp reads as the *source* rather than as a pool with
/// nothing above it.
///
/// The caller splats it with the fan's own baked `ShadowProfile`, so the wall it
/// hangs on still occludes it — a raised surface keeps an 8 px shoulder in
/// `ShadowProfile::visibility` — and it stops at the wall's far side exactly as
/// the fan does. It carries the fan's already-attenuated energy, so a lamp in a
/// dim passage, or a spent one, washes as faintly as it lights; and it does not
/// flicker, so the hot core stays still while the fan breathes and the held-step
/// flicker only ever repaints the fan's own cells.
pub(super) fn sconce_wash(fan: &Light, seed: u32, elapsed: f32) -> Light {
    // The fan's energy with its held flicker divided back out.
    let steady = fan.energy / sconce_flicker_gain(seed, elapsed).max(1e-3);
    let mut wash = Light::radial(fan.pos, SCONCE_WASH_RADIUS, lens_color(fan.color), steady);
    wash.scatter = fan.scatter;
    wash.height = fan.height;
    // A small pool: the smooth core plus the engine's faint long tail, out to
    // `TAIL_REACH` times the radius, which runs the glow along the wall face.
    wash.falloff = Falloff::Small;
    wash.casts_shadow = false;
    wash.layers = fan.layers;
    wash.occluders = fan.occluders;
    wash
}

/// A live wall lamp's held-step flicker gain (1.0 for a spent lamp, whose
/// sputter is its own rule).
pub(super) fn sconce_flicker_gain(seed: u32, elapsed: f32) -> f32 {
    if sconce_spent(seed) {
        return 1.0;
    }
    let held = (elapsed * SCONCE_FLICKER_HZ).floor() / SCONCE_FLICKER_HZ;
    Flicker {
        amount: SCONCE_FLICKER_AMOUNT,
        speed: 7.0,
        phase: (seed & 63) as f32 * 0.43,
    }
    .gain(held)
}

pub(super) fn light(
    index: usize,
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
            // its own wall. Batch 12 made it a room light: three times the
            // intensity, a 150° fan and a 110 px reach, plus the wash on its
            // own face (`sconce_wash`, splatted by the lighting pass).
            light.shape = LightShape::Cone {
                radius,
                dir: fixture.wall_facing(&scene.level.tiles),
                half_angle: SCONCE_HALF_ANGLE,
            };
            light.color = sconce_color(theme, fixture.seed);
            light.height = SCONCE_HEIGHT;
            light.energy = SCONCE_ENERGY;
            light.scatter = SCONCE_SCATTER;
            if scene.level.sconce_snuffed(index) {
                light.energy = 0.0;
                light.scatter = 0.0;
            } else if sconce_spent(fixture.seed) {
                // One in eight is spent: dead, with an occasional sputter. The
                // drawn lens follows the same rule (`sconce_sputter`), and the
                // sputter stays a fraction of a live lamp's core so a dead wall
                // never outshines a working one.
                light.energy = if sconce_sputter(fixture.seed, scene.elapsed) {
                    0.42
                } else {
                    0.020
                };
                light.scatter = light.energy * 0.30;
            } else {
                // A wall lamp breathes in steps, not every frame: the same
                // gentle curve a hanging lamp uses, sampled three times a
                // second and held. A continuous flicker repainted its whole
                // pool every frame (measured: +46–80 % changed cells on a still
                // screen with one lamp in view); held steps repaint it three
                // times a second. The amplitude is 3 % of the light, so the
                // step a lit pixel takes is the size it took in batch 9.
                light.energy *= sconce_flicker_gain(fixture.seed, scene.elapsed);
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
    if fixture.kind != Kind::Sun {
        let power = 1.0 - scene.blackout;
        light.energy *= power;
        light.scatter *= power;
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
    use crate::game::world::World;
    use crate::render::lighting::{luminance, LightingMode};
    use crate::term::ColorDepth;

    /// The world px a generated wall lamp's fan reaches (mirrors
    /// `levelgen::lights::SCONCE_RADIUS`, which is private to its module).
    const LAMP_RADIUS: f32 = 110.0;
    /// The floor a test frame is cleared to, so a light's contribution reads as
    /// a difference and not as art.
    const FLOOR: Rgb = (85, 71, 78);

    /// The wall lamp's light, built the way [`light`]'s `Sconce` arm builds it.
    fn sconce_light(fixture: LightFixture, dir: Vec2) -> Light {
        let mut light = Light::radial(
            fixture.pos,
            fixture.radius,
            sconce_color(Theme::Sewer, fixture.seed),
            SCONCE_ENERGY,
        );
        light.shape = LightShape::Cone {
            radius: fixture.radius,
            dir,
            half_angle: SCONCE_HALF_ANGLE,
        };
        light.height = SCONCE_HEIGHT;
        light.scatter = SCONCE_SCATTER;
        light
    }

    /// An empty world with the camera at the origin, so a fixture placed near
    /// the top-left of the map is in view and no zone or hanging lamp exists to
    /// attenuate the reading.
    fn test_world(theme: Theme) -> World {
        let mut w = World::new(theme, 80);
        w.level = Level::empty(theme);
        w.player.pos = Vec2::new(2600.0, 2000.0);
        w.smoke_enabled = false;
        w
    }

    fn scene(w: &World) -> LightScene<'_> {
        LightScene {
            ecs: &w.ecs,
            level: &w.level,
            player: &w.player,
            particles: &w.particles,
            thrash: None,
            squeak: None,
            cam: Vec2::ZERO,
            elapsed: w.elapsed,
            blackout: w.blackout(),
            mode: LightingMode::Full,
            depth: ColorDepth::Truecolor,
            fx: false,
        }
    }

    /// Draw the map's fixtures into a fresh dark frame and run the lighting
    /// pass over it, exactly as `World::draw` does.
    fn lit(w: &World) -> PixelBuffer {
        let mut pb = PixelBuffer::new(160, 160);
        pb.clear(FLOOR);
        let mut map = LightMap::new();
        let scene = scene(w);
        draw_fixtures(&mut map, &mut pb, &w.level, Vec2::ZERO, scene.elapsed, scene.blackout);
        map.apply_scene(&mut pb, &scene);
        pb
    }

    /// A lamp mounted on the south face of a three-cell wall row at `wall_y`,
    /// with open floor below it: the generator's own geometry, around a mount
    /// at (72, 48.5) on a 160 px frame.
    fn mount(level: &mut Level, x: i32, wall_y: i32, seed: u32) -> LightFixture {
        level
            .tiles
            .fill_cells(x - 1, wall_y, 3, 1, TileKind::Wall(WallKind::Solid));
        let pos = TileMap::tile_rect(x, wall_y).center() + Vec2::new(0.0, TILE_F * 0.5 + 0.5);
        let fixture = LightFixture {
            pos,
            kind: LightFixtureKind::Sconce,
            radius: LAMP_RADIUS,
            seed,
        };
        level.objects.lights.push(fixture);
        fixture
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
            radius: LAMP_RADIUS,
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
            let (r, g, b) = sconce_color(Theme::Outfall, seed);
            if seed % 5 == 0 {
                assert!(r > g && g > b && r > 200, "outfall warm fallback: {:?}", (r, g, b));
            } else {
                assert!(g > 220 && b > 220 && r < 140, "outfall cyan lens: {:?}", (r, g, b));
            }
            // One lamp in eight is spent.
            assert_eq!(sconce_spent(seed), seed.is_multiple_of(8));
            // The Printworks safelight is the only light in its room, so it has
            // to be seeing-bright even in the red.
            assert!(luminance(sconce_color(Theme::Printworks, 5)) > 100);
        }
    }

    /// Batch 12: a wall lamp's core, on the floor directly in front of it, is at
    /// least as bright as a hanging Pendant's at its own centre. It was a dimmer
    /// pool than the hanging lamps when the owner said the wall lights needed to
    /// be "WAAAAY brighter".
    #[test]
    fn a_wall_lamp_core_is_at_least_a_pendants() {
        let mut lamp = test_world(Theme::Sewer);
        let fixture = mount(&mut lamp.level, 4, 2, 3);
        let wall_lit = lit(&lamp);

        let mut hanging = test_world(Theme::Sewer);
        hanging.level.objects.lights.push(LightFixture {
            pos: TileMap::tile_rect(4, 3).center(),
            kind: LightFixtureKind::Pendant,
            radius: 88.0,
            // Not a multiple of four: no secondary tint, no flicker.
            seed: 1,
        });
        let pendant_lit = lit(&hanging);

        // The sample that shades a pixel sits on the 4 px light grid, so both
        // readings are within a few pixels of the source.
        let x = fixture.pos.x.round() as usize;
        let core = luminance(wall_lit.pixel_at(x, 53));
        let pendant = luminance(pendant_lit.pixel_at(x, 52));
        assert!(
            core >= pendant,
            "wall lamp core {core} is dimmer than a pendant's {pendant}"
        );
        // ...and it is not merely at the pendant's level: the mount is hot.
        assert!(core > 140, "wall lamp core {core}");
    }

    /// The fan is still light eighty pixels into the room, and it fades: the
    /// old 46 px radius put nothing at all there.
    #[test]
    fn a_wall_lamp_is_still_light_eighty_pixels_out() {
        let mut lamp = test_world(Theme::Sewer);
        let fixture = mount(&mut lamp.level, 4, 2, 3);
        let wall_lit = lit(&lamp);
        let bare = test_world(Theme::Sewer);
        let dark = lit(&bare);
        let x = fixture.pos.x.round() as usize;
        // About 2 px, 62 px and 81 px in front of the mount (its pos is y 48.5);
        // those pixel rows sit on the light grid's own sample points.
        let (near, mid, far) = (50usize, 110, 130);
        let l = |pb: &PixelBuffer, y: usize| luminance(pb.pixel_at(x, y));
        assert!(
            l(&wall_lit, far) > l(&dark, far) + 20,
            "no light at 80 px: {} against the floor's {}",
            l(&wall_lit, far),
            l(&dark, far)
        );
        assert!(l(&wall_lit, near) > l(&wall_lit, mid));
        assert!(l(&wall_lit, mid) > l(&wall_lit, far));
    }

    /// The lamp's wall still stops it: floor on the far side of the mount's own
    /// one-cell wall is exactly the darkened floor, not a tinted one.
    #[test]
    fn a_wall_lamp_lights_nothing_behind_its_wall() {
        let mut lamp = test_world(Theme::Sewer);
        let fixture = mount(&mut lamp.level, 4, 2, 3);
        let wall_lit = lit(&lamp);
        let bare = test_world(Theme::Sewer);
        let dark = lit(&bare);
        let x = fixture.pos.x.round() as usize;
        // The wall is the row at tile y = 2 (world y 32..48); these samples are
        // the open floor above it.
        for y in [20usize, 24, 28] {
            assert_eq!(
                luminance(wall_lit.pixel_at(x, y)),
                luminance(dark.pixel_at(x, y)),
                "light leaked through the wall at y {y}"
            );
        }
    }

    #[test]
    fn blackout_extinguishes_fixture_lenses_pools_and_sconce_wash() {
        let mut w = test_world(Theme::Sewer);
        let sconce = mount(&mut w.level, 4, 2, 3);
        let pendant = LightFixture {
            pos: Vec2::new(128.0, 112.0),
            kind: LightFixtureKind::Pendant,
            radius: 46.0,
            seed: 1,
        };
        w.level.objects.lights.push(pendant);
        w.elapsed = 299.0;
        let before = lit(&w);
        w.elapsed = 304.0;
        let dark = lit(&w);
        let fixtures = std::mem::take(&mut w.level.objects.lights);
        let bare = lit(&w);
        w.level.objects.lights = fixtures;
        w.elapsed = 310.0;
        let after = lit(&w);

        for (x, y) in [(72, 53), (72, 64), (128, 116)] {
            assert_eq!(dark.pixel_at(x, y), bare.pixel_at(x, y));
            assert!(luminance(before.pixel_at(x, y)) > luminance(dark.pixel_at(x, y)));
            assert!(luminance(after.pixel_at(x, y)) > luminance(dark.pixel_at(x, y)));
        }
        for fixture in [sconce, pendant] {
            let x = fixture.pos.x.round() as usize;
            let y = fixture.pos.y.round() as usize;
            assert!(luminance(before.pixel_at(x, y)) > luminance(dark.pixel_at(x, y)));
            assert!(luminance(after.pixel_at(x, y)) > luminance(dark.pixel_at(x, y)));
        }
    }

    /// Every housing is at least five pixels wide and every one of its pixels is
    /// marked emissive: the lighting pass can never darken the lamp itself.
    #[test]
    fn every_wall_lamp_housing_is_wide_and_never_darkened() {
        for theme in Theme::playable() {
            let h = housing(theme);
            assert!(h.width() >= 5, "{theme:?} housing {} px", h.width());
            assert!(h.width() <= 7, "{theme:?} housing {} px", h.width());
            assert!(h.lens <= h.half, "{theme:?} lens wider than its housing");
        }
        let mut lamp = test_world(Theme::Sewer);
        let fixture = mount(&mut lamp.level, 4, 2, 3);
        let mut pb = PixelBuffer::new(160, 160);
        pb.clear(FLOOR);
        let mut map = LightMap::new();
        draw_fixtures(&mut map, &mut pb, &lamp.level, Vec2::ZERO, 0.0, 0.0);
        let h = housing(Theme::Sewer);
        let (x, y) = (fixture.pos.x.round() as i32, fixture.pos.y.round() as i32);
        let drawn: Vec<(i32, i32, Rgb)> = (-h.half..=h.half)
            .flat_map(|dx| (-1..=1).map(move |dy| (x + dx, y + dy)))
            .map(|(px, py)| (px, py, pb.pixel_at(px as usize, py as usize)))
            .collect();
        // A seven-pixel housing is three rows of seven, and the lens is a bright
        // band across its middle: three lit pixels on every floor (the Sewer's
        // caged bulkhead among them).
        assert_eq!(drawn.len(), (h.width() * 3) as usize);
        assert!(
            drawn.iter().filter(|(_, _, c)| luminance(*c) > 150).count() >= 3,
            "the housing has no bright lens"
        );
        // Take the lamps off the map and run the pass again: only the emissive
        // mark keeps those pixels at their drawn value now.
        lamp.level.objects.lights.clear();
        map.apply_scene(&mut pb, &scene(&lamp));
        for (px, py, drawn) in drawn {
            let after = pb.pixel_at(px as usize, py as usize);
            assert!(
                after.0 >= drawn.0 && after.1 >= drawn.1 && after.2 >= drawn.2,
                "housing pixel ({px}, {py}) darkened: {drawn:?} -> {after:?}"
            );
        }
        // ...and the floor around it *was* darkened, so that is not vacuous.
        assert!(luminance(pb.pixel_at(60, 24)) < luminance(FLOOR));
    }

    #[test]
    fn a_snuffed_outfall_sconce_rejects_housing_emission_fan_and_wash() {
        let mut live = test_world(Theme::Outfall);
        let fixture = mount(&mut live.level, 4, 2, 3);
        let live_pb = lit(&live);

        let mut bare = test_world(Theme::Outfall);
        bare.level
            .tiles
            .fill_cells(3, 2, 3, 1, TileKind::Wall(WallKind::Solid));
        let bare_pb = lit(&bare);

        let x = fixture.pos.x.round() as usize;
        let y = fixture.pos.y.round() as usize;
        assert!(
            luminance(live_pb.pixel_at(x, y)) > 150,
            "live Outfall sconce lens must stay emissive"
        );
        assert!(
            luminance(live_pb.pixel_at(x, 60)) > luminance(bare_pb.pixel_at(x, 60)) + 25,
            "live Outfall sconce lights the floor in front of it"
        );

        // Snuff via elite slot, then boss slot: housing emission, fan and wash all go dark.
        for use_boss in [false, true] {
            let mut snuffed = test_world(Theme::Outfall);
            mount(&mut snuffed.level, 4, 2, 3);
            if use_boss {
                snuffed.level.objects.outfall_boss_snuff = Some(0);
            } else {
                snuffed.level.objects.outfall_elite_snuff = Some(0);
            }
            let snuffed_pb = lit(&snuffed);
            assert!(
                luminance(snuffed_pb.pixel_at(x, y)) < 60,
                "snuffed lens must not stay emissive (boss={use_boss})"
            );
            assert_eq!(
                luminance(snuffed_pb.pixel_at(x, 60)),
                luminance(bare_pb.pixel_at(x, 60)),
                "snuffed sconce must emit no fan or wash (boss={use_boss})"
            );
        }
    }
}
