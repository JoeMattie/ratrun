//! Outfall ordinary enemies. Steering commits body motion before the grid
//! builds; this floor's late pass resolves the warned hits against the rat.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::SizeClass;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{DebugFlags, SfxQueue};
use crate::game::ecs::systems::boss::{Boss, BossKind, HazardShape};
use crate::game::ecs::systems::projectiles::los_clear;
use crate::game::enemy::{Enemy, EnemyKind, OutfallAttack};
use crate::game::hazards::{clamp_outfall_displacement, outfall_sand_clear};
use crate::game::level::{Level, SeabedKind, Theme};
use crate::game::player::Player;
use crate::math::Vec2;

pub const SKATER_WET_SPEED: f32 = 36.0;
pub const SKATER_WATER_REACH: f32 = 48.0;
pub const SKATER_SPLASH_LEN: f32 = 22.0;
pub const SKATER_SPLASH_WIDTH: f32 = 4.0;
pub const SKATER_TELL_SECS: f32 = 0.45;
pub const SKATER_REARM_SECS: f32 = 3.2;
pub const SKATER_SPLASH_DAMAGE: f32 = 6.0;

pub const LAMPFISH_BULB_OFFSET: f32 = 16.0;
pub const LAMPFISH_LIGHT_RADIUS: f32 = 26.0;
pub const LAMPFISH_LIGHT_FLICKER_HZ: f32 = 3.0;
pub const LAMPFISH_TRIGGER_REACH: f32 = 12.0;
pub const LAMPFISH_TELL_SECS: f32 = 0.4;
pub const LAMPFISH_LUNGE_LEN: f32 = 18.0;
pub const LAMPFISH_LUNGE_SECS: f32 = 0.18;
pub const LAMPFISH_REARM_SECS: f32 = 2.8;
pub const LAMPFISH_LUNGE_DAMAGE: f32 = 8.0;

pub const GROUPER_SNUFF_REACH: f32 = 96.0;
pub const SCHOOL_LIGHT_REACH: f32 = 48.0;
pub const SCHOOL_RAT_REACH: f32 = 36.0;
pub const SCHOOL_BUNCHED_RADIUS: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutfallTellPhase {
    Tell,
    Strike,
}

/// Locked footprint and current phase shared by the hostile pass and bot.
#[derive(Clone, Copy, Debug)]
pub struct OutfallTell {
    pub shape: HazardShape,
    pub remaining: f32,
    pub phase: OutfallTellPhase,
}

pub fn outfall_enemy_tell(e: &Enemy) -> Option<OutfallTell> {
    if !e.targetable() {
        return None;
    }
    let (start, dir, remaining, phase) = match e.outfall.attack {
        OutfallAttack::Idle => return None,
        OutfallAttack::Tell { start, dir, remaining } => {
            (start, dir, remaining, OutfallTellPhase::Tell)
        }
        OutfallAttack::Splash { start, dir, .. } => (start, dir, 0.0, OutfallTellPhase::Strike),
        OutfallAttack::Lunge { start, dir, remaining, .. } => {
            (start, dir, remaining, OutfallTellPhase::Strike)
        }
    };
    Some(OutfallTell {
        shape: attack_shape(e, start, dir),
        remaining,
        phase,
    })
}

pub fn lampfish_bulb(e: &Enemy) -> Option<Vec2> {
    (e.kind == EnemyKind::Lampfish && e.targetable())
        .then(|| e.pos + facing(e) * LAMPFISH_BULB_OFFSET)
}

fn facing(e: &Enemy) -> Vec2 {
    let dir = e.facing.normalized();
    if dir == Vec2::ZERO { Vec2::new(1.0, 0.0) } else { dir }
}

fn attack_shape(e: &Enemy, start: Vec2, dir: Vec2) -> HazardShape {
    if e.kind == EnemyKind::Skater {
        HazardShape::Rect {
            center: start + dir * (SKATER_SPLASH_LEN * 0.5),
            facing: dir,
            half: SKATER_SPLASH_LEN * 0.5,
            half_perp: SKATER_SPLASH_WIDTH * 0.5,
        }
    } else {
        lunge_segment(e.radius, start, start + dir * LAMPFISH_LUNGE_LEN)
    }
}

fn lunge_segment(radius: f32, from: Vec2, to: Vec2) -> HazardShape {
    HazardShape::Corridor { from, to, half_width: radius }
}

/// Refresh before miniboss aim locks. The late pass refreshes again after
/// movement, boss fixture writes and damage, so a dead elite restores its
/// lamp on the same frame and the boss slot always takes precedence.
pub fn outfall_snuff_system(
    mut level: ResMut<Level>,
    enemies: Query<(&SpawnId, &Enemy)>,
    bosses: Query<&Boss>,
) {
    if level.theme == Theme::Outfall {
        // A new elite claim during the Lantern fight goes through after the
        // boss's fixture selection. An established claim stays dark.
        let defer_new = bosses.iter().any(|boss| boss.kind == BossKind::Lantern && !boss.dead);
        refresh_snuff(&mut level, enemies.iter(), defer_new);
    }
}

fn refresh_snuff<'a>(
    level: &mut Level,
    enemies: impl Iterator<Item = (&'a SpawnId, &'a Enemy)>,
    defer_new: bool,
) {
    let previous = level.objects.outfall_elite_snuff.take();
    let mut best: Option<(f32, u64, usize)> = None;
    for (id, e) in enemies {
        if e.kind != EnemyKind::Grouper || !e.elite || e.named || !e.targetable() {
            continue;
        }
        if let Some((index, pos)) = level.nearest_live_sconce(e.pos, GROUPER_SNUFF_REACH) {
            let key = (e.pos.dist_sq(pos), id.0, index);
            if best.is_none_or(|old| key < old) {
                best = Some(key);
            }
        }
    }
    level.objects.outfall_elite_snuff = best
        .map(|(_, _, index)| index)
        .filter(|&index| !defer_new || Some(index) == previous);
}

/// Snapshot the gameplay bulbs before steering so a Lampfish processed
/// earlier in SpawnId order cannot give a school a different lighting test.
pub(super) fn collect_school_lights<'a>(
    level: &Level,
    enemies: impl Iterator<Item = &'a Enemy>,
    bosses: impl Iterator<Item = &'a Boss>,
    bulbs: &mut Vec<Vec2>,
) {
    bulbs.clear();
    bulbs.extend(enemies.filter_map(lampfish_bulb));
    bulbs.extend(level.objects.lure_traps.iter().filter(|lure| lure.is_lit()).map(|lure| lure.pos));
    bulbs.extend(bosses.filter_map(|boss| Some(boss.lantern_view()?.real_bulb?.pos)));
}

fn visible_light(level: &Level, from: Vec2, pos: Vec2, reach: f32) -> bool {
    from.dist_sq(pos) <= reach * reach && level.tiles.segment_hit(from, pos, 0.0).is_none()
}

fn update_school(e: &mut Enemy, level: &Level, rat: Vec2, bulbs: &[Vec2]) {
    let lit = visible_light(level, e.pos, rat, SCHOOL_RAT_REACH)
        || level.nearest_live_sconce(e.pos, SCHOOL_LIGHT_REACH).is_some()
        || bulbs.iter().any(|&pos| visible_light(level, e.pos, pos, SCHOOL_LIGHT_REACH));
    let dark_radius = e.kind.radius();
    e.outfall.bunched = lit
        || (e.outfall.bunched && !outfall_sand_clear(level, e.pos, dark_radius, SizeClass::Large));
    e.radius = if e.outfall.bunched { SCHOOL_BUNCHED_RADIUS } else { dark_radius };
    // In darkness it can swim the smaller body away from a wall before
    // expanding. Locking it here would leave the school parked indefinitely.
    e.position_locked = lit;
    if e.position_locked {
        e.vel = Vec2::ZERO;
    }
}

/// The ground a Skater can skim. Any standing water (puddles, which the
/// Outfall no longer has) plus, on the Outfall, the loose **Silt and Shingle**
/// seabed patches that replaced its lakes. The trick is unchanged: while it
/// stands on that ground it can release its splash, and stepping off it
/// cancels the attack.
pub(super) fn skater_ground(level: &Level, p: Vec2) -> bool {
    if level.wet_at(p) {
        return true;
    }
    level.theme == Theme::Outfall
        && level.seabed_patches.iter().any(|patch| {
            matches!(patch.kind, SeabedKind::Silt | SeabedKind::Shingle) && patch.contains(p)
        })
}

/// The nearest reachable sample of one blob (a puddle or a seabed patch), with
/// its squared distance. The local two-pixel lattice is clipped to the blob's
/// bounds, each sample is tested against the blob's actual rim, and the first
/// equidistant sample in row and column order wins.
fn nearest_on_ground<F>(
    level: &Level,
    from: Vec2,
    radius: f32,
    center: Vec2,
    rx: f32,
    ry: f32,
    covered: F,
) -> Option<(f32, Vec2)>
where
    F: Fn(Vec2) -> bool,
{
    let class = SizeClass::of_radius(radius);
    let rel = center - from;
    let x0 = ((rel.x - rx).max(-SKATER_WATER_REACH) * 0.5).ceil() as i32;
    let x1 = ((rel.x + rx).min(SKATER_WATER_REACH) * 0.5).floor() as i32;
    let y0 = ((rel.y - ry).max(-SKATER_WATER_REACH) * 0.5).ceil() as i32;
    let y1 = ((rel.y + ry).min(SKATER_WATER_REACH) * 0.5).floor() as i32;
    let mut best: Option<(f32, Vec2)> = None;
    let mut best_distance = SKATER_WATER_REACH * SKATER_WATER_REACH;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let step = Vec2::new(x as f32 * 2.0, y as f32 * 2.0);
            let distance = step.len_sq();
            if distance > best_distance || (best.is_some() && distance == best_distance) {
                continue;
            }
            let at = from + step;
            if level.tiles.get_at(at).is_open()
                && covered(at)
                && clamp_outfall_displacement(level, from, step, radius, class) == at
            {
                best = Some((distance, at));
                best_distance = distance;
            }
        }
    }
    best
}

/// Pick the nearest reachable soft ground within [`SKATER_WATER_REACH`]: a
/// puddle anywhere, or a Silt/Shingle seabed patch on the Outfall.
fn nearest_water(level: &Level, from: Vec2, radius: f32) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2)> = None;
    let mut consider = |sample: Option<(f32, Vec2)>| {
        if let Some((distance, at)) = sample {
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, at));
            }
        }
    };
    for puddle in &level.puddles {
        consider(nearest_on_ground(
            level,
            from,
            radius,
            puddle.pos,
            puddle.rx,
            puddle.ry,
            |at| puddle.contains(at),
        ));
    }
    if level.theme == Theme::Outfall {
        for patch in level
            .seabed_patches
            .iter()
            .filter(|patch| matches!(patch.kind, SeabedKind::Silt | SeabedKind::Shingle))
        {
            consider(nearest_on_ground(
                level,
                from,
                radius,
                patch.center,
                patch.rx,
                patch.ry,
                |at| patch.contains(at),
            ));
        }
    }
    best.map(|(_, at)| at)
}

fn lane_clear(e: &Enemy, level: &Level, start: Vec2, dir: Vec2) -> bool {
    if e.kind == EnemyKind::Skater {
        let step = dir * SKATER_SPLASH_LEN;
        clamp_outfall_displacement(level, start, step, SKATER_SPLASH_WIDTH * 0.5, SizeClass::Small)
            == start + step
    } else {
        let to = start + dir * LAMPFISH_LUNGE_LEN;
        los_clear(level, start, to, e.radius)
            && (1..=5).all(|sample| {
                level.tiles.circle_clear(start + (to - start) * (sample as f32 / 5.0),
                    e.radius, SizeClass::Small)
            })
    }
}

pub(super) fn cancel_attack(e: &mut Enemy) {
    if e.kind == EnemyKind::Lampfish {
        e.outfall.rearm = LAMPFISH_REARM_SECS;
    }
    e.outfall.attack = OutfallAttack::Idle;
    e.position_locked = false;
}

fn advance_attack(e: &mut Enemy, level: &Level, wet: bool, dt: f32) {
    e.outfall.rearm = (e.outfall.rearm - dt).max(0.0);
    let (start, dir, remaining, hit) = match e.outfall.attack {
        OutfallAttack::Idle => return,
        OutfallAttack::Splash { .. } => {
            e.outfall.attack = OutfallAttack::Idle;
            e.position_locked = false;
            return;
        }
        OutfallAttack::Tell { start, dir, remaining } => {
            let valid_plant = if e.kind == EnemyKind::Skater {
                wet
            } else {
                e.pos.dist_sq(start) < 1e-4
            };
            if e.stun > 0.0 || !valid_plant || !lane_clear(e, level, start, dir) {
                cancel_attack(e);
                return;
            }
            if remaining > dt {
                e.outfall.attack = OutfallAttack::Tell { start, dir, remaining: remaining - dt };
                return;
            }
            if e.kind == EnemyKind::Skater {
            e.outfall.attack = OutfallAttack::Splash { start, dir, spent: false };
                return;
            }
            let lunge_dt = dt - remaining;
            step_lunge(e, start, dir, LAMPFISH_LUNGE_SECS - lunge_dt, false, dt);
            return;
        }
        OutfallAttack::Lunge { start, dir, remaining, hit, .. } => (start, dir, remaining, hit),
    };
    let expected = lunge_position(start, dir, remaining);
    if e.stun > 0.0 || e.pos.dist_sq(expected) >= 1e-4 || !lane_clear(e, level, start, dir) {
        cancel_attack(e);
        return;
    }
    step_lunge(e, start, dir, remaining - dt, hit, dt);
}

fn lunge_position(start: Vec2, dir: Vec2, remaining: f32) -> Vec2 {
    let progress = (1.0 - remaining / LAMPFISH_LUNGE_SECS).clamp(0.0, 1.0);
    start + dir * (progress * LAMPFISH_LUNGE_LEN)
}

fn step_lunge(e: &mut Enemy, start: Vec2, dir: Vec2, remaining: f32, hit: bool, dt: f32) {
    let previous = e.pos;
    let remaining = remaining.max(0.0);
    e.pos = lunge_position(start, dir, remaining);
    // Ordinary contact runs before the locked sweep's late damage pass.
    e.contact_cd = e.contact_cd.max(dt + 1e-4);
    e.outfall.attack = OutfallAttack::Lunge { start, dir, remaining, previous, hit };
}

/// Steering overrides are applied once, before the common mover and grid.
#[allow(clippy::too_many_arguments)]
pub(super) fn steering_intent(
    e: &mut Enemy,
    level: &Level,
    rat: Vec2,
    rat_wet: bool,
    bulbs: &[Vec2],
    target: &mut Vec2,
    flow_dir: &mut Vec2,
    dt: f32,
) -> f32 {
    match e.kind {
        EnemyKind::Skater => {
            let wet = skater_ground(level, e.pos);
            advance_attack(e, level, wet, dt);
            if !e.position_locked && !rat_wet && *target == rat {
                if wet {
                    *target = e.pos;
                    *flow_dir = Vec2::ZERO;
                    e.vel = Vec2::ZERO;
                } else if let Some(water) = nearest_water(level, e.pos, e.radius) {
                    *target = water;
                    *flow_dir = (water - e.pos).normalized();
                }
            }
            if wet { SKATER_WET_SPEED / e.kind.speed() } else { 1.0 }
        }
        EnemyKind::Lampfish => {
            advance_attack(e, level, false, dt);
            1.0
        }
        EnemyKind::FrySchool => {
            update_school(e, level, rat, bulbs);
            1.0
        }
        _ => 1.0,
    }
}

fn arm_attack(e: &mut Enemy, level: &Level, rat: Vec2) -> bool {
    if e.outfall.rearm > 0.0 || e.stun > 0.0 {
        return false;
    }
    let start = e.pos;
    let aim = (rat - start).normalized();
    let dir = if aim == Vec2::ZERO { facing(e) } else { aim };
    let remaining = if e.kind == EnemyKind::Skater {
        let reach = SKATER_SPLASH_LEN + PLAYER_RADIUS;
        if rat.dist_sq(start) > reach * reach
            || !skater_ground(level, start)
        {
            return false;
        }
        SKATER_TELL_SECS
    } else {
        if rat.dist_sq(start) > LAMPFISH_TRIGGER_REACH * LAMPFISH_TRIGGER_REACH {
            return false;
        }
        LAMPFISH_TELL_SECS
    };
    if !los_clear(level, start, rat, 0.0) || !lane_clear(e, level, start, dir) {
        return false;
    }
    if e.kind == EnemyKind::Skater {
        e.outfall.rearm = SKATER_REARM_SECS;
    }
    e.outfall.attack = OutfallAttack::Tell { start, dir, remaining };
    e.position_locked = true;
    e.vel = Vec2::ZERO;
    e.facing = dir;
    true
}

fn damage_rat(
    e: &Enemy,
    player: &mut Player,
    base: f32,
    sfx: &mut SfxQueue,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) {
    let damage = base * (e.damage / e.kind.damage()) * dmg_scale;
    if player.take_damage(damage) {
        sfx.0.push(Sfx::Hurt);
    }
}

fn resolve_attack(
    e: &mut Enemy,
    level: &Level,
    player: &mut Player,
    sfx: &mut SfxQueue,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) {
    match e.outfall.attack {
        OutfallAttack::Idle => {
            if arm_attack(e, level, player.pos) {
                sfx.0.push(Sfx::Warn);
            }
        }
        OutfallAttack::Tell { .. } => {}
        OutfallAttack::Splash { start, dir, spent: false } => {
            if e.stun > 0.0 || !skater_ground(level, e.pos) || !lane_clear(e, level, start, dir) {
                cancel_attack(e);
            } else {
                if attack_shape(e, start, dir).contains(player.pos, PLAYER_RADIUS + player.buffs.giant_hit_padding()) {
                    damage_rat(e, player, SKATER_SPLASH_DAMAGE, sfx, dmg_scale);
                }
                e.outfall.attack = OutfallAttack::Splash { start, dir, spent: true };
            }
        }
        OutfallAttack::Splash { spent: true, .. } => {}
        OutfallAttack::Lunge { start, dir, remaining, previous, mut hit } => {
            if e.stun > 0.0 {
                cancel_attack(e);
                return;
            }
            if !hit && lunge_segment(e.radius, previous, e.pos).contains(player.pos, PLAYER_RADIUS + player.buffs.giant_hit_padding()) {
                damage_rat(e, player, LAMPFISH_LUNGE_DAMAGE, sfx, dmg_scale);
                hit = true;
            }
            if remaining <= 0.0 {
                cancel_attack(e);
            } else {
                e.outfall.attack = OutfallAttack::Lunge { start, dir, remaining, previous, hit };
            }
        }
    }
}

/// Run after wave4. The same SpawnId order decides competing warned hits and
/// the phase changes seen by art, with no spawns or run-stream draws.
pub fn floor_outfall_system(
    debug: Res<DebugFlags>,
    mut level: ResMut<Level>,
    mut player: ResMut<Player>,
    mut sfx: ResMut<SfxQueue>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    // The floor's hit softening (`[enemies] sector_damage_mult`): the Skater's
    // splash and the Lampfish's lunge carry it.
    sector: crate::game::ecs::resources::SectorDamage,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    if level.theme != Theme::Outfall {
        return;
    }
    let dmg_scale = sector.mult();
    refresh_snuff(&mut level, enemies.iter().map(|(_, id, e)| (id, e)), false);
    if debug.freeze {
        return;
    }
    order.clear();
    order.extend(enemies.iter().filter(|(_, _, e)| {
        e.targetable() && !e.crab_pot_held && !e.named
            && matches!(e.kind, EnemyKind::Skater | EnemyKind::Lampfish)
    }).map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|&(id, _)| id);
    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else { continue };
        resolve_attack(&mut e, &level, &mut player, &mut sfx, dmg_scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;
    use crate::game::level::{LightFixture, LightFixtureKind, Puddle, SeabedKind, SeabedPatch};
    use crate::game::weapon::WeaponKind;

    const START: Vec2 = Vec2 { x: 104.0, y: 104.0 };
    const RIGHT: Vec2 = Vec2 { x: 1.0, y: 0.0 };

    fn enemy(kind: EnemyKind, pos: Vec2) -> Enemy {
        Enemy::spawn_with_phases(kind, pos, 1.0, 2.2, 0.0, 0.0)
    }

    fn puddle(pos: Vec2, radius: f32) -> Puddle {
        Puddle { pos, rx: radius, ry: radius, seed: 7 }
    }

    #[test]
    fn skater_water_seek_takes_a_clear_swept_path() {
        let mut level = Level::empty(Theme::Outfall);
        level.puddles.push(puddle(START + RIGHT * 30.0, 4.0));
        level.puddles.push(puddle(START - RIGHT * 38.0, 6.0));
        let open = nearest_water(&level, START, 3.0).expect("right puddle is reachable");
        assert!(open.x > START.x && level.wet_at(open));

        level.tiles.set(7, 6, TileKind::Wall(WallKind::Solid));
        level.tiles.rebuild_masks();
        let around = nearest_water(&level, START, 3.0).expect("left puddle is reachable");
        assert!(around.x < START.x && level.wet_at(around));
        level.puddles.pop();
        assert!(nearest_water(&level, START, 3.0).is_none());
    }

    #[test]
    fn skater_skims_the_soft_seabed_patches_too() {
        // The Outfall has no puddles any more: the Silt and Shingle patches are
        // its soft ground, and the Skater's whole trick follows them.
        let mut level = Level::empty(Theme::Outfall);
        level.seabed_patches.push(SeabedPatch {
            center: START + RIGHT * 20.0,
            rx: 30.0,
            ry: 22.0,
            kind: SeabedKind::Silt,
            seed: 3,
        });
        assert!(skater_ground(&level, START + RIGHT * 20.0));
        let at = nearest_water(&level, START, 3.0).expect("the Silt is reachable");
        assert!(skater_ground(&level, at));
        let mut e = enemy(EnemyKind::Skater, START + RIGHT * 20.0);
        assert!(arm_attack(&mut e, &level, START + RIGHT * 36.0));
        // Hard ground is not soft ground, and stepping off cancels the tell.
        level.tiles.set(7, 6, TileKind::Wall(WallKind::Solid));
        level.seabed_patches.clear();
        assert!(!skater_ground(&level, START));
        assert!(nearest_water(&level, START, 3.0).is_none());
        advance_attack(&mut e, &level, false, SKATER_TELL_SECS);
        assert!(outfall_enemy_tell(&e).is_none());
    }

    #[test]
    fn skater_warns_then_splashes_once_with_spawn_damage_scaling() {
        let mut level = Level::empty(Theme::Outfall);
        level.puddles.push(puddle(START, 32.0));
        let mut e = enemy(EnemyKind::Skater, START);
        e.damage *= 1.5;
        let rat_pos = START + RIGHT * 20.0;
        let mut rat = Player::new(rat_pos, WeaponKind::Gnaw);
        rat.buffs.giant = 1.0;
        rat.loadout.stats.armor = 1.0;
        let mut sfx = SfxQueue::default();
        let hp = rat.hp;

        resolve_attack(&mut e, &level, &mut rat, &mut sfx, 1.0);
        let tell = outfall_enemy_tell(&e).expect("warn before release");
        assert_eq!(tell.phase, OutfallTellPhase::Tell);
        assert!(tell.shape.contains(rat_pos, 0.0));
        assert!(!tell.shape.contains(rat_pos + Vec2::new(0.0, 3.0), 0.0));
        assert_eq!(rat.hp, hp);
        advance_attack(&mut e, &level, true, SKATER_TELL_SECS);
        resolve_attack(&mut e, &level, &mut rat, &mut sfx, 1.0);
        assert!((hp - rat.hp - (SKATER_SPLASH_DAMAGE * 1.5 * 0.5 - 1.0)).abs() < 1e-4);
        let after = rat.hp;
        rat.iframes = 0.0;
        resolve_attack(&mut e, &level, &mut rat, &mut sfx, 1.0);
        assert_eq!(rat.hp, after);
    }

    #[test]
    fn skater_cancels_a_splash_if_it_leaves_the_water() {
        let mut level = Level::empty(Theme::Outfall);
        level.puddles.push(puddle(START, 10.0));
        let mut e = enemy(EnemyKind::Skater, START);
        assert!(arm_attack(&mut e, &level, START + RIGHT * 20.0));
        e.pos += RIGHT * 16.0;
        let wet = level.wet_at(e.pos);
        assert!(!wet);
        advance_attack(&mut e, &level, wet, SKATER_TELL_SECS);
        assert!(outfall_enemy_tell(&e).is_none());
        assert!(!e.position_locked);
    }

    #[test]
    fn lampfish_lunges_on_its_locked_sweep_and_can_only_hit_once() {
        let level = Level::empty(Theme::Outfall);
        let mut e = enemy(EnemyKind::Lampfish, START);
        let mut rat = Player::new(START + RIGHT * LAMPFISH_TRIGGER_REACH, WeaponKind::Gnaw);
        let mut sfx = SfxQueue::default();
        let hp = rat.hp;
        assert_eq!(lampfish_bulb(&e), Some(START + RIGHT * LAMPFISH_BULB_OFFSET));
        assert!(arm_attack(&mut e, &level, rat.pos));
        assert!(outfall_enemy_tell(&e).unwrap().shape.contains(rat.pos, PLAYER_RADIUS));

        advance_attack(&mut e, &level, false, LAMPFISH_TELL_SECS);
        resolve_attack(&mut e, &level, &mut rat, &mut sfx, 1.0);
        assert_eq!(rat.hp, hp, "the head has yet to reach the rat");

        let half = LAMPFISH_LUNGE_SECS * 0.5;
        advance_attack(&mut e, &level, false, half);
        resolve_attack(&mut e, &level, &mut rat, &mut sfx, 1.0);
        assert!(rat.hp < hp);
        assert!(e.pos.dist(START + RIGHT * (LAMPFISH_LUNGE_LEN * 0.5)) < 1e-4);
        let after = rat.hp;
        rat.iframes = 0.0;
        rat.pos = START + RIGHT * 17.0;
        advance_attack(&mut e, &level, false, half);
        resolve_attack(&mut e, &level, &mut rat, &mut sfx, 1.0);
        assert_eq!(rat.hp, after);
        assert!(e.pos.dist(START + RIGHT * LAMPFISH_LUNGE_LEN) < 1e-4);
        assert_eq!(e.outfall.rearm, LAMPFISH_REARM_SECS);
    }

    #[test]
    fn fry_school_bunches_in_live_light_and_unbunches_when_its_radius_fits() {
        let mut level = Level::empty(Theme::Outfall);
        level.tiles.set(8, 6, TileKind::Wall(WallKind::Solid));
        level.tiles.rebuild_masks();
        let mut e = enemy(EnemyKind::FrySchool, START + RIGHT * 16.0);
        e.vel = RIGHT * 10.0;
        let far = START - RIGHT * 60.0;
        let bulb = e.pos - RIGHT * 32.0;
        update_school(&mut e, &level, far, &[bulb]);
        assert_eq!(e.radius, SCHOOL_BUNCHED_RADIUS);
        assert_eq!(e.vel, Vec2::ZERO);
        assert!(e.position_locked);
        let hp = e.hp;
        e.hurt(2.0);
        assert_eq!(e.hurt_from(3.0, far), 6.0);
        assert_eq!(e.damage_mult_from(far), 2.0, "pool and runoff multiplier");
        assert_eq!(e.hp, hp - 10.0);

        update_school(&mut e, &level, far, &[]);
        assert_eq!(e.radius, SCHOOL_BUNCHED_RADIUS, "the wall blocks expansion");
        assert!(!e.position_locked, "in darkness it can leave the wall");
        e.pos = START;
        update_school(&mut e, &level, far, &[]);
        assert_eq!(e.radius, EnemyKind::FrySchool.radius());
        assert_eq!(e.damage_mult_from(far), 1.0);
        assert!(!e.position_locked);
    }

    #[test]
    fn grouper_snuff_breaks_pair_ties_by_spawn_then_fixture_and_yields_to_boss() {
        let mut level = Level::empty(Theme::Outfall);
        let mut left = enemy(EnemyKind::Grouper, START);
        let mut right = enemy(EnemyKind::Grouper, START + RIGHT * 230.0);
        left.elite = true;
        right.elite = true;
        level.objects.lights = [
            left.pos + RIGHT * 16.0, left.pos - RIGHT * 16.0, right.pos + RIGHT * 16.0,
        ].into_iter().map(|pos| LightFixture {
            pos, radius: 26.0, kind: LightFixtureKind::Sconce, seed: 1,
        }).collect();

        let early = SpawnId(2);
        let late = SpawnId(3);
        refresh_snuff(&mut level, [(&late, &left), (&early, &right)].into_iter(), true);
        assert_eq!(level.objects.outfall_elite_snuff, None, "defer a fresh claim during the boss beat");
        refresh_snuff(&mut level, [(&late, &left), (&early, &right)].into_iter(), false);
        assert_eq!(level.objects.outfall_elite_snuff, Some(2));

        level.objects.outfall_boss_snuff = Some(2);
        refresh_snuff(&mut level, [(&late, &left), (&early, &right)].into_iter(), false);
        assert_eq!(level.objects.outfall_elite_snuff, Some(0));
        left.hp = 0.0;
        refresh_snuff(&mut level, [(&late, &left), (&early, &right)].into_iter(), false);
        assert!(!level.sconce_snuffed(0));
        assert!(level.sconce_snuffed(2));
    }
}
