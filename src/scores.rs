//! High-score table persisted as JSON in the user data dir.
//!
//! The table holds two things:
//!
//! * `entries` — one record per finished run: the **cumulative** campaign
//!   score, the campaign level the run reached (1..=3) and whether the run won
//!   the campaign. Files written by an older build have no campaign fields;
//!   they load as "reached level 1, campaign not won" (see
//!   [`ScoreEntry::campaign_level`]).
//! * `progress` — the per-level bests and clear flags behind the title's
//!   sector-select. A level unlocks once the previous one has been cleared in
//!   any run, so the flags must persist independently of the top-ten entries
//!   (a strong table can drop an old entry, but it never relocks a level).

use serde::{Deserialize, Serialize};

use crate::audio;
use crate::config;
use crate::controls::{Controls, SavedControls, SavedScheme};
use crate::game::campaign::LEVEL_COUNT;
use crate::render::cursor::CursorSetting;
use crate::render::lighting::LightingMode;
use crate::render::minimap::MinimapSize;

pub const MAX_ENTRIES: usize = 10;

/// The campaign level an old score file's entries belong to.
fn default_campaign_level() -> u8 {
    1
}

fn default_level_bests() -> [LevelBestEntry; LEVEL_COUNT] {
    [LevelBestEntry::default(); LEVEL_COUNT]
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

/// Best results recorded for one campaign level (level-select cards).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LevelBestEntry {
    /// Has this level been completed in any run?
    #[serde(default)]
    pub cleared: bool,
    /// Highest score earned on this level alone.
    #[serde(default)]
    pub score: u32,
    /// Fastest completion of this level, in seconds (`0.0` = none yet).
    #[serde(default)]
    pub time: f32,
}

/// Persisted per-level progress: clear flags + best results.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CampaignProgress {
    #[serde(default = "default_level_bests")]
    pub levels: [LevelBestEntry; LEVEL_COUNT],
}

impl Default for CampaignProgress {
    fn default() -> Self {
        CampaignProgress {
            levels: default_level_bests(),
        }
    }
}

impl CampaignProgress {
    fn get(&self, level: usize) -> Option<&LevelBestEntry> {
        self.levels.get(level)
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

fn serialize_lighting_mode<S: serde::Serializer>(
    mode: &Option<LightingMode>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match mode {
        Some(m) => serializer.serialize_str(m.as_str()),
        None => serializer.serialize_none(),
    }
}

fn deserialize_lighting_mode<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<LightingMode>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().and_then(LightingMode::parse_setting))
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
    /// Player's preferred lighting mode (`full` | `soft` | `off`).
    /// `None` (missing or unreadable in old files) defers to the terminal colour depth.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_lighting_mode",
        deserialize_with = "deserialize_lighting_mode"
    )]
    pub lighting: Option<LightingMode>,
    /// View zoom preference: how much world the playfield shows (`1.0` =
    /// unzoomed, `2.0` = twice the world, `0.5` = half). Missing or unreadable
    /// values load as `1.0`, the view every old build had.
    #[serde(default = "default_zoom", deserialize_with = "deserialize_zoom")]
    pub zoom: f32,
    /// Control scheme, remappable bindings and the pad tuning. Missing or
    /// unreadable fields load their defaults, so an old file keeps working.
    #[serde(default)]
    pub controls: ControlsFile,
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
            lighting: None,
            zoom: default_zoom(),
            controls: ControlsFile::default(),
        }
    }
}

impl ScoreTable {
    pub fn load() -> ScoreTable {
        match std::fs::read_to_string(config::scores_path()) {
            Ok(s) => ScoreTable::parse(&s),
            Err(_) => ScoreTable::default(),
        }
    }

    /// Parse a persisted table. The file on disk may be hand-edited or written
    /// by an older build, so it is normalized (sorted high→low, trimmed to
    /// `MAX_ENTRIES`) before anything trusts `best()` or the top-five display.
    /// Unparseable JSON yields an empty table.
    pub fn parse(json: &str) -> ScoreTable {
        let mut table: ScoreTable = serde_json::from_str(json).unwrap_or_default();
        table.normalize();
        table
    }

    /// The table as it would be written to disk (round-trips through [`parse`]).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    fn normalize(&mut self) {
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.score));
        self.entries.truncate(MAX_ENTRIES);
        self.music_volume = audio::snap_percent(self.music_volume);
        self.effects_volume = audio::snap_percent(self.effects_volume);
        self.zoom = crate::render::zoom::snap(self.zoom);
        if !(self.controls.deadzone.is_finite() && self.controls.deadzone > 0.0) {
            self.controls.deadzone = crate::controls::DEFAULT_DEADZONE;
        }
        self.controls.deadzone = self.controls.deadzone.clamp(0.05, 0.9);
    }

    pub fn save(&self) {
        let path = config::scores_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Insert an entry, keep sorted high→low, trim to MAX_ENTRIES.
    /// Returns the entry's 0-based rank, or `None` when it scored too low to
    /// make the table at all.
    pub fn insert(&mut self, entry: ScoreEntry) -> Option<usize> {
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

    // ---- Campaign progress (sector select) ------------------------------

    /// Has campaign level `level` (0-based) been cleared in any run?
    pub fn level_cleared(&self, level: usize) -> bool {
        self.progress.get(level).is_some_and(|l| l.cleared)
    }

    /// Is level `level` (0-based) selectable? Level 1 is always open; later
    /// levels unlock once the previous one has been cleared in any run.
    pub fn level_unlocked(&self, level: usize) -> bool {
        if level == 0 {
            return true;
        }
        level < LEVEL_COUNT && self.level_cleared(level - 1)
    }

    /// Recorded bests for a level, if it has ever been cleared.
    pub fn level_best(&self, level: usize) -> Option<LevelBestEntry> {
        self.progress
            .get(level)
            .filter(|l| l.cleared)
            .copied()
    }

    /// Record a level completion: flag it cleared, keep the highest level score
    /// and the fastest time. Out-of-range levels are ignored so a corrupt file
    /// cannot index past the campaign.
    pub fn record_level(&mut self, level: usize, score: u32, time: f32) {
        let Some(entry) = self.progress.levels.get_mut(level) else {
            return;
        };
        entry.cleared = true;
        if score > entry.score {
            entry.score = score;
        }
        if time > 0.0 && (entry.time <= 0.0 || time < entry.time) {
            entry.time = time;
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
        assert_eq!(t.progress.levels[0], LevelBestEntry::default());
        assert!(t.level_unlocked(0), "level 1 is always open");
        assert!(!t.level_unlocked(1), "an old file unlocks nothing later");
        assert!(t.level_best(0).is_none());
    }

    #[test]
    fn level_progress_unlocks_and_round_trips() {
        let mut t = ScoreTable::default();
        assert!(t.level_unlocked(0));
        assert!(!t.level_unlocked(1));
        assert!(!t.level_unlocked(2));

        t.record_level(0, 6420, 252.0);
        assert!(t.level_cleared(0));
        assert!(t.level_unlocked(1), "clearing level 1 unlocks level 2");
        assert!(!t.level_unlocked(2));
        assert_eq!(
            t.level_best(0),
            Some(LevelBestEntry {
                cleared: true,
                score: 6420,
                time: 252.0
            })
        );

        // Bests keep the highest score and the fastest time.
        t.record_level(0, 5000, 260.0);
        t.record_level(0, 8000, 240.0);
        let best = t.level_best(0).unwrap();
        assert_eq!(best.score, 8000, "a lower score must not replace the best");
        assert_eq!(best.time, 240.0, "a slower time must not replace the best");

        t.record_level(1, 4180, 198.0);
        assert!(t.level_unlocked(2));

        // Progress survives a save/load round trip, including the clear flags.
        let back = ScoreTable::parse(&t.to_json());
        assert!(back.level_unlocked(2));
        assert_eq!(back.level_best(0), t.level_best(0));
        assert_eq!(back.level_best(1), t.level_best(1));

        // Out-of-range levels are ignored rather than panicking.
        t.record_level(9, 1, 1.0);
        assert!(!t.level_unlocked(9));
    }

    /// A cleared level stays unlocked even when its score entry has been pushed
    /// out of the top ten by later runs.
    #[test]
    fn clearing_survives_the_top_ten_rolling_over() {
        let mut t = ScoreTable::default();
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

    #[test]
    fn lighting_persists_and_old_files_load_as_auto_without_dropping_scores() {
        // An old file written before lighting existed loads as `None` (auto by color depth).
        let missing = ScoreTable::parse(r#"{"entries":[]}"#);
        assert_eq!(missing.lighting, None);

        // Invalid values fall back to `None` without losing high scores.
        for bad in [
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"lighting":"blinding"}"#,
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"lighting":42}"#,
            r#"{"entries":[{"score":500,"time":10.0,"level":2,"map":"Sewer","won":false}],"lighting":null}"#,
        ] {
            let parsed = ScoreTable::parse(bad);
            assert_eq!(parsed.lighting, None);
            assert_eq!(parsed.best(), 500);
        }

        // Valid values round-trip cleanly.
        for mode in [LightingMode::Full, LightingMode::Soft, LightingMode::Off] {
            let t = ScoreTable {
                lighting: Some(mode),
                ..Default::default()
            };
            let reloaded = ScoreTable::parse(&t.to_json());
            assert_eq!(reloaded.lighting, Some(mode));
        }
    }
}
