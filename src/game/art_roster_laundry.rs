//! The Laundry's own bodies (`WAVE4_ROSTERS.md`, "Laundry"): ivory yarn and
//! copper fittings against indigo grout.
//!
//! Five registrations get a body here, each a full-detail draw and a far-LOD
//! draw with `art_enemy`'s `BodyDraw` signature
//! (`fn(&mut PixelBuffer, &Frame, &Enemy, Vec2, bool)`), registered by one row
//! each in `art_enemy::bespoke_body` / `bespoke_lod`:
//!
//! | kind | silhouette | motion | tell pose |
//! |---|---|---|---|
//! | Wool Pill | pale scalloped tuft on a trailing stitch | gathers and stretches | bunched wide, strand pulled taut & bright (its 0.15 s stall) |
//! | Clothes Moth | broad moth-eaten triangular shawl over a blunt head | shawl opens and shuts | none |
//! | Peg Snapper | forked wooden clothespin jaw on a gleaming brass hinge | walks open, rear handles rock | prongs flung wide, hinge white-hot, then snapped shut; floor jaws |
//! | Starch Bulb | ribbed rubber pear on a copper-footed tripod, crooked nozzle | tripod steps, nozzle rocks | bulb squeezed & lit white, sparkle bead at tip, post-shot recoil |
//! | Lint Golem | broad uneven shoulders, dark 4-hole dryer-button eye | shambles heavily | none (elite: two brass coat buttons + thick double shoulder stripe) |
//!
//! Everything is drawn in the body's local frame (`fr.f` forward, `fr.p`
//! sideways) and scaled by `fr.r` over the kind's registered radius, so a
//! body that spawns larger keeps its proportions. Directional lighting comes
//! from the upper left (`LIGHT = (-0.55, -0.83)`), matching `art_enemy.rs`,
//! with dark silhouette rims, down-right drop shadows, and bright eye/brass
//! glints that survive pooled low ambient light and 16-colour folding.
//! Render-only: nothing here reads an rng or writes the world.
//!
//! The Peg Snapper's floor jaws are drawn from
//! [`floor_laundry::jaw_view`](crate::game::ecs::systems::floor_laundry::jaw_view),
//! the same geometry its hit test uses.

use std::f32::consts::TAU;

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_laundry as rules;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// Direction *toward* the light in screen space (up-left), matching `art_enemy.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// ---- Palette ----------------------------------------------------------------

// Wool Pill: warm unbleached ivory/oatmeal yarn (distinct from the cool-white rat).
const WOOL_RIM: Rgb = (24, 18, 16);
const WOOL_DARK: Rgb = (122, 106, 84);
const WOOL_MID: Rgb = (218, 202, 168);
const WOOL_HI: Rgb = (255, 248, 224);
const WOOL_STRAND: Rgb = (196, 152, 108);
const WOOL_STRAND_TAUT: Rgb = (255, 232, 168);
const WOOL_EYE_DARK: Rgb = (42, 24, 22);
const WOOL_EYE_GLINT: Rgb = (255, 198, 118);

// Clothes Moth: woven camel-taupe wool shawl with ivory lace trim and dark blunt head.
const MOTH_RIM: Rgb = (20, 14, 18);
const MOTH_DARK: Rgb = (86, 64, 44);
const MOTH_MID: Rgb = (176, 146, 108);
const MOTH_HI: Rgb = (236, 214, 176);
const MOTH_LACE: Rgb = (248, 236, 206);
const MOTH_HOLE: Rgb = (48, 34, 28);
const MOTH_HEAD_DARK: Rgb = (44, 34, 32);
const MOTH_HEAD_MID: Rgb = (78, 62, 56);
const MOTH_HEAD_HI: Rgb = (126, 104, 92);
const MOTH_EYE: Rgb = (255, 208, 92);
const MOTH_ANT: Rgb = (214, 186, 138);

// Peg Snapper: warm beechwood clothespin with gleaming brass-copper coil spring.
const PEG_RIM: Rgb = (26, 16, 10);
const PEG_DARK: Rgb = (102, 64, 34);
const PEG_MID: Rgb = (196, 146, 86);
const PEG_HI: Rgb = (248, 218, 162);
const PEG_GRAIN: Rgb = (142, 94, 48);
const PEG_SPRING_DARK: Rgb = (148, 88, 28);
const PEG_HINGE: Rgb = (255, 206, 84);
const PEG_HINGE_HOT: Rgb = (255, 250, 218);

// Starch Bulb: porcelain-starch rubber pear, copper collar & crooked nozzle, copper-footed tripod.
const BULB_RIM: Rgb = (18, 22, 34);
const BULB_DARK: Rgb = (72, 92, 122);
const BULB_MID: Rgb = (186, 200, 216);
const BULB_HI: Rgb = (248, 252, 255);
const BULB_RIB: Rgb = (114, 134, 160);
const BULB_LEG: Rgb = (82, 94, 114);
const BULB_LEG_HI: Rgb = (138, 152, 174);
const BULB_FOOT: Rgb = (224, 156, 82);
const BULB_COPPER_DARK: Rgb = (112, 64, 34);
const BULB_COPPER_MID: Rgb = (208, 134, 70);
const BULB_COPPER_HI: Rgb = (255, 218, 150);
const BULB_GAUGE: Rgb = (146, 238, 255);
const BULB_CHARGE: Rgb = (255, 255, 255);

// Lint Golem: heather-grey dryer-trap felt, stray sock-pink & denim flecks, ebony button eye.
const LINT_RIM: Rgb = (18, 16, 22);
const LINT_DARK: Rgb = (58, 56, 64);
const LINT_MID: Rgb = (136, 134, 138);
const LINT_HI: Rgb = (200, 198, 194);
const LINT_TUFT: Rgb = (228, 226, 220);
const LINT_SOCK_PINK: Rgb = (206, 128, 148);
const LINT_DENIM: Rgb = (96, 130, 158);
const LINT_EYE: Rgb = (16, 14, 20);
const LINT_EYE_RIM: Rgb = (110, 108, 120);
const LINT_EYE_GLINT: Rgb = (240, 246, 255);
const LINT_EYE_THREAD: Rgb = (180, 172, 156);
const BRASS: Rgb = (240, 198, 96);
const BRASS_DARK: Rgb = (144, 96, 30);
const BRASS_HI: Rgb = (255, 246, 190);
const ELITE_STRIPE: Rgb = (34, 28, 36);
const ELITE_AURA: Rgb = (228, 182, 74);

// Peg Snapper floor jaws: warning amber, lethal red-white, resting beechwood.
const JAW_WARN: Rgb = (255, 192, 56);
const JAW_WARN_HOT: Rgb = (255, 242, 164);
const JAW_LIVE: Rgb = (255, 78, 52);
const JAW_LIVE_CORE: Rgb = (255, 246, 226);
const JAW_SHUT: Rgb = (158, 120, 78);

/// Seconds before the shot at which the Starch Bulb shows its charge swell
/// (matches the Spitter's 0.75 s muzzle swell in `WAVE4_ROSTERS.md`).
const BULB_CHARGE_SECS: f32 = 0.75;

// ---- Local shading & geometry helpers ---------------------------------------

#[inline(always)]
fn mix_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline(always)]
fn scale_rgb(a: Rgb, f: f32) -> Rgb {
    let f = f.max(0.0);
    (
        (a.0 as f32 * f).min(255.0) as u8,
        (a.1 as f32 * f).min(255.0) as u8,
        (a.2 as f32 * f).min(255.0) as u8,
    )
}

#[inline(always)]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.65 {
        mix_rgb(shadow, mid, l * (1.0 / 0.65))
    } else {
        mix_rgb(mid, hi, (l - 0.65) * 2.0)
    }
}

/// Sculptural warm-white hit flash matching `art_enemy.rs`.
#[inline(always)]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.0 {
        return c;
    }
    let flash_col = if hit > 0.6 {
        (255, 244, 218)
    } else {
        (255, 218, 152)
    };
    mix_rgb(c, flash_col, (0.30 + hit * 0.56).min(0.86))
}

#[inline(always)]
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
        pb.plot(x, y, mix_rgb(bg, c, a));
    }
}

#[inline(always)]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale_rgb(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

#[inline]
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
        darken_px(pb, x + 1, y + 1, 0.30);
        if horiz {
            blend_px(pb, x, y + 1, edge, 0.88);
        } else {
            blend_px(pb, x + 1, y, edge, 0.88);
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

/// Local-space projection and directional-lighting wrapper over `art_enemy::Frame`.
#[derive(Clone, Copy)]
struct Local {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    hit: f32,
    fl: f32,
    pl: f32,
}

impl Local {
    #[inline(always)]
    fn new(fr: &Frame) -> Self {
        Self {
            pos: fr.pos,
            f: fr.f,
            p: fr.p,
            hit: fr.hit,
            fl: fr.f.x * LIGHT.x + fr.f.y * LIGHT.y,
            pl: fr.p.x * LIGHT.x + fr.p.y * LIGHT.y,
        }
    }

    #[inline(always)]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    #[inline(always)]
    fn uv(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline(always)]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    #[inline(always)]
    fn tint(&self, c: Rgb) -> Rgb {
        apply_hit(c, self.hit)
    }

    /// Tight oriented screen bounding box covering local `[u_min, u_max] × [-v_max, v_max]`,
    /// padded on `+x/+y` for drop shadows.
    #[inline(always)]
    fn bbox(
        &self,
        pb: &PixelBuffer,
        u_min: f32,
        u_max: f32,
        v_max: f32,
        pad_x: i32,
        pad_y: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        let u_mid = 0.5 * (u_min + u_max);
        let hu = 0.5 * (u_max - u_min);
        let cx = self.pos.x + self.f.x * u_mid;
        let cy = self.pos.y + self.f.y * u_mid;
        let hx = hu * self.f.x.abs() + v_max * self.p.x.abs();
        let hy = hu * self.f.y.abs() + v_max * self.p.y.abs();
        let x0 = ((cx - hx).floor() as i32).max(0);
        let x1 = ((cx + hx).ceil() as i32 + pad_x).min(pb.w as i32 - 1);
        let y0 = ((cy - hy).floor() as i32).max(0);
        let y1 = ((cy + hy).ceil() as i32 + pad_y).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            None
        } else {
            Some((x0, x1, y0, y1))
        }
    }
}

#[derive(Clone, Copy)]
struct EllipseSdf {
    cu: f32,
    cv: f32,
    inv_ru: f32,
    inv_rv: f32,
    min_r: f32,
}

impl EllipseSdf {
    #[inline(always)]
    fn new(cu: f32, cv: f32, ru: f32, rv: f32) -> Self {
        let ru = ru.max(0.2);
        let rv = rv.max(0.2);
        Self {
            cu,
            cv,
            inv_ru: 1.0 / ru,
            inv_rv: 1.0 / rv,
            min_r: ru.min(rv),
        }
    }

    #[inline(always)]
    fn eval(&self, u: f32, v: f32) -> f32 {
        let nu = (u - self.cu) * self.inv_ru;
        let nv = (v - self.cv) * self.inv_rv;
        ((nu * nu + nv * nv).sqrt() - 1.0) * self.min_r
    }
}

#[derive(Clone, Copy)]
struct TaperSdf {
    au: f32,
    av: f32,
    bax: f32,
    bay: f32,
    inv_denom: f32,
    r0: f32,
    dr: f32,
}

impl TaperSdf {
    #[inline(always)]
    fn new(a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> Self {
        let bax = b.0 - a.0;
        let bay = b.1 - a.1;
        let inv_denom = 1.0 / (bax * bax + bay * bay).max(1e-6);
        Self {
            au: a.0,
            av: a.1,
            bax,
            bay,
            inv_denom,
            r0,
            dr: r1 - r0,
        }
    }

    #[inline(always)]
    fn eval(&self, u: f32, v: f32) -> f32 {
        let pax = u - self.au;
        let pay = v - self.av;
        let h = ((pax * self.bax + pay * self.bay) * self.inv_denom).clamp(0.0, 1.0);
        let dx = pax - self.bax * h;
        let dy = pay - self.bay * h;
        let rad = self.r0 + self.dr * h;
        (dx * dx + dy * dy).sqrt() - rad
    }
}

#[inline(always)]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let diff = (a - b).abs();
    if diff >= k || k <= 1e-5 {
        a.min(b)
    } else {
        let h = (k - diff) / k;
        a.min(b) - h * h * k * 0.25
    }
}

/// Subtle rotating perimeter pips for an elite body (matches the Brute/W3 elite grammar).
fn draw_elite_aura(pb: &mut PixelBuffer, fr: &Frame, extra_r: f32) {
    let pulse = 0.55 + 0.45 * (fr.anim * 1.4).sin();
    let aura_r = fr.r + extra_r;
    let gold = scale_rgb(ELITE_AURA, 0.56 + 0.36 * pulse);
    for i in 0..8 {
        let a0 = i as f32 * (TAU / 8.0) + fr.anim * 0.14;
        let mut last = (i32::MIN, i32::MIN);
        for step in 0..4 {
            let (sa, ca) = (a0 + step as f32 * 0.08).sin_cos();
            let tx = (fr.pos.x + ca * aura_r).round() as i32;
            let ty = (fr.pos.y + sa * aura_r).round() as i32;
            if (tx, ty) != last {
                blend_px(pb, tx, ty, gold, 0.72);
                last = (tx, ty);
            }
        }
    }
}

/// Draw a sculpted 3×3 brass coat button at `(cx, cy)`. Always plots exact `BRASS`
/// on the face when `hit == 0` so the coat buttons glint cleanly from any side.
fn draw_brass_button(pb: &mut PixelBuffer, cx: i32, cy: i32, hit: f32) {
    let dark = apply_hit(BRASS_DARK, hit);
    let mid = apply_hit(BRASS, hit);
    let hi = apply_hit(BRASS_HI, hit);
    let rim = apply_hit(LINT_RIM, hit);
    // Dark felt recess shadow underneath the button.
    darken_px(pb, cx + 1, cy + 2, 0.45);
    darken_px(pb, cx + 2, cy + 1, 0.45);
    // Outer brass bevel ring + bright specular upper-left corner.
    pb.plot(cx - 1, cy - 1, hi);
    pb.plot(cx, cy - 1, mid);
    pb.plot(cx + 1, cy - 1, dark);
    pb.plot(cx - 1, cy, mid);
    pb.plot(cx, cy, rim); // central sewn thread hole
    pb.plot(cx + 1, cy, dark);
    pb.plot(cx - 1, cy + 1, dark);
    pb.plot(cx, cy + 1, dark);
    pb.plot(cx + 1, cy + 1, dark);
}

// ---- Wool Pill --------------------------------------------------------------

/// Pale round tuft on a trailing stitch. Gathers into a plump scalloped puff
/// and stretches forward like an inchworm; during its 0.15 s stall it bunches
/// wide and snaps its trailing yarn taut and bright.
pub(crate) fn draw_wool_pill(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let lf = Local::new(fr);
    let k = fr.r / 2.5;
    let gather = rules::wool_pill_gather(e);
    let stalled = rules::wool_pill_paused(e);
    let gait = (fr.anim * 1.15).sin(); // -1 (gathered) .. +1 (stretched)
    let stretch = if stalled { 0.0 } else { gait.max(0.0) };
    let bunch = if stalled {
        0.55 + 0.45 * gather
    } else {
        (-gait).max(0.0) * 0.45
    };

    if elite {
        draw_elite_aura(pb, fr, 2.2 * k);
    }

    // 1) Trailing yarn stitch: wavy spun thread with knot while walking;
    // pulled ramrod straight, longer, and glowing ivory-gold while stalled.
    let start_u = -(2.0 + 1.1 * stretch) * k;
    let tail_len = (if stalled { 6.6 } else { 5.2 }) * k;
    let steps = (tail_len * 2.0).ceil() as i32;
    let strand_base = lf.tint(if stalled {
        mix_rgb(WOOL_STRAND_TAUT, WOOL_HI, 0.35 + 0.65 * gather)
    } else {
        WOOL_STRAND
    });
    let strand_ply = lf.tint(WOOL_HI);
    let strand_dark = lf.tint(WOOL_DARK);
    let mut prev: Option<(i32, i32)> = None;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let u = start_u - t * tail_len;
        let v = if stalled {
            0.0
        } else {
            (t * 5.2 + fr.anim * 1.6).sin() * (0.65 + 0.65 * t) * k
        };
        let p = lf.px(u, v);
        if prev != Some(p) {
            darken_px(pb, p.0 + 1, p.1 + 1, 0.28);
            let col = if stalled {
                strand_base
            } else if i % 4 == 0 {
                strand_ply
            } else if i % 4 == 2 {
                strand_dark
            } else {
                strand_base
            };
            pb.plot(p.0, p.1, col);
            prev = Some(p);
        }
    }
    // Snag-knot at the far end of the trailing stitch.
    let knot_v = if stalled {
        0.0
    } else {
        (5.2 + fr.anim * 1.6).sin() * 1.3 * k
    };
    let knot = lf.px(start_u - tail_len, knot_v);
    if stalled {
        pb.plot(knot.0, knot.1, lf.tint(WOOL_HI));
        blend_px(pb, knot.0 - 1, knot.1, WOOL_STRAND_TAUT, 0.65);
        blend_px(pb, knot.0 + 1, knot.1, WOOL_STRAND_TAUT, 0.65);
    } else {
        pb.plot(knot.0, knot.1, strand_dark);
        let knot2 = lf.px(start_u - tail_len - 0.8 * k, knot_v + 0.5 * k);
        blend_px(pb, knot2.0, knot2.1, strand_base, 0.85);
    }

    // 2) Tiny alternating curled yarn-loop "feet" paddling on the flanks when walking.
    if !stalled {
        let foot_sw = gait * 0.9 * k;
        let foot_col = lf.tint(WOOL_DARK);
        let foot_hi = lf.tint(WOOL_STRAND);
        for (side, sw) in [(-1.0f32, foot_sw), (1.0, -foot_sw)] {
            let root = lf.px(0.2 * k, side * 1.9 * k);
            let tip = lf.px((0.5 + sw) * k, side * (3.1 - 0.3 * stretch) * k);
            line_blend(pb, root, tip, foot_col, 0.85);
            pb.plot(tip.0, tip.1, foot_hi);
        }
    }

    // 3) Sculpted scalloped wool-tuft body (front puff + rear gather lobe + yarn texture).
    let front_u = (0.35 + 1.35 * stretch) * k;
    let rear_u = (-0.55 - 1.25 * stretch) * k;
    let front_sdf = EllipseSdf::new(
        front_u,
        0.0,
        (2.35 - 0.25 * bunch) * k,
        (2.30 - 0.35 * stretch + 0.72 * bunch) * k,
    );
    let rear_sdf = EllipseSdf::new(
        rear_u,
        0.0,
        (1.85 - 0.15 * bunch) * k,
        (1.95 - 0.35 * stretch + 0.52 * bunch) * k,
    );
    let inv_k = 1.0 / k.max(0.2);
    let body_sd = |u: f32, v: f32| -> f32 {
        let base = smin(front_sdf.eval(u, v), rear_sdf.eval(u, v), 0.9 * k);
        // Soft woolly pom-pom scallops around the silhouette.
        let fluff = 0.28 * k * ((u * inv_k * 2.15).sin() * (v * inv_k * 2.15).cos());
        base - fluff
    };

    let Some((x0, x1, y0, y1)) = lf.bbox(pb, -4.6 * k, 4.9 * k, 4.2 * k, 1, 2) else {
        return;
    };
    let rim_col = if fr.hit > 0.4 { (170, 148, 112) } else { WOOL_RIM };
    let rim_lim = 0.85 * k;
    let sh_lim = 0.35 * k;
    let du_sh = -(fr.f.x + fr.f.y);
    let dv_sh = -(fr.p.x + fr.p.y);
    let inv_nu = 1.0 / (3.1 * k);
    let inv_nv = 1.0 / (2.7 * k);

    for y in y0..=y1 {
        let (mut u, mut v) = lf.uv(x0, y);
        for x in x0..=x1 {
            let d = body_sd(u, v);
            let cover = (0.50 - d * 1.18).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = (u * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.48 + nv * nv * 0.65)).clamp(0.0, 1.0);
                let lit = lf.lit(nu, nv);
                // Diagonal spun-yarn twist grain.
                let ply = ((u + v) * inv_k * 2.2).sin() * 0.08;
                let mut lum = 0.44 + 0.36 * dome + 0.56 * lit + ply;
                if d > -0.75 * k && lit > 0.05 {
                    lum += 0.18; // bright woolly rim in pooled light
                }
                if stalled {
                    lum += 0.10 * gather;
                }
                let col = lf.tint(shade3(WOOL_DARK, WOOL_MID, WOOL_HI, lum));
                if cover >= 1.0 {
                    pb.plot(x, y, col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.62 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.62));
            } else if body_sd(u + du_sh, v + dv_sh) < sh_lim {
                darken_px(pb, x, y, 0.34);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 4) Forward wool-tuft eyes: twin dark-socketed warm amber bead eyes (read in the dark).
    let eye_u = (front_u + 1.15 * k).min(3.2 * k);
    let eye_glint = if fr.hit > 0.65 { WOOL_EYE_DARK } else { WOOL_EYE_GLINT };
    for side in [-1.0f32, 1.0] {
        let brow = lf.px(eye_u - 0.55 * k, side * 0.90 * k);
        let ep = lf.px(eye_u + 0.15 * k, side * 0.85 * k);
        pb.plot(brow.0, brow.1, WOOL_EYE_DARK);
        pb.plot(ep.0, ep.1, eye_glint);
    }

    if elite {
        let b = lf.px(0.0, 0.0);
        pb.plot(b.0, b.1, lf.tint(BRASS));
    }
}

pub(crate) fn draw_wool_pill_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let lf = Local::new(fr);
    let stalled = rules::wool_pill_paused(e);
    let c = lf.px(0.0, 0.0);
    // Trailing 3-pixel stitch (wavy while walking, straight & bright while stalled).
    for (n, d) in [2.4f32, 3.6, 4.8].into_iter().enumerate() {
        let v = if stalled {
            0.0
        } else if n == 1 {
            0.9
        } else {
            -0.5
        };
        let p = lf.px(-d, v);
        pb.plot(p.0, p.1, lf.tint(if stalled { WOOL_HI } else { WOOL_STRAND }));
    }
    // Compact scalloped tuft with dark rim & bright lit crown.
    darken_px(pb, c.0 + 1, c.1 + 2, 0.35);
    pb.plot(c.0, c.1, lf.tint(WOOL_MID));
    pb.plot(c.0 - 1, c.1, lf.tint(WOOL_HI));
    pb.plot(c.0, c.1 - 1, lf.tint(WOOL_HI));
    pb.plot(c.0 + 1, c.1, lf.tint(WOOL_DARK));
    pb.plot(c.0, c.1 + 1, lf.tint(WOOL_DARK));
    pb.plot(c.0 + 1, c.1 + 1, WOOL_RIM);
    let eye = lf.px(1.2, -0.5);
    pb.plot(eye.0, eye.1, WOOL_EYE_GLINT);
}

// ---- Clothes Moth -----------------------------------------------------------

/// How wide the shawl is right now, 0 (shut) to 1 (open).
#[inline]
fn shawl_open(fr: &Frame) -> f32 {
    0.5 + 0.5 * (fr.anim * 0.95).sin()
}

/// Broad woven triangular shawl that opens and shuts over a blunt dark head:
/// scalloped moth-eaten trailing hem, pleated wool folds, ivory hem border,
/// feathery antennae, and golden pin-eyes.
pub(crate) fn draw_clothes_moth(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let lf = Local::new(fr);
    let k = fr.r / 2.0;
    let open = shawl_open(fr);
    let flap = open * 2.0 - 1.0; // -1..1
    let half = (2.0 + 3.7 * open) * k;
    let u_head = 1.65 * k;
    let len = 6.5 * k;
    let u_back = u_head - len;

    if elite {
        draw_elite_aura(pb, fr, 2.6 * k);
    }

    // 1) Elevated flying shadow shifted down-right (so the moth hovers above the tiles).
    let scx = fr.pos.x.round() as i32 + (2.0 * k).round() as i32;
    let scy = fr.pos.y.round() as i32 + (3.1 * k + 0.4 * flap * k).round() as i32;
    let srx = (1.8 + 1.2 * open) * k;
    let sry = 1.45 * k;
    let inv_srx = 1.0 / srx;
    let inv_sry = 1.0 / sry;
    let sx0 = (scx - srx.ceil() as i32).max(0);
    let sx1 = (scx + srx.ceil() as i32).min(pb.w as i32 - 1);
    let sy0 = (scy - sry.ceil() as i32).max(0);
    let sy1 = (scy + sry.ceil() as i32).min(pb.h as i32 - 1);
    for y in sy0..=sy1 {
        let ny = (y - scy) as f32 * inv_sry;
        for x in sx0..=sx1 {
            let nx = (x - scx) as f32 * inv_srx;
            let r2 = nx * nx + ny * ny;
            if r2 <= 1.0 {
                darken_px(pb, x, y, 0.34 * (1.0 - r2 * 0.45));
            }
        }
    }

    // 2) Sweeping woven triangular shawl + blunt dark furry head.
    let head_sdf = EllipseSdf::new(2.2 * k, 0.0, 1.05 * k, 1.42 * k);
    let side_norm = len / (len * len + half * half).sqrt().max(0.5);
    let inv_len = 1.0 / len;
    let inv_half = 1.0 / half.max(0.5);
    let inv_k = 1.0 / k.max(0.2);

    let Some((x0, x1, y0, y1)) = lf.bbox(pb, u_back - 1.6 * k, 4.0 * k, half + 1.5 * k, 0, 0)
    else {
        return;
    };
    let rim_col = if fr.hit > 0.4 { (160, 134, 102) } else { MOTH_RIM };
    let rim_lim = 0.85 * k;

    for y in y0..=y1 {
        let (mut u, mut v) = lf.uv(x0, y);
        for x in x0..=x1 {
            let av = v.abs();
            let d_head = head_sdf.eval(u, v);

            // Cloak-like triangular shawl flaring back from the blunt head.
            let t = ((u_head - u) * inv_len).clamp(0.0, 1.0);
            let t_curve = t * (1.18 - 0.18 * t);
            let w_at_u = 0.85 * k + (half - 0.85 * k) * t_curve;
            let vn = (av * inv_half).min(1.2);
            // Gently scalloped hem: trailing outer corners sweep slightly back, center curves in.
            let u_hem = u_back + (0.42 - 0.78 * vn * vn) * k;
            let du_front = u - u_head;
            let du_back = u_hem - u;
            let dv_side = (av - w_at_u) * side_norm;
            let mut d_shawl = du_front.max(du_back).max(dv_side);

            // Moth-eaten semicircular bite notch on each side of the trailing hem when open.
            if open > 0.25 && u < u_back + 1.6 * k {
                let bu = u - (u_hem - 0.15 * k);
                let bv = av - 0.54 * half;
                let bite_r = (0.72 * open + 0.15) * k;
                let d_bite = bite_r - (bu * bu + bv * bv).sqrt();
                d_shawl = d_shawl.max(d_bite);
            }

            let d_all = d_head.min(d_shawl);
            if d_all >= rim_lim {
                u += fr.f.x;
                v += fr.p.x;
                continue;
            }

            let head_cov = (0.50 - d_head * 1.2).clamp(0.0, 1.0);
            let shawl_cov = (0.48 - d_shawl * 1.18).clamp(0.0, 1.0);
            let cover = head_cov.max(shawl_cov);

            if cover <= 0.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.58));
            } else if head_cov >= shawl_cov {
                let nu = ((u - 2.2 * k) / (1.1 * k)).clamp(-1.0, 1.0);
                let nv = (v / (1.4 * k)).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.5 + nv * nv * 0.65)).clamp(0.0, 1.0);
                let lum = 0.36 + 0.34 * dome + 0.58 * lf.lit(nu, nv);
                let col = lf.tint(shade3(MOTH_HEAD_DARK, MOTH_HEAD_MID, MOTH_HEAD_HI, lum));
                if head_cov >= 1.0 {
                    pb.plot(x, y, col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.58 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, col, head_cov));
                }
            } else {
                // Woven shawl shading: radial cloth pleats + directional lighting + ivory hem trim.
                let wing_side = if v >= 0.0 { 0.65 } else { -0.65 };
                let lit = lf.lit(0.22, wing_side);
                let ray = (av / (w_at_u.max(0.5))).clamp(0.0, 1.0);
                let pleat = (ray * 8.5 + open * 1.2).cos() * 0.10;
                let mut lum = 0.44 + 0.42 * lit + pleat + 0.14 * (1.0 - t);
                // Bright lit leading edge rim so the cloak edge gleams in low light.
                if dv_side > -0.85 * k && lit > -0.1 {
                    lum += 0.24;
                }
                let mut col = shade3(MOTH_DARK, MOTH_MID, MOTH_HI, lum);

                // Pale woven ivory border running just inside the trailing hem.
                let hem_dist = u - u_hem;
                if hem_dist >= 0.35 * k && hem_dist <= 1.25 * k {
                    col = mix_rgb(col, MOTH_LACE, 0.68);
                } else if hem_dist < 0.35 * k {
                    // Dark tasseled fringe right on the outer edge.
                    col = mix_rgb(col, MOTH_DARK, 0.65);
                }

                // Deep central cloak crease when folded/closing.
                let crease_w = (0.55 - 0.25 * open) * k;
                if av < crease_w && t > 0.18 {
                    col = mix_rgb(col, MOTH_DARK, 0.72 - 0.32 * open);
                }

                // Furry ivory collar ruff right behind the blunt dark head.
                if t < 0.18 && av < 1.5 * k {
                    col = mix_rgb(col, MOTH_LACE, 0.55);
                }

                // Tiny moth-eaten pinholes inside the open fabric.
                if open > 0.35 && t > 0.38 && t < 0.78 && av > 0.9 * k {
                    let hu = ((u * inv_k * 0.85).round() - u * inv_k * 0.85).abs();
                    let hv = ((v * inv_k * 0.85).round() - v * inv_k * 0.85).abs();
                    if hu < 0.18 && hv < 0.18 && ((u * inv_k) as i32 + (v * inv_k) as i32).rem_euclid(3) == 0 {
                        col = MOTH_HOLE;
                    }
                }

                let lit_col = lf.tint(col);
                if shawl_cov >= 1.0 {
                    pb.plot(x, y, lit_col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.58 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, lit_col, shawl_cov));
                }
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Connected feathery swept antennae + piercing golden-amber pin-eyes on the blunt head.
    let ant_col = lf.tint(MOTH_ANT);
    let ant_sw = flap * 0.35 * k;
    let eye_col = if fr.hit > 0.7 { MOTH_HEAD_DARK } else { MOTH_EYE };
    for side in [-1.0f32, 1.0] {
        let a0 = lf.px(2.6 * k, side * 0.7 * k);
        let a1 = lf.px(3.6 * k, side * (1.5 + ant_sw) * k);
        let a2 = lf.px(4.3 * k, side * (2.2 + ant_sw * 1.4) * k);
        line_blend(pb, a0, a1, ant_col, 0.90);
        line_blend(pb, a1, a2, lf.tint(MOTH_HI), 0.75);
        let ep = lf.px(2.45 * k, side * 0.92 * k);
        pb.plot(ep.0, ep.1, eye_col);
    }

    if elite {
        let b = lf.px(0.0, 0.0);
        pb.plot(b.0, b.1, lf.tint(BRASS));
    }
}

pub(crate) fn draw_clothes_moth_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let lf = Local::new(fr);
    let open = shawl_open(fr) > 0.5;
    let (dark, mid, hi, lace) = (
        lf.tint(MOTH_DARK),
        lf.tint(MOTH_MID),
        lf.tint(MOTH_HI),
        lf.tint(MOTH_LACE),
    );
    // Soft hovering shadow dot.
    let sh = lf.px(-0.5, 0.0);
    darken_px(pb, sh.0 + 1, sh.1 + 2, 0.32);

    // Triangular shawl: apex at collar, widening to a clear trimmed hem behind.
    let mid_p = lf.px(0.2, 0.0);
    pb.plot(mid_p.0, mid_p.1, hi);
    let w1: i32 = if open { 2 } else { 1 };
    for iv in -w1..=w1 {
        let p = lf.px(-1.0, iv as f32);
        let col = if iv < 0 {
            hi
        } else if iv == 0 {
            mid
        } else {
            dark
        };
        pb.plot(p.0, p.1, col);
    }
    let w2: i32 = if open { 3 } else { 1 };
    for iv in -w2..=w2 {
        let p = lf.px(-2.4, iv as f32);
        let col = if iv.abs() == w2 {
            dark
        } else if open && iv.abs() == 1 {
            lace
        } else {
            mid
        };
        pb.plot(p.0, p.1, col);
    }
    // Blunt dark head + bright golden eye glint forward.
    let head = lf.px(1.5, 0.0);
    pb.plot(head.0, head.1, lf.tint(MOTH_HEAD_DARK));
    let eye = lf.px(1.8, -0.6);
    pb.plot(eye.0, eye.1, MOTH_EYE);
}

// ---- Peg Snapper ------------------------------------------------------------

/// The floor jaws: two `16 × 3` bars drawn from the exact same `JawView`
/// geometry the hit test reads (`PEG_JAW_LEN × PEG_JAW_THICK` around each jaw
/// centre in `view.jaws()`). Hollow serrated amber while warning, blazing
/// crimson-white while sliding shut, beveled wood while resting shut.
fn draw_floor_jaws(pb: &mut PixelBuffer, view: &rules::JawView, cam: Vec2) {
    let side = view.axis.perp();
    let half_len = rules::PEG_JAW_LEN * 0.5;
    let half_thick = rules::PEG_JAW_THICK * 0.5;
    let warn_frac = (view.t / rules::PEG_TELL_SECS).clamp(0.0, 1.0);
    let pulse_rate = 9.0 + warn_frac * 9.0;
    let pulse = ((view.t * pulse_rate) as i32) & 1 == 0;

    for (jaw_idx, c) in view.jaws().into_iter().enumerate() {
        // jaw_idx 0 sits at +side * offset, so -side faces the centre; jaw_idx 1 is opposite.
        let inward_sign = if jaw_idx == 0 { -1.0f32 } else { 1.0f32 };
        let nu = (half_len * 2.0) as i32;
        let nv = (half_thick * 2.0).ceil() as i32;
        for iu in -nu..=nu {
            for iv in -nv..=nv {
                let u = iu as f32 * 0.5;
                let v = (iv as f32 * 0.5).clamp(-half_thick, half_thick);
                let w = c + view.axis * u + side * v - cam;
                let (x, y) = (w.x.round() as i32, w.y.round() as i32);
                let end_rim = u.abs() > half_len - 0.85;
                let side_rim = v.abs() > half_thick - 0.62;
                let rim = end_rim || side_rim;
                let inner_edge = v * inward_sign > half_thick - 0.65;
                let tooth = inner_edge && (((u * 0.8).round() as i32) & 1 == 0);
                match view.phase {
                    rules::PEG_PHASE_TELL => {
                        if tooth {
                            pb.plot(x, y, if pulse || warn_frac > 0.65 { JAW_WARN_HOT } else { JAW_WARN });
                        } else if rim {
                            let col = if pulse || (end_rim && warn_frac > 0.6) {
                                JAW_WARN_HOT
                            } else {
                                JAW_WARN
                            };
                            pb.plot(x, y, col);
                        } else {
                            blend_px(pb, x, y, JAW_WARN, 0.24 + 0.14 * warn_frac);
                        }
                    }
                    rules::PEG_PHASE_CLOSE => {
                        pb.plot(x, y, if rim && !tooth { JAW_LIVE } else { JAW_LIVE_CORE });
                    }
                    _ => {
                        // Resting shut: tactile beveled beechwood jaws with a dark inner bite seam.
                        if inner_edge {
                            blend_px(pb, x, y, PEG_RIM, 0.90);
                        } else if rim {
                            blend_px(pb, x, y, PEG_DARK, 0.88);
                        } else {
                            let grain = if (u * 0.7).sin() > 0.55 { PEG_GRAIN } else { JAW_SHUT };
                            blend_px(pb, x, y, grain, 0.86);
                        }
                    }
                }
            }
        }
    }

    // Warned slide tracks across the open gap (leaving both open ends along ±axis clear).
    if view.phase == rules::PEG_PHASE_TELL {
        for u_track in [-4.5f32, 0.0, 4.5] {
            let mut d = -view.offset + 2.4;
            while d <= view.offset - 2.4 {
                let w = view.centre + view.axis * u_track + side * d - cam;
                let alpha = if u_track == 0.0 { 0.52 } else { 0.30 };
                blend_px(pb, w.x.round() as i32, w.y.round() as i32, JAW_WARN, alpha);
                d += 2.2;
            }
        }
    }
}

/// The hostile-layer stamp (`art_enemy::draw_w3_tells` / `world.rs`): the floor
/// jaws alone, re-drawn after the lighting pass so the warning is self-lit above
/// every friendly effect. Same view, same geometry as the body pass.
pub(crate) fn draw_peg_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if let Some(view) = rules::jaw_view(e) {
        draw_floor_jaws(pb, &view, cam);
    }
}

/// Returns `(jaw_spread, hinge_heat, close_frac, planted)` for the Peg Snapper's
/// clothespin animation.
fn peg_pose(fr: &Frame, e: &Enemy) -> (f32, f32, f32, bool) {
    match rules::jaw_view(e) {
        Some(v) if v.phase == rules::PEG_PHASE_TELL => {
            let wind = (v.t / rules::PEG_TELL_SECS).clamp(0.0, 1.0);
            let shiver = if wind > 0.55 {
                (v.t * 42.0).sin() * 0.25
            } else {
                0.0
            };
            (4.4 + 0.85 * wind + shiver, 0.55 + 0.45 * wind, 0.0, true)
        }
        Some(v) if v.phase == rules::PEG_PHASE_CLOSE => {
            let t = (v.t / rules::PEG_CLOSE_SECS).clamp(0.0, 1.0);
            (5.2 - 4.35 * t, 1.0, t, true)
        }
        Some(_) => (0.85, 0.0, 0.0, true),
        None => {
            // Walking open: brisk clothespin chomp-stride.
            let stride = (fr.anim * 0.95).sin();
            (2.85 + 0.90 * stride, 0.0, 0.0, false)
        }
    }
}

/// Forked wooden clothespin jaw on a gleaming coiled brass hinge. Walks open
/// with alternating wooden feet; rear pinch-handles close in mechanical
/// opposition as the front prongs crank wide during the tell, then snap shut.
pub(crate) fn draw_peg_snapper(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    if let Some(view) = rules::jaw_view(e) {
        draw_floor_jaws(pb, &view, cam);
    }
    let lf = Local::new(fr);
    let k = fr.r / 2.0;
    let (spread, heat, close_frac, planted) = peg_pose(fr, e);

    if elite {
        draw_elite_aura(pb, fr, 2.5 * k);
    }

    // 1) Stub wooden walking feet tucked beneath the fulcrum (planted still during jaws).
    let step = if planted {
        0.0
    } else {
        (fr.anim * 0.95).sin() * 1.15 * k
    };
    let foot_dark = lf.tint(PEG_DARK);
    let foot_mid = lf.tint(PEG_GRAIN);
    for (side, sw) in [(-1.0f32, step), (1.0, -step)] {
        let hip = lf.px(-1.3 * k, side * 1.1 * k);
        let foot = lf.px(-1.9 * k + sw, side * 2.55 * k);
        darken_px(pb, foot.0 + 1, foot.1 + 1, 0.34);
        line_blend(pb, hip, foot, foot_dark, 0.92);
        pb.plot(foot.0, foot.1, foot_mid);
    }

    // 2) Sculpted clothespin SDF: twin forked wooden jaws + rear counter-lever pinch handles
    // + central brass coil spring boss.
    let handle_v = (2.35 - 0.28 * spread).clamp(0.72, 2.35) * k;
    let jaw_sdf = TaperSdf::new((-1.05 * k, 0.90 * k), (5.05 * k, spread * k), 1.02 * k, 0.64 * k);
    let handle_sdf = TaperSdf::new((-1.25 * k, 0.85 * k), (-4.25 * k, handle_v), 0.92 * k, 0.68 * k);
    let coil_sdf = EllipseSdf::new(-1.15 * k, 0.0, 1.32 * k, 1.32 * k);

    let inv_k = 1.0 / k.max(0.2);
    let body_sd = |u: f32, v: f32| -> (f32, bool) {
        let av = v.abs();
        let mut d_jaw = jaw_sdf.eval(u, av);
        // Carve a clothespin circular finger-grip notch on the inward face of each jaw.
        if u > 0.6 * k && u < 3.4 * k {
            let nu = u - 2.0 * k;
            let inner_v = (0.90 + (spread - 0.90) * ((2.0 + 1.05) / 6.10)) * k - 0.72 * k;
            let nv = av - inner_v.max(0.15 * k);
            let notch = 0.58 * k - (nu * nu + nv * nv).sqrt();
            d_jaw = d_jaw.max(notch);
        }
        let d_handle = handle_sdf.eval(u, av);
        let d_wood = smin(d_jaw, d_handle, 0.32 * k);
        let d_coil = coil_sdf.eval(u, v);
        let is_coil = d_coil <= d_wood + 0.12 * k;
        (d_wood.min(d_coil), is_coil)
    };

    let v_bound = (spread.max(2.6) + 1.8) * k;
    let Some((x0, x1, y0, y1)) = lf.bbox(pb, -5.5 * k, 6.2 * k, v_bound, 1, 2) else {
        return;
    };
    let rim_col = if fr.hit > 0.4 { (180, 134, 84) } else { PEG_RIM };
    let rim_lim = 0.85 * k;
    let sh_lim = 0.35 * k;
    let du_sh = -1.2 * (fr.f.x + fr.f.y);
    let dv_sh = -1.2 * (fr.p.x + fr.p.y);

    let coil_mid = mix_rgb(PEG_HINGE, PEG_HINGE_HOT, heat * 0.82);
    let coil_hi = PEG_HINGE_HOT;

    for y in y0..=y1 {
        let (mut u, mut v) = lf.uv(x0, y);
        for x in x0..=x1 {
            let (d, is_coil) = body_sd(u, v);
            let cover = (0.50 - d * 1.22).clamp(0.0, 1.0);
            if cover > 0.0 {
                let col = if is_coil {
                    let nu = ((u + 1.15 * k) / (1.3 * k)).clamp(-1.0, 1.0);
                    let nv = (v / (1.3 * k)).clamp(-1.0, 1.0);
                    let lit = lf.lit(nu, nv);
                    let lum = 0.46 + 0.48 * lit + 0.32 * heat;
                    lf.tint(shade3(PEG_SPRING_DARK, coil_mid, coil_hi, lum))
                } else {
                    let nu = (u / (5.0 * k)).clamp(-1.0, 1.0);
                    let nv = (v / (spread.max(2.0) * k)).clamp(-1.0, 1.0);
                    let lit = lf.lit(nu * 0.45, nv);
                    let mut lum = 0.44 + 0.56 * lit;
                    if u > 3.8 * k {
                        lum += 0.16; // beveled gripping tips catch the light
                    }
                    let mut c = shade3(PEG_DARK, PEG_MID, PEG_HI, lum);
                    // Subtle wood-grain striation running along the wooden arms.
                    if ((v * inv_k * 3.1).sin()).abs() > 0.82 {
                        c = mix_rgb(c, PEG_GRAIN, 0.38);
                    }
                    // Copper spring-wire clasp bands gripping the outside of the wood near the fulcrum.
                    if (u - 0.35 * k).abs() < 0.42 * k && v.abs() > 1.0 * k {
                        c = mix_rgb(coil_mid, coil_hi, 0.35 + 0.4 * heat);
                    }
                    lf.tint(c)
                };
                if cover >= 1.0 {
                    pb.plot(x, y, col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.64 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.64));
            } else if body_sd(u + du_sh, v + dv_sh).0 < sh_lim {
                darken_px(pb, x, y, 0.34);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Bright brass-spring glint at the hinge (pulses white-hot through tell & snap).
    let hinge = lf.px(-1.15 * k, 0.0);
    pb.plot(hinge.0, hinge.1, lf.tint(if heat > 0.1 { PEG_HINGE_HOT } else { PEG_HINGE }));
    if heat > 0.25 {
        let spark = scale_rgb(if heat > 0.75 { PEG_HINGE_HOT } else { PEG_HINGE }, 0.45 + 0.5 * heat);
        pb.plot_add(hinge.0 + 1, hinge.1, spark);
        pb.plot_add(hinge.0 - 1, hinge.1, spark);
        pb.plot_add(hinge.0, hinge.1 + 1, spark);
        pb.plot_add(hinge.0, hinge.1 - 1, spark);
    }
    // Snap-shut impact spark right between the closing tips.
    if close_frac > 0.55 {
        let snap = lf.px(4.9 * k, 0.0);
        pb.plot(snap.0, snap.1, JAW_LIVE_CORE);
        pb.plot_add(snap.0 + 1, snap.1, JAW_LIVE);
        pb.plot_add(snap.0 - 1, snap.1, JAW_LIVE);
    }

    if elite {
        let b = lf.px(0.6 * k, 0.0);
        pb.plot(b.0, b.1, lf.tint(BRASS));
    }
}

pub(crate) fn draw_peg_snapper_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    // The floor jaws are gameplay, so the far body keeps them whole.
    if let Some(view) = rules::jaw_view(e) {
        draw_floor_jaws(pb, &view, cam);
    }
    let lf = Local::new(fr);
    let (spread, heat, _, _) = peg_pose(fr, e);
    let wood_mid = lf.tint(PEG_MID);
    let wood_hi = lf.tint(PEG_HI);
    let wood_dark = lf.tint(PEG_DARK);

    // Compact wooden V-prongs + rear handle stub + bright brass coil fulcrum.
    let hinge = lf.px(-1.0, 0.0);
    let rear = lf.px(-2.6, 0.0);
    line_blend(pb, hinge, rear, wood_dark, 0.90);
    for side in [-1.0f32, 1.0] {
        let a = lf.px(-0.2, side * (spread * 0.25).max(0.5));
        let b = lf.px(2.8, side * (spread * 0.55).max(0.6));
        thick_line(pb, a, b, wood_mid, wood_dark);
        pb.plot(b.0, b.1, wood_hi);
    }
    pb.plot(
        hinge.0,
        hinge.1,
        lf.tint(if heat > 0.1 { PEG_HINGE_HOT } else { PEG_HINGE }),
    );
}

// ---- Starch Bulb ------------------------------------------------------------

/// 0..1 as the shot comes due (`fire_cd` runs down to 0 and holds there while
/// cover blocks the line).
#[inline]
fn bulb_charge(fr: &Frame, e: &Enemy) -> f32 {
    if e.fire_cd <= 0.0 {
        0.84 + 0.16 * (fr.anim * 2.0).sin().abs()
    } else if e.fire_cd < BULB_CHARGE_SECS {
        (1.0 - e.fire_cd / BULB_CHARGE_SECS).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 0..1 brief rebound puff right after firing (`enemy_fire_system` resets `fire_cd` to 2.2).
#[inline]
fn bulb_recoil(e: &Enemy) -> f32 {
    if e.fire_cd > 2.02 {
        ((e.fire_cd - 2.02) / 0.18).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Molded rubber pear on a copper-footed tripod with a thin crooked copper
/// nozzle that rocks while walking. Charging squeezes the pear inward, flushes
/// it starch-white, braces the tripod, and swells a bright bead at the tip.
pub(crate) fn draw_starch_bulb(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let lf = Local::new(fr);
    let k = fr.r / 3.0;
    let charge = bulb_charge(fr, e);
    let recoil = bulb_recoil(e);

    if elite {
        draw_elite_aura(pb, fr, 2.5 * k);
    }

    // 1) Three jointed steel tripod struts with bright copper foot pads so the
    // legs read clearly against dark indigo tiles.
    let gait = (fr.anim * 1.05).sin() * (1.0 - charge * 0.85);
    let brace = 0.45 * charge;
    let leg_col = lf.tint(BULB_LEG);
    let leg_hi = lf.tint(BULB_LEG_HI);
    let foot_col = lf.tint(BULB_FOOT);
    let legs: [(f32, f32, f32, f32); 3] = [
        (-4.5 - brace, 0.0, -gait * 0.55, gait * 0.65),
        (1.3 + brace * 0.4, -4.6 - brace, gait * 1.15, 0.0),
        (1.3 + brace * 0.4, 4.6 + brace, -gait * 1.15, 0.0),
    ];
    for (u, v, du, dv) in legs {
        let hip = lf.px(u * 0.25 * k, v * 0.25 * k);
        let knee = lf.px((u * 0.65 + du * 0.45) * k, (v * 0.72 + dv * 0.45) * k);
        let foot = lf.px((u + du) * k, (v + dv) * k);
        darken_px(pb, foot.0 + 1, foot.1 + 1, 0.34);
        line_blend(pb, hip, knee, leg_hi, 0.94);
        line_blend(pb, knee, foot, leg_col, 0.96);
        pb.plot(foot.0, foot.1, foot_col);
    }

    // 2) Sculpted rubber pear belly + tapered neck + copper ferrule collar.
    let squeeze = 0.24 * charge;
    let puff = 0.14 * recoil;
    let belly_sdf = EllipseSdf::new(
        (-1.35 - 0.30 * recoil) * k,
        0.0,
        (3.15 + 0.22 * charge + 0.35 * recoil) * k,
        (3.15 * (1.0 - squeeze) + 0.45 * puff * 3.0) * k,
    );
    let neck_sdf = EllipseSdf::new(
        1.75 * k,
        0.0,
        1.65 * k,
        (1.50 * (1.0 - squeeze * 0.5) + 0.20 * recoil) * k,
    );

    let body_sd = |u: f32, v: f32| -> f32 { smin(belly_sdf.eval(u, v), neck_sdf.eval(u, v), 0.95 * k) };

    let Some((x0, x1, y0, y1)) = lf.bbox(pb, -5.6 * k, 4.5 * k, 4.2 * k, 1, 2) else {
        return;
    };
    let rim_col = if fr.hit > 0.4 { (160, 172, 194) } else { BULB_RIM };
    let rim_lim = 0.85 * k;
    let sh_lim = 0.35 * k;
    let du_sh = -1.2 * (fr.f.x + fr.f.y);
    let dv_sh = -1.2 * (fr.p.x + fr.p.y);
    let inv_nu = 1.0 / (3.8 * k);
    let inv_nv = 1.0 / (3.1 * k);

    for y in y0..=y1 {
        let (mut u, mut v) = lf.uv(x0, y);
        for x in x0..=x1 {
            let d = body_sd(u, v);
            let cover = (0.50 - d * 1.18).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = ((u + 0.5 * k) * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.46 + nv * nv * 0.68)).clamp(0.0, 1.0);
                let lit = lf.lit(nu, nv);
                let mut col = if u > 2.25 * k {
                    // Burnished copper ferrule collar clamping the pear to the nozzle.
                    let clum = 0.44 + 0.56 * lit + 0.20 * charge;
                    shade3(BULB_COPPER_DARK, BULB_COPPER_MID, BULB_COPPER_HI, clum)
                } else {
                    let mut lum = 0.40 + 0.38 * dome + 0.58 * lit;
                    if lit > 0.38 && dome > 0.34 {
                        lum += 0.28; // glossy vulcanised rubber sheen
                    }
                    let mut c = shade3(BULB_DARK, BULB_MID, BULB_HI, lum);
                    // Molded rubber squeeze-ribs across the pear belly.
                    let r1 = (u + 1.9 * k).abs() < 0.36 * k;
                    let r2 = (u + 0.1 * k).abs() < 0.34 * k;
                    if r1 || r2 {
                        c = mix_rgb(c, BULB_RIB, 0.56 * (1.0 - 0.5 * charge));
                    }
                    if charge > 0.06 {
                        c = mix_rgb(c, rules::STARCH_JET_COLOR, charge * (0.35 + 0.55 * dome));
                    }
                    c
                };
                col = lf.tint(col);
                if cover >= 1.0 {
                    pb.plot(x, y, col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.64 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.64));
            } else if body_sd(u + du_sh, v + dv_sh) < sh_lim {
                darken_px(pb, x, y, 0.34);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // Tiny bright pressurized meniscus/gauge glint on the lit shoulder (survives dark rooms).
    let gauge = lf.px(0.95 * k, -0.85 * k);
    pb.plot(
        gauge.0,
        gauge.1,
        if charge > 0.45 {
            BULB_CHARGE
        } else {
            lf.tint(BULB_GAUGE)
        },
    );

    // 3) Thin crooked copper spout nozzle: two kinked segments that rock while
    // walking, lock straight while charging, and kick back on recoil.
    let rock = (fr.anim * 0.75).sin() * 1.45 * k * (1.0 - charge);
    let kick = 0.75 * recoil * k;
    let n0 = lf.px(2.8 * k, 0.0);
    let n1 = lf.px(5.1 * k - kick * 0.6, 1.45 * k + rock);
    let n2 = lf.px(7.5 * k - kick, 0.15 * k + rock * 0.35);
    let noz_core = lf.tint(mix_rgb(BULB_COPPER_MID, BULB_COPPER_HI, 0.35 + 0.5 * charge));
    let noz_edge = lf.tint(BULB_COPPER_DARK);
    thick_line(pb, n0, n1, noz_core, noz_edge);
    thick_line(pb, n1, n2, lf.tint(BULB_COPPER_HI), noz_edge);

    // 4) Charged starch-jet bead + 4-point sparkle at the crooked tip.
    if charge > 0.08 {
        let bead = mix_rgb(rules::STARCH_JET_COLOR, BULB_CHARGE, charge * 0.75);
        pb.plot(n2.0, n2.1, bead);
        let glow = scale_rgb(rules::STARCH_JET_COLOR, 0.48 + 0.52 * charge);
        if charge > 0.35 {
            pb.plot_add(n2.0 + 1, n2.1, glow);
            pb.plot_add(n2.0 - 1, n2.1, glow);
            pb.plot_add(n2.0, n2.1 + 1, glow);
            pb.plot_add(n2.0, n2.1 - 1, glow);
        }
        if charge > 0.72 {
            let tip = scale_rgb(bead, 0.62);
            pb.plot_add(n2.0 + 2, n2.1, tip);
            pb.plot_add(n2.0 - 2, n2.1, tip);
            pb.plot_add(n2.0, n2.1 + 2, tip);
            pb.plot_add(n2.0, n2.1 - 2, tip);
        }
    }

    if elite {
        let b = lf.px(-1.2 * k, 0.0);
        pb.plot(b.0, b.1, lf.tint(BRASS));
    }
}

pub(crate) fn draw_starch_bulb_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let lf = Local::new(fr);
    let charge = bulb_charge(fr, e);
    let (dark, mid, hi) = (lf.tint(BULB_DARK), lf.tint(BULB_MID), lf.tint(BULB_HI));
    let c = lf.px(-0.6, 0.0);
    darken_px(pb, c.0 + 1, c.1 + 2, 0.34);

    // Three copper-footed tripod leg ticks.
    for (u, v) in [(-3.2f32, 0.0f32), (1.0, -2.8), (1.0, 2.8)] {
        let p = lf.px(u, v);
        pb.plot(p.0, p.1, lf.tint(BULB_FOOT));
    }
    // Compact rubber pear core with lit upper-left rim + dark edge.
    let body_c = if charge > 0.3 {
        lf.tint(mix_rgb(BULB_MID, rules::STARCH_JET_COLOR, charge * 0.75))
    } else {
        mid
    };
    pb.plot(c.0, c.1, body_c);
    pb.plot(c.0 - 1, c.1, hi);
    pb.plot(c.0, c.1 - 1, hi);
    pb.plot(c.0 + 1, c.1, dark);
    pb.plot(c.0, c.1 + 1, dark);
    pb.plot(c.0 - 1, c.1 + 1, body_c);
    let neck = lf.px(1.2, 0.0);
    pb.plot(neck.0, neck.1, lf.tint(BULB_COPPER_MID));

    // Crooked copper nozzle + full charge sparkle so the tell survives far zoom.
    let elbow = lf.px(2.4, 0.9);
    let tip = lf.px(3.8, 0.1);
    pb.plot(elbow.0, elbow.1, lf.tint(BULB_COPPER_MID));
    pb.plot(
        tip.0,
        tip.1,
        if charge > 0.1 {
            mix_rgb(rules::STARCH_JET_COLOR, BULB_CHARGE, charge)
        } else {
            lf.tint(BULB_COPPER_HI)
        },
    );
    if charge > 0.45 {
        let glow = scale_rgb(rules::STARCH_JET_COLOR, 0.65);
        pb.plot_add(tip.0 + 1, tip.1, glow);
        pb.plot_add(tip.0 - 1, tip.1, glow);
        pb.plot_add(tip.0, tip.1 + 1, glow);
        pb.plot_add(tip.0, tip.1 - 1, glow);
    }
}

// ---- Lint Golem -------------------------------------------------------------

/// Broad uneven shoulders and a dark 4-hole dryer-button eye; shambles with
/// alternating heavy steps. Pressed-felt lint texture with trapped stray
/// pink-sock and denim threads. The elite wears two gleaming brass coat buttons
/// down its chest/spine and a thick double dark stripe across both shoulders.
pub(crate) fn draw_lint_golem(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let lf = Local::new(fr);
    let k = fr.r / 6.5;
    let stomp = fr.anim * 0.65;
    let lead = stomp.sin() * 1.55 * k;
    let hp_frac = if e.max_hp > 0.0 {
        (e.hp / e.max_hp).clamp(0.0, 1.0)
    } else {
        1.0
    };

    if elite {
        draw_elite_aura(pb, fr, 2.3 * k);
    }

    // 1) Heavy lumbering felt feet / dragged knuckles stepping out from beneath the shoulders.
    let foot_dark = lf.tint(LINT_DARK);
    let foot_mid = lf.tint(LINT_MID);
    for (u_base, v_base, sw) in [(1.4f32, -5.8f32, lead * 1.25), (0.6, 5.1, -lead * 1.15)] {
        let root = lf.px(u_base * k, v_base * 0.72 * k);
        let pad = lf.px(u_base * k + sw, v_base * k);
        darken_px(pb, pad.0 + 1, pad.1 + 2, 0.36);
        thick_line(pb, root, pad, foot_mid, foot_dark);
        pb.plot(pad.0, pad.1, lf.tint(LINT_HI));
    }

    // 2) Broad uneven shoulders + bulky matted torso + low hunched head lump.
    let left_u = 0.75 * k + lead;
    let left_v = -5.0 * k;
    let right_u = -0.35 * k - lead * 0.85;
    let right_v = 4.35 * k;

    let big_shoulder = EllipseSdf::new(left_u, left_v, 3.7 * k, 4.1 * k);
    let small_shoulder = EllipseSdf::new(right_u, right_v, 2.75 * k, 2.95 * k);
    let torso_sdf = EllipseSdf::new(-1.15 * k, -0.3 * k, 4.55 * k, 4.85 * k);
    let head_sdf = EllipseSdf::new(3.55 * k, 0.45 * k, 2.55 * k, 2.75 * k);
    let inv_k = 1.0 / k.max(0.2);

    let body_sd = |u: f32, v: f32| -> f32 {
        let s1 = smin(torso_sdf.eval(u, v), big_shoulder.eval(u, v), 1.35 * k);
        let s2 = smin(s1, small_shoulder.eval(u, v), 1.15 * k);
        let base = smin(s2, head_sdf.eval(u, v), 0.95 * k);
        // Irregular fuzzy lint-mat perimeter so it reads as shaggy compressed felt.
        let fuzz = 0.38 * k * ((u * inv_k * 1.15 + v * inv_k * 0.65).sin() * (v * inv_k * 1.25 - u * inv_k * 0.55).cos());
        base - fuzz
    };

    let Some((x0, x1, y0, y1)) = lf.bbox(pb, -7.2 * k, 7.6 * k, 10.3 * k, 2, 2) else {
        return;
    };
    let rim_col = if fr.hit > 0.4 { (165, 160, 152) } else { LINT_RIM };
    let rim_lim = 0.95 * k;
    let sh_lim = 0.45 * k;
    let du_sh = -1.8 * (fr.f.x + fr.f.y);
    let dv_sh = -1.8 * (fr.p.x + fr.p.y);
    let inv_nu = 1.0 / (6.0 * k);
    let inv_nv = 1.0 / (7.5 * k);
    let damaged = hp_frac < 0.65;
    let critical = hp_frac < 0.35;

    for y in y0..=y1 {
        let (mut u, mut v) = lf.uv(x0, y);
        for x in x0..=x1 {
            let d = body_sd(u, v);
            let cover = (0.50 - d * 1.15).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = ((u + 0.3 * k) * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.50 + nv * nv * 0.62)).clamp(0.0, 1.0);
                let lit = lf.lit(nu, nv);
                let strata = ((u * inv_k * 0.85 + v * inv_k * 0.35) * 1.5).sin();
                let mut lum = 0.36 + 0.38 * dome + 0.60 * lit + strata * 0.08;
                // Catch pooled light along the fuzzy upper-left wool silhouette edge.
                if d > -1.05 * k && lit > 0.12 {
                    lum += 0.24;
                }
                let mut col = shade3(LINT_DARK, LINT_MID, LINT_HI, lum);
                if d > -0.95 * k && lit > 0.30 {
                    col = mix_rgb(col, LINT_TUFT, 0.52);
                }

                // Elite thick double shoulder stripe on BOTH uneven shoulders:
                // two bold dark bands framing a crisp bright lint groove.
                if elite {
                    let on_big = (u - left_u).abs() < 3.1 * k && (v - left_v).abs() < 2.6 * k;
                    let on_small = (u - right_u).abs() < 2.3 * k && (v - right_v).abs() < 2.1 * k;
                    if on_big {
                        let dv = (v - left_v).abs();
                        if (dv - 1.35 * k).abs() < 0.58 * k {
                            col = ELITE_STRIPE;
                        } else if dv < 0.72 * k {
                            col = mix_rgb(col, LINT_TUFT, 0.65);
                        }
                    } else if on_small {
                        let dv = (v - right_v).abs();
                        if (dv - 1.10 * k).abs() < 0.52 * k {
                            col = ELITE_STRIPE;
                        } else if dv < 0.56 * k {
                            col = mix_rgb(col, LINT_TUFT, 0.65);
                        }
                    }
                }

                // Frayed felt tears as the golem takes damage.
                if damaged {
                    let tear1 = ((u + 1.4 * k) - (v + 1.2 * k) * 0.6).abs() < 0.42 * k
                        && u < 1.2 * k
                        && v < -0.6 * k;
                    let tear2 = ((u + 2.6 * k) + (v - 1.4 * k) * 0.7).abs() < 0.42 * k
                        && u < 0.0
                        && v > 0.5 * k;
                    if tear1 || (critical && tear2) {
                        col = mix_rgb(LINT_RIM, LINT_SOCK_PINK, if critical { 0.45 } else { 0.15 });
                    }
                }

                let lit_col = lf.tint(col);
                if cover >= 1.0 {
                    pb.plot(x, y, lit_col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.66 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, lit_col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.66));
            } else if body_sd(u + du_sh, v + dv_sh) < sh_lim {
                darken_px(pb, x, y, 0.36);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Trapped laundry lint flecks (ivory wisps, ruined-wash sock pink, faded denim)
    // fixed in local `(u, v)` so they ride the shambling body.
    for (u, v, col) in [
        (-2.6f32, -2.0f32, LINT_TUFT),
        (1.4, -6.6, LINT_TUFT),
        (-3.6, 1.4, LINT_TUFT),
        (0.4, 5.2, LINT_TUFT),
        (-1.4, 2.5, LINT_SOCK_PINK),
        (1.6, -3.0, LINT_SOCK_PINK),
        (-3.8, -3.4, LINT_SOCK_PINK),
        (-0.8, -5.8, LINT_DENIM),
        (-2.8, 3.0, LINT_DENIM),
        (2.0, 2.2, LINT_DENIM),
    ] {
        let p = lf.px(u * k, v * k);
        pb.plot(p.0, p.1, lf.tint(col));
    }

    // 4) Iconic dark bakelite 4-hole dryer-button eye on the low hunched head:
    // dark recessed disc, beveled rim, pale stitched thread holes, and a crisp
    // upper-left specular glint that reads even in dark rooms.
    let eye = lf.px(4.15 * k, 0.50 * k);
    let er = (2.0 * k).round().max(1.0) as i32;
    let er2 = er * er;
    let inner2 = (er - 1).max(0) * (er - 1).max(0);
    for dy in -er..=er {
        for dx in -er..=er {
            let d2 = dx * dx + dy * dy;
            if d2 > er2 {
                continue;
            }
            let c = if d2 <= inner2 && er > 1 {
                LINT_EYE
            } else if dx + dy < 0 {
                lf.tint(LINT_EYE_RIM)
            } else {
                LINT_RIM
            };
            pb.plot(eye.0 + dx, eye.1 + dy, c);
        }
    }
    // Glossy bakelite rim glint upper-left + tiny stitched thread cross inside the button.
    pb.plot(eye.0 - 1, eye.1 - 1, LINT_EYE_GLINT);
    pb.plot(eye.0, eye.1, lf.tint(LINT_EYE_THREAD));
    if er >= 2 {
        pb.plot(eye.0 + 1, eye.1, LINT_EYE);
        pb.plot(eye.0, eye.1 + 1, LINT_EYE);
    }

    // 5) Elite attachment: two prominent gleaming brass coat buttons mounted
    // down the chest/spine so they read unmistakably from either side.
    if elite {
        for (bu, bv) in [(0.85f32, -0.45f32), (-2.65, -0.30)] {
            let b = lf.px(bu * k, bv * k);
            draw_brass_button(pb, b.0, b.1, fr.hit);
        }
    }
}

pub(crate) fn draw_lint_golem_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let lf = Local::new(fr);
    let k = fr.r / 6.5;
    let (dark, mid, hi) = (lf.tint(LINT_DARK), lf.tint(LINT_MID), lf.tint(LINT_HI));
    let rim_col = if fr.hit > 0.4 { (165, 160, 152) } else { LINT_RIM };

    // Fast zero-sqrt quadratic ellipses keeping the broad asymmetric shoulder silhouette.
    let Some((x0, x1, y0, y1)) = lf.bbox(pb, -4.6 * k, 4.8 * k, 7.0 * k, 1, 1) else {
        return;
    };
    let inv_tu = 1.0 / (3.2 * k);
    let inv_tv = 1.0 / (3.3 * k);
    let inv_bu = 1.0 / (2.4 * k);
    let inv_bv = 1.0 / (2.7 * k);
    let inv_su = 1.0 / (1.8 * k);
    let inv_sv = 1.0 / (2.0 * k);

    for y in y0..=y1 {
        let (mut u, mut v) = lf.uv(x0, y);
        for x in x0..=x1 {
            let tnu = (u + 0.5 * k) * inv_tu;
            let tnv = v * inv_tv;
            let r2_t = tnu * tnu + tnv * tnv;

            let bnu = (u - 0.5 * k) * inv_bu;
            let bnv = (v + 3.8 * k) * inv_bv;
            let r2_b = bnu * bnu + bnv * bnv;

            let snu = (u + 0.2 * k) * inv_su;
            let snv = (v - 3.3 * k) * inv_sv;
            let r2_s = snu * snu + snv * snv;

            let r2 = r2_t.min(r2_b).min(r2_s);
            if r2 <= 1.0 {
                let lit = lf.lit(tnu.clamp(-1.0, 1.0), (v / (5.5 * k)).clamp(-1.0, 1.0));
                let mut col = if lit > 0.25 {
                    hi
                } else if lit < -0.25 {
                    dark
                } else {
                    mid
                };
                if elite {
                    // Double shoulder stripe bands kept readable at far zoom.
                    let db = (v + 3.8 * k).abs();
                    let ds = (v - 3.3 * k).abs();
                    if (db - 0.9 * k).abs() < 0.45 * k || (ds - 0.75 * k).abs() < 0.40 * k {
                        col = ELITE_STRIPE;
                    }
                }
                pb.plot(x, y, col);
            } else if r2 <= 1.55 {
                blend_px(pb, x, y, rim_col, 0.60);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // Dark dryer-button eye with crisp specular glint at the front.
    let eye = lf.px(2.6 * k, 0.3 * k);
    pb.plot(eye.0, eye.1, LINT_EYE);
    pb.plot(eye.0 - 1, eye.1, LINT_EYE_GLINT);

    // Elite attachment at far zoom: two unmistakable bright BRASS coat buttons.
    if elite {
        let brass = lf.tint(BRASS);
        let brass_hi = lf.tint(BRASS_HI);
        for u in [0.7f32, -1.8] {
            let b = lf.px(u * k, -0.2 * k);
            pb.plot(b.0, b.1, brass);
            pb.plot(b.0 - 1, b.1, brass_hi);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_enemy;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const KINDS: [EnemyKind; 5] = [
        EnemyKind::WoolPill,
        EnemyKind::ClothesMoth,
        EnemyKind::PegSnapper,
        EnemyKind::StarchBulb,
        EnemyKind::LintGolem,
    ];
    const BG: Rgb = (20, 22, 40);

    fn body(kind: EnemyKind) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut e = Enemy::spawn(kind, Vec2::new(40.0, 40.0), 1.0, &mut rng);
        e.facing = Vec2::new(1.0, 0.0);
        e.fire_cd = 2.0;
        e
    }

    /// Render one body and return the full pixel slice.
    fn render(e: &Enemy, lod: bool) -> Vec<Rgb> {
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear(BG);
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, e, Vec2::ZERO, (255, 255, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, e, Vec2::ZERO, (255, 255, 255));
        }
        pb.pixels().to_vec()
    }

    /// Every pixel a draw changed, as a mask.
    fn mask(e: &Enemy, lod: bool) -> Vec<bool> {
        render(e, lod).into_iter().map(|p| p != BG).collect()
    }

    fn count(m: &[bool]) -> usize {
        m.iter().filter(|&&b| b).count()
    }

    #[test]
    fn every_laundry_kind_draws_a_body_and_a_smaller_far_body() {
        for kind in KINDS {
            let e = body(kind);
            let (full, far) = (mask(&e, false), mask(&e, true));
            assert!(count(&full) >= 12, "{kind:?} full body is {} px", count(&full));
            assert!(count(&far) >= 4, "{kind:?} far body is {} px", count(&far));
            assert!(
                count(&far) < count(&full),
                "{kind:?} far body ({}) is not smaller than full ({})",
                count(&far),
                count(&full)
            );
        }
    }

    #[test]
    fn the_five_silhouettes_are_distinct() {
        let masks: Vec<Vec<bool>> = KINDS.iter().map(|&k| mask(&body(k), false)).collect();
        for i in 0..masks.len() {
            for j in i + 1..masks.len() {
                let differ = masks[i].iter().zip(&masks[j]).filter(|(a, b)| a != b).count();
                assert!(differ >= 8, "{:?} and {:?} differ by {differ} px", KINDS[i], KINDS[j]);
            }
        }
    }

    #[test]
    fn each_body_has_two_walk_frames() {
        for kind in KINDS {
            let a = body(kind);
            let mut seen = false;
            for step in 1..40 {
                let mut b = body(kind);
                b.anim = a.anim + step as f32 * 0.5;
                if mask(&a, false) != mask(&b, false) {
                    seen = true;
                    break;
                }
            }
            assert!(seen, "{kind:?} never changes pose as its gait advances");
        }
    }

    #[test]
    fn each_body_has_a_light_and_a_dark_edge() {
        for kind in KINDS {
            let e = body(kind);
            let pixels = render(&e, false);
            let lum = |p: Rgb| p.0 as u32 + p.1 as u32 + p.2 as u32;
            let painted: Vec<u32> = pixels.into_iter().filter(|&p| p != BG).map(lum).collect();
            let (lo, hi) = (painted.iter().min().unwrap(), painted.iter().max().unwrap());
            assert!(hi - lo >= 150, "{kind:?} has no light/dark contrast ({lo}..{hi})");
        }
    }

    #[test]
    fn the_elite_golem_wears_brass_buttons_and_a_shoulder_stripe() {
        let plain = body(EnemyKind::LintGolem);
        let mut elite = body(EnemyKind::LintGolem);
        elite.elite = true;
        for lod in [false, true] {
            let (a, b) = (render(&plain, lod), render(&elite, lod));
            assert_ne!(a, b, "elite reads the same (lod {lod})");
            assert!(b.contains(&BRASS), "no brass button (lod {lod})");
            assert!(!a.contains(&BRASS), "a plain golem has no brass (lod {lod})");
            assert!(b.contains(&ELITE_STRIPE), "no double shoulder stripe (lod {lod})");
        }
    }

    #[test]
    fn the_wool_pill_bunches_when_it_stalls() {
        let walking = body(EnemyKind::WoolPill);
        let mut stalled = body(EnemyKind::WoolPill);
        stalled.w3.flea_t = rules::WOOL_PILL_PERIOD - rules::WOOL_PILL_PAUSE * 0.5;
        assert!(rules::wool_pill_paused(&stalled));
        assert_ne!(mask(&walking, false), mask(&stalled, false));
    }

    #[test]
    fn the_starch_bulb_shows_its_charge() {
        let idle = body(EnemyKind::StarchBulb);
        let mut charged = body(EnemyKind::StarchBulb);
        charged.fire_cd = 0.02;
        assert_ne!(mask(&idle, false), mask(&charged, false));
    }

    #[test]
    fn starch_bulb_recoil_differs_from_idle_and_charge() {
        let idle = body(EnemyKind::StarchBulb);
        let mut charged = body(EnemyKind::StarchBulb);
        charged.fire_cd = 0.05;
        let mut recoiled = body(EnemyKind::StarchBulb);
        recoiled.fire_cd = 2.18;
        let (p_idle, p_charge, p_recoil) = (
            render(&idle, false),
            render(&charged, false),
            render(&recoiled, false),
        );
        assert_ne!(p_recoil, p_idle);
        assert_ne!(p_recoil, p_charge);
    }

    #[test]
    fn the_peg_snapper_draws_its_floor_jaws_from_the_hit_geometry() {
        let mut e = body(EnemyKind::PegSnapper);
        let walking = mask(&e, false);
        e.w3.bar_pos = e.pos + Vec2::new(15.0, 0.0);
        e.w3.bar_dir = Vec2::new(1.0, 0.0);
        e.w3.home = Vec2::new(rules::PEG_PHASE_TELL as f32, 0.2);
        let view = rules::jaw_view(&e).expect("jaws set");
        let tell = mask(&e, false);
        assert!(count(&tell) > count(&walking) + 40, "the tell paints two bars");
        // Every pixel under either jaw's centre line is painted.
        let pb_w = 80usize;
        for c in view.jaws() {
            for du in [-7.0f32, 0.0, 7.0] {
                let p = c + view.axis * du;
                let idx = p.y.round() as usize * pb_w + p.x.round() as usize;
                assert!(tell[idx], "jaw pixel at {p:?} is not drawn");
            }
        }
        // The strike pose is its own frame, and so is the far body's tell.
        e.w3.home = Vec2::new(rules::PEG_PHASE_CLOSE as f32, 0.15);
        assert_ne!(mask(&e, false), tell);
        assert!(count(&mask(&e, true)) > 40, "the far body keeps the jaws");
    }

    #[test]
    fn peg_snapper_poses_all_differ_across_phases() {
        let walk = body(EnemyKind::PegSnapper);
        let mut tell = body(EnemyKind::PegSnapper);
        tell.w3.bar_pos = tell.pos + Vec2::new(15.0, 0.0);
        tell.w3.bar_dir = Vec2::new(1.0, 0.0);
        tell.w3.home = Vec2::new(rules::PEG_PHASE_TELL as f32, 0.4);
        let mut close = tell.clone();
        close.w3.home = Vec2::new(rules::PEG_PHASE_CLOSE as f32, 0.2);
        let mut shut = tell.clone();
        shut.w3.home = Vec2::new(rules::PEG_PHASE_SHUT as f32, 0.5);

        let frames = [
            render(&walk, false),
            render(&tell, false),
            render(&close, false),
            render(&shut, false),
        ];
        for i in 0..frames.len() {
            for j in i + 1..frames.len() {
                assert_ne!(frames[i], frames[j], "peg phase {i} matched phase {j}");
            }
        }
    }

    #[test]
    fn draw_peg_tell_paints_only_when_jaws_are_active() {
        let mut e = body(EnemyKind::PegSnapper);
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear(BG);
        draw_peg_tell(&mut pb, &e, Vec2::ZERO);
        assert!(pb.pixels().iter().all(|&p| p == BG), "walking peg paints no floor tell");

        e.w3.bar_pos = e.pos + Vec2::new(15.0, 0.0);
        e.w3.bar_dir = Vec2::new(1.0, 0.0);
        e.w3.home = Vec2::new(rules::PEG_PHASE_TELL as f32, 0.25);
        draw_peg_tell(&mut pb, &e, Vec2::ZERO);
        let painted = pb.pixels().iter().filter(|&&p| p != BG).count();
        assert!(painted >= 60, "active peg tell painted only {painted} px");
    }

    #[test]
    fn hit_flash_brightens_every_laundry_body() {
        for kind in KINDS {
            let plain = body(kind);
            let mut flashed = body(kind);
            flashed.flash = 0.12;
            for lod in [false, true] {
                let sum = |px: Vec<Rgb>| -> u64 {
                    px.into_iter()
                        .filter(|&p| p != BG)
                        .map(|p| p.0 as u64 + p.1 as u64 + p.2 as u64)
                        .sum()
                };
                let (lo, hi) = (sum(render(&plain, lod)), sum(render(&flashed, lod)));
                assert!(hi > lo, "{kind:?} (lod {lod}) did not brighten on hit flash ({lo} vs {hi})");
            }
        }
    }
}
