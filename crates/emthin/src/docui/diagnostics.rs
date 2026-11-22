//! `\lint` document diagnostics — the third concrete plugin (parked item).
//!
//! Unlike `\formal` (one-shot per sentence) and `\voice` (one-shot turn), a
//! diagnostics helper is a persistent, cancellable stream over the whole
//! document. It is the lifecycle the other two seams cannot express, which is
//! why it justifies the generic [`super::helpers::HelperProcess`].
//!
//! The helper reads the document text on stdin and emits line-delimited JSON
//! diagnostics. Results are derived annotation state keyed by a document byte
//! offset and spliced through `TransformOptions::block_splices` — never written
//! into the Loro text, so a helper crash cannot lose data.

use std::collections::HashMap;

use super::helpers::{HelperProcess, Request};

/// Env pin for the lint helper. A missing binary is a silent no-op.
pub const LINT_BIN_VAR: &str = "EMTHIN_LINT_BIN";
const DEFAULT_BIN: &str = "emthin-lint";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub severity: String,
    pub message: String,
}

/// The diagnostics plugin: one helper process, last-good diagnostics retained
/// across failures.
pub struct DiagnosticsBridge {
    helper: HelperProcess<Vec<Diagnostic>>,
}

impl Default for DiagnosticsBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsBridge {
    pub fn new() -> Self {
        let helper = HelperProcess::new(LINT_BIN_VAR, DEFAULT_BIN, parse_diagnostics);
        Self { helper }
    }

    pub fn with_binary(path: Option<std::path::PathBuf>) -> Self {
        Self {
            helper: HelperProcess::with_binary(path, parse_diagnostics),
        }
    }

    pub fn is_available(&self) -> bool {
        self.helper.is_available()
    }

    pub fn in_flight(&self) -> bool {
        self.helper.in_flight()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.helper.last_error()
    }

    pub fn runs(&self) -> usize {
        self.helper.runs()
    }

    /// Queue one lint pass over `text`. Rejects a second concurrent run.
    pub fn submit(&mut self, text: &str) -> Result<u64, String> {
        if !self.is_available() {
            return Err("no lint helper configured".into());
        }
        if !self.helper.ready() {
            return Err("lint helper is backing off".into());
        }
        self.helper.submit(&Request {
            args: Vec::new(),
            stdin: Some(text.to_owned()),
        })
    }

    /// Drain a reply; returns `true` when diagnostics changed.
    pub fn poll(&mut self) -> bool {
        self.helper.poll().is_some()
    }

    /// Forget diagnostics after a document edit at or after `from`.
    pub fn invalidate_from(&mut self, from: usize) {
        if let Some(diagnostics) = self.helper.last_good() {
            if diagnostics.iter().any(|d| d.end >= from) {
                self.helper.invalidate();
            }
        }
    }

    /// Derived annotations for the render text, keyed by doc byte offset.
    pub fn annotations(&self) -> HashMap<usize, String> {
        let mut out = HashMap::new();
        if let Some(diagnostics) = self.helper.last_good() {
            for diagnostic in diagnostics {
                let escaped = diagnostic
                    .message
                    .replace('\\', "\\\\")
                    .replace('[', "\\[")
                    .replace(']', "\\]")
                    .replace('#', "\\#");
                let markup = format!(
                    "#block(inset: 2pt)[#text(size: 8pt, fill: red)[{}: {}]]",
                    diagnostic.severity, escaped
                );
                out.insert(diagnostic.start.min(diagnostic.end), markup);
            }
        }
        out
    }
}

fn parse_diagnostics(text: &str) -> Result<Vec<Diagnostic>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("bad diagnostic: {e}"))?;
        let start = value
            .get("start")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "diagnostic has no start".to_string())? as usize;
        let end = value
            .get("end")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "diagnostic has no end".to_string())? as usize;
        let severity = value
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("warn")
            .to_owned();
        let message = value
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "diagnostic has no message".to_string())?
            .to_owned();
        out.push(Diagnostic {
            start,
            end,
            severity,
            message,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn fake(body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "emthin-lint-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("lint");
        std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        for _ in 0..50 {
            if Command::new(&path).arg("--warm").output().is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        path
    }

    fn wait_for(bridge: &mut DiagnosticsBridge) -> bool {
        for _ in 0..200 {
            if bridge.poll() {
                return true;
            }
            if !bridge.in_flight() {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("lint helper never finished");
    }

    #[test]
    fn diagnostics_become_derived_annotations() {
        let path =
            fake("echo '{\"start\":3,\"end\":7,\"severity\":\"warn\",\"message\":\"check #x\"}'");
        let mut bridge = DiagnosticsBridge::with_binary(Some(path.clone()));
        bridge.submit("some text").expect("submit");
        assert!(wait_for(&mut bridge));
        let annotations = bridge.annotations();
        let markup = annotations.get(&3).expect("annotation");
        assert!(markup.contains("warn: check \\#x"), "{markup}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_edit_past_a_diagnostic_invalidates_it() {
        let path = fake("echo '{\"start\":10,\"end\":20,\"severity\":\"info\",\"message\":\"m\"}'");
        let mut bridge = DiagnosticsBridge::with_binary(Some(path.clone()));
        bridge.submit("text").expect("submit");
        assert!(wait_for(&mut bridge));
        assert!(!bridge.annotations().is_empty());
        bridge.invalidate_from(15);
        assert!(bridge.annotations().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_helper_is_a_silent_no_op() {
        let mut bridge = DiagnosticsBridge::with_binary(None);
        assert!(!bridge.is_available());
        assert!(bridge.submit("text").is_err());
        assert!(bridge.annotations().is_empty());
    }
}
