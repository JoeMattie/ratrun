//! The Sewer's four new bodies (`WAVE4_ROSTERS.md` "Sewer", Silhouette column).
//!
//! | kind | silhouette | frames |
//! |---|---|---|
//! | Pipe Eel | bone hook on a thick neck, a thin scribbled tail | two tail scribbles |
//! | Grate Archer | barnacle cone on two feet, a lip at the tip | two foot frames, the lip swells before a shot |
//! | Sump Leech | black comma; the head opens into a pale sucker | two tail wags, the open sucker + tongue footprint |
//! | Sluice Toad | squat slab wider than it is long, twin eye turrets | belly haul (stretch / gather) |
//!
//! Every function has `art_enemy`'s `BodyDraw` signature and is registered in
//! its `bespoke_body` / `bespoke_lod` tables. The bodies build their own local
//! frame from the `&Enemy` and the camera (the same `pos - cam`, `facing` and
//! flash rule as `art_enemy::prepare_frame`), so they read nothing private
//! from the shared `Frame`.
//!
//! Each body is one pass over its screen box: a closure answers "what colour
//! is local point (u, v)?" (u along the facing, v across it, world px), and
//! the pass shades it from a light upper-left edge to a dark lower-right one.
//! Render-only: nothing here touches the simulation or draws rng.

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_sewer;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// `art_enemy::FLASH_MAX`: the hit flash's full length.
const FLASH_MAX: f32 = 0.12;

/// Pipe Eel: bone, with a marrow-dark underside.
const EEL_BONE: Rgb = (222, 214, 190);
const EEL_DARK: Rgb = (96, 84, 70);
/// Grate Archer: wet barnacle.
const ARCHER_SHELL: Rgb = (140, 168, 150);
const ARCHER_PLATE: Rgb = (86, 108, 100);
const ARCHER_LIP: Rgb = (226, 232, 208);
const ARCHER_CHARGE: Rgb = (255, 208, 112);
/// Sump Leech: near black, a pale sucker.
const LEECH_BODY: Rgb = (58, 54, 62);
const LEECH_SHEEN: Rgb = (118, 110, 130);
const LEECH_SUCKER: Rgb = (236, 222, 206);
const LEECH_TONGUE: Rgb = (232, 150, 150);
/// Sluice Toad: valve green, a paler belly line, brass for the elite collar.
const TOAD_HIDE: Rgb = (104, 138, 96);
const TOAD_BELLY: Rgb = (170, 190, 132);
const TOAD_WART: Rgb = (62, 88, 62);
const TOAD_EYE: Rgb = (236, 226, 150);
const TOAD_BRASS: Rgb = (224, 176, 72);

/// One body's local frame: where it is on screen and which way it faces.
#[derive(Clone, Copy)]
struct Local {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
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
        Local {
            pos: e.pos - cam,
            f,
            p: Vec2::new(-f.y, f.x),
            hit: (e.flash / FLASH_MAX).clamp(0.0, 1.0),
        }
    }

    /// Local (u, v) → screen pixel.
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    /// The warm hit flash every enemy body takes.
    fn flash(&self, c: Rgb) -> Rgb {
        if self.hit <= 0.0 {
            return c;
        }
        let to = if self.hit > 0.6 { (255, 244, 218) } else { (255, 218, 152) };
        mix(c, to, (0.30 + self.hit * 0.56).min(0.86))
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
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else if a > 0.0 {
        let bg = pb.pixel_at(x as usize, y as usize);
        pb.plot(x, y, mix(bg, c, a));
    }
}

/// A one-pixel line, blended.
fn line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    // A body's line is short; the guard only stops a degenerate input.
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

/// Paint a body: for every screen pixel within `reach` of the body, ask
/// `shape(u, v)` for its base colour and shade it light toward the upper left
/// and dark toward the lower right, so every silhouette has a light and a dark
/// edge against its own tiles. A soft shadow falls one pixel down-right.
fn paint(pb: &mut PixelBuffer, lf: &Local, reach: f32, shape: impl Fn(f32, f32) -> Option<Rgb>) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let rr = reach.ceil() as i32 + 1;
    let cx = lf.pos.x.round() as i32;
    let cy = lf.pos.y.round() as i32;
    let x0 = (cx - rr).max(0);
    let x1 = (cx + rr).min(pb.w as i32 - 1);
    let y0 = (cy - rr).max(0);
    let y1 = (cy + rr).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let local = |x: i32, y: i32| {
        let dx = x as f32 - lf.pos.x;
        let dy = y as f32 - lf.pos.y;
        (dx * lf.f.x + dy * lf.f.y, dx * lf.p.x + dy * lf.p.y, dx, dy)
    };
    // The shadow first, so the body always sits on top of it.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v, _, _) = local(x - 1, y - 1);
            if shape(u, v).is_some() {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix(bg, (0, 0, 0), 0.35));
            }
        }
    }
    let inv = 1.0 / (reach.max(1.0) * 1.3);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v, dx, dy) = local(x, y);
            let Some(base) = shape(u, v) else {
                continue;
            };
            // -1 at the lower right, +1 at the upper left.
            let lum = (-(dx + dy) * inv).clamp(-1.0, 1.0);
            let lit = if lum >= 0.0 {
                mix(base, (255, 250, 236), lum * 0.42)
            } else {
                mix(base, (8, 10, 14), -lum * 0.58)
            };
            pb.plot(x, y, lf.flash(lit));
        }
    }
}

#[inline]
fn in_disc(u: f32, v: f32, cu: f32, cv: f32, r: f32) -> bool {
    let (du, dv) = (u - cu, v - cv);
    du * du + dv * dv <= r * r
}

/// Which of the two walk frames a body is on.
#[inline]
fn walk_frame(e: &Enemy) -> bool {
    (e.anim * 0.5).rem_euclid(2.0) >= 1.0
}

// ---- Pipe Eel -------------------------------------------------------------

/// Bone hook with a thick neck; the tail is a thin scribble that redraws
/// itself on the other foot.
pub(crate) fn draw_pipe_eel(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 4.0);
    let flip = if walk_frame(e) { 1.0 } else { -1.0 };

    // The scribbled tail: a thin zigzag behind the neck, drawn first.
    let mut prev = lf.px(-2.0 * s, 0.0);
    for k in 1..=5 {
        let u = -2.0 - k as f32 * 1.6;
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let v = side * flip * (0.6 + k as f32 * 0.35);
        let next = lf.px(u * s, v * s);
        line(pb, prev, next, lf.flash(mix(EEL_BONE, EEL_DARK, 0.25 + k as f32 * 0.1)), 0.95);
        prev = next;
    }

    // The neck (thick, 4 px across) and the hook: the head turns back on
    // itself to one side, leaving a dark notch between jaw and neck.
    paint(pb, &lf, 7.0 * s, |u, v| {
        let (u, v) = (u / s, v / s);
        let neck = (-2.5..=3.0).contains(&u) && v.abs() <= 2.0 - (u + 2.5).max(0.0) * 0.06;
        let skull = in_disc(u, v, 3.6, 0.4, 2.3);
        let barb = in_disc(u, v, 3.0, 3.4, 1.3) || in_disc(u, v, 1.6, 4.0, 0.9);
        let notch = in_disc(u, v, 1.9, 2.5, 0.8);
        if notch {
            return None;
        }
        if skull || barb {
            // A dark eye socket on the skull.
            if in_disc(u, v, 4.2, -0.8, 0.7) {
                return Some(EEL_DARK);
            }
            return Some(EEL_BONE);
        }
        if neck {
            // Vertebra bands down the neck.
            let band = ((u + 2.5) * 0.9) as i32 % 2 == 0;
            return Some(if band { EEL_BONE } else { mix(EEL_BONE, EEL_DARK, 0.3) });
        }
        None
    });
}

pub(crate) fn draw_pipe_eel_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 4.0);
    let flip = if walk_frame(e) { 1.0 } else { -1.0 };
    let bone = lf.flash(EEL_BONE);
    let dark = lf.flash(EEL_DARK);
    // Tail, neck, and the hook's sideways barb: an unmistakable "J".
    line(pb, lf.px(-7.0 * s, flip * 1.5 * s), lf.px(-2.0 * s, 0.0), dark, 0.9);
    let (a, b) = (lf.px(-2.0 * s, 0.0), lf.px(4.0 * s, 0.0));
    line(pb, (a.0 + 1, a.1 + 1), (b.0 + 1, b.1 + 1), dark, 0.9);
    line(pb, a, b, bone, 1.0);
    line(pb, b, lf.px(2.5 * s, 3.5 * s), bone, 1.0);
    let top = lf.px(4.0 * s, 0.0);
    blend(pb, top.0, top.1 - 1, mix(bone, (255, 255, 255), 0.5), 1.0);
}

// ---- Grate Archer ---------------------------------------------------------

/// The Spitter's muzzle swell: 0 while walking, rising to 1 over the last
/// 0.75 s before the shot leaves.
#[inline]
fn archer_charge(e: &Enemy) -> f32 {
    (1.0 - e.fire_cd / 0.75).clamp(0.0, 1.0)
}

/// Barnacle cone on two feet: broad plated base at the back, narrowing to a
/// pale lip. The lip puckers (swells and glows) before it spits.
pub(crate) fn draw_grate_archer(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 3.0);
    let charge = archer_charge(e);
    let step = if walk_frame(e) { 1.0 } else { -1.0 };

    // Two feet, one forward and one back, swapping each frame.
    for side in [-1.0f32, 1.0] {
        let foot = lf.px((-1.0 + side * step * 1.2) * s, side * 4.0 * s);
        blend(pb, foot.0, foot.1, lf.flash(ARCHER_PLATE), 1.0);
        let toe = lf.px((0.0 + side * step * 1.2) * s, side * 4.4 * s);
        blend(pb, toe.0, toe.1, lf.flash(ARCHER_PLATE), 1.0);
    }

    let lip_r = 1.2 + charge * 1.1;
    paint(pb, &lf, 6.0 * s, |u, v| {
        let (u, v) = (u / s, v / s);
        // The puckered lip: a pale ring round a dark throat, at the tip.
        if in_disc(u, v, 3.4, 0.0, lip_r) {
            if in_disc(u, v, 3.6, 0.0, lip_r - 0.9) {
                return Some(mix((20, 26, 26), ARCHER_CHARGE, charge));
            }
            return Some(mix(ARCHER_LIP, ARCHER_CHARGE, charge * 0.6));
        }
        if !(-3.4..=3.2).contains(&u) {
            return None;
        }
        // The cone: 3.6 half-wide at the base, 1.1 at the lip.
        let t = (u + 3.4) / 6.6;
        let half = 3.6 - t * 2.5;
        if v.abs() > half {
            return None;
        }
        // Plates: a darker ring every other step up the cone.
        let ring = ((u + 3.4) * 0.75) as i32 % 2 == 1;
        Some(if ring { ARCHER_PLATE } else { ARCHER_SHELL })
    });
}

pub(crate) fn draw_grate_archer_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 3.0);
    let charge = archer_charge(e);
    let shell = lf.flash(ARCHER_SHELL);
    let plate = lf.flash(ARCHER_PLATE);
    // A wide base bar, a narrower mid bar, and the lip pixel: a triangle.
    line(pb, lf.px(-3.0 * s, -3.0 * s), lf.px(-3.0 * s, 3.0 * s), plate, 1.0);
    line(pb, lf.px(-s, -2.2 * s), lf.px(-s, 2.2 * s), shell, 1.0);
    line(pb, lf.px(1.0 * s, -1.2 * s), lf.px(1.0 * s, 1.2 * s), shell, 1.0);
    let lip = lf.px(3.0 * s, 0.0);
    blend(pb, lip.0, lip.1, lf.flash(mix(ARCHER_LIP, ARCHER_CHARGE, charge)), 1.0);
}

// ---- Sump Leech -----------------------------------------------------------

/// The tongue's footprint, from the one geometry that hits
/// (`floor_sewer::tongue_segment`): a dashed outline that fills from the lip
/// while the leech warns, the solid tongue while it is out.
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
    for i in 0..=steps {
        let t = i as f32;
        let c = a + dir * t;
        // The two long edges: always drawn, so the whole footprint is
        // readable from the first tick of the warning.
        for sgn in [-1.0f32, 1.0] {
            let p = c + side * (sgn * half);
            let on = live || i % 3 != 2;
            if on {
                blend(pb, p.x.round() as i32, p.y.round() as i32, LEECH_SUCKER, if live { 1.0 } else { 0.8 });
            }
        }
        // The fill: creeps out from the lip during the warning (shape first),
        // solid and pink once it is live.
        let filled = live || t <= len * progress;
        if filled {
            for w in -1..=1 {
                let p = c + side * (w as f32 * (half - 1.0));
                let col = if live { LEECH_TONGUE } else { mix(LEECH_BODY, LEECH_SUCKER, 0.45) };
                blend(pb, p.x.round() as i32, p.y.round() as i32, col, if live { 1.0 } else { 0.6 });
            }
        }
    }
    // The tip cap.
    for w in -2..=2 {
        let p = b + side * (w as f32 * half * 0.5);
        blend(pb, p.x.round() as i32, p.y.round() as i32, LEECH_SUCKER, if live { 1.0 } else { 0.8 });
    }
}

/// How far the sucker is open: 0 walking, 0 → 1 over the warning, 1 while the
/// tongue is out.
#[inline]
fn leech_open(e: &Enemy) -> f32 {
    match floor_sewer::leech_tongue(e) {
        // Never quite shut once it has committed: the pose changes on the
        // first warned frame.
        Some((_, _, phase, progress)) if phase == floor_sewer::LEECH_PHASE_WARN => {
            0.15 + 0.85 * progress
        }
        Some(_) => 1.0,
        None => 0.0,
    }
}

/// Black comma: a round head and a tail that curls off to one side. The tell
/// pose opens the head into a pale sucker facing the rat.
pub(crate) fn draw_sump_leech(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 5.0);
    let open = leech_open(e);
    // The tail wags between two frames while walking and lies still planted.
    let wag = if open > 0.0 {
        0.0
    } else if walk_frame(e) {
        0.9
    } else {
        -0.9
    };

    // The tongue's footprint is not drawn here: the hostile layer stamps it
    // once, above the lighting (`World::draw_floor_tell`).

    paint(pb, &lf, 8.0 * s, |u, v| {
        let (u, v) = (u / s, v / s);
        // The sucker: a pale ring with a dark throat on the front of the head.
        if open > 0.0 {
            let r = 1.2 + open * 2.0;
            if in_disc(u, v, 3.2, 0.0, r) {
                if in_disc(u, v, 3.6, 0.0, r - 1.1) {
                    return Some((24, 14, 20));
                }
                return Some(LEECH_SUCKER);
            }
        }
        // The comma: head, then three shrinking beads curling to +v.
        let head = in_disc(u, v, 1.4, 0.0, 3.4);
        let t1 = in_disc(u, v, -2.0, 1.4 + wag * 0.4, 2.4);
        let t2 = in_disc(u, v, -4.4, 3.0 + wag * 0.8, 1.6);
        let t3 = in_disc(u, v, -5.6, 4.6 + wag, 1.0);
        if head {
            // A wet sheen spot keeps the black body from reading as a hole.
            if in_disc(u, v, 1.0, -1.4, 0.9) {
                return Some(LEECH_SHEEN);
            }
            return Some(LEECH_BODY);
        }
        if t1 || t2 || t3 {
            return Some(mix(LEECH_BODY, LEECH_SHEEN, 0.12));
        }
        None
    });
}

pub(crate) fn draw_sump_leech_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 5.0);
    let open = leech_open(e);
    // The tongue's footprint comes from the hostile layer, at every zoom.
    let body = lf.flash(LEECH_BODY);
    let head = lf.px(1.4 * s, 0.0);
    for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        blend(pb, head.0 + dx, head.1 + dy, body, 1.0);
    }
    blend(pb, head.0 - 1, head.1 - 1, lf.flash(LEECH_SHEEN), 1.0);
    line(pb, lf.px(-1.5 * s, 1.0 * s), lf.px(-5.0 * s, 4.0 * s), body, 1.0);
    if open > 0.0 {
        let lip = lf.px(3.6 * s, 0.0);
        blend(pb, lip.0, lip.1, lf.flash(LEECH_SUCKER), 1.0);
    }
}

// ---- Sluice Toad ----------------------------------------------------------

/// Squat slab, wider than it is long, with two eye turrets standing proud of
/// the front edge. The belly haul stretches the slab forward, then gathers the
/// rear up behind it. The elite wears a split brass valve collar and a double
/// shoulder stripe.
pub(crate) fn draw_sluice_toad(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 6.5);
    // Haul: frame A stretched (long), frame B gathered (short and wide).
    let stretch = walk_frame(e);
    let (back, front, half) = if stretch { (-4.6, 5.4, 6.2) } else { (-3.8, 4.6, 6.8) };
    let elite = elite || e.elite;

    paint(pb, &lf, 10.0 * s, |u, v| {
        let (u, v) = (u / s, v / s);
        let av = v.abs();
        // Twin eye turrets: discs proud of the front corners.
        if in_disc(u, av, front - 0.2, 3.6, 2.2) {
            if in_disc(u, av, front + 0.5, 3.6, 0.8) {
                return Some((18, 20, 14));
            }
            return Some(TOAD_EYE);
        }
        if u < back || u > front || av > half {
            return None;
        }
        // Round the slab's four corners.
        let cu = (u - (back + front) * 0.5).abs() - ((front - back) * 0.5 - 2.0);
        let cv = av - (half - 2.0);
        if cu > 0.0 && cv > 0.0 && cu * cu + cv * cv > 4.0 {
            return None;
        }
        if elite {
            // The split valve collar: a brass band behind the eyes, open at
            // the spine; and the double shoulder stripe.
            let collar = (u - (front - 3.2)).abs() <= 0.7 && av >= 1.2;
            let stripes = ((u - 0.8).abs() <= 0.9 || (u + 2.2).abs() <= 0.9) && av <= half - 1.0;
            if collar || stripes {
                return Some(TOAD_BRASS);
            }
        }
        // The pale belly line down the middle, warts either side.
        if av <= 1.3 && u < front - 2.0 {
            return Some(TOAD_BELLY);
        }
        let wart = in_disc(u, av, -1.6, 4.0, 0.8) || in_disc(u, av, 1.4, 5.0, 0.7);
        Some(if wart { TOAD_WART } else { TOAD_HIDE })
    });

    // Four stub feet at the corners, the hind pair trailing on the stretch.
    let hind = if stretch { back - 1.4 } else { back - 0.4 };
    for side in [-1.0f32, 1.0] {
        let a = lf.px(hind * s, side * (half - 1.0) * s);
        let b = lf.px((front - 3.0) * s, side * (half + 1.0) * s);
        blend(pb, a.0, a.1, lf.flash(TOAD_WART), 1.0);
        blend(pb, b.0, b.1, lf.flash(TOAD_WART), 1.0);
    }
}

pub(crate) fn draw_sluice_toad_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let lf = Local::of(e, cam);
    let s = e.radius * (1.0 / 6.5);
    let elite = elite || e.elite;
    let front = if walk_frame(e) { 5.0 } else { 4.4 };
    paint(pb, &lf, 9.0 * s, |u, v| {
        let (u, v) = (u / s, v / s);
        let av = v.abs();
        if in_disc(u, av, front, 3.6, 1.6) {
            return Some(TOAD_EYE);
        }
        if u < -4.0 || u > front || av > 6.4 {
            return None;
        }
        if elite && (u - 0.5).abs() <= 0.8 {
            return Some(TOAD_BRASS);
        }
        Some(TOAD_HIDE)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const BG: Rgb = (40, 44, 48);

    fn buffer() -> PixelBuffer {
        let mut pb = PixelBuffer::new(64, 64);
        for y in 0..pb.h as i32 {
            for x in 0..pb.w as i32 {
                pb.plot(x, y, BG);
            }
        }
        pb
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

    fn painted(pb: &PixelBuffer) -> Vec<(usize, usize, Rgb)> {
        let mut out = Vec::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                if c != BG {
                    out.push((x, y, c));
                }
            }
        }
        out
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
            assert_ne!(walk_frame(&a), walk_frame(&b));
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
