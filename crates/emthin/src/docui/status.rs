//! The compact projection of document state shown at the bottom of the page.
//!
//! This is not a second source of truth: every field is copied from an owner
//! (`DocUi`, `FindBar`, the collaboration session) at draw time. Quiet state
//! produces no line at all, so a clean document with no peers and no failure
//! adds no furniture.

/// Whether the document has changed since the last successful save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    Clean,
    Unsaved,
    Failed,
}

/// Recovery/session persistence, separate from the explicit file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryState {
    Clean,
    Recovered,
    Failed,
}

/// The rendered preview: current, rebuilding, or retaining a stale raster
/// because the newest compile failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewState {
    Current,
    Updating,
    Failed,
}

impl SaveState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Unsaved => "unsaved",
            Self::Failed => "failed",
        }
    }
}

impl RecoveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Recovered => "recovered",
            Self::Failed => "failed",
        }
    }
}

impl PreviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Updating => "updating",
            Self::Failed => "failed",
        }
    }
}

/// One status line's worth of applicable state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusModel {
    pub document: String,
    pub save: SaveState,
    pub recovery: RecoveryState,
    pub preview: PreviewState,
    pub save_warning: Option<String>,
    /// A recovery failure that is separate from the explicit file's state.
    pub recovery_error: Option<String>,
    pub layout_error: Option<String>,
    pub find: Option<String>,
    pub peers: usize,
    pub voice: bool,
    pub formal: bool,
    /// A `\lint` diagnostics pass is in flight (third plugin).
    pub diagnostics: bool,
}

impl StatusModel {
    /// The line to draw, or `None` when nothing needs saying.
    ///
    /// Order is deliberate: layout/save failures first, then Find, then
    /// collaborators and background work. The document name appears only when
    /// there is something else to say, which keeps the quiet state empty.
    pub fn line(&self) -> Option<String> {
        let quiet = self.save == SaveState::Clean
            && self.recovery == RecoveryState::Clean
            && self.preview == PreviewState::Current
            && self.save_warning.is_none()
            && self.recovery_error.is_none()
            && self.layout_error.is_none()
            && self.find.is_none()
            && self.peers == 0
            && !self.voice
            && !self.formal
            && !self.diagnostics;
        if quiet {
            return None;
        }

        let mut parts = vec![self.document.clone()];
        match self.preview {
            PreviewState::Failed => parts.push(format!(
                "preview failed: {}",
                self.layout_error
                    .as_deref()
                    .map(first_line)
                    .unwrap_or_else(|| "unknown error".to_owned())
            )),
            PreviewState::Updating => parts.push("preview updating".to_owned()),
            PreviewState::Current => {
                if let Some(error) = &self.layout_error {
                    parts.push(format!("preview failed: {}", first_line(error)));
                }
            }
        }
        match self.save {
            SaveState::Clean => {}
            SaveState::Unsaved => parts.push("unsaved".to_owned()),
            SaveState::Failed => parts.push("save failed".to_owned()),
        }
        match self.recovery {
            RecoveryState::Clean => {}
            RecoveryState::Recovered => parts.push("recovered copy".to_owned()),
            RecoveryState::Failed => {
                if let Some(error) = &self.recovery_error {
                    parts.push(format!("recovery failed: {}", first_line(error)));
                } else {
                    parts.push("recovery failed".to_owned());
                }
            }
        }
        if self.save_warning.is_some() && self.recovery != RecoveryState::Recovered {
            parts.push("recovered backup".to_owned());
        }
        if let Some(find) = &self.find {
            parts.push(find.clone());
        }
        if self.peers > 0 {
            parts.push(if self.peers == 1 {
                "1 peer".to_owned()
            } else {
                format!("{} peers", self.peers)
            });
        }
        if self.voice {
            parts.push("voice".to_owned());
        }
        if self.formal {
            parts.push("formal".to_owned());
        }
        if self.diagnostics {
            parts.push("lint".to_owned());
        }
        Some(parts.join("  "))
    }
}

/// Keep the line compact; the full error remains available over IPC.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut out: String = line.chars().take(96).collect();
    if line.chars().count() > 96 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> StatusModel {
        StatusModel {
            document: "paper.typ".into(),
            save: SaveState::Clean,
            recovery: RecoveryState::Clean,
            preview: PreviewState::Current,
            save_warning: None,
            recovery_error: None,
            layout_error: None,
            find: None,
            peers: 0,
            voice: false,
            formal: false,
            diagnostics: false,
        }
    }

    #[test]
    fn quiet_state_renders_nothing() {
        assert_eq!(model().line(), None);
    }

    #[test]
    fn failures_precede_find_then_peers() {
        let mut m = model();
        m.save = SaveState::Failed;
        m.preview = PreviewState::Failed;
        m.layout_error = Some("expected `]`".into());
        m.find = Some("Find 2/3".into());
        m.peers = 2;
        m.voice = true;
        let line = m.line().expect("a line");
        let layout = line.find("preview failed").expect("preview");
        let save = line.find("save failed").expect("save");
        let find = line.find("Find 2/3").expect("find");
        let peers = line.find("2 peers").expect("peers");
        let voice = line.find("voice").expect("voice");
        assert!(layout < save && save < find && find < peers && peers < voice);
        assert!(line.starts_with("paper.typ"), "{line}");
    }

    #[test]
    fn a_recovered_backup_is_a_warning_not_a_failure() {
        let mut m = model();
        m.recovery = RecoveryState::Recovered;
        m.save_warning = Some("loaded the backup pair".into());
        assert_eq!(m.save, SaveState::Clean);
        assert_eq!(
            m.line().as_deref(),
            Some("paper.typ  recovered copy"),
            "a recovery is visible without claiming the save failed"
        );
    }

    #[test]
    fn a_recovery_failure_is_visible_while_the_file_is_clean() {
        let mut m = model();
        m.recovery = RecoveryState::Failed;
        m.recovery_error = Some("session save failed: disk full".into());
        assert_eq!(m.save, SaveState::Clean);
        assert_eq!(
            m.line().as_deref(),
            Some("paper.typ  recovery failed: session save failed: disk full"),
            "a failed recovery does not make the explicit file look unsaved"
        );
    }

    #[test]
    fn peer_count_and_names_are_compact() {
        let mut m = model();
        m.peers = 1;
        assert_eq!(m.line().as_deref(), Some("paper.typ  1 peer"));
    }

    #[test]
    fn preview_updating_and_failed_are_distinct_from_save_state() {
        let mut m = model();
        m.preview = PreviewState::Updating;
        assert_eq!(m.line().as_deref(), Some("paper.typ  preview updating"));

        m.preview = PreviewState::Failed;
        m.layout_error = Some("expected `]`\nsecond line".into());
        assert_eq!(
            m.line().as_deref(),
            Some("paper.typ  preview failed: expected `]`")
        );
        assert_eq!(m.save, SaveState::Clean, "the file state is untouched");
    }

    #[test]
    fn file_and_recovery_failures_are_reported_independently() {
        let mut m = model();
        m.save = SaveState::Failed;
        m.recovery = RecoveryState::Failed;
        m.recovery_error = Some("session write failed".into());
        let line = m.line().expect("both failures are visible");
        assert!(line.contains("save failed"), "{line}");
        assert!(
            line.contains("recovery failed: session write failed"),
            "{line}"
        );
    }

    #[test]
    fn a_long_layout_error_is_truncated_to_one_line() {
        let mut m = model();
        m.preview = PreviewState::Failed;
        m.layout_error = Some(format!("{}\nsecond line", "x".repeat(200)));
        let line = m.line().expect("a line");
        assert!(line.ends_with('…'), "{line}");
        assert!(!line.contains("second line"), "{line}");
    }
}
