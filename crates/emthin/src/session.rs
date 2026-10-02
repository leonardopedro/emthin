//! Session persistence: the document snapshot plus the per-figure
//! launch commands that make dormant figures relaunchable.
//!
//! Two files, both under `$XDG_STATE_HOME/emthin/` (or wherever
//! `--session-file` points):
//!
//! - `session.loro` — a raw `MathDoc::snapshot()`. The document *is* the
//!   session: the figures come back with it, because they are `\app`
//!   statements in the text.
//! - `session.json` — bookkeeping the document can't hold: which
//!   command launched each figure, so a dormant figure knows what to
//!   re-run. Without this a restored document would show a dozen empty
//!   frames with no way to revive them.
//!
//! Nothing is spawned on restore. Every figure comes back dormant; Enter
//! (or the `spawn` IPC op) launches it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Version of the on-disk format. Bump on any breaking change; the
/// loader drops anything it doesn't recognise rather than guessing.
pub const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub version: u32,
    /// Page the user was on.
    pub current_page: usize,
    /// XWayland display number, for diagnostics.
    #[serde(default)]
    pub xwayland_display: Option<u32>,
}

impl Default for SessionFile {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            current_page: 0,
            xwayland_display: None,
        }
    }
}

impl SessionFile {
    /// Read a session file. A missing or unparseable file yields the
    /// default — losing the session's bookkeeping is always better than
    /// refusing to start.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(session) if session.version == SESSION_VERSION => session,
                Ok(session) => {
                    tracing::warn!(
                        "session {} has version {}, expected {}; ignoring",
                        path.display(),
                        session.version,
                        SESSION_VERSION
                    );
                    Self::default()
                }
                Err(e) => {
                    tracing::warn!("session {} is unreadable ({e}); ignoring", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Write the session file. Logs on failure; a session that can't be
    /// saved must not take the compositor down with it.
    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    tracing::warn!("session save failed {}: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("session serialisation failed: {e}"),
        }
    }
}

/// The two paths a session occupies.
#[derive(Debug, Clone)]
pub struct SessionPaths {
    /// Loro snapshot of the document.
    pub snapshot: PathBuf,
    /// JSON bookkeeping.
    pub metadata: PathBuf,
}

impl SessionPaths {
    /// Derive both paths from the session file's stem: `foo.loro` →
    /// `foo.json`.
    pub fn from_file(file: &Path) -> Self {
        let snapshot = file.to_path_buf();
        let metadata = file.with_extension("json");
        Self { snapshot, metadata }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("emthin-session-test-{name}"));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tmp("roundtrip");
        let path = dir.join("session.json");
        let s = SessionFile {
            current_page: 3,
            ..Default::default()
        };
        s.save(&path);

        assert_eq!(SessionFile::load(&path).current_page, 3);
    }

    #[test]
    fn missing_file_yields_the_default() {
        let loaded = SessionFile::load(Path::new("/nonexistent/emthin/session.json"));
        assert_eq!(loaded.current_page, 0);
    }

    #[test]
    fn unparseable_file_yields_the_default() {
        let dir = tmp("garbage");
        let path = dir.join("session.json");
        std::fs::write(&path, b"not json at all").expect("write");
        assert_eq!(SessionFile::load(&path).current_page, 0);
    }

    #[test]
    fn future_version_is_rejected_rather_than_guessed() {
        let dir = tmp("version");
        let path = dir.join("session.json");
        std::fs::write(&path, br#"{"version":999,"current_page":7}"#).expect("write");
        let loaded = SessionFile::load(&path);
        assert_eq!(
            loaded.current_page, 0,
            "a future format must not be half-read"
        );
    }

    #[test]
    fn paths_derive_from_the_snapshot_file() {
        let paths = SessionPaths::from_file(Path::new("/run/user/1000/emthin/session.loro"));
        assert_eq!(
            paths.metadata,
            PathBuf::from("/run/user/1000/emthin/session.json")
        );
    }
}
