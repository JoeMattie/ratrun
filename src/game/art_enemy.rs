//! Art for the enemies (the vermin horde, alley cats, venomous spitters,
//! armored brutes, gilded elites, and the boss). Pure drawing from an enemy's
//! state.
//!
//! Every enemy is rasterised in its own local frame (`u` along `e.facing`, `v`
//! across it) with consistent top-left directional lighting, a dark silhouette
//! rim, and a down-right drop shadow so it reads cleanly against all three dark
//! map floors while staying unmistakably distinct from the white lab rat,
//! glowing pickups, and bullets.
//!
//! Performance: up to 600 enemies can be active at once. Small swarmers use
//! tight clamped bounding boxes and single-sample analytic SDF shading (zero
//! heap allocations, no supersampling), scaling detail up with `e.radius`.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::enemy::{Enemy, EnemyKind};

/// Direction *toward* the light in screen space (up-left), matching
/// `art_rat.rs` and `art_props.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Hit-flash peak duration in `Enemy::hurt`.
const FLASH_MAX: f32 = 0.12;

/// Draw one enemy at `e.pos`, facing `e.facing`. `e.anim` is its own animation
/// phase, `e.flash > 0` means it was just hit, and `accent` is the current
/// map's accent colour.
pub fn draw_enemy(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    draw_enemy_styled(pb, e, cam, accent, e.elite);
}

/// Draw one enemy with an explicit `elite` flag (used for gilded elite brutes).
pub fn draw_enemy_styled(
    pb: &mut PixelBuffer,
    e: &Enemy,
    cam: Vec2,
    accent: Rgb,
    elite: bool,
) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let f = if e.facing.len_sq() > 1e-4 {
        e.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let p = f.perp();
    let fr = Frame {
        pos: e.pos - cam,
        f,
        p,
        r: e.radius.max(1.0),
        anim: e.anim,
        hit: (e.flash / FLASH_MAX).clamp(0.0, 1.0),
        hp_frac: if e.max_hp > 0.0 {
            (e.hp / e.max_hp).clamp(0.0, 1.0)
        } else {
            1.0
        },
        fl: f.dot(LIGHT),
        pl: p.dot(LIGHT),
        accent,
    };

    match e.kind {
        EnemyKind::Skitterer => draw_skitterer(pb, &fr),
        EnemyKind::Bat => draw_bat(pb, &fr),
        EnemyKind::Cat => draw_cat(pb, &fr, e.vel, e.contact_cd),
        EnemyKind::Spitter => draw_spitter(pb, &fr, e.fire_cd),
        EnemyKind::Brute => draw_brute(pb, &fr, elite),
        EnemyKind::Boss => draw_boss(pb, &fr, e.fire_cd),
    }
}

/// Precomputed per-enemy screen-space frame.
#[derive(Clone, Copy)]
struct Frame {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    r: f32,
    anim: f32,
    hit: f32,
    hp_frac: f32,
    fl: f32,
    pl: f32,
    accent: Rgb,
}

impl Frame {
    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.pos + self.f * u + self.p * v
    }

    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        let q = self.place(u, v);
        (q.x.round() as i32, q.y.round() as i32)
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    #[inline]
    fn bbox_pb(&self, pb: &PixelBuffer, reach: f32) -> Option<(i32, i32, i32, i32)> {
        let rr = reach.ceil() as i32;
        let cx = self.pos.x.round() as i32;
        let cy = self.pos.y.round() as i32;
        let x0 = (cx - rr).max(0);
        let x1 = (cx + rr).min(pb.w as i32 - 1);
        let y0 = (cy - rr).max(0);
        let y1 = (cy + rr).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            None
        } else {
            Some((x0, x1, y0, y1))
        }
    }
}

// ---- Pixel & colour helpers -------------------------------------------------

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

#[inline]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.65 {
        mix(shadow, mid, l / 0.65)
    } else {
        mix(mid, hi, (l - 0.65) / 0.5)
    }
}

/// Sculptural warm-white hit flash: snaps bright on impact while preserving
/// slight form contrast so struck enemies stay distinct from the cool-white rat.
#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.0 {
        return c;
    }
    let flash_col = if hit > 0.6 {
        (255, 244, 218)
    } else {
        (255, 218, 152)
    };
    let strength = (0.30 + hit * 0.56).min(0.86);
    mix(c, flash_col, strength)
}

fn line_blend(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    loop {
        blend_px(pb, x, y, c, alpha);
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn thick_line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), core: Rgb, edge: Rgb) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    let horiz = dx >= -dy;
    loop {
        darken_px(pb, x + 1, y + 1, 0.35);
        if horiz {
            blend_px(pb, x, y + 1, edge, 0.85);
        } else {
            blend_px(pb, x + 1, y, edge, 0.85);
        }
        pb.plot(x, y, core);
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[inline]
fn sd_ellipse(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let nu = (u - cu) / ru;
    let nv = (v - cv) / rv;
    let d = (nu * nu + nv * nv).sqrt();
    (d - 1.0) * ru.min(rv)
}

#[inline]
fn sd_capsule(u: f32, v: f32, au: f32, av: f32, bu: f32, bv: f32, rad: f32) -> f32 {
    let pax = u - au;
    let pay = v - av;
    let bax = bu - au;
    let bay = bv - av;
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    (dx * dx + dy * dy).sqrt() - rad
}

/// Tapered capsule SDF (radius interpolates linearly from `r0` at `a` to `r1` at `b`).
#[inline]
fn sd_taper(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> f32 {
    let pax = u - a.0;
    let pay = v - a.1;
    let bax = b.0 - a.0;
    let bay = b.1 - a.1;
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    let rad = r0 + (r1 - r0) * h;
    (dx * dx + dy * dy).sqrt() - rad
}

#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

// ============================================================================
// 1. SKITTERER — Sleek Copper Sewer Roach / Scuttler (r = 2.5)
// ============================================================================

fn draw_skitterer(pb: &mut PixelBuffer, fr: &Frame) {
    let s = fr.r / 2.5;
    let leg_col = apply_hit((144, 86, 46), fr.hit);
    let foot_col = apply_hit((98, 56, 30), fr.hit);
    let ant_col = apply_hit((184, 122, 68), fr.hit);

    // 6 crisp alternating tripod-gait legs with visible copper knees.
    for (i, &u_hip) in [1.1f32, -0.35, -1.75].iter().enumerate() {
        let phase = fr.anim + i as f32 * 2.1;
        for (side, side_ph) in [(-1.0f32, 0.0f32), (1.0, PI)] {
            let swing = (phase + side_ph).sin() * 0.85 * s;
            let hip = fr.px(u_hip * s, side * 1.15 * s);
            let knee = fr.px(u_hip * s + swing * 0.45, side * 2.45 * s);
            let foot = fr.px(u_hip * s + swing - 0.3 * s, side * 3.25 * s);
            darken_px(pb, foot.0 + 1, foot.1 + 1, 0.25);
            line_blend(pb, hip, knee, leg_col, 0.92);
            line_blend(pb, knee, foot, foot_col, 0.85);
        }
    }

    // Sweeping twitching roach antennae that telegraph facing clearly.
    let wave = (fr.anim * 0.7).sin() * 0.42 * s;
    for side in [-1.0f32, 1.0] {
        let a0 = fr.px(2.25 * s, side * 0.55 * s);
        let a1 = fr.px(3.55 * s, side * 1.15 * s + wave * side * 0.5);
        let a2 = fr.px(4.35 * s, side * 1.95 * s + wave * side);
        line_blend(pb, a0, a1, ant_col, 0.82);
        line_blend(pb, a1, a2, scale(ant_col, 0.72), 0.62);
    }

    // Elongated copper-mahogany carapace: tapered abdomen + pinched head.
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 4.3 * s) else {
        return;
    };
    let shadow_c = (72, 36, 18);
    let mid_c = (196, 118, 52);
    let hi_c = (252, 206, 126);
    let seam_c = (48, 22, 12);
    let rim_col = if fr.hit > 0.4 { (160, 110, 60) } else { (0, 0, 0) };

    let body_sd = |u: f32, v: f32| -> f32 {
        let abd = sd_ellipse(u, v, -0.75 * s, 0.0, 2.25 * s, 1.75 * s);
        let thorax = sd_ellipse(u, v, 0.95 * s, 0.0, 1.05 * s, 1.35 * s);
        let head = sd_ellipse(u, v, 2.15 * s, 0.0, 0.82 * s, 1.05 * s);
        smin(smin(abd, thorax, 0.35 * s), head, 0.25 * s)
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let d = body_sd(u, v);
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            let mut dark = 0.0f32;
            if cover < 1.0 {
                if d < 0.85 * s {
                    dark = 0.6;
                } else {
                    let (su, sv) = fr.local(x - 1, y - 1);
                    if body_sd(su, sv) < 0.35 * s {
                        dark = 0.34;
                    }
                }
            }
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_col, dark);
            if cover > 0.0 {
                let nu = ((u + 0.1 * s) / (2.9 * s)).clamp(-1.0, 1.0);
                let nv = (v / (1.8 * s)).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.85)).clamp(0.0, 1.0);
                let lit = fr.lit(nu, nv);
                let mut lum = 0.44 + 0.38 * dome + 0.56 * lit;
                if lit > 0.4 && dome > 0.42 {
                    lum += 0.26;
                }
                let mut col = shade3(shadow_c, mid_c, hi_c, lum);
                // Darker pronotum collar + elytra wing-case split.
                if u < 0.7 * s && v.abs() < 0.28 * s {
                    col = mix(col, seam_c, 0.48);
                } else if (u - 1.35 * s).abs() < 0.3 * s {
                    col = mix(col, seam_c, 0.58);
                }
                out = mix(out, apply_hit(col, fr.hit), cover);
            }
            pb.plot(x, y, out);
        }
    }

    // Tiny amber-red eyes & mandible glint at the snout.
    if fr.r >= 2.2 {
        let eye_col = if fr.hit > 0.65 {
            (40, 20, 16)
        } else {
            (255, 138, 62)
        };
        for side in [-1.0f32, 1.0] {
            let ep = fr.px(2.35 * s, side * 0.75 * s);
            blend_px(pb, ep.0, ep.1, eye_col, 0.9);
        }
    }
}

// ============================================================================
// 2. BAT — Cave/Sewer Bat with Classic M-Arched Scalloped Wings (r = 2.0)
// ============================================================================

fn draw_bat(pb: &mut PixelBuffer, fr: &Frame) {
    let s = fr.r / 2.0;
    let flap = fr.anim.sin(); // -1..1
    let spread = 0.5 + 0.5 * flap; // 0..1
    // Wide dramatic bat wingspan throughout the whole flap cycle.
    let span = (4.1 + 1.85 * spread) * s;
    let sweep = (-0.75 + 1.35 * flap) * s;

    // 1) Elevated ground shadow shifted down-right so the bat visibly hovers.
    let sh_ofs_x = (2.2 * s).round() as i32;
    let sh_ofs_y = (3.4 * s + 0.45 * flap * s).round() as i32;
    let sh_rx = (2.4 + 1.0 * spread) * s;
    let sh_ry = 1.45 * s;
    let scx = fr.pos.x.round() as i32 + sh_ofs_x;
    let scy = fr.pos.y.round() as i32 + sh_ofs_y;
    let srx = sh_rx.ceil() as i32;
    let sry = sh_ry.ceil() as i32;
    for dy in -sry..=sry {
        let y = scy + dy;
        if y < 0 || y >= pb.h as i32 {
            continue;
        }
        for dx in -srx..=srx {
            let x = scx + dx;
            if x < 0 || x >= pb.w as i32 {
                continue;
            }
            let nx = dx as f32 / sh_rx;
            let ny = dy as f32 / sh_ry;
            let r2 = nx * nx + ny * ny;
            if r2 <= 1.0 {
                darken_px(pb, x, y, 0.34 * (1.0 - r2 * 0.5));
            }
        }
    }

    // 2) Classic M-arched scalloped bat wings + furry violet torso + pointed ears.
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, (span + 2.0 * s).max(6.4 * s)) else {
        return;
    };

    let body_sh = (54, 32, 92);
    let body_mid = (144, 96, 206);
    let body_hi = (218, 178, 255);
    let wing_sh = (46, 24, 84);
    let wing_mid = (114, 70, 178);
    let wing_bone = (204, 166, 252);
    let rim_col = if fr.hit > 0.4 { (140, 100, 180) } else { (0, 0, 0) };

    // Classic M-shape bat wing: dips slightly at shoulder behind the ears,
    // arches forward to a crisp wing elbow at t≈0.48, sweeps back to sharp tips,
    // with two concave scalloped arches cut out of the trailing membrane.
    let wing_sd = |u: f32, av: f32| -> (f32, bool) {
        if av < 0.9 * s || av > span + 1.2 * s {
            return (10.0, false);
        }
        let t = ((av - 1.05 * s) / (span - 1.05 * s).max(0.5)).clamp(0.0, 1.0);
        // Distinct dip near shoulder (t=0 -> u=-0.25s), rising to elbow at t=0.48 (+1.6s),
        // then sweeping back to pointed wingtip at t=1.0 (-0.55s).
        let elbow = if t < 0.48 {
            let k = t / 0.48;
            -0.25 + 1.85 * k * k * (3.0 - 2.0 * k)
        } else {
            let k = (t - 0.48) / 0.52;
            1.6 - 2.15 * k.powf(1.15)
        };
        let u_lead = elbow * s + sweep * t;
        // Double concave bat-wing scallops along trailing edge.
        let scallop = (t * PI * 2.0).sin().abs();
        let chord = (2.45 * (1.0 - t * 0.84) - 0.85 * scallop * (1.0 - t * 0.25)) * s;
        let u_trail = u_lead - chord.max(0.28 * s);
        let du_top = u - u_lead;
        let du_bot = u_trail - u;
        let dv_tip = av - span;
        let d = du_top.max(du_bot).max(dv_tip);
        let on_bone = du_top.abs() < 0.58 * s && d < 0.35 * s;
        (d, on_bone)
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let av = v.abs();

            let torso = sd_ellipse(u, v, 0.2 * s, 0.0, 2.0 * s, 1.3 * s);
            // Distinct pointed bat ears rising forward past the shoulder notch.
            let ear = sd_taper(u, av, (1.45 * s, 0.65 * s), (2.9 * s, 1.35 * s), 0.5 * s, 0.16 * s);
            let d_body = torso.min(ear);
            let (d_wing, on_bone) = wing_sd(u, av);
            let d_all = d_body.min(d_wing);

            let body_cov = (0.5 - d_body * 1.2).clamp(0.0, 1.0);
            let wing_cov = (0.48 - d_wing * 1.15).clamp(0.0, 1.0);
            let cover = body_cov.max(wing_cov);

            if cover <= 0.0 {
                if d_all < 0.85 * s {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    pb.plot(x, y, mix(bg, rim_col, 0.54));
                }
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = if cover < 1.0 {
                mix(bg, rim_col, 0.54 * (1.0 - cover))
            } else {
                bg
            };

            if body_cov >= wing_cov {
                let nu = (u / (2.0 * s)).clamp(-1.0, 1.0);
                let nv = (v / (1.35 * s)).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.5 + nv * nv * 0.7)).clamp(0.0, 1.0);
                let lum = 0.44 + 0.38 * dome + 0.56 * fr.lit(nu, nv);
                let col = apply_hit(shade3(body_sh, body_mid, body_hi, lum), fr.hit);
                out = mix(out, col, body_cov);
            } else {
                let t = ((av - 1.05 * s) / (span - 1.05 * s).max(0.5)).clamp(0.0, 1.0);
                let lit = fr.lit(0.25, v.signum() * 0.65);
                let mut col = if on_bone {
                    mix(wing_mid, wing_bone, (0.82 + 0.25 * lit).clamp(0.0, 1.0))
                } else {
                    let rib = ((t * 2.5).fract() - 0.5).abs() < 0.1;
                    let base = shade3(wing_sh, wing_mid, wing_bone, 0.46 + 0.36 * lit);
                    if rib {
                        scale(base, 0.76)
                    } else {
                        base
                    }
                };
                col = apply_hit(col, fr.hit);
                out = mix(out, col, wing_cov * 0.92);
            }
            pb.plot(x, y, out);
        }
    }

    // Piercing golden-amber bat eyes cleanly set on the forward head.
    let eye_col = if fr.hit > 0.7 {
        (30, 18, 45)
    } else {
        (255, 222, 74)
    };
    for side in [-1.0f32, 1.0] {
        let ep = fr.px(1.65 * s, side * 0.65 * s);
        pb.plot(ep.0, ep.1, eye_col);
    }
}

// ============================================================================
// 3. CAT — Prowling Tuxedo-Tabby Alley Cat (r = 4.0)
// ============================================================================

fn draw_cat(pb: &mut PixelBuffer, fr: &Frame, vel: Vec2, contact_cd: f32) {
    let s = fr.r / 4.0;

    let spine = |u: f32| -> f32 {
        let flex = ((2.0 * s - u) / (6.4 * s)).clamp(0.0, 1.0);
        0.52 * s * flex * (fr.anim - u * 0.48 / s).sin()
    };

    // 1) Bold curved feline S-hook tail (always curls gracefully, never straightens!).
    let lag = if vel.len_sq() > 4.0 {
        let vd = vel.normalized();
        let cross = fr.f.x * vd.y - fr.f.y * vd.x;
        (cross.atan2(fr.f.dot(vd)) * 1.1).clamp(-1.4, 1.4)
    } else {
        0.0
    };
    const TAIL_STEPS: usize = 16;
    let step = 7.4 * s / TAIL_STEPS as f32;
    let back = fr.f * -1.0;
    let mut tail_pts = [(0i32, 0i32); TAIL_STEPS];
    let mut tp = fr.place(-4.0 * s, spine(-4.0 * s));
    for (i, slot) in tail_pts.iter_mut().enumerate() {
        let t = (i as f32 + 0.5) / TAIL_STEPS as f32;
        // Strong feline hook (~105 deg curl) with lively tail-tip swish.
        let curl = 1.75 * t.powf(1.25);
        let wave = 0.38 * t * (fr.anim * 0.9 - t * 3.2).sin();
        let theta = lag * t + curl + wave;
        tp += back.rotate(theta) * step;
        *slot = (tp.x.round() as i32, tp.y.round() as i32);
    }
    let mut last_sh = (i32::MIN, i32::MIN);
    for &(tx, ty) in &tail_pts {
        if (tx, ty) != last_sh {
            darken_px(pb, tx + 1, ty + 1, 0.34);
            last_sh = (tx, ty);
        }
    }
    for (i, &(tx, ty)) in tail_pts.iter().enumerate() {
        let stripe = (i / 2) % 2 == 1;
        let mut c = if i >= TAIL_STEPS - 2 {
            (178, 190, 212) // pale silver tip
        } else if stripe {
            (34, 40, 54)
        } else {
            (106, 118, 144)
        };
        c = apply_hit(c, fr.hit);
        if i < TAIL_STEPS - 4 {
            let side = fr.p;
            blend_px(
                pb,
                tx + side.x.round() as i32,
                ty + side.y.round() as i32,
                scale(c, 0.6),
                0.72,
            );
        }
        pb.plot(tx, ty, c);
    }

    // 2) Four stalking feline paws (diagonal stride + claw swipe on contact).
    let swiping = contact_cd > 0.32;
    let paw_col = apply_hit((76, 86, 108), fr.hit);
    let toe_col = apply_hit((168, 180, 202), fr.hit);
    for (k, &(u0, side)) in [
        (1.6f32, 1.0f32),
        (-2.5, -1.0),
        (1.6, -1.0),
        (-2.5, 1.0),
    ]
    .iter()
    .enumerate()
    {
        let ph = fr.anim + if k < 2 { 0.0 } else { PI };
        let swing = ph.sin() * 1.3 * s;
        let reach_u = u0 * s + swing + if swiping && u0 > 0.0 { 1.2 * s } else { 0.0 };
        let reach_v = side * (2.6 + 0.55 * ph.cos().max(0.0)) * s + spine(u0 * s);
        let p0 = fr.px(u0 * s, side * 1.6 * s + spine(u0 * s));
        let p1 = fr.px(reach_u, reach_v);
        darken_px(pb, p1.0 + 1, p1.1 + 1, 0.32);
        line_blend(pb, p0, p1, paw_col, 0.92);
        blend_px(pb, p1.0, p1.1, toe_col, 0.88);
        if swiping && u0 > 0.0 {
            let claw = fr.px(reach_u + 1.1 * s, reach_v + side * 0.3 * s);
            pb.plot(claw.0, claw.1, (240, 245, 255));
        }
    }

    // 3) Sculpted feline torso + slim neck + prominent pointed triangular cat ears.
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 7.4 * s) else {
        return;
    };

    let fur_sh = (38, 44, 62);
    let fur_mid = (104, 116, 142);
    let fur_hi = (182, 196, 222);
    let stripe_c = (26, 30, 44);
    let ear_in = (176, 112, 134); // clear dusty-rose inner ear triangle
    let rim_col = if fr.hit > 0.4 { (145, 130, 120) } else { (0, 0, 0) };

    let body_sd = |u: f32, v: f32| -> (f32, bool) {
        let vs = v - spine(u);
        let haunch = sd_ellipse(u, vs, -2.4 * s, 0.0, 2.2 * s, 2.5 * s);
        let chest = sd_ellipse(u, vs, 0.7 * s, 0.0, 1.95 * s, 2.2 * s);
        let torso = smin(haunch, chest, 0.6 * s);
        let neck = sd_capsule(u, vs, 1.25 * s, 0.0, 2.45 * s, 0.0, 1.15 * s);
        let head = sd_ellipse(u, vs, 3.25 * s, 0.0, 1.5 * s, 1.95 * s);
        let body = smin(torso.min(neck), head, 0.28 * s);
        // Sharp forward-and-outward pointed cat ears clearly standing proud of the skull!
        let ear = sd_taper(
            u,
            vs.abs(),
            (2.3 * s, 1.3 * s),
            (3.75 * s, 3.3 * s),
            0.96 * s,
            0.2 * s,
        );
        if ear < body {
            (ear, true)
        } else {
            (body, false)
        }
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let (d, is_ear) = body_sd(u, v);
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            let mut dark = 0.0f32;
            if cover < 1.0 {
                if d < 0.92 * s {
                    dark = 0.64;
                } else {
                    let (su, sv) = fr.local(x - 1, y - 2);
                    if body_sd(su, sv).0 < 0.45 * s {
                        dark = 0.36;
                    }
                }
            }
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_col, dark);
            if cover > 0.0 {
                let vs = v - spine(u);
                let col = if is_ear {
                    // Inner dusty-rose ear triangle vs dark charcoal outer ear edge.
                    let inner = u > 2.8 * s && vs.abs() > 1.7 * s && vs.abs() < 2.95 * s;
                    if inner {
                        ear_in
                    } else {
                        mix(fur_sh, fur_mid, 0.42)
                    }
                } else {
                    let nu = ((u - 0.3 * s) / (4.4 * s)).clamp(-1.0, 1.0);
                    let nv = (vs / (2.4 * s)).clamp(-1.0, 1.0);
                    let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.85)).clamp(0.0, 1.0);
                    let lum = 0.42 + 0.35 * dome + 0.58 * fr.lit(nu, nv);
                    let mut fur = shade3(fur_sh, fur_mid, fur_hi, lum);

                    // Bold tiger-tabby dorsal stripes across back & haunches.
                    if u < 1.8 * s {
                        let band = ((u / s) * 2.25).sin();
                        let dorsal = (1.0 - (vs.abs() / (2.5 * s))).clamp(0.0, 1.0);
                        if band > 0.22 && dorsal > 0.18 {
                            fur = mix(fur, stripe_c, 0.74 * dorsal);
                        }
                        if vs.abs() < 0.38 * s {
                            fur = mix(fur, stripe_c, 0.42);
                        }
                    } else if u > 2.2 * s && u < 3.45 * s && vs.abs() < 0.75 * s {
                        fur = mix(fur, stripe_c, 0.55);
                    }
                    fur
                };
                out = mix(out, apply_hit(col, fr.hit), cover);
            }
            pb.plot(x, y, out);
        }
    }

    // 4) Fierce glowing yellow-green cat eyes, dark nose, and crisp cheek whiskers.
    let sp_head = spine(3.7 * s);
    let eye_col = if fr.hit > 0.7 {
        (24, 28, 22)
    } else {
        (232, 255, 68)
    };
    for side in [-1.0f32, 1.0] {
        let ep = fr.px(3.75 * s, sp_head + side * 0.9 * s);
        pb.plot(ep.0, ep.1, eye_col);
        let w1 = fr.px(4.35 * s, sp_head + side * 2.0 * s);
        let w2 = fr.px(4.55 * s, sp_head + side * 2.95 * s);
        pb.plot_add(w1.0, w1.1, (64, 72, 86));
        pb.plot_add(w2.0, w2.1, (38, 44, 54));
    }
    let nose = fr.px(4.65 * s, sp_head);
    blend_px(pb, nose.0, nose.1, apply_hit((176, 112, 130), fr.hit), 0.92);
}

// ============================================================================
// 4. SPITTER — Venom Wasp / Acid-Sac Bug with Live Shooting Telegraph (r = 3.0)
// ============================================================================

fn draw_spitter(pb: &mut PixelBuffer, fr: &Frame, fire_cd: f32) {
    let s = fr.r / 3.0;

    let charge = if fire_cd <= 0.0 {
        0.82 + 0.18 * (fr.anim * 2.0).sin().abs()
    } else if fire_cd < 0.75 {
        ((0.75 - fire_cd) / 0.75).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let recoil = if fire_cd > 1.95 {
        ((fire_cd - 1.95) / 0.25).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let swell = 1.0 + 0.28 * charge - 0.14 * recoil;

    // 1) Slender dark-emerald legs.
    let leg_col = apply_hit((52, 102, 42), fr.hit);
    for (i, &u_hip) in [0.95f32, -0.35, -1.6].iter().enumerate() {
        let phase = fr.anim + i as f32 * 2.1;
        for (side, sph) in [(-1.0f32, 0.0f32), (1.0, PI)] {
            let swing = (phase + sph).sin() * 0.65 * s;
            let hip = fr.px(u_hip * s, side * 1.15 * s);
            let foot = fr.px(u_hip * s + swing, side * 3.15 * s);
            line_blend(pb, hip, foot, leg_col, 0.85);
        }
    }

    // 2) Distinct hourglass wasp body: plump venom sac + pinched waist + thorax/head + curved mandibles.
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 5.8 * s) else {
        return;
    };

    let ch_sh = (30, 68, 26);
    let ch_mid = (114, 214, 68);
    let ch_hi = (210, 255, 118);
    let stripe_c = (20, 44, 18);
    let venom_hot = mix(fr.accent, (242, 255, 138), 0.55);
    let rim_col = if fr.hit > 0.4 { (130, 160, 80) } else { (0, 0, 0) };

    let mand_tip_v = (0.45 + 1.15 * charge) * s;
    let body_sd = |u: f32, v: f32| -> (f32, u8) {
        let abd = sd_ellipse(u, v, -1.9 * s, 0.0, 1.9 * s * swell, 2.0 * s * swell);
        let thorax = sd_ellipse(u, v, 0.85 * s, 0.0, 1.2 * s, 1.3 * s);
        let head = sd_ellipse(u, v, 2.4 * s, 0.0, 0.95 * s, 1.25 * s);
        let front = smin(thorax, head, 0.32 * s);
        let waist = sd_capsule(u, v, -0.55 * s, 0.0, 0.35 * s, 0.0, 0.52 * s);
        // Sculpted curved pincer mandibles (hooked inward when calm, wide when charging!).
        let mand = sd_taper(
            u,
            v.abs(),
            (2.85 * s, 0.95 * s),
            ((4.2 + 0.3 * charge) * s, mand_tip_v),
            0.48 * s,
            0.2 * s,
        );
        let body = abd.min(front).min(waist).min(mand);
        let part = if mand < abd && mand < front && mand < waist {
            2 // mandible
        } else if abd < front && abd < waist {
            0 // venom sac abdomen
        } else {
            1 // thorax/head
        };
        (body, part)
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let (d, part) = body_sd(u, v);
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            let mut dark = 0.0f32;
            if cover < 1.0 {
                if d < 0.85 * s {
                    dark = 0.62;
                } else {
                    let (su, sv) = fr.local(x - 1, y - 1);
                    if body_sd(su, sv).0 < 0.35 * s {
                        dark = 0.34;
                    }
                }
            }
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_col, dark);
            if cover > 0.0 {
                let col = if part == 2 {
                    mix((60, 118, 44), (195, 248, 108), 0.55 + 0.35 * charge)
                } else {
                    let (cu, ru, rv) = if part == 0 {
                        (-1.9 * s, 1.9 * s * swell, 2.0 * s * swell)
                    } else {
                        (1.4 * s, 1.85 * s, 1.3 * s)
                    };
                    let nu = ((u - cu) / ru).clamp(-1.0, 1.0);
                    let nv = (v / rv).clamp(-1.0, 1.0);
                    let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.72)).clamp(0.0, 1.0);
                    let lum = 0.44 + 0.38 * dome + 0.52 * fr.lit(nu, nv);
                    let mut c = shade3(ch_sh, ch_mid, ch_hi, lum);
                    if part == 0 {
                        let stripe = ((u / s + 1.9) * 3.2).sin() > 0.42;
                        if stripe {
                            c = mix(c, stripe_c, 0.76 * (1.0 - 0.5 * charge));
                        } else if charge > 0.05 {
                            c = mix(c, venom_hot, charge * (0.45 + 0.48 * dome));
                        }
                    }
                    c
                };
                out = mix(out, apply_hit(col, fr.hit), cover);
            }
            pb.plot(x, y, out);
        }
    }

    // 3) Tapered gauzy buzzing wasp V-wings blended softly over thorax.
    let buzz = (fr.anim * 2.6).sin();
    let wing_reach = (3.4 + 1.0 * buzz.abs()) * s;
    let wing_col = mix((185, 240, 200), fr.accent, 0.45);
    if let Some((wx0, wx1, wy0, wy1)) = fr.bbox_pb(pb, 5.2 * s) {
        for y in wy0..=wy1 {
            for x in wx0..=wx1 {
                let (u, v) = fr.local(x, y);
                let av = v.abs();
                if av < 0.9 * s || av > wing_reach {
                    continue;
                }
                let t = (av - 0.9 * s) / (wing_reach - 0.9 * s).max(0.5);
                // Smooth tapered elliptical teardrop swept back from thorax shoulder.
                let u_center = (0.7 - 3.0 * t + 0.45 * buzz * t) * s;
                let half_ch = (0.95 * (t * (1.0 - t) * 4.0).clamp(0.0, 1.0).sqrt()) * s;
                let du = (u - u_center).abs();
                if du <= half_ch {
                    let edge = du > half_ch - 0.45 * s;
                    let alpha = if edge { 0.46 } else { 0.26 } * (1.0 - t * 0.35);
                    blend_px(pb, x, y, wing_col, alpha);
                }
            }
        }
    }

    // 4) Warning orange-red compound eyes.
    let eye_col = if fr.hit > 0.7 {
        (30, 30, 20)
    } else {
        (255, 108, 44)
    };
    for side in [-1.0f32, 1.0] {
        let ep = fr.px(2.4 * s, side * 1.05 * s);
        pb.plot(ep.0, ep.1, eye_col);
    }

    // 5) Live shot telegraph orb + 4-point charge sparkle between open mandibles!
    if charge > 0.08 {
        let mp = fr.px(4.0 * s, 0.0);
        let orb_col = mix(venom_hot, (255, 255, 225), charge * 0.65);
        pb.plot(mp.0, mp.1, orb_col);
        let glow = scale(orb_col, 0.45 + 0.55 * charge);
        if charge > 0.35 {
            pb.plot_add(mp.0 + 1, mp.1, glow);
            pb.plot_add(mp.0 - 1, mp.1, glow);
            pb.plot_add(mp.0, mp.1 + 1, glow);
            pb.plot_add(mp.0, mp.1 - 1, glow);
        }
        if charge > 0.7 {
            let tip = scale(orb_col, 0.6);
            pb.plot_add(mp.0 + 2, mp.1, tip);
            pb.plot_add(mp.0 - 2, mp.1, tip);
            pb.plot_add(mp.0, mp.1 + 2, tip);
            pb.plot_add(mp.0, mp.1 - 2, tip);
        }
    }
}

// ============================================================================
// 5. BRUTE & ELITE BRUTE — Armored Rhino-Beetle Juggernaut (r = 6.5)
// ============================================================================

fn draw_brute(pb: &mut PixelBuffer, fr: &Frame, elite: bool) {
    let s = fr.r / 6.5;
    let stomp = fr.anim * 0.72;

    // 1) Elite threat perimeter: crisp 1px rotating royal-gold corner arcs.
    if elite {
        let pulse = 0.55 + 0.45 * (fr.anim * 1.4).sin();
        let aura_r = fr.r + 2.3 * s;
        let gold = scale((255, 206, 72), 0.52 + 0.35 * pulse);
        for i in 0..8 {
            let a0 = i as f32 / 8.0 * TAU + fr.anim * 0.14;
            let mut last = (i32::MIN, i32::MIN);
            for step in 0..5 {
                let a = a0 + step as f32 * 0.07;
                let tx = (fr.pos.x + a.cos() * aura_r).round() as i32;
                let ty = (fr.pos.y + a.sin() * aura_r).round() as i32;
                if (tx, ty) != last {
                    blend_px(pb, tx, ty, gold, 0.72);
                    last = (tx, ty);
                }
            }
        }
    }

    // 2) Six thick heavy-stomping armored legs with lit top-left knees.
    let leg_core = apply_hit(
        if elite { (154, 98, 52) } else { (134, 54, 44) },
        fr.hit,
    );
    let leg_edge = apply_hit((46, 18, 22), fr.hit);
    let knee_hi = apply_hit(
        if elite { (252, 212, 92) } else { (212, 98, 72) },
        fr.hit,
    );
    for (i, &u_hip) in [2.6f32, -0.5, -3.6].iter().enumerate() {
        let phase = stomp + i as f32 * 2.1;
        for (side, sph) in [(-1.0f32, 0.0f32), (1.0, PI)] {
            let swing = (phase + sph).sin() * 1.35 * s;
            let hip = fr.px(u_hip * s, side * 4.2 * s);
            let knee = fr.px(u_hip * s + swing * 0.55, side * 7.0 * s);
            let foot = fr.px(u_hip * s + swing - 0.55 * s, side * 8.2 * s);
            thick_line(pb, hip, knee, leg_core, leg_edge);
            line_blend(pb, knee, foot, leg_edge, 0.95);
            pb.plot(knee.0, knee.1, knee_hi);
        }
    }

    // 3) Massive domed armored carapace + sculpted SDF rhino horn & cheek tusks!
    let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 10.6 * s) else {
        return;
    };

    let (sh_c, mid_c, hi_c, trim_c) = if elite {
        (
            (62, 18, 42),
            (158, 36, 78),
            (240, 108, 150),
            (252, 206, 76),
        )
    } else {
        (
            (68, 18, 20),
            (186, 54, 44),
            (250, 138, 84),
            (116, 34, 30),
        )
    };
    let (horn_sh, horn_mid, horn_hi) = if elite {
        ((138, 92, 26), (235, 182, 56), (255, 246, 178))
    } else {
        ((64, 32, 28), (156, 102, 74), (235, 198, 152))
    };
    let seam_c = (32, 12, 16);
    let rim_col = if fr.hit > 0.4 { (165, 95, 75) } else { (0, 0, 0) };

    // Returns (signed_distance, is_horn).
    let shell_sd = |u: f32, v: f32| -> (f32, bool) {
        let elytra = sd_ellipse(u, v, -1.55 * s, 0.0, 5.1 * s, 5.65 * s);
        let pronotum = sd_ellipse(u, v, 3.15 * s, 0.0, 2.75 * s, 4.45 * s);
        let shell = smin(elytra, pronotum, 0.95 * s);
        // Sculpted heavy armored rhino horn growing smoothly from the brow!
        let main_horn = sd_taper(u, v, (4.4 * s, 0.0), (8.6 * s, 0.0), 1.85 * s, 0.45 * s);
        let fork = sd_taper(
            u,
            v.abs(),
            (7.2 * s, 0.35 * s),
            (8.9 * s, 1.35 * s),
            0.55 * s,
            0.22 * s,
        );
        // Short heavy cheek tusks (or swept royal crown horns on Elite!).
        let side_horn = if elite {
            sd_taper(
                u,
                v.abs(),
                (3.6 * s, 2.8 * s),
                (6.8 * s, 5.0 * s),
                0.95 * s,
                0.28 * s,
            )
        } else {
            sd_taper(
                u,
                v.abs(),
                (4.4 * s, 2.5 * s),
                (6.2 * s, 3.35 * s),
                0.72 * s,
                0.22 * s,
            )
        };
        let horns = main_horn.min(fork).min(side_horn);
        let combined = smin(shell, horns, 0.4 * s);
        let is_horn = horns < shell + 0.1 * s && u > 4.6 * s;
        (combined, is_horn)
    };

    let damaged = fr.hp_frac < 0.68;
    let critical = fr.hp_frac < 0.38;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            let (d, is_horn) = shell_sd(u, v);
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            let mut dark = 0.0f32;
            if cover < 1.0 {
                if d < 1.0 * s {
                    dark = 0.68;
                } else {
                    let (su, sv) = fr.local(x - 2, y - 2);
                    if shell_sd(su, sv).0 < 0.55 * s {
                        dark = 0.42;
                    }
                }
            }
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_col, dark);
            if cover > 0.0 {
                let mut col = if is_horn {
                    let nu = ((u - 6.4 * s) / (2.5 * s)).clamp(-1.0, 1.0);
                    let nv = (v / (2.0 * s)).clamp(-1.0, 1.0);
                    let lit = fr.lit(nu * 0.6, nv);
                    let tip_t = ((u - 5.2 * s) / (3.4 * s)).clamp(0.0, 1.0);
                    let lum = 0.42 + 0.35 * tip_t + 0.55 * lit;
                    shade3(horn_sh, horn_mid, horn_hi, lum)
                } else {
                    let nu = ((u + 0.2 * s) / (6.4 * s)).clamp(-1.0, 1.0);
                    let nv = (v / (5.7 * s)).clamp(-1.0, 1.0);
                    let dome = (1.0 - (nu * nu * 0.55 + nv * nv * 0.72)).clamp(0.0, 1.0);
                    let lit = fr.lit(nu, nv);
                    let mut lum = 0.36 + 0.4 * dome + 0.62 * lit;
                    if lit > 0.46 && dome > 0.32 {
                        lum += 0.28;
                    }
                    let mut c = shade3(sh_c, mid_c, hi_c, lum);

                    let collar = (u - 1.5 * s).abs();
                    if collar < 0.55 * s {
                        c = mix(c, seam_c, 0.72);
                    } else if u > 1.5 * s && collar < 1.15 * s {
                        c = mix(c, trim_c, if elite { 0.85 } else { 0.35 });
                    }

                    if u < 1.4 * s && v.abs() < 0.45 * s {
                        c = if elite {
                            mix(trim_c, (255, 245, 180), (0.35 + 0.5 * lit).clamp(0.0, 1.0))
                        } else {
                            mix(c, seam_c, 0.72)
                        };
                    }

                    if d > -1.05 * s {
                        if elite {
                            c = mix(c, trim_c, 0.65);
                        } else if lit < -0.15 {
                            c = scale(c, 0.72);
                        }
                    }

                    if u < 0.8 * s
                        && ((u + 1.2 * s).abs() < 0.35 * s || (u + 3.8 * s).abs() < 0.35 * s)
                    {
                        c = scale(c, 0.8);
                    }

                    if damaged {
                        let crack1 = ((u + 1.8 * s) - (v - 1.5 * s) * 0.7).abs() < 0.38 * s
                            && u < 0.6 * s
                            && v > 0.4 * s;
                        let crack2 = ((u + 3.1 * s) + (v + 1.2 * s) * 0.85).abs() < 0.38 * s
                            && u < -0.5 * s
                            && v < -0.3 * s;
                        if crack1 || (critical && crack2) {
                            c = if critical {
                                (255, 165, 65)
                            } else {
                                (24, 10, 12)
                            };
                        }
                    }
                    c
                };

                col = apply_hit(col, fr.hit);
                out = mix(out, col, cover);
            }
            pb.plot(x, y, out);
        }
    }

    // 4) Recessed glowing ember eyes tucked beneath the brow.
    let eye_col = if fr.hit > 0.7 {
        (30, 14, 16)
    } else if elite {
        (255, 118, 232)
    } else {
        (255, 202, 56)
    };
    for side in [-1.0f32, 1.0] {
        let e0 = fr.px(4.7 * s, side * 2.15 * s);
        let e1 = fr.px(4.15 * s, side * 2.55 * s);
        pb.plot(e0.0, e0.1, eye_col);
        blend_px(pb, e1.0, e1.1, scale(eye_col, 0.72), 0.9);
    }
}

// ============================================================================
// 6. BOSS — Extermination Queen / Apex Hornet-Scorpion Matriarch (r = 14.0)
// ============================================================================

fn draw_boss(pb: &mut PixelBuffer, fr: &Frame, fire_cd: f32) {
    let s = fr.r / 14.0;
    let charge = if fire_cd <= 0.0 {
        0.85 + 0.15 * (fr.anim * 2.6).sin().abs()
    } else if fire_cd < 0.50 {
        ((0.50 - fire_cd) / 0.50).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let enraged = fr.hp_frac < 0.45;
    let pulse = 0.5 + 0.5 * (fr.anim * 2.2).sin();

    // Sculpted multi-part queen anatomy + curved scythe mandibles + crown horns + needle stinger!
    let mand_open = (1.8 + 3.0 * charge) * s;
    let body_sd = |u: f32, v: f32| -> (f32, u8) {
        let av = v.abs();
        let abd = sd_ellipse(u, v, -7.0 * s, 0.0, 6.4 * s, 7.9 * s);
        let stinger = sd_taper(u, v, (-12.0 * s, 0.0), (-16.0 * s, 0.0), 1.8 * s, 0.25 * s);
        let rear = smin(abd, stinger, 1.0 * s);

        let thorax = sd_ellipse(u, v, 1.9 * s, 0.0, 5.4 * s, 8.2 * s);
        let head = sd_ellipse(u, v, 9.2 * s, 0.0, 3.6 * s, 5.4 * s);
        let crown = sd_taper(u, av, (8.2 * s, 4.2 * s), (5.4 * s, 8.6 * s), 1.35 * s, 0.32 * s);
        // Sculpted curved scythe mandibles (outer base + inward-hooking blade!).
        let m_base = sd_taper(
            u,
            av,
            (11.0 * s, 3.1 * s),
            (13.8 * s, (3.7 + 1.5 * charge) * s),
            1.15 * s,
            0.65 * s,
        );
        let m_blade = sd_taper(
            u,
            av,
            (13.8 * s, (3.7 + 1.5 * charge) * s),
            (16.1 * s, mand_open),
            0.65 * s,
            0.22 * s,
        );
        let mand = m_base.min(m_blade);

        let front = smin(thorax, head, 1.0 * s);
        let core = smin(rear, front, 0.9 * s);
        let all = core.min(crown).min(mand);

        let part = if mand < core && mand < crown {
            4 // scythe mandible
        } else if crown < core {
            3 // crown horn
        } else if head < thorax && head < rear {
            2 // head
        } else if thorax < rear {
            1 // armored thorax
        } else {
            0 // striped abdomen + stinger
        };
        (all, part)
    };

    // 1) Deep hover drop shadow so the Boss looms over the arena floor.
    if let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 19.5 * s) {
        let sx = (3.0 * s).round() as i32;
        let sy = (3.5 * s).round() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (su, sv) = fr.local(x - sx, y - sy);
                let (d, _) = body_sd(su, sv);
                if d < 0.6 * s {
                    let a = (0.44 * (1.0 - (d / (1.5 * s)).clamp(0.0, 1.0))).max(0.18);
                    darken_px(pb, x, y, a);
                }
            }
        }
    }

    // 2) Eight jointed armored legs (4 pairs) gripping outward.
    let leg_core = apply_hit((116, 30, 66), fr.hit);
    let leg_dark = apply_hit((36, 12, 26), fr.hit);
    let leg_joint = apply_hit((244, 202, 130), fr.hit);
    for (i, &u_hip) in [5.4f32, 1.8, -2.2, -6.2].iter().enumerate() {
        let phase = fr.anim * 0.8 + i as f32 * 1.6;
        for (side, sph) in [(-1.0f32, 0.0f32), (1.0, PI)] {
            let swing = (phase + sph).sin() * 2.1 * s;
            let hip = fr.px(u_hip * s, side * 6.8 * s);
            let knee = fr.px(u_hip * s + swing * 0.5, side * 12.2 * s);
            let foot = fr.px(u_hip * s + swing - 1.4 * s, side * 15.0 * s);
            thick_line(pb, hip, knee, leg_core, leg_dark);
            line_blend(pb, knee, foot, leg_dark, 0.95);
            pb.plot(knee.0, knee.1, leg_joint);
        }
    }

    // 3) Sculpted royal carapace body.
    if let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, 17.6 * s) {
        let sh_c = (56, 14, 36);
        let mid_c = (196, 38, 96);
        let hi_c = (255, 138, 188);
        let armor_sh = (42, 12, 32);
        let armor_mid = (128, 26, 68);
        let armor_hi = (206, 68, 124);
        let obsidian = (30, 12, 28);
        let gold_sh = (146, 98, 42);
        let gold_hi = (252, 224, 146);
        let hot_core = mix((255, 118, 182), (255, 248, 224), charge);
        let rim_col = if fr.hit > 0.4 { (185, 95, 130) } else { (0, 0, 0) };

        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = fr.local(x, y);
                let (d, part) = body_sd(u, v);
                let cover = (0.55 - d * 1.18).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    if d < 1.2 * s {
                        let bg = pb.pixel_at(x as usize, y as usize);
                        pb.plot(x, y, mix(bg, rim_col, 0.72));
                    }
                    continue;
                }

                let bg = pb.pixel_at(x as usize, y as usize);
                let mut out = if cover < 1.0 {
                    mix(bg, rim_col, 0.72 * (1.0 - cover))
                } else {
                    bg
                };

                let (cu, ru, rv) = match part {
                    0 => (-7.0 * s, 6.8 * s, 8.0 * s),
                    1 => (1.9 * s, 5.6 * s, 8.2 * s),
                    _ => (9.5 * s, 4.0 * s, 5.5 * s),
                };
                let nu = ((u - cu) / ru).clamp(-1.0, 1.0);
                let nv = (v / rv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.72)).clamp(0.0, 1.0);
                let lit = fr.lit(nu, nv);
                let mut lum = 0.36 + 0.4 * dome + 0.62 * lit;
                if lit > 0.45 && dome > 0.35 {
                    lum += 0.24;
                }

                let mut col = match part {
                    0 => {
                        // Royal hornet-striped abdomen + needle venom stinger.
                        let mut c = shade3(sh_c, mid_c, hi_c, lum);
                        if u < -12.4 * s {
                            let tip_t = ((-12.4 * s - u) / (3.6 * s)).clamp(0.0, 1.0);
                            c = mix(gold_sh, hot_core, 0.35 + 0.65 * tip_t);
                        } else {
                            let band = ((u / s + 7.0) * 1.6).sin();
                            if band > 0.18 {
                                c = mix(c, obsidian, 0.8);
                            } else if charge > 0.1 || enraged {
                                let glow =
                                    (charge * 0.65 + if enraged { 0.28 * pulse } else { 0.0 })
                                        .min(0.92);
                                c = mix(c, hot_core, glow * dome);
                            }
                            if v.abs() < 0.7 * s {
                                c = mix(c, gold_hi, 0.55);
                            }
                        }
                        c
                    }
                    1 => {
                        // Layered chevron chitinous thorax plates + recessed glowing core!
                        let mut c = shade3(armor_sh, armor_mid, armor_hi, lum);
                        // Overlapping V-shaped armor plate seams.
                        let chev = ((u / s - 2.2) + v.abs() * 0.42 / s).rem_euclid(2.8);
                        if chev < 0.48 {
                            c = mix(c, obsidian, 0.82);
                        } else if chev < 0.85 {
                            c = mix(c, gold_hi, 0.35 * (0.5 + 0.5 * lit).clamp(0.0, 1.0));
                        }
                        // Gilded royal shoulder pauldrons along outer flank.
                        if d > -1.4 * s && v.abs() > 3.6 * s {
                            c = shade3(gold_sh, (226, 178, 92), gold_hi, 0.45 + 0.55 * lit);
                        }
                        // Dark armored collar framing the glowing central nova reactor!
                        let r_core = (((u - 2.2 * s).powi(2) + v * v).sqrt() / (2.5 * s))
                            .clamp(0.0, 1.2);
                        if r_core < 0.72 {
                            let intensity = (0.45
                                + 0.55 * charge
                                + if enraged { 0.25 * pulse } else { 0.0 })
                            .min(1.0);
                            c = mix((185, 32, 88), hot_core, (1.0 - r_core / 0.72) * intensity);
                        } else if r_core < 0.96 {
                            c = mix(c, obsidian, 0.75);
                        }
                        c
                    }
                    2 => {
                        // Angular armored royal helm with golden nasal crest.
                        let mut c = shade3(armor_sh, mid_c, hi_c, lum);
                        if v.abs() < 0.95 * s && u > 7.2 * s && u < 11.8 * s {
                            c = shade3(gold_sh, (226, 178, 92), gold_hi, 0.5 + 0.5 * lit);
                        }
                        c
                    }
                    3 => {
                        // Swept gilded crown horns.
                        shade3(gold_sh, (226, 178, 92), gold_hi, 0.45 + 0.55 * lit)
                    }
                    _ => {
                        // Scythe mandibles: obsidian outer blade, bright ivory-gold cutting edge.
                        let tip_t = ((u - 11.2 * s) / (5.0 * s)).clamp(0.0, 1.0);
                        shade3(obsidian, (92, 46, 72), gold_hi, 0.25 + 0.75 * tip_t)
                    }
                };

                // Glowing enraged battle-damage fissures below 55% HP.
                if fr.hp_frac < 0.55 && part <= 1 {
                    let f1 = ((u + 5.5 * s) - (v - 2.2 * s) * 0.8).abs() < 0.48 * s
                        && u < -2.5 * s
                        && v > 0.8 * s;
                    let f2 = ((u - 1.5 * s) + (v + 3.0 * s) * 0.75).abs() < 0.48 * s
                        && u > -s
                        && u < 5.5 * s
                        && v < -s;
                    if f1 || f2 {
                        col = mix(
                            (255, 110, 56),
                            (255, 240, 155),
                            pulse * 0.6 + charge * 0.4,
                        );
                    }
                }

                out = mix(out, apply_hit(col, fr.hit), cover);
                pb.plot(x, y, out);
            }
        }
    }

    // 4) Four blazing predator eyes with bright inner specular gleam.
    let eye_main = if fr.hit > 0.7 {
        (30, 14, 24)
    } else {
        mix((255, 236, 86), (255, 255, 235), charge * 0.5)
    };
    let eye_sub = if fr.hit > 0.7 {
        (30, 14, 24)
    } else {
        (255, 122, 62)
    };
    for side in [-1.0f32, 1.0] {
        let e0 = fr.px(10.3 * s, side * 2.35 * s);
        let e1 = fr.px(9.8 * s, side * 3.0 * s);
        pb.plot(e0.0, e0.1, eye_main);
        pb.plot(e1.0, e1.1, scale(eye_main, 0.88));
        let eo = fr.px(8.5 * s, side * 4.35 * s);
        pb.plot(eo.0, eo.1, eye_sub);
    }

    // 5) Four smoothly curved tapered hornet wings (tucked cleanly inside the HP arc!).
    let flap = (fr.anim * 1.8).sin();
    let wing_span = (13.8 + 2.1 * flap.abs()) * s;
    let wing_tint = mix((235, 148, 212), fr.accent, 0.42);
    let vein_col = mix((78, 28, 66), wing_tint, 0.32);
    if let Some((x0, x1, y0, y1)) = fr.bbox_pb(pb, wing_span + 1.5 * s) {
        let inv_span = 1.0 / (wing_span - 4.4 * s).max(1.0);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = fr.local(x, y);
                let av = v.abs();
                if av < 4.4 * s || av > wing_span {
                    continue;
                }
                let t = (av - 4.4 * s) * inv_span; // 0 at root -> 1 at tip

                // Graceful curved hornet forewing sweeping cleanly inside the HP ring.
                let fore_env = (t.powf(0.42) * (1.0 - t).powf(0.6) * 2.2).clamp(0.0, 1.0);
                let fore_mid = (2.8 - 4.6 * t - 2.9 * t * t + 1.3 * flap * t) * s;
                let fore_half = 2.9 * s * fore_env;
                let df = (u - fore_mid).abs();
                let in_fore = fore_half > 0.35 * s && df <= fore_half;

                // Sleeker hindwing nested cleanly behind forewing (ends at t = 0.72).
                let th = t / 0.72;
                let in_hind = if th < 1.0 {
                    let hind_env = (th.powf(0.45) * (1.0 - th).powf(0.6) * 2.2).clamp(0.0, 1.0);
                    let hind_mid = (-1.8 - 4.2 * th - 2.4 * th * th + 0.85 * flap * th) * s;
                    let hind_half = 2.2 * s * hind_env;
                    hind_half > 0.35 * s && (u - hind_mid).abs() <= hind_half
                } else {
                    false
                };

                if !in_fore && !in_hind {
                    continue;
                }
                let leading_spar = in_fore && (fore_mid + fore_half - u) < 0.85 * s;
                let cross_vein =
                    ((t * 4.5).fract() < 0.13) || ((u / s + 10.0).rem_euclid(3.4) < 0.34);
                let edge_fade = (1.0 - t * 0.42).clamp(0.28, 1.0);
                if leading_spar {
                    blend_px(
                        pb,
                        x,
                        y,
                        apply_hit(wing_tint, fr.hit * 0.45),
                        0.66 * edge_fade,
                    );
                } else if cross_vein {
                    blend_px(pb, x, y, vein_col, 0.38 * edge_fade);
                } else {
                    blend_px(pb, x, y, wing_tint, 0.22 * edge_fade);
                }
            }
        }
    }

    // Tiny glowing venom bead at the rear stinger tip.
    let tip = fr.px(-16.1 * s, 0.0);
    let bead = mix((255, 110, 175), (255, 250, 220), 0.35 + 0.65 * charge);
    blend_px(pb, tip.0, tip.1, bead, 0.9);

    // 6) 10-bullet nova telegraph pips aligned to exact firing angles (k/10 * TAU).
    if charge > 0.08 {
        let pip_r = fr.r + (1.6 + (1.0 - charge) * 2.8) * s;
        let pip_col = mix((255, 100, 165), (255, 250, 230), charge);
        for k in 0..10 {
            let a = k as f32 / 10.0 * TAU;
            let px = (fr.pos.x + a.cos() * pip_r).round() as i32;
            let py = (fr.pos.y + a.sin() * pip_r).round() as i32;
            pb.plot(px, py, pip_col);
            if charge > 0.5 {
                let glow = scale(pip_col, 0.55 * charge);
                pb.plot_add(px + 1, py, glow);
                pb.plot_add(px - 1, py, glow);
                pb.plot_add(px, py + 1, glow);
                pb.plot_add(px, py - 1, glow);
            }
        }
    }

    // 7) Clean segmented radial Boss Health Arc at `r + 4.3s`.
    draw_boss_hp_arc(pb, fr, s, pulse);
}

fn draw_boss_hp_arc(pb: &mut PixelBuffer, fr: &Frame, s: f32, pulse: f32) {
    let arc_r = fr.r + 4.3 * s;
    let steps = (arc_r * 7.0).round() as i32;
    let hp_col = if fr.hp_frac > 0.5 {
        mix((255, 165, 62), (255, 72, 136), (fr.hp_frac - 0.5) * 2.0)
    } else if fr.hp_frac > 0.25 {
        mix((255, 65, 58), (255, 165, 62), (fr.hp_frac - 0.25) * 4.0)
    } else {
        mix((190, 28, 36), (255, 86, 76), pulse)
    };
    let track_col = (18, 12, 18);
    let mut last = (i32::MIN, i32::MIN);

    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let near_tick = (t - 0.25).abs() < 0.012
            || (t - 0.50).abs() < 0.012
            || (t - 0.75).abs() < 0.012;
        let a = -FRAC_PI_2 + t * TAU;
        let x = (fr.pos.x + a.cos() * arc_r).round() as i32;
        let y = (fr.pos.y + a.sin() * arc_r).round() as i32;
        if (x, y) == last {
            continue;
        }
        last = (x, y);
        if near_tick {
            blend_px(pb, x, y, (10, 8, 12), 0.8);
        } else if t <= fr.hp_frac {
            // Subtle outer dark rim keeps the arc crisp over bright wings.
            let ox = (fr.pos.x + a.cos() * (arc_r + 1.0)).round() as i32;
            let oy = (fr.pos.y + a.sin() * (arc_r + 1.0)).round() as i32;
            darken_px(pb, ox, oy, 0.45);
            blend_px(pb, x, y, hp_col, 0.94);
        } else {
            blend_px(pb, x, y, track_col, 0.52);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor;
    use crate::game::art_rat;
    use crate::game::level::{Level, Theme};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const ALL_KINDS: [EnemyKind; 6] = [
        EnemyKind::Skitterer,
        EnemyKind::Bat,
        EnemyKind::Cat,
        EnemyKind::Spitter,
        EnemyKind::Brute,
        EnemyKind::Boss,
    ];

    fn make_enemy(kind: EnemyKind, pos: Vec2, ang: f32) -> Enemy {
        let mut rng = StdRng::seed_from_u64(42);
        let mut e = Enemy::spawn(kind, pos, 1.0, &mut rng);
        e.facing = Vec2::from_angle(ang);
        e.vel = e.facing * kind.speed();
        e.fire_cd = 1.5;
        e.anim = 0.7;
        e
    }

    #[test]
    fn enemies_draw_everywhere_without_panic() {
        for (bw, bh) in [(0usize, 0usize), (1, 1), (4, 4), (60, 40)] {
            let mut pb = PixelBuffer::new(bw, bh);
            for &kind in &ALL_KINDS {
                for i in 0..8 {
                    let ang = i as f32 / 8.0 * TAU;
                    for &(x, y) in &[
                        (30.0, 20.0),
                        (0.0, 0.0),
                        (-25.0, -25.0),
                        (500.0, 500.0),
                        (59.5, 39.5),
                    ] {
                        let mut e = make_enemy(kind, Vec2::new(x, y), ang);
                        if i == 0 {
                            e.facing = Vec2::ZERO;
                        }
                        if i % 2 == 0 {
                            e.flash = 0.12;
                        }
                        if i % 3 == 0 {
                            e.fire_cd = 0.05;
                            e.hp = e.max_hp * 0.25;
                            e.contact_cd = 0.5;
                        }
                        draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                        e.elite = true;
                        draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                    }
                }
            }
        }
    }

    #[test]
    fn hit_flash_brightens_every_kind() {
        for &kind in &ALL_KINDS {
            let render = |flash: f32| {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear((16, 22, 20));
                let mut e = make_enemy(kind, Vec2::new(32.0, 32.0), 0.2);
                e.flash = flash;
                draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                let mut sum = 0u64;
                for y in 0..pb.h {
                    for x in 0..pb.w {
                        let c = pb.pixel_at(x, y);
                        sum += c.0 as u64 + c.1 as u64 + c.2 as u64;
                    }
                }
                sum
            };
            let normal = render(0.0);
            let hit = render(0.12);
            assert!(
                hit > normal + 200,
                "{kind:?}: hit flash ({hit}) should brighten over normal ({normal})"
            );
        }
    }

    #[test]
    fn ranged_telegraph_changes_spitter_and_boss() {
        for kind in [EnemyKind::Spitter, EnemyKind::Boss] {
            let render = |cd: f32| {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear((16, 22, 20));
                let mut e = make_enemy(kind, Vec2::new(32.0, 32.0), 0.0);
                e.fire_cd = cd;
                draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                let mut diff_hash = 0u64;
                for y in 0..pb.h {
                    for x in 0..pb.w {
                        let c = pb.pixel_at(x, y);
                        diff_hash = diff_hash
                            .wrapping_mul(31)
                            .wrapping_add(c.0 as u64 + (c.1 as u64) * 257 + (c.2 as u64) * 65537);
                    }
                }
                diff_hash
            };
            assert_ne!(
                render(1.8),
                render(0.05),
                "{kind:?}: firing telegraph must be visually distinct"
            );
        }
    }

    #[test]
    fn elite_brute_differs_from_normal_brute() {
        let render = |elite: bool| {
            let mut pb = PixelBuffer::new(48, 48);
            pb.clear((16, 22, 20));
            let mut e = make_enemy(EnemyKind::Brute, Vec2::new(24.0, 24.0), 0.3);
            e.elite = elite;
            draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
            (0..48)
                .flat_map(|y| (0..48).map(move |x| (x, y)))
                .map(|(x, y)| pb.pixel_at(x, y))
                .collect::<Vec<_>>()
        };
        assert_ne!(render(false), render(true));
    }

    /// Every kind must paint visible body pixels inside its collision radius
    /// and never look identical to the white rat.
    #[test]
    fn drawn_body_covers_center_and_distinguishes_every_kind() {
        let bg = (14, 22, 20);
        let mut signatures = Vec::new();
        for &kind in &ALL_KINDS {
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear(bg);
            let e = make_enemy(kind, Vec2::new(32.0, 32.0), 0.0);
            draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
            // Center pixel must be painted (not background).
            let center = pb.pixel_at(32, 32);
            assert_ne!(center, bg, "{kind:?} left center pixel unpainted");
            // Compute average colour & lit pixel count within collision radius.
            let r = e.radius.ceil() as i32;
            let mut painted = 0usize;
            let mut sig = 0u64;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy <= r * r {
                        let c = pb.pixel_at((32 + dx) as usize, (32 + dy) as usize);
                        if c != bg {
                            painted += 1;
                        }
                        sig = sig
                            .wrapping_mul(31)
                            .wrapping_add(c.0 as u64 + (c.1 as u64) * 257 + (c.2 as u64) * 65537);
                    }
                }
            }
            assert!(painted >= 4, "{kind:?} too sparse inside radius ({painted})");
            signatures.push((kind, sig));
        }
        // Every kind should produce a distinct visual signature.
        for i in 0..signatures.len() {
            for j in (i + 1)..signatures.len() {
                assert_ne!(
                    signatures[i].1, signatures[j].1,
                    "{:?} and {:?} rendered identically",
                    signatures[i].0, signatures[j].0
                );
            }
        }
    }

    /// Worst-case 600-enemy swarm draw must complete cleanly without panicking.
    #[test]
    fn draws_600_enemy_swarm_cleanly() {
        let mut pb = PixelBuffer::new(200, 120);
        pb.clear((14, 22, 20));
        let mut rng = StdRng::seed_from_u64(99);
        let kinds = [
            EnemyKind::Skitterer,
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
        ];
        for i in 0..600 {
            let kind = if i == 599 {
                EnemyKind::Boss
            } else {
                kinds[i % kinds.len()]
            };
            let x = (i * 19 % 196) as f32 + 2.0;
            let y = ((i * 37) % 116) as f32 + 2.0;
            let mut e = Enemy::spawn(kind, Vec2::new(x, y), 1.0, &mut rng);
            e.facing = Vec2::from_angle(i as f32 * 0.37);
            e.anim = i as f32 * 0.2;
            e.elite = kind == EnemyKind::Brute && i % 12 == 0;
            draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
        }
    }

    /// Detailed bestiary + states sheet across all three themes:
    /// `RATRUN_DUMP=enemies cargo test dump_enemy_sheet` writes `/tmp/ratrun_enemies.ppm`.
    #[test]
    fn dump_enemy_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("enemies") {
            return;
        }
        let w = 260usize;
        let h = 128usize;
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();

        for theme in Theme::all() {
            let level = Level::new(theme);
            let cam = Vec2::new(300.0, 300.0);
            let mut pb = PixelBuffer::new(w, h);
            art_floor::draw_floor(&mut pb, &level, cam, 0.0);
            let acc = level.palette.accent;

            // Row 1 (y=24): Lineup facing down-right + Rat for scale/contrast.
            let y1 = cam.y + 24.0;
            let mut rat = Player::new(Vec2::new(cam.x + 20.0, y1), WeaponKind::Gnaw);
            rat.face = Vec2::new(0.4, 0.9).normalized();
            art_rat::draw_rat(&mut pb, &rat, 0.5, cam);

            let lineup: [(EnemyKind, bool, f32); 6] = [
                (EnemyKind::Skitterer, false, 48.0),
                (EnemyKind::Bat, false, 74.0),
                (EnemyKind::Cat, false, 104.0),
                (EnemyKind::Spitter, false, 136.0),
                (EnemyKind::Brute, false, 174.0),
                (EnemyKind::Brute, true, 218.0),
            ];
            for &(kind, elite, x) in &lineup {
                let mut e = make_enemy(kind, Vec2::new(cam.x + x, y1), 0.7);
                e.elite = elite;
                draw_enemy(&mut pb, &e, cam, acc);
            }

            // Row 2 (y=60): Action states (angles, wing flap, swipe, spitter telegraph, flash, damaged).
            let y2 = cam.y + 60.0;
            for (k, &(dx, dy, ang, anim)) in [
                (18.0, -4.0, -2.4, 0.2),
                (28.0, 4.0, -1.8, 1.8),
                (38.0, -2.0, -2.9, 3.1),
            ]
            .iter()
            .enumerate()
            {
                let mut e = make_enemy(EnemyKind::Skitterer, Vec2::new(cam.x + dx, y2 + dy), ang);
                e.anim = anim;
                if k == 1 {
                    e.flash = 0.11;
                }
                draw_enemy(&mut pb, &e, cam, acc);
            }
            let mut bat1 = make_enemy(EnemyKind::Bat, Vec2::new(cam.x + 64.0, y2 - 3.0), -0.5);
            bat1.anim = 1.5;
            draw_enemy(&mut pb, &bat1, cam, acc);
            let mut bat2 = make_enemy(EnemyKind::Bat, Vec2::new(cam.x + 84.0, y2 + 3.0), 2.5);
            bat2.anim = -1.5;
            draw_enemy(&mut pb, &bat2, cam, acc);

            let mut cat = make_enemy(EnemyKind::Cat, Vec2::new(cam.x + 114.0, y2), 2.8);
            cat.contact_cd = 0.5;
            cat.anim = 2.1;
            draw_enemy(&mut pb, &cat, cam, acc);

            let mut sp1 = make_enemy(EnemyKind::Spitter, Vec2::new(cam.x + 144.0, y2 - 4.0), 0.0);
            sp1.fire_cd = 0.35;
            draw_enemy(&mut pb, &sp1, cam, acc);
            let mut sp2 = make_enemy(EnemyKind::Spitter, Vec2::new(cam.x + 144.0, y2 + 12.0), 0.0);
            sp2.fire_cd = 0.05;
            draw_enemy(&mut pb, &sp2, cam, acc);

            let mut br_dmg = make_enemy(EnemyKind::Brute, Vec2::new(cam.x + 178.0, y2), -0.6);
            br_dmg.hp = br_dmg.max_hp * 0.28;
            draw_enemy(&mut pb, &br_dmg, cam, acc);
            let mut br_hit = make_enemy(EnemyKind::Brute, Vec2::new(cam.x + 218.0, y2), -2.4);
            br_hit.flash = 0.10;
            br_hit.elite = true;
            draw_enemy(&mut pb, &br_hit, cam, acc);

            // Row 3 (y=98): Boss showcase — full HP, telegraphing nova, enraged low HP!
            let y3 = cam.y + 98.0;
            let mut boss1 = make_enemy(EnemyKind::Boss, Vec2::new(cam.x + 48.0, y3), 0.55);
            boss1.hp = boss1.max_hp * 0.92;
            boss1.fire_cd = 1.0;
            draw_enemy(&mut pb, &boss1, cam, acc);

            let mut boss2 = make_enemy(EnemyKind::Boss, Vec2::new(cam.x + 130.0, y3), 1.35);
            boss2.hp = boss2.max_hp * 0.55;
            boss2.fire_cd = 0.06;
            boss2.anim = 1.2;
            draw_enemy(&mut pb, &boss2, cam, acc);

            let mut boss3 = make_enemy(EnemyKind::Boss, Vec2::new(cam.x + 212.0, y3), -2.2);
            boss3.hp = boss3.max_hp * 0.22;
            boss3.fire_cd = 0.22;
            boss3.anim = 2.6;
            draw_enemy(&mut pb, &boss3, cam, acc);

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_enemies.ppm", out).unwrap();
    }
}
