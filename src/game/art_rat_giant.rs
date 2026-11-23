//! High-resolution procedural art for the player's rat under the **Growth Serum**.
//!
//! Unlike the old scratch-buffer nearest-neighbour upscale (which blew up the
//! 1× sprite into chunky 5×5 blocks), this module authors the giant rat directly
//! from smooth procedural geometry — sculpted SDF haunches, shoulders, neck,
//! wedge cranium, pointed snout, cupped ears with pink inner bowls, four
//! articulated paws with splayed toes in the diagonal trot cycle, glossy dark
//! eyes with a 1-px glint, crisp 1-px whiskers, and a long tapering segmented
//! pink tail that sways with movement and lashes when `tail_flash > 0`.
//!
//! Everything is authored in the rat's own local `(u, v)` frame scaled by
//! `scale` (`1.0..=5.0` across the Growth Serum's grow-in, hold, warning blink,
//! and shrink-out), while screen-space linework (the 1-px darker outline, 1-px
//! eye glint, and 1-px whiskers) stays razor-sharp at every scale and at any
//! 360-degree facing.
//!
//! Pure drawing: reads only `Player`, `elapsed`, `cam`, `scale`, and
//! `tail_flash`; holds no state, allocates nothing on the heap, and draws no
//! rng. Cast drop shadows temporarily clear the lighting material pen (`0`) so
//! the lighting pass never rim-lights floor shadow pixels as creature fur.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale as scale_rgb, Rgb};

use super::art_rat::{tail_charge, TailCharge};
use super::ecs::systems::powerups::{GIANT_STOMP_INTERVAL, GROWTH_SECS};
use super::player::Player;

// --- Palette (same white/lavender-shadow & rose-pink family as `art_rat`) ---
const OUTLINE: Rgb = (22, 18, 30);
const FUR_DEEP: Rgb = (84, 76, 112);
const FUR_SHADOW: Rgb = (118, 110, 144);
const FUR_DORSAL: Rgb = (142, 134, 168);
const FUR_BASE: Rgb = (212, 206, 218);
const FUR_LIGHT: Rgb = (254, 252, 248);
const FUR_BELLY: Rgb = (246, 232, 226);

const EAR_OUTER: Rgb = (196, 186, 204);
const EAR_RIM: Rgb = (194, 102, 132);
const EAR_PINK: Rgb = (255, 172, 190);
const EAR_DEEP: Rgb = (214, 108, 140);

const NOSE_PINK: Rgb = (255, 114, 146);
const NOSE_DARK: Rgb = (176, 60, 94);
const NOSE_SPEC: Rgb = (255, 224, 236);

const PAW_PINK: Rgb = (248, 198, 198);
const PAW_SHADOW: Rgb = (186, 130, 144);
const PAW_LIGHT: Rgb = (255, 228, 226);
const CLAW_TIP: Rgb = (255, 246, 238);

const TAIL_SHADOW: Rgb = (150, 72, 100);
const TAIL_BASE: Rgb = (214, 122, 146);
const TAIL_TIP: Rgb = (255, 188, 198);
const TAIL_RING: Rgb = (144, 68, 96);

const EYE: Rgb = (18, 12, 22);
const EYE_GLINT: Rgb = (255, 255, 255);
const WHISKER: Rgb = (232, 226, 242);

const HURT: Rgb = (255, 56, 64);
const GHOST: Rgb = (130, 200, 255);

// Tail Thrash charge & lash glow (matching `art_rat`).
const THRASH_GLOW: Rgb = (255, 178, 92);
const THRASH_HOT: Rgb = (255, 246, 214);
const THRASH_HALO: Rgb = (255, 138, 56);

// Super Dash charge ring (matching `art_rat`).
const CHARGE_PIPS: i32 = 24;
const CHARGE_COLOR: Rgb = (198, 230, 255);
const CHARGE_FULL: Rgb = (255, 255, 255);
const CHARGE_SOCKET: Rgb = (46, 58, 84);
const CHARGE_COIL_SQUASH: f32 = 0.16;
const CHARGE_COIL_SPREAD: f32 = 0.12;

/// Direction *toward* the light in screen space (up-left), matching `art_rat`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Resting tail length in normalised rat units (matches `art_rat::TAIL_LEN` ~13).
const TAIL_BASE_UNITS: f32 = 12.6;
/// Stack-allocated tail spline and capsule counts (zero heap allocation).
const TAIL_MAX_PTS: usize = 96;
const TAIL_MAX_CAPS: usize = 24;

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Polynomial smooth minimum for organic anatomical blends.
#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

/// Approximate Euclidean signed distance to an axis-aligned ellipse.
#[inline]
fn sd_ellipse(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let dx = (x - cx) / rx;
    let dy = (y - cy) / ry;
    (dx.hypot(dy) - 1.0) * rx.min(ry)
}

/// Signed distance to a linearly tapered capsule from `a` (`ra`) to `b` (`rb`).
#[inline]
fn sd_taper_capsule(p: Vec2, a: Vec2, b: Vec2, ra: f32, rb: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let ba2 = ba.len_sq().max(1e-6);
    let t = (pa.dot(ba) / ba2).clamp(0.0, 1.0);
    let d = pa - ba * t;
    let r = ra + (rb - ra) * t;
    d.len() - r
}

/// Deterministic integer hash in `[0, 1]` for body-locked fur grain.
#[inline]
fn hash2(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut n = (ix as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((iy as u32).wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(1_442_695_041));
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    n ^= n >> 16;
    (n & 0xFFFF) as f32 / 65535.0
}

#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, c, a));
}

#[inline]
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale_rgb(bg, 1.0 - amount.clamp(0.0, 1.0)));
}

#[derive(Clone, Copy)]
struct Tint {
    hurt: f32,
    ghost: bool,
}

impl Tint {
    fn from_player(player: &Player, elapsed: f32) -> Self {
        Self {
            hurt: if player.hurt_flash > 0.0 {
                (0.55 + player.hurt_flash * 3.0).min(0.85)
            } else {
                0.0
            },
            ghost: player.iframes > 0.0 && ((elapsed * 20.0) as i32 % 2 == 0),
        }
    }

    #[inline]
    fn apply(self, c: Rgb) -> Rgb {
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

    #[inline]
    fn alpha(self) -> f32 {
        if self.ghost { 0.5 } else { 1.0 }
    }
}

#[derive(Clone, Copy)]
struct GiantPose {
    /// Player centre in screen pixels (`player.pos - cam`).
    center: Vec2,
    face: Vec2,
    perp: Vec2,
    scale: f32,
    /// Stretch along / squash across the facing (includes `scale`, dash, coil, stomp).
    su: f32,
    sv: f32,
    /// Detail ramp: `0.0` near scale 1, `1.0` by scale 3+.
    detail: f32,
    /// Gait phase (same timing as `art_rat`: `pos.dot(face) * 1.15`).
    phase: f32,
    wig: f32,
    breathe: f32,
    moving: f32,
    dash: f32,
    hip_shift: f32,
    sh_shift: f32,
    light_u: f32,
    light_v: f32,
}

impl GiantPose {
    fn from_player(player: &Player, elapsed: f32, cam: Vec2, scale: f32) -> Self {
        let scale = scale.max(1.0);
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
        let coil = if player.dash_charging {
            player.dash_charge.clamp(0.0, 1.0) * (1.0 - dash)
        } else {
            0.0
        };

        // Subtle stomp squash aligned to `powerups::GIANT_STOMP_INTERVAL` (0.25 s),
        // derived directly from the exposed `player.buffs.giant` countdown so it
        // stays locked to the stomp beat with zero render-side state.
        let squash = if player.buffs.is_giant() && scale > 1.5 {
            let buff_elapsed = (GROWTH_SECS - player.buffs.giant).max(0.0);
            let stomp_t = (buff_elapsed % GIANT_STOMP_INTERVAL) / GIANT_STOMP_INTERVAL;
            let ramp = ((scale - 1.5) / 3.5).clamp(0.0, 1.0);
            if stomp_t < 0.35 {
                ((stomp_t / 0.35) * PI).sin() * 0.055 * ramp
            } else {
                0.0
            }
        } else {
            0.0
        };

        let phase = player.pos.dot(face) * 1.15;
        let wig = 0.42 * moving * (1.0 - dash);
        let breathe = 0.16 * (1.0 - moving) * (elapsed * 3.2).sin();
        let hip_shift = 0.22 * moving * phase.sin() * (1.0 - dash);
        let sh_shift = -0.18 * moving * phase.sin() * (1.0 - dash);

        Self {
            center: player.pos - cam,
            face,
            perp,
            scale,
            su: (1.0 + 0.30 * dash - CHARGE_COIL_SQUASH * coil - squash * 0.7) * scale,
            sv: (1.0 - 0.13 * dash + CHARGE_COIL_SPREAD * coil + squash * 1.15) * scale,
            detail: ((scale - 1.15) / 1.85).clamp(0.0, 1.0),
            phase,
            wig,
            breathe,
            moving,
            dash,
            hip_shift,
            sh_shift,
            light_u: face.dot(LIGHT),
            light_v: perp.dot(LIGHT),
        }
    }

    /// Lateral spine wave in normalised rat units (fades out at the head).
    #[inline]
    fn spine(&self, u: f32) -> f32 {
        let steady = ((3.0 - u) * 0.24).clamp(0.0, 1.0);
        self.wig * steady * (self.phase - u * 0.7).sin()
    }

    /// Local `(u, v)` (normalised rat units, `v` measured from the spine) → screen position.
    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        let sp = self.spine(u);
        self.center + self.face * (u * self.su) + self.perp * ((v + sp) * self.sv)
    }

    /// Screen offset from `center` → local `(u, v)` in normalised rat units.
    #[inline]
    fn to_local(self, d: Vec2) -> (f32, f32) {
        (d.dot(self.face) / self.su, d.dot(self.perp) / self.sv)
    }

    /// Multi-part smooth-blended body SDF in normalised rat units:
    /// plump pear haunches → athletic mid-torso → shoulders → neck pinch →
    /// wedge cranium → pointed snout.
    #[inline]
    fn body_sdf(&self, u: f32, v: f32) -> f32 {
        let vs = v - self.spine(u);
        let haunch_w = 2.68 + self.breathe;
        let d_haunch = sd_ellipse(u, vs, -1.85, 0.0, 2.05, haunch_w);
        let d_hip_l = sd_ellipse(
            u,
            vs,
            -2.05 + self.hip_shift,
            -0.95,
            1.45,
            1.65 + self.breathe * 0.6,
        );
        let d_hip_r = sd_ellipse(
            u,
            vs,
            -2.05 - self.hip_shift,
            0.95,
            1.45,
            1.65 + self.breathe * 0.6,
        );
        let d_rear = smin(d_haunch, d_hip_l.min(d_hip_r), 0.75);

        let d_mid = sd_taper_capsule(
            Vec2::new(u, vs),
            Vec2::new(-1.5, 0.0),
            Vec2::new(1.05, 0.0),
            2.42 + self.breathe * 0.6,
            1.72,
        );

        let d_sh_l = sd_ellipse(u, vs, 0.92 + self.sh_shift, -0.62, 1.05, 1.18);
        let d_sh_r = sd_ellipse(u, vs, 0.92 - self.sh_shift, 0.62, 1.05, 1.18);
        let d_torso = smin(smin(d_rear, d_mid, 0.65), smin(d_sh_l, d_sh_r, 0.55), 0.55);

        let d_neck = sd_taper_capsule(
            Vec2::new(u, vs),
            Vec2::new(1.2, 0.0),
            Vec2::new(2.35, 0.0),
            1.42,
            1.38,
        );

        let d_head = sd_ellipse(u, vs, 2.95, 0.0, 1.38, 1.78);
        let d_snout = sd_taper_capsule(
            Vec2::new(u, vs),
            Vec2::new(3.25, 0.0),
            Vec2::new(5.22, 0.0),
            1.24,
            0.34,
        );
        let d_skull = smin(d_head, d_snout, 0.55);

        smin(smin(d_torso, d_neck, 0.38), d_skull, 0.32)
    }

    #[inline]
    fn ear_center_uv(&self) -> (f32, f32) {
        (2.24 - 0.55 * self.dash, 1.84 - 0.25 * self.dash)
    }

    #[inline]
    fn ear_sdf_sym(&self, u: f32, v: f32) -> f32 {
        let (eu, ev) = self.ear_center_uv();
        let du = u - eu;
        let dv = (v - self.spine(eu)).abs() - ev;
        const CA: f32 = 0.94;
        const SA: f32 = 0.34;
        let lu = du * CA + dv * SA;
        let lv = -du * SA + dv * CA;
        sd_ellipse(lu, lv, 0.0, 0.0, 0.94, 1.14)
    }
}

/// Draw the player's rat at arbitrary `scale` (`1.0..=5.0`) for the **Growth Serum**.
///
/// Matches the argument conventions of `art_rat::draw_rat_flash`, adding `scale`.
/// Authored directly from procedural geometry so the giant rat reads as a crisp,
/// detailed top-down white-and-pink rat at every scale from 1× to 5× and at any
/// 360-degree facing.
pub fn draw_giant_rat(
    pb: &mut PixelBuffer,
    player: &Player,
    elapsed: f32,
    cam: Vec2,
    scale: f32,
    tail_flash: f32,
) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = GiantPose::from_player(player, elapsed, cam, scale);
    let tint = Tint::from_player(player, elapsed);
    let lash = tail_flash.clamp(0.0, 1.0);

    let mut charge = tail_charge(player);
    if let Some(t) = charge.as_mut() {
        t.flash = lash;
    }

    if pose.dash > 0.0 {
        draw_dash_streaks(pb, &pose);
    }

    // 1. Cast ground shadow first (stamped with floor pen `0` so the lighting
    //    pass never rim-lights the floor shadow as creature fur).
    if !tint.ghost {
        let prev_pen = pb.set_pen(0);
        draw_body_shadow(pb, &pose);
        pb.set_pen(prev_pen);
    }

    // 2. Long tapering segmented pink tail (sways with movement, lashes when
    //    `tail_flash > 0`, and carries Tail Thrash charge glow).
    draw_tail(pb, &pose, player.vel, elapsed, tint, charge, lash);

    // 3. Four articulated paws with toes stepping in diagonal trot pairs.
    draw_paws(pb, &pose, tint);

    // 4. Sculpted body & head (4-shade white/lavender coat, dorsal stripe,
    //    pale belly edge, anatomical creases, fur grain, 1-px outline).
    draw_body(pb, &pose, tint);

    // 5. Two round ears with furred rims and cupped pink inner bowls.
    draw_ears(pb, &pose, tint);

    // 6. Glossy dark eyes with 1-px glint, pink nose, and crisp 1-px whiskers.
    draw_face(pb, &pose, elapsed, tint);

    // 7. Super Dash charge ring around the giant while holding dash.
    if player.dash_charging {
        draw_charge_ring(pb, w2s(player.pos, cam), player.dash_charge, elapsed, pose.scale);
    }
}

/// Speed lines peeling off the giant's haunches during a dash.
fn draw_dash_streaks(pb: &mut PixelBuffer, pose: &GiantPose) {
    let streaks = (7.0 * (0.6 + 0.4 * pose.scale)).round() as i32;
    let prev_pen = pb.set_pen(0);
    for side in [-1.0f32, 1.0] {
        for j in 0..streaks {
            let t = j as f32 / streaks.max(1) as f32;
            let p = pose.place(-3.2 - t * 6.5, side * (2.3 - 0.9 * t));
            let f = pose.dash * (1.0 - t) * 0.85;
            pb.plot_add(p.x.round() as i32, p.y.round() as i32, scale_rgb((90, 120, 170), f));
        }
    }
    pb.set_pen(prev_pen);
}

/// Soft down-right ground drop shadow under the giant body and ears.
fn draw_body_shadow(pb: &mut PixelBuffer, pose: &GiantPose) {
    let sh_offset = Vec2::new(1.15 * pose.scale + 0.5, 1.45 * pose.scale + 0.5);
    let reach = (6.8 * pose.scale * (1.0 + 0.3 * pose.dash)).ceil() as i32 + 4;
    let cx = pose.center.x.round() as i32;
    let cy = pose.center.y.round() as i32;
    let x0 = (cx - reach).max(0);
    let x1 = (cx + reach).min(pb.w as i32 - 1);
    let y0 = (cy - reach).max(0);
    let y1 = (cy + reach).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let feather = 2.0 + 0.5 * pose.scale;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - pose.center - sh_offset;
            let (u, v) = pose.to_local(d);
            if u < -4.8 || u > 6.2 || v.abs() > 3.8 {
                continue;
            }
            let dist = pose.body_sdf(u, v).min(pose.ear_sdf_sym(u, v)) * pose.scale;
            if dist < 1.5 {
                let soft = ((1.5 - dist) / feather).clamp(0.0, 1.0);
                darken(pb, x, y, 0.34 * soft);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct TailCapsule {
    a: Vec2,
    b: Vec2,
    ra: f32,
    rb: f32,
    s0: f32,
    s1: f32,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

/// Long tapering segmented pink tail: thick muscular root tapering to a fine
/// tip, with annular scale rings, cylindrical top-left lighting, a 1-px dark
/// outline, walk sway, turn lag, Tail Thrash charge glow, and whip lash when
/// `tail_flash > 0`.
fn draw_tail(
    pb: &mut PixelBuffer,
    pose: &GiantPose,
    vel: Vec2,
    elapsed: f32,
    tint: Tint,
    tail: Option<TailCharge>,
    lash: f32,
) {
    let lag = if vel.len_sq() > 4.0 {
        let vd = vel.normalized();
        let cross = pose.face.x * vd.y - pose.face.y * vd.x;
        (cross.atan2(pose.face.dot(vd)) * 1.5).clamp(-2.0, 2.0)
    } else {
        0.0
    };

    let charge = tail.map_or(0.0, |t| t.charge.clamp(0.0, 1.0));
    let ready = tail.is_some_and(|t| t.ready);
    let extra_units = tail.map_or(0.0, |t| (t.tail_len() - 13.0).max(0.0));

    let whip_damp = 1.0 - 0.85 * pose.dash;
    let base_len = TAIL_BASE_UNITS * pose.scale;
    let charge_extra = extra_units * (1.0 + (pose.scale - 1.0) * 0.35);
    let lash_extra = 2.8 * pose.scale * lash;
    let tail_len = base_len + charge_extra + lash_extra + 2.5 * pose.dash * pose.scale;

    let n_pts = ((tail_len * 0.75).round() as usize).clamp(24, TAIL_MAX_PTS - 1);
    let step_len = tail_len / n_pts as f32;
    let back = -pose.face;

    let mut pts = [Vec2::ZERO; TAIL_MAX_PTS];
    let mut p = pose.place(-3.1, 0.0);
    pts[0] = p;
    let mut min_x = p.x;
    let mut max_x = p.x;
    let mut min_y = p.y;
    let mut max_y = p.y;

    for (i, slot) in pts.iter_mut().enumerate().take(n_pts + 1).skip(1) {
        let s = (i as f32 - 0.5) / n_pts as f32;
        let idle_th = 1.55 * s.powf(1.35) + 0.24 * (elapsed * 1.7 - s * 2.2).sin();
        let run_th = 0.18 * s + 0.58 * s * (elapsed * 13.0 - s * 5.2).sin() * whip_damp;
        let lash_th = lash * (1.15 * (s * 5.5 - (1.0 - lash) * 8.5).sin() * s + 0.65 * s);
        let th = lag * s + idle_th * (1.0 - pose.moving) + run_th * pose.moving + lash_th;
        p += back.rotate(th) * step_len;
        *slot = p;
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    // Subsample spline into capsules for fast analytical SDF shading.
    let cap_count = n_pts.min(TAIL_MAX_CAPS);
    let mut caps = [TailCapsule {
        a: Vec2::ZERO,
        b: Vec2::ZERO,
        ra: 0.5,
        rb: 0.5,
        s0: 0.0,
        s1: 1.0,
        min_x: 0.0,
        max_x: 0.0,
        min_y: 0.0,
        max_y: 0.0,
    }; TAIL_MAX_CAPS];

    for (ci, cap) in caps.iter_mut().take(cap_count).enumerate() {
        let i0 = (ci * n_pts) / cap_count;
        let i1 = ((ci + 1) * n_pts) / cap_count;
        let s0 = i0 as f32 / n_pts as f32;
        let s1 = i1 as f32 / n_pts as f32;
        let r0 = ((0.64 * (1.0 - s0).powf(0.78) + 0.14 + 0.15 * charge * s0) * pose.scale).max(0.45);
        let r1 = ((0.64 * (1.0 - s1).powf(0.78) + 0.14 + 0.15 * charge * s1) * pose.scale).max(0.42);
        let a = pts[i0];
        let b = pts[i1];
        let rm = r0.max(r1) + 2.2;
        *cap = TailCapsule {
            a,
            b,
            ra: r0,
            rb: r1,
            s0,
            s1,
            min_x: a.x.min(b.x) - rm,
            max_x: a.x.max(b.x) + rm,
            min_y: a.y.min(b.y) - rm,
            max_y: a.y.max(b.y) + rm,
        };
    }
    let caps = &caps[..cap_count];

    let pad = (0.85 * pose.scale).ceil() as i32 + 3;
    let sh_ox = (0.55 * pose.scale).round().max(1.0);
    let sh_oy = (0.75 * pose.scale).round().max(1.0);

    // Tail drop shadow pass (with floor pen `0`).
    if !tint.ghost {
        let prev_pen = pb.set_pen(0);
        let sx0 = ((min_x + sh_ox).floor() as i32 - pad).max(0);
        let sx1 = ((max_x + sh_ox).ceil() as i32 + pad).min(pb.w as i32 - 1);
        let sy0 = ((min_y + sh_oy).floor() as i32 - pad).max(0);
        let sy1 = ((max_y + sh_oy).ceil() as i32 + pad).min(pb.h as i32 - 1);
        if sx0 <= sx1 && sy0 <= sy1 {
            for y in sy0..=sy1 {
                let fy = y as f32 - sh_oy;
                for x in sx0..=sx1 {
                    let fx = x as f32 - sh_ox;
                    let mut hit = false;
                    for cap in caps {
                        if fx < cap.min_x || fx > cap.max_x || fy < cap.min_y || fy > cap.max_y {
                            continue;
                        }
                        if sd_taper_capsule(Vec2::new(fx, fy), cap.a, cap.b, cap.ra, cap.rb) <= 0.35 {
                            hit = true;
                            break;
                        }
                    }
                    if hit {
                        darken(pb, x, y, 0.28);
                    }
                }
            }
        }
        pb.set_pen(prev_pen);
    }

    // Tail body + outline + annular scale rings + charge/lash highlights.
    let tx0 = (min_x.floor() as i32 - pad).max(0);
    let tx1 = (max_x.ceil() as i32 + pad).min(pb.w as i32 - 1);
    let ty0 = (min_y.floor() as i32 - pad).max(0);
    let ty1 = (max_y.ceil() as i32 + pad).min(pb.h as i32 - 1);
    if tx0 > tx1 || ty0 > ty1 {
        return;
    }

    let alpha = tint.alpha();
    let pulse = 0.5 + 0.5 * (elapsed * 6.2).sin();
    let ring_freq = 14.0f32;

    for y in ty0..=ty1 {
        let fy = y as f32;
        for x in tx0..=tx1 {
            let fx = x as f32;
            let mut best_d = f32::MAX;
            let mut best_s = 0.0f32;
            let mut best_r = 1.0f32;
            let mut best_n = Vec2::ZERO;

            for cap in caps {
                if fx < cap.min_x || fx > cap.max_x || fy < cap.min_y || fy > cap.max_y {
                    continue;
                }
                let pa = Vec2::new(fx, fy) - cap.a;
                let ba = cap.b - cap.a;
                let ba2 = ba.len_sq().max(1e-6);
                let t = (pa.dot(ba) / ba2).clamp(0.0, 1.0);
                let diff = pa - ba * t;
                let dist = diff.len();
                let r = cap.ra + (cap.rb - cap.ra) * t;
                let d = dist - r;
                if d < best_d {
                    best_d = d;
                    best_s = cap.s0 + (cap.s1 - cap.s0) * t;
                    best_r = r;
                    if dist > 1e-4 {
                        best_n = diff * (1.0 / dist);
                    }
                }
            }

            if best_d > 1.15 {
                continue;
            }
            if best_d > 0.0 {
                let oa = ((1.15 - best_d) / 1.15).clamp(0.0, 1.0) * 0.88 * alpha;
                blend(pb, x, y, tint.apply(OUTLINE), oa);
                continue;
            }

            let radial = ((best_r + best_d) / best_r.max(0.5)).clamp(0.0, 1.0);
            let lit = best_n.dot(LIGHT);
            let mut col = mix(TAIL_BASE, TAIL_TIP, best_s);
            if lit > 0.0 {
                col = mix(
                    col,
                    mix(TAIL_TIP, (255, 232, 236), 0.5),
                    lit * 0.42 * (0.4 + 0.6 * radial),
                );
            } else {
                col = mix(col, TAIL_SHADOW, (-lit) * 0.55 * (0.3 + 0.7 * radial));
            }

            // Segmented rat-tail scale rings once the tail is thick enough.
            if pose.detail > 0.15 && best_r > 0.95 {
                let ring_phase = (best_s * ring_freq) % 1.0;
                if ring_phase < 0.26 {
                    let ring_w = (1.0 - (ring_phase - 0.13).abs() / 0.13) * 0.44 * pose.detail;
                    col = mix(col, TAIL_RING, ring_w);
                }
            }

            if charge > 0.0 {
                col = mix(col, THRASH_GLOW, 0.5 * charge);
            }
            if ready || lash > 0.0 {
                let ready_term = if ready { 0.25 + 0.45 * pulse } else { 0.0 };
                let heat = ((ready_term + 0.75 * lash) * (0.2 + 0.8 * best_s * best_s)).clamp(0.0, 1.0);
                col = mix(col, THRASH_HOT, heat);
            }

            blend(pb, x, y, tint.apply(col), alpha);
            if (ready || lash > 0.2) && best_s > 0.45 {
                let g = (if ready { 0.22 * pulse } else { 0.0 }) + 0.35 * lash * best_s;
                let glow = if lash > 0.3 { THRASH_HOT } else { THRASH_GLOW };
                pb.plot_add(x, y, scale_rgb(glow, g * alpha));
            }
        }
    }

    // Subtle floor halo + tip starburst when Tail Thrash is ready (or flashing).
    if ready || lash > 0.15 {
        let prev_pen = pb.set_pen(0);
        let tip = pts[n_pts];
        let (tx, ty) = (tip.x.round() as i32, tip.y.round() as i32);
        if ready {
            let halo_gain = 0.08 * (0.55 + 0.65 * pulse + 0.8 * lash) * alpha;
            for pt in pts.iter().take(n_pts + 1).step_by(3) {
                let (px, py) = (pt.x.round() as i32, pt.y.round() as i32);
                for (dx, dy) in [(2, 0), (-2, 0), (0, 2), (0, -2), (3, 3), (-3, -3)] {
                    pb.plot_add(px + dx, py + dy, scale_rgb(THRASH_HALO, halo_gain));
                }
            }
        }
        let burst = if ready { (0.35 + 0.45 * pulse).max(lash) } else { lash };
        let r = (1.5 + 3.5 * burst) * (0.6 + 0.4 * pose.scale.min(3.0));
        for i in 0..8 {
            let ang = i as f32 * (TAU / 8.0) + 0.26;
            let dx = (ang.cos() * r).round() as i32;
            let dy = (ang.sin() * r).round() as i32;
            pb.plot_add(tx + dx, ty + dy, scale_rgb(THRASH_HOT, 0.55 * burst * alpha));
        }
        pb.set_pen(prev_pen);
    }
}

/// Four articulated paws with splayed toes stepping in diagonal trot pairs
/// (same gait phase as `art_rat::draw_feet`).
fn draw_paws(pb: &mut PixelBuffer, pose: &GiantPose, tint: Tint) {
    let reach_f = 0.32 + 0.72 * pose.moving;
    let alpha = tint.alpha();
    let paws: [(f32, f32, bool); 4] = [
        (0.45, 1.0, true),
        (-2.15, -1.0, false),
        (0.45, -1.0, true),
        (-2.15, 1.0, false),
    ];

    for (k, &(u0, side, is_front)) in paws.iter().enumerate() {
        let ph = pose.phase + if k < 2 { 0.0 } else { PI };
        let swing = ph.sin() * (if is_front { 0.85 } else { 1.05 }) * pose.moving * (1.0 - pose.dash)
            - 0.85 * pose.dash;
        let lift = ph.cos().max(0.0);
        let u_paw = u0 + swing + if is_front { 0.15 } else { -0.10 };
        let base_hw = if is_front { 1.82 } else { 2.48 };
        let v_paw = side
            * (base_hw + reach_f * (if is_front { 0.68 } else { 0.58 }) + 0.28 * lift * pose.moving);

        let root_w = pose.place(u0 + swing * 0.25, side * (base_hw - 0.45));
        let paw_w = pose.place(u_paw, v_paw);

        let paw_r = (if is_front { 0.48 } else { 0.62 }) * pose.scale;
        let limb_r = (if is_front { 0.40 } else { 0.56 }) * pose.scale;

        let toe_count: usize = if is_front { 3 } else { 4 };
        let toe_len = (if is_front { 0.60 } else { 0.78 }) * pose.scale;
        let toe_r = (0.15 * pose.scale).max(0.32);

        let mut toes = [(Vec2::ZERO, Vec2::ZERO); 4];
        let mut min_x = root_w.x.min(paw_w.x);
        let mut max_x = root_w.x.max(paw_w.x);
        let mut min_y = root_w.y.min(paw_w.y);
        let mut max_y = root_w.y.max(paw_w.y);

        for (ti, slot) in toes.iter_mut().take(toe_count).enumerate() {
            let frac = ti as f32 / (toe_count - 1).max(1) as f32 - 0.5;
            let ang = side * (0.24 + frac * 0.68);
            let tdir = pose.face.rotate(ang);
            let t1 = paw_w + tdir * (paw_r * 0.42);
            let t2 = paw_w + tdir * (paw_r * 0.52 + toe_len);
            *slot = (t1, t2);
            min_x = min_x.min(t2.x);
            max_x = max_x.max(t2.x);
            min_y = min_y.min(t2.y);
            max_y = max_y.max(t2.y);
        }

        let pad = (paw_r.max(limb_r) + 2.5).ceil() as i32;
        let x0 = (min_x.floor() as i32 - pad).max(0);
        let x1 = (max_x.ceil() as i32 + pad).min(pb.w as i32 - 1);
        let y0 = (min_y.floor() as i32 - pad).max(0);
        let y1 = (max_y.ceil() as i32 + pad).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            continue;
        }

        let lb = paw_w - root_w;
        let lb2 = lb.len_sq().max(1e-4);
        let smooth_k = (0.25 * pose.scale).max(0.5);

        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Vec2::new(x as f32, y as f32);
                let d_limb = sd_taper_capsule(p, root_w, paw_w, limb_r * 1.05, paw_r * 0.85);
                let d_pad = (p - paw_w).len() - paw_r;
                let mut d_toes = f32::MAX;
                if pose.detail > 0.15 {
                    for &(t1, t2) in &toes[..toe_count] {
                        let dt = sd_taper_capsule(p, t1, t2, toe_r * 1.1, toe_r * 0.7);
                        d_toes = d_toes.min(dt);
                    }
                }
                let d = smin(d_limb, d_pad, smooth_k).min(d_toes);
                if d > 1.1 {
                    continue;
                }
                if d > 0.0 {
                    let oa = ((1.1 - d) / 1.1).clamp(0.0, 1.0) * 0.85 * alpha;
                    blend(pb, x, y, tint.apply(OUTLINE), oa);
                    continue;
                }

                let lt = ((p - root_w).dot(lb) / lb2).clamp(0.0, 1.0);
                let lit = (p - paw_w).dot(LIGHT) / (paw_r * 1.2).max(1.0);
                let col = if d_toes <= d_limb.min(d_pad) + 0.15 && pose.detail > 0.15 {
                    let mut c = mix(PAW_SHADOW, PAW_PINK, (0.55 + 0.45 * lit).clamp(0.0, 1.0));
                    if (p - paw_w).len() > paw_r * 0.55 + toe_len * 0.65 {
                        c = mix(c, CLAW_TIP, 0.65);
                    }
                    c
                } else if lt < 0.45 && pose.detail > 0.2 {
                    mix(FUR_SHADOW, FUR_BASE, (0.5 + 0.4 * lit).clamp(0.0, 1.0))
                } else {
                    let mut c = mix(PAW_SHADOW, PAW_PINK, (0.55 + 0.45 * lit).clamp(0.0, 1.0));
                    if lit > 0.25 {
                        c = mix(c, PAW_LIGHT, ((lit - 0.25) * 0.7).clamp(0.0, 0.55));
                    }
                    c
                };
                blend(pb, x, y, tint.apply(col), alpha);
            }
        }
    }
}

/// Sculpted giant body and wedge head with 4 fur shades, dorsal spine stripe,
/// pale belly rim, anatomical haunch/neck creases, body-locked combed fur grain,
/// and a crisp 1-px darker outline.
fn draw_body(pb: &mut PixelBuffer, pose: &GiantPose, tint: Tint) {
    let reach = (6.4 * pose.scale * (1.0 + 0.3 * pose.dash)).ceil() as i32 + 4;
    let cx = pose.center.x.round() as i32;
    let cy = pose.center.y.round() as i32;
    let x0 = (cx - reach).max(0);
    let x1 = (cx + reach).min(pb.w as i32 - 1);
    let y0 = (cy - reach).max(0);
    let y1 = (cy + reach).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let alpha = tint.alpha();
    let outline_col = if tint.hurt > 0.0 { (90, 10, 18) } else { OUTLINE };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - pose.center;
            let (u, v) = pose.to_local(d);
            if u < -4.6 || u > 6.2 || v.abs() > 3.6 {
                continue;
            }
            let d_local = pose.body_sdf(u, v);
            let d_px = d_local * pose.scale;
            if d_px > 1.15 {
                continue;
            }
            if d_px > 0.0 {
                let oa = ((1.15 - d_px) / 1.15).clamp(0.0, 1.0) * 0.92 * alpha;
                blend(pb, x, y, outline_col, oa);
                continue;
            }

            // Smooth SDF gradient for 3D dome normal.
            const EPS: f32 = 0.08;
            let mut gu = (pose.body_sdf(u + EPS, v) - pose.body_sdf(u - EPS, v)) / (2.0 * EPS);
            let mut gv = (pose.body_sdf(u, v + EPS) - pose.body_sdf(u, v - EPS)) / (2.0 * EPS);
            let glen = gu.hypot(gv);
            if glen > 1e-4 {
                gu /= glen;
                gv /= glen;
            }

            let depth = (-d_local / 1.9).clamp(0.0, 1.0);
            let dome = (1.0 - (1.0 - depth) * (1.0 - depth)).clamp(0.0, 1.0).sqrt();
            let slope = (1.0 - depth).powf(0.75);
            let dir_lit = (gu * pose.light_u + gv * pose.light_v) * slope;
            let vs = v - pose.spine(u);
            let form_lit = u * 0.11 * pose.light_u + vs * 0.22 * pose.light_v;
            let lum = (0.46 + 0.32 * dome + 0.52 * dir_lit + 0.22 * form_lit).clamp(0.0, 1.15);

            // 4-shade fur ramp: FUR_DEEP → FUR_SHADOW → FUR_BASE → FUR_LIGHT.
            let mut col = if lum < 0.30 {
                mix(FUR_DEEP, FUR_SHADOW, lum / 0.30)
            } else if lum < 0.68 {
                mix(FUR_SHADOW, FUR_BASE, (lum - 0.30) / 0.38)
            } else {
                mix(FUR_BASE, FUR_LIGHT, ((lum - 0.68) / 0.42).clamp(0.0, 1.0))
            };

            if pose.detail > 0.05 {
                // 1. Darker dorsal stripe along the spine (rump to crown).
                if u > -3.50 && u < 2.55 {
                    let u_env = smoothstep(-3.50, -2.55, u) * (1.0 - smoothstep(1.35, 2.55, u));
                    let du_h = (u + 1.35) / 1.55;
                    let stripe_w = 0.62 + 0.28 * (-du_h * du_h).exp();
                    let wobble = 0.12 * (u * 4.5 + vs * 2.0).sin();
                    let st = 1.0 - smoothstep(0.12, stripe_w + wobble, vs.abs());
                    let stripe_amt = st * u_env * 0.68 * pose.detail;
                    let dorsal_col = mix(FUR_DEEP, FUR_DORSAL, (0.25 + 0.65 * lum).clamp(0.15, 0.92));
                    col = mix(col, dorsal_col, stripe_amt);
                }

                // 2. Pale warm belly edge peeking along the mid-body flank rim.
                if u > -2.9 && u < 1.6 && d_px > -3.0 * (0.4 + 0.6 * pose.detail) {
                    let flank_u = smoothstep(-2.9, -1.7, u) * (1.0 - smoothstep(0.5, 1.6, u));
                    let edge_band = smoothstep(-3.0, -1.1, d_px) * (1.0 - smoothstep(-0.65, 0.0, d_px));
                    col = mix(col, FUR_BELLY, flank_u * edge_band * 0.65 * pose.detail);
                }

                // 3. Anatomical haunch and neck creases.
                let hc_t = (u + 0.45 - 0.22 * vs.abs()) / 0.28;
                let haunch_crease = (-hc_t * hc_t).exp() * smoothstep(0.65, 1.5, vs.abs());
                let nc_t = (u - 1.82) / 0.24;
                let neck_crease = (-nc_t * nc_t).exp() * smoothstep(0.35, 1.1, vs.abs());
                let crease = (haunch_crease * 0.30 + neck_crease * 0.26).clamp(0.0, 0.38) * pose.detail;
                col = mix(col, FUR_DEEP, crease);

                // 4. Body-locked combed fur grain (moves with the body, zero shimmer).
                let g1 = hash2((u * 2.2).floor() as i32, (vs * 5.2).floor() as i32, 41);
                let g2 = hash2((u * 1.4).floor() as i32, (vs * 3.1).floor() as i32, 73);
                let fur_grain = (g1 * 0.6 + g2 * 0.4) - 0.5;
                if fur_grain > 0.0 {
                    col = mix(col, FUR_LIGHT, fur_grain * 0.24 * pose.detail);
                } else {
                    col = mix(col, FUR_SHADOW, (-fur_grain) * 0.22 * pose.detail);
                }

                // 5. Snout bridge & forehead highlight.
                if u > 2.6 && vs.abs() < 0.75 {
                    let ridge = (1.0 - vs.abs() / 0.75)
                        * smoothstep(2.6, 3.6, u)
                        * (1.0 - smoothstep(4.5, 5.2, u));
                    col = mix(col, FUR_LIGHT, ridge * 0.38 * pose.detail);
                }

                // 6. Tight inner ambient occlusion on the shaded rim.
                if d_px > -1.35 && dir_lit < 0.0 {
                    let ao = (1.0 - (-d_px) / 1.35) * (-dir_lit) * 0.45 * pose.detail;
                    col = mix(col, FUR_DEEP, ao);
                }
            }

            if d_px > -0.45 {
                let edge_t = (-d_px / 0.45).clamp(0.0, 1.0);
                col = mix(outline_col, col, 0.35 + 0.65 * edge_t);
            }

            blend(pb, x, y, tint.apply(col), alpha);
        }
    }
}

/// Two round rat ears with furred outer rims, cupped pink inner bowls, deeper
/// concha shadows, and a crisp 1-px outline.
fn draw_ears(pb: &mut PixelBuffer, pose: &GiantPose, tint: Tint) {
    let (eu, ev) = pose.ear_center_uv();
    let ear_pad = (1.60 * pose.scale).ceil() as i32 + 3;
    let alpha = tint.alpha();
    const CA: f32 = 0.94;
    const SA: f32 = 0.34;

    for side in [-1.0f32, 1.0] {
        let ear_w = pose.place(eu, side * ev);
        let ecx = ear_w.x.round() as i32;
        let ecy = ear_w.y.round() as i32;
        let x0 = (ecx - ear_pad).max(0);
        let x1 = (ecx + ear_pad).min(pb.w as i32 - 1);
        let y0 = (ecy - ear_pad).max(0);
        let y1 = (ecy + ear_pad).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            continue;
        }
        let sp_eu = pose.spine(eu);

        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Vec2::new(x as f32, y as f32);
                let (u, v) = pose.to_local(p - pose.center);
                let vs = v - sp_eu;
                if vs * side < -0.2 {
                    continue;
                }
                let du = u - eu;
                let dv = vs * side - ev;
                let lu = du * CA + dv * SA;
                let lv = -du * SA + dv * CA;
                let d_local = sd_ellipse(lu, lv, 0.0, 0.0, 0.94, 1.14);
                let d_px = d_local * pose.scale;
                if d_px > 1.1 {
                    continue;
                }
                if d_px > 0.0 {
                    let oa = ((1.1 - d_px) / 1.1).clamp(0.0, 1.0) * 0.90 * alpha;
                    blend(pb, x, y, tint.apply(OUTLINE), oa);
                    continue;
                }

                let d_inner = sd_ellipse(lu, lv, 0.12, -0.05, 0.60, 0.74);
                let d_concha = sd_ellipse(lu, lv, 0.00, -0.30, 0.33, 0.38);
                let lit = (p - ear_w).dot(LIGHT) / (1.2 * pose.scale).max(1.0);

                let col = if d_inner > 0.0 {
                    let rim_fur = mix(EAR_OUTER, FUR_LIGHT, (0.4 + 0.5 * lit).clamp(0.0, 1.0));
                    mix(EAR_RIM, rim_fur, (d_inner / 0.32).clamp(0.0, 1.0))
                } else {
                    let bowl_t = (-d_inner / 0.55).clamp(0.0, 1.0);
                    let mut c = mix(EAR_RIM, EAR_PINK, 0.35 + 0.65 * bowl_t);
                    if lit > 0.0 {
                        c = mix(c, (255, 214, 224), lit * 0.35);
                    }
                    if d_concha < 0.0 && pose.detail > 0.15 {
                        let ct = (-d_concha / 0.35).clamp(0.0, 1.0);
                        c = mix(c, EAR_DEEP, ct * 0.65 * pose.detail);
                    }
                    c
                };
                blend(pb, x, y, tint.apply(col), alpha);
            }
        }
    }
}

/// Glossy dark eyes with a crisp 1-px white glint, pink button nose with
/// specular highlight, and thin 1-px whiskers fanning out from the snout pads.
fn draw_face(pb: &mut PixelBuffer, pose: &GiantPose, elapsed: f32, tint: Tint) {
    let alpha = tint.alpha();

    // 1. Dark eyes with 1-px specular glint.
    let eye_rx = (0.40 * pose.scale).max(0.55);
    let eye_ry = (0.33 * pose.scale).max(0.55);
    for side in [-1.0f32, 1.0] {
        let eye_w = pose.place(3.45, side * 0.94);
        let ecx = eye_w.x.round() as i32;
        let ecy = eye_w.y.round() as i32;
        if pose.scale < 1.6 {
            pb.plot(ecx, ecy, EYE);
        } else {
            let er = eye_rx.max(eye_ry).ceil() as i32 + 2;
            let x0 = (ecx - er).max(0);
            let x1 = (ecx + er).min(pb.w as i32 - 1);
            let y0 = (ecy - er).max(0);
            let y1 = (ecy + er).min(pb.h as i32 - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = Vec2::new(x as f32, y as f32) - eye_w;
                    let lu = d.dot(pose.face);
                    let lv = d.dot(pose.perp);
                    let de = sd_ellipse(lu, lv, 0.0, 0.0, eye_rx, eye_ry);
                    if de > 0.9 {
                        continue;
                    }
                    if de > 0.0 {
                        let sa = ((0.9 - de) / 0.9).clamp(0.0, 1.0) * 0.45 * alpha;
                        blend(pb, x, y, tint.apply(FUR_DEEP), sa);
                    } else {
                        blend(pb, x, y, EYE, alpha);
                    }
                }
            }
            let gx = (eye_w.x + LIGHT.x * (eye_rx * 0.34).max(0.45)).round() as i32;
            let gy = (eye_w.y + LIGHT.y * (eye_ry * 0.34).max(0.45)).round() as i32;
            blend(pb, gx, gy, EYE_GLINT, 0.95 * alpha);
        }
    }

    // 2. Pink button nose at the snout tip.
    let nose_w = pose.place(5.26, 0.0);
    let ncx = nose_w.x.round() as i32;
    let ncy = nose_w.y.round() as i32;
    if pose.scale < 1.6 {
        blend(pb, ncx, ncy, tint.apply(NOSE_PINK), alpha);
    } else {
        let nrx = (0.42 * pose.scale).max(0.7);
        let nry = (0.54 * pose.scale).max(0.7);
        let nr = nrx.max(nry).ceil() as i32 + 2;
        let x0 = (ncx - nr).max(0);
        let x1 = (ncx + nr).min(pb.w as i32 - 1);
        let y0 = (ncy - nr).max(0);
        let y1 = (ncy + nr).min(pb.h as i32 - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = Vec2::new(x as f32, y as f32) - nose_w;
                let lu = d.dot(pose.face);
                let lv = d.dot(pose.perp);
                let dn = sd_ellipse(lu, lv, 0.0, 0.0, nrx, nry);
                if dn > 1.0 {
                    continue;
                }
                if dn > 0.0 {
                    let oa = ((1.0 - dn) / 1.0).clamp(0.0, 1.0) * 0.85 * alpha;
                    blend(pb, x, y, tint.apply(OUTLINE), oa);
                } else {
                    let lit = d.dot(LIGHT) / nry.max(1.0);
                    let col = mix(NOSE_DARK, NOSE_PINK, (0.6 + 0.5 * lit).clamp(0.0, 1.0));
                    blend(pb, x, y, tint.apply(col), alpha);
                }
            }
        }
        let ngx = (nose_w.x + LIGHT.x * nrx * 0.30).round() as i32;
        let ngy = (nose_w.y + LIGHT.y * nry * 0.30).round() as i32;
        blend(pb, ngx, ngy, tint.apply(NOSE_SPEC), 0.85 * alpha);
    }

    // 3. Crisp thin 1-px whiskers sweeping out and slightly back from the snout.
    if pose.scale < 1.6 {
        let twitch = 0.35 * (elapsed * 9.0).sin();
        for side in [-1.0f32, 1.0] {
            for (k, &(wu, wv)) in [(4.5f32, 1.9f32), (4.9, 2.9)].iter().enumerate() {
                let wp = pose.place(wu + twitch * (k as f32 + 1.0) * 0.5, side * wv);
                let g = if k == 0 { 70 } else { 40 };
                pb.plot_add(wp.x.round() as i32, wp.y.round() as i32, (g, g, g));
            }
        }
    } else {
        let configs: [(f32, f32, f32, f32); 3] = [
            (4.74, 0.56, 1.02, 2.35),
            (4.48, 0.68, 1.38, 2.85),
            (4.20, 0.78, 1.72, 2.30),
        ];
        for side in [-1.0f32, 1.0] {
            for (k, &(wu, wv, base_ang, wlen)) in configs.iter().enumerate() {
                let twitch = 0.07 * (elapsed * 9.0 + k as f32 * 1.1 + side * 0.4).sin();
                let ang = side * (base_ang + twitch);
                let start = pose.place(wu, side * wv);
                let wdir = pose.face.rotate(ang);
                let total_len = wlen * (0.72 + 0.28 * pose.detail) * pose.scale;
                let steps = (total_len * 1.25).ceil().max(4.0) as i32;
                let mut last = (i32::MIN, i32::MIN);
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let sweep_back = -0.22 * total_len * (t * t);
                    let pt = start + wdir * (total_len * t) + pose.face * sweep_back;
                    let sx = pt.x.round() as i32;
                    let sy = pt.y.round() as i32;
                    if (sx, sy) == last {
                        continue;
                    }
                    last = (sx, sy);
                    let wa = (0.52 + 0.18 * pose.detail) * (1.0 - 0.68 * t) * alpha;
                    blend(pb, sx, sy, tint.apply(WHISKER), wa);
                }
            }
        }
    }
}

/// Super Dash charge ring scaled around the giant rat while a charge is held.
fn draw_charge_ring(
    pb: &mut PixelBuffer,
    centre: (i32, i32),
    charge: f32,
    elapsed: f32,
    scale: f32,
) {
    let charge = charge.clamp(0.0, 1.0);
    let full = charge >= 1.0;
    let pulse = if full {
        0.62 + 0.38 * (elapsed * 9.0).sin().abs()
    } else {
        1.0
    };
    let radius = 10.0 * (1.0 + (scale - 1.0) * 0.65);
    let prev_pen = pb.set_pen(0);
    for i in 0..CHARGE_PIPS {
        let t = i as f32 / CHARGE_PIPS as f32;
        let a = -FRAC_PI_2 + TAU * t;
        let dir = Vec2::from_angle(a);
        let x = centre.0 + (dir.x * radius).round() as i32;
        let y = centre.1 + (dir.y * radius).round() as i32;
        if t < charge {
            let colour = if full { CHARGE_FULL } else { CHARGE_COLOR };
            blend(pb, x, y, colour, (0.45 + 0.55 * pulse).min(1.0));
            blend(pb, x, y + 1, colour, (0.32 + 0.42 * pulse).min(1.0));
            if full {
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    blend(pb, x + dx, y + dy, CHARGE_COLOR, 0.22 * pulse);
                }
            }
        } else {
            blend(pb, x, y, CHARGE_SOCKET, 0.7);
            blend(pb, x, y + 1, CHARGE_SOCKET, 0.45);
        }
    }
    pb.set_pen(prev_pen);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_rat::BODY_RADIUS;
    use crate::game::weapon::WeaponKind;

    const BG: Rgb = (38, 44, 48);

    fn test_rat(pos: Vec2, face: Vec2) -> Player {
        let mut p = Player::new(pos, WeaponKind::Gnaw);
        p.face = face;
        p.buffs.giant = GROWTH_SECS * 0.5;
        p
    }

    fn count_painted(pb: &PixelBuffer, bg: Rgb) -> usize {
        let mut n = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) != bg {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn giant_rat_paints_at_every_scale_and_facing() {
        let scales = [1.0f32, 1.5, 2.5, 4.0, 5.0];
        for &s in &scales {
            for i in 0..8 {
                let ang = i as f32 / 8.0 * TAU;
                let mut pb = PixelBuffer::new(240, 148);
                pb.clear(BG);
                let mut p = test_rat(Vec2::new(120.0, 74.0), Vec2::from_angle(ang));
                p.vel = p.face * 50.0;
                draw_giant_rat(&mut pb, &p, 0.4, Vec2::ZERO, s, 0.0);
                let painted = count_painted(&pb, BG);
                assert!(
                    painted > 20,
                    "scale {s}, angle {ang}: expected painted rat pixels, got {painted}"
                );
            }
        }
    }

    #[test]
    fn giant_rat_area_grows_monotonically_with_scale() {
        let scales = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let mut counts = [0usize; 5];
        for (idx, &s) in scales.iter().enumerate() {
            let mut pb = PixelBuffer::new(240, 148);
            pb.clear(BG);
            let mut p = test_rat(Vec2::new(120.0, 74.0), Vec2::new(1.0, 0.0));
            p.vel = Vec2::new(50.0, 0.0);
            draw_giant_rat(&mut pb, &p, 0.35, Vec2::ZERO, s, 0.0);
            counts[idx] = count_painted(&pb, BG);
            if idx > 0 {
                assert!(
                    counts[idx] > counts[idx - 1],
                    "painted area must grow with scale: scale {} had {} px vs scale {} with {} px",
                    s,
                    counts[idx],
                    scales[idx - 1],
                    counts[idx - 1]
                );
            }
        }
        // At 5x linear scale the giant's footprint (~5x BODY_RADIUS radius) covers
        // well over 10x the pixel area of the 1x rat.
        assert!(
            counts[4] > counts[0] * 10 && BODY_RADIUS > 0.0,
            "scale 5x ({} px) should be dramatically larger than 1x ({} px)",
            counts[4],
            counts[0]
        );
    }

    #[test]
    fn giant_rat_clips_cleanly_at_buffer_edges_and_offscreen() {
        // Zero-sized buffer must not panic.
        let mut empty = PixelBuffer::new(0, 0);
        let p0 = test_rat(Vec2::ZERO, Vec2::ZERO);
        draw_giant_rat(&mut empty, &p0, 0.0, Vec2::ZERO, 5.0, 0.5);

        let mut pb = PixelBuffer::new(40, 30);
        let positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(39.5, 29.5),
            Vec2::new(-45.0, 15.0),
            Vec2::new(20.0, -45.0),
            Vec2::new(500.0, 500.0),
            Vec2::new(20.0, 15.0),
        ];
        for &pos in &positions {
            for &s in &[1.0f32, 2.5, 5.0] {
                for &(fx, fy) in &[(0.0, 0.0), (1.0, 0.0), (0.0, -1.0), (-0.7, 0.7)] {
                    let mut p = test_rat(pos, Vec2::new(fx, fy));
                    p.vel = Vec2::new(55.0, -20.0);
                    p.dash_active = 0.12;
                    p.dash_charging = true;
                    p.dash_charge = 0.8;
                    p.hurt_flash = 0.15;
                    p.iframes = 0.4;
                    draw_giant_rat(&mut pb, &p, 1.25, Vec2::ZERO, s, 0.85);
                }
            }
        }
    }

    #[test]
    fn giant_rat_tail_flash_and_walk_frames_change_pixels() {
        let render = |pos: Vec2, cam: Vec2, flash: f32| {
            let mut pb = PixelBuffer::new(200, 140);
            pb.clear(BG);
            let mut p = test_rat(pos, Vec2::new(1.0, 0.0));
            p.vel = Vec2::new(58.0, 0.0);
            draw_giant_rat(&mut pb, &p, 0.4, cam, 5.0, flash);
            pb
        };
        let base_a = render(Vec2::new(110.0, 70.0), Vec2::ZERO, 0.0);
        let base_b = render(Vec2::new(110.0, 70.0), Vec2::ZERO, 0.0);
        let lashed = render(Vec2::new(110.0, 70.0), Vec2::ZERO, 0.85);
        // Advance position and camera together by half a stride so the rat stays
        // at the exact same screen pixel while its trot phase flips by PI.
        let step = PI / 1.15;
        let half_stride = render(Vec2::new(110.0 + step, 70.0), Vec2::new(step, 0.0), 0.0);

        let mut lash_diff = 0usize;
        let mut walk_diff = 0usize;
        for y in 0..base_a.h {
            for x in 0..base_a.w {
                assert_eq!(
                    base_a.pixel_at(x, y),
                    base_b.pixel_at(x, y),
                    "draw_giant_rat must be deterministic at ({x}, {y})"
                );
                if base_a.pixel_at(x, y) != lashed.pixel_at(x, y) {
                    lash_diff += 1;
                }
                if base_a.pixel_at(x, y) != half_stride.pixel_at(x, y) {
                    walk_diff += 1;
                }
            }
        }
        assert!(lash_diff > 40, "tail_flash should visibly lash the tail ({lash_diff} px changed)");
        assert!(walk_diff > 40, "walk cycle should visibly move paws/body ({walk_diff} px changed)");
    }

    #[test]
    fn giant_rat_drop_shadow_leaves_floor_material_pen() {
        let mut pb = PixelBuffer::new(160, 120);
        pb.set_materials(true);
        pb.clear(BG);
        let creature_pen = 42u8;
        let prev = pb.set_pen(creature_pen);
        let p = test_rat(Vec2::new(80.0, 60.0), Vec2::new(1.0, 0.0));
        draw_giant_rat(&mut pb, &p, 0.2, Vec2::ZERO, 5.0, 0.0);
        pb.set_pen(prev);

        let mut creature_px = 0usize;
        let mut shadow_floor_px = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                let m = pb.mat()[y * pb.w + x];
                if m == creature_pen {
                    creature_px += 1;
                } else if m == 0 && c != BG {
                    shadow_floor_px += 1;
                }
            }
        }
        assert!(creature_px > 400, "body should stamp creature pen ({creature_px} px)");
        assert!(
            shadow_floor_px > 50,
            "drop shadow should darken floor with pen 0 ({shadow_floor_px} px)"
        );
    }
}
