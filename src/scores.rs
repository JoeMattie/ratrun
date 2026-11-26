//! High-score table persisted as JSON in the user data dir.
//!
//! The table holds run records and profile data:
//!
//! * `entries` — one record per finished run: the **cumulative** campaign
//!   score, the campaign level the run reached (1..=3) and whether the run won
//!   the campaign. Files written by an older build have no campaign fields;
//!   they load as "reached level 1, campaign not won" (see
//!   [`ScoreEntry::campaign_level`]).
//! * `progress` — the **per-floor** bests and clear flags behind the title's
//!   sector-select. A campaign stage unlocks once the previous stage has been
//!   cleared on any of its floors, so the flags must persist independently of
//!   the top-ten entries (a strong table can drop an old entry, but it never
//!   relocks a floor). Files written before floors were tracked separately
//!   store one record per campaign *stage*; they migrate to floors on load
//!   (see [`CampaignProgress::migrate_legacy`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::audio;
#[cfg(not(target_arch = "wasm32"))]
use crate::config;
use crate::controls::{Controls, SavedControls, SavedScheme};
use crate::game::campaign::{self, LEVEL_COUNT};
use crate::game::level::Theme;
use crate::game::library::Library;
use crate::render::cursor::CursorSetting;
use crate::render::minimap::MinimapSize;

pub const MAX_ENTRIES: usize = 10;

/// The campaign level an old score file's entries belong to.
fn default_campaign_level() -> u8 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreEntry {
    /// Cumulative score of the whole run (completed levels + this one + bonuses).
    pub score: u32,
    /// Total seconds played across the whole run.
    pub time: f32,
    /// The player's character level at the end of the run.
    pub level: u32,
    /// The sector the run ended (or finished) in, for the records table.
    pub map: String,
    /// Did the run win? For a campaign that means the Lab boss died; a practice
    /// clear also shows the victory styling.
    pub won: bool,
    /// Which campaign level the run reached, 1..=3.
    ///
    /// Missing in files written before the campaign existed, which default to
    /// `1` (every old run was a single Sewer map).
    #[serde(default = "default_campaign_level")]
    pub campaign_level: u8,
    /// Did the run clear the whole three-level campaign (not just a practice
    /// sector)? Missing in old files ⇒ `false`.
    #[serde(default)]
    pub campaign_won: bool,
}

/// Best results recorded for one **floor** (sector-select cards and the bests
/// the title reads).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LevelBestEntry {
    /// Has this floor been completed in any run?
    #[serde(default)]
    pub cleared: bool,
    /// Highest score earned on this floor alone.
    #[serde(default)]
    pub score: u32,
    /// Fastest completion of this floor, in seconds (`0.0` = none yet).
    #[serde(default)]
    pub time: f32,
}

impl LevelBestEntry {
    /// Fold `other`'s record into this one: any clear counts, the higher score
    /// wins, the faster positive time wins. Used when a legacy stage record is
    /// migrated onto the floor it belonged to.
    fn merge(&mut self, other: LevelBestEntry) {
        self.cleared |= other.cleared;
        self.score = self.score.max(other.score);
        if other.time > 0.0 && (self.time <= 0.0 || other.time < self.time) {
            self.time = other.time;
        }
    }

    /// Does this record hold anything at all? (An all-default legacy slot is
    /// not migrated, so an old file does not fabricate floor records.)
    fn is_empty(&self) -> bool {
        !self.cleared && self.score == 0 && self.time <= 0.0
    }
}

/// Persisted per-floor progress: clear flags + best results.
///
/// Keyed by [`Theme::name`] rather than by campaign stage: the Kitchen and the
/// Laundry are both stage two, and each keeps its own best. Unknown keys in a
/// hand-edited file are preserved (they round-trip) but never read.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CampaignProgress {
    /// Per-floor records, keyed by the floor's [`Theme::name`].
    #[serde(default)]
    pub floors: BTreeMap<String, LevelBestEntry>,
    /// Stage-keyed records written by the three-stage build. Loaded into
    /// [`Self::floors`] by [`Self::migrate_legacy`] and never written back
    /// (`skip_serializing_if`), so a migrated file is a floors-only file.
    ///
    /// A `Vec`, not `[LevelBestEntry; LEVEL_COUNT]`: files on disk carry the
    /// **three** records of the three-level campaign, and a fixed array of four
    /// would refuse to deserialise them. (Batch twelve: the Flatbed joined the
    /// campaign as its fourth level.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<LevelBestEntry>>,
}

impl CampaignProgress {
    /// The record stored for `theme`, cleared or not.
    fn get(&self, theme: Theme) -> Option<&LevelBestEntry> {
        self.floors.get(theme.name())
    }

    /// Record a completion on `theme`, keeping the highest score and the
    /// fastest positive time.
    fn record(&mut self, theme: Theme, score: u32, time: f32) {
        let entry = self.floors.entry(theme.name().to_string()).or_default();
        // A non-finite time would be written as `null` and fail the next load;
        // `0.0` is this record's "no time yet".
        let time = if time.is_finite() { time.max(0.0) } else { 0.0 };
        let fresh = LevelBestEntry {
            cleared: true,
            score,
            time,
        };
        entry.merge(fresh);
        entry.cleared = true;
    }

    /// Move the old stage-keyed `levels` array onto the floors it belonged to:
    /// record 0 was the Sewer, 1 the Kitchen, 2 the Lab (the original route;
    /// `campaign::level_theme` is that same mapping). Idempotent: the legacy
    /// array is taken, so a second call does nothing.
    ///
    /// A three-level file therefore folds onto Sewer/Kitchen/Lab, exactly as
    /// before the ride joined the campaign: a save whose campaign was complete
    /// at three levels has the Lab cleared, which opens the Flatbed's card
    /// (stage four unlocks on a cleared stage three) **without** inventing a
    /// clear for the ride itself. See `campaign::Route::options`.
    fn migrate_legacy(&mut self) {
        let Some(levels) = self.levels.take() else {
            return;
        };
        for (index, entry) in levels.iter().enumerate() {
            if entry.is_empty() {
                continue;
            }
            let theme = campaign::level_theme(index);
            self.floors
                .entry(theme.name().to_string())
                .or_default()
                .merge(*entry);
        }
    }
}

fn serialize_minimap_size<S: serde::Serializer>(
    size: &MinimapSize,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(size.as_str())
}

fn deserialize_minimap_size<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<MinimapSize, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_str()
        .and_then(MinimapSize::parse_setting)
        .unwrap_or(MinimapSize::Normal))
}

fn serialize_cursor_setting<S: serde::Serializer>(
    setting: &CursorSetting,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(setting.as_str())
}

fn deserialize_cursor_setting<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<CursorSetting, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(b) = value.as_bool() {
        return Ok(if b {
            CursorSetting::On
        } else {
            CursorSetting::Off
        });
    }
    Ok(value
        .as_str()
        .and_then(CursorSetting::parse_setting)
        .unwrap_or(CursorSetting::On))
}

/// A 0..=100 volume preference, tolerant of hand-edited files: a number or a
/// numeric string is snapped to the 5% grid and clamped, anything else (or a
/// missing field) falls back to `default_volume` so a single bad value can
/// never drop the rest of the table.
fn deserialize_volume<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u8, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let raw = value
        .as_u64()
        .map(|n| n.min(255) as u8)
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<u8>().ok()));
    Ok(raw.map(audio::snap_percent).unwrap_or(default_volume()))
}

/// Full volume: the balance every build before the sliders shipped had.
fn default_volume() -> u8 {
    100
}

/// The unzoomed view: what every build before the mouse wheel shipped had.
fn default_zoom() -> f32 {
    crate::render::zoom::DEFAULT
}

/// A zoom preference, tolerant of hand-edited files: a number (or a numeric
/// string) is clamped onto the ladder and snapped to its nearest rung; anything
/// else — including a missing field in an old file — loads unzoomed.
fn deserialize_zoom<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let raw = value
        .as_f64()
        .map(|n| n as f32)
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<f32>().ok()));
    Ok(raw
        .map(crate::render::zoom::snap)
        .unwrap_or_else(default_zoom))
}


/// The saved form of one scheme's bindings (see [`crate::controls`]). A
/// missing or short list falls back to that entry's default, so a hand-edited
/// or old file can never lose the whole control table.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedSchemeFile {
    #[serde(default)]
    pub kbm: Vec<String>,
    #[serde(default)]
    pub alt: Vec<String>,
    #[serde(default)]
    pub pad: Vec<String>,
}

/// The Graphics page's preferences ([`crate::render::graphics`]): the
/// render-only picture settings that used to be console/env switches only.
///
/// Every field is a *preference*, not run state: nothing here is read by the
/// simulation (see the determinism rules in `CLAUDE.md`), so a hand-edited
/// value can change what the player sees and nothing else. `#[serde(default)]`
/// on the container means an old `scores.json` — written before the page
/// existed — loads with today's defaults, and a single unreadable field keeps
/// the rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsFile {
    /// Version 2 reset smoothing to off. Version 3 starts foreground on.
    #[serde(default)]
    pub graphics_version: u32,
    /// `full` | `hard` | `coarse` | `compact` — the lighting quality the console's
    /// `lights` command sets. Absent means "whatever this terminal's colour
    /// depth defaults to" (the only correct answer before the player picks).
    pub lighting: Option<String>,
    /// `off` | `xbr` | `crisp`.
    pub smooth: String,
    /// The parallax overhead layer (`render::foreground`).
    pub foreground: bool,
    /// The batch-13 lighting effects (`render::lighting::fx`).
    pub fx: bool,
    /// The round-glyph particle layer (`render::glyphfx`).
    pub glyphfx: bool,
    /// The camera's share of the simulation's shake (`World::shake_scale`),
    /// one of [`crate::render::graphics::SHAKE_STEPS`].
    pub shake_scale: f32,
    /// `off` | `ao` | `gi` — ambient occlusion and one-bounce global
    /// illumination (`render::lighting::GlobalLight`). Defaults to `off`.
    pub global_light: String,
}

const GRAPHICS_VERSION: u32 = 3;

impl Default for GraphicsFile {
    /// The terminal's own lighting tier, smoothing and global light off,
    /// foreground, light effects and glyph particles on, and 65 % shake.
    fn default() -> Self {
        GraphicsFile {
            graphics_version: GRAPHICS_VERSION,
            lighting: None,
            smooth: crate::render::subcell::Smooth::default().as_str().to_string(),
            foreground: true,
            fx: true,
            glyphfx: true,
            shake_scale: crate::game::world::SHAKE_SCALE,
            global_light: crate::render::lighting::GlobalLight::default().as_str().to_string(),
        }
    }
}

impl GraphicsFile {
    /// The saved lighting quality, if the player ever chose one and it is still
    /// a mode this build knows (`null` for a fresh profile, `None` again for a
    /// word a future build wrote).
    pub fn lighting_mode(&self) -> Option<crate::render::lighting::LightingMode> {
        use crate::render::lighting::LightingMode;
        match self.lighting.as_deref()?.trim().to_ascii_lowercase().as_str() {
            "full" | "on" => Some(LightingMode::Full),
            "hard" => Some(LightingMode::Hard),
            "coarse" => Some(LightingMode::Coarse),
            "compact" => Some(LightingMode::Compact),
            _ => None,
        }
    }

    pub fn set_lighting(&mut self, mode: crate::render::lighting::LightingMode) {
        self.lighting = Some(format!("{mode:?}").to_ascii_lowercase());
    }

    /// The saved 1× edge treatment; an unreadable word loads the default.
    pub fn smooth(&self) -> crate::render::subcell::Smooth {
        crate::render::subcell::Smooth::parse(&self.smooth)
            .unwrap_or_default()
    }

    pub fn set_smooth(&mut self, level: crate::render::subcell::Smooth) {
        self.smooth = level.as_str().to_string();
    }

    /// The saved global-light mode; an unreadable word loads `Off`.
    pub fn global_light(&self) -> crate::render::lighting::GlobalLight {
        crate::render::lighting::GlobalLight::parse(&self.global_light).unwrap_or_default()
    }

    pub fn set_global_light(&mut self, mode: crate::render::lighting::GlobalLight) {
        self.global_light = mode.as_str().to_string();
    }

    /// The saved shake share, clamped into `0.0..=1.0` (a hand-edited file, or
    /// a NaN, loads the default).
    pub fn shake(&self) -> f32 {
        if self.shake_scale.is_finite() {
            self.shake_scale.clamp(0.0, 1.0)
        } else {
            crate::game::world::SHAKE_SCALE
        }
    }

    pub fn set_shake(&mut self, scale: f32) {
        self.shake_scale = if scale.is_finite() {
            scale.clamp(0.0, 1.0)
        } else {
            crate::game::world::SHAKE_SCALE
        };
    }

    /// Clamp what a hand-edited file can get wrong (see
    /// [`ScoreTable::normalize`]).
    fn normalize(&mut self) {
        if self.lighting_mode().is_none() {
            self.lighting = None;
        }
        // Older builds saved defaults alongside any edited graphics row.
        let smooth = if self.graphics_version < 2 {
            crate::render::subcell::Smooth::Off
        } else {
            self.smooth()
        };
        if self.graphics_version < 3 {
            self.foreground = true;
            self.graphics_version = GRAPHICS_VERSION;
        }
        self.set_smooth(smooth);
        self.set_global_light(self.global_light());
        self.set_shake(self.shake());
    }
}

/// The persisted control configuration: scheme, per-scheme bindings and the
/// pad tuning. Missing (an old `scores.json`) loads the defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControlsFile {
    #[serde(default)]
    pub scheme: String,
    #[serde(default)]
    pub schemes: Vec<Option<SavedSchemeFile>>,
    #[serde(default)]
    pub deadzone: f32,
    #[serde(default)]
    pub rumble: bool,
}

/// Load the control configuration from its file form.
pub fn controls_from_file(file: &ControlsFile) -> Controls {
    let mut saved = SavedControls {
        scheme: file.scheme.clone(),
        schemes: [None, None, None],
        deadzone: file.deadzone,
        rumble: file.rumble,
    };
    for (i, slot) in saved.schemes.iter_mut().enumerate() {
        if let Some(s) = file.schemes.get(i).and_then(|o| o.as_ref()) {
            *slot = Some(SavedScheme {
                kbm: s.kbm.clone(),
                alt: s.alt.clone(),
                pad: s.pad.clone(),
            });
        }
    }
    Controls::from_saved(&saved)
}

/// The file form of a control configuration.
pub fn controls_to_file(controls: &Controls) -> ControlsFile {
    let saved = controls.to_saved();
    ControlsFile {
        scheme: saved.scheme.clone(),
        schemes: saved
            .schemes
            .iter()
            .map(|s| {
                s.as_ref().map(|s| SavedSchemeFile {
                    kbm: s.kbm.clone(),
                    alt: s.alt.clone(),
                    pad: s.pad.clone(),
                })
            })
            .collect(),
        deadzone: saved.deadzone,
        rumble: saved.rumble,
    }
}

/// The nest, tolerant of a hand-edited file: anything that is not a nest object
/// (a string, a list, null) loads as a fresh profile instead of failing the
/// whole table and dropping the player's scores with it.
fn deserialize_meta<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<crate::meta::MetaProfile, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match serde_json::from_value(value.clone()) {
        Ok(profile) => profile,
        Err(_) => salvage_meta(&value),
    })
}

/// Read what can be read of a nest whose strict parse failed: one bad number
/// (a bank edited past `u32`, a level past `u8`, a negative or fractional
/// value) must not wipe every track the player bought. Out-of-range numbers
/// saturate, unreadable ones are dropped; `MetaProfile::normalize` clamps the
/// levels afterwards.
fn salvage_meta(value: &serde_json::Value) -> crate::meta::MetaProfile {
    let mut profile = crate::meta::MetaProfile::default();
    let Some(object) = value.as_object() else {
        return profile;
    };
    if let Some(bank) = object.get("bank").and_then(|b| b.as_u64()) {
        profile.bank = bank.min(u64::from(u32::MAX)) as u32;
    }
    if let Some(levels) = object.get("levels").and_then(|l| l.as_object()) {
        for (key, level) in levels {
            if let Some(n) = level.as_u64() {
                profile.levels.insert(key.clone(), n.min(u64::from(u8::MAX)) as u8);
            }
        }
    }
    profile
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreTable {
    #[serde(default)]
    pub entries: Vec<ScoreEntry>,
    /// Per-level clear flags + bests: what the sector select reads.
    #[serde(default)]
    pub progress: CampaignProgress,
    /// Player's preferred compact minimap size (`normal` | `small` | `hidden`).
    /// Missing or unreadable values fall back to `Normal`.
    #[serde(
        default,
        serialize_with = "serialize_minimap_size",
        deserialize_with = "deserialize_minimap_size"
    )]
    pub minimap_size: MinimapSize,
    /// Player's preference for the in-game cheese mouse cursor (`on` | `off`).
    /// Missing or unreadable values fall back to `On`.
    #[serde(
        default,
        serialize_with = "serialize_cursor_setting",
        deserialize_with = "deserialize_cursor_setting"
    )]
    pub cheese_cursor: CursorSetting,
    /// Music level, 0..=100 in steps of 5. Missing ⇒ `100`, today's balance.
    #[serde(default = "default_volume", deserialize_with = "deserialize_volume")]
    pub music_volume: u8,
    /// Effects level, 0..=100 in steps of 5. Missing ⇒ `100`.
    #[serde(default = "default_volume", deserialize_with = "deserialize_volume")]
    pub effects_volume: u8,
    /// View zoom preference: how much world the playfield shows (`1.0` =
    /// unzoomed, `2.0` = twice the world, `0.5` = half). Missing or unreadable
    /// values load as `1.0`, the view every old build had.
    #[serde(default = "default_zoom", deserialize_with = "deserialize_zoom")]
    pub zoom: f32,
    /// The Library's discoveries and lifetime counters (the LIBRARY screen's
    /// `seen` bitsets and per-kind records). Missing in any file written
    /// before the screen existed, which loads as "nothing discovered yet".
    /// [`crate::game::library::Library`] serialises as its sparse
    /// [`crate::game::library::LibrarySave`] form.
    #[serde(default)]
    pub library: Library,
    /// Control scheme, remappable bindings and the pad tuning. Missing or
    /// unreadable fields load their defaults, so an old file keeps working.
    #[serde(default)]
    pub controls: ControlsFile,
    /// Auto-pick level-ups: take the 1-of-3 card without opening the screen.
    /// Missing in old files (and for a fresh install) ⇒ `false`: the cards are
    /// the default way to level up.
    #[serde(default)]
    pub auto_pick: bool,
    /// **The Graphics page's** render-only preferences (lighting quality, light
    /// effects, 1× smoothing, the foreground layer, the glyph particles and the
    /// camera's shake share). Missing in any file written before the page
    /// existed, which loads today's defaults; `null` for a field a future build
    /// adds is that field's default too.
    #[serde(default)]
    pub graphics: GraphicsFile,
    // A build before batch nine saved a `detail: bool` here (the DETAIL
    // option). There is no option any more — the finest block glyphs the
    // terminal is known to draw are always used — so the field is gone; serde
    // ignores the key a save still carries, and an old file therefore loads
    // and its value is simply not read.
    /// **The nest**: gold banked across runs and one level per permanent
    /// upgrade track (`crate::meta`). Missing in any file written before the
    /// nest existed, which loads as a fresh profile (bank 0, every track 0, so
    /// the racks are 4 + 4 and no bonus applies).
    #[serde(default, deserialize_with = "deserialize_meta")]
    pub meta: crate::meta::MetaProfile,
}

impl Default for ScoreTable {
    /// An empty table at full volume: the balance every build before the
    /// sliders shipped had. (The derive would zero the volume fields and start
    /// the game silent.)
    fn default() -> Self {
        ScoreTable {
            entries: Vec::new(),
            progress: CampaignProgress::default(),
            minimap_size: MinimapSize::default(),
            cheese_cursor: CursorSetting::default(),
            music_volume: default_volume(),
            effects_volume: default_volume(),
            zoom: default_zoom(),
            library: Library::default(),
            controls: ControlsFile::default(),
            auto_pick: false,
            graphics: GraphicsFile::default(),
            meta: crate::meta::MetaProfile::default(),
        }
    }
}

impl ScoreTable {
    /// Where the save file lives, or `None` when this process must not touch
    /// one.
    ///
    /// **The test guard.** In a test build the real data dir is never used:
    /// [`Self::load`] returns the default table and [`Self::save`] writes
    /// nothing, whatever constructor the test called (`App::new` loads and
    /// persists; see its docs). A test that wants a real file sets
    /// `RATRUN_DATA_DIR` to a directory of its own and gets exactly that.
    #[cfg(not(target_arch = "wasm32"))]
    fn store_path() -> Option<std::path::PathBuf> {
        #[cfg(test)]
        {
            if std::env::var_os("RATRUN_DATA_DIR").is_none_or(|dir| dir.is_empty()) {
                return None;
            }
        }
        Some(config::scores_path())
    }

    pub fn load() -> ScoreTable {
        ProfileStorage::read().map_or_else(Self::default, |json| Self::parse(&json))
    }

    /// Is `json` a JSON object (the only shape a save file has)?
    fn is_json_object(json: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(json).is_ok_and(|v| v.is_object())
    }

    /// Parse a persisted table. The file on disk may be hand-edited or written
    /// by an older build, so it is normalized (sorted high→low, trimmed to
    /// `MAX_ENTRIES`) before anything trusts `best()` or the top-five display.
    /// Unparseable JSON yields an empty table; valid JSON with a field this
    /// build cannot read loses **that field only** (see [`Self::salvage`]).
    pub fn parse(json: &str) -> ScoreTable {
        let mut table: ScoreTable = match serde_json::from_str(json) {
            Ok(table) => table,
            Err(_) => ScoreTable::salvage(json),
        };
        table.normalize();
        table
    }

    /// Load what can be loaded from a file whose strict parse failed.
    ///
    /// The strict parse is all-or-nothing: one field of the wrong type (a
    /// `null` time, `"auto_pick": "yes"`, one malformed score entry) used to
    /// drop the whole table, and the next save then erased the nest's bank and
    /// every unlock with it. Here each malformed score entry is dropped on its
    /// own, then each top-level field that cannot be read alone is dropped and
    /// loads its default; everything else survives. Not JSON, or not an
    /// object, is still an empty table.
    fn salvage(json: &str) -> ScoreTable {
        let Ok(serde_json::Value::Object(mut map)) =
            serde_json::from_str::<serde_json::Value>(json)
        else {
            return ScoreTable::default();
        };
        if let Some(serde_json::Value::Array(items)) = map.get_mut("entries") {
            items.retain(|item| serde_json::from_value::<ScoreEntry>(item.clone()).is_ok());
        }
        let keys: Vec<String> = map.keys().cloned().collect();
        for key in keys {
            let Some(value) = map.get(&key).cloned() else {
                continue;
            };
            // Every field has a default, so a one-field object parses exactly
            // when that field does.
            let mut probe = serde_json::Map::new();
            probe.insert(key.clone(), value);
            if serde_json::from_value::<ScoreTable>(serde_json::Value::Object(probe)).is_err() {
                map.remove(&key);
            }
        }
        serde_json::from_value(serde_json::Value::Object(map)).unwrap_or_default()
    }

    /// The table as it would be written to disk (round-trips through [`parse`]).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    fn normalize(&mut self) {
        // A file written before floors were tracked separately carries one
        // record per campaign stage: fold those onto the floors they belonged
        // to (Sewer, Kitchen, Lab) before anything reads a record.
        self.progress.migrate_legacy();
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.score));
        self.entries.truncate(MAX_ENTRIES);
        self.music_volume = audio::snap_percent(self.music_volume);
        self.effects_volume = audio::snap_percent(self.effects_volume);
        self.zoom = crate::render::zoom::snap(self.zoom);
        if !(self.controls.deadzone.is_finite() && self.controls.deadzone > 0.0) {
            self.controls.deadzone = crate::controls::DEFAULT_DEADZONE;
        }
        self.controls.deadzone = self.controls.deadzone.clamp(0.05, 0.9);
        // The Graphics page: clamp the shake share a hand-edited file wrote and
        // drop a word this build cannot read, so the page always shows a value
        // it can cycle.
        self.graphics.normalize();
        // The nest: drop unknown track keys and clamp a hand-edited level into
        // its track's range, so a corrupt save can never over-upgrade a run.
        self.meta.normalize();
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            ProfileStorage::write(&json);
        }
    }

    /// Insert an entry, keep sorted high→low, trim to MAX_ENTRIES.
    /// Returns the entry's 0-based rank, or `None` when it scored too low to
    /// make the table at all.
    pub fn insert(&mut self, mut entry: ScoreEntry) -> Option<usize> {
        // JSON has no NaN: serde writes a non-finite float as `null`, which
        // this struct cannot read back.
        if !entry.time.is_finite() {
            entry.time = 0.0;
        }
        let score = entry.score;
        // Entries that already scored the same stay above the new one, matching
        // the stable ordering used by `normalize`.
        let rank = self.entries.iter().filter(|e| e.score >= score).count();
        self.entries.insert(rank, entry);
        self.entries.truncate(MAX_ENTRIES);
        if rank < MAX_ENTRIES {
            Some(rank)
        } else {
            None
        }
    }

    pub fn best(&self) -> u32 {
        self.entries.first().map(|e| e.score).unwrap_or(0)
    }

    /// Whether `score` beats the table's current best — i.e. whether a run
    /// deserves the "NEW HIGH SCORE" banner. An empty table has best 0, so any
    /// scoring run qualifies; an under-full table does not make every run a
    /// record.
    pub fn is_high_score(&self, score: u32) -> bool {
        score > self.best()
    }

    // ---- Floor progress (sector select) ---------------------------------

    /// Has `theme` been cleared in any run?
    pub fn floor_cleared(&self, theme: Theme) -> bool {
        self.progress.get(theme).is_some_and(|l| l.cleared)
    }

    /// Is `theme` selectable?
    ///
    /// The rule: **a floor unlocks when its campaign stage is reachable.**
    /// Stage one is always open; a later stage is reachable once every earlier
    /// stage has been cleared on at least one of its floors (so clearing the
    /// Laundry or the Outfall opens stage three exactly like clearing the Kitchen does).
    ///
    /// Since batch twelve the Flatbed is the campaign's **fourth** level, and
    /// the campaign ends on it; as a practice floor it is **always open** (the
    /// sector select starts it as a practice run), and it is never cleared
    /// until its own boss dies.
    pub fn floor_unlocked(&self, theme: Theme) -> bool {
        if theme == campaign::getaway_theme() {
            return true;
        }
        let stage = campaign::theme_stage(theme);
        (0..stage).all(|earlier| {
            campaign::Route::options(earlier)
                .iter()
                .any(|floor| self.floor_cleared(*floor))
        })
    }

    /// Recorded bests for `theme`, if it has ever been cleared.
    pub fn floor_best(&self, theme: Theme) -> Option<LevelBestEntry> {
        self.progress.get(theme).filter(|l| l.cleared).copied()
    }

    /// Record a floor completion: flag it cleared, keep the highest floor score
    /// and the fastest time. Floors are independent, so the Laundry's best can
    /// never overwrite the Kitchen's.
    pub fn record_floor(&mut self, theme: Theme, score: u32, time: f32) {
        self.progress.record(theme, score, time);
    }

    // ---- Legacy stage-indexed view --------------------------------------

    /// Has the original route's stage `level` (0 Sewer, 1 Kitchen, 2 Lab,
    /// 3 the Flatbed ride) been cleared?
    ///
    /// Kept for older call sites: a stage is not a floor, so anything the
    /// sector select reads should use [`Self::floor_cleared`].
    pub fn level_cleared(&self, level: usize) -> bool {
        level < LEVEL_COUNT && self.floor_cleared(campaign::level_theme(level))
    }

    /// Is the original route's stage `level` selectable? Out-of-range stages
    /// are never unlocked.
    pub fn level_unlocked(&self, level: usize) -> bool {
        level < LEVEL_COUNT && self.floor_unlocked(campaign::level_theme(level))
    }

    /// Recorded bests for the original route's stage `level`, if cleared.
    pub fn level_best(&self, level: usize) -> Option<LevelBestEntry> {
        if level >= LEVEL_COUNT {
            return None;
        }
        self.floor_best(campaign::level_theme(level))
    }

    /// Record a completion against the original route's stage `level`.
    /// Out-of-range stages are ignored so a corrupt file cannot index past the
    /// campaign.
    pub fn record_level(&mut self, level: usize, score: u32, time: f32) {
        if level >= LEVEL_COUNT {
            return;
        }
        self.record_floor(campaign::level_theme(level), score, time);
    }

    // ---- The nest (`crate::meta`) ---------------------------------------

    /// Wipe the nest: the bank and every permanent upgrade. Scores, floor
    /// progress and the Library are untouched (this is "reset progress" on the
    /// upgrade track, not a new save file).
    pub fn reset_nest(&mut self) {
        self.meta = crate::meta::MetaProfile::default();
    }
}

trait ScoreStorage {
    fn read() -> Option<String>;
    fn write(json: &str);
}

struct ProfileStorage;

#[cfg(not(target_arch = "wasm32"))]
impl ScoreStorage for ProfileStorage {
    fn read() -> Option<String> {
        let path = ScoreTable::store_path()?;
        let json = std::fs::read_to_string(&path).ok()?;
        // Preserve bytes that would otherwise be overwritten by the next save.
        #[cfg(not(test))]
        if !json.trim().is_empty() && !ScoreTable::is_json_object(&json) {
            let _ = std::fs::copy(&path, path.with_extension("json.corrupt"));
        }
        Some(json)
    }

    fn write(json: &str) {
        let Some(path) = ScoreTable::store_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Write beside the save and rename over it. Fall back to a direct
        // write if the platform refuses the rename.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            return;
        }
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::write(&path, json);
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
impl ProfileStorage {
    const KEY: &str = "ratrun.scores.json";

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
impl ScoreStorage for ProfileStorage {
    fn read() -> Option<String> {
        let storage = Self::storage()?;
        let json = storage.get_item(Self::KEY).ok().flatten()?;
        if !json.trim().is_empty() && !ScoreTable::is_json_object(&json) {
            let _ = storage.set_item("ratrun.scores.json.corrupt", &json);
        }
        Some(json)
    }

    fn write(json: &str) {
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(Self::KEY, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(score: u32) -> ScoreEntry {
        ScoreEntry {
            score,
            time: 0.0,
            level: 1,
            map: "Sewer".into(),
            won: false,
            campaign_level: 1,
            campaign_won: false,
        }
    }

    #[test]
    fn legacy_save_loads_and_drops_retired_fields() {
        let json = r#"{
            "entries":[{"score":1200,"time":180.0,"level":7,"map":"Sewer","won":false}],
            "meta":{"bank":250},
            "daily_bests":{"20783":1400,"20784":800},
            "daily_day":20784
        }"#;

        // Unknown fields load directly, without needing the save salvage path.
        let _: ScoreTable = serde_json::from_str(json).unwrap();
        let table = ScoreTable::parse(json);
        assert_eq!(table.best(), 1_200);
        assert_eq!(table.meta.bank, 250);
        let saved: serde_json::Value = serde_json::from_str(&table.to_json()).unwrap();
        let fields = saved.as_object().unwrap();
        assert!(!fields.contains_key("daily_bests"));
        assert!(!fields.contains_key("daily_day"));
    }

    #[test]
    fn sorts_and_trims() {
        let mut t = ScoreTable::default();
        for s in [50, 10, 90, 30, 70, 20, 80, 5, 60, 40, 100] {
            t.insert(e(s));
        }
        assert_eq!(t.entries.len(), MAX_ENTRIES);
        assert_eq!(t.entries[0].score, 100);
        // Sorted descending.
        for w in t.entries.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
        // The lowest (5) should have been trimmed.
        assert!(t.entries.iter().all(|x| x.score != 5));
    }

    #[test]
    fn high_score_detection() {
        let mut t = ScoreTable::default();
        // Any scoring run beats an empty table.
        assert!(t.is_high_score(1));
        for s in 0..MAX_ENTRIES as u32 {
            t.insert(e((s + 1) * 100));
        }
        assert!(t.is_high_score(10_000));
        assert!(!t.is_high_score(1));

        // A table with room left is *not* a free pass: only a new best counts.
        let mut small = ScoreTable::default();
        small.insert(e(900));
        assert!(!small.is_high_score(500));
        assert!(!small.is_high_score(900));
        assert!(small.is_high_score(901));
    }

    #[test]
    fn insert_returns_real_rank() {
        let mut t = ScoreTable::default();
        assert_eq!(t.insert(e(500)), Some(0));
        assert_eq!(t.insert(e(900)), Some(0));
        assert_eq!(t.insert(e(100)), Some(2));

        // Fill the table, then insert a score that cannot make it.
        for s in 0..(MAX_ENTRIES as u32 - 2) {
            t.insert(e(1000 + s));
        }
        assert_eq!(t.entries.len(), MAX_ENTRIES);
        assert_eq!(t.insert(e(50)), None);
        assert_eq!(t.entries.len(), MAX_ENTRIES);
        // 500 is the lowest survivor: 100 was trimmed when the table filled.
        assert_eq!(t.entries[MAX_ENTRIES - 1].score, 500);
    }

    #[test]
    fn one_bad_field_does_not_drop_the_rest_of_the_save() {
        // `auto_pick` has the wrong type and the second entry has no score:
        // both are dropped, and the nest, the unlock and the good entry stay.
        let json = r#"{
            "entries": [
                {"score": 700, "time": 12.0, "level": 4, "map": "Sewer", "won": false},
                {"time": 1.0, "level": 1, "map": "Sewer", "won": false},
                {"score": 300, "time": null, "level": 2, "map": "Sewer", "won": false}
            ],
            "progress": {"floors": {"Sewer": {"cleared": true, "score": 900, "time": 600.0}}},
            "auto_pick": "yes",
            "meta": {"bank": 4321, "levels": {"move_speed": 3}}
        }"#;
        let t = ScoreTable::parse(json);
        assert_eq!(t.entries.len(), 1, "only the well-formed entry survives");
        assert_eq!(t.best(), 700);
        assert!(!t.auto_pick, "the unreadable field loads its default");
        assert_eq!(t.meta.bank, 4321, "the bank survives a bad sibling field");
        assert_eq!(t.meta.level(crate::meta::Track::MoveSpeed), 3);
        assert!(t.floor_cleared(Theme::Sewer), "the unlock survives");
        assert_eq!(t.music_volume, 100);
    }

    #[test]
    fn graphics_before_v2_reset_smoothing_and_enable_foreground() {
        use crate::render::subcell::Smooth;
        let table = ScoreTable::parse(
            r#"{"graphics":{"lighting":"coarse","smooth":"xbr","foreground":false,
                 "fx":false,"glyphfx":false,"shake_scale":0.5}}"#,
        );
        let graphics = table.graphics;
        assert_eq!(graphics.graphics_version, 3);
        assert_eq!(graphics.smooth(), Smooth::Off);
        assert_eq!(graphics.lighting.as_deref(), Some("coarse"));
        assert!(graphics.foreground && !graphics.fx && !graphics.glyphfx);
        assert_eq!(graphics.shake(), 0.5);

        let table = ScoreTable::parse(r#"{"graphics":{"graphics_version":1,"smooth":"wedge"}}"#);
        assert_eq!(table.graphics.graphics_version, 3);
        assert_eq!(table.graphics.smooth(), Smooth::Off);
    }

    #[test]
    fn version_two_graphics_saves_enable_foreground_and_keep_smoothing() {
        use crate::render::subcell::Smooth;
        let table = ScoreTable::parse(
            r#"{"graphics":{"graphics_version":2,"smooth":"crisp","foreground":false}}"#,
        );
        assert_eq!(table.graphics.graphics_version, 3);
        assert_eq!(table.graphics.smooth(), Smooth::XbrCrisp);
        assert!(table.graphics.foreground);

        // Saves naming a retired smoothing mode normalize to Off.
        for retired in ["on", "epx", "strong", "wedge"] {
            let table = ScoreTable::parse(&format!(
                r#"{{"graphics":{{"graphics_version":3,"smooth":"{retired}"}}}}"#
            ));
            assert_eq!(table.graphics.smooth(), Smooth::Off, "{retired}");
            assert_eq!(table.graphics.smooth, "off", "{retired}");
        }
    }

    #[test]
    fn version_three_graphics_saves_keep_chosen_smoothing_and_foreground() {
        use crate::render::subcell::Smooth;
        let mut table = ScoreTable::default();
        table.graphics.set_smooth(Smooth::Xbr);
        table.graphics.foreground = false;
        let json = table.to_json();
        let saved: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(saved["graphics"]["graphics_version"].as_u64(), Some(3));
        let loaded = ScoreTable::parse(&json);
        assert_eq!(loaded.graphics.smooth(), Smooth::Xbr);
        assert!(!loaded.graphics.foreground);
    }

    #[test]
    fn a_broken_progress_block_costs_only_the_progress() {
        let json = r#"{"progress": {"floors": {"Sewer": {"time": null}}},
                       "meta": {"bank": 55}}"#;
        let t = ScoreTable::parse(json);
        assert_eq!(t.meta.bank, 55);
        assert!(!t.floor_cleared(Theme::Sewer));
    }

    #[test]
    fn an_out_of_range_nest_number_saturates_instead_of_wiping_the_nest() {
        let json = r#"{"meta": {"bank": 99999999999, "levels": {"move_speed": 900, "regen": -1, "armor": 2}}}"#;
        let t = ScoreTable::parse(json);
        assert_eq!(t.meta.bank, u32::MAX);
        let track = crate::meta::Track::MoveSpeed;
        assert_eq!(t.meta.level(track), track.max_level());
        assert_eq!(t.meta.level(crate::meta::Track::Regen), 0);
        assert_eq!(t.meta.level(crate::meta::Track::Armor), 2);
    }

    #[test]
    fn a_non_finite_time_never_reaches_the_file() {
        let mut t = ScoreTable::default();
        let mut entry = e(10);
        entry.time = f32::NAN;
        t.insert(entry);
        t.record_floor(Theme::Sewer, 10, f32::INFINITY);
        let back = ScoreTable::parse(&t.to_json());
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].time, 0.0);
        assert!(back.floor_cleared(Theme::Sewer));
    }

    /// No test may read or write the player's save, whichever constructor it
    /// used: with no `RATRUN_DATA_DIR` there is no store at all, and with one
    /// (another test may be setting it concurrently) the store is that
    /// directory, never the platform data dir.
    #[test]
    fn a_test_build_never_resolves_the_real_save_path() {
        match ScoreTable::store_path() {
            None => {}
            Some(path) => {
                if let Some(real) = dirs::data_dir() {
                    assert!(
                        !path.starts_with(real.join("ratrun")),
                        "a test resolved the real save file: {path:?}"
                    );
                }
            }
        }
        // Both are no-ops without a store, and must not panic.
        let table = ScoreTable::load();
        table.save();
    }

    #[test]
    fn only_a_json_object_counts_as_a_save_file() {
        assert!(ScoreTable::is_json_object("{}"));
        assert!(!ScoreTable::is_json_object("{ not json"));
        assert!(!ScoreTable::is_json_object("[]"));
        assert!(!ScoreTable::is_json_object(""));
    }

    #[test]
    fn parse_normalizes_unsorted_overlong_table() {
        // 14 entries in a scrambled order, as a hand-edited file could hold.
        let order: Vec<u32> = (0..(MAX_ENTRIES as u32 + 4))
            .map(|i| (i * 137 + 11) % 1000)
            .collect();
        let body: Vec<String> = order
            .iter()
            .map(|s| format!(r#"{{"score":{s},"time":0.0,"level":1,"map":"Sewer","won":false}}"#))
            .collect();
        let json = format!(r#"{{"entries":[{}]}}"#, body.join(","));

        let t = ScoreTable::parse(&json);
        assert_eq!(t.entries.len(), MAX_ENTRIES, "should be trimmed");
        assert_eq!(t.best(), *order.iter().max().unwrap());
        for w in t.entries.windows(2) {
            assert!(w[0].score >= w[1].score, "should be sorted descending");
        }

        // Junk on disk degrades to an empty table instead of panicking.
        assert!(ScoreTable::parse("{ not json").entries.is_empty());
    }

    /// A file written before the campaign existed has neither campaign field:
    /// every entry loads as "reached level 1, campaign not won", and nothing is
    /// unlocked by it.
    #[test]
    fn old_format_entries_load_as_level_one_and_not_won() {
        let json = r#"{"entries":[
            {"score":900,"time":300.0,"level":9,"map":"Sewer","won":true},
            {"score":400,"time":120.0,"level":4,"map":"Lab","won":false}
        ]}"#;
        let t = ScoreTable::parse(json);
        assert_eq!(t.entries.len(), 2);
        for entry in &t.entries {
            assert_eq!(entry.campaign_level, 1, "old entries reached level 1");
            assert!(!entry.campaign_won, "no old entry won a campaign");
        }
        assert_eq!(t.entries[0].score, 900);
        assert!(t.floor_best(Theme::Sewer).is_none());
        assert!(t.floor_unlocked(Theme::Sewer), "stage one is always open");
        assert!(!t.floor_unlocked(Theme::Kitchen), "an old file unlocks nothing later");
    }

    #[test]
    fn floor_progress_unlocks_and_round_trips() {
        let mut t = ScoreTable::default();
        assert!(t.floor_unlocked(Theme::Sewer));
        assert!(!t.floor_unlocked(Theme::Kitchen));
        assert!(!t.floor_unlocked(Theme::Laundry));
        assert!(!t.floor_unlocked(Theme::Outfall));
        assert!(!t.floor_unlocked(Theme::Lab));
        assert!(!t.floor_unlocked(Theme::Printworks));
        assert!(
            t.floor_unlocked(Theme::Trailer),
            "the getaway ride is a bonus floor and is always open"
        );

        t.record_floor(Theme::Sewer, 6420, 252.0);
        assert!(t.floor_cleared(Theme::Sewer));
        assert!(
            t.floor_unlocked(Theme::Kitchen) && t.floor_unlocked(Theme::Laundry),
            "clearing stage one opens the Kitchen and the Laundry"
        );
        assert!(t.floor_unlocked(Theme::Outfall));
        assert!(!t.floor_unlocked(Theme::Lab));
        assert_eq!(
            t.floor_best(Theme::Sewer),
            Some(LevelBestEntry {
                cleared: true,
                score: 6420,
                time: 252.0
            })
        );

        // Bests keep the highest score and the fastest time.
        t.record_floor(Theme::Sewer, 5000, 260.0);
        t.record_floor(Theme::Sewer, 8000, 240.0);
        let best = t.floor_best(Theme::Sewer).unwrap();
        assert_eq!(best.score, 8000, "a lower score must not replace the best");
        assert_eq!(best.time, 240.0, "a slower time must not replace the best");

        // Any floor of the next stage opens the stage after it.
        t.record_floor(Theme::Laundry, 4180, 198.0);
        assert!(
            t.floor_unlocked(Theme::Lab) && t.floor_unlocked(Theme::Printworks),
            "clearing the Laundry opens stage three"
        );

        // Progress survives a save/load round trip, including the clear flags.
        let back = ScoreTable::parse(&t.to_json());
        assert!(back.floor_unlocked(Theme::Lab));
        assert_eq!(back.floor_best(Theme::Sewer), t.floor_best(Theme::Sewer));
        assert_eq!(back.floor_best(Theme::Laundry), t.floor_best(Theme::Laundry));

        // Out-of-range stage indices are ignored rather than panicking.
        t.record_level(9, 1, 1.0);
        assert!(!t.level_unlocked(9));
    }

    /// Floors on the same campaign stage keep **separate** bests: clearing the
    /// Laundry must not overwrite the Kitchen's record (the bug the old
    /// stage-keyed array had).
    #[test]
    fn same_stage_floors_do_not_share_bests() {
        let mut t = ScoreTable::default();
        t.record_floor(Theme::Kitchen, 4000, 210.0);
        t.record_floor(Theme::Laundry, 7000, 190.0);
        assert_eq!(t.floor_best(Theme::Kitchen).map(|b| b.score), Some(4000));
        assert_eq!(t.floor_best(Theme::Laundry).map(|b| b.score), Some(7000));
        assert_eq!(t.floor_best(Theme::Kitchen).map(|b| b.time), Some(210.0));
        assert_eq!(t.floor_best(Theme::Laundry).map(|b| b.time), Some(190.0));
        // The Lab and the Printworks are the other pair of stage-mates.
        t.record_floor(Theme::Printworks, 9000, 180.0);
        assert!(t.floor_best(Theme::Lab).is_none());
        assert_eq!(t.floor_best(Theme::Printworks).map(|b| b.score), Some(9000));
    }

    /// A pre-floors score file stores one record per campaign **stage**. It
    /// loads as three floor records (Sewer, Kitchen, Lab) and unlocks the next
    /// stage exactly as the old build did.
    #[test]
    fn stage_keyed_score_files_migrate_to_floors() {
        let json = r#"{"entries":[],"progress":{"levels":[
            {"cleared":true,"score":6420,"time":252.0},
            {"cleared":true,"score":4180,"time":198.0},
            {"cleared":false,"score":0,"time":0.0}
        ]}}"#;
        let t = ScoreTable::parse(json);
        assert_eq!(t.floor_best(Theme::Sewer).map(|b| b.score), Some(6420));
        assert_eq!(t.floor_best(Theme::Kitchen).map(|b| b.score), Some(4180));
        assert!(t.floor_best(Theme::Lab).is_none(), "stage three was not cleared");
        assert_eq!(
            t.floor_best(Theme::Laundry),
            None,
            "the old file had no Laundry record, and none is invented"
        );
        assert!(t.floor_unlocked(Theme::Lab) && t.floor_unlocked(Theme::Printworks));

        // The next save writes floors only; reloading it changes nothing.
        let json = t.to_json();
        assert!(
            !json.contains("\"levels\""),
            "a migrated file must not write the legacy array back: {json}"
        );
        let back = ScoreTable::parse(&json);
        assert_eq!(back.floor_best(Theme::Sewer), t.floor_best(Theme::Sewer));
        assert_eq!(back.floor_best(Theme::Kitchen), t.floor_best(Theme::Kitchen));
    }

    /// The legacy stage accessors still mean the original route's floors.
    #[test]
    fn legacy_stage_accessors_read_the_original_route() {
        let mut t = ScoreTable::default();
        t.record_level(0, 1000, 250.0);
        t.record_level(1, 2000, 200.0);
        assert_eq!(t.level_best(0).map(|b| b.score), Some(1000));
        assert_eq!(t.level_best(1).map(|b| b.score), Some(2000));
        assert!(t.floor_unlocked(Theme::Lab));
        assert!(t.level_unlocked(3), "the ride is always open for practice");
        assert!(!t.level_cleared(3));
        assert!(t.level_best(3).is_none());
    }

    /// A file written by the **three-level** build (its `levels` array holds
    /// three records) still loads, and a campaign that was complete at three
    /// levels reaches the Flatbed: its card is open and **not cleared**.
    #[test]
    fn a_three_level_save_loads_and_reaches_the_flatbed_open() {
        // The old campaign was won: Sewer, an S2 floor and an S3 floor cleared.
        let json = r#"{"entries":[],"progress":{"levels":[
            {"cleared":true,"score":6420,"time":252.0},
            {"cleared":true,"score":4180,"time":198.0},
            {"cleared":true,"score":5210,"time":264.0}
        ]}}"#;
        let t = ScoreTable::parse(json);
        assert_eq!(t.floor_best(Theme::Lab).map(|b| b.score), Some(5210));
        // Stage four is now reachable and not yet cleared.
        assert!(
            t.floor_unlocked(Theme::Trailer),
            "a complete three-level campaign reaches the ride"
        );
        assert!(!t.floor_cleared(Theme::Trailer));
        assert_eq!(t.floor_best(Theme::Trailer), None, "nothing is invented");
        // The ride is a level index of its own, and clearing it records it.
        let mut t = t;
        t.record_level(crate::game::campaign::LEVEL_COUNT - 1, 7000, 300.0);
        assert!(t.floor_cleared(Theme::Trailer));
        assert_eq!(t.level_best(3).map(|b| b.score), Some(7000));
        assert_eq!(t.floor_best(Theme::Lab).map(|b| b.score), Some(5210));
        // The practice ride is always open, even on an incomplete old save.
        let partial = ScoreTable::parse(
            r#"{"entries":[],"progress":{"levels":[
                {"cleared":true,"score":6420,"time":252.0},
                {"cleared":true,"score":4180,"time":198.0},
                {"cleared":false,"score":0,"time":0.0}
            ]}}"#,
        );
        assert!(partial.floor_unlocked(Theme::Trailer));
    }

    /// A cleared floor stays unlocked even when its score entry has been pushed
    /// out of the top ten by later runs.
    #[test]
    fn clearing_survives_the_top_ten_rolling_over() {
        let mut t = ScoreTable::default();
        // The route needs both earlier stages cleared before stage three opens.
        t.record_level(0, 100, 210.0);
        t.record_level(1, 100, 200.0);
        for s in 0..(MAX_ENTRIES as u32 + 3) {
            t.insert(e(1000 + s));
        }
        assert_eq!(t.entries.len(), MAX_ENTRIES);
        assert!(t
            .entries
            .iter()
            .all(|x| x.campaign_level == 1 && !x.campaign_won));
        assert!(t.level_unlocked(2), "the clear flag is independent of the table");
    }

    #[test]
    fn roundtrip_json() {
        let mut t = ScoreTable::default();
        t.insert(e(123));
        let json = serde_json::to_string(&t).unwrap();
        let back: ScoreTable = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entries[0].score, 123);
        assert_eq!(back.entries[0].campaign_level, 1);
        assert_eq!(back.minimap_size, MinimapSize::Normal);
    }

    #[test]
    fn minimap_size_persists_and_falls_back_to_normal_when_missing_or_invalid() {
        // Default / missing field -> Normal.
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(missing.minimap_size, MinimapSize::Normal);

        // Unreadable / invalid field values fall back to Normal without dropping scores.
        for bad in [
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"minimap_size":"huge"}"#,
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"minimap_size":42}"#,
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"minimap_size":null}"#,
        ] {
            let parsed = ScoreTable::parse(bad);
            assert_eq!(parsed.minimap_size, MinimapSize::Normal);
            assert_eq!(parsed.best(), 500);
        }

        // Completely corrupt JSON falls back to default (Normal).
        assert_eq!(ScoreTable::parse("{ broken").minimap_size, MinimapSize::Normal);

        // Valid values round-trip cleanly.
        for size in [MinimapSize::Normal, MinimapSize::Small, MinimapSize::Hidden] {
            let t = ScoreTable {
                minimap_size: size,
                ..Default::default()
            };
            let reloaded = ScoreTable::parse(&t.to_json());
            assert_eq!(reloaded.minimap_size, size);
        }
    }

    #[test]
    fn volumes_persist_and_old_files_load_at_full() {
        // A file written before the sliders existed has no volume fields.
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(missing.music_volume, 100);
        assert_eq!(missing.effects_volume, 100);
        assert_eq!(missing.best(), 0);

        // Hand-edited values are snapped to the 5% grid and clamped, without
        // dropping the scores beside them.
        let edited = ScoreTable::parse(
            r#"{"entries":[{"score":700,"time":10.0,"level":2,"map":"Sewer","won":false}],
                "music_volume":42,"effects_volume":"17"}"#,
        );
        assert_eq!(edited.music_volume, 40);
        assert_eq!(edited.effects_volume, 15);
        assert_eq!(edited.best(), 700);
        for bad in [
            r#"{"music_volume":400,"effects_volume":-1}"#,
            r#"{"music_volume":null,"effects_volume":"loud"}"#,
            r#"{"music_volume":{},"effects_volume":true}"#,
        ] {
            let parsed = ScoreTable::parse(bad);
            assert_eq!(parsed.music_volume, 100, "bad value falls back to 100");
            assert_eq!(parsed.effects_volume, 100, "bad value falls back to 100");
        }

        // Valid settings survive a save/load round trip next to the scores.
        let mut t = ScoreTable::default();
        t.insert(e(500));
        t.music_volume = 35;
        t.effects_volume = 80;
        let back = ScoreTable::parse(&t.to_json());
        assert_eq!((back.music_volume, back.effects_volume), (35, 80));
        assert_eq!(back.best(), 500);
    }

    #[test]
    fn auto_pick_persists_and_old_files_load_it_off() {
        // Old files (and a fresh install) have no auto-pick field: the cards
        // stay the default way to level up.
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert!(!missing.auto_pick, "old files default to manual cards");
        assert!(!ScoreTable::default().auto_pick);

        // Unreadable values fall back to off without dropping the scores.
        for bad in [
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"auto_pick":"yes"}"#,
            r#"{"entries":[],"auto_pick":null}"#,
            r#"{"entries":[],"auto_pick":7}"#,
        ] {
            let parsed = ScoreTable::parse(bad);
            assert!(!parsed.auto_pick, "unreadable auto_pick loads as off");
        }

        // Both states round-trip next to the scores.
        for on in [false, true] {
            let mut t = ScoreTable::default();
            t.insert(e(500));
            t.auto_pick = on;
            let back = ScoreTable::parse(&t.to_json());
            assert_eq!(back.auto_pick, on);
            assert_eq!(back.best(), 500);
        }
    }

    #[test]
    fn zoom_persists_and_old_files_load_unzoomed() {
        // A file written before the wheel existed has no zoom field.
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(missing.zoom, 1.0, "old files play at the reference view");
        assert_eq!(ScoreTable::default().zoom, crate::render::zoom::DEFAULT);

        // Every rung round-trips; hand-edited values are clamped onto the
        // ladder without dropping the scores beside them.
        for zoom in crate::render::zoom::STEPS {
            let t = ScoreTable {
                zoom,
                ..Default::default()
            };
            assert_eq!(ScoreTable::parse(&t.to_json()).zoom, zoom);
        }
        let edited = ScoreTable::parse(
            r#"{"entries":[{"score":700,"time":10.0,"level":2,"map":"Sewer","won":false}],
                "zoom":1.6}"#,
        );
        assert_eq!(edited.zoom, 1.5, "1.6 snaps to the nearest rung");
        assert_eq!(edited.best(), 700);
        for (raw, want) in [
            ("0.1", 0.5),
            ("99", 2.0),
            ("\"0.75\"", 0.75),
            ("-1", 0.5),
            ("0.9", 1.0),
            ("null", 1.0),
            ("true", 1.0),
            ("{}", 1.0),
        ] {
            let json = format!(r#"{{"zoom":{raw}}}"#);
            assert_eq!(ScoreTable::parse(&json).zoom, want, "zoom = {raw}");
        }
    }

    #[test]
    fn cheese_cursor_persists_and_falls_back_to_on_when_missing_or_invalid() {
        // Default / missing field -> On.
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(missing.cheese_cursor, CursorSetting::On);

        // Hand-edited booleans also parse cleanly.
        assert_eq!(
            ScoreTable::parse(r#"{"cheese_cursor":false}"#).cheese_cursor,
            CursorSetting::Off
        );
        assert_eq!(
            ScoreTable::parse(r#"{"cheese_cursor":true}"#).cheese_cursor,
            CursorSetting::On
        );

        // Unreadable / invalid field values fall back to On without dropping scores.
        for bad in [
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"cheese_cursor":"gouda"}"#,
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"cheese_cursor":42}"#,
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"cheese_cursor":null}"#,
        ] {
            let parsed = ScoreTable::parse(bad);
            assert_eq!(parsed.cheese_cursor, CursorSetting::On);
            assert_eq!(parsed.best(), 500);
        }

        // Valid values round-trip cleanly.
        for setting in [CursorSetting::On, CursorSetting::Off] {
            let t = ScoreTable {
                cheese_cursor: setting,
                ..Default::default()
            };
            let reloaded = ScoreTable::parse(&t.to_json());
            assert_eq!(reloaded.cheese_cursor, setting);
        }
    }

    /// The nest rides in the same save file: a file written before it existed
    /// loads as a fresh profile, a saved nest round-trips next to the scores,
    /// and a hand-edited nest can never over-upgrade a run or drop the table.
    #[test]
    fn the_nest_persists_and_old_files_load_as_a_fresh_profile() {
        use crate::meta::{MetaProfile, RunModifiers, Track};

        // A file written before the nest has no `meta` field at all.
        let old = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(old.meta, MetaProfile::default());
        assert_eq!(old.meta.bank, 0);
        assert_eq!(old.meta.total_levels(), 0);
        assert_eq!(RunModifiers::from_profile(&old.meta), RunModifiers::NONE);

        // A saved nest round-trips, and the scores beside it survive.
        let mut t = ScoreTable::default();
        t.insert(e(700));
        t.meta.bank = 321;
        t.meta.set_level(Track::WeaponSlots, 2);
        t.meta.set_level(Track::MoveSpeed, 4);
        let back = ScoreTable::parse(&t.to_json());
        assert_eq!(back.meta.bank, 321);
        assert_eq!(back.meta.level(Track::WeaponSlots), 2);
        assert_eq!(back.meta.level(Track::MoveSpeed), 4);
        assert_eq!(back.best(), 700);
        assert_eq!(
            RunModifiers::from_profile(&back.meta).caps,
            crate::game::loadout::SlotCaps::new(6, 4),
            "two weapon-slot purchases are a 6 + 4 rack"
        );

        // A hand-edited nest is clamped to the tracks it names; unknown keys are
        // dropped and the scores are kept.
        let edited = ScoreTable::parse(
            r#"{"entries":[{"score":900,"time":10.0,"level":2,"map":"Sewer","won":false}],
                "meta":{"bank":42,"levels":{"damage":99,"bogus":3,"move_speed":0}}}"#,
        );
        assert_eq!(edited.meta.bank, 42);
        assert_eq!(edited.meta.level(Track::Damage), Track::Damage.max_level());
        assert_eq!(edited.meta.level(Track::MoveSpeed), 0);
        assert_eq!(edited.meta.levels.len(), 1, "unknown/zero keys are dropped");
        assert_eq!(edited.best(), 900);

        // A junk `meta` value loads as a fresh nest instead of failing the whole
        // table (the same tolerance the volume and zoom fields have).
        for bad in [r#""lots""#, "[1,2]", "null", "7"] {
            let json = format!(
                r#"{{"entries":[{{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}}],"meta":{bad}}}"#
            );
            let parsed = ScoreTable::parse(&json);
            assert_eq!(parsed.meta, MetaProfile::default(), "meta = {bad}");
            assert_eq!(parsed.best(), 500, "a bad nest must not drop the scores");
        }

        // "Reset progress" wipes the nest and keeps everything else.
        let mut wiped = ScoreTable::default();
        wiped.record_floor(Theme::Sewer, 4100, 240.0);
        wiped.insert(e(600));
        wiped.meta.bank = 90;
        wiped.meta.set_level(Track::Revive, 1);
        wiped.reset_nest();
        assert_eq!(wiped.meta, MetaProfile::default());
        assert_eq!(wiped.best(), 600);
        assert!(wiped.floor_cleared(Theme::Sewer), "the sector stays cleared");
        assert_eq!(
            ScoreTable::parse(&wiped.to_json()).meta,
            MetaProfile::default()
        );
    }

    #[test]
    fn old_lighting_fields_load_without_dropping_scores() {
        for value in [r#""off""#, r#""soft""#, r#""full""#, "42", "null"] {
            let json = format!(
                r#"{{"entries":[{{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}}],"lighting":{value}}}"#
            );
            let parsed = ScoreTable::parse(&json);
            assert_eq!(parsed.best(), 500);
            // The pre-Graphics-page top-level field is dropped, not adopted.
            assert!(parsed.graphics.lighting.is_none());
        }
    }
}
