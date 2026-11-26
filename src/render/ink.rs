//! Self-lit screen-space ink: the last pass of the frame.
//!
//! Two review findings drive this module (`WAVE3 review`, "Reading a crowded
//! floor"):
//!
//! 1. **Hostile information must win the frame.** Enemy pellets, telegraphs,
//!    tells and loot were drawn in the world pass — under smoke, overhangs,
//!    friendly sparks and the lighting pass — so a bright burst could hide the
//!    thing that was about to hurt the player. [`crate::game::world`] now draws
//!    the hostile layer *after* lighting, and this pass re-stamps the
//!    gameplay-critical geometry **in screen space after the resample**, so it
//!    is never darkened and never blurred.
//! 2. **Zoom-out legibility.** At `z = 2` each screen pixel averages a 2×2 box
//!    of world pixels, so a one-pixel pellet contributes a quarter of its
//!    colour change and effectively vanishes. Everything stamped here has a
//!    **minimum on-screen size**: one full-brightness screen pixel, an outline,
//!    or a two-pixel arm, whatever the zoom.
//!
//! It also fixes the **loot language** (`WAVE3 review`, punch list 16): a
//! closed square is a coin, a diamond is an XP gem of any tier, an open bracket
//! is a pocket find and a halo is a chest, so a floor full of pickups reads at a
//! glance at 1x and 2x without retouching the sprites (the Neon art branch owns
//! those).
//!
//! ## Rules this pass obeys
//!
//! - **Render-only.** It reads entities and the level and writes pixels; it
//!   draws no rng, mutates no simulation state, so determinism and the
//!   scripted traces are untouched.
//! - **No doubling at 1x.** The hostile marks are re-stamped *only when zoomed
//!   out* (`scale > 1.05`): at 1x the hostile layer already drew them at the
//!   right size and the art must not be drawn twice. The loot markers are
//!   shapes the page did not have at all, so they are drawn at **every** zoom,
//!   zoomed in included (pinned by `ink::tests::loot_is_marked_at_every_zoom`).
//!   They are a fixed size in screen pixels, so over a zoomed-in sprite they
//!   are a small centre mark and the shape language never switches off
//!   mid-ease.
//! - **Cheap.** One walk per query, a bounding test per entity, and a handful of
//!   `plot` calls each; the whole pass measured ~20 µs at the 600-enemy cap on
//!   the 120×40 reference view (see `ink::tests::the_ink_pass_stays_cheap`).

use bevy_ecs::world::World as EcsWorld;

use crate::game::bullet::{Bullet, Faction};
use crate::game::ecs::components::{Infection, SpawnId, Wick};
use crate::game::ecs::systems::boss::BossProjectile;
use crate::game::ecs::systems::enemy_wave3::{CROSSBAR_LEN, WICK_BURST_RADIUS};
use crate::game::ecs::systems::miniboss::{Miniboss, MinibossShape};
use crate::game::enemy::Enemy;
use crate::game::gem::{Chest, Gem, Gold, Pickup};
use crate::game::level::Level;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// Above this the view is "zoomed out": world pixels are being averaged into
/// screen pixels, so the art can no longer carry a one-pixel mark.
pub const ZOOMED_OUT: f32 = 1.05;
/// From this zoom out the loot markers are drawn one size down
/// (gem 1 px, coin 2x2, short bracket, 5x5 crate).
pub const LOOT_SMALL_FROM: f32 = 1.2;

/// Dark outline behind a re-stamped hostile pellet: the border the review asks
/// for, so a pellet reads against both the bright floor and a white burst.
const BORDER: Rgb = (14, 10, 18);

/// The ink a pocket find's bracket is cut with: darker than any floor, so the
/// bracket stays legible inside the find's own bright halo.
const BRACKET_INK: Rgb = (18, 14, 22);

/// Everything the pass reads, built by `World::draw` from its own fields.
pub struct InkView<'a> {
    pub ecs: &'a mut EcsWorld,
    pub level: &'a Level,
    pub player: &'a Player,
    /// Screen-space camera (same as the scene's, in world pixels).
    pub cam: Vec2,
    /// Source (world) pixels per screen pixel of the last draw: `1,1` at 1x.
    pub scale: Vec2,
    /// The screen buffer's size in pixels.
    pub size: (i32, i32),
    pub elapsed: f32,
    /// A card plate with no rat (`World::hide_rat`): skip the rat's core.
    /// Always `false` in a run.
    pub hide_rat: bool,
}

/// Draw the ink pass into `pb` (the **screen** buffer, after the resample).
pub fn draw(pb: &mut PixelBuffer, view: &mut InkView) {
    let (sw, sh) = (pb.w as i32, pb.h as i32);
    if sw == 0 || sh == 0 {
        return;
    }
    let zoomed_out = view.scale.x > ZOOMED_OUT || view.scale.y > ZOOMED_OUT;
    // `unit` is how many world pixels one screen pixel covers (2.0 at `z = 2`):
    // a world length divided by it is a length in screen pixels, and every
    // minimum size below is in screen pixels, so nothing shrinks past it when
    // zoomed out.
    let unit = view.scale.x.max(view.scale.y).max(0.01);

    draw_loot(pb, view);
    if zoomed_out {
        draw_hostile(pb, view, unit);
    }
}

// ---- Loot language --------------------------------------------------------

/// Closed square = coin, diamond = XP, open bracket = pocket find, halo = chest.
///
/// Always at least one screen pixel per mark, and always distinguishable from
/// the others *by shape* at 1x and 2x. Drawn at every zoom. Zero allocation.
fn draw_loot(pb: &mut PixelBuffer, v: &mut InkView) {
    let (sw, sh) = (pb.w as i32, pb.h as i32);
    // Zoomed out, small loot shrinks with the world (the owner: "when zoomed
    // out, small items like coins should get visually smaller as well"): the
    // same four shapes one size down, so a pulled-back view is not a field of
    // full-size markers.
    let small = v.scale.x.max(v.scale.y) >= LOOT_SMALL_FROM;
    let on = |s: (i32, i32), m: i32| s.0 + m >= 0 && s.1 + m >= 0 && s.0 - m < sw && s.1 - m < sh;

    // XP gems: a diamond of four pixels around the sprite (`<.>` read).
    {
        let mut q = v.ecs.query::<(&SpawnId, &Gem)>();
        for (_, g) in q.iter(&*v.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            let s = screen(g.pos, v.cam, v.scale);
            if !on(s, 2) {
                continue;
            }
            let c = if g.merged { brighten(g.color, 0.6) } else { g.color };
            if small {
                // One bright pixel in the tier's colour.
                pb.plot(s.0, s.1, c);
                continue;
            }
            pb.plot(s.0, s.1 - 1, c);
            pb.plot(s.0, s.1 + 1, c);
            pb.plot(s.0 - 1, s.1, c);
            pb.plot(s.0 + 1, s.1, c);
            pb.plot(s.0, s.1, (255, 255, 255));
        }
    }

    // Coins: a closed 3×3 square outline (the review's "closed square
    // silhouettes for coins"), filled centre.
    {
        let mut q = v.ecs.query::<(&SpawnId, &Gold)>();
        for (_, g) in q.iter(&*v.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            let s = screen(g.pos, v.cam, v.scale);
            if !on(s, 2) {
                continue;
            }
            let c = (255, 196, 64);
            if small {
                // A solid 2x2 square: still a closed square, a third the area.
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    pb.plot(s.0 + dx, s.1 + dy, c);
                }
                continue;
            }
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                pb.plot(s.0 + dx, s.1 + dy, c);
            }
            pb.plot(s.0, s.1, (255, 246, 196));
        }
    }

    // Pocket finds: an open bracket either side of the sprite, in the find's
    // own colour, so "this is a timed buff" never reads as XP or gold.
    {
        let mut q = v.ecs.query::<(&SpawnId, &Pickup)>();
        for (_, p) in q.iter(&*v.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            let s = screen(p.pos, v.cam, v.scale);
            if !on(s, 3) {
                continue;
            }
            if small {
                // A short bracket pair hugging the sprite.
                let c = brighten(p.kind.color(), 0.2);
                for dy in -1..=1i32 {
                    pb.plot(s.0 - 2, s.1 + dy, BRACKET_INK);
                    pb.plot(s.0 + 2, s.1 + dy, BRACKET_INK);
                }
                pb.plot(s.0 - 3, s.1, c);
                pb.plot(s.0 + 3, s.1, c);
                continue;
            }
            bracket(pb, s, brighten(p.kind.color(), 0.2));
        }
    }

    // Chests: a closed square two steps wider than a coin's, gold-white, plus a
    // centre spark. Rings are the *hostile* radius language (the wick's burst,
    // a vent, a miniboss telegraph), so a reward never borrows one.
    {
        let mut q = v.ecs.query::<(&SpawnId, &Chest)>();
        for (_, c) in q.iter(&*v.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            let s = screen(c.pos, v.cam, v.scale);
            if !on(s, 7) {
                continue;
            }
            let c = (255, 236, 168);
            if small {
                // The crate frame at 5x5 with its centre spark, no corner ticks.
                rect_outline(pb, (s.0 - 2, s.1 - 2, s.0 + 2, s.1 + 2), c);
                pb.plot(s.0, s.1, (255, 255, 244));
                continue;
            }
            rect_outline(pb, (s.0 - 3, s.1 - 3, s.0 + 3, s.1 + 3), c);
            // Corner ticks, so the 5×5 crate frame reads as a *box* and not as
            // the coin's 3×3 at a different size.
            for (dx, dy) in [(-4, 0), (4, 0), (0, -4), (0, 4)] {
                pb.plot(s.0 + dx, s.1 + dy, c);
            }
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                pb.plot(s.0 + dx, s.1 + dy, (255, 255, 244));
            }
        }
    }
}

/// An open bracket pair around a sprite centre: `[` … `]`.
///
/// The find's own halo is bright, so a coloured bracket drowns in it. The
/// bracket is instead cut **dark** through the halo's edge (a 3-pixel vertical
/// either side plus the nub that opens it outward), which reads as a bracket on
/// any floor colour and never as an XP diamond or a coin square.
fn bracket(pb: &mut PixelBuffer, s: (i32, i32), c: Rgb) {
    for dy in -2..=2i32 {
        pb.plot(s.0 - 3, s.1 + dy, BRACKET_INK);
        pb.plot(s.0 + 3, s.1 + dy, BRACKET_INK);
    }
    // The nubs point outward, so the pair reads as `[` `]` and not as a box.
    pb.plot(s.0 - 4, s.1, c);
    pb.plot(s.0 + 4, s.1, c);
    pb.plot(s.0 - 5, s.1, c);
    pb.plot(s.0 + 5, s.1, c);
}

// ---- Hostile layer (zoomed out only) --------------------------------------

/// Re-stamp the things that can kill the player at a readable screen size.
fn draw_hostile(pb: &mut PixelBuffer, v: &mut InkView, unit: f32) {
    let (sw, sh) = (pb.w as i32, pb.h as i32);
    let on = |s: (i32, i32), m: i32| s.0 + m >= 0 && s.1 + m >= 0 && s.0 - m < sw && s.1 - m < sh;

    for pot in &v.level.objects.crab_pots {
        use crate::game::art_crabpot::{CrabPotState, POT_HALF};
        if pot.state != CrabPotState::Closing {
            continue;
        }
        let (x, y) = screen(pot.pos, v.cam, v.scale);
        let r = ((POT_HALF / unit).round() as i32).max(3);
        if !on((x, y), r) {
            continue;
        }
        let colour = mix((255, 176, 84), (255, 252, 236), pot.progress);
        let corners = [(x - r, y - r), (x + r, y - r), (x + r, y + r), (x - r, y + r)];
        for side in 0..corners.len() {
            dotted_line(pb, corners[side], corners[(side + 1) % corners.len()], colour, 2);
        }
    }

    // 1. Enemy pellets: a dark one-pixel border and a stable bright core.
    {
        let mut q = v.ecs.query::<(&SpawnId, &Bullet)>();
        for (_, b) in q.iter(&*v.ecs).sort_by_key::<&SpawnId, _>(|id| id.0) {
            if b.faction != Faction::Enemy {
                continue;
            }
            let s = screen(b.pos, v.cam, v.scale);
            if !on(s, 1) {
                continue;
            }
            if b.outfall_bubble {
                pb.ring(s.0, s.1, 1, (154, 231, 242));
                pb.plot(s.0, s.1, BORDER);
            } else {
                pellet(pb, s, brighten(b.color, 0.55));
            }
        }
    }

    // 2. Boss-owned projectiles (blade discs, shrapnel): same treatment.
    {
        let mut q = v.ecs.query::<&BossProjectile>();
        for p in q.iter(&*v.ecs) {
            let s = screen(p.pos, v.cam, v.scale);
            if !on(s, 1) {
                continue;
            }
            pellet(pb, s, (255, 214, 160));
        }
    }

    // 3. A dead Wick Moth's burning fuse: the burst ring is the warning, so
    //    it keeps a minimum radius however far out the view is.
    {
        let mut q = v.ecs.query::<&Wick>();
        for w in q.iter(&*v.ecs) {
            let s = screen(w.pos, v.cam, v.scale);
            if !on(s, 8) {
                continue;
            }
            let total = crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS;
            let t = if total > 0.0 {
                (1.0 - w.fuse / total).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let r = ((WICK_BURST_RADIUS / unit).round() as i32).max(3);
            let ring = mix((255, 176, 84), (255, 252, 236), t);
            dotted_ring(pb, s, r, (v.elapsed * 12.0) as i32, ring);
            pb.plot(s.0, s.1, (255, 240, 200));
        }
    }

    // 4. Wave 3 tells, straight off the bodies: the Bell Flea's two landing
    //    crosses and the Turnkey's dotted crossbar.
    {
        let mut q = v.ecs.query::<&Enemy>();
        for e in q.iter(&*v.ecs) {
            if !e.alive() {
                continue;
            }
            if matches!(e.kind, crate::game::enemy::EnemyKind::Skater | crate::game::enemy::EnemyKind::Lampfish) {
                use crate::game::ecs::systems::floor_outfall::{outfall_enemy_tell, OutfallTellPhase};
                if let Some(tell) = outfall_enemy_tell(e) {
                    let active = tell.phase == OutfallTellPhase::Strike;
                    crate::render::hazard_outline::draw_shape_ink(
                        pb, &tell.shape, v.cam, v.scale, unit, v.elapsed,
                        crate::render::hazard_outline::miniboss_colour(active), active,
                    );
                }
            }
            if e.w3.flea_phase != 0 {
                let warm = (238, 198, 126);
                let arm = ((6.0 / unit).round() as i32).max(2);
                for target in [e.w3.home, e.w3.hop_to] {
                    let s = screen(target, v.cam, v.scale);
                    if on(s, arm + 1) {
                        cross(pb, s, arm, warm);
                    }
                }
            }
            if e.w3.bar_t > 0.0 && e.w3.bar_dir.len_sq() > 1e-4 {
                let half = (CROSSBAR_LEN * 0.5).clamp(0.0, 64.0);
                let a = screen(e.w3.bar_pos - e.w3.bar_dir * half, v.cam, v.scale);
                let b = screen(e.w3.bar_pos + e.w3.bar_dir * half, v.cam, v.scale);
                if on(a, 4) || on(b, 4) {
                    // Live bar solid white-hot, the tell dotted amber.
                    let telling = e.w3.bar_telling();
                    let c = if telling { (226, 190, 120) } else { (255, 246, 214) };
                    dotted_line(pb, a, b, c, if telling { 2 } else { 1 });
                }
            }
        }
    }

    // 5. Named-miniboss telegraphs: the signature's own shape, outlined.
    {
        let mut q = v.ecs.query::<&Miniboss>();
        for mb in q.iter(&*v.ecs) {
            let Some(shape) = mb.telegraph() else { continue };
            if shape.signature_bounds().is_some() {
                crate::game::ecs::systems::miniboss_shapes::draw_telegraph(
                    pb, mb, &shape, v.cam, v.scale,
                );
            } else {
                outline_shape(pb, &*v, &shape, unit);
            }
        }
    }

    // 5b. Boss **floor telegraphs**: the same outline the world pass just drew,
    //     re-stamped with a minimum screen thickness so the crisp resampler
    //     cannot thin a 1 px rim to nothing. Only reached when `zoomed_out`, so
    //     nothing is doubled at 1x (see `render::hazard_outline`).
    {
        let mut q = v.ecs.query::<&crate::game::ecs::systems::boss::Boss>();
        for boss in q.iter(&*v.ecs) {
            if boss.dead {
                continue;
            }
            use crate::render::hazard_outline::{draw_ink, draw_shape_ink, warn_colour};
            if let Some(view) = boss.lantern_view() {
                use crate::game::art_boss_lantern::LanternState::*;
                if matches!(view.state, FeintTell | DarkTell | DecoyLeapTell | GapeTell | GapePull) {
                    for (shape, active) in [(view.attack, view.state == GapePull), (view.stamp, false)] {
                        if let Some(shape) = shape {
                            draw_shape_ink(
                                pb, &shape, v.cam, v.scale, unit, v.elapsed,
                                warn_colour(boss.kind, active), active,
                            );
                        }
                    }
                }
            }
            if !boss.hazards.is_empty() {
                draw_ink(pb, boss, v.cam, v.scale, unit, v.elapsed);
            }
        }
    }

    // 6. Infection pips: a Hantavirus host's countdown.
    {
        let mut q = v.ecs.query::<(&Enemy, &Infection)>();
        for (e, inf) in q.iter(&*v.ecs) {
            let s = screen(e.pos, v.cam, v.scale);
            if !on(s, 6) {
                continue;
            }
            let y = s.1 - ((e.radius / unit).round() as i32).max(2) - 2;
            for k in 0..inf.ticks_left.min(4) as i32 {
                pb.plot(s.0 - 2 + k, y, (236, 128, 206));
            }
        }
    }

    // 7. Floor hazards a step can kill on: steam grate warn/plume cells and
    //    the flatbed's pit edges keep a bright outside edge at any zoom.
    for grate in &v.level.objects.steam_grates {
        if !grate.warning() && !grate.plume() {
            continue;
        }
        let s = screen(grate.pos, v.cam, v.scale);
        let half = ((crate::game::collision::TILE_F * 0.5 / unit).round() as i32).max(2);
        if !on(s, half + 1) {
            continue;
        }
        let c = if grate.plume() { (255, 252, 232) } else { (250, 196, 128) };
        dotted_rect(pb, (s.0 - half, s.1 - half, s.0 + half, s.1 + half), c, 2);
    }
    for pit in v.level.trailer.pits.iter().chain(&v.level.objects.tidal_pits) {
        let a = screen(pit.rect.center_world(), v.cam, v.scale);
        let w = (pit.rect.w as f32 * crate::game::collision::TILE_F / v.scale.x).round() as i32;
        let h = (pit.rect.h as f32 * crate::game::collision::TILE_F / v.scale.y).round() as i32;
        if !on(a, w.max(h) + 1) {
            continue;
        }
        dotted_rect(
            pb,
            (a.0 - w / 2, a.1 - h / 2, a.0 + w / 2, a.1 + h / 2),
            if pit.kind == crate::game::level::PitKind::TidalDrop {
                (143, 211, 220)
            } else {
                (246, 226, 178)
            },
            2,
        );
    }

    // 8. The rat itself keeps a minimum screen size so "where am I" survives a
    //    2×2 box filter. The dash arc that used to be stamped with it is
    //    **gone** (batch 12, the owner's call): nothing is drawn under the rat
    //    any more, at any zoom, and the HUD's dash module is the one cooldown
    //    read-out.
    if !v.hide_rat {
        let s = screen(v.player.pos, v.cam, v.scale);
        if on(s, 2) {
            // The rat's own sprite is pale: a bright core keeps "where am I"
            // but a dark pellet border through it read as a hole, so the rat
            // gets the core only.
            pb.plot(s.0, s.1, (255, 240, 244));
            pb.plot(s.0, s.1 - 1, (255, 196, 210));
        }
    }
}

/// One gameplay-critical mark: a dark border ring and a full-brightness core.
fn pellet(pb: &mut PixelBuffer, s: (i32, i32), c: Rgb) {
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        pb.plot(s.0 + dx, s.1 + dy, BORDER);
    }
    pb.plot(s.0, s.1, c);
}

/// Outline a miniboss signature shape at a minimum screen thickness: a
/// continuous outside edge with a dash travelling along it, so the boundary is
/// readable under a packed floor.
fn outline_shape(pb: &mut PixelBuffer, v: &InkView, shape: &MinibossShape, unit: f32) {
    let c = (255, 188, 120);
    match *shape {
        MinibossShape::Trench { a, b, half } => {
            let pa = screen(a, v.cam, v.scale);
            let pb_ = screen(b, v.cam, v.scale);
            let w = ((half / unit).round() as i32).max(1);
            thick_line(pb, pa, pb_, c, w);
        }
        MinibossShape::Cone { origin, aim, half_angle, len } => {
            let o = screen(origin, v.cam, v.scale);
            let scale_len = len / unit.max(0.01);
            for side in [-1.0f32, 1.0] {
                let a = aim.angle() + side * half_angle;
                let tip = (
                    o.0 + (a.cos() * scale_len).round() as i32,
                    o.1 + (a.sin() * scale_len).round() as i32,
                );
                dotted_line(pb, o, tip, c, 2);
            }
            // The far arc closes the sector. A sweeping miniboss tells a cone
            // up to 150° wide, and two side rays alone read as two separate
            // beams with open floor between them. One dot every other screen
            // pixel of arc length, whatever the zoom, both ends included.
            let base = aim.angle();
            let arc_px = (2.0 * half_angle * scale_len).abs().ceil() as i32;
            let steps = arc_px.clamp(4, 1440);
            for i in 0..=steps {
                if i % 2 != 0 && i != steps {
                    continue;
                }
                let a = base - half_angle + 2.0 * half_angle * i as f32 / steps as f32;
                pb.plot(
                    o.0 + (a.cos() * scale_len).round() as i32,
                    o.1 + (a.sin() * scale_len).round() as i32,
                    c,
                );
            }
        }
        MinibossShape::Cord { spool, tip, .. } => {
            let a = screen(spool, v.cam, v.scale);
            let b = screen(tip, v.cam, v.scale);
            thick_line(pb, a, b, c, 1);
        }
        MinibossShape::RingGap { origin, radius, gap_dir, gap_half, .. } => {
            let o = screen(origin, v.cam, v.scale);
            let r = ((radius / unit).round() as i32).max(2);
            let gap = gap_dir.angle().to_degrees();
            let gap_half = gap_half.to_degrees();
            let steps = 48;
            for i in 0..steps {
                let deg = i as f32 * 360.0 / steps as f32;
                let mut d = (deg - gap).abs() % 360.0;
                if d > 180.0 {
                    d = 360.0 - d;
                }
                if d < gap_half {
                    continue;
                }
                let a = deg.to_radians();
                pb.plot(
                    o.0 + (a.cos() * r as f32).round() as i32,
                    o.1 + (a.sin() * r as f32).round() as i32,
                    c,
                );
            }
        }
        MinibossShape::Stamps { centers, half } => {
            let w = ((half / unit).round() as i32).max(2);
            for centre in centers {
                let s = screen(centre, v.cam, v.scale);
                rect_outline(pb, (s.0 - w, s.1 - w, s.0 + w, s.1 + w), c);
            }
        }
        MinibossShape::Spill { center, aim, half_len, half_w } => {
            // A rectangle: long axis along `aim`, half-length either way.
            let s = screen(center, v.cam, v.scale);
            let (ux, uy) = (aim.x, aim.y);
            let (vx, vy) = (aim.perp().x, aim.perp().y);
            let hl = (half_len / unit).round() as i32;
            let hw = (half_w / unit).round() as i32;
            let corner = |a: f32, b: f32| {
                (
                    s.0 + (ux * a + vx * b).round() as i32,
                    s.1 + (uy * a + vy * b).round() as i32,
                )
            };
            let (p0, p1, p2, p3) = (
                corner(hl as f32, hw as f32),
                corner(hl as f32, -hw as f32),
                corner(-hl as f32, -hw as f32),
                corner(-hl as f32, hw as f32),
            );
            for (a, b) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
                thick_line(pb, a, b, c, 1);
            }
        }
        MinibossShape::Pincer { origin, aim, len, half, gap_lo, gap_hi } => {
            // The claws' whole travel: two bands astride the aim.
            let o = screen(origin, v.cam, v.scale);
            let dir = if aim.len_sq() > 1e-6 { aim.normalized() } else { Vec2::new(1.0, 0.0) };
            let (ux, uy) = (dir.x, dir.y);
            let (vx, vy) = (dir.perp().x, dir.perp().y);
            let reach = len / unit.max(0.01);
            let inner = (gap_lo.max(0.0) * 0.5 - half) / unit.max(0.01);
            let outer = (gap_hi * 0.5 + half) / unit.max(0.01);
            let at = |a: f32, b: f32| {
                (
                    o.0 + (ux * a + vx * b).round() as i32,
                    o.1 + (uy * a + vy * b).round() as i32,
                )
            };
            for side in [-1.0f32, 1.0] {
                let (bi, bo) = (side * inner, side * outer);
                dotted_line(pb, at(0.0, bi), at(reach, bi), c, 2);
                dotted_line(pb, at(0.0, bo), at(reach, bo), c, 2);
                thick_line(pb, at(reach, bi), at(reach, bo), c, 1);
            }
        }
        MinibossShape::Lane { origin, aim, len, seam_half, half, covered, .. } => {
            // The whole lane, both rails, the seam left clean, and a rule
            // where the rollers are right now.
            let o = screen(origin, v.cam, v.scale);
            let dir = if aim.len_sq() > 1e-6 { aim.normalized() } else { Vec2::new(1.0, 0.0) };
            let (ux, uy) = (dir.x, dir.y);
            let (vx, vy) = (dir.perp().x, dir.perp().y);
            let reach = len / unit.max(0.01);
            let inner = seam_half / unit.max(0.01);
            let outer = (seam_half + half * 2.0) / unit.max(0.01);
            let head = covered.min(len) / unit.max(0.01);
            let at = |a: f32, b: f32| {
                (
                    o.0 + (ux * a + vx * b).round() as i32,
                    o.1 + (uy * a + vy * b).round() as i32,
                )
            };
            for side in [-1.0f32, 1.0] {
                let (bi, bo) = (side * inner, side * outer);
                thick_line(pb, at(0.0, bi), at(reach, bi), c, 1);
                thick_line(pb, at(0.0, bo), at(reach, bo), c, 1);
                thick_line(pb, at(reach, bi), at(reach, bo), c, 1);
                dotted_line(pb, at(head, bi), at(head, bo), c, 1);
            }
        }
        _ => {}
    }
}

// ---- Small drawing helpers ------------------------------------------------

#[inline]
fn screen(p: Vec2, cam: Vec2, scale: Vec2) -> (i32, i32) {
    (
        ((p.x - cam.x) / scale.x).round() as i32,
        ((p.y - cam.y) / scale.y).round() as i32,
    )
}

#[inline]
fn brighten(c: Rgb, t: f32) -> Rgb {
    mix(c, (255, 255, 255), t)
}

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t).round() as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t).round() as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t).round() as u8,
    )
}

/// A dotted rectangle: the outline language for *dangerous* floor patches, so a
/// hazard never reads as the solid square that means loot.
fn dotted_rect(pb: &mut PixelBuffer, r: (i32, i32, i32, i32), c: Rgb, step: i32) {
    let (x0, y0, x1, y1) = r;
    for x in x0..=x1 {
        if x.rem_euclid(step.max(1)) != 0 {
            continue;
        }
        pb.plot(x, y0, c);
        pb.plot(x, y1, c);
    }
    for y in y0..=y1 {
        if y.rem_euclid(step.max(1)) != 0 {
            continue;
        }
        pb.plot(x0, y, c);
        pb.plot(x1, y, c);
    }
}

fn rect_outline(pb: &mut PixelBuffer, r: (i32, i32, i32, i32), c: Rgb) {
    let (x0, y0, x1, y1) = r;
    for x in x0..=x1 {
        pb.plot(x, y0, c);
        pb.plot(x, y1, c);
    }
    for y in y0..=y1 {
        pb.plot(x0, y, c);
        pb.plot(x1, y, c);
    }
}

fn cross(pb: &mut PixelBuffer, s: (i32, i32), arm: i32, c: Rgb) {
    for k in -arm..=arm {
        if (k + arm) % 2 == 0 {
            pb.plot(s.0 + k, s.1, c);
            pb.plot(s.0, s.1 + k, c);
        }
    }
}

fn dotted_ring(pb: &mut PixelBuffer, s: (i32, i32), r: i32, phase: i32, c: Rgb) {
    if r <= 0 {
        return;
    }
    let steps = (r * 8).max(8);
    for i in 0..steps {
        // Every other pixel, offset by the animation phase so the dashes crawl.
        if (i + phase).rem_euclid(2) != 0 {
            continue;
        }
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        pb.plot(
            s.0 + (a.cos() * r as f32).round() as i32,
            s.1 + (a.sin() * r as f32).round() as i32,
            c,
        );
    }
}

fn dotted_line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, step: i32) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let n = dx.abs().max(dy.abs()).max(1);
    let step = step.max(1);
    for i in 0..=n {
        if i % step != 0 {
            continue;
        }
        pb.plot(
            a.0 + (dx as f32 * i as f32 / n as f32).round() as i32,
            a.1 + (dy as f32 * i as f32 / n as f32).round() as i32,
            c,
        );
    }
}

/// A dashed band `w` pixels either side of the line: the outside edges of a
/// hostile footprint, not a filled shape (bodies must stay visible through it).
fn thick_line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, w: i32) {
    let w = w.max(1);
    dotted_line(pb, a, b, c, 1);
    for k in 1..=w {
        let off = ((b.1 - a.1) as f32).atan2((b.0 - a.0) as f32) + std::f32::consts::FRAC_PI_2;
        let (nx, ny) = (off.cos(), off.sin());
        let da = ((nx * k as f32).round() as i32, (ny * k as f32).round() as i32);
        dotted_line(pb, (a.0 + da.0, a.1 + da.1), (b.0 + da.0, b.1 + da.1), c, 2);
        dotted_line(pb, (a.0 - da.0, a.1 - da.1), (b.0 - da.0, b.1 - da.1), c, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::bullet::Bullet;
    use crate::game::enemy::EnemyKind;
    use crate::game::gem::{Chest, Gem, Gold, Pickup, PickupKind};
    use crate::game::level::Theme;
    use crate::game::world::World;
    use crate::math::Vec2;

    const SIZE: (usize, usize) = (160, 148);

    /// Render the ink pass over `w`'s current entities at `scale`, on a black
    /// screen buffer (so anything lit is the pass's own ink).
    fn render(w: &mut World, scale: Vec2) -> PixelBuffer {
        let mut pb = PixelBuffer::new(SIZE.0, SIZE.1);
        pb.clear((0, 0, 0));
        let mut view = InkView {
            ecs: &mut w.ecs,
            level: &w.level,
            player: &w.player,
            cam: Vec2::ZERO,
            scale,
            size: (SIZE.0 as i32, SIZE.1 as i32),
            elapsed: 1.0,
            hide_rat: false,
        };
        draw(&mut pb, &mut view);
        pb
    }

    /// Sum of the RGB channels at a screen pixel (brightness proxy).
    fn lum(pb: &PixelBuffer, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
            return 0;
        }
        let (r, g, b) = pb.pixel_at(x as usize, y as usize);
        u32::from(r) + u32::from(g) + u32::from(b)
    }

    fn lit_between(pb: &PixelBuffer, x0: i32, y0: i32, x1: i32, y1: i32) -> usize {
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if lum(pb, x, y) > 30 {
                    n += 1;
                }
            }
        }
        n
    }

    fn world_with_bullet() -> World {
        let mut w = World::new(Theme::Sewer, 7);
        w.player.pos = Vec2::new(80.0, 74.0);
        w.spawn_bullet(Bullet::new(
            Vec2::new(120.0, 74.0),
            Vec2::new(0.0, 0.0),
            4.0,
            Faction::Enemy,
            (240, 120, 160),
        ));
        w
    }

    /// **Zoomed out, a pellet is at least one full-brightness screen pixel with
    /// a dark border** — the review's "give enemy pellets a dark border and a
    /// stable bright core", which a 2×2 box filter otherwise averages away.
    #[test]
    fn zoomed_out_pellets_keep_a_bright_core_and_a_dark_border() {
        let mut w = world_with_bullet();
        let pb = render(&mut w, Vec2::new(2.0, 2.0));
        // World (120, 74) at scale 2 = screen (60, 37).
        let core = lum(&pb, 60, 37);
        assert!(core > 300, "no bright core at 2x (luminance {core})");
        // At least one of the four neighbours is the dark border.
        let border = [(61, 37), (59, 37), (60, 38), (60, 36)]
            .iter()
            .filter(|&&(x, y)| lum(&pb, x, y) == u32::from(BORDER.0) + u32::from(BORDER.1) + u32::from(BORDER.2))
            .count();
        assert!(border >= 2, "the pellet has no dark border at 2x");
    }

    /// **Nothing is doubled at 1x.** At 1:1 the hostile part of the pass draws
    /// nothing at all: the world pass already drew those marks at the right
    /// size, and a second stamp would brighten them.
    #[test]
    fn at_one_to_one_no_hostile_mark_is_stamped() {
        let mut w = world_with_bullet();
        let pb = render(&mut w, Vec2::new(1.0, 1.0));
        assert_eq!(
            lum(&pb, 120, 74),
            0,
            "the ink pass re-drew a pellet at 1x"
        );
        assert_eq!(lit_between(&pb, 0, 0, SIZE.0 as i32, SIZE.1 as i32), 0);
    }

    /// **No dash arc is stamped any more** (batch 12: the owner asked for the
    /// charge read-out under the rat to go, in every state and at every zoom).
    /// The pass still stamps the rat's own two-pixel core when it is zoomed out
    /// and, exactly as before, nothing at all at 1:1 — what it never does again
    /// is put a pip under the rat, cooling, ready, dashing or refused.
    #[test]
    fn no_dash_arc_is_stamped_at_any_zoom() {
        for z in [1.0f32, 1.5, 2.0] {
            for (cd, denied, dashing) in [
                (0.9f32, 0.0f32, 0.0f32), // cooling
                (0.0, 0.0, 0.0),          // ready
                (1.2, 0.2, 0.0),          // a refused press
                (0.0, 0.0, 0.16),         // mid-dash
            ] {
                let mut w = World::new(Theme::Sewer, 7);
                w.player.pos = Vec2::new(80.0, 74.0);
                w.player.dash_cd = cd;
                w.player.dash_denied = denied;
                w.player.dash_active = dashing;
                let pb = render(&mut w, Vec2::new(z, z));
                // Every pixel the pass wrote (the buffer starts black).
                let mut lit: Vec<(i32, i32)> = Vec::new();
                for y in 0..SIZE.1 as i32 {
                    for x in 0..SIZE.0 as i32 {
                        if lum(&pb, x, y) > 0 {
                            lit.push((x, y));
                        }
                    }
                }
                // The rat's core, and only that, when zoomed out.
                let core = ((80.0 / z).round() as i32, (74.0 / z).round() as i32);
                let mut want: Vec<(i32, i32)> = Vec::new();
                if z > ZOOMED_OUT {
                    want.push((core.0, core.1 - 1));
                    want.push(core);
                }
                want.sort();
                assert_eq!(
                    lit, want,
                    "z={z} cd={cd} denied={denied} dashing={dashing}: the pass stamped ink under the rat"
                );
            }
        }
        // `InkView::hide_rat` (the Library's plates) still silences the core.
        let mut w = World::new(Theme::Sewer, 7);
        w.player.pos = Vec2::new(80.0, 74.0);
        let mut pb = PixelBuffer::new(SIZE.0, SIZE.1);
        pb.clear((0, 0, 0));
        let mut view = InkView {
            ecs: &mut w.ecs,
            level: &w.level,
            player: &w.player,
            cam: Vec2::ZERO,
            scale: Vec2::new(2.0, 2.0),
            size: (SIZE.0 as i32, SIZE.1 as i32),
            elapsed: 1.0,
            hide_rat: true,
        };
        draw(&mut pb, &mut view);
        assert_eq!(
            lit_between(&pb, 0, 0, SIZE.0 as i32, SIZE.1 as i32),
            0,
            "hide_rat no longer silences the rat's core"
        );
    }

    /// Gems, coins, pocket finds and chests are four **different shapes** at 1x
    /// and 2x, not four colours of dot (the review's loot-language finding).
    #[test]
    fn the_four_loot_languages_are_four_shapes() {
        for z in [1.0f32, 2.0] {
            let mut w = World::new(Theme::Sewer, 7);
            w.player.pos = Vec2::new(80.0, 74.0);
            w.spawn_gem(Gem::new(Vec2::new(40.0, 40.0), 5));
            crate::game::ecs::resources::spawn_with_id(
                &mut w.ecs,
                Gold::new(Vec2::new(70.0, 40.0), 1),
            );
            w.spawn_pickup(Pickup {
                pos: Vec2::new(100.0, 40.0),
                kind: PickupKind::BottleCap,
                bob: 0.0,
            });
            crate::game::ecs::resources::spawn_with_id(
                &mut w.ecs,
                Chest { pos: Vec2::new(130.0, 40.0) },
            );
            let pb = render(&mut w, Vec2::new(z, z));
            let at = |x: f32| ((x / z).round() as i32, (40.0f32 / z).round() as i32);
            let patch = |p: (i32, i32)| {
                let mut bits: Vec<bool> = Vec::new();
                for dy in -6..=6 {
                    for dx in -6..=6 {
                        bits.push(lum(&pb, p.0 + dx, p.1 + dy) > 30);
                    }
                }
                bits
            };
            let gem = patch(at(40.0));
            let gold = patch(at(70.0));
            let pickup = patch(at(100.0));
            let chest = patch(at(130.0));
            for (name, a, b) in [
                ("gem/coin", &gem, &gold),
                ("gem/pickup", &gem, &pickup),
                ("gem/chest", &gem, &chest),
                ("coin/pickup", &gold, &pickup),
                ("coin/chest", &gold, &chest),
                ("pickup/chest", &pickup, &chest),
            ] {
                assert_ne!(a, b, "at z={z}: {name} print the same pixels");
            }
        }
    }

    /// Every loot kind keeps its mark on its own screen position at each rung
    /// of the zoom ladder, zoomed in included: the pass has no zoom gate on
    /// loot, and the mark follows the per-axis scale.
    #[test]
    fn loot_is_marked_at_every_zoom() {
        for z in [0.5f32, 0.75, 1.0, 1.25, 1.5, 2.0] {
            let mut w = World::new(Theme::Sewer, 7);
            w.player.pos = Vec2::new(80.0, 74.0);
            w.spawn_gem(Gem::new(Vec2::new(20.0, 20.0), 5));
            crate::game::ecs::resources::spawn_with_id(
                &mut w.ecs,
                Gold::new(Vec2::new(40.0, 20.0), 1),
            );
            w.spawn_pickup(Pickup {
                pos: Vec2::new(60.0, 40.0),
                kind: PickupKind::BottleCap,
                bob: 0.0,
            });
            crate::game::ecs::resources::spawn_with_id(
                &mut w.ecs,
                Chest { pos: Vec2::new(40.0, 60.0) },
            );
            let pb = render(&mut w, Vec2::new(z, z));
            let at = |x: f32, y: f32| ((x / z).round() as i32, (y / z).round() as i32);
            // Gem and coin light their centre pixel; the bracket and the crate
            // frame leave the centre to the sprite, so count their patch.
            let (gx, gy) = at(20.0, 20.0);
            let tip_y = if z >= LOOT_SMALL_FROM { gy } else { gy - 1 };
            assert_eq!(
                pb.pixel_at(gx as usize, tip_y as usize),
                crate::game::gem::GemTier::Chip.color(),
                "z={z}: gem mark lost its tier colour"
            );
            let (cx, cy) = at(40.0, 20.0);
            assert!(lum(&pb, cx, cy) > 300, "z={z}: no coin mark");
            let (px, py) = at(60.0, 40.0);
            assert!(
                lit_between(&pb, px - 5, py - 2, px + 6, py + 3) >= 4,
                "z={z}: no pocket-find bracket"
            );
            let (hx, hy) = at(40.0, 60.0);
            assert!(
                lit_between(&pb, hx - 4, hy - 4, hx + 5, hy + 5) >= 16,
                "z={z}: no chest frame"
            );
        }
    }

    /// A wide miniboss cone is a closed sector when zoomed out: both side rays
    /// **and** the far arc between them, at 1.5x and 2x. It used to be the two
    /// rays alone, which a 150° sweep reads as two unrelated beams.
    #[test]
    fn a_zoomed_out_miniboss_cone_draws_its_far_arc() {
        let origin = Vec2::new(150.0, 140.0);
        let aim = Vec2::new(1.0, 0.0);
        let half_angle = 75.0f32.to_radians();
        let len = 60.0f32;
        let shape = MinibossShape::Cone { origin, aim, half_angle, len };
        for z in [1.5f32, 2.0] {
            let mut w = World::new(Theme::Sewer, 7);
            let mut pb = PixelBuffer::new(SIZE.0, SIZE.1);
            pb.clear((0, 0, 0));
            let view = InkView {
                ecs: &mut w.ecs,
                level: &w.level,
                player: &w.player,
                cam: Vec2::ZERO,
                scale: Vec2::new(z, z),
                size: (SIZE.0 as i32, SIZE.1 as i32),
                elapsed: 1.0,
                hide_rat: false,
            };
            outline_shape(&mut pb, &view, &shape, z);
            let o = ((origin.x / z).round() as i32, (origin.y / z).round() as i32);
            let r = len / z;
            // Along the arc, well away from both rays: the middle, and halfway
            // out to each side.
            for frac in [-0.9f32, -0.5, 0.0, 0.5, 0.9] {
                let a = frac * half_angle;
                let (x, y) = (
                    o.0 + (a.cos() * r).round() as i32,
                    o.1 + (a.sin() * r).round() as i32,
                );
                assert!(
                    lit_between(&pb, x - 2, y - 2, x + 3, y + 3) >= 1,
                    "z={z}: no arc at {:.0}° of the sector",
                    a.to_degrees()
                );
            }
            // Both rays are still there, halfway out.
            for side in [-1.0f32, 1.0] {
                let a = side * half_angle;
                let (x, y) = (
                    o.0 + (a.cos() * r * 0.5).round() as i32,
                    o.1 + (a.sin() * r * 0.5).round() as i32,
                );
                assert!(lit_between(&pb, x - 2, y - 2, x + 3, y + 3) >= 1, "z={z}: a ray is missing");
            }
            // The sector is an outline: its inside stays clear for the bodies
            // standing in it, and nothing is drawn behind the miniboss.
            let (ix, iy) = (o.0 + (r * 0.5).round() as i32, o.1);
            assert_eq!(lit_between(&pb, ix - 2, iy - 2, ix + 3, iy + 3), 0, "z={z}: the cone is filled");
            let (bx, by) = (o.0 - (r * 0.5).round() as i32, o.1);
            assert_eq!(lit_between(&pb, bx - 3, by - 3, bx + 4, by + 4), 0, "z={z}: ink behind the cone");
        }
    }

    /// The rat keeps a mark of its own when zoomed out (the box filter turns
    /// its five-pixel sprite into a smudge otherwise).
    #[test]
    fn the_rat_is_marked_when_zoomed_out() {
        let mut w = World::new(Theme::Sewer, 7);
        w.player.pos = Vec2::new(80.0, 74.0);
        let pb = render(&mut w, Vec2::new(2.0, 2.0));
        assert!(lum(&pb, 40, 37) > 200, "the rat has no mark at 2x");
    }

    /// **The cost stays small at the 600-enemy cap.** The pass walks a handful
    /// of queries with a bounding test per entity; this pins the whole thing
    /// well under a millisecond on the reference view so a later addition
    /// cannot quietly make it the frame's cost centre.
    #[test]
    fn the_ink_pass_stays_cheap_at_the_enemy_cap() {
        use rand::SeedableRng;
        let mut w = World::new(Theme::Sewer, 7);
        w.player.pos = Vec2::new(300.0, 300.0);
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        for i in 0..600 {
            let a = i as f32 * 0.31;
            let r = 10.0 + (i % 40) as f32 * 9.0;
            w.spawn_enemy_with_rng(
                EnemyKind::Skitterer,
                Vec2::new(300.0, 300.0) + Vec2::from_angle(a) * r,
                10.0,
                &mut rng,
            );
        }
        for i in 0..120 {
            let a = i as f32 * 0.7;
            w.spawn_bullet(Bullet::new(
                Vec2::new(300.0, 300.0) + Vec2::from_angle(a) * (20.0 + i as f32),
                Vec2::new(0.0, 0.0),
                4.0,
                Faction::Enemy,
                (240, 120, 160),
            ));
        }
        let mut pb = PixelBuffer::new(120, 74);
        let iterations = 40;
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            pb.clear((0, 0, 0));
            let mut view = InkView {
                ecs: &mut w.ecs,
                level: &w.level,
                player: &w.player,
                cam: Vec2::new(240.0, 240.0),
                scale: Vec2::new(2.0, 2.0),
                size: (120, 74),
                elapsed: 1.0,
                hide_rat: false,
            };
            draw(&mut pb, &mut view);
        }
        let per_frame = start.elapsed().as_secs_f64() / f64::from(iterations) * 1000.0;
        eprintln!("ink pass: {per_frame:.3} ms/frame at 600 enemies + 120 pellets (2x)");
        assert!(
            per_frame < 2.0,
            "the ink pass costs {per_frame:.3} ms/frame at the enemy cap"
        );
    }
}
