//! The Sewer's four new bodies (`WAVE4_ROSTERS.md` "Sewer", Silhouette column):
//! copper and bone against wet masonry.
//!
//! | kind | silhouette | motion & poses |
//! |---|---|---|
//! | Pipe Eel | bone hook on a thick ribbed neck, thin scribbled tail | continuous tail scribble + turn lag, jaw flare on contact bite |
//! | Grate Archer | fluted barnacle cone on two articulated copper feet, puckering lip | two-foot waddle stride, feet brace + lip swells & ignites before shot, post-shot recoil |
//! | Sump Leech | glossy ink-black comma with wet highlight ribbon; opens into a pale toothed sucker | undulating comma tail curl; coils & flares pale sucker on warn, lunges on hold |
//! | Sluice Toad | squat mossy slab wider than long, twin raised eye turrets, four heavy legs | continuous belly haul (stretch / gather), contact mouth gulp; elite wears protruding split brass valve collar + thick double shoulder stripe |
//!
//! Every draw function keeps `art_enemy`'s `BodyDraw` signature and is
//! registered in its `bespoke_body` / `bespoke_lod` tables. Each body builds
//! its own local screen frame from `&Enemy` and `cam` (matching `art_enemy`'s
//! `LIGHT`, `FLASH_MAX`, dark silhouette rim and down-right drop shadow), so
//! nothing private in `art_enemy::Frame` is read.
//!
//! [`draw_leech_tongue`] draws the Sump Leech's 24×4 tongue footprint from the
//! exact simulation geometry (`floor_sewer::leech_tongue`,
//! `floor_sewer::LEECH_TONGUE_WIDTH`) on the post-lighting hostile layer.
//! Render-only: nothing here mutates simulation state or draws rng.

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_sewer;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// Direction *toward* the light in screen space (up-left), matching `art_enemy`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };
/// `art_enemy::FLASH_MAX`: the hit flash's full length.
const FLASH_MAX: f32 = 0.12;

// ---- Palettes (Sewer: copper and bone against wet dark masonry) -----------

/// Pipe Eel: weathered bone, marrow-brown shadow, bleached ivory rim, amber eye.
const EEL_SHADOW: Rgb = (78, 66, 54);
const EEL_BONE: Rgb = (214, 204, 178);
const EEL_IVORY: Rgb = (254, 248, 228);
const EEL_JOINT: Rgb = (54, 44, 36);
const EEL_EYE: Rgb = (255, 168, 52);

/// Grate Archer: calcified barnacle shell, copper-rust base & feet, puckering lip.
const ARCHER_SHADOW: Rgb = (58, 74, 68);
const ARCHER_SHELL: Rgb = (152, 174, 156);
const ARCHER_HI: Rgb = (238, 236, 214);
const ARCHER_GROOVE: Rgb = (42, 56, 54);
const ARCHER_CRUST: Rgb = (138, 88, 52);
const ARCHER_FOOT_DARK: Rgb = (72, 40, 24);
const ARCHER_FOOT: Rgb = (168, 98, 54);
const ARCHER_CLAW: Rgb = (232, 174, 108);
const ARCHER_LIP: Rgb = (232, 224, 202);
const ARCHER_CHARGE: Rgb = (255, 198, 96);
const ARCHER_EYE: Rgb = (255, 164, 64);

/// Sump Leech: glossy ink-black comma with bright wet sheen, pale bone sucker.
const LEECH_DEEP: Rgb = (22, 19, 30);
const LEECH_BODY: Rgb = (54, 50, 68);
const LEECH_SHEEN: Rgb = (146, 150, 174);
const LEECH_SPEC: Rgb = (228, 232, 246);
const LEECH_SUCKER: Rgb = (242, 228, 206);
const LEECH_TONGUE: Rgb = (236, 134, 146);
const LEECH_TONGUE_HI: Rgb = (255, 214, 218);
const LEECH_TONGUE_DARK: Rgb = (146, 58, 74);

/// Sluice Toad: wet sluice-moss slab, pale belly ridge, gold eye turrets, brass collar.
const TOAD_SHADOW: Rgb = (40, 62, 44);
const TOAD_HIDE: Rgb = (96, 134, 86);
const TOAD_HI: Rgb = (182, 210, 140);
const TOAD_BELLY: Rgb = (196, 212, 152);
const TOAD_WART: Rgb = (46, 68, 42);
const TOAD_WART_HI: Rgb = (154, 174, 108);
const TOAD_LEG_CORE: Rgb = (66, 98, 60);
const TOAD_LEG_DARK: Rgb = (28, 42, 30);
const TOAD_EYE: Rgb = (248, 224, 96);
const TOAD_EYE_ELITE: Rgb = (255, 198, 72);
const TOAD_BRASS_SH: Rgb = (124, 80, 24);
const TOAD_BRASS: Rgb = (224, 172, 60);
const TOAD_BRASS_HI: Rgb = (255, 242, 158);

// ---- Local frame & pixel helpers ------------------------------------------

/// One body's local screen frame: position, facing basis, light projections, hit flash.
#[derive(Clone, Copy)]
struct Local {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    hit: f32,
    flash_col: Rgb,
    flash_str: f32,
}

impl Local {
    fn of(e: &Enemy, cam: Vec2) -> Local {
        let len_sq = e.facing.x * e.facing.x + e.facing.y * e.facing.y;
        let f = if len_sq > 1e-4 {
            let inv = 1.0 / len_sq.sqrt();
            Vec2::new(e.facing.x * inv, e.facing.y * inv)
        } else {
            Vec2::new(1.0, 0.0)
        };
        let p = Vec2::new(-f.y, f.x);
        let hit = (e.flash / FLASH_MAX).clamp(0.0, 1.0);
        let (flash_col, flash_str) = if hit > 0.0 {
            let fc = if hit > 0.6 {
                (255, 244, 218)
            } else {
                (255, 218, 152)
            };
            (fc, (0.30 + hit * 0.56).min(0.86))
        } else {
            ((0, 0, 0), 0.0)
        };
        Local {
            pos: e.pos - cam,
            f,
            p,
            fl: f.x * LIGHT.x + f.y * LIGHT.y,
            pl: p.x * LIGHT.x + p.y * LIGHT.y,
            hit,
            flash_col,
            flash_str,
        }
    }

    /// Local `(u forward, v sideways)` → screen pixel.
    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    /// Screen pixel `(x, y)` → local `(u, v)`.
    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    /// Directional light dot product for a local surface normal `(nu, nv)`.
    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    /// The warm hit flash every enemy body takes.
    #[inline]
    fn flash(&self, c: Rgb) -> Rgb {
        if self.flash_str <= 0.0 {
            c
        } else {
            mix(c, self.flash_col, self.flash_str)
        }
    }

    /// Tight oriented screen bounding box for local `[u_min, u_max] × [v_min, v_max]`,
    /// expanded in `+x/+y` by `pad` for the down-right drop shadow.
    fn bbox(
        &self,
        pb: &PixelBuffer,
        u_min: f32,
        u_max: f32,
        v_min: f32,
        v_max: f32,
        pad: (i32, i32),
    ) -> Option<(i32, i32, i32, i32)> {
        if pb.w == 0 || pb.h == 0 {
            return None;
        }
        let u_mid = 0.5 * (u_min + u_max);
        let v_mid = 0.5 * (v_min + v_max);
        let hu = 0.5 * (u_max - u_min);
        let hv = 0.5 * (v_max - v_min);
        let cx = self.pos.x + self.f.x * u_mid + self.p.x * v_mid;
        let cy = self.pos.y + self.f.y * u_mid + self.p.y * v_mid;
        let hx = hu * self.f.x.abs() + hv * self.p.x.abs();
        let hy = hu * self.f.y.abs() + hv * self.p.y.abs();
        let x0 = ((cx - hx).floor() as i32).max(0);
        let x1 = ((cx + hx).ceil() as i32 + pad.0).min(pb.w as i32 - 1);
        let y0 = ((cy - hy).floor() as i32).max(0);
        let y1 = ((cy + hy).ceil() as i32 + pad.1).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            None
        } else {
            Some((x0, x1, y0, y1))
        }
    }
}

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline]
fn scale_rgb(a: Rgb, f: f32) -> Rgb {
    let f = f.max(0.0);
    (
        (a.0 as f32 * f).min(255.0) as u8,
        (a.1 as f32 * f).min(255.0) as u8,
        (a.2 as f32 * f).min(255.0) as u8,
    )
}

#[inline]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(shadow, mid, l * (1.0 / 0.62))
    } else {
        mix(mid, hi, (l - 0.62) * (1.0 / 0.53))
    }
}

#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(x as usize, y as usize);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale_rgb(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

/// A one-pixel line, blended.
fn line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    for _ in 0..256 {
        blend(pb, x, y, c, alpha);
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

/// A shaded two-pixel limb stroke with a soft down-right contact shadow.
fn thick_line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), core: Rgb, edge: Rgb) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    let horiz = dx >= -dy;
    for _ in 0..256 {
        darken(pb, x + 1, y + 1, 0.34);
        if horiz {
            blend(pb, x, y + 1, edge, 0.88);
        } else {
            blend(pb, x + 1, y, edge, 0.88);
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

// ---- SDF building blocks --------------------------------------------------

#[inline]
fn sd_disc(u: f32, v: f32, cu: f32, cv: f32, r: f32) -> f32 {
    let (du, dv) = (u - cu, v - cv);
    (du * du + dv * dv).sqrt() - r
}

#[inline]
fn sd_ellipse(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let nu = (u - cu) / ru;
    let nv = (v - cv) / rv;
    ((nu * nu + nv * nv).sqrt() - 1.0) * ru.min(rv)
}

#[inline]
fn sd_taper(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> f32 {
    let (pax, pay) = (u - a.0, v - a.1);
    let (bax, bay) = (b.0 - a.0, b.1 - a.1);
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt() - (r0 + (r1 - r0) * h)
}

#[inline]
fn sd_round_box(u: f32, v: f32, cu: f32, cv: f32, hu: f32, hv: f32, r: f32) -> f32 {
    let qx = (u - cu).abs() - (hu - r).max(0.0);
    let qy = (v - cv).abs() - (hv - r).max(0.0);
    let out = qx.max(0.0).hypot(qy.max(0.0));
    let inside = qx.max(qy).min(0.0);
    out + inside - r
}

#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let diff = (a - b).abs();
    if diff >= k {
        a.min(b)
    } else {
        let h = (k - diff) / k;
        a.min(b) - h * h * k * 0.25
    }
}

/// Rasterise one SDF body with anti-aliased coverage, a crisp dark silhouette
/// rim (matching `art_enemy.rs`), and a soft down-right drop shadow.
#[allow(clippy::too_many_arguments)]
fn raster_sdf(
    pb: &mut PixelBuffer,
    lf: &Local,
    bounds: (f32, f32, f32, f32),
    rim_lim: f32,
    shadow_off: (i32, i32),
    rim_tint: Rgb,
    sd: impl Fn(f32, f32) -> f32,
    shade: impl Fn(f32, f32, f32) -> Rgb,
) {
    let (u_min, u_max, v_min, v_max) = bounds;
    let Some((x0, x1, y0, y1)) = lf.bbox(pb, u_min, u_max, v_min, v_max, shadow_off) else {
        return;
    };
    let rim_col = if lf.hit > 0.4 { rim_tint } else { (0, 0, 0) };
    let sh_lim = rim_lim * 0.45;
    let du_sh = -(shadow_off.0 as f32 * lf.f.x + shadow_off.1 as f32 * lf.f.y);
    let dv_sh = -(shadow_off.0 as f32 * lf.p.x + shadow_off.1 as f32 * lf.p.y);

    for y in y0..=y1 {
        let (mut u, mut v) = lf.local(x0, y);
        for x in x0..=x1 {
            let in_box = (u_min..=u_max).contains(&u) && (v_min..=v_max).contains(&v);
            let d = if in_box { sd(u, v) } else { 10.0 };
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            if cover >= 1.0 {
                pb.plot(x, y, lf.flash(shade(u, v, d)));
            } else {
                let dark = if d < rim_lim {
                    0.66
                } else {
                    let su = u + du_sh;
                    let sv = v + dv_sh;
                    if (u_min..=u_max).contains(&su)
                        && (v_min..=v_max).contains(&sv)
                        && sd(su, sv) < sh_lim
                    {
                        0.36
                    } else {
                        0.0
                    }
                };
                if cover > 0.0 || dark > 0.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let mut out = if dark > 0.0 {
                        mix(bg, rim_col, dark)
                    } else {
                        bg
                    };
                    if cover > 0.0 {
                        out = mix(out, lf.flash(shade(u, v, d)), cover);
                    }
                    pb.plot(x, y, out);
                }
            }
            u += lf.f.x;
            v += lf.p.x;
        }
    }
}

/// Signed angular lag between `e.vel` and `e.facing` (radians, clamped).
#[inline]
fn turn_lag(lf: &Local, vel: Vec2) -> f32 {
    let vsq = vel.x * vel.x + vel.y * vel.y;
    if vsq <= 4.0 {
        return 0.0;
    }
    let inv = 1.0 / vsq.sqrt();
    let (vx, vy) = (vel.x * inv, vel.y * inv);
    let cross = lf.f.x * vy - lf.f.y * vx;
    let dot = lf.f.x * vx + lf.f.y * vy;
    cross.atan2(dot).clamp(-1.0, 1.0)
}

// ---- Pipe Eel -------------------------------------------------------------

/// Bone hook on a thick ribbed neck, trailing a whipping scribbled tail.
/// Turn lag bends the neck and tail around corners; contact snaps the barb wide.
pub(crate) fn draw_pipe_eel(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * 0.25;
    let phase = e.anim * 1.15;
    let lag = turn_lag(&lf, e.vel);
    let sway_back = (0.45 * phase.cos() + lag * 0.55) * s;
    let bite = ((e.contact_cd - 0.20) * 2.5).clamp(0.0, 1.0);

    // 1) Thin scribbled bone-and-cartilage tail whipping behind the neck.
    let mut tail_pts = [(0i32, 0i32); 7];
    tail_pts[0] = lf.px(-2.9 * s, sway_back);
    for (k, slot) in tail_pts.iter_mut().enumerate().skip(1) {
        let kf = k as f32;
        let u = (-2.9 - kf * 1.1) * s;
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let whip = (phase - kf * 1.15).cos();
        let v = sway_back
            + (lag * kf * 0.30
                + side * (0.55 + kf * 0.20)
                + whip * (0.65 + kf * 0.18))
                * s;
        *slot = lf.px(u, v);
    }
    // Shadow under the scribbled tail first.
    for w in tail_pts.windows(2) {
        let a = (w[0].0 + 1, w[0].1 + 1);
        let b = (w[1].0 + 1, w[1].1 + 1);
        line(pb, a, b, (0, 0, 0), 0.36);
    }
    // Tail segments fade from bone-marrow at the root to dark needle tip,
    // with bright cartilage glint dots at the scribble elbows.
    for (k, w) in tail_pts.windows(2).enumerate() {
        let t = k as f32 / 5.0;
        let col = lf.flash(mix(EEL_BONE, EEL_SHADOW, 0.28 + t * 0.52));
        line(pb, w[0], w[1], col, 0.96 - t * 0.14);
    }
    for idx in [1usize, 3, 5] {
        let pt = tail_pts[idx];
        blend(pb, pt.0, pt.1, lf.flash(EEL_IVORY), 0.78);
    }

    // 2) Thick articulated neck + sculpted skull + recurved bone hook barb.
    let neck_a = (-3.2 * s, sway_back);
    let neck_b = (2.7 * s, 0.0);
    let bend_a = (4.2 * s, 0.65 * s);
    let bend_b = ((3.85 + bite * 0.55) * s, (3.15 + bite * 0.45) * s);
    let tip = ((1.45 + bite * 0.35) * s, (3.70 + bite * 0.50) * s);
    let k_join = 0.55 * s;

    let eel_sd = |u: f32, v: f32| -> f32 {
        let neck = sd_taper(u, v, neck_a, neck_b, 1.50 * s, 1.85 * s);
        let skull = sd_ellipse(u, v, 3.35 * s, 0.15 * s, 1.95 * s, 1.75 * s);
        let body = smin(neck, skull, k_join);
        let bend = sd_taper(u, v, bend_a, bend_b, 1.22 * s, 0.88 * s);
        let barb = sd_taper(u, v, bend_b, tip, 0.84 * s, 0.32 * s);
        let hook = smin(bend, barb, 0.35 * s);
        smin(body, hook, 0.35 * s)
    };

    let inv_u = 1.0 / (4.4 * s);
    let inv_v = 1.0 / (2.8 * s);
    let u_min = -5.4 * s;
    let u_max = (6.3 + bite * 0.6) * s;
    let v_min = -3.4 * s;
    let v_max = (5.3 + bite * 0.6) * s;

    raster_sdf(
        pb,
        &lf,
        (u_min, u_max, v_min, v_max),
        0.88 * s,
        (1, 1),
        (185, 165, 130),
        eel_sd,
        |u, v, d| {
            let nu = ((u - 0.8 * s) * inv_u).clamp(-1.0, 1.0);
            let nv = ((v - 0.5 * s) * inv_v).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.38 + nv * nv * 0.65)).clamp(0.0, 1.0);
            let lit = lf.lit(nu, nv);
            let mut lum = 0.42 + 0.34 * dome + 0.58 * lit;
            // Bright bleached-bone rim catch toward the lamp.
            if d > -s && lit > 0.25 {
                lum += 0.24;
            }
            let mut col = shade3(EEL_SHADOW, EEL_BONE, EEL_IVORY, lum);

            // Articulated vertebra rings along the thick neck.
            if u < 2.2 * s && v < 2.0 * s {
                let rib = ((u / s + 3.2) * 0.78).rem_euclid(1.0);
                if rib < 0.24 {
                    col = mix(col, EEL_JOINT, 0.65);
                } else if v.abs() < 0.48 * s {
                    // Pale dorsal bone spine ridge.
                    col = mix(col, EEL_IVORY, 0.42);
                }
            }
            // Subtle dark brow ridge separating skull from the hook jaw.
            if (2.4 * s..4.4 * s).contains(&u) && (v - 1.35 * s).abs() < 0.30 * s {
                col = mix(col, EEL_JOINT, 0.45);
            }
            // Bleached ivory barb tip.
            if v > 2.6 * s && u < 2.6 * s {
                col = mix(col, EEL_IVORY, 0.48);
            }
            col
        },
    );

    // 3) Dark recessed orbital socket + bright copper-amber predatory eye glint.
    let sock = lf.px(3.55 * s, -0.55 * s);
    let eye = lf.px(3.85 * s, -0.60 * s);
    let brow = lf.px(3.30 * s, -1.25 * s);
    blend(pb, sock.0, sock.1, lf.flash(EEL_JOINT), 0.92);
    let eye_col = if lf.hit > 0.7 { (44, 24, 16) } else { EEL_EYE };
    pb.plot(eye.0, eye.1, eye_col);
    blend(pb, brow.0, brow.1, lf.flash(EEL_IVORY), 0.85);

    // Sharp ivory fang glint inside the hook throat on a contact bite.
    if bite > 0.15 {
        let fang = lf.px((2.35 + bite * 0.3) * s, (2.25 + bite * 0.35) * s);
        pb.plot(fang.0, fang.1, EEL_IVORY);
    }
    // Crisp bleached glint on the recurved barb point.
    let tip_px = lf.px(tip.0, tip.1);
    blend(pb, tip_px.0, tip_px.1, lf.flash(EEL_IVORY), 0.92);
}

pub(crate) fn draw_pipe_eel_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * 0.25;
    let whip = (e.anim * 1.15).cos();
    let bone = lf.flash(EEL_BONE);
    let ivory = lf.flash(EEL_IVORY);
    let dark = lf.flash(EEL_SHADOW);

    // Scribbled 3-segment tail behind the neck.
    let t0 = lf.px(-2.4 * s, 0.0);
    let t1 = lf.px(-4.8 * s, (1.1 + whip * 0.9) * s);
    let t2 = lf.px(-7.4 * s, (-1.1 + whip * 1.1) * s);
    let t3 = lf.px(-9.0 * s, (1.0 - whip * 0.8) * s);
    line(pb, t0, t1, dark, 0.92);
    line(pb, t1, t2, dark, 0.88);
    line(pb, t2, t3, dark, 0.78);

    // Thick bone neck + unmistakable sideways recurved "J" hook with dark rim.
    let n0 = lf.px(-2.6 * s, 0.0);
    let n1 = lf.px(4.1 * s, 0.1 * s);
    let h1 = lf.px(3.9 * s, 3.1 * s);
    let h2 = lf.px(1.7 * s, 3.6 * s);
    thick_line(pb, n0, n1, bone, dark);
    thick_line(pb, n1, h1, bone, dark);
    line(pb, h1, h2, ivory, 1.0);
    // Lit skull crown + amber eye glint.
    let top = lf.px(3.4 * s, -0.8 * s);
    pb.plot(top.0, top.1, ivory);
    let eye = lf.px(3.9 * s, -0.4 * s);
    pb.plot(eye.0, eye.1, if lf.hit > 0.7 { dark } else { EEL_EYE });
}

// ---- Grate Archer ---------------------------------------------------------

/// The Spitter's muzzle swell: 0 while walking, rising to 1 over the last
/// 0.75 s before the shot leaves.
#[inline]
fn archer_charge(e: &Enemy) -> f32 {
    (1.0 - e.fire_cd / 0.75).clamp(0.0, 1.0)
}

/// Fluted barnacle cone striding on two articulated copper feet. The lip
/// thrusts forward, puckers wide, and ignites before a shot; the shell recoils
/// immediately after release.
pub(crate) fn draw_grate_archer(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let base_charge = archer_charge(e);
    let charge = if e.fire_cd <= 0.0 {
        0.86 + 0.14 * (e.anim * 2.2).sin().abs()
    } else {
        base_charge
    };
    let recoil = if e.fire_cd > 2.02 {
        ((e.fire_cd - 2.02) / 0.18).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Feet alternate in a jaunty waddle while moving, then plant wide to brace.
    let stride = (e.anim * 0.95).cos() * (1.0 - charge * 0.85);
    let wobble = stride * 0.28 * s;

    // 1) Two articulated copper-bronze barnacle feet under the shell.
    let leg_core = lf.flash(ARCHER_FOOT);
    let leg_edge = lf.flash(ARCHER_FOOT_DARK);
    let claw_col = lf.flash(ARCHER_CLAW);
    for side in [-1.0f32, 1.0] {
        let step_u = side * stride;
        let hip = lf.px((-1.6 - 0.45 * charge) * s, side * 2.3 * s + wobble);
        let knee = lf.px(
            (-1.35 + step_u * 1.05 - 0.55 * charge) * s,
            side * (3.85 + 0.35 * charge) * s + wobble * 0.5,
        );
        let foot = lf.px(
            (-0.15 + step_u * 1.65 - 0.75 * charge) * s,
            side * (4.45 + 0.40 * charge) * s,
        );
        thick_line(pb, hip, knee, leg_core, leg_edge);
        line(pb, knee, foot, leg_edge, 0.96);
        pb.plot(foot.0, foot.1, claw_col);
    }

    // 2) Fluted calcareous barnacle cone + puckering lip ring.
    let shift_u = (-0.55 * charge - 0.65 * recoil) * s;
    let u_base = -3.7 * s + shift_u;
    let u_neck = 2.05 * s + shift_u * 0.5;
    let cone_len = (u_neck - u_base).max(1.0);
    let base_w = (3.65 + 0.32 * charge + 0.38 * recoil) * s;
    let lip_u = (2.55 + 1.05 * charge - 0.50 * recoil) * s;
    let lip_r = (1.35 + 1.05 * charge - 0.25 * recoil) * s;
    let throat_r = (0.55 + 0.68 * charge - 0.22 * recoil).max(0.28) * s;

    let cone_sd = |u: f32, v: f32| -> f32 {
        let vc = v - wobble;
        // Rounded fluted cone: smooth distance to the tapered profile.
        let t = ((u - u_base) / cone_len).clamp(0.0, 1.0);
        let arch = t * (1.0 - t) * 4.0;
        let half = base_w * (1.0 - t * 0.62) + 0.24 * s * arch;
        // Scalloped calcareous base rim.
        let base_cap = u_base - u + (vc.abs() / base_w.max(1.0)).powi(2) * 0.65 * s;
        let top_cap = u - (u_neck + 0.35 * s);
        let side_d = vc.abs() - half;
        let d_cone = side_d.max(base_cap).max(top_cap);
        let d_lip = sd_disc(u, vc, lip_u, 0.0, lip_r);
        smin(d_cone, d_lip, 0.45 * s)
    };

    let u_min = u_base - 1.4 * s;
    let u_max = lip_u + lip_r + 1.4 * s;
    let v_max = base_w + 1.4 * s;
    let inv_u = 1.0 / (3.8 * s);
    let inv_v = 1.0 / (3.6 * s);

    raster_sdf(
        pb,
        &lf,
        (u_min, u_max, -v_max, v_max),
        0.85 * s,
        (1, 1),
        (160, 168, 146),
        cone_sd,
        |u, v, d| {
            let vc = v - wobble;
            // The puckered lip: bright pale-to-amber fleshy torus around a hot throat.
            let dlip = sd_disc(u, vc, lip_u, 0.0, lip_r);
            if dlip <= 0.15 * s {
                let dthroat = sd_disc(u, vc, lip_u + 0.25 * s, 0.0, throat_r);
                if dthroat <= 0.0 {
                    return mix((18, 22, 24), (255, 244, 148), charge);
                }
                let lnu = ((u - lip_u) / lip_r.max(0.5)).clamp(-1.0, 1.0);
                let lnv = (vc / lip_r.max(0.5)).clamp(-1.0, 1.0);
                let llit = lf.lit(lnu, lnv);
                let lip_mid = mix(ARCHER_LIP, ARCHER_CHARGE, charge * 0.68);
                let lip_sh = mix(ARCHER_SHADOW, (186, 112, 48), charge * 0.55);
                let lip_hi = mix((255, 252, 238), (255, 242, 178), charge * 0.7);
                return shade3(lip_sh, lip_mid, lip_hi, 0.50 + 0.55 * llit);
            }

            // Fluted barnacle shell shading.
            let t = ((u - u_base) / cone_len).clamp(0.0, 1.0);
            let half = (base_w * (1.0 - t * 0.62) + 0.24 * s * t * (1.0 - t) * 4.0).max(0.6);
            let nu = ((u - (u_base + u_neck) * 0.5) * inv_u).clamp(-1.0, 1.0);
            let nv = (vc * inv_v).clamp(-1.0, 1.0);
            let dome = (1.0 - (nv * nv * 0.75 + nu * nu * 0.25)).clamp(0.0, 1.0);
            let lit = lf.lit(nu * 0.65, nv);
            let mut lum = 0.40 + 0.36 * dome + 0.58 * lit;
            if d > -0.95 * s && lit > 0.25 {
                lum += 0.24;
            }
            let mut col = shade3(ARCHER_SHADOW, ARCHER_SHELL, ARCHER_HI, lum);

            // Warm oxidized copper crust band around the barnacle base.
            if t < 0.26 {
                col = mix(col, ARCHER_CRUST, 0.52 * (1.0 - t / 0.26));
            }
            // Longitudinal fluted barnacle ribs + calcareous growth seam.
            let flute = ((vc / half) * 2.5).abs().fract();
            if flute < 0.24 {
                col = mix(col, ARCHER_GROOVE, 0.52);
            } else if flute > 0.78 && lit > -0.1 {
                col = mix(col, ARCHER_HI, 0.26);
            }
            if (t - 0.52).abs() < 0.08 {
                col = mix(col, ARCHER_GROOVE, 0.46);
            }
            col
        },
    );

    // 3) Tiny bright copper-amber peepers tucked beneath the front shell ridge.
    let eye_col = if lf.hit > 0.7 { (34, 24, 18) } else { ARCHER_EYE };
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(0.85 * s + shift_u * 0.5, side * 1.15 * s + wobble);
        pb.plot(ep.0, ep.1, eye_col);
    }

    // 4) Live muzzle charge bead + 4-way sparkle when puckered to fire.
    if charge > 0.28 {
        let mp = lf.px(lip_u + 0.45 * s, wobble);
        let orb = mix(ARCHER_CHARGE, (255, 255, 224), charge * 0.75);
        pb.plot(mp.0, mp.1, orb);
        if charge > 0.55 {
            let glow = scale_rgb(orb, 0.48 + 0.45 * charge);
            pb.plot_add(mp.0 + 1, mp.1, glow);
            pb.plot_add(mp.0 - 1, mp.1, glow);
            pb.plot_add(mp.0, mp.1 + 1, glow);
            pb.plot_add(mp.0, mp.1 - 1, glow);
        }
    } else if recoil > 0.25 {
        let puff = lf.px(lip_u + 1.4 * s, wobble);
        blend(pb, puff.0, puff.1, (226, 232, 208), recoil * 0.65);
    }
}

pub(crate) fn draw_grate_archer_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let charge = if e.fire_cd <= 0.0 {
        0.9
    } else {
        archer_charge(e)
    };
    let stride = (e.anim * 0.95).cos() * (1.0 - charge * 0.8);
    let shell = lf.flash(ARCHER_SHELL);
    let hi = lf.flash(ARCHER_HI);
    let dark = lf.flash(ARCHER_SHADOW);
    let foot_c = lf.flash(ARCHER_FOOT);

    // Two alternating copper feet at the cone base.
    for side in [-1.0f32, 1.0] {
        let fp = lf.px((-1.4 + side * stride * 1.4) * s, side * 3.8 * s);
        pb.plot(fp.0, fp.1, foot_c);
    }
    // Crisp barnacle wedge with dark base crust + bright highlight bar.
    thick_line(
        pb,
        lf.px(-3.2 * s, -3.1 * s),
        lf.px(-3.2 * s, 3.1 * s),
        lf.flash(ARCHER_CRUST),
        dark,
    );
    thick_line(pb, lf.px(-1.2 * s, -2.3 * s), lf.px(-1.2 * s, 2.3 * s), shell, dark);
    line(pb, lf.px(0.8 * s, -1.3 * s), lf.px(0.8 * s, 1.3 * s), hi, 1.0);

    // Puckering lip bead (swells and glints on charge).
    let lip = lf.px((2.8 + charge * 0.9) * s, 0.0);
    let lip_col = lf.flash(mix(ARCHER_LIP, ARCHER_CHARGE, charge));
    pb.plot(lip.0, lip.1, lip_col);
    if charge > 0.45 {
        let glow = scale_rgb(ARCHER_CHARGE, 0.55 + 0.4 * charge);
        pb.plot_add(lip.0 + 1, lip.1, glow);
        pb.plot_add(lip.0 - 1, lip.1, glow);
        pb.plot_add(lip.0, lip.1 + 1, glow);
        pb.plot_add(lip.0, lip.1 - 1, glow);
    }
}

// ---- Sump Leech -----------------------------------------------------------

/// The tongue's footprint, from the one geometry that hits
/// (`floor_sewer::tongue_segment`): dashed pale-bone rails framing the exact
/// 24×4 corridor with a creeping fill during the warning, then a glossy wet
/// rose tongue while it is live.
///
/// Drawn once, by the hostile layer after the lighting pass
/// (`World::draw_floor_tell`); the body draws only its own sucker pose.
pub fn draw_leech_tongue(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let Some((a, b, phase, progress)) = floor_sewer::leech_tongue(e) else {
        return;
    };
    let a = a - cam;
    let b = b - cam;
    let along = b - a;
    let len = along.len();
    if len < 1.0 {
        return;
    }
    let dir = along * (1.0 / len);
    let side = Vec2::new(-dir.y, dir.x);
    let half = floor_sewer::LEECH_TONGUE_WIDTH * 0.5;
    let live = phase == floor_sewer::LEECH_PHASE_HOLD;
    let steps = len.ceil() as i32;
    let lead_t = len * progress;

    for i in 0..=steps {
        let t = i as f32;
        let c = a + dir * t;

        if live {
            // Live glossy wet tongue: dark underside edge, fleshy pink core,
            // bright dorsal specular streak, crisp pale upper rail.
            let p_hi = c + side * (-half);
            let p_lo = c + side * half;
            blend(pb, p_hi.x.round() as i32, p_hi.y.round() as i32, LEECH_SUCKER, 0.95);
            blend(pb, p_lo.x.round() as i32, p_lo.y.round() as i32, LEECH_TONGUE_DARK, 0.95);
            for w in -1..=1 {
                let p = c + side * (w as f32 * (half - 1.0));
                let col = match w {
                    -1 => LEECH_TONGUE_HI,
                    0 => LEECH_TONGUE,
                    _ => mix(LEECH_TONGUE, LEECH_TONGUE_DARK, 0.35),
                };
                blend(pb, p.x.round() as i32, p.y.round() as i32, col, 1.0);
            }
        } else {
            // Warning: dashed pale-bone boundary rails across the full reach
            // from the very first tick, plus a creeping inner fill.
            for sgn in [-1.0f32, 1.0] {
                let p = c + side * (sgn * half);
                if i % 3 != 2 {
                    blend(pb, p.x.round() as i32, p.y.round() as i32, LEECH_SUCKER, 0.85);
                }
            }
            if t <= lead_t {
                let at_front = (lead_t - t) <= 1.5;
                for w in -1..=1 {
                    let p = c + side * (w as f32 * (half - 1.0));
                    let col = if at_front {
                        LEECH_TONGUE_HI
                    } else {
                        mix(LEECH_BODY, LEECH_TONGUE, 0.42 + 0.25 * progress)
                    };
                    blend(
                        pb,
                        p.x.round() as i32,
                        p.y.round() as i32,
                        col,
                        if at_front { 0.92 } else { 0.62 },
                    );
                }
            }
        }
    }

    // Tip cap: crisp pale bracket on warn, plump bright sucker-pad tip on hold.
    for w in -2i32..=2 {
        let p = b + side * (w as f32 * half * 0.5);
        let col = if live && w.abs() <= 1 {
            LEECH_TONGUE_HI
        } else {
            LEECH_SUCKER
        };
        blend(pb, p.x.round() as i32, p.y.round() as i32, col, if live { 1.0 } else { 0.88 });
    }
}

/// How far the sucker is open: 0 walking, 0.2 → 1 over the warning, 1 while the
/// tongue is out.
#[inline]
fn leech_open(e: &Enemy) -> f32 {
    match floor_sewer::leech_tongue(e) {
        // Never quite shut once it has committed: the pose changes on the
        // first warned frame.
        Some((_, _, phase, progress)) if phase == floor_sewer::LEECH_PHASE_WARN => {
            0.20 + 0.80 * progress
        }
        Some(_) => 1.0,
        None => 0.0,
    }
}

/// Glossy black comma: a plump wet teardrop head tapering smoothly into a
/// curled, undulating tail. When warning or striking, the tail coils tight and
/// the head flares open into a pale toothed sucker facing the rat.
pub(crate) fn draw_sump_leech(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * 0.2;
    let tongue = floor_sewer::leech_tongue(e);
    let open = leech_open(e);
    let live = matches!(tongue, Some((_, _, phase, _)) if phase == floor_sewer::LEECH_PHASE_HOLD);

    // Continuous undulating comma swim while walking; coiled anchor when firing.
    let swim = (e.anim * 0.95).cos();
    let (wag, surge) = if live {
        (-0.85 * s, 0.45 * s)
    } else if open > 0.0 {
        (-0.70 * open * s, -0.35 * open * s)
    } else {
        (1.15 * swim * s, 0.38 * swim * s)
    };

    // Smooth curved comma spine from head to curling tip.
    let head_u = 1.45 * s + surge;
    let p0 = (head_u - 0.35 * s, -0.15 * s);
    let p1 = (-1.65 * s + surge * 0.4, 1.05 * s + wag * 0.35);
    let p2 = (-4.15 * s, 2.65 * s + wag * 0.72);
    let p3 = (-5.55 * s, 4.55 * s + wag * 1.08);

    // Sucker geometry (stays strictly within u <= 6.2 * s, well inside the
    // body-vs-tongue test boundary of e.radius + 4.0 = 9.0).
    let sucker_u = (3.15 * s + surge).min(3.75 * s);
    let sucker_ru = (1.10 + 0.95 * open) * s;
    let sucker_rv = (1.50 + 1.95 * open) * s;

    let leech_sd = |u: f32, v: f32| -> f32 {
        let head = sd_ellipse(u, v, head_u, -0.25 * s, 3.05 * s, 2.95 * s);
        let t01 = sd_taper(u, v, p0, p1, 2.55 * s, 1.95 * s);
        let t12 = sd_taper(u, v, p1, p2, 1.95 * s, 1.25 * s);
        let t23 = sd_taper(u, v, p2, p3, 1.25 * s, 0.52 * s);
        let tail = smin(smin(t01, t12, 0.65 * s), t23, 0.55 * s);
        let body = smin(head, tail, 0.75 * s);
        if open > 0.0 {
            let sucker = sd_ellipse(u, v, sucker_u, 0.0, sucker_ru, sucker_rv);
            smin(body, sucker, 0.40 * s)
        } else {
            body
        }
    };

    let u_min = -7.1 * s;
    let u_max = if open > 0.0 {
        (sucker_u + sucker_ru + 1.1 * s).min(7.5 * s)
    } else {
        (head_u + 3.05 * s + 1.1 * s).min(6.8 * s)
    };
    let v_min = -4.2 * s;
    let v_max = (p3.1 + 1.7 * s).max(5.5 * s);
    let inv_u = 1.0 / (4.4 * s);
    let inv_v = 1.0 / (3.4 * s);
    let spec_u = head_u + lf.fl * 1.10 * s;
    let spec_v = -0.25 * s + lf.pl * 1.10 * s;

    raster_sdf(
        pb,
        &lf,
        (u_min, u_max, v_min, v_max),
        0.85 * s,
        (1, 1),
        (120, 110, 138),
        leech_sd,
        |u, v, d| {
            // Open pale toothed sucker cup on the front of the head.
            if open > 0.0 {
                let ds = sd_ellipse(u, v, sucker_u, 0.0, sucker_ru, sucker_rv);
                if ds <= 0.12 * s {
                    let throat_ru = (sucker_ru - 0.85 * s).max(0.32 * s);
                    let throat_rv = (sucker_rv - 1.05 * s).max(0.45 * s);
                    let dt = sd_ellipse(u, v, sucker_u + 0.30 * s, 0.0, throat_ru, throat_rv);
                    if dt <= 0.0 {
                        return if live {
                            LEECH_TONGUE
                        } else {
                            mix((38, 16, 26), LEECH_TONGUE, open * 0.72)
                        };
                    }
                    // Subtle radial tooth notches around the pale sucker rim.
                    let snu = (u - sucker_u) / sucker_ru.max(0.5);
                    let snv = v / sucker_rv.max(0.5);
                    let notch = open > 0.45 && (snu.abs() - snv.abs()).abs() < 0.16 && dt > 0.15 * s;
                    if notch {
                        return (74, 48, 56);
                    }
                    let lit = lf.lit(snu.clamp(-1.0, 1.0), snv.clamp(-1.0, 1.0));
                    return shade3((164, 144, 132), LEECH_SUCKER, (255, 250, 238), 0.55 + 0.50 * lit);
                }
            }

            // Glossy wet-black body with strong top-left specular ribbon so the
            // black comma pops against dark wet Sewer masonry.
            let nu = ((u - head_u) * inv_u).clamp(-1.0, 1.0);
            let nv = ((v + 0.2 * s) * inv_v).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.5 + nv * nv * 0.7)).clamp(0.0, 1.0);
            let lit = lf.lit(nu, nv);
            let mut lum = 0.28 + 0.34 * dome + 0.62 * lit;

            // Glossy oil-wet rim catch on the lamp-facing silhouette edge.
            if d > -1.05 * s && lit > 0.32 {
                lum += 0.38;
            }
            let mut col = shade3(LEECH_DEEP, LEECH_BODY, LEECH_SHEEN, lum);

            // Annelid leech segments across the curling tail.
            if u < head_u - 1.2 * s {
                let seg = ((u - v * 0.55) / (1.35 * s)).rem_euclid(1.0);
                if seg < 0.22 {
                    col = mix(col, LEECH_DEEP, 0.58);
                }
            }
            // Crisp specular wet highlight arching on the lamp-facing crown.
            let spec_d = sd_disc(u, v, spec_u, spec_v, 0.95 * s);
            if spec_d <= 0.0 {
                col = mix(col, LEECH_SPEC, 0.82);
            }
            col
        },
    );

    // Guaranteed wet specular glint on the lamp-facing crown (survives low ambient & 16 colours).
    let glint = lf.px(spec_u, spec_v);
    pb.plot(glint.0, glint.1, lf.flash(LEECH_SPEC));
    // Subtle pale mouth slit when walking so the facing end is always clear.
    if open <= 0.0 {
        let mouth = lf.px((head_u + 2.65 * s).min(6.0 * s), -0.2 * s);
        blend(pb, mouth.0, mouth.1, lf.flash(LEECH_SUCKER), 0.72);
    }
}

pub(crate) fn draw_sump_leech_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * 0.2;
    let open = leech_open(e);
    let wag = if open > 0.0 {
        -0.7 * s
    } else {
        (e.anim * 0.95).cos() * 1.1 * s
    };
    let body = lf.flash(LEECH_BODY);
    let deep = lf.flash(LEECH_DEEP);
    let spec = lf.flash(LEECH_SPEC);

    // Curved comma tail stroke + plump dark head + bright wet crown glint.
    let h = lf.px(1.4 * s, -0.2 * s);
    let mid = lf.px(-2.0 * s, 1.3 * s + wag * 0.4);
    let tip = lf.px(-5.2 * s, 4.3 * s + wag);
    thick_line(pb, h, mid, body, deep);
    line(pb, mid, tip, body, 0.95);

    for (dx, dy) in [
        (0, 0),
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ] {
        let c = if dx + dy > 0 { deep } else { body };
        blend(pb, h.0 + dx, h.1 + dy, c, 1.0);
    }
    pb.plot(h.0 - 1, h.1 - 1, spec);

    if open > 0.0 {
        let lip = lf.px(3.8 * s, 0.0);
        let l_up = lf.px(3.5 * s, -1.3 * s);
        let l_dn = lf.px(3.5 * s, 1.3 * s);
        let sucker = lf.flash(LEECH_SUCKER);
        pb.plot(lip.0, lip.1, LEECH_TONGUE);
        pb.plot(l_up.0, l_up.1, sucker);
        pb.plot(l_dn.0, l_dn.1, sucker);
    }
}

// ---- Sluice Toad ----------------------------------------------------------

/// Squat slab wider than it is long, twin raised eye turrets standing proud of
/// the front brow, four heavy splayed legs hauling its broad belly forward.
/// The elite wears a protruding split brass valve collar visible from either
/// side plus a thick double brass shoulder stripe.
pub(crate) fn draw_sluice_toad(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 6.5);
    let elite = elite || e.elite;

    // Continuous belly haul: +1 stretched forward, -1 gathered wide and squat.
    let haul = (e.anim * 0.85).cos();
    let gulp = ((e.contact_cd - 0.22) * 2.6).clamp(0.0, 1.0);
    let back = (-4.6 - 0.55 * haul) * s;
    let front = (4.5 + 0.45 * haul + 0.65 * gulp) * s;
    let half = (6.8 - 0.55 * haul) * s;

    // 1) Four heavy splayed amphibian legs hauling the slab across the masonry.
    let leg_core = lf.flash(TOAD_LEG_CORE);
    let leg_dark = lf.flash(TOAD_LEG_DARK);
    let toe_hi = lf.flash(TOAD_BELLY);
    for side in [-1.0f32, 1.0] {
        // Foreleg plants ahead and drags back during the haul.
        let f_sh = lf.px(front - 2.2 * s, side * (half - 1.2 * s));
        let f_el = lf.px(front - 1.3 * s + haul * 1.05 * s, side * (half + 1.15 * s));
        let f_hd = lf.px(front + 0.1 * s + haul * 1.45 * s, side * (half + 1.45 * s));
        thick_line(pb, f_sh, f_el, leg_core, leg_dark);
        line(pb, f_el, f_hd, leg_core, 0.96);
        pb.plot(f_hd.0, f_hd.1, toe_hi);

        // Muscular folded hind haunch pushes off behind the rear corners.
        let h_hp = lf.px(back + 1.2 * s, side * (half - 1.6 * s));
        let h_kn = lf.px(back - 0.8 * s - haul * 0.85 * s, side * (half + 0.80 * s));
        let h_ft = lf.px(back - 1.6 * s - haul * 1.25 * s, side * (half - 0.9 * s));
        thick_line(pb, h_hp, h_kn, leg_core, leg_dark);
        line(pb, h_kn, h_ft, leg_dark, 0.96);
        pb.plot(h_ft.0, h_ft.1, toe_hi);
    }

    // 2) Sculpted squat slab + eye turret bosses + (elite) protruding split brass valve collar.
    let mid_u = 0.5 * (back + front);
    let hu = 0.5 * (front - back);
    let eye_u = front - 0.15 * s;
    let eye_v = 3.65 * s;
    let collar_u = front - 2.55 * s;
    let collar_reach = half + 1.85 * s;

    let toad_sd = |u: f32, v: f32| -> f32 {
        let av = v.abs();
        let slab = sd_round_box(u, v, mid_u, 0.0, hu, half, 2.25 * s);
        let snout = sd_ellipse(u, v, front - 0.25 * s, 0.0, 1.35 * s, 3.1 * s);
        let turret = sd_disc(u, av, eye_u, eye_v, 2.20 * s);
        let body = smin(smin(slab, snout, 0.75 * s), turret, 0.45 * s);
        if elite {
            // Split valve collar: two protruding brass flange halves behind the
            // eyes, split open at the spine (av >= 1.35*s) and extending proudly
            // past both flanks with bolted valve-flange lugs.
            let band = sd_round_box(
                u,
                av,
                collar_u,
                0.5 * (1.35 * s + collar_reach),
                1.00 * s,
                0.5 * (collar_reach - 1.35 * s),
                0.45 * s,
            );
            let lug = sd_round_box(
                u,
                av,
                collar_u,
                half + 0.75 * s,
                1.35 * s,
                1.10 * s,
                0.35 * s,
            );
            body.min(band.min(lug))
        } else {
            body
        }
    };

    let u_min = back - 1.6 * s;
    let u_max = front + 2.8 * s;
    let v_lim = if elite { collar_reach + 1.4 * s } else { half + 1.4 * s };
    let inv_u = 1.0 / (hu + 1.8 * s);
    let inv_v = 1.0 / (half + 1.0 * s);

    raster_sdf(
        pb,
        &lf,
        (u_min, u_max, -v_lim, v_lim),
        0.95 * s,
        (1, 2),
        (140, 165, 110),
        toad_sd,
        |u, v, d| {
            let av = v.abs();
            let nu = ((u - mid_u) * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);

            // Eye turret housings stand proud at the front brow.
            let d_turret = sd_disc(u, av, eye_u, eye_v, 2.15 * s);
            if d_turret <= 0.0 {
                let d_iris = sd_disc(u, av, eye_u + 0.20 * s, eye_v, 1.48 * s);
                if d_iris <= 0.0 {
                    let iris_base = if elite { TOAD_EYE_ELITE } else { TOAD_EYE };
                    let el = lf.lit(
                        ((u - eye_u) / (1.5 * s)).clamp(-1.0, 1.0),
                        ((v - v.signum() * eye_v) / (1.5 * s)).clamp(-1.0, 1.0),
                    );
                    return shade3((168, 128, 34), iris_base, (255, 252, 206), 0.52 + 0.50 * el);
                }
                return mix(TOAD_SHADOW, TOAD_LEG_DARK, 0.55);
            }

            // Elite split brass valve collar (visible from both sides + split at spine).
            if elite {
                let on_band = (u - collar_u).abs() <= 1.02 * s
                    && (1.32 * s..=collar_reach + 0.1 * s).contains(&av);
                let on_lug = (u - collar_u).abs() <= 1.35 * s
                    && (half - 0.35 * s..=collar_reach + 0.1 * s).contains(&av);
                if on_band || on_lug {
                    // Dark bolt dimple on each protruding flange ear.
                    if sd_disc(u, av, collar_u, half + 0.85 * s, 0.42 * s) <= 0.0 {
                        return (58, 36, 14);
                    }
                    // Dark bevel frame at the central spine split.
                    if av < 1.80 * s {
                        return TOAD_BRASS_SH;
                    }
                    let blum = 0.46 + 0.62 * lit + if on_lug { 0.16 } else { 0.0 };
                    return shade3(TOAD_BRASS_SH, TOAD_BRASS, TOAD_BRASS_HI, blum);
                }
            }

            // Squat mossy toad hide.
            let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.55)).clamp(0.0, 1.0);
            let mut lum = 0.36 + 0.38 * dome + 0.62 * lit;
            if d > -1.05 * s && lit > 0.22 {
                lum += 0.24;
            }
            let mut col = shade3(TOAD_SHADOW, TOAD_HIDE, TOAD_HI, lum);

            // Wide horizontal mouth slit between the turrets (cracks open on gulp).
            if u > front - 0.55 * s && av < 2.15 * s {
                return if gulp > 0.12 {
                    mix((68, 34, 36), (206, 128, 124), gulp)
                } else {
                    (24, 34, 24)
                };
            }

            // Elite thick double brass shoulder stripe across the broad back.
            if elite {
                let s1 = (u - (collar_u - 2.05 * s)).abs();
                let s2 = (u - (collar_u - 3.85 * s)).abs();
                let in_s1 = s1 <= 0.62 * s && av <= half - 0.85 * s;
                let in_s2 = s2 <= 0.62 * s && av <= half - 1.25 * s;
                if in_s1 || in_s2 {
                    return shade3(TOAD_BRASS_SH, TOAD_BRASS, TOAD_BRASS_HI, 0.48 + 0.58 * lit);
                }
                if (s1 <= 0.92 * s || s2 <= 0.92 * s) && av <= half - 0.85 * s {
                    col = mix(col, TOAD_LEG_DARK, 0.45);
                }
            }

            // Pale belly / dorsal ridge down the spine with a subtle central furrow.
            if av <= 1.45 * s && (back + 0.8 * s..front - 1.8 * s).contains(&u) {
                let ridge = shade3(TOAD_HIDE, TOAD_BELLY, TOAD_HI, 0.48 + 0.48 * lit);
                col = if av < 0.32 * s {
                    mix(ridge, TOAD_SHADOW, 0.42)
                } else {
                    ridge
                };
            }

            // Sculpted dorsal warts (dark rim + pale bump highlight).
            let wart_hit = sd_disc(u, av, mid_u - 1.3 * s, 3.8 * s, 0.88 * s)
                .min(sd_disc(u, av, mid_u + 0.6 * s, 5.1 * s, 0.78 * s))
                .min(sd_disc(u, av, mid_u - 2.5 * s, 2.0 * s, 0.70 * s));
            if wart_hit <= 0.0 {
                col = if lit > 0.1 && wart_hit < -0.25 * s {
                    TOAD_WART_HI
                } else {
                    TOAD_WART
                };
            }

            col
        },
    );

    // 3) Crisp horizontal amphibian pupils + pure-white wet eye glints (survive low ambient).
    let pupil_col = if lf.hit > 0.7 { (220, 200, 140) } else { (16, 18, 12) };
    for side in [-1.0f32, 1.0] {
        let p0 = lf.px(eye_u + 0.55 * s, side * eye_v);
        let p1 = lf.px(eye_u + 0.45 * s, side * (eye_v + 0.55 * s));
        pb.plot(p0.0, p0.1, pupil_col);
        blend(pb, p1.0, p1.1, pupil_col, 0.85);
        // Lamp-side wet specular glint on each eye turret.
        let glint = lf.px(eye_u + lf.fl * 0.55 * s, side * eye_v + lf.pl * 0.55 * s);
        pb.plot(glint.0, glint.1, (255, 255, 242));
    }
}

pub(crate) fn draw_sluice_toad_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 6.5);
    let elite = elite || e.elite;
    let haul = (e.anim * 0.85).cos();
    let back = (-4.5 - 0.45 * haul) * s;
    let front = (4.4 + 0.40 * haul) * s;
    let half = (6.6 - 0.45 * haul) * s;
    let mid_u = 0.5 * (back + front);
    let hu = 0.5 * (front - back);
    let collar_u = front - 2.4 * s;
    let collar_reach = half + 1.6 * s;

    // Four stubby corner foot pips.
    let leg_c = lf.flash(TOAD_LEG_CORE);
    for side in [-1.0f32, 1.0] {
        let f = lf.px(front - 0.6 * s + haul * s, side * (half + 1.0 * s));
        let h = lf.px(back - 0.8 * s - haul * s, side * (half - 0.5 * s));
        pb.plot(f.0, f.1, leg_c);
        pb.plot(h.0, h.1, leg_c);
    }

    let u_min = back - 1.2 * s;
    let u_max = front + 2.4 * s;
    let v_lim = if elite { collar_reach + 1.0 * s } else { half + 1.0 * s };
    let eye_u = front - 0.1 * s;
    let eye_v = 3.6 * s;

    raster_sdf(
        pb,
        &lf,
        (u_min, u_max, -v_lim, v_lim),
        0.85 * s,
        (1, 1),
        (140, 165, 110),
        |u, v| {
            let av = v.abs();
            let slab = sd_round_box(u, v, mid_u, 0.0, hu, half, 1.8 * s);
            let eye = sd_disc(u, av, eye_u, eye_v, 1.85 * s);
            let base = slab.min(eye);
            if elite {
                let collar = sd_round_box(
                    u,
                    av,
                    collar_u,
                    0.5 * (1.3 * s + collar_reach),
                    0.95 * s,
                    0.5 * (collar_reach - 1.3 * s),
                    0.35 * s,
                );
                base.min(collar)
            } else {
                base
            }
        },
        |u, v, _| {
            let av = v.abs();
            if sd_disc(u, av, eye_u, eye_v, 1.45 * s) <= 0.0 {
                return if elite { TOAD_EYE_ELITE } else { TOAD_EYE };
            }
            if elite {
                let on_collar = (u - collar_u).abs() <= 0.95 * s && av >= 1.3 * s;
                let on_stripe = ((u - (collar_u - 2.0 * s)).abs() <= 0.60 * s
                    || (u - (collar_u - 3.7 * s)).abs() <= 0.60 * s)
                    && av <= half - 0.8 * s;
                if on_collar || on_stripe {
                    return TOAD_BRASS;
                }
            }
            let lit = lf.lit(
                ((u - mid_u) / (hu + 1.0)).clamp(-1.0, 1.0),
                (v / (half + 1.0)).clamp(-1.0, 1.0),
            );
            if av <= 1.2 * s && u < front - 1.8 * s {
                return TOAD_BELLY;
            }
            shade3(TOAD_SHADOW, TOAD_HIDE, TOAD_HI, 0.44 + 0.54 * lit)
        },
    );

    // Pupils on both eye turrets so far LOD keeps the toad's stare.
    for side in [-1.0f32, 1.0] {
        let p = lf.px(eye_u + 0.45 * s, side * eye_v);
        pb.plot(p.0, p.1, (16, 18, 12));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const BG: Rgb = (40, 44, 48);
    const DARK_SEWER_BG: Rgb = (22, 34, 30);

    fn buffer_with(bg: Rgb) -> PixelBuffer {
        let mut pb = PixelBuffer::new(64, 64);
        for y in 0..pb.h as i32 {
            for x in 0..pb.w as i32 {
                pb.plot(x, y, bg);
            }
        }
        pb
    }

    fn buffer() -> PixelBuffer {
        buffer_with(BG)
    }

    fn body(kind: EnemyKind) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(11);
        let mut e = Enemy::spawn(kind, Vec2::new(32.0, 32.0), 1.0, &mut rng);
        e.anim = 0.0;
        e
    }

    /// Draw through the public entry point, so the test also proves the
    /// bespoke table routes the kind here.
    fn drawn(e: &Enemy) -> PixelBuffer {
        let mut pb = buffer();
        crate::game::art_enemy::draw_enemy(&mut pb, e, Vec2::ZERO, (200, 120, 60));
        pb
    }

    fn painted_over(pb: &PixelBuffer, bg: Rgb) -> Vec<(usize, usize, Rgb)> {
        let mut out = Vec::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                if c != bg {
                    out.push((x, y, c));
                }
            }
        }
        out
    }

    fn painted(pb: &PixelBuffer) -> Vec<(usize, usize, Rgb)> {
        painted_over(pb, BG)
    }

    fn lum(c: Rgb) -> u32 {
        c.0 as u32 + c.1 as u32 + c.2 as u32
    }

    const KINDS: [EnemyKind; 4] = [
        EnemyKind::PipeEel,
        EnemyKind::GrateArcher,
        EnemyKind::SumpLeech,
        EnemyKind::SluiceToad,
    ];

    #[test]
    fn every_body_has_a_light_and_a_dark_edge() {
        for kind in KINDS {
            let px = painted(&drawn(&body(kind)));
            assert!(px.len() >= 12, "{kind:?} paints a real body ({} px)", px.len());
            let brightest = px.iter().map(|p| lum(p.2)).max().unwrap();
            let darkest = px.iter().map(|p| lum(p.2)).min().unwrap();
            assert!(brightest > lum(BG) + 60, "{kind:?} has no light edge");
            assert!(darkest + 30 < lum(BG), "{kind:?} has no dark edge");
        }
    }

    #[test]
    fn the_two_walk_frames_differ() {
        for kind in KINDS {
            let mut a = body(kind);
            let mut b = body(kind);
            a.anim = 0.5;
            b.anim = 2.5;
            assert!(
                painted(&drawn(&a)) != painted(&drawn(&b)),
                "{kind:?} needs two walk frames"
            );
        }
    }

    #[test]
    fn the_four_silhouettes_are_distinct() {
        let masks: Vec<Vec<(usize, usize)>> = KINDS
            .iter()
            .map(|&k| painted(&drawn(&body(k))).into_iter().map(|p| (p.0, p.1)).collect())
            .collect();
        for i in 0..masks.len() {
            for j in i + 1..masks.len() {
                assert_ne!(masks[i], masks[j], "{:?} and {:?}", KINDS[i], KINDS[j]);
            }
        }
    }

    #[test]
    fn the_far_lod_bodies_paint_and_stay_small() {
        for kind in KINDS {
            let mut pb = buffer();
            crate::game::art_enemy::draw_enemy_lod(&mut pb, &body(kind), Vec2::ZERO, (200, 120, 60));
            let n = painted(&pb).len();
            assert!(n >= 4, "{kind:?} far LOD is visible ({n} px)");
        }
    }

    #[test]
    fn the_far_lod_silhouettes_are_distinct() {
        let masks: Vec<Vec<(usize, usize)>> = KINDS
            .iter()
            .map(|&k| {
                let mut pb = buffer();
                crate::game::art_enemy::draw_enemy_lod(&mut pb, &body(k), Vec2::ZERO, (200, 120, 60));
                painted(&pb).into_iter().map(|p| (p.0, p.1)).collect()
            })
            .collect();
        for i in 0..masks.len() {
            for j in i + 1..masks.len() {
                assert_ne!(masks[i], masks[j], "LOD {:?} and {:?}", KINDS[i], KINDS[j]);
            }
        }
    }

    #[test]
    fn the_archer_lip_swells_before_the_shot() {
        let mut calm = body(EnemyKind::GrateArcher);
        calm.fire_cd = 2.0;
        let mut ready = body(EnemyKind::GrateArcher);
        ready.fire_cd = 0.05;
        assert_eq!(archer_charge(&calm), 0.0);
        assert!(archer_charge(&ready) > 0.9);
        assert!(painted(&drawn(&calm)) != painted(&drawn(&ready)));
    }

    #[test]
    fn the_archer_recoils_after_a_shot() {
        let mut calm = body(EnemyKind::GrateArcher);
        calm.fire_cd = 1.6;
        let mut recoiling = body(EnemyKind::GrateArcher);
        recoiling.fire_cd = 2.15;
        assert!(painted(&drawn(&calm)) != painted(&drawn(&recoiling)));
    }

    #[test]
    fn the_eel_and_toad_have_contact_attack_poses() {
        for kind in [EnemyKind::PipeEel, EnemyKind::SluiceToad] {
            let mut idle = body(kind);
            idle.contact_cd = 0.0;
            let mut biting = body(kind);
            biting.contact_cd = 0.55;
            assert!(
                painted(&drawn(&idle)) != painted(&drawn(&biting)),
                "{kind:?} should shift pose on contact"
            );
        }
    }

    #[test]
    fn the_elite_toad_wears_a_wider_split_valve_collar() {
        let normal = body(EnemyKind::SluiceToad);
        let mut elite = body(EnemyKind::SluiceToad);
        elite.elite = true;
        let n_px = painted(&drawn(&normal));
        let e_px = painted(&drawn(&elite));
        assert!(n_px != e_px, "elite toad must look distinct");
        // Facing +x, flanks run along y: the protruding split valve collar
        // makes the elite silhouette wider on both sides.
        let n_span = n_px.iter().map(|p| p.1).max().unwrap() - n_px.iter().map(|p| p.1).min().unwrap();
        let e_span = e_px.iter().map(|p| p.1).max().unwrap() - e_px.iter().map(|p| p.1).min().unwrap();
        assert!(
            e_span > n_span,
            "split valve collar should protrude past both flanks (elite {e_span} vs normal {n_span})"
        );
        // Far LOD also shows the elite attachment.
        let mut pb_n = buffer();
        let mut pb_e = buffer();
        crate::game::art_enemy::draw_enemy_lod(&mut pb_n, &normal, Vec2::ZERO, (200, 120, 60));
        crate::game::art_enemy::draw_enemy_lod(&mut pb_e, &elite, Vec2::ZERO, (200, 120, 60));
        assert!(painted(&pb_n) != painted(&pb_e), "far-LOD elite toad must show collar");
    }

    #[test]
    fn every_body_survives_dark_sewer_ambient() {
        for kind in KINDS {
            let mut pb = buffer_with(DARK_SEWER_BG);
            crate::game::art_enemy::draw_enemy(&mut pb, &body(kind), Vec2::ZERO, (200, 120, 60));
            let px = painted_over(&pb, DARK_SEWER_BG);
            let brightest = px.iter().map(|p| lum(p.2)).max().unwrap_or(0);
            assert!(
                brightest > 450,
                "{kind:?} needs a bright rim/eye glint for low ambient (max lum {brightest})"
            );
        }
    }

    #[test]
    fn every_body_stays_inside_its_cull_reach() {
        let cases = [
            (EnemyKind::PipeEel, false, 14.0_f32),
            (EnemyKind::GrateArcher, false, 9.6_f32),
            (EnemyKind::SumpLeech, false, 12.0_f32),
            (EnemyKind::SluiceToad, false, 13.7_f32),
            (EnemyKind::SluiceToad, true, 13.7_f32),
        ];
        for (kind, is_elite, max_r) in cases {
            let mut e = body(kind);
            e.elite = is_elite;
            if kind == EnemyKind::GrateArcher {
                e.fire_cd = 0.05;
            }
            let px = painted(&drawn(&e));
            for (x, y, _) in px {
                let d = Vec2::new(x as f32, y as f32).dist(e.pos);
                assert!(
                    d <= max_r,
                    "{kind:?} (elite={is_elite}) painted at ({x},{y}), dist {d:.2} > cull_reach {max_r}"
                );
            }
        }
    }

    #[test]
    fn the_leech_tell_draws_the_footprint_that_hits() {
        let mut e = body(EnemyKind::SumpLeech);
        e.pos = Vec2::new(16.0, 16.0);
        e.facing = Vec2::new(1.0, 0.0);
        let walking = painted(&drawn(&e));

        e.w4.phase = floor_sewer::LEECH_PHASE_WARN;
        e.w4.t = floor_sewer::LEECH_WARN_SECS;
        e.w4.dir = Vec2::new(1.0, 0.0);
        let pose = drawn(&e);
        assert!(painted(&pose) != walking, "the warning changes the body's pose");
        // The body alone never paints the footprint (the hostile layer does,
        // once): nothing of it reaches past the sucker.
        assert!(
            painted(&pose).iter().all(|p| p.0 as f32 <= e.pos.x + e.radius + 4.0),
            "the body draw must not paint the tongue"
        );
        // What the hostile layer adds on top.
        let mut warned = pose;
        draw_leech_tongue(&mut warned, &e, Vec2::ZERO);

        // The outline reaches the tongue's tip on the very first warned frame,
        // and every painted footprint pixel lies inside the hit geometry
        // (within a pixel of rounding).
        let (_, tip) = floor_sewer::tongue_segment(e.pos, e.radius, e.w4.dir);
        let near_tip = painted(&warned)
            .iter()
            .any(|p| (p.0 as f32 - tip.x).abs() <= 1.5 && (p.1 as f32 - tip.y).abs() <= 3.0);
        assert!(near_tip, "the footprint shows the full reach");
        for (x, y, _) in painted(&warned) {
            let at = Vec2::new(x as f32, y as f32);
            if at.x > e.pos.x + e.radius + 4.0 {
                assert!(
                    floor_sewer::tongue_touches(e.pos, e.radius, e.w4.dir, at, 1.5),
                    "footprint pixel ({x},{y}) is outside the tongue"
                );
            }
        }
    }
}
