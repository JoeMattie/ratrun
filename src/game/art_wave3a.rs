//! Pixel art for the two wave-3 weapons that live on the floor and on bodies:
//! **Acid Rain**'s marked, falling drops and **Hantavirus**'s infection pips.
//!
//! Both are deliberately small, flat and legible at two half-block pixels: a
//! floor mark that never moves (so the player learns *where* the splash will
//! land) and a three-pip counter over an infected body (so the player learns
//! *when* the chain jumps). A Neon art pass replaces the shapes, not the
//! contract: `draw_acid_drop` gets the drop entity + its screen position, and
//! `draw_infection_pips` gets a body position, radius and pips left.

use crate::game::ecs::components::AcidDrop;
use crate::game::weapon::RAIN_FALL;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Bright white-lime core of a drop.
const CORE: Rgb = (248, 255, 206);
/// Vivid neon-lime body of the descending strokes.
const STROKE: Rgb = (162, 255, 58);
/// Deep caustic emerald shadow / trailing mist.
const EMERALD: Rgb = (54, 168, 36);
/// Plum infection pip highlight, body, shade, and spent socket.
const PIP_HI: Rgb = (255, 224, 252);
const PIP: Rgb = (232, 78, 204);
const PIP_SHADE: Rgb = (154, 34, 136);
const PIP_DIM: Rgb = (82, 38, 84);
const PIP_BG: Rgb = (20, 8, 24);

/// Subtle floor darkening helper for readable ground reticles on bright tiles.
#[inline]
fn shade_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let cur = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, palette::scale(cur, 1.0 - amount));
}

/// Draw one locked Acid Rain drop at screen position `s`.
///
/// Three reads, in the design's order:
///
/// 1. **The mark** — a dashed caustic lime target ring with cardinal reticle
///    ticks and a shrinking convergence ring so the landing spot reads clearly
///    on dark Sewer bricks and bright Lab/Kitchen tiles alike.
/// 2. **The fall** — in the last [`RAIN_FALL`] seconds, three slanted, tapered
///    lime rain strokes plunge from above into the mark, with hot white-lime
///    teardrop heads.
/// 3. **The hit** — the last frames flare the ring and pop a splash crown.
pub fn draw_acid_drop(pb: &mut PixelBuffer, d: &AcidDrop, s: (i32, i32)) {
    let ring_r = (d.radius.round() as i32).clamp(3, 12);
    // `1 - delay/RAIN_FALL`: 0 while only the mark shows, 1 at impact.
    let fall = ((RAIN_FALL - d.delay) / RAIN_FALL).clamp(0.0, 1.0);
    let dim = 0.58 + 0.42 * fall;
    let phase = d.index as i32;

    // 1. Dashed target ring with subtle dark floor etch so it reads on bright floors.
    let steps = (ring_r * 8).max(16);
    for i in 0..steps {
        // 2-on, 1-off dashed cadence shifted by volley index so stacked drops separate.
        if (i + phase * 2).rem_euclid(3) == 2 {
            continue;
        }
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        let dx = (a.cos() * ring_r as f32).round() as i32;
        let dy = (a.sin() * ring_r as f32).round() as i32;
        shade_px(pb, s.0 + dx, s.1 + dy, 0.28);
        pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(STROKE, 0.55 + 0.35 * dim));
    }
    // Four inward cardinal reticle ticks so the mark reads as a locked strike zone.
    let tick_col = palette::scale(CORE, 0.45 + 0.40 * fall);
    for &(tx, ty) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
        pb.plot_add(s.0 + tx * (ring_r - 1), s.1 + ty * (ring_r - 1), tick_col);
        if ring_r >= 6 {
            pb.plot_add(s.0 + tx * (ring_r - 2), s.1 + ty * (ring_r - 2), palette::scale(STROKE, 0.55));
        }
    }

    // Converging inner focus ring once the fall begins.
    if fall > 0.0 {
        let r_in = ((1.0 - fall) * (ring_r as f32 * 0.72)).round() as i32;
        if r_in >= 2 {
            let in_steps = (r_in * 6).max(8);
            let c_in = palette::scale(CORE, 0.35 + 0.45 * fall);
            for i in (0..in_steps).step_by(2) {
                let a = i as f32 / in_steps as f32 * std::f32::consts::TAU + fall * 1.4;
                let rx = s.0 + (a.cos() * r_in as f32).round() as i32;
                let ry = s.1 + (a.sin() * r_in as f32).round() as i32;
                pb.plot_add(rx, ry, c_in);
            }
        } else {
            // Final bullseye pinpoint.
            pb.plot(s.0, s.1, CORE);
            for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
                pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(STROKE, 0.85));
            }
        }
    }

    // 2. Three descending slanted lime strokes plunging toward the mark.
    if fall > 0.0 {
        let drop_y = s.1 - (2.0 + (1.0 - fall) * 16.0).round() as i32;
        let bright = (0.45 + 0.55 * fall).clamp(0.15, 1.0);
        for k in 0..3i32 {
            // Spread across the target with volley-staggered heights.
            let x = s.0 + (k - 1) * 3;
            let stagger = (((k + phase) % 3) - 1) * 2;
            let y = drop_y + stagger;
            let is_lead = k == 1;
            let len = if is_lead { 6 } else { 5 };
            // Hot teardrop head at the leading bottom tip.
            pb.plot(x, y, palette::scale(if is_lead { CORE } else { STROKE }, bright));
            if is_lead {
                pb.plot_add(x, y + 1, palette::scale(CORE, bright * 0.75));
                pb.plot_add(x - 1, y, palette::scale(STROKE, bright * 0.45));
                pb.plot_add(x + 1, y, palette::scale(STROKE, bright * 0.45));
            }
            // Slanted tapered caustic streak trailing up-left (matching top-left rain slant).
            for seg in 1..len {
                let t = seg as f32 / len as f32;
                let sx = x - seg / 3;
                let sy = y - seg;
                let col = if seg == 1 {
                    palette::mix(CORE, STROKE, 0.5)
                } else {
                    palette::mix(STROKE, EMERALD, t)
                };
                pb.plot_add(sx, sy, palette::scale(col, bright * (1.0 - t * 0.65)));
            }
        }
    }

    // 3. Impact frames: caustic splash crown & white-lime ring flare.
    if fall >= 0.86 {
        let k = ((fall - 0.86) / 0.14).clamp(0.0, 1.0);
        pb.ring(s.0, s.1, ring_r, palette::scale(CORE, 0.55 + 0.45 * k));
        // Little upward splash crown droplets popping off the center.
        for &(dx, dy) in &[(-2, -2), (0, -3), (2, -2), (-3, -1), (3, -1)] {
            pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(CORE, 0.65 + 0.35 * k));
        }
    }
}

/// Draw the three Hantavirus pips above a body at screen position `s`.
///
/// `ticks_left` pips are lit and the rest are empty sockets, so the countdown
/// is readable at a glance: 3 at the tag, then one gone per tick, and the pips
/// vanish with the mark on the third.
pub fn draw_infection_pips(pb: &mut PixelBuffer, s: (i32, i32), radius: f32, ticks_left: u8) {
    if ticks_left == 0 {
        return;
    }
    let lit = ticks_left.min(3) as i32;
    let r = radius.round() as i32;
    let y = s.1 - (r + 9);
    let x0 = s.0 - 4;

    // Dark contoured backing pill behind all three 2x2 bio-beads so the counter
    // clears floating damage digits and survives bright enemies or bursts.
    pb.rect_fill(x0 - 1, y - 1, 10, 4, PIP_BG);
    for i in 0..3i32 {
        let x = x0 + i * 3;
        if i < 3 - lit {
            // Spent pip: crisp hollow dark-plum socket.
            pb.plot(x, y, PIP_DIM);
            pb.plot(x + 1, y, PIP_DIM);
            pb.plot(x, y + 1, palette::scale(PIP_DIM, 0.72));
            pb.plot(x + 1, y + 1, palette::scale(PIP_DIM, 0.72));
        } else {
            // Lit bio-bead: vivid 2x2 plum pearl with hot white-pink glint + aura.
            pb.plot(x, y, PIP_HI);
            pb.plot(x + 1, y, (246, 94, 224));
            pb.plot(x, y + 1, PIP);
            pb.plot(x + 1, y + 1, PIP_SHADE);
            pb.plot_add(x, y - 1, palette::scale((246, 94, 224), 0.65));
            pb.plot_add(x + 1, y - 1, palette::scale(PIP, 0.45));
        }
    }

    // Four plum spore-ticks hugging the host's diagonal perimeter so the body
    // itself reads as infected even if a floating digit overlaps the pip bar.
    let hr = (r + 2).max(3);
    for &(dx, dy) in &[(-hr, -hr + 1), (hr, -hr + 1), (-hr, hr - 1), (hr, hr - 1)] {
        pb.plot(s.0 + dx, s.1 + dy, PIP_BG);
        pb.plot_add(s.0 + dx, s.1 + dy, (246, 94, 224));
    }
    // Thin vertical plum tether pixel linking the bead bar down to the host.
    pb.plot_add(s.0, y + 3, palette::scale(PIP, 0.7));
}

// ============================================================================
// Wave-3 Evolution World Art (render-only, zero rng, zero allocation)
// ============================================================================
//
// Direct entity rendering for the persistent & stateful wave-3 evolutions
// (`WAVE3_DESIGN.md` §3 "Half-block read" column), so every evolution reads
// crisply in motion and in still frames regardless of particle load.

use crate::game::ecs::components::{
    CheeseString, CorrosiveChannel, FairyRing, MilkPhase, MilkTooth,
};
use crate::game::ecs::systems::evolutions::{CHANNEL_ADVANCE, CHANNEL_SETTLE, MILK_LODGE_SECS};
use crate::math::Vec2;

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, palette::mix(bg, c, a));
}

/// **Milk Teeth** (Gnaw + Sharp Teeth): curved ivory hook dart and dotted
/// return thread tethered back to the rat.
pub fn draw_milk_tooth(pb: &mut PixelBuffer, dart: &MilkTooth, player_pos: Vec2, cam: Vec2) {
    let ivory_hi: Rgb = (255, 254, 238);
    let ivory: Rgb = (242, 228, 186);
    let bone: Rgb = (176, 158, 122);
    let gum: Rgb = (242, 114, 140);
    let out: Rgb = (36, 28, 26);

    // 1. Dotted return thread from the rat's muzzle to the lodged/returning hook.
    let seg = dart.pos - player_pos;
    let dist = seg.len();
    if dist > 2.5 {
        let dir = seg * (1.0 / dist);
        let thread_bright = match dart.phase {
            MilkPhase::Out => 0.65,
            MilkPhase::Lodged => 0.72 + 0.28 * (dart.age / MILK_LODGE_SECS).clamp(0.0, 1.0),
            MilkPhase::Back => 1.0,
        };
        let step_px = 2.4f32;
        let mut d = 2.5f32;
        let mut idx = 0i32;
        while d < dist - 1.5 {
            let p = w2s(player_pos + dir * d, cam);
            shade_px(pb, p.0, p.1 + 1, 0.35);
            let col = if idx % 2 == 0 {
                palette::scale(ivory_hi, thread_bright)
            } else {
                palette::scale(ivory, thread_bright * 0.85)
            };
            pb.plot(p.0, p.1, col);
            d += step_px;
            idx += 1;
        }
    }

    // 2. Curved ivory hook / milk-tooth fang at `dart.pos`.
    let dir = if dart.vel.len_sq() > 1.0 {
        dart.vel.normalized()
    } else if dist > 0.5 {
        (player_pos - dart.pos) * (1.0 / dist)
    } else {
        Vec2::new(0.0, -1.0)
    };
    let perp = dir.perp();
    let c = dart.pos;
    let cs = w2s(c, cam);
    shade_px(pb, cs.0 + 1, cs.1 + 2, 0.42);
    for &(u, v) in &[
        (-2.0f32, 0.0f32),
        (-1.0, 0.0),
        (0.0, 0.0),
        (1.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ] {
        let p = w2s(c + dir * u + perp * v, cam);
        for &(ox, oy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
            blend_px(pb, p.0 + ox, p.1 + oy, out, 0.65);
        }
    }
    // Pink root knot where the thread ties onto the milk tooth.
    let root = w2s(c - dir * 2.0, cam);
    pb.plot(root.0, root.1, gum);
    // Curved ivory shaft + hooked barb tip.
    for &(u, v, col) in &[
        (-1.0f32, 0.0f32, bone),
        (0.0, 0.0, ivory),
        (1.0, 0.0, ivory_hi),
        (2.0, 0.0, ivory_hi),
        (2.0, 1.0, ivory),
        (1.0, 2.0, ivory_hi),
        (0.0, 2.0, ivory_hi),
    ] {
        let p = w2s(c + dir * u + perp * v, cam);
        pb.plot(p.0, p.1, col);
    }
    if matches!(dart.phase, MilkPhase::Lodged | MilkPhase::Back) {
        pb.plot_add(cs.0, cs.1, palette::scale(ivory_hi, 0.45));
    }
}

/// **Fondue Fork** (Cheese Spray + Fat Reserves): two planted yellow fork posts
/// linked by a single continuous sagging melted-cheese line.
pub fn draw_cheese_string(pb: &mut PixelBuffer, line: &CheeseString, cam: Vec2, elapsed: f32) {
    let life_frac = (line.life / line.total.max(1e-4)).clamp(0.0, 1.0);
    let alpha = (life_frac / 0.20).clamp(0.0, 1.0);
    let hot: Rgb = (255, 250, 198);
    let cheese: Rgb = (255, 206, 58);
    let ochre: Rgb = (182, 114, 20);
    let out: Rgb = (46, 28, 12);

    // 1. Single sagging catenary cheese filament between the two posts.
    let sag_max = 2.6 + 3.6 * (1.0 - life_frac);
    let span = line.a.dist(line.b);
    let steps = (span * 1.5).round().clamp(8.0, 96.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let sag = sag_max * (4.0 * t * (1.0 - t));
        let p = line.a + (line.b - line.a) * t + Vec2::new(0.0, sag);
        let s = w2s(p, cam);
        pb.plot_add(s.0, s.1 + 1, palette::scale(ochre, 0.48 * alpha));
        let strand_col = if (i + (elapsed * 12.0) as i32) % 5 == 0 {
            hot
        } else {
            cheese
        };
        blend_px(pb, s.0, s.1, strand_col, 0.92 * alpha);
        if i % 3 == 0 {
            pb.plot_add(s.0, s.1 - 1, palette::scale(hot, 0.28 * alpha));
        }
    }
    // Glossy cheese drip bead hanging from the lowest midpoint.
    let mid = (line.a + line.b) * 0.5 + Vec2::new(0.0, sag_max);
    let ms = w2s(mid, cam);
    let drip_y = ((elapsed * 3.2).sin() * 0.5 + 0.5).round() as i32;
    blend_px(pb, ms.0, ms.1 + 1 + drip_y, cheese, 0.85 * alpha);
    pb.plot_add(ms.0, ms.1 + 1, palette::scale(hot, 0.5 * alpha));

    // 2. Two planted yellow fondue-fork posts at `a` and `b`.
    for post in [line.a, line.b] {
        let (x, y) = w2s(post, cam);
        shade_px(pb, x, y + 2, 0.45 * alpha);
        shade_px(pb, x + 1, y + 2, 0.45 * alpha);
        for dy in -4..=1 {
            blend_px(pb, x - 2, y + dy, out, 0.65 * alpha);
            blend_px(pb, x + 2, y + dy, out, 0.65 * alpha);
        }
        // Twin fork prongs at the top (`y - 4..=y - 3`) and gold shaft (`y - 2..=y + 1`).
        blend_px(pb, x - 1, y - 4, hot, alpha);
        blend_px(pb, x + 1, y - 4, cheese, alpha);
        blend_px(pb, x - 1, y - 3, cheese, alpha);
        blend_px(pb, x + 1, y - 3, ochre, alpha);
        blend_px(pb, x - 1, y - 2, hot, alpha);
        blend_px(pb, x, y - 2, cheese, alpha);
        blend_px(pb, x + 1, y - 2, ochre, alpha);
        blend_px(pb, x, y - 1, hot, alpha);
        blend_px(pb, x, y, cheese, alpha);
        blend_px(pb, x, y + 1, ochre, alpha);
    }
}

/// **Echo Chamber** (Squeak Nova + Frenzy): outbound violet dots become crisp
/// hollow white inward ticks after reflecting off a wall.
pub fn draw_echo_spoke(pb: &mut PixelBuffer, s: (i32, i32), vel: Vec2, reflected: bool) {
    let dir = if vel.len_sq() > 1.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();
    if !reflected {
        // Outbound: vivid violet orb with a bright lavender core & short comet pixel.
        let core: Rgb = (246, 224, 255);
        let violet: Rgb = (196, 108, 255);
        let tail: Rgb = (132, 54, 214);
        pb.plot(s.0, s.1, core);
        for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
            pb.plot_add(s.0 + dx, s.1 + dy, palette::scale(violet, 0.72));
        }
        let back = (
            s.0 - (dir.x * 2.0).round() as i32,
            s.1 - (dir.y * 2.0).round() as i32,
        );
        pb.plot_add(back.0, back.1, tail);
    } else {
        // Reflected inward: crisp hollow white chevron tick (`>`) racing back!
        let white: Rgb = (255, 254, 255);
        let ice: Rgb = (214, 198, 255);
        let dark: Rgb = (24, 16, 38);
        let tip = (
            s.0 + (dir.x * 2.0).round() as i32,
            s.1 + (dir.y * 2.0).round() as i32,
        );
        // Dark hollow center so the tick visibly reads as *hollow* white!
        pb.plot(s.0, s.1, dark);
        pb.plot(tip.0, tip.1, white);
        for side in [-1.0f32, 1.0] {
            let w1 = (
                s.0 + (dir.x * 0.5 + perp.x * side * 1.4).round() as i32,
                s.1 + (dir.y * 0.5 + perp.y * side * 1.4).round() as i32,
            );
            let w2 = (
                s.0 + (-dir.x * 1.2 + perp.x * side * 2.3).round() as i32,
                s.1 + (-dir.y * 1.2 + perp.y * side * 2.3).round() as i32,
            );
            pb.plot(w1.0, w1.1, white);
            pb.plot(w2.0, w2.1, white);
            // Soft lavender echo wake behind each wing.
            let wake = (
                w2.0 - (dir.x * 1.5).round() as i32,
                w2.1 - (dir.y * 1.5).round() as i32,
            );
            pb.plot_add(wake.0, wake.1, palette::scale(ice, 0.65));
        }
    }
}

/// **Fairy Ring** (Spore Orbit + Resilience): bioluminescent mint mushrooms
/// outlining a clean empty circle on the floor.
pub fn draw_fairy_ring(pb: &mut PixelBuffer, ring: &FairyRing, cam: Vec2, elapsed: f32) {
    let life_frac = (ring.life / ring.total.max(1e-4)).clamp(0.0, 1.0);
    let alpha = (life_frac / 0.16).clamp(0.0, 1.0);
    let pulse = 0.80 + 0.20 * (elapsed * 4.2).sin();

    let center = w2s(ring.pos, cam);
    let r = ring.radius.round() as i32;

    // 1. Delicate dotted mycelium floor ring outlining the empty circle.
    let ring_col = palette::scale((82, 214, 152), 0.42 * alpha * pulse);
    let steps = (r * 7).max(24);
    for i in (0..steps).step_by(2) {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU + ring.spin * 0.3;
        let rx = center.0 + (a.cos() * ring.radius).round() as i32;
        let ry = center.1 + (a.sin() * ring.radius).round() as i32;
        pb.plot_add(rx, ry, ring_col);
    }

    // 2. Mint mushrooms spaced evenly around the circle perimeter.
    let count = (ring.satellites as usize).max(6);
    let out: Rgb = (18, 42, 32);
    let stem: Rgb = (212, 240, 224);
    let cap_hi: Rgb = (224, 255, 240);
    let cap_mid: Rgb = (114, 255, 178);
    let cap_lo: Rgb = (42, 162, 102);

    for k in 0..count {
        let a = ring.spin + k as f32 / count as f32 * std::f32::consts::TAU;
        let p = ring.pos + Vec2::from_angle(a) * ring.radius;
        let (mx, my) = w2s(p, cam);
        let glow = (0.75 + 0.25 * (elapsed * 4.8 + k as f32 * 1.1).sin()) * alpha;

        pb.plot_add(mx, my + 1, palette::scale(cap_mid, 0.35 * glow));
        pb.plot_add(mx - 1, my + 1, palette::scale(cap_mid, 0.22 * glow));
        pb.plot_add(mx + 1, my + 1, palette::scale(cap_mid, 0.22 * glow));

        shade_px(pb, mx, my + 2, 0.40 * alpha);
        blend_px(pb, mx - 2, my - 1, out, 0.6 * alpha);
        blend_px(pb, mx + 2, my - 1, out, 0.6 * alpha);
        blend_px(pb, mx - 1, my - 3, out, 0.55 * alpha);
        blend_px(pb, mx, my - 3, out, 0.55 * alpha);
        blend_px(pb, mx + 1, my - 3, out, 0.55 * alpha);

        // Pale slender stem (`y = 0..=1`).
        blend_px(pb, mx, my, stem, alpha);
        blend_px(pb, mx, my + 1, palette::scale(stem, 0.8), alpha);

        // Domed bioluminescent mint mushroom cap (`y = -2..=-1`).
        blend_px(pb, mx - 1, my - 2, cap_hi, alpha);
        blend_px(pb, mx, my - 2, cap_hi, alpha);
        blend_px(pb, mx + 1, my - 2, cap_mid, alpha);
        blend_px(pb, mx - 2, my - 1, cap_mid, alpha);
        blend_px(pb, mx - 1, my - 1, cap_mid, alpha);
        blend_px(pb, mx, my - 1, cap_lo, alpha);
        blend_px(pb, mx + 1, my - 1, cap_lo, alpha);
        blend_px(pb, mx + 2, my - 1, cap_lo, alpha);
        pb.plot_add(mx - 1, my - 2, palette::scale(cap_hi, 0.45 * glow));
    }
}

/// **Drain Cleaner** (Acid Spit + Thick Hide): 36 × 6 px corrosive channel with
/// crisp bile-green advancing chevrons (`>>>`) marching down the stripe.
pub fn draw_corrosive_channel(
    pb: &mut PixelBuffer,
    ch: &CorrosiveChannel,
    cam: Vec2,
    elapsed: f32,
) {
    let total = CHANNEL_ADVANCE + CHANNEL_SETTLE;
    let fade = ((total - ch.age) / 0.42).clamp(0.0, 1.0);
    if fade <= 0.01 {
        return;
    }
    let advancing = ch.age <= CHANNEL_ADVANCE;
    let open_len = ch.length * (ch.age / CHANNEL_ADVANCE).clamp(0.0, 1.0);
    let dir = if ch.dir.len_sq() > 1e-4 {
        ch.dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let side = dir.perp();

    let trough_dark: Rgb = (24, 58, 18);
    let bile_fill: Rgb = (96, 214, 42);
    let chev_bright: Rgb = (176, 255, 72);
    let chev_hot: Rgb = (238, 255, 166);

    // 1. Etched 6-px corrosive gutter fill from `start` to `open_len`.
    let steps = open_len.ceil() as i32;
    for along in 0..=steps {
        let af = along as f32;
        for w in -3..=3i32 {
            let p = w2s(ch.start + dir * af + side * w as f32, cam);
            if w.abs() == 3 {
                blend_px(pb, p.0, p.1, trough_dark, 0.55 * fade);
            } else {
                blend_px(pb, p.0, p.1, trough_dark, 0.30 * fade);
                pb.plot_add(
                    p.0,
                    p.1,
                    palette::scale(bile_fill, (0.34 - 0.05 * w.abs() as f32) * fade),
                );
            }
        }
    }

    // 2. Advancing bile-green chevrons (`>>>`) marching down the channel.
    let speed = if advancing { 42.0 } else { 14.0 };
    let spacing = 6.0f32;
    let scroll = (elapsed * speed).rem_euclid(spacing);
    let mut head = scroll;
    while head <= open_len + 1.0 {
        let near_front = (open_len - head).abs() < 4.5 && advancing;
        let tip_col = if near_front { chev_hot } else { chev_bright };
        for w in -2..=2i32 {
            let back = w.abs() as f32;
            let pos_along = head - back;
            if pos_along < 0.0 || pos_along > open_len {
                continue;
            }
            let p = w2s(ch.start + dir * pos_along + side * w as f32, cam);
            let c = if w == 0 {
                tip_col
            } else {
                palette::mix(tip_col, bile_fill, 0.35)
            };
            pb.plot_add(p.0, p.1, palette::scale(c, (0.85 - 0.10 * back) * fade));
        }
        head += spacing;
    }

    // 3. Blazing leading chevron right at the advancing front!
    if advancing && open_len >= 2.0 {
        for w in -3..=3i32 {
            let back = w.abs() as f32 * 0.9;
            let pos_along = (open_len - back).max(0.0);
            let p = w2s(ch.start + dir * pos_along + side * w as f32, cam);
            let c = if w.abs() <= 1 { chev_hot } else { chev_bright };
            pb.plot(p.0, p.1, palette::scale(c, fade));
        }
    }
}

/// **Flywheel** (Tail Thrash + Counterweight): copper counterweight studs &
/// caught-pellet copper pips riding along the sweeping tail arc.
pub fn draw_flywheel_pips(
    pb: &mut PixelBuffer,
    player_pos: Vec2,
    angle: f32,
    reach: f32,
    fade: f32,
    caught: u8,
    cam: Vec2,
) {
    let fade = fade.clamp(0.0, 1.0);
    if fade <= 0.02 || reach <= 6.0 {
        return;
    }
    let hot: Rgb = (255, 244, 198);
    let copper: Rgb = (242, 146, 56);
    let bronze: Rgb = (158, 72, 22);
    let out: Rgb = (46, 24, 12);

    let pip_count = (caught as usize).clamp(3, 6);
    let has_caught = caught > 0;
    for i in 0..pip_count {
        let t = (i as f32 + 1.0) / (pip_count as f32 + 0.6);
        let r = 8.0 + (reach - 10.0) * t;
        // Follow the exact lag curve of `art_rat::draw_tail_thrash`.
        let a = angle - t * 0.42;
        let p = player_pos + Vec2::from_angle(a) * r;
        let (x, y) = w2s(p, cam);

        if has_caught && i < caught as usize {
            // Blazing caught-pellet copper orb mounted on the tail.
            for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
                pb.plot_add(x + dx, y + dy, palette::scale(copper, 0.75 * fade));
            }
            pb.plot(x - 1, y - 1, palette::scale(hot, fade));
            pb.plot(x, y - 1, palette::scale(copper, fade));
            pb.plot(x - 1, y, palette::scale(copper, fade));
            pb.plot(x, y, palette::scale(bronze, fade));
        } else {
            // Riveted copper stud along the evolved Flywheel rim.
            blend_px(pb, x + 1, y + 1, out, 0.5 * fade);
            pb.plot(x, y, palette::scale(hot, 0.9 * fade));
            pb.plot_add(x + 1, y, palette::scale(copper, 0.65 * fade));
            pb.plot_add(x, y + 1, palette::scale(bronze, 0.65 * fade));
        }
    }
}

/// **Flywheel** flung copper dart (the returned copper fan projectiles).
pub fn draw_flywheel_dart(pb: &mut PixelBuffer, s: (i32, i32), vel: Vec2) {
    let dir = if vel.len_sq() > 1.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();
    let hot: Rgb = (255, 246, 204);
    let copper: Rgb = (246, 152, 60);
    let bronze: Rgb = (168, 82, 24);

    let tip = (
        s.0 + (dir.x * 1.6).round() as i32,
        s.1 + (dir.y * 1.6).round() as i32,
    );
    let tail = (
        s.0 - (dir.x * 2.2).round() as i32,
        s.1 - (dir.y * 2.2).round() as i32,
    );
    pb.plot(tip.0, tip.1, hot);
    pb.plot(s.0, s.1, copper);
    pb.plot_add(tail.0, tail.1, bronze);
    for side in [-1.0f32, 1.0] {
        let w = (
            s.0 + (-dir.x * 0.6 + perp.x * side * 1.3).round() as i32,
            s.1 + (-dir.y * 0.6 + perp.y * side * 1.3).round() as i32,
        );
        pb.plot(w.0, w.1, copper);
    }
}

/// **Roll Call** (Sonic Squeak + Whiskers): cyan quotation marks (`“ ”`)
/// framing a tagged enemy that wink shut in sequence before its answer pulse.
pub fn draw_roll_call_quote(
    pb: &mut PixelBuffer,
    s: (i32, i32),
    radius: f32,
    progress: f32,
    slot: usize,
) {
    let wink = (progress * 1.45 - slot as f32 * 0.06).clamp(0.0, 1.0);
    let r = radius.round() as i32;
    let spread = ((r + 4) as f32 * (1.0 - 0.55 * wink)).round() as i32;
    let y = s.1 - r - 4 + (wink * 2.0).round() as i32;

    let bg: Rgb = (14, 28, 38);
    let hi: Rgb = (232, 255, 255);
    let cyan: Rgb = (96, 236, 255);
    let deep: Rgb = (38, 162, 198);

    let flare = if wink > 0.72 { 1.0 } else { 0.78 + 0.22 * wink };

    for &(side, lx) in &[(-1i32, s.0 - spread), (1i32, s.0 + spread)] {
        if wink > 0.85 {
            // Winked shut: crisp bright horizontal cyan blink slit!
            pb.rect_fill(lx - 1, y, 3, 3, bg);
            pb.plot(lx - 1, y + 1, palette::scale(hi, flare));
            pb.plot(lx, y + 1, palette::scale(hi, flare));
            pb.plot(lx + 1, y + 1, palette::scale(cyan, flare));
            pb.plot_add(lx, y, palette::scale(cyan, 0.55));
            pb.plot_add(lx, y + 2, palette::scale(cyan, 0.55));
        } else {
            // Open double-comma quotation mark (`2x3` curved tick pair with dark backing).
            pb.rect_fill(lx - 1, y - 1, 4, 4, bg);
            if side < 0 {
                pb.plot(lx, y + 1, palette::scale(hi, flare));
                pb.plot(lx, y, palette::scale(cyan, flare));
                pb.plot(lx + 1, y, palette::scale(deep, flare));
                pb.plot(lx + 2, y + 1, palette::scale(cyan, flare));
                pb.plot(lx + 2, y, palette::scale(hi, flare));
            } else {
                pb.plot(lx, y, palette::scale(hi, flare));
                pb.plot(lx, y + 1, palette::scale(cyan, flare));
                pb.plot(lx - 1, y + 1, palette::scale(deep, flare));
                pb.plot(lx + 2, y, palette::scale(hi, flare));
                pb.plot(lx + 2, y + 1, palette::scale(cyan, flare));
            }
        }
    }
}

/// Crisp segmented white-tin Bottle Cap shield rim circling the rat while
/// `player.buffs.shield > 0.0` (design §10: "a small white rim marks the charge").
pub fn draw_bottle_cap_rim(pb: &mut PixelBuffer, player_pos: Vec2, elapsed: f32, cam: Vec2) {
    let center = w2s(player_pos, cam);
    let r = 8.0f32;
    let spin = elapsed * 2.2;
    let hi: Rgb = (255, 255, 250);
    let tin: Rgb = (212, 220, 228);
    // Four sleek segmented tin-white rim arcs (each ~40 deg) rotating smoothly.
    for seg in 0..4 {
        let base = spin + seg as f32 * std::f32::consts::FRAC_PI_2;
        for step in 0..5 {
            let t = step as f32 / 4.0;
            let a = base + (t - 0.5) * 0.68;
            let x = center.0 + (a.cos() * r).round() as i32;
            let y = center.1 + (a.sin() * r).round() as i32;
            let edge = 1.0 - (t * 2.0 - 1.0).abs();
            let col = if step == 2 { hi } else { tin };
            pb.plot_add(x, y, palette::scale(col, 0.42 + 0.50 * edge));
        }
    }
}

/// Evolved **Fairy Ring** active orbiter: mini bioluminescent mint mushroom
/// circling the rat so evolved Spore Orbit reads differently on every cycle.
pub fn draw_evolved_spore_orbiter(pb: &mut PixelBuffer, s: (i32, i32), elapsed: f32) {
    let (mx, my) = s;
    let glow = 0.75 + 0.25 * (elapsed * 5.0 + mx as f32 * 0.3).sin();
    let out: Rgb = (16, 40, 30);
    let stem: Rgb = (218, 244, 228);
    let cap_hi: Rgb = (226, 255, 242);
    let cap_mid: Rgb = (112, 255, 176);
    let cap_lo: Rgb = (38, 164, 100);

    // Soft mint aura.
    for &(dx, dy) in &[(-2, 0), (2, 0), (0, -2), (0, 2)] {
        pb.plot_add(mx + dx, my + dy, palette::scale(cap_mid, 0.26 * glow));
    }
    blend_px(pb, mx - 2, my - 1, out, 0.6);
    blend_px(pb, mx + 2, my - 1, out, 0.6);
    // Stem.
    pb.plot(mx, my + 1, stem);
    pb.plot(mx, my, stem);
    // Mint mushroom cap.
    pb.plot(mx - 1, my - 1, cap_hi);
    pb.plot(mx, my - 1, cap_hi);
    pb.plot(mx + 1, my - 1, cap_mid);
    pb.plot(mx - 2, my, cap_mid);
    pb.plot(mx - 1, my, cap_mid);
    pb.plot(mx + 1, my, cap_lo);
    pb.plot(mx + 2, my, cap_lo);
}

/// **Super Dash** kinetic blue-white comb drawn cleanly over the swept corridor
/// (above enemy hit-flashes so the comb teeth visibly cut through crossed bodies).
pub fn draw_super_dash_comb(
    pb: &mut PixelBuffer,
    head: Vec2,
    dir: Vec2,
    length: f32,
    radius: f32,
    dash_frac: f32,
    cam: Vec2,
) {
    let dir = if dir.len_sq() > 1e-4 {
        dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();
    let fade = dash_frac.clamp(0.15, 1.0);
    let white: Rgb = (252, 255, 255);
    let cyan: Rgb = (88, 222, 255);
    let cobalt: Rgb = (38, 128, 248);
    let navy: Rgb = (16, 42, 92);

    // 1. Blazing white-cyan central spine along the travelled segment behind `head`.
    let steps = length.ceil().clamp(6.0, 34.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = head - dir * (t * length);
        let s = w2s(p, cam);
        let a = (1.0 - t * 0.65) * fade;
        blend_px(pb, s.0, s.1, white, (0.92 * a).clamp(0.0, 1.0));
        for side in [-1.0f32, 1.0] {
            let sh = w2s(p + perp * side, cam);
            pb.plot_add(sh.0, sh.1, palette::scale(cyan, 0.65 * a));
        }
    }

    // 2. Crisp evenly-spaced perpendicular comb teeth raking OUT PAST the crossed
    //    bodies (`radius + 4.5 px`), with dark-cobalt edge contrast so they pop
    //    sharply even over bright enemy white hit flashes!
    let tooth_reach = (radius + 4.5).clamp(7.0, 11.5);
    let spacing = 3.2f32;
    let mut dist = 1.0f32;
    while dist <= length {
        let t = (dist / length.max(1.0)).clamp(0.0, 1.0);
        let a = (1.0 - t * 0.55) * fade;
        let root = head - dir * dist;
        let reach = tooth_reach * (1.0 - 0.25 * t);
        let max_k = reach.ceil() as i32;
        for side in [-1.0f32, 1.0] {
            for k in 2..=max_k {
                let kt = k as f32 / reach;
                // Slightly raked backward like supersonic comb tines.
                let tp = root + perp * (side * k as f32) - dir * (k as f32 * 0.28);
                let ts = w2s(tp, cam);
                let col = if kt < 0.45 {
                    white
                } else if kt < 0.78 {
                    cyan
                } else {
                    cobalt
                };
                // Subtle dark backing on outer tips so bright white hit-flashes
                // cannot swallow the blue comb teeth.
                if kt >= 0.45 {
                    let shadow = w2s(tp - dir, cam);
                    blend_px(pb, shadow.0, shadow.1, navy, 0.55 * a);
                }
                blend_px(pb, ts.0, ts.1, col, (0.90 * a * (1.0 - kt * 0.25)).clamp(0.0, 1.0));
            }
        }
        dist += spacing;
    }

    // 3. Leading blue-white spearhead chevron right ahead of the rat's nose.
    let tip = head + dir * 4.5;
    for k in 0..=5i32 {
        let kf = k as f32;
        for side in [-1.0f32, 1.0] {
            let p = tip - dir * (kf * 0.9) + perp * (side * kf * 0.85);
            let s = w2s(p, cam);
            let col = if k <= 2 { white } else { cyan };
            blend_px(pb, s.0, s.1, col, (0.95 - kf * 0.12) * fade);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::particle::{vfx, Particles};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// Gated behind `RATRUN_EVO_SHEET`: writes a 4×2 close-up PPM sheet of all
    /// eight wave-3 evolutions in their signature half-block poses for review.
    #[test]
    fn dump_evo_showcase_ppm() {
        let Ok(out_path) = std::env::var("RATRUN_EVO_SHEET") else {
            return;
        };
        let (cw, ch) = (84usize, 76usize);
        let (cols, rows) = (4usize, 2usize);
        let mut sheet = PixelBuffer::new(cw * cols, ch * rows);
        let center = Vec2::new(cw as f32 * 0.5, ch as f32 * 0.52);

        let mut render = |idx: usize, f: &mut dyn FnMut(&mut PixelBuffer)| {
            let mut cell = PixelBuffer::new(cw, ch);
            for y in 0..ch {
                for x in 0..cw {
                    let dark = ((x / 10) + (y / 8)) % 2 == 0;
                    cell.plot(x as i32, y as i32, if dark { (20, 27, 29) } else { (24, 33, 35) });
                }
            }
            f(&mut cell);
            let ox = (idx % cols) * cw;
            let oy = (idx / cols) * ch;
            for y in 0..ch {
                for x in 0..cw {
                    sheet.plot((ox + x) as i32, (oy + y) as i32, cell.pixel_at(x, y));
                }
            }
        };

        // 0: Milk Teeth (ivory hook + dotted return thread)
        render(0, &mut |pb| {
            let mut dart = MilkTooth::new(
                center + Vec2::new(22.0, -14.0),
                Vec2::new(-120.0, 75.0),
                20.0,
                crate::game::ecs::systems::evolutions::MILK_REACH,
            );
            dart.phase = MilkPhase::Back;
            dart.age = 0.25;
            draw_milk_tooth(pb, &dart, center + Vec2::new(-18.0, 14.0), Vec2::ZERO);
        });

        // 1: Fondue Fork (two yellow posts + sagging cheese string)
        render(1, &mut |pb| {
            let mut line = CheeseString::new(
                center + Vec2::new(-22.0, -4.0),
                center + Vec2::new(22.0, 4.0),
                1.5,
                18.0,
            );
            line.life = 0.75;
            draw_cheese_string(pb, &line, Vec2::ZERO, 1.4);
        });

        // 2: Echo Chamber (violet outbound dots + hollow white inward ticks)
        render(2, &mut |pb| {
            for k in 0..6 {
                let a = k as f32 / 6.0 * std::f32::consts::TAU;
                let dir = Vec2::from_angle(a);
                let out_p = center + dir * 24.0;
                draw_echo_spoke(pb, w2s(out_p, Vec2::ZERO), dir * 95.0, false);
                let in_p = center + Vec2::from_angle(a + 0.52) * 16.0;
                draw_echo_spoke(pb, w2s(in_p, Vec2::ZERO), -Vec2::from_angle(a + 0.52) * 95.0, true);
            }
        });

        // 3: Fairy Ring (mint mushrooms outlining an empty circle)
        render(3, &mut |pb| {
            let mut ring = FairyRing::new(center, 22.0, 3.0, 8, 16.0);
            ring.life = 2.0;
            ring.spin = 0.3;
            draw_fairy_ring(pb, &ring, Vec2::ZERO, 2.2);
        });

        // 4: Ratchet Tail (pink hook + brown body on bright quarter arc)
        render(4, &mut |pb| {
            let mut ps = Particles::new(256);
            let mut rng = StdRng::seed_from_u64(7);
            vfx::emit_ratchet_swing(&mut ps, &mut rng, center + Vec2::new(-6.0, 10.0), 26.0, -1.65, 1.57);
            ps.update(0.05);
            ps.draw(pb, Vec2::ZERO);
        });

        // 5: Drain Cleaner (bile-green advancing chevrons)
        render(5, &mut |pb| {
            let mut ch = CorrosiveChannel::new(
                center + Vec2::new(-20.0, 12.0),
                Vec2::new(0.85, -0.52).normalized(),
                40.0,
                20.0,
                18.0,
            );
            ch.age = 0.45;
            draw_corrosive_channel(pb, &ch, Vec2::ZERO, 2.0);
        });

        // 6: Flywheel (copper pips on tail + copper fan darts)
        render(6, &mut |pb| {
            draw_flywheel_pips(pb, center + Vec2::new(-12.0, 14.0), -0.6, 32.0, 1.0, 4, Vec2::ZERO);
            for i in 0..4 {
                let a = -0.35 + i as f32 * 0.22;
                let d = Vec2::from_angle(a);
                let p = center + Vec2::new(-2.0, 4.0) + d * 24.0;
                draw_flywheel_dart(pb, w2s(p, Vec2::ZERO), d * 150.0);
            }
        });

        // 7: Roll Call (cyan quotation marks winking shut in sequence)
        render(7, &mut |pb| {
            for (slot, &(ox, oy)) in [(-20.0f32, -8.0f32), (0.0, -2.0), (20.0, 6.0)]
                .iter()
                .enumerate()
            {
                let p = w2s(center + Vec2::new(ox, oy), Vec2::ZERO);
                pb.filled_circle(p.0, p.1, 4, (132, 92, 62));
                draw_roll_call_quote(pb, p, 4.0, 0.48 + slot as f32 * 0.22, slot);
            }
        });

        let mut out = format!("P6\n{} {}\n255\n", sheet.w, sheet.h).into_bytes();
        for &(r, g, b) in sheet.pixels() {
            out.push(r);
            out.push(g);
            out.push(b);
        }
        std::fs::write(out_path, out).unwrap();
    }
}
