//! Document files: source text, Loro snapshots, atomic replacement and
//! outside-write detection.
//!
//! The opened file and the session snapshot are different things. A document
//! file is what `--doc`/`open_doc` reads and `save_doc` writes; the session
//! file is the automatic restore path. This module keeps the file contract
//! small: detect the representation, remember the bytes we last saw, refuse to
//! replace a file that changed underneath us unless the caller forces it, and
//! write through the same atomic helper the session store uses.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest as _, Sha256};

use super::model::DocModel;

/// The representation stored in a document file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// UTF-8 Typst/mathed source.
    Source,
    /// Raw Loro snapshot bytes.
    Snapshot,
}

impl FileKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "source" => Some(Self::Source),
            "snapshot" => Some(Self::Snapshot),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Snapshot => "snapshot",
        }
    }

    /// Pick the save representation from an explicit request, then the path
    /// extension. Source is the conservative default.
    pub fn for_save(path: &Path, format: Option<SaveFormat>) -> Self {
        format.map(SaveFormat::kind).unwrap_or_else(|| {
            match path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("loro" | "mathed") => Self::Snapshot,
                _ => Self::Source,
            }
        })
    }
}

/// Optional `save_doc` format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    Source,
    Snapshot,
}

impl SaveFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "source" => Ok(Self::Source),
            "snapshot" => Ok(Self::Snapshot),
            other => Err(format!(
                "invalid document format '{other}' (expected source or snapshot)"
            )),
        }
    }

    pub fn kind(self) -> FileKind {
        match self {
            Self::Source => FileKind::Source,
            Self::Snapshot => FileKind::Snapshot,
        }
    }
}

/// What we read from disk when a document was opened or saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub canonical_path: PathBuf,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub sha256: String,
}

impl FileStamp {
    /// Rebuild the stamp a previous save left behind. Used when a recovered
    /// session records its explicit destination: the recorded digest is the
    /// conflict baseline, not whatever the file happens to contain now.
    pub fn from_baseline(path: &Path, bytes: u64, sha256: String) -> Self {
        let canonical_path = canonical_target(path);
        let modified = std::fs::metadata(&canonical_path)
            .and_then(|metadata| metadata.modified())
            .ok();
        Self {
            canonical_path,
            bytes,
            modified,
            sha256,
        }
    }

    fn capture(path: &Path, bytes: &[u8]) -> Self {
        let canonical_path = canonical_target(path);
        let modified = std::fs::metadata(&canonical_path)
            .and_then(|metadata| metadata.modified())
            .ok();
        Self {
            canonical_path,
            bytes: bytes.len() as u64,
            modified,
            sha256: sha256_hex(bytes),
        }
    }
}

/// A document read from disk, with the stamp needed to save it back safely.
pub struct OpenedDocument {
    pub kind: FileKind,
    pub model: DocModel,
    pub stamp: FileStamp,
}

/// A successfully written document.
#[derive(Debug, Clone)]
pub struct SavedDocument {
    pub path: PathBuf,
    pub kind: FileKind,
    pub digest: String,
    pub stamp: FileStamp,
}

#[derive(Debug)]
pub enum FileError {
    Io(String),
    /// The file changed after it was opened or last saved. `path` is the
    /// target that would have been replaced.
    Conflict(PathBuf),
    Invalid(String),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => f.write_str(message),
            Self::Conflict(path) => write!(
                f,
                "{} changed on disk since it was opened; save with force to replace it",
                path.display()
            ),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FileError {}

impl From<std::io::Error> for FileError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Read a source or snapshot document. Snapshot detection comes first, so a
/// `.typ` file that happens to contain a valid Loro snapshot is still treated
/// as a snapshot.
pub fn read(path: &Path) -> Result<OpenedDocument, FileError> {
    let bytes = std::fs::read(path)
        .map_err(|error| FileError::Io(format!("could not read {}: {error}", path.display())))?;
    let stamp = FileStamp::capture(path, &bytes);
    if let Ok(model) = DocModel::from_snapshot(&bytes) {
        return Ok(OpenedDocument {
            kind: FileKind::Snapshot,
            model,
            stamp,
        });
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        FileError::Invalid(format!(
            "{} is neither a Loro snapshot nor UTF-8 source: {error}",
            path.display()
        ))
    })?;
    Ok(OpenedDocument {
        kind: FileKind::Source,
        model: DocModel::new(&text),
        stamp,
    })
}

/// Write a document atomically and return the stamp for the new bytes.
pub fn write(
    path: &Path,
    bytes: &[u8],
    kind: FileKind,
    previous: Option<&FileStamp>,
    force: bool,
) -> Result<SavedDocument, FileError> {
    let is_link = std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    // Resolve the symlink once and use that target for the conflict check,
    // the temporary file and the publish. Renaming onto the link itself would
    // replace it and break the user's stable path.
    let destination = resolve_destination(path)?;
    let existed = destination.exists();
    let same_target = previous.is_some_and(|stamp| stamp.canonical_path == destination);

    if !force {
        if let Some(previous) = previous {
            if same_target {
                ensure_unchanged(&destination, previous, path)?;
            } else if existed || is_link {
                // Save As to a destination this document never observed. Do
                // not replace it without the caller's explicit force.
                return Err(FileError::Conflict(path.to_path_buf()));
            }
        } else if existed {
            return Err(FileError::Conflict(path.to_path_buf()));
        }
    }

    // Re-resolve just before publishing so a retargeted link is a conflict,
    // not a write into the old target while the link points somewhere else.
    if resolve_destination(path)? != destination {
        return Err(FileError::Conflict(path.to_path_buf()));
    }
    if !force && same_target {
        if let Some(previous) = previous {
            ensure_unchanged(&destination, previous, path)?;
        }
    }

    let replace = existed;
    crate::session::store::write_atomic_publish(&destination, bytes, replace).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            FileError::Conflict(path.to_path_buf())
        } else {
            FileError::Io(format!("could not write {}: {error}", path.display()))
        }
    })?;
    let stamp = FileStamp::capture(&destination, bytes);
    Ok(SavedDocument {
        path: stamp.canonical_path.clone(),
        kind,
        digest: stamp.sha256.clone(),
        stamp,
    })
}

fn resolve_destination(path: &Path) -> Result<PathBuf, FileError> {
    let is_link = std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok(canonical_target(path));
    }
    std::fs::canonicalize(path).map_err(|_| FileError::Conflict(path.to_path_buf()))
}

fn ensure_unchanged(
    destination: &Path,
    previous: &FileStamp,
    requested: &Path,
) -> Result<(), FileError> {
    let current = std::fs::read(destination).map_err(|_| FileError::Conflict(requested.into()))?;
    if current.len() as u64 != previous.bytes || sha256_hex(&current) != previous.sha256 {
        return Err(FileError::Conflict(requested.to_path_buf()));
    }
    Ok(())
}

fn canonical_target(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    match path.file_name() {
        Some(name) => parent.join(name),
        None => parent,
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
            let path = std::env::temp_dir().join(format!(
                "emthin-docfile-{name}-{}-{seq}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_snapshot_file_round_trips_marks() {
        let dir = TempDir::new("snapshot");
        let path = dir.0.join("notes.loro");
        let mut model = DocModel::new("#1 visible #2\n\\app(#1, #2, 300, 200)");
        model.take_dirty();
        let bytes = model.snapshot();
        let saved = write(&path, &bytes, FileKind::Snapshot, None, false).expect("save");
        assert_eq!(saved.kind, FileKind::Snapshot);

        let opened = read(&path).expect("open");
        assert_eq!(opened.kind, FileKind::Snapshot);
        assert_eq!(opened.model.text(), model.text());
        assert_eq!(
            opened.model.scan().stmts.len(),
            model.scan().stmts.len(),
            "a snapshot preserves the parsed statements"
        );
    }

    #[test]
    fn source_and_snapshot_are_detected_by_content() {
        let dir = TempDir::new("detect");
        let source = dir.0.join("notes.typ");
        std::fs::write(&source, "plain source").expect("source");
        assert_eq!(read(&source).expect("read source").kind, FileKind::Source);

        let model = DocModel::new("snapshot source");
        let snapshot = dir.0.join("notes.mathed");
        write(
            &snapshot,
            &model.snapshot(),
            FileKind::Snapshot,
            None,
            false,
        )
        .expect("snapshot");
        assert_eq!(
            read(&snapshot).expect("read snapshot").kind,
            FileKind::Snapshot
        );
    }

    #[test]
    fn an_outside_write_is_refused_without_force() {
        let dir = TempDir::new("conflict");
        let path = dir.0.join("notes.typ");
        std::fs::write(&path, "first").expect("source");
        let opened = read(&path).expect("open");
        std::fs::write(&path, "second").expect("outside write");

        assert!(matches!(
            write(
                &path,
                b"saved",
                FileKind::Source,
                Some(&opened.stamp),
                false
            ),
            Err(FileError::Conflict(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).expect("source"), "second");

        let saved = write(&path, b"saved", FileKind::Source, Some(&opened.stamp), true)
            .expect("forced save");
        assert_eq!(std::fs::read_to_string(&path).expect("source"), "saved");
        assert_eq!(saved.digest, sha256_hex(b"saved"));
    }

    #[test]
    fn saving_through_a_stable_symlink_writes_its_target_and_keeps_the_link() {
        let dir = TempDir::new("symlink");
        let target = dir.0.join("target.typ");
        let link = dir.0.join("notes.typ");
        std::fs::write(&target, "first").expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("link");
        let opened = read(&link).expect("open through link");

        let saved = write(
            &link,
            b"second",
            FileKind::Source,
            Some(&opened.stamp),
            false,
        )
        .expect("save through link");
        assert_eq!(saved.path, target.canonicalize().unwrap());
        assert_eq!(std::fs::read_to_string(&target).expect("target"), "second");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink(),
            "the stable link still points at the target"
        );
    }

    #[test]
    fn a_retargeted_symlink_is_a_conflict() {
        let dir = TempDir::new("retargeted-link");
        let first = dir.0.join("first.typ");
        let second = dir.0.join("second.typ");
        let link = dir.0.join("notes.typ");
        std::fs::write(&first, "first").expect("first");
        std::fs::write(&second, "second").expect("second");
        std::os::unix::fs::symlink(&first, &link).expect("link");
        let opened = read(&link).expect("open");
        std::fs::remove_file(&link).expect("remove link");
        std::os::unix::fs::symlink(&second, &link).expect("retarget");

        assert!(matches!(
            write(
                &link,
                b"saved",
                FileKind::Source,
                Some(&opened.stamp),
                false
            ),
            Err(FileError::Conflict(_))
        ));
        assert_eq!(std::fs::read_to_string(&first).expect("first"), "first");
        assert_eq!(std::fs::read_to_string(&second).expect("second"), "second");
    }

    #[test]
    fn an_unknown_existing_save_as_target_is_not_replaced_without_force() {
        let dir = TempDir::new("save-as-existing");
        let source = dir.0.join("source.typ");
        let destination = dir.0.join("existing.typ");
        std::fs::write(&source, "source").expect("source");
        std::fs::write(&destination, "existing").expect("destination");
        let opened = read(&source).expect("open");

        assert!(matches!(
            write(
                &destination,
                b"copy",
                FileKind::Source,
                Some(&opened.stamp),
                false
            ),
            Err(FileError::Conflict(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&destination).expect("destination"),
            "existing"
        );

        write(
            &destination,
            b"copy",
            FileKind::Source,
            Some(&opened.stamp),
            true,
        )
        .expect("forced copy");
        assert_eq!(
            std::fs::read_to_string(&destination).expect("destination"),
            "copy"
        );
    }

    #[test]
    fn a_new_destination_is_published_without_clobbering_an_existing_file() {
        let dir = TempDir::new("create-only");
        let destination = dir.0.join("created.typ");
        std::fs::write(&destination, "concurrent").expect("concurrent creator");

        assert!(matches!(
            write(&destination, b"mine", FileKind::Source, None, false),
            Err(FileError::Conflict(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&destination).expect("destination"),
            "concurrent"
        );
    }

    #[test]
    fn a_failed_atomic_write_leaves_the_old_file() {
        let dir = TempDir::new("atomic");
        let path = dir.0.join("notes.typ");
        std::fs::write(&path, "old").expect("source");
        let opened = read(&path).expect("open");
        let mut temp = path.as_os_str().to_os_string();
        temp.push(".tmp");
        std::fs::create_dir(PathBuf::from(temp)).expect("block temp path");

        assert!(write(&path, b"new", FileKind::Source, Some(&opened.stamp), false).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("source"), "old");
    }

    #[test]
    fn a_failed_publish_keeps_the_previous_destination() {
        let dir = TempDir::new("publish-failure");
        let path = dir.0.join("notes.typ");
        std::fs::create_dir(&path).expect("make destination a directory");
        std::fs::write(path.join("keep"), "kept").expect("keep");

        assert!(write(&path, b"new", FileKind::Source, None, true).is_err());
        assert!(
            path.is_dir(),
            "the failed rename left the destination alone"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("keep")).expect("kept"),
            "kept"
        );
    }
}
