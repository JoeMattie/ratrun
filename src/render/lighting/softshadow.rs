//! W6 — soft penumbra and long shadows from low fixtures (an extended cache
//! beside the cached `ShadowProfile`), body shadows from real lights, and
//! wet-floor reflections (contract §4, techniques 05 + 06, and the owner's
//! "entities should cast shadows").
//!
//! * [`SoftProfile`] + [`bake`]: per ray of the angular atlas, the **first**
//!   blocker of any height and that height (the old profile only records
//!   blockers within 5 px of the light's height). Keyed on its own inputs, so
//!   a static fixture is traced once and a profile left stale while the
//!   effects were off is rebuilt the first frame they are on again.
//! * [`visibility`] (Full only): a percentage-closer soft shadow over the
//!   cached rays. A blocker search over a few neighbouring rays gives the mean
//!   blocker distance `d_b`; the penumbra is `LIGHT_SIZE · (d − d_b) / d_b`
//!   wide, so it widens with the distance past the blocker, and is filtered
//!   with five taps. A blocker lower than the light casts a *finite* shadow of
//!   length `d_b · h / (H − h)` (similar triangles), so a low fixture (a sconce
//!   at 9 px, a fire at 6) throws long casts and a high pendant short ones.
//!   Hard is the crisp old tap, Coarse/Compact the old path.
//! * [`BodyShadows`] + [`entity_shadows`]: every on-screen body (≤ 64, nearest
//!   the rat first) casts a soft wedge away from each of the ≤ 2 strongest
//!   lights reaching it, removing that light's share of the floor's light.
//!   Analytic, per pixel over the wedge's box, after the composite; nothing is
//!   re-baked. Flyers cast a detached ellipse. Clamped so that, stacked on the
//!   overhead shadow, the floor keeps at least 45 % of its lit brightness.
//! * [`reflections`]: lamp streaks on floor pixels whose resolved material
//!   reflects (`env > 0`; Hard: Water and Ink only), for the brightest five
//!   visible lights, displaced by the bump normals so grout breaks them, gated
//!   by the covering sample's received light. Static: no clock is read.
//!
//! Render-only and deterministic; integer arithmetic in the pixel loops.

use std::cell::Cell;

use super::engine::{Grid, LightingMode};
use super::fx::{strength, Canvas, FxFrame, FxLight, LightTag, Normal, ShadowQuery};
use super::material::{self, MaterialKind};
use super::shadow::{Geometry, ShadowProfile, RAYS};
use super::{LayerMask, Light, LightScene, ShadowSoftness};
use crate::game::ecs::systems::boss::{Boss, BossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::math::Vec2;
use crate::term::ColorDepth;

// ---------------------------------------------------------------------------
// Soft penumbra and long casts
// ---------------------------------------------------------------------------

/// The apparent size of a lamp for the penumbra, in world px (bold = 8).
const LIGHT_SIZE: f32 = 8.0 * strength::SOFT_SHADOW;
/// The widest filter, in rays either side of the centre ray.
const MAX_SPREAD: f32 = 8.0;
/// Rays visited by the blocker search (offsets from the centre ray).
const SEARCH: [isize; 9] = [-8, -4, -2, -1, 0, 1, 2, 4, 8];
/// Darkness of a finite (lower-than-the-light) blocker's cast at its foot,
/// fading to nothing at the tip (bold = 1.0 of the occluded light).
const LONG_OPACITY: f32 = 1.2 * strength::LONG_SHADOW;
/// The longest finite cast, world px.
const MAX_LONG: f32 = 160.0;

/// The per-light cache beside each `ShadowProfile` (one per static fixture,
/// one per moving shadow slot): the first blocker of any height per ray.
#[derive(Clone)]
pub(super) struct SoftProfile {
    /// L∞ distance to the first blocker on each ray (infinite = clear).
    hit: [f32; RAYS],
    /// That blocker's height, px (0 = clear).
    height: [u8; RAYS],
    /// `(geometry revision, pos, reach, occluders, height)` of the bake.
    key: Option<(u32, u32, u32, u32, u8, u32)>,
}

impl Default for SoftProfile {
    fn default() -> Self {
        Self {
            hit: [f32::INFINITY; RAYS],
            height: [0; RAYS],
            key: None,
        }
    }
}

impl SoftProfile {
    /// How much of the light the finite cast on ray `i` takes at L∞ distance
    /// `dist` from a light at height `light_h` (0 = none).
    #[inline]
    fn long_occlusion(&self, i: usize, dist: f32, light_h: f32) -> f32 {
        let hit = self.hit[i];
        if !hit.is_finite() || dist <= hit {
            return 0.0;
        }
        let h = f32::from(self.height[i]);
        // A blocker within 5 px of the light's height is the old profile's
        // (an infinite shadow); one the light is under casts nothing here.
        if h <= 0.0 || h + 5.0 >= light_h {
            return 0.0;
        }
        let len = (hit * h / (light_h - h)).min(MAX_LONG);
        if len < 1.0 {
            return 0.0;
        }
        let t = dist - hit;
        if t >= len {
            return 0.0;
        }
        LONG_OPACITY * (1.0 - t / len)
    }
}

/// The ray direction of atlas entry `i` (as `ShadowProfile::bake` builds it).
#[inline]
fn ray_dir(i: usize) -> Vec2 {
    let n = (i & 63) as f32 / 32.0 - 1.0;
    match i >> 6 {
        0 => Vec2::new(1.0, n),
        1 => Vec2::new(-n, 1.0),
        2 => Vec2::new(-1.0, -n),
        _ => Vec2::new(n, -1.0),
    }
}

/// Called every frame after the light's `ShadowProfile::bake`; `rebaked` is
/// true when that profile was rebuilt (a static fixture's is cached).
pub(super) fn bake(soft: &mut SoftProfile, geometry: &Geometry, light: &Light, tag: LightTag, rebaked: bool) {
    let _ = tag;
    let radius = light.extent();
    let key = (
        geometry.revision,
        light.pos.x.to_bits(),
        light.pos.y.to_bits(),
        radius.to_bits(),
        light.occluders.0,
        light.height.to_bits(),
    );
    // Our own key as well as `rebaked`: while the effects are off this is not
    // called, so a profile rebuilt then must not leave this one stale.
    if !rebaked && soft.key == Some(key) {
        return;
    }
    soft.key = Some(key);
    for i in 0..RAYS {
        let seen = Cell::new(0.0f32);
        let t = geometry.trace(
            light.pos,
            ray_dir(i),
            radius,
            |_, h| {
                seen.set(h);
                true
            },
            light.occluders,
        );
        if t.is_finite() {
            soft.hit[i] = t;
            soft.height[i] = seen.get().clamp(0.0, 255.0) as u8;
        } else {
            soft.hit[i] = f32::INFINITY;
            soft.height[i] = 0;
        }
    }
}

/// The shadow visibility of one sample (replaces
/// `ShadowProfile::visibility` while the effects are on).
pub(super) fn visibility(profile: &ShadowProfile, soft: Option<&SoftProfile>, q: &ShadowQuery<'_>) -> f32 {
    let soft = match soft {
        Some(s) if q.mode == LightingMode::Full && s.key.is_some() => s,
        // Hard: the crisp old tap (`softness` is `None` below Full);
        // Coarse/Compact: the old path.
        _ => return profile.visibility(q.delta, q.surface, q.softness),
    };
    let dist = q.delta.x.abs().max(q.delta.y.abs());
    let Some(index) = ShadowProfile::ray_index(q.delta) else {
        return 1.0;
    };
    let depth = profile.depth();
    let floor = q.surface.height == 0.0;
    // As the old tap: a raised receiver's leading edge and cap catch light.
    let reach = if floor { 0.9 } else { 8.0 };
    let light_h = q.light.height;
    let at = |off: isize| index.wrapping_add_signed(off) & (RAYS - 1);
    let tap = |i: usize| {
        let inf = ((depth[i] + reach - dist) * 0.35 + 1.0).clamp(0.0, 1.0);
        if floor {
            inf * (1.0 - soft.long_occlusion(i, dist, light_h)).clamp(0.0, 1.0)
        } else {
            inf
        }
    };
    // Blocker search: the mean distance of the tall blockers around this ray.
    let mut sum = 0.0;
    let mut n = 0u32;
    for off in SEARCH {
        let d = depth[at(off)];
        if d + reach < dist {
            sum += d;
            n += 1;
        }
    }
    if n == 0 {
        return tap(index);
    }
    let blocker = (sum / n as f32).max(4.0);
    let size = match q.light.shadow_softness {
        ShadowSoftness::None => LIGHT_SIZE * 0.5,
        ShadowSoftness::Soft => LIGHT_SIZE,
        ShadowSoftness::Softer => LIGHT_SIZE * 1.5,
    };
    let width_px = size * (dist - blocker).max(0.0) / blocker;
    // One ray spans dist/32 px across at this distance.
    let spread = (width_px * 32.0 / dist.max(1.0)).min(MAX_SPREAD);
    if spread < 0.5 {
        return tap(index);
    }
    let full = spread.round() as isize;
    let half = (spread * 0.5).round() as isize;
    (tap(at(-full)) + 2.0 * tap(at(-half)) + 2.0 * tap(index) + 2.0 * tap(at(half)) + tap(at(full)))
        / 8.0
}

/// Keep the old hard-coded Sewer/Lab floor streak in `Grid::splat`? False
/// where [`reflections`] draws the real ones on those floors: Full. (Hard
/// reflects only Water and Ink, so the Sewer's brick and the Lab's steel keep
/// the old streak there; Coarse/Compact have no reflections.)
pub(super) fn keep_floor_glint(mode: LightingMode) -> bool {
    !matches!(mode, LightingMode::Full)
}

// ---------------------------------------------------------------------------
// Body shadows from real lights
// ---------------------------------------------------------------------------

/// Bodies that cast from real lights, nearest the rat first.
pub(super) const MAX_BODIES: usize = 64;
/// Lights each body casts from (the strongest reaching it).
pub(super) const LIGHTS_PER_BODY: usize = 2;
/// The floor keeps at least this share of its lit brightness under the
/// overhead shadow and a body shadow together.
pub(super) const FLOOR_KEEP: f32 = 0.32;
/// The deepest body shadow, absolute: `1 − 0.32 / (1 − OVERHEAD_DARKEN)`
/// (≈ 0.448), so even stacked on the overhead core the floor keeps 32 %.
pub(super) const MAX_DARK: f32 = 1.0 - FLOOR_KEEP / (1.0 - strength::OVERHEAD_DARKEN);
const MAX_DARK_Q8: i32 = (MAX_DARK * 256.0) as i32;
/// How much of the light's share a body's shadow core takes away.
const BODY_OPACITY: f32 = 0.5 + 0.6 * strength::LONG_SHADOW;
const BODY_OPACITY_Q8: i32 = (BODY_OPACITY * 256.0) as i32;
/// A light must bring at least this much (`Q8.8` peak) to a body to cast.
const MIN_CAST: f32 = 14.0;
/// The longest body shadow, world px.
const MAX_BODY_LEN: f32 = 40.0;
/// Penumbra growth per px of distance behind the body (Q12 below).
const BODY_PEN_RATE: f32 = 0.5 * strength::SOFT_SHADOW;
/// Bodies this far outside the view still cast into it.
const VIEW_MARGIN: f32 = 48.0;
/// Pixel evaluations per frame for all body shadows (≈ 60 µs at ~1 ns
/// each); casts past it are skipped, farthest bodies from the rat first.
const MAX_CAST_PX: i64 = 60_000;
/// Darkness steps (flat steps keep the terminal's bytes down).
const DARK_STEP_MASK: i32 = !3;

/// A per-frame light with less reach than this (world px) casts no body
/// shadow: every halo (`Falloff::Small`, at most 52 x 1.7 = 88 px) is under it.
const MOVING_CAST_EXTENT: f32 = 90.0;

const RAT_RADIUS: f32 = 4.5;
const RAT_HEIGHT: f32 = 8.0;
const FLYER_LIFT: f32 = 10.0;
const FLYER_HEIGHT: f32 = 4.0;
const BOSS_HEIGHT: f32 = 23.0;

/// One body that occludes real lights (world px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShadowBody {
    pub pos: Vec2,
    pub radius: f32,
    /// Body height above its base.
    pub height: f32,
    /// Height of its base above the floor (flyers, airborne bosses; 0 = on
    /// the ground).
    pub lift: f32,
}

/// What [`entity_shadows`] did this frame (tests and the profiler).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CastStats {
    pub bodies: usize,
    pub casts: usize,
    pub max_per_body: usize,
}

/// The frame's body list and the darkness scratch, kept by `LightMap`
/// (no allocation once the horde and the view have reached their size).
#[derive(Default)]
pub(super) struct BodyShadows {
    list: Vec<ShadowBody>,
    dark: Vec<u8>,
}

impl BodyShadows {
    pub(super) fn new() -> Self {
        Self {
            list: Vec::with_capacity(256),
            dark: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn bodies(&self) -> &[ShadowBody] {
        &self.list
    }

    /// Offer one body (call [`BodyShadows::finish`] after the last).
    #[cfg(test)]
    pub(super) fn push(&mut self, body: ShadowBody) {
        self.list.push(body);
    }

    /// Keep the [`MAX_BODIES`] nearest `rat` (ties by position, so the set
    /// does not depend on the order the bodies were offered in).
    pub(super) fn finish(&mut self, rat: Vec2) {
        if self.list.len() > MAX_BODIES {
            let key = |b: &ShadowBody| b.pos.dist_sq(rat);
            self.list.select_nth_unstable_by(MAX_BODIES - 1, |a, b| {
                key(a)
                    .total_cmp(&key(b))
                    .then_with(|| a.pos.x.total_cmp(&b.pos.x))
                    .then_with(|| a.pos.y.total_cmp(&b.pos.y))
            });
            self.list.truncate(MAX_BODIES);
        }
        // Nearest first, so the pixel budget drops the farthest casts.
        self.list.sort_unstable_by(|a, b| {
            a.pos
                .dist_sq(rat)
                .total_cmp(&b.pos.dist_sq(rat))
                .then_with(|| a.pos.x.total_cmp(&b.pos.x))
                .then_with(|| a.pos.y.total_cmp(&b.pos.y))
        });
    }

    /// The rat, the living enemies (minibosses are enemies) and the bosses
    /// within `VIEW_MARGIN` of a `view`-sized frame at `scene.cam`.
    pub(super) fn gather(&mut self, scene: &LightScene<'_>, view: (usize, usize)) {
        self.list.clear();
        let cam = scene.cam;
        let (w, h) = (view.0 as f32, view.1 as f32);
        let on = |p: Vec2| {
            let s = p - cam;
            s.x > -VIEW_MARGIN && s.y > -VIEW_MARGIN && s.x < w + VIEW_MARGIN && s.y < h + VIEW_MARGIN
        };
        if on(scene.player.pos) {
            self.list.push(ShadowBody {
                pos: scene.player.pos,
                radius: RAT_RADIUS,
                height: RAT_HEIGHT,
                lift: 0.0,
            });
        }
        if scene.mode != LightingMode::Compact {
            for entity in scene.ecs.iter_entities() {
                if let Some(boss) = entity.get::<Boss>() {
                    if boss.dead || !on(boss.pos) {
                        continue;
                    }
                    let magpie_up = matches!(
                        boss.state,
                        BossState::Magpie(st) if st.flying()
                    );
                    // Its art draws its own detached shadow (as `LightMap::sun`):
                    // a lamp shadow under it would be a third one.
                    if boss.is_airborne() || magpie_up {
                        continue;
                    }
                    self.list.push(ShadowBody {
                        pos: boss.pos,
                        radius: boss.kind.body_radius(),
                        height: BOSS_HEIGHT,
                        lift: 0.0,
                    });
                } else if let Some(e) = entity.get::<Enemy>() {
                    if !e.alive() || !on(e.pos) {
                        continue;
                    }
                    let flyer = e.kind.archetype() == EnemyKind::Bat;
                    self.list.push(ShadowBody {
                        pos: e.pos,
                        radius: e.radius.max(1.5),
                        height: if flyer {
                            FLYER_HEIGHT
                        } else {
                            (e.radius * 1.8).clamp(4.0, 22.0)
                        },
                        lift: if flyer { FLYER_LIFT } else { 0.0 },
                    });
                }
            }
        }
        self.finish(scene.player.pos);
    }
}

/// The contribution (`Q8.8` peak) light `l` brings to a point `d` px away.
#[inline]
fn reach_gain(l: &FxLight, d: f32) -> f32 {
    if !l.extent.is_finite() || d >= l.extent {
        return 0.0;
    }
    let u = d / l.extent;
    let k = 1.0 - u * u;
    l.gain * k * k
}

/// Does light `l` have a clear line to body `b`? One short DDA trace: a lamp
/// behind a wall casts no body shadow (the pixels it would darken never took
/// its light).
fn reaches(geometry: &Geometry, l: &FxLight, b: &ShadowBody) -> bool {
    let to = b.pos - l.pos;
    let linf = to.x.abs().max(to.y.abs());
    if linf <= b.radius + 1.0 {
        return true;
    }
    let t = geometry.trace(
        l.pos,
        to * (1.0 / linf),
        linf - b.radius,
        |_, h| h + 5.0 >= l.height,
        LayerMask::WALLS,
    );
    !t.is_finite()
}

/// The ≤ 2 strongest lights reaching `b` (index into `lights`, gain at `b`).
fn strongest(
    b: &ShadowBody,
    lights: &[FxLight],
    geometry: &Geometry,
) -> [Option<(usize, f32)>; LIGHTS_PER_BODY] {
    let mut best: [Option<(usize, f32)>; LIGHTS_PER_BODY] = [None; LIGHTS_PER_BODY];
    for (i, l) in lights.iter().enumerate() {
        if l.tag == LightTag::Sun {
            continue; // the sun's body shadows are `Grid::sun`'s
        }
        // Lamps cast body shadows; a projectile's or a pickup's halo does not
        // (a cheese wedge at the rat's feet threw a dark, twitching wedge
        // behind it). A big moving light (a blast, a boss glow) still does.
        if matches!(l.tag, LightTag::Moving { .. }) && l.extent < MOVING_CAST_EXTENT {
            continue;
        }
        let d = l.pos.dist(b.pos);
        // A light inside the body is its own (the rat's aura): no shadow.
        if d <= b.radius + 1.5 {
            continue;
        }
        let c = reach_gain(l, d);
        if c < MIN_CAST {
            continue;
        }
        // Cheap rejects first: only a light that would displace one is traced.
        if best[1].is_some_and(|(_, g)| c <= g) || !reaches(geometry, l, b) {
            continue;
        }
        // Strictly stronger displaces: ties keep the earlier (more important).
        if best[0].is_none_or(|(_, g)| c > g) {
            best[1] = best[0];
            best[0] = Some((i, c));
        } else if best[1].is_none_or(|(_, g)| c > g) {
            best[1] = Some((i, c));
        }
    }
    best
}

/// One body's shadow from one light, in screen-space Q4 fixed point.
struct Cast {
    bx: i32,
    by: i32,
    ux: i32,
    uy: i32,
    r: i32,
    /// Ground wedge: total length; flyer: ellipse centre along `u`.
    len: i32,
    spread: i32,
    pen: i32,
    flyer: Option<(i32, i32)>,
    gain: i32,
    bounds: (i32, i32, i32, i32),
}

fn plan_cast(b: &ShadowBody, l: &FxLight, gain: f32, cam: Vec2) -> Option<Cast> {
    let to = b.pos - l.pos;
    let d = to.len();
    if d < 1e-3 {
        return None;
    }
    let u = to * (1.0 / d);
    let light_h = l.height.max(1.0);
    let r = b.radius;
    let bs = b.pos - cam;
    let (len, flyer, reach) = if b.lift > 0.0 {
        if b.lift + 1.0 >= light_h {
            return None; // the light is under the flyer: nothing on the floor
        }
        let centre = (d * b.lift / (light_h - b.lift)).min(MAX_BODY_LEN);
        let a = r * 1.3 + centre * 0.15;
        let wide = r * 1.1;
        (centre, Some((a, wide)), centre + a)
    } else {
        let cast = if b.height + 1.0 >= light_h {
            MAX_BODY_LEN
        } else {
            (d * b.height / (light_h - b.height)).min(MAX_BODY_LEN)
        };
        let len = r + cast.max(r * 1.5);
        (len, None, len)
    };
    // The wedge widens as (d + t) / d; a light almost touching the body
    // would fan it out to the whole view, so the divergence is capped at
    // 2.5 × the body's width (keeps the cast's box, i.e. its cost, bounded).
    let d_eff = d.max(reach / 1.5);
    let tip = bs + u * reach;
    let side = match flyer {
        Some((a, wide)) => a.max(wide) + 1.0,
        None => r * (1.0 + reach / d_eff) + 0.5 * (1.0 + reach * BODY_PEN_RATE) + 1.0,
    };
    let x0 = bs.x.min(tip.x) - side;
    let x1 = bs.x.max(tip.x) + side;
    let y0 = bs.y.min(tip.y) - side;
    let y1 = bs.y.max(tip.y) + side;
    let q4 = |v: f32| (v * 16.0).round() as i32;
    Some(Cast {
        bx: q4(bs.x),
        by: q4(bs.y),
        ux: (u.x * 4096.0).round() as i32,
        uy: (u.y * 4096.0).round() as i32,
        r: q4(r).max(1),
        len: q4(len).max(1),
        spread: ((r / d_eff) * 4096.0).round() as i32,
        pen: (BODY_PEN_RATE * 4096.0).round() as i32,
        flyer: flyer.map(|(a, w)| (q4(a).max(1), q4(w).max(1))),
        gain: gain.round() as i32,
        bounds: (
            x0.floor() as i32,
            y0.floor() as i32,
            x1.ceil() as i32 + 1,
            y1.ceil() as i32 + 1,
        ),
    })
}

/// The shadow weight (Q8, 0..=256) of pixel `(px, py)` in `c`.
#[inline]
fn cast_weight(c: &Cast, px: i32, py: i32) -> i32 {
    let rx = px * 16 + 8 - c.bx;
    let ry = py * 16 + 8 - c.by;
    // Bodies do not shadow themselves.
    if rx * rx + ry * ry < c.r * c.r {
        return 0;
    }
    let along = (rx * c.ux + ry * c.uy) >> 12;
    let across = ((ry * c.ux - rx * c.uy) >> 12).abs();
    if let Some((a, w)) = c.flyer {
        let da = i64::from(along - c.len);
        let ac = i64::from(across);
        let (a, w) = (i64::from(a), i64::from(w));
        let q = da * da * 256 / (a * a) + ac * ac * 256 / (w * w);
        return (256 - q).max(0) as i32;
    }
    if along <= 0 || along >= c.len {
        return 0;
    }
    let width = c.r + ((along * c.spread) >> 12);
    let pen = 16 + ((along * c.pen) >> 12);
    let edge = width + pen / 2 - across;
    if edge <= 0 {
        return 0;
    }
    let wc = (edge * 256 / pen).min(256);
    let wa = ((c.len - along) * 512 / c.len).min(256);
    (wc * wa) >> 8
}

/// Post-composite pixel stage: the body shadows from real lights (Full and
/// Hard). Darkens floor pixels only (no wall or prop samples, no creature or
/// emissive pixels), by that light's share of the sample's light, never past
/// [`MAX_DARK`]; overlapping shadows take the darker, never the product.
///
/// The coordinator wires it (see the report): `bodies.gather(scene, (w, h))`
/// before the pixel stages and this call right after `upsample::composite`.
pub(super) fn entity_shadows(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    bodies: &mut BodyShadows,
    frame: &FxFrame<'_>,
    geometry: &Geometry,
) -> CastStats {
    let mut stats = CastStats::default();
    if !matches!(frame.mode, LightingMode::Full | LightingMode::Hard) {
        return stats;
    }
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 || grid.samples.is_empty() || frame.lights.is_empty() {
        return stats;
    }
    if bodies.dark.len() < w * h {
        bodies.dark.resize(w * h, 0);
    }
    let amb_peak = f32::from(frame.ambient[0].max(frame.ambient[1]).max(frame.ambient[2]));
    let amb = (amb_peak * super::upsample::ambient_scale(frame.theme, frame.mode)) as i32;
    let (mut ux0, mut uy0, mut ux1, mut uy1) = (w as i32, h as i32, 0i32, 0i32);
    let mut budget = MAX_CAST_PX;
    let BodyShadows { list, dark } = bodies;
    for b in list.iter().take(MAX_BODIES) {
        stats.bodies += 1;
        let mut here = 0;
        for (li, gain) in strongest(b, frame.lights, geometry).into_iter().flatten() {
            let Some(cast) = plan_cast(b, &frame.lights[li], gain, frame.cam) else {
                continue;
            };
            here += 1;
            let (bx0, by0, bx1, by1) = cast.bounds;
            let (x0, y0) = (bx0.max(0), by0.max(0));
            let (x1, y1) = (bx1.min(w as i32), by1.min(h as i32));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let area = i64::from(x1 - x0) * i64::from(y1 - y0);
            if area > budget {
                continue;
            }
            budget -= area;
            for py in y0..y1 {
                for px in x0..x1 {
                    let wt = cast_weight(&cast, px, py);
                    if wt <= 0 {
                        continue;
                    }
                    let i = py as usize * w + px as usize;
                    let m = canvas.mat_at(i);
                    if material::is_creature(m) || material::kind(m) == MaterialKind::Emissive {
                        continue;
                    }
                    let s = grid.sample_index(px as usize, py as usize);
                    let (Some(sample), Some(surface)) = (grid.samples.get(s), grid.surfaces.get(s)) else {
                        continue;
                    };
                    if surface.height > 0.0 || surface.layer & LayerMask::FLOOR.0 == 0 {
                        continue;
                    }
                    let peak = i32::from(sample.mul[0].max(sample.mul[1]).max(sample.mul[2]));
                    if peak <= 0 {
                        continue;
                    }
                    let lit = (peak - amb).max(0);
                    let cc = cast.gain.min(lit);
                    if cc <= 0 {
                        continue;
                    }
                    let share = (cc * 256 / peak).min(256);
                    let dk = ((((wt * share) >> 8) * BODY_OPACITY_Q8) >> 8).min(MAX_DARK_Q8);
                    let slot = &mut dark[i];
                    if dk > i32::from(*slot) {
                        *slot = dk as u8;
                    }
                }
            }
            ux0 = ux0.min(x0);
            uy0 = uy0.min(y0);
            ux1 = ux1.max(x1);
            uy1 = uy1.max(y1);
        }
        stats.casts += here;
        stats.max_per_body = stats.max_per_body.max(here);
    }
    // Apply and reset the touched box (the scratch stays zero elsewhere).
    for py in uy0.max(0)..uy1.min(h as i32) {
        let row = py as usize * w;
        for px in ux0.max(0)..ux1.min(w as i32) {
            let i = row + px as usize;
            let q = i32::from(dark[i]) & DARK_STEP_MASK;
            dark[i] = 0;
            if q <= 0 {
                continue;
            }
            let keep = 256 - q;
            let p = &mut canvas.px[i];
            *p = (
                ((i32::from(p.0) * keep) >> 8) as u8,
                ((i32::from(p.1) * keep) >> 8) as u8,
                ((i32::from(p.2) * keep) >> 8) as u8,
            );
        }
    }
    stats
}

// ---------------------------------------------------------------------------
// Wet-floor reflections
// ---------------------------------------------------------------------------

/// Lights reflected per frame (the brightest visible ones).
const MAX_REFLECT: usize = 5;
/// A light dimmer than this (`Q8.8` peak) is not reflected.
const MIN_REFLECT_GAIN: f32 = 48.0;
/// The gain at which a streak reaches its full brightness.
const FULL_REFLECT_GAIN: f32 = 256.0;
/// Peak share of the light's colour a fully reflective floor returns (bold).
const REFLECT_PEAK: f32 = 1.0;
/// Received light (`LightInfo::spec`, `Q8.8`) at which the streak is ungated.
const GATE_SPEC: i32 = 80;
/// Bump displacement of the streak, px at a full tangent normal.
const BUMP_PX: i32 = 4;
/// Additive steps of 4/256: flat steps, and nothing below one (8 hid the tile
/// floors' streak entirely: its peak is 9).
const REFLECT_STEP_MASK: i32 = !3;
/// Ink pads considered per light.
const MAX_PADS: usize = 8;

/// Post-composite pixel stage: lamp streaks on reflective pixels
/// (`fx::strength::REFLECTION`).
pub(super) fn reflections(grid: &Grid, canvas: &mut Canvas<'_>, normals: &[Normal], frame: &FxFrame<'_>) {
    let hard = match frame.mode {
        LightingMode::Full => false,
        LightingMode::Hard => true,
        _ => return,
    };
    if frame.depth == ColorDepth::Ansi16 {
        return;
    }
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 || grid.info.is_empty() {
        return;
    }
    let mut done = 0;
    for l in frame.lights {
        if done >= MAX_REFLECT {
            break;
        }
        if l.tag == LightTag::Sun || !l.extent.is_finite() || l.gain < MIN_REFLECT_GAIN {
            continue;
        }
        if streak(grid, canvas, normals, frame, l, hard) {
            done += 1;
        }
    }
}

/// One light's streak; false if it is off the canvas.
fn streak(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    normals: &[Normal],
    frame: &FxFrame<'_>,
    l: &FxLight,
    hard: bool,
) -> bool {
    let (w, h) = (canvas.w as i32, canvas.h as i32);
    let s = l.pos - frame.cam;
    let height = l.height.clamp(2.0, 40.0);
    let start = s.y + height * 0.5;
    let len = 18.0 + height * 1.8;
    let half = 2.0 + height * 0.08;
    let x0 = ((s.x - 2.0 * half - BUMP_PX as f32 - 1.0).floor() as i32).max(0);
    let x1 = ((s.x + 2.0 * half + BUMP_PX as f32 + 2.0).ceil() as i32).min(w);
    let y0 = ((start - BUMP_PX as f32 - 1.0).floor() as i32).max(0);
    let y1 = ((start + len + BUMP_PX as f32 + 2.0).ceil() as i32).min(h);
    if x0 >= x1 || y0 >= y1 {
        return false;
    }
    let level = (l.gain / FULL_REFLECT_GAIN).min(1.0);
    let k_q8 = (strength::REFLECTION * REFLECT_PEAK * level * 256.0).round() as i32;
    if k_q8 <= 0 {
        return true;
    }
    let lx16 = (s.x * 16.0).round() as i32;
    let start16 = (start * 16.0).round() as i32;
    let len16 = ((len * 16.0).round() as i32).max(16);
    let half16 = ((half * 16.0).round() as i32).max(8);
    let color = (i32::from(l.color.0), i32::from(l.color.1), i32::from(l.color.2));
    // Ink pads touching the streak's box (untagged: read from the level).
    let mut pads = [(0i32, 0i32, 0i32, 0i32); MAX_PADS];
    let mut npads = 0;
    for pad in &frame.level.objects.ink_pads {
        if npads >= MAX_PADS {
            break;
        }
        let p = pad.pos - frame.cam;
        let r = (
            (p.x - pad.half.x).floor() as i32,
            (p.y - pad.half.y).floor() as i32,
            (p.x + pad.half.x).ceil() as i32,
            (p.y + pad.half.y).ceil() as i32,
        );
        if r.0 < x1 && r.2 > x0 && r.1 < y1 && r.3 > y0 {
            pads[npads] = r;
            npads += 1;
        }
    }
    let flat_normals = normals.len() != canvas.w * canvas.h;
    for py in y0..y1 {
        for px in x0..x1 {
            let i = py as usize * canvas.w + px as usize;
            let si = grid.sample_index(px as usize, py as usize);
            let (Some(surface), Some(info)) = (grid.surfaces.get(si), grid.info.get(si)) else {
                continue;
            };
            // Floors only: a wall face or a prop top does not hold a puddle.
            if surface.height > 0.0 || surface.layer & LayerMask::FLOOR.0 == 0 {
                continue;
            }
            let m = canvas.mat_at(i);
            if material::is_creature(m) {
                continue;
            }
            let mut kind = material::resolve(m, frame.theme, false);
            if material::kind(m) == MaterialKind::Default
                && pads[..npads]
                    .iter()
                    .any(|r| px >= r.0 && px < r.2 && py >= r.1 && py < r.3)
            {
                kind = MaterialKind::Ink;
            }
            if kind == MaterialKind::Emissive
                || (hard && !matches!(kind, MaterialKind::Water | MaterialKind::Ink))
            {
                continue;
            }
            let env = i32::from(material::params(kind).env);
            if env == 0 {
                continue;
            }
            let gate = (i32::from(info.spec) * 256 / GATE_SPEC).min(256);
            if gate <= 0 {
                continue;
            }
            let n = if flat_normals { [0, 0] } else { normals[i] };
            let xp = px * 16 + 8 + ((i32::from(n[0]) * BUMP_PX * 16) >> 7);
            let yp = py * 16 + 8 + ((i32::from(n[1]) * BUMP_PX * 16) >> 7);
            let t = yp - start16;
            if t < 0 || t >= len16 {
                continue;
            }
            let width = half16 + half16 * t / len16;
            let across = (xp - lx16).abs();
            if across >= width {
                continue;
            }
            let wc = 256 - across * 256 / width;
            let wc = (wc * wc) >> 8;
            let fade_in = (t * 256 / 48).min(256);
            let fade_out = (len16 - t) * 256 / len16;
            let wt = (wc * fade_in.min(fade_out)) >> 8;
            let amt = ((((((wt * gate) >> 8) * env) >> 8) * k_q8) >> 8) & REFLECT_STEP_MASK;
            if amt <= 0 {
                continue;
            }
            let p = &mut canvas.px[i];
            *p = (
                (i32::from(p.0) + ((color.0 * amt) >> 8)).min(255) as u8,
                (i32::from(p.1) + ((color.1 * amt) >> 8)).min(255) as u8,
                (i32::from(p.2) + ((color.2 * amt) >> 8)).min(255) as u8,
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::fx::testkit;
    use super::super::shadow::Surface;
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;
    use crate::game::level::{Level, Prop, PropKind, Rectf, Theme};
    use crate::render::framebuffer::PixelBuffer;
    use crate::render::palette::Rgb;

    const WALL: TileKind = TileKind::Wall(WallKind::Solid);

    fn floor() -> Surface {
        Surface {
            layer: LayerMask::FLOOR.0,
            ..Surface::default()
        }
    }

    fn baked(level: &Level, light: &Light) -> (ShadowProfile, SoftProfile) {
        let mut geometry = Geometry::default();
        geometry.sync(level);
        let mut profile = ShadowProfile::default();
        profile.bake(&geometry, *light);
        let mut soft = SoftProfile::default();
        bake(&mut soft, &geometry, light, LightTag::Moving { priority: 3 }, true);
        (profile, soft)
    }

    fn vis(profile: &ShadowProfile, soft: &SoftProfile, light: &Light, at: Vec2, mode: LightingMode) -> f32 {
        visibility(
            profile,
            Some(soft),
            &ShadowQuery {
                light,
                tag: LightTag::Moving { priority: 3 },
                delta: at - light.pos,
                surface: floor(),
                softness: if mode == LightingMode::Full {
                    light.shadow_softness
                } else {
                    ShadowSoftness::None
                },
                mode,
            },
        )
    }

    /// A wall stub (x 64..80, y 0..80) and a lamp left of it at y 70: the
    /// shadow's lower edge runs from the corner (64, 80) at slope 10/34
    /// (y ≈ 91 at x = 100, ≈ 114 at x = 180); everything above it is deep.
    fn wall_scene() -> (Level, Light) {
        let mut level = Level::empty(Theme::Sewer);
        level.tiles.fill_cells(4, 0, 1, 5, WALL);
        let light = Light::radial(Vec2::new(30.0, 70.0), 220.0, (255, 230, 190), 1.0);
        (level, light)
    }

    #[test]
    fn soft_visibility_is_open_deep_and_between_in_the_penumbra() {
        let (level, light) = wall_scene();
        let (profile, soft) = baked(&level, &light);
        let full = LightingMode::Full;
        assert_eq!(vis(&profile, &soft, &light, Vec2::new(180.0, 170.0), full), 1.0);
        assert_eq!(vis(&profile, &soft, &light, Vec2::new(40.0, 100.0), full), 1.0);
        assert_eq!(vis(&profile, &soft, &light, Vec2::new(180.0, 40.0), full), 0.0);
        let column = |x: f32| -> Vec<f32> {
            (0..200)
                .map(|y| vis(&profile, &soft, &light, Vec2::new(x, y as f32), full))
                .collect()
        };
        let between = |c: &[f32]| c.iter().filter(|v| **v > 0.02 && **v < 0.98).count();
        let near = column(100.0);
        let far = column(180.0);
        assert!(between(&far) > 0, "a penumbra exists");
        assert!(
            between(&far) > between(&near),
            "the penumbra widens past the blocker: near {} far {}",
            between(&near),
            between(&far)
        );
    }

    #[test]
    fn hard_and_coarse_keep_the_old_visibility() {
        let (level, light) = wall_scene();
        let (profile, soft) = baked(&level, &light);
        for mode in [LightingMode::Hard, LightingMode::Coarse, LightingMode::Compact] {
            for y in (0..200).step_by(3) {
                let at = Vec2::new(180.0, y as f32);
                assert_eq!(
                    vis(&profile, &soft, &light, at, mode),
                    profile.visibility(at - light.pos, floor(), ShadowSoftness::None),
                    "{mode:?} at {y}"
                );
            }
        }
    }

    #[test]
    fn a_low_fixture_casts_longer_than_a_high_one() {
        let mut level = Level::empty(Theme::Sewer);
        level.props.push(Prop {
            rect: Rectf::new(96.0, 56.0, 16.0, 16.0),
            kind: PropKind::Tyre, // 13 px
        });
        let mut low = Light::radial(Vec2::new(40.0, 64.0), 260.0, (255, 230, 190), 1.0);
        low.height = 20.0;
        let mut high = low;
        high.height = 40.0;
        let (pl, sl) = baked(&level, &low);
        let (ph, sh) = baked(&level, &high);
        let behind = Vec2::new(152.0, 64.0);
        let v_low = vis(&pl, &sl, &low, behind, LightingMode::Full);
        let v_high = vis(&ph, &sh, &high, behind, LightingMode::Full);
        assert!(v_low < 0.95, "the low lamp's cast still reaches: {v_low}");
        assert!(v_high > 0.99, "the high lamp's cast has ended: {v_high}");
        // Neither shadows the open floor in front of the prop.
        assert_eq!(vis(&pl, &sl, &low, Vec2::new(70.0, 64.0), LightingMode::Full), 1.0);
    }

    #[test]
    fn a_static_soft_profile_is_traced_once() {
        let (level, light) = wall_scene();
        let mut geometry = Geometry::default();
        geometry.sync(&level);
        let mut soft = SoftProfile::default();
        bake(&mut soft, &geometry, &light, LightTag::Moving { priority: 3 }, false);
        let key = soft.key;
        assert!(key.is_some(), "an empty profile bakes without `rebaked`");
        let hit = soft.hit;
        bake(&mut soft, &geometry, &light, LightTag::Moving { priority: 3 }, false);
        assert_eq!(soft.key, key);
        assert_eq!(soft.hit, hit);
    }

    // --- body shadows ---------------------------------------------------

    const FLOOR_RGB: Rgb = (200, 200, 200);

    fn lamp(pos: Vec2) -> FxLight {
        FxLight {
            pos,
            height: 20.0,
            color: (255, 230, 190),
            gain: 300.0,
            extent: 120.0,
            tag: LightTag::Moving { priority: 2 },
            steady: 300.0,
        }
    }

    fn shade(bodies: &[ShadowBody], lights: &[FxLight], mode: LightingMode) -> (PixelBuffer, CastStats) {
        let level = Level::empty(Theme::Sewer);
        let grid = testkit::flat_grid(17, 17, 4, [420; 3]);
        let mut frame = testkit::frame(&level, mode);
        frame.lights = lights;
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(FLOOR_RGB);
        let mut set = BodyShadows::new();
        for &b in bodies {
            set.push(b);
        }
        set.finish(frame.rat);
        let stats = {
            let mut canvas = testkit::canvas(&mut pb);
            let mut geometry = Geometry::default();
            geometry.sync(&level);
            entity_shadows(&grid, &mut canvas, &mut set, &frame, &geometry)
        };
        (pb, stats)
    }

    fn body(x: f32, y: f32) -> ShadowBody {
        ShadowBody {
            pos: Vec2::new(x, y),
            radius: 4.0,
            height: 8.0,
            lift: 0.0,
        }
    }

    fn lum(pb: &PixelBuffer, x: usize, y: usize) -> i32 {
        let p = pb.pixel_at(x, y);
        i32::from(p.0) + i32::from(p.1) + i32::from(p.2)
    }

    #[test]
    fn a_body_shadows_the_floor_behind_it_and_not_in_front() {
        let lights = [lamp(Vec2::new(12.0, 32.0))];
        let (pb, stats) = shade(&[body(32.0, 32.0)], &lights, LightingMode::Full);
        let open = 3 * 200;
        assert_eq!(stats.casts, 1);
        assert!(lum(&pb, 40, 32) < open - 30, "behind: {}", lum(&pb, 40, 32));
        assert_eq!(lum(&pb, 22, 32), open, "in front");
        assert_eq!(lum(&pb, 32, 20), open, "beside");
        assert_eq!(lum(&pb, 32, 32), open, "the body itself");
        // Off below Hard.
        let (pb, stats) = shade(&[body(32.0, 32.0)], &lights, LightingMode::Coarse);
        assert_eq!(stats.casts, 0);
        assert!(pb.pixels().iter().all(|p| *p == FLOOR_RGB));
    }

    #[test]
    fn a_flyer_casts_a_detached_shadow() {
        let lights = [lamp(Vec2::new(12.0, 32.0))];
        let mut b = body(28.0, 32.0);
        b.lift = 10.0;
        b.height = 4.0;
        let (pb, _) = shade(&[b], &lights, LightingMode::Full);
        let open = 3 * 200;
        // d = 16, lift 10 under a 20 px lamp: centred 16 px behind the body.
        assert!(lum(&pb, 44, 32) < open - 20, "detached core");
        assert_eq!(lum(&pb, 34, 32), open, "a gap at its feet");
    }

    #[test]
    fn at_most_two_lights_per_body_and_sixty_four_bodies() {
        let lights: Vec<FxLight> = (0..6)
            .map(|i| lamp(Vec2::new(4.0 + 11.0 * i as f32, 2.0)))
            .collect();
        let bodies: Vec<ShadowBody> = (0..100)
            .map(|i| body(6.0 + (i % 10) as f32 * 6.0, 14.0 + (i / 10) as f32 * 5.0))
            .collect();
        let (_, stats) = shade(&bodies, &lights, LightingMode::Full);
        assert_eq!(stats.bodies, MAX_BODIES);
        assert_eq!(stats.max_per_body, LIGHTS_PER_BODY);
        assert!(stats.casts <= MAX_BODIES * LIGHTS_PER_BODY);
    }

    #[test]
    fn body_darkening_never_passes_the_clamp() {
        const {
            assert!(MAX_DARK > 0.42 && MAX_DARK < 0.48);
            // Under the overhead core the floor keeps 32 %.
            assert!((1.0 - strength::OVERHEAD_DARKEN) * (1.0 - MAX_DARK) >= FLOOR_KEEP - 1e-4);
        }
        let lights: Vec<FxLight> = (0..4)
            .map(|i| {
                let mut l = lamp(Vec2::new(2.0 + 20.0 * i as f32, 30.0));
                l.gain = 2000.0;
                l.height = 9.0;
                l
            })
            .collect();
        let bodies: Vec<ShadowBody> = (0..40)
            .map(|i| body(4.0 + (i % 8) as f32 * 8.0, 8.0 + (i / 8) as f32 * 12.0))
            .collect();
        let (pb, _) = shade(&bodies, &lights, LightingMode::Full);
        let floor = 200.0 * (1.0 - MAX_DARK) - 1.0;
        assert!(pb.pixels().iter().any(|p| *p != FLOOR_RGB));
        for p in pb.pixels() {
            assert!(f32::from(p.0) >= floor, "{p:?} below {floor}");
        }
    }

    #[test]
    fn the_body_set_is_the_nearest_sixty_four_whatever_the_order() {
        let mut a = BodyShadows::new();
        let mut b = BodyShadows::new();
        let all: Vec<ShadowBody> = (0..90).map(|i| body(i as f32 * 3.0, (i % 7) as f32)).collect();
        for &x in &all {
            a.push(x);
        }
        for &x in all.iter().rev() {
            b.push(x);
        }
        a.finish(Vec2::ZERO);
        b.finish(Vec2::ZERO);
        let key = |v: &[ShadowBody]| {
            let mut k: Vec<(u32, u32)> = v.iter().map(|b| (b.pos.x.to_bits(), b.pos.y.to_bits())).collect();
            k.sort_unstable();
            k
        };
        assert_eq!(a.bodies().len(), MAX_BODIES);
        assert_eq!(key(a.bodies()), key(b.bodies()));
    }

    #[test]
    fn the_gather_finds_the_rat() {
        let w = testkit::fx_world(Theme::Sewer);
        let cam = w.player.pos - Vec2::new(64.0, 48.0);
        let scene = testkit::fx_scene(&w, LightingMode::Full, ColorDepth::Truecolor, cam);
        let mut set = BodyShadows::new();
        set.gather(&scene, (128, 96));
        assert!(set.bodies().iter().any(|b| b.pos == w.player.pos && b.lift == 0.0));
    }

    // --- reflections ----------------------------------------------------

    fn reflect(
        mode: LightingMode,
        lights: &[FxLight],
        spec: u16,
        elapsed: f32,
        paint: impl Fn(&mut PixelBuffer),
    ) -> PixelBuffer {
        let level = Level::empty(Theme::Kitchen); // Tile floor: env 40
        let mut grid = testkit::flat_grid(17, 17, 4, [300; 3]);
        for info in &mut grid.info {
            info.spec = spec;
        }
        let mut frame = testkit::frame(&level, mode);
        frame.lights = lights;
        frame.elapsed = elapsed;
        frame.step8 = super::super::fx::step8(elapsed);
        let mut pb = PixelBuffer::new(64, 64);
        pb.set_materials(true);
        pb.set_pen(material::pen(MaterialKind::Stone, false, 0));
        pb.clear((60, 60, 60));
        paint(&mut pb);
        pb.set_pen(0);
        let normals = vec![[0i8, 0i8]; 64 * 64];
        {
            let mut canvas = testkit::canvas(&mut pb);
            reflections(&grid, &mut canvas, &normals, &frame);
        }
        pb
    }

    fn puddle(pb: &mut PixelBuffer) {
        let prev = pb.set_pen(material::pen(MaterialKind::Water, false, 0));
        pb.rect_fill(24, 0, 16, 64, (60, 60, 60));
        pb.set_pen(prev);
    }

    #[test]
    fn reflections_only_on_wet_pixels_and_only_with_the_light_on() {
        let lights = [lamp(Vec2::new(32.0, 8.0))];
        let pb = reflect(LightingMode::Full, &lights, 200, 0.0, puddle);
        let plain = (60, 60, 60);
        let mut lit = 0;
        for y in 0..64 {
            for x in 0..64 {
                let p = pb.pixel_at(x, y);
                if (24..40).contains(&x) {
                    lit += usize::from(p != plain);
                } else {
                    assert_eq!(p, plain, "dry stone at {x},{y}");
                }
            }
        }
        assert!(lit > 10, "a streak on the puddle ({lit} px)");
        let dark = reflect(LightingMode::Full, &[], 200, 0.0, puddle);
        assert!(dark.pixels().iter().all(|p| *p == plain), "no light, no streak");
        let shadowed = reflect(LightingMode::Full, &lights, 0, 0.0, puddle);
        assert!(shadowed.pixels().iter().all(|p| *p == plain), "in shadow, no streak");
        let coarse = reflect(LightingMode::Coarse, &lights, 200, 0.0, puddle);
        assert!(coarse.pixels().iter().all(|p| *p == plain), "off on Coarse");
    }

    #[test]
    fn hard_reflects_water_but_not_tile() {
        let lights = [lamp(Vec2::new(32.0, 8.0))];
        let tile = |pb: &mut PixelBuffer| {
            let prev = pb.set_pen(material::pen(MaterialKind::Tile, false, 0));
            pb.rect_fill(0, 0, 64, 64, (60, 60, 60));
            pb.set_pen(prev);
        };
        let full = reflect(LightingMode::Full, &lights, 200, 0.0, tile);
        let hard = reflect(LightingMode::Hard, &lights, 200, 0.0, tile);
        assert!(full.pixels().iter().any(|p| *p != (60, 60, 60)), "tile reflects on Full");
        assert!(hard.pixels().iter().all(|p| *p == (60, 60, 60)), "not on Hard");
        let water = reflect(LightingMode::Hard, &lights, 200, 0.0, puddle);
        assert!(water.pixels().iter().any(|p| *p != (60, 60, 60)), "water on Hard");
    }

    #[test]
    fn a_still_scene_draws_identical_frames() {
        let lights = [lamp(Vec2::new(32.0, 8.0))];
        let a = reflect(LightingMode::Full, &lights, 200, 1.01, puddle);
        let b = reflect(LightingMode::Full, &lights, 200, 1.07, puddle);
        let c = reflect(LightingMode::Full, &lights, 200, 9.5, puddle);
        assert_eq!(a.pixels(), b.pixels());
        assert_eq!(a.pixels(), c.pixels(), "nothing animates");
        let body_lights = [lamp(Vec2::new(12.0, 32.0))];
        let (x, _) = shade(&[body(32.0, 32.0)], &body_lights, LightingMode::Full);
        let (y, _) = shade(&[body(32.0, 32.0)], &body_lights, LightingMode::Full);
        assert_eq!(x.pixels(), y.pixels());
    }

    #[test]
    fn the_floor_glint_stays_only_where_reflections_are_off() {
        assert!(!keep_floor_glint(LightingMode::Full));
        assert!(keep_floor_glint(LightingMode::Hard));
        assert!(keep_floor_glint(LightingMode::Coarse));
        assert!(keep_floor_glint(LightingMode::Compact));
    }
}
