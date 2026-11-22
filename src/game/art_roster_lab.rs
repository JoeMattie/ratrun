//! The Lab's own bodies (batch 6b, polished in batch 8): Swab Foot, Spinner
//! Lid, Glass Runner, Walking Syringe and Autoclave Cart.
//!
//! Palette (`WAVE4_ROSTERS.md`, Lab): white ceramic feet, black rubber and
//! specimen pink against console glow. Every body here has a 6–12 px core, a
//! crisp dark silhouette rim, directional top-left lighting (`LIGHT`), a bright
//! rim or specimen/console glint that survives pooled low ambient light, smooth
//! multi-frame gait motion, tell and contact poses where the kind has them, and
//! a far-LOD twin that keeps the silhouette readable in a handful of pixels.
//!
//! Each draw has the `BodyDraw` signature `art_enemy::bespoke_body` expects.
//! The shared `Frame` is accepted and ignored: these bodies build their own
//! small local frame from the `Enemy`, so this file depends on nothing private
//! in `art_enemy` beyond the type's name.
//!
//! | kind | silhouette | walk frames | tell / attack pose |
//! |---|---|---|---|
//! | Swab Foot | plump cotton club with pink dipped tip on a bent black rubber stem + ceramic foot | kinked stem rows left/right, cotton head paddles fore/aft | forward pink-smear lunge on contact |
//! | Spinner Lid | hovering slanted chrome jar lid, pink gasket, white pinch knob | rim glints spin, disk breathes broad face ↔ thin edge from weave clock | razor-sharp gleaming blade sliver at the flip |
//! | Glass Runner | translucent teardrop ampoule with sloshing pink fluid on two long rubber legs + white ceramic boots | long opposing vaulting stride, fluid sloshes | forward glass-tip jab on contact |
//! | Walking Syringe | graduated glass/steel barrel, wide T-flange, sliding plunger, 4 bent stilts | diagonal alternating stilt gait | stilts brace wide, plunger draws back, barrel glows pink → flush white-hot thrust → empty retract |
//! | Autoclave Cart | heavy steel sterilizer box, glowing cyan porthole, rear push-bar, two big treaded wheels | wheel treads & spokes turn, front brass latch oscillates | forward ram on contact; elite wears double gold shoulder stripes and an open crown of safety latches |
//!
//! The Walking Syringe's ray is drawn by [`draw_syringe_tell`] from the same
//! segment the hit test uses (`floor_lab::syringe_ray`).

use crate::game::ecs::systems::floor_lab;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

use super::art_enemy::Frame;
use super::enemy::Enemy;

/// Direction toward the light in screen space (matches `art_enemy::LIGHT`).
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };
/// Hit-flash peak (matches `art_enemy::FLASH_MAX`).
const FLASH_MAX: f32 = 0.12;

// Lab clinical palette: dark rim, vulcanized black rubber (with readable sheen),
// cool white ceramic, surgical steel, lab glass, specimen pink, console cyan,
// and safety-latch brass/gold.
const RIM_DARK: Rgb = (8, 12, 18);

const RUBBER_DARK: Rgb = (20, 22, 30);
const RUBBER_MID: Rgb = (44, 48, 60);
const RUBBER_HI: Rgb = (112, 124, 144);
const RUBBER_KNEE: Rgb = (148, 160, 180);

const CERAMIC_DARK: Rgb = (114, 126, 146);
const CERAMIC_MID: Rgb = (218, 226, 238);
const CERAMIC_HI: Rgb = (252, 255, 255);

const STEEL_DARK: Rgb = (44, 52, 66);
const STEEL_MID: Rgb = (140, 152, 170);
const STEEL_HI: Rgb = (230, 240, 250);

const GLASS_DARK: Rgb = (38, 76, 88);
const GLASS_TINT: Rgb = (152, 220, 216);
const GLASS_HI: Rgb = (242, 255, 252);

const PINK_DARK: Rgb = (150, 44, 90);
const SPECIMEN_PINK: Rgb = (238, 92, 154);
const PINK_HI: Rgb = (255, 184, 218);

const CONSOLE_DARK: Rgb = (26, 72, 92);
const CONSOLE_MID: Rgb = (104, 218, 238);
const CONSOLE_HI: Rgb = (232, 254, 255);

const BRASS_DARK: Rgb = (124, 82, 24);
const BRASS_MID: Rgb = (228, 178, 58);

// Elite gold tones (strictly satisfy r > 240, g > 190, b < 170 for distinct
// readability and test pinning against plain brass).
const ELITE_GOLD_DARK: Rgb = (132, 84, 20);
const ELITE_GOLD: Rgb = (254, 204, 72);
const ELITE_GOLD_HI: Rgb = (255, 236, 142);

/// A body's local screen frame (`u` forward along `e.facing`, `v` right-hand).
#[derive(Clone, Copy)]
struct Local {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    hit: f32,
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
        Local {
            pos: e.pos - cam,
            f,
            p,
            fl: f.x * LIGHT.x + f.y * LIGHT.y,
            pl: p.x * LIGHT.x + p.y * LIGHT.y,
            hit: (e.flash / FLASH_MAX).clamp(0.0, 1.0),
        }
    }

    /// Body-space `(u forward, v sideways)` to a screen pixel.
    #[inline]
    fn at(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    /// Directional lighting dot product for a local normal `(nu, nv)`.
    #[inline]
    fn lit_dot(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    /// A body colour with the warm sculptural hit flash folded in.
    #[inline]
    fn tint(&self, c: Rgb) -> Rgb {
        if self.hit <= 0.0 {
            c
        } else {
            let flash = if self.hit > 0.6 {
                (255, 244, 218)
            } else {
                (255, 222, 160)
            };
            mix(c, flash, (0.30 + self.hit * 0.56).min(0.86))
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
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.60 {
        mix(shadow, mid, l * (1.0 / 0.60))
    } else {
        mix(mid, hi, (l - 0.60) * (1.0 / 0.55))
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

/// A soft contact shadow pixel.
#[inline]
fn shade(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    blend(pb, x, y, (0, 0, 0), amount);
}

/// Soft elliptical ground shadow.
fn shadow_oval(pb: &mut PixelBuffer, cx: i32, cy: i32, rx: i32, ry: i32, amount: f32) {
    if rx <= 0 || ry <= 0 {
        shade(pb, cx, cy, amount);
        return;
    }
    let rx2 = (rx * rx) as f32;
    let ry2 = (ry * ry) as f32;
    for dy in -ry..=ry {
        for dx in -rx..=rx {
            let r2 = (dx * dx) as f32 / rx2 + (dy * dy) as f32 / ry2;
            if r2 <= 1.0 {
                shade(pb, cx + dx, cy + dy, amount * (1.0 - r2 * 0.45));
            }
        }
    }
}

/// Bounds-safe blended Bresenham line.
fn line_blend(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    loop {
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

/// Thick two-tone limb segment: a dark shadow/core edge plus a lit sheen edge
/// offset toward the up-left light so black rubber limbs stay crisp against
/// dark Lab tiles.
fn line_lit(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), dark: Rgb, hi: Rgb) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let horiz = dx >= -dy;
    let (ox, oy) = if horiz { (0, -1) } else { (-1, 0) };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    loop {
        // Dark underside/rim first, then core, then subtle lit sheen edge.
        shade(pb, x + 1, y + 1, 0.32);
        blend(pb, x + ox, y + oy, hi, 0.88);
        pb.plot(x, y, dark);
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

/// Two-frame binary phase for compact far-LOD alternation.
#[inline]
fn step(e: &Enemy) -> bool {
    (e.anim * 0.5).sin() > 0.0
}

// ---- Swab Foot ---------------------------------------------------------------

/// Cotton club on a bent black rubber stem with a small white ceramic foot:
/// a plump sculpted cotton bud leading with a specimen-pink dipped tip,
/// crimped by a steel collar onto a visibly kinked glossy-rubber stalk. As it
/// crawls, the stalk rows left and right while the cotton head paddles forward.
pub(crate) fn draw_swabfoot(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let phase = (e.anim * 0.5).sin();
    let cphase = (e.anim * 0.5).cos();
    let lunge = if e.contact_cd > 0.25 { 1.1 } else { 0.0 };

    let bend = phase * 2.1;
    let surge = cphase * 0.7 + lunge;
    let head_u = 1.8 + surge;
    let head_v = -phase * 0.4;

    let foot = l.at(-4.5 + cphase * 0.45, -bend * 0.55);
    let knee = l.at(-1.7, bend);
    let neck = l.at(head_u - 1.6, head_v * 0.35);
    let head_c = l.at(head_u + 0.5, head_v);

    // Ground shadows beneath cotton bud and kicking foot.
    shadow_oval(pb, head_c.0 + 1, head_c.1 + 2, 2, 1, 0.34);
    shade(pb, foot.0 + 1, foot.1 + 1, 0.32);

    // Bent black rubber stem with crisp up-left specular sheen so it never
    // vanishes into dark Lab tiles.
    line_lit(pb, foot, knee, l.tint(RUBBER_DARK), l.tint(RUBBER_HI));
    line_lit(pb, knee, neck, l.tint(RUBBER_DARK), l.tint(RUBBER_HI));
    pb.plot(knee.0, knee.1, l.tint(RUBBER_KNEE));

    // Small white ceramic foot pad at the tail end pushing off the floor.
    let heel = l.at(-5.1 + cphase * 0.45, -bend * 0.55);
    pb.plot(heel.0, heel.1, l.tint(CERAMIC_MID));
    pb.plot(foot.0, foot.1, l.tint(CERAMIC_HI));

    // Metallic ferrule collar crimping the cotton head onto the stem.
    for sv in [-0.9_f32, 0.0, 0.9] {
        let p = l.at(head_u - 1.5, head_v + sv);
        let c = if l.lit_dot(-0.2, sv) > 0.0 {
            STEEL_HI
        } else {
            STEEL_DARK
        };
        pb.plot(p.0, p.1, l.tint(c));
    }

    // Plump elongated cotton-bud club with directional shading, dark rim, and
    // a vivid specimen-pink dipped leading tip.
    let ru = 2.65_f32;
    let rv = 2.15_f32;
    let cx = head_c.0;
    let cy = head_c.1;
    let rim_col = if l.hit > 0.4 { (140, 120, 130) } else { RIM_DARK };

    for dy in -4i32..=4 {
        for dx in -4i32..=4 {
            let rx = (cx + dx) as f32 - (l.pos.x + l.f.x * (head_u + 0.5) + l.p.x * head_v);
            let ry = (cy + dy) as f32 - (l.pos.y + l.f.y * (head_u + 0.5) + l.p.y * head_v);
            let du = rx * l.f.x + ry * l.f.y;
            let dv = rx * l.p.x + ry * l.p.y;
            let nu = du / ru;
            let nv = dv / rv;
            let q = nu * nu + nv * nv;
            if q > 1.72 {
                continue;
            }
            if q > 1.0 {
                blend(pb, cx + dx, cy + dy, rim_col, 0.62);
                continue;
            }
            let dome = (1.0 - q).max(0.0);
            let dir_lit = l.lit_dot(nu, nv);
            let lum = 0.44 + 0.34 * dome + 0.56 * dir_lit;
            let mut col = shade3(CERAMIC_DARK, CERAMIC_MID, CERAMIC_HI, lum);
            // Subtle cotton fiber dimple near the middle of the bud.
            if (du + 0.4).abs() < 0.55 && dv.abs() > 0.65 && dv.abs() < 1.3 {
                col = mix(col, CERAMIC_DARK, 0.22);
            }
            // Specimen-pink stain soaked into the leading tip.
            if du > 0.85 {
                let stain = ((du - 0.85) * 0.72).clamp(0.0, 1.0);
                let pink = shade3(PINK_DARK, SPECIMEN_PINK, PINK_HI, lum + 0.12);
                col = mix(col, pink, stain);
            }
            pb.plot(cx + dx, cy + dy, l.tint(col));
        }
    }

    // Wet pink-white glint on the leading tip (survives pooled dark ambient).
    let tip = l.at(head_u + 2.6, head_v - 0.35);
    let glint = if lunge > 0.0 {
        (255, 224, 240)
    } else {
        PINK_HI
    };
    blend(pb, tip.0, tip.1, l.tint(glint), 0.92);
}

pub(crate) fn draw_swabfoot_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    let side = if step(e) { 1.3 } else { -1.3 };
    let tail = l.at(-3.5, -side * 0.4);
    let mid = l.at(-1.2, side);
    let neck = l.at(0.4, 0.0);
    let head = l.at(1.8, 0.0);

    // Readable bent rubber stem + ceramic foot dot.
    line_blend(pb, tail, mid, l.tint(RUBBER_HI), 0.90);
    line_blend(pb, mid, neck, l.tint(RUBBER_DARK), 0.95);
    pb.plot(mid.0, mid.1, l.tint(RUBBER_HI));
    pb.plot(tail.0, tail.1, l.tint(CERAMIC_MID));

    // Compact cotton club + specimen-pink leading nose.
    shade(pb, head.0 + 1, head.1 + 1, 0.35);
    pb.plot(head.0, head.1, l.tint(CERAMIC_MID));
    pb.plot(head.0 - 1, head.1 - 1, l.tint(CERAMIC_HI));
    pb.plot(head.0, head.1 + 1, l.tint(CERAMIC_DARK));
    let tip = l.at(3.0, 0.0);
    pb.plot(tip.0, tip.1, l.tint(SPECIMEN_PINK));
}

// ---- Spinner Lid ------------------------------------------------------------

/// Floating slanted specimen-jar lid: major axis on a crisp slant in screen
/// space, minor axis breathing with `floor_lab::lid_face` (1 = broad face,
/// 0 = razor-sharp edge-on blade at the weave-flip instant). Orbiting specular
/// rim glints show spin direction reversing with `floor_lab::lid_flipped`.
fn lid_disk(pb: &mut PixelBuffer, l: &Local, e: &Enemy, major: f32, lift: i32, detailed: bool) {
    let face = floor_lab::lid_face(e);
    let flipped = floor_lab::lid_flipped(e.w4.t);
    let minor = (major * 0.76 * face).max(0.55);

    // Slanted axis tilts subtly with handedness while keeping a crisp angle.
    let (ax, ay) = if flipped {
        (0.915_f32, -0.403_f32)
    } else {
        (0.940_f32, -0.341_f32)
    };
    let (bx, by) = (-ay, ax);

    let bob = i32::from(detailed && (e.anim * 0.35).sin() > 0.25);
    let hover = lift + bob;
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32 - hover;
    let reach = major.ceil() as i32 + 2;

    // Hovering shadow on the floor below, compressing as the lid turns edge-on.
    let sh_rx = ((major * 0.75).round() as i32).max(1);
    let sh_ry = ((major * 0.38 * face).round() as i32).max(1);
    shadow_oval(pb, cx + 1, cy + hover + 2, sh_rx, sh_ry, 0.32);

    // Spinning rim glint direction reverses when the weave flips.
    let spin = if flipped {
        -e.anim * 0.55
    } else {
        e.anim * 0.55
    };
    let (ss, cs) = (spin.sin(), spin.cos());

    let rim_col = if l.hit > 0.4 { (120, 148, 160) } else { RIM_DARK };
    let dish_dark = (22, 50, 68);
    let dish_mid = mix(e.kind.color(), (42, 108, 134), 0.42);
    let dish_hi = (176, 232, 244);

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let a = dx as f32 * ax + dy as f32 * ay;
            let b = dx as f32 * bx + dy as f32 * by;
            let na = a / major;
            let nb = b / minor;
            let q = na * na + nb * nb;
            if q > 1.0 {
                // Tight dark silhouette rim outside the disk.
                let approx_d = (q.sqrt() - 1.0) * minor.min(major);
                if approx_d < 0.85 {
                    blend(pb, cx + dx, cy + dy, rim_col, 0.62);
                }
                continue;
            }
            let dir_lit = (dx as f32 * LIGHT.x + dy as f32 * LIGHT.y) / major.max(1.0);
            let c = if q > 0.56 || minor < 1.15 {
                // Beveled bright chrome/cyan rim with orbiting spin glints.
                let base = shade3(CONSOLE_DARK, CONSOLE_MID, CONSOLE_HI, 0.56 + 0.68 * dir_lit);
                let glint = (na * cs + nb * ss).abs();
                if detailed && glint > 0.90 && q > 0.62 {
                    (255, 255, 255)
                } else {
                    base
                }
            } else if detailed && q > 0.40 && minor >= 1.4 {
                // Specimen-pink rubber seal gasket nestled inside the rim.
                let gasket = shade3(PINK_DARK, SPECIMEN_PINK, PINK_HI, 0.52 + 0.45 * dir_lit);
                mix(dish_mid, gasket, 0.65)
            } else if q < 0.14 {
                // Raised white ceramic pinch-knob at the center of the lid.
                shade3(CERAMIC_DARK, CERAMIC_MID, CERAMIC_HI, 0.70 + 0.55 * dir_lit)
            } else {
                // Concave brushed steel dish (inverted light catches far wall).
                shade3(dish_dark, dish_mid, dish_hi, 0.46 - 0.36 * dir_lit)
            };
            pb.plot(cx + dx, cy + dy, l.tint(c));
        }
    }

    // Edge-on flip tell: unbroken razor-bright metallic blade with flashing tip
    // glints so the player spots the exact handedness reversal at a glance.
    if face < 0.24 {
        let m = major.round() as i32;
        for k in -m..=m {
            let x = cx + (k as f32 * ax).round() as i32;
            let y = cy + (k as f32 * ay).round() as i32;
            let tip = k.abs() >= m;
            let top_c = if tip {
                (255, 226, 240)
            } else if k % 2 == 0 {
                (250, 255, 255)
            } else {
                CONSOLE_HI
            };
            pb.plot(x, y, l.tint(top_c));
            blend(pb, x, y + 1, l.tint(CONSOLE_DARK), 0.90);
            if detailed {
                blend(pb, x, y - 1, rim_col, 0.45);
            }
        }
    }
}

/// Floating slanted disk whose bright rim alternates broad face and thin
/// edge. It is edge-on exactly when its weave flips handedness.
pub(crate) fn draw_spinnerlid(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    lid_disk(pb, &l, e, 4.8, 2, true);
}

pub(crate) fn draw_spinnerlid_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    lid_disk(pb, &l, e, 2.8, 1, false);
}

// ---- Glass Runner -----------------------------------------------------------

/// Clear ampoule on two long rubber feet: a translucent teardrop glass bulb
/// with sloshing specimen-pink liquid inside and a snapped-off neck leading,
/// mounted on exactly two long articulated black-rubber legs with heavy
/// rubber-and-ceramic boots that vault fore and aft in opposition.
pub(crate) fn draw_glassrunner(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    let phase = (e.anim * 0.5).sin();
    let cphase = (e.anim * 0.5).cos();
    let lunge = if e.contact_cd > 0.25 { 1.2 } else { 0.0 };

    let swing = phase * 3.8;
    // Vaulting cadence: the ampoule surges forward slightly on each stride.
    let ride = cphase.abs() * 0.85 + lunge;

    // Two long articulated black-rubber legs with heavy boots and white ceramic
    // toe caps (drawn behind the translucent ampoule).
    for (side, sw, lift_k) in [
        (1.0_f32, swing, cphase.max(0.0)),
        (-1.0_f32, -swing, (-cphase).max(0.0)),
    ] {
        let hip = l.at(ride - 0.8, side * 1.8);
        let knee = l.at(ride + sw * 0.42 - 0.3, side * (3.9 + lift_k * 0.55));
        let ankle = l.at(sw, side * 5.2);

        shadow_oval(pb, ankle.0 + 1, ankle.1 + 2, 1, 1, 0.34);
        line_lit(pb, hip, knee, l.tint(RUBBER_DARK), l.tint(RUBBER_HI));
        line_lit(pb, knee, ankle, l.tint(RUBBER_DARK), l.tint(RUBBER_HI));
        pb.plot(knee.0, knee.1, l.tint(RUBBER_KNEE));

        // Chunky vulcanized rubber boot heel + sole with readable sheen.
        for (du, dv, col) in [
            (-0.8_f32, 0.0_f32, RUBBER_DARK),
            (0.0, 0.0, RUBBER_MID),
            (0.0, side * 0.9, RUBBER_DARK),
            (0.8, 0.0, RUBBER_HI),
            (0.8, side * 0.9, RUBBER_MID),
        ] {
            let (x, y) = l.at(sw + du, side * 5.2 + dv);
            pb.plot(x, y, l.tint(col));
        }
        // White ceramic toe bumper pointing forward on each foot.
        let toe0 = l.at(sw + 1.7, side * 5.1);
        let toe1 = l.at(sw + 2.2, side * 4.8);
        pb.plot(toe0.0, toe0.1, l.tint(CERAMIC_MID));
        pb.plot(toe1.0, toe1.1, l.tint(CERAMIC_HI));
    }

    // Translucent glass ampoule: fat bulb at rear tapering cleanly into a
    // pinched neck, with sloshing specimen-pink fluid inside.
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    let glass = mix(e.kind.color(), GLASS_TINT, 0.35);
    let fill_u = -0.25 - phase * 0.65 + lunge * 1.3;
    let rim_col = if l.hit > 0.4 { (120, 150, 148) } else { RIM_DARK };
    let reach = 9i32;

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let rx = (cx + dx) as f32 - l.pos.x;
            let ry = (cy + dy) as f32 - l.pos.y;
            let u = rx * l.f.x + ry * l.f.y - ride;
            let v = rx * l.p.x + ry * l.p.y;
            let cu = u.clamp(-2.4, 3.2);
            let rad = 2.95 - (cu + 2.4) * 0.34;
            let du = u - cu;
            let d = (du * du + v * v).sqrt() - rad;
            if d > 0.85 {
                continue;
            }
            if d > 0.0 {
                blend(pb, cx + dx, cy + dy, rim_col, 0.64);
                continue;
            }
            let norm_inv = 1.0 / rad.max(0.5);
            let dir_lit = l.lit_dot(du * norm_inv, v * norm_inv);
            if d > -0.95 {
                // Glass wall: hard white-mint specular on lit side, deep teal
                // refraction on shadow side.
                let c = if dir_lit > 0.38 {
                    (255, 255, 255)
                } else if dir_lit > -0.10 {
                    GLASS_HI
                } else {
                    GLASS_DARK
                };
                pb.plot(cx + dx, cy + dy, l.tint(c));
            } else {
                // Translucent glass interior (floor subtly visible through it).
                blend(pb, cx + dx, cy + dy, l.tint(glass), 0.48);
                // Sloshing specimen-pink fluid filling the rear bulb.
                if u < fill_u {
                    let meniscus = (fill_u - u) < 0.72;
                    let fluid = if meniscus {
                        PINK_HI
                    } else {
                        shade3(PINK_DARK, SPECIMEN_PINK, PINK_HI, 0.52 + 0.42 * dir_lit)
                    };
                    blend(pb, cx + dx, cy + dy, l.tint(fluid), if meniscus { 0.92 } else { 0.84 });
                } else if dir_lit > 0.30 && d > -1.7 {
                    // Inner curved specular streak down the clear glass shoulder.
                    blend(pb, cx + dx, cy + dy, l.tint(GLASS_HI), 0.72);
                }
            }
        }
    }

    // Pinched glass collar ring and snapped-off sharp ampoule tip.
    for sv in [-1.0_f32, 1.0] {
        let ring = l.at(ride + 3.8, sv);
        pb.plot(ring.0, ring.1, l.tint(GLASS_HI));
    }
    let n0 = l.at(ride + 4.1, 0.0);
    let n1 = l.at(ride + 6.4, 0.0);
    line_lit(pb, n0, n1, l.tint(GLASS_DARK), l.tint(GLASS_HI));
    pb.plot(
        n1.0,
        n1.1,
        l.tint(if lunge > 0.0 {
            (255, 255, 255)
        } else {
            GLASS_HI
        }),
    );
}

pub(crate) fn draw_glassrunner_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    let sw = if step(e) { 2.4 } else { -2.4 };

    // Two opposing long rubber/ceramic feet.
    for (side, s_u) in [(1.0_f32, sw), (-1.0_f32, -sw)] {
        let hip = l.at(-0.5, side * 1.4);
        let foot = l.at(s_u, side * 3.8);
        line_blend(pb, hip, foot, l.tint(RUBBER_HI), 0.90);
        pb.plot(foot.0, foot.1, l.tint(RUBBER_DARK));
        let toe = l.at(s_u + 1.0, side * 3.7);
        pb.plot(toe.0, toe.1, l.tint(CERAMIC_HI));
    }

    // Compact teardrop glass vial + pink specimen core + bright snapped tip.
    let back = l.at(-2.8, 0.0);
    let tip = l.at(4.2, 0.0);
    line_blend(pb, back, tip, l.tint(GLASS_TINT), 0.92);
    for sv in [-1.2_f32, 1.2] {
        let wall = l.at(-1.4, sv);
        let c = if l.lit_dot(-0.2, sv) > 0.0 {
            GLASS_HI
        } else {
            GLASS_DARK
        };
        pb.plot(wall.0, wall.1, l.tint(c));
    }
    let core = l.at(-1.4, 0.0);
    pb.plot(core.0, core.1, l.tint(SPECIMEN_PINK));
    pb.plot(tip.0, tip.1, l.tint((255, 255, 255)));
}

// ---- Walking Syringe --------------------------------------------------------

/// Pose tuple `(plunger_pull, specimen_fill, planted, thrusting, retracting)`
/// derived from `floor_lab::syringe_pose`.
fn plunger_pose(e: &Enemy) -> (f32, f32, bool, bool, bool) {
    match floor_lab::syringe_pose(e) {
        floor_lab::SyringePose::Walk => (1.0, 0.0, false, false, false),
        floor_lab::SyringePose::Tell(k) => (1.0 + 3.2 * k, k, true, false, false),
        floor_lab::SyringePose::Thrust => (0.0, 0.0, true, true, false),
        floor_lab::SyringePose::Retract(k) => (1.0 * k, 0.0, true, false, true),
    }
}

/// Needle on four bent stilt legs: graduated glass-and-steel barrel, signature
/// wide finger flange, sliding ceramic thumb plunger, and a long gleaming
/// needle whose tip sits at `floor_lab::SYRINGE_TIP`. While telling, the stilts
/// splay wide to brace, the plunger draws far back, and the barrel fills with
/// glowing specimen pink; on thrust the plunger slams flush and the needle
/// flashes white-hot; on retract the empty barrel sags during the punish window.
pub(crate) fn draw_walkingsyringe(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    let (pull, fill, planted, thrusting, retracting) = plunger_pose(e);
    let phase = (e.anim * 0.5).sin();

    // Four articulated mechanical stilt legs (front pair + rear pair).
    let spread = if thrusting || fill > 0.0 {
        5.5
    } else if retracting {
        4.6
    } else {
        4.0
    };
    let (fa, fb) = if planted {
        let brace = if thrusting { -0.6 } else { 0.5 };
        (brace, -brace)
    } else {
        (phase * 2.0, -phase * 2.0)
    };
    let stilts = [
        (1.6_f32, 1.0_f32, 2.3 + fa),
        (1.6, -1.0, 2.3 + fb),
        (-2.0, 1.0, -2.5 + fb),
        (-2.0, -1.0, -2.5 + fa),
    ];
    for (hu, side, fu) in stilts {
        let hip = l.at(hu, side * 1.4);
        let knee = l.at(hu + (fu - hu) * 0.45, side * spread * 0.62);
        let foot = l.at(fu, side * spread);
        shade(pb, foot.0 + 1, foot.1 + 1, 0.34);
        line_lit(pb, hip, knee, l.tint(STEEL_DARK), l.tint(STEEL_MID));
        line_lit(pb, knee, foot, l.tint(STEEL_DARK), l.tint(RUBBER_HI));
        pb.plot(knee.0, knee.1, l.tint(STEEL_HI));
        pb.plot(foot.0, foot.1, l.tint(RUBBER_DARK));
    }

    // Graduated glass-and-steel barrel (`u` in [-3.2, +2.8], `v` in [-1.7, +1.7]).
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    let barrel_mid = if retracting {
        mix(e.kind.color(), STEEL_DARK, 0.35)
    } else {
        e.kind.color()
    };
    let rim_col = if l.hit > 0.4 { (140, 148, 162) } else { RIM_DARK };
    let fill_back_u = 2.7 - 5.9 * fill;
    let reach = 6i32;

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let rx = (cx + dx) as f32 - l.pos.x;
            let ry = (cy + dy) as f32 - l.pos.y;
            let u = rx * l.f.x + ry * l.f.y;
            let v = rx * l.p.x + ry * l.p.y;
            let av = v.abs();
            if u < -4.0 || u > 3.6 || av > 2.4 {
                continue;
            }
            if u < -3.2 || u > 2.8 || av > 1.65 {
                blend(pb, cx + dx, cy + dy, rim_col, 0.62);
                continue;
            }
            let dir_lit = l.lit_dot(0.0, v * 0.6);
            let col = if av > 0.95 {
                // Outer cylindrical glass/steel barrel walls.
                if dir_lit >= -0.05 {
                    STEEL_HI
                } else {
                    STEEL_DARK
                }
            } else if thrusting {
                // Hot discharge flash inside the barrel during thrust.
                if av < 0.45 {
                    (255, 244, 232)
                } else {
                    PINK_HI
                }
            } else if fill > 0.0 && u >= fill_back_u {
                // Glowing specimen-pink fluid filling the chamber as the
                // plunger pulls back.
                let edge = (u - fill_back_u) < 0.65;
                if edge {
                    PINK_HI
                } else {
                    shade3(PINK_DARK, SPECIMEN_PINK, PINK_HI, 0.56 + 0.45 * dir_lit)
                }
            } else {
                // Translucent chamber with subtle dosage graduation ticks.
                let tick = (u + 1.6).abs() < 0.35 || (u - 0.1).abs() < 0.35 || (u - 1.6).abs() < 0.35;
                if tick && v > 0.25 {
                    STEEL_DARK
                } else {
                    shade3(STEEL_DARK, barrel_mid, GLASS_HI, 0.50 + 0.42 * dir_lit)
                }
            };
            pb.plot(cx + dx, cy + dy, l.tint(col));
        }
    }

    // Wide signature finger flange crossbar at the rear of the barrel.
    for vi in -3i32..=3 {
        let (x, y) = l.at(-3.2, vi as f32);
        let c = if vi.abs() == 3 {
            STEEL_DARK
        } else if l.lit_dot(-0.2, vi as f32 * 0.3) > 0.0 {
            CERAMIC_HI
        } else {
            STEEL_HI
        };
        pb.plot(x, y, l.tint(c));
    }

    // Sliding plunger rod + white ceramic thumb plate (draws back during Tell,
    // slams flush on Thrust).
    let rod_end = -4.2 - pull;
    let r0 = l.at(-3.6, 0.0);
    let r1 = l.at(rod_end, 0.0);
    line_lit(pb, r0, r1, l.tint(STEEL_DARK), l.tint(CERAMIC_MID));
    for vi in -2i32..=2 {
        let (x, y) = l.at(rod_end - 0.7, vi as f32 * 0.85);
        let c = if vi.abs() == 2 {
            CERAMIC_DARK
        } else if l.lit_dot(-0.3, vi as f32 * 0.4) > 0.0 {
            CERAMIC_HI
        } else {
            CERAMIC_MID
        };
        pb.plot(x, y, l.tint(c));
    }

    // Black-rubber Luer collar and long gleaming needle to exact `SYRINGE_TIP`.
    for sv in [-0.7_f32, 0.0, 0.7] {
        let col_p = l.at(3.2, sv);
        pb.plot(
            col_p.0,
            col_p.1,
            l.tint(if sv == 0.0 { RUBBER_HI } else { RUBBER_DARK }),
        );
    }
    let n0 = l.at(4.0, 0.0);
    let n1 = l.at(floor_lab::SYRINGE_TIP, 0.0);
    let needle_hi = if thrusting {
        (255, 250, 224)
    } else {
        (248, 252, 255)
    };
    line_lit(pb, n0, n1, l.tint(STEEL_MID), l.tint(needle_hi));
    pb.plot(n1.0, n1.1, l.tint(needle_hi));

    // Glowing specimen-pink bead forming on the needle tip during late Tell.
    if fill > 0.35 {
        blend(pb, n1.0, n1.1, PINK_HI, 0.85);
    }
}

pub(crate) fn draw_walkingsyringe_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    _elite: bool,
) {
    let l = Local::of(e, cam);
    let (pull, fill, planted, thrusting, _) = plunger_pose(e);

    // Compact stilt feet (wider when planted).
    let s = if planted {
        3.6
    } else if step(e) {
        2.6
    } else {
        1.7
    };
    for side in [1.0_f32, -1.0] {
        let foot = l.at(0.0, side * s);
        let hip = l.at(0.0, side * 1.0);
        line_blend(pb, hip, foot, l.tint(STEEL_DARK), 0.90);
        pb.plot(foot.0, foot.1, l.tint(RUBBER_HI));
    }

    // Barrel + T-flange + plunger + needle.
    let body = if thrusting {
        (255, 240, 224)
    } else if fill > 0.25 {
        SPECIMEN_PINK
    } else {
        e.kind.color()
    };
    let back = l.at(-2.4, 0.0);
    let front = l.at(1.6, 0.0);
    line_blend(pb, back, front, l.tint(body), 1.0);
    for sv in [-2.2_f32, 2.2] {
        let fl = l.at(-2.4, sv);
        pb.plot(fl.0, fl.1, l.tint(STEEL_HI));
    }
    let thumb = l.at(-3.5 - pull * 0.6, 0.0);
    pb.plot(thumb.0, thumb.1, l.tint(CERAMIC_HI));
    let n0 = l.at(2.4, 0.0);
    let n1 = l.at(floor_lab::SYRINGE_TIP - 1.5, 0.0);
    line_blend(pb, n0, n1, l.tint((248, 252, 255)), 1.0);
}

/// The Walking Syringe's ray: dotted while it is a warning, solid while it
/// bites. Drawn from `floor_lab::syringe_ray`, the segment the hit test reads,
/// so what is shown is exactly what hurts. Belongs in the hostile layer
/// (self-lit, after the lighting pass).
pub(crate) fn draw_syringe_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let Some(ray) = floor_lab::syringe_ray(e) else {
        return;
    };
    let a = ray.from - cam;
    let d = ray.to - ray.from;
    let len = d.len();
    if len < 1.0 {
        return;
    }
    let dir = d * (1.0 / len);
    let side = Vec2::new(-dir.y, dir.x);
    let steps = len.round() as i32;
    let half = (floor_lab::SYRINGE_RAY_WIDTH * 0.5).floor() as i32;
    let warn = mix((255, 168, 68), (255, 86, 128), ray.progress);
    let live_core: Rgb = (255, 250, 228);
    let live_edge: Rgb = (255, 76, 130);
    for k in 0..=steps {
        let c = a + dir * k as f32;
        if ray.live {
            // The thrust fills the whole 26×3 lane with a white-hot core and
            // vivid specimen-pink edges.
            for w in -half..=half {
                let x = (c.x + side.x * w as f32).round() as i32;
                let y = (c.y + side.y * w as f32).round() as i32;
                pb.plot(x, y, if w == 0 { live_core } else { live_edge });
            }
        } else if (k / 2) % 2 == 0 {
            // Dotted warning: 2 px on, 2 px off, brightening and heating toward
            // specimen-crimson as the plunger finishes its draw; the lane's two
            // edges are marked at the far end.
            let x = c.x.round() as i32;
            let y = c.y.round() as i32;
            blend(pb, x, y, warn, 0.58 + 0.42 * ray.progress);
            if k >= steps - 1 {
                for w in [-half, half] {
                    let x = (c.x + side.x * w as f32).round() as i32;
                    let y = (c.y + side.y * w as f32).round() as i32;
                    blend(pb, x, y, warn, 0.85);
                }
            }
        }
    }
}

// ---- Autoclave Cart ---------------------------------------------------------

/// Box door on two big wheels: a Brute-scale brushed stainless-steel sterilizer
/// chassis with a recessed pressure hatch, glowing console-cyan porthole,
/// pulsing pressurized indicator LED, ceramic rear push-bar, two chunky proud
/// rubber cart wheels whose treads and hubs turn with the walk, and a heavy
/// brass latch that oscillates smoothly across the front door face.
///
/// The elite wears a thick double gold shoulder stripe and an open crown of
/// six hooked gold safety latches standing proud of both flanks
/// (`WAVE4_ROSTERS.md`, the elite rule).
pub(crate) fn draw_autoclavecart(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    elite: bool,
) {
    let l = Local::of(e, cam);
    let phase = (e.anim * 0.5).sin();
    let cphase = (e.anim * 0.5).cos();
    let lunge = if e.contact_cd > 0.25 { 1.0 } else { 0.0 };
    let damaged = e.max_hp > 0.0 && e.hp < e.max_hp * 0.62;

    let body = e.kind.color();
    let door_mid = mix(body, STEEL_DARK, 0.42);
    let door_dark = (30, 36, 48);
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    let rim_col = if l.hit > 0.4 { (148, 150, 162) } else { RIM_DARK };

    // Grounded heavy cart shadow slab.
    shadow_oval(pb, cx + 1, cy + 4, 6, 4, 0.34);

    // Rear white ceramic push-handle bar (gives instant cart silhouette and
    // clear front/back orientation).
    let bar_a = l.at(-6.1 + lunge, -3.4);
    let bar_b = l.at(-6.1 + lunge, 3.4);
    for side in [-3.2_f32, 3.2] {
        let p0 = l.at(-5.0 + lunge, side);
        let p1 = l.at(-6.1 + lunge, side);
        line_blend(pb, p0, p1, l.tint(STEEL_DARK), 0.95);
    }
    line_lit(pb, bar_a, bar_b, l.tint(CERAMIC_DARK), l.tint(CERAMIC_HI));

    // Two big proud black-rubber cart wheels flanking the chassis, with
    // rotating tread notches and a bright steel hubcap so they read boldly
    // against dark Lab tiles.
    let spoke_a = phase >= 0.0;
    let tread_shift = phase * 1.4;
    for side in [1.0_f32, -1.0] {
        let w_u = -0.4 + lunge;
        let w_v = side * 5.6;
        let (wx, wy) = l.at(w_u, w_v);
        shadow_oval(pb, wx + 1, wy + 2, 2, 2, 0.34);
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let rx = (wx + dx) as f32 - (l.pos.x + l.f.x * w_u + l.p.x * w_v);
                let ry = (wy + dy) as f32 - (l.pos.y + l.f.y * w_u + l.p.y * w_v);
                let du = rx * l.f.x + ry * l.f.y;
                let dv = rx * l.p.x + ry * l.p.y;
                let q = (du / 3.0) * (du / 3.0) + (dv / 1.9) * (dv / 1.9);
                if q > 1.55 {
                    continue;
                }
                if q > 1.0 {
                    blend(pb, wx + dx, wy + dy, rim_col, 0.58);
                    continue;
                }
                let tread = ((du + tread_shift) * 1.65).sin() > 0.35;
                let c = if q < 0.24 {
                    STEEL_HI
                } else if tread && dv.abs() > 0.45 {
                    RUBBER_HI
                } else if l.lit_dot(du * 0.3, dv * 0.5) > 0.15 {
                    RUBBER_MID
                } else {
                    RUBBER_DARK
                };
                pb.plot(wx + dx, wy + dy, l.tint(c));
            }
        }
        // Hubcap + turning spoke cross.
        pb.plot(wx, wy, l.tint(STEEL_HI));
        let (su, sv) = if spoke_a { (1.2, 0.0) } else { (0.0, 1.0) };
        let s1 = l.at(w_u + su, w_v + sv);
        let s2 = l.at(w_u - su, w_v - sv);
        pb.plot(s1.0, s1.1, l.tint(RUBBER_KNEE));
        pb.plot(s2.0, s2.1, l.tint(RUBBER_KNEE));
    }

    // Heavy chamfered stainless-steel autoclave chassis (`10.4 × 9.2` px).
    let reach = 9i32;
    let porthole_flare = lunge > 0.0;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let rx = (cx + dx) as f32 - l.pos.x;
            let ry = (cy + dy) as f32 - l.pos.y;
            let u = rx * l.f.x + ry * l.f.y - lunge;
            let v = rx * l.p.x + ry * l.p.y;
            let au = u.abs();
            let av = v.abs();
            // Chamfered box SDF-like test: clip the 4 outer corners cleanly.
            let corner = (au - 4.3).max(0.0) + (av - 3.7).max(0.0);
            let outside = au > 5.2 || av > 4.6 || corner > 1.3;
            if outside {
                if au <= 6.0 && av <= 5.4 && corner <= 2.1 {
                    blend(pb, cx + dx, cy + dy, rim_col, 0.66);
                }
                continue;
            }
            let on_bevel = au > 4.1 || av > 3.55 || corner > 0.45;
            let c = if on_bevel {
                let eu = if au > 4.1 { u.signum() } else { 0.0 };
                let ev = if av > 3.55 { v.signum() } else { 0.0 };
                let dl = l.lit_dot(eu, ev);
                if dl > 0.10 {
                    STEEL_HI
                } else if dl > -0.25 {
                    body
                } else {
                    STEEL_DARK
                }
            } else if au < 2.95 && av < 2.85 {
                // Recessed autoclave pressure door with dark gasket groove and
                // round glowing cyan-console porthole.
                let pu = u - 0.15;
                let pr2 = pu * pu + v * v;
                if au > 2.35 || av > 2.25 {
                    door_dark
                } else if pr2 < 2.25 {
                    let dl = l.lit_dot(pu * 0.65, v * 0.65);
                    if pr2 > 1.45 {
                        if dl > 0.1 { STEEL_HI } else { door_dark }
                    } else if dl > 0.42 {
                        (248, 255, 255)
                    } else if porthole_flare {
                        CONSOLE_HI
                    } else {
                        shade3(CONSOLE_DARK, CONSOLE_MID, CONSOLE_HI, 0.55 + 0.52 * dl)
                    }
                } else {
                    let dl = l.lit_dot(u * 0.25, v * 0.25);
                    shade3(door_dark, door_mid, STEEL_MID, 0.50 + 0.35 * dl)
                }
            } else {
                // Brushed steel deck around the recessed door.
                let dl = l.lit_dot(u * 0.18, v * 0.18);
                shade3(STEEL_DARK, body, STEEL_HI, 0.52 + 0.34 * dl)
            };
            pb.plot(cx + dx, cy + dy, l.tint(c));
        }
    }

    // Four machined corner bolt rivets on the steel casing.
    for su in [-3.7_f32, 3.7] {
        for sv in [-3.2_f32, 3.2] {
            let rivet = l.at(su + lunge, sv);
            pb.plot(rivet.0, rivet.1, l.tint(STEEL_DARK));
        }
    }

    // Pulsing specimen-pink pressurized status LED on the rear shoulder
    // (glints clearly even in dark pooled ambient).
    let led_p = l.at(-3.5 + lunge, 2.2);
    let led_c = if cphase > 0.0 { PINK_HI } else { SPECIMEN_PINK };
    pb.plot(led_p.0, led_p.1, l.tint(led_c));

    // Damage seam + venting steam glint when below 62% HP.
    if damaged {
        let c0 = l.at(-1.8 + lunge, -2.6);
        let c1 = l.at(-3.4 + lunge, -1.2);
        line_blend(pb, c0, c1, l.tint(RIM_DARK), 0.90);
        let hiss = l.at(-2.8 + lunge, -2.2);
        blend(pb, hiss.0, hiss.1, CONSOLE_HI, 0.78);
    }

    // Recessed front latch rail + oscillating brass locking lever.
    for vi in -3i32..=3 {
        let track = l.at(4.6 + lunge, vi as f32);
        pb.plot(track.0, track.1, l.tint(door_dark));
    }
    let slide = phase * 2.7;
    let latch_root = l.at(3.6 + lunge, slide);
    let latch_mid = l.at(4.9 + lunge, slide);
    let latch_tip = l.at(6.1 + lunge, slide);
    line_lit(
        pb,
        latch_root,
        latch_tip,
        l.tint(BRASS_DARK),
        l.tint(if lunge > 0.0 { CERAMIC_HI } else { BRASS_MID }),
    );
    pb.plot(latch_mid.0, latch_mid.1, l.tint(BRASS_MID));
    pb.plot(
        latch_tip.0,
        latch_tip.1,
        l.tint(if lunge > 0.0 { (255, 255, 255) } else { (240, 232, 184) }),
    );

    if elite {
        // Thick double royal-gold shoulder stripe across the top of the box
        // from flank to flank, separated by a crisp dark bronze channel.
        for vi in -4i32..=4 {
            let groove = l.at(-2.5 + lunge, vi as f32);
            pb.plot(groove.0, groove.1, l.tint(ELITE_GOLD_DARK));
            for (u, c) in [
                (-1.0_f32, ELITE_GOLD_HI),
                (-1.8, ELITE_GOLD),
                (-3.3, ELITE_GOLD_HI),
                (-4.1, ELITE_GOLD),
            ] {
                let (x, y) = l.at(u + lunge, vi as f32);
                pb.plot(x, y, l.tint(c));
            }
        }
        // Open crown of safety latches: six hooked gold prongs standing proud
        // of the flanks (three per side), open across the front door and rear
        // push-bar so the crown reads unmistakably from either side.
        for side in [1.0_f32, -1.0] {
            for u in [-3.5_f32, 0.2, 3.6] {
                let root = l.at(u + lunge, side * 4.9);
                let shaft = l.at(u + lunge, side * 6.5);
                let tip = l.at(u + lunge, side * 7.6);
                let hook = l.at(u + lunge + 0.9, side * 7.5);
                line_lit(pb, root, tip, l.tint(ELITE_GOLD_DARK), l.tint(ELITE_GOLD));
                pb.plot(shaft.0, shaft.1, l.tint(ELITE_GOLD));
                pb.plot(tip.0, tip.1, l.tint(ELITE_GOLD_HI));
                pb.plot(hook.0, hook.1, l.tint(ELITE_GOLD_HI));
            }
        }
    }
}

pub(crate) fn draw_autoclavecart_lod(
    pb: &mut PixelBuffer,
    _fr: &Frame,
    e: &Enemy,
    cam: Vec2,
    elite: bool,
) {
    let l = Local::of(e, cam);
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    let body = e.kind.color();

    // Compact proud rubber wheels with bright steel hub dot.
    for side in [1.0_f32, -1.0] {
        let (wx, wy) = l.at(-0.3, side * 4.1);
        pb.plot(wx - 1, wy, l.tint(RUBBER_DARK));
        pb.plot(wx + 1, wy, l.tint(RUBBER_DARK));
        pb.plot(wx, wy, l.tint(RUBBER_HI));
    }

    // Compact 7×7 steel box with lit chrome edge, dark steel edge, and glowing
    // cyan porthole.
    for dy in -3i32..=3 {
        for dx in -3i32..=3 {
            let edge = dx.abs() == 3 || dy.abs() == 3;
            let c = if edge {
                if dx == -3 || dy == -3 {
                    STEEL_HI
                } else {
                    STEEL_DARK
                }
            } else if dx.abs() <= 1 && dy.abs() <= 1 {
                mix(body, STEEL_DARK, 0.42)
            } else {
                body
            };
            pb.plot(cx + dx, cy + dy, l.tint(c));
        }
    }
    pb.plot(cx, cy, l.tint(CONSOLE_MID));
    let latch = l.at(3.5, if step(e) { 1.4 } else { -1.4 });
    pb.plot(latch.0, latch.1, l.tint(BRASS_MID));

    if elite {
        // Far LOD: bold double gold shoulder stripes across the block and two
        // gleaming crown safety-latch prongs standing proud of each flank.
        for vi in -3i32..=3 {
            for (u, c) in [(-0.8_f32, ELITE_GOLD_HI), (-2.2, ELITE_GOLD)] {
                let (x, y) = l.at(u, vi as f32);
                pb.plot(x, y, l.tint(c));
            }
        }
        for side in [1.0_f32, -1.0] {
            for u in [-1.5_f32, 1.8] {
                let root = l.at(u, side * 4.3);
                let tip = l.at(u, side * 5.2);
                pb.plot(root.0, root.1, l.tint(ELITE_GOLD));
                pb.plot(tip.0, tip.1, l.tint(ELITE_GOLD_HI));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    fn body(kind: EnemyKind) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(kind, Vec2::new(32.0, 32.0), 1.0, &mut rng);
        e.facing = Vec2::new(1.0, 0.0);
        e
    }

    fn painted(kind: EnemyKind, anim: f32, full: bool) -> Vec<Rgb> {
        let mut pb = PixelBuffer::new(64, 64);
        let floor: Rgb = (9, 9, 9);
        pb.clear(floor);
        let mut e = body(kind);
        e.anim = anim;
        // Through the public entry points, so the table rows are exercised.
        if full {
            crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
        } else {
            crate::game::art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
        }
        pb.pixels().to_vec()
    }

    const LAB: [EnemyKind; 5] = [
        EnemyKind::SwabFoot,
        EnemyKind::SpinnerLid,
        EnemyKind::GlassRunner,
        EnemyKind::WalkingSyringe,
        EnemyKind::AutoclaveCart,
    ];

    #[test]
    fn every_lab_body_paints_a_light_and_a_dark_edge() {
        for kind in LAB {
            let px = painted(kind, 0.4, true);
            let body: Vec<&Rgb> = px.iter().filter(|c| **c != (9, 9, 9)).collect();
            assert!(body.len() >= 6, "{kind:?} paints a readable core ({})", body.len());
            let lum = |c: &Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
            assert!(body.iter().any(|c| lum(c) > 600), "{kind:?} has a light edge");
            assert!(body.iter().any(|c| lum(c) < 260), "{kind:?} has a dark edge");
        }
    }

    #[test]
    fn every_lab_body_has_a_far_lod() {
        for kind in LAB {
            let px = painted(kind, 0.4, false);
            let n = px.iter().filter(|c| **c != (9, 9, 9)).count();
            assert!(n >= 3, "{kind:?} far LOD is visible ({n})");
            let full = painted(kind, 0.4, true)
                .iter()
                .filter(|c| **c != (9, 9, 9))
                .count();
            assert!(n <= full, "{kind:?} far LOD is no bigger than the full body");
        }
    }

    #[test]
    fn walkers_have_two_distinct_frames() {
        // Gait uses sin/cos of anim * 0.5, which flips sign between 1.0 and 8.0.
        for kind in [
            EnemyKind::SwabFoot,
            EnemyKind::GlassRunner,
            EnemyKind::WalkingSyringe,
            EnemyKind::AutoclaveCart,
        ] {
            assert_ne!(painted(kind, 1.0, true), painted(kind, 8.0, true), "{kind:?}");
        }
    }

    #[test]
    fn spinner_lid_alternates_broad_face_and_thin_edge() {
        let render = |t: f32| {
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear((9, 9, 9));
            let mut e = body(EnemyKind::SpinnerLid);
            e.anim = 0.4;
            e.w4.t = t;
            crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
            pb.pixels().to_vec()
        };
        let edge = render(0.0);
        let broad = render(floor_lab::LID_FLIP_SECS * 0.5);
        assert_ne!(edge, broad);
        let edge_n = edge.iter().filter(|c| **c != (9, 9, 9)).count();
        let broad_n = broad.iter().filter(|c| **c != (9, 9, 9)).count();
        assert!(
            broad_n > edge_n,
            "broad face paints a larger silhouette than thin edge ({broad_n} vs {edge_n})"
        );
    }

    #[test]
    fn syringe_body_changes_pose_across_tell_thrust_and_retract() {
        let render = |phase: u8| {
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear((9, 9, 9));
            let mut e = body(EnemyKind::WalkingSyringe);
            e.anim = 0.5;
            floor_lab::pose_for_test(&mut e, phase, Vec2::new(1.0, 0.0));
            crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
            pb.pixels().to_vec()
        };
        let walk = render(floor_lab::SYRINGE_WALK);
        let tell = render(floor_lab::SYRINGE_TELL);
        let thrust = render(floor_lab::SYRINGE_THRUST);
        let retract = render(floor_lab::SYRINGE_RETRACT);
        assert_ne!(walk, tell, "tell winds up visibly");
        assert_ne!(tell, thrust, "thrust slams forward");
        assert_ne!(thrust, retract, "retract relaxes to punish pose");
        // Tell fills the barrel with glowing specimen pink.
        let pink = |c: &Rgb| c.0 > 200 && c.2 > 120 && c.0 > c.1 + 40;
        let tell_pink = tell.iter().filter(|c| pink(c)).count();
        let walk_pink = walk.iter().filter(|c| pink(c)).count();
        assert!(
            tell_pink > walk_pink,
            "tell fills the syringe barrel with specimen pink ({walk_pink} -> {tell_pink})"
        );
    }

    #[test]
    fn melee_bodies_show_a_contact_lunge_pose() {
        for kind in [
            EnemyKind::SwabFoot,
            EnemyKind::GlassRunner,
            EnemyKind::AutoclaveCart,
        ] {
            let render = |cd: f32| {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear((9, 9, 9));
                let mut e = body(kind);
                e.anim = 0.6;
                e.contact_cd = cd;
                crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
                pb.pixels().to_vec()
            };
            assert_ne!(render(0.0), render(0.45), "{kind:?} lunges on contact");
        }
    }

    #[test]
    fn the_elite_cart_wears_its_crown_and_stripe_at_both_lods() {
        let gold = |c: &Rgb| c.0 > 240 && c.1 > 190 && c.2 < 170;
        for full in [true, false] {
            let count = |elite: bool| {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear((9, 9, 9));
                let mut e = body(EnemyKind::AutoclaveCart);
                e.elite = elite;
                if full {
                    crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
                } else {
                    crate::game::art_enemy::draw_enemy_lod(
                        &mut pb,
                        &e,
                        Vec2::ZERO,
                        (200, 200, 200),
                    );
                }
                pb.pixels().iter().filter(|c| gold(c)).count()
            };
            let (plain, elite) = (count(false), count(true));
            assert!(
                elite >= plain + 6,
                "full={full}: the elite is marked in gold ({plain} -> {elite})"
            );
        }
    }

    #[test]
    fn the_five_silhouettes_differ() {
        for (i, a) in LAB.iter().enumerate() {
            for b in LAB.iter().skip(i + 1) {
                assert_ne!(
                    painted(*a, 0.4, true),
                    painted(*b, 0.4, true),
                    "{a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn the_syringe_ray_is_dotted_then_solid() {
        let count = |phase: u8| {
            let mut pb = PixelBuffer::new(96, 64);
            pb.clear((9, 9, 9));
            let mut e = body(EnemyKind::WalkingSyringe);
            floor_lab::pose_for_test(&mut e, phase, Vec2::new(1.0, 0.0));
            draw_syringe_tell(&mut pb, &e, Vec2::ZERO);
            pb.pixels().iter().filter(|c| **c != (9, 9, 9)).count()
        };
        assert_eq!(count(floor_lab::SYRINGE_WALK), 0, "no ray while walking");
        let dotted = count(floor_lab::SYRINGE_TELL);
        let solid = count(floor_lab::SYRINGE_THRUST);
        assert!(dotted > 8, "the warning is visible ({dotted})");
        assert!(
            solid > dotted * 2,
            "the thrust fills the lane ({solid} vs {dotted})"
        );
        assert_eq!(count(floor_lab::SYRINGE_RETRACT), 0, "no ray while retracting");
    }

    #[test]
    fn lab_bodies_draw_safely_on_tiny_and_edge_buffers() {
        for (bw, bh) in [(0usize, 0usize), (1, 1), (4, 4), (48, 48)] {
            let mut pb = PixelBuffer::new(bw, bh);
            for kind in LAB {
                for &(x, y) in &[(-12.0, -12.0), (0.0, 0.0), (24.0, 24.0), (47.5, 47.5)] {
                    let mut e = body(kind);
                    e.pos = Vec2::new(x, y);
                    e.facing = Vec2::ZERO;
                    e.flash = 0.12;
                    crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                    crate::game::art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                    e.elite = true;
                    crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                    crate::game::art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (90, 220, 200));
                }
            }
        }
    }
}
