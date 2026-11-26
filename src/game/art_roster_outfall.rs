//! The Outfall's eight ordinary enemies (`OUTFALL_PLAN.md` §3): neon abyssal
//! seabed creatures against dark wet sand and slate-blue shallows.
//!
//! | kind | slot | silhouette & motion |
//! |---|---|---|
//! | Sand Hopper | A | curved sand-ochre segmented amphipod with swept antennae and springing hind legs |
//! | Glass Shrimp | B | translucent ice-cyan swimming needle body, fanned uropods, whiskers, and two bright stalked eyes |
//! | Conger | C | sinuous dark steel-indigo predator eel with silver belly edge, dorsal ribbon crest, amber eyes, turn lag, and toothed contact bite |
//! | Skater | I | obsidian-slate water-strider with bright silver-cyan wax back and four wide skating legs with foam dimples (tucked plod on dry sand; crouched/snapping on 22×4 splash tell) |
//! | Pistol Shrimp | R | warm coral-vermilion asymmetrical snapping shrimp whose giant right claw cocks open and charges with cyan cavitation plasma before firing a bubble pellet |
//! | Lampfish | II | dark bulbous abyssal fish with pale needle underbite teeth, golden eyes, and a curved dorsal stalk reaching 16 px ahead to its bright cyan bulb (3 Hz flicker on 0.4 s ring + 18 px lunge tell) |
//! | Grouper | H | massive mottled slate-olive wreckfish with broad rowing paddles and cavernous jaw; elite wears a protruding Y-forked gold/cyan split-stalk crown and twin forked gold shoulder stripes |
//! | Fry School | III | one living school of tiny silver-teal minnows: loose coordinated cruise across r=12 in the dark, compact swirling gold-flashing baitball at r=6 when bunched in light |
//!
//! Every body draw keeps `art_enemy`'s `BodyDraw` signature and is registered
//! in `art_enemy::bespoke_body` and `bespoke_lod`. Each builds its own local
//! screen frame from `&Enemy` and `cam` (`LIGHT = (-0.55, -0.83)`, `FLASH_MAX`,
//! dark silhouette rim, down-right drop shadow), keeping zero heap allocations.
//!
//! ## Public draws for the hostile layer
//!
//! * [`draw_tell`]`(pb, e, cam, accent)`: reads
//!   `floor_outfall::outfall_enemy_tell(e)` and `floor_outfall::lampfish_bulb(e)`
//!   and draws Skater's 22×4 splash corridor or Lampfish's 3 Hz bulb ring +
//!   locked 18 px lunge sweep after the lighting pass.
//! * [`draw_bubble_pellet`]`(pb, b, cam)`: rasterises Pistol Shrimp's tagged
//!   cavitation bubble shot from the ordinary `&Bullet` view on the hostile
//!   bullet pass.
//!
//! Render-only: nothing here mutates simulation state or draws rng.

#![allow(clippy::disallowed_methods)] // render-only: platform maths is fine

use std::f32::consts::TAU;

use crate::game::art_enemy::Frame;
use crate::game::bullet::Bullet;
use crate::game::ecs::systems::floor_outfall;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// Direction *toward* the light in screen space (up-left), matching `art_enemy`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };
/// `art_enemy::FLASH_MAX`: the hit flash's full length.
const FLASH_MAX: f32 = 0.12;

/// Default Lampfish stalk reach in world pixels (matches `OUTFALL_PLAN.md` §3)
/// used as a fallback if `floor_outfall::lampfish_bulb` returns `None`.
const LAMPFISH_STALK_REACH: f32 = 16.0;
/// Skater splash tell duration (seconds, `OUTFALL_PLAN.md` §3).
/// Lampfish warning ring duration (seconds, `OUTFALL_PLAN.md` §3).
/// Lampfish lunge duration (seconds, `OUTFALL_PLAN.md` §3).
/// Pistol Shrimp charge window before firing (seconds; 2.2 s cadence).
const PISTOL_CHARGE_SECS: f32 = 0.55;
/// Fry School radius threshold separating bunched (r=6) from dark (r=12).
const FRY_BUNCHED_RADIUS: f32 = 8.5;

// ---- Palettes (Outfall: bioluminescent cyan, warm coral/gold accents & dark seabed) ---

// 1. Sand Hopper (Slot A, r=2.5)
const HOPPER_SHADOW: Rgb = (68, 54, 38);
const HOPPER_SHELL: Rgb = (178, 148, 102);
const HOPPER_HI: Rgb = (242, 224, 178);
const HOPPER_BAND: Rgb = (88, 68, 46);
const HOPPER_LEG: Rgb = (128, 104, 72);
const HOPPER_EYE: Rgb = (255, 212, 104);

// 2. Glass Shrimp (Slot B, r=2.0)
const GLASS_RIM: Rgb = (54, 118, 142);
const GLASS_CORE: Rgb = (146, 224, 240);
const GLASS_HI: Rgb = (224, 252, 255);
const GLASS_EYE_CORE: Rgb = (255, 244, 162);
const GLASS_EYE_GLINT: Rgb = (255, 255, 240);

// 3. Conger (Slot C, r=4.0)
const CONGER_DEEP: Rgb = (26, 44, 62);
const CONGER_MID: Rgb = (66, 108, 132);
const CONGER_HI: Rgb = (152, 202, 216);
const CONGER_BELLY: Rgb = (206, 228, 232);
const CONGER_FIN: Rgb = (42, 74, 96);
const CONGER_TOOTH: Rgb = (238, 248, 250);
const CONGER_EYE: Rgb = (248, 208, 76);

// 4. Skater (Slot I, r=3.0)
const SKATER_DARK: Rgb = (24, 36, 52);
const SKATER_MID: Rgb = (56, 84, 112);
const SKATER_SHEEN: Rgb = (198, 240, 252);
const SKATER_LEG: Rgb = (96, 142, 168);
const SKATER_LEG_HI: Rgb = (188, 236, 248);
const SKATER_FOAM: Rgb = (226, 252, 255);
const SKATER_CYAN: Rgb = (84, 228, 248);
const SKATER_EYE: Rgb = (255, 196, 92);

// 5. Pistol Shrimp (Slot R, r=3.0)
const PISTOL_SHADOW: Rgb = (96, 36, 28);
const PISTOL_SHELL: Rgb = (212, 96, 66);
const PISTOL_HI: Rgb = (252, 174, 128);
const PISTOL_JOINT: Rgb = (246, 224, 190);
const PISTOL_CLAW_DARK: Rgb = (128, 42, 30);
const PISTOL_CLAW: Rgb = (236, 114, 76);
const PISTOL_CHARGE: Rgb = (156, 248, 255);
const PISTOL_CORE: Rgb = (248, 255, 255);
const PISTOL_EYE: Rgb = (255, 236, 126);

// 6. Lampfish (Slot II, r=3.0)
const LAMP_DEEP: Rgb = (20, 30, 48);
const LAMP_MID: Rgb = (48, 72, 104);
const LAMP_HI: Rgb = (120, 164, 196);
const LAMP_STALK: Rgb = (36, 56, 78);
const LAMP_TOOTH: Rgb = (216, 246, 252);
const LAMP_EYE: Rgb = (248, 212, 84);
const LAMP_BULB_GLOW: Rgb = (64, 218, 242);
const LAMP_BULB: Rgb = (132, 250, 255);
const LAMP_BULB_CORE: Rgb = (244, 255, 255);

// 7. Grouper (Slot H, r=6.5) + Elite split-stalk stripe
const GROUPER_DEEP: Rgb = (26, 40, 46);
const GROUPER_MID: Rgb = (64, 94, 100);
const GROUPER_HI: Rgb = (146, 186, 182);
const GROUPER_MOTTLE: Rgb = (182, 216, 206);
const GROUPER_LIP: Rgb = (196, 218, 208);
const GROUPER_FIN: Rgb = (42, 66, 74);
const GROUPER_EYE: Rgb = (246, 204, 78);
const GROUPER_ELITE_SH: Rgb = (126, 86, 24);
const GROUPER_ELITE_GOLD: Rgb = (236, 186, 68);
const GROUPER_ELITE_HI: Rgb = (255, 246, 174);
const GROUPER_ELITE_CYAN: Rgb = (120, 246, 255);

// 8. Fry School (Slot III, r=12 dark / r=6 bunched)
const FRY_DEEP: Rgb = (28, 56, 78);
const FRY_MID: Rgb = (94, 166, 196);
const FRY_SILVER: Rgb = (214, 244, 252);
const FRY_GLINT: Rgb = (248, 255, 255);
const FRY_VULN_MID: Rgb = (196, 224, 198);
const FRY_VULN_HI: Rgb = (255, 248, 192);
const FRY_VULN_RING: Rgb = (255, 218, 124);

// Hostile tell colours (self-lit post-lighting, high contrast at zero FX)
const TELL_CYAN: Rgb = (104, 240, 255);
const TELL_WHITE: Rgb = (244, 255, 255);
const TELL_WARN: Rgb = (255, 218, 126);
const TELL_HOT: Rgb = (255, 118, 96);

// ---- Local frame & pixel helpers ------------------------------------------

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

    fn of_lifted(e: &Enemy, cam: Vec2, lift: f32) -> Local {
        let mut lf = Self::of(e, cam);
        lf.pos.y -= lift;
        lf
    }

    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn local_vec(&self, screen: Vec2) -> (f32, f32) {
        let dx = screen.x - self.pos.x;
        let dy = screen.y - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    #[inline]
    fn flash(&self, c: Rgb) -> Rgb {
        if self.flash_str <= 0.0 {
            c
        } else {
            mix(c, self.flash_col, self.flash_str)
        }
    }

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

#[allow(clippy::too_many_arguments)]
fn raster_sdf_alpha(
    pb: &mut PixelBuffer,
    lf: &Local,
    bounds: (f32, f32, f32, f32),
    rim_lim: f32,
    shadow_off: (i32, i32),
    rim_tint: Rgb,
    body_alpha: f32,
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
    let rim_dark = if body_alpha > 0.8 { 0.66 } else { 0.45 };
    let sh_dark = if body_alpha > 0.8 { 0.36 } else { 0.22 };

    for y in y0..=y1 {
        let (mut u, mut v) = lf.local(x0, y);
        for x in x0..=x1 {
            let in_box = (u_min..=u_max).contains(&u) && (v_min..=v_max).contains(&v);
            let d = if in_box { sd(u, v) } else { 10.0 };
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0) * body_alpha;

            if cover >= 0.999 {
                pb.plot(x, y, lf.flash(shade(u, v, d)));
            } else {
                let dark = if d < rim_lim {
                    rim_dark
                } else {
                    let su = u + du_sh;
                    let sv = v + dv_sh;
                    if (u_min..=u_max).contains(&su)
                        && (v_min..=v_max).contains(&sv)
                        && sd(su, sv) < sh_lim
                    {
                        sh_dark
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
    raster_sdf_alpha(pb, lf, bounds, rim_lim, shadow_off, rim_tint, 1.0, sd, shade);
}

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

// ---- Pelagic swimmer float & detached seabed shadow -----------------------

/// Whether `kind` is an Outfall pelagic swimmer drawn floating above the seabed.
/// Bottom dwellers (`SandHopper`, `PistolShrimp`, `Skater`, crabs) stay grounded.
#[inline]
pub(crate) fn is_swimmer(kind: EnemyKind) -> bool {
    matches!(
        kind,
        EnemyKind::GlassShrimp
            | EnemyKind::Conger
            | EnemyKind::Lampfish
            | EnemyKind::Grouper
            | EnemyKind::FrySchool
    )
}

/// Deterministic phase offset in `[0, TAU)` derived from an enemy's ID so a
/// group of swimmers bobs smoothly out of phase with one another.
#[inline]
fn float_phase(id: u32) -> f32 {
    let mut n = id.wrapping_mul(2_654_435_761);
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    n ^= n >> 16;
    (n & 0xFFFF) as f32 * (TAU / 65536.0)
}

/// Base float height and current screen-upward lift (world px) of an Outfall
/// swimmer above its true ground position `e.pos`. Render-only.
#[inline]
pub(crate) fn swimmer_lift(e: &Enemy) -> f32 {
    let (base, amp, freq) = match e.kind {
        EnemyKind::GlassShrimp => (4.0, 0.75, 0.72),
        EnemyKind::Conger => (4.8, 0.85, 0.62),
        EnemyKind::Lampfish => (4.8, 0.85, 0.65),
        EnemyKind::Grouper => (5.2, 0.90, 0.48),
        EnemyKind::FrySchool => (4.5, 0.75, 0.68),
        _ => return 0.0,
    };
    base + amp * (e.anim * freq + float_phase(e.id)).sin()
}

/// Soft detached seabed shadow left under a floating swimmer's TRUE ground
/// position (`e.pos - cam`), shrinking and fading slightly as `lift` rises.
/// Stamped with pen `0` so the lighting pass treats it as seabed floor.
fn draw_floating_shadow(
    pb: &mut PixelBuffer,
    ground: Vec2,
    rx: f32,
    ry: f32,
    lift: f32,
    base_lift: f32,
) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let prev_pen = pb.set_pen(0);
    let h_frac = (lift / base_lift.max(1.0)).clamp(0.75, 1.30);
    let r_scale = 1.0 / (0.82 + 0.18 * h_frac);
    let rx_e = (rx * r_scale).max(1.2);
    let ry_e = (ry * r_scale).max(0.9);
    let core = (0.30 / (0.72 + 0.28 * h_frac)).clamp(0.14, 0.34);

    let cx = ground.x.round() as i32;
    let cy = ground.y.round() as i32;
    let px_r = rx_e.ceil() as i32 + 1;
    let py_r = ry_e.ceil() as i32 + 1;
    let x0 = (cx - px_r).max(0);
    let x1 = (cx + px_r).min(pb.w as i32 - 1);
    let y0 = (cy - py_r).max(0);
    let y1 = (cy + py_r).min(pb.h as i32 - 1);
    if x0 <= x1 && y0 <= y1 {
        let inv_rx = 1.0 / rx_e;
        let inv_ry = 1.0 / ry_e;
        for y in y0..=y1 {
            let dy = (y as f32 - ground.y) * inv_ry;
            let dy2 = dy * dy;
            if dy2 >= 1.0 {
                continue;
            }
            for x in x0..=x1 {
                let dx = (x as f32 - ground.x) * inv_rx;
                let r2 = dx * dx + dy2;
                if r2 < 1.0 {
                    let fall = 1.0 - r2;
                    darken(pb, x, y, core * (0.35 + 0.65 * fall));
                }
            }
        }
    }
    pb.set_pen(prev_pen);
}

// ---- Simulation tell contract adapter (single seam with `floor_outfall`) ---

/// Normalized internal rendering view of an active Outfall ordinary enemy tell.
#[derive(Clone, Copy, Debug)]
enum TellView {
    SkaterSplash {
        start: Vec2,
        end: Vec2,
        half_width: f32,
        progress: f32,
        active: bool,
    },
    LampfishLunge {
        start: Vec2,
        end: Vec2,
        bulb: Vec2,
        remaining: f32,
        progress: f32,
        active: bool,
    },
}

/// Read the locked endpoints and width from the simulation footprint.
fn tell_view(e: &Enemy) -> Option<TellView> {
    use crate::game::ecs::systems::boss::HazardShape;
    let tell = floor_outfall::outfall_enemy_tell(e)?;
    let active = tell.phase == floor_outfall::OutfallTellPhase::Strike;
    match tell.shape {
        HazardShape::Rect { center, facing, half, half_perp } => {
            let progress = (1.0 - tell.remaining / floor_outfall::SKATER_TELL_SECS).clamp(0.0, 1.0);
            Some(TellView::SkaterSplash {
                start: center - facing * half,
                end: center + facing * half,
                half_width: half_perp,
                progress,
                active,
            })
        }
        HazardShape::Corridor { from, to, .. } => {
            let duration = if active {
                floor_outfall::LAMPFISH_LUNGE_SECS
            } else {
                floor_outfall::LAMPFISH_TELL_SECS
            };
            let progress = (1.0 - tell.remaining / duration).clamp(0.0, 1.0);
            Some(TellView::LampfishLunge {
                start: from,
                end: to,
                bulb: floor_outfall::lampfish_bulb(e)?,
                remaining: tell.remaining,
                progress,
                active,
            })
        }
        _ => None,
    }
}

/// 3 Hz stepped flicker phase for Lampfish warning illumination (`OUTFALL_PLAN.md` §3).
#[inline]
fn lampfish_flicker_hi(remaining: f32, active: bool) -> bool {
    if active {
        return true;
    }
    let elapsed = (floor_outfall::LAMPFISH_TELL_SECS - remaining).max(0.0);
    (((elapsed * 6.0).floor() as i32) & 1) == 0
}

// ---- 1. Sand Hopper (Slot A, r=2.5) ---------------------------------------

/// Curved wet-sand amphipod with banded back plates, swept hair antennae, and
/// springing hind legs that kick out on each hop stride.
pub(crate) fn draw_sand_hopper(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 2.5);
    let hop = (e.anim * 1.35).sin().max(0.0);
    let stride = (e.anim * 1.35).cos();
    let stretch = 0.65 * hop * s;

    // Swept-back twin antennae.
    let ant_c = lf.flash(HOPPER_HI);
    for side in [-1.0f32, 1.0] {
        let a0 = lf.px(2.4 * s + stretch * 0.4, side * 0.7 * s);
        let a1 = lf.px(4.3 * s + stretch * 0.6, side * (1.8 + 0.3 * stride) * s);
        line(pb, a0, a1, ant_c, 0.85);
    }

    // Kicking hind spring legs + front tarsi.
    let leg_c = lf.flash(HOPPER_LEG);
    let sh_c = lf.flash(HOPPER_SHADOW);
    let hi_c = lf.flash(HOPPER_HI);
    for side in [-1.0f32, 1.0] {
        let hip = lf.px(-1.3 * s, side * 1.3 * s);
        let knee = lf.px((-2.3 - 0.9 * hop) * s, side * (2.6 - 0.4 * hop) * s);
        let foot = lf.px((-3.6 - 1.3 * hop) * s, side * (2.0 + 0.95 * stride * side) * s);
        line(pb, hip, knee, leg_c, 0.95);
        line(pb, knee, foot, sh_c, 0.92);
        pb.plot(foot.0, foot.1, hi_c);

        let f0 = lf.px(0.8 * s, side * 1.4 * s);
        let f1 = lf.px((1.7 + 0.95 * stride * side) * s, side * 2.5 * s);
        line(pb, f0, f1, leg_c, 0.90);
    }

    // Arched banded amphipod carapace.
    let inv_u = 1.0 / (3.2 * s);
    let inv_v = 1.0 / (1.8 * s);
    raster_sdf(
        pb,
        &lf,
        (-4.2 * s, 4.0 * s, -2.6 * s, 2.6 * s),
        0.85 * s,
        (1, 1),
        (200, 178, 128),
        |u, v| {
            let thorax = sd_ellipse(
                u,
                v,
                0.6 * s + stretch * 0.3,
                0.0,
                (2.3 + 0.3 * hop) * s,
                1.75 * s,
            );
            let tail = sd_taper(
                u,
                v,
                (-s, 0.0),
                ((-2.8 - 0.5 * hop) * s, 0.25 * stride * s),
                1.35 * s,
                0.55 * s,
            );
            smin(thorax, tail, 0.5 * s)
        },
        |u, v, d| {
            let nu = (u * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);
            let dome = (1.0 - (nu * nu * 0.4 + nv * nv * 0.65)).clamp(0.0, 1.0);
            let mut lum = 0.42 + 0.34 * dome + 0.58 * lit;
            if d > -0.75 * s && lit > 0.22 {
                lum += 0.25;
            }
            let mut col = shade3(HOPPER_SHADOW, HOPPER_SHELL, HOPPER_HI, lum);
            let band = ((u / (1.05 * s)).rem_euclid(1.0) - 0.5).abs();
            if band < 0.14 && (-2.2 * s..2.1 * s).contains(&u) {
                col = mix(col, HOPPER_BAND, 0.55);
            }
            col
        },
    );

    // Crisp amber-glint eyes.
    let eye_c = lf.flash(HOPPER_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(2.1 * s + stretch * 0.3, side * 0.95 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }
}

pub(crate) fn draw_sand_hopper_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 2.5);
    let hop = (e.anim * 1.35).sin().max(0.0);
    let body_c = lf.flash(HOPPER_SHELL);
    let hi_c = lf.flash(HOPPER_HI);
    let sh_c = lf.flash(HOPPER_SHADOW);
    for &(dv, col) in &[(-0.8f32, hi_c), (0.0, body_c), (0.8, sh_c)] {
        line(
            pb,
            lf.px((-2.2 - 0.5 * hop) * s, dv * s),
            lf.px((1.9 + 0.4 * hop) * s, dv * s),
            col,
            0.96,
        );
    }
    let leg_c = lf.flash(HOPPER_LEG);
    let eye_c = lf.flash(HOPPER_EYE);
    for side in [-1.0f32, 1.0] {
        let f = lf.px((-2.4 - 0.9 * hop) * s, side * 2.2 * s);
        pb.plot(f.0, f.1, leg_c);
        let ep = lf.px(2.1 * s, side * 0.85 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }
}

// ---- 2. Glass Shrimp (Slot B, r=2.0) --------------------------------------

/// Translucent swimming pelagic shrimp: thin ice-cyan body, fanned tail
/// uropods, trailing hair whiskers, and two bright stalked eyes.
pub(crate) fn draw_glass_shrimp(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let s = e.radius.max(1.0) * 0.5;
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 2.8 * s, 1.7 * s, lift, 4.0);
    let lf = Local::of_lifted(e, cam, lift);
    let wag = (e.anim * 1.8).sin();
    let wag2 = (e.anim * 1.8).cos();

    // Long delicate swept-back hair whiskers.
    let whisk_c = lf.flash(GLASS_CORE);
    let rim_c = lf.flash(GLASS_RIM);
    for side in [-1.0f32, 1.0] {
        let w0 = lf.px(2.6 * s, side * 0.6 * s);
        let w1 = lf.px(0.2 * s, side * (2.6 + 0.4 * wag) * s);
        let w2 = lf.px(-2.8 * s, side * (3.4 + 0.7 * wag2) * s);
        line(pb, w0, w1, whisk_c, 0.55);
        line(pb, w1, w2, rim_c, 0.42);
    }

    // Fanned translucent tail uropods.
    let tail_u = -3.8 * s;
    let tail_v = 0.75 * wag * s;
    let hi_c = lf.flash(GLASS_HI);
    for side in [-1.0f32, 1.0] {
        let t0 = lf.px(-2.4 * s, tail_v * 0.5);
        let t1 = lf.px(tail_u - 1.2 * s, tail_v + side * (1.4 + 0.35 * wag2) * s);
        line(pb, t0, t1, hi_c, 0.72);
    }

    // Slender translucent needle carapace.
    let inv_u = 1.0 / (3.0 * s);
    let inv_v = 1.0 / (1.2 * s);
    raster_sdf_alpha(
        pb,
        &lf,
        (-5.0 * s, 3.8 * s, -2.5 * s, 2.5 * s),
        0.65 * s,
        (0, 0),
        (150, 225, 240),
        0.72,
        |u, v| {
            let bend_v = tail_v * ((-u) / (3.8 * s)).clamp(0.0, 1.0);
            let head = sd_ellipse(u, v, 1.5 * s, 0.0, 1.75 * s, 1.05 * s);
            let tail = sd_taper(u, v, (0.6 * s, 0.0), (tail_u, bend_v), 0.95 * s, 0.38 * s);
            smin(head, tail, 0.35 * s)
        },
        |u, v, d| {
            let nu = (u * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);
            let edge = (1.0 + d / (0.9 * s)).clamp(0.0, 1.0);
            let lum = 0.38 + 0.45 * edge + 0.48 * lit;
            let mut col = shade3(GLASS_RIM, GLASS_CORE, GLASS_HI, lum);
            if v.abs() < 0.35 * s && (-2.2 * s..1.8 * s).contains(&u) {
                col = mix(col, GLASS_HI, 0.45);
            }
            col
        },
    );

    // Two bright stalked eyes (signature read in dark water).
    let eye_c = lf.flash(GLASS_EYE_CORE);
    let glint = scale_rgb(GLASS_EYE_GLINT, 0.42);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(2.35 * s, side * 1.15 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let gl = lf.px(2.65 * s, side * 1.35 * s);
        pb.plot_add(gl.0, gl.1, glint);
    }
}

pub(crate) fn draw_glass_shrimp_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let s = e.radius.max(1.0) * 0.5;
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 2.4 * s, 1.5 * s, lift, 4.0);
    let lf = Local::of_lifted(e, cam, lift);
    let wag = (e.anim * 1.8).sin() * 0.7 * s;
    line(pb, lf.px(-3.4 * s, wag), lf.px(2.0 * s, 0.0), lf.flash(GLASS_CORE), 0.78);
    let hi_c = lf.flash(GLASS_HI);
    let eye_c = lf.flash(GLASS_EYE_CORE);
    for side in [-1.0f32, 1.0] {
        let tp = lf.px(-4.2 * s, wag + side * 1.2 * s);
        blend(pb, tp.0, tp.1, hi_c, 0.72);
        let ep = lf.px(2.2 * s, side * 1.2 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }
}

// ---- 3. Conger (Slot C, r=4.0) --------------------------------------------

/// Muscular deep-sea predator eel with blunt snapping jaw, steel-indigo hide,
/// pale silver belly edge, undulating dorsal crest, and yellow predator eyes.
pub(crate) fn draw_conger(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let s = e.radius.max(1.0) * 0.25;
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 5.6 * s, 2.2 * s, lift, 4.8);
    let lf = Local::of_lifted(e, cam, lift);
    let phase = e.anim * 1.2;
    let lag = turn_lag(&lf, e.vel);
    let bite = ((e.contact_cd - 0.15) * 2.6).clamp(0.0, 1.0);

    // 5-node sinuous eel spine.
    let p0 = (3.4 * s, 0.0);
    let p1 = (0.4 * s, (0.55 * phase.sin() + lag * 0.45) * s);
    let p2 = (-3.0 * s, (1.25 * (phase - 1.1).sin() + lag * 1.15) * s);
    let p3 = (-6.6 * s, (1.85 * (phase - 2.2).sin() + lag * 2.05) * s);
    let p4 = (-10.2 * s, (2.25 * (phase - 3.3).sin() + lag * 3.0) * s);

    // Pectoral fin stubs behind the gills.
    let fin_c = lf.flash(CONGER_FIN);
    let hi_c = lf.flash(CONGER_HI);
    for side in [-1.0f32, 1.0] {
        let f0 = lf.px(1.6 * s, side * 1.7 * s);
        let f1 = lf.px(0.1 * s, side * (3.1 + 0.35 * phase.cos()) * s);
        let f2 = lf.px(-0.8 * s, side * 2.4 * s);
        line(pb, f0, f1, fin_c, 0.92);
        line(pb, f1, f2, hi_c, 0.72);
    }

    // Dorsal fin ribbon crest trailing behind the spine.
    for (a, b) in [(p1, p2), (p2, p3), (p3, p4)] {
        let da = lf.px(a.0, a.1 - 1.35 * s);
        let db = lf.px(b.0, b.1 - 1.15 * s);
        line(pb, da, db, fin_c, 0.78);
    }

    let inv_u = 1.0 / (7.0 * s);
    let inv_v = 1.0 / (2.5 * s);
    let v_ext = (4.8 + lag.abs() * 3.0) * s;

    raster_sdf(
        pb,
        &lf,
        (-11.6 * s, 7.2 * s, -v_ext, v_ext),
        0.9 * s,
        (0, 0),
        (130, 182, 200),
        |u, v| {
            let s0 = sd_taper(u, v, p0, p1, 2.15 * s, 2.05 * s);
            let s1 = sd_taper(u, v, p1, p2, 2.05 * s, 1.65 * s);
            let s2 = sd_taper(u, v, p2, p3, 1.65 * s, 1.05 * s);
            let s3 = sd_taper(u, v, p3, p4, 1.05 * s, 0.38 * s);
            let snout = sd_ellipse(
                u,
                v,
                (4.35 + 0.4 * bite) * s,
                0.0,
                (1.95 + 0.3 * bite) * s,
                (1.85 + 0.35 * bite) * s,
            );
            let body = smin(smin(s0, s1, 0.55 * s), smin(s2, s3, 0.45 * s), 0.5 * s);
            smin(body, snout, 0.45 * s)
        },
        |u, v, d| {
            let nu = ((u + 2.0 * s) * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);
            let dome = (1.0 - nv * nv * 0.62).clamp(0.0, 1.0);
            let mut lum = 0.36 + 0.36 * dome + 0.58 * lit;
            if d > -0.9 * s && lit > 0.2 {
                lum += 0.24;
            }
            let mut col = shade3(CONGER_DEEP, CONGER_MID, CONGER_HI, lum);
            if d > -0.85 * s && nv > 0.35 {
                col = mix(col, CONGER_BELLY, 0.55);
            }
            if (1.1 * s..2.2 * s).contains(&u)
                && v.abs() > 1.3 * s
                && ((u / (0.55 * s)).rem_euclid(1.0) - 0.5).abs() < 0.2
            {
                col = mix(col, CONGER_DEEP, 0.65);
            }
            if u > (4.6 - 0.3 * bite) * s && v.abs() < (1.1 + 0.5 * bite) * s {
                if bite > 0.15 {
                    let tooth = ((v / (0.65 * s)).rem_euclid(1.0) - 0.5).abs() < 0.22
                        && u > 5.0 * s;
                    return if tooth { CONGER_TOOTH } else { (34, 18, 28) };
                }
                return CONGER_DEEP;
            }
            col
        },
    );

    // Twin amber predator eyes with bright highlights.
    let eye_c = lf.flash(CONGER_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(4.1 * s, side * 1.15 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let gp = lf.px(4.4 * s, side * 0.95 * s);
        pb.plot(gp.0, gp.1, (255, 250, 210));
    }
}

pub(crate) fn draw_conger_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let s = e.radius.max(1.0) * 0.25;
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 5.2 * s, 2.0 * s, lift, 4.8);
    let lf = Local::of_lifted(e, cam, lift);
    let w1 = (e.anim * 1.2).sin() * 1.2 * s;
    let w2 = (e.anim * 1.2 - 1.8).sin() * 1.8 * s;
    thick_line(
        pb,
        lf.px(-8.6 * s, w2),
        lf.px(-2.6 * s, w1),
        lf.flash(CONGER_MID),
        lf.flash(CONGER_DEEP),
    );
    for &(dv, col) in &[
        (-1.0f32, lf.flash(CONGER_HI)),
        (0.0, lf.flash(CONGER_MID)),
        (1.0, lf.flash(CONGER_BELLY)),
    ] {
        line(
            pb,
            lf.px(-2.6 * s, w1 + dv * 0.75 * s),
            lf.px(4.5 * s, dv * 0.9 * s),
            col,
            0.96,
        );
    }
    let eye_c = lf.flash(CONGER_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(3.9 * s, side * 1.15 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }
}

// ---- 4. Skater (Slot I, r=3.0) --------------------------------------------

/// Whether the Skater should show its wide wet-gliding stance (true when moving
/// at water speed, actively telling, or posed stationary in a portrait/test;
/// false while plodding slowly on dry sand at ~12 px/s).
#[inline]
fn skater_wet_stance(e: &Enemy, has_tell: bool) -> bool {
    if has_tell {
        return true;
    }
    let speed_sq = e.vel.len_sq();
    speed_sq < 1.0 || speed_sq >= 20.0 * 20.0
}

/// Marine water-strider: sleek obsidian-slate torpedo body with bright
/// silver-cyan dorsal wax ridge and four wide outstretched skating legs.
pub(crate) fn draw_skater(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let (has_tell, active, prog) = match tell_view(e) {
        Some(TellView::SkaterSplash {
            progress, active, ..
        }) => (true, active, progress),
        _ => (false, false, 0.0),
    };
    let wet = skater_wet_stance(e, has_tell);
    let spread = if wet { 1.0 } else { 0.62 };
    let stroke = (e.anim * if wet { 1.8 } else { 0.9 }).sin();
    let cock = if !has_tell {
        0.0
    } else if active {
        1.4 * s
    } else {
        -1.1 * s * prog
    };

    let leg_c = lf.flash(SKATER_LEG);
    let leg_hi = lf.flash(SKATER_LEG_HI);
    for side in [-1.0f32, 1.0] {
        let f_hip = lf.px(0.9 * s + cock * 0.4, side * 1.0 * s);
        let f_knee = lf.px(
            (2.8 * spread + 0.9 * stroke * side + cock) * s,
            side * (3.8 * spread) * s,
        );
        let f_tip = lf.px(
            (5.2 * spread + 1.2 * stroke * side + cock * 1.2) * s,
            side * (5.6 * spread) * s,
        );
        line(pb, f_hip, f_knee, leg_c, 0.95);
        line(pb, f_knee, f_tip, leg_hi, 0.92);

        let r_hip = lf.px(-s + cock * 0.3, side * 1.0 * s);
        let r_knee = lf.px(
            (-2.8 * spread - 0.8 * stroke * side) * s,
            side * (4.0 * spread) * s,
        );
        let r_tip = lf.px(
            (-5.4 * spread - 1.0 * stroke * side) * s,
            side * (5.2 * spread) * s,
        );
        line(pb, r_hip, r_knee, leg_c, 0.92);
        line(pb, r_knee, r_tip, leg_hi, 0.88);

        if wet {
            pb.plot(f_tip.0, f_tip.1, SKATER_FOAM);
            pb.plot(r_tip.0, r_tip.1, SKATER_CYAN);
            blend(pb, f_tip.0 - 1, f_tip.1, SKATER_CYAN, 0.55);
            blend(pb, f_tip.0 + 1, f_tip.1, SKATER_CYAN, 0.55);
        }
    }

    let inv_u = 1.0 / (4.0 * s);
    let inv_v = 1.0 / (1.3 * s);
    raster_sdf(
        pb,
        &lf,
        (-5.2 * s, 5.8 * s, -2.2 * s, 2.2 * s),
        0.85 * s,
        (1, 1),
        (160, 224, 244),
        |u, v| {
            let uc = u - cock * 0.5;
            let body = sd_ellipse(uc, v, 0.0, 0.0, 3.6 * s, 1.25 * s);
            let head = sd_ellipse(uc, v, 3.0 * s, 0.0, 1.15 * s, 0.95 * s);
            smin(body, head, 0.35 * s)
        },
        |u, v, _| {
            let uc = u - cock * 0.5;
            let nu = (uc * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);
            let lum = 0.36 + 0.34 * (1.0 - nv * nv) + 0.56 * lit;
            let mut col = shade3(SKATER_DARK, SKATER_MID, SKATER_SHEEN, lum);
            if v.abs() < 0.42 * s && (-2.6 * s..2.6 * s).contains(&uc) {
                col = mix(col, SKATER_SHEEN, 0.72);
            }
            if has_tell && !active && uc > 1.8 * s {
                col = mix(col, SKATER_FOAM, 0.4 + 0.5 * prog);
            } else if active && uc > 1.2 * s {
                col = SKATER_FOAM;
            }
            col
        },
    );

    let eye_c = lf.flash(SKATER_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(3.3 * s + cock * 0.5, side * 0.72 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let a0 = lf.px(3.8 * s + cock * 0.5, side * 0.4 * s);
        let a1 = lf.px(5.1 * s + cock * 0.5, side * 1.2 * s);
        line(pb, a0, a1, leg_hi, 0.75);
    }
}

pub(crate) fn draw_skater_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let wet = skater_wet_stance(e, false);
    let spread = if wet { 1.0 } else { 0.65 };
    let leg_hi = lf.flash(SKATER_LEG_HI);
    let leg_c = lf.flash(SKATER_LEG);
    for side in [-1.0f32, 1.0] {
        line(
            pb,
            lf.px(0.5 * s, side * 0.8 * s),
            lf.px(4.2 * spread * s, side * 4.8 * spread * s),
            leg_hi,
            0.9,
        );
        line(
            pb,
            lf.px(-0.8 * s, side * 0.8 * s),
            lf.px(-4.2 * spread * s, side * 4.4 * spread * s),
            leg_c,
            0.85,
        );
    }
    thick_line(
        pb,
        lf.px(-3.2 * s, 0.0),
        lf.px(3.2 * s, 0.0),
        lf.flash(SKATER_SHEEN),
        lf.flash(SKATER_DARK),
    );
}

// ---- 5. Pistol Shrimp (Slot R, r=3.0) + Bubble Pellet ---------------------

#[inline]
fn pistol_charge(e: &Enemy) -> f32 {
    if e.fire_cd <= PISTOL_CHARGE_SECS {
        (1.0 - e.fire_cd.max(0.0) / PISTOL_CHARGE_SECS).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Warm coral-vermilion snapping shrimp with a massive asymmetrical right
/// snapper claw that cocks open and charges with cyan cavitation plasma before
/// firing its bubble shot.
pub(crate) fn draw_pistol_shrimp(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let charge = pistol_charge(e);
    let recoil = if e.fire_cd > 1.90 {
        ((e.fire_cd - 1.90) * (1.0 / 0.30)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let stride = (e.anim * 1.15).sin() * (1.0 - 0.7 * charge);
    let shift_u = -0.8 * recoil * s;

    // Swept coral antennae.
    let joint_c = lf.flash(PISTOL_JOINT);
    for side in [-1.0f32, 1.0] {
        let a0 = lf.px(2.4 * s + shift_u, side * 0.8 * s);
        let a1 = lf.px(4.6 * s + shift_u, side * (2.2 + 0.3 * stride) * s);
        line(pb, a0, a1, joint_c, 0.78);
    }

    // Small walking legs.
    let sh_c = lf.flash(PISTOL_SHADOW);
    let hi_c = lf.flash(PISTOL_HI);
    for side in [-1.0f32, 1.0] {
        for k in [-1.0f32, 0.0] {
            let l0 = lf.px((-0.8 + k * 1.1) * s + shift_u, side * 1.4 * s);
            let l1 = lf.px((-1.2 + k * 1.1 + 0.95 * stride * side) * s + shift_u, side * 2.8 * s);
            line(pb, l0, l1, sh_c, 0.90);
            pb.plot(l1.0, l1.1, hi_c);
        }
    }

    // Small left pincer (v < 0).
    thick_line(
        pb,
        lf.px(1.6 * s + shift_u, -1.5 * s),
        lf.px(3.3 * s + shift_u, -2.3 * s),
        lf.flash(PISTOL_SHELL),
        sh_c,
    );

    // Tail fan.
    for tv in [-1.4f32, 0.0, 1.4] {
        line(
            pb,
            lf.px(-2.6 * s + shift_u, 0.0),
            lf.px(-4.4 * s + shift_u, tv * s),
            hi_c,
            0.88,
        );
    }

    let claw_u = (3.4 + 0.5 * charge - 0.6 * recoil) * s + shift_u;
    let claw_v = (2.15 + 0.25 * charge) * s;
    let hammer_open = 0.95 * charge * s;

    let inv_u = 1.0 / (3.8 * s);
    let inv_v = 1.0 / (3.0 * s);
    raster_sdf(
        pb,
        &lf,
        (-4.5 * s, 7.2 * s, -3.2 * s, 5.2 * s),
        0.85 * s,
        (1, 1),
        (242, 148, 112),
        |u, v| {
            let uc = u - shift_u;
            let body = sd_ellipse(uc, v, -0.3 * s, 0.0, 2.8 * s, 1.65 * s);
            let head = sd_ellipse(uc, v, 2.1 * s, 0.0, 1.35 * s, 1.3 * s);
            let arm = sd_taper(
                u,
                v,
                (1.4 * s + shift_u, 1.1 * s),
                (claw_u - 0.6 * s, claw_v),
                1.0 * s,
                1.45 * s,
            );
            let palm = sd_ellipse(u, v, claw_u, claw_v, 1.95 * s, 1.48 * s);
            let hammer = sd_taper(
                u,
                v,
                (claw_u + 0.6 * s, claw_v + 0.55 * s),
                (claw_u + 2.3 * s, claw_v + (0.65 + hammer_open) * s),
                0.78 * s,
                0.42 * s,
            );
            let anvil = sd_taper(
                u,
                v,
                (claw_u + 0.7 * s, claw_v - 0.45 * s),
                (claw_u + 2.2 * s, claw_v - 0.55 * s),
                0.70 * s,
                0.36 * s,
            );
            let claw = smin(smin(arm, palm, 0.4 * s), smin(hammer, anvil, 0.25 * s), 0.3 * s);
            smin(smin(body, head, 0.45 * s), claw, 0.35 * s)
        },
        |u, v, d| {
            let uc = u - shift_u;
            let nu = (uc * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);
            let dome = (1.0 - (nu * nu * 0.35 + nv * nv * 0.45)).clamp(0.0, 1.0);
            let mut lum = 0.40 + 0.34 * dome + 0.58 * lit;
            if d > -0.8 * s && lit > 0.22 {
                lum += 0.25;
            }
            if v > 0.9 * s && u > 1.5 * s + shift_u {
                let mut col = shade3(PISTOL_CLAW_DARK, PISTOL_CLAW, PISTOL_HI, lum + 0.08);
                if charge > 0.15 && u > claw_u + 0.9 * s {
                    col = mix(col, PISTOL_CHARGE, charge * 0.75);
                }
                col
            } else {
                let mut col = shade3(PISTOL_SHADOW, PISTOL_SHELL, PISTOL_HI, lum);
                if (-2.4 * s..1.4 * s).contains(&uc)
                    && ((uc / (1.15 * s)).rem_euclid(1.0) - 0.5).abs() < 0.12
                {
                    col = mix(col, PISTOL_JOINT, 0.55);
                }
                col
            }
        },
    );

    let eye_c = lf.flash(PISTOL_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(2.6 * s + shift_u, side * 0.85 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }

    // Cavitation plasma spark inside the cocked snapper jaws.
    if charge > 0.25 {
        let mp = lf.px(claw_u + 2.0 * s, claw_v + 0.15 * hammer_open);
        let orb = mix(PISTOL_CHARGE, PISTOL_CORE, charge * 0.75);
        pb.plot(mp.0, mp.1, orb);
        if charge > 0.55 {
            let glow = scale_rgb(orb, 0.52 + 0.42 * charge);
            pb.plot_add(mp.0 + 1, mp.1, glow);
            pb.plot_add(mp.0 - 1, mp.1, glow);
            pb.plot_add(mp.0, mp.1 + 1, glow);
            pb.plot_add(mp.0, mp.1 - 1, glow);
        }
    } else if recoil > 0.25 {
        let puff = lf.px(claw_u + 2.5 * s, claw_v);
        blend(pb, puff.0, puff.1, PISTOL_CHARGE, recoil * 0.75);
    }
}

pub(crate) fn draw_pistol_shrimp_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let lf = Local::of(e, cam);
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let charge = pistol_charge(e);
    for &(dv, col) in &[
        (-0.9f32, lf.flash(PISTOL_HI)),
        (0.0, lf.flash(PISTOL_SHELL)),
        (0.9, lf.flash(PISTOL_SHADOW)),
    ] {
        line(pb, lf.px(-2.8 * s, dv * s), lf.px(2.4 * s, dv * s), col, 0.96);
    }
    for &(dv, col) in &[
        (1.4f32, lf.flash(PISTOL_HI)),
        (2.2, lf.flash(PISTOL_CLAW)),
        (2.9, lf.flash(PISTOL_CLAW)),
    ] {
        line(pb, lf.px(1.4 * s, dv * s), lf.px(4.7 * s, dv * s), col, 0.98);
    }
    let eye_c = lf.flash(PISTOL_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(2.3 * s, side * 0.8 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }
    if charge > 0.35 {
        let mp = lf.px(5.0 * s, 2.2 * s);
        pb.plot(mp.0, mp.1, PISTOL_CORE);
        pb.plot_add(mp.0 + 1, mp.1, scale_rgb(PISTOL_CHARGE, 0.65));
    }
}

/// Hostile-layer bubble-pellet raster helper for Pistol Shrimp's tagged shot.
///
/// Draws a translucent cyan cavitation bubble (radius ~2 px matching the
/// standard bullet radius) with an up-left white specular crescent, warm
/// coral-pearl iridescence on the bottom-right rim, and a micro-bubble wake
/// behind `b.vel`. Call from `World::draw_hostile_layer` when the bullet's
/// Outfall-bubble tag is set.
pub fn draw_bubble_pellet(pb: &mut PixelBuffer, b: &Bullet, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let sp = b.pos - cam;
    let cx = sp.x.round() as i32;
    let cy = sp.y.round() as i32;
    if cx < -6 || cy < -6 || cx > pb.w as i32 + 6 || cy > pb.h as i32 + 6 {
        return;
    }

    // Trailing micro-bubble wake behind velocity (or prev_pos fallback).
    let dir = if b.vel.len_sq() > 1.0 {
        b.vel.normalized()
    } else if (b.pos - b.prev_pos).len_sq() > 0.01 {
        (b.pos - b.prev_pos).normalized()
    } else {
        Vec2::ZERO
    };
    if dir.len_sq() > 0.5 {
        let t1 = sp - dir * 3.0;
        let t2 = sp - dir * 5.0;
        blend(pb, t1.x.round() as i32, t1.y.round() as i32, (132, 232, 250), 0.72);
        blend(pb, t2.x.round() as i32, t2.y.round() as i32, (74, 174, 210), 0.42);
    }

    // Crisp outer contrast shoulder + translucent cyan bubble membrane.
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let d2 = dx * dx + dy * dy;
            if d2 == 5 {
                darken(pb, cx + dx, cy + dy, 0.45);
            } else if d2 == 2 || d2 == 4 {
                let col = if dx <= 0 && dy <= 0 {
                    (224, 252, 255)
                } else if dx > 0 && dy > 0 {
                    (242, 186, 152)
                } else {
                    (116, 230, 252)
                };
                blend(pb, cx + dx, cy + dy, col, 0.92);
            } else if d2 <= 1 {
                blend(pb, cx + dx, cy + dy, (74, 178, 214), 0.48);
            }
        }
    }
    // Blazing up-left specular glint pixel.
    pb.plot(cx - 1, cy - 1, (250, 255, 255));
}

// ---- 6. Lampfish (Slot II, r=3.0) -----------------------------------------

/// Screen-space bulb position from the simulation view (`floor_outfall::lampfish_bulb`),
/// lifted with the fish's body float (`draw_tell` keeps its warning ring on the ground).
#[inline]
fn lampfish_bulb_screen(lf: &Local, e: &Enemy, cam: Vec2, lift: f32) -> Vec2 {
    if let Some(world_pos) = floor_outfall::lampfish_bulb(e) {
        let mut p = world_pos - cam;
        p.y -= lift;
        p
    } else {
        lf.pos + lf.f * LAMPFISH_STALK_REACH
    }
}

/// Mini abyssal lanternfish: dark bulbous body with pale needle underbite
/// teeth, golden eyes, fluttering side fins, and a curved dorsal stalk reaching
/// 16 px ahead to its bright cyan bioluminescent bulb.
pub(crate) fn draw_lampfish(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 3.6 * s, 2.2 * s, lift, 4.8);
    let lf = Local::of_lifted(e, cam, lift);
    let wag = (e.anim * 1.45).sin();
    let (lunge_flare, flicker_hi) = match tell_view(e) {
        Some(TellView::LampfishLunge {
            remaining,
            progress,
            active,
            ..
        }) => (
            if active { 1.0 } else { 0.35 + 0.65 * progress },
            lampfish_flicker_hi(remaining, active),
        ),
        _ => (0.0, true),
    };

    let bulb_s = lampfish_bulb_screen(&lf, e, cam, lift);
    let (bu, bv) = lf.local_vec(bulb_s);

    // 1) Forked tail + fluttering pectoral fins.
    let tail_tip_v = 1.4 * wag * s;
    let hi_c = lf.flash(LAMP_HI);
    let mid_c = lf.flash(LAMP_MID);
    for side in [-1.0f32, 1.0] {
        let t0 = lf.px(-2.8 * s, 0.3 * wag * s);
        let t1 = lf.px(-5.2 * s, tail_tip_v + side * 1.8 * s);
        line(pb, t0, t1, hi_c, 0.85);

        let f0 = lf.px(0.2 * s, side * 2.1 * s);
        let f1 = lf.px((-1.2 + 0.4 * wag * side) * s, side * (3.6 + 0.3 * wag) * s);
        line(pb, f0, f1, mid_c, 0.90);
        pb.plot(f1.0, f1.1, hi_c);
    }

    // 2) Arched dorsal illicium stalk reaching cleanly out to the bulb.
    let arch_v = (3.4 + 0.6 * (e.anim * 1.1).cos()) * s;
    let mut stalk_pts = [(0i32, 0i32); 9];
    for (i, slot) in stalk_pts.iter_mut().enumerate() {
        let t = i as f32 * (1.0 / 8.0);
        let omt = 1.0 - t;
        let cu = omt * omt * (1.2 * s) + 2.0 * omt * t * (0.52 * bu) + t * t * bu;
        let cv = 2.0 * omt * t * (0.5 * bv - arch_v) + t * t * bv;
        *slot = lf.px(cu, cv);
    }
    for (i, w) in stalk_pts.windows(2).enumerate() {
        let frac = i as f32 * (1.0 / 8.0);
        let scol = mix(LAMP_STALK, LAMP_BULB_GLOW, frac * frac * 0.55);
        line(pb, w[0], w[1], lf.flash(scol), 0.88);
    }

    // 3) Dark bulbous angler body + pale needle underbite teeth.
    let inv_u = 1.0 / (3.8 * s);
    let inv_v = 1.0 / (2.6 * s);
    raster_sdf(
        pb,
        &lf,
        (-4.6 * s, 4.8 * s, -3.2 * s, 3.2 * s),
        0.9 * s,
        (0, 0),
        (120, 180, 210),
        |u, v| {
            let body = sd_ellipse(u, v, -0.2 * s, 0.0, 3.35 * s, 2.55 * s);
            let jaw = sd_ellipse(
                u,
                v,
                (2.3 + 0.5 * lunge_flare) * s,
                0.0,
                1.55 * s,
                (2.05 + 0.45 * lunge_flare) * s,
            );
            let tail_root = sd_taper(
                u,
                v,
                (-2.0 * s, 0.0),
                (-3.8 * s, 0.5 * wag * s),
                1.45 * s,
                0.55 * s,
            );
            smin(smin(body, jaw, 0.45 * s), tail_root, 0.4 * s)
        },
        |u, v, d| {
            let nu = (u * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);
            let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.55)).clamp(0.0, 1.0);
            let mut lum = 0.28 + 0.30 * dome + 0.52 * lit;
            if u > 1.2 * s && d > -s {
                lum += 0.22;
            }
            let mut col = shade3(LAMP_DEEP, LAMP_MID, LAMP_HI, lum);
            if u > 1.8 * s && d > -0.9 * s {
                col = mix(col, LAMP_BULB_GLOW, 0.35);
            }
            if u > (2.6 - 0.3 * lunge_flare) * s && v.abs() < (1.65 + 0.45 * lunge_flare) * s {
                let tooth = ((v / (0.72 * s)).rem_euclid(1.0) - 0.5).abs() < 0.22;
                return if tooth && u > 2.85 * s {
                    LAMP_TOOTH
                } else {
                    (10, 14, 24)
                };
            }
            col
        },
    );

    // Golden-amber deep-sea eyes on brow.
    let eye_c = lf.flash(LAMP_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(1.65 * s, side * 1.45 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }

    // 4) Bright cyan bioluminescent bulb at the stalk tip.
    let bpx = bulb_s.x.round() as i32;
    let bpy = bulb_s.y.round() as i32;
    let glow_scale = if flicker_hi { 1.0 } else { 0.45 };
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let d2 = dx * dx + dy * dy;
            if d2 == 0 {
                pb.plot(bpx, bpy, if flicker_hi { LAMP_BULB_CORE } else { LAMP_BULB });
            } else if d2 == 1 {
                pb.plot(bpx + dx, bpy + dy, scale_rgb(LAMP_BULB, glow_scale));
            } else if d2 == 2 {
                pb.plot_add(bpx + dx, bpy + dy, scale_rgb(LAMP_BULB_GLOW, 0.55 * glow_scale));
            } else if d2 == 4 {
                pb.plot_add(bpx + dx, bpy + dy, scale_rgb(LAMP_BULB_GLOW, 0.25 * glow_scale));
            }
        }
    }
}

pub(crate) fn draw_lampfish_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let s = e.radius.max(1.0) * (1.0 / 3.0);
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 3.2 * s, 2.0 * s, lift, 4.8);
    let lf = Local::of_lifted(e, cam, lift);
    let bulb_s = lampfish_bulb_screen(&lf, e, cam, lift);
    let (bpx, bpy) = (bulb_s.x.round() as i32, bulb_s.y.round() as i32);
    let (bu, bv) = lf.local_vec(bulb_s);

    // Arched stalk + chunky dark bulbous body + pale teeth + bright cyan bulb.
    let mid = lf.px(0.5 * bu, 0.5 * bv - 2.5 * s);
    line(pb, lf.px(1.2 * s, 0.0), mid, LAMP_STALK, 0.85);
    line(pb, mid, (bpx, bpy), mix(LAMP_STALK, LAMP_BULB_GLOW, 0.45), 0.85);
    for &(dv, col) in &[
        (-1.4f32, lf.flash(LAMP_HI)),
        (-0.5, lf.flash(LAMP_MID)),
        (0.5, lf.flash(LAMP_MID)),
        (1.4, lf.flash(LAMP_DEEP)),
    ] {
        line(pb, lf.px(-2.4 * s, dv * s), lf.px(2.4 * s, dv * s), col, 0.98);
    }
    let eye_c = lf.flash(LAMP_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(1.5 * s, side * 1.2 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let tp = lf.px(2.8 * s, side * 0.8 * s);
        pb.plot(tp.0, tp.1, LAMP_TOOTH);
    }
    pb.plot(bpx, bpy, LAMP_BULB_CORE);
    let g = scale_rgb(LAMP_BULB_GLOW, 0.65);
    pb.plot_add(bpx + 1, bpy, g);
    pb.plot_add(bpx - 1, bpy, g);
    pb.plot_add(bpx, bpy + 1, g);
    pb.plot_add(bpx, bpy - 1, g);
}

// ---- 7. Grouper (Slot H, r=6.5) + Elite split-stalk stripe ----------------

/// Massive mottled slate-olive wreckfish with broad rowing pectoral paddles,
/// cavernous lower jaw, and spiny dorsal crest. Elite Grouper wears a
/// protruding Y-forked gold/cyan split-stalk crown and twin forked gold
/// shoulder stripes across its broad flanks.
pub(crate) fn draw_grouper(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    elite: bool,
) {
    let s = e.radius.max(1.0) * (1.0 / 6.5);
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 6.0 * s, 3.8 * s, lift, 5.2);
    let lf = Local::of_lifted(e, cam, lift);
    let elite = elite || e.elite;
    let row = (e.anim * 0.8).sin();
    let chomp = ((e.contact_cd - 0.15) * 2.5).clamp(0.0, 1.0);

    // 1) Thick rowing pectoral paddle fins + broad squared caudal tail fin.
    let fin_c = lf.flash(GROUPER_FIN);
    let fin_hi = lf.flash(GROUPER_HI);
    let deep_c = lf.flash(GROUPER_DEEP);
    for side in [-1.0f32, 1.0] {
        let p0 = lf.px(1.2 * s, side * 5.0 * s);
        let p1 = lf.px((-0.4 + 1.1 * row * side) * s, side * 8.4 * s);
        let p2 = lf.px((-2.8 + 0.7 * row * side) * s, side * 7.6 * s);
        let p3 = lf.px(-2.4 * s, side * 4.8 * s);
        thick_line(pb, p0, p1, fin_c, deep_c);
        thick_line(pb, p1, p2, fin_hi, fin_c);
        line(pb, p2, p3, fin_c, 0.92);
    }

    let tw = 0.95 * row * s;
    for tv in [-3.2f32, -1.6, 0.0, 1.6, 3.2] {
        let t0 = lf.px(-5.8 * s, tw * 0.4);
        let t1 = lf.px(-8.6 * s, tw + tv * s);
        line(pb, t0, t1, if tv.abs() > 2.0 { fin_hi } else { fin_c }, 0.92);
    }

    // 2) Broad-shouldered grouper hull + protruding jaw + (elite) split-stalk crown.
    let u_max = if elite { 8.4 * s } else { 7.6 * s };
    let inv_u = 1.0 / (6.8 * s);
    let inv_v = 1.0 / (5.6 * s);

    raster_sdf(
        pb,
        &lf,
        (-7.8 * s, u_max, -6.4 * s, 6.4 * s),
        0.95 * s,
        (0, 0),
        (146, 186, 182),
        |u, v| {
            let av = v.abs();
            let hull = sd_ellipse(u, v, -0.4 * s, 0.0, 6.2 * s, 5.45 * s);
            let jaw = sd_round_box(
                u,
                v,
                (5.2 + 0.4 * chomp) * s,
                0.0,
                1.75 * s,
                3.85 * s,
                1.2 * s,
            );
            let tail_ped = sd_taper(u, v, (-4.2 * s, 0.0), (-6.8 * s, tw * 0.5), 3.1 * s, 1.75 * s);
            let body = smin(smin(hull, jaw, 0.9 * s), tail_ped, 0.7 * s);
            if elite {
                let stem = sd_taper(u, v, (1.4 * s, 0.0), (3.6 * s, 0.0), 0.95 * s, 0.85 * s);
                let fork = sd_taper(u, av, (3.4 * s, 0.2 * s), (6.5 * s, 2.75 * s), 0.72 * s, 0.62 * s);
                let knob = sd_disc(u, av, 6.7 * s, 2.85 * s, 0.95 * s);
                body.min(stem.min(fork.min(knob)))
            } else {
                body
            }
        },
        |u, v, d| {
            let av = v.abs();
            let nu = (u * inv_u).clamp(-1.0, 1.0);
            let nv = (v * inv_v).clamp(-1.0, 1.0);
            let lit = lf.lit(nu, nv);

            // Elite split-stalk crown & forked snuff nodules.
            if elite {
                if sd_disc(u, av, 6.7 * s, 2.85 * s, 0.98 * s) <= 0.0 {
                    return if sd_disc(u, av, 6.8 * s, 2.9 * s, 0.52 * s) <= 0.0 {
                        GROUPER_ELITE_CYAN
                    } else {
                        GROUPER_ELITE_HI
                    };
                }
                if sd_taper(u, av, (3.4 * s, 0.2 * s), (6.5 * s, 2.75 * s), 0.76 * s, 0.66 * s) <= 0.0 {
                    return shade3(
                        GROUPER_ELITE_SH,
                        GROUPER_ELITE_GOLD,
                        GROUPER_ELITE_HI,
                        0.52 + 0.55 * lit,
                    );
                }
            }

            let dome = (1.0 - (nu * nu * 0.42 + nv * nv * 0.52)).clamp(0.0, 1.0);
            let mut lum = 0.34 + 0.38 * dome + 0.60 * lit;
            if d > -1.05 * s && lit > 0.2 {
                lum += 0.24;
            }
            let mut col = shade3(GROUPER_DEEP, GROUPER_MID, GROUPER_HI, lum);

            // Wide cavernous mouth lip (flashes pale biting teeth on chomp).
            if u > (5.7 + 0.15 * chomp) * s && av < 3.3 * s {
                if chomp > 0.15 && u > 5.85 * s {
                    let tooth = ((v / (1.05 * s)).rem_euclid(1.0) - 0.5).abs() < 0.24
                        && u > 6.0 * s
                        && u < 6.85 * s;
                    return if tooth { (242, 248, 240) } else { (42, 20, 28) };
                }
                return shade3(GROUPER_MID, GROUPER_LIP, (236, 246, 240), 0.55 + 0.45 * lit);
            }

            // Elite split-stalk shoulder stripes (twin forked gold chevrons off the spine).
            if elite {
                let s1 = ((u - 1.2 * s) + av * 0.45).abs();
                let s2 = ((u + 1.4 * s) + av * 0.45).abs();
                if (s1 < 0.72 * s || s2 < 0.68 * s) && av > 0.85 * s && av < 4.9 * s {
                    return shade3(
                        GROUPER_ELITE_SH,
                        GROUPER_ELITE_GOLD,
                        GROUPER_ELITE_HI,
                        0.50 + 0.58 * lit,
                    );
                }
                if av < 0.85 * s && (-3.5 * s..3.6 * s).contains(&u) {
                    return mix(GROUPER_ELITE_SH, GROUPER_DEEP, 0.45);
                }
            }

            // Pale seafoam-chalk grouper mottling spots.
            let spot = sd_disc(u, av, -1.8 * s, 3.2 * s, 1.05 * s)
                .min(sd_disc(u, av, 0.8 * s, 3.8 * s, 0.90 * s))
                .min(sd_disc(u, av, -3.4 * s, 1.8 * s, 0.95 * s));
            if spot <= 0.0 {
                col = mix(col, GROUPER_MOTTLE, if lit > -0.1 { 0.55 } else { 0.30 });
            }

            // Spiny dorsal crest ridge down the spine (non-elite).
            if !elite && av < 0.75 * s && (-4.5 * s..2.4 * s).contains(&u) {
                let spine_bump = ((u / (1.35 * s)).rem_euclid(1.0) - 0.5).abs() < 0.25;
                col = if spine_bump { GROUPER_HI } else { GROUPER_DEEP };
            }

            col
        },
    );

    // Heavy brow amber-gold eyes with crisp dark pupils & white glints.
    let eye_c = lf.flash(if elite { GROUPER_ELITE_HI } else { GROUPER_EYE });
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(3.9 * s, side * 3.1 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let e2 = lf.px(4.3 * s, side * 3.3 * s);
        pb.plot(e2.0, e2.1, (16, 20, 24));
        let gl = lf.px(3.5 * s, side * 2.8 * s);
        pb.plot(gl.0, gl.1, (255, 255, 242));
    }

    // Additive snuff-lure glint on the elite split-stalk tips.
    if elite {
        let glow = scale_rgb(GROUPER_ELITE_CYAN, 0.45);
        for side in [-1.0f32, 1.0] {
            let tp = lf.px(6.8 * s, side * 2.9 * s);
            pb.plot(tp.0, tp.1, (244, 255, 255));
            pb.plot_add(tp.0 + 1, tp.1, glow);
            pb.plot_add(tp.0 - 1, tp.1, glow);
        }
    }
}

pub(crate) fn draw_grouper_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    elite: bool,
) {
    let s = e.radius.max(1.0) * (1.0 / 6.5);
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, 5.4 * s, 3.4 * s, lift, 5.2);
    let lf = Local::of_lifted(e, cam, lift);
    let elite = elite || e.elite;
    let row = (e.anim * 0.8).sin();
    let fin_c = lf.flash(GROUPER_FIN);
    for side in [-1.0f32, 1.0] {
        let f = lf.px((-0.8 + row * side) * s, side * 7.2 * s);
        pb.plot(f.0, f.1, fin_c);
    }

    let inv_u = 1.0 / (6.0 * s);
    let inv_v = 1.0 / (5.2 * s);
    raster_sdf(
        pb,
        &lf,
        (-7.0 * s, 7.6 * s, -6.0 * s, 6.0 * s),
        0.85 * s,
        (0, 0),
        (146, 186, 182),
        |u, v| {
            let hull = sd_ellipse(u, v, -0.4 * s, 0.0, 6.0 * s, 5.2 * s);
            if elite {
                let fork = sd_taper(
                    u,
                    v.abs(),
                    (3.0 * s, 0.2 * s),
                    (6.4 * s, 2.7 * s),
                    0.75 * s,
                    0.65 * s,
                );
                hull.min(fork)
            } else {
                hull
            }
        },
        |u, v, _| {
            let av = v.abs();
            if elite {
                if u > 4.2 * s && av > 1.2 * s {
                    return if u > 5.8 * s {
                        GROUPER_ELITE_CYAN
                    } else {
                        GROUPER_ELITE_GOLD
                    };
                }
                let s1 = ((u - 1.2 * s) + av * 0.45).abs();
                let s2 = ((u + 1.4 * s) + av * 0.45).abs();
                if (s1 < 0.75 * s || s2 < 0.72 * s) && av > 0.8 * s {
                    return GROUPER_ELITE_GOLD;
                }
            }
            let lit = lf.lit((u * inv_u).clamp(-1.0, 1.0), (v * inv_v).clamp(-1.0, 1.0));
            shade3(GROUPER_DEEP, GROUPER_MID, GROUPER_HI, 0.42 + 0.56 * lit)
        },
    );

    let eye_c = lf.flash(GROUPER_EYE);
    for side in [-1.0f32, 1.0] {
        let ep = lf.px(3.6 * s, side * 2.9 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }
}

// ---- 8. Fry School (Slot III, r=12 dark / r=6 bunched) --------------------

/// Deterministic school layout (`u`, `v`, `phase`) for 15 shimmering minnows.
const FRY_SLOTS: [(f32, f32, f32); 15] = [
    (0.00, 0.00, 0.0),
    (0.32, 0.18, 0.6),
    (0.35, -0.22, 1.3),
    (-0.30, 0.25, 2.1),
    (-0.34, -0.20, 2.8),
    (0.62, 0.02, 3.5),
    (-0.62, -0.04, 4.1),
    (0.08, 0.58, 4.9),
    (-0.06, -0.58, 5.5),
    (0.54, 0.52, 0.9),
    (0.56, -0.50, 1.9),
    (-0.52, 0.54, 2.7),
    (-0.55, -0.52, 3.8),
    (0.84, 0.22, 4.6),
    (-0.82, -0.24, 5.2),
];

/// Balanced 11-slot subset covering all four quadrants for far LOD.
const FRY_LOD_INDICES: [usize; 11] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12];

/// One living baitfish school:
/// * **Dark (`e.radius > 8.5`)**: loose coordinated cruise filling the r=12 disc.
/// * **Bunched / lit (`e.radius <= 8.5`, ×2 damage)**: tight swirling panic
///   baitball flashing bright gold-silver scales with a crisp outer rim.
pub(crate) fn draw_fry_school(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let r = e.radius.max(3.0);
    let bunched = r <= FRY_BUNCHED_RADIUS;
    let spread = r * if bunched { 0.82 } else { 0.88 };
    let lift = swimmer_lift(e);
    let ground = e.pos - cam;

    // 1) Detached soft shadow eddy on the seabed under the TRUE ground position (pen 0).
    draw_floating_shadow(pb, ground, r * 0.78, r * 0.52, lift, 4.5);

    let lf = Local::of_lifted(e, cam, lift);
    // Bunched vulnerability halo stays around the lifted swirling baitball.
    if bunched {
        let cx = lf.pos.x.round() as i32;
        let cy = lf.pos.y.round() as i32;
        let rad_i = (r * 0.92).round().max(3.0) as i32;
        let inv_rad = 1.0 / (rad_i as f32).max(1.0);
        for dy in -rad_i..=rad_i {
            for dx in -rad_i..=rad_i {
                let d2 = dx * dx + dy * dy;
                if d2 <= rad_i * rad_i {
                    let dist = (d2 as f32).sqrt() * inv_rad;
                    if dist > 0.78 {
                        blend(
                            pb,
                            cx + dx,
                            cy + dy,
                            lf.flash(FRY_VULN_RING),
                            0.42 * (1.0 - (dist - 0.78) * (1.0 / 0.22)),
                        );
                    }
                }
            }
        }
    }

    // 2) Individual minnows with waving tails and shimmering scales, floating above the shadow.
    for (idx, &(su, sv, ph)) in FRY_SLOTS.iter().enumerate() {
        let (fu, fv, hu, hv) = if bunched {
            let base_ang = sv.atan2(su) + e.anim * 2.4 + ph * 0.25;
            let rr = su.hypot(sv) * spread * (0.82 + 0.14 * (e.anim * 3.1 + ph).sin());
            let (sa, ca) = base_ang.sin_cos();
            (ca * rr, sa * rr, -sa, ca)
        } else {
            let sway_u = (e.anim * 1.4 + ph).sin() * 1.35;
            let sway_v = (e.anim * 1.7 + ph * 1.3).cos() * 1.15;
            let yaw = (e.anim * 1.7 + ph * 1.3).sin() * 0.25;
            let (sy, cy_yaw) = yaw.sin_cos();
            (su * spread + sway_u, sv * spread + sway_v, cy_yaw, sy)
        };

        let tail_w = (e.anim * 3.6 + ph).sin() * 0.55;
        let p_head = lf.px(fu + hu * 1.25, fv + hv * 1.25);
        let p_mid = lf.px(fu, fv);
        let p_tail = lf.px(fu - hu * 1.55 - hv * tail_w, fv - hv * 1.55 + hu * tail_w);

        let shimmer = (e.anim * if bunched { 4.2 } else { 2.2 } + ph * 2.0).sin() > 0.35;
        let (head_c, mid_c, tail_c) = if bunched {
            (
                if shimmer { FRY_GLINT } else { FRY_VULN_HI },
                if shimmer { FRY_VULN_HI } else { FRY_VULN_MID },
                FRY_MID,
            )
        } else {
            (
                if shimmer { FRY_GLINT } else { FRY_SILVER },
                if idx % 3 == 0 { FRY_SILVER } else { FRY_MID },
                FRY_DEEP,
            )
        };

        pb.plot(p_tail.0, p_tail.1, lf.flash(tail_c));
        pb.plot(p_mid.0, p_mid.1, lf.flash(mid_c));
        pb.plot(p_head.0, p_head.1, lf.flash(head_c));
    }
}

pub(crate) fn draw_fry_school_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let r = e.radius.max(3.0);
    let bunched = r <= FRY_BUNCHED_RADIUS;
    let spread = r * 0.80;
    let lift = swimmer_lift(e);
    draw_floating_shadow(pb, e.pos - cam, r * 0.72, r * 0.48, lift, 4.5);
    let lf = Local::of_lifted(e, cam, lift);
    if bunched {
        let cx = lf.pos.x.round() as i32;
        let cy = lf.pos.y.round() as i32;
        let rad_i = (r * 0.82).round().max(2.0) as i32;
        let inv_rad = 1.0 / (rad_i as f32).max(1.0);
        for dy in -rad_i..=rad_i {
            for dx in -rad_i..=rad_i {
                let d2 = dx * dx + dy * dy;
                if d2 <= rad_i * rad_i {
                    let dist = (d2 as f32).sqrt() * inv_rad;
                    if dist > 0.76 {
                        blend(pb, cx + dx, cy + dy, lf.flash(FRY_VULN_RING), 0.38);
                    }
                }
            }
        }
    }
    let tail_c = lf.flash(if bunched { FRY_VULN_MID } else { FRY_MID });
    for &idx in &FRY_LOD_INDICES {
        let (su, sv, _ph) = FRY_SLOTS[idx];
        let (fu, fv) = if bunched {
            let ang = sv.atan2(su) + e.anim * 2.2;
            let rr = su.hypot(sv) * spread;
            let (sa, ca) = ang.sin_cos();
            (ca * rr, sa * rr)
        } else {
            (su * spread, sv * spread)
        };
        let p0 = lf.px(fu + 0.8, fv);
        let p1 = lf.px(fu - 0.8, fv);
        let head_c = if bunched {
            FRY_VULN_HI
        } else if idx % 3 == 0 {
            FRY_GLINT
        } else {
            FRY_SILVER
        };
        pb.plot(p1.0, p1.1, tail_c);
        pb.plot(p0.0, p0.1, lf.flash(head_c));
    }
}

// ---- Hostile-layer tells (`draw_tell`) ------------------------------------

/// Draw an Outfall ordinary enemy's floor tell on the post-lighting hostile
/// layer (`OUTFALL_PLAN.md` §6 `outfall-art-enemies`).
///
/// Reads `floor_outfall::outfall_enemy_tell(e)` and
/// `floor_outfall::lampfish_bulb(e)`:
/// * **Skater**: crisp 22×4 splash lane — dashed cyan/white rails with a
///   creeping foam fill during the 0.45 s tell, then a blazing white-cyan wave
///   with spray droplets on the active test.
/// * **Lampfish**: 3 Hz stepped warning ring around the forward bulb plus the
///   locked 18 px lunge corridor and fang chevron during the 0.4 s tell, then a
///   bright cyan-white lunge streak during the 0.18 s sweep.
///
/// A no-op for any enemy without an active Outfall tell.
pub fn draw_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    if !matches!(e.kind, EnemyKind::Skater | EnemyKind::Lampfish) {
        return;
    }
    let Some(tv) = tell_view(e) else {
        return;
    };
    match tv {
        TellView::SkaterSplash {
            start,
            end,
            half_width,
            progress,
            active,
        } => draw_skater_splash_tell(pb, start - cam, end - cam, half_width, progress, active, accent),
        TellView::LampfishLunge {
            start,
            end,
            bulb,
            remaining,
            progress,
            active,
        } => draw_lampfish_lunge_tell(
            pb,
            start - cam,
            end - cam,
            bulb - cam,
            remaining,
            progress,
            active,
            accent,
        ),
    }
}

/// Rasterise the Skater's 22×4 splash corridor in screen space.
fn draw_skater_splash_tell(
    pb: &mut PixelBuffer,
    start: Vec2,
    end: Vec2,
    half_width: f32,
    progress: f32,
    active: bool,
    accent: Rgb,
) {
    let along = end - start;
    let len = along.len();
    if len < 1.0 {
        return;
    }
    let dir = along * (1.0 / len);
    let side = Vec2::new(-dir.y, dir.x);
    let steps = len.ceil() as i32;
    let lead = len * progress;
    let rail_base = mix(TELL_CYAN, accent, 0.18);

    for i in 0..=steps {
        let t = i as f32;
        let c = start + dir * t;
        if active {
            for w in -2i32..=2 {
                let p = c + side * (w as f32 * half_width * 0.5);
                let (px, py) = (p.x.round() as i32, p.y.round() as i32);
                match w.abs() {
                    2 => blend(pb, px, py, TELL_CYAN, 0.95),
                    1 => pb.plot(px, py, TELL_WHITE),
                    _ => pb.plot(px, py, (186, 252, 255)),
                }
            }
            if i % 5 == 2 && i > 2 {
                for sgn in [-1.0f32, 1.0] {
                    let dp = c + side * (sgn * (half_width + 1.4));
                    blend(pb, dp.x.round() as i32, dp.y.round() as i32, TELL_WHITE, 0.85);
                }
            }
        } else {
            let rail_c = mix(rail_base, TELL_WHITE, progress);
            for sgn in [-1.0f32, 1.0] {
                if i % 3 != 2 {
                    let p = c + side * (sgn * half_width);
                    blend(pb, p.x.round() as i32, p.y.round() as i32, rail_c, 0.92);
                }
            }
            if t <= lead {
                let at_front = (lead - t) <= 1.8;
                let fill_c = if at_front {
                    TELL_WHITE
                } else {
                    mix((48, 128, 162), TELL_CYAN, 0.45 + 0.4 * progress)
                };
                let alpha = if at_front { 0.94 } else { 0.58 };
                for w in -1i32..=1 {
                    let p = c + side * (w as f32 * (half_width - 0.9).max(0.5));
                    blend(pb, p.x.round() as i32, p.y.round() as i32, fill_c, alpha);
                }
            }
        }
    }

    // Crisp tip cap bracket.
    let tip_c = if active { TELL_WHITE } else { TELL_CYAN };
    for w in -2i32..=2 {
        let p = end + side * (w as f32 * half_width * 0.5);
        blend(pb, p.x.round() as i32, p.y.round() as i32, tip_c, 0.96);
    }
}

/// Rasterise the Lampfish's 3 Hz bulb warning ring + locked 18 px lunge lane
/// in screen space.
#[allow(clippy::too_many_arguments)]
fn draw_lampfish_lunge_tell(
    pb: &mut PixelBuffer,
    start: Vec2,
    end: Vec2,
    bulb: Vec2,
    remaining: f32,
    progress: f32,
    active: bool,
    accent: Rgb,
) {
    // 1) Bulb warning ring (3 Hz stepped pulse during warn, bright flare on lunge).
    let flicker_hi = lampfish_flicker_hi(remaining, active);
    let ring_r = if active { 4.0 } else { 5.5 - 1.5 * progress };
    let ring_col = if active {
        TELL_WHITE
    } else if flicker_hi {
        mix(TELL_CYAN, accent, 0.12)
    } else {
        TELL_WARN
    };
    let ring_alpha = if flicker_hi { 0.95 } else { 0.72 };
    const SEGS: usize = 20;
    for k in 0..SEGS {
        let a = k as f32 * (TAU / SEGS as f32);
        let (sa, ca) = a.sin_cos();
        let rx = (bulb.x + ca * ring_r).round() as i32;
        let ry = (bulb.y + sa * ring_r).round() as i32;
        blend(pb, rx, ry, ring_col, ring_alpha);
    }

    // 2) Locked 18 px lunge corridor from body toward locked end.
    let along = end - start;
    let len = along.len();
    if len < 1.0 {
        return;
    }
    let dir = along * (1.0 / len);
    let side = Vec2::new(-dir.y, dir.x);
    let half_w = 2.8f32;
    let steps = len.ceil() as i32;
    let warn_rail = mix(TELL_WARN, TELL_HOT, progress);

    for i in 0..=steps {
        let t = i as f32;
        let c = start + dir * t;
        if active {
            for w in -1i32..=1 {
                let p = c + side * (w as f32 * 1.4);
                pb.plot(
                    p.x.round() as i32,
                    p.y.round() as i32,
                    if w == 0 { TELL_WHITE } else { TELL_CYAN },
                );
            }
        } else {
            for sgn in [-1.0f32, 1.0] {
                if i % 3 != 2 {
                    let p = c + side * (sgn * half_w);
                    blend(pb, p.x.round() as i32, p.y.round() as i32, warn_rail, 0.90);
                }
            }
            if i % 4 == 0 && t <= len * (0.25 + 0.75 * progress) {
                blend(pb, c.x.round() as i32, c.y.round() as i32, TELL_WHITE, 0.85);
            }
        }
    }

    // Fang chevron at the lunge endpoint.
    let tip_col = if active { TELL_WHITE } else { TELL_WARN };
    let tip_px = (end.x.round() as i32, end.y.round() as i32);
    for sgn in [-1.0f32, 1.0] {
        let wing = end - dir * 3.0 + side * (sgn * 3.0);
        line(pb, tip_px, (wing.x.round() as i32, wing.y.round() as i32), tip_col, 0.96);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::bullet::Faction;
    use rand::SeedableRng;

    const BG: Rgb = (24, 38, 46);
    const DARK_OUTFALL_BG: Rgb = (12, 20, 28);
    const ACCENT: Rgb = (96, 224, 240);

    const KINDS: [EnemyKind; 8] = [
        EnemyKind::SandHopper,
        EnemyKind::GlassShrimp,
        EnemyKind::Conger,
        EnemyKind::Skater,
        EnemyKind::PistolShrimp,
        EnemyKind::Lampfish,
        EnemyKind::Grouper,
        EnemyKind::FrySchool,
    ];

    fn buffer_with(bg: Rgb) -> PixelBuffer {
        let mut pb = PixelBuffer::new(96, 96);
        pb.clear(bg);
        pb
    }

    fn buffer() -> PixelBuffer {
        buffer_with(BG)
    }

    fn body(kind: EnemyKind) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let mut e = Enemy::spawn(kind, Vec2::new(48.0, 48.0), 1.0, &mut rng);
        e.anim = 0.4;
        e.facing = Vec2::new(1.0, 0.0);
        e
    }

    fn drawn(e: &Enemy) -> PixelBuffer {
        let mut pb = buffer();
        crate::game::art_enemy::draw_enemy(&mut pb, e, Vec2::ZERO, ACCENT);
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

    #[test]
    fn every_outfall_body_has_a_light_and_a_dark_edge() {
        for kind in KINDS {
            let px = painted(&drawn(&body(kind)));
            assert!(px.len() >= 12, "{kind:?} paints a real body ({} px)", px.len());
            let brightest = px.iter().map(|p| lum(p.2)).max().unwrap();
            let darkest = px.iter().map(|p| lum(p.2)).min().unwrap();
            assert!(brightest > lum(BG) + 80, "{kind:?} has no light edge ({brightest})");
            assert!(darkest + 24 < lum(BG), "{kind:?} has no dark edge ({darkest})");
        }
    }

    #[test]
    fn every_outfall_body_animates_across_walk_frames() {
        for kind in KINDS {
            let mut a = body(kind);
            let mut b = body(kind);
            a.anim = 0.4;
            b.anim = 4.1;
            assert!(
                painted(&drawn(&a)) != painted(&drawn(&b)),
                "{kind:?} needs distinct animation frames"
            );
        }
    }

    #[test]
    fn all_eight_outfall_silhouettes_are_distinct() {
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
    fn all_eight_far_lod_bodies_paint_and_are_distinct() {
        let mut masks = Vec::new();
        for kind in KINDS {
            let mut pb = buffer();
            crate::game::art_enemy::draw_enemy_lod(&mut pb, &body(kind), Vec2::ZERO, ACCENT);
            let px = painted(&pb);
            assert!(px.len() >= 4, "{kind:?} far LOD is visible ({} px)", px.len());
            masks.push(px.into_iter().map(|p| (p.0, p.1)).collect::<Vec<_>>());
        }
        for i in 0..masks.len() {
            for j in i + 1..masks.len() {
                assert_ne!(masks[i], masks[j], "LOD {:?} and {:?}", KINDS[i], KINDS[j]);
            }
        }
    }

    #[test]
    fn every_outfall_body_survives_dark_outfall_ambient() {
        for kind in KINDS {
            let mut pb = buffer_with(DARK_OUTFALL_BG);
            crate::game::art_enemy::draw_enemy(&mut pb, &body(kind), Vec2::ZERO, ACCENT);
            let px = painted_over(&pb, DARK_OUTFALL_BG);
            let brightest = px.iter().map(|p| lum(p.2)).max().unwrap_or(0);
            assert!(
                brightest > 450,
                "{kind:?} needs a bright rim/eye/bulb glint for dark seabed (max lum {brightest})"
            );
        }
    }

    #[test]
    fn pistol_shrimp_charges_and_recoils_with_fire_cd() {
        let mut calm = body(EnemyKind::PistolShrimp);
        calm.fire_cd = 1.5;
        let mut charging = body(EnemyKind::PistolShrimp);
        charging.fire_cd = 0.08;
        let mut recoiling = body(EnemyKind::PistolShrimp);
        recoiling.fire_cd = 2.15;

        assert_eq!(pistol_charge(&calm), 0.0);
        assert!(pistol_charge(&charging) > 0.8);
        assert!(painted(&drawn(&calm)) != painted(&drawn(&charging)));
        assert!(painted(&drawn(&calm)) != painted(&drawn(&recoiling)));
    }

    #[test]
    fn bubble_pellet_paints_translucent_ring_and_wake() {
        let mut pb = buffer();
        let b = Bullet::new(
            Vec2::new(48.0, 48.0),
            Vec2::new(75.0, 0.0),
            4.0,
            Faction::Enemy,
            (132, 232, 250),
        );
        draw_bubble_pellet(&mut pb, &b, Vec2::ZERO);
        let px = painted(&pb);
        assert!(px.len() >= 12, "bubble pellet should paint ring + wake ({} px)", px.len());
        let has_glint = px.iter().any(|p| lum(p.2) > 700);
        let has_wake = px.iter().any(|p| p.0 <= 45);
        assert!(has_glint, "bubble pellet needs a bright specular glint");
        assert!(has_wake, "bubble pellet needs a trailing micro-bubble wake");
    }

    #[test]
    fn elite_grouper_wears_split_stalk_in_full_and_lod() {
        let normal = body(EnemyKind::Grouper);
        let mut elite = body(EnemyKind::Grouper);
        elite.elite = true;
        let n_px = painted(&drawn(&normal));
        let e_px = painted(&drawn(&elite));
        assert!(n_px != e_px, "elite grouper must look distinct");
        let n_front = n_px.iter().map(|p| p.0).max().unwrap();
        let e_front = e_px.iter().map(|p| p.0).max().unwrap();
        assert!(
            e_front >= n_front,
            "split-stalk crown should protrude forward (elite {e_front} vs normal {n_front})"
        );
        let mut pb_n = buffer();
        let mut pb_e = buffer();
        crate::game::art_enemy::draw_enemy_lod(&mut pb_n, &normal, Vec2::ZERO, ACCENT);
        crate::game::art_enemy::draw_enemy_lod(&mut pb_e, &elite, Vec2::ZERO, ACCENT);
        assert!(painted(&pb_n) != painted(&pb_e), "far-LOD elite grouper must show split stalk");
    }

    #[test]
    fn fry_school_shrinks_and_changes_pose_when_bunched() {
        let mut dark = body(EnemyKind::FrySchool);
        dark.radius = 12.0;
        let mut bunched = body(EnemyKind::FrySchool);
        bunched.radius = 6.0;
        let d_px = painted(&drawn(&dark));
        let b_px = painted(&drawn(&bunched));
        assert!(d_px != b_px);
        let max_dist = |px: &[(usize, usize, Rgb)]| {
            px.iter()
                .map(|p| Vec2::new(p.0 as f32, p.1 as f32).dist(Vec2::new(48.0, 48.0)))
                .fold(0.0f32, f32::max)
        };
        assert!(
            max_dist(&b_px) < max_dist(&d_px) - 2.0,
            "bunched baitball must visibly tighten (bunched {:.1} vs dark {:.1})",
            max_dist(&b_px),
            max_dist(&d_px)
        );
    }

    #[test]
    fn skater_dry_vs_wet_stance_differs() {
        let mut dry = body(EnemyKind::Skater);
        dry.vel = Vec2::new(12.0, 0.0);
        let mut wet = body(EnemyKind::Skater);
        wet.vel = Vec2::new(36.0, 0.0);
        let d_px = painted(&drawn(&dry));
        let w_px = painted(&drawn(&wet));
        assert!(d_px != w_px);
        let span = |px: &[(usize, usize, Rgb)]| {
            px.iter().map(|p| p.1).max().unwrap() - px.iter().map(|p| p.1).min().unwrap()
        };
        assert!(
            span(&w_px) > span(&d_px),
            "wet gliding skater should splay legs wider than dry plodding skater"
        );
    }

    #[test]
    fn outfall_tell_rasterizers_paint_full_skater_and_lampfish_footprints() {
        // Skater 22x4 splash warn and active.
        let mut pb_sw = buffer();
        draw_skater_splash_tell(
            &mut pb_sw,
            Vec2::new(30.0, 48.0),
            Vec2::new(52.0, 48.0),
            2.0,
            0.55,
            false,
            ACCENT,
        );
        let sw_px = painted(&pb_sw);
        assert!(sw_px.iter().any(|p| p.0 >= 51), "skater warn must reach full 22 px tip");

        let mut pb_sa = buffer();
        draw_skater_splash_tell(
            &mut pb_sa,
            Vec2::new(30.0, 48.0),
            Vec2::new(52.0, 48.0),
            2.0,
            1.0,
            true,
            ACCENT,
        );
        assert!(painted(&pb_sa) != sw_px, "active splash must brighten over warn");

        // Lampfish 0.4s bulb ring + 18 px lunge warn and active.
        let mut pb_lw = buffer();
        draw_lampfish_lunge_tell(
            &mut pb_lw,
            Vec2::new(30.0, 48.0),
            Vec2::new(48.0, 48.0),
            Vec2::new(46.0, 48.0),
            0.20,
            0.50,
            false,
            ACCENT,
        );
        let lw_px = painted(&pb_lw);
        assert!(lw_px.iter().any(|p| p.0 >= 49), "lampfish warn must ring the forward bulb");

        let mut pb_la = buffer();
        draw_lampfish_lunge_tell(
            &mut pb_la,
            Vec2::new(30.0, 48.0),
            Vec2::new(48.0, 48.0),
            Vec2::new(46.0, 48.0),
            0.05,
            0.72,
            true,
            ACCENT,
        );
        assert!(painted(&pb_la) != lw_px, "active lunge must differ from warn");
    }

    #[test]
    fn every_outfall_body_stays_inside_its_cull_reach() {
        let cases = [
            (EnemyKind::SandHopper, false, 11.5_f32),
            (EnemyKind::GlassShrimp, false, 10.0_f32),
            (EnemyKind::Conger, false, 18.0_f32),
            (EnemyKind::Skater, false, 11.5_f32),
            (EnemyKind::PistolShrimp, false, 13.0_f32),
            (EnemyKind::Lampfish, false, 27.0_f32),
            (EnemyKind::Grouper, false, 18.3_f32),
            (EnemyKind::Grouper, true, 18.3_f32),
            (EnemyKind::FrySchool, false, 17.0_f32),
        ];
        for (kind, is_elite, max_r) in cases {
            let mut e = body(kind);
            e.elite = is_elite;
            if kind == EnemyKind::PistolShrimp {
                e.fire_cd = 0.08;
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
    fn outfall_death_cues_emit_particles_for_every_kind() {
        use crate::game::particle::Particles;
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for kind in KINDS {
            let mut ps = Particles::new(128);
            crate::game::particle::vfx::emit_enemy_death(
                &mut ps,
                &mut rng,
                Vec2::new(40.0, 40.0),
                kind.radius(),
                kind,
                (38, 128, 148),
            );
            assert!(
                ps.len() >= 4 && ps.len() <= 48,
                "{kind:?} death burst should be visible and bounded ({})",
                ps.len()
            );
        }
    }

    #[test]
    fn outfall_swimmers_float_above_detached_shadows_and_bottom_dwellers_stay_grounded() {
        let swimmers = [
            EnemyKind::GlassShrimp,
            EnemyKind::Conger,
            EnemyKind::Lampfish,
            EnemyKind::Grouper,
            EnemyKind::FrySchool,
        ];
        for kind in swimmers {
            let mut e = body(kind);
            e.id = 17;
            let mut pb = buffer();
            pb.set_materials(true);
            pb.clear(BG);
            let pen = crate::render::lighting::enemy_pen(kind, None);
            pb.set_pen(pen);
            crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, ACCENT);
            pb.set_pen(0);

            // Detached seabed shadow sits at the TRUE ground position with floor pen 0.
            let gx = e.pos.x as usize;
            let gy = e.pos.y as usize;
            let mut shadow_near_ground = 0usize;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let x = (gx as i32 + dx) as usize;
                    let y = (gy as i32 + dy) as usize;
                    let c = pb.pixel_at(x, y);
                    if pb.mat()[y * pb.w + x] == 0 && lum(c) < lum(BG) {
                        shadow_near_ground += 1;
                    }
                }
            }
            assert!(
                shadow_near_ground >= 3,
                "{kind:?} must cast a detached seabed shadow (pen 0) at its true ground position"
            );

            // Creature sprite pixels sit clearly ABOVE the ground shadow.
            let mut sum_y = 0.0f32;
            let mut body_px = 0usize;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    if pb.mat()[y * pb.w + x] == pen {
                        sum_y += y as f32;
                        body_px += 1;
                    }
                }
            }
            assert!(body_px >= 10, "{kind:?} should draw creature pixels ({body_px})");
            let mean_y = sum_y / body_px as f32;
            assert!(
                mean_y <= e.pos.y - 2.0,
                "{kind:?} sprite should float above ground y={} (mean_y={mean_y:.2})",
                e.pos.y
            );

            // Different enemy IDs bob out of phase with one another.
            let mut peer = e.clone();
            peer.id = 42;
            assert_ne!(
                swimmer_lift(&e),
                swimmer_lift(&peer),
                "{kind:?} per-body ID hash should desync float phase"
            );
        }

        // Bottom dwellers stay grounded with zero lift.
        for grounded in [
            EnemyKind::SandHopper,
            EnemyKind::PistolShrimp,
            EnemyKind::Skater,
        ] {
            let e = body(grounded);
            assert_eq!(swimmer_lift(&e), 0.0, "{grounded:?} must stay grounded");
            let mut pb = buffer();
            pb.set_materials(true);
            pb.clear(BG);
            let pen = crate::render::lighting::enemy_pen(grounded, None);
            pb.set_pen(pen);
            crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, ACCENT);
            pb.set_pen(0);
            assert_eq!(
                pb.mat()[e.pos.y as usize * pb.w + e.pos.x as usize],
                pen,
                "{grounded:?} body should stay centred on its true ground position"
            );
        }
    }
}
