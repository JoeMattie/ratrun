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

/// A solid obstacle drawn as a themed object. Its `rect` is the collision +
/// nav footprint; the kind selects how it's rendered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    PipeV,
    PipeH,
    Valve,
    Grate,
    Table,
    Crate,
    Barrel,
    Counter,
    Console,
    Tank,
    Barrier,
}

impl PropKind {
    pub fn size(self) -> (f32, f32) {
        match self {
            PropKind::PipeV => (14.0, 72.0),
            PropKind::PipeH => (72.0, 14.0),
            PropKind::Valve => (20.0, 20.0),
            PropKind::Grate => (30.0, 30.0),
            PropKind::Table => (40.0, 26.0),
            PropKind::Crate => (22.0, 22.0),
            PropKind::Barrel => (20.0, 20.0),
            PropKind::Counter => (66.0, 22.0),
            PropKind::Console => (46.0, 18.0),
            PropKind::Tank => (24.0, 24.0),
            PropKind::Barrier => (16.0, 66.0),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Prop {
    pub kind: PropKind,
    pub rect: Rectf,
}

fn theme_props(theme: Theme) -> &'static [PropKind] {
    match theme {
        Theme::Sewer => &[
            PropKind::PipeV,
            PropKind::PipeH,
            PropKind::Valve,
            PropKind::Grate,
            PropKind::Barrier,
            PropKind::PipeH,
            PropKind::PipeV,
            PropKind::Valve,
        ],
        Theme::Kitchen => &[
            PropKind::Table,
            PropKind::Crate,
            PropKind::Barrel,
            PropKind::Counter,
            PropKind::Crate,
            PropKind::Table,
            PropKind::Barrel,
            PropKind::Counter,
        ],
        Theme::Lab => &[
            PropKind::Console,
            PropKind::Tank,
            PropKind::Crate,
            PropKind::Barrier,
            PropKind::Console,
            PropKind::Tank,
            PropKind::Barrier,
            PropKind::Crate,
        ],
    }
}

pub struct Level {
    pub theme: Theme,
    pub arena: Vec2,
    /// Collision + nav footprints (one per prop).
    pub walls: Vec<Rectf>,
    /// Rendered obstacle objects.
    pub props: Vec<Prop>,
    pub palette: Palette,
    pub nav: NavGrid,
}

impl Level {
    pub fn new(theme: Theme) -> Level {
        // ~8x the area of the original 360x280 arena.
        let arena = Vec2::new(1020.0, 800.0);
        let center = arena * 0.5;

        let palette = match theme {
            Theme::Sewer => Palette {
                bg: (14, 22, 20),
                bg_alt: (18, 28, 26),
                wall: (40, 70, 64),
                wall_edge: (70, 120, 110),
                accent: (90, 220, 200),
                blood: (120, 200, 90),
            },
            Theme::Kitchen => Palette {
                bg: (26, 20, 16),
                bg_alt: (34, 26, 20),
                wall: (110, 80, 50),
                wall_edge: (170, 130, 80),
                accent: (255, 180, 90),
                blood: (200, 60, 50),
            },
            Theme::Lab => Palette {
                bg: (12, 14, 24),
                bg_alt: (16, 20, 34),
                wall: (40, 50, 90),
                wall_edge: (90, 120, 220),
                accent: (120, 220, 255),
                blood: (90, 200, 255),
            },
        };

        // Scatter many small props on a staggered grid, skipping the spawn
        // circle and the arena margins.
        let kinds = theme_props(theme);
        let slot = 132.0;
        let cols = ((arena.x - 120.0) / slot) as i32;
        let rows = ((arena.y - 120.0) / slot) as i32;
        let mut props = Vec::new();
        let mut idx = 0usize;
        for j in 0..=rows {
            for i in 0..=cols {
                let kind = kinds[idx % kinds.len()];
                idx += 1;
                let stagger = if j % 2 == 0 { 0.0 } else { slot * 0.5 };
                let cxp = 90.0 + i as f32 * slot + stagger;
                let cyp = 90.0 + j as f32 * slot;
                let (w, h) = kind.size();
                let rect = Rectf::new(cxp - w * 0.5, cyp - h * 0.5, w, h);
                if rect.center().dist(center) < 120.0 {
                    continue; // keep the spawn clear
                }
                if rect.x < 26.0
                    || rect.right() > arena.x - 26.0
                    || rect.y < 26.0
                    || rect.bottom() > arena.y - 26.0
                {
                    continue; // keep off the arena edges
                }
                props.push(Prop { kind, rect });
            }
        }

        let walls: Vec<Rectf> = props.iter().map(|p| p.rect).collect();
        let nav = NavGrid::build(arena, &walls, 16.0);
        Level {
            theme,
            arena,
            walls,
            props,
            palette,
            nav,
        }
    }
}
