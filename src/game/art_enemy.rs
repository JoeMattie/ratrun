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
//! Performance: up to 600 enemies can be active at once. Every draw path uses
//! zero heap allocations, early screen-bounds culling, oriented tight bounding
//! boxes, incremental affine `(u, v)` stepping, precomputed SDF reciprocals,
//! and compile-time math lookup tables (zero per-pixel trig or `powf`). For
//! massive crowds (`visible_count > CROWD_LOD_THRESHOLD`) or tiny radii
//! (`r < 1.8`), [`draw_enemy_lod`] / [`draw_enemy_crowd`] provide a zero-`sqrt`
//! level-of-detail path that preserves silhouette, colour identity, facing,
//! hit flash, and ranged telegraphs.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

use super::enemy::{Enemy, EnemyKind};

/// Direction *toward* the light in screen space (up-left), matching
/// `art_rat.rs` and `art_props.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Hit-flash peak duration in `Enemy::hurt`.
const FLASH_MAX: f32 = 0.12;

// ---- Compile-time math lookup tables (zero heap, zero per-pixel trig/powf) ---

const SIN_LUT_BITS: usize = 10;
const SIN_LUT_LEN: usize = 1 << SIN_LUT_BITS; // 1024
const SIN_LUT_MASK: usize = SIN_LUT_LEN - 1;
const COS_LUT_QUARTER: usize = SIN_LUT_LEN / 4;
const INV_TAU_LUT: f32 = SIN_LUT_LEN as f32 / TAU;

const fn const_sin_reduced(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x2 * x;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;
    let x11 = x9 * x2;
    x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0 + x9 / 362_880.0 - x11 / 39_916_800.0
}

const fn const_sin(mut x: f32) -> f32 {
    while x > PI {
        x -= TAU;
    }
    while x < -PI {
        x += TAU;
    }
    if x > FRAC_PI_2 {
        const_sin_reduced(PI - x)
    } else if x < -FRAC_PI_2 {
        const_sin_reduced(-PI - x)
    } else {
        const_sin_reduced(x)
    }
}

const fn const_ln(mut x: f32) -> f32 {
    const LN2: f32 = core::f32::consts::LN_2;
    let mut k = 0i32;
    while x > core::f32::consts::SQRT_2 {
        x *= 0.5;
        k += 1;
    }
    while x < core::f32::consts::FRAC_1_SQRT_2 {
        x *= 2.0;
        k -= 1;
    }
    let y = (x - 1.0) / (x + 1.0);
    let y2 = y * y;
    let mut term = y;
    let mut sum = term;
    term *= y2;
    sum += term / 3.0;
    term *= y2;
    sum += term / 5.0;
    term *= y2;
    sum += term / 7.0;
    term *= y2;
    sum += term / 9.0;
    term *= y2;
    sum += term / 11.0;
    2.0 * sum + k as f32 * LN2
}

const fn const_exp(x: f32) -> f32 {
    const LN2: f32 = core::f32::consts::LN_2;
    let kf = x / LN2;
    let k = if kf >= 0.0 {
        (kf + 0.5) as i32
    } else {
        (kf - 0.5) as i32
    };
    let r = x - k as f32 * LN2;
    let r2 = r * r;
    let r3 = r2 * r;
    let r4 = r2 * r2;
    let r5 = r4 * r;
    let r6 = r4 * r2;
    let r7 = r6 * r;
    let r8 = r4 * r4;
    let mut e = 1.0
        + r
        + r2 / 2.0
        + r3 / 6.0
        + r4 / 24.0
        + r5 / 120.0
        + r6 / 720.0
        + r7 / 5040.0
        + r8 / 40_320.0;
    let mut n = k;
    while n > 0 {
        e *= 2.0;
        n -= 1;
    }
    while n < 0 {
        e *= 0.5;
        n += 1;
    }
    e
}

const fn const_pow(x: f32, p: f32) -> f32 {
    if x <= 0.0 {
        0.0
    } else {
        const_exp(p * const_ln(x))
    }
}

const fn build_sin_lut() -> [f32; SIN_LUT_LEN + 1] {
    let mut tbl = [0.0f32; SIN_LUT_LEN + 1];
    let mut i = 0;
    while i < SIN_LUT_LEN {
        tbl[i] = const_sin(i as f32 * TAU / SIN_LUT_LEN as f32);
        i += 1;
    }
    tbl[SIN_LUT_LEN] = 0.0;
    tbl
}

static SIN_LUT: [f32; SIN_LUT_LEN + 1] = build_sin_lut();

#[inline(always)]
fn fast_sin(x: f32) -> f32 {
    let scaled = x * INV_TAU_LUT;
    let floored = scaled.floor();
    let frac = scaled - floored;
    let idx = (floored as i32 as usize) & SIN_LUT_MASK;
    let a = SIN_LUT[idx];
    a + (SIN_LUT[idx + 1] - a) * frac
}

#[inline(always)]
fn fast_sin_cos(x: f32) -> (f32, f32) {
    let scaled = x * INV_TAU_LUT;
    let floored = scaled.floor();
    let frac = scaled - floored;
    let idx = (floored as i32 as usize) & SIN_LUT_MASK;
    let cidx = (idx + COS_LUT_QUARTER) & SIN_LUT_MASK;
    let sa = SIN_LUT[idx];
    let ca = SIN_LUT[cidx];
    (
        sa + (SIN_LUT[idx + 1] - sa) * frac,
        ca + (SIN_LUT[cidx + 1] - ca) * frac,
    )
}

/// Precomputed Bat M-wing profile `(elbow_u_unit, chord_unit)` over `t in [0, 1]`.
const WING_LUT_N: usize = 128;
const fn build_bat_wing_lut() -> [(f32, f32); WING_LUT_N + 1] {
    let mut tbl = [(0.0f32, 0.0f32); WING_LUT_N + 1];
    let mut i = 0;
    while i <= WING_LUT_N {
        let t = i as f32 / WING_LUT_N as f32;
        let elbow = if t < 0.48 {
            let k = t / 0.48;
            -0.25 + 1.85 * k * k * (3.0 - 2.0 * k)
        } else {
            let k = (t - 0.48) / 0.52;
            1.6 - 2.15 * const_pow(k, 1.15)
        };
        let sc = const_sin(t * TAU);
        let scallop = if sc < 0.0 { -sc } else { sc };
        let raw_chord = 2.45 * (1.0 - t * 0.84) - 0.85 * scallop * (1.0 - t * 0.25);
        let chord = if raw_chord > 0.28 { raw_chord } else { 0.28 };
        tbl[i] = (elbow, chord);
        i += 1;
    }
    tbl
}

static BAT_WING_LUT: [(f32, f32); WING_LUT_N + 1] = build_bat_wing_lut();

#[inline(always)]
fn bat_wing_profile(t: f32) -> (f32, f32) {
    let f = t * WING_LUT_N as f32;
    let i = (f as usize).min(WING_LUT_N - 1);
    let frac = f - i as f32;
    let (e0, c0) = BAT_WING_LUT[i];
    let (e1, c1) = BAT_WING_LUT[i + 1];
    (e0 + (e1 - e0) * frac, c0 + (c1 - c0) * frac)
}

/// Precomputed Boss forewing & hindwing width envelopes `(fore_env, hind_env)` over `t in [0, 1]`.
const fn build_boss_wing_lut() -> [(f32, f32); WING_LUT_N + 1] {
    let mut tbl = [(0.0f32, 0.0f32); WING_LUT_N + 1];
    let mut i = 0;
    while i <= WING_LUT_N {
        let t = i as f32 / WING_LUT_N as f32;
        let omt = 1.0 - t;
        let f_raw = const_pow(t, 0.42) * const_pow(omt, 0.6) * 2.2;
        let h_raw = const_pow(t, 0.45) * const_pow(omt, 0.6) * 2.2;
        let f_env = if f_raw > 1.0 { 1.0 } else { f_raw };
        let h_env = if h_raw > 1.0 { 1.0 } else { h_raw };
        tbl[i] = (f_env, h_env);
        i += 1;
    }
    tbl
}

static BOSS_WING_LUT: [(f32, f32); WING_LUT_N + 1] = build_boss_wing_lut();

#[inline(always)]
fn boss_wing_fore_env(t: f32) -> f32 {
    let f = t * WING_LUT_N as f32;
    let i = (f as usize).min(WING_LUT_N - 1);
    let frac = f - i as f32;
    let a = BOSS_WING_LUT[i].0;
    a + (BOSS_WING_LUT[i + 1].0 - a) * frac
}

#[inline(always)]
fn boss_wing_hind_env(t: f32) -> f32 {
    let f = t * WING_LUT_N as f32;
    let i = (f as usize).min(WING_LUT_N - 1);
    let frac = f - i as f32;
    let a = BOSS_WING_LUT[i].1;
    a + (BOSS_WING_LUT[i + 1].1 - a) * frac
}

/// Precomputed Spitter wing teardrop profile `sqrt(4 * t * (1 - t))`.
const fn build_spitter_wing_lut() -> [f32; WING_LUT_N + 1] {
    let mut tbl = [0.0f32; WING_LUT_N + 1];
    let mut i = 0;
    while i <= WING_LUT_N {
        let t = i as f32 / WING_LUT_N as f32;
        let v = t * (1.0 - t) * 4.0;
        tbl[i] = const_pow(v, 0.5);
        i += 1;
    }
    tbl
}

static SPITTER_WING_LUT: [f32; WING_LUT_N + 1] = build_spitter_wing_lut();

#[inline(always)]
fn spitter_wing_profile(t: f32) -> f32 {
    let f = t * WING_LUT_N as f32;
    let i = (f as usize).min(WING_LUT_N - 1);
    let frac = f - i as f32;
    let a = SPITTER_WING_LUT[i];
    a + (SPITTER_WING_LUT[i + 1] - a) * frac
}

/// Precomputed feline tail curl curve `1.75 * t.powf(1.25)` for the 16 tail segments.
const CAT_TAIL_STEPS: usize = 16;
const fn build_cat_tail_curl() -> [f32; CAT_TAIL_STEPS] {
    let mut tbl = [0.0f32; CAT_TAIL_STEPS];
    let mut i = 0;
    while i < CAT_TAIL_STEPS {
        let t = (i as f32 + 0.5) / CAT_TAIL_STEPS as f32;
        tbl[i] = 1.75 * const_pow(t, 1.25);
        i += 1;
    }
    tbl
}

static CAT_TAIL_CURL: [f32; CAT_TAIL_STEPS] = build_cat_tail_curl();

/// Precomputed unit directions `(cos, sin)` for the Boss's 10-bullet nova telegraph.
const fn build_nova_10() -> [(f32, f32); 10] {
    let mut tbl = [(0.0f32, 0.0f32); 10];
    let mut k = 0;
    while k < 10 {
        let a = k as f32 / 10.0 * TAU;
        tbl[k] = (const_sin(a + FRAC_PI_2), const_sin(a));
        k += 1;
    }
    tbl
}

static NOVA_10: [(f32, f32); 10] = build_nova_10();

/// Conservative screen-space culling radius per kind (covers legs, wings, tail,
/// shadow, and boss HP ring).
#[inline(always)]
fn cull_reach(kind: EnemyKind, r: f32) -> f32 {
    match kind {
        EnemyKind::Skitterer => r * 2.2 + 2.0, // 5.5 * (r/2.5) + 2
        EnemyKind::Bat => r * 4.25 + 2.0,      // 8.5 * (r/2.0) + 2
        EnemyKind::Cat => r * 3.0 + 2.0,       // 12.0 * (r/4.0) + 2
        EnemyKind::Spitter => r * 2.2 + 3.0,   // 6.6 * (r/3.0) + 3
        EnemyKind::Brute => r * 1.8 + 2.0,     // 11.7 * (r/6.5) + 2
        EnemyKind::Boss => r * 1.5 + 3.0,      // 21.0 * (r/14.0) + 3
        // Wave 3. The Flea's landing crosses sit up to 26 px from the body and
        // the Turnkey's crossbar up to ~70 px behind it (it walks away for the
        // 2.8 s the bar lives), so those two need a much wider screen bound.
        EnemyKind::Pillbug => r * 2.1 + 3.0,
        EnemyKind::BellFlea => r + 36.0,
        EnemyKind::WickMoth => r * 3.0 + 4.0,
        EnemyKind::Porter => r * 2.4 + 6.0,
        EnemyKind::Turnkey => r + 80.0,
        // Wave 4. The snake is drawn as a whole chain of body points behind
        // the head, so its bound is the body length; the horde is drawn far
        // bigger than its collision radius.
        EnemyKind::BigSnake => r + (crate::game::enemy::SNAKE_MAX_SEGMENTS as f32) * 8.0,
        EnemyKind::HouseCentipede => r * 4.0 + 6.0,
        EnemyKind::MurderHornet => r * 4.5 + 6.0,
        EnemyKind::TermiteHorde => r * 4.0 + 8.0,
        EnemyKind::Lilliputian => r * 4.0 + 6.0,
    }
}

/// Visible enemy count above which callers (e.g. `World::draw`) should switch
/// non-boss enemies to [`draw_enemy_lod`] to preserve frame headroom.
pub const CROWD_LOD_THRESHOLD: usize = 120;

/// Draw one enemy at `e.pos`, facing `e.facing`. `e.anim` is its own animation
/// phase, `e.flash > 0` means it was just hit, and `accent` is the current
/// map's accent colour.
pub fn draw_enemy(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    draw_enemy_styled(pb, e, cam, accent, e.elite);
}

/// Draw one enemy using the fast level-of-detail path (for massive crowds or
/// tiny radii), preserving silhouette, colour identity, facing, hit flash, and
/// attack telegraphs with zero per-pixel `sqrt` or trig.
pub fn draw_enemy_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    let Some(fr) = prepare_frame(pb, e, cam, accent) else {
        return;
    };
    draw_enemy_lod_frame(pb, &fr, e, cam, e.elite);
}

/// Crowd-aware entry point: uses full detail up to [`CROWD_LOD_THRESHOLD`]
/// visible enemies (and always for bosses), switching smaller swarmers to the
/// fast level-of-detail path beyond that threshold.
pub fn draw_enemy_crowd(
    pb: &mut PixelBuffer,
    e: &Enemy,
    cam: Vec2,
    accent: Rgb,
    visible_count: usize,
) {
    if visible_count > CROWD_LOD_THRESHOLD && e.kind != EnemyKind::Boss {
        draw_enemy_lod(pb, e, cam, accent);
    } else {
        draw_enemy(pb, e, cam, accent);
    }
}

/// The wave-3 tells that are anchored to a body but are warnings in their own
/// right — the Bell Flea's landing crosses and the Turnkey's crossbar — drawn
/// **without** the body. `World::draw_hostile_layer` re-stamps them after the
/// lighting pass with this; the sprites are the ones the body pass draws.
pub fn draw_w3_tells(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    if !matches!(e.kind, EnemyKind::BellFlea | EnemyKind::Turnkey) {
        return;
    }
    let Some(fr) = prepare_frame(pb, e, cam, accent) else {
        return;
    };
    match e.kind {
        EnemyKind::BellFlea => draw_bellflea_tells(pb, &fr, e, cam),
        EnemyKind::Turnkey => draw_turnkey_bar(pb, &fr, e, cam),
        _ => {}
    }
}

/// The wave-4 tells that are drawn on the floor under (and over the lighting
/// of) the body: the snake's lunge lane, the centipede's flutter line and the
/// hornet's dive line. `World::draw_hostile_layer` stamps them after the
/// lighting pass, exactly like [`draw_w3_tells`].
pub fn draw_wave4_tells(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) {
    if !e.kind.is_wave4() {
        return;
    }
    let Some(fr) = prepare_frame(pb, e, cam, accent) else {
        return;
    };
    match e.kind {
        EnemyKind::BigSnake => draw_snake_warn(pb, &fr, e),
        EnemyKind::HouseCentipede => draw_centipede_warn(pb, &fr, e),
        EnemyKind::MurderHornet => draw_hornet_warn(pb, &fr, e),
        EnemyKind::TermiteHorde => draw_horde_split_warn(pb, &fr, e),
        EnemyKind::Lilliputian => draw_lilliputian_warn(pb, &fr, e),
        _ => {}
    }
}

/// Shared early-out (screen culling + sub-pixel fast path) and local-frame setup.
#[inline(always)]
fn prepare_frame(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2, accent: Rgb) -> Option<Frame> {
    if pb.w == 0 || pb.h == 0 {
        return None;
    }
    let pos = e.pos - cam;
    let r = e.radius.max(1.0);
    let reach = cull_reach(e.kind, r);
    if pos.x + reach < -1.0
        || pos.x - reach > pb.w as f32 + 1.0
        || pos.y + reach < -1.0
        || pos.y - reach > pb.h as f32 + 1.0
    {
        return None;
    }
    // Sub-pixel early-out for vanishingly tiny radii.
    if e.radius < 0.6 {
        let px = pos.x.round() as i32;
        let py = pos.y.round() as i32;
        let hit = (e.flash / FLASH_MAX).clamp(0.0, 1.0);
        darken_px(pb, px + 1, py + 1, 0.35);
        pb.plot(px, py, apply_hit(e.kind.color(), hit));
        return None;
    }

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
    Some(Frame {
        pos,
        f,
        p,
        r,
        anim: e.anim,
        hit,
        flash_col,
        flash_str,
        hp_frac: if e.max_hp > 0.0 {
            (e.hp / e.max_hp).clamp(0.0, 1.0)
        } else {
            1.0
        },
        fl: f.x * LIGHT.x + f.y * LIGHT.y,
        pl: p.x * LIGHT.x + p.y * LIGHT.y,
        accent,
    })
}

/// Draw one enemy with an explicit `elite` flag (used for gilded elite brutes).
pub fn draw_enemy_styled(
    pb: &mut PixelBuffer,
    e: &Enemy,
    cam: Vec2,
    accent: Rgb,
    elite: bool,
) {
    let Some(fr) = prepare_frame(pb, e, cam, accent) else {
        return;
    };
    // Automatic LOD for tiny radii below the smallest standard kind (Bat = 2.0).
    if e.radius < 1.8 {
        draw_enemy_lod_frame(pb, &fr, e, cam, elite);
        return;
    }

    match e.kind {
        EnemyKind::Skitterer => draw_skitterer(pb, &fr),
        EnemyKind::Bat => draw_bat(pb, &fr),
        EnemyKind::Cat => draw_cat(pb, &fr, e.vel, e.contact_cd),
        EnemyKind::Spitter => draw_spitter(pb, &fr, e.fire_cd),
        EnemyKind::Brute => draw_brute(pb, &fr, elite),
        EnemyKind::Boss => draw_boss(pb, &fr, e.fire_cd),
        EnemyKind::Pillbug => draw_pillbug(pb, &fr, e, elite),
        EnemyKind::BellFlea => draw_bellflea(pb, &fr, e, cam, elite),
        EnemyKind::WickMoth => draw_wickmoth(pb, &fr, e, elite),
        EnemyKind::Porter => draw_porter(pb, &fr, e, elite),
        EnemyKind::Turnkey => draw_turnkey(pb, &fr, e, cam, elite),
        EnemyKind::BigSnake => draw_bigsnake(pb, &fr, e, elite),
        EnemyKind::HouseCentipede => draw_centipede(pb, &fr, e, elite),
        EnemyKind::MurderHornet => draw_hornet(pb, &fr, e, elite),
        EnemyKind::TermiteHorde => draw_horde(pb, &fr, e, elite),
        EnemyKind::Lilliputian => draw_lilliputian(pb, &fr, e, elite),
    }
}

#[inline]
fn draw_enemy_lod_frame(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    match e.kind {
        EnemyKind::Skitterer => draw_skitterer_lod(pb, fr),
        EnemyKind::Bat => draw_bat_lod(pb, fr),
        EnemyKind::Cat => draw_cat_lod(pb, fr, e.contact_cd),
        EnemyKind::Spitter => draw_spitter_lod(pb, fr, e.fire_cd),
        EnemyKind::Brute => draw_brute_lod(pb, fr, elite),
        EnemyKind::Boss => draw_boss_lod(pb, fr, e.fire_cd),
        EnemyKind::Pillbug => draw_pillbug_lod(pb, fr, e, elite),
        EnemyKind::BellFlea => draw_bellflea_lod(pb, fr, e, cam, elite),
        EnemyKind::WickMoth => draw_wickmoth_lod(pb, fr, e, elite),
        EnemyKind::Porter => draw_porter_lod(pb, fr, e, elite),
        EnemyKind::Turnkey => draw_turnkey_lod(pb, fr, e, cam, elite),
        EnemyKind::BigSnake => draw_bigsnake_lod(pb, fr, e, elite),
        EnemyKind::HouseCentipede => draw_centipede_lod(pb, fr, e, elite),
        EnemyKind::MurderHornet => draw_hornet_lod(pb, fr, e, elite),
        EnemyKind::TermiteHorde => draw_horde_lod(pb, fr, e, elite),
        EnemyKind::Lilliputian => draw_lilliputian_lod(pb, fr, e, elite),
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
    flash_col: Rgb,
    flash_str: f32,
    hp_frac: f32,
    fl: f32,
    pl: f32,
    accent: Rgb,
}

impl Frame {
    #[inline(always)]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        Vec2::new(
            self.pos.x + self.f.x * u + self.p.x * v,
            self.pos.y + self.f.y * u + self.p.y * v,
        )
    }

    #[inline(always)]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    #[inline(always)]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline(always)]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    #[inline(always)]
    fn apply_hit(&self, c: Rgb) -> Rgb {
        if self.flash_str <= 0.0 {
            c
        } else {
            mix_rgb(c, self.flash_col, self.flash_str)
        }
    }

    /// Tight oriented screen bounding box for local `[u_min, u_max] × [-v_max, v_max]`,
    /// expanded by `(pad_x_pos, pad_y_pos)` in `+x/+y` for drop shadows.
    #[inline(always)]
    fn bbox_oriented(
        &self,
        pb: &PixelBuffer,
        u_min: f32,
        u_max: f32,
        v_max: f32,
        pad_x_pos: i32,
        pad_y_pos: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        let u_mid = 0.5 * (u_min + u_max);
        let hu = 0.5 * (u_max - u_min);
        let cx = self.pos.x + self.f.x * u_mid;
        let cy = self.pos.y + self.f.y * u_mid;
        let hx = hu * self.f.x.abs() + v_max * self.p.x.abs();
        let hy = hu * self.f.y.abs() + v_max * self.p.y.abs();
        let x0 = ((cx - hx).floor() as i32).max(0);
        let x1 = ((cx + hx).ceil() as i32 + pad_x_pos).min(pb.w as i32 - 1);
        let y0 = ((cy - hy).floor() as i32).max(0);
        let y1 = ((cy + hy).ceil() as i32 + pad_y_pos).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            None
        } else {
            Some((x0, x1, y0, y1))
        }
    }

    /// Intersects [`Self::bbox_oriented`] with a circular bound `circ_r` centered
    /// at `self.pos` — trims the empty corners of diagonal `+`-shaped sprites (e.g. Bat).
    #[inline(always)]
    fn bbox_oriented_circ(
        &self,
        pb: &PixelBuffer,
        u_min: f32,
        u_max: f32,
        v_max: f32,
        circ_r: f32,
        pad: (i32, i32),
    ) -> Option<(i32, i32, i32, i32)> {
        let (x0, x1, y0, y1) =
            self.bbox_oriented(pb, u_min, u_max, v_max, pad.0, pad.1)?;
        let rr = circ_r.ceil() as i32;
        let cx = self.pos.x.round() as i32;
        let cy = self.pos.y.round() as i32;
        let x0 = x0.max(cx - rr).max(0);
        let x1 = x1.min(cx + rr + pad.0).min(pb.w as i32 - 1);
        let y0 = y0.max(cy - rr).max(0);
        let y1 = y1.min(cy + rr + pad.1).min(pb.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            None
        } else {
            Some((x0, x1, y0, y1))
        }
    }
}

// ---- Fast inline pixel & colour helpers -------------------------------------

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

#[inline(always)]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.65 {
        mix_rgb(shadow, mid, l * (1.0 / 0.65))
    } else {
        mix_rgb(mid, hi, (l - 0.65) * 2.0)
    }
}

/// Sculptural warm-white hit flash: snaps bright on impact while preserving
/// slight form contrast so struck enemies stay distinct from the cool-white rat.
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
    let strength = (0.30 + hit * 0.56).min(0.86);
    mix_rgb(c, flash_col, strength)
}

#[inline]
fn line_blend(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let ady = (b.1 - a.1).abs();
    // Fast path for 1- or 2-pixel leg/antenna segments (avoids Bresenham setup).
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
    let (bw, bh) = (pb.w as i32, pb.h as i32);
    let inside = a.0 >= 0 && a.1 >= 0 && a.0 < bw && a.1 < bh && b.0 >= 0 && b.1 >= 0 && b.0 < bw && b.1 < bh;
    if inside {
        loop {
            let bg = pb.pixel_at(x as usize, y as usize);
            pb.plot(x, y, mix_rgb(bg, c, alpha));
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
    } else {
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

/// Precomputed ellipse SDF (eliminates two per-pixel divisions).
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

    /// Precomputed squared-norm threshold `(1 + max_d / min_r)^2` for `eval_near` / `within_sq`.
    #[inline(always)]
    fn r2_limit(&self, max_d: f32) -> f32 {
        let k = (1.0 + max_d / self.min_r).max(0.0);
        k * k
    }

    /// Evaluates exact SDF only when `d < max_d` (via squared norm `r2 < lim_sq`),
    /// avoiding `sqrt()` on far exterior pixels.
    #[inline(always)]
    fn eval_near(&self, u: f32, v: f32, lim_sq: f32) -> f32 {
        let nu = (u - self.cu) * self.inv_ru;
        let nv = (v - self.cv) * self.inv_rv;
        let r2 = nu * nu + nv * nv;
        if r2 >= lim_sq {
            10.0
        } else {
            (r2.sqrt() - 1.0) * self.min_r
        }
    }

    /// Zero-`sqrt` exact check for `self.eval(u, v) < d_thresh` using precomputed `lim_sq`.
    #[inline(always)]
    fn within_sq(&self, u: f32, v: f32, lim_sq: f32) -> bool {
        let nu = (u - self.cu) * self.inv_ru;
        let nv = (v - self.cv) * self.inv_rv;
        nu * nu + nv * nv < lim_sq
    }
}

/// Precomputed capsule SDF (eliminates per-pixel segment projection division).
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
    fn new(au: f32, av: f32, bu: f32, bv: f32, rad: f32) -> Self {
        let bax = bu - au;
        let bay = bv - av;
        let inv_denom = 1.0 / (bax * bax + bay * bay).max(1e-6);
        Self {
            au,
            av,
            bax,
            bay,
            inv_denom,
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

/// Precomputed tapered capsule SDF (radius interpolates from `r0` at `a` to `r1` at `b`).
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
fn smin_inv(a: f32, b: f32, k: f32, inv_k: f32) -> f32 {
    let diff = (a - b).abs();
    if diff >= k {
        a.min(b)
    } else {
        let h = (k - diff) * inv_k;
        a.min(b) - h * h * k * 0.25
    }
}

// ============================================================================
// 1. SKITTERER — Sleek Copper Sewer Roach / Scuttler (r = 2.5)
// ============================================================================

fn draw_skitterer(pb: &mut PixelBuffer, fr: &Frame) {
    let s = fr.r * (1.0 / 2.5);
    let leg_col = fr.apply_hit((144, 86, 46));
    let foot_col = fr.apply_hit((98, 56, 30));
    let ant_col = fr.apply_hit((184, 122, 68));
    let ant_tip = scale_rgb(ant_col, 0.72);

    // 6 crisp alternating tripod-gait legs with visible copper knees.
    for (i, &u_hip) in [1.1f32, -0.35, -1.75].iter().enumerate() {
        let phase = fr.anim + i as f32 * 2.1;
        let sw_base = fast_sin(phase) * 0.85 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw_base), (1.0, -sw_base)] {
            let hip = fr.px(uh, side * 1.15 * s);
            let knee = fr.px(uh + swing * 0.45, side * 2.45 * s);
            let foot = fr.px(uh + swing - 0.3 * s, side * 3.25 * s);
            darken_px(pb, foot.0 + 1, foot.1 + 1, 0.25);
            line_blend(pb, hip, knee, leg_col, 0.92);
            line_blend(pb, knee, foot, foot_col, 0.85);
        }
    }

    // Sweeping twitching roach antennae that telegraph facing clearly.
    let wave = fast_sin(fr.anim * 0.7) * 0.42 * s;
    for side in [-1.0f32, 1.0] {
        let a0 = fr.px(2.25 * s, side * 0.55 * s);
        let a1 = fr.px(3.55 * s, side * 1.15 * s + wave * side * 0.5);
        let a2 = fr.px(4.35 * s, side * 1.95 * s + wave * side);
        line_blend(pb, a0, a1, ant_col, 0.82);
        line_blend(pb, a1, a2, ant_tip, 0.62);
    }

    // Elongated copper-mahogany carapace: tapered abdomen + pinched head.
    let Some((x0, x1, y0, y1)) =
        fr.bbox_oriented_circ(pb, -4.0 * s, 4.0 * s, 2.75 * s, 4.3 * s, (1, 1))
    else {
        return;
    };
    let shadow_c = (72, 36, 18);
    let mid_c = (196, 118, 52);
    let hi_c = (252, 206, 126);
    let seam_c = (48, 22, 12);
    let rim_col = if fr.hit > 0.4 { (160, 110, 60) } else { (0, 0, 0) };

    let abd_sdf = EllipseSdf::new(-0.75 * s, 0.0, 2.25 * s, 1.75 * s);
    let thorax_sdf = EllipseSdf::new(0.95 * s, 0.0, 1.05 * s, 1.35 * s);
    let head_sdf = EllipseSdf::new(2.15 * s, 0.0, 0.82 * s, 1.05 * s);
    let k1 = 0.35 * s;
    let inv_k1 = 1.0 / k1;
    let k2 = 0.25 * s;
    let inv_k2 = 1.0 / k2;

    let rim_lim = 0.85 * s;
    let sh_lim = 0.35 * s;
    // Near-field r^2 limits for zero-sqrt rejection (include smin smoothing margin).
    let abd_near = abd_sdf.r2_limit(rim_lim + k1 + 0.05);
    let thx_near = thorax_sdf.r2_limit(rim_lim + k1 + k2 + 0.05);
    let head_near = head_sdf.r2_limit(rim_lim + k2 + 0.05);
    let abd_sh_sq = abd_sdf.r2_limit(sh_lim);
    let thx_sh_sq = thorax_sdf.r2_limit(sh_lim);
    let head_sh_sq = head_sdf.r2_limit(sh_lim);
    let abd_sh_smin_sq = abd_sdf.r2_limit(sh_lim + k1);
    let thx_sh_smin_sq = thorax_sdf.r2_limit(sh_lim + k1 + k2);
    let head_sh_smin_sq = head_sdf.r2_limit(sh_lim + k2);

    let body_sd = |u: f32, v: f32| -> f32 {
        let abd = abd_sdf.eval_near(u, v, abd_near);
        let thorax = thorax_sdf.eval_near(u, v, thx_near);
        let head = head_sdf.eval_near(u, v, head_near);
        smin_inv(smin_inv(abd, thorax, k1, inv_k1), head, k2, inv_k2)
    };

    let body_sh_hit = |su: f32, sv: f32| -> bool {
        if abd_sdf.within_sq(su, sv, abd_sh_sq)
            || thorax_sdf.within_sq(su, sv, thx_sh_sq)
            || head_sdf.within_sq(su, sv, head_sh_sq)
        {
            return true;
        }
        // Only evaluate smin when at least one shape is within smoothing reach.
        if abd_sdf.within_sq(su, sv, abd_sh_smin_sq)
            || thorax_sdf.within_sq(su, sv, thx_sh_smin_sq)
            || head_sdf.within_sq(su, sv, head_sh_smin_sq)
        {
            body_sd(su, sv) < sh_lim
        } else {
            false
        }
    };

    let u_body_lim = 3.95 * s;
    let v_body_lim = 2.7 * s;
    let u_sh_lim = 3.45 * s;
    let v_sh_lim = 2.2 * s;
    let inv_nu = 1.0 / (2.9 * s);
    let inv_nv = 1.0 / (1.8 * s);
    let du_sh = -(fr.f.x + fr.f.y);
    let dv_sh = -(fr.p.x + fr.p.y);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let in_body_box = u >= -u_body_lim && u <= u_body_lim && v.abs() <= v_body_lim;
            let d = if in_body_box { body_sd(u, v) } else { 10.0 };
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            if cover >= 1.0 {
                let nu = ((u + 0.1 * s) * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.85)).clamp(0.0, 1.0);
                let lit = fr.lit(nu, nv);
                let mut lum = 0.44 + 0.38 * dome + 0.56 * lit;
                if lit > 0.4 && dome > 0.42 {
                    lum += 0.26;
                }
                let mut col = shade3(shadow_c, mid_c, hi_c, lum);
                if u < 0.7 * s && v.abs() < 0.28 * s {
                    col = mix_rgb(col, seam_c, 0.48);
                } else if (u - 1.35 * s).abs() < 0.3 * s {
                    col = mix_rgb(col, seam_c, 0.58);
                }
                pb.plot(x, y, fr.apply_hit(col));
            } else {
                let dark = if d < rim_lim {
                    0.6
                } else {
                    let su = u + du_sh;
                    let sv = v + dv_sh;
                    if su >= -u_sh_lim
                        && su <= u_sh_lim
                        && sv.abs() <= v_sh_lim
                        && body_sh_hit(su, sv)
                    {
                        0.34
                    } else {
                        0.0
                    }
                };
                if cover > 0.0 || dark > 0.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let mut out = if dark > 0.0 {
                        mix_rgb(bg, rim_col, dark)
                    } else {
                        bg
                    };
                    if cover > 0.0 {
                        let nu = ((u + 0.1 * s) * inv_nu).clamp(-1.0, 1.0);
                        let nv = (v * inv_nv).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.85)).clamp(0.0, 1.0);
                        let lit = fr.lit(nu, nv);
                        let mut lum = 0.44 + 0.38 * dome + 0.56 * lit;
                        if lit > 0.4 && dome > 0.42 {
                            lum += 0.26;
                        }
                        let mut col = shade3(shadow_c, mid_c, hi_c, lum);
                        if u < 0.7 * s && v.abs() < 0.28 * s {
                            col = mix_rgb(col, seam_c, 0.48);
                        } else if (u - 1.35 * s).abs() < 0.3 * s {
                            col = mix_rgb(col, seam_c, 0.58);
                        }
                        out = mix_rgb(out, fr.apply_hit(col), cover);
                    }
                    pb.plot(x, y, out);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
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
    let s = fr.r * 0.5;
    let flap = fast_sin(fr.anim); // -1..1
    let spread = 0.5 + 0.5 * flap; // 0..1
    // Wide dramatic bat wingspan throughout the whole flap cycle.
    let span = (4.1 + 1.85 * spread) * s;
    let sweep = (-0.75 + 1.35 * flap) * s;

    // 1) Elevated ground shadow shifted down-right so the bat visibly hovers.
    let sh_ofs_x = (2.2 * s).round() as i32;
    let sh_ofs_y = (3.4 * s + 0.45 * flap * s).round() as i32;
    let sh_rx = (2.4 + 1.0 * spread) * s;
    let sh_ry = 1.45 * s;
    let inv_sh_rx = 1.0 / sh_rx;
    let inv_sh_ry = 1.0 / sh_ry;
    let scx = fr.pos.x.round() as i32 + sh_ofs_x;
    let scy = fr.pos.y.round() as i32 + sh_ofs_y;
    let srx = sh_rx.ceil() as i32;
    let sry = sh_ry.ceil() as i32;
    let sy0 = (scy - sry).max(0);
    let sy1 = (scy + sry).min(pb.h as i32 - 1);
    let sx0 = (scx - srx).max(0);
    let sx1 = (scx + srx).min(pb.w as i32 - 1);
    for y in sy0..=sy1 {
        let ny = (y - scy) as f32 * inv_sh_ry;
        let ny2 = ny * ny;
        for x in sx0..=sx1 {
            let nx = (x - scx) as f32 * inv_sh_rx;
            let r2 = nx * nx + ny2;
            if r2 <= 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let amt = 0.34 * (1.0 - r2 * 0.5);
                pb.plot(x, y, scale_rgb(bg, 1.0 - amt));
            }
        }
    }

    // 2) Classic M-arched scalloped bat wings + furry violet torso + pointed ears.
    let u_min = (-0.65 * s + sweep.min(0.0) - 2.55 * s).min(-2.1 * s) - 1.1 * s;
    let u_max = (1.65 * s + sweep.max(0.0)).max(3.2 * s) + 1.1 * s;
    let v_max = span + 1.3 * s;
    let circ_r = (span + 2.0 * s).max(6.4 * s);
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented_circ(pb, u_min, u_max, v_max, circ_r, (0, 0))
    else {
        return;
    };

    let body_sh = (54, 32, 92);
    let body_mid = (144, 96, 206);
    let body_hi = (218, 178, 255);
    let wing_sh = (46, 24, 84);
    let wing_mid = (114, 70, 178);
    let wing_bone = (204, 166, 252);
    let rim_col = if fr.hit > 0.4 { (140, 100, 180) } else { (0, 0, 0) };

    let torso_sdf = EllipseSdf::new(0.2 * s, 0.0, 2.0 * s, 1.3 * s);
    let ear_sdf = TaperSdf::new((1.45 * s, 0.65 * s), (2.9 * s, 1.35 * s), 0.5 * s, 0.16 * s);
    let wing_start = 1.05 * s;
    let inv_wing_span = 1.0 / (span - wing_start).max(0.5);
    let wing_av_min = 0.9 * s;
    let wing_av_max = span + 1.2 * s;
    let rim_lim = 0.85 * s;
    let inv_nu = 1.0 / (2.0 * s);
    let inv_nv = 1.0 / (1.35 * s);
    let bone_w = 0.58 * s;
    let bone_d = 0.35 * s;

    let lit_wing_pos = fr.lit(0.25, 0.65);
    let lit_wing_neg = fr.lit(0.25, -0.65);
    let wing_bone_pos = fr.apply_hit(mix_rgb(
        wing_mid,
        wing_bone,
        (0.82 + 0.25 * lit_wing_pos).clamp(0.0, 1.0),
    ));
    let wing_bone_neg = fr.apply_hit(mix_rgb(
        wing_mid,
        wing_bone,
        (0.82 + 0.25 * lit_wing_neg).clamp(0.0, 1.0),
    ));
    let wing_base_pos = shade3(wing_sh, wing_mid, wing_bone, 0.46 + 0.36 * lit_wing_pos);
    let wing_base_neg = shade3(wing_sh, wing_mid, wing_bone, 0.46 + 0.36 * lit_wing_neg);
    let wing_mem_pos = fr.apply_hit(wing_base_pos);
    let wing_mem_neg = fr.apply_hit(wing_base_neg);
    let wing_rib_pos = fr.apply_hit(scale_rgb(wing_base_pos, 0.76));
    let wing_rib_neg = fr.apply_hit(scale_rgb(wing_base_neg, 0.76));

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let av = v.abs();

            let d_body = if av < 2.6 * s && u > -2.85 * s && u < 4.15 * s {
                let torso = if av < 2.35 * s && u < 3.25 * s {
                    torso_sdf.eval(u, v)
                } else {
                    10.0
                };
                let ear = if u > 0.1 * s {
                    ear_sdf.eval(u, av)
                } else {
                    10.0
                };
                torso.min(ear)
            } else {
                10.0
            };

            let (d_wing, on_bone, t) = if av >= wing_av_min && av <= wing_av_max {
                let t = ((av - wing_start) * inv_wing_span).clamp(0.0, 1.0);
                let (elbow, chord) = bat_wing_profile(t);
                let u_lead = elbow * s + sweep * t;
                let u_trail = u_lead - chord * s;
                let du_top = u - u_lead;
                let du_bot = u_trail - u;
                let dv_tip = av - span;
                let d = du_top.max(du_bot).max(dv_tip);
                let on_bone = du_top.abs() < bone_w && d < bone_d;
                (d, on_bone, t)
            } else {
                (10.0, false, 0.0)
            };

            let d_all = d_body.min(d_wing);
            if d_all >= rim_lim {
                u += fr.f.x;
                v += fr.p.x;
                continue;
            }

            let body_cov = (0.5 - d_body * 1.2).clamp(0.0, 1.0);
            let wing_cov = (0.48 - d_wing * 1.15).clamp(0.0, 1.0);
            let cover = body_cov.max(wing_cov);

            if cover <= 0.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.54));
            } else if body_cov >= wing_cov {
                let nu = (u * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.5 + nv * nv * 0.7)).clamp(0.0, 1.0);
                let lum = 0.44 + 0.38 * dome + 0.56 * fr.lit(nu, nv);
                let col = fr.apply_hit(shade3(body_sh, body_mid, body_hi, lum));
                if body_cov >= 1.0 {
                    pb.plot(x, y, col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.54 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, col, body_cov));
                }
            } else {
                let pos_side = v >= 0.0;
                let col = if on_bone {
                    if pos_side {
                        wing_bone_pos
                    } else {
                        wing_bone_neg
                    }
                } else {
                    let rib = ((t * 2.5).fract() - 0.5).abs() < 0.1;
                    if rib {
                        if pos_side {
                            wing_rib_pos
                        } else {
                            wing_rib_neg
                        }
                    } else if pos_side {
                        wing_mem_pos
                    } else {
                        wing_mem_neg
                    }
                };
                let bg = pb.pixel_at(x as usize, y as usize);
                let out = if cover < 1.0 {
                    mix_rgb(bg, rim_col, 0.54 * (1.0 - cover))
                } else {
                    bg
                };
                pb.plot(x, y, mix_rgb(out, col, wing_cov * 0.92));
            }
            u += fr.f.x;
            v += fr.p.x;
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
    let s = fr.r * 0.25;
    let inv_s = 1.0 / s;
    let inv_flex_denom = 1.0 / (6.4 * s);
    let spine_freq = 0.48 * inv_s;

    let spine = |u: f32| -> f32 {
        let flex = ((2.0 * s - u) * inv_flex_denom).clamp(0.0, 1.0);
        if flex <= 0.0 {
            0.0
        } else {
            0.52 * s * flex * fast_sin(fr.anim - u * spine_freq)
        }
    };

    // 1) Bold curved feline S-hook tail (always curls gracefully, never straightens!).
    let vel_sq = vel.x * vel.x + vel.y * vel.y;
    let lag = if vel_sq > 4.0 {
        let inv_v = 1.0 / vel_sq.sqrt();
        let vdx = vel.x * inv_v;
        let vdy = vel.y * inv_v;
        let cross = fr.f.x * vdy - fr.f.y * vdx;
        let dot = fr.f.x * vdx + fr.f.y * vdy;
        (cross.atan2(dot) * 1.1).clamp(-1.4, 1.4)
    } else {
        0.0
    };
    let step = 7.4 * s / CAT_TAIL_STEPS as f32;
    let back_x = -fr.f.x;
    let back_y = -fr.f.y;
    let mut tail_pts = [(0i32, 0i32); CAT_TAIL_STEPS];
    let mut tp = fr.place(-4.0 * s, spine(-4.0 * s));
    for (i, slot) in tail_pts.iter_mut().enumerate() {
        let t = (i as f32 + 0.5) * (1.0 / CAT_TAIL_STEPS as f32);
        let curl = CAT_TAIL_CURL[i];
        let wave = 0.38 * t * fast_sin(fr.anim * 0.9 - t * 3.2);
        let theta = lag * t + curl + wave;
        let (st, ct) = fast_sin_cos(theta);
        tp.x += (back_x * ct - back_y * st) * step;
        tp.y += (back_x * st + back_y * ct) * step;
        *slot = (tp.x.round() as i32, tp.y.round() as i32);
    }
    let mut last_sh = (i32::MIN, i32::MIN);
    for &(tx, ty) in &tail_pts {
        if (tx, ty) != last_sh {
            darken_px(pb, tx + 1, ty + 1, 0.34);
            last_sh = (tx, ty);
        }
    }
    let side_dx = fr.p.x.round() as i32;
    let side_dy = fr.p.y.round() as i32;
    let tail_tip_c = fr.apply_hit((178, 190, 212));
    let tail_dark_c = fr.apply_hit((34, 40, 54));
    let tail_mid_c = fr.apply_hit((106, 118, 144));
    let tail_dark_edge = scale_rgb(tail_dark_c, 0.6);
    let tail_mid_edge = scale_rgb(tail_mid_c, 0.6);
    for (i, &(tx, ty)) in tail_pts.iter().enumerate() {
        let stripe = (i / 2) % 2 == 1;
        let c = if i >= CAT_TAIL_STEPS - 2 {
            tail_tip_c
        } else if stripe {
            tail_dark_c
        } else {
            tail_mid_c
        };
        if i < CAT_TAIL_STEPS - 4 {
            let ec = if stripe { tail_dark_edge } else { tail_mid_edge };
            blend_px(pb, tx + side_dx, ty + side_dy, ec, 0.72);
        }
        pb.plot(tx, ty, c);
    }

    // 2) Four stalking feline paws (diagonal stride + claw swipe on contact).
    let swiping = contact_cd > 0.32;
    let paw_col = fr.apply_hit((76, 86, 108));
    let toe_col = fr.apply_hit((168, 180, 202));
    let (s0, c0) = fast_sin_cos(fr.anim);
    let sp_front = spine(1.6 * s);
    let sp_back = spine(-2.5 * s);
    for (k, &(u0, side, sp_u)) in [
        (1.6f32, 1.0f32, sp_front),
        (-2.5, -1.0, sp_back),
        (1.6, -1.0, sp_front),
        (-2.5, 1.0, sp_back),
    ]
    .iter()
    .enumerate()
    {
        let (sph, cph) = if k < 2 { (s0, c0) } else { (-s0, -c0) };
        let swing = sph * 1.3 * s;
        let reach_u = u0 * s + swing + if swiping && u0 > 0.0 { 1.2 * s } else { 0.0 };
        let reach_v = side * (2.6 + 0.55 * cph.max(0.0)) * s + sp_u;
        let p0 = fr.px(u0 * s, side * 1.6 * s + sp_u);
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
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -5.8 * s, 5.9 * s, 4.9 * s, 1, 2) else {
        return;
    };

    let fur_sh = (38, 44, 62);
    let fur_mid = (104, 116, 142);
    let fur_hi = (182, 196, 222);
    let stripe_c = (26, 30, 44);
    let ear_in = fr.apply_hit((176, 112, 134));
    let ear_out = fr.apply_hit(mix_rgb(fur_sh, fur_mid, 0.42));
    let rim_col = if fr.hit > 0.4 { (145, 130, 120) } else { (0, 0, 0) };

    let haunch_sdf = EllipseSdf::new(-2.4 * s, 0.0, 2.2 * s, 2.5 * s);
    let chest_sdf = EllipseSdf::new(0.7 * s, 0.0, 1.95 * s, 2.2 * s);
    let neck_sdf = CapsuleSdf::new(1.25 * s, 0.0, 2.45 * s, 0.0, 1.15 * s);
    let head_sdf = EllipseSdf::new(3.25 * s, 0.0, 1.5 * s, 1.95 * s);
    let ear_sdf = TaperSdf::new((2.3 * s, 1.3 * s), (3.75 * s, 3.3 * s), 0.96 * s, 0.2 * s);
    let k_torso = 0.6 * s;
    let inv_k_torso = 1.0 / k_torso;
    let k_head = 0.28 * s;
    let inv_k_head = 1.0 / k_head;

    let body_sd_vs = |u: f32, vs: f32| -> (f32, bool) {
        let avs = vs.abs();
        let haunch = if u < 1.6 * s && avs < 4.5 * s {
            haunch_sdf.eval(u, vs)
        } else {
            10.0
        };
        let chest = if u > -3.1 * s && u < 4.5 * s && avs < 4.2 * s {
            chest_sdf.eval(u, vs)
        } else {
            10.0
        };
        let torso = smin_inv(haunch, chest, k_torso, inv_k_torso);
        let neck = if u > -0.8 * s && u < 4.5 * s && avs < 3.1 * s {
            neck_sdf.eval(u, vs)
        } else {
            10.0
        };
        let head = if u > 0.3 * s && avs < 3.8 * s {
            head_sdf.eval(u, vs)
        } else {
            10.0
        };
        let body = smin_inv(torso.min(neck), head, k_head, inv_k_head);
        let ear = if u > 0.6 * s && avs > 0.2 * s && avs < 4.8 * s {
            ear_sdf.eval(u, avs)
        } else {
            10.0
        };
        if ear < body {
            (ear, true)
        } else {
            (body, false)
        }
    };

    let rim_lim = 0.92 * s;
    let sh_lim = 0.45 * s;
    let haunch_sh_sq = haunch_sdf.r2_limit(sh_lim);
    let chest_sh_sq = chest_sdf.r2_limit(sh_lim);
    let head_sh_sq = head_sdf.r2_limit(sh_lim);
    let inv_nu = 1.0 / (4.4 * s);
    let inv_nv = 1.0 / (2.4 * s);
    let inv_dorsal = 1.0 / (2.5 * s);
    let du_sh = -(fr.f.x + 2.0 * fr.f.y);
    let dv_sh = -(fr.p.x + 2.0 * fr.p.y);
    let u_lim_lo = -5.6 * s;
    let u_lim_hi = 5.8 * s;
    let v_lim = 4.8 * s;
    let u_sh_lo = -5.2 * s;
    let u_sh_hi = 5.4 * s;
    let v_sh_lim = 4.4 * s;

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let in_body_box = u >= u_lim_lo && u <= u_lim_hi && v.abs() <= v_lim;
            let (d, is_ear, vs) = if in_body_box {
                let vs = v - spine(u);
                let (d, is_ear) = body_sd_vs(u, vs);
                (d, is_ear, vs)
            } else {
                (10.0, false, 0.0)
            };
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            if cover >= 1.0 {
                let col = if is_ear {
                    let avs = vs.abs();
                    if u > 2.8 * s && avs > 1.7 * s && avs < 2.95 * s {
                        ear_in
                    } else {
                        ear_out
                    }
                } else {
                    let nu = ((u - 0.3 * s) * inv_nu).clamp(-1.0, 1.0);
                    let nv = (vs * inv_nv).clamp(-1.0, 1.0);
                    let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.85)).clamp(0.0, 1.0);
                    let lum = 0.42 + 0.35 * dome + 0.58 * fr.lit(nu, nv);
                    let mut fur = shade3(fur_sh, fur_mid, fur_hi, lum);
                    let avs = vs.abs();
                    if u < 1.8 * s {
                        let band = fast_sin((u * inv_s) * 2.25);
                        let dorsal = (1.0 - avs * inv_dorsal).clamp(0.0, 1.0);
                        if band > 0.22 && dorsal > 0.18 {
                            fur = mix_rgb(fur, stripe_c, 0.74 * dorsal);
                        }
                        if avs < 0.38 * s {
                            fur = mix_rgb(fur, stripe_c, 0.42);
                        }
                    } else if u > 2.2 * s && u < 3.45 * s && avs < 0.75 * s {
                        fur = mix_rgb(fur, stripe_c, 0.55);
                    }
                    fr.apply_hit(fur)
                };
                pb.plot(x, y, col);
            } else {
                let dark = if d < rim_lim {
                    0.64
                } else {
                    let su = u + du_sh;
                    let sv = v + dv_sh;
                    if su >= u_sh_lo && su <= u_sh_hi && sv.abs() <= v_sh_lim {
                        let svs = sv - spine(su);
                        if haunch_sdf.within_sq(su, svs, haunch_sh_sq)
                            || chest_sdf.within_sq(su, svs, chest_sh_sq)
                            || head_sdf.within_sq(su, svs, head_sh_sq)
                            || body_sd_vs(su, svs).0 < sh_lim
                        {
                            0.36
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                };
                if cover > 0.0 || dark > 0.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let mut out = if dark > 0.0 {
                        mix_rgb(bg, rim_col, dark)
                    } else {
                        bg
                    };
                    if cover > 0.0 {
                        let col = if is_ear {
                            let avs = vs.abs();
                            if u > 2.8 * s && avs > 1.7 * s && avs < 2.95 * s {
                                ear_in
                            } else {
                                ear_out
                            }
                        } else {
                            let nu = ((u - 0.3 * s) * inv_nu).clamp(-1.0, 1.0);
                            let nv = (vs * inv_nv).clamp(-1.0, 1.0);
                            let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.85)).clamp(0.0, 1.0);
                            let lum = 0.42 + 0.35 * dome + 0.58 * fr.lit(nu, nv);
                            let mut fur = shade3(fur_sh, fur_mid, fur_hi, lum);
                            let avs = vs.abs();
                            if u < 1.8 * s {
                                let band = fast_sin((u * inv_s) * 2.25);
                                let dorsal = (1.0 - avs * inv_dorsal).clamp(0.0, 1.0);
                                if band > 0.22 && dorsal > 0.18 {
                                    fur = mix_rgb(fur, stripe_c, 0.74 * dorsal);
                                }
                                if avs < 0.38 * s {
                                    fur = mix_rgb(fur, stripe_c, 0.42);
                                }
                            } else if u > 2.2 * s && u < 3.45 * s && avs < 0.75 * s {
                                fur = mix_rgb(fur, stripe_c, 0.55);
                            }
                            fr.apply_hit(fur)
                        };
                        out = mix_rgb(out, col, cover);
                    }
                    pb.plot(x, y, out);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
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
    blend_px(pb, nose.0, nose.1, fr.apply_hit((176, 112, 130)), 0.92);
}

// ============================================================================
// 4. SPITTER — Venom Wasp / Acid-Sac Bug with Live Shooting Telegraph (r = 3.0)
// ============================================================================

fn draw_spitter(pb: &mut PixelBuffer, fr: &Frame, fire_cd: f32) {
    let s = fr.r * (1.0 / 3.0);
    let inv_s = 1.0 / s;

    let charge = if fire_cd <= 0.0 {
        0.82 + 0.18 * fast_sin(fr.anim * 2.0).abs()
    } else if fire_cd < 0.75 {
        ((0.75 - fire_cd) * (1.0 / 0.75)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let recoil = if fire_cd > 1.95 {
        ((fire_cd - 1.95) * 4.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let swell = 1.0 + 0.28 * charge - 0.14 * recoil;

    // 1) Slender dark-emerald legs.
    let leg_col = fr.apply_hit((52, 102, 42));
    for (i, &u_hip) in [0.95f32, -0.35, -1.6].iter().enumerate() {
        let phase = fr.anim + i as f32 * 2.1;
        let sw_base = fast_sin(phase) * 0.65 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw_base), (1.0, -sw_base)] {
            let hip = fr.px(uh, side * 1.15 * s);
            let foot = fr.px(uh + swing, side * 3.15 * s);
            line_blend(pb, hip, foot, leg_col, 0.85);
        }
    }

    // 2) Distinct hourglass wasp body: plump venom sac + pinched waist + thorax/head + curved mandibles.
    let u_min = (-1.9 - 1.9 * swell - 1.0) * s;
    let u_max = (4.2 + 0.3 * charge + 1.2) * s;
    let mand_tip_v = (0.45 + 1.15 * charge) * s;
    let v_max = (2.0 * swell * s + 1.0 * s).max(mand_tip_v + 1.3 * s);
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, u_min, u_max, v_max, 1, 1) else {
        return;
    };

    let ch_sh = (30, 68, 26);
    let ch_mid = (114, 214, 68);
    let ch_hi = (210, 255, 118);
    let stripe_c = (20, 44, 18);
    let venom_hot = mix_rgb(fr.accent, (242, 255, 138), 0.55);
    let mand_col = fr.apply_hit(mix_rgb((60, 118, 44), (195, 248, 108), 0.55 + 0.35 * charge));
    let rim_col = if fr.hit > 0.4 { (130, 160, 80) } else { (0, 0, 0) };

    let abd_ru = 1.9 * s * swell;
    let abd_rv = 2.0 * s * swell;
    let abd_sdf = EllipseSdf::new(-1.9 * s, 0.0, abd_ru, abd_rv);
    let thorax_sdf = EllipseSdf::new(0.85 * s, 0.0, 1.2 * s, 1.3 * s);
    let head_sdf = EllipseSdf::new(2.4 * s, 0.0, 0.95 * s, 1.25 * s);
    let waist_sdf = CapsuleSdf::new(-0.55 * s, 0.0, 0.35 * s, 0.0, 0.52 * s);
    let mand_sdf = TaperSdf::new(
        (2.85 * s, 0.95 * s),
        ((4.2 + 0.3 * charge) * s, mand_tip_v),
        0.48 * s,
        0.2 * s,
    );
    let k_front = 0.32 * s;
    let inv_k_front = 1.0 / k_front;

    let body_sd = |u: f32, v: f32| -> (f32, u8) {
        let av = v.abs();
        let abd = if u < (-1.9 + 1.9 * swell + 1.1) * s {
            abd_sdf.eval(u, v)
        } else {
            10.0
        };
        let thorax = if u > -1.5 * s && u < 3.4 * s && av < 2.6 * s {
            thorax_sdf.eval(u, v)
        } else {
            10.0
        };
        let head = if u > 0.2 * s && av < 2.6 * s {
            head_sdf.eval(u, v)
        } else {
            10.0
        };
        let front = smin_inv(thorax, head, k_front, inv_k_front);
        let waist = if u > -2.0 * s && u < 1.8 * s && av < 1.8 * s {
            waist_sdf.eval(u, v)
        } else {
            10.0
        };
        let mand = if u > 1.4 * s && av < mand_tip_v + 1.6 * s {
            mand_sdf.eval(u, av)
        } else {
            10.0
        };
        let body = abd.min(front).min(waist).min(mand);
        let part = if mand < abd && mand < front && mand < waist {
            2
        } else if abd < front && abd < waist {
            0
        } else {
            1
        };
        (body, part)
    };

    let rim_lim = 0.85 * s;
    let sh_lim = 0.35 * s;
    let abd_sh_sq = abd_sdf.r2_limit(sh_lim);
    let thx_sh_sq = thorax_sdf.r2_limit(sh_lim);
    let head_sh_sq = head_sdf.r2_limit(sh_lim);
    let inv_abd_ru = 1.0 / abd_ru;
    let inv_abd_rv = 1.0 / abd_rv;
    let inv_th_ru = 1.0 / (1.85 * s);
    let inv_th_rv = 1.0 / (1.3 * s);
    let du_sh = -(fr.f.x + fr.f.y);
    let dv_sh = -(fr.p.x + fr.p.y);
    let stripe_mix = 0.76 * (1.0 - 0.5 * charge);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let in_box = u >= u_min && u <= u_max && v.abs() <= v_max;
            let (d, part) = if in_box { body_sd(u, v) } else { (10.0, 0) };
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            if cover >= 1.0 {
                let col = if part == 2 {
                    mand_col
                } else {
                    let (nu, nv) = if part == 0 {
                        (
                            ((u + 1.9 * s) * inv_abd_ru).clamp(-1.0, 1.0),
                            (v * inv_abd_rv).clamp(-1.0, 1.0),
                        )
                    } else {
                        (
                            ((u - 1.4 * s) * inv_th_ru).clamp(-1.0, 1.0),
                            (v * inv_th_rv).clamp(-1.0, 1.0),
                        )
                    };
                    let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.72)).clamp(0.0, 1.0);
                    let lum = 0.44 + 0.38 * dome + 0.52 * fr.lit(nu, nv);
                    let mut c = shade3(ch_sh, ch_mid, ch_hi, lum);
                    if part == 0 {
                        let stripe = fast_sin((u * inv_s + 1.9) * 3.2) > 0.42;
                        if stripe {
                            c = mix_rgb(c, stripe_c, stripe_mix);
                        } else if charge > 0.05 {
                            c = mix_rgb(c, venom_hot, charge * (0.45 + 0.48 * dome));
                        }
                    }
                    fr.apply_hit(c)
                };
                pb.plot(x, y, col);
            } else {
                let dark = if d < rim_lim {
                    0.62
                } else {
                    let su = u + du_sh;
                    let sv = v + dv_sh;
                    if su >= u_min
                        && su <= u_max
                        && sv.abs() <= v_max
                        && (abd_sdf.within_sq(su, sv, abd_sh_sq)
                            || thorax_sdf.within_sq(su, sv, thx_sh_sq)
                            || head_sdf.within_sq(su, sv, head_sh_sq)
                            || body_sd(su, sv).0 < sh_lim)
                    {
                        0.34
                    } else {
                        0.0
                    }
                };
                if cover > 0.0 || dark > 0.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let mut out = if dark > 0.0 {
                        mix_rgb(bg, rim_col, dark)
                    } else {
                        bg
                    };
                    if cover > 0.0 {
                        let col = if part == 2 {
                            mand_col
                        } else {
                            let (nu, nv) = if part == 0 {
                                (
                                    ((u + 1.9 * s) * inv_abd_ru).clamp(-1.0, 1.0),
                                    (v * inv_abd_rv).clamp(-1.0, 1.0),
                                )
                            } else {
                                (
                                    ((u - 1.4 * s) * inv_th_ru).clamp(-1.0, 1.0),
                                    (v * inv_th_rv).clamp(-1.0, 1.0),
                                )
                            };
                            let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.72)).clamp(0.0, 1.0);
                            let lum = 0.44 + 0.38 * dome + 0.52 * fr.lit(nu, nv);
                            let mut c = shade3(ch_sh, ch_mid, ch_hi, lum);
                            if part == 0 {
                                let stripe = fast_sin((u * inv_s + 1.9) * 3.2) > 0.42;
                                if stripe {
                                    c = mix_rgb(c, stripe_c, stripe_mix);
                                } else if charge > 0.05 {
                                    c = mix_rgb(c, venom_hot, charge * (0.45 + 0.48 * dome));
                                }
                            }
                            fr.apply_hit(c)
                        };
                        out = mix_rgb(out, col, cover);
                    }
                    pb.plot(x, y, out);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Tapered gauzy buzzing wasp V-wings blended softly over thorax.
    let buzz = fast_sin(fr.anim * 2.6);
    let wing_reach = (3.4 + 1.0 * buzz.abs()) * s;
    let wing_col = mix_rgb((185, 240, 200), fr.accent, 0.45);
    let wu_min = (-2.3 + 0.45 * buzz.min(0.0) - 1.1) * s;
    let wu_max = (0.7 + 0.45 * buzz.max(0.0) + 1.1) * s;
    if let Some((wx0, wx1, wy0, wy1)) = fr.bbox_oriented(pb, wu_min, wu_max, wing_reach + 0.5, 0, 0)
    {
        let w_start = 0.9 * s;
        let inv_w_span = 1.0 / (wing_reach - w_start).max(0.5);
        let u_slope = (-3.0 + 0.45 * buzz) * s;
        let u_base = 0.7 * s;
        let max_ch = 0.95 * s;
        let edge_pad = 0.45 * s;
        for y in wy0..=wy1 {
            let (mut u, mut v) = fr.local(wx0, y);
            for x in wx0..=wx1 {
                let av = v.abs();
                if av >= w_start && av <= wing_reach {
                    let t = (av - w_start) * inv_w_span;
                    let u_center = u_base + u_slope * t;
                    let du = (u - u_center).abs();
                    if du <= max_ch {
                        let half_ch = max_ch * spitter_wing_profile(t);
                        if du <= half_ch {
                            let edge = du > half_ch - edge_pad;
                            let alpha = if edge { 0.46 } else { 0.26 } * (1.0 - t * 0.35);
                            let bg = pb.pixel_at(x as usize, y as usize);
                            pb.plot(x, y, mix_rgb(bg, wing_col, alpha));
                        }
                    }
                }
                u += fr.f.x;
                v += fr.p.x;
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
        let orb_col = mix_rgb(venom_hot, (255, 255, 225), charge * 0.65);
        pb.plot(mp.0, mp.1, orb_col);
        let glow = scale_rgb(orb_col, 0.45 + 0.55 * charge);
        if charge > 0.35 {
            pb.plot_add(mp.0 + 1, mp.1, glow);
            pb.plot_add(mp.0 - 1, mp.1, glow);
            pb.plot_add(mp.0, mp.1 + 1, glow);
            pb.plot_add(mp.0, mp.1 - 1, glow);
        }
        if charge > 0.7 {
            let tip = scale_rgb(orb_col, 0.6);
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
    let s = fr.r * (1.0 / 6.5);
    let stomp = fr.anim * 0.72;

    // 1) Elite threat perimeter: crisp 1px rotating royal-gold corner arcs.
    if elite {
        let pulse = 0.55 + 0.45 * fast_sin(fr.anim * 1.4);
        let aura_r = fr.r + 2.3 * s;
        let gold = scale_rgb((255, 206, 72), 0.52 + 0.35 * pulse);
        for i in 0..8 {
            let a0 = i as f32 * (TAU / 8.0) + fr.anim * 0.14;
            let mut last = (i32::MIN, i32::MIN);
            for step in 0..5 {
                let (sa, ca) = fast_sin_cos(a0 + step as f32 * 0.07);
                let tx = (fr.pos.x + ca * aura_r).round() as i32;
                let ty = (fr.pos.y + sa * aura_r).round() as i32;
                if (tx, ty) != last {
                    blend_px(pb, tx, ty, gold, 0.72);
                    last = (tx, ty);
                }
            }
        }
    }

    // 2) Six thick heavy-stomping armored legs with lit top-left knees.
    let leg_core = fr.apply_hit(if elite { (154, 98, 52) } else { (134, 54, 44) });
    let leg_edge = fr.apply_hit((46, 18, 22));
    let knee_hi = fr.apply_hit(if elite { (252, 212, 92) } else { (212, 98, 72) });
    for (i, &u_hip) in [2.6f32, -0.5, -3.6].iter().enumerate() {
        let phase = stomp + i as f32 * 2.1;
        let sw_base = fast_sin(phase) * 1.35 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw_base), (1.0, -sw_base)] {
            let hip = fr.px(uh, side * 4.2 * s);
            let knee = fr.px(uh + swing * 0.55, side * 7.0 * s);
            let foot = fr.px(uh + swing - 0.55 * s, side * 8.2 * s);
            thick_line(pb, hip, knee, leg_core, leg_edge);
            line_blend(pb, knee, foot, leg_edge, 0.95);
            pb.plot(knee.0, knee.1, knee_hi);
        }
    }

    // 3) Massive domed armored carapace + sculpted SDF rhino horn & cheek tusks!
    let u_min = -7.8 * s;
    let u_max = 10.3 * s;
    let v_max = 6.9 * s;
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, u_min, u_max, v_max, 2, 2) else {
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

    let elytra_sdf = EllipseSdf::new(-1.55 * s, 0.0, 5.1 * s, 5.65 * s);
    let pronotum_sdf = EllipseSdf::new(3.15 * s, 0.0, 2.75 * s, 4.45 * s);
    let main_horn_sdf = TaperSdf::new((4.4 * s, 0.0), (8.6 * s, 0.0), 1.85 * s, 0.45 * s);
    let fork_sdf = TaperSdf::new((7.2 * s, 0.35 * s), (8.9 * s, 1.35 * s), 0.55 * s, 0.22 * s);
    let side_horn_sdf = if elite {
        TaperSdf::new((3.6 * s, 2.8 * s), (6.8 * s, 5.0 * s), 0.95 * s, 0.28 * s)
    } else {
        TaperSdf::new((4.4 * s, 2.5 * s), (6.2 * s, 3.35 * s), 0.72 * s, 0.22 * s)
    };
    let k_shell = 0.95 * s;
    let inv_k_shell = 1.0 / k_shell;
    let k_comb = 0.4 * s;
    let inv_k_comb = 1.0 / k_comb;

    let shell_sd = |u: f32, v: f32| -> (f32, bool) {
        let av = v.abs();
        let elytra = if u < 5.6 * s {
            elytra_sdf.eval(u, v)
        } else {
            10.0
        };
        let pronotum = if u > -1.7 * s && u < 8.0 * s && av < 6.5 * s {
            pronotum_sdf.eval(u, v)
        } else {
            10.0
        };
        let shell = smin_inv(elytra, pronotum, k_shell, inv_k_shell);
        if u <= 1.2 * s {
            return (shell, false);
        }
        let main_horn = if av < 3.5 * s {
            main_horn_sdf.eval(u, v)
        } else {
            10.0
        };
        let fork = if u > 5.0 * s && av < 3.2 * s {
            fork_sdf.eval(u, av)
        } else {
            10.0
        };
        let side_horn = if u < 8.6 * s && av > 0.6 * s {
            side_horn_sdf.eval(u, av)
        } else {
            10.0
        };
        let horns = main_horn.min(fork).min(side_horn);
        let combined = smin_inv(shell, horns, k_comb, inv_k_comb);
        let is_horn = horns < shell + 0.1 * s && u > 4.6 * s;
        (combined, is_horn)
    };

    let damaged = fr.hp_frac < 0.68;
    let critical = fr.hp_frac < 0.38;
    let rim_lim = 1.0 * s;
    let sh_lim = 0.55 * s;
    let elytra_sh_sq = elytra_sdf.r2_limit(sh_lim);
    let pronotum_sh_sq = pronotum_sdf.r2_limit(sh_lim);
    let inv_h_nu = 1.0 / (2.5 * s);
    let inv_h_nv = 1.0 / (2.0 * s);
    let inv_h_tip = 1.0 / (3.4 * s);
    let inv_s_nu = 1.0 / (6.4 * s);
    let inv_s_nv = 1.0 / (5.7 * s);
    let du_sh = -2.0 * (fr.f.x + fr.f.y);
    let dv_sh = -2.0 * (fr.p.x + fr.p.y);

    let shade_brute = |u: f32, v: f32, d: f32, is_horn: bool| -> Rgb {
        let col = if is_horn {
            let nu = ((u - 6.4 * s) * inv_h_nu).clamp(-1.0, 1.0);
            let nv = (v * inv_h_nv).clamp(-1.0, 1.0);
            let lit = fr.lit(nu * 0.6, nv);
            let tip_t = ((u - 5.2 * s) * inv_h_tip).clamp(0.0, 1.0);
            let lum = 0.42 + 0.35 * tip_t + 0.55 * lit;
            shade3(horn_sh, horn_mid, horn_hi, lum)
        } else {
            let nu = ((u + 0.2 * s) * inv_s_nu).clamp(-1.0, 1.0);
            let nv = (v * inv_s_nv).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.55 + nv * nv * 0.72)).clamp(0.0, 1.0);
            let lit = fr.lit(nu, nv);
            let mut lum = 0.36 + 0.4 * dome + 0.62 * lit;
            if lit > 0.46 && dome > 0.32 {
                lum += 0.28;
            }
            let mut c = shade3(sh_c, mid_c, hi_c, lum);

            let collar = (u - 1.5 * s).abs();
            if collar < 0.55 * s {
                c = mix_rgb(c, seam_c, 0.72);
            } else if u > 1.5 * s && collar < 1.15 * s {
                c = mix_rgb(c, trim_c, if elite { 0.85 } else { 0.35 });
            }

            if u < 1.4 * s && v.abs() < 0.45 * s {
                c = if elite {
                    mix_rgb(trim_c, (255, 245, 180), (0.35 + 0.5 * lit).clamp(0.0, 1.0))
                } else {
                    mix_rgb(c, seam_c, 0.72)
                };
            }

            if d > -1.05 * s {
                if elite {
                    c = mix_rgb(c, trim_c, 0.65);
                } else if lit < -0.15 {
                    c = scale_rgb(c, 0.72);
                }
            }

            if u < 0.8 * s
                && ((u + 1.2 * s).abs() < 0.35 * s || (u + 3.8 * s).abs() < 0.35 * s)
            {
                c = scale_rgb(c, 0.8);
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
        fr.apply_hit(col)
    };

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let in_box = u >= u_min && u <= u_max && v.abs() <= v_max;
            let (d, is_horn) = if in_box { shell_sd(u, v) } else { (10.0, false) };
            let cover = (0.5 - d * 1.18).clamp(0.0, 1.0);

            if cover >= 1.0 {
                pb.plot(x, y, shade_brute(u, v, d, is_horn));
            } else {
                let dark = if d < rim_lim {
                    0.68
                } else {
                    let su = u + du_sh;
                    let sv = v + dv_sh;
                    if su >= u_min
                        && su <= u_max
                        && sv.abs() <= v_max
                        && (elytra_sdf.within_sq(su, sv, elytra_sh_sq)
                            || pronotum_sdf.within_sq(su, sv, pronotum_sh_sq)
                            || shell_sd(su, sv).0 < sh_lim)
                    {
                        0.42
                    } else {
                        0.0
                    }
                };
                if cover > 0.0 || dark > 0.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let mut out = if dark > 0.0 {
                        mix_rgb(bg, rim_col, dark)
                    } else {
                        bg
                    };
                    if cover > 0.0 {
                        out = mix_rgb(out, shade_brute(u, v, d, is_horn), cover);
                    }
                    pb.plot(x, y, out);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
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
    let eye_sub = scale_rgb(eye_col, 0.72);
    for side in [-1.0f32, 1.0] {
        let e0 = fr.px(4.7 * s, side * 2.15 * s);
        let e1 = fr.px(4.15 * s, side * 2.55 * s);
        pb.plot(e0.0, e0.1, eye_col);
        blend_px(pb, e1.0, e1.1, eye_sub, 0.9);
    }
}

// ============================================================================
// 6. BOSS — Extermination Queen / Apex Hornet-Scorpion Matriarch (r = 14.0)
// ============================================================================

fn draw_boss(pb: &mut PixelBuffer, fr: &Frame, fire_cd: f32) {
    let s = fr.r * (1.0 / 14.0);
    let inv_s = 1.0 / s;
    let charge = if fire_cd <= 0.0 {
        0.85 + 0.15 * fast_sin(fr.anim * 2.6).abs()
    } else if fire_cd < 0.50 {
        ((0.50 - fire_cd) * 2.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let enraged = fr.hp_frac < 0.45;
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 2.2);

    // Sculpted multi-part queen anatomy + curved scythe mandibles + crown horns + needle stinger!
    let mand_open = (1.8 + 3.0 * charge) * s;
    let mand_mid_v = (3.7 + 1.5 * charge) * s;
    let abd_sdf = EllipseSdf::new(-7.0 * s, 0.0, 6.4 * s, 7.9 * s);
    let stinger_sdf = TaperSdf::new((-12.0 * s, 0.0), (-16.0 * s, 0.0), 1.8 * s, 0.25 * s);
    let thorax_sdf = EllipseSdf::new(1.9 * s, 0.0, 5.4 * s, 8.2 * s);
    let head_sdf = EllipseSdf::new(9.2 * s, 0.0, 3.6 * s, 5.4 * s);
    let crown_sdf = TaperSdf::new((8.2 * s, 4.2 * s), (5.4 * s, 8.6 * s), 1.35 * s, 0.32 * s);
    let m_base_sdf = TaperSdf::new((11.0 * s, 3.1 * s), (13.8 * s, mand_mid_v), 1.15 * s, 0.65 * s);
    let m_blade_sdf = TaperSdf::new((13.8 * s, mand_mid_v), (16.1 * s, mand_open), 0.65 * s, 0.22 * s);
    let k1 = 1.0 * s;
    let inv_k1 = 1.0 / k1;
    let k_core = 0.9 * s;
    let inv_k_core = 1.0 / k_core;

    let body_sd = |u: f32, v: f32| -> (f32, u8) {
        let av = v.abs();
        let abd = if u < 1.8 * s && av < 10.5 * s {
            abd_sdf.eval(u, v)
        } else {
            20.0
        };
        let stinger = if u < -8.0 * s && av < 4.5 * s {
            stinger_sdf.eval(u, v)
        } else {
            20.0
        };
        let rear = smin_inv(abd, stinger, k1, inv_k1);

        let thorax = if u > -6.0 * s && u < 9.8 * s && av < 10.8 * s {
            thorax_sdf.eval(u, v)
        } else {
            20.0
        };
        let head = if u > 3.2 * s && av < 8.0 * s {
            head_sdf.eval(u, v)
        } else {
            20.0
        };
        let front = smin_inv(thorax, head, k1, inv_k1);
        let core = smin_inv(rear, front, k_core, inv_k_core);

        let crown = if u > 2.5 * s && u < 11.8 * s && av > 1.5 * s && av < 11.2 * s {
            crown_sdf.eval(u, av)
        } else {
            20.0
        };
        let mand = if u > 8.2 * s && av < mand_mid_v + 3.5 * s {
            m_base_sdf.eval(u, av).min(m_blade_sdf.eval(u, av))
        } else {
            20.0
        };
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
    let sx = (3.0 * s).round() as i32;
    let sy = (3.5 * s).round() as i32;
    let mut sh_fr = *fr;
    sh_fr.pos.x += sx as f32;
    sh_fr.pos.y += sy as f32;
    if let Some((x0, x1, y0, y1)) = sh_fr.bbox_oriented(pb, -17.2 * s, 17.5 * s, 10.2 * s, 0, 0) {
        let sh_lim = 0.6 * s;
        let inv_sh_fade = 1.0 / (1.5 * s);
        for y in y0..=y1 {
            let (mut su, mut sv) = fr.local(x0 - sx, y - sy);
            for x in x0..=x1 {
                if su >= -17.2 * s && su <= 17.5 * s && sv.abs() <= 10.2 * s {
                    // Fast zero-sqrt interior accept: anywhere inside abd/thorax/head has d <= 0 -> a = 0.44.
                    if abd_sdf.within_sq(su, sv, 1.0)
                        || thorax_sdf.within_sq(su, sv, 1.0)
                        || head_sdf.within_sq(su, sv, 1.0)
                    {
                        let bg = pb.pixel_at(x as usize, y as usize);
                        pb.plot(x, y, scale_rgb(bg, 1.0 - 0.44));
                    } else {
                        let (d, _) = body_sd(su, sv);
                        if d < sh_lim {
                            let a = (0.44 * (1.0 - (d * inv_sh_fade).clamp(0.0, 1.0))).max(0.18);
                            let bg = pb.pixel_at(x as usize, y as usize);
                            pb.plot(x, y, scale_rgb(bg, 1.0 - a));
                        }
                    }
                }
                su += fr.f.x;
                sv += fr.p.x;
            }
        }
    }

    // 2) Eight jointed armored legs (4 pairs) gripping outward.
    let leg_core = fr.apply_hit((116, 30, 66));
    let leg_dark = fr.apply_hit((36, 12, 26));
    let leg_joint = fr.apply_hit((244, 202, 130));
    for (i, &u_hip) in [5.4f32, 1.8, -2.2, -6.2].iter().enumerate() {
        let phase = fr.anim * 0.8 + i as f32 * 1.6;
        let sw_base = fast_sin(phase) * 2.1 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw_base), (1.0, -sw_base)] {
            let hip = fr.px(uh, side * 6.8 * s);
            let knee = fr.px(uh + swing * 0.5, side * 12.2 * s);
            let foot = fr.px(uh + swing - 1.4 * s, side * 15.0 * s);
            thick_line(pb, hip, knee, leg_core, leg_dark);
            line_blend(pb, knee, foot, leg_dark, 0.95);
            pb.plot(knee.0, knee.1, leg_joint);
        }
    }

    // 3) Sculpted royal carapace body.
    if let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -17.6 * s, 17.8 * s, 10.6 * s, 0, 0) {
        let sh_c = (56, 14, 36);
        let mid_c = (196, 38, 96);
        let hi_c = (255, 138, 188);
        let armor_sh = (42, 12, 32);
        let armor_mid = (128, 26, 68);
        let armor_hi = (206, 68, 124);
        let obsidian = (30, 12, 28);
        let gold_sh = (146, 98, 42);
        let gold_hi = (252, 224, 146);
        let hot_core = mix_rgb((255, 118, 182), (255, 248, 224), charge);
        let rim_col = if fr.hit > 0.4 { (185, 95, 130) } else { (0, 0, 0) };
        let fissure_col = mix_rgb(
            (255, 110, 56),
            (255, 240, 155),
            pulse * 0.6 + charge * 0.4,
        );
        let rim_lim = 1.2 * s;
        let inv_p0_ru = 1.0 / (6.8 * s);
        let inv_p0_rv = 1.0 / (8.0 * s);
        let inv_p1_ru = 1.0 / (5.6 * s);
        let inv_p1_rv = 1.0 / (8.2 * s);
        let inv_p2_ru = 1.0 / (4.0 * s);
        let inv_p2_rv = 1.0 / (5.5 * s);
        let inv_stinger = 1.0 / (3.6 * s);
        let inv_mand = 1.0 / (5.0 * s);
        let core_u = 2.2 * s;
        let core_r_max_sq = (2.5 * s * 0.96) * (2.5 * s * 0.96);
        let inv_core_r = 1.0 / (2.5 * s);
        let abd_glow = (charge * 0.65 + if enraged { 0.28 * pulse } else { 0.0 }).min(0.92);
        let core_intensity =
            (0.45 + 0.55 * charge + if enraged { 0.25 * pulse } else { 0.0 }).min(1.0);
        let has_fissures = fr.hp_frac < 0.55;

        for y in y0..=y1 {
            let (mut u, mut v) = fr.local(x0, y);
            for x in x0..=x1 {
                if u >= -17.6 * s && u <= 17.8 * s && v.abs() <= 10.6 * s {
                    let (d, part) = body_sd(u, v);
                    if d < rim_lim {
                        let cover = (0.55 - d * 1.18).clamp(0.0, 1.0);
                        if cover <= 0.0 {
                            let bg = pb.pixel_at(x as usize, y as usize);
                            pb.plot(x, y, mix_rgb(bg, rim_col, 0.72));
                        } else {
                            let (nu, nv) = match part {
                                0 => (
                                    ((u + 7.0 * s) * inv_p0_ru).clamp(-1.0, 1.0),
                                    (v * inv_p0_rv).clamp(-1.0, 1.0),
                                ),
                                1 => (
                                    ((u - 1.9 * s) * inv_p1_ru).clamp(-1.0, 1.0),
                                    (v * inv_p1_rv).clamp(-1.0, 1.0),
                                ),
                                _ => (
                                    ((u - 9.5 * s) * inv_p2_ru).clamp(-1.0, 1.0),
                                    (v * inv_p2_rv).clamp(-1.0, 1.0),
                                ),
                            };
                            let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.72)).clamp(0.0, 1.0);
                            let lit = fr.lit(nu, nv);
                            let mut lum = 0.36 + 0.4 * dome + 0.62 * lit;
                            if lit > 0.45 && dome > 0.35 {
                                lum += 0.24;
                            }
                            let av = v.abs();

                            let mut col = match part {
                                0 => {
                                    let mut c = shade3(sh_c, mid_c, hi_c, lum);
                                    if u < -12.4 * s {
                                        let tip_t = ((-12.4 * s - u) * inv_stinger).clamp(0.0, 1.0);
                                        c = mix_rgb(gold_sh, hot_core, 0.35 + 0.65 * tip_t);
                                    } else {
                                        let band = fast_sin((u * inv_s + 7.0) * 1.6);
                                        if band > 0.18 {
                                            c = mix_rgb(c, obsidian, 0.8);
                                        } else if charge > 0.1 || enraged {
                                            c = mix_rgb(c, hot_core, abd_glow * dome);
                                        }
                                        if av < 0.7 * s {
                                            c = mix_rgb(c, gold_hi, 0.55);
                                        }
                                    }
                                    c
                                }
                                1 => {
                                    let mut c = shade3(armor_sh, armor_mid, armor_hi, lum);
                                    let chev = ((u * inv_s - 2.2) + av * 0.42 * inv_s).rem_euclid(2.8);
                                    if chev < 0.48 {
                                        c = mix_rgb(c, obsidian, 0.82);
                                    } else if chev < 0.85 {
                                        c = mix_rgb(
                                            c,
                                            gold_hi,
                                            0.35 * (0.5 + 0.5 * lit).clamp(0.0, 1.0),
                                        );
                                    }
                                    if d > -1.4 * s && av > 3.6 * s {
                                        c = shade3(
                                            gold_sh,
                                            (226, 178, 92),
                                            gold_hi,
                                            0.45 + 0.55 * lit,
                                        );
                                    }
                                    let du_c = u - core_u;
                                    let r2_core = du_c * du_c + v * v;
                                    if r2_core < core_r_max_sq {
                                        let r_core = r2_core.sqrt() * inv_core_r;
                                        if r_core < 0.72 {
                                            c = mix_rgb(
                                                (185, 32, 88),
                                                hot_core,
                                                (1.0 - r_core * (1.0 / 0.72)) * core_intensity,
                                            );
                                        } else {
                                            c = mix_rgb(c, obsidian, 0.75);
                                        }
                                    }
                                    c
                                }
                                2 => {
                                    let mut c = shade3(armor_sh, mid_c, hi_c, lum);
                                    if av < 0.95 * s && u > 7.2 * s && u < 11.8 * s {
                                        c = shade3(
                                            gold_sh,
                                            (226, 178, 92),
                                            gold_hi,
                                            0.5 + 0.5 * lit,
                                        );
                                    }
                                    c
                                }
                                3 => shade3(gold_sh, (226, 178, 92), gold_hi, 0.45 + 0.55 * lit),
                                _ => {
                                    let tip_t = ((u - 11.2 * s) * inv_mand).clamp(0.0, 1.0);
                                    shade3(obsidian, (92, 46, 72), gold_hi, 0.25 + 0.75 * tip_t)
                                }
                            };

                            if has_fissures && part <= 1 {
                                let f1 = ((u + 5.5 * s) - (v - 2.2 * s) * 0.8).abs() < 0.48 * s
                                    && u < -2.5 * s
                                    && v > 0.8 * s;
                                let f2 = ((u - 1.5 * s) + (v + 3.0 * s) * 0.75).abs() < 0.48 * s
                                    && u > -s
                                    && u < 5.5 * s
                                    && v < -s;
                                if f1 || f2 {
                                    col = fissure_col;
                                }
                            }

                            let lit_col = fr.apply_hit(col);
                            if cover >= 1.0 {
                                pb.plot(x, y, lit_col);
                            } else {
                                let bg = pb.pixel_at(x as usize, y as usize);
                                let out = mix_rgb(bg, rim_col, 0.72 * (1.0 - cover));
                                pb.plot(x, y, mix_rgb(out, lit_col, cover));
                            }
                        }
                    }
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }

    // 4) Four blazing predator eyes with bright inner specular gleam.
    let eye_main = if fr.hit > 0.7 {
        (30, 14, 24)
    } else {
        mix_rgb((255, 236, 86), (255, 255, 235), charge * 0.5)
    };
    let eye_main_dim = scale_rgb(eye_main, 0.88);
    let eye_sub = if fr.hit > 0.7 {
        (30, 14, 24)
    } else {
        (255, 122, 62)
    };
    for side in [-1.0f32, 1.0] {
        let e0 = fr.px(10.3 * s, side * 2.35 * s);
        let e1 = fr.px(9.8 * s, side * 3.0 * s);
        pb.plot(e0.0, e0.1, eye_main);
        pb.plot(e1.0, e1.1, eye_main_dim);
        let eo = fr.px(8.5 * s, side * 4.35 * s);
        pb.plot(eo.0, eo.1, eye_sub);
    }

    // 5) Four smoothly curved tapered hornet wings (tucked cleanly inside the HP arc!).
    let flap = fast_sin(fr.anim * 1.8);
    let wing_span = (13.8 + 2.1 * flap.abs()) * s;
    let wing_tint = mix_rgb((235, 148, 212), fr.accent, 0.42);
    let wing_spar_col = apply_hit(wing_tint, fr.hit * 0.45);
    let vein_col = mix_rgb((78, 28, 66), wing_tint, 0.32);
    let wu_min = (-8.6 + 1.3 * flap.min(0.0) - 3.0) * s;
    let wu_max = (3.4 + 1.3 * flap.max(0.0) + 3.0) * s;
    if let Some((x0, x1, y0, y1)) = fr.bbox_oriented_circ(
        pb,
        wu_min,
        wu_max,
        wing_span + 0.5,
        wing_span + 1.5 * s,
        (0, 0),
    ) {
        let w_start = 4.4 * s;
        let inv_span = 1.0 / (wing_span - w_start).max(1.0);
        let min_half = 0.35 * s;
        let spar_w = 0.85 * s;
        for y in y0..=y1 {
            let (mut u, mut v) = fr.local(x0, y);
            for x in x0..=x1 {
                let av = v.abs();
                if av >= w_start && av <= wing_span && u >= wu_min && u <= wu_max {
                    let t = (av - w_start) * inv_span;

                    let fore_env = boss_wing_fore_env(t);
                    let fore_mid = (2.8 - 4.6 * t - 2.9 * t * t + 1.3 * flap * t) * s;
                    let fore_half = 2.9 * s * fore_env;
                    let in_fore = fore_half > min_half && (u - fore_mid).abs() <= fore_half;

                    let th = t * (1.0 / 0.72);
                    let in_hind = if !in_fore && th < 1.0 {
                        let hind_env = boss_wing_hind_env(th);
                        let hind_mid = (-1.8 - 4.2 * th - 2.4 * th * th + 0.85 * flap * th) * s;
                        let hind_half = 2.2 * s * hind_env;
                        hind_half > min_half && (u - hind_mid).abs() <= hind_half
                    } else {
                        false
                    };

                    if in_fore || in_hind {
                        let leading_spar = in_fore && (fore_mid + fore_half - u) < spar_w;
                        let cross_vein = ((t * 4.5).fract() < 0.13)
                            || ((u * inv_s + 10.0).rem_euclid(3.4) < 0.34);
                        let edge_fade = (1.0 - t * 0.42).clamp(0.28, 1.0);
                        let bg = pb.pixel_at(x as usize, y as usize);
                        let out = if leading_spar {
                            mix_rgb(bg, wing_spar_col, 0.66 * edge_fade)
                        } else if cross_vein {
                            mix_rgb(bg, vein_col, 0.38 * edge_fade)
                        } else {
                            mix_rgb(bg, wing_tint, 0.22 * edge_fade)
                        };
                        pb.plot(x, y, out);
                    }
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }

    // Tiny glowing venom bead at the rear stinger tip.
    let tip = fr.px(-16.1 * s, 0.0);
    let bead = mix_rgb((255, 110, 175), (255, 250, 220), 0.35 + 0.65 * charge);
    blend_px(pb, tip.0, tip.1, bead, 0.9);

    // 6) 10-bullet nova telegraph pips aligned to exact firing angles (k/10 * TAU).
    if charge > 0.08 {
        let pip_r = fr.r + (1.6 + (1.0 - charge) * 2.8) * s;
        let pip_col = mix_rgb((255, 100, 165), (255, 250, 230), charge);
        let glow = scale_rgb(pip_col, 0.55 * charge);
        let do_glow = charge > 0.5;
        for &(ca, sa) in &NOVA_10 {
            let px = (fr.pos.x + ca * pip_r).round() as i32;
            let py = (fr.pos.y + sa * pip_r).round() as i32;
            pb.plot(px, py, pip_col);
            if do_glow {
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
    let outer_r = arc_r + 1.0;
    let steps = (arc_r * 7.0).round() as i32;
    let inv_steps = 1.0 / steps as f32;
    let hp_col = if fr.hp_frac > 0.5 {
        mix_rgb((255, 165, 62), (255, 72, 136), (fr.hp_frac - 0.5) * 2.0)
    } else if fr.hp_frac > 0.25 {
        mix_rgb((255, 65, 58), (255, 165, 62), (fr.hp_frac - 0.25) * 4.0)
    } else {
        mix_rgb((190, 28, 36), (255, 86, 76), pulse)
    };
    let track_col = (18, 12, 18);
    let mut last = (i32::MIN, i32::MIN);

    for i in 0..steps {
        let t = i as f32 * inv_steps;
        let near_tick = (t - 0.25).abs() < 0.012
            || (t - 0.50).abs() < 0.012
            || (t - 0.75).abs() < 0.012;
        let a = -FRAC_PI_2 + t * TAU;
        let (sa, ca) = fast_sin_cos(a);
        let x = (fr.pos.x + ca * arc_r).round() as i32;
        let y = (fr.pos.y + sa * arc_r).round() as i32;
        if (x, y) == last {
            continue;
        }
        last = (x, y);
        if near_tick {
            blend_px(pb, x, y, (10, 8, 12), 0.8);
        } else if t <= fr.hp_frac {
            let ox = (fr.pos.x + ca * outer_r).round() as i32;
            let oy = (fr.pos.y + sa * outer_r).round() as i32;
            darken_px(pb, ox, oy, 0.45);
            blend_px(pb, x, y, hp_col, 0.94);
        } else {
            blend_px(pb, x, y, track_col, 0.52);
        }
    }
}

// ============================================================================
// LEVEL-OF-DETAIL (CROWD / SMALL-RADIUS) PATH
//
// Zero per-pixel `sqrt` or trig, pre-baked hit-flashed 4-stop directional ramps,
// quadratic ellipse coverage + crisp 1px silhouette rim, clear facing markers,
// and intact ranged/elite telegraphs.
// ============================================================================

#[inline(always)]
fn build_ramp4(sh: Rgb, mid: Rgb, hi: Rgb, fr: &Frame) -> [Rgb; 4] {
    [
        fr.apply_hit(sh),
        fr.apply_hit(mix_rgb(sh, mid, 0.58)),
        fr.apply_hit(mid),
        fr.apply_hit(mix_rgb(mid, hi, 0.68)),
    ]
}

#[inline(always)]
fn ramp4_idx(lit: f32, r2: f32) -> usize {
    let lum = 0.52 + 0.30 * (1.0 - r2).max(0.0) + 0.58 * lit;
    (lum * 3.1).clamp(0.0, 3.0) as usize
}

#[inline(always)]
fn plot_rim(pb: &mut PixelBuffer, x: i32, y: i32, rim_col: Rgb, dark: f32, hit_tint: bool) {
    let bg = pb.pixel_at(x as usize, y as usize);
    let out = if hit_tint {
        mix_rgb(bg, rim_col, dark)
    } else {
        scale_rgb(bg, 1.0 - dark)
    };
    pb.plot(x, y, out);
}

fn draw_skitterer_lod(pb: &mut PixelBuffer, fr: &Frame) {
    let s = fr.r * (1.0 / 2.5);
    let leg_col = fr.apply_hit((144, 86, 46));
    let foot_col = fr.apply_hit((98, 56, 30));
    let ant_col = fr.apply_hit((184, 122, 68));

    // Compact 6-leg dots (knee + foot) and 2 sweeping antennae for facing.
    for (i, &u_hip) in [1.1f32, -0.35, -1.75].iter().enumerate() {
        let sw = fast_sin(fr.anim + i as f32 * 2.1) * 0.85 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw), (1.0, -sw)] {
            let knee = fr.px(uh + swing * 0.45, side * 2.35 * s);
            let foot = fr.px(uh + swing - 0.3 * s, side * 3.15 * s);
            pb.plot(knee.0, knee.1, leg_col);
            blend_px(pb, foot.0, foot.1, foot_col, 0.82);
        }
    }
    let wave = fast_sin(fr.anim * 0.7) * 0.42 * s;
    for side in [-1.0f32, 1.0] {
        let a1 = fr.px(3.35 * s, side * 1.15 * s + wave * side * 0.5);
        let a2 = fr.px(4.25 * s, side * 1.9 * s + wave * side);
        blend_px(pb, a1.0, a1.1, ant_col, 0.82);
        blend_px(pb, a2.0, a2.1, ant_col, 0.56);
    }

    let Some((x0, x1, y0, y1)) =
        fr.bbox_oriented_circ(pb, -3.9 * s, 3.9 * s, 2.6 * s, 4.2 * s, (1, 1))
    else {
        return;
    };
    let ramp = build_ramp4((72, 36, 18), (196, 118, 52), (252, 206, 126), fr);
    let seam_col = fr.apply_hit((64, 30, 16));
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (160, 110, 60) } else { (0, 0, 0) };
    let inv_au = 1.0 / (2.3 * s);
    let inv_av = 1.0 / (1.75 * s);
    let inv_hu = 1.0 / (1.42 * s);
    let inv_hv = 1.0 / (1.22 * s);
    let seam_u = 1.35 * s;
    let seam_w = 0.32 * s;

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let anu = (u + 0.72 * s) * inv_au;
            let anv = v * inv_av;
            let r2_a = anu * anu + anv * anv;
            let hnu = (u - 1.55 * s) * inv_hu;
            let hnv = v * inv_hv;
            let r2_h = hnu * hnu + hnv * hnv;
            let r2 = r2_a.min(r2_h);
            if r2 <= 1.0 {
                let col = if (u - seam_u).abs() < seam_w || (u < 0.7 * s && v.abs() < 0.28 * s) {
                    seam_col
                } else {
                    let (nu, nv) = if r2_h < r2_a { (hnu, hnv) } else { (anu, anv) };
                    ramp[ramp4_idx(fr.lit(nu, nv), r2)]
                };
                pb.plot(x, y, col);
            } else if r2 <= 1.95 {
                plot_rim(pb, x, y, rim_col, 0.56, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    if fr.r >= 2.0 {
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

fn draw_bat_lod(pb: &mut PixelBuffer, fr: &Frame) {
    let s = fr.r * 0.5;
    let flap = fast_sin(fr.anim);
    let spread = 0.5 + 0.5 * flap;
    let span = (4.1 + 1.85 * spread) * s;
    let sweep = (-0.75 + 1.35 * flap) * s;

    // Compact elevated hover shadow matching full-detail footprint.
    let scx = fr.pos.x.round() as i32 + (2.2 * s).round() as i32;
    let scy = fr.pos.y.round() as i32 + (3.4 * s + 0.45 * flap * s).round() as i32;
    for dy in -1..=1 {
        for dx in -2..=2 {
            if dx * dx + dy * dy * 2 <= 5 {
                darken_px(pb, scx + dx, scy + dy, 0.30);
            }
        }
    }

    let u_min = (-0.65 * s + sweep.min(0.0) - 2.55 * s).min(-2.1 * s) - 0.9 * s;
    let u_max = (1.65 * s + sweep.max(0.0)).max(3.1 * s) + 0.9 * s;
    let v_max = span + 1.0 * s;
    let circ_r = (span + 1.8 * s).max(6.2 * s);
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented_circ(pb, u_min, u_max, v_max, circ_r, (0, 0))
    else {
        return;
    };

    let ramp = build_ramp4((54, 32, 92), (144, 96, 206), (218, 178, 255), fr);
    let wing_sh = (46, 24, 84);
    let wing_mid = (114, 70, 178);
    let wing_bone = (204, 166, 252);
    let lit_pos = fr.lit(0.25, 0.65);
    let lit_neg = fr.lit(0.25, -0.65);
    let wing_mem_pos = fr.apply_hit(shade3(wing_sh, wing_mid, wing_bone, 0.44 + 0.34 * lit_pos));
    let wing_mem_neg = fr.apply_hit(shade3(wing_sh, wing_mid, wing_bone, 0.44 + 0.34 * lit_neg));
    let wing_bone_pos = fr.apply_hit(mix_rgb(
        wing_mid,
        wing_bone,
        (0.76 + 0.24 * lit_pos).clamp(0.0, 1.0),
    ));
    let wing_bone_neg = fr.apply_hit(mix_rgb(
        wing_mid,
        wing_bone,
        (0.76 + 0.24 * lit_neg).clamp(0.0, 1.0),
    ));
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (140, 100, 180) } else { (0, 0, 0) };
    let inv_ru = 1.0 / (2.0 * s);
    let inv_rv = 1.0 / (1.3 * s);
    let wing_start = 1.05 * s;
    let inv_wing_span = 1.0 / (span - wing_start).max(0.5);
    let wing_av_min = 0.9 * s;
    let wing_av_max = span + 0.9 * s;
    let bone_w = 0.50 * s;
    let rim_w = 0.80 * s;

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let av = v.abs();
            let nu = (u - 0.2 * s) * inv_ru;
            let nv = v * inv_rv;
            let r2 = nu * nu + nv * nv;
            if r2 <= 1.0 {
                pb.plot(x, y, ramp[ramp4_idx(fr.lit(nu, nv), r2)]);
            } else if av >= wing_av_min && av <= wing_av_max {
                let t = ((av - wing_start) * inv_wing_span).clamp(0.0, 1.0);
                let (elbow, chord) = bat_wing_profile(t);
                let u_lead = elbow * s + sweep * t;
                let u_trail = u_lead - chord * s;
                let d_wing = (u - u_lead).max(u_trail - u).max(av - span);
                if d_wing <= 0.16 * s {
                    let pos_side = v >= 0.0;
                    let col = if (u - u_lead).abs() < bone_w {
                        if pos_side {
                            wing_bone_pos
                        } else {
                            wing_bone_neg
                        }
                    } else if pos_side {
                        wing_mem_pos
                    } else {
                        wing_mem_neg
                    };
                    let bg = pb.pixel_at(x as usize, y as usize);
                    pb.plot(x, y, mix_rgb(bg, col, 0.88));
                } else if d_wing < rim_w {
                    plot_rim(pb, x, y, rim_col, 0.52, hit_tint);
                }
            } else if r2 <= 1.85 {
                plot_rim(pb, x, y, rim_col, 0.52, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // Pointed bat ear tips + golden eyes.
    let ear_col = ramp[2];
    let eye_col = if fr.hit > 0.7 {
        (30, 18, 45)
    } else {
        (255, 222, 74)
    };
    for side in [-1.0f32, 1.0] {
        let ear0 = fr.px(2.05 * s, side * 0.95 * s);
        let ear1 = fr.px(2.75 * s, side * 1.25 * s);
        pb.plot(ear0.0, ear0.1, ear_col);
        blend_px(pb, ear1.0, ear1.1, ear_col, 0.82);
        let ep = fr.px(1.65 * s, side * 0.65 * s);
        pb.plot(ep.0, ep.1, eye_col);
    }
}

fn draw_cat_lod(pb: &mut PixelBuffer, fr: &Frame, contact_cd: f32) {
    let s = fr.r * 0.25;
    let inv_s = 1.0 / s;

    // 1) Compact 8-segment curved S-hook tail with silver tip.
    let step = 7.4 * s / 8.0;
    let back_x = -fr.f.x;
    let back_y = -fr.f.y;
    let mut tp = fr.place(-4.0 * s, 0.0);
    let tail_tip_c = fr.apply_hit((178, 190, 212));
    let tail_dark_c = fr.apply_hit((34, 40, 54));
    let tail_mid_c = fr.apply_hit((106, 118, 144));
    for i in 0..8 {
        let t = (i as f32 + 0.5) * 0.125;
        let curl = CAT_TAIL_CURL[i * 2];
        let wave = 0.35 * t * fast_sin(fr.anim * 0.9 - t * 3.2);
        let (st, ct) = fast_sin_cos(curl + wave);
        tp.x += (back_x * ct - back_y * st) * step;
        tp.y += (back_x * st + back_y * ct) * step;
        let tx = tp.x.round() as i32;
        let ty = tp.y.round() as i32;
        let c = if i >= 7 {
            tail_tip_c
        } else if i % 2 == 1 {
            tail_dark_c
        } else {
            tail_mid_c
        };
        pb.plot(tx, ty, c);
    }

    // 2) Four paw dots + swipe claw.
    let swiping = contact_cd > 0.32;
    let paw_col = fr.apply_hit((76, 86, 108));
    let toe_col = fr.apply_hit((168, 180, 202));
    let s0 = fast_sin(fr.anim);
    for (k, &(u0, side)) in [(1.6f32, 1.0f32), (-2.5, -1.0), (1.6, -1.0), (-2.5, 1.0)]
        .iter()
        .enumerate()
    {
        let swing = (if k < 2 { s0 } else { -s0 }) * 1.3 * s;
        let ru = u0 * s + swing + if swiping && u0 > 0.0 { 1.2 * s } else { 0.0 };
        let rv = side * 2.75 * s;
        let p0 = fr.px(u0 * s, side * 1.9 * s);
        let p1 = fr.px(ru, rv);
        pb.plot(p0.0, p0.1, paw_col);
        pb.plot(p1.0, p1.1, toe_col);
        if swiping && u0 > 0.0 {
            let claw = fr.px(ru + 1.1 * s, rv + side * 0.3 * s);
            pb.plot(claw.0, claw.1, (240, 245, 255));
        }
    }

    // 3) Quadratic torso + head + triangular cat ears.
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -5.4 * s, 5.6 * s, 4.4 * s, 1, 1) else {
        return;
    };
    let ramp = build_ramp4((38, 44, 62), (104, 116, 142), (182, 196, 222), fr);
    let stripe_col = fr.apply_hit((30, 35, 50));
    let ear_in = fr.apply_hit((176, 112, 134));
    let ear_out = fr.apply_hit((65, 74, 95));
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (145, 130, 120) } else { (0, 0, 0) };

    let inv_tu = 1.0 / (3.65 * s);
    let inv_tv = 1.0 / (2.35 * s);
    let inv_hu = 1.0 / (1.55 * s);
    let inv_hv = 1.0 / (1.98 * s);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let av = v.abs();
            let tnu = (u + 0.9 * s) * inv_tu;
            let tnv = v * inv_tv;
            let r2_t = tnu * tnu + tnv * tnv;
            let hnu = (u - 3.25 * s) * inv_hu;
            let hnv = v * inv_hv;
            let r2_h = hnu * hnu + hnv * hnv;
            let r2 = r2_t.min(r2_h);

            // Pointed triangular ears: u in [2.0s, 3.85s], av in [1.3s, 3.35s].
            let in_ear = if u > 1.9 * s && u < 3.9 * s && av > 1.25 * s && av < 3.4 * s {
                let ear_t = ((av - 1.3 * s) * (1.0 / (2.1 * s))).clamp(0.0, 1.0);
                let u_center = (2.35 + 1.4 * ear_t) * s;
                let half_w = (0.98 - 0.78 * ear_t) * s;
                (u - u_center).abs() <= half_w
            } else {
                false
            };

            if r2 <= 1.0 {
                let (nu, nv) = if r2_h < r2_t { (hnu, hnv) } else { (tnu, tnv) };
                let mut col = ramp[ramp4_idx(fr.lit(nu, nv), r2)];
                if u < 1.8 * s {
                    if av < 0.38 * s || fast_sin((u * inv_s) * 2.25) > 0.25 {
                        col = stripe_col;
                    }
                } else if u > 2.2 * s && u < 3.45 * s && av < 0.75 * s {
                    col = stripe_col;
                }
                pb.plot(x, y, col);
            } else if in_ear {
                let inner = u > 2.8 * s && av > 1.7 * s && av < 2.95 * s;
                pb.plot(x, y, if inner { ear_in } else { ear_out });
            } else if r2 <= 1.65 {
                plot_rim(pb, x, y, rim_col, 0.58, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // Glowing yellow-green cat eyes + pink nose.
    let eye_col = if fr.hit > 0.7 {
        (24, 28, 22)
    } else {
        (232, 255, 68)
    };
    for side in [-1.0f32, 1.0] {
        let ep = fr.px(3.75 * s, side * 0.9 * s);
        pb.plot(ep.0, ep.1, eye_col);
    }
    let nose = fr.px(4.65 * s, 0.0);
    blend_px(pb, nose.0, nose.1, fr.apply_hit((176, 112, 130)), 0.92);
}

fn draw_spitter_lod(pb: &mut PixelBuffer, fr: &Frame, fire_cd: f32) {
    let s = fr.r * (1.0 / 3.0);
    let inv_s = 1.0 / s;

    let charge = if fire_cd <= 0.0 {
        0.82 + 0.18 * fast_sin(fr.anim * 2.0).abs()
    } else if fire_cd < 0.75 {
        ((0.75 - fire_cd) * (1.0 / 0.75)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let recoil = if fire_cd > 1.95 {
        ((fire_cd - 1.95) * 4.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let swell = 1.0 + 0.28 * charge - 0.14 * recoil;

    // 1) Compact leg dots.
    let leg_col = fr.apply_hit((52, 102, 42));
    for (i, &u_hip) in [0.95f32, -0.35, -1.6].iter().enumerate() {
        let sw = fast_sin(fr.anim + i as f32 * 2.1) * 0.65 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw), (1.0, -sw)] {
            let foot = fr.px(uh + swing, side * 2.95 * s);
            blend_px(pb, foot.0, foot.1, leg_col, 0.85);
        }
    }

    // 2) Quadratic hourglass body (abdomen venom sac + thorax/head).
    let u_min = (-1.9 - 1.9 * swell - 0.9) * s;
    let u_max = (4.2 + 0.3 * charge + 0.8) * s;
    let mand_tip_v = (0.45 + 1.15 * charge) * s;
    let v_max = (2.0 * swell * s + 0.9 * s).max(mand_tip_v + 1.0 * s);
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, u_min, u_max, v_max, 1, 1) else {
        return;
    };

    let ramp = build_ramp4((30, 68, 26), (114, 214, 68), (210, 255, 118), fr);
    let stripe_col = fr.apply_hit((24, 52, 22));
    let venom_hot = mix_rgb(fr.accent, (242, 255, 138), 0.55);
    let hot_col = fr.apply_hit(mix_rgb((114, 214, 68), venom_hot, charge * 0.78));
    let mand_col = fr.apply_hit(mix_rgb((60, 118, 44), (195, 248, 108), 0.55 + 0.35 * charge));
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (130, 160, 80) } else { (0, 0, 0) };

    let inv_aru = 1.0 / (1.92 * s * swell);
    let inv_arv = 1.0 / (2.02 * s * swell);
    let inv_fru = 1.0 / (1.95 * s);
    let inv_frv = 1.0 / (1.35 * s);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let anu = (u + 1.9 * s) * inv_aru;
            let anv = v * inv_arv;
            let r2_a = anu * anu + anv * anv;
            let fnu = (u - 1.5 * s) * inv_fru;
            let fnv = v * inv_frv;
            let r2_f = fnu * fnu + fnv * fnv;
            let r2 = r2_a.min(r2_f);

            if r2 <= 1.0 {
                let col = if r2_a < r2_f {
                    if fast_sin((u * inv_s + 1.9) * 3.2) > 0.42 {
                        stripe_col
                    } else if charge > 0.15 {
                        hot_col
                    } else {
                        ramp[ramp4_idx(fr.lit(anu, anv), r2_a)]
                    }
                } else {
                    ramp[ramp4_idx(fr.lit(fnu, fnv), r2_f)]
                };
                pb.plot(x, y, col);
            } else if r2 <= 1.75 {
                plot_rim(pb, x, y, rim_col, 0.58, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Mandibles, translucent wing glint, eyes, and FULL live charge telegraph!
    let wing_col = mix_rgb((185, 240, 200), fr.accent, 0.45);
    let eye_col = if fr.hit > 0.7 {
        (30, 30, 20)
    } else {
        (255, 108, 44)
    };
    for side in [-1.0f32, 1.0] {
        let m0 = fr.px(3.2 * s, side * 0.9 * s);
        let m1 = fr.px((4.1 + 0.3 * charge) * s, side * mand_tip_v);
        pb.plot(m0.0, m0.1, mand_col);
        pb.plot(m1.0, m1.1, mand_col);
        let w0 = fr.px(-0.5 * s, side * 1.9 * s);
        let w1 = fr.px(-1.3 * s, side * 2.7 * s);
        let w2 = fr.px(-2.1 * s, side * 3.3 * s);
        blend_px(pb, w0.0, w0.1, wing_col, 0.42);
        blend_px(pb, w1.0, w1.1, wing_col, 0.36);
        blend_px(pb, w2.0, w2.1, wing_col, 0.28);
        let ep = fr.px(2.4 * s, side * 1.05 * s);
        pb.plot(ep.0, ep.1, eye_col);
    }

    if charge > 0.08 {
        let mp = fr.px(4.0 * s, 0.0);
        let orb_col = mix_rgb(venom_hot, (255, 255, 225), charge * 0.65);
        pb.plot(mp.0, mp.1, orb_col);
        let glow = scale_rgb(orb_col, 0.45 + 0.55 * charge);
        if charge > 0.35 {
            pb.plot_add(mp.0 + 1, mp.1, glow);
            pb.plot_add(mp.0 - 1, mp.1, glow);
            pb.plot_add(mp.0, mp.1 + 1, glow);
            pb.plot_add(mp.0, mp.1 - 1, glow);
        }
        if charge > 0.7 {
            let tip = scale_rgb(orb_col, 0.6);
            pb.plot_add(mp.0 + 2, mp.1, tip);
            pb.plot_add(mp.0 - 2, mp.1, tip);
            pb.plot_add(mp.0, mp.1 + 2, tip);
            pb.plot_add(mp.0, mp.1 - 2, tip);
        }
    }
}

fn draw_brute_lod(pb: &mut PixelBuffer, fr: &Frame, elite: bool) {
    let s = fr.r * (1.0 / 6.5);
    let stomp = fr.anim * 0.72;

    // 1) Elite gold perimeter pips.
    if elite {
        let pulse = 0.55 + 0.45 * fast_sin(fr.anim * 1.4);
        let aura_r = fr.r + 2.3 * s;
        let gold = scale_rgb((255, 206, 72), 0.55 + 0.35 * pulse);
        for i in 0..8 {
            let a0 = i as f32 * (TAU / 8.0) + fr.anim * 0.14;
            for step in 0..2 {
                let (sa, ca) = fast_sin_cos(a0 + step as f32 * 0.14);
                let tx = (fr.pos.x + ca * aura_r).round() as i32;
                let ty = (fr.pos.y + sa * aura_r).round() as i32;
                blend_px(pb, tx, ty, gold, 0.75);
            }
        }
    }

    // 2) Six armored leg segments.
    let leg_core = fr.apply_hit(if elite { (154, 98, 52) } else { (134, 54, 44) });
    let knee_hi = fr.apply_hit(if elite { (252, 212, 92) } else { (212, 98, 72) });
    for (i, &u_hip) in [2.6f32, -0.5, -3.6].iter().enumerate() {
        let sw = fast_sin(stomp + i as f32 * 2.1) * 1.35 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw), (1.0, -sw)] {
            let hip = fr.px(uh, side * 4.5 * s);
            let knee = fr.px(uh + swing * 0.55, side * 6.9 * s);
            let foot = fr.px(uh + swing - 0.55 * s, side * 8.1 * s);
            line_blend(pb, hip, knee, leg_core, 0.92);
            pb.plot(knee.0, knee.1, knee_hi);
            pb.plot(foot.0, foot.1, leg_core);
        }
    }

    // 3) Quadratic carapace + rhino horn wedge.
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -7.6 * s, 9.8 * s, 6.6 * s, 1, 1) else {
        return;
    };
    let (sh_c, mid_c, hi_c, trim_c) = if elite {
        ((62, 18, 42), (158, 36, 78), (240, 108, 150), (252, 206, 76))
    } else {
        ((68, 18, 20), (186, 54, 44), (250, 138, 84), (116, 34, 30))
    };
    let ramp = build_ramp4(sh_c, mid_c, hi_c, fr);
    let horn_base = fr.apply_hit(if elite { (190, 140, 42) } else { (128, 82, 60) });
    let horn_tip = fr.apply_hit(if elite { (252, 232, 148) } else { (215, 174, 134) });
    let trim_lit = fr.apply_hit(trim_c);
    let seam_col = fr.apply_hit((32, 12, 16));
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (165, 95, 75) } else { (0, 0, 0) };

    let inv_eu = 1.0 / (5.15 * s);
    let inv_ev = 1.0 / (5.68 * s);
    let inv_pu = 1.0 / (2.8 * s);
    let inv_pv = 1.0 / (4.48 * s);
    let damaged = fr.hp_frac < 0.68;
    let critical = fr.hp_frac < 0.38;

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let av = v.abs();
            let enu = (u + 1.55 * s) * inv_eu;
            let env = v * inv_ev;
            let r2_e = enu * enu + env * env;
            let pnu = (u - 3.15 * s) * inv_pu;
            let pnv = v * inv_pv;
            let r2_p = pnu * pnu + pnv * pnv;
            let r2 = r2_e.min(r2_p);

            let in_main_horn = u > 4.4 * s
                && u < 9.1 * s
                && (av < (9.2 * s - u) * 0.36
                    || (u > 7.0 * s
                        && (av - (0.35 + (u - 7.0 * s) * 0.55 / s) * s).abs() < 0.45 * s));
            let in_side_horn = elite
                && u > 3.8 * s
                && u < 7.0 * s
                && (av - (2.8 + (u - 3.8 * s) * 0.68 / s) * s).abs() < 0.62 * s;

            if in_main_horn || in_side_horn {
                pb.plot(x, y, if u > 6.6 * s { horn_tip } else { horn_base });
            } else if r2 <= 1.0 {
                let (nu, nv) = if r2_p < r2_e { (pnu, pnv) } else { (enu, env) };
                let mut col = ramp[ramp4_idx(fr.lit(nu, nv), r2)];
                let collar = (u - 1.5 * s).abs();
                if collar < 0.55 * s {
                    col = seam_col;
                } else if elite
                    && (r2 > 0.80
                        || (u < 1.4 * s && av < 0.48 * s)
                        || (u > 1.5 * s && collar < 1.15 * s))
                {
                    col = trim_lit;
                } else if !elite && u < 1.4 * s && av < 0.45 * s {
                    col = seam_col;
                }
                if damaged
                    && ((u + 1.8 * s) - (v - 1.5 * s) * 0.7).abs() < 0.42 * s
                    && u < 0.6 * s
                    && v > 0.4 * s
                {
                    col = if critical { (255, 165, 65) } else { (24, 10, 12) };
                }
                pb.plot(x, y, col);
            } else if r2 <= 1.42 {
                plot_rim(pb, x, y, rim_col, 0.64, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // Glowing ember eyes.
    let eye_col = if fr.hit > 0.7 {
        (30, 14, 16)
    } else if elite {
        (255, 118, 232)
    } else {
        (255, 202, 56)
    };
    for side in [-1.0f32, 1.0] {
        let e0 = fr.px(4.7 * s, side * 2.15 * s);
        pb.plot(e0.0, e0.1, eye_col);
    }
}

fn draw_boss_lod(pb: &mut PixelBuffer, fr: &Frame, fire_cd: f32) {
    let s = fr.r * (1.0 / 14.0);
    let inv_s = 1.0 / s;
    let charge = if fire_cd <= 0.0 {
        0.85 + 0.15 * fast_sin(fr.anim * 2.6).abs()
    } else if fire_cd < 0.50 {
        ((0.50 - fire_cd) * 2.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 2.2);

    // 1) Eight compact armored legs.
    let leg_core = fr.apply_hit((116, 30, 66));
    let leg_joint = fr.apply_hit((244, 202, 130));
    for (i, &u_hip) in [5.4f32, 1.8, -2.2, -6.2].iter().enumerate() {
        let sw = fast_sin(fr.anim * 0.8 + i as f32 * 1.6) * 2.1 * s;
        let uh = u_hip * s;
        for (side, swing) in [(-1.0f32, sw), (1.0, -sw)] {
            let hip = fr.px(uh, side * 7.2 * s);
            let knee = fr.px(uh + swing * 0.5, side * 12.2 * s);
            let foot = fr.px(uh + swing - 1.4 * s, side * 14.8 * s);
            line_blend(pb, hip, knee, leg_core, 0.92);
            line_blend(pb, knee, foot, leg_core, 0.78);
            pb.plot(knee.0, knee.1, leg_joint);
        }
    }

    // 2) Quadratic 3-segment carapace (abdomen, thorax, head) + stinger + mandibles.
    if let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -16.8 * s, 16.8 * s, 9.8 * s, 0, 0) {
        let abd_ramp = build_ramp4((56, 14, 36), (196, 38, 96), (255, 138, 188), fr);
        let th_ramp = build_ramp4((42, 12, 32), (128, 26, 68), (206, 68, 124), fr);
        let obsidian = fr.apply_hit((30, 12, 28));
        let gold_hi = fr.apply_hit((252, 224, 146));
        let hot_core = fr.apply_hit(mix_rgb((255, 118, 182), (255, 248, 224), charge));
        let hit_tint = fr.hit > 0.4;
        let rim_col = if hit_tint { (185, 95, 130) } else { (0, 0, 0) };

        let inv_au = 1.0 / (6.45 * s);
        let inv_av = 1.0 / (7.95 * s);
        let inv_tu = 1.0 / (5.45 * s);
        let inv_tv = 1.0 / (8.25 * s);
        let inv_hu = 1.0 / (3.65 * s);
        let inv_hv = 1.0 / (5.45 * s);
        let core_r2_max = (2.1 * s) * (2.1 * s);

        for y in y0..=y1 {
            let (mut u, mut v) = fr.local(x0, y);
            for x in x0..=x1 {
                let av = v.abs();
                let anu = (u + 7.0 * s) * inv_au;
                let anv = v * inv_av;
                let r2_a = anu * anu + anv * anv;
                let tnu = (u - 1.9 * s) * inv_tu;
                let tnv = v * inv_tv;
                let r2_t = tnu * tnu + tnv * tnv;
                let hnu = (u - 9.2 * s) * inv_hu;
                let hnv = v * inv_hv;
                let r2_h = hnu * hnu + hnv * hnv;
                let r2 = r2_a.min(r2_t).min(r2_h);

                if r2 <= 1.0 {
                    let col = if r2_h <= r2_t && r2_h <= r2_a {
                        if av < 0.95 * s && u > 7.2 * s && u < 11.8 * s {
                            gold_hi
                        } else {
                            abd_ramp[ramp4_idx(fr.lit(hnu, hnv), r2_h)]
                        }
                    } else if r2_t <= r2_a {
                        let du_c = u - 2.2 * s;
                        if du_c * du_c + v * v < core_r2_max {
                            hot_core
                        } else if r2_t > 0.74 && av > 3.6 * s {
                            gold_hi
                        } else {
                            th_ramp[ramp4_idx(fr.lit(tnu, tnv), r2_t)]
                        }
                    } else if fast_sin((u * inv_s + 7.0) * 1.6) > 0.18 {
                        obsidian
                    } else if av < 0.7 * s {
                        gold_hi
                    } else {
                        abd_ramp[ramp4_idx(fr.lit(anu, anv), r2_a)]
                    };
                    pb.plot(x, y, col);
                } else if u < -12.0 * s && u > -16.2 * s && av < (16.4 * s + u) * 0.42 {
                    pb.plot(x, y, gold_hi);
                } else if r2 <= 1.28 {
                    plot_rim(pb, x, y, rim_col, 0.68, hit_tint);
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }

    // 3) Mandible blades, crown horns, eyes, translucent forewings, nova pips, and HP arc.
    let gold_hi = fr.apply_hit((252, 224, 146));
    let mand_open = (1.8 + 3.0 * charge) * s;
    let eye_main = if fr.hit > 0.7 {
        (30, 14, 24)
    } else {
        mix_rgb((255, 236, 86), (255, 255, 235), charge * 0.5)
    };
    for side in [-1.0f32, 1.0] {
        let c0 = fr.px(8.2 * s, side * 4.2 * s);
        let c1 = fr.px(5.4 * s, side * 8.4 * s);
        line_blend(pb, c0, c1, gold_hi, 0.92);
        let m0 = fr.px(11.0 * s, side * 3.1 * s);
        let m1 = fr.px(15.8 * s, side * mand_open);
        line_blend(pb, m0, m1, gold_hi, 0.92);
        let e0 = fr.px(10.3 * s, side * 2.35 * s);
        pb.plot(e0.0, e0.1, eye_main);
    }

    // Fast translucent hornet forewing sweep (single-pass LUT, zero powf).
    let flap = fast_sin(fr.anim * 1.8);
    let wing_span = (13.8 + 2.1 * flap.abs()) * s;
    let wing_tint = mix_rgb((235, 148, 212), fr.accent, 0.42);
    let wu_min = (-6.5 + 1.3 * flap.min(0.0) - 2.6) * s;
    let wu_max = (3.4 + 1.3 * flap.max(0.0) + 2.6) * s;
    if let Some((x0, x1, y0, y1)) =
        fr.bbox_oriented_circ(pb, wu_min, wu_max, wing_span + 0.5, wing_span + 1.0, (0, 0))
    {
        let w_start = 4.6 * s;
        let inv_span = 1.0 / (wing_span - w_start).max(1.0);
        for y in y0..=y1 {
            let (mut u, mut v) = fr.local(x0, y);
            for x in x0..=x1 {
                let av = v.abs();
                if av >= w_start && av <= wing_span && u >= wu_min && u <= wu_max {
                    let t = (av - w_start) * inv_span;
                    let fore_half = 2.9 * s * boss_wing_fore_env(t);
                    let fore_mid = (2.8 - 4.6 * t - 2.9 * t * t + 1.3 * flap * t) * s;
                    if fore_half > 0.4 * s && (u - fore_mid).abs() <= fore_half {
                        let spar = (fore_mid + fore_half - u) < 0.85 * s;
                        let alpha = if spar { 0.56 } else { 0.24 } * (1.0 - t * 0.4);
                        let bg = pb.pixel_at(x as usize, y as usize);
                        pb.plot(x, y, mix_rgb(bg, wing_tint, alpha));
                    }
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }

    if charge > 0.08 {
        let pip_r = fr.r + (1.6 + (1.0 - charge) * 2.8) * s;
        let pip_col = mix_rgb((255, 100, 165), (255, 250, 230), charge);
        let glow = scale_rgb(pip_col, 0.55 * charge);
        let do_glow = charge > 0.5;
        for &(ca, sa) in &NOVA_10 {
            let px = (fr.pos.x + ca * pip_r).round() as i32;
            let py = (fr.pos.y + sa * pip_r).round() as i32;
            pb.plot(px, py, pip_col);
            if do_glow {
                pb.plot_add(px + 1, py, glow);
                pb.plot_add(px - 1, py, glow);
                pb.plot_add(px, py + 1, glow);
                pb.plot_add(px, py - 1, glow);
            }
        }
    }
    draw_boss_hp_arc(pb, fr, s, pulse);
}

// ============================================================================
// WAVE 3 ENEMIES (Pillbug, Bell Flea, Wick Moth, Porter, Turnkey)
// ============================================================================
//
// Sculpted SDF silhouettes with top-left directional lighting, dark silhouette
// rims, drop shadows, animated limbs/wings, gilded elite variants, fast zero-sqrt
// crowd LOD paths, and unmistakable danger tells (`WAVE3_DESIGN.md` §9).

/// Rotating gilded perimeter aura for elite Wave 3 enemies (matches Brute elite grammar).
#[inline]
fn draw_w3_elite_aura(pb: &mut PixelBuffer, fr: &Frame, extra_r: f32) {
    let pulse = 0.55 + 0.45 * fast_sin(fr.anim * 1.6);
    let aura_r = fr.r + extra_r;
    let gold = scale_rgb((255, 210, 78), 0.52 + 0.36 * pulse);
    for i in 0..8 {
        let a = i as f32 * (TAU / 8.0) + fr.anim * 0.16;
        let (sa, ca) = fast_sin_cos(a);
        let tx = (fr.pos.x + ca * aura_r).round() as i32;
        let ty = (fr.pos.y + sa * aura_r).round() as i32;
        blend_px(pb, tx, ty, gold, 0.78);
    }
}

/// High-contrast `×` landing cross + 6 px impact ring for the Bell Flea.
#[allow(clippy::too_many_arguments)]
fn draw_flea_cross(
    pb: &mut PixelBuffer,
    at: (i32, i32),
    r: i32,
    col: Rgb,
    hi: Rgb,
    alpha: f32,
    solid_ring: bool,
    pulse: f32,
) {
    let (cx, cy) = at;
    let arm = (r - 2).max(2);
    // Subtle dark backing under the cross arms so it pops on sunlit planks too.
    for k in -arm..=arm {
        darken_px(pb, cx + k, cy + k + 1, 0.42 * alpha);
        darken_px(pb, cx + k, cy - k + 1, 0.42 * alpha);
    }
    // Crisp diagonal `×` cross.
    for k in -arm..=arm {
        let edge_fade = if k.abs() == arm { 0.78 } else { 1.0 };
        let c = if k.abs() <= 1 { hi } else { col };
        blend_px(pb, cx + k, cy + k, c, alpha * edge_fade);
        blend_px(pb, cx + k, cy - k, c, alpha * edge_fade);
    }
    // Center hot pip.
    blend_px(pb, cx, cy, hi, (alpha + 0.12).min(1.0));
    if solid_ring {
        pb.plot_add(cx, cy, scale_rgb(hi, 0.35 + 0.25 * pulse));
    }

    // 6 px impact perimeter ring (16 points: every other when dotted, all when solid).
    let n = 16;
    let step = if solid_ring { 1 } else { 2 };
    let phase = if solid_ring { 0 } else { (pulse > 0.5) as usize };
    let mut i = phase;
    while i < n {
        let a = i as f32 * (TAU / n as f32);
        let (sa, ca) = fast_sin_cos(a);
        let x = cx + (ca * r as f32).round() as i32;
        let y = cy + (sa * r as f32).round() as i32;
        darken_px(pb, x, y + 1, 0.32 * alpha);
        blend_px(pb, x, y, if i % 4 == 0 { hi } else { col }, alpha * if solid_ring { 0.95 } else { 0.80 });
        i += step;
    }
}

/// Shared Bell Flea landing crosses + jump trajectory tell (used by both Full & LOD).
fn draw_bellflea_tells(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2) {
    if e.w3.flea_phase == 0 {
        return;
    }
    let home = e.w3.home - cam;
    let hop = e.w3.hop_to - cam;
    let home_px = (home.x.round() as i32, home.y.round() as i32);
    let hop_px = (hop.x.round() as i32, hop.y.round() as i32);
    let landed = e.w3.flea_phase == 2;
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 5.2);
    let r = crate::game::ecs::systems::enemy_wave3::FLEA_CROSS_RADIUS.round() as i32;

    // Dotted trajectory tether between home and hop_to so both crosses read as a pair.
    let dv = hop - home;
    let dist = dv.len();
    if dist > 4.0 {
        let steps = (dist / 4.0).round() as i32;
        let line_col = if landed { (182, 214, 236) } else { (236, 176, 98) };
        for k in 1..steps {
            if k % 2 == 1 {
                let t = k as f32 / steps as f32;
                let p = home + dv * t;
                blend_px(pb, p.x.round() as i32, p.y.round() as i32, line_col, 0.42);
            }
        }
    }

    let hot_col = mix_rgb((255, 134, 62), (255, 224, 128), 0.35 + 0.45 * pulse);
    let hot_hi = (255, 248, 208);
    let return_col = (158, 206, 236);
    let return_hi = (232, 246, 255);

    if landed {
        // First cross just struck (bright solid chime ring); second cross (home)
        // is now the active imminent threat pulsing bright amber-red!
        draw_flea_cross(pb, hop_px, r, (255, 214, 118), hot_hi, 0.92, true, pulse);
        draw_flea_cross(pb, home_px, r, hot_col, hot_hi, 0.86, false, pulse);
    } else {
        // Winding up: hop_to is imminent (hot amber-vermilion), home is the secondary return mark.
        draw_flea_cross(pb, home_px, r, return_col, return_hi, 0.68, false, pulse);
        draw_flea_cross(pb, hop_px, r, hot_col, hot_hi, 0.88, false, pulse);
    }
}

/// Shared Turnkey crossbar tell & live electrified barrier (used by both Full & LOD).
fn draw_turnkey_bar(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2) {
    if e.w3.bar_t <= 0.0 {
        return;
    }
    let centre = e.w3.bar_pos - cam;
    let d = e.w3.bar_dir;
    let perp = Vec2::new(-d.y, d.x);
    let half = crate::game::ecs::systems::enemy_wave3::CROSSBAR_LEN * 0.5;
    let a = centre - d * half;
    let b = centre + d * half;
    let telling = e.w3.bar_telling();
    let spent = e.w3.bar_spent;
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 6.0);

    let steps = (crate::game::ecs::systems::enemy_wave3::CROSSBAR_LEN.round() as i32).max(12);
    if telling {
        // Marching dotted warning line: dark contrast backing + alternating bright amber dashes.
        let tell_col = mix_rgb((248, 178, 72), (255, 236, 152), 0.35 + 0.50 * pulse);
        let shift = ((fr.anim * 3.5).floor() as i32).rem_euclid(4);
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let p = a + (b - a) * t;
            let px = (p.x.round() as i32, p.y.round() as i32);
            if (k + shift) % 4 < 2 {
                darken_px(pb, px.0 + 1, px.1 + 1, 0.48);
                blend_px(pb, px.0, px.1, tell_col, 0.90);
            } else {
                darken_px(pb, px.0, px.1, 0.30);
            }
        }
        // Brass anchor pins at both ends so the exact 22 px span reads immediately.
        for end in [a, b] {
            let ex = end.x.round() as i32;
            let ey = end.y.round() as i32;
            darken_px(pb, ex + 1, ey + 1, 0.5);
            pb.plot(ex, ey, (255, 236, 160));
            blend_px(pb, ex, ey - 1, (196, 154, 68), 0.85);
        }
    } else if spent {
        // Spent crossbar: cooled, dull bronze bar so the player knows it is safe.
        let cool = (128, 118, 92);
        let dark = (62, 56, 44);
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let p = a + (b - a) * t;
            let px = (p.x.round() as i32, p.y.round() as i32);
            blend_px(pb, px.0, px.1, cool, 0.65);
            blend_px(pb, px.0, px.1 + 1, dark, 0.45);
        }
        for end in [a, b] {
            blend_px(pb, end.x.round() as i32, end.y.round() as i32, dark, 0.8);
        }
    } else {
        // Live crossbar: blazing 3-layer brass/amber hazard rail with white-gold filament!
        let outer = (232, 134, 42);
        let core = mix_rgb((255, 212, 88), (255, 250, 218), 0.45 + 0.40 * pulse);
        let glow = scale_rgb((255, 196, 84), 0.32 + 0.20 * pulse);
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let p = a + (b - a) * t;
            let px = p.x.round() as i32;
            let py = p.y.round() as i32;
            let s1 = p + perp * 1.1;
            let s2 = p - perp * 1.1;
            darken_px(pb, px + 1, py + 2, 0.52);
            blend_px(pb, s1.x.round() as i32, s1.y.round() as i32, outer, 0.82);
            blend_px(pb, s2.x.round() as i32, s2.y.round() as i32, outer, 0.82);
            pb.plot(px, py, core);
            if k % 3 == 0 {
                pb.plot_add(px, py, glow);
            }
        }
        // Gleaming brass lock-pins at both endpoints.
        for end in [a, b] {
            let ex = end.x.round() as i32;
            let ey = end.y.round() as i32;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                blend_px(pb, ex + dx, ey + dy, (214, 152, 54), 0.85);
            }
            pb.plot(ex, ey, (255, 252, 232));
            pb.plot_add(ex, ey, (120, 96, 40));
        }
    }
}

// ============================================================================
// 7. PILLBUG — Ribbed Silver Armored Isopod (r = 5.0)
// ============================================================================

fn draw_pillbug(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 5.0);
    let curled = e.w3.curl_t > 0.0;

    if elite {
        draw_w3_elite_aura(pb, fr, 2.2 * s);
    }

    let (sh_c, mid_c, hi_c, seam_c, trim_c) = if elite {
        (
            (54, 62, 92),
            (136, 150, 198),
            (238, 242, 255),
            (32, 36, 58),
            (252, 206, 78),
        )
    } else {
        (
            (62, 68, 82),
            (158, 168, 186),
            (242, 248, 255),
            (40, 44, 56),
            (218, 232, 252),
        )
    };
    let rim_col = if fr.hit > 0.4 { (160, 168, 190) } else { (0, 0, 0) };

    // 1) Six scuttling chitin legs + twitching front antennae (tucked away when curled!).
    if !curled {
        let leg_c = fr.apply_hit(if elite { (128, 108, 72) } else { (86, 92, 106) });
        let foot_c = fr.apply_hit(if elite { (220, 184, 88) } else { (122, 128, 144) });
        for (i, &u_hip) in [2.1f32, 0.2, -1.7].iter().enumerate() {
            let sw = fast_sin(fr.anim * 1.15 + i as f32 * 2.1) * 0.85 * s;
            for (side, dir) in [(-1.0f32, sw), (1.0, -sw)] {
                let hip = fr.px(u_hip * s, side * 2.6 * s);
                let knee = fr.px(u_hip * s + dir * 0.5, side * 3.8 * s);
                let foot = fr.px(u_hip * s + dir - 0.3 * s, side * 4.5 * s);
                darken_px(pb, foot.0 + 1, foot.1 + 1, 0.28);
                line_blend(pb, hip, knee, leg_c, 0.90);
                line_blend(pb, knee, foot, foot_c, 0.88);
            }
        }
        // Short curved antennae at the front head shield.
        let ant_sw = fast_sin(fr.anim * 1.8) * 0.35 * s;
        let ant_c = fr.apply_hit(if elite { trim_c } else { (132, 142, 162) });
        for side in [-1.0f32, 1.0] {
            let a0 = fr.px(4.1 * s, side * 1.1 * s);
            let a1 = fr.px(5.4 * s, side * (2.1 * s + ant_sw));
            line_blend(pb, a0, a1, ant_c, 0.85);
        }
    }

    // 2) Sculpted ribbed carapace (broad articulated oval walking, tight armored sphere curled).
    let ru = if curled { 3.5 * s } else { 4.7 * s };
    let rv = if curled { 3.5 * s } else { 3.55 * s };
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -ru - 1.5 * s, ru + 1.8 * s, rv + 1.5 * s, 2, 2)
    else {
        return;
    };

    let body_sdf = EllipseSdf::new(0.0, 0.0, ru, rv);
    let rim_lim = 0.95 * s;
    let sh_lim = 0.45 * s;
    let sh_sq = body_sdf.r2_limit(sh_lim);
    let inv_ru = 1.0 / ru;
    let inv_rv = 1.0 / rv;
    let du_sh = -1.6 * (fr.f.x + fr.f.y);
    let dv_sh = -1.6 * (fr.p.x + fr.p.y);
    let damaged = fr.hp_frac < 0.55;
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 4.5);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let d = body_sdf.eval(u, v);
            let cover = (0.52 - d * 1.18).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = (u * inv_ru).clamp(-1.0, 1.0);
                let nv = (v * inv_rv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.55 + nv * nv * 0.65)).clamp(0.0, 1.0);
                let lit = fr.lit(nu, nv);
                let mut lum = 0.36 + 0.42 * dome + 0.60 * lit;
                if lit > 0.38 && dome > 0.32 {
                    lum += 0.28; // crisp metallic silver sheen
                }

                // Curved transverse articulated armor ribs (4–5 overlapping plates).
                let arch_u = u + (v * v) * (if curled { 0.14 } else { 0.08 }) / s.max(0.5);
                let rib_period = if curled { 2.05 * s } else { 2.15 * s };
                let phase = ((arch_u + 12.0 * s) / rib_period).fract();
                let mut col = shade3(sh_c, mid_c, hi_c, lum);

                if phase < 0.22 {
                    // Recessed overlap groove between armor plates.
                    col = mix_rgb(col, seam_c, 0.78);
                } else if phase < 0.42 {
                    // Beveled specular leading ridge of each steel plate.
                    let ridge = if elite { trim_c } else { hi_c };
                    col = mix_rgb(col, ridge, 0.48 * (0.55 + 0.45 * lit).clamp(0.0, 1.0));
                }

                // Subtle dorsal ridge line + rim plate skirt.
                if !curled && v.abs() > rv * 0.76 {
                    col = mix_rgb(col, if elite { trim_c } else { sh_c }, if elite { 0.55 } else { 0.45 });
                }

                // Curled state visual reads: gleaming frontal 120° deflector shell & vulnerable pink rear!
                if curled {
                    if nu > 0.30 {
                        // Frontal armored shield: gleaming blue-white steel sheen.
                        let shield_boost = ((nu - 0.30) * 1.4).clamp(0.0, 1.0);
                        col = mix_rgb(col, (240, 248, 255), shield_boost * (0.48 + 0.16 * pulse));
                    } else if nu < -0.52 && nv.abs() < 0.62 {
                        // Exposed vulnerable soft underbelly at the rear (2× damage cue!).
                        let rear_col = mix_rgb((218, 96, 112), (255, 178, 172), 0.45 + 0.35 * pulse);
                        col = mix_rgb(col, rear_col, 0.85);
                    }
                } else if damaged && ((u + 0.8 * s) - v * 0.7).abs() < 0.35 * s && u < 1.8 * s {
                    col = (28, 30, 38);
                }

                let lit_col = fr.apply_hit(col);
                if cover >= 1.0 {
                    pb.plot(x, y, lit_col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.68 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, lit_col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.68));
            } else {
                let su = u + du_sh;
                let sv = v + dv_sh;
                if body_sdf.within_sq(su, sv, sh_sq) {
                    darken_px(pb, x, y, 0.38);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    if curled {
        // Unmistakable frontal 120° deflector shield arc (-60°..+60°) standing 1.2 px proud of the shell.
        let shield_col = mix_rgb((190, 224, 255), (255, 255, 255), 0.45 + 0.40 * pulse);
        let arc_r = ru + 1.15 * s;
        for k in 0..=12 {
            let a = -1.047 + k as f32 * (2.094 / 12.0); // ±60° = 120° frontal arc
            let (sa, ca) = fast_sin_cos(a);
            let pt = fr.px(ca * arc_r, sa * arc_r);
            darken_px(pb, pt.0 + 1, pt.1 + 1, 0.35);
            blend_px(pb, pt.0, pt.1, shield_col, 0.95);
            if k % 3 == 0 {
                pb.plot_add(pt.0, pt.1, scale_rgb((180, 220, 255), 0.45 + 0.25 * pulse));
            }
        }
        // Subtle pulsing coral vulnerability dot at the exposed rear gap.
        let rear = fr.px(-ru - 0.5 * s, 0.0);
        blend_px(pb, rear.0, rear.1, (255, 132, 138), 0.85);
    } else {
        // Tiny amber-orange isopod eyes tucked on the front head plate.
        let eye_col = if fr.hit > 0.7 {
            (28, 30, 38)
        } else if elite {
            (255, 118, 228)
        } else {
            (255, 178, 72)
        };
        for side in [-1.0f32, 1.0] {
            let ep = fr.px(3.75 * s, side * 1.25 * s);
            pb.plot(ep.0, ep.1, eye_col);
        }
    }
}

fn draw_pillbug_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 5.0);
    let curled = e.w3.curl_t > 0.0;
    if elite {
        draw_w3_elite_aura(pb, fr, 2.0 * s);
    }
    let ru = if curled { 3.5 * s } else { 4.6 * s };
    let rv = if curled { 3.5 * s } else { 3.45 * s };
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -ru - s, ru + s, rv + s, 1, 1) else {
        return;
    };
    let ramp = if elite {
        build_ramp4((54, 62, 92), (136, 150, 198), (238, 242, 255), fr)
    } else {
        build_ramp4((62, 68, 82), (158, 168, 186), (242, 248, 255), fr)
    };
    let seam_col = fr.apply_hit((40, 44, 56));
    let rear_col = fr.apply_hit((242, 134, 140));
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (160, 168, 190) } else { (0, 0, 0) };
    let inv_ru = 1.0 / ru;
    let inv_rv = 1.0 / rv;
    let rib_inv = 1.0 / (2.15 * s);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let nu = u * inv_ru;
            let nv = v * inv_rv;
            let r2 = nu * nu + nv * nv;
            if r2 <= 1.0 {
                let col = if curled && nu < -0.52 && nv.abs() < 0.62 {
                    rear_col
                } else if ((u + 12.0 * s) * rib_inv).fract() < 0.24 {
                    seam_col
                } else {
                    ramp[ramp4_idx(fr.lit(nu, nv), r2)]
                };
                pb.plot(x, y, col);
            } else if r2 <= 1.58 {
                plot_rim(pb, x, y, rim_col, 0.62, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }
    if curled {
        let arc_r = ru + 1.0 * s;
        for k in 0..=8 {
            let a = -1.047 + k as f32 * (2.094 / 8.0);
            let (sa, ca) = fast_sin_cos(a);
            let pt = fr.px(ca * arc_r, sa * arc_r);
            blend_px(pb, pt.0, pt.1, (224, 240, 255), 0.92);
        }
    }
}

// ============================================================================
// 8. BELL FLEA — Copper Tuning-Fork Jumper (r = 2.0)
// ============================================================================

fn draw_bellflea(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let s = fr.r * 0.5;
    let ringing = e.w3.flea_phase != 0;

    // 1) Landing crosses on the floor FIRST so the flea's body sits cleanly over them.
    draw_bellflea_tells(pb, fr, e, cam);

    if elite {
        draw_w3_elite_aura(pb, fr, 2.4 * s);
    }

    let (sh_c, mid_c, hi_c) = if elite {
        ((104, 48, 38), (236, 152, 82), (255, 244, 190))
    } else {
        ((94, 48, 22), (218, 134, 68), (255, 226, 168))
    };
    let leg_c = fr.apply_hit((112, 62, 30));
    let leg_hi = fr.apply_hit((192, 118, 60));
    let rim_col = if fr.hit > 0.4 { (186, 130, 86) } else { (0, 0, 0) };

    // 2) Spring-loaded bent hind legs (coiled tight when winding up, kicked out when landed!).
    let crouch = match e.w3.flea_phase {
        1 => 0.55,
        2 => -0.45,
        _ => fast_sin(fr.anim * 1.4) * 0.30,
    };
    for side in [-1.0f32, 1.0] {
        let hip = fr.px(-0.7 * s, side * 1.1 * s);
        let knee = fr.px((-1.7 + crouch * 0.7) * s, side * (2.6 - crouch * 0.4) * s);
        let foot = fr.px((-2.9 - crouch * 0.6) * s, side * (2.1 + crouch * 0.4) * s);
        darken_px(pb, knee.0 + 1, knee.1 + 1, 0.34);
        darken_px(pb, foot.0 + 1, foot.1 + 1, 0.34);
        line_blend(pb, hip, knee, leg_hi, 0.92);
        line_blend(pb, knee, foot, leg_c, 0.95);
    }

    // 3) Sculpted copper tuning-fork body: rear stem + curved yoke + two bold parallel forward tines!
    let vib = if ringing { fast_sin(fr.anim * 9.0) * 0.25 * s } else { 0.0 };
    let tine_v = 1.50 * s + vib;
    let stem_sdf = TaperSdf::new((-2.8 * s, 0.0), (-0.4 * s, 0.0), 0.55 * s, 0.95 * s);
    let yoke_sdf = EllipseSdf::new(-0.2 * s, 0.0, 1.05 * s, 1.70 * s);
    let tine_sdf = CapsuleSdf::new(0.1 * s, tine_v, 3.95 * s, tine_v, 0.64 * s);
    let k_join = 0.45 * s;
    let inv_k = 1.0 / k_join;

    let body_sd = |u: f32, v: f32| -> f32 {
        let av = v.abs();
        let stem = stem_sdf.eval(u, v);
        let yoke = yoke_sdf.eval(u, v);
        let tine = tine_sdf.eval(u, av);
        // Carve a crisp U-notch between the two prongs so the tuning-fork profile reads at 2x zoom-out!
        let base = smin_inv(stem, yoke, k_join, inv_k);
        let fork = smin_inv(base, tine, 0.25 * s, 4.0 / s);
        if u > 0.35 * s && av < tine_v - 0.52 * s {
            fork.max((tine_v - 0.52 * s) - av)
        } else {
            fork
        }
    };

    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -3.8 * s, 4.7 * s, 2.8 * s, 2, 2) else {
        return;
    };
    let rim_lim = 0.85 * s;
    let sh_lim = 0.35 * s;
    let du_sh = -1.3 * (fr.f.x + fr.f.y);
    let dv_sh = -1.3 * (fr.p.x + fr.p.y);
    let inv_nu = 1.0 / (3.4 * s);
    let inv_nv = 1.0 / (2.0 * s);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let d = body_sd(u, v);
            let cover = (0.50 - d * 1.22).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = (u * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let lit = fr.lit(nu, nv);
                let ridge = (1.0 - (d / (-0.7 * s)).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                let mut lum = 0.42 + 0.36 * (1.0 - ridge * 0.5) + 0.60 * lit;
                if lit > 0.35 {
                    lum += 0.25;
                }
                if u > 2.4 * s {
                    lum += 0.16; // bright polished copper tips
                }
                let col = fr.apply_hit(shade3(sh_c, mid_c, hi_c, lum));
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
            } else {
                let su = u + du_sh;
                let sv = v + dv_sh;
                if body_sd(su, sv) < sh_lim {
                    darken_px(pb, x, y, 0.36);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 4) Turquoise/white acoustic chime spark humming between the vibrating prongs when ringing!
    if ringing {
        let p = 0.5 + 0.5 * fast_sin(fr.anim * 8.0);
        let chime = fr.px(2.1 * s, 0.0);
        let ccol = mix_rgb((138, 232, 255), (255, 252, 224), 0.45 + 0.45 * p);
        pb.plot(chime.0, chime.1, ccol);
        pb.plot_add(chime.0, chime.1, scale_rgb(ccol, 0.45));
    } else {
        // Tiny bright bead eyes on the yoke shoulders.
        let eye_c = if fr.hit > 0.7 { (36, 20, 14) } else { (118, 242, 234) };
        for side in [-1.0f32, 1.0] {
            let ep = fr.px(0.35 * s, side * 1.1 * s);
            blend_px(pb, ep.0, ep.1, eye_c, 0.85);
        }
    }
}

fn draw_bellflea_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let s = fr.r * 0.5;
    draw_bellflea_tells(pb, fr, e, cam);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.2 * s);
    }
    let copper = fr.apply_hit(if elite { (240, 162, 86) } else { (218, 134, 68) });
    let copper_hi = fr.apply_hit((255, 232, 178));
    let leg = fr.apply_hit((112, 62, 30));
    for side in [-1.0f32, 1.0] {
        let hip = fr.px(-0.8 * s, side * 1.1 * s);
        let foot = fr.px(-2.4 * s, side * 2.4 * s);
        line_blend(pb, hip, foot, leg, 0.85);
        // Two clear forward copper prongs.
        let p0 = fr.px(0.0, side * 1.35 * s);
        let p1 = fr.px(3.5 * s, side * 1.35 * s);
        thick_line(pb, p0, p1, copper_hi, copper);
    }
    let h0 = fr.px(-2.6 * s, 0.0);
    let h1 = fr.px(0.2 * s, 0.0);
    thick_line(pb, h0, h1, copper_hi, copper);
}

// ============================================================================
// 9. WICK MOTH — Teardrop Lantern Moth with Lit Wick Tail (r = 2.5)
// ============================================================================

fn draw_wickmoth(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let flap = fast_sin(fr.anim * 2.1); // -1..1
    let spread = 0.5 + 0.5 * flap;
    let span = (3.6 + 1.15 * spread) * s;

    if elite {
        draw_w3_elite_aura(pb, fr, 3.0 * s);
    }

    // 1) Elevated ground shadow shifted down-right (it flies!).
    let scx = fr.pos.x.round() as i32 + (2.0 * s).round() as i32;
    let scy = fr.pos.y.round() as i32 + (3.2 * s + 0.35 * flap * s).round() as i32;
    let srx = (2.4 + 0.6 * spread) * s;
    let sry = 1.6 * s;
    let sy0 = (scy - sry.ceil() as i32).max(0);
    let sy1 = (scy + sry.ceil() as i32).min(pb.h as i32 - 1);
    let sx0 = (scx - srx.ceil() as i32).max(0);
    let sx1 = (scx + srx.ceil() as i32).min(pb.w as i32 - 1);
    let inv_srx = 1.0 / srx;
    let inv_sry = 1.0 / sry;
    for y in sy0..=sy1 {
        let ny = (y - scy) as f32 * inv_sry;
        for x in sx0..=sx1 {
            let nx = (x - scx) as f32 * inv_srx;
            let r2 = nx * nx + ny * ny;
            if r2 <= 1.0 {
                darken_px(pb, x, y, 0.32 * (1.0 - r2 * 0.45));
            }
        }
    }

    // 2) Swept parchment/soot moth wings + plump teardrop torso (distinct delta silhouette vs Bat!).
    let (body_sh, body_mid, body_hi) = if elite {
        ((64, 24, 34), (154, 62, 68), (242, 168, 126))
    } else {
        ((58, 38, 30), (138, 96, 74), (218, 178, 134))
    };
    let (wing_sh, wing_mid, wing_hi, spot_c) = if elite {
        ((72, 24, 36), (176, 86, 76), (252, 212, 134), (255, 224, 102))
    } else {
        ((86, 62, 48), (196, 162, 122), (244, 224, 184), (62, 36, 26))
    };
    let rim_col = if fr.hit > 0.4 { (186, 150, 118) } else { (0, 0, 0) };

    let torso_sdf = TaperSdf::new((2.1 * s, 0.0), (-2.3 * s, 0.0), 1.05 * s, 0.45 * s);
    let head_sdf = EllipseSdf::new(2.2 * s, 0.0, 0.85 * s, 0.95 * s);
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented_circ(
        pb,
        -3.6 * s,
        3.8 * s,
        span + 1.2 * s,
        span + 1.8 * s,
        (0, 0),
    ) else {
        return;
    };

    let w_start = 0.75 * s;
    let inv_wspan = 1.0 / (span - w_start).max(0.5);
    let rim_lim = 0.85 * s;
    let inv_nu = 1.0 / (2.5 * s);
    let inv_nv = 1.0 / (1.3 * s);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let av = v.abs();
            let d_body = torso_sdf.eval(u, v).min(head_sdf.eval(u, v));

            // Leaf-shaped delta moth wings sweeping back into a warm teardrop silhouette.
            let (d_wing, wt, w_u_frac) = if av >= w_start * 0.7 && av <= span + 1.0 * s {
                let t = ((av - w_start) * inv_wspan).clamp(0.0, 1.0);
                // Swept leading edge + scalloped soft trailing margin.
                let u_lead = (1.65 - 2.35 * t + 0.55 * flap * t) * s;
                let chord = (2.95 * (1.0 - t * 0.55) + 0.45 * fast_sin(t * PI)) * s;
                let u_trail = u_lead - chord;
                let du_top = u - u_lead;
                let du_bot = u_trail - u;
                let dv_tip = av - span;
                let d = du_top.max(du_bot).max(dv_tip);
                let uf = ((u - u_trail) / chord.max(0.5)).clamp(0.0, 1.0);
                (d, t, uf)
            } else {
                (10.0, 0.0, 0.0)
            };

            let d_all = d_body.min(d_wing);
            if d_all >= rim_lim {
                u += fr.f.x;
                v += fr.p.x;
                continue;
            }
            let body_cov = (0.50 - d_body * 1.2).clamp(0.0, 1.0);
            let wing_cov = (0.48 - d_wing * 1.15).clamp(0.0, 1.0);
            let cover = body_cov.max(wing_cov);

            if cover <= 0.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.56));
            } else if body_cov >= wing_cov {
                let nu = (u * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.4 + nv * nv * 0.7)).clamp(0.0, 1.0);
                let lum = 0.42 + 0.36 * dome + 0.54 * fr.lit(nu, nv);
                let mut col = shade3(body_sh, body_mid, body_hi, lum);
                // Warm ember underglow near the tail root.
                if u < -1.1 * s {
                    let g = ((-1.1 * s - u) / (1.5 * s)).clamp(0.0, 1.0);
                    col = mix_rgb(col, (255, 166, 72), g * 0.65);
                }
                let lit_col = fr.apply_hit(col);
                if body_cov >= 1.0 {
                    pb.plot(x, y, lit_col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.56 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, lit_col, body_cov));
                }
            } else {
                let lit = fr.lit(0.25, if v >= 0.0 { 0.6 } else { -0.6 });
                let mut col = shade3(wing_sh, wing_mid, wing_hi, 0.48 + 0.38 * lit + 0.14 * w_u_frac);
                // Distinctive moth wing eyespot + warm smoky trailing margin.
                let spot_d2 = (wt - 0.52) * (wt - 0.52) * 14.0 + (w_u_frac - 0.46) * (w_u_frac - 0.46) * 12.0;
                if spot_d2 < 0.42 {
                    col = spot_c;
                } else if spot_d2 < 0.85 {
                    col = mix_rgb(col, (255, 198, 108), 0.65);
                } else if w_u_frac < 0.22 {
                    col = mix_rgb(col, body_sh, 0.55);
                }
                let lit_col = fr.apply_hit(col);
                let bg = pb.pixel_at(x as usize, y as usize);
                let out = if cover < 1.0 {
                    mix_rgb(bg, rim_col, 0.56 * (1.0 - cover))
                } else {
                    bg
                };
                pb.plot(x, y, mix_rgb(out, lit_col, wing_cov * 0.94));
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Feathery moth antennae forward.
    let ant_c = fr.apply_hit((226, 196, 152));
    for side in [-1.0f32, 1.0] {
        let a0 = fr.px(2.5 * s, side * 0.5 * s);
        let a1 = fr.px(3.5 * s, side * 1.4 * s);
        line_blend(pb, a0, a1, ant_c, 0.78);
    }

    // 4) Braided wick stub + live flickering flame trailing behind the teardrop tail!
    let flick = 0.5 + 0.5 * fast_sin(fr.anim * 7.5);
    let wick_base = fr.px(-2.4 * s, 0.0);
    let flame_p = fr.px((-3.3 - 0.3 * flick) * s, (fast_sin(fr.anim * 5.2) * 0.35) * s);
    pb.plot(wick_base.0, wick_base.1, (76, 54, 40));

    let outer_fire = if e.hp > 0.0 {
        mix_rgb((255, 118, 36), (255, 196, 76), flick)
    } else {
        (255, 224, 148)
    };
    let core_fire = if elite { (255, 252, 236) } else { (255, 246, 196) };
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        blend_px(pb, flame_p.0 + dx, flame_p.1 + dy, outer_fire, 0.80);
        pb.plot_add(flame_p.0 + dx, flame_p.1 + dy, scale_rgb(outer_fire, 0.45));
    }
    pb.plot(flame_p.0, flame_p.1, core_fire);
    pb.plot_add(flame_p.0, flame_p.1 - 1, scale_rgb(core_fire, 0.55));
}

fn draw_wickmoth_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let flap = fast_sin(fr.anim * 2.1);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.8 * s);
    }
    let wing_c = fr.apply_hit(if elite { (198, 108, 92) } else { (204, 170, 128) });
    let body_c = fr.apply_hit(if elite { (154, 62, 68) } else { (138, 96, 74) });
    let span = (3.4 + 0.9 * (0.5 + 0.5 * flap)) * s;
    for side in [-1.0f32, 1.0] {
        let r0 = fr.px(1.2 * s, side * 0.6 * s);
        let tip = fr.px((-0.7 + 0.5 * flap) * s, side * span);
        let back = fr.px(-1.6 * s, side * (span * 0.55));
        thick_line(pb, r0, tip, wing_c, scale_rgb(wing_c, 0.65));
        line_blend(pb, tip, back, scale_rgb(wing_c, 0.72), 0.85);
    }
    let head = fr.px(2.0 * s, 0.0);
    let tail_r = fr.px(-2.0 * s, 0.0);
    thick_line(pb, head, tail_r, body_c, scale_rgb(body_c, 0.6));
    let flame = fr.px(-3.1 * s, 0.0);
    let ember = if e.hp > 0.0 { (255, 196, 84) } else { (255, 248, 210) };
    pb.plot(flame.0, flame.1, (255, 248, 214));
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        pb.plot_add(flame.0 + dx, flame.1 + dy, scale_rgb(ember, 0.55));
    }
}

// ============================================================================
// 10. PORTER — Emerald Scarab Beetle Carrying a White Sugar Cube (r = 4.5)
// ============================================================================

fn draw_porter(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 4.5);
    let raising = e.w3.raise_t > 0.0;
    let raise_total = crate::game::ecs::systems::enemy_wave3::PORTER_RAISE_SECS;
    let raise_p = if raising && raise_total > 0.0 {
        (1.0 - e.w3.raise_t / raise_total).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 5.0);

    if elite {
        draw_w3_elite_aura(pb, fr, 2.3 * s);
    }

    // Pulsing mint-green healing tell ring on the floor while raising the sugar cube!
    if raising {
        let ring_r = (6.8 + 1.8 * raise_p) * s;
        let heal_c = mix_rgb((118, 238, 174), (242, 255, 246), 0.35 + 0.45 * pulse);
        for i in 0..16 {
            if i % 2 == 0 || raise_p > 0.55 {
                let a = i as f32 * (TAU / 16.0) + fr.anim * 0.4;
                let (sa, ca) = fast_sin_cos(a);
                let rx = (fr.pos.x + ca * ring_r).round() as i32;
                let ry = (fr.pos.y + sa * ring_r).round() as i32;
                blend_px(pb, rx, ry, heal_c, 0.58 + 0.32 * raise_p);
            }
        }
    }

    let (sh_c, mid_c, hi_c, trim_c) = if elite {
        ((26, 74, 68), (66, 168, 148), (168, 248, 224), (252, 208, 82))
    } else {
        ((38, 66, 36), (112, 164, 96), (196, 238, 170), (62, 92, 52))
    };
    let leg_c = fr.apply_hit(if elite { (72, 118, 102) } else { (68, 94, 58) });
    let leg_hi = fr.apply_hit(if elite { (220, 188, 92) } else { (116, 148, 96) });
    let rim_col = if fr.hit > 0.4 { (142, 178, 130) } else { (0, 0, 0) };

    // 1) Six sturdy scarab legs scuttling on alternating tripods.
    for (i, &u_hip) in [1.7f32, -0.3, -2.2].iter().enumerate() {
        let sw = if raising {
            0.0
        } else {
            fast_sin(fr.anim * 1.1 + i as f32 * 2.0) * 0.75 * s
        };
        for (side, dir) in [(-1.0f32, sw), (1.0, -sw)] {
            let hip = fr.px(u_hip * s, side * 2.5 * s);
            let knee = fr.px(u_hip * s + dir * 0.5, side * 3.8 * s);
            let foot = fr.px(u_hip * s + dir - 0.35 * s, side * 4.5 * s);
            darken_px(pb, foot.0 + 1, foot.1 + 1, 0.30);
            line_blend(pb, hip, knee, leg_hi, 0.90);
            line_blend(pb, knee, foot, leg_c, 0.92);
        }
    }

    // 2) Sculpted domed scarab carapace (elytra + pronotum collar + head).
    let elytra_sdf = EllipseSdf::new(-0.7 * s, 0.0, 3.05 * s, 3.25 * s);
    let pronotum_sdf = EllipseSdf::new(2.05 * s, 0.0, 1.65 * s, 2.55 * s);
    let k_s = 0.55 * s;
    let inv_ks = 1.0 / k_s;
    let body_sd = |u: f32, v: f32| -> f32 {
        smin_inv(elytra_sdf.eval(u, v), pronotum_sdf.eval(u, v), k_s, inv_ks)
    };

    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -4.8 * s, 4.8 * s, 4.4 * s, 2, 2) else {
        return;
    };
    let rim_lim = 0.92 * s;
    let sh_lim = 0.42 * s;
    let ely_sh_sq = elytra_sdf.r2_limit(sh_lim);
    let pro_sh_sq = pronotum_sdf.r2_limit(sh_lim);
    let du_sh = -1.6 * (fr.f.x + fr.f.y);
    let dv_sh = -1.6 * (fr.p.x + fr.p.y);
    let inv_nu = 1.0 / (3.8 * s);
    let inv_nv = 1.0 / (3.3 * s);
    let seam_c = (24, 42, 24);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let d = body_sd(u, v);
            let cover = (0.52 - d * 1.18).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = (u * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.68)).clamp(0.0, 1.0);
                let lit = fr.lit(nu, nv);
                let mut lum = 0.36 + 0.40 * dome + 0.58 * lit;
                if lit > 0.42 && dome > 0.35 {
                    lum += 0.24;
                }
                let mut col = shade3(sh_c, mid_c, hi_c, lum);
                // Pronotum collar + split wing-case seam.
                if (u - 1.1 * s).abs() < 0.38 * s {
                    col = mix_rgb(col, seam_c, 0.72);
                } else if u < 1.0 * s && v.abs() < 0.32 * s {
                    col = if elite { trim_c } else { mix_rgb(col, seam_c, 0.65) };
                } else if elite && d > -0.75 * s {
                    col = mix_rgb(col, trim_c, 0.55);
                }
                let lit_col = fr.apply_hit(col);
                if cover >= 1.0 {
                    pb.plot(x, y, lit_col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.68 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, lit_col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.68));
            } else {
                let su = u + du_sh;
                let sv = v + dv_sh;
                if elytra_sdf.within_sq(su, sv, ely_sh_sq) || pronotum_sdf.within_sq(su, sv, pro_sh_sq) {
                    darken_px(pb, x, y, 0.38);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // Small dark-rimmed amber beetle eyes forward.
    let eye_c = if fr.hit > 0.7 { (22, 34, 22) } else { (255, 206, 94) };
    for side in [-1.0f32, 1.0] {
        let ep = fr.px(3.2 * s, side * 1.1 * s);
        pb.plot(ep.0, ep.1, eye_c);
    }

    // 3) The iconic white crystalline SUGAR CUBE (4×4 beveled 3D block in screen space so
    //    "carrying & lifting the sugar cube" reads cleanly at every facing angle!).
    let lift_px = if raising {
        (2.5 + 3.2 * raise_p).round() as i32
    } else {
        (fast_sin(fr.anim * 1.1).abs() * 0.6).round() as i32
    };
    let cx = fr.pos.x.round() as i32 - 2;
    let cy = fr.pos.y.round() as i32 - 4 - lift_px;

    // Tiny cradling fore-arms reaching up to hold the cube while raised.
    if raising {
        let arm_c = fr.apply_hit(if elite { trim_c } else { (92, 134, 78) });
        line_blend(pb, (cx - 1, fr.pos.y.round() as i32 - 1), (cx - 1, cy + 3), arm_c, 0.9);
        line_blend(pb, (cx + 4, fr.pos.y.round() as i32 - 1), (cx + 4, cy + 3), arm_c, 0.9);
    }

    // Contact shadow of the cube onto the beetle's back (softens as cube lifts higher).
    if !raising {
        for dx in 0..4 {
            darken_px(pb, cx + dx + 1, cy + 4, 0.45);
        }
    }

    // Dark outline around the sugar cube so pure white pops on sunlit wood planks too!
    for dx in -1..=4 {
        darken_px(pb, cx + dx, cy - 1, 0.60);
        darken_px(pb, cx + dx, cy + 4, 0.60);
    }
    for dy in 0..4 {
        darken_px(pb, cx - 1, cy + dy, 0.60);
        darken_px(pb, cx + 4, cy + dy, 0.60);
    }

    let cube_hi = fr.apply_hit(if elite { (255, 252, 232) } else { (255, 255, 252) });
    let cube_mid = fr.apply_hit(if elite { (246, 238, 206) } else { (236, 244, 238) });
    let cube_sh = fr.apply_hit((176, 194, 184));
    let cube_deep = fr.apply_hit((132, 152, 144));

    for dy in 0..4 {
        for dx in 0..4 {
            let col = if (dy == 0 && dx < 3) || (dx == 0 && dy < 3) {
                cube_hi
            } else if dy == 3 || dx == 3 {
                if dy == 3 && dx == 3 { cube_deep } else { cube_sh }
            } else if (dx + dy) == 2 {
                cube_hi // subtle sugar crystal facet glint
            } else {
                cube_mid
            };
            pb.plot(cx + dx, cy + dy, col);
        }
    }

    // Blazing mint-white healing glint rays around the raised sugar cube!
    if raising {
        let ray_c = mix_rgb((176, 255, 210), (255, 255, 250), 0.45 + 0.45 * pulse);
        let mid_x = cx + 2;
        let top_y = cy - 1;
        // Upward healing cross + sparkle bursts above the cube.
        blend_px(pb, mid_x - 1, top_y, ray_c, 0.92);
        blend_px(pb, mid_x, top_y, ray_c, 0.92);
        blend_px(pb, mid_x - 1, top_y - 1, ray_c, 0.78);
        blend_px(pb, mid_x, top_y - 1, ray_c, 0.78);
        blend_px(pb, cx - 2, cy + 1, ray_c, 0.70);
        blend_px(pb, cx + 5, cy + 1, ray_c, 0.70);
        if raise_p > 0.35 {
            pb.plot_add(mid_x - 1, top_y - 1, scale_rgb(ray_c, 0.6));
            pb.plot_add(mid_x, top_y - 1, scale_rgb(ray_c, 0.6));
            pb.plot_add(cx - 2, cy + 1, scale_rgb(ray_c, 0.45));
            pb.plot_add(cx + 5, cy + 1, scale_rgb(ray_c, 0.45));
        }
    }
}

fn draw_porter_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 4.5);
    let raising = e.w3.raise_t > 0.0;
    if elite {
        draw_w3_elite_aura(pb, fr, 2.0 * s);
    }
    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -4.0 * s, 4.0 * s, 3.8 * s, 1, 1) else {
        return;
    };
    let ramp = if elite {
        build_ramp4((26, 74, 68), (66, 168, 148), (168, 248, 224), fr)
    } else {
        build_ramp4((38, 66, 36), (112, 164, 96), (196, 238, 170), fr)
    };
    let hit_tint = fr.hit > 0.4;
    let rim_col = if hit_tint { (142, 178, 130) } else { (0, 0, 0) };
    let inv_u = 1.0 / (3.3 * s);
    let inv_v = 1.0 / (3.1 * s);
    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let nu = u * inv_u;
            let nv = v * inv_v;
            let r2 = nu * nu + nv * nv;
            if r2 <= 1.0 {
                pb.plot(x, y, ramp[ramp4_idx(fr.lit(nu, nv), r2)]);
            } else if r2 <= 1.6 {
                plot_rim(pb, x, y, rim_col, 0.60, hit_tint);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }
    // Crisp 3×3 white sugar cube with dark rim (lifted when raising) so it pops cleanly in LOD.
    let lift = if raising { 4 } else { 0 };
    let cx = fr.pos.x.round() as i32 - 1;
    let cy = fr.pos.y.round() as i32 - 4 - lift;
    for dx in -1..=3 {
        darken_px(pb, cx + dx, cy - 1, 0.58);
        darken_px(pb, cx + dx, cy + 3, 0.58);
    }
    for dy in 0..3 {
        darken_px(pb, cx - 1, cy + dy, 0.58);
        darken_px(pb, cx + 3, cy + dy, 0.58);
    }
    for dy in 0..3 {
        for dx in 0..3 {
            let c = if dx == 2 || dy == 2 { (182, 198, 188) } else { (252, 255, 250) };
            pb.plot(cx + dx, cy + dy, fr.apply_hit(c));
        }
    }
    if raising {
        pb.plot_add(cx + 1, cy - 1, (160, 245, 200));
        pb.plot_add(cx - 1, cy + 1, (140, 235, 190));
        pb.plot_add(cx + 3, cy + 1, (140, 235, 190));
    }
}

// ============================================================================
// 11. TURNKEY — Ornate Brass Skeleton-Key Drone + Crossbar Barrier (r = 4.0)
// ============================================================================

fn draw_turnkey(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let s = fr.r * (1.0 / 4.0);

    // 1) The laid crossbar (dotted tell or blazing live barrier) on the floor first.
    draw_turnkey_bar(pb, fr, e, cam);

    if elite {
        draw_w3_elite_aura(pb, fr, 2.5 * s);
    }

    let (sh_c, mid_c, hi_c) = if elite {
        ((62, 54, 46), (176, 152, 92), (255, 248, 206))
    } else {
        ((88, 66, 26), (208, 174, 82), (255, 246, 188))
    };
    let rim_col = if fr.hit > 0.4 { (196, 174, 112) } else { (0, 0, 0) };
    let hover = fast_sin(fr.anim * 1.9) * 0.45 * s;

    // 2) Sculpted antique brass skeleton key: hollow circular bow, ornate collar, machined shaft,
    //    and two crisp ward teeth jutting from the bit!
    let bow_u = -2.2 * s;
    let bow_r = 1.95 * s;
    let bow_thick = 0.72 * s;
    let shaft_sdf = CapsuleSdf::new(-0.35 * s, 0.0, 3.85 * s, 0.0, 0.68 * s);
    let collar_sdf = EllipseSdf::new(-0.25 * s, 0.0, 0.55 * s, 1.15 * s);
    let tooth1_sdf = CapsuleSdf::new(1.75 * s, -0.35 * s, 1.75 * s, -2.35 * s, 0.50 * s);
    let tooth2_sdf = CapsuleSdf::new(3.15 * s, -0.35 * s, 3.15 * s, -2.10 * s, 0.48 * s);

    let key_sd = |u: f32, v: f32| -> (f32, bool) {
        // Hollow ring SDF for the key bow (so the floor shows right through the handle!).
        let du = u - bow_u;
        let d_bow = ((du * du + v * v).sqrt() - bow_r).abs() - bow_thick;
        let d_shaft = shaft_sdf.eval(u, v).min(collar_sdf.eval(u, v));
        let d_teeth = tooth1_sdf.eval(u, v).min(tooth2_sdf.eval(u, v));
        let d_all = d_bow.min(d_shaft).min(d_teeth);
        (d_all, d_teeth < d_shaft && d_teeth < d_bow)
    };

    let Some((x0, x1, y0, y1)) = fr.bbox_oriented(pb, -5.4 * s, 5.2 * s, 3.8 * s, 3, 3) else {
        return;
    };
    let rim_lim = 0.85 * s;
    let sh_lim = 0.38 * s;
    let du_sh = -(2.0 + hover) * fr.f.x - (2.4 + hover) * fr.f.y;
    let dv_sh = -(2.0 + hover) * fr.p.x - (2.4 + hover) * fr.p.y;
    let inv_nu = 1.0 / (4.5 * s);
    let inv_nv = 1.0 / (2.8 * s);

    for y in y0..=y1 {
        let (mut u, mut v) = fr.local(x0, y);
        for x in x0..=x1 {
            let (d, is_tooth) = key_sd(u, v);
            let cover = (0.50 - d * 1.22).clamp(0.0, 1.0);
            if cover > 0.0 {
                let nu = (u * inv_nu).clamp(-1.0, 1.0);
                let nv = (v * inv_nv).clamp(-1.0, 1.0);
                let lit = fr.lit(nu, nv);
                let bevel = (-d / (0.65 * s)).clamp(0.0, 1.0);
                let mut lum = 0.34 + 0.42 * bevel + 0.62 * lit;
                if lit > 0.36 && bevel > 0.4 {
                    lum += 0.30; // polished brass metallic glint
                }
                if is_tooth {
                    lum *= 0.88; // notched steel/brass ward teeth slightly darker for depth
                }
                let mut col = shade3(sh_c, mid_c, hi_c, lum);
                if elite && (u + 0.25 * s).abs() < 0.45 * s {
                    col = mix_rgb(col, (255, 118, 228), 0.55);
                }
                let lit_col = fr.apply_hit(col);
                if cover >= 1.0 {
                    pb.plot(x, y, lit_col);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let out = mix_rgb(bg, rim_col, 0.68 * (1.0 - cover));
                    pb.plot(x, y, mix_rgb(out, lit_col, cover));
                }
            } else if d < rim_lim {
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix_rgb(bg, rim_col, 0.68));
            } else {
                let su = u + du_sh;
                let sv = v + dv_sh;
                if key_sd(su, sv).0 < sh_lim {
                    darken_px(pb, x, y, 0.36);
                }
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Glowing mechanical sensor eye set on the collar (brightens when laying a bar!).
    let pulse = 0.5 + 0.5 * fast_sin(fr.anim * 4.5);
    let eye_c = if fr.hit > 0.7 {
        (42, 32, 18)
    } else if e.w3.bar_t > 0.0 && !e.w3.bar_spent {
        mix_rgb((255, 146, 52), (255, 250, 210), 0.4 + 0.5 * pulse)
    } else if elite {
        (255, 96, 212)
    } else {
        (255, 86, 62)
    };
    let ep = fr.px(-0.25 * s, 0.0);
    pb.plot(ep.0, ep.1, eye_c);
    if e.w3.bar_t > 0.0 && !e.w3.bar_spent {
        pb.plot_add(ep.0, ep.1, scale_rgb(eye_c, 0.45));
    }
}

fn draw_turnkey_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let s = fr.r * (1.0 / 4.0);
    draw_turnkey_bar(pb, fr, e, cam);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.2 * s);
    }
    let brass = fr.apply_hit(if elite { (216, 194, 128) } else { (208, 174, 82) });
    let brass_hi = fr.apply_hit((255, 246, 188));
    let brass_lo = fr.apply_hit((112, 86, 36));
    let bow = fr.px(-2.1 * s, 0.0);
    let r = (1.85 * s).round() as i32;
    for a in 0..12 {
        let ang = a as f32 * (TAU / 12.0);
        let (sa, ca) = fast_sin_cos(ang);
        let x = bow.0 + (ca * r as f32).round() as i32;
        let y = bow.1 + (sa * r as f32).round() as i32;
        darken_px(pb, x + 1, y + 1, 0.40);
        pb.plot(x, y, if a % 3 == 0 { brass_hi } else { brass });
    }
    let s0 = fr.px(-0.3 * s, 0.0);
    let s1 = fr.px(3.7 * s, 0.0);
    thick_line(pb, s0, s1, brass_hi, brass);
    for &u_t in &[1.75f32, 3.1] {
        let a = fr.px(u_t * s, 0.0);
        let b = fr.px(u_t * s, -2.1 * s);
        thick_line(pb, a, b, brass, brass_lo);
    }
    let ep = fr.px(-0.25 * s, 0.0);
    pb.plot(ep.0, ep.1, (255, 96, 68));
}

/// A dead Wick Moth's burning fuse (its own entity; see `components::Wick`).
///
/// `t` is the fuse's remaining fraction: the braided charred wick sputters, the
/// flame flashes faster and grows hotter, and the 9 px blast ring tightens into
/// an unmistakable white-hot warning before detonation.
pub fn draw_wick(pb: &mut PixelBuffer, wick: &crate::game::ecs::components::Wick, cam: Vec2) {
    let pos = wick.pos - cam;
    let total = crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS;
    let t = if total > 0.0 {
        (1.0 - wick.fuse / total).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    if cx < -16 || cy < -16 || cx > pb.w as i32 + 16 || cy > pb.h as i32 + 16 {
        return;
    }

    // Accelerating strobe pulse as fuse burns down (slow flicker -> urgent flash!).
    let freq = 10.0 + 26.0 * t * t;
    let strobe = 0.5 + 0.5 * fast_sin(t * freq * PI);
    let urgent = t > 0.62;

    // 1) Subtle scorched ground ring under the dead moth's wick.
    for dy in -2..=2 {
        for dx in -3..=3 {
            if dx * dx + dy * dy * 2 <= 9 {
                darken_px(pb, cx + dx, cy + dy + 1, 0.36);
            }
        }
    }

    // 2) The 9 px burst hazard circle (dashed amber early -> solid urgent orange/white late!).
    let burst = crate::game::ecs::systems::enemy_wave3::WICK_BURST_RADIUS.round() as i32;
    let ring_col = if urgent {
        mix_rgb((255, 108, 42), (255, 246, 198), strobe)
    } else {
        mix_rgb((255, 168, 68), (255, 224, 132), 0.35 + 0.45 * strobe)
    };
    let ring_alpha = 0.52 + 0.42 * t;
    let n_pts = 24;
    for i in 0..n_pts {
        let show = urgent || (i % 2 == 0);
        if show {
            let a = i as f32 * (TAU / n_pts as f32);
            let (sa, ca) = fast_sin_cos(a);
            let rx = cx + (ca * burst as f32).round() as i32;
            let ry = cy + (sa * burst as f32).round() as i32;
            darken_px(pb, rx, ry + 1, 0.35);
            blend_px(pb, rx, ry, ring_col, ring_alpha);
        }
    }
    // Four inward warning ticks on the cardinal axes so the blast boundary pops at 2x zoom.
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        blend_px(pb, cx + dx * (burst - 1), cy + dy * (burst - 1), (255, 236, 176), 0.65 + 0.30 * t);
    }

    // 3) Charred braided wick cord shortening as the fuse burns down.
    let cord_len = ((1.0 - t * 0.65) * 4.0).round() as i32;
    for k in 0..=cord_len {
        let wx = cx - k;
        let wy = cy + (k / 2);
        let c = if k == 0 { (214, 96, 44) } else if k % 2 == 0 { (92, 68, 48) } else { (64, 46, 34) };
        pb.plot(wx, wy, c);
    }

    // 4) Layered flickering flame: deep orange halo -> hot gold mid -> blazing white core + sparks!
    let r = 1 + (t * 2.2 + strobe * 0.5) as i32;
    let outer = mix_rgb((255, 96, 28), (255, 188, 64), t * 0.6 + strobe * 0.4);
    let mid = mix_rgb((255, 192, 68), (255, 244, 178), t * 0.7 + strobe * 0.3);
    let core = (255, 254, 240);
    let fy = cy - 1;
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = dx * dx + dy * dy;
            let r2 = r * r;
            if d2 <= r2 {
                let c = if d2 <= 1 {
                    core
                } else if d2 * 2 <= r2 {
                    mid
                } else {
                    outer
                };
                let a = if d2 >= r2 - r { 0.62 } else { 0.96 };
                blend_px(pb, cx + dx, fy + dy, c, a);
                if d2 <= 2 {
                    pb.plot_add(cx + dx, fy + dy, scale_rgb(outer, 0.35 + 0.30 * strobe));
                }
            }
        }
    }
    // Sputtering upward spark flecks popping off the burning tip.
    let sp_y = fy - r - 1 - (strobe > 0.5) as i32;
    let sp_x = cx + if strobe > 0.5 { 1 } else { -1 };
    pb.plot_add(sp_x, sp_y, (255, 224, 128));
    if urgent {
        pb.plot_add(cx - (sp_x - cx), fy - r, (255, 186, 92));
    }
}

// ============================================================================
// WAVE 4 ENEMIES (Big Snake, House Centipede, Murder Hornet, Termite Horde)
// ============================================================================
//
// Placeholder-but-recognisable silhouettes in the same grammar as the rest of
// the file: a dark rim, top-left lighting, a hit flash and an elite aura on the
// shared paths. A Neon art pass can redraw any of these; the tells below are the
// part that must not move, because each one is a full tick of warning.

/// Filled pixel disc with a soft edge, used for the snake's body coils and the
/// horde's mass. No trig: the "circle" is the local `dx² + dy²` test.
fn disc_blend(pb: &mut PixelBuffer, cx: i32, cy: i32, rad: i32, col: Rgb, alpha: f32) {
    let r2 = rad * rad;
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            let a = if d2 >= r2 - rad { alpha * 0.6 } else { alpha };
            blend_px(pb, cx + dx, cy + dy, col, a);
        }
    }
}

/// Deterministic churn for the horde's specks: a cheap hash of the speck index
/// and the animation phase, so the mass seethes without an rng draw per pixel.
#[inline]
fn churn(i: u32, phase: f32) -> f32 {
    let h = (i.wrapping_mul(2_654_435_761) >> 8) as f32 / 16_777_216.0;
    fast_sin(h * TAU + phase) * 0.5 + 0.5
}

#[allow(clippy::needless_range_loop)]
fn draw_bigsnake(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 6.0);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.2 * s);
    }
    let (dark, mid, hi) = if elite {
        ((42, 58, 28), (150, 186, 96), (236, 246, 190))
    } else {
        ((38, 52, 26), (108, 136, 72), (196, 216, 142))
    };
    // The body, tail first so the head sits on top of its own neck.
    let n = e.w4.trail_len as usize;
    for i in (1..n).rev() {
        let p = fr.pos + (e.w4.trail[i] - e.pos);
        let t = i as f32 / (n as f32);
        let rad = ((1.0 - t * 0.45) * 3.4 * s).round().max(1.0) as i32;
        let cx = p.x.round() as i32;
        let cy = p.y.round() as i32;
        darken_px(pb, cx + 1, cy + 2, 0.30);
        disc_blend(pb, cx, cy, rad + 1, dark, 0.92);
        disc_blend(pb, cx - 1, cy - 1, rad, mid, 0.95);
        // Diamond markings along the spine.
        if i % 2 == 0 {
            blend_px(pb, cx, cy, hi, 0.85);
            blend_px(pb, cx - 1, cy, hi, 0.45);
        }
    }
    // The head: a wedge with two eyes and a flicking tongue.
    let hc = fr.apply_hit(mid);
    let hd = fr.apply_hit(dark);
    let head_r = (fr.r * 0.9).round().max(2.0) as i32;
    let (hx, hy) = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
    darken_px(pb, hx + 1, hy + 2, 0.32);
    disc_blend(pb, hx, hy, head_r + 1, hd, 0.95);
    disc_blend(pb, hx - 1, hy - 1, head_r, hc, 0.95);
    // Brow ridge across the eyes.
    let brow_a = fr.px(-head_r as f32 * 0.4, -head_r as f32 * 0.9);
    let brow_b = fr.px(-head_r as f32 * 0.4, head_r as f32 * 0.9);
    line_blend(pb, brow_a, brow_b, fr.apply_hit(dark), 0.9);
    // Two hard eye pixels looking down the facing axis.
    for side in [-1.0f32, 1.0] {
        let eye = fr.px(head_r as f32 * 0.45, side * head_r as f32 * 0.5);
        pb.plot(eye.0, eye.1, fr.apply_hit((255, 224, 96)));
    }
    // A forked tongue flicking on the tell (the lunge is coming).
    if e.w4.phase == crate::game::ecs::systems::enemy_wave4::SNAKE_PHASE_COIL
        || e.w4.phase == crate::game::ecs::systems::enemy_wave4::SNAKE_PHASE_LUNGE
    {
        let base = fr.px(head_r as f32, 0.0);
        let tip = fr.px(head_r as f32 + 3.0, 0.0);
        line_blend(pb, base, tip, (236, 84, 108), 0.95);
    }
    // Stretched-and-still pose: the body lies flatter and greyer (the punish
    // window is readable from the silhouette alone).
    if e.w4.phase == crate::game::ecs::systems::enemy_wave4::SNAKE_PHASE_STRETCHED {
        let a = fr.px(-3.0, 0.0);
        let b = fr.px(3.0, 0.0);
        line_blend(pb, a, b, (210, 196, 180), 0.55);
    }
}

#[allow(clippy::needless_range_loop)]
fn draw_bigsnake_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 6.0);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.0 * s);
    }
    let col = apply_hit(e.kind.color(), fr.hit);
    let n = e.w4.trail_len as usize;
    for i in (0..n).rev() {
        let p = fr.pos + (e.w4.trail[i] - e.pos);
        let cx = p.x.round() as i32;
        let cy = p.y.round() as i32;
        darken_px(pb, cx + 1, cy + 1, 0.30);
        pb.plot(cx, cy, col);
        if i == 0 {
            pb.plot(cx - 1, cy - 1, mix_rgb(col, (255, 244, 200), 0.35));
        }
    }
}

fn draw_centipede(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 2.4);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.2 * s);
    }
    let (dark, mid, hi, leg) = if elite {
        (
            (58, 44, 30),
            (212, 190, 146),
            (250, 240, 214),
            (236, 208, 128),
        )
    } else {
        (
            (52, 42, 30),
            (186, 166, 132),
            (238, 226, 198),
            (168, 150, 118),
        )
    };
    // Six segments down the body with two legs each, flickering fast.
    let flutter = e.w4.phase == crate::game::ecs::systems::enemy_wave4::CENTIPEDE_PHASE_FLUTTER;
    let cadence = if flutter { 34.0 } else { 22.0 };
    for i in 0..6 {
        let t = i as f32 * 0.9;
        let u = -t * s;
        let rad = (2.6 - i as f32 * 0.28).max(1.0) * s;
        let p = fr.px(u, 0.0);
        darken_px(pb, p.0 + 1, p.1 + 2, 0.28);
        disc_blend(pb, p.0, p.1, rad.round().max(1.0) as i32 + 1, fr.apply_hit(dark), 0.9);
        disc_blend(pb, p.0 - 1, p.1 - 1, rad.round().max(1.0) as i32, fr.apply_hit(mid), 0.95);
        // Legs: the wave runs down the body, faster in the flutter.
        let ph = fr.anim * cadence * 0.05 - i as f32 * 0.8;
        let sw = fast_sin(ph) * 1.5 * s;
        for side in [-1.0f32, 1.0] {
            let hip = fr.px(u, side * rad * 0.8);
            let foot = fr.px(u + sw * side, side * (rad + 1.9 * s));
            line_blend(pb, hip, foot, fr.apply_hit(leg), 0.85);
        }
    }
    // Head at the front with two long antennae and a pair of venom claws.
    let head = fr.px(1.6 * s, 0.0);
    disc_blend(pb, head.0, head.1, (3.0 * s).round().max(1.0) as i32, fr.apply_hit(hi), 0.9);
    for side in [-1.0f32, 1.0] {
        let a0 = fr.px(2.4 * s, side * 1.0 * s);
        let a1 = fr.px(4.6 * s, side * (2.4 * s + fast_sin(fr.anim * 0.4) * 0.4 * s));
        line_blend(pb, a0, a1, fr.apply_hit(leg), 0.9);
    }
    if flutter {
        // The tell: the whole body shivers, a bright seam along its spine.
        let a = fr.px(1.4 * s, 0.0);
        let b = fr.px(-5.0 * s, 0.0);
        line_blend(pb, a, b, (255, 244, 210), 0.55);
    }
}

fn draw_centipede_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 2.4);
    if elite {
        draw_w3_elite_aura(pb, fr, 1.8 * s);
    }
    let col = apply_hit(e.kind.color(), fr.hit);
    for i in 0..6 {
        let p = fr.px(-(i as f32) * 0.9 * s, 0.0);
        darken_px(pb, p.0 + 1, p.1 + 1, 0.28);
        pb.plot(p.0, p.1, if i == 0 { mix_rgb(col, (255, 244, 210), 0.4) } else { col });
    }
}

fn draw_hornet(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 2.6);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.2 * s);
    }
    let grounded = e.w4.phase == crate::game::ecs::systems::enemy_wave4::HORNET_PHASE_RECOVER;
    let (dark, stripe, light, wing) = if elite {
        (
            (34, 30, 24),
            (244, 176, 40),
            (255, 244, 200),
            (168, 214, 236),
        )
    } else {
        ((30, 26, 22), (214, 148, 40), (238, 226, 190), (140, 186, 208))
    };
    // Wings: still while grounded, a blur in the air.
    if !grounded {
        let flap = fast_sin(fr.anim * 6.0).abs() * 1.8 * s;
        for side in [-1.0f32, 1.0] {
            let a = fr.px(-0.6 * s, side * 1.2 * s);
            let b = fr.px(-2.6 * s, side * (3.2 * s + flap));
            line_blend(pb, a, b, fr.apply_hit(wing), 0.55);
        }
    }
    // Abdomen with hazard stripes, thorax, head, stinger.
    let tail = fr.px(-2.4 * s, 0.0);
    let mid = fr.px(-0.4 * s, 0.0);
    let head = fr.px(1.8 * s, 0.0);
    line_blend(pb, tail, head, fr.apply_hit(dark), 0.95);
    for k in 0..3 {
        let u = -2.0 + k as f32 * 0.8;
        let a = fr.px(u * s, -1.1 * s);
        let b = fr.px(u * s, 1.1 * s);
        line_blend(pb, a, b, fr.apply_hit(stripe), 0.9);
    }
    disc_blend(pb, mid.0, mid.1, (1.8 * s).round().max(1.0) as i32, fr.apply_hit(stripe), 0.9);
    disc_blend(pb, head.0, head.1, (1.6 * s).round().max(1.0) as i32, fr.apply_hit(light), 0.9);
    // Eyes: two dark pips that read as a face at 1x.
    for side in [-1.0f32, 1.0] {
        let eye = fr.px(2.1 * s, side * 0.7 * s);
        pb.plot(eye.0, eye.1, fr.apply_hit(dark));
    }
    // The stinger, out when it is about to dive.
    if !grounded {
        let a = fr.px(-2.6 * s, 0.0);
        let b = fr.px(-4.0 * s, 0.0);
        line_blend(pb, a, b, fr.apply_hit((246, 236, 214)), 0.9);
    } else {
        // Grounded: a squat shadow makes the punish window obvious.
        let p = fr.px(-0.4 * s, 0.0);
        line_blend(
            pb,
            (p.0 - 2, p.1 + 2),
            (p.0 + 2, p.1 + 2),
            (28, 24, 20),
            0.5,
        );
    }
}

fn draw_hornet_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 2.6);
    if elite {
        draw_w3_elite_aura(pb, fr, 1.8 * s);
    }
    let col = apply_hit(e.kind.color(), fr.hit);
    let a = fr.px(-2.0 * s, 0.0);
    let b = fr.px(1.8 * s, 0.0);
    line_blend(pb, a, b, col, 0.95);
    pb.plot(a.0 - 1, a.1, mix_rgb(col, (240, 236, 214), 0.4));
    pb.plot(b.0, b.1 - 1, mix_rgb(col, (255, 250, 230), 0.5));
}

fn draw_horde(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 3.2);
    if elite {
        draw_w3_elite_aura(pb, fr, 2.0 * s);
    }
    let tier = e.w4.tier.max(1) as f32;
    // Drawn radius: tier 2 reads as a ~7 px boiling mass, tier 0 as a coin.
    let spread =
        crate::game::ecs::systems::enemy_wave4::HORDE_DRAW_RADIUS * (0.6 + 0.2 * tier) * s;
    let (dark, mid, hi) = if elite {
        ((44, 34, 20), (232, 216, 178), (255, 250, 232))
    } else {
        ((40, 34, 24), (206, 190, 150), (246, 238, 216))
    };
    // A soft dark under-mass so the swarm reads as one body, then ~18 specks
    // churning through it.
    let c = (
        fr.pos.x.round() as i32,
        fr.pos.y.round() as i32,
    );
    let rad = spread.round().max(2.0) as i32;
    disc_blend(pb, c.0, c.1, rad, fr.apply_hit(dark), 0.55);
    disc_blend(pb, c.0 - 1, c.1 - 1, (rad - 1).max(1), fr.apply_hit(mid), 0.32);
    let count = 10 + (tier as u32) * 5;
    for i in 0..count {
        let a = churn(i, fr.anim * 0.6) * std::f32::consts::TAU;
        let rr = spread * (0.35 + 0.65 * churn(i + 41, fr.anim * 0.9));
        let p = Vec2::new(fr.pos.x + a.cos() * rr, fr.pos.y + a.sin() * rr);
        let px = p.x.round() as i32;
        let py = p.y.round() as i32;
        let body = if i % 4 == 0 { hi } else { mid };
        blend_px(pb, px, py, fr.apply_hit(body), 0.95);
        if i % 3 == 0 {
            darken_px(pb, px + 1, py + 1, 0.22);
        }
    }
}

fn draw_horde_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    let s = fr.r * (1.0 / 3.2);
    if elite {
        draw_w3_elite_aura(pb, fr, 1.8 * s);
    }
    let col = apply_hit(e.kind.color(), fr.hit);
    let c = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
    let rad = (crate::game::ecs::systems::enemy_wave4::HORDE_DRAW_RADIUS * 0.5 * s)
        .round()
        .max(1.0) as i32;
    disc_blend(pb, c.0, c.1, rad, col, 0.75);
    pb.plot(c.0 - 1, c.1 - 1, mix_rgb(col, (255, 250, 232), 0.45));
}

// ---- Wave-4 floor tells ---------------------------------------------------

/// The snake's lunge lane: a dotted 44 px line the length of the lunge, with a
/// bright head marker. The body's own silhouette shows the coil.
fn draw_snake_warn(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy) {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    if e.w4.phase != w4::SNAKE_PHASE_COIL {
        return;
    }
    let progress = 1.0 - (e.w4.t / w4::SNAKE_TELL_SECS).clamp(0.0, 1.0);
    let len = w4::SNAKE_WARN_LINE;
    let steps = (len / 3.0) as i32;
    let core = mix_rgb((255, 232, 150), (255, 96, 96), progress);
    for k in 0..=steps {
        let u = 2.0 + k as f32 * 3.0;
        if u > len {
            break;
        }
        // Dotted, and the dots fill in as the tell runs down.
        if (k as f32 / steps as f32) > progress + 0.15 {
            continue;
        }
        let p = fr.px(u, 0.0);
        blend_px(pb, p.0, p.1, core, 0.85);
        let edge = fr.px(u, 0.9);
        blend_px(pb, edge.0, edge.1, core, 0.40);
        let edge = fr.px(u, -0.9);
        blend_px(pb, edge.0, edge.1, core, 0.40);
    }
    // The lunge's landing point.
    let end = fr.px(len, 0.0);
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        blend_px(pb, end.0 + dx, end.1 + dy, core, 0.75);
    }
}

/// The centipede's flutter line: a short bright lane along the locked dart.
fn draw_centipede_warn(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy) {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    if e.w4.phase != w4::CENTIPEDE_PHASE_FLUTTER {
        return;
    }
    let progress = 1.0 - (e.w4.t / w4::CENTIPEDE_FLUTTER_SECS).clamp(0.0, 1.0);
    let col = mix_rgb((255, 236, 168), (255, 120, 96), progress);
    let len = w4::CENTIPEDE_DART_SPEED * w4::CENTIPEDE_DART_SECS;
    let steps = (len / 4.0) as i32;
    for k in 0..=steps {
        let u = 2.0 + k as f32 * 4.0;
        if u > len {
            break;
        }
        if (k as f32 / steps.max(1) as f32) > progress + 0.2 {
            continue;
        }
        let p = fr.px(u, 0.0);
        blend_px(pb, p.0, p.1, col, 0.80);
    }
}

/// The hornet's dive line and the tightening ring: it locks the aim at the tell
/// and the ring closes as the dive approaches. Grounded recovery shows a plain
/// ring so "punish now" is readable from the floor.
fn draw_hornet_warn(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy) {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    match e.w4.phase {
        w4::HORNET_PHASE_CHARGE => {
            let progress = 1.0 - (e.w4.t / w4::HORNET_TELL_SECS).clamp(0.0, 1.0);
            let col = mix_rgb((255, 232, 150), (255, 96, 96), progress);
            let len = w4::HORNET_DIVE_SPEED * w4::HORNET_DIVE_SECS;
            let steps = (len / 4.0) as i32;
            for k in 0..=steps {
                let u = 2.0 + k as f32 * 4.0;
                if u > len {
                    break;
                }
                if (k as f32 / steps.max(1) as f32) > progress + 0.15 {
                    continue;
                }
                let p = fr.px(u, 0.0);
                blend_px(pb, p.0, p.1, col, 0.85);
                blend_px(pb, p.0, p.1 + 1, col, 0.35);
            }
        }
        w4::HORNET_PHASE_RECOVER => {
            // The punish window: a calm white ring where it sits.
            let r = (e.radius + 4.0).round() as i32;
            let c = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
            for k in 0..24 {
                let a = k as f32 / 24.0 * std::f32::consts::TAU;
                let p = (c.0 + (a.cos() * r as f32).round() as i32,
                         c.1 + (a.sin() * r as f32).round() as i32);
                blend_px(pb, p.0, p.1, (236, 248, 255), 0.55);
            }
        }
        _ => {}
    }
}

/// The horde's split warning: the mass glows pale for the ticks before it tears
/// in two.
fn draw_horde_split_warn(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy) {
    if e.w4.split_pending == 0 {
        return;
    }
    let c = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
    let r = (crate::game::ecs::systems::enemy_wave4::HORDE_DRAW_RADIUS * 0.8).round() as i32;
    line_blend(pb, (c.0 - r, c.1), (c.0 + r, c.1), (255, 246, 214), 0.6);
    line_blend(pb, (c.0, c.1 - r), (c.0, c.1 + r), (255, 246, 214), 0.6);
}

// ---- Lilliputians --------------------------------------------------------

/// One tiny person, about five pixels tall: a head, a pale facility coat, two
/// legs, and the one prop that says which job it has (a levelled spear, a bent
/// bow, a coil of rope, or the squad's little flag). At 1x the silhouette is a
/// person; at zoom-out the LOD path leaves a bright dot, so a squad reads as a
/// formation.
fn draw_lilliputian(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    let s = fr.r * 0.75;
    if elite {
        draw_w3_elite_aura(pb, fr, 2.0 * s);
    }
    let coat = if elite {
        (226, 232, 240)
    } else {
        e.kind.color()
    };
    let coat = fr.apply_hit(coat);
    let skin = fr.apply_hit((228, 190, 152));
    let dark = fr.apply_hit((52, 58, 70));
    let steel = fr.apply_hit((212, 220, 232));

    // Feet, body, head (drawn in the local frame so it faces the rat).
    let feet = fr.px(-0.4 * s, 0.0);
    let body = fr.px(0.6 * s, 0.0);
    let head = fr.px(1.9 * s, 0.0);
    darken_px(pb, feet.0 + 1, feet.1 + 1, 0.25);
    // Legs.
    for side in [-1.0f32, 1.0] {
        let knee = fr.px(-0.2 * s, side * 0.5 * s);
        let foot = fr.px(0.2 * s + fast_sin(fr.anim * 2.2 + side) * 0.3 * s, side * 0.6 * s);
        line_blend(pb, knee, foot, dark, 0.9);
    }
    // Coat: two stacked pixels, widest at the shoulders.
    line_blend(pb, feet, body, coat, 0.98);
    for side in [-1.0f32, 1.0] {
        let sh = fr.px(0.8 * s, side * 0.9 * s);
        blend_px(pb, sh.0, sh.1, coat, 0.85);
    }
    // Head + a little hard hat (the facility's, shrunk with them).
    blend_px(pb, head.0, head.1, skin, 0.98);
    let hat = fr.px(2.2 * s, 0.0);
    blend_px(pb, hat.0, hat.1, steel, 0.85);

    // The role prop.
    match e.w4.role {
        w4::LILLI_ROLE_SPEAR => {
            let levelled = e.w4.phase != w4::LILLI_PHASE_WALK;
            let (a, b) = if levelled {
                (fr.px(1.0 * s, 0.8 * s), fr.px(4.6 * s, 0.6 * s))
            } else {
                (fr.px(1.2 * s, 0.8 * s), fr.px(2.4 * s, 2.2 * s))
            };
            line_blend(pb, a, b, steel, 0.95);
            blend_px(pb, b.0, b.1, (255, 250, 236), if levelled { 0.95 } else { 0.6 });
        }
        w4::LILLI_ROLE_ARCHER => {
            let kneel = e.w4.phase == w4::LILLI_PHASE_TELL;
            let nock = fr.px(2.2 * s, 0.9 * s);
            let tip_a = fr.px(2.6 * s, 1.9 * s);
            let tip_b = fr.px(2.6 * s, -0.1 * s);
            line_blend(pb, nock, tip_a, (166, 122, 74), 0.95);
            line_blend(pb, nock, tip_b, (166, 122, 74), 0.95);
            // The string, drawn back when the arrow is nocked.
            let pull = if kneel { 0.9 * s } else { 0.2 * s };
            let draw = fr.px(2.2 * s - pull, 0.9 * s);
            line_blend(pb, tip_a, draw, (232, 238, 246), 0.7);
            line_blend(pb, draw, tip_b, (232, 238, 246), 0.7);
        }
        w4::LILLI_ROLE_ROPE => {
            let coil = fr.px(0.4 * s, 1.1 * s);
            let taut = e.w4.phase != w4::LILLI_PHASE_WALK;
            let end = if taut {
                fr.px(3.0 * s, 1.3 * s)
            } else {
                fr.px(1.4 * s, 1.7 * s)
            };
            line_blend(pb, coil, end, (196, 170, 118), if taut { 0.95 } else { 0.7 });
        }
        _ => {
            // The squad's little flag: a pole and a pale pennant.
            let base = fr.px(0.8 * s, -1.0 * s);
            let top = fr.px(1.9 * s, -1.0 * s);
            line_blend(pb, base, top, (128, 110, 82), 0.9);
            let wave = fast_sin(fr.anim * 2.0) * 0.5;
            let flag = fr.px(2.6 * s, -1.4 * s + wave);
            blend_px(pb, flag.0, flag.1, (232, 96, 108), 0.92);
        }
    }
    // Panic: the whole tiny body leans into the run.
    if e.w4.phase == w4::LILLI_PHASE_PANIC {
        let a = fr.px(-0.6 * s, 0.0);
        let b = fr.px(1.0 * s, 0.0);
        line_blend(pb, a, b, (236, 248, 255), 0.35);
    }
}

fn draw_lilliputian_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, elite: bool) {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    if elite {
        draw_w3_elite_aura(pb, fr, 2.0);
    }
    let col = apply_hit(e.kind.color(), fr.hit);
    let p = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
    darken_px(pb, p.0 + 1, p.1 + 1, 0.30);
    pb.plot(p.0, p.1, col);
    // A one-pixel role tint so a distant squad still reads as mixed.
    let tint = match e.w4.role {
        w4::LILLI_ROLE_ARCHER => (166, 122, 74),
        w4::LILLI_ROLE_ROPE => (196, 170, 118),
        w4::LILLI_ROLE_LEADER => (232, 96, 108),
        _ => (212, 220, 232),
    };
    pb.plot(p.0, p.1 - 1, tint);
}

/// The squad's floor tells: the levelled spear line, the archer's marked arc,
/// the rope tightening around the rat and the panic wheel.
fn draw_lilliputian_warn(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy) {
    use crate::game::ecs::systems::enemy_wave4 as w4;
    match (e.w4.role, e.w4.phase) {
        (w4::LILLI_ROLE_SPEAR, w4::LILLI_PHASE_TELL)
        | (w4::LILLI_ROLE_LEADER, w4::LILLI_PHASE_TELL) => {
            // The points glint: a short line out past the 44 px reach.
            let progress = 1.0 - (e.w4.t / w4::SPEAR_TELL_SECS).clamp(0.0, 1.0);
            let col = mix_rgb((255, 236, 170), (255, 120, 96), progress);
            let a = fr.px(3.0, 0.0);
            let b = fr.px(w4::SPEAR_REACH, 0.0);
            line_blend(pb, a, b, col, 0.55 + 0.35 * progress);
        }
        (w4::LILLI_ROLE_ARCHER, w4::LILLI_PHASE_TELL) => {
            // The marked arc the pellets will cross.
            let progress = 1.0 - (e.w4.t / w4::ARCHER_TELL_SECS).clamp(0.0, 1.0);
            let col = mix_rgb((255, 236, 170), (255, 120, 96), progress);
            let reach = w4::ARCHER_BULLET_SPEED * 0.9;
            let half = w4::ARCHER_SPREAD;
            for k in 0..=6 {
                let t = k as f32 / 6.0;
                let a = -half + 2.0 * half * t;
                let dir = e.w4.dir.rotate(a);
                let p = fr.pos + dir * reach * (0.25 + 0.75 * progress);
                blend_px(
                    pb,
                    p.x.round() as i32,
                    p.y.round() as i32,
                    col,
                    0.75,
                );
            }
        }
        (w4::LILLI_ROLE_ROPE, w4::LILLI_PHASE_TELL) => {
            // The rope tightening: a closing ring with four spokes.
            let progress = 1.0 - (e.w4.t / w4::ROPE_TIE_SECS).clamp(0.0, 1.0);
            let r = w4::ROPE_REACH * (1.0 - 0.55 * progress);
            let c = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
            let steps = 20;
            let col = mix_rgb((214, 190, 140), (255, 236, 180), progress);
            for k in 0..steps {
                let a = k as f32 / steps as f32 * std::f32::consts::TAU;
                let p = (
                    c.0 + (a.cos() * r).round() as i32,
                    c.1 + (a.sin() * r).round() as i32,
                );
                blend_px(pb, p.0, p.1, col, 0.30 + 0.55 * progress);
            }
        }
        (w4::LILLI_ROLE_ROPE, w4::LILLI_PHASE_ACTION) => {
            // The knot is closed: a taut ring the rat has to dash out of.
            let c = (fr.pos.x.round() as i32, fr.pos.y.round() as i32);
            let r = (w4::ROPE_REACH * 0.45).round() as i32;
            for k in 0..20 {
                let a = k as f32 / 20.0 * std::f32::consts::TAU;
                let p = (
                    c.0 + (a.cos() * r as f32).round() as i32,
                    c.1 + (a.sin() * r as f32).round() as i32,
                );
                blend_px(pb, p.0, p.1, (255, 244, 210), 0.85);
            }
        }
        _ => {}
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

    fn fnv1a_pixels(pb: &PixelBuffer) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let (r, g, b) = pb.pixel_at(x, y);
                for byte in [r, g, b] {
                    h ^= byte as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }

    /// Deterministic self-contained bestiary sheet (no external art_floor/art_rat
    /// dependency so the pinned hash only tracks `art_enemy.rs`).
    fn render_bestiary_pb(draw: impl Fn(&mut PixelBuffer, &Enemy, Vec2, Rgb)) -> PixelBuffer {
        let w = 260usize;
        let h = 128usize;
        let mut pb = PixelBuffer::new(w, h);
        pb.clear((18, 28, 26));
        let cam = Vec2::new(300.0, 300.0);
        let acc = (90, 220, 200);

        // Row 1 (y=24): Lineup facing down-right.
        let y1 = cam.y + 24.0;
        let lineup: [(EnemyKind, bool, f32); 6] = [
            (EnemyKind::Skitterer, false, 38.0),
            (EnemyKind::Bat, false, 68.0),
            (EnemyKind::Cat, false, 100.0),
            (EnemyKind::Spitter, false, 134.0),
            (EnemyKind::Brute, false, 174.0),
            (EnemyKind::Brute, true, 218.0),
        ];
        for &(kind, elite, x) in &lineup {
            let mut e = make_enemy(kind, Vec2::new(cam.x + x, y1), 0.7);
            e.elite = elite;
            draw(&mut pb, &e, cam, acc);
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
            draw(&mut pb, &e, cam, acc);
        }
        let mut bat1 = make_enemy(EnemyKind::Bat, Vec2::new(cam.x + 64.0, y2 - 3.0), -0.5);
        bat1.anim = 1.5;
        draw(&mut pb, &bat1, cam, acc);
        let mut bat2 = make_enemy(EnemyKind::Bat, Vec2::new(cam.x + 84.0, y2 + 3.0), 2.5);
        bat2.anim = -1.5;
        draw(&mut pb, &bat2, cam, acc);

        let mut cat = make_enemy(EnemyKind::Cat, Vec2::new(cam.x + 114.0, y2), 2.8);
        cat.contact_cd = 0.5;
        cat.anim = 2.1;
        draw(&mut pb, &cat, cam, acc);

        let mut sp1 = make_enemy(EnemyKind::Spitter, Vec2::new(cam.x + 144.0, y2 - 4.0), 0.0);
        sp1.fire_cd = 0.35;
        draw(&mut pb, &sp1, cam, acc);
        let mut sp2 = make_enemy(EnemyKind::Spitter, Vec2::new(cam.x + 144.0, y2 + 12.0), 0.0);
        sp2.fire_cd = 0.05;
        draw(&mut pb, &sp2, cam, acc);

        let mut br_dmg = make_enemy(EnemyKind::Brute, Vec2::new(cam.x + 178.0, y2), -0.6);
        br_dmg.hp = br_dmg.max_hp * 0.28;
        draw(&mut pb, &br_dmg, cam, acc);
        let mut br_hit = make_enemy(EnemyKind::Brute, Vec2::new(cam.x + 218.0, y2), -2.4);
        br_hit.flash = 0.10;
        br_hit.elite = true;
        draw(&mut pb, &br_hit, cam, acc);

        // Row 3 (y=98): Boss showcase — full HP, telegraphing nova, enraged low HP.
        let y3 = cam.y + 98.0;
        let mut boss1 = make_enemy(EnemyKind::Boss, Vec2::new(cam.x + 48.0, y3), 0.55);
        boss1.hp = boss1.max_hp * 0.92;
        boss1.fire_cd = 1.0;
        draw(&mut pb, &boss1, cam, acc);

        let mut boss2 = make_enemy(EnemyKind::Boss, Vec2::new(cam.x + 130.0, y3), 1.35);
        boss2.hp = boss2.max_hp * 0.55;
        boss2.fire_cd = 0.06;
        boss2.anim = 1.2;
        draw(&mut pb, &boss2, cam, acc);

        let mut boss3 = make_enemy(EnemyKind::Boss, Vec2::new(cam.x + 212.0, y3), -2.2);
        boss3.hp = boss3.max_hp * 0.22;
        boss3.fire_cd = 0.22;
        boss3.anim = 2.6;
        draw(&mut pb, &boss3, cam, acc);

        pb
    }

    /// Pins the exact FNV-1a 64-bit hash of the full-detail bestiary sheet so
    /// any visual change to `draw_enemy` must be deliberate.
    #[test]
    fn bestiary_sheet_full_detail_hash_is_pinned() {
        const PINNED_BESTIARY_HASH: u64 = 0x44c7_96bc_47ed_af91;
        let pb = render_bestiary_pb(draw_enemy);
        let actual = fnv1a_pixels(&pb);
        assert_eq!(
            actual, PINNED_BESTIARY_HASH,
            "Full-detail bestiary sheet changed (expected 0x{PINNED_BESTIARY_HASH:016x}, got 0x{actual:016x}); inspect RATRUN_DUMP=enemies before updating the pin"
        );
    }

    #[test]
    fn lookup_tables_match_std_math() {
        for i in -100..=200 {
            let x = i as f32 * 0.137;
            let (fs, fc) = fast_sin_cos(x);
            assert!(
                (fs - x.sin()).abs() < 2e-4,
                "fast_sin({x}) = {fs} vs {}",
                x.sin()
            );
            assert!(
                (fc - x.cos()).abs() < 2e-4,
                "fast_cos({x}) = {fc} vs {}",
                x.cos()
            );
            assert!((fast_sin(x) - fs).abs() < 1e-6);
        }
        for i in 0..=32 {
            let t = i as f32 / 32.0;
            let (el, ch) = bat_wing_profile(t);
            let exp_el = if t < 0.48 {
                let k = t / 0.48;
                -0.25 + 1.85 * k * k * (3.0 - 2.0 * k)
            } else {
                let k = (t - 0.48) / 0.52;
                1.6 - 2.15 * k.powf(1.15)
            };
            let exp_ch = (2.45 * (1.0 - t * 0.84)
                - 0.85 * (t * TAU).sin().abs() * (1.0 - t * 0.25))
            .max(0.28);
            assert!((el - exp_el).abs() < 5e-3, "bat elbow t={t}: {el} vs {exp_el}");
            assert!((ch - exp_ch).abs() < 5e-3, "bat chord t={t}: {ch} vs {exp_ch}");

            let f_env = boss_wing_fore_env(t);
            let exp_f = (t.powf(0.42) * (1.0 - t).powf(0.6) * 2.2).clamp(0.0, 1.0);
            assert!((f_env - exp_f).abs() < 1e-2, "boss fore t={t}: {f_env} vs {exp_f}");

            let h_env = boss_wing_hind_env(t);
            let exp_h = (t.powf(0.45) * (1.0 - t).powf(0.6) * 2.2).clamp(0.0, 1.0);
            assert!((h_env - exp_h).abs() < 1e-2, "boss hind t={t}: {h_env} vs {exp_h}");

            let sp = spitter_wing_profile(t);
            let exp_sp = (t * (1.0 - t) * 4.0).clamp(0.0, 1.0).sqrt();
            assert!((sp - exp_sp).abs() < 1e-2, "spitter wing t={t}: {sp} vs {exp_sp}");
        }
        for (k, &(ca, sa)) in NOVA_10.iter().enumerate() {
            let a = k as f32 / 10.0 * TAU;
            assert!((ca - a.cos()).abs() < 1e-5);
            assert!((sa - a.sin()).abs() < 1e-5);
        }
    }

    #[test]
    fn lod_matches_full_detail_within_tolerance() {
        let bg = (18, 28, 26);
        let acc = (90, 220, 200);
        for &(kind, elite, max_mae, min_overlap) in &[
            (EnemyKind::Skitterer, false, 18.0f32, 0.55f32),
            (EnemyKind::Bat, false, 18.0, 0.50),
            (EnemyKind::Cat, false, 24.0, 0.58),
            (EnemyKind::Spitter, false, 22.0, 0.55),
            (EnemyKind::Brute, false, 26.0, 0.65),
            (EnemyKind::Brute, true, 28.0, 0.65),
            (EnemyKind::Boss, false, 32.0, 0.65),
        ] {
            let mut full_pb = PixelBuffer::new(48, 48);
            let mut lod_pb = PixelBuffer::new(48, 48);
            full_pb.clear(bg);
            lod_pb.clear(bg);
            let mut e = make_enemy(kind, Vec2::new(24.0, 24.0), 0.7);
            e.elite = elite;
            e.fire_cd = 0.2;
            draw_enemy(&mut full_pb, &e, Vec2::ZERO, acc);
            draw_enemy_lod(&mut lod_pb, &e, Vec2::ZERO, acc);

            // Compare inside active silhouette union (excluding untouched bg).
            let mut union_px = 0usize;
            let mut both_px = 0usize;
            let mut abs_err_sum = 0f64;
            let mut full_rgb = [0f64; 3];
            let mut lod_rgb = [0f64; 3];
            let mut full_cnt = 0usize;
            let mut lod_cnt = 0usize;

            for y in 0..48 {
                for x in 0..48 {
                    let a = full_pb.pixel_at(x, y);
                    let b = lod_pb.pixel_at(x, y);
                    let a_on = a != bg;
                    let b_on = b != bg;
                    if a_on {
                        full_cnt += 1;
                        full_rgb[0] += a.0 as f64;
                        full_rgb[1] += a.1 as f64;
                        full_rgb[2] += a.2 as f64;
                    }
                    if b_on {
                        lod_cnt += 1;
                        lod_rgb[0] += b.0 as f64;
                        lod_rgb[1] += b.1 as f64;
                        lod_rgb[2] += b.2 as f64;
                    }
                    if a_on || b_on {
                        union_px += 1;
                        if a_on && b_on {
                            both_px += 1;
                        }
                        abs_err_sum += (a.0 as f64 - b.0 as f64).abs()
                            + (a.1 as f64 - b.1 as f64).abs()
                            + (a.2 as f64 - b.2 as f64).abs();
                    }
                }
            }
            let overlap = both_px as f32 / union_px.max(1) as f32;
            let mae_box = (abs_err_sum / (48.0 * 48.0 * 3.0)) as f32;
            let mean_color_err = [0, 1, 2]
                .iter()
                .map(|&c| {
                    ((full_rgb[c] / full_cnt.max(1) as f64)
                        - (lod_rgb[c] / lod_cnt.max(1) as f64))
                    .abs()
                })
                .sum::<f64>()
                / 3.0;

            assert!(
                overlap >= min_overlap,
                "{kind:?} (elite={elite}): LOD silhouette overlap {overlap:.2} < {min_overlap:.2}"
            );
            assert!(
                mae_box <= max_mae,
                "{kind:?} (elite={elite}): LOD MAE {mae_box:.2} > {max_mae:.2}"
            );
            assert!(
                mean_color_err < 35.0,
                "{kind:?} (elite={elite}): LOD mean colour drift {mean_color_err:.1} >= 35"
            );

            // Hit flash must brighten LOD too.
            let mut hit_pb = PixelBuffer::new(48, 48);
            hit_pb.clear(bg);
            e.flash = 0.12;
            draw_enemy_lod(&mut hit_pb, &e, Vec2::ZERO, acc);
            let sum_rgb = |pb: &PixelBuffer| -> u64 {
                (0..48)
                    .flat_map(|y| (0..48).map(move |x| pb.pixel_at(x, y)))
                    .map(|c| c.0 as u64 + c.1 as u64 + c.2 as u64)
                    .sum()
            };
            assert!(
                sum_rgb(&hit_pb) > sum_rgb(&lod_pb) + 150,
                "{kind:?} (elite={elite}): LOD hit flash must brighten"
            );

            // Facing change must visibly rotate the LOD sprite.
            let mut rot_pb = PixelBuffer::new(48, 48);
            rot_pb.clear(bg);
            e.flash = 0.0;
            e.facing = Vec2::from_angle(0.7 + PI);
            draw_enemy_lod(&mut rot_pb, &e, Vec2::ZERO, acc);
            assert_ne!(
                fnv1a_pixels(&lod_pb),
                fnv1a_pixels(&rot_pb),
                "{kind:?}: LOD facing must stay legible"
            );
        }
    }

    #[test]
    fn crowd_and_subpixel_paths_work_cleanly() {
        let mut pb = PixelBuffer::new(48, 48);
        pb.clear((18, 28, 26));
        let mut tiny = make_enemy(EnemyKind::Skitterer, Vec2::new(24.0, 24.0), 0.5);
        tiny.radius = 0.4;
        draw_enemy(&mut pb, &tiny, Vec2::ZERO, (90, 220, 200));
        assert_ne!(pb.pixel_at(24, 24), (18, 28, 26));

        let mut small = make_enemy(EnemyKind::Bat, Vec2::new(12.0, 12.0), 0.5);
        small.radius = 1.4;
        draw_enemy(&mut pb, &small, Vec2::ZERO, (90, 220, 200));
        assert_ne!(pb.pixel_at(12, 12), (18, 28, 26));

        let e = make_enemy(EnemyKind::Skitterer, Vec2::new(36.0, 36.0), 0.0);
        let mut pb_full = PixelBuffer::new(48, 48);
        let mut pb_crowd = PixelBuffer::new(48, 48);
        pb_full.clear((18, 28, 26));
        pb_crowd.clear((18, 28, 26));
        draw_enemy_crowd(&mut pb_full, &e, Vec2::ZERO, (90, 220, 200), 10);
        draw_enemy_crowd(
            &mut pb_crowd,
            &e,
            Vec2::ZERO,
            (90, 220, 200),
            CROWD_LOD_THRESHOLD + 50,
        );
        assert_ne!(pb_full.pixel_at(36, 36), (18, 28, 26));
        assert_ne!(pb_crowd.pixel_at(36, 36), (18, 28, 26));
    }

    /// Side-by-side Full Detail (top), LOD (middle), and 3x diff heatmap (bottom):
    /// `RATRUN_DUMP=enemies_lod cargo test dump_lod_comparison_sheet` writes `/tmp/ratrun_enemies_lod.ppm`.
    #[test]
    fn dump_lod_comparison_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("enemies_lod") {
            return;
        }
        let full = render_bestiary_pb(draw_enemy);
        let lod = render_bestiary_pb(draw_enemy_lod);
        let w = full.w;
        let h = full.h;
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();
        for pb in [&full, &lod] {
            for y in 0..h {
                for x in 0..w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                let a = full.pixel_at(x, y);
                let b = lod.pixel_at(x, y);
                let dr = (a.0 as i32 - b.0 as i32).unsigned_abs().min(85) as u8 * 3;
                let dg = (a.1 as i32 - b.1 as i32).unsigned_abs().min(85) as u8 * 3;
                let db = (a.2 as i32 - b.2 as i32).unsigned_abs().min(85) as u8 * 3;
                out.extend_from_slice(&[dr, dg, db]);
            }
        }
        std::fs::write("/tmp/ratrun_enemies_lod.ppm", out).unwrap();
    }

    #[test]
    fn benchmark_draw_enemy_per_kind() {
        use std::hint::black_box;
        use std::time::Instant;

        let pb_ref = render_bestiary_pb(draw_enemy);
        println!("bestiary_hash = 0x{:016x}", fnv1a_pixels(&pb_ref));

        let mut pb = PixelBuffer::new(160, 168);
        pb.clear((18, 28, 26));
        let acc = (90, 220, 200);
        // Baseline timings measured on unmodified `art_enemy.rs` (dev opt=1 / release opt=3):
        let cases: [(&str, EnemyKind, bool, f32, f32, f64, f64); 7] = [
            ("Skitterer", EnemyKind::Skitterer, false, 1.5, 0.0, 2.547, 2.912),
            ("Bat", EnemyKind::Bat, false, 1.5, 0.0, 3.552, 3.508),
            ("Cat", EnemyKind::Cat, false, 1.5, 0.0, 10.674, 9.224),
            ("Spitter", EnemyKind::Spitter, false, 0.35, 0.0, 4.779, 3.563),
            ("Brute", EnemyKind::Brute, false, 1.5, 0.0, 13.985, 12.373),
            ("EliteBrute", EnemyKind::Brute, true, 1.5, 0.0, 16.362, 13.895),
            ("Boss", EnemyKind::Boss, false, 0.15, 0.0, 124.580, 86.908),
        ];

        println!(
            "{:<12} | {:>10} | {:>10} | {:>10} | {:>10}",
            "Kind", "Before(us)", "Full(us)", "LOD(us)", "Speedup"
        );
        for &(label, kind, elite, fire_cd, flash, before_dev, before_rel) in &cases {
            let before = if cfg!(debug_assertions) {
                before_dev
            } else {
                before_rel
            };
            let n = if kind == EnemyKind::Boss { 400 } else { 4000 };
            let enemies: Vec<Enemy> = (0..16)
                .map(|i| {
                    let mut e = make_enemy(
                        kind,
                        Vec2::new(80.0 + (i % 4) as f32 * 0.25, 84.0 + (i / 4) as f32 * 0.25),
                        i as f32 * (TAU / 16.0),
                    );
                    e.elite = elite;
                    e.fire_cd = fire_cd;
                    e.flash = flash;
                    e.anim = i as f32 * 0.41;
                    e
                })
                .collect();
            for e in &enemies {
                draw_enemy(&mut pb, black_box(e), Vec2::ZERO, acc);
            }
            let t0 = Instant::now();
            for i in 0..n {
                draw_enemy(&mut pb, black_box(&enemies[i & 15]), Vec2::ZERO, acc);
            }
            let full_us = t0.elapsed().as_secs_f64() * 1e6 / n as f64;

            for e in &enemies {
                draw_enemy_lod(&mut pb, black_box(e), Vec2::ZERO, acc);
            }
            let t1 = Instant::now();
            for i in 0..n {
                draw_enemy_lod(&mut pb, black_box(&enemies[i & 15]), Vec2::ZERO, acc);
            }
            let lod_us = t1.elapsed().as_secs_f64() * 1e6 / n as f64;

            println!(
                "{label:<12} | {before:10.3} | {full_us:10.3} | {lod_us:10.3} | {:>4.2}x / {:>4.2}x",
                before / full_us,
                before / lod_us
            );
        }

        // Off-screen early-out benchmark
        let off_n = 20_000;
        let off_e = make_enemy(EnemyKind::Cat, Vec2::new(-40.0, -40.0), 0.7);
        let t0 = Instant::now();
        for _ in 0..off_n {
            draw_enemy(&mut pb, black_box(&off_e), Vec2::ZERO, acc);
        }
        let off_us = t0.elapsed().as_secs_f64() * 1e6 / off_n as f64;
        println!("{:<12} | {:10.4} | {:10.4}", "OffScreenCat", 0.250, off_us);

        // 600-enemy cap realistic mix (same 160x168 view)
        let mut rng = StdRng::seed_from_u64(123);
        let mix_kinds = [
            EnemyKind::Skitterer,
            EnemyKind::Skitterer,
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
        ];
        let swarm: Vec<Enemy> = (0..600)
            .map(|i| {
                let kind = if i == 599 {
                    EnemyKind::Boss
                } else {
                    mix_kinds[i % mix_kinds.len()]
                };
                let x = ((i * 29) % 150) as f32 + 5.0;
                let y = ((i * 47) % 158) as f32 + 5.0;
                let mut e = Enemy::spawn(kind, Vec2::new(x, y), 1.0, &mut rng);
                e.facing = Vec2::from_angle(i as f32 * 0.37);
                e.anim = i as f32 * 0.2;
                e.elite = kind == EnemyKind::Brute && i % 16 == 0;
                e.fire_cd = if kind.ranged() && i % 3 == 0 { 0.2 } else { 1.5 };
                e
            })
            .collect();
        let frames = 20;
        let t0 = Instant::now();
        for _ in 0..frames {
            for e in &swarm {
                draw_enemy(&mut pb, black_box(e), Vec2::ZERO, acc);
            }
        }
        let full_frame_us = t0.elapsed().as_secs_f64() * 1e6 / frames as f64;

        let t1 = Instant::now();
        for _ in 0..frames {
            for e in &swarm {
                draw_enemy_crowd(&mut pb, black_box(e), Vec2::ZERO, acc, swarm.len());
            }
        }
        let crowd_frame_us = t1.elapsed().as_secs_f64() * 1e6 / frames as f64;
        println!(
            "600-swarm   : full={full_frame_us:6.1} us/frame, crowd-LOD={crowd_frame_us:6.1} us/frame"
        );
        black_box(pb.pixel_at(80, 84));
    }

    #[test]
    fn wave3_enemies_hit_flash_elite_and_lod_work_cleanly() {
        let w3_kinds = [
            EnemyKind::Pillbug,
            EnemyKind::BellFlea,
            EnemyKind::WickMoth,
            EnemyKind::Porter,
            EnemyKind::Turnkey,
        ];
        let bg = (16, 22, 20);
        let acc = (90, 220, 200);
        for &kind in &w3_kinds {
            let sum_pb = |e: &Enemy, lod: bool| -> (u64, u64) {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear(bg);
                if lod {
                    draw_enemy_lod(&mut pb, e, Vec2::ZERO, acc);
                } else {
                    draw_enemy(&mut pb, e, Vec2::ZERO, acc);
                }
                let mut sum = 0u64;
                for y in 0..pb.h {
                    for x in 0..pb.w {
                        let c = pb.pixel_at(x, y);
                        sum += c.0 as u64 + c.1 as u64 + c.2 as u64;
                    }
                }
                (sum, fnv1a_pixels(&pb))
            };
            let base = make_enemy(kind, Vec2::new(32.0, 32.0), 0.35);
            let mut hit = base.clone();
            hit.flash = 0.12;
            let mut elite = base.clone();
            elite.elite = true;
            let (s_base, h_base) = sum_pb(&base, false);
            let (s_hit, _) = sum_pb(&hit, false);
            let (_, h_elite) = sum_pb(&elite, false);
            let (s_lod, _) = sum_pb(&base, true);
            let (s_lod_hit, _) = sum_pb(&hit, true);
            assert!(s_hit > s_base + 80, "{kind:?}: hit flash must brighten");
            assert!(s_lod_hit > s_lod + 40, "{kind:?}: LOD hit flash must brighten");
            assert_ne!(h_base, h_elite, "{kind:?}: elite must differ from normal");
        }
    }

    /// Full Wave 3 art sheet: Row 1 idle, Row 2 tells, Row 3 elites, Row 4 LOD + 2x zoom-out.
    /// `RATRUN_DUMP=w3_art3_enemies cargo test dump_wave3_art3_sheet`.
    #[test]
    fn dump_wave3_art3_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("w3_art3_enemies") {
            return;
        }
        use crate::game::ecs::components::Wick;
        use crate::game::ecs::systems::enemy_wave3::*;
        let w = 240usize;
        let h = 160usize;
        let mut pb = PixelBuffer::new(w, h);
        let level = Level::new(Theme::Sewer);
        let cam = Vec2::new(300.0, 300.0);
        art_floor::draw_floor(&mut pb, &level, cam, 0.0);
        let acc = level.palette.accent;
        let kinds = [
            EnemyKind::Pillbug,
            EnemyKind::BellFlea,
            EnemyKind::WickMoth,
            EnemyKind::Porter,
            EnemyKind::Turnkey,
        ];
        // Row 1 (y=24): Idle facing down-right + rat scale ref.
        let mut rat = Player::new(Vec2::new(cam.x + 18.0, cam.y + 24.0), WeaponKind::Gnaw);
        rat.face = Vec2::new(0.4, 0.9).normalized();
        art_rat::draw_rat(&mut pb, &rat, 0.5, cam);
        for (i, &kind) in kinds.iter().enumerate() {
            let mut e = make_enemy(kind, Vec2::new(cam.x + 54.0 + i as f32 * 40.0, cam.y + 24.0), 0.35);
            e.anim = 0.8 + i as f32 * 0.6;
            draw_enemy(&mut pb, &e, cam, acc);
        }
        // Row 2 (y=66): Tells (curled pillbug, flea crosses, burning wick, raised cube, dotted + live bar).
        let mut pb_e = make_enemy(EnemyKind::Pillbug, Vec2::new(cam.x + 30.0, cam.y + 66.0), 0.25);
        pb_e.w3.curl_t = PILLBUG_CURL_SECS * 0.5;
        draw_enemy(&mut pb, &pb_e, cam, acc);

        let mut fl = make_enemy(EnemyKind::BellFlea, Vec2::new(cam.x + 68.0, cam.y + 66.0), 0.15);
        fl.w3.flea_phase = 1;
        fl.w3.home = fl.pos;
        fl.w3.hop_to = fl.pos + Vec2::new(22.0, 4.0);
        draw_enemy(&mut pb, &fl, cam, acc);

        draw_wick(&mut pb, &Wick { pos: Vec2::new(cam.x + 124.0, cam.y + 66.0), fuse: WICK_FUSE_SECS * 0.22 }, cam);

        let mut po = make_enemy(EnemyKind::Porter, Vec2::new(cam.x + 158.0, cam.y + 66.0), -0.35);
        po.w3.raise_t = PORTER_RAISE_SECS * 0.35;
        draw_enemy(&mut pb, &po, cam, acc);

        let mut tk1 = make_enemy(EnemyKind::Turnkey, Vec2::new(cam.x + 194.0, cam.y + 66.0), 0.0);
        tk1.w3.bar_pos = tk1.pos;
        tk1.w3.bar_dir = Vec2::new(0.0, 1.0);
        tk1.w3.bar_t = CROSSBAR_LIVE_SECS + CROSSBAR_TELL_SECS * 0.5;
        draw_enemy(&mut pb, &tk1, cam, acc);

        let mut tk2 = make_enemy(EnemyKind::Turnkey, Vec2::new(cam.x + 220.0, cam.y + 66.0), 0.0);
        tk2.w3.bar_pos = tk2.pos;
        tk2.w3.bar_dir = Vec2::new(0.0, 1.0);
        tk2.w3.bar_t = CROSSBAR_LIVE_SECS * 0.5;
        draw_enemy(&mut pb, &tk2, cam, acc);

        // Row 3 (y=108): Elites (all 5 kinds).
        for (i, &kind) in kinds.iter().enumerate() {
            let mut e = make_enemy(kind, Vec2::new(cam.x + 40.0 + i as f32 * 44.0, cam.y + 108.0), -0.4);
            e.elite = true;
            e.anim = 1.4 + i as f32 * 0.5;
            draw_enemy(&mut pb, &e, cam, acc);
        }
        // Row 4 (y=142): Left half LOD, Right half 2x zoom-out (downsampled 2:1!).
        for (i, &kind) in kinds.iter().enumerate() {
            let e = make_enemy(kind, Vec2::new(cam.x + 20.0 + i as f32 * 22.0, cam.y + 142.0), 0.25);
            draw_enemy_lod(&mut pb, &e, cam, acc);
        }
        let mut hi = PixelBuffer::new(220, 48);
        art_floor::draw_floor(&mut hi, &level, cam, 0.0);
        for (i, &kind) in kinds.iter().enumerate() {
            let e = make_enemy(kind, Vec2::new(cam.x + 22.0 + i as f32 * 42.0, cam.y + 24.0), 0.35);
            draw_enemy(&mut hi, &e, cam, acc);
        }
        for dy in 0..24usize {
            for dx in 0..110usize {
                let c = hi.pixel_at(dx * 2, dy * 2);
                pb.plot((124 + dx) as i32, (130 + dy) as i32, c);
            }
        }
        let mut out = format!("P6\n{} {}\n255\n", w, h).into_bytes();
        for y in 0..h {
            for x in 0..w {
                let c = pb.pixel_at(x, y);
                out.extend_from_slice(&[c.0, c.1, c.2]);
            }
        }
        let dir = std::env::var_os("RATRUN_DUMP_DIR")
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        std::fs::write(dir.join("ratrun_w3_art3_enemies.ppm"), out).unwrap();
    }
}
