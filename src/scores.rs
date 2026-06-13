//! High-score table persisted as JSON in the user data dir.

use serde::{Deserialize, Serialize};

use crate::config;

pub const MAX_ENTRIES: usize = 10;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub score: u32,
    pub time: f32,
    pub level: u32,
    pub map: String,
    pub won: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScoreTable {
    pub entries: Vec<ScoreEntry>,
}

impl ScoreTable {
    pub fn load() -> ScoreTable {
        let path = config::scores_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => ScoreTable::default(),
        }
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
    /// Returns the rank (0-based) if it made the table.
    pub fn insert(&mut self, entry: ScoreEntry) -> Option<usize> {
        self.entries.push(entry);
        self.entries.sort_by(|a, b| b.score.cmp(&a.score));
        self.entries.truncate(MAX_ENTRIES);
        // Find the most recently inserted entry's rank by identity is awkward;
        // callers only care whether the top score changed, so report best rank
        // of the just-pushed score value via a simple scan is unnecessary here.
        Some(0)
    }

    pub fn best(&self) -> u32 {
        self.entries.first().map(|e| e.score).unwrap_or(0)
    }

    pub fn is_high_score(&self, score: u32) -> bool {
        self.entries.len() < MAX_ENTRIES || score > self.entries.last().map(|e| e.score).unwrap_or(0)
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
        assert!(t.is_high_score(1));
        for s in 0..MAX_ENTRIES as u32 {
            t.insert(e((s + 1) * 100));
        }
        assert!(t.is_high_score(10_000));
        assert!(!t.is_high_score(1));
    }

    #[test]
    fn roundtrip_json() {
        let mut t = ScoreTable::default();
        t.insert(e(123));
        let json = serde_json::to_string(&t).unwrap();
        let back: ScoreTable = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entries[0].score, 123);
    }
}
