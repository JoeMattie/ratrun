//! Filesystem paths for persistent data (high scores).

use std::path::PathBuf;

/// Root directory for persistent data.
///
/// `RATRUN_DATA_DIR` overrides the platform data dir, which lets tests (and
/// portable installs) keep scores out of the user's real profile.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RATRUN_DATA_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ratrun")
}

pub fn scores_path() -> PathBuf {
    data_dir().join("scores.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_honours_override() {
        // Only this test touches RATRUN_DATA_DIR, and every consumer of this
        // module resolves the env var lazily, so mutating it here is safe.
        std::env::set_var("RATRUN_DATA_DIR", "/tmp/ratrun-test-data");
        assert_eq!(data_dir(), PathBuf::from("/tmp/ratrun-test-data"));
        assert_eq!(
            scores_path(),
            PathBuf::from("/tmp/ratrun-test-data/scores.json")
        );
        std::env::remove_var("RATRUN_DATA_DIR");
    }
}
