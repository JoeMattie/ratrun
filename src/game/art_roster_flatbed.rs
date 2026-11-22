//! The Flatbed's own bodies (`WAVE4_ROSTERS.md`, "Flatbed trailer"): Gravel
//! Tick, June Bug, Tar Rat, Loose Can, Toll Gull and Loose Bungee.
//!
//! Road-yellow, bare aluminium and white ties over dark oak. Every body here
//! is a full-detail SDF/pixel-art draw plus a far-LOD draw with the `BodyDraw`
//! signature `art_enemy::bespoke_body`/`bespoke_lod` expect. The draws are
//! self-contained: they read only the `&Enemy` and the camera (the `Frame`
//! argument is accepted for the signature and ignored), so nothing in
//! `art_enemy` had to open up beyond naming `Frame`.
//!
//! Each kind keeps a crisp 6–12 px core with directional up-left lighting
//! (`LIGHT = (-0.55, -0.83)` matching `art_enemy`), a dark silhouette rim and
//! bright rim/eye glints that survive dark pooled light and 16-colour folding,
//! multi-frame procedural motion driven from `Enemy::anim`, and distinct tell,
//! attack and recovery poses:
//!
//! * **Gravel Tick**: a chipped angular road-gravel pebble with a highway-yellow
//!   paint fleck, crab-walking on four jointed splayed legs with snapping front
//!   pincers on contact.
//! * **June Bug**: two glossy chestnut elytra hinged like an open book over
//!   translucent buzzing flight wings, a plump segmented golden-tan dangling
//!   belly, kicking hind legs and a clear deck shadow below.
//! * **Tar Rat**: a sleek hunched rodent coated in glossy black road tar with
//!   oily bluish-silver ridge highlights, piercing road-yellow eyes, bounding
//!   gallop paws and a dead-straight stiff tar-matted pennant tail.
//! * **Loose Can**: a 3D-shaded horizontal crushed aluminium can with crimped
//!   end caps, a rolling painted label band, a dented sidewall and a road-yellow
//!   ring pull that beats on rock ([`can_pose`]), streaks on roll and lies flat
//!   while resting.
//! * **Toll Gull**: sculpted snow-white and coastal-slate elbow-bent M-wings
//!   with jet-black primary tips, a hovering ground shadow and a long flat
//!   road-yellow beak (crimson tip spot) that splits wide open with a glowing
//!   throat charge before firing ([`gull_beak_open`]).
//! * **Loose Bungee**: a thick forged-steel J-hook and brass crimp ferrule
//!   dragging a continuous braided crimson-and-white striped elastic loop that
//!   whips as it travels and snaps wire-taut with tension glints while its
//!   crossbar tells.
//!
//! ## Public draws for the hostile layer
//!
//! One tell is drawn outside the body, after the lighting pass:
//!
//! * [`draw_loose_can_tell`]`(pb, e, cam)`: the Loose Can's marked roll (lane
//!   edges on each leg, a pip at the bounce, a cross at the stop). Call it for
//!   every live `EnemyKind::LooseCan`; it draws nothing unless the can is
//!   rocking or rolling ([`can_pose`]). It does its own geometry from world
//!   coordinates and must **not** be culled by the body's screen box: the path
//!   reaches 48 px from the can. `art_enemy::draw_wave4_tells` already calls
//!   it, so a second call from the hostile-layer gate would only repaint the
//!   same pixels.
//!
//! The other tells are poses inside the body draw and need no call: the Toll
//! Gull's open beak ([`gull_beak_open`]) and the Loose Bungee's taut loop (its
//! crossbar itself is the Turnkey's, drawn by `art_enemy::draw_w3_tells`).
//! [`can_pose`], [`gull_beak_open`] and [`walk_frame`] are public readers.
//!
//! The Flatbed's elite is the existing Brute with its plated crest in
//! `art_enemy.rs`, so no body here draws an elite attachment (the `elite`
//! argument is ignored).
//!
//! Render-only: nothing here draws rng or writes simulation state.

use std::f32::consts::PI;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

use super::art_enemy::Frame;
use super::ecs::systems::floor_flatbed as rules;
use super::enemy::Enemy;

/// Direction *toward* the light in screen space (up-left), matching
/// `art_enemy.rs`, `art_rat.rs` and `art_props.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Seconds of hit flash that read as a full flash (`art_enemy::FLASH_MAX`).
const FLASH_MAX: f32 = 0.12;

/// Road yellow (beak, ring pull, paint fleck, eyes, hazard marks).
const ROAD_YELLOW: Rgb = (236, 196, 62);
/// Bright road-yellow specular glint.
const ROAD_YELLOW_HI: Rgb = (255, 238, 138);
/// Bare aluminium, lit specular.
const ALU_HI: Rgb = (244, 248, 252);
/// Bare aluminium, mid brushed metal.
const ALU_MID: Rgb = (188, 194, 202);
/// Bare aluminium, recessed steel shade.
const ALU_LO: Rgb = (82, 88, 100);
/// White tie / bungee stripe / stop cross.
const TIE_WHITE: Rgb = (244, 240, 228);

// ---- Local frame & pixel-art primitives ------------------------------------

/// One body's screen-space frame, rebuilt from the `&Enemy`.
#[derive(Clone, Copy)]
struct Fx {
    /// Screen position of the body's centre.
    pos: Vec2,
    /// Unit facing (`u` axis).
    f: Vec2,
    /// Unit lateral (`v` axis, facing rotated a quarter turn).
    p: Vec2,
    /// Per-kind scale relative to the kind's standard collision radius.
    s: f32,
    /// Continuous animation clock (`e.anim`).
    anim: f32,
    /// Discrete two-frame walk parity (`0` or `1`).
    frame: u8,
    /// Hit flash, `0..=1`.
    hit: f32,
    /// Precomputed hit-flash blend target and strength.
    flash_col: Rgb,
    flash_str: f32,
    /// Directional light projections onto `f` and `p`.
    fl: f32,
    pl: f32,
}

impl Fx {
    fn of(e: &Enemy, cam: Vec2, base_r: f32) -> Fx {
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
        Fx {
            pos: Vec2::new(e.pos.x - cam.x, e.pos.y - cam.y),
            f,
            p,
            s: (e.radius.max(1.5) / base_r).clamp(0.65, 2.0),
            anim: e.anim,
            frame: walk_frame(e.anim),
            hit,
            flash_col,
            flash_str,
            fl: f.x * LIGHT.x + f.y * LIGHT.y,
            pl: p.x * LIGHT.x + p.y * LIGHT.y,
        }
    }

    /// Body-local `(u, v)` in screen pixels (already scaled by `s`) -> screen pixel.
    #[inline(always)]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    /// Screen pixel -> body-local `(u, v)` in screen pixels.
    #[inline(always)]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    /// Directional dot product against the up-left light in local `(nu, nv)`.
    #[inline(always)]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    /// Colour with warm sculptural hit flash folded in.
    #[inline(always)]
    fn c(&self, col: Rgb) -> Rgb {
        if self.flash_str <= 0.0 {
            col
        } else {
            mix(col, self.flash_col, self.flash_str)
        }
    }

    /// Silhouette rim colour (near-black normally, warmed under peak hit flash).
    #[inline(always)]
    fn rim(&self, hit_tint: Rgb) -> Rgb {
        if self.hit > 0.4 {
            hit_tint
        } else {
            (0, 0, 0)
        }
    }

    /// Tight oriented screen bounding box for local `[u_min, u_max] × [-v_max, v_max]`.
    #[inline(always)]
    fn bbox_oriented(
        &self,
        pb: &PixelBuffer,
        u_min: f32,
        u_max: f32,
        v_max: f32,
        pad_x: i32,
        pad_y: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        if pb.w == 0 || pb.h == 0 {
            return None;
        }
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

/// The two-frame walk cycle every body here shares: which half of the gait
/// phase `anim` is in.
#[inline]
pub fn walk_frame(anim: f32) -> u8 {
    if anim.sin() >= 0.0 {
        0
    } else {
        1
    }
}

#[inline(always)]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
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
        mix(shadow, mid, l * (1.0 / 0.65))
    } else {
        mix(mid, hi, (l - 0.65) * 2.0)
    }
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
        pb.plot(x, y, mix(bg, c, a));
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
fn seg(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), col: Rgb) {
    pb.line(a.0, a.1, b.0, b.1, col);
}

#[inline]
fn line_blend(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let ady = (b.1 - a.1).abs();
    if dx <= 1 && ady <= 1 {
        blend_px(pb, a.0, a.1, c, alpha);
        if a != b {
            blend_px(pb, b.0, b.1, c, alpha);
        }
        return;
    }
    let dy = -ady;
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
        if horiz {
            blend_px(pb, x, y + 1, edge, 0.86);
        } else {
            blend_px(pb, x + 1, y, edge, 0.86);
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

/// Soft elliptical contact or hover shadow on the oak deck.
fn shadow_ellipse(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, strength: f32) {
    if pb.w == 0 || pb.h == 0 || rx <= 0.2 || ry <= 0.2 {
        return;
    }
    let inv_rx = 1.0 / rx;
    let inv_ry = 1.0 / ry;
    let x0 = ((cx - rx).floor() as i32).max(0);
    let x1 = ((cx + rx).ceil() as i32).min(pb.w as i32 - 1);
    let y0 = ((cy - ry).floor() as i32).max(0);
    let y1 = ((cy + ry).ceil() as i32).min(pb.h as i32 - 1);
    for y in y0..=y1 {
        let ny = (y as f32 - cy) * inv_ry;
        let ny2 = ny * ny;
        for x in x0..=x1 {
            let nx = (x as f32 - cx) * inv_rx;
            let r2 = nx * nx + ny2;
            if r2 <= 1.0 {
                darken_px(pb, x, y, strength * (1.0 - r2 * 0.45));
            }
        }
    }
}

// ---- Signed distance primitives (zero-alloc) -------------------------------

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
        Self {
            cu,
            cv,
            inv_ru: 1.0 / ru.max(1e-4),
            inv_rv: 1.0 / rv.max(1e-4),
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
struct CapsuleSdf {
    au: f32,
    av: f32,
    bax: f32,
    bay: f32,
    inv_denom: f32,
    rad: f32,
}

impl CapsuleSdf {
    #[inline(always)]
    fn new(a: (f32, f32), b: (f32, f32), rad: f32) -> Self {
        let bax = b.0 - a.0;
        let bay = b.1 - a.1;
        Self {
            au: a.0,
            av: a.1,
            bax,
            bay,
            inv_denom: 1.0 / (bax * bax + bay * bay).max(1e-6),
            rad,
        }
    }

    #[inline(always)]
    fn eval(&self, u: f32, v: f32) -> f32 {
        let pax = u - self.au;
        let pay = v - self.av;
        let h = ((pax * self.bax + pay * self.bay) * self.inv_denom).clamp(0.0, 1.0);
        let dx = pax - self.bax * h;
        let dy = pay - self.bay * h;
        (dx * dx + dy * dy).sqrt() - self.rad
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
        Self {
            au: a.0,
            av: a.1,
            bax,
            bay,
            inv_denom: 1.0 / (bax * bax + bay * bay).max(1e-6),
            r0,
            dr: r1 - r0,
        }
    }

    /// Returns `(signed_distance, t_along_segment)`.
    #[inline(always)]
    fn eval_t(&self, u: f32, v: f32) -> (f32, f32) {
        let pax = u - self.au;
        let pay = v - self.av;
        let h = ((pax * self.bax + pay * self.bay) * self.inv_denom).clamp(0.0, 1.0);
        let dx = pax - self.bax * h;
        let dy = pay - self.bay * h;
        let rad = self.r0 + self.dr * h;
        ((dx * dx + dy * dy).sqrt() - rad, h)
    }

    #[inline(always)]
    fn eval(&self, u: f32, v: f32) -> f32 {
        self.eval_t(u, v).0
    }
}

#[inline(always)]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let diff = (a - b).abs();
    if diff >= k {
        a.min(b)
    } else {
        let h = (k - diff) / k;
        a.min(b) - h * h * k * 0.25
    }
}

#[inline(always)]
fn sd_rounded_box(u: f32, v: f32, hu: f32, hv: f32, r: f32) -> f32 {
    let qx = u.abs() - (hu - r);
    let qy = v.abs() - (hv - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Composite one SDF pixel with anti-aliased fill and dark silhouette rim.
#[inline(always)]
fn plot_sdf(pb: &mut PixelBuffer, at: (i32, i32), d: f32, rim_lim: f32, col: Rgb, rim: (Rgb, f32)) {
    let (x, y) = at;
    let (rim_col, rim_alpha) = rim;
    let cover = (0.50 - d * 1.18).clamp(0.0, 1.0);
    if cover >= 1.0 {
        pb.plot(x, y, col);
    } else if cover > 0.0 {
        let bg = pb.pixel_at(x as usize, y as usize);
        let out = mix(bg, rim_col, rim_alpha * (1.0 - cover));
        pb.plot(x, y, mix(out, col, cover));
    } else if d < rim_lim {
        let bg = pb.pixel_at(x as usize, y as usize);
        pb.plot(x, y, mix(bg, rim_col, rim_alpha));
    }
}

// ---- Gravel Tick -----------------------------------------------------------

/// Gravel Tick: a squared, chipped highway pebble with splayed crab-walking
/// feet and a bright road-yellow paint fleck.
///
/// The stone stays **screen-square** whatever the heading (a chip of gravel
/// does not rotate its corners), with a diagonally chipped top-right bevel,
/// planar granite shading, a dark silhouette rim and a bright paint-chip
/// streak. Four jointed crab legs step in alternating diagonals; on contact
/// (`contact_cd`) tiny front mandibles snap out toward the rat.
pub(crate) fn draw_gravel_tick(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    gravel_tick(pb, e, cam);
}

fn gravel_tick(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fx = Fx::of(e, cam, 2.5);
    let s = fx.s;
    let gait = fx.anim.sin();
    let lift = fx.anim.cos();
    let biting = e.contact_cd > 0.25;

    // Subtle sideways scuttle rock as it crab-walks between planks.
    let cx = fx.pos.x + gait * 0.45 * s;
    let cy = fx.pos.y + lift.abs() * -0.35 * s;

    // 1) Ground shadow under the pebble.
    shadow_ellipse(pb, cx + 0.5 * s, cy + 2.8 * s, 3.6 * s, 1.7 * s, 0.42);

    // 2) Four splayed, jointed crab legs (drawn first so the pebble overhangs the hips).
    let leg_upper = fx.c((102, 92, 80));
    let leg_lower = fx.c((48, 42, 36));
    let knee_hi = fx.c((172, 162, 146));
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let pair = if sx * sy > 0.0 { 1.0 } else { -1.0 };
        let step = pair * gait;
        let flare = if biting { 0.55 * s } else { 0.0 };
        let hip = (
            (cx + sx * 2.4 * s).round() as i32,
            (cy + sy * 1.7 * s).round() as i32,
        );
        let knee = (
            (cx + sx * (3.9 * s + flare) + step * 0.65 * s).round() as i32,
            (cy + sy * (2.5 * s - 0.45 * pair * lift * s)).round() as i32,
        );
        let foot = (
            (cx + sx * (5.1 * s + flare) + step * 0.95 * s).round() as i32,
            (cy + sy * (3.6 * s + 0.55 * step * s)).round() as i32,
        );
        darken_px(pb, foot.0 + 1, foot.1 + 1, 0.28);
        line_blend(pb, hip, knee, leg_upper, 0.94);
        line_blend(pb, knee, foot, leg_lower, 0.95);
        blend_px(pb, knee.0, knee.1, knee_hi, 0.85);
    }

    // 3) Faceted, chipped limestone/gravel pebble with a bright highway-yellow paint chip.
    let base = e.kind.color();
    let stone_sh = mix(base, (34, 30, 28), 0.72);
    let stone_mid = base;
    let stone_hi = mix(base, (246, 244, 234), 0.76);
    let crack_c = (44, 40, 36);
    let rim_col = fx.rim((146, 138, 122));

    let rad_x = 3.25 * s;
    let rad_y = 2.65 * s;
    let rim_lim = 0.90 * s;
    let x0 = ((cx - rad_x - 2.0 * s).floor() as i32).max(0);
    let x1 = ((cx + rad_x + 2.0 * s).ceil() as i32).min(pb.w as i32 - 1);
    let y0 = ((cy - rad_y - 2.0 * s).floor() as i32).max(0);
    let y1 = ((cy + rad_y + 2.0 * s).ceil() as i32).min(pb.h as i32 - 1);

    for y in y0..=y1 {
        let dy = y as f32 - cy;
        for x in x0..=x1 {
            let dx = x as f32 - cx;
            // Squared stone with slightly rounded edges and a fractured top-right chip.
            let box_d = sd_rounded_box(dx, dy, rad_x, rad_y, 0.55 * s);
            let chip_cut = (dx - dy) - 3.35 * s;
            let d = box_d.max(chip_cut * 0.72);
            if d >= rim_lim {
                continue;
            }
            let col = if d < 0.50 {
                // Planar bevel facets so it reads as angular crushed gravel, not a blob.
                let nx = (dx / rad_x).clamp(-1.0, 1.0);
                let ny = (dy / rad_y).clamp(-1.0, 1.0);
                let facet_lit = nx * LIGHT.x + ny * LIGHT.y;
                let mut lum = if dx < -1.4 * s || dy < -1.3 * s {
                    0.86 + 0.20 * facet_lit
                } else if dx > 1.4 * s || dy > 1.0 * s || chip_cut > -0.75 * s {
                    0.26 + 0.22 * facet_lit
                } else {
                    0.54 + 0.30 * facet_lit
                };
                // Subtle diagonal stone fracture ridge.
                let seam = (dx * 0.8 + dy * 1.15 - 0.3 * s).abs();
                let mut c = shade3(stone_sh, stone_mid, stone_hi, lum);
                if seam < 0.42 * s && dx > -1.4 * s && dy > -1.0 * s {
                    c = mix(c, crack_c, 0.52);
                }
                // Highway-yellow paint fleck clinging to the top-left table facet.
                if dx >= -2.1 * s && dx <= 0.1 * s && dy >= -2.0 * s && dy <= -0.5 * s {
                    let paint = if dx < -1.1 * s || dy < -1.3 * s {
                        ROAD_YELLOW_HI
                    } else {
                        ROAD_YELLOW
                    };
                    lum = (lum + 0.15).clamp(0.0, 1.1);
                    c = mix(paint, ROAD_YELLOW_HI, (lum - 0.7).clamp(0.0, 0.5));
                }
                fx.c(c)
            } else {
                rim_col
            };
            plot_sdf(pb, (x, y), d, rim_lim, col, (rim_col, 0.70));
        }
    }

    // 4) Piercing amber eyes tucked under the leading edge + snapping front mandibles.
    let eye_col = if fx.hit > 0.65 {
        (36, 26, 18)
    } else {
        (255, 214, 72)
    };
    for side in [-1.0f32, 1.0] {
        let ep = (
            (cx + fx.f.x * 2.1 * s + fx.p.x * side * 1.15 * s).round() as i32,
            (cy + fx.f.y * 2.1 * s + fx.p.y * side * 1.15 * s).round() as i32,
        );
        pb.plot(ep.0, ep.1, eye_col);
        if biting {
            let claw_a = (
                (cx + fx.f.x * 2.8 * s + fx.p.x * side * 1.2 * s).round() as i32,
                (cy + fx.f.y * 2.8 * s + fx.p.y * side * 1.2 * s).round() as i32,
            );
            let claw_b = (
                (cx + fx.f.x * 4.1 * s + fx.p.x * side * 0.65 * s).round() as i32,
                (cy + fx.f.y * 4.1 * s + fx.p.y * side * 0.65 * s).round() as i32,
            );
            line_blend(pb, claw_a, claw_b, fx.c(ROAD_YELLOW_HI), 0.92);
        }
    }
}

/// Far LOD: a compact chipped stone square with a yellow paint glint and two
/// alternating diagonal crab feet.
pub(crate) fn draw_gravel_tick_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    gravel_tick_lod(pb, e, cam);
}

fn gravel_tick_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam, 2.5);
    let cx = fx.pos.x.round() as i32;
    let cy = fx.pos.y.round() as i32;
    let base = e.kind.color();
    let hi = fx.c(mix(base, (248, 246, 236), 0.72));
    let mid = fx.c(base);
    let lo = fx.c(mix(base, (30, 26, 24), 0.68));
    let foot = fx.c((64, 56, 48));
    let d = if fx.frame == 0 { 1 } else { -1 };
    // Splayed alternating crab feet.
    pb.plot(cx - 2, cy - 2 * d, foot);
    pb.plot(cx + 2, cy + 2 * d, foot);
    // 3×3 chipped stone core with dark drop shadow and paint-chip glint.
    darken_px(pb, cx, cy + 2, 0.45);
    darken_px(pb, cx + 1, cy + 2, 0.45);
    pb.rect_fill(cx - 1, cy - 1, 3, 3, mid);
    pb.plot(cx - 1, cy - 1, hi);
    pb.plot(cx, cy - 1, fx.c(ROAD_YELLOW));
    pb.plot(cx + 1, cy + 1, lo);
    pb.plot(cx + 1, cy, lo);
}

// ---- June Bug --------------------------------------------------------------

/// June Bug: glossy chestnut split wing cases like an open book above buzzing
/// flight wings and a plump segmented dangling belly.
///
/// The two curved elytra hinge at the forward pronotum and open backward into
/// a wide V that rattles with `anim`, while pale flight wings beat rapidly
/// underneath. The segmented golden-tan belly dangles **below** in screen space
/// above a grounded belly shadow ("step across its belly shadow").
pub(crate) fn draw_june_bug(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    june_bug(pb, e, cam);
}

fn june_bug(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fx = Fx::of(e, cam, 2.0);
    let s = fx.s;
    let flap = fx.anim.sin(); // -1..1
    let spread_t = 0.5 + 0.5 * flap; // 0..1
    let buzz = (fx.anim * 2.5).sin();
    let hover_y = -0.65 * flap * s;

    // 1) Distinct elevated ground shadow ("belly shadow" on the oak planks).
    shadow_ellipse(
        pb,
        fx.pos.x + 0.6 * s,
        fx.pos.y + 5.8 * s,
        (2.8 + 0.45 * spread_t) * s,
        1.55 * s,
        0.44,
    );

    // Body centre bobs slightly above the ground shadow.
    let mut bfx = fx;
    bfx.pos.y += hover_y;
    let rim_col = bfx.rim((128, 92, 62));

    // 2) Kicking dangling hind legs + plump segmented golden-tan belly (screen-down).
    let belly_cx = bfx.pos.x;
    let belly_cy = bfx.pos.y + 2.1 * s;
    let leg_c = bfx.c((72, 42, 22));
    for side in [-1.0f32, 1.0] {
        let kick = side * flap * 0.55 * s;
        let hip = (
            (belly_cx + side * 1.4 * s).round() as i32,
            (belly_cy + 1.5 * s).round() as i32,
        );
        let toe = (
            (belly_cx + side * 2.3 * s + kick).round() as i32,
            (belly_cy + 3.0 * s - kick * 0.6).round() as i32,
        );
        line_blend(pb, hip, toe, leg_c, 0.90);
    }

    let shell = e.kind.color();
    let belly_sh = (86, 52, 24);
    let belly_mid = mix(shell, (232, 186, 108), 0.64);
    let belly_hi = (252, 232, 174);
    let belly_rx = 2.35 * s;
    let belly_ry = 2.15 * s;
    let belly_sdf = EllipseSdf::new(0.0, 0.0, belly_rx, belly_ry);
    let bx0 = ((belly_cx - belly_rx - 1.5 * s).floor() as i32).max(0);
    let bx1 = ((belly_cx + belly_rx + 1.5 * s).ceil() as i32).min(pb.w as i32 - 1);
    let by0 = ((belly_cy - belly_ry - 1.5 * s).floor() as i32).max(0);
    let by1 = ((belly_cy + belly_ry + 1.5 * s).ceil() as i32).min(pb.h as i32 - 1);
    for y in by0..=by1 {
        let dy = y as f32 - belly_cy;
        for x in bx0..=bx1 {
            let dx = x as f32 - belly_cx;
            let d = belly_sdf.eval(dx, dy);
            if d >= 0.85 * s {
                continue;
            }
            let nx = (dx / belly_rx).clamp(-1.0, 1.0);
            let ny = (dy / belly_ry).clamp(-1.0, 1.0);
            let dome = (1.0 - (nx * nx * 0.55 + ny * ny * 0.65)).clamp(0.0, 1.0);
            let lit = nx * LIGHT.x + ny * LIGHT.y;
            let mut c = shade3(belly_sh, belly_mid, belly_hi, 0.42 + 0.36 * dome + 0.54 * lit);
            // Segmented chitin rings across the dangling abdomen.
            if ((dy / s) * 2.8).sin() > 0.62 && dy > -0.6 * s {
                c = mix(c, belly_sh, 0.52);
            }
            plot_sdf(pb, (x, y), d, 0.85 * s, bfx.c(c), (rim_col, 0.68));
        }
    }

    // 3) Pale translucent flight wings buzzing rapidly beneath the wing cases.
    let wing_span = (4.6 + 1.5 * buzz.abs()) * s;
    let wing_col = bfx.c((238, 234, 212));
    for side in [-1.0f32, 1.0] {
        let w_root = bfx.px(0.4 * s, side * 1.0 * s);
        let w_tip = bfx.px((-2.3 + 0.85 * buzz) * s, side * wing_span);
        let w_back = bfx.px((-3.4 + 0.5 * buzz) * s, side * (wing_span * 0.72));
        line_blend(pb, w_root, w_tip, wing_col, 0.50);
        line_blend(pb, w_root, w_back, wing_col, 0.32);
    }

    // 4) Sculpted glossy chestnut wing cases ("open book") + pronotum & head.
    let spread = (3.45 + 1.35 * spread_t) * s;
    let ely_sh = (46, 20, 10);
    let ely_mid = mix(shell, (164, 86, 38), 0.55);
    let ely_hi = (248, 198, 122);
    let ely_spec = (255, 246, 208);

    let ely_l = TaperSdf::new((1.1 * s, -0.75 * s), (-3.15 * s, -spread), 1.05 * s, 0.72 * s);
    let ely_r = TaperSdf::new((1.1 * s, 0.75 * s), (-3.15 * s, spread), 1.05 * s, 0.72 * s);
    let pronotum = EllipseSdf::new(1.35 * s, 0.0, 1.35 * s, 1.55 * s);
    let head_sdf = EllipseSdf::new(2.65 * s, 0.0, 0.85 * s, 1.05 * s);

    if let Some((x0, x1, y0, y1)) =
        bfx.bbox_oriented(pb, -4.8 * s, 4.4 * s, spread + 1.8 * s, 0, 0)
    {
        let rim_lim = 0.86 * s;
        for y in y0..=y1 {
            let (mut u, mut v) = bfx.local(x0, y);
            for x in x0..=x1 {
                let (dl, tl) = ely_l.eval_t(u, v);
                let (dr, tr) = ely_r.eval_t(u, v);
                let (d_ely, t_ely, side_sign) = if dl <= dr {
                    (dl, tl, -1.0f32)
                } else {
                    (dr, tr, 1.0f32)
                };
                let d_pro = smin(pronotum.eval(u, v), head_sdf.eval(u, v), 0.35 * s);
                let d = d_ely.min(d_pro);
                if d < rim_lim {
                    let is_ely = d_ely < d_pro;
                    let col = if is_ely {
                        // Curved glossy beetle elytron with bright inner spine ridge.
                        let ridge = (1.0 - (-d_ely / (1.05 * s)).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                        let lit = bfx.lit(0.35 - 0.55 * t_ely, side_sign * 0.72);
                        let mut lum = 0.38 + 0.34 * (1.0 - ridge * 0.45) + 0.56 * lit;
                        let mut c = shade3(ely_sh, ely_mid, ely_hi, lum);
                        // Glossy lacquer specular glint along the upper-inner spine of the open book.
                        if lit > 0.28 && t_ely > 0.15 && t_ely < 0.72 && ridge < 0.55 {
                            lum += 0.25;
                            c = mix(c, ely_spec, (0.45 + 0.35 * (lum - 0.8)).clamp(0.0, 0.82));
                        }
                        bfx.c(c)
                    } else {
                        let nu = ((u - 1.6 * s) / (1.8 * s)).clamp(-1.0, 1.0);
                        let nv = (v / (1.6 * s)).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu * 0.5 + nv * nv * 0.65)).clamp(0.0, 1.0);
                        let lum = 0.32 + 0.36 * dome + 0.55 * bfx.lit(nu, nv);
                        bfx.c(shade3((34, 16, 10), (96, 52, 26), ely_hi, lum))
                    };
                    plot_sdf(pb, (x, y), d, rim_lim, col, (rim_col, 0.72));
                }
                u += bfx.f.x;
                v += bfx.p.x;
            }
        }
    }

    // 5) Bright amber eyes & clubbed antennae tips ahead of the head.
    let eye_c = if bfx.hit > 0.65 {
        (32, 18, 12)
    } else {
        (255, 218, 76)
    };
    let ant_c = bfx.c((210, 152, 92));
    for side in [-1.0f32, 1.0] {
        let ep = bfx.px(2.65 * s, side * 0.85 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let a0 = bfx.px(3.1 * s, side * 0.55 * s);
        let a1 = bfx.px(3.9 * s, side * 1.45 * s);
        line_blend(pb, a0, a1, ant_c, 0.82);
    }
}

/// Far LOD: a glossy chestnut open-book V above a bright dangling belly and
/// its grounded shadow pixel.
pub(crate) fn draw_june_bug_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    june_bug_lod(pb, e, cam);
}

fn june_bug_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam, 2.0);
    let cx = fx.pos.x.round() as i32;
    let cy = fx.pos.y.round() as i32;
    let shell = e.kind.color();
    let hi = fx.c(mix(shell, (248, 206, 134), 0.62));
    let mid = fx.c(mix(shell, (158, 82, 36), 0.5));
    let lo = fx.c((48, 22, 12));
    let belly = fx.c(mix(shell, (240, 202, 124), 0.68));
    let w = if fx.frame == 0 { 3 } else { 2 };
    // Elevated belly shadow.
    darken_px(pb, cx, cy + 4, 0.42);
    darken_px(pb, cx + 1, cy + 4, 0.35);
    // Dangling golden belly with dark rim.
    darken_px(pb, cx, cy + 2, 0.55);
    pb.plot(cx, cy + 1, belly);
    pb.plot(cx - 1, cy + 1, lo);
    // Open-book V wing cases.
    thick_line(pb, (cx, cy), (cx - w, cy - 2), hi, lo);
    thick_line(pb, (cx, cy), (cx + w, cy - 2), mid, lo);
    pb.plot(cx, cy - 1, fx.c(ROAD_YELLOW_HI));
}

// ---- Tar Rat ---------------------------------------------------------------

/// Tar Rat: a sleek, hunched road-tar rodent with a stiff pennant tail, low
/// nose and bounding gallop.
///
/// Glossy obsidian-tar coat with oily bluish-silver ridge highlights so it
/// pops off dark oak and low pooled light, piercing road-yellow eyes, four
/// bounding gallop paws (flaring with pale claws on contact) and a dead-straight
/// tail ending in a solid triangular tar-matted pennant.
pub(crate) fn draw_tar_rat(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    tar_rat(pb, e, cam);
}

fn tar_rat(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fx = Fx::of(e, cam, 4.0);
    let s = fx.s;
    let stretch = fx.anim.sin(); // +1 extended gallop, -1 gathered tuck
    let lunge = if e.contact_cd > 0.28 { 1.15 * s } else { 0.0 };
    let rim_col = fx.rim((122, 126, 138));

    // 1) Bounding gallop paws (under the body so the haunches overhang them).
    let leg_dark = fx.c((24, 22, 26));
    let leg_sheen = fx.c((94, 100, 114));
    let claw_c = fx.c(if lunge > 0.0 { (236, 240, 248) } else { (154, 158, 168) });
    let fore_reach = (3.4 + 1.85 * stretch) * s + lunge;
    let aft_reach = (-3.7 - 1.75 * stretch) * s;
    let stance_v = (2.65 - 0.35 * stretch) * s;
    for side in [-1.0f32, 1.0] {
        let fh = fx.px(2.0 * s, side * 1.45 * s);
        let fp = fx.px(fore_reach, side * stance_v);
        darken_px(pb, fp.0 + 1, fp.1 + 1, 0.34);
        line_blend(pb, fh, fp, leg_dark, 0.96);
        blend_px(pb, fp.0, fp.1, claw_c, 0.90);

        let ah = fx.px(-2.4 * s, side * 1.55 * s);
        let ap = fx.px(aft_reach, side * (stance_v + 0.15 * s));
        darken_px(pb, ap.0 + 1, ap.1 + 1, 0.34);
        line_blend(pb, ah, ap, leg_dark, 0.96);
        blend_px(pb, ah.0, ah.1, leg_sheen, 0.65);
        blend_px(pb, ap.0, ap.1, claw_c, 0.72);
    }

    // 2) Sculpted tar-slick rodent body + dead-straight stiff pennant tail.
    let u_min = -12.6 * s;
    let u_max = 6.6 * s + lunge;
    let v_max = 4.2 * s;
    let Some((x0, x1, y0, y1)) = fx.bbox_oriented(pb, u_min, u_max, v_max, 1, 2) else {
        return;
    };

    let base = e.kind.color();
    let tar_sh = (12, 11, 15);
    let tar_mid = mix(base, (40, 38, 46), 0.65);
    let tar_hi = (126, 134, 152);
    let oil_sheen = (218, 226, 242);
    let ear_hi = fx.c((144, 152, 170));

    let haunch = EllipseSdf::new((-2.0 - 0.35 * stretch) * s, 0.0, (2.3 + 0.25 * stretch) * s, (2.2 - 0.15 * stretch) * s);
    let chest = EllipseSdf::new(0.95 * s, 0.0, 2.05 * s, 1.95 * s);
    let snout = TaperSdf::new((2.1 * s, 0.0), (5.0 * s + lunge, 0.0), 1.42 * s, 0.56 * s);
    let ear = EllipseSdf::new(2.25 * s, 1.85 * s, 0.85 * s, 0.70 * s);
    let tail_rod = CapsuleSdf::new((-3.8 * s, 0.0), (-11.4 * s, 0.0), 0.52 * s);

    let du_sh = -(fx.f.x + 1.5 * fx.f.y);
    let dv_sh = -(fx.p.x + 1.5 * fx.p.y);
    let rim_lim = 0.88 * s;

    // Evaluate body + solid filled triangular pennant tail.
    let eval_rat = |u: f32, v: f32| -> (f32, u8) {
        let av = v.abs();
        let d_torso = smin(haunch.eval(u, v), chest.eval(u, v), 0.55 * s);
        let d_head = smin(d_torso, snout.eval(u, v), 0.32 * s);
        let d_ear = if u > 0.6 * s && u < 4.0 * s && av > 0.4 * s {
            ear.eval(u, av)
        } else {
            10.0
        };
        let d_body = d_head.min(d_ear);
        let d_tail = if u < -2.6 * s {
            let rod = tail_rod.eval(u, v);
            // Solid triangular tar-matted pennant from u = -6.7 to tip u = -11.5.
            let d_flag = if u >= -12.6 * s && u <= -5.6 * s {
                let t = ((u + 11.5 * s) / (4.8 * s)).clamp(0.0, 1.0);
                let half_w = (2.35 * t) * s;
                let dv = av - half_w;
                let du_base = u - (-6.7 * s);
                let du_tip = (-11.5 * s) - u;
                dv.max(du_base).max(du_tip)
            } else {
                10.0
            };
            rod.min(d_flag)
        } else {
            10.0
        };
        if d_ear < d_head && d_ear < d_tail {
            (d_ear, 2)
        } else if d_tail < d_body {
            (d_tail, 1)
        } else {
            (d_body, 0)
        }
    };

    for y in y0..=y1 {
        let (mut u, mut v) = fx.local(x0, y);
        for x in x0..=x1 {
            let (d, part) = eval_rat(u, v);
            if d < rim_lim {
                let col = match part {
                    2 => {
                        // Swept-back rat ear: dark inner hollow with oily silver upper rim.
                        if fx.lit(0.2, if v >= 0.0 { 0.8 } else { -0.8 }) > 0.05 {
                            ear_hi
                        } else {
                            fx.c((34, 34, 42))
                        }
                    }
                    1 => {
                        // Stiff tar pennant: deep obsidian core with crisp silvery-oil lit edge.
                        let nv = (v / (2.2 * s)).clamp(-1.0, 1.0);
                        let lit = fx.lit(-0.2, nv);
                        let edge = (d > -0.82 * s) && lit > -0.1;
                        let mut c = shade3(tar_sh, tar_mid, tar_hi, 0.34 + 0.56 * lit);
                        if edge {
                            c = mix(c, oil_sheen, 0.62);
                        } else if (u + 6.9 * s).abs() < 0.55 * s && v.abs() < 2.2 * s {
                            // Crisp leading mast edge of the stiff pennant flag.
                            c = mix(c, oil_sheen, 0.52);
                        }
                        fx.c(c)
                    }
                    _ => {
                        let nu = ((u - 0.5 * s) / (4.4 * s)).clamp(-1.0, 1.0);
                        let nv = (v / (2.2 * s)).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu * 0.42 + nv * nv * 0.80)).clamp(0.0, 1.0);
                        let lit = fx.lit(nu, nv);
                        let lum = 0.28 + 0.34 * dome + 0.66 * lit;
                        let mut c = shade3(tar_sh, tar_mid, tar_hi, lum);
                        // Wet oily bluish-silver road-tar specular ribbon along lit spine.
                        if lit > 0.34 && dome > 0.26 {
                            c = mix(c, oil_sheen, ((lit - 0.28) * 1.15).clamp(0.0, 0.84));
                        }
                        // Subtle matted tar ridge streaks across the haunch.
                        if u < 1.2 * s && ((u / s) * 2.4).sin() > 0.55 && v.abs() < 1.3 * s {
                            c = mix(c, tar_sh, 0.45);
                        }
                        fx.c(c)
                    }
                };
                plot_sdf(pb, (x, y), d, rim_lim, col, (rim_col, 0.72));
            } else {
                // Ground drop shadow under the galloping body and stiff tail.
                let su = u + du_sh;
                let sv = v + dv_sh;
                if su >= u_min && su <= u_max && sv.abs() <= v_max && eval_rat(su, sv).0 < 0.40 * s {
                    darken_px(pb, x, y, 0.36);
                }
            }
            u += fx.f.x;
            v += fx.p.x;
        }
    }

    // 3) Low snout tip, fierce piercing road-yellow eyes and stiff tar whiskers.
    let nose = fx.px(5.35 * s + lunge, 0.0);
    pb.plot(nose.0, nose.1, fx.c((204, 132, 126)));
    let eye_c = if fx.hit > 0.70 {
        (28, 22, 16)
    } else {
        (255, 226, 68)
    };
    for side in [-1.0f32, 1.0] {
        let ep = fx.px(3.65 * s + lunge * 0.6, side * 0.92 * s);
        pb.plot(ep.0, ep.1, eye_c);
        let w = fx.px(4.35 * s + lunge * 0.6, side * 1.95 * s);
        blend_px(pb, w.0, w.1, fx.c((134, 140, 156)), 0.55);
    }
}

/// Far LOD: a glossy black dash with an oily silver back ridge, yellow eye
/// glint, stepping gallop paw and stiff pennant cross.
pub(crate) fn draw_tar_rat_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    tar_rat_lod(pb, e, cam);
}

fn tar_rat_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam, 4.0);
    let s = fx.s;
    let tar = fx.c((26, 24, 30));
    let sheen = fx.c((182, 192, 210));
    let a = fx.px(-2.4 * s, 0.0);
    let b = fx.px(3.0 * s, 0.0);
    let tail = fx.px(-5.6 * s, 0.0);
    thick_line(pb, a, b, tar, (8, 8, 10));
    seg(pb, a, tail, tar);
    // Solid mini pennant + bright mast edge.
    let fa = fx.px(-4.6 * s, -1.3 * s);
    let fb = fx.px(-4.6 * s, 1.3 * s);
    seg(pb, fa, tail, tar);
    seg(pb, fb, tail, tar);
    seg(pb, fa, fb, sheen);
    // Oily dorsal highlight & yellow eye glint.
    let back = fx.px(0.4 * s, 0.0);
    pb.plot(back.0, back.1 - 1, sheen);
    let eye = fx.px(2.8 * s, 0.0);
    pb.plot(eye.0, eye.1, fx.c(ROAD_YELLOW_HI));
    // Alternating gallop paw.
    let paw = if fx.frame == 0 {
        fx.px(2.8 * s, 1.8 * s)
    } else {
        fx.px(-2.2 * s, 1.8 * s)
    };
    pb.plot(paw.0, paw.1, tar);
}

// ---- Loose Can -------------------------------------------------------------

/// What the can is doing, for the body's pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanPose {
    /// Trundling toward the rat.
    Walk,
    /// The tell: rocking in place, ring pull beating against the deck.
    Rock,
    /// Rolling along its marked path.
    Roll,
    /// Lying still after the roll (the punish window).
    Rest,
}

/// The pose a Loose Can is in, read from the phase `floor_flatbed` mirrors
/// onto the body.
#[inline]
pub fn can_pose(e: &Enemy) -> CanPose {
    match e.w4.phase {
        rules::CAN_PHASE_ROCK => CanPose::Rock,
        rules::CAN_PHASE_ROLL => CanPose::Roll,
        rules::CAN_PHASE_REST => CanPose::Rest,
        _ => CanPose::Walk,
    }
}

/// Loose Can: a dented horizontal aluminium cylinder lying across its roll
/// heading, with crimped steel end caps, a rolling painted label stripe and a
/// road-yellow ring pull.
///
/// * **Walk**: specular stripe and label rotate smoothly around the barrel as
///   the ring pull clacks up and down.
/// * **Rock (tell)**: rocks briskly along the roll axis while the ring pull
///   slaps back and forth against the deck with a road-yellow spark.
/// * **Roll (attack)**: fast spinning specular glint, ring pull whipped back
///   and aluminium motion streaks trailing behind.
/// * **Rest (punish window)**: still, muted sheen, ring pull lying limp and
///   flat beside the end cap.
pub(crate) fn draw_loose_can(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_can(pb, e, cam);
}

fn loose_can(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let mut fx = Fx::of(e, cam, 2.6);
    let s = fx.s;
    let pose = can_pose(e);
    let rock_beat = ((e.w4.warn_age * 10.0) as i32 & 1) as u8;
    let rock_t = if pose == CanPose::Rock {
        (e.w4.warn_age / rules::CAN_ROCK_SECS).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if pose == CanPose::Rock {
        let shift = (1.05 + 0.45 * rock_t) * s * if rock_beat == 0 { 1.0 } else { -1.0 };
        fx.pos = Vec2::new(fx.pos.x + fx.f.x * shift, fx.pos.y + fx.f.y * shift);
    }

    let rim_col = fx.rim((158, 166, 178));

    // 1) Ground shadow + fast roll motion streaks behind the cylinder.
    shadow_ellipse(pb, fx.pos.x + 0.6 * s, fx.pos.y + 2.6 * s, 4.4 * s, 1.8 * s, 0.38);
    if pose == CanPose::Roll {
        let streak_c = fx.c((232, 236, 242));
        for &v in &[-2.8f32, 0.0, 2.8] {
            let a = fx.px(-2.8 * s, v * s);
            let b = fx.px(-5.2 * s, v * s);
            line_blend(pb, a, b, streak_c, 0.52);
        }
    }

    // 2) 3D-shaded horizontal dented cylinder (axis along `v`, diameter along `u`).
    let alu_mid = mix(e.kind.color(), ALU_MID, 0.5);
    let label_sh = (96, 28, 24);
    let label_mid = (186, 56, 46);
    let label_hi = (242, 124, 98);
    let cap_dark = (58, 64, 74);

    // Rotation angle around the cylinder circumference drives the rolling stripe & specular bar.
    let roll_sin = match pose {
        CanPose::Rock => if rock_beat == 0 { 0.65 } else { -0.65 },
        CanPose::Rest => -0.35,
        CanPose::Roll => (fx.anim * 1.4).sin(),
        CanPose::Walk => (fx.anim * 0.95).sin(),
    };
    let spec_u = roll_sin * 1.25 * s;
    let band_u = -roll_sin * 1.05 * s;

    let hu = 2.35 * s;
    let hv = 4.25 * s;
    let rim_lim = 0.88 * s;

    if let Some((x0, x1, y0, y1)) = fx.bbox_oriented(pb, -3.5 * s, 3.5 * s, 5.4 * s, 0, 0) {
        for y in y0..=y1 {
            let (mut u, mut v) = fx.local(x0, y);
            for x in x0..=x1 {
                let box_d = sd_rounded_box(u, v, hu, hv, 0.72 * s);
                // Real carved V-dent pinching one side of the aluminium wall.
                let dent_d = (u - 1.55 * s).hypot(v + 1.35 * s) - 1.15 * s;
                let d = box_d.max(-dent_d * 0.58);
                if d < rim_lim {
                    let av = v.abs();
                    let nu = (u / hu).clamp(-1.0, 1.0);
                    let cyl_curve = (1.0 - nu * nu * 0.75).clamp(0.0, 1.0);
                    let lit = fx.lit(nu, (v / hv).clamp(-1.0, 1.0) * 0.35);
                    let lum = if pose == CanPose::Rest {
                        0.32 + 0.30 * cyl_curve + 0.42 * lit
                    } else {
                        0.38 + 0.36 * cyl_curve + 0.56 * lit
                    };

                    let mut col = if av > 3.55 * s {
                        // Recessed circular metal end caps.
                        shade3(cap_dark, ALU_LO, alu_mid, lum * 0.82)
                    } else if av > 3.05 * s {
                        // Raised bright crimped lip at both ends of the can.
                        shade3(ALU_LO, ALU_HI, (255, 255, 255), lum + 0.18)
                    } else if av < 1.85 * s && (u - band_u).abs() < 1.15 * s {
                        // Painted highway red/yellow label stripe rolling around the barrel.
                        let mut lc = shade3(label_sh, label_mid, label_hi, lum);
                        if (av - 1.55 * s).abs() < 0.38 * s {
                            lc = mix(lc, ROAD_YELLOW, 0.75);
                        }
                        lc
                    } else {
                        shade3(ALU_LO, alu_mid, ALU_HI, lum)
                    };

                    // Darkened dent pocket with bright crinkled crease along its rim.
                    if dent_d < 0.25 * s {
                        col = mix(col, cap_dark, 0.62);
                    } else if dent_d < 0.75 * s && av < 3.0 * s {
                        col = mix(col, ALU_HI, 0.52);
                    }

                    // Anisotropic specular bar sliding across the aluminium barrel.
                    if pose != CanPose::Rest && av < 3.2 * s && (u - spec_u).abs() < 0.58 * s {
                        let boost = if pose == CanPose::Roll { 0.88 } else { 0.68 };
                        col = mix(col, (255, 255, 255), boost);
                    }
                    // Warm road-yellow wind-up glint on the leading edge while rocking.
                    if pose == CanPose::Rock && u > 1.1 * s && av < 3.1 * s {
                        let glint = (0.35 + 0.45 * rock_t).clamp(0.0, 0.85);
                        col = mix(col, ROAD_YELLOW_HI, glint);
                    }
                    plot_sdf(pb, (x, y), d, rim_lim, fx.c(col), (rim_col, 0.72));
                }
                u += fx.f.x;
                v += fx.p.x;
            }
        }
    }

    // 3) Road-yellow ring pull at the +v lid end (open loop + bright glint).
    let ring_col = fx.c(ROAD_YELLOW);
    let ring_hi = fx.c(ROAD_YELLOW_HI);
    let rivet = fx.px(0.0, 4.3 * s);
    pb.plot(rivet.0, rivet.1, fx.c(ALU_LO));
    match pose {
        CanPose::Rest => {
            // Limp and flat on the deck beside the can.
            let mid = fx.px(0.0, 5.4 * s);
            let tip = fx.px(0.0, 6.4 * s);
            let rim = fx.px(0.8 * s, 5.8 * s);
            line_blend(pb, rivet, tip, fx.c((168, 138, 46)), 0.92);
            pb.plot(mid.0, mid.1, ring_col);
            blend_px(pb, rim.0, rim.1, fx.c((120, 96, 32)), 0.80);
        }
        CanPose::Roll => {
            // Whipped tight back behind the spinning lid.
            let a = fx.px(-1.2 * s, 5.4 * s);
            let b = fx.px(-1.8 * s, 6.2 * s);
            thick_line(pb, rivet, a, ring_hi, ring_col);
            pb.plot(b.0, b.1, ring_hi);
        }
        CanPose::Rock => {
            // Slapping back and forth against the deck with a bright spark pixel!
            let dir = if rock_beat == 0 { 1.0 } else { -1.0 };
            let a = fx.px(dir * 1.1 * s, 5.2 * s);
            let b = fx.px(dir * 2.1 * s, 6.1 * s);
            let c = fx.px(dir * 0.4 * s, 6.2 * s);
            thick_line(pb, rivet, b, ring_hi, ring_col);
            line_blend(pb, b, c, ring_col, 0.92);
            pb.plot(a.0, a.1, ring_hi);
            pb.plot_add(b.0, b.1, (90, 76, 24));
        }
        CanPose::Walk => {
            let swing = fx.anim.sin() * 1.55 * s;
            let a = fx.px(swing * 0.55, 5.3 * s);
            let b = fx.px(swing, 6.2 * s);
            let c = fx.px(swing * 0.2, 6.1 * s);
            thick_line(pb, rivet, b, ring_hi, ring_col);
            line_blend(pb, a, c, ring_col, 0.88);
        }
    }
}

/// Far LOD: a dark-rimmed bare-aluminium cylinder bar across the heading with
/// a stepping specular glint and a road-yellow ring-pull end.
pub(crate) fn draw_loose_can_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_can_lod(pb, e, cam);
}

fn loose_can_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam, 2.6);
    let s = fx.s;
    let pose = can_pose(e);
    let a = fx.px(0.0, -2.5 * s);
    let b = fx.px(0.0, 2.5 * s);
    thick_line(pb, a, b, fx.c(e.kind.color()), fx.c(ALU_LO));
    let mid = fx.px(0.0, 0.0);
    pb.plot(mid.0, mid.1, fx.c((186, 56, 46)));
    let step = if fx.frame == 0 { -1.2 * s } else { 1.2 * s };
    let spec = fx.px(0.0, step);
    pb.plot(spec.0, spec.1, fx.c(ALU_HI));
    let pull_u = match pose {
        CanPose::Rest => 0.0,
        CanPose::Rock => if ((e.w4.warn_age * 10.0) as i32 & 1) == 0 { 1.0 * s } else { -1.0 * s },
        _ => if fx.frame == 0 { 0.8 * s } else { -0.8 * s },
    };
    let r = fx.px(pull_u, 3.6 * s);
    pb.plot(r.0, r.1, fx.c(ROAD_YELLOW_HI));
}

/// The Loose Can's warned footprint, drawn from the **same** path the roll
/// hits along ([`rules::can_path`]): the outline of the 6 px lane on each leg,
/// a bright pip at the bounce, a cross at the stop.
///
/// Call it from the hostile layer (`art_enemy::draw_wave4_tells`) for a body
/// whose [`can_pose`] is `Rock` or `Roll`.
pub fn draw_loose_can_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let pose = can_pose(e);
    if pose != CanPose::Rock && pose != CanPose::Roll {
        return;
    }
    let Some(path) = rules::can_path(e) else {
        return;
    };
    // The outline brightens through the tell and holds while the can rolls.
    let t = if pose == CanPose::Rock {
        (e.w4.warn_age / rules::CAN_ROCK_SECS).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let edge = mix((150, 112, 36), ROAD_YELLOW, t);
    let half = rules::CAN_LANE_HALF_WIDTH;
    let to_px = |p: Vec2| ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32);
    for leg in 0..path.legs() {
        let (a, b) = path.leg(leg);
        let d = Vec2::new(b.x - a.x, b.y - a.y);
        let len = (d.x * d.x + d.y * d.y).sqrt();
        if len < 0.5 {
            continue;
        }
        let n = Vec2::new(-d.y / len * half, d.x / len * half);
        for side in [-1.0f32, 1.0] {
            let o = Vec2::new(n.x * side, n.y * side);
            seg(
                pb,
                to_px(Vec2::new(a.x + o.x, a.y + o.y)),
                to_px(Vec2::new(b.x + o.x, b.y + o.y)),
                edge,
            );
        }
    }
    if path.legs() > 1 {
        let (bx, by) = to_px(path.points[1]);
        pb.plot(bx, by, TIE_WHITE);
        pb.plot(bx + 1, by, ROAD_YELLOW);
        pb.plot(bx - 1, by, ROAD_YELLOW);
        pb.plot(bx, by + 1, ROAD_YELLOW);
        pb.plot(bx, by - 1, ROAD_YELLOW);
    }
    let (ex, ey) = to_px(path.end());
    for d in -2i32..=2 {
        pb.plot(ex + d, ey + d, TIE_WHITE);
        pb.plot(ex + d, ey - d, TIE_WHITE);
    }
}

// ---- Toll Gull -------------------------------------------------------------

/// Toll Gull: sculpted snow-white and coastal-slate elbow wings, jet-black
/// primary tips, a long flat road-yellow beak, hovering broadside.
///
/// Each wing is a thick tapered airfoil bent forward at the elbow and swept
/// back into a black primary tip with a white mirror dot; smooth wingbeats
/// extend and tuck the tips above a grounded hover shadow. Tell pose: in the
/// last [`rules::GULL_BEAK_OPEN_SECS`] before a shot the long flat beak splits
/// wide into an open V with a crimson throat and swelling white-gold charge
/// glint: "reverse as the beak opens".
pub(crate) fn draw_toll_gull(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    toll_gull(pb, e, cam);
}

fn toll_gull(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fx = Fx::of(e, cam, 3.0);
    let s = fx.s;
    let flap = fx.anim.sin(); // -1..1
    let open = gull_beak_open(e);
    let charge = if open {
        1.0 - (e.fire_cd / rules::GULL_BEAK_OPEN_SECS).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let recoil = if e.fire_cd > 1.95 {
        ((e.fire_cd - 1.95) * 3.5).clamp(0.0, 1.0) * 0.75 * s
    } else {
        0.0
    };

    // 1) Elevated hover shadow on the deck below the hovering gull.
    let shadow_rx = (3.4 + 0.7 * flap) * s;
    shadow_ellipse(pb, fx.pos.x + 0.5 * s, fx.pos.y + 5.6 * s, shadow_rx, 1.45 * s, 0.38);

    // Subtle vertical hover bob.
    let mut gfx = fx;
    gfx.pos.y += -0.55 * flap * s;
    let rim_col = gfx.rim((164, 170, 182));

    // 2) Sculpted elbow-bent M-wings + streamlined white torso + fanned tail.
    let span = if open {
        (6.8 + 0.5 * flap) * s
    } else {
        (6.4 + 1.35 * flap) * s
    };
    let elbow_u = (2.05 + 0.35 * flap - 0.3 * charge) * s;
    let elbow_v = (3.75 + 0.25 * flap) * s;
    let tip_u = (-2.1 + 0.85 * flap - 0.45 * charge) * s;

    let inner_wing = TaperSdf::new((0.15 * s, 0.9 * s), (elbow_u, elbow_v), 1.32 * s, 1.08 * s);
    let outer_wing = TaperSdf::new((elbow_u, elbow_v), (tip_u, span), 1.06 * s, 0.44 * s);
    let torso = EllipseSdf::new(-0.15 * s, 0.0, 2.55 * s, 1.52 * s);
    let head = EllipseSdf::new(2.25 * s - recoil, 0.0, 1.25 * s, 1.22 * s);
    let tail = TaperSdf::new((-2.0 * s, 0.0), (-4.2 * s, 0.0), 0.72 * s, 1.32 * s);

    let white_hi = (252, 252, 248);
    let white_mid = e.kind.color();
    let white_sh = (146, 154, 168);
    let mantle_sh = (86, 96, 114);
    let mantle_mid = (148, 160, 178);
    let mantle_hi = (224, 232, 242);
    let primary_blk = (24, 26, 32);

    if let Some((x0, x1, y0, y1)) =
        gfx.bbox_oriented(pb, -5.2 * s, 4.4 * s, span + 1.4 * s, 0, 0)
    {
        let rim_lim = 0.86 * s;
        for y in y0..=y1 {
            let (mut u, mut v) = gfx.local(x0, y);
            for x in x0..=x1 {
                let av = v.abs();
                let (d_in, t_in) = inner_wing.eval_t(u, av);
                let (d_out, t_out) = outer_wing.eval_t(u, av);
                let d_wing = d_in.min(d_out);
                let d_body = smin(torso.eval(u, v), head.eval(u, v), 0.42 * s).min(tail.eval(u, v));
                let d = d_body.min(d_wing);
                if d < rim_lim {
                    let col = if d_body <= d_wing {
                        let nu = ((u - 0.4 * s) / (3.4 * s)).clamp(-1.0, 1.0);
                        let nv = (v / (1.5 * s)).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.75)).clamp(0.0, 1.0);
                        let lum = 0.46 + 0.34 * dome + 0.50 * gfx.lit(nu, nv);
                        let mut c = shade3(white_sh, white_mid, white_hi, lum);
                        // Dark subterminal tail bar.
                        if u < -3.45 * s {
                            c = mix(c, (58, 64, 76), 0.72);
                        }
                        gfx.c(c)
                    } else {
                        let side_sign = if v >= 0.0 { 1.0 } else { -1.0 };
                        let lit = gfx.lit(0.35, side_sign * 0.65);
                        if d_out <= d_in && t_out > 0.62 {
                            // Jet-black primary wingtip with a tiny white gull mirror spot.
                            let mirror = (t_out - 0.78).abs() < 0.09 && d_out < -0.15 * s;
                            let c = if mirror {
                                white_hi
                            } else {
                                mix(primary_blk, (64, 70, 84), (0.35 + 0.4 * lit).clamp(0.0, 0.7))
                            };
                            gfx.c(c)
                        } else {
                            // Bright snow-white leading wing edge over cool coastal slate-grey mantle.
                            let lead_u = if d_in <= d_out {
                                0.15 * s + (elbow_u - 0.15 * s) * t_in
                            } else {
                                elbow_u + (tip_u - elbow_u) * t_out
                            };
                            let leading = u >= lead_u - 0.15 * s;
                            let c = if leading {
                                shade3(white_sh, white_mid, white_hi, 0.68 + 0.38 * lit)
                            } else {
                                shade3(mantle_sh, mantle_mid, mantle_hi, 0.46 + 0.46 * lit)
                            };
                            gfx.c(c)
                        }
                    };
                    plot_sdf(pb, (x, y), d, rim_lim, col, (rim_col, 0.70));
                }
                u += gfx.f.x;
                v += gfx.p.x;
            }
        }
    }

    // 3) Long flat road-yellow beak (splits wide open with glowing throat tell before firing!).
    let beak_mid = gfx.c(ROAD_YELLOW);
    let beak_hi = gfx.c(ROAD_YELLOW_HI);
    let beak_lo = gfx.c((142, 104, 24));
    let base_u = 3.1 * s - recoil;
    let tip_u_b = 6.7 * s - recoil;
    if open {
        let gape = (1.65 + 0.55 * charge) * s;
        // Vivid crimson-red open gullet inside the V.
        let g0 = gfx.px(base_u + 0.3 * s, 0.0);
        let g1 = gfx.px(base_u + 1.7 * s, 0.0);
        thick_line(pb, g0, g1, gfx.c((214, 52, 54)), gfx.c((118, 22, 26)));
        // Upper and lower splayed yellow mandibles with dark shadow rims.
        for side in [-1.0f32, 1.0] {
            let b0 = gfx.px(base_u, side * 0.45 * s);
            let b1 = gfx.px(tip_u_b, side * gape);
            thick_line(pb, b0, b1, if side < 0.0 { beak_hi } else { beak_mid }, beak_lo);
        }
        // Swelling white-gold shot orb between the open jaws!
        let orb = gfx.px(base_u + 2.25 * s, 0.0);
        let orb_c = mix(ROAD_YELLOW_HI, (255, 255, 240), 0.35 + 0.65 * charge);
        pb.plot(orb.0, orb.1, orb_c);
        if charge > 0.30 {
            let glow = scale_rgb(orb_c, 0.42 + 0.45 * charge);
            pb.plot_add(orb.0 + 1, orb.1, glow);
            pb.plot_add(orb.0 - 1, orb.1, glow);
            pb.plot_add(orb.0, orb.1 + 1, glow);
            pb.plot_add(orb.0, orb.1 - 1, glow);
        }
    } else {
        // Thick flat yellow bill with the gull's signature crimson lower-tip spot.
        let b0 = gfx.px(base_u, 0.0);
        let b1 = gfx.px(tip_u_b, 0.0);
        thick_line(pb, b0, b1, beak_hi, beak_lo);
        let spot = gfx.px(tip_u_b - 1.2 * s, 0.45 * s);
        pb.plot(spot.0, spot.1, gfx.c((212, 56, 48)));
    }

    // 4) Piercing dark-rimmed amber gull eyes on both sides of the white head.
    let eye_c = if gfx.hit > 0.65 {
        (28, 24, 22)
    } else {
        (255, 206, 54)
    };
    for side in [-1.0f32, 1.0] {
        let ep = gfx.px(2.35 * s - recoil, side * 0.92 * s);
        let pupil = gfx.px(2.55 * s - recoil, side * 1.05 * s);
        pb.plot(ep.0, ep.1, eye_c);
        blend_px(pb, pupil.0, pupil.1, (22, 22, 26), 0.85);
    }
}

/// Is the gull's beak open (the shot is less than
/// [`rules::GULL_BEAK_OPEN_SECS`] away)?
#[inline]
pub fn gull_beak_open(e: &Enemy) -> bool {
    e.stun <= 0.0 && e.fire_cd <= rules::GULL_BEAK_OPEN_SECS
}

/// Far LOD: a crisp white-and-slate M-wing silhouette with black primary tips,
/// a bold road-yellow beak and an open-beak charge glint.
pub(crate) fn draw_toll_gull_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    toll_gull_lod(pb, e, cam);
}

fn toll_gull_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam, 3.0);
    let s = fx.s;
    let white = fx.c(e.kind.color());
    let slate = fx.c((118, 128, 144));
    let blk = fx.c((22, 24, 30));
    let span = if fx.frame == 0 { 4.2 * s } else { 2.9 * s };
    let nose = fx.px(1.2 * s, 0.0);
    for side in [-1.0f32, 1.0] {
        let elbow = fx.px(1.6 * s, side * 1.8 * s);
        let tip = fx.px(-1.2 * s, side * span);
        thick_line(pb, nose, elbow, white, slate);
        thick_line(pb, elbow, tip, white, slate);
        pb.plot(tip.0, tip.1, blk);
    }
    if gull_beak_open(e) {
        let b1 = fx.px(3.2 * s, -1.0 * s);
        let b2 = fx.px(3.2 * s, 1.0 * s);
        let g = fx.px(2.6 * s, 0.0);
        pb.plot(b1.0, b1.1, fx.c(ROAD_YELLOW_HI));
        pb.plot(b2.0, b2.1, fx.c(ROAD_YELLOW_HI));
        pb.plot(g.0, g.1, (226, 62, 58));
    } else {
        let b0 = fx.px(2.1 * s, 0.0);
        let b1 = fx.px(3.5 * s, 0.0);
        seg(pb, b0, b1, fx.c(ROAD_YELLOW));
    }
}

// ---- Loose Bungee ----------------------------------------------------------

/// Loose Bungee: a forged-steel J-hook and brass crimp ferrule dragging a
/// thick braided crimson-and-white striped rubber loop.
///
/// The hook leads on a 2 px sculpted steel shank with a dipped safety tip on
/// the J-return. Behind the ferrule trails a continuous tubular striped
/// elastic loop that whips side-to-side as it moves. Tell pose: while its
/// crossbar is telling (`bar_telling`), the loop snaps pulled wire-taut and
/// quivers while the ferrule and hook bend blaze with road-yellow tension
/// glints.
pub(crate) fn draw_loose_bungee(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_bungee(pb, e, cam);
}

fn loose_bungee(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fx = Fx::of(e, cam, 4.0);
    let s = fx.s;
    let taut = e.w3.bar_telling();
    let live = e.w3.bar_t > 0.0 && !taut && !e.w3.bar_spent;
    let rim_col = fx.rim((154, 134, 134));

    let (ru, rv, sway) = if taut {
        (4.85 * s, 1.32 * s, (fx.anim * 9.0).sin() * 0.35 * s)
    } else if live {
        (3.95 * s, 1.85 * s, fx.anim.sin() * 0.55 * s)
    } else {
        (3.45 * s, 2.45 * s, fx.anim.sin() * 1.65 * s)
    };
    let centre_u = -1.1 * s - ru;
    let cord_r = 0.88 * s;

    // 1) Continuous tubular braided crimson-and-white striped rubber loop + forged J-hook.
    let rubber = e.kind.color();
    let red_sh = (82, 22, 22);
    let red_mid = mix(rubber, (198, 62, 58), 0.5);
    let red_hi = (248, 126, 114);
    let wht_sh = (134, 128, 118);
    let wht_mid = TIE_WHITE;
    let wht_hi = (255, 254, 248);

    let steel_sh = (50, 56, 68);
    let steel_mid = (168, 178, 192);
    let steel_hi = (248, 252, 255);
    let brass_sh = (118, 82, 20);
    let brass_mid = ROAD_YELLOW;
    let brass_hi = (255, 246, 184);

    let shank = CapsuleSdf::new((0.5 * s, 0.0), (4.2 * s, 0.0), 0.78 * s);
    let bend_a = CapsuleSdf::new((4.2 * s, 0.0), (5.55 * s, 1.35 * s), 0.78 * s);
    let bend_b = CapsuleSdf::new((5.55 * s, 1.35 * s), (4.35 * s, 3.05 * s), 0.76 * s);
    let barb = CapsuleSdf::new((4.35 * s, 3.05 * s), (2.15 * s, 3.15 * s), 0.72 * s);
    let ferrule = EllipseSdf::new(-0.25 * s, 0.0, 1.05 * s, 1.25 * s);

    let u_min = centre_u - ru - cord_r - 1.5 * s;
    let u_max = 6.8 * s;
    let v_max = (rv + sway.abs() + cord_r + 1.5 * s).max(4.6 * s);
    let Some((x0, x1, y0, y1)) = fx.bbox_oriented(pb, u_min, u_max, v_max, 1, 2) else {
        return;
    };

    let inv_ru = 1.0 / ru.max(0.5);
    let inv_rv = 1.0 / rv.max(0.5);
    let inv_span_u = 1.0 / (2.0 * ru).max(0.5);
    let du_sh = -(fx.f.x + 1.4 * fx.f.y);
    let dv_sh = -(fx.p.x + 1.4 * fx.p.y);
    let rim_lim = 0.86 * s;

    let eval_bungee = |u: f32, v: f32| -> (f32, u8, f32) {
        // Loop SDF with first-order Euclidean gradient normalization so the cord
        // keeps uniform 2-px thickness both slack and stretched taut.
        let d_loop = if u <= 0.6 * s {
            let tail_w = ((centre_u + ru - u) * inv_span_u).clamp(0.0, 1.0);
            let v_adj = v - sway * tail_w;
            let du = (u - centre_u) * inv_ru;
            let dv = v_adj * inv_rv;
            let r_el = du.hypot(dv);
            let grad = (du * inv_ru).hypot(dv * inv_rv).max(0.12);
            (r_el - 1.0).abs() / grad - cord_r
        } else {
            10.0
        };
        let d_ferr = if u > -2.4 * s && u < 2.2 * s && v.abs() < 2.6 * s {
            ferrule.eval(u, v)
        } else {
            10.0
        };
        let d_hook = if u > -0.8 * s {
            shank
                .eval(u, v)
                .min(bend_a.eval(u, v))
                .min(bend_b.eval(u, v))
                .min(barb.eval(u, v))
        } else {
            10.0
        };

        if d_ferr <= d_loop && d_ferr <= d_hook {
            (d_ferr, 2, 0.0)
        } else if d_hook <= d_loop {
            (d_hook, 1, 0.0)
        } else {
            let tail_w = ((centre_u + ru - u) * inv_span_u).clamp(0.0, 1.0);
            let v_adj = v - sway * tail_w;
            let theta = v_adj.atan2(u - centre_u);
            (d_loop, 0, theta)
        }
    };

    for y in y0..=y1 {
        let (mut u, mut v) = fx.local(x0, y);
        for x in x0..=x1 {
            let (d, part, theta) = eval_bungee(u, v);
            if d < rim_lim {
                let col = match part {
                    2 => {
                        // Brass/road-yellow crimping collar anchoring cord to hook.
                        let nu = ((u + 0.25 * s) / (1.05 * s)).clamp(-1.0, 1.0);
                        let nv = (v / (1.25 * s)).clamp(-1.0, 1.0);
                        let lum = 0.44 + 0.56 * fx.lit(nu, nv) + if taut { 0.26 } else { 0.0 };
                        let mut c = shade3(brass_sh, brass_mid, brass_hi, lum);
                        if (u + 0.25 * s).abs() < 0.28 * s {
                            c = mix(c, brass_sh, 0.55);
                        }
                        fx.c(c)
                    }
                    1 => {
                        // Forged brushed-steel J-hook with dipped vinyl safety tip on the barb.
                        let bevel = (-d / (0.8 * s)).clamp(0.0, 1.0);
                        let lit = fx.lit(0.25, if v > 1.8 * s { 0.7 } else { -0.7 });
                        let is_tip = u < 3.1 * s && v > 2.2 * s;
                        let c = if is_tip {
                            shade3(red_sh, red_mid, red_hi, 0.44 + 0.36 * bevel + 0.48 * lit)
                        } else {
                            let mut lum = 0.36 + 0.40 * bevel + 0.62 * lit;
                            if lit > 0.22 && bevel > 0.35 {
                                lum += 0.28;
                            }
                            shade3(steel_sh, steel_mid, steel_hi, lum)
                        };
                        fx.c(c)
                    }
                    _ => {
                        // Alternating bold crimson-and-white braided elastic stripes.
                        let stripe = ((theta * 5.0 / PI).floor() as i32) & 1 == 0;
                        let bevel = (-d / cord_r).clamp(0.0, 1.0);
                        let lit = fx.lit(theta.cos() * 0.5, theta.sin() * 0.7);
                        let lum = 0.38 + 0.38 * bevel + 0.52 * lit + if taut { 0.14 } else { 0.0 };
                        let c = if stripe {
                            shade3(red_sh, red_mid, red_hi, lum)
                        } else {
                            shade3(wht_sh, wht_mid, wht_hi, lum)
                        };
                        fx.c(c)
                    }
                };
                plot_sdf(pb, (x, y), d, rim_lim, col, (rim_col, 0.70));
            } else {
                let su = u + du_sh;
                let sv = v + dv_sh;
                if su >= u_min && su <= u_max && sv.abs() <= v_max && eval_bungee(su, sv).0 < 0.35 * s {
                    darken_px(pb, x, y, 0.34);
                }
            }
            u += fx.f.x;
            v += fx.p.x;
        }
    }

    // 2) Specular steel hook glint + bright road-yellow tension spark on tell / live bar.
    let bend_glint = fx.px(4.85 * s, 0.75 * s);
    pb.plot(
        bend_glint.0,
        bend_glint.1,
        fx.c(if taut { (255, 255, 245) } else { ALU_HI }),
    );
    if taut {
        let g = fx.px(5.45 * s, 1.45 * s);
        pb.plot(g.0, g.1, ROAD_YELLOW_HI);
        pb.plot_add(g.0 + 1, g.1, (120, 96, 28));
        pb.plot_add(g.0 - 1, g.1, (120, 96, 28));
        pb.plot_add(g.0, g.1 + 1, (120, 96, 28));
        pb.plot_add(g.0, g.1 - 1, (120, 96, 28));
        let fc = fx.px(-0.25 * s, 0.0);
        pb.plot(fc.0, fc.1, (255, 252, 224));
    } else if live {
        let fc = fx.px(-0.25 * s, 0.0);
        pb.plot(fc.0, fc.1, ROAD_YELLOW_HI);
    }
}

/// Far LOD: a bright steel J-hook leading a brass ferrule and swaying
/// crimson-and-white rubber loop (pulls straight with a yellow glint on tell).
pub(crate) fn draw_loose_bungee_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_bungee_lod(pb, e, cam);
}

fn loose_bungee_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam, 4.0);
    let s = fx.s;
    let taut = e.w3.bar_telling();
    let steel = fx.c((188, 196, 208));
    let steel_lo = fx.c((64, 70, 82));
    // Forged steel J-hook corner.
    let h0 = fx.px(0.0, 0.0);
    let h1 = fx.px(2.8 * s, 0.0);
    let h2 = fx.px(3.2 * s, 1.5 * s);
    let h3 = fx.px(1.8 * s, 1.8 * s);
    thick_line(pb, h0, h1, steel, steel_lo);
    seg(pb, h1, h2, steel);
    seg(pb, h2, h3, fx.c(e.kind.color()));
    // Brass crimp ferrule.
    pb.plot(h0.0, h0.1, fx.c(if taut { (255, 252, 220) } else { ROAD_YELLOW }));
    // Striped trailing loop.
    let sway = if taut {
        0.0
    } else if fx.frame == 0 {
        1.2 * s
    } else {
        -1.2 * s
    };
    let tail_u = if taut { -5.2 * s } else { -3.8 * s };
    let l1 = fx.px(-1.7 * s, sway);
    let l2 = fx.px(tail_u, sway * 0.5);
    let l3 = fx.px(-2.2 * s, -sway * 0.6);
    pb.plot(l1.0, l1.1, fx.c(e.kind.color()));
    pb.plot(l2.0, l2.1, fx.c(TIE_WHITE));
    pb.plot(l3.0, l3.1, fx.c(e.kind.color()));
    if taut {
        pb.plot(h2.0, h2.1, ROAD_YELLOW_HI);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const FLOOR: Rgb = (40, 30, 22);
    type Inner = fn(&mut PixelBuffer, &Enemy, Vec2);

    const FULL: [(EnemyKind, Inner); 6] = [
        (EnemyKind::GravelTick, gravel_tick),
        (EnemyKind::JuneBug, june_bug),
        (EnemyKind::TarRat, tar_rat),
        (EnemyKind::LooseCan, loose_can),
        (EnemyKind::TollGull, toll_gull),
        (EnemyKind::LooseBungee, loose_bungee),
    ];
    const LOD: [(EnemyKind, Inner); 6] = [
        (EnemyKind::GravelTick, gravel_tick_lod),
        (EnemyKind::JuneBug, june_bug_lod),
        (EnemyKind::TarRat, tar_rat_lod),
        (EnemyKind::LooseCan, loose_can_lod),
        (EnemyKind::TollGull, toll_gull_lod),
        (EnemyKind::LooseBungee, loose_bungee_lod),
    ];

    fn body(kind: EnemyKind, anim: f32) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(kind, Vec2::new(24.0, 24.0), 1.0, &mut rng);
        e.anim = anim;
        e.fire_cd = 2.0;
        e
    }

    /// The painted mask of one draw on a flat floor.
    fn mask(draw: Inner, e: &Enemy) -> Vec<bool> {
        let mut pb = PixelBuffer::new(48, 48);
        pb.clear(FLOOR);
        draw(&mut pb, e, Vec2::new(0.0, 0.0));
        pb.pixels().iter().map(|&c| c != FLOOR).collect()
    }

    fn extent(m: &[bool]) -> (usize, usize, usize) {
        let (mut x0, mut x1, mut y0, mut y1, mut n) = (48, 0, 48, 0, 0);
        for (i, &on) in m.iter().enumerate() {
            if on {
                let (x, y) = (i % 48, i / 48);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
                n += 1;
            }
        }
        (x1.saturating_sub(x0) + 1, y1.saturating_sub(y0) + 1, n)
    }

    #[test]
    fn every_flatbed_body_paints_a_readable_core() {
        for (kind, draw) in FULL {
            let (w, h, n) = extent(&mask(draw, &body(kind, 0.5)));
            assert!(n >= 14, "{kind:?} paints only {n} pixels");
            assert!(w.max(h) >= 6, "{kind:?} core is {w}x{h}");
            assert!(w.max(h) <= 28, "{kind:?} sprawls to {w}x{h}");
        }
    }

    #[test]
    fn every_flatbed_body_has_two_walk_frames() {
        assert_ne!(walk_frame(0.5), walk_frame(4.0));
        for (kind, draw) in FULL.into_iter().chain(LOD) {
            let a = mask(draw, &body(kind, 0.5));
            let b = mask(draw, &body(kind, 4.0));
            let mut pa = PixelBuffer::new(48, 48);
            let mut pbuf = PixelBuffer::new(48, 48);
            pa.clear(FLOOR);
            pbuf.clear(FLOOR);
            draw(&mut pa, &body(kind, 0.5), Vec2::new(0.0, 0.0));
            draw(&mut pbuf, &body(kind, 4.0), Vec2::new(0.0, 0.0));
            assert!(
                a != b || pa.pixels() != pbuf.pixels(),
                "{kind:?} draws the same picture on both walk frames"
            );
        }
    }

    #[test]
    fn silhouettes_are_pairwise_distinct() {
        for table in [FULL, LOD] {
            for i in 0..table.len() {
                for j in (i + 1)..table.len() {
                    let a = mask(table[i].1, &body(table[i].0, 0.5));
                    let b = mask(table[j].1, &body(table[j].0, 0.5));
                    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
                    assert!(
                        differ >= 3,
                        "{:?} and {:?} differ in only {differ} pixels",
                        table[i].0,
                        table[j].0
                    );
                }
            }
        }
    }

    #[test]
    fn far_lod_is_smaller_than_the_full_body() {
        for ((kind, full), (_, lod)) in FULL.into_iter().zip(LOD) {
            let (_, _, n_full) = extent(&mask(full, &body(kind, 0.5)));
            let (_, _, n_lod) = extent(&mask(lod, &body(kind, 0.5)));
            assert!(n_lod >= 3, "{kind:?} LOD paints {n_lod} pixels");
            assert!(n_lod < n_full, "{kind:?} LOD ({n_lod}) is not lighter than full ({n_full})");
        }
    }

    #[test]
    fn each_body_has_a_light_and_a_dark_edge() {
        for (kind, draw) in FULL {
            let mut pb = PixelBuffer::new(48, 48);
            pb.clear(FLOOR);
            draw(&mut pb, &body(kind, 0.5), Vec2::new(0.0, 0.0));
            let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
            let painted: Vec<u32> = pb
                .pixels()
                .iter()
                .filter(|&&c| c != FLOOR)
                .map(|&c| lum(c))
                .collect();
            let hi = painted.iter().copied().max().unwrap_or(0);
            let lo = painted.iter().copied().min().unwrap_or(0);
            assert!(hi >= lo + 150, "{kind:?} has no edge contrast ({lo}..{hi})");
        }
    }

    #[test]
    fn the_gull_opens_its_beak_before_the_shot() {
        let mut e = body(EnemyKind::TollGull, 0.5);
        let closed = mask(toll_gull, &e);
        e.fire_cd = rules::GULL_BEAK_OPEN_SECS * 0.5;
        assert!(gull_beak_open(&e));
        let open = mask(toll_gull, &e);
        assert_ne!(closed, open, "the open beak is the tell");
        e.stun = 1.0;
        assert!(!gull_beak_open(&e), "a stunned gull cannot fire, so no tell");
    }

    #[test]
    fn the_can_rocks_and_rests_in_different_poses() {
        let mut e = body(EnemyKind::LooseCan, 0.5);
        assert_eq!(can_pose(&e), CanPose::Walk);
        let walk = mask(loose_can, &e);
        e.w4.phase = rules::CAN_PHASE_ROCK;
        e.w4.warn_age = 0.05;
        let rock_a = mask(loose_can, &e);
        e.w4.warn_age = 0.15;
        let rock_b = mask(loose_can, &e);
        assert_ne!(rock_a, rock_b, "the rock beats");
        e.w4.phase = rules::CAN_PHASE_ROLL;
        let roll = mask(loose_can, &e);
        assert_ne!(walk, roll, "rolling adds speed streaks and whips the ring pull");
        e.w4.phase = rules::CAN_PHASE_REST;
        assert_eq!(can_pose(&e), CanPose::Rest);
        let rest = mask(loose_can, &e);
        assert_ne!(roll, rest, "resting lies still with a flat ring pull");
    }

    #[test]
    fn the_bungee_pulls_taut_while_its_bar_tells() {
        let mut e = body(EnemyKind::LooseBungee, 0.5);
        let slack = mask(loose_bungee, &e);
        e.w3.bar_t = crate::game::ecs::systems::enemy_wave3::CROSSBAR_TELL_SECS
            + crate::game::ecs::systems::enemy_wave3::CROSSBAR_LIVE_SECS;
        assert!(e.w3.bar_telling());
        assert_ne!(slack, mask(loose_bungee, &e));
    }

    #[test]
    fn the_can_tell_draws_only_while_warned() {
        let mut e = body(EnemyKind::LooseCan, 0.5);
        let mut pb = PixelBuffer::new(96, 96);
        pb.clear(FLOOR);
        draw_loose_can_tell(&mut pb, &e, Vec2::new(0.0, 0.0));
        assert!(pb.pixels().iter().all(|&c| c == FLOOR), "no tell while walking");
        rules::store_can_path(
            &mut e,
            &rules::CanPath::straight(Vec2::new(24.0, 24.0), Vec2::new(72.0, 24.0)),
        );
        e.w4.phase = rules::CAN_PHASE_ROCK;
        draw_loose_can_tell(&mut pb, &e, Vec2::new(0.0, 0.0));
        let lit = pb.pixels().iter().filter(|&&c| c != FLOOR).count();
        assert!(lit >= 80, "two 48 px lane edges, got {lit}");
    }

    #[test]
    fn hit_flash_brightens_every_flatbed_body_and_lod() {
        for (kind, draw) in FULL.into_iter().chain(LOD) {
            let sum_lum = |flash: f32| -> u64 {
                let mut pb = PixelBuffer::new(48, 48);
                pb.clear(FLOOR);
                let mut e = body(kind, 0.5);
                e.flash = flash;
                draw(&mut pb, &e, Vec2::ZERO);
                pb.pixels()
                    .iter()
                    .map(|&c| c.0 as u64 + c.1 as u64 + c.2 as u64)
                    .sum()
            };
            let normal = sum_lum(0.0);
            let hit = sum_lum(FLASH_MAX);
            assert!(
                hit > normal + 40,
                "{kind:?}: hit flash ({hit}) must brighten over normal ({normal})"
            );
        }
    }

    #[test]
    fn lod_preserves_gull_and_bungee_tells() {
        let mut gull = body(EnemyKind::TollGull, 0.5);
        let closed = mask(toll_gull_lod, &gull);
        gull.fire_cd = rules::GULL_BEAK_OPEN_SECS * 0.4;
        assert_ne!(closed, mask(toll_gull_lod, &gull), "gull LOD reflects open beak");

        let mut bungee = body(EnemyKind::LooseBungee, 0.5);
        let slack = mask(loose_bungee_lod, &bungee);
        bungee.w3.bar_t = crate::game::ecs::systems::enemy_wave3::CROSSBAR_TELL_SECS
            + crate::game::ecs::systems::enemy_wave3::CROSSBAR_LIVE_SECS;
        assert_ne!(slack, mask(loose_bungee_lod, &bungee), "bungee LOD reflects taut tell");
    }

    #[test]
    fn flatbed_draws_survive_degenerate_buffers_and_orientations() {
        for (bw, bh) in [(0usize, 0usize), (1, 1), (4, 4), (48, 48)] {
            let mut pb = PixelBuffer::new(bw, bh);
            for (kind, draw) in FULL.into_iter().chain(LOD) {
                for step in 0..8 {
                    let ang = step as f32 * (std::f32::consts::TAU / 8.0);
                    let mut e = body(kind, step as f32 * 0.9);
                    e.facing = if step == 0 {
                        Vec2::ZERO
                    } else {
                        Vec2::new(ang.cos(), ang.sin())
                    };
                    e.flash = if step % 2 == 0 { FLASH_MAX } else { 0.0 };
                    e.contact_cd = if step % 3 == 0 { 0.4 } else { 0.0 };
                    e.fire_cd = if step % 3 == 1 { 0.1 } else { 2.0 };
                    draw(&mut pb, &e, Vec2::ZERO);
                    draw(&mut pb, &e, Vec2::new(200.0, 200.0));
                }
            }
        }
    }
}
