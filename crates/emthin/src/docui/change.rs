//! Typed committed changes and source diagnostics (Kyneta K1).
//!
//! A [`DocChange`] is the one notification published for a *grouped* edit at
//! the document boundary. It names the owner (document instance plus the
//! committed Loro version), the origin, and what changed. Presentation-only
//! activity — caret moves, pan, status, presence — is deliberately not a
//! `DocChange`: it is a read-only projection and never advances the durable
//! version.
//!
//! This module depends only on `mathed_core` and `std` so the pad can compile
//! it verbatim alongside `docui/model.rs` (see `emthin-android/src/lib.rs`).

use std::collections::HashMap;
use std::ops::Range;

use mathed_core::markers::{MarkerScan, PropKind, Segment};
use mathed_core::semantics::SemanticIndex;
use mathed_core::transform::{to_render_text, TransformOptions};

/// Where a committed change came from.
///
/// Only `Local`, `Peer`, `Undo` and `Redo` advance the durable document
/// version. `Restore` re-applies saved view state and is read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOrigin {
    Local,
    Peer,
    Undo,
    Redo,
    Restore,
}

impl ChangeOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Peer => "peer",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Restore => "restore",
        }
    }

    /// The inverse of [`Self::as_str`], for validating an incoming notification
    /// before it is applied to a derived view.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "peer" => Some(Self::Peer),
            "undo" => Some(Self::Undo),
            "redo" => Some(Self::Redo),
            "restore" => Some(Self::Restore),
            _ => None,
        }
    }

    /// Whether this origin advances the durable document version.
    pub fn is_durable(self) -> bool {
        !matches!(self, Self::Restore)
    }
}

/// Whether a change touched durable content or only a read-only projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeScope {
    /// Text or marks; advances the Loro version and the content generation.
    Content,
    /// View/caret/status/presence; never advances the durable version.
    Presentation,
}

impl ChangeScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Presentation => "presentation",
        }
    }
}

/// One published, grouped change summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocChange {
    /// Monotonic sequence, assigned by the document that produced the change.
    pub seq: u64,
    /// The document instance that produced the change. A replacement document
    /// starts a new instance, so a change from an old instance is never applied
    /// to a new one.
    pub document: u64,
    /// The content generation the change was committed at.
    pub content_generation: u64,
    /// The committed Loro version vector, hex encoded. Marks are included; the
    /// text-only `revision()` is not sufficient to discriminate a mark change.
    pub version: String,
    pub origin: ChangeOrigin,
    pub scope: ChangeScope,
    /// Source ranges whose rendering was affected. Empty for a change whose
    /// affected spans cannot be attributed (an undo, for example).
    pub ranges: Vec<Range<usize>>,
    /// SHA-256 of the text only. Identical across a marks-only edit.
    pub source_digest: String,
    /// SHA-256 of text plus marks. Changes on a marks-only edit.
    pub content_digest: String,
}

/// Hard ceiling on an imported document payload, mirroring
/// `emthin_collab::wire::MAX_FRAME`. A peer must not be able to make the
/// document allocate an unbounded buffer before validation runs.
pub const MAX_IMPORT_BYTES: usize = 16 * 1024 * 1024;

/// A malformed imported payload. No field of the document is applied when one
/// of these is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Empty,
    TooLarge { len: usize, max: usize },
    BadVersion(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty import payload"),
            Self::TooLarge { len, max } => write!(f, "import payload is {len} bytes (max {max})"),
            Self::BadVersion(message) => write!(f, "invalid import version: {message}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Validate an imported payload's type, compatibility and bounds *before* any
/// document data is applied.
///
/// A snapshot is a Loro snapshot: its own decoder checks the internal bytes.
/// This gate is the cheap outer envelope — size, non-emptiness, and a version
/// token (when one is supplied) that must decode to a real version vector.
pub fn validate_import(payload: &[u8], version: Option<&[u8]>) -> Result<(), ImportError> {
    if payload.is_empty() {
        return Err(ImportError::Empty);
    }
    if payload.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge {
            len: payload.len(),
            max: MAX_IMPORT_BYTES,
        });
    }
    if let Some(version) = version {
        // `export_delta_since` decodes the token before it exports anything, so
        // a malformed version is rejected without applying or producing a
        // delta. An empty token is the documented "from the beginning" case.
        mathed_core::doc::MathDoc::new()
            .export_delta_since(version)
            .map_err(|error| ImportError::BadVersion(error.to_string()))?;
    }
    Ok(())
}

// ----- source diagnostics ---------------------------------------------

/// The stable identity of a diagnostic. Codes are shared between the desktop
/// compositor and the Android pad so a golden set can be compared verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    /// A `\name(#a, #b, …)` statement whose markers are missing or reversed.
    MarkerPair,
    /// A definition use that resolves to no definition.
    UnresolvedName,
    /// An `\app` figure whose marker pair does not resolve to a caption span.
    DanglingFigure,
    /// A `\formal` statement that carries no verified result yet.
    MissingProofResult,
}

impl DiagnosticCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MarkerPair => "marker-pair",
            Self::UnresolvedName => "unresolved-name",
            Self::DanglingFigure => "dangling-figure",
            Self::MissingProofResult => "missing-proof-result",
        }
    }
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// A structured, source-ranged diagnostic derived from shared document facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    /// UTF-8 byte range in the document source.
    pub range: Range<usize>,
    pub message: String,
}

/// The golden set of marker, citation and proof-note diagnostics.
///
/// Derived only from the shared scanner, the semantic index and the formal
/// specs — no Typst render and no frontend-specific state — so both frontends
/// produce the same records for the same text.
pub fn source_diagnostics(text: &str, scan: &MarkerScan, segments: &[Segment]) -> Vec<Diagnostic> {
    let mut result = Vec::new();

    // Missing or reversed marker pairs. A statement needs at least two marker
    // arguments; the first two are the opening and closing marker.
    let by_id: HashMap<&str, &mathed_core::markers::Marker> = scan
        .markers
        .iter()
        .map(|marker| (marker.id.as_str(), marker))
        .collect();
    for stmt in &scan.stmts {
        let [first, second, ..] = stmt.args.as_slice() else {
            continue;
        };
        let (Some(start_id), Some(end_id)) = (first.marker_id(), second.marker_id()) else {
            continue;
        };
        let message = match (by_id.get(start_id), by_id.get(end_id)) {
            (None, Some(_)) => Some(format!("Marker #{start_id} is missing.")),
            (Some(_), None) => Some(format!("Marker #{end_id} is missing.")),
            (None, None) => Some(format!("Markers #{start_id} and #{end_id} are missing.")),
            (Some(a), Some(b)) if a.range.end > b.range.start => Some(format!(
                "Markers #{start_id} and #{end_id} are reversed; the first must precede the second."
            )),
            _ => None,
        };
        if let Some(message) = message {
            result.push(Diagnostic {
                code: DiagnosticCode::MarkerPair,
                severity: Severity::Error,
                range: stmt.range.clone(),
                message,
            });
        }
    }

    // Unresolved definition uses. The semantic index derives occurrences from
    // the transformed render, so build the same one render the pad uses.
    let render = to_render_text(text, scan, segments, &TransformOptions::default());
    let mut index = SemanticIndex::default();
    index.build_index(text, segments, &[&render]);
    for occurrence in index.unresolved_occurrences() {
        result.push(Diagnostic {
            code: DiagnosticCode::UnresolvedName,
            severity: Severity::Warning,
            range: occurrence.range,
            message: format!("Name “{}” has no resolved definition.", occurrence.name),
        });
    }

    // An `\app` whose marker pair does not resolve to a caption span reserves
    // no figure, so the document cannot place the bound app.
    for segment in segments {
        if segment.kind == PropKind::App && segment.span.is_none() {
            let range = scan
                .stmts
                .get(segment.stmt)
                .map(|stmt| stmt.range.clone())
                .unwrap_or(0..text.len());
            result.push(Diagnostic {
                code: DiagnosticCode::DanglingFigure,
                severity: Severity::Error,
                range,
                message: "A figure statement has no resolved caption span; no figure is placed."
                    .to_owned(),
            });
        }
    }

    // A `\formal` statement that has not been verified yet.
    for segment in segments {
        if segment.kind == PropKind::Formal && segment.span.is_some() {
            if let Some(resolved) = mathed_core::formalize::resolve_formal(segment) {
                if !resolved.spec.has_result() {
                    result.push(Diagnostic {
                        code: DiagnosticCode::MissingProofResult,
                        severity: Severity::Warning,
                        range: resolved.span,
                        message: "The formal statement has no verified result yet.".to_owned(),
                    });
                }
            }
        }
    }

    // Stable order independent of the four producers above.
    result.sort_by(|a, b| {
        let a = (a.severity.as_str(), a.range.start, a.code.as_str());
        let b = (b.severity.as_str(), b.range.start, b.code.as_str());
        a.cmp(&b)
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_round_trip_and_only_restore_is_read_only() {
        for origin in [
            ChangeOrigin::Local,
            ChangeOrigin::Peer,
            ChangeOrigin::Undo,
            ChangeOrigin::Redo,
            ChangeOrigin::Restore,
        ] {
            assert_eq!(ChangeOrigin::parse(origin.as_str()), Some(origin));
        }
        assert!(ChangeOrigin::Local.is_durable());
        assert!(!ChangeOrigin::Restore.is_durable());
        assert_eq!(ChangeOrigin::parse("nope"), None);
    }

    #[test]
    fn imports_are_bounded_and_version_checked_before_application() {
        assert_eq!(validate_import(b"", None), Err(ImportError::Empty));
        assert_eq!(
            validate_import(&vec![0u8; MAX_IMPORT_BYTES + 1], None),
            Err(ImportError::TooLarge {
                len: MAX_IMPORT_BYTES + 1,
                max: MAX_IMPORT_BYTES
            })
        );
        assert!(validate_import(b"opaque snapshot bytes", None).is_ok());
        assert!(validate_import(b"x", Some(&[0xff, 0xff])).is_err());
    }

    #[test]
    fn the_proof_note_golden_set_is_stable_and_source_ranged() {
        // One claim with no proof result, one dangling figure, and one
        // unresolved math identifier — the marker, proof-note and citation
        // families that both frontends must agree on.
        let text = "#1 claim #2 \\formal(#1,#2, \"a claim\")\n\
                    #3 fig #4 \\app(#3,#8, 10, 20)\n\
                    $missing$";
        let scan = mathed_core::markers::scan(text);
        let segments = mathed_core::markers::resolve_segments(&scan);
        let diagnostics = source_diagnostics(text, &scan, &segments);
        let codes: Vec<_> = diagnostics.iter().map(|d| d.code).collect();
        for expected in [
            DiagnosticCode::MarkerPair,
            DiagnosticCode::DanglingFigure,
            DiagnosticCode::MissingProofResult,
            DiagnosticCode::UnresolvedName,
        ] {
            assert!(
                codes.contains(&expected),
                "missing {expected:?}: {diagnostics:?}"
            );
        }
        // Diagnostics are sorted by (severity, start, code).
        for pair in diagnostics.windows(2) {
            let a = (
                pair[0].severity.as_str(),
                pair[0].range.start,
                pair[0].code.as_str(),
            );
            let b = (
                pair[1].severity.as_str(),
                pair[1].range.start,
                pair[1].code.as_str(),
            );
            assert!(a <= b);
        }
        for diagnostic in &diagnostics {
            assert!(diagnostic.range.start <= diagnostic.range.end);
            assert!(diagnostic.range.end <= text.len());
            assert_eq!(
                &text[diagnostic.range.start..diagnostic.range.end],
                text.get(diagnostic.range.start..diagnostic.range.end)
                    .unwrap_or_default(),
            );
        }
    }
}
