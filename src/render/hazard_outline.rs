//! Boss **floor telegraphs**: one generic outline pass over `Boss::hazards`.
//!
//! Every boss queues its attacks as `Boss.hazards` (`HazardShape` + a warn and
//! an active window) and paints them inside its own `draw_ground`. That art is
//! *under* the lighting pass, so in a dark room — and at any zoom-out, where the
//! resampler keeps one pixel per box and can drop a dim one-pixel rim — the most dangerous marks in the
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
use crate::game::ecs::systems::miniboss::{Miniboss, MinibossShape, MinibossState, SignatureShape};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// Most points one hazard boundary polyline can hold (an arc or an annulus is
/// the longest: 49 outer points and 49 inner).
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
        BossKind::Lantern => ((255, 188, 120), (255, 244, 224)),
    };
    if active {
        hot
    } else {
        warn
    }
}

/// Samples around a circle of `radius`: 1.5 per pixel of radius keeps a big
/// hazard's outline continuous without spending thousands of plots.
#[inline]
fn circle_samples(radius: f32) -> usize {
    ((radius * 1.5).ceil() as usize).clamp(12, 48)
}

/// Is edge `i → i + 1` of an `n`-point [`boundary`] a **seam**, i.e. a join
/// between two separate loops that must not be drawn?
///
/// Only the annulus has one: its boundary is two closed rings stored as one
/// run (outer, then inner), so the edge leaving each ring would be a radial
/// spoke straight across the band the player is meant to read.
#[inline]
fn is_seam(shape: &HazardShape, i: usize, n: usize) -> bool {
    match *shape {
        HazardShape::Annulus { outer, .. } => {
            let outer_pts = circle_samples(outer) + 1;
            i + 1 == outer_pts || i + 1 == n
        }
        _ => false,
    }
}

/// The closed boundary of `shape` in world space, written into `out`.
/// Returns the number of points used.
///
/// Consecutive points (wrapping at the end) are the outline's edges, except
/// the annulus's two seam edges (see [`is_seam`]).
pub fn boundary(shape: &HazardShape, out: &mut [Vec2; MAX_PTS]) -> usize {
    let mut n = 0usize;
    let mut push = |p: Vec2| {
        if n < MAX_PTS {
            out[n] = p;
            n += 1;
        }
    };
    let samples = circle_samples;
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
            // Two closed rings in one run: outer forwards, then inner
            // backwards. Each ring repeats its first point at the end so it
            // closes on itself; the two joins between the rings are seams
            // (`is_seam`) and are never drawn. (They used to be: the outline
            // had a spoke across the band at angle 0 and a gap in each rim.)
            let steps = samples(outer);
            for i in 0..=steps {
                let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                push(center + Vec2::from_angle(a) * outer);
            }
            let steps_in = samples(inner);
            for i in (0..=steps_in).rev() {
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
            if is_seam(&hazard.shape, i, n) {
                continue;
            }
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
    for hazard in &boss.hazards {
        if !hazard.is_expired() {
            let active = hazard.is_active();
            draw_shape_ink(
                pb, &hazard.shape, cam, scale, unit, elapsed,
                warn_colour(boss.kind, active), active,
            );
        }
    }
}

/// Re-stamp a published combat footprint at the ink pass's pixel scale.
#[allow(clippy::too_many_arguments)]
pub fn draw_shape_ink(
    pb: &mut PixelBuffer,
    shape: &HazardShape,
    cam: Vec2,
    scale: Vec2,
    unit: f32,
    elapsed: f32,
    colour: Rgb,
    active: bool,
) {
    let mut pts = [Vec2::ZERO; MAX_PTS];
    let n = boundary(shape, &mut pts);
    if n < 2 {
        return;
    }
    let thickness = ((1.0 / unit.max(0.01)).round() as i32).clamp(1, 3);
    let phase = (elapsed * 10.0) as i32;
    let screen = |p: Vec2| (
        ((p.x - cam.x) / scale.x).round() as i32,
        ((p.y - cam.y) / scale.y).round() as i32,
    );
    for i in 0..n {
        if is_seam(shape, i, n) {
            continue;
        }
        let a = screen(pts[i]);
        let b = screen(pts[(i + 1) % n]);
        if active {
            thick_line(pb, a, b, colour, thickness);
        } else {
            dashed_line(pb, a, b, colour, thickness, phase + i as i32);
        }
    }
}

// ---- Miniboss footprints: the swept cord, the pincer, the rollers ----------
//
// A miniboss tell must mark **every place the strike will pass through**. For
// three shapes that is not the shape the simulation hands the art for one
// instant: a cord is a line that swings (`SignatureShape::Cord::sweep`), a
// pincer is two claws that close (`gap_hi → gap_lo`) and the rollers are two
// bars that travel a lane. This half outlines the whole footprint of those
// three from the same row data the hit test reads, with the same self-lit
// dashed-then-solid language as the boss outlines above. The other miniboss
// shapes are static (their tell *is* their footprint) and stay with
// `art_miniboss::draw_telegraph` and `render::ink`.

//
// NOT WIRED YET: `World::draw_hostile_layer` (world.rs) and `render::ink` were
// off limits when this was written, so nothing calls the two draw functions
// outside this module's tests. The call sites are one line each, next to the
// boss calls: `hazard_outline::draw_miniboss_ground(pb, mb, cam, elapsed)` and
// `hazard_outline::draw_miniboss_ink(pb, mb, cam, scale, unit, elapsed)`.

/// Most closed loops one miniboss footprint needs (two claws / two rollers).
#[allow(dead_code)]
pub const MAX_LOOPS: usize = 2;

/// The miniboss telegraph colours: the ink pass's amber while the tell runs,
/// hot once the strike is live.
#[allow(dead_code)]
#[inline]
pub fn miniboss_colour(active: bool) -> Rgb {
    if active {
        (255, 96, 72)
    } else {
        (255, 188, 120)
    }
}

/// The whole swept footprint of a miniboss's signature, as up to
/// [`MAX_LOOPS`] closed loops written into `out`; `loops[k]` is the
/// `(start, len)` of loop `k`. Returns the loop count — `0` for a miniboss that
/// is not telling/striking and for the shapes this pass does not own.
///
/// * **Cord** — the sector the cord swings through: the pivot, then an arc of
///   radius `len + half` from `-sweep/2` to `+sweep/2` about the locked aim,
///   widened at each end by the cord's own half-thickness.
/// * **Pincer** — one band per claw, from the closed position's inner edge to
///   the open position's outer edge, `len` long.
/// * **Lane** — one rail per roller, `seam_half .. seam_half + 2·half` either
///   side of the aim, `len` long; the seam between them stays open.
#[allow(dead_code)]
pub fn miniboss_footprint(
    mb: &Miniboss,
    out: &mut [Vec2; MAX_PTS],
    loops: &mut [(usize, usize); MAX_LOOPS],
) -> usize {
    if mb.telegraph().is_none() {
        return 0;
    }
    let dir = if mb.aim.len_sq() > 1e-6 {
        mb.aim.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let side = dir.perp();
    let mut n = 0usize;
    let mut push = |p: Vec2| {
        if n < MAX_PTS {
            out[n] = p;
            n += 1;
        }
    };
    // The sweep lives on the signature row, not on the per-instant shape.
    if let SignatureShape::Cord { len, half, sweep } = mb.signature().shape {
        let reach = len + half;
        // The cord is `2·half` thick, so its edge leads the centre line by
        // this much at the tip — the same widening `MinibossKind::telegraph`
        // gives the cord's tell wedge, so the two agree.
        let pad = (half / len.max(1.0)).clamp(0.0, 1.0).asin();
        let half_sweep = (sweep * 0.5 + pad).min(std::f32::consts::PI);
        let start = dir.angle() - half_sweep;
        let steps = ((half_sweep * reach).ceil() as usize).clamp(6, 48);
        push(mb.origin);
        for i in 0..=steps {
            let a = start + half_sweep * 2.0 * (i as f32 / steps as f32);
            push(mb.origin + Vec2::from_angle(a) * reach);
        }
        loops[0] = (0, n);
        return 1;
    }
    // Two mirrored bands astride the aim: lateral `lo..hi`, `0..len` along it.
    let bands = match mb.kind.telegraph(mb) {
        MinibossShape::Pincer {
            len,
            half,
            gap_lo,
            gap_hi,
            ..
        } => Some((len, (gap_lo.max(0.0) * 0.5 - half).max(0.0), gap_hi * 0.5 + half)),
        MinibossShape::Lane {
            len,
            seam_half,
            half,
            ..
        } => Some((len, seam_half, seam_half + half * 2.0)),
        _ => None,
    };
    let Some((len, lo, hi)) = bands else {
        return 0;
    };
    for sign in [-1.0f32, 1.0] {
        for (along, across) in [(0.0, lo), (len, lo), (len, hi), (0.0, hi)] {
            push(mb.origin + dir * along + side * (across * sign));
        }
    }
    loops[0] = (0, 4);
    loops[1] = (4, 4);
    2
}

/// Is the miniboss's strike live (solid, hot) rather than telling (dashed)?
#[allow(dead_code)]
#[inline]
fn miniboss_live(mb: &Miniboss) -> bool {
    mb.state == MinibossState::Active
}

/// World-space, self-lit outline of a miniboss's swept footprint (cord sector,
/// pincer travel, roller lane). Belongs beside [`draw_ground`] in
/// `World::draw_hostile_layer`, after the lighting pass. Draws nothing for
/// the other shapes. Render-only.
#[allow(dead_code)]
pub fn draw_miniboss_ground(pb: &mut PixelBuffer, mb: &Miniboss, cam: Vec2, elapsed: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let mut pts = [Vec2::ZERO; MAX_PTS];
    let mut loops = [(0usize, 0usize); MAX_LOOPS];
    let count = miniboss_footprint(mb, &mut pts, &mut loops);
    let active = miniboss_live(mb);
    let colour = miniboss_colour(active);
    let phase = (elapsed * 10.0) as i32;
    for &(start, len) in loops.iter().take(count) {
        if len < 2 {
            continue;
        }
        for i in 0..len {
            plot_edge(
                pb,
                pts[start + i],
                pts[start + (i + 1) % len],
                cam,
                colour,
                if active { 0 } else { 2 },
                phase + i as i32 * 3,
            );
        }
    }
}

/// Screen-space twin of [`draw_miniboss_ground`] with a minimum thickness, for
/// the zoomed-out view (`render::ink`, `scale > ZOOMED_OUT`). `scale` and
/// `unit` are as in [`draw_ink`].
#[allow(dead_code)]
pub fn draw_miniboss_ink(
    pb: &mut PixelBuffer,
    mb: &Miniboss,
    cam: Vec2,
    scale: Vec2,
    unit: f32,
    elapsed: f32,
) {
    let mut pts = [Vec2::ZERO; MAX_PTS];
    let mut loops = [(0usize, 0usize); MAX_LOOPS];
    let count = miniboss_footprint(mb, &mut pts, &mut loops);
    let thickness = ((1.0 / unit.max(0.01)).round() as i32).clamp(1, 3);
    let active = miniboss_live(mb);
    let colour = miniboss_colour(active);
    let phase = (elapsed * 10.0) as i32;
    let to_screen = |p: Vec2| {
        (
            ((p.x - cam.x) / scale.x).round() as i32,
            ((p.y - cam.y) / scale.y).round() as i32,
        )
    };
    for &(start, len) in loops.iter().take(count) {
        if len < 2 {
            continue;
        }
        for i in 0..len {
            let a = to_screen(pts[start + i]);
            let b = to_screen(pts[start + (i + 1) % len]);
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
    use crate::game::ecs::systems::miniboss::MinibossKind;
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

    /// The annulus outline is two closed rings and nothing else: every drawn
    /// edge stays on one rim, each rim closes on itself, and no spoke crosses
    /// the band (the old run joined the rims at angle 0 and left a gap in each).
    #[test]
    fn an_annulus_outline_is_two_closed_rings_with_no_spoke() {
        let center = Vec2::new(90.0, 74.0);
        for (inner, outer) in [(12.0f32, 26.0f32), (4.0, 9.0), (30.0, 60.0)] {
            let shape = HazardShape::Annulus { center, inner, outer };
            let mut pts = [Vec2::ZERO; MAX_PTS];
            let n = boundary(&shape, &mut pts);
            assert!(n < MAX_PTS, "the run must not be clipped: {n}");
            let radius = |p: Vec2| ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt();
            let mut seams = 0;
            for i in 0..n {
                if is_seam(&shape, i, n) {
                    seams += 1;
                    continue;
                }
                let (a, b) = (radius(pts[i]), radius(pts[(i + 1) % n]));
                assert!((a - b).abs() < 0.01, "edge {i} crosses the band: {a} -> {b}");
            }
            assert_eq!(seams, 2, "exactly the two joins between the rims are skipped");
            // Each rim closes: the points either side of a seam are a ring's
            // last point, which repeats that ring's first.
            let outer_pts = circle_samples(outer) + 1;
            let close = |a: Vec2, b: Vec2| (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01;
            assert!(close(pts[0], pts[outer_pts - 1]), "the outer rim is open");
            assert!(close(pts[outer_pts], pts[n - 1]), "the inner rim is open");
        }

        // And on pixels: a live annulus lights both rims at angle 0 and leaves
        // the middle of the band there dark.
        let shape = HazardShape::Annulus { center, inner: 12.0, outer: 26.0 };
        let boss = boss_with(BossKind::Cat, BossHazard::new(shape, 0.0, 2.0, 4.0));
        let mut pb = PixelBuffer::new(160, 148);
        draw_ground(&mut pb, &boss, Vec2::ZERO, 0.0);
        assert!(lum(&pb, 116, 74) > 0, "the outer rim is missing at angle 0");
        assert!(lum(&pb, 102, 74) > 0, "the inner rim is missing at angle 0");
        for x in 105..=113 {
            assert_eq!(lum(&pb, x, 74), 0, "a spoke crosses the band at x={x}");
        }
    }

    /// A miniboss mid-tell (or mid-strike) at `origin`, aimed along `aim`.
    fn miniboss(kind: MinibossKind, state: MinibossState, origin: Vec2, aim: Vec2) -> Miniboss {
        let mut mb = Miniboss::new(kind, 0);
        mb.state = state;
        mb.origin = origin;
        mb.aim = aim;
        mb.target = origin + aim * 20.0;
        mb
    }

    /// Even-odd point-in-polygon over one footprint loop.
    fn inside(pts: &[Vec2], p: Vec2) -> bool {
        let mut hit = false;
        let n = pts.len();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if p.x < x {
                    hit = !hit;
                }
            }
        }
        hit
    }

    /// **The cord's tell is the whole sector it swings through**, not the one
    /// line it starts on: every cord row's tip and mid-point, at every moment
    /// of its strike, lies inside the outlined footprint, and a point behind
    /// the pivot does not.
    #[test]
    fn a_cord_footprint_covers_the_whole_sweep_and_nothing_behind_the_pivot() {
        let origin = Vec2::new(80.0, 74.0);
        let mut cords = 0;
        for kind in MinibossKind::all() {
            let sig = kind.signatures()[0];
            let SignatureShape::Cord { len, .. } = sig.shape else {
                continue;
            };
            cords += 1;
            for aim in [Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(-0.6, 0.8)] {
                let mb = miniboss(kind, MinibossState::Tell, origin, aim);
                let mut pts = [Vec2::ZERO; MAX_PTS];
                let mut loops = [(0usize, 0usize); MAX_LOOPS];
                assert_eq!(miniboss_footprint(&mb, &mut pts, &mut loops), 1, "{kind:?}");
                let (start, n) = loops[0];
                assert!((8..MAX_PTS).contains(&n), "{kind:?}: {n} points");
                let poly = &pts[start..start + n];
                for step in 0..=20 {
                    let t = sig.active * step as f32 / 20.0;
                    let Some(MinibossShape::Cord { spool, tip, .. }) = kind.active_shape(&mb, t) else {
                        panic!("{kind:?}: a cord row must sweep a cord at t={t}");
                    };
                    // Pull the samples a hair inside so float error on the
                    // boundary itself cannot flip the test.
                    let toward = spool + (tip - spool) * 0.97;
                    assert!(inside(poly, toward), "{kind:?} aim {aim:?} t={t}: the tip leaves the tell");
                    assert!(
                        inside(poly, spool + (tip - spool) * 0.5),
                        "{kind:?} aim {aim:?} t={t}: mid-cord leaves the tell"
                    );
                }
                let behind = origin - aim.normalized() * (len * 0.5);
                assert!(!inside(poly, behind), "{kind:?}: behind the pivot is safe and must stay unmarked");
            }
        }
        assert!(cords >= 1, "no cord rows left to test");
    }

    /// **Pincer and rollers: two bands astride the aim**, the centre line (the
    /// pincer's closed gap when it has one, the rollers' seam always) left
    /// open, drawn dashed in the tell colour and solid hot when live.
    #[test]
    fn pincer_and_lane_footprints_are_two_bands_with_the_seam_open() {
        let origin = Vec2::new(60.0, 74.0);
        let aim = Vec2::new(1.0, 0.0);
        for kind in [MinibossKind::Mudlark, MinibossKind::WringerBoy] {
            let mb = miniboss(kind, MinibossState::Tell, origin, aim);
            let mut pts = [Vec2::ZERO; MAX_PTS];
            let mut loops = [(0usize, 0usize); MAX_LOOPS];
            assert_eq!(miniboss_footprint(&mb, &mut pts, &mut loops), 2, "{kind:?}");
            let (lo, hi, len) = match kind.telegraph(&mb) {
                MinibossShape::Pincer { len, half, gap_lo, gap_hi, .. } => {
                    ((gap_lo.max(0.0) * 0.5 - half).max(0.0), gap_hi * 0.5 + half, len)
                }
                MinibossShape::Lane { len, seam_half, half, .. } => (seam_half, seam_half + half * 2.0, len),
                other => panic!("{kind:?} is not a pincer or a lane: {other:?}"),
            };
            let mid = (lo + hi) * 0.5;
            for (k, sign) in [(0usize, -1.0f32), (1, 1.0)] {
                let (start, n) = loops[k];
                assert_eq!(n, 4);
                let poly = &pts[start..start + n];
                assert!(inside(poly, origin + Vec2::new(len * 0.5, sign * mid)), "{kind:?}: band {k}");
                assert!(!inside(poly, origin + Vec2::new(len + 2.0, sign * mid)), "{kind:?}: past the reach");
                assert!(!inside(poly, origin + Vec2::new(len * 0.5, sign * (hi + 2.0))), "{kind:?}: outside");
                if lo > 1.0 {
                    assert!(!inside(poly, origin + Vec2::new(len * 0.5, 0.0)), "{kind:?}: the seam is open");
                }
            }

            // On pixels: tell colour while telling, hot while live, and both
            // long edges of the +side band are present.
            for (state, colour) in [
                (MinibossState::Tell, miniboss_colour(false)),
                (MinibossState::Active, miniboss_colour(true)),
            ] {
                let mb = miniboss(kind, state, origin, aim);
                let mut pb = PixelBuffer::new(160, 148);
                draw_miniboss_ground(&mut pb, &mb, Vec2::ZERO, 0.0);
                let on_row = |y: i32| (60..=60 + len as i32).filter(|&x| pixel_is(&pb, x, y, colour)).count();
                let outer = on_row(74 + hi.round() as i32);
                assert!(outer * 3 >= len as usize, "{kind:?} {state:?}: outer edge only {outer} px of {len}");
                if state == MinibossState::Active {
                    assert!(outer >= len as usize, "{kind:?}: a live edge is solid");
                }
            }
        }
    }

    /// Nothing is drawn for a miniboss that is not telling or striking, nor
    /// for the static shapes the art and ink passes already own; the ink twin
    /// draws the cord sector with at least the ground pass's coverage at 2x.
    #[test]
    fn the_miniboss_outline_is_only_the_swept_shapes_and_only_while_telling() {
        let origin = Vec2::new(80.0, 74.0);
        let aim = Vec2::new(1.0, 0.0);
        let lit = |pb: &PixelBuffer| pb.pixels().iter().filter(|c| **c != (0, 0, 0)).count();

        let idle = miniboss(MinibossKind::Bobbin, MinibossState::Approach, origin, aim);
        let mut pb = PixelBuffer::new(160, 148);
        draw_miniboss_ground(&mut pb, &idle, Vec2::ZERO, 0.0);
        draw_miniboss_ink(&mut pb, &idle, Vec2::ZERO, Vec2::new(2.0, 2.0), 0.5, 0.0);
        assert_eq!(lit(&pb), 0, "an approaching miniboss has no tell");

        // Corkscrew's trench is static: its tell is its footprint already.
        let trench = miniboss(MinibossKind::Corkscrew, MinibossState::Tell, origin, aim);
        let mut pb = PixelBuffer::new(160, 148);
        draw_miniboss_ground(&mut pb, &trench, Vec2::ZERO, 0.0);
        assert_eq!(lit(&pb), 0, "the trench belongs to the art pass");

        let cord = miniboss(MinibossKind::Bobbin, MinibossState::Active, origin, aim);
        let mut ground = PixelBuffer::new(160, 148);
        draw_miniboss_ground(&mut ground, &cord, Vec2::ZERO, 0.0);
        assert!(lit(&ground) > 60, "the cord sector drew almost nothing: {}", lit(&ground));
        let mut ink = PixelBuffer::new(160, 148);
        draw_miniboss_ink(&mut ink, &cord, Vec2::ZERO, Vec2::new(2.0, 2.0), 0.5, 0.0);
        assert!(lit(&ink) > 30, "the 2x ink sector drew almost nothing: {}", lit(&ink));
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
                hide_rat: false,
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
