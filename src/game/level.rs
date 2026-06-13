//! Map / theme definitions: arena size, palette, and obstacle layout.

use crate::math::Vec2;
use crate::render::palette::Rgb;

use super::nav::NavGrid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Sewer,
    Kitchen,
    Lab,
}

impl Theme {
    pub fn all() -> [Theme; 3] {
        [Theme::Sewer, Theme::Kitchen, Theme::Lab]
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Sewer => "Sewer",
            Theme::Kitchen => "Kitchen",
            Theme::Lab => "Lab",
        }
    }
}

/// Axis-aligned rectangle in world-pixel space.
#[derive(Clone, Copy, Debug)]
pub struct Rectf {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rectf {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
}

pub struct Palette {
    pub bg: Rgb,
    pub bg_alt: Rgb,
    pub wall: Rgb,
    pub wall_edge: Rgb,
    pub accent: Rgb,
    pub blood: Rgb,
}

pub struct Level {
    pub theme: Theme,
    pub arena: Vec2,
    pub walls: Vec<Rectf>,
    pub palette: Palette,
    pub nav: NavGrid,
}

impl Level {
    pub fn new(theme: Theme) -> Level {
        // ~8x the area of the original 360x280 arena.
        let arena = Vec2::new(1020.0, 800.0);
        let center = arena * 0.5;
        // Keep a clear circle around the player spawn (arena center).
        let clear = |r: &Rectf| r.center().dist(center) > 90.0;

        let (palette, mut walls) = match theme {
            Theme::Sewer => {
                let mut w = Vec::new();
                // A grid of pipe pillars with horizontal connectors.
                for &x in &[150.0, 360.0, 570.0, 780.0] {
                    for &y in &[120.0, 330.0, 540.0] {
                        w.push(Rectf::new(x, y, 20.0, 130.0));
                    }
                }
                for &y in &[230.0, 470.0, 690.0] {
                    w.push(Rectf::new(260.0, y, 220.0, 18.0));
                    w.push(Rectf::new(560.0, y, 220.0, 18.0));
                }
                (
                    Palette {
                        bg: (14, 22, 20),
                        bg_alt: (18, 28, 26),
                        wall: (40, 70, 64),
                        wall_edge: (70, 120, 110),
                        accent: (90, 220, 200),
                        blood: (120, 200, 90),
                    },
                    w,
                )
            }
            Theme::Kitchen => {
                let mut w = Vec::new();
                // Counter blocks around the edges + scattered islands.
                for &x in &[70.0, 860.0] {
                    for &y in &[80.0, 320.0, 560.0] {
                        w.push(Rectf::new(x, y, 90.0, 120.0));
                    }
                }
                for &x in &[260.0, 520.0, 760.0] {
                    w.push(Rectf::new(x, 60.0, 120.0, 30.0));
                    w.push(Rectf::new(x, 710.0, 120.0, 30.0));
                }
                for &(x, y) in &[(300.0, 300.0), (640.0, 300.0), (300.0, 520.0), (640.0, 520.0)] {
                    w.push(Rectf::new(x, y, 40.0, 40.0));
                }
                (
                    Palette {
                        bg: (26, 20, 16),
                        bg_alt: (34, 26, 20),
                        wall: (110, 80, 50),
                        wall_edge: (170, 130, 80),
                        accent: (255, 180, 90),
                        blood: (200, 60, 50),
                    },
                    w,
                )
            }
            Theme::Lab => {
                let mut w = Vec::new();
                // Long barrier walls forming partial chambers.
                for &y in &[150.0, 650.0] {
                    w.push(Rectf::new(200.0, y, 280.0, 16.0));
                    w.push(Rectf::new(560.0, y, 280.0, 16.0));
                }
                for &x in &[200.0, 820.0] {
                    w.push(Rectf::new(x, 250.0, 16.0, 300.0));
                }
                for &(x, y) in &[(360.0, 360.0), (660.0, 360.0), (510.0, 540.0)] {
                    w.push(Rectf::new(x, y, 16.0, 100.0));
                }
                (
                    Palette {
                        bg: (12, 14, 24),
                        bg_alt: (16, 20, 34),
                        wall: (40, 50, 90),
                        wall_edge: (90, 120, 220),
                        accent: (120, 220, 255),
                        blood: (90, 200, 255),
                    },
                    w,
                )
            }
        };
        walls.retain(clear);
        let nav = NavGrid::build(arena, &walls, 16.0);
        Level {
            theme,
            arena,
            walls,
            palette,
            nav,
        }
    }
}
