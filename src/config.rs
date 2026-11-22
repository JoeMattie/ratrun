//! Filesystem paths for persistent data (high scores, the nest, controls).
//!
//! This module only *names* the paths. It never creates, probes or writes
//! anything: `scores.rs` creates the directory on save and treats every I/O
//! failure as "this session is not persisted", so a read-only or missing data
//! directory can never stop the game from starting or panic it mid-run.

use std::ffi::OsString;
use std::path::PathBuf;

/// The environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "RATRUN_DATA_DIR";

/// Root directory for persistent data.
///
/// `RATRUN_DATA_DIR` overrides the platform data dir, which lets tests (and
/// portable installs) keep scores out of the user's real profile. The value is
/// used as given — a relative path stays relative to the working directory.
pub fn data_dir() -> PathBuf {
    resolve_data_dir(
        std::env::var_os(DATA_DIR_ENV),
        dirs::data_dir(),
        dirs::home_dir(),
    )
}

/// [`data_dir`] against explicit inputs, so the rules are testable without
/// mutating the process environment (tests run in parallel and share it).
///
/// In order:
/// 1. the override, unless it is empty or only whitespace (an exported-but-
///    blank variable must not turn the data directory into the working one);
/// 2. `<platform data dir>/ratrun`;
/// 3. `<home>/.ratrun` when the platform has no data dir to offer;
/// 4. `./ratrun` as the last resort (no home at all: a stripped container).
fn resolve_data_dir(
    override_dir: Option<OsString>,
    platform: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        let blank = dir.is_empty() || dir.to_str().is_some_and(|s| s.trim().is_empty());
        if !blank {
            return PathBuf::from(dir);
        }
    }
    if let Some(dir) = platform.filter(|d| !d.as_os_str().is_empty()) {
        return dir.join("ratrun");
    }
    if let Some(dir) = home.filter(|d| !d.as_os_str().is_empty()) {
        return dir.join(".ratrun");
    }
    PathBuf::from(".").join("ratrun")
}

/// The save file: scores, the nest profile and the control bindings.
pub fn scores_path() -> PathBuf {
    data_dir().join("scores.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    fn path(s: &str) -> Option<PathBuf> {
        Some(PathBuf::from(s))
    }

    #[test]
    fn the_override_wins_and_is_used_verbatim() {
        assert_eq!(
            resolve_data_dir(os("/tmp/ratrun-test-data"), path("/data"), path("/home/rat")),
            PathBuf::from("/tmp/ratrun-test-data")
        );
        // A relative override stays relative: a portable install beside the binary.
        assert_eq!(
            resolve_data_dir(os("saves"), path("/data"), None),
            PathBuf::from("saves")
        );
    }

    #[test]
    fn a_blank_override_is_ignored() {
        for blank in ["", " ", "\t "] {
            assert_eq!(
                resolve_data_dir(os(blank), path("/data"), path("/home/rat")),
                PathBuf::from("/data/ratrun"),
                "override {blank:?}"
            );
        }
    }

    #[test]
    fn the_fallback_chain_is_platform_then_home_then_here() {
        assert_eq!(
            resolve_data_dir(None, path("/data"), path("/home/rat")),
            PathBuf::from("/data/ratrun")
        );
        assert_eq!(
            resolve_data_dir(None, None, path("/home/rat")),
            PathBuf::from("/home/rat/.ratrun")
        );
        assert_eq!(
            resolve_data_dir(None, path(""), path("/home/rat")),
            PathBuf::from("/home/rat/.ratrun")
        );
        assert_eq!(resolve_data_dir(None, None, None), PathBuf::from("./ratrun"));
    }

    #[test]
    fn the_save_file_lives_in_the_data_dir() {
        // Read-only on the environment: whatever the directory resolves to,
        // the file is `scores.json` directly inside it.
        let file = scores_path();
        assert_eq!(file.file_name().and_then(|n| n.to_str()), Some("scores.json"));
        assert_eq!(file.parent(), Some(data_dir().as_path()));
    }
}
