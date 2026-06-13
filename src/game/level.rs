//! Map / theme definitions: arena size, palette, and obstacle layout.

use crate::math::Vec2;
use crate::render::palette::Rgb;

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
}

impl Level {
    pub fn new(theme: Theme) -> Level {
        let arena = Vec2::new(360.0, 280.0);
        let (palette, walls) = match theme {
            Theme::Sewer => (
                Palette {
                    bg: (14, 22, 20),
                    bg_alt: (18, 28, 26),
                    wall: (40, 70, 64),
                    wall_edge: (70, 120, 110),
                    accent: (90, 220, 200),
                    blood: (120, 200, 90),
                },
                vec![
                    Rectf::new(70.0, 60.0, 14.0, 80.0),
                    Rectf::new(276.0, 60.0, 14.0, 80.0),
                    Rectf::new(70.0, 150.0, 14.0, 80.0),
                    Rectf::new(276.0, 150.0, 14.0, 80.0),
                    Rectf::new(150.0, 120.0, 60.0, 14.0),
                    Rectf::new(150.0, 170.0, 60.0, 14.0),
                ],
            ),
            Theme::Kitchen => (
                Palette {
                    bg: (26, 20, 16),
                    bg_alt: (34, 26, 20),
                    wall: (110, 80, 50),
                    wall_edge: (170, 130, 80),
                    accent: (255, 180, 90),
                    blood: (200, 60, 50),
                },
                vec![
                    Rectf::new(40.0, 40.0, 90.0, 22.0),
                    Rectf::new(230.0, 40.0, 90.0, 22.0),
                    Rectf::new(40.0, 218.0, 90.0, 22.0),
                    Rectf::new(230.0, 218.0, 90.0, 22.0),
                    Rectf::new(168.0, 128.0, 24.0, 24.0),
                ],
            ),
            Theme::Lab => (
                Palette {
                    bg: (12, 14, 24),
                    bg_alt: (16, 20, 34),
                    wall: (40, 50, 90),
                    wall_edge: (90, 120, 220),
                    accent: (120, 220, 255),
                    blood: (90, 200, 255),
                },
                vec![
                    Rectf::new(110.0, 60.0, 140.0, 12.0),
                    Rectf::new(110.0, 208.0, 140.0, 12.0),
                    Rectf::new(110.0, 60.0, 12.0, 60.0),
                    Rectf::new(238.0, 160.0, 12.0, 60.0),
                ],
            ),
        };
        Level {
            theme,
            arena,
            walls,
            palette,
        }
    }
}
