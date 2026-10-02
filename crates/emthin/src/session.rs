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
    /// Per-figure launch commands, keyed by figure key (`f<stmt-index>`).
    ///
    /// A key that no longer exists in the document is ignored on load;
    /// a document figure with no entry is simply not relaunchable.
    #[serde(default)]
    pub spawns: Vec<SpawnEntry>,
    /// XWayland display number, for diagnostics.
    #[serde(default)]
    pub xwayland_display: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnEntry {
    /// The `\app` statement key this command belongs to.
    pub figure: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for SessionFile {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            current_page: 0,
            spawns: Vec::new(),
            xwayland_display: None,
        }
    }
}

impl SessionFile {
    /// The command recorded for a figure, if any.
    pub fn spawn_for(&self, figure: &str) -> Option<(&str, &[String])> {
        self.spawns
            .iter()
            .find(|e| e.figure == figure)
            .map(|e| (e.cmd.as_str(), e.args.as_slice()))
    }

    /// Record (or replace) a figure's launch command.
    pub fn set_spawn(&mut self, figure: &str, cmd: String, args: Vec<String>) {
        match self.spawns.iter_mut().find(|e| e.figure == figure) {
            Some(entry) => {
                entry.cmd = cmd;
                entry.args = args;
            }
            None => self.spawns.push(SpawnEntry {
                figure: figure.to_string(),
                cmd,
                args,
            }),
        }
    }

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
    fn spawns_round_trip_and_replace() {
        let mut s = SessionFile::default();
        s.set_spawn("f0", "foot".into(), vec!["-T".into()]);
        assert_eq!(s.spawn_for("f0"), Some(("foot", &["-T".to_string()][..])));
        // Same figure again replaces rather than duplicates.
        s.set_spawn("f0", "alacritty".into(), vec![]);
        assert_eq!(s.spawn_for("f0"), Some(("alacritty", &[][..])));
        assert_eq!(s.spawns.len(), 1);
        assert!(s.spawn_for("f9").is_none());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tmp("roundtrip");
        let path = dir.join("session.json");
        let mut s = SessionFile {
            current_page: 3,
            ..Default::default()
        };
        s.set_spawn("f1", "foot".into(), vec!["-x".into()]);
        s.save(&path);

        let loaded = SessionFile::load(&path);
        assert_eq!(loaded.current_page, 3);
        assert_eq!(loaded.spawn_for("f1").map(|(c, _)| c), Some("foot"));
    }

    #[test]
    fn missing_file_yields_the_default() {
        let loaded = SessionFile::load(Path::new("/nonexistent/emthin/session.json"));
        assert_eq!(loaded.current_page, 0);
        assert!(loaded.spawns.is_empty());
    }

    #[test]
    fn unparseable_file_yields_the_default() {
        let dir = tmp("garbage");
        let path = dir.join("session.json");
        std::fs::write(&path, b"not json at all").expect("write");
        assert!(SessionFile::load(&path).spawns.is_empty());
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
