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

    fn normalize(&mut self) {
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.score));
        self.entries.truncate(MAX_ENTRIES);
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

    #[test]
    fn roundtrip_json() {
        let mut t = ScoreTable::default();
        t.insert(e(123));
        let json = serde_json::to_string(&t).unwrap();
        let back: ScoreTable = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entries[0].score, 123);
    }
}
