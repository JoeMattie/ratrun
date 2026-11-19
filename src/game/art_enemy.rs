//! Art for the enemies (the insect horde and the boss). Pure drawing from an
//! enemy's state.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

use super::enemy::{Enemy, EnemyKind};

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

/// Draw one enemy at `e.pos`, facing `e.facing`. `e.anim` is its own animation
/// phase, `e.flash > 0` means it was just hit, and `accent` is the current
/// map's accent colour.
pub fn draw_enemy(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    let s = w2s(e.pos, cam);
    let base = Vec2::new(s.0 as f32, s.1 as f32);
    let f = if e.facing.len_sq() > 0.01 {
        e.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let p = f.perp();
    let r = e.radius;
    let flash = e.flash > 0.0;
    let body = if flash {
        (255, 255, 255)
    } else {
        e.kind.color()
    };
    let dark = palette::scale(body, 0.55);
    let st = bug_style(e.kind);

    // Wings under the body, flapping (additive so they read as gauzy).
    if st.wings {
        let flap = e.anim.sin().abs();
        let wr = (r * (0.45 + flap * 0.7)).max(1.0);
        let wb = (wr * 0.6).max(1.0);
        let wc = palette::scale(accent, 0.4 + flap * 0.45);
        for side in [-1.0f32, 1.0] {
            let c = base + p * (side * r * 0.7) - f * (r * 0.1);
            let cx = c.x as i32;
            let cy = (c.y - r * 0.4) as i32;
            let rr = wr as i32;
            for dy in -rr..=rr {
                for dx in -rr..=rr {
                    let nx = dx as f32 / wr;
                    let ny = dy as f32 / wb;
                    if nx * nx + ny * ny <= 1.0 {
                        pb.plot_add(cx + dx, cy + dy, wc);
                    }
                }
            }
        }
    }

    // Legs: pairs distributed along the body, alternating tripod gait.
    for i in 0..st.leg_pairs {
        let along = f * (r * (0.55 - i as f32 * 0.55));
        let phase = e.anim + i as f32 * 2.1;
        for (k, side) in [(-1.0f32, 0.0f32), (1.0, std::f32::consts::PI)] {
            let swing = (phase + side).sin() * 0.5;
            let hip = base + along + p * (k * r * 0.45);
            let dir = (p * k + f * swing).normalized();
            let knee = hip + dir * (r * 0.85);
            let foot = knee + (p * k * 0.4 + f * (swing - 0.2)).normalized() * (r * 0.7);
            pb.line(
                hip.x as i32,
                hip.y as i32,
                knee.x as i32,
                knee.y as i32,
                dark,
            );
            pb.line(
                knee.x as i32,
                knee.y as i32,
                foot.x as i32,
                foot.y as i32,
                dark,
            );
        }
    }

    // Body segments, head (front) → abdomen (back).
    for seg in 0..st.segs {
        let t = seg as f32 / st.segs.max(1) as f32;
        let c = base + f * (r * (0.7 - t * 1.5));
        let rad = (r * (0.42 + 0.28 * (1.0 - t))).max(1.0);
        let col = if st.stripes && seg % 2 == 1 {
            if flash {
                (255, 255, 255)
            } else {
                palette::scale(body, 0.55)
            }
        } else {
            body
        };
        pb.filled_circle(c.x as i32, c.y as i32, rad as i32, col);
    }

    // Head detail: antennae, eyes, optional horn.
    let head = base + f * (r * 0.7);
    let wave = (e.anim * 0.5).sin() * 0.4;
    for side in [-1.0f32, 1.0] {
        let dir = (f + p * (side * 0.55 + wave * side)).normalized();
        let tip = head + dir * (r * 1.05);
        pb.line(
            head.x as i32,
            head.y as i32,
            tip.x as i32,
            tip.y as i32,
            dark,
        );
    }
    if st.eyes && r >= 3.0 {
        for side in [-1.0f32, 1.0] {
            let ep = head + p * (side * r * 0.35);
            pb.plot(ep.x as i32, ep.y as i32, (15, 15, 18));
        }
    }
    if st.horn && r >= 4.0 {
        let tip = head + f * (r * 0.9);
        pb.line(
            head.x as i32,
            head.y as i32,
            tip.x as i32,
            tip.y as i32,
            palette::scale(body, 1.25),
        );
    }

    // Boss health ring.
    if e.kind == EnemyKind::Boss {
        let frac = (e.hp / e.max_hp).clamp(0.0, 1.0);
        pb.ring(
            s.0,
            s.1,
            r as i32 + 4,
            palette::mix((90, 20, 20), (255, 80, 80), frac),
        );
    }
}

struct BugStyle {
    segs: i32,
    leg_pairs: i32,
    wings: bool,
    stripes: bool,
    horn: bool,
    eyes: bool,
}

/// Per-kind insect anatomy.
fn bug_style(kind: EnemyKind) -> BugStyle {
    match kind {
        // Ant: three crisp segments, six legs, antennae.
        EnemyKind::Skitterer => BugStyle {
            segs: 3,
            leg_pairs: 3,
            wings: false,
            stripes: false,
            horn: false,
            eyes: false,
        },
        // Fly/mosquito: tiny body, fast buzzing wings.
        EnemyKind::Bat => BugStyle {
            segs: 2,
            leg_pairs: 2,
            wings: true,
            stripes: false,
            horn: false,
            eyes: true,
        },
        // Beetle: rounded shell, six legs, eyes.
        EnemyKind::Cat => BugStyle {
            segs: 2,
            leg_pairs: 3,
            wings: false,
            stripes: false,
            horn: false,
            eyes: true,
        },
        // Wasp: striped abdomen, wings, stinger-ish.
        EnemyKind::Spitter => BugStyle {
            segs: 3,
            leg_pairs: 3,
            wings: true,
            stripes: true,
            horn: false,
            eyes: true,
        },
        // Rhino beetle: bulky, horn, heavy legs.
        EnemyKind::Brute => BugStyle {
            segs: 2,
            leg_pairs: 3,
            wings: false,
            stripes: false,
            horn: true,
            eyes: true,
        },
        // Hornet queen: big, winged, striped, mandible eyes.
        EnemyKind::Boss => BugStyle {
            segs: 3,
            leg_pairs: 4,
            wings: true,
            stripes: true,
            horn: true,
            eyes: true,
        },
    }
}
