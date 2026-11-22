//! Notebook / kernel execution seam (parked item).
//!
//! Execution never happens in the compositor process. A pinned external
//! kernel binary receives one line-delimited JSON request on stdin and replies
//! with line-delimited JSON output events on stdout. Results are **derived
//! state** keyed by a document offset — they are spliced into the render text
//! through `TransformOptions::block_splices` and never written into the Loro
//! document, so a failed or stale run cannot lose data.
//!
//! Ownership follows the same rule as `VoiceBridge`: one active job, a
//! generation fence so a late reply from a replaced document is dropped, and
//! last-good outputs retained on failure.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

/// Env var naming the kernel binary, mirroring `EMTHIN_LOGOS_BIN` /
/// `EMTHIN_VOICE_BIN`.
pub const KERNEL_BIN_VAR: &str = "EMTHIN_KERNEL_BIN";
const DEFAULT_BIN: &str = "emthin-kernel";

/// One output event from the kernel. Shapes mirror mathed's `KernelOutput`
/// without pulling in its `unfer` path dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelOutput {
    Stream { text: String },
    Result { text: String },
    Error { message: String },
}

impl KernelOutput {
    fn parse(line: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        match value.get("kind")?.as_str()? {
            "stream" => Some(Self::Stream {
                text: value.get("text")?.as_str()?.to_owned(),
            }),
            "result" => Some(Self::Result {
                text: value.get("text")?.as_str()?.to_owned(),
            }),
            "error" => Some(Self::Error {
                message: value.get("message")?.as_str()?.to_owned(),
            }),
            _ => None,
        }
    }
}

/// A reply delivered on the control channel.
#[derive(Debug)]
struct KernelEvent {
    generation: u64,
    block_end: usize,
    outputs: Vec<KernelOutput>,
    failure: Option<String>,
}

/// The one kernel bridge per document.
pub struct KernelBridge {
    binary: Option<PathBuf>,
    tx: Sender<KernelEvent>,
    rx: Receiver<KernelEvent>,
    /// Monotonic job id. A reply with a generation that is no longer active is
    /// dropped, so a late result cannot land after a document replacement.
    generation: u64,
    active: Option<(u64, usize)>,
    /// Last good output per block end, retained across failures.
    last_good: HashMap<usize, String>,
    last_error: Option<String>,
    runs: usize,
}

impl Default for KernelBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelBridge {
    pub fn new() -> Self {
        Self::with_binary(resolve_binary(std::env::var_os(KERNEL_BIN_VAR)))
    }

    pub fn with_binary(binary: Option<PathBuf>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            binary,
            tx,
            rx,
            generation: 0,
            active: None,
            last_good: HashMap::new(),
            last_error: None,
            runs: 0,
        }
    }

    pub fn is_available(&self) -> bool {
        self.binary.is_some()
    }

    pub fn binary(&self) -> Option<&Path> {
        self.binary.as_deref()
    }

    pub fn in_flight(&self) -> bool {
        self.active.is_some()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn runs(&self) -> usize {
        self.runs
    }

    /// Repin the binary. A job spawned against the old one is abandoned; its
    /// last good output stays visible (never blanked by a seam change).
    pub fn set_binary(&mut self, binary: Option<PathBuf>) {
        if self.binary == binary {
            return;
        }
        self.binary = binary;
        while self.rx.try_recv().is_ok() {}
        self.active = None;
    }

    /// Queue one execution for the block ending at `block_end`.
    ///
    /// Rejecting a second submission is the "one owner per async job" rule:
    /// two workers owning the same block would race to splice its output.
    pub fn submit(
        &mut self,
        block_end: usize,
        code: &str,
        lang: Option<&str>,
    ) -> Result<u64, String> {
        if self.active.is_some() {
            return Err("a kernel job is already running".into());
        }
        let binary = self.binary.clone().ok_or_else(|| {
            format!("no kernel binary (set {KERNEL_BIN_VAR} or put {DEFAULT_BIN} on PATH)")
        })?;
        let generation = self.generation.checked_add(1).unwrap_or(1);
        self.generation = generation;
        self.active = Some((generation, block_end));
        let tx = self.tx.clone();
        let request = serde_json::json!({
            "block_end": block_end,
            "lang": lang,
            "code": code,
        })
        .to_string();
        self.runs += 1;
        std::thread::spawn(move || {
            let outcome = run_kernel(&binary, &request);
            let _ = tx.send(KernelEvent {
                generation,
                block_end,
                outputs: outcome.outputs,
                failure: outcome.failure,
            });
        });
        Ok(generation)
    }

    /// Drain replies. Returns the block ends whose output changed, so the
    /// caller can rebuild the transform only when something actually moved.
    pub fn poll(&mut self) -> Vec<usize> {
        let mut changed = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            let current = self.active;
            let is_current = current.is_some_and(|(gen, _)| gen == event.generation);
            if !is_current {
                continue; // stale generation: a replaced document owns it now
            }
            self.active = None;
            if let Some(reason) = event.failure {
                self.last_error = Some(reason);
                continue; // keep the last-good output for this block
            }
            self.last_error = None;
            let rendered = render_outputs(&event.outputs);
            if rendered.is_empty() {
                self.last_good.remove(&event.block_end);
            } else {
                self.last_good.insert(event.block_end, rendered);
            }
            changed.push(event.block_end);
        }
        changed
    }

    /// The derived output splices, keyed by document byte offset.
    pub fn outputs(&self) -> &HashMap<usize, String> {
        &self.last_good
    }

    /// Forget outputs for blocks at or after `from` (the document was edited
    /// there, so the cached region is stale).
    pub fn invalidate_from(&mut self, from: usize) {
        self.last_good.retain(|end, _| *end < from);
        self.active = None;
    }
}

struct RunOutcome {
    outputs: Vec<KernelOutput>,
    failure: Option<String>,
}

fn run_kernel(binary: &Path, request: &str) -> RunOutcome {
    let mut child = match Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return RunOutcome {
                outputs: Vec::new(),
                failure: Some(format!("could not run {binary:?}: {error}")),
            }
        }
    };
    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{request}");
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            return RunOutcome {
                outputs: Vec::new(),
                failure: Some(format!("kernel failed: {error}")),
            }
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return RunOutcome {
            outputs: Vec::new(),
            failure: Some(if stderr.is_empty() {
                format!("kernel exited {}", output.status)
            } else {
                stderr
            }),
        };
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut outputs = Vec::new();
    for line in stdout.lines() {
        if let Some(event) = KernelOutput::parse(line) {
            outputs.push(event);
        }
    }
    RunOutcome {
        outputs,
        failure: None,
    }
}

/// Render output events into the Typst markup spliced beneath the block.
/// Errors are shown inline rather than turning into a silent success, and the
/// text is escaped so a kernel cannot inject arbitrary markup.
fn render_outputs(outputs: &[KernelOutput]) -> String {
    let mut parts = Vec::new();
    for output in outputs {
        let (tag, text) = match output {
            KernelOutput::Stream { text } => ("stream", text),
            KernelOutput::Result { text } => ("result", text),
            KernelOutput::Error { message } => ("error", message),
        };
        parts.push(format!(
            "#block(fill: rgb(\"#f6f6f6\"), inset: 4pt)[#text(size: 8pt)[{tag}: {}]]",
            escape_markup(text)
        ));
    }
    parts.join("\n")
}

fn escape_markup(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('#', "\\#")
}

fn resolve_binary(env_value: Option<OsString>) -> Option<PathBuf> {
    env_value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .map(|dir| dir.join(DEFAULT_BIN))
                .find(|candidate| candidate.is_file())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_kernel(body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "emthin-fake-kernel-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("kernel");
        std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\n{body}\n"))
            .expect("write kernel");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        for attempt in 0..50 {
            match Command::new(&path).arg("--warm").output() {
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 49 => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => panic!("fake kernel not executable: {e}"),
            }
        }
        path
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn wait_for(bridge: &mut KernelBridge) -> Vec<usize> {
        for _ in 0..200 {
            let changed = bridge.poll();
            if !changed.is_empty() || !bridge.in_flight() {
                return changed;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("kernel job never finished");
    }

    #[test]
    fn one_job_at_a_time_and_output_is_derived_not_persisted() {
        let path = fake_kernel("echo '{\"kind\":\"result\",\"text\":\"42\"}'");
        let mut bridge = KernelBridge::with_binary(Some(path.clone()));
        assert_eq!(bridge.submit(10, "1+1", Some("python")), Ok(1));
        assert!(
            bridge.submit(20, "2+2", None).is_err(),
            "second submit rejected"
        );
        let changed = wait_for(&mut bridge);
        assert_eq!(changed, vec![10]);
        let markup = bridge.outputs().get(&10).expect("output").clone();
        assert!(markup.contains("result: 42"), "{markup}");
        // Invalidation drops derived output without touching any document.
        bridge.invalidate_from(10);
        assert!(!bridge.outputs().contains_key(&10));
        cleanup(&path);
    }

    #[test]
    fn a_failed_run_keeps_the_last_good_output() {
        let good = fake_kernel("echo '{\"kind\":\"result\",\"text\":\"ok\"}'");
        let mut bridge = KernelBridge::with_binary(Some(good.clone()));
        bridge.submit(5, "code", None).expect("submit");
        wait_for(&mut bridge);
        assert!(bridge.outputs().contains_key(&5));

        let bad = fake_kernel("echo 'boom' >&2; exit 3");
        bridge.set_binary(Some(bad.clone()));
        bridge.submit(5, "code", None).expect("submit");
        wait_for(&mut bridge);
        assert!(bridge.last_error().is_some());
        assert!(bridge.outputs().contains_key(&5), "last-good retained");
        cleanup(&good);
        cleanup(&bad);
    }

    #[test]
    fn a_late_reply_from_a_replaced_document_is_dropped() {
        let path = fake_kernel("sleep 0.05; echo '{\"kind\":\"result\",\"text\":\"late\"}'");
        let mut bridge = KernelBridge::with_binary(Some(path.clone()));
        let gen = bridge.submit(7, "code", None).expect("submit");
        // Simulate the document moving on: the active generation is abandoned.
        bridge.invalidate_from(7);
        assert!(bridge.active.is_none());
        // Let the worker finish and deliver; the stale generation must not land.
        std::thread::sleep(std::time::Duration::from_millis(150));
        let changed = bridge.poll();
        assert!(changed.is_empty(), "stale reply changed {changed:?}");
        assert!(!bridge.outputs().contains_key(&7));
        let _ = gen;
        cleanup(&path);
    }

    #[test]
    fn no_binary_is_a_silent_rejection() {
        let mut bridge = KernelBridge::with_binary(None);
        assert!(!bridge.is_available());
        assert!(bridge.submit(0, "code", None).is_err());
    }

    #[test]
    fn markup_is_escaped_so_kernel_text_cannot_inject_markup() {
        let rendered = render_outputs(&[KernelOutput::Stream {
            text: "#set page(width: 1pt) [x]".into(),
        }]);
        assert!(rendered.contains("\\#set"), "{rendered}");
        assert!(
            !rendered.contains("[#set"),
            "unescaped markup injection: {rendered}"
        );
        assert!(rendered.contains("\\[x\\]"), "{rendered}");
    }
}
