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

/// Bright aqua-white core of an Acid Rain drop (cool blue-green, distinct from
/// Acid Spit's yellow-lime pools).
const CORE: Rgb = (224, 255, 252);
/// Vivid blue-green / teal body of the vertical rain streaks.
const STROKE: Rgb = (48, 228, 204);
/// Deep sea-teal shadow / trailing mist.
const EMERALD: Rgb = (20, 128, 134);
/// Sickly magenta infection mark highlight, body, shade, and spent socket.
const PIP_HI: Rgb = (255, 214, 252);
const PIP: Rgb = (248, 56, 212);
const PIP_SHADE: Rgb = (164, 28, 146);
const PIP_DIM: Rgb = (86, 34, 88);
const PIP_BG: Rgb = (22, 6, 26);

/// Subtle floor darkening helper for readable ground reticles on bright tiles.
#[inline]
fn shade_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let cur = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, palette::scale(cur, 1.0 - amount));
}

/// Drops the repeats from a ring walked by angle.
///
/// A dotted ring steps round its circle more often than the circle has pixels,
/// so consecutive steps round to the same pixel. With an additive or shading
/// plot that pixel was hit twice and came out brighter than its neighbours —
/// and which pixels doubled changed with the radius and the spin, so a growing
/// or turning ring shimmered. Because the walk is monotonic in angle, a repeat
/// can only be the pixel just plotted, or (closing the loop) the first one.
pub(crate) struct RingDedup {
    first: Option<(i32, i32)>,
    last: Option<(i32, i32)>,
}

impl RingDedup {
    pub(crate) fn new() -> Self {
        Self {
            first: None,
            last: None,
        }
    }

    /// `true` when `p` was already plotted by this walk (skip it).
    pub(crate) fn repeat(&mut self, p: (i32, i32)) -> bool {
        if self.last == Some(p) {
            return true;
        }
        if self.first == Some(p) && self.last.is_some() {
            // Back at the start: only a repeat once the walk has moved on.
            self.last = Some(p);
            return true;
        }
        if self.first.is_none() {
            self.first = Some(p);
        }
        self.last = Some(p);
        false
    }
}

/// Draw one locked Acid Rain drop at screen position `s`.
///
/// Three reads, unmistakably distinct from Acid Spit's lobbed green pools:
///
/// 1. **The shadow dot** — a small dark ground shadow dot with a tiny blue-green
///    pinpoint glint (never a big green pool ring!). Evolved **Downspout** draws
///    a stepped teal-emerald stair tread instead.
/// 2. **The vertical fall** — thin blue-green streaks plunging straight down
///    from above onto the shadow dot.
/// 3. **The ripple** — on impact, a small crisp concentric blue-green ripple
///    ring (never a big pool) + tiny upward droplets.
pub fn draw_acid_drop(pb: &mut PixelBuffer, d: &AcidDrop, s: (i32, i32)) {
    let ring_r = (d.radius.round() as i32).clamp(3, 12);
    // `1 - delay/RAIN_FALL`: 0 while only the shadow dot shows, 1 at impact.
    let fall = ((RAIN_FALL - d.delay) / RAIN_FALL).clamp(0.0, 1.0);
    let dim = 0.58 + 0.42 * fall;
    let phase = d.index as i32;

    // 1. Mark: Downspout (`d.evolved`) uses a crisp stepped teal-emerald stair
    //    tread; base Acid Rain uses a compact ground shadow dot + tiny aqua center.
    if d.evolved {
        let hw = ring_r.clamp(3, 7);
        let dark_riser: Rgb = (12, 48, 52);
        let tread_mid: Rgb = (44, 212, 188);
        let tread_hi: Rgb = (196, 255, 246);
        // Recessed riser shadow directly underneath the stair tread.
        for dx in -hw..=hw {
            blend_px(pb, s.0 + dx, s.1 + 1, dark_riser, 0.72 * dim);
            blend_px(pb, s.0 + dx, s.1 + 2, dark_riser, 0.52 * dim);
        }
        // Bright horizontal stair tread bar across the landing mark.
        for dx in -hw..=hw {
            let edge = dx.abs() == hw;
            let col = if edge { tread_mid } else { tread_hi };
            blend_px(pb, s.0 + dx, s.1 - 1, palette::scale(col, 0.72 + 0.28 * fall), 0.90);
            blend_px(pb, s.0 + dx, s.1, palette::scale(tread_mid, 0.65 + 0.35 * fall), 0.85);
        }
        // Upper step lip offset slightly back to give a two-step staircase silhouette.
        let top_hw = (hw - 1).max(2);
        for dx in -top_hw..=top_hw {
            blend_px(
                pb,
                s.0 + dx,
                s.1 - 3,
                palette::scale(STROKE, 0.55 + 0.40 * fall),
                0.78,
            );
            blend_px(pb, s.0 + dx, s.1 - 2, dark_riser, 0.55 * dim);
        }
        for &(tx, ty) in &[(-hw - 1, -1), (hw + 1, -1), (-hw - 1, 1), (hw + 1, 1)] {
            pb.plot_add(s.0 + tx, s.1 + ty, palette::scale(CORE, 0.45 + 0.45 * fall));
        }
    } else {
        // Compact elliptical dark ground shadow dot directly under the falling drops
        // (deepens slightly as the drop nears impact). Never a big pool!
        let sh = 0.34 + 0.28 * fall;
        for dx in -2..=2i32 {
            shade_px(pb, s.0 + dx, s.1, sh);
            if dx.abs() <= 1 {
                shade_px(pb, s.0 + dx, s.1 - 1, sh * 0.72);
                shade_px(pb, s.0 + dx, s.1 + 1, sh * 0.72);
            }
        }
        // Subtle blue-green pinpoint center so the landing spot is readable even
        // on dark floors during the wind-up.
        pb.plot_add(s.0, s.1, palette::scale(STROKE, 0.45 + 0.45 * fall));
        if fall > 0.25 {
            pb.plot_add(s.0 - 1, s.1, palette::scale(EMERALD, 0.50 * fall));
            pb.plot_add(s.0 + 1, s.1, palette::scale(EMERALD, 0.50 * fall));
        }
    }

    // 2. Thin vertical blue-green streaks falling straight down from above.
    if fall > 0.0 {
        let drop_y = s.1 - (1.0 + (1.0 - fall) * 20.0).round() as i32;
        let bright = (0.55 + 0.45 * fall).clamp(0.25, 1.0);
        for k in 0..3i32 {
            let x = s.0 + (k - 1) * 3;
            let stagger = (((k + phase) % 3) - 1) * 3;
            let y = (drop_y + stagger).min(s.1);
            let is_lead = k == 1;
            let len = if is_lead { 8 } else { 5 };
            // Hot aqua-white needle head at the leading bottom tip.
            pb.plot(x, y, palette::scale(if is_lead { CORE } else { STROKE }, bright));
            if is_lead && y < s.1 {
                pb.plot_add(x, y + 1, palette::scale(CORE, bright * 0.65));
            }
            // Strictly vertical (dx = 0) 1px blue-green rain streak above the head.
            for seg in 1..len {
                let t = seg as f32 / len as f32;
                let sy = y - seg;
                let col = if seg == 1 {
                    palette::mix(CORE, STROKE, 0.45)
                } else {
                    palette::mix(STROKE, EMERALD, t)
                };
                pb.plot_add(x, sy, palette::scale(col, bright * (1.0 - t * 0.62)));
            }
        }
    }

    // 3. Impact frames: small crisp blue-green ripple ring (never a big pool).
    if fall >= 0.72 {
        let k = ((fall - 0.72) / 0.28).clamp(0.0, 1.0);
        let ripple_r = (1.5 + k * (ring_r as f32 * 0.55).clamp(2.5, 4.5)).round() as i32;
        let steps = (ripple_r * 8).max(12);
        let mut seen = RingDedup::new();
        let ring_col = palette::scale(palette::mix(CORE, STROKE, k * 0.65), 1.0 - k * 0.35);
        for i in 0..steps {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            let rx = s.0 + (a.cos() * ripple_r as f32).round() as i32;
            let ry = s.1 + (a.sin() * (ripple_r as f32 * 0.72)).round() as i32;
            if seen.repeat((rx, ry)) {
                continue;
            }
            pb.plot_add(rx, ry, ring_col);
        }
        // Two tiny upward rain-splash flecks right off the center.
        let lift = 1 + (k * 2.0).round() as i32;
        pb.plot_add(s.0 - 1, s.1 - lift, palette::scale(CORE, 0.85 * (1.0 - k * 0.4)));
        pb.plot_add(s.0 + 1, s.1 - lift, palette::scale(STROKE, 0.80 * (1.0 - k * 0.4)));
    }
}

/// Draw the three sickly-magenta Hantavirus pips + host perimeter mark at `s`.
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
            // Lit bio-bead: vivid 2x2 sickly-magenta pearl with pale highlight.
            pb.plot(x, y, PIP_HI);
            pb.plot(x + 1, y, PIP);
            pb.plot(x, y + 1, PIP);
            pb.plot(x + 1, y + 1, PIP_SHADE);
            pb.plot_add(x, y - 1, palette::scale(PIP, 0.65));
        }
    }

    // Four crisp sickly-magenta corner bracket ticks hugging the host so infected
    // bodies read immediately at 1x and 2x zoom-out even under damage numbers.
    let hr = (r + 2).max(3);
    for &(sx, sy) in &[(-1i32, -1i32), (1, -1), (-1, 1), (1, 1)] {
        let cx = s.0 + sx * hr;
        let cy = s.1 + sy * (hr - 1).max(2);
        pb.plot(cx + sx, cy, PIP_BG);
        pb.plot(cx, cy + sy, PIP_BG);
        pb.plot(cx, cy, PIP_HI);
        pb.plot(cx - sx, cy, PIP);
        pb.plot(cx, cy - sy, PIP);
    }
    // Thin vertical sickly-magenta tether linking the bead bar down to the host.
    pb.plot(s.0, y + 3, PIP);
    pb.plot_add(s.0, y + 4, palette::scale(PIP_SHADE, 0.8));
}

// ============================================================================
// Wave-3 Evolution World Art (render-only, zero rng, zero allocation)
// ============================================================================
//
// Direct entity rendering for the persistent & stateful wave-3 evolutions
// (`WAVE3_DESIGN.md` §3 "Half-block read" column), so every evolution reads
// crisply in motion and in still frames regardless of particle load.

use crate::game::ecs::components::{
    CheeseString, CorrosiveChannel, FairyRing, MilkPhase, MilkTooth, Pool,
};
use crate::game::ecs::resources::ActiveFx;
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

/// **Echo Chamber** (Squeak Nova + Frenzy): outbound pale-cyan wavefront arcs
/// become crisp hollow white inward ticks after reflecting off a wall.
pub fn draw_echo_spoke(pb: &mut PixelBuffer, s: (i32, i32), vel: Vec2, reflected: bool) {
    let dir = if vel.len_sq() > 1.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();
    if !reflected {
        // Outbound: thin curved pale-cyan wavefront arc matching Squeak Nova's rings.
        let core: Rgb = (238, 255, 255);
        let cyan: Rgb = (104, 236, 255);
        let tail: Rgb = (56, 174, 224);
        pb.plot(s.0, s.1, core);
        for side in [-1.0f32, 1.0] {
            let w1 = (
                s.0 + (perp.x * side * 1.4 - dir.x * 0.4).round() as i32,
                s.1 + (perp.y * side * 1.4 - dir.y * 0.4).round() as i32,
            );
            pb.plot_add(w1.0, w1.1, cyan);
        }
        let back = (
            s.0 - (dir.x * 2.0).round() as i32,
            s.1 - (dir.y * 2.0).round() as i32,
        );
        pb.plot_add(back.0, back.1, tail);
    } else {
        // Reflected inward: crisp hollow white chevron tick (`>`) racing back!
        let white: Rgb = (255, 254, 255);
        let ice: Rgb = (168, 238, 255);
        let dark: Rgb = (16, 30, 42);
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
            // Soft pale-cyan echo wake behind each wing.
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
    let mut seen = RingDedup::new();
    for i in (0..steps).step_by(2) {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU + ring.spin * 0.3;
        let rx = center.0 + (a.cos() * ring.radius).round() as i32;
        let ry = center.1 + (a.sin() * ring.radius).round() as i32;
        if seen.repeat((rx, ry)) {
            continue;
        }
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
    let dir = if ch.dir.len_sq() > 1e-4 {
        ch.dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let side = dir.perp();

    // Downspout runoff (`ch.runoff == true`): settled immediately for `ch.settle`
    // seconds and drawn as a crisp stepped green staircase (alternating bright
    // lime treads and deep dark-emerald risers framed by wet-stone stringers)
    // with animated runoff cascading down the steps.
    if ch.runoff {
        let total = ch.settle.max(0.05);
        let fade = ((total - ch.age) / 0.35).clamp(0.0, 1.0);
        if fade <= 0.01 {
            return;
        }
        let stringer: Rgb = (8, 30, 34);
        let riser_dark: Rgb = (12, 46, 52);
        let riser_mid: Rgb = (28, 134, 134);
        let tread_mid: Rgb = (48, 224, 198);
        let tread_hi: Rgb = (182, 255, 242);
        let foam: Rgb = (232, 255, 254);

        let steps_px = ch.length.ceil() as i32;
        let step_stride = 6i32;
        // Animated caustic runoff wave cascading step-by-step down the gutter.
        let cascade = ((elapsed * 18.0) as i32).rem_euclid(step_stride);

        for along in 0..=steps_px {
            let af = along as f32;
            let phase = along.rem_euclid(step_stride);
            let is_lip = phase == 0;
            let is_tread = phase == 1 || phase == 2;
            let is_cascade = phase == cascade;
            // Notched stair silhouette: wide lit horizontal treads (`±5`), narrow
            // jet-dark vertical risers (`±3`) so every step reads cleanly at 1x!
            let width = if is_lip || is_tread { 5i32 } else { 3i32 };

            for w in -width..=width {
                let p = w2s(ch.start + dir * af + side * w as f32, cam);
                if w.abs() == width {
                    // Crisp dark stair-stringer side border.
                    blend_px(pb, p.0, p.1, stringer, 0.88 * fade);
                } else if is_lip {
                    // Blazing leading stair-tread nose.
                    let col = if is_cascade && w.abs() <= 1 {
                        foam
                    } else if w.abs() <= 3 {
                        tread_hi
                    } else {
                        tread_mid
                    };
                    blend_px(pb, p.0, p.1, col, 0.96 * fade);
                } else if is_tread {
                    // Flat lit lime step tread with crisp edge shading.
                    let col = if phase == 1 { tread_mid } else { riser_mid };
                    blend_px(pb, p.0, p.1, col, (0.86 - 0.06 * w.abs() as f32) * fade);
                    if is_cascade && w.abs() <= 1 {
                        pb.plot_add(p.0, p.1, palette::scale(tread_hi, 0.45 * fade));
                    }
                } else {
                    // Deep recessed vertical stair riser shadow between treads.
                    blend_px(pb, p.0, p.1, riser_dark, 0.92 * fade);
                    if is_cascade && w == 0 {
                        pb.plot_add(p.0, p.1, palette::scale(tread_mid, 0.40 * fade));
                    }
                }
            }
        }
        return;
    }

    let total = CHANNEL_ADVANCE + CHANNEL_SETTLE;
    let fade = ((total - ch.age) / 0.42).clamp(0.0, 1.0);
    if fade <= 0.01 {
        return;
    }
    let advancing = ch.age <= CHANNEL_ADVANCE;
    let open_len = ch.length * (ch.age / CHANNEL_ADVANCE).clamp(0.0, 1.0);

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

/// A cross-section of the sweep, fading toward the outer edge.
pub(super) fn draw_dash_lane_slice(
    pb: &mut PixelBuffer,
    centre: Vec2,
    across: Vec2,
    radius: f32,
    color: Rgb,
    alpha: f32,
) {
    let reach = radius.ceil() as i32;
    let inv_radius = 1.0 / radius;
    for k in -reach..=reach {
        let offset = k as f32;
        let cover = (radius + 0.5 - offset.abs()).clamp(0.0, 1.0);
        let fade = 1.0 - 0.75 * (offset.abs() * inv_radius).min(1.0);
        let p = centre + across * offset;
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, color, alpha * cover * fade);
    }
}

/// Cobalt width cue beneath Ripcord's braided centre and sliding knots.
pub(super) fn draw_ripcord_lane(
    pb: &mut PixelBuffer,
    route: &[(Vec2, Vec2)],
    radius: f32,
    life_frac: f32,
    cam: Vec2,
) {
    let alpha = 0.06 + 0.08 * life_frac;
    for &(from, to) in route {
        let leg = to - from;
        let steps = leg.len().ceil() as i32;
        let across = leg.normalized().perp();
        for i in 0..steps {
            let p = from - cam + leg * (i as f32 / steps as f32);
            draw_dash_lane_slice(pb, p, across, radius, (62, 146, 255), alpha);
        }
    }
}

/// **Super Dash** kinetic blue-white comb drawn cleanly over the swept corridor
/// (above enemy hit-flashes so the comb teeth visibly cut through crossed bodies).
#[allow(clippy::too_many_arguments)]
pub fn draw_super_dash_comb(
    pb: &mut PixelBuffer,
    head: Vec2,
    dir: Vec2,
    length: f32,
    radius: f32,
    scale: f32,
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
    let steps = length.ceil().max(6.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = head - dir * (t * length);
        let s = w2s(p, cam);
        let a = (1.0 - t * 0.65) * fade;
        blend_px(pb, s.0, s.1, white, (0.92 * a).clamp(0.0, 1.0));
        for k in 1..=scale.ceil() as i32 {
            let gain = (scale + 1.0 - k as f32).clamp(0.0, 1.0);
            for side in [-1.0f32, 1.0] {
                let sh = w2s(p + perp * (side * k as f32), cam);
                pb.plot_add(sh.0, sh.1, palette::scale(cyan, 0.65 * a * gain));
            }
        }
    }

    // 2. Crisp evenly-spaced perpendicular comb teeth raking OUT PAST the crossed
    //    bodies (`radius + 4.5 px`), with dark-cobalt edge contrast so they pop
    //    sharply even over bright enemy white hit flashes!
    let tooth_reach = radius + 4.5 * scale;
    let spacing = (3.2 * scale).max(2.0);
    let mut dist = scale;
    while dist <= length {
        let t = (dist / length.max(1.0)).clamp(0.0, 1.0);
        let a = (1.0 - t * 0.55) * fade;
        let root = head - dir * dist;
        let reach = tooth_reach * (1.0 - 0.25 * t);
        let max_k = reach.ceil() as i32;
        for side in [-1.0f32, 1.0] {
            for k in (scale.ceil() as i32 + 1).min(max_k)..=max_k {
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
                    let shadow = w2s(tp - dir * scale, cam);
                    blend_px(pb, shadow.0, shadow.1, navy, 0.55 * a);
                }
                blend_px(pb, ts.0, ts.1, col, (0.90 * a * (1.0 - kt * 0.25)).clamp(0.0, 1.0));
            }
        }
        dist += spacing;
    }

    // 3. Leading blue-white spearhead chevron right ahead of the rat's nose.
    let tip = head + dir * (4.5 * scale);
    let steps = (5.0 * scale).ceil() as i32;
    for k in 0..=steps {
        let kf = 5.0 * k as f32 / steps as f32;
        for side in [-1.0f32, 1.0] {
            let p = tip + (perp * (side * kf * 0.85) - dir * (kf * 0.9)) * scale;
            let s = w2s(p, cam);
            let col = if kf <= 2.0 { white } else { cyan };
            blend_px(pb, s.0, s.1, col, (0.95 - kf * 0.12) * fade);
        }
    }
}

// ============================================================================
// Batch 12: Distinct per-weapon shapes (render-only, zero rng, zero alloc)
// ============================================================================

/// **Cheese Spray**: a fat tumbling warm yellow-orange triangular wedge that
/// slows down and drops, trailing a single crumb behind it.
pub fn draw_cheese_wedge(
    pb: &mut PixelBuffer,
    s: (i32, i32),
    vel: Vec2,
    spin: f32,
    life: f32,
) {
    let hi: Rgb = (255, 250, 188);
    let body: Rgb = (255, 210, 48);
    let rind: Rgb = (238, 128, 22);
    let hole: Rgb = (196, 108, 18);
    let rim: Rgb = (52, 26, 10);

    // Subtle settle drop right before expiring.
    let drop_y = if life < 0.18 { 1 } else { 0 };
    let cx = s.0;
    let cy = s.1 + drop_y;

    // Three vertices of a chunky isosceles cheese wedge rotated by `spin`.
    let (sa, ca) = spin.sin_cos();
    let rot = |x: f32, y: f32| -> (f32, f32) { (x * ca - y * sa, x * sa + y * ca) };
    let v0 = rot(2.8, 0.0); // pointed wedge tip
    let v1 = rot(-2.0, -2.2); // upper rind corner
    let v2 = rot(-2.0, 2.2); // lower rind corner

    let edge = |a: (f32, f32), b: (f32, f32), px: f32, py: f32| -> f32 {
        (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0)
    };
    let area = edge(v0, v1, v2.0, v2.1);
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };

    // Ground drop shadow pixel directly beneath the wedge.
    shade_px(pb, cx + 1, cy + 3, 0.38);
    shade_px(pb, cx, cy + 3, 0.30);

    for dy in -3..=3i32 {
        for dx in -3..=3i32 {
            let px = dx as f32;
            let py = dy as f32;
            let e0 = edge(v0, v1, px, py) * sign;
            let e1 = edge(v1, v2, px, py) * sign;
            let e2 = edge(v2, v0, px, py) * sign;
            if e0 >= -0.35 && e1 >= -0.35 && e2 >= -0.35 {
                // Classify pixel: rind along the base edge (v1..v2), butter-white
                // highlight near the pointed tip (v0), warm cheddar inside.
                let tip_d2 = (px - v0.0).powi(2) + (py - v0.1).powi(2);
                let col = if e1 < 1.15 {
                    rind
                } else if tip_d2 <= 2.4 {
                    hi
                } else if dx == 0 && dy == 0 {
                    hole
                } else {
                    body
                };
                pb.plot(cx + dx, cy + dy, col);
            } else if e0 >= -1.6 && e1 >= -1.6 && e2 >= -1.6 && dy >= 0 {
                // Crisp dark lower-rim contrast so yellow reads on bright Kitchen floors.
                blend_px(pb, cx + dx, cy + dy, rim, 0.58);
            }
        }
    }

    // Trailing cheddar crumb behind the travel vector.
    if vel.len_sq() > 16.0 {
        let dir = vel.normalized();
        let cr = (
            cx - (dir.x * 3.4).round() as i32,
            cy - (dir.y * 3.4).round() as i32,
        );
        pb.plot_add(cr.0, cr.1, palette::scale(rind, 0.85));
    }
}

/// **Fondue Fork** outer tine in flight: a gleaming two-pronged golden fork tip
/// racing out to plant one end of the sagging cheese string.
pub fn draw_fondue_tine(pb: &mut PixelBuffer, s: (i32, i32), vel: Vec2) {
    let dir = if vel.len_sq() > 1.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();
    let hot: Rgb = (255, 252, 206);
    let gold: Rgb = (255, 204, 54);
    let ochre: Rgb = (176, 108, 18);

    // Shaft back, twin prongs forward.
    let shaft = (
        s.0 - (dir.x * 2.0).round() as i32,
        s.1 - (dir.y * 2.0).round() as i32,
    );
    pb.plot(shaft.0, shaft.1, ochre);
    pb.plot(s.0, s.1, gold);
    for side in [-1.0f32, 1.0] {
        let p1 = (
            s.0 + (dir.x * 1.0 + perp.x * side * 1.1).round() as i32,
            s.1 + (dir.y * 1.0 + perp.y * side * 1.1).round() as i32,
        );
        let p2 = (
            s.0 + (dir.x * 2.2 + perp.x * side * 1.1).round() as i32,
            s.1 + (dir.y * 2.2 + perp.y * side * 1.1).round() as i32,
        );
        pb.plot(p1.0, p1.1, gold);
        pb.plot(p2.0, p2.1, hot);
    }
}

/// **Squeak Nova**: two or three thin concentric pale-cyan rings racing outward
/// from `fx.nova_pos` and fading; evolved **Echo Chamber** rebounds inward as
/// a crisp dashed hollow-white ring.
pub fn draw_squeak_nova_ring(pb: &mut PixelBuffer, fx: &ActiveFx, cam: Vec2) {
    if !fx.nova_active() || fx.nova_max_r <= 1.0 {
        return;
    }
    let center = w2s(fx.nova_pos, cam);
    let r = fx.nova_r.max(0.5);
    let frac = (r / fx.nova_max_r).clamp(0.0, 1.0);
    let inbound = fx.nova_echo && fx.nova_r < fx.nova_prev_r;

    // Fade gently as the wave reaches its peak, staying legible at full reach.
    let base_fade = if inbound {
        (0.45 + 0.55 * frac).clamp(0.25, 0.92)
    } else {
        (1.0 - frac * 0.62).clamp(0.28, 1.0)
    };

    let lead_core: Rgb = (242, 255, 255);
    let lead_cyan: Rgb = (112, 240, 255);
    let mid_cyan: Rgb = (78, 212, 248);
    let tail_cyan: Rgb = (52, 168, 220);
    let dark_in: Rgb = (16, 30, 42);

    // Three thin concentric rings (lead + two trailing harmonics).
    let rings: [(f32, Rgb, f32, bool); 3] = if inbound {
        // Echo Chamber inbound contraction: crisp dashed white rebound lead +
        // dimmer pale-cyan trailing rings.
        [
            (r, (255, 255, 255), base_fade, true),
            ((r * 1.12).min(fx.nova_max_r), lead_cyan, base_fade * 0.52, false),
            ((r * 0.84).max(2.0), mid_cyan, base_fade * 0.36, false),
        ]
    } else {
        [
            (r, palette::mix(lead_core, lead_cyan, frac * 0.65), base_fade, false),
            (r * 0.78, mid_cyan, base_fade * 0.65, false),
            (r * 0.56, tail_cyan, base_fade * 0.38, false),
        ]
    };

    for (rad, col, weight, dashed) in rings {
        if rad < 2.0 || weight <= 0.04 {
            continue;
        }
        let steps = (rad * 6.5).round().clamp(16.0, 320.0) as i32;
        let mut seen = RingDedup::new();
        for i in 0..steps {
            if dashed && i % 3 == 2 {
                continue;
            }
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            let px = center.0 + (a.cos() * rad).round() as i32;
            let py = center.1 + (a.sin() * rad).round() as i32;
            if seen.repeat((px, py)) {
                continue;
            }
            if dashed {
                // Dark inner contrast pixel makes the inbound Echo ring read hollow white.
                let ix = center.0 + (a.cos() * (rad - 1.2)).round() as i32;
                let iy = center.1 + (a.sin() * (rad - 1.2)).round() as i32;
                if (ix, iy) != (px, py) {
                    blend_px(pb, ix, iy, dark_in, 0.55 * weight);
                }
                blend_px(pb, px, py, col, (0.95 * weight).clamp(0.0, 1.0));
            } else {
                pb.plot_add(px, py, palette::scale(col, weight));
            }
        }
    }
}

/// **Sonic Squeak** (ACTIVE): a directional forward wedge of nested pure-white
/// chevrons (`>`) pushing outward along `fx.squeak_facing` — NEVER a ring!
pub fn draw_sonic_cone(pb: &mut PixelBuffer, fx: &ActiveFx, cam: Vec2) {
    if !fx.squeak_active() || fx.squeak_max_r <= 2.0 {
        return;
    }
    let dir = if fx.squeak_facing.len_sq() > 1e-4 {
        fx.squeak_facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let half = if fx.squeak_half > 0.05 {
        fx.squeak_half
    } else {
        0.66
    };
    let prog = (fx.squeak_r / fx.squeak_max_r).clamp(0.0, 1.0);
    let fade = (1.0 - prog * 0.55).clamp(0.25, 1.0);

    let white: Rgb = (255, 255, 255);
    let ice: Rgb = (204, 238, 255);
    let rim_dark: Rgb = (18, 26, 38);

    let edge_l = dir.rotate(-half);
    let edge_r = dir.rotate(half);

    // Subtle cone-boundary dashed guide ticks so the forward wedge angle reads instantly.
    let rail_r = fx.squeak_r.max(8.0);
    let rail_steps = ((rail_r / 5.0) as i32).clamp(2, 16);
    for i in 1..=rail_steps {
        let d = i as f32 * (rail_r / rail_steps as f32);
        let a = (0.35 * fade * (1.0 - i as f32 / (rail_steps as f32 + 2.0))).max(0.08);
        for edge_dir in [edge_l, edge_r] {
            let s = w2s(fx.squeak_pos + edge_dir * d, cam);
            pb.plot_add(s.0, s.1, palette::scale(ice, a));
        }
    }

    // Three nested crisp forward-pointing white chevrons (`>`) marching outward.
    // Each chevron is two straight wings meeting at a sharp apex on `dir`
    // (angular V-wedge silhouette — unmistakable against Squeak Nova's circular rings).
    let layers: [(f32, f32, bool); 3] = [
        (fx.squeak_r, 1.0, true),
        (fx.squeak_r * 0.72, 0.76, false),
        (fx.squeak_r * 0.45, 0.48, false),
    ];

    for (rad, weight, thick) in layers {
        if rad < 5.0 {
            continue;
        }
        let apex = fx.squeak_pos + dir * rad;
        let wing_l = fx.squeak_pos + edge_l * (rad * 0.72);
        let wing_r = fx.squeak_pos + edge_r * (rad * 0.72);
        let alpha = (weight * fade).clamp(0.0, 1.0);

        for wing in [wing_l, wing_r] {
            let span = apex.dist(wing);
            let steps = (span * 1.4).ceil().clamp(4.0, 96.0) as i32;
            let mut last = (i32::MIN, i32::MIN);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let p = apex + (wing - apex) * t;
                let s = w2s(p, cam);
                if s == last {
                    continue;
                }
                last = s;
                let taper = 1.0 - t * 0.35;
                if thick {
                    // Subtle dark backing directly ahead of the lead chevron so pure
                    // white reads crisply even on bright Kitchen / Lab floors.
                    let ahead = w2s(p + dir * 1.2, cam);
                    if ahead != s {
                        blend_px(pb, ahead.0, ahead.1, rim_dark, 0.45 * alpha * taper);
                    }
                    blend_px(pb, s.0, s.1, white, (0.96 * alpha * taper).clamp(0.0, 1.0));
                    // 1px ice-white trailing thickness directly behind the lead chevron.
                    let back = w2s(p - dir * 1.1, cam);
                    if back != s {
                        pb.plot_add(back.0, back.1, palette::scale(ice, 0.75 * alpha * taper));
                    }
                } else {
                    blend_px(
                        pb,
                        s.0,
                        s.1,
                        palette::mix(white, ice, 0.35),
                        (0.88 * alpha * taper).clamp(0.0, 1.0),
                    );
                }
            }
        }
        // Bright apex glint right on the tip of the chevron.
        let sa = w2s(apex, cam);
        pb.plot(sa.0, sa.1, palette::scale(white, (0.85 + 0.15 * alpha).min(1.0)));
    }
}

/// **Spore Orbit**: faint dotted mint-green orbit track circling the rat so the
/// puffballs visibly ride a ring.
pub fn draw_spore_orbit_track(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    elapsed: f32,
    evolved: bool,
    cam: Vec2,
) {
    if radius < 4.0 {
        return;
    }
    let c = w2s(center, cam);
    let dots = ((radius * 1.2).round() as i32).clamp(16, 40);
    let col: Rgb = if evolved { (114, 240, 178) } else { (68, 188, 118) };
    let spin = elapsed * 0.45;
    let mut seen = RingDedup::new();
    for i in 0..dots {
        let a = spin + i as f32 / dots as f32 * std::f32::consts::TAU;
        let px = c.0 + (a.cos() * radius).round() as i32;
        let py = c.1 + (a.sin() * radius).round() as i32;
        if seen.repeat((px, py)) {
            continue;
        }
        pb.plot_add(px, py, palette::scale(col, 0.36));
    }
}

/// **Spore Orbit** base orbiter: a plump 5×5 round green puffball with soft
/// organic tufts and a luminous mint-white core.
pub fn draw_spore_orbiter(pb: &mut PixelBuffer, s: (i32, i32), elapsed: f32) {
    let (cx, cy) = s;
    let out: Rgb = (16, 46, 26);
    let dark: Rgb = (34, 126, 62);
    let mid: Rgb = (96, 236, 132);
    let hi: Rgb = (228, 255, 224);

    // Subtle dark outer silhouette rim on bottom/right for bright-floor contrast.
    for &(dx, dy) in &[(2, 1), (1, 2), (0, 2), (-1, 2), (-2, 1)] {
        blend_px(pb, cx + dx, cy + dy, out, 0.58);
    }

    // Plump rounded puffball body (3×3 core + cardinal shoulders).
    pb.plot(cx - 1, cy - 1, hi);
    pb.plot(cx, cy - 1, hi);
    pb.plot(cx + 1, cy - 1, mid);
    pb.plot(cx - 1, cy, hi);
    pb.plot(cx, cy, mid);
    pb.plot(cx + 1, cy, dark);
    pb.plot(cx - 1, cy + 1, mid);
    pb.plot(cx, cy + 1, dark);
    pb.plot(cx + 1, cy + 1, dark);

    // Soft cardinal puff tufts gently alternating with `elapsed`.
    let alt = ((elapsed * 4.0) as i32).rem_euclid(2);
    let tuft_hi = palette::scale(mid, 0.72);
    let tuft_lo = palette::scale(dark, 0.72);
    pb.plot_add(cx, cy - 2, tuft_hi);
    pb.plot_add(cx - 2, cy, tuft_hi);
    pb.plot_add(cx + 2, cy, tuft_lo);
    pb.plot_add(cx, cy + 2, tuft_lo);
    if alt == 0 {
        pb.plot_add(cx - 2, cy - 1, palette::scale(mid, 0.45));
        pb.plot_add(cx + 2, cy + 1, palette::scale(dark, 0.45));
    } else {
        pb.plot_add(cx + 1, cy - 2, palette::scale(mid, 0.45));
        pb.plot_add(cx - 1, cy + 2, palette::scale(dark, 0.45));
    }
}

/// **Acid Spit** in flight: a single large green blob travelling on a parabolic
/// arc above its own travelling ground shadow.
pub fn draw_acid_glob(
    pb: &mut PixelBuffer,
    pos: Vec2,
    target: Vec2,
    player_pos: Vec2,
    vel: Vec2,
    color: Rgb,
    cam: Vec2,
) {
    let ground = w2s(pos, cam);
    let dist_left = pos.dist(target);
    let span = player_pos.dist(target).max(dist_left).max(16.0);
    let u = (1.0 - dist_left / span).clamp(0.0, 1.0);
    // Smooth parabolic arc: 0 at launch & landing, peaking cleanly mid-flight.
    let arc = 4.0 * u * (1.0 - u);
    let arc_h = (span * 0.22).clamp(6.0, 18.0);
    let lift = (arc * arc_h).round() as i32;

    // 1. Travelling elliptical ground shadow right on the floor at `pos`.
    let sh = 0.34 + 0.24 * (1.0 - arc);
    let rw = if arc > 0.55 { 2 } else { 3 };
    for dx in -rw..=rw {
        shade_px(pb, ground.0 + dx, ground.1, sh);
        if dx.abs() < rw {
            shade_px(pb, ground.0 + dx, ground.1 - 1, sh * 0.65);
            shade_px(pb, ground.0 + dx, ground.1 + 1, sh * 0.65);
        }
    }
    pb.plot_add(ground.0, ground.1, palette::scale(color, 0.22));

    // 2. Single large green blob lifted above its ground shadow.
    let (bx, by) = (ground.0, ground.1 - lift);
    let rim: Rgb = (22, 54, 14);
    let deep: Rgb = (68, 174, 30);
    let body: Rgb = palette::mix(color, (148, 246, 56), 0.5);
    let hi: Rgb = (238, 255, 162);

    // Dark outline around the 5×5 rounded glob.
    for &(dx, dy) in &[
        (-2, -2), (2, -2), (-2, 2), (2, 2),
        (-3, 0), (3, 0), (0, -3), (0, 3),
    ] {
        blend_px(pb, bx + dx, by + dy, rim, 0.62);
    }
    // Chunky bright-rimmed glob interior.
    for dy in -2..=2i32 {
        for dx in -2..=2i32 {
            if dx.abs() == 2 && dy.abs() == 2 {
                continue;
            }
            let col = if dx <= 0 && dy <= 0 && (dx + dy) >= -2 {
                if dx == -1 && dy == -1 { hi } else { body }
            } else if dx == 2 || dy == 2 {
                deep
            } else {
                body
            };
            pb.plot(bx + dx, by + dy, col);
        }
    }
    // Wet glossy specular core + trailing droplet behind velocity.
    pb.plot(bx - 1, by - 1, hi);
    pb.plot(bx, by - 1, hi);
    if vel.len_sq() > 16.0 {
        let dir = vel.normalized();
        let tx = bx - (dir.x * 3.2).round() as i32;
        let ty = by - (dir.y * 3.2).round() as i32;
        pb.plot(tx, ty, deep);
        pb.plot_add(tx, ty - 1, palette::scale(body, 0.7));
    }
}

/// **Acid Spit** landed pool: a dark translucent toxic interior ringed by a
/// bright sizzling lime rim and slow animated surface bubbles.
pub fn draw_acid_pool(
    pb: &mut PixelBuffer,
    pool: &Pool,
    s: (i32, i32),
    elapsed: f32,
    seed: u64,
) {
    let fade = (pool.life / 0.55).clamp(0.18, 1.0);
    let r = (pool.radius.round() as i32).clamp(4, 24);
    let rf = pool.radius.max(3.0);
    let r2 = rf * rf;

    let fill_dark: Rgb = (26, 72, 16);
    let fill_mid: Rgb = (58, 152, 28);
    let rim_bright: Rgb = (168, 255, 62);
    let rim_hot: Rgb = (238, 255, 168);

    // 1. Translucent dark toxic interior (clean, no noisy checkerboard).
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = (dx * dx + dy * dy) as f32;
            if d2 <= r2 {
                let norm = (d2 / r2).sqrt();
                let col = if norm > 0.76 { fill_mid } else { fill_dark };
                let a = (0.48 + 0.16 * norm) * fade;
                blend_px(pb, s.0 + dx, s.1 + dy, col, a);
            }
        }
    }

    // 2. Bright sizzling lime perimeter rim (deduplicated, zero shimmer).
    let steps = (r * 7).max(20);
    let mut seen = RingDedup::new();
    let shimmer_phase = ((elapsed * 3.0) as i32).wrapping_add(seed as i32);
    for i in 0..steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        let px = s.0 + (a.cos() * rf).round() as i32;
        let py = s.1 + (a.sin() * rf).round() as i32;
        if seen.repeat((px, py)) {
            continue;
        }
        let hot_arc = (i + shimmer_phase).rem_euclid(7) <= 1;
        let col = if hot_arc { rim_hot } else { rim_bright };
        blend_px(pb, px, py, col, (0.92 * fade).clamp(0.0, 1.0));
    }

    // 3. 3..5 deterministic animated surface bubbles inside the pool.
    let bubbles = (r / 4).clamp(3, 5) as u64;
    for k in 0..bubbles {
        let h = seed
            .wrapping_mul(2654435761)
            .wrapping_add(k.wrapping_mul(1013904223));
        let ang = ((h & 0xFF) as f32 / 255.0) * std::f32::consts::TAU;
        let dist = (((h >> 8) & 0xFF) as f32 / 255.0) * (rf * 0.65);
        let cycle = (elapsed * 1.8 + ((h >> 16) & 0xFF) as f32 * 0.03).fract();
        let bx = s.0 + (ang.cos() * dist).round() as i32;
        let by = s.1 + (ang.sin() * dist).round() as i32;
        if cycle < 0.45 {
            // Rising small blister.
            pb.plot(bx, by, palette::scale(rim_bright, fade));
        } else if cycle < 0.80 {
            // Plump 2×2 bubble with bright white-lime specular dot.
            pb.plot(bx, by, palette::scale(rim_hot, fade));
            pb.plot(bx + 1, by, palette::scale(rim_bright, fade));
            pb.plot(bx, by + 1, palette::scale(rim_bright, fade));
            pb.plot(bx + 1, by + 1, palette::scale(fill_dark, fade));
        } else {
            // Tiny pop ring.
            for &(ox, oy) in &[(-1, 0), (1, 0), (0, -1), (0, 1)] {
                pb.plot_add(bx + ox, by + oy, palette::scale(rim_hot, 0.72 * fade));
            }
        }
    }
}

/// **Spiked Tail** broken-off shard (L4+): a razor steel-grey thorn flying on
/// past the overhead slam.
pub fn draw_spike_shard(pb: &mut PixelBuffer, s: (i32, i32), vel: Vec2) {
    let dir = if vel.len_sq() > 1.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let white: Rgb = (255, 255, 255);
    let steel: Rgb = (196, 206, 222);
    let dark: Rgb = (58, 64, 78);

    let tip = (
        s.0 + (dir.x * 1.6).round() as i32,
        s.1 + (dir.y * 1.6).round() as i32,
    );
    let tail = (
        s.0 - (dir.x * 1.8).round() as i32,
        s.1 - (dir.y * 1.8).round() as i32,
    );
    pb.plot(tail.0, tail.1, dark);
    pb.plot(s.0, s.1, steel);
    pb.plot(tip.0, tip.1, white);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::particle::{vfx, Particles};

    #[test]
    fn super_dash_comb_width_follows_the_dash_scale() {
        use crate::game::ecs::systems::weapons_wave3b::super_dash_radius;

        let reach = |scale| {
            let head = Vec2::new(100.0, 80.0);
            let mut pb = PixelBuffer::new(160, 160);
            let radius = super_dash_radius(4.0, 1.0, scale);
            draw_super_dash_comb(
                &mut pb, head, Vec2::new(1.0, 0.0), 52.0, radius, scale, 1.0, Vec2::ZERO,
            );
            let outer = (0..80)
                .find(|&y| (0..100).any(|x| pb.pixel_at(x, y) != (0, 0, 0)))
                .unwrap();
            let covers_offset = (0..100).any(|x| pb.pixel_at(x, 80 - 16) != (0, 0, 0));
            ((80 - outer) as f32, covers_offset)
        };
        let (normal, normal_covers_offset) = reach(1.0);
        let (giant, giant_covers_offset) = reach(5.0);
        let (tiny, _) = reach(0.35);
        assert!((giant - normal * 5.0).abs() <= 4.0, "{giant} vs {normal}");
        assert!((tiny - normal * 0.35).abs() <= 1.0, "{tiny} vs {normal}");
        assert!(!normal_covers_offset);
        assert!(giant_covers_offset);
    }

    /// A dotted ring lights each pixel at most once, at every radius: the
    /// additive plot used to land twice on some pixels.
    #[test]
    fn dotted_rings_plot_each_pixel_once() {
        // `RingDedup` on a closed walk: no pixel is ever let through twice.
        for r in 1..=14i32 {
            let steps = (r * 8).max(16);
            let mut seen = RingDedup::new();
            let mut plotted: Vec<(i32, i32)> = Vec::new();
            for i in 0..steps {
                let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                let p = (
                    (a.cos() * r as f32).round() as i32,
                    (a.sin() * r as f32).round() as i32,
                );
                if !seen.repeat(p) {
                    assert!(!plotted.contains(&p), "radius {r}: {p:?} plotted twice");
                    plotted.push(p);
                }
            }
            assert!(plotted.len() >= 4, "radius {r}: the ring vanished");
        }
    }

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

        // 4: Ratchet Tail (pink hook + brown body thrown forward)
        render(4, &mut |pb| {
            let mut ps = Particles::new(256);
            vfx::emit_ratchet_throw(
                &mut ps,
                center + Vec2::new(-24.0, 0.0),
                center + Vec2::new(18.0, 0.0),
            );
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

    #[test]
    fn batch12_weapon_shapes_render_distinctly_without_panic() {
        let mut pb = PixelBuffer::new(96, 96);
        let bg = (16, 20, 22);

        let count_lit = |buf: &PixelBuffer| -> usize {
            (0..buf.h)
                .flat_map(|y| (0..buf.w).map(move |x| (x, y)))
                .filter(|&(x, y)| buf.pixel_at(x, y) != bg)
                .count()
        };

        // Cheese wedge + Fondue tine.
        pb.clear(bg);
        draw_cheese_wedge(&mut pb, (32, 32), Vec2::new(78.0, 0.0), 0.7, 0.4);
        draw_fondue_tine(&mut pb, (60, 32), Vec2::new(110.0, 20.0));
        assert!(count_lit(&pb) >= 14, "cheese wedge and tine should paint crisp shapes");

        // Squeak Nova expanding ring + Echo Chamber inbound ring.
        pb.clear(bg);
        let mut fx = ActiveFx {
            nova_left: 0.25,
            nova_age: 0.15,
            nova_prev_r: 18.0,
            nova_r: 22.0,
            nova_max_r: 50.0,
            nova_pos: Vec2::new(48.0, 48.0),
            ..Default::default()
        };
        draw_squeak_nova_ring(&mut pb, &fx, Vec2::ZERO);
        assert!(count_lit(&pb) >= 40, "squeak nova rings should paint concentric cyan rings");
        fx.nova_echo = true;
        fx.nova_prev_r = 26.0;
        fx.nova_r = 22.0;
        draw_squeak_nova_ring(&mut pb, &fx, Vec2::ZERO);

        // Sonic Squeak directional chevron cone (never a ring).
        pb.clear(bg);
        fx.squeak_left = 0.20;
        fx.squeak_r = 34.0;
        fx.squeak_max_r = 60.0;
        fx.squeak_pos = Vec2::new(24.0, 48.0);
        fx.squeak_facing = Vec2::new(1.0, 0.0);
        fx.squeak_half = 0.66;
        draw_sonic_cone(&mut pb, &fx, Vec2::ZERO);
        assert!(count_lit(&pb) >= 30, "sonic cone should paint nested white chevrons");
        // Verify nothing was painted behind the apex (proving it is a cone, never a ring).
        let behind = (0..20usize)
            .flat_map(|x| (0..pb.h).map(move |y| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != bg)
            .count();
        assert_eq!(behind, 0, "sonic squeak cone must never draw behind its apex");

        // Spore orbit track + puffball.
        pb.clear(bg);
        draw_spore_orbit_track(&mut pb, Vec2::new(48.0, 48.0), 22.0, 1.0, false, Vec2::ZERO);
        draw_spore_orbiter(&mut pb, (70, 48), 1.0);
        assert!(count_lit(&pb) >= 24, "spore orbit track and puffball should draw");

        // Acid Spit lobbed glob + bubbling pool + Acid Rain vertical drop + Spike shard.
        pb.clear(bg);
        draw_acid_glob(
            &mut pb,
            Vec2::new(40.0, 48.0),
            Vec2::new(72.0, 48.0),
            Vec2::new(16.0, 48.0),
            Vec2::new(110.0, 0.0),
            (160, 255, 90),
            Vec2::ZERO,
        );
        let pool = Pool {
            pos: Vec2::new(72.0, 48.0),
            radius: 10.0,
            dps: 16.0,
            life: 2.0,
            color: (140, 220, 70),
        };
        draw_acid_pool(&mut pb, &pool, (72, 48), 1.3, 7);
        draw_spike_shard(&mut pb, (20, 20), Vec2::new(150.0, 0.0));
        assert!(count_lit(&pb) >= 80, "acid glob, pool and shard should draw");

        // Zero-size buffer must never panic.
        let mut zero = PixelBuffer::new(0, 0);
        draw_cheese_wedge(&mut zero, (0, 0), Vec2::ZERO, 0.0, 0.0);
        draw_fondue_tine(&mut zero, (0, 0), Vec2::ZERO);
        draw_squeak_nova_ring(&mut zero, &fx, Vec2::ZERO);
        draw_sonic_cone(&mut zero, &fx, Vec2::ZERO);
        draw_spore_orbit_track(&mut zero, Vec2::ZERO, 20.0, 0.0, false, Vec2::ZERO);
        draw_spore_orbiter(&mut zero, (0, 0), 0.0);
        draw_acid_glob(&mut zero, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, (160, 255, 90), Vec2::ZERO);
        draw_acid_pool(&mut zero, &pool, (0, 0), 0.0, 0);
        draw_spike_shard(&mut zero, (0, 0), Vec2::ZERO);
    }
}
