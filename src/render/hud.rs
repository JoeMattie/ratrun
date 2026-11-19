//! Heads-up display: top tactical rail, bottom vitals/XP/dash rail, and
//! zero-allocation retained HUD state + telemetry.

use std::fmt::Write as _;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::audio::Sfx;
use crate::game::director;
use crate::game::gem::PickupKind;
use crate::game::level::Theme;
use crate::game::weapon::WeaponKind;
use crate::game::world::World;
use crate::math::Vec2;
use crate::render::palette::{self, Rgb, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};

/// Maximum simultaneous transient callouts held on the HUD.
pub const MAX_CALLOUTS: usize = 3;

/// Danger / critical alert red (cohesive with rat tail pink, higher urgency).
pub const DANGER: Color = Color::Rgb(248, 92, 102);
pub const DANGER_DIM: Color = Color::Rgb(168, 52, 62);
pub const WARN_AMBER: Color = Color::Rgb(242, 184, 102);
pub const BOSS_ROSE: Color = Color::Rgb(236, 76, 128);
pub const BOSS_HOT: Color = Color::Rgb(255, 118, 88);
pub const HEAL_MINT: Color = Color::Rgb(126, 224, 152);
pub const MAGNET_ICE: Color = Color::Rgb(128, 206, 250);
pub const TRAIL_GOLD: Color = Color::Rgb(248, 214, 152);

/// Distinct single-cell geometric glyph for each weapon kind.
pub fn weapon_glyph(kind: WeaponKind) -> char {
    match kind {
        WeaponKind::Gnaw => '▸',
        WeaponKind::CheeseSpray => '»',
        WeaponKind::SqueakNova => '✦',
        WeaponKind::SporeOrbit => '◎',
        WeaponKind::TailWhip => '◈',
        WeaponKind::AcidSpit => '♦',
    }
}

/// Sector-specific codename for the end-of-run apex boss.
pub fn boss_codename(theme: Theme) -> (&'static str, &'static str) {
    match theme {
        Theme::Sewer => ("SEWER BROODMOTHER", "CLASS-V BIOWEAPON"),
        Theme::Kitchen => ("THE HEAD BUTCHER", "CLASS-V EXTERMINATOR"),
        Theme::Lab => ("PRIME CHIMERA", "CLASS-V CONTAINMENT UNIT"),
    }
}

/// Difficulty escalation tier (1..=5) derived purely from the director clock.
pub fn threat_tier(elapsed: f32, boss_alive: bool) -> u8 {
    if boss_alive || elapsed >= director::BOSS_TIME {
        5
    } else if elapsed >= 120.0 {
        4
    } else if elapsed >= director::ELITE_START {
        3
    } else if elapsed >= 20.0 {
        2
    } else {
        1
    }
}

/// Category of a transient in-game HUD callout pill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CalloutKind {
    Upgrade,
    Heal,
    Magnet,
    Nuke,
    Escalation,
    Milestone,
    BossSlain,
}

impl CalloutKind {
    pub fn color(self, sector_accent: Color) -> Color {
        match self {
            CalloutKind::Upgrade => TAIL,
            CalloutKind::Heal => HEAL_MINT,
            CalloutKind::Magnet => MAGNET_ICE,
            CalloutKind::Nuke => BOSS_HOT,
            CalloutKind::Escalation => WARN_AMBER,
            CalloutKind::Milestone => sector_accent,
            CalloutKind::BossSlain => HEAL_MINT,
        }
    }
}

/// One transient callout slot with retained buffers (no per-frame allocation).
#[derive(Default)]
pub struct CalloutSlot {
    pub active: bool,
    pub kind: Option<CalloutKind>,
    pub tag: String,
    pub message: String,
    pub timer: f32,
    pub duration: f32,
}

/// Snapshot of world entity telemetry fed into the HUD without reading
/// `World`'s internal entity lists directly from render code.
#[derive(Default, Clone, Copy)]
pub struct HudTelemetry {
    /// Active boss `(hp, max_hp)` if a boss is currently alive.
    pub boss_hp: Option<(f32, f32)>,
    /// World-pixel vector from player to active boss (`boss.pos - player.pos`).
    pub boss_offset: Option<Vec2>,
    /// Total living enemy count (for threat cues).
    pub enemy_count: usize,
    /// Whether a boss is known to be alive even before `boss_hp` is populated.
    pub boss_alive: bool,
    /// Whether audio is currently muted.
    pub muted: bool,
}

impl HudTelemetry {
    pub fn set_boss_hp(&mut self, hp: Option<(f32, f32)>) {
        self.boss_hp = hp;
        if hp.is_some() {
            self.boss_alive = true;
        }
    }

    pub fn set_boss_offset(&mut self, offset: Option<Vec2>) {
        self.boss_offset = offset;
    }

    pub fn set_enemy_count(&mut self, count: usize) {
        self.enemy_count = count;
    }

    pub fn set_boss_alive(&mut self, alive: bool) {
        self.boss_alive = alive;
        if !alive {
            self.boss_hp = None;
            self.boss_offset = None;
        }
    }
}

/// Reused text buffers and smooth visual animation state for the HUD.
///
/// `App` keeps one of these and hands it to [`draw_top`] / [`draw_bottom`] /
/// `overlay::draw_overlays` every frame: each readout is written into a
/// retained buffer that keeps its capacity instead of allocating per frame.
pub struct HudState {
    // Top bar retained buffers.
    time: String,
    level: String,
    kills: String,
    streak_buf: String,
    score: String,
    /// `(tag, level)` text per weapon slot, reused across frames.
    weapon_tags: Vec<(String, String)>,
    right_badge: String,

    // Bottom bar retained buffers.
    hp: String,
    xp_label: String,
    dash: String,

    // Overlay retained buffers (accessed by `overlay.rs`).
    pub(crate) boss_hp_text: String,
    pub(crate) boss_banner: String,

    // Smooth visual state & feedback timers.
    pub clock: f32,
    pub hp_trail: f32,
    hp_trail_hold: f32,
    pub boss_hp_trail: f32,
    boss_trail_hold: f32,
    pub hurt_edge: f32,
    pub level_flash: f32,
    pub dash_ready_flash: f32,
    pub boss_intro_timer: f32,

    // Frame-to-frame delta tracking (initialized per run via `reset`).
    initialized: bool,
    prev_hp_ratio: f32,
    prev_hp: f32,
    prev_dash_cd: f32,
    prev_level: u32,
    prev_kills: u32,
    prev_boss_alive: bool,
    prev_boss_ratio: f32,
    escalation_stage: u8,
    next_kill_milestone: u32,
    /// Rolling recent kill burst (for combo streak readout).
    pub streak_kills: u32,
    streak_timer: f32,

    /// Fixed-size callout slots (zero allocation on steady-state pushes).
    pub callouts: [CalloutSlot; MAX_CALLOUTS],
    /// Telemetry snapshot updated by `App` each tick.
    pub telemetry: HudTelemetry,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            time: String::with_capacity(8),
            level: String::with_capacity(6),
            kills: String::with_capacity(8),
            streak_buf: String::with_capacity(8),
            score: String::with_capacity(12),
            weapon_tags: Vec::with_capacity(6),
            right_badge: String::with_capacity(24),
            hp: String::with_capacity(16),
            xp_label: String::with_capacity(12),
            dash: String::with_capacity(16),
            boss_hp_text: String::with_capacity(24),
            boss_banner: String::with_capacity(48),
            clock: 0.0,
            hp_trail: 1.0,
            hp_trail_hold: 0.0,
            boss_hp_trail: 1.0,
            boss_trail_hold: 0.0,
            hurt_edge: 0.0,
            level_flash: 0.0,
            dash_ready_flash: 0.0,
            boss_intro_timer: 0.0,
            initialized: false,
            prev_hp_ratio: 1.0,
            prev_hp: 100.0,
            prev_dash_cd: 0.0,
            prev_level: 1,
            prev_kills: 0,
            prev_boss_alive: false,
            prev_boss_ratio: 1.0,
            escalation_stage: 0,
            next_kill_milestone: 50,
            streak_kills: 0,
            streak_timer: 0.0,
            callouts: Default::default(),
            telemetry: HudTelemetry::default(),
        }
    }
}

impl HudState {
    /// Reset transient per-run HUD timers, trails, and callouts when starting a new run.
    pub fn reset(&mut self) {
        self.clock = 0.0;
        self.hp_trail = 1.0;
        self.hp_trail_hold = 0.0;
        self.boss_hp_trail = 1.0;
        self.boss_trail_hold = 0.0;
        self.hurt_edge = 0.0;
        self.level_flash = 0.0;
        self.dash_ready_flash = 0.0;
        self.boss_intro_timer = 0.0;
        self.initialized = false;
        self.prev_hp_ratio = 1.0;
        self.prev_hp = 100.0;
        self.prev_dash_cd = 0.0;
        self.prev_level = 1;
        self.prev_kills = 0;
        self.prev_boss_alive = false;
        self.prev_boss_ratio = 1.0;
        self.escalation_stage = 0;
        self.next_kill_milestone = 50;
        self.streak_kills = 0;
        self.streak_timer = 0.0;
        for slot in &mut self.callouts {
            slot.active = false;
            slot.timer = 0.0;
        }
        let muted = self.telemetry.muted;
        self.telemetry = HudTelemetry {
            muted,
            ..HudTelemetry::default()
        };
    }

    /// Push a transient callout pill without allocating once slot buffers are warm.
    pub fn push_callout(&mut self, kind: CalloutKind, tag: &str, message: &str, duration: f32) {
        // Prefer an inactive slot; otherwise replace the slot closest to expiring.
        let idx = self
            .callouts
            .iter()
            .position(|c| !c.active)
            .unwrap_or_else(|| {
                self.callouts
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.timer.total_cmp(&b.timer))
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        let slot = &mut self.callouts[idx];
        slot.active = true;
        slot.kind = Some(kind);
        slot.tag.clear();
        slot.tag.push_str(tag);
        slot.message.clear();
        slot.message.push_str(message);
        slot.timer = duration;
        slot.duration = duration.max(0.1);
    }

    /// Notify the HUD that a specific pickup was collected.
    pub fn notify_pickup(&mut self, kind: PickupKind) {
        match kind {
            PickupKind::Heal => {
                self.push_callout(CalloutKind::Heal, "+MEDKIT", "HP RESTORED", 2.4);
            }
            PickupKind::Magnet => {
                self.push_callout(CalloutKind::Magnet, "+MAGNET", "XP VACUUM ACTIVE", 2.4);
            }
            PickupKind::Nuke => {
                self.push_callout(CalloutKind::Nuke, "+PURGE", "SECTOR CLEARED", 2.8);
            }
        }
    }

    /// Notify the HUD that the player just equipped an upgrade from the level-up screen.
    pub fn notify_upgrade(&mut self, title: &str) {
        self.level_flash = 0.9;
        self.push_callout(CalloutKind::Upgrade, "EQUIPPED", title, 2.5);
    }

    /// Advance smooth HUD animations, damage trails, and event callouts for one tick.
    pub fn tick(&mut self, dt: f32, world: &World, sfx_frame: &[Sfx], muted: bool) {
        let dt = dt.clamp(0.0, 0.1);
        self.clock += dt;
        self.telemetry.muted = muted;

        let hp_ratio = (world.player.hp.max(0.0) / world.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        if !self.initialized {
            self.initialized = true;
            self.hp_trail = hp_ratio;
            self.prev_hp_ratio = hp_ratio;
            self.prev_hp = world.player.hp;
            self.prev_dash_cd = world.player.dash_cd;
            self.prev_level = world.player.level;
            self.prev_kills = world.kills;
            self.prev_boss_alive = self.telemetry.boss_alive;
            self.escalation_stage = Self::stage_for_elapsed(world.elapsed);
            return;
        }

        // 1. Player HP damage trail & hurt-edge vignette.
        if hp_ratio < self.prev_hp_ratio - 0.001 || world.player.hurt_flash > 0.18 {
            self.hp_trail = self.hp_trail.max(self.prev_hp_ratio);
            self.hp_trail_hold = 0.28;
            self.hurt_edge = 0.35;
        } else if hp_ratio > self.hp_trail {
            self.hp_trail = hp_ratio;
            self.hp_trail_hold = 0.0;
        }
        if self.hp_trail_hold > 0.0 {
            self.hp_trail_hold = (self.hp_trail_hold - dt).max(0.0);
        } else if self.hp_trail > hp_ratio {
            self.hp_trail = (self.hp_trail - dt * 0.55).max(hp_ratio);
        }
        self.hurt_edge = (self.hurt_edge - dt).max(0.0);

        // 2. Boss HP damage trail, arrival intro banner, and defeat detection.
        if let Some((bhp, bmax)) = self.telemetry.boss_hp {
            let bratio = (bhp.max(0.0) / bmax.max(1.0)).clamp(0.0, 1.0);
            if !self.prev_boss_alive {
                self.boss_hp_trail = bratio;
                self.prev_boss_ratio = bratio;
            } else if bratio < self.prev_boss_ratio - 0.001 {
                self.boss_hp_trail = self.boss_hp_trail.max(self.prev_boss_ratio);
                self.boss_trail_hold = 0.22;
            }
            if self.boss_trail_hold > 0.0 {
                self.boss_trail_hold = (self.boss_trail_hold - dt).max(0.0);
            } else if self.boss_hp_trail > bratio {
                self.boss_hp_trail = (self.boss_hp_trail - dt * 0.45).max(bratio);
            }
            self.prev_boss_ratio = bratio;
        }
        if sfx_frame.contains(&Sfx::Boss) || (self.telemetry.boss_alive && !self.prev_boss_alive) {
            self.boss_intro_timer = 3.2;
        }
        if self.prev_boss_alive && !self.telemetry.boss_alive && world.director.boss_spawned {
            self.push_callout(
                CalloutKind::BossSlain,
                "APEX DOWN",
                "SPECIMEN NEUTRALIZED",
                3.4,
            );
        }
        self.boss_intro_timer = (self.boss_intro_timer - dt).max(0.0);

        // 3. Dash-ready glint when cooldown finishes.
        if self.prev_dash_cd > 0.0 && world.player.dash_cd <= 0.0 {
            self.dash_ready_flash = 0.45;
        }
        self.dash_ready_flash = (self.dash_ready_flash - dt).max(0.0);

        // 4. Level-up flash.
        if world.player.level > self.prev_level {
            self.level_flash = 0.85;
        }
        self.level_flash = (self.level_flash - dt).max(0.0);

        // 5. Exact pickup callouts from `World::last_pickups()` (with SFX fallback for tests).
        let pickups = world.last_pickups();
        if !pickups.is_empty() {
            for &pk in pickups {
                self.notify_pickup(pk);
            }
        } else if sfx_frame.contains(&Sfx::Pickup) {
            if sfx_frame.contains(&Sfx::Explosion) {
                self.notify_pickup(PickupKind::Nuke);
            } else if world.player.hp > self.prev_hp + 4.0 {
                self.notify_pickup(PickupKind::Heal);
            } else {
                self.notify_pickup(PickupKind::Magnet);
            }
        }

        // 6. Kill streak burst & kill milestones.
        let dk = world.kills.saturating_sub(self.prev_kills);
        if dk > 0 {
            self.streak_kills = self.streak_kills.saturating_add(dk);
            self.streak_timer = 2.6;
        } else if self.streak_timer > 0.0 {
            self.streak_timer = (self.streak_timer - dt).max(0.0);
            if self.streak_timer <= 0.0 {
                self.streak_kills = 0;
            }
        }
        if world.kills >= self.next_kill_milestone {
            let m = self.next_kill_milestone;
            let msg = match m {
                50 => "50 KILLS  ·  SWARM RISING",
                100 => "100 KILLS  ·  CONTAINMENT STRAINED",
                250 => "250 KILLS  ·  PROTOCOL OVERWHELMED",
                500 => "500 KILLS  ·  APEX PREDATOR",
                _ => "EXTERMINATION THRESHOLD EXCEEDED",
            };
            self.push_callout(CalloutKind::Milestone, "MILESTONE", msg, 2.5);
            self.next_kill_milestone = match m {
                50 => 100,
                100 => 250,
                250 => 500,
                _ => m.saturating_add(500),
            };
        }

        // 7. Time-curve escalation callouts (fire once per threshold).
        let stage = Self::stage_for_elapsed(world.elapsed);
        if stage > self.escalation_stage {
            self.escalation_stage = stage;
            match stage {
                1 => self.push_callout(
                    CalloutKind::Escalation,
                    "THREAT +",
                    "SWARM BATS INBOUND",
                    2.5,
                ),
                2 => self.push_callout(
                    CalloutKind::Escalation,
                    "THREAT +",
                    "STALKER CATS DETECTED",
                    2.5,
                ),
                3 => self.push_callout(
                    CalloutKind::Escalation,
                    "WARNING",
                    "ELITE BRUTES UNLOCKED",
                    2.8,
                ),
                4 => self.push_callout(
                    CalloutKind::Escalation,
                    "THREAT +",
                    "ACID SPITTERS ACTIVE",
                    2.5,
                ),
                5 => self.push_callout(
                    CalloutKind::Escalation,
                    "WARNING",
                    "HEAVY BRUTES DEPLOYED",
                    2.8,
                ),
                6 => self.push_callout(
                    CalloutKind::Escalation,
                    "FINAL 30s",
                    "HOLD OUT FOR EXTRACTION",
                    3.0,
                ),
                _ => {}
            }
        }

        // 8. Tick & expire active callout slots.
        for slot in &mut self.callouts {
            if slot.active {
                slot.timer -= dt;
                if slot.timer <= 0.0 {
                    slot.active = false;
                    slot.timer = 0.0;
                }
            }
        }

        self.prev_hp_ratio = hp_ratio;
        self.prev_hp = world.player.hp;
        self.prev_dash_cd = world.player.dash_cd;
        self.prev_level = world.player.level;
        self.prev_kills = world.kills;
        self.prev_boss_alive = self.telemetry.boss_alive;
    }

    fn stage_for_elapsed(elapsed: f32) -> u8 {
        if elapsed >= director::RUN_SECONDS - 30.0 {
            6
        } else if elapsed >= 120.0 {
            5
        } else if elapsed >= 75.0 {
            4
        } else if elapsed >= director::ELITE_START {
            3
        } else if elapsed >= 45.0 {
            2
        } else if elapsed >= 20.0 {
            1
        } else {
            0
        }
    }
}

/// Clear a retained buffer and refill it, so digits from a longer previous
/// value can never survive into this frame's text.
fn refill(buf: &mut String, fill: impl FnOnce(&mut String)) {
    buf.clear();
    fill(buf);
}

fn fmt_time_into(dst: &mut String, secs: f32) {
    let s = secs.max(0.0) as u32;
    let _ = write!(dst, "{:02}:{:02}", s / 60, s % 60);
}

/// Short weapon tag for the HUD: up to four characters, counted in chars so a
/// non-ASCII name can never slice mid-codepoint.
fn push_short_name(dst: &mut String, name: &str) {
    dst.extend(name.chars().take(4));
}

/// Vertical eighth-cell block glyph for weapon cooldown charge (`0.0..=1.0`).
fn cooldown_pip(ready: f32) -> char {
    const VSTEPS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let idx = ((ready.clamp(0.0, 1.0) * 7.0).round() as usize).min(7);
    VSTEPS[idx]
}

fn rgb_of_color(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (180, 180, 180),
    }
}

pub fn draw_top(frame: &mut Frame, area: Rect, world: &World, state: &mut HudState) {
    if area.is_empty() {
        return;
    }

    let accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    let time_left = world.time_left();
    let urgent = time_left <= 30.0;
    let boss_alive = state.telemetry.boss_alive;
    let boss_soon = !world.director.boss_spawned
        && world.elapsed >= director::BOSS_TIME - 10.0
        && world.elapsed < director::BOSS_TIME;
    let pulse = 0.5 + 0.5 * (state.clock * 7.5).sin();

    // Format numbers into retained buffers (zero per-frame heap allocation).
    refill(&mut state.time, |b| fmt_time_into(b, time_left));
    refill(&mut state.level, |b| {
        let _ = write!(b, "{}", world.player.level);
    });
    refill(&mut state.kills, |b| {
        let _ = write!(b, "{}", world.kills);
    });
    refill(&mut state.streak_buf, |b| {
        if state.streak_kills >= 8 {
            let _ = write!(b, " ×{}", state.streak_kills);
        }
    });
    refill(&mut state.score, |b| {
        let _ = write!(b, "{}", world.score);
    });

    // Weapon tags: reused `(tag, level)` buffers per slot.
    let weapons = &world.player.loadout.weapons;
    while state.weapon_tags.len() < weapons.len() {
        state.weapon_tags.push((String::new(), String::new()));
    }
    state.weapon_tags.truncate(weapons.len());
    for (i, w) in weapons.iter().enumerate() {
        let (tag, lvl) = &mut state.weapon_tags[i];
        tag.clear();
        push_short_name(tag, w.kind.name());
        lvl.clear();
        if w.is_max() {
            lvl.push('★');
        } else {
            let _ = write!(lvl, "{}", w.level);
        }
    }

    // Build right-aligned status pill text.
    refill(&mut state.right_badge, |b| {
        if urgent {
            b.push_str(" FINAL SECONDS ");
        } else if boss_alive {
            b.push_str(" ◆ APEX ACTIVE ");
        } else if boss_soon {
            let rem = (director::BOSS_TIME - world.elapsed).ceil().max(1.0) as u32;
            let _ = write!(b, " ▲ BOSS {:02}s ", rem);
        } else if !world.director.boss_spawned
            && world.elapsed >= director::ELITE_START - 5.0
            && world.director.elite_timer <= 4.0
        {
            b.push_str(" ▲ ELITE ");
        } else if state.telemetry.muted {
            b.push_str(" MUTE ");
        } else if state.telemetry.enemy_count >= 55 {
            let _ = write!(b, " SWARM {} ", state.telemetry.enemy_count);
        } else {
            let _ = write!(b, " {} ", world.level.theme.name().to_uppercase());
        }
    });

    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(PLATE));
    let mut x = area.x;
    let right = area.right();

    let mut put = |s: &str, style: Style| {
        let avail = right.saturating_sub(x) as usize;
        if avail == 0 {
            return;
        }
        (x, _) = buf.set_stringn(x, area.y, s, avail, style);
    };

    let compact = area.width < 72 && weapons.len() > 3;
    let tight = area.width < 64;
    let show_threat = area.width >= 96;
    let show_cd_pip = area.width >= 82 || (area.width >= 68 && weapons.len() <= 4);
    let show_icons = !tight || weapons.len() <= 3;
    let show_streak = area.width >= 92 && !state.streak_buf.is_empty();

    let value = Style::default().fg(COAT).add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(MUTED);

    // 1. Clock badge + countdown.
    let badge_bg = if urgent {
        palette::to_color(palette::mix((239, 168, 185), (248, 92, 102), pulse))
    } else if boss_alive {
        palette::to_color(palette::mix((220, 64, 118), (248, 116, 152), pulse))
    } else if boss_soon {
        WARN_AMBER
    } else {
        accent
    };
    let badge_label = if boss_alive {
        if compact {
            "◆"
        } else {
            " ◆ "
        }
    } else if urgent {
        if compact {
            "!"
        } else {
            " ! "
        }
    } else if compact {
        "T"
    } else {
        " T "
    };
    put(
        badge_label,
        Style::default()
            .bg(badge_bg)
            .fg(PLATE)
            .add_modifier(Modifier::BOLD),
    );
    let time_style = if urgent {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    } else if boss_alive {
        Style::default().fg(TAIL).add_modifier(Modifier::BOLD)
    } else {
        value
    };
    put(" ", Style::default().bg(PLATE));
    put(&state.time, time_style);

    // 2. Threat escalation micro-pips on wider screens (`▪▪▪▫▫`).
    if show_threat {
        let tier = threat_tier(world.elapsed, boss_alive);
        put(" ", Style::default().bg(PLATE));
        for t in 1..=5u8 {
            let pip_col = if t <= tier {
                match tier {
                    5 => BOSS_ROSE,
                    4 => WARN_AMBER,
                    _ => accent,
                }
            } else {
                TRACK
            };
            put(
                if t <= tier { "▪" } else { "▫" },
                Style::default().fg(pip_col).bg(PLATE),
            );
        }
    }

    // 3. Level, Kills (+ optional streak), Score.
    let lv_style = if state.level_flash > 0.0 {
        Style::default().fg(TRAIL_GOLD).add_modifier(Modifier::BOLD)
    } else {
        value
    };
    put(if compact { " Lv" } else { "  Lv" }, muted);
    put(&state.level, lv_style);

    put(if compact { " KO" } else { "  KO " }, muted);
    put(&state.kills, value);
    if show_streak {
        put(
            &state.streak_buf,
            Style::default().fg(WARN_AMBER).add_modifier(Modifier::BOLD),
        );
    }

    put(if compact { " Score " } else { "  Score " }, muted);
    put(&state.score, value);

    // 4. Weapon rack with icons, level, firing flare, and live cooldown pip.
    put(if compact { " │" } else { "  │" }, Style::default().fg(EDGE));
    let fire_rate = world.player.loadout.stats.fire_rate_mult.max(0.1);
    for ((tag, lvl), weapon) in state.weapon_tags.iter().zip(weapons) {
        let eff_cd = (weapon.cooldown() / fire_rate).max(0.05);
        let ready = (1.0 - weapon.timer / eff_cd).clamp(0.0, 1.0);
        // Immediately after firing, `timer` resets near `eff_cd` (`ready` near 0):
        // flare the weapon icon white-hot while the cooldown bar starts low and climbs.
        let just_fired = ready < 0.12 && world.elapsed > 0.25;
        let base_rgb = weapon.kind.color();
        let w_col = palette::to_color(base_rgb);
        let icon_style = if just_fired {
            Style::default().fg(COAT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(w_col)
        };
        let w_style = if just_fired {
            Style::default()
                .fg(palette::to_color(palette::mix(base_rgb, (255, 255, 255), 0.35)))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(w_col)
        };

        put(" ", Style::default().bg(PLATE));
        if show_icons {
            let mut gbuf = [0u8; 4];
            let g = weapon_glyph(weapon.kind).encode_utf8(&mut gbuf);
            put(g, icon_style);
        }
        // Two letters distinguish Spore from Squeak even in a 57-col full loadout.
        let end = if compact {
            tag.char_indices().nth(2).map_or(tag.len(), |(i, _)| i)
        } else {
            tag.len()
        };
        put(&tag[..end], w_style);
        let lvl_style = if weapon.is_max() {
            Style::default().fg(TRAIL_GOLD).add_modifier(Modifier::BOLD)
        } else {
            muted
        };
        put(lvl, lvl_style);

        if show_cd_pip {
            let pip_ch = cooldown_pip(ready);
            let pip_fg = if ready >= 0.88 {
                w_col
            } else {
                palette::to_color(palette::mix((82, 94, 100), base_rgb, 0.35 + ready * 0.65))
            };
            let mut pbuf = [0u8; 4];
            let p = pip_ch.encode_utf8(&mut pbuf);
            put(p, Style::default().fg(pip_fg).bg(TRACK));
        }
    }

    // 5. Right-aligned contextual status pill (only if it won't collide with weapons).
    let badge_w = state.right_badge.chars().count() as u16;
    if badge_w > 0 && right.saturating_sub(x) >= badge_w + 2 {
        let bx = right - badge_w;
        // If audio is muted while a higher-priority status badge owns the right slot,
        // tuck a compact `MUTE` chip immediately to its left when space permits.
        if state.telemetry.muted
            && state.right_badge != " MUTE "
            && bx.saturating_sub(x) >= 8
        {
            buf.set_stringn(
                bx - 7,
                area.y,
                " MUTE ",
                6,
                Style::default().fg(TAIL).bg(TRACK),
            );
        }
        let style = if urgent {
            Style::default()
                .fg(PLATE)
                .bg(palette::to_color(palette::mix(
                    (239, 168, 185),
                    (248, 92, 102),
                    pulse,
                )))
                .add_modifier(Modifier::BOLD)
        } else if boss_alive {
            Style::default()
                .fg(BOSS_ROSE)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else if boss_soon {
            Style::default()
                .fg(PLATE)
                .bg(WARN_AMBER)
                .add_modifier(Modifier::BOLD)
        } else if !world.director.boss_spawned
            && world.elapsed >= director::ELITE_START - 5.0
            && world.director.elite_timer <= 4.0
        {
            Style::default()
                .fg(WARN_AMBER)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else if state.telemetry.muted {
            Style::default().fg(TAIL).bg(TRACK)
        } else if state.telemetry.enemy_count >= 55 {
            Style::default()
                .fg(WARN_AMBER)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(EDGE).bg(PLATE)
        };
        buf.set_stringn(bx, area.y, &state.right_badge, badge_w as usize, style);
    }
}

pub fn draw_bottom(frame: &mut Frame, area: Rect, world: &World, state: &mut HudState) {
    if area.is_empty() {
        return;
    }

    let max_hp = world.player.max_hp().max(1.0);
    let hp_val = world.player.hp.max(0.0);
    let hp = (hp_val / max_hp).clamp(0.0, 1.0);
    let low_hp = hp <= 0.28;
    let pulse = 0.5 + 0.5 * (state.clock * 8.0).sin();

    let hp_color = if low_hp {
        palette::to_color(palette::mix((214, 58, 70), (255, 110, 118), pulse))
    } else if hp <= 0.55 {
        palette::to_color(palette::mix((244, 154, 114), (239, 168, 185), (hp - 0.28) / 0.27))
    } else {
        TAIL
    };

    let hurt_active = world.player.hurt_flash > 0.0 || state.hurt_edge > 0.15;
    refill(&mut state.hp, |b| {
        let _ = write!(b, "♥ {:.0}/{:.0}", hp_val, max_hp);
        if area.width >= 96 && world.player.loadout.stats.armor > 0.0 {
            let _ = write!(b, " A{:.0}", world.player.loadout.stats.armor);
        }
    });

    let xp_next = world.player.xp_to_next.max(1);
    let xp = (world.player.xp as f32 / xp_next as f32).clamp(0.0, 1.0);
    refill(&mut state.xp_label, |b| {
        if area.width >= 92 {
            let pct = (xp * 100.0).round() as u32;
            let _ = write!(b, "XP {:>2}%", pct.min(99));
        } else {
            b.push_str("XP");
        }
    });

    // Dash readiness + charge meter text.
    let dashing = world.player.dash_active > 0.0;
    let dash_ready = world.player.dash_cd <= 0.0;
    let dash_max_cd = world.player.loadout.stats.dash_cd.max(0.1);
    let dash_fill = if dashing {
        1.0
    } else {
        (1.0 - world.player.dash_cd / dash_max_cd).clamp(0.0, 1.0)
    };
    refill(&mut state.dash, |b| {
        if dashing {
            b.push_str("» DASHING «");
        } else if dash_ready {
            if area.width < 48 {
                b.push_str("▸ DASH");
            } else {
                b.push_str("▸ DASH READY");
            }
        } else {
            let _ = write!(b, "dash {:.1}s", world.player.dash_cd);
        }
    });

    let accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    let xp_color = if state.level_flash > 0.0 {
        palette::to_color(palette::mix(
            rgb_of_color(accent),
            (252, 226, 152),
            (state.level_flash / 0.85).clamp(0.0, 1.0),
        ))
    } else if xp >= 0.85 {
        palette::to_color(palette::mix(
            world.level.palette.accent,
            (248, 232, 186),
            0.45 + 0.25 * pulse,
        ))
    } else {
        palette::to_color(palette::mix(
            world.level.palette.accent,
            (176, 198, 194),
            0.42,
        ))
    };

    // Reserve dash module width: `│ ` divider + label + optional 5-cell mini charge bar.
    let show_sep = area.width >= 52;
    let sep_w: u16 = if show_sep { 2 } else { 0 };
    let show_dash_bar = !dash_ready && !dashing && area.width >= 76;
    let dash_bar_w: u16 = if show_dash_bar { 6 } else { 0 };
    let dash_text_w = state.dash.chars().count() as u16;
    let pad_w: u16 = if dash_ready || dashing { 2 } else { 1 };
    let dash_width = (sep_w + dash_text_w + dash_bar_w + pad_w).min(area.width);
    let meters = area.width.saturating_sub(dash_width);
    let left = (meters / 2 + 3).min(meters);

    // 1. HP meter with trailing damage ghost bar.
    let hp_label_style = if hurt_active {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    } else if low_hp {
        Style::default().fg(hp_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(COAT)
    };
    meter_with_trail(
        frame,
        Rect::new(area.x, area.y, left, 1),
        &state.hp,
        hp_label_style,
        hp,
        state.hp_trail,
        hp_color,
        TRAIL_GOLD,
    );

    // 2. XP meter.
    meter_with_trail(
        frame,
        Rect::new(area.x + left, area.y, meters.saturating_sub(left), 1),
        &state.xp_label,
        Style::default().fg(COAT),
        xp,
        xp,
        xp_color,
        xp_color,
    );

    // 3. Dash module: crisp divider, then text label, then optional recharge gauge.
    let dash_rect = Rect::new(area.right() - dash_width, area.y, dash_width, 1);
    let buf = frame.buffer_mut();
    buf.set_style(dash_rect, Style::default().bg(PLATE));
    let mut dx = dash_rect.x;
    if show_sep && dx < dash_rect.right() {
        buf[(dx, dash_rect.y)]
            .set_char('│')
            .set_fg(EDGE)
            .set_bg(PLATE);
        dx = dx.saturating_add(2);
    }
    let dash_style = if dashing {
        Style::default()
            .fg(PLATE)
            .bg(COAT)
            .add_modifier(Modifier::BOLD)
    } else if dash_ready {
        if state.dash_ready_flash > 0.0 {
            Style::default()
                .fg(PLATE)
                .bg(accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(COAT)
                .bg(TRACK)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().fg(MUTED).bg(PLATE)
    };
    let avail = dash_rect.right().saturating_sub(dx) as usize;
    if avail > 0 {
        if dash_ready || dashing {
            buf.set_stringn(dx, dash_rect.y, " ", avail, dash_style);
            let (end, _) = buf.set_stringn(
                dx.saturating_add(1),
                dash_rect.y,
                &state.dash,
                avail.saturating_sub(1),
                dash_style,
            );
            if end < dash_rect.right() {
                buf[(end, dash_rect.y)].set_char(' ').set_style(dash_style);
            }
        } else {
            let (end, _) = buf.set_stringn(dx, dash_rect.y, &state.dash, avail, dash_style);
            if show_dash_bar {
                let bx = end.saturating_add(1);
                let bw = dash_rect.right().saturating_sub(bx + 1).min(5);
                if bw > 0 {
                    draw_mini_bar(buf, bx, dash_rect.y, bw, dash_fill, accent);
                }
            }
        }
    }
}

/// Draw a compact eighth-cell bar directly into `buf` with no label.
pub(crate) fn draw_mini_bar(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    width: u16,
    ratio: f32,
    color: Color,
) {
    const STEPS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let units = (ratio.clamp(0.0, 1.0) * f32::from(width) * 8.0).round() as usize;
    for i in 0..width {
        let step = units.saturating_sub(8 * i as usize).min(8);
        buf[(x + i, y)]
            .set_char(STEPS[step])
            .set_fg(color)
            .set_bg(TRACK);
    }
}

/// Labels occupy their own dark strip so their contrast is independent of fill.
/// Supports an optional trailing damage-ghost bar (`trail > ratio`).
#[allow(clippy::too_many_arguments)]
fn meter_with_trail(
    frame: &mut Frame,
    r: Rect,
    label: &str,
    label_style: Style,
    ratio: f32,
    trail: f32,
    color: Color,
    trail_color: Color,
) {
    const STEPS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    if r.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(r, Style::default().bg(PLATE));
    let (end, _) = buf.set_stringn(r.x, r.y, label, r.width as usize, label_style);
    let start = end.saturating_add(1).min(r.right());
    let width = r
        .right()
        .saturating_sub(start + u16::from(start < r.right()));
    if width == 0 {
        return;
    }
    let ratio = ratio.clamp(0.0, 1.0);
    let trail = trail.clamp(ratio, 1.0);
    let units = (ratio * f32::from(width) * 8.0).round() as usize;
    let trail_units = (trail * f32::from(width) * 8.0).round() as usize;
    for i in 0..width {
        let cell_base = 8 * i as usize;
        let fill_step = units.saturating_sub(cell_base).min(8);
        let trail_step = trail_units.saturating_sub(cell_base).min(8);
        let cell = &mut buf[(start + i, r.y)];
        if fill_step == 8 {
            cell.set_char('█').set_fg(color).set_bg(TRACK);
        } else if fill_step > 0 {
            let bg = if trail_step > fill_step {
                trail_color
            } else {
                TRACK
            };
            cell.set_char(STEPS[fill_step]).set_fg(color).set_bg(bg);
        } else if trail_step > 0 {
            cell.set_char(STEPS[trail_step])
                .set_fg(trail_color)
                .set_bg(TRACK);
        } else {
            cell.set_char(' ').set_fg(color).set_bg(TRACK);
        }
    }
}

/// Preserved thin wrapper for unit testing single-fill eighth-cell precision.
#[cfg(test)]
fn meter(frame: &mut Frame, r: Rect, label: &str, ratio: f32, color: Color) {
    meter_with_trail(
        frame,
        r,
        label,
        Style::default().fg(COAT),
        ratio,
        ratio,
        color,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::weapon::Weapon;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn push_short_name_is_char_safe() {
        let mut s = String::new();
        push_short_name(&mut s, "Gnaw");
        assert_eq!(s, "Gnaw");

        s.clear();
        push_short_name(&mut s, "Teeth");
        assert_eq!(s, "Teet");

        s.clear();
        push_short_name(&mut s, "");
        assert_eq!(s, "");

        s.clear();
        push_short_name(&mut s, "🗡️ knife");
        assert_eq!(s.chars().count(), 4);
        assert!(s.starts_with('🗡'));
    }

    fn known_world() -> World {
        let mut w = World::new(Theme::Sewer, 7);
        w.elapsed = 65.0; // 300s run → 235s left
        w.player.level = 3;
        w.kills = 42;
        w.score = 1234;
        w.player.loadout.weapons[0].level = 2;
        w.player.hp = 100.0;
        w.player.dash_cd = 0.0;
        w
    }

    fn row_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn top_line_shows_clock_counters_and_weapon_icon() {
        let w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_top(f, area, &w, &mut state);
            })
            .unwrap();

        let text = row_text(&terminal);
        assert!(text.contains("03:55"), "missing countdown in {text:?}");
        assert!(text.contains("Lv3"), "missing level in {text:?}");
        assert!(text.contains("KO 42"), "missing kills in {text:?}");
        assert!(text.contains("Score 1234"), "missing score in {text:?}");
        assert!(text.contains("▸Gnaw2"), "missing icon + weapon in {text:?}");
    }

    #[test]
    fn bottom_line_text_matches_reference_and_reuse_is_clean() {
        let mut w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(57, 1)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_bottom(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("XP"));
        assert!(text.contains("♥ 100/100"), "HP missing in {text:?}");
        assert!(text.contains("DASH READY"), "dash missing in {text:?}");

        // Shrink every number, reusing the same buffers: no stale digits may survive.
        w.player.hp = 1.0;
        w.player.dash_cd = 0.25;
        w.elapsed = 299.9;
        w.kills = 1;
        w.score = 2;
        w.player.level = 1;
        terminal
            .draw(|f| {
                let area = f.area();
                draw_bottom(f, area, &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("XP"));
        assert!(text.contains("♥ 1/100"), "HP not updated in {text:?}");
        assert!(text.contains("dash 0.2s"), "dash cd not updated in {text:?}");
    }

    #[test]
    fn repeated_draws_are_identical() {
        let w = known_world();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();

        let mut draw = |state: &mut HudState| {
            terminal
                .draw(|f| {
                    let area = f.area();
                    draw_top(f, area, &w, state);
                    let bottom = Rect::new(0, 1, area.width, 1);
                    draw_bottom(f, bottom, &w, state);
                })
                .unwrap();
            row_text(&terminal)
        };

        let first = draw(&mut state);
        let second = draw(&mut state);
        assert_eq!(first, second, "reused buffers must render identically");
        assert!(
            first.contains('♥') && first.contains("DASH READY"),
            "bottom bars should still render: {first:?}"
        );
    }

    #[test]
    fn full_six_weapon_loadout_fits_at_57_cols_and_shows_max_star() {
        let mut w = known_world();
        w.player.loadout.weapons = WeaponKind::all().into_iter().map(Weapon::new).collect();
        w.player.loadout.weapons[0].level = WeaponKind::Gnaw.max_level();
        let mut state = HudState::default();
        let mut terminal = Terminal::new(TestBackend::new(57, 1)).unwrap();
        terminal
            .draw(|f| {
                draw_top(f, f.area(), &w, &mut state);
            })
            .unwrap();
        let text = row_text(&terminal);
        assert!(text.contains("Gn★"), "max-level star missing in {text:?}");
        assert!(text.contains("Ac1"), "sixth weapon clipped in {text:?}");
    }

    #[test]
    fn meter_resolves_all_eighths_in_the_last_cell() {
        let mut terminal = Terminal::new(TestBackend::new(4, 1)).unwrap();
        for (n, expected) in " ▏▎▍▌▋▊▉█".chars().enumerate() {
            terminal
                .draw(|f| meter(f, f.area(), "X", n as f32 / 8.0, TAIL))
                .unwrap();
            let cell = &terminal.backend().buffer()[(2, 0)];
            assert_eq!(cell.symbol().chars().next(), Some(expected));
            assert_eq!(cell.bg, TRACK);
        }
    }

    #[test]
    fn damage_trail_renders_gold_between_hp_and_trail() {
        let mut terminal = Terminal::new(TestBackend::new(14, 1)).unwrap();
        terminal
            .draw(|f| {
                meter_with_trail(
                    f,
                    f.area(),
                    "H",
                    Style::default().fg(COAT),
                    0.3,
                    0.8,
                    TAIL,
                    TRAIL_GOLD,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let has_trail = (0..14).any(|x| buf[(x, 0)].fg == TRAIL_GOLD || buf[(x, 0)].bg == TRAIL_GOLD);
        assert!(has_trail, "damage trail should render in TRAIL_GOLD");
    }

    #[test]
    fn hud_tick_emits_escalation_and_pickup_callouts() {
        let mut w = World::new(Theme::Sewer, 7);
        let mut state = HudState::default();
        state.tick(1.0 / 60.0, &w, &[], false);

        // Cross 20s Bat unlock threshold + Heal pickup.
        w.elapsed = 21.0;
        w.player.hp = 60.0;
        state.tick(1.0 / 60.0, &w, &[], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "THREAT +"),
            "crossing 20s should emit escalation callout"
        );

        w.player.hp = 95.0;
        state.tick(1.0 / 60.0, &w, &[Sfx::Pickup], false);
        assert!(
            state.callouts.iter().any(|c| c.active && c.tag == "+MEDKIT"),
            "heal pickup should emit MEDKIT callout"
        );
    }
}
