//! Boss **floor telegraphs**: one generic outline pass over `Boss::hazards`.
//!
//! Every boss queues its attacks as `Boss.hazards` (`HazardShape` + a warn and
//! an active window) and paints them inside its own `draw_ground`. That art is
//! *under* the lighting pass, so in a dark room — and at any zoom-out, where the
//! resampler can average a one-pixel rim away — the most dangerous marks in the
//! game were the least legible hostile information left on screen (the
//! readability merge covered bullets, enemy tells, miniboss telegraphs and boss
//! projectiles, not these).
//!
//! So this module is one **generic** pass, in two halves:
//!
//! - [`draw_ground`] runs inside `World::draw_hostile_layer`, i.e. *after* the
//!   lighting pass, so the outline is self-lit and cannot be dimmed by the
//!   ambient or hidden by a friendly burst. It outlines the shape's own
//!   boundary from its shape data: **dashed** in the boss's warning colour
//!   while the warn window runs, **solid and brighter** while it is active.
//!   The boss's ground art stays underneath exactly as it was.
//! - [`draw_ink`] runs in `render::ink` **only when the view is zoomed out**
//!   (`scale > ZOOMED_OUT`), the same rule the hostile layer's other marks use,
//!   so nothing is doubled at 1x. It re-draws the same boundary with a minimum
//!   screen thickness, so the mark survives the crisp resampler at 2x.
//!
//! Both halves are render-only: they read `Boss.hazards` and write pixels, draw
//! no rng and mutate no simulation state, so the goldens cannot move.

use crate::game::ecs::systems::boss::{Boss, BossKind, HazardShape};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// Most points one hazard boundary polyline can hold (an arc is the longest:
/// 48 outer samples, 48 inner, plus the two radial edges).
pub const MAX_PTS: usize = 128;

/// The boss's own warning colour: amber while the tell runs, hot red once the
/// shape is live (the ramp every boss's tell already uses — see the lighting
/// pass's per-hazard pool, which picks the same pair).
#[inline]
pub fn warn_colour(kind: BossKind, active: bool) -> Rgb {
    let (warn, hot) = match kind {
        BossKind::Cat | BossKind::Broom | BossKind::Mower | BossKind::Mangle => {
            ((255, 186, 64), (255, 72, 62))
        }
        // The press works under a cold compositor's lamp: its tell is a little
        // whiter than the kitchen's.
        BossKind::NightEdition => ((255, 196, 78), (255, 96, 64)),
        // The bird's own warm plumage.
        BossKind::Magpie => ((255, 178, 92), (255, 86, 74)),
        // Cold drain-light on wet shell.
        BossKind::HorseshoeCrab => ((255, 198, 116), (255, 84, 70)),
        // The first success fights under the kitchen's own warm ramp.
        BossKind::MegaRat => ((255, 186, 64), (255, 72, 62)),
        // A raptor's talon print on oak: warmer and browner than the horde's.
        BossKind::Hawk => ((255, 198, 118), (255, 96, 66)),
        // Venom on a cold tile: the same amber tell, a greener hot beat so a
        // live puddle is never mistaken for the loop's clock.
        BossKind::Cobra => ((255, 196, 92), (210, 255, 96)),
    };
    if active {
        hot
    } else {
        warn
    }
}

/// The closed boundary of `shape` in world space, written into `out`.
/// Returns the number of points used.
pub fn boundary(shape: &HazardShape, out: &mut [Vec2; MAX_PTS]) -> usize {
    let mut n = 0usize;
    let mut push = |p: Vec2| {
        if n < MAX_PTS {
            out[n] = p;
            n += 1;
        }
    };
    // Samples around a circle: 1.5 per pixel of radius keeps a big hazard's
    // outline continuous without spending thousands of plots.
    let samples = |radius: f32| ((radius * 1.5).ceil() as usize).clamp(12, 48);
    match *shape {
        HazardShape::Circle { center, radius } => {
            let steps = samples(radius);
            for i in 0..steps {
                let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                push(center + Vec2::from_angle(a) * radius);
            }
        }
        HazardShape::Annulus {
            center,
            inner,
            outer,
        } => {
            // Both rims, as one closed run: outer forwards, inner backwards.
            let steps = samples(outer);
            for i in 0..steps {
                let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                push(center + Vec2::from_angle(a) * outer);
            }
            let steps_in = samples(inner);
            for i in (0..steps_in).rev() {
                let a = i as f32 / steps_in as f32 * std::f32::consts::TAU;
                push(center + Vec2::from_angle(a) * inner);
            }
        }
        HazardShape::Arc {
            center,
            inner,
            outer,
            facing,
            half_angle,
        } => {
            let start = facing.angle() - half_angle;
            let steps = ((half_angle * outer).ceil() as usize).clamp(6, 48);
            for i in 0..=steps {
                let a = start + half_angle * 2.0 * (i as f32 / steps as f32);
                push(center + Vec2::from_angle(a) * outer);
            }
            if inner > 0.5 {
                for i in (0..=steps).rev() {
                    let a = start + half_angle * 2.0 * (i as f32 / steps as f32);
                    push(center + Vec2::from_angle(a) * inner);
                }
            } else {
                push(center);
            }
        }
        HazardShape::Rect {
            center,
            facing,
            half,
            half_perp,
        } => {
            let u = facing * half;
            let v = facing.perp() * half_perp;
            for corner in [u + v, u - v, -u - v, -u + v] {
                push(center + corner);
            }
        }
        HazardShape::Corridor {
            from,
            to,
            half_width,
        } => {
            let axis = to - from;
            let dir = if axis.len_sq() > 1e-6 {
                axis.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            };
            let side = dir.perp() * half_width;
            // A rounded end cap either side, so a corridor reads as a lane and
            // not as a rectangle that stops dead.
            let cap = ((half_width * 1.5).ceil() as usize).clamp(4, 16);
            for corner in [from + side, to + side] {
                push(corner);
            }
            for i in 1..cap {
                let a = i as f32 / cap as f32 * std::f32::consts::PI;
                push(to + Vec2::from_angle(a) * half_width);
            }
            for corner in [to - side, from - side] {
                push(corner);
            }
            for i in 1..cap {
                let a = std::f32::consts::PI + i as f32 / cap as f32 * std::f32::consts::PI;
                push(from + Vec2::from_angle(a) * half_width);
            }
        }
    }
    n
}

/// World-space, self-lit outline of every live hazard the boss owns.
///
/// Called from `World::draw_hostile_layer` (after the lighting pass), so these
/// pixels are never darkened and never covered by a friendly effect.
pub fn draw_ground(pb: &mut PixelBuffer, boss: &Boss, cam: Vec2, elapsed: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let mut pts = [Vec2::ZERO; MAX_PTS];
    for hazard in &boss.hazards {
        if hazard.is_expired() {
            continue;
        }
        let n = boundary(&hazard.shape, &mut pts);
        if n < 2 {
            continue;
        }
        let active = hazard.is_active();
        let colour = warn_colour(boss.kind, active);
        // The dashes crawl along the boundary while the tell runs, and the
        // live window is solid: "this is about to happen" vs "this is happening".
        let phase = (elapsed * 10.0) as i32;
        for i in 0..n {
            plot_edge(
                pb,
                pts[i],
                pts[(i + 1) % n],
                cam,
                colour,
                if active { 0 } else { 2 },
                phase + i as i32 * 3,
            );
        }
    }
}

/// Screen-space outline with a minimum thickness, for the zoomed-out view.
///
/// `scale` is source (world) pixels per screen pixel and `unit` one world pixel
/// in screen pixels, exactly as `render::ink` defines them.
pub fn draw_ink(
    pb: &mut PixelBuffer,
    boss: &Boss,
    cam: Vec2,
    scale: Vec2,
    unit: f32,
    elapsed: f32,
) {
    let mut pts = [Vec2::ZERO; MAX_PTS];
    let mut screen = [(0i32, 0i32); MAX_PTS];
    let thickness = ((1.0 / unit.max(0.01)).round() as i32).clamp(1, 3);
    for hazard in &boss.hazards {
        if hazard.is_expired() {
            continue;
        }
        let n = boundary(&hazard.shape, &mut pts);
        if n < 2 {
            continue;
        }
        for i in 0..n {
            screen[i] = (
                ((pts[i].x - cam.x) / scale.x).round() as i32,
                ((pts[i].y - cam.y) / scale.y).round() as i32,
            );
        }
        let active = hazard.is_active();
        let colour = warn_colour(boss.kind, active);
        let phase = (elapsed * 10.0) as i32;
        for i in 0..n {
            let a = screen[i];
            let b = screen[(i + 1) % n];
            if active {
                thick_line(pb, a, b, colour, thickness);
            } else {
                dashed_line(pb, a, b, colour, thickness, phase + i as i32);
            }
        }
    }
}

// ---- Small drawing helpers ------------------------------------------------

/// Plot one world-space edge, `dash` px on / `dash` px off (`0` = solid).
fn plot_edge(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    cam: Vec2,
    colour: Rgb,
    dash: i32,
    phase: i32,
) {
    let (ax, ay) = ((a.x - cam.x).round() as i32, (a.y - cam.y).round() as i32);
    let (bx, by) = ((b.x - cam.x).round() as i32, (b.y - cam.y).round() as i32);
    let n = (bx - ax).abs().max((by - ay).abs()).max(1);
    for i in 0..=n {
        if dash > 0 && (i + phase).rem_euclid(dash * 2) >= dash {
            continue;
        }
        let x = ax + ((bx - ax) as f32 * i as f32 / n as f32).round() as i32;
        let y = ay + ((by - ay) as f32 * i as f32 / n as f32).round() as i32;
        pb.plot(x, y, colour);
    }
}

/// A dashed screen-space line `w` pixels thick.
fn dashed_line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, w: i32, phase: i32) {
    let thickness = w.max(1);
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let n = dx.abs().max(dy.abs()).max(1);
    let normal = ((dy as f32).atan2(dx as f32) + std::f32::consts::FRAC_PI_2).cos();
    let normal_y = ((dy as f32).atan2(dx as f32) + std::f32::consts::FRAC_PI_2).sin();
    for offset in 0..thickness {
        let ox = (normal * offset as f32).round() as i32;
        let oy = (normal_y * offset as f32).round() as i32;
        for i in 0..=n {
            if (i + phase).rem_euclid(2) != 0 {
                continue;
            }
            let x = a.0 + ox + ((dx as f32 * i as f32) / n as f32).round() as i32;
            let y = a.1 + oy + ((dy as f32 * i as f32) / n as f32).round() as i32;
            pb.plot(x, y, c);
        }
    }
}

/// A solid screen-space line `w` pixels thick.
fn thick_line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, w: i32) {
    let thickness = w.max(1);
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let n = dx.abs().max(dy.abs()).max(1);
    let angle = (dy as f32).atan2(dx as f32) + std::f32::consts::FRAC_PI_2;
    let (nx, ny) = (angle.cos(), angle.sin());
    for offset in 0..thickness {
        let ox = (nx * offset as f32).round() as i32;
        let oy = (ny * offset as f32).round() as i32;
        for i in 0..=n {
            let x = a.0 + ox + ((dx as f32 * i as f32) / n as f32).round() as i32;
            let y = a.1 + oy + ((dy as f32 * i as f32) / n as f32).round() as i32;
            pb.plot(x, y, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ecs::systems::boss::{BossHazard, BossKind};
    use crate::game::level::Theme;
    use crate::game::world::World;
    use crate::render::lighting::LightingMode;

    /// One representative hazard per shape, sized like the real bosses' ones.
    fn shapes() -> Vec<HazardShape> {
        vec![
            HazardShape::Circle {
                center: Vec2::new(90.0, 74.0),
                radius: 18.0,
            },
            HazardShape::Annulus {
                center: Vec2::new(90.0, 74.0),
                inner: 12.0,
                outer: 26.0,
            },
            HazardShape::Arc {
                center: Vec2::new(90.0, 74.0),
                inner: 6.0,
                outer: 30.0,
                facing: Vec2::new(1.0, 0.0),
                half_angle: 0.6,
            },
            HazardShape::Rect {
                center: Vec2::new(90.0, 74.0),
                facing: Vec2::new(1.0, 0.0),
                half: 20.0,
                half_perp: 8.0,
            },
            HazardShape::Corridor {
                from: Vec2::new(60.0, 74.0),
                to: Vec2::new(120.0, 74.0),
                half_width: 5.0,
            },
        ]
    }

    fn boss_with(kind: BossKind, hazard: BossHazard) -> Boss {
        let arena = crate::game::level::TileRect::new(0, 0, 25, 19);
        let mut boss = Boss::new(kind, Vec2::new(90.0, 74.0), 900.0, arena, Vec2::ZERO, false);
        assert!(boss.push_hazard(hazard), "the ring has room");
        boss
    }

    /// Lum of a world pixel, 0 off the buffer.
    fn lum(pb: &PixelBuffer, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
            return 0;
        }
        let (r, g, b) = pb.pixel_at(x as usize, y as usize);
        u32::from(r) + u32::from(g) + u32::from(b)
    }

    /// Borrow `World::draw`'s exact render path: the whole frame, lighting
    /// included. `darkest` is the darkest theme with lighting **Full**.
    fn render(w: &mut World, zoom: f32) -> PixelBuffer {
        let mut pb = PixelBuffer::new(160, 148);
        w.set_zoom_instant(zoom);
        w.draw(&mut pb);
        pb
    }

    /// **Every boss, at 1x and 2x, with lighting Full in the darkest theme: a
    /// queued hazard is drawn as a lit outline** (the review's "the only hostile
    /// information still drawn under the lighting").
    ///
    /// The check is that the warning colour appears **on the shape's own
    /// boundary**, in the frame the world pass produced, with lighting on: a
    /// pixel is only that exact colour if the outline was stamped after the
    /// ambient pass.
    #[test]
    fn every_boss_hazard_gets_a_lit_outline_at_1x_and_2x() {
        let themes = [Theme::Sewer, Theme::Lab, Theme::Printworks];
        let kinds = [
            BossKind::Cat,
            BossKind::Broom,
            BossKind::Mower,
            BossKind::Mangle,
            BossKind::NightEdition,
            BossKind::Magpie,
        ];
        for kind in kinds {
            for theme in themes {
                for zoom in [1.0f32, 2.0] {
                    for active in [false, true] {
                        let shape = HazardShape::Circle {
                            center: Vec2::new(90.0, 74.0),
                            radius: 18.0,
                        };
                        let hazard = if active {
                            BossHazard::new(shape, 0.0, 2.0, 4.0)
                        } else {
                            BossHazard::new(shape, 0.8, 0.4, 4.0)
                        };
                        let boss = boss_with(kind, hazard);
                        let expect = warn_colour(kind, active);
                        let mut w = World::new(theme, 5);
                        w.lighting = LightingMode::Full;
                        w.player.pos = Vec2::new(90.0, 74.0);
                        w.clear_enemies();
                        crate::game::ecs::resources::spawn_with_id(&mut w.ecs, boss);
                        // The camera follows the rat; one draw settles it.
                        let _ = render(&mut w, zoom);
                        let pb = render(&mut w, zoom);
                        // Sample the drawn boundary in world space.
                        let mut pts = [Vec2::ZERO; MAX_PTS];
                        let n = boundary(&shape, &mut pts);
                        let scale = w.zoom_scale;
                        let mut hits = 0usize;
                        let mut samples = 0usize;
                        for &p in pts.iter().take(n) {
                            let x = ((p.x - w.cam.x) / scale.x).round() as i32;
                            let y = ((p.y - w.cam.y) / scale.y).round() as i32;
                            samples += 1;
                            // The dashed warn outline skips pixels, so look in
                            // a 2 px window (and at 2x the box resolved one
                            // pixel per screen pixel).
                            let mut found = false;
                            for dy in -1..=1 {
                                for dx in -1..=1 {
                                    if lum(&pb, x + dx, y + dy) >= 100
                                        && pixel_is(&pb, x + dx, y + dy, expect)
                                    {
                                        found = true;
                                    }
                                }
                            }
                            if found {
                                hits += 1;
                            }
                        }
                        assert!(
                            hits * 2 >= samples,
                            "{kind:?} in {theme:?} at z={zoom} (active {active}): only \
                             {hits}/{samples} boundary samples are the warning colour"
                        );
                    }
                }
            }
        }
    }

    /// Is the pixel exactly this colour (an unlit, unblended stamp)?
    fn pixel_is(pb: &PixelBuffer, x: i32, y: i32, c: Rgb) -> bool {
        if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
            return false;
        }
        pb.pixel_at(x as usize, y as usize) == c
    }

    /// **Nothing is doubled at 1x.** The ink pass stamps hostile marks only when
    /// the view is zoomed out, so at 1:1 the world pass's outline is the only
    /// one. This drives the real call site (`render::ink::draw`) at both scales.
    #[test]
    fn the_ink_outline_is_only_drawn_when_zoomed_out() {
        use crate::render::ink::{draw as ink_draw, InkView};

        let shape = HazardShape::Circle {
            center: Vec2::new(80.0, 74.0),
            radius: 18.0,
        };
        let boss = boss_with(BossKind::Cat, BossHazard::new(shape, 0.8, 0.4, 4.0));
        let render = |scale: Vec2| {
            let mut w = World::new(Theme::Sewer, 5);
            w.player.pos = Vec2::new(60.0, 74.0);
            crate::game::ecs::resources::spawn_with_id(&mut w.ecs, boss.clone());
            let mut pb = PixelBuffer::new(160, 148);
            pb.clear((0, 0, 0));
            let mut view = InkView {
                ecs: &mut w.ecs,
                level: &w.level,
                player: &w.player,
                cam: Vec2::ZERO,
                scale,
                size: (160, 148),
                elapsed: 0.4,
            };
            ink_draw(&mut pb, &mut view);
            
            (0..148usize)
                .flat_map(|y| (0..160usize).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let (r, g, b) = pb.pixel_at(x, y);
                    u32::from(r) + u32::from(g) + u32::from(b) > 0
                })
                .count()
        };
        assert_eq!(render(Vec2::new(1.0, 1.0)), 0, "the ink pass stamped the outline at 1x");
        assert!(
            render(Vec2::new(2.0, 2.0)) > 20,
            "the 2x ink outline drew almost nothing"
        );
    }
}
