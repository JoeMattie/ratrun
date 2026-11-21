//! Floating enemy damage numbers rendered directly into `PixelBuffer`.
//!
//! Designed for a compact half-block pixel viewport (~160×80 to 240×120 px):
//! - Tiny, razor-sharp 3×5 pixel digits for normal hits and 4×6 bold digits
//!   for big hits / major kills, backed by a soft dark outline halo so numbers
//!   stay legible over bright hit-flashes, acid pools, and dense hordes.
//! - Aggregates rapid hits on the same enemy (acid pool ticks, Spore Orbit,
//!   multi-pellet Cheese Spray) so the screen climbs smoothly instead of
//!   filling with `1`s.
//! - Pre-allocated fixed capacity with priority eviction and zero per-frame
//!   heap allocations.

use crate::game::weapon::WeaponKind;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Default maximum simultaneous floating damage numbers.
/// Kept lean so dense horde fights stay legible over sprites and bullets.
pub const DEFAULT_CAP: usize = 28;

/// Time window (seconds since last hit) during which subsequent hits on the
/// same enemy merge into the existing popup.
const MERGE_WINDOW: f32 = 0.36;

/// Maximum total time a popup may be kept alive by repeated DoT/orbit merges
/// before a fresh popup splits off.
const MAX_MERGE_AGE: f32 = 1.05;

/// Horizontal/vertical spacing thresholds for anti-overlap nudging.
const OVERLAP_X: f32 = 11.0;
const OVERLAP_Y: f32 = 7.5;

/// Local density limit: if this many visible numbers already occupy a tight
/// radius, suppress redundant minor (<12 dmg, non-kill) ticks so AoE mobs stay
/// readable instead of turning into a stack of overlapping digits.
const CLUSTER_RADIUS_SQ: f32 = 22.0 * 22.0;
const CLUSTER_MINOR_LIMIT: usize = 4;

const MASK_W: usize = 32;
const MASK_H: usize = 10;

/// What dealt the damage — lets acid melts and nukes read at a glance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DamageSource {
    Normal,
    Acid,
    Orbit,
    TailWhip,
    Nuke,
    /// Acid Rain's splash (lime, one big number per drop).
    Rain,
    /// Hantavirus's damage ticks (plum).
    Virus,
}

impl DamageSource {
    pub fn from_weapon(weapon: Option<WeaponKind>) -> Self {
        match weapon {
            Some(WeaponKind::AcidSpit) => DamageSource::Acid,
            Some(WeaponKind::SporeOrbit) => DamageSource::Orbit,
            Some(WeaponKind::TailWhip) => DamageSource::TailWhip,
            _ => DamageSource::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DamageNumber {
    /// Runtime `Enemy::id` (0 = anonymous / spatial-fallback).
    pub enemy_id: u32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub total_dmg: f32,
    /// Remaining display time.
    pub life: f32,
    pub max_life: f32,
    /// Total elapsed time since first created (caps infinite DoT refreshes).
    pub total_age: f32,
    /// Seconds since the most recent merged hit.
    pub since_hit: f32,
    /// Pop-in flash timer (0..1), boosted on spawn and merge.
    pub pop: f32,
    pub is_kill: bool,
    pub source: DamageSource,
}

impl DamageNumber {
    /// Visible once accumulated damage reaches at least 1 or the hit killed.
    #[inline]
    pub fn visible(&self) -> bool {
        self.total_dmg >= 1.0 || self.is_kill
    }

    /// Rounded integer shown to the player (minimum 1 when visible).
    #[inline]
    pub fn display_value(&self) -> u32 {
        (self.total_dmg.round() as u32).max(1)
    }

    #[inline]
    pub fn is_big(&self) -> bool {
        self.total_dmg >= 30.0 || self.source == DamageSource::Nuke
    }

    /// Eviction priority (higher = keep longer under cap pressure).
    #[inline]
    fn priority(&self) -> f32 {
        let mut p = self.total_dmg;
        if self.is_kill {
            p += 40.0;
        }
        if self.source == DamageSource::Nuke {
            p += 120.0;
        }
        if !self.visible() {
            p *= 0.2;
        }
        let frac = (self.life / self.max_life.max(0.01)).clamp(0.0, 1.0);
        p * (0.35 + 0.65 * frac)
    }
}

#[derive(Default, bevy_ecs::resource::Resource)]
pub struct DamageNumbers {
    pub items: Vec<DamageNumber>,
    cap: usize,
    /// Monotonic counter for deterministic slight horizontal stagger.
    seq: u32,
}

impl DamageNumbers {
    pub fn new(cap: usize) -> Self {
        Self {
            items: Vec::with_capacity(cap),
            cap,
            seq: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Record damage dealt to an enemy at `enemy_pos` (with collision `radius`).
    /// Rapid hits on the same enemy merge into one climbing number.
    pub fn record(
        &mut self,
        enemy_id: u32,
        enemy_pos: Vec2,
        enemy_radius: f32,
        amount: f32,
        is_kill: bool,
        source: DamageSource,
    ) {
        if self.cap == 0 || amount <= 0.0 || !amount.is_finite() {
            return;
        }

        // Try to merge into an active popup for the same enemy.
        if let Some(idx) = self.find_merge_target(enemy_id, enemy_pos, enemy_radius) {
            let entry = &mut self.items[idx];
            entry.total_dmg += amount;
            entry.since_hit = 0.0;
            entry.pop = 1.0;
            if is_kill {
                entry.is_kill = true;
            }
            if source == DamageSource::Nuke {
                entry.source = DamageSource::Nuke;
            } else if source != DamageSource::Acid && entry.source == DamageSource::Acid {
                // Direct hit overrides an acid tint so solid impacts pop.
                entry.source = source;
            }
            let target_life = base_lifetime(entry.total_dmg, entry.is_kill, entry.source);
            entry.max_life = target_life;
            entry.life = target_life;
            // Track a moving enemy gently while climbing, staying above its head.
            let anchor = Vec2::new(enemy_pos.x, enemy_pos.y - enemy_radius - 3.5);
            entry.pos.x += (anchor.x - entry.pos.x) * 0.35;
            entry.pos.y = entry.pos.y.min(anchor.y);
            entry.vel.y = entry.vel.y.min(-18.0);
            return;
        }

        // Compute deterministic slight horizontal stagger so cluster hits breathe.
        self.seq = self.seq.wrapping_add(1);
        let stagger = (((self.seq.wrapping_mul(2654435761) >> 24) & 7) as f32 - 3.5) * 1.1;
        let mut spawn_pos = Vec2::new(
            enemy_pos.x + stagger,
            enemy_pos.y - enemy_radius - 4.0,
        );

        // Local density throttle: in a packed AoE horde, don't bury big hits
        // and kills under dozens of minor ticks in the exact same 22px patch.
        if !is_kill && amount < 12.0 && source != DamageSource::Nuke {
            let nearby = self
                .items
                .iter()
                .filter(|o| o.visible() && o.pos.dist_sq(spawn_pos) < CLUSTER_RADIUS_SQ)
                .count();
            if nearby >= CLUSTER_MINOR_LIMIT {
                return;
            }
        }

        // Multi-pass separation so simultaneous cluster hits fan out cleanly
        // instead of slicing each other's glyphs in half.
        let mut fan_vx = stagger * 3.0;
        if amount >= 1.0 || is_kill {
            for _ in 0..4 {
                let mut moved = false;
                for other in &self.items {
                    if !other.visible() {
                        continue;
                    }
                    let dx = spawn_pos.x - other.pos.x;
                    let dy = spawn_pos.y - other.pos.y;
                    if dx.abs() < OVERLAP_X && dy.abs() < OVERLAP_Y {
                        let push_x = if dx.abs() < 0.5 {
                            if (self.seq & 1) == 0 { 6.0 } else { -6.0 }
                        } else {
                            dx.signum() * (OVERLAP_X - dx.abs()).clamp(3.0, 7.0)
                        };
                        spawn_pos.x += push_x;
                        spawn_pos.y -= (OVERLAP_Y - dy.abs()).clamp(2.5, 5.5);
                        fan_vx += push_x * 1.4;
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }

        let life = base_lifetime(amount, is_kill, source);
        let vx = fan_vx.clamp(-22.0, 22.0);
        let vy = if is_kill || amount >= 30.0 { -28.0 } else { -22.0 };
        let candidate = DamageNumber {
            enemy_id,
            pos: spawn_pos,
            vel: Vec2::new(vx, vy),
            total_dmg: amount,
            life,
            max_life: life,
            total_age: 0.0,
            since_hit: 0.0,
            pop: 1.0,
            is_kill,
            source,
        };

        if self.items.len() >= self.cap {
            // Priority eviction: replace the lowest-priority slot only if the
            // incoming number outranks it (or is at least comparable).
            let cand_prio = candidate.priority();
            let mut worst_idx = 0usize;
            let mut worst_prio = f32::INFINITY;
            for (i, it) in self.items.iter().enumerate() {
                let p = it.priority();
                if p < worst_prio {
                    worst_prio = p;
                    worst_idx = i;
                }
            }
            if cand_prio >= worst_prio {
                self.items[worst_idx] = candidate;
            }
            return;
        }

        self.items.push(candidate);
    }

    fn find_merge_target(
        &self,
        enemy_id: u32,
        enemy_pos: Vec2,
        enemy_radius: f32,
    ) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut best_since = f32::INFINITY;
        for (i, it) in self.items.iter().enumerate() {
            if it.is_kill || it.since_hit > MERGE_WINDOW || it.total_age > MAX_MERGE_AGE {
                continue;
            }
            let matches = if enemy_id != 0 && it.enemy_id != 0 {
                it.enemy_id == enemy_id
            } else {
                // Spatial fallback for enemies constructed directly in tests (`id == 0`).
                let anchor = Vec2::new(enemy_pos.x, enemy_pos.y - enemy_radius - 3.5);
                it.pos.dist_sq(anchor) <= 81.0
            };
            if matches && it.since_hit < best_since {
                best_since = it.since_hit;
                best = Some(i);
            }
        }
        best
    }

    pub fn update(&mut self, dt: f32) {
        let drag = (1.0 - 4.8 * dt).max(0.0);
        for it in self.items.iter_mut() {
            it.life -= dt;
            it.total_age += dt;
            it.since_hit += dt;
            it.pop = (it.pop - dt * 11.0).max(0.0);
            it.vel = it.vel * drag;
            // Gentle upward drift remains even after the initial pop settles.
            it.pos.x += it.vel.x * dt;
            it.pos.y += (it.vel.y - 4.0) * dt;
        }
        self.items.retain(|it| it.life > 0.0);

        // Soft live separation (bounded by DEFAULT_CAP <= 28) so rising
        // numbers never drift back over each other's glyphs.
        let n = self.items.len();
        let push = (dt * 36.0).min(2.5);
        for i in 0..n {
            if !self.items[i].visible() {
                continue;
            }
            for j in (i + 1)..n {
                if !self.items[j].visible() {
                    continue;
                }
                let dx = self.items[i].pos.x - self.items[j].pos.x;
                let dy = self.items[i].pos.y - self.items[j].pos.y;
                if dx.abs() < OVERLAP_X && dy.abs() < OVERLAP_Y {
                    let sx = if dx.abs() < 0.25 {
                        if (i & 1) == 0 { 1.0 } else { -1.0 }
                    } else {
                        dx.signum()
                    };
                    let sy = if dy.abs() < 0.25 { -1.0 } else { dy.signum() };
                    self.items[i].pos.x += sx * push * 0.7;
                    self.items[j].pos.x -= sx * push * 0.7;
                    self.items[i].pos.y += sy * push * 0.45;
                    self.items[j].pos.y -= sy * push * 0.45;
                }
            }
        }
    }

    pub fn draw(&self, pb: &mut PixelBuffer, cam: Vec2) {
        self.draw_scaled(pb, cam, Vec2::new(1.0, 1.0));
    }

    /// Draw into a **screen-space** buffer: `scale` is the world-pixels per
    /// screen-pixel ratio of the last render (`World::zoom_scale`, `1,1`
    /// unzoomed). Positions scale with the view but the glyphs do not — these
    /// are text-sized read-outs, not world sprites, so a zoomed-out frame keeps
    /// its damage numbers legible instead of box-filtering them into mush.
    pub fn draw_scaled(&self, pb: &mut PixelBuffer, cam: Vec2, scale: Vec2) {
        self.draw_scaled_clear_of(pb, cam, scale, None);
    }

    /// [`draw_scaled`](Self::draw_scaled), but ordinary hit digits whose centre
    /// is inside `focus` (the rat, in world pixels) are clipped.
    ///
    /// The review's crowded-floor capture is a wall of orange digits sitting
    /// exactly on the rat, hiding the pellets that are about to hit it: "clip
    /// or suppress ordinary hit digits inside the immediate warning/rat
    /// footprint. Keep clipped health or boss-critical damage feedback." So a
    /// kill, a big tick and a Nuke always print; a routine 3-damage tick beside
    /// the rat does not (it still prints from the *body* it hit, which is
    /// where the read-out belongs anyway, and every digit further out stays).
    pub fn draw_scaled_clear_of(
        &self,
        pb: &mut PixelBuffer,
        cam: Vec2,
        scale: Vec2,
        focus: Option<(Vec2, f32)>,
    ) {
        if pb.w == 0 || pb.h == 0 || self.items.is_empty() {
            return;
        }
        let (sx, sy) = (scale.x.max(0.01), scale.y.max(0.01));
        let bw = pb.w as i32;
        let bh = pb.h as i32;
        // The clear radius is a *world* radius converted to the screen grid, so
        // it covers the same patch of floor at every zoom.
        let clear = focus.map(|(p, r)| {
            (
                ((p.x - cam.x) / sx).round() as i32,
                ((p.y - cam.y) / sy).round() as i32,
                (r / sx.max(sy)).round().max(4.0) as i32,
            )
        });

        // Draw normal hits first, then big/kill hits on top so big moments
        // never get buried under small ticks.
        for pass in 0..2 {
            let draw_priority = pass == 1;
            for it in &self.items {
                if !it.visible() {
                    continue;
                }
                let high = it.is_kill || it.is_big();
                if high != draw_priority {
                    continue;
                }
                let cx = ((it.pos.x - cam.x) / sx).round() as i32;
                let cy = ((it.pos.y - cam.y) / sy).round() as i32 - (it.pop * 1.6).round() as i32;
                if cx < -18 || cx > bw + 18 || cy < -10 || cy > bh + 10 {
                    continue;
                }
                // Ordinary ticks inside the rat's own footprint are the noise
                // the review photographed. Kills, big ticks and Nukes survive:
                // the player still sees what a big hit did.
                if let Some((fx, fy, fr)) = clear {
                    if !high {
                        let (dx, dy) = (cx - fx, cy - fy);
                        if dx * dx + dy * dy <= fr * fr {
                            continue;
                        }
                    }
                }

                let age_frac = 1.0 - (it.life / it.max_life.max(0.01)).clamp(0.0, 1.0);
                // Crisp for the first 60% of lifetime, then smooth fade.
                let alpha = if age_frac < 0.60 {
                    1.0
                } else {
                    (1.0 - (age_frac - 0.60) / 0.40).clamp(0.0, 1.0)
                };
                if alpha <= 0.02 {
                    continue;
                }

                let base_color = damage_color(it);
                let color = if it.pop > 0.0 {
                    palette::mix(base_color, (255, 255, 255), it.pop * 0.72)
                } else {
                    base_color
                };
                let bold = it.is_big();
                let bang = (it.is_kill && it.total_dmg >= 30.0) || it.source == DamageSource::Nuke;
                draw_number_glyphs(pb, cx, cy, it.display_value(), bold, bang, color, alpha);
            }
        }
    }
}

fn base_lifetime(dmg: f32, is_kill: bool, source: DamageSource) -> f32 {
    if source == DamageSource::Nuke {
        0.92
    } else if dmg >= 30.0 {
        0.76
    } else if is_kill {
        0.60
    } else {
        0.46
    }
}

fn damage_color(it: &DamageNumber) -> Rgb {
    if it.source == DamageSource::Nuke {
        return (255, 220, 110);
    }
    if it.is_kill {
        return if it.total_dmg >= 25.0 {
            (255, 200, 95)
        } else if it.total_dmg >= 12.0 {
            (255, 130, 150)
        } else {
            (255, 118, 148)
        };
    }
    if it.source == DamageSource::Rain {
        // Lime, but hotter and whiter than Acid Spit's pool ticks so a rain
        // splash is not mistaken for a lingering pool.
        return if it.total_dmg >= 20.0 {
            (232, 255, 150)
        } else {
            (198, 255, 96)
        };
    }
    if it.source == DamageSource::Virus {
        return if it.total_dmg >= 15.0 {
            (246, 208, 246)
        } else {
            (214, 130, 206)
        };
    }
    if it.source == DamageSource::Acid {
        return if it.total_dmg >= 20.0 {
            (215, 255, 120)
        } else {
            (168, 248, 108)
        };
    }
    if it.total_dmg >= 25.0 {
        (255, 152, 58)
    } else if it.total_dmg >= 10.0 {
        (255, 220, 102)
    } else if it.source == DamageSource::TailWhip {
        (255, 185, 218)
    } else if it.source == DamageSource::Orbit {
        (178, 252, 205)
    } else {
        (242, 238, 230)
    }
}

/// Format `val` into a stack buffer of ASCII digits (clamped at 9999).
fn format_digits(val: u32, out: &mut [u8; 6]) -> usize {
    let mut n = val.min(9999);
    let mut tmp = [0u8; 6];
    let mut count = 0usize;
    if n == 0 {
        out[0] = 0;
        return 1;
    }
    while n > 0 && count < 6 {
        tmp[count] = (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    for i in 0..count {
        out[i] = tmp[count - 1 - i];
    }
    count
}

/// Crisp 3×5 pixel font (each byte's low 3 bits = row, bit 2 = left pixel).
const FONT_3X5: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b011, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b011, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b110], // 9
];

/// Bold 4×6 pixel font (each byte's low 4 bits = row, bit 3 = left pixel).
const FONT_4X6: [[u8; 6]; 10] = [
    [0b0110, 0b1001, 0b1001, 0b1001, 0b1001, 0b0110], // 0
    [0b0110, 0b1110, 0b0110, 0b0110, 0b0110, 0b1111], // 1
    [0b1110, 0b0001, 0b0110, 0b1000, 0b1000, 0b1111], // 2
    [0b1110, 0b0001, 0b0111, 0b0001, 0b0001, 0b1110], // 3
    [0b1001, 0b1001, 0b1111, 0b0001, 0b0001, 0b0001], // 4
    [0b1111, 0b1000, 0b1110, 0b0001, 0b0001, 0b1110], // 5
    [0b0111, 0b1000, 0b1110, 0b1001, 0b1001, 0b0110], // 6
    [0b1111, 0b0001, 0b0010, 0b0100, 0b0100, 0b0100], // 7
    [0b0110, 0b1001, 0b0110, 0b1001, 0b1001, 0b0110], // 8
    [0b0110, 0b1001, 0b0111, 0b0001, 0b0001, 0b1110], // 9
];

/// Render a centered pixel number into `pb` using a zero-allocation stack mask
/// so the dark outline halo never double-darkens or clips adjacent digits.
#[allow(clippy::too_many_arguments)]
fn draw_number_glyphs(
    pb: &mut PixelBuffer,
    cx: i32,
    cy: i32,
    value: u32,
    bold: bool,
    bang: bool,
    color: Rgb,
    alpha: f32,
) {
    let mut digits = [0u8; 6];
    let count = format_digits(value, &mut digits);

    let gw = if bold { 4usize } else { 3usize };
    let gh = if bold { 6usize } else { 5usize };
    let gap = 1usize;
    let bang_w = if bang { 2usize } else { 0usize }; // 1px gap + 1px `!`
    let text_w = count * gw + (count.saturating_sub(1)) * gap + bang_w;

    // 1 px margin all around for outline halo (+ 1 px bottom-right drop shadow).
    let mask_w = (text_w + 3).min(MASK_W);
    let mask_h = (gh + 3).min(MASK_H);
    let mut mask = [0u8; MASK_W * MASK_H];

    // Pass 1: stamp foreground glyph pixels as `2`.
    let mut cursor = 1usize;
    for &d in &digits[..count] {
        if bold {
            let rows = FONT_4X6[d as usize];
            for (ry, &bits) in rows.iter().enumerate() {
                for rx in 0..4usize {
                    if (bits & (1 << (3 - rx))) != 0 {
                        let mx = cursor + rx;
                        let my = 1 + ry;
                        if mx < mask_w && my < mask_h {
                            mask[my * MASK_W + mx] = 2;
                        }
                    }
                }
            }
        } else {
            let rows = FONT_3X5[d as usize];
            for (ry, &bits) in rows.iter().enumerate() {
                for rx in 0..3usize {
                    if (bits & (1 << (2 - rx))) != 0 {
                        let mx = cursor + rx;
                        let my = 1 + ry;
                        if mx < mask_w && my < mask_h {
                            mask[my * MASK_W + mx] = 2;
                        }
                    }
                }
            }
        }
        cursor += gw + gap;
    }

    if bang {
        let mx = cursor;
        if mx < mask_w {
            let stem_end = gh.saturating_sub(2);
            for ry in 0..stem_end {
                let my = 1 + ry;
                if my < mask_h {
                    mask[my * MASK_W + mx] = 2;
                }
            }
            let dot_y = 1 + gh - 1;
            if dot_y < mask_h {
                mask[dot_y * MASK_W + mx] = 2;
            }
        }
    }

    // Pass 2: mark 4-neighbor outline halo + down-right drop shadow as `1`.
    for my in 0..mask_h {
        for mx in 0..mask_w {
            if mask[my * MASK_W + mx] != 0 {
                continue;
            }
            let left = mx > 0 && mask[my * MASK_W + (mx - 1)] == 2;
            let right = mx + 1 < mask_w && mask[my * MASK_W + (mx + 1)] == 2;
            let up = my > 0 && mask[(my - 1) * MASK_W + mx] == 2;
            let down = my + 1 < mask_h && mask[(my + 1) * MASK_W + mx] == 2;
            // Subtle down-right drop shadow matches the game's top-left light.
            let shadow = mx > 0 && my > 0 && mask[(my - 1) * MASK_W + (mx - 1)] == 2;
            if left || right || up || down || shadow {
                mask[my * MASK_W + mx] = 1;
            }
        }
    }

    // Pass 3: composite onto the PixelBuffer.
    let origin_x = cx - (text_w as i32) / 2 - 1;
    let origin_y = cy - (gh as i32) / 2 - 1;
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let top_tint = palette::mix(color, (255, 255, 255), 0.24);
    let bot_tint = palette::scale(color, 0.88);
    let halo_col: Rgb = (6, 8, 12);

    for my in 0..mask_h {
        let sy = origin_y + my as i32;
        if sy < 0 || sy >= bh {
            continue;
        }
        for mx in 0..mask_w {
            let cell = mask[my * MASK_W + mx];
            if cell == 0 {
                continue;
            }
            let sx = origin_x + mx as i32;
            if sx < 0 || sx >= bw {
                continue;
            }
            let bg = pb.pixel_at(sx as usize, sy as usize);
            if cell == 1 {
                pb.plot(sx, sy, palette::mix(bg, halo_col, 0.78 * alpha));
            } else {
                // Subtle top-to-bottom pixel-art bevel matching top-left lighting.
                let shade = if my <= 2 {
                    top_tint
                } else if my >= gh {
                    bot_tint
                } else {
                    color
                };
                pb.plot(sx, sy, palette::mix(bg, shade, alpha));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_hits_on_same_enemy_aggregate() {
        let mut dn = DamageNumbers::new(DEFAULT_CAP);
        let pos = Vec2::new(50.0, 50.0);
        // Simulate 10 rapid acid pool ticks of 0.25 dmg each (total 2.5).
        for _ in 0..10 {
            dn.record(7, pos, 3.0, 0.25, false, DamageSource::Acid);
            dn.update(1.0 / 60.0);
        }
        assert_eq!(dn.len(), 1, "rapid ticks should fold into a single number");
        assert!(dn.items[0].visible());
        assert_eq!(dn.items[0].display_value(), 3);
    }

    #[test]
    fn sub_integer_dot_stays_hidden_until_reaching_one() {
        let mut dn = DamageNumbers::new(DEFAULT_CAP);
        let pos = Vec2::new(30.0, 30.0);
        dn.record(42, pos, 3.0, 0.2, false, DamageSource::Acid);
        assert_eq!(dn.len(), 1);
        assert!(!dn.items[0].visible(), "sub-1 acid tick should not flash 1");

        // Drawing while invisible must not paint anything.
        let mut pb = PixelBuffer::new(60, 60);
        pb.clear((20, 24, 28));
        dn.draw(&mut pb, Vec2::ZERO);
        for y in 0..pb.h {
            for x in 0..pb.w {
                assert_eq!(pb.pixel_at(x, y), (20, 24, 28));
            }
        }

        // Accumulating past 1.0 makes it visible.
        for _ in 0..4 {
            dn.record(42, pos, 3.0, 0.25, false, DamageSource::Acid);
        }
        assert!(dn.items[0].visible());
        dn.draw(&mut pb, Vec2::ZERO);
        let changed = (0..pb.h)
            .flat_map(|y| (0..pb.w).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != (20, 24, 28))
            .count();
        assert!(changed > 5, "visible damage number should render pixels");
    }

    #[test]
    fn killing_blow_upgrades_aggregated_number_in_place() {
        let mut dn = DamageNumbers::new(DEFAULT_CAP);
        let pos = Vec2::new(40.0, 40.0);
        dn.record(9, pos, 3.0, 4.0, false, DamageSource::Normal);
        dn.update(0.05);
        dn.record(9, pos, 3.0, 5.0, true, DamageSource::Normal);
        assert_eq!(dn.len(), 1);
        assert!(dn.items[0].is_kill);
        assert_eq!(dn.items[0].display_value(), 9);
    }

    #[test]
    fn cap_is_respected_and_big_hits_survive_eviction() {
        let mut dn = DamageNumbers::new(8);
        // Record a huge boss nuke hit first.
        dn.record(1, Vec2::new(40.0, 40.0), 14.0, 200.0, false, DamageSource::Nuke);
        // Flood with 30 tiny 2-damage hits on other enemies.
        for id in 2..32u32 {
            dn.record(
                id,
                Vec2::new(10.0 + id as f32 * 3.0, 40.0),
                2.5,
                2.0,
                false,
                DamageSource::Normal,
            );
        }
        assert_eq!(dn.len(), 8);
        assert!(
            dn.items.iter().any(|it| it.enemy_id == 1 && it.display_value() == 200),
            "priority eviction must preserve big hits over minor ticks"
        );
    }

    #[test]
    fn zero_cap_and_tiny_viewports_never_panic() {
        let mut dn = DamageNumbers::default();
        dn.record(1, Vec2::ZERO, 2.0, 10.0, true, DamageSource::Normal);
        assert!(dn.is_empty());

        let mut dn2 = DamageNumbers::new(8);
        dn2.record(1, Vec2::new(0.0, 0.0), 2.0, 99999.0, true, DamageSource::Nuke);
        for (w, h) in [(0, 0), (1, 1), (4, 4)] {
            let mut pb = PixelBuffer::new(w, h);
            dn2.draw(&mut pb, Vec2::ZERO);
        }
    }

    #[test]
    fn numbers_expire_cleanly() {
        let mut dn = DamageNumbers::new(16);
        dn.record(1, Vec2::new(20.0, 20.0), 3.0, 7.0, false, DamageSource::Normal);
        assert_eq!(dn.len(), 1);
        for _ in 0..120 {
            dn.update(1.0 / 60.0);
        }
        assert!(dn.is_empty());
    }
}
