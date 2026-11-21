//! Art for the player's rat. Pure drawing from the player's state.
//!
//! The rat is rasterised in its own local frame (`u` along the facing, `v`
//! across it), so it holds its shape at any of 360 degrees instead of being a
//! sprite that only looks right on the axes. Each pixel in a small box around
//! the player is supersampled against a spine + half-width profile (a pear:
//! wide haunches, narrow shoulders, wedge head) and blended over whatever the
//! floor already drew, with a dark rim and a down-right drop shadow so the
//! pale fur reads on every theme.
//!
//! Nothing here is stored between frames: the gait is keyed to the distance
//! travelled along the facing, and the tail's lag comes from `vel` trailing
//! `face` during a turn (the sim eases velocity toward the input).

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::player::Player;

/// Collision radius of the rat in world pixels (the sim's `PLAYER_RADIUS`).
/// The sprite may extend past it (tail, ears, whiskers) but the body should
/// read as roughly this size so hits look fair.
pub const BODY_RADIUS: f32 = 3.0;

// --- Palette ---------------------------------------------------------------
const FUR_SHADOW: Rgb = (112, 104, 138);
const FUR_BASE: Rgb = (212, 206, 218);
const FUR_LIGHT: Rgb = (255, 255, 248);
const EAR_PINK: Rgb = (255, 172, 190);
const EAR_RIM: Rgb = (204, 104, 134);
const NOSE_PINK: Rgb = (255, 120, 150);
const PAW_PINK: Rgb = (250, 206, 204);
const TAIL_BASE: Rgb = (214, 122, 146);
const TAIL_TIP: Rgb = (255, 188, 198);
const EYE: Rgb = (18, 12, 22);
const HURT: Rgb = (255, 56, 64);
const GHOST: Rgb = (130, 200, 255);

/// Direction *toward* the light in screen space (up-left), matching the
/// down-right drop shadow.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Half-width of the body at each station along the spine, nose at `+u`.
/// Haunches peak just behind the hit-circle centre; the widest point stays
/// under `BODY_RADIUS` so the silhouette is honest about what can be hit.
const PROFILE: [(f32, f32); 10] = [
    (-3.6, 0.0),
    (-3.1, 1.7),
    (-2.0, 2.7),
    (-0.7, 2.65),
    (0.6, 2.0),
    (1.5, 1.3),
    (2.6, 1.9),
    (3.6, 1.35),
    (4.7, 0.6),
    (5.4, 0.0),
];
const U_TAIL: f32 = -3.6;
const U_NOSE: f32 = 5.4;

/// Tail length in world pixels and the number of half-pixel steps along it.
const TAIL_LEN: f32 = 13.0;
const TAIL_STEPS: usize = 28;

/// Half-size of the box of pixels rasterised around the player.
const REACH: i32 = 10;

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

/// Body half-width at spine station `u` (0 outside the body).
fn half_width(u: f32) -> f32 {
    if u <= U_TAIL || u >= U_NOSE {
        return 0.0;
    }
    for i in 1..PROFILE.len() {
        let (u1, w1) = PROFILE[i];
        if u <= u1 {
            let (u0, w0) = PROFILE[i - 1];
            return w0 + (w1 - w0) * (u - u0) / (u1 - u0);
        }
    }
    0.0
}

/// Everything about this frame's pose, derived once.
#[derive(Clone, Copy)]
struct Pose {
    pos: Vec2,
    face: Vec2,
    perp: Vec2,
    /// Stretch along / squash across the facing (dash).
    su: f32,
    sv: f32,
    /// Gait phase (radians) and spine wiggle amplitude (px).
    phase: f32,
    wig: f32,
    /// Idle breathing, added to the haunch width (px).
    breathe: f32,
    /// 0 = standing, 1 = full run.
    moving: f32,
    /// 0..1 while dashing.
    dash: f32,
}

impl Pose {
    /// Lateral offset of the spine at station `u`: a wave that runs down the
    /// body while scurrying, fading out toward the head so the eyes stay put.
    #[inline]
    fn spine(&self, u: f32) -> f32 {
        let steady = ((3.2 - u) * 0.22).clamp(0.0, 1.0);
        self.wig * steady * (self.phase - u * 0.7).sin()
    }

    /// World offset → local `(u, v)` in unstretched body units.
    #[inline]
    fn local(&self, d: Vec2) -> (f32, f32) {
        (d.dot(self.face) / self.su, d.dot(self.perp) / self.sv)
    }

    /// Local `(u, v)` (v measured from the wiggling spine) → world position.
    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.pos + self.face * (u * self.su) + self.perp * ((v + self.spine(u)) * self.sv)
    }

    #[inline]
    fn ear_centre(&self, side: f32) -> (f32, f32) {
        // Ears fold back and in while dashing.
        (2.0 - 0.6 * self.dash, side * (1.85 - 0.3 * self.dash))
    }

    /// Is local point `(u, v)` inside the body, grown by `margin`?
    #[inline]
    fn in_body(&self, u: f32, v: f32, margin: f32) -> bool {
        if u < U_TAIL - margin || u > U_NOSE + margin {
            return false;
        }
        let uc = u.clamp(U_TAIL + 0.3, U_NOSE - 0.3);
        let mut w = half_width(uc);
        if uc < 0.5 {
            w += self.breathe;
        }
        let cap = margin - (u - uc).abs();
        (v - self.spine(uc)).abs() <= w + cap.min(margin)
    }

    #[inline]
    fn in_ear(&self, u: f32, v: f32, margin: f32) -> bool {
        let r = 1.15 + margin;
        let (eu, ev) = self.ear_centre(1.0);
        let du = u - eu;
        let dv = v.abs() - ev;
        du * du + dv * dv <= r * r
    }
}

/// Final tint for every rat pixel: red on hurt, cold and see-through on the
/// "off" beat of the invulnerability blink.
#[derive(Clone, Copy)]
struct Tint {
    hurt: f32,
    ghost: bool,
}

impl Tint {
    #[inline]
    fn apply(&self, c: Rgb) -> Rgb {
        let c = if self.hurt > 0.0 {
            mix(c, HURT, self.hurt)
        } else {
            c
        };
        if self.ghost {
            mix(c, GHOST, 0.45)
        } else {
            c
        }
    }

    /// Opacity of the rat over the floor.
    #[inline]
    fn alpha(&self) -> f32 {
        if self.ghost {
            0.5
        } else {
            1.0
        }
    }
}

/// Blend `c` over the pixel already at `(x, y)`.
#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, c, a));
}

/// Darken the pixel already at `(x, y)` by `amount` (0..1).
#[inline]
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale(bg, 1.0 - amount));
}

/// Draw the rat at `player.pos`, facing `player.face`. `elapsed` is the run
/// clock in seconds, for animation and the invulnerability blink.
pub fn draw_rat(pb: &mut PixelBuffer, player: &Player, elapsed: f32, cam: Vec2) {
    let face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let perp = face.perp();
    let speed = player.vel.len();
    let top = player.loadout.stats.move_speed.max(1.0);
    let moving = (speed / top).clamp(0.0, 1.0);
    let dash = (player.dash_active / 0.16).clamp(0.0, 1.0).sqrt();

    let pose = Pose {
        pos: player.pos,
        face,
        perp,
        su: 1.0 + 0.32 * dash,
        sv: 1.0 - 0.14 * dash,
        // Keyed to distance along the facing: feet step with the ground and
        // stop dead when the rat does.
        phase: player.pos.dot(face) * 1.15,
        wig: 0.45 * moving * (1.0 - dash),
        breathe: 0.18 * (1.0 - moving) * (elapsed * 3.2).sin(),
        moving,
        dash,
    };
    let tint = Tint {
        hurt: if player.hurt_flash > 0.0 {
            // Snap to near-solid red, then ease back as the flash runs out.
            (0.55 + player.hurt_flash * 3.0).min(0.85)
        } else {
            0.0
        },
        ghost: player.iframes > 0.0 && ((elapsed * 20.0) as i32 % 2 == 0),
    };

    if dash > 0.0 {
        draw_dash_streaks(pb, &pose, cam);
    }
    draw_tail(pb, &pose, player.vel, elapsed, tint, cam);
    draw_feet(pb, &pose, tint, cam);
    draw_body(pb, &pose, tint, cam);
    draw_face(pb, &pose, elapsed, tint, cam);
}

/// The **Tail Thrash** arc: the rat's tail stretched into a long whipping arc
/// that sweeps around it.
///
/// `visual` is `(angle, reach, brightness)` from `World::thrash_visual`: the
/// sweep angle in world radians, the arc's radius in px, and a 0..1 fade that
/// snaps the arc on with the trigger's screen shake and lingers out as the
/// burst winds down. The arc is drawn as a tapering whip: a bright leading edge
/// at `angle` with two dimmer ``ghost`` arcs trailing behind it (where the tip
/// just was), so a still frame reads as motion rather than a hoop.
pub fn draw_tail_thrash(
    pb: &mut PixelBuffer,
    player: &Player,
    visual: Option<(f32, f32, f32)>,
    cam: Vec2,
) {
    let Some((angle, reach, fade)) = visual else {
        return;
    };
    let fade = fade.clamp(0.0, 1.0);
    if fade <= 0.0 || reach <= 1.0 {
        return;
    }
    let origin = w2s(player.pos, cam);
    // Three passes: the live leading edge and two ghosts behind it (where the
    // tip just was), so one frame reads as motion, not as a hoop.
    for pass in 0..3 {
        let lead = -(pass as f32) * 0.17;
        let atten = if pass == 0 { 1.0 } else { 0.34 / pass as f32 };
        let steps = (reach * 1.3) as i32;
        for i in 0..steps.max(2) {
            let t = i as f32 / steps.max(2) as f32;
            // Radius from the rat's rump (4 px) out to the full reach.
            let r = 4.0 + (reach - 4.0) * t;
            // The arc lags behind the leading edge as it unwinds from the root,
            // which is what makes a still frame look like a whip in motion.
            let a = angle + lead - t * 0.42;
            let p = Vec2::from_angle(a);
            let nx = -p.y;
            let ny = p.x;
            let bx = origin.0 + (p.x * r).round() as i32;
            let by = origin.1 + (p.y * r).round() as i32;
            let weight = atten * fade * (0.32 + 0.68 * t);
            let hot = if pass == 0 {
                // Near-white at the tip, tail-rose at the root.
                (
                    (205.0 + 50.0 * t) as u8,
                    (95.0 + 145.0 * t) as u8,
                    (120.0 + 110.0 * t) as u8,
                )
            } else {
                TAIL_BASE
            };
            // Core plus a two-step band each side: the whip has mass, and a
            // 1-px line disappears against a busy floor. The 1.8x gain keeps
            // the additive blend above the floor's own dark tones (a raw
            // weight reads brown on the Sewer's stone).
            let glow = (weight * 1.8).min(1.4);
            plot_add(pb, bx, by, scale(hot, glow));
            for (k, gain) in [(1.0f32, 0.6f32), (2.0, 0.26)] {
                plot_add(
                    pb,
                    origin.0 + (p.x * r + nx * k).round() as i32,
                    origin.1 + (p.y * r + ny * k).round() as i32,
                    scale(hot, glow * gain),
                );
                plot_add(
                    pb,
                    origin.0 + (p.x * r - nx * k).round() as i32,
                    origin.1 + (p.y * r - ny * k).round() as i32,
                    scale(hot, glow * gain),
                );
            }
        }
    }
}

/// Additive plot with bounds check (the arc can sweep past the viewport).
#[inline]
fn plot_add(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb) {
    pb.plot_add(x, y, c);
}

/// Speed lines peeling off the haunches during a dash.
fn draw_dash_streaks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    for side in [-1.0f32, 1.0] {
        for j in 0..7 {
            let t = j as f32 / 7.0;
            let p = pose.place(-3.0 - j as f32 * 0.9, side * (2.2 - 0.9 * t));
            let s = w2s(p, cam);
            let f = pose.dash * (1.0 - t) * 0.9;
            pb.plot_add(s.0, s.1, scale((90, 120, 170), f));
        }
    }
}

/// The tail: a chain of half-pixel steps whose heading curls away from the
/// spine. Standing still it hooks lazily to one side and sways; running it
/// straightens and a wave whips down it; through a turn it swings toward
/// where the rat came from (the direction `vel` still points).
fn draw_tail(pb: &mut PixelBuffer, pose: &Pose, vel: Vec2, elapsed: f32, tint: Tint, cam: Vec2) {
    let m = pose.moving;
    // Signed angle from the facing to the velocity: non-zero only mid-turn.
    let lag = if vel.len_sq() > 4.0 {
        let vd = vel.normalized();
        let cross = pose.face.x * vd.y - pose.face.y * vd.x;
        (cross.atan2(pose.face.dot(vd)) * 1.5).clamp(-2.0, 2.0)
    } else {
        0.0
    };
    let whip = 1.0 - 0.85 * pose.dash;
    let step = (TAIL_LEN + 3.0 * pose.dash) / TAIL_STEPS as f32;
    let back = pose.face * -1.0;

    let mut pts = [(0i32, 0i32); TAIL_STEPS];
    let mut p = pose.place(-3.2, 0.0);
    for (i, slot) in pts.iter_mut().enumerate() {
        let s = (i as f32 + 0.5) / TAIL_STEPS as f32;
        let idle = 1.75 * s.powf(1.3) + 0.24 * (elapsed * 1.7 - s * 2.2).sin();
        let run = 0.22 * s + 0.6 * s * (elapsed * 13.0 - s * 5.5).sin() * whip;
        let theta = lag * s + idle * (1.0 - m) + run * m;
        p += back.rotate(theta) * step;
        *slot = w2s(p, cam);
    }

    // Shadow pass first so it never lands on the tail itself.
    if !tint.ghost {
        let mut last = (i32::MIN, i32::MIN);
        for &(x, y) in pts.iter() {
            if (x, y) != last {
                darken(pb, x + 1, y + 1, 0.3);
                last = (x, y);
            }
        }
    }
    let a = tint.alpha();
    for (i, &(x, y)) in pts.iter().enumerate() {
        let s = i as f32 / (TAIL_STEPS - 1) as f32;
        blend(pb, x, y, tint.apply(mix(TAIL_BASE, TAIL_TIP, s)), a);
    }
}

/// Four pink paws that poke out past the flanks in diagonal pairs.
fn draw_feet(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let reach = 0.25 + 0.75 * pose.moving;
    let a = tint.alpha();
    for (k, &(u0, side)) in [(1.0f32, 1.0f32), (-2.2, -1.0), (1.0, -1.0), (-2.2, 1.0)]
        .iter()
        .enumerate()
    {
        // Diagonal pairs move together (trot); a dash throws all four back.
        let ph = pose.phase + if k < 2 { 0.0 } else { std::f32::consts::PI };
        let swing = ph.sin() * 1.0 * pose.moving * (1.0 - pose.dash) - 0.8 * pose.dash;
        let u = u0 + swing;
        let v = side * (half_width(u0) + reach * (0.55 + 0.3 * ph.cos().max(0.0)));
        let s = w2s(pose.place(u, v), cam);
        if !tint.ghost {
            darken(pb, s.0 + 1, s.1 + 1, 0.3);
        }
        blend(pb, s.0, s.1, tint.apply(PAW_PINK), a);
    }
}

/// Fur colour at local `(u, v)`: a dome across the back lit from up-left.
fn fur_at(pose: &Pose, u: f32, v: f32) -> Rgb {
    let uc = u.clamp(U_TAIL + 0.3, U_NOSE - 0.3);
    let w = half_width(uc).max(0.7);
    let n = ((v - pose.spine(uc)) / w).clamp(-1.0, 1.0);
    let along = ((u - 0.6) / 4.4).clamp(-1.0, 1.0);
    let dome = 1.0 - n * n;
    let lit = n * pose.perp.dot(LIGHT) + 0.55 * along * along * along.signum() * pose.face.dot(LIGHT);
    let lum = (0.5 + 0.34 * dome + 0.6 * lit).clamp(0.0, 1.15);

    if lum < 0.75 {
        mix(FUR_SHADOW, FUR_BASE, lum / 0.75)
    } else {
        mix(FUR_BASE, FUR_LIGHT, (lum - 0.75) / 0.4)
    }
}

/// Body and ears: supersampled coverage blended over the floor, inside a dark
/// rim and a drop shadow.
fn draw_body(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let c = w2s(pose.pos, cam);
    let x0 = (c.0 - REACH).max(0);
    let x1 = (c.0 + REACH).min(pb.w as i32 - 1);
    let y0 = (c.1 - REACH).max(0);
    let y1 = (c.1 + REACH).min(pb.h as i32 - 1);
    let a_all = tint.alpha();
    let rim_colour: Rgb = if tint.hurt > 0.0 { (90, 0, 10) } else { (0, 0, 0) };

    for y in y0..=y1 {
        for x in x0..=x1 {
            // Pixel centre relative to the player, in world pixels.
            let d = Vec2::new(x as f32 + cam.x, y as f32 + cam.y) - pose.pos;
            if d.len_sq() > (REACH * REACH) as f32 {
                continue;
            }
            let (u, v) = pose.local(d);

            // 3x3 coverage.
            let mut body = 0u32;
            let mut ear = 0u32;
            for sy in -1..=1 {
                for sx in -1..=1 {
                    let o = Vec2::new(sx as f32 * 0.33, sy as f32 * 0.33);
                    let (su, sv) = pose.local(d + o);
                    if pose.in_ear(su, sv, 0.0) {
                        ear += 1;
                    } else if pose.in_body(su, sv, 0.0) {
                        body += 1;
                    }
                }
            }
            let total = body + ear;
            // Firm the edge up: this is pixel art, not a blur.
            let cover = ((total as f32 / 9.0 - 0.18) / 0.5).clamp(0.0, 1.0);

            // Rim (grown silhouette) and drop shadow (silhouette shifted
            // down-right, i.e. sampled up-left of this pixel).
            let mut dark = 0.0f32;
            if cover < 1.0 {
                if pose.in_body(u, v, 0.95) || pose.in_ear(u, v, 0.8) {
                    dark = 0.62;
                } else {
                    let (hu, hv) = pose.local(d - Vec2::new(1.3, 1.3));
                    if pose.in_body(hu, hv, 0.5) || pose.in_ear(hu, hv, 0.4) {
                        dark = 0.38;
                    }
                }
            }
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_colour, dark * a_all);
            if cover > 0.0 {
                let fur = fur_at(pose, u, v);
                let colour = if ear * 2 > total {
                    // Ear: pink bowl with a darker rim toward the outside.
                    let (eu, ev) = pose.ear_centre(1.0);
                    let r = ((u - eu).powi(2) + (v.abs() - ev).powi(2)).sqrt();
                    mix(EAR_PINK, EAR_RIM, ((r - 0.5) / 0.9).clamp(0.0, 1.0))
                } else {
                    fur
                };
                out = mix(out, tint.apply(colour), cover * a_all);
            }
            pb.plot(x, y, out);
        }
    }
}

/// Eyes, nose and whiskers: single pixels placed on top of the head.
fn draw_face(pb: &mut PixelBuffer, pose: &Pose, elapsed: f32, tint: Tint, cam: Vec2) {
    for side in [-1.0f32, 1.0] {
        let e = w2s(pose.place(3.4, side * 0.85), cam);
        pb.plot(e.0, e.1, EYE);
    }
    let n = w2s(pose.place(5.0, 0.0), cam);
    blend(pb, n.0, n.1, tint.apply(NOSE_PINK), tint.alpha());

    // Whiskers: faint additive flecks fanning from the snout, twitching.
    let twitch = 0.35 * (elapsed * 9.0).sin();
    for side in [-1.0f32, 1.0] {
        for (k, &(u, v)) in [(4.5f32, 1.9f32), (4.9, 2.9)].iter().enumerate() {
            let w = w2s(pose.place(u + twitch * (k as f32 + 1.0) * 0.5, side * v), cam);
            let g = if k == 0 { 70 } else { 40 };
            pb.plot_add(w.0, w.1, (g, g, g));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::{Level, Theme};
    use crate::game::weapon::WeaponKind;

    fn rat(pos: Vec2, ang: f32) -> Player {
        let mut p = Player::new(pos, WeaponKind::Gnaw);
        p.face = Vec2::from_angle(ang);
        p
    }

    /// Every pose at every angle, including off-buffer positions: must never
    /// panic or write outside the buffer.
    #[test]
    fn rat_draws_everywhere_without_panic() {
        let mut pb = PixelBuffer::new(40, 30);
        for i in 0..64 {
            let ang = i as f32 / 64.0 * std::f32::consts::TAU;
            for &(x, y) in &[(20.0, 15.0), (0.0, 0.0), (39.5, 29.5), (-30.0, 12.0), (500.0, 500.0)] {
                let mut p = rat(Vec2::new(x, y), ang);
                p.vel = Vec2::from_angle(ang + 0.7) * 58.0;
                p.dash_active = if i % 3 == 0 { 0.16 } else { 0.0 };
                p.hurt_flash = if i % 5 == 0 { 0.2 } else { 0.0 };
                p.iframes = if i % 2 == 0 { 0.5 } else { 0.0 };
                draw_rat(&mut pb, &p, i as f32 * 0.37, Vec2::ZERO);
            }
        }
    }

    /// The hurt flash and the blink must change what is drawn.
    #[test]
    fn hurt_and_blink_are_visible() {
        let render = |hurt: f32, iframes: f32| {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear((20, 30, 28));
            let mut p = rat(Vec2::new(16.0, 16.0), 0.3);
            p.hurt_flash = hurt;
            p.iframes = iframes;
            draw_rat(&mut pb, &p, 0.0, Vec2::ZERO);
            pb.pixel_at(16, 16)
        };
        let normal = render(0.0, 0.0);
        let hurt = render(0.2, 0.0);
        let blink = render(0.0, 0.5);
        assert!(hurt.0 > hurt.1 + 60, "hurt should be red: {hurt:?}");
        assert_ne!(normal, blink);
    }

    /// Pose sheet: `RATRUN_DUMP=rat cargo test dump_rat_sheet` writes
    /// `/tmp/ratrun_rat.ppm` — each theme's floor with rows of idle / run /
    /// turn / dash / hurt / blink across 16 headings.
    #[test]
    fn dump_rat_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("rat") {
            return;
        }
        const CELL: usize = 26;
        const COLS: usize = 16;
        const ROWS: usize = 6;
        let (w, h) = (CELL * COLS, CELL * ROWS);
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();
        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let level = Level::new(theme);
            let cam = Vec2::new(300.0, 300.0);
            let mut pb = PixelBuffer::new(w, h);
            crate::game::art_floor::draw_floor(&mut pb, &level, cam, 0.0);
            for row in 0..ROWS {
                for col in 0..COLS {
                    let ang = col as f32 / COLS as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let pos = cam
                        + Vec2::new(
                            (col * CELL + CELL / 2) as f32 + 0.3 * col as f32,
                            (row * CELL + CELL / 2) as f32 + 0.2 * col as f32,
                        );
                    let mut p = rat(pos, ang);
                    let run = p.face * 58.0;
                    let t = 0.4 + col as f32 * 0.11;
                    match row {
                        0 => {}
                        1 => p.vel = run,
                        2 => p.vel = Vec2::from_angle(ang - 0.9) * 58.0,
                        3 => {
                            p.vel = run * 3.2;
                            p.dash_active = 0.12;
                        }
                        4 => {
                            p.vel = run;
                            p.hurt_flash = 0.02 + 0.012 * col as f32;
                        }
                        _ => {
                            p.vel = run;
                            p.iframes = 0.5;
                        }
                    }
                    // Blink row: alternate the on/off beat per column.
                    let t = if row == 5 { col as f32 * 0.05 } else { t };
                    draw_rat(&mut pb, &p, t, cam);
                }
            }
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_rat.ppm", out).unwrap();
    }
}
