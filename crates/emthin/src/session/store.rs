//! Crash-safe storage for the document snapshot and its metadata.
//!
//! The snapshot stays at the path the user supplied. Metadata is written
//! beside it, and both files are replaced through same-directory temporary
//! files. Before the new pair is installed, the previous verified pair is
//! moved aside as the backup. The metadata carries a SHA-256 of its snapshot,
//! so a torn or externally modified snapshot is never paired with the wrong
//! preferences.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use mathed_core::doc::MathDoc;
use sha2::{Digest as _, Sha256};

use super::{MetadataError, SessionFile, SessionPaths, SESSION_VERSION};

/// Where a successfully loaded pair came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSource {
    Primary,
    Backup,
    Legacy,
}

/// A snapshot and the metadata that was verified with it.
#[derive(Debug)]
pub struct Loaded {
    pub snapshot: Vec<u8>,
    pub preferences: SessionFile,
    pub source: LoadSource,
}

/// The result of opening a session path.
#[derive(Debug)]
pub enum LoadResult {
    Loaded(Loaded),
    Missing,
    /// Files exist but no valid snapshot could be recovered. Copies of the
    /// unreadable files are named in `quarantined_paths`; the originals are
    /// left in place and the caller must not autosave over them.
    Unrecoverable {
        quarantined_paths: Vec<PathBuf>,
    },
}

/// Owns reads and writes for one session file and its sidecars.
#[derive(Debug, Clone)]
pub struct SessionStore {
    paths: SessionPaths,
}

impl SessionStore {
    pub fn new(file: &Path) -> Self {
        Self {
            paths: SessionPaths::from_file(file),
        }
    }

    /// Read the current metadata without touching the snapshot. Used when an
    /// explicit document is opened and only its preferences are needed.
    pub fn metadata(&self) -> Option<SessionFile> {
        read_metadata(&self.paths.metadata).or_else(|| read_metadata(&self.paths.metadata_backup))
    }

    /// Load the newest verified pair, then the backup, then the legacy
    /// unpaired shape.
    pub fn load(&self) -> LoadResult {
        let primary = match read_optional(&self.paths.snapshot) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    "cannot read session snapshot {}: {error}",
                    self.paths.snapshot.display()
                );
                return self.unrecoverable(&[]);
            }
        };
        let backup = match read_optional(&self.paths.snapshot_backup) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    "cannot read session backup {}: {error}; continuing without it",
                    self.paths.snapshot_backup.display()
                );
                None
            }
        };
        let metadata_bytes = match read_optional(&self.paths.metadata) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    "cannot read session metadata {}: {error}",
                    self.paths.metadata.display()
                );
                return self.unrecoverable(&[]);
            }
        };
        let backup_metadata_bytes = match read_optional(&self.paths.metadata_backup) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    "cannot read session backup metadata {}: {error}; continuing without it",
                    self.paths.metadata_backup.display()
                );
                None
            }
        };

        let mut primary_meta = None;
        let mut primary_meta_parse_failed = false;
        if let Some(bytes) = &metadata_bytes {
            match SessionFile::from_bytes(bytes) {
                Ok(metadata) => primary_meta = Some(metadata),
                Err(MetadataError::Version(version)) => {
                    tracing::warn!(
                        "session metadata {} has version {version}, expected {SESSION_VERSION}; \
                         refusing to overwrite it",
                        self.paths.metadata.display()
                    );
                    return self.unrecoverable(&[]);
                }
                Err(MetadataError::Parse(error)) => {
                    tracing::warn!(
                        "session metadata {} is unreadable ({error}); using the snapshot with \
                         default preferences",
                        self.paths.metadata.display()
                    );
                    primary_meta_parse_failed = true;
                }
            }
        }

        let mut backup_meta = None;
        if let Some(bytes) = &backup_metadata_bytes {
            match SessionFile::from_bytes(bytes) {
                Ok(metadata) => backup_meta = Some(metadata),
                Err(MetadataError::Version(version)) => {
                    tracing::warn!(
                        "session backup metadata {} has version {version}, expected \
                         {SESSION_VERSION}; refusing to overwrite it",
                        self.paths.metadata_backup.display()
                    );
                    return self.unrecoverable(&[]);
                }
                Err(MetadataError::Parse(error)) => {
                    tracing::warn!(
                        "session backup metadata {} is unreadable ({error}); ignoring it",
                        self.paths.metadata_backup.display()
                    );
                }
            }
        }

        // Newest pair first. Metadata without a digest is a verified legacy
        // pair: the snapshot is usable, but it must be migrated and reported
        // as legacy rather than as a current verified write.
        if let (Some(snapshot), Some(metadata)) = (&primary, &primary_meta) {
            if pair_is_valid(snapshot, metadata) && metadata.snapshot_digest.is_some() {
                return LoadResult::Loaded(Loaded {
                    snapshot: snapshot.clone(),
                    preferences: metadata.clone(),
                    source: LoadSource::Primary,
                });
            }
        }

        // A legitimate legacy pair has a snapshot but no digest in its
        // metadata. A parse failure is handled the same way: keep the
        // document, drop only the unreadable bookkeeping.
        if let Some(snapshot) = &primary {
            if snapshot_is_valid(snapshot) {
                if let Some(metadata) = &primary_meta {
                    if metadata.snapshot_digest.is_none() {
                        let preferences = self.migrate_legacy(snapshot, metadata);
                        return LoadResult::Loaded(Loaded {
                            snapshot: snapshot.clone(),
                            preferences,
                            source: LoadSource::Legacy,
                        });
                    }
                } else if metadata_bytes.is_none() {
                    let preferences = self.migrate_legacy(snapshot, &SessionFile::default());
                    return LoadResult::Loaded(Loaded {
                        snapshot: snapshot.clone(),
                        preferences,
                        source: LoadSource::Legacy,
                    });
                } else if primary_meta_parse_failed {
                    return LoadResult::Loaded(Loaded {
                        snapshot: snapshot.clone(),
                        preferences: SessionFile::default(),
                        source: LoadSource::Legacy,
                    });
                }
            }
        }

        // Canonical backup pair.
        if let (Some(snapshot), Some(metadata)) = (&backup, &backup_meta) {
            if pair_is_valid(snapshot, metadata) {
                return LoadResult::Loaded(Loaded {
                    snapshot: snapshot.clone(),
                    preferences: metadata.clone(),
                    source: if metadata.snapshot_digest.is_some() {
                        LoadSource::Backup
                    } else {
                        LoadSource::Legacy
                    },
                });
            }
        }

        // A crash can land between moving the old snapshot aside and moving
        // its metadata aside. The primary metadata still describes the
        // backup snapshot in that window.
        if let (Some(snapshot), Some(metadata)) = (&backup, &primary_meta) {
            if pair_is_valid(snapshot, metadata) {
                return LoadResult::Loaded(Loaded {
                    snapshot: snapshot.clone(),
                    preferences: metadata.clone(),
                    source: LoadSource::Backup,
                });
            }
        }

        // A legacy unpaired snapshot is still a document. Prefer the primary;
        // a backup is only used when the primary is absent or unreadable.
        if let Some(snapshot) = &backup {
            if primary.is_none() && backup_meta.is_none() && snapshot_is_valid(snapshot) {
                return LoadResult::Loaded(Loaded {
                    snapshot: snapshot.clone(),
                    preferences: SessionFile::default(),
                    source: LoadSource::Backup,
                });
            }
        }

        if primary.is_none() && backup.is_none() {
            // Metadata without a snapshot is stale bookkeeping, not a
            // document that can be lost.
            return LoadResult::Missing;
        }

        self.unrecoverable(&[])
    }

    /// Write a verified snapshot/metadata pair. The previous valid primary
    /// becomes the backup; a failed write leaves the old pair recoverable.
    pub fn save(&self, snapshot: &[u8], preferences: &SessionFile) -> io::Result<()> {
        let mut metadata = preferences.clone();
        metadata.version = SESSION_VERSION;
        metadata.snapshot_digest = Some(digest_hex(snapshot));
        let metadata_bytes = serde_json::to_vec_pretty(&metadata).map_err(io::Error::other)?;

        let snapshot_tmp = temp_path(&self.paths.snapshot);
        let metadata_tmp = temp_path(&self.paths.metadata);
        write_file(&snapshot_tmp, snapshot)?;
        if let Err(error) = write_file(&metadata_tmp, &metadata_bytes) {
            let _ = fs::remove_file(&snapshot_tmp);
            return Err(error);
        }

        let mut backed_up = false;
        if pair_on_disk_is_valid(&self.paths.snapshot, &self.paths.metadata) {
            if let Err(error) = fs::rename(&self.paths.snapshot, &self.paths.snapshot_backup) {
                cleanup(&[&snapshot_tmp, &metadata_tmp]);
                return Err(error);
            }
            backed_up = true;
            if let Err(error) = fs::rename(&self.paths.metadata, &self.paths.metadata_backup) {
                // Put the snapshot back when possible. If that fails, the
                // backup snapshot and the still-present primary metadata
                // remain a verified pair.
                if !self.paths.snapshot.exists() && self.paths.snapshot_backup.exists() {
                    let _ = fs::rename(&self.paths.snapshot_backup, &self.paths.snapshot);
                }
                cleanup(&[&snapshot_tmp, &metadata_tmp]);
                return Err(error);
            }
            sync_parent(&self.paths.snapshot);
        }

        if let Err(error) = fs::rename(&snapshot_tmp, &self.paths.snapshot) {
            if backed_up && !self.paths.snapshot.exists() && self.paths.snapshot_backup.exists() {
                let _ = fs::rename(&self.paths.snapshot_backup, &self.paths.snapshot);
            }
            cleanup(&[&snapshot_tmp, &metadata_tmp]);
            return Err(error);
        }
        sync_parent(&self.paths.snapshot);

        if let Err(error) = fs::rename(&metadata_tmp, &self.paths.metadata) {
            // The backup pair remains loadable. Do not report a successful
            // save until both renames have completed.
            cleanup(&[&snapshot_tmp, &metadata_tmp]);
            return Err(error);
        }

        sync_parent(&self.paths.snapshot);
        Ok(())
    }

    fn migrate_legacy(&self, snapshot: &[u8], preferences: &SessionFile) -> SessionFile {
        let mut migrated = preferences.clone();
        migrated.snapshot_digest = Some(digest_hex(snapshot));
        match serde_json::to_vec_pretty(&migrated) {
            Ok(bytes) => {
                if let Err(error) = write_atomic(&self.paths.metadata, &bytes) {
                    tracing::warn!(
                        "could not migrate session metadata {}: {error}",
                        self.paths.metadata.display()
                    );
                    return preferences.clone();
                }
            }
            Err(error) => {
                tracing::warn!("could not serialise migrated session metadata: {error}");
                return preferences.clone();
            }
        }
        migrated
    }

    fn unrecoverable(&self, extra: &[PathBuf]) -> LoadResult {
        let mut paths: Vec<PathBuf> = [
            &self.paths.snapshot,
            &self.paths.snapshot_backup,
            &self.paths.metadata,
            &self.paths.metadata_backup,
        ]
        .into_iter()
        .filter(|path| path.exists())
        .cloned()
        .collect();
        for path in extra {
            if path.exists() && !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        let quarantined_paths = quarantine(&paths);
        tracing::warn!(
            "session {} is unrecoverable; preserved copies: {}",
            self.paths.snapshot.display(),
            quarantined_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        LoadResult::Unrecoverable { quarantined_paths }
    }
}

/// Write one file through a same-directory temporary and rename it into
/// place. Used for metadata-only legacy migration.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_publish(path, bytes, true)
}

/// Publish a completed temporary file. `replace` selects rename semantics;
/// `false` uses a hard link so a concurrent creator wins with `AlreadyExists`
/// instead of being silently clobbered.
pub(crate) fn write_atomic_publish(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let temp = temp_path(path);
    write_file(&temp, bytes)?;
    let publish = if replace {
        fs::rename(&temp, path)
    } else {
        fs::hard_link(&temp, path).map(|()| {
            // The destination is published; a stale temp must not turn a
            // successful create into a reported failure.
            let _ = fs::remove_file(&temp);
        })
    };
    if let Err(error) = publish {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    sync_parent(path);
    Ok(())
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_metadata(path: &Path) -> Option<SessionFile> {
    let bytes = read_optional(path).ok().flatten()?;
    SessionFile::from_bytes(&bytes).ok()
}

fn pair_on_disk_is_valid(snapshot_path: &Path, metadata_path: &Path) -> bool {
    let Ok(Some(snapshot)) = read_optional(snapshot_path) else {
        return false;
    };
    let Some(metadata) = read_metadata(metadata_path) else {
        return false;
    };
    pair_is_valid(&snapshot, &metadata)
}

fn pair_is_valid(snapshot: &[u8], metadata: &SessionFile) -> bool {
    digest_matches(metadata, snapshot) && snapshot_is_valid(snapshot)
}

fn digest_matches(metadata: &SessionFile, snapshot: &[u8]) -> bool {
    match metadata.snapshot_digest.as_deref() {
        Some(expected) => expected.eq_ignore_ascii_case(&digest_hex(snapshot)),
        None => true,
    }
}

fn snapshot_is_valid(snapshot: &[u8]) -> bool {
    MathDoc::from_snapshot(snapshot).is_ok()
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn temp_path(path: &Path) -> PathBuf {
    with_suffix(path, ".tmp")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sync_parent(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    if parent.as_os_str().is_empty() {
        return;
    }
    match File::open(parent).and_then(|dir| dir.sync_all()) {
        Ok(()) => {}
        Err(error) => tracing::warn!(
            "could not sync session directory {}: {error}",
            parent.display()
        ),
    }
}

fn cleanup(paths: &[&PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn quarantine(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut preserved = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let target = with_suffix(path, ".rejected");
        if target.exists() {
            preserved.push(target);
            continue;
        }
        match fs::copy(path, &target) {
            Ok(_) => {
                if let Err(error) = File::open(&target).and_then(|file| file.sync_all()) {
                    tracing::warn!(
                        "could not sync quarantined session copy {}: {error}",
                        target.display()
                    );
                }
                preserved.push(target);
            }
            Err(error) => tracing::warn!(
                "could not quarantine session file {}: {error}",
                path.display()
            ),
        }
    }
    preserved
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let thread = format!("{:?}", std::thread::current().id());
            let path =
                std::env::temp_dir().join(format!("emthin-store-{name}-{pid}-{thread}-{seq}"));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("doc.loro")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot(text: &str) -> Vec<u8> {
        crate::docui::DocModel::new(text).snapshot()
    }

    fn preferences(page: usize) -> SessionFile {
        let mut session = SessionFile::default();
        session.set_preferences(
            "/documents/notes.doc".into(),
            crate::session::DocumentPreferences {
                current_page: page,
                ..Default::default()
            },
        );
        session
    }

    fn loaded(result: LoadResult) -> Loaded {
        match result {
            LoadResult::Loaded(loaded) => loaded,
            other => panic!("expected a loaded session, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_writes_a_digest_and_reads_the_primary_pair() {
        let dir = TempDir::new("roundtrip");
        let store = SessionStore::new(&dir.file());
        let snapshot = snapshot("the document text");
        store.save(&snapshot, &preferences(3)).expect("save");

        let result = loaded(store.load());
        assert_eq!(result.snapshot, snapshot);
        assert_eq!(result.source, LoadSource::Primary);
        assert_eq!(
            result
                .preferences
                .preferences_for("/documents/notes.doc")
                .current_page,
            3
        );
        let metadata =
            read_metadata(&SessionPaths::from_file(&dir.file()).metadata).expect("metadata");
        assert!(
            metadata.snapshot_digest.is_some(),
            "new metadata carries a digest"
        );
    }

    #[test]
    fn a_missing_session_is_missing() {
        let dir = TempDir::new("missing");
        let store = SessionStore::new(&dir.file());
        assert!(matches!(store.load(), LoadResult::Missing));
    }

    #[test]
    fn create_only_publication_does_not_clobber_a_concurrent_creator() {
        let dir = TempDir::new("create-only");
        let path = dir.0.join("created.bin");
        fs::write(&path, b"concurrent").expect("concurrent creator");

        let error = write_atomic_publish(&path, b"mine", false)
            .expect_err("an existing destination must win");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).expect("destination"), b"concurrent");
    }

    #[test]
    fn a_future_metadata_version_is_not_overwritten() {
        let dir = TempDir::new("future");
        let store = SessionStore::new(&dir.file());
        let snapshot = snapshot("newer document");
        fs::write(dir.file(), &snapshot).expect("snapshot");
        fs::write(
            SessionPaths::from_file(&dir.file()).metadata,
            br#"{"version":999,"snapshot_digest":null}"#,
        )
        .expect("metadata");

        assert!(matches!(store.load(), LoadResult::Unrecoverable { .. }));
        let bytes = fs::read(dir.file()).expect("snapshot still exists");
        assert_eq!(bytes, snapshot, "the load does not replace the snapshot");
    }

    #[test]
    fn a_truncated_primary_falls_back_to_the_backup_pair() {
        let dir = TempDir::new("truncated");
        let store = SessionStore::new(&dir.file());
        let first = snapshot("first document");
        let second = snapshot("second document");
        store.save(&first, &preferences(1)).expect("first save");
        store.save(&second, &preferences(2)).expect("second save");
        fs::write(dir.file(), b"torn").expect("truncate primary");

        let result = loaded(store.load());
        assert_eq!(result.snapshot, first);
        assert_eq!(result.source, LoadSource::Backup);
        assert_eq!(
            result
                .preferences
                .preferences_for("/documents/notes.doc")
                .current_page,
            1,
            "the backup metadata travels with the backup snapshot"
        );
    }

    #[test]
    fn a_checksum_mismatch_is_unrecoverable_when_no_backup_exists() {
        let dir = TempDir::new("digest");
        let store = SessionStore::new(&dir.file());
        store
            .save(&snapshot("document"), &preferences(0))
            .expect("save");
        let other = snapshot("different document");
        fs::write(dir.file(), other).expect("replace outside the store");

        assert!(matches!(store.load(), LoadResult::Unrecoverable { .. }));
    }

    #[test]
    fn a_failed_temporary_write_leaves_the_old_pair_untouched() {
        let dir = TempDir::new("temp-failure");
        let store = SessionStore::new(&dir.file());
        let first = snapshot("first document");
        store.save(&first, &preferences(1)).expect("first save");
        let paths = SessionPaths::from_file(&dir.file());
        fs::create_dir(temp_path(&paths.snapshot)).expect("block snapshot temp");

        assert!(store
            .save(&snapshot("second document"), &preferences(2))
            .is_err());
        let result = loaded(store.load());
        assert_eq!(result.snapshot, first);
        assert_eq!(result.source, LoadSource::Primary);

        fs::remove_dir(temp_path(&paths.snapshot)).expect("clean temp");
    }

    #[test]
    fn a_failed_backup_rename_leaves_the_old_pair_untouched() {
        let dir = TempDir::new("rename-failure");
        let store = SessionStore::new(&dir.file());
        let first = snapshot("first document");
        store.save(&first, &preferences(1)).expect("first save");
        let paths = SessionPaths::from_file(&dir.file());
        fs::create_dir(&paths.snapshot_backup).expect("block backup path");

        assert!(store
            .save(&snapshot("second document"), &preferences(2))
            .is_err());
        let result = loaded(store.load());
        assert_eq!(result.snapshot, first);
        assert_eq!(result.source, LoadSource::Primary);

        fs::remove_dir(&paths.snapshot_backup).expect("clean backup");
    }

    #[test]
    fn a_valid_backup_survives_a_torn_primary() {
        let dir = TempDir::new("torn");
        let store = SessionStore::new(&dir.file());
        let first = snapshot("first document");
        store.save(&first, &preferences(1)).expect("first save");
        store
            .save(&snapshot("second document"), &preferences(2))
            .expect("second save");
        let paths = SessionPaths::from_file(&dir.file());
        fs::write(&paths.snapshot, b"partial").expect("tear primary");

        let result = loaded(store.load());
        assert_eq!(result.snapshot, first);
        assert_eq!(result.source, LoadSource::Backup);
    }

    #[test]
    fn a_legacy_pair_is_migrated_after_a_verified_load() {
        let dir = TempDir::new("legacy");
        let store = SessionStore::new(&dir.file());
        let snapshot = snapshot("legacy document");
        fs::write(dir.file(), &snapshot).expect("snapshot");
        let mut metadata = preferences(2);
        metadata.snapshot_digest = None;
        metadata.save(&SessionPaths::from_file(&dir.file()).metadata);

        let result = loaded(store.load());
        assert_eq!(result.source, LoadSource::Legacy);
        assert_eq!(result.snapshot, snapshot);
        let migrated = read_metadata(&SessionPaths::from_file(&dir.file()).metadata)
            .expect("migrated metadata");
        assert_eq!(
            migrated.snapshot_digest.as_deref(),
            Some(digest_hex(&snapshot).as_str())
        );
        assert_eq!(
            migrated
                .preferences_for("/documents/notes.doc")
                .current_page,
            2,
            "migration keeps the preferences"
        );
    }

    #[test]
    fn an_unrecoverable_pair_is_preserved_and_quarantined() {
        let dir = TempDir::new("preserve");
        let store = SessionStore::new(&dir.file());
        let paths = SessionPaths::from_file(&dir.file());
        fs::write(&paths.snapshot, b"not a loro snapshot").expect("snapshot");
        let mut metadata = preferences(4);
        metadata.snapshot_digest = Some("0".repeat(64));
        metadata.save(&paths.metadata);

        let LoadResult::Unrecoverable { quarantined_paths } = store.load() else {
            panic!("expected an unrecoverable session");
        };
        assert!(!quarantined_paths.is_empty());
        for path in &quarantined_paths {
            assert!(
                path.exists(),
                "quarantine copy remains at {}",
                path.display()
            );
        }
        let snapshot_before = fs::read(&paths.snapshot).expect("snapshot");
        let metadata_before = fs::read(&paths.metadata).expect("metadata");

        // A later write is allowed only by the caller; the store itself does
        // not claim this is safe. The rejection copies are the preservation
        // guarantee for the caller's blocked-autosave path.
        assert_eq!(
            fs::read(&paths.snapshot).expect("snapshot"),
            snapshot_before
        );
        assert_eq!(
            fs::read(&paths.metadata).expect("metadata"),
            metadata_before
        );
    }

    #[test]
    fn preferences_for_another_document_survive_a_failed_save() {
        let dir = TempDir::new("other-preferences");
        let store = SessionStore::new(&dir.file());
        let paths = SessionPaths::from_file(&dir.file());
        let first = snapshot("first document");
        let mut before = preferences(1);
        before.set_preferences(
            "/documents/other.doc".into(),
            crate::session::DocumentPreferences {
                current_page: 7,
                ..Default::default()
            },
        );
        store.save(&first, &before).expect("first save");
        fs::create_dir(temp_path(&paths.snapshot)).expect("block snapshot temp");

        assert!(store
            .save(&snapshot("second document"), &preferences(2))
            .is_err());
        let after = read_metadata(&paths.metadata).expect("metadata");
        assert_eq!(
            after.preferences_for("/documents/other.doc").current_page,
            7
        );

        fs::remove_dir(temp_path(&paths.snapshot)).expect("clean temp");
    }
}
