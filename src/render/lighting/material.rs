//! The per-pixel material byte (batch 13): what a world pixel is made of, so
//! the lighting effects can give slime its sheen, steel its glint and fur none.
//!
//! One byte per world pixel beside [`crate::render::framebuffer::PixelBuffer`]'s
//! colours, stamped by every buffer writer with the buffer's current *pen*
//! (`PixelBuffer::set_pen`), so a scoped pen around a sprite's draw call tags
//! exactly the pixels it wrote, and anything drawn over them later re-tags
//! them. Layout:
//!
//! ```text
//! bits 0..=3  MaterialKind (0 = Default: resolved from the theme and surface)
//! bit  4      CREATURE (rat, enemies, minibosses, bosses)
//! bits 5..=7  height class 0..=7 (body height / 4 px; 0 = unknown or flat)
//! ```
//!
//! Foundation-owned: the stage files read this table and do not edit it.

#![allow(dead_code)] // Read by the stage files as they are filled in.

use super::fx::Normal;
use crate::game::level::Theme;

pub(super) const KIND_MASK: u8 = 0x0f;
pub(super) const CREATURE: u8 = 0x10;
pub(super) const HEIGHT_SHIFT: u8 = 5;

/// What a pixel is made of. `repr(u8)` values are the low nibble of the byte.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MaterialKind {
    /// Untagged: [`resolve`] picks the theme's floor or wall material.
    Default = 0,
    /// Matte concrete, plaster, cardboard.
    Stone = 1,
    /// Damp brick and flagstone (the Sewer).
    WetBrick = 2,
    /// Glazed tile (Kitchen, Laundry).
    Tile = 3,
    /// Planks, pallets, the Flatbed's bed.
    Wood = 4,
    /// Brushed steel, pipes, consoles' housings.
    Metal = 5,
    /// Lab glass, tanks, lenses.
    Glass = 6,
    /// Grease film, oily puddles.
    Grease = 7,
    /// Wet printing ink.
    Ink = 8,
    /// Standing water, puddles.
    Water = 9,
    /// Fur, feathers.
    Fur = 10,
    /// Slime, wet skin (leeches, toads).
    Slime = 11,
    /// Chitin, carapace, shell.
    Shell = 12,
    /// Cloth, paper.
    Cloth = 13,
    /// Self-lit: every stage leaves it alone.
    Emissive = 14,
    /// Unused.
    Reserved = 15,
}

impl MaterialKind {
    pub(super) const fn from_bits(b: u8) -> Self {
        match b & KIND_MASK {
            0 => Self::Default,
            1 => Self::Stone,
            2 => Self::WetBrick,
            3 => Self::Tile,
            4 => Self::Wood,
            5 => Self::Metal,
            6 => Self::Glass,
            7 => Self::Grease,
            8 => Self::Ink,
            9 => Self::Water,
            10 => Self::Fur,
            11 => Self::Slime,
            12 => Self::Shell,
            13 => Self::Cloth,
            14 => Self::Emissive,
            _ => Self::Reserved,
        }
    }
}

/// The byte for a pen: `kind`, the creature bit and a height class (0..=7).
#[inline]
pub(super) const fn pen(kind: MaterialKind, creature: bool, height: u8) -> u8 {
    let h = if height > 7 { 7 } else { height };
    (kind as u8) | if creature { CREATURE } else { 0 } | (h << HEIGHT_SHIFT)
}

#[inline]
pub(super) const fn kind(b: u8) -> MaterialKind {
    MaterialKind::from_bits(b)
}

#[inline]
pub(super) const fn is_creature(b: u8) -> bool {
    b & CREATURE != 0
}

#[inline]
pub(super) const fn height(b: u8) -> u8 {
    b >> HEIGHT_SHIFT
}

/// The material a pixel is lit as: its own tag, or for an untagged pixel the
/// theme's floor (`wall == false`) or wall material.
pub(super) fn resolve(b: u8, theme: Theme, wall: bool) -> MaterialKind {
    let own = kind(b);
    if own != MaterialKind::Default {
        return own;
    }
    match (theme, wall) {
        (Theme::Sewer, _) => MaterialKind::WetBrick,
        (Theme::Kitchen, false) | (Theme::Laundry, false) => MaterialKind::Tile,
        (Theme::Kitchen, true) | (Theme::Laundry, true) => MaterialKind::Stone,
        (Theme::Lab, _) => MaterialKind::Metal,
        (Theme::Printworks, _) => MaterialKind::Stone,
        (Theme::Trailer, false) => MaterialKind::Wood,
        (Theme::Trailer, true) => MaterialKind::Metal,
        // Wet seabed sand catches the sewer streak; pier pilings are weathered timber.
        (Theme::Outfall, false) => MaterialKind::WetBrick,
        (Theme::Outfall, true) => MaterialKind::Wood,
    }
}

/// How a material takes light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MaterialParams {
    /// Specular strength, /255.
    pub gloss: u8,
    /// Blinn-Phong exponent (the LUT index of `specular.rs`).
    pub shininess: u8,
    /// Environment reflection (wet-floor streaks), /255.
    pub env: u8,
    /// Bump depth, /255.
    pub bump: u8,
    /// A broad wet sheen plus a sharp pinpoint glint (slime).
    pub double_lobe: bool,
}

const fn p(gloss: u8, shininess: u8, env: u8, bump: u8) -> MaterialParams {
    MaterialParams {
        gloss,
        shininess,
        env,
        bump,
        double_lobe: false,
    }
}

/// The material table. `Default` is never looked up (resolve it first).
pub(super) const fn params(kind: MaterialKind) -> MaterialParams {
    match kind {
        MaterialKind::Default | MaterialKind::Stone => p(10, 8, 0, 160),
        MaterialKind::WetBrick => p(70, 24, 90, 200),
        MaterialKind::Tile => p(60, 40, 90, 140),
        MaterialKind::Wood => p(20, 10, 0, 180),
        MaterialKind::Metal => p(140, 48, 110, 90),
        MaterialKind::Glass => p(180, 96, 140, 0),
        MaterialKind::Grease => p(110, 20, 60, 60),
        MaterialKind::Ink => p(120, 32, 150, 40),
        // Batch fourteen: 160/200 → 210/255, so a lamp's streak and glint on a
        // puddle are the brightest wet response on any floor.
        MaterialKind::Water => p(210, 64, 255, 30),
        MaterialKind::Fur => p(12, 6, 0, 0),
        MaterialKind::Slime => MaterialParams {
            gloss: 200,
            shininess: 80,
            env: 60,
            bump: 0,
            double_lobe: true,
        },
        MaterialKind::Shell => p(120, 40, 30, 0),
        MaterialKind::Cloth => p(4, 4, 0, 120),
        MaterialKind::Emissive | MaterialKind::Reserved => p(0, 1, 0, 0),
    }
}

/// The up component of a Q0.7 tangent normal, in the same units (0..=127).
#[inline]
pub(super) fn nz(n: Normal) -> i32 {
    let (x, y) = (i32::from(n[0]), i32::from(n[1]));
    (127 * 127 - x * x - y * y).max(0).isqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pen_round_trips_its_fields() {
        for k in 0..16u8 {
            let kind = MaterialKind::from_bits(k);
            assert_eq!(kind as u8, k);
            for creature in [false, true] {
                for h in 0..8 {
                    let b = pen(kind, creature, h);
                    assert_eq!(super::kind(b), kind);
                    assert_eq!(is_creature(b), creature);
                    assert_eq!(height(b), h);
                }
            }
        }
        assert_eq!(pen(MaterialKind::Default, false, 0), 0, "0 is the untagged floor");
    }

    #[test]
    fn a_flat_normal_points_straight_up() {
        assert_eq!(nz([0, 0]), 127);
        assert_eq!(nz([127, 0]), 0);
    }

    #[test]
    fn outfall_resolves_wet_sand_floor_and_timber_piling_walls() {
        assert_eq!(resolve(0, Theme::Outfall, false), MaterialKind::WetBrick);
        assert_eq!(resolve(0, Theme::Outfall, true), MaterialKind::Wood);
    }
}
