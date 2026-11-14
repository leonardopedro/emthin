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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Version of the on-disk format. Bump on any breaking change; the
/// loader drops anything it doesn't recognise rather than guessing.
pub const SESSION_VERSION: u32 = 1;

/// Reading preferences that belong to one document, not to the process that
/// happens to have it open.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentPreferences {
    /// View zoom multiplier on the fit-to-window page.
    #[serde(default = "default_view_zoom")]
    pub view_zoom: f64,
    /// Pan offsets in output-logical pixels.
    #[serde(default)]
    pub view_pan_x: f64,
    #[serde(default)]
    pub view_pan_y: f64,
    /// Presentation-only CSL style override for rendered citations.
    #[serde(default)]
    pub citation_style: Option<String>,
    /// Presentation-only CSL locale for rendered citations.
    #[serde(default)]
    pub citation_locale: Option<String>,
    /// Visible page index. Per document because a file's page count and the
    /// reader's place in it are properties of that file.
    #[serde(default)]
    pub current_page: usize,
    /// Reading size in points. Per document because it reflows that file's
    /// pagination, unlike the presentation-only view zoom.
    #[serde(default = "default_text_size")]
    pub text_size: f32,
}

impl Default for DocumentPreferences {
    fn default() -> Self {
        Self {
            view_zoom: 1.0,
            view_pan_x: 0.0,
            view_pan_y: 0.0,
            citation_style: None,
            citation_locale: None,
            current_page: 0,
            text_size: default_text_size(),
        }
    }
}

impl DocumentPreferences {
    /// Whether these values are the defaults, so a caller can skip writing an
    /// empty entry.
    pub fn is_default(&self) -> bool {
        (self.view_zoom - 1.0).abs() < 0.001
            && self.view_pan_x.abs() < 0.01
            && self.view_pan_y.abs() < 0.01
            && self.citation_style.is_none()
            && self.citation_locale.is_none()
            && self.current_page == 0
            && (self.text_size - default_text_size()).abs() < 0.01
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub version: u32,
    /// Legacy session-wide page from before page and reading size became
    /// per-document. Read as a fallback for a session with no `documents` map,
    /// and never written for a document that has its own entry.
    #[serde(default, skip_serializing_if = "is_zero_page")]
    pub current_page: usize,
    /// Legacy session-wide reading size in points. Optional so a session written
    /// before this field existed still loads at the document default.
    #[serde(
        default = "default_text_size",
        skip_serializing_if = "is_default_text_size"
    )]
    pub text_size: f32,
    /// View, reading, and citation preferences keyed by document path. The
    /// values follow the document when another file is opened in the same
    /// session.
    #[serde(default)]
    pub documents: BTreeMap<String, DocumentPreferences>,
    /// Legacy top-level fields from before preferences were per document.
    /// Read as a fallback for the current document, and never written back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_locale: Option<String>,
    #[serde(default = "default_view_zoom", skip_serializing_if = "is_default_view_zoom")]
    pub view_zoom: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub view_pan_x: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub view_pan_y: f64,
    /// XWayland display number, for diagnostics.
    #[serde(default)]
    pub xwayland_display: Option<u32>,
}

fn is_default_view_zoom(zoom: &f64) -> bool {
    (*zoom - 1.0).abs() < 0.001
}

fn is_zero(value: &f64) -> bool {
    value.abs() < 0.01
}

fn default_text_size() -> f32 {
    mathed_mini::DEFAULT_TEXT_SIZE_PT
}

fn is_default_text_size(pt: &f32) -> bool {
    (*pt - default_text_size()).abs() < 0.01
}

fn is_zero_page(page: &usize) -> bool {
    *page == 0
}

fn default_view_zoom() -> f64 {
    1.0
}

impl Default for SessionFile {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            current_page: 0,
            text_size: default_text_size(),
            documents: BTreeMap::new(),
            citation_style: None,
            citation_locale: None,
            view_zoom: 1.0,
            view_pan_x: 0.0,
            view_pan_y: 0.0,
            xwayland_display: None,
        }
    }
}

impl SessionFile {
    /// Preferences for `key`, falling back to the pre-per-document top-level
    /// fields only for a file that predates the `documents` map. Once any
    /// document has its own entry, a document without one starts at defaults:
    /// otherwise opening a new file would inherit the previous file's page.
    pub fn preferences_for(&self, key: &str) -> DocumentPreferences {
        if let Some(preferences) = self.documents.get(key) {
            return preferences.clone();
        }
        if !self.documents.is_empty() {
            return DocumentPreferences::default();
        }
        DocumentPreferences {
            view_zoom: self.view_zoom,
            view_pan_x: self.view_pan_x,
            view_pan_y: self.view_pan_y,
            citation_style: self.citation_style.clone(),
            citation_locale: self.citation_locale.clone(),
            current_page: self.current_page,
            text_size: self.text_size,
        }
    }

    /// Store the current document's preferences and drop the legacy top-level
    /// copy so it cannot leak into a different document on the next load.
    pub fn set_preferences(&mut self, key: String, preferences: DocumentPreferences) {
        self.documents.insert(key, preferences);
        self.citation_style = None;
        self.citation_locale = None;
        self.view_zoom = 1.0;
        self.view_pan_x = 0.0;
        self.view_pan_y = 0.0;
        self.current_page = 0;
        self.text_size = default_text_size();
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
    fn older_sessions_without_a_text_size_use_the_document_default() {
        let dir = tmp("older-session");
        let path = dir.join("session.json");
        std::fs::write(&path, br#"{"version":1,"current_page":2}"#).expect("write");
        let loaded = SessionFile::load(&path);
        assert_eq!(loaded.current_page, 2);
        assert_eq!(loaded.text_size, mathed_mini::DEFAULT_TEXT_SIZE_PT);
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

    /// A file written before preferences were per document still applies its
    /// top-level zoom and citation format to the current document.
    #[test]
    fn legacy_top_level_preferences_are_read_for_the_current_document() {
        let dir = tmp("legacy-preferences");
        let path = dir.join("session.json");
        std::fs::write(
            &path,
            br#"{"version":1,"current_page":2,"text_size":24.0,"view_zoom":2.5,"view_pan_x":4.0,"citation_style":"vancouver","citation_locale":"de-DE"}"#,
        )
        .expect("write");
        let loaded = SessionFile::load(&path);
        let preferences = loaded.preferences_for("/documents/notes.doc");
        assert_eq!(preferences.view_zoom, 2.5);
        assert_eq!(preferences.view_pan_x, 4.0);
        assert_eq!(preferences.citation_style.as_deref(), Some("vancouver"));
        assert_eq!(preferences.citation_locale.as_deref(), Some("de-DE"));
        assert_eq!(preferences.current_page, 2);
        assert_eq!(preferences.text_size, 24.0);
    }

    #[test]
    fn preferences_for_a_new_document_do_not_inherit_the_previous_page() {
        let mut session = SessionFile {
            current_page: 4,
            text_size: 24.0,
            ..Default::default()
        };
        session.set_preferences(
            "/documents/a.doc".into(),
            DocumentPreferences {
                current_page: 3,
                text_size: 20.0,
                ..Default::default()
            },
        );

        let a = session.preferences_for("/documents/a.doc");
        assert_eq!(a.current_page, 3);
        assert_eq!(a.text_size, 20.0);

        let b = session.preferences_for("/documents/b.doc");
        assert_eq!(b.current_page, 0, "a new document starts on its first page");
        assert_eq!(
            b.text_size,
            mathed_mini::DEFAULT_TEXT_SIZE_PT,
            "and at the default reading size"
        );
    }

    #[test]
    fn writing_preferences_migrates_away_from_the_legacy_fields() {
        let dir = tmp("preference-migration");
        let path = dir.join("session.json");
        let mut session = SessionFile {
            view_zoom: 2.0,
            citation_style: Some("mla".into()),
            current_page: 6,
            text_size: 24.0,
            ..Default::default()
        };
        session.set_preferences(
            "/documents/notes.doc".into(),
            DocumentPreferences {
                view_zoom: 1.5,
                citation_style: Some("apa".into()),
                current_page: 2,
                text_size: 14.0,
                ..Default::default()
            },
        );
        session.save(&path);
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(!text.contains("view_zoom\": 2.0"));
        assert!(!text.contains("citation_style\": \"mla\""));
        assert!(!text.contains("\"current_page\": 6"));
        assert!(!text.contains("\"text_size\": 24.0"));
        let loaded = SessionFile::load(&path);
        let preferences = loaded.preferences_for("/documents/notes.doc");
        assert_eq!(preferences.view_zoom, 1.5);
        assert_eq!(preferences.citation_style.as_deref(), Some("apa"));
        assert_eq!(preferences.current_page, 2);
        assert_eq!(preferences.text_size, 14.0);
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
