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

pub mod store;

/// Version of the on-disk format. Bump on any breaking change; the
/// loader drops anything it doesn't recognise rather than guessing.
pub const SESSION_VERSION: u32 = 1;

/// A document-bound Loro cursor plus its capture-time fallback. The encoded
/// position is opaque and belongs only to the snapshot it was created from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CursorAnchor {
    pub encoded: String,
    pub side: i32,
    pub fallback: usize,
}

impl CursorAnchor {
    pub fn new(encoded: Vec<u8>, side: i32, fallback: usize) -> Self {
        Self {
            encoded: crate::ipc::hex::encode(&encoded),
            side,
            fallback,
        }
    }

    pub fn encoded_bytes(&self) -> Option<Vec<u8>> {
        crate::ipc::hex::decode(&self.encoded).ok()
    }
}

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
    /// Logical caret position, document-bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret_anchor: Option<CursorAnchor>,
    /// Fixed end of a non-empty selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_anchor: Option<CursorAnchor>,
    /// Caret end of a non-empty selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_caret: Option<CursorAnchor>,
    /// Top-visible source position for the reading page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_anchor: Option<CursorAnchor>,
    /// Vertical displacement in page points from the viewport top to the
    /// reading anchor's line. Absent in older records, which fall back to the
    /// saved page and pan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_offset: Option<f64>,
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
            caret_anchor: None,
            selection_anchor: None,
            selection_caret: None,
            reading_anchor: None,
            reading_offset: None,
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
            && self.caret_anchor.is_none()
            && self.selection_anchor.is_none()
            && self.selection_caret.is_none()
            && self.reading_anchor.is_none()
            && self.reading_offset.is_none()
    }
}

/// The explicit document file and the bytes last acknowledged as written.
/// This is recovery bookkeeping, not the session snapshot: a recovery save
/// must not claim the file is clean, and a file save must not discard an
/// unsafe recovery pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExplicitFileRecord {
    pub path: PathBuf,
    pub kind: String,
    pub bytes: u64,
    pub sha256: String,
    /// Digest of the content this representation covers: source text for a
    /// source export, text plus marks for a snapshot.
    pub content: String,
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
    #[serde(
        default = "default_view_zoom",
        skip_serializing_if = "is_default_view_zoom"
    )]
    pub view_zoom: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub view_pan_x: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub view_pan_y: f64,
    /// XWayland display number, for diagnostics.
    #[serde(default)]
    pub xwayland_display: Option<u32>,
    /// SHA-256 of the snapshot this metadata was written with. Absent in
    /// sessions written before the integrity check existed; such a pair is
    /// accepted as legacy and migrated on the next successful load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    /// Explicit destination and its confirmed baseline. Absent in older
    /// sessions; such a recovery reports only its own clean state until the
    /// user opens or saves a file again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_file: Option<Box<ExplicitFileRecord>>,
}

/// Why a metadata file could not be trusted.
#[derive(Debug)]
pub enum MetadataError {
    /// The file is not valid JSON or is missing required fields.
    Parse(String),
    /// The file was written by a different session format version.
    Version(u32),
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
            snapshot_digest: None,
            explicit_file: None,
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
            ..Default::default()
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

    /// Parse metadata bytes strictly enough for the store to distinguish a
    /// corrupt file from one written by a newer format.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MetadataError> {
        let session: Self = serde_json::from_slice(bytes)
            .map_err(|error| MetadataError::Parse(error.to_string()))?;
        if session.version != SESSION_VERSION {
            return Err(MetadataError::Version(session.version));
        }
        Ok(session)
    }

    /// Read a session file for callers that only need best-effort
    /// bookkeeping. A missing or unparseable file yields the default;
    /// the store uses [`Self::from_bytes`] when it must report the reason.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match Self::from_bytes(&bytes) {
                Ok(session) => session,
                Err(MetadataError::Version(version)) => {
                    tracing::warn!(
                        "session {} has version {}, expected {}; ignoring",
                        path.display(),
                        version,
                        SESSION_VERSION
                    );
                    Self::default()
                }
                Err(MetadataError::Parse(error)) => {
                    tracing::warn!(
                        "session {} is unreadable ({error}); ignoring",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Write the metadata file atomically. The session store writes the
    /// snapshot and metadata as one verified pair; this is for the metadata-
    /// only path used by tests and legacy migration.
    pub fn save(&self, path: &Path) {
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(e) = store::write_atomic(path, &bytes) {
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
    /// Previous snapshot, retained until the next successful save.
    pub snapshot_backup: PathBuf,
    /// Metadata matching `snapshot_backup`.
    pub metadata_backup: PathBuf,
}

impl SessionPaths {
    /// Derive the sidecar paths from the session snapshot. The metadata is
    /// always `session.json` in the snapshot's directory, matching the
    /// long-standing `--session-file` contract.
    pub fn from_file(file: &Path) -> Self {
        let snapshot = file.to_path_buf();
        let metadata = file
            .parent()
            .map(|dir| dir.join("session.json"))
            .unwrap_or_else(|| PathBuf::from("session.json"));
        Self {
            snapshot_backup: with_suffix(file, ".bak"),
            metadata_backup: with_suffix(&metadata, ".bak"),
            snapshot,
            metadata,
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
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
    fn old_preferences_without_a_reading_offset_still_load() {
        let dir = tmp("old-reading-offset");
        let path = dir.join("session.json");
        std::fs::write(
            &path,
            br#"{"version":1,"documents":{"/documents/notes.doc":{"current_page":2,"view_zoom":2.0,"reading_anchor":{"encoded":"","side":-1,"fallback":0}}}}"#,
        )
        .expect("write");
        let preferences = SessionFile::load(&path).preferences_for("/documents/notes.doc");
        assert_eq!(preferences.current_page, 2);
        assert!(preferences.reading_anchor.is_some());
        assert_eq!(preferences.reading_offset, None);
    }

    #[test]
    fn reading_offset_round_trips() {
        let dir = tmp("reading-offset-roundtrip");
        let path = dir.join("session.json");
        let mut session = SessionFile::default();
        session.set_preferences(
            "/documents/notes.doc".into(),
            DocumentPreferences {
                reading_offset: Some(37.5),
                ..Default::default()
            },
        );
        session.save(&path);
        let loaded = SessionFile::load(&path).preferences_for("/documents/notes.doc");
        assert_eq!(loaded.reading_offset, Some(37.5));
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
    fn explicit_destination_record_round_trips_and_old_sessions_default_to_none() {
        let dir = tmp("explicit-record");
        let path = dir.join("session.json");
        let session = SessionFile {
            explicit_file: Some(Box::new(ExplicitFileRecord {
                path: PathBuf::from("/documents/notes.typ"),
                kind: "source".into(),
                bytes: 12,
                sha256: "a".repeat(64),
                content: "b".repeat(64),
            })),
            ..Default::default()
        };
        session.save(&path);
        let loaded = SessionFile::load(&path);
        assert_eq!(loaded.explicit_file, session.explicit_file);

        std::fs::write(&path, br#"{"version":1,"current_page":2}"#).expect("old write");
        assert_eq!(SessionFile::load(&path).explicit_file, None);
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
        assert_eq!(
            paths.snapshot_backup,
            PathBuf::from("/run/user/1000/emthin/session.loro.bak")
        );
        assert_eq!(
            paths.metadata_backup,
            PathBuf::from("/run/user/1000/emthin/session.json.bak")
        );
    }

    #[test]
    fn metadata_is_session_json_even_for_a_differently_named_snapshot() {
        let paths = SessionPaths::from_file(Path::new("/tmp/doc.loro"));
        assert_eq!(paths.metadata, PathBuf::from("/tmp/session.json"));
    }
}
