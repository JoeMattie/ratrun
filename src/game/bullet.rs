//! Projectiles — both player weapon fire and enemy bullet-hell shots.

use crate::math::Vec2;
use crate::render::palette::Rgb;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    Player,
    Enemy,
}

/// Orbiting projectiles (Spore Orbit weapon) track around the player.
#[derive(Clone, Copy)]
pub struct Orbit {
    pub angle: f32,
    pub radius: f32,
    pub speed: f32,
}

/// A lobbed Acid Spit shot: it flies to `target` (or stops on a wall / when
/// its life runs out) and only then turns into a damaging pool. The stats are
/// carried on the bullet so `update_bullets` can land it without knowing the
/// weapon that fired it.
#[derive(Clone, Copy)]
pub struct Lob {
    pub target: Vec2,
    pub radius: f32,
    pub dps: f32,
    pub life: f32,
    pub color: Rgb,
}

pub struct Bullet {
    pub pos: Vec2,
    /// Position at the start of this tick's movement, so collision passes can
    /// sweep the whole `prev_pos -> pos` segment instead of only its endpoint.
    pub prev_pos: Vec2,
    pub vel: Vec2,
    pub radius: f32,
    pub damage: f32,
    pub faction: Faction,
    pub life: f32,
    pub pierce: i32,
    pub color: Rgb,
    pub size: i32,
    pub orbit: Option<Orbit>,
    /// Set for Acid Spit's lobbed shot (no in-flight damage, lands as a pool).
    pub lob: Option<Lob>,
    /// Re-hit cooldown so orbit/piercing shots don't delete an enemy in one frame.
    pub hit_cd: f32,
}

impl Bullet {
    pub fn new(pos: Vec2, vel: Vec2, damage: f32, faction: Faction, color: Rgb) -> Self {
        Self {
            pos,
            prev_pos: pos,
            vel,
            radius: 2.0,
            damage,
            faction,
            life: 2.5,
            pierce: 0,
            color,
            size: 1,
            orbit: None,
            lob: None,
            hit_cd: 0.0,
        }
    }
}
