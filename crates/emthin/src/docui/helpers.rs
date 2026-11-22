//! Generic helper-process primitive for the third plugin (parked item).
//!
//! `\formal` and `\voice` each hand-roll a helper-process lifecycle: env
//! resolution, a missing-binary no-op, one in-flight job, and a late-reply
//! guard. This module factors that shape once so a new plugin can own a
//! process without reimplementing the safety rules:
//!
//! - one owner per async job (a second submit is rejected);
//! - a generation fence drops replies from a replaced document;
//! - last-good state survives a helper crash (never blank the page);
//! - a missing binary is a silent no-op, not a startup failure;
//! - restart backoff is bounded.
//!
//! The plugin supplies only the argv/stdin and a parser for the reply.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// What to run and what to feed it.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

/// One reply from the worker.
struct HelperEvent<B> {
    generation: u64,
    outcome: Result<B, String>,
}

/// The reusable one-in-flight helper process.
pub struct HelperProcess<B: Send + Clone + 'static> {
    binary: Option<PathBuf>,
    parse: fn(&str) -> Result<B, String>,
    tx: Sender<HelperEvent<B>>,
    rx: Receiver<HelperEvent<B>>,
    generation: u64,
    active: Option<u64>,
    last_good: Option<B>,
    last_error: Option<String>,
    failures: u32,
    backoff_until: Option<Instant>,
    runs: usize,
}

impl<B: Send + Clone + 'static> HelperProcess<B> {
    /// `var` names the env pin, `default` the $PATH fallback. `parse` turns the
    /// helper's stdout into plugin state; an unreadable answer is an error.
    pub fn new(var: &str, default: &str, parse: fn(&str) -> Result<B, String>) -> Self {
        let binary = resolve_binary(std::env::var_os(var), default);
        let (tx, rx) = mpsc::channel();
        Self {
            binary,
            parse,
            tx,
            rx,
            generation: 0,
            active: None,
            last_good: None,
            last_error: None,
            failures: 0,
            backoff_until: None,
            runs: 0,
        }
    }

    pub fn with_binary(binary: Option<PathBuf>, parse: fn(&str) -> Result<B, String>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            binary,
            parse,
            tx,
            rx,
            generation: 0,
            active: None,
            last_good: None,
            last_error: None,
            failures: 0,
            backoff_until: None,
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

    pub fn last_good(&self) -> Option<&B> {
        self.last_good.as_ref()
    }

    pub fn runs(&self) -> usize {
        self.runs
    }

    /// Whether a new job may start now. A crashed helper backs off briefly
    /// rather than being respawned in a tight loop every tick.
    pub fn ready(&self) -> bool {
        self.active.is_none()
            && self
                .backoff_until
                .is_none_or(|until| Instant::now() >= until)
    }

    /// Repin the binary. In-flight replies from the old one are abandoned;
    /// last-good state stays visible.
    pub fn set_binary(&mut self, binary: Option<PathBuf>) {
        if self.binary == binary {
            return;
        }
        self.binary = binary;
        while self.rx.try_recv().is_ok() {}
        self.active = None;
        self.backoff_until = None;
        self.failures = 0;
    }

    /// Queue one job. Rejects a second concurrent run and a missing binary.
    pub fn submit(&mut self, request: &Request) -> Result<u64, String> {
        if self.active.is_some() {
            return Err("a helper job is already running".into());
        }
        let binary = self
            .binary
            .clone()
            .ok_or_else(|| "no helper binary configured".to_string())?;
        let generation = self.generation.checked_add(1).unwrap_or(1);
        self.generation = generation;
        self.active = Some(generation);
        self.runs += 1;
        let tx = self.tx.clone();
        let parse = self.parse;
        let request = request.clone();
        std::thread::spawn(move || {
            let outcome = run_helper(&binary, &request, parse);
            let _ = tx.send(HelperEvent {
                generation,
                outcome,
            });
        });
        Ok(generation)
    }

    /// Drain replies. Returns the new last-good state when a current job
    /// succeeded. Failures keep the previous good state and start a bounded
    /// backoff.
    pub fn poll(&mut self) -> Option<B> {
        let mut updated = None;
        while let Ok(event) = self.rx.try_recv() {
            if self.active.is_some_and(|gen| gen == event.generation) {
                self.active = None;
                match event.outcome {
                    Ok(value) => {
                        self.last_error = None;
                        self.failures = 0;
                        self.backoff_until = None;
                        self.last_good = Some(value.clone());
                        updated = Some(value);
                    }
                    Err(reason) => {
                        self.last_error = Some(reason);
                        self.failures = self.failures.saturating_add(1);
                        let steps = self.failures.min(5);
                        self.backoff_until =
                            Some(Instant::now() + Duration::from_millis(50 * steps as u64));
                    }
                }
            }
            // A stale generation is dropped, keeping the last-good state.
        }
        updated
    }

    /// Forget last-good state (the document changed under it).
    pub fn invalidate(&mut self) {
        self.last_good = None;
        self.active = None;
    }
}

fn run_helper<B>(
    binary: &Path,
    request: &Request,
    parse: fn(&str) -> Result<B, String>,
) -> Result<B, String> {
    let mut command = Command::new(binary);
    command.args(&request.args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if request.stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run {binary:?}: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (&request.stdin, child.stdin.take()) {
        use std::io::Write;
        let _ = stdin.write_all(input.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("helper failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("helper exited {}", output.status)
        } else {
            stderr
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse(&stdout)
}

fn resolve_binary(env_value: Option<OsString>, default: &str) -> Option<PathBuf> {
    env_value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .map(|dir| dir.join(default))
                .find(|candidate| candidate.is_file())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "emthin-helper-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("helper");
        std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        for attempt in 0..50 {
            match Command::new(&path).arg("--warm").output() {
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 49 => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("helper not executable: {e}"),
            }
        }
        path
    }

    fn parse_number(text: &str) -> Result<u32, String> {
        text.trim().parse::<u32>().map_err(|e| e.to_string())
    }

    fn wait_for(helper: &mut HelperProcess<u32>) -> Option<u32> {
        for _ in 0..200 {
            if let Some(value) = helper.poll() {
                return Some(value);
            }
            if !helper.in_flight() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("helper never finished");
    }

    #[test]
    fn one_job_at_a_time_and_last_good_survives_a_crash() {
        let good = fake("echo 7");
        let mut helper = HelperProcess::with_binary(Some(good.clone()), parse_number);
        assert_eq!(helper.submit(&Request::default()), Ok(1));
        assert!(helper.submit(&Request::default()).is_err());
        assert_eq!(wait_for(&mut helper), Some(7));
        assert_eq!(helper.last_good(), Some(&7));

        let bad = fake("echo boom >&2; exit 4");
        helper.set_binary(Some(bad.clone()));
        assert_eq!(helper.submit(&Request::default()), Ok(2));
        assert_eq!(wait_for(&mut helper), None);
        assert!(helper.last_error().is_some());
        assert_eq!(helper.last_good(), Some(&7), "last-good retained");
        let _ = std::fs::remove_dir_all(good.parent().unwrap());
        let _ = std::fs::remove_dir_all(bad.parent().unwrap());
    }

    #[test]
    fn a_stale_generation_does_not_overwrite_last_good() {
        let path = fake("sleep 0.05; echo 9");
        let mut helper = HelperProcess::with_binary(Some(path.clone()), parse_number);
        helper.submit(&Request::default()).expect("submit");
        helper.invalidate();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(helper.poll(), None);
        assert_eq!(helper.last_good(), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_binary_is_a_silent_rejection() {
        let helper = HelperProcess::<u32>::with_binary(None, parse_number);
        assert!(!helper.is_available());
        let mut helper = helper;
        assert!(helper.submit(&Request::default()).is_err());
        assert!(helper.last_good().is_none());
    }
}
