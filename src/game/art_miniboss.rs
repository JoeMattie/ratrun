//! Placeholder miniboss silhouettes and telegraphs (wave 3).
//!
//! `WAVE3_DESIGN.md` §8 asks for distinct 12-20 px silhouettes and a telegraph
//! on the rat's side of the screen. Neon redraws these later; this module is
//! the binding geometry the simulation and the telegraph share, so a redraw
//! only changes pixels, never hit shapes.
//!
//! Every sprite is drawn from the body's world position with its facing, and
//! every telegraph is drawn on the ground layer (before props/entities) so the
//! rats's side of the shape is visibly announced one second before damage.

use crate::game::ecs::systems::miniboss::{
    Miniboss, MinibossKind, MinibossShape, MinibossState,
};
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Dark outline every silhouette shares.
const INK: Rgb = (28, 24, 30);
/// The body base colour per kind.
fn body_color(kind: MinibossKind) -> Rgb {
    match kind {
        MinibossKind::Corkscrew => (196, 152, 62),
        MinibossKind::SisterSnip => (210, 214, 228),
        MinibossKind::Bobbin => (188, 92, 120),
        MinibossKind::BellDiver => (96, 188, 170),
        MinibossKind::Foreman => (168, 96, 60),
        MinibossKind::MilkCarton => (226, 222, 206),
    }
}

/// The accent used for the telegraph per kind.
fn telegraph_color(kind: MinibossKind) -> Rgb {
    match kind {
        MinibossKind::Corkscrew => (255, 210, 110),
        MinibossKind::SisterSnip => (226, 236, 255),
        MinibossKind::Bobbin => (255, 150, 190),
        MinibossKind::BellDiver => (140, 240, 224),
        MinibossKind::Foreman => (255, 170, 110),
        MinibossKind::MilkCarton => (255, 246, 214),
    }
}

/// Draw one miniboss body at `e.pos`.
pub fn draw(pb: &mut PixelBuffer, e: &Enemy, mb: &Miniboss, cam: Vec2, accent: Rgb) {
    let p = e.pos - cam;
    let (x, y) = (p.x.round() as i32, p.y.round() as i32);
    let r = e.radius.max(4.0) as i32;
    let base = if e.flash > 0.0 {
        palette::mix(body_color(mb.kind), (255, 244, 218), 0.6)
    } else {
        body_color(mb.kind)
    };
    // Shadow.
    pb.filled_circle(x + 1, y + 2, r, (14, 12, 16));
    // Body: a distinct silhouette per kind.
    match mb.kind {
        MinibossKind::Corkscrew => {
            // Brass auger: a stack of shrinking discs trailing behind.
            for i in 0..4 {
                let rr = (r - i).max(1);
                pb.filled_circle(x - i, y, rr, palette::mix(base, INK, i as f32 * 0.12));
            }
            pb.filled_circle(x + 2, y, (r / 2).max(1), (240, 160, 180));
        }
        MinibossKind::SisterSnip => {
            // Tall shears: two blades crossed over the pivot.
            pb.line(x - r, y - r, x + r, y + r, base);
            pb.line(x - r, y + r, x + r, y - r, base);
            pb.filled_circle(x, y, 2, INK);
        }
        MinibossKind::Bobbin => {
            // Spindle: a bobbin with wound thread.
            pb.rect_fill(x - r, y - (r / 2), 2 * r, r, base);
            for i in (-r..r).step_by(2) {
                pb.plot(x + i, y - (r / 2), palette::mix(base, INK, 0.4));
            }
            pb.filled_circle(x, y, 2, (240, 220, 200));
        }
        MinibossKind::BellDiver => {
            // Bell: a dome with a chipped rim.
            pb.filled_circle(x, y - 1, r, base);
            pb.rect_fill(x - r, y + (r / 2), 2 * r, 2, palette::mix(base, INK, 0.35));
            pb.plot(x + r - 1, y + (r / 2), (255, 255, 255));
        }
        MinibossKind::Foreman => {
            // Rubber stamp: a square head over a handle.
            pb.rect_fill(x - r, y - r, 2 * r, 2 * r, base);
            pb.rect_fill(x - (r / 2), y + r - 1, r, 3, palette::mix(base, INK, 0.4));
            pb.plot(x, y - r, (255, 230, 180));
        }
        MinibossKind::MilkCarton => {
            // Carton: a folded box with a printed spout.
            pb.rect_fill(x - r, y - r, 2 * r, 2 * r, base);
            pb.line(x - r, y - r, x, y - r - 3, INK);
            pb.line(x + r, y - r, x, y - r - 3, INK);
            pb.plot(x + (r / 2), y + (r / 2), (120, 140, 160));
        }
    }
    // Health ring: an arc of the remaining fraction (the nameplate carries the
    // exact number; this is the in-world cue).
    if e.max_hp > 0.0 {
        let frac = (e.hp / e.max_hp).clamp(0.0, 1.0);
        let lit = (frac * 16.0).round() as i32;
        for i in 0..16 {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            let c = if i < lit { accent } else { (54, 58, 62) };
            let px = x + (a.cos() * (r as f32 + 3.0)).round() as i32;
            let py = y + (a.sin() * (r as f32 + 3.0)).round() as i32;
            pb.plot(px, py, c);
        }
    }
}

/// Draw the current telegraph (tell + active) on the ground layer.
pub fn draw_telegraph(pb: &mut PixelBuffer, mb: &Miniboss, cam: Vec2, _accent: Rgb) {
    let Some(shape) = mb.telegraph() else {
        return;
    };
    let color = telegraph_color(mb.kind);
    // The tell is bright and solid; the active window is brighter still.
    let tell = mb.state == MinibossState::Tell;
    let base = if tell {
        palette::mix(color, (255, 255, 255), 0.25)
    } else {
        color
    };
    draw_shape(pb, &shape, cam, base, tell);
}

fn draw_shape(pb: &mut PixelBuffer, shape: &MinibossShape, cam: Vec2, color: Rgb, dotted: bool) {
    match *shape {
        MinibossShape::Trench { a, b, half } => {
            let a = a - cam;
            let b = b - cam;
            let dir = (b - a).normalized();
            let perp = dir.perp() * half;
            // Outline both long edges.
            let steps = ((b - a).len() / 3.0).ceil().max(1.0) as i32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let c = a + (b - a) * t;
                let (x, y) = ((c + perp).x.round() as i32, (c + perp).y.round() as i32);
                let (x2, y2) = ((c - perp).x.round() as i32, (c - perp).y.round() as i32);
                if !dotted || i % 2 == 0 {
                    pb.plot(x, y, color);
                    pb.plot(x2, y2, color);
                }
            }
            pb.line(a.x.round() as i32, a.y.round() as i32, b.x.round() as i32, b.y.round() as i32, palette::mix(color, (0, 0, 0), 0.35));
        }
        MinibossShape::Cone {
            origin,
            aim,
            half_angle,
            len,
        } => {
            let o = origin - cam;
            let a = aim.normalized();
            let l = o + a * len;
            let l1 = o + a.rotate(half_angle) * len;
            let l2 = o + a.rotate(-half_angle) * len;
            pb.line(o.x.round() as i32, o.y.round() as i32, l1.x.round() as i32, l1.y.round() as i32, color);
            pb.line(o.x.round() as i32, o.y.round() as i32, l2.x.round() as i32, l2.y.round() as i32, color);
            let steps = 12;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let ang = -half_angle + t * half_angle * 2.0;
                let p = o + a.rotate(ang) * len;
                pb.plot(p.x.round() as i32, p.y.round() as i32, color);
            }
            let _ = l;
        }
        MinibossShape::Cord { spool, tip, half } => {
            let s = spool - cam;
            let t = tip - cam;
            pb.line(s.x.round() as i32, s.y.round() as i32, t.x.round() as i32, t.y.round() as i32, color);
            let dir = (t - s).normalized();
            let perp = dir.perp() * half;
            pb.line(
                (s + perp).x.round() as i32,
                (s + perp).y.round() as i32,
                (t + perp).x.round() as i32,
                (t + perp).y.round() as i32,
                palette::mix(color, (0, 0, 0), 0.35),
            );
            pb.line(
                (s - perp).x.round() as i32,
                (s - perp).y.round() as i32,
                (t - perp).x.round() as i32,
                (t - perp).y.round() as i32,
                palette::mix(color, (0, 0, 0), 0.35),
            );
        }
        MinibossShape::RingGap {
            origin,
            radius,
            half,
            gap_dir,
            gap_half,
        } => {
            let o = origin - cam;
            let steps = 48;
            for i in 0..=steps {
                let ang = i as f32 / steps as f32 * std::f32::consts::TAU;
                let d = Vec2::from_angle(ang);
                if d.dot(gap_dir.normalized()) >= gap_half.cos() {
                    continue;
                }
                if dotted && i % 3 != 0 {
                    continue;
                }
                for rr in [radius - half, radius + half] {
                    let p = o + d * rr;
                    pb.plot(p.x.round() as i32, p.y.round() as i32, color);
                }
            }
        }
        MinibossShape::Stamps { centers, half } => {
            for c in centers {
                let p = c - cam;
                let s = half.max(1.0) as i32;
                pb.rect_fill(p.x.round() as i32 - s, p.y.round() as i32 - s, s * 2, 1, color);
                pb.rect_fill(p.x.round() as i32 - s, p.y.round() as i32 + s - 1, s * 2, 1, color);
                pb.rect_fill(p.x.round() as i32 - s, p.y.round() as i32 - s, 1, s * 2, color);
                pb.rect_fill(p.x.round() as i32 + s - 1, p.y.round() as i32 - s, 1, s * 2, color);
            }
        }
        MinibossShape::Spill {
            center,
            aim,
            half_len,
            half_w,
        } => {
            let c = center - cam;
            let a = aim.normalized();
            let p = a.perp();
            let corners = [
                c + a * half_len + p * half_w,
                c + a * half_len - p * half_w,
                c - a * half_len + p * half_w,
                c - a * half_len - p * half_w,
            ];
            for i in 0..4 {
                let s = corners[i];
                let e = corners[(i + 1) % 4];
                pb.line(s.x.round() as i32, s.y.round() as i32, e.x.round() as i32, e.y.round() as i32, color);
            }
            for i in 0..=6 {
                let t = i as f32 / 6.0;
                let s = c - a * half_len + p * (half_w * 2.0 * t - half_w);
                let e = c + a * half_len + p * (half_w * 2.0 * t - half_w);
                if !dotted || i % 2 == 0 {
                    pb.line(s.x.round() as i32, s.y.round() as i32, e.x.round() as i32, e.y.round() as i32, palette::mix(color, (0, 0, 0), 0.4));
                }
            }
        }
    }
}
