//! Seeded, non-colliding room lights. This stage only reads the finished map.
use rand::RngExt;

use crate::game::collision::{TileKind, TileMap};
use crate::game::level::{
    level_rng, AmbientEmitter, EmitterKind, Level, LightFixture, LightFixtureKind, LightZone,
    LightZoneKind, RoomKind, Theme,
};
use crate::math::Vec2;

pub(super) fn place(level: &mut Level, attempt: u32) {
    let mut rng = level_rng(level.theme, level.seed ^ 0x4c49_4748_5453, attempt);
    if level.theme == Theme::Trailer {
        level.objects.lights.push(LightFixture {
            pos: level.start,
            kind: LightFixtureKind::Sun,
            radius: 0.0,
            seed: rng.random(),
        });
        // The warren below the planks is still dark. The sun pass skips it.
        level
            .objects
            .light_zones
            .extend(level.mazes.iter().map(|maze| LightZone {
                rect: maze.bounds,
                kind: LightZoneKind::Maze,
            }));
        place_emitters(level, &mut rng);
        return;
    }

    for room in &level.rooms {
        let seed = rng.random::<u32>();
        let zone = match room.kind {
            RoomKind::Start => LightZoneKind::Start,
            RoomKind::Boss => LightZoneKind::Arena,
            RoomKind::Corridor => LightZoneKind::Corridor,
            RoomKind::Maze => LightZoneKind::Maze,
            RoomKind::Room if seed.is_multiple_of(5) => LightZoneKind::Blackout,
            _ => LightZoneKind::Room,
        };
        level.objects.light_zones.push(LightZone {
            rect: room.rect,
            kind: zone,
        });

        let span = room.rect.w.max(room.rect.h) as f32 * 16.0;
        let short = room.rect.w.min(room.rect.h) as f32 * 16.0;
        let horizontal = room.rect.w >= room.rect.h;
        let (kind, count, radius) = match zone {
            LightZoneKind::Start => (
                LightFixtureKind::Pendant,
                2,
                (span * 0.65).clamp(88.0, 160.0),
            ),
            LightZoneKind::Arena => (
                LightFixtureKind::Arena,
                4,
                (span * 0.64).clamp(148.0, 272.0),
            ),
            LightZoneKind::Room => (
                LightFixtureKind::Pendant,
                if span > 208.0 { 2 } else { 1 },
                (span * 0.70).clamp(88.0, 172.0),
            ),
            LightZoneKind::Corridor => (
                LightFixtureKind::Passage,
                (span / 132.0).ceil().clamp(1.0, 6.0) as usize,
                58.0,
            ),
            LightZoneKind::Blackout => (LightFixtureKind::Broken, 1, 27.0),
            LightZoneKind::Maze => continue,
        };
        for i in 0..count {
            let along = (i as f32 + 0.5) / count as f32 - 0.5;
            let across = if kind == LightFixtureKind::Arena {
                if i.is_multiple_of(2) {
                    -short * 0.33
                } else {
                    short * 0.33
                }
            } else {
                0.0
            };
            let shift = along * span * 0.76;
            let intended = room.center()
                + if horizontal {
                    Vec2::new(shift, across)
                } else {
                    Vec2::new(across, shift)
                };
            let (x, y) = crate::game::collision::cell_of(intended);
            let usable = |cx, cy| {
                room.rect.contains(cx, cy)
                    && matches!(level.tiles.get(cx, cy), TileKind::Open)
                    && !level.mazes.iter().any(|maze| maze.bounds.contains(cx, cy))
            };
            let cell = if usable(x, y) {
                Some((x, y))
            } else {
                room.rect
                    .cells()
                    .filter(|&(cx, cy)| usable(cx, cy))
                    .min_by_key(|&(cx, cy)| (cx - x).pow(2) + (cy - y).pow(2))
            };
            if let Some((cx, cy)) = cell {
                let pos = TileMap::tile_rect(cx, cy).center();
                level.objects.lights.push(LightFixture {
                    pos,
                    kind,
                    radius,
                    seed: seed.wrapping_add(i as u32 * 127),
                });
            }
        }
    }
    place_emitters(level, &mut rng);
}

fn place_emitters(level: &mut Level, rng: &mut rand::rngs::StdRng) {
    use EmitterKind::*;
    // Sparse, ordered placements. A room need not have an emitter at all.
    // The first entry prefers a blackout for the theme's signature vignette.
    let plan = match level.theme {
        Theme::Sewer => [Fire, Drip, Steam, Fumes],
        Theme::Kitchen => [Fire, Drip, Pilot, Exhaust],
        Theme::Lab => [Cable, Steam, Fumes, Dust],
        Theme::Laundry => [Fog, Steam, Drip, Insects],
        Theme::Printworks => [Cable, Dust, Exhaust, Pilot],
        Theme::Trailer => [Exhaust, Fire, Dust, Cable],
    };
    let (cx, cy) = crate::game::collision::cell_of(level.start);
    for (slot, kind) in plan.into_iter().enumerate() {
        let seed = rng.random::<u32>();
        let at = level
            .rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| {
                matches!(
                    room.kind,
                    RoomKind::Room | RoomKind::Hall | RoomKind::Corridor
                )
            })
            .filter_map(|(index, room)| {
                let world = room.rect.to_world();
                let edge = Vec2::new(
                    world.x + world.w * if seed & 1 == 0 { 0.30 } else { 0.73 },
                    world.y + world.h * if seed & 2 == 0 { 0.25 } else { 0.76 },
                );
                let (x, y) = crate::game::collision::cell_of(edge);
                let valid = |x, y| {
                    let pos = TileMap::tile_rect(x, y).center();
                    matches!(level.tiles.get(x, y), TileKind::Open)
                        && (x - cx).abs().max((y - cy).abs()) >= 13
                        && !level.rooms.iter().any(|r| {
                            matches!(r.kind, RoomKind::Boss | RoomKind::Start)
                                && r.rect.contains(x, y)
                        })
                        && !level.mazes.iter().any(|m| m.bounds.contains(x, y))
                        && level.pit_at(pos).is_none()
                        && !level
                            .objects
                            .shop_holes
                            .iter()
                            .any(|shop| shop.mouth == (x, y))
                        && level
                            .objects
                            .emitters
                            .iter()
                            .all(|e| e.pos.dist_sq(pos) > 100.0 * 100.0)
                };
                // Try a bounded neighbourhood rather than walking each room.
                let (x, y) = [
                    (0, 0),
                    (1, 0),
                    (0, 1),
                    (-1, 0),
                    (0, -1),
                    (2, 0),
                    (0, 2),
                    (-2, 0),
                    (0, -2),
                ]
                .into_iter()
                .map(|(dx, dy)| (x + dx, y + dy))
                .find(|&(x, y)| room.rect.contains(x, y) && valid(x, y))?;
                let dark = level
                    .objects
                    .light_zones
                    .iter()
                    .any(|z| z.kind == LightZoneKind::Blackout && z.rect == room.rect);
                let priority = match (slot, dark, room.kind) {
                    (0, true, _) => 0,
                    (1, _, RoomKind::Corridor) => 0,
                    (_, _, RoomKind::Hall | RoomKind::Room) => 1,
                    _ => 2,
                };
                let order = (index as u32)
                    .wrapping_mul(0x9e37_79b1)
                    .rotate_left(seed & 31)
                    ^ seed;
                Some(((priority, order), TileMap::tile_rect(x, y).center()))
            })
            .min_by_key(|a| a.0)
            .map(|(_, pos)| pos);
        if let Some(pos) = at {
            let intensity = if level.theme == Theme::Trailer && kind == Fire {
                0.39
            } else {
                1.0
            };
            level.objects.emitters.push(AmbientEmitter {
                kind,
                pos,
                seed,
                rate: 0.9 + (seed % 31) as f32 / 100.0,
                intensity,
            });
        }
    }
    for maze in &level.mazes {
        level.objects.emitters.push(AmbientEmitter {
            kind: Pilot,
            pos: maze.reward + Vec2::new(2.0, 0.0),
            seed: rng.random(),
            rate: 0.70,
            intensity: 0.51,
        });
    }
}
