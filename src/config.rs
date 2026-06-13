//! Filesystem paths for persistent data (high scores).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ratrun")
}

pub fn scores_path() -> PathBuf {
    data_dir().join("scores.json")
}
